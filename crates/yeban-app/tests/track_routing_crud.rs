//! ⭐ 任务单 `#160` 的判据：**从界面建轨 / 删轨 / 删发送 / 防重复发送**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 加 MIDI 轨 = Δ1 | 点击一次 ⇒ `commit_count +1`、`tracks +1`、新轨 `kind == Midi`、**有一条到 master 的 `TrackToBus` 边**、`nodes +1`；`TrackRouting` 引擎计划里该轨的目的地是 master、线性增益 1.0 | 证"一次点击 = 一步可撤销的编辑，且新轨在路由图里真的接上了" |
//! | ② 加 AuxReturn = Δ1 | `demo_project`（0 条 aux）⇒ `send-create-available == false`，建一条 aux 轨之后 **Δ1** 且 `send-create-available` 从 `false` 变 `true`；随后一次 `send-create` 再 Δ1 | 证"灰按钮变可用不是画上去的" |
//! | ③ 删轨 = Δ1 | `filled_project` 的展示轨（Lead）有 2 条边 ⇒ 删除后 `tracks −1`、`edges −2`、`nodes −1`、轨数读数 `4 → 3` | 证"删轨把引用它的边 / 节点一起收干净"（否则 `RemoveRoutingNode` 会被 `RoutingNodeInUse` 拒掉） |
//! | ④ 主总线不可删 | 只剩主总线的工程 ⇒ `track-delete-available == false`、原因逐字 `主总线（Master）不可删除`；直接调用回调也 **Δ0**；`track_delete_ops` 返回 `Err(同一个原因)` | 证"禁用不是只画个灰" |
//! | ⑤ 删发送 = Δ1 | `filled_project` 的 1 条发送 ⇒ **Δ1**、`edges −1`、`send-*-row` 归零（`无发送`）、`send-create` 重新可用；同一条边再删一次 ⇒ **Δ0** | 证"删发送是真实边删除，且删完不留悬空按钮" |
//! | ⑥ 防重复发送 | 已有同源同目的地的 `SendToAux` ⇒ 按钮禁用、原因逐字 `该发送已存在（源与目的地相同），无法重复新建`；再点一次 **Δ0**、边数不变 | 证"`#159` 登记的'连点会各建一条新边'被关掉" |
//! | ⑦ 预算三件套 | 不在本文件：读数在 `tests/perf_draw_budget.rs`（`APPLY_VIEW_WRITE_SITE_BUDGET` **65** / `PER_FRAME_ARRAY_REWRITE_BUDGET` **42**，常量与原因同处） | — |
//! | ⑧ 不退化 | `cargo test -p yeban-app --locked --no-fail-fast` rc 与 passed 数在交付报告里 | — |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `track_create_ops` 里的 `ConnectRouting` 摘掉 ⇒ ① 的"有一条到 master 的边"红；
//! - 把 `track_delete_ops` 里的 `DisconnectRouting` 循环摘掉 ⇒ ③ 提交会被
//!   `RemoveRoutingNode` 的前置条件拒掉（`commit_count` 不动）；
//! - 把 `send_create_availability` 的重复分支删掉 ⇒ ⑥ 的 `+0` 变 `+1`；
//! - 把 `host::wire_track_strip` 的 `commit_ops` 摘掉 ⇒ ①②③ 的 `commit_count` 差 0；
//! - 把 `track_delete_availability` 的主总线分支删掉 ⇒ ④ 的原因与可用性红。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"人耳听到声音"：本判据只证工程里多了一条到 master 的边、引擎的
//! `RoutingPlan` 把它编译成 `source → master`。⛔ 不证"0 轨空工程能建轨"：`Op` 全集
//! 没有写 `master_bus_track_id` 的变体 ⇒ 那一路被 `TRACK_CREATE_NO_MASTER_REASON`
//! 响亮拒绝（本文件不断言它之外的行为）。

use std::collections::BTreeSet;
use std::rc::Rc;
use std::str::FromStr as _;

use slint::Model as _;
use yeban_app::bridge::{
    NewTrackKind, SEND_CREATE_DUPLICATE_REASON, SEND_CREATE_NO_AUX_REASON,
    SEND_CREATE_NO_TRACK_REASON, TRACK_DELETE_MASTER_REASON, TRACK_DELETE_NO_TRACK_REASON,
    ViewState, master_bus_label, send_create_availability, track_create_available,
    track_create_ops, track_delete_ops, track_delete_reason,
};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1};
use yeban_model::samples::{default_project, demo_project, filled_project};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸：比既有 Tier-1 判据高 100px —— 轨 / 路由段与发送段同住一块面板，
/// 面板在默认 900px 窗口里已经很满；这里给判据自己留出可见性余量。
const WINDOW: Size = Size::new(1440, 1000);

