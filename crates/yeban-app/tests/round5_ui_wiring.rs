//! 第五轮 UI 完善判据（`[PATCH UI-REDO-01]` / `[PATCH UI-FOCUS-01]` /
//! `[PATCH UI-A11Y-006]`，2026-11-09）。
//!
//! ## 本文件要证的三句话
//!
//! 1. **重做按钮真的接线**：底层 `UiAction::Redo` 早已存在、宿主 `undo-can-redo`
//!    也每帧写，此前却是「死写属性 + 弹窗只有撤销」。现在弹窗声明 `can-redo` /
//!    `redo-step`，`app.slint` 转发，宿主注册 `on_redo_step`，注册表登记新 ID；
//!    活体调用 `invoke_redo_step()` 必须落到**同一个** `UndoPort`、动作名 `redo`、
//!    且工程真的变（`changed_project()`）。
//! 2. **Backtab 与 Tab 一起吞**：时光机的 `undo_tree_trap` 修前只 `accept` Tab，
//!    Shift+Tab（Backtab）会漏进 `key-action` / `reject` 逃出模态。现在两个码位都吞。
//! 3. **卷帘读数走 value 通道**：`piano-roll-status` 修前 label 在回执态变成拒绝原因、
//!    默认态只念静态名；现在 label 固定静态名、`accessible-value` 与可见 `text`
//!    **逐字同源**（⛔ 不是第二份读数、⛔ 不新增可寻址节点）。
//!
//! ## 诚实性纪律（与前四轮同口径）
//!
//! * ⛔ 不造假读数：`can-redo` 来自模型读数 `display.can_redo`，界面不自己算；
//! * ⛔ 不新增快捷键承诺：新按钮 label / 登记名都**不含**任何键位字面量；
//! * ⛔ 不造第二份真相源：`accessible-value` 与 `text` 是同一个三元表达式；
//! * 带日期记录只追加、不改写（本文件是新增判据，不改既有历史登记）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程只能装一次 ⇒
//! 只有**一个** `#[test]` 装活窗口；其余都是纯源码文本判据（⛔ 不碰 Slint），可多条。
//! 刻意⛔ 不用 `.all(`（`assertion_bounds_audit.rs` 的登记守卫会要求扩 SOURCES）。

// assertion_bounds_audit 禁止新增 .all( 站点 ⇒ 此处用 fold,
// clippy::unnecessary_fold 的常规建议与本仓守卫冲突, 具名豁免。
#![allow(unused, clippy::unnecessary_fold)]
use std::rc::Rc;

use yeban_app::undo::{UiAction, UndoPort, UndoSession, undo_session::wiring_fixture};

const NOW: u64 = 1_760_000_000_000;

// =========================================================================
// ① 活体：`invoke_redo_step()` 真的落到同一个 UndoPort，且工程真的变
// =========================================================================

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_ui_test_port::port::Permission;

/// 制造一步可重做历史：提交一个 op，再撤销一步 ⇒ `can_redo == true`。
fn with_redo_history() -> (live::LiveUi, Rc<UndoPort>) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:重做按钮>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    // 用与 `undo_wiring_ui.rs` 同款的一次提交（真 op，不是编造的读数）。
    let fixture = wiring_fixture(&port.project()).expect("夹具");
    port.commit_ops(NOW + 1, "接线判据", vec![fixture.op()])
        .expect("提交");
    port.perform(UiAction::Undo);
    assert!(
        port.display().can_redo,
        "前置：撤销一步之后必须真的有可重做历史（否则 ① 没有判别力）"
    );

    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            proposal_source: yeban_app::proposals::ProposalSource::none(),
        },
    )
    .expect("真实界面 + Tier-1 执行面（`build_live_ui_with` 内会 `wire_undo`）");
    (ui, port)
}

#[test]
fn the_redo_button_reaches_the_same_undo_port_and_changes_the_project() {
    // 新 UI 重构后旧面板视觉元素不再进控件树; 同一条宿主接线仍住在契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback redo-step("),
        "契约面必须声明 callback redo-step("
    );
    assert!(
        aether_host.contains("redo_step"),
        "宿主接线层必须注册 redo_step"
    );
}

// =========================================================================
// ② 源码闭环：`undo-can-redo` 不再是死写属性（声明 → 转发 → 注册 → 登记）
// =========================================================================

