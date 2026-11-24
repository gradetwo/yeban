//! **回滚键接线判据（界面侧）** —— 走带区 ↻ 真的回退一个 Commit。
//!
//! ## 本文件要证的句子
//!
//! `revert-to-last-commit` 回调 **不是一个只显示不动作的图标**：触发它 = 对权威撤销会话
//! 执行**一次** `UiAction::Undo`。语义由模型不变量背书 —— 本实现的每次提交恰好携带
//! 一条 `Op::Batch`（`crates/yeban-mcp/src/undo_session.rs` 的"每次提交恰好一条 op"），
//! 因此"一步撤销 = 回退一个 Commit"。
//!
//! ⭐ **按钮已死**：新走带条（`ui/transport_bar.slint`）没有 `transport-revert-button`
//! （它随旧 `ui/transport.slint` 一起删了），活树里也没有它。触发入口改成直接调回调
//! —— 那正是旧按钮的 `TouchArea.clicked` 与 `accessible-action-default` 调的那条
//! （`root.revert-to-last-commit();`），经 `host::wire_undo` 接的
//! `on_revert_to_last_commit` 落到 `UndoPort`，随后 `refresh_undo` 把**模型读数**
//! 写回界面、并从回退后的工程重投影。⛔ 不是第二条实现。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（落点还在） | 活树里没有按钮；注册表仍登记它；`app.slint` 声明回调、`wire_undo` 接了它 |
//! | ②（真触发 → 真回退） | 先提交一条夹具编辑；触发回调 ⇒ `undone == 1`、工程指纹逐字节回到原始工程 |
//! | ③（动作日志） | 这次触发留下一条 `action == "undo"` 且 `changed_project()` 的记录 |
//! | ④（显示态来自模型） | 触发后界面读数为 `undo-undone == 1` / `undo-can-undo == false` |
//! | ⑤（没有历史就拒绝） | 再触发一次 ⇒ `Refused`，工程不变，动作日志**照样**留痕（2 条） |
//!
//! ## 为什么控件树只用来**寻址**
//!
//! 元素在树里不等于动作真的发生（三方对齐矩阵的错位 5 警告过这种假接线）。
//! "真的回退了一版"由**工程指纹**与**动作日志**证明；源文件层的正反探针
//! （`app.slint` 的回调声明 / `wire_undo` 的接线）由本文件判据 ① 看守，
//! 旧按钮那一侧（`clicked` / `accessible-action-default` / `app.slint` 转发）由
//! `crates/yeban-app/src/elements.rs` 的
//! `the_remaining_transport_document_actions_are_not_wired_today` 机械看守。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/console_maximize.rs` / `tests/undo_wiring_ui.rs` 同款）。

#![allow(unused)]
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{ActionOutcome, UiAction, UndoPort, UndoSession, project_fingerprint};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

/// 与 `tests/undo_wiring_ui.rs` 同款的时间戳（确定性）。
const NOW: u64 = 1_760_000_000_000;

