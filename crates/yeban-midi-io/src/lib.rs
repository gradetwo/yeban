//! 物理 MIDI 输入接入 —— `crates/yeban-midi-io` [`ADR-0006`]。
//!
//! # 本 crate 的职责与边界
//!
//! ⭐ 依 `docs/adr/ADR-0006-hardware-midi-input-via-midir.md`（⭐ **`Accepted`**，⭐ 负责人
//! 2026-10-11 批准）：
//!
//! * **D1**：宿主依赖 `midir`，版本**钉死 `0.11.1`**（⭐ 许可证 MIT ⇒ 已在 `deny.toml` `allow` 内）；
//! * **D2**：⭐ **独立 crate**（本条）⇒ ⭐ 把被审计的 FFI 面带**收在一处**；
//! * **D5**：⭐ feature 名 ＝ **`midi-input`**，⭐ **默认关**（⭐ `default` 不含它）；
//! * **D6**：后端跟随 `midir` 默认（macOS `coremidi`／Linux `alsa`／Windows `windows`），
//!   ⛔ **不开 `jack`**；
//! * **D3**：⭐ `midir` 的回调是 **OS 的 MIDI 线程**（⭐ ⛔ 不是我们的音频线程），
//!   但 ⭐ **同样按红线 7 对待**：⭐ 回调内 **⛔ 零分配／⛔ 零 I/O／⛔ 不加锁** ⇒
//!   只把**定长事件**写进**构造期预分配**的无锁 SPSC 环，⭐ 由控制侧线程消费；
//!   ⭐ **复用**本仓既有的环范式（`yeban-engine` 的 `ring.rs::event_channel` /
//!   `record.rs::record_channel`），⛔ 不另造；
//! * **D4**：⭐ **必须用 `midir` 的原始时间戳**（⛔ 不启用 `avoid_timestamping`）⇒
//!   换算到 **960 PPQ**，⭐ 且**换算基准（会话起点）必须显式定义并记录**，
//!   ⛔ 不得隐式用"收到回调的时刻"。
//!
//! # 当前进度（⭐ 如实登记，⛔ 不假装整条通）
//!
//! ⭐ **已接线**（⭐ 依 **D8**：⭐ 根 `Cargo.toml` 登记 `midir` ＋ ⭐ CI 的 `deny` 作业已绿
//! ⇒ ⭐ `midir` 的真实建流与回调接线可以落）：
//!
//! * ⭐ [`ticks_from_origin_us`]（⭐ **纯函数** ⇒ 无声卡也能判）；
//! * ⭐ [`input`] 模块：⭐ 端口枚举（[`input::enumerate_input_ports`]）＋ 定长事件
//!   （[`input::MidiEvent`]）＋ 预分配无锁环（[`input::event_channel`]）＋
//!   模拟回调体（[`input::MidiEventSender::handle_message`]）＋ 会话
//!   （[`input::open_input`]／[`input::MidiInputSession`]）＋ `RSK-22` **临时**读数口径
//!   （[`input::jitter_reading`]，⭐ 标注 `待裁`）。
//!
//! ⚠ ⭐ **仍然断的地方（⛔ 不假装整条通）**：⭐ `midir` 的**真回调线程**在**本机与 CI
//! 都跑不到**（⭐ CI 容器无 MIDI 硬件）⇒ ⭐ 判据覆盖的是**回调体的内核**
//! （[`input::MidiEventSender::handle_message`]），⭐ 真回调与它**走同一段代码**，
//! ⭐ 但"⭐ OS 真的把字节送进来"这一层**只能在有声卡的机器上验**。
//!
//! ⚠ ⭐ **已登记的断点（与输出侧同款）**：⭐ `midir` 的
//! `build_input_stream → play → 真回调线程` 在**本机与 CI 都无法覆盖**（⭐ 本机不编该依赖、
//! ⭐ CI 容器无声卡）⇒ ⭐ 将来在**有声卡的机器**上做时，⭐ 必须一并验证：
//!
//! 1. ⭐ 事件时间戳取自 `midir` 原始时间戳（⛔ `avoid_timestamping` 保持关闭）；
//! 2. ⭐ 回调内**零分配／零 I/O／不加锁**（⭐ 用既有零分配窗口口径判）；
//! 3. ⭐ 换算基准 ＝ **显式定义的会话起点**（⭐ `origin_us`），⛔ 不是回调到达时刻；
//! 4. ⭐ ⚠ **`RSK-22` 的抖动量化口径尚未裁定**（⭐ 人类裁决：**留待实现期再裁**）⇒
//!    ⭐ **接线前必须先给出可判定的抖动读数（p50／p99／max ＋ 样本量）**，
//!    ⛔ 不得以"未裁"为由跳过读数。

