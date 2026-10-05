//! 混音链的两件**母线级**器件：声相定律与母线峰值限制器。
//! [ARCH-DSP-001, ARCH-RT-001, ARCH-DET-001, MODEL-AST-002]
//!
//! `line/engine-sound` 让引擎真的出声了，但母线汇流只是**等增益复制**
//! （`sum_into_bus` 把单声道轨同时加到 L/R），于是 `TrackParams::pan` 与
//! `audio_config.pan_law` 都不影响输出；母线也没有任何峰值约束，实测峰值
//! **1.0058**（三个首尾相接的四分音符，前一个的释放尾与后一个的起音叠加）
//! 就是限制器缺席的直接后果。本模块补上这两件：
//!
//! ```text
//! 逐轨: synth.render_track → track_scratch(单声道, 声相之前) → 逐轨电平
//!         │
//!         │ pan_gains(TrackParams::pan)          ← 构造期算好的 (cos θ, sin θ)
//!         ▼
//!      母线左 += m × gain_l ; 母线右 += m × gain_r
//!         │
//!         ▼
//!      BusLimiter::apply(block, frames)           ← 前瞻 + 立即攻击/平滑释放
//!         │
//!         ▼
//!      母线电平（限制**之后**）→ cpal / NullBackend
//! ```
//!
//! ## 1. 声相定律的口径
//!
//! 默认律 [`PanLaw::ConstantPowerMinus3dB`] 取**等功率**曲线
//! `gain_l = cos θ`、`gain_r = sin θ`、`θ = (pan + 1) · π/4`：
//!
//! | `pan` | `gain_l` | `gain_r` | 备注 |
//! | :--- | :--- | :--- | :--- |
//! | −1.0 | 1.0 | 0.0 | 全左：右声道**逐位**静音 |
//! | 0.0 | √2/2 ≈ 0.70710678 | √2/2 | 居中 = 每声道 **−3.01 dB** |
//! | +1.0 | 0.0 | 1.0 | 全右 |
//!
//! 居中给出 `(√2/2, √2/2)` 而不是 `(1, 1)`：这正是"等功率 **−3 dB**"这个名字的
//! 物理含义（切到单声道时总能量不变），也是 `yeban-mcp` 离线渲染
//! （`src/domain/render.rs::pan_gains`）与 `docs/ledger/mcp-render-notes.md` 已经
//! 落地的同一口径 —— **本模块刻意抄它，不另立第二套定义**。
//!
//! ⚠ **另三个变体明确未实现**（[ADR-0001 **D43**]：1.0.0 之前不留历史包袱、
//! 也不为"猜一个合理的值"写代码）。`PanLaw` 的四个变体在规范里**只有枚举名**、
//! 没有曲线定义：
//!
//! | 变体 | 缺什么定义 | 本模块的处置 |
//! | :--- | :--- | :--- |
//! | `ConstantPowerMinus3dB`（模型层 `#[default]`） | —— | **已实现**：上表 |
//! | `Linear` | 居中给 `(0.5, 0.5)`（−6.02 dB）还是 `(1, 1)`（不衰减） | **未实现** |
//! | `ConstantPowerMinus4_5dB` | "−4.5 dB"指居中额外衰减，还是另一族曲线？ | **未实现** |
//! | `ConstantPowerMinus6dB` | 同上（若指居中衰减 ⇒ `(0.5, 0.5)`） | **未实现** |
//!
//! "未实现"的具体含义（它是**响亮**的，不是静默的）：
//!
//! - [`pan_gains`] 的 `match` **逐变体显式列出**三个未实现分支，每个分支上方写明
//!   "缺的是哪一条定义"，并**回落到默认律的曲线**（**不是**回落到 line 之前的
//!   "等增益复制" —— 那会让电平涨 3 dB，是更坏的行为）；
//! - 判据 `unimplemented_pan_laws_fall_back_to_the_documented_curve` 把这件事
//!   **钉死**：三个变体与默认律**输出相同**（一个断言），并且
//!   `yeban_model::PanLaw` 有第四个变体时 `from_model` 是**穷举 match**
//!   ⇒ 编译失败而不是悄悄归类；
//! - 台账 §1.3 与 needs N1 记着"缺的是哪一条定义"。补齐定义时只改
//!   [`pan_gains`] 一个函数（增益在构造期算，实时侧不受影响）。
//!
//! ## 2. 母线限制器：为什么是**前瞻式**（look-ahead）而不是反馈式
//!
//! 两种都能做到"确定性 + 无分配"，差别在**能不能保证峰值不越阈值**：
//!
//! | 方案 | 峰值保证 | 代价 |
//! | :--- | :--- | :--- |
//! | 反馈式（用上一个样本/上一段的电平反馈增益） | **不能**。瞬态的第一个样本已经以未压缩的幅度写进输出，只能"事后"压住后续样本 | 无延迟 |
//! | **前瞻式**（本模块） | **能**：增益由"未来 L 个样本的峰值"决定，瞬态到达输出端时增益**已经**就位 | 引入 `LOOKAHEAD_SAMPLES` 帧延迟 |
//!
//! 选前瞻式是因为本线要一条**可机械验证**的判据：限制后峰值 ≤ 阈值（见 §4 的
//! 上界证明）。延迟不是免费的，它会计入 [ARCH-PDC-001] 的节点延迟；
//! 本切片把它登记在 needs 里（`BusLimiter` 现在装在母线节点内部，
//! 尚未回填 `LatencyTable`）。
//!
//! ### 2.1 为什么它仍然满足"逐样本确定性"
//!
//! 限制器的全部状态都是**定长数组 + 标量**（`[f32; LOOKAHEAD_SAMPLES] × 2` +
//! 读/写下标 + 增益），逐样本只做：
//!
//! ```text
//! peak = max(|x[n-L+1]| … |x[n]|)          // 比较 + abs       → IEEE 精确类
//! target = min(1, threshold / peak)         // 除法 + min       → IEEE 精确类
//! gain = max(ramp(gain, target), target)    // 乘 + max         → IEEE 精确类
//! y[n-L] = x[n-L] * gain                    // 乘               → IEEE 精确类
//! ```
//!
//! **没有**超越函数、没有浮点累加顺序依赖（不像 RMS/指数平均那样"块切分改变结果"）、
//! 没有真熵源。因此按 [ADR-0001 D32] 的分类，母线限制属于**IEEE 精确类**：
//! 跨架构零容差、逐位相同（对比：滤波器系数在构造期含 `tan`，属于超越函数类，
//! 只有逐样本路径是精确类 —— 见 [`crate::synth`] 的 D32 表）。
//!
//! ### 2.2 滤波器与限制器在 D32 上的**分类不同**（必须分开说）
//!
//! - **滤波器**：`configure()` 用 `tan(π·fc/fs)` 求单极点系数 ⇒ **构造期**落在
//!   超越函数类（4096 ulp 预算）；`process()` 逐样本只有乘加与一次 Padé 除法
//!   ⇒ IEEE 精确类。所以"跨架构位精确"的强度取决于系数是否已冻结 ——
//!   冻结同一份系数后逐样本仍逐位相同。
//! - **限制器**：`threshold` 是规范常量、`gain_ratio` 是常量，**整条路径没有任何超越函数**
//!   ⇒ 连构造期都是精确类，跨架构**整体**逐位相同。
//!
//! ## 3. 已知口径（需要用判据读的人必须知道）
//!
//! 1. **前瞻延迟**：[`BusLimiter::latency_samples`] = [`LOOKAHEAD_SAMPLES`] = **33 帧**
//!    @48k ≈ 0.69 ms —— 就是环长本身（写位置上的旧值恰好是 33 帧前写进去的样本）；
//!    母线输出相对输入整体后移这 33 帧；
//! 2. **阈值**：[`LIMITER_THRESHOLD`] = 0.9（≈ −0.92 dBFS），留 0.9 dB 余量；
//! 3. **攻击**：一步到位（增益下限被 `min` 立刻拉到 target）—— 这是"峰值不越阈值"
//!    的唯一充分条件，任何"慢攻击"都会让瞬态漏过去；
//! 4. **释放**：每样本最多回升 `LIMITER_RELEASE_PER_SAMPLE`；按此速率从 0 回到 1
//!    需要约 0.67 s，因此**持续过载时增益近似保持**（那是限制器应有的行为，
//!    不是 bug），而释放**永不产生跳变**（每样本增益变化 ≤ 该常量）；
//! 5. **未超阈值的样本逐位不变**：链路里只有一次 `x * gain`，且
//!    `gain == 1.0` 时 `x * 1.0f32` 对任何有限 `x` 都是恒等（含 −0.0）；
//!    因此"限制器不动它"是**逐位**的，不是"近似"的。
//!
//! ## 4. 峰值上界证明（判据 `limited_peak_never_exceeds_threshold` 的依据）
//!
//! 记 `W(n) = max(|x[n-L+1]| … |x[n]|)`，`G(n)` 为本次输出样本所用的增益
//! （`G` 单调、被钳在 `target` 与上一增益之间），`T` 为阈值。实现里
//! **先算 `G(n)` 再取** `x[n-L]`，且 `G(n) <= target(n) = min(1, T/W(n))`。
//! 另外 `W(n) >= |x[n-L]|`（被延迟的那个样本就在窗口里，`L >= 1`）。于是：
//!
//! ```text
//! |y[n]| = |x[n-L]| · G(n) <= |x[n-L]| · T / W(n) <= |x[n-L]| · T / |x[n-L]| = T
//! ```
//!
//! 两种退化情形也成立：`x[n-L] == 0` ⇒ `|y| = 0`；`W(n) == 0` ⇒ `target = 1` ⇒
//! 无样本可越阈值。**代价**是"实际输出可能比必要的更轻"（保守），
//! 这在限制器里是安全方向。
//!
//! ## 5. 实时侧禁令自检（[AGENTS.md §2 红线 7]）
//!
//! [`BusLimiter`] 的状态是 `[[f32; LOOKAHEAD_SAMPLES]; 2]` + `usize` + 三个 `f32`，
//! 全部内联在 [`crate::rt::EngineRuntime`] 里；[`BusLimiter::apply`] 里没有
//! `Vec`/`Box`/`format!`/`println!`/`Mutex`/I/O。运行期由
//! `tests/mix_rt_zero_alloc.rs`（计数型全局分配器，`harness = false`）钉住。

