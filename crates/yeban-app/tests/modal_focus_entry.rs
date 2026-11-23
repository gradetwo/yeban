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
//! T-FIX-182 补上 131 漏掉的三只（另存为模板 / 音源管理 / 缺音源救援），同一份
//! 契约再加一条：④ `Tab` 在模态内闭环（读数绕回第一格，焦点从未离开弹窗）。
//! 另存为模板五格全轮、音源管理与缺音源救援各两格；`Shift+Tab` 反向未注入 ——
//! 平台事件通道带不了修饰位，与 `undo_tree_focus_trap.rs` 同款边界（反向分支与
//! 正向共用同一张 `focus-control` 表，见变异表）。
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
//! ## 缺音源救援那只的 `close` 回调同样由本文件自己装
//!
//! 活窗口装配不跑 `host::wire_missing_sounds_dialog`（它要工程端口与缺失清单，
//! 只 `main.rs` 调），`Esc` → 陷阱 → `missing-close()` 在判据里没人接。本文件
//! 在打开前按宿主回调的**同一写法**装一个 `on_missing_close`（只有关闭 + 复位）。
//! 音源管理那只不用装：活窗口装配跑 `host::wire_sound_library_dialog`。
//! 另存为模板那只也不用装：它的关窗本就写在 `.slint` 里（`close =>` 置位 + 复位），
//! 宿主没有 `close` 回调 —— 复位那一行与五只模态的宿主回调同一语义。
//!
//! ## 本判据证明什么、不证明什么
//!
//! * 证明：七只模态的 `focus-*` 入口函数真的把焦点移进弹窗第一格；陷阱的 `Esc`
//!   真的关弹窗并把焦点送回快捷键层；陷阱的 `Tab` 真的在模态内轮转（正向）。
//! * 不证明：`Shift+Tab` 反向轮转（平台通道注不进来，与 `undo_tree_focus_trap.rs`
//!   同款边界）；各弹窗的业务行为（导出真的落盘 / 人性化真的改音符 / 鼓机真的生成 /
//!   音频设置真的切设备 / 模板真的存盘 / 音源真的下载 —— 那些各有自己的判据）；
//!   像素（无头测试不截图）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | `app.slint` 里某只 `focus-*` 改回空块 `{ }` | 本文件对应那只的 ① 红（焦点读数不动） |
//! | 删掉某只模态的陷阱 `FocusScope` | ② 红（`Esc` 到不了 `close()`，弹窗关不掉） |
//! | 宿主 `close-*` 回调里删掉 `invoke_focus_shortcut_layer()` | ③ 红（焦点不复位） |
//! | 生产 `wire_audio_settings` 的关闭语义改了，本文件装的那份没跟上 | 音频设置那只的 ②③ 与生产分叉 —— 本表如实登记这份跟随义务 |
//! | 生产 `wire_missing_sounds_dialog` 的关闭语义改了，本文件装的那份没跟上 | 缺音源救援那只的 ②③ 与生产分叉 —— 本表如实登记这份跟随义务 |
//! | 某只新模态的陷阱 `Tab` 分支改回 `reject` | 对应那只的 ④ 红（读数停在 0；`Tab` 会撞上画布语义） |
//! | 某只新模态的 `focus-next-control()` 删掉绕回 | 对应那只的 ④ 红（末格再 `Tab` 读数停在末格，不回 0） |

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

