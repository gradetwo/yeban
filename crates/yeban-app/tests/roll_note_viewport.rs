//! T-FIX-920：写音符不得带动排列横向视口；明确的“回开头”动作必须把视口一起带回 001。
//!
//! 两条判据都走**真实指针**（`piano-roll-scroll-area` 的 MCP 指针注入），不是直接调宿主回调：
//! ① 铅笔在 tick 14640 写一颗音符 ⇒ scroll / 标尺起点不变，音符 tick 与可见范围可核对；
//! ② 用户把视口滚到第 6 小节后按停止（= rewind 的产品落点）⇒ scroll 与标尺一起回 0。
//!
//! 选择路径另由 `clip_selection_viewport.rs` 钉住：选中可见片段不滚、选中屏外片段才最小滚动。
//! 本文件只断言“写音符”与“回开头”两条，避免把选片段的定位纪律混进来。

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, LiveWiringOptions, build_live_ui_with};
use serde_json::json;
use slint::ComponentHandle;
use yeban_app::bridge::{ViewState, host_to_painted};
use yeban_app::host;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;

const SCROLL_AREA: &str = "piano-roll-scroll-area";
const NOW_MS: u64 = 1_760_000_000_000;
const VIEWPORT_PX: f32 = 1200.0;
/// tick 14640 = 第 4 小节第 3 拍又 720 tick（004.03.720，与上报里的落点一致）。
const CLICK_TICK_BAR4: u64 = 14_640;
/// 由 tick 反算宿主帧 x：tick / ticks-per-pixel（默认 30）。
const CLICK_HOST_X: f32 = 488.0;
const CLICK_HOST_Y: f32 = 100.0;

fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("filled_project 可投影");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<roll-note-viewport>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&undo)),
            ..Default::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    let window = ui.ui().clone_strong();

    // 选一条有 MIDI 片段的轨，并把该片段设为“当前编辑片段”——这是用户画音符前最常见的状态。
    let lead = project
        .tracks
        .values()
        .find(|t| {
            t.kind == yeban_model::project::TrackKind::Midi && t.id != project.master_bus_track_id
        })
        .map(|t| t.id.to_canonical_string())
        .expect("filled_project 有 MIDI 轨");
    window.set_selected_track_id(lead.clone().into());
    let lead_index = view
        .tracks
        .iter()
        .position(|t| t.id == lead)
        .expect("投影含同一轨");
    let visible_clip = view
        .clips
        .iter()
        .find(|c| c.track_index == lead_index)
        .map(|c| c.clip_id.clone())
        .expect("该轨有片段");
    window.set_selected_clip_ulid(visible_clip.into());
    window.set_active_tool(2); // Pencil
    window.set_active_tool_name("draw".into());
    window.set_arrangement_timeline_px(VIEWPORT_PX);
    host::apply_view(&window, &view, VIEWPORT_PX, 0.0);

    let plane = ui.into_control_plane(Permission::Interactive);
    (window, undo, plane)
}

fn element_origin(plane: &mut LiveControlPlane) -> (f32, f32) {
    let node = plane.plane().try_line(&request_line(
        1,
        METHOD_NODE,
        Some(json!({ "elementId": SCROLL_AREA })),
    ));
    assert!(
        !node.is_error(),
        "{} 必须在运行时树里: {node:?}",
        SCROLL_AREA
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    (
        bounds["x"].as_f64().expect("x") as f32,
        bounds["y"].as_f64().expect("y") as f32,
    )
}

fn click_pencil_at(plane: &mut LiveControlPlane, host_x: f32, host_y: f32) {
    let (origin_x, origin_y) = element_origin(plane);
    let (painted_x, painted_y) = host_to_painted(host_x, host_y);
    let down = plane.plane().try_line(&request_line(
        2,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": f64::from(painted_x),
            "yOffset": f64::from(painted_y),
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    let moved = plane.plane().try_line(&request_line(
        3,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({
            "x": f64::from(origin_x + painted_x),
            "y": f64::from(origin_y + painted_y)
        })),
    ));
    assert!(!moved.is_error(), "移动必须落到真实窗口: {moved:?}");
    let up = plane.plane().try_line(&request_line(
        4,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

fn midi_notes(project: &yeban_model::YebanProjectV1) -> Vec<(u64, u8)> {
    project
        .clip_pool
        .values()
        .flat_map(|entry| match &entry.content {
            yeban_model::project::ClipContent::Midi { notes } => notes
                .values()
                .map(|note| (note.start_tick, note.pitch))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn pencil_click_at_known_tick_keeps_the_arrangement_viewport() {
    let (window, undo, mut plane) = assemble();
    let before_notes = midi_notes(&undo.try_project().expect("权威工程"));
    let before_scroll = window.get_roll_scroll_x();
    let before_bar = window.get_bar_start_index();
    assert_eq!(before_scroll, 0.0, "装配后视口在开头");
    assert_eq!(before_bar, 0, "装配后标尺从第 1 小节起");

    click_pencil_at(&mut plane, CLICK_HOST_X, CLICK_HOST_Y);

    let after_notes = midi_notes(&undo.try_project().expect("权威工程"));
    assert_eq!(
        after_notes.len(),
        before_notes.len() + 1,
        "一次铅笔点击必须真的写入一颗音符（否则后面的视口断言是空话）"
    );
    let inserted = after_notes
        .iter()
        .find(|(tick, _)| *tick == CLICK_TICK_BAR4)
        .copied()
        .expect("新音符必须落在点击的 tick 14640");
    assert!(!before_notes.is_empty(), "夹具工程必须先有音符");
    assert!(
        before_notes.iter().all(|note| *note != inserted),
        "新音符是新身份"
    );

    assert_eq!(
        window.get_roll_scroll_x(),
        0.0,
        "写音符不得改变排列横向视口"
    );
    assert_eq!(window.get_bar_start_index(), 0, "标尺起点必须保持第 1 小节");
    let min_tick = window.get_roll_min_tick();
    let max_tick = window.get_roll_max_tick();
    assert!(
        i64::from(min_tick) <= i64::try_from(CLICK_TICK_BAR4).unwrap()
            && i64::try_from(CLICK_TICK_BAR4).unwrap() < i64::from(max_tick),
        "新音符 tick 必须仍在可见范围内: [{min_tick}, {max_tick})"
    );
}

#[test]
fn stop_returns_the_horizontal_viewport_to_the_first_bar() {
    let (window, _undo, _plane) = assemble();
    let view = ViewState::from_project(&_undo.try_project().expect("权威工程")).expect("投影");
    host::apply_scroll_view(&window, &view, VIEWPORT_PX, 800.0);
    assert_eq!(window.get_roll_scroll_x(), 800.0, "先把视口滚到第 6 小节");
    assert_eq!(window.get_bar_start_index(), 6, "标尺起点跟着滚动");

    // 停止按钮是产品里 rewind 的落点（`wire_aether_actions` 的 transport-rewind → invoke_stop）。
    window.invoke_stop();

    assert_eq!(
        window.get_roll_scroll_x(),
        0.0,
        "停止必须把横向视口带回开头"
    );
    assert_eq!(window.get_bar_start_index(), 0, "标尺必须回到第 1 小节");
}
