//! `ROAD-M3-001` 的**编辑语义**（工具状态机 → 真实模型编辑）＋ `AGENTS.md` §3.6 双重验证。
//!
//! ## 侦察结论（本判据就是它的机械见证）
//!
//! **铅笔这条链已经端到端接好**，本判据**不重造**，只按 §3.6 补"注入指针 ⇒ 断言模型"这一路：
//!
//! | 环 | 位置 |
//! | :--- | :--- |
//! | 装配 | `src/main.rs` 的 `host::wire_roll_edit(&ui, &undo_port)` |
//! | 点击源 | `ui/console/piano_roll.slint` 的拖动 `TouchArea`：`up` ⇒ `root.clicked(self.mouse-x, self.mouse-y)` |
//! | 工具门 | `host::pencil_op_for(project, scroll, x, y, grid, active_tool)` —— `active_tool != 铅笔 => None` |
//! | 提交 | `port.commit_ops(now_ms, "pencil: add note", vec![op])` ⇒ **既有的 `Op` 通道**（⛔ 无旁路） |
//! | 反馈 | 提交后**重新投影**：新音符以 `note-{ulid}-rect` 出现在运行时控件树里（**可视**反馈） |
//!
//! ## 本判据断言什么
//!
//! | 判据 | 证明 |
//! | :--- | :--- |
//! | `a_pencil_click_by_semantic_id_adds_one_note_at_the_snapped_tick_and_pitch` | §3.6：**按语义 ID**（`piano-roll-scroll-area`）注入 `down → up` ⇒ 权威工程音符 **+1**，且新音符的 `start_tick` / `pitch` 与**坐标换算**（`snapped_tick_at` / `lane_at_y` ＋ `pitch_lane`）逐值一致；并给出**即时可视反馈**读数（新元素进运行时树） |
//! | `the_eraser_click_path_is_registered_as_not_yet_wired` | 如实登记**缺口**：`wire_roll_edit` 的门只认铅笔 ⇒ **橡皮擦点击不删音符**。本判据把"今天的事实"钉住（⛔ 不假装）；它变红之日就是橡皮擦接上之时，届时**更新本判据**（不是删掉它） |
//!
//! ## 未接的部分（如实登记，⛔ 不假装）
//!
//! **听觉**即时试听（`Action::AuditionMain` / `AuditionProposal`）**仍未接**：`host::action_has_implementation`
//! 对它们返回 `false`（`src/host.rs`）。引擎侧已经并入 main 的"等功率淡化 ＋ `crossfade_len_frames`"
//! **不等于** `ARCH-RT-005` 已实现 —— 机械读数：`grep -c 'ARCH-RT-005' crates/yeban-engine/src/lib.rs` = **0**
//! ⇒ 预登记（`tests/cli_contract.rs` 的那条 `assert!`）**未触发**，本轮按预登记要求**保持拒绝**（⛔ 未改 `yeban-engine`）。
//! 本判据记录的是**可视**反馈（音符进树），⛔ 不把它说成"发声"。
//!
//! ## 本机可编译（`AGENTS.md` §5 的边界）
//!
//! 依赖已缓存 ⇒ 本判据在本机跑过 `cargo check` / `cargo clippy -D warnings` / `cargo test`（读数见交付报告）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `piano_roll_drag_scroll.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use yeban_app::host::{PENCIL_TOOL_DIGIT, ROLL_SNAP_GRID_TICKS, pencil_op_for, wire_roll_edit};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 拖动面（点击源）的**稳定语义 ID**（`[UI-TEST-001]` §12.2）。
const SCROLL_AREA: &str = "piano-roll-scroll-area";

/// 卷帘内的点击高度（窗口 y = 包围盒 y ＋ 这个值）。取 100px：落在音高泳道带内。
const HIT_Y_IN_ROLL: f32 = 100.0;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 装配：真窗口 ＋ 端口 ＋ **与产品同一条** `wire_roll_edit`。
fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<roll-edit>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_roll_edit(ui.ui(), &port);
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 工程里的音符数（与 `live_ui_mcp.rs` 的 `model_note_count` 同一口径：`clip_pool` 的各段落音符之和）。
fn note_count(project: &yeban_model::YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(|notes| notes.len())
        .sum()
}

