//! 卷积混响**外壳**：把卷积核装配成一台可用的混响（预延迟 ＋ 湿干混合 ＋ IR 增益）。
//! [ARCH-RT-001] [ARCH-PDC-001]
//!
//! # 0. 本模块是什么、**不是**什么
//!
//! [`crate::convolution`] 交付的是**单声道卷积核**（均匀分块 UPOLA），
//! [`crate::convolution_stereo`] 交付的是**真立体声四通路装配**。两者都只做
//! "输入卷 IR"这一件事。本模块是它们上面的**器件外壳**，只补三件事：
//!
//! | 本模块补的 | 单位 | 值域 |
//! | :--- | :--- | :--- |
//! | 湿路径**预延迟** | 秒 | `0.0 … 0.1` |
//! | **湿 / 干**两路独立电平 | 线性 | `0.0 … 1.0` |
//! | **IR 增益** | dB | `-60 … +24` |
//!
//! ⛔ 本模块**不是**：
//!
//! 1. **不是** IR 载入器。文件 I/O、采样率换算、首波对齐、长度归一化、淡出都不在
//!    本 crate 的物理边界内（`yeban-dsp` 无 I/O）。本器件只接受已经就绪的 `&[f32]`；
//! 2. **不是**新的卷积算法。分块、频域延迟线、重叠相加全部沿用
//!    [`crate::convolution`]，本模块不碰它们；
//! 3. **不是**非均匀分区。分块大小仍是固定的 [`crate::convolution::CONV_BLOCK_FRAMES`]，
//!    长 IR 的尾部成本没有优化（代价读数见 [`crate::convolution`] 的实测表）；
//! 4. **不是** `reverb`（Freeverb）的替代。那个是**合成**尾巴（梳状 ＋ 全通），
//!    本模块是**测量**尾巴（把一条真实脉冲响应原样卷进来）。两者并存，由调用方选。
//!
//! # 1. 依据（先量后做：规范**没有**这一条）
//!
//! **实测的规范状态**：规范正文里**没有**任何关于卷积或混响的原文。量法
//! `grep -rniE 'reverb|卷积|混响|impulse' docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
//! docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ⇒ **0 命中**；
//! 规范里 `yeban-dsp` 那一行的器件清单只有"真峰值限制器, SSL 压缩, 通道条, 808/909"。
//! ⇒ 本模块的依据是以下**三条已提交的登记**，不是规范要求：
//!
//! | 出处 | 原文（照抄） |
//! | :--- | :--- |
//! | `docs/ledger/legacy-reuse-audit.md` 的复用来源表 | `dsp/convolution.rs` … **移植后可用**（分区大小是按 wasm arena 调的）⇒ 落到 `yeban-dsp` |
//! | [`crate::convolution`] 的 §7 第 4 条 | **IR 的增益/湿干混合/预延迟**：本器件只做卷积，不碰干信号。混合与预延迟由调用方（或 `reverb` 那类外壳）承担 |
//! | `docs/ledger/session-handoff-notes.md` 的未做项第 3 条 | **卷积混响只是起步**（`convolution.rs` ＋ 一条零分配判据）；"对标 ReaVerb"远未完成 |
//!
//! 本条即"由调用方（或 `reverb` 那类外壳）承担"里那个**外壳**。
//!
//! ⚠ **不做外部主张**：本模块**不声称**与 ReaVerb 的旋钮一一对应，也不声称
//! "对标完成"。ReaVerb 的控件清单与通道顺序在本机取不回（本机网络已断），因此
//! 这里只实现上面那张表里点名的三件事。
//!
//! # 2. 信号链（逐样本一次，顺序固定）
//!
//! ```text
//!        ┌──────────────── 干路 ────────────────┐
//! x ──┬──┤ x · dry                             ├── + ── out
//!     │  └──────────────────────────────────────┘
//!     └─→ 预延迟(N 帧) ─→ 四通路卷积 ─→ · ir_gain ─→ · wet
//! ```
//!
//! `out = x · dry + conv(predelay(x)) · ir_gain · wet`。两条路**各自独立**：
//! `dry = 1, wet = 0` 是逐位直通；`dry = 0, wet = 0` 是**精确静音**（不是直通）。
//!
//! ⚠ "逐位直通"有一个例外，**实测**（本机 aarch64，本票读数）：全干时湿项是 `±0.0`，
//! 而 IEEE-754 规定 `-0.0 + 0.0 = +0.0` ⇒ 输入里的 `-0.0` 会变成 `+0.0`
//! （实得比特 `0x8000_0000` → `0x0000_0000`；湿项为 `-0.0` 时则保持 `-0.0`，
//! 因此这条还依赖该样本处的湿信号符号）。其余每一个比特都与输入相同。
//! 这是加法单位的固有行为，不是混合公式的缺陷：改成"全干就短路"能让它逐位成立，
//! 但那样湿路的历史就不再推进，与本节"没有短路"的设计正面冲突。
//! ⚠ 本例外**没有**单独的判据：`wet = 0` 时混合式对除干项以外的任何改动都不敏感，
//! 写一条只钉住这个 IEEE 行为的表征测试没有判别力。它在此登记，不假装被覆盖。
//!
//! ⚠ **没有**"`wet == 0` 就跳过卷积"的短路：短路会让 `dry = 0, wet = 0` 输出原信号
//! 而不是静音，那不是本模块的语义。热路径的成本因此与 `wet` 无关；调用方若要
//! 真正旁通，用 [`ConvolutionReverb::is_active`] 在**器件之外**跳过。
//!
//! ## 2.1 预延迟是**湿路**上的效果，不是 PDC 延迟
//!
//! 预延迟把**送进卷积的那份**输入推迟 N 帧；干路不动。因此本器件的
//! [`ConvolutionReverb::latency_samples`] 仍是卷积核的 `0`，预延迟**不是**
//! `ARCH-PDC-001` 的补偿量、不进 `RenderPlan` 的延迟路径。口径与
//! [`crate::reverb`] 的 `predelay` 相同（那里也只延迟湿路）。
//!
//! `pre_delay_s = 0` 时**不**走延迟线（`pre_len = 0`），湿输入就是当前样本，
//! 逐位不受影响。
//!
//! ## 2.2 预延迟**长度变更**时清零（本轮的缺口修复）
//!
//! "秒 → 帧"的读数（`pre_len`）**变了**的时候，[`ConvolutionReverb::set_params`] /
//! [`ConvolutionReverb::set_sample_rate`] 会把两条预延迟线清零并把写头归零。
//!
//! 为什么必须清：线按 [`MAX_PRE_DELAY_FRAMES`] 预分配、永不缩小，因此**长度变长**
//! （例如 `1 ms → 100 ms`）时，新读头会指向一段在**旧**、更短的延迟下写进缓冲、
//! 但从未被读出的旧音频。那段旧激励成为"幽灵回声"出现在湿路里 —— 它既不是新延迟
//! 该给的内容，也依赖上一次的处理历史，因此不是确定性的可复现输出。判据
//! `convolution_reverb::tests::changing_the_pre_delay_length_never_replays_stale_audio`
//! 用一个相反的要求钉住它：清空延迟线所需的全部可见激励恰好被随后的静音冲掉
//! ⇒ 湿输出只能留下卷积核的 FFT 舍入底（本机实测峰值约 `1.2e-6`）。
//!
//! ⚠ 代价与边界（不隐藏）：本器件**没有**做交叉淡化，也**没有**做分数延迟插值。
//! 长度只按整帧变化；持续拖动 `pre_delay_s` 时，每当读数跨过一帧就清一次线
//! （`2 × MAX_PRE_DELAY_FRAMES` 次原地写，**零分配**，由
//! `tests/convolution_reverb_rt_zero_alloc.rs` 判据 1 的"每 250 个量子换一次
//! `pre_delay_s`"覆盖）。要平滑拖动，调用方应在器件之外做参数平滑。
//!
//! # 3. 分配纪律 [ARCH-RT-001]
//!
//! 两个分配入口，都必须在音频回调**之外**调用：
//!
//! | 入口 | 分配什么 |
//! | :--- | :--- |
//! | [`ConvolutionReverb::set_sample_rate`] | 两条 `MAX_PRE_DELAY_FRAMES` 帧的预延迟线（各一次） |
//! | [`ConvolutionReverb::set_impulse_response`] | 转发给四条 [`crate::convolution::Convolution`]（IR 频谱与频域延迟线） |
//!
//! [`ConvolutionReverb::process`] / [`ConvolutionReverb::reset`] /
//! [`ConvolutionReverb::set_params`] **逐样本零分配**：
//!
//! - 预延迟线按**上限**预分配，`set_params` 只改一个 `usize` 长度 ⇒ 拖预延迟旋钮
//!   在音频线程上**不分配**（这一条由运行期判据钉住）；
//! - 卷积的湿输入 scratch 是结构体里的**内联数组** `[f32; 2 · 128]`，不经堆；
//! - 没有 `push` / `Box` / `format!` / `collect`，没有锁，没有 I/O，没有日志。
//!
//! # 4. 输入的块长（**与前两层核不同的地方，明说**）
//!
//! [`crate::convolution::Convolution`] 与 [`crate::convolution_stereo::TrueStereoConvolution`]
//! 一次只吃**至多 [`crate::convolution::CONV_BLOCK_FRAMES`] 帧**，超出的尾部原样不碰。
//! 本器件在它们之上把**任意长的交错立体声块**切成 128 帧的子块依次处理，因此
//! "块长 300 帧"不会静默丢掉后 172 帧。切分口径由两条判据钉住：一次 256 帧的调用
//! 与两次 128 帧的调用逐位相同；一次 300 帧的调用与 128 → 128 → 44 的调用逐位相同。
//!
//! ⚠ **诚实边界**：卷积核的子块网格锚在**每次调用**的起点，不是绝对样本时间
//! （短子块按零补齐推进历史，这是 [`crate::convolution`] 的既有语义）。因此调用方
//! 应当用**固定**块长（引擎的量子恒为 128 帧）；块长抖动会让网格相对绝对时间漂移。
//! 本器件不改变这条既有语义，只把它推广到长块。
//!
//! # 5. 数值边界（⛔ 不许 NaN / Inf）
//!
//! [`ConvolutionReverb::set_params`] 对四个参数逐个做"非有限值回落 + 钳制"
//! （与 [`crate::reverb`] 同款纪律）。IR 增益经 [`crate::math::db_to_gain`] 折算，
//! 该函数的实参被钳到 `≥ -60 dB`，因此增益恒为正常数、不会是 `0` 或非有限值。
//! 非有限**输入样本**仍会产生非有限输出（卷积核的既有口径：逐样本净化是调用方的
//! 职责，见 [`crate::convolution`] 的输入取值域一节），本模块不额外承诺。
//!
//! **IR 走的是相反的一条**：它在配置期被**校验**。非有限样本（`NaN`/`±inf`）或
//! 频谱溢出的 IR 会被拒绝，整台回到未配置的直通（`is_configured()` 为假）——
//! 那是唯一能挡住"湿路从此永久非有限"的地方，因为 `reset()` 清不掉已经写坏的
//! IR 频谱。判据：`non_finite_impulse_responses_are_rejected_by_the_shell` 与
//! `a_rejected_impulse_response_cannot_poison_the_wet_path`。

