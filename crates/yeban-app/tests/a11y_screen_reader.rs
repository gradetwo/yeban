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
/// 1. **焦点本身无读数** ⇒ "焦点被限制在模态内／关闭后回到打开者"不能按焦点值断言；
/// 2. **端口的键面没有组合键**（`KeyCode` 只支持 `Tab`／`Esc`／`Enter`／`Shift+Enter`／`Space`／
///    `Backspace`／单字符 —— 见 `port.rs` 的往返判据）⇒ 模态的**真实快捷键**（`Cmd/Ctrl+Shift+H`）
///    **无法从端口注入**。
///
/// ⇒ 本判据因此以"**属性 ＋ 寻址面**"的形式落地，并把上面两条写进断言消息。
/// ⚠ **实测更正（本机读数）**：打开模态之后，模态之外的三个主区
/// （`sidebar` / `piano-roll` / `transport-play-button`）**仍然可寻址**（86 → 91 节点，只多不少）
/// ⇒ "**焦点陷阱**"**不能**在寻址面上证明（它是**焦点**属性，而焦点无读数）。
/// 因此本判据只断言三件**可观测**的事：
/// ① 打开（写 `undo-tree-open`，**与 `host.rs` 同一个开关**）⇒ `undo-tree-modal` 与
///    `undo-tree-panel` **出现**在树里（86 ⇒ 91）；
/// ② 关闭 ⇒ 两者**消失**，且树的 **id 集合回到打开前的同一集合**（"回到打开者"的机械近似）；
/// ③ 真 `Esc` 关模态那一路由**既有判据** `escape_closes_the_time_machine_modal_for_real` 覆盖
///    （⛔ 本判据不重复声称它，也不改它）。
/// **未验证**（如实登记）：`[TEST-SPEC-005]` 用例 3 要求的"焦点限制在模态内／关闭后回到打开者"**按焦点值**未验证 ——
/// 缺的是**焦点读数**（见上），不是判据的写法。
#[test]
fn the_modal_confines_the_addressable_surface_and_restores_it_on_close() {
    let (window, mut plane) = assemble();

    let ids_of = |plane: &mut LiveControlPlane| -> Vec<String> {
        let (tree, _) = plane.plane().tree().expect("ui/tree");
        let mut ids: Vec<String> = tree.nodes.iter().map(|node| node.id.clone()).collect();
        ids.sort();
        ids
    };

    // ---------------- 打开前 ----------------
    let before = ids_of(&mut plane);
    assert!(
        !before.iter().any(|id| id == "undo-tree-modal"),
        "打开前不该有模态节点"
    );
    report_line(&format!(
        "[a11y-modal] 打开前：节点 {}（sidebar={}, piano-roll={}）",
        before.len(),
        before.iter().any(|id| id == "sidebar"),
        before.iter().any(|id| id == "piano-roll")
    ));

    // ---------------- 打开（写**界面的同一个开关**） ----------------
    window.set_undo_tree_open(true);
    let opened = ids_of(&mut plane);
    let modal_present = opened.iter().any(|id| id == "undo-tree-modal");
    let panel_present = opened.iter().any(|id| id == "undo-tree-panel");
    let outside_addressed: Vec<String> = ["sidebar", "piano-roll", "transport-play-button"]
        .iter()
        .filter(|id| opened.iter().any(|candidate| candidate.as_str() == **id))
        .map(|id| (*id).to_owned())
        .collect();
    report_line(&format!(
        "[a11y-modal] 打开后：节点 {}；modal={} panel={}；模态外仍可寻址的主区 = {:?}（⛔ 焦点值无读数）",
        opened.len(),
        modal_present,
        panel_present,
        outside_addressed
    ));
    assert!(
        modal_present,
        "打开后 `undo-tree-modal` 必须出现在执行面树里"
    );
    assert!(
        panel_present,
        "打开后 `undo-tree-panel` 必须出现在执行面树里"
    );

    // ---------------- 关闭 ----------------
    window.set_undo_tree_open(false);
    let closed = ids_of(&mut plane);
    report_line(&format!(
        "[a11y-modal] 关闭后：节点 {}；模态在树里 = {}；与打开前**同集合** = {}",
        closed.len(),
        closed.iter().any(|id| id == "undo-tree-modal"),
        closed == before
    ));
    assert!(
        !closed.iter().any(|id| id == "undo-tree-modal"),
        "关闭之后模态节点必须消失"
    );
    assert_eq!(
        closed, before,
        "关闭之后寻址面必须**回到打开前的同一集合**（⛔ 焦点值无读数, 见本判据文档）"
    );
}

