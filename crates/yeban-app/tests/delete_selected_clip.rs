//! `D-UI-15`：**编曲视图里选中的剪辑删不掉** —— `Delete` / `Backspace` 接上摆放删除。
//!
//! ## 缺陷（本文件的判据就是它的机械见证）
//!
//! [`yeban_model::Op::RemoveClipPlacement`] 在模型侧早就有 apply / inverse / 前置条件
//! （`crates/yeban-model/src/ops.rs`），MCP 侧也用它的**逆** op（`AddClipPlacement`）做撤销，
//! 而 `grep -rn RemoveClipPlacement crates/yeban-app/src` 在修之前**一处都没有** ⇒
//! 界面侧没有任何路径能把剪辑从轨道上取走：剪辑只能被**移动**（`Op::MoveClipPlacement`，
//! `tests/clip_drag.rs`）。用户做完一段编排想拿掉一条摆放，只能删整条轨（连带别的剪辑）
//! 或者根本做不到 —— 这是"能选、能拖、不能删"的半截写入面。
//!
//! ## 修法（本文件钉住的口径）
//!
//! `host::delete_selected_clip_placement`：把 `Delete` / `Backspace` 在**音符选区为空**时
//! 落到编曲视图选中的那条**摆放**上，一次 `commit_ops` = 一步撤销。四条口径逐条有判据：
//!
//! | 口径 | 为什么这么定 | 判据 |
//! | :--- | :--- | :--- |
//! | 音符**优先** | `Delete` 的既有承诺是"删除选中的音符"。卷帘的作用域正是那条选中剪辑，若反过来，用户指着几颗音符按下一键会带走**整个**剪辑 | `note_selection_out_rank_the_clip_deletion` |
//! | 只在**编曲视图** | Session 视图里 `selected-clip-ulid` 只是上一次编曲视图留下的读数 ⇒ 拿它删东西 = 删掉用户此刻**看不见**的对象 | `a_selection_left_over_from_the_arrangement_view_is_not_deleted_in_session_view` |
//! | 解析不到 ⇒ **不消费** | 与 `delete_ops_for` 同一取向："把键吞掉却什么都不做，比不处理更糟" | `a_stale_or_illegal_clip_ulid_is_not_consumed` |
//! | **一次**提交 ＝ **一步**撤销 | `[ARCH-OPS-002]`；op 自带 `previous_placement` ⇒ 撤销不需要回放历史 | `backspace...` ＋ `one_undo_puts_the_placement_back_byte_for_byte` |
//!
//! ## 注入分两格（如实标注各自证明的链）
//!
//! - **按键**走真 `WindowEvent`：`ui/dispatch_key_press(keyCode="Backspace")`
//!   → `yeban_ui_test_port::dispatch_key_press_impl`（`Key::Backspace`）
//!   → `slint::platform::WindowEvent::KeyPressed { text: "\u{8}" }`
//!   → `.slint` 的 `key-handler` → `key-action` → `host::wire_keys` → `LogicalKey::from_text`
//!   → `resolve_logical` → `apply_action` → 本文件的删除。这是**用户按键那条链**本体
//!   （与 `tests/roll_edit_tools.rs` 的 `arrow_nudge_d_focus_*` 同一手法）。
//! - **选中**用 `MainWindow::invoke_clip_select(ulid)` —— 即 `.slint` 里包头 `clicked`
//!   调的那**一个**回调（`ui/workspace/arrangement_view.slint`）。本切片不改选中那条链
//!   （它由 `tests/clip_selection.rs` 钉住），因此不为它再冒一次命中测试的不确定性。
//!   ⛔ 由此本文件**不**证明"鼠标点包头再按 Delete"整条像素级链 —— 那一格由两份判据合起来覆盖。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的 ⇒ 每个 `#[test]` 自己装配一个活窗口
//! （libtest 每例一新线程，与 `tests/d_ui_13_roll_hit_scope.rs` 同一做法）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头执行交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `tests/clip_drag.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host::clip_delete_ops_for;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{METHOD_DISPATCH_KEY_PRESS, METHOD_NODE};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// `Delete` 的键文本（`\u{7f}`）。
///
/// ⚠ 它**不能**经端口注入：`yeban_ui_test_port::port::KeyCode` 有 `Backspace` 而**没有**
/// `Delete` 变体（§12.4 的键表就没列它）。于是"这一键消费与否"用
/// `MainWindow::invoke_key_action` 读 —— 那正是 `.slint` 的 `key-handler` 派发快捷键时调的
/// **同一个**回调，返回值就是宿主给出的"消费 / 不消费"。
const DELETE_TEXT: &str = "\u{7f}";

