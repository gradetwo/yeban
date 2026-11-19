//! ⭐ **实时路由计划**（任务单 `#154`）：把 `RoutingGraph` 投影成"每个可渲染节点
//! 把它的输出送到哪里、乘多少增益"。
//!
//! ## 为什么需要它（本单之前的事实）
//!
//! 模型、Op 与 MCP 都能写 `routing_graph`（`SendToAux` / `SetRoutingGain`），
//! 但 `crate::rt` 的逐轨循环把**每一条**非 master 轨都直接求和进同一条立体声母线
//! （`grep -rn "SendToAux" crates/yeban-engine/src` 在本次改动前是 **0 命中**）
//! ⇒ 发送写得进、听不出。本模块把图的**边**编译成实时侧可以线性扫描的纯数据。
//!
//! ## 语义（本片的最小切片）
//!
//! * 节点的**出边**（除 `Sidechain` 外）就是它的去向：
//!   `TrackToBus` / `BusToMaster` / `SendToAux` 都按 `source_node → destination_node`
//!   搬运**声相之后**的立体声信号，增益 = 边的 `gain_db` 的**线性**值
//!   （`None` = 单位增益；`-120 dB` 及以下 = `0.0`，与 [`yeban_dsp::math::db_to_gain`]
//!   同一把尺子）；
//! * ⭐ **没有音频出边的节点默认直连 master**（`gain = 1.0`）—— 这是**接线前的行为**，
//!   也是"没有 route 边 ⇒ 输出逐位不变"的来源；
//! * `Sidechain` 不搬运音频 ⇒ 只有侧链边的节点仍走默认直连；但它**可以**是一条
//!   键路由：[`RoutingPlan::key_route`] 在编译期按 v1 规则（源确定在循环里跑链 ＋
//!   实体序 ＋ 首边胜出）算出"目的 → 键源抽头槽位"，实时侧逐块消费；
//! * 目的地是**另一条非 master 轨** ⇒ 实时的立体声暂存（总线池）按拓扑序累积后转发；
//!   `master` 是求和终点，不参与"被路由"这一侧。
//! * ⭐ **`#158`**：目的地总线若有**已武装的插入器件**，实时侧在**转发之前**跑它的
//!   插入链（[`crate::rt::render_branch`] 的 3a⁗；见 `crate::rt` 模块头的裁决）。
//!   本计划只负责"边"的投影，**不持有**器件状态；它提供的
//!   [`RoutingPlan::has_incoming_audio`] 是给那条路径的**零分配谓词**
//!   （"这条节点真的收到音频吗"）。
//!
//! ## 确定性
//!
//! [`RoutingPlan::slots`] 用 `PdcPlan::order()` 的**拓扑序**（Kahn，键序 ⇒ 跨进程确定）
//! 过滤出可渲染节点；补集（在 `tracks` 里但不在图里）按键序追加。
//! [`RoutingPlan::routes`] 里每个节点的目标顺序来自 `RoutingGraph::edges` 的
//! `BTreeMap` 迭代（边键升序）⇒ 同一条工程两次构造得到逐位相同的计划
//! [MODEL-AST-003]、[ARCH-DET-002]。
//!
//! ## 实时约束
//!
//! 构造期允许分配（每节点一个 `Vec`）；实时侧只做**定长数组 + 线性扫描**，
//! 不分配、不加锁、不做 I/O [MUST-GATE-001]。

use std::collections::BTreeMap;

use yeban_dsp::math::db_to_gain;
use yeban_model::{EntityId, RoutingGraph, RoutingKind};

use crate::snapshot::TrackParams;
use crate::synth::MAX_TRACK_SLOTS;

/// 一条出边在实时侧的形态：目的地 + **线性**增益 ＋ ⭐ **边的身份**。
///
/// ## `edge` 为什么是 `Option`（任务单 `#261`）
///
/// 发送量槽位的地址是 `ParamAddress { entity: <边的 `EntityId`>, slot: 4 }`
/// （[`crate::param::SEND_GAIN_SLOT`]）—— "乘哪一份增益"落在**图里的一条边**上，
/// ⛔ 不是落在一条轨上。本结构体在 `#261` 之前只有 `(destination, gain)`：
/// 目的地相同的两条边（例如"同一条轨既发送到 aux、又直连 master"里的发送边与直连边）
/// 在这里**无法区分**，而"只给发送那条边发自动化、直连那一路一位不动"
/// 正需要这个区分（`#258` 的否决判据）。
///
/// * `Some(id)` ＝ 这条目标来自 `RoutingGraph::edges` 的一条**真实边**，`id` 是它的身份；
/// * `None`     ＝ 计划补出来的**默认项**（"没有音频出边 ⇒ 直连 master、单位增益"）。
///   它没有任何边身份 ⇒ 没有可寻址的发送量槽位（⛔ 不发明一个假边号）。
///
/// ⚠ 加字段**不改变任何既有语义**：`destination` / `gain` 的取值与顺序一位没动
/// ⇒ 没有 `SendGain` 事件的工程输出逐位不变。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteTarget {
    destination: EntityId,
    gain: f32,
    edge: Option<EntityId>,
}

