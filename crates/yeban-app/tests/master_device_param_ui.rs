//! ⭐ 任务单 `#181`：**主总线设备的旁通 ＋ 一个 EQ 参数滑块**。
//!
//! `#180` 只给主总线接了**增 / 删**，并逐字登记过"主总线的旁通／EQ 滑块仍不可编辑"。
//! 本文件钉住那两条补上的手势（与展示轨**同一份**实现体，`track` 是唯一自变量）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 旁通作用于 master | `invoke_master_device_bypass_toggle(0)` ⇒ `commit_count +1`；主总线 `devices[0].bypassed` **false → true**，**展示轨 `devices[0].bypassed` 不动**；界面 `master-device-bypassed=[true]` / `device-bypassed=[false]` | 证"旁通落在主总线，⛔ 不是展示轨"（注入：把目标改回 `device_rack_track` ⇒ 这条红） |
//! | ② 滑块作用于 master | `invoke_master_device_eq_grab(1.0)` ＋ `release()` ⇒ `commit_count +1`；读回主总线 `eq_low_gain = 18.0`（契约上界），**展示轨仍是 0.0**；界面 `master-device-eq-gain-db = 18` / `device-eq-gain-db = 0` | 同上，滑块那条腿 |
//! | ③ 注入 | 把 `master_device_eq_low_gain_target` / `master_device_bypass_ops` 的目标改回 `device_rack_track` ⇒ ①② 各自变红（读数见交付报告，⛔ 未提交） | 证这两条腿真的靠"显式的目标轨"工作 |
//! | ④ 同位置再按 Δ0 | 值已经是上界时再 `grab(1.0)` ＋ `release()` ⇒ `commit_count` / `undoable` 一位不动 | 证"没变不产生空提交" |
//! | ⑤ 旁通真的关掉处理 | `master_insert_frames`：未旁通 = `400 × 128`；UI 旁通之后 = **0** | 证器件的武装位真的被旁通位关掉；⛔ 不证人耳听感 |
//! | ⑥ 无 master 设备 | 主总线存在但 0 台 ⇒ `master-device-eq-available=false`、滑块 / 旁通开关**不出现**、原因逐字 = [`MASTER_DEVICE_EQ_NO_TARGET_REASON`]；没有主总线 ⇒ 逐字 = [`MASTER_DEVICE_EQ_NO_MASTER_REASON`] | 证"没有假的可编辑控件，也不静默" |
//! | ⑦ 预算 ＋ 几何 | 预算在 `tests/perf_draw_budget.rs`（**87** 写点 / **49** 数组）；本文件在**运行时控件树**里读新控件的 `bounds`：`x + width ≤ 1384`（第二段右沿） | 证"没有把第二段往右扩" |
//! | ⑧ 不退化 | 不在本文件：`cargo test -p yeban-app --locked --no-fail-fast` 的 rc 与通过数 | — |
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"有人耳听过"；⛔ 不证界面有"选中轨"（没有这个概念）；⛔ 不救窄窗口
//! （第二段本来就住在绝对坐标里，本单只保证**不加剧**：新控件全在 `840..1384` 内）。

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
    let (project, master_id, display_id) = two_eq_project();
    let (live, port) = live_window_with_undo(&project, "#181 旁通");
    let before = port.display();

    // 初始读数：两条轨都各有 1 台、都不是旁通。
    assert_eq!(model_master_names(live.ui()).len(), 1);
    assert_eq!(model_display_names(live.ui()).len(), 1);
    assert_eq!(model_master_bypassed(live.ui()), vec![false]);
    assert_eq!(model_display_bypassed(live.ui()), vec![false]);
    assert_eq!(
        live.ui().get_master_device_bypassed().row_count(),
        live.ui().get_master_device_names().row_count(),
        "旁通位数组必须与名字数组同索引集"
    );

    live.ui().invoke_master_device_bypass_toggle(0);

    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    // 核心读数：master 变了、展示轨**一位没动**。
    let committed = port.project();
    let master_bypassed = committed.tracks[&master_id].devices[0].bypassed;
    let display_bypassed = committed.tracks[&display_id].devices[0].bypassed;
    assert!(
        master_bypassed,
        "主总线 `devices[0].bypassed` 必须 false → true"
    );
    assert!(
        !display_bypassed,
        "展示轨 `devices[0].bypassed` 必须一位不动（变 true 就是旁通指错轨了）"
    );

    // 界面跟着重投影（`refresh_undo_window` 的唯一出口）。
    assert_eq!(model_master_bypassed(live.ui()), vec![true]);
    assert_eq!(model_display_bypassed(live.ui()), vec![false]);

    // 运行时控件树：两个开关的 `checked` 是**真实状态**（不是界面自造的）。
    let tree = runtime_tree(live.ui());
    let eq_name = committed.tracks[&master_id].devices[0].name.clone();
    let master_switch = tree
        .find_by_id("master-device-0-bypass-switch")
        .expect("主总线第 0 台的旁通开关");
    assert_eq!(master_switch.checked, Some(true));
    assert_eq!(master_switch.label, format!("旁通主总线设备 {eq_name}"));
    let display_switch = tree
        .find_by_id("device-0-bypass-switch")
        .expect("展示轨第 0 台的旁通开关");
    assert_eq!(
        display_switch.checked,
        Some(false),
        "展示轨的开关不得跟着主总线一起变"
    );
    let card = tree
        .find_by_id("master-device-0-card")
        .expect("主总线第 0 台卡片");
    assert_eq!(card.label, format!("主总线设备 {eq_name}（已旁通）"));

    report_line(&format!(
        "[#181 ①] 主总线旁通：commit_count {} → {}（Δ1）；undoable {} → {}；\
         主总线 devices[0].bypassed={}；展示轨 devices[0].bypassed={}；\
         界面 master-device-bypassed={:?} / device-bypassed={:?}；\
         运行时 `master-device-0-bypass-switch`.checked={:?} / `device-0-bypass-switch`.checked={:?}",
        before.commit_count,
        after.commit_count,
        before.undoable,
        after.undoable,
        master_bypassed,
        display_bypassed,
        model_master_bypassed(live.ui()),
        model_display_bypassed(live.ui()),
        master_switch.checked,
        display_switch.checked
    ));
}