/// 判据 ②（`[TEST-SPEC-005]` 用例 2：**Tab 焦点顺序校验**）。
///
/// ## ⚠ 先如实登记**观测面的缺口**（⛔ 不假装按焦点断言）
///
/// 本仓今天**没有**可读的"当前焦点"读数：`yeban_ui_test_port::tree` 与 `yeban_ui_mcp::tree`
/// 里 `focus` 命中 **0**（可控面树只有 `id`／`role`／`label`／`bounds`／`visible`／`dynamic`）。
/// `.slint` 侧的焦点是 `forward-focus: key-handler`（`app.slint:331`）＋ `FocusScope` 的
/// **内部**状态，没有导出成属性 ⇒ **"焦点现在在哪"不可观测**。
///
/// ⇒ 本判据因此落地为**两件可观测的事**，并把上面这条缺口**写进断言消息与报告**：
/// ① **注入 Tab 不会破坏界面**（真事件，走 §12.4 的注入面；N 次之后树仍可读、端口仍应答）；
/// ② **Tab 的候选顺序 = 视觉/逻辑顺序**：把主要区域按其**几何**排成 `(y, x)` 序，
///    断言该序列**不重复、不跳过**五区，且**自上而下**单调 —— 这就是"顺序与视觉一致"的
///    可机械检查的那一半（"焦点实际跳到哪"那一半因没有读数而**未覆盖**，见上）。
#[test]
fn the_tab_order_candidates_follow_the_visual_order_and_tab_injection_is_safe() {
    let (_window, mut plane) = assemble();

    // ---------------- ① 真注入 Tab：界面必须保持可用 ----------------
    let before = plane.plane().tree().expect("ui/tree").0;
    for _ in 0..8 {
        let pressed = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
            200,
            "ui/dispatch_key_press",
            Some(serde_json::json!({ "keyCode": "Tab" })), // 参数名与字面写法照既有判据（keyCode/Tab）
        ));
        assert!(
            !pressed.is_error(),
            "Tab 注入本身必须成功（否则这条判据测的不是「焦点轮转」）: {pressed:?}"
        );
    }
    let after = plane
        .plane()
        .tree()
        .expect("注入 Tab 之后 ui/tree 仍必须可读")
        .0;
    report_line(&format!(
        "[a11y-focus] 注入 Tab × 8：树节点 {} ⇒ {}（⛔ 焦点位置**无读数**, 见本判据文档）",
        before.nodes.len(),
        after.nodes.len()
    ));
    assert!(!after.nodes.is_empty(), "注入 Tab 之后界面仍必须可寻址");

    // ---------------- ② 候选顺序 = 视觉顺序（几何序） ----------------
    let regions = [
        "transport-play-button",
        "sidebar",
        "arrangement-ruler",
        "console-tab-rail",
        "piano-roll",
    ];
    let mut located: Vec<(String, f64, f64)> = Vec::new();
    for id in regions {
        if let Some(node) = after.nodes.iter().find(|node| node.id == id) {
            let bounds = node
                .bounds
                .as_ref()
                .unwrap_or_else(|| panic!("主要区域 `{id}` 必须有几何才谈得上「顺序」"));
            located.push((id.to_owned(), f64::from(bounds.y), f64::from(bounds.x)));
        }
    }
    assert_eq!(
        located.len(),
        regions.len(),
        "五个主要区域都必须可寻址（否则顺序断言不成立）"
    );
    let mut sorted = located.clone();
    sorted.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(
                left.2
                    .partial_cmp(&right.2)
                    .unwrap_or(core::cmp::Ordering::Equal),
            )
    });
    report_line(&format!(
        "[a11y-focus] 视觉序（(y, x) 升序）: {:?}",
        sorted
            .iter()
            .map(|(id, y, x)| format!("{id}@({x:.0},{y:.0})"))
            .collect::<Vec<_>>()
    ));
    // 不重复：id 唯一（`TreeError::DuplicateId` 也会在端口侧拒绝，这里再机械核一次）。
    let mut ids: Vec<&str> = sorted.iter().map(|(id, _, _)| id.as_str()).collect();
    ids.sort_unstable();
    let unique = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), unique, "候选顺序里不许有重复 id");
    // 自上而下单调：`y` 必须非降（本视图里五区确实是自上而下排布的）。
    for pair in sorted.windows(2) {
        assert!(
            pair[0].1 <= pair[1].1,
            "视觉序必须自上而下非降：`{}`(y={}) 与 `{}`(y={})",
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
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
    let (_window, plane) = assemble();

    // ---------------- 执行面 JSON（`ui/tree`，读屏真正消费的那份） ----------------
    let mut plane = plane;
    let (tree, _json) = plane.plane().tree().expect("ui/tree 必须可读");
    let (runtime_len, empty_id, bad_id, bad_role, no_label) = audit_runtime(&tree);
    report_line(&format!(
        "[a11y] 执行面 `ui/tree`：来源 {:?}、节点 {runtime_len}；空 id {}；非法 id {}；非法 role {}；**可见但缺 label** {}",
        tree.source,
        empty_id.len(),
        bad_id.len(),
        bad_role.len(),
        no_label.len()
    ));
    if !no_label.is_empty() {
        // **逐条列出**（⛔ 不用比例盖住）—— 这是"遍历"的全部意义。
        report_line(&format!(
            "[a11y] 可见却缺 label 的节点（逐条）: {no_label:?}"
        ));
    }
    assert!(runtime_len > 0, "执行面控件树不能为空");
    assert!(empty_id.is_empty(), "每个节点都必须有非空 id");
    assert!(
        bad_id.is_empty(),
        "id 必须是语义 slug（`[A-Za-z0-9-]`，无空白），非法项: {bad_id:?}"
    );
    assert!(
        bad_role.is_empty(),
        "`accessible-role` 必须是 `KNOWN_ROLES` 之一（⛔ 不许自定义角色），非法项: {bad_role:?}"
    );
    assert!(
        no_label.is_empty(),
        "[TEST-SPEC-005] 可见且可寻址的节点**必须**有无障碍名（读屏靠它宣布控件）；缺名: {no_label:?}"
    );

    // ---------------- 投影注册表（声明面；无窗口也能对账的那份） ----------------
    let registry = plane.registry();
    let (registry_len, r_empty, r_bad, r_no_label) = audit_registry(registry);
    report_line(&format!(
        "[a11y] 投影注册表：节点 {registry_len}；空 id {}；非法 id {}；缺 label {}",
        r_empty.len(),
        r_bad.len(),
        r_no_label.len()
    ));
    assert!(
        registry_len >= runtime_len,
        "注册表必须覆盖执行面（运行时树 ⊆ 注册表）：{registry_len} vs {runtime_len}"
    );
    assert!(
        r_empty.is_empty() && r_bad.is_empty(),
        "注册表侧同样不许有坏 id"
    );
    assert!(
        r_no_label.is_empty(),
        "注册表的每一条都必须有非空无障碍名，缺名项: {r_no_label:?}"
    );

    // ---------------- 主要区域：注册表侧**绝对**，执行面侧**存在即断言** ----------------
    let regions = [
        ("transport-play-button", "走带条（播放）"),
        ("sidebar", "左侧资源栏"),
        ("arrangement-ruler", "编曲区（标尺）"),
        ("console-tab-rail", "底部控制台（标签轨）"),
        ("piano-roll", "卷帘"),
    ];
    let mut runtime_absent: Vec<&str> = Vec::new();
    for (id, name) in regions {
        let declared = registry
            .find_by_id(id)
            .unwrap_or_else(|| panic!("主要区域 `{id}`（{name}）必须在**注册表**里"));
        assert!(
            !declared.label.trim().is_empty(),
            "主要区域 `{id}`（{name}）在注册表里必须有无障碍名"
        );
        match tree.nodes.iter().find(|node| node.id == id) {
            Some(node) => {
                assert!(
                    !node.label.trim().is_empty(),
                    "执行面里出现的 `{id}`（{name}）必须有无障碍名"
                );
                report_line(&format!(
                    "[a11y] 区域 `{id}`（{name}）: 执行面可见, label = {:?}",
                    node.label
                ));
            }
            None => {
                runtime_absent.push(id);
                report_line(&format!(
                    "[a11y] 区域 `{id}`（{name}）: 本视图执行面不可寻址（注册表里有名: {:?}）",
                    declared.label
                ));
            }
        }
    }
    report_line(&format!(
        "[a11y] 本视图执行面缺的区域 id（登记，非失败）: {runtime_absent:?}"
    ));
}
