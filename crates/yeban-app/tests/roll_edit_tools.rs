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
//! | `clicking_the_painted_note_rect_with_the_select_tool_selects_it_in_production_order` | **本票修的缺陷的判据**：按**生产顺序**（`build_live_ui` ＋ `wire_roll_edit`）装配后，用**默认选择工具**点一个现成音符的**绘制坐标**（= 人手真正点得到的矩形）⇒ `selected-note-count == 1` 且该音符 ULID 进 `selected-ulids`（宿主从不减 `(68, 28)` 帧原点时这里恒为 0 —— 见文件末的"怎么变红"） |
//! | `a_click_at_the_host_frame_coordinates_does_not_select_the_painted_note` | **帧控制臂**：把**宿主坐标数**原样当绘制坐标注入（= 旧判据的做法）⇒ 点不到靶子（预期 ULID 不得进选区）。它与上一条一起把两件事钉住：`hit_test_visible` 说**宿主帧**，界面回调报**绘制帧**，两者差 `bridge::painted_to_host` |
//! | `clicking_a_note_without_wire_roll_edit_still_selects_it` | **覆盖控制臂**：同样是 `build_live_ui` 但**不调** `wire_roll_edit`（= 覆盖发生前的形态）的同一个**绘制坐标**点击也选中它 ⇒ 把上一轮的缺陷隔离到"后装 handler 覆盖"，并证明坐标 / 夹具本身是对的 |
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
//! ## 怎么变红（帧缺陷 + 两条旧缺陷各一条）
//!
//! - **本票（帧原点）**：让 `install_roll_click` 把界面报的 `(x, y)` 当宿主坐标用
//!   （= 不在派发器里做 `bridge::painted_to_host`）⇒
//!   `clicking_the_painted_note_rect_with_the_select_tool_selects_it_in_production_order`
//!   的 `selected-note-count` 停在 0（实测断言：左 0、右 1），因为绘制坐标的 y 比音符顶沿
//!   低 28px、超过音符高 12px。帧控制臂在**同一份代码**上**也红**（它断言宿主坐标数字点不到），
//!   两条一起才说明"少了一次换算"而不是"坐标 / 夹具坏了"。
//! - 让 `wire_roll_edit` 重新注册一个**只认铅笔**的 handler（即覆盖派发器里的选区那一段）
//!   ⇒ 生产顺序那条判据的 `selected-note-count` 停在 0；**覆盖控制臂仍绿** ——
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
use yeban_app::bridge::{host_to_painted, painted_to_host};
use yeban_app::host::{PENCIL_TOOL_DIGIT, ROLL_SNAP_GRID_TICKS, pencil_op_for, wire_roll_edit};
use yeban_app::input::Tool;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_CANCEL, METHOD_DISPATCH_POINTER_DOWN,
    METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP, METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 拖动面（点击源）的**稳定语义 ID**（`[UI-TEST-001]` §12.2）。
const SCROLL_AREA: &str = "piano-roll-scroll-area";

/// 卷帘内的点击高度，**宿主帧**（`hit_test_visible` / `pencil_op_for` / `lane_at_y` 那一套，
/// 泳道顶 = y 0）。取 100px：落在音高泳道带内。注入指针时经 `host_to_painted` 搬成绘制坐标。
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

/// **宿主帧**下"点在音符中心"：`(x, y)`。
///
/// 这是 `hit_test_visible` / `pencil_op_for` 用的那一套（tick 0 = x 0，泳道顶 = y 0），
/// **不是** `.slint` 里把音符层放在琴键列（56px）＋ 内缩（`Tokens.space-5` = 12px）＋
/// 表头（28px）之后的**绘制坐标**。两帧的关系只有 `bridge::painted_to_host` 一处
/// （它的反向是 `host_to_painted`）；本文件的 `a_painted_note` 用宿主帧做几何自证，
/// 而真正注入的指针一律走下面 [`note_center_in_painted_view`]。
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

/// **绘制帧**（人看得见的矩形）下"点在音符中心"：宿主中心经 [`host_to_painted`] 搬到绘制坐标。
///
/// 界面回调报的就是这一帧 ⇒ 注入指针必须用它，否则点的是"宿主以为的位置"而不是"人看到的位置"。
fn note_center_in_painted_view(
    view: &yeban_app::bridge::ViewState,
    index: usize,
    scroll: f32,
) -> (f32, f32) {
    let (x, y) = note_center_in_host_view(view, index, scroll);
    host_to_painted(x, y)
}

