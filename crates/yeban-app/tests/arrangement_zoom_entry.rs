//! T-FIX-887：编曲区缩放入口在控件树里可见、标签写明快捷键/滚轮修饰键，
//! 且点击后 `roll-ticks-per-pixel` 真的变化。
//!
//! 红对照：删掉 arrangement.slint 里四个缩放键的 `accessible-id`（或
//! elements.rs 的登记）⇒ 第一条即红；断开 `zoom-h-step` 接线 ⇒ 第二条红。
#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;
use live::{LiveWiringOptions, build_live_ui_with};
use std::rc::Rc;
use yeban_ui_test_port::port::Permission;

fn setup() -> live::LiveUi {
    let project = yeban_model::samples::filled_project();
    let undo = Rc::new(yeban_app::undo::UndoPort::new(
        yeban_app::undo::UndoSession::open(
            "<zoom-entry>",
            "yeban-app",
            project.clone(),
            1_700_000_000_000,
        )
        .expect("undo session"),
    ));
    build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(undo),
            ..LiveWiringOptions::default()
        },
    )
    .expect("live ui")
}

#[test]
fn zoom_entries_are_visible_in_the_control_tree_with_shortcut_labels() {
    let mut ui = setup();
    let tree = ui.tree_snapshot();
    for id in [
        "arrangement-zoom-h-out-button",
        "arrangement-zoom-h-in-button",
        "arrangement-zoom-v-out-button",
        "arrangement-zoom-v-in-button",
    ] {
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("控件树必须含缩放入口 `{id}`"));
        let label = node.label.as_str();
        // 语言前提：标签用当前源码里的中文词条；换语言时需要同步更新本判据。
        if id.contains("-h-") {
            assert!(
                label.contains("Shift+Alt+滚轮"),
                "横向缩放标签必须写明滚轮修饰键: {label}"
            );
        } else {
            assert!(
                label.contains("Alt+滚轮"),
                "纵向缩放标签必须写明滚轮修饰键: {label}"
            );
        }
        assert!(node.bounds.is_some(), "缩放入口必须有几何: {id}");
    }
}

#[test]
fn clicking_the_visible_zoom_button_changes_horizontal_zoom() {
    let mut ui = setup();
    let before = ui.ui().get_roll_ticks_per_pixel();
    // 走 Slint 生成的 GUI 回调（与按钮 `zoom-h-step(true)` 同一条入口），
    // ⛔ 不手写 op、不直接改属性。
    ui.ui().invoke_zoom_h_step(true);
    let after = ui.ui().get_roll_ticks_per_pixel();
    assert!(
        after < before,
        "点 H+ 必须真的放大时间轴: {before} -> {after}"
    );
    ui.ui().invoke_zoom_h_step(false);
    let restored = ui.ui().get_roll_ticks_per_pixel();
    assert_eq!(
        restored, before,
        "H− 必须回到原缩放: {before} -> {restored}"
    );
}
