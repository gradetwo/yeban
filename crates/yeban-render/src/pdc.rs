//! 内部插件延迟补偿 (PDC) 的关键路径分析与延迟分配 [ARCH-PDC-001] [ARCH-PDC-002]。
//!
//! ## 规范化要求
//!
//! > **[ARCH-PDC-001]** 每个插件与内置设备必须精确上报其引入的处理延迟
//! > (`DeviceDefinition::latency_samples`)。
//! >
//! > **DAG 关键路径拓扑分析**: 在非实时线程构建 `EngineSnapshot` 时, 分析
//! > `RoutingGraph` 中从每个信号源到 Master 总线的所有声学通路, 计算各并行分支的
//! > 累积延迟, 确定最长延迟关键路径 `L_max`。
//! >
//! > **自动补偿对齐**: 对于累积延迟为 `L_i` 的并行分支, 在进入总线求和节点前自动
//! > 插入 `D_i = L_max − L_i` 采样点的环形延迟缓冲 (PDC Delay Line); 实时音频引擎与
//! > Rayon 离线母带渲染器**完全共用同一套 PDC 算法**。
//!
//! ## 本模块的状态: 最小同构实现 —— **待 `yeban-engine` 线提供后必须改为复用**
//!
//! `crates/yeban-engine` 在本分支上仍是 scaffold (`src/lib.rs` 只有文档), 没有暴露
//! 任何 PDC API。为了让离线渲染器的相位对齐不是"写死的 0", 这里实现了一份**最小
//! 同构**版本。它是纯函数、零 cpal 依赖、零第三方依赖, 因此:
//!
//! - 本机可以用 `rustc --edition 2024 --test` 单独验证 (见 `verify/pure_modules.rs`);
//! - 引擎线提供接口后, 这里的 `plan()` 应当**整体退役**, 换成对
//!   `yeban_engine::pdc::plan()` 的调用, 并由 `render.rs` 的等价性测试
//!   (`pdc_plan_matches_engine_reference`) 防止两条实现漂移。
//!
//! 这一条已登记进 `docs/ledger/render-master-notes.md` 的 `needs` 清单。
//!
//! ## 补偿延迟插在哪条边上: `D = arrival[目的] − output_latency[源]`
//!
//! 规范正文写的是 `D_i = L_max − L_i`, 并指定插在"进入总线求和节点前"。
//! 本实现把同一个式子**逐边**求解成 `Plan::edge_delay`:
//!
//! ```text
//! D(s → d) = arrival[d] − output_latency[s]      (饱和减; 结果恒 ≥ 0)
//! ```
//!
//! `arrival[d]` 是 `d` 全部入边 `output_latency` 的**最大值**, 也就是 `d` 自己的 `L_max`。
//! 三条可算的后果:
//!
//! 1. **单入边节点不补**。入边只有一条时 `arrival[d] == output_latency[s]` ⇒ `D = 0`。
//!    ⇒ 补偿只出现在**多入边**（真正的求和）节点上, 与规范的"进入总线求和节点前"一致。
//! 2. **求和节点用自己的 `arrival`, 不用 Master 的全局 `L_max`**。用全局值会把下游节点
//!    的**自身延迟补偿第二次**: 同一份信号在 Master 上分成相差数十帧的两波, 正是
//!    [ARCH-PDC-001] 要消灭的低频相位干涉。判据
//!    `a_deeper_bus_uses_its_own_arrival_not_the_master_critical_path` 钉住这一点。
//! 3. **`longest_path` 就是补偿后的实际前滚量**。逐边按上式补偿后 `A(n) == output_latency[n]`
//!    （归纳见 `every_branch_arrives_at_its_summing_node_together` 的文档）,
//!    因此 Master 的输出时刻恰好是 `longest_path` 而不是"比它更大"。
//!
//! ## 本模块零第三方依赖
//!
//! 图用泛型键 `K` 表达, 因此这里不出现 `EntityId` —— `render.rs` 负责把
//! `yeban_model::RoutingGraph` 投影成本模块的 [`Graph`]。副作用是 `pdc.rs` 可以
//! 被单独编译执行, 而"实体身份"这件事仍然只有一个事实源。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

/// 参与 PDC 分析的有向无环图 (路由图的 PDC 投影)。
///
/// `edges` 是 `(source, destination)` 对。允许重复对 (同一对节点之间有并行边),
/// 因为"取入边延迟最大值"对重复是幂等的。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph<K> {
    /// 全部节点。
    pub nodes: Vec<K>,
    /// 全部有向边。
    pub edges: Vec<(K, K)>,
    /// 每个节点**自身**引入的处理延迟 (采样帧数), 即
    /// `DeviceDefinition::latency_samples` 的投影 [ARCH-PDC-001]。
    /// 未登记的节点按 0 处理。
    pub latencies: BTreeMap<K, u32>,
}

