//! T-FIX-404：缺音源救援面板的行列表取焦时必须把焦点行滚进可视区。
//!
//! T-FIX-333 只钉住"焦点不丢"，不覆盖"焦点可见"：行列表装在 `Flickable`
//! 里，`Tab` 到屏外行时焦点落在看不见的行上，面板不跟随（333 回执 §5
//! 与 217 回执登记的同款已知缺口）。本文件钉住修好的行为（活窗口判据，
//! 与 `tests/missing_sounds_candidate_focus.rs` 同款装配）：
//!
//! ① 6 行、活窗口 1920×1080 时行列表视口高 384px（面板高 520 − 84 − 52），
//!    内容高 616px，只装得下约 3.7 行；`Tab` 沿树序走进各行时，焦点行
//!    一越出可视区下沿，`row-scroll.content-y` 就跟着动（最小滚动）；
//! ② 已在可视区里的行取焦时不滚动；行数少、内容装得下时全程不滚动；
//! ③ 绕回首行时向上滚回（行顶贴住视口上沿），随后已可见的行不再动。
//!
//! ## 数字口径（行高固定 102px，行 i 的内容区间 `[i*102+2, i*102+98]`）
//!
//! 视口 `[−content-y, −content-y+384]`。最小滚动 ⇒ 内容 y 取值：
//! 行 0/1/2 取焦时 0；行 3 取焦时 −20（行底 404 − 384）；行 4 取焦时 −122；
//! 行 5 取焦时 −224；绕回「先不管」/✕ 时不动（−224）；再进首行时 −2
//! （行顶 2px 贴住视口上沿）；随即行 1 取焦时不动（−2）。
//!
//! ## 本判据证明什么、不证明什么
//!
//! * 证明：`Tab` 到屏外行时视口真的跟随（读 `missing-sounds-row-scroll-y`）；
//!   已可见行取焦时视口不动；内容装得下时全程不滚；绕回时向上滚回。
//! * 不证明：鼠标滚轮（判据不注入鼠标；机制上滚动只从行 `focus-gained`
//!   的四处调用触发，`FocusScope` 的 `focus-on-click` 为 false，鼠标点按
//!   只走 `TouchArea` → `activate()`，不取焦 ⇒ 不滚动）；`Shift+Tab` 反向
//!   （平台通道注不进来，与 `modal_focus_entry.rs` 同款边界）；像素
//!   （无头测试不截图）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

fn build() -> live::LiveUi {
    let project = yeban_model::samples::filled_project();
    build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面")
}

fn strings(values: &[&str]) -> slint::ModelRc<slint::SharedString> {
    slint::ModelRc::new(slint::VecModel::from(
        values
            .iter()
            .map(|v| slint::SharedString::from(*v))
            .collect::<Vec<_>>(),
    ))
}

fn bools(values: &[bool]) -> slint::ModelRc<bool> {
    slint::ModelRc::new(slint::VecModel::from(values.to_vec()))
}

/// N 行救援数据：每行只有一枚已装候选（槽位 `100+i*10+1`），不可下载原音源。
fn set_rows(ui: &MainWindow, n: usize) {
    let names: Vec<String> = (0..n).map(|i| format!("Track {i}")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let s1: Vec<String> = (0..n).map(|i| format!("Piano {i}")).collect();
    let s1_refs: Vec<&str> = s1.iter().map(String::as_str).collect();
    let paths: Vec<String> = (0..n).map(|i| format!("/tmp/piano-{i}.sfz")).collect();
    let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let packs: Vec<String> = (0..n).map(|i| format!("pack-{i}")).collect();
    let pack_refs: Vec<&str> = packs.iter().map(String::as_str).collect();
    let details: Vec<&str> = vec!["Grand Piano"; n];
    let hints: Vec<&str> = vec!["hint"; n];
    let empty: Vec<&str> = vec![""; n];
    ui.set_missing_row_count(n as i32);
    ui.set_missing_track_names(strings(&name_refs));
    ui.set_missing_details(strings(&details));
    ui.set_missing_hints(strings(&hints));
    ui.set_missing_can_download(bools(&vec![false; n]));
    ui.set_missing_s1_name(strings(&s1_refs));
    ui.set_missing_s1_path(strings(&path_refs));
    ui.set_missing_s1_id(strings(&pack_refs));
    ui.set_missing_s1_installed(bools(&vec![true; n]));
    ui.set_missing_s2_name(strings(&empty));
    ui.set_missing_s2_path(strings(&empty));
    ui.set_missing_s2_id(strings(&empty));
    ui.set_missing_s2_installed(bools(&vec![false; n]));
    ui.set_missing_s3_name(strings(&empty));
    ui.set_missing_s3_path(strings(&empty));
    ui.set_missing_s3_id(strings(&empty));
    ui.set_missing_s3_installed(bools(&vec![false; n]));
    ui.set_missing_sounds_summary(slint::SharedString::from("多条轨缺音源"));
}

/// `on_missing_close` 由活窗口装配之外的测试自己装（与
/// `missing_sounds_candidate_focus.rs` 同款：只有关闭 + 复位）。
fn install_close(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_missing_close(move || {
        if let Some(ui) = weak.upgrade() {
            let was_open = ui.get_missing_sounds_open();
            ui.set_missing_sounds_open(false);
            if was_open {
                ui.invoke_focus_shortcut_layer();
            }
        }
    });
}

fn open_dialog(live: &mut live::LiveUi) {
    let ui = live.ui().clone_strong();
    ui.set_missing_sounds_open(true);
    ui.invoke_focus_missing_sounds_dialog();
    assert!(ui.get_missing_sounds_open(), "缺音源：弹窗必须打开");
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "缺音源：入口焦点必须落在 ✕（0 格）"
    );
    assert!(
        (ui.get_missing_sounds_row_scroll_y() - 0.0).abs() < 0.5,
        "缺音源：打开时行列表必须未滚动（content-y == 0），实测 {}",
        ui.get_missing_sounds_row_scroll_y()
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "缺音源：画布层必须不再同时持有键盘"
    );
}

