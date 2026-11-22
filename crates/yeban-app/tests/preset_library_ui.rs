//! T-FIX-77 原生预制库面板的 Tier-1 活窗口判据：
//! 面板真的进控件树、真的光栅化出非全黑像素，行模型与旁通态由宿主写。
//!
//! 本测试只碰界面副作用，不写用户库；错误原文与 App 语义由
//! `yeban_mcp::domain::preset` 的既有单测钉住。

use slint::{Model as _, ModelRc, VecModel};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::PresetRowData;
use yeban_ui_test_port::render::{LivePort, golden_evidence};
use yeban_ui_test_port::{Permission, Size};

#[test]
fn preset_library_panel_lists_rows_and_renders() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");

    let ui = live.ui();
    ui.set_preset_rows(ModelRc::new(VecModel::from(vec![
        PresetRowData {
            id: "factory:test:system".into(),
            name: "Test System Preset".into(),
            detail: "test.sfz · 2 个宏 · 1 条发送".into(),
            kind: "system".into(),
            instrument: "test".into(),
        },
        PresetRowData {
            id: "user:test:user".into(),
            name: "Test User Preset".into(),
            detail: "test.sfz · 0 个宏 · 0 条发送".into(),
            kind: "user".into(),
            instrument: "test".into(),
        },
    ])));
    ui.set_preset_current_track("当前轨：测试轨 · user:test:user".into());
    ui.set_preset_has_current(true);
    ui.set_preset_current_bypassed(false);
    ui.set_preset_library_open(true);

    let image = live.window().capture().expect("面板必须能抓帧");
    let evidence = golden_evidence(&image).expect("面板帧必须非全黑");
    let png = yeban_ui_test_port::png::encode_rgb8_limited(&image, 8 * 1024 * 1024)
        .expect("面板帧必须能编码 PNG");
    let path = std::env::temp_dir().join("preset-library-tfix77.png");
    std::fs::write(&path, &png).expect("面板帧必须能落盘");

    assert!(ui.get_preset_library_open());
    assert_eq!(ui.get_preset_rows().row_count(), 2);
    assert!(evidence.non_black_pixels > 1000, "{evidence:?}");
    assert!(evidence.distinct_colors > 20, "{evidence:?}");
    eprintln!("{}", evidence.summary());
    eprintln!("PNG: {}", path.display());
}