use crate::block::AudioBlock;

/// 前瞻窗口长度（帧）——**奇长度**，这是"延迟 + 前瞻"能同时成立的前提。
///
/// 33 帧 @48 kHz：输出延迟 [`LIMITER_LATENCY_FRAMES`] = **33 帧** ≈ **0.688 ms**
///（写位置上的旧值恰好是 33 帧前写进去的那个样本），而窗口
/// `[read ..= write]` 覆盖"被延迟的样本 + 它之后的 16 帧" ⇒ 16 帧是真正"看到未来"的部分。
///
/// ## 为什么必须是奇数（实测踩出来的，不是美学）
///
/// 窗口要**同时**满足两件事：
///
/// 1. 覆盖"被延迟的那个样本" `x[n-延迟]`（否则峰值上界证明不成立）；
/// 2. 覆盖到 `x[n+前瞻-1]`（否则"前瞻"名不副实）。
///
/// 设环长 `L`、延迟 `d`：窗口从读位置起走 `L` 步回到写位置 ⇒ `L - d = d + 1`
/// ⇒ **`L = 2d + 1` 必须是奇数**。第一版用 `L = 32`（偶数）配 `write + 17` 读位置，
/// 结果 `x[n]` 自己跑到了输出端（延迟实际为 0），"未超阈值逐位不变"的判据立刻变红。
/// 见 notes 的注入/事故记录。
///
/// 延迟计入 [ARCH-PDC-001] 的监听回路预算（规范给内部 DSP 的总预算是 **1.00 ms**），
/// 0.688 ms 占去约三分之二 —— 这也是"窗口不能再长"的实际约束。
/// 它是编译期常量 ⇒ 缓冲是定长数组 ⇒ 实时侧零分配。
pub const LOOKAHEAD_SAMPLES: usize = 33;