/// 一个**合法但不存在**的摆放身份（夹具 ULID 方案：`01J8ZQ` ＋ 20 位十进制序号，共 26 位）。
///
/// `filled_project()` 用的序号在 50 与几百之间，`999_999_999` 这一位构造上不可能撞车 ⇒
/// 它检验的是"身份解析不到"这一条，而不是"ULID 非法"那一条（后者另有判据）。
fn stale_clip_ulid() -> String {
    EntityId::from_str(&format!("01J8ZQ{:020}", 999_999_999_u128))
        .expect("canonical fixture ulid")
        .to_canonical_string()
}

/// 装配：真窗口 ＋ **撤销端口当装配选项传进去**（`host::wire_keys` 从第一刻起就握着端口，
/// 与产品进程 `main.rs` 同一形态）＋ 与生产同一条 `wire_clip_scope` / `wire_clip_drag`。
fn assemble() -> (
    MainWindow,
    Rc<UndoPort>,
    LiveControlPlane,
    Rc<std::cell::RefCell<yeban_app::input::InputContext>>,
) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<delete-clip>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
    ));
    let ui = build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let input = ui.input_context();
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane, input)
}

/// 权威工程里**全部**摆放：`(摆放身份 ULID, 摆放)`，按 `tracks` 的 `BTreeMap` 键序（确定）。
fn placements(project: &YebanProjectV1) -> Vec<(String, ClipPlacement)> {
    let mut out = Vec::new();
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            out.push((placement.id.to_canonical_string(), *placement));
        }
    }
    out
}

/// 按身份从权威工程读一条摆放（读不到 ⇒ `None`）。
fn placement_by_ulid(project: &YebanProjectV1, ulid: &str) -> Option<ClipPlacement> {
    placements(project)
        .into_iter()
        .find(|(id, _)| id.as_str() == ulid)
        .map(|(_, placement)| placement)
}

/// 界面上的剪辑身份数组（`clip-ulids` —— 重投影后的绘制清单）。
fn clip_ulids(window: &MainWindow) -> Vec<String> {
    window
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 把某个剪辑**选中**（走 `.slint` 包头 `clicked` 调的那一个回调；宿主自己校验身份）。
fn select_clip(window: &MainWindow, ulid: &str) {
    window.invoke_clip_select(ulid.into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        ulid,
        "夹具前提：宿主必须接受这个身份（否则下面的按键判据空转）"
    );
}

/// 注入一次**真** `WindowEvent` 的 `Backspace`（用户删除键那条链的本体）。
fn press_backspace(plane: &mut LiveControlPlane, seq: i64) {
    let pressed = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({ "keyCode": "Backspace" })),
    ));
    assert!(
        !pressed.is_error(),
        "`Backspace` 注入必须落到真实窗口: {pressed:?}"
    );
}

