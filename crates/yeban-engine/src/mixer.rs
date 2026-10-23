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
//!      BusLimiter::process_stereo(...)            ← 前瞻 + 立即攻击/平滑释放
//!         │                                        （实现住在 yeban_dsp::limiter）
//!         ▼
//!      母线电平（限制**之后**）→ cpal / NullBackend
//! ```
//!
//! ## 1. 声相定律的口径
//!
//! 默认律 [`PanLaw::ConstantPowerMinus3dB`] 的**曲线**是等功率
//! `gain_l = cos θ`、`gain_r = sin θ`、`θ = (pan + 1) · π/4`：
//!
//! | `pan` | `gain_l` | `gain_r` | 备注 |
//! | :--- | :--- | :--- | :--- |
//! | −1.0 | 1.0 | 0.0 | 全左：右声道**逐位**静音 |
//! | 0.0 | √2/2 ≈ 0.70710678 | √2/2 | **曲线**在中心的值（不等于**落点**，见下） |
//! | +1.0 | 0.0 | 1.0 | 全右 |
//!
//! ⚠ **曲线 ≠ 落点**（任务单 `#300` 把这条写成字）：`pan == 0` 的**落点**由
//! [`centered_pan_gains`] 统一成 `(1.0, 1.0)` —— **源侧与输出级、监听与导出同一条**。
//! 于是 `pan = 0` 不衰减、`pan = ±ε` 走曲线 ⇒ 中心有一个 **3.01 dB** 的台阶；
//! 这是离线渲染一直以来的既有规则（`yeban_render::automation::centered_pan_gains`），
//! `#300` 把监听对齐过去（导出是成品、有钉死摘要），⛔ 不是反过来让导出变小声。
//! [`pan_gains`] 仍是**曲线本身**：`pan != 0` 的每一档、以及 MCP／离线的曲线端口
//! 都读它；`#299` 量到的"监听比导出低 3.0103 dB"是修前**源侧落点**错用了曲线中心值。
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
//! ## 2. 母线限制器：**实现已上移到 `yeban-dsp`**（本模块只 re-export）
//!
//! `BusLimiter` 的算法（前瞻窗口、立即攻击、速率上限释放、软膝天花板）**一行未改**，
//! 现在住在 [`yeban_dsp::limiter`]（`crates/yeban-dsp/src/limiter.rs`）。
//! 本模块只做两件事：
//!
//! ```text
//! pub use yeban_dsp::limiter::Limiter as BusLimiter;   // 类型（含方法与 Default）
//! pub use yeban_dsp::limiter::{LIMITER_CEILING, ...};  // 五个规范常量
//! ```
//!
//! 上移的裁决与方式（**逐条可查**）：
//!
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:107` 把限制器算法的目的地
//!   写成 `crates/yeban-dsp/src/limiter.rs`；
//! - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md:465`（D44(c)）写明
//!   上移之后 `yeban-engine` 侧**保留 `pub use` re-export**、**不得留第二份实现**
//!   —— 与 `level.rs` 上移 `yeban_dsp::meter` 是**同一条处理方式**；
//! - 因此本模块**没有**限制器的类型定义、没有它的固有实现块、没有两个私有辅助
//!   函数（软膝映射与 `NaN` 归零）、也没有那五个规范常量。这件事由
//!   `tests::engine_mixer_module_has_no_second_limiter_implementation`
//!   （`include_str!("mixer.rs")` 源码级检查）与
//!   `tests::engine_bus_limiter_is_literally_the_dsp_limiter`（类型/函数/常量同一性）
//!   两条判据机械钉住。
//!
//! ### 2.1 调用形状的变化（唯一的一处）
//!
//! 原 `BusLimiter::apply(&mut AudioBlock<FRAMES>, frames)` 的签名里含**本 crate 的块类型**
//! ⇒ 它不可能跟着实现一起上移（`yeban-dsp` 不认识 `AudioBlock`）。上移时入口改成
//! `process_stereo(&mut [f32], &mut [f32]) -> usize`（与 `yeban_dsp::compressor`、
//! `yeban_dsp::channel_strip` 的既有风格一致），语义**不变**：
//!
//! - `frames == 0` ⇒ 空操作（不推进状态）；
//! - `frames` 超界 ⇒ 按"两条切片长度的较小者"钳制（原实现按块容量钳制）。
//!
//! `crates/yeban-engine/src/rt.rs` 的调用点因此写成
//! `limiter.process_stereo(&mut left[..frames], &mut right[..frames])`；
//! 其余调用点（判据）用同样的方式把本量子的帧数切出来。
//!
//! ### 2.2 延迟的归属（**已回填**，不再是待办）
//!
//! 限制器的 [`LOOKAHEAD_SAMPLES`] = **33 帧**延迟**只在限制器开启时**按
//! [ADR-0001 D44(b)] 回填：`EngineSnapshot::from_project_with_limiter(.., true)`
//! 在**构造期**用 [`crate::graph::LatencyTable::add`] 把它登记到 `master` 的自身延迟上。
//!
//! ⭐ **任务单 `#302` 起默认关**：`from_project` 不跑限制器、不回填 ⇒
//! [`crate::graph::PdcPlan::output_latency`] 等于
//! [`crate::graph::PdcPlan::total_latency`]，监听与导出在样本层对齐。需要限制器的
//! 调用方显式用 `from_project_with_limiter(.., true)`。
//!
//! 它接在**总线求和之后**（`rt.rs` 的步骤 3b），因此 PDC 的对齐基准
//! [`crate::graph::PdcPlan::total_latency`]（= `L(master)`）**不含**这 33 帧 ——
//! 支路的 `D(v)` 一位没变 ⇒ **开启前后渲染只差 33 帧时间**（判据：
//! `tests/pdc_mix_path.rs`）。

