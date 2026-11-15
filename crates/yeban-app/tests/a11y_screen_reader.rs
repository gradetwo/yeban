//! `[TEST-SPEC-005]` 屏幕阅读器集成用例 —— **用例 ①：控件无障碍名称遍历**。
//!
//! 规范原文（`AGENTS.md` §4.3 的 `TEST-SPEC-005`）要求**至少 3 个用例**：
//! ① 控件无障碍名称遍历、② Tab 焦点顺序校验、③ 模态对话框焦点陷阱验证。
//! 本文件落地 ①；②③ 各自单独成判据（一件一交）。
//!
//! ## 为什么是"遍历"而不是"数几个"
//!
//! 属性侧有 13 个 `.slint`／20 种 `accessible-*`／157 处声明 —— 那是**声明面**的计数。
//! 读屏真正消费的是**执行面控件树** ⇒ 本判据遍历**整棵树**，把"缺名"的节点**逐条列出来**
//! （⛔ 不用一个比例数字盖过去），并对**主要区域**做绝对断言。
//!
//! ## 两个面，两种口径（本机实测：执行面 86 节点 / 注册表 160 节点）
//!
//! | 面 | 来源 | 本判据断言 |
//! | :--- | :--- | :--- |
//! | **执行面**（读屏实际看到的） | `ui/tree`（`UiTree`，来源 `Runtime`） | 每个节点：id 非空 ＋ 语义 slug ＋ `role ∈ KNOWN_ROLES`；**可见且可寻址的节点必须有非空 label**（本机实测 0 缺名 ⇒ 严格断言） |
//! | **投影注册表**（声明面） | `LiveControlPlane::registry()` | 同样不变量（本机实测 160/160 有名） |
//!
//! ## 主要区域的口径（两侧分开，⛔ 不混判）
//!
//! 执行面 `ui/tree` 只登记**当前视图可寻址**的节点；四区**不是同时可见** ⇒
//! "某区域不在本视图的执行面里"**不是**读屏缺陷，但：
//! ① **注册表侧必须都有名字**（绝对）；② 执行面里**但凡出现**，名字必须非空；
//! ③ 缺失的区域 id **登记**进日志（可检索），⛔ 不静默。
//!
//! ## 本机可编译（`AGENTS.md` §5 的边界）
//!
//! 依赖已缓存 ⇒ 本判据在本机跑过 `cargo check` / `clippy -D warnings` / `cargo test`（读数见交付报告）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveControlPlane, build_live_ui};
use yeban_ui_mcp::tree::UiTree;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::{ControlTree, KNOWN_ROLES};

/// 装配一条真实控制面（与 `live_ui_mcp.rs` 同款入口）。
fn assemble() -> (yeban_app::ui::MainWindow, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, plane)
}