/// 语义 ID 当前**在不在**运行时控件树里（`ui/node` 对不存在的 ID 回 `-32006`）。
///
/// 读之前控制面自己会刷新树 ⇒ 这里拿到的是**当前**画面，不是装配期那份缓存。
fn node_exists(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> bool {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    !node.is_error()
}

/// 音符身份全集（用于"音符优先"那一臂）。
fn note_ulids(project: &YebanProjectV1) -> Vec<String> {
    let mut out = Vec::new();
    for entry in project.clip_pool.values() {
        if let Some(map) = entry.content.notes() {
            for id in map.keys() {
                out.push(id.to_canonical_string());
            }
        }
    }
    out
}

/// 把一批音符身份写进选区事实源 `selected-ulids`（与 `tests/roll_edit_tools.rs` 同款）。
fn select_notes(window: &MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

/// ① 夹具前提：`filled_project()` 真的有 ≥2 条摆放，且**全部**出现在绘制清单里。
///
/// 这一臂存在的理由：后面每一条都按"删掉 A、B 必须还在"来断言。若夹具只有一条摆放、
/// 或投影漏了几条，那些断言会**因为前提不成立而空转**（看起来绿，其实什么都没比）。
#[test]
fn the_fixture_has_several_placements_and_the_ui_paints_all_of_them() {
    let (window, port, mut plane, _input) = assemble();
    let project = port.project();
    let placed = placements(&project);
    assert!(
        placed.len() >= 2,
        "夹具前提：`filled_project()` 必须至少两条摆放才能判'只删一条'，实测 {placed:?}"
    );
    assert!(
        !note_ulids(&project).is_empty(),
        "夹具前提：还必须至少有一颗音符，才能判'音符优先'那一臂"
    );

    let painted = clip_ulids(&window);
    assert_eq!(
        painted.len(),
        placed.len(),
        "投影必须把每一条摆放都画出来（`clip-ulids` 与权威摆放数不等 ⇒ 后面的断言会空转）"
    );
    for (ulid, _) in &placed {
        assert!(
            painted.iter().any(|id| id == ulid),
            "摆放 `{ulid}` 必须在 `clip-ulids` 里"
        );
        assert!(
            node_exists(&mut plane, 1, &format!("clip-{ulid}-header")),
            "摆放 `{ulid}` 的包头必须在运行时控件树里（[UI-TEST-001] 语义 ID 寻址）"
        );
    }
    report_line(&format!(
        "[delete-clip] 夹具前提：共 {} 条摆放（{placed:?}），绘制清单 {} 条，音符 {} 颗",
        placed.len(),
        painted.len(),
        note_ulids(&project).len()
    ));
}

/// ② 主判据：真 `WindowEvent` 的 `Backspace` 删掉**选中的那一条**摆放，别的毫发无损。
///
/// 逐项（对象 ＋ 单位）：
/// · 权威摆放**条数** −1，被删的是**点名的那条**身份；另一条 `ClipPlacement` **逐字段相同**；
/// · `clip_pool` 的序列化字节**一位不变**（摆放删除⛔ 不连带取走池条目 —— 那是 `Op::RemoveClip`）；
/// · 提交**计数** +1（一次按键 = 一次原子提交）；
/// · `selected-clip-ulid` 的**字符长度** 从身份变成 0（选中的对象已不存在）；
/// · `clip-{A}-header` 在运行时树里的**存在性** 由真变假，`clip-{B}-header` 仍为真。
#[test]
fn backspace_on_a_selected_clip_placement_removes_exactly_that_placement() {
    let (window, port, mut plane, input) = assemble();
    // ---- 前置条件：否则"没反应"会被误读成接线坏了（与 arrow_nudge_d 同一手法）----
    assert_eq!(
        input.borrow().focus(),
        yeban_app::input::Focus::MainCanvas,
        "注入前焦点必须在主画布（`forward-focus: key-handler` 的默认态）"
    );
    assert!(
        !input.borrow().is_composing(),
        "注入前必须是非合成态，否则 §7.2 会把整个键吞掉"
    );
    assert!(
        window.get_arrangement_view(),
        "夹具前提：装配后默认是编曲视图（本判据的视图门控靠它）"
    );

    let painted = clip_ulids(&window);
    let (a, b) = (painted[0].clone(), painted[1].clone());
    select_clip(&window, &a);

    let project_before = port.project();
    let bytes_before = serde_json::to_vec(&project_before).expect("序列化基线");
    let pool_before = serde_json::to_vec(&project_before.clip_pool).expect("序列化池子");
    let placement_b = placement_by_ulid(&project_before, &b).expect("B 在场");
    let commits_before = port.display().commit_count;

    // ---- 真 `WindowEvent`：Backspace ----
    press_backspace(&mut plane, 10);

    let project_after = port.project();
    assert_eq!(
        placement_by_ulid(&project_after, &a),
        None,
        "`Backspace` 必须把选中的摆放 `{a}` 从轨道上取走"
    );
    assert_eq!(
        placement_by_ulid(&project_after, &b),
        Some(placement_b),
        "另一条摆放 `{b}` 必须**逐字段**不变（`ClipPlacement` 派生 `PartialEq`）"
    );
    assert_eq!(
        placements(&project_after).len(),
        placements(&project_before).len() - 1,
        "只能少那**一条**摆放"
    );
    assert_eq!(
        serde_json::to_vec(&project_after.clip_pool).expect("序列化"),
        pool_before,
        "删摆放⛔ 不得连带删掉片段池条目（内容还在，重新摆回去不必重录；单位：序列化字节）"
    );
    assert_ne!(
        serde_json::to_vec(&project_after).expect("序列化"),
        bytes_before,
        "正对照：工程确实变了（否则上面那些相等只是没动手）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次按键 = **恰好一步**可撤销的编辑"
    );

    // ---- 视图态跟着走：选中态清空、绘制清单里没有那条剪辑 ----
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        "",
        "被删的剪辑不可能还选中（`selected-clip-ulid` 是卷帘作用域的键）"
    );
    let painted_after = clip_ulids(&window);
    assert!(
        !painted_after.iter().any(|id| id == &a),
        "重投影后 `{a}` 不得留在 `clip-ulids` 里"
    );
    assert!(
        painted_after.iter().any(|id| id == &b),
        "`{b}` 必须还在 `clip-ulids` 里"
    );
    assert!(
        !node_exists(&mut plane, 20, &format!("clip-{a}-header")),
        "运行时控件树里不得留下已被删除的包头 `clip-{a}-header`"
    );
    assert!(
        node_exists(&mut plane, 30, &format!("clip-{b}-header")),
        "另一条剪辑的包头 `clip-{b}-header` 必须还在树上"
    );

    report_line(&format!(
        "[delete-clip] `{a}` 经真 `WindowEvent` 的 `Backspace` 被取走；`{b}` 逐字段不变；\
         池子字节不变（{} 字节）；提交 +1；选中态清空；`clip-{a}-header` 离树",
        pool_before.len()
    ));
}

/// ③ **音符优先**：同时选中音符与剪辑时，`Delete` 删的是音符，剪辑一位不动。
///
/// 这条是"反过来会怎样"的防线：卷帘的作用域正是那条选中剪辑，若剪辑优先，用户指着
/// 几颗音符按下一键会带走**整个**剪辑（数据损失，且一步撤销也未必找得回正确的粒度）。
#[test]
fn note_selection_out_rank_the_clip_deletion() {
    let (window, port, _plane, _input) = assemble();
    let project_before = port.project();
    let painted = clip_ulids(&window);
    let clip = painted[0].clone();
    select_clip(&window, &clip);
    let note = note_ulids(&project_before)[0].clone();
    select_notes(&window, &[note.clone()]);

    let commits_before = port.display().commit_count;
    let consumed = window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
    assert!(consumed, "选区里有音符 ⇒ `Delete` 必须被消费");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次按键 = 一次提交"
    );
    let project_after = port.project();
    assert_eq!(
        placement_by_ulid(&project_after, &clip).map(|p| p.id),
        Some(EntityId::from_str(clip.as_str()).expect("剪辑身份是合法 ULID")),
        "⚠ 音符优先：`Delete` ⛔ 不得顺手删掉整条剪辑摆放 `{clip}`"
    );
    assert!(
        !note_ulids(&project_after).iter().any(|id| id == &note),
        "被删的必须是那颗被选中的音符 `{note}`"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        clip,
        "音符那条臂⛔ 不碰剪辑选中态（它只清 `selected-ulids`）"
    );

    report_line(&format!(
        "[delete-clip] 音符 `{note}` 与剪辑 `{clip}` 同时选中 ⇒ `Delete` 删音符、剪辑逐字段不变（提交 1 次）"
    ));
}

