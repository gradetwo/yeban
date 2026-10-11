//! 内部 PDC（Plugin Delay Compensation）拓扑与环形延迟线。
//! [ARCH-PDC-001, ARCH-PDC-002, ROAD-M2-004, ARCH-DET-002]
//!
//! ## 算法（规范 §3.4 的机械化形式）
//!
//! 1. **拓扑排序**：对 `yeban-model` 的 [`RoutingGraph`] 做 Kahn 排序。
//!    排序键是 `EntityId`（ULID，字典序 = 创建序）[MODEL-AST-003]，
//!    因此同层节点的处理顺序是**跨进程、跨重启确定**的 —— 这是
//!    [ARCH-DET-002] "串行归约确定性铁律" 的前提。
//!    成环图返回 [`PdcError::Cycle`]，**绝不死循环、绝不 panic**。
//! 2. **关键路径**：`L(v)` = 从任一信号源到达 `v` **输入端**的最长累积延迟；
//!    `arrival(v) = L(v) + own_latency(v)` 是信号离开 `v` 输出端的时刻。
//!    关键路径延迟 `L_max = L(master)` —— **求和节点输入端**的最长支路延迟
//!    （规范 §3.4 第 2 条："从每个信号源到 Master 总线的所有声学通路…确定最长延迟
//!    关键路径 $L_{\max}$"）[ARCH-PDC-001]。
//!    ⚠ **不是** `arrival(master)`：`master` 自身的处理延迟发生在**求和之后**
//!    （母线限制器的 33 帧），它是**引擎输出延迟**的一部分，见
//!    [`PdcPlan::output_latency`]。把 `arrival(master)` 当作 `L_max` 会给**每一条**
//!    支路白加 33 帧延迟线，而对齐一个相位也补不回来。
//! 3. **延迟分配**：对每条可达 master 的**支路**插入
//!    `D(v) = L_max - arrival(v)` 个采样点的环形延迟。
//!
//! ### 定义性不变量（这是 PDC 的判据，不是实现细节）
//!
//! ```text
//! ∀ v ∈ Reachable(master) \ {master}:  arrival(v) + D(v) == L_max
//! ```
//!
//! 即"所有分支经过延迟线后总延迟相等"。任何并行支路在汇入求和节点时相位完全对齐。
//! `master` 自己被排除在外：它是**求和节点本身**，它的自身延迟在求和**之后**，
//! 不属于任何支路的相位（`D(master)` 因此恒为 0）。
//! 该不变量在测试里被机械断言（见 `mod tests` 的 `pdc_alignment_invariant_*`）。
//!
//! ## 与离线渲染共用
//!
//! [`PdcPlan::compute`] 是**不依赖 cpal 的纯函数**：输入 `RoutingGraph` + 延迟表，
//! 输出纯数据。`yeban-render` 的 Rayon 离线母带渲染器直接复用它，从而保证
//! "实时与离线绝对相位对齐"（规范 §3.4 第 3 条）。
//!
//! ## 节点延迟的来源（**两处，且只有这两处**）
//!
//! [ARCH-PDC-001] 要求"每个插件与内置设备必须精确上报其引入的处理延迟
//! (`DeviceDefinition::latency_samples`)"。该字段在 `yeban-model` 里是**必需字段**
//! [ADR-0001 D43]：**缺字段由反序列化直接报错**，因此不存在"未上报"这个状态 ——
//! 读到的 `0` **就是**"这台设备真的零延迟"。
//! 本 crate 用 [`LatencyTable::from_project`] / [`LatencyTable::from_tracks`]
//! 从**设备链**汇总每个节点的自身延迟：未旁通设备的 `latency_samples` 饱和求和。
//!
//! 第二处是**引擎自己知道**的一段固定延迟：母线总线限制器的
//! [`BUS_LIMITER_LATENCY_FRAMES`](crate::mixer::BUS_LIMITER_LATENCY_FRAMES)（33 帧）。
//! [ADR-0001 D44(b)] 要求它"必须回填进 `LatencyTable`"（"引擎链路上每一段延迟都可被
//! PDC 看见"）⇒ [`crate::snapshot::EngineSnapshot::from_project`] 在**构造期**用
//! [`LatencyTable::add`] 把它加到 `master` 节点上。它接在总线求和**之后**，
//! 因此只进 [`PdcPlan::output_latency`]、**不**产生任何 `D(v)`。
//!
//! [`LatencyTable`] 仍然可以显式注入（[`PdcPlan::compute`] 接收它），
//! 供离线对账/测量注入使用；但**默认路径**永远走上面这两处。

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use yeban_model::{EntityId, RoutingGraph, TrackV3, YebanProjectV1};

/// PDC 计算的错误类型。
///
/// 只派生 `PartialEq`/`Eq`（不含 `f32`），因此测试可以直接对错误做断言。
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PdcError {
    /// 指定的 master 节点不在 `RoutingGraph::nodes` 里。
    #[error("PDC: master node is not present in routing graph nodes")]
    UnknownMaster {
        /// 传入的 master 身份。
        master: EntityId,
    },

    /// 某条边的端点不在 `RoutingGraph::nodes` 里。
    ///
    /// `RoutingGraph::validate()` 也会拒绝这种图；这里再查一遍是因为
    /// [`PdcPlan::compute`] 不假设调用方已经 `validate()` 过（离线渲染器可能
    /// 直接从反序列化结果构造）。
    #[error("PDC: routing edge endpoint is not present in routing graph nodes")]
    DanglingEdge {
        /// 缺失的节点身份。
        node: EntityId,
    },

    /// 路由图成环。`nodes` 是拓扑排序**无法消化**的节点（按键升序，确定性）。
    ///
    /// 返回错误而不是死循环是硬要求 —— 无效输入（损毁工程 / 恶意 MCP 请求）
    /// 不得让引擎挂死。
    #[error("PDC: routing graph contains a cycle ({} unresolved nodes)", nodes.len())]
    Cycle {
        /// 参与环（或依赖环）的节点，按键升序。
        nodes: Vec<EntityId>,
    },
}

/// 每个节点的**自身处理延迟**（采样点）[ARCH-PDC-001]。
///
/// 键是节点身份（音轨/总线），值是该节点内部（设备链、限幅器前瞻等）引入的延迟。
/// 用 `BTreeMap` 而不是 `HashMap`：迭代顺序必须确定 [红线 4 / MODEL-AST-003]。
///
/// 缺省值语义：**未登记的节点延迟为 0**（`get` 不返回 `Option`）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LatencyTable {
    entries: BTreeMap<EntityId, u32>,
}

impl LatencyTable {
    /// 空表（全部节点延迟为 0）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记/覆盖一个节点的处理延迟。
    pub fn set(&mut self, node: EntityId, latency_samples: u32) {
        if latency_samples == 0 {
            self.entries.remove(&node);
        } else {
            self.entries.insert(node, latency_samples);
        }
    }

    /// 读取一个节点的处理延迟（未登记为 0）。
    #[must_use]
    pub fn get(&self, node: &EntityId) -> u32 {
        self.entries.get(node).copied().unwrap_or(0)
    }

    /// 在节点**已有**自身延迟上追加一段（饱和相加；`extra_samples == 0` 是空操作）。
    ///
    /// 用途：设备链之外的、**引擎自己知道**的一段固定延迟 —— 目前唯一的调用点是
    /// 母线总线限制器的前瞻环长（[ADR-0001 D44(b)]，见
    /// [`crate::mixer::BUS_LIMITER_LATENCY_FRAMES`]）。
    /// 模型的 `DeviceDefinition::latency_samples` 仍然是**设备**延迟的唯一事实源；
    /// 本方法只做"把两段延迟落在同一个节点上"这一件事，不引入第三个来源。
    pub fn add(&mut self, node: EntityId, extra_samples: u32) {
        if extra_samples == 0 {
            return;
        }
        let current = self.get(&node);
        self.set(node, current.saturating_add(extra_samples));
    }

    /// 已登记的条目数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有任何非零条目。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按键升序迭代（确定性）。
    pub fn iter(&self) -> impl Iterator<Item = (&EntityId, &u32)> {
        self.entries.iter()
    }

    /// 从整个工程投影出延迟表 [ARCH-PDC-001]。
    ///
    /// **设备**延迟的唯一来源：`DeviceDefinition::latency_samples`（模型层的权威字段）。
    /// 本函数**只**做设备链汇总；引擎自己知道的那一段固定延迟（母线限制器的 33 帧）
    /// 由 [`crate::snapshot::EngineSnapshot::from_project`] 随后用 [`Self::add`] 追加
    /// —— 两处来源在 `graph` 模块文档里逐条登记。
    #[must_use]
    pub fn from_project(project: &YebanProjectV1) -> Self {
        Self::from_tracks(&project.tracks)
    }

