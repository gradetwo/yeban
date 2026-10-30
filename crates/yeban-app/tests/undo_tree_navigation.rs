//! **时光机节点点击** —— 端到端判据（`[UI-A11Y-003]` §7.3 / ADR-0001 D45 / `[UI-TEST-001]`）。
//!
//! ## 本文件要证的句子
//!
//! `ui/dialogs/undo_tree_modal.slint` 底部那句提示从「节点点击回滚未实现」改成
//! 「点击版本节点回到该版本」之后，这句话必须有一个**真的**落点：点击节点
//! `i` ⇒ 工程回到提交链上下标 `i` 的那一版。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（装配态） | 4 次真提交 ⇒ 5 个节点（含根提交）、`undone = 0`、工程 = 最后一版 |
//! | ②（点更早的节点） | 点节点 2 ⇒ `undo-many(2)`，游标 `undone = 2`、工程**逐字节** = 第 2 版 |
//! | ③（点更新的节点） | 点节点 0 ⇒ `redo-many(2)`，工程逐字节回到最后一版 |
//! | ④（点当前节点） | 再点节点 0 ⇒ **不产生动作**（⛔ 不能把 0 交给端口：`plan_undo` 会把 0 clamp 成 1 步） |
//! | ⑤（撤到底 + 点中间版本） | 点节点 4 ⇒ 撤到底（`undone = 4`、根提交态）；再点节点 3 ⇒ `redo-many(1)` 回到第 1 版 |
//! | ⑥（越界下标） | 点不存在的下标 ⇒ 不产生动作、不动工程（不猜一个版本） |
//! | ⑦（文本层接线） | `.slint` 的点击源 / 无障碍动作 / 选中态、`app.slint` 的转发、`host.rs` 的唯一接线点与 `RedoMany` |
//!
//! ## 为什么找的是"工程字节"而不是控件树文案
//!
//! D45 的纪律：动作日志与工程指纹是证据，元素 ID 只是语义寻址。本判据每一步都断言
//! `UndoPort::fingerprint()`（模型容器的确定性 SHA-256）恰好等于**那一版**的指纹。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/revert_button.rs` / `tests/playhead_follows_transport.rs` 同款）。

use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::Op;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
const NOW: u64 = 1_760_000_000_000;