use crate::convolution::{CONV_BLOCK_FRAMES, CONV_LATENCY, CONV_MAX_IR_FRAMES};
use crate::convolution_stereo::TrueStereoConvolution;
use crate::math::{db_to_gain, sanitise_sample_rate};

/// 预延迟的上限（秒）。`0.1 s` = `100 ms`。
///
/// 与 [`crate::reverb`] 的预延迟上限同值。上界取 100 ms 的理由是音乐上的：
/// 再长的"预延迟"就是一段可听见的间隔，应当用两条 IR 表达，而不是一个旋钮。
pub const MAX_PRE_DELAY_SECONDS: f32 = 0.1;

/// 预延迟线的帧数上限。`9_600` = `100 ms @ 96 kHz`，与 [`crate::reverb`] 的
/// 预延迟缓冲上限同值。
///
/// 这是**按上限预分配**的长度：任何采样率下的合法预延迟都不会超过它，因此调参
/// 永远不触发分配。
pub const MAX_PRE_DELAY_FRAMES: usize = 9_600;

/// IR 增益的下限（dB）。`-60 dB` 在听感上已是静音，但仍是正常数（不是 0）。
pub const MIN_IR_GAIN_DB: f32 = -60.0;

/// IR 增益的上限（dB）。`+24 dB` 足够把一条录得很轻的 IR 抬起来。
pub const MAX_IR_GAIN_DB: f32 = 24.0;

/// "湿路可闻"的门限（线性）。小于它时 [`ConvolutionReverb::is_active`] 为假。
///
/// 取值与 [`crate::reverb`] 的 `mix > 1e-4` 同口径（`-80 dB`）。
const WET_EPSILON: f32 = 1e-4;

/// 从未调用 [`ConvolutionReverb::set_sample_rate`] 时用的采样率（Hz）。
///
/// 它只影响预延迟的"秒 → 帧"换算；卷积核本身对采样率无感（IR 就是按帧给的）。
const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

/// 卷积混响的三个电平/时间参数。
///
/// 四个字段都是**线性/秒**的器件参数，不是硬件旋钮标度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConvolutionReverbParams {
    /// 湿路径的预延迟（**秒**），钳到 `0.0 … MAX_PRE_DELAY_SECONDS`。
    pub pre_delay_s: f32,
    /// 干路径电平（**线性**），钳到 `0.0 … 1.0`。
    pub dry: f32,
    /// 湿路径电平（**线性**），钳到 `0.0 … 1.0`。
    pub wet: f32,
    /// 脉冲响应的增益（**dB**），钳到 `MIN_IR_GAIN_DB … MAX_IR_GAIN_DB`。
    pub ir_gain_db: f32,
}

impl Default for ConvolutionReverbParams {
    /// 缺省：无预延迟、干路单位增益、湿路 `0.25`、IR 增益 `0 dB`。
    ///
    /// 湿路默认 `0.25` 与 [`crate::reverb`] 的 `mix = 0.25` 同口径：把一条
    /// 未归一的 IR 直接以 `wet = 1.0` 混进来通常过响。
    fn default() -> Self {
        Self {
            pre_delay_s: 0.0,
            dry: 1.0,
            wet: 0.25,
            ir_gain_db: 0.0,
        }
    }
}

/// 非有限值回落 + 钳制。返回的参数保证四个字段都是有限值且在值域内。
fn sanitise(params: ConvolutionReverbParams) -> ConvolutionReverbParams {
    let fallback = ConvolutionReverbParams::default();
    let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
    ConvolutionReverbParams {
        pre_delay_s: finite(params.pre_delay_s, fallback.pre_delay_s)
            .clamp(0.0, MAX_PRE_DELAY_SECONDS),
        dry: finite(params.dry, fallback.dry).clamp(0.0, 1.0),
        wet: finite(params.wet, fallback.wet).clamp(0.0, 1.0),
        ir_gain_db: finite(params.ir_gain_db, fallback.ir_gain_db)
            .clamp(MIN_IR_GAIN_DB, MAX_IR_GAIN_DB),
    }
}

/// "秒"换算成预延迟线的帧数（单位：帧），并按上限钳制。
///
/// 非有限输入 ⇒ `0`。负数 ⇒ `0`（钳在 `clamp` 里，`as usize` 之前）。
fn frames_for_pre_delay(seconds: f32, sample_rate: f32) -> usize {
    if !seconds.is_finite() {
        return 0;
    }
    let frames = (seconds.clamp(0.0, MAX_PRE_DELAY_SECONDS) * sample_rate) as usize;
    frames.min(MAX_PRE_DELAY_FRAMES)
}

/// 真立体声卷积混响：四条通路卷积 ＋ 预延迟 ＋ 湿干混合 ＋ IR 增益。
///
/// 通路布局完全沿用 [`crate::convolution_stereo::TrueStereoConvolution`]
/// （`h_LL` / `h_LR` / `h_RL` / `h_RR`），本类型只在其**外面**加混合与延迟。
pub struct ConvolutionReverb {
    /// 四条卷积核（真立体声装配）。
    kernels: TrueStereoConvolution,
    /// 两条声道的预延迟线：`pre[0]` = `L`、`pre[1]` = `R`。
    ///
    /// 长度恒为 [`MAX_PRE_DELAY_FRAMES`]（在 [`Self::set_sample_rate`] 或
    /// [`Self::set_impulse_response`] 里**一次性**分配）。实际延迟是 `pre_len` 帧。
    /// 长度变更时的清零见 [`Self::retune_pre_delay`]。
    pre: [Vec<f32>; 2],
    /// 实际预延迟（帧）。`0` = 不延迟（此时不读延迟线）。
    pre_len: usize,
    /// 预延迟线的写/读头。
    pre_index: usize,
    /// 采样率（Hz），只用于"秒 → 帧"换算。
    sample_rate: f32,
    /// 当前参数（已净化）。
    params: ConvolutionReverbParams,
    /// `params.ir_gain_db` 折算出的线性增益。
    ir_gain: f32,
    /// 卷积的湿输入 scratch（交错立体声，`2 · CONV_BLOCK_FRAMES` 个 `f32`）。
    ///
    /// 内联数组 ⇒ 不经堆，构造期就随结构体成型。
    wet_scratch: [f32; 2 * CONV_BLOCK_FRAMES],
}

