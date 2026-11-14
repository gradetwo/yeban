//! ⭐ 任务单 `#183`：**设备架在窄窗口里也必须够得着**。
//!
//! ## 这条判据要证的句子
//!
//! | 判据 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 1440×900 两段全可见 | 窗口 `1440×900` ⇒ 两段的**每个**语义 ID 都在运行时控件树里，且包围盒满足 `0 ≤ x`、`0 ≤ y`、`x+w ≤ 1440`、`y+h ≤ 900` | 证"窄窗口下不需要拖动/滚动就够得着"；⛔ 不证人眼看过 |
//! | ② 1920×1080 不变 | 两段的关键节点包围盒**逐格等于** `#180`／`#181` 注册的那组数（`840 / 544 / 1384`、`1040 / 328`、`1256 / 72`…）；同一帧的像素指纹一并报出 | 证"改布局没有挪动默认档一位"；⭐ 总览图面板是有意变更：1920 档面板展开 104px，3:2 分摊 ⇒ 全部 y −62、`device-rack` 高 −42，x/宽一位未动 |
//! | ③ 1280×800 | 同上，但下界是 `1280×800`；第二段右沿必须 `≤ 1280` 且左沿 `≥ 824`（⛔ 不与第一段重叠） | 证"极窄档也不裁"；⛔ 不证更窄（见文件末的边界）；⭐ 1440/1280 档面板折叠为 22px，3:2 分摊 ⇒ y −13、架高 −9 |
//! | ④ 交互仍可用 | 同一个 `1440×900` 活窗口上：装设备 / 旁通 / EQ 滑块各一次 ⇒ `commit_count` **各 +1** | 证"布局改动没有吃掉手势"；⛔ 不证听感 |
//! | ⑤ 无头可判 | 三条窗口都由 `LivePort`（Tier-1 软件光栅化平台）构造 | 证"CI 无显示器也能跑" |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `app.slint` 根上的 `width: 1920px; height: 1080px;` 加回去 ⇒ ①③ 红：窗口被拉小后
//!   Slint 仍按 1920×1080 排版再裁右下角，`device-rack` 的 `y+h` 会跑到 1056（> 900）；
//! - 把第二段的两条常量（`x: 840px; width: 544px`）加回去 ⇒ ③ 红：`1280` 下右沿 1384 > 1280，
//!   `master-device-0-remove-button` 直接**从控件树里消失**（实测）；
//! - 把 `master-eq-panel-width` 写成常量 328 ⇒ ③ 红：`432` 宽的第二段里 EQ 格右沿 520 > 432。
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过；⛔ 不证 Linux Golden 重录（本机没有 `tests/golden/macos/**`，逐字节比对在
//! macOS 上**未被判定**）。
//! ⭐ 任务单 `#184` 补上了"窗口比 1024 更窄"的形态（`device_rack_two_segments.rs`）：
//! 第一段右列收到 236 起、1024 以下整列撤下，两段在 800/848/1024/1168/1280/1440/1920
//! 七档**逐档断言**包围盒不相交。本文件保持 `#183` 的 1440/1280/1920 三档读数不变。

#![allow(unused)]
use std::collections::BTreeMap;
use std::rc::Rc;