/// ④ 视图门控：Session 视图里那一次按键**不消费、不提交**，而**同一份选中态**回到
/// 编曲视图后立刻删得掉 —— 两臂只差 `arrangement-view`，于是差值只能归因于这道门。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_deleted_in_session_view() {
    let (window, port, _plane, _input) = assemble();
    let painted = clip_ulids(&window);
    let clip = painted[0].clone();
    select_clip(&window, &clip);
    let commits_before = port.display().commit_count;
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    // ---- Session 视图（`arrangement-view = false`）：同样的键必须什么都不做 ----
    window.set_arrangement_view(false);
    let consumed_in_session =
        window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
    assert!(
        !consumed_in_session,
        "Session 视图里没有'选中的剪辑'这一格 ⇒ 拿陈旧读数删东西必须是**不消费**"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "Session 视图里那一次按键不得提交任何 op"
    );
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "Session 视图里那一次按键不得改动工程"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        clip,
        "不消费时⛔ 也不得顺手改动选中态（那是另一件事）"
    );

    // ---- 回到编曲视图：**同一份**选中态 ⇒ 同一键真的删掉 ----
    window.set_arrangement_view(true);
    let consumed_in_arrangement =
        window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
    assert!(
        consumed_in_arrangement,
        "正对照：唯一改变的是视图 ⇒ 编曲视图里这一键必须消费并删除"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "编曲视图那一次 = 一次提交"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &clip),
        None,
        "编曲视图那一次必须真的删掉 `{clip}`"
    );

    report_line(&format!(
        "[delete-clip] 同一份选中态 `{clip}`：Session 视图不消费 / 0 提交 / 工程不变；\
         编曲视图消费 / +1 提交 / 摆放取走"
    ));
}

