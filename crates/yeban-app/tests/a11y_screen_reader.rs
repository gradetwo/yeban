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
//! ## 两个面，两种口径（本机实测：执行面 94 节点 / 注册表 864 节点）
//!
//! | 面 | 来源 | 本判据断言 |
//! | :--- | :--- | :--- |
//! | **执行面**（读屏实际看到的） | `ui/tree`（`UiTree`，来源 `Runtime`） | 每个节点：id 非空 ＋ 语义 slug ＋ `role ∈ KNOWN_ROLES`；**可见且可寻址的节点必须有非空 label**（本机实测 0 缺名 ⇒ 严格断言） |
//! | **投影注册表**（声明面） | `LiveControlPlane::registry()` | 同样不变量（本机实测 864/864 有名） |
//!
//! ## 主要区域的口径（两侧分开，⛔ 不混判）
//!
//! 执行面 `ui/tree` 只登记**当前视图可寻址**的节点；四区**不是同时可见** ⇒
//! "某区域不在本视图的执行面里"**不是**读屏缺陷，但：
//! ① **注册表侧必须都有名字**（绝对）；② 执行面里**但凡出现**，名字必须非空；
//! ③ 缺失的区域 id **登记**进日志（可检索），⛔ 不静默。
//!
//! ## 三条判据各自的**观测面**（本文件曾经全部退化成源码字符串包含，本节是那份修复的说明）
//!
//! | 判据 | 观测面（运行时） | ⛔ 不再使用 |
//! | :--- | :--- | :--- |
//! | ① 名称 / role / id 遍历 | `audit_registry`（`ControlTree` 864 节点）＋ `audit_runtime`（`ui/tree` 94 节点） | `app.slint` / `host.rs` 的 `contains(...)` |
//! | ② 模态增删可寻址面 | 活窗口 `LivePort`：开 / 关模态前后抓树，比**节点集合与标签** | 同上 |
//! | ③ Tab 顺序 | 真 `Tab` 注入（8 次）＋ 主要区域的**几何矩形**序 | 同上 |
//! | ④ value / checked 通道 | 活窗口：`accessible-value` / `accessible-checked` 读数 vs **活属性 / 同源模型** | —（新增，本单要求） |
//!
//! ## 如实登记的观测面缺口（⛔ 不假装已覆盖）
//!
//! 1. **控件树里没有焦点字段**：`yeban_ui_test_port::tree` 与 `yeban_ui_mcp::tree` 的
//!    `ControlNode` / `UiNode` 只有 `id`／`role`／`label`／`bounds`／`visible`／`value`／`checked`。
//!    ⇒ "Tab 轮到了哪个控件" / "焦点是否被限制在模态内" **不能**在本文件的树上断言；
//!    后者由 `tests/undo_tree_focus_trap.rs` 按两个镜像布尔覆盖，两者互不冒充。
//! 2. **端口的键面没有组合键**（`KeyCode` 只支持 `Tab`／`Esc`／`Enter`／`Shift+Enter`／
//!    `Space`／`Backspace`／单字符）⇒ 模态的真实快捷键（`Cmd/Ctrl+Shift+H`）无法从端口注入。
//! 3. **`accessible-checked` 会被"点击但无落点"骗过**：本机实测（`track-0-mute-button`
//!    在只接了 `wire_view_callbacks` 的装配里）点击后元素 checked 翻成 `true`，而
//!    `track-mutes` 与工程 `TrackV3::mute` **一位没动** —— Slint 对 `accessible-checkable`
//!    元素在收到指针事件后会翻转自己的内部态。⇒ 判据④因此**只用有生产落点的控件**
//!    （`console-maximize-button`：点击真的改 `console-maximized`）当"读数跟随状态"的证据，
//!    并把这条陷阱写进断言消息，⛔ 不拿任何一次点击翻转换算成"通道正确"。
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
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_mcp::tree::UiTree;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::{ControlTree, KNOWN_ROLES};
use yeban_ui_test_port::{Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

/// 装配一条真实控制面（与 `live_ui_mcp.rs` 同款入口）。
fn assemble() -> (yeban_app::ui::MainWindow, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, plane)
}

