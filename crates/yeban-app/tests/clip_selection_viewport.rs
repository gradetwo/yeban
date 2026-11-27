//! T-FIX-973：选中已在视口内的片段不得自动横向滚动。
//!
//! 夹具在 `filled_project` 的 Lead 轨上放三条同 clip_pool 条目的摆放：
//! A `0..3840`、B `3840..7680`、C `60000..63840`。默认 30 tick/px、视口 1200px
//! ⇒ A/B 都完整可见，C 在视口右侧之外。
//!
//! 判据：
//! 1. 点 B（走生产 `on_clip_select`）⇒ `roll-scroll-x` 一位不动，标尺起点不变，
//!    A 仍完整可见；
//! 2. 点屏外 C ⇒ 只滚到「刚好看得见 C」的最小位置（右沿贴视口右沿），
//!    并有可见定位回执。

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::project::{ClipPlacement, TrackKind, YebanProjectV1};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size};

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;
const VIEWPORT_PX: f32 = 1200.0;

fn lead_track_id(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::Midi && !track.clips.is_empty())
        .map(|track| track.id)
        .expect("filled_project 必须有带 MIDI 片段的展示轨")
}

/// 在 Lead 轨上再放一条同 clip_pool 条目的摆放；返回新摆放身份。
fn add_placement(
    project: &mut YebanProjectV1,
    source: &ClipPlacement,
    start_tick: u64,
) -> EntityId {
    let track_id = lead_track_id(project);
    let id = EntityId::new();
    let mut placement = *source;
    placement.id = id;
    placement.start_tick = start_tick;
    project
        .tracks
        .get_mut(&track_id)
        .expect("刚查到的轨")
        .clips
        .insert(id, placement);
    id
}

struct Fixture {
    project: YebanProjectV1,
    a: EntityId,
    b: EntityId,
    c: EntityId,
}

fn fixture() -> Fixture {
    let mut project = yeban_model::samples::filled_project();
    let lead_id = lead_track_id(&project);
    let source = *project
        .tracks
        .get(&lead_id)
        .expect("lead")
        .clips
        .values()
        .next()
        .expect("lead 必须有摆放");
    let a = source.id;
    let b = add_placement(&mut project, &source, 3840);
    let c = add_placement(&mut project, &source, 60_000);
    Fixture { project, a, b, c }
}

fn live(fixture: &Fixture) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(&fixture.project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<T-FIX-973>", "yeban-app", fixture.project.clone(), NOW_MS)
            .expect("撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口");
    // 与生产同一套选中接线；有端口 ⇒ `roll_focus_on_clip` 真的会跑。
    host::wire_view_callbacks(live.ui());
    host::wire_clip_scope(
        live.ui(),
        Rc::new(RefCell::new(view.clone())),
        Some(Rc::clone(&port)),
    );
    (live, port)
}

fn clip_range_px(ui: &MainWindow, project: &YebanProjectV1, id: EntityId) -> (f32, f32) {
    let placement = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .find(|placement| placement.id == id)
        .expect("夹具摆放必须在工程里");
    let tpp = host::roll_ticks_per_pixel(ui);
    let start = yeban_app::bridge::tick_to_px(placement.start_tick, tpp).expect("start px");
    let end = yeban_app::bridge::tick_to_px(
        placement
            .start_tick
            .saturating_add(placement.duration_ticks),
        tpp,
    )
    .expect("end px");
    (start as f32, end as f32)
}

fn assert_fully_visible(ui: &MainWindow, (start, end): (f32, f32), label: &str) {
    let scroll = ui.get_roll_scroll_x();
    let viewport = ui.get_arrangement_timeline_px();
    assert!(
        start >= scroll && end <= scroll + viewport,
        "{label} 必须完整落在视口 [{scroll}, {}] 内，实际 [{start}, {end}]",
        scroll + viewport
    );
}

#[test]
fn selecting_a_visible_clip_does_not_scroll_the_arrangement() {
    let fixture = fixture();
    let (live, _port) = live(&fixture);
    let ui = live.ui();
    ui.set_arrangement_timeline_px(VIEWPORT_PX);
    ui.set_roll_scroll_x(0.0);
    host::apply_view(
        ui,
        &ViewState::from_project(&fixture.project).expect("投影"),
        VIEWPORT_PX,
        0.0,
    );
    let before_scroll = ui.get_roll_scroll_x();
    let before_bar = ui.get_bar_start_index();
    let a_range = clip_range_px(ui, &fixture.project, fixture.a);
    let b_range = clip_range_px(ui, &fixture.project, fixture.b);
    assert_fully_visible(ui, a_range, "A");
    assert_fully_visible(ui, b_range, "B");

    ui.invoke_clip_select(fixture.b.to_canonical_string().into());
    assert_eq!(
        ui.get_selected_clip_ulid().as_str(),
        fixture.b.to_canonical_string(),
        "单击必须选中 B"
    );
    assert_eq!(
        ui.get_roll_scroll_x(),
        before_scroll,
        "B 已在视口内 ⇒ 选中不得改变横向偏移"
    );
    assert_eq!(
        ui.get_bar_start_index(),
        before_bar,
        "B 已在视口内 ⇒ 标尺起点不得跳"
    );
    assert_fully_visible(ui, a_range, "A（选中 B 后）");
    assert_fully_visible(ui, b_range, "B（选中后）");
}

#[test]
fn selecting_an_offscreen_clip_scrolls_the_minimum_distance_and_reports_it() {
    let fixture = fixture();
    let (live, _port) = live(&fixture);
    let ui = live.ui();
    ui.set_arrangement_timeline_px(VIEWPORT_PX);
    ui.set_roll_scroll_x(0.0);
    host::apply_view(
        ui,
        &ViewState::from_project(&fixture.project).expect("投影"),
        VIEWPORT_PX,
        0.0,
    );
    let c_range = clip_range_px(ui, &fixture.project, fixture.c);
    let (c_start, c_end) = c_range;
    assert!(
        c_start > VIEWPORT_PX,
        "夹具前提：C 必须在视口右侧之外，实际起点 {c_start}"
    );
    let content = ui.get_arrangement_content_px();
    let max_scroll = (content - VIEWPORT_PX).max(0.0);
    let expected = (c_end + 16.0 - VIEWPORT_PX).clamp(0.0, max_scroll);

    ui.invoke_clip_select(fixture.c.to_canonical_string().into());
    let scroll = ui.get_roll_scroll_x();
    assert!(
        (scroll - expected).abs() <= 1.0,
        "屏外 C 必须滚到最小可见位置 {expected}，实际 {scroll}"
    );
    assert_fully_visible(ui, c_range, "C");
    assert!(
        ui.get_module_status_succeeded() && ui.get_module_status_text().contains("定位"),
        "屏外定位必须有可见回执，实际「{}」",
        ui.get_module_status_text()
    );
}
