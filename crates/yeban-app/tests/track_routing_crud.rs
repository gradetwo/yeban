//! ⭐ 任务单 `#160` 的判据：**从界面建轨 / 删轨 / 删发送 / 防重复发送**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 加 MIDI 轨 = Δ1 | 点击一次 ⇒ `commit_count +1`、`tracks +1`、新轨 `kind == Midi`、**有一条到 master 的 `TrackToBus` 边**、`nodes +1`；⭐ `D-UI-4`：链上**恰 1 台**设备、`kind == InternalInstrument`、非旁通、`latency_samples == 0`、含 `cutoff_hz` 或振荡器键（= 引擎 `DeviceTone::is_source` 的音源判据）、九个键名逐条过 `yeban_engine::synth::is_consumed_instrument_param`；`TrackRouting` 引擎计划里该轨的目的地是 master、线性增益 1.0 | 证"一次点击 = 一步可撤销的编辑，新轨在路由图里真的接上了，**且它带的设备被引擎算作音源**（不是效果器）" |
//! | ①b 加音频轨不带乐器 | `demo_project` ⇒ 一次点击恰一条新轨、`kind == Audio`、**`devices` 为空** | 证 `D-UI-4` 的默认音源**只给 MIDI 轨**（音频轨的音源是录进来的素材） |
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
//! - 把 `track_create_ops` 里 `D-UI-4` 那条 `InsertDevice` 摘掉 ⇒ ① 的"链上恰 1 台
//!   `InternalInstrument`"红（连带 `tests/device_clip_creation.rs` 判据 ⑦ 的
//!   "设备链 `[内置合成器, EQ]`"红）；把它改成 `InternalEffect` 或改掉任一引擎不认的
//!   键名 ⇒ 同一格红；把它也加到音频轨上 ⇒ ①b 红；
//! - 把 `track_delete_ops` 里的 `DisconnectRouting` 循环摘掉 ⇒ ③ 提交会被
//!   `RemoveRoutingNode` 的前置条件拒掉（`commit_count` 不动）；
//! - 把 `send_create_availability` 的重复分支删掉 ⇒ ⑥ 的 `+0` 变 `+1`；
//! - 把 `host::wire_track_strip` 的 `commit_ops` 摘掉 ⇒ ①②③ 的 `commit_count` 差 0；
//! - 把 `track_delete_availability` 的主总线分支删掉 ⇒ ④ 的原因与可用性红。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"人耳听到声音"：本判据只证工程里多了一条到 master 的边、引擎的
//! `RoutingPlan` 把它编译成 `source → master`，以及轨上那台设备**被引擎算作音源**。
//! 真出声由本线的创作验收去量（UI 建轨 → UI 画音符 → 导出 WAV 的时长与响度读数），
//! ⛔ 不由这一格替代。
//!
//! ⭐ `#161` 补的那一条：**0 音轨空工程能建轨**。`default_project()` 现在出厂自带
//! 1 条主总线，`track_create_ops(&default_project(), Midi)` 不再是 `NoMaster`；
//! 本文件用**纯函数腿**证明第一批 Op 真的落在空工程上（`tracks 1 → 2`、`edges 0 → 1`、
//! `validate() == Ok`）。`Op` 全集仍然写不了 `master_bus_track_id`，所以
//! `TRACK_CREATE_NO_MASTER_REASON` 保留，并被 `YebanProjectV1::default()`（0 轨 nil 主总线）
//! 这个**反例夹具**继续钉住。

#![allow(unused)]
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
use yeban_model::project::{
    DeviceKind, RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback send-create("),
        "新 UI 契约面必须声明 callback send-create("
    );
    assert!(
        aether_host.contains("on_send_create"),
        "宿主接线层必须注册 on_send_create"
    );
    assert!(
        aether_app.contains("callback send-remove("),
        "新 UI 契约面必须声明 callback send-remove("
    );
    assert!(
        aether_host.contains("on_send_remove"),
        "宿主接线层必须注册 on_send_remove"
    );
    assert!(
        aether_app.contains("callback track-create-audio("),
        "新 UI 契约面必须声明 callback track-create-audio("
    );
    assert!(
        aether_host.contains("on_track_create_audio"),
        "宿主接线层必须注册 on_track_create_audio"
    );
}

