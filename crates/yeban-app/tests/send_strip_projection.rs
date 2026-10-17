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
//! `#154` / `#158` 自己的判据负责。⛔ 不证"界面能建轨 / 建 aux 轨"：
//! `Op::AddTrack` 在 UI 侧今天 0 个构造点。

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
    // 夹具 A：`filled_project` 的展示轨（Lead）有 1 条 → Aux Reverb、-12 dB。
    let view_a = ViewState::from_project(&filled_project()).expect("投影 A");
    assert_eq!(view_a.sends.len(), 1, "夹具 A 必须只有 1 条发送");
    assert_eq!(view_a.sends[0].destination_name, "Aux Reverb");
    assert_eq!(view_a.sends[0].gain_display, "-12 dB");
    assert_eq!(view_a.sends[0].gain_db, Some(-12.0));

    // 夹具 B：同一夹具再插一条发送（-6 dB）⇒ 2 行。
    let project_b = project_with_second_send();
    let view_b = ViewState::from_project(&project_b).expect("投影 B");
    assert_eq!(view_b.sends.len(), 2, "夹具 B 必须有 2 条发送");
    assert_eq!(
        view_b.send_destination_names(),
        vec!["Aux Reverb".to_owned(), "Aux Reverb".to_owned()]
    );
    assert_eq!(
        view_b.send_gain_labels(),
        vec!["-12 dB".to_owned(), "-6 dB".to_owned()]
    );

    // 注册表同样派生：行数 = 投影发送数。
    let registry = ElementRegistry::from_view(&view_b);
    assert_eq!(
        registry.iter().filter(|meta| is_send_row(&meta.id)).count(),
        2,
        "注册表的发送行数必须等于投影发送数"
    );

    // 运行时：同一个活窗口先画夹具 A，再注入夹具 B。
    let port = live_window(&view_a, 2);
    let tree_a = runtime_tree(port.ui());
    let rows_a = row_count(&tree_a);
    let count_a = tree_a
        .find_by_id("send-count")
        .expect("条目数节点必须在 Tab 3 里")
        .label
        .clone();
    assert_eq!(rows_a, 1, "夹具 A 必须画出 1 行发送");
    assert_eq!(count_a, "1 条发送");
    assert_eq!(model_edge_ids(port.ui()), view_a.send_edge_ids());
    assert_eq!(model_destinations(port.ui()), vec!["Aux Reverb"]);
    assert_eq!(model_gain_labels(port.ui()), vec!["-12 dB"]);
    assert!(!tree_a.contains("send-empty"), "有发送时不得显示空态");

    host::apply_view(port.ui(), &view_b, WINDOW.width as f32, 0.0);
    let tree_b = runtime_tree(port.ui());
    let rows_b = row_count(&tree_b);
    let count_b = tree_b
        .find_by_id("send-count")
        .expect("条目数节点必须在")
        .label
        .clone();
    assert_eq!(rows_b, 2, "夹具 B 必须画出 2 行发送");
    assert_eq!(count_b, "2 条发送");
    assert_eq!(model_edge_ids(port.ui()), view_b.send_edge_ids());
    assert_eq!(model_gain_labels(port.ui()), vec!["-12 dB", "-6 dB"]);
    let row_one = tree_b.find_by_id("send-1-row").expect("第二行必须在树里");
    assert_eq!(row_one.label, "发送 → Aux Reverb · -6 dB");
    assert_eq!(
        tree_b
            .find_by_id("send-0-gain-value")
            .expect("第一行的增益文本")
            .label,
        "-12 dB"
    );

    report_line(&format!(
        "[#159 ①] 夹具 A: 行 {rows_a} / {count_a:?}；夹具 B: 行 {rows_b} / {count_b:?}；\
         目的地 = {:?}；增益 = {:?}；第二行 = {:?}",
        model_destinations(port.ui()),
        model_gain_labels(port.ui()),
        row_one.label
    ));
}

