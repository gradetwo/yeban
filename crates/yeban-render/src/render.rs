//! 离线母带渲染调度器 [ROAD-M4-004] [ROAD-M4-005] [ARCH-DET-002]。
//!
//! ## 这一层做什么
//!
//! 把 `yeban_model::RoutingGraph` 编译成一份**分层执行计划**, 然后逐块渲染:
//!
//! 1. **按拓扑层级并行**: 同一层内的节点互不依赖, 用 Rayon 的工作窃取池并行渲染
//!    ([ROAD-M4-004] "基于 RoutingGraph 的无环有向图拓扑排序, 按音轨依赖层级生成
//!    并行工作单元");
//! 2. **汇聚确定性串行归约**: 每个总线节点的入边贡献**先按
//!    `(source_node EntityId, edge_id)` 字典序排成固定顺序**, 再单线程串行累加
//!    ([ARCH-DET-002])。归约核在 [`crate::sum`], 顺序推导也在那里 —— 这一层不
//!    自己实现求和;
//! 3. **PDC 相位对齐**: 每条边的补偿延迟来自 [`crate::pdc`] 的关键路径分析, 在
//!    进入总线求和前插入延迟线 ([ARCH-PDC-001])。
//!
//! ## 为什么归档顺序与线程数无关
//!
//! 每个节点缓冲写在自己的槽位里, 槽位索引由"层号 + EntityId 字典序"唯一决定
//! (因此每层在槽数组里是**连续区间**, `par_iter_mut` 才能给出互不重叠的可变借用)。
//! 线程只决定"谁先算完", 而 **bus 的求和顺序在读的是预先排好的 `incoming` 列表**,
//! 与完成时刻无关。所以:
//!
//! ```text
//! 同一输入 + 任意线程数 (1 / 2 / 4 / 8)  =>  输出缓冲逐位相同
//! ```
//!
//! 这正是 `tests::output_is_bit_identical_across_thread_counts` 钉住的判据。
//! 把它改坏的办法只有一种 —— 让归约按"完成顺序"进行 (例如把 `par_iter` 的结果
//! 边收集边累加)。那种改法会让该判据变红; 注入记录见
//! `docs/ledger/render-master-notes.md` §6。
//!
//! ## 样本源是注入的, 不是内建的
//!
//! 本 crate **不依赖 `yeban-engine`** (避免与实时引擎线争同一个 crate)。渲染器只
//! 认识 [`AudioSource`] 这个 trait; 测试用合成信号源, 产品侧由 engine 线提供实现。
//! 代价与待办见 notes 的 `needs`。
//!
//! ## 边界（这次没有证明什么）
//!
//! - 所有节点缓冲声道数一致 (`RenderOptions::channels`); 单声道源需要自己复制到
//!   各声道。真正的 per-node 声道布局不在本切片内。
//! - 只渲染**能到达 Master** 的子图; 悬挂节点被剪掉 (这也会让"不需要的源"不必注册)。
//! - 侧链 (`RoutingKind::Sidechain`) 只按普通音频边处理, 侧链键控语义未实现。
//! - 渲染失败时并行收集的是"某一个"错误, 不承诺是哪一个 (成功路径的确定性不受影响)。
//! - **插入链** (`crate::inserts`) 给**每个**节点挂 —— 包括**主总线**（任务单 `#173`
//!   起; 此前主总线刻意是 `None`）, 且只支持立体声
//!   (`channels != 2` + 非空链 ⇒ [`RenderError::UnsupportedInsertChannels`])。
//!   紧接在**求和之后**处理（源节点是"声源之后"）; 顺序差异见该模块头 §2。
//!   ⚠ 实时引擎的**主总线**插入是"下混成单声道 → 链 → 复制两路"（裁决 (甲)）,
//!   离线是真立体声 ⇒ 两边的**宽度/尾巴形状**有差异（与 `#155`/`#158` 同族）。
//! - **输出级**（[`OutputStage`]，任务单 `#296` / B2）：插入链**之后**、母线求和**之前**
//!   的一级。`#296` 让**总线节点**有推子（`TrackVolume`）、**主总线**有声相；B2 再把
//!   **总线**的静态声相 / `TrackPan` 自动化 / `mute` / `solo` 接到同一级（`mute`/`solo`
//!   由调用方折算进静态 `gain`，输出级只乘 0/1 与推子）。主总线的 `TrackPan` 自动化与
//!   推子仍由调用方在执行之后乘（主总线是终端节点）。其它节点是恒等输出级 ⇒
//!   `render_node` 的最后一级一个样本都不碰（零默认回归）。

use std::collections::BTreeMap;
use std::fmt::Debug;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use yeban_model::{EntityId, RoutingEdge, RoutingGraph, TrackV3};

use crate::automation::{
    AUTOMATION_BLOCK_SIZE, DeviceParamAutomation, SendGainAutomation, TrackGainAutomation,
    TrackPanAutomation, centered_pan_gains,
};
use crate::inserts::{InsertChain, InsertRuntime};
use crate::pdc::{self, DelayLine, PdcError};
use crate::sum;

/// [ARCH-DET-001] 规定的统一处理块大小: **128 采样点**。
///
/// 规范把"固定统一处理块大小 (128 采样点)"列为 L1 位级一致的**前提条件**之一,
/// 而不仅仅是性能选择: 块大小变了, 参数平滑与延迟线的时间量化就变了。
pub const L1_BLOCK_SIZE: usize = 128;

/// 渲染选项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    /// 处理块大小 (帧)。默认 [`L1_BLOCK_SIZE`]。
    pub block_size: usize,
    /// 工作线程数。`None` = 由 Rayon 决定。
    pub threads: Option<usize>,
    /// 确定性随机种子 ([ARCH-DET-001] "固定种子 PRNG")。渲染器自身不使用熵源;
    /// 它把种子透传给样本源 (抖动/噪声必须由它派生)。
    pub seed: u64,
    /// 声道数。
    pub channels: usize,
    /// 总帧数。
    pub frames: u64,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
}

impl Default for RenderOptions {
    /// L1 契约要求的默认配置: 128 帧块、无额外熵源。
    fn default() -> Self {
        Self {
            block_size: L1_BLOCK_SIZE,
            threads: None,
            seed: 0,
            channels: 2,
            frames: 0,
            sample_rate: 48_000,
        }
    }
}

impl RenderOptions {
    /// 按 [ARCH-DET-001] 的 L1 条件构造: 固定 128 帧块 + 调用方给定的固定种子。
    #[must_use]
    pub const fn l1(frames: u64, channels: usize, sample_rate: u32, seed: u64) -> Self {
        Self {
            block_size: L1_BLOCK_SIZE,
            threads: None,
            seed,
            channels,
            frames,
            sample_rate,
        }
    }

    /// 只用指定的线程数跑 (判据用: 1/2/4/8 线程必须逐位相同)。
    #[must_use]
    pub const fn with_threads(mut self, threads: usize) -> Self {
        self.threads = Some(threads);
        self
    }
}

/// 传给样本源的一块上下文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockContext {
    /// 块序号 (从 0 开始)。
    pub block_index: u64,
    /// 本块第一帧在整条时间轴上的位置。
    pub first_frame: u64,
    /// 本块的帧数 (最后一块可能不足 `block_size`)。
    pub frames: usize,
    /// 声道数。
    pub channels: usize,
    /// 采样率。
    pub sample_rate: u32,
    /// 工程级确定性种子。
    pub seed: u64,
}

impl BlockContext {
    /// 本块需要写入的交错样本个数 (`frames * channels`)。
    #[must_use]
    pub const fn width(&self) -> usize {
        self.frames * self.channels
    }
}

/// "每轨道样本源"的注入抽象。
///
/// 实现必须做到: 对同一 `(context, 调用序列)` 给出**逐位相同**的输出 —— 否则
/// L1 bit-exact 在源这一层就已经破了, 渲染器再怎么确定也救不回来。
///
/// ## 为什么同时要求 `Send + Sync`
///
/// 层内并行时, 每个任务既要写自己的节点, 又要**只读**地看已完成节点的缓冲。
/// 而"已完成节点"的只读视图里包含源对象, 所以 `&[NodeState]` 要被判为 `Send`,
/// 就必须 `NodeState: Sync`, 也就必须 `dyn AudioSource: Sync`。
///
/// 这条约束顺带禁止了源内部使用 `Rc`/`RefCell`/`Cell` 这类内部可变性 —— 对确定性
/// 渲染而言这是**有益**的约束: 内部可变性会让"同一输入两次渲染结果不同"变得可能。
pub trait AudioSource: Send + Sync {
    /// 把本块的 `context.width()` 个交错样本写进 `out`。
    ///
    /// `out` 的长度保证等于 `context.width()`, 且调用前已被清零。
    ///
    /// # Errors
    ///
    /// 源自身失败时返回 [`RenderError::Source`]。
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError>;
}

/// 渲染失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// 路由图自洽性校验失败 (`RoutingGraph::validate`)。
    InvalidGraph(String),
    /// Master 节点不在路由图里。
    MasterNotInGraph(EntityId),
    /// PDC 分析失败 (有环 / 端点缺失)。
    Pdc(String),
    /// 声道数为 0。
    ZeroChannels,
    /// 块大小为 0。
    ZeroBlockSize,
    /// 总帧数为 0。
    ZeroFrames,
    /// 尺寸的乘法**溢出 `usize`** —— `what` 是被算溢出的那个乘积。
    ///
    /// [`RenderPlan::compile`] 要算两个乘积: `block_size × channels`（一个节点的交错缓冲
    /// 长度）与 `frames × channels`（整条母带的采样总数）。两者都用 `checked_mul`。
    ///
    /// # 为什么必须由编译期挡（实测）
    ///
    /// 修复前 `block_size = usize::MAX`、`channels = 2` 会在 debug 下**直接 panic**
    /// （`attempt to multiply with overflow`）, 在 release 下回绕成一个小缓冲、随后在
    /// `buffer[..width]` 上越界。判据是
    /// `tests::an_extreme_block_size_is_rejected_instead_of_overflowing`。
    ///
    /// 这是一条**算术可表示性**检查, 不是内存检查: "这台机器装不下"不是本层能回答的问题,
    /// 本层只回答"这个乘积在 `usize` 里存不存在"。
    SizeOverflow {
        /// 溢出的那个乘积（如 `"block_size * channels"`）。
        what: &'static str,
    },
    /// 某条源节点没有注册样本源。
    MissingSource(EntityId),
    /// 给一个不在（剪枝后的）渲染计划里的节点注册了样本源。
    UnknownNode(EntityId),
    /// 线程池构建失败。
    ThreadPool(String),
    /// 样本源自己报的错。
    Source {
        /// 出错的节点。
        node: EntityId,
        /// 源给的说明。
        message: String,
    },
    /// 有插入链要处理, 但渲染声道数不是 2（本切片的混响/卷积只接立体声）。
    ///
    /// EQ 本身逐声道线性、任意声道数都能算, 但 Freeverb 与卷积混响的器件入口是
    /// **立体声块**（L/R 两条独立状态）。在没有裁决"单声道插入点"之前不猜:
    /// 任何非立体声 + 非空插入链的组合在这里**如实失败**, 而不是静默跳过。
    UnsupportedInsertChannels(usize),
}

impl core::fmt::Display for RenderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidGraph(message) => write!(f, "路由图非法: {message}"),
            Self::MasterNotInGraph(master) => write!(f, "Master 节点不在路由图里: {master}"),
            Self::Pdc(message) => write!(f, "PDC 分析失败: {message}"),
            Self::ZeroChannels => f.write_str("声道数为 0"),
            Self::ZeroBlockSize => f.write_str("块大小为 0"),
            Self::ZeroFrames => f.write_str("总帧数为 0"),
            Self::SizeOverflow { what } => write!(f, "{what} 溢出 usize"),
            Self::MissingSource(node) => write!(f, "节点 {node} 没有注册样本源"),
            Self::UnknownNode(node) => {
                write!(f, "节点 {node} 不在渲染计划里（未知或被剪枝）")
            }
            Self::ThreadPool(message) => write!(f, "线程池构建失败: {message}"),
            Self::Source { node, message } => write!(f, "样本源 {node} 失败: {message}"),
            Self::UnsupportedInsertChannels(channels) => write!(
                f,
                "有插入链要处理, 但声道数是 {channels}（本切片只支持立体声）"
            ),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<PdcError> for RenderError {
    fn from(error: PdcError) -> Self {
        Self::Pdc(error.to_string())
    }
}

/// 某个总线节点的一条入边在**固定归约顺序**里的位置。
///
/// 用具名结构而不是 `(EntityId, EntityId, u32)` 元组: 后者会触发
/// `clippy::type_complexity`（`midi.rs` 的 `track_chunks` 已经因为这个红过一次）,
/// 而且 `entry.0 / .1 / .2` 在调用点读不出含义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusInput {
    /// 上游（源）节点身份 —— 归约顺序的**第一排序键**。
    pub source_node: EntityId,
    /// 边身份 —— 第二排序键（保证键唯一）。
    pub edge_id: EntityId,
    /// 该边的 PDC 补偿延迟（帧）。
    pub delay_frames: u32,
}

/// 一条入边在固定归约顺序里的位置。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Contribution {
    /// 上游节点槽位。
    source_slot: usize,
    /// 上游节点身份 (判据要断言顺序, 因此留痕)。
    source_node: EntityId,
    /// 边身份 (顺序键的第二段)。
    edge_id: EntityId,
    /// 线性增益 (`gain_db` 转线性, `None` 即 1.0)。
    gain: f32,
    /// PDC 补偿延迟 (帧)。
    delay_frames: u32,
}

