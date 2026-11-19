//! 磁带饱和（链级饱和器；设计文档 `docs/adr/ADR-0062`，状态 `Proposed`）。
//!
//! ## 为什么是链级
//!
//! `drive` / `filter_drive` 是 `InternalInstrument` 的滤波参数，由引擎音源投影消费，
//! 采样器轨不走该投影 ⇒ `drive` 对采样器轨无效（Nightfall 采样版 README 的缺口）。
//! 本级跑在插入链里（通道条之后、Freeverb 之前），所有源（波表、采样、音频片段）
//! 都经过它 ⇒ 饱和覆盖采样器轨。
//!
//! ## 信号模型
//!
//! ```text
//! y_sat = [ pade_tanh(g · (x + asym)) / g − pade_tanh(g · asym) / g ] / pade_tanh'(g · asym) ，其中 g = 1 + 9 · drive
//! y     = (1 - mix) · x + mix · y_sat ，再乘 tape_trim_db 的线性增益
//! ```
//!
//! - 曲线复用 [`crate::math::pade_tanh`]（与滤波驱动同款，不另发明；同一事实只写一处）。
//! - 小信号增益 ≈ 1：`drive` 只改谐波，不改电平（判据钉住）。
//! - 湿路径先过一阶低通（`tape_hf_hz`，模拟磁带高频损失），干路径不滚降；
//!   `tape_hf_hz ≥ 20000` 时滤波器旁通（恒等）。
//! - 2x 过采样往返**固定**（[`crate::oversample::Oversampler2x`]，v1 不设开关）：
//!   饱和谐波在 2x 域产生、下采样低通滤掉 Nyquist 之外的部分 ⇒ 混叠低于
//!   无过采样的直算（判据钉住方向，不编门限数字）。
//! - 磁带磁滞模型 v1 不做（列 v2）。
//!
//! ## ⭐ 偶次谐波（`tape_asym`）—— 为什么要平移曲线
//!
//! `pade_tanh` 是**奇函数**（`f(−x) = −f(x)`）⇒ 它的展开只有奇次项
//! （3f / 5f / …），**没有任何偶次分量**。而"磁带暖"的听感来源正是偶次
//! （2f 为主）：奇次加多了只会更"硬"、更像过载。这是 `ADR-0062` v1 的一个
//! 如实登记的局限（"饱和从离线链搬进工程"时丢掉了那 5% 偶次）。
//!
//! 补法是**把曲线沿 x 轴平移**（教科书做法，⛔ 不新增任何状态）：
//!
//! ```text
//! y_sat = pade_tanh(g · (x + asym)) / g − pade_tanh(g · asym) / g
//!         └──────────── 平移后的曲线 ────────────┘   └──── 常量直流补偿 ────┘
//!         整个结果再 ÷ pade_tanh'(g · asym)   ← 工作点斜率归一化（`#347`）
//! ```
//!
//! * 平移后正负半周**不再对称** ⇒ 二次谐波出现（`asym` 越大越强）；
//! * **减掉常量**是必须的：不减的话 `x = 0` 输出 `pade_tanh(g·asym)/g ≠ 0`
//!   ⇒ 整条链带上 DC 偏移（下游限幅器会为它让出余量，听感是"低频被抬"）。
//!   减掉之后**零输入 ⇒ 零输出**（判据 `asym_keeps_silence_silent`）。
//! * ⭐ **还要按工作点斜率归一化**（`#347`）：减常量只把 `x = 0` 的输出拉回 `0`，
//!   **没有**把小信号**增益**拉回 `1`。平移把工作点从 `0`（斜率恰 `1`）搬到
//!   `g·asym`，那里 `pade_tanh' < 1` ⇒ 整条湿路被静默压低。实测：
//!   `drive = 0.20`、`asym = 0.32`（`g·asym = 0.896`，斜率 `0.517`）是 **−5.7 dB**；
//!   把整首 183 s 的作品走一遍母带 LUFS，就是 **−25.33 → −19.86 LUFS**（差 5.47 dB）。
//!   ⇒ 除以 `pade_tanh'(g·asym)`（[`crate::math::pade_tanh_slope`]），小信号增益
//!   恒 `1`，`tape_drive` / `tape_asym` 才是真"只改谐波"、`tape_trim_db` 才只是
//!   作者的手动配平。
//!   - `asym == 0` ⇒ 斜率恰 `1.0` ⇒ `÷ 1.0` 在 binary32 下逐位恒等（后向兼容）。
//!   - 斜率下界 [`MIN_ASYMMETRY_SLOPE`]（`1/8`）⇒ 归一化最多 `+18 dB`；
//!     `drive → 1`、`asym → 0.5` 时曲线被压平，那已不是"配平"而是"改动态"。
//! * ⚠ `asym` **只在级建成时起作用**（`drive > 0` 且 `mix > 0`，见下一节）——
//!   曲线是 `tanh(g·x)`，`g` 只由 `drive` 决定 ⇒ `drive = 0` 时级本来就不建，
//!   `asym` 写了也不生效。这是既有建成条件的直接后果，⛔ 不为它开口子。
//!
//! ## ⭐ wow / flutter（音高调制段，鼓机缺口 P7）
//!
//! 真磁带机的走带速度不是常数：转盘偏心 ⇒ **wow**（慢，0.05–2 Hz），
//! 电机与压带轮的高频抖动 ⇒ **flutter**（快，2–15 Hz）。两者都是**延迟调制**：
//! 信号经过一台速度在晃的机器，等价于经过一条长度在晃的延迟线
//! ⇒ 瞬时音高跟着晃。本段就是这条延迟线：
//!
//! ```text
//! τ(t) = τc + Dw·(0.75·sin(2π·fw·t + φw) + 0.25·drift(t)) + Df·sin(2π·ff·t + φf)
//! τc   = Dw + Df + 0.5 ms（中心延迟，保证 τ(t) ≥ 0.5 ms > 0）
//! ```
//!
//! * **深度单位是 ms（延迟峰值偏离）**，不是音分：音高偏离由 `深度 × 速率`
//!   决定（`ε ≈ 2π·f·D`），把深度钉在延迟域里，判据才能直接量延迟
//!   （互相关峰位）而不必从音高反推。
//! * **两声道共用同一条调制**（同一个走带机构）：写钟、帧计数、调制值
//!   两声道完全一致 ⇒ 立体声像不晃，只有音高晃。
//! * **种子可复现（硬规则）**：相位 `φw` / `φf` 与漂移 `drift(t)` 全部是
//!   `(seed, 帧号)` 的纯函数（splitmix32 哈希），**没有运行期 PRNG 状态**
//!   ⇒ 同 seed 同输出逐位成立；`reset()` 只需把帧计数归零；块边界改 seed
//!   等价于按新种子重播种（与 `noise_seed` 同口径）。
//! * **深度全 0 ⇒ 调制段不存在**：`process` 根本不碰延迟线，输出与本段
//!   接入之前**逐位一致**（判据 `modulation_depth_zero_is_bit_identical`），
//!   也不引入任何延迟。任一深度 > 0 ⇒ 延迟线接入，级延迟增加
//!   [`TapeSaturation::modulation_latency_samples`]（名义值 = 中心延迟）。
//! * 调制段位于**饱和之后**（整条级是一个"磁带机"，干湿混合后的信号
//!   一起过走带机构）；深度与 `mix` 无关——`mix < 1` 时干声同样被调制，
//!   这是走带物理，不是合唱效果器。
//!
//! ## 级建成条件（与运行态同口径）
//!
//! `tape_drive > 0` **且** `tape_mix > 0`（两者任一为缺省 ⇒ 不建器件，
//! 不另设 `tape_enabled` 开关 —— 沿压缩 `#303` 口径“写了非缺省参数 ⇒ 级执行”）。
//! [`TapeSaturation::is_active`] 就是这条判定（读的是 `sanitised()` 后的值）。
//!
//! ## 延迟与 PDC
//!
//! 本级执行时引入 [`crate::oversample::OS_LATENCY`]（31 样本 @ 任何采样率，
//! 单位是基础采样率样本）⇒ 设备作者须把该值计入设备的
//! `latency_samples` 声明（[`TapeSaturation::latency_samples`] 即此数；
//! 未武装时级不执行、延迟为 0）。模型层 `latency_samples` 是唯一事实源
//! [ARCH-PDC-001]，本 crate 不替模型层推导它。
//! wow/flutter 武装时**再加** [`TapeSaturation::modulation_latency_samples`]
//! （中心延迟，随采样率变）——同一条声明纪律：本 crate 给出读数，
//! 计入声明是设备作者的事。
//!
//! ## 实时安全
//!
//! - 全部状态是定长数组（过采样历史、滤波器状态），构造期确定；
//!   [`TapeSaturation::process`] 的暂存（上采样缓冲、下采样暂存）**由调用方提供**
//!   （[`Oversampler2x::process_round_trip`] 同款）⇒ 逐块零分配、零锁、零 I/O。
//! - 入口守卫：非有限样本就地回落成 `0.0`（过采样滤波器有记忆，`NaN` 会污染记忆，
//!   与 `delay.rs` 同一条理由）；有限样本逐位不变。
//! - `configure` 是**唯一**的状态初始化入口（采样率＋清零），必须在音频回调之外
//!   调用一次；未 `configure` 时 [`TapeSaturation::process`] 是直通（不改变缓冲）。