/// 找一个**人真的点得到**的音符（返回它在 `view.notes` 里的下标）：两个条件都满足 ——
///
/// 1. **宿主帧**：它的宿主帧中心被 `hit_test_visible` 解析到它自己（帧的几何一侧正确）；
/// 2. **绘制帧**：它的绘制帧中心还落在卷帘元素矩形内（否则指针根本到不了这个元素，
///    判据会因"事件没送到"而不是"帧算错"变红 —— 那是一条假对照）。
///
/// 用 `find` 而不是取第 0 个：夹具里未必每个音符都落在这一帧的可见窗口内。
fn a_painted_note(
    view: &yeban_app::bridge::ViewState,
    viewport_width: f32,
    viewport_height: f32,
    scroll: f32,
) -> usize {
    (0..view.notes.len())
        .find(|&index| {
            let (hx, hy) = note_center_in_host_view(view, index, scroll);
            if view.hit_test_visible(scroll, viewport_width, hx, hy, NOTE_HEIGHT) != Some(index) {
                return false;
            }
            let (px, py) = note_center_in_painted_view(view, index, scroll);
            (0.0..=viewport_width).contains(&px) && (0.0..=viewport_height).contains(&py)
        })
        .expect("夹具里必须有一个宿主命中的音符，其绘制帧中心也落在卷帘元素矩形内")
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
///
/// 两个入参都是**宿主帧**；本函数用唯一的 [`host_to_painted`] 把它们搬到界面实际报上来的
/// 绘制帧再注入（⛔ 不在这里另写 `+68` / `+28`）。
fn click_in_roll_by_id(plane: &mut LiveControlPlane, seq: i64, hit_x: f32) {
    let (painted_x, painted_y) = host_to_painted(hit_x, HIT_Y_IN_ROLL);
    click_in_roll_by_id_at(plane, seq, painted_x, painted_y);
}

/// 按**语义 ID** 在卷帘内 `(hit_x, hit_y)`（**绘制帧**，即界面回调 `on_clicked` 收到的值）处
/// 注入一次真实点击。
///
/// `xOffset` / `yOffset` 是**相对 `piano-roll-scroll-area` 元素**的偏移；测试端口把它换成
/// 绝对窗口坐标（`elementId` 的包围盒原点 ＋ 偏移）再交给 Slint，因此这里的 `(hit_x, hit_y)`
/// 与宿主回调拿到的 `(x, y)` 逐值相同 —— 这就是"注入的是绘制帧坐标"的机械依据。
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
    // 注入的是**绘制帧**：用唯一换算把宿主点搬过去；宿主收到后再由同一个 `painted_to_host`
    // 搬回来 ⇒ 这里先证明往返回到原点（否则"注入的"与"期望的"不是同一个点）。
    let (painted_x, painted_y) = host_to_painted(hit_x, HIT_Y_IN_ROLL);
    assert_eq!(
        painted_to_host(painted_x, painted_y),
        (hit_x, HIT_Y_IN_ROLL),
        "帧换算必须往返回到同一个宿主点"
    );
    report_line(&format!(
        "[roll-edit] 宿主点 x={hit_x} y={HIT_Y_IN_ROLL} ⇒ 绘制帧注入点 x={painted_x} y={painted_y}；\
         期望 tick={expected_tick} pitch={expected_pitch} lane={lane}"
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

/// 判据 ②（**本票修的帧缺陷**）：生产顺序装配后，用默认选择工具点一个现成音符
/// **人看得见的矩形**（绘制帧中心）⇒ 选中它。
///
/// ## 缺陷（本判据必须在修复前的代码上变红）
///
/// `.slint` 把音符层画在琴键列（56px）＋ `Tokens.space-5`（12px）之后，并且在 28px 表头之下；
/// 界面回调报的是这一套**绘制坐标**。旧宿主把它当**网格局部**坐标直接喂给 `hit_test_visible`
/// ⇒ 命中点比人点的位置高 28px、左 68px，而音符框只有 12px 高 ⇒ 纵向必然落空。
/// 实测红：本判据的 `selected-note-count` 停在 0（`assert_eq!(count, 1)` 左 0、右 1）。
/// 帧控制臂（下一条判据）在**同一份旧代码**上**也红**（它断言宿主坐标数字点不到），
/// 两条一起才把缺陷定位到"少了一次 `bridge::painted_to_host`"而不是坐标 / 夹具坏了。
#[test]
fn clicking_the_painted_note_rect_with_the_select_tool_selects_it_in_production_order() {
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
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let index = a_painted_note(&view, viewport_width, viewport_height, 0.0);
    let expected_ulid = view.notes[index].id.clone();
    let (host_x, host_y) = note_center_in_host_view(&view, index, 0.0);
    let (x, y) = note_center_in_painted_view(&view, index, 0.0);
    // 夹具自证：宿主命中测试在**宿主帧**命中的**就是**这一个音符（帧的几何一侧）。
    assert_eq!(
        view.hit_test_visible(0.0, viewport_width, host_x, host_y, NOTE_HEIGHT),
        Some(index),
        "夹具前提：宿主帧中心必须被宿主命中测试解析到这一个音符"
    );
    assert_eq!(window.get_selected_note_count(), 0, "起点不该已有选区");

    // 注入的是**绘制帧**坐标（人看得见的矩形中心）—— 与界面 `on_clicked` 收到的同一帧。
    click_in_roll_by_id_at(&mut plane, 10, x, y);

    let count = window.get_selected_note_count();
    let selected: Vec<String> = window
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    report_line(&format!(
        "[roll-select] 生产顺序 + 选择工具点击绘制帧 `{expected_ulid}` @({x},{y}) \
         （宿主帧 ({host_x},{host_y})）⇒ selected-note-count={count} selected-ulids={selected:?}"
    ));
    assert_eq!(
        count, 1,
        "选择工具点中**人看得见的**音符矩形必须恰好选中 1 个（宿主不减帧原点时这里恒为 0）"
    );
    assert!(
        selected.iter().any(|id| id == &expected_ulid),
        "选中的必须是点中的那个音符身份 `{expected_ulid}`"
    );
}

/// 判据 ②-b（**帧控制臂**）：把**宿主帧的数字**原样当绘制坐标注入（= 修复前判据的做法）
/// ⇒ **点不到**靶子。
///
/// 它把"两帧不是同一个数"钉死：宿主帧中心 `(host_x, host_y)` 换算回宿主后是
/// `(host_x − 68, host_y − 28)`，比音符框（12px 高）高了 28px ⇒ 靶子不可能被选中。
/// 修复前这条判据红（旧宿主把数字当宿主帧用，于是选中了靶子），修复后绿。
/// ⛔ 断言只看"靶子不在选区里"，不假定选区一定为空 —— 换回来的点可能压到别的音符上。
#[test]
fn a_click_at_the_host_frame_coordinates_does_not_select_the_painted_note() {
    let (window, port, mut plane) = assemble();
    use slint::Model as _;
    let view = yeban_app::bridge::ViewState::from_project(&port.project()).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let index = a_painted_note(&view, viewport_width, viewport_height, 0.0);
    let expected_ulid = view.notes[index].id.clone();
    let (host_x, host_y) = note_center_in_host_view(&view, index, 0.0);
    assert_eq!(window.get_selected_note_count(), 0, "起点不该已有选区");

    // 数字就是宿主帧中心；但注入面是绘制帧 ⇒ 宿主换算后会落到靶子左上方。
    click_in_roll_by_id_at(&mut plane, 10, host_x, host_y);

    let count = window.get_selected_note_count();
    let selected: Vec<String> = window
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    report_line(&format!(
        "[roll-frame] 把宿主帧数 ({host_x},{host_y}) 当绘制帧注入 ⇒ selected-note-count={count} \
         selected-ulids={selected:?}；靶子 `{expected_ulid}` 必须**不在**选区里"
    ));
    assert!(
        !selected.iter().any(|id| id == &expected_ulid),
        "宿主帧数字不是绘制坐标：帧原点 (68, 28) 未减时才会选中 `{expected_ulid}`（旧代码的读数）"
    );
}

/// 判据 ③（**覆盖控制臂**）：同样 `build_live_ui` 但**不调** `wire_roll_edit` 时，
/// 同一个**绘制帧**点击也选中它 —— 覆盖发生前的形态。
///
/// 它隔离出上一轮缺陷的位置：不是坐标、不是命中测试、也不是"选区逻辑不存在"，而是
/// `wire_roll_edit` 后装的 handler 把选区 handler 覆盖了。本判据在上一轮修复前后都绿；
/// 在本票的帧修复前后也都绿（它走的是同一个派发器里的同一条换算）。
#[test]
fn clicking_a_note_without_wire_roll_edit_still_selects_it() {
    let (window, mut plane) = assemble_without_roll_edit();
    use slint::Model as _;
    let project = yeban_model::samples::filled_project();
    let view = yeban_app::bridge::ViewState::from_project(&project).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let index = a_painted_note(&view, viewport_width, viewport_height, 0.0);
    let expected_ulid = view.notes[index].id.clone();
    let (host_x, host_y) = note_center_in_host_view(&view, index, 0.0);
    let (x, y) = note_center_in_painted_view(&view, index, 0.0);
    assert_eq!(
        view.hit_test_visible(0.0, viewport_width, host_x, host_y, NOTE_HEIGHT),
        Some(index),
        "夹具前提：宿主帧中心必须被宿主命中测试解析到这一个音符"
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
        "[roll-select] 覆盖控制臂（不调 wire_roll_edit）点击绘制帧 `{expected_ulid}` @({x},{y}) ⇒ \
         selected-note-count={count} selected-ulids={selected:?}"
    ));
    assert_eq!(
        count, 1,
        "覆盖控制臂：覆盖发生前，选区 handler 必须选中一个"
    );
    assert!(
        selected.iter().any(|id| id == &expected_ulid),
        "覆盖控制臂选中的必须是点中的那个音符身份 `{expected_ulid}`"
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
    let (host_x, host_y) = note_center_in_host_view(&view, index, 0.0);
    let (x, y) = note_center_in_painted_view(&view, index, 0.0);
    let bounds = element_bounds(&mut plane, 45, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    assert_eq!(
        view.hit_test_visible(0.0, viewport_width, host_x, host_y, NOTE_HEIGHT),
        Some(index),
        "夹具前提：橡皮擦靶子的宿主帧中心必须被宿主命中测试解析到它"
    );

    window.set_active_tool(i32::from(Tool::Eraser.digit()));
    // 橡皮擦也点**人看得见的**矩形（绘制帧），与选择 / 铅笔同一条 `painted_to_host`。
    click_in_roll_by_id_at(&mut plane, 50, x, y);

    let after = note_count(&port.project());
    report_line(&format!(
        "[roll-edit] 橡皮擦点击绘制帧 `{new_id}` @({x},{y})（宿主帧 ({host_x},{host_y})）\
         ⇒ 音符数 {before}＋1 ⇒ {after}；selected-note-count={}",
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

// ===========================================================================
// 本切片：`[UI-NOTE-003]` 左键拖拽的**移动臂**（选择工具）
// ===========================================================================
//
// ## 接了哪一格、⛔ 明确不接哪几格
//
// | 格 | 决定 |
// | :--- | :--- |
// | 移动臂（选择工具按在**音符**上拖 ⇒ 松手一次原子 `Op::MoveNote`） | **本切片接上** |
// | 空白处拖动 | ⛔ **语义一位不改**（仍是滚动；marquee 属规范 mermaid 与 aux 列，未裁决） |
// | 辅助键（`Shift` 微移 / `Alt` 复制） | ⛔ 不接（与 `[UI-NOTE-005]` 有未裁决冲突） |
// | 力度臂（`ToolDrag::AdjustVelocity`） | ⛔ 不接（四条子裁决未定，登记在交付报告） |
// | 键盘半场（`[UI-NOTE-005]`） | ⛔ 不接（`Space` / `Shift+Enter` 两处绑定冲突，登记在交付报告） |
//
// ## 每条判据证明什么 / ⛔ 不证明什么
//
// | 判据 | 证明 | ⛔ 不证明 |
// | :--- | :--- | :--- |
// | `move_arm_a_...` | 选择工具在音符上拖 ⇒ 权威工程的 `start_tick` / `pitch` 按**吸附后的指针位移**改变，且时值一位不动 | 拖动期的**画面**（草稿预览的像素）—— ⛔ 没有截图判据 |
// | `move_arm_b_...` | 音符拖动**不**顺带滚动（捕获）；并带一条**空白处拖动照旧滚动**的控制臂 | 空白处的 marquee 语义（本切片不实现，仍是滚动） |
// | `move_arm_c_...` | `Escape` 插在拖动中途 ⇒ 工程**逐字节不变**、提交 **0** 次 | 「必须有东西可取消」—— 未接移动臂时本判据**空转绿**（见文件末"怎么变红"） |
// | `move_arm_d_...` | 一次拖动（4 步 move）= **恰好一步**撤销；第二次手势再 +1 | 撤销**内容**的正确性（由 `move_arm_a` 与模型层判据管） |
// | `move_arm_e_...` | 结构防漂移：字面 `Op::MoveNote` 进 `crates/yeban-app/src` 生产区、`ToolDrag` 有 `src/input.rs` **之外**的消费者 | 行为（字面计数可被一行注释满足 ⇒ 行为由 a–d 管） |
//
// ## 夹具口径（与既有判据同一套）
//
// 指针一律注入**绘制帧**（人看得见的矩形）坐标，宿主回调再经**唯一**的
// `bridge::painted_to_host` 搬回宿主帧；期望值从**投影 / 权威工程**读（⛔ 不从被测代码读）。

/// 指针位移（**宿主帧**逻辑像素；两帧只差常量原点 ⇒ 位移在两帧里同值）。
///
/// 方向是**决定**，不是随便挑的：
/// - **向右 `+16px`**：默认 `30 tick/px` 下是 `+480 tick` = 恰好 **2 × 240** 吸附网格。
///   吸附对网格的整数倍平移是**不变**的 ⇒ 两端吸附点之差**恒等于 480**，与音符落在
///   网格的哪一侧无关。⛔ 向左则会被 `snapped_tick_at` 的 `x.max(0.0)` 在视口左缘夹住
///   （实测：宿主 x≈8 的音符向左拖 16px 被夹到 0 ⇒ 吸附增量变成 `−240`）——
///   那是**夹具**的性质，不是被测语义的性质；
/// - **向上 `−28px`**：恰好 **2 条 14px 泳道** ⇒ 音高增量与小数无关，恒为 `+2`。
const DRAG_HOST_DX: f32 = 16.0;
const DRAG_HOST_DY: f32 = -28.0;

/// 拖动**分几步**注入（`move` 的步数）。判据 (d) 靠它区分"一次拖动一步撤销"与"每步一次"。
const DRAG_STEPS: usize = 4;

/// 按语义 ID 在**绘制帧** `from` 处按下；返回 `piano-roll-scroll-area` 的绝对窗口原点
/// （后续 `move` 用绝对坐标注入，与既有 [`click_in_roll_by_id_at`] 同一手法）。
fn pointer_down_in_roll(plane: &mut LiveControlPlane, seq: i64, from: (f32, f32)) -> (f32, f32) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let origin_x = bounds["x"].as_f64().expect("x") as f32;
    let origin_y = bounds["y"].as_f64().expect("y") as f32;
    let down = plane.plane().try_line(&request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": SCROLL_AREA,
            "xOffset": f64::from(from.0),
            "yOffset": f64::from(from.1),
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    (origin_x, origin_y)
}

/// 从按下点分 `steps` 步拖到 `from + (host_dx, host_dy)`（**绝对窗口坐标**注入）。
fn pointer_drag_steps_in_roll(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_origin: (f32, f32),
    from: (f32, f32),
    host_dx: f32,
    host_dy: f32,
    steps: usize,
) {
    assert!(steps >= 1, "至少要一步移动，否则越不过规范的 4px 阈值");
    for step in 1..=steps {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f32 / steps as f32;
        let x = element_origin.0 + from.0 + host_dx * fraction;
        let y = element_origin.1 + from.1 + host_dy * fraction;
        let moved = plane.plane().try_line(&request_line(
            seq + 1 + i64::try_from(step).expect("步号"),
            METHOD_DISPATCH_POINTER_MOVE,
            Some(json!({ "x": f64::from(x), "y": f64::from(y) })),
        ));
        assert!(
            !moved.is_error(),
            "第 {step} 步移动必须落到真实窗口: {moved:?}"
        );
    }
}

/// 松手（位置复用最后一次按下 / 移动，`[UI-TEST-002]` §12.4 的口径）。
fn pointer_up_in_roll(plane: &mut LiveControlPlane, seq: i64) {
    let up = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

/// 注入一次指针**取消**（`PointerEventKind.cancel`；`.slint` 侧由 `PointerExited` 送达）。
///
/// ⚠ 入参只有 `dryRun`（可选）：上游 `WindowEvent::PointerExited` 没有坐标也没有按键
/// （`i-slint-core-1.18.1/platform.rs:395-399`）⇒ 这里**不**传 `button` / `x` / `y`
/// （传了会被参数校验以 `-32602` 拒掉，而不是被静默忽略）。
fn pointer_cancel_in_roll(plane: &mut LiveControlPlane, seq: i64) {
    let cancelled = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_CANCEL,
        Some(json!({})),
    ));
    assert!(
        !cancelled.is_error(),
        "指针取消必须落到真实窗口: {cancelled:?}"
    );
    assert_eq!(
        cancelled.result.expect("有 result")["dispatched"],
        json!("pointer_cancel"),
        "回执必须自证走的是取消分支"
    );
}

/// 按下 → 分步拖动 → 松手（一次完整手势）。
fn drag_in_roll_by_id_at(
    plane: &mut LiveControlPlane,
    seq: i64,
    from: (f32, f32),
    host_dx: f32,
    host_dy: f32,
    steps: usize,
) {
    let origin = pointer_down_in_roll(plane, seq, from);
    pointer_drag_steps_in_roll(plane, seq + 1, origin, from, host_dx, host_dy, steps);
    pointer_up_in_roll(plane, seq + 1 + i64::try_from(steps).expect("步数") + 1);
}

/// 注入一次 `Escape`（`Action::Cancel` 的唯一键盘入口）。
fn press_escape(plane: &mut LiveControlPlane, seq: i64) {
    let pressed = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({ "keyCode": "Escape" })),
    ));
    assert!(!pressed.is_error(), "`Escape` 注入必须成功: {pressed:?}");
}

/// 按 ULID 从**权威工程**读一个音符的 `(start_tick, pitch, duration_ticks)`。
fn note_by_ulid(project: &yeban_model::YebanProjectV1, ulid: &str) -> (u64, u8, u64) {
    for entry in project.clip_pool.values() {
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for (id, note) in map {
            if id.to_canonical_string() == ulid {
                return (note.start_tick, note.pitch, note.duration_ticks);
            }
        }
    }
    panic!("权威工程里必须有音符 `{ulid}`");
}

/// 找一个**空白网格**点（宿主帧）：`hit_test_visible` 在那里解析不到任何音符，
/// 且它的绘制帧仍落在卷帘元素矩形内。用于"滚动没有被音符手势接管"的**控制臂**。
fn a_blank_point(
    view: &yeban_app::bridge::ViewState,
    viewport_width: f32,
    viewport_height: f32,
    scroll: f32,
) -> (f32, f32) {
    let mut y = 8.0_f32;
    while y < viewport_height {
        let mut x = 1.0_f32;
        while x < viewport_width {
            if view
                .hit_test_visible(scroll, viewport_width, x, y, NOTE_HEIGHT)
                .is_none()
            {
                let (painted_x, painted_y) = host_to_painted(x, y);
                if (0.0..=viewport_width).contains(&painted_x)
                    && (0.0..=viewport_height).contains(&painted_y)
                {
                    return (x, y);
                }
            }
            x += 7.0;
        }
        y += 7.0;
    }
    panic!("夹具里必须存在一个宿主帧空白点，其绘制帧也落在卷帘元素矩形内")
}

/// 靶子音符的一切读数（判据共用的装配段）。
struct RollTarget {
    ulid: String,
    host_center: (f32, f32),
    painted_center: (f32, f32),
    before: (u64, u8, u64),
}

/// 装配一个"人真的拖得动"的靶子音符（见 [`a_painted_note`] 的两个条件）。
fn target_note(
    window: &yeban_app::ui::MainWindow,
    plane: &mut LiveControlPlane,
    project: &yeban_model::YebanProjectV1,
    seq: i64,
    scroll: f32,
) -> RollTarget {
    let view = yeban_app::bridge::ViewState::from_project(project).expect("投影");
    assert_eq!(
        view.ticks_per_pixel,
        yeban_app::bridge::DEFAULT_TICKS_PER_PIXEL,
        "夹具前提：窗口的缩放就是投影的默认缩放（否则吸附期望值不是界面那一套）"
    );
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：默认工具必须是选择箭头（本切片只接选择工具的拖拽臂）"
    );
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let index = a_painted_note(&view, viewport_width, viewport_height, scroll);
    let ulid = view.notes[index].id.clone();
    RollTarget {
        before: note_by_ulid(project, &ulid),
        host_center: note_center_in_host_view(&view, index, scroll),
        painted_center: note_center_in_painted_view(&view, index, scroll),
        ulid,
    }
}

/// 判据 (a)：选择工具在音符上拖动 ⇒ 权威工程的 `start_tick` 与 `pitch` 按**吸附后的
/// 指针位移**改变，且**时值一位不动**（是"移动"不是"改时值"）。
#[test]
fn move_arm_a_a_note_drag_moves_the_note_by_the_grid_snapped_pointer_delta() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let target = target_note(&window, &mut plane, &project_before, 1, 0.0);
    assert_eq!(window.get_roll_scroll_x(), 0.0, "夹具前提：起点偏移为 0");
    // ---- 独立期望值（⛔ 不读被测的拖拽代码）----
    //
    // ① tick：用**投影自己的吸附口径**（`snapped_tick_at`，唯一真相源）算两端之差；
    let view_before = yeban_app::bridge::ViewState::from_project(&project_before).expect("投影");
    let origin_tick = view_before.snapped_tick_at(0.0, target.host_center.0, ROLL_SNAP_GRID_TICKS);
    let current_tick = view_before.snapped_tick_at(
        0.0,
        target.host_center.0 + DRAG_HOST_DX,
        ROLL_SNAP_GRID_TICKS,
    );
    let expected_delta_tick = i64::try_from(current_tick).expect("tick 适配 i64")
        - i64::try_from(origin_tick).expect("tick 适配 i64");
    // ② 音高：**手算** 14px 泳道几何 —— `-28px` 恰好两条泳道 ⇒ `+2` 半音（与小数无关）。
    #[allow(clippy::cast_possible_truncation)]
    let expected_delta_pitch = (-DRAG_HOST_DY / 14.0) as i32;
    assert_eq!(
        expected_delta_pitch, 2,
        "夹具自证：28px 恰好是 2 条 14px 泳道"
    );
    // ③ 位移必须可解释为"指针动了多少 tick"：吸附增量与原始位移之差 ≤ 半个网格。
    #[allow(clippy::cast_precision_loss)]
    let raw_tick_delta = f64::from(DRAG_HOST_DX) * view_before.ticks_per_pixel as f64;
    assert!(
        (expected_delta_tick as f64 - raw_tick_delta).abs() <= ROLL_SNAP_GRID_TICKS as f64 / 2.0,
        "吸附后的 tick 增量必须落在原始指针位移的半个网格内：吸附 {expected_delta_tick} vs 原始 {raw_tick_delta}"
    );
    assert_eq!(
        expected_delta_tick,
        2 * i64::try_from(ROLL_SNAP_GRID_TICKS).expect("网格"),
        "夹具自证：16px 在 30tpp 下是 480 tick = 恰好 2 个 240 网格（吸附对网格整数倍平移不变）"
    );
    assert_ne!(expected_delta_tick, 0, "夹具前提：这一拖必须真的跨过网格");
    assert_eq!(
        expected_delta_tick % i64::try_from(ROLL_SNAP_GRID_TICKS).expect("网格"),
        0,
        "吸附后的位移必须是整格"
    );

    drag_in_roll_by_id_at(
        &mut plane,
        10,
        target.painted_center,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );

    let project_after = port.project();
    let after = note_by_ulid(&project_after, &target.ulid);
    report_line(&format!(
        "[roll-move-a] `{}` 绘制帧 ({:.1},{:.1}) 拖 ({DRAG_HOST_DX},{DRAG_HOST_DY})px × {DRAG_STEPS} 步 ⇒ \
         start_tick {} ⇒ {}（期望 {:+}）; pitch {} ⇒ {}（期望 {:+}）; duration {} ⇒ {}",
        target.ulid,
        target.painted_center.0,
        target.painted_center.1,
        target.before.0,
        after.0,
        expected_delta_tick,
        target.before.1,
        after.1,
        expected_delta_pitch,
        target.before.2,
        after.2
    ));
    assert_eq!(
        i64::try_from(after.0).expect("tick") - i64::try_from(target.before.0).expect("tick"),
        expected_delta_tick,
        "`start_tick` 必须按 `snapped_tick_at` 算出的吸附位移改变"
    );
    assert_eq!(
        i32::from(after.1) - i32::from(target.before.1),
        expected_delta_pitch,
        "`pitch` 必须按 14px 泳道几何改变（-28px ⇒ +2 半音）"
    );
    assert_eq!(
        after.2, target.before.2,
        "移动**不得**改时值（那是铅笔的拖拽臂，本切片 ⛔ 不接）"
    );
}

/// 判据 (b)：音符拖动**不**顺带滚动；控制臂 = **同一次装配里**的空白处拖动照旧滚动。
#[test]
fn move_arm_b_a_note_drag_does_not_scroll_the_roll() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let view_before = yeban_app::bridge::ViewState::from_project(&project_before).expect("投影");
    assert_eq!(window.get_roll_scroll_x(), 0.0, "夹具前提：起点偏移为 0");

    // ---- 控制臂 + 前置：先把视图滚到**非零**偏移 ----
    // 为什么要预滚（这是一条**防假绿**的决定）：向右拖在**零偏移**下会被宿主
    // `advance_scroll` 的 `0.0` 下限夹住 ⇒ **未修复**的代码也会"看起来没滚动"。
    // 预滚之后，未被捕获的手势会把偏移从 40 改成 24（可观测），被捕获的则一位不变。
    // 同一条空白处拖动本身就是控制臂：它证明"滚动这条路"在同一份代码上仍然通。
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let (blank_x, blank_y) = a_blank_point(&view_before, viewport_width, viewport_height, 0.0);
    let (blank_painted_x, blank_painted_y) = host_to_painted(blank_x, blank_y);
    drag_in_roll_by_id_at(
        &mut plane,
        10,
        (blank_painted_x, blank_painted_y),
        -40.0,
        0.0,
        DRAG_STEPS,
    );
    let scroll_base = window.get_roll_scroll_x();
    assert!(
        scroll_base > 0.5,
        "控制臂：空白处拖动必须把偏移推到非零（否则本判据证明不了\"捕获\"）—— 实测 {scroll_base}"
    );

    // ---- 在**新偏移**下取靶 ----
    let target = target_note(&window, &mut plane, &project_before, 40, scroll_base);
    let scroll_before = window.get_roll_scroll_x();
    assert!(
        (scroll_before - scroll_base).abs() < 0.001,
        "夹具前提：取靶本身不得改变偏移"
    );

    drag_in_roll_by_id_at(
        &mut plane,
        60,
        target.painted_center,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );
    let scroll_after = window.get_roll_scroll_x();
    // 非空转：这一拖必须真的移动了模型（否则"没滚动"可能只是"什么都没发生"）。
    let moved = note_by_ulid(&port.project(), &target.ulid).0 != target.before.0;
    report_line(&format!(
        "[roll-move-b] 预滚偏移 {scroll_base}；音符（宿主帧中心 {:.1},{:.1}）向右拖 \
         ({DRAG_HOST_DX},{DRAG_HOST_DY})px × {DRAG_STEPS} 步 ⇒ `roll-scroll-x` {scroll_before} ⇒ \
         {scroll_after}（必须一位不变）；同一拖里音符真的移动了 = {moved}",
        target.host_center.0, target.host_center.1
    ));
    assert!(
        (scroll_after - scroll_before).abs() < 0.001,
        "音符拖动**不得**同时滚动（手势已被捕获）：{scroll_before} ⇒ {scroll_after}"
    );
    assert!(
        moved,
        "非空转前提：这一拖必须真的改变 `start_tick`（判据 (a) 的内容）"
    );
}