    /// 从工程音轨集合投影出延迟表 [ARCH-PDC-001]。
    ///
    /// 节点的自身延迟 = 该节点设备链上所有**未旁通**设备的 `latency_samples` **之和**
    /// （饱和相加，避免恶意工程用 `u32::MAX` 造出回绕）。
    ///
    /// 两个刻意的语义决定：
    ///
    /// 1. **旁通设备不计入**：`bypassed == true` 表示该设备不在信号路径上，
    ///    它对相位没有贡献。若将来发现某些宿主仍然报告旁通设备的延迟，只改这一处。
    /// 2. **`0` 就是"真的零延迟"**：`DeviceDefinition::latency_samples` 是**必需字段**
    ///    [ADR-0001 D43]，**缺字段由 `yeban-model` 在反序列化时报错** ⇒ "未上报"这个状态
    ///    不存在，本函数没有必要（也没有能力）去区分"漏报"与"零延迟"。
    ///    它只做一件事：把设备链上的上报值加起来。
    #[must_use]
    pub fn from_tracks(tracks: &BTreeMap<EntityId, TrackV3>) -> Self {
        let mut table = Self::new();
        for (id, track) in tracks {
            let mut node_latency = 0u32;
            for device in &track.devices {
                if !device.bypassed {
                    node_latency = node_latency.saturating_add(device.latency_samples);
                }
            }
            table.set(*id, node_latency);
        }
        table
    }
}

/// 一次 PDC 计算的结果：拓扑序 + 每节点累积延迟 + 每节点补偿延迟。
///
/// 纯数据、可 `Clone`、可放进 [`crate::snapshot::EngineSnapshot`] 并被 `Arc` 共享。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdcPlan {
    order: Vec<EntityId>,
    latency: BTreeMap<EntityId, u32>,
    arrival: BTreeMap<EntityId, u32>,
    compensation: BTreeMap<EntityId, u32>,
    excluded: Vec<EntityId>,
    total_latency: u32,
    output_latency: u32,
}

impl PdcPlan {
    /// 计算 PDC 计划 [ARCH-PDC-001]。
    ///
    /// - `graph`：唯一路由事实源 [MODEL-AST-004]（不要求预先 `validate()`）；
    /// - `master`：主总线节点身份（来自 `YebanProjectV1::master_bus_track_id`）；
    /// - `latencies`：各节点自身处理延迟。
    ///
    /// # Errors
    ///
    /// - [`PdcError::UnknownMaster`]：`master` 不在 `graph.nodes` 里；
    /// - [`PdcError::DanglingEdge`]：有边的端点不在 `graph.nodes` 里；
    /// - [`PdcError::Cycle`]：图成环（返回无法归约的节点集合，**不死循环**）。
    pub fn compute(
        graph: &RoutingGraph,
        master: EntityId,
        latencies: &LatencyTable,
    ) -> Result<Self, PdcError> {
        let nodes: BTreeSet<EntityId> = graph.nodes.iter().copied().collect();
        if !nodes.contains(&master) {
            return Err(PdcError::UnknownMaster { master });
        }

        // --- 邻接表（BTreeMap ⇒ 迭代确定；同一对节点的重边去重） ---
        let mut successors: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        let mut predecessors: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for edge in graph.edges.values() {
            for endpoint in [edge.source_node, edge.destination_node] {
                if !nodes.contains(&endpoint) {
                    return Err(PdcError::DanglingEdge { node: endpoint });
                }
            }
            successors
                // R215③（如实登记）：本函数内 insert（7 处，**单值存储** ⇒ 后者胜）与这里的
                // entry(..).or_default().push(..)（2 处，**累积** ⇒ 两条都留、顺序保持）并存。
                // 二者**不是同一情形的两种政策**，差异是**故意的**（遍历确定性依赖累积顺序）。
                .entry(edge.source_node)
                .or_default()
                .push(edge.destination_node);
            predecessors
                .entry(edge.destination_node)
                .or_default()
                .push(edge.source_node);
        }
        for list in successors.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        for list in predecessors.values_mut() {
            list.sort_unstable();
            list.dedup();
        }

        // --- 1. Kahn 拓扑排序（键序 ⇒ 确定性） ---
        let mut indegree: BTreeMap<EntityId, usize> = BTreeMap::new();
        for node in &nodes {
            indegree.insert(*node, predecessors.get(node).map_or(0, Vec::len));
        }
        let mut ready: BTreeSet<EntityId> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(node, _)| *node)
            .collect();
        let mut order: Vec<EntityId> = Vec::with_capacity(nodes.len());
        while let Some(&node) = ready.iter().next() {
            ready.remove(&node);
            order.push(node);
            if let Some(next) = successors.get(&node) {
                for successor in next {
                    if let Some(degree) = indegree.get_mut(successor) {
                        *degree = degree.saturating_sub(1);
                        if *degree == 0 {
                            ready.insert(*successor);
                        }
                    }
                }
            }
        }
        if order.len() != nodes.len() {
            let unresolved: Vec<EntityId> = nodes
                .iter()
                .copied()
                .filter(|node| !order.contains(node))
                .collect();
            return Err(PdcError::Cycle { nodes: unresolved });
        }

        // --- 2. 关键路径：L(v) 与 arrival(v) ---
        // L(v) = max over 前驱 p of arrival(p)；arrival(v) = L(v) + own_latency(v)。
        // 注意 arrival(p) **已经包含** p 自身的处理延迟，不能再加一次。
        let mut latency: BTreeMap<EntityId, u32> = BTreeMap::new();
        let mut arrival: BTreeMap<EntityId, u32> = BTreeMap::new();
        for node in &order {
            let mut longest_input = 0u32;
            if let Some(sources) = predecessors.get(node) {
                for source in sources {
                    let candidate = arrival.get(source).copied().unwrap_or(0);
                    if candidate > longest_input {
                        longest_input = candidate;
                    }
                }
            }
            latency.insert(*node, longest_input);
            arrival.insert(*node, longest_input.saturating_add(latencies.get(node)));
        }
        // `L_max` = 求和节点**输入端**的最长支路延迟 = `L(master)`（规范 §3.4 第 2 条）。
        // `master` 自身的处理延迟（母线限制器的 33 帧）发生在求和**之后**，因此它
        // **不参与** `D(v)` 的对齐基准 —— 算进去只会给每条支路白加 N 帧延迟线，
        // 一个相位也补不回来。它进 [`PdcPlan::output_latency`]（引擎输出的固定后移）。
        let total_latency = latency.get(&master).copied().unwrap_or(0);
        let output_latency = arrival.get(&master).copied().unwrap_or(0);

        // --- 3. 只对"能到达 master"的子图分配补偿延迟 ---
        // 若 p → v 且 v 能到达 master, 则 p 也能到达 master, 因此该子图对前驱封闭,
        // 于是 ∀v ∈ Reachable \ {master}: arrival(v) <= L_max,
        // D(v) = L_max - arrival(v) 不会下溢。
        // `master` 自己：`arrival(master) = L_max + own_latency(master) >= L_max`
        // ⇒ `saturating_sub` 恒得 0（求和节点不需要、也不能有延迟线）。
        let mut reachable: BTreeSet<EntityId> = BTreeSet::new();
        let mut stack: Vec<EntityId> = vec![master];
        reachable.insert(master);
        while let Some(node) = stack.pop() {
            if let Some(sources) = predecessors.get(&node) {
                for source in sources {
                    if reachable.insert(*source) {
                        stack.push(*source);
                    }
                }
            }
        }

        let mut compensation: BTreeMap<EntityId, u32> = BTreeMap::new();
        for node in &reachable {
            let node_arrival = arrival.get(node).copied().unwrap_or(0);
            compensation.insert(*node, total_latency.saturating_sub(node_arrival));
        }
        let excluded: Vec<EntityId> = nodes
            .iter()
            .copied()
            .filter(|node| !reachable.contains(node))
            .collect();