/// `Tab` 一次，断言焦点槽位与行列表滚动量（`content-y`，向下滚为负值）。
fn tab_to(live: &mut live::LiveUi, expected_slot: i32, expected_scroll_y: f32, what: &str) {
    live.dispatch_key_press(KeyCode::Tab)
        .expect("缺音源：注入 Tab");
    let ui = live.ui().clone_strong();
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        expected_slot,
        "缺音源：Tab 必须走到{what}（槽位 {expected_slot}）"
    );
    let actual = ui.get_missing_sounds_row_scroll_y();
    assert!(
        (actual - expected_scroll_y).abs() < 0.5,
        "缺音源：{what}取焦时行列表必须滚到 content-y {expected_scroll_y}，实测 {actual}"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "缺音源：Tab 轮转中画布层必须不持有键盘"
    );
}

fn esc_closes(live: &mut live::LiveUi) {
    live.dispatch_key_press(KeyCode::Escape)
        .expect("缺音源：注入 Esc");
    let ui = live.ui().clone_strong();
    assert!(!ui.get_missing_sounds_open(), "缺音源：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        -1,
        "缺音源：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "缺音源：关闭后焦点必须复位回快捷键层"
    );
}

/// 6 行：`Tab` 走进屏外行时视口跟随（最小滚动）；绕回首行时向上滚回。
#[test]
fn tab_scrolls_focused_row_into_view() {
    let mut live = build();
    install_close(&live.ui().clone_strong());
    set_rows(&live.ui().clone_strong(), 6);
    open_dialog(&mut live);

    // ✕ → 行 0（101，0）→ 行 1（111，0）→ 行 2（121，0）
    // → 行 3（131，−20）→ 行 4（141，−122）→ 行 5（151，−224）
    // → 「先不管」（1，−224 不动）→ ✕（0，−224 不动）
    // → 行 0（101，−2 向上滚回）→ 行 1（111，−2 不动）。
    for (slot, scroll_y, what) in [
        (101, 0.0, "行 0 候选 1"),
        (111, 0.0, "行 1 候选 1"),
        (121, 0.0, "行 2 候选 1"),
        (131, -20.0, "行 3 候选 1"),
        (141, -122.0, "行 4 候选 1"),
        (151, -224.0, "行 5 候选 1"),
        (1, -224.0, "「先不管」"),
        (0, -224.0, "✕（绕回）"),
        (101, -2.0, "行 0 候选 1（绕回后）"),
        (111, -2.0, "行 1 候选 1（绕回后）"),
    ] {
        tab_to(&mut live, slot, scroll_y, what);
    }

    esc_closes(&mut live);
    report_line("missing-sounds-focus-scroll tab: 0,-20,-122,-224 wrap -2 esc-closed restored");
}

/// 2 行：内容装得下，全程不滚动（钉住"已可见 ⇒ 不动"）。
#[test]
fn few_rows_never_scroll() {
    let mut live = build();
    install_close(&live.ui().clone_strong());
    set_rows(&live.ui().clone_strong(), 2);
    open_dialog(&mut live);

    for (slot, what) in [
        (101, "行 0 候选 1"),
        (111, "行 1 候选 1"),
        (1, "「先不管」"),
        (0, "✕（绕回）"),
    ] {
        tab_to(&mut live, slot, 0.0, what);
    }

    esc_closes(&mut live);
    report_line("missing-sounds-focus-scroll few-rows: no-scroll esc-closed restored");
}