/// 判据 (c)：`Escape` 插在拖动中途 ⇒ 工程**逐字节不变**、提交 **0** 次。
///
/// ⚠ 本判据在**未接移动臂**的代码上**空转绿**（选择工具本来就不写工程）—— 它的对照
/// 不在"修复前 / 修复后"，而在"**取消这条收尾路径**在不在"（见文件末"怎么变红"）。
#[test]
fn move_arm_c_escape_mid_drag_leaves_the_project_byte_identical_and_commits_nothing() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let before_bytes = serde_json::to_vec(&project_before).expect("序列化起点工程");
    let target = target_note(&window, &mut plane, &project_before, 1, 0.0);
    let commits_before = port.display().commit_count;

    let element_origin = pointer_down_in_roll(&mut plane, 10, target.painted_center);
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        element_origin,
        target.painted_center,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );
    // 拖到一半：`Escape`（`Action::Cancel` 的唯一键盘入口）⇒ 丢弃草稿、⛔ 不提交。
    press_escape(&mut plane, 30);
    pointer_up_in_roll(&mut plane, 40);

    let project_after = port.project();
    let after_bytes = serde_json::to_vec(&project_after).expect("序列化终点工程");
    let commits_after = port.display().commit_count;
    report_line(&format!(
        "[roll-move-c] 拖动中途 `Escape` 后松手：工程字节 {} ⇒ {}（逐字节相同 = {}）；\
         提交数 {} ⇒ {}；undoable={}",
        before_bytes.len(),
        after_bytes.len(),
        before_bytes == after_bytes,
        commits_before,
        commits_after,
        port.display().undoable
    ));
    assert_eq!(
        before_bytes, after_bytes,
        "`Escape` 取消后工程必须**逐字节**不变（草稿从未写进工程，收尾后也不许补写）"
    );
    assert_eq!(
        commits_after, commits_before,
        "`Escape` 取消不得提交任何 op"
    );
    assert_eq!(port.display().undoable, 0, "取消之后不得有可撤销的编辑");
}

/// 判据 (c2) 的**控制臂**：同一手势改用 `up` 收尾 ⇒ 音符真的移动、恰好提交 1 次。
///
/// 它与 [`pointer_cancel_mid_drag_discards_the_draft_and_commits_nothing`] 是**一对**：
/// 控制臂证明"按下 → 4 步 move → 收尾"这条路是**活的**，因此见证臂的"什么都没发生"
/// 不是空转。
///
/// ⚠ 两条**必须**各自是一个 `#[test]`：Tier-1 平台每个线程只能 `set_platform` 一次
/// （`crates/yeban-ui-test-port/src/render.rs` 的线程约束），一个测试里调两次 `assemble()`
/// 会以 `PlatformUnavailable { "The platform has already been initialized." }` 红 —— 实测过。
#[test]
fn pointer_up_mid_drag_is_the_control_arm_for_the_cancel_witness() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let target = target_note(&window, &mut plane, &project_before, 1, 0.0);
    assert_eq!(port.display().undoable, 0, "控制臂：起点不该有可撤销的编辑");
    let commits_before = port.display().commit_count;
    let origin = pointer_down_in_roll(&mut plane, 10, target.painted_center);
    assert!(
        window.get_note_drag_active(),
        "控制臂前提：选择工具按在音符上必须由宿主接管（`note-drag-active`）"
    );
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        origin,
        target.painted_center,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );
    pointer_up_in_roll(&mut plane, 40);
    let (tick_after, _, _) = note_by_ulid(&port.project(), &target.ulid);
    report_line(&format!(
        "[roll-cancel-control] 同一手势改用 `up` 收尾：音符 tick {} ⇒ {tick_after}；\
         提交数 {commits_before} ⇒ {}；undoable {}",
        target.before.0,
        port.display().commit_count,
        port.display().undoable
    ));
    assert_ne!(
        tick_after, target.before.0,
        "控制臂：`up` 收尾必须真的移动音符（否则取消臂的『什么都没发生』证明不了任何事）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "控制臂：`up` 收尾必须恰好提交一个 op"
    );
}