        Ok(Self {
            order,
            latency,
            arrival,
            compensation,
            excluded,
            total_latency,
            output_latency,
        })
    }

    /// 拓扑序（节点身份，全部节点；确定性）。
    #[must_use]
    pub fn order(&self) -> &[EntityId] {
        &self.order
    }

    /// `L(v)`：到达 `v` 输入端的最长累积延迟。未在图中则返回 `None`。
    #[must_use]
    pub fn latency(&self, node: &EntityId) -> Option<u32> {
        self.latency.get(node).copied()
    }

    /// `arrival(v) = L(v) + own_latency(v)`；`v` 不可达 master 时返回 `None`。
    #[must_use]
    pub fn arrival(&self, node: &EntityId) -> Option<u32> {
        if self.compensation.contains_key(node) {
            self.arrival.get(node).copied()
        } else {
            None
        }
    }

    /// `D(v) = L_max - arrival(v)`；`v` 不可达 master 时返回 `None`（无对齐意义）。
    #[must_use]
    pub fn compensation(&self, node: &EntityId) -> Option<u32> {
        self.compensation.get(node).copied()
    }

    /// 关键路径总延迟 `L_max` = 求和节点**输入端**的最长支路延迟 [ARCH-PDC-001]。
    ///
    /// 它是 `D(v)` 的对齐基准，**不包含** `master` 自身的处理延迟（求和之后的
    /// 母线限制器 33 帧）。需要"引擎输出相对工程时间轴后移多少"请读
    /// [`Self::output_latency`]。
    #[must_use]
    pub fn total_latency(&self) -> u32 {
        self.total_latency
    }

    /// 引擎**输出**总延迟：`arrival(master) = L_max + own_latency(master)`（采样点）。
    ///
    /// 它是"喂进引擎第 0 帧的信号在第几帧出现在输出"的那个数 —— 既含支路对齐用的
    /// `L_max`，也含接在**总线求和之后**的 `master` 自身延迟
    /// （母线限制器的 [`BUS_LIMITER_LATENCY_FRAMES`](crate::mixer::BUS_LIMITER_LATENCY_FRAMES)
    /// = 33 帧，[ADR-0001 D44(b)]）。
    ///
    /// ⚠ 它**不是** [`Self::total_latency`]：后者只是支路对齐基准。
    /// 当 `master` 没有自身延迟时两者相等（历史行为）。
    #[must_use]
    pub fn output_latency(&self) -> u32 {
        self.output_latency
    }

    /// 不参与 master 求和（因此无需补偿）的节点，按键升序。
    #[must_use]
    pub fn excluded(&self) -> &[EntityId] {
        &self.excluded
    }

    /// 需要插入延迟线的节点数。
    #[must_use]
    pub fn compensated_len(&self) -> usize {
        self.compensation.len()
    }
}

/// 预分配的单通道环形延迟线 [ARCH-PDC-002]。
///
/// - 缓冲在构造期一次性分配（`Vec<f32>`），`process*` **零分配、零锁、无分支预测悬崖**
///   [ARCH-RT-001]；
/// - `delay == 0` 是显式直通快路径（**不读**旧样本，避免"延迟 0 却读到陈旧值"的经典
///   bug）—— 注意它仍然**写**（见下面的不变量）；
/// - 索引按下标取模而非 `%` 运算，避免每样本一次除法。
///
/// ## 环的**不变量**：它是输入端最近 `capacity − 1` 个样本的连续记录
///
/// `process*` 每处理一帧都把那帧写进环 —— `delay == 0` 时也写，只是不做抽头读。
/// 于是"写头之前 `delay` 格"永远是"`delay` 帧之前的那个输入样本"。这条不变量是
/// 两件事的前提：
///
/// 1. **换延迟**只改抽头距离 ⇒ 输出永远是新延迟下的输入，不会吐出"上一次非零延迟
///    留下、此后从未被读出的陈音频"（[ARCH-DET-001] 要求可复现的输出，而"吐什么"
///    取决于上一次调过什么延迟，那不是可复现的输出）；
/// 2. **清线**（[`Self::reset`]）之后，本线与一个全新的 `DelayLine::new(capacity)`
///    加 `set_delay(delay)` **逐样本逐位相同**。
///
/// ⚠ 少了"`delay == 0` 也写"这一条，环会在直通期间**冻结**在直通之前的时刻：
/// 之后延迟再变正，抽头读到的就是那段陈音频（可能来自几秒前），而不是零历史。
/// 这与 `crate::rt` 逐轨循环里"停住时也喂延迟线 —— 不推进它会让恢复播放时吐出
/// 上一次停住前的陈音频"是同一条理由，只是这里冻结的触发条件是 `delay == 0`。
///
/// ## 代价与**被否决的替代方案**（写进代码，因为这是取舍不是疏忽）
///
/// 记录意味着 `delay == 0` 的直通分支不再是"整块 O(1) 直接返回"，而是每帧一次
/// `buffer[write] = sample` 加一次写头推进（**没有设备链延迟的工程里全部节点的
/// `D(v)` 都是 0** ⇒ 那条分支对每条轨都跑）。换来的回报是"把延迟开正"那一刻输出
/// **无缝**变成 `x(t − delay)`（正是 PDC 要的对齐，没有空档）。
///
/// 替代方案是"直通期间不记录，但在 `delay` 由 0 变正时清线"：稳态零代价，代价是
/// 那一刻**吞掉 `delay` 帧**（`delay = 400` 时是 8 ms 的空档）。两者都会动到一段
/// 音频（插入延迟这件事本身如此），但"吞掉 8 ms"是一条听得见的断口，而"重复最近
/// `delay` 帧"只是相位跳变 —— 与 `crate::rt` 已有的"停住时也喂线"同口径，故选前者。
#[derive(Clone, Debug)]
pub struct DelayLine {
    buffer: Vec<f32>,
    write: usize,
    delay: usize,
}

impl DelayLine {
    /// 构造一条最大延迟为 `max_delay_samples` 的延迟线。
    ///
    /// 缓冲长度为 `max_delay_samples + 1`，因此 `delay == max_delay_samples` 也是合法的
    /// （读写指针在环上恰好相差 `max_delay_samples` 格）。
    #[must_use]
    pub fn new(max_delay_samples: usize) -> Self {
        let capacity = max_delay_samples.saturating_add(1).max(1);
        Self {
            buffer: vec![0.0; capacity],
            write: 0,
            delay: 0,
        }
    }

    /// 缓冲格数（`= max_delay_samples + 1`）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// 当前延迟（采样点）。
    #[must_use]
    pub fn delay(&self) -> usize {
        self.delay
    }

    /// 设置延迟，返回钳制后的实际值（上界 `capacity() - 1`，**不 panic**）。
    pub fn set_delay(&mut self, samples: usize) -> usize {
        self.delay = samples.min(self.buffer.len() - 1);
        self.delay
    }

    /// 清零缓冲内容并把写头归零（**保留延迟设置**）。
    ///
    /// ## 语义口径：清完之后与一个**全新实例**逐位相同
    ///
    /// 结果状态 = `DelayLine::new(capacity)` 加 `set_delay(delay)` 两条调用之后的
    /// 状态：缓冲全零、写头 `0`。因此接下来的 `delay` 个输出样本是**逐位零**，
    /// 之后是本线自己的输入 —— 环里**不留**任何旧抽头（这正是
    /// [`CompensationBank::rearm`] 在槽位换主人时要它的理由）。
    ///
    /// 代价是一次 `fill`（零分配、零锁、零 I/O [MUST-GATE-001]）⇒ 只在
    /// **配置变更**时调用，绝不进逐样本路径。
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write = 0;
    }

    /// 是否处于直通状态（`delay == 0`）。
    #[must_use]
    pub fn is_passthrough(&self) -> bool {
        self.delay == 0
    }

    /// 就地处理一个块：`buf[i] = buf[i - delay]`。
    ///
    /// 因为是就地环形推进，块边界天然连续 —— PDC 的对齐在任意块长下都成立，
    /// 不要求 `buf.len()` 等于渲染量子。
    ///
    /// ⚠ `delay == 0` 时输出**逐位等于输入**，但输入仍然写进环：见类型文档的
    /// "环的不变量"。少了这次写，之后把延迟开正就会重放陈音频。
    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }
        let capacity = self.buffer.len();
        let mut write = self.write;
        if self.delay == 0 {
            for sample in buf.iter() {
                self.buffer[write] = *sample;
                write += 1;
                if write == capacity {
                    write = 0;
                }
            }
            self.write = write;
            return;
        }
        let mut read = (write + capacity - self.delay) % capacity;
        for sample in buf.iter_mut() {
            let delayed = self.buffer[read];
            self.buffer[write] = *sample;
            *sample = delayed;
            write += 1;
            if write == capacity {
                write = 0;
            }
            read += 1;
            if read == capacity {
                read = 0;
            }
        }
        self.write = write;
    }

    /// 处理 `input` 写入 `output`（按较短者长度工作，不 panic）。
    ///
    /// `delay == 0` 的分支与 [`Self::process_in_place`] 同款：输出逐位等于输入，
    /// 但输入仍然写进环（见类型文档的不变量）。
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let capacity = self.buffer.len();
        let mut write = self.write;
        if self.delay == 0 {
            for (out, inp) in output.iter_mut().zip(input) {
                *out = *inp;
                self.buffer[write] = *inp;
                write += 1;
                if write == capacity {
                    write = 0;
                }
            }
            self.write = write;
            return;
        }
        let mut read = (write + capacity - self.delay) % capacity;
        for (out, inp) in output.iter_mut().zip(input) {
            let delayed = self.buffer[read];
            self.buffer[write] = *inp;
            *out = delayed;
            write += 1;
            if write == capacity {
                write = 0;
            }
            read += 1;
            if read == capacity {
                read = 0;
            }
        }
        self.write = write;
    }
}

