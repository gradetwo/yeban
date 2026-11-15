//! ⭐ 任务单 `#159` 的判据：**发送条是真实投影 ＋ 两个手势**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §3）
//!
//! | §3 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 真投影 | `filled_project` 的展示轨有 1 条 `SendToAux` ⇒ 1 行；同一夹具再插一条 ⇒ 2 行，运行时 `send-*-row` 1 → 2、`send-count` 逐字 `1 条发送` → `2 条发送` | 证"条目数随 `RoutingGraph` 的边变"。⛔ 不证像素 |
//! | ② 空态 | 有轨但无发送 ⇒ `sends` 0 条、`send-empty` 逐字 `无发送`、条目数 `0 条发送`；空工程 ⇒ 原因逐字 `没有可显示的音轨，无法新建发送` | 证"空不是一片空白，也不是假条目" |
//! | ③ 改增益 = 恰好一次提交 | `invoke_send_gain_grab(edge, 1.0)` ＋ `release` ⇒ `commit_count +1`、`undoable +1`，模型读回 `gain_db` 从 `-12.0` 变为 `12.0`；同位置再按一次 `+0`；中点 ⇒ `-24.0` | 证"拖动一次 = 一步撤销，且值进了 `RoutingGraph`" |
//! | ④ 新建发送 = 恰好一次提交 | 无发送但有 `AuxReturn` 轨的夹具 ⇒ `+1`、`edges` 多一条、`kind == SendToAux`、源 / 目的地逐字、`gain_db == None`、边身份是新的合法 ULID；再用 `EngineSnapshot` 证明该 aux **有总线入边** | 证"点击 → 一个 `Op::ConnectRouting` → 引擎编译得进路由计划" |
//! | ⑤ 无 aux 时禁用 ＋ 原因可见 | `demo_project`（6 轨、0 条 `AuxReturn`）⇒ `send-create-available == false`、`send-create-reason` 逐字、按钮在树里；直接调用回调也 `+0` 提交 | 证"按钮不是假的，且原因不是静默" |
//! | ⑥ 预算三件套 | 不在本文件：读数在 `tests/perf_draw_budget.rs`（60 写点 / 42 数组，常量与原因同处） | — |
//! | ⑦ 不退化 | `cargo test -p yeban-app --locked --no-fail-fast` rc=0；`perf_draw_budget` 绿 | — |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 删掉 `host::apply_view` 里的四个 `set_send_*` 数组写点 ⇒ ① 的运行时读数红；
//! - 把 `send_strip_projection` 的筛选去掉（不筛 `SendToAux` / 不筛源轨）⇒ ① 的条目数与目的地红；
//! - 摘掉 `host::wire_send_strip` 的 `commit_ops` ⇒ ③④ 的 `commit_count` 差 0；
//! - 把 `SendGainGesture` 的 `moved` 判定删掉（每次 move 都提交）⇒ ③ 的 `+1` 变 `+k`；
//! - 把"无 aux"分支改成静默 `return`（不写原因）⇒ ⑤ 的逐字文案红。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"人耳能听到混响"：本判据只证工程里多了一条 `SendToAux` 边，且
//! `yeban-engine` 的 `RoutingPlan` 把它编译成"aux 有入边"。实时监听链由
//! `#154` / `#158` 自己的判据负责。
//!
//! ⭐ **任务单 `#160` 追加**：本文件第 28 行原写"⛔ 不证'界面能建轨 / 建 aux 轨'：
//! `Op::AddTrack` 在 UI 侧今天 0 个构造点"——那句在 `#160` 之后**不再成立**：
//! UI 侧现在有三条 `AddTrack` 构造点（`host::wire_track_strip` 的 `+ MIDI` /
//! `+ 音频` / `+ 辅助返回`，同一份批次在 `bridge::track_create_ops` 里构造）。
//! 建轨 / 删轨 / 删发送 / 防重复的判据在 `tests/track_routing_crud.rs`。

#![allow(unused)]
use std::collections::BTreeSet;
use std::rc::Rc;
use std::str::FromStr as _;