/// 判据 (c2)：**指针取消**（`PointerEventKind.cancel`）插在拖动中途 ⇒ 工程**逐字节不变**、
/// 提交 **0** 次、可撤销步数不变，且收尾**真的跑了**（`note-drag-active` 归零）。
///
/// ## 这条判据补哪一个空格（本切片唯一的缺口）
///
/// `cancel_note_drag` / `cancel_velocity_drag` 两个**函数**本来就有 `Escape` 的证人
/// （`move_arm_c_*` 与 `velocity_arm_d_*`）。无证人的是 `.slint` 的 **`cancel` 分支**
/// （`ui/console/piano_roll.slint` 的 `else` 臂：`root.note-drag-cancel()`）与它的接线
/// （`src/host.rs` 的 `ui.on_note_drag_cancel(...)`）—— 在控制面没有
/// `ui/dispatch_pointer_cancel` 之前，**没有任何办法**把 `PointerEventKind.cancel` 注入进去。
///
/// ## 三件事一起才叫"取消被见证了"（防空转）
///
/// 1. **手势真的在手**：按下靶音符后 `note-drag-active == true`（宿主已接管），
///    随后 4 步 `move` 越过规范的 `4px` 阈值 —— 同一个手势若走 `up` 会**提交**
///    （兄弟判据 `pointer_up_mid_drag_is_the_control_arm_for_the_cancel_witness` 逐字证明）；
/// 2. **取消后不提交**：工程字节 == 拖动前、`commit_count` 不变、`undoable` 不变；
/// 3. **收尾真的跑了**：取消后 `note-drag-active == false`。它**只**由 `cancel_note_drag`
///    （取消）或 `commit_note_drag`（提交）清零；既然 ② 证明没提交，归零就只能是
///    `on_note_drag_cancel` → `cancel_note_drag` 这条线。
///    ⚠ 再补一次 `up`：若那个 `cancel` **没送达** `TouchArea`，`.slint` 的 `note-dragging`
///    与 `drag-moved` 仍为真 ⇒ 这一 `up` 会走 `note-drag-release` 提交一个 op；
///    它仍报 0 才是"取消确实到达了控件的抓取通道"的机械证据。
#[test]
fn pointer_cancel_mid_drag_discards_the_draft_and_commits_nothing() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let before_bytes = serde_json::to_vec(&project_before).expect("序列化起点工程");
    let target = target_note(&window, &mut plane, &project_before, 1, 0.0);
    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");
    let commits_before = port.display().commit_count;

    let origin = pointer_down_in_roll(&mut plane, 10, target.painted_center);
    assert!(
        window.get_note_drag_active(),
        "夹具前提：这一按必须由宿主接管（否则 `.slint` 的 cancel 分支根本不会被走到）"
    );
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        origin,
        target.painted_center,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );
    let active_at_cancel = window.get_note_drag_active();
    assert!(
        active_at_cancel,
        "拖动中途手势必须仍在手（`note-drag-active`）—— 否则下面取消的是空气"
    );

    pointer_cancel_in_roll(&mut plane, 40);
    let active_after_cancel = window.get_note_drag_active();
    // 取消若没送达，这次 `up` 会补一次 `note-drag-release` 提交 ⇒ 它仍报 0 才是送达证据。
    pointer_up_in_roll(&mut plane, 50);

    let project_after = port.project();
    let after_bytes = serde_json::to_vec(&project_after).expect("序列化终点工程");
    report_line(&format!(
        "[roll-cancel] 拖动中途 `PointerEventKind.cancel` 后松手：工程字节 {} ⇒ {}（逐字节相同 = {}）；\
         提交数 {} ⇒ {}；undoable={}；`note-drag-active` {active_at_cancel} ⇒ {active_after_cancel}",
        before_bytes.len(),
        after_bytes.len(),
        before_bytes == after_bytes,
        commits_before,
        port.display().commit_count,
        port.display().undoable,
    ));
    assert_eq!(
        before_bytes, after_bytes,
        "指针取消后工程必须**逐字节**不变（草稿从未写进工程，收尾后也不许补写）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "指针取消不得提交任何 op（含随后那次 `up`）"
    );
    assert_eq!(port.display().undoable, 0, "取消之后不得有可撤销的编辑");
    assert!(
        !active_after_cancel,
        "取消后 `note-drag-active` 必须归零 —— 它只由取消/提交两条收尾清零；\
         既然提交数没动，归零就只能是 `on_note_drag_cancel` → `cancel_note_drag` 跑了"
    );
    let (tick_after, pitch_after, dur_after) = note_by_ulid(&port.project(), &target.ulid);
    assert_eq!(
        (tick_after, pitch_after, dur_after),
        target.before,
        "取消后靶音符必须一位不动（tick / pitch / 时值）"
    );
}

/// 判据 (d)：一次拖动（`DRAG_STEPS` 步 `move`）= **恰好一步**撤销；第二次手势再 +1。
///
/// 这正是"草稿在手、松手才提交"设计的意义：⛔ 不是每个 `move` 提交一次。
#[test]
fn move_arm_d_exactly_one_history_entry_per_gesture() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let target = target_note(&window, &mut plane, &project_before, 1, 0.0);
    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");
    let commits_before = port.display().commit_count;

    // ---- 第一次手势 ----
    drag_in_roll_by_id_at(
        &mut plane,
        10,
        target.painted_center,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );
    let undoable_first = port.display().undoable;
    let commits_first = port.display().commit_count;
    let after_first = note_by_ulid(&port.project(), &target.ulid);

    // ---- 第二次手势：从**工程读回的**新位置重新取靶（顺带见证重投影没把画面落下）----
    let view_second = yeban_app::bridge::ViewState::from_project(&port.project()).expect("投影");
    let index_second = view_second
        .notes
        .iter()
        .position(|note| note.id == target.ulid)
        .expect("被移动的音符必须仍在投影里");
    let painted_second = note_center_in_painted_view(&view_second, index_second, 0.0);
    // 第二次也**向右 / 向上**：`Op::MoveNote` 的前置条件只拒绝"结果越界"
    // （`shifted_tick` / `shifted_pitch` 为 `None`）⇒ 反向拖回可能因越下界被模型拒绝，
    // 那样"撤销步数 = 2"就会因**模型拒绝**而假红。同向两次 ⇒ 结果必然合法。
    drag_in_roll_by_id_at(
        &mut plane,
        50,
        painted_second,
        DRAG_HOST_DX,
        DRAG_HOST_DY,
        DRAG_STEPS,
    );
    let undoable_second = port.display().undoable;
    let commits_second = port.display().commit_count;
    let after_second = note_by_ulid(&port.project(), &target.ulid);
    report_line(&format!(
        "[roll-move-d] 每次手势 {DRAG_STEPS} 步 move：撤销步数 0 ⇒ {undoable_first} ⇒ {undoable_second}；\
         提交数 {commits_before} ⇒ {commits_first} ⇒ {commits_second}；\
         起点 tick {}，第一次后 {}，第二次后 {}",
        target.before.0, after_first.0, after_second.0
    ));
    assert_eq!(
        undoable_first, 1,
        "一次拖动 = **恰好一步**撤销（若每个 `move` 都提交，这里会是 {DRAG_STEPS}）"
    );
    assert_eq!(
        commits_first - commits_before,
        1,
        "一次拖动恰好**一次**提交"
    );
    assert_ne!(
        after_first.0, target.before.0,
        "非空转前提：第一次手势必须真的移动音符（否则上面的计数可能只是\"什么都没发生\"）"
    );
    assert_eq!(undoable_second, 2, "第二次手势再 +1（不是一次性的效果）");
    assert_eq!(commits_second - commits_before, 2, "两次手势恰好两次提交");
}

/// 判据 (e)：**结构防漂移**。计数规则（逐条写明，单位 = **字面出现次数**）：
///
/// ① 单位 = `str::matches(needle).count()` 的**子串出现次数**（⛔ 不是行数、⛔ 不是"消费者个数"）；
/// ② 目录 = `<crate>/src` **递归**，只收 `*.rs`（`CARGO_MANIFEST_DIR` 起算）；
/// ③ `crates/yeban-app/tests/**` **不在**扫描范围（本函数只走 `src`）；
/// ④ 每个文件只取**第一个 `#[cfg(test)]` 之前**的文本 = 该文件的生产区
///    ⇒ `src/**` 内联 `#[cfg(test)] mod tests` 里的字面**不计**；
/// ⑤ "消费者"那一条把 `src/input.rs` 整个**排除**（那是拖拽分类自己的定义处）。
///
/// **它证明什么**：本切片之后，`Op::MoveNote` 在 app 层有一个生产写者、`ToolDrag` 有一个
/// `input.rs` 之外的消费者 —— 两处今天都是 **0**。
/// **它不证明什么**：行为（一行注释就能让字面计数 ≥1）⇒ 行为由 (a)–(d) 管；也不证明
/// 每条 `ToolDrag` 变体都被消费（`AdjustVelocity` **故意**仍未被消费，见交付报告）。
#[test]
fn move_arm_e_the_drag_taxonomy_gains_a_production_consumer() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);
    files.sort();
    // 走查自证：路径错了会让计数恒 0（判据会红），但也可能"恰好"命中别的文件 ⇒ 先把两个
    // 必在场的源文件钉住。
    let names: Vec<String> = files
        .iter()
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    assert!(
        names.iter().any(|name| name == "input.rs"),
        "走查自证：`src/input.rs` 必须在扫描集里，实际 {names:?}"
    );
    assert!(
        names.iter().any(|name| name == "host.rs"),
        "走查自证：`src/host.rs` 必须在扫描集里，实际 {names:?}"
    );

    let mut move_note = 0_usize;
    let mut tool_drag_outside_input = 0_usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("读源文件");
        let end = text.find("#[cfg(test)]").unwrap_or(text.len());
        let production = &text[..end];
        move_note += production.matches("Op::MoveNote").count();
        if path.file_name().is_some_and(|name| name != "input.rs") {
            tool_drag_outside_input += production.matches("ToolDrag").count();
        }
    }
    report_line(&format!(
        "[roll-move-e] 生产区（`src/**` 首个 `#[cfg(test)]` 之前）字面计数：\
         `Op::MoveNote` = {move_note}（本切片之前 0）；`ToolDrag` 在 `src/input.rs` 之外 = \
         {tool_drag_outside_input}（本切片之前 0）；扫描 {} 个 `.rs` 文件",
        files.len()
    ));
    assert!(
        move_note >= 1,
        "字面 `Op::MoveNote` 必须在 `crates/yeban-app/src` 的生产区出现 ≥1 次（本切片之前 = 0）"
    );
    assert!(
        tool_drag_outside_input >= 1,
        "`ToolDrag` 必须在 `src/input.rs` **之外**有 ≥1 个生产消费者（本切片之前 = 0）"
    );
}

/// 递归收集 `dir` 下的 `*.rs`。
fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("读源码目录") {
        let path = entry.expect("目录项").path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// **相邻缺陷**的判据（与本切片同一条通道 `[UI-NOTE-003]`）：铅笔工具下的**滚动拖拽**
/// 不得在松手处插入音符。
///
/// ## 缺陷（本判据在修复前**红**）
///
/// `piano_roll.slint` 的 `up` 曾经**无条件** fired 一次 `clicked` ⇒ 一次"滚动拖动"在松手时
/// 被当成一次单击，铅笔于是把音符插在松手处。根因与移动臂**同一条通道**：`up` 没有区分
/// "位移越过 4px 的**拖拽**"与"没越过的**单击**"（规范 §6 的 4px 距离定义）。
///
/// 靶点必须**同时**满足两个条件，才保证修复前的代码**真的**会插进一个音符：
/// ① 该点在**空白**网格上（`hit_test_visible` 无命中）；② 该点落在片段内（`pencil_op_for`
/// 有解）。⛔ 少任何一条，修复前也可能"恰好没插"，判据就成了空转绿。
///
/// **它证明什么**：`up` 的单击 / 拖拽分流真的生效于**所有工具**（不只是被接管的移动臂），
/// 因此"滚动一下 = 插入一个音符"这条缺陷路径不再存在。
/// **它不证明什么**：其它工具（剪刀 / 橡皮擦 / 力度）的**拖拽**语义 —— 那些仍未接线。
#[test]
fn a_scroll_drag_with_the_pencil_tool_does_not_insert_a_note() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let before = note_count(&project_before);
    let view = yeban_app::bridge::ViewState::from_project(&project_before).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;

    // 靶点：空白 **且** 铅笔画得出来。
    let mut target = None;
    let mut y = 8.0_f32;
    while y < viewport_height && target.is_none() {
        let mut x = 1.0_f32;
        while x < viewport_width {
            if view
                .hit_test_visible(0.0, viewport_width, x, y, NOTE_HEIGHT)
                .is_none()
                && pencil_op_for(
                    &project_before,
                    0.0,
                    x,
                    y,
                    ROLL_SNAP_GRID_TICKS,
                    i32::from(PENCIL_TOOL_DIGIT),
                )
                .is_some()
            {
                target = Some((x, y));
                break;
            }
            x += 7.0;
        }
        y += 7.0;
    }
    let (host_x, host_y) = target.expect("夹具里必须存在一个既空白又在片段内的点");
    let (painted_x, painted_y) = host_to_painted(host_x, host_y);
    window.set_active_tool(i32::from(Tool::Pencil.digit()));
    let scroll_before = window.get_roll_scroll_x();

    drag_in_roll_by_id_at(
        &mut plane,
        10,
        (painted_x, painted_y),
        -40.0,
        0.0,
        DRAG_STEPS,
    );

    let after = note_count(&port.project());
    let scroll_after = window.get_roll_scroll_x();
    report_line(&format!(
        "[roll-scroll-defect] 铅笔下的**滚动拖拽**（空白且在片段内的点 宿主帧 {host_x},{host_y}，左拖 40px）⇒ \
         音符数 {before} ⇒ {after}（必须不变）；`roll-scroll-x` {scroll_before} ⇒ {scroll_after}（必须照旧滚动）"
    ));
    assert!(
        (scroll_after - scroll_before).abs() > 0.5,
        "非空转前提：这一拖必须真的滚动了（否则测的不是\"滚动拖拽\"）"
    );
    assert_eq!(
        after, before,
        "滚动拖拽**不得**在松手处插入音符（`up` 只在位移未越 4px 阈值时才算一次单击）"
    );
}

