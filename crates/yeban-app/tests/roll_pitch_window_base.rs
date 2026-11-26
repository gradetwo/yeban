//! ⭐ T-FIX-511：卷帘发布的音高窗口必须跟着**当前基音**走（不是固定 60..=75）。
//!
//! 判据：四档基音下 `roll-min-pitch` / `roll-max-pitch` 必须等于
//! `base ..= min(base + 泳道数 - 1, 127)`，且 `base` 是界面此刻真实的
//! `roll-base-pitch`（取景可以改它，但发布值必须与它同源）。
//! 反向：把 host 那一路改回固定 `PITCH_LANE_BASE` 时，本判据在非默认基音档变红。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size};

const WINDOW: Size = Size::new(1440, 900);
const LANES: u8 = 16;

#[test]
fn published_pitch_window_follows_the_current_base() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + MainWindow");
    let ui = port.ui();

    for requested in [60_u8, 72, 0, 112] {
        ui.set_roll_base_pitch(i32::from(requested));
        // 走一次真实重投影：`apply_view` 会先按作用域取景，再发布 min/max。
        host::apply_view(ui, &view, 1440.0, 0.0);
        let base = u8::try_from(ui.get_roll_base_pitch()).expect("基音在 u8 内");
        let expected_max = base.saturating_add(LANES - 1).min(127);
        assert_eq!(
            ui.get_roll_min_pitch(),
            i32::from(base),
            "请求基音 {requested} 时发布的下界必须等于界面实际基音 {base}"
        );
        assert_eq!(
            ui.get_roll_max_pitch(),
            i32::from(expected_max),
            "请求基音 {requested}（实际 {base}）时上界必须是 min(base+15, 127)"
        );
    }
}
