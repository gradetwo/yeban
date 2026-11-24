//! T-FIX-333：缺音源救援面板的行内按钮必须能用键盘到达并激活。
//!
//! T-FIX-182 只把 ✕ / 「先不管」放进轮转表，行内候选按钮（"换成 X" / "下载 X"）
//! 与「下载原音源」全是鼠标专属 —— `Tab` 永远到不了它们，救援的主动作键盘
//! 够不着。本文件钉住修好的行为（与 `tests/sound_library_row_focus.rs` 同款
//! 活窗口判据，不是源字串；T-FIX-217 的槽位表口径）：
//!
//! ① 打开 ⇒ 焦点落在 ✕（`focused-control == 0`）；
//! ② `Tab` 沿树序走进各行：行 0 候选 1 → 候选 2 → 下载原音源 → 行 1 候选 1 →
//!    「先不管」→ 绕回 ✕，读数与槽位表一致（`100+i*10+{1,2,3}` 候选、
//!    `100+i*10+4` 下载原音源）；空行时只剩 0/1 两格；
//! ③ 行按钮上按 `Return` / `Space` ⇒ 与鼠标点击同一回调（已装候选发
//!    `swap-suggested`，未装候选发 `open-pack`，下载原音源发
//!    `download-original`）；
//! ④ 换上（行会消失）前焦点先挪到 ✕，不留死焦点；下载类动作不删行，焦点
//!    留在原格；
//! ⑤ `Esc` ⇒ 弹窗关闭、焦点复位回快捷键层。
//!
//! ## 本判据证明什么、不证明什么
//!
//! * 证明：`Tab` 真的走到每行每格（读数到行槽位）；`Return` / `Space` 真的
//!   触发行动作（回调收到正确的行号与参数）；换上动作前焦点真的挪走；
//!   `Esc` 关闭与复位；空行时轮转不越界。
//! * 不证明：`Shift+Tab` 反向（平台通道注不进来，与 `modal_focus_entry.rs`
//!   同款边界，反向与正向共用同一张槽位表）；宿主的换装 / 下载实现本身
//!   （回调被测试换成本地记录，见下面"回调置换"一节）；像素（无头测试不截图）。
//!
//! ## 回调置换
//!
//! 活窗口装配（`live_surface.rs`）不跑 `host::wire_missing_sounds_dialog`
//! （它要工程端口与缺失清单，只 `main.rs` 调）。本文件按宿主回调的同一写法
//! 装一个 `on_missing_close`（只有关闭 + 复位），并把三条行动回调换成本地
//! 记录（只记参数，不跑换装 / 下载实现），与 T-FIX-217 在音源管理那只上装
//! `on_sound_library_download` 同款做法。判据证明的是"对话框的键盘激活
//! 真的发到回调"，不是宿主的换装 / 下载实现 —— 后者各有自己的判据。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
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

/// 两行救援数据：行 0 三枚候选（已装 / 未装）+ 可下载原音源；
/// 行 1 只有一枚已装候选，不可下载原音源。
fn set_two_rows(ui: &MainWindow) {
    ui.set_missing_row_count(2);
    ui.set_missing_track_names(strings(&["Lead", "Bass"]));
    ui.set_missing_details(strings(&["Grand Piano", "Bass"]));
    ui.set_missing_hints(strings(&["hint 0", "hint 1"]));
    ui.set_missing_can_download(bools(&[true, false]));
    ui.set_missing_s1_name(strings(&["Piano A", "Bass A"]));
    ui.set_missing_s1_path(strings(&["/tmp/piano-a.sfz", "/tmp/bass-a.sfz"]));
    ui.set_missing_s1_id(strings(&["pack-a", "pack-c"]));
    ui.set_missing_s1_installed(bools(&[true, true]));
    ui.set_missing_s2_name(strings(&["Piano B", ""]));
    ui.set_missing_s2_path(strings(&["/tmp/piano-b.sfz", ""]));
    ui.set_missing_s2_id(strings(&["pack-b", ""]));
    ui.set_missing_s2_installed(bools(&[false, false]));
    ui.set_missing_s3_name(strings(&["", ""]));
    ui.set_missing_s3_path(strings(&["", ""]));
    ui.set_missing_s3_id(strings(&["", ""]));
    ui.set_missing_s3_installed(bools(&[false, false]));
    ui.set_missing_sounds_summary(slint::SharedString::from("两条轨缺音源"));
}

/// `on_missing_close` 由活窗口装配之外的测试自己装（与 `modal_focus_entry.rs`
/// 同款：只有关闭 + 复位，按宿主的同一写法）。
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
        !ui.get_shortcut_layer_focused(),
        "缺音源：画布层必须不再同时持有键盘"
    );
}