// ===========================================================================
// 本切片：`[UI-NOTE-003]` 左键拖拽的**力度臂**（力度工具，工具行 4）
// ===========================================================================
//
// ## 接了哪一格、⛔ 明确不接哪几格
//
// | 格 | 决定 |
// | :--- | :--- |
// | 力度臂（力度工具按在**力度柱**上竖直拖 ⇒ 松手一个原子 `Op::ModifyNoteVelocity`） | **本切片接上** |
// | 多选作用域（拖一根柱是否带动整个选区） | ⛔ 只作用**指针下那一个**音符（登记为 pending） |
// | 零力度音符（柱高 0 ⇒ 不可命中） | ⛔ 不给柱体加最小高度、⛔ 不发明 affordance（登记为 pending） |
// | 双击到 100（`[UI-NOTE-003]` aux 列 —— 把零力度音符提起来的**规范路径**） | ⛔ 本切片**不实现**（登记为 pending） |
// | 空白处拖动 | ⛔ 语义一位不改（仍是滚动）—— 判据 (f) 是兄弟臂行为改变的**回归钉** |
//
// ## 每条判据证明什么 / ⛔ 不证明什么
//
// | 判据 | 证明 | ⛔ 不证明 |
// | :--- | :--- | :--- |
// | `velocity_arm_a_...` | 拖到某条车道内 y ⇒ 权威工程力度 = **判据自己按车道几何算出的绝对数** | 拖动期的**画面**（草稿柱高）—— ⛔ 无截图判据 |
// | `velocity_arm_b_...` | 拖到车道上 / 下沿之外 ⇒ **恰好** 127 / 0（夹紧，⛔ 不越界） | 区间内的取值（由 (a) 管） |
// | `velocity_arm_c_...` | 一次手势（4 步 move）= **恰好一步**撤销；第二次手势再 +1 | 撤销**内容**的正确性（由 (a) 与模型层管） |
// | `velocity_arm_d_...` | `Escape` 插在拖动中途 ⇒ 工程**逐字节不变**、提交 **0** 次 | 未接力度臂的代码上它**空转绿**（同一条臂的兄弟判据踩过这个坑）—— 红必须靠**变异**挣（见该判据） |
// | `velocity_arm_e_...` | 结构防漂移：**去注释后** `ToolDrag::AdjustVelocity` 在 `src/input.rs` 之外 ≥1 次；且枚举**每个变体**都标了「已接线 / 未消费」 | 可达性（字面出现也可能是死分支）⇒ 行为由 (a)–(d) 管 |
// | `a_blank_drag_...`（(f)） | 选择工具下空白处 > 4px 拖动**不清空选区** | 它**不是**本切片修的行为：兄弟臂（`ac2c72f1`）已经把无条件的 `up ⇒ clicked` 改成分流 ⇒ 在本切片之前的代码上它也**不能**变红（回归钉，红靠变异挣） |
//
// ## 期望值的来源（为什么不是"把被测代码读回来"）
//
// 力度期望值由判据**自己**按 `piano_roll.slint` 的几何（`44` / `28` / `4` / `127`）与注入的
// **车道内 y** 算出（见 [`expected_velocity_at_lane_y`]）。那三个数与 `.slint` 的一致性由
// **运行时树**里的真实包围盒在 [`velocity_target`] 里交叉核对（车道高 44、柱高 `28 * v/127`、
// 柱顶沿 `40 - 28 * v/127`）—— 若判据的几何假设与界面不符，夹具自己先红。

/// 力度车道的几何（**判据侧独立的**手抄值，与 `piano_roll.slint` 的
/// `y: parent.height - 44px` / `28px * velocity` / `- 4px` / `+ 30px` / `6px` 同源）。
const VELOCITY_LANE_HEIGHT: f32 = 44.0;
const VELOCITY_BAR_MAX_HEIGHT: f32 = 28.0;
const VELOCITY_LANE_BOTTOM_MARGIN: f32 = 4.0;
const VELOCITY_BAR_OFFSET_X: f32 = 30.0;
const VELOCITY_BAR_WIDTH: f32 = 6.0;

/// 车道内 y ⇒ 期望的**绝对**力度（**判据自己**算，⛔ 不读被测代码）。
///
/// 口径：柱底沿（`44 - 4`）= 0，往上 `28px` = 127，区间外**夹紧**，四舍五入到整数。
fn expected_velocity_at_lane_y(lane_y: f32) -> u8 {
    let fraction = ((VELOCITY_LANE_HEIGHT - VELOCITY_LANE_BOTTOM_MARGIN - lane_y)
        / VELOCITY_BAR_MAX_HEIGHT)
        .clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let velocity = (fraction * 127.0).round() as u8;
    velocity.min(127)
}

/// 力度 `velocity` 的柱**顶沿**（车道内 y，0 = 车道顶沿）——"把这个力度拖出来"要落的 y。
fn velocity_bar_top_lane_y(velocity: u8) -> f32 {
    (VELOCITY_LANE_HEIGHT - VELOCITY_LANE_BOTTOM_MARGIN)
        - VELOCITY_BAR_MAX_HEIGHT * (f32::from(velocity) / 127.0)
}

/// 包围盒里的一个 f32 分量（缺键 / 非数 ⇒ panic：静默 0 会让判据"看起来有"）。
#[allow(clippy::cast_possible_truncation)]
fn bound_f32(bounds: &serde_json::Value, key: &str) -> f32 {
    bounds[key]
        .as_f64()
        .unwrap_or_else(|| panic!("包围盒必须有 `{key}`: {bounds}")) as f32
}

/// 语义 ID 的包围盒，**不存在时返回 `None`**（[`element_bounds`] 是"必须存在"版）。
fn try_element_bounds(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
) -> Option<serde_json::Value> {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    if node.is_error() {
        return None;
    }
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    if bounds.is_null() { None } else { Some(bounds) }
}

/// 工程里某个音符的**权威力度**（0–127）。
fn velocity_of(project: &yeban_model::YebanProjectV1, ulid: &str) -> u8 {
    for entry in project.clip_pool.values() {
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for (id, note) in map {
            if id.to_canonical_string() == ulid {
                return note.velocity;
            }
        }
    }
    panic!("权威工程里必须有音符 `{ulid}`");
}

/// 一个"人真的抓得到"的力度柱靶子（判据共用的装配段）。
struct VelocityTarget {
    ulid: String,
    /// 按下前的权威力度（> 0 ⇒ 柱高 > 0 ⇒ 可命中；零力度柱不在本臂的作用域内）。
    before: u8,
    /// 抓取点（**绘制帧**，即 `piano-roll-scroll-area` 的偏移）：柱内 1px。
    grab: (f32, f32),
    /// 车道顶沿在**卷帘局部坐标系**里的 y（= 车道内 y 的零点）。
    lane_top_local: f32,
}

/// 装配力度柱靶子：找第一个"有运行时力度柱节点、且柱完全落在视口内"的可见音符。
///
/// 顺带**交叉核对**判据自己的几何假设（车道高 / 柱高 / 柱顶沿）与界面真实包围盒。
fn velocity_target(
    window: &yeban_app::ui::MainWindow,
    plane: &mut LiveControlPlane,
    project: &yeban_model::YebanProjectV1,
    seq: i64,
    scroll: f32,
) -> VelocityTarget {
    let view = yeban_app::bridge::ViewState::from_project(project).expect("投影");
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Velocity.digit()),
        "夹具前提：力度臂只认力度工具（工具行 4）"
    );
    assert_eq!(
        window.get_roll_scroll_x(),
        scroll,
        "夹具前提：偏移与调用方给的一致"
    );
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let origin_x = bound_f32(&bounds, "x");
    let origin_y = bound_f32(&bounds, "y");
    let viewport_width = bound_f32(&bounds, "width");
    let viewport_height = bound_f32(&bounds, "height");

    // ---- 车道元素自证：高 44、顶沿落在 `H - 44`（判据的常数就是从这里核对的）----
    let lane = element_bounds(plane, seq + 1, "piano-roll-velocity-lane");
    let lane_height = bound_f32(&lane, "height");
    let lane_top_local = bound_f32(&lane, "y") - origin_y;
    assert!(
        (lane_height - VELOCITY_LANE_HEIGHT).abs() <= 1.0,
        "判据前提：力度车道高必须是 {VELOCITY_LANE_HEIGHT}px（实测 {lane_height}）"
    );
    assert!(
        (lane_top_local - (viewport_height - VELOCITY_LANE_HEIGHT)).abs() <= 1.0,
        "判据前提：车道顶沿必须落在卷帘底往上 {VELOCITY_LANE_HEIGHT}px（实测车道内 y0 = {lane_top_local}，卷帘高 {viewport_height}）"
    );

    // ---- 找靶：运行时树里真的有它的力度柱，且用**判据自己的**柱左沿能对上 ----
    for candidate in 0..view.notes.len() {
        let node_id = format!("velocity-{candidate}-bar");
        let Some(bar) = try_element_bounds(
            plane,
            seq + 2 + i64::try_from(candidate).expect("下标"),
            &node_id,
        ) else {
            continue;
        };
        let bar_left = bound_f32(&bar, "x") - origin_x;
        let bar_width = bound_f32(&bar, "width");
        let bar_height = bound_f32(&bar, "height");
        let bar_top_local = bound_f32(&bar, "y") - origin_y - lane_top_local;
        // 判据自己的柱左沿：`host_to_painted(note.x - scroll)` ＋ `.slint` 的 `+ 30px`。
        let index = view.notes.iter().position(|note| {
            let (painted_x, _) = host_to_painted(note.x - scroll, 0.0);
            (painted_x + VELOCITY_BAR_OFFSET_X - bar_left).abs() <= 1.0
        });
        let Some(index) = index else {
            panic!(
                "`{node_id}` 的柱左沿 {bar_left}（绘制帧）对不上任何音符（判据的 `.slint` 几何假设错了？）"
            );
        };
        let note = &view.notes[index];
        if note.velocity == 0 {
            continue;
        }
        if bar_left < 0.0 || bar_left + bar_width > viewport_width {
            continue;
        }
        // 交叉核对柱宽 / 柱高 / 柱顶沿：`6px`、`28px * v/127` 与 `40 - 28px * v/127`。
        assert!(
            (bar_width - VELOCITY_BAR_WIDTH).abs() <= 1.0,
            "判据前提：`{node_id}` 的柱宽必须是 {VELOCITY_BAR_WIDTH}px（实测 {bar_width}）"
        );
        let fraction = f32::from(note.velocity) / 127.0;
        let expected_height = VELOCITY_BAR_MAX_HEIGHT * fraction;
        let expected_top = (VELOCITY_LANE_HEIGHT - VELOCITY_LANE_BOTTOM_MARGIN) - expected_height;
        assert!(
            (bar_height - expected_height).abs() <= 1.0,
            "判据前提：`{node_id}`（力度 {}）的柱高必须是 {expected_height}px（实测 {bar_height}）—— \
             `.slint` 的 `28px * velocity` 或归一化口径变了",
            note.velocity
        );
        assert!(
            (bar_top_local - expected_top).abs() <= 1.0,
            "判据前提：`{node_id}`（力度 {}）的柱顶沿（车道内 y）必须是 {expected_top}（实测 {bar_top_local}）—— \
             `.slint` 的 `parent.height - 4px - 28px * velocity` 变了",
            note.velocity
        );
        // 抓取点：柱内 1px（柱高 > 0 已由 `velocity > 0` 保证）。
        let grab_lane_y = bar_top_local + 1.0;
        return VelocityTarget {
            ulid: note.id.clone(),
            before: note.velocity,
            grab: (bar_left + bar_width / 2.0, lane_top_local + grab_lane_y),
            lane_top_local,
        };
    }
    panic!("夹具里必须有一个可见、非零力度、柱完全落在视口内的音符");
}

/// 力度臂的一次完整手势：从 `target.grab`（**绘制帧**）竖直拖到**车道内** y = `target_lane_y`。
///
/// 返回注入的 `dy`（必须 > 4px，否则 `.slint` 会把它当**单击**而不是拖拽）与**判据算出的**
/// 期望力度。
fn drag_velocity_to_lane_y(
    plane: &mut LiveControlPlane,
    seq: i64,
    target: &VelocityTarget,
    target_lane_y: f32,
) -> (f32, u8) {
    let target_painted_y = target.lane_top_local + target_lane_y;
    let dy = target_painted_y - target.grab.1;
    assert!(
        dy.abs() > 4.0,
        "夹具前提：这一拖必须越过规范的 4px 阈值（否则 `.slint` 把它当单击，判据测不到拖拽）：dy = {dy}"
    );
    drag_in_roll_by_id_at(plane, seq, target.grab, 0.0, dy, DRAG_STEPS);
    (dy, expected_velocity_at_lane_y(target_lane_y))
}

