//! ⭐ 任务单 `#185`：**发送条内的控件两两不许重叠**。
//!
//! ## 这条判据要证的句子
//!
//! | 判据 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 空态与按钮不压 | `1920×1080 / 1440×900 / 1024×800` 三档：`send-empty` 与 `send-create-button` 的包围盒**不相交**，且空态文案的上/下沿在按钮**上方**（两者间距 4px） | 证"`#184` 登记的那处重叠消失了"；⛔ 不证人眼看过 |
//! | ② 两两不相交 | 空态三档 ＋ 有 1 条发送一档（1440）：`send-strip` **所有在场**的非容器节点逐对断言包围盒不相交；逐档报"控件数 / 对数 / 交集为空" | 证"同一段内没有第二处重叠"；⛔ 不证与段外控件不重叠（那是 `#184` 的判据） |
//! | ③ 空态仍可见 | `send-empty` 在树里、逐字 `无发送`、包围盒在 `send-strip` 的裁剪区之内 | 证"修重叠没有把空态删掉" |
//! | ④ 按钮仍可点 | 无 aux ⇒ `send-create-available = false` ＋ 逐字原因在树里 ＋ 直接调用回调 `Δcommit = 0`；有 aux 但无发送 ⇒ `true` ＋ 点击 `Δcommit = 1` | 证"按钮不是假按钮"；⛔ 不证听感 |
//! | ⑤ 三档逐格 | 两个控件的 `bounds` 在三档上逐格等于本单登记值；`send-strip` 的高度在空态是 `148`（多出的 22px 是让给按钮下移的） | 证"默认档的几何是**有意**改的，不是漂移" |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! 两条注入都**实测**过（读数见交付报告 §4）：
//!
//! - 注入 A：把 `send-create-y` 的空态 `+ 22px` 去掉（按钮回到 `82 + 40n`）⇒ **① 红**：
//!   `send-empty` `(452,879,364,20)` 与按钮 `(452,881,160,22)` 重新相交
//!   （与 `#184` 在 1024 档记的 `(452,711,220,20)` vs `(452,713,160,22)` 同一组相对几何）；
//! - 注入 B：把 `send-create-reason` 的 `y` 换回 `#159` 的 `108 + 40n`（空态 `104` 让位保留）
//!   ⇒ **② 红**：`send-create-button (452,903,160,22) ⟂ send-create-reason (452,907,364,14)`
//!   —— 这一条**只**被"两两不相交"抓住（① 的两节点断言仍是绿的），证明判据 ② 是承重墙。
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过（本机是 Tier-1 软件光栅化，没有声卡）；⛔ 不证 Linux Golden：
//! 本机没有 `tests/golden/macos/**`，逐字节比对在 macOS 上**未被判定**；
//! ⛔ 不证"所有宽度"—— 只验 1920/1440/1024 三档（`#184` 已证 800/848 两档右列整列撤下，
//! 发送条根本不在场，本判据在更窄档上会找不到 `send-strip` 而**如实红**，⛔ 不静默跳过）。
//!
//! ## 为什么它一直没被抓（`#184` 的原话）
//!
//! 两个节点都在树里、各自的几何**单独看都合法**、各自的判据都过 ⇒ 只有"两两不相交"
//! 这种**跨节点**的判据才能抓住它。`#184` 用同一条判据钉住了"两段之间"，
//! 本文件把口径扩到"**同一段内**"。

use std::collections::BTreeMap;
use std::rc::Rc;

use yeban_app::bridge::{SEND_CREATE_NO_AUX_REASON, ViewState};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::project::{RoutingKind, TrackKind, TrackV3, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::UiTestPort as _;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::{ControlNode, ControlTree};

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 三档：默认档 ＋ 仓里界面判据的默认窗口 ＋ `#184` 的窄档下界（右列仍在场的最小宽度）。
const TIERS: &[(u32, u32)] = &[(1920, 1080), (1440, 900), (1024, 800)];

type Bounds = (i32, i32, u32, u32);

/// 夹具：展示轨 ＋ 主总线，**没有** `AuxReturn`、**没有**任何发送边。
///
/// 这是最坏的一格：发送条空态（`send-empty` 在场）＋ 按钮在场（禁用）＋
/// 逐字原因也在场（`!send-create-available`）—— 三个节点挤在同一段里。
fn project() -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();

    let display = TrackV3 {
        id: track,
        name: "展示轨".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    let master_track = TrackV3 {
        id: master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };

    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(track, display);
    tracks.insert(master, master_track);

    YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        ..YebanProjectV1::default()
    }
}