/// 装配一个**活窗口**（`host::build_main_window` ＝ `main.rs` 的同一个构造入口）。
///
/// `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
/// 每个用到本函数的 `#[test]` 只建**一个** `LivePort`（与 `tests/console_maximize.rs` 同款）。
fn live_window() -> LivePort<MainWindow> {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_view_callbacks(port.ui());
    port
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

/// 前 `limit` 条进断言消息（⛔ 只报一个数字，缺名的节点要**逐条**可检索）。
fn head(list: &[String], limit: usize) -> String {
    let shown = list
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if list.len() > limit {
        format!("{shown}, … 共 {} 条", list.len())
    } else {
        shown
    }
}

/// 抓一次活窗口的控件树（`dispatch_pointer_*` 按**缓存的**包围盒换算坐标，先抓一次是既有口径）。
fn bounds_of(port: &mut LivePort<MainWindow>, id: &str) -> Rect {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

/// 在一个控件的中心点一次真实指针点击（down + up）。
fn click_center(port: &mut LivePort<MainWindow>, id: &str) {
    let bounds = bounds_of(port, id);
    port.dispatch_pointer_down(
        id,
        f64::from(bounds.width) / 2.0,
        f64::from(bounds.height) / 2.0,
        yeban_ui_test_port::PointerButton::Left,
    )
    .expect("点下必须送达");
    port.dispatch_pointer_up(yeban_ui_test_port::PointerButton::Left)
        .expect("松开必须送达");
}

/// 树里所有节点的 `(id, role, label)`（判据②③④ 比集合用）。
fn id_set(port: &mut LivePort<MainWindow>) -> Vec<String> {
    let mut ids: Vec<String> = port
        .refresh_tree(None)
        .expect("必须能重抓控件树")
        .iter()
        .map(|node| node.id.clone())
        .collect();
    ids.sort();
    ids
}

// ---------------------------------------------------------------------------
// 判据①：名称 / role / id 遍历（两个面，各跑一次真遍历）
// ---------------------------------------------------------------------------

/// 判据 ①（`[TEST-SPEC-005]`）：**控件无障碍名称遍历**。
///
/// ## 怎么变红（四条，各自独立）
///
/// 1. 给任一节点加 `accessible-id` 却起个带空格/非法字符的名字 ⇒ 第 2 条断言红；
/// 2. 把某节点的 `accessible-role` 改成 `KNOWN_ROLES` 之外 ⇒ 第 3 条断言红；
/// 3. 让一个**可见**节点失去 `accessible-label`（或新增无标签的可寻址节点）⇒ 第 4 条断言红；
/// 4. 注册表里出现一个空 id / 非法 id / 空 label 的条目 ⇒ 第 5 条断言红。
///
/// 本判据**实测跑完两个面的整棵树**（不是抽样、不是数个数）：执行面 94 节点、
/// 注册表 864 节点；缺名/非法项逐条列进断言消息。
#[test]
fn every_addressable_node_carries_a_semantic_id_and_a_non_empty_accessible_name() {
    let (_window, mut plane) = assemble();

    // ---- 执行面：`ui/tree` 的运行时投影 ----
    let runtime = plane.plane().tree().expect("ui/tree 必须可读").0;
    let (count, empty_id, bad_id, bad_role, no_label) = audit_runtime(&runtime);
    report_line(&format!(
        "a11y runtime: nodes={count} empty_id={} bad_id={} bad_role={} visible_without_label={}",
        empty_id.len(),
        bad_id.len(),
        bad_role.len(),
        no_label.len()
    ));
    assert!(
        count > 0,
        "执行面控件树不得为空：读屏在这棵树上找节点，空树等于一位都读不到"
    );
    assert!(
        empty_id.is_empty(),
        "执行面有 {} 个节点没有语义 id：{}",
        empty_id.len(),
        head(&empty_id, 10)
    );
    assert!(
        bad_id.is_empty(),
        "执行面有 {} 个节点的 id 不是语义 slug（只允许 [A-Za-z0-9-]）：{}",
        bad_id.len(),
        head(&bad_id, 10)
    );
    assert!(
        bad_role.is_empty(),
        "执行面有 {} 个节点的 role 不在 KNOWN_ROLES（{:?}）里：{}",
        bad_role.len(),
        KNOWN_ROLES,
        head(&bad_role, 10)
    );
    assert!(
        no_label.is_empty(),
        "执行面有 {} 个**可见**节点没有可读名（读屏会念空）：{}",
        no_label.len(),
        head(&no_label, 10)
    );

    // ---- 声明面：投影注册表（覆盖声明过的全集，包含当前视图不可见的分支） ----
    let registry = plane.registry().clone();
    let (reg_count, reg_empty_id, reg_bad_id, reg_no_label) = audit_registry(&registry);
    report_line(&format!(
        "a11y registry: nodes={reg_count} empty_id={} bad_id={} no_label={}",
        reg_empty_id.len(),
        reg_bad_id.len(),
        reg_no_label.len()
    ));
    assert!(
        reg_count >= count,
        "注册表（声明全集）不得少于执行面（当前可寻址）：reg={reg_count} runtime={count}"
    );
    assert!(
        reg_empty_id.is_empty() && reg_bad_id.is_empty(),
        "注册表有 {} 个空 id、{} 个非法 id：{} / {}",
        reg_empty_id.len(),
        reg_bad_id.len(),
        head(&reg_empty_id, 5),
        head(&reg_bad_id, 5)
    );
    assert!(
        reg_no_label.is_empty(),
        "注册表有 {} 个条目没有可读名（注册表侧是**绝对**要求：它覆盖声明过的全集）：{}",
        reg_no_label.len(),
        head(&reg_no_label, 10)
    );

    // ---- 主要区域：执行面里凡是出现，就必须有名（且 role 合法） ----
    for id in [
        "workspace-arrangement-canvas",
        "piano-roll",
        "console-tab-rail",
        "status-bar",
        "transport-play-button",
    ] {
        let node = runtime
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap_or_else(|| {
                panic!("主要区域 `{id}` 必须在执行面里可寻址（本机实测 94 节点全部可见）")
            });
        assert!(
            !node.label.trim().is_empty(),
            "主要区域 `{id}` 的读屏名是空的"
        );
        assert!(
            KNOWN_ROLES.contains(&node.role.as_str()),
            "主要区域 `{id}` 的 role={:?} 不在 KNOWN_ROLES 里",
            node.role
        );
    }
}

// ---------------------------------------------------------------------------
// 判据②：模态对话框的**可寻址面**增删（真窗口、真开关）
// ---------------------------------------------------------------------------

/// 判据 ③（`[TEST-SPEC-005]` 用例 3：**模态对话框**）。
///
/// ## 本仓**确有**模态（先抓树再断言，⛔ 不读源码）
///
/// `undo-tree-modal`（role=`region`）＋ `undo-tree-panel` 是「时光机」模态的两个容器，
/// 另有 `undo-tree-undo-button` / `undo-tree-redo-button` / `undo-tree-close-button` /
/// `undo-tree-empty-state` 四个可操作/可读节点。开关是 `MainWindow::undo-tree-open`
/// （`host.rs` 明记「时光机是模态面板」）。
///
/// ## ⚠ 一个字**不能**在这棵树上证：焦点陷阱
///
/// 控件树没有焦点字段（见文件头缺口 1）⇒ "焦点被限制在模态内／关闭后回到打开者"
/// **不能**在此断言；它由 `tests/undo_tree_focus_trap.rs` 按镜像布尔
/// （`undo-tree-focus-trapped` / `shortcut-layer-focused`）覆盖。本判据只证**寻址面**。
///
/// ## 本判据证的三件事（都是运行时读数）
///
/// ① **打开**（写与 `host.rs` 同一个开关）⇒ 六个 `undo-tree-*` 节点**出现在树里**，
///    且每一个都有可读名（读屏念得出「关闭时光机」「撤销一步」）；
/// ② 打开后节点数**严格变多**（不是"换个名字还在原地"）；
/// ③ **关闭** ⇒ 六个节点**全部消失**，且整棵树的 **id 集合逐项回到打开前**（BTreeMap 序）。
#[test]
fn the_modal_adds_and_removes_its_addressable_surface_on_a_live_window() {
    let mut port = live_window();
    let before = id_set(&mut port);
    report_line(&format!("a11y modal: closed nodes={}", before.len()));
    for id in MODAL_NODES {
        assert!(
            !before.iter().any(|existing| existing == id),
            "关闭态不应有模态节点 `{id}`（本机实测关闭态 {} 节点里没有 undo-tree-*）",
            before.len()
        );
    }

    port.ui().set_undo_tree_open(true);
    let opened_ids = id_set(&mut port);
    let opened = port.refresh_tree(None).expect("打开后必须能重抓控件树");
    report_line(&format!(
        "a11y modal: opened nodes={} (+{})",
        opened.len(),
        opened.len().saturating_sub(before.len())
    ));
    assert!(
        opened.len() > before.len(),
        "打开模态后树必须**变多**（模态的可寻址面加进来）：closed={} opened={}",
        before.len(),
        opened.len()
    );
    for id in MODAL_NODES {
        let node = opened
            .find_by_id(id)
            .unwrap_or_else(|| panic!("打开后模态节点 `{id}` 必须进控件树（读屏要能念到它）"));
        assert!(
            !node.label.trim().is_empty(),
            "模态节点 `{id}` 的读屏名是空的"
        );
        assert!(
            KNOWN_ROLES.contains(&node.role.as_str()),
            "模态节点 `{id}` 的 role={:?} 不在 KNOWN_ROLES 里",
            node.role
        );
    }

    port.ui().set_undo_tree_open(false);
    let after = id_set(&mut port);
    report_line(&format!("a11y modal: re-closed nodes={}", after.len()));
    for id in MODAL_NODES {
        assert!(
            !after.iter().any(|existing| existing == id),
            "关闭后模态节点 `{id}` 必须离开控件树（模态不该留一个可寻址的空壳）"
        );
    }
    assert_eq!(
        after, before,
        "关闭后整棵树的 id 集合必须**逐项**回到打开前（模态是一次纯增删，不该顺手改别的节点）"
    );
    for id in &before {
        assert!(
            opened_ids.iter().any(|existing| existing == id),
            "打开模态不得**弄丢**原有节点 `{id}`（模态是叠加，不是替换整棵树）"
        );
    }
}

/// 时光机模态的六个可寻址节点（本机实测，全部来自 `ui/dialogs/` 的 `accessible-id`）。
const MODAL_NODES: [&str; 6] = [
    "undo-tree-modal",
    "undo-tree-panel",
    "undo-tree-close-button",
    "undo-tree-empty-state",
    "undo-tree-redo-button",
    "undo-tree-undo-button",
];

// ---------------------------------------------------------------------------
// 判据③：Tab 注入安全 + 主要区域的几何序
// ---------------------------------------------------------------------------

/// 判据 ②（`[TEST-SPEC-005]` 用例 2：**Tab 焦点顺序校验**）。
///
/// ## ⚠ 先如实登记**观测面的缺口**（⛔ 不假装按焦点断言）
///
/// 本仓今天**没有**"当前焦点在哪"的逐节点读数：`yeban_ui_test_port::tree` 与
/// `yeban_ui_mcp::tree` 里 `focus` 命中 **0**（节点只有
/// `id`／`role`／`label`／`bounds`／`visible`／`dynamic`）。`.slint` 侧的焦点是
/// `forward-focus: key-handler` ＋ `FocusScope` 的**内部**状态；只有模态那一个作用域与
/// 键盘事件源另有两个镜像布尔（`undo-tree-focus-trapped` / `shortcut-layer-focused`，
/// 由 `tests/undo_tree_focus_trap.rs` 使用），它们**不能**回答"Tab 轮到了哪个控件"。
///
/// ⇒ 本判据落地为**两件可观测的事**，并把缺口写进断言消息：
/// ① **注入 Tab 不会破坏界面**（真事件，走 §12.4 的注入面；8 次之后树仍可读、节点数不减）；
/// ② **主要区域的几何序 = 视觉顺序**：顶栏 → 走带 → 工作区 → 控制台 → 状态栏，
///    按 `y` **严格递增** —— 这是"Tab 候选顺序与视觉一致"里可机械检查的那一半
///    （"焦点实际跳到哪"那一半因没有读数而**未覆盖**，见上）。
#[test]
fn the_tab_order_candidates_follow_the_visual_order_and_tab_injection_is_safe() {
    let mut port = live_window();
    let before = port.refresh_tree(None).expect("首抓必须成功").len();

    for round in 0..8 {
        port.dispatch_key_press(yeban_ui_test_port::KeyCode::Tab)
            .unwrap_or_else(|error| panic!("第 {round} 次 Tab 注入必须送达: {error:?}"));
    }
    let after = port
        .refresh_tree(None)
        .expect("注入 Tab 之后 ui/tree 仍必须可读");
    report_line(&format!(
        "a11y tab: nodes before={before} after={}",
        after.len()
    ));
    assert!(
        !after.iter().next().is_none(),
        "注入 Tab 之后界面仍必须可寻址（空树 = 键盘把界面打没了）"
    );
    assert_eq!(
        after.len(),
        before,
        "注入 8 次 Tab 不得增删可寻址节点（焦点移动不是视图切换）"
    );

    // ② 视觉顺序：五个区域的 y 必须严格递增（同一列从上往下就是 Tab 的候选序）。
    let order = [
        "transport-play-button",
        "workspace-arrangement-canvas",
        "console-tab-rail",
        "status-bar",
    ];
    let mut previous: Option<(i32, &str)> = None;
    for id in order {
        let node = after
            .find_by_id(id)
            .unwrap_or_else(|| panic!("主要区域 `{id}` 必须在执行面里可寻址"));
        let bounds = node
            .bounds
            .unwrap_or_else(|| panic!("主要区域 `{id}` 必须有几何（顺序断言靠 y）"));
        if let Some((prev_y, prev_id)) = previous {
            assert!(
                bounds.y > prev_y,
                "视觉顺序必须自上而下：`{prev_id}` 的 y={prev_y}，`{id}` 的 y={} —— \
                 Tab 候选序与画面不一致（焦点读数缺失，本条是能判的那一半）",
                bounds.y
            );
        }
        previous = Some((bounds.y, id));
    }
    report_line("a11y tab: 四个主要区域自上而下单调（transport < workspace < console < status）");
}

// ---------------------------------------------------------------------------
// 判据④：value / checked 通道的运行时读数（本单新增）
// ---------------------------------------------------------------------------

/// 判据 ④（本单要求）：**`accessible-value` / `accessible-checked` 通道读数与当前状态一致**。
///
/// ## 三条子判据
///
/// | 子判据 | 断言 | 能抓住的错法 |
/// | :--- | :--- | :--- |
/// | ① checked 的**语义域** | 凡有 `checked` 读数的节点，`role ∈ {button, checkbox, switch, radio-button}`；且读数个数 ≥ 15（本机实测 15：9 个轨头开关 ＋ 5 个工具 ＋ 1 个控制台最大化） | 给文本/区域乱挂勾选态；整条通道被摘掉 |
/// | ② checked **跟随真实状态** | 点 `console-maximize-button`（生产落点 = `console-maximized`）⇒ 读数 `false→true`，且 `window.get_console_maximized()` 同步为真；再点一次 ⇒ 双双回 `false` | 读数写死；读数与属性两条事实源分叉 |
/// | ③ value **与同源模型一致** | `velocity-{i}-bar.value == round(note_velocities[i] × 127)`（四根柱）；`transport-timecode.value == piano-roll-playhead.value` 且形如 `ddd.dd.ddd` | 值写死（旧实现曾把每根柱写死 `"100"`）；两个走带投影各说各话 |
///
/// ## ⚠ 为什么不拿 `track-0-mute-button` 当"跟随状态"的证据
///
/// 见文件头缺口 3：在只接了 `wire_view_callbacks` 的装配里，点它元素 checked 会翻成
/// `true`，而 `track-mutes` 与工程 `TrackV3::mute` **一位没动** —— 那是 Slint 对
/// `accessible-checkable` 元素的内部态，不是文档状态。本判据只用**有生产落点**的控件，
/// 且每一次翻转都与活属性对账，⛔ 不拿"点了就变"冒充"通道正确"。
#[test]
fn the_value_and_checked_channels_track_the_live_state() {
    let mut port = live_window();
    // 先抓一次：指针注入按缓存的包围盒换算坐标（既有口径）。
    port.refresh_tree(None).expect("首抓必须成功");

    // ---- 子判据 ①：checked 的语义域 ----
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    let checkable_roles = ["button", "checkbox", "switch", "radio-button"];
    let mut checked_nodes = Vec::new();
    for node in tree.iter() {
        let Some(checked) = node.checked else {
            continue;
        };
        assert!(
            checkable_roles.contains(&node.role.as_str()),
            "节点 `{}` 有 checked={checked}，但 role={:?} 不是可勾选语义（允许：{checkable_roles:?}）",
            node.id,
            node.role
        );
        checked_nodes.push(node.id.clone());
    }
    report_line(&format!(
        "a11y channels: checked_nodes={} {:?}",
        checked_nodes.len(),
        checked_nodes
    ));
    assert!(
        checked_nodes.len() >= 15,
        "至少有 15 个节点该有勾选态读数（9 个轨头开关 ＋ 5 个工具 ＋ 1 个控制台最大化），实得 {}：{:?}",
        checked_nodes.len(),
        checked_nodes
    );

    // ---- 子判据 ②：checked 跟随真实状态（console-maximize-button） ----
    let before_checked = tree
        .find_by_id("console-maximize-button")
        .expect("控制台最大化按钮必须在树里")
        .checked;
    assert_eq!(
        before_checked,
        Some(false),
        "装配态 console-maximized == false ⇒ 读屏读到的勾选态必须是 false"
    );
    assert!(
        !port.ui().get_console_maximized(),
        "装配态 console-maximized 属性必须为 false（与上一条是同一个事实的两个出口）"
    );
    click_center(&mut port, "console-maximize-button");
    assert!(
        port.ui().get_console_maximized(),
        "点击后 console-maximized 必须为 true（生产落点：`Action::ToggleConsoleMaximize` 同一个 setter）"
    );
    let after_click = port
        .refresh_tree(None)
        .expect("点击后必须能重抓控件树")
        .find_by_id("console-maximize-button")
        .and_then(|node| node.checked);
    assert_eq!(
        after_click,
        Some(true),
        "点击后勾选态读数必须跟随属性翻成 true（否则读屏与画面分叉）"
    );
    click_center(&mut port, "console-maximize-button");
    assert!(
        !port.ui().get_console_maximized(),
        "再点一次必须还原（`false → true → false` 全程可判）"
    );
    let after_second = port
        .refresh_tree(None)
        .expect("还原后必须能重抓控件树")
        .find_by_id("console-maximize-button")
        .and_then(|node| node.checked);
    assert_eq!(after_second, Some(false), "还原后勾选态读数必须回到 false");

    // ---- 子判据 ③a：velocity 柱的 value 与同源模型一致 ----
    let velocities: Vec<f32> = {
        use slint::Model as _;
        port.ui().get_note_velocities().iter().collect()
    };
    assert!(
        velocities.len() >= 4,
        "夹具至少要有 4 个速度读数（力度泳道画 4 根柱），实得 {}",
        velocities.len()
    );
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    for (index, velocity) in velocities.iter().take(4).enumerate() {
        let id = format!("velocity-{index}-bar");
        let node = tree
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("力度柱 `{id}` 必须在树里"));
        let raw = node
            .value
            .as_deref()
            .unwrap_or_else(|| panic!("力度柱 `{id}` 必须有 accessible-value（0–127）"));
        let reading: f32 = raw
            .parse()
            .unwrap_or_else(|error| panic!("力度柱 `{id}` 的 value={raw:?} 不是数字: {error}"));
        let expected = (velocity * 127.0).round();
        assert!(
            (reading - expected).abs() < 0.5,
            "力度柱 `{id}` 的读屏值必须与同源模型一致：读数={reading} 期望=round({velocity}×127)={expected}"
        );
        assert!(
            (0.0..=127.0).contains(&reading),
            "力度柱 `{id}` 的读数 {reading} 越界（合法域 0–127）"
        );
    }
    report_line("a11y channels: 4 根力度柱的 value 与 note-velocities 同源一致");

    // ---- 子判据 ③b：两个走带投影的 value 必须一致，且格式合法 ----
    let timecode = tree
        .find_by_id("transport-timecode")
        .and_then(|node| node.value.clone())
        .expect("走带时间码必须有 accessible-value（读屏要念时间）");
    let playhead = tree
        .find_by_id("piano-roll-playhead")
        .and_then(|node| node.value.clone())
        .expect("卷帘走带光标必须有 accessible-value");
    assert_eq!(
        timecode, playhead,
        "同一走带位置的两个投影（顶栏时间码 / 卷帘光标）必须读出同一个值：{timecode:?} vs {playhead:?}"
    );
    assert!(
        is_timecode(&timecode),
        "时间码必须形如 ddd.dd.ddd（小节.拍.ticks），实得 {timecode:?}"
    );
    report_line(&format!(
        "a11y channels: timecode={timecode} playhead={playhead} （一致）"
    ));
}

/// `ddd.dd.ddd`：三小节数字点两位拍点三位 ticks（`transport-timecode` 的唯一读法）。
fn is_timecode(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts[0].len() == 3
        && parts[1].len() == 2
        && parts[2].len() == 3
        && parts
            .iter()
            .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
}

// ---------------------------------------------------------------------------
// 判据的**自检**：审计函数对每一种退化都必须判红
// ---------------------------------------------------------------------------

/// 审计函数本身的负向对照（`audit_runtime` / `audit_registry`）。
///
/// ## 为什么需要这一条
///
/// 产品侧有**上游守门**，有两种退化写不出来 ⇒ 端到端判据无法为它们构造红点：
///
/// 1. **非法 `accessible-role`**：`.slint` 的 `accessible-role` 是枚举，写不出
///    `KNOWN_ROLES` 之外的值；`yeban_ui_test_port::tree::Role` 又是 `parse` 时才校验的
///    newtype ⇒ 只有"投影线格式"能带非法 role；
/// 2. **非法 id**：`ControlTree::insert` 用 `MalformedId` 直接拒绝（本机实测：把
///    `piano-roll-status` 改成 `"piano roll status"` 会在**装配**阶段就红
///    `Render(Tree(MalformedId { id: "piano roll status" }))`，走不到断言）。
///
/// ⇒ 这两种"判据声称能抓"的退化，改由本用例直接喂给审计函数来证明判别力。
/// ⛔ 这不替代端到端判据：`audit_runtime` / `audit_registry` 是判据①的**唯一**判据体，
/// 本用例证明的是"判据体本身会红"，端到端的两条红点见交付回执（`accessible-value`
/// 写死 99、`undo-tree-panel` 清空 `accessible-label`）。
#[test]
fn the_audit_rejects_every_degradation_it_claims_to_catch() {
    use yeban_ui_mcp::tree::{TreeSource, UiNode};
    use yeban_ui_test_port::tree::{ControlNode, Role};

    fn node(id: &str, role: &str, label: &str) -> UiNode {
        UiNode {
            id: id.to_owned(),
            role: role.to_owned(),
            label: label.to_owned(),
            bounds: None,
            visible: Some(true),
            dynamic_region: false,
            maskable: false,
            parent: None,
            value: None,
            checked: None,
        }
    }

    let degraded = UiTree {
        source: TreeSource::Runtime,
        count: 4,
        nodes: vec![
            node("", "button", "空 id"),
            node("bad id", "button", "带空格的 id"),
            node("bogus-role", "bogus-role", "非法 role"),
            node("visible-no-name", "button", ""),
        ],
    };
    let (count, empty_id, bad_id, bad_role, no_label) = audit_runtime(&degraded);
    assert_eq!(count, 4, "审计必须数到全部 4 个节点");
    assert_eq!(empty_id.len(), 1, "空 id 必须被逐条抓出：{empty_id:?}");
    assert_eq!(bad_id, vec!["bad id".to_owned()], "带空格的 id 必须被抓出");
    assert_eq!(
        bad_role.len(),
        1,
        "非法 role 必须被抓出（role 词汇表来自 KNOWN_ROLES）：{bad_role:?}"
    );
    assert_eq!(
        no_label,
        vec!["visible-no-name".to_owned()],
        "可见而无名的节点必须被抓出"
    );

    let mut registry = ControlTree::new();
    registry
        .insert(ControlNode::new(
            "no-label-node",
            Role::parse("button").expect("button 是合法 role"),
            "",
        ))
        .expect("`no-label-node` 是合法 id");
    let (reg_count, reg_empty_id, reg_bad_id, reg_no_label) = audit_registry(&registry);
    assert_eq!(reg_count, 1);
    assert!(
        reg_empty_id.is_empty() && reg_bad_id.is_empty(),
        "本样本的 id 是合法的（空 id / 非法 id 由 `ControlTree::insert` 的更早一层守住）"
    );
    assert_eq!(
        reg_no_label,
        vec!["no-label-node".to_owned()],
        "注册表侧的无名条目必须被抓出"
    );
    report_line("a11y self-check: audit_runtime / audit_registry 对 4 种退化都判红");
}
