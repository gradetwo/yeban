//! T-FIX-131：其余四只模态的**入口**——打开时焦点真的移进弹窗，`Esc` 真的关掉并复位。
//!
//!  undo 树那只由 `tests/undo_tree_focus_trap.rs` 覆盖（焦点闭环整条链）；本文件只钉
//!  另外四只的入口行为：导出 WAV / 人性化 / 鼓机 / 音频设置。每一只都走宿主开弹窗的
//!  **同一个顺序**（`set_<x>_open(true)` 再调 `invoke_focus_*`），断言三件事：
//!
//! ① 打开 ⇒ 焦点落在第一格（`focused-* == 0`），画布键盘层不再同时持有；
//! ② `Esc`（真平台按键，经陷阱 `FocusScope`）⇒ 弹窗关闭；
//! ③ 关闭 ⇒ 焦点复位回快捷键层（宿主 `close-*` 回调里那一次 `invoke_focus_shortcut_layer`）。
//!
//! ## 音频设置那只的 `close` 回调为什么由本文件自己装
//!
//! 导出 / 人性化 / 鼓机的 `close-*` 回调由活窗口装配（`wire_export_wav_browse` /
//! `wire_humanize_dialog` / `wire_drum_gen_dialog`）接好；音频设置的接线函数
//! `host::wire_audio_settings` 需要一条 `ProductionLoop`，活窗口装配不跑它（只
//! `main.rs` 调）。`Esc` → 陷阱 → `close()` 这一段是四只模态**同一份** Slint 机制，
//! 为让音频设置那只的 ②③ 也能被量到，本文件在打开前按宿主回调的**同一写法**装一个
//! `on_close_audio_settings`（只有关闭 + 复位，不碰设备 / 计时器那些生产态）。
//! ⛔ 这不是第二份实现：生产那份仍在 `wire_audio_settings` 里，本文件这份只在判据里
//! 存在；若生产那份改了关闭语义，这里必须跟着改（见下面"变异表"）。
//!
//! ## 本判据证明什么、不证明什么
//!
//! * 证明：四只模态的 `focus-*` 入口函数真的把焦点移进弹窗第一格；陷阱的 `Esc`
//!   真的关弹窗并把焦点送回快捷键层（宿主 `close-*` 回调那一段）。
//! * 不证明：模态内各控件之间的 Tab 顺序（与 `undo_tree_focus_trap.rs` 同款边界）；
//!   各弹窗的业务行为（导出真的落盘 / 人性化真的改音符 / 鼓机真的生成 / 音频设置
//!   真的切设备 —— 那些各有自己的判据）；像素（无头测试不截图）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | `app.slint` 里某只 `focus-*` 改回空块 `{ }` | 本文件对应那只的 ① 红（焦点读数不动） |
//! | 删掉某只模态的陷阱 `FocusScope` | ② 红（`Esc` 到不了 `close()`，弹窗关不掉） |
//! | 宿主 `close-*` 回调里删掉 `invoke_focus_shortcut_layer()` | ③ 红（焦点不复位） |
//! | 生产 `wire_audio_settings` 的关闭语义改了，本文件装的那份没跟上 | 音频设置那只的 ②③ 与生产分叉 —— 本表如实登记这份跟随义务 |

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
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