use crate::math::{db_to_gain, finite_or_zero, pade_tanh, pade_tanh_slope, sanitise_sample_rate};

/// 工作点斜率的下界（`#347`）：归一化增益最多 `+18 dB`。
///
/// `g · asym` 在合法域内最大到 `10 · 0.5 = 5`，闭式斜率 `0.036` ⇒ 不钳的话
/// 归一化会要 `27.5×`（+28.8 dB）——那已经不是"配平电平"而是"把曲线压平"。
const MIN_ASYMMETRY_SLOPE: f32 = 1.0 / 8.0;
use crate::oversample::{OS_LATENCY, OS_TAPS, Oversampler2x};

/// `tape_trim_db` 的合法域（dB）。
pub const TAPE_TRIM_MIN_DB: f32 = -24.0;
/// `tape_trim_db` 的合法域（dB）。
pub const TAPE_TRIM_MAX_DB: f32 = 24.0;
/// `tape_hf_hz` 的合法域（Hz）。
pub const TAPE_HF_MIN_HZ: f32 = 2000.0;
/// `tape_hf_hz` 的合法域（Hz）；≥ 该值时湿路低通旁通（恒等）。
pub const TAPE_HF_MAX_HZ: f32 = 20_000.0;
/// `tape_asym` 的合法域（无量纲）。`0` ＝ 对称（纯奇次谐波）。
///
/// 上界取 `0.5`：`pade_tanh` 的自变量是 `g · (x + asym)` 而 `g = 1 + 9 · drive`
/// ⇒ `drive = 1` 时工作点已经推到 `5.0`（`pade_tanh(5) ≈ 1.03`），再大就不是
/// "磁带"而是硬削波了。
pub const TAPE_ASYM_MAX: f32 = 0.5;

/// `tape_wow_rate_hz` 的合法域（Hz）：走带偏心的转速量级。
pub const TAPE_WOW_RATE_MIN_HZ: f32 = 0.05;
/// `tape_wow_rate_hz` 的合法域（Hz）。
pub const TAPE_WOW_RATE_MAX_HZ: f32 = 2.0;
/// `tape_wow_depth_ms` 的合法域（ms）：延迟峰值偏离。
///
/// 上界 8 ms 的出处：wow 音高偏离 `ε ≈ 2π·f·D`，在速率上界 2 Hz 处
/// 8 ms 已是 ±10% 的音高晃动——那是坏机器，不是乐器。再大不给写。
pub const TAPE_WOW_DEPTH_MAX_MS: f32 = 8.0;
/// `tape_flutter_rate_hz` 的合法域（Hz）：电机抖动的量级。
pub const TAPE_FLUTTER_RATE_MIN_HZ: f32 = 2.0;
/// `tape_flutter_rate_hz` 的合法域（Hz）。
pub const TAPE_FLUTTER_RATE_MAX_HZ: f32 = 15.0;
/// `tape_flutter_depth_ms` 的合法域（ms）：延迟峰值偏离。
///
/// flutter 的音高偏离同样是 `2π·f·D`：15 Hz 处 2 ms 已是 ±19%，
/// 远超任何真实机器的 flutter（规格书量级 0.1% 以下 ⇒ 微秒级深度就够），
/// 2 ms 是给"效果化"留的顶，不是写实值。
pub const TAPE_FLUTTER_DEPTH_MAX_MS: f32 = 2.0;
/// `tape_seed` 的合法域（整数，与 `noise_seed` 同口径）。
pub const TAPE_SEED_MAX: f32 = 16_777_215.0;

/// wow/flutter 延迟线长度（样本 / 声道，环形缓冲）。
///
/// 需求侧：中心延迟最大 `(8 + 2 + 0.5) ms`；`8192` 样本在 384 kHz 下有
/// 21 ms、768 kHz 下有 10.7 ms ⇒ 任何现实采样率下都装得下最大中心延迟。
/// 读延迟在 `apply_modulation` 里另有钳制兜底（见彼处注释）。
const MOD_BUFFER_LEN: usize = 8_192;

/// 调制中心延迟的安全边距（ms）：保证调制摆幅最大时延迟仍 ≥ 此值，
/// 线性插值的两个读点永远落在已写入的历史里。
const MOD_MARGIN_MS: f32 = 0.5;

/// splitmix32 终结器：把 `(seed, index)` 压成均匀分布的 `u32`。
///
/// 调制的全部"随机性"（相位、漂移端点）都由它从 `(seed, 帧号/段号)`
/// 现场导出 ⇒ 没有运行期 PRNG 状态需要保存或复位，同 seed 同输出
/// 是构造性质，不是纪律（与 `noise` 的 `Rng` 同一哈希家族，不另发明）。
fn seed_hash(seed: u32, index: u32) -> u32 {
    let mut z = seed
        .wrapping_add(index.wrapping_mul(0x9E37_79B9))
        .wrapping_add(0x9E37_79B9);
    z = (z ^ (z >> 16)).wrapping_mul(0x21F0_AAAD);
    z = (z ^ (z >> 15)).wrapping_mul(0x735A_2D97);
    z ^ (z >> 15)
}

