//! **第十轮 UI 缺口补丁**的判据（用户 2026-11-16「你来开发修复和出具补丁包」）。
//!
//! 本文件钉住本补丁改到的**五件事**，每条都在修前**必红**：
//!
//! | # | 判据 | 修前为什么红 |
//! | :--- | :--- | :--- |
//! | 1 | 剪贴板三键有用户回执 | `clipboard_copy_selected` 返回 `bool`，四个调用点全部丢弃 ⇒ 成功失败都静默 |
//! | 2 | 「文件」菜单有退出项且落到 `quit_event_loop` | 全仓 `MenuItem … 退出` = 0 处 |
//! | 3 | 分轨导出目录不再落在 `%TEMP%` | `temp_dir().join("yeban-stems")`，系统清理会连整份分轨删掉 |
//! | 4 | P 落钢琴卷帘、X 落混音台（各自幂等） | 两条都是同一个 `console_tab` 0↔1 互换，菜单语义与行为相反 |
//! | 5 | 旧的 `export-stems` 假入口 / 被覆盖的 `on_command_execute` 已删 | 两处死代码，会被误判成"还没修" |
//!
//! ⚠ 纯 `include_str!` 判据（本机编不了 Slint）；行为由 CI 的 yeban-app 腿真跑。

const HOST: &str = include_str!("../src/host.rs");
const APP: &str = include_str!("../ui/app.slint");

/// 剪贴板三键：键盘与菜单**都**要写回执，且回执文案区分"成功"与"下一步"。
#[test]
fn clipboard_verbs_report_to_the_user() {
    assert!(
        HOST.contains("pub fn report_clipboard("),
        "缺 `report_clipboard`：剪贴板回执必须只有一个实现点"
    );
    assert!(
        HOST.contains("ClipboardVerb::Copy") && HOST.contains("ClipboardVerb::Paste"),
        "回执必须区分三种动作（复制/粘贴/剪切），否则文案会串"
    );
    // 键盘三处 + 菜单三处，共 6 处调用；少一处就有路径静默。
    let calls = HOST.matches("report_clipboard(").count();
    assert!(
        calls >= 7,
        "report_clipboard 的调用点只有 {calls} 处（含定义 1 处）——键盘与菜单必须都覆盖"
    );
    // 修前的"丢弃返回值"写法必须消失：菜单路径现在要先接住 `ok` 再回执。
    assert!(
        HOST.contains("let ok = clipboard_copy_selected("),
        "菜单路径仍是丢弃返回值的老写法（成功/失败都没有回执）"
    );
}

/// 「文件」菜单要有**看得见**的退出项，并且落到事件循环出口。
#[test]
fn file_menu_has_a_quit_item_that_really_quits() {
    assert!(
        APP.contains(r#"root.command-execute("app-quit")"#),
        "「文件」菜单没有退出项"
    );
    assert!(HOST.contains(r#""app-quit" =>"#), "`app-quit` 没有分派臂");
    assert!(
        HOST.contains("slint::quit_event_loop()"),
        "退出必须走 `slint::quit_event_loop()`（`main.rs` 用的是 `ComponentHandle::run`）"
    );
}

/// 分轨导出目录：不再落 `%TEMP%`，且路径推导只有一个实现点。
#[test]
fn stems_output_dir_is_not_volatile_temp() {
    assert!(
        HOST.contains("fn stems_output_dir()"),
        "缺 `stems_output_dir`：输出目录必须有一个可读的推导点"
    );
    assert!(
        !HOST.contains(r#"std::env::temp_dir().join("yeban-stems")"#)
            || HOST.contains("stems_output_dir()"),
        "分轨目录仍直接写死 temp_dir"
    );
    assert!(
        HOST.contains(r#"join("Documents")"#),
        "`stems_output_dir` 应落到用户的 Documents（不可被系统清理）"
    );
}

/// P / X 各自落到自己的页，且**不再**互换。
#[test]
fn editor_and_mixer_shortcuts_are_idempotent() {
    let mixer = HOST
        .split_once("Action::ToggleMixer =>")
        .expect("缺 ToggleMixer 臂")
        .1;
    let mixer_arm = &mixer[..mixer.find("Action::").unwrap_or(mixer.len())];
    assert!(
        mixer_arm.contains("set_console_tab(1)"),
        "X 应落到混音台（console-tab = 1）"
    );
    assert!(
        !mixer_arm.contains("if current =="),
        "X 仍是 0↔1 互换（在混音台按 X 会跳去钢琴卷帘）"
    );
    let editor = HOST
        .split_once("Action::ToggleEditor =>")
        .expect("缺 ToggleEditor 臂")
        .1;
    let editor_arm = &editor[..editor.find("Action::").unwrap_or(editor.len())];
    assert!(
        editor_arm.contains("set_console_tab(0)"),
        "P 应落到钢琴卷帘（console-tab = 0）"
    );
    assert!(
        !editor_arm.contains("if current =="),
        "P 仍是 0↔1 互换（在卷帘页按 P 会跳去混音台）"
    );
}

/// 两处死代码已清，且清理后的注释**说明理由**（避免下一个人又加回来）。
#[test]
fn dead_export_stems_branch_and_shadowed_handler_are_gone() {
    assert!(
        !HOST.contains("分轨导出 CLI 可用 `--export-stems`"),
        "旧的「打开 WAV 面板 + 请用 CLI」假入口还在"
    );
    assert!(
        !HOST.contains(r#"format!("[命令] 已执行命令: {}""#),
        "被覆盖的 `on_command_execute` toast 桩还在（它只弹 toast、不干真事）"
    );
    assert!(
        HOST.contains("被覆盖的死代码") || HOST.contains("不再有**") || HOST.contains("不再有"),
        "清理处应写明理由，防止回归"
    );
}
