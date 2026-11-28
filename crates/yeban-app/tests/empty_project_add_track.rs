// T-FIX-923：启动空工程主入口「添加新轨道」必须建真实轨道。
//
// 夹具 `host::new_project_archive().project` 与启动选择器「新建空工程」逐字段同形
// （只有主总线）。装配用 `build_live_ui_with` + 撤销端口，与活窗口 / MCP 执行面
// 同一条 `wire_track_strip` 建轨链；该执行面**不跑** `wire_aether_actions`，
// 正是缺陷「界面 1 条通道 / 工程 0 条轨道」的现场。
//
// 本文件不断言 UI 文案，只断言工程轨数 / 排列行 / 通道条三者一致，
// 因此不依赖 LC_ALL（无语言前提需要钉）。
#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle as _;
use slint::Model as _;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_700_000_000_000;

fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>) {
    let project = yeban_app::host::new_project_archive().project;
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<empty-project-add-track>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    (ui.ui().clone_strong(), port)
}

fn counts(ui: &yeban_app::ui::MainWindow, port: &UndoPort) -> (usize, usize, usize) {
    let project = port.try_project().expect("权威工程必须在场");
    // 工程轨数 = 全部轨（含主总线）；界面通道条 / 排列行只列非主总线展示轨。
    let tracks = project.tracks.len();
    let strips = slint::Model::row_count(&ui.get_tracks());
    let rows = slint::Model::row_count(&ui.get_track_names());
    (tracks, strips, rows)
}

/// 主入口：点击「添加新轨道」后，工程轨数 +1，通道条 +1，排列行 +1。
#[test]
fn main_entry_adds_a_real_track_with_a_matching_strip_and_row() {
    let (ui, port) = assemble();
    let (tracks0, strips0, rows0) = counts(&ui, &port);
    assert_eq!(tracks0, 1, "启动空工程形状：只有主总线");

    ui.invoke_track_add_requested();

    let (tracks1, strips1, rows1) = counts(&ui, &port);
    assert_eq!(
        tracks1,
        tracks0 + 1,
        "主入口必须提交真实 AddTrack（工程轨 +1）"
    );
    assert_eq!(strips1, strips0 + 1, "通道条与新增轨道一一对应");
    assert_eq!(rows1, rows0 + 1, "排列区轨道名行数 +1");
}

/// 轨道菜单路径作为对照：仍走同一个建轨入口，三者同样 +1。
#[test]
fn track_menu_path_still_creates_one_track_strip_row() {
    let (ui, port) = assemble();
    let (tracks0, strips0, rows0) = counts(&ui, &port);

    ui.invoke_track_create_midi();

    let (tracks1, strips1, rows1) = counts(&ui, &port);
    assert_eq!(tracks1, tracks0 + 1, "菜单路径工程轨 +1");
    assert_eq!(strips1, strips0 + 1, "菜单路径通道条 +1");
    assert_eq!(rows1, rows0 + 1, "菜单路径排列行 +1");
}

/// 连续两次主入口：两次真实建轨，不出现「UI 两条 / 工程零条」的分叉。
#[test]
fn two_consecutive_main_entry_clicks_keep_all_three_views_in_sync() {
    let (ui, port) = assemble();
    let (tracks0, strips0, rows0) = counts(&ui, &port);
    ui.invoke_track_add_requested();
    ui.invoke_track_add_requested();
    let (tracks1, strips1, rows1) = counts(&ui, &port);
    assert_eq!(tracks1, tracks0 + 2);
    assert_eq!(strips1, strips0 + 2);
    assert_eq!(rows1, rows0 + 2);
}