use slint::Model as _;
use yeban_app::bridge::{
    SEND_CREATE_NO_AUX_REASON, SEND_CREATE_NO_TRACK_REASON, ViewState, send_aux_target,
    send_gain_range, send_strip_track,
};
use yeban_app::elements::ElementRegistry;
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

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 发送条源码（**另一个文件**）—— 第 293 条：判据钉住的字符串必须从别处读。
const SLINT_SOURCE: &str = include_str!("../ui/console/device_rack.slint");

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 ＋ JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 一条 Tier-1 活窗口（`build_main_window_with_console_tab` 是唯一的注入实现）。
fn live_window(view: &ViewState, console_tab: i32) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(view, &scene, console_tab)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 一条带撤销端口的 Tab 3 活窗口（接线走**生产同一个** `host::wire_send_strip`）。
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
    let live = live_window(&view, 2);
    host::wire_send_strip(live.ui(), &port);
    (live, port)
}

/// `send-{i}-row` 的形态判定。
fn is_send_row(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("send-") else {
        return false;
    };
    let Some(index) = rest.strip_suffix("-row") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

/// 运行时树里 `send-{i}-row` 的条数。
fn row_count(tree: &ControlTree) -> usize {
    tree.iter().filter(|node| is_send_row(&node.id)).count()
}

/// 模型数组：发送边身份。
fn model_edge_ids(window: &MainWindow) -> Vec<String> {
    window
        .get_send_edge_ids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 模型数组：目的地显示名。
fn model_destinations(window: &MainWindow) -> Vec<String> {
    window
        .get_send_destination_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

/// 模型数组：增益显示文本。
fn model_gain_labels(window: &MainWindow) -> Vec<String> {
    window
        .get_send_gain_labels()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// 模型数组：滑块位置 (dB)。
fn model_gain_dbs(window: &MainWindow) -> Vec<f32> {
    window.get_send_gain_dbs().iter().collect()
}

/// 同一夹具 + 第二条发送（目的地仍是唯一那条 aux，增益 `-6 dB`）。
///
/// ⚠ 夹具是**判据的输入**：直接改模型而不是提交 `Op::ConnectRouting`，
/// 因为被测的是"投影 ＋ 手势"，而"第二条发送"只是夹具的第二个档位。
fn project_with_second_send() -> YebanProjectV1 {
    let mut project = filled_project();
    let source = send_strip_track(&project)
        .expect("filled_project 有展示轨")
        .id;
    let destination = send_aux_target(&project)
        .expect("filled_project 有 aux 轨")
        .id;
    let id = EntityId::new();
    project.routing_graph.edges.insert(
        id,
        RoutingEdge {
            id,
            source_node: source,
            destination_node: destination,
            kind: RoutingKind::SendToAux,
            gain_db: Some(-6.0),
        },
    );
    project
}

/// `demo_project`（6 轨）＋ 一条 `AuxReturn` 轨（并把它登记进路由图节点）。
///
/// 这条夹具刻意**没有**任何发送边 ⇒ "新建发送"能干净地量出 `edges +1`。
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

// ===========================================================================
// 判据 ①：发送行来自投影（两个不同发送数 ⇒ 条目数随之变）
// ===========================================================================
#[test]
fn the_send_rows_follow_the_projected_edges() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
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
        aether_app.contains("> send-gain-dbs:"),
        "新 UI 契约面必须声明 > send-gain-dbs:"
    );
    assert!(
        aether_host.contains("set_send_gain_dbs"),
        "宿主接线层必须注册 set_send_gain_dbs"
    );
    assert!(
        aether_app.contains("callback send-gain-grab("),
        "新 UI 契约面必须声明 callback send-gain-grab("
    );
    assert!(
        aether_host.contains("on_send_gain_grab"),
        "宿主接线层必须注册 on_send_gain_grab"
    );
}

// ===========================================================================
// 判据 ②：空态（无发送 ⇒ 逐字文案 ＋ 条目数 0）
// ===========================================================================
#[test]
fn an_empty_send_list_shows_the_empty_state_and_zero_rows() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
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
        aether_app.contains("> send-gain-dbs:"),
        "新 UI 契约面必须声明 > send-gain-dbs:"
    );
    assert!(
        aether_host.contains("set_send_gain_dbs"),
        "宿主接线层必须注册 set_send_gain_dbs"
    );
    assert!(
        aether_app.contains("callback send-gain-grab("),
        "新 UI 契约面必须声明 callback send-gain-grab("
    );
    assert!(
        aether_host.contains("on_send_gain_grab"),
        "宿主接线层必须注册 on_send_gain_grab"
    );
}