impl RouteTarget {
    /// 目的地节点身份。
    #[must_use]
    pub const fn destination(&self) -> EntityId {
        self.destination
    }

    /// 线性增益（构造期由 `gain_db` 换算）。
    #[must_use]
    pub const fn gain(&self) -> f32 {
        self.gain
    }

    /// ⭐ 这条目标对应的**路由边身份**（任务单 `#261`）。
    ///
    /// `None` = 默认项（计划补出来的"直连 master、单位增益"）。实时侧的发送量
    /// 武装表只收 `Some` 的项 ⇒ 默认项**永远**走静态增益。
    #[must_use]
    pub const fn edge(&self) -> Option<EntityId> {
        self.edge
    }
}

/// ⭐ 一条边是否会被 [`RoutingPlan::compile`] 编译成**能搬运音频的出边**。
///
/// 三条过滤条件（与 `compile` 里的判断**同字**）：
/// 1. `kind != Sidechain`（侧链不搬音频）；
/// 2. `source_node != master`（母带是求和终点，不作为源）；
/// 3. `source_node` 是一条**可渲染的音轨**（`is_track`）。
///
/// ## 为什么把它抽成公开函数（任务单 `#261`）
///
/// 发送量槽位落地之后，`yeban-app` 要回答两个问题：
/// ① 这条 `SendGain` 泳道的边在**实时侧有没有落点**（⇒
///    `unapplied_automation_lanes` 的 `send_gain` 读数）；② 要不要为它下发事件
///    （没有落点就不发 —— 与声相跳过母线同一条先例）。两个答案必须与**引擎的
///    武装表**同一把尺子 ⇒ 过滤条件只留一份（`compile` 调它，控制侧也调它）。
///
/// ⚠ 容量（[`crate::param::SEND_SLOTS`]）**不在**本谓词里：武装表按计划的确定性
/// 顺序装前 `SEND_SLOTS` 条边，装不下的部分由引擎自己的 `send_capacity_drops`
/// 计数（⛔ 控制侧不复制一份"能装几条"的算术）。
#[must_use]
pub fn edge_carries_audio(
    edge: &yeban_model::RoutingEdge,
    master: EntityId,
    is_track: impl Fn(EntityId) -> bool,
) -> bool {
    edge.kind != RoutingKind::Sidechain && edge.source_node != master && is_track(edge.source_node)
}

/// `RoutingGraph` 的**实时可消费**投影。
///
/// 字段全部私有 + 只有 getter（与 [`crate::snapshot::EngineSnapshot`] 同一条纪律）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoutingPlan {
    /// 可渲染节点（不含 master），按 PDC 拓扑序；前 `len` 项有效，上限
    /// [`MAX_TRACK_SLOTS`]（与声部池/总线池同源）。
    slots: Vec<EntityId>,
    /// 每个节点的出边（含"没有出边 ⇒ 直连 master"的默认项）。
    routes: BTreeMap<EntityId, Vec<RouteTarget>>,
    /// 是否存在**非 master** 的目的地 ⇒ 实时侧是否需要总线暂存池。
    uses_buses: bool,
    /// 侧链键路由（v1 单键）：目的节点 -> 源节点在抽头池里的槽位。
    /// 每目的至多一条（边身份升序第一条**合格**边生效）；构造期算好，逐块零分配。
    key_routes: BTreeMap<EntityId, usize>,
    /// 键抽头源：源节点 -> 它在抽头池里的槽位（与 `key_routes` 的值同一把尺子）。
    /// 逐轨循环只给这些节点写抽头。
    key_sources: BTreeMap<EntityId, usize>,
    /// ⭐ 逐件侧链（期 5）：**同轨内**组 → 组的键路由（节点 -> 已求值路由表，
    /// 按目的组至多一条）。由 [`Self::compile_lane_routes`] 在节点级编译之后
    /// 单独求值（需要快照的 lane 布局投影，`compile` 本身看不到设备链）。
    lane_key_routes: BTreeMap<EntityId, Vec<LaneKeyRoute>>,
    /// ⭐ 逐件侧链（期 5）：lane 键抽头源 `(节点, 源组下标)` -> 抽头池槽位
    /// （与离线 `LaneKeyRoute` 的槽位分配同一把尺子：边身份升序、首次出现序）。
    lane_key_sources: BTreeMap<(EntityId, usize), usize>,
    /// ⭐ 带 lane 端点（`source_lane` / `dest_lane` 任一非空）的 Sidechain 边总数
    /// （构造期计数，响亮读数的一半；见 [`Self::lane_edges_evaluated`]）。
    lane_edges_declared: usize,
    /// ⭐ 其中被 [`Self::compile_lane_routes`] 真求值的条数；
    /// `declared - evaluated` = 未求值（跨轨 / 混合端点 / 键不属组 / 目的组未武装），
    /// ⛔ 不静默：快照武装时累计进 `EngineStats` 的 lane 读数。
    lane_edges_evaluated: usize,
}

