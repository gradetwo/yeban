//! T-FIX-52 回归：路由音频子图与 folder 父子关系都必须在成环时**响亮拒绝**，
//! 且错误报文给出闭合路径；合法图（含侧链键输入）逐位不变。
//!
//! 口径：
//! - `TrackToBus` / `BusToMaster` / `SendToAux` 是音频连接，参与判环；
//! - `Sidechain` 是键输入、不搬运音频，**不参与**判环；
//! - 环路径首尾为同一节点，节点用规范 ULID 文本逐字断言。

use std::collections::BTreeMap;
use std::str::FromStr;

use yeban_model::samples::filled_project;
use yeban_model::{
    EntityId, ModelError, Op, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};

fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

fn edge(
    edge_id: EntityId,
    source: EntityId,
    destination: EntityId,
    kind: RoutingKind,
) -> RoutingEdge {
    RoutingEdge {
        id: edge_id,
        source_node: source,
        destination_node: destination,
        kind,
        gain_db: None,
        source_lane: None,
        dest_lane: None,
    }
}

fn assert_routing_cycle(graph: &RoutingGraph, expected: &[EntityId]) {
    let error = graph.validate().expect_err("音频路由成环必须被拒绝");
    let ModelError::RoutingCycle { path } = &error else {
        panic!("拒绝理由必须是 RoutingCycle，实测为 {error:?}");
    };
    assert_eq!(path.as_slice(), expected, "闭合路径不符: {path:?}");
    assert_eq!(path.first(), path.last(), "路径必须首尾闭合: {path:?}");
    let message = error.to_string();
    for node in expected {
        assert!(
            message.contains(&node.to_canonical_string()),
            "错误报文必须点名环上节点 {node}: {message}"
        );
    }
}

/// **自环**：单节点 `a → a` 必须拒绝，路径 `[a, a]`。
#[test]
fn a_routing_self_loop_is_rejected_with_its_path() {
    let a = id(1);
    let graph = RoutingGraph {
        nodes: vec![a],
        edges: BTreeMap::from([(id(10), edge(id(10), a, a, RoutingKind::TrackToBus))]),
    };
    assert_routing_cycle(&graph, &[a, a]);
}

/// **两节点环**：`a → b → a` 必须拒绝，路径 `[a, b, a]`。
#[test]
fn a_two_node_routing_cycle_is_rejected_with_its_path() {
    let (a, b) = (id(1), id(2));
    let graph = RoutingGraph {
        nodes: vec![a, b],
        edges: BTreeMap::from([
            (id(10), edge(id(10), a, b, RoutingKind::TrackToBus)),
            (id(11), edge(id(11), b, a, RoutingKind::TrackToBus)),
        ]),
    };
    assert_routing_cycle(&graph, &[a, b, a]);
}

/// **侧链不是音频边**：音频 `a → b` 加侧链 `b → a` 仍是合法无环图。
#[test]
fn a_sidechain_key_input_does_not_count_as_an_audio_cycle() {
    let (a, b) = (id(1), id(2));
    let graph = RoutingGraph {
        nodes: vec![a, b],
        edges: BTreeMap::from([
            (id(10), edge(id(10), a, b, RoutingKind::TrackToBus)),
            (id(11), edge(id(11), b, a, RoutingKind::Sidechain)),
        ]),
    };
    graph.validate().expect("侧链是键输入，不得被当成音频回边");
}

/// **成环操作**：`ConnectRouting` 在落盘前拒绝第二条闭环边，文档保持不变。
#[test]
fn connect_routing_rejects_a_cycle_before_committing() {
    let mut project = filled_project();
    let (a, b) = (id(9_001), id(9_002));
    project.routing_graph.nodes.push(a);
    project.routing_graph.nodes.push(b);
    project.routing_graph.sort_nodes();

    let a_to_b = edge(id(20), a, b, RoutingKind::TrackToBus);
    Op::ConnectRouting { edge: a_to_b }
        .apply(&mut project)
        .expect("第一条边不产生环，必须放行");

    let before = project.clone();
    let b_to_a = edge(id(21), b, a, RoutingKind::TrackToBus);
    let error = Op::ConnectRouting { edge: b_to_a }
        .apply(&mut project)
        .expect_err("第二条边闭合 a→b→a，必须在落盘前拒绝");
    let ModelError::RoutingCycle { path } = &error else {
        panic!("拒绝理由必须是 RoutingCycle，实测为 {error:?}");
    };
    assert_eq!(path.as_slice(), &[a, b, a], "路径不符: {path:?}");
    assert_eq!(project, before, "拒绝后文档必须逐字段不变");
}

/// **folder 构建面**：`AddTrack` 自指 `folder_id` 在落盘前拒绝。
#[test]
fn add_track_rejects_a_self_referential_folder_link() {
    let mut project = filled_project();
    let me = id(30);
    let mut track = TrackV3 {
        id: me,
        name: "Loop".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.folder_id = Some(me);
    let before = project.clone();
    let error = Op::AddTrack { track }
        .apply(&mut project)
        .expect_err("新增轨自指 folder_id 必须在落盘前拒绝");
    let ModelError::FolderCycle { path } = &error else {
        panic!("拒绝理由必须是 FolderCycle，实测为 {error:?}");
    };
    assert_eq!(path.as_slice(), &[me, me], "路径不符: {path:?}");
    assert_eq!(project, before, "拒绝后文档必须逐字段不变");
}

/// 合法工程（既有 4 轨 / 主总线可达）在新增判环后仍逐位通过。
#[test]
fn the_untouched_fixture_still_validates_after_cycle_checks() {
    let project: YebanProjectV1 = filled_project();
    project
        .validate()
        .expect("合法夹具必须逐位不变地通过新增判环");
}