/// 判据 (a)：力度工具把某根柱竖直拖到一条车道内 y ⇒ 权威工程的力度 = **判据自己按车道几何
/// 算出的绝对数**（不是"起点加增量"，也不是从被测代码读回来的数）。
#[test]
fn velocity_arm_a_a_velocity_drag_sets_the_absolute_velocity_from_the_lane_y() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let target = velocity_target(&window, &mut plane, &project_before, 1, 0.0);
    assert!(
        target.before > 0,
        "夹具前提：靶子的柱高必须 > 0（零力度柱不可命中）"
    );

    // ---- 独立期望值：把指针拖到"力度 32 的柱顶沿"这条 y 上 ----
    // 32 与 100 差得足够远（> 4px 阈值），且 `32/127` 远离 .5 ⇒ 四舍五入无歧义。
    let wanted = 32_u8;
    let target_lane_y = velocity_bar_top_lane_y(wanted);
    let (dy, expected) = {
        // 先自证：判据的"柱顶沿"反函数与"车道内 y → 力度"正函数互逆。
        assert_eq!(
            expected_velocity_at_lane_y(target_lane_y),
            wanted,
            "夹具自证：判据自己的两个几何函数必须互逆"
        );
        (
            target.lane_top_local + target_lane_y - target.grab.1,
            wanted,
        )
    };
    assert_ne!(expected, target.before, "夹具前提：这一拖必须改变力度");
    report_line(&format!(
        "[roll-velocity-a] 靶 `{}` 力度 {}，抓取点（绘制帧）({:.2},{:.2})，目标力度 {wanted} ⇒ \
         车道内 y = {target_lane_y:.3}（车道顶沿局部 y = {:.3}）；注入 dy = {dy:.3}px × {DRAG_STEPS} 步",
        target.ulid, target.before, target.grab.0, target.grab.1, target.lane_top_local
    ));

    drag_velocity_to_lane_y(&mut plane, 10, &target, target_lane_y);

    let after = velocity_of(&port.project(), &target.ulid);
    let (start_tick, pitch, duration) = note_by_ulid(&port.project(), &target.ulid);
    let (start_tick_before, pitch_before, duration_before) =
        note_by_ulid(&project_before, &target.ulid);
    report_line(&format!(
        "[roll-velocity-a] 权威工程力度 {} ⇒ {after}（期望 {expected}）；\
         tick/pitch/duration 必须一位不动：({start_tick_before},{pitch_before},{duration_before}) ⇒ \
         ({start_tick},{pitch},{duration})",
        target.before
    ));
    assert_eq!(
        after, expected,
        "拖到车道内 y = {target_lane_y:.3} 必须把力度**绝对**设成判据按车道几何算出的 {expected}"
    );
    assert_eq!(
        (start_tick, pitch, duration),
        (start_tick_before, pitch_before, duration_before),
        "力度臂**不得**改 tick / 音高 / 时值（那是移动臂与铅笔的事）"
    );
}

/// 判据 (a-2)：拖动期的**草稿**真的在窗口上、**按音符身份**键控，而工程**一位不动**。
///
/// ## 为什么值得单列一条
///
/// "拖动期只写 Slint 草稿、松手才提交"这条设计有两个可观察面：(a)/(c) 覆盖的是**提交**那一面
/// （松手恰好一个 op、一步撤销），本条覆盖**拖动期**那一面：
/// - 宿主必须在 `move` 时把草稿写进窗口（`note-drag-preview-velocity`）—— 否则界面上那根柱
///   在整个拖动过程里不会动，"拖到哪就是哪"只是松手之后才发生的跳变；
/// - 草稿必须**按身份**（`note-drag-ulid`）选中被拖的那一个音符 —— 兄弟臂把身份写在
///   `note-drag-note`（界面从不读它）⇒ 它的草稿预览**从来没有显示过**；本切片把身份统一到
///   界面真正比较的那个属性上，这条把它钉住；
/// - 拖动期工程必须**逐字节不变**、提交 **0** 次（草稿不是把工程改小步）。
///
/// ⛔ 它**不证明**像素：读的是窗口上的草稿属性，不是截图。柱高与草稿值的关系由
/// `piano_roll.slint` 的 `28px * velocity_reading` 表达（由 `bridge.rs` 的文本守卫对账）。
#[test]
fn velocity_arm_a2_the_draft_is_keyed_by_the_note_identity_and_writes_nothing() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let before_bytes = serde_json::to_vec(&project_before).expect("序列化起点工程");
    let commits_before = port.display().commit_count;
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let target = velocity_target(&window, &mut plane, &project_before, 1, 0.0);

    // ---- 按下：身份与草稿起点必须在窗口上 ----
    let origin = pointer_down_in_roll(&mut plane, 10, target.grab);
    assert_eq!(
        window.get_note_drag_ulid().to_string(),
        target.ulid,
        "按下之后被拖音符的身份必须在窗口上（界面按它选被拖的那一根柱；兄弟臂写在界面\
         从不读的 `note-drag-note` ⇒ 草稿预览从不显示）"
    );
    let draft_at_grab = window.get_note_drag_preview_velocity();
    let before_normalized = f32::from(target.before) / 127.0;
    assert!(
        (draft_at_grab - before_normalized).abs() < 1e-6,
        "按下的草稿起点必须是该音符**当前**力度（实测 {draft_at_grab}，期望 {before_normalized}）"
    );

    // ---- 拖到"力度 32"：草稿跟着走，工程一位不动 ----
    let target_lane_y = velocity_bar_top_lane_y(32);
    let dy = target.lane_top_local + target_lane_y - target.grab.1;
    pointer_drag_steps_in_roll(&mut plane, 20, origin, target.grab, 0.0, dy, DRAG_STEPS);
    let draft_mid = window.get_note_drag_preview_velocity();
    let mid_bytes = serde_json::to_vec(&port.project()).expect("序列化拖动中工程");
    let commits_mid = port.display().commit_count;
    let expected_normalized = 32.0_f32 / 127.0;
    report_line(&format!(
        "[roll-velocity-a2] 按下时身份 = `{}`、草稿 = {draft_at_grab:.6}；拖到车道内 y = \
         {target_lane_y:.3}（dy = {dy:.3}px）后草稿 = {draft_mid:.6}（期望 {expected_normalized:.6}）；\
         拖动期工程字节不变 = {}、提交数 {commits_before} ⇒ {commits_mid}、undoable = {}",
        window.get_note_drag_ulid(),
        mid_bytes == before_bytes,
        port.display().undoable
    ));
    assert!(
        (draft_mid - expected_normalized).abs() < 1e-6,
        "拖动期草稿必须等于 y 派生的**绝对**力度（实测 {draft_mid}，期望 {expected_normalized}）"
    );
    assert_eq!(
        mid_bytes, before_bytes,
        "拖动期工程必须**逐字节**不变（草稿只在 Slint 本地）"
    );
    assert_eq!(commits_mid, commits_before, "拖动期不得提交任何 op");
    assert_eq!(port.display().undoable, 0, "拖动期不得有可撤销的编辑");

    // ---- 松手：恰好一步，且手势读数清零（不留粘住的拖拽态）----
    pointer_up_in_roll(&mut plane, 40);
    assert_eq!(
        velocity_of(&port.project(), &target.ulid),
        32,
        "松手才把草稿落成**一个**原子 op"
    );
    assert_eq!(port.display().undoable, 1, "松手恰好一步撤销");
    assert!(
        !window.get_velocity_drag_active(),
        "松手之后手势必须不在手（`velocity-drag-active` 是唯一读数）"
    );
    assert_eq!(
        window.get_note_drag_ulid().to_string(),
        "",
        "松手之后身份必须清空（否则下一根柱会拿旧身份画草稿）"
    );
    assert!(
        window.get_note_drag_preview_velocity().abs() < 1e-6,
        "松手之后草稿必须清零"
    );
}

/// 判据 (b)：拖到车道上沿之外 ⇒ **恰好 127**；拖到下沿之下 ⇒ **恰好 0**（夹紧，⛔ 不越界）。
#[test]
fn velocity_arm_b_a_velocity_drag_clamps_to_127_and_0() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let target = velocity_target(&window, &mut plane, &project_before, 1, 0.0);

    // ---- 非空转前提：这两个 y 对应的**未夹紧**值真的在区间外 ----
    let raw_up = (VELOCITY_LANE_HEIGHT - VELOCITY_LANE_BOTTOM_MARGIN - 0.0)
        / VELOCITY_BAR_MAX_HEIGHT
        * 127.0;
    let raw_down = (VELOCITY_LANE_HEIGHT - VELOCITY_LANE_BOTTOM_MARGIN - 43.0)
        / VELOCITY_BAR_MAX_HEIGHT
        * 127.0;
    assert!(
        raw_up > 127.0,
        "夹具自证：上沿那个 y 的原始值必须 > 127（实测 {raw_up}）"
    );
    assert!(
        raw_down < 0.0,
        "夹具自证：下沿那个 y 的原始值必须 < 0（实测 {raw_down}）"
    );

    // ---- ① 拖到车道顶沿（车道内 y = 0）：原始 181.4 ⇒ 夹到 127 ----
    drag_velocity_to_lane_y(&mut plane, 10, &target, 0.0);
    let after_up = velocity_of(&port.project(), &target.ulid);
    // ⚠ 这条断言**紧跟在第一次手势之后**（而不是留到两次之后一起断）：未夹紧的实现会把
    //    181 交给模型 ⇒ 前置条件以 `VelocityOutOfRange` **拒绝** ⇒ 读数停在 100。
    //    放在这里，红就直接落在"夹到 127"这句话上，而不是落在一个夹具前提上。
    assert_eq!(
        after_up, 127,
        "拖到上沿之外必须夹到 **恰好** 127（⛔ 不得更大；未夹紧时模型会拒绝 181 ⇒ 停在上一个值）"
    );
    assert!(after_up <= 127, "力度不可能超过 127");

    // ---- ② 换个靶（同一个音符，柱现在顶到 127 ⇒ 顶沿 = 40 - 28 = 12），拖到 y = 43 ⇒ 0 ----
    let retarget = velocity_target(&window, &mut plane, &port.project(), 40, 0.0);
    assert_eq!(
        retarget.ulid, target.ulid,
        "夹具前提：① 之后还是同一个音符可拖"
    );
    assert_eq!(
        retarget.before, 127,
        "夹具前提：① 的结果必须已经写进投影（重投影生效）"
    );
    drag_velocity_to_lane_y(&mut plane, 60, &retarget, 43.0);
    let after_down = velocity_of(&port.project(), &target.ulid);

    report_line(&format!(
        "[roll-velocity-b] 抓取点车道内 y ≈ {:.2}；拖到 y = 0（原始 {raw_up:.2}）⇒ 力度 {after_up}；\
         再拖到 y = 43（原始 {raw_down:.2}）⇒ 力度 {after_down}",
        target.grab.1 - target.lane_top_local
    ));
    assert_eq!(
        after_down, 0,
        "拖到下沿之下必须夹到 **恰好** 0（⛔ 不得更小 / 不得回弹）"
    );
}

/// 判据 (c)：一次力度手势（`DRAG_STEPS` 步 move）= **恰好一步**撤销；第二次手势再 +1。
///
/// 这正是"草稿在手、松手才提交"设计的意义：⛔ 不是每个 `move` 提交一次。
#[test]
fn velocity_arm_c_exactly_one_history_entry_per_gesture() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let target = velocity_target(&window, &mut plane, &project_before, 1, 0.0);
    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");
    let commits_before = port.display().commit_count;

    // ---- 第一次手势：100 ⇒ 32 ----
    drag_velocity_to_lane_y(&mut plane, 10, &target, velocity_bar_top_lane_y(32));
    let undoable_first = port.display().undoable;
    let commits_first = port.display().commit_count;
    let after_first = velocity_of(&port.project(), &target.ulid);

    // ---- 第二次手势：从**工程读回的**新柱高重新取靶（顺带见证重投影没把画面落下）----
    let second = velocity_target(&window, &mut plane, &port.project(), 40, 0.0);
    assert_eq!(
        second.ulid, target.ulid,
        "夹具前提：第二次拖的还是同一个音符"
    );
    drag_velocity_to_lane_y(&mut plane, 60, &second, velocity_bar_top_lane_y(96));
    let undoable_second = port.display().undoable;
    let commits_second = port.display().commit_count;
    let after_second = velocity_of(&port.project(), &target.ulid);

    report_line(&format!(
        "[roll-velocity-c] 每次手势 {DRAG_STEPS} 步 move：撤销步数 0 ⇒ {undoable_first} ⇒ {undoable_second}；\
         提交数 {commits_before} ⇒ {commits_first} ⇒ {commits_second}；力度 {} ⇒ {after_first} ⇒ {after_second}",
        target.before
    ));
    assert_eq!(
        undoable_first, 1,
        "一次拖动 = **恰好一步**撤销（⛔ 不是每个 `move` 一次：本判据的 {DRAG_STEPS} 步里有 3 步\
         越过了 4px 阈值 ⇒ 那 3 步会各提交一次；变异实测 undoable = 3）"
    );
    assert_eq!(
        commits_first - commits_before,
        1,
        "一次拖动恰好**一次**提交"
    );
    assert_ne!(
        after_first, target.before,
        "非空转前提：第一次手势必须真的改了力度（否则上面的计数可能只是\"什么都没发生\"）"
    );
    assert_eq!(after_first, 32, "第一次手势的落点由判据独立算出");
    assert_eq!(after_second, 96, "第二次手势的落点由判据独立算出");
    assert_eq!(undoable_second, 2, "第二次手势再 +1（不是一次性的效果）");
    assert_eq!(commits_second - commits_before, 2, "两次手势恰好两次提交");
}

