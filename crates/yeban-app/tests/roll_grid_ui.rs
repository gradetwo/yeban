//! T-FIX-579：卷帘网格选择器的**真实回调**（无头活窗口）。
//!
//! 只钉视图态与派生 tick：界面报下标 → 宿主唯一落点写属性 → 下一个编辑动作
//! 再按同一份换算读网格。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_model::samples::filled_project;

fn window() -> yeban_app::ui::MainWindow {
    let project = filled_project();
    let ui = build_live_ui_with(&project, &LiveWiringOptions::default())
        .expect("真实界面 + 默认接线（含 wire_roll_grid）");
    slint::ComponentHandle::clone_strong(ui.ui())
}

/// 缺省仍是 1/16 · 直 · 240 tick（零回归口径）。
#[test]
fn default_grid_is_sixteenth_straight_240_ticks() {
    let ui = window();
    assert_eq!(ui.get_roll_grid_note(), 4);
    assert_eq!(ui.get_roll_grid_mode(), 0);
    assert_eq!(ui.get_roll_grid_ticks(), 240);
}

/// 点 1/4 / 附点 / 三连音：下标与派生 tick 必须同步变化。
#[test]
fn grid_callbacks_write_the_view_state_and_derived_ticks() {
    let ui = window();

    ui.invoke_roll_grid_note_picked(2); // 1/4
    assert_eq!(ui.get_roll_grid_note(), 2);
    assert_eq!(ui.get_roll_grid_ticks(), 960, "1/4 = 960 tick");

    ui.invoke_roll_grid_mode_picked(1); // 附点
    assert_eq!(ui.get_roll_grid_mode(), 1);
    assert_eq!(ui.get_roll_grid_ticks(), 1440, "1/4 附点 = 1440 tick");

    ui.invoke_roll_grid_mode_picked(2); // 三连音
    assert_eq!(ui.get_roll_grid_mode(), 2);
    assert_eq!(ui.get_roll_grid_ticks(), 640, "1/4 三连音 = 640 tick");

    ui.invoke_roll_grid_note_picked(4); // 1/16
    assert_eq!(ui.get_roll_grid_ticks(), 160, "1/16 三连音 = 160 tick");

    ui.invoke_roll_grid_mode_picked(0); // 均分
    assert_eq!(ui.get_roll_grid_ticks(), 240, "回到 1/16 均分 = 240 tick");
}

/// 越界下标不写视图态（⛔ 不把非法值塞进吸附算术）。
#[test]
fn out_of_range_grid_indices_are_ignored() {
    let ui = window();
    ui.invoke_roll_grid_note_picked(7);
    ui.invoke_roll_grid_mode_picked(3);
    assert_eq!(ui.get_roll_grid_note(), 4);
    assert_eq!(ui.get_roll_grid_mode(), 0);
    assert_eq!(ui.get_roll_grid_ticks(), 240);
}