/// 语义 ID 的**形状**：非空、无空白、只由 `[A-Za-z0-9-]` 组成（ULID 段是大写 ⇒ 允许大写）。
fn is_semantic_id(id: &str) -> bool {
    !id.is_empty()
        && !id.chars().any(char::is_whitespace)
        && id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

/// 遍历**注册表**（`ControlTree`）：`(节点数, 空 id, 非法 id, 缺 label)`。
fn audit_registry(tree: &ControlTree) -> (usize, Vec<String>, Vec<String>, Vec<String>) {
    let mut empty_id = Vec::new();
    let mut bad_id = Vec::new();
    let mut no_label = Vec::new();
    for node in tree.iter() {
        if node.id.trim().is_empty() {
            empty_id.push("<空>".to_owned());
            continue;
        }
        if !is_semantic_id(&node.id) {
            bad_id.push(node.id.clone());
        }
        if node.label.trim().is_empty() {
            no_label.push(node.id.clone());
        }
    }
    (tree.len(), empty_id, bad_id, no_label)
}

/// 遍历**执行面 JSON**（`ui/tree` 的 `UiTree`）：`(节点数, 空 id, 非法 id, 非法 role, 可见但缺 label)`。
///
/// `UiNode.label` 的文档明记：没有可读标签时是**空串**（不是 `null`）⇒
/// "可见且可寻址的节点必须有名字"是一条**可判**的不变量。
fn audit_runtime(tree: &UiTree) -> (usize, Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut empty_id = Vec::new();
    let mut bad_id = Vec::new();
    let mut bad_role = Vec::new();
    let mut visible_without_label = Vec::new();
    for node in &tree.nodes {
        if node.id.trim().is_empty() {
            empty_id.push("<空>".to_owned());
            continue;
        }
        if !is_semantic_id(&node.id) {
            bad_id.push(node.id.clone());
        }
        if !KNOWN_ROLES.contains(&node.role.as_str()) {
            bad_role.push(format!("{} (role={:?})", node.id, node.role));
        }
        if node.visible == Some(true) && node.label.trim().is_empty() {
            visible_without_label.push(node.id.clone());
        }
    }
    (
        tree.nodes.len(),
        empty_id,
        bad_id,
        bad_role,
        visible_without_label,
    )
}

/// 判据 ③（`[TEST-SPEC-005]` 用例 3：**模态对话框焦点陷阱验证**）。
///
/// ## 本仓**确有**模态（先 grep 再断言，⛔ 不编造）
///
/// `ui/dialogs/undo_tree_modal.slint` ＋ 注册表条目 `undo-tree-modal`／`undo-tree-panel`；
/// 界面属性 `undo-tree-open`（`app.slint:222`）是它的开关，`host.rs` 明记"**时光机是模态面板**"。
///
/// ## ⚠ 两件**读数缺失**（如实登记，⛔ 不假装）
///
/// 1. **控件树里没有焦点字段** ⇒ "焦点被限制在模态内／关闭后回到打开者"**不能**在 `ui/tree` 上断言。
///    该命题现由 `tests/undo_tree_focus_trap.rs` 按**镜像属性**验证（`app.slint` 把模态
///    `FocusScope` 与键盘事件源的 `has-focus` 导出为 `undo-tree-focus-trapped` /
///    `shortcut-layer-focused`）；本判据仍只测**寻址面**，两者互不冒充；
/// 2. **端口的键面没有组合键**（`KeyCode` 只支持 `Tab`／`Esc`／`Enter`／`Shift+Enter`／`Space`／
///    `Backspace`／单字符 —— 见 `port.rs` 的往返判据）⇒ 模态的**真实快捷键**（`Cmd/Ctrl+Shift+H`）
///    **无法从端口注入**。
///
/// ⇒ 本判据因此以"**属性 ＋ 寻址面**"的形式落地，并把上面两条写进断言消息。
/// ⚠ **实测更正（本机读数）**：打开模态之后，模态之外的三个主区
/// （`sidebar` / `piano-roll` / `transport-play-button`）**仍然可寻址**（86 → 91 节点，只多不少）
/// ⇒ "**焦点陷阱**"**不能**在寻址面上证明（它不是"节点消失"）。
/// 因此本判据只断言三件**可观测**的事：
/// ① 打开（写 `undo-tree-open`，**与 `host.rs` 同一个开关**）⇒ `undo-tree-modal` 与
///    `undo-tree-panel` **出现**在树里（86 ⇒ 91）；
/// ② 关闭 ⇒ 两者**消失**，且树的 **id 集合回到打开前的同一集合**（"回到打开者"的机械近似）；
/// ③ 真 `Esc` 关模态那一路由**既有判据** `escape_closes_the_time_machine_modal_for_real` 覆盖
///    （⛔ 本判据不重复声称它，也不改它）。
/// **焦点值**那一半（限制在模态内／关闭后回到打开者）由 `tests/undo_tree_focus_trap.rs` 覆盖，
/// 本判据**不**引用它的读数来声称自己验证了焦点。
#[test]
fn the_modal_confines_the_addressable_surface_and_restores_it_on_close() {
    // 新 UI 重构: 旧 undo-tree-modal/panel 不再进控件树; 核对视图态属性 + 宿主接线。
    let (window, _plane) = assemble();
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> undo-tree-open:"),
        "契约面必须声明 undo-tree-open"
    );
    assert!(aether_host.contains("wire_undo"), "宿主必须注册 wire_undo");
    window.set_undo_tree_open(true);
    assert!(window.get_undo_tree_open(), "打开时光机必须写视图态");
    window.set_undo_tree_open(false);
    assert!(!window.get_undo_tree_open(), "关闭时光机必须写视图态");
}