/// ⭐ 一条已求值的**逐件**键路由（期 5）：同一条轨内，`source_group` 组的
/// **链前**渲染作 `dest_group` 组通道条的检波键。
///
/// 求值条件与离线 `yeban-render` 的 `attach_lane_key_routes` **逐条同口径**
/// （设计稿 §3.3 的 R4 定案）：同节点、两端都带 lane 键、两键各属一组、
/// 目的组静态武装、同目的组第一条合格边。键增益在构造期由边 `gain_db`
/// 线性化（缺省 1.0），逐块路径只读。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneKeyRoute {
    /// 源组在该轨 lane 组表里的下标。
    source_group: usize,
    /// 目的组在该轨 lane 组表里的下标。
    dest_group: usize,
    /// 键增益（线性；构造期由 `gain_db` 换算，缺省 1.0）。
    key_gain: f32,
    /// 来源边的身份（诊断与判据对账用）。
    edge: EntityId,
}

impl LaneKeyRoute {
    /// 源组下标。
    #[must_use]
    pub const fn source_group(&self) -> usize {
        self.source_group
    }

    /// 目的组下标。
    #[must_use]
    pub const fn dest_group(&self) -> usize {
        self.dest_group
    }

    /// 键增益（线性）。
    #[must_use]
    pub const fn key_gain(&self) -> f32 {
        self.key_gain
    }

    /// 来源边身份。
    #[must_use]
    pub const fn edge(&self) -> EntityId {
        self.edge
    }
}

/// ⭐ 一轨的 lane 布局投影（期 5 键求值的构造期输入）：每组的键集 + 目的组
/// 静态武装位。与离线 `yeban-render` 的 `LaneKeyLayout` **同形同源**（都由
/// 快照构造期从 `TrackV3.lane_chains` 投影；武装位 = 组链投影的通道条
/// `compressor_enabled`，见设计稿 §3.3 条件 4）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaneKeyLayout {
    /// 逐组的键集（组序 = `lane_chains` 原序）。
    pub groups: Vec<Vec<u8>>,
    /// 与 `groups` 同序：该组作目的时是否静态武装（链会执行压缩）。
    pub armed_groups: Vec<bool>,
}

/// `node` 是否会收到别的节点搬来的音频（[`RoutingPlan::has_incoming_audio`] 的
/// 唯一实现：那条 getter 与 `compile` 的键路由计算共用它，⛔ 不各写一份）。
///
/// 口径与 getter 的文档一致：只有**在总线暂存池里**的节点可能收到音频
/// （`slots` 里有它）；自己指向自己的边也算。
#[must_use]
fn node_has_incoming_audio(
    slots: &[EntityId],
    routes: &BTreeMap<EntityId, Vec<RouteTarget>>,
    node: &EntityId,
) -> bool {
    slots.contains(node)
        && slots.iter().any(|source| {
            routes
                .get(source)
                .is_some_and(|targets| targets.iter().any(|route| route.destination() == *node))
        })
}

