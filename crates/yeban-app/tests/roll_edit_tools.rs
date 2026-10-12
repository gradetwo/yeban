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
//! | `clicking_a_note_with_the_select_tool_selects_it_in_production_order` | **本票修的缺陷的判据**：按**生产顺序**（`build_live_ui` ＋ `wire_roll_edit`）装配后，用**默认选择工具**点一个现成音符 ⇒ `selected-note-count == 1` 且该音符 ULID 进 `selected-ulids`（覆盖缺陷下这里恒为 0 —— 见文件末的"怎么变红"） |
//! | `clicking_a_note_without_wire_roll_edit_still_selects_it` | **控制臂**：同样是 `build_live_ui` 但**不调** `wire_roll_edit`（= 覆盖发生前的形态）的同一个点击也选中它 ⇒ 把缺陷隔离到"后装 handler 覆盖"，并证明坐标 / 夹具本身是对的 |
//! | `the_eraser_click_deletes_the_note_under_the_cursor` | 橡皮擦点击接上后：点光标下的音符 ⇒ 权威工程音符 **−1**，删掉的正是点中的那个身份，且选区被清空（与键盘 `Delete` 同一取向） |
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
//!
//! ## 怎么变红（新增两条判据）
//!
//! - 让 `wire_roll_edit` 重新注册一个**只认铅笔**的 handler（即覆盖派发器里的选区那一段）
//!   ⇒ `clicking_a_note_with_the_select_tool_selects_it_in_production_order` 的
//!   `selected-note-count` 停在 0（实测断言：左 0、右 1）。控制臂**仍绿** ——
//!   这正是"缺陷在覆盖、不在坐标"的证据。
//! - 让橡皮擦分支重新什么都不做 ⇒ `the_eraser_click_deletes_the_note_under_the_cursor` 红。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `piano_roll_drag_scroll.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use yeban_app::host::{PENCIL_TOOL_DIGIT, ROLL_SNAP_GRID_TICKS, pencil_op_for, wire_roll_edit};
use yeban_app::input::Tool;
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

/// **控制臂**的装配：同一个 `build_live_ui`，但**不调** `wire_roll_edit`
/// （= 覆盖发生**之前**的形态：只有 `build_main_window_with_console_tab` 装的选区派发器）。
fn assemble_without_roll_edit() -> (yeban_app::ui::MainWindow, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, plane)
}

/// 音符矩形的高度（与 `host::NOTE_HEIGHT_PX` / `piano_roll.slint` 的 `12px` 同一读数）。
const NOTE_HEIGHT: f32 = 12.0;

/// **宿主视口坐标**下"点在音符中心"：`(x, y)`。
///
/// ⚠ 这里是 `hit_test_visible` / `pencil_op_for` 用的那一套（tick 0 = x 0，泳道顶 = y 0），
/// **不是** `.slint` 里把音符层放在琴键列（56px）＋ 内缩（`Tokens.space-5` = 12px）＋
/// 表头（28px）之后的绘制坐标。判据必须与宿主命中测试同一口径，否则命中测试自己都不认；
/// 控制臂用**同一坐标**选中音符，就是这条口径正确的机械见证。
fn note_center_in_host_view(
    view: &yeban_app::bridge::ViewState,
    index: usize,
    scroll: f32,
) -> (f32, f32) {
    let note = &view.notes[index];
    (
        note.x - scroll + note.width / 2.0,
        note.y + NOTE_HEIGHT / 2.0,
    )
}

/// 找一个**宿主命中测试真的能命中**的音符（返回它在 `view.notes` 里的下标）。
///
/// 用 `find` 而不是取第 0 个：夹具里未必每个音符都落在这一帧的可见窗口内。
fn a_hittable_note(view: &yeban_app::bridge::ViewState, viewport_width: f32, scroll: f32) -> usize {
    (0..view.notes.len())
        .find(|&index| {
            let (x, y) = note_center_in_host_view(view, index, scroll);
            view.hit_test_visible(scroll, viewport_width, x, y, NOTE_HEIGHT) == Some(index)
        })
        .expect("夹具里必须有一个宿主命中测试能命中的音符")
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
    click_in_roll_by_id_at(plane, seq, hit_x, HIT_Y_IN_ROLL);
}