/// PDC 分析可能失败的方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdcError {
    /// 图里有环 —— 路由图必须无环 [ARCH-PDC-001 "DAG"]。
    Cycle {
        /// 未能完成拓扑排序、因而留在环里的节点 (升序)。
        remaining: Vec<String>,
    },
    /// 某条边的端点不在 `nodes` 里。
    UnknownNode {
        /// 缺失的节点。
        node: String,
    },
    /// Master 节点不在图里。
    MasterNotInGraph {
        /// 传入的 master。
        master: String,
    },
}

impl core::fmt::Display for PdcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cycle { remaining } => write!(f, "路由图有环, 剩余节点: {remaining:?}"),
            Self::UnknownNode { node } => write!(f, "边的端点不在节点表里: {node}"),
            Self::MasterNotInGraph { master } => write!(f, "Master 节点不在图里: {master}"),
        }
    }
}

impl std::error::Error for PdcError {}

/// 一次 PDC 分析的完整结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan<K> {
    /// 到达各节点的累积延迟 = 全部入边的 `output_latency` 最大值 (无入边则为 0)。
    pub arrival: BTreeMap<K, u32>,
    /// 离开各节点的累积延迟 = `arrival + 该节点自身延迟`。
    pub output_latency: BTreeMap<K, u32>,
    /// 最长延迟关键路径 `L_max` = Master 的 `output_latency`。补偿之后, 它同时是
    /// Master 输出的**实际前滚量**（帧）。
    pub longest_path: u32,
    /// 每条边需要插入的补偿延迟（帧）: `arrival[destination] − output_latency[source]`。
    ///
    /// 单入边节点上的值恒为 `0`（见模块文档第 1 条）; 多入边（求和）节点上, 它就是
    /// 规范 `D_i = L_max − L_i` 在**该节点**上的取值（`L_max` 取该节点自己的 `arrival`）。
    pub edge_delay: BTreeMap<(K, K), u32>,
    /// 确定性拓扑序 (Kahn + 最小键优先)。
    ///
    /// 求最大值的算法本身与处理顺序无关, 但**顺序本身也要确定**: `render.rs` 用它
    /// 来分层并行, 而层级划分必须逐次运行一致。
    pub topological_order: Vec<K>,
}

impl<K: Ord + Copy> Plan<K> {
    /// 取出某条边的补偿延迟 (未登记的边返回 `None`)。
    #[must_use]
    pub fn delay_of(&self, source: K, destination: K) -> Option<u32> {
        self.edge_delay.get(&(source, destination)).copied()
    }
}