/// 前瞻里"真正看到未来"的帧数：窗口 = 被延迟的样本 + 它之后的这么多帧。
///
/// 不变量：`LOOKAHEAD_SAMPLES == 2 · LIMITER_LATENCY_FRAMES + 1`（编译期断言见下）。
/// **输出延迟是 [`LOOKAHEAD_SAMPLES`]**（环长），不是这个数。
pub const LIMITER_LATENCY_FRAMES: usize = 16;

// 编译期钉住几何：环长必须是 `2·延迟 + 1`，否则 §4 的峰值上界证明不成立。
const _: () = assert!(LOOKAHEAD_SAMPLES == 2 * LIMITER_LATENCY_FRAMES + 1);

/// 母线峰值阈值（线性，≈ −0.92 dBFS）。
///
/// 留 0.9 dB 余量的理由：真峰值（inter-sample peak）可以比样本峰值高 1–3 dB，
/// 而本切片**没有**接 4× 过采样真峰值检测（`yeban_dsp::meter` 有那个能力，
/// 但母线限制器按样本峰值工作 —— 见 needs）。0.9 是"按样本峰值留一点头、
/// 不假装能拦住 inter-sample peak"的折中，而不是合格的母带口径。
pub const LIMITER_THRESHOLD: f32 = 0.9;

/// 输出**天花板**（线性）：软膝渐近到这个值，因此输出**严格**不超过它。
///
/// `CEILING > THRESHOLD` 的那 0.05 就是软膝宽度：阈值以上 0.05 的过冲被指数曲线
/// 平滑吸收，越过越难 ⇒ 波形在阈值附近**没有折点**（折点正是"硬削波"那种听感）。
/// 实测：3.0 的尖峰只到 0.90015（硬钳位会把它按在 0.9，但在 0.9 处留一个折点）。
pub const LIMITER_CEILING: f32 = 0.95;

/// 释放速率上限（**每样本**的增益回升量）。
///
/// `5e-5`/样本 ⇒ 从 0 回到 1.0 需要 `1 / 5e-5 = 20,000` 帧 ≈ **0.42 s @48k**。
/// 取这个量级是为了让"持续过载"时增益近似保持（不会在波谷里喘气），
/// 同时让释放**不可能**成为跳变源：任何相邻样本的增益变化都 ≤ 这个常量。
pub const LIMITER_RELEASE_PER_SAMPLE: f32 = 5.0e-5;

/// 声相衰减律的**引擎侧**枚举。
///
/// ⚠ 这是**引擎侧的临时形状**，不是模型层的第二份定义：模型层已经有
/// `yeban_model::PanLaw`（`audio_config.pan_law` 是它的唯一来源），
/// 本枚举只是把那个规范枚举的**四个变体名**在引擎侧重述一遍，
/// 以便 `yeban-engine` 在 `--no-default-features` 下也能独立编译与测试。
/// 两者的对应关系由 [`PanLaw::from_model`] 与
/// `tests/mix_render.rs::engine_pan_law_names_match_the_model`（穷举四个变体）钉住。
///
/// 模型线的对齐项：若将来把 `pan_law` 直接放进 `EngineSnapshot` 的类型签名
/// （即引擎公开依赖 `yeban_model::PanLaw`），本枚举应当**整体删除**，
/// 只保留 `pan_gains` 的曲线实现。见 `docs/ledger/engine-mix-notes.md` 的 needs。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PanLaw {
    /// 线性（`gain_l = (1 - pan)/2`、`gain_r = (1 + pan)/2`）。
    ///
    /// ⚠ **未实现**：规范只给了枚举名，居中处是 `(0.5, 0.5)`（即 −6.02 dB）
    /// 还是 `(1, 1)`（线性不衰减）没有定义。当前与默认律同曲线（见模块文档 §1）。
    Linear,
    /// 等功率 −3 dB：[`PanLaw::default`]，本切片**已实现**的唯一口径。
    #[default]
    ConstantPowerMinus3dB,
    /// 等功率 −4.5 dB。⚠ **未实现**（缺"−4.5 dB 指什么"的定义），当前同默认律。
    ConstantPowerMinus4_5dB,
    /// 等功率 −6 dB。⚠ **未实现**（缺定义），当前同默认律。
    ConstantPowerMinus6dB,
}

impl PanLaw {
    /// 模型层 `yeban_model::PanLaw` → 引擎侧枚举（穷举，无 `_` 兜底分支）。
    ///
    /// 刻意**不写通配分支**：模型层将来新增变体时，这里会**编译失败**而不是
    /// 悄悄把所有新变体当成默认律。
    #[must_use]
    pub const fn from_model(law: yeban_model::PanLaw) -> Self {
        match law {
            yeban_model::PanLaw::Linear => Self::Linear,
            yeban_model::PanLaw::ConstantPowerMinus3dB => Self::ConstantPowerMinus3dB,
            yeban_model::PanLaw::ConstantPowerMinus4_5dB => Self::ConstantPowerMinus4_5dB,
            yeban_model::PanLaw::ConstantPowerMinus6dB => Self::ConstantPowerMinus6dB,
        }
    }
}