/// 一个节点的运行时状态。
struct NodeState {
    node: EntityId,
    /// 交错样本缓冲, 长度恒为 `block_size * channels`。
    buffer: Vec<f32>,
    /// 仅源节点有值。
    source: Option<Box<dyn AudioSource>>,
    /// 入边贡献, **已按 `(source_node, edge_id)` 字典序排好**。
    incoming: Vec<Contribution>,
    /// 与 `incoming` 一一对应的发送增益自动化（任务单 `#256`）。
    ///
    /// `None` = 这条边没有可求值的 `SendGain` 泳道 ⇒ 走原来的静态
    /// [`sum::accumulate_into`]（逐位不变）；`Some` 且恒等 ⇒ 同样走静态路径；
    /// `Some` 且非恒等 ⇒ 每块求值一次 + 逐样本平滑后累加。
    send_automations: Vec<Option<SendGainAutomation>>,
    /// 本节点的**设备参数**自动化（任务单 `#266`；`#267` 起接通道条整组字段；
    /// `#293` 起接 Freeverb / 卷积标量字段）。
    ///
    /// 与发送/声相不同，它们不乘样本：每 128 帧块起点求值一次，把该字段的
    /// **原生绝对值**（`_db` / `_hz` / 秒 / 开关 `0.0`·`1.0`）交给插入链运行态对应
    /// 器件的 `set_params`（重算系数）。空表 ⇒ 插入链的调用形状与接线前**逐位相同**。
    device_param_automations: Vec<DeviceParamAutomation>,
    /// 与 `incoming` 一一对应的延迟线 (零延迟时走旁路, 不使用 `scratch`)。
    delay_lines: Vec<DelayLine>,
    /// 延迟路径用的中转缓冲。
    scratch: Vec<f32>,
    /// 本节点的插入链运行态 (通道条 → Freeverb → 卷积)。
    ///
    /// `None` 表示"本节点没有可执行器件" ⇒ `render_node` 整段跳过 ⇒ 逐位不退化。
    /// ⚠ 主总线**也**挂链（`#173`；旧注释写"主总线恒为 `None`"已陈）。
    /// 有**非恒等**设备参数泳道的轨，对应器件在编译期被强制实例化
    /// （`#266` 通道条 / `#293` Freeverb 与卷积）⇒ 有器件可改。
    inserts: Option<InsertRuntime>,
    /// 本节点的**输出级**（任务单 `#296`）：插入链之后、母线求和之前。
    output: OutputStage,
}

/// 声相曲线（`pan → (左, 右)`）的持有者。
///
/// [`TrackPanAutomation::apply_pan_block`] 要逐帧把位置换成左右增益；曲线由调用方
/// 提供（CLI 走 `PanLaw`，MCP 走自己的等功率函数），本层⛔ 不复制曲线。用 `Arc`
/// 而不是 `Box` 是为了让 [`OutputStage`] 仍可 `Clone`（`Box<dyn Fn>` 不可 `Clone`）。
#[derive(Clone)]
struct PanCurve(std::sync::Arc<dyn Fn(f32) -> (f32, f32) + Send + Sync>);

impl Debug for PanCurve {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PanCurve(..)")
    }
}

impl PanCurve {
    fn gains(&self, pan: f32) -> (f32, f32) {
        (self.0)(pan)
    }
}

/// 节点的**输出级**：通道条的最后一级（源 → 插入 → 推子 → 声相 → 母线）。
///
/// ## 本单开的格子（`#296` ＋ B2）
///
/// - **推子**（静态 `gain` ＋ 可选 [`TrackGainAutomation`]）：只对**总线节点**启用
///   （`render_node` 的 `incoming` 非空、且不是主总线）。源节点的音量**仍在源侧**
///   （引擎的 `track_gain` / 音频片段的 `combined_gain_db` / MCP 的 `TrackSource`），
///   ⛔ 本层不搬家 —— 因此普通轨的 `TrackVolume` 行为与 `#296` 之前逐位相同。
///   B2 起调用方把总线的 `mute` / `solo`（含 `solo_safe`）折进这个静态 `gain`
///   （静音 ⇒ `0.0`），因此它们是**整条总线**的 0/1 门，而不是只染自有源；
/// - **声相**（静态 `(pan_left, pan_right)` ＋ 可选 [`TrackPanAutomation`]）：
///   `#296` 只对**主总线**启用；B2 起**总线节点**也启用（静态与泳道同一处落点）。
///   非恒等泳道时调用方把静态声相关掉（`pan = 0`），由本层逐块乘绝对左右增益。
///
/// ## 恒等快路径
///
/// `gain == 1.0`、没有非恒等的推子自动化、`(left, right) == (1.0, 1.0)`、没有非恒等
/// 的声相自动化 ⇒ [`Self::apply`] 在第一条判据就返回：**一个样本都不读不写**。没有
/// 输出级的工程（含默认夹具）因此与接线前逐位相同。
#[derive(Clone, Debug)]
pub struct OutputStage {
    /// 静态线性推子增益。
    gain: f32,
    /// 本节点的 `TrackVolume` 自动化（总线轨；`None` = 没有可求值泳道）。
    gain_automation: Option<TrackGainAutomation>,
    /// 左声道声相增益。
    pan_left: f32,
    /// 右声道声相增益。
    pan_right: f32,
    /// 本节点的 `TrackPan` 自动化（总线轨；`None` = 没有可求值泳道）。
    pan_automation: Option<TrackPanAutomation>,
    /// 声相曲线（与 `pan_automation` 同时存在；`None` 表示没有泳道）。
    pan_curve: Option<PanCurve>,
}

impl Default for OutputStage {
    /// 与 [`OutputStage::new`] 相同：恒等（`gain = 1.0`、`pan = (1, 1)`）。
    fn default() -> Self {
        Self::new()
    }
}

impl OutputStage {
    /// 恒等输出级（`gain = 1.0`、无声相、无自动化）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            gain: 1.0,
            gain_automation: None,
            pan_left: 1.0,
            pan_right: 1.0,
            pan_automation: None,
            pan_curve: None,
        }
    }

    /// 设静态推子（线性增益；非有限值退回恒等）。
    #[must_use]
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = if gain.is_finite() { gain } else { 1.0 };
        self
    }

    /// 挂上该节点的 `TrackVolume` 自动化（求值 / 5 ms 平滑 / 恒等判定都在
    /// [`TrackGainAutomation`]；本层⛔ 不复制）。
    #[must_use]
    pub fn with_gain_automation(mut self, automation: TrackGainAutomation) -> Self {
        self.gain_automation = Some(automation);
        self
    }

    /// 设静态声相：**位置**经调用方给的既有等功率曲线，并由
    /// [`centered_pan_gains`] 统一中心规则（`pan == 0` ⇒ `(1.0, 1.0)`，⛔ 不是
    /// `pan_gains(0)` 的 0.707）。
    #[must_use]
    pub fn with_pan<F>(mut self, pan: f32, curve: F) -> Self
    where
        F: Fn(f32) -> (f32, f32),
    {
        let (left, right) = centered_pan_gains(pan, curve);
        self.pan_left = left;
        self.pan_right = right;
        self
    }

    /// 挂上该节点的 `TrackPan` 自动化（求值 / 5 ms 平滑 / 恒等判定都在
    /// [`TrackPanAutomation`]；本层⛔ 不复制曲线与插值）。
    ///
    /// 调用方前置条件：非恒等时静态声相已被关掉（`with_pan(0.0, ..)`），否则同一
    /// 位置会乘两次。恒等泳道**不要**挂（挂了也只是走恒等快路，但会多一次判定）。
    #[must_use]
    pub fn with_pan_automation<F>(mut self, automation: TrackPanAutomation, curve: F) -> Self
    where
        F: Fn(f32) -> (f32, f32) + Send + Sync + 'static,
    {
        self.pan_automation = Some(automation);
        self.pan_curve = Some(PanCurve(std::sync::Arc::new(curve)));
        self
    }

    /// 恒等 ⇒ [`Self::apply`] 一个样本都不碰。
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.gain == 1.0
            && self.pan_left == 1.0
            && self.pan_right == 1.0
            && self
                .gain_automation
                .as_ref()
                .is_none_or(TrackGainAutomation::is_identity)
            && self
                .pan_automation
                .as_ref()
                .is_none_or(TrackPanAutomation::is_identity)
    }

    /// 在**插入链之后**应用这一级：先推子（含可选 R55 自动化），后声相（静态或
    /// `TrackPan` 自动化 —— 二者在非恒等时互斥，调用方负责关静态那一半）。
    fn apply(&mut self, out: &mut [f32], first_frame: u64, frames: usize, channels: usize) {
        if channels == 0 || frames == 0 || self.is_identity() {
            return;
        }
        let frames = frames.min(out.len() / channels);
        if frames == 0 {
            return;
        }
        match self.gain_automation.as_mut() {
            Some(automation) => {
                automation.apply_gain_block(out, first_frame, frames, channels, self.gain);
            }
            None => {
                if self.gain != 1.0 {
                    for sample in &mut out[..frames * channels] {
                        *sample *= self.gain;
                    }
                }
            }
        }
        if self.pan_left != 1.0 || self.pan_right != 1.0 {
            for frame in 0..frames {
                let base = frame * channels;
                out[base] *= self.pan_left;
                if channels > 1 {
                    out[base + 1] *= self.pan_right;
                }
                for channel in 2..channels {
                    out[base + channel] = 0.0;
                }
            }
        }
        // 非恒等 `TrackPan` 泳道：静态声相已被调用方关掉 ⇒ 这里乘的是**绝对**左右
        // 增益（与 `#257` 的源侧门同一条纪律，⛔ 不乘第二次）。
        if let Some(automation) = self.pan_automation.as_mut()
            && let Some(curve) = self.pan_curve.as_ref()
        {
            automation.apply_pan_block(out, first_frame, frames, channels, |pan| curve.gains(pan));
        }
    }
}

/// 编译后的执行计划。
pub struct RenderPlan {
    /// 全部保留节点, 按 `(层号, EntityId)` 排列 —— 因此每层是连续区间。
    states: Vec<NodeState>,
    /// 每层的槽位区间 `(start, end)`。
    layers: Vec<(usize, usize)>,
    /// Master 槽位。
    master_slot: usize,
    /// 最长延迟关键路径 (帧) [ARCH-PDC-001]。
    longest_path_frames: u32,
    /// 选项。
    options: RenderOptions,
    /// 槽位 -> 节点身份 (供 `bus_reduction_order` 使用)。
    slot_of: BTreeMap<EntityId, usize>,
}

impl Debug for RenderPlan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderPlan")
            .field("nodes", &self.states.len())
            .field("layers", &self.layers.len())
            .field("longest_path_frames", &self.longest_path_frames)
            .field("options", &self.options)
            .finish()
    }
}

/// 渲染产物。
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOutput {
    /// 交错 (interleaved) 母带样本。
    pub samples: Vec<f32>,
    /// 总帧数。
    pub frames: u64,
    /// 声道数。
    pub channels: usize,
    /// 处理块数。
    pub blocks: u64,
    /// 最长延迟关键路径 (帧)。
    pub longest_path_frames: u32,
    /// 输出缓冲的 SHA-256 位级摘要 (MUST-GATE-002 的 L1 判据载体)。
    pub digest: [u8; 32],
}

impl RenderOutput {
    /// 逐位哈希: 对每个 `f32` 取 IEEE-754 位型的小端字节流做 SHA-256。
    ///
    /// 用位型而不是数值: `+0.0` 与 `-0.0`、`NaN` 的载荷都是"位级一致"的一部分。
    #[must_use]
    pub fn digest_of(samples: &[f32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for sample in samples {
            hasher.update(sample.to_bits().to_le_bytes());
        }
        let output = hasher.finalize();
        let mut digest = [0u8; 32];
        // 逐字节拷贝而不是 `copy_from_slice(&output)`: 只依赖 `Array` 的 `Deref<[u8]>`
        // 带来的 `.iter()`, 不依赖版本的 `From`/`AsRef` 具体实现。
        for (slot, byte) in digest.iter_mut().zip(output.iter()) {
            *slot = *byte;
        }
        digest
    }
}

/// 从模型层的设备链累加每个轨道自身引入的处理延迟（帧）[ARCH-PDC-001]。
///
/// 语义与规范一致:
///
/// - 只有**未旁通**的设备计入 (`bypassed == true` 的插件不产生延迟);
/// - `latency_samples` 是**必需字段**（ADR-0001 D43 之后不再有"取 0 表示未上报"这个区分：
///   缺字段会直接报错）。因此 `0` 的含义就是**真的零延迟**，本函数把它
///   当 0 参与求和 —— 这是**保守**的: 少补只会少对齐, 不会造成相位错误; 而凭空发明一个
///   延迟才是错的。设备作者有义务显式声明真实值
///   (见 `DeviceDefinition::latency_samples` 的文档)。
/// - 多个设备串接时延迟相加, 用 `saturating_add` 以免病态输入溢出。
///
/// 用法:
///
/// ```text
/// let latencies = yeban_render::render::track_latencies(&project.tracks);
/// let plan = RenderPlan::compile_with_latencies(
///     &project.routing_graph, project.master_bus_track_id, options, &latencies)?;
/// ```
#[must_use]
pub fn track_latencies(tracks: &BTreeMap<EntityId, TrackV3>) -> BTreeMap<EntityId, u32> {
    tracks
        .iter()
        .map(|(id, track)| {
            let total = track
                .devices
                .iter()
                .filter(|device| !device.bypassed)
                .fold(0u32, |accumulated, device| {
                    accumulated.saturating_add(device.latency_samples)
                });
            (*id, total)
        })
        .collect()
}

/// 把 `gain_db` 转成线性增益。
///
/// 走 `libm::powf` 而不是 `f32::powf`: [ARCH-DET-001] 要求"统一启用纯 Rust `libm`
/// 数学库", 平台的 `powf` 在不同 libm 实现下可能差 LSB。
#[must_use]
pub fn db_to_linear(gain_db: f32) -> f32 {
    if !gain_db.is_finite() {
        return 1.0;
    }
    if gain_db == 0.0 {
        return 1.0;
    }
    libm::powf(10.0, gain_db / 20.0)
}

impl RenderPlan {
    /// 编译 `graph` 到 `master` 的执行计划。
    ///
    /// # Errors
    ///
    /// - [`RenderError::InvalidGraph`] 路由图不自洽;
    /// - [`RenderError::MasterNotInGraph`] `master` 不在节点表里;
    /// - [`RenderError::Pdc`] 图有环或端点缺失;
    /// - [`RenderError::ZeroChannels`] / [`RenderError::ZeroBlockSize`] /
    ///   [`RenderError::ZeroFrames`] 选项非法。
    pub fn compile(
        graph: &RoutingGraph,
        master: EntityId,
        options: RenderOptions,
    ) -> Result<Self, RenderError> {
        Self::compile_with_latencies(graph, master, options, &BTreeMap::new())
    }