// 母线限制器的唯一实现住在 yeban-dsp；这里只做转发（类型 + 五个规范常量）。
// ⚠ 名字保留 `BusLimiter`：它是引擎内全部调用点与判据使用的公共面，改名会让
// "搬家"对调用方可见（与 `level.rs` 保留模块名同理）。
pub use yeban_dsp::limiter::{
    LIMITER_CEILING, LIMITER_LATENCY_FRAMES, LIMITER_RELEASE_PER_SAMPLE, LIMITER_THRESHOLD,
    LOOKAHEAD_SAMPLES, Limiter as BusLimiter,
};

/// 母线限制器给**引擎输出**引入的固定延迟（**帧**，与采样率无关）。
///
/// 等于 [`LOOKAHEAD_SAMPLES`] = **33 帧** @48 kHz ≈ **0.688 ms**：前瞻环长就是写位置
/// 上的那个旧值"恰好是 33 帧前写进去的样本"（[`yeban_dsp::limiter`] 的不变量）。
/// 单位是帧而不是时间：环是定长数组，采样率变了帧数不变。
///
/// [ADR-0001 D44(b)] 要求"引擎链路上每一段延迟都可被 PDC 看见" ⇒
/// `EngineSnapshot::from_project_with_limiter(.., true)` 在**构造期**把它加到
/// `master` 节点的自身延迟上；默认档（`from_project`，任务单 `#302`）**不跑
/// 限制器、不回填**。因为它发生在**总线求和之后**，它只进
/// [`crate::graph::PdcPlan::output_latency`]，**不**进
/// [`crate::graph::PdcPlan::total_latency`]、也**不**产生任何 `D(v)`。
pub const BUS_LIMITER_LATENCY_FRAMES: u32 = LOOKAHEAD_SAMPLES as u32;

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

/// 声相增益**曲线**：`(左, 右)`，由 `pan ∈ [-1, 1]` 与衰减律给出。
///
/// ⚠ 本函数是**曲线**，不是**落点**：`pan == 0` 时它给 `(√2/2, √2/2)`，而源侧与
/// 输出级的落点都经 [`centered_pan_gains`] ⇒ 中心恒等 `(1.0, 1.0)`（任务单 `#300`）。
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
        PanLaw::ConstantPowerMinus3dB => (libm::cosf(angle), libm::sinf(angle)),
        // ⚠ 未实现：缺"居中给 (0.5, 0.5)（−6.02 dB）还是 (1, 1)（不衰减）"的定义。
        // 回落到**默认律的曲线**（而不是 line 之前的等增益复制 —— 那会涨 3 dB）。
        PanLaw::Linear => (libm::cosf(angle), libm::sinf(angle)),
        // ⚠ 未实现：缺"−4.5 dB 指居中额外衰减，还是另一族曲线"的定义。
        PanLaw::ConstantPowerMinus4_5dB => (libm::cosf(angle), libm::sinf(angle)),
        // ⚠ 未实现：同上（若指居中衰减 ⇒ 居中 `(0.5, 0.5)`）。
        PanLaw::ConstantPowerMinus6dB => (libm::cosf(angle), libm::sinf(angle)),
    }
}