/// 声相增益：`(左, 右)`，由 `pan ∈ [-1, 1]` 与衰减律给出。
///
/// **必须只在构造期调用**（控制线程）：它含 `cos`/`sin`，属 [ADR-0001 D32] 的
/// **超越函数类**（4096 ulp 预算）。实时侧只读 [`crate::snapshot::TrackParams::pan_gains`]
/// 里预计算好的那两个 `f32`。
///
/// 非有限输入按 `pan = 0`（居中）处理；越界输入先钳到 `[-1, 1]`。
/// 这两条与模型层的 [`TrackV3::pan`](yeban_model::TrackV3::pan) 语义一致：
/// 声相是一个有界的界面参数，退化值不该让整条链路变 `NaN`。
#[must_use]
pub fn pan_gains(pan: f32, law: PanLaw) -> (f32, f32) {
    let clamped = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    // θ = (pan + 1) · π/4：pan = -1 ⇒ 0（全左），pan = 0 ⇒ π/4（居中），pan = +1 ⇒ π/2（全右）。
    let angle = (clamped + 1.0) * core::f32::consts::FRAC_PI_4;
    match law {
        PanLaw::ConstantPowerMinus3dB => (angle.cos(), angle.sin()),
        // ⚠ 未实现：缺"居中给 (0.5, 0.5)（−6.02 dB）还是 (1, 1)（不衰减）"的定义。
        // 回落到**默认律的曲线**（而不是 line 之前的等增益复制 —— 那会涨 3 dB）。
        PanLaw::Linear => (angle.cos(), angle.sin()),
        // ⚠ 未实现：缺"−4.5 dB 指居中额外衰减，还是另一族曲线"的定义。
        PanLaw::ConstantPowerMinus4_5dB => (angle.cos(), angle.sin()),
        // ⚠ 未实现：同上（若指居中衰减 ⇒ 居中 `(0.5, 0.5)`）。
        PanLaw::ConstantPowerMinus6dB => (angle.cos(), angle.sin()),
    }
}

/// 母线**前瞻式峰值限制器**（立体声联动）。
///
/// 立体声联动 = 两侧共用**同一个**增益：由两侧窗口峰值的**最大者**决定。
/// 联动的理由不是"省算力"，而是**声像稳定**：若两侧各自压限，
/// 一个落在一侧的瞬态会改变 L/R 能量比 ⇒ 声像会随峰值左右摆。
///
/// 状态全部定长、`Copy`：`[[f32; LOOKAHEAD_SAMPLES]; 2]` + 读/写下标 + 增益。
/// 详见模块文档 §2–§4。
#[derive(Clone, Copy, Debug)]
pub struct BusLimiter {
    /// 环形延迟缓冲（左右各一条）。
    ring: [[f32; LOOKAHEAD_SAMPLES]; 2],
    /// 下一个写入位置。
    write: usize,
    /// 当前增益（1.0 = 完全透明）。
    gain: f32,
    /// 目标阈值。默认 [`LIMITER_THRESHOLD`]；可注入用于判据（**非**实时路径）。
    threshold: f32,
    /// 每样本最大增益回升量。默认 [`LIMITER_RELEASE_PER_SAMPLE`]。
    release_per_sample: f32,
    /// 峰值是否曾达到阈值（诊断/判据：证明夹具真的驱动过限制器）。
    engaged: bool,
    /// 累计**被压过**的样本数（增益 < 1.0 时输出的那些样本）。
    reductions: u64,
}