    /// 与 [`Self::compile`] 相同, 但显式注入每个节点**自身**引入的处理延迟（帧）,
    /// 即 [ARCH-PDC-001] 的 `DeviceDefinition::latency_samples`。
    ///
    /// # 延迟从哪里来
    ///
    /// `yeban_model::DeviceDefinition::latency_samples` 是延迟的唯一事实源
    /// ([ARCH-PDC-001]), [`track_latencies`] 把它从 `TrackV3` 的设备链累加成
    /// "节点 → 延迟" 映射。这个参数保留为**显式输入**, 因为它同时是:
    ///
    /// - **上层推导的注入点**（用 [`track_latencies`] 从工程算出来就是最常见的用法）;
    /// - **覆盖/替身入口**（测试用固定延迟; 将来引入外部沙盒插件时, 其真实延迟可能
    ///   来自运行时握手而不是工程文档）。
    ///
    /// [`Self::compile`] 等价于传一张空表 —— 那表示"所有节点都是零延迟", 于是
    /// `L_max = 0`、没有任何补偿延迟。**这是刻意的保守默认**: 没有延迟信息时不做对齐,
    /// 而不是猜一个值。
    ///
    /// # Errors
    ///
    /// 与 [`Self::compile`] 相同。
    pub fn compile_with_latencies(
        graph: &RoutingGraph,
        master: EntityId,
        options: RenderOptions,
        latencies: &BTreeMap<EntityId, u32>,
    ) -> Result<Self, RenderError> {
        Self::compile_with_latencies_and_inserts(
            graph,
            master,
            options,
            latencies,
            &BTreeMap::new(),
        )
    }

