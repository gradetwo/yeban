//! 判据：命令面板 `tab-stacked` 条目的 shortcut 文案 `F3` 必须是真绑的那一把（T-FIX-377）。
//!
//! 三面钉死，缺一面即谎：
//! 1. **文案面**：`ui/modals.slint` 里 `tab-stacked` 的 `shortcut:` 字段就是 `"F3"`；
//! 2. **解析面**：`F3` 经 `input.rs` 两条入口（物理码 / 逻辑键）都解出
//!    `SelectConsoleTab { index: 3 }`（下标对上 `scene::CONSOLE_TABS[3]`「设备堆叠」）；
//! 3. **界面面**：`ui/app.slint` 的 `key-handler` 里 `F3` 臂直达 `apply-dock-tab("stacked")`，
//!    不再是旧臂的混音控制台最大化。
//!
//! 背景：此前只有文案面是真的 —— `input.rs` 全文无 `F3` 臂（`resolve_logical` 落到
//! `PassThrough`），而界面层 `F3` 臂做的是「混音控制台最大化」，两层与文案说的都不是
//! 同一个动作。`--print-shortcuts` 的主表与「界面直达键」段也同步改成了设备堆叠
//! （`tests/ui_panel_naming.rs` 判据 ④ 与 `tests/ui_keyboard_menu_consistency.rs`
//! 判据 ①/⑤ 各钉一面）。
//!
//! 本文件只读源码文本 + 纯逻辑解析，⛔ 不装配 Slint 平台、不开活窗口（与 d_ui_26 同款）。

use yeban_app::host::action_has_implementation;
use yeban_app::input::{
    Action, Focus, InputContext, LogicalKey, Modifiers, PhysicalKey, Resolution,
};

/// 命令面板 `tab-stacked` 条目的 `shortcut:` 字段值。
fn tab_stacked_shortcut() -> String {
    let modals = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/modals.slint"),
    )
    .expect("读 modals.slint");
    let line = modals
        .lines()
        .find(|l| l.contains("id: \"tab-stacked\""))
        .expect("命令面板必须有 tab-stacked 条目");
    let after = line
        .split("shortcut:")
        .nth(1)
        .expect("tab-stacked 条目必须有 shortcut 字段");
    let start = after.find('"').expect("shortcut 值必须有引号") + 1;
    let end = after[start..].find('"').expect("shortcut 值必须闭合引号") + start;
    let shortcut = after[start..end].to_owned();
    assert!(
        !shortcut.is_empty(),
        "tab-stacked 的 shortcut 字段不得为空：{line}"
    );
    shortcut
}

fn canvas() -> InputContext {
    let mut ctx = InputContext::new();
    ctx.set_focus(Focus::MainCanvas);
    ctx
}

/// ① 文案面：`tab-stacked` 的 shortcut 文案就是 `F3`。
#[test]
fn tab_stacked_shortcut_text_is_f3() {
    assert_eq!(tab_stacked_shortcut(), "F3");
}

/// ② 解析面：`F3` 两条入口同解 `SelectConsoleTab { index: 3 }`，且宿主认领它。
#[test]
fn f3_resolves_to_the_stacked_console_tab() {
    assert_eq!(
        yeban_app::scene::CONSOLE_TABS[3].name,
        "stacked",
        "下标 3 必须是 scene::CONSOLE_TABS 的「设备堆叠」"
    );
    assert_eq!(
        canvas().resolve(PhysicalKey::F3, Modifiers::none()),
        Resolution::Action(Action::SelectConsoleTab { index: 3 }),
        "物理码入口：F3 必须解出下标 3"
    );
    assert_eq!(
        canvas().resolve_logical(
            LogicalKey::from_text("\u{f706}").expect("F3 的 Slint 私有区文本"),
            Modifiers::none()
        ),
        Resolution::Action(Action::SelectConsoleTab { index: 3 }),
        "逻辑键入口：F3 文本必须解出下标 3"
    );
    assert!(
        action_has_implementation(Action::SelectConsoleTab { index: 3 }),
        "宿主必须认领 SelectConsoleTab {{ index: 3 }}（否则 --print-shortcuts 主表在说谎）"
    );
}

/// ③ 界面面：`key-handler` 的 `F3` 臂直达「设备堆叠」，不再碰最大化。
#[test]
fn f3_key_handler_arm_applies_the_stacked_tab() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let code: String = app
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let f3_start = code
        .find("if (event.text == Key.F3)")
        .expect("key-handler 必须有 F3 臂");
    let f3_end = code[f3_start..]
        .find("if (event.text == Key.F7)")
        .map(|offset| f3_start + offset)
        .expect("F3 臂之后必须有 F7 臂");
    let f3_arm = &code[f3_start..f3_end];
    assert!(
        f3_arm.contains("apply-dock-tab(\"stacked\")"),
        "F3 臂必须直达「设备堆叠」页签"
    );
    assert!(
        !f3_arm.contains("console-maximize-toggle"),
        "F3 臂不得再碰控制台最大化（旧臂的行为；最大化走 Ctrl+Alt+M 与最大化按钮）"
    );
}