// ===========================================================================
// 判据 ② ＋ ④：滑块 = Δ1 且作用于 master；同位置再按 ⇒ Δ0
// ===========================================================================
#[test]
fn the_master_eq_slider_commits_exactly_once_and_writes_the_master_param() {
    let (project, master_id, display_id) = two_eq_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(
        view.master_device_eq_available(),
        "主总线上有一台契约 EQ ⇒ 那一格必须可编辑"
    );
    assert_eq!(
        (
            view.master_device_eq_min_db(),
            view.master_device_eq_max_db()
        ),
        (-18.0, 18.0),
        "值域必须来自效果器契约"
    );
    assert_eq!(view.master_device_eq_gain_db(), 0.0);
    assert!(
        view.master_device_eq_reason.is_empty(),
        "可用时原因必须是空串（实得 {:?}）",
        view.master_device_eq_reason
    );

    let (live, port) = live_window_with_undo(&project, "#181 滑块");
    let before = port.display();
    assert!((live.ui().get_master_device_eq_gain_db() - 0.0).abs() < f32::EPSILON);
    assert!((live.ui().get_device_eq_gain_db() - 0.0).abs() < f32::EPSILON);

    // ---- 一次拖动（按下在最右端 ⇒ +18 dB，松手提交一次）----
    live.ui().invoke_master_device_eq_grab(1.0);
    live.ui().invoke_master_device_eq_release();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次拖动必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    // 读回**主总线**的 `eq_low_gain`（⛔ 不信界面），展示轨仍是 0 dB。
    let committed = port.project();
    let master_low_gain = committed.tracks[&master_id].devices[0]
        .params
        .iter()
        .find(|param| param.name == "eq_low_gain")
        .expect("主总线那台 EQ 必须有 eq_low_gain");
    let display_low_gain = committed.tracks[&display_id].devices[0]
        .params
        .iter()
        .find(|param| param.name == "eq_low_gain")
        .expect("展示轨那台 EQ 必须有 eq_low_gain");
    assert_eq!(
        master_low_gain.value.to_bits(),
        18.0_f32.to_bits(),
        "拖到最右端必须把**主总线**那台写成值域上界（读回 {}）",
        master_low_gain.value
    );
    assert_eq!(
        display_low_gain.value.to_bits(),
        0.0_f32.to_bits(),
        "展示轨那台必须一位不动（读回 {} —— 变 18 就是滑块指错轨了）",
        display_low_gain.value
    );
    let (target_track, slot, param_index, value, lo, hi) =
        master_device_eq_low_gain_target(&committed).expect("提交之后主总线目标仍在");
    assert_eq!(target_track, master_id);
    assert_eq!((slot, param_index), (0, 0));
    assert_eq!((lo, hi), (-18.0, 18.0), "值域来自契约");
    assert_eq!(value.to_bits(), 18.0_f32.to_bits());
    let (_, _, _, display_value, _, _) =
        device_eq_low_gain_target(&committed).expect("展示轨目标仍在");
    assert_eq!(display_value.to_bits(), 0.0_f32.to_bits());

    // 界面读数：主总线那一格跟上，展示轨那一格不动。
    assert!((live.ui().get_master_device_eq_gain_db() - value).abs() < f32::EPSILON);
    assert!((live.ui().get_device_eq_gain_db() - 0.0).abs() < f32::EPSILON);
    // 记下**提交之后那一帧**的界面读数（后面还要跑 Δ0 与中点两条腿，会把它覆盖）。
    let ui_master_after_commit = live.ui().get_master_device_eq_gain_db();
    let ui_display_after_commit = live.ui().get_device_eq_gain_db();
    let tree = runtime_tree(live.ui());
    let slider = tree
        .find_by_id("master-device-eq-slider")
        .expect("主总线的 EQ 滑块必须存在（可用时）");
    assert_eq!(slider.role.as_str(), "slider");
    assert_eq!(
        slider.value.as_deref(),
        Some("18 dB"),
        "无障碍值必须逐字等于注入的当前值"
    );

    // ---- 判据 ④：同一个位置再按一次 ⇒ **不提交**（⛔ 不留一步什么都没改的撤销）----
    let quiet = port.display();
    live.ui().invoke_master_device_eq_grab(1.0);
    live.ui().invoke_master_device_eq_release();
    let still = port.display();
    assert_eq!(
        still.commit_count, quiet.commit_count,
        "同一个位置按下再松手不得产生提交"
    );
    assert_eq!(still.undoable, quiet.undoable);

    // ---- 回到中点 ⇒ 0 dB，仍是一次提交（证明值真的往返）----
    live.ui().invoke_master_device_eq_grab(0.5);
    live.ui().invoke_master_device_eq_release();
    let clicked = port.display();
    assert_eq!(clicked.commit_count, still.commit_count + 1);
    let (_, _, _, mid, _, _) = master_device_eq_low_gain_target(&port.project()).expect("目标仍在");
    assert_eq!(mid.to_bits(), 0.0_f32.to_bits(), "中点必须写 0 dB");
    let (_, _, _, display_mid, _, _) =
        device_eq_low_gain_target(&port.project()).expect("展示轨目标仍在");
    assert_eq!(
        display_mid.to_bits(),
        0.0_f32.to_bits(),
        "展示轨那台仍是一位不动"
    );

    report_line(&format!(
        "[#181 ②] 主总线 EQ：commit_count {} → {}（Δ1）；undoable {} → {}；\
         读回 master.eq_low_gain={} dB（值域 {}..={}）；展示轨 eq_low_gain={} dB；\
         界面 master-device-eq-gain-db={} / device-eq-gain-db={}；\
         同位置再按 Δ0（{}）；单击中点读回 {mid} dB（Δ1）",
        before.commit_count,
        after.commit_count,
        before.undoable,
        after.undoable,
        master_low_gain.value,
        lo,
        hi,
        display_low_gain.value,
        ui_master_after_commit,
        ui_display_after_commit,
        still.commit_count
    ));
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
    live.ui().invoke_master_device_bypass_toggle(0);
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
    // ---- A. 主总线在，但 0 台设备 ----
    let project = master_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(view.master_devices.is_empty());
    assert!(!view.master_device_eq_available());
    assert!(view.master_eq_low_gain.is_none());
    assert!(!view.master_device_eq_reason.is_empty());
    assert_eq!(
        view.master_device_eq_reason, MASTER_DEVICE_EQ_NO_TARGET_REASON,
        "没有设备时原因必须逐字"
    );
    assert_eq!(
        master_device_eq_availability(&project),
        (false, MASTER_DEVICE_EQ_NO_TARGET_REASON.to_owned())
    );
    assert!(master_device_eq_low_gain_target(&project).is_none());
    assert_eq!(
        master_device_bypass_ops(&project, 0),
        Err(MASTER_DEVICE_BYPASS_NO_DEVICE_REASON)
    );

    let live = live_window(&view);
    assert!(!live.ui().get_master_device_eq_available());
    assert_eq!(
        live.ui().get_master_device_eq_reason().to_string(),
        MASTER_DEVICE_EQ_NO_TARGET_REASON
    );
    assert_eq!(live.ui().get_master_device_bypassed().row_count(), 0);
    let tree = runtime_tree(live.ui());
    // 旁通开关由 `for … in master-device-names` 生成 ⇒ 0 台时一个都没有。
    assert!(
        tree.find_by_id("master-device-0-bypass-switch").is_none(),
        "主总线 0 台设备时不得画出旁通开关"
    );
    assert!(
        tree.find_by_id("master-device-0-card").is_none(),
        "主总线 0 台设备时不得画出卡片"
    );
    // 滑块只随 `master-device-eq-available` 出现；不可用时只留**逐字原因**。
    assert!(
        tree.find_by_id("master-device-eq-slider").is_none(),
        "没有 eq_low_gain 目标时滑块不得出现"
    );
    assert!(
        tree.find_by_id("master-device-eq-value").is_none(),
        "没有目标时不得画一个假读数"
    );
    let reason = tree
        .find_by_id("master-device-eq-reason")
        .expect("不可用时必须有一格逐字原因");
    assert_eq!(reason.label, MASTER_DEVICE_EQ_NO_TARGET_REASON);
    assert!(tree.find_by_id("master-device-rack-empty").is_some());

    // ---- B. 没有主总线（0 轨工程）----
    let empty = YebanProjectV1::default();
    let empty_view = ViewState::from_project(&empty).expect("0 轨工程也必须能投影");
    assert!(!empty_view.master_device_eq_available());
    assert_eq!(
        empty_view.master_device_eq_reason,
        MASTER_DEVICE_EQ_NO_MASTER_REASON
    );
    assert_eq!(
        master_device_eq_availability(&empty),
        (false, MASTER_DEVICE_EQ_NO_MASTER_REASON.to_owned())
    );
    assert_eq!(
        master_device_bypass_ops(&empty, 0),
        Err(MASTER_DEVICE_BYPASS_NO_MASTER_REASON)
    );
    assert!(master_device_track(&empty).is_none());

    // 注册表侧：这两个元素是**无条件声明**的（只有 `visible` 随投影变）⇒ 仍在清单里。
    let registry = ElementRegistry::from_view(&empty_view);
    for id in [
        "master-device-eq-panel",
        "master-device-eq-slider",
        "master-device-eq-value",
        "master-device-eq-reason",
    ] {
        assert!(registry.contains(id), "注册表缺少 `{id}`");
    }

    report_line(&format!(
        "[#181 ⑥] 主总线 0 台：available=false · reason={:?} · 运行时 slider/card/switch 全 0 个；\
         无主总线：available=false · reason={:?} · bypass_ops={:?}；注册表 4 个静态 ID 全在",
        MASTER_DEVICE_EQ_NO_TARGET_REASON,
        MASTER_DEVICE_EQ_NO_MASTER_REASON,
        master_device_bypass_ops(&empty, 0)
    ));
}