    /// 与 [`Self::compile_with_latencies`] 相同, 但给每条轨挂上**插入链**。
    ///
    /// # 插入点
    ///
    /// 链接在 `render_node` 的**求和之后**（源节点则是"声源渲染之后"）：这样
    /// `SendToAux` 送进 Aux 返回轨的信号也会被该 Aux 轨自己的 EQ / 混响 / 卷积处理。
    /// ⭐ **主总线也挂链**（任务单 `#173`）：它和别的总线走**同一条**路径 ——
    /// 全部入边求和之后 → EQ → 混响 → 卷积。实时引擎在 `#173` 同时补上了主总线的
    /// 插入（`rt.rs` 的 3a⁗⁗），因此"导出与监听"在这件事上**不再**分叉；
    /// 剩下的差异只是形状（实时单声道插入 vs 离线立体声），见
    /// [`crate::inserts`] 模块头 §1／§2。
    ///
    /// `inserts` 由 [`crate::inserts::project_inserts`] 从 `TrackV3.devices` 投影得到。
    /// 空表等价于 [`Self::compile_with_latencies`]（逐位不退化）。
    ///
    /// # Errors
    ///
    /// 与 [`Self::compile`] 相同, 外加 [`RenderError::UnsupportedInsertChannels`]:
    /// 有非空插入链且 `options.channels != 2` 时如实失败（本切片只接立体声器件）。
    pub fn compile_with_latencies_and_inserts(
        graph: &RoutingGraph,
        master: EntityId,
        options: RenderOptions,
        latencies: &BTreeMap<EntityId, u32>,
        inserts: &BTreeMap<EntityId, InsertChain>,
    ) -> Result<Self, RenderError> {
        if options.channels == 0 {
            return Err(RenderError::ZeroChannels);
        }
        if options.block_size == 0 {
            return Err(RenderError::ZeroBlockSize);
        }
        if options.frames == 0 {
            return Err(RenderError::ZeroFrames);
        }
        // 两个尺寸乘积的**可表示性**在编译期定下, `execute` 里的同款乘法因此不再需要
        // 检查 (它用的是同一对因子)。`frames` 先 `try_from` 再乘: 在 32 位宿主上
        // `frames as usize` 本身会截断, 截断后乘积仍然"不溢出" —— 那会静默把母带长度
        // 改掉。`try_from` 把这一格也变成显式的 `Err`。
        let Some(block_width) = options.block_size.checked_mul(options.channels) else {
            return Err(RenderError::SizeOverflow {
                what: "block_size * channels",
            });
        };
        let total_width = usize::try_from(options.frames)
            .ok()
            .and_then(|frames| frames.checked_mul(options.channels));
        if total_width.is_none() {
            return Err(RenderError::SizeOverflow {
                what: "frames * channels",
            });
        }
        graph
            .validate()
            .map_err(|error| RenderError::InvalidGraph(error.to_string()))?;
        if !graph.nodes.contains(&master) {
            return Err(RenderError::MasterNotInGraph(master));
        }

        // ---- 1. 只保留能到达 Master 的子图 (悬挂节点被剪掉) ----
        let mut predecessors: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for edge in graph.edges.values() {
            predecessors
                .entry(edge.destination_node)
                .or_default()
                .push(edge.source_node);
        }
        let mut keep: std::collections::BTreeSet<EntityId> = std::collections::BTreeSet::new();
        let mut frontier = vec![master];
        while let Some(node) = frontier.pop() {
            // 平铺而不是嵌套 `if`: `clippy::collapsible_if` 属于 `clippy::all`,
            // 而工作区 lints 把它设为 deny。
            if !keep.insert(node) {
                continue;
            }
            if let Some(parents) = predecessors.get(&node) {
                frontier.extend(parents.iter().copied());
            }
        }
        // 插入链只在立体声下接（混响/卷积的器件入口是 L/R 两条独立状态）。
        // ⭐ 任务单 `#173`：主总线的链也算"要处理"（此前它被 `*node != master`
        // 排除在外 ⇒ 单声道导出里主总线器件被静默忽略）。
        if options.channels != 2
            && keep
                .iter()
                .any(|node| inserts.get(node).is_some_and(|chain| !chain.is_empty()))
        {
            return Err(RenderError::UnsupportedInsertChannels(options.channels));
        }
        let edges: Vec<&RoutingEdge> = graph
            .edges
            .values()
            .filter(|edge| {
                keep.contains(&edge.source_node) && keep.contains(&edge.destination_node)
            })
            .collect();

        // ---- 2. PDC 关键路径分析 (与实时引擎共用的算法) ----
        let pdc_graph = pdc::Graph {
            nodes: keep.iter().copied().collect(),
            edges: edges
                .iter()
                .map(|edge| (edge.source_node, edge.destination_node))
                .collect(),
            latencies: latencies
                .iter()
                .filter(|(node, _)| keep.contains(node))
                .map(|(node, &frames)| (*node, frames))
                .collect(),
        };
        let pdc_plan = pdc::plan(&pdc_graph, master)?;

        // ---- 3. 分层: level(n) = 无入边 ? 0 : max(level(src)) + 1 ----
        let mut level: BTreeMap<EntityId, u32> = BTreeMap::new();
        let mut incoming_nodes: BTreeMap<EntityId, Vec<&RoutingEdge>> = BTreeMap::new();
        for &edge in &edges {
            incoming_nodes
                .entry(edge.destination_node)
                .or_default()
                .push(edge);
        }
        for &node in &pdc_plan.topological_order {
            let node_level = incoming_nodes
                .get(&node)
                .map(|list| {
                    list.iter()
                        .map(|edge| level.get(&edge.source_node).copied().unwrap_or(0) + 1)
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            level.insert(node, node_level);
        }

        // ---- 4. 排槽位: (层号, EntityId) 升序 => 每层是连续区间 ----
        let mut ordered: Vec<EntityId> = keep.iter().copied().collect();
        ordered.sort_by_key(|node| (level.get(node).copied().unwrap_or(0), *node));
        let slot_of: BTreeMap<EntityId, usize> = ordered
            .iter()
            .enumerate()
            .map(|(slot, &node)| (node, slot))
            .collect();
        let layer_count = ordered
            .iter()
            .map(|node| level.get(node).copied().unwrap_or(0))
            .max()
            .map_or(0, |max| max as usize + 1);
        let layers: Vec<(usize, usize)> = (0..layer_count)
            .map(|wanted| {
                let range: Vec<usize> = ordered
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| level.get(node).copied().unwrap_or(0) as usize == wanted)
                    .map(|(slot, _)| slot)
                    .collect();
                match (range.first(), range.last()) {
                    (Some(&start), Some(&end)) => (start, end + 1),
                    _ => (0, 0),
                }
            })
            .collect();

        // ---- 5. 每个节点的入边贡献, 按 (source_node, edge_id) 字典序固定 ----
        // `block_width` 已在上面用 `checked_mul` 校验过, 这里直接复用那个值 ——
        // 不再做第二遍可能溢出的乘法。
        let mut states: Vec<NodeState> = ordered
            .iter()
            .map(|&node| {
                // ⭐ 任务单 `#173`：主总线**同样挂链** —— 它在 `render_node` 里
                // 走"求和之后 → 插入"这条与其它总线**完全相同**的路径
                // （此前这里刻意写 `None`，见 `crate::inserts` 模块头 §1）。
                let inserts = inserts
                    .get(&node)
                    .filter(|chain| !chain.is_empty())
                    .map(|chain| {
                        InsertRuntime::new(chain, options.sample_rate, options.block_size)
                    });
                NodeState {
                    node,
                    buffer: vec![0.0; block_width],
                    source: None,
                    incoming: Vec::new(),
                    send_automations: Vec::new(),
                    device_param_automations: Vec::new(),
                    delay_lines: Vec::new(),
                    scratch: vec![0.0; block_width],
                    inserts,
                    // 默认恒等：没有调用方挂总线推子 / 主总线声相时，`render_node`
                    // 的最后一级一个样本都不碰（零音频回归）。
                    output: OutputStage::new(),
                }
            })
            .collect();
        for (slot, &node) in ordered.iter().enumerate() {
            let Some(list) = incoming_nodes.get(&node) else {
                continue;
            };
            // `fixed_order` 断言键唯一 —— 边身份唯一, 因此这个复合键一定唯一。
            let contributions = sum::fixed_order(list.iter().map(|edge| {
                (
                    (edge.source_node, edge.id),
                    Contribution {
                        source_slot: slot_of[&edge.source_node],
                        source_node: edge.source_node,
                        edge_id: edge.id,
                        gain: edge.gain_db.map_or(1.0, db_to_linear),
                        delay_frames: pdc_plan
                            .delay_of(edge.source_node, edge.destination_node)
                            .unwrap_or(0),
                    },
                )
            }));
            states[slot].delay_lines = contributions
                .iter()
                .map(|contribution| {
                    DelayLine::new(contribution.delay_frames as usize, options.channels)
                })
                .collect();
            states[slot].send_automations = vec![None; contributions.len()];
            states[slot].incoming = contributions;
        }

        Ok(Self {
            states,
            layers,
            master_slot: slot_of[&master],
            longest_path_frames: pdc_plan.longest_path,
            options,
            slot_of,
        })
    }

    /// 渲染选项。
    #[must_use]
    pub const fn options(&self) -> RenderOptions {
        self.options
    }

    /// 最长延迟关键路径 `L_max` (帧) [ARCH-PDC-001]。
    #[must_use]
    pub const fn longest_path_frames(&self) -> u32 {
        self.longest_path_frames
    }

    /// 保留下来的节点身份 (按 `(层号, EntityId)` 排列)。
    #[must_use]
    pub fn nodes(&self) -> Vec<EntityId> {
        self.states.iter().map(|state| state.node).collect()
    }

    /// 每个节点所属的层号。
    #[must_use]
    pub fn node_levels(&self) -> BTreeMap<EntityId, usize> {
        let mut out = BTreeMap::new();
        for (wanted, &(start, end)) in self.layers.iter().enumerate() {
            for state in &self.states[start..end] {
                out.insert(state.node, wanted);
            }
        }
        out
    }

    /// 某个总线节点的**归约顺序**（[`BusInput`] 列表, 按 `(source_node, edge_id)`
    /// 字典序), 以及每条入边的补偿延迟。
    ///
    /// 这是 [ARCH-DET-002] 的可断言形式: 判据直接比较 `source_node` 序列是否等于
    /// EntityId 字典序, 而不是"看起来对"。
    #[must_use]
    pub fn bus_reduction_order(&self, bus: EntityId) -> Option<Vec<BusInput>> {
        let slot = *self.slot_of.get(&bus)?;
        Some(
            self.states[slot]
                .incoming
                .iter()
                .map(|contribution| BusInput {
                    source_node: contribution.source_node,
                    edge_id: contribution.edge_id,
                    delay_frames: contribution.delay_frames,
                })
                .collect(),
        )
    }

    /// 把一条**发送边**的自动化接到计划上（任务单 `#256`）。
    ///
    /// 只接受**保留子图里真的有这条贡献**的边：悬挂/被剪枝的边返回 `false`（调用方
    /// 据此把该泳道继续登记为不支持，而不是谎称应用了）。挂上之后，`execute` 在
    /// [`render_node`] 的入边归约里对这条边乘“静态边增益 × R55 比率”。
    ///
    /// 恒等泳道也可以挂（`is_identity` 为真 ⇒ 仍走原来的静态累加，逐字节不变）；
    /// 是否挂上不影响“没有泳道”的工程。
    pub fn attach_send_automation(&mut self, automation: SendGainAutomation) -> bool {
        let edge_id = automation.edge_id();
        for state in &mut self.states {
            if let Some(index) = state
                .incoming
                .iter()
                .position(|contribution| contribution.edge_id == edge_id)
            {
                state.send_automations[index] = Some(automation);
                return true;
            }
        }
        false
    }

    /// 给一个节点挂**输出级**（任务单 `#296`：总线推子 / 主总线声相）。
    ///
    /// 落点是 `render_node` 的**插入链之后**。恒等输出级可以挂（一个样本都不碰）；
    /// 返回 `false` = 该节点不在保留子图（被剪枝或未知）⇒ 调用方据此如实登记，
    /// 而不是谎称"推子已应用"。
    pub fn attach_output_stage(&mut self, node: EntityId, stage: OutputStage) -> bool {
        let Some(slot) = self.slot_of.get(&node).copied() else {
            return false;
        };
        self.states[slot].output = stage;
        true
    }

    /// 保留子图里全部入边身份（按 `(层号, EntityId)` 与归约顺序）。
    ///
    /// 读数层用它判定一条 `SendGain` 泳道的落点是否真的会进这次渲染 ——
    /// 与 [`Self::nodes`] 是同一份剪枝事实，⛔ 不复制图算法。
    #[must_use]
    pub fn edge_ids(&self) -> Vec<EntityId> {
        self.states
            .iter()
            .flat_map(|state| {
                state
                    .incoming
                    .iter()
                    .map(|contribution| contribution.edge_id)
            })
            .collect()
    }

    /// 把一条**设备参数**自动化接到计划上（任务单 `#266`）。
    ///
    /// 落点是该节点的插入链运行态（通道条 / Freeverb / 卷积标量）。恒等泳道可以挂
    /// （`value_at_block` 返回 `None`，一个参数都不改）；非恒等泳道要求该节点**已经有**
    /// 插入链运行态（调用方必须先用
    /// [`crate::inserts::project_inserts_with_forced_instances`] 按族强制实例化，见
    /// `crate::inserts` 模块头 §3.1 与本单 `#293` 的 `reverb_wet: 0` 缝），否则返回
    /// `false` —— 调用方据此如实登记，而不是谎称应用了。
    pub fn attach_device_param_automation(&mut self, automation: DeviceParamAutomation) -> bool {
        let track_id = automation.track_id();
        let Some(slot) = self.slot_of.get(&track_id).copied() else {
            return false;
        };
        let state = &mut self.states[slot];
        if !automation.is_identity() && state.inserts.is_none() {
            return false;
        }
        state.device_param_automations.push(automation);
        true
    }

    /// 执行渲染。
    ///
    /// `sources` 为计划里**每一条需要发声的节点**提供实现。⭐ 任务单 `#240`
    /// （Logic Pro 的通道条语义）：**有入边的节点同样可以注册自己的源** ——
    /// 源输出与全部入边在节点的**输入端按单位权重线性相加**，再进插入链。
    /// 只有"既没有入边、也没有源"的节点会被拒绝
    /// ([`RenderError::MissingSource`])。
    ///
    /// # 可重复执行 (复位契约)
    ///
    /// 本方法在**每一次**执行开始时复位全部跨执行存活的可变状态 (源槽与各条 PDC 延迟线),
    /// 因此**同一个 [`RenderPlan`] 连续执行多次与每次全新编译再执行逐位等价**。
    /// 入参 `sources` 是"**这一次**用哪些源": 上一次装载的对象一律不再沿用, 本次没给的
    /// 源节点报 [`RenderError::MissingSource`]。
    ///
    /// 没有这一段复位时, 第二次执行的前 `delay_frames` 帧会读到上一次留在延迟线环里的
    /// 尾部样本 (旧数据被重放), 且本次缺失的源会静默沿用上一次的对象。判据是
    /// `tests::a_second_execution_matches_a_fresh_plan` 与
    /// `tests::a_second_execution_does_not_reuse_sources_from_the_first`。
    ///
    /// 源表被拒绝时, 计划里**一个源都没装载** (先整表校验、再装载)。
    ///
    /// # Errors
    ///
    /// 见 [`RenderError`]。
    pub fn execute(
        &mut self,
        sources: BTreeMap<EntityId, Box<dyn AudioSource>>,
    ) -> Result<RenderOutput, RenderError> {
        let channels = self.options.channels;
        let block_size = self.options.block_size;
        let total_frames = self.options.frames;

        // ---- 0. 复位上一次执行留下的可变状态 ----
        //
        // 这一段必须发生在**任何装载之前**, 否则"同一个计划执行两次"与"全新编译再
        // 执行"就不等价: 延迟线的环形缓冲里还留着上一次的尾部样本, 源槽里还留着上一次
        // 的对象。`NodeState::buffer` / `NodeState::scratch` **不需要**复位 ——
        // `render_node` 在读取之前会写满 `buffer[..width]`, 而 `DelayLine::process`
        // 会写满 `scratch[..width]` (见各自文档), 因此每一块读到的都是本块刚写进去的值。
        for state in &mut self.states {
            state.source = None;
            for line in &mut state.delay_lines {
                line.reset();
            }
            // 发送增益自动化的 5 ms 平滑器也是**跨执行存活**的时间状态 ⇒ 一起复位
            // （回到“tick 0 的曲线值”，与全新构造逐位一致）。
            for automation in state.send_automations.iter_mut().flatten() {
                automation.reset_for_execution();
            }
            // 设备参数自动化的读数（任务单 `#266`）也是跨执行存活的；器件状态在
            // `inserts.reset()` 里复位（`set_params` 不清状态，`reset` 才清）。
            for automation in &mut state.device_param_automations {
                automation.reset_for_execution();
            }
            // 插入器件（EQ 的双二阶状态、混响的梳状/全通线、卷积的分块尾）也是
            // **跨执行存活**的时间状态 ⇒ 必须一起复位，否则第二次执行的尾巴会
            // 带着上一次的残留。
            if let Some(inserts) = &mut state.inserts {
                inserts.reset();
            }
            // 输出级的推子自动化（任务单 `#296`）同样是跨执行存活的时间状态。
            if let Some(automation) = state.output.gain_automation.as_mut() {
                automation.reset_for_execution();
            }
            // B2：输出级的声相自动化同样要复位（平滑器回到 tick 0 的曲线值）。
            if let Some(automation) = state.output.pan_automation.as_mut() {
                automation.reset_for_execution();
            }
        }

        // ---- 1. 先校验整张源表, 再装载 ----
        //
        // 分两遍是刻意的: 校验失败时计划必须留在**定义明确**的状态 (一个源都没装载),
        // 而不是"装到一半就返回"。旧实现在同一个循环里边查边装, 于是被拒的源表会在
        // `states` 里留下排在它前面的那几个源, 下一次执行就把它们当成"调用方给的源"。
        //
        // ⭐ 任务单 `#240`: 这里**不再**拒绝"给有入边的节点注册源"。Logic Pro 的通道条
        // 只有一种形状 —— 输入端求和 → 插入链 → 输出, 软件音源的输出就接在输入端
        // ⇒ "总线既有自己的音源、又接别处的发送"是常态, 两路相加。`render_node` 的
        // 输入端求和负责那一步; 本层只保留"未知节点"的校验。
        for &node in sources.keys() {
            self.slot_of
                .get(&node)
                .copied()
                .ok_or(RenderError::UnknownNode(node))?;
        }
        for (node, source) in sources {
            let slot = self.slot_of[&node];
            self.states[slot].source = Some(source);
        }
        for state in &self.states {
            if state.incoming.is_empty() && state.source.is_none() {
                return Err(RenderError::MissingSource(state.node));
            }
        }

        let pool = rayon::ThreadPoolBuilder::new()
            // `None`（auto）解析为 **1 线程**，依据是**实测扫描**而不是直觉：在参考机（M2 Max）上以
            // tracks=4/8/16/32/64、各 30 秒扫过两种模式，**并行在每一个尺寸都更慢**（1.9–4.4×；
            // 32 轨时 auto 95.5× vs 单线程 229.3×），说明这条路是**开销/带宽受限**而非粒度受限 ——
            // 为一个 ~130 ms 的顺序任务转 12 个线程、还要做固定顺序合并（为保 bit-exact），是亏本买卖。
            // 因此默认不再启用并行；要用多核请显式 `with_threads(n)`，其逐字节等价性由 lib.rs 的
            // 1/2/4/8 线程等价判据守着。等有人**量出**真正的交叉点再改回按阈值启用。
            .num_threads(self.options.threads.unwrap_or(1))
            .build()
            .map_err(|error| RenderError::ThreadPool(error.to_string()))?;

        let blocks = total_frames.div_ceil(block_size as u64);
        // `frames * channels` 的可表示性由 `compile` 的 `checked_mul` 定下 (那一步不过就
        // 拿不到这个计划), 因此这里的乘法不会溢出 —— 不需要第二遍检查。
        let mut samples = vec![0.0f32; (total_frames as usize) * channels];

        pool.install(|| -> Result<(), RenderError> {
            let states = &mut self.states;
            let layers = &self.layers;
            let master_slot = self.master_slot;
            for block_index in 0..blocks {
                let first_frame = block_index * block_size as u64;
                let frames = ((total_frames - first_frame) as usize).min(block_size);
                let context = BlockContext {
                    block_index,
                    first_frame,
                    frames,
                    channels,
                    sample_rate: self.options.sample_rate,
                    seed: self.options.seed,
                };
                let width = context.width();

                for &(start, end) in layers {
                    let (done, current) = states.split_at_mut(start);
                    let done: &[NodeState] = done;
                    let current = &mut current[..end - start];
                    current
                        .par_iter_mut()
                        .try_for_each(|state| render_node(state, done, context, width))?;
                }

                let master = &states[master_slot].buffer[..width];
                let offset = (first_frame as usize) * channels;
                samples[offset..offset + width].copy_from_slice(master);
            }
            Ok(())
        })?;

        let digest = RenderOutput::digest_of(&samples);
        Ok(RenderOutput {
            samples,
            frames: total_frames,
            channels,
            blocks,
            longest_path_frames: self.longest_path_frames,
            digest,
        })
    }
}

/// 渲染一个节点的一块：**输入端求和**（自己的源 ＋ 全部入边）→ 插入链 → **输出级**
/// （[`OutputStage`]，任务单 `#296`）。
///
/// ⭐ 任务单 `#240`（Logic Pro 的通道条语义）：Logic 的通道条只有一种形状 ——
/// `输入端求和 → 插入链 → 输出`，软件音源的输出就接在**输入端**。因此这里**不是**
/// 两条分支（"源节点" vs "总线节点"），而是一条：`buffer` 先装自己的源（若有），
/// 再把全部入边按固定顺序累加到**同一个** `buffer` 上，最后过插入链。
///
/// 关键性质（判据 `insert_chain.rs::the_source_and_the_incoming_edge_are_summed_before_the_insert_chain`
/// 钉住）：两路在插入链**之前**已经合并 ⇒ 插入链同时处理两路，而不是只处理入边。
fn render_node(
    state: &mut NodeState,
    done: &[NodeState],
    context: BlockContext,
    width: usize,
) -> Result<(), RenderError> {
    let NodeState {
        node,
        buffer,
        source,
        incoming,
        send_automations,
        device_param_automations,
        delay_lines,
        scratch,
        inserts,
        output,
    } = state;

    // ---- 输入端: 自己的源（若有）先写进来 ----
    //
    // 与 Logic 同形: 软件音源的输出进入通道条的**输入端**（与 send 同一个加法点）,
    // 而不是在插入链之后另加一路。源节点的 `inserts` 因此也处理自己的源
    // （这正是 `#173` 主总线挂链之后一直没变的那条路径）。
    buffer[..width].fill(0.0);
    if let Some(source) = source.as_mut() {
        source.render_block(context, &mut buffer[..width])?;
    } else if incoming.is_empty() {
        // 源槽为空且没有入边 ⇒ 没有声音可发; 保留 `MissingSource` 的响亮失败
        // （编译期不拒绝源槽为空的总线, 因为它的声音由入边归约得到）。
        return Err(RenderError::MissingSource(*node));
    }

    // ---- 输入端: 全部入边累加到同一个 buffer（含自己的源） ----
    //
    // [ARCH-DET-002] 固定顺序的**单线程串行累加**。`incoming` 在编译期已按
    // (source_node, edge_id) 字典序排好, 这里的 for 循环顺序就是全部。
    for (index, contribution) in incoming.iter().enumerate() {
        let upstream = &done[contribution.source_slot].buffer[..width];
        let line = &mut delay_lines[index];
        let source_slice = if line.is_bypass() {
            // 绝大多数边没有补偿延迟: 直接累加, 连一次拷贝都不要。
            upstream
        } else {
            line.process(upstream, &mut scratch[..width]);
            &scratch[..width]
        };
        // 任务单 `#256`：发送增益自动化挂在这一条**入边**上。没有泳道、或泳道恒等
        // ⇒ 走与修前**逐位相同**的 `sum::accumulate_into`；非恒等 ⇒ 块级求值 +
        // 逐样本 5 ms 平滑后累加（与 `TrackVolume` 共用同一套求值/平滑实现）。
        match &mut send_automations[index] {
            Some(automation) if !automation.is_identity() => {
                automation.accumulate_block(
                    source_slice,
                    &mut buffer[..width],
                    context.first_frame,
                    context.frames,
                    context.channels,
                    contribution.gain,
                );
            }
            _ => {
                sum::accumulate_into(source_slice, &mut buffer[..width], contribution.gain);
            }
        }
    }
    // 插入点: **输入端求和之后**（本票的裁决 —— Aux 返回轨因此也能被它自己的链处理）。
    // ⭐ 任务单 `#173`：主总线也走这一行（`inserts` 不再是 `None`）⇒ 导出里
    // master 上的设备真的生效；顺序与实时侧同一条链：EQ → 混响 → 卷积。
    // ⭐ 任务单 `#240`：自己的源也已经在上面进了 `buffer` ⇒ 同一条链同时处理两路。
    // ⭐ 任务单 `#266`：有**非恒等**设备参数泳道时，把本块按 128 帧切段，每段**起点**
    // 求一次曲线值 → `apply_device_params`（`ChannelStrip::set_params` 重算系数）→
    // 再跑那一段。⛔ 不逐样本平滑：改的是参数，不是增益。没有设备参数泳道（或全是恒等）
    // 时走**原来那一句** `inserts.process` ⇒ 产物逐位不变（含任意 `block_size`）。
    if let Some(inserts) = inserts.as_mut() {
        if device_param_automations
            .iter()
            .all(DeviceParamAutomation::is_identity)
        {
            inserts.process(&mut buffer[..width]);
        } else {
            let channels = context.channels;
            let mut offset = 0usize;
            while offset < context.frames {
                let chunk = (context.frames - offset).min(AUTOMATION_BLOCK_SIZE);
                let block_first = context.first_frame.saturating_add(offset as u64);
                for automation in device_param_automations.iter_mut() {
                    // 字段来自泳道自己（`#267`：通道条整组；`#293`：混响/卷积标量），
                    // ⛔ 本层不写"哪些名字"的表。
                    if let (Some(value), Some(field)) =
                        (automation.value_at_block(block_first), automation.field())
                    {
                        inserts.apply_device_params(field, value);
                    }
                }
                inserts.process(&mut buffer[offset * channels..(offset + chunk) * channels]);
                offset += chunk;
            }
        }
    }
    // ---- 输出级（任务单 `#296`）：**插入链之后**、母线求和之前 ----
    //
    // 这是"通道条"的最后一级（源 → 插入 → 推子 → 声相 → 母线）。本单只让
    // 总线节点有推子、主总线有声相（见 [`OutputStage`] 的文档）；恒等时
    // `apply` 第一条判据就返回 ⇒ 其它节点、以及没有接线过的计划逐位不变。
    output.apply(
        buffer,
        context.first_frame,
        context.frames,
        context.channels,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use yeban_model::{DeviceDefinition, RoutingEdge, RoutingKind};

    /// Crockford Base32 字母表（ULID 的规范字母表，排除 I/L/O/U）。
    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    /// 造一个合法的 26 字符 ULID 文本: 末 4 位按 5 位一组大端编码 `index`。
    /// 因此 `ulid(a) < ulid(b)` 当且仅当 `a < b` —— 判据可以直接用数字序号推理字典序。
    fn ulid(index: u32) -> EntityId {
        let mut text = [b'0'; 26];
        for position in 0..4 {
            text[25 - position] = CROCKFORD[((index >> (5 * position)) & 0x1F) as usize];
        }
        let text = core::str::from_utf8(&text).expect("ASCII");
        EntityId::from_str(text).expect("合法 ULID")
    }

    /// 造一条边。
    ///
    /// 边身份用 `EntityId::new()`（随机 ULID）: 本模块的判据要么按 `(source_node, edge_id)`
    /// 排序（键的第一段是源节点, 随机第二段不影响顺序）, 要么用 `contains`/按源节点查找,
    /// 因此与边身份无关。`star_graph` 是唯一需要"边身份与源节点顺序相反"的地方,
    /// 它在构造后显式覆写 `id`。
    fn edge(source: EntityId, destination: EntityId, gain_db: Option<f32>) -> RoutingEdge {
        RoutingEdge {
            id: EntityId::new(),
            source_node: source,
            destination_node: destination,
            kind: RoutingKind::TrackToBus,
            gain_db,
        }
    }

    fn graph(nodes: &[EntityId], edges: Vec<RoutingEdge>) -> RoutingGraph {
        RoutingGraph {
            nodes: nodes.to_vec(),
            edges: edges.into_iter().map(|edge| (edge.id, edge)).collect(),
        }
    }

    /// "N 轨 -> Master" 的星形图。边的 EntityId 刻意与源节点序号**逆序**,
    /// 这样"按边身份排序"与"按源节点排序"会给出相反的归约顺序。
    fn star_graph(tracks: u32) -> (RoutingGraph, EntityId, Vec<EntityId>) {
        let master = ulid(0xFFFF);
        let mut nodes = vec![master];
        let mut edges = Vec::new();
        let mut sources = Vec::new();
        for index in 1..=tracks {
            let track = ulid(index);
            nodes.push(track);
            sources.push(track);
            let mut created = edge(track, master, None);
            created.id = ulid(0x8000 - index); // 逆序边身份
            edges.push(created);
        }
        (graph(&nodes, edges), master, sources)
    }

    /// 确定性测试源: 只用乘加与 `round`, 不含任何超越函数 —— 源本身不是不确定源,
    /// 于是判据测到的就真的是汇聚顺序。
    struct SyntheticSource {
        amplitude: f32,
        phase: f32,
        step: f32,
    }

    impl SyntheticSource {
        fn new(seed: u64, index: u64, amplitude: f32) -> Self {
            let spread = ((seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 33) as u32;
            Self {
                amplitude,
                phase: 0.0,
                step: 0.0001 + (spread % 977) as f32 * 0.000_01,
            }
        }
    }

    impl AudioSource for SyntheticSource {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            for (frame, slot) in out.chunks_mut(context.channels).enumerate() {
                let raw = self.phase + (frame as f32) * self.step;
                let value = self.amplitude * (raw - raw.round());
                for sample in slot.iter_mut() {
                    *sample = value;
                }
            }
            self.phase += (context.frames as f32) * self.step;
            while self.phase > 1.0 {
                self.phase -= 1.0;
            }
            Ok(())
        }
    }

    /// 常量源 (算术判据用)。
    struct ConstantSource(f32);

    impl AudioSource for ConstantSource {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            out.fill(self.0);
            let _ = context;
            Ok(())
        }
    }

    /// 记录它看到的每一块上下文 (块切分判据用)。
    struct RecordingSource {
        inner: ConstantSource,
        seen: Vec<(u64, u64, usize)>,
    }

    impl AudioSource for RecordingSource {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            self.seen
                .push((context.block_index, context.first_frame, context.frames));
            self.inner.render_block(context, out)
        }
    }

