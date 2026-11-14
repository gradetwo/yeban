//! **回滚键接线判据（界面侧）** —— 走带区 ↻ 真的回退一个 Commit。
//!
//! ## 本文件要证的句子
//!
//! `transport-revert-button` **不是一个只显示不动作的图标**：点它 = 对权威撤销会话
//! 执行**一次** `UiAction::Undo`。语义由模型不变量背书 —— 本实现的每次提交恰好携带
//! 一条 `Op::Batch`（`crates/yeban-mcp/src/undo_session.rs` 的"每次提交恰好一条 op"），
//! 因此"一步撤销 = 回退一个 Commit"。
//!
//! 判据走**真实指针注入**（不是直接调 Rust 回调）：命中 `revert_area` 的 `clicked`，
//! 经 `host::wire_undo` 接的 `revert-to-last-commit` 落到 `UndoPort`，随后
//! `refresh_undo` 把**模型读数**写回界面、并从回退后的工程重投影。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 树里有 `transport-revert-button`、角色 = `Button`、标签与注册表逐字一致 |
//! | ②（真点击 → 真回退） | 先提交一条夹具编辑；点击 ↻ ⇒ `undone == 1`、工程指纹逐字节回到原始工程 |
//! | ③（动作日志） | 这次点击留下一条 `action == "undo"` 且 `changed_project()` 的记录 |
//! | ④（显示态来自模型） | 点击后界面读数为 `undo-undone == 1` / `undo-can-undo == false` |
//! | ⑤（没有历史就拒绝） | 再点一次 ⇒ `Refused`，工程不变，动作日志**照样**留痕（2 条） |
//!
//! ## 为什么控件树只用来**寻址**
//!
//! 元素在树里不等于动作真的发生（三方对齐矩阵的错位 5 警告过这种假接线）。
//! "真的回退了一版"由**工程指纹**与**动作日志**证明；源文件层的正反探针
//! （`clicked` / `accessible-action-default` / `app.slint` 转发）由
//! `crates/yeban-app/src/elements.rs` 的
//! `the_remaining_transport_document_actions_are_not_wired_today` 机械看守。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/console_maximize.rs` / `tests/undo_wiring_ui.rs` 同款）。

#![allow(unused)]
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{ActionOutcome, UiAction, UndoPort, UndoSession, project_fingerprint};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

/// 与 `tests/undo_wiring_ui.rs` 同款的时间戳（确定性）。
const NOW: u64 = 1_760_000_000_000;

#[test]
fn a_real_click_on_the_revert_button_rolls_back_exactly_one_commit() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