/// 工程里所有音符：`(ulid 文本, start_tick, pitch)`。
fn notes(project: &yeban_model::YebanProjectV1) -> Vec<(String, u64, u8)> {
    let mut out = Vec::new();
    for entry in project.clip_pool.values() {
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for (id, note) in map {
            out.push((id.to_canonical_string(), note.start_tick, note.pitch));
        }
    }
    out
}

/// 语义 ID 的包围盒。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> serde_json::Value {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时树里: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    bounds
}

/// 按**语义 ID** 在卷帘内 `(hit_x, HIT_Y_IN_ROLL)` 处注入一次真实点击（`down → move → up`）。
fn click_in_roll_by_id(plane: &mut LiveControlPlane, seq: i64, hit_x: f32) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let origin_x = bounds["x"].as_f64().expect("x") as f32;
    let origin_y = bounds["y"].as_f64().expect("y") as f32;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": f64::from(hit_x),
            "yOffset": f64::from(HIT_Y_IN_ROLL),
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    let moved = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({
            "x": f64::from(origin_x + hit_x),
            "y": f64::from(origin_y + HIT_Y_IN_ROLL)
        })),
    ));
    assert!(!moved.is_error(), "移动必须落到真实窗口: {moved:?}");
    let up = plane.plane().try_line(&request_line(
        seq + 3,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

/// 判据 ①（§3.6）：按语义 ID 注入一次铅笔点击 ⇒ **恰好一个**新音符，且 tick / pitch 与坐标换算一致。
#[test]
fn a_pencil_click_by_semantic_id_adds_one_note_at_the_snapped_tick_and_pitch() {
    let (_window, port, mut plane) = assemble();
    let project_before = port.project();

    // ---- 夹具自证：必须存在一个**真的落在片段内**的点击 x（否则本判据没有对照）----
    let scroll = 0.0_f32;
    let hit_x = (0..240)
        .map(|step| 10.0 * f32::from(u8::try_from(step).unwrap_or(0)))
        .find(|x| {
            pencil_op_for(
                &project_before,
                scroll,
                *x,
                HIT_Y_IN_ROLL,
                ROLL_SNAP_GRID_TICKS,
                i32::from(PENCIL_TOOL_DIGIT),
            )
            .is_some()
        })
        .expect("夹具里必须存在落在片段内的点击位置");
    // **独立的期望值**（⛔ 不用 `pencil_op_for` 的返回值当期望）：
    // ① tick 由 `ViewState::snapped_tick_at` 算；② 音高由 `lane_at_y` ＋ `pitch_lane` 反查。
    let view = yeban_app::bridge::ViewState::from_project(&project_before).expect("投影");
    let expected_tick = view.snapped_tick_at(scroll, hit_x, ROLL_SNAP_GRID_TICKS);
    let lane = yeban_app::bridge::lane_at_y(HIT_Y_IN_ROLL).expect("该高度必须有音高泳道");
    let expected_pitch = (0u8..=127)
        .find(|pitch| yeban_app::bridge::pitch_lane(*pitch) == lane)
        .expect("泳道必须对应一个音高");
    report_line(&format!(
        "[roll-edit] 注入点 x={hit_x} y={HIT_Y_IN_ROLL} ⇒ 期望 tick={expected_tick} pitch={expected_pitch} lane={lane}"
    ));

    // ---- 选铅笔（**工具状态机**）：走界面上的工具按钮语义 ID ----
    _window.set_active_tool(i32::from(PENCIL_TOOL_DIGIT));
    assert_eq!(
        _window.get_active_tool(),
        i32::from(PENCIL_TOOL_DIGIT),
        "工具状态机：`active-tool` 必须真的切到铅笔"
    );

    // ---- 注入真实点击（按语义 ID）----
    let before = note_count(&project_before);
    click_in_roll_by_id(&mut plane, 10, hit_x);

    let project_after = port.project();
    let after = note_count(&project_after);
    let new_notes: Vec<(String, u64, u8)> = notes(&project_after)
        .into_iter()
        .filter(|(id, _, _)| !notes(&project_before).iter().any(|(old, _, _)| old == id))
        .collect();
    report_line(&format!(
        "[roll-edit] 注入前后音符数 {before} ⇒ {after}；新增 {} 个: {:?}",
        new_notes.len(),
        new_notes
    ));
    assert_eq!(
        after,
        before + 1,
        "铅笔点击必须恰好在**权威工程**里加一个音符"
    );
    assert_eq!(new_notes.len(), 1, "新增音符必须恰为一个");
    let (new_id, new_tick, new_pitch) = new_notes[0].clone();
    assert_eq!(
        new_tick, expected_tick,
        "新音符的 `start_tick` 必须等于 `snapped_tick_at(scroll, x, grid)` 的结果"
    );
    assert_eq!(
        new_pitch, expected_pitch,
        "新音符的 `pitch` 必须等于 `lane_at_y(y)` 经 `pitch_lane` 反查出的音高"
    );

    // ---- **可视**即时反馈：新音符以 `note-{ulid}-rect` 进入运行时控件树 ----
    let node = plane.plane().try_line(&request_line(
        20,
        METHOD_NODE,
        Some(json!({ "elementId": format!("note-{new_id}-rect") })),
    ));
    report_line(&format!(
        "[roll-edit] 可视反馈：`note-{new_id}-rect` 在运行时树里 = {}（⛔ 这不是发声）",
        !node.is_error()
    ));
    assert!(
        !node.is_error(),
        "新音符必须**立刻**出现在运行时控件树里（`note-{{ulid}}-rect`）"
    );
}

/// 判据 ②（**如实登记缺口**）：橡皮擦的**点击**路径尚未接通。
///
/// ⛔ 本判据断言的是"**今天的事实**"：`wire_roll_edit` 的门只认铅笔（`pencil_op_for` 的第一行），
/// 因此把 `active-tool` 切到橡皮擦再点同一个点，**音符数不变**。
/// 它**变红之日**就是橡皮擦接上之时 ⇒ 届时**更新本判据**（⛔ 不是删掉它）。
#[test]
fn the_eraser_click_path_is_registered_as_not_yet_wired() {
    let (_window, port, mut plane) = assemble();
    let project_before = port.project();
    let hit_x = (0..240)
        .map(|step| 10.0 * f32::from(u8::try_from(step).unwrap_or(0)))
        .find(|x| {
            pencil_op_for(
                &project_before,
                0.0,
                *x,
                HIT_Y_IN_ROLL,
                ROLL_SNAP_GRID_TICKS,
                i32::from(PENCIL_TOOL_DIGIT),
            )
            .is_some()
        })
        .expect("夹具里必须存在落在片段内的点击位置");

    // 先用铅笔加一个音符（作为橡皮擦的靶子），再切到橡皮擦点它。
    _window.set_active_tool(i32::from(PENCIL_TOOL_DIGIT));
    click_in_roll_by_id(&mut plane, 30, hit_x);
    let with_note = note_count(&port.project());
    assert_eq!(
        with_note,
        note_count(&project_before) + 1,
        "前置：铅笔必须先真的加出一个音符"
    );

    const ERASER_TOOL_DIGIT: i32 = 5; // `Tool::from_digit(5)` = 橡皮擦（与五工具矩阵同源）
    _window.set_active_tool(ERASER_TOOL_DIGIT);
    click_in_roll_by_id(&mut plane, 40, hit_x);
    let after_eraser = note_count(&port.project());
    report_line(&format!(
        "[roll-edit] 缺口登记：橡皮擦点击之后音符数 {with_note} ⇒ {after_eraser}\
（相等即「点击路径未接」；本判据变红 = 该路径已接上, 请更新本判据）"
    ));
    assert_eq!(
        after_eraser, with_note,
        "如实登记：橡皮擦的**点击**路径今天没有接通（`wire_roll_edit` 的门只认铅笔）；\
         若本断言失败, 说明该路径已接上, 请把本判据更新为「点击 ⇒ 音符数 −1」"
    );
}