    /// 一定失败的源 (错误传播判据用)。
    struct FailingSource;

    impl AudioSource for FailingSource {
        fn render_block(
            &mut self,
            _context: BlockContext,
            _out: &mut [f32],
        ) -> Result<(), RenderError> {
            Err(RenderError::Source {
                node: EntityId::default(),
                message: "故意失败".to_owned(),
            })
        }
    }

    fn synthetic_sources(
        sources: &[EntityId],
        seed: u64,
    ) -> BTreeMap<EntityId, Box<dyn AudioSource>> {
        sources
            .iter()
            .enumerate()
            .map(|(index, &node)| {
                let amplitude = 1.0e7 * ((index % 3) as f32 + 1.0);
                let source: Box<dyn AudioSource> =
                    Box::new(SyntheticSource::new(seed, index as u64, amplitude));
                (node, source)
            })
            .collect()
    }

    /// **核心判据 [ARCH-DET-002]**: 同一输入 + 任意线程数 ⇒ 输出逐位相同。
    #[test]
    fn output_is_bit_identical_across_thread_counts() {
        let (routing, master, sources) = star_graph(16);
        let options = RenderOptions::l1(2_048, 2, 48_000, 0xC0FF_EE00);
        let mut reference: Option<RenderOutput> = None;
        for threads in [1usize, 2, 4, 8] {
            let mut plan =
                RenderPlan::compile(&routing, master, options.with_threads(threads)).expect("编译");
            let output = plan
                .execute(synthetic_sources(&sources, options.seed))
                .expect("渲染");
            let previous = reference.replace(output);
            if let Some(expected) = previous {
                let current = reference.as_ref().expect("刚写入");
                assert_eq!(
                    current.digest, expected.digest,
                    "{threads} 线程的输出与 1 线程不同 —— 汇聚没有走固定顺序"
                );
                assert_eq!(current.samples, expected.samples);
            }
        }
        let reference = reference.expect("至少跑过一次");
        assert_eq!(reference.frames, 2_048);
        assert_eq!(reference.channels, 2);
        assert_eq!(reference.blocks, 16, "2048 / 128 = 16 块");
        assert_eq!(reference.samples.len(), 2_048 * 2);
    }

    /// 判据: 同线程数、同输入重复执行 ⇒ 同摘要 (进程内可复现)。
    ///
    /// 注意本判据刻意**各自编译一个计划**: 它证明的是"两个全新实例互相一致"。
    /// "**同一个**计划连续执行两次"是另一条契约, 见
    /// `a_second_execution_matches_a_fresh_plan` —— 那一条需要真实延迟线才有的区分力,
    /// 本判据的星形图没有延迟。
    #[test]
    fn repeated_runs_agree() {
        let (routing, master, sources) = star_graph(8);
        let options = RenderOptions::l1(512, 2, 48_000, 7).with_threads(2);
        let mut first = RenderPlan::compile(&routing, master, options).expect("编译");
        let a = first
            .execute(synthetic_sources(&sources, options.seed))
            .expect("渲染");
        let mut second = RenderPlan::compile(&routing, master, options).expect("编译");
        let b = second
            .execute(synthetic_sources(&sources, options.seed))
            .expect("渲染");
        assert_eq!(a.digest, b.digest);
    }

    /// 判据 (**复位契约**): **同一个计划**连续执行两次, 第二次必须与**全新编译**的
    /// 计划逐位一致。
    ///
    /// 延迟线的环形缓冲与帧游标是跨执行存活的可变状态。不复位时, 第二次执行的前
    /// `delay_frames` 帧读到的是**上一次执行留在环里的尾部样本** —— 旧数据被重放,
    /// 同一份输入两次执行因此给出不同的母带。
    ///
    /// 图必须含**真实补偿延迟**, 否则这条判据没有区分力: 这里复用
    /// `pdc_compensation_delays_the_short_branch` 的形状 (短支路补 `L_max` = 96 帧)。
    #[test]
    fn a_second_execution_matches_a_fresh_plan() {
        let master = ulid(0xFFFF);
        let fast = ulid(1);
        let slow_bus = ulid(2);
        let slow_in = ulid(3);
        let routing = graph(
            &[master, fast, slow_bus, slow_in],
            vec![
                edge(fast, master, None),
                edge(slow_bus, master, None),
                edge(slow_in, slow_bus, None),
            ],
        );
        let mut latencies = BTreeMap::new();
        latencies.insert(slow_bus, 96u32);
        let options = RenderOptions::l1(256, 1, 48_000, 0).with_threads(1);
        let sources = || -> BTreeMap<EntityId, Box<dyn AudioSource>> {
            let mut map: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
            map.insert(fast, Box::new(ConstantSource(1.0)));
            map.insert(slow_in, Box::new(ConstantSource(0.0)));
            map
        };

        let mut reused = RenderPlan::compile_with_latencies(&routing, master, options, &latencies)
            .expect("编译");
        let first = reused.execute(sources()).expect("第一次渲染");
        let second = reused.execute(sources()).expect("第二次渲染");

        let mut fresh = RenderPlan::compile_with_latencies(&routing, master, options, &latencies)
            .expect("编译");
        let reference = fresh.execute(sources()).expect("渲染");

        assert_eq!(
            first.samples, reference.samples,
            "第一次执行必须与全新计划一致"
        );
        assert_eq!(
            second.samples, reference.samples,
            "第二次执行必须与全新计划一致 —— 延迟线的环形缓冲与游标必须先复位"
        );
        assert_eq!(second.digest, reference.digest);

        // 延迟真的在这张图里: 前 96 帧里 fast 的贡献是静音。
        assert_eq!(reference.samples[0], 0.0, "延迟线前段必须是静音");
        assert_eq!(reference.samples[96], 1.0, "延迟 96 帧后出现");
    }

