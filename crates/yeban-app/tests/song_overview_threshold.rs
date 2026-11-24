//! T-FIX-372：活窗口尺寸断点判据 —— `song-overview-collapsed` 必须跟**窗口真尺寸**
//! 走（UI-GRID-002 全展开档 ≥1920×1080 时展开），而不是跟布局缓存格高走。
//! 背景：`self.height` 在此组件里被 Slint 编译为根布局缓存 1 号格的高度
//! （= 窗口高 − 40px），旧绑定在 1920×1080 时读到 1040px < 1080px，
//! 于是任何宽度下都翻不过来；现高度臂读 `key-handler.absolute-position.y
//! + key-handler.height`（= 窗口真高）。每步都打读数：窗口尺寸、scale、collapsed。

use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};

/// 生产窗口的唯一构造入口（`run_gui` 用的同一条；与
/// `tests/transport_overflow_follows_window_width.rs` 同款）。
fn build(size: Size) -> LivePort<MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

fn read(port: &LivePort<MainWindow>, label: &str) -> bool {
    port.window().capture().expect("必须能重绘一帧");
    let ui = port.ui();
    let size = port.window().size();
    let collapsed = ui.get_song_overview_collapsed();
    report_line(&format!(
        "[T-FIX-372 {label}] 窗口 {}×{} scale={} collapsed={}",
        size.width,
        size.height,
        ui.window().scale_factor(),
        collapsed
    ));
    collapsed
}

#[test]
fn born_at_1920_is_expanded() {
    let port = build(Size::new(1920, 1080));
    assert!(!read(&port, "出生 1920"), "1920×1080 下总览面板必须展开");
}

#[test]
fn resize_round_trip_1920_2560_1920() {
    let port = build(Size::new(1920, 1080));
    assert!(!read(&port, "扫档 1920"), "1920×1080 下总览面板必须展开");
    port.window()
        .resize(Size::new(2560, 1440))
        .expect("resize 到 2560");
    assert!(!read(&port, "扫档 2560"), "2560×1440 下总览面板必须展开");
    port.window()
        .resize(Size::new(1920, 1080))
        .expect("resize 回 1920");
    assert!(
        !read(&port, "扫档回 1920"),
        "resize 回 1920×1080 后总览面板必须展开"
    );
}

#[test]
fn below_threshold_stays_collapsed() {
    let port = build(Size::new(1280, 720));
    assert!(read(&port, "出生 1280"), "1280×720 下总览面板必须折叠");
    port.window()
        .resize(Size::new(1919, 1079))
        .expect("resize 到 1919");
    assert!(read(&port, "扫档 1919"), "1919×1079 下总览面板必须折叠");
}

/// 单臂边界：只差高（1920×1079）或只差宽（1919×1080）时仍须折叠，
/// 证明宽、高两臂各自独立生效，而不是其中一臂恒真/恒假。
#[test]
fn single_arm_boundary_stays_collapsed() {
    let port = build(Size::new(1920, 1079));
    assert!(
        read(&port, "只差高 1920×1079"),
        "1920×1079 下总览面板必须折叠"
    );
    port.window()
        .resize(Size::new(1919, 1080))
        .expect("resize 到 1919×1080");
    assert!(
        read(&port, "只差宽 1919×1080"),
        "1919×1080 下总览面板必须折叠"
    );
}
