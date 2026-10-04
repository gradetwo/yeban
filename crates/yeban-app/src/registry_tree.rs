//! `ElementRegistry` → `yeban_ui_test_port::tree::ControlTree` 的**唯一**转换实现。
//!
//! ## 为什么这个文件不在 `lib.rs` 的模块表里
//!
//! 它需要 `yeban-ui-test-port`，而那个依赖在 `crates/yeban-app/Cargo.toml` 里只是
//! `[dev-dependencies]`（内省能力不进默认 release 构建，`AGENTS.md` §2 红线 6）。
//! 因此本文件**只**被测试目标用 `#[path]` 装进去：
//!
//! ```ignore
//! #[path = "../src/registry_tree.rs"]
//! mod registry_tree;
//! ```
//!
//! 于是仓库里这 9 行转换只有一份实现，两个使用方（真实接线
//! `src/live_surface.rs` 与判据）共用它；本机零 Slint 探针也用**同一个文件**
//! （见 `docs/ledger/live-port-notes.md` 的"本机真跑"一节）。
//!
//! ## 它是纯函数（零 Slint、零 I/O）
//!
//! 静态注册表**没有几何**：`bounds` 一律 `None`、`parent` 一律 `None`（不编造数字）。
//! `LivePort::new` 会用它把"哪些节点是高频刷新区"注入到**运行时**树上 ——
//! 运行时读不到这种业务知识，而 `[UI-MCP-002]` 的遮罩需要它。

use yeban_app::elements::ElementRegistry;
use yeban_ui_test_port::tree::{ControlNode, ControlTree, Role, TreeError};

/// 把 `yeban-app` 的静态语义元素注册表适配成 test-port 的控件树。
///
/// # Errors
///
/// 注册表里出现 `yeban_ui_test_port::tree::KNOWN_ROLES` 之外的 `accessible-role`
/// （⇒ [`TreeError`]），或 ID 重复/格式非法。
pub fn control_tree_from_registry(registry: &ElementRegistry) -> Result<ControlTree, TreeError> {
    let mut tree = ControlTree::new();
    for meta in registry.iter() {
        let role = Role::parse(meta.kind.accessible_role())?;
        let node = if meta.dynamic_region {
            ControlNode::new(meta.id.clone(), role, meta.label.clone()).as_dynamic()
        } else {
            ControlNode::new(meta.id.clone(), role, meta.label.clone())
        };
        tree.insert(node)?;
    }
    Ok(tree)
}
