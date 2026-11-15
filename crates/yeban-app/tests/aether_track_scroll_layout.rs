//! 多轨工程的编排区**布局**判据（报告反馈：轨道一多，下方钢琴卷帘整块消失）。
//!
//! 旧实现把轨道行放进固定高度的 `VerticalLayout`（每行 `min-height: 64px`）：
//! 16 轨就会把工作区撑出窗口，底部控制台被顶到窗口外。本判据钉住三件事：
//!
//! 1. 行几何 / 内容高来自投影（`track-ys` / `track-heights` / `arrangement-sheet-height`）；
//! 2. 轨道行在自己的 `Flickable` 里滚动 ⇒ `console-tab-rail` 仍在窗口内；
//! 3. 横向 / 纵向缩放都能从可见控件生效（`zoom-h-step` / `zoom-v-step`），
//!    滚轮分流（alt = 纵向、shift = 横向）也接到同一条宿主入口。

use std::rc::Rc;

use slint::{LogicalPosition, Model, platform::WindowEvent};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Size, UiTestPort};

const NOW_MS: u64 = 1_700_000_000_000;

fn sixteen_track_project() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    let template = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .cloned()
        .expect("夹具必须有非主总线轨");
    let mut next = 0;
    while project.tracks.len() < 17 {
        let mut track = template.clone();
        track.id = EntityId::new();
        track.name = format!("Scroll Track {next:02}");
        track.clips.clear();
        project.tracks.insert(track.id, track);
        next += 1;
    }
    project.track_order = project
        .tracks
        .keys()
        .copied()
        .filter(|id| *id != project.master_bus_track_id)
        .collect();
    project
}

#[test]
fn many_tracks_scroll_inside_the_arrangement_and_keep_the_lower_dock_visible() {
    let project = sixteen_track_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-scroll>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));

    let mut live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_zoom_controls(ui, &undo);
    host::wire_arrangement_wheel(ui, &undo);

    // ① 行几何与内容高来自投影：16 条轨、内容高于窗口（否则本判据是空绿）。
    assert_eq!(ui.get_track_ys().row_count(), view.tracks.len());
    assert_eq!(ui.get_track_heights().row_count(), view.tracks.len());
    let sheet = ui.get_arrangement_sheet_height();
    // 默认行高 56px × 16 = 896px，而工作区只剩约 500px（窗口 900 − 下坞 320）⇒
    // 内容高必须明显超出工作区；这里用 700px 作下界（不写死 56，避免行高改动即红）。
    assert!(sheet > 700.0, "16 轨的内容高必须撑出可视区（实测 {sheet}）");
    eprintln!(
        "[layout] tracks={} sheet={sheet}px window=900px",
        view.tracks.len()
    );

    // ③ 纵向缩放：V+ 让首行变高（行高不再是 stretch 的均分）。
    let height_before = ui.get_track_heights().row_data(0).expect("首行几何");
    ui.invoke_zoom_v_step(true);
    let height_after = ui.get_track_heights().row_data(0).expect("首行几何");
    assert!(
        height_after > height_before,
        "V+ 必须把行高从 {height_before} 放大，实测 {height_after}"
    );
    eprintln!("[zoom/v] row height {height_before} -> {height_after}");

    // ④ 横向缩放：H+ 让每像素 tick 变小（内容变宽）。
    let tpp_before = ui.get_roll_ticks_per_pixel();
    ui.invoke_zoom_h_step(true);
    let tpp_after = ui.get_roll_ticks_per_pixel();
    assert!(
        tpp_after < tpp_before,
        "H+ 必须把 ticks-per-pixel 从 {tpp_before} 变小，实测 {tpp_after}"
    );
    eprintln!("[zoom/h] ticks-per-pixel {tpp_before} -> {tpp_after}");

    // ⑤ 滚轮分流：alt = 纵向放大、shift = 横向滚动（与按钮同一条宿主入口）。
    let height_before = ui.get_track_heights().row_data(0).expect("首行几何");
    ui.invoke_arrangement_wheel(0.0, 120.0, false, true);
    let height_after = ui.get_track_heights().row_data(0).expect("首行几何");
    assert!(
        height_after > height_before,
        "alt+滚轮必须纵向放大：{height_before} -> {height_after}"
    );
    let scroll_before = ui.get_roll_scroll_x();
    ui.invoke_arrangement_wheel(0.0, -160.0, true, false);
    let scroll_after = ui.get_roll_scroll_x();
    assert!(
        scroll_after > scroll_before,
        "shift+滚轮必须横向滚动：{scroll_before} -> {scroll_after}（内容宽 {}）",
        ui.get_arrangement_content_px()
    );
    eprintln!(
        "[wheel] row {height_before}->{height_after}, scroll-x {scroll_before}->{scroll_after}"
    );

    // ② 下方控制台仍在窗口内（修复前它会被轨道行顶出 900px）。
    let tree = live.refresh_tree(None).expect("重抓控件树");
    let rail = tree
        .iter()
        .find(|node| node.id == "console-tab-rail")
        .expect("底部控制台的 tab rail 必须在控件树里");
    let bounds = rail.bounds.expect("活窗口节点必须有几何");
    assert!(
        bounds.height > 0
            && bounds.y >= 0
            && bounds.y + i32::try_from(bounds.height).unwrap() <= 901,
        "底部控制台必须留在窗口内（实测 y={} height={}）",
        bounds.y,
        bounds.height
    );
    eprintln!(
        "[layout] console-tab-rail y={} height={} (window 900)",
        bounds.y, bounds.height
    );
}