/// **声相位置的恒等规则**：`pan == 0`（或非有限）⇒ `(1.0, 1.0)`，否则走 [`pan_gains`]。
///
/// ## 谁用这条规则
///
/// **源侧与输出级共用同一条**（任务单 `#300` 起）：
///
/// - **逐轨源**：`crate::snapshot::TrackParams::pan_gains`（引擎监听）＋
///   `yeban_app::audio_clip::track_pan_gains`（CLI 离线）＋ MCP 的
///   `centered_pan_gains(source_pan, pan_gains)`；
/// - **总线／主总线输出级**：`crate::snapshot::BusOutputStage` 与 `master_pan_left/right`，
///   离线对应 `yeban_render::render::OutputStage::with_pan`（同一函数）。
///
/// ## 为什么中心必须恒等
///
/// `pan == 0` 是绝大多数工程的**中性**状态（也是"没有动过声相"的同义词）：
/// 若这里给 `(√2/2, √2/2)`，居中轨会无端低 3.01 dB，而离线导出（成品、有钉死摘要）
/// 在中心是 `(1.0, 1.0)` ⇒ 监听与导出之间出现一条**只在中性声相上**的 −3.0103 dB 差
/// （`#299` 逐字量到：监听 `0.548988640` / 导出 `0.776387155`）。
/// `#300` 的裁决是**监听对齐导出**：本函数现在是源侧的唯一落点。
///
/// ⚠ **如实登记**：这条规则在 `pan = 0` 与 `pan = ±ε` 之间有 3.01 dB 的台阶
/// （`#300` 未改，属离线的既有口径）；凡 `pan != 0` 的轨读数与修前**逐位相同**。
///
/// **必须只在构造期调用**（`cos`/`sin` 属超越函数类 [ADR-0001 D32]）。
#[must_use]
pub fn centered_pan_gains(pan: f32, law: PanLaw) -> (f32, f32) {
    if !pan.is_finite() || pan == 0.0 {
        (1.0, 1.0)
    } else {
        pan_gains(pan, law)
    }
}

/// 盲听 A/B 的等功率交叉淡化时长（毫秒）—— [ARCH-RT-005] / [RSK-30]。
///
/// 规范值 **30 ms**；48 kHz 下折合 [`crossfade_len_frames`] = **1440** 采样点。
pub const AUDITION_CROSSFADE_MS: f32 = 30.0;

/// 盲听预滚缓冲（采样点）—— [ARCH-RT-005]。
///
/// 规范值 **2048**（42.7 ms @ 48 kHz），**足以覆盖** 30 ms 淡化所需的 1440 采样点；
/// 这一覆盖关系由 `tests::preroll_covers_the_crossfade_at_any_supported_rate` 钉住。
pub const AUDITION_PREROLL_SAMPLES: usize = 2048;

/// 30 ms 淡化在给定采样率下的**帧数**。
///
/// ⚠ **不写死 1440**：1440 只是 48 kHz 的取值；本函数按
/// `采样率 × 时长` 换算 ⇒ 96 kHz 得 2880。⛔ 任何把 1440 当常量的做法都会
/// 让 96 kHz 的淡化缩短一半。
///
/// 非有限或非正的采样率按 **1 帧**兜底（⛔ 不返回 0 —— 0 会让淡化**从不下拍**）。
#[must_use]
pub fn crossfade_len_frames(sample_rate: f32) -> usize {
    let frames = (sample_rate * AUDITION_CROSSFADE_MS / 1000.0).round();
    if frames.is_finite() && frames >= 1.0 {
        frames as usize
    } else {
        1
    }
}

