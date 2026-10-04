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
//!    关键路径延迟 `L_max = arrival(master)` [ARCH-PDC-001]。
//! 3. **延迟分配**：对每条可达 master 的路径插入
//!    `D(v) = L_max - arrival(v)` 个采样点的环形延迟。
//!
//! ### 定义性不变量（这是 PDC 的判据，不是实现细节）
//!
//! ```text
//! ∀ v ∈ Reachable(master):  arrival(v) + D(v) == L_max
//! ```
//!
//! 即"所有分支经过延迟线后总延迟相等"。任何并行支路在汇入求和节点时相位完全对齐。
//! 该不变量在测试里被机械断言（见 `mod tests` 的 `pdc_alignment_invariant_*`）。
//!
//! ## 与离线渲染共用
//!
//! [`PdcPlan::compute`] 是**不依赖 cpal 的纯函数**：输入 `RoutingGraph` + 延迟表，
//! 输出纯数据。`yeban-render` 的 Rayon 离线母带渲染器直接复用它，从而保证
//! "实时与离线绝对相位对齐"（规范 §3.4 第 3 条）。
//!
//! ## 规范缺口：节点延迟的来源
//!
//! [ARCH-PDC-001] 要求"每个插件与内置设备必须精确上报其引入的处理延迟
//! (`DeviceDefinition::latency_samples`)"。但 `yeban-model` 的
//! [`yeban_model::DeviceDefinition`] **当前没有** `latency_samples` 字段，
//! 因此延迟由调用方通过 [`LatencyTable`] 显式提供。
//! 详见 `docs/ledger/engine-rt-notes.md` 的 needs 清单。

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use yeban_model::{EntityId, RoutingGraph};

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

    /// 从工程音轨集合投影出延迟表。
    ///
    /// **当前实现返回全零表**，因为 `yeban_model::DeviceDefinition` 尚未提供
    /// `latency_samples`（[ARCH-PDC-001] 的规范缺口，见模块文档）。函数签名先立起来，
    /// 等模型层补上字段后只需改这里一行，调用方无感。
    #[must_use]
    pub fn from_tracks(tracks: &BTreeMap<EntityId, yeban_model::TrackV3>) -> Self {
        let mut table = Self::new();
        for id in tracks.keys() {
            // TODO(model): [ARCH-PDC-001] 要求 `device.latency_samples`，
            // yeban-model 暂无该字段 —— 目前每个节点的处理延迟恒为 0
            // （即"设备链不引入延迟"），这是**待补的缺口**而不是设计选择。
            table.set(*id, 0);
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
        let total_latency = arrival.get(&master).copied().unwrap_or(0);

        // --- 3. 只对"能到达 master"的子图分配补偿延迟 ---
        // 若 p → v 且 v 能到达 master, 则 p 也能到达 master, 因此该子图对前驱封闭,
        // 于是 ∀v ∈ Reachable: arrival(v) <= L_max, D(v) = L_max - arrival(v) 不会下溢。
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

    /// 关键路径总延迟 `L_max`。
    #[must_use]
    pub fn total_latency(&self) -> u32 {
        self.total_latency
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
/// - `delay == 0` 是显式直通快路径（不读旧样本，避免"延迟 0 却读到陈旧值"的经典 bug）；
/// - 索引按下标取模而非 `%` 运算，避免每样本一次除法。
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

    /// 清零缓冲内容（保留延迟设置）。
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
    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        if self.delay == 0 || buf.is_empty() {
            return;
        }
        let capacity = self.buffer.len();
        let mut write = self.write;
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
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        if self.delay == 0 {
            for (out, inp) in output.iter_mut().zip(input) {
                *out = *inp;
            }
            return;
        }
        let capacity = self.buffer.len();
        let mut write = self.write;
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
#[derive(Clone, Debug)]
pub struct CompensationBank {
    /// 按键升序（`PdcPlan::compensation` 是 `BTreeMap`，天然有序）。
    entries: Vec<(EntityId, DelayLine)>,
}

impl CompensationBank {
    /// 按 PDC 计划构造延迟线集合。
    #[must_use]
    pub fn from_plan(plan: &PdcPlan) -> Self {
        let mut entries: Vec<(EntityId, DelayLine)> = Vec::with_capacity(plan.compensated_len());
        for (node, samples) in plan.compensation.iter() {
            let mut line = DelayLine::new(*samples as usize);
            line.set_delay(*samples as usize);
            entries.push((*node, line));
        }
        Self { entries }
    }

    /// 延迟线数量（= 需要补偿的节点数）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否没有任何延迟线。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
        match self
            .entries
            .binary_search_by(|(candidate, _)| candidate.cmp(node))
        {
            Ok(index) => {
                self.entries[index].1.process_in_place(buf);
                true
            }
            Err(_) => false,
        }
    }

    /// 只读访问某个节点的延迟线。
    #[must_use]
    pub fn line(&self, node: &EntityId) -> Option<&DelayLine> {
        self.entries
            .binary_search_by(|(candidate, _)| candidate.cmp(node))
            .ok()
            .map(|index| &self.entries[index].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn latency_table_from_tracks_is_all_zero_pending_model_gap() {
        use yeban_model::{DeviceDefinition, DeviceKind, TrackV3};
        let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
        let id = EntityId::new();
        let mut track = TrackV3 {
            id,
            ..TrackV3::default()
        };
        track.devices.push(DeviceDefinition {
            kind: DeviceKind::InternalEffect,
            ..DeviceDefinition::default()
        });
        tracks.insert(id, track);
        let table = LatencyTable::from_tracks(&tracks);
        // TODO(model): [ARCH-PDC-001] 需要 DeviceDefinition::latency_samples;
        // 字段落地前这里恒为 0 —— 本测试把这个"待补"事实钉住, 免得被误读成"已实现"。
        assert_eq!(table.get(&id), 0);
        assert!(table.is_empty(), "全零表不保留条目（set(_, 0) == 未登记）");
    }
}