    /// 判据 (**复位契约**): 第二次执行**不得沿用**上一次装载的样本源。
    ///
    /// `execute` 的入参是"**这一次**执行用哪些源"。空表意味着一个源都没有, 必须报
    /// [`RenderError::MissingSource`]; 旧实现在 `states[slot].source` 上留着上一次的
    /// 对象, 于是空表静默渲染**上一次的旧源** —— 调用方拿到成功, 却渲染了他没给的东西。
    #[test]
    fn a_second_execution_does_not_reuse_sources_from_the_first() {
        let (routing, master, sources) = star_graph(1);
        let options = RenderOptions::l1(128, 1, 48_000, 0);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        plan.execute(synthetic_sources(&sources, options.seed))
            .expect("第一次渲染");
        let empty: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        assert!(
            matches!(plan.execute(empty), Err(RenderError::MissingSource(_))),
            "空源表必须报 MissingSource, 不许沿用上一次的源"
        );
    }

    /// 判据: 源表被拒绝之后, 计划里**不许残留**任何已装载的源。
    ///
    /// 本判据的图只有一个活节点, 被拒的源表里同时有一个合法源和一个**被剪掉的**
    /// 节点。若"被拒时装载一半"的残留能跨执行存活, 那么随后用空表执行会**静默成功**
    /// (源"还在"); 正确行为是报 [`RenderError::MissingSource`]。
    ///
    /// 兜住这条性质的是每次执行开始时的那一段复位 (源槽清空); `execute` 里"先整表校验、
    /// 再装载"的两遍写法把同一条性质收紧到**调用内部也成立**, 但它**单独不可观测** ——
    /// 实测把两遍合并成一遍时本判据仍然绿 (复位已经兜住了跨执行的那一半)。
    #[test]
    fn a_rejected_source_map_loads_nothing() {
        let master = ulid(0xFFFF);
        let live = ulid(1);
        let dead = ulid(2);
        let routing = graph(&[master, live, dead], vec![edge(live, master, None)]);
        let options = RenderOptions::l1(128, 1, 48_000, 0);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");

        let mut mixed: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        mixed.insert(live, Box::new(ConstantSource(1.0)));
        mixed.insert(dead, Box::new(ConstantSource(1.0)));
        assert_eq!(plan.execute(mixed), Err(RenderError::UnknownNode(dead)));

        let empty: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        assert!(
            matches!(plan.execute(empty), Err(RenderError::MissingSource(_))),
            "被拒的源表不得留下任何已装载的源"
        );
    }

    /// 判据: Master 的归约顺序等于 `EntityId` 字典序, 且**不是**边身份顺序。
    /// 星形图的边身份与源节点序号刻意逆序, 所以这条断言真的在区分两者。
    #[test]
    fn master_reduction_order_is_entity_id_lexicographic() {
        let tracks = 8;
        let (routing, master, sources) = star_graph(tracks);
        let plan = RenderPlan::compile(&routing, master, RenderOptions::l1(128, 2, 48_000, 1))
            .expect("编译");
        let order = plan.bus_reduction_order(master).expect("master 有入边");
        assert_eq!(order.len() as u32, tracks);
        let by_source: Vec<EntityId> = order.iter().map(|entry| entry.source_node).collect();
        let mut expected = sources.clone();
        expected.sort();
        assert_eq!(by_source, expected, "归约顺序必须是源节点 EntityId 字典序");

        // 归约顺序是**复合键** `(source_node, edge_id)` 的全序 —— 只有这个才是被承诺的性质。
        // 单独看边身份**不是**全局升序: 在 star_graph 里两条键的顺序被刻意做成相反,
        // 因此"边身份升序"这条断言本身是错的（第 4 轮 CI 把它抓出来了）。
        let by_key: Vec<(EntityId, EntityId)> = order
            .iter()
            .map(|entry| (entry.source_node, entry.edge_id))
            .collect();
        let mut key_sorted = by_key.clone();
        key_sorted.sort();
        assert_eq!(
            by_key, key_sorted,
            "归约顺序必须按 (source_node, edge_id) 全序"
        );

        // 反向证明: 两种键给出**不同**的序列, 因此上面的断言有区分力。
        let by_edge: Vec<EntityId> = order.iter().map(|entry| entry.edge_id).collect();
        assert_ne!(
            by_source, by_edge,
            "测试图必须让两种键给出不同顺序, 否则判据没有区分力"
        );
    }

    /// 判据: **注入验证的镜像** —— 按"完成顺序"（即输入顺序）归约会给出不同的位型。
    /// 这条证明上面那条属性测试不是真空判据。
    #[test]
    fn completion_order_reduction_is_detectably_different() {
        // 直接对 sum 的归约核做: 同一组贡献, 固定顺序 vs 反序。
        let magnitudes = [1.0e8f32, 0.5, 3.0, 3.0, 1.0e7];
        let buffers: Vec<Vec<f32>> = magnitudes.iter().map(|&m| vec![m]).collect();
        let ordered: Vec<(&[f32], f32)> = buffers.iter().map(|b| (b.as_slice(), 1.0)).collect();
        let mut fixed = vec![0.0f32; 1];
        sum::reduce_ordered(&ordered, &mut fixed);
        let reversed: Vec<(&[f32], f32)> =
            buffers.iter().rev().map(|b| (b.as_slice(), 1.0)).collect();
        let mut by_completion = vec![0.0f32; 1];
        sum::reduce_ordered(&reversed, &mut by_completion);
        assert_ne!(fixed, by_completion, "这组数据无法暴露浮点非结合律");
    }

    /// 判据: 总线把两条支路按固定顺序求和, 结果与手算的定点顺序一致。
    #[test]
    fn bus_sum_matches_the_hand_computed_fixed_order() {
        let master = ulid(0xFFFF);
        let first = ulid(1);
        let second = ulid(2);
        let routing = graph(
            &[master, first, second],
            vec![edge(first, master, None), edge(second, master, Some(6.0))],
        );
        let options = RenderOptions::l1(128, 1, 48_000, 0).with_threads(1);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(first, Box::new(ConstantSource(0.5)));
        sources.insert(second, Box::new(ConstantSource(0.25)));
        let output = plan.execute(sources).expect("渲染");

        // 固定顺序是 (first, second): 0.5 * 1.0 + 0.25 * 10^(6/20)
        let mut expected = [0.0f32; 1];
        sum::accumulate_into(&[0.5], &mut expected, 1.0);
        sum::accumulate_into(&[0.25], &mut expected, db_to_linear(6.0));
        for sample in &output.samples {
            assert_eq!(
                sample.to_bits(),
                expected[0].to_bits(),
                "总线求和顺序或增益不对"
            );
        }
    }

    /// 判据: PDC —— 有延迟的支路不补, 无延迟的支路补 `L_max`。
    /// 延迟由调用方**显式注入**: 模型层 `DeviceDefinition::latency_samples` 是唯一事实源,
    /// 生产路径用 [`track_latencies`] 把它投影成本表; 本判据手写固定值, 免得依赖模型夹具。
    /// （旧注释写"model 层尚无 `latency_samples` 字段"—— 该字段已存在, 见
    /// `DeviceDefinition::latency_samples` 的文档。）
    #[test]
    fn pdc_compensation_delays_the_short_branch() {
        let master = ulid(0xFFFF);
        let fast = ulid(1);
        let slow_bus = ulid(2);
        let slow_in = ulid(3);
        let routing = graph(
            &[master, fast, slow_bus, slow_in],
            vec![
                edge(fast, master, None),
                edge(slow_bus, master, None),
                edge(slow_in, slow_bus, None),
            ],
        );
        let mut latencies = BTreeMap::new();
        latencies.insert(slow_bus, 96u32);
        let mut plan = RenderPlan::compile_with_latencies(
            &routing,
            master,
            RenderOptions::l1(128, 1, 48_000, 0),
            &latencies,
        )
        .expect("编译");
        assert_eq!(plan.longest_path_frames(), 96);

        let order = plan.bus_reduction_order(master).expect("master 有入边");
        let fast_delay = order
            .iter()
            .find(|entry| entry.source_node == fast)
            .map(|entry| entry.delay_frames);
        let slow_delay = order
            .iter()
            .find(|entry| entry.source_node == slow_bus)
            .map(|entry| entry.delay_frames);
        assert_eq!(fast_delay, Some(96), "短支路必须补满 L_max");
        assert_eq!(slow_delay, Some(0), "长支路不补");

        // 渲染仍然成功, 且因为延迟线的作用, 前 96 帧里 fast 的贡献是静音。
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(fast, Box::new(ConstantSource(1.0)));
        sources.insert(slow_in, Box::new(ConstantSource(0.0)));
        let output = plan.execute(sources).expect("渲染");
        assert_eq!(output.samples[0], 0.0, "延迟线前段必须是静音");
        assert_eq!(output.samples[96], 1.0, "延迟 96 帧后出现");
    }

    /// 判据: PDC 的补偿延迟**只插在真正合并支路的求和节点上**。
    ///
    /// `slow_bus` 自身报 96 帧延迟, 但它只有**一条**入边 ⇒ 它不是求和节点, 没有可对齐的
    /// 对象 ⇒ 它的入边不得补偿。旧实现在这条入边上插了 **96 帧**（用 Master 的全局
    /// `L_max` 减源节点的 `output_latency`）, 于是长支路反而多绕一圈, 并在 Master 上
    /// 与其它支路分成两波 —— [ARCH-PDC-001] 要消灭的正是这个。
    ///
    /// 与 `pdc::tests::every_branch_arrives_at_its_summing_node_together` 的分工:
    /// 那一条验**图上的对齐数学**, 这一条验**计划真的把该延迟交给了渲染器**。
    #[test]
    fn pdc_compensation_is_inserted_only_where_branches_merge() {
        let master = ulid(0xFFFF);
        let fast = ulid(1);
        let slow_bus = ulid(2);
        let slow_in = ulid(3);
        let routing = graph(
            &[master, fast, slow_bus, slow_in],
            vec![
                edge(fast, master, None),
                edge(slow_bus, master, None),
                edge(slow_in, slow_bus, None),
            ],
        );
        let mut latencies = BTreeMap::new();
        latencies.insert(slow_bus, 96u32);
        let plan = RenderPlan::compile_with_latencies(
            &routing,
            master,
            RenderOptions::l1(128, 1, 48_000, 0),
            &latencies,
        )
        .expect("编译");
        assert_eq!(plan.longest_path_frames(), 96);

        // 单入边节点: 不补。旧实现这里是 `Some(96)`。
        let interior = plan.bus_reduction_order(slow_bus).expect("slow_bus 有入边");
        assert_eq!(interior.len(), 1, "slow_bus 只有一条入边");
        assert_eq!(interior[0].source_node, slow_in);
        assert_eq!(
            interior[0].delay_frames, 0,
            "单入边节点不是求和节点, 不得插补偿延迟"
        );

        // 求和节点 (Master): 短支路补满, 长支路不补。
        let merging = plan.bus_reduction_order(master).expect("master 有入边");
        let delay_of = |node| {
            merging
                .iter()
                .find(|entry| entry.source_node == node)
                .map(|entry| entry.delay_frames)
        };
        assert_eq!(delay_of(fast), Some(96), "短支路必须补满 L_max");
        assert_eq!(delay_of(slow_bus), Some(0), "长支路不补");
    }

    /// 判据: 悬挂节点 (到不了 Master) 被剪掉, 且不需要为它注册样本源。
    #[test]
    fn nodes_not_reaching_master_are_pruned() {
        let master = ulid(0xFFFF);
        let live = ulid(1);
        let dead = ulid(2);
        let dead_sink = ulid(3);
        let routing = graph(
            &[master, live, dead, dead_sink],
            vec![
                edge(live, master, None),
                edge(dead, dead_sink, None), // 另一端也不连 master
            ],
        );
        let mut plan = RenderPlan::compile(&routing, master, RenderOptions::l1(128, 1, 48_000, 0))
            .expect("编译");
        let kept = plan.nodes();
        assert!(kept.contains(&live));
        assert!(!kept.contains(&dead), "悬挂节点必须被剪掉");
        assert!(!kept.contains(&dead_sink));
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(live, Box::new(ConstantSource(1.0)));
        plan.execute(sources).expect("只给活节点注册源即可");
    }

    /// 判据: 给被剪掉的节点 / 未知节点注册源必须报 `UnknownNode`, 不是 `MissingSource`。
    #[test]
    fn registering_a_pruned_node_is_an_explicit_error() {
        let master = ulid(0xFFFF);
        let live = ulid(1);
        let dead = ulid(2);
        let dead_sink = ulid(3);
        let routing = graph(
            &[master, live, dead, dead_sink],
            vec![edge(live, master, None), edge(dead, dead_sink, None)],
        );
        let mut plan = RenderPlan::compile(&routing, master, RenderOptions::l1(128, 1, 48_000, 0))
            .expect("编译");
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(live, Box::new(ConstantSource(0.0)));
        sources.insert(dead, Box::new(ConstantSource(0.0)));
        assert_eq!(plan.execute(sources), Err(RenderError::UnknownNode(dead)));
    }

    /// 判据: 缺源必须被拒绝; ⭐ 任务单 `#240` 起, **总线可以有自己的源** ——
    /// 源输出与入边在输入端线性相加（Logic Pro 的通道条语义）。
    #[test]
    fn missing_source_is_rejected_and_a_bus_can_carry_its_own_source() {
        let (routing, master, sources) = star_graph(2);
        let options = RenderOptions::l1(128, 1, 48_000, 0);

        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        assert_eq!(
            plan.execute(BTreeMap::new()),
            Err(RenderError::MissingSource(sources[0].min(sources[1])))
        );

        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        registry.insert(sources[0], Box::new(ConstantSource(0.25)));
        registry.insert(sources[1], Box::new(ConstantSource(0.125)));
        // ⭐ 给**有入边**的 master 注册自己的源：这不再被拒绝。
        registry.insert(master, Box::new(ConstantSource(0.5)));
        let output = plan.execute(registry).expect("源 ＋ 入边必须合法");
        assert!(
            !output.samples.is_empty() && output.samples.iter().all(|&s| s == 0.875),
            "输入端求和必须是 0.5（自己的源）＋ 0.25 ＋ 0.125（两条入边）= 0.875"
        );
    }

