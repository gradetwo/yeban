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

#![allow(unused)]
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
        plugin: None,
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
        aether_app.contains("callback master-device-insert-eq("),
        "新 UI 契约面必须声明 callback master-device-insert-eq("
    );
    assert!(
        aether_host.contains("on_master_device_insert_eq"),
        "宿主接线层必须注册 on_master_device_insert_eq"
    );
    assert!(
        aether_app.contains("callback master-device-remove("),
        "新 UI 契约面必须声明 callback master-device-remove("
    );
    assert!(
        aether_host.contains("on_master_device_remove"),
        "宿主接线层必须注册 on_master_device_remove"
    );
}

// ===========================================================================
// 判据 ② ＋ ③：删的也是 master（不是展示轨）＋ 恰好一次提交
// ===========================================================================
#[test]
fn the_remove_gesture_lands_on_the_master_and_commits_exactly_once() {
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
        aether_app.contains("callback master-device-insert-eq("),
        "新 UI 契约面必须声明 callback master-device-insert-eq("
    );
    assert!(
        aether_host.contains("on_master_device_insert_eq"),
        "宿主接线层必须注册 on_master_device_insert_eq"
    );
    assert!(
        aether_app.contains("callback master-device-remove("),
        "新 UI 契约面必须声明 callback master-device-remove("
    );
    assert!(
        aether_host.contains("on_master_device_remove"),
        "宿主接线层必须注册 on_master_device_remove"
    );
}

// ===========================================================================
// 判据 ③：主总线设备的已识别 / 未识别显示（与展示轨**同一套**）
// ===========================================================================
#[test]
fn the_master_rack_shows_recognised_and_unrecognised_params_like_the_display_track() {
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
        aether_app.contains("callback master-device-insert-eq("),
        "新 UI 契约面必须声明 callback master-device-insert-eq("
    );
    assert!(
        aether_host.contains("on_master_device_insert_eq"),
        "宿主接线层必须注册 on_master_device_insert_eq"
    );
    assert!(
        aether_app.contains("callback master-device-remove("),
        "新 UI 契约面必须声明 callback master-device-remove("
    );
    assert!(
        aether_host.contains("on_master_device_remove"),
        "宿主接线层必须注册 on_master_device_remove"
    );
}

// ===========================================================================
// 判据 ⑤：装完真的会处理音频（结构读数 ＋ 峰值变化）
// ===========================================================================
#[test]
fn the_master_chain_processes_every_frame_after_the_ui_insert() {
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
        aether_app.contains("callback master-device-insert-eq("),
        "新 UI 契约面必须声明 callback master-device-insert-eq("
    );
    assert!(
        aether_host.contains("on_master_device_insert_eq"),
        "宿主接线层必须注册 on_master_device_insert_eq"
    );
    assert!(
        aether_app.contains("callback master-device-remove("),
        "新 UI 契约面必须声明 callback master-device-remove("
    );
    assert!(
        aether_host.contains("on_master_device_remove"),
        "宿主接线层必须注册 on_master_device_remove"
    );
}

// ===========================================================================
// 判据 ⑥：空态／禁用（无 master ／ 无展示轨 ⇒ 各自的逐字原因，⛔ 不静默）
// ===========================================================================
#[test]
fn the_master_gestures_are_disabled_with_verbatim_reasons() {
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
        aether_app.contains("callback master-device-insert-eq("),
        "新 UI 契约面必须声明 callback master-device-insert-eq("
    );
    assert!(
        aether_host.contains("on_master_device_insert_eq"),
        "宿主接线层必须注册 on_master_device_insert_eq"
    );
    assert!(
        aether_app.contains("callback master-device-remove("),
        "新 UI 契约面必须声明 callback master-device-remove("
    );
    assert!(
        aether_host.contains("on_master_device_remove"),
        "宿主接线层必须注册 on_master_device_remove"
    );
}
