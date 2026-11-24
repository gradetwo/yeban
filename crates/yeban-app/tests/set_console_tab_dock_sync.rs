//! T-FIX-379 定性判据：**`set_console_tab` 的同步要等窗口更新一圈**。
//!
//! ## 结论（无头 harness 假象，⛔ 不是跑着的应用里的缺陷）
//!
//! 建窗后调 `set_console_tab(1)` 只改 `console-tab` 的读数：`Theme.active-dock-tab`
//! 仍是 `"pianoroll"`，运行时树一位不动 —— `changed console-tab` 那条到
//! `Theme.active-dock-tab` 的同步**没有跑**。再渲染一帧（`capture()`，即跑一次窗口
//! 更新 pass），`Theme.active-dock-tab` 立刻变 `"mixer"`，树随之切换到混音台
//! （与 `invoke_apply_dock_tab("mixer")` 的终态逐项一致）。`request_redraw()`
//! 光标记脏是不够的；`apply-dock-tab` 之所以在 harness 里立刻生效，是因为它
//! 直接写 `Theme.active-dock-tab`，根本不走 `changed`  handler。
//!
//! 跑着的应用里事件循环每圈都跑更新 pass（Slint 在更新 pass 里交付 `changed`
//! handler 并重画脏窗口），所以 F7–F9 / X / P / 布局恢复这几条生产路径在真机上
//! 都能同步 —— 它们缺的只是 harness 里从来不跑的那一圈。本文件钉住的正是这一条：
//! `set` 本身不同步，**一帧之后同步**；若将来某天 `set` 后立刻就同步了（引擎改了
//! 交付时机），② 的断言会红 —— 那是信号，⛔ 是误报。
//!
//! ## 证据（2026-11-25 探针，yanshi-5，基线 8b58d19b，两次复现一致）
//!
//! 活窗口（`filled_project`，Tab 0）：树 61 节点，`console-tab` 0，
//! `Theme.active-dock-tab` `"pianoroll"`；`set_console_tab(1)` 后读数 1，
//! `Theme` 仍 `"pianoroll"`，树仍 61；`request_redraw()` 后仍 61；
//! 渲染一帧后 `Theme` `"mixer"`、树 61→40；对照
//! `invoke_apply_dock_tab("mixer")` 同样 40 节点、`"mixer"`。
//! （`asset-pool-panel` 在本基线未入座 —— 它是 T-FIX-356 Group B 的生产改动，
//! 不在本基线上；本文件不钉它，只钉 tab 切换本身。）
//!
//! ## 数值口径
//!
//! 61/40 是当前活树的节点数（面板构成变了会跟着变）；断言只钉**关系**（
//! `set` 后树长不变、`Theme` 不变；一帧后 `Theme` 变、树变短、与对照终态同长），
//! 不钉绝对数 —— 绝对数只写进报告行，供回执引用。

use slint::ComponentHandle as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

const WINDOW: Size = Size::new(1440, 900);

fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

fn live_window(view: &ViewState, console_tab: i32) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::ReadOnly, None, || {
        host::build_main_window_with_console_tab(view, &scene, console_tab)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 读一帧快照：`console-tab` 读数、`Theme.active-dock-tab`、运行时树节点数。
fn probe(port: &LivePort<MainWindow>, tag: &str) -> (i32, String, usize) {
    let tab = port.ui().get_console_tab();
    let dock = port
        .ui()
        .global::<yeban_app::ui::Theme<'_>>()
        .get_active_dock_tab()
        .to_string();
    let n = runtime_tree(port.ui()).len();
    report_line(&format!(
        "[379] {tag}: console-tab={tab} Theme.active-dock-tab={dock} tree={n}"
    ));
    (tab, dock, n)
}

#[test]
fn set_console_tab_syncs_after_one_frame_not_immediately() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = live_window(&view, 0);

    // ① 基线：Tab 0（钢琴卷帘）。
    let (tab0, dock0, n0) = probe(&port, "baseline tab0");
    assert_eq!(tab0, 0, "基线 console-tab 必须是 0");
    assert_eq!(dock0, "pianoroll", "基线 active-dock-tab 必须是 pianoroll");

    // ② `set_console_tab(1)`：读数变了，同步没跑 —— 树一位不动。
    port.ui().set_console_tab(1);
    let (tab1, dock1, n1) = probe(&port, "after set_console_tab(1)");
    assert_eq!(tab1, 1, "set_console_tab(1) 后读数必须是 1");
    assert_eq!(
        dock1, "pianoroll",
        "只调 set_console_tab 时 changed handler 还没跑：Theme 仍是 pianoroll"
    );
    assert_eq!(
        n1, n0,
        "只调 set_console_tab 时 dock 不得切换：树节点数必须仍是 {n0}，实际 {n1}"
    );

    // ③ 光标记脏不够。
    port.window().request_redraw();
    let (_t, dock1b, n1b) = probe(&port, "after request_redraw");
    assert_eq!(
        dock1b, "pianoroll",
        "request_redraw 不跑更新 pass，Theme 仍是 pianoroll"
    );
    assert_eq!(n1b, n0, "request_redraw 后树仍不得切换");

    // ④ 渲染一帧（跑一次窗口更新 pass）：同步发生，dock 切换。
    let captured = port.window().capture();
    assert!(
        captured.is_ok(),
        "capture 必须能渲染一帧: {:?}",
        captured.err()
    );
    let (tab2, dock2, n2) = probe(&port, "after capture (1 frame)");
    assert_eq!(tab2, 1, "渲染一帧后 console-tab 仍是 1");
    assert_eq!(
        dock2, "mixer",
        "渲染一帧后 changed handler 必须已跑：Theme 必须是 mixer"
    );
    assert!(
        n2 < n0,
        "渲染一帧后 dock 必须切换到混音台：树节点数必须小于基线 {n0}，实际 {n2}"
    );

    // ⑤ 对照：生产 X/P 路（`invoke_apply_dock_tab` 直接写 Theme）终态一致。
    port.ui().set_console_tab(0);
    let _ = port.window().capture();
    let (tab0b, dock0b, _) = probe(&port, "back to tab0");
    assert_eq!(tab0b, 0);
    assert_eq!(dock0b, "pianoroll", "回到 Tab 0 后 Theme 必须是 pianoroll");

    port.ui().invoke_apply_dock_tab("mixer".into());
    let (tab3, dock3, n3) = probe(&port, "after invoke_apply_dock_tab(mixer)");
    assert_eq!(tab3, 1, "apply-dock-tab(mixer) 后 console-tab 必须是 1");
    assert_eq!(
        dock3, "mixer",
        "apply-dock-tab(mixer) 后 Theme 必须是 mixer"
    );
    assert_eq!(
        n3, n2,
        "两条路的终态树必须一致：set+一帧 {n2} vs apply-dock-tab {n3}"
    );
}