/// [ARCH-RT-005] 片 3：**按采样率放大后的**预滚帧数 —— 调用侧必须用它，
/// ⛔ **不要**直接用 [`AUDITION_PREROLL_SAMPLES`]。
///
/// ## 为什么需要这个函数
///
/// [`AUDITION_PREROLL_SAMPLES`] = **2048** 是**按 48 kHz 定**的：它覆盖 48 kHz 的
/// 30 ms 淡化（1440）绰绰有余。但 96 kHz 的 30 ms 淡化需要
/// [`crossfade_len_frames`]`(96_000.0)` = **2880 > 2048** ⇒ 在那个采样率下，
/// 用裸常量会让第二分支的**准备时间短于淡化本身**。
/// 这条边界不是猜的：`tests::preroll_covers_the_crossfade_at_any_supported_rate`
/// 里已经登记了它，并写着"调用侧须放大预滚" —— 本函数就是那个调用侧口径。
///
/// ## 口径
///
/// 1. **至少覆盖一整段淡化**（`crossfade_len_frames`）；
/// 2. **不低于**规范下限 [`AUDITION_PREROLL_SAMPLES`]（48 kHz 下结果不变 ⇒ 零回退）；
/// 3. 再**向上取到 2 的幂**：预滚环按位与取模（`& (len − 1)`），⛔ 回调里不做除法。
///
/// ⚠ 非有限 / 非正的采样率由 [`crossfade_len_frames`] 兜到 1，再被第 2 条抬到 2048
/// ⇒ 结果永远 ≥ 2048 且是 2 的幂。
#[must_use]
pub fn audition_preroll_frames(sample_rate: f32) -> usize {
    crossfade_len_frames(sample_rate)
        .max(AUDITION_PREROLL_SAMPLES)
        .next_power_of_two()
}

