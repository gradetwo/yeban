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
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:时光机×关闭>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let window_port = Rc::clone(&port);
    let mut live = LivePort::new(
        Size::new(1440, 900),
        Permission::Interactive,
        None,
        move || {
            let ui = host::build_main_window(&view, &scene)?;
            // 生产 GUI 的装配顺序（`src/main.rs`）：先接撤销入口，再注入显示态。
            host::wire_undo(&ui, &window_port);
            host::apply_undo(&ui, &window_port);
            Ok(ui)
        },
    )
    .expect("Tier-1 平台 + 生产窗口构造入口");

    // ---- ① 打开：端口 + 运行时树 ----
    port.perform(UiAction::OpenUndoTree);
    host::apply_undo(live.ui(), &port);
    assert!(port.undo_tree_open(), "① 前置：端口读数必须是打开");
    let tree = live.refresh_tree(None).expect("控件树").dump_json();
    assert!(
        tree.contains("undo-tree-modal"),
        "① 前置：模态必须在运行时树里（否则 ② 没有可点的按钮）"
    );

    // ---- ② 真点击 ×（生产装配：wire_undo 已接） ----
    live.dispatch_pointer_down("undo-tree-close-button", 12.0, 12.0, PointerButton::Left)
        .expect("② × 按下必须落到真实窗口（Interactive 档）");
    live.dispatch_pointer_up(PointerButton::Left)
        .expect("② × 抬起");

    // ---- ③ 端口读数 + 动作日志 + 树 ----
    assert!(
        !port.undo_tree_open(),
        "③ × 必须把端口读数写成关闭 —— 改动前端口仍记着「开着」（只写了界面属性）"
    );
    let last = port.last_record().expect("③ × 之后必须有动作记录");
    assert_eq!(
        last.action, "close-undo-tree",
        "③ × 必须落到 `UiAction::CloseUndoTree`（与 `Esc` 同一个动作）"
    );
    let tree = live.refresh_tree(None).expect("控件树").dump_json();
    assert!(
        !tree.contains("undo-tree-modal"),
        "③ 关闭之后模态必须离开运行时树"
    );

    // ---- ④ 撤销路径的同一个回写点不得把弹窗复活 ----
    host::apply_undo(live.ui(), &port);
    let tree = live.refresh_tree(None).expect("控件树").dump_json();
    assert!(
        !tree.contains("undo-tree-modal"),
        "④ 端口已是关闭 ⇒ 再次 `apply_undo` 不得把弹窗翻回来（这正是改动前的缺陷）"
    );

    // ---- ⑤ 焦点复位 ----
    assert!(
        !live.ui().get_undo_tree_focus_trapped(),
        "⑤ × 关闭后模态不得再持有焦点"
    );
    assert!(
        live.ui().get_shortcut_layer_focused(),
        "⑤ × 关闭后焦点必须复位到唯一键盘事件源"
    );

    // ---- ⑥ 源码层接线 ----
    let app = include_str!("../ui/app.slint");
    assert!(
        app.contains("callback close-undo-tree();"),
        "⑥ `MainWindow` 必须声明 `close-undo-tree`（宿主 setter 的唯一落点）"
    );
    assert!(
        app.contains("close => { root.close-undo-tree(); }"),
        "⑥ 弹窗的 `close` 必须转给窗口回调（⛔ 不许直接写 `undo-tree-open`）"
    );
    let source = include_str!("../src/host.rs");
    assert_eq!(
        source.matches("ui.on_close_undo_tree(").count(),
        1,
        "⑥ × 关闭必须**恰好**一个接线点"
    );
}
