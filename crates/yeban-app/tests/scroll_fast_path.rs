//! ⭐ T-FIX-494：滚轮/双指滚动走**快路径**（只重写依赖横向偏移的属性）。
//!
//! 两条判据：
//! ① **等价性**：`apply_scroll_view(scroll)` 写出的那几个属性，与整份 `apply_view`
//!    在同一偏移下写出的值**逐项相同**（快路径不是第二套算法，而是同一段写入）；
//! ② **接线**：两个滚动回调（`on_scroll_requested` / `on_piano_roll_wheel`）只调
//!    `apply_scroll_view` 与 `scroll_projection`，⛔ 不再调整份 `apply_view`。

use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size};

const WINDOW: Size = Size::new(1440, 900);
const VIEWPORT: f32 = 1440.0;

/// 依赖横向偏移的那组读数（快路径必须与整份重投影逐项一致）。
#[derive(Debug, PartialEq)]
struct ScrollSlice {
    bar_start: i32,
    bar_count: i32,
    bar_width: f32,
    scroll_x: f32,
    min_tick: i32,
    max_tick: i32,
    note_ulids: Vec<String>,
    note_positions: Vec<f32>,
    note_widths: Vec<f32>,
}

fn capture(ui: &yeban_app::ui::MainWindow) -> ScrollSlice {
    let strings = |model: slint::ModelRc<slint::SharedString>| -> Vec<String> {
        model.iter().map(|value| value.to_string()).collect()
    };
    let numbers = |model: slint::ModelRc<f32>| -> Vec<f32> { model.iter().collect() };
    ScrollSlice {
        bar_start: ui.get_bar_start_index(),
        bar_count: ui.get_bar_count(),
        bar_width: ui.get_bar_width_px(),
        scroll_x: ui.get_roll_scroll_x(),
        min_tick: ui.get_roll_min_tick(),
        max_tick: ui.get_roll_max_tick(),
        note_ulids: strings(ui.get_note_ulids()),
        note_positions: numbers(ui.get_note_positions()),
        note_widths: numbers(ui.get_note_widths()),
    }
}

#[test]
fn the_scroll_fast_path_matches_a_full_reprojection() {
    let project = yeban_model::samples::project_with_notes(400);
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + MainWindow");
    let ui = port.ui();

    for scroll in [0.0_f32, 300.0, 1200.0] {
        // 整份重投影的读数（基准）。
        host::apply_view(ui, &view, VIEWPORT, scroll);
        let expected = capture(ui);
        // 回到别的偏移，再让快路径写同一个值。
        host::apply_view(ui, &view, VIEWPORT, scroll + 137.0);
        host::apply_scroll_view(ui, &view, VIEWPORT, scroll);
        let actual = capture(ui);
        assert_eq!(
            actual, expected,
            "偏移 {scroll}：快路径必须与整份重投影逐项相同"
        );
    }
}

#[test]
fn both_wheel_callbacks_use_the_fast_path() {
    let source = include_str!("../src/host.rs");
    let start = source
        .find("fn install_roll_scroll(")
        .expect("install_roll_scroll 必须在");
    let rest = &source[start..];
    let end = rest.find("\n}\n").expect("函数体必须以行首 } 结束");
    let body = &rest[..end];
    assert_eq!(
        body.matches("apply_scroll_view(").count(),
        2,
        "两个滚动回调都必须走快路径"
    );
    assert!(
        !body.contains("apply_view("),
        "滚动回调里⛔ 不得再调整份 apply_view（那是 T-FIX-494 要消除的每事件全量重投影）"
    );
    assert_eq!(
        body.matches("scroll_projection(").count(),
        2,
        "两条路径都必须经投影缓存（工程版本/宽度/缩放不变就复用）"
    );
}