/// 轨 / 路由段源码（**另一个文件**）—— 第 293 条：判据钉住的字符串必须从别处读。
const SLINT_SOURCE: &str = include_str!("../ui/console/device_rack.slint");
/// 窗口回调声明（驱动链在 `host::wire_track_strip` / `host::wire_send_strip`）。
const APP_SOURCE: &str = include_str!("../ui/app.slint");

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

fn live_window(view: &ViewState) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 一条带撤销端口的 Tab 3 活窗口：接线走**生产同一个** `host::wire_send_strip` /
/// `host::wire_track_strip`。
fn live_window_with_undo(
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let live = live_window(&view);
    host::wire_send_strip(live.ui(), &port);
    host::wire_track_strip(live.ui(), &port);
    (live, port)
}

/// `track` 的出边（按边身份序 ⇒ `BTreeMap` 迭代序）。
fn outgoing(project: &YebanProjectV1, track: EntityId) -> Vec<&RoutingEdge> {
    project
        .routing_graph
        .edges
        .values()
        .filter(|edge| edge.source_node == track)
        .collect()
}

/// 同一夹具 ＋ 一条 `AuxReturn` 轨（并把它登记进路由图节点）—— 刻意**没有**发送边。
///
/// 与 `tests/send_strip_projection.rs` 的同名夹具逐条同款；本文件要在"零发送"上量
/// `+1` 与防重复，所以不复用 `filled_project`（它已经有一条 Lead → Aux Reverb）。
fn project_with_aux_but_no_send() -> (YebanProjectV1, EntityId) {
    let mut project = demo_project();
    let aux_id = EntityId::new();
    project.tracks.insert(
        aux_id,
        TrackV3 {
            id: aux_id,
            name: "Aux Send Test".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(aux_id);
    project.routing_graph.nodes.sort_unstable();
    (project, aux_id)
}

/// 只剩主总线的合法工程（`validate()` 通过）：④ 的夹具。
fn project_with_master_only() -> YebanProjectV1 {
    let mut project = YebanProjectV1::default();
    let master_id = EntityId::new();
    project.master_bus_track_id = master_id;
    project.tracks.insert(
        master_id,
        TrackV3 {
            id: master_id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![master_id];
    project.validate().expect("只剩主总线的工程必须合法");
    project
}

// ===========================================================================
// 判据 ①：加 MIDI 轨 = Δ1 ＋ 读回（轨数 +1、kind、到 master 的边）
// ===========================================================================
#[test]
fn creating_a_midi_track_commits_once_and_wires_it_to_the_master() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(view.track_create_available, "有主总线 ⇒ 建轨可用");
    assert_eq!(view.track_count_label, "共 7 条轨");
    assert_eq!(view.master_bus_label, "主总线：Master（不可删除）");
    assert_eq!(
        (project.tracks.len(), project.routing_graph.edges.len()),
        (7, 6)
    );
    let master = project.master_bus_track_id;
    let before_ids: BTreeSet<EntityId> = project.tracks.keys().copied().collect();
    let before_nodes = project.routing_graph.nodes.len();

    let (live, port) = live_window_with_undo(&project, "#160 建 MIDI");
    let before = port.display();
    live.ui().invoke_track_create_midi();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    assert_eq!(committed.tracks.len(), 8, "轨数必须 +1");
    assert_eq!(
        committed.routing_graph.edges.len(),
        7,
        "边数必须 +1（新轨到 master）"
    );
    assert_eq!(committed.routing_graph.nodes.len(), before_nodes + 1);
    let new_tracks: Vec<&TrackV3> = committed
        .tracks
        .values()
        .filter(|track| !before_ids.contains(&track.id))
        .collect();
    assert_eq!(new_tracks.len(), 1, "必须恰好一条新轨");
    let track = new_tracks[0];
    assert_eq!(track.kind, TrackKind::Midi);
    assert_eq!(track.name, "MIDI 5", "名字 = 同类现有轨数 + 1");
    assert_eq!(track.id.to_canonical_string().len(), 26);
    assert!(!track.id.is_nil(), "新轨身份不得是 nil");
    assert!(!before_ids.contains(&track.id), "新轨身份必须是新分配的");
    assert!(
        committed.routing_graph.nodes.contains(&track.id),
        "新轨必须同时进 `routing_graph.nodes`"
    );

    let edges = outgoing(&committed, track.id);
    assert_eq!(edges.len(), 1, "新轨必须有**恰好一条**出边");
    assert_eq!(edges[0].destination_node, master);
    assert_eq!(
        edges[0].kind,
        RoutingKind::TrackToBus,
        "MIDI 轨是音轨 ⇒ `TrackToBus`"
    );
    assert_eq!(edges[0].gain_db, None, "新边是单位增益");

    // 引擎能消费：`RoutingPlan` 里这条轨的目的地是 master、线性增益 1.0。
    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&committed, 1).expect("引擎快照");
    let routing = snapshot.routing();
    let route = routing
        .routes(&track.id)
        .iter()
        .find(|route| route.destination() == master)
        .expect("引擎计划里必须有这条轨 → master 的路由");
    assert_eq!(
        route.gain().to_bits(),
        1.0_f32.to_bits(),
        "`None` = 单位增益 ⇒ 线性增益 1.0"
    );

    // 界面读回：轨数读数在同一个活窗口上变。
    assert_eq!(live.ui().get_routing_count_label(), "共 8 条轨");
    let tree = runtime_tree(live.ui());
    assert_eq!(
        tree.find_by_id("track-routing-count")
            .expect("轨数读数必须在树里")
            .label,
        "共 8 条轨"
    );

    report_line(&format!(
        "[#160 ①] 建 MIDI：commit_count {} → {}（Δ1）；tracks 7 → 8；edges 6 → 7；\
         nodes {} → {}；新轨 id={} · kind=Midi · name={:?} · 出边 → master（TrackToBus, gain_db=None）· \
         引擎线性增益={}；轨数读数={:?}",
        before.commit_count,
        after.commit_count,
        before_nodes,
        committed.routing_graph.nodes.len(),
        track.id.to_canonical_string(),
        track.name,
        route.gain(),
        live.ui().get_routing_count_label()
    ));
}

// ===========================================================================
// 判据 ②：加 AuxReturn 轨 = Δ1 ＋ "新建发送"从灰变可用
// ===========================================================================
#[test]
fn adding_an_aux_return_track_commits_once_and_unlocks_the_send_button() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(
        !view.send_create_available,
        "演示工程没有 AuxReturn 轨 ⇒ 按钮必须不可用"
    );
    assert_eq!(view.send_create_reason, SEND_CREATE_NO_AUX_REASON);

    let (live, port) = live_window_with_undo(&project, "#160 建 aux");
    assert!(!live.ui().get_send_create_available(), "点击之前必须是灰的");
    assert_eq!(
        live.ui().get_send_create_reason(),
        SEND_CREATE_NO_AUX_REASON
    );

    let before = port.display();
    live.ui().invoke_track_create_aux();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    let aux = committed
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::AuxReturn)
        .expect("必须有一条 AuxReturn 轨");
    assert_eq!(aux.name, "Aux 1");
    let edges = outgoing(&committed, aux.id);
    assert_eq!(edges.len(), 1, "aux 轨也必须有一条到 master 的出边");
    assert_eq!(
        edges[0].kind,
        RoutingKind::BusToMaster,
        "AuxReturn 是总线 ⇒ `BusToMaster`"
    );
    assert_eq!(edges[0].destination_node, committed.master_bus_track_id);

    // 界面读回：灰 → 可用（同一个活窗口，没有换工程）。
    let unlocked = live.ui().get_send_create_available();
    assert!(unlocked, "有一条 AuxReturn 轨之后必须可用");
    assert_eq!(live.ui().get_send_create_reason(), "");
    assert_eq!(live.ui().get_routing_count_label(), "共 8 条轨");

    // ⭐ 可用不是只画一个亮色：再点一次"新建发送"必须真的 Δ1。
    let before_send = port.display();
    live.ui().invoke_send_create();
    let after_send = port.display();
    assert_eq!(
        after_send.commit_count,
        before_send.commit_count + 1,
        "aux 建好之后 `send-create` 必须真的提交一次"
    );
    let with_send = port.project();
    let source = yeban_app::bridge::send_strip_track(&with_send)
        .expect("展示轨")
        .id;
    let sends = with_send
        .routing_graph
        .edges
        .values()
        .filter(|edge| {
            edge.kind == RoutingKind::SendToAux
                && edge.source_node == source
                && edge.destination_node == aux.id
        })
        .count();
    assert_eq!(sends, 1, "点击之后必须有恰好一条 SendToAux 边");

    report_line(&format!(
        "[#160 ②] 建 AuxReturn：commit_count {} → {}（Δ1）；available false → {}；\
         新轨 kind=AuxReturn · 出边 → master（BusToMaster）；随后 send-create Δ{}（边数 {}）",
        before.commit_count,
        after.commit_count,
        unlocked,
        after_send.commit_count - before_send.commit_count,
        sends
    ));
}