/// 四只模态的入口 + `Esc` 关闭 + 焦点复位（一条链，一只接一只）。
#[test]
fn each_modal_focuses_its_first_control_on_entry_and_restores_focus_on_esc() {
    let mut live = build();
    let ui = live.ui().clone_strong();

    // 音频设置的 `close` 回调活窗口装配没接（见文件头）：按宿主的同一写法装一份。
    {
        let weak = ui.as_weak();
        ui.on_close_audio_settings(move || {
            if let Some(ui) = weak.upgrade() {
                let was_open = ui.get_audio_settings_open();
                ui.set_audio_settings_open(false);
                if was_open {
                    ui.invoke_focus_shortcut_layer();
                }
            }
        });
    }

    // 起点：焦点在画布键盘层。
    assert!(
        ui.get_shortcut_layer_focused(),
        "起点：快捷键层必须持有键盘"
    );

    // ---- 导出 WAV ---------------------------------------------------------
    ui.set_export_wav_open(true);
    ui.invoke_focus_export_wav_dialog();
    assert!(ui.get_export_wav_open(), "导出：弹窗必须打开");
    assert_eq!(
        ui.get_export_wav_focused_field(),
        0,
        "导出：入口焦点必须落在第一格（路径框）"
    );
    assert_eq!(
        ui.get_export_wav_focused_control(),
        0,
        "导出：入口焦点必须落在第一格（轮转全集）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "导出：画布层必须不再同时持有键盘"
    );
    live.dispatch_key_press(KeyCode::Escape)
        .expect("导出：注入 Esc");
    assert!(!ui.get_export_wav_open(), "导出：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_export_wav_focused_control(),
        -1,
        "导出：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "导出：关闭后焦点必须复位回快捷键层"
    );
    report_line("modal-entry export-wav: field=0 control=0 esc-closed restored");

    // ---- 人性化 -----------------------------------------------------------
    ui.set_humanize_open(true);
    ui.invoke_focus_humanize_dialog();
    assert!(ui.get_humanize_open(), "人性化：弹窗必须打开");
    assert_eq!(
        ui.get_humanize_focused_field(),
        0,
        "人性化：入口焦点必须落在第一格（时值抖动量，focus-section 默认为 0）"
    );
    assert_eq!(
        ui.get_humanize_focused_control(),
        0,
        "人性化：入口焦点必须落在第一格（轮转全集）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "人性化：画布层必须不再同时持有键盘"
    );
    live.dispatch_key_press(KeyCode::Escape)
        .expect("人性化：注入 Esc");
    assert!(!ui.get_humanize_open(), "人性化：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_humanize_focused_control(),
        -1,
        "人性化：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "人性化：关闭后焦点必须复位回快捷键层"
    );
    report_line("modal-entry humanize: field=0 control=0 esc-closed restored");

    // ---- 鼓机 -------------------------------------------------------------
    ui.set_drum_gen_open(true);
    ui.invoke_focus_drum_gen_dialog();
    assert!(ui.get_drum_gen_open(), "鼓机：弹窗必须打开");
    assert_eq!(
        ui.get_drum_gen_focused_control(),
        0,
        "鼓机：入口焦点必须落在第一格（复杂度）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "鼓机：画布层必须不再同时持有键盘"
    );
    live.dispatch_key_press(KeyCode::Escape)
        .expect("鼓机：注入 Esc");
    assert!(!ui.get_drum_gen_open(), "鼓机：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_drum_gen_focused_control(),
        -1,
        "鼓机：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "鼓机：关闭后焦点必须复位回快捷键层"
    );
    report_line("modal-entry drum-gen: control=0 esc-closed restored");

    // ---- 音频设置 ---------------------------------------------------------
    ui.set_audio_settings_open(true);
    ui.invoke_focus_audio_settings_dialog();
    assert!(ui.get_audio_settings_open(), "音频设置：弹窗必须打开");
    assert_eq!(
        ui.get_audio_settings_focused_control(),
        0,
        "音频设置：入口焦点必须落在第一格（输出选择器）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "音频设置：画布层必须不再同时持有键盘"
    );
    live.dispatch_key_press(KeyCode::Escape)
        .expect("音频设置：注入 Esc");
    assert!(!ui.get_audio_settings_open(), "音频设置：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_audio_settings_focused_control(),
        -1,
        "音频设置：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "音频设置：关闭后焦点必须复位回快捷键层"
    );
    report_line("modal-entry audio-settings: control=0 esc-closed restored");
}

/// 出处纪律（⛔ 不开窗口）：四只模态的入口各恰好一条直连。
#[test]
fn each_modal_entry_is_a_single_direct_delegation() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let code_app = code_only(&app);

    assert_eq!(
        code_app
            .matches("focus-export-wav-dialog() { export-wav-dialog.focus-first-field(); }")
            .count(),
        1,
        "导出入口必须恰好一条直连"
    );
    assert_eq!(
        code_app
            .matches("focus-humanize-dialog() { humanize-dialog.focus-first-field(); }")
            .count(),
        1,
        "人性化入口必须恰好一条直连"
    );
    assert_eq!(
        code_app
            .matches("focus-drum-gen-dialog() { drum-gen-dialog.focus-first-field(); }")
            .count(),
        1,
        "鼓机入口必须恰好一条直连"
    );
    assert_eq!(
        code_app
            .matches(
                "focus-audio-settings-dialog() { audio-settings-dialog.focus-first-control(); }"
            )
            .count(),
        1,
        "音频设置入口必须恰好一条直连"
    );
}

/// 只在**代码行**上数，注释里提到这个名字不算调用点（与 `tests/search_focus_key.rs` ⑤ 同口径）。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
