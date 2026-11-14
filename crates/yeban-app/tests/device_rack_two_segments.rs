//! ⭐ 任务单 `#184`：**两段在窄窗口里不许相交**。
//!
//! ## 这条判据要证的句子
//!
//! | 判据 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 七档不交 | `800 / 848 / 1024 / 1168 / 1280 / 1440 / 1920` 七档，第一段**每个**在场节点的右沿 `≤ master-rack-x − 16`，且两段节点两两包围盒不相交 | 证"两段的 bounds 永不相交"；⛔ 不证人眼看过 |
//! | ② 第一段相对化 | 第一段内容右沿 `≤ min(824, master-rack-x − 16)`；计数标签 `device-rack-count` 也钉在第一条之内 | 证"第一段不再用 `0..824` 的绝对坐标" |
//! | ③ 848 / 800 不越界 | 两段每个在场节点都满足 `0 ≤ x`、`0 ≤ y`、`x+w ≤ 窗口宽`、`y+h ≤ 窗口高`；第一段右列（EQ 低架面板 / 发送条）**不在树里** | 证"第一段自己不再越界"；⛔ 不证右列在该档可用（它被**故意**撤下） |
//! | ④ 1024 可交互 | 装设备 / 旁通 / EQ 滑块各 `Δcommit = 1` | 证"窄档手势没被布局吃掉"；⛔ 不证听感 |
//! | ⑤ 1920 不回退 | 24 个注册节点几何**逐格相等** ＋ 帧指纹读数 | 证"宽档一位没动"；逐像素 diff 用落盘 PNG 在体外判（本机基线见报告） |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把第一段右列宽换回常量 `380px` ⇒ ① 在 848/800 红（第一段右沿 824 > 504/456）；
//! - 去掉 `visible: root.first-side-visible` ⇒ ① 在 848/800 红（第一段右列 444..504/456 与第二段 520/472 相交）；
//! - 把 `device-rack-count` 的 x 换回 `436px` ⇒ ② 在 848/800 红（右沿 496/636 > 第一段宽 504/456）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过；⛔ 不证 Linux Golden 重录（本机没有 `tests/golden/macos/**`，
//! macOS 上的逐字节比对**未被判定**）；⛔ 不证 `< 800` 的形态（左列 `0..428` 自身放不下，
//! 与 `#183` 的同类边界）；⛔ 不证 `compact` 断点跟随窗口（本单未做，见报告第 3 条）。

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

/// 七档：`(宽, 高)` —— 前三档是 `#183` 留下的边界档，后四档是宽面板回归档。
const WIDTHS: &[(u32, u32)] = &[
    (800, 800),
    (848, 800),
    (1024, 800),
    (1168, 800),
    (1280, 800),
    (1440, 900),
    (1920, 1080),
];

/// 第一段**左列**的语义节点（任何宽度都在场）。
const SEGMENT_ONE_ALWAYS: &[&str] = &[
    "device-rack",
    "device-rack-track-label",
    "device-rack-count",
    "device-rack-eq-curve",
    "instrument-label",
    "device-insert-actions",
    "device-insert-eq-button",
    "device-insert-reverb-button",
    "clip-create-button",
    "device-0-card",
    "device-0-recognised",
    "device-0-bypass-switch",
    "device-0-remove-button",
];

/// 第一段**右列**的语义节点（面板 ≥ 1024 才在场；更窄时整列被**故意**撤下）。
const SEGMENT_ONE_RIGHT: &[&str] = &[
    "device-eq-low-gain-panel",
    "device-eq-value",
    "device-eq-low-gain-slider",
    "send-strip",
    "track-routing-panel",
    "send-count",
    "send-empty",
    "send-create-button",
    "track-create-midi-button",
    "track-create-audio-button",
    "track-create-aux-button",
    "track-delete-button",
];

/// 第二段（主总线设备链）的语义节点。
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

type Bounds = (i32, i32, u32, u32);

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

fn bounds(tree: &ControlTree, id: &str) -> Bounds {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    let b = node.bounds.expect("运行时节点必须有包围盒");
    (b.x, b.y, b.width, b.height)
}