// ===========================================================================
// 判据 ③：删轨 = Δ1 ＋ 读回（轨数 −1 ＋ 边数相应减）
// ===========================================================================
#[test]
fn deleting_the_displayed_track_commits_once_and_drops_its_edges_and_node() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let target = yeban_app::bridge::track_delete_target(&project)
        .expect("filled_project 有展示轨")
        .id;
    assert!(view.track_delete_available);
    assert_eq!(view.track_delete_reason, "");
    // Lead 有两条边：Lead → master（TrackToBus）与 Lead → Aux Reverb（SendToAux）。
    assert_eq!(project.routing_graph.edges.len(), 3);
    assert_eq!(outgoing(&project, target).len(), 2);
    assert_eq!(project.tracks.len(), 4);
    let nodes_before = project.routing_graph.nodes.len();

    let (live, port) = live_window_with_undo(&project, "#160 删轨");
    let before = port.display();
    live.ui().invoke_track_delete();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    assert_eq!(committed.tracks.len(), 3, "轨数必须 −1");
    assert!(
        !committed.tracks.contains_key(&target),
        "被删的轨必须不在 `tracks` 里"
    );
    assert_eq!(
        committed.routing_graph.edges.len(),
        1,
        "引用它的 2 条边必须一起断掉（3 → 1）"
    );
    assert!(
        committed
            .routing_graph
            .edges
            .values()
            .all(|edge| edge.source_node != target && edge.destination_node != target),
        "不得留下任何引用被删轨的边"
    );
    assert_eq!(
        committed.routing_graph.nodes.len(),
        nodes_before - 1,
        "它的路由节点必须一起取走"
    );
    assert!(
        !committed.routing_graph.nodes.contains(&target),
        "不得留下悬空节点"
    );
    assert_eq!(committed.validate(), Ok(()), "删完之后文档必须仍然自洽");
    // 展示轨换成了下一条（Bass）：轨数读数从 4 变 3。
    assert_eq!(live.ui().get_routing_count_label(), "共 3 条轨");
    assert_eq!(
        live.ui().get_device_track_label(),
        ViewState::from_project(&committed)
            .expect("投影")
            .device_track_label
    );

    report_line(&format!(
        "[#160 ③] 删轨：commit_count {} → {}（Δ1）；tracks 4 → 3；edges 3 → {}；\
         nodes {} → {}；被删轨={} · 文档 validate={:?} · 轨数读数={:?}",
        before.commit_count,
        after.commit_count,
        committed.routing_graph.edges.len(),
        nodes_before,
        committed.routing_graph.nodes.len(),
        target.to_canonical_string(),
        committed.validate(),
        live.ui().get_routing_count_label()
    ));
}