/// `(seed, index)` → `[0, 1)` 的均匀值（取哈希高 24 位 ⇒ binary32 可精确表示）。
fn seed_unit(seed: u32, index: u32) -> f32 {
    (seed_hash(seed, index) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// `(seed, segment)` → `[−1, 1]` 的漂移端点值（盐与相位错开，互不相关）。
fn seed_signed(seed: u32, segment: u32) -> f32 {
    seed_unit(seed ^ 0xD1B5_4A32, segment) * 2.0 - 1.0
}

/// 磁带饱和参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapeParams {
    /// 饱和驱动（0..1）。`0` = 不饱和（级不建，见模块文档）。
    pub drive: f32,
    /// 干湿比（0..1）。`0` = 全干（级不建，见模块文档）。
    pub mix: f32,
    /// 输出补偿增益（dB，−24..+24）。
    pub trim_db: f32,
    /// 湿路低通截止频率（Hz，2000..20000；≥ 20000 旁通）。
    pub hf_hz: f32,
    /// ⭐ **曲线不对称量**（`tape_asym`，0..=0.5）。`0` ＝ 对称（纯奇次谐波）。
    ///
    /// 磁带"暖"的那一半是**偶次**谐波，而 `pade_tanh` 是**奇函数** ⇒ 只出奇次
    /// （3f / 5f / …），听感偏硬。本参数把曲线沿 x 轴平移（见模块文档的
    /// "⭐ 偶次谐波"一节）：平移后正负半周不再对称 ⇒ 二次谐波出现。
    ///
    /// ⚠ `0` ⇒ **逐位等于接线前的公式**（补偿项恰 `0.0`、加法是恒等）。
    pub asym: f32,
    /// wow 速率（Hz，0.05..=2）。深度为 0 时不生效。
    pub wow_rate_hz: f32,
    /// ⭐ **wow 深度**（ms，0..=8）：延迟线长度的慢速峰值偏离。
    ///
    /// `0`（缺省）⇒ wow 分量不存在；wow 与 flutter 深度**都**为 0 时
    /// 整段调制不存在、输出逐位等于接入前（见模块文档「wow / flutter」节）。
    pub wow_depth_ms: f32,
    /// flutter 速率（Hz，2..=15）。深度为 0 时不生效。
    pub flutter_rate_hz: f32,
    /// ⭐ **flutter 深度**（ms，0..=2）：延迟线长度的快速峰值偏离。
    pub flutter_depth_ms: f32,
    /// 调制种子（整数，0..=2²⁴−1）：wow/flutter 相位与随机漂移的唯一来源。
    ///
    /// 相位与漂移都是 `(seed, 帧号)` 的纯函数（无运行期 PRNG 状态）
    /// ⇒ 同 seed 同输出逐位成立；与 `noise_seed` 同口径。
    pub seed: f32,
}

impl TapeParams {
    /// 缺省：全干、无驱动、无补偿、湿路低通旁通 ⇒ 级不建（恒等）。
    /// 调制深度全 0 ⇒ 调制段不存在；速率取典型走带值，作者只写深度即出声。
    pub const DEFAULT: Self = Self {
        drive: 0.0,
        mix: 0.0,
        trim_db: 0.0,
        hf_hz: TAPE_HF_MAX_HZ,
        asym: 0.0,
        wow_rate_hz: 0.5,
        wow_depth_ms: 0.0,
        flutter_rate_hz: 6.0,
        flutter_depth_ms: 0.0,
        seed: 0.0,
    };

    /// 缺省参数。
    #[must_use]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// 钳制表（与 [`TapeSaturation::set_params`] 同一次；判据钉死）。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | 非有限 |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `drive` / `mix` | `[0, 1]` | `0` | `1` | **`0`**（下界） |
    /// | `trim_db` | `[−24, +24]` | 下界 | 上界 | **下界** |
    /// | `hf_hz` | `[2000, 20000]` | 下界 | 上界 | **下界** |
    /// | `asym` | `[0, 0.5]` | `0` | 上界 | **`0`**（下界 ⇒ 对称，恒等） |
    /// | `wow_rate_hz` | `[0.05, 2]` | 下界 | 上界 | **下界** |
    /// | `wow_depth_ms` | `[0, 8]` | `0` | 上界 | **`0`**（下界 ⇒ 无调制，恒等） |
    /// | `flutter_rate_hz` | `[2, 15]` | 下界 | 上界 | **下界** |
    /// | `flutter_depth_ms` | `[0, 2]` | `0` | 上界 | **`0`**（下界 ⇒ 无调制，恒等） |
    /// | `seed` | `[0, 2²⁴−1]` 整数 | `0` | 上界 | **`0`**；有限值先钳后取整 |
    ///
    /// 非有限一律归下界（沿 [`crate::compressor::CompressorParams::sanitised`] 的
    /// 口径：`f32::clamp` 对 `NaN` 是恒等，单靠 `clamp` 会漏一个 `NaN` 进实时路径）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        let finite_or = |value: f32, low: f32, high: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                low
            }
        };
        Self {
            drive: finite_or(self.drive, 0.0, 1.0),
            mix: finite_or(self.mix, 0.0, 1.0),
            trim_db: finite_or(self.trim_db, TAPE_TRIM_MIN_DB, TAPE_TRIM_MAX_DB),
            hf_hz: finite_or(self.hf_hz, TAPE_HF_MIN_HZ, TAPE_HF_MAX_HZ),
            asym: finite_or(self.asym, 0.0, TAPE_ASYM_MAX),
            wow_rate_hz: finite_or(self.wow_rate_hz, TAPE_WOW_RATE_MIN_HZ, TAPE_WOW_RATE_MAX_HZ),
            wow_depth_ms: finite_or(self.wow_depth_ms, 0.0, TAPE_WOW_DEPTH_MAX_MS),
            flutter_rate_hz: finite_or(
                self.flutter_rate_hz,
                TAPE_FLUTTER_RATE_MIN_HZ,
                TAPE_FLUTTER_RATE_MAX_HZ,
            ),
            flutter_depth_ms: finite_or(self.flutter_depth_ms, 0.0, TAPE_FLUTTER_DEPTH_MAX_MS),
            seed: finite_or(self.seed, 0.0, TAPE_SEED_MAX).floor(),
        }
    }
}

impl Default for TapeParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 磁带饱和器（立体声，两声道各一台过采样器＋一阶湿路低通）。
///
/// 构造期不分配（状态全是定长数组）；`process` 的暂存由调用方提供。
pub struct TapeSaturation {
    /// 已 `sanitised` 的参数（`set_params` 的唯一入口保证）。
    params: TapeParams,
    oversampler_l: Oversampler2x,
    oversampler_r: Oversampler2x,
    /// 湿路一阶低通状态，每声道一个。
    lp_state: [f32; 2],
    /// 湿路低通系数（`1 − exp(−2π·fc/fs)`）；旁通时不用它。
    lp_coeff: f32,
    /// 湿路低通是否旁通（`hf_hz ≥ 20000`）。
    wet_bypass: bool,
    sample_rate: f32,
    ready: bool,
    /// wow/flutter 调制延迟线（每声道一条环形缓冲）。
    ///
    /// 只在任一调制深度 > 0 时才读写；深度全 0 时 `process` 根本不碰它
    /// ⇒ 输出逐位等于调制段接入之前。两声道共用 [`Self::delay_pos`] 写钟
    /// 与同一条调制曲线（同一台走带机构，见模块文档）。
    delay_line: [[f32; MOD_BUFFER_LEN]; 2],
    /// 延迟线写位置（两声道共用）。
    delay_pos: usize,
    /// 自 `configure` / `reset` 起的帧计数：调制相位与漂移的唯一时间基。
    ///
    /// 相位是 `(seed, frame)` 的纯函数 ⇒ 帧计数归零即回到与全新实例
    /// 完全相同的调制轨迹（判据 `reset_reproduces_…` 的调制版钉住）。
    frame: u64,
}

