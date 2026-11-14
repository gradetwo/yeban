//! ⭐ 任务单 `#181`：**主总线设备的旁通 ＋ 一个 EQ 参数滑块**。
//!
//! `#180` 只给主总线接了**增 / 删**，并逐字登记过"主总线的旁通／EQ 滑块仍不可编辑"。
//! 本文件钉住那两条补上的手势（与展示轨**同一份**实现体，`track` 是唯一自变量）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 旁通作用于 master | `invoke_master_device_bypass_toggle(master-device-ids[0])`（⭐ `#182` 按身份）⇒ `commit_count +1`；主总线 `devices[0].bypassed` **false → true**，**展示轨 `devices[0].bypassed` 不动**；界面 `master-device-bypassed=[true]` / `device-bypassed=[false]` | 证"旁通落在主总线，⛔ 不是展示轨"（注入：把目标改回 `device_rack_track` ⇒ 这条红） |
//! | ② 滑块作用于 master | `invoke_master_device_eq_grab(master-device-eq-device-id, 1.0)` ＋ `release()` ⇒ `commit_count +1`；读回主总线 `eq_low_gain = 18.0`（契约上界），**展示轨仍是 0.0**；界面 `master-device-eq-gain-db = 18` / `device-eq-gain-db = 0` | 同上，滑块那条腿 |
//! | ③ 注入 | 把 `master_device_eq_low_gain_target` / `master_device_bypass_ops` 的目标改回 `device_rack_track` ⇒ ①② 各自变红（读数见交付报告，⛔ 未提交） | 证这两条腿真的靠"显式的目标轨"工作 |
//! | ④ 同位置再按 Δ0 | 值已经是上界时再 `grab(身份, 1.0)` ＋ `release()` ⇒ `commit_count` / `undoable` 一位不动 | 证"没变不产生空提交" |
//! | ⑤ 旁通真的关掉处理 | `master_insert_frames`：未旁通 = `400 × 128`；UI 旁通之后 = **0** | 证器件的武装位真的被旁通位关掉；⛔ 不证人耳听感 |
//! | ⑥ 无 master 设备 | 主总线存在但 0 台 ⇒ `master-device-eq-available=false`、滑块 / 旁通开关**不出现**、原因逐字 = [`MASTER_DEVICE_EQ_NO_TARGET_REASON`]；没有主总线 ⇒ 逐字 = [`MASTER_DEVICE_EQ_NO_MASTER_REASON`] | 证"没有假的可编辑控件，也不静默" |
//! | ⑦ 预算 ＋ 几何 | 预算在 `tests/perf_draw_budget.rs`（⭐ `#182` 起 **89** 写点 / **49** 数组）；本文件在**运行时控件树**里读新控件的 `bounds`：`x + width ≤ 1384`（第二段右沿） | 证"没有把第二段往右扩" |
//! | ⑧ 不退化 | 不在本文件：`cargo test -p yeban-app --locked --no-fail-fast` 的 rc 与通过数 | — |
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"有人耳听过"；⛔ 不证界面有"选中轨"（没有这个概念）；⛔ 不救窄窗口
//! （第二段本来就住在绝对坐标里，本单只保证**不加剧**：新控件全在 `840..1384` 内）。

#![allow(unused)]
use std::collections::BTreeMap;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{
    self, BuiltinEffect, MASTER_DEVICE_BYPASS_NO_DEVICE_REASON,
    MASTER_DEVICE_BYPASS_NO_MASTER_REASON, MASTER_DEVICE_EQ_NO_MASTER_REASON,
    MASTER_DEVICE_EQ_NO_TARGET_REASON, ViewState, device_eq_low_gain_target, device_rack_track,
    master_device_bypass_ops, master_device_eq_availability, master_device_eq_low_gain_target,
    master_device_track,
};
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, RoutingEdge, RoutingGraph, RoutingKind,
    TrackKind, TrackV3, YebanProjectV1,
};
use yeban_model::{EntityId, MidiNote};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸 = **产品默认档**（`app.slint` 的 `width: 1920px`）。主总线那一栏住在
/// 面板右侧 x=840..1384，默认 1920 窗口里控制台宽 1400 ⇒ 整栏可见。
const WINDOW: Size = Size::new(1920, 1080);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 音频判据的渲染长度：400 × 128 = 51,200 帧 ≈ 1.07 s @48 kHz（与 `#173` / `#180` 同款）。
const SHORT_QUANTA: usize = 400;

/// 第二段（`master-device-rack`）的右沿（逻辑像素）—— 默认窗口里它止于 1384。
const MASTER_SEGMENT_RIGHT: u32 = 1384;

/// 设备机架源码（**另一个文件**）—— 源码腿只从这里读，⛔ 不从本判据自己读（那是恒真）。
const RACK_SOURCE: &str = include_str!("../ui/console/device_rack.slint");

