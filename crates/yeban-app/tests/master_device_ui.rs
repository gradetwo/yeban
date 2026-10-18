//! ⭐ 任务单 `#180`：**主总线（master）上的设备在界面上可增删**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 装到 master 而非展示轨 | `invoke_master_device_insert_eq()` ⇒ 权威工程的主总线 `devices.len()` **0 → 1**、**展示轨 0 → 0**；界面 `master-device-names` 0 → 1、`device-names` 0 → 0 | 证"不再指错轨"（`#163` 的洞：那条实现走 `device_rack_track`，它**排除**主总线） |
//! | ② 删的也是 master | `invoke_master_device_remove(master-device-ids[0])` ⇒ 主总线 **1 → 0**、展示轨 **0 → 0** | 同上，删的那条腿 |
//! | ③ 每手势恰好一次提交 | 两个手势各 `commit_count +1` / `undoable +1`；被拒绝的腿 `Δ0` | 证"一次点击 = 一步可撤销的编辑" |
//! | ④ 装完能在链上 | 读回 `master.devices[0].params` 非空且含 `eq_low_gain`（值 = 契约缺省 `0`） | 证"设备真的进了权威工程"，⛔ 不信界面 |
//! | ⑤ 装完真的会处理音频 | `master_insert_frames == 400 × 128`（主总线链每量子恰好处理一次）；无设备时 **0**；把同一夹具的同名参数改成 `+12 dB` ⇒ 低频 RMS 上升 | 证"接在最终输出上"；⛔ 不证人耳听感 |
//! | ⑥ 空态／禁用 | 0 轨工程 ⇒ 主总线装设备 `false`，原因逐字 = [`MASTER_DEVICE_INSERT_NO_MASTER_REASON`]；只有主总线 ⇒ 装主总线可用、展示轨那三个手势仍 `false` 且逐字原因 = [`DEVICE_INSERT_NO_TRACK_REASON`] | 证"两条轨的可用性互不蕴含，且⛔ 不静默" |
//! | ⑦ 预算三件套 | 不在本文件：`tests/perf_draw_budget.rs` 的 **81** 写点 / **48** 数组（常量与原因同处） | — |
//! | ⑧ 不退化 | 不在本文件：`cargo test -p yeban-app --locked --no-fail-fast` 的 rc 与通过数 | — |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `master_device_insert_ops` 改回 `device_rack_track` ⇒ ① 红（主总线 0，展示轨 1）；
//! - 把 `master_device_remove_ops` 改回 `device_rack_track` ⇒ ② 红（主总线不动，展示轨被删）；
//! - 摘掉 `host::wire_device_rack` 里新的 `commit_ops` ⇒ ① / ② 的 `commit_count` 差 0；
//! - 把 `master_device_insert_availability` 写成"恒 true" ⇒ ⑥ 的逐字原因腿红；
//! - 把 `.slint` 的删除按钮改成传行下标 ⇒ ④ 的源码腿红（本文件扫的是 `device_rack.slint`）。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"有人耳听过"；⛔ 不证"界面有选中轨"（没有这个概念 —— 面板服务的两条轨都是
//! **固定对象**：身份序第一条非主总线轨 ＋ `master_bus_track_id`）。

use std::collections::BTreeMap;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{
    self, BuiltinEffect, DEVICE_INSERT_NO_TRACK_REASON, MASTER_DEVICE_INSERT_NO_MASTER_REASON,
    MASTER_DEVICE_REMOVE_NO_MASTER_REASON, ViewState, device_rack_track,
    master_device_insert_availability, master_device_insert_ops, master_device_remove_ops,
    master_device_track, master_device_track_label,
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
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, LoopConfig,
    ParameterValue, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_model::{EntityId, MidiNote, samples};
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

/// 音频判据的渲染长度：400 × 128 = 51,200 帧 ≈ 1.07 s @48 kHz（与 `#173` 的 M1 同款）。
const SHORT_QUANTA: usize = 400;

/// 设备机架源码（**另一个文件**）—— 源码腿只从这里读，⛔ 不从本判据自己读（那是恒真）。
const RACK_SOURCE: &str = include_str!("../ui/console/device_rack.slint");

/// 一台设备（`params` 逐条给 `(名字, 值, 单位)`）。
fn device(name: &str, params: &[(&str, f32, Option<&str>)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(param, value, unit)| ParameterValue {
                name: (*param).to_owned(),
                value: *value,
                unit: unit.map(str::to_owned),
            })
            .collect(),
        latency_samples: 0,
    }
}