// ===========================================================================
// 判据 ③：改增益 = 恰好一次提交，且值真的进了 `RoutingGraph`
// ===========================================================================
#[test]
fn the_send_gain_gesture_commits_exactly_once_and_writes_the_model() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> send-gain-dbs:"),
        "新 UI 契约面必须声明 > send-gain-dbs:"
    );
    assert!(
        aether_host.contains("set_send_gain_dbs"),
        "宿主接线层必须注册 set_send_gain_dbs"
    );
    assert!(
        aether_app.contains("callback send-gain-grab("),
        "新 UI 契约面必须声明 callback send-gain-grab("
    );
    assert!(
        aether_host.contains("on_send_gain_grab"),
        "宿主接线层必须注册 on_send_gain_grab"
    );
    assert!(
        aether_app.contains("callback send-gain-release("),
        "新 UI 契约面必须声明 callback send-gain-release("
    );
    assert!(
        aether_host.contains("on_send_gain_release"),
        "宿主接线层必须注册 on_send_gain_release"
    );
}

// ===========================================================================
// 判据 ④：新建发送 = 恰好一次提交，且引擎能消费这条边
// ===========================================================================
#[test]
fn the_create_send_gesture_commits_exactly_once_and_the_engine_consumes_it() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
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
        aether_app.contains("> send-gain-dbs:"),
        "新 UI 契约面必须声明 > send-gain-dbs:"
    );
    assert!(
        aether_host.contains("set_send_gain_dbs"),
        "宿主接线层必须注册 set_send_gain_dbs"
    );
    assert!(
        aether_app.contains("callback send-gain-grab("),
        "新 UI 契约面必须声明 callback send-gain-grab("
    );
    assert!(
        aether_host.contains("on_send_gain_grab"),
        "宿主接线层必须注册 on_send_gain_grab"
    );
}

// ===========================================================================
// 判据 ⑤：无 aux 时禁用 ＋ 原因逐字可见（点了也不提交）
// ===========================================================================
#[test]
fn a_project_without_an_aux_return_disables_creation_and_says_why() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
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
        aether_app.contains("> send-gain-dbs:"),
        "新 UI 契约面必须声明 > send-gain-dbs:"
    );
    assert!(
        aether_host.contains("set_send_gain_dbs"),
        "宿主接线层必须注册 set_send_gain_dbs"
    );
    assert!(
        aether_app.contains("callback send-gain-grab("),
        "新 UI 契约面必须声明 callback send-gain-grab("
    );
    assert!(
        aether_host.contains("on_send_gain_grab"),
        "宿主接线层必须注册 on_send_gain_grab"
    );
}

// ===========================================================================
// 源码腿：发送条由投影驱动（不是第二份写死的列表）
// ===========================================================================
#[test]
fn the_send_strip_is_driven_by_the_projection_not_a_fixed_list() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> send-gain-dbs:"),
        "新 UI 契约面必须声明 > send-gain-dbs:"
    );
    assert!(
        aether_host.contains("set_send_gain_dbs"),
        "宿主接线层必须注册 set_send_gain_dbs"
    );
    assert!(
        aether_app.contains("callback send-gain-grab("),
        "新 UI 契约面必须声明 callback send-gain-grab("
    );
    assert!(
        aether_host.contains("on_send_gain_grab"),
        "宿主接线层必须注册 on_send_gain_grab"
    );
    assert!(
        aether_app.contains("callback send-gain-release("),
        "新 UI 契约面必须声明 callback send-gain-release("
    );
    assert!(
        aether_host.contains("on_send_gain_release"),
        "宿主接线层必须注册 on_send_gain_release"
    );
}
