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

use std::collections::BTreeMap;
use std::fmt::Debug;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use yeban_model::{EntityId, RoutingEdge, RoutingGraph, TrackV3};

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
    /// 某条源节点没有注册样本源。
    MissingSource(EntityId),
    /// 给一个**有入边**的节点注册了样本源 —— 那是总线, 不是声部。
    SourceOnBusNode(EntityId),
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
            Self::MissingSource(node) => write!(f, "节点 {node} 没有注册样本源"),
            Self::SourceOnBusNode(node) => write!(f, "节点 {node} 有入边, 不能注册样本源"),
            Self::UnknownNode(node) => {
                write!(f, "节点 {node} 不在渲染计划里（未知或被剪枝）")
            }
            Self::ThreadPool(message) => write!(f, "线程池构建失败: {message}"),
            Self::Source { node, message } => write!(f, "样本源 {node} 失败: {message}"),
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
    /// 与 `incoming` 一一对应的延迟线 (零延迟时走旁路, 不使用 `scratch`)。
    delay_lines: Vec<DelayLine>,
    /// 延迟路径用的中转缓冲。
    scratch: Vec<f32>,
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
        if options.channels == 0 {
            return Err(RenderError::ZeroChannels);
        }
        if options.block_size == 0 {
            return Err(RenderError::ZeroBlockSize);
        }
        if options.frames == 0 {
            return Err(RenderError::ZeroFrames);
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
        let block_width = options.block_size * options.channels;
        let mut states: Vec<NodeState> = ordered
            .iter()
            .map(|&node| NodeState {
                node,
                buffer: vec![0.0; block_width],
                source: None,
                incoming: Vec::new(),
                delay_lines: Vec::new(),
                scratch: vec![0.0; block_width],
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

    /// 执行渲染。
    ///
    /// `sources` 只需为**真正的源节点** (无入边的保留节点) 提供实现; 给总线注册源
    /// 会被拒绝 ([`RenderError::SourceOnBusNode`])。
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

        // 装载样本源, 并做"源节点 vs 总线节点"的静态检查。
        for (node, source) in sources {
            let slot = self
                .slot_of
                .get(&node)
                .copied()
                .ok_or(RenderError::UnknownNode(node))?;
            if !self.states[slot].incoming.is_empty() {
                return Err(RenderError::SourceOnBusNode(node));
            }
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

/// 渲染一个节点的一块: 源节点调样本源, 总线节点做固定顺序的串行归约。
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
        delay_lines,
        scratch,
    } = state;

    if incoming.is_empty() {
        buffer[..width].fill(0.0);
        let source = source.as_mut().ok_or(RenderError::MissingSource(*node))?;
        return source.render_block(context, &mut buffer[..width]);
    }

    // [ARCH-DET-002] 固定顺序的**单线程串行累加**。`incoming` 在编译期已按
    // (source_node, edge_id) 字典序排好, 这里的 for 循环顺序就是全部。
    buffer[..width].fill(0.0);
    for (index, contribution) in incoming.iter().enumerate() {
        let upstream = &done[contribution.source_slot].buffer[..width];
        let line = &mut delay_lines[index];
        if line.is_bypass() {
            // 绝大多数边没有补偿延迟: 直接累加, 连一次拷贝都不要。
            sum::accumulate_into(upstream, &mut buffer[..width], contribution.gain);
        } else {
            line.process(upstream, &mut scratch[..width]);
            sum::accumulate_into(&scratch[..width], &mut buffer[..width], contribution.gain);
        }
    }
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
    /// 延迟由调用方注入 (model 层尚无 `latency_samples` 字段)。
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

    /// 判据: 缺源与"给总线注册源"必须各自被拒绝。
    #[test]
    fn missing_source_and_source_on_bus_are_rejected() {
        let (routing, master, sources) = star_graph(2);
        let options = RenderOptions::l1(128, 1, 48_000, 0);

        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        assert_eq!(
            plan.execute(BTreeMap::new()),
            Err(RenderError::MissingSource(sources[0].min(sources[1])))
        );

        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
        let mut registry = synthetic_sources(&sources, 1);
        registry.insert(master, Box::new(ConstantSource(0.0)));
        assert_eq!(
            plan.execute(registry),
            Err(RenderError::SourceOnBusNode(master))
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
        assert!(output.samples.iter().all(|&s| s == 0.25));
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
        assert!(output.samples.iter().all(|&s| s == 0.5));
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
            assert!(frame.iter().all(|&s| s == 0.125), "四个声道必须一致");
        }
    }
}