fn tab_to(live: &mut live::LiveUi, expected: i32, what: &str) {
    live.dispatch_key_press(KeyCode::Tab)
        .expect("缺音源：注入 Tab");
    let ui = live.ui().clone_strong();
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        expected,
        "缺音源：Tab 必须走到{what}（槽位 {expected}）"
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

/// `Tab` 沿树序走进每行每格，读数与槽位表一致，末格再进绕回 ✕。
#[test]
fn tab_walks_every_row_cell_in_tree_order_and_wraps() {
    let mut live = build();
    install_close(&live.ui().clone_strong());
    set_two_rows(&live.ui().clone_strong());
    open_dialog(&mut live);

    // ✕ → 行 0 候选 1（101）→ 候选 2（102）→ 下载原音源（104，候选 3 为空）
    // → 行 1 候选 1（111）→ 「先不管」（1）→ 绕回 ✕（0）。
    for (expected, what) in [
        (101, "行 0 候选 1"),
        (102, "行 0 候选 2"),
        (104, "行 0 下载原音源"),
        (111, "行 1 候选 1"),
        (1, "「先不管」"),
        (0, "✕（绕回）"),
    ] {
        tab_to(&mut live, expected, what);
    }

    esc_closes(&mut live);
    report_line("missing-sounds-candidate-focus tab: 0-101-102-104-111-1-0 esc-closed restored");
}

/// `Return` / `Space` 触发与鼠标点击同一回调；换上前焦点挪到 ✕，下载类不动。
#[test]
fn return_and_space_trigger_the_same_callbacks_as_click() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    install_close(&ui);
    let swaps: Rc<RefCell<Vec<(i32, String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let packs: Rc<RefCell<Vec<(i32, String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let downloads: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let swaps = swaps.clone();
        ui.on_missing_swap_suggested(move |row, path, name| {
            swaps
                .borrow_mut()
                .push((row, path.to_string(), name.to_string()));
        });
        let packs = packs.clone();
        ui.on_missing_open_pack(move |row, pack_id, name| {
            packs
                .borrow_mut()
                .push((row, pack_id.to_string(), name.to_string()));
        });
        let downloads = downloads.clone();
        ui.on_missing_download_original(move |row| {
            downloads.borrow_mut().push(row);
        });
    }
    set_two_rows(&ui);
    open_dialog(&mut live);

    // 行 0 候选 1（已装）上按 Return ⇒ 发 swap-suggested，且焦点先挪到 ✕。
    tab_to(&mut live, 101, "行 0 候选 1");
    live.dispatch_key_press(KeyCode::Return)
        .expect("缺音源：注入 Return");
    assert_eq!(
        swaps.borrow().as_slice(),
        [(0, "/tmp/piano-a.sfz".to_string(), "Piano A".to_string())],
        "缺音源：已装候选上按 Return 必须发 swap-suggested(0, piano-a)"
    );
    assert_eq!(
        live.ui().get_missing_sounds_focused_control(),
        0,
        "缺音源：换上前焦点必须先挪到 ✕（行随即消失，不能留死焦点）"
    );

    // 行 0 候选 2（未装）上按 Return ⇒ 发 open-pack，焦点留在原格（行不删）。
    tab_to(&mut live, 101, "行 0 候选 1");
    tab_to(&mut live, 102, "行 0 候选 2");
    live.dispatch_key_press(KeyCode::Return)
        .expect("缺音源：注入 Return");
    assert_eq!(
        packs.borrow().as_slice(),
        [(0, "pack-b".to_string(), "Piano B".to_string())],
        "缺音源：未装候选上按 Return 必须发 open-pack(0, pack-b)"
    );
    assert_eq!(
        live.ui().get_missing_sounds_focused_control(),
        102,
        "缺音源：下载类动作不删行，焦点必须留在原格"
    );

    // 行 0 下载原音源上按 Return ⇒ 发 download-original，焦点留在原格。
    tab_to(&mut live, 104, "行 0 下载原音源");
    live.dispatch_key_press(KeyCode::Return)
        .expect("缺音源：注入 Return");
    assert_eq!(
        downloads.borrow().as_slice(),
        [0],
        "缺音源：下载原音源上按 Return 必须发 download-original(0)"
    );
    assert_eq!(
        live.ui().get_missing_sounds_focused_control(),
        104,
        "缺音源：下载原音源后焦点必须留在原格"
    );

    // 行 1 候选 1（已装）上按 Space ⇒ 同样发 swap-suggested，焦点挪到 ✕。
    tab_to(&mut live, 111, "行 1 候选 1");
    live.dispatch_key_press(KeyCode::Space)
        .expect("缺音源：注入 Space");
    assert_eq!(
        swaps.borrow().as_slice(),
        [
            (0, "/tmp/piano-a.sfz".to_string(), "Piano A".to_string()),
            (1, "/tmp/bass-a.sfz".to_string(), "Bass A".to_string()),
        ],
        "缺音源：已装候选上按 Space 必须发 swap-suggested(1, bass-a)"
    );
    assert_eq!(
        live.ui().get_missing_sounds_focused_control(),
        0,
        "缺音源：Space 换上前焦点同样必须先挪到 ✕"
    );

    esc_closes(&mut live);
    report_line(
        "missing-sounds-candidate-focus actions: return-swap return-pack return-download space-swap esc-closed restored",
    );
}

/// 空行时轮转只有 0/1 两格，不越界（T-FIX-182 的旧行为，本单不得破坏）。
#[test]
fn zero_rows_keep_the_two_cell_loop_and_do_not_go_out_of_bounds() {
    let mut live = build();
    install_close(&live.ui().clone_strong());
    // 行数 0：不写任何行数据（与 `modal_focus_entry.rs` 的缺音源那段同一状态）。
    live.ui().set_missing_row_count(0);
    open_dialog(&mut live);

    for expected in [1, 0, 1, 0] {
        tab_to(&mut live, expected, "「先不管」/✕");
    }

    esc_closes(&mut live);
    report_line("missing-sounds-candidate-focus zero-row: tab-0-1-0-1-0 esc-closed restored");
}
