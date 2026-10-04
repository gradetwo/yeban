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
//! ## 为什么用**全局** `L_max` 而不是"每个求和节点各自对齐"
//!
//! 两者给出的**相对**对齐完全相同; 全局形式额外给整条链引入一个统一的 `L_max`
//! 前滚延迟, 这正是规范字面要求的形式 ("确定最长延迟关键路径 L_max ... D_i = L_max − L_i")。
//! 母线求和的正确性不受影响: 每条分支到达任一求和节点时都恰好是 `L_max`。
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
    /// 最长延迟关键路径 `L_max` = Master 的 `output_latency`。
    pub longest_path: u32,
    /// 每条边需要插入的补偿延迟 `D_e = L_max − output_latency[source]`。
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
        edge_delay.insert(
            (source, destination),
            longest_path.saturating_sub(source_latency),
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
    /// 输出比输入晚 `delay_frames` 帧; 长度按最短者处理, 不越界、不分配。
    /// 延迟为 0 时是**逐位拷贝** —— 旁路路径不得引入任何浮点运算, 否则确定性
    /// 契约会在"有/无 PDC"之间出现 LSB 差异。
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let frames = input.len().min(out.len()) / self.channels.max(1);
        if self.is_bypass() || self.channels == 0 {
            let end = frames * self.channels;
            out[..end].copy_from_slice(&input[..end]);
            return;
        }
        let channels = self.channels;
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
        let tail = frames * self.channels;
        out[tail..].copy_from_slice(&input[tail..]);
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

    /// 串接: 两级各 10 帧 + 母线自身 5 帧 => L_max = 25, 源节点补偿 25。
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
        assert_eq!(plan.longest_path, 25);
        assert_eq!(plan.delay_of("a", "b"), Some(15));
        assert_eq!(plan.delay_of("b", "master"), Some(5));
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
}