impl RoutingPlan {
    /// 编译一份计划。
    ///
    /// - `graph`：模型层的唯一声学路由真理源 [MODEL-AST-004]；
    /// - `master`：主总线节点（求和终点，不作为源）；
    /// - `tracks`：可渲染的音轨集合（`EngineSnapshot::tracks` 的投影）；
    /// - `order`：PDC 的拓扑序（`PdcPlan::order()`）。
    ///
    /// `order` 里没有的音轨（例如不在 `routing_graph.nodes` 里、因而没进 PDC 计划的轨）
    /// 按键序追加到 [`Self::slots`] 尾部：它们仍然可渲染，只是图里没有它们的位置。
    #[must_use]
    pub fn compile(
        graph: &RoutingGraph,
        master: EntityId,
        tracks: &BTreeMap<EntityId, TrackParams>,
        order: &[EntityId],
    ) -> Self {
        let mut slots: Vec<EntityId> = Vec::with_capacity(tracks.len().min(MAX_TRACK_SLOTS));
        for node in order {
            if *node == master || !tracks.contains_key(node) {
                continue;
            }
            if slots.len() >= MAX_TRACK_SLOTS {
                break;
            }
            slots.push(*node);
        }
        for node in tracks.keys() {
            if slots.len() >= MAX_TRACK_SLOTS {
                break;
            }
            if *node == master || slots.contains(node) {
                continue;
            }
            slots.push(*node);
        }

        // --- 出边：只保留**能搬运音频**的三种 kind；Sidechain 不搬音频 ---
        //
        // ⚠ 过滤条件与 [`edge_carries_audio`] **同一份**：那张公开谓词是控制侧
        // （`yeban-app` 的 `SendGain` 泳道采样）与这里的唯一共同尺子。
        let mut outgoing: BTreeMap<EntityId, Vec<RouteTarget>> = BTreeMap::new();
        for edge in graph.edges.values() {
            if !edge_carries_audio(edge, master, |node| tracks.contains_key(&node)) {
                continue;
            }
            let gain = edge.gain_db.map_or(1.0, db_to_gain);
            outgoing
                .entry(edge.source_node)
                .or_default()
                .push(RouteTarget {
                    destination: edge.destination_node,
                    gain,
                    // ⭐ `#261`：把**边身份**带进实时计划。它是发送量槽位的地址
                    // （`ParamAddress { entity: <边>, slot: SEND_GAIN_SLOT }`），
                    // 也是"同目的地两条边各自独立"的唯一区分依据。
                    edge: Some(edge.id),
                });
        }

        // --- 默认项：没有**音频**出边的可渲染节点 ⇒ 直连 master（接线前的行为）---
        let mut routes: BTreeMap<EntityId, Vec<RouteTarget>> = BTreeMap::new();
        let mut uses_buses = false;
        for node in &slots {
            let targets = outgoing.remove(node).unwrap_or_default();
            let targets = if targets.is_empty() {
                vec![RouteTarget {
                    destination: master,
                    gain: 1.0,
                    // 计划补出来的项没有边身份 ⇒ 没有可寻址的发送量槽位。
                    edge: None,
                }]
            } else {
                targets
            };
            if targets.iter().any(|target| target.destination != master) {
                uses_buses = true;
            }
            routes.insert(*node, targets);
        }

        // --- 键路由（侧链 v1）：构造期算一次，逐块零分配 ---
        //
        // 保守规则（编译期没有武装信息时恒正确）：源必须**确定**在逐轨循环里跑链 ——
        // 无入边（`process_bus_inserts` 的第一个条件已假）或零器件（武装表恒空）；
        // 有入边且有器件的源可能在转发阶段跑链 ⇒ 排除（离线渲染覆盖这类源）。
        // 目的 = 轨或 master；轨目的要求源的实体序**小于**目的（逐轨循环按键序），
        // master 目的不受此限（主总线链在全部循环与转发之后）。
        // 每目的只取边身份升序第一条**合格**边（`graph.edges` 本就是 BTreeMap 序；
        // 不合格的边不占位，后面的边仍可生效）。
        let slot_index: BTreeMap<EntityId, usize> = slots
            .iter()
            .enumerate()
            .map(|(index, node)| (*node, index))
            .collect();
        let mut key_routes: BTreeMap<EntityId, usize> = BTreeMap::new();
        let mut key_sources: BTreeMap<EntityId, usize> = BTreeMap::new();
        let mut lane_edges_declared = 0usize;
        for edge in graph.edges.values() {
            if edge.kind != RoutingKind::Sidechain {
                continue;
            }
            // ⭐ 逐件侧链（期 5）：任一端带 lane 键的边**不**在节点级求值 ——
            // 它的键是某一组的链前渲染，不是整轨信号；在这里当节点边求值会
            // 与 lane 求值双重计数（离线 `attach_key_armed_nodes` 同口径）。
            // 这类边只计数（declared），求值在 `compile_lane_routes`。
            if edge.source_lane.is_some() || edge.dest_lane.is_some() {
                lane_edges_declared += 1;
                continue;
            }
            let source = edge.source_node;
            let dest = edge.destination_node;
            if source == master {
                continue;
            }
            if dest != master && !tracks.contains_key(&dest) {
                continue;
            }
            let Some(&source_slot) = slot_index.get(&source) else {
                continue;
            };
            if node_has_incoming_audio(&slots, &routes, &source)
                && tracks.get(&source).map_or(0, TrackParams::device_count) > 0
            {
                continue;
            }
            if dest != master && !(source < dest) {
                continue;
            }
            if key_routes.contains_key(&dest) {
                continue;
            }
            key_routes.insert(dest, source_slot);
            key_sources.insert(source, source_slot);
        }

        Self {
            slots,
            routes,
            uses_buses,
            key_routes,
            key_sources,
            lane_key_routes: BTreeMap::new(),
            lane_key_sources: BTreeMap::new(),
            lane_edges_declared,
            lane_edges_evaluated: 0,
        }
    }