/// ⑤ 解析不到 ⇒ **不消费**（两种形态：合法但不存在的身份、根本不是 ULID 的串）。
///
/// 与 `delete_ops_for` 同一取向："把键吞掉却什么都不做，比不处理更糟" —— 调用方读到
/// `consumed = false` 才知道这条绑定今天没活儿，而 `true` ＋ 零改动是**谎报**。
#[test]
fn a_stale_or_illegal_clip_ulid_is_not_consumed() {
    let (window, port, _plane, _input) = assemble();
    let commits_before = port.display().commit_count;
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    for stale in [stale_clip_ulid(), "not-a-real-clip".to_owned()] {
        window.set_selected_clip_ulid(stale.clone().into());
        let consumed = window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
        assert!(
            !consumed,
            "选中身份 `{stale}` 解析不到 ⇒ 必须是**不消费**（⛔ 不是'吞掉却什么都不做'）"
        );
        assert_eq!(
            port.display().commit_count,
            commits_before,
            "解析不到时不得提交任何 op（`{stale}`）"
        );
        assert_eq!(
            window.get_selected_clip_ulid().as_str(),
            stale.as_str(),
            "解析不到时⛔ 不得顺手清掉那个陈旧读数（那是另一件事，不归这一键）"
        );
    }

    // ---- 没有任何选中（音符与剪辑都空）⇒ 同样不消费 ----
    window.set_selected_clip_ulid("".into());
    let consumed_empty = window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
    assert!(
        !consumed_empty,
        "什么都没选 ⇒ `Delete` 不得被消费（没有可判定的对象，猜一个是数据损失）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "无选区时零提交"
    );
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "上面每一臂都必须逐字节不动工程"
    );

    report_line(&format!(
        "[delete-clip] 陈旧 `{}` / 非法 `not-a-real-clip` / 空选区 三种都**不消费**、0 提交",
        stale_clip_ulid()
    ));
}

/// ⑥ **一步**撤销逐字节放回那条摆放（含 `previous_placement` 载荷），再 **redo** 又取走。
///
/// 这条同时是"撤销不需要回放历史"的证据：模型对 `AddClipPlacement` 的前置条件之一是
/// 片段仍在池子里 —— 判据 ② 证了池子没动，这里才可能放回成功。
#[test]
fn one_undo_puts_the_placement_back_byte_for_byte() {
    let (window, port, _plane, _input) = assemble();
    let painted = clip_ulids(&window);
    let clip = painted[0].clone();
    select_clip(&window, &clip);
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    let consumed = window.invoke_key_action(DELETE_TEXT.into(), false, false, false, false);
    assert!(consumed, "夹具前提：编曲视图里这一键必须消费");
    assert_ne!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "夹具前提：工程确实被改过"
    );

    let undone = port.perform(UiAction::Undo);
    assert!(undone.changed(), "撤销必须真的改了工程");
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "**一步**撤销必须将工程逐字节放回删除前（载荷自带整份摆放）"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &clip).map(|p| p.id.to_canonical_string()),
        Some(clip.clone()),
        "放回后 `{clip}` 必须在原轨上"
    );
    assert!(
        clip_ulids(&window).iter().any(|id| id == &clip),
        "重投影后 `{clip}` 必须回到绘制清单（画面跟得上权威）"
    );

    let redone = port.perform(UiAction::Redo);
    assert!(redone.changed(), "redo 必须真的改了工程");
    assert_eq!(
        placement_by_ulid(&port.project(), &clip),
        None,
        "redo 后 `{clip}` 再次被取走"
    );

    report_line(&format!(
        "[delete-clip] `{clip}`：删除 → 一步撤销逐字节放回（{} 字节工程相同）→ redo 再取走",
        bytes_before.len()
    ));
}