/// 判据夹具：**一条 MIDI 轨（直连主总线）＋ 主总线轨**（与 `#173` 的 M1 同一条栅格：
/// 120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本）。
///
/// ⚠ **展示轨 `devices` 是空的** —— 这是判据 ① / ② 的检测器：如果装 / 删仍然落在
/// 展示轨上，读数会是 `0 → 1`，而正确形态是 `0 → 0`。
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

/// 渲染一段音频（产品路径：`EngineSnapshot::from_project` → `EngineRuntime::process_quantum`）。
///
/// 返回 `(左声道样本, 统计读数)`。这是 `#173` 判据用的同一条链 —— 本文件只复制驱动
/// （约 15 行），⛔ 不复制它的夹具与断言。
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

/// 线性 RMS（与 `#173` 的判据同一把尺子：只看音符还在响的窗口）。
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let energy: f64 = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    (energy / samples.len() as f64).sqrt()
}

// ===========================================================================
// 判据 ① ＋ ③ ＋ ④：装到 master（不是展示轨）＋ 恰好一次提交 ＋ 读回参数
// ===========================================================================
#[test]
fn the_insert_gesture_lands_on_the_master_and_commits_exactly_once() {
    let project = master_project();
    let master_id = project.master_bus_track_id;
    let (live, port) = live_window_with_undo(&project, "#180 装");
    let before = port.display();

    // 夹具读数：两条轨的设备链都是空的；界面两段也都是 0 台。
    assert_eq!(port.project().tracks[&master_id].devices.len(), 0);
    assert_eq!(
        device_rack_track(&port.project())
            .expect("夹具必须有展示轨")
            .devices
            .len(),
        0
    );
    assert!(model_master_names(live.ui()).is_empty());
    assert!(model_display_names(live.ui()).is_empty());

    live.ui().invoke_master_device_insert_eq();

    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(
        after.undoable,
        before.undoable + 1,
        "一次点击必须恰好留下一步可撤销的编辑"
    );

    // ⭐ 核心读数：装到 master，展示轨**一位没动**。
    let committed = port.project();
    let master_devices = &committed.tracks[&master_id].devices;
    let display_devices = &device_rack_track(&committed).expect("展示轨仍在").devices;
    assert_eq!(
        master_devices.len(),
        1,
        "主总线的设备链必须 0 → 1（实得 {}）",
        master_devices.len()
    );
    assert_eq!(
        display_devices.len(),
        0,
        "展示轨的设备链必须 0 → 0（实得 {} —— 非 0 就是又指错轨了）",
        display_devices.len()
    );

    // ④ 装完能在链上：参数非空且 `eq_low_gain` 在里面，值 = 契约缺省 0 dB。
    let inserted = &master_devices[0];
    assert!(
        !inserted.params.is_empty(),
        "装进去的设备必须有参数（⛔ 不是空壳）"
    );
    let low_gain = inserted
        .params
        .iter()
        .find(|param| param.name == "eq_low_gain")
        .expect("设备参数里必须有 eq_low_gain");
    assert_eq!(low_gain.value.to_bits(), 0.0_f32.to_bits());

    // 界面跟着重投影：主总线数组 0 → 1、展示轨数组 0 → 0。
    let names = model_master_names(live.ui());
    let display_names = model_display_names(live.ui());
    assert_eq!(names.len(), 1);
    assert!(
        display_names.is_empty(),
        "展示轨的数组不得被写：{display_names:?}"
    );
    assert_eq!(names[0], inserted.name);
    assert_eq!(
        live.ui().get_master_device_ids().row_count(),
        1,
        "身份数组必须与名字数组同索引集"
    );

    // 投影一致：`ViewState::master_devices` / `devices` 与权威工程逐条对齐。
    let view = ViewState::from_project(&committed).expect("投影");
    assert_eq!(view.master_devices.len(), 1);
    assert!(view.devices.is_empty());
    assert_eq!(
        view.master_device_label,
        master_device_track_label(&committed)
    );

    report_line(&format!(
        "[#180 ①] 装 EQ：commit_count {} → {}（Δ1）；undoable {} → {}；\
         主总线 devices 0 → {}；展示轨 devices 0 → {}；参数 {} 条、eq_low_gain={} dB；\
         界面 master-device-names={:?} / device-names={:?}",
        before.commit_count,
        after.commit_count,
        before.undoable,
        after.undoable,
        master_devices.len(),
        display_devices.len(),
        inserted.params.len(),
        low_gain.value,
        names,
        display_names
    ));
}