// ===========================================================================
// 判据 ④：主总线不可删（禁用 ＋ 逐字原因 ＋ 直接调用也 Δ0）
// ===========================================================================
#[test]
fn the_master_bus_cannot_be_deleted_even_by_calling_the_callback_directly() {
    let project = project_with_master_only();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(
        !view.track_delete_available,
        "只剩主总线 ⇒ 删除按钮必须不可用"
    );
    assert_eq!(view.track_delete_reason, TRACK_DELETE_MASTER_REASON);
    assert_eq!(track_delete_reason(&project), TRACK_DELETE_MASTER_REASON);
    assert_eq!(
        track_delete_ops(&project),
        Err(TRACK_DELETE_MASTER_REASON),
        "纯函数也必须拒绝（⛔ 不只靠界面禁用）"
    );
    // 同一条面板上的其他读数：建轨仍然可用（有主总线），发送不可用（没有展示轨）。
    assert!(view.track_create_available);
    assert_eq!(view.master_bus_label, "主总线：Master（不可删除）");
    assert_eq!(view.send_create_reason, SEND_CREATE_NO_TRACK_REASON);

    let (live, port) = live_window_with_undo(&project, "#160 主总线");
    let tree = runtime_tree(live.ui());
    let reason = tree
        .find_by_id("track-delete-reason")
        .expect("原因必须逐字可见（⛔ 不许静默禁用）");
    assert_eq!(reason.label, TRACK_DELETE_MASTER_REASON);
    assert!(
        tree.contains("track-delete-button"),
        "按钮本体必须还在（只是不可用），而不是从树上消失"
    );
    assert_eq!(
        tree.find_by_id("master-bus-label")
            .expect("主总线说明必须在")
            .label,
        "主总线：Master（不可删除）"
    );

    // 即便有人绕过按钮直接调用回调，宿主也**不**提交。
    let before = port.display();
    live.ui().invoke_track_delete();
    let after = port.display();
    assert_eq!(
        after.commit_count, before.commit_count,
        "主总线：直接调用 `track-delete` 不得提交"
    );
    assert_eq!(after.undoable, before.undoable);
    assert_eq!(port.project().tracks.len(), 1, "工程一位都不能动");

    report_line(&format!(
        "[#160 ④] 主总线：available=false / 原因={:?} / 纯函数={:?} / 按钮在树里={} / \
         直接调用 Δ{} · tracks={}",
        reason.label,
        track_delete_ops(&project),
        tree.contains("track-delete-button"),
        after.commit_count - before.commit_count,
        port.project().tracks.len()
    ));
}

