//! 离屏取证：宏面板（含**级联去向**）在活窗口里长什么样（审计 `T-FIX-88` 项 1）。
//!
//! 只出图，不断言像素（判据在 `macro_panel_mappings.rs`）。产物：
//! `target/ui-test-port/shot-macro-panel-1x.png` / `-3x.png`。
//!
//! 为什么必须出图：这一格是"宏 → 各去向"的可读性，机械断言只能证明字符串对不对，
//! 证明不了它有没有被**画出来**、有没有压到滑块、有没有在 3 倍字号下糊成一条。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, write_artifact};

/// 抓一张 1400×900（1x）或更大（3x 等效放大）的宏面板帧。
fn shot(name: &str, width: u32, height: u32) {
    shot_with_panel(name, width, height, true);
}

fn shot_with_panel(name: &str, width: u32, height: u32, open: bool) {
    let project = yeban_model::samples::filled_project();
    // 展示轨必须是**有宏的那条**：`ViewState::from_project` 的兜底轨是身份序第一条
    // 非主总线轨，规范样本里带宏的是 lead ⇒ 显式按身份选中它。
    let selected = project
        .tracks
        .values()
        .find(|track| !track.macros.is_empty())
        .map(|track| track.id.to_string());
    let view = ViewState::from_project_with_zoom_cursor_layout_and_selection(
        &project,
        30,
        0,
        &yeban_app::bridge::TrackHeightLayout::default(),
        selected.as_deref(),
    )
    .expect("规范样本必须能投影");
    assert!(
        !view.macro_names().is_empty(),
        "取证前提：展示轨必须带宏，否则面板画的是空态"
    );
    assert!(
        view.macro_mapping_summaries()
            .iter()
            .any(|summary| !summary.is_empty()),
        "取证前提：至少要有一条非空级联去向，否则证明不了那一行画得出来"
    );

    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(
        Size::new(width, height),
        Permission::Interactive,
        None,
        || host::build_main_window_with_console_tab(&view, &scene, 0),
    )
    .expect("Tier-1 平台");
    host::apply_view(live.ui(), &view, width as f32, 0.0);
    live.ui().set_macro_panel_open(open);

    let frame = live.window().capture().expect("抓帧");
    write_artifact(name, &frame).expect("写截图工件");
}

#[test]
fn shot_macro_panel_1x() {
    shot("shot-macro-panel-1x", 1400, 900);
}

/// 关着面板的那一帧：只用来核对**入口按钮**在标签栏里的落位（不遮挡）。
#[test]
fn shot_macro_panel_entry_1x() {
    shot_with_panel("shot-macro-entry-1x", 1400, 900, false);
}

#[test]
fn shot_macro_panel_3x() {
    shot("shot-macro-panel-3x", 1800, 1150);
}