// ===========================================================================
// 判据 ⑦b：新增控件全在第二段的矩形内（运行时几何读数，x + width ≤ 1384）
// ===========================================================================
#[test]
fn the_new_master_controls_live_inside_the_second_segment() {
    let (project, _master_id, _display_id) = two_eq_project();
    let view = ViewState::from_project(&project).expect("投影");
    let live = live_window(&view);
    let tree = runtime_tree(live.ui());

    let segment = tree
        .find_by_id("master-device-rack")
        .expect("第二段（主总线设备链）");
    let segment_bounds = segment.bounds.expect("运行时必须有几何");
    assert_eq!(
        (segment_bounds.x, segment_bounds.width),
        (840, 544),
        "第二段的几何是本判据的基准（x=840 / width=544）"
    );
    assert_eq!(
        segment_bounds.x as u32 + segment_bounds.width,
        MASTER_SEGMENT_RIGHT,
        "第二段右沿必须钉在 1384"
    );

    let mut readings: Vec<String> = Vec::new();
    for id in [
        "master-device-eq-panel",
        "master-device-eq-slider",
        "master-device-eq-value",
        "master-device-0-bypass-switch",
        "master-device-0-card",
    ] {
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("运行时控件树里缺少 `{id}`"));
        let bounds = node.bounds.unwrap_or_else(|| panic!("`{id}` 必须有几何"));
        let right = bounds.x as u32 + bounds.width;
        assert!(
            bounds.x as u32 >= segment_bounds.x as u32,
            "`{id}` 的左沿 {} 越出第二段左沿 {}",
            bounds.x,
            segment_bounds.x
        );
        assert!(
            right <= MASTER_SEGMENT_RIGHT,
            "`{id}` 的右沿 {right} 超出第二段右沿 {MASTER_SEGMENT_RIGHT}（x={} / width={}）",
            bounds.x,
            bounds.width
        );
        readings.push(format!(
            "{id}: x={} width={} right={right}",
            bounds.x, bounds.width
        ));
    }

    // 源码腿（⛔ 不是靠"我写的时候算过"）：回调 / 属性名与两段的手势落点逐字在 `.slint` 里。
    for needle in [
        "callback master-device-bypass-toggle(int);",
        "callback master-device-eq-grab(float);",
        "callback master-device-eq-drag(float);",
        "callback master-device-eq-release();",
        "in property <[bool]> master-device-bypassed: [];",
        "root.master-device-bypass-toggle(master_device_index)",
        "root.master-device-eq-grab(self.mouse-x / self.width)",
        "root.master-device-eq-drag(self.mouse-x / self.width)",
        "root.master-device-eq-release()",
    ] {
        assert!(
            RACK_SOURCE.contains(needle),
            "`device_rack.slint` 必须逐字含 `{needle}`"
        );
    }

    report_line(&format!(
        "[#181 ⑦b] 第二段 x=840 width=544 ⇒ 右沿 {}；新控件右沿读数：{}",
        MASTER_SEGMENT_RIGHT,
        readings.join("；")
    ));
}
