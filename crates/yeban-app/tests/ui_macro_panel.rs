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
    let project = yeban_model::samples::filled_project();
    // 展示轨 = 身份序第一条非主总线轨（`bridge::display_track` 的唯一口径）。
    let display_id = yeban_app::bridge::display_track(&project, None)
        .expect("filled_project 必须有一条展示轨")
        .id;
    let start_value = project
        .tracks
        .get(&display_id)
        .expect("展示轨必须在工程里")
        .macros
        .first()
        .expect("filled_project 的展示轨必须带一条宏")
        .value;
    assert_eq!(start_value, 0.5, "夹具的宏位置变了：本判据的读数要一起改");

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据: 宏面板>", "yeban-app", project.clone(), 1).expect("打开撤销会话"),
    ));
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 2,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + 宏接线");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let mut plane = live_ui.into_control_plane(Permission::Interactive);
    let ui = &window;

    // ---- ① 默认关着 ----
    let before = tree_ids(&mut plane);
    assert!(
        !before.iter().any(|id| id == "macro-panel"),
        "未打开时抽屉不该在运行时控件树里（默认金帧因此不变）"
    );
    assert!(!ui.get_macro_panel_open(), "初始必须是关闭态");
    assert!(
        before.iter().any(|id| id == "macro-open-button"),
        "Tab 3 的「宏 (N)」入口必须在场"
    );
    {
        let (tree, _) = plane.plane().tree().expect("ui/tree");
        let entry = node(&tree, "macro-open-button");
        assert_eq!(entry.role, ElementKind::Button.accessible_role());
        assert_eq!(
            entry.label, "打开宏面板（1 个宏）",
            "入口按钮的无障碍名必须带上投影里的宏数"
        );
    }

    // ---- ② 打开 ⇒ 七个 ID 可寻址，角色与注册表一致 ----
    ui.set_macro_panel_open(true);
    assert!(ui.get_macro_panel_open(), "打开开关必须真的落在属性上");
    let (tree, _) = plane.plane().tree().expect("ui/tree");
    for (id, kind) in PANEL_IDS {
        let node = node(&tree, id);
        assert_eq!(
            node.role,
            kind.accessible_role(),
            "`{id}` 的角色必须与注册表一致"
        );
        assert!(
            !node.label.trim().is_empty(),
            "`{id}` 可见时必须有无障碍名称"
        );
        assert!(
            plane.registry().contains(id),
            "`{id}` 不在装配时的注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
        );
    }
    let unknown: Vec<&str> = tree
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .filter(|id| !plane.registry().contains(id))
        .collect();
    assert!(
        unknown.is_empty(),
        "运行时树里出现了注册表没登记的 ID: {unknown:?}"
    );
    assert_eq!(
        ui.get_macro_names().row_count(),
        1,
        "宏行数必须等于投影里的宏数"
    );

    // ---- ③ 手势：拖动只写视图态，松手恰好一次 SetMacro ----
    let commits_before = port.display().commit_count;
    ui.invoke_macro_grab(0, 0.75);
    assert!(
        (f32_row(&ui.get_macro_values(), 0) - 0.75).abs() < f32::EPSILON,
        "按下即定位（单击也是一次编辑）"
    );
    ui.invoke_macro_drag(0, 0.9);
    assert!(
        (f32_row(&ui.get_macro_values(), 0) - 0.9).abs() < f32::EPSILON,
        "拖动期只改预览值"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "拖动期 ⛔ 不提交工程内容"
    );
    ui.invoke_macro_release(0);
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次手势 = 恰好一次 commit_ops"
    );
    let committed = port.project();
    let new_value = committed
        .tracks
        .get(&display_id)
        .expect("展示轨必须还在工程里")
        .macros
        .first()
        .expect("宏必须还在")
        .value;
    assert!(
        (new_value - 0.9).abs() < f32::EPSILON,
        "权威工程里的宏位置必须是 0.9，实得 {new_value}"
    );
    assert_eq!(
        ui.get_macro_value_labels().row_count(),
        1,
        "提交之后必须重投影"
    );
    {
        let (tree, _) = plane.plane().tree().expect("ui/tree");
        assert_eq!(
            node(&tree, "macro-0-value").label,
            "90%",
            "位置文本必须跟着权威工程变"
        );
    }

    // 撤销回到按下前的值 ⇒ 这一步真的是可撤销的编辑。
    let _ = port.perform(UiAction::Undo);
    let undone = port
        .project()
        .tracks
        .get(&display_id)
        .expect("展示轨必须还在工程里")
        .macros
        .first()
        .expect("宏必须还在")
        .value;
    assert!(
        (undone - start_value).abs() < f32::EPSILON,
        "撤销之后必须回到 {start_value}，实得 {undone}"
    );

    // ---- ④ 空态：宏行全部消失，只剩空态文案 ----
    ui.set_macro_names(slint::ModelRc::new(slint::VecModel::from(Vec::<
        slint::SharedString,
    >::new())));
    ui.set_macro_values(slint::ModelRc::new(
        slint::VecModel::from(Vec::<f32>::new()),
    ));
    ui.set_macro_value_labels(slint::ModelRc::new(slint::VecModel::from(Vec::<
        slint::SharedString,
    >::new())));
    let (tree, _) = plane.plane().tree().expect("ui/tree");
    node(&tree, "macro-empty-state");
    assert!(
        !tree.nodes.iter().any(|node| node.id == "macro-0-row"),
        "零宏时不许还画着一行"
    );

    // ---- ⑤ 入口接线：控制台的回调必须真的被转发到打开开关 ----
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    assert!(
        app.contains("open-macro-panel() => {") && app.contains("root.macro-panel-open = true;"),
        "「宏 (N)」按钮的回调必须落到 `macro-panel-open` 上"
    );
}