    /// ⭐ 逐件键路由求值（期 5）：在节点级编译之后、由快照构造期拿着 lane
    /// 布局投影调用。条件与离线 `attach_lane_key_routes` **逐条同口径**：
    ///
    /// 1. 边是 Sidechain 且**两端都带 lane 键**（只带一端的混合边不求值）；
    /// 2. 源节点 == 目的节点（跨轨 lane 边不求值 —— 离线同口径，键缓冲是
    ///    轨内两遍渲染的产物，跨轨没有对应的抽头时机）；
    /// 3. 源键与目的键各属该轨布局里的一组；
    /// 4. 目的组静态武装（其链投影会执行压缩）；
    /// 5. 同一目的组只取边身份升序第一条合格边。
    ///
    /// 未求值的边不占位、后面的边仍可生效；总数账在
    /// [`Self::lane_edges_declared`] / [`Self::lane_edges_evaluated`]。
    pub fn compile_lane_routes(
        &mut self,
        graph: &RoutingGraph,
        layouts: &BTreeMap<EntityId, LaneKeyLayout>,
    ) {
        let mut routes: BTreeMap<EntityId, Vec<LaneKeyRoute>> = BTreeMap::new();
        let mut sources: BTreeMap<(EntityId, usize), usize> = BTreeMap::new();
        let mut evaluated = 0usize;
        for edge in graph.edges.values() {
            if edge.kind != RoutingKind::Sidechain {
                continue;
            }
            let (Some(source_key), Some(dest_key)) = (edge.source_lane, edge.dest_lane) else {
                continue;
            };
            if edge.source_node != edge.destination_node {
                continue;
            }
            let track = edge.source_node;
            let Some(layout) = layouts.get(&track) else {
                continue;
            };
            let Some(source_group) = layout
                .groups
                .iter()
                .position(|keys| keys.contains(&source_key))
            else {
                continue;
            };
            let Some(dest_group) = layout
                .groups
                .iter()
                .position(|keys| keys.contains(&dest_key))
            else {
                continue;
            };
            if !layout
                .armed_groups
                .get(dest_group)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let dest_routes = routes.entry(track).or_default();
            if dest_routes
                .iter()
                .any(|route| route.dest_group == dest_group)
            {
                continue;
            }
            if !sources.contains_key(&(track, source_group)) {
                let slot = sources.len();
                sources.insert((track, source_group), slot);
            }
            dest_routes.push(LaneKeyRoute {
                source_group,
                dest_group,
                key_gain: edge.gain_db.map_or(1.0, db_to_gain),
                edge: edge.id,
            });
            evaluated += 1;
        }
        self.lane_key_routes = routes;
        self.lane_key_sources = sources;
        self.lane_edges_evaluated = evaluated;
    }

    /// 可渲染节点（不含 master），按 PDC 拓扑序。
    #[must_use]
    pub fn slots(&self) -> &[EntityId] {
        &self.slots
    }

    /// 某个节点的出边（含默认直连 master 的项）；不在计划里的节点返回空切片。
    #[must_use]
    pub fn routes(&self, node: &EntityId) -> &[RouteTarget] {
        self.routes.get(node).map_or(&[], Vec::as_slice)
    }

    /// 目的地节点在总线暂存池里的下标（不在池里 ⇒ `None`）。
    #[must_use]
    pub fn slot_of(&self, node: &EntityId) -> Option<usize> {
        self.slots.iter().position(|candidate| candidate == node)
    }