// ===========================================================================
// 判据 ⑤：删发送 = Δ1 ＋ 读回（边数 −1）
// ===========================================================================
#[test]
fn removing_a_send_commits_once_and_drops_the_edge() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert_eq!(view.sends.len(), 1, "filled_project 的展示轨有 1 条发送");
    let edge_id = EntityId::from_str(&view.sends[0].edge_id).expect("边身份必须是合法 ULID");
    let before_edges = project.routing_graph.edges.len();
    assert!(project.routing_graph.edges.contains_key(&edge_id));

    let (live, port) = live_window_with_undo(&project, "#160 删发送");
    let tree = runtime_tree(live.ui());
    assert!(
        tree.contains("send-0-remove-button"),
        "每一条发送必须有删除按钮"
    );
    let before = port.display();
    live.ui()
        .invoke_send_remove(view.sends[0].edge_id.clone().into());
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    assert_eq!(
        committed.routing_graph.edges.len(),
        before_edges - 1,
        "边数必须 −1"
    );
    assert!(
        !committed.routing_graph.edges.contains_key(&edge_id),
        "被删的边必须不在 `edges` 里"
    );
    assert_eq!(
        committed.validate(),
        Ok(()),
        "删完一条边之后文档必须仍然自洽"
    );

    // 界面读回：发送行归零、空态逐字、`send-create` 重新可用（不再重复）。
    assert!(
        live.ui().get_send_edge_ids().iter().next().is_none(),
        "边身份数组必须为空"
    );
    let tree_after = runtime_tree(live.ui());
    assert_eq!(
        tree_after
            .find_by_id("send-empty")
            .expect("空态必须出现")
            .label,
        "无发送"
    );
    assert_eq!(
        tree_after.find_by_id("send-count").expect("条目数").label,
        "0 条发送"
    );
    assert!(
        live.ui().get_send_create_available(),
        "删掉唯一的发送之后，同一条 aux 又可以新建发送"
    );

    // 同一条边再删一次：⛔ 不提交（边已经不存在）。
    let quiet = port.display();
    live.ui()
        .invoke_send_remove(view.sends[0].edge_id.clone().into());
    assert_eq!(
        port.display().commit_count,
        quiet.commit_count,
        "删一条不存在的边不得提交"
    );

    report_line(&format!(
        "[#160 ⑤] 删发送：commit_count {} → {}（Δ1）；edges {} → {}；edge={} 已消失；\
         空态={:?} / {:?}；再次删除 Δ{}",
        before.commit_count,
        after.commit_count,
        before_edges,
        committed.routing_graph.edges.len(),
        edge_id.to_canonical_string(),
        tree_after.find_by_id("send-empty").map(|n| n.label.clone()),
        tree_after.find_by_id("send-count").map(|n| n.label.clone()),
        port.display().commit_count - quiet.commit_count
    ));
}

