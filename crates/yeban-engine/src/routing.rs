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
//! * `Sidechain` 不搬运音频（本 crate 还没有侧链消费面）⇒ 只有侧链边的节点仍走默认直连；
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

/// 一条出边在实时侧的形态：目的地 + **线性**增益。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteTarget {
    destination: EntityId,
    gain: f32,
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
        let mut outgoing: BTreeMap<EntityId, Vec<RouteTarget>> = BTreeMap::new();
        for edge in graph.edges.values() {
            if edge.kind == RoutingKind::Sidechain {
                continue;
            }
            if edge.source_node == master || !tracks.contains_key(&edge.source_node) {
                continue;
            }
            let gain = edge.gain_db.map_or(1.0, db_to_gain);
            outgoing
                .entry(edge.source_node)
                .or_default()
                .push(RouteTarget {
                    destination: edge.destination_node,
                    gain,
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
                }]
            } else {
                targets
            };
            if targets.iter().any(|target| target.destination != master) {
                uses_buses = true;
            }
            routes.insert(*node, targets);
        }

        Self {
            slots,
            routes,
            uses_buses,
        }
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
        self.slot_of(node).is_some()
            && self.slots.iter().any(|source| {
                self.routes(source)
                    .iter()
                    .any(|route| route.destination() == *node)
            })
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