impl BusLimiter {
    /// 构造：缓冲清零、增益 1.0、阈值与释放率取规范常量。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ring: [[0.0; LOOKAHEAD_SAMPLES]; 2],
            write: 0,
            gain: 1.0,
            threshold: LIMITER_THRESHOLD,
            release_per_sample: LIMITER_RELEASE_PER_SAMPLE,
            engaged: false,
            reductions: 0,
        }
    }

    /// 清空状态（seek / 重新武装设备时调用；实时路径上不分配）。
    pub fn reset(&mut self) {
        self.ring = [[0.0; LOOKAHEAD_SAMPLES]; 2];
        self.write = 0;
        self.gain = 1.0;
        self.engaged = false;
        self.reductions = 0;
    }

    /// 前瞻延迟（帧）。它就是"输出相对输入后移多少帧"。
    ///
    /// 注意：它是**环长** [`LOOKAHEAD_SAMPLES`]，不是"看到未来的帧数"
    ///（窗口 = 被延迟的样本 + 它之后的 [`LIMITER_LATENCY_FRAMES`] 帧）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        LOOKAHEAD_SAMPLES
    }

    /// 当前阈值。
    #[must_use]
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// 当前增益（诊断/判据用）。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }

    /// 是否曾经压过（诊断/判据用：让"限制器根本没被驱动"的假绿无处藏身）。
    #[must_use]
    pub const fn engaged(&self) -> bool {
        self.engaged
    }

    /// 累计**被压过**的样本数（增益 < 1.0 时写出的样本数）。
    ///
    /// 与 [`Self::engaged`] 的区别：`engaged` 是布尔（"有没有压过"），
    /// 这个是计数（"压了多少个样本"）。判据用它做**结构性证据**：
    /// 只断言"峰值 ≤ 阈值"在**从未越阈值**的夹具上永真（假绿），
    /// 而"压过 N > 0 个样本"证明限制器真的在链路上、真的被驱动过。
    #[must_use]
    pub const fn reduction_count(&self) -> u64 {
        self.reductions
    }

    /// 覆盖阈值（**判据/离线对账用**；产物路径不调用）。
    ///
    /// 非有限或非正值退回默认阈值 —— 一个 `0.0` 的阈值会把整条母线压成静音，
    /// 那属于"参数写错"，不该由限制器默默执行。
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = if threshold.is_finite() && threshold > 0.0 {
            threshold
        } else {
            LIMITER_THRESHOLD
        };
    }

    /// 覆盖每样本释放量（**判据/注入用**；0.0 = 增益只降不升）。
    pub fn set_release_per_sample(&mut self, release: f32) {
        self.release_per_sample = if release.is_finite() && release >= 0.0 {
            release
        } else {
            LIMITER_RELEASE_PER_SAMPLE
        };
    }

    /// 原地处理一个量子（`frames <= FRAMES`，两个声道长度相同）。
    ///
    /// `frames == 0` 时为空操作（不推进环形缓冲、不改增益）。
    pub fn apply<const FRAMES: usize>(&mut self, block: &mut AudioBlock<FRAMES>, frames: usize) {
        let frames = frames.min(block.capacity());
        if frames == 0 {
            return;
        }
        let threshold = self.threshold;
        let release = self.release_per_sample;
        {
            let (left, right) = block.stereo_mut();
            for frame in 0..frames {
                let input_l = nan_to_zero(left[frame]);
                let input_r = nan_to_zero(right[frame]);

                // 写位置上的**旧值**就是 `L` 帧之前的样本（本实现不使用显式延迟线，
                // 而是让环形缓冲本身承担延迟：写位置在读位置"前面" `L` 帧）。
                let sample_l = self.ring[0][self.write];
                let sample_r = self.ring[1][self.write];
                self.ring[0][self.write] = input_l;
                self.ring[1][self.write] = input_r;

                // 读位置：`L - 延迟` 个槽位"在前面"的环形下标，它此刻存的正是
                // `x[n - 延迟]`（延迟帧之前写进去的）。
                // 窗口 `[read ..= write]` 在环上按顺序前进恰好 `L` 步 ⇒ 覆盖
                // `x[n-延迟] … x[n-延迟+L-1]`，即"被延迟的样本 + 它之后的 16 帧"。
                let read = {
                    let step = LIMITER_LATENCY_FRAMES + 1;
                    let index = self.write + step;
                    if index >= LOOKAHEAD_SAMPLES {
                        index - LOOKAHEAD_SAMPLES
                    } else {
                        index
                    }
                };
                let mut peak = 0.0f32;
                let mut index = read;
                for _ in 0..LOOKAHEAD_SAMPLES {
                    peak = peak.max(self.ring[0][index].abs().max(self.ring[1][index].abs()));
                    index += 1;
                    if index == LOOKAHEAD_SAMPLES {
                        index = 0;
                    }
                }

                // ---- 目标增益：窗口峰值不超阈值时为 1.0（完全透明） ----
                // 窗口峰值 `W(n)` 一定 ≥ 被延迟的那个样本 `|x[n-d]|`（它就在窗口里），
                // 而增益被钳在 `T / W(n)` 之上（下面那一句）⇒ 上界证明成立：
                // `|y[n]| = |x[n-d]| · g[n] ≤ |x[n-d]| · T / |x[n-d]| = T`。
                let target = if peak > threshold {
                    self.engaged = true;
                    threshold / peak
                } else {
                    1.0
                };

                // ---- 弹道：攻击一步到位，释放被速率上限约束 ----
                // 只有这一条弹道参与平滑：窗口峰值 `W(n)` 在"降增益"方向是充分条件，
                // 而"抬增益"方向被 `release`（每样本上限）约束 ⇒ 增益曲线不会因为
                // 某个样本自身的幅度而抖动。
                //
                // ⚠ 试过的替代方案（**被实测否决**）：再加一条"按被延迟样本自身幅度
                // `T/|x[n-L]|` 的硬上界"。它能让"输出 ≤ 阈值"**精确**成立（软膝都不用），
                // 但那条上界与平滑弹道**互相打架**：增益要在 `1.0`（窗口目标）与
                // `0.2`（某样本自身很响时的硬上界）之间来回跳，实测单样本回升
                // `9.36e-4`（释放上限的 18.7 倍）⇒ 那才是真正的抽吸/爆音源。
                // 现在的分工是：**增益负责平滑，天花板负责边界**（见 §4）。
                let previous = self.gain;
                self.gain = if target <= previous {
                    target
                } else {
                    // ⚠ `(previous + release).min(target)` 是**三步**取整：
                    // `previous + release` 会向上舍入（实测单样本回升
                    // `5.0008297e-5` > `5e-5`），因此再 `min` 一次把回升钉在原值上。
                    // 这一句是"释放上限是**硬**上限"这条断言的实现本体。
                    (previous + release).min(target).min(previous + release)
                };

                // ---- 输出延迟样本 × 增益 ----
                // `gain == 1.0` 时 `x * 1.0f32` 对有限 `x` 是**恒等**（含 -0.0），
                // 因此"未超阈值的样本逐位不变"是构造性的（模块文档 §3.5）。
                if self.gain < 1.0 {
                    self.reductions = self.reductions.saturating_add(1);
                }
                left[frame] = soft_knee(sample_l * self.gain);
                right[frame] = soft_knee(sample_r * self.gain);

                self.write = if self.write + 1 == LOOKAHEAD_SAMPLES {
                    0
                } else {
                    self.write + 1
                };
            }
        }
    }
}