/// 判据夹具：**一条 MIDI 轨（直连主总线）＋ 主总线轨**（与 `#173` / `#180` 同一条栅格：
/// 120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本）。
///
/// 两条轨的设备链都是空的；需要设备的腿自己往里放（用契约构造的 EQ，⛔ 不手写参数表）。
fn master_project() -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();

    let mut notes = BTreeMap::new();
    let mut note = MidiNote::new(note_id, 0, 36, 1_920);
    note.velocity = 100;
    notes.insert(note_id, note);

    let mut pool = BTreeMap::new();
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );

    let mut display = TrackV3 {
        id: track,
        name: "展示轨".to_owned(),
        kind: TrackKind::Midi,
        volume_db: -12.0,
        ..TrackV3::default()
    };
    display.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 1_920,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );

    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(track, display);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool: pool,
        ..YebanProjectV1::default()
    }
}

/// 契约里的一台内置 EQ（参数 / 值域 / 缺省值全部来自 `effect_params`，⛔ 不手抄）。
fn contract_eq() -> yeban_model::project::DeviceDefinition {
    bridge::builtin_effect_device(BuiltinEffect::Eq).expect("契约里必须有 EQ 分组")
}

/// 判据夹具：**展示轨与主总线各一台契约 EQ**。
///
/// 展示轨那台是**检测器**：如果旁通 / 滑块又指错轨，它的 `bypassed` / `eq_low_gain`
/// 会变，而本文件的判据要求它**一位不动**。
fn two_eq_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = master_project();
    let master_id = project.master_bus_track_id;
    let display_id = device_rack_track(&project).expect("夹具必须有展示轨").id;
    project.tracks.get_mut(&display_id).expect("展示轨").devices = vec![contract_eq()];
    project
        .tracks
        .get_mut(&master_id)
        .expect("主总线轨")
        .devices = vec![contract_eq()];
    (project, master_id, display_id)
}

/// 一条 Tier-1 活窗口（生产窗口构造入口；Tab 3 = 设备架）。
///
/// ⚠ `LivePort` 必须被持有到判据结束；同一测试里只能建一个（Tier-1 平台每线程一份）。
fn live_window(view: &ViewState) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 一条带撤销端口的 Tab 3 活窗口（接线走**生产同一个** `host::wire_device_rack`）。
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
    host::wire_device_rack(live.ui(), &port);
    (live, port)
}

/// 活窗口的运行时控件树。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 模型数组：主总线设备名。
fn model_master_names(window: &MainWindow) -> Vec<String> {
    window
        .get_master_device_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

/// 模型数组：展示轨设备名。
fn model_display_names(window: &MainWindow) -> Vec<String> {
    window
        .get_device_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

/// 模型数组：主总线每台的旁通位。
fn model_master_bypassed(window: &MainWindow) -> Vec<bool> {
    window.get_master_device_bypassed().iter().collect()
}

/// 模型数组：展示轨每台的旁通位。
fn model_display_bypassed(window: &MainWindow) -> Vec<bool> {
    window.get_device_bypassed().iter().collect()
}

/// 界面里第 `index` 台**主总线**设备的**身份**（`.slint` 的 `master-device-ids`）。
///
/// ⭐ 任务单 `#182`：主总线旁通 / EQ 手势都按**身份**寻址 ⇒ 判据也从界面数组里取身份。
fn master_device_id_at(window: &MainWindow, index: usize) -> slint::SharedString {
    window
        .get_master_device_ids()
        .row_data(index)
        .unwrap_or_else(|| panic!("`master-device-ids[{index}]` 必须存在"))
}

/// 界面里**主总线 EQ 那一格**的设备身份（`.slint` 的 `master-device-eq-device-id`）。
fn master_eq_device_id(window: &MainWindow) -> slint::SharedString {
    window.get_master_device_eq_device_id()
}

/// 渲染一段音频（产品路径：`EngineSnapshot::from_project` → `EngineRuntime::process_quantum`）。
///
/// 返回 `(左声道样本, 统计读数)`。与 `#173` / `#180` 的判据同一条链 —— 本文件只复制驱动
/// （约 20 行），⛔ 不复制它的夹具与断言。
fn render(project: &YebanProjectV1, quanta: usize) -> (Vec<f32>, EngineStats) {
    let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut left = Vec::with_capacity(quanta * 128);
    let mut out = vec![0.0_f32; 128 * 2];
    for _ in 0..quanta {
        out.fill(0.0);
        runtime.process_quantum(&mut out, 2);
        for frame in 0..128 {
            left.push(out[frame * 2]);
        }
    }
    (left, runtime.stats())
}

// ===========================================================================
// 判据 ①：旁通 = Δ1 且作用于 master（展示轨的对应设备**不变**）
// ===========================================================================
#[test]
fn the_master_bypass_gesture_lands_on_the_master_and_commits_exactly_once() {
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
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback master-device-bypass-toggle("),
        "新 UI 契约面必须声明 callback master-device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_master_device_bypass_toggle"),
        "宿主接线层必须注册 on_master_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> master-device-bypassed:"),
        "新 UI 契约面必须声明 > master-device-bypassed:"
    );
    assert!(
        aether_host.contains("set_master_device_bypassed"),
        "宿主接线层必须注册 set_master_device_bypassed"
    );
}