#[test]
fn arrangement_supports_vertical_and_horizontal_trackpad_two_finger_scroll() {
    let project = sixteen_track_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-2d-scroll>", "yeban-app", project, NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let mut live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("主窗口装配成功");
    {
        let ui = live.ui();
        host::wire_arrangement_wheel(ui, &undo);
        host::wire_track_select(ui, &undo);
        host::apply_view(ui, &view, 1440.0, 0.0);
    }

    live.refresh_tree(None).expect("刷新控件树");
    let arr_bounds = live
        .tree()
        .find_by_id("workspace-arrangement-canvas")
        .expect("workspace-arrangement-canvas 必须在树里")
        .bounds
        .expect("arrangement 必须有几何");
    let lane_pt = LogicalPosition::new(arr_bounds.x as f32 + 420.0, arr_bounds.y as f32 + 120.0);
    let header_pt = LogicalPosition::new(arr_bounds.x as f32 + 96.0, arr_bounds.y as f32 + 120.0);

    // 1. 无修饰 macOS 双指左右滑动（delta_x != 0, shift = false）在时间轴与轨道头区域均生效
    assert_eq!(live.ui().get_roll_scroll_x(), 0.0);
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: lane_pt,
        delta_x: -96.0,
        delta_y: 0.0,
    });
    assert!(
        (live.ui().get_roll_scroll_x() - 96.0).abs() < 0.5,
        "无修饰双指右滑 delta_x=-96 应推进 roll_scroll_x 至 96，实际={}",
        live.ui().get_roll_scroll_x()
    );

    live.window().dispatch(WindowEvent::PointerScrolled {
        position: header_pt,
        delta_x: 48.0,
        delta_y: 0.0,
    });
    assert!(
        (live.ui().get_roll_scroll_x() - 48.0).abs() < 0.5,
        "轨道头区域双指左滑 delta_x=+48 应回退 roll_scroll_x 至 48，实际={}",
        live.ui().get_roll_scroll_x()
    );

    live.window().dispatch(WindowEvent::PointerScrolled {
        position: lane_pt,
        delta_x: 200.0,
        delta_y: 0.0,
    });
    assert_eq!(live.ui().get_roll_scroll_x(), 0.0, "向左滑动必须夹紧在 0");

    // 2. 纯纵向滚轮 / 双指上下滑动使轨道行内容区域发生像素位移（上移显示下方轨道），而底坞不变
    let rail_before = live
        .tree()
        .find_by_id("console-tab-rail")
        .unwrap()
        .bounds
        .unwrap()
        .y;
    let frame_before = live.window().capture().expect("纵向滚动前抓帧");
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: header_pt,
        delta_x: 0.0,
        delta_y: -160.0,
    });
    let frame_after_v = live.window().capture().expect("纵向滚动后抓帧");
    let mut vertical_diffs = 0_usize;
    let scan_y0 = (arr_bounds.y + 72).max(0) as u32;
    let scan_y1 = (arr_bounds.y + i32::try_from(arr_bounds.height).unwrap() - 12).max(0) as u32;
    let scan_x0 = (arr_bounds.x + 8).max(0) as u32;
    let scan_x1 = (arr_bounds.x + 200).max(0) as u32;
    for py in scan_y0..scan_y1 {
        for px in scan_x0..scan_x1 {
            if frame_before.pixel(px, py) != frame_after_v.pixel(px, py) {
                vertical_diffs += 1;
            }
        }
    }
    assert!(
        vertical_diffs > 200,
        "轨道头纵向双指滑动后内容像素必须发生实际位移，差异像素={vertical_diffs}"
    );
    live.refresh_tree(None).expect("刷新树");
    let rail_after = live
        .tree()
        .find_by_id("console-tab-rail")
        .unwrap()
        .bounds
        .unwrap()
        .y;
    assert_eq!(rail_before, rail_after, "轨道内部滚动不得挤动下坞位置");

    // 3. 对角线双指手势（delta_x 与 delta_y 同非零）：同时推进横向与纵向
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: lane_pt,
        delta_x: -60.0,
        delta_y: -120.0,
    });
    let frame_after_diag = live.window().capture().expect("对角滑动后抓帧");
    assert!(
        (live.ui().get_roll_scroll_x() - 60.0).abs() < 0.5,
        "对角双指滑动必须同步推进横向 roll_scroll_x，实际={}",
        live.ui().get_roll_scroll_x()
    );
    let mut diag_header_diffs = 0_usize;
    for py in scan_y0..scan_y1 {
        for px in scan_x0..scan_x1 {
            if frame_after_v.pixel(px, py) != frame_after_diag.pixel(px, py) {
                diag_header_diffs += 1;
            }
        }
    }
    assert!(
        diag_header_diffs > 200,
        "对角双指滑动必须同时更新纵向 content-y，轨道头差异像素={diag_header_diffs}"
    );
}
