//! 通道条：把既有器件串成一条固定顺序的**逐样本**链。[ARCH-RT-001] [ARCH-DET-001] [ARCH-DSP-001]
//!
//! 本模块**不实现**任何滤波器、EQ 或压缩器算法。它的全部内容是**组合**：
//! 复用 [`crate::shaping::ShapingEq`]、[`crate::filter::LadderFilter`] 与
//! [`crate::compressor::Compressor`]，一处实现、一处参数 [ARCH-DSP-001]。
//!
//! ## 1. 规范怎么说"通道条"（原文引用）
//!
//! 规范**有**对通道条的产地与内容的要求，共两处：
//!
//! | 出处 | 原文 |
//! | :--- | :--- |
//! | `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109` | `- 通道条算法 ➔ crates/yeban-dsp/src/channel_strip.rs（EQ、滤波、动态旁通链）` |
//! | `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:411` | `\| **内部 DSP 拓扑调度** \| - \| **1.00 ms** \| 声部合成、通道条 EQ/压缩与 PDC 延迟线插入计算 \|` |
//!
//! `crates/yeban-dsp/Cargo.toml:3` 的包描述也把"通道条"列为本 crate 的交付之一。
//!
//! **规范没有规定的**（下面是工程选择，逐条给出理由）：
//!
//! 1. **三级的先后顺序**。规范只写了"EQ、滤波、动态旁通链"这个**无序**的集合。
//! 2. **各级的参数域**（截止频率范围、增益范围、旁通语义）。
//! 3. **计量口径**（峰值 / RMS 的窗口与联动规则）。
//!
//! 规范里出现的"通道条"另有一批**界面**含义（`crates/yeban-app/ui/console/mixer_console.slint`
//! 的 `track-{i}-channel-strip`、`docs/ledger/app-mixer-notes.md:153`）。那些是**混音台控件**，
//! 与本模块不是同一个东西。本模块只做 dsp 层的器件；接线是引擎侧的独立裁决。
//!
//! ## 2. 信号顺序（**这是契约**，判据 `stage_order_is_the_contract` 钉住它）
//!
//! ```text
//! ① 输入增益   s ← s · 10^(input_gain_db/20)                ← 逐样本，无状态
//! ② EQ         ShapingEq（低架 / 中频峰 / 高架）             ← 块 API，见 §3
//! ③ 滤波       LadderFilter（四极梯形低通）                  ← 逐样本
//! ④ 动态       Compressor（RMS 检波 + 软膝 + 增益弹道）      ← 逐样本
//! ⑤ 输出增益   s ← s · 10^(output_gain_db/20)                ← 逐样本，无状态
//! ```
//!
//! 为什么是**这个**顺序：
//!
//! | 决定 | 理由 |
//! | :--- | :--- |
//! | 输入增益在**最前** | 它是"配平"（trim）。放在最前 ⇒ 后面三级的阈值与截止频率都定义在**配平之后**的电平上。这是调音台通道条的既有做法：先定工作电平，再处理。 |
//! | EQ 在滤波**之前** | EQ 是音色塑形，滤波是限带。滤波放在 EQ 之后 ⇒ **限带说了最后一句**：高架提升之后紧跟低通，被提升的高频仍会被低通切掉。反过来的话，低通之后的高架提升会重新把高频抬起来，限带就失效了。 |
//! | 滤波在动态**之前** | 压缩器的**检波器**必须看见限带之后的信号。若压缩在前，被滤波去掉的隆隆声与超声噪声仍然会驱动增益衰减 ⇒ 听不见的东西压缩了听得见的东西。这是通道条把高通放在动态之前的**唯一**理由。 |
//! | 输出增益在**最后** | 它是推子。放在最后 ⇒ 推子位置**不改变**压缩量。若放在动态之前，它就是第二个输入增益，压缩器会跟着推子走（"推上去反而压得更狠"）。 |
//!
//! ## 3. EQ 的块 API 适配（本模块唯一一处非逐样本的代码）
//!
//! [`crate::shaping::ShapingEq::process`] 的签名是块式的（`in_l/in_r/out_l/out_r`
//! 四个切片），它是既有器件，本票**不重写**它。本模块把它切成
//! [`EQ_CHUNK_FRAMES`] 帧的小块调用，中间落在**栈数组**上：
//!
//! - 栈数组是定长 `[f32; EQ_CHUNK_FRAMES]` ⇒ **零堆分配**（§6）。
//! - 系数在每次 `ShapingEq::process` 里按 `(params, sample_rate)` 重算。重算是
//!   **幂等**的：同一组参数算出同一组系数。因此"调用方怎么切块"与"内部怎么切块"
//!   都不改变输出（§5 的判据实测这一点）。
//!
//! ⚠ **平坦 EQ 不是逐位恒等变换**（本票实测）。`ShapingEq` 的三个双二阶在 0 dB 时
//! 系数在数学上恒等（`b0 = 1`、`b1 = a1`、`b2 = a2`），但直接型递推在 `f32` 下会留下
//! 末位舍入：判据 `the_eq_really_changes_the_level` 实测 48 000 帧里有 **47 900** 帧
//! 与"EQ 全旁通"逐位不同，**最大绝对差 4.566e-5**。`crates/yeban-dsp/src/shaping.rs:798`
//! 的 `eq_flat_is_unity` 断言的是**幅频响应** `|gain| < 0.05 dB`，不是逐位恒等。
//! ⇒ 本模块**只**声称平坦 EQ 在 `4.6e-5` 以内，**不**声称逐位透明。
//! （真正逐位透明的做法是把 `eq_enabled` 设为 `false`，判据
//! `an_all_bypassed_strip_at_unity_gain_is_bit_identical` 钉住这一点。）
//!
//! ## 4. 旁通（`*_enabled`）
//!
//! 规范写的是"动态**旁通链**"。本模块把它读成：链上每一级都可以被旁通。因此
//! [`ChannelStripParams`] 有 `eq_enabled` / `filter_enabled` / `compressor_enabled`
//! 三个开关。语义（**声明的**，不是副作用）：
//!
//! - 旁通 = **整级不执行**（不调用它的 `process*`）。该级的状态因此**不推进**。
//! - 运行中翻转开关会产生**不连续**（该级的记忆被跳过一拍）。本模块**不做**旁通
//!   交叉淡化，也不做参数平滑；爆音消除是调用方的职责 [ARCH-DSP-001]。
//! - 增益级（输入 / 输出）没有开关：`0 dB` 已经是数学恒等（`db_to_gain(0) = 1.0`，
//!   一次乘 1.0 不改动任何位）。
//!
//! ## 5. 与块切分的关系（[ARCH-DET-001]）
//!
//! 链上三级的状态全部是**逐样本前向递归**：EQ 的 3 个双二阶、滤波的 4 个积分器、
//! 压缩器的 2 个标量。没有任何一级有前视缓冲、延迟线或跨块状态。加上 §3 的
//! "系数重算幂等"，本器件满足：
//!
//! > 同一输入信号，无论 `process_stereo` / `process_mono` 被怎样切分，输出**逐位相同**。
//!
//! 延迟为 0（[`ChannelStrip::latency_samples`]），不参与 PDC。
//! 判据 `chunking_does_not_change_output` 实测这条。
//!
//! ## 6. 数值边界（⛔ 不许 NaN / Inf）与实时安全
//!
//! | 项 | 处理 |
//! | :--- | :--- |
//! | 输入样本 `NaN` | [提前] 归零（[`crate::meter::sanitize_sample`]） |
//! | 输入样本 `±∞` / 越界 | [提前] 钳到 `±`[`crate::meter::MAX_LINEAR_MAGNITUDE`]（`16.0`） |
//! | 参数 `NaN` / `±∞` | [构造期] [`ChannelStripParams::sanitised`] 钳到合法域，见该函数文档 |
//! | 采样率 `NaN` / `0` / 负数 | [构造期] 钳到 [`crate::MIN_SAMPLE_RATE`]（复用 [`crate::math`] 的同名口径） |
//! | 输出 | [出口] 非有限值替换为 `0.0`（[`finite_or_zero`]，最后一道兜底） |
//!
//! 逐样本路径里**没有** `Vec` / `Box` / `String` / `format!` / `println!` / 锁 / 文件 I/O。
//! [`ChannelStrip`] 本身**不含任何 `Vec`**：全部状态是定长数组与标量。运行期读数见
//! `tests/channel_strip_rt_zero_alloc.rs`。
//!
//! ### 6.1 出口兜底 [`finite_or_zero`] 的可达性（**实测的未证事实**）
//!
//! 出口兜底是"最后一道保险"，本票**没有**证据表明它在当前参数域下会触发：
//! 把这两行删掉（注入 I4）之后，`cargo test -p yeban-dsp` 的全部 **223** 条判据
//! 仍然全绿（`204 + 4 + 3 + 3 + 4 + 5`，退出码 0）。原因在 §5 / §6 的构造：输入先经
//! [`sanitize_sample`] 钳到 `±16`，参数经 `ChannelStripParams::sanitised` 钳到合法域，
//! 三个器件各自有界 ⇒ 链上不会产生非有限值。
//!
//! **因此**：本兜底是**防御性**的，不是"被证明会走到"的路径。保留它的理由是
//! "任何未来的参数域扩张下出口仍然有限"；⛔ **不要**把它当成一条已验证的判据读。
//!
//! ## 7. 判据的牙（注入实测，逐条记录）
//!
//! | 注入 | 改了什么 | 结果 |
//! | :--- | :--- | :--- |
//! | I1 | 把 §2 的③滤波与④动态**互换** | **红 2 条**：`the_chain_matches_an_independently_built_reference_chain`（4095 / 4096 帧不同，最大差 6.392e-2）与 `stage_order_is_the_contract`（0 / 4096 不同 ⇒ 模块与"互换后的参照"变成同一条链） |
//! | I2 | `process_stereo` 直接 `return`（通道条完全不工作） | **红 6 条**：`hostile_inputs_never_escape_as_non_finite`（`NaN`/`inf` 原样穿出）、`the_eq_really_changes_the_level`、`the_chain_matches_an_independently_built_reference_chain`、`mono_path_matches_a_duplicated_stereo_pair`、`degenerate_slice_lengths_do_not_panic`、`set_params_and_sample_rate_are_live_and_allocation_free_paths`；另加运行期零分配那两条判据的**覆盖度自检**报红 |
//! | I3 | 在 `process_stereo` 的 chunk 循环里加一次 `Vec::<u8>::with_capacity(1)` | **红 2 条**：`rt_path_allocates_nothing_over_10_000_quanta` 报 `allocations=20000 deallocations=20000`；`the_eq_block_path_allocates_nothing_with_and_without_the_eq` 报 `allocations=40000 deallocations=40000` |
//! | I4 | **删掉**出口兜底（§6.1） | ⚠ **不红**：全部 223 条判据仍绿 ⇒ 见 §6.1 的可达性声明 |

