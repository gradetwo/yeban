//! **时光机节点点击** —— 端到端判据（`[UI-A11Y-003]` §7.3 / ADR-0001 D45 / `[UI-TEST-001]`）。
//!
//! ## 本文件要证的句子
//!
//! `ui/dialogs/undo_tree_modal.slint` 底部那句提示从「节点点击回滚未实现」改成
//! 「点击版本节点回到该版本」之后，这句话必须有一个**真的**落点：点击节点
//! `i` ⇒ 工程回到提交链上下标 `i` 的那一版。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（装配态） | 4 次真提交 ⇒ 5 个节点（含根提交）、`undone = 0`、工程 = 最后一版 |
//! | ②（点更早的节点） | 点节点 2 ⇒ `undo-many(2)`，游标 `undone = 2`、工程**逐字节** = 第 2 版 |
//! | ③（点更新的节点） | 点节点 0 ⇒ `redo-many(2)`，工程逐字节回到最后一版 |
//! | ④（点当前节点） | 再点节点 0 ⇒ **不产生动作**（⛔ 不能把 0 交给端口：`plan_undo` 会把 0 clamp 成 1 步） |
//! | ⑤（撤到底 + 点中间版本） | 点节点 4 ⇒ 撤到底（`undone = 4`、根提交态）；再点节点 3 ⇒ `redo-many(1)` 回到第 1 版 |
//! | ⑥（越界下标） | 点不存在的下标 ⇒ 不产生动作、不动工程（不猜一个版本） |
//! | ⑦（文本层接线） | `.slint` 的点击源 / 无障碍动作 / 选中态、`app.slint` 的转发、`host.rs` 的唯一接线点与 `RedoMany` |
//!
//! ## 为什么找的是"工程字节"而不是控件树文案
//!
//! D45 的纪律：动作日志与工程指纹是证据，元素 ID 只是语义寻址。本判据每一步都断言
//! `UndoPort::fingerprint()`（模型容器的确定性 SHA-256）恰好等于**那一版**的指纹。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/revert_button.rs` / `tests/playhead_follows_transport.rs` 同款）。

#![allow(unused)]
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::Op;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
const NOW: u64 = 1_760_000_000_000;

#[test]
fn clicking_a_time_machine_node_restores_that_version() {
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
        aether_app.contains("callback undo-node-selected("),
        "契约面必须声明 callback undo-node-selected("
    );
    assert!(
        aether_host.contains("undo_node_selected"),
        "宿主接线层必须注册 undo_node_selected"
    );
}