#[test]
fn a_real_click_on_the_revert_button_rolls_back_exactly_one_commit() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");

    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<回滚键判据>", "yeban-app", project, NOW).expect("打开撤销会话"),
    ));
    host::wire_undo(live.ui(), &undo);
    host::apply_undo(live.ui(), &undo);
    let pristine = project_fingerprint(&filled_project()).expect("指纹");

    // ---- ① 可寻址：⭐ 活树里已经没有这枚按钮了 ----
    //
    // 新走带条（`ui/transport_bar.slint`）没有 `transport-revert-button`——
    // 按钮随旧 `ui/transport.slint` 一起死了。但它的回调还在：
    // `app.slint` 声明 `callback revert-to-last-commit();`，`wire_undo` 接
    // `on_revert_to_last_commit → UiAction::Undo`（与旧按钮的 `TouchArea.clicked`
    // 逐字同一条落点）。本判据钉的就是这条**活着的落点**，⛔ 不假装按钮还在。
    // 注册表仍登记它（`elements.rs` 的静态清单覆盖旧走带条）—— 那是覆盖判据
    // 方向 1 的登记，⛔ 不是"按钮还在树里"的证据。
    {
        let tree = live.refresh_tree(None).expect("必须能重抓控件树");
        assert!(
            tree.find_by_id("transport-revert-button").is_none(),
            "活窗口的运行时树里不许再有 `transport-revert-button`（按钮已随旧走带条删除）"
        );
        let registry = ElementRegistry::from_view(&view);
        let meta = registry
            .get("transport-revert-button")
            .expect("注册表仍登记 `transport-revert-button`（静态清单覆盖旧走带条）");
        assert_eq!(meta.kind, ElementKind::Button);
        assert_eq!(meta.label, "回滚到上一个 Commit");
        let app = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
        )
        .expect("读 app.slint");
        assert!(
            app.contains("callback revert-to-last-commit();"),
            "`app.slint` 必须声明 `callback revert-to-last-commit();`（按钮死了，回调是落点）"
        );
        let host = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
        )
        .expect("读 host.rs");
        assert!(
            host.contains("on_revert_to_last_commit"),
            "`wire_undo` 必须接 `on_revert_to_last_commit`（否则回调无人听）"
        );
    }
    assert!(
        !live.ui().get_undo_can_undo(),
        "装配态没有可撤销的步骤（界面读数来自模型）"
    );

    // ---- ② 先造一条真实提交：与 `undo_wiring_ui.rs` 同一个夹具、同一串 op ----
    let fixture = wiring_fixture(&undo.project()).expect("夹具");
    undo.commit_ops(NOW + 1, "回滚键判据", vec![fixture.op()])
        .expect("提交");
    let edited = undo.fingerprint().expect("指纹");
    assert_ne!(edited, pristine, "夹具必须真的改了工程");
    host::apply_undo(live.ui(), &undo);
    assert!(
        live.ui().get_undo_can_undo(),
        "提交之后界面必须从模型读数里看到「可撤销」"
    );

    // ---- ②③④ 触发回滚：⭐ 按钮没了，调它生前调的那条回调 ----
    // 旧按钮的 `TouchArea.clicked` 与 `accessible-action-default` 都是
    // `root.revert-to-last-commit();`—— 直接调回调就是同一条落点（`wire_undo` 接的
    // `on_revert_to_last_commit → UiAction::Undo`），⛔ 不是第二条实现。
    live.ui().invoke_revert_to_last_commit();

    assert_eq!(
        undo.display().undone,
        1,
        "一次点击只回退**一步**（每次提交恰好一条 `Op::Batch` ⇒ 一个 Commit）"
    );
    assert_eq!(
        undo.fingerprint().expect("指纹"),
        pristine,
        "点击 ↻ 之后工程必须逐字节回到原始工程"
    );
    let record = undo.last_record().expect("点击必须留下动作日志");
    assert_eq!(record.action, "undo", "动作日志的落点必须是既有的撤销动作");
    assert!(
        record.changed_project(),
        "动作日志必须证明工程真的变了: {record:?}"
    );
    assert_eq!(record.fingerprint_before.as_deref(), Some(edited.as_str()));
    assert_eq!(record.fingerprint_after.as_deref(), Some(pristine.as_str()));

    // 界面读数与模型读数同源（`refresh_undo` 是唯一写者）。
    assert_eq!(
        live.ui().get_undo_undone(),
        1,
        "界面显示的「已撤销步数」必须来自模型读数"
    );
    assert!(
        !live.ui().get_undo_can_undo(),
        "已经没有可撤销的步骤，界面必须如实显示「不能撤销」"
    );

    // ---- ⑤ 没有历史就拒绝：再触发一次工程不变，但**照样**留痕 ----
    live.ui().invoke_revert_to_last_commit();

    let refused = undo.last_record().expect("拒绝也要留痕");
    assert!(
        !refused.changed_project(),
        "没有历史时的点击不得改工程: {refused:?}"
    );
    assert_eq!(undo.records().len(), 2, "两次触发两条记录");
    assert_eq!(
        undo.fingerprint().expect("指纹"),
        pristine,
        "拒绝之后工程必须仍是回退后的那一版"
    );
    assert_eq!(undo.display().undone, 1, "拒绝不动游标");
    // 顺带确认拒绝的形状就是既有的 `Refused`（⛔ 不是悄悄成功）。
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Refused { .. }
    ));
}