    /// 判据: 总帧数不是块大小整数倍时, 最后一块按实际帧数切分, 输出长度精确。
    #[test]
    fn partial_last_block_is_split_correctly() {
        let (routing, master, sources) = star_graph(1);
        let options = RenderOptions::l1(300, 2, 48_000, 0).with_threads(1);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        registry.insert(
            sources[0],
            Box::new(RecordingSource {
                inner: ConstantSource(0.25),
                seen: Vec::new(),
            }),
        );
        let output = plan.execute(registry).expect("渲染");
        assert_eq!(output.blocks, 3, "300 / 128 = 3 块（最后一块 44 帧）");
        assert_eq!(
            output.samples.len(),
            300 * 2,
            "输出长度必须精确等于 frames * channels"
        );
        assert!(
            !output.samples.is_empty() && output.samples.iter().all(|&s| s == 0.25),
            "R102/R111"
        );
    }

    /// 判据: 空转图 (Master 没有任何入边, 自己就是唯一源) 也能渲染。
    #[test]
    fn master_only_graph_renders() {
        let master = ulid(0xFFFF);
        let routing = graph(&[master], vec![]);
        let mut plan = RenderPlan::compile(&routing, master, RenderOptions::l1(128, 1, 48_000, 0))
            .expect("编译");
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(master, Box::new(ConstantSource(0.5)));
        let output = plan.execute(sources).expect("渲染");
        assert!(
            !output.samples.is_empty() && output.samples.iter().all(|&s| s == 0.5),
            "R102: 空输出上 `.all` 恒真"
        );
        assert_eq!(plan.longest_path_frames(), 0);
    }

    /// 判据: 源报错会原样传播出来, 不会被吞成"静音输出"。
    #[test]
    fn source_errors_propagate() {
        let (routing, master, sources) = star_graph(1);
        let mut plan = RenderPlan::compile(&routing, master, RenderOptions::l1(128, 1, 48_000, 0))
            .expect("编译");
        let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        registry.insert(sources[0], Box::new(FailingSource));
        assert!(matches!(
            plan.execute(registry),
            Err(RenderError::Source { .. })
        ));
    }

    /// 判据: 有环的路由图被拒绝 (PDC 拓扑分析必须发现环)。
    #[test]
    fn cyclic_graph_is_rejected() {
        let a = ulid(1);
        let b = ulid(2);
        let master = ulid(0xFFFF);
        let routing = graph(
            &[master, a, b],
            vec![edge(a, b, None), edge(b, a, None), edge(b, master, None)],
        );
        assert!(matches!(
            RenderPlan::compile(&routing, master, RenderOptions::l1(128, 1, 48_000, 0)),
            Err(RenderError::Pdc(_))
        ));
    }

    /// 判据: 非法选项与非法路由图都被明确拒绝。
    #[test]
    fn invalid_inputs_are_rejected() {
        let master = ulid(0xFFFF);
        let routing = graph(&[master], vec![]);
        assert_eq!(
            RenderPlan::compile(
                &routing,
                master,
                RenderOptions {
                    channels: 0,
                    ..RenderOptions::l1(128, 1, 48_000, 0)
                }
            )
            .err(),
            Some(RenderError::ZeroChannels)
        );
        assert_eq!(
            RenderPlan::compile(
                &routing,
                master,
                RenderOptions {
                    block_size: 0,
                    ..RenderOptions::l1(128, 1, 48_000, 0)
                }
            )
            .err(),
            Some(RenderError::ZeroBlockSize)
        );
        assert_eq!(
            RenderPlan::compile(&routing, master, RenderOptions::l1(0, 1, 48_000, 0)).err(),
            Some(RenderError::ZeroFrames)
        );
        assert_eq!(
            RenderPlan::compile(&routing, ulid(7), RenderOptions::l1(128, 1, 48_000, 0)).err(),
            Some(RenderError::MasterNotInGraph(ulid(7)))
        );
        // 边指向不存在的节点 -> RoutingGraph::validate 失败
        let broken = graph(&[master], vec![edge(ulid(1), master, None)]);
        assert!(matches!(
            RenderPlan::compile(&broken, master, RenderOptions::l1(128, 1, 48_000, 0)),
            Err(RenderError::InvalidGraph(_))
        ));
    }

    /// 判据 (**参数极值**): 尺寸乘积溢出 `usize` 时必须报错, **不得 panic**。
    ///
    /// 修复前 `block_size = usize::MAX`、`channels = 2` 在 debug 下直接
    /// `attempt to multiply with overflow`（实测 panic 位置是 `compile` 里那一行
    /// `options.block_size * options.channels`）, 在 release 下回绕成一个小缓冲、
    /// 随后在 `buffer[..width]` 上越界。两条都在**编译期**被这条检查消灭。
    ///
    /// 本判据的图是"只有一个 Master 的最小图", 因此除了尺寸之外没有别的拒绝理由。
    #[test]
    fn an_extreme_block_size_is_rejected_instead_of_overflowing() {
        let master = ulid(0xFFFF);
        let routing = graph(&[master], vec![]);
        let extreme = RenderOptions {
            block_size: usize::MAX,
            ..RenderOptions::l1(1, 2, 48_000, 0)
        };
        assert_eq!(
            RenderPlan::compile(&routing, master, extreme).err(),
            Some(RenderError::SizeOverflow {
                what: "block_size * channels"
            })
        );

        // 另一条乘积同族: `usize::MAX` 帧的母带长度也存不下。
        let huge = RenderOptions::l1(u64::MAX, 2, 48_000, 0);
        assert_eq!(
            RenderPlan::compile(&routing, master, huge).err(),
            Some(RenderError::SizeOverflow {
                what: "frames * channels"
            })
        );

        // 有区分力: 同一个图在正常尺寸下编译成功。
        assert!(RenderPlan::compile(&routing, master, RenderOptions::l1(1, 2, 48_000, 0)).is_ok());
    }

    /// 判据: `track_latencies` 累加未旁通设备的延迟, 并跳过旁通设备。
    ///
    /// 语义要点: `latency_samples` 是必需字段（D43）, `0` 就是零延迟, 按 0 参与求和
    /// (保守做法), 而 `bypassed == true` 的设备**完全不产生延迟**。
    #[test]
    fn track_latencies_sum_unbypassed_devices_only() {
        let mut project_tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
        let chain = ulid(0x10);
        project_tracks.insert(
            chain,
            TrackV3 {
                devices: vec![
                    DeviceDefinition {
                        latency_samples: 32,
                        ..DeviceDefinition::default()
                    },
                    DeviceDefinition {
                        latency_samples: 4096,
                        bypassed: true,
                        ..DeviceDefinition::default()
                    },
                    DeviceDefinition {
                        latency_samples: 100,
                        ..DeviceDefinition::default()
                    },
                ],
                ..TrackV3::default()
            },
        );
        let bare = ulid(0x11);
        project_tracks.insert(bare, TrackV3::default());

        let latencies = track_latencies(&project_tracks);
        assert_eq!(latencies[&chain], 132, "32 + 100, 旁通的 4096 被跳过");
        assert_eq!(latencies[&bare], 0, "没有设备就是 0（真的零延迟）");
        assert_eq!(latencies.len(), 2);

        // 缺省（无设备）必须真的走"零延迟、零补偿"那条保守路径。
        let (routing, master, _) = star_graph(2);
        let plan = RenderPlan::compile_with_latencies(
            &routing,
            master,
            RenderOptions::l1(128, 1, 48_000, 0),
            &latencies,
        )
        .expect("编译");
        assert_eq!(plan.longest_path_frames(), 0, "源节点自身没有延迟");
    }

    /// 判据: 分层正确 —— 一条串联链的层号严格递增, 且每层在槽数组里连续。
    #[test]
    fn levels_are_correct_and_contiguous() {
        let master = ulid(0xFFFF);
        let a = ulid(1);
        let b = ulid(2);
        let c = ulid(3);
        let routing = graph(
            &[master, a, b, c],
            vec![edge(a, b, None), edge(b, c, None), edge(c, master, None)],
        );
        let plan = RenderPlan::compile(&routing, master, RenderOptions::l1(128, 1, 48_000, 0))
            .expect("编译");
        let levels = plan.node_levels();
        assert_eq!(levels[&a], 0);
        assert_eq!(levels[&b], 1);
        assert_eq!(levels[&c], 2);
        assert_eq!(levels[&master], 3);
        // 槽位顺序 = (层号, EntityId) 升序, 因此每层是连续区间。
        let nodes = plan.nodes();
        assert_eq!(nodes.len(), 4);
        for (slot, node) in nodes.iter().enumerate() {
            assert_eq!(
                nodes.iter().filter(|n| **n == *node).count(),
                1,
                "槽位 {slot} 重复"
            );
        }
    }

    /// 判据: 多声道 master 的交错写回是整个块从同一槽位拷贝, 声道内容一致。
    #[test]
    fn multichannel_output_is_interleaved_from_the_master_slot() {
        let (routing, master, sources) = star_graph(1);
        let options = RenderOptions::l1(256, 4, 48_000, 0).with_threads(2);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        registry.insert(sources[0], Box::new(ConstantSource(0.125)));
        let output = plan.execute(registry).expect("渲染");
        assert_eq!(output.samples.len(), 256 * 4);
        for frame in output.samples.chunks(4) {
            assert!(
                frame.len() == 4 && frame.iter().all(|&s| s == 0.125),
                "R102/R111 ④值界: 四个声道必须一致"
            );
        }
    }

    /// 逐帧变化的样本源: 每一帧的**所有声道**写入同一个值（值由帧号决定）。
    ///
    /// 与 [`ConstantSource`] 的区别是"同一信号喂给每一路"这件事在**逐帧**上被验证 ——
    /// 常量源下"四路一致"也可能只是"源恰好填了同一个常数"。
    struct FrameRamp;

    impl AudioSource for FrameRamp {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            for (frame, slot) in out.chunks_mut(context.channels).enumerate() {
                let value = (context.first_frame as usize + frame) as f32 * 0.125 - 1.0;
                slot.fill(value);
            }
            Ok(())
        }
    }

    /// 只写**第 0 声道**的样本源: "单声道信号喂进一个立体声节点"在交错缓冲里的形状。
    ///
    /// 它**故意**不碰其余声道 —— [`AudioSource::render_block`] 的契约是"`out` 在调用前
    /// 已被清零", 这条判据就是那条契约的可执行形式。
    struct FirstChannelOnly(f32);

