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
//! ⭐ 本文件目前只有**可本机判定**的那一层：[`ticks_from_origin_us`]（⭐ **纯函数** ⇒
//! 无声卡也能判）。⚠ **尚未接线** —— ⭐ 依 **D8**：⭐ 根 `Cargo.toml` 登记 `midir` ⇒
//! 重生成 `docs/ledger/dependency-licenses.md` ⇒ ⭐ **CI 的 `deny` 作业绿** ⇒ 才可写
//! `midir` 的真实建流与回调接线。
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
/// ⭐ **不 panic**：⭐ `now_us < origin_us`（⭐ 例：⭐ 时间戳回绕或起点晚于事件）⇒
/// ⭐ 返回 **0**（⭐ 饱和），⭐ ⛔ 不触发减法下溢。
///
/// [`MODEL-AST-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
#[must_use]
pub fn ticks_from_origin_us(now_us: u64, origin_us: u64) -> u64 {
    let elapsed = now_us.saturating_sub(origin_us);
    elapsed.saturating_mul(u64::from(PPQ)) / MICROS_PER_SECOND
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 ①：⭐ 起点本身 ⇒ **0 Tick**。
    #[test]
    fn the_origin_maps_to_tick_zero() {
        assert_eq!(ticks_from_origin_us(1_234_567, 1_234_567), 0);
    }

    /// 判据 ②：⭐ 恰好 1 秒 ⇒ **960 Tick**（⭐ 即 PPQ 的口径被钉住）。
    #[test]
    fn one_second_is_exactly_one_quarter_note_of_ticks() {
        let origin = 10_000;
        assert_eq!(ticks_from_origin_us(origin + 1_000_000, origin), 960);
    }

    /// 判据 ③：⭐ 半秒 ⇒ **480 Tick**（⭐ 向下取整，⭐ 且**可判定**而非"接近"）。
    #[test]
    fn half_a_second_is_half_a_quarter_note_of_ticks() {
        let origin = 0;
        assert_eq!(ticks_from_origin_us(500_000, origin), 480);
    }

    /// 判据 ④：⭐ **`now_us < origin_us` ⇒ 0、⭐ ⛔ 不 panic**（⭐ 饱和而非下溢）。
    #[test]
    fn a_timestamp_before_the_origin_saturates_to_zero_instead_of_panicking() {
        assert_eq!(ticks_from_origin_us(999, 1_000), 0);
        assert_eq!(ticks_from_origin_us(0, u64::MAX), 0);
    }

    /// 判据 ⑤：⭐ **单调不减**（⭐ 时间递增 ⇒ Tick 不倒退）＋ ⭐ **反向对照**：
    /// 序列**确实**在推进（⛔ 不是"全 0 也算单调"）。
    #[test]
    fn the_mapping_is_monotonic_and_actually_advances() {
        let origin = 5_000;
        let ticks: Vec<u64> = (0..8)
            .map(|step| ticks_from_origin_us(origin + step * 125_000, origin))
            .collect();
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
                ticks_from_origin_us(now, origin),
                ticks_from_origin_us(1_000_000, 0),
                "差值相同必须同值：(now={now}, origin={origin})"
            );
        }
    }
}