use crate::compressor::{Compressor, CompressorParams};
use crate::filter::LadderFilter;
use crate::math::{db_to_gain, sanitise_sample_rate};
use crate::meter::{dbfs, sanitize_sample};
use crate::shaping::{EqParams, ShapingEq};

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// 增益级（输入 / 输出）的**下界**（dB）。`-60 dB` ≈ 增益 `0.001`。
pub const MIN_GAIN_DB: f32 = -60.0;

/// 增益级（输入 / 输出）的**上界**（dB）。`+24 dB` ≈ 增益 `15.849`。
pub const MAX_GAIN_DB: f32 = 24.0;

/// 低通截止频率的下界（Hz）。
pub const MIN_CUTOFF_HZ: f32 = 20.0;

/// 低通截止频率的上界（Hz）。这个值在 44.1 kHz 下仍低于 Nyquist，
/// 更低的采样率下由 [`LadderFilter::configure`] 自己再钳一次。
pub const MAX_CUTOFF_HZ: f32 = 20_000.0;

/// 内部 EQ 分块的帧数。
///
/// 它**不影响输出**（§3 / §5），只影响系数重算的频率。`64` 与"每量子一次系数重算"
/// 同量级，且让栈占用保持在 4 × 64 × 4 B = 1 KiB。
pub const EQ_CHUNK_FRAMES: usize = 64;

// ---------------------------------------------------------------------------
// 参数
// ---------------------------------------------------------------------------

/// 低通滤波级的参数（[`LadderFilter`] 的三个旋钮）。
///
/// 它们是 [`LadderFilter::configure`] 的实参，本模块**不**改变它的含义。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterParams {
    /// 截止频率（Hz），钳到 `[`[`MIN_CUTOFF_HZ`]`, `[`MAX_CUTOFF_HZ`]`]`。
    pub cutoff_hz: f32,
    /// 谐振量（`0..=1` 的旋钮值）。
    pub resonance: f32,
    /// 驱动量（`0..=1` 的旋钮值）。
    pub drive: f32,
}

impl FilterParams {
    /// 默认参数：截止 `20 kHz`（链上最透明的一侧）、无谐振、无驱动。
    pub const DEFAULT: Self = Self {
        cutoff_hz: MAX_CUTOFF_HZ,
        resonance: 0.0,
        drive: 0.0,
    };

    /// 返回把每个字段钳到合法域后的副本。**全定义**：任何输入（含 `NaN`/`∞`）都有返回值。
    ///
    /// | 字段 | 合法域 | 低于下界 | 高于上界 | `NaN` / `±∞` |
    /// | :--- | :--- | :--- | :--- | :--- |
    /// | `cutoff_hz` | `[`[`MIN_CUTOFF_HZ`]`, `[`MAX_CUTOFF_HZ`]`]` | 下界 | 上界 | **上界** |
    /// | `resonance` | `[0, 1]` | `0` | `1` | **`0`** |
    /// | `drive` | `[0, 1]` | `0` | `1` | **`0`** |
    ///
    /// ⚠ `cutoff_hz` 的非有限输入归到**上界**（不像 [`CompressorParams::sanitised`] 归下界）。
    /// 理由是本级是**低通**：截止频率越高，去掉的东西越少 ⇒ 高频端是**保守**的一侧。
    /// 归到下界（20 Hz）会把几乎整个频带切掉，那是一次巨大的、听得见的改动。
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            cutoff_hz: clamp_or(self.cutoff_hz, MIN_CUTOFF_HZ, MAX_CUTOFF_HZ, MAX_CUTOFF_HZ),
            resonance: clamp_or(self.resonance, 0.0, 1.0, 0.0),
            drive: clamp_or(self.drive, 0.0, 1.0, 0.0),
        }
    }
}

impl Default for FilterParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 通道条的全部参数。
///
/// 全部字段是 `f32` 或 `bool`；单位在字段名里写明（`_db` = dB、`_hz` = Hz）。
/// 非法值**不会 panic**：它们经 [`ChannelStripParams::sanitised`] 钳到合法域，
/// 该函数在 [`ChannelStrip::new`] 与 [`ChannelStrip::set_params`] 里各调用一次。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelStripParams {
    /// 输入增益（trim，dB）。链上**第一级**。钳到 `[`[`MIN_GAIN_DB`]`, `[`MAX_GAIN_DB`]`]`。
    pub input_gain_db: f32,
    /// 是否执行 EQ 级（见模块注释 §4 的旁通语义）。
    pub eq_enabled: bool,
    /// EQ 参数，直接交给 [`ShapingEq::process`]。
    pub eq: EqParams,
    /// 是否执行滤波级。
    pub filter_enabled: bool,
    /// 低通参数。
    pub filter: FilterParams,
    /// 是否执行动态级。
    pub compressor_enabled: bool,
    /// 压缩器参数，**复用** [`CompressorParams`]（本模块不定义第二份）。
    pub compressor: CompressorParams,
    /// 输出增益（推子，dB）。链上**最后一级**。
    pub output_gain_db: f32,
}

impl ChannelStripParams {
    /// 默认参数：输入 / 输出增益 `0 dB`（数学恒等），三级**全部启用**，
    /// EQ 平坦（[`EqParams::default`]），低通截止 `20 kHz`（最透明），
    /// 压缩用 [`CompressorParams::DEFAULT`]。
    pub const DEFAULT: Self = Self {
        input_gain_db: 0.0,
        eq_enabled: true,
        eq: EqParams {
            low_gain: 0.0,
            low_freq: 200.0,
            mid_gain: 0.0,
            mid_freq: 1000.0,
            mid_q: 0.9,
            high_gain: 0.0,
            high_freq: 4000.0,
        },
        filter_enabled: true,
        filter: FilterParams::DEFAULT,
        compressor_enabled: true,
        compressor: CompressorParams::DEFAULT,
        output_gain_db: 0.0,
    };

    /// 返回把每个字段钳到合法域后的副本。**全定义**：任何输入（含 `NaN`/`∞`）都有返回值。
    ///
    /// 增益级用 [`sanitise_gain_db`]：非有限值归到 **`0 dB`**（恒等），不归到下界。
    /// 理由与 [`CompressorParams::sanitised`] 的"归下界"不同 —— 对**增益**而言 `0 dB`
    /// 才是保守的一侧：`-60 dB` 是静音，那是一次巨大的、听得见的改动；而 `0 dB`
    /// 逐位透明（`x · 1.0`）。两者的共同原则是"非有限输入取**最不改变信号**的一侧"，
    /// 只是在 dB 增益轴上那一侧是 `0`，在阈值轴上那一侧是下界。
    ///
    /// EQ 的每个字段用的是 [`ShapingEq::process`] 内部的同一组上下界与同一个
    /// fallback（见 `crates/yeban-dsp/src/shaping.rs` 的 `clamp_or`），因此本函数
    /// 的输出与 `ShapingEq` 自己的钳制**一致**，不会互相覆盖。
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            input_gain_db: sanitise_gain_db(self.input_gain_db),
            eq_enabled: self.eq_enabled,
            eq: sanitise_eq(self.eq),
            filter_enabled: self.filter_enabled,
            filter: self.filter.sanitised(),
            compressor_enabled: self.compressor_enabled,
            compressor: self.compressor.sanitised(),
            output_gain_db: sanitise_gain_db(self.output_gain_db),
        }
    }
}

impl Default for ChannelStripParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ---------------------------------------------------------------------------
// 器件
// ---------------------------------------------------------------------------

/// 一条通道条：输入增益 → EQ → 滤波 → 动态 → 输出增益（顺序见模块注释 §2）。
///
/// 全部状态是**定长数组与标量**，`Copy` 之外没有堆所有权，**不含 `Vec`/`Box`**
/// ⇒ 构造之后不再有任何堆交互 [ARCH-RT-001]。
#[derive(Clone, Copy, Debug)]
pub struct ChannelStrip {
    params: ChannelStripParams,
    sample_rate: f32,
    /// 输入增益的线性值（`db_to_gain(input_gain_db)`，构造期算一次）。
    input_gain: f32,
    /// 输出增益的线性值。
    output_gain: f32,
    /// ② 级状态。
    eq: ShapingEq,
    /// ③ 级状态，**左**声道。
    ///
    /// ⚠ [`LadderFilter`] 是**单声道**器件（一个 4 级状态、一个输入一个输出）。
    /// 立体声通道条需要**两个独立实例**。第一版把两声道串进**同一个**实例，
    /// 结果是两声道共享一条 4 级状态机 —— 那会让左右声道互相污染，并且让
    /// `the_chain_matches_an_independently_built_reference_chain` 实测到
    /// 4094 / 4096 帧不同（最大差 7.45e-2）。缺陷由那条判据抓出，本版拆成两份。
    filter_l: LadderFilter,
    /// ③ 级状态，**右**声道。
    filter_r: LadderFilter,
    /// ④ 级状态。
    compressor: Compressor,
    /// 输入峰值（线性，自上次 `reset` 起的**联动**峰值，见 [`Self::input_peak`]）。
    input_peak: f32,
    /// 输入均方（`f64` 累加器，避免长窗口的 `f32` 精度损失）。
    input_mean_square: f64,
    /// 输出峰值（线性）。
    output_peak: f32,
    /// 输出均方（`f64` 累加器）。
    output_mean_square: f64,
    /// 自上次 `reset` 起处理过的**帧**数（RMS 的分母）。
    frames: u64,
}

impl ChannelStrip {
    /// 构造。非法参数与非法采样率在这里被钳到合法域（[`ChannelStripParams::sanitised`]）。
    #[must_use]
    pub fn new(params: ChannelStripParams, sample_rate: f32) -> Self {
        let sample_rate = sanitise_sample_rate(sample_rate);
        let params = params.sanitised();
        let mut filter_l = LadderFilter::new();
        let mut filter_r = LadderFilter::new();
        let (cutoff, resonance, drive) = (
            params.filter.cutoff_hz,
            params.filter.resonance,
            params.filter.drive,
        );
        filter_l.configure(sample_rate, cutoff, resonance, drive);
        filter_r.configure(sample_rate, cutoff, resonance, drive);
        Self {
            input_gain: db_to_gain(params.input_gain_db),
            output_gain: db_to_gain(params.output_gain_db),
            eq: ShapingEq::new(),
            filter_l,
            filter_r,
            compressor: Compressor::new(params.compressor, sample_rate),
            input_peak: 0.0,
            input_mean_square: 0.0,
            output_peak: 0.0,
            output_mean_square: 0.0,
            frames: 0,
            params,
            sample_rate,
        }
    }

