//! T-FIX-323：换工程后纯编辑面必须重绑到新 `UndoPort`。
//!
//! 判据按生产顺序复现：启动期用旧端口接 `host::wire_edit_surfaces`；换工程时新建
//! `UndoPort`，先跑 `wire_view_callbacks`（旧回调层），再跑 `wire_edit_surfaces`
//! 重绑编辑面。随后依次触发设备机架旁通、宏释放、BPM 敲入、建 MIDI 轨：
//! 四类 Op 都必须只落在**新**端口，旧端口的工程必须一位不动。
//!
//! 红对照：`T_FIX_323_RED=1` 时跳过最后的重绑，四类编辑会写进旧端口，判据变红。

use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_model::project::TrackKind;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn live_window(project: &YebanProjectV1) -> LivePort<MainWindow> {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口")
}

fn undo_port(project: &YebanProjectV1, tag: &str) -> Rc<UndoPort> {
    Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ))
}

/// 展示轨里第一台带 `eq_low_gain` 的设备身份（旁通判据用）。
fn first_device_id(
    project: &YebanProjectV1,
    track_id: yeban_model::EntityId,
) -> yeban_model::EntityId {
    project.tracks[&track_id]
        .devices
        .first()
        .map(|device| device.id)
        .expect("展示轨必须至少有一台设备")
}

#[test]
fn project_switch_rebinds_edit_surfaces_to_new_undo_port() {
    let old_project = yeban_model::samples::filled_project();
    let new_project = yeban_model::samples::filled_project();
    let live = live_window(&old_project);
    let ui = live.ui();

    // 启动装配：旧端口接全部纯编辑面。
    let old_port = undo_port(&old_project, "old");
    host::wire_edit_surfaces(ui, &old_port);
    let old_snapshot = old_port.try_project().expect("旧工程");

    // 换工程：新端口 + 旧回调层重跑，再按生产顺序重绑编辑面。
    let new_port = undo_port(&new_project, "new");
    host::wire_view_callbacks(ui);
    if std::env::var_os("T_FIX_323_RED").is_none() {
        host::wire_edit_surfaces(ui, &new_port);
    }

    // ① 设备机架旁通：按设备身份触发，必须写进新端口。
    let device_track = new_port
        .try_project()
        .expect("新工程")
        .tracks
        .values()
        .find(|track| track.kind != TrackKind::Master && !track.devices.is_empty())
        .expect("新工程必须有一条带设备的展示轨")
        .id;
    let device_id = first_device_id(&new_port.try_project().expect("新工程"), device_track);
    ui.set_selected_track_id(device_track.to_string().into());
    ui.invoke_device_bypass_toggle(device_id.to_string().into());
    assert!(
        new_port.try_project().expect("新工程").tracks[&device_track].devices[0].bypassed,
        "T-FIX-323 换工程后设备旁通必须写进新端口"
    );

    // ② 宏释放：按同一轨的宏 0 做一次真实 grab → drag → release。
    let macro_track = new_port
        .try_project()
        .expect("新工程")
        .tracks
        .values()
        .find(|track| track.kind != TrackKind::Master && !track.macros.is_empty())
        .expect("新工程必须有一条带宏的展示轨")
        .id;
    ui.set_selected_track_id(macro_track.to_string().into());
    let old_macro = new_port.try_project().expect("新工程").tracks[&macro_track].macros[0].value;
    ui.invoke_macro_grab(0, 0.0);
    ui.invoke_macro_drag(0, 1.0);
    ui.invoke_macro_release(0);
    assert_ne!(
        new_port.try_project().expect("新工程").tracks[&macro_track].macros[0].value,
        old_macro,
        "T-FIX-323 换工程后宏释放必须写进新端口"
    );

    // ③ BPM 敲入：必须写进新端口。
    let old_bpm = new_port.try_project().expect("新工程").bpm;
    ui.invoke_bpm_commit("140".into());
    assert!(
        (new_port.try_project().expect("新工程").bpm - 140.0).abs() < f64::EPSILON,
        "T-FIX-323 换工程后 BPM 敲入必须写进新端口"
    );
    assert_ne!(old_bpm, 140.0, "判据要求 BPM 真变化，而不是同值早退");

    // ④ 建 MIDI 轨：必须写进新端口。
    let before_tracks = new_port.try_project().expect("新工程").tracks.len();
    ui.invoke_track_create_midi();
    assert_eq!(
        new_port.try_project().expect("新工程").tracks.len(),
        before_tracks + 1,
        "T-FIX-323 换工程后建 MIDI 轨必须写进新端口"
    );

    // 旧端口必须一位不动（修前四类编辑会落到这里）。
    assert_eq!(
        old_port.try_project().expect("旧工程"),
        old_snapshot,
        "T-FIX-323 换工程后的编辑不得再写旧端口"
    );
}
