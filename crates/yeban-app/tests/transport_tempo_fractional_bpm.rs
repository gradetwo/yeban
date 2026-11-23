//! T-FIX-161 live UI 判据：走带 TEMPO 读数必须保留小数 BPM。
//!
//! 直接读 LivePort 抓到的 `transport-tempo-value` 无障碍标签 —— 它绑在
//! `.slint` 的真实 `Text` 上，不是只断言 Rust 侧的 `bpm-display` 字符串。
//!
//! Slint 的软件平台一进程只能初始化一次，所以这里用一个窗口依次验证两个值：
//! 124.5 由 `ViewState` 经 `apply_view` 注入；124.05 通过根属性重设后刷新控件树。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

fn build_rig(view: &ViewState) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    let view_for_window = view.clone();
    LivePort::new(
        Size::new(1280, 800),
        Permission::Interactive,
        None,
        move || host::build_main_window(&view_for_window, &scene),
    )
    .expect("LivePort 必须成功装配 MainWindow")
}

fn tempo_label(port: &mut LivePort<MainWindow>) -> String {
    let tree = port.refresh_tree(None).expect("必须能抓 TEMPO 读数控件树");
    tree.find_by_id("transport-tempo-value")
        .expect("TEMPO 读数必须声明 transport-tempo-value")
        .label
        .clone()
}

#[test]
fn transport_tempo_value_keeps_fractional_bpm() {
    let mut view = ViewState::demo();
    view.bpm = 124.5;
    view.bpm_display = "124.50".into();
    view.bpm_millis = 124_500;

    let mut port = build_rig(&view);
    assert_eq!(
        tempo_label(&mut port),
        "124.50",
        "工程 BPM 124.5 的 TEMPO 读数必须保留为 124.50"
    );

    port.ui().set_bpm_display("124.05".into());
    assert_eq!(
        tempo_label(&mut port),
        "124.05",
        "工程 BPM 124.05 的 TEMPO 读数必须保留为 124.05"
    );
}