    /// 换参数。**不清状态**（与 [`Compressor::set_params`] 同纪律）。
    ///
    /// 采样率不变 ⇒ 两个滤波器的系数按新参数各重算一次。
    pub fn set_params(&mut self, params: ChannelStripParams) {
        let params = params.sanitised();
        self.input_gain = db_to_gain(params.input_gain_db);
        self.output_gain = db_to_gain(params.output_gain_db);
        let (cutoff, resonance, drive) = (
            params.filter.cutoff_hz,
            params.filter.resonance,
            params.filter.drive,
        );
        self.filter_l
            .configure(self.sample_rate, cutoff, resonance, drive);
        self.filter_r
            .configure(self.sample_rate, cutoff, resonance, drive);
        self.compressor.set_params(params.compressor);
        self.params = params;
    }

    /// 换采样率。**不清状态**（与 [`Compressor::set_sample_rate`] 同纪律）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        self.sample_rate = sample_rate;
        let (cutoff, resonance, drive) = (
            self.params.filter.cutoff_hz,
            self.params.filter.resonance,
            self.params.filter.drive,
        );
        self.filter_l
            .configure(sample_rate, cutoff, resonance, drive);
        self.filter_r
            .configure(sample_rate, cutoff, resonance, drive);
        self.compressor.set_sample_rate(sample_rate);
    }

    /// 清空**全部**状态与**全部**计量读数（EQ、滤波、压缩器的状态 + 峰值/RMS/帧数）。
    pub fn reset(&mut self) {
        self.eq.reset();
        self.filter_l.reset();
        self.filter_r.reset();
        self.compressor.reset();
        self.input_peak = 0.0;
        self.input_mean_square = 0.0;
        self.output_peak = 0.0;
        self.output_mean_square = 0.0;
        self.frames = 0;
    }

    /// 当前参数（已钳制后的副本）。
    #[must_use]
    pub const fn params(&self) -> ChannelStripParams {
        self.params
    }

    /// 当前采样率（已钳制）。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 本器件的延迟：**恒为 `0`**。
    ///
    /// 链上没有前视、没有延迟线、没有过采样往返 ⇒ 不参与 PDC 补偿 [ARCH-PDC-001]。
    /// 这条与 [`Compressor::latency_samples`] 同口径，也是 §5 的构造性依据。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        0
    }

    /// 原地处理一个立体声块，返回处理的**帧**数。
    ///
    /// - 两个切片长度不等时只处理 `min(len)` 帧（**不 panic**）。
    /// - 空切片 ⇒ 空操作，返回 `0`。
    /// - 逐样本**零堆分配**（EQ 的中间缓冲是栈数组）。
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) -> usize {
        let frames = left.len().min(right.len());
        // EQ 的中间缓冲：栈上定长，**不是** `Vec`。
        let mut eq_l = [0.0f32; EQ_CHUNK_FRAMES];
        let mut eq_r = [0.0f32; EQ_CHUNK_FRAMES];
        let mut start = 0usize;
        while start < frames {
            let n = EQ_CHUNK_FRAMES.min(frames - start);
            let end = start + n;
            // ① 输入侧：清洗 + 计量 + 输入增益（逐样本）。
            for (l, r) in left[start..end]
                .iter_mut()
                .zip(right[start..end].iter_mut())
            {
                let (l_in, r_in) = self.observe_input(*l, *r);
                *l = l_in * self.input_gain;
                *r = r_in * self.input_gain;
            }
            // ② EQ 级（块 API，落在栈数组上）。
            if self.params.eq_enabled {
                self.eq.process(
                    &left[start..end],
                    &right[start..end],
                    &mut eq_l[..n],
                    &mut eq_r[..n],
                    self.params.eq,
                    self.sample_rate,
                );
                left[start..end].copy_from_slice(&eq_l[..n]);
                right[start..end].copy_from_slice(&eq_r[..n]);
            }
            // ③④⑤ 逐样本。
            for (l, r) in left[start..end]
                .iter_mut()
                .zip(right[start..end].iter_mut())
            {
                let (out_l, out_r) = self.render_frame(*l, *r);
                *l = out_l;
                *r = out_r;
            }
            start = end;
        }
        frames
    }

    /// 原地处理一个单声道块，返回处理的样本数。
    ///
    /// 立体声联动（[`Compressor::process_gain`] 的 `max(l², r²)`）在这里退化为
    /// `l = r = 样本`，因此"把立体声的两个声道填成同一个信号"与"直接走单声道路径"
    /// 产出**逐位相同**的样本（判据 `mono_path_matches_a_duplicated_stereo_pair`）。
    pub fn process_mono(&mut self, samples: &mut [f32]) -> usize {
        let frames = samples.len();
        let mut eq_l = [0.0f32; EQ_CHUNK_FRAMES];
        let mut eq_r = [0.0f32; EQ_CHUNK_FRAMES];
        let mut start = 0usize;
        while start < frames {
            let n = EQ_CHUNK_FRAMES.min(frames - start);
            let end = start + n;
            for s in samples[start..end].iter_mut() {
                let (mono, _) = self.observe_input(*s, *s);
                *s = mono * self.input_gain;
            }
            if self.params.eq_enabled {
                // 两个声道都喂同一个信号：EQ 的左右状态因此保持相同，
                // 取左声道的输出就是这条单声道流的输出。
                self.eq.process(
                    &samples[start..end],
                    &samples[start..end],
                    &mut eq_l[..n],
                    &mut eq_r[..n],
                    self.params.eq,
                    self.sample_rate,
                );
                samples[start..end].copy_from_slice(&eq_l[..n]);
            }
            for s in samples[start..end].iter_mut() {
                let (out, _) = self.render_frame(*s, *s);
                *s = out;
            }
            start = end;
        }
        frames
    }

    // -- 逐帧内核 ----------------------------------------------------------

    /// ① 输入侧：清洗两个样本、记输入计量，返回清洗后的 `(l, r)`。
    ///
    /// 计量**不含**输入增益：`input_peak` / `input_rms` 量的是**进入通道条的电平**，
    /// 与推子 / 配平位置无关。要量配平之后的电平，用 `output_*`（或直接读 `compressor`
    /// 的 `level_db`）。
    #[inline]
    fn observe_input(&mut self, left: f32, right: f32) -> (f32, f32) {
        let l = sanitize_sample(left);
        let r = sanitize_sample(right);
        let peak = l.abs().max(r.abs());
        if peak > self.input_peak {
            self.input_peak = peak;
        }
        // 联动均方：与 compressor 检波器**同一个** `max(l², r²)` 口径
        // （见 `crates/yeban-dsp/src/compressor.rs` 的模块注释 §2）。
        self.input_mean_square += f64::from((l * l).max(r * r));
        self.frames = self.frames.saturating_add(1);
        (l, r)
    }

    /// ③④⑤ 一帧：滤波 → 动态 → 输出增益，并记输出计量。
    #[inline]
    fn render_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        // ③ 滤波（逐样本；**两个声道各一份独立状态**，见 `filter_l` 的文档）。
        let (l, r) = if self.params.filter_enabled {
            (self.filter_l.process(left), self.filter_r.process(right))
        } else {
            (left, right)
        };

        // ④ 动态：**一个**增益，两个声道共用（立体声联动，声像不被拉开）。
        let (l, r) = if self.params.compressor_enabled {
            let gain = self.compressor.process_gain(l, r);
            (l * gain, r * gain)
        } else {
            (l, r)
        };

        // ⑤ 输出增益。
        let l = l * self.output_gain;
        let r = r * self.output_gain;

        // 出口兜底：非有限值归零。见模块注释 §6。
        let l = finite_or_zero(l);
        let r = finite_or_zero(r);

        let peak = l.abs().max(r.abs());
        if peak > self.output_peak {
            self.output_peak = peak;
        }
        self.output_mean_square += f64::from((l * l).max(r * r));

        (l, r)
    }

    // -- 计量读取器 --------------------------------------------------------
    //
    // 口径（**与 compressor 检波器同一条**）：逐帧取两声道中较大者。对**不相关**
    // 立体声，联动 RMS 比"两声道各自的 RMS"高至多 3 dB（能量是两个声道之和时
    // 才准确）。这条选择让本模块只有一个立体声口径；逐声道计量是
    // `yeban_dsp::meter::LevelDetector` 与引擎 `MeterBank` 的职责，本票不接线。

    /// 输入峰值（**线性**满刻度，`1.0` = 0 dBFS），自上次 `reset` 起。
    #[must_use]
    pub const fn input_peak(&self) -> f32 {
        self.input_peak
    }

    /// 输入峰值（dBFS）。静音 ⇒ `f32::NEG_INFINITY`（[`crate::meter::dbfs`] 口径）。
    #[must_use]
    pub fn input_peak_dbfs(&self) -> f32 {
        dbfs(self.input_peak)
    }

    /// 输入 RMS（**线性**，自上次 `reset` 起的联动均方根）。
    #[must_use]
    pub fn input_rms(&self) -> f32 {
        if self.frames == 0 {
            0.0
        } else {
            (self.input_mean_square / self.frames as f64).sqrt() as f32
        }
    }

    /// 输入 RMS（dBFS）。
    #[must_use]
    pub fn input_rms_dbfs(&self) -> f32 {
        dbfs(self.input_rms())
    }

    /// 输出峰值（**线性**），自上次 `reset` 起。这是链**全部五级之后**的读数。
    #[must_use]
    pub const fn output_peak(&self) -> f32 {
        self.output_peak
    }

    /// 输出峰值（dBFS）。
    #[must_use]
    pub fn output_peak_dbfs(&self) -> f32 {
        dbfs(self.output_peak)
    }

    /// 输出 RMS（**线性**，自上次 `reset` 起的联动均方根）。
    #[must_use]
    pub fn output_rms(&self) -> f32 {
        if self.frames == 0 {
            0.0
        } else {
            (self.output_mean_square / self.frames as f64).sqrt() as f32
        }
    }

    /// 输出 RMS（dBFS）。
    #[must_use]
    pub fn output_rms_dbfs(&self) -> f32 {
        dbfs(self.output_rms())
    }

    /// 被压缩过的**帧**数（转自 [`Compressor::reduction_count`]）。
    ///
    /// ⚠ 它计的是压缩器的**帧**数（`process_gain` 的调用次数），不是声道数。
    /// 动态级被旁通时该计数**不推进**（旁通 = 整级不执行，§4）。
    #[must_use]
    pub const fn gain_reduction_count(&self) -> u64 {
        self.compressor.reduction_count()
    }

    /// 全程最大增益衰减（dB，`≥ 0`，转自 [`Compressor::max_reduction_db`]）。
    #[must_use]
    pub const fn max_gain_reduction_db(&self) -> f32 {
        self.compressor.max_reduction_db()
    }

    /// **当前**增益衰减（dB，`≥ 0`，转自 [`Compressor::gain_db`]）。
    #[must_use]
    pub fn current_gain_reduction_db(&self) -> f32 {
        -self.compressor.gain_db()
    }

    /// 动态级当前的输入电平估计（dBFS，转自 [`Compressor::level_db`]）。
    ///
    /// ⚠ 这是**压缩器检波器**看到的电平，即滤波级之后的电平（§2 顺序的直接后果）。
    #[must_use]
    pub const fn detector_level_db(&self) -> f32 {
        self.compressor.level_db()
    }

    /// 已处理的**帧**数（自上次 `reset`）。
    #[must_use]
    pub const fn processed_frames(&self) -> u64 {
        self.frames
    }
}