// ===========================================================================
// 判据 ⑥：防重复发送（同源同目的地再点 ⇒ Δ0 ＋ 逐字原因）
// ===========================================================================
#[test]
fn a_duplicate_send_is_refused_and_costs_zero_commits() {
    let (project, aux_id) = project_with_aux_but_no_send();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(view.send_create_available, "零发送 + 有 aux ⇒ 可用");

    let (live, port) = live_window_with_undo(&project, "#160 重复发送");
    let source = yeban_app::bridge::send_strip_track(&project)
        .expect("展示轨")
        .id;
    assert_eq!(live.ui().get_send_edge_ids().iter().count(), 0);
    let before_first = port.display();
    live.ui().invoke_send_create();
    let after_first = port.display();
    let committed = port.project();
    let matching: Vec<&RoutingEdge> = committed
        .routing_graph
        .edges
        .values()
        .filter(|edge| {
            edge.kind == RoutingKind::SendToAux
                && edge.source_node == source
                && edge.destination_node == aux_id
        })
        .collect();
    assert_eq!(matching.len(), 1, "第一次点击必须建出 1 条发送");

    // 重新投影（= 宿主 `refresh_undo_window` 走的那条路）：可用性必须翻成 false，
    // 原因逐字是"该发送已存在…"。
    let after_view = ViewState::from_project(&committed).expect("投影");
    assert!(
        !after_view.send_create_available,
        "同源同目的地已有一条 SendToAux ⇒ 必须禁用"
    );
    assert_eq!(
        after_view.send_create_reason, SEND_CREATE_DUPLICATE_REASON,
        "原因必须逐字来自投影"
    );
    let (available, reason) = send_create_availability(&committed);
    assert!(!available);
    assert_eq!(reason, SEND_CREATE_DUPLICATE_REASON);
    assert!(!live.ui().get_send_create_available());
    assert_eq!(
        live.ui().get_send_create_reason(),
        SEND_CREATE_DUPLICATE_REASON
    );
    let tree = runtime_tree(live.ui());
    assert_eq!(
        tree.find_by_id("send-create-reason")
            .expect("原因必须逐字可见")
            .label,
        SEND_CREATE_DUPLICATE_REASON
    );
    assert!(tree.contains("send-create-button"), "按钮本体必须还在");

    // 再点一次：宿主也必须拒绝（⛔ 不只靠界面禁用）。
    let before_second = port.display();
    live.ui().invoke_send_create();
    let after_second = port.display();
    assert_eq!(
        after_second.commit_count, before_second.commit_count,
        "重复发送不得提交（前 {}，后 {}）",
        before_second.commit_count, after_second.commit_count
    );
    assert_eq!(after_second.undoable, before_second.undoable);
    assert_eq!(
        port.project().routing_graph.edges.len(),
        committed.routing_graph.edges.len(),
        "边数一位都不能变"
    );
    assert_eq!(
        matching_edges(&port.project(), source, aux_id),
        1,
        "同源同目的地的边仍然只有 1 条"
    );

    report_line(&format!(
        "[#160 ⑥] 防重复：首次 Δ{}；available true → false；原因={:?}；再点 Δ{}；\
         同源同目的地边数={}（第一次 committed={}）",
        after_first.commit_count - before_first.commit_count,
        SEND_CREATE_DUPLICATE_REASON,
        after_second.commit_count - before_second.commit_count,
        matching_edges(&port.project(), source, aux_id),
        matching.len()
    ));
}

/// 同源同目的地的 `SendToAux` 边数（判据 ⑥ 的读数）。
fn matching_edges(project: &YebanProjectV1, source: EntityId, destination: EntityId) -> usize {
    project
        .routing_graph
        .edges
        .values()
        .filter(|edge| {
            edge.kind == RoutingKind::SendToAux
                && edge.source_node == source
                && edge.destination_node == destination
        })
        .count()
}