impl ConvolutionReverb {
    /// 构造一个**未配置**的实例（直通）。不分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kernels: TrueStereoConvolution::new(),
            pre: [Vec::new(), Vec::new()],
            pre_len: 0,
            pre_index: 0,
            sample_rate: DEFAULT_SAMPLE_RATE,
            params: ConvolutionReverbParams {
                pre_delay_s: 0.0,
                dry: 1.0,
                wet: 0.25,
                ir_gain_db: 0.0,
            },
            ir_gain: 1.0,
            wet_scratch: [0.0; 2 * CONV_BLOCK_FRAMES],
        }
    }

    /// 设定采样率（Hz）。**这是分配入口之一**（建两条预延迟线），必须在音频回调
    /// 之外调用。退化输入（`NaN` / `±inf` / `< 1000`）按 `math::sanitise_sample_rate`
    /// 钳到 [`crate::MIN_SAMPLE_RATE`]。
    ///
    /// 采样率只影响预延迟的"秒 → 帧"换算；它**不**重采样 IR（IR 是按帧给的）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitise_sample_rate(sample_rate);
        self.retune_pre_delay();
    }

    /// 设定四条脉冲响应：`h_ll` / `h_lr` / `h_rl` / `h_rr`。
    ///
    /// 语义完全沿用 [`crate::convolution_stereo::TrueStereoConvolution::set_impulse_response`]：
    /// 四条必须**等长且非空**，且每一条都必须通过卷积核的取值校验（非有限样本 /
    /// 频谱溢出 ⇒ 拒绝）；任一条件不满足就整台拒绝，返回 `0` 并置回未配置的
    /// **直通**状态。本器件**不做** IR 的净化与修补：拒绝是唯一的出路。
    ///
    /// 返回**真正接受**的帧数。**这是分配入口之一**，必须在音频回调之外调用。
    pub fn set_impulse_response(
        &mut self,
        h_ll: &[f32],
        h_lr: &[f32],
        h_rl: &[f32],
        h_rr: &[f32],
    ) -> usize {
        self.retune_pre_delay();
        self.kernels.set_impulse_response(h_ll, h_lr, h_rl, h_rr)
    }

    /// 设定普通立体声的两条脉冲响应：`h_l`（`L → L`）与 `h_r`（`R → R`）。
    ///
    /// 两条**交叉**通路被显式设成**同长度的零 IR**（`H = 0` ⇒ 湿输出恒 `0`），
    /// 而不是留空 —— 留空在这套核里是**直通**，会把对侧信号原样漏过来。
    ///
    /// 两条不等长，或任一条未通过取值校验（非有限样本 / 频谱溢出，见
    /// [`Self::set_impulse_response`]）时，返回 `0` 并把整台置回未配置直通。
    ///
    /// ⚠ 代价：四条核仍然各自运行（两条对角有效、两条乘的是一片零谱），因此
    /// 普通立体声的代价与真立体声**同阶**。这是"不复制第二份 2×2 路由"的代价。
    ///
    /// 临时零切片在**本方法内**分配一次（属允许的构造期分配），长度**钳到接受上限**
    /// [`CONV_MAX_IR_FRAMES`]（理由见下）。返回值同
    /// [`Self::set_impulse_response`]。
    ///
    /// ⚠ **为什么临时切片也要钳制**：[`Self::set_impulse_response`] 只接受前
    /// [`CONV_MAX_IR_FRAMES`] 帧，而那个上限的文档承诺是"`载入一个误选的超长文件`
    /// 有一个**有界的、可预测的**后果"。按**未钳制**的输入长度开零切片会让那半句
    /// 不成立：超出的部分一个字节都不参与卷积，却照样占内存。本机实测（计数分配器，
    /// 单位：一次调用的峰值存活字节数）：输入 1× 上限 = 32 894 336 B，
    /// 4× 上限 = 38 654 336 B（多 5 760 000 B = 3 × 480 000 帧 × 4 B），
    /// 16× 上限 = 61 694 336 B。钳制之后三个读数**逐字节相同**，而接受的帧数与
    /// 产出的音频一个比特都不变。
    pub fn set_stereo_impulse_response(&mut self, h_l: &[f32], h_r: &[f32]) -> usize {
        if h_l.len() != h_r.len() {
            return self.set_impulse_response(&[], &[], &[], &[]);
        }
        let frames = h_l.len().min(CONV_MAX_IR_FRAMES);
        let silence = vec![0.0f32; frames];
        self.set_impulse_response(&h_l[..frames], &silence, &silence, &h_r[..frames])
    }

    /// 设定一条单声道脉冲响应，喂给**两个输出声道**（`h_LL = h_RR = ir`，两条交叉
    /// 通路设为同长度的零 IR）。
    ///
    /// 注意这不是"复制成两条独立 IR"：对侧不泄漏，因此单声道 IR 得到的是**居中**
    /// 的湿信号，而不是一段假立体声。
    ///
    /// `ir` 未通过取值校验（非有限样本 / 频谱溢出）时返回 `0` 并置回未配置直通。
    ///
    /// 临时零切片同样**钳到接受上限** [`CONV_MAX_IR_FRAMES`]，理由与
    /// [`Self::set_stereo_impulse_response`] 的文档相同（那里有实测读数）。
    pub fn set_mono_impulse_response(&mut self, ir: &[f32]) -> usize {
        let frames = ir.len().min(CONV_MAX_IR_FRAMES);
        let silence = vec![0.0f32; frames];
        self.set_impulse_response(&ir[..frames], &silence, &silence, &ir[..frames])
    }

    /// 设定参数（**零分配**，可以逐块调用）。
    ///
    /// 退化输入（`NaN` / `±inf`）会被替换成缺省值再钳制（见 `sanitise`）。
    ///
    /// ⚠ `pre_delay_s` 折算出的帧数**变了**时，两条预延迟线会被清零、写头归零
    /// （见 [`Self::retune_pre_delay`] 与模块文档 §2.2）。这条路径逐样本零分配。
    pub fn set_params(&mut self, params: ConvolutionReverbParams) {
        self.params = sanitise(params);
        self.ir_gain = db_to_gain(self.params.ir_gain_db);
        self.retune_pre_delay();
    }

    /// 当前参数（已净化）。
    #[must_use]
    pub const fn params(&self) -> ConvolutionReverbParams {
        self.params
    }

    /// `ir_gain_db` 折算出的线性增益。
    #[must_use]
    pub const fn ir_gain(&self) -> f32 {
        self.ir_gain
    }

    /// 是否已配置：四条 IR 曾按等长非空被接受，且预延迟线已建好。
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.kernels.is_configured() && !self.pre[0].is_empty()
    }

    /// 接受的 IR 帧数（四条相同；`0` = 未配置）。
    #[must_use]
    pub const fn ir_frames(&self) -> usize {
        self.kernels.ir_frames()
    }

    /// 当前实际预延迟（帧）。单位：帧。
    #[must_use]
    pub const fn pre_delay_frames(&self) -> usize {
        self.pre_len
    }

    /// 本器件引入的**处理延迟**（帧），恒为 [`CONV_LATENCY`] = `0` [ARCH-PDC-001]。
    ///
    /// ⚠ 预延迟**不是**这个数的一部分：它只推迟湿路，干路即时，见模块文档 §2.1。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        CONV_LATENCY
    }

    /// 湿信号是否可闻（已配置且 `wet > WET_EPSILON`）。
    ///
    /// 这是给调用方的**旁通建议**，不是内部短路：本器件即使 `wet == 0` 也照样
    /// 计算干湿混合（否则 `dry = 0, wet = 0` 就不是静音了）。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_configured() && self.params.wet > WET_EPSILON
    }

    /// 清空四条卷积核的历史（频域延迟线 ＋ 重叠相加尾）与两条预延迟线。
    ///
    /// 下一个块与"刚配置完"逐位相同。零分配。
    pub fn reset(&mut self) {
        self.kernels.reset();
        for line in &mut self.pre {
            line.fill(0.0);
        }
        self.pre_index = 0;
    }

    /// 原地处理一个**交错立体声**块（`[L, R, L, R, …]`），返回处理的**帧**数。
    ///
    /// 语义：
    ///
    /// 1. 未配置时**直通**（逐位不变）；
    /// 2. 处理的帧数是 `block.len() / 2`；奇数长度的块**最后一帧**（只有 `L`、
    ///    没有 `R`）原样不碰；
    /// 3. 长于 [`CONV_BLOCK_FRAMES`] 帧的块按 128 帧的子块依次处理（见模块文档 §4）；
    /// 4. 全程零分配、零锁、零 I/O [ARCH-RT-001]。
    pub fn process(&mut self, block: &mut [f32]) -> usize {
        let frames = block.len() / 2;
        if frames == 0 {
            return 0;
        }
        if !self.is_configured() {
            return frames;
        }
        let mut done = 0;
        while done < frames {
            let chunk_frames = (frames - done).min(CONV_BLOCK_FRAMES);
            let chunk = &mut block[2 * done..2 * (done + chunk_frames)];
            self.process_chunk(chunk, chunk_frames);
            done += chunk_frames;
        }
        frames
    }

    /// 处理一个**至多** [`CONV_BLOCK_FRAMES`] 帧的子块（`chunk.len() == 2 · frames`）。
    fn process_chunk(&mut self, chunk: &mut [f32], frames: usize) {
        let wet = self.params.wet;
        let dry = self.params.dry;
        let gain = self.ir_gain;
        // `pre_len == 0` 时不读延迟线；模数取 1 只是为了让写头停在第 0 格。
        let modulus = if self.pre_len == 0 { 1 } else { self.pre_len };

        // 段 1：预延迟。先读后写 ⇒ 读到的值是 `pre_len` 个样本之前写进去的。
        for i in 0..frames {
            let left = chunk[2 * i];
            let right = chunk[2 * i + 1];
            let (wet_left, wet_right) = if self.pre_len == 0 {
                (left, right)
            } else {
                (self.pre[0][self.pre_index], self.pre[1][self.pre_index])
            };
            self.pre[0][self.pre_index] = left;
            self.pre[1][self.pre_index] = right;
            self.pre_index += 1;
            if self.pre_index >= modulus {
                self.pre_index = 0;
            }
            self.wet_scratch[2 * i] = wet_left;
            self.wet_scratch[2 * i + 1] = wet_right;
        }

        // 段 2：四通路卷积（原地写回 scratch）。
        self.kernels.process(&mut self.wet_scratch[..2 * frames]);

        // 段 3：干湿混合。两条路各自独立 ⇒ `dry = 1, wet = 0` 是逐位直通。
        for i in 0..frames {
            let left = chunk[2 * i];
            let right = chunk[2 * i + 1];
            chunk[2 * i] = left * dry + self.wet_scratch[2 * i] * gain * wet;
            chunk[2 * i + 1] = right * dry + self.wet_scratch[2 * i + 1] * gain * wet;
        }
    }

    /// 按当前参数保证两条预延迟线已就绪，并处理**长度变更**。
    ///
    /// 两个动作，顺序固定：
    ///
    /// 1. 线的长度**恒为** [`MAX_PRE_DELAY_FRAMES`]（按上限一次性分配，此后永不重新
    ///    分配 —— 调参因此逐样本零分配 [ARCH-RT-001]）；
    /// 2. 重算 `pre_len`；**长度真的变了**就把两条线清零、写头归零。
    ///
    /// 第 2 步的清零不是装饰。不清的话，长度**变长**时读头会指向上一次（更短）
    /// 延迟留下的旧音频：新预延迟的前若干帧会读出一段"幽灵回声"（旧激励在
    /// `pre_len` 帧之前并不存在，却出现在湿路里），而且它依赖上一次的处理历史，
    /// 因此不是确定性的可复现输出。清零让"长度变更"有一个干净的、定义明确的后果
    /// （与 [`Self::reset`] 同一条纪律）。
    ///
    /// ⚠ 代价（诚实边界）：本函数**不**在长度变更时对旧/新延迟做交叉淡化，也不做
    /// 分数延迟插值 —— 那需要第二条读头与混合状态（另一张票）。因此持续拖动
    /// [`ConvolutionReverbParams::pre_delay_s`] 时，每当"秒 → 帧"的读数跨过一帧就会
    /// 清一次线（`2 × MAX_PRE_DELAY_FRAMES` 次原地写，零分配）。接线的调用方若要
    /// 平滑拖动，应当在**器件之外**做参数平滑，或接受这段湿路的硬切换。
    fn retune_pre_delay(&mut self) {
        if self.pre[0].len() != MAX_PRE_DELAY_FRAMES {
            self.pre = [
                vec![0.0; MAX_PRE_DELAY_FRAMES],
                vec![0.0; MAX_PRE_DELAY_FRAMES],
            ];
            self.pre_index = 0;
        }
        let wanted = frames_for_pre_delay(self.params.pre_delay_s, self.sample_rate);
        if wanted != self.pre_len {
            for line in &mut self.pre {
                line.fill(0.0);
            }
            self.pre_index = 0;
            self.pre_len = wanted;
        }
    }
}