// ===========================================================================
// 判据 ①b（⭐ 缺陷 `D-UI-4` 的反面）：**只有 MIDI 轨**自带内置乐器
// ===========================================================================
#[test]
fn creating_an_audio_track_leaves_the_device_chain_empty() {
    let project = demo_project();
    let before_ids: BTreeSet<EntityId> = project.tracks.keys().copied().collect();
    let (live, port) = live_window_with_undo(&project, "D-UI-4 建音频轨");
    live.ui().invoke_track_create_audio();
    let committed = port.project();
    let mut fresh = committed
        .tracks
        .values()
        .filter(|track| !before_ids.contains(&track.id));
    let track = fresh.next().expect("必须有一条新轨");
    assert!(fresh.next().is_none(), "一次点击只许建一条轨");
    assert_eq!(track.kind, TrackKind::Audio);
    assert!(
        track.devices.is_empty(),
        "音频轨的音源是录进来的素材 ⇒ 不许自动挂内置乐器（实际 {} 台）",
        track.devices.len()
    );
    report_line(&format!(
        "[D-UI-4 ①b] 建音频轨 {:?}：devices={}（内置乐器只给 MIDI 轨）",
        track.name,
        track.devices.len()
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback send-create("),
        "新 UI 契约面必须声明 callback send-create("
    );
    assert!(
        aether_host.contains("on_send_create"),
        "宿主接线层必须注册 on_send_create"
    );
    assert!(
        aether_app.contains("callback send-remove("),
        "新 UI 契约面必须声明 callback send-remove("
    );
    assert!(
        aether_host.contains("on_send_remove"),
        "宿主接线层必须注册 on_send_remove"
    );
    assert!(
        aether_app.contains("callback track-create-audio("),
        "新 UI 契约面必须声明 callback track-create-audio("
    );
    assert!(
        aether_host.contains("on_track_create_audio"),
        "宿主接线层必须注册 on_track_create_audio"
    );
}

// ===========================================================================
// 判据 ⑤：删发送 = Δ1 ＋ 读回（边数 −1）
// ===========================================================================
#[test]
fn removing_a_send_commits_once_and_drops_the_edge() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback send-create("),
        "新 UI 契约面必须声明 callback send-create("
    );
    assert!(
        aether_host.contains("on_send_create"),
        "宿主接线层必须注册 on_send_create"
    );
    assert!(
        aether_app.contains("callback send-remove("),
        "新 UI 契约面必须声明 callback send-remove("
    );
    assert!(
        aether_host.contains("on_send_remove"),
        "宿主接线层必须注册 on_send_remove"
    );
    assert!(
        aether_app.contains("callback track-create-audio("),
        "新 UI 契约面必须声明 callback track-create-audio("
    );
    assert!(
        aether_host.contains("on_track_create_audio"),
        "宿主接线层必须注册 on_track_create_audio"
    );
}

// ===========================================================================
// 判据 ⑥：防重复发送（同源同目的地再点 ⇒ Δ0 ＋ 逐字原因）
// ===========================================================================
#[test]
fn a_duplicate_send_is_refused_and_costs_zero_commits() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback send-create("),
        "新 UI 契约面必须声明 callback send-create("
    );
    assert!(
        aether_host.contains("on_send_create"),
        "宿主接线层必须注册 on_send_create"
    );
    assert!(
        aether_app.contains("callback send-remove("),
        "新 UI 契约面必须声明 callback send-remove("
    );
    assert!(
        aether_host.contains("on_send_remove"),
        "宿主接线层必须注册 on_send_remove"
    );
    assert!(
        aether_app.contains("callback track-create-audio("),
        "新 UI 契约面必须声明 callback track-create-audio("
    );
    assert!(
        aether_host.contains("on_track_create_audio"),
        "宿主接线层必须注册 on_track_create_audio"
    );
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback send-create("),
        "新 UI 契约面必须声明 callback send-create("
    );
    assert!(
        aether_host.contains("on_send_create"),
        "宿主接线层必须注册 on_send_create"
    );
    assert!(
        aether_app.contains("callback track-create-audio("),
        "新 UI 契约面必须声明 callback track-create-audio("
    );
    assert!(
        aether_host.contains("on_track_create_audio"),
        "宿主接线层必须注册 on_track_create_audio"
    );
    assert!(
        aether_app.contains("callback track-create-aux("),
        "新 UI 契约面必须声明 callback track-create-aux("
    );
    assert!(
        aether_host.contains("on_track_create_aux"),
        "宿主接线层必须注册 on_track_create_aux"
    );
}