    impl AudioSource for FirstChannelOnly {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            for slot in out.chunks_mut(context.channels) {
                slot[0] = self.0;
            }
            Ok(())
        }
    }

    /// 判据 (**类别 6: 多声道一致性**): 同一个信号喂给左右两路 ⇒ 两路输出**逐位相同**。
    ///
    /// # ⚠ 这条判据**只证明输出对称**, 不证明"本 crate 保证声道对称"（R83 刀①）
    ///
    /// 判据体只比**输出**, 而 `AudioSource::render_block` 拿到的是**交错的原始切片**
    /// —— 源**可以**按声道写不同的值。因此"L/R 对称"是**这个源的性质**, 不是本层的**不变量**。
    /// 状态（每个节点的交错缓冲）**不会**被本层对称化; 证明"不对称会被原样保留"的是
    /// [`a_mono_source_does_not_leak_into_the_other_channel`]（用 `FirstChannelOnly`
    /// 只写第 0 声道 ⇒ 第 1 声道必须逐位是 `+0.0`, 且不得残留上一段的 `0.25`）。
    ///
    /// # R83 刀②: 静态枚举写入者（只有两个, 都不可能**造出**不对称）
    ///
    /// 1. `source.render_block(.., &mut buffer[..width])` —— 源**逐声道**写（因此上面的作用域限定）;
    /// 2. `sum::accumulate_into(upstream, &mut buffer[..width], gain)` —— **逐样本**累加 ＋ **每个汇入节点一个增益标量** ⇒ 上游对称则下游必然对称（同增益联动者不可能造不对称）。
    ///
    /// ⇒ 对**对称的源**, 本层不引入不对称; 对不对称的源, 本层原样保留 —— 两条合起来才是完整契约。
    ///
    /// 信号是**逐帧变化的**（[`FrameRamp`]）, 因此这条判据不只是"常数在四路上相等";
    /// 比较用位型而不是数值 —— `+0.0` 与 `-0.0` 在数值上相等, 在位级判据下才现形。
    #[test]
    fn both_channels_carry_the_same_signal_bit_for_bit() {
        let (routing, master, sources) = star_graph(1);
        let options = RenderOptions::l1(16, 2, 48_000, 0).with_threads(1);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        let output = plan
            .execute(BTreeMap::from([(
                sources[0],
                Box::new(FrameRamp) as Box<dyn AudioSource>,
            )]))
            .expect("渲染");

        assert_eq!(output.samples.len(), 16 * 2);
        let mut values = Vec::new();
        for (index, frame) in output.samples.chunks(2).enumerate() {
            assert_eq!(
                frame[0].to_bits(),
                frame[1].to_bits(),
                "第 {index} 帧的左右两路位型不同: {} vs {}",
                frame[0],
                frame[1]
            );
            values.push(frame[0]);
        }
        // 敏感度自证: 这条判据不是"每一帧都相同"的空判据 —— 帧与帧之间必须不同。
        assert!(
            values.len() >= 2
                && values
                    .windows(2)
                    .any(|pair| pair[0].to_bits() != pair[1].to_bits()),
            "信号在逐帧上是常数 ⇒ 本判据测不到声道错位"
        );
    }

    /// 判据 (**类别 6: 单声道信号喂立体声器件**): 一个只写第 0 声道的源
    /// **不得**把它的值漏进第 1 声道, 也**不得**让第 1 声道读到上一轮执行留在缓冲里的样本。
    ///
    /// 第 1 声道必须是 `+0.0`（位型 `0x0000_0000`）, 不是"数值上等于 0"的 `-0.0`。
    /// 两段都用同一个计划: 第一段把两个声道都填上 `0.25`（把缓冲弄"脏"）,
    /// 第二段只写第 0 声道 —— 因此这条判据同时是"执行之间不留残留"的落点。
    #[test]
    fn a_mono_source_does_not_leak_into_the_other_channel() {
        let (routing, master, sources) = star_graph(1);
        let options = RenderOptions::l1(16, 2, 48_000, 0).with_threads(1);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");

        let filled = plan
            .execute(BTreeMap::from([(
                sources[0],
                Box::new(ConstantSource(0.25)) as Box<dyn AudioSource>,
            )]))
            .expect("第一段渲染");
        assert!(
            !filled.samples.is_empty() && filled.samples.iter().all(|&sample| sample == 0.25),
            "第一段必须把两个声道都填上 0.25, 否则本判据测不到残留"
        );

        let mono = plan
            .execute(BTreeMap::from([(
                sources[0],
                Box::new(FirstChannelOnly(0.5)) as Box<dyn AudioSource>,
            )]))
            .expect("第二段渲染");
        assert_eq!(mono.samples.len(), 16 * 2);
        for (index, frame) in mono.samples.chunks(2).enumerate() {
            assert_eq!(frame[0], 0.5, "第 {index} 帧的第 0 声道");
            assert_eq!(
                frame[1].to_bits(),
                0.0f32.to_bits(),
                "第 {index} 帧的第 1 声道必须是 +0.0（不得是上一段的 0.25, 也不得是 -0.0）"
            );
        }
    }

    /// 判据 (**类别 4: 参数极值**): 同一条轨道上多个设备的延迟累加是**饱和**的 ——
    /// 病态输入（真实和超过 `u32::MAX`）只饱和, **不得**在 debug 下 panic。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `accumulated.saturating_add(device.latency_samples)` 换回普通的 `+` 之后,
    /// 全量判据**全绿**。既有的 `track_latencies_sum_unbypassed_devices_only` 用的是
    /// `32 / 4096（旁通）/ 100`, 和是 132, 远在 `u32` 之内。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一条轨道, 两个**未旁通**设备（单位: 采样帧）, 延迟分别是 `u32::MAX` 与 `1`;
    /// 另一条轨道只有**旁通**的 `u32::MAX` 设备。读数: [`track_latencies`] 的 `u32`
    /// （单位: 帧）。
    ///
    /// # 非空证明
    ///
    /// 真实和 `u32::MAX + 1` 在 `u64` 口径下超过 `u32::MAX`（下面直接断言）; 而旁通那一格
    /// 期望 `0` —— 两格不同, 因此不是"所有输入都给同一个值"。
    #[test]
    fn a_track_latency_sum_saturates_instead_of_overflowing() {
        let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
        let hot = ulid(0x21);
        tracks.insert(
            hot,
            TrackV3 {
                devices: vec![
                    DeviceDefinition {
                        latency_samples: u32::MAX,
                        ..DeviceDefinition::default()
                    },
                    DeviceDefinition {
                        latency_samples: 1,
                        ..DeviceDefinition::default()
                    },
                ],
                ..TrackV3::default()
            },
        );
        let bypassed = ulid(0x22);
        tracks.insert(
            bypassed,
            TrackV3 {
                devices: vec![DeviceDefinition {
                    latency_samples: u32::MAX,
                    bypassed: true,
                    ..DeviceDefinition::default()
                }],
                ..TrackV3::default()
            },
        );

        assert!(
            u64::from(u32::MAX) + 1 > u64::from(u32::MAX),
            "这一格的真实和必须超过 u32, 否则本判据测不到溢出"
        );
        let latencies = track_latencies(&tracks);
        assert_eq!(latencies[&hot], u32::MAX, "u32::MAX + 1 必须饱和, 不得回绕");
        assert_eq!(latencies[&bypassed], 0, "旁通设备完全不产生延迟");
    }

    /// 判据: [`L1_BLOCK_SIZE`] 就是 [ARCH-DET-001] 写死的 **128** 采样点, 而
    /// [`RenderOptions::default`] 必须**是**那一档 L1 配置。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `impl Default for RenderOptions` 里的 `block_size: L1_BLOCK_SIZE` 改成 `64`
    /// 之后, 全量判据**全绿** —— `RenderOptions::default()` 在产线与判据里都**没有**调用点
    /// （`grep -rn 'RenderOptions::default' crates/yeban-render` 无命中）, 既有的每一条
    /// 判据都用 `RenderOptions::l1(..)`。同样的注入对常量
    /// `pub const L1_BLOCK_SIZE: usize = 128;`（改成 64）也全绿。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 常量 [`L1_BLOCK_SIZE`] 与 [`RenderOptions::default`] 的字段。
    /// 读数: 块大小（帧）、线程数（`Option<usize>`）、种子（无单位计数）。
    ///
    /// # 非空证明
    ///
    /// 后半段把 `l1()` 的**显式**参数取成与默认值不同的一档（512 帧 / 4 声道 / 96 kHz /
    /// 种子 7）⇒ 它必须带上 `L1_BLOCK_SIZE` 而不是调用方给的那个数 —— 少了这一半,
    /// "常量改成 512"与"`l1` 把块大小透传成实参"都会让前半段全绿。
    #[test]
    fn the_l1_block_size_and_the_default_options_are_the_spec_shape() {
        assert_eq!(L1_BLOCK_SIZE, 128, "ARCH-DET-001 的固定处理块大小（帧）");
        let default = RenderOptions::default();
        assert_eq!(default.block_size, L1_BLOCK_SIZE, "默认块大小（帧）");
        assert_eq!(default.threads, None, "默认线程数 = 让调度器决定");
        assert_eq!(default.seed, 0, "默认种子");

        // `l1` 无论收到什么, 块大小恒为 L1_BLOCK_SIZE。
        let explicit = RenderOptions::l1(512, 4, 96_000, 7);
        assert_eq!(explicit.block_size, L1_BLOCK_SIZE);
        assert_eq!(explicit.frames, 512);
        assert_eq!(explicit.channels, 4);
        assert_eq!(explicit.sample_rate, 96_000);
        assert_eq!(explicit.seed, 7);
        assert_eq!(explicit.threads, None);
        assert_eq!(explicit.with_threads(3).threads, Some(3));
    }

    /// 判据 (**手写 `Debug` 的形状普查**): [`RenderPlan`] 的 `Debug` 输出是
    /// **本 crate 默认构建里唯一的手写 `Debug`**（`grep 'impl Debug for'` 只命中它）,
    /// 而 `assert_eq!` 的失败消息用的正是 `{:?}` ⇒ 它的形状是诊断面的一部分。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `.field("layers", …)` 改名成 `.field("layer_count", …)`（或整条删掉）之后,
    /// 全量判据**全绿** —— 没有任何判据读过 `format!("{plan:?}")`。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个 4 节点 / 3 层的图（`a → b → master` 与 `c → master`）编译出的计划,
    /// `a` 自身延迟 **100** 帧。读数: `format!("{plan:?}")` 的字符串（字符）,
    /// 加上三个计数（节点数 / 层数 / `L_max`, 单位分别是节点、层、帧）。
    ///
    /// # 非空证明
    ///
    /// 三个数字**互不相同**（4 / 3 / 100）⇒ "把三个字段写成同一个读数"也会红;
    /// 后半段断言首尾括号与四个字段名 ⇒ 改名、删字段、换成 derive 都会红。
    #[test]
    fn the_hand_written_render_plan_debug_shape_is_pinned() {
        let master = ulid(0xFFFF);
        let a = ulid(1);
        let b = ulid(2);
        let c = ulid(3);
        let routing = graph(
            &[master, a, b, c],
            vec![
                edge(a, b, None),
                edge(b, master, None),
                edge(c, master, None),
            ],
        );
        let mut latencies = BTreeMap::new();
        latencies.insert(a, 100u32);
        let plan = RenderPlan::compile_with_latencies(
            &routing,
            master,
            RenderOptions::l1(128, 1, 48_000, 0),
            &latencies,
        )
        .expect("编译");

        let nodes = plan.nodes().len();
        let layers = plan
            .node_levels()
            .values()
            .copied()
            .max()
            .map_or(0, |max| max + 1);
        assert_eq!((nodes, layers, plan.longest_path_frames()), (4, 3, 100));
        assert_ne!(nodes, layers, "本判据要求三个数字互不相同");

        let text = format!("{plan:?}");
        assert!(text.starts_with("RenderPlan { "), "实际 {text}");
        assert!(text.ends_with(" }"), "实际 {text}");
        for field in [
            "nodes: ",
            "layers: ",
            "longest_path_frames: ",
            "options: RenderOptions {",
        ] {
            assert!(text.contains(field), "`Debug` 少了字段 {field:?}: {text}");
        }
        assert!(text.contains("nodes: 4"), "实际 {text}");
        assert!(text.contains("layers: 3"), "实际 {text}");
        assert!(text.contains("longest_path_frames: 100"), "实际 {text}");
        // 非空证明的后半: 三个数字确实互不相同, 因此上面三条断言各自都有射程。
        assert_ne!(plan.longest_path_frames() as usize, nodes);
    }

    /// 判据 (**R58**): [`RenderError`] 的相等必须读**变体与载荷**。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// `invalid_inputs_are_rejected` 用 `assert_eq!(..err(), Some(RenderError::ZeroChannels))`
    /// 这类**整值**比较钉住 6 种拒绝（含 `MasterNotInGraph(ulid(7))` 的**载荷**）。
    /// 把 `RenderError` 的 `==` 削成"只比判别式"（`std::mem::discriminant`）之后,
    /// **6 条断言全部继续通过** —— 载荷（哪个 ULID、哪个 `what`、哪条消息）无人看管。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 7 对 `RenderError`（单位: 一个错误）。读数: `==` / `!=` 的判决。
    ///
    /// # 非空证明
    ///
    /// 成对的取值**既有跨变体**（`ZeroChannels` vs `ZeroBlockSize`）**也有同变体不同载荷**
    /// （两个不同 ULID、两个不同 `what`、两条不同消息）⇒ "只比判别式"这种改法必红。
    #[test]
    fn the_render_error_equality_reads_every_variant_and_payload() {
        let pairs: [(&str, RenderError, RenderError); 7] = [
            (
                "跨变体 1",
                RenderError::ZeroChannels,
                RenderError::ZeroBlockSize,
            ),
            (
                "跨变体 2",
                RenderError::ZeroBlockSize,
                RenderError::ZeroFrames,
            ),
            (
                "同变体的 ULID 载荷",
                RenderError::MasterNotInGraph(ulid(7)),
                RenderError::MasterNotInGraph(ulid(8)),
            ),
            (
                "同变体的 what 载荷",
                RenderError::SizeOverflow {
                    what: "block_size * channels",
                },
                RenderError::SizeOverflow {
                    what: "frames * channels",
                },
            ),
            (
                "同变体的消息载荷",
                RenderError::InvalidGraph("环".to_owned()),
                RenderError::InvalidGraph("端点缺失".to_owned()),
            ),
            (
                "Source 的消息载荷",
                RenderError::Source {
                    node: ulid(1),
                    message: "a".to_owned(),
                },
                RenderError::Source {
                    node: ulid(1),
                    message: "b".to_owned(),
                },
            ),
            (
                "Source 的节点载荷",
                RenderError::Source {
                    node: ulid(1),
                    message: "a".to_owned(),
                },
                RenderError::Source {
                    node: ulid(2),
                    message: "a".to_owned(),
                },
            ),
        ];
        assert_eq!(pairs.len(), 7);
        for (label, left, right) in &pairs {
            assert_ne!(left, right, "{label}: 两个不同取值必须不相等");
            assert_ne!(right, left, "{label}: 不等必须对称");
            assert_eq!(*left, left.clone(), "{label}: 自反");
        }
        // 非空证明（**R58 的"同变体、不同载荷"**）: 探针不能只是"判别式先判完"。
        // 这里**按判别式**逐对分类: 同变体的那 5 对必须由**载荷**把它们分开。
        let same_variant: Vec<&(&str, RenderError, RenderError)> = pairs
            .iter()
            .filter(|(_, a, b)| core::mem::discriminant(a) == core::mem::discriminant(b))
            .collect();
        assert_eq!(
            same_variant.len(),
            5,
            "夹具里必须恰有 5 对**同变体不同载荷**（跨变体的那 2 对不算数）"
        );
        for (label, a, b) in same_variant {
            assert_ne!(
                a, b,
                "{label}: 同变体的**载荷**必须不同, 否则探针只测到判别式"
            );
        }
        // 非空证明: 另外 2 对确实是**跨变体**的（所以"只比判别式"的 `==` 也过不了它们）。
        let cross_variant = pairs
            .iter()
            .filter(|(_, a, b)| core::mem::discriminant(a) != core::mem::discriminant(b))
            .count();
        assert_eq!(cross_variant, 2, "另有 2 对跨变体");
    }
}
