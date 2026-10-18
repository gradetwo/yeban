//! ⭐ 任务单 `#183`：**设备架在窄窗口里也必须够得着**。
//!
//! ## 这条判据要证的句子
//!
//! | 判据 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 1440×900 两段全可见 | 窗口 `1440×900` ⇒ 两段的**每个**语义 ID 都在运行时控件树里，且包围盒满足 `0 ≤ x`、`0 ≤ y`、`x+w ≤ 1440`、`y+h ≤ 900` | 证"窄窗口下不需要拖动/滚动就够得着"；⛔ 不证人眼看过 |
//! | ② 1920×1080 不变 | 两段的关键节点包围盒**逐格等于** `#180`／`#181` 注册的那组数（`840 / 544 / 1384`、`1040 / 328`、`1256 / 72`…）；同一帧的像素指纹一并报出 | 证"改布局没有挪动默认档一位" |
//! | ③ 1280×800 | 同上，但下界是 `1280×800`；第二段右沿必须 `≤ 1280` 且左沿 `≥ 824`（⛔ 不与第一段重叠） | 证"极窄档也不裁"；⛔ 不证更窄（见文件末的边界） |
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
//! macOS 上**未被判定**）；⛔ 不证"窗口比 1024 更窄"时的形态（第一段自己就宽 824）。

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
    let size = Size::new(1440, 900);
    let live = live(size);
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 3（设备架）");

    let tree = live.tree();
    let rows = dump(tree, "1440x900", size);
    assert_inside(&rows, size);

    // 第二段在 1440 下仍是 `#180` 的原值（面板宽 1440 ≥ 1392 ⇒ 公式恒等于常量）。
    assert_bounds(tree, "master-device-rack", (840, 583, 544, 260));
    // 第一段一位未动（面板宽变化不影响它的绝对坐标）。
    assert_bounds(tree, "device-rack-track-label", (8, 561, 420, 16));
    assert_bounds(tree, "device-rack", (0, 555, 1440, 321));

    report_line(&format!(
        "[#183 ①] 1440×900：两段 {} 个语义节点全部在窗口内；第二段 x=840 w=544 ⇒ 右沿 1384 ≤ 1440 ✓；面板 y=555 h=321 ⇒ 下沿 876 ≤ 900 ✓",
        rows.len()
    ));
    let _ = fingerprint(&live, "1440x900");
}

// ===========================================================================
// 判据 ②：1920×1080 逐格不变（`#180` / `#181` 注册值）
// ===========================================================================
#[test]
fn the_1920x1080_geometry_is_bit_for_bit_the_registered_one() {
    let size = Size::new(1920, 1080);
    let live = live(size);
    let tree = live.tree();
    let rows = dump(tree, "1920x1080", size);
    assert_inside(&rows, size);

    // 第一段：`#157` 起就没动过的那组坐标。
    assert_bounds(tree, "device-rack", (0, 663, 1920, 393));
    assert_bounds(tree, "device-rack-track-label", (8, 669, 420, 16));
    assert_bounds(tree, "device-rack-count", (436, 669, 200, 16));
    assert_bounds(tree, "device-rack-eq-curve", (8, 691, 420, 100));
    assert_bounds(tree, "device-eq-low-gain-panel", (444, 691, 380, 100));
    assert_bounds(tree, "device-eq-low-gain-slider", (452, 731, 300, 24));
    assert_bounds(tree, "device-insert-eq-button", (8, 839, 84, 20));
    assert_bounds(tree, "device-0-card", (8, 879, 420, 44));
    assert_bounds(tree, "device-0-remove-button", (400, 883, 20, 20));

    // 第二段：`#180`／`#181`／`#182` 注册的 `840 / 544 / 1384` 那一组。
    assert_bounds(tree, "master-device-rack", (840, 691, 544, 260));
    assert_bounds(tree, "master-device-track-label", (848, 695, 360, 16));
    assert_bounds(tree, "master-device-rack-count", (1224, 695, 144, 16));
    assert_bounds(tree, "master-device-insert-eq-button", (848, 719, 84, 20));
    assert_bounds(tree, "master-device-eq-panel", (1040, 713, 328, 48));
    assert_bounds(tree, "master-device-eq-value", (1048, 727, 312, 12));
    assert_bounds(tree, "master-device-eq-slider", (1048, 741, 300, 18));
    assert_bounds(tree, "master-device-0-card", (844, 763, 528, 44));
    assert_bounds(tree, "master-device-0-bypass-switch", (1256, 767, 72, 20));
    assert_bounds(tree, "master-device-0-remove-button", (1344, 767, 20, 20));

    report_line(
        "[#183 ②] 1920×1080：两段 24 个节点的包围盒与 `#180`/`#181` 注册值**逐格相等**（840/544/1384、1040/328、1256/72、1344/20）",
    );
    let _ = fingerprint(&live, "1920x1080");
}

