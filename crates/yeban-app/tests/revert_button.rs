//! **回滚键接线判据（界面侧）** —— 走带区 ↻ 真的回退一个 Commit。
//!
//! ## 本文件要证的句子
//!
//! `transport-revert-button` **不是一个只显示不动作的图标**：点它 = 对权威撤销会话
//! 执行**一次** `UiAction::Undo`。语义由模型不变量背书 —— 本实现的每次提交恰好携带
//! 一条 `Op::Batch`（`crates/yeban-mcp/src/undo_session.rs` 的"每次提交恰好一条 op"），
//! 因此"一步撤销 = 回退一个 Commit"。
//!
//! 判据走**真实指针注入**（不是直接调 Rust 回调）：命中 `revert_area` 的 `clicked`，
//! 经 `host::wire_undo` 接的 `revert-to-last-commit` 落到 `UndoPort`，随后
//! `refresh_undo` 把**模型读数**写回界面、并从回退后的工程重投影。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 树里有 `transport-revert-button`、角色 = `Button`、标签与注册表逐字一致 |
//! | ②（真点击 → 真回退） | 先提交一条夹具编辑；点击 ↻ ⇒ `undone == 1`、工程指纹逐字节回到原始工程 |
//! | ③（动作日志） | 这次点击留下一条 `action == "undo"` 且 `changed_project()` 的记录 |
//! | ④（显示态来自模型） | 点击后界面读数为 `undo-undone == 1` / `undo-can-undo == false` |
//! | ⑤（没有历史就拒绝） | 再点一次 ⇒ `Refused`，工程不变，动作日志**照样**留痕（2 条） |
//!
//! ## 为什么控件树只用来**寻址**
//!
//! 元素在树里不等于动作真的发生（三方对齐矩阵的错位 5 警告过这种假接线）。
//! "真的回退了一版"由**工程指纹**与**动作日志**证明；源文件层的正反探针
//! （`clicked` / `accessible-action-default` / `app.slint` 转发）由
//! `crates/yeban-app/src/elements.rs` 的
//! `the_remaining_transport_document_actions_are_not_wired_today` 机械看守。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/console_maximize.rs` / `tests/undo_wiring_ui.rs` 同款）。

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

    // ---- ① 可寻址：节点在场、角色 / 标签与注册表一致 ----
    {
        let tree = live.refresh_tree(None).expect("必须能重抓控件树");
        let node = tree
            .find_by_id("transport-revert-button")
            .expect("`transport-revert-button` 必须在运行时控件树里");
        assert_eq!(
            node.role.as_str(),
            ElementKind::Button.accessible_role(),
            "回滚键的角色必须与注册表一致"
        );
        assert_eq!(
            node.label, "回滚到上一个 Commit",
            "无障碍名必须与注册表一致"
        );
        let meta = ElementRegistry::from_view(&view)
            .get("transport-revert-button")
            .expect("`transport-revert-button` 必须在装配时的注册表里（[UI-TEST-001]）");
        assert_eq!(meta.kind, ElementKind::Button);
        assert_eq!(meta.label, "回滚到上一个 Commit");
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

    // ---- ②③④ 真实点击：按下 → 松手（命中 `revert_area`，不是直接调 Rust 回调） ----
    live.dispatch_pointer_down("transport-revert-button", 16.0, 16.0, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    live.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");

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

    // ---- ⑤ 没有历史就拒绝：再点一次工程不变，但**照样**留痕 ----
    live.dispatch_pointer_down("transport-revert-button", 16.0, 16.0, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    live.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");

    let refused = undo.last_record().expect("拒绝也要留痕");
    assert!(
        !refused.changed_project(),
        "没有历史时的点击不得改工程: {refused:?}"
    );
    assert_eq!(undo.records().len(), 2, "两次点击两条记录");
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