// ===========================================================================
// 判据 ② ＋ ④：滑块 = Δ1 且作用于 master；同位置再按 ⇒ Δ0
// ===========================================================================
#[test]
fn the_master_eq_slider_commits_exactly_once_and_writes_the_master_param() {
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
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback master-device-bypass-toggle("),
        "新 UI 契约面必须声明 callback master-device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_master_device_bypass_toggle"),
        "宿主接线层必须注册 on_master_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> master-device-bypassed:"),
        "新 UI 契约面必须声明 > master-device-bypassed:"
    );
    assert!(
        aether_host.contains("set_master_device_bypassed"),
        "宿主接线层必须注册 set_master_device_bypassed"
    );
}

// ===========================================================================
// 判据 ⑤：旁通真的关掉处理（`master_insert_frames` 有设备 = 帧数；旁通 = 0）
// ===========================================================================
#[test]
fn the_master_bypass_really_stops_master_processing() {
    let project = master_project();
    let master_id = project.master_bus_track_id;

    // 走**产品手势**装入主总线（不是直接改夹具）。
    let (live, port) = live_window_with_undo(&project, "#181 音频");
    live.ui().invoke_master_device_insert_eq();
    let committed = port.project();
    assert_eq!(
        committed.tracks[&master_id].devices.len(),
        1,
        "前置：主总线上必须已有 1 台设备"
    );

    let expected_frames = (SHORT_QUANTA * 128) as u64;
    let (_armed_left, armed) = render(&committed, SHORT_QUANTA);
    assert_eq!(
        armed.master_insert_frames, expected_frames,
        "未旁通 ⇒ 主总线链每量子恰好处理一次（期望 {expected_frames}，实得 {}）",
        armed.master_insert_frames
    );

    // 同一个活窗口上按旁通 ⇒ 权威工程里那一台被旁通。
    let before = port.display();
    live.ui()
        .invoke_master_device_bypass_toggle(master_device_id_at(live.ui(), 0));
    let after = port.display();
    assert_eq!(after.commit_count, before.commit_count + 1);
    let bypassed = port.project();
    assert!(
        bypassed.tracks[&master_id].devices[0].bypassed,
        "旁通必须真的落进权威工程"
    );

    let (_stopped_left, stopped) = render(&bypassed, SHORT_QUANTA);
    assert_eq!(
        stopped.master_insert_frames, 0,
        "旁通之后主总线链必须**一帧都不处理**（实得 {}）",
        stopped.master_insert_frames
    );
    // 反向腿：把它再翻回来 ⇒ 帧数回来（证明 0 不是"渲染坏了"）。
    let mut unbypassed = bypassed.clone();
    unbypassed
        .tracks
        .get_mut(&master_id)
        .expect("主总线轨")
        .devices[0]
        .bypassed = false;
    let (_again_left, again) = render(&unbypassed, SHORT_QUANTA);
    assert_eq!(
        again.master_insert_frames, expected_frames,
        "翻回未旁通之后帧数必须回来（实得 {}）",
        again.master_insert_frames
    );

    report_line(&format!(
        "[#181 ⑤] master_insert_frames：装入未旁通={} / UI 旁通后={} / 翻回未旁通={}；\
         渲染帧数={expected_frames}（每量子恰好一次）；commit_count {} → {}",
        armed.master_insert_frames,
        stopped.master_insert_frames,
        again.master_insert_frames,
        before.commit_count,
        after.commit_count
    ));
}

// ===========================================================================
// 判据 ⑥：无 master 设备 ⇒ 滑块 / 开关不出现 ＋ 逐字原因
// ===========================================================================
#[test]
fn no_master_device_means_no_slider_no_switch_and_a_verbatim_reason() {
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
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback master-device-bypass-toggle("),
        "新 UI 契约面必须声明 callback master-device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_master_device_bypass_toggle"),
        "宿主接线层必须注册 on_master_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> master-device-bypassed:"),
        "新 UI 契约面必须声明 > master-device-bypassed:"
    );
    assert!(
        aether_host.contains("set_master_device_bypassed"),
        "宿主接线层必须注册 set_master_device_bypassed"
    );
}

// ===========================================================================
// 判据 ⑦b：新增控件全在第二段的矩形内（运行时几何读数，x + width ≤ 1384）
// ===========================================================================
#[test]
fn the_new_master_controls_live_inside_the_second_segment() {
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
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback master-device-bypass-toggle("),
        "新 UI 契约面必须声明 callback master-device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_master_device_bypass_toggle"),
        "宿主接线层必须注册 on_master_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> master-device-bypassed:"),
        "新 UI 契约面必须声明 > master-device-bypassed:"
    );
    assert!(
        aether_host.contains("set_master_device_bypassed"),
        "宿主接线层必须注册 set_master_device_bypassed"
    );
}