/// ⑦ 纯函数臂 ＋ **源码纪律**（`clip_delete_ops_for` 是全 app 唯一的 `RemoveClipPlacement` 构造点）。
///
/// 纯函数部分不建窗口（投影层判据同款）：喂真实工程与三种非法输入，逐条对账 op 内容。
/// 源码纪律部分钉的是"写入面只有一处"：日后若长出第二个构造点，两个地方就可能对
/// "删的是哪条摆放"给出不同答案 —— 那正是本文件要防的分叉形状。
#[test]
fn clip_delete_ops_for_is_the_only_remove_clip_placement_construction_site() {
    let project = yeban_model::samples::filled_project();
    let placed = placements(&project);
    let (a_ulid, a_placement) = &placed[0];
    let track_of_a = project
        .tracks
        .iter()
        .find(|(_, track)| track.clips.contains_key(&a_placement.id))
        .map(|(track_id, _)| *track_id)
        .expect("摆放必然住在某条轨上");

    let ops = clip_delete_ops_for(&project, a_ulid);
    assert_eq!(ops.len(), 1, "一条选中剪辑 ⇒ **恰好一条** op");
    match &ops[0] {
        yeban_model::ops::Op::RemoveClipPlacement {
            track_id,
            placement_id,
            previous_placement,
        } => {
            assert_eq!(*track_id, track_of_a, "op 必须指向摆放**所在**的那条轨");
            assert_eq!(
                *placement_id, a_placement.id,
                "op 的目标身份必须是被选中的那条"
            );
            assert_eq!(
                *previous_placement, *a_placement,
                "撤销载荷必须是当前摆放的**整份**副本（一步撤销不必回放历史）"
            );
        }
        other => panic!("只允许 `RemoveClipPlacement`，实际 {other:?}"),
    }

    // 三种"解析不到"⇒ 空批（⛔ 不删一个猜的对象）。
    let stale = stale_clip_ulid();
    for bogus in ["", "not-a-real-clip", stale.as_str()] {
        assert!(
            clip_delete_ops_for(&project, bogus).is_empty(),
            "非法 / 陈旧身份 `{bogus}` 必须解析成空批"
        );
    }

    // ---- 源码纪律：从 `src/host.rs` 现读，⛔ 不靠注释里的数字 ----
    let host_source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/host.rs"))
        .expect("读本 crate 的 host.rs");
    let construction_sites = host_source
        .match_indices("Op::RemoveClipPlacement {")
        .count();
    assert_eq!(
        construction_sites, 1,
        "`host.rs` 里 `Op::RemoveClipPlacement` 的构造点必须**恰好一处**（实测 {construction_sites}）"
    );
    let placement_fn_sites = host_source
        .match_indices("delete_selected_clip_placement(")
        .count();
    assert_eq!(
        placement_fn_sites, 2,
        "`delete_selected_clip_placement(` 必须只有**定义 ＋ 唯一调用点**两处（实测 {placement_fn_sites}）"
    );

    report_line(&format!(
        "[delete-clip] `clip_delete_ops_for` 对 `{a_ulid}` 给出 1 条 op（载荷逐字段相同）；\
         3 种非法输入为空批；`host.rs` 里构造点 1 处、动作函数站点 2 处"
    ));
}

/// ⑧ 投影层前提复核：`ViewState` 的池子过滤口径与本判据无关，但**视图构造**必须能投影
/// 删除后的工程（否则宿主提交后重投影会 panic —— 那是"模型删了、画面崩了"的另一种坏）。
#[test]
fn the_view_state_projects_a_project_with_the_placement_removed() {
    let mut project = yeban_model::samples::filled_project();
    let placed = placements(&project);
    let (a_ulid, _) = placed[0].clone();
    let a_id = EntityId::from_str(a_ulid.as_str()).expect("合法 ULID");
    for track in project.tracks.values_mut() {
        track.clips.remove(&a_id);
    }
    assert!(
        placements(&project).len() < placed.len(),
        "夹具前提：这里确实取走了一条摆放"
    );
    let view = ViewState::from_project(&project).expect("删除后的工程必须仍能投影");
    assert!(
        !view.clip_ulids().iter().any(|id| id.as_str() == a_ulid),
        "投影出的绘制清单里不得留下被删的 `{a_ulid}`"
    );
    report_line(&format!(
        "[delete-clip] 取走 `{a_ulid}` 后 `ViewState::from_project` 正常，清单 {} 条",
        view.clip_ulids().len()
    ));
}