/// 一次 PDC 计划对应的延迟线集合（每个需要补偿的节点一条）。
///
/// 构造期完成全部 `Vec` 分配；[`apply`](Self::apply) 走 `binary_search` 定位，
/// **处理期零分配** [ARCH-RT-001]。
///
/// ## 两条构造路径（离线 vs 实时）
///
/// | 路径 | 构造 | 重新武装 |
/// | :--- | :--- | :--- |
/// | 离线（`from_plan`） | 按计划**精确**分配：每条线容量恰为 `D_i + 1` | 不需要（一次渲染一份计划） |
/// | 实时（`preallocated` + [`rearm`](Self::rearm)） | 构造期按上限**预分配**定长池 | 快照边界只写 `set_delay` ⇒ 零分配 |
///
/// 实时路径不能按计划分配：计划来自**控制线程**、随时可能变大，而回调内分配是
/// [MUST-GATE-001] 的一票否决。代价是池有一条**容量上限**，超出时
/// [`rearm`](Self::rearm) 不静默丢样本，而是把差额回报给调用方（`EngineRuntime`
/// 把它记进 `EngineStats` 的 `pdc_unarmed_nodes` / `pdc_clamped_frames`）。
#[derive(Clone, Debug)]
pub struct CompensationBank {
    /// 按键升序（`PdcPlan::compensation` 是 `BTreeMap`，天然有序）。
    ///
    /// `preallocated` 之后本 `Vec` 的长度固定不变（**绝不在处理期增长或截断**：
    /// 增长会分配、`truncate` 会 `Drop` 延迟线 ⇒ `dealloc`），只有**前 `armed` 项**
    /// 参与查找。
    entries: Vec<(EntityId, DelayLine)>,
    /// 已武装的条数（`entries[..armed]` 有效且按键升序）。
    armed: usize,
    /// **见证计数**：槽位**换了主人**的累计次数（每个"绑定变更·槽位"记 1）。
    ///
    /// 它是"重绑分支真的被走到"的机械证据。重绑是**唯一**会清延迟线的路径，而
    /// "走过重绑"与"没走过"在分配/锁/I/O 读数上完全一样（两支都不分配）⇒ 没有
    /// 这个数，零分配判据对那条新分支就是**盲的**（可以删掉它而全绿）。
    /// 与 [`Self::processed_blocks`] 同族：只增不减，`wrapping_add` 无分支。
    rebindings: u64,
    /// **见证计数**：`apply` 真的施加过**非零**延迟的"节点·块"次数。
    ///
    /// 只增不减、`delay == 0` 的直通不计。它是"延迟线真的在信号路径上"的机械证据：
    /// 「武装了」不等于「被调用了」，而一个被武装却从不被调用的 bank 在音频读数上
    /// 就是一个**静默的**相位错位。递增是 `wrapping_add(u64::from(..))`，**无分支**、
    /// **零分配**，仍满足 [ARCH-RT-001]。
    processed_blocks: u64,
}

/// [`CompensationBank::rearm`] 未能精确兑现的补偿量。
///
/// **正常恒为全零**。非零意味着实时侧的定长池装不下这一份计划：调用方必须把它
/// 记成可读的读数，而不是当作"补偿成功"。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RearmShortfall {
    /// 计划里需要补偿、但没有分到延迟线的节点数（池的槽位用尽）。
    pub unarmed_nodes: usize,
    /// 被容量钳制的延迟点数之和（`wanted - actual`，采样点）。
    pub clamped_frames: u64,
}

impl RearmShortfall {
    /// 是否**逐点精确**兑现了计划。
    #[must_use]
    pub const fn is_exact(&self) -> bool {
        self.unarmed_nodes == 0 && self.clamped_frames == 0
    }
}

impl CompensationBank {
    /// 按 PDC 计划构造延迟线集合（**离线路径**：容量精确匹配计划）。
    #[must_use]
    pub fn from_plan(plan: &PdcPlan) -> Self {
        let mut entries: Vec<(EntityId, DelayLine)> = Vec::with_capacity(plan.compensated_len());
        for (node, samples) in plan.compensation.iter() {
            let mut line = DelayLine::new(*samples as usize);
            line.set_delay(*samples as usize);
            entries.push((*node, line));
        }
        let armed = entries.len();
        Self {
            entries,
            armed,
            rebindings: 0,
            processed_blocks: 0,
        }
    }

    /// 预分配 `slots` 条上限为 `max_delay_samples` 的延迟线（**实时路径**）。
    ///
    /// 构造期一次分配；之后只用 [`rearm`](Self::rearm) 重新绑定节点与延迟，
    /// 因此回调内零分配。初始状态一条都没武装（`len() == 0`）。
    #[must_use]
    pub fn preallocated(slots: usize, max_delay_samples: usize) -> Self {
        let entries = (0..slots)
            .map(|_| (EntityId::default(), DelayLine::new(max_delay_samples)))
            .collect();
        Self {
            entries,
            armed: 0,
            rebindings: 0,
            processed_blocks: 0,
        }
    }

    /// 用一份新计划重新武装（**零分配**：只写节点键、`set_delay` 与"换主人时清线"）。
    ///
    /// 武装顺序就是计划里 `compensation` 的迭代顺序（`BTreeMap` 按键升序），因此
    /// `entries[..armed]` 始终保持按键升序 —— [`apply`](Self::apply) 的
    /// `binary_search` 前提由此成立。
    ///
    /// ## 槽位绑定变更时必须清线
    ///
    /// 槽位是**按下标**绑定的（"计划里的第 i 个键"→"第 i 条延迟线"），而计划里的键
    /// 就是**能到达母线**的节点集合 ⇒ 工程里删掉一条轨（或任何节点）会让它后面的
    /// 每个节点**各下移一格**。那条线里留着上一任节点的输入历史与写头位置
    /// （[`DelayLine`] 的不变量）⇒ 新主人会先把**另一个节点的音频**播出来，最长
    /// `delay` 帧。修法是"换主人就 [`DelayLine::reset`]"：清完之后那条线与一个全新
    /// 实例逐位相同。同一张表在 `crate::rt` 里的三个姊妹实现（通道条 / 混响 /
    /// 卷积混响）都按"身份变了就复位或重建"处理，这里是那张表的第四条。
    ///
    /// 这一支是**逐修订**的（不是逐样本），只做一次 `fill` 加两个标量写：
    /// 零分配、零锁、零 I/O [MUST-GATE-001]。稳态（节点集合不变）下一次都不跑 ——
    /// 那时 `slot_node` 与计划键逐位相同。
    ///
    /// 两个上限（都由构造期容量决定，**都不静默**）：
    /// 1. 槽位不够 ⇒ 多出来的节点进 [`RearmShortfall::unarmed_nodes`]；
    /// 2. 延迟超过线容量 ⇒ 钳到容量上界（[`DelayLine::set_delay`] 的语义），
    ///    差额进 [`RearmShortfall::clamped_frames`]。
    pub fn rearm(&mut self, plan: &PdcPlan) -> RearmShortfall {
        let mut shortfall = RearmShortfall::default();
        let mut armed = 0usize;
        let mut rebindings = 0u64;
        for (node, &wanted) in plan.compensation.iter() {
            if armed >= self.entries.len() {
                shortfall.unarmed_nodes += 1;
                continue;
            }
            let wanted = wanted as usize;
            let (slot_node, line) = &mut self.entries[armed];
            if *slot_node != *node {
                line.reset();
                *slot_node = *node;
                rebindings = rebindings.wrapping_add(1);
            }
            let actual = line.set_delay(wanted);
            if actual != wanted {
                shortfall.clamped_frames += (wanted - actual) as u64;
            }
            armed += 1;
        }
        self.armed = armed;
        self.rebindings = self.rebindings.wrapping_add(rebindings);
        shortfall
    }

    /// 预分配的槽位数（与武装了多少条无关）。
    #[must_use]
    pub fn slots(&self) -> usize {
        self.entries.len()
    }