/// T-FIX-182：131 漏掉的三只模态 —— 入口 + `Tab` 闭环 + `Esc` 关闭 + 焦点复位。
///
/// 顺序与上面四只同一条链，一只接一只（起点都是快捷键层持有键盘）。
#[test]
fn the_three_missed_modals_focus_tab_in_a_loop_and_restore_on_esc() {
    let mut live = build();
    let ui = live.ui().clone_strong();

    // 缺音源救援的 `close` 回调活窗口装配没接（见文件头）：按宿主的同一写法装一份。
    {
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

    // 起点：焦点在画布键盘层。
    assert!(
        ui.get_shortcut_layer_focused(),
        "起点：快捷键层必须持有键盘"
    );

    // ---- 另存为模板 ---------------------------------------------------------
    ui.set_template_save_open(true);
    ui.invoke_focus_template_save_dialog();
    assert!(ui.get_template_save_open(), "模板：弹窗必须打开");
    assert_eq!(
        ui.get_template_save_focused_control(),
        0,
        "模板：入口焦点必须落在第一格（曲风）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "模板：画布层必须不再同时持有键盘"
    );
    // `Tab` ×5：曲风 → 子曲风 → 模板名 → 保存钮 → ✕ → 绕回曲风。
    for expected in [1, 2, 3, 4, 0] {
        live.dispatch_key_press(KeyCode::Tab)
            .expect("模板：注入 Tab");
        assert_eq!(
            ui.get_template_save_focused_control(),
            expected,
            "模板：Tab 轮转读数必须是 {expected}"
        );
        assert!(
            !ui.get_shortcut_layer_focused(),
            "模板：Tab 轮转中画布层必须不持有键盘"
        );
    }
    live.dispatch_key_press(KeyCode::Escape)
        .expect("模板：注入 Esc");
    assert!(!ui.get_template_save_open(), "模板：Esc 必须关掉弹窗");
    assert_eq!(
        ui.get_template_save_focused_control(),
        -1,
        "模板：关闭后焦点必须不在模态内"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "模板：关闭后焦点必须复位回快捷键层"
    );
    report_line("modal-entry template-save: field=0 tab-0-1-2-3-4-0 esc-closed restored");

    // ---- 音源管理 -----------------------------------------------------------
    ui.set_sound_library_open(true);
    ui.invoke_focus_sound_library_dialog();
    assert!(ui.get_sound_library_open(), "音源管理：弹窗必须打开");
    assert_eq!(
        ui.get_sound_library_focused_control(),
        0,
        "音源管理：入口焦点必须落在第一格（搜索框）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "音源管理：画布层必须不再同时持有键盘"
    );
    // `Tab` ×2：搜索框 → ✕ → 绕回搜索框。
    for expected in [1, 0] {
        live.dispatch_key_press(KeyCode::Tab)
            .expect("音源管理：注入 Tab");
        assert_eq!(
            ui.get_sound_library_focused_control(),
            expected,
            "音源管理：Tab 轮转读数必须是 {expected}"
        );
        assert!(
            !ui.get_shortcut_layer_focused(),
            "音源管理：Tab 轮转中画布层必须不持有键盘"
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
    report_line("modal-entry sound-library: control=0 tab-0-1-0 esc-closed restored");

    // ---- 缺音源救援 ---------------------------------------------------------
    ui.set_missing_sounds_open(true);
    ui.invoke_focus_missing_sounds_dialog();
    assert!(ui.get_missing_sounds_open(), "缺音源：弹窗必须打开");
    assert_eq!(
        ui.get_missing_sounds_focused_control(),
        0,
        "缺音源：入口焦点必须落在第一格（✕）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "缺音源：画布层必须不再同时持有键盘"
    );
    // `Tab` ×2：✕ → 「先不管」→ 绕回 ✕。
    for expected in [1, 0] {
        live.dispatch_key_press(KeyCode::Tab)
            .expect("缺音源：注入 Tab");
        assert_eq!(
            ui.get_missing_sounds_focused_control(),
            expected,
            "缺音源：Tab 轮转读数必须是 {expected}"
        );
        assert!(
            !ui.get_shortcut_layer_focused(),
            "缺音源：Tab 轮转中画布层必须不持有键盘"
        );
    }
    live.dispatch_key_press(KeyCode::Escape)
        .expect("缺音源：注入 Esc");
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
    report_line("modal-entry missing-sounds: control=0 tab-0-1-0 esc-closed restored");
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
    assert_eq!(
        code_app
            .matches("focus-template-save-dialog() { template-save-dialog.focus-first-field(); }")
            .count(),
        1,
        "另存为模板入口必须恰好一条直连"
    );
    assert_eq!(
        code_app
            .matches("focus-sound-library-dialog() { sound-library-dialog.focus-first-control(); }")
            .count(),
        1,
        "音源管理入口必须恰好一条直连"
    );
    assert_eq!(
        code_app
            .matches(
                "focus-missing-sounds-dialog() { missing-sounds-dialog.focus-first-control(); }"
            )
            .count(),
        1,
        "缺音源救援入口必须恰好一条直连"
    );
}

/// 只在**代码行**上数，注释里提到这个名字不算调用点（与 `tests/search_focus_key.rs` ⑤ 同口径）。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