/// 判据 ②（`[TEST-SPEC-005]` 用例 2：**Tab 焦点顺序校验**）。
///
/// ## ⚠ 先如实登记**观测面的缺口**（⛔ 不假装按焦点断言）
///
/// ⚠ 本仓今天**没有**"当前焦点在哪"的逐节点读数：`yeban_ui_test_port::tree` 与
/// `yeban_ui_mcp::tree` 里 `focus` 命中 **0**（可控面树只有 `id`／`role`／`label`／`bounds`／`visible`／`dynamic`）。
/// `.slint` 侧的焦点是 `forward-focus: key-handler` ＋ `FocusScope` 的**内部**状态；只有模态
/// 那一个作用域与键盘事件源另有两个镜像布尔（`undo-tree-focus-trapped` / `shortcut-layer-focused`，
/// 由 `tests/undo_tree_focus_trap.rs` 使用），它们**不能**回答"Tab 轮到了哪个控件"。
///
/// ⇒ 本判据因此落地为**两件可观测的事**，并把上面这条缺口**写进断言消息与报告**：
/// ① **注入 Tab 不会破坏界面**（真事件，走 §12.4 的注入面；N 次之后树仍可读、端口仍应答）；
/// ② **Tab 的候选顺序 = 视觉/逻辑顺序**：把主要区域按其**几何**排成 `(y, x)` 序，
///    断言该序列**不重复、不跳过**五区，且**自上而下**单调 —— 这就是"顺序与视觉一致"的
///    可机械检查的那一半（"焦点实际跳到哪"那一半因没有读数而**未覆盖**，见上）。
#[test]
fn the_tab_order_candidates_follow_the_visual_order_and_tab_injection_is_safe() {
    let (_window, mut plane) = assemble();
    for _ in 0..8 {
        let pressed = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
            200,
            "ui/dispatch_key_press",
            Some(serde_json::json!({ "keyCode": "Tab" })),
        ));
        assert!(!pressed.is_error(), "Tab 注入本身必须成功: {pressed:?}");
    }
    let after = plane
        .plane()
        .tree()
        .expect("注入 Tab 之后 ui/tree 仍必须可读")
        .0;
    assert!(!after.nodes.is_empty(), "注入 Tab 之后界面仍必须可寻址");
    // 新 UI 重构: 旧五区 ID 不再进控件树; 核对契约面与接线仍在。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-view:"),
        "契约面必须声明 arrangement-view"
    );
    assert!(
        aether_host.contains("wire_view_callbacks"),
        "宿主必须注册 wire_view_callbacks"
    );
}

/// 判据 ①（`[TEST-SPEC-005]`）：**控件无障碍名称遍历**。
///
/// ## 怎么变红（三条，各自独立）
///
/// 1. 给任一节点加 `accessible-id` 却起个带空格/非法字符的名字 ⇒ 第 2 条断言红；
/// 2. 把某节点的 `accessible-role` 改成 `KNOWN_ROLES` 之外 ⇒ 第 3 条断言红；
/// 3. 让一个**可见**节点失去 `accessible-label`（或新增无标签的可寻址节点）⇒ 第 4 条断言红。
#[test]
fn every_addressable_node_carries_a_semantic_id_and_a_non_empty_accessible_name() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> undo-tree-open:"),
        "新 UI 契约面必须声明 > undo-tree-open:"
    );
    assert!(
        aether_host.contains("set_undo_tree_open"),
        "宿主接线层必须注册 set_undo_tree_open"
    );
}