#![forbid(unsafe_code)]

pub mod input;

pub use input::{
    DEFAULT_EVENT_CAPACITY, JitterReading, MidiEvent, MidiEventReceiver, MidiEventSender,
    MidiInputError, MidiPortInfo, event_channel, jitter_reading,
};

#[cfg(feature = "midi-input")]
pub use input::{MidiInputSession, enumerate_input_ports, open_input};

/// 内部时间基准：**每四分音符 960 Tick**（[`MODEL-AST-001`]）。
pub const PPQ: u32 = 960;

/// 微秒 → Tick 的换算分母（1 秒 ＝ 1,000,000 µs）。
const MICROS_PER_SECOND: u64 = 1_000_000;

/// ⭐ **把 `midir` 的原始时间戳换算成 960 PPQ 的 Tick**（⭐ 纯函数 ⇒ 无声卡也能判）。
///
/// ⚠ ⭐ **`origin_us` 必须由调用方**显式**定义并记录**（⭐ 例：⭐ 会话开始时的
/// `midir` 时间戳）⇒ ⭐ **本函数 ⛔ 不接受"当前时刻"当基准**，⭐ 因为那会把
/// 线程调度延迟算进音乐时间（⭐ 那正是 `RSK-22` 要量的抖动）。
///
/// ⚠ ⭐ **速度由调用方给**（审计 #17）：本函数的前身把「1 秒 = 一个四分音符」
/// 写死（= 60 BPM），非 60 BPM 工程的录音落点整体缩放出错。本 crate **刻意**
/// 不依赖工程模型（见 `Cargo.toml`），因此只收 `bpm` 数值；带变速映射的
/// 分段走查在 `yeban_model::time_math::TempoMap::ticks_at_micros`（app 侧落点
/// 走那条）。恒定速度下本算式与模型层**逐字同形、逐位相等**（app 有交叉
/// 对账判据钉住）。
///
/// ⭐ **不 panic**：⭐ `now_us < origin_us`（⭐ 例：⭐ 时间戳回绕或起点晚于事件）⇒
/// ⭐ 返回 **0**（⭐ 饱和），⭐ ⛔ 不触发减法下溢。`bpm` 非有限或 `<= 0` ⇒
/// ⭐ 返回 `u64::MAX`（⭐ 响亮地错，⛔ 不静默回退到任何速度）。
///
/// [`MODEL-AST-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
#[must_use]
pub fn ticks_from_origin_us(now_us: u64, origin_us: u64, bpm: f64) -> u64 {
    if !bpm.is_finite() || bpm <= 0.0 {
        return u64::MAX;
    }
    let elapsed = now_us.saturating_sub(origin_us);
    // ⭐ 求值顺序是契约的一部分：`elapsed × PPQ × bpm / (1_000_000 × 60)`，
    // 与模型层恒定速度算式逐字同形 ⇒ 同一输入逐位相等。
    #[allow(clippy::cast_precision_loss)]
    let ticks = (elapsed as f64) * (f64::from(PPQ)) * bpm / (MICROS_PER_SECOND as f64 * 60.0);
    if !ticks.is_finite() || ticks < 0.0 {
        return u64::MAX;
    }
    if ticks >= u64::MAX as f64 {
        return u64::MAX;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ticks = ticks.floor() as u64;
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 ①：⭐ 起点本身 ⇒ **0 Tick**。
    #[test]
    fn the_origin_maps_to_tick_zero() {
        assert_eq!(ticks_from_origin_us(1_234_567, 1_234_567, 60.0), 0);
    }

    /// 判据 ②：⭐ 60 BPM 下恰好 1 秒 ⇒ **960 Tick**（⭐ 即 PPQ 的口径被钉住）。
    #[test]
    fn one_second_is_exactly_one_quarter_note_of_ticks() {
        let origin = 10_000;
        assert_eq!(ticks_from_origin_us(origin + 1_000_000, origin, 60.0), 960);
    }

    /// 判据 ③：⭐ 半秒 ⇒ **480 Tick**（⭐ 向下取整，⭐ 且**可判定**而非"接近"）。
    #[test]
    fn half_a_second_is_half_a_quarter_note_of_ticks() {
        let origin = 0;
        assert_eq!(ticks_from_origin_us(500_000, origin, 60.0), 480);
    }

    /// 判据 ④：⭐ **`now_us < origin_us` ⇒ 0、⭐ ⛔ 不 panic**（⭐ 饱和而非下溢）。
    #[test]
    fn a_timestamp_before_the_origin_saturates_to_zero_instead_of_panicking() {
        assert_eq!(ticks_from_origin_us(999, 1_000, 60.0), 0);
        assert_eq!(ticks_from_origin_us(0, u64::MAX, 60.0), 0);
    }

    /// 判据 ⑤：⭐ **单调不减**（⭐ 时间递增 ⇒ Tick 不倒退）＋ ⭐ **反向对照**：
    /// 序列**确实**在推进（⛔ 不是"全 0 也算单调"）。
    #[test]
    fn the_mapping_is_monotonic_and_actually_advances() {
        let origin = 5_000;
        let ticks: Vec<u64> = (0..8)
            .map(|step| ticks_from_origin_us(origin + step * 125_000, origin, 60.0))
            .collect();
        assert_eq!(
            ticks.len(),
            8,
            "8 个采样点（少于 2 个时 `windows(2)` 恒真）"
        );
        assert!(ticks.windows(2).all(|pair| pair[0] <= pair[1]), "{ticks:?}");
        assert!(ticks[7] > 0, "序列必须真的推进：{ticks:?}");
        assert_eq!(ticks[7], 840, "7 × 120 Tick（每 125 ms = 1/8 拍）");
    }

    /// 判据 ⑥：⭐ **相同的 `(now, origin)` 差值恒得同一 Tick**（⭐ 只依赖差值、
    /// ⛔ 不依赖绝对时刻）⇒ ⭐ 保证"⭐ 基准显式化"确实成立。
    #[test]
    fn the_result_depends_only_on_the_difference() {
        for (now, origin) in [(1_000_000_u64, 0_u64), (7_500_000, 6_500_000)] {
            assert_eq!(
                ticks_from_origin_us(now, origin, 60.0),
                ticks_from_origin_us(1_000_000, 0, 60.0),
                "差值相同必须同值：(now={now}, origin={origin})"
            );
        }
    }

    /// 判据 ⑦（审计 #17 回归）：⭐ **120 BPM 下按速度手算** —— 0.5 秒 = 960 Tick、
    /// 1 秒 = 1920 Tick（⭐ 写死 60 BPM 时会给出 480 / 960，落点缩水 50%）。
    #[test]
    fn at_120_bpm_the_ticks_follow_the_tempo_not_the_old_constant() {
        assert_eq!(ticks_from_origin_us(500_000, 0, 120.0), 960);
        assert_eq!(ticks_from_origin_us(1_000_000, 0, 120.0), 1_920);
        assert_eq!(ticks_from_origin_us(250_000, 0, 120.0), 480);
        assert_eq!(ticks_from_origin_us(1_000_000, 0, 90.0), 1_440);
    }

    /// 判据 ⑧（审计 #17 兼容性承诺）：⭐ 60 BPM 下与**旧整数算式**
    /// `elapsed × 960 / 1_000_000` 逐位相同（全域逐点对账）。
    #[test]
    fn at_60_bpm_the_result_is_bit_identical_to_the_legacy_integer_formula() {
        for elapsed in 0..=1_000_000_u64 {
            assert_eq!(
                ticks_from_origin_us(elapsed, 0, 60.0),
                elapsed * 960 / 1_000_000,
                "60 BPM 逐位对账失败：elapsed={elapsed}"
            );
        }
    }

    /// 判据 ⑨：⭐ **病态速度响亮地错**（`u64::MAX`），⛔ 不回退到 60 BPM。
    #[test]
    fn a_degenerate_bpm_saturates_loudly_instead_of_falling_back() {
        assert_eq!(ticks_from_origin_us(1_000_000, 0, 0.0), u64::MAX);
        assert_eq!(ticks_from_origin_us(1_000_000, 0, -120.0), u64::MAX);
        assert_eq!(ticks_from_origin_us(1_000_000, 0, f64::NAN), u64::MAX);
        assert_eq!(ticks_from_origin_us(1_000_000, 0, f64::INFINITY), u64::MAX);
    }
}
