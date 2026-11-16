//! **Logic Pro 键位收敛（ADR-0074 R1）回归判据**。
//!
//! 背景：键盘曾有两套事实源 —— `app.slint:840` 无条件先调 `trigger-shortcut`
//! （→ `host::on_trigger_shortcut` → `invoke_key_action` → `input.rs` 解析表），
//! 回来后 `app.slint` 的 key-handler 又按 `event.text` 跑一遍界面直达分支。
//! 于是 `Ctrl+T`（切分＋加轨）、`Tab`（换视图＋开 Clip 矩阵）、
//! `Ctrl+Alt+M`（两次取反净零）三键**按一次触发两个动作**。
//!
//! 人类 2026-11-16 确认「参考 Logic Pro」⇒ 采纳 R1：**解析表唯一执行**，
//! 界面层重复分支删除。需人类裁决的语义冲突登记在 `docs/adr/ADR-0074`。
//!
//! ⚠ 偏离如实登记：Logic 的 New Track 是 `Option+Cmd+N`，但本仓该格已是节拍器
//! （10+ 文件 / 2 个专测依赖）⇒ 本轮 NewTrack 暂用空闲的 `Cmd/Ctrl+N`；
//! 严格对齐需把节拍器改 `K`（另立一票），本判据不假装它已是 `Option+Cmd+N`。
//!
//! 纯 `include_str!` 源码判据：零 GUI / 零重依赖，`rustc --test` 可直跑；
//! CI 的 `yeban-app` 腿亦真跑（解析表**行为**由 `input.rs` 既有单测 + CI 覆盖）。

const APP: &str = include_str!("../ui/app.slint");
const INPUT: &str = include_str!("../src/input.rs");
const HOST: &str = include_str!("../src/host.rs");
const CLI: &str = include_str!("../src/cli.rs");

fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ① `Ctrl+T` 在解析表里是**切分**（Logic `Cmd+T`），不是加轨。
#[test]
fn ctrl_t_resolves_to_split_not_track_add() {
    assert!(
        INPUT.contains(
            "(Key::Character('t'), false, false) => Resolution::Action(Action::SplitAtPlayhead)"
        ),
        "解析表必须把 Cmd/Ctrl+T 绑成 SplitAtPlayhead（Logic Cmd+T）"
    );
}

/// ② `Cmd/Ctrl+N` 新建 MIDI 轨有解析表绑定 + 宿主落点 + 快捷键表登记。
#[test]
fn cmd_n_is_wired_to_new_track_end_to_end() {
    assert!(
        INPUT.contains(
            "(Key::Character('n'), false, false) => Resolution::Action(Action::NewTrack)"
        ),
        "Cmd/Ctrl+N 必须解析成 Action::NewTrack（该格此前空闲）"
    );
    assert!(
        HOST.contains("Action::NewTrack =>"),
        "apply_action 必须有 Action::NewTrack 臂，否则键被消费却什么都不做"
    );
    assert!(
        CLI.contains("Cmd/Ctrl+N → 新建 MIDI 合成器轨"),
        "SHORTCUTS 主表必须登记 Cmd/Ctrl+N（--print-shortcuts 的官方口径）"
    );
    assert!(
        CLI.contains("const SHORTCUTS: [ShortcutRow; 59]"),
        "SHORTCUTS 数组长度必须随新增 5 行（解析表收敛键）同步为 59"
    );
}

/// ③ 界面层不再重复执行解析表已处理的 5 个键（双层键路已收敛）。
#[test]
fn app_key_handler_no_longer_duplicates_parsed_keys() {
    let a = code_lines(APP);
    assert!(
        !a.contains("if (event.text == Key.Tab) {"),
        "Tab 的 show-clip-matrix/zone-toggle 分支必须删除（解析表 ToggleView 唯一执行）"
    );
    assert!(
        !a.contains(r#"if (event.text == "x" || event.text == "X") {"#),
        "X 的 apply-dock-tab 分支必须删除（解析表 ToggleMixer 唯一执行）"
    );
    assert!(
        !a.contains(r#"if (event.text == "p" || event.text == "P") {"#),
        "P 的 apply-dock-tab 分支必须删除（解析表 ToggleEditor 唯一执行）"
    );
    assert!(
        !a.contains(
            r#"if ((event.modifiers.control || event.modifiers.meta) && (event.text == "t" || event.text == "T")) {"#
        ),
        "Ctrl+T 的 track-add 分支必须删除（解析表 SplitAtPlayhead 唯一执行）"
    );
    assert!(
        !a.contains(
            r#"if (event.modifiers.alt && (event.modifiers.control || event.modifiers.meta) && (event.text == "m" || event.text == "M")) {"#
        ),
        "Ctrl+Alt+M 的 console-maximize-toggle 分支必须删除（解析表 ToggleConsoleMaximize 唯一执行）"
    );
}

/// ④ 那条与解析表直接矛盾的误导注释必须消失。
#[test]
fn misleading_ctrl_t_comment_is_gone() {
    assert!(
        !APP.contains("Ctrl/Cmd+T 单一语义 = 新建轨道"),
        "该注释与 input.rs 的 SplitAtPlayhead 矛盾，是双绑的温床，必须删除"
    );
}

/// ⑤ 菜单如实标注新建轨的键位（Ctrl+N）。
#[test]
fn new_track_menu_labels_the_shortcut() {
    assert!(
        APP.contains("新建 MIDI 合成器轨 (Ctrl+N)"),
        "菜单项必须标注 Ctrl+N，否则用户不知道新建轨的键盘入口"
    );
}

/// ⑥ 纯界面键（不在解析表）仍保留 —— 收敛不得误删。
#[test]
fn ui_only_keys_are_preserved() {
    let a = code_lines(APP);
    for needle in [
        r#"if (event.text == "i" || event.text == "I") {"#,
        r#"if (event.text == "h" || event.text == "H") {"#,
        "if (event.text == Key.F3) {",
        "if (event.text == Key.F7) {",
        "if (event.text == Key.F8) {",
        "if (event.text == Key.F9) {",
        "if (event.text == Key.F4) {",
        "if (event.text == Key.Escape) {",
    ] {
        assert!(a.contains(needle), "纯界面键分支被误删：{needle}");
    }
}