/// 等功率交叉淡化的增益对 `(淡出, 淡入)`，`t ∈ [0, 1]`。
///
/// 曲线是**精确**的正弦/余弦：`淡出 = cos(t·π/2)`、`淡入 = sin(t·π/2)`，于是
/// **`淡出² + 淡入² = 1`**（[RSK-30]：⛔ 不用线性淡化 —— 那在中点掉到
/// `0.5² + 0.5² = 0.5`，即 **−3 dB 的功率凹陷**）。
///
/// 越界 `t` 先钳到 `[0, 1]`；非有限按 `t = 0`（起淡）处理。本函数**无堆分配**、
/// 无锁、无 I/O，可在实时回调内逐样本调用。
#[must_use]
pub fn crossfade_gains(t: f32) -> (f32, f32) {
    let clamped = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let angle = clamped * core::f32::consts::FRAC_PI_2;
    (libm::cosf(angle), libm::sinf(angle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [ARCH-RT-005] / [RSK-30]：淡化曲线必须是**等功率**的，⛔ 不是线性。
    ///
    /// 读数一（等功率）：145 个位置上 `淡出² + 淡入²` 与 1 的偏差 ≤ 1e-5。
    /// 读数二（反线性）：中点 `t = 0.5` 的增益必须 ≈ **0.7071**，并且与**线性**值 0.5
    /// 相差 ≥ **0.2** ⇒ ⭐ 这一条才是"曲线不是线性"的证据（⛔ 只测等式测不出线性凹陷）。
    #[test]
    fn audition_crossfade_is_equal_power_not_linear() {
        let mut worst = 0.0f32;
        for index in 0..=144 {
            let t = index as f32 / 144.0;
            let (out_gain, in_gain) = crossfade_gains(t);
            let power = out_gain * out_gain + in_gain * in_gain;
            worst = worst.max((power - 1.0).abs());
        }
        println!("[mixer] 等功率最大偏差 = {worst:e}");
        assert!(worst <= 1e-5, "等功率偏差过大：{worst}");

        let (out_mid, in_mid) = crossfade_gains(0.5);
        let linear = 0.5f32;
        println!("[mixer] 中点 淡出={out_mid} 淡入={in_mid}；线性值={linear}");
        assert!(
            (out_mid - core::f32::consts::FRAC_1_SQRT_2).abs() <= 1e-5,
            "中点淡出必须 ≈ 1/√2 = 0.7071，实得 {out_mid}"
        );
        assert!(
            (out_mid - linear).abs() >= 0.2,
            "⭐ 反线性证据：中点必须明显偏离线性 0.5（实得 {out_mid}）⇒ 否则说明用了线性淡化"
        );
        assert!(
            (in_mid - out_mid).abs() <= 1e-6,
            "中点两侧必须对称（实得 {in_mid}）"
        );
    }

    /// [ARCH-RT-005]：端点与单调性 —— `t = 0` 全淡出、`t = 1` 全淡入，
    /// 且淡出**不增**、淡入**不减**（逐样本无回头）。
    #[test]
    fn audition_crossfade_endpoints_and_monotonicity() {
        assert_eq!(crossfade_gains(0.0), (1.0, 0.0), "t=0：必须全淡出");
        let (out_end, in_end) = crossfade_gains(1.0);
        assert!(
            out_end.abs() <= 1e-7 && (in_end - 1.0).abs() <= 1e-7,
            "t=1：必须全淡入"
        );
        // 越界与非有限：钳制/兜底（⛔ 不得产生 NaN 或反向增益）
        assert_eq!(crossfade_gains(-1.0), (1.0, 0.0), "负 t 钳到 0");
        assert_eq!(crossfade_gains(2.0), crossfade_gains(1.0), "t>1 钳到 1");
        assert_eq!(crossfade_gains(f32::NAN), (1.0, 0.0), "NaN 按起淡处理");
        let mut previous_out = f32::INFINITY;
        let mut previous_in = f32::NEG_INFINITY;
        for index in 0..=1000 {
            let (out_gain, in_gain) = crossfade_gains(index as f32 / 1000.0);
            assert!(
                out_gain <= previous_out + 1e-6,
                "淡出必须单调不增（第 {index} 点）"
            );
            assert!(
                in_gain >= previous_in - 1e-6,
                "淡入必须单调不减（第 {index} 点）"
            );
            previous_out = out_gain;
            previous_in = in_gain;
        }
    }

    /// [ARCH-RT-005]：**30 ms 是按时间定义**的 ⇒ 长度必须随采样率换算（⛔ 不写死 1440）。
    #[test]
    fn audition_crossfade_length_is_time_derived() {
        assert_eq!(crossfade_len_frames(48_000.0), 1440, "48 kHz ⇒ 规范值 1440");
        assert_eq!(
            crossfade_len_frames(96_000.0),
            2880,
            "96 kHz ⇒ 2880（⭐ 证明未写死）"
        );
        assert_eq!(
            crossfade_len_frames(44_100.0),
            1323,
            "44.1 kHz ⇒ 1323（30 ms 四舍五入）"
        );
        // 退化输入兜底：⛔ 不得为 0（0 会让淡化永不下拍）
        assert_eq!(crossfade_len_frames(0.0), 1, "0 采样率兜底为 1");
        assert_eq!(crossfade_len_frames(f32::NAN), 1, "NaN 采样率兜底为 1");
        println!(
            "[mixer] 淡化帧数: 48k={} 96k={} 44.1k={}",
            crossfade_len_frames(48_000.0),
            crossfade_len_frames(96_000.0),
            crossfade_len_frames(44_100.0)
        );
    }

    /// [ARCH-RT-005]：预滚缓冲 **2048** 必须**覆盖** 30 ms 淡化所需的帧数（规范说"充分覆盖"）。
    ///
    /// ⭐ 这是**不等式**判据（⛔ 不是"计数 ≥ 地板"那类反向指标）：左边是规范常量，
    /// 右边是按时长算出的**实际需要** ⇒ 两者都会变，等式/不等式才有信息量。
    #[test]
    fn preroll_covers_the_crossfade_at_any_supported_rate() {
        assert_eq!(AUDITION_PREROLL_SAMPLES, 2048, "规范值 2048");
        let needed_48k = crossfade_len_frames(48_000.0);
        let margin = AUDITION_PREROLL_SAMPLES.saturating_sub(needed_48k);
        println!(
            "[mixer] 预滚余量（48k）= {AUDITION_PREROLL_SAMPLES} − {needed_48k} = {margin} 采样点"
        );
        assert!(
            AUDITION_PREROLL_SAMPLES >= needed_48k,
            "预滚必须覆盖 48 kHz 的 30 ms 淡化（需要 {needed_48k}）"
        );
        assert!(margin > 0, "⭐ 余量必须报数值且为正（实得 {margin}）");
        // 96 kHz 下 30 ms 需要 2880 > 2048 ⇒ ⭐ 这正是"2048 是按 48 kHz 定的"的证据，
        // 也是本项**未在 96 kHz 下声称覆盖**的诚实边界（登记为下一件待做的调用侧约束）。
        let needed_96k = crossfade_len_frames(96_000.0);
        println!(
            "[mixer] ⚠ 96 kHz 需要 {needed_96k} > 预滚 {AUDITION_PREROLL_SAMPLES} ⇒ 调用侧须放大预滚"
        );
        assert!(
            needed_96k > AUDITION_PREROLL_SAMPLES,
            "96 kHz 的边界必须成立，否则上面那行注释失真"
        );
    }

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

    /// ⭐ 输出级的中心恒等：`pan == 0` ⇒ **逐位** `(1, 1)`；非零 ⇒ 与 `pan_gains` 逐位相同。
    ///
    /// 这条判据同时是"中性总线/中性主总线不衰减"的**构造期**证据：
    /// 实时侧只看这对增益的 `!= 1.0` 分支，因此 `(1,1)` 就意味着一个样本都不碰。
    #[test]
    fn centered_pan_gains_is_exactly_identity_at_the_centre_and_the_curve_elsewhere() {
        let law = PanLaw::ConstantPowerMinus3dB;
        for pan in [0.0f32, -0.0] {
            let (left, right) = centered_pan_gains(pan, law);
            assert_eq!(
                (left.to_bits(), right.to_bits()),
                (1.0f32.to_bits(), 1.0f32.to_bits()),
                "pan={pan}（含 -0.0）必须逐位恒等 (1,1)"
            );
        }
        for pan in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let (left, right) = centered_pan_gains(pan, law);
            assert_eq!(
                (left.to_bits(), right.to_bits()),
                (1.0f32.to_bits(), 1.0f32.to_bits()),
                "非有限 pan={pan} 必须退回恒等 (1,1)"
            );
        }
        for pan in [-1.0f32, -0.5, 0.25, 0.5, 1.0] {
            let centered = centered_pan_gains(pan, law);
            let raw = pan_gains(pan, law);
            assert_eq!(
                (centered.0.to_bits(), centered.1.to_bits()),
                (raw.0.to_bits(), raw.1.to_bits()),
                "非零 pan={pan} 必须与 pan_gains 逐位相同（同一条曲线）"
            );
        }
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

    // -----------------------------------------------------------------------
    // 上移的机械证据（类型/函数/常量同一性 + 无第二份实现）
    // -----------------------------------------------------------------------

    /// 判据：engine 的 `BusLimiter` 与 dsp 的 `Limiter` **是同一个东西**（不是同构复制品）。
    ///
    /// 这是编译期判据（类型赋值）+ 运行期判据（函数地址相等 + 常量位模式相等）。
    /// 注入：在 engine 里偷偷加回一份自己的限制器类型或 `soft_knee`
    /// ⇒ 类型赋值与 `fn_addr_eq` 双双编译失败/变红。
    #[test]
    fn engine_bus_limiter_is_literally_the_dsp_limiter() {
        // 类型同一性（编译期）：dsp 类型的变量可以直接由 engine 路径构造。
        let via_engine: yeban_dsp::limiter::Limiter = BusLimiter::new();
        let via_dsp: yeban_dsp::limiter::Limiter = yeban_dsp::limiter::Limiter::new();
        assert_eq!(via_engine.gain().to_bits(), via_dsp.gain().to_bits());

        // 方法同一性（地址相等）：这些**必须**是同一个函数项，而不是两份同构实现。
        let engine_process: fn(&mut BusLimiter, &mut [f32], &mut [f32]) -> usize =
            BusLimiter::process_stereo;
        let dsp_process: fn(&mut yeban_dsp::limiter::Limiter, &mut [f32], &mut [f32]) -> usize =
            yeban_dsp::limiter::Limiter::process_stereo;
        assert!(core::ptr::fn_addr_eq(engine_process, dsp_process));

        let engine_new: fn() -> BusLimiter = BusLimiter::new;
        let dsp_new: fn() -> yeban_dsp::limiter::Limiter = yeban_dsp::limiter::Limiter::new;
        assert!(core::ptr::fn_addr_eq(engine_new, dsp_new));

        let engine_reset: fn(&mut BusLimiter) = BusLimiter::reset;
        let dsp_reset: fn(&mut yeban_dsp::limiter::Limiter) = yeban_dsp::limiter::Limiter::reset;
        assert!(core::ptr::fn_addr_eq(engine_reset, dsp_reset));

        // 常量同一性（逐位）：engine 路径读到的五个规范常量就是 dsp 的那五个。
        assert_eq!(LOOKAHEAD_SAMPLES, yeban_dsp::limiter::LOOKAHEAD_SAMPLES);
        assert_eq!(
            LIMITER_LATENCY_FRAMES,
            yeban_dsp::limiter::LIMITER_LATENCY_FRAMES
        );
        assert_eq!(
            LIMITER_THRESHOLD.to_bits(),
            yeban_dsp::limiter::LIMITER_THRESHOLD.to_bits()
        );
        assert_eq!(
            LIMITER_CEILING.to_bits(),
            yeban_dsp::limiter::LIMITER_CEILING.to_bits()
        );
        assert_eq!(
            LIMITER_RELEASE_PER_SAMPLE.to_bits(),
            yeban_dsp::limiter::LIMITER_RELEASE_PER_SAMPLE.to_bits()
        );

        // 母线限制器给**引擎输出**引入的延迟常量（[ADR-0001 D44(b)] 回填用的那个数）
        // 必须是 dsp 环长的 u32 投影 —— 不许在 engine 侧写死第二个 33。
        assert_eq!(
            usize::try_from(BUS_LIMITER_LATENCY_FRAMES).expect("33 能装进 usize"),
            yeban_dsp::limiter::LOOKAHEAD_SAMPLES,
            "BUS_LIMITER_LATENCY_FRAMES 必须等于 dsp 的前瞻环长"
        );
        assert_eq!(BUS_LIMITER_LATENCY_FRAMES, 33, "前瞻环长是 33 帧");
    }

    /// 判据：**engine 侧没有第二份限制器实现**（源码级机械检查）。
    ///
    /// `include_str!("mixer.rs")` 读到本文件自身的源码；断言其中不出现实现记号。
    /// 记号用 `concat!` 拼出来，避免判据自己的字面量命中自己。
    ///
    /// 注入：把 `yeban_dsp::limiter` 的实现复制进本文件（例如加回环形缓冲与
    /// `soft_knee`）⇒ 本判据立即变红。
    #[test]
    fn engine_mixer_module_has_no_second_limiter_implementation() {
        let source = include_str!("mixer.rs");
        let forbidden = [
            concat!("struct", " BusLimiter", " {"),
            concat!("impl", " BusLimiter"),
            concat!("struct", " Limiter", " {"),
            concat!("impl", " Limiter"),
            concat!("fn", " soft_knee("),
            concat!("fn", " nan_to_zero("),
            concat!("const", " LOOKAHEAD_SAMPLES"),
            concat!("const", " LIMITER_LATENCY_FRAMES"),
            concat!("const", " LIMITER_THRESHOLD"),
            concat!("const", " LIMITER_CEILING"),
            concat!("const", " LIMITER_RELEASE_PER_SAMPLE"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 mixer.rs 里出现了实现记号 `{needle}` —— 上移之后这里只允许 `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::limiter::"),
            "engine 的 mixer.rs 必须是 dsp 限制器的再导出"
        );
    }
    /// 判据（**R60：两条契约**）：声相律的**两个端点**，f32 实测读数。
    ///
    /// * 硬左（`pan = -1`）：右增益**恰好**是 `+0.0`（`sin(0) == 0`，逐位成立）；
    /// * 硬右（`pan = +1`）：左增益**不是 0** —— `cos(FRAC_PI_2 as f32)` 实测
    ///   `-4.371139e-8`（f32 的 `π/2` 不精确）⇒ 契约写成"有限、且落在 `[-1e-6, 0.0]`"，
    ///   ⛔ 不写"等于 0"（那是错的）。
    ///
    /// ⚠ **负号是真的**：控制侧把这两个增益发进引擎之前**必须 clamp 到 `>= 0`**，
    /// 否则引擎"有限且非负"的值规则会拒掉硬右的左增益（静默丢失）。
    /// `rt.rs` 有一条判据钉住"负值被计数拒绝"。
    #[test]
    fn pan_law_endpoints_are_exact_zero_on_one_side_and_tiny_negative_on_the_other() {
        let (left_l, left_r) = pan_gains(-1.0, PanLaw::default());
        assert_eq!(left_l, 1.0, "硬左的左增益是 1.0");
        assert!(
            left_r == 0.0,
            "硬左的右增益必须恰好是 +0.0（实得 {left_r:?}）"
        );
        let (right_l, right_r) = pan_gains(1.0, PanLaw::default());
        assert!(
            right_l.is_finite() && right_r.is_finite(),
            "两个端点都必须有限"
        );
        assert!(
            (-1e-6..=0.0).contains(&right_l),
            "硬右的左增益是「极小负值或 0」（实测 -4.371139e-8，实得 {right_l:?}）"
        );
        assert_eq!(right_r, 1.0, "硬右的右增益是 1.0");
        // R58：等号判据的 `assert_ne!` 落在同一个表达式上（两个端点必须真的不同）。
        assert_ne!(left_r, right_r, "硬左与硬右的右增益必须不同");
    }

    /// [ARCH-RT-005] 片 3：预滚**按采样率放大**，⛔ 不是裸常量。
    ///
    /// ⭐ **这条判据存在的理由**：`AUDITION_PREROLL_SAMPLES` = 2048 是按 48 kHz 定的，
    /// 而 96 kHz 的 30 ms 淡化需要 2880 ⇒ 裸常量在那个采样率下**不够**。
    #[test]
    fn audition_preroll_scales_with_the_crossfade_length() {
        // 48 kHz：等于规范下限 ⇒ 零回退。
        assert_eq!(
            audition_preroll_frames(48_000.0),
            AUDITION_PREROLL_SAMPLES,
            "48 kHz 下预滚必须仍等于规范下限（零回退）"
        );
        // 96 kHz：⭐ 本条的核心 —— 裸 2048 **不够**。
        let needed_96k = crossfade_len_frames(96_000.0);
        assert_eq!(needed_96k, 2880, "96 kHz 的 30 ms ⇒ 2880");
        assert!(
            AUDITION_PREROLL_SAMPLES < needed_96k,
            "裸常量在 96 kHz 必须是不够的（否则本函数没有存在的理由）"
        );
        let preroll_96k = audition_preroll_frames(96_000.0);
        assert!(
            preroll_96k >= needed_96k,
            "96 kHz 预滚必须覆盖淡化：需要 {needed_96k}，实得 {preroll_96k}"
        );
        assert_eq!(preroll_96k, 4096, "2880 向上取到 2 的幂 ⇒ 4096");
        // 全部支持档位：覆盖淡化 + 2 的幂（预滚环按位与取模）+ 不低于下限。
        for rate in [44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
            let frames = audition_preroll_frames(rate);
            assert!(
                frames >= crossfade_len_frames(rate),
                "{rate} ⇒ {frames} 必须覆盖淡化 {}",
                crossfade_len_frames(rate)
            );
            assert!(frames.is_power_of_two(), "{rate} ⇒ {frames} 必须是 2 的幂");
            assert!(
                frames >= AUDITION_PREROLL_SAMPLES,
                "{rate} ⇒ {frames} 不得低于规范下限"
            );
        }
        // 退化输入：兜到下限（⛔ 不是 0）。
        assert_eq!(audition_preroll_frames(0.0), AUDITION_PREROLL_SAMPLES);
        assert_eq!(audition_preroll_frames(f32::NAN), AUDITION_PREROLL_SAMPLES);
    }
}