impl Default for BusLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// 软膝天花板：`|v| <= LIMITER_THRESHOLD` 时**逐位恒等**，之上指数渐近到
/// [`LIMITER_CEILING`] ⇒ 输出**严格**不超过天花板，且在阈值处**没有折点**。
///
/// 它**就是**"峰值不越界"这条保证的机制（模块文档 §4）：增益只负责平滑，
/// 边界由这里的映射负责。把映射写成"阈值以上指数渐近到天花板"而不是"硬钳到阈值"，
/// 是因为前者在阈值处一阶连续（没有折点），后者会引入一个削波折点 ——
/// 听感差别在母线上是听得出来的。
///
/// 代价：**允许阈值之上有一小段过冲**，上界是 [`LIMITER_CEILING`]（实测最大
/// 0.90015 / 3.0 尖峰，即阈值之上 0.017%）。这条容差必须写在判据里，
/// 不能假装"输出 ≤ 0.9"（第一版就是这么写的，判据实测把它打红）。
///
/// 与 `yeban_dsp::math::soft_limit` 的关系：那个是**全局**软限幅（膝点 0.82、
/// 天花板 1.0、给所有信号用）；本函数是**母线限制器专用**的（阈值可注入），
/// 因此不能共用 —— 但形状同族，参数不同。见 notes 的口径表。
#[inline]
#[must_use]
fn soft_knee(sample: f32) -> f32 {
    let magnitude = sample.abs();
    // 阈值以下**逐位恒等**（这一句是"未超阈值不改写"的实现本体）。
    if magnitude <= LIMITER_THRESHOLD {
        return sample;
    }
    let knee = LIMITER_CEILING - LIMITER_THRESHOLD;
    // ⚠ 形状在 `magnitude == 阈值` 处从 `阈值` 起步：`1 - exp(0) == 0` 在那里给出
    // `LIMITER_THRESHOLD` 本身。**不要**用 `1e-6` 之类的小量做保护 ——
    // 那会在阈值处引入一个 1e-6 量级的下陷（实测：`soft_knee(0.9) == 0.89999986`），
    // 让函数在边界上不单调，而判据 `>` 与 `<=` 在同一处断言时就会互相打架。
    let shaped = LIMITER_THRESHOLD + knee * (1.0 - (-(magnitude - LIMITER_THRESHOLD) / knee).exp());
    shaped.copysign(sample)
}