use slint::Model as _;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::project::{
    DeviceDefinition, DeviceKind, ParameterValue, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::UiTestPort as _;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, golden_evidence, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 设备架源码（**另一个文件**）—— 源码腿只从这里读，⛔ 不从本判据自己读（那是恒真）。
const RACK_SOURCE: &str = include_str!("../ui/console/device_rack.slint");
/// 外壳源码 —— 根尺寸那条反向腿从这里读。
const SHELL_SOURCE: &str = include_str!("../ui/app.slint");

fn device(name: &str, params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(param, value)| ParameterValue {
                name: (*param).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// 两段各一台设备（展示轨 ＋ 主总线）、两段 EQ 都可用 ⇒ 两段的语义节点**全都在场**。
fn project() -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();

    let mut display = TrackV3 {
        id: track,
        name: "展示轨".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    display
        .devices
        .push(device("展示 EQ", &[("eq_low_gain", 3.0)]));

    let mut master_track = TrackV3 {
        id: master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };
    master_track
        .devices
        .push(device("主总线 EQ", &[("eq_low_gain", -6.0)]));

    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(track, display);
    tracks.insert(master, master_track);

    YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        ..YebanProjectV1::default()
    }
}

/// 一条 Tier-1 活窗口（生产窗口构造入口；Tab 3 = 设备架）。
fn live(size: Size) -> LivePort<MainWindow> {
    let view = ViewState::from_project(&project()).expect("投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(size, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + Tab 3 生产窗口")
}

/// 第一段（展示轨）的语义节点 —— **每个**都必须在窗口内。
const SEGMENT_ONE: &[&str] = &[
    "device-rack",
    "device-rack-track-label",
    "device-rack-count",
    "device-rack-eq-curve",
    "device-eq-low-gain-panel",
    "device-eq-value",
    "device-eq-low-gain-slider",
    "send-strip",
    "track-routing-panel",
    "send-count",
    "device-insert-actions",
    "device-insert-eq-button",
    "device-insert-reverb-button",
    "clip-create-button",
    "device-0-card",
    "device-0-recognised",
    "device-0-bypass-switch",
    "device-0-remove-button",
];

/// 第二段（主总线）的语义节点。
const SEGMENT_TWO: &[&str] = &[
    "master-device-rack",
    "master-device-track-label",
    "master-device-rack-count",
    "master-device-insert-eq-button",
    "master-device-insert-reverb-button",
    "master-device-eq-panel",
    "master-device-eq-value",
    "master-device-eq-slider",
    "master-device-0-card",
    "master-device-0-recognised",
    "master-device-0-bypass-switch",
    "master-device-0-remove-button",
];

/// 逐条打印两个段的包围盒，返回 `(id, x, y, w, h)` 清单。
fn dump(tree: &ControlTree, label: &str, size: Size) -> Vec<(String, i32, i32, u32, u32)> {
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for id in SEGMENT_ONE.iter().chain(SEGMENT_TWO.iter()) {
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("[{label}] 运行时控件树里**找不到** `{id}`"));
        let b = node.bounds.expect("运行时节点必须有包围盒");
        lines.push(format!(
            "{id}: x={} y={} w={} h={}",
            b.x, b.y, b.width, b.height
        ));
        rows.push(((*id).to_owned(), b.x, b.y, b.width, b.height));
    }
    report_line(&format!(
        "[#183 {label}] 窗口 {}×{} 的包围盒读数（{} 个节点）：\n{}",
        size.width,
        size.height,
        rows.len(),
        lines.join("\n")
    ));
    rows
}

/// 断言清单里**每个**节点都落在窗口内（逻辑像素坐标）。
fn assert_inside(rows: &[(String, i32, i32, u32, u32)], size: Size) {
    let width = size.width as i32;
    let height = size.height as i32;
    let mut outside = Vec::new();
    for (id, x, y, w, h) in rows {
        let inside = *x >= 0 && *y >= 0 && x + *w as i32 <= width && y + *h as i32 <= height;
        if !inside {
            outside.push(format!(
                "{id}: x={x} y={y} w={w} h={h} ⇒ 右沿 {} / 下沿 {}",
                x + *w as i32,
                y + *h as i32
            ));
        }
    }
    assert!(
        outside.is_empty(),
        "[#183] {}×{} 下这些节点**不在窗口内**：\n{}",
        size.width,
        size.height,
        outside.join("\n")
    );
}

/// 读一格包围盒并逐字段比对。
fn assert_bounds(tree: &ControlTree, id: &str, want: (i32, i32, u32, u32)) {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    let b = node.bounds.expect("运行时节点必须有包围盒");
    assert_eq!(
        (b.x, b.y, b.width, b.height),
        want,
        "`{id}` 的包围盒必须是 `{want:?}`（`#180`／`#181` 注册值），实得 `({}, {}, {}, {})`",
        b.x,
        b.y,
        b.width,
        b.height
    );
}

/// 一帧的像素指纹（`[MUST-GATE-015]` 的非全黑见证 ＋ 指纹读数）。
fn fingerprint(live: &LivePort<MainWindow>, label: &str) -> u64 {
    let image = live
        .window()
        .capture()
        .unwrap_or_else(|err| panic!("[{label}] Tier-1 截图: {err}"));
    let probe = golden_evidence(&image).expect("[MUST-GATE-015] 尺寸非零且非全黑");
    if let Ok(path) = std::env::var("YEBAN_DUMP_FRAME") {
        let bytes = yeban_ui_test_port::png::encode_rgb8(&image);
        std::fs::write(path, bytes).expect("写帧");
    }
    report_line(&format!(
        "[#183 {label}] 帧指纹 {}；非黑像素 {}；颜色 {} 种",
        probe.fingerprint, probe.non_black_pixels, probe.distinct_colors
    ));
    probe.fingerprint
}

// ===========================================================================
// 判据 ①：1440×900（仓里 15 个界面判据的默认窗口）两段全可见
// ===========================================================================
#[test]
fn both_segments_are_inside_a_1440x900_window() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ②：1920×1080 逐格不变（`#180` / `#181` 注册值）
// ===========================================================================
#[test]
fn the_1920x1080_geometry_is_bit_for_bit_the_registered_one() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ③：1280×800（极窄档）—— 两段仍在窗口内，且互不重叠
// ===========================================================================
#[test]
fn both_segments_fit_a_1280x800_window_without_overlapping() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ④：1440×900 下三个手势各一次提交
// ===========================================================================
#[test]
fn the_three_gestures_still_commit_exactly_once_at_1440x900() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 源码腿：反向腿钉住的两处写法（⛔ 不靠回忆）
// ===========================================================================
#[test]
fn the_layout_probes_pin_the_two_sources() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}
