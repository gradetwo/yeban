//! T-FIX-217：音源管理对话框的行内按钮必须能用键盘到达并激活。
//!
//! T-FIX-182 只把搜索框与 ✕ 放进轮转表，行内按钮（删除 / 体检 / 下载 /
//! 用到当前轨 / 全部下载 / 重试）全是鼠标专属 —— `Tab` 永远到不了它们。
//! 本文件钉住修好的行为（与 `tests/modal_focus_entry.rs` 同款活窗口判据，
//! 不是源字串）：
//!
//! ① 打开 ⇒ 焦点落在搜索框（`focused-control == 0`）；
//! ② `Tab` 沿树序走进各行：已安装删除钮 → 全部下载 → 可下载包的下载钮 → ✕，
//!    读数与槽位表一致（见对话框文件头的槽位表）；
//! ③ 行按钮上按 `Return` ⇒ 与鼠标点击同一回调（下载真的发 `download-pack`，
//!    删除两步：先武装确认态，再在确认钮上 `Return` 才发 `delete-library`）；
//! ④ `Esc` ⇒ 弹窗关闭、焦点复位回快捷键层。
//!
//! ## 本判据证明什么、不证明什么
//!
//! * 证明：`Tab` 真的走到行按钮上（读数到行槽位）；`Return` 真的触发行动作
//!   （回调收到正确的 id）；删除确认态的两步键盘链路；`Esc` 关闭与复位。
//! * 不证明：像素（焦点环是边框绑定，无头测试不截图 —— 见回执）；滚出可视区
//!   的行被 `Tab` 到时 `Flickable` 不自动滚动（已知缺口，见回执）；
//!   `Shift+Tab` 反向（平台通道注不进来，与 `modal_focus_entry.rs` 同款边界，
//!   反向与正向共用同一张槽位表）；宿主的下载 / 删除实现本身（回调被测试
//!   换成本地记录，见下面"回调置换"一节）。
//!
//! ## 回调置换
//!
//! 活窗口装配（`live_surface.rs`）已经接了宿主的
//! `on_sound_library_download` / `on_sound_library_delete`。本文件把这两条
//! 换成本地记录（只记 id，不跑下载 / 删除实现），与 T-FIX-182 在音频设置那只
//! 上装 `on_close_audio_settings` 同款做法。判据证明的是"对话框的键盘激活
//! 真的发到回调"，不是宿主的下载 / 删除实现 —— 后者各有自己的判据。
//!
//! ## 删除钮按 `Return` 后的焦点去向
//!
//! 删除钮只武装确认态（`confirm-delete-id = id`），它自己随即从树上消失
//! （`if` 条件翻转）。元素销毁不发 `focus-lost`，焦点不能留在将死元素上，
//! 所以武装动作在翻转前先把焦点挪到搜索框（0）—— 随后 `Tab` 到达确认钮
//! （101）。同款处理在全部六个会销毁本钮的行动作上（确认 / 取消 / 体检 /
//! 开始检查 / 不用了），键盘路径无条件挪、鼠标路径只在焦点正是本钮时挪。
//! （曾试过让武装动作直接点名确认钮取焦：Slint 不允许跨 `if` 分支访问 id，
//! 编译就拒绝；也试过陷阱在读数 `-1` 时重锚 —— 死焦点的 `Tab` 根本到不了
//! 陷阱（冒泡链从焦点起步，焦点死了链就断了），所以只能在销毁前挪走。）

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
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