// ===========================================================================
// 判据 ② ＋ ③：删的也是 master（不是展示轨）＋ 恰好一次提交
// ===========================================================================
#[test]
fn the_remove_gesture_lands_on_the_master_and_commits_exactly_once() {
    let project = master_project();
    let master_id = project.master_bus_track_id;
    let (live, port) = live_window_with_undo(&project, "#180 删");

    live.ui().invoke_master_device_insert_eq();
    let inserted_count = port.project().tracks[&master_id].devices.len();
    assert_eq!(inserted_count, 1, "前置：主总线上必须已有 1 台");
    let device_id = port.project().tracks[&master_id].devices[0].id;

    let before = port.display();
    // 界面报的是**身份**（与投影同索引集），⛔ 不是行下标。
    let ui_id = live
        .ui()
        .get_master_device_ids()
        .row_data(0)
        .expect("身份行");
    assert_eq!(ui_id.to_string(), device_id.to_canonical_string());
    live.ui().invoke_master_device_remove(ui_id);

    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`"
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    assert_eq!(
        committed.tracks[&master_id].devices.len(),
        0,
        "主总线 1 → 0 必须生效（实得 {} —— 1 说明删到了别处）",
        committed.tracks[&master_id].devices.len()
    );
    let display_devices = &device_rack_track(&committed).expect("展示轨仍在").devices;
    assert!(
        display_devices.is_empty(),
        "展示轨必须 0 → 0（实得 {} —— 非 0 就是删错轨了）",
        display_devices.len()
    );
    assert!(model_master_names(live.ui()).is_empty());
    assert!(model_display_names(live.ui()).is_empty());

    report_line(&format!(
        "[#180 ②] 删设备：commit_count {} → {}（Δ1）；undoable {} → {}；\
         主总线 devices 1 → {}；展示轨 devices 0 → {}；删除的身份={}",
        before.commit_count,
        after.commit_count,
        before.undoable,
        after.undoable,
        committed.tracks[&master_id].devices.len(),
        display_devices.len(),
        device_id.to_canonical_string()
    ));
}

// ===========================================================================
// 判据 ③：主总线设备的已识别 / 未识别显示（与展示轨**同一套**）
// ===========================================================================
#[test]
fn the_master_rack_shows_recognised_and_unrecognised_params_like_the_display_track() {
    let mut project = master_project();
    let master_id = project.master_bus_track_id;
    project
        .tracks
        .get_mut(&master_id)
        .expect("主总线轨")
        .devices = vec![device(
        "主总线 EQ",
        &[("eq_low_gain", 6.0, Some("dB")), ("not_a_param", 1.0, None)],
    )];

    let view = ViewState::from_project(&project).expect("投影");
    assert_eq!(view.master_devices.len(), 1);
    let row = &view.master_devices[0];
    assert!(
        row.recognised.contains("eq_low_gain= 6 dB"),
        "已识别分档必须与展示轨同一份：{:?}",
        row.recognised
    );
    assert!(
        row.unrecognised.contains("not_a_param"),
        "未识别分档必须与展示轨同一份：{:?}",
        row.unrecognised
    );

    let live = live_window(&view);
    assert_eq!(
        live.ui()
            .get_master_device_recognised()
            .row_data(0)
            .expect("已识别行")
            .to_string(),
        row.recognised
    );
    assert_eq!(
        live.ui()
            .get_master_device_unrecognised()
            .row_data(0)
            .expect("未识别行")
            .to_string(),
        row.unrecognised
    );

    // 运行时控件树：两行读数各有一个可见节点（与展示轨那一段同一套 ID 形状）。
    let tree = runtime_tree(live.ui());
    let recognised = tree
        .find_by_id("master-device-0-recognised")
        .expect("主总线第 0 台的已识别读数节点");
    assert!(recognised.label.contains("eq_low_gain= 6 dB"));
    let remove = tree
        .find_by_id("master-device-0-remove-button")
        .expect("主总线第 0 台的删除按钮");
    assert_eq!(remove.label, "移除主总线设备 主总线 EQ");
    // 展示轨那一段仍是空的（主总线的设备⛔ 不得并进可编辑的第一段）。
    assert!(
        tree.find_by_id("device-0-card").is_none(),
        "展示轨 0 台设备 ⇒ 不得画出 `device-0-card`"
    );

    // 注册表也必须登记主总线那一族（`.slint` ↔ 注册表双向覆盖由
    // `elements.rs` 的 `slint_accessible_ids_and_registry_cover_each_other` 机械检查）。
    let registry = ElementRegistry::from_view(&view);
    for id in [
        "master-device-rack",
        "master-device-track-label",
        "master-device-rack-count",
        "master-device-insert-eq-button",
        "master-device-insert-reverb-button",
        "master-device-insert-reason",
        "master-device-rack-empty",
        "master-device-0-card",
        "master-device-0-recognised",
        "master-device-0-unrecognised",
        "master-device-0-remove-button",
    ] {
        assert!(registry.contains(id), "注册表缺少 `{id}`");
    }

    // 源码腿：三个回调用真名字（⛔ 不是靠行下标），且删除携带**身份**数组。
    for needle in [
        "callback master-device-insert-eq();",
        "callback master-device-insert-reverb();",
        "callback master-device-remove(string);",
        "root.master-device-remove(root.master-device-ids[master_device_index])",
        "for master_device_name[master_device_index] in root.master-device-names",
    ] {
        assert!(
            RACK_SOURCE.contains(needle),
            "`device_rack.slint` 必须逐字含 `{needle}`"
        );
    }

    report_line(&format!(
        "[#180 ③] 主总线第 0 台：recognised={:?} / unrecognised={:?}；\
         运行时 `master-device-0-recognised` / `-remove-button` 各 1；注册表 11 个 ID 全在；\
         `.slint` 源码腿 5 条全在",
        row.recognised, row.unrecognised
    ));
}

// ===========================================================================
// 判据 ⑤：装完真的会处理音频（结构读数 ＋ 峰值变化）
// ===========================================================================
#[test]
fn the_master_chain_processes_every_frame_after_the_ui_insert() {
    let project = master_project();
    let master_id = project.master_bus_track_id;

    // 走**产品手势**装入主总线（不是直接改夹具）。
    let (live, port) = live_window_with_undo(&project, "#180 音频");
    live.ui().invoke_master_device_insert_eq();
    let committed = port.project();
    assert_eq!(committed.tracks[&master_id].devices.len(), 1);

    // 无设备的同一夹具：快路 ⇒ 主总线链**一帧都不处理**。
    let (plain_left, plain_stats) = render(&project, SHORT_QUANTA);
    // 装完之后：每量子**恰好一次** ⇒ 400 × 128 = 51,200 帧。
    let (installed_left, installed_stats) = render(&committed, SHORT_QUANTA);
    let expected_frames = (SHORT_QUANTA * 128) as u64;
    assert_eq!(
        plain_stats.master_insert_frames, 0,
        "主总线无设备时必须是快路（0 帧）"
    );
    assert_eq!(
        installed_stats.master_insert_frames, expected_frames,
        "主总线设备每量子恰好处理一次：期望 {expected_frames}，实得 {}",
        installed_stats.master_insert_frames
    );
    assert!(
        installed_stats.insert_strip_frames >= expected_frames,
        "通道条读数也必须非零（至少主总线那一份）：{}",
        installed_stats.insert_strip_frames
    );

    // 峰值变化：把**同一份**已装好的工程里的 `eq_low_gain` 改成 +12 dB（夹具级变异），
    // 再看音符还在响的那个窗口的 RMS。默认值 0 dB 是契约缺省 ⇒ 不改就**理应**听不出
    // 差别，所以"处理了"由上面的覆盖度见证给，而"能改变声音"由这一条给。
    let mut boosted_project = committed.clone();
    boosted_project
        .tracks
        .get_mut(&master_id)
        .expect("主总线轨")
        .devices[0]
        .params
        .iter_mut()
        .find(|param| param.name == "eq_low_gain")
        .expect("eq_low_gain")
        .value = 12.0;
    let (boosted_left, _) = render(&boosted_project, SHORT_QUANTA);
    let window = 0..48_000;
    let plain_rms = rms(&plain_left[window.clone()]);
    let installed_rms = rms(&installed_left[window.clone()]);
    let boosted_rms = rms(&boosted_left[window]);
    assert!(
        boosted_rms > installed_rms,
        "master `eq_low_gain: +12 dB` 必须抬高低频夹具的 RMS：装好（0 dB）={installed_rms:.9} / \
         +12 dB={boosted_rms:.9}（无设备={plain_rms:.9}）"
    );

    report_line(&format!(
        "[#180 ⑤] master_insert_frames：无设备={} / 装完={}（渲染帧数 {}，每量子恰好一次）；\
         insert_strip_frames={}；RMS 无设备={plain_rms:.9} / 装好(0 dB)={installed_rms:.9} / \
         +12 dB={boosted_rms:.9}（差 {:+.3} dB）",
        plain_stats.master_insert_frames,
        installed_stats.master_insert_frames,
        expected_frames,
        installed_stats.insert_strip_frames,
        20.0 * (boosted_rms / installed_rms).log10()
    ));
}

// ===========================================================================
// 判据 ⑥：空态／禁用（无 master ／ 无展示轨 ⇒ 各自的逐字原因，⛔ 不静默）
// ===========================================================================
#[test]
fn the_master_gestures_are_disabled_with_verbatim_reasons() {
    // ---- 没有主总线（0 轨 / nil 主总线的工程）----
    let empty = YebanProjectV1::default();
    let empty_view = ViewState::from_project(&empty).expect("0 轨工程也必须能投影");
    assert!(empty_view.master_devices.is_empty());
    assert_eq!(
        empty_view.master_device_label, MASTER_DEVICE_INSERT_NO_MASTER_REASON,
        "没有主总线时标题格**就是**逐字原因"
    );
    assert!(!empty_view.master_device_insert_available);
    assert_eq!(
        empty_view.master_device_insert_reason,
        MASTER_DEVICE_INSERT_NO_MASTER_REASON
    );
    assert_eq!(
        master_device_insert_availability(&empty),
        (false, MASTER_DEVICE_INSERT_NO_MASTER_REASON.to_owned())
    );
    assert_eq!(
        master_device_insert_ops(&empty, BuiltinEffect::Eq),
        Err(MASTER_DEVICE_INSERT_NO_MASTER_REASON)
    );
    assert_eq!(
        master_device_remove_ops(&empty, EntityId::new()),
        Err(MASTER_DEVICE_REMOVE_NO_MASTER_REASON)
    );
    assert!(master_device_track(&empty).is_none());

    // ---- 只有主总线（没有展示轨）----
    let master_only = samples::default_project();
    let master_only_view = ViewState::from_project(&master_only).expect("只有主总线的工程");
    assert!(
        master_only_view.master_device_insert_available,
        "有主总线 ⇒ 主总线装设备可用（与展示轨是否存在无关）"
    );
    assert!(master_only_view.master_device_insert_reason.is_empty());
    assert!(
        !master_only_view.device_insert_available,
        "没有展示轨 ⇒ 第一段的三个手势仍禁用"
    );
    assert_eq!(
        master_only_view.device_insert_reason,
        DEVICE_INSERT_NO_TRACK_REASON
    );
    assert_eq!(
        bridge::device_insert_ops(&master_only, BuiltinEffect::Eq),
        Err(DEVICE_INSERT_NO_TRACK_REASON)
    );
    // 反方向：即使没有展示轨，主总线的装设备**仍然**可用，而且真的落在主总线上。
    let (live, port) = live_window_with_undo(&master_only, "#180 空态");
    assert!(live.ui().get_master_device_insert_available());
    assert!(!live.ui().get_device_insert_available());
    assert_eq!(
        live.ui().get_device_insert_reason().to_string(),
        DEVICE_INSERT_NO_TRACK_REASON
    );
    assert!(live.ui().get_master_device_insert_reason().is_empty());
    live.ui().invoke_master_device_insert_eq();
    let committed = port.project();
    let master_id = committed.master_bus_track_id;
    assert_eq!(committed.tracks[&master_id].devices.len(), 1);
    assert!(
        device_rack_track(&committed).is_none(),
        "工程仍然没有展示轨（⛔ 不得凭空造一条）"
    );

    report_line(&format!(
        "[#180 ⑥] 无主总线：master-insert-available=false · reason={:?} \
         （remove 的另一句={:?}）；无展示轨：device-insert-available=false · reason={:?} · \
         主总线装设备=true 且真的落在主总线上（devices={}）",
        MASTER_DEVICE_INSERT_NO_MASTER_REASON,
        MASTER_DEVICE_REMOVE_NO_MASTER_REASON,
        DEVICE_INSERT_NO_TRACK_REASON,
        committed.tracks[&master_id].devices.len()
    ));
}