/// 按**语义 ID** 在卷帘内 `(hit_x, hit_y)`（宿主视口坐标）处注入一次真实点击。
///
/// `xOffset` / `yOffset` 是**相对 `piano-roll-scroll-area` 元素**的偏移；测试端口把它换成
/// 绝对窗口坐标（`elementId` 的包围盒原点 ＋ 偏移）再交给 Slint，因此这里的 `(hit_x, hit_y)`
/// 与宿主回调拿到的 `(x, y)` 逐值相同 —— 这就是"注入的是宿主那套命中测试坐标"的机械依据。
fn click_in_roll_by_id_at(plane: &mut LiveControlPlane, seq: i64, hit_x: f32, hit_y: f32) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let origin_x = bounds["x"].as_f64().expect("x") as f32;
    let origin_y = bounds["y"].as_f64().expect("y") as f32;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": f64::from(hit_x),
            "yOffset": f64::from(hit_y),
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    let moved = plane.plane().try_line(&request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({
            "x": f64::from(origin_x + hit_x),
            "y": f64::from(origin_y + hit_y)
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

/// 判据 ②（**本票修的缺陷**）：生产顺序装配后，用默认选择工具点一个现成音符 ⇒ 选中它。
///
/// ## 缺陷（本判据必须在修复前的代码上变红）
///
/// Slint 一个 `callback` 只留一个 handler ⇒ 旧实现里 `wire_roll_edit` 那个**只认铅笔**的
/// handler 把 `build_main_window_with_console_tab` 装的选区 handler **整个换掉**，
/// 于是本判据的 `selected-note-count` 恒为 0（实测红：`assert_eq!(count, 1)` 左 0、右 1）。
/// 控制臂（下一条判据）在同样代码上**仍然绿**，把缺陷隔离到"后装 handler 覆盖"这一点。
#[test]
fn clicking_a_note_with_the_select_tool_selects_it_in_production_order() {
    let (window, port, mut plane) = assemble();
    use slint::Model as _;
    // 默认工具就是选择箭头（`app.slint` 的 `active-tool: 1`）—— 本判据不切工具。
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：窗口的默认工具必须是选择箭头"
    );
    let view = yeban_app::bridge::ViewState::from_project(&port.project()).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let index = a_hittable_note(&view, viewport_width, 0.0);
    let expected_ulid = view.notes[index].id.clone();
    let (x, y) = note_center_in_host_view(&view, index, 0.0);
    // 夹具自证：宿主命中测试命中的**就是**这一个音符。
    assert_eq!(
        view.hit_test_visible(0.0, viewport_width, x, y, NOTE_HEIGHT),
        Some(index),
        "夹具前提：注入点必须被宿主命中测试解析到这一个音符"
    );
    assert_eq!(window.get_selected_note_count(), 0, "起点不该已有选区");

    click_in_roll_by_id_at(&mut plane, 10, x, y);

    let count = window.get_selected_note_count();
    let selected: Vec<String> = window
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    report_line(&format!(
        "[roll-select] 生产顺序 + 选择工具点击 `{expected_ulid}` @({x},{y}) ⇒ \
         selected-note-count={count} selected-ulids={selected:?}"
    ));
    assert_eq!(
        count, 1,
        "选择工具点中一个音符必须恰好选中 1 个（覆盖缺陷下这里恒为 0）"
    );
    assert!(
        selected.iter().any(|id| id == &expected_ulid),
        "选中的必须是点中的那个音符身份 `{expected_ulid}`"
    );
}

/// 判据 ③（**控制臂**）：同样 `build_live_ui` 但**不调** `wire_roll_edit` 时，
/// 同一个点击也选中它 —— 覆盖发生前的形态。
///
/// 它隔离出缺陷的位置：不是坐标、不是命中测试、也不是"选区逻辑不存在"，而是
/// `wire_roll_edit` 后装的 handler 把选区 handler 覆盖了。本判据在**修复前后都绿**。
#[test]
fn clicking_a_note_without_wire_roll_edit_still_selects_it() {
    let (window, mut plane) = assemble_without_roll_edit();
    use slint::Model as _;
    let project = yeban_model::samples::filled_project();
    let view = yeban_app::bridge::ViewState::from_project(&project).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let index = a_hittable_note(&view, viewport_width, 0.0);
    let expected_ulid = view.notes[index].id.clone();
    let (x, y) = note_center_in_host_view(&view, index, 0.0);
    assert_eq!(
        view.hit_test_visible(0.0, viewport_width, x, y, NOTE_HEIGHT),
        Some(index),
        "夹具前提：注入点必须被宿主命中测试解析到这一个音符"
    );
    assert_eq!(window.get_selected_note_count(), 0, "起点不该已有选区");

    click_in_roll_by_id_at(&mut plane, 10, x, y);

    let count = window.get_selected_note_count();
    let selected: Vec<String> = window
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    report_line(&format!(
        "[roll-select] 控制臂（不调 wire_roll_edit）点击 `{expected_ulid}` @({x},{y}) ⇒ \
         selected-note-count={count} selected-ulids={selected:?}"
    ));
    assert_eq!(count, 1, "控制臂：覆盖发生前，选区 handler 必须选中一个");
    assert!(
        selected.iter().any(|id| id == &expected_ulid),
        "控制臂选中的必须是点中的那个音符身份 `{expected_ulid}`"
    );
}

/// 判据 ④（**橡皮擦的点击路径已接通**）：点光标下的音符 ⇒ 权威工程音符 **−1**，
/// 删掉的正是点中的那个身份，且选区被清空（与键盘 `Delete` 同一取向）。
///
/// 本判据是旧**缺口登记**（`the_eraser_click_path_is_registered_as_not_yet_wired`）按它
/// 自己写明的方式**更新**而来：唯一派发器把橡皮擦接到了 [`yeban_app::host::delete_ops_for`]
/// （与键盘 `Delete` 同一个"选区身份 → 待提交批"解析点），所以"点击不删"的登记到此为止。
#[test]
fn the_eraser_click_deletes_the_note_under_the_cursor() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let before = note_count(&project_before);
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

    // 靶子：先用铅笔加一个音符，再切到橡皮擦点它。
    window.set_active_tool(i32::from(Tool::Pencil.digit()));
    click_in_roll_by_id(&mut plane, 30, hit_x);
    assert_eq!(
        note_count(&port.project()),
        before + 1,
        "前置：铅笔必须先真的加出一个音符"
    );
    let new_id = notes(&port.project())
        .into_iter()
        .map(|(id, _, _)| id)
        .find(|id| !notes(&project_before).iter().any(|(old, _, _)| old == id))
        .expect("必须能读出新增音符的身份");

    // 靶子的几何从**权威工程**读回（⛔ 不猜坐标），再切橡皮擦点它。
    let view = yeban_app::bridge::ViewState::from_project(&port.project()).expect("投影");
    let index = view
        .notes
        .iter()
        .position(|note| note.id == new_id)
        .expect("新音符必须在投影里");
    let (x, y) = note_center_in_host_view(&view, index, 0.0);
    let bounds = element_bounds(&mut plane, 45, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    assert_eq!(
        view.hit_test_visible(0.0, viewport_width, x, y, NOTE_HEIGHT),
        Some(index),
        "夹具前提：橡皮擦注入点必须被宿主命中测试解析到靶子"
    );

    window.set_active_tool(i32::from(Tool::Eraser.digit()));
    click_in_roll_by_id_at(&mut plane, 50, x, y);

    let after = note_count(&port.project());
    report_line(&format!(
        "[roll-edit] 橡皮擦点击 `{new_id}` @({x},{y}) ⇒ 音符数 {before}＋1 ⇒ {after}；\
         selected-note-count={}",
        window.get_selected_note_count()
    ));
    assert_eq!(after, before, "橡皮擦点音符必须恰好删掉那一个");
    assert!(
        !notes(&port.project())
            .iter()
            .any(|(id, _, _)| id == &new_id),
        "删掉的必须正是点中的身份 `{new_id}`"
    );
    assert_eq!(
        window.get_selected_note_count(),
        0,
        "被删身份不得留在选区里（与键盘 `Delete` 同一取向）"
    );
}