#[test]
fn clicking_a_time_machine_node_restores_that_version() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("节点点击判据", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let window_port = Rc::clone(&port);
    let mut live = LivePort::new(WINDOW, Permission::Interactive, None, move || {
        let ui = host::build_main_window(&view, &scene)?;
        // 生产 GUI 的装配顺序（`src/main.rs`）：先接撤销入口，再注入显示态。
        host::wire_undo(&ui, &window_port);
        host::apply_undo(&ui, &window_port);
        Ok(ui)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");

    // ---- 4 次真提交（每次改同一个音符的力度）⇒ 5 个版本（含根提交） ----
    let fixture = wiring_fixture(&port.project()).expect("夹具：工程里必须有可改的音符");
    let mut old_velocity = fixture.old_velocity;
    let mut fingerprints = vec![port.fingerprint().expect("根提交指纹")];
    for step in 0..4_u32 {
        let new_velocity = u8::try_from(41 + step).expect("力度落在 u8 内");
        port.commit_ops(
            NOW + u64::from(step) + 1,
            &format!("版本 {step}"),
            vec![Op::ModifyNoteVelocity {
                track_id: fixture.track_id,
                clip_id: fixture.clip_id,
                note_id: fixture.note_id,
                old_vel: old_velocity,
                new_vel: new_velocity,
            }],
        )
        .expect("提交必须被接受");
        old_velocity = new_velocity;
        fingerprints.push(port.fingerprint().expect("提交后指纹"));
    }
    host::apply_undo(live.ui(), &port);

    let labels: Vec<String> = {
        use slint::Model as _;
        live.ui()
            .get_undo_node_labels()
            .iter()
            .map(|label| label.to_string())
            .collect()
    };
    assert_eq!(
        labels,
        vec![
            "版本 3".to_owned(),
            "版本 2".to_owned(),
            "版本 1".to_owned(),
            "版本 0".to_owned(),
            "open 节点点击判据".to_owned(),
        ],
        "节点标签必须 = 权威提交链（最新在前），根提交是 `open <label>`"
    );
    assert_eq!(live.ui().get_undo_node_count(), 5, "4 次提交 + 根提交");
    assert_eq!(live.ui().get_undo_undone(), 0, "装配态游标在头");
    assert_eq!(live.ui().get_undo_depth(), 4, "4 条提交 op 可撤");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        fingerprints[4],
        "装配态工程 = 最后一次提交后的那一版"
    );

    // ---- 打开时光机：节点确实进树（语义 ID `undo-tree-node-{i}`） ----
    port.perform(UiAction::OpenUndoTree);
    host::apply_undo(live.ui(), &port);
    // `LivePort` 的树快照入口是 `refresh_tree(registry) -> &ControlTree`
    // （`tests/clip_selection.rs` 同款）；`dump_json()` 把语义 ID 摊平成字符串。
    let tree = live
        .refresh_tree(None)
        .expect("必须能重抓控件树")
        .dump_json();
    for index in 0..5 {
        assert!(
            tree.contains(&format!("undo-tree-node-{index}")),
            "链上有 5 个版本 ⇒ `undo-tree-node-{index}` 必须在树里"
        );
    }
    assert!(
        !tree.contains("undo-tree-node-5"),
        "链上只有 5 个版本 ⇒ 第 6 个节点不许存在"
    );
    assert!(
        !tree.contains("undo-tree-truncation-note"),
        "5 ≤ 画布上限 6 ⇒ 不允许出现截断报数"
    );

    // ---- ② 点更早的节点 2 ⇒ undo-many(2) ----
    live.ui().invoke_undo_node_selected(2);
    assert_eq!(port.display().undone, 2, "点节点 2 ⇒ 游标后退 2 步");
    assert_eq!(port.display().undoable, 2, "4 条 op - 2 步 = 2 步可撤");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        fingerprints[2],
        "工程必须**逐字节**等于第 2 版（⛔ 不是只动了游标）"
    );
    assert_eq!(live.ui().get_undo_undone(), 2, "界面游标读数跟随模型");
    assert_eq!(live.ui().get_undo_depth(), 2, "界面可撤步数跟随模型");
    let record = port.last_record().expect("动作日志");
    assert_eq!(record.action, UiAction::UndoMany(2).name(), "动作名");
    assert_eq!(record.undone_before, 0);
    assert_eq!(record.undone_after, 2);

    // ---- ③ 点更新的节点 0 ⇒ redo-many(2) ----
    live.ui().invoke_undo_node_selected(0);
    assert_eq!(port.display().undone, 0, "点节点 0 ⇒ 回到头");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        fingerprints[4],
        "工程必须逐字节回到最后一版"
    );
    let record = port.last_record().expect("动作日志");
    assert_eq!(record.action, UiAction::RedoMany(2).name(), "动作名");
    assert_eq!(record.undone_before, 2);
    assert_eq!(record.undone_after, 0);

    // ---- ④ 点**当前**节点 ⇒ 不产生动作（0 步的守卫） ----
    let before = port.records().len();
    live.ui().invoke_undo_node_selected(0);
    assert_eq!(
        port.records().len(),
        before,
        "点在当前版本上不得产生动作：`UndoMany(0)` 会被模型 clamp 成**撤销一步**，\
         所以宿主必须先跳过"
    );
    assert_eq!(port.display().undone, 0, "游标一位不动");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        fingerprints[4],
        "工程一位不动"
    );

    // ---- ⑤ 撤到底（点节点 4 = 根提交态），再点节点 3 重做一步 ----
    live.ui().invoke_undo_node_selected(4);
    assert_eq!(port.display().undone, 4, "点根提交 ⇒ 撤到底");
    assert_eq!(port.display().undoable, 0, "撤到底之后没有可撤的 op");
    assert!(!port.display().can_undo, "模型读数：不能再撤");
    assert!(port.display().can_redo, "模型读数：可以重做");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        fingerprints[0],
        "根提交态必须逐字节等于打开会话时的那一版"
    );
    live.ui().invoke_undo_node_selected(3);
    assert_eq!(port.display().undone, 3, "点节点 3 ⇒ 重做一步");
    assert_eq!(
        port.fingerprint().expect("指纹"),
        fingerprints[1],
        "第 3 个节点 = 第 1 次提交之后的工程"
    );
    assert_eq!(
        port.last_record().expect("动作日志").action,
        UiAction::RedoMany(1).name(),
        "两版之间差一步 ⇒ 恰好 redo-many(1)"
    );

    // ---- ⑥ 越界下标：不猜一个版本、不动工程 ----
    let before = port.records().len();
    let undone = port.display().undone;
    let fingerprint = port.fingerprint();
    live.ui().invoke_undo_node_selected(99);
    live.ui().invoke_undo_node_selected(-1);
    assert_eq!(port.records().len(), before, "越界下标不得产生动作");
    assert_eq!(port.display().undone, undone, "越界下标不得移动游标");
    assert_eq!(port.fingerprint(), fingerprint, "越界下标不得改动工程");

    // ---- ⑦ 文本层：点击源 / 无障碍动作 / 选中态 / 转发 / 唯一接线点 ----
    let modal = include_str!("../ui/dialogs/undo_tree_modal.slint");
    assert!(
        modal.contains("callback node-selected(int);"),
        "弹窗必须声明 `callback node-selected(int);`"
    );
    assert!(
        modal.contains("clicked => { root.node-selected(node_index); }"),
        "每个版本节点必须有真的点击源"
    );
    assert!(
        modal.contains("accessible-action-default => { root.node-selected(node_index); }"),
        "`[UI-A11Y-003]` §7.3：无障碍动作必须与点击**逐字同一落点**"
    );
    assert!(
        modal.contains("accessible-item-selected: node_index == root.undone-depth;"),
        "选中态必须绑**游标读数**（`undone-depth`），⛔ 不是写死的 0"
    );
    assert!(
        !modal.contains("节点点击回滚未实现"),
        "旧提示「节点点击回滚未实现」必须已删除（点击现在真的回到那一版）"
    );
    assert!(modal.contains("Esc 关闭"), "底部提示仍必须承诺 Esc 关闭");

    let app = include_str!("../ui/app.slint");
    assert!(
        app.contains("callback undo-node-selected(int);"),
        "`MainWindow` 必须声明 `undo-node-selected(int)`（宿主 setter 的唯一落点）"
    );
    assert!(
        app.contains("node-selected(index) => { root.undo-node-selected(index); }"),
        "`app.slint` 必须把弹窗回调转给窗口回调"
    );

    let source = include_str!("../src/host.rs");
    assert_eq!(
        source.matches("ui.on_undo_node_selected(").count(),
        1,
        "节点点击必须**恰好**一个接线点"
    );
    for required in [
        "if target == display.undone {",
        "UiAction::UndoMany(target - display.undone)",
        "UiAction::RedoMany(display.undone - target)",
    ] {
        assert!(
            source.contains(required),
            "`host::wire_undo` 必须包含 `{required}`（节点下标 → 步数的唯一换算）"
        );
    }

    let actions = include_str!("../src/undo.rs");
    assert!(
        actions.contains("Self::RedoMany(_) => \"redo-many\","),
        "`UiAction::RedoMany` 的动作名必须进日志"
    );
    assert!(
        actions.contains("UiAction::RedoMany(steps) => self.redo_steps(steps),"),
        "`UiAction::RedoMany` 必须落到 `redo_steps`（与 `Cmd+Shift+Z` 同一个实现）"
    );
}