impl TapeSaturation {
    /// 构造一个**未配置**的实例（直通）。采样率在 [`Self::configure`] 里定。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            params: TapeParams::DEFAULT,
            oversampler_l: Oversampler2x::new(),
            oversampler_r: Oversampler2x::new(),
            lp_state: [0.0; 2],
            lp_coeff: 0.0,
            wet_bypass: true,
            sample_rate: 0.0,
            ready: false,
            delay_line: [[0.0; MOD_BUFFER_LEN]; 2],
            delay_pos: 0,
            frame: 0,
        }
    }

    /// 按 `sample_rate` 定采样率并清零全部时间状态（过采样历史、滤波器状态）。
    ///
    /// **这是本类型唯一的状态初始化入口**，必须在音频回调之外调用一次。
    pub fn configure(&mut self, sample_rate: f32) {
        self.sample_rate = sanitise_sample_rate(sample_rate);
        self.reset();
        self.refresh_filter();
        self.ready = true;
    }

    /// 是否已经过 [`Self::configure`]。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.ready
    }

    /// 写参数（`sanitised()` 后存入，并重算湿路低通系数；**不清时间状态**，
    /// 与 `Reverb::set_params` / `ChannelStrip::set_params` 同一条纪律 ⇒
    /// 块边界自动化改参数不会切断饱和尾巴）。
    pub fn set_params(&mut self, params: TapeParams) {
        self.params = params.sanitised();
        self.refresh_filter();
    }

    /// 当前参数（已 `sanitised`）。
    #[must_use]
    pub const fn params(&self) -> TapeParams {
        self.params
    }

    /// 本级是否执行：`drive > 0` 且 `mix > 0`（读的是已 `sanitised` 的值）。
    ///
    /// 任一为缺省 ⇒ 运行态不建本级（恒等），与插入链的建成条件同一条判定。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.params.drive > 0.0 && self.params.mix > 0.0
    }

    /// 本级执行时引入的处理延迟（帧），恒为 [`OS_LATENCY`] [ARCH-PDC-001]。
    ///
    /// 未武装时级不执行、延迟为 0；设备作者按“武装时声明此数”的口径把该值
    /// 计入设备的 `latency_samples`（见模块文档“延迟与 PDC”）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        OS_LATENCY
    }

    /// wow/flutter 调制是否武装：任一深度 > 0（读的是已 `sanitised` 的值）。
    ///
    /// 两深度全 0 ⇒ 调制段不存在：`process` 不碰延迟线、不引入延迟，
    /// 输出逐位等于调制段接入之前。
    #[must_use]
    pub fn modulation_active(&self) -> bool {
        self.params.wow_depth_ms > 0.0 || self.params.flutter_depth_ms > 0.0
    }

    /// 调制中心延迟（ms）：两深度之和 + 安全边距；未武装时为 0。
    ///
    /// 调制摆幅的最大值恰是两深度之和（wow 分量 `0.75·sin + 0.25·drift`
    /// 的值域是 `[−1, 1]`）⇒ 实际延迟恒落在
    /// `[MOD_MARGIN_MS, 2·(Dw + Df) + MOD_MARGIN_MS]` ms 内、永不为负。
    fn modulation_center_ms(&self) -> f32 {
        if self.modulation_active() {
            self.params.wow_depth_ms + self.params.flutter_depth_ms + MOD_MARGIN_MS
        } else {
            0.0
        }
    }

    /// 调制段引入的延迟（帧，名义值 = 中心延迟按当前采样率换算、四舍五入）。
    ///
    /// 未武装或未 `configure` 时为 0。设备作者按与 [`Self::latency_samples`]
    /// 同一条纪律把武装时的此数计入 `latency_samples` 声明（见模块文档
    /// 「延迟与 PDC」）——实际延迟在中心值 ±（两深度之和）内摆动，
    /// PDC 只能补名义值，这与一切 LFO 调制延迟的宿命相同。
    #[must_use]
    pub fn modulation_latency_samples(&self) -> usize {
        if self.sample_rate > 0.0 {
            (self.modulation_center_ms() * self.sample_rate / 1000.0).round() as usize
        } else {
            0
        }
    }

    /// 第 `frame` 帧的调制偏离（ms，可正可负）。
    ///
    /// 全部输入是 `(params.seed, frame)` 的纯函数（splitmix32 哈希，
    /// 见 [`seed_unit`]）：无运行期 PRNG 状态 ⇒ 同 seed 同轨迹逐位成立，
    /// `reset()` 把帧计数归零即回到轨迹起点，块边界改 seed 等价于重播种。
    fn modulation_offset_ms(&self, frame: u64) -> f32 {
        let params = &self.params;
        let seed = params.seed as u32;
        let t = frame as f32 / self.sample_rate;
        let tau = core::f32::consts::TAU;
        // wow：正弦（走带偏心）+ 慢速随机漂移（轴承/皮带的不规则），
        // 权重 0.75 / 0.25 ⇒ 合成值域仍是 [−1, 1]，深度语义不被漂移偷走。
        let wow = if params.wow_depth_ms > 0.0 {
            let phase = tau * params.wow_rate_hz * t + tau * seed_unit(seed, 1);
            // 漂移：以一个 wow 周期为段长，段端点取种子哈希值（∈ [−1, 1]），
            // 段内 smoothstep 插值 ⇒ 处处连续、无台阶。
            let period_frames = (self.sample_rate / params.wow_rate_hz).max(1.0);
            let pos = frame as f32 / period_frames;
            let segment = pos.floor();
            let frac = pos - segment;
            let smooth = frac * frac * (3.0 - 2.0 * frac);
            let a = seed_signed(seed, segment as u32);
            let b = seed_signed(seed, (segment as u32).wrapping_add(1));
            let drift = a + (b - a) * smooth;
            params.wow_depth_ms * (0.75 * libm::sinf(phase) + 0.25 * drift)
        } else {
            0.0
        };
        let flutter = if params.flutter_depth_ms > 0.0 {
            let phase = tau * params.flutter_rate_hz * t + tau * seed_unit(seed, 2);
            params.flutter_depth_ms * libm::sinf(phase)
        } else {
            0.0
        };
        wow + flutter
    }

    /// 从延迟线读 `delay` 样本之前的样本（线性插值；`delay ≥ 0`）。
    ///
    /// 调用方保证 `delay ≤ MOD_BUFFER_LEN − 2`（`apply_modulation` 钳过）。
    fn read_delayed(&self, channel: usize, delay: f32) -> f32 {
        let whole = delay.floor();
        let frac = delay - whole;
        let back = whole as usize;
        let here = (self.delay_pos + MOD_BUFFER_LEN - back) % MOD_BUFFER_LEN;
        let older = (here + MOD_BUFFER_LEN - 1) % MOD_BUFFER_LEN;
        let a = self.delay_line[channel][here];
        let b = self.delay_line[channel][older];
        a + (b - a) * frac
    }

    /// 把一对已下采样的声道块送进调制延迟线（两声道同调制，见模块文档）。
    ///
    /// 逐帧先写后读：当前帧先入线、再按当帧延迟读回 ⇒ 延迟恒 ≥ 0 且
    /// 读点永远落在已写入的历史里（最小延迟 = [`MOD_MARGIN_MS`]）。
    fn apply_modulation(&mut self, left: &mut [f32], right: &mut [f32]) {
        let center = self.modulation_center_ms() * self.sample_rate / 1000.0;
        // 兜底钳制：中心延迟在合法采样率（≤ 768 kHz）下恒 < MOD_BUFFER_LEN − 2，
        // 此钳只对病态采样率生效，防的是环形缓冲回绕读错位，不是参数口径。
        let max_delay = (MOD_BUFFER_LEN - 2) as f32;
        for i in 0..left.len() {
            self.delay_line[0][self.delay_pos] = left[i];
            self.delay_line[1][self.delay_pos] = right[i];
            let offset = self.modulation_offset_ms(self.frame) * self.sample_rate / 1000.0;
            let delay = (center + offset).clamp(0.0, max_delay);
            left[i] = self.read_delayed(0, delay);
            right[i] = self.read_delayed(1, delay);
            self.delay_pos = (self.delay_pos + 1) % MOD_BUFFER_LEN;
            self.frame += 1;
        }
    }

    /// 清空全部时间状态（过采样历史、滤波器状态、调制延迟线与帧计数），
    /// 不碰参数与采样率。
    ///
    /// 复位后的实例与一台刚 [`Self::configure`] 过、同参数的实例在逐帧输出上
    /// **逐位相同**（判据 `reset_reproduces_a_freshly_built_device_bit_for_bit`）。
    pub fn reset(&mut self) {
        self.oversampler_l.reset();
        self.oversampler_r.reset();
        self.lp_state = [0.0; 2];
        self.delay_line = [[0.0; MOD_BUFFER_LEN]; 2];
        self.delay_pos = 0;
        self.frame = 0;
    }

    /// 按当前参数重算湿路低通系数与旁通位（`configure` / `set_params` 共用）。
    fn refresh_filter(&mut self) {
        self.wet_bypass = self.params.hf_hz >= TAPE_HF_MAX_HZ;
        if self.sample_rate > 0.0 {
            self.lp_coeff = 1.0
                - libm::expf(-2.0 * core::f32::consts::PI * self.params.hf_hz / self.sample_rate);
        }
    }

    /// 对一个声道的上采样缓冲原地做饱和（湿路低通＋`pade_tanh`＋干湿＋补偿）。
    ///
    /// 入口守卫：非有限样本先回落成 `0.0`（再进滤波器记忆与过采样历史）。
    fn saturate_channel(&mut self, channel: usize, samples: &mut [f32]) {
        let g = 1.0 + 9.0 * self.params.drive;
        let asym = self.params.asym;
        // ⭐ **不对称的直流补偿**（见模块文档「⭐ 偶次谐波」）：曲线沿 x 轴平移后
        // `x = 0` 处不再过零 ⇒ 减掉 `pade_tanh(g·asym) / g` 这个**常量**把它拉回零。
        // ⚠ `asym == 0` ⇒ `pade_tanh(0.0)` 恰 `0.0` ⇒ 补偿项恰 `+0.0`，
        // 而 `a - 0.0` 在 binary32 下**逐位等于** `a`（含 `a == -0.0`）⇒ 逐位不变。
        let bias = pade_tanh(g * asym) / g;
        // ⭐ **工作点斜率归一化**（`#347`，见模块文档「⭐ 偶次谐波」末段）：
        // 平移把工作点从 `0`（`pade_tanh'(0) = 1`）搬到 `g·asym`，斜率随之降到
        // `pade_tanh'(g·asym)` ⇒ 小信号增益跟着掉（`drive 0.2 / asym 0.32` 实测
        // **−5.7 dB**）。这里把它除回去，`drive` / `asym` 才真的"只改谐波"，
        // `tape_trim_db` 才真的只做"作者的手动配平"。
        // 上界 `8.0`（+18 dB）：`drive → 1`、`asym → 0.5` 时 `g·asym = 5`，
        // 闭式斜率 `0.036` ⇒ 归一化会要 `27.5×`，那是"把曲线压平"而不是电平配平。
        let slope = pade_tanh_slope(g * asym).clamp(MIN_ASYMMETRY_SLOPE, 1.0);
        let normalise = 1.0 / slope;
        let trim = db_to_gain(self.params.trim_db);
        let mix = self.params.mix;
        let coeff = self.lp_coeff;
        let bypass = self.wet_bypass;
        let state = &mut self.lp_state[channel];
        for slot in samples.iter_mut() {
            let x = finite_or_zero(*slot);
            let wet = if bypass {
                x
            } else {
                *state += (x - *state) * coeff;
                *state
            };
            let y_sat = (pade_tanh(g * (wet + asym)) / g - bias) * normalise;
            *slot = ((1.0 - mix) * x + mix * y_sat) * trim;
        }
    }

    /// 原地处理一个立体声块（`left.len() == right.len()`，取两者较短者）。
    ///
    /// 暂存由调用方提供（[`Oversampler2x`] 的口径）：`upsampled` 至少 `2 * n` 长，
    /// `scratch` 至少 `OS_TAPS − 1 + 2 * n` 长（`n` = 块帧数）；长度不足时**什么都不做**
    /// （实时路径上宁可静默跳过也不可能展开栈，与 `Oversampler2x` 同一条纪律）。
    /// 未 [`Self::configure`] 时是直通（不改变缓冲）。
    ///
    /// 入口归零：先把 `left[..n]` / `right[..n]` 里的非有限样本就地置 0，**再**进
    /// 过采样——滤波器历史（`up_tail` / `down_tail`）永远见不到 NaN/Inf，否则脏块
    /// 之后的首个干净块前约 `OS_LATENCY` 帧会被错误地压成 0（上采样先于饱和，
    /// 饱和处的守卫拦不住已经进历史的 NaN）。饱和处的 `finite_or_zero` 仍保留，
    /// 作第二层（防上采样溢出产生的非有限值）。
    ///
    /// 调用方缓冲区即工作缓冲，归零后不再恢复——离线链与实时池传进来的都是
    /// 自有缓冲，不是工程持久数据。
    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        upsampled: &mut [f32],
        scratch: &mut [f32],
    ) {
        if !self.ready {
            return;
        }
        let n = left.len().min(right.len());
        if n == 0 || upsampled.len() < n * 2 || scratch.len() < OS_TAPS - 1 + n * 2 {
            return;
        }
        for sample in &mut left[..n] {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        for sample in &mut right[..n] {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        self.oversampler_l
            .upsample(&left[..n], &mut upsampled[..n * 2], scratch);
        self.saturate_channel(0, &mut upsampled[..n * 2]);
        self.oversampler_l
            .downsample(&upsampled[..n * 2], &mut left[..n], scratch);
        self.oversampler_r
            .upsample(&right[..n], &mut upsampled[..n * 2], scratch);
        self.saturate_channel(1, &mut upsampled[..n * 2]);
        self.oversampler_r
            .downsample(&upsampled[..n * 2], &mut right[..n], scratch);
        // ⭐ wow/flutter 调制段（饱和之后，整台"磁带机"一起晃）：
        // 深度全 0 时不进入 ⇒ 延迟线不被触碰、输出逐位等于接入之前。
        if self.modulation_active() {
            self.apply_modulation(&mut left[..n], &mut right[..n]);
        }
    }
}

impl Default for TapeSaturation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn make_tape(drive: f32, mix: f32, trim_db: f32, hf_hz: f32) -> TapeSaturation {
        make_tape_asym(drive, mix, trim_db, hf_hz, 0.0)
    }

    /// ⭐ 带 `tape_asym` 的同款构造（偶次谐波的判据用）。
    fn make_tape_asym(drive: f32, mix: f32, trim_db: f32, hf_hz: f32, asym: f32) -> TapeSaturation {
        let mut tape = TapeSaturation::new();
        tape.configure(SR);
        tape.set_params(TapeParams {
            drive,
            mix,
            trim_db,
            hf_hz,
            asym,
            ..TapeParams::DEFAULT
        });
        tape
    }

    /// 带 wow/flutter 调制段的同款构造（P7 判据用）。
    fn make_tape_mod(
        wow_rate_hz: f32,
        wow_depth_ms: f32,
        flutter_rate_hz: f32,
        flutter_depth_ms: f32,
        seed: f32,
    ) -> TapeSaturation {
        let mut tape = TapeSaturation::new();
        tape.configure(SR);
        tape.set_params(TapeParams {
            drive: 0.1,
            mix: 1.0,
            wow_rate_hz,
            wow_depth_ms,
            flutter_rate_hz,
            flutter_depth_ms,
            seed,
            ..TapeParams::DEFAULT
        });
        tape
    }

    /// 块暂存：上采样缓冲＋下采样暂存（`process` 的口径）。
    fn scratch_for(n: usize) -> (Vec<f32>, Vec<f32>) {
        (vec![0.0; n * 2], vec![0.0; OS_TAPS - 1 + n * 2])
    }

    fn process_all(tape: &mut TapeSaturation, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let (mut upsampled, mut scratch) = scratch_for(n);
        tape.process(left, right, &mut upsampled, &mut scratch);
    }

    #[test]
    fn sanitised_clamps_every_field_into_its_documented_domain() {
        for value in [
            f32::NAN,
            f32::NEG_INFINITY,
            f32::INFINITY,
            -1.0,
            0.0,
            0.5,
            1.0,
            2.0,
        ] {
            let params = TapeParams {
                drive: value,
                mix: value,
                trim_db: value,
                hf_hz: value,
                asym: value,
                wow_rate_hz: value,
                wow_depth_ms: value,
                flutter_rate_hz: value,
                flutter_depth_ms: value,
                seed: value,
            }
            .sanitised();
            assert!((0.0..=1.0).contains(&params.drive), "drive {value}");
            assert!((0.0..=1.0).contains(&params.mix), "mix {value}");
            assert!(
                (TAPE_TRIM_MIN_DB..=TAPE_TRIM_MAX_DB).contains(&params.trim_db),
                "trim_db {value}"
            );
            assert!(
                (TAPE_HF_MIN_HZ..=TAPE_HF_MAX_HZ).contains(&params.hf_hz),
                "hf_hz {value}"
            );
            assert!((0.0..=TAPE_ASYM_MAX).contains(&params.asym), "asym {value}");
            assert!(
                (TAPE_WOW_RATE_MIN_HZ..=TAPE_WOW_RATE_MAX_HZ).contains(&params.wow_rate_hz),
                "wow_rate_hz {value}"
            );
            assert!(
                (0.0..=TAPE_WOW_DEPTH_MAX_MS).contains(&params.wow_depth_ms),
                "wow_depth_ms {value}"
            );
            assert!(
                (TAPE_FLUTTER_RATE_MIN_HZ..=TAPE_FLUTTER_RATE_MAX_HZ)
                    .contains(&params.flutter_rate_hz),
                "flutter_rate_hz {value}"
            );
            assert!(
                (0.0..=TAPE_FLUTTER_DEPTH_MAX_MS).contains(&params.flutter_depth_ms),
                "flutter_depth_ms {value}"
            );
            assert!(
                (0.0..=TAPE_SEED_MAX).contains(&params.seed) && params.seed.fract() == 0.0,
                "seed {value} ⇒ {}",
                params.seed
            );
        }
        // 非有限一律归下界（不是上界、不是 NaN 透传）。
        let nan = TapeParams {
            drive: f32::NAN,
            mix: f32::NAN,
            trim_db: f32::NAN,
            hf_hz: f32::NAN,
            asym: f32::NAN,
            wow_rate_hz: f32::NAN,
            wow_depth_ms: f32::NAN,
            flutter_rate_hz: f32::NAN,
            flutter_depth_ms: f32::NAN,
            seed: f32::NAN,
        }
        .sanitised();
        assert_eq!(nan.drive, 0.0);
        assert_eq!(nan.mix, 0.0);
        assert_eq!(nan.trim_db, TAPE_TRIM_MIN_DB);
        assert_eq!(nan.hf_hz, TAPE_HF_MIN_HZ);
        assert_eq!(nan.asym, 0.0, "非有限的 `asym` 归 `0`（＝对称 ⇒ 恒等）");
        assert_eq!(nan.wow_rate_hz, TAPE_WOW_RATE_MIN_HZ);
        assert_eq!(nan.wow_depth_ms, 0.0, "非有限的 wow 深度归 `0`（⇒ 无调制）");
        assert_eq!(nan.flutter_rate_hz, TAPE_FLUTTER_RATE_MIN_HZ);
        assert_eq!(nan.flutter_depth_ms, 0.0, "非有限的 flutter 深度归 `0`");
        assert_eq!(nan.seed, 0.0);
        // seed 取整：先钳后 floor（2.9 ⇒ 2，不是四舍五入到 3）。
        assert_eq!(
            TapeParams {
                seed: 2.9,
                ..TapeParams::DEFAULT
            }
            .sanitised()
            .seed,
            2.0
        );
    }

    #[test]
    fn small_signal_gain_is_approximately_one() {
        // 小信号处 pade_tanh(g·x)/g ≈ x（与 drive 无关）：drive 只改谐波，不改电平。
        // 跳过级联 FIR 的建立段（约 2 × OS_LATENCY 帧），只看稳态。
        for drive in [0.0, 0.5, 1.0] {
            let mut tape = make_tape(drive, 1.0, 0.0, TAPE_HF_MAX_HZ);
            let mut left = vec![0.01; 512];
            let mut right = vec![0.01; 512];
            process_all(&mut tape, &mut left, &mut right);
            for sample in &left[128..] {
                assert!(
                    (*sample - 0.01).abs() < 1e-3,
                    "drive {drive}: 小信号增益偏离 0.01，实读 {sample}"
                );
            }
        }
    }

    /// ⭐ **`asym` 不改小信号电平**（`#347`）。
    ///
    /// 归一化不是万能的：`pade_tanh` 在 `|x| → 3` 处压平（斜率 → 0），`g·asym`
    /// 一旦进到那一带，"电平中性"本身就没有意义（那是硬削波区，不是磁带）。
    /// 因此判据只钉**能钉的那一段**，其余如实登记：
    ///
    /// | 区间 | 要求 | 依据 |
    /// | :--- | :--- | :--- |
    /// | 音乐区间 `drive ≤ 0.2`、`asym ≤ 0.32`（`g·asym ≤ 0.9`） | 与对称时相差 **< 0.5 dB** | 斜率 `≥ 0.5` ⇒ 归一化 `≤ 2×`，未触上界 |
    /// | 上界触发（`slope < 1/8`） | 归一化**恰好**取 [`MIN_ASYMMETRY_SLOPE`] | 钳制行为本身可验证 |
    ///
    /// 补丁前音乐区间就是 **−5.7 dB**（`drive 0.2 / asym 0.32`）。
    #[test]
    fn asym_does_not_change_the_small_signal_level() {
        let level = |asym: f32, drive: f32| {
            let mut tape = make_tape_asym(drive, 1.0, 0.0, TAPE_HF_MAX_HZ, asym);
            let mut left = vec![0.01; 4096];
            let mut right = vec![0.01; 4096];
            process_all(&mut tape, &mut left, &mut right);
            // 稳态取尾部（FIR 建立段之后），用均值而不是单点（压掉残余纹波）。
            let tail = &left[2048..];
            tail.iter().sum::<f32>() / tail.len() as f32
        };
        let error_db =
            |drive: f32, asym: f32| 20.0 * libm::log10f(level(asym, drive) / level(0.0, drive));
        for drive in [0.06, 0.12, 0.2] {
            for asym in [0.1, 0.2, 0.32] {
                let db = error_db(drive, asym);
                assert!(
                    db.abs() < 0.5,
                    "drive {drive} / asym {asym}: 电平必须与对称时一致，实差 {db:+.2} dB"
                );
            }
        }
        // 钳制行为：斜率跌破下界时，归一化恰好是 `1 / MIN_ASYMMETRY_SLOPE`。
        // 合法域里最陡的工作点是 `drive = 1`、`asym = 0.5` ⇒ `g·asym = 5`。
        let slope_at_extreme = crate::math::pade_tanh_slope(5.0);
        assert!(slope_at_extreme < MIN_ASYMMETRY_SLOPE);
        assert!(
            (slope_at_extreme.clamp(MIN_ASYMMETRY_SLOPE, 1.0) - MIN_ASYMMETRY_SLOPE).abs()
                < f32::EPSILON
        );
        // ⚠ **如实登记的边界**：`g·asym ≳ 1.5` 之后曲线已进硬削波段
        // （`pade_tanh` 在 `|x| > 3` 甚至非单调），那里"电平中性"没有定义，
        // 归一化只能钳在上界。判据只钉"绝不小于 1"（不反过来压电平）。
        for drive in [0.5, 1.0] {
            for asym in [0.32, 0.5] {
                let slope = crate::math::pade_tanh_slope((1.0 + 9.0 * drive) * asym);
                let normalise = 1.0 / slope.clamp(MIN_ASYMMETRY_SLOPE, 1.0);
                assert!(
                    (1.0..=8.0).contains(&normalise),
                    "drive {drive} / asym {asym}: 归一化 {normalise} 必须落在 [1, 8]"
                );
            }
        }
    }

    #[test]
    fn mix_zero_is_identity_within_float_tolerance() {
        // mix = 0 ⇒ 干路直通（往返延迟 OS_LATENCY 仍在，故跳过建立段；
        // FIR 直流增益的浮点误差在 1e-5 内，不按位比）。
        let mut tape = make_tape(0.8, 0.0, 0.0, TAPE_HF_MAX_HZ);
        let mut left = vec![0.7; 512];
        let mut right = vec![0.7; 512];
        process_all(&mut tape, &mut left, &mut right);
        for sample in &left[128..] {
            assert!(
                (*sample - 0.7).abs() < 1e-5,
                "mix = 0 时稳态必须与输入相等，实读 {sample}"
            );
        }
    }

    #[test]
    fn inactive_params_report_not_active() {
        assert!(!make_tape(0.0, 0.5, 0.0, TAPE_HF_MAX_HZ).is_active());
        assert!(!make_tape(0.5, 0.0, 0.0, TAPE_HF_MAX_HZ).is_active());
        assert!(!make_tape(0.0, 0.0, 0.0, TAPE_HF_MAX_HZ).is_active());
        assert!(make_tape(0.5, 0.5, 0.0, TAPE_HF_MAX_HZ).is_active());
        // 负值经 sanitised 归 0 ⇒ 不激活（不是“负驱动”）。
        assert!(!make_tape(-1.0, 0.5, 0.0, TAPE_HF_MAX_HZ).is_active());
    }

    #[test]
    fn latency_samples_is_the_oversampler_round_trip() {
        assert_eq!(TapeSaturation::new().latency_samples(), OS_LATENCY);
        assert_eq!(OS_LATENCY, 31);
    }

    #[test]
    fn reset_reproduces_a_freshly_built_device_bit_for_bit() {
        let mut first = make_tape(0.6, 0.8, -3.0, 8000.0);
        let mut second = make_tape(0.6, 0.8, -3.0, 8000.0);
        let mut left: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.05).sin() * 0.7).collect();
        let mut right = left.clone();
        let mut left2 = left.clone();
        let mut right2 = right.clone();
        process_all(&mut first, &mut left, &mut right);
        // second 先跑一段别的音频、再复位 ⇒ 与刚建好的 first 逐位相同。
        let mut noise = vec![0.9; 512];
        let mut noise_r = vec![-0.9; 512];
        process_all(&mut second, &mut noise, &mut noise_r);
        second.reset();
        process_all(&mut second, &mut left2, &mut right2);
        assert_eq!(left, left2);
        assert_eq!(right, right2);
    }

    #[test]
    fn non_finite_input_falls_back_to_zero_and_does_not_poison_state() {
        let mut tape = make_tape(0.7, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut fresh = make_tape(0.7, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut dirty = vec![f32::NAN; 256];
        let mut dirty_r = vec![f32::INFINITY; 256];
        process_all(&mut tape, &mut dirty, &mut dirty_r);
        assert!(
            dirty.iter().chain(dirty_r.iter()).all(|s| s.is_finite()),
            "非有限输入不得透传出非有限输出"
        );
        // 脏块写进历史的全是 0 ⇒ 随后干净块的输出与全新实例逐位相同。
        let mut clean: Vec<f32> = (0..512).map(|i| (i as f32 * 0.03).sin() * 0.6).collect();
        let mut clean_r = clean.clone();
        let mut clean2 = clean.clone();
        let mut clean2_r = clean_r.clone();
        process_all(&mut tape, &mut clean, &mut clean_r);
        process_all(&mut fresh, &mut clean2, &mut clean2_r);
        assert_eq!(clean, clean2);
        assert_eq!(clean_r, clean2_r);
    }

    #[test]
    fn impulse_peak_lands_on_the_round_trip_latency() {
        // 小冲激（近线性区）⇒ 级联对称 FIR 的群延迟峰落在 OS_LATENCY。
        let mut tape = make_tape(0.5, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let n = 512;
        let mut left = vec![0.0; n];
        let mut right = vec![0.0; n];
        left[0] = 0.1;
        process_all(&mut tape, &mut left, &mut right);
        let peak = left
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(peak, OS_LATENCY, "冲激峰必须落在往返延迟处，实读 {peak}");
        assert!(right.iter().all(|s| *s == 0.0), "右声道无输入 ⇒ 无串音");
    }

    /// 单点 DFT 幅值（`bin` 必须是整周期点，避免泄漏进断言）。
    fn dft_magnitude(samples: &[f32], bin: usize) -> f32 {
        let n = samples.len() as f32;
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (i, sample) in samples.iter().enumerate() {
            let angle = 2.0 * core::f32::consts::PI * bin as f32 * i as f32 / n;
            re += *sample * libm::cosf(angle);
            im -= *sample * libm::sinf(angle);
        }
        (re * re + im * im).sqrt() / n
    }

    #[test]
    fn oversampling_aliasing_is_below_the_direct_formula() {
        // 2x 过采样把“3 次谐波折叠”这件事推迟了一个倍频程：基频 10044.6 Hz
        // 的 3 次谐波 30133.9 Hz 在基带采样率下折到 17866.1 Hz（1334 bin），
        // 在 2x 域里它低于 2x 奈奎斯特（48 kHz）且落在下采样低通的阻带
        // （OS_H 在 30 kHz 处 |H| ≈ 0，实测）⇒ 器件输出的该 bin 必须远低于直算。
        //
        // 频率选择是算过的：窗口 3584 帧、bin 宽 48000/3584 Hz；基频取 750 bin
        // （10044.6 Hz），3 次谐波在两域都只落 1334 bin 一处（5/7/9/11 次分别落
        // 166/1666/418/…bin，不与 1334 碰撞 ⇒ 单 bin 比较无抵消混杂）。
        // 18 kHz 不能作此比较：它的 5 次谐波 90 kHz 在 2x 域里已 alias 成 6 kHz
        // 通带分量（实测 alias_2x > alias_1x），那是 2x 的固有上限，不是器件缺陷。
        // 加窗：跳过前 512 帧（含级联 FIR 建立段），剩余 3584 帧仍是整周期。
        const N: usize = 4096;
        const SKIP: usize = 512;
        const F0_BIN: usize = 750; // 10044.6 Hz @ 48 kHz, 窗口 3584
        const THIRD_BIN: usize = 1334; // 17866.1 Hz（3 次谐波的折叠位置）
        let input: Vec<f32> = (0..N)
            .map(|i| {
                0.9 * libm::sinf(2.0 * core::f32::consts::PI * F0_BIN as f32 * i as f32 / 3584.0)
            })
            .collect();
        let mut tape = make_tape(1.0, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut left = input.clone();
        let mut right = vec![0.0; N];
        process_all(&mut tape, &mut left, &mut right);
        let third_2x = dft_magnitude(&left[SKIP..], THIRD_BIN);
        let direct: Vec<f32> = input
            .iter()
            .map(|x| pade_tanh(10.0 * finite_or_zero(*x)) / 10.0)
            .collect();
        let third_1x = dft_magnitude(&direct[SKIP..], THIRD_BIN);
        assert!(
            third_1x > 0.001,
            "1x 直算的 3 次谐波折叠必须显著非零（否则比较真空），实读 {third_1x}"
        );
        assert!(
            third_2x < third_1x,
            "2x 过采样的 3 次谐波折叠（{third_2x}）必须低于 1x 直算（{third_1x}）"
        );
        // 基频本身必须在（输入不是静默、DFT 口径没错）：drive 1.0 时饱和把 0.9
        // 正弦峰值压到约 0.129，基频幅值约 0.063，故不按 0.1 判，只判器件基频
        // 不低于直算基频的一半（上下采样通带纹波在 10 kHz 处 |H| = 1.0000）。
        let f0_direct = dft_magnitude(&direct[SKIP..], F0_BIN);
        assert!(f0_direct > 0.05, "直算基频读数异常，实读 {f0_direct}");
        let f0_tape = dft_magnitude(&left[SKIP..], F0_BIN);
        assert!(
            f0_tape > 0.5 * f0_direct,
            "器件基频（{f0_tape}）不得远低于直算基频（{f0_direct}）"
        );
    }
    /// ⭐ **偶次谐波真的出现**（`tape_asym` 的全部意义）。
    ///
    /// 判据：纯正弦进、看 2f；`asym = 0` ⇒ 2f 只剩数值噪声，`asym = 0.3` ⇒
    /// 2f 至少高 **20 dB** 且相对基频**可闻**（> 2%）。同一窗口里奇次（3f）
    /// 必须**不动**（±20% 内）—— 否则就不是"加偶次"，而是整体换了曲线。
    #[test]
    fn asym_creates_the_second_harmonic_that_pade_tanh_cannot() {
        const N: usize = 4096;
        const SKIP: usize = 512;
        /// 窗口长度（`dft_magnitude` 的 `n`）：输入按它整周期构造 ⇒ 无泄漏。
        const W: usize = N - SKIP;
        const F0: usize = 64; // 64 / 3584 × 48 kHz ≈ 857 Hz
        let input: Vec<f32> = (0..N)
            .map(|i| {
                0.9 * libm::sinf(2.0 * core::f32::consts::PI * F0 as f32 * i as f32 / W as f32)
            })
            .collect();

        let run = |asym: f32| {
            let mut tape = make_tape_asym(0.06, 1.0, 0.0, TAPE_HF_MAX_HZ, asym);
            let mut left = input.clone();
            let mut right = vec![0.0; N];
            process_all(&mut tape, &mut left, &mut right);
            left
        };
        let symmetric = run(0.0);
        let asymmetric = run(0.3);

        let even0 = dft_magnitude(&symmetric[SKIP..], 2 * F0);
        let even1 = dft_magnitude(&asymmetric[SKIP..], 2 * F0);
        let odd0 = dft_magnitude(&symmetric[SKIP..], 3 * F0);
        let odd1 = dft_magnitude(&asymmetric[SKIP..], 3 * F0);
        let f0_1 = dft_magnitude(&asymmetric[SKIP..], F0);
        println!(
            "[dsp-tape/asym] 2f: {even0:e} → {even1:e}（{:.1} dB）  3f: {odd0:e} → {odd1:e}  f0: {f0_1:e}",
            20.0 * libm::log10f(even1 / even0.max(1e-12))
        );
        assert!(
            even1 > 10.0 * even0,
            "⭐ `asym = 0.3` 的二次谐波必须比 `asym = 0` 高 ≥ 20 dB（实读 {even0:e} → {even1:e}）"
        );
        assert!(
            even1 > 0.02 * f0_1,
            "⭐ 偶次必须**可闻**（2f > 基频的 2%）；实读 2f = {even1:e}、f0 = {f0_1:e}"
        );
        // ⚠ 奇次**会**变（工作点离开零点 ⇒ 整条曲线的形状都变），但⛔ 不该消失：
        //    实测 `asym = 0.3` 时 3f 从 3.37e-2 降到 2.26e-2（−3.5 dB）—— 这个量级
        //    的变化是"换了个工作点"的正常后果，不是缺陷 ⇒ 判据只钉"仍在同一量级"。
        assert!(
            odd1 > 0.5 * odd0,
            "⛔ 奇次（3f）不该被 `asym` 压掉一个量级：{odd0:e} → {odd1:e}"
        );
        // ⭐ 这才是"暖"的机械特征：偶次**超过**奇次（磁带 vs 过载的分界）。
        assert!(
            even1 > odd1,
            "⭐ `asym` 之后二次谐波（{even1:e}）必须**超过**三次（{odd1:e}）—— 否则只是更硬"
        );
    }

    /// ⭐ **零输入 ⇒ 零输出**（平移曲线的直流补偿真的生效）。
    ///
    /// 没有那一条常量补偿时，`x = 0` 的输出是 `pade_tanh(g·asym) / g ≈ 0.093`
    /// ⇒ 整条链带上 DC（下游限幅器会为它让出余量）。本判据把那个版本变成红。
    #[test]
    fn asym_keeps_silence_silent() {
        for asym in [0.0, 0.1, 0.3, TAPE_ASYM_MAX] {
            let mut tape = make_tape_asym(0.5, 1.0, 0.0, TAPE_HF_MAX_HZ, asym);
            let mut left = vec![0.0f32; 512];
            let mut right = vec![0.0f32; 512];
            process_all(&mut tape, &mut left, &mut right);
            let worst = left.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
            assert!(
                worst == 0.0,
                "⭐ `asym = {asym}` 的静音输入必须**逐位**输出 0（实读峰值 {worst:e}）"
            );
        }
    }

    /// ⭐ **`asym = 0` 就是接线前的曲线**（后向兼容，⛔ 不是"差不多"）。
    ///
    /// 与手算的旧公式 `pade_tanh(g·x) / g` 比：基频与三次谐波的幅值都在
    /// 上下采样通带纹波（`|H| = 1.0000`）以内，容差 `1e-3`（与
    /// `oversampling_aliasing_is_below_the_direct_formula` 同一口径）。
    #[test]
    fn asym_zero_is_the_pre_change_curve() {
        const N: usize = 4096;
        const SKIP: usize = 512;
        const W: usize = N - SKIP;
        const F0: usize = 64;
        let input: Vec<f32> = (0..N)
            .map(|i| {
                0.9 * libm::sinf(2.0 * core::f32::consts::PI * F0 as f32 * i as f32 / W as f32)
            })
            .collect();
        let mut tape = make_tape(0.5, 1.0, 0.0, TAPE_HF_MAX_HZ);
        let mut left = input.clone();
        let mut right = vec![0.0; N];
        process_all(&mut tape, &mut left, &mut right);

        // 旧公式（`g = 1 + 9 × 0.5 = 5.5`），1x 直算。
        let g = 1.0 + 9.0 * 0.5;
        let direct: Vec<f32> = input
            .iter()
            .map(|x| pade_tanh(g * finite_or_zero(*x)) / g)
            .collect();
        for (label, bin) in [("f0", F0), ("2f", 2 * F0), ("3f", 3 * F0)] {
            let a = dft_magnitude(&left[SKIP..], bin);
            let b = dft_magnitude(&direct[SKIP..], bin);
            assert!(
                (a - b).abs() < 1e-3,
                "⭐ `asym = 0` 必须与接线前的公式一致（{label}: 器件 {a:e} vs 直算 {b:e}）"
            );
        }
    }

    /// 确定性噪声（xorshift32；判据专用，不进产品路径）。
    fn test_noise(n: usize, seed: u32) -> Vec<f32> {
        let mut state = if seed == 0 { 0x2545_F491 } else { seed };
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                ((state >> 8) as f32 / 16_777_216.0 - 0.5) * 0.6
            })
            .collect()
    }

    /// 分块处理（一块 512 帧 ⇒ 调制状态必须跨块连续，顺带钉住这一点）。
    fn process_in_blocks(tape: &mut TapeSaturation, left: &mut [f32], right: &mut [f32]) {
        let mut offset = 0;
        while offset < left.len() {
            let end = (offset + 512).min(left.len());
            process_all(tape, &mut left[offset..end], &mut right[offset..end]);
            offset = end;
        }
    }

    /// ⭐ **深度全 0 ⇒ 调制段不存在**（P7 的零回归钉）。
    ///
    /// 速率与 seed 随便写、深度全 0 ⇒ 输出与"没有调制参数"逐位相同，
    /// 且 `modulation_latency_samples` 为 0（没有偷偷加延迟）。
    #[test]
    fn modulation_depth_zero_is_bit_identical() {
        let input = test_noise(8192, 42);
        let mut plain = make_tape(0.4, 0.9, -2.0, 9000.0);
        let mut dressed = TapeSaturation::new();
        dressed.configure(SR);
        dressed.set_params(TapeParams {
            drive: 0.4,
            mix: 0.9,
            trim_db: -2.0,
            hf_hz: 9000.0,
            asym: 0.0,
            wow_rate_hz: 1.7,
            wow_depth_ms: 0.0,
            flutter_rate_hz: 13.0,
            flutter_depth_ms: 0.0,
            seed: 999.0,
        });
        assert!(!dressed.modulation_active());
        assert_eq!(dressed.modulation_latency_samples(), 0);
        let (mut l1, mut r1) = (input.clone(), input.clone());
        let (mut l2, mut r2) = (input.clone(), input.clone());
        process_in_blocks(&mut plain, &mut l1, &mut r1);
        process_in_blocks(&mut dressed, &mut l2, &mut r2);
        assert_eq!(l1, l2, "⭐ 深度全 0 的左声道必须逐位等于无调制参数");
        assert_eq!(r1, r2, "⭐ 深度全 0 的右声道必须逐位等于无调制参数");
    }

    /// 在 `output` 的 `center` 附近用互相关估有效延迟（样本，整数）。
    ///
    /// 估计窗内的输出与**输入**对应窗互相关，峰位即（过采样延迟 + 调制延迟）。
    /// 噪声输入的自相关峰唯一 ⇒ 无周期歧义；窗长 8192（0.17 s）内 wow 相位
    /// 移动 ≤ 31° ⇒ 峰位误差在数个样本内（断言容差按此留）。
    fn estimate_delay(input: &[f32], output: &[f32], center: usize, max_lag: usize) -> usize {
        const WIN: usize = 8192;
        let start = center - WIN / 2;
        let mut best_lag = 0usize;
        let mut best = f32::MIN;
        for lag in 0..=max_lag {
            let mut acc = 0.0f32;
            for i in 0..WIN {
                // `start ≥ 4096 > max_lag`（判据的中心帧最小 24_000）⇒ 不下溢。
                acc += output[start + i] * input[start + i - lag];
            }
            if acc > best {
                best = acc;
                best_lag = lag;
            }
        }
        best_lag
    }

    /// ⭐ **深度非 0 ⇒ 延迟调制可量**（P7 的功能钉）。
    ///
    /// wow 深度 2.0 ms（96 样本）@ 0.5 Hz：互相关估出的有效延迟必须
    /// ① 全程落在理论包络 `[OS_LATENCY + 边距, OS_LATENCY + 中心 + 深度]`
    /// 内（±4 样本估计容差）；② 随时间真的动（极差 ≥ 48 样本 = 1 ms；
    /// 纯正弦分量一个周期内的峰峰值是 144 样本，漂移只会加不会减）。
    #[test]
    fn modulation_delay_is_measurable_by_cross_correlation() {
        let n = 144_000; // 3 s @ 48 kHz（wow 周期 2 s，采样点跨整周期）
        let input = test_noise(n, 7);
        let mut tape = make_tape_mod(0.5, 2.0, 6.0, 0.0, 7.0);
        assert!(tape.modulation_active());
        let mut left = input.clone();
        let mut right = input.clone();
        process_in_blocks(&mut tape, &mut left, &mut right);
        // 两声道共用同一条调制 ⇒ 同一输入必须给出逐位相同的输出。
        assert_eq!(left, right, "⭐ 两声道同调制：同输入 ⇒ 同输出逐位成立");

        let center_samples = (2.0 + MOD_MARGIN_MS) * SR / 1000.0; // 120
        let depth_samples = 2.0 * SR / 1000.0; // 96
        let lo = OS_LATENCY as f32 + MOD_MARGIN_MS * SR / 1000.0 - 4.0;
        let hi = OS_LATENCY as f32 + center_samples + depth_samples + 4.0;
        let max_lag = (hi as usize) + 8;
        let mut estimates = Vec::new();
        for center in [24_000usize, 48_000, 72_000, 96_000, 120_000] {
            let lag = estimate_delay(&input, &left, center, max_lag) as f32;
            assert!(
                (lo..=hi).contains(&lag),
                "⭐ 中心帧 {center} 的有效延迟 {lag} 必须落在理论包络 [{lo}, {hi}] 内"
            );
            estimates.push(lag);
        }
        let spread = estimates.iter().fold(f32::MIN, |a, b| a.max(*b))
            - estimates.iter().fold(f32::MAX, |a, b| a.min(*b));
        println!("[dsp-tape/p7] 有效延迟读数 {estimates:?}，极差 {spread} 样本");
        assert!(
            spread >= 48.0,
            "⭐ 调制必须真的动：有效延迟极差 {spread} 样本 < 48（1 ms）"
        );
    }

    /// ⭐ **同 seed 两次渲染逐位一致；换 seed 轨迹必变**（种子可复现钉）。
    #[test]
    fn modulation_same_seed_is_bit_identical_and_seed_matters() {
        let input = test_noise(16_384, 99);
        let render = |seed: f32| {
            let mut tape = make_tape_mod(0.55, 0.9, 6.5, 0.04, seed);
            let mut left = input.clone();
            let mut right = input.clone();
            process_in_blocks(&mut tape, &mut left, &mut right);
            left
        };
        let a = render(2026.0);
        let b = render(2026.0);
        assert_eq!(a, b, "⭐ 同 seed 两次渲染必须逐位一致");
        let c = render(2027.0);
        assert_ne!(a, c, "⭐ 换 seed 后调制轨迹必须变（seed 不是摆设）");
    }

    /// ⭐ 调制武装时 `reset()` 复现全新实例（帧计数与延迟线都归零）。
    #[test]
    fn modulation_reset_reproduces_fresh_instance_bit_for_bit() {
        let input = test_noise(8192, 5);
        let mut fresh = make_tape_mod(0.8, 1.5, 9.0, 0.3, 31.0);
        let mut reused = make_tape_mod(0.8, 1.5, 9.0, 0.3, 31.0);
        // reused 先跑一段别的音频（把延迟线与帧计数推离起点），再复位。
        let mut junk = test_noise(4096, 6);
        let mut junk_r = junk.clone();
        process_in_blocks(&mut reused, &mut junk, &mut junk_r);
        reused.reset();
        let (mut l1, mut r1) = (input.clone(), input.clone());
        let (mut l2, mut r2) = (input.clone(), input.clone());
        process_in_blocks(&mut fresh, &mut l1, &mut r1);
        process_in_blocks(&mut reused, &mut l2, &mut r2);
        assert_eq!(l1, l2);
        assert_eq!(r1, r2);
    }

    /// 调制延迟读数：未武装 0；武装 = 中心延迟（两深度和 + 边距）。
    #[test]
    fn modulation_latency_samples_reports_the_center_delay() {
        let mut tape = make_tape_mod(0.5, 2.0, 6.0, 0.5, 1.0);
        // 中心 = (2.0 + 0.5 + 0.5) ms = 3 ms @ 48 kHz = 144 样本。
        assert_eq!(tape.modulation_latency_samples(), 144);
        tape.set_params(TapeParams {
            wow_depth_ms: 0.0,
            flutter_depth_ms: 0.0,
            ..tape.params()
        });
        assert_eq!(tape.modulation_latency_samples(), 0);
    }
}