/// 分析 `graph` 到 `master` 的延迟, 分配每条边的补偿延迟。
///
/// 每条边的补偿延迟是 `D(s → d) = arrival[d] − output_latency[s]`（见模块文档）。
/// 单入边节点因此恒得 `0`; 补偿只出现在真正合并多条支路的求和节点上。
///
/// # Errors
///
/// - [`PdcError::Cycle`] 图有环;
/// - [`PdcError::UnknownNode`] 边的端点不在 `nodes` 里;
/// - [`PdcError::MasterNotInGraph`] `master` 不在 `nodes` 里。
pub fn plan<K>(graph: &Graph<K>, master: K) -> Result<Plan<K>, PdcError>
where
    K: Ord + Copy + Debug,
{
    let node_set: BTreeSet<K> = graph.nodes.iter().copied().collect();
    if !node_set.contains(&master) {
        return Err(PdcError::MasterNotInGraph {
            master: format!("{master:?}"),
        });
    }
    for &(source, destination) in &graph.edges {
        for node in [source, destination] {
            if !node_set.contains(&node) {
                return Err(PdcError::UnknownNode {
                    node: format!("{node:?}"),
                });
            }
        }
    }

    // 入边表按"目的节点"分组, 并用 BTreeMap 保证组内顺序确定。同时算入度。
    let mut incoming: BTreeMap<K, Vec<K>> =
        node_set.iter().map(|&node| (node, Vec::new())).collect();
    let mut indegree: BTreeMap<K, usize> = node_set.iter().map(|&node| (node, 0)).collect();
    for &(source, destination) in &graph.edges {
        incoming
            .get_mut(&destination)
            .expect("端点已校验")
            .push(source);
        *indegree.get_mut(&destination).expect("端点已校验") += 1;
    }

    // Kahn 拓扑排序。就绪集合用 BTreeSet: 每一步取**最小键**, 因此拓扑序唯一。
    let mut ready: BTreeSet<K> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(&node, _)| node)
        .collect();
    let mut order = Vec::with_capacity(node_set.len());
    let mut arrival: BTreeMap<K, u32> = BTreeMap::new();
    let mut output_latency: BTreeMap<K, u32> = BTreeMap::new();

    while let Some(&node) = ready.iter().next() {
        ready.remove(&node);
        order.push(node);

        let arrives = incoming
            .get(&node)
            .expect("入边表覆盖全部节点")
            .iter()
            .map(|source| output_latency.get(source).copied().unwrap_or(0))
            .max()
            .unwrap_or(0);
        arrival.insert(node, arrives);
        let own = graph.latencies.get(&node).copied().unwrap_or(0);
        output_latency.insert(node, arrives.saturating_add(own));

        for &(source, destination) in &graph.edges {
            if source != node {
                continue;
            }
            let degree = indegree.get_mut(&destination).expect("端点已校验");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(destination);
            }
        }
    }

    if order.len() != node_set.len() {
        let remaining = node_set
            .iter()
            .filter(|node| !order.contains(node))
            .map(|node| format!("{node:?}"))
            .collect();
        return Err(PdcError::Cycle { remaining });
    }

    let longest_path = output_latency.get(&master).copied().unwrap_or(0);
    let mut edge_delay: BTreeMap<(K, K), u32> = BTreeMap::new();
    for &(source, destination) in &graph.edges {
        let source_latency = output_latency.get(&source).copied().unwrap_or(0);
        // 目的节点的 `arrival` 是它全部入边 `output_latency` 的最大值, 因此对**每一条**
        // 入边都有 `arrival[destination] >= output_latency[source]`。取 `arrival` 而不是
        // Master 的 `longest_path`: 后者会把目的节点下游堆起来的自身延迟补偿第二次,
        // 让同一份信号在 Master 上分成两波（见模块文档的第 2 条）。
        let destination_arrival = arrival.get(&destination).copied().unwrap_or(0);
        edge_delay.insert(
            (source, destination),
            destination_arrival.saturating_sub(source_latency),
        );
    }

    Ok(Plan {
        arrival,
        output_latency,
        longest_path,
        edge_delay,
        topological_order: order,
    })
}

/// 按 [ARCH-PDC-002] 的预算表把采样帧数换算成毫秒。
///
/// 规范表在 48 kHz 下给出: 64 帧 = **1.33 ms**, 128 帧 = 2.67 ms。本函数是那张表
/// 的可执行形式, 判据见本模块测试 `latency_budget_table_matches_the_spec`。
#[must_use]
pub fn frames_to_millis(frames: u32, sample_rate: u32) -> f64 {
    f64::from(frames) * 1000.0 / f64::from(sample_rate)
}

/// 交错多声道的环形延迟线 (PDC Delay Line)。
///
/// 延迟量以**帧**计 (一帧 = 每声道一个采样点), 与 `latency_samples` 的口径一致:
/// 规范里的"延迟采样点"在监听回路预算表里是按帧算的 (64 帧 = 1.33 ms @48 kHz)。
///
/// 语义: 输出比输入**晚** `delay_frames` 帧。因此前 `delay_frames` 帧输出静音 ——
/// 这正是延迟线应有的行为, 不是 bug。
#[derive(Clone, Debug)]
pub struct DelayLine {
    buffer: Vec<f32>,
    channels: usize,
    frame_cursor: usize,
    delay_frames: usize,
}

impl DelayLine {
    /// 构造一条 `delay_frames` 帧、`channels` 声道的延迟线。
    ///
    /// `channels == 0` 时退化为"丢弃一切"的空线 (不会 panic), 但调用方应在此之前
    /// 拒绝 0 声道。
    #[must_use]
    pub fn new(delay_frames: usize, channels: usize) -> Self {
        Self {
            buffer: vec![0.0; delay_frames.saturating_mul(channels)],
            channels,
            frame_cursor: 0,
            delay_frames,
        }
    }

    /// 该延迟线引入的延迟 (帧)。
    #[must_use]
    pub const fn delay_frames(&self) -> usize {
        self.delay_frames
    }

    /// 声道数。
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// `true` 表示零延迟, [`Self::process`] 会把输入原样拷到输出。
    #[must_use]
    pub const fn is_bypass(&self) -> bool {
        self.delay_frames == 0
    }