/// 判据 (d)：`Escape` 插在力度拖动中途 ⇒ 工程**逐字节不变**、提交 **0** 次。
///
/// ⚠ **本判据在"力度臂未接线"的代码上会空转绿**（力度工具本来就不写工程）—— 这正是兄弟
/// 判据 `move_arm_c_...` 踩过的坑（它自己在文档里如实登记了）。因此本判据的**红**不是来自
/// "未修复的代码"，而是来自一个**变异**：把 `Action::Cancel` 里的
/// `cancel_velocity_drag(ui)` 那一句删掉 ⇒ 松手时宿主的手势标志还在手，
/// `note-drag-release` 会把最后一次 `move` 的位置提交成一步 ⇒ 工程变、`undoable` 变 1，
/// 三条断言同时红（实测读数见交付报告）。它证明的是**这条收尾路径真的在链上**，
/// ⛔ 不证明"取消在任何夹具下都能还原"（草稿从未进过工程，见判据 (c) 的提交计数）。
#[test]
fn velocity_arm_d_escape_mid_drag_leaves_the_project_byte_identical_and_commits_nothing() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    let before_bytes = serde_json::to_vec(&project_before).expect("序列化起点工程");
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let target = velocity_target(&window, &mut plane, &project_before, 1, 0.0);
    let commits_before = port.display().commit_count;

    let element_origin = pointer_down_in_roll(&mut plane, 10, target.grab);
    // 拖到"力度 32"那条 y（> 4px 阈值 ⇒ `.slint` 认它是拖拽，抬起会 fire `release`）。
    let target_lane_y = velocity_bar_top_lane_y(32);
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        element_origin,
        target.grab,
        0.0,
        target.lane_top_local + target_lane_y - target.grab.1,
        DRAG_STEPS,
    );
    // 拖到一半：`Escape`（`Action::Cancel` 的唯一键盘入口）⇒ 丢弃草稿、⛔ 不提交。
    press_escape(&mut plane, 30);
    pointer_up_in_roll(&mut plane, 40);

    let project_after = port.project();
    let after_bytes = serde_json::to_vec(&project_after).expect("序列化终点工程");
    let commits_after = port.display().commit_count;
    report_line(&format!(
        "[roll-velocity-d] 拖动中途 `Escape` 后松手：工程字节 {} ⇒ {}（逐字节相同 = {}）；\
         提交数 {} ⇒ {}；undoable={}；力度 {} ⇒ {}",
        before_bytes.len(),
        after_bytes.len(),
        before_bytes == after_bytes,
        commits_before,
        commits_after,
        port.display().undoable,
        target.before,
        velocity_of(&project_after, &target.ulid)
    ));
    assert_eq!(
        before_bytes, after_bytes,
        "`Escape` 取消后工程必须**逐字节**不变（草稿从未写进工程，收尾后也不许补写）"
    );
    assert_eq!(
        commits_after, commits_before,
        "`Escape` 取消不得提交任何 op"
    );
    assert_eq!(port.display().undoable, 0, "取消之后不得有可撤销的编辑");
}

/// 去掉 `//` 注释（含行尾注释）之后的**代码文本**（与 `tests/tool_matrix_wiring.rs` 的
/// `code_only` 同一口径）。
///
/// ⚠ 它按**字符串**切 `//` ⇒ 字符串字面量里的 `//` 也会被切掉。本判据只数
/// `ToolDrag::AdjustVelocity` 这种标识符，被测源文件里没有含 `//` 的字面量。
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _comment)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 判据 (e)：**结构防漂移**。计数规则（逐条写明）：
///
/// ① 单位 = `str::matches(needle).count()` 的**子串出现次数**（⛔ 不是行数、⛔ 不是"消费者个数"）；
/// ② **先去掉 `//` 注释**（含 `///` 文档注释）再数 —— 兄弟臂的判据自曝"它的计数大部分是
///    文档注释"；本判据的针是 `ToolDrag::AdjustVelocity`，去注释后**文档里的引用一律不算**；
/// ③ 目录 = `<crate>/src` **递归**，只收 `*.rs`（`CARGO_MANIFEST_DIR` 起算）；
/// ④ `crates/yeban-app/tests/**` **不在**扫描范围（本函数只走 `src`）⇒ 判据自己写的字面
///    **不算**；
/// ⑤ 每个文件只取**第一个 `#[cfg(test)]` 之前**的文本（该文件的生产区）⇒ `src/**` 内联
///    `mod tests` 里的字面不计；
/// ⑥ `src/input.rs` **整个排除**（那是变体自己的定义处，必然出现）。
///
/// 另加一条**不成对的**检查（用**原始**文本，注释**不**去掉）：`input.rs` 的 `ToolDrag`
/// 五个变体里，每一个的文档必须逐字包含「已接线」或「未消费」，且**恰好两**个说已接线 ——
/// 这条钉的是"消费现状表不许漂"（把未接的写成已接会立刻红）。
///
/// **它证明什么**：去注释后 `ToolDrag::AdjustVelocity` 在 `src/input.rs` 之外有一个**代码**
/// 出现（本切片之前 = 0）——移动臂的判据数的是 `ToolDrag` 这个宽针，本判据数的是**这一格**。
/// **它不证明什么**：可达性 —— 一个 `if false { ToolDrag::AdjustVelocity }` 也能让它绿；
/// 真正"拖得动"由 (a)–(d) 用真实指针注入管。也**不证明**其余三个变体没被误接。
#[test]
fn velocity_arm_e_the_velocity_drag_variant_gains_a_commented_out_consumer() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);
    files.sort();
    assert!(
        files
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == "input.rs")),
        "走查自证：`src/input.rs` 必须在扫描集里"
    );

    let needle = "ToolDrag::AdjustVelocity";
    let mut occurrences = 0_usize;
    let mut hits: Vec<String> = Vec::new();
    for path in &files {
        if path.file_name().is_some_and(|name| name == "input.rs") {
            continue;
        }
        let text = std::fs::read_to_string(path).expect("读源文件");
        let end = text.find("#[cfg(test)]").unwrap_or(text.len());
        let production = code_only(&text[..end]);
        let count = production.matches(needle).count();
        if count > 0 {
            hits.push(format!(
                "{}×{}",
                path.file_name().expect("文件名").to_string_lossy(),
                count
            ));
            occurrences += count;
        }
    }
    report_line(&format!(
        "[roll-velocity-e] 去注释后 `{needle}` 在 `src/input.rs` **之外**的生产区出现 {occurrences} 次\
         （本切片之前 0）：{hits:?}；扫描 {} 个 `.rs` 文件",
        files.len()
    ));
    assert!(
        occurrences >= 1,
        "去注释后字面 `{needle}` 必须在 `crates/yeban-app/src` 的生产区（`input.rs` 之外）出现 ≥1 次\
         （本切片之前 = 0）"
    );

    // ---- 消费现状表：每个变体都必须自报「已接线 / 未消费」，且恰好两个说已接线 ----
    let input = std::fs::read_to_string(root.join("input.rs")).expect("读 input.rs");
    let enum_start = input
        .find("pub enum ToolDrag {")
        .expect("`input.rs` 必须有 `pub enum ToolDrag {`");
    let body = &input[enum_start..];
    let variants = [
        "MoveNote",
        "ResizeDuration",
        "SliceAcross",
        "AdjustVelocity",
        "EraseSweep",
    ];
    let mut consumed = 0_usize;
    let mut previous = 0_usize;
    for name in variants {
        let declaration = format!("\n    {name},");
        let at = body
            .find(&declaration)
            .unwrap_or_else(|| panic!("`ToolDrag` 里找不到变体 `{name}`"));
        assert!(at > previous, "变体 `{name}` 的声明必须按枚举顺序出现");
        let doc = &body[previous..at];
        let says_consumed = doc.contains("已接线");
        let says_unconsumed = doc.contains("未消费");
        assert!(
            says_consumed ^ says_unconsumed,
            "变体 `{name}` 的文档必须**恰好**说一种（已接线 / 未消费），实际[已接线]={says_consumed} \
             [未消费]={says_unconsumed}；两边都不说或都说会让人读不出它的接线现状"
        );
        if says_consumed {
            consumed += 1;
        }
        previous = at;
    }
    assert_eq!(
        consumed, 2,
        "消费现状表必须**恰好两个**变体说已接线（移动臂 + 力度臂）—— 把未接的写成已接会在这里红"
    );
}

/// 判据 (f)：选择工具下**空白处**拖动超过 4px **不得**清空选区（**回归钉**）。
///
/// ## 这是什么行为、为什么它在本切片之前的代码上**不能**变红
///
/// `.slint` 的 `up` 原先**无条件** fire `clicked` ⇒ 空白处一次滚动拖拽在松手时被当成
/// 单击 ⇒ 派发器按"点空白 = 清除选区"把选区清掉。兄弟臂（`ac2c72f1`）把它改成按规范 §6 的
/// **4px 阈值分流**：越阈值的抬起是**拖拽**（不发 `clicked`），没越的才是单击。
///
/// ⇒ 本判据是**兄弟臂行为改变的回归钉**，不是本切片修的东西：在**现行**代码上它绿，
/// 在"把 `up` 改回无条件 `clicked`"的**变异**上它红（实测读数见交付报告）。
/// 兄弟臂在交付时**披露了这个行为改变、但没有加断言** —— 这条把那个缺口补上。
///
/// **它证明什么**：空白处 > 4px 的拖动不会经由 `clicked` 清空选区，而且这一拖真的滚动了
/// （非空转）。
/// **它不证明什么**：空白处的 marquee / 框选语义（仍未裁决，本切片不实现）；
/// 也不证明"单击空白仍然清空选区"（那是另一条判据的事，本文件不重复）。
#[test]
fn a_blank_drag_longer_than_the_threshold_does_not_clear_the_selection() {
    let (window, port, mut plane) = assemble();
    use slint::Model as _;
    let project = port.project();
    let view = yeban_app::bridge::ViewState::from_project(&project).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bound_f32(&bounds, "width");
    let viewport_height = bound_f32(&bounds, "height");
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：本判据走默认选择工具"
    );

    // ---- 前置：先用一次**真单击**选中一个音符（没越阈值 ⇒ 抬起算单击）----
    let index = a_painted_note(&view, viewport_width, viewport_height, 0.0);
    let expected_ulid = view.notes[index].id.clone();
    let (painted_x, painted_y) = note_center_in_painted_view(&view, index, 0.0);
    click_in_roll_by_id_at(&mut plane, 10, painted_x, painted_y);
    assert_eq!(
        window.get_selected_note_count(),
        1,
        "前置：单击必须选中一个音符"
    );

    // ---- 空白处拖动 > 4px（左拖 40px ⇒ 滚动）----
    let (blank_x, blank_y) = a_blank_point(&view, viewport_width, viewport_height, 0.0);
    let (blank_painted_x, blank_painted_y) = host_to_painted(blank_x, blank_y);
    let scroll_before = window.get_roll_scroll_x();
    drag_in_roll_by_id_at(
        &mut plane,
        20,
        (blank_painted_x, blank_painted_y),
        -40.0,
        0.0,
        DRAG_STEPS,
    );
    let scroll_after = window.get_roll_scroll_x();
    let count = window.get_selected_note_count();
    let selected: Vec<String> = window
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    report_line(&format!(
        "[roll-velocity-f] 先单击选中 `{expected_ulid}`；再在空白处（宿主帧 {blank_x},{blank_y}）\
         左拖 40px × {DRAG_STEPS} 步：`roll-scroll-x` {scroll_before} ⇒ {scroll_after}；\
         selected-note-count = {count}，selected-ulids = {selected:?}"
    ));
    assert!(
        (scroll_after - scroll_before).abs() > 0.5,
        "非空转前提：空白处拖动必须真的滚动（否则这一拖可能根本没被送达）"
    );
    assert_eq!(
        count, 1,
        "空白处 > 4px 的拖动**不得**把抬起当单击 ⇒ 选区必须还在（旧行为：`up` 无条件 fire `clicked` ⇒ 0）"
    );
    assert!(
        selected.iter().any(|id| id == &expected_ulid),
        "保住的必须还是原来那个身份 `{expected_ulid}`"
    );
}