/// `demo_project`（6 轨、0 条发送）＋ 一条 `AuxReturn` 轨 ⇒ 按钮**可用**但发送数仍是 0。
fn project_with_aux_but_no_send() -> YebanProjectV1 {
    let mut project = yeban_model::samples::demo_project();
    let aux_id = EntityId::new();
    project.tracks.insert(
        aux_id,
        TrackV3 {
            id: aux_id,
            name: "Aux Send Test".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(aux_id);
    project.routing_graph.nodes.sort_unstable();
    project
}

/// 一条 Tier-1 活窗口（生产窗口构造入口；Tab 3 = 设备架）＋ 生产同一个 `wire_send_strip`。
fn live_with_port(
    size: Size,
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(size, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + Tab 3 生产窗口");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    host::wire_send_strip(live.ui(), &port);
    (live, port)
}

/// 手势之后再读一次**活的**控件树 —— `LivePort::tree()` 是构造时的快照，不会随重绘更新。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

fn bounds(tree: &ControlTree, id: &str) -> Bounds {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    let b = node.bounds.expect("运行时节点必须有包围盒");
    (b.x, b.y, b.width, b.height)
}

/// 两个包围盒是否相交（半开区间：边贴边**不算**相交）。
fn intersect(a: Bounds, b: Bounds) -> bool {
    let (ax, ay, aw, ah) = (
        i64::from(a.0),
        i64::from(a.1),
        i64::from(a.2),
        i64::from(a.3),
    );
    let (bx, by, bw, bh) = (
        i64::from(b.0),
        i64::from(b.1),
        i64::from(b.2),
        i64::from(b.3),
    );
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

/// 容器角色：它们的包围盒**按设计**包住子节点，不参与"两两不相交"。
///
/// ⚠ 运行时控件树的 `parent` 字段在 `yeban-ui-test-port::inspect` 里**恒为 `None`**
/// （`node_from_handle` 不填它），所以这里不能靠父链认容器；改用**角色**这一机械判据
/// —— 发送条里被当作容器的只有 `region`（`send-strip` / `track-routing-panel`）
/// 与 `list-item`（`send-*-row`，它的子节点是滑块 / 读数 / 删除键）。
fn is_container_role(role: &str) -> bool {
    matches!(role, "region" | "groupbox" | "list" | "list-item")
}

/// `send-strip` 内的**全部在场控件**（不含 `send-strip` 自己）。
///
/// 口径是**几何**的：包围盒**完全落在** `send-strip` 之内。这条判据同时把
/// "段外的邻居"与"只压到一条边"的碎片都排除掉（`#184` 的两段判据已经证过段外不重叠）。
fn inside_strip(tree: &ControlTree, strip: Bounds) -> Vec<&ControlNode> {
    let right = strip.0 + strip.2 as i32;
    let bottom = strip.1 + strip.3 as i32;
    tree.iter()
        .filter(|node| node.id != "send-strip")
        .filter(|node| {
            let Some(b) = node.bounds else {
                return false;
            };
            b.x >= strip.0
                && b.y >= strip.1
                && b.x + b.width as i32 <= right
                && b.y + b.height as i32 <= bottom
        })
        .collect()
}

/// 把段内节点分成"容器"与"控件"（按 [`is_container_role`]）。
fn split_containers<'a>(all: &[&'a ControlNode]) -> (Vec<&'a ControlNode>, Vec<&'a ControlNode>) {
    let mut containers = Vec::new();
    let mut controls = Vec::new();
    for node in all {
        if is_container_role(node.role.as_str()) {
            containers.push(*node);
        } else {
            controls.push(*node);
        }
    }
    (containers, controls)
}

/// 空态下发送条里**必须在场**的控件（少了任何一个，两两判据就会漏掉它）。
const EXPECTED_EMPTY: &[&str] = &[
    "track-routing-count",
    "master-bus-label",
    "track-create-midi-button",
    "track-create-audio-button",
    "track-create-aux-button",
    "track-delete-button",
    "send-count",
    "send-empty",
    "send-create-button",
    "send-create-reason",
];

/// 有 1 条发送时**必须在场**的控件（逐字行 / 滑块 / 读数 / 删除键都进来了；此时按钮
/// 因"展示轨 → 这条 aux 已有边"而**重新禁用**，逐字原因也跟着回到 `send-create-y + 26`）。
const EXPECTED_FILLED: &[&str] = &[
    "track-routing-count",
    "master-bus-label",
    "track-create-midi-button",
    "track-create-audio-button",
    "track-create-aux-button",
    "track-delete-button",
    "send-count",
    "send-0-gain-slider",
    "send-0-gain-value",
    "send-0-remove-button",
    "send-create-button",
    "send-create-reason",
];

/// 判据 ② 的唯一实现：段内在场控件 → 排除容器 → 逐对断言不相交 → 全部在裁剪区内。
///
/// 返回一段可直接 `report_line` 的读数（控件清单 ＋ 对数 ＋ 容器清单）。
fn assert_pairwise(tree: &ControlTree, strip: Bounds, expected: &[&str], label: &str) -> String {
    let all = inside_strip(tree, strip);
    let (containers, controls) = split_containers(&all);
    // 清单必须**一个不少**被数到（否则"两两不相交"会退化成空判据）。
    for id in expected {
        assert!(
            controls.iter().any(|node| node.id == *id),
            "[#185 {label}] 段内控件清单缺 `{id}` —— 两两判据会漏掉它"
        );
    }
    let mut pair_count = 0_usize;
    let mut hits = Vec::new();
    for (index, a) in controls.iter().enumerate() {
        let a_bounds = a.bounds.expect("缺包围盒");
        for b in controls.iter().skip(index + 1) {
            pair_count += 1;
            let b_bounds = b.bounds.expect("缺包围盒");
            if intersect(
                (a_bounds.x, a_bounds.y, a_bounds.width, a_bounds.height),
                (b_bounds.x, b_bounds.y, b_bounds.width, b_bounds.height),
            ) {
                hits.push(format!(
                    "`{}` (x={} y={} w={} h={}) ⟂ `{}` (x={} y={} w={} h={})",
                    a.id,
                    a_bounds.x,
                    a_bounds.y,
                    a_bounds.width,
                    a_bounds.height,
                    b.id,
                    b_bounds.x,
                    b_bounds.y,
                    b_bounds.width,
                    b_bounds.height
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "[#185 {label}] 发送条内**有重叠**：\n{}",
        hits.join("\n")
    );
    // 控件必须全在 `send-strip` 的裁剪区之内（原因行靠 `height` 让出来的 22px 才装得下）。
    let right = strip.0 + strip.2 as i32;
    let bottom = strip.1 + strip.3 as i32;
    for node in &controls {
        let b = node.bounds.expect("缺包围盒");
        assert!(
            b.x >= strip.0
                && b.y >= strip.1
                && b.x + b.width as i32 <= right
                && b.y + b.height as i32 <= bottom,
            "[#185 {label}] `{}` (x={} y={} w={} h={}) 不在 `send-strip` \
             (x={} y={} w={} h={}) 的裁剪区内",
            node.id,
            b.x,
            b.y,
            b.width,
            b.height,
            strip.0,
            strip.1,
            strip.2,
            strip.3
        );
    }
    let dump = controls
        .iter()
        .map(|node| {
            let b = node.bounds.expect("缺包围盒");
            format!(
                "{}: x={} y={} w={} h={}",
                node.id, b.x, b.y, b.width, b.height
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[#185 {label}] 发送条内在场控件 {} 个 / 容器 {} 个 / {pair_count} 对 ⇒ 相交 0 对：\n{dump}\n容器（按设计包住子节点，不参与两两判定）：{}",
        controls.len(),
        containers.len(),
        containers
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// 一档的完整读数与断言（在**独立线程**里调用 —— Tier-1 平台是线程局部的）。
#[allow(clippy::too_many_lines)]
fn check_tier(width: u32, height: u32) {
    let base = project();
    let (live, port) = live_with_port(Size::new(width, height), &base, "#185 三档");
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 3（设备架）");
    assert!(
        !live.ui().get_send_create_available(),
        "无 aux ⇒ 按钮必须是**渲染上**的禁用态（不是只把回调拦住）"
    );
    let tree = live.tree();

    let strip = bounds(tree, "send-strip");
    let empty = bounds(tree, "send-empty");
    let button = bounds(tree, "send-create-button");
    let reason = bounds(tree, "send-create-reason");

    // ---- 判据 ③：空态仍可见、逐字、在裁剪区之内。
    let empty_node = tree
        .find_by_id("send-empty")
        .expect("空态必须还在树里（⛔ 修重叠不是删空态）");
    assert_eq!(empty_node.label, "无发送", "空态文案必须逐字 `无发送`");
    assert_eq!(
        strip.3, 148,
        "[#185 {width}x{height}] 空态 send-strip 高度必须是 148（= 126 + 22）"
    );

    // ---- 判据 ①：两个节点不相交，且空态文案在按钮**上方**。
    assert!(
        !intersect(empty, button),
        "[#185 {width}x{height}] `send-empty` {empty:?} 与 `send-create-button` {button:?} **相交**"
    );
    let empty_bottom = empty.1 + empty.3 as i32;
    let gap = button.1 - empty_bottom;
    assert!(
        gap >= 0,
        "[#185 {width}x{height}] 空态文案必须在按钮上方：空态下沿 {empty_bottom} > 按钮上沿 {}",
        button.1
    );

    // ---- 判据 ②：段内所有在场控件两两不相交。
    report_line(&assert_pairwise(
        tree,
        strip,
        EXPECTED_EMPTY,
        &format!("{width}x{height} 空态"),
    ));

    // ---- 判据 ⑤：三档逐格登记值（空态文案在上、按钮在下）。
    let want: Bounds = match width {
        1920 => (444, 799, 380, 148),
        1440 => (444, 691, 380, 148),
        1024 => (444, 631, 236, 148),
        other => panic!("没有登记 {other} 档的几何"),
    };
    assert_eq!(strip, want, "[#185 {width}x{height}] send-strip 包围盒");
    let want_empty: Bounds = (452, want.1 + 80, want.2 - 16, 20);
    let want_button: Bounds = (452, want.1 + 104, 160, 22);
    let want_reason: Bounds = (452, want.1 + 130, want.2 - 16, 14);
    assert_eq!(
        empty, want_empty,
        "[#185 {width}x{height}] send-empty 包围盒"
    );
    assert_eq!(
        button, want_button,
        "[#185 {width}x{height}] send-create-button 包围盒"
    );
    assert_eq!(
        reason, want_reason,
        "[#185 {width}x{height}] send-create-reason 包围盒（必须在裁剪区内）"
    );
    assert_eq!(
        tree.find_by_id("send-create-reason")
            .expect("无 aux ⇒ 逐字原因必须在树里")
            .label,
        SEND_CREATE_NO_AUX_REASON,
        "不可用原因必须逐字来自投影"
    );

    // ---- 判据 ④：禁用不是只画个灰 —— 直接调用回调也不提交。
    let before = port.display();
    let before_edges = port.project().routing_graph.edges.len();
    live.ui().invoke_send_create();
    let after = port.display();
    assert_eq!(
        after.commit_count, before.commit_count,
        "无 aux 时直接调用 `send-create` 不得提交"
    );
    assert_eq!(
        port.project().routing_graph.edges.len(),
        before_edges,
        "无 aux 时 `edges` 一位都不能变"
    );

    report_line(&format!(
        "[#185 {width}x{height}] send-empty {empty:?}；send-create-button {button:?}；\
         空态下沿 {empty_bottom} ≤ 按钮上沿 {} ⇒ 相交 = false；间距 {gap}px；\
         send-create-reason {reason:?}；send-strip {strip:?}",
        button.1
    ));
    report_line(&format!(
        "[#185 {width}x{height} 无 aux] available={}（渲染禁用）；原因逐字={:?}；\
         直接调用 send-create ⇒ commit Δ{}；edges {} → {}",
        live.ui().get_send_create_available(),
        tree.find_by_id("send-create-reason").expect("原因").label,
        after.commit_count - before.commit_count,
        before_edges,
        port.project().routing_graph.edges.len()
    ));
}

// ===========================================================================
// 判据 ①＋②＋③＋⑤：三档 —— 空态与按钮不重叠 / 段内两两不相交 / 原因不被裁
// ===========================================================================
#[test]
fn the_empty_label_and_the_create_button_never_overlap() {
    for &(width, height) in TIERS {
        // ⚠ Tier-1 平台是**线程局部**的，一根线程只能装一次 ⇒ 每一档在自己的线程里跑。
        if let Err(payload) = std::thread::spawn(move || check_tier(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ④（另一条腿）：有 aux、无发送 ⇒ 按钮可用，点一次 Δcommit = 1
// ===========================================================================
#[test]
fn the_create_button_still_commits_exactly_once_when_an_aux_track_exists() {
    let base = project_with_aux_but_no_send();
    let view = ViewState::from_project(&base).expect("投影");
    assert!(
        view.send_create_available,
        "有 `AuxReturn` 轨 ⇒ 按钮必须可用（否则本腿证不到 Δ1）"
    );
    assert!(view.sends.is_empty(), "夹具必须从 0 条发送开始");
    let (live, port) = live_with_port(Size::new(1440, 900), &base, "#185 有 aux");
    let tree = live.tree();

    let before = port.display();
    let before_edges = port.project().routing_graph.edges.len();
    let empty_before = bounds(tree, "send-empty");
    let button_before = bounds(tree, "send-create-button");
    assert!(!intersect(empty_before, button_before), "空态下仍不许重叠");
    assert!(
        tree.find_by_id("send-create-reason").is_none(),
        "可用时逐字原因 `visible: false` ⇒ 不得在树里"
    );

    live.ui().invoke_send_create();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "有 aux 时点一次 = 恰好一次提交"
    );
    assert_eq!(
        port.project().routing_graph.edges.len(),
        before_edges + 1,
        "`edges` 必须多一条"
    );
    // 演示工程本来就有 `BusToMaster` 边 ⇒ 只数 `SendToAux`，⛔ 不假设"第一条边就是新边"。
    let send_edges: Vec<_> = port
        .project()
        .routing_graph
        .edges
        .values()
        .filter(|edge| edge.kind == RoutingKind::SendToAux)
        .cloned()
        .collect();
    assert_eq!(send_edges.len(), 1, "点一次只该多一条 `SendToAux`");
    assert_eq!(
        send_edges[0].gain_db, None,
        "新发送的增益必须是单位增益（`None`）"
    );

    // 发送数 0 → 1 之后：空态撤下、按钮回到 `82 + 40n` 那条式子。
    // 空态的让位 22px 被收回、再按 40px 行距下移 ⇒ 净 **+18px**（⛔ 不是 +40）。
    let after_tree = runtime_tree(live.ui());
    let button_after = bounds(&after_tree, "send-create-button");
    assert!(
        after_tree.find_by_id("send-empty").is_none(),
        "有发送时空态必须撤下（`visible: n == 0`）"
    );
    assert_eq!(
        button_after.1,
        button_before.1 + 18,
        "空态让位的 22px 收回后再按 40px 行距下移 ⇒ 净 +18px"
    );

    // 判据 ② 的第二档：有发送时行 / 滑块 / 读数 / 删除键**也在场** ⇒ 同样逐对断言。
    let strip_after = bounds(&after_tree, "send-strip");
    report_line(&assert_pairwise(
        &after_tree,
        strip_after,
        EXPECTED_FILLED,
        "1440x900 有 1 条发送",
    ));

    report_line(&format!(
        "[#185 1440 有 aux] available=true；空态 {empty_before:?} / 按钮 {button_before:?} ⇒ 不相交；\
         原因不在树里 = true；点击后 commit {} → {}（Δ1）、edges {} → {}（SendToAux 1 条，gain_db=None）；\
         send-empty 撤下 = true；按钮 y {} → {}",
        before.commit_count,
        after.commit_count,
        before_edges,
        port.project().routing_graph.edges.len(),
        button_before.1,
        button_after.1
    ));
}
