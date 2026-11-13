//! 重构 UI（Aether）的功能接线判据。
//!
//! 新 UI 的 `MainWindow` 与 main 版接线层通过"契约兼容面"对接：数据数组
//! (`tracks` / `track-names` / `notes` …) 两版同名，`apply_view` 写入后组件树直接
//! 消费；少数不同名的显示态由 [`yeban_app::host::sync_aether_display`] 同步。
//! 本文件钉住这两条，以及"新 UI 的控件回调不是静默的"。
//!
//! 真实 `.yeban` 工程（17 轨 / 2867 音符 / 10 段落）的容器导入另由
//! `yeban-app --headless --open <file>` 的批处理出口验证（不把 120MB 资产带进仓库）。

use slint::Model;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

fn build_rig(view: &ViewState) -> LivePort<yeban_app::ui::MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(view, &scene)
    })
    .expect("LivePort 必须成功装配 MainWindow")
}

/// 契约面（旧接线写入）的显示态必须镜像到新 UI 的显示属性。
#[test]
fn sync_aether_display_mirrors_contract_state() {
    let port = build_rig(&ViewState::demo());
    let ui = port.ui();

    ui.set_playing(true);
    ui.set_recording(true);
    ui.set_cycle_enabled(true);
    ui.set_timecode("002.03.04".into());
    ui.set_bpm_display("132.00".into());
    ui.set_master_meter_level(0.72);

    host::sync_aether_display(ui);

    assert!(ui.get_is_playing(), "playing -> is-playing");
    assert!(ui.get_is_recording(), "recording -> is-recording");
    assert!(ui.get_is_looping(), "cycle-enabled -> is-looping");
    assert_eq!(ui.get_timecode_text().as_str(), "002.03.04");
    assert_eq!(ui.get_bpm_val(), 132, "bpm-display -> bpm-val");
    assert!(
        (ui.get_master_meter_l() - 0.72).abs() < 1e-4,
        "master-meter-level -> master-meter-l"
    );
}

/// 工程投影必须驱动新 UI 的同名数据数组（导入后看得见轨道/片段/音符）。
#[test]
fn apply_view_drives_new_ui_data_arrays() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let port = build_rig(&view);
    let ui = port.ui();

    host::apply_view(ui, &view, 1920.0, 0.0);

    assert!(
        !view.tracks.is_empty(),
        "参照工程必须有音轨, 否则本判据是空绿"
    );
    assert_eq!(
        ui.get_tracks().row_count(),
        view.tracks.len(),
        "新 UI 的 tracks 必须等于投影轨道数"
    );
    assert_eq!(
        ui.get_track_names().row_count(),
        view.tracks.len(),
        "新 UI 的 track-names 必须与投影等长"
    );
    assert_eq!(
        ui.get_track_mutes().row_count(),
        view.tracks.len(),
        "混音开关数组必须与投影等长"
    );
}

/// 新 UI 独有的轨道回调必须真的产生状态变化（不是弹完 toast 什么都不做）。
#[test]
fn new_ui_track_callback_is_not_silent() {
    let port = build_rig(&ViewState::demo());
    let ui = port.ui();
    let before = ui
        .get_tracks()
        .row_data(0)
        .expect("演示视图必须至少有一条轨道")
        .muted;

    ui.invoke_track_toggle_mute(1);

    let after = ui.get_tracks().row_data(0).expect("轨道仍在").muted;
    assert_ne!(before, after, "点击静音必须翻转该轨的 muted");
}
