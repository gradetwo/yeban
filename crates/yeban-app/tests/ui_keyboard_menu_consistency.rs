//! 键盘 / 菜单 / 键位表一致性的源码层看守（控件逐一体检 @ `3ff6914`）。
//!
//! 那份报告 §2 的核心发现是"测试打 `input.rs` 的解析表，真实窗口里
//! Slint 的 `key-handler` 会在解析之后再插一脚"：F7/F8 一律打开最大化
//! 钢琴卷帘、`[D]`/`[F4]` 提示承诺了不存在的绑定、`--print-shortcuts`
//! 漏掉一半实际生效的单键、菜单缺「另存为」且命名不统一。
//!
//! 本文件把这些修复**钉在源码层**（与 `ui_display_honesty` 同款：只读
//! `.slint` / `.rs` 文本，⛔ 不编译 Slint）。任一处被静默改回去对应判据
//! 立即变红。`cli_contract` 的一批占位判据依赖 `app.slint` /
//! `host.rs` 里的两条契约声明，判据 ⑧ 顺手钉住别被删。

#![allow(unused)]

const APP: &str = include_str!("../ui/app.slint");
const HOST: &str = include_str!("../src/host.rs");
const CLI: &str = include_str!("../src/cli.rs");
const LOWER_DOCK: &str = include_str!("../ui/lower_dock.slint");
const TRANSPORT_BAR: &str = include_str!("../ui/transport_bar.slint");

/// 滤掉整行注释后剩下的源码行（注释里解释"为什么改"必然提到旧标识符）。
fn code_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **判据 ①**：F7/F8/F9 按文档直达三个控制台页签（控件体检 §2.2）。
#[test]
fn f7_f8_f9_open_the_three_documented_console_tabs() {
    let s = code_lines(APP);
    assert!(
        !s.contains("Key.F7 || event.text == Key.F8"),
        "F7/F8 不应再被合并成「一律打开最大化钢琴卷帘」的一个分支"
    );
    assert!(s.contains("if (event.text == Key.F7)"));
    assert!(s.contains("if (event.text == Key.F8)"));
    assert!(s.contains("if (event.text == Key.F9)"));
    assert!(s.contains("apply-dock-tab(\"pianoroll\")"));
    assert!(s.contains("apply-dock-tab(\"mixer\")"));
    assert!(s.contains("apply-dock-tab(\"devices\")"));
}

/// **判据 ②**：页签 tooltip 承诺的 `[D]` / `[F4]` 真的有绑定（控件体检 §2.3）。
#[test]
fn d_and_f4_tooltips_have_real_bindings() {
    let app = code_lines(APP);
    assert!(app.contains("Key.F4"), "F4 必须绑定监听控制室");
    assert!(
        app.contains("event.text == \"d\" || event.text == \"D\""),
        "裸 d 必须绑定设备机架页签"
    );
    assert!(LOWER_DOCK.contains("[D]"), "[D] 提示仍在（绑定已兑现）");
    assert!(
        TRANSPORT_BAR.contains("[F4]"),
        "[F4] 提示仍在（绑定已兑现）"
    );
}

/// **判据 ③**：「另存为」从菜单到宿主都有落点（控件体检 §3 / §7）。
#[test]
fn save_as_is_wired_from_menu_to_host() {
    assert!(code_lines(APP).contains("root.command-execute(\"file-save-as\")"));
    assert!(
        code_lines(HOST).contains("\"file-save-as\" => ui.invoke_save_project_as(),"),
        "dispatch_aether_command 必须有 file-save-as 落点，否则菜单是死项"
    );
}

/// **判据 ④**：菜单文案用平台无关的 `Ctrl+`，不再 `Cmd/Ctrl+` 混排（报告 §5.11 / §2.5）。
#[test]
fn menu_labels_use_platform_neutral_ctrl_prefix() {
    for line in code_lines(APP).lines().filter(|l| l.contains("MenuItem")) {
        assert!(
            !line.contains("Cmd/Ctrl+"),
            "菜单项仍有 Mac 混排写法：{line}"
        );
    }
}

/// **判据 ⑤**：`--print-shortcuts` 如实列出界面直达键（控件体检 §2.4）。
#[test]
fn printed_shortcuts_list_the_ui_direct_keys() {
    let c = code_lines(CLI);
    assert!(c.contains("界面直达键"));
    // ⭐ ADR-0074 R1 之后，X/E/P/C/Tab/Ctrl+T/Ctrl+Alt+M 由解析表唯一执行，
    // 不再出现在本段（改由 SHORTCUTS 主表覆盖）；此处只剩纯界面键。
    for key in ["I", "H", "T", "D", "F3", "F4", "F7 / F8 / F9", "Escape"] {
        assert!(
            c.contains(&format!("#   {key}")),
            "--print-shortcuts 的界面直达键段漏了 {key}"
        );
    }
}

/// **判据 ⑥**：同一块面板命名统一（报告 §5.12）。
#[test]
fn device_rack_menu_naming_is_consistent() {
    let s = code_lines(APP);
    assert!(!s.contains("DSP 效果器机架"));
    assert!(!s.contains("打开底部 设备机架 (DSP)"));
    assert!(s.contains("title: \"设备机架\""));
}

/// **判据 ⑦**：`on_trigger_shortcut` 不再与界面分支重复处理功能键。
#[test]
fn trigger_shortcut_no_longer_duplicates_function_key_handling() {
    let h = code_lines(HOST);
    assert!(
        !h.contains("text == \"F3\" || text == \"F7\" || text == \"F8\""),
        "F3/F7/F8 的重复 maximize-toggle 会让界面分支两次取反相互抵消"
    );
}

/// **判据 ⑧**：新 UI 契约面声明仍在（`cli_contract` 的占位判据依赖）。
#[test]
fn aether_contract_declarations_are_preserved() {
    assert!(APP.contains("callback accept-ai-proposal("));
    assert!(HOST.contains("on_accept_ai_proposal"));
}