// ===========================================================================
// 判据 ⑦ 的另一半（源码腿）：四个手势由**投影 / 回调**驱动，⛔ 不是第二份写死列表
// ===========================================================================
#[test]
fn the_track_actions_are_wired_to_the_projection_not_a_fixed_list() {
    // 三个建轨按钮 ＋ 删除按钮：各有稳定语义 ID，且可用性绑在注入的投影字段上。
    for (id, label) in [
        ("track-create-midi-button", "新建 MIDI 轨"),
        ("track-create-audio-button", "新建音频轨"),
        ("track-create-aux-button", "新建辅助返回轨"),
        ("track-delete-button", "删除展示轨"),
    ] {
        assert!(
            SLINT_SOURCE.contains(&format!("accessible-id: \"{id}\"")),
            "`{id}` 必须声明在 `device_rack.slint` 里"
        );
        assert!(
            SLINT_SOURCE.contains(&format!("accessible-label: \"{label}\"")),
            "`{id}` 的无障碍名必须逐字是 `{label}`"
        );
    }
    assert!(
        SLINT_SOURCE.contains("enabled: root.routing-create-available"),
        "三个建轨按钮必须由 `track-create-available` 禁用（⛔ 不是永远可点）"
    );
    assert!(
        SLINT_SOURCE.contains("enabled: root.routing-delete-available"),
        "删除按钮必须由 `track-delete-available` 禁用"
    );
    assert!(
        SLINT_SOURCE.contains("text: root.routing-delete-reason"),
        "不可用时必须逐字显示 `track-delete-reason`"
    );
    assert!(
        SLINT_SOURCE.contains("text: root.routing-master-label"),
        "主总线说明 / 建轨不可用原因必须来自投影"
    );
    // 发送行的删除按钮携带**边身份**，⛔ 不是行下标。
    assert!(
        SLINT_SOURCE.contains("root.send-remove(root.send-edge-ids[send_index])"),
        "`send-remove` 必须携带 `send-edge-ids[send_index]`"
    );
    // 发送循环仍然只有一条（`#159` 的源码腿，本单没有动它）。
    let loops = SLINT_SOURCE
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("for ") && line.contains("in root.send-destination-names"))
        .count();
    assert_eq!(loops, 1, "发送循环必须恰好一条（现在 {loops} 条）");
    // 界面上不得出现硬编码的 ULID 字面量（身份只能来自投影 / 分配器）。
    assert!(
        !SLINT_SOURCE.contains("01J8"),
        "轨 / 路由段里不得出现硬编码的 ULID 字面量"
    );

    // 窗口侧的五个回调必须存在（少一个 ⇒ 按钮点了没有出口）。
    for callback in [
        "send-remove(string)",
        "track-create-midi()",
        "track-create-audio()",
        "track-create-aux()",
        "track-delete()",
    ] {
        assert!(
            APP_SOURCE.contains(&format!("callback {callback};")),
            "`app.slint` 必须声明 `callback {callback};`"
        );
    }

    // 注册表派生：四个按钮 / 原因 / 轨数都在（⛔ 不是 `.slint` 单方面多出来的节点）。
    let registry = ElementRegistry::from_view(&ViewState::from_project(&filled_project()).unwrap());
    for id in [
        "track-routing-panel",
        "track-routing-count",
        "master-bus-label",
        "track-create-midi-button",
        "track-create-audio-button",
        "track-create-aux-button",
        "track-delete-button",
        "track-delete-reason",
        "send-0-remove-button",
    ] {
        assert!(registry.contains(id), "注册表缺少 `{id}`");
    }
    assert_eq!(
        registry
            .get("track-create-midi-button")
            .map(|meta| meta.kind),
        Some(ElementKind::Button)
    );

    // 纯函数：建轨的前置条件 / 拒绝原因也走同一份口径。
    assert!(track_create_available(&filled_project()));
    assert!(!track_create_available(&default_project()));
    assert_eq!(
        track_create_ops(&default_project(), NewTrackKind::Midi),
        Err(yeban_app::bridge::TRACK_CREATE_NO_MASTER_REASON),
        "0 轨空工程必须响亮拒绝（Op 全集写不了 `master_bus_track_id`）"
    );
    assert_eq!(
        master_bus_label(&default_project()),
        yeban_app::bridge::TRACK_CREATE_NO_MASTER_REASON
    );
    assert_eq!(
        yeban_app::bridge::track_delete_availability(&default_project()),
        (false, TRACK_DELETE_NO_TRACK_REASON.to_owned())
    );

    report_line(&format!(
        "[#160 源码腿] device_rack.slint：4 个按钮 / 原因 / 主总线行 / 删除发送都在；\
         发送循环 1 条；app.slint 回调 5 条；源码 {} 字节",
        SLINT_SOURCE.len()
    ));
}