impl Default for ChannelStrip {
    fn default() -> Self {
        Self::new(ChannelStripParams::DEFAULT, 48_000.0)
    }
}

// ---------------------------------------------------------------------------
// 私有辅助
// ---------------------------------------------------------------------------

/// 钳到 `[low, high]`，且**非有限输入一律取 `fallback`**。
///
/// 与 [`crate::compressor`] 的 `clamp_low` 的差别只有 `fallback` 这一点：本模块的
/// 两个轴（dB 增益、截止频率）各自需要**不同的**保守侧，见 [`sanitise_gain_db`] 与
/// [`FilterParams::sanitised`]。
#[inline]
#[must_use]
fn clamp_or(value: f32, low: f32, high: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

/// 增益（dB）的钳制：非有限值归 **`0 dB`**（恒等，`db_to_gain(0) = 1.0`）。
#[inline]
#[must_use]
fn sanitise_gain_db(value: f32) -> f32 {
    clamp_or(value, MIN_GAIN_DB, MAX_GAIN_DB, 0.0)
}

/// EQ 参数的钳制。上下界与 fallback 与 `crates/yeban-dsp/src/shaping.rs` 的
/// `clamp_or` 调用点**逐个相同**（`shaping.rs:398-410`），因此本函数幂等、
/// 且与 `ShapingEq::process` 自己的钳制不冲突。
#[inline]
#[must_use]
fn sanitise_eq(params: EqParams) -> EqParams {
    EqParams {
        low_gain: clamp_or(params.low_gain, -18.0, 18.0, 0.0),
        low_freq: clamp_or(params.low_freq, 20.0, 1000.0, 200.0),
        mid_gain: clamp_or(params.mid_gain, -18.0, 18.0, 0.0),
        mid_freq: clamp_or(params.mid_freq, 200.0, 8000.0, 1000.0),
        mid_q: clamp_or(params.mid_q, 0.3, 6.0, 0.9),
        high_gain: clamp_or(params.high_gain, -18.0, 18.0, 0.0),
        high_freq: clamp_or(params.high_freq, 1000.0, 16000.0, 4000.0),
    }
}

/// `NaN` 与 `±∞` 归零，其它值原样返回（出口兜底，见模块注释 §6）。
#[inline]
#[must_use]
fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// [ARCH-DET-001] 用的确定性指纹：**FNV-1a 64**。
///
/// ⚠ 这是**代理指标**，不是密码学哈希：它只用来判"两段输出是否逐位相同"。
/// 与 `crates/yeban-dsp/src/compressor.rs` 的判据同一种做法（同一个常数与同一个
/// 逐字节递推），**不是** SHA-256。
#[cfg(test)]
#[must_use]
fn fnv1a64(samples: &[f32]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::gain_db_for;

    // -- 夹具 --------------------------------------------------------------

    /// 采样率（Hz）：本模块全部判据用它。
    const SR: f32 = 48_000.0;

    /// 含**高频**的复合测试信号：`220 Hz` 基波 + `9 kHz` 分量。
    ///
    /// 为什么需要高频分量：低通截止设在 `1 kHz` 时，`9 kHz` 分量是"滤波真的发生"
    /// 的可观测证据（§2 的判据 2）。全部样本**有限**且 |s| < 0.7。
    fn composite(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / SR;
                // 0.5·sin(220) + 0.2·sin(9000)：峰值 ≤ 0.7。
                0.5 * (core::f32::consts::TAU * 220.0 * t).sin()
                    + 0.2 * (core::f32::consts::TAU * 9_000.0 * t).sin()
            })
            .collect()
    }

    /// 只含低频的信号（`220 Hz`，幅度 `0.5`），用来把"滤波的影响"隔离掉。
    fn low_only(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| 0.5 * (core::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect()
    }

    /// 只开滤波不压缩的参数。
    fn filter_only(cutoff_hz: f32) -> ChannelStripParams {
        ChannelStripParams {
            eq_enabled: false,
            compressor_enabled: false,
            filter: FilterParams {
                cutoff_hz,
                resonance: 0.0,
                drive: 0.0,
            },
            ..ChannelStripParams::DEFAULT
        }
    }

    /// 只开压缩不滤波不 EQ 的参数。
    fn dynamics_only(compressor: CompressorParams) -> ChannelStripParams {
        ChannelStripParams {
            eq_enabled: false,
            filter_enabled: false,
            compressor_enabled: true,
            compressor,
            ..ChannelStripParams::DEFAULT
        }
    }

    /// 跑一遍立体声（两声道同一信号），返回左声道输出。
    fn run_mono_in_stereo(params: ChannelStripParams, input: &[f32]) -> Vec<f32> {
        let mut strip = ChannelStrip::new(params, SR);
        let mut left = input.to_vec();
        let mut right = input.to_vec();
        strip.process_stereo(&mut left, &mut right);
        left
    }

    /// 跑一遍单声道，返回`gain_reduction_count`（用来自检"这条参数真的压缩过"）。
    fn run_gain_reduction(params: ChannelStripParams, input: &[f32]) -> u64 {
        let mut strip = ChannelStrip::new(params, SR);
        let mut buf = input.to_vec();
        strip.process_mono(&mut buf);
        strip.gain_reduction_count()
    }

    // -- 1. 构造与参数域 ---------------------------------------------------

    /// 判据：`sanitised` 把每个字段钳进它文档里写明的域，且**幂等**。
    #[test]
    fn sanitised_clamps_every_field_into_its_documented_domain() {
        let hostile = ChannelStripParams {
            input_gain_db: f32::NAN,
            eq_enabled: true,
            eq: EqParams {
                low_gain: f32::NAN,
                low_freq: f32::INFINITY,
                mid_gain: -1.0e9,
                mid_freq: 1.0e9,
                mid_q: f32::NEG_INFINITY,
                high_gain: 1.0e9,
                high_freq: f32::NAN,
            },
            filter_enabled: true,
            filter: FilterParams {
                cutoff_hz: f32::NAN,
                resonance: 5.0,
                drive: -3.0,
            },
            compressor_enabled: true,
            compressor: CompressorParams {
                threshold_db: f32::NAN,
                ratio: f32::NAN,
                knee_db: f32::NAN,
                detector_s: f32::NAN,
                attack_s: f32::NAN,
                release_s: f32::NAN,
                makeup_db: f32::NAN,
            },
            output_gain_db: f32::INFINITY,
        };
        let p = hostile.sanitised();
        assert_eq!(p.input_gain_db, 0.0, "NaN 增益必须归到恒等（0 dB）");
        assert_eq!(p.output_gain_db, 0.0, "+∞ 增益必须归到恒等（0 dB）");
        assert_eq!(p.eq.low_gain, 0.0);
        assert_eq!(
            p.eq.low_freq, 200.0,
            "非有限 low_freq 取 shaping 的 fallback"
        );
        assert_eq!(p.eq.mid_gain, -18.0);
        assert_eq!(p.eq.mid_freq, 8000.0);
        assert_eq!(p.eq.mid_q, 0.9, "非有限 mid_q 取 shaping 的 fallback");
        assert_eq!(p.eq.high_gain, 18.0);
        assert_eq!(
            p.eq.high_freq, 4000.0,
            "NaN high_freq 取 shaping 的 fallback"
        );
        assert_eq!(
            p.filter.cutoff_hz, MAX_CUTOFF_HZ,
            "NaN 截止频率取最透明一侧"
        );
        assert_eq!(p.filter.resonance, 1.0);
        assert_eq!(p.filter.drive, 0.0);
        // 压缩器：全字段 `NaN` ⇒ `CompressorParams::sanitised` 的"NaN 归**下界**"规则，
        // 不是 `DEFAULT`（`DEFAULT` 是一组合法的中位值，NaN 到不了那里）。
        let c = p.compressor;
        assert_eq!(c.threshold_db, crate::compressor::MIN_LEVEL_DB);
        assert_eq!(c.ratio, 1.0, "NaN 比率必须归到 1（透明）");
        assert_eq!(c.knee_db, 0.0, "NaN 膝宽必须归到 0（硬膝）");
        assert_eq!(c.detector_s, crate::compressor::MIN_TIME_S);
        assert_eq!(c.attack_s, crate::compressor::MIN_TIME_S);
        assert_eq!(c.release_s, crate::compressor::MIN_TIME_S);
        assert_eq!(c.makeup_db, -crate::compressor::MAX_MAKEUP_DB);
        assert_eq!(p, p.sanitised(), "sanitised 必须幂等");
        // 每个字段都真的有限。
        assert!(p.input_gain_db.is_finite() && p.output_gain_db.is_finite());
        assert!(p.filter.cutoff_hz.is_finite() && p.filter.resonance.is_finite());
        assert!(p.eq.low_gain.is_finite() && p.eq.mid_q.is_finite());
    }

    /// 判据：非法采样率（`0` / 负 / `NaN`）被钳到 [`crate::MIN_SAMPLE_RATE`]，不 panic。
    #[test]
    fn illegal_sample_rates_are_clamped() {
        for bad in [0.0f32, -48_000.0, f32::NAN, f32::INFINITY] {
            let strip = ChannelStrip::new(ChannelStripParams::DEFAULT, bad);
            assert_eq!(strip.sample_rate(), crate::MIN_SAMPLE_RATE);
        }
        assert_eq!(
            ChannelStrip::new(ChannelStripParams::DEFAULT, 44_100.0).sample_rate(),
            44_100.0
        );
    }

    /// 判据：默认参数在 `0 dB` 时逐位透明 —— 但**只对不触发压缩的电平**成立。
    ///
    /// 这里把三级全部旁通，验证"旁通 + 0 dB 增益 = 恒等变换"这条构造性事实。
    #[test]
    fn an_all_bypassed_strip_at_unity_gain_is_bit_identical() {
        let params = ChannelStripParams {
            eq_enabled: false,
            filter_enabled: false,
            compressor_enabled: false,
            ..ChannelStripParams::DEFAULT
        };
        let input = composite(512);
        let output = run_mono_in_stereo(params, &input);
        for (i, (a, b)) in input.iter().zip(output.iter()).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "第 {i} 帧必须逐位相同");
        }
    }

    // -- 2. 信号顺序契约 ---------------------------------------------------

    /// 判据（**顺序契约的主判据**）：本器件与"按模块注释 §2 手工搭的同序参照链"
    /// **逐位相同**。
    ///
    /// 为什么这条能红：参照链把 ① 输入增益 → ② EQ → ③ 滤波 → ④ 动态 → ⑤ 输出增益
    /// 的**顺序与逐样本步骤**在判据里独立写了一遍。改动本模块链上任何一步的顺序、
    /// 位置或乘法的先后 ⇒ 两侧逐位不同 ⇒ 红。
    ///
    /// ⚠ 本判据**不**用冻结哈希常量：两侧都在**同一台机器上**跑，因此不受
    /// `sin` 之类的 libm 跨平台末位差异影响。
    ///
    /// ⚠ EQ 的参照用**每帧 1 个样本**的切片调用。这与模块内部的 64 帧分块等价，
    /// 依据是 `ShapingEq::process` 的系数重算只依赖 `(params, sample_rate)`（幂等）；
    /// 判据 `chunking_does_not_change_output` 实测了这一点。
    #[test]
    fn the_chain_matches_an_independently_built_reference_chain() {
        use crate::filter::LadderFilter;
        use crate::meter::sanitize_sample;
        use crate::shaping::ShapingEq;

        const LEN: usize = 4_096;
        let input = composite(LEN);
        let params = ChannelStripParams {
            input_gain_db: 4.0,
            eq: EqParams {
                low_gain: 5.0,
                mid_gain: -4.0,
                mid_freq: 1_500.0,
                mid_q: 1.5,
                high_gain: 7.0,
                ..EqParams::default()
            },
            filter: FilterParams {
                cutoff_hz: 3_000.0,
                resonance: 0.3,
                drive: 0.2,
            },
            compressor: CompressorParams {
                threshold_db: -22.0,
                ratio: 6.0,
                knee_db: 4.0,
                makeup_db: 2.0,
                ..CompressorParams::DEFAULT
            },
            output_gain_db: -2.0,
            ..ChannelStripParams::DEFAULT
        };
        let actual = run_mono_in_stereo(params, &input);

        // ---- 手工参照链（顺序见模块注释 §2）----
        let in_gain = db_to_gain(params.input_gain_db);
        let out_gain = db_to_gain(params.output_gain_db);
        let mut eq = ShapingEq::new();
        let mut filter = LadderFilter::new();
        filter.configure(
            SR,
            params.filter.cutoff_hz,
            params.filter.resonance,
            params.filter.drive,
        );
        let mut comp = Compressor::new(params.compressor, SR);
        let mut expected = Vec::with_capacity(LEN);
        for raw in &input {
            // ① 输入侧清洗 + 输入增益。
            let mut s = sanitize_sample(*raw) * in_gain;
            // ② EQ（1 帧切片）。
            let mut eq_out = [0.0f32; 1];
            let mut eq_out_r = [0.0f32; 1];
            eq.process(&[s], &[s], &mut eq_out, &mut eq_out_r, params.eq, SR);
            s = eq_out[0];
            // ③ 滤波。
            s = filter.process(s);
            // ④ 动态（一个联动增益）。
            let gain = comp.process_gain(s, s);
            s *= gain;
            // ⑤ 输出增益 + 出口兜底。
            s *= out_gain;
            expected.push(if s.is_finite() { s } else { 0.0 });
        }

        let differing = actual
            .iter()
            .zip(expected.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        let max_delta = actual
            .iter()
            .zip(expected.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "[channel_strip/顺序契约] 本器件 vs 手工同序参照链：逐位不同的帧 {differing} / {LEN}，最大绝对差 {max_delta:.3e}"
        );
        assert!(
            differing == 0,
            "本器件的输出与按 §2 顺序手工搭的参照链不同（{differing} / {LEN} 帧，最大差 {max_delta:.3e}）⇒ 链的顺序或步骤被改动了"
        );
        // 覆盖度自检：这条判据必须真的压缩过（否则它可能比较的是两条恒等链）。
        assert!(
            run_gain_reduction(params, &input) > 0,
            "参照参数没有触发压缩 ⇒ 本判据没有覆盖动态级的位置"
        );
        assert!(
            !expected.iter().all(|s| *s == 0.0),
            "参照链输出全 0 ⇒ 判据测错了对象"
        );
    }

    /// 判据（**顺序契约**）：把"滤波"与"动态"两级互换，输出**必须不同**。
    ///
    /// 量什么：同一条输入经过 (滤波 → 压缩) 与 (压缩 → 滤波) 两条链之后，
    /// 左声道 4 096 个样本的逐位差异计数（单位：**帧**）。
    ///
    /// 为什么这条能红：交换顺序会让 (a) 压缩器检波器看到不同带宽的信号
    /// (b) 滤波器处理不同幅度的信号（它的饱和级非线性）。若某次重构把顺序改掉，
    /// 差异计数会变成 0 ⇒ 本判据红。
    ///
    /// ⚠ 互换后的链**不是**本模块的公开 API。本判据手工按互换顺序组合同一批既有
    /// 器件（`LadderFilter` / `Compressor`），这正是"顺序可观测"的最小证明。
    #[test]
    fn stage_order_is_the_contract() {
        const LEN: usize = 4_096;
        let input = composite(LEN);
        // 低声平信号，避免饱和与压缩把两条链的差别掩盖掉。幅度已由 `composite` 限定。

        // (A) 本模块的顺序：滤波 → 压缩（用**公开 API**：把 EQ 旁通，只留③④）。
        let params = ChannelStripParams {
            eq_enabled: false,
            filter_enabled: true,
            compressor_enabled: true,
            filter: FilterParams {
                cutoff_hz: 1_000.0,
                resonance: 0.2,
                drive: 0.0,
            },
            compressor: CompressorParams {
                threshold_db: -18.0,
                ratio: 4.0,
                ..CompressorParams::DEFAULT
            },
            ..ChannelStripParams::DEFAULT
        };
        let filter_then_comp = run_mono_in_stereo(params, &input);

        // (B) 互换后的顺序：压缩 → 滤波。手工组合同一批器件。
        let mut comp = Compressor::new(params.compressor, SR);
        let mut filter = LadderFilter::new();
        filter.configure(
            SR,
            params.filter.cutoff_hz,
            params.filter.resonance,
            params.filter.drive,
        );
        let swapped: Vec<f32> = input
            .iter()
            .map(|s| {
                let g = comp.process_gain(*s, *s);
                filter.process(*s * g)
            })
            .collect();

        let differing = filter_then_comp
            .iter()
            .zip(swapped.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        let max_delta = filter_then_comp
            .iter()
            .zip(swapped.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "[channel_strip/顺序] 滤波→压缩 与 压缩→滤波：逐位不同的帧 {differing} / {LEN}，最大绝对差 {max_delta:.6e}"
        );
        assert!(
            differing > 0,
            "交换滤波与压缩之后输出逐位相同 ⇒ 顺序不是契约（判据失效）"
        );
        // 反方向的牙：两条链不能只是差在 1 ULP 的浮点噪声上。
        assert!(
            max_delta > 1.0e-3,
            "两条链的最大绝对差只有 {max_delta:.6e} ⇒ 顺序差别不可观测"
        );
    }

    /// 判据（**顺序契约的第二个方向**）：EQ 在滤波之前。
    ///
    /// 同一条信号走 (EQ 高架提升 → 低通) 与本模块的 (EQ → 滤波) 一致；把低通换成
    /// 放在 EQ **之前**（低通 → EQ）⇒ 输出必须不同。
    #[test]
    fn eq_before_filter_is_observable() {
        const LEN: usize = 4_096;
        let input = composite(LEN);
        let params = ChannelStripParams {
            eq_enabled: true,
            eq: EqParams {
                high_gain: 12.0,
                high_freq: 4_000.0,
                ..EqParams::default()
            },
            filter_enabled: true,
            compressor_enabled: false,
            filter: FilterParams {
                cutoff_hz: 1_000.0,
                resonance: 0.0,
                drive: 0.0,
            },
            ..ChannelStripParams::DEFAULT
        };
        let eq_then_filter = run_mono_in_stereo(params, &input);

        // 互换：先低通，再 EQ。
        let mut filter = LadderFilter::new();
        filter.configure(SR, 1_000.0, 0.0, 0.0);
        let pre_filtered: Vec<f32> = input.iter().map(|s| filter.process(*s)).collect();
        let filter_then_eq = run_mono_in_stereo(
            ChannelStripParams {
                filter_enabled: false,
                ..params
            },
            &pre_filtered,
        );

        let differing = eq_then_filter
            .iter()
            .zip(filter_then_eq.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        eprintln!("[channel_strip/顺序] EQ→滤波 与 滤波→EQ：逐位不同的帧 {differing} / {LEN}");
        assert!(
            differing > 0,
            "交换 EQ 与滤波之后输出逐位相同 ⇒ 顺序不是契约"
        );
    }

    // -- 3. 各器件真的在工作（数值证据） -----------------------------------

    /// 判据：**滤波真的发生** —— 低通把 `9 kHz` 分量压下去。
    ///
    /// 量什么（两个可观测读数，单位都写明）：
    ///
    /// 1. **分量幅度**（线性满刻度）：用整数周期 DFT 单 bin 相关法量
    ///    `220 Hz` 与 `9 kHz` 两个分量在输入 / 输出里的幅度。窗口 `48 000` 帧
    ///    在 48 kHz 下对 `220 Hz` 与 `9 000 Hz` **都是整数个周期**，因此单 bin
    ///    相关**无泄漏**。
    /// 2. **输出 RMS**（线性满刻度）。
    ///
    /// 算术对比（手算）：
    ///
    /// - 输入分量幅度是 `220 Hz → 0.5`、`9 kHz → 0.2`。
    /// - 截止 `20 kHz` ⇒ 两个分量都留下 ⇒ 总 RMS 的理论值是
    ///   `√(0.5² + 0.2²)/√2 = √0.29/√2 = 0.38079`（实测 `0.37765`，差 −0.07 dB）。
    /// - 截止 `1 kHz` ⇒ `9 kHz` 被四极低通压掉 **≥ 40 dB**（实测 **75.6 dB**）。
    ///   ⚠ **不能**假设 `220 Hz` 落在平坦通带里：本仓库的 `LadderFilter` 是
    ///   TPT/ZDF 梯形低通，在 `220 Hz / 1 kHz = 0.22×` 截止处实测已有 **0.82 dB**
    ///   的衰减（`0.5 → 0.45490`）。因此这条判据不用"纯 220 Hz 的 `0.35355`"当
    ///   参照，而是用**同一台机器上量到的 `220 Hz` 分量**当参照 —— 这样断言的是
    ///   "滤波之后剩下的**就是**那个 220 Hz 分量"，而不是一个未经核实的通带假设。
    /// - 两条 RMS 的**理论差只有 `20·log10(0.38079/0.35355) = 0.646 dB`**（因为被去掉
    ///   的分量本来就比基波低 7.96 dB）⇒ **分量幅度**才是主要读数，RMS 只证方向。
    #[test]
    fn the_filter_really_removes_the_high_component() {
        const LEN: usize = 48_000;
        let input = composite(LEN);
        let input_rms = rms(&input);
        let in_220 = tone_amplitude(&input, 220.0);
        let in_9k = tone_amplitude(&input, 9_000.0);

        let mut narrow = ChannelStrip::new(filter_only(1_000.0), SR);
        let mut wide = ChannelStrip::new(filter_only(MAX_CUTOFF_HZ), SR);
        let mut a = input;
        let mut b = a.clone();
        narrow.process_mono(&mut a);
        wide.process_mono(&mut b);

        let rms_narrow = rms(&a);
        let rms_wide = rms(&b);
        let narrow_db = dbfs(rms_narrow);
        let wide_db = dbfs(rms_wide);

        // 输出侧的两个分量幅度。
        let narrow_220 = tone_amplitude(&a, 220.0);
        let narrow_9k = tone_amplitude(&a, 9_000.0);
        let wide_220 = tone_amplitude(&b, 220.0);
        let wide_9k = tone_amplitude(&b, 9_000.0);
        eprintln!(
            "[channel_strip/滤波] 输入 RMS {input_rms:.6} ({:.3} dBFS)；输入分量 220 Hz {in_220:.6} / 9 kHz {in_9k:.6}",
            dbfs(input_rms)
        );
        eprintln!(
            "[channel_strip/滤波] 截止 20 kHz: 输出 RMS {rms_wide:.6} ({wide_db:.3} dBFS)，220 Hz {wide_220:.6}，9 kHz {wide_9k:.6} ({:.3} dBFS)",
            dbfs(wide_9k)
        );
        eprintln!(
            "[channel_strip/滤波] 截止  1 kHz: 输出 RMS {rms_narrow:.6} ({narrow_db:.3} dBFS)，220 Hz {narrow_220:.6}，9 kHz {narrow_9k:.6} ({:.3} dBFS)",
            dbfs(narrow_9k)
        );
        eprintln!(
            "[channel_strip/滤波] 9 kHz 分量被压掉的量: {:.3} dB（截止 1 kHz 相对截止 20 kHz）",
            dbfs(narrow_9k) - dbfs(wide_9k)
        );

        // ① 主读数：9 kHz 分量被压掉 ≥ 40 dB（四极低通在 9× 截止处的理论值远大于此）。
        let killed_db = dbfs(wide_9k) - dbfs(narrow_9k);
        assert!(
            killed_db > 40.0,
            "低通只把 9 kHz 压了 {killed_db:.3} dB（要求 > 40 dB）⇒ 滤波级没工作"
        );
        // ② 基波基本不动（±1.5 dB；实测这一版梯形低通在 0.22× 截止处有 0.82 dB 的衰减）。
        let kept_db = dbfs(narrow_220) - dbfs(wide_220);
        assert!(
            kept_db.abs() < 1.5,
            "低通把 220 Hz 基波改了 {kept_db:.3} dB（要求 |·| < 1.5 dB）"
        );
        // ③ 窄带输出的总 RMS 应当**就是那一个 220 Hz 分量**的 RMS（±0.1 dB）。
        //    `tone_amplitude` 量的正弦振幅 A，其 RMS = A/√2。
        let narrow_220_rms = narrow_220 / core::f32::consts::SQRT_2;
        let narrow_error_db = dbfs(rms_narrow) - dbfs(narrow_220_rms);
        assert!(
            narrow_error_db.abs() < 0.1,
            "窄带输出 RMS {rms_narrow:.6} 与它自己的 220 Hz 分量 RMS {narrow_220_rms:.6} 差 {narrow_error_db:.3} dB（要求 |·| < 0.1 dB）⇒ 输出里还有别的东西"
        );
        // ④ 宽带输出应当接近两个分量的合成 0.38079（±0.5 dB）。
        let expected_both = (0.25f32 + 0.04).sqrt() / core::f32::consts::SQRT_2;
        let wide_error_db = dbfs(rms_wide) - dbfs(expected_both);
        assert!(
            wide_error_db.abs() < 0.5,
            "宽带输出 {rms_wide:.6} 与 {expected_both:.6} 差 {wide_error_db:.3} dB（超出 0.5 dB 容差）"
        );
        // ⑤ 方向：窄带 RMS 必须更低（理论差 0.646 dB，取 0.3 dB 作方向阈值）。
        assert!(
            narrow_db < wide_db - 0.3,
            "低通没有降低总电平：窄带 {narrow_db:.3} dBFS 对宽带 {wide_db:.3} dBFS"
        );
    }

    /// 判据：**EQ 真的发生** —— 高架提升把输出电平抬起来。
    ///
    /// 量什么：平坦 EQ 与 `high_gain = +12 dB @ 4 kHz` 两种设置下，滤波与压缩都旁通时
    /// 输出 RMS 的差（dB）。
    #[test]
    fn the_eq_really_changes_the_level() {
        const LEN: usize = 48_000;
        let input = composite(LEN);
        let flat = run_mono_in_stereo(
            ChannelStripParams {
                filter_enabled: false,
                compressor_enabled: false,
                ..ChannelStripParams::DEFAULT
            },
            &input,
        );
        let boosted = run_mono_in_stereo(
            ChannelStripParams {
                eq: EqParams {
                    high_gain: 12.0,
                    high_freq: 4_000.0,
                    ..EqParams::default()
                },
                filter_enabled: false,
                compressor_enabled: false,
                ..ChannelStripParams::DEFAULT
            },
            &input,
        );
        let flat_rms = rms(&flat);
        let boosted_rms = rms(&boosted);
        eprintln!(
            "[channel_strip/EQ] 平坦 EQ 输出 RMS {:.6} ({:.3} dBFS) | 高架 +12 dB @4 kHz 输出 RMS {:.6} ({:.3} dBFS) | 提升 {:.3} dB",
            flat_rms,
            dbfs(flat_rms),
            boosted_rms,
            dbfs(boosted_rms),
            dbfs(boosted_rms) - dbfs(flat_rms)
        );
        assert!(
            dbfs(boosted_rms) > dbfs(flat_rms) + 1.0,
            "高架提升没有抬高输出电平 ⇒ EQ 级没工作"
        );
        // 平坦 EQ 与"EQ 全旁通"**不是逐位相同**：`ShapingEq` 的架子/峰值在 0 dB 时
        // 系数在数学上恒等（`b0 = 1`、`b1 = a1`、`b2 = a2`），但双二阶的**直接型**
        // 递推在 `f32` 下会留下末位舍入。`crates/yeban-dsp/src/shaping.rs:798` 的
        // `eq_flat_is_unity` 断言的是**幅频响应** `|gain| < 0.05 dB`，不是逐位恒等
        // ⇒ 这里只能断言"差异极小"，并把它打印出来。**这条是本票的实测发现**：
        // 本模块**不**声称平坦 EQ 是逐位恒等变换。
        let eq_off = run_mono_in_stereo(
            ChannelStripParams {
                eq_enabled: false,
                filter_enabled: false,
                compressor_enabled: false,
                ..ChannelStripParams::DEFAULT
            },
            &input,
        );
        let max_delta_flat = flat
            .iter()
            .zip(eq_off.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        let differing = flat
            .iter()
            .zip(eq_off.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        eprintln!(
            "[channel_strip/EQ] 平坦 EQ 与 EQ 旁通：逐位不同帧 {differing} / {LEN}，最大绝对差 {max_delta_flat:.3e}（平坦 EQ 不是逐位恒等，见判据注释）"
        );
        assert!(
            max_delta_flat < 1.0e-3,
            "平坦 EQ 与旁通的差达到 {max_delta_flat:.3e} ⇒ 平坦 EQ 不只是舍入"
        );
    }

    /// 判据：**压缩真的发生** —— 实测增益衰减与静态曲线的**算术**对账。
    ///
    /// 量什么：正弦（幅度 `0.7`，即 RMS `0.7/√2` ⇒ −6.10 dBFS）在
    /// 阈值 `−18 dBFS` / 比率 `4:1` / 膝 `0`（硬膝）下的稳态增益衰减（dB）。
    ///
    /// 算术对比（手算，硬膝、膝上分支）：
    /// `gain_db = (1/R − 1)·(L − T) = (1/4 − 1)·(−6.10 + 18) = −0.75 × 11.90 = −8.92 dB`。
    /// 也等于 [`gain_db_for`] 的返回值。判据要求实测 `max_gain_reduction_db`
    /// 与算术值相差 **< 0.5 dB**（剩下的差来自检波器的一阶平均纹波与增益弹道的
    /// 指数收敛，两者在 1 秒窗口末尾都已收敛）。
    #[test]
    fn compression_happens_and_matches_the_arithmetic() {
        const LEN: usize = 48_000;
        let amplitude = 0.7f32;
        let input: Vec<f32> = (0..LEN)
            .map(|i| amplitude * (core::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect();
        // 硬膝（knee = 0）⇒ 静态曲线是精确的两段直线，可以手算对账。
        let comp_params = CompressorParams {
            threshold_db: -18.0,
            ratio: 4.0,
            knee_db: 0.0,
            makeup_db: 0.0,
            ..CompressorParams::DEFAULT
        };
        let params = dynamics_only(comp_params);

        let mut strip = ChannelStrip::new(params, SR);
        let mut mono = input.clone();
        strip.process_mono(&mut mono);

        let input_rms_db = dbfs(amplitude / core::f32::consts::SQRT_2);
        let output_rms_db = strip.output_rms_dbfs();
        let measured_gr = strip.max_gain_reduction_db();
        let arithmetic_gr = -gain_db_for(input_rms_db, comp_params);

        eprintln!(
            "[channel_strip/压缩] 输入 RMS {:.6} ({:.3} dBFS) | 输出 RMS {:.6} ({:.3} dBFS) | 实测衰减 {:.3} dB | 算术衰减 {:.3} dB | 差 {:.3} dB",
            rms(&input),
            input_rms_db,
            strip.output_rms(),
            output_rms_db,
            measured_gr,
            arithmetic_gr,
            (measured_gr - arithmetic_gr).abs()
        );
        assert!(
            strip.gain_reduction_count() > 0,
            "窗口里从未压缩过 ⇒ 判据测错了对象"
        );
        assert!(
            (measured_gr - arithmetic_gr).abs() < 0.5,
            "实测衰减 {measured_gr:.3} dB 与算术 {arithmetic_gr:.3} dB 差 {:.3} dB（超出 0.5 dB 容差）",
            (measured_gr - arithmetic_gr).abs()
        );
        // 方向：输出 RMS 必须低于输入 RMS（压缩 + 无 makeup）。
        assert!(
            output_rms_db < input_rms_db - 4.0,
            "输出 {output_rms_db:.3} dBFS 没有明显低于输入 {input_rms_db:.3} dBFS"
        );
    }

    /// 判据：**旁通开关真的有牙** —— 关掉动态级，增益衰减计数不再推进。
    #[test]
    fn bypassing_the_dynamics_stops_the_reduction_counter() {
        let input = low_only(4_800);
        let on = {
            let mut strip = ChannelStrip::new(
                ChannelStripParams {
                    eq_enabled: false,
                    filter_enabled: false,
                    compressor_enabled: true,
                    compressor: CompressorParams {
                        threshold_db: -30.0,
                        ratio: 8.0,
                        ..CompressorParams::DEFAULT
                    },
                    ..ChannelStripParams::DEFAULT
                },
                SR,
            );
            let mut buf = input.clone();
            strip.process_mono(&mut buf);
            (strip.gain_reduction_count(), strip.max_gain_reduction_db())
        };
        let off = {
            let mut strip = ChannelStrip::new(
                ChannelStripParams {
                    eq_enabled: false,
                    filter_enabled: false,
                    compressor_enabled: false,
                    compressor: CompressorParams {
                        threshold_db: -30.0,
                        ratio: 8.0,
                        ..CompressorParams::DEFAULT
                    },
                    ..ChannelStripParams::DEFAULT
                },
                SR,
            );
            let mut buf = input;
            strip.process_mono(&mut buf);
            (strip.gain_reduction_count(), strip.max_gain_reduction_db())
        };
        eprintln!(
            "[channel_strip/旁通] 动态开: 计数 {} 最大衰减 {:.3} dB | 动态关: 计数 {} 最大衰减 {:.3} dB",
            on.0, on.1, off.0, off.1
        );
        assert!(on.0 > 0, "动态级开着却不压缩 ⇒ 判据测错了对象");
        assert_eq!(off.0, 0, "动态级旁通后计数仍推进 ⇒ 旁通开关没牙");
        assert_eq!(off.1, 0.0, "动态级旁通后最大衰减非 0");
    }

    // -- 4. 数值边界 -------------------------------------------------------

    /// 判据：敌意输入（`NaN` / `±∞` / 超幅 / 极端参数）不得产出非有限值，也不得 panic。
    #[test]
    fn hostile_inputs_never_escape_as_non_finite() {
        let hostile_input = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e30,
            -1.0e30,
            16.0,
            -16.0,
            0.0,
            1.0e-40,
        ];
        let hostile_params = [
            ChannelStripParams::DEFAULT,
            ChannelStripParams {
                input_gain_db: MAX_GAIN_DB,
                output_gain_db: MAX_GAIN_DB,
                eq: EqParams {
                    low_gain: 18.0,
                    mid_gain: 18.0,
                    high_gain: 18.0,
                    mid_q: 6.0,
                    ..EqParams::default()
                },
                filter: FilterParams {
                    cutoff_hz: MIN_CUTOFF_HZ,
                    resonance: 1.0,
                    drive: 1.0,
                },
                compressor: CompressorParams {
                    threshold_db: -120.0,
                    ratio: 1_000.0,
                    knee_db: 0.0,
                    detector_s: 0.000_1,
                    attack_s: 0.000_1,
                    release_s: 0.000_1,
                    makeup_db: 24.0,
                },
                ..ChannelStripParams::DEFAULT
            },
            ChannelStripParams {
                input_gain_db: MIN_GAIN_DB,
                output_gain_db: MIN_GAIN_DB,
                ..ChannelStripParams::DEFAULT
            },
        ];

        let mut finite_outputs = 0usize;
        for params in hostile_params {
            // 立体声路径
            let mut strip = ChannelStrip::new(params, SR);
            let mut left = hostile_input.to_vec();
            let mut right = hostile_input.iter().rev().copied().collect::<Vec<f32>>();
            strip.process_stereo(&mut left, &mut right);
            assert!(
                left.iter().chain(right.iter()).all(|s| s.is_finite()),
                "立体声路径产出了非有限值: {left:?} / {right:?}"
            );
            assert!(strip.output_peak().is_finite() && strip.output_rms().is_finite());
            assert!(strip.input_peak().is_finite() && strip.input_rms().is_finite());
            finite_outputs += left.len() + right.len();

            // 单声道路径
            let mut strip = ChannelStrip::new(params, SR);
            let mut mono = hostile_input.to_vec();
            strip.process_mono(&mut mono);
            assert!(
                mono.iter().all(|s| s.is_finite()),
                "单声道路径产出了非有限值: {mono:?}"
            );
            finite_outputs += mono.len();
        }
        eprintln!(
            "[channel_strip/边界] 敌意输入 × 敌对参数：检查了 {finite_outputs} 个输出样本，全部有限"
        );
        assert!(finite_outputs > 0);
    }

    /// 判据：**长时间满幅方波**不把状态推到非有限。
    ///
    /// 方波在每个半周期都有阶跃，是"弹道 + 滤波器饱和"的最坏情况输入。
    #[test]
    fn a_long_full_scale_square_wave_stays_finite() {
        const LEN: usize = 96_000;
        let params = ChannelStripParams {
            input_gain_db: MAX_GAIN_DB,
            eq: EqParams {
                low_gain: 18.0,
                mid_gain: 18.0,
                high_gain: 18.0,
                ..EqParams::default()
            },
            filter: FilterParams {
                cutoff_hz: 800.0,
                resonance: 1.0,
                drive: 1.0,
            },
            compressor: CompressorParams {
                threshold_db: -60.0,
                ratio: 1_000.0,
                knee_db: 0.0,
                ..CompressorParams::DEFAULT
            },
            output_gain_db: MAX_GAIN_DB,
            ..ChannelStripParams::DEFAULT
        };
        let mut strip = ChannelStrip::new(params, SR);
        let mut buf: Vec<f32> = (0..LEN)
            .map(|i| if (i / 24) % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        strip.process_mono(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()), "满幅方波推爆了状态");
        eprintln!(
            "[channel_strip/边界] 满幅方波 {LEN} 帧：输出峰值 {:.6} ({:.3} dBFS)，最大衰减 {:.3} dB",
            strip.output_peak(),
            strip.output_peak_dbfs(),
            strip.max_gain_reduction_db()
        );
    }

    /// 判据：空切片与不等长切片不 panic，返回值等于 `min(len)`。
    #[test]
    fn degenerate_slice_lengths_do_not_panic() {
        let mut strip = ChannelStrip::default();
        assert_eq!(strip.process_stereo(&mut [], &mut []), 0);
        assert_eq!(strip.process_mono(&mut []), 0);
        let mut left = [0.1f32; 8];
        let mut right = [0.1f32; 3];
        assert_eq!(strip.process_stereo(&mut left, &mut right), 3);
        assert_eq!(strip.processed_frames(), 3);
    }

    // -- 5. 确定性与块切分 -------------------------------------------------

    /// 判据（[ARCH-DET-001]）：同一输入跑两次 ⇒ 输出**逐位相同**（FNV-1a 64 代理指标）。
    #[test]
    fn the_same_input_twice_is_bit_identical() {
        let params = ChannelStripParams::DEFAULT;
        let input = composite(4_096);
        let first = run_mono_in_stereo(params, &input);
        let second = run_mono_in_stereo(params, &input);
        let h1 = fnv1a64(&first);
        let h2 = fnv1a64(&second);
        let differing = first
            .iter()
            .zip(second.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        eprintln!(
            "[channel_strip/确定性] 同输入两次：FNV-1a 64 = {h1:#018x} / {h2:#018x}（代理指标），逐位不同帧 {differing} / {}",
            first.len()
        );
        assert_eq!(h1, h2);
        assert_eq!(differing, 0);
    }

    /// 判据（[ARCH-DET-001]）：**不同块切分 ⇒ 输出逐位相同**。
    ///
    /// 量什么：同一条 4 096 帧输入按 `1 / 3 / 7 / 63 / 64 / 65 / 128 / 999 / 4096`
    /// 九种切分喂给**同一个**参数下的九个新实例，输出与"一次喂完"的参照逐位比对。
    ///
    /// 覆盖点：`63/64/65` 跨过内部的 [`EQ_CHUNK_FRAMES`] 边界，`1` 是最极端的切分。
    #[test]
    fn chunking_does_not_change_output() {
        const LEN: usize = 4_096;
        let params = ChannelStripParams::DEFAULT;
        let input = composite(LEN);
        let reference = run_mono_in_stereo(params, &input);
        let reference_hash = fnv1a64(&reference);
        eprintln!(
            "[channel_strip/切分] 参照（一次 {LEN} 帧）FNV-1a 64 = {reference_hash:#018x}（代理指标）"
        );
        for chunk in [1usize, 3, 7, 63, 64, 65, 128, 999, 4_096] {
            let mut strip = ChannelStrip::new(params, SR);
            let mut left = input.clone();
            let mut right = input.clone();
            let mut start = 0usize;
            while start < LEN {
                let end = (start + chunk).min(LEN);
                strip.process_stereo(&mut left[start..end], &mut right[start..end]);
                start = end;
            }
            let differing = reference
                .iter()
                .zip(left.iter())
                .filter(|(a, b)| a.to_bits() != b.to_bits())
                .count();
            eprintln!(
                "[channel_strip/切分] 块长 {chunk:>4}：FNV-1a 64 = {:#018x}，逐位不同帧 {differing} / {LEN}",
                fnv1a64(&left)
            );
            assert_eq!(
                differing, 0,
                "块长 {chunk} 改变了输出 ⇒ 器件不是逐样本前向依赖"
            );
            assert_eq!(fnv1a64(&left), reference_hash, "块长 {chunk} 的指纹不同");
        }
    }

    /// 判据：单声道路径与"两个声道填同一个信号"的立体声路径**逐位相同**。
    #[test]
    fn mono_path_matches_a_duplicated_stereo_pair() {
        let params = ChannelStripParams::DEFAULT;
        let input = composite(2_048);
        let mut strip_stereo = ChannelStrip::new(params, SR);
        let mut left = input.clone();
        let mut right = input.clone();
        strip_stereo.process_stereo(&mut left, &mut right);
        let mut strip_mono = ChannelStrip::new(params, SR);
        let mut mono = input;
        strip_mono.process_mono(&mut mono);
        let differing = left
            .iter()
            .zip(mono.iter())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        eprintln!(
            "[channel_strip/单声道] 单声道路径 vs 复制成对的立体声：逐位不同帧 {differing} / {}",
            left.len()
        );
        assert_eq!(differing, 0);
        assert_eq!(
            strip_stereo.gain_reduction_count(),
            strip_mono.gain_reduction_count()
        );
    }

    // -- 6. 参数与采样率切换 -----------------------------------------------

    /// 判据：`set_params` / `set_sample_rate` 能在**运行中**被调用，且不清空计量。
    #[test]
    fn set_params_and_sample_rate_are_live_and_allocation_free_paths() {
        let mut strip = ChannelStrip::new(ChannelStripParams::DEFAULT, SR);
        let mut buf = composite(512);
        strip.process_mono(&mut buf);
        let frames_before = strip.processed_frames();
        assert_eq!(frames_before, 512);

        strip.set_params(ChannelStripParams {
            input_gain_db: -6.0,
            filter: FilterParams {
                cutoff_hz: 2_000.0,
                resonance: 0.5,
                drive: 0.5,
            },
            ..ChannelStripParams::DEFAULT
        });
        assert_eq!(strip.params().input_gain_db, -6.0);
        strip.set_sample_rate(96_000.0);
        assert_eq!(strip.sample_rate(), 96_000.0);
        // 计量**不清空**（与"set_params 不清状态"同纪律）。
        assert_eq!(strip.processed_frames(), frames_before);

        let mut buf2 = composite(512);
        let mut buf3 = buf2.clone();
        strip.process_stereo(&mut buf2, &mut buf3);
        assert_eq!(strip.processed_frames(), frames_before + 512);
        assert!(buf2.iter().all(|s| s.is_finite()));

        // `reset` 清空全部状态与读数。
        strip.reset();
        assert_eq!(strip.processed_frames(), 0);
        assert_eq!(strip.input_peak(), 0.0);
        assert_eq!(strip.output_peak(), 0.0);
        assert_eq!(strip.input_rms(), 0.0);
        assert_eq!(strip.output_rms(), 0.0);
        assert_eq!(strip.gain_reduction_count(), 0);
        assert_eq!(strip.max_gain_reduction_db(), 0.0);
    }

    /// 判据：增益级是**乘性**的，且输入计量**不含**配平。
    #[test]
    fn input_metering_excludes_the_trim_and_the_gains_are_multiplicative() {
        let input = low_only(1_000);
        // 三级全旁通 ⇒ 输出 = 输入 × in_gain × out_gain，且输入计量 = 原始输入。
        let params = ChannelStripParams {
            input_gain_db: 6.0,
            eq_enabled: false,
            filter_enabled: false,
            compressor_enabled: false,
            output_gain_db: -6.0,
            ..ChannelStripParams::DEFAULT
        };
        let mut strip = ChannelStrip::new(params, SR);
        let mut buf = input.clone();
        strip.process_mono(&mut buf);
        let gain = db_to_gain(6.0) * db_to_gain(-6.0);
        for (i, (a, b)) in input.iter().zip(buf.iter()).enumerate() {
            let expected = a * gain;
            assert!(
                (b - expected).abs() < 1.0e-6,
                "第 {i} 帧 {b} 与 a·gain {expected} 不符"
            );
        }
        let raw_peak = input.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            (strip.input_peak() - raw_peak).abs() < 1.0e-6,
            "输入峰值 {} 应当等于原始峰值 {raw_peak}（不含配平）",
            strip.input_peak()
        );
    }

    /// 判据：`latency_samples` 恒为 0（不参与 PDC）。
    #[test]
    fn latency_is_zero() {
        assert_eq!(ChannelStrip::default().latency_samples(), 0);
    }

    // -- 夹具辅助 ----------------------------------------------------------

    /// 线性 RMS（未经联动，把这个切片当一个声道量）。
    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    /// 用**整数周期** DFT 单 bin 相关法量某个频率的分量幅度（线性满刻度）。
    ///
    /// `2·|Σ s[n]·e^(−jωn)| / N`。当窗口长度是该频率的整数个周期时无泄漏。
    /// 判据用的窗口 `48 000` 帧 @ 48 kHz 对 `220 Hz`（220 周期）与 `9 000 Hz`
    /// （9 000 周期）都满足这一条。
    fn tone_amplitude(samples: &[f32], freq_hz: f32) -> f32 {
        let n = samples.len();
        if n == 0 {
            return 0.0;
        }
        let mut re = 0.0f64;
        let mut im = 0.0f64;
        for (i, s) in samples.iter().enumerate() {
            let phase = core::f64::consts::TAU * f64::from(freq_hz) * i as f64 / f64::from(SR);
            re += f64::from(*s) * phase.cos();
            im += f64::from(*s) * phase.sin();
        }
        2.0 * ((re * re + im * im).sqrt() / n as f64) as f32
    }

    /// **判据（新写，可红）**：立体声计量取**较响**的那一声道（两条链的四项读数）。
    ///
    /// 量什么：`input_peak()` / `input_rms()` / `output_peak()` / `output_rms()`
    /// （前两项线性幅度、后两项线性 RMS）。
    ///
    /// 模块注释的"计量读取器"写明逐帧取两声道中**较大者**（与压缩机检波器同一个
    /// `max(l², r²)` 口径）。既有判据的夹具全部走 `process_mono`（左右同值）或
    /// 两级相同时的立体声 ⇒ `max` 与 `min` 在那些夹具上不可分辨。注入实测：
    /// `input_mean_square` 的 `.max(r * r)` 改成 `.min(...)`、`output_mean_square`
    /// 同样一处改坏 ⇒ 两次都**全绿** ⇒ 本判据的前两项与后两项分别变红。
    ///
    /// 夹具把三级全旁通、两个增益都为 `0 dB` ⇒ 输出等于输入，四项读数都应是 `1.0`
    /// （较响的 L 声道），与 `.min` 给出的 `0.0` 正好互相排斥。
    #[test]
    fn stereo_metering_follows_the_louder_channel() {
        let params = ChannelStripParams {
            input_gain_db: 0.0,
            eq_enabled: false,
            filter_enabled: false,
            compressor_enabled: false,
            output_gain_db: 0.0,
            ..ChannelStripParams::DEFAULT
        };
        let mut strip = ChannelStrip::new(params, SR);
        const FRAMES: usize = 512;
        let mut left = [1.0f32; FRAMES];
        let mut right = [0.0f32; FRAMES];
        strip.process_stereo(&mut left, &mut right);
        assert!(
            (strip.input_peak() - 1.0).abs() < 1.0e-6,
            "输入峰值 {} 必须取较响的 L 声道（1.0）",
            strip.input_peak()
        );
        assert!(
            (strip.input_rms() - 1.0).abs() < 1.0e-6,
            "输入 RMS {} 必须取较响的 L 声道（1.0）",
            strip.input_rms()
        );
        assert!(
            (strip.output_peak() - 1.0).abs() < 1.0e-6,
            "输出峰值 {} 必须取较响的 L 声道（1.0）",
            strip.output_peak()
        );
        assert!(
            (strip.output_rms() - 1.0).abs() < 1.0e-6,
            "输出 RMS {} 必须取较响的 L 声道（1.0）",
            strip.output_rms()
        );
    }

    /// **判据（新写，可红）**：两条声道必须各自处理**自己**的输入。
    ///
    /// 量什么：一段只在**左**路有脉冲的输入经 `process_stereo` 之后的
    /// 两条输出切片（`f32` 位型，帧数）。
    ///
    /// 为什么需要它（机械读数）：链上唯一的跨声道耦合是**共享增益**
    ///（压缩器的 `gain`，两侧同乘）；滤波级与 EQ 级都是**每声道一份状态**
    ///（`filter_l`／`filter_r`、`eq` 的两组双二阶）。既有判据的夹具几乎都用
    /// **左右相同**的输入（`mono_path_matches_a_duplicated_stereo_pair`、
    /// `the_same_input_twice_is_bit_identical`），因此"滤波级的输入被互换"
    /// 这类改动**不可观测**。把
    /// `(self.filter_l.process(left), self.filter_r.process(right))`
    /// 改成 `(self.filter_l.process(right), self.filter_r.process(left))` 时，
    /// 全库 457 条判据**全绿**（第四批注入表的 C04）。
    ///
    /// 注入实测：滤波级左右输入互换 ⇒ 本判据变红（左路恒为 0）。
    #[test]
    fn the_two_channels_keep_their_own_input() {
        /// 观测帧数：够长，让滤波级的瞬态完全走出。
        const FRAMES: usize = 512;
        let mut strip = ChannelStrip::new(ChannelStripParams::DEFAULT, SR);
        let mut left = vec![0.0f32; FRAMES];
        let mut right = vec![0.0f32; FRAMES];
        left[0] = 1.0;
        strip.process_stereo(&mut left, &mut right);
        assert!(
            left.iter().any(|sample| sample.abs() > 1e-6),
            "左路脉冲必须真的穿过链路（否则本判据测的是空壳）"
        );
        for (frame, sample) in right.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                0.0f32.to_bits(),
                "第 {frame} 帧右路不是逐位静音（幅度 {}）⇒ 另一路的样本串过来了",
                sample
            );
        }
    }
}