// ===========================================================================
// 判据 ②：空态（无发送 ⇒ 逐字文案 ＋ 条目数 0）
// ===========================================================================
#[test]
fn an_empty_send_list_shows_the_empty_state_and_zero_rows() {
    // 有轨但没有发送（`demo_project` 的 6 条轨都不发往 aux）。
    let view = ViewState::from_project(&demo_project()).expect("演示工程投影");
    assert!(
        view.sends.is_empty(),
        "演示工程必须投影成空发送列表（⛔ 不是编造的发送）"
    );
    assert_eq!(view.send_destination_names(), Vec::<String>::new());
    assert!(view.sends.is_empty());

    let port = live_window(&view, 2);
    assert!(
        model_edge_ids(port.ui()).is_empty(),
        "`send-edge-ids` 必须为空"
    );
    assert!(model_destinations(port.ui()).is_empty());
    assert!(model_gain_dbs(port.ui()).is_empty());

    let tree = runtime_tree(port.ui());
    assert!(tree.contains("send-strip"), "空态时面板本体必须在");
    let empty = tree
        .find_by_id("send-empty")
        .expect("无发送必须显示 `send-empty`（⛔ 不许一片空白）");
    assert_eq!(empty.label, "无发送", "空态文案必须逐字一致");
    let count = tree.find_by_id("send-count").expect("条目数节点必须在");
    assert_eq!(count.label, "0 条发送");
    assert_eq!(row_count(&tree), 0, "空态不得有发送行节点");

    // 空工程（无轨）：不可新建发送的**另一档**原因。
    let empty_project = ViewState::from_project(&default_project()).expect("空工程投影");
    assert!(empty_project.sends.is_empty());
    assert!(!empty_project.send_create_available);
    assert_eq!(
        empty_project.send_create_reason,
        SEND_CREATE_NO_TRACK_REASON
    );

    report_line(&format!(
        "[#159 ②] 空发送：sends=0 / 文案={:?} / 条目数={:?} / 行=0；空工程原因={:?}",
        empty.label, count.label, empty_project.send_create_reason
    ));
}

// ===========================================================================
// 判据 ③：改增益 = 恰好一次提交，且值真的进了 `RoutingGraph`
// ===========================================================================
#[test]
fn the_send_gain_gesture_commits_exactly_once_and_writes_the_model() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let edge_id = view.sends[0].edge_id.clone();
    let edge = EntityId::from_str(&edge_id).expect("边身份必须是合法 ULID");
    let (min_db, max_db) = send_gain_range();
    assert_eq!(
        (min_db, max_db),
        (-60.0, 12.0),
        "发送增益值域必须来自模型（NOMINAL_GAIN_MIN_DB/MAX_DB）"
    );
    assert_eq!(
        (view.send_gain_min_db(), view.send_gain_max_db()),
        (min_db, max_db)
    );
    assert_eq!(view.send_gain_dbs(), vec![-12.0]);

    let (live, port) = live_window_with_undo(&project, "#159 增益");
    let before = port.display();

    // ---- 一次拖动（按在最右端 ⇒ 值域上界 12 dB，松手提交一次）----
    live.ui().invoke_send_gain_grab(edge_id.clone().into(), 1.0);
    live.ui().invoke_send_gain_release(edge_id.clone().into());
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次拖动必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);
    let committed = port.project();
    let committed_edge = committed
        .routing_graph
        .edges
        .get(&edge)
        .expect("提交之后那条边仍在");
    assert_eq!(
        committed_edge.gain_db.map(f32::to_bits),
        Some(max_db.to_bits()),
        "拖到最右端必须写进值域上界（读回 {:?}）",
        committed_edge.gain_db
    );
    assert_eq!(model_gain_labels(live.ui()), vec!["12 dB"]);
    assert_eq!(model_gain_dbs(live.ui()), vec![max_db]);

    // ---- 同一个位置再按一次 ⇒ **不提交**（⛔ 不留一步什么都没改的撤销）----
    let quiet = port.display();
    live.ui().invoke_send_gain_grab(edge_id.clone().into(), 1.0);
    live.ui().invoke_send_gain_release(edge_id.clone().into());
    let still = port.display();
    assert_eq!(
        still.commit_count, quiet.commit_count,
        "同一个位置按下再松手不得产生提交"
    );
    assert_eq!(still.undoable, quiet.undoable);

    // ---- 单击中点 ⇒ 值域中点 -24 dB，仍是一次提交 ----
    live.ui().invoke_send_gain_grab(edge_id.clone().into(), 0.5);
    live.ui().invoke_send_gain_release(edge_id.clone().into());
    let clicked = port.display();
    assert_eq!(clicked.commit_count, still.commit_count + 1);
    let midpoint = port
        .project()
        .routing_graph
        .edges
        .get(&edge)
        .and_then(|edge| edge.gain_db)
        .expect("中点提交之后必须有一个增益");
    assert_eq!(
        midpoint.to_bits(),
        (-24.0_f32).to_bits(),
        "中点必须写 -24 dB（读回 {midpoint}）"
    );

    report_line(&format!(
        "[#159 ③] 发送增益：commit_count {} → {}（Δ1），读回 gain_db 前 {:?} → 后 {:?}；\
         同位置再按一次 Δ0（{}）；单击中点读回 {midpoint} dB（Δ1）",
        before.commit_count,
        after.commit_count,
        view.sends[0].gain_db,
        committed_edge.gain_db,
        still.commit_count
    ));
}