/// `Tab` 走进各行，`Return` 真的下载。
#[test]
fn tab_reaches_row_buttons_and_return_downloads_the_pack() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    let downloads: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let downloads = downloads.clone();
        ui.on_sound_library_download(move |id| {
            downloads.borrow_mut().push(id.to_string());
        });
    }

    // 一行已安装（可删）+ 一个未装包；目录已拿到 ⇒ 「全部下载」出现。
    ui.set_sound_installed_ids(strings(&["lib-test-1"]));
    ui.set_sound_installed_names(strings(&["测试音源"]));
    ui.set_sound_installed_locations(strings(&["/tmp/test-lib"]));
    ui.set_sound_installed_sizes(strings(&["1 MB"]));
    ui.set_sound_installed_deletable(bools(&[true]));
    ui.set_sound_installed_doctor_needed(bools(&[false]));
    ui.set_sound_pack_ids(strings(&["pack-test-1"]));
    ui.set_sound_pack_names(strings(&["测试包"]));
    ui.set_sound_pack_versions(strings(&["1.0"]));
    ui.set_sound_pack_sizes(strings(&["2 MB"]));
    ui.set_sound_pack_descriptions(strings(&["测试用包"]));
    ui.set_sound_pack_installed(bools(&[false]));
    ui.set_sound_catalog_state(1);

    ui.set_sound_library_open(true);
    ui.invoke_focus_sound_library_dialog();
    assert!(ui.get_sound_library_open(), "音源管理：弹窗必须打开");
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "音源管理：入口焦点必须落在搜索框"
    );

    // Tab：搜索框 → 删除钮（100）→ 全部下载（300）→ 包下载钮（401）。
    for (expected, what) in [(100, "删除钮"), (300, "全部下载"), (401, "包下载钮")] {
        live.dispatch_key_press(KeyCode::Tab)
            .expect("音源管理：注入 Tab");
        assert_eq!(
            ui.get_sound_library_focused_control(),
            expected,
            "音源管理：Tab 必须走到{what}（槽位 {expected}）"
        );
        assert!(
            !ui.get_shortcut_layer_focused(),
            "音源管理：Tab 轮转中画布层必须不持有键盘"
        );
    }

    // Return ⇒ 与鼠标点击同一回调：下载真的发 pack id。
    live.dispatch_key_press(KeyCode::Return)
        .expect("音源管理：注入 Return");
    assert_eq!(
        downloads.borrow().as_slice(),
        ["pack-test-1"],
        "音源管理：行下载钮上按 Return 必须发 download-pack(pack-test-1)"
    );

    // 再 Tab ⇒ ✕（700），再 Tab ⇒ 绕回搜索框（0）。
    for expected in [700, 0] {
        live.dispatch_key_press(KeyCode::Tab)
            .expect("音源管理：注入 Tab");
        assert_eq!(
            ui.get_sound_library_focused_control(),
            expected,
            "音源管理：Tab 轮转读数必须是 {expected}"
        );
    }

    live.dispatch_key_press(KeyCode::Escape)
        .expect("音源管理：注入 Esc");
    assert!(!ui.get_sound_library_open(), "音源管理：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_sound_library_focused_control(),
        -1,
        "音源管理：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "音源管理：关闭后焦点必须复位回快捷键层"
    );
    report_line(
        "sound-library-row-focus download: tab-0-100-300-401 return-downloaded tab-700-0 esc-closed restored",
    );
}

/// 删除是两步：`Return` 武装确认态，`Tab` 到确认钮再 `Return` 才真删。
#[test]
fn return_on_delete_arms_confirm_and_return_on_confirm_deletes() {
    let mut live = build();
    let ui = live.ui().clone_strong();
    let deletes: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let deletes = deletes.clone();
        ui.on_sound_library_delete(move |id| {
            deletes.borrow_mut().push(id.to_string());
        });
    }

    // 只有一行已安装（可删），没有包：轮转表里只有搜索框 / 删除钮 / ✕。
    ui.set_sound_installed_ids(strings(&["lib-test-1"]));
    ui.set_sound_installed_names(strings(&["测试音源"]));
    ui.set_sound_installed_locations(strings(&["/tmp/test-lib"]));
    ui.set_sound_installed_sizes(strings(&["1 MB"]));
    ui.set_sound_installed_deletable(bools(&[true]));
    ui.set_sound_installed_doctor_needed(bools(&[false]));
    ui.set_sound_catalog_state(0);

    ui.set_sound_library_open(true);
    ui.invoke_focus_sound_library_dialog();
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "音源管理：入口焦点必须落在搜索框"
    );

    // Tab ⇒ 删除钮（100）。
    live.dispatch_key_press(KeyCode::Tab)
        .expect("音源管理：注入 Tab");
    assert_eq!(
        ui.get_sound_library_focused_control(),
        100,
        "音源管理：Tab 必须走到删除钮（槽位 100）"
    );

    // Return ⇒ 只武装确认态，不发删除。
    live.dispatch_key_press(KeyCode::Return)
        .expect("音源管理：注入 Return");
    assert!(
        deletes.borrow().is_empty(),
        "音源管理：删除钮上按 Return 只武装确认态，不能直接删"
    );
    // 删除钮随确认态从树上消失：动作先把焦点挪到搜索框（0），再武装 ——
    // 焦点不能留在将死元素上（销毁不发 focus-lost，读数会 stale）。
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "音源管理：武装后焦点必须先回到搜索框（槽位 0）"
    );

    // Tab ⇒ 确认删除钮（101）：搜索框之后第一个可聚焦控件就是它（删除钮已消失）。
    live.dispatch_key_press(KeyCode::Tab)
        .expect("音源管理：注入 Tab");
    assert_eq!(
        ui.get_sound_library_focused_control(),
        101,
        "音源管理：Tab 必须走到确认删除钮（槽位 101）"
    );

    // Return ⇒ 真发 delete-library(lib-test-1)。
    live.dispatch_key_press(KeyCode::Return)
        .expect("音源管理：注入 Return");
    assert_eq!(
        deletes.borrow().as_slice(),
        ["lib-test-1"],
        "音源管理：确认删除钮上按 Return 必须发 delete-library(lib-test-1)"
    );

    live.dispatch_key_press(KeyCode::Escape)
        .expect("音源管理：注入 Esc");
    assert!(!ui.get_sound_library_open(), "音源管理：Esc 必须关掉弹窗");
    assert!(
        ui.get_shortcut_layer_focused(),
        "音源管理：关闭后焦点必须复位回快捷键层"
    );
    report_line(
        "sound-library-row-focus delete: tab-100 return-arms tab-0-101 return-deleted esc-closed restored",
    );
}