impl Default for ConvolutionReverb {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for ConvolutionReverb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 手写而不是 derive：两条预延迟线各有上限帧数个元素，derive 会把它们全打印出来。
        f.debug_struct("ConvolutionReverb")
            .field("configured", &self.is_configured())
            .field("ir_frames", &self.ir_frames())
            .field("pre_delay_frames", &self.pre_len)
            .field("sample_rate", &self.sample_rate)
            .field("params", &self.params)
            .field("ir_gain", &self.ir_gain)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据用的采样率（Hz）。取 1024 的倍数是为了让"秒 ↔ 帧"的换算**精确**：
    /// `N / 1024` 在 f32 里可精确表示，乘回 1024 恰得整数 `N`。
    const SR: f32 = 4_096.0;

    /// 单抽头 IR：`frames` 帧，第 `tap` 帧为 `value`，其余 0。
    fn impulse_ir(frames: usize, tap: usize, value: f32) -> Vec<f32> {
        let mut ir = vec![0.0f32; frames];
        if tap < frames {
            ir[tap] = value;
        }
        ir
    }

    /// 衰减 IR（非平凡，用于混合/增益判据）。
    fn decaying_ir(frames: usize, seed: f32) -> Vec<f32> {
        (0..frames)
            .map(|i| (i as f32 * 0.11 + seed).sin() * 0.4 / (1.0 + i as f32 * 0.05))
            .collect()
    }

    /// 构造一台已配置的外壳：单声道 IR = 给定 IR，四条通路都有效。
    fn shell_with_ir(ir: &[f32]) -> ConvolutionReverb {
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        assert_eq!(shell.set_mono_impulse_response(ir), ir.len());
        shell
    }

    /// 确定性激励（不用分配，也不用 RNG 对象）。
    fn fill(block: &mut [f32], quantum: u64, scale: f32) {
        for (i, sample) in block.iter_mut().enumerate() {
            *sample = scale * (quantum as f32 * 0.37 + i as f32 * 0.011).sin();
        }
    }

    /// **判据（新写，可红）**：预延迟的**文档上界**（`0.1 s`）与帧数上限
    /// （`MAX_PRE_DELAY_FRAMES` = `9 600`）被钉住。
    ///
    /// 量什么：`frames_for_pre_delay(seconds, sample_rate)` 的返回值（单位：帧）。
    ///
    /// 为什么需要它：实测（本票注入 C07）把 `seconds.clamp(0.0, MAX_PRE_DELAY_SECONDS)`
    /// 里的上界放宽成 `MAX_PRE_DELAY_SECONDS * 2.0`（即 `0.2 s`）之后，全量 417 条
    /// 判据仍全绿 —— 既有判据只喂 `0.1 s` 及以下的预延迟，从不喂越界值。
    ///
    /// 链路只有比较、钳位、乘 `f32` 再转 `usize` ⇒ IEEE 精确类 ⇒ 处处硬断言。
    #[test]
    fn the_pre_delay_upper_bound_is_pinned_by_a_literal() {
        assert_eq!(
            MAX_PRE_DELAY_SECONDS.to_bits(),
            0.1f32.to_bits(),
            "上界常数漂移"
        );
        assert_eq!(MAX_PRE_DELAY_FRAMES, 9_600, "帧数上限漂移");
        // 界内：按秒换算（`4 800 Hz` × `0.1 s` = `480` 帧）。
        assert_eq!(frames_for_pre_delay(0.0, 4_800.0), 0);
        assert_eq!(frames_for_pre_delay(0.05, 4_800.0), 240);
        assert_eq!(frames_for_pre_delay(0.1, 4_800.0), 480);
        // 越界：必须**折到 0.1 秒**，不是各自生效。
        for seconds in [0.1 + f32::EPSILON, 0.2, 1.0, 1.0e6] {
            assert_eq!(
                frames_for_pre_delay(seconds, 4_800.0),
                frames_for_pre_delay(0.1, 4_800.0),
                "{seconds} s 的预延迟请求必须被折到 0.1 s 上"
            );
        }
        // 负值与非有限值 ⇒ 0（不是回绕成一个巨大的 `usize`）。
        assert_eq!(frames_for_pre_delay(-1.0, 4_800.0), 0);
        assert_eq!(frames_for_pre_delay(f32::NAN, 4_800.0), 0);
        assert_eq!(frames_for_pre_delay(f32::INFINITY, 4_800.0), 0);
        // 帧数上限：`96 kHz` 下 `0.1 s` 恰好是 `9 600` 帧，再大的采样率被帧数上限压住。
        assert_eq!(frames_for_pre_delay(0.1, 96_000.0), MAX_PRE_DELAY_FRAMES);
        assert_eq!(
            frames_for_pre_delay(0.1, 192_000.0),
            MAX_PRE_DELAY_FRAMES,
            "帧数上限必须同时生效"
        );
        // 非空证明：界内读数与越界读数必须真的不同。
        assert_ne!(
            frames_for_pre_delay(0.05, 4_800.0),
            frames_for_pre_delay(0.1, 4_800.0)
        );
    }

