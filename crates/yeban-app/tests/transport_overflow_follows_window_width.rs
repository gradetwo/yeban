//! ⭐ **顶栏窄窗分档跟着真实窗口宽走**（Wang 2026-11-17 裁决 F1/F2；落法参考
//! Logic Pro 控制栏：位置与速度恒显、次要读数进溢出区、再窄收辅助入口）。
//!
//! 上一轮的死路（git 历史 95e1c16）：判据接的是 Rust 心跳单向注入的 `window-w`，
//! 实测注入没有到达 `TransportHeaderBar`，紧凑分支全灭。本轮改走与 `compact`
//! 同款的形态：四个布尔在 `MainWindow` 上绑 **Window 自身宽度**（平台给定、
//! 不经布局回授），经属性传进走带栏。本判据只走生产构造入口
//! [`host::build_main_window`]，逐档读回布尔 + 看 ⋯ 溢出钮在控件树里的在场性：
//!
//! | 窗口宽 | overflow | hide-brand | hide-tips | hide-cmd | ⋯ 钮 |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | 1920 | false（< 1880 不成立） | false | false | false | 不在场 |
//! | 1600 | true | false（≥ 1540） | false | false | 在场 |
//! | 1445 | true | true（< 1540） | true（< 1500） | true（< 1460） | 在场 |
//!
//! 同一实例真 resize 扫 1920 → 1600 → 1445 → 1920，两次翻向都测（⛔ 不是
//! 一次性的初值巧合）。⛔ 不证：真 winit 窗口被人类用手拖动（无头 Tier-1 平台；
//! `resize` 与 OS 侧 resize 在 `WindowAdapter` 上是同一条事件）。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 生产窗口的唯一构造入口（`run_gui` 用的同一条）。
fn build(size: Size) -> LivePort<MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 一次布局后的完整读数：(overflow, hide-brand, hide-tips, hide-cmd, ⋯ 钮在场)。
fn observe(port: &LivePort<MainWindow>, label: &str) -> (bool, bool, bool, bool, bool) {
    port.window().capture().expect("必须能重绘一帧");
    let ui = port.ui();
    let tree = runtime_tree(ui);
    let got = (
        ui.get_transport_overflow(),
        ui.get_transport_hide_brand(),
        ui.get_transport_hide_tips(),
        ui.get_transport_hide_cmd(),
        tree.contains("transport-overflow-button"),
    );
    let window = port.window().size();
    report_line(&format!(
        "[F1/F2 {label}] 窗口 {}×{} ⇒ overflow={} hide-brand={} hide-tips={} \
         hide-cmd={} ⋯钮在场={}",
        window.width, window.height, got.0, got.1, got.2, got.3, got.4
    ));
    got
}

fn assert_tier(
    label: &str,
    got: (bool, bool, bool, bool, bool),
    want: (bool, bool, bool, bool, bool),
) {
    assert_eq!(
        got, want,
        "[F1/F2 {label}] 分档读数必须 {want:?}（阈值：overflow < 1880、\
         hide-brand < 1540、hide-tips < 1500、hide-cmd < 1460）"
    );
}

#[test]
fn windows_born_at_1920_and_1445_start_in_their_own_tier() {
    let wide = build(Size::new(1920, 1080));
    assert_tier(
        "出生 1920",
        observe(&wide, "出生 1920"),
        (false, false, false, false, false),
    );

    let narrow = build(Size::new(1445, 700));
    assert_tier(
        "出生 1445",
        observe(&narrow, "出生 1445"),
        (true, true, true, true, true),
    );
}

#[test]
fn tiers_follow_real_resizes_in_both_directions() {
    let port = build(Size::new(1920, 1080));
    assert_tier(
        "扫档 1920",
        observe(&port, "扫档 1920"),
        (false, false, false, false, false),
    );

    port.window().resize(Size::new(1600, 900)).expect("resize 到 1600");
    assert_tier(
        "扫档 1600",
        observe(&port, "扫档 1600"),
        (true, false, false, false, true),
    );

    port.window().resize(Size::new(1445, 700)).expect("resize 到 1445");
    assert_tier(
        "扫档 1445",
        observe(&port, "扫档 1445"),
        (true, true, true, true, true),
    );

    port.window().resize(Size::new(1920, 1080)).expect("resize 回 1920");
    assert_tier(
        "扫档回 1920",
        observe(&port, "扫档回 1920"),
        (false, false, false, false, false),
    );
}
