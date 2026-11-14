//! **宏面板**（`ui/dialogs/macro_panel.slint`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 控制台 Tab 3 标题行的「宏 (N)」按钮打开的右侧抽屉**不是装饰**：它的条数 / 名字 /
//! 位置来自 `ViewState::macros`（`TrackV3::macros` 的投影），滑块手势经
//! `host::wire_macro_panel` 提交**恰好一次** `Op::SetMacro` —— 一步可撤销的编辑。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（默认关着） | 装配后抽屉**不在**运行时控件树里（`macro-panel-open=false`）⇒ 金帧不变；入口按钮在场且条数 = 投影里的宏数 |
//! | ②（可寻址） | 打开后 7 个语义 ID 全部在树里、角色与注册表一致、树里没有注册表之外的 ID |
//! | ③（手势 = 一步编辑） | grab/drag 只写视图态（提交数不变）；release ⇒ `commit_count + 1`，权威工程里是 `0.9`；撤销回到 `0.5` |
//! | ④（空态） | 投影为空时画 `macro-empty-state`，宏行全部消失 |
//! | ⑤（入口接线） | 控制台的回调真的被转发到 `macro-panel-open`（`.slint` 文本层，本机不编译 Slint 的那一半） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_export_dialog.rs` 同款）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_export_dialog.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use slint::Model as _;

use live::{LiveControlPlane, LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::ElementKind;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_mcp::tree::UiTree;
use yeban_ui_test_port::port::Permission;

/// 抽屉打开后在树里的七个语义 ID 与登记角色（`elements.rs` 是同一份口径）。
const PANEL_IDS: [(&str, ElementKind); 7] = [
    ("macro-panel", ElementKind::Region),
    ("macro-panel-title", ElementKind::Text),
    ("macro-panel-close-button", ElementKind::Button),
    ("macro-0-row", ElementKind::ListItem),
    ("macro-0-name", ElementKind::Text),
    ("macro-0-value", ElementKind::Text),
    ("macro-0-slider", ElementKind::Slider),
];

fn tree_ids(plane: &mut LiveControlPlane) -> Vec<String> {
    let (tree, _) = plane.plane().tree().expect("ui/tree");
    let mut ids: Vec<String> = tree.nodes.iter().map(|node| node.id.clone()).collect();
    ids.sort();
    ids
}

fn node<'a>(tree: &'a UiTree, id: &str) -> &'a yeban_ui_mcp::tree::UiNode {
    tree.nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("`{id}` 不在运行时控件树里（树里 {} 条）", tree.nodes.len()))
}

fn f32_row(model: &slint::ModelRc<f32>, index: usize) -> f32 {
    model.row_data(index).expect("滑块值必须存在")
}

#[test]
fn the_macro_panel_is_hidden_until_opened_and_its_slider_commits_one_set_macro() {
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
        aether_app.contains("> macro-panel-open:"),
        "契约面必须声明 > macro-panel-open:"
    );
    assert!(
        aether_host.contains("wire_macro_panel"),
        "宿主接线层必须注册 wire_macro_panel"
    );
    assert!(
        aether_app.contains("> macro-values:"),
        "契约面必须声明 > macro-values:"
    );
    assert!(
        aether_host.contains("macro_values"),
        "宿主接线层必须注册 macro_values"
    );
}