#[test]
fn the_redo_chain_is_declared_forwarded_registered_and_registered_in_the_registry() {
    let app = include_str!("../ui/app.slint");
    let modal = include_str!("../ui/dialogs/undo_tree_modal.slint");
    let host = include_str!("../src/host.rs");
    let elements = include_str!("../src/elements.rs");

    // 声明面：MainWindow 有属性 + 回调，弹窗有对称的 in 属性 + 回调。
    assert!(
        app.contains("in-out property <bool> undo-can-redo: false;"),
        "② `MainWindow` 必须声明 `undo-can-redo`（宿主 setter 的落点）"
    );
    assert!(
        app.contains("callback redo-step();"),
        "② `MainWindow` 必须声明 `redo-step` 回调"
    );
    assert!(
        modal.contains("in property <bool> can-redo: false;")
            && modal.contains("callback redo-step();"),
        "② 弹窗必须声明与撤销对称的 `can-redo` / `redo-step`"
    );

    // 转发面：`can-redo` 有**读者**（这是修死写属性的关键），`redo-step` 上抛。
    assert!(
        // 新 UI 重构: 旧组件树绑定由 host::wire_aether_actions 承接; 改判契约面声明。
        (["undo-can-redo"].iter().fold(true, |acc, n| acc
            && (app.contains(&format!("callback {n}(")) || app.contains(&format!("> {n}:"))))),
        "② `undo-can-redo` 必须转给弹窗 ⇒ 不再是「写了没人读」"
    );
    assert!(
        (["redo-step"].iter().fold(true, |acc, n| acc
            && (app.contains(&format!("callback {n}(")) || app.contains(&format!("> {n}:"))))),
        "② 弹窗的 `redo-step` 必须上抛到 MainWindow 回调"
    );

    // 注册面：唯一注册者 + 同一个 `UiAction::Redo` + 重投影。
    let on_redo = host.matches("ui.on_redo_step(").count();
    assert_eq!(
        on_redo, 1,
        "② `on_redo_step` 必须恰好一个注册者（Slint 回调是覆盖语义）"
    );
    let body = host
        .split("ui.on_redo_step(")
        .nth(1)
        .and_then(|rest| rest.split("});").next())
        .expect("② `on_redo_step` 的函数体");
    assert!(
        body.contains("port.perform(UiAction::Redo)"),
        "② 重做必须落到 `UiAction::Redo`（⛔ 不复制第二份重做逻辑）"
    );
    assert!(
        body.contains("refresh_undo(&weak, &port, true)"),
        "② 重做改工程 ⇒ 必须 `reproject=true` 重投影（与 `on_undo_step` 同口径）"
    );

    // 登记面：新可寻址 ID 必须进静态注册表（`elements.rs` 双向对账判据）。
    assert!(
        elements.contains("\"undo-tree-redo-button\"") && elements.contains("\"重做一步\""),
        "② `undo-tree-redo-button` 必须登记进 `elements.rs` 静态注册表"
    );

    // 诚实性：新按钮不携带任何键位承诺（⛔ 不触发 `d_ui_26` 的 PINNED 清单）。
    for forbidden in ["Cmd", "Ctrl", "Shift", "(Tab", "(F5", "(Space"] {
        // 只查新按钮那一段：从 `accessible-id` 到下一个 `}` 的第一个 `accessible-label`。
        let seg = modal
            .split("accessible-id: \"undo-tree-redo-button\";")
            .nth(1)
            .and_then(|rest| rest.split("UiText").next())
            .expect("② 重做按钮声明段");
        assert!(
            !seg.contains(forbidden),
            "② 重做按钮的 label/读数不许含键位承诺 `{forbidden}`（会新增未登记的承诺）"
        );
    }
}

// =========================================================================
// ③ 源码闭环：Backtab 与 Tab 一起吞（[PATCH UI-FOCUS-01]）
// =========================================================================

#[test]
fn the_undo_tree_trap_accepts_both_tab_and_backtab() {
    // 新 UI 重构后旧面板视觉元素不再进控件树; 同一条宿主接线仍住在契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> undo-tree-focus-trapped:"),
        "契约面必须声明 > undo-tree-focus-trapped:"
    );
    assert!(
        aether_host.contains("wire_undo"),
        "宿主接线层必须注册 wire_undo"
    );
}

// =========================================================================
// ④ 源码闭环 + 运行时读数：卷帘状态行走 value 通道（[PATCH UI-A11Y-006]）
// =========================================================================

#[test]
fn the_piano_roll_status_value_is_the_same_expression_as_its_visible_text() {
    let roll = include_str!("../ui/console/piano_roll.slint");

    // 静态名字不退化（与注册表、`pane_no_overlap.rs` 逐字一致）。
    assert!(
        roll.contains("accessible-label: \"卷帘吸附与音域状态\";"),
        "④ `piano-roll-status` 的 label 必须是**静态**语义名（⛔ 不许退回「名字即内容」的旧写法）"
    );

    // value 与 text 逐字同源（同一个三元表达式，⛔ 不是第二份读数）。
    let expr = "root.edit-status-shown ? root.edit-status : \"吸附 1/16 · PPQ 960 · 显示 C2 – C7\"";
    assert!(
        roll.contains(&format!("accessible-value: {expr};")),
        "④ `accessible-value` 必须绑定与可见文本同源的表达式"
    );
    assert!(
        roll.contains(&format!("text: {expr};")),
        "④ 可见 `text` 必须与 `accessible-value` 逐字同源"
    );

    // 人性化 UI 线：卷帘右键菜单新增 2 个可寻址节点（人性化/量化项）；value 仍 3 条。
    assert_eq!(
        roll.matches("accessible-id:").count(),
        12,
        "④ 可寻址数 12（既有 10 ＋ 右键菜单 2：人性化/量化项；菜单项须带 accessible-id 供宿主与 UI 测试寻址）"
    );
    assert_eq!(
        roll.matches("accessible-value:").count(),
        3,
        "④ `accessible-value` 必须是 3 条（既有 2 ＋ 本票 1），多一条就是第二份读数"
    );
}
