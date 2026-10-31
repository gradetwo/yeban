//! `[UI-A11Y-003]` §7.3 / `[TEST-SPEC-005]` 用例 ③：模态**焦点闭环**与关闭后**焦点复位**。
//!
//! ## 为什么这里能按"焦点值"断言（既有 a11y 判据做不到）
//!
//! 控件树里没有焦点字段（`yeban_ui_test_port::tree` 与 `yeban_ui_mcp::tree` 里 `focus` 命中 0），
//! 因此 `tests/a11y_screen_reader.rs` 只能按"寻址面"近似。本切片在 `ui/app.slint` 里把两个
//! `FocusScope.has-focus` **镜像**成 MainWindow 的只读属性：
//!
//! * `undo-tree-focus-trapped` —— 模态自己的 `FocusScope`（`undo_tree_trap`）是否持有焦点；
//! * `shortcut-layer-focused` —— 唯一键盘事件源 `key-handler` 是否持有焦点。
//!
//! 两个属性只是 Slint 焦点状态的**读数**：界面自己不读它们 ⇒ 不引入第二份状态。
//!
//! ## 判据（一条链，五个读数）
//!
//! ① 起点：快捷键层有焦点、模态未持有；
//! ② 打开（端口 + `apply_undo`，与宿主同一个开关）⇒ 焦点**真的**移进模态；
//! ③ 模态内按 `3` ⇒ 仍从**同一个**键盘入口到达宿主（`active-tool` = 3）；
//! ④ 模态内按 `Tab` ⇒ 焦点**不**轮转出去，也不触发画布语义的切视图（视图不变）；
//! ⑤ `Esc` 关闭 ⇒ 焦点复位到快捷键层，随后 `1` 又能到达宿主（`active-tool` = 1）。
//!
//! ⛔ 本判据**不**声称"模态内各控件之间的 Tab 顺序"：本骨架里模态的按钮不是 `FocusScope`
//! （只有 `TouchArea`），闭环 = 焦点停在模态这一个作用域上。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与既有撤销判据同一个夹具常量）。
const NOW: u64 = 1_760_000_000_000;

#[test]
fn the_undo_tree_traps_focus_and_restores_it_to_the_shortcut_layer() {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:时光机焦点闭环>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let mut ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            proposal_source: yeban_app::proposals::ProposalSource::none(),
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    yeban_app::host::apply_undo(ui.ui(), &port);
    ui.pump_meters();

    // ---- ① 起点 ----
    let mut trace: Vec<String> = vec![format!(
        "起点 trapped={} key-layer={}",
        ui.ui().get_undo_tree_focus_trapped(),
        ui.ui().get_shortcut_layer_focused()
    )];
    assert!(
        !ui.ui().get_undo_tree_focus_trapped(),
        "① 起点：时光机没开 ⇒ 模态的 FocusScope 不得持有焦点"
    );
    assert!(
        ui.ui().get_shortcut_layer_focused(),
        "① 起点：快捷键层必须持有焦点（`forward-focus: key-handler` 的机械证据）"
    );

    // ---- ② 打开：焦点进模态 ----
    port.perform(UiAction::OpenUndoTree);
    yeban_app::host::apply_undo(ui.ui(), &port);
    ui.pump_meters();
    assert!(
        port.undo_tree_open(),
        "② 经端口打开之后端口读数必须是 `true`"
    );
    assert!(
        ui.tree_snapshot().contains("undo-tree-modal"),
        "② 时光机开着 ⇒ 它必须进运行时树（否则本判据的 ⑤ 步没有判别力）"
    );
    assert!(
        ui.ui().get_undo_tree_focus_trapped(),
        "② 打开之后焦点必须落在模态的 `undo_tree_trap` 上（`changed undo-tree-open` 的 `focus()`）"
    );
    assert!(
        !ui.ui().get_shortcut_layer_focused(),
        "② 打开之后快捷键层不得同时持有焦点 —— 焦点是**唯一**的"
    );
    trace.push(format!(
        "打开后 trapped={} key-layer={}",
        ui.ui().get_undo_tree_focus_trapped(),
        ui.ui().get_shortcut_layer_focused()
    ));

    // ---- ③ 模态内按键仍到达宿主 ----
    ui.dispatch_key_press(KeyCode::Character('3'))
        .expect("③ `3` 必须落到真实窗口（Interactive 档）");
    assert_eq!(
        ui.ui().get_active_tool(),
        3,
        "③ 模态内按 `3` 必须仍经同一个键盘入口到达宿主（工具矩阵第 3 行）"
    );
    assert!(
        ui.ui().get_undo_tree_focus_trapped(),
        "③ 普通键不得把焦点带出模态"
    );
    trace.push(format!(
        "`3` 后 active-tool={} trapped={}",
        ui.ui().get_active_tool(),
        ui.ui().get_undo_tree_focus_trapped()
    ));

    // ---- ④ Tab 被吞：焦点不轮转、也不切视图 ----
    let view_before = ui.ui().get_arrangement_view();
    ui.dispatch_key_press(KeyCode::Tab)
        .expect("④ `Tab` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_arrangement_view(),
        view_before,
        "④ 模态内 `Tab` 不得触发画布语义的切视图（`accept` 把这一键吞在模态里）"
    );
    assert!(
        ui.ui().get_undo_tree_focus_trapped(),
        "④ `Tab` 之后焦点必须仍在模态内（闭环）"
    );
    assert!(
        !ui.ui().get_shortcut_layer_focused(),
        "④ `Tab` 不得把焦点送回快捷键层"
    );
    trace.push(format!(
        "`Tab` 后 视图={} trapped={} key-layer={}",
        ui.ui().get_arrangement_view(),
        ui.ui().get_undo_tree_focus_trapped(),
        ui.ui().get_shortcut_layer_focused()
    ));

    // ---- ⑤ Esc 关闭 + 焦点复位 ----
    ui.dispatch_key_press(KeyCode::Escape)
        .expect("⑤ `Esc` 必须落到真实窗口");
    ui.pump_meters();
    assert!(!port.undo_tree_open(), "⑤ `Esc` 必须关掉时光机");
    assert!(
        !ui.tree_snapshot().contains("undo-tree-modal"),
        "⑤ 关闭之后模态必须离开运行时树"
    );
    assert!(
        !ui.ui().get_undo_tree_focus_trapped(),
        "⑤ 关闭之后模态不得再持有焦点"
    );
    assert!(
        ui.ui().get_shortcut_layer_focused(),
        "⑤ 关闭之后焦点必须复位到快捷键层（`changed undo-tree-open` 的另一半）"
    );
    ui.dispatch_key_press(KeyCode::Character('1'))
        .expect("⑤ `1` 必须落到真实窗口");
    assert_eq!(
        ui.ui().get_active_tool(),
        1,
        "⑤ 复位之后单键快捷键必须重新到达宿主（矩阵第 1 行）"
    );
    trace.push(format!(
        "`Esc` 后 trapped={} key-layer={}；`1` 后 active-tool={}",
        ui.ui().get_undo_tree_focus_trapped(),
        ui.ui().get_shortcut_layer_focused(),
        ui.ui().get_active_tool()
    ));

    report_line(&format!("[undo-focus] {}", trace.join(" ⇒ ")));
}