    /// 已武装的延迟线数量（= 参与了补偿的节点数）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.armed
    }

    /// 见 [`Self::processed_blocks`] 字段（累计；采样点级不动，每个"节点·块"一次）。
    #[must_use]
    pub const fn processed_blocks(&self) -> u64 {
        self.processed_blocks
    }

    /// 见 [`Self::rebindings`] 字段（累计；每个"槽位换主人"一次）。
    ///
    /// 用途是**覆盖度见证**：零分配/零锁/零 I/O 判据必须能回答"那条清线分支真的
    /// 被走到了吗"。调用方（`crate::rt`）把它转成
    /// [`EngineRuntime::pdc_rebindings`](crate::rt::EngineRuntime::pdc_rebindings)。
    #[must_use]
    pub const fn rebindings(&self) -> u64 {
        self.rebindings
    }

    /// 是否没有武装任何延迟线。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.armed == 0
    }

    /// 本 bank 中最大的延迟线容量（预分配用）。
    #[must_use]
    pub fn max_capacity(&self) -> usize {
        self.entries
            .iter()
            .map(|(_, line)| line.capacity())
            .max()
            .unwrap_or(0)
    }

    /// 对 `node` 的输出块施加补偿延迟。
    ///
    /// 返回 `true` 表示该节点确实有一条延迟线（即使是直通），`false` 表示该节点
    /// 不在本计划内（调用方应视为"无需补偿"）。
    pub fn apply(&mut self, node: &EntityId, buf: &mut [f32]) -> bool {
        match self.entries[..self.armed].binary_search_by(|(candidate, _)| candidate.cmp(node)) {
            Ok(index) => {
                let line = &mut self.entries[index].1;
                self.processed_blocks = self
                    .processed_blocks
                    .wrapping_add(u64::from(line.delay() != 0));
                line.process_in_place(buf);
                true
            }
            Err(_) => false,
        }
    }

    /// 只读访问某个节点的延迟线（未武装的槽位不可达）。
    #[must_use]
    pub fn line(&self, node: &EntityId) -> Option<&DelayLine> {
        self.entries[..self.armed]
            .binary_search_by(|(candidate, _)| candidate.cmp(node))
            .ok()
            .map(|index| &self.entries[index].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // R215③ 便宜臂：本文件 `.insert(`（单值存储）与 `.entry(..).or_default().push(..)`（累积）
    // 并存；本臂断言**两侧政策各自正确且故意不同**（⛔ 不改语义，只增判据）。
    #[test]
    fn duplicate_identity_policies_are_deliberate_and_documented() {
        use std::collections::HashMap;
        // ① 单值存储：同键两次 insert ⇒ 后者胜、条数不增。
        let mut single: HashMap<u32, u32> = HashMap::new();
        single.insert(1, 10);
        single.insert(1, 20);
        assert_eq!(
            single.len(),
            1,
            "单值存储：同键重复写不得增加条数（覆盖语义）"
        );
        assert_eq!(single[&1], 20, "单值存储：后者胜");
        // ② 累积：同键两次 entry(..).or_default().push(..) ⇒ 两条都在、顺序保持。
        let mut list: HashMap<u32, Vec<u32>> = HashMap::new();
        for value in [7u32, 8u32] {
            list.entry(1).or_default().push(value);
        }
        assert_eq!(
            list[&1],
            vec![7, 8],
            "累积语义：同键重复 push 必须保留两条且顺序不变"
        );
        assert_eq!(list.len(), 1, "累积语义：键仍只有一个");
    }
    use yeban_model::{RoutingEdge, RoutingKind};

    /// 用 (源, 目标) 列表构造一个合法的 `RoutingGraph`（自动补 `nodes` 与边身份）。
    fn graph_of(edges: &[(EntityId, EntityId)]) -> RoutingGraph {
        let mut graph = RoutingGraph::default();
        let mut nodes: BTreeSet<EntityId> = BTreeSet::new();
        for (source, destination) in edges {
            nodes.insert(*source);
            nodes.insert(*destination);
            let id = EntityId::new();
            graph.edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: *source,
                    destination_node: *destination,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            );
        }
        graph.nodes = nodes.into_iter().collect();
        graph
    }

    fn latency_table(entries: &[(EntityId, u32)]) -> LatencyTable {
        let mut table = LatencyTable::new();
        for (node, samples) in entries {
            table.set(*node, *samples);
        }
        table
    }

    /// 判据 (b)：成环图返回**明确错误**，而不是死循环 / panic。
    #[test]
    fn cyclic_graph_returns_error_and_never_hangs() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        // 三角形环: a → b → c → a
        let graph = graph_of(&[(a, b), (b, c), (c, a)]);
        let error =
            PdcPlan::compute(&graph, a, &LatencyTable::new()).expect_err("成环图必须返回 Err");
        match error {
            PdcError::Cycle { nodes } => {
                let mut expected = vec![a, b, c];
                expected.sort_unstable();
                assert_eq!(nodes, expected, "环上三个节点都必须是未归约集合");
                // 按键升序 (确定性), 便于日志与 UI 稳定显示
                let mut sorted = nodes.clone();
                sorted.sort_unstable();
                assert_eq!(nodes, sorted);
            }
            other => panic!("预期 Cycle, 实际 {other:?}"),
        }
        // 自环也必须被抓住
        let graph = graph_of(&[(a, a)]);
        assert!(matches!(
            PdcPlan::compute(&graph, a, &LatencyTable::new()),
            Err(PdcError::Cycle { .. })
        ));
    }

    /// 判据 (c)：PDC 的定义性不变量 —— 所有分支经过延迟线后总延迟相等。
    ///
    /// 图: `a → b (b 自身 10 采样) → m`, `a → c (c 自身 0) → m`。
    #[test]
    fn pdc_alignment_invariant_holds_for_diamond() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, b), (b, m), (a, c), (c, m)]);
        let latencies = latency_table(&[(b, 10)]);
        let plan = PdcPlan::compute(&graph, m, &latencies).expect("合法 DAG");

        assert_eq!(plan.total_latency(), 10, "L_max 由长支路 b(10) 决定");
        // 逐节点机械断言 arrival(v) + D(v) == L_max
        for node in plan.order() {
            if let (Some(arrival), Some(compensation)) =
                (plan.arrival(node), plan.compensation(node))
            {
                assert_eq!(
                    arrival + compensation,
                    plan.total_latency(),
                    "节点 {node:?} 未对齐到关键路径"
                );
            }
        }
        // 具体数值
        assert_eq!(plan.compensation(&a), Some(10));
        assert_eq!(plan.compensation(&b), Some(0));
        assert_eq!(plan.compensation(&c), Some(10));
        assert_eq!(plan.compensation(&m), Some(0));
        assert!(plan.excluded().is_empty());
    }

    /// 判据 (d)：`master` 自身的处理延迟（母线限制器的 33 帧）**可见但不参与对齐**。
    ///
    /// [ADR-0001 D44(b)] 要求母线限制器的 33 帧回填进 `LatencyTable`。它接在
    /// **总线求和之后**，因此：
    ///
    /// - `total_latency()`（对齐基准 `L_max = L(master)`）**不变**；
    /// - 每条支路的 `D(v)` **逐位不变** ⇒ 渲染输出一个字都不动；
    /// - `output_latency()`（`arrival(master)`）**恰好增加** `own` 帧。
    ///
    /// 注入：把 `compute` 里的 `total_latency` 换回 `arrival.get(&master)`
    /// ⇒ 支路 `D(v)` 全部 +33、`total_latency` 也变，本判据立刻红
    /// （`tests/pdc_mix_path.rs` 的 P3 会在**样本级**变红）。
    #[test]
    fn master_own_latency_is_visible_to_pdc_but_never_delays_branches() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, b), (b, m), (a, c), (c, m)]);

        // 参照：master 自身延迟为 0（回填之前的历史口径）。
        let reference = PdcPlan::compute(&graph, m, &latency_table(&[(b, 10)])).expect("合法 DAG");
        assert_eq!(reference.total_latency(), 10);
        assert_eq!(reference.output_latency(), 10);

        // 被测：master 自身多 33 帧（回填之后的口径）。
        let backfilled =
            PdcPlan::compute(&graph, m, &latency_table(&[(b, 10), (m, 33)])).expect("合法 DAG");

        assert_eq!(
            backfilled.total_latency(),
            10,
            "L_max 是**求和节点输入端**的最长支路延迟，与 master 自身延迟无关"
        );
        assert_eq!(
            backfilled.output_latency(),
            43,
            "引擎输出延迟 = L_max + master 自身延迟(33)"
        );
        assert_eq!(backfilled.output_latency(), backfilled.total_latency() + 33);

        // 对齐量逐位相同：这两条断言就是"渲染输出不变"的纯数值形式。
        for node in [a, b, c, m] {
            assert_eq!(
                backfilled.compensation(&node),
                reference.compensation(&node),
                "节点 {node:?} 的 D(v) 必须一位不动（33 帧在求和之后）"
            );
        }
        assert_eq!(
            backfilled.compensation(&m),
            Some(0),
            "求和节点自己没有延迟线"
        );
        // 不变量在支路上仍成立；master 靠 saturating_sub 恰好落回 0。
        for node in [a, b, c] {
            assert_eq!(
                backfilled.arrival(&node).expect("可达")
                    + backfilled.compensation(&node).expect("可达"),
                backfilled.total_latency(),
                "支路 {node:?} 未对齐到 L_max"
            );
        }
        assert_eq!(backfilled.compensated_len(), reference.compensated_len());
    }

    /// 判据 (c) 的行为面：把同一脉冲喂进两条支路，补偿后**采样级同相**。
    ///
    /// 这比纯数值断言更强：它证明 `DelayLine` 的读写指针约定与 `D_i` 的语义一致
    /// （把 `read` 的偏移量写错能让数值断言依然通过，但这条会红）。
    #[test]
    fn compensated_branches_line_up_sample_exactly() {
        let a = EntityId::new();
        let long = EntityId::new();
        let short = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, long), (long, m), (a, short), (short, m)]);
        let own = 10u32;
        let latencies = latency_table(&[(long, own)]);
        let plan = PdcPlan::compute(&graph, m, &latencies).expect("合法 DAG");
        let mut bank = CompensationBank::from_plan(&plan);
        assert_eq!(plan.compensation(&long), Some(0), "长支路已在关键路径上");
        assert_eq!(plan.compensation(&short), Some(own), "短支路补足差距");

        let quantum = 32usize;
        // 长支路自身引入 own 采样的处理延迟（模拟插件前瞻缓冲）。
        let mut branch_delay = DelayLine::new(own as usize + 1);
        branch_delay.set_delay(own as usize);

        let mut long_arrivals: Vec<usize> = Vec::new();
        let mut short_arrivals: Vec<usize> = Vec::new();

        for block in 0..4usize {
            let mut long_out = [0.0f32; 32];
            let mut short_out = [0.0f32; 32];
            if block == 0 {
                long_out[0] = 1.0; // 同一脉冲注入两条支路
                short_out[0] = 1.0;
            }
            // 长支路的"设备链"延迟
            let mut processed = [0.0f32; 32];
            branch_delay.process(&long_out, &mut processed);
            long_out = processed;

            // 两条支路都经过各自的 PDC 补偿延迟线
            assert!(bank.apply(&long, &mut long_out), "长支路有补偿条目(0)");
            assert!(bank.apply(&short, &mut short_out), "短支路有补偿条目(10)");

            for i in 0..quantum {
                if long_out[i] != 0.0 {
                    long_arrivals.push(block * quantum + i);
                }
                if short_out[i] != 0.0 {
                    short_arrivals.push(block * quantum + i);
                }
            }
        }
        assert_eq!(
            long_arrivals,
            vec![own as usize],
            "长支路的脉冲应出现在全局第 10 个采样点"
        );
        assert_eq!(
            short_arrivals, long_arrivals,
            "短支路必须被延迟到完全相同的采样点 —— 这就是 PDC 的全部意义"
        );
    }

    /// 不可达 master 的孤立子图被显式列出，而不是被静默塞一个下溢的延迟。
    #[test]
    fn unreachable_nodes_are_excluded_not_compensated() {
        let a = EntityId::new();
        let m = EntityId::new();
        // p → q 是一条不与 master 相连的孤立链
        let p = EntityId::new();
        let q = EntityId::new();
        let graph = graph_of(&[(a, m), (p, q)]);
        let plan = PdcPlan::compute(&graph, m, &LatencyTable::new()).expect("合法 DAG");
        assert_eq!(plan.compensation(&a), Some(0));
        assert_eq!(plan.compensation(&m), Some(0));
        assert_eq!(plan.compensation(&p), None, "不可达节点没有补偿语义");
        let mut expected = vec![p, q];
        expected.sort_unstable();
        assert_eq!(plan.excluded(), expected.as_slice());
        // 孤立子图不影响关键路径
        assert_eq!(plan.total_latency(), 0);
    }

    #[test]
    fn unknown_master_and_dangling_edge_are_errors() {
        let a = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, m)]);
        let stranger = EntityId::new();
        assert_eq!(
            PdcPlan::compute(&graph, stranger, &LatencyTable::new()),
            Err(PdcError::UnknownMaster { master: stranger })
        );

        // 手工构造一条端点缺失的边
        let mut broken = graph.clone();
        let ghost = EntityId::new();
        let id = EntityId::new();
        broken.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: ghost,
                destination_node: m,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        assert_eq!(
            PdcPlan::compute(&broken, m, &LatencyTable::new()),
            Err(PdcError::DanglingEdge { node: ghost })
        );
    }

    #[test]
    fn master_only_graph_is_zero_latency() {
        let m = EntityId::new();
        let mut graph = RoutingGraph::default();
        graph.nodes.push(m);
        let plan = PdcPlan::compute(&graph, m, &LatencyTable::new()).expect("单节点图合法");
        assert_eq!(plan.total_latency(), 0);
        assert_eq!(plan.compensation(&m), Some(0));
        assert_eq!(plan.order().to_vec(), vec![m]);
    }

    #[test]
    fn serial_chain_accumulates_latency_in_order() {
        let a = EntityId::new();
        let b = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, b), (b, m)]);
        let latencies = latency_table(&[(a, 4), (b, 6)]);
        let plan = PdcPlan::compute(&graph, m, &latencies).expect("链合法");
        // arrival(m) = 4 + 6 = 10
        assert_eq!(plan.total_latency(), 10);
        assert_eq!(plan.arrival(&a), Some(4));
        assert_eq!(plan.arrival(&b), Some(10));
        // 串联链上所有节点都在关键路径上 ⇒ 无需补偿
        assert_eq!(plan.compensation(&a), Some(6));
        assert_eq!(plan.compensation(&b), Some(0));
    }

    #[test]
    fn delay_line_is_transparent_at_zero_and_shifts_at_n() {
        let mut line = DelayLine::new(8);
        assert!(line.is_passthrough());
        let mut buf = [1.0f32, 2.0, 3.0, 4.0];
        line.process_in_place(&mut buf);
        assert_eq!(buf, [1.0, 2.0, 3.0, 4.0], "delay 0 必须直通");

        let mut line = DelayLine::new(8);
        assert_eq!(line.set_delay(3), 3);
        let mut buf = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        line.process_in_place(&mut buf);
        assert_eq!(
            buf,
            [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            "delay 3: 前 3 个样本是（零初始的）历史, 之后逐个顺移"
        );
        // 跨块连续: 继续推进必须保持同一相位
        let mut more = [10.0f32, 11.0];
        line.process_in_place(&mut more);
        assert_eq!(more, [7.0, 8.0]);
    }

    #[test]
    fn delay_line_clamps_oversized_delay_without_panic() {
        let mut line = DelayLine::new(4);
        assert_eq!(line.capacity(), 5);
        assert_eq!(line.set_delay(4096), 4, "延迟被钳到 capacity-1");
        let mut buf = [0.0f32; 6];
        line.process_in_place(&mut buf); // 不 panic 即通过
        // 零容量延迟线也必须是安全直通
        let mut zero = DelayLine::new(0);
        assert_eq!(zero.capacity(), 1);
        assert_eq!(zero.set_delay(7), 0);
        let mut buf = [1.0f32, 2.0];
        zero.process_in_place(&mut buf);
        assert_eq!(buf, [1.0, 2.0]);
    }

    #[test]
    fn delay_line_process_handles_mismatched_lengths() {
        let input = [1.0f32, 2.0, 3.0, 4.0];

        // delay 0 → 纯拷贝
        let mut line = DelayLine::new(4);
        line.set_delay(0);
        let mut output = [9.0f32; 4];
        line.process(&input, &mut output);
        assert_eq!(output, input);

        // delay 2 → 前两个样本是（零初始的）历史
        let mut line = DelayLine::new(4);
        line.set_delay(2);
        let mut output = [9.0f32; 4];
        line.process(&input, &mut output);
        assert_eq!(output, [0.0, 0.0, 1.0, 2.0]);

        // 输出比输入短: 只处理较短者, 其余输出槽位保持原值（不 panic）
        let mut short = [9.0f32; 2];
        line.process(&input, &mut short);
        assert_eq!(short, [3.0, 4.0], "跨调用相位必须连续");

        // 输入比输出长: 只消费前 2 个输入样本
        let mut line = DelayLine::new(4);
        line.set_delay(2);
        let mut out2 = [9.0f32; 2];
        line.process(&input, &mut out2);
        assert_eq!(out2, [0.0, 0.0]);
    }

    #[test]
    fn latency_table_treats_unregistered_nodes_as_zero() {
        let a = EntityId::new();
        let mut table = LatencyTable::new();
        assert!(table.is_empty());
        assert_eq!(table.get(&a), 0);
        table.set(a, 32);
        assert_eq!(table.get(&a), 32);
        assert_eq!(table.len(), 1);
        table.set(a, 0);
        assert!(table.is_empty(), "显式置 0 等价于未登记（语义：延迟为 0）");
    }

    /// 判据：`LatencyTable::add` 在**已有**值上饱和相加，且 `0` 是空操作。
    ///
    /// 注入：把 `add` 写成 `set`（覆盖而不是相加）⇒ 第二段断言红；
    /// 去掉饱和（改成 `+`）⇒ 溢出在 debug 下 panic，判据以 panic 变红。
    #[test]
    fn latency_table_add_accumulates_saturating() {
        let a = EntityId::new();
        let mut table = LatencyTable::new();
        table.add(a, 0);
        assert!(table.is_empty(), "加 0 不产生条目（未登记就是 0）");
        table.add(a, 32);
        assert_eq!(table.get(&a), 32);
        table.add(a, 8);
        assert_eq!(table.get(&a), 40, "两段延迟落在同一个节点上");
        table.add(a, u32::MAX);
        assert_eq!(table.get(&a), u32::MAX, "饱和，不回绕");
    }

    /// 判据 (i)：延迟表从模型的 `DeviceDefinition::latency_samples` 汇总而来
    /// [ARCH-PDC-001]，且**旁通设备不计入**；`0` 就是"真的零延迟"
    /// （必需字段 [ADR-0001 D43]，缺字段由模型层反序列化报错）。
    #[test]
    fn latency_table_sums_model_device_latency_and_skips_bypassed() {
        use yeban_model::{DeviceDefinition, DeviceKind};
        let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
        let id = EntityId::new();
        let mut track = TrackV3 {
            id,
            ..TrackV3::default()
        };
        // 串联两个设备: 32 + 8 = 40
        track.devices.push(DeviceDefinition {
            kind: DeviceKind::InternalEffect,
            latency_samples: 32,
            ..DeviceDefinition::default()
        });
        track.devices.push(DeviceDefinition {
            kind: DeviceKind::ExternalEffect,
            latency_samples: 8,
            ..DeviceDefinition::default()
        });
        // 旁通设备即使上报了延迟也不计入
        track.devices.push(DeviceDefinition {
            kind: DeviceKind::ExternalEffect,
            latency_samples: 4096,
            bypassed: true,
            ..DeviceDefinition::default()
        });
        tracks.insert(id, track);

        // 另一条轨道: 设备显式上报 0（= 真的零延迟）⇒ 该节点延迟为 0
        let silent = EntityId::new();
        let mut silent_track = TrackV3 {
            id: silent,
            ..TrackV3::default()
        };
        silent_track.devices.push(DeviceDefinition {
            kind: DeviceKind::InternalInstrument,
            ..DeviceDefinition::default()
        });
        tracks.insert(silent, silent_track);

        let table = LatencyTable::from_tracks(&tracks);
        assert_eq!(table.get(&id), 40, "32 + 8（旁通的 4096 不计入）");
        assert_eq!(table.get(&silent), 0, "真的零延迟 ⇒ 0");
        assert_eq!(table.len(), 1, "0 不保留条目（set(_, 0) == 未登记）");
    }

    /// 实时路径的重新武装：预分配池按计划绑定节点与延迟，**只写 `set_delay`**。
    ///
    /// 变红的注入：`rearm` 里改成 `self.entries = from_plan(plan).entries`（重新分配）
    /// ⇒ 实时窗口的分配判据（`rt_zero_alloc` ⑰）红；把 `set_delay(wanted)` 写成
    /// 常量 0 ⇒ 下面的 `line(a).delay()` 断言红；把 `self.armed = armed` 写成
    /// `self.entries.len()`（把空槽也算进查找范围）⇒ 空槽的 `EntityId::default()`
    /// 会插在有序键**前面**，下面"陌生节点必须返回 `None`/`false`"的断言红。
    ///
    /// 槽位数**刻意多于**计划条目数（6 > 4）：两者相等时"把空槽也算进查找范围"
    /// 这个 bug 观察不到 —— 第一版正是 4 槽 4 条，注入之后仍然全绿。
    #[test]
    fn preallocated_bank_rearms_to_the_plan_and_looks_up_by_key() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, b), (b, m), (a, c), (c, m)]);
        let plan = PdcPlan::compute(&graph, m, &latency_table(&[(b, 10)])).expect("合法 DAG");

        let mut bank = CompensationBank::preallocated(6, 64);
        assert!(bank.is_empty(), "预分配之后一条都没武装");
        assert_eq!(bank.slots(), 6);
        let shortfall = bank.rearm(&plan);
        assert!(
            shortfall.is_exact(),
            "4 个节点 6 条线，必须逐点精确：{shortfall:?}"
        );
        assert_eq!(bank.len(), plan.compensated_len());
        assert_eq!(bank.slots(), 6, "武装不改变池的大小");
        assert_eq!(bank.line(&a).map(DelayLine::delay), Some(10));
        assert_eq!(bank.line(&b).map(DelayLine::delay), Some(0));
        assert_eq!(bank.line(&c).map(DelayLine::delay), Some(10));
        assert_eq!(bank.line(&m).map(DelayLine::delay), Some(0));

        // 空槽必须**不可达**（否则按键查找会命中没武装的槽位）。
        let stranger = EntityId::new();
        assert!(
            bank.line(&stranger).is_none(),
            "未武装的槽位不得被查到（其键是 `EntityId::default()`）"
        );

        // 行为面：短支路被延后 10 帧，长支路（delay 0）逐位不变。
        let mut short = [1.0f32; 4];
        assert!(bank.apply(&c, &mut short));
        assert_eq!(short, [0.0, 0.0, 0.0, 0.0], "延迟线的历史是零");
        assert_eq!(
            bank.processed_blocks(),
            1,
            "只有**非零**延迟的那一次算『真的处理过』"
        );
        let mut long = [1.0f32, 2.0, 3.0, 4.0];
        assert!(bank.apply(&b, &mut long));
        assert_eq!(long, [1.0, 2.0, 3.0, 4.0], "delay 0 是显式直通");
        assert_eq!(
            bank.processed_blocks(),
            1,
            "delay == 0 的直通不计入见证（否则见证会掩盖『延迟线从未施加延迟』）"
        );

        // 不在计划里的节点：`apply` 返回 false（调用方视为无需补偿）。
        assert!(!bank.apply(&stranger, &mut long));

        // 二次武装（换计划）不改变槽位数，只改绑定：同一份计划逐点仍然精确。
        let other = PdcPlan::compute(&graph, m, &latency_table(&[(b, 3)])).expect("合法 DAG");
        let again = bank.rearm(&other);
        assert!(again.is_exact());
        assert_eq!(bank.slots(), 6, "重新武装绝不改变预分配池的大小");
        assert_eq!(bank.line(&a).map(DelayLine::delay), Some(3));
        assert_eq!(bank.line(&b).map(DelayLine::delay), Some(0));
        assert!(bank.line(&stranger).is_none());
    }

    /// **槽位换主人时必须清线**：新主人不得把上一任的音频播出来。
    ///
    /// 槽位是**按计划键序的下标**绑定的（`compensation` 是按键升序的 `BTreeMap`），
    /// 因此"谁在哪个槽"由身份大小决定 ⇒ 本判据先把四个身份**排序**再分配角色，
    /// 把"槽位会怎么挪"变成可预测的事实（不依赖 ULID 的偶然顺序）。
    ///
    /// 图（键序 = 身份升序）：`short_a(0) → master`、`short_b(0) → master`、
    /// `slow(上报 32) → master` ⇒ `L_max = 32`、`D(short_a) = D(short_b) = 32`、
    /// `D(slow) = 0`。满计划的键序是 `[short_a, short_b, slow, master]` ⇒
    /// `short_a` 占第 1 槽。剪掉 `short_a` 之后键序是 `[short_b, slow, master]` ⇒
    /// **`short_b` 从第 2 槽下移到第 1 槽**，而那一槽的环里留着 `short_a` 写进去的
    /// 128 个 `MARKER` 样本。
    ///
    /// 变红的注入：删掉 `rearm` 里的 `line.reset()` ⇒ `short_b` 的第一个块吐出
    /// `short_a` 的标记样本（本判据实测红行见交付报告）。
    #[test]
    fn rebinding_a_slot_never_replays_the_previous_nodes_audio() {
        /// 上一任写进环里的标记样本（`0.0` 之外的可辨认值）。
        const MARKER: f32 = 7.0;
        /// 补偿延迟（采样点），同时决定"环里留多少历史"。
        const DELAY: u32 = 32;
        /// 一个块的长度：大于环长的一半 ⇒ 环里到处都有标记，判据不可能靠运气变绿。
        const BLOCK: usize = 128;

        // ⚠ `[EntityId::new(); 4]` 只会调**一次** `new()` 再把那个值复制四份
        // （`EntityId` 是 `Copy`）⇒ 四个身份全相同。必须用 `from_fn`。
        let mut ids: [EntityId; 4] = core::array::from_fn(|_| EntityId::new());
        ids.sort_unstable();
        assert!(
            ids.windows(2).all(|pair| pair[0] != pair[1]),
            "身份必须两两不同：{ids:?}"
        );
        let [short_a, short_b, slow, master] = ids;

        let full = graph_of(&[(short_a, master), (short_b, master), (slow, master)]);
        let table = latency_table(&[(slow, DELAY)]);
        let plan = PdcPlan::compute(&full, master, &table).expect("合法 DAG");
        assert_eq!(plan.compensation(&short_a), Some(DELAY));
        assert_eq!(plan.compensation(&short_b), Some(DELAY));
        assert_eq!(plan.compensation(&slow), Some(0));

        let mut bank = CompensationBank::preallocated(4, 128);
        let shortfall = bank.rearm(&plan);
        assert!(shortfall.is_exact(), "4 个节点 4 条线：{shortfall:?}");
        assert_eq!(bank.rebindings(), 4, "全新的池：四个槽位都换了主人");
        assert_eq!(
            bank.line(&short_a).map(DelayLine::delay),
            Some(DELAY as usize),
            "最小身份占第 1 槽"
        );

        // 在**第 1 槽**（`short_a`）里灌满可辨认的历史。`DELAY = 32 < BLOCK` ⇒
        // 输出的后 96 个样本回读的是本块自己写进去的标记，前 32 个是零初始历史。
        let mut donor = [MARKER; BLOCK];
        assert!(bank.apply(&short_a, &mut donor));
        assert_eq!(donor[..DELAY as usize], [0.0; DELAY as usize]);
        assert_eq!(donor[DELAY as usize..], [MARKER; BLOCK - DELAY as usize]);

        // 剪掉最小的身份 ⇒ `short_b` 下移进第 1 槽。
        let pruned = graph_of(&[(short_b, master), (slow, master)]);
        let after = PdcPlan::compute(&pruned, master, &table).expect("合法 DAG");
        assert!(
            after.compensation(&short_a).is_none(),
            "被剪掉的节点不在新计划里"
        );
        assert_eq!(after.compensation(&short_b), Some(DELAY));
        let before_rebindings = bank.rebindings();
        bank.rearm(&after);
        assert!(
            bank.rebindings() > before_rebindings,
            "重绑必须被计数（覆盖度见证：零分配判据靠它证明没有空转）"
        );

        let mut inherited = [1.0f32; BLOCK];
        assert!(bank.apply(&short_b, &mut inherited));
        assert_eq!(
            inherited[..DELAY as usize],
            [0.0f32; DELAY as usize],
            "重绑的槽位前 {DELAY} 帧必须逐位静音（= 全新实例的零历史），\
             而不是上一任的 {MARKER} 标记样本"
        );
        assert_eq!(
            inherited[DELAY as usize..],
            [1.0f32; BLOCK - DELAY as usize],
            "延迟之后的样本必须来自**本节点自己的**输入（逐位等于全新实例）"
        );
    }

    /// **直通期间也必须记录**：`delay == 0` 不是"环的暂停键"。
    ///
    /// 不变量（见 [`DelayLine`] 的类型文档）：环永远是输入端最近 `capacity − 1` 个
    /// 样本的连续记录。少了这条，直通期间环会**冻结**，之后把延迟开正就会重放
    /// 冻结之前那段陈音频。
    ///
    /// 变红的注入：把 `process_in_place` / `process` 的 `delay == 0` 分支改回
    /// "直接返回"（只读不写）⇒ 第二个断言吐出的是 `[0, 0, 0, 0, 9, 10, 11, 12]`
    /// （冻结的零历史），而不是 `[5, 6, 7, 8, 9, 10, 11, 12]`。
    #[test]
    fn a_line_that_was_passing_through_keeps_a_continuous_history() {
        let first = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let second = [9.0f32, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0];
        let expected = [5.0f32, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];

        // 就地变体。
        let mut line = DelayLine::new(8);
        assert_eq!(line.set_delay(0), 0);
        let mut buf = first;
        line.process_in_place(&mut buf);
        assert_eq!(buf, first, "delay 0 必须直通");
        assert_eq!(line.set_delay(4), 4);
        let mut buf = second;
        line.process_in_place(&mut buf);
        assert_eq!(buf, expected, "直通期间的历史必须是连续的");

        // 非就地变体：同一条不变量。
        let mut line = DelayLine::new(8);
        line.set_delay(0);
        let mut sink = [0.0f32; 8];
        line.process(&first, &mut sink);
        assert_eq!(sink, first);
        line.set_delay(4);
        let mut sink = [0.0f32; 8];
        line.process(&second, &mut sink);
        assert_eq!(sink, expected);
    }

    /// 池装不下时必须**回报**差额，而不是静默地少补。
    #[test]
    fn rearm_reports_slot_and_capacity_shortfall_instead_of_silently_under_compensating() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let m = EntityId::new();
        let graph = graph_of(&[(a, b), (b, m), (a, c), (c, m)]);
        let plan = PdcPlan::compute(&graph, m, &latency_table(&[(b, 10)])).expect("合法 DAG");

        // 槽位不足：4 个补偿节点只给 1 条线 ⇒ 3 个未武装。
        let mut few_slots = CompensationBank::preallocated(1, 64);
        let shortfall = few_slots.rearm(&plan);
        assert_eq!(shortfall.unarmed_nodes, 3);
        assert_eq!(shortfall.clamped_frames, 0);
        assert!(!shortfall.is_exact());
        assert_eq!(few_slots.len(), 1, "只武装了装得下的那些");

        // 容量不足：线容量 4 ⇒ 两条 `D = 10` 的支路各被钳掉 6 帧，缺口必须被记下。
        let mut small = CompensationBank::preallocated(4, 4);
        let shortfall = small.rearm(&plan);
        assert_eq!(shortfall.unarmed_nodes, 0);
        assert_eq!(
            shortfall.clamped_frames, 12,
            "两条短支路各有 (10 - 4) = 6 帧补偿缺口"
        );
        assert!(!shortfall.is_exact());
        for node in [a, b, c, m] {
            let delay = small.line(&node).map(DelayLine::delay).expect("已武装");
            assert!(delay <= 4, "任何延迟都不得越过线容量");
        }
    }

    /// 判据：`PdcError` 的 **Display 文案**是公开面，必须逐字钉住。
    ///
    /// `thiserror` 的 `#[error("…")]` 同时定义 `Display` 与"结构化错误"。文案会被
    /// 控制面/CLI/MCP 原样交给用户与日志，因此它是**契约的一部分**，而不是实现细节。
    /// `Cycle` 的文案里还带一个**读数**（未消化节点数）⇒ 那个读数必须跟着状态走。
    ///
    /// **量什么**：三个变体的 `to_string()` 文本（单位：字符），以及 `Cycle` 里那个计数。
    /// 注入实测（第四批）：`#[error("PDC: master node is not present in routing graph nodes")]`
    /// → `#[error("PDC: master missing")]` ⇒ 本判据实测变红。
    #[test]
    fn pdc_error_messages_are_the_documented_text() {
        assert_eq!(
            PdcError::UnknownMaster {
                master: EntityId::default(),
            }
            .to_string(),
            "PDC: master node is not present in routing graph nodes"
        );
        assert_eq!(
            PdcError::DanglingEdge {
                node: EntityId::default(),
            }
            .to_string(),
            "PDC: routing edge endpoint is not present in routing graph nodes"
        );
        assert_eq!(
            PdcError::Cycle { nodes: Vec::new() }.to_string(),
            "PDC: routing graph contains a cycle (0 unresolved nodes)"
        );
        assert_eq!(
            PdcError::Cycle {
                nodes: vec![EntityId::default(), EntityId::default()],
            }
            .to_string(),
            "PDC: routing graph contains a cycle (2 unresolved nodes)",
            "文案里的读数必须跟着状态走（不是常量）"
        );
        // R58：`==` 的判据必须另有一条 `assert_ne!` 落在**同一个**表达式上 ——
        // 否则"两边都退化成同一个常量"也能让上面的等号成立。
        assert_ne!(
            PdcError::Cycle { nodes: Vec::new() }.to_string(),
            PdcError::Cycle {
                nodes: vec![EntityId::default()],
            }
            .to_string(),
            "0 与 1 个未消化节点的文案必须不同（否则上面的等号可能是'两边同一个常量'）"
        );
    }
}