    /// 交错块的延迟处理: 读写 `input` 指向的交错块, 把延迟后的样本写进 `out`。
    ///
    /// **只处理两条切片共有的那一段**, 即 `common = input.len().min(out.len())`:
    /// 长度不等时按最短者处理, 不越界、不分配、不 panic。
    /// `out[common..]` **不被本函数写入** —— 那是调用方的字节。
    ///
    /// 整帧 (每帧 `channels` 个样本) 走延迟线; 不足一整帧的尾巴**逐位透传**。
    /// 零延迟时是整段 `common` 的**逐位拷贝** —— 旁路路径不得引入任何浮点运算,
    /// 否则确定性契约会在"有/无 PDC"之间出现 LSB 差异。
    /// `channels == 0` 退化为"丢弃一切": 一个字节都不写。
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let common = input.len().min(out.len());
        if self.channels == 0 {
            return;
        }
        if self.is_bypass() {
            out[..common].copy_from_slice(&input[..common]);
            return;
        }
        let channels = self.channels;
        let frames = common / channels;
        let capacity_frames = self.delay_frames;
        for frame in 0..frames {
            let in_base = frame * channels;
            let out_base = in_base;
            let slot = self.frame_cursor * channels;
            // 先读出 slot 里的旧样本 (恰好是 delay_frames 帧之前写进去的),
            out[out_base..out_base + channels].copy_from_slice(&self.buffer[slot..slot + channels]);
            // 再把新样本写回同一个 slot。
            self.buffer[slot..slot + channels].copy_from_slice(&input[in_base..in_base + channels]);
            self.frame_cursor += 1;
            if self.frame_cursor == capacity_frames {
                self.frame_cursor = 0;
            }
        }
        // 尾部 (不足一整帧的部分) 原样透传, 避免调用方读到未初始化的旧数据。
        // 上界是 `common` 而不是 `input.len()`: 两条切片长度不等时只写共有段。
        let tail = frames * channels;
        out[tail..common].copy_from_slice(&input[tail..common]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(
        nodes: &[&'static str],
        edges: &[(&'static str, &'static str)],
        lats: &[(&'static str, u32)],
    ) -> Graph<&'static str> {
        Graph {
            nodes: nodes.to_vec(),
            edges: edges.to_vec(),
            latencies: lats.iter().copied().collect(),
        }
    }

    /// 每个节点的入边源节点（用于"各路到达时刻"的对齐断言）。
    fn incoming(graph: &Graph<&'static str>) -> BTreeMap<&'static str, Vec<&'static str>> {
        let mut out: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
        for &(source, destination) in &graph.edges {
            out.entry(destination).or_default().push(source);
        }
        out
    }

    /// 沿图传播一遍"节点输出代表输入 t = 0 的绝对时刻"（帧）。
    ///
    /// 模型与真实设备链一致: `latency_samples` 是设备**自己**引入的延迟, 设备内部已经
    /// 晚 `own` 帧输出, 渲染器因此**只**插延迟线, 不再额外补 `own`。于是
    ///
    /// ```text
    /// A(源) = own(源)                                     (无入边)
    /// A(n)  = max_(s→n) [ A(s) + D(s→n) ] + own(n)
    /// ```
    ///
    /// 这是对 `Plan` 的**独立复算**: 它只读每条边的 `D` 与每个节点的 `own`, 不重算
    /// `arrival` / `output_latency`。
    fn propagate(
        graph: &Graph<&'static str>,
        plan: &Plan<&'static str>,
    ) -> BTreeMap<&'static str, u32> {
        let mut arrivals: BTreeMap<&'static str, u32> = BTreeMap::new();
        for &node in &plan.topological_order {
            let own = graph.latencies.get(&node).copied().unwrap_or(0);
            let delivered = graph
                .edges
                .iter()
                .filter(|(_, destination)| *destination == node)
                .map(|(source, destination)| {
                    arrivals.get(source).copied().unwrap_or(0)
                        + plan
                            .delay_of(*source, *destination)
                            .expect("每条边都有补偿延迟")
                })
                .max()
                .unwrap_or(0);
            arrivals.insert(node, delivered + own);
        }
        arrivals
    }

    /// [ARCH-PDC-002] 的预算表是规范正文里的硬数字, 这里把它变成可执行判据。
    #[test]
    fn latency_budget_table_matches_the_spec() {
        // 表: 物理输入缓冲 64 帧 = 1.33 ms; 输出缓冲 64 帧 = 1.33 ms;
        // 备选方案 128 帧 => 5.33 ms (输入+输出合计)。
        assert!((frames_to_millis(64, 48_000) - 1.3333).abs() < 0.001);
        assert!((frames_to_millis(128, 48_000) - 2.6667).abs() < 0.001);
        assert!((frames_to_millis(256, 48_000) - 5.3333).abs() < 0.001);
    }

    /// 并联分支: 一条 100 帧延迟的支路与一条 0 帧的支路汇聚到 Master。
    /// 零延迟那一条必须被补偿 100 帧, 长延迟那条不补。
    #[test]
    fn shorter_branch_gets_the_larger_compensation() {
        let g = graph(
            &["track-fast", "track-slow", "master"],
            &[("track-fast", "master"), ("track-slow", "master")],
            &[("track-slow", 100)],
        );
        let plan = plan(&g, "master").expect("无环");
        assert_eq!(plan.longest_path, 100);
        assert_eq!(plan.output_latency["track-fast"], 0);
        assert_eq!(plan.output_latency["track-slow"], 100);
        assert_eq!(plan.delay_of("track-fast", "master"), Some(100));
        assert_eq!(plan.delay_of("track-slow", "master"), Some(0));
    }

    /// 串接: 两级各 10 帧 + 母线自身 5 帧 => `L_max = 25`。
    ///
    /// 串接链上**没有并行支路**, 因此两个节点都只有一条入边 ⇒ 两条边都不补
    /// （`D = 0`）。补偿的语义是"把短支路补齐到与长支路同一时刻", 单支路没有可对齐的
    /// 对象。自身延迟仍然算进 `output_latency` 与 `L_max`, 它们决定实际前滚量。
    ///
    /// ⚠ 本判据的两条期望值**在本次提交里被改写**: 旧实现给 `a → b` 补 15、给
    /// `b → master` 补 5（用 Master 的全局 `L_max`）。那会让一条**单支路**多绕 20 帧,
    /// 并在有并行支路时把下游节点的自身延迟补偿第二次 —— 见模块文档第 2 条与
    /// `every_branch_arrives_at_its_summing_node_together`。
    #[test]
    fn serial_latency_accumulates_along_the_path() {
        let g = graph(
            &["a", "b", "master"],
            &[("a", "b"), ("b", "master")],
            &[("a", 10), ("b", 10), ("master", 5)],
        );
        let plan = plan(&g, "master").expect("无环");
        assert_eq!(plan.arrival["a"], 0);
        assert_eq!(plan.output_latency["a"], 10);
        assert_eq!(plan.arrival["b"], 10);
        assert_eq!(plan.output_latency["b"], 20);
        assert_eq!(plan.arrival["master"], 20);
        assert_eq!(plan.longest_path, 25);
        assert_eq!(plan.delay_of("a", "b"), Some(0), "单入边节点不补");
        assert_eq!(plan.delay_of("b", "master"), Some(0), "单入边节点不补");
        // 补偿为 0 时, 实际前滚量仍然等于 `L_max`。
        let arrivals = propagate(&g, &plan);
        assert_eq!(arrivals["master"], plan.longest_path);
    }

    /// 菱形: a -> b -> master 与 a -> master 两条路径。
    /// 短路径 (a 直连 master) 必须按较长路径的到达延迟补齐。
    #[test]
    fn diamond_aligns_both_branches_to_the_longest_path() {
        let g = graph(
            &["a", "b", "master"],
            &[("a", "b"), ("b", "master"), ("a", "master")],
            &[("b", 7)],
        );
        let plan = plan(&g, "master").expect("无环");
        // 经 b 到 master 的到达延迟是 7; a 直连 master 的到达延迟是 0。
        assert_eq!(plan.arrival["master"], 7);
        assert_eq!(plan.longest_path, 7);
        assert_eq!(plan.delay_of("a", "master"), Some(7));
        assert_eq!(plan.delay_of("b", "master"), Some(0));
    }

    /// 每条到达 Master 的分支, 补偿后到达时刻必须全部等于 `L_max` —— 这是
    /// [ARCH-PDC-001] "绝对相位对齐"的可执行形式。
    #[test]
    fn every_branch_arrives_at_master_at_exactly_l_max() {
        let g = graph(
            &["drums", "bus-a", "vox", "bus-b", "master"],
            &[
                ("drums", "bus-a"),
                ("bus-a", "master"),
                ("vox", "bus-b"),
                ("bus-b", "master"),
                ("drums", "master"),
            ],
            &[("drums", 3), ("bus-a", 11), ("vox", 40), ("bus-b", 2)],
        );
        let plan = plan(&g, "master").expect("无环");
        for &(source, destination) in &g.edges {
            if destination != "master" {
                continue;
            }
            let arrived = plan.output_latency[source] + plan.delay_of(source, destination).unwrap();
            assert_eq!(
                arrived, plan.longest_path,
                "分支 {source} -> {destination} 到达时刻 {arrived} != L_max {}",
                plan.longest_path
            );
        }
    }

    /// **绝对相位对齐的可执行形式**: 把每条边的补偿延迟与每个节点的**自身延迟**
    /// 沿图传播一遍, 再断言到每个求和节点的**每一路**贡献都在同一时刻到达。
    ///
    /// # 归纳（为什么 `D = arrival[d] − output_latency[s]` 使 `A(n) == output_latency[n]`）
    ///
    /// 假设全部上游满足 `A(s) == output_latency[s]`, 则到 `d` 的每一路贡献是
    /// `output_latency[s] + arrival[d] − output_latency[s] == arrival[d]` —— **与 `s` 无关**
    /// ⇒ 全路对齐, 且 `A(d) == arrival[d] + own(d) == output_latency[d]`。归纳成立;
    /// 源节点是基例 (`A == own == output_latency`)。于是 `A(master) == longest_path`。
    ///
    /// # 这条判据在旧实现上是**红**的
    ///
    /// 旧公式是 `D = longest_path − output_latency[s]`（Master 的全局值）。它对
    /// `bus-a` 这样**自身有延迟**的中间节点把该延迟补偿了第二次: 旧实现下三路到达
    /// Master 的时刻是 **81 / 44 / 42**（本判据实测到的那组字面量, 见提交说明）,
    /// 相差 39 帧 —— 正是 [ARCH-PDC-001] 要消灭的低频相位干涉。
    #[test]
    fn every_branch_arrives_at_its_summing_node_together() {
        let g = graph(
            &["drums", "bus-a", "vox", "bus-b", "master"],
            &[
                ("drums", "bus-a"),
                ("bus-a", "master"),
                ("vox", "bus-b"),
                ("bus-b", "master"),
                ("drums", "master"),
            ],
            &[("drums", 3), ("bus-a", 11), ("vox", 40), ("bus-b", 2)],
        );
        let plan = plan(&g, "master").expect("无环");
        let arrivals = propagate(&g, &plan);
        let inbound = incoming(&g);

        // 1. 实际前滚量 == 计划预测的 `output_latency`（逐节点, 不是只查 Master）。
        for (&node, &predicted) in &plan.output_latency {
            assert_eq!(
                arrivals[node], predicted,
                "{node}: 实际前滚 {} != 计划 {predicted}",
                arrivals[node]
            );
        }

        // 2. 每一路到达每个求和节点的时刻都相同 —— 这就是"绝对相位对齐"。
        for (&destination, sources) in &inbound {
            if sources.len() < 2 {
                continue;
            }
            let times: Vec<u32> = sources
                .iter()
                .map(|source| {
                    arrivals[source] + plan.delay_of(*source, destination).expect("有补偿延迟")
                })
                .collect();
            assert!(
                times.windows(2).all(|pair| pair[0] == pair[1]),
                "{destination} 的各路到达时刻必须一致, 实际 {times:?}"
            );
        }

        // 3. 手算的定点: 关键路径是 `bus-b` 那一条 (40 + 2 = 42)。
        assert_eq!(plan.longest_path, 42);
        assert_eq!(arrivals["master"], 42);
        assert_eq!(plan.delay_of("drums", "master"), Some(39));
        assert_eq!(plan.delay_of("bus-a", "master"), Some(28));
        assert_eq!(plan.delay_of("bus-b", "master"), Some(0));
        // 中间节点的自身延迟**不得**在它的入边上被补偿第二次。
        assert_eq!(plan.delay_of("drums", "bus-a"), Some(0));
        assert_eq!(plan.delay_of("vox", "bus-b"), Some(0));
    }

    /// **回归护栏**: 求和节点用自己的 `arrival`, 不用 Master 的全局 `L_max`。
    ///
    /// 图: `s`(自身 100) 直连 Master; `d`(求和, 自身 0) 由 `x`(10) 与 `y`(20) 喂。
    /// ⇒ `arrival[d] = 20`（`d` 自己的 `L_max`）, `longest_path = 100`（Master 的）。
    ///
    /// 这条判据同时拒绝**两种**写法: "每条边都用全局 `L_max`"（旧实现）与
    /// "只给求和节点补, 但用全局 `L_max`"。两者都会给 `d → master` 补 80 而让
    /// `s` 支路与 `d` 支路在 Master 上相差 80 帧。
    #[test]
    fn a_deeper_bus_uses_its_own_arrival_not_the_master_critical_path() {
        let g = graph(
            &["s", "x", "y", "d", "master"],
            &[("s", "master"), ("x", "d"), ("y", "d"), ("d", "master")],
            &[("s", 100), ("x", 10), ("y", 20)],
        );
        let plan = plan(&g, "master").expect("无环");
        assert_eq!(plan.arrival["d"], 20, "d 自己的 L_max 是 20, 不是 100");
        assert_eq!(plan.longest_path, 100);
        // 手算: d 的两路各自对齐到 20; Master 的两路各自对齐到 100。
        assert_eq!(plan.delay_of("x", "d"), Some(10));
        assert_eq!(plan.delay_of("y", "d"), Some(0));
        assert_eq!(plan.delay_of("d", "master"), Some(80));
        assert_eq!(plan.delay_of("s", "master"), Some(0));

        let arrivals = propagate(&g, &plan);
        assert_eq!(arrivals["d"], 20);
        assert_eq!(arrivals["master"], 100);
        assert_eq!(
            arrivals["d"] + plan.delay_of("d", "master").unwrap(),
            arrivals["s"] + plan.delay_of("s", "master").unwrap(),
            "两条支路必须在 Master 上同时到达"
        );
    }

    #[test]
    fn cycles_are_rejected() {
        let g = graph(
            &["a", "b", "master"],
            &[("a", "b"), ("b", "a"), ("b", "master")],
            &[],
        );
        match plan(&g, "master") {
            Err(PdcError::Cycle { remaining }) => {
                // a 与 b 互相依赖, 没有入度为 0 的节点, 因此 master 也无法排程 ——
                // 剩下的就是全部三个节点。关键是它们被**拒绝**, 而不是被算出一个假的延迟。
                assert_eq!(remaining.len(), 3, "三个节点都无法排程");
                assert!(remaining.iter().any(|node| node.contains('a')));
                assert!(remaining.iter().any(|node| node.contains('b')));
            }
            other => panic!("期望 Cycle, 得到 {other:?}"),
        }
    }

    #[test]
    fn dangling_edges_and_unknown_master_are_rejected() {
        let dangling = graph(&["a", "master"], &[("a", "ghost")], &[]);
        assert!(matches!(
            plan(&dangling, "master"),
            Err(PdcError::UnknownNode { .. })
        ));
        let no_master = graph(&["a"], &[], &[]);
        assert!(matches!(
            plan(&no_master, "nowhere"),
            Err(PdcError::MasterNotInGraph { .. })
        ));
    }

    /// 拓扑序必须唯一且确定: 同样的图、任意节点/边的书写顺序, 结果一致。
    #[test]
    fn topological_order_is_independent_of_input_order() {
        let forward = graph(
            &["a", "b", "c", "master"],
            &[("a", "c"), ("b", "c"), ("c", "master")],
            &[],
        );
        let shuffled = graph(
            &["master", "c", "b", "a"],
            &[("c", "master"), ("b", "c"), ("a", "c")],
            &[],
        );
        let first = plan(&forward, "master").expect("无环");
        let second = plan(&shuffled, "master").expect("无环");
        assert_eq!(first.topological_order, second.topological_order);
        assert_eq!(first.topological_order, vec!["a", "b", "c", "master"]);
    }

    #[test]
    fn zero_latency_graph_needs_no_compensation() {
        let g = graph(
            &["a", "b", "master"],
            &[("a", "master"), ("b", "master")],
            &[],
        );
        let plan = plan(&g, "master").expect("无环");
        assert_eq!(plan.longest_path, 0);
        assert!(plan.edge_delay.values().all(|&delay| delay == 0));
    }

    /// 无入边的源节点: `arrival` 必须是 0, 而不是 `None` 或缺键。
    #[test]
    fn source_nodes_have_zero_arrival() {
        let g = graph(&["a", "master"], &[("a", "master")], &[]);
        let plan = plan(&g, "master").expect("无环");
        assert_eq!(plan.arrival["a"], 0);
        assert_eq!(plan.output_latency["a"], 0);
    }

    #[test]
    fn delay_line_bypass_is_a_bit_copy() {
        let mut line = DelayLine::new(0, 2);
        assert!(line.is_bypass());
        let input = [0.1f32, -0.2, 0.3, -0.4];
        let mut out = [f32::NAN; 4];
        line.process(&input, &mut out);
        assert_eq!(
            out.map(f32::to_bits),
            input.map(f32::to_bits),
            "零延迟路径必须是逐位拷贝"
        );
    }

    /// 延迟线语义: 输出晚 `delay` 帧; 前 `delay` 帧是静音。
    #[test]
    fn delay_line_delays_by_exactly_the_requested_frames() {
        let mut line = DelayLine::new(3, 1);
        let mut collected = Vec::new();
        let mut out = [0.0f32; 1];
        for frame in 0..8 {
            line.process(&[frame as f32], &mut out);
            collected.push(out[0]);
        }
        assert_eq!(collected, vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    /// 多声道: 两个声道各自独立延迟, 不得串台。
    #[test]
    fn delay_line_keeps_channels_separate() {
        let mut line = DelayLine::new(2, 2);
        let mut out = [0.0f32; 2];
        line.process(&[1.0, -1.0], &mut out);
        assert_eq!(out, [0.0, 0.0]);
        line.process(&[2.0, -2.0], &mut out);
        assert_eq!(out, [0.0, 0.0]);
        line.process(&[3.0, -3.0], &mut out);
        assert_eq!(out, [1.0, -1.0]);
        line.process(&[4.0, -4.0], &mut out);
        assert_eq!(out, [2.0, -2.0]);
    }

    /// 连续多块处理必须与一次处理等价 —— 否则离线渲染按块切分就会改变输出。
    #[test]
    fn delay_line_is_block_size_invariant() {
        let signal: Vec<f32> = (0..64).map(|i| i as f32).collect();
        let mut whole = vec![0.0f32; 64];
        DelayLine::new(5, 1).process(&signal, &mut whole);

        let mut split = vec![0.0f32; 64];
        let mut line = DelayLine::new(5, 1);
        let mut offset = 0;
        for block in split.chunks_mut(7) {
            let len = block.len();
            line.process(&signal[offset..offset + len], block);
            offset += len;
        }
        assert_eq!(whole, split);
    }

    /// 尾块长度不是帧整数倍时不得越界, 也不得把未初始化数据漏给调用方。
    #[test]
    fn delay_line_tolerates_ragged_blocks() {
        let mut line = DelayLine::new(2, 2);
        let input = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let mut out = [f32::MAX; 5];
        line.process(&input, &mut out);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0, 5.0]);
    }

    /// 两条切片长度不等时按**最短者**处理 (函数文档的承诺), 不得 panic;
    /// `out` 超出共有段的部分**不被写入**。
    ///
    /// 这条判据在旧实现上是**红**的: 尾部透传写的是 `out[tail..]` 与 `input[tail..]`,
    /// 两条切片不等长时长度不匹配 ⇒ `copy_from_slice: source slice length (0) does
    /// not match destination slice length (4)`。
    #[test]
    fn delay_line_takes_the_shortest_of_the_two_slices() {
        // out 比 input 长: 只写 input 有的那一段。
        let mut line = DelayLine::new(2, 2);
        let input = [1.0f32, 2.0, 3.0, 4.0];
        let mut out = [f32::MAX; 8];
        line.process(&input, &mut out);
        assert_eq!(
            out,
            [0.0, 0.0, 0.0, 0.0, f32::MAX, f32::MAX, f32::MAX, f32::MAX]
        );

        // input 比 out 长: 只算 out 放得下的那两帧, 多余的输入被忽略。
        let mut line = DelayLine::new(2, 2);
        let input = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut out = [f32::MAX; 4];
        line.process(&input, &mut out);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0]);
    }