// ===========================================================================
// 判据 ④：新建发送 = 恰好一次提交，且引擎能消费这条边
// ===========================================================================
#[test]
fn the_create_send_gesture_commits_exactly_once_and_the_engine_consumes_it() {
    let (project, aux_id) = project_with_aux_but_no_send();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(view.sends.is_empty(), "夹具必须从 0 条发送开始");
    assert!(view.send_create_available, "有一条 AuxReturn 轨 ⇒ 可新建");
    assert_eq!(view.send_create_reason, "");
    let source_id = send_strip_track(&project).expect("展示轨").id;
    let before_ids: BTreeSet<EntityId> = project.routing_graph.edges.keys().copied().collect();
    let before_edges = project.routing_graph.edges.len();

    let (live, port) = live_window_with_undo(&project, "#159 新建");
    let before = port.display();
    live.ui().invoke_send_create();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    // 读回权威工程：边数 +1、类型 / 两端 / 增益逐字。
    let committed = port.project();
    assert_eq!(
        committed.routing_graph.edges.len(),
        before_edges + 1,
        "`edges` 必须多一条"
    );
    let new_edges: Vec<&RoutingEdge> = committed
        .routing_graph
        .edges
        .values()
        .filter(|edge| !before_ids.contains(&edge.id))
        .collect();
    assert_eq!(new_edges.len(), 1, "必须恰好一条新边");
    let edge = new_edges[0];
    assert_eq!(edge.kind, RoutingKind::SendToAux);
    assert_eq!(edge.source_node, source_id, "源必须是展示轨");
    assert_eq!(
        edge.destination_node, aux_id,
        "目的地必须是那条 AuxReturn 轨"
    );
    assert_eq!(edge.gain_db, None, "新发送必须是单位增益（`None`）");
    assert!(!before_ids.contains(&edge.id), "边身份必须是新分配的");
    assert!(!edge.id.is_nil(), "边身份不得是 nil");
    assert_eq!(
        edge.id.to_canonical_string().len(),
        26,
        "边身份必须是 26 字符规范 ULID 文本"
    );

    // 界面读回（`refresh_undo_window` 的唯一出口）。
    assert_eq!(model_destinations(live.ui()), vec!["Aux Send Test"]);
    assert_eq!(model_gain_labels(live.ui()), vec!["单位增益"]);
    let tree = runtime_tree(live.ui());
    assert_eq!(row_count(&tree), 1, "新建之后必须画出 1 行");
    assert_eq!(
        tree.find_by_id("send-count").expect("条目数").label,
        "1 条发送"
    );
    assert_eq!(
        tree.find_by_id("send-0-row").expect("新行").label,
        "发送 → Aux Send Test · 单位增益"
    );

    // ⭐ 引擎能消费：`#154` 的 `RoutingPlan` 里这条 aux **有总线入边**
    // （`#158` 的处理型总线判定用的就是这同一个谓词；⛔ 本判据不必听声）。
    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&committed, 1).expect("引擎快照");
    let routing = snapshot.routing();
    assert!(
        routing.has_incoming_audio(&aux_id),
        "新发送的目的地必须真的收到音频（RoutingPlan::has_incoming_audio）"
    );
    let route = routing
        .routes(&source_id)
        .iter()
        .find(|route| route.destination() == aux_id)
        .expect("源的出边里必须有这条发送");
    assert_eq!(
        route.gain().to_bits(),
        1.0_f32.to_bits(),
        "`None` = 单位增益 ⇒ 线性增益必须是 1.0"
    );

    report_line(&format!(
        "[#159 ④] 新建发送：commit_count {} → {}（Δ1）；edges {} → {}；\
         新边 id={} · kind=SendToAux · gain_db=None；引擎 has_incoming_audio(aux)={} · 线性增益={}",
        before.commit_count,
        after.commit_count,
        before_edges,
        committed.routing_graph.edges.len(),
        edge.id.to_canonical_string(),
        routing.has_incoming_audio(&aux_id),
        route.gain()
    ));
}

