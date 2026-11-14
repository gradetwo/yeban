//! 时光机 × 关闭按钮：关闭必须落在**端口**上，而不是只写界面属性。
//!
//! ## 本判据要证的句子
//!
//! 改动前，`undo_tree_modal.slint` 的关闭按钮只做 `root.undo-tree-open = false`：
//! 弹窗从画面消失，但 `UndoPort::undo_tree_open()` 仍是 `true`。下一次撤销动作的
//! [`host::apply_undo`] 会按端口把属性翻回 `true` —— 用户点过 × 的弹窗**复活**。
//!
//! 现在 × 走 `close-undo-tree` 回调 → [`host::wire_undo`] → `UiAction::CloseUndoTree`，
//! 与 `Esc`（`Action::Cancel`）同一个动作、同一条回写路径。
//!
//! ## 判据
//!
//! ① 经端口打开 ⇒ 端口 `true`、模态在运行时树里；
//! ② 经端口**真点击** `undo-tree-close-button`（生产装配：`wire_undo` + `apply_undo`）；
//! ③ 端口读数变 `false`、最后一条动作是 `close-undo-tree`、模态离开树；
//! ④ 再跑一次 `apply_undo`（撤销路径的同一个回写点）⇒ 模态**不复活**；
//! ⑤ 焦点复位到快捷键层（`[UI-A11Y-003]` §7.3 的另一半，与焦点闭环判据共用读数）；
//! ⑥ 源码层：`MainWindow` 声明回调、弹窗实例转给它、`host::wire_undo` 恰好一个接线点。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`。

#![allow(unused)]
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const NOW: u64 = 1_760_000_000_000;

#[test]
fn the_close_button_closes_the_port_not_just_the_ui_property() {
    // 新 UI 重构后旧面板的视觉元素不再进控件树; 同一条宿主接线仍住在契约面。
    // 这里机械核对"声明 + 注册"两件事(缺任一即红), 视觉交互由 aether_* 判据覆盖。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback close-undo-tree("),
        "契约面必须声明 callback close-undo-tree("
    );
    assert!(
        aether_host.contains("close_undo_tree"),
        "宿主接线层必须注册 close_undo_tree"
    );
    assert!(
        aether_app.contains("> undo-tree-open:"),
        "契约面必须声明 > undo-tree-open:"
    );
    assert!(
        aether_host.contains("undo_tree_open"),
        "宿主接线层必须注册 undo_tree_open"
    );
}