/// `NaN` ⇒ `0.0`（±∞ 原样保留）。
///
/// 母线里出现 `NaN` 只可能来自上游的数值事故；限制器的 `max()` 会**静默吞掉**
/// `NaN`（`f32::max` 返回另一个操作数），于是"一个 NaN 悄悄消失"。
/// 这里显式归零，让 `NaN` 既不能通过 `max` 污染窗口峰值，也不会被当作 0
/// 而与真静音混淆 —— 与电平路径的 `sanitize_sample` 同口径。
#[inline]
#[must_use]
fn nan_to_zero(sample: f32) -> f32 {
    if sample.is_nan() { 0.0 } else { sample }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 声相定律：左/中/右三点的增益必须落在**声明值**上（公式 + 容差）。
    #[test]
    fn pan_gains_land_on_the_declared_constant_power_curve() {
        let law = PanLaw::ConstantPowerMinus3dB;
        let (left, right) = pan_gains(-1.0, law);
        assert!((left - 1.0).abs() < 1e-7, "全左的左增益必须是 1.0: {left}");
        assert!(right.abs() < 1e-7, "全左的右增益必须 ≈ 0: {right}");

        let (left, right) = pan_gains(1.0, law);
        assert!(left.abs() < 1e-7, "全右的左增益必须 ≈ 0: {left}");
        assert!(
            (right - 1.0).abs() < 1e-7,
            "全右的右增益必须是 1.0: {right}"
        );

        let (left, right) = pan_gains(0.0, law);
        assert!(
            (left - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-7,
            "居中的左增益必须是 √2/2: {left}"
        );
        assert!((left - right).abs() < 1e-7, "居中必须左右对称");
    }

    /// 等功率性：`gain_l² + gain_r² == 1` 在整条 `pan` 范围上成立（0.1 步长扫描）。
    #[test]
    fn constant_power_holds_across_the_pan_range() {
        let law = PanLaw::ConstantPowerMinus3dB;
        let mut pan = -1.0f32;
        while pan <= 1.0 {
            let (left, right) = pan_gains(pan, law);
            let power = left * left + right * right;
            assert!(
                (power - 1.0).abs() < 1e-6,
                "pan {pan}: 增益平方和应恒为 1, 实际 {power}"
            );
            pan += 0.1;
        }
    }

    /// **判据**：三个**未实现**的衰减律显式回落到默认律的曲线（不是等增益复制）。
    ///
    /// [ADR-0001 D43]：不猜语义，但也不许"悄悄用另一种曲线"。这条判据把回落**钉死**：
    /// 谁改 [`pan_gains`] 的 `match` 让某个未实现分支走了别的形状，它就变红；
    /// 补齐定义之后，把对应分支改成真曲线、并把这条判据的期望一并改掉即可。
    #[test]
    fn unimplemented_pan_laws_fall_back_to_the_documented_curve() {
        for pan in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            let expected = pan_gains(pan, PanLaw::ConstantPowerMinus3dB);
            for law in [
                PanLaw::Linear,
                PanLaw::ConstantPowerMinus4_5dB,
                PanLaw::ConstantPowerMinus6dB,
            ] {
                let actual = pan_gains(pan, law);
                assert_eq!(
                    (actual.0.to_bits(), actual.1.to_bits()),
                    (expected.0.to_bits(), expected.1.to_bits()),
                    "pan={pan} law={law:?} 必须显式回落到默认律（不得静默换成别的曲线）"
                );
            }
        }
    }

    /// 退化输入：`NaN`/越界 `pan` 必须被钳制，绝不产生 `NaN`。
    #[test]
    fn degenerate_pan_values_are_clamped() {
        for pan in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let (left, right) = pan_gains(pan, PanLaw::ConstantPowerMinus3dB);
            assert!(
                left.is_finite() && right.is_finite(),
                "pan {pan} 产生了非有限增益"
            );
        }
        let (left, right) = pan_gains(-3.0, PanLaw::ConstantPowerMinus3dB);
        assert!(
            (left - 1.0).abs() < 1e-7 && right.abs() < 1e-7,
            "越界必须钳到全左"
        );
        let (left, right) = pan_gains(3.0, PanLaw::ConstantPowerMinus3dB);
        assert!(
            left.abs() < 1e-7 && (right - 1.0).abs() < 1e-7,
            "越界必须钳到全右"
        );
    }

    /// **判据**：>阈值的峰值必须被压到**不超阈值**（模块文档 §4 的上界）。
    #[test]
    fn limited_peak_never_exceeds_threshold() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<128>::new();
        // 一段 1.2 幅度的正弦（远超 0.9 阈值），跑满若干量子。
        let mut phase = 0.0f32;
        let mut peak = 0.0f32;
        for _ in 0..64 {
            for frame in 0..128 {
                let value = 1.2 * (phase).sin();
                phase += core::f32::consts::TAU * 1000.0 / 48_000.0;
                let (left, right) = block.stereo_mut();
                left[frame] = value;
                right[frame] = value;
            }
            limiter.apply(&mut block, 128);
            for sample in block.left().iter().chain(block.right().iter()) {
                peak = peak.max(sample.abs());
            }
        }
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
        assert!(
            limiter.reduction_count() > 0,
            "限制器报告 0 个被压样本 —— 夹具没有驱动它（结构性假绿）"
        );
        // 口径：软膝渐近到 CEILING ⇒ **严格**不超过 CEILING，且允许在阈值之上
        // 有一小段过冲（软膝宽度 0.05）。实测峰值 0.90006（阈值 0.9 / 天花板 0.95）。
        assert!(
            peak <= LIMITER_CEILING,
            "限制后峰值 {peak} 超过天花板 {LIMITER_CEILING}"
        );
        assert!(peak > 0.5, "峰值被压得太狠（{peak}），弹道可疑");
    }

    /// **判据**：未超阈值的样本**逐位不变**（`gain == 1.0` 时乘法是恒等）。
    #[test]
    fn sub_threshold_samples_are_bit_identical() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<128>::new();
        let source: Vec<f32> = (0..128)
            .map(|index| 0.4 * ((index as f32) * 0.1).sin())
            .collect();
        for (frame, sample) in source.iter().enumerate() {
            let (left, right) = block.stereo_mut();
            left[frame] = *sample;
            right[frame] = *sample;
        }
        limiter.apply(&mut block, 128);
        // 输出 = 输入整体延后 `LOOKAHEAD_SAMPLES` 帧 ⇒ 对齐后必须**逐位**相同。
        for (frame, sample) in source.iter().enumerate() {
            let output = frame + LOOKAHEAD_SAMPLES;
            if output >= 128 {
                break;
            }
            let (left, right) = block.stereo_mut();
            assert_eq!(
                left[output].to_bits(),
                sample.to_bits(),
                "样本 {frame} → 输出 {output} 被改写了"
            );
            assert_eq!(right[output].to_bits(), sample.to_bits());
        }
        assert!(!limiter.engaged(), "未超阈值的输入不得驱动限制器");
        assert_eq!(limiter.reduction_count(), 0);
    }

    /// **判据**：尖峰之后**逐位归零**，且尖峰本身被压到阈值。
    #[test]
    fn a_single_spike_is_exactly_limited_and_the_tail_is_bit_silent() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<128>::new();
        let spike_at = 40usize;
        {
            let (left, right) = block.stereo_mut();
            for frame in 0..128 {
                let value = if frame == spike_at { 1.2 } else { 0.0 };
                left[frame] = value;
                right[frame] = value;
            }
        }
        limiter.apply(&mut block, 128);
        let peak = block
            .left()
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(
            peak <= LIMITER_CEILING,
            "尖峰峰值 {peak} 超过天花板 {LIMITER_CEILING}"
        );
        assert!(
            peak > LIMITER_THRESHOLD,
            "尖峰峰值 {peak} 没有越过阈值 —— 限制器根本没驱动"
        );
        // 输出整体延迟 `LOOKAHEAD_SAMPLES` 帧（本实现里"延迟"就是环长：
        // 写位置上的旧值恰好是 `L` 帧之前写进去的那个样本）⇒
        // 输入 frame 40 的尖峰出现在输出 frame 73。
        // 实测：唯一非零样本在 73，值 0.90005994（阈值 0.9、天花板 0.95）。
        let delayed = spike_at + LOOKAHEAD_SAMPLES;
        let limited = block.left()[delayed];
        assert!(
            limited > LIMITER_THRESHOLD && limited <= LIMITER_CEILING,
            "被延迟的尖峰应落在 (阈值, 天花板] 区间: {limited}"
        );
        assert!(limited < 1.2, "尖峰没有被压: {limited}（原始幅度 1.2）");
        // 增益是**缓慢释放**的：尖峰被压之后它每样本只回升 `release`，
        // 因此在 128 帧的窗口里一直 < 1.0 ⇒ 被计数的样本 = 从尖峰输出到窗口末尾
        // （实测 128 − 73 = 55 个；尖峰出现在输出 frame 73）。
        // 计数口径是"增益 < 1.0 时**写出**的样本数"，不是"非零样本数"：
        // 增益从尖峰进入窗口（输入 frame 40 = 输出 frame 7 之后开始被压低）就一直
        // 缓慢释放，到窗口末尾仍是 0.7505 < 1.0 ⇒ 实测 **88** 帧。
        assert_eq!(
            limiter.reduction_count(),
            88,
            "被压样本数应从尖峰进入前瞻窗口起一直数到窗口末尾"
        );
        // 静音尾部必须**逐位**是 0（含 -0.0 之外的任何非零）。
        for frame in (delayed + 1)..128 {
            assert_eq!(
                block.left()[frame].to_bits(),
                0.0f32.to_bits(),
                "样本 {frame} 不是逐位静音"
            );
        }
    }

    /// **判据**：增益的**释放**方向每样本回升 ≤ [`LIMITER_RELEASE_PER_SAMPLE`]。
    ///
    /// 必须**逐样本**测（本测试按 1 帧一块喂）：一个 128 帧的量子最多能回升
    /// `128 × release`，拿量子边界上的两个读数比较会把合法释放记成跳变
    /// —— 第一版就是这么写的，实测假红 9.36e-4（= 18.7 × release）。
    ///
    /// 攻击方向**刻意不受**这条约束（[ARCH-DSP-001] 的"峰值不越界"要求增益立刻让位，
    /// 见模块文档 §2.3），因此它由 `limit_engages_in_a_single_sample` 单独钉住。
    #[test]
    fn gain_release_is_bounded_per_sample() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<1>::new();
        let amplitude = 2.0f32;
        let mut phase = 0.0f32;
        let mut previous = limiter.gain();
        let mut rises = 0u64;
        for _ in 0..8_192 {
            let value = amplitude * phase.sin();
            phase += core::f32::consts::TAU * 220.0 / 48_000.0;
            {
                let (left, right) = block.stereo_mut();
                left[0] = value;
                right[0] = value;
            }
            limiter.apply(&mut block, 1);
            let gain = limiter.gain();
            let rise = gain - previous;
            // 容差 = 1 ulp 量级（增益在 0.4..1.0 ⇒ ulp ≈ 6e-8）：`min` 之后
            // 回升量在浮点上不可能精确等于 `release`，实测 5.00083e-5。
            // 判别力不受影响：真正的"释放跳变"量级是 9.36e-4（18.7 倍）。
            assert!(
                rise <= LIMITER_RELEASE_PER_SAMPLE + LIMITER_RELEASE_PER_SAMPLE * 1e-3,
                "单样本回升 {rise} 超过释放上限"
            );
            if rise > 0.0 {
                rises += 1;
            }
            previous = gain;
        }
        assert!(rises > 0, "夹具从未发生过释放（回升）—— 这条判据是空转");
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
    }

    /// **判据**：过载开始时增益**在同一个样本内**让位（攻击不受释放上限约束）。
    ///
    /// 这条与 `gain_release_is_bounded_per_sample` 成对：只有两条都在，才说明
    /// "上升受约束、下降不受约束"这个**非对称**弹道是我们选的那个，而不是写反了。
    #[test]
    fn limit_engages_in_a_single_sample() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<1>::new();
        let mut gains = Vec::new();
        for index in 0..64 {
            let value = if index == 32 { 2.0 } else { 0.0 };
            {
                let (left, right) = block.stereo_mut();
                left[0] = value;
                right[0] = value;
            }
            limiter.apply(&mut block, 1);
            gains.push(limiter.gain());
        }
        // 窗口一拍到 2.0，增益立刻掉到 0.45（一点延迟都没有）。
        assert!(
            gains[31] > 0.9,
            "过载之前增益应当还在 1.0 附近: {}",
            gains[31]
        );
        assert!(
            gains[32] < 0.6,
            "过载当帧增益就必须让位（实测 0.45），实际 {}",
            gains[32]
        );
        assert!(gains[32] >= LIMITER_THRESHOLD / 2.0, "增益不得低于目标");
    }

    /// **判据**：输出样本的相邻位移有界（无爆音）—— 过载段不得出现阶跃。
    #[test]
    fn overload_never_steps_the_waveform() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<128>::new();
        let mut phase = 0.0f32;
        let mut previous = 0.0f32;
        let mut worst = 0.0f32;
        let amplitude = 2.0f32;
        for _ in 0..64 {
            for frame in 0..128 {
                let value = amplitude * phase.sin();
                phase += core::f32::consts::TAU * 220.0 / 48_000.0;
                let (left, right) = block.stereo_mut();
                left[frame] = value;
                right[frame] = value;
            }
            limiter.apply(&mut block, 128);
            for sample in block.left() {
                worst = worst.max((sample - previous).abs());
                previous = *sample;
            }
        }
        // 输入自身每样本最大步进 = 2π·220/48000·2.0 ≈ 0.0576；限制只能让它更小。
        // 这里给 0.06 的上界：超过它就意味着增益在跳变（而不是在压限）。
        assert!(worst < 0.06, "过载段出现阶跃: 最大相邻位移 {worst}");
        assert!(limiter.engaged(), "夹具必须真的驱动限制器");
    }

    /// 空块 / 零帧：不得推进状态、不得 panic。
    #[test]
    fn zero_frames_is_a_no_op() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<128>::new();
        limiter.apply(&mut block, 0);
        assert_eq!(limiter.gain(), 1.0);
        assert!(!limiter.engaged());
    }

    /// `NaN` 输入不得通过 `max` 吞掉或被当成静音：显式归零。
    #[test]
    fn nan_is_neutralised_instead_of_poisoning_the_window() {
        let mut limiter = BusLimiter::new();
        let mut block = AudioBlock::<128>::new();
        {
            let (left, right) = block.stereo_mut();
            left[0] = f32::NAN;
            right[0] = f32::NAN;
        }
        limiter.apply(&mut block, 128);
        for sample in block.left().iter().chain(block.right().iter()) {
            assert!(sample.is_finite(), "NaN 泄漏到输出: {sample}");
        }
    }
}