// ===========================================================================
// `[UI-NOTE-003]` §3.3 力度工具 aux 列：**双击恢复默认力度 100**
// ===========================================================================
//
// ## 它补的是哪一个缺口（两条都是机械读数，不是转述）
//
// 规范 §3.3 的力度行写着「双击恢复默认力度 100」
// （`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:200`），而本切片之前
// **`grep -rn "double-clicked" crates/yeban-app/ui/ crates/yeban-app/src/` = 0** ——
// 不是"Slint 不提供"（`TouchArea` 确实暴露 `double-clicked`，
// `i-slint-compiler-1.18.1/builtin_elements.rs`），是**没接**。
//
// 为什么这不止是"少一个快捷键"：一颗 **0 力度**音符的柱高是 `28px * 0 = 0px`
// （`ui/console/piano_roll.slint` 的 `height: 28px * velocity_reading`）⇒ 屏幕上**不可抓取**，
// 而力度臂的命中函数 `host::velocity_bar_at` 也**如实拒绝**它
// （`(view.notes[index].velocity > 0).then_some(index)`）⇒ 0 力度音符在**拖拽**这条路上
// **不可达**。双击是规范指定的恢复路径 ⇒ 它正是为"拖拽够不着的那一部分"存在的。
//
// ## 手势范围：**音符**，不是力度柱（这是决定，理由在下一行）
//
// 双击走的是**单击那一个** `view.hit_test_visible(...)` 命中（音符矩形），⛔ 不是力度柱命中。
// 理由只有一个：最需要恢复的那颗音符力度为 0 ⇒ 柱高 0px ⇒ **柱范围的手势永远够不着它**。
// ⛔ 本切片不给柱体加最小高度、⛔ 不放宽 `velocity_bar_at` 的 `velocity > 0` 门 ——
// 那些都会发明新的 affordance；双击覆盖它够不着的那一部分。
//
// ## 判据各自证明什么 / 不证明什么
//
// | 判据 | 证明 | ⛔ 不证明 |
// | :--- | :--- | :--- |
// | (a) `velocity_double_click_a_...` | 0 力度音符上 `down/up × 2` ⇒ 权威力度 **100**，且 tick / pitch / duration 一位不动 | 像素（没有截图）；"人手也是这样"（注入的是控制面指针，不是真人鼠标） |
// | (b) `velocity_double_click_b_...` | 一次双击 = **恰好一个**提交 / **恰好一步**撤销；已是 100 的第二次双击 = **0** 个 op | op 的**类型**是 `ModifyNoteVelocity`（判据读的是力度结果；构造点在 `host::velocity_op_for`，由源码读） |
// | (c1) `velocity_double_click_c1_...` | 力度工具**单击**不改力度、不提交 | 规范里"单击选中力度柱"的语义（仍未接线，如实不做） |
// | (c2) `velocity_double_click_c2_...` | 工具门真的在：**选择工具**双击不改力度 | 选择工具双击的其它语义（"创建默认音符"仍未接线） |
//
// ⚠ **(c1)/(c2) 在修复前的代码上也是绿的** —— 它们断言"不变"，而旧代码正好也不变。
// 这是控制臂的**性质**（它们只能排除假阳性），不是本切片能改的；能力那一半由 (a)/(b) 管。
//
// ## 怎么变红（**行为**红，不是编译红）
//
// 把 `host.rs` 的 `ToolClick::SelectVelocity` 臂改回 `{}`（= 本切片之前那行
// `SplitNote | SelectVelocity => {}`）⇒ (a) 的力度停在 **0**、(b) 的提交增量 = **0**；
// (c1)/(c2) 仍然绿（见上面那条 ⚠）。

/// 判据夹具：靶子音符的身份解析 —— ULID ⇒ `(clip_id, track_id, 当前力度)`。
///
/// 口径与 `host::note_identity_for` 的**文档**一致（音符住片段池；`track_id` 取摆放了该
/// 片段的第一条轨道；未摆放则回退第一条轨道），但这是**判据自己的**一份遍历
/// （⛔ 不调 `host::velocity_op_for` / `note_identity_for`）⇒ 期望值不来自被测代码。
fn identity_by_ulid(
    project: &yeban_model::YebanProjectV1,
    ulid: &str,
) -> (yeban_model::EntityId, yeban_model::EntityId, u8) {
    use std::collections::BTreeMap;
    let mut owner: BTreeMap<yeban_model::EntityId, yeban_model::EntityId> = BTreeMap::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            owner.entry(placement.clip_id).or_insert(*track_id);
        }
    }
    let fallback = project
        .tracks
        .keys()
        .next()
        .copied()
        .expect("夹具工程必须有轨道");
    for (clip_id, entry) in &project.clip_pool {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for (id, note) in notes {
            if id.to_canonical_string() == ulid {
                let track_id = owner.get(clip_id).copied().unwrap_or(fallback);
                return (*clip_id, track_id, note.velocity);
            }
        }
    }
    panic!("权威工程里必须有身份 `{ulid}`");
}

/// 判据夹具：把一个音符的权威力度**经 `Op::ModifyNoteVelocity`** 写成 0。
///
/// 为什么需要它：`filled_project()` 的每颗音符都是 `music::DEFAULT_VELOCITY`（100）
/// ⇒ 夹具里没有"柱高 0px、拖拽臂够不着"的靶子，判据必须自己造一个。造法走的是
/// **产品同一条 op 通道**（[`UndoPort::commit_ops`] ＋ 模型自己的前置条件），
/// 因此这不是"往模型里塞私货"。
fn zero_velocity_via_authority(port: &Rc<UndoPort>, ulid: &str) {
    let (clip_id, track_id, old_vel) = identity_by_ulid(&port.project(), ulid);
    assert!(
        old_vel > 0,
        "夹具自证：起点力度必须 > 0（否则这一写什么都没改，靶子还没造出来）"
    );
    let op = yeban_model::ops::Op::ModifyNoteVelocity {
        track_id,
        clip_id,
        note_id: ulid.parse::<yeban_model::EntityId>().expect("ULID"),
        old_vel,
        new_vel: 0,
    };
    port.commit_ops(NOW_MS, "fixture: zero the velocity", vec![op])
        .expect("把靶子降到 0 力度必须被权威接受");
    assert_eq!(
        velocity_of(&port.project(), ulid),
        0,
        "夹具：靶子力度必须已经是 0（此时柱高 0px，`velocity_bar_at` 拒绝它）"
    );
}

/// 判据夹具：在**同一个绘制点**上注入 `down → up → down → up`（两次单击）。
///
/// 为什么两对按下 / 抬起**就是**一次双击（⇒ ⛔ 不需要新 MCP 方法）：
/// - Slint 在**释放**时按 `click_count % 2 == 1` 触发 `double-clicked`
///   （`i-slint-core-1.18.1/items/input_items.rs`）；
/// - `ClickState::check_repeat` 只在**两次按下**间隔 < `click_interval`（默认 500ms）且
///   按下点位移 < 10 逻辑像素时递增 `click_count`（`i-slint-core-1.18.1/input.rs`）。
///   注入的两对相隔微秒、位移 0px（同一个 `from`）⇒ 必然触发。
fn double_click_in_roll(plane: &mut LiveControlPlane, seq: i64, from: (f32, f32)) {
    let _first_origin = pointer_down_in_roll(plane, seq, from);
    pointer_up_in_roll(plane, seq + 2);
    let _second_origin = pointer_down_in_roll(plane, seq + 3, from);
    pointer_up_in_roll(plane, seq + 5);
}

/// 判据夹具：找一个"人真的点得到"的可见音符 ⇒ `(ULID, 绘制帧中心)`。
fn painted_target_note(
    plane: &mut LiveControlPlane,
    project: &yeban_model::YebanProjectV1,
    seq: i64,
) -> (String, (f32, f32)) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let viewport_width = bound_f32(&bounds, "width");
    let viewport_height = bound_f32(&bounds, "height");
    let view = yeban_app::bridge::ViewState::from_project(project).expect("投影");
    let index = a_painted_note(&view, viewport_width, viewport_height, 0.0);
    (
        view.notes[index].id.clone(),
        note_center_in_painted_view(&view, index, 0.0),
    )
}

/// 判据 (a)：**力度工具**在一颗 **0 力度**音符上双击 ⇒ 权威力度 = 默认 **100**，
/// 且 tick / pitch / duration 一位不动。
///
/// 为什么靶子必须是 0 力度：那颗音符的柱高是 0px、`velocity_bar_at` 也拒绝它
/// ⇒ 拖拽臂**够不着**；双击是规范给它的恢复路径。100 力度音符上的双击由 (b) 的第二拍管。
#[test]
fn velocity_double_click_a_restores_default_velocity_on_a_zero_velocity_note() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let (ulid, point) = painted_target_note(&mut plane, &project_before, 1);
    let geometry_before = note_by_ulid(&project_before, &ulid);

    // ---- 前置：把它降到 0（柱高 0px ⇒ 力度臂的 `velocity_bar_at` 如实拒绝它）----
    zero_velocity_via_authority(&port, &ulid);
    let commits_before = port.display().commit_count;

    double_click_in_roll(&mut plane, 10, point);

    let after = velocity_of(&port.project(), &ulid);
    let geometry_after = note_by_ulid(&port.project(), &ulid);
    report_line(&format!(
        "[roll-velocity-dc-a] 靶 `{ulid}`（绘制帧中心 {:.2},{:.2}、权威力度 0）双击后力度 = {after}；\
         提交数 {commits_before} ⇒ {}；tick/pitch/duration {geometry_before:?} ⇒ {geometry_after:?}",
        point.0,
        point.1,
        port.display().commit_count
    ));
    assert_eq!(
        after, 100,
        "力度工具在**0 力度音符**上双击必须把权威力度恢复成默认 100 —— 本切片之前这条路径\
         不存在（柱高 0px ⇒ 拖不动），力度会停在 0"
    );
    assert_eq!(
        geometry_after, geometry_before,
        "双击**只**改力度：tick / pitch / duration 必须一位不动"
    );
}

/// 判据 (b)：一次双击 = **恰好一个**原子 op（一次提交 / 一步撤销），
/// 而**已经是 100** 的音符上再双击 = **0** 个 op（⛔ 不写空编辑）。
#[test]
fn velocity_double_click_b_is_exactly_one_undo_step_and_idempotent() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let (ulid, point) = painted_target_note(&mut plane, &project_before, 1);
    zero_velocity_via_authority(&port, &ulid);

    let commits_before = port.display().commit_count;
    let undoable_before = port.display().undoable;

    double_click_in_roll(&mut plane, 10, point);
    let velocity_first = velocity_of(&port.project(), &ulid);
    let commits_first = port.display().commit_count;
    let undoable_first = port.display().undoable;

    double_click_in_roll(&mut plane, 30, point);
    let velocity_second = velocity_of(&port.project(), &ulid);
    let commits_second = port.display().commit_count;
    let undoable_second = port.display().undoable;

    report_line(&format!(
        "[roll-velocity-dc-b] 靶 `{ulid}`：力度 0 ⇒ {velocity_first} ⇒ {velocity_second}；\
         提交数 {commits_before} ⇒ {commits_first} ⇒ {commits_second}；\
         可撤销步数 {undoable_before} ⇒ {undoable_first} ⇒ {undoable_second}"
    ));
    assert_eq!(velocity_first, 100, "第一次双击必须恢复 100");
    assert_eq!(
        commits_first - commits_before,
        1,
        "一次双击 = **恰好一个**原子 op（⛔ 不是两个：那会让撤销要按两次）"
    );
    assert_eq!(
        undoable_first - undoable_before,
        1,
        "一次双击 = **恰好一步**撤销"
    );
    assert_eq!(velocity_second, 100, "第二次双击时力度已经是 100");
    assert_eq!(
        commits_second, commits_first,
        "已经是 100 的双击必须**一个 op 都不写** —— 模型**不会**拒绝空 op，\
         所以'不提交'必须在宿主那一侧短路"
    );
    assert_eq!(
        undoable_second, undoable_first,
        "已经是 100 的双击不得在撤销栈上留一步空编辑"
    );
}

/// 判据 (c1) **控制臂**：力度工具的**单击**不得改力度、不得提交。
///
/// ⚠ 它在**修复前的代码上也是绿的**（旧代码什么都不做）—— 控制臂证明的是"没有假阳性"，
/// 不是"能力存在"。能力由 (a)/(b) 管。
#[test]
fn velocity_double_click_c1_a_single_velocity_click_leaves_velocity_unchanged() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    window.set_active_tool(i32::from(Tool::Velocity.digit()));
    let (ulid, point) = painted_target_note(&mut plane, &project_before, 1);
    zero_velocity_via_authority(&port, &ulid);
    let commits_before = port.display().commit_count;

    // **单击**：一对 `down/up`（⛔ 不是两对）。
    let _origin = pointer_down_in_roll(&mut plane, 10, point);
    pointer_up_in_roll(&mut plane, 12);

    let after = velocity_of(&port.project(), &ulid);
    report_line(&format!(
        "[roll-velocity-dc-c1] 力度工具**单击**靶 `{ulid}`（力度 0）：力度 = {after}；\
         提交数 {commits_before} ⇒ {}",
        port.display().commit_count
    ));
    assert_eq!(
        after, 0,
        "力度工具的**单击**语义（选中力度柱）仍未接线 ⇒ 单击必须**如实什么都不做**\
         （⛔ 不能顺手恢复成 100 —— 那会把'双击'变成'单击也能改'）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "单击不得提交任何 op"
    );
}

/// 判据 (c2) **工具门控制臂**：**选择工具**下双击不得改力度。
///
/// ⚠ 与 (c1) 同款：它在**修复前的代码上也是绿的** —— 控制臂只排除假阳性。
#[test]
fn velocity_double_click_c2_a_select_tool_double_click_leaves_velocity_unchanged() {
    let (window, port, mut plane) = assemble();
    let project_before = port.project();
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：默认工具就是选择箭头（本判据不设工具 ⇒ 走默认）"
    );
    let (ulid, point) = painted_target_note(&mut plane, &project_before, 1);
    zero_velocity_via_authority(&port, &ulid);
    let commits_before = port.display().commit_count;

    double_click_in_roll(&mut plane, 10, point);

    let after = velocity_of(&port.project(), &ulid);
    report_line(&format!(
        "[roll-velocity-dc-c2] **选择工具**双击靶 `{ulid}`（力度 0）：力度 = {after}；\
         提交数 {commits_before} ⇒ {}",
        port.display().commit_count
    ));
    assert_eq!(
        after, 0,
        "双击恢复力度必须**只在力度工具下**发生（工具门）—— 选择工具双击不改力度"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "选择工具双击不得提交任何 op"
    );
}