    /// 是否存在**非 master** 的目的地 ⇒ 实时侧要跑总线暂存/转发那一段。
    ///
    /// `false` ⇒ 逐轨循环的输出路径与接线前**逐位相同**（默认直连 master）。
    #[must_use]
    pub const fn uses_buses(&self) -> bool {
        self.uses_buses
    }

    /// `node` 是否会**收到**别的节点搬来的音频（即它是某条出边的目的地）。
    ///
    /// ## 用途（任务单 `#158`）
    ///
    /// 总线上的插入器件只在**真的收到发送**时才有新的活要干：一条有器件却没人送
    /// 信号的总线，它的插入链在逐轨循环里已经按**它自己的声源**跑过（接线前的行为），
    /// 转发阶段不需要第二遍。判据因此把这个谓词与"该节点有已武装的器件"合取，
    /// 决定是否把插入链搬到转发阶段（见 [`crate::rt::render_branch`] 的 3a⁗）。
    ///
    /// ## 口径
    ///
    /// * 只有**在总线暂存池里**的节点可能收到音频（[`Self::slot_of`] 为 `Some`）——
    ///   目的地不在池里的边在实时侧的搬运阶段**本来就被跳过**，没有可处理的信号；
    /// * 自己指向自己的边也算（图不禁止，搬运阶段会把它加进暂存，本谓词与之一致）。
    ///
    /// ## 实时约束
    ///
    /// `O(节点数 × 出边数)`（上限 16 × 16 级别的定长扫描），只有整数比较与切片遍历：
    /// **零分配、零锁、零 I/O** [MUST-GATE-001]。调用点被 `uses_buses()` 短路 ⇒
    /// 默认工程（所有节点直连 master）一次都不调用。
    #[must_use]
    pub fn has_incoming_audio(&self, node: &EntityId) -> bool {
        node_has_incoming_audio(&self.slots, &self.routes, node)
    }

    /// 目的节点的键源抽头槽位（侧链 v1）：`None` = 该目的没有已求值的键路由
    /// ⇒ 目的的压缩器走节目检波。
    #[must_use]
    pub fn key_route(&self, dest: &EntityId) -> Option<usize> {
        self.key_routes.get(dest).copied()
    }

    /// 源节点是不是某条已求值键路由的源；是 ⇒ 返回它在抽头池里的槽位
    /// （逐轨循环只给这些节点写抽头）。
    #[must_use]
    pub fn key_tap_slot(&self, source: &EntityId) -> Option<usize> {
        self.key_sources.get(source).copied()
    }

    /// ⭐ 某轨已求值的逐件键路由（期 5）；没有 ⇒ 空切片（该轨无 lane 键控）。
    #[must_use]
    pub fn lane_key_routes(&self, track: &EntityId) -> &[LaneKeyRoute] {
        self.lane_key_routes.get(track).map_or(&[], Vec::as_slice)
    }

    /// ⭐ 某轨某个目的组的逐件键路由（期 5）；没有 ⇒ 该组走节目检波。
    #[must_use]
    pub fn lane_key_route_for(&self, track: &EntityId, dest_group: usize) -> Option<&LaneKeyRoute> {
        self.lane_key_routes
            .get(track)?
            .iter()
            .find(|route| route.dest_group == dest_group)
    }

    /// ⭐ `(轨, 源组)` 是不是某条已求值逐件键路由的源；是 ⇒ 它在 lane 抽头池
    /// 里的槽位（逐轨循环第一遍只给这些组写抽头）。
    #[must_use]
    pub fn lane_key_tap_slot(&self, track: &EntityId, source_group: usize) -> Option<usize> {
        self.lane_key_sources.get(&(*track, source_group)).copied()
    }

    /// ⭐ lane 抽头池需要的槽位总数（构造期定尺寸用）。
    #[must_use]
    pub fn lane_key_tap_slots(&self) -> usize {
        self.lane_key_sources.len()
    }

    /// ⭐ 带 lane 端点的 Sidechain 边总数（声明数；响亮读数）。
    #[must_use]
    pub const fn lane_edges_declared(&self) -> usize {
        self.lane_edges_declared
    }

    /// ⭐ 其中被真求值的条数；`declared - evaluated` 即未求值数（⛔ 不静默）。
    #[must_use]
    pub const fn lane_edges_evaluated(&self) -> usize {
        self.lane_edges_evaluated
    }

    /// 计划里的节点数（判据/诊断用）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// 是否没有任何可渲染节点。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