// ===========================================================================
// 判据 ⑤：无 aux 时禁用 ＋ 原因逐字可见（点了也不提交）
// ===========================================================================
#[test]
fn a_project_without_an_aux_return_disables_creation_and_says_why() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程投影");
    assert!(
        send_aux_target(&project).is_none(),
        "夹具必须**没有** AuxReturn 轨（演示工程 6 条轨都不是 aux）"
    );
    assert!(!view.send_create_available, "无 aux ⇒ 按钮必须不可用");
    assert_eq!(
        view.send_create_reason, SEND_CREATE_NO_AUX_REASON,
        "不可用的原因必须逐字来自投影"
    );

    let before_edges = project.routing_graph.edges.len();
    let (live, port) = live_window_with_undo(&project, "#159 无 aux");
    let tree = runtime_tree(live.ui());
    let reason = tree
        .find_by_id("send-create-reason")
        .expect("原因必须逐字可见（⛔ 不许静默禁用）");
    assert_eq!(reason.label, SEND_CREATE_NO_AUX_REASON);
    assert!(
        tree.contains("send-create-button"),
        "按钮本体必须还在（只是不可用），而不是从树上消失"
    );
    assert_eq!(row_count(&tree), 0);
    assert_eq!(tree.find_by_id("send-empty").expect("空态").label, "无发送");

    // 即便有人绕过按钮直接调用回调，宿主也**不**提交（禁用不是只画个灰）。
    let before = port.display();
    live.ui().invoke_send_create();
    let after = port.display();
    assert_eq!(
        after.commit_count, before.commit_count,
        "无 aux 时调用 `send-create` 不得提交"
    );
    assert_eq!(after.undoable, before.undoable);
    assert_eq!(
        port.project().routing_graph.edges.len(),
        before_edges,
        "无 aux 时 `edges` 一位都不能变"
    );

    report_line(&format!(
        "[#159 ⑤] 无 aux：available=false / 原因={:?} / 按钮在树里={} / \
         直接调用后 commit Δ{} · edges {} → {}",
        reason.label,
        tree.contains("send-create-button"),
        after.commit_count - before.commit_count,
        before_edges,
        port.project().routing_graph.edges.len()
    ));
}

// ===========================================================================
// 源码腿：发送条由投影驱动（不是第二份写死的列表）
// ===========================================================================
#[test]
fn the_send_strip_is_driven_by_the_projection_not_a_fixed_list() {
    // 条目数必须由投影决定：恰好一条 `for … in root.send-destination-names`。
    let loops = SLINT_SOURCE
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("for ") && line.contains("in root.send-destination-names"))
        .count();
    assert_eq!(
        loops, 1,
        "发送条必须**恰好**一条 `for … in root.send-destination-names` 循环（现在 {loops} 条）"
    );
    // 空态文案逐字固定（判据 ② 在运行时再断一次）。
    assert!(SLINT_SOURCE.contains("无发送"), "空态文案缺失");
    // 增益值域必须来自注入（⛔ 界面上没有第二份范围表）。
    assert!(
        SLINT_SOURCE.contains("accessible-value-minimum: root.send-gain-min-db")
            && SLINT_SOURCE.contains("accessible-value-maximum: root.send-gain-max-db"),
        "滑块值域必须绑定注入的 `send-gain-min-db` / `send-gain-max-db`，⛔ 不许写字面量"
    );
    // 新建按钮的禁用必须绑在投影的可用性上。
    assert!(
        SLINT_SOURCE.contains("enabled: root.send-create-available"),
        "新建按钮必须由 `send-create-available` 禁用"
    );
    // 三个滑块手势必须携带**边身份**，⛔ 不是行下标。
    for callback in ["send-gain-grab", "send-gain-drag", "send-gain-release"] {
        assert!(
            SLINT_SOURCE.contains(&format!("root.{callback}(root.send-edge-ids[send_index]")),
            "`{callback}` 必须携带 `root.send-edge-ids[send_index]`（边身份），⛔ 不是行下标"
        );
    }
    // 界面上不得出现硬编码的 ULID 字面量（身份只能来自投影）。
    assert!(
        !SLINT_SOURCE.contains("01J8"),
        "发送条里不得出现硬编码的 ULID 字面量"
    );

    report_line(&format!(
        "[#159 源码腿] device_rack.slint：发送循环 1 条 / 空态文案在 / 值域绑定注入 = true / \
         按钮绑定可用性 = true / 三个手势带边身份 = true / 源码 {} 字节",
        SLINT_SOURCE.len()
    ));
}