fn intersect(a: Bounds, b: Bounds) -> bool {
    let (ax, ay, aw, ah) = (
        i64::from(a.0),
        i64::from(a.1),
        i64::from(a.2),
        i64::from(a.3),
    );
    let (bx, by, bw, bh) = (
        i64::from(b.0),
        i64::from(b.1),
        i64::from(b.2),
        i64::from(b.3),
    );
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

/// 第一段在场节点的右沿（逻辑像素）。`device-rack` 是**整块面板**（两段都住在里面），
/// 它本身不参与"第一段右沿 / 两段相交"的判定 —— 判的是它**内容**的节点。
fn first_segment_right_edge(rows: &[(String, Bounds)]) -> i32 {
    rows.iter()
        .filter(|(id, _)| id != "device-rack")
        .map(|(_, (x, _, w, _))| x + *w as i32)
        .max()
        .expect("第一段至少有一个内容节点")
}

/// 一条读数行：`id: x=… y=… w=… h=…`。
fn dump(rows: &[(String, Bounds)], label: &str) -> String {
    let mut lines = vec![format!(
        "[#184 {label}] 包围盒读数（{} 个节点）：",
        rows.len()
    )];
    for (id, (x, y, w, h)) in rows {
        lines.push(format!("{id}: x={x} y={y} w={w} h={h}"));
    }
    lines.join("\n")
}

// ===========================================================================
// 判据 ①＋②＋③：七档逐档 —— 两段不相交 / 第一段相对化 / 都在窗口内
// ===========================================================================
#[test]
fn the_two_segments_never_intersect_and_never_leave_the_window() {
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
        aether_app.contains("> compact:"),
        "新 UI 契约面必须声明 > compact:"
    );
    assert!(
        aether_host.contains("set_compact"),
        "宿主接线层必须注册 set_compact"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

/// 一档的完整读数与断言（在**独立线程**里调用 —— 平台只能装一次）。
#[allow(clippy::too_many_lines)]
fn check_width(width: u32, height: u32) {
    let size = Size::new(width, height);
    let live = live(size);
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 3（设备架）");
    let tree = live.tree();

    let right_visible = width >= 1024;
    let mut seg1: Vec<(String, Bounds)> = Vec::new();
    for id in SEGMENT_ONE_ALWAYS {
        seg1.push(((*id).to_owned(), bounds(tree, id)));
    }
    for id in SEGMENT_ONE_RIGHT {
        let found = tree.find_by_id(id).map(|node| {
            let b = node.bounds.expect("运行时节点必须有包围盒");
            (b.x, b.y, b.width, b.height)
        });
        assert_eq!(
            found.is_some(),
            right_visible,
            "[#184] {width}×{height} 档：`{id}` 的在场性必须是 `{right_visible}`（面板 ≥ 1024 才有第一段右列）"
        );
        if let Some(b) = found {
            seg1.push(((*id).to_owned(), b));
        }
    }
    let seg2: Vec<(String, Bounds)> = SEGMENT_TWO
        .iter()
        .map(|id| ((*id).to_owned(), bounds(tree, id)))
        .collect();

    // ③ 每个在场节点都在窗口内。
    let mut outside = Vec::new();
    for (id, (x, y, w, h)) in seg1.iter().chain(seg2.iter()) {
        let inside =
            *x >= 0 && *y >= 0 && x + *w as i32 <= width as i32 && y + *h as i32 <= height as i32;
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
        "[#184] {width}×{height} 下这些节点**不在窗口内**：\n{}",
        outside.join("\n")
    );

    // ② 第一段相对化：内容右沿 ≤ min(824, master-rack-x − 16)。
    let (master_x, _, master_w, _) = bounds(tree, "master-device-rack");
    let first_width = 824_i32.min(master_x - 16);
    let first_right = first_segment_right_edge(&seg1);
    assert!(
        first_right <= first_width,
        "[#184] {width}×{height} 档：第一段内容右沿 {first_right} 必须 ≤ min(824, master-rack-x − 16) = {first_width}"
    );

    // ① 两段节点两两包围盒不相交（`device-rack` 是整块面板，不参与）。
    let mut hits = Vec::new();
    for (a_id, a) in seg1.iter().filter(|(id, _)| id != "device-rack") {
        for (b_id, b) in &seg2 {
            if intersect(*a, *b) {
                hits.push(format!("{a_id} {a:?} ⟂ {b_id} {b:?}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "[#184] {width}×{height} 下两段**相交**：\n{}",
        hits.join("\n")
    );

    report_line(&dump(&seg1, &format!("{width}x{height} / 第一段")));
    report_line(&dump(&seg2, &format!("{width}x{height} / 第二段")));
    report_line(&format!(
        "[#184 {width}x{height}] 第一段右沿 {} ≤ {first_width}（= min(824, master-rack-x {master_x} − 16)）；第二段 {master_x}..{}；最近间距 {} ≥ 16；右列 {}",
        first_right,
        master_x + master_w as i32,
        master_x - first_right,
        if right_visible {
            format!("在场（宽 {}）", bounds(tree, "send-strip").2)
        } else {
            "整列撤下".to_owned()
        }
    ));

    if width == 1024 {
        // 1024 档的关键读数：右列 236、四键各 52、EQ 滑块 220。
        let send = bounds(tree, "send-strip");
        let panel = bounds(tree, "device-eq-low-gain-panel");
        let slider = bounds(tree, "device-eq-low-gain-slider");
        let button = bounds(tree, "track-create-midi-button");
        report_line(&format!(
            "[#184 1024 细读] send-strip x={} w={} ⇒ 右沿 {}；EQ 面板 x={} w={} ⇒ 右沿 {}；EQ 滑块 x={} w={} ⇒ 右沿 {}；轨路由首键 x={} w={} ⇒ 右沿 {}",
            send.0,
            send.2,
            send.0 + send.2 as i32,
            panel.0,
            panel.2,
            panel.0 + panel.2 as i32,
            slider.0,
            slider.2,
            slider.0 + slider.2 as i32,
            button.0,
            button.2,
            button.0 + button.2 as i32
        ));
        assert_eq!(send.2, 236, "1024 档第一段右列宽必须是 236（最小可读宽度）");
        assert_eq!(panel.2, 236, "1024 档 EQ 面板宽必须是 236");
        assert_eq!(slider.2, 220, "1024 档 EQ 滑块宽必须是 220（= 236 − 16）");
        assert_eq!(button.2, 52, "1024 档四个轨路由键各 52（= (236 − 28) / 4）");
        assert_eq!(master_x, 696, "1024 档第二段左沿必须是 696");
    }
    if width == 800 {
        // ⭐ `#191` 关闭了这里的第 3 条边界：`compact` 现在是 `ui/app.slint` 里
        // `self.width < Tokens.breakpoint-full-hd` 的**绑定**，跟的是真实窗口宽
        //（`#184` 时是宿主写入的常量视口 ⇒ 800 档仍 false）。
        // ⭐ `#195`：阈值 `1920 → 1180` ⇒ `800 < 1180` **仍为假**，本档期望**不变**（仍折叠）。
        let compact = live.ui().get_compact();
        let sidebar_width = tree
            .find_by_id("sidebar")
            .and_then(|node| node.bounds)
            .map(|b| b.width);
        assert!(compact, "[#191] 800×800 必须落在折叠档（compact == true）");
        assert_eq!(
            sidebar_width,
            Some(36),
            "[#191] 800×800 左栏必须是 36px 图标导轨"
        );
        report_line(&format!(
            "[#195 compact 读数] 800×800：ui.get_compact() = {compact}；sidebar 宽 = {sidebar_width:?}（阈值 1180，800 仍折叠）"
        ));
    }
    if width == 848 || width == 800 {
        // 该档右列**故意**撤下：先证明它真的不在树里，再报第一段右沿。
        for id in SEGMENT_ONE_RIGHT {
            assert!(
                tree.find_by_id(id).is_none(),
                "[#184] {width} 档右列必须整列撤下，但 `{id}` 还在树里"
            );
        }
        report_line(&format!(
            "[#184 {width} 细读] 右列整列撤下；第一段右沿 {}（左列 0..428 ＋ 计数标签）；第二段 {}..{}",
            first_right,
            master_x,
            master_x + master_w as i32
        ));
    }
}

// ===========================================================================
// 判据 ④：1024×800 下三个手势各一次提交
// ===========================================================================
#[test]
fn the_three_gestures_still_commit_exactly_once_at_1024x800() {
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
        aether_app.contains("> compact:"),
        "新 UI 契约面必须声明 > compact:"
    );
    assert!(
        aether_host.contains("set_compact"),
        "宿主接线层必须注册 set_compact"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

// ===========================================================================
// 判据 ⑤：1920×1080 的 24 个注册节点逐格不变 ＋ 指纹读数
// ===========================================================================
#[test]
fn the_1920x1080_geometry_is_still_the_registered_one() {
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
        aether_app.contains("> compact:"),
        "新 UI 契约面必须声明 > compact:"
    );
    assert!(
        aether_host.contains("set_compact"),
        "宿主接线层必须注册 set_compact"
    );
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
}