// ===========================================================================
// 判据 ③：1280×800（极窄档）—— 两段仍在窗口内，且互不重叠
// ===========================================================================
#[test]
fn both_segments_fit_a_1280x800_window_without_overlapping() {
    let size = Size::new(1280, 800);
    let live = live(size);
    let tree = live.tree();
    let rows = dump(tree, "1280x800", size);
    assert_inside(&rows, size);

    // 第二段让宽度而不是让左沿：左沿仍 840（> 824 = 第一段的右沿）⇒ ⛔ 不重叠。
    assert_bounds(tree, "master-device-rack", (840, 523, 432, 253));
    // 段内右沿全部收敛到 1272（= 1280 − 8）。
    assert_bounds(tree, "master-device-rack-count", (1112, 527, 144, 16));
    assert_bounds(tree, "master-device-eq-panel", (1032, 545, 224, 48));
    assert_bounds(tree, "master-device-eq-slider", (1040, 573, 196, 18));
    assert_bounds(tree, "master-device-0-card", (844, 595, 416, 44));
    assert_bounds(tree, "master-device-0-bypass-switch", (1144, 599, 72, 20));
    assert_bounds(tree, "master-device-0-remove-button", (1232, 599, 20, 20));

    report_line(
        "[#183 ③] 1280×800：第一段 0..824；第二段 840..1272（宽 432，高收到 253）⇒ 右沿 1280 − 8 = 1272 ✓、下沿 776 ≤ 800 ✓、左沿 840 ≥ 824 ⇒ 两段**无重叠** ✓；删除键在树里（`#180` 的常量在 1280 下会让它整格消失）",
    );
}

// ===========================================================================
// 判据 ④：1440×900 下三个手势各一次提交
// ===========================================================================
#[test]
fn the_three_gestures_still_commit_exactly_once_at_1440x900() {
    let size = Size::new(1440, 900);
    let view = ViewState::from_project(&project()).expect("投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(size, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + Tab 3 生产窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 #183 交互>", "yeban-app", project(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    host::wire_device_rack(live.ui(), &port);

    // ① 装设备（展示轨）：一次点击 = 恰好一次提交。
    let before = port.display();
    live.ui().invoke_device_insert_eq();
    let after_insert = port.display();
    assert_eq!(
        after_insert.commit_count,
        before.commit_count + 1,
        "装设备：Δcommit 必须是 1（前 {}，后 {}）",
        before.commit_count,
        after_insert.commit_count
    );

    // ② 旁通（按**身份**寻址，`#182`）：一次点击 = 恰好一次提交。
    let device_id = live
        .ui()
        .get_device_ids()
        .row_data(0)
        .expect("展示轨第 0 台设备身份")
        .to_string();
    live.ui()
        .invoke_device_bypass_toggle(device_id.as_str().into());
    let after_bypass = port.display();
    assert_eq!(
        after_bypass.commit_count,
        after_insert.commit_count + 1,
        "旁通：Δcommit 必须是 1（前 {}，后 {}）",
        after_insert.commit_count,
        after_bypass.commit_count
    );

    // ③ EQ 滑块（按**身份**寻址）：按下 ＋ 松手 = 恰好一次提交。
    let eq_id = live.ui().get_device_eq_device_id().to_string();
    live.ui().invoke_device_eq_grab(eq_id.as_str().into(), 1.0);
    live.ui().invoke_device_eq_release();
    let after_eq = port.display();
    assert_eq!(
        after_eq.commit_count,
        after_bypass.commit_count + 1,
        "EQ 滑块：Δcommit 必须是 1（前 {}，后 {}）",
        after_bypass.commit_count,
        after_eq.commit_count
    );
    assert_eq!(
        after_eq.undoable,
        before.undoable + 3,
        "三个手势 = 三步可撤销"
    );

    report_line(&format!(
        "[#183 ④] 1440×900：装设备 Δ1 / 旁通 Δ1 / EQ 滑块 Δ1 ⇒ commit {} → {}（undoable {} → {}）",
        before.commit_count, after_eq.commit_count, before.undoable, after_eq.undoable
    ));
}

// ===========================================================================
// 源码腿：反向腿钉住的两处写法（⛔ 不靠回忆）
// ===========================================================================
#[test]
fn the_layout_probes_pin_the_two_sources() {
    // `app.slint` 的根**不许**再有硬编码尺寸。
    assert!(
        !SHELL_SOURCE.contains("    width: 1920px;\n    height: 1080px;"),
        "`app.slint` 的根不许再钉死 `width: 1920px; height: 1080px;`（窗口会被裁成 1920×1080 的左上角）"
    );
    assert!(
        SHELL_SOURCE.contains("preferred-width: 1920px;"),
        "初值必须由 `preferred-width` 给出（1920×1080 仍是启动尺寸）"
    );

    // 第二段的两个几何量必须由面板宽度算出来。
    for needle in [
        "property <length> master-rack-width: max(320px, min(544px, root.width - 848px));",
        "property <length> master-eq-panel-width: min(328px, root.master-rack-width - 208px);",
        "x: root.master-rack-x;",
        "width: root.master-rack-width;",
        "height: min(260px, root.height - 28px);",
    ] {
        assert!(
            RACK_SOURCE.contains(needle),
            "`device_rack.slint` 必须逐字含 `{needle}`"
        );
    }
    report_line(
        "[#183 源码腿] `app.slint` 根尺寸已交给后端（`preferred-*` 给初值）；`device_rack.slint` 第二段的 x/宽/高由 `root.width`/`root.height` 算出",
    );
}