    /// 共有段里的**尾巴** (不足一整帧) 在两条切片长度不等时也逐位透传。
    #[test]
    fn delay_line_passes_a_ragged_tail_through_across_unequal_lengths() {
        let mut line = DelayLine::new(2, 2);
        let input = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let mut out = [f32::MAX; 7];
        line.process(&input, &mut out);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0, 5.0, f32::MAX, f32::MAX]);
    }

    /// 旁路 (零延迟) 同样是"按最短者的逐位拷贝": 尾巴也逐位拷贝, `out` 的其余部分不动。
    ///
    /// 尾巴这一格在旧实现上是**红**的: 旧旁路只拷贝 `frames * channels` 个样本,
    /// 不足一整帧的尾巴**被丢掉** —— 那不是"逐位拷贝", 也与延迟路径的尾巴透传不一致。
    #[test]
    fn delay_line_bypass_copies_the_shortest_length_bit_for_bit() {
        let mut line = DelayLine::new(0, 2);
        let input = [0.1f32, -0.2, 0.3, -0.4, 0.5];
        let mut out = [f32::MAX; 5];
        line.process(&input, &mut out);
        assert_eq!(
            out.map(f32::to_bits),
            input.map(f32::to_bits),
            "把尾巴丢掉就不是逐位拷贝"
        );

        let mut line = DelayLine::new(0, 2);
        let input = [1.0f32, 2.0];
        let mut out = [f32::MAX; 4];
        line.process(&input, &mut out);
        assert_eq!(out, [1.0, 2.0, f32::MAX, f32::MAX]);
    }

    /// `channels == 0` 退化为"丢弃一切": 一个字节都不写, 且**不 panic**。
    ///
    /// 这条判据钉住函数顶部的 `channels == 0` 早退: 少了它, 帧数计算就是 `x / 0`。
    #[test]
    fn delay_line_with_zero_channels_writes_nothing() {
        let mut line = DelayLine::new(4, 0);
        assert_eq!(line.channels(), 0);
        assert!(!line.is_bypass(), "延迟 4 帧, 不是旁路");
        let mut out = [f32::MAX; 3];
        line.process(&[1.0f32, 2.0, 3.0], &mut out);
        assert_eq!(out, [f32::MAX; 3]);
    }
}