    fn interleaved(frames: usize, quantum: u64, scale: f32) -> Vec<f32> {
        let mut block = vec![0.0f32; 2 * frames];
        fill(&mut block, quantum, scale);
        block
    }

    // -----------------------------------------------------------------------
    // 判据 1 … 3：直通 / 静音 / 恒等
    // -----------------------------------------------------------------------

    /// 量什么：**未配置**时处理一个块，输出的每一个比特。
    ///
    /// 判据：逐位不变（`to_bits` 相等）。未配置的直通是三层核一致的约定。
    #[test]
    fn unconfigured_is_bit_exact_passthrough() {
        let mut shell = ConvolutionReverb::new();
        let mut block = interleaved(128, 3, 0.8);
        let expected = block.clone();
        assert_eq!(shell.process(&mut block), 128);
        for (out, want) in block.iter().zip(&expected) {
            assert_eq!(out.to_bits(), want.to_bits(), "未配置时输出必须逐位不变");
        }
    }

    /// 量什么：`dry = 1, wet = 0`（已配置、IR 非平凡）时输出的每一个比特。
    ///
    /// 判据：逐位等于输入。`out = x · 1 + conv · g · 0`，第二项是 `0`，故逐位相同。
    /// 注入：把 `dry` 与 `wet` 对调（或漏乘 `dry`）⇒ 本判据变红。
    #[test]
    fn full_dry_is_bit_exact_passthrough() {
        let ir = decaying_ir(300, 0.5);
        let mut shell = shell_with_ir(&ir);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.003,
            dry: 1.0,
            wet: 0.0,
            ir_gain_db: 0.0,
        });
        let mut block = interleaved(128, 5, 0.7);
        let expected = block.clone();
        shell.process(&mut block);
        for (i, (out, want)) in block.iter().zip(&expected).enumerate() {
            assert_eq!(out.to_bits(), want.to_bits(), "样本 {i}: 全干必须逐位直通");
        }
    }

    /// 量什么：`dry = 0, wet = 0`（已配置）时输出的每一个比特。
    ///
    /// 判据：全部**恰好**是 `0.0`。这条把"`wet == 0` 就短路成直通"这种写法钉死为
    /// **错**：那样 `dry = 0` 会被忽略，输出是原信号而不是静音。
    #[test]
    fn zero_dry_and_zero_wet_is_exact_silence() {
        let ir = decaying_ir(257, 1.5);
        let mut shell = shell_with_ir(&ir);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.01,
            dry: 0.0,
            wet: 0.0,
            ir_gain_db: 0.0,
        });
        let mut block = interleaved(128, 11, 0.9);
        shell.process(&mut block);
        for (i, out) in block.iter().enumerate() {
            assert_eq!(*out, 0.0, "样本 {i}: 干湿都是 0 必须是精确静音");
        }
    }

    // -----------------------------------------------------------------------
    // 判据 4 … 5：预延迟
    // -----------------------------------------------------------------------

    /// 量什么：预延迟 N 帧时，`L` 声道单位脉冲在输出里出现的**样本位置**。
    ///
    /// 判据：`0 … N-1` 帧**恰好**是 `0.0`，第 `N` 帧是 `1.0`（容差 `1e-4`，覆盖
    /// 一次 FFT 往返的舍入）。IR = 单位脉冲 ⇒ 卷积本身是恒等，任何偏移都只能来自
    /// 预延迟。注入：把写头先加后读（差一格）⇒ 第 `N-1` 帧变红。
    #[test]
    fn pre_delay_shifts_the_wet_impulse_by_exactly_n_frames() {
        const N: usize = 37;
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        let delta = impulse_ir(1, 0, 1.0);
        let silence = impulse_ir(1, 0, 0.0);
        assert_eq!(
            shell.set_impulse_response(&delta, &silence, &silence, &delta),
            1
        );
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: N as f32 / SR,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        assert_eq!(shell.pre_delay_frames(), N, "秒 → 帧换算必须是精确的 N");

        let frames = 256;
        let mut block = vec![0.0f32; 2 * frames];
        block[0] = 1.0; // L 的单位脉冲
        shell.process(&mut block);

        // ⚠ 预延迟"未到"的判据是**噪声底**判据，不是逐位 0：单位脉冲要过一次
        // 256 点 FFT 往返（实测噪声底约 `1.6e-9`）。`1e-6` 比底高三个数量级，
        // 又比脉冲低六个数量级 ⇒ 任何真的偏移都会被抓住。
        for i in 0..N {
            assert!(
                block[2 * i].abs() < 1e-6,
                "第 {i} 帧不该有能量（预延迟未到），实测 {}",
                block[2 * i]
            );
        }
        assert!(
            (block[2 * N] - 1.0).abs() < 1e-4,
            "第 {N} 帧应当是卷积出来的单位脉冲，实测 {}",
            block[2 * N]
        );
        for i in (N + 1)..frames {
            assert!(
                block[2 * i].abs() < 1e-6,
                "第 {i} 帧不该有能量，实测 {}",
                block[2 * i]
            );
        }
        // R 声道既没有输入、两条交叉通路又是**零 IR** ⇒ 必须是**恰好** 0。
        for i in 0..frames {
            assert_eq!(block[2 * i + 1], 0.0, "R 声道没有输入，第 {i} 帧必须是 0");
        }
    }

    /// 量什么：跨块的预延迟连续性。把一个 512 帧的脉冲序列按 128 帧分四次喂进去。
    ///
    /// 判据：无论块边界落在哪里，第 100 帧的单位脉冲都在输出第 `100 + N` 帧出现。
    /// 注入：预延迟写头在块边界被重置 ⇒ 本判据变红。
    #[test]
    fn pre_delay_survives_block_boundaries() {
        const N: usize = 50;
        const PULSE: usize = 100;
        // 两条独立的实例，参数完全相同；`process` 是有状态的，同一台不能跑两遍。
        let build = || {
            let mut shell = ConvolutionReverb::new();
            shell.set_sample_rate(SR);
            let delta = impulse_ir(1, 0, 1.0);
            let silence = impulse_ir(1, 0, 0.0);
            shell.set_impulse_response(&delta, &silence, &silence, &delta);
            shell.set_params(ConvolutionReverbParams {
                pre_delay_s: N as f32 / SR,
                dry: 0.0,
                wet: 1.0,
                ir_gain_db: 0.0,
            });
            shell
        };

        let frames = 512;
        let mut source = vec![0.0f32; 2 * frames];
        source[2 * PULSE] = 1.0;

        // 一次性处理。
        let mut one_call = source.clone();
        build().process(&mut one_call);

        // 分四次处理（每次 128 帧）。
        let mut blockwise = source.clone();
        let mut shell = build();
        for chunk in blockwise.chunks_mut(2 * CONV_BLOCK_FRAMES) {
            shell.process(chunk);
        }

        assert!(
            (one_call[2 * (PULSE + N)] - 1.0).abs() < 1e-4,
            "一次调用：脉冲应出现在 {}，实测 {}",
            PULSE + N,
            one_call[2 * (PULSE + N)]
        );
        assert!(
            (blockwise[2 * (PULSE + N)] - 1.0).abs() < 1e-4,
            "分块：脉冲应出现在 {}",
            PULSE + N
        );
        for (i, (a, b)) in one_call.iter().zip(&blockwise).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "样本 {i}: 一次调用与分块调用必须逐位相同"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 判据 5b：预延迟**长度变更**不许重放旧音频
    // -----------------------------------------------------------------------

    /// 量什么：把预延迟从**短**改成**长**之后，一个 300 帧静音块的湿输出
    /// （单位：线性样本值；读数取 300 帧的 `Σ|out|` 与 `peak|out|`）。
    ///
    /// 判据：两个读数都 `≤ STALE_FLOOR = 1e-3`（见下"为什么不是逐位 0"）。
    ///
    /// 观测方式（为什么这样读出的必然是"旧音频"）：延迟线按上限预分配、永不缩小。
    /// 先用 `pre_len = 100` 把 200 帧常数 `7.0` 推进去 —— 写头走过 200 格，而短延迟
    /// 只读得到其中 100 格，因此**至少** 100 格 `7.0` 停在缓冲里没被读出。再把
    /// `pre_len` 改成 300：若线不被清，写头仍停在第 200 格，新延迟要读的格位正落在
    /// 那些旧样本上 ⇒ 湿输出在"新预延迟尚未到齐"的样本上给出 `7.0`。
    ///
    /// 为什么这些旧音频本不该被听见：旧样本写进来的时刻比新读头**早不到** `pre_len`
    /// 帧，而随后的 300 帧静音会覆盖读头走过的全部格位（`300 ≥ 3 × pre_len`）⇒
    /// 长度变更后的前 300 帧湿输出只能是卷积核的舍入底，不能含旧激励。
    ///
    /// 为什么判据不是逐位 `0`：卷积核是频域实现，单位脉冲也要过一次 256 点 FFT
    /// 往返，因此"输入全零"在湿路上留下一个**实测约 `1.2e-6`** 的舍入底（本机
    /// aarch64，本票读数：`peak = 1.1920929e-6`、`Σ|out| = 9.1179485e-5`）。逐位 `0`
    /// 会把这条物理底当成缺陷。`STALE_FLOOR = 1e-3` 比那个底高三个数量级，又比
    /// 旧音频的读数（`peak = 7.0`、`Σ|out| = 1400`）低六个数量级 ⇒ 两侧都不含糊。
    ///
    /// 注入（实测见本票报告）：删掉 [`ConvolutionReverb::retune_pre_delay`] 里
    /// "长度变了就清零那一步"、只保留 `self.pre_len = wanted` ⇒ 本判据变红
    /// （实测 `Σ|out| = 1400`、`peak = 7.0000024`，门限 `1e-3`）。
    #[test]
    fn changing_the_pre_delay_length_never_replays_stale_audio() {
        /// 旧音频的读数（`1400` / `7.0`）与 FFT 舍入底（`9.1e-5` / `1.2e-6`）之间的门限。
        const STALE_FLOOR: f32 = 1e-3;
        const SHORT: usize = 100;
        const LONG: usize = 300;
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        let delta = impulse_ir(1, 0, 1.0);
        let silence = impulse_ir(1, 0, 0.0);
        assert_eq!(
            shell.set_impulse_response(&delta, &silence, &silence, &delta),
            1
        );

        // 短预延迟：把 200 帧常数灌进延迟线（短延迟读不完，旧样本留在缓冲里）。
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: SHORT as f32 / SR,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        assert_eq!(shell.pre_delay_frames(), SHORT, "前置条件：短延迟生效");
        let mut fill = vec![7.0f32; 2 * 200];
        shell.process(&mut fill);

        // 变长：读窗口移进那段旧音频本该在的地方。
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: LONG as f32 / SR,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        assert_eq!(shell.pre_delay_frames(), LONG, "前置条件：长延迟生效");

        let mut probe = vec![0.0f32; 2 * LONG];
        shell.process(&mut probe);
        let energy: f32 = probe.iter().map(|v| v.abs()).sum();
        let peak = probe.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let first_nonzero = probe
            .iter()
            .position(|v| *v != 0.0)
            .map_or("无".to_string(), |i| format!("第 {} 个 f32", i));
        assert!(
            peak <= STALE_FLOOR && energy <= STALE_FLOOR,
            "预延迟变长后湿路重放了旧音频：peak = {peak}、Σ|out| = {energy}、\
             首个非零样本 = {first_nonzero}（门限 {STALE_FLOOR}）"
        );
    }

    // -----------------------------------------------------------------------
    // 判据 6 … 8：混合与 IR 增益
    // -----------------------------------------------------------------------

    /// 量什么：同一个块在三种参数下的输出 —— `(dry, wet)` = `(1, 0)`、`(0, 1)`、
    /// `(0.25, 0.75)`。
    ///
    /// 判据：第三种的每一个样本都等于 `0.25 · 第一种 + 0.75 · 第二种`（容差 `1e-6`，
    /// 覆盖一次乘加舍入）。这条把"权重是独立的"钉死。注入：对调 `dry`/`wet` ⇒ 变红。
    #[test]
    fn dry_and_wet_mix_is_the_stated_linear_combination() {
        let ir = decaying_ir(200, 2.0);
        let mut dry_only = shell_with_ir(&ir);
        let mut wet_only = shell_with_ir(&ir);
        let mut mixed = shell_with_ir(&ir);
        dry_only.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 1.0,
            wet: 0.0,
            ir_gain_db: 0.0,
        });
        wet_only.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        mixed.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.25,
            wet: 0.75,
            ir_gain_db: 0.0,
        });

        let source = interleaved(128, 13, 0.6);
        let mut a = source.clone();
        let mut b = source.clone();
        let mut c = source.clone();
        dry_only.process(&mut a);
        wet_only.process(&mut b);
        mixed.process(&mut c);

        // 干路的独立读数必须在**纯卷积**之外：先确认湿路不是恒等。
        let wet_energy: f32 = b.iter().map(|v| v * v).sum();
        assert!(wet_energy > 1e-6, "湿路输出全零 ⇒ 判据测的是空壳");

        for (i, ((mixed_v, dry_v), wet_v)) in c.iter().zip(&a).zip(&b).enumerate() {
            let want = 0.25 * dry_v + 0.75 * wet_v;
            assert!(
                (mixed_v - want).abs() < 1e-6,
                "样本 {i}: 混合值 {mixed_v} 与 0.25·{dry_v} + 0.75·{wet_v} = {want} 不符"
            );
        }
    }

    /// 量什么：`ir_gain_db = 0` 与 `ir_gain_db = 6.0206`（⇒ 线性增益**恰好** `2.0`）
    /// 两次全湿输出的每一个比特。
    ///
    /// 判据：后者逐位等于前者的 `2.0` 倍（乘 2 是精确的，不引入舍入）。
    /// 注入：漏掉 `ir_gain` 这一乘 ⇒ 两次读数相同 ⇒ 变红。
    #[test]
    fn ir_gain_db_doubling_is_bit_exact() {
        let ir = decaying_ir(180, 0.75);
        let mut unity = shell_with_ir(&ir);
        let mut doubled = shell_with_ir(&ir);
        unity.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        doubled.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 6.020_6,
        });
        assert_eq!(unity.ir_gain().to_bits(), 1.0f32.to_bits());
        assert_eq!(doubled.ir_gain().to_bits(), 2.0f32.to_bits());

        let source = interleaved(128, 17, 0.5);
        let mut a = source.clone();
        let mut b = source.clone();
        unity.process(&mut a);
        doubled.process(&mut b);
        let energy: f32 = a.iter().map(|v| v * v).sum();
        assert!(energy > 1e-6, "全湿输出全零 ⇒ 判据测的是空壳");
        for (i, (base, gain)) in a.iter().zip(&b).enumerate() {
            assert_eq!(
                gain.to_bits(),
                (base * 2.0).to_bits(),
                "样本 {i}: +6.0206 dB 必须逐位等于 2×（{gain} vs {}）",
                base * 2.0
            );
        }
    }

    /// 量什么：`set_params` 对退化输入的净化结果（单位：dB / 线性 / 帧）。
    ///
    /// 判据：四个字段都是有限值且在值域内；`NaN`/`±inf` 回落到缺省值。
    #[test]
    fn params_are_sanitised() {
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: f32::NAN,
            dry: f32::INFINITY,
            wet: f32::NEG_INFINITY,
            ir_gain_db: f32::NAN,
        });
        let p = shell.params();
        assert_eq!(
            p.pre_delay_s,
            ConvolutionReverbParams::default().pre_delay_s
        );
        assert_eq!(p.dry, ConvolutionReverbParams::default().dry);
        assert_eq!(p.wet, ConvolutionReverbParams::default().wet);
        assert_eq!(p.ir_gain_db, ConvolutionReverbParams::default().ir_gain_db);
        assert!(shell.ir_gain().is_finite() && shell.ir_gain() > 0.0);

        // 超界但不退化：钳到边界。用 96 kHz 让 `0.1 s` 恰好落在帧数上限上。
        shell.set_sample_rate(96_000.0);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 5.0,
            dry: 9.0,
            wet: -3.0,
            ir_gain_db: 900.0,
        });
        let p = shell.params();
        assert_eq!(p.pre_delay_s, MAX_PRE_DELAY_SECONDS);
        assert_eq!(p.dry, 1.0);
        assert_eq!(p.wet, 0.0);
        assert_eq!(p.ir_gain_db, MAX_IR_GAIN_DB);
        assert_eq!(shell.pre_delay_frames(), MAX_PRE_DELAY_FRAMES);
    }

    // -----------------------------------------------------------------------
    // 判据 9 … 11：通路映射与两个便捷入口
    // -----------------------------------------------------------------------

    /// 量什么：四条通路各自的**具名映射**。四条 IR 取互不相同的单抽头值
    /// （`1 / 2 / 4 / 8`）⇒ 任何一处错接都会被点名。
    ///
    /// 判据：`L` 打脉冲 ⇒ `(out_L, out_R) ≈ (h_LL, h_LR) = (1, 2)`；
    /// `R` 打脉冲 ⇒ `(out_L, out_R) ≈ (h_RL, h_RR) = (4, 8)`（容差 `1e-4`）。
    /// 注入：把 `h_lr` 与 `h_rl` 的位置对调 ⇒ 变红。
    #[test]
    fn the_four_paths_keep_their_named_order() {
        let h_ll = impulse_ir(1, 0, 1.0);
        let h_lr = impulse_ir(1, 0, 2.0);
        let h_rl = impulse_ir(1, 0, 4.0);
        let h_rr = impulse_ir(1, 0, 8.0);
        let make = || {
            let mut shell = ConvolutionReverb::new();
            shell.set_sample_rate(SR);
            assert_eq!(shell.set_impulse_response(&h_ll, &h_lr, &h_rl, &h_rr), 1);
            shell.set_params(ConvolutionReverbParams {
                pre_delay_s: 0.0,
                dry: 0.0,
                wet: 1.0,
                ir_gain_db: 0.0,
            });
            shell
        };
        let near = |v: f32, want: f32| (v - want).abs() < 1e-4;

        let mut from_left = vec![0.0f32; 256];
        from_left[0] = 1.0;
        let mut shell = make();
        shell.process(&mut from_left);
        assert!(near(from_left[0], 1.0), "L→L 期望 1，实测 {}", from_left[0]);
        assert!(near(from_left[1], 2.0), "L→R 期望 2，实测 {}", from_left[1]);

        let mut from_right = vec![0.0f32; 256];
        from_right[1] = 1.0;
        let mut shell = make();
        shell.process(&mut from_right);
        assert!(
            near(from_right[0], 4.0),
            "R→L 期望 4，实测 {}",
            from_right[0]
        );
        assert!(
            near(from_right[1], 8.0),
            "R→R 期望 8，实测 {}",
            from_right[1]
        );
    }

    /// 量什么：`set_stereo_impulse_response` 的两条对角通路与两条**静音**交叉通路。
    ///
    /// 判据：**只**往 `L` 打信号 ⇒ `R` 输出恰好 `0.0`；**只**往 `R` 打信号 ⇒ `L` 输出
    /// 恰好 `0.0`；有效的那一侧不为零（零 IR ⇒ 频域恒 0 ⇒ 逆变换恒 0，故是**恰好**
    /// 零，不是"很小"）。
    ///
    /// ⚠ 若把交叉通路留成**空 IR**，卷积核的语义是**直通** ⇒ 对侧会原样泄漏，
    /// 本判据立即变红。
    #[test]
    fn stereo_convenience_keeps_the_cross_paths_silent() {
        let h_l = decaying_ir(128, 0.25);
        let h_r = decaying_ir(128, 1.25);
        let build = || {
            let mut shell = ConvolutionReverb::new();
            shell.set_sample_rate(SR);
            assert_eq!(shell.set_stereo_impulse_response(&h_l, &h_r), 128);
            shell.set_params(ConvolutionReverbParams {
                pre_delay_s: 0.0,
                dry: 0.0,
                wet: 1.0,
                ir_gain_db: 0.0,
            });
            shell
        };

        // 只喂 L（`R` 全零）。
        let mut left_only = interleaved(128, 19, 0.5);
        for i in 0..128 {
            left_only[2 * i + 1] = 0.0;
        }
        let mut from_left = left_only.clone();
        build().process(&mut from_left);
        assert!(
            from_left.iter().step_by(2).any(|v| v.abs() > 1e-4),
            "L → L 通路输出全零 ⇒ 判据测的是空壳"
        );
        for i in 0..128 {
            assert_eq!(
                from_left[2 * i + 1],
                0.0,
                "帧 {i}: L → R 交叉通路必须是恰好 0"
            );
        }

        // 只喂 R（`L` 全零）。
        let mut right_only = interleaved(128, 23, 0.5);
        for i in 0..128 {
            right_only[2 * i] = 0.0;
        }
        let mut from_right = right_only.clone();
        build().process(&mut from_right);
        assert!(
            from_right.iter().skip(1).step_by(2).any(|v| v.abs() > 1e-4),
            "R → R 通路输出全零 ⇒ 判据测的是空壳"
        );
        for i in 0..128 {
            assert_eq!(from_right[2 * i], 0.0, "帧 {i}: R → L 交叉通路必须是恰好 0");
        }
    }

    /// 量什么：`set_mono_impulse_response` 的映射：同一条 IR 进两条对角通路。
    ///
    /// 判据：把**同一个序列**同时喂给 `L` 与 `R` ⇒ 两个输出声道**逐位相同**
    /// （两条对角通路走同一条 IR 与同一段算术），且不是全零。
    #[test]
    fn mono_convenience_feeds_both_channels_equally() {
        let ir = decaying_ir(96, 3.0);
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        assert_eq!(shell.set_mono_impulse_response(&ir), 96);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        // L 与 R 是**同一个序列**。
        let mut block = vec![0.0f32; 256];
        for i in 0..128 {
            let value = (i as f32 * 0.17).sin() * 0.5;
            block[2 * i] = value;
            block[2 * i + 1] = value;
        }
        shell.process(&mut block);
        let mut nonzero = false;
        for i in 0..128 {
            assert_eq!(
                block[2 * i].to_bits(),
                block[2 * i + 1].to_bits(),
                "帧 {i}: 单声道 IR 的两条对角通路必须逐位相同"
            );
            nonzero |= block[2 * i].abs() > 1e-6;
        }
        assert!(nonzero, "两条对角通路都输出 0 ⇒ 判据测的是空壳");
    }

    // -----------------------------------------------------------------------
    // 判据 12 … 15：块切分、奇数尾、reset、拒绝
    // -----------------------------------------------------------------------

    /// 量什么：**块长**对结果的影响 —— 一次 256 帧 vs 两次 128 帧；一次 300 帧 vs
    /// 128 → 128 → 44 帧。单位：比特。
    ///
    /// 判据：两条都**逐位相同**。这条把模块文档 §4 的切分口径钉死。
    /// 注入：把子块上界写成 `CONV_BLOCK_FRAMES - 1` ⇒ 变红。
    #[test]
    fn block_lengths_are_split_on_the_kernel_grid() {
        let ir = decaying_ir(300, 0.5);
        let params = ConvolutionReverbParams {
            pre_delay_s: 0.004,
            dry: 0.5,
            wet: 0.5,
            ir_gain_db: 3.0,
        };

        // 256 帧：一次 vs 两次 128 帧。
        let source = interleaved(256, 23, 0.6);
        let mut one = source.clone();
        let mut two = source.clone();
        let mut a = shell_with_ir(&ir);
        a.set_params(params);
        a.process(&mut one);
        let mut b = shell_with_ir(&ir);
        b.set_params(params);
        for chunk in two.chunks_mut(2 * CONV_BLOCK_FRAMES) {
            b.process(chunk);
        }
        for (i, (x, y)) in one.iter().zip(&two).enumerate() {
            assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "样本 {i}: 256 帧一次调用与两次 128 帧必须逐位相同"
            );
        }

        // 300 帧：一次 vs 128 → 128 → 44。
        let source = interleaved(300, 29, 0.5);
        let mut one = source.clone();
        let mut many = source.clone();
        let mut a = shell_with_ir(&ir);
        a.set_params(params);
        a.process(&mut one);
        let mut b = shell_with_ir(&ir);
        b.set_params(params);
        let split = [128usize, 128, 44];
        let mut offset = 0;
        for n in split {
            b.process(&mut many[2 * offset..2 * (offset + n)]);
            offset += n;
        }
        assert_eq!(offset, 300);
        for (i, (x, y)) in one.iter().zip(&many).enumerate() {
            assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "样本 {i}: 300 帧一次调用与 128+128+44 必须逐位相同"
            );
        }
    }

    /// 量什么：**奇数长度**块的处理帧数与最后一个样本的比特。
    ///
    /// 判据：返回 `block.len() / 2` 帧；最后一格（只有 `L`、没有 `R`）原样不碰。
    #[test]
    fn an_odd_trailing_sample_is_left_untouched() {
        let ir = decaying_ir(64, 0.0);
        let mut shell = shell_with_ir(&ir);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        // 7 个 f32 = 3 帧 + 1 个孤立的 L。
        let mut block = [0.0f32; 7];
        for (i, sample) in block.iter_mut().enumerate() {
            *sample = 0.25 + i as f32;
        }
        let orphan = block[6];
        assert_eq!(shell.process(&mut block), 3);
        assert_eq!(
            block[6].to_bits(),
            orphan.to_bits(),
            "奇数尾样本必须原样不碰"
        );
        assert_ne!(
            block[0].to_bits(),
            0.25f32.to_bits(),
            "前 3 帧必须真的处理过"
        );
    }

    /// 量什么：`reset()` 之后处理静音块的输出。
    ///
    /// 判据：全部**恰好**是 `0.0`（卷积历史与预延迟线都被清空）。
    /// 注入：删掉 `pre.fill(0.0)` ⇒ 预延迟线里残留的激励会漏出来 ⇒ 变红。
    #[test]
    fn reset_clears_the_tail_and_the_delay_line() {
        let ir = decaying_ir(256, 0.0);
        let mut shell = shell_with_ir(&ir);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.005,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        let mut excite = interleaved(128, 31, 0.9);
        shell.process(&mut excite);
        assert!(
            excite.iter().any(|v| v.abs() > 1e-3),
            "激励块没有产生湿信号 ⇒ 本判据测的是空壳"
        );
        shell.reset();
        let mut silence = vec![0.0f32; 256];
        shell.process(&mut silence);
        for (i, out) in silence.iter().enumerate() {
            assert_eq!(*out, 0.0, "样本 {i}: reset 之后静音输入必须给精确静音");
        }
    }

    /// 量什么：**不等长**与**空** IR 的返回值、配置状态与输出。
    ///
    /// 判据：拒绝（返回 `0`）、`is_configured()` 为假、`process` 逐位直通。
    #[test]
    fn degenerate_impulse_responses_are_rejected() {
        let three = decaying_ir(3, 0.0);
        let two = decaying_ir(2, 0.0);
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        assert_eq!(shell.set_impulse_response(&three, &two, &three, &three), 0);
        assert!(!shell.is_configured());
        let mut block = interleaved(128, 37, 0.7);
        let expected = block.clone();
        shell.process(&mut block);
        for (i, (out, want)) in block.iter().zip(&expected).enumerate() {
            assert_eq!(out.to_bits(), want.to_bits(), "样本 {i}: 拒绝后必须直通");
        }

        // 先成功配置，再用空 IR 拒绝 ⇒ 必须回到直通（不许留着上一条 IR 的能量）。
        assert_eq!(
            shell.set_impulse_response(&three, &three, &three, &three),
            3
        );
        assert!(shell.is_configured());
        assert_eq!(shell.set_impulse_response(&[], &[], &[], &[]), 0);
        assert!(!shell.is_configured());
        let mut block = interleaved(128, 41, 0.7);
        let expected = block.clone();
        shell.process(&mut block);
        for (i, (out, want)) in block.iter().zip(&expected).enumerate() {
            assert_eq!(out.to_bits(), want.to_bits(), "样本 {i}: 空 IR 后必须直通");
        }
    }

    /// 量什么：三个 IR 入口各自对一条含 `NaN` 的 IR 的返回值（单位：帧）与
    /// `is_configured()` 的读数（单位：布尔）。
    ///
    /// 判据：三个入口都返回 `0` 并把整台置回未配置；随后一个交错块的输出**逐位**
    /// 等于输入。好 IR 仍必须被接受（否则"拒绝"会退化成"永远拒绝"）。
    ///
    /// 注入（实测见本票报告）：两种都试过 —— 删掉卷积核的频谱有限性校验，
    /// 或让 `TrueStereoConvolution` 忽略四个返回值 ⇒ 本判据都在第一条断言处变红
    /// （实得 `256`，期望 `0`）。
    #[test]
    fn non_finite_impulse_responses_are_rejected_by_the_shell() {
        let good = decaying_ir(256, 0.3);
        let mut bad = good.clone();
        bad[5] = f32::NAN;

        // 四通路入口：坏的是 `h_LL`。
        let mut shell = shell_with_ir(&good);
        assert!(shell.is_configured());
        assert_eq!(
            shell.set_impulse_response(&bad, &good, &good, &good),
            0,
            "含 NaN 的 h_LL 必须被拒绝"
        );
        assert!(!shell.is_configured(), "拒绝后必须回到未配置");
        assert_eq!(shell.ir_frames(), 0, "拒绝后不得留下帧数");
        assert!(!shell.is_active(), "未配置时湿路不可闻");

        let mut block = interleaved(128, 43, 0.8);
        let expected = block.clone();
        shell.process(&mut block);
        for (i, (out, want)) in block.iter().zip(&expected).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: 拒绝后必须逐位直通"
            );
        }

        // 四通路入口：坏的是对侧的一条（`h_LR`）—— 不许只挡住对角线。
        assert_eq!(
            shell.set_impulse_response(&good, &bad, &good, &good),
            0,
            "含 NaN 的 h_LR 也必须拒绝整台"
        );
        assert!(!shell.is_configured());

        // 普通立体声入口：`h_l` 坏；`h_r` 坏同样要拒。
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        assert_eq!(shell.set_stereo_impulse_response(&bad, &good), 0);
        assert!(!shell.is_configured());
        assert_eq!(shell.set_stereo_impulse_response(&good, &bad), 0);
        assert!(!shell.is_configured());
        assert_eq!(
            shell.set_stereo_impulse_response(&good, &good),
            256,
            "好 IR 必须被接受"
        );
        assert!(shell.is_configured());

        // 单声道入口。
        assert_eq!(shell.set_mono_impulse_response(&bad), 0);
        assert!(!shell.is_configured());
        assert_eq!(shell.set_mono_impulse_response(&good), 256);
        assert!(shell.is_configured());

        // 频谱溢出（有限但过大）走同一条拒绝路径。
        let overflow = vec![3.0e38f32; 256];
        assert_eq!(shell.set_mono_impulse_response(&overflow), 0);
        assert!(!shell.is_configured());
    }

    /// 量什么：一条**已经跑起来**的湿路在换入坏 IR 之后的非有限样本个数（单位：个）
    /// 与一个交错块的每一个比特。
    ///
    /// 判据：非有限样本 `0` 个，且输出**逐位**等于输入。没有这条，坏 IR 会让湿路
    /// 从此恒为 `NaN` —— `reset()` 清不掉 `ir_*`，而 `wet = 0` 也救不回来
    /// （实测：`finite * 0.0` 仍是 `NaN`）。
    ///
    /// 注入（实测见本票报告）：删掉卷积核的频谱有限性校验 ⇒ 本判据在**第一条**
    /// 断言（`set_impulse_response` 必须返回 `0`）处变红（实得 `512`，期望 `0`）；
    /// `non_finite == 0` 那条是**第二道**防线，只有在返回值被另一次改动静默忽略时
    /// 才会轮到它。
    #[test]
    fn a_rejected_impulse_response_cannot_poison_the_wet_path() {
        let good = decaying_ir(512, 0.7);
        let mut shell = shell_with_ir(&good);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: 0.0,
            dry: 0.0,
            wet: 1.0,
            ir_gain_db: 0.0,
        });
        let mut warmup = interleaved(128, 47, 0.9);
        shell.process(&mut warmup);
        assert!(shell.is_active(), "前置条件：湿路必须是开的");

        let mut bad = good.clone();
        bad[11] = f32::INFINITY;
        assert_eq!(shell.set_impulse_response(&bad, &bad, &bad, &bad), 0);
        assert!(!shell.is_configured(), "拒绝后必须回到未配置");
        assert!(!shell.is_active());

        let mut block = interleaved(128, 53, 0.9);
        let expected = block.clone();
        shell.process(&mut block);
        let non_finite = block.iter().filter(|v| !v.is_finite()).count();
        assert_eq!(
            non_finite, 0,
            "换入坏 IR 之后湿路仍产出非有限值 ⇒ IR 校验没挡住"
        );
        for (i, (out, want)) in block.iter().zip(&expected).enumerate() {
            assert_eq!(
                out.to_bits(),
                want.to_bits(),
                "样本 {i}: 拒绝后必须逐位直通"
            );
        }

        // 好 IR 换回来必须能重新工作（拒绝不是"一票永久停机"）。
        assert_eq!(shell.set_impulse_response(&good, &good, &good, &good), 512);
        assert!(shell.is_configured());
        let mut again = interleaved(128, 53, 0.9);
        shell.process(&mut again);
        assert!(
            again.iter().all(|v| v.is_finite()),
            "换回好 IR 之后仍有非有限值"
        );
        assert!(
            again
                .iter()
                .zip(&expected)
                .any(|(out, want)| (out - want).abs() > 1e-6),
            "换回好 IR 之后湿路没有参与运算"
        );
    }

    /// 量什么：`latency_samples()` 的读数（单位：帧）。
    ///
    /// 判据：恒为 `CONV_LATENCY = 0`；预延迟**不**进这个数（模块文档 §2.1）。
    #[test]
    fn pre_delay_is_not_pdc_latency() {
        let mut shell = ConvolutionReverb::new();
        shell.set_sample_rate(SR);
        // 96 kHz 下 `0.1 s` 恰好是帧数上限；预延迟再长也不进 PDC 延迟。
        shell.set_sample_rate(96_000.0);
        shell.set_params(ConvolutionReverbParams {
            pre_delay_s: MAX_PRE_DELAY_SECONDS,
            ..ConvolutionReverbParams::default()
        });
        assert_eq!(shell.pre_delay_frames(), MAX_PRE_DELAY_FRAMES);
        assert_eq!(shell.latency_samples(), 0);
    }

    /// 量什么：`Debug` 的形状（单位：无）。
    ///
    /// 判据：只打印标量字段，**不**把两条上限长的预延迟线打进字符串。
    #[test]
    fn debug_prints_only_scalars() {
        let mut shell = shell_with_ir(&decaying_ir(64, 0.0));
        shell.set_params(ConvolutionReverbParams::default());
        let text = format!("{shell:?}");
        assert!(text.contains("ConvolutionReverb"), "实测: {text}");
        assert!(
            text.len() < 400,
            "Debug 输出过长（{} 字节）⇒ 缓冲被打印了: {text}",
            text.len()
        );
    }
}
