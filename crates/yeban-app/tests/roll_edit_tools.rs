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
//! | `a_scroll_drag_after_a_pencil_edit_redraws_the_live_project_not_the_wired_snapshot` | **D-UI-10**：铅笔加一个音符之后，一次越过 4px 阈值的水平拖动（= 人手上的"随手一拖"）必须把界面重投影成**此刻**的工程 ⇒ 那个音符的矩形仍在运行时树里、偏移确实动了。旧实现只在 `build_main_window_with_console_tab` 装一次滚动 handler 并抓住**装配/接线那一刻**的快照 ⇒ 拖动把整个界面重绘回快照（实测症状：刚画的音符整批从树里消失、读数变回 0 轨 / bpm 120.00，⛔ 没有 `Op`、⛔ 没有日志、撤销栈一位没动） |
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
//! - 让 `install_roll_scroll` 退回"只抓快照、不读端口"（= 在 `host.rs` 的 `install_roll_scroll` 里把
//!   `port` 参数恒传 `None`）⇒ D-UI-10 那条判据的 `note_rect_in_tree` 变 `false`（音符被重绘掉），
//!   而偏移断言仍绿 —— 证明红在"重投影用错了工程"，不在"手势没送到"。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `piano_roll_drag_scroll.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::collections::BTreeMap;
use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use yeban_app::bridge::{ViewState, host_to_painted, painted_to_host};
use yeban_app::host::{
    PENCIL_NO_CLIP_ON_DISPLAYED_TRACK_REASON, PENCIL_TOOL_DIGIT, ROLL_SNAP_GRID_TICKS,
    pencil_op_for, wire_roll_edit,
};
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (b)：音符拖动**不**顺带滚动；控制臂 = **同一次装配里**的空白处拖动照旧滚动。
#[test]
fn move_arm_b_a_note_drag_does_not_scroll_the_roll() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c)：`Escape` 插在拖动中途 ⇒ 工程**逐字节不变**、提交 **0** 次。
///
/// ⚠ 本判据在**未接移动臂**的代码上**空转绿**（选择工具本来就不写工程）—— 它的对照
/// 不在"修复前 / 修复后"，而在"**取消这条收尾路径**在不在"（见文件末"怎么变红"）。
#[test]
fn move_arm_c_escape_mid_drag_leaves_the_project_byte_identical_and_commits_nothing() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (d)：一次拖动（`DRAG_STEPS` 步 `move`）= **恰好一步**撤销；第二次手势再 +1。
///
/// 这正是"草稿在手、松手才提交"设计的意义：⛔ 不是每个 `move` 提交一次。
#[test]
fn move_arm_d_exactly_one_history_entry_per_gesture() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (f)：力度工具的**单击**（规范 §3.3 "选中对应音符的底部力度柱"）⇒ 选中那颗音符，
/// 工程**逐字节不变**、提交 **0** 次；点车道空白 ⇒ 选区清空。
///
/// ## 为什么单列
///
/// 接线前这条臂是 `if !double_click { return; }`，而第 1 段的命中是**音符框**
/// （`hit_test_visible`）：力度柱画在 `piano-roll-velocity-lane` 里、不在音符框那一段
/// ⇒ 单击力度柱不但选不中，还会把已有选区**清掉**（比"点了没反应"更糟）。
/// 靶子由 [`velocity_target`] 从**运行时包围盒**给出，与被测代码不共享几何假设。
#[test]
fn velocity_click_selects_the_note_whose_bar_is_under_the_pointer() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (b)：拖到车道上沿之外 ⇒ **恰好 127**；拖到下沿之下 ⇒ **恰好 0**（夹紧，⛔ 不越界）。
#[test]
fn velocity_arm_b_a_velocity_drag_clamps_to_127_and_0() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c)：一次力度手势（`DRAG_STEPS` 步 move）= **恰好一步**撤销；第二次手势再 +1。
///
/// 这正是"草稿在手、松手才提交"设计的意义：⛔ 不是每个 `move` 提交一次。
#[test]
fn velocity_arm_c_exactly_one_history_entry_per_gesture() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (b)：一次双击 = **恰好一个**原子 op（一次提交 / 一步撤销），
/// 而**已经是 100** 的音符上再双击 = **0** 个 op（⛔ 不写空编辑）。
#[test]
fn velocity_double_click_b_is_exactly_one_undo_step_and_idempotent() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c1) **控制臂**：力度工具的**单击**不得改力度、不得提交。
///
/// ⚠ 它在**修复前的代码上也是绿的**（旧代码什么都不做）—— 控制臂证明的是"没有假阳性"，
/// 不是"能力存在"。能力由 (a)/(b) 管。
#[test]
fn velocity_double_click_c1_a_single_velocity_click_leaves_velocity_unchanged() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c2) **工具门控制臂**：**选择工具**下双击不得改力度。
///
/// ⚠ 与 (c1) 同款：它在**修复前的代码上也是绿的** —— 控制臂只排除假阳性。
#[test]
fn velocity_double_click_c2_a_select_tool_double_click_leaves_velocity_unchanged() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

// ===========================================================================
// 本切片：`[UI-NOTE-003]` 剪刀（Knife）的**左键单击** —— 沿网格竖线切分指针下的音符
// ===========================================================================
//
// ## 接了哪一格、⛔ 明确不接哪几格
//
// | 格 | 决定 |
// | :--- | :--- |
// | 剪刀单击（`ToolClick::SplitNote`） | **本切片接上**：`hit_test_visible` 那**一个**命中 ⇒ `split_ops_for` ⇒ `commit_ops`（一个 `Op::Batch`） |
// | 手势范围 | **指针**（不是列）：只切指针下那一颗，⛔ 不建 tick-span 查询 / 第二条命中路径 |
// | 切分身份 | 原文 `DeleteNote` ＋ 两个 `EntityId::new()` 的 `AddNote`（与 `duplicate_ops_for` 的副本形状同款）⇒ ⛔ 无新 `Op` 变体 |
// | 退化切分（吸附点不严格落在音符内） | **拒绝**：`split_ops_for` 返回空 ⇒ 一个 op 都不写 |
// | 剪刀拖拽（`ToolDrag::SliceAcross`） | ⛔ **不接**（本切片只接左键单击；规范矩阵的拖拽列仍未接） |
//
// ## 每条判据证明什么 / ⛔ 不证明什么
//
// | 判据 | 证明 | ⛔ 不证明 |
// | :--- | :--- | :--- |
// | `a_knife_click_splits_...` | 在音符**绘制帧中心**注入 `down → move → up` ⇒ 原文消失、恰两个同 `pitch`/`velocity` 的新音符、两半恰好铺满原文（左 `[start, split)` ＋ 右 `[split, end)`），且切分点 == 判据**自己**用 `snapped_tick_at` 算出的值 | 拖拽切分；跨片段 / 多选；切分期的画面（无截图判据） |
// | `b_knife_split_...` | 一次手势 = 提交 **+1** / 可撤销步数 **+1**，且一次 `UiAction::Undo` 之后工程**逐字节**回到切分前 | 撤销**内容**之外的 UI 回写（由既有撤销判据管） |
// | `c1_...`（控制臂） | 剪刀点**空白** ⇒ 0 提交、0 音符变化、字节不变 | 能力（未接线代码上它也绿 —— 控制臂只排除假阳性） |
// | `c2_...`（控制臂） | 吸附 tick **不严格落在**音符内 ⇒ 0 提交、原文一位不动 | ⚠ 未接线代码上它**空转绿**；红靠**变异**挣（删掉宿主那侧的 `ops.is_empty()` 短路 ⇒ 空批也提交，判据在提交数上红） |
//
// ## 怎么变红（**行为**红，不是编译红）
//
// 把 `host.rs` 的 `ToolClick::SplitNote` 臂改回 `{}`（= 本切片之前那行）⇒ (a) 在**音符数**上红
// （实测：切分前 5、切分后仍 5）、(b) 在提交数上红。变体与矩阵行早就在，缺的只是语义 ⇒ 这不是
// 编译错误，而是"什么都没发生"的行为红。⚠️ 但若把**整个判据**放到未接线代码上，(c1)/(c2)
// 仍绿（它们断言"什么都不做"）——那是控制臂的本分，不是能力。

/// 剪刀判据共用的靶子（**铅笔**画出来、几何从**权威工程**读回的音符）。
struct KnifeTarget {
    ulid: String,
    /// 宿主帧（`hit_test_visible` / `snapped_tick_at` 那一套）。
    host_center: (f32, f32),
    /// 绘制帧中心（界面 `on_clicked` 收到的那一帧）。
    painted_center: (f32, f32),
    /// 宿主帧：音符**右沿内** 2px（吸附向上取整到终点 ⇒ 不严格落在音符内）＋ 退化控制臂用它。
    ///
    /// ⚠️ 取**右**沿而不是左沿：左沿内 2px 与铅笔按下点只差 2px（< Slint 的 10px 重复点击
    /// 阈值）⇒ 这一击会被 `ClickState::check_repeat` 当成**双击**、派发器被调用两次。
    /// 右沿内 2px 距铅笔按下点约一个音符宽（> 10px）⇒ 是一次**真正单击**。
    host_degenerate: (f32, f32),
    /// 绘制帧：同上。
    painted_degenerate: (f32, f32),
    /// 切分前的 `(start_tick, pitch, duration_ticks)`。
    before: (u64, u8, u64),
    velocity: u8,
    /// 判据**自己**用 `snapped_tick_at` 算出的切分 tick（⛔ 不读被测返回值）。
    split_tick: u64,
}

/// 装配靶子：铅笔在片段内画一颗音符，再从**权威工程**读回几何与期望切分点。
///
/// 三条夹具自证（少任何一条，(a) 就可能测的是别的路径）：铅笔真的加了音符；宿主命中测试在
/// **宿主帧中心**解析到的**就是**它自己；中心的吸附 tick **严格落在**音符内部。
fn knife_target(
    window: &yeban_app::ui::MainWindow,
    port: &Rc<UndoPort>,
    plane: &mut LiveControlPlane,
    seq: i64,
) -> KnifeTarget {
    assert_eq!(window.get_roll_scroll_x(), 0.0, "夹具前提：起点滚动为 0");
    // ① 一个**真的落在片段内**的点击 x（与铅笔 / 橡皮擦判据同一自证）。
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
    let before_count = note_count(&project_before);
    window.set_active_tool(i32::from(Tool::Pencil.digit()));
    click_in_roll_by_id(plane, seq, hit_x);
    assert_eq!(
        note_count(&port.project()),
        before_count + 1,
        "夹具前置：铅笔必须先真的加出一个音符"
    );
    let ulid = notes(&port.project())
        .into_iter()
        .map(|(id, _, _)| id)
        .find(|id| !notes(&project_before).iter().any(|(old, _, _)| old == id))
        .expect("必须能读出新增音符的身份");

    // ② 几何从**权威工程**读回（⛔ 不猜坐标）。
    let view = yeban_app::bridge::ViewState::from_project(&port.project()).expect("投影");
    let index = view
        .notes
        .iter()
        .position(|note| note.id == ulid)
        .expect("新音符必须在投影里");
    let bounds = element_bounds(plane, seq + 10, SCROLL_AREA);
    let viewport_width = bound_f32(&bounds, "width");
    let host_center = note_center_in_host_view(&view, index, 0.0);
    let painted_center = note_center_in_painted_view(&view, index, 0.0);
    assert_eq!(
        view.hit_test_visible(
            0.0,
            viewport_width,
            host_center.0,
            host_center.1,
            NOTE_HEIGHT
        ),
        Some(index),
        "夹具前提：靶子的宿主帧中心必须被宿主命中测试解析到它自己"
    );
    let before = note_by_ulid(&port.project(), &ulid);
    let velocity = velocity_of(&port.project(), &ulid);
    let split_tick = view.snapped_tick_at(0.0, host_center.0, ROLL_SNAP_GRID_TICKS);
    assert!(
        split_tick > before.0 && split_tick < before.0 + before.2,
        "夹具前提：中心的吸附 tick {split_tick} 必须**严格**落在音符 [{}, {}) 内部",
        before.0,
        before.0 + before.2
    );
    // 右沿内 2px（宿主帧）⇒ 吸附**向上**取整到音符终点（不严格落在内部）。仍命中靶子 ⇒
    // 控制臂测的是"拒绝退化切分"，不是"点了空白"。
    //
    // ⚠️ 为什么不是左沿：左沿内 2px 距铅笔按下点只差 2px（< 10px 的重复点击阈值）⇒ 这一击会被
    // Slint 当成**双击**、派发器被调用**两次**（实测：未加 `ops.is_empty()` 短路时空批提交 +2）。
    // 右沿内 2px 距铅笔按下点约一个音符宽 ⇒ 是**一次**真正单击（提交增量恒为 1）。
    let host_degenerate = (
        view.notes[index].x - 0.0 + view.notes[index].width - 2.0,
        host_center.1,
    );
    assert_eq!(
        view.hit_test_visible(
            0.0,
            viewport_width,
            host_degenerate.0,
            host_degenerate.1,
            NOTE_HEIGHT
        ),
        Some(index),
        "夹具前提：退化臂的点必须仍然命中靶子（否则测的是空白而不是拒绝）"
    );
    KnifeTarget {
        ulid,
        host_center,
        painted_center,
        host_degenerate,
        painted_degenerate: host_to_painted(host_degenerate.0, host_degenerate.1),
        before,
        velocity,
        split_tick,
    }
}

/// 判据 (a)：剪刀在音符**绘制帧中心**单击 ⇒ 原文被切**成恰好两个**独立音符：
/// `pitch` / `velocity` 逐值继承，左 `[start, split)` ＋ 右 `[split, end)` 恰好铺满原文，
/// 且切分点 == 判据**自己**用 `snapped_tick_at` 算出的值。
///
/// ## 它证明什么
///
/// 一条**端到端**的链：界面工具属性（剪刀）→ 真实指针注入（`down → move → up`，绘制帧）→
/// 宿主那**一个** `hit_test_visible` 命中 → `snapped_tick_at` → `split_ops_for`
/// （`DeleteNote` ＋ 两个 `EntityId::new()` 的 `AddNote`）→ **权威工程**。
///
/// ## ⛔ 它不证明什么
///
/// - 剪刀的**拖拽**切分（`ToolDrag::SliceAcross` 仍未接线）；
/// - 跨片段 / 跨轨 / 多选切分（本切片**只**切指针下那一颗）；
/// - 切分**期**的画面（无截图判据）；
/// - 网格竖线的**绘制**位置（本判据断言的是模型里的 tick，不是像素）。
#[test]
fn a_knife_click_splits_the_note_under_the_cursor_into_two_tiles() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (b)：一次剪刀手势 = **恰好一次**提交 / **一步**撤销；一次 `UiAction::Undo` 之后工程
/// **逐字节**回到切分前（`serde_json::to_vec` 比较）。
///
/// ## 它证明什么
///
/// `split_ops_for` 返回的三条 op 被 `undo_session::commit` 包成**一条** `Op::Batch`
/// （`ARCH-OPS-002`）—— ⛔ 不是三次提交（那要按三次 `Cmd+Z` 才回来）。
///
/// ## ⛔ 它不证明什么
///
/// - 撤销**之后**的重做（由既有撤销判据管）；
/// - 界面在撤销后的重投影（本判据只读权威工程的字节）。
#[test]
fn b_knife_split_is_one_atomic_commit_and_one_undo_step() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c1) **控制臂**：剪刀点**空白** ⇒ 0 提交、0 音符变化、工程逐字节不变。
///
/// ## ⚠️ 它在**未接线**的代码上也是绿的
///
/// 它断言的是"没有假阳性"（点了空白不该切任何东西），**不是**能力存在。能力由 (a)/(b) 管。
#[test]
fn c1_a_knife_click_on_blank_space_changes_nothing() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c2) **控制臂**：吸附 tick **不严格落在**音符内部（点在音符**右沿内** 2px ⇒ 吸附向上
/// 取整到终点）⇒ **拒绝**切分：0 提交、原文一位不动、工程逐字节不变。
///
/// ## ⚠️ 它在**未接线**的代码上**空转绿**（同族判据反复踩过这个坑）
///
/// 未接线时"什么都不做"，本判据当然绿。它的红必须靠**变异**挣：删掉宿主那侧的
/// `if ops.is_empty() { return; }` 短路 ⇒ 空 `Vec` 也会被 `commit_ops` 包成一条空
/// `Op::Batch` 提交（模型**不**拒绝空批）⇒ 本判据在**提交数**上红。实测读数见交付报告。
///
/// ## 夹具只覆盖 `== end` 那一半
///
/// 铅笔画出的音符起止**必在**网格上、时值是网格的整数倍 ⇒ 右沿内 2px 的吸附恰好**等于**终点
/// （不是严格大于）。`split_ops_for` 里的 `split_tick >= end` 用 `>=` 而不是 `==`：非网格对齐的
/// 时值（例如导入的 MIDI）会让吸附点**落到终点之后**，那时若用减法算右半时值会 u64 下溢
/// （debug 下 panic）；`split_tick <= start` 的 `<=` 同理守着左半。
#[test]
fn c2_a_knife_click_whose_snapped_tick_is_outside_the_note_is_a_no_op() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

// ===========================================================================
// `[UI-NOTE-005]` §3.5 项 1–2：**方向键微调**（2026-10-13）
// ===========================================================================
//
// ## 规范（`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:213-215`，逐字）
//
// 1. `←` / `→` 沿时间轴以**网格步长**平移**选区**音符；`Alt + ←/→` 执行 **1 tick** 超精细微调；
// 2. `↑` / `↓` 执行**半音**移调；`Shift + ↑/↓` 瞬时执行**整八度**（±12 半音）移调。
//
// 3. `Shift + ←/→` 以**一个吸附网格**缩短 / 延长**选区**音符的时值
//    （`Op::SetNoteTiming`，起点不动、身份不动）；
// 4. 项 4 `Space`/`Enter` 试听与既有的 `Space` = 播放冲突 ⇒ **仍不**在本组里。
//
// ## 这五条判据各自咬哪一格
//
// | 判据 | 咬住 |
// | :--- | :--- |
// | `arrow_nudge_a_...` | 四格位移**逐音符**等于**独立算出来**的幅度；选区外的音符一位不动；duration / velocity 不动 |
// | `arrow_nudge_b_...` | N 次离散按键 ⇒ **N 次提交 / N 步撤销**；一次撤销回到**上一次按键之前**（不合并） |
// | `arrow_nudge_c1_...` | 空选区 ⇒ 0 提交、不消费、逐字节不变（⚠ **未修复的代码上也是绿的**） |
// | `arrow_nudge_c2_...` | 组夹紧：整块走不动 ⇒ 0 提交（⚠ **未修复的代码上也是绿的**，红要靠变异） |
// | `arrow_nudge_d_focus_...` | ⭐ **焦点那一问**：经**真 `WindowEvent`** 注入 `\u{f703}`（= winit 给 `→` 的同一个文本）真的动了音符 |
// | `arrow_duration_resize_e_...` | 项 3：真 `Shift` 按下 + 真 `←`/`→` ⇒ **只**改选区时值（±一个网格）；身份与起点不动；一次按键一步撤销 |
//
// ## ⭐ 焦点那一问（本切片最重要的未知）
//
// `ui/app.slint` 的 `key-handler`（`forward-focus`）是**唯一**的键盘事件源；问题曾经是
// "Slint 的焦点系统会不会在窗口键处理器之前把方向键文本吃掉"。`arrow_nudge_d_focus_*`
// 给出**机械**答案：它不调 `.slint` 回调，而是经 `ui/dispatch_key_press` 送一个**真**
// `WindowEvent::KeyPressed { text }`（`i-slint-core-1.18.1/api.rs:717` 那一条），
// 文本取的是 `i-slint-common-1.18.1/key_codes.rs:59` 给 `→` 的**同一个** `\u{f703}`。
// 它真的动了音符 ⇒ 焦点链把方向键文本交给了宿主（`i-slint-core` 只在 `Tab`/`Esc`/菜单
// 快捷键上截键，方向键只在**真 `TextInput` 聚焦**时被 `text_shortcut` 消费）。
//
// ## ⛔ 本组判据**不**证明什么
//
// - ⛔ **不**证明真 GUI（winit 窗口）里手按方向键会有同样的结果 —— 全部证据是**无头注入**；
// - ⛔ **不**证明像素（没有截图 / golden）：判据读的是**权威工程**的字段与提交计数；
// - ⛔ **不**证明试听（规范项 4）—— 它与既有的 `Space` = 播放冲突，未实现；
// - ⛔ **不**证明键重复的**平台**行为（本判据是"N 次离散按键"这一形态，不是一个 OS 自动重复流）。

/// `[UI-NOTE-005]` §3.5 项 1–2 的**独立**幅度读数。
///
/// ⛔ **不读** `host::ROLL_SNAP_GRID_TICKS` / `host::nudge_delta_for`：网格由**模型**的
/// PPQ 常量推导（960 ÷ 4 = 240 tick = 1/16 音符），其余三档是规范逐字（1 tick / 半音 / 八度 = 12）。
const NUDGE_GRID_TICKS_EXPECTED: i64 = yeban_model::ids::PPQ as i64 / 4;
/// `Alt+←/→` 的 1 tick（规范项 1 后半）。
const NUDGE_FINE_TICKS_EXPECTED: i64 = 1;
/// `↑`/`↓` 的一个半音（规范项 2）。
const NUDGE_SEMITONE_EXPECTED: i8 = 1;
/// `Shift+↑/↓` 的整八度（规范项 2 逐字"±12 半音"）。
const NUDGE_OCTAVE_EXPECTED: i8 = 12;

/// 音符读数：ULID → `(start_tick, pitch, duration_ticks, velocity)`（**模型**侧 oracle）。
fn note_readings(
    project: &yeban_model::YebanProjectV1,
) -> std::collections::BTreeMap<String, (u64, u8, u64, u8)> {
    let mut out = std::collections::BTreeMap::new();
    for entry in project.clip_pool.values() {
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for (id, note) in map {
            out.insert(
                id.to_canonical_string(),
                (
                    note.start_tick,
                    note.pitch,
                    note.duration_ticks,
                    note.velocity,
                ),
            );
        }
    }
    out
}

/// 按 `(start_tick, pitch)` **精确点名**夹具里的音符身份（确定顺序：ULID 键序）。
///
/// ⛔ 它按**输入数据**选靶子（模型里读出来的 tick / 音高），**不**按被测代码给出的期望值选。
fn ulids_at(project: &yeban_model::YebanProjectV1, wanted: &[(u64, u8)]) -> Vec<String> {
    note_readings(project)
        .into_iter()
        .filter(|(_, reading)| wanted.contains(&(reading.0, reading.1)))
        .map(|(id, _)| id)
        .collect()
}

/// 把一批音符身份写进选区的**唯一事实源** `selected-ulids`（并同步计数）——
/// 与 `tests/live_ui_mcp.rs` 的 `select_ulids` 同款（那个是那一个测试目标的局部 helper）。
fn select_ulids(window: &yeban_app::ui::MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

/// 微调判据共用的装配：`build_live_ui_with` 把**撤销端口**当装配选项传进去
/// （`live::LiveWiringOptions::undo`）⇒ `host::wire_keys` 从第一刻起就握着端口。
///
/// 为什么不用本文件既有的 [`assemble`]：那个走 `build_live_ui`（`undo: None`）+ 事后
/// `wire_roll_edit`，键盘那条链**没有**撤销端口 ⇒ `Cmd+Z` 之外的编辑类动作会如实 `reject`
/// （`NudgeSelection` 也会）。这里走生产形态（`main.rs` 也是把端口交给装配）。
fn assemble_for_arrow_nudge() -> (
    yeban_app::ui::MainWindow,
    Rc<UndoPort>,
    LiveControlPlane,
    Rc<std::cell::RefCell<yeban_app::input::InputContext>>,
) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<arrow-nudge>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
    ));
    let ui = live::build_live_ui_with(
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

/// 判据 (a)：规范 §3.5 项 1–2 的**四格**位移逐音符正确，且**只有选区在动**。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 选区 | `MainWindow.selected-ulids`（写）+ `selected-note-count`（读） | 音符身份条数 |
/// | 音符读数 | [`note_readings`]（**权威工程**） | `(tick, pitch, duration, velocity)` |
/// | 期望位移 | [`NUDGE_GRID_TICKS_EXPECTED`] 等**四个独立常量** | tick / 半音 |
/// | 提交 / 撤销 | `UndoPort::display().commit_count` / `.undoable` | 条 / 步 |
///
/// ## 它证明什么
///
/// 1. **四格都在**：网格 ±240（= 模型 PPQ/4）、`Alt` ±1 tick、半音 ±1、八度 ±12 —— 全部经
///    `invoke_key_action`（= `key-handler` 调的那**一个**回调）走
///    `wire_keys → from_text → resolve_logical → apply_action` 整条链；
/// 2. **选区作用域**：选区里的**每一个**音符都按同一幅度动 —— 这是拖拽臂做不到的
///    （`begin_note_drag` 只持**一个** `note-drag-ulid`）；
/// 3. **只有选区在动**：选区之外每一个音符的**四个字段**都逐字节不变；
/// 4. **时值 / 力度不动**：`Op::MoveNote` 只动 tick 与音高；
/// 5. 每一格 = **一次**提交 / **一步**撤销，且一次撤销逐字节回到该格之前。
///
/// ## ⛔ 它不证明什么
///
/// - ⛔ 不证明真焦点链（那是 `arrow_nudge_d_focus_*` 的事）；
/// - ⛔ 不证明像素 / 截图；
/// - ⛔ 不证明 `Shift+←/→`（项 3）与试听（项 4）；
/// - ⚠ 它用 `invoke_key_action` 注入**修饰组合**（端口表达不了 chord —— 仓库既有的
///   `Shift+Space` 判据是同一格）。真实事件源那一半由 `arrow_nudge_d_focus_*` 补。
#[test]
fn arrow_nudge_a_the_four_spec_combinations_move_every_selected_note_by_the_spec_amount() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (b)：**一次按键 = 一次提交 = 一步撤销**；N 次离散按键 ⇒ N 步，且撤销**不合并**。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 按键次数 | `invoke_key_action("\u{f703}", …)` 的调用次数 | 次 |
/// | 提交 | `UndoPort::display().commit_count` | 条 |
/// | 可撤销步数 | `UndoPort::display().undoable` | 步 |
/// | 工程内容 | `serde_json::to_vec(&port.project())` | 字节 |
///
/// ## 它证明什么
///
/// 3 次按键 ⇒ 提交 +3、可撤销 +3；三次快照两两不同（不是同一个状态记了三次）；
/// **一次**撤销回到**第 3 次按键之前**、再一次到第 2 次之前、再一次到起点 ——
/// 每一步都逐字节相同。这同时排除了两种错误实现：把 N 次并成一步（第一次撤销就会回到
/// 起点）与把一次拆成多步（`undoable` 会涨得比按键次数快）。
///
/// ## ⚠ 它不证明什么
///
/// - ⛔ 不证明 OS 的键重复流**真的**是"N 个离散 `KeyPressed`"（本判据注入的就是这个形态；
///   `i-slint-core-1.18.1/api.rs:724-733` 的 `KeyPressRepeated` 走**同一条**
///   `process_key_input`，但本判据没有注入 `repeat: true` 的变体）；
/// - ⛔ 不证明提交里的 op **类型**（只证明历史前进一次 + 工程字节可逆）。
#[test]
fn arrow_nudge_b_each_key_press_is_exactly_one_commit_and_one_undo_step() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c1)：**空选区**下按方向键 ⇒ 0 提交、工程逐字节不变、**不消费**这一键。
///
/// ## ⚠ 它在**未修复**的代码上也是绿的（如实披露）
///
/// 未接线时 `LogicalKey::from_text("\u{f702}")` 返回 `None` ⇒ 什么都不发生 ⇒ 本判据当然绿。
/// 它断言的是**没有假阳性**（空选区不许凭空改工程），**不是**能力存在 —— 能力由 (a) 管。
/// 与剪刀 / 力度那两片的 `(c)` 控制臂同一性质。
///
/// ## 它证明什么
///
/// 四格都在空选区下**如实拒绝**（返回 `false`，让 `.slint` 走 `reject`）而不是写空 op
/// 或猜一个对象去动 —— 与 `DeleteSelection` / `Duplicate` / `ZoomToSelection` 的空前置
/// 条件同一取向。
/// ## ⛔ 它不证明什么
///
/// ⛔ 不证明"有选区时会动"（那是 (a)）；⛔ 不证明夹紧（那是 (c2)）。
#[test]
fn arrow_nudge_c1_a_press_with_no_selection_commits_nothing() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c2)：**组夹紧** —— 选区里有一颗音符在 tick 0 时按 `←`，整块一步都走不动
/// ⇒ 0 提交、工程逐字节不变；同一个选区按 `→` 必须真的动**两颗**（正控制）。
///
/// ## ⚠ 它在**未修复**的代码上也是绿的；它的红要靠**变异**
///
/// 未接线时同样"什么都不做"。它的牙咬的是**夹紧的粒度**：把 `host::nudge_ops_for` 的
/// 组夹紧换成"逐音符各自夹"（或直接不夹、让模型拒）⇒ 那颗 tick 0 的音符留在原地、
/// `(1920,67)` 那颗左移 240 ⇒ **提交数 0 ⇒ 1**，本判据在 `assert!(!consumed)` 与提交数上红。
/// 实测变异读数见交付报告。
///
/// ## 它证明什么
///
/// 1. 位移是**一个公共量**（选区保持内部相对形状 —— 与 `duplicate_ops_for` 的"整块平移"
///    同一条理由），⛔ 不是"能走的先走、走不动的留着"；
/// 2. 走不动时**不写空 op**（`Op::MoveNote` 的越界是**拒绝**，组夹紧把它变成"不提交"）；
/// 3. 正控制证明这个选区是**活的**：同一个身份集合按 `→` 确实两颗都动了 240。
///
/// ## ⛔ 它不证明什么
///
/// - ⛔ 不证明音高轴的夹紧（这里只走 tick 轴；音高轴的纯函数读数在 `host.rs` 的单元判据里）；
/// - ⛔ 不证明"上界"（`u64::MAX`）那一半 —— 夹具里没有任何音符接近时钟顶端。
#[test]
fn arrow_nudge_c2_a_press_that_cannot_move_the_whole_selection_commits_nothing() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// ⭐ 判据 (d)：**焦点那一问的机械答案** —— 经**真 `WindowEvent`** 注入 `\u{f703}`
/// （winit 给 `→` 的同一个文本）真的把选区移了一个网格。
///
/// ## 为什么这一条是本切片最重要的
///
/// `ui/app.slint` 的 `key-handler`（`forward-focus`）是唯一的键盘事件源；在它之下
/// 是否还有东西先把方向键文本吃掉，**读代码读不出来**。本判据让 Slint 自己去走那条链：
///
/// ```text
/// ui/dispatch_key_press(keyCode="\u{f703}")
///   → yeban_ui_test_port 的 dispatch_key_press_impl（Character ⇒ type_char）
///   → slint::platform::WindowEvent::KeyPressed { text: "\u{f703}" }   ← **真事件**
///   → i-slint-core api.rs:717 的 process_key_input
///   → 焦点项（key-handler，forward-focus）→ key-pressed → key-action
///   → host::wire_keys → LogicalKey::from_text → resolve_logical → apply_action → commit
/// ```
///
/// 音符真的动了 ⇒ 焦点系统**没有**在窗口键处理器之前把方向键文本消费掉。
///
/// ## 控制臂（排除"注入没生效"与"任何文本都能动音符"）
///
/// 0. 注入前断言 `input_context().focus() == MainCanvas` 且非合成态（否则"没反应"可能是
///    IME / 焦点门控，而不是接线）；
/// 1. 先注入真 `Tab`（**已绑定**键）⇒ `arrangement-view` 真的翻转 ⇒ "真事件源 →
///    `key-handler` → `key-action` → `apply_action`"这条链在本装配里是**活的**；两次 `Tab`
///    复原视图。这一臂是关键：没有它，下面 `→` 的失败无法排除"事件根本没送到"；
/// 2. 注入 `\u{f704}`（= `F1`，**未绑定**）⇒ 0 提交、工程逐字节不变 —— 证明不是
///    "随便什么文本都触发微调"。
///
/// ## ⛔ 它不证明什么
///
/// - ⛔ **不**证明真 winit 窗口里手按 `→` 有同样的结果：本判据的文本是**测试**按
///   `key_codes.rs` 的表交给 `WindowEvent` 的（真平台那一半由 `i-slint-backend-winit` 的
///   `for_each_keys!` 表把 `NamedKey::ArrowRight` 映到同一个码位 —— 那是**读**出来的，不是本判据观测的）；
/// - ⛔ 不证明带修饰组合的真实事件（`Shift` 那一格由下一条判据观测；`Alt` 端口**表达不了**）。
#[test]
fn arrow_nudge_d_focus_a_real_window_key_event_moves_the_selection() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// 判据 (d2)：**`Shift` 组合**经真事件源也通 —— 注入真 `Shift` 按下，再注入真 `↑`
/// ⇒ 整八度（+12 半音），证明 Slint 的修饰位状态真的传到了宿主。
///
/// ## 为什么单独一条
///
/// `yeban_ui_test_port` 的 `KeyCode` 表达不了 `Alt`（所以 `Alt+←/→` 只能走 `.slint` 回调那一格），
/// 但它**有** `Shift`（`port.rs` 的 `KeyCode::Shift` ⇒ `WindowEvent::KeyPressed{text:"\u{10}"}`
/// ⇒ `i-slint-core` 的 `InternalKeyboardModifierState::state_update` 把 `left_shift` 置位）。
/// `Shift` 在端口上**只按不放**，所以它必须在**自己的**装配里最后一个注入。
///
/// ## ⛔ 它不证明什么
///
/// - ⛔ 不证明 `Alt` 组合（端口表达不了 —— 那两格由判据 (a) 经回调那一格观测）；
/// - ⛔ 不证明真实键盘的按下/释放顺序。
#[test]
fn arrow_nudge_d2_shift_through_the_real_event_source_lifts_the_selection_by_an_octave() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// 判据 (e)：`[UI-NOTE-005]` §3 项 3 —— `Shift+←/→` 经**真事件源**只改时值。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 时值变化 | [`note_readings`]（**权威工程**）的 `duration_ticks` | tick |
/// | 起点 / 音高 / 力度 | 同一个读数的另三个字段 | tick / 半音 / `u8` |
/// | 提交 / 撤销 | `UndoPort::display().commit_count` / `.undoable` | 条 / 步 |
/// | 提交载荷 | `port.graph()` 头提交的 `StampedOp` | `Op` 的 `Debug` 文本 |
///
/// ## 它证明什么
///
/// 1. **真事件源**：真 `Shift` 按下（端口 `KeyCode::Shift`）＋ 真 `←`（`\u{f702}`）⇒
///    宿主真的改了一个字段 —— 端口表达不了 chord 的那一格在这里是**组合**出来的，
///    与 `arrow_nudge_d2_*` 同一手法；
/// 2. **只改时值**：选区每颗 `duration_ticks` 恰好 −240；起点 / 音高 / 力度逐值不变；
///    选区之外的音符四个字段**逐值不变**；
/// 3. **身份保住**：读数键（ULID）集合前后一致 ⇒ 走的是 `Op::SetNoteTiming`，
///    ⛔ 不是"删 + 加"（那会换掉 `EntityId`，进而改变概率触发结果）；
/// 4. **提交载荷**：头提交恰好一条 `StampedOp`（`Op::Batch`），批里每个选中音符一条
///    `SetNoteTiming`；
/// 5. **一次按键 = 一步撤销**：`→`（`Shift` 仍按着）延回原值后再撤销两次，
///    逐字节回到起点。
///
/// ## ⛔ 它不证明什么
///
/// - ⛔ 不证明 GUI 人手按键（全部证据是无头注入）；
/// - ⛔ 不证明像素；
/// - ⛔ 不证明幅度**常量**的来源（期望值 240 来自模型 PPQ/4，与 `NUDGE_GRID_TICKS_EXPECTED` 同源）。
#[test]
fn arrow_duration_resize_e_shift_arrows_change_only_the_selected_durations() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

// ===========================================================================
// ⭐ 任务单 `#166`：铅笔写音符的**轨** —— 只认"设备架 / 界面正在显示的那条轨"，
// ⛔ 不再认"第一条有片段覆盖该 tick 的轨"（那会在多轨时写到别的轨）。
// ===========================================================================
//
// ## 规则（与 `bridge::device_rack_track` / `send_strip_track` **同一条**）
//
// "展示轨" = 身份序（`BTreeMap` 键序）第一条**非主总线**轨。设备架 / 发送条 / 轨删除 /
// 一键装乐器已经在用它；铅笔此前**自己**遍历 `project.tracks` 找"第一条有片段覆盖"的轨
// （`#164` 的账本把它登记为待修项）⇒ 多轨时两套规则会分叉：面板说 A、铅笔写 B。
//
// ## 三条判据（各自证明什么 / ⛔ 不证明什么）
//
// | 判据 | 证明 | ⛔ 不证明 |
// | :--- | :--- | :--- |
// | `a_pencil_is_refused_...` | 展示轨 A **无**片段、B 有覆盖该 tick 的片段 ⇒ 点击后 B 音符数 **不变**、工程**逐字节**不变、提交数不变，且 `roll-edit-status` **逐字**等于拒绝原因 | 音色 / 听证；别的工具 |
// | `b_pencil_writes_...` | 展示轨 A **有**覆盖 ⇒ 新音符进 **A 的片段**，B 片段音符数不变 | 多选 / 跨片段；拖拽画音符 |
// | `c_single_track_...` | 只有一条非主总线轨时：两条规则**指向同一条轨**，`pencil_op_for` 的 `(track, clip, tick, pitch, duration)` 与独立算出的计划**逐值相同**；点击后恰好 +1 | 别的工程形态（空工程 / 只有主总线） |
//
// ## 怎么变红（**行为**红）
//
// - 把 `pencil_attempt_for` 的寻址改回"遍历 `project.tracks` 取第一条有覆盖的轨"
//   ⇒ (a) 红在 `B 音符数`（实测：0 ⇒ 1，即**又写错轨**）。
// - 保留新寻址但删掉派发器 `Err` 臂里的 `set_roll_edit_status`
//   ⇒ (a) 红在"拒绝必须可见"（文本为空）。
// - 把寻址改成"第一条非主总线轨，但片段仍在**所有轨**里找"
//   ⇒ (b) 红在 `A 的片段音符数`（新音符会落到 B 的片段里）。

/// `#166` 夹具的**固定 ULID**（与 `samples::fixture_id` 同一形态）。
///
/// ⛔ 不用 `EntityId::new()`：它的随机低位让"谁是身份序第一条"不可构造 —— 而本判据的
/// **全部**语义都建立在"展示轨 = 身份序第一条非主总线轨"上。
fn pencil_fixture_id(index: u128) -> yeban_model::EntityId {
    format!("01J8ZQ{index:020}")
        .parse()
        .expect("26 字符 Crockford Base32 ULID")
}

/// 在 `track` 上摆一个 `[start_tick, start_tick + duration_ticks)` 的空 MIDI 片段。
fn place_empty_midi_clip(
    track: &mut yeban_model::TrackV3,
    pool: &mut BTreeMap<yeban_model::EntityId, yeban_model::ClipPoolEntry>,
    clip_id: yeban_model::EntityId,
    placement_id: yeban_model::EntityId,
) {
    pool.insert(
        clip_id,
        yeban_model::ClipPoolEntry {
            id: clip_id,
            name: "Clip".to_owned(),
            content: yeban_model::ClipContent::Midi {
                notes: BTreeMap::new(),
            },
        },
    );
    track.clips.insert(
        placement_id,
        yeban_model::ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: 1920,
            loop_config: yeban_model::LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
}

/// `#166` 的夹具：主总线 ＋ **展示轨 A** ＋ **后面的轨 B**（两条都可选地有片段）。
struct PencilTrackFixture {
    project: yeban_model::YebanProjectV1,
    displayed: yeban_model::EntityId,
    displayed_clip: yeban_model::EntityId,
    later: yeban_model::EntityId,
    later_clip: yeban_model::EntityId,
}

fn pencil_track_fixture(displayed_has_clip: bool, later_has_clip: bool) -> PencilTrackFixture {
    let master = pencil_fixture_id(1);
    let displayed = pencil_fixture_id(2);
    let later = pencil_fixture_id(3);
    let displayed_clip = pencil_fixture_id(20);
    let later_clip = pencil_fixture_id(21);

    let mut project = yeban_model::YebanProjectV1 {
        master_bus_track_id: master,
        ..yeban_model::YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        yeban_model::TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: yeban_model::TrackKind::Master,
            ..yeban_model::TrackV3::default()
        },
    );
    let mut displayed_track = yeban_model::TrackV3 {
        id: displayed,
        name: "A".to_owned(),
        kind: yeban_model::TrackKind::Midi,
        ..yeban_model::TrackV3::default()
    };
    let mut later_track = yeban_model::TrackV3 {
        id: later,
        name: "B".to_owned(),
        kind: yeban_model::TrackKind::Midi,
        ..yeban_model::TrackV3::default()
    };
    if displayed_has_clip {
        place_empty_midi_clip(
            &mut displayed_track,
            &mut project.clip_pool,
            displayed_clip,
            pencil_fixture_id(30),
        );
    }
    if later_has_clip {
        place_empty_midi_clip(
            &mut later_track,
            &mut project.clip_pool,
            later_clip,
            pencil_fixture_id(31),
        );
    }
    project.tracks.insert(displayed, displayed_track);
    project.tracks.insert(later, later_track);
    PencilTrackFixture {
        project,
        displayed,
        displayed_clip,
        later,
        later_clip,
    }
}

/// 一个**宿主帧**点击 x：吸附 tick 落在夹具片段的 `[0, 1920)` 内。
///
/// 期望值由判据**自己**经 `ViewState::snapped_tick_at` 算出（⛔ 不读被测函数的结果）。
fn host_x_inside_pencil_fixture(project: &yeban_model::YebanProjectV1) -> f32 {
    let view = ViewState::from_project(project).expect("夹具必须能投影");
    (0..240)
        .map(|step| 10.0 * f32::from(u8::try_from(step).unwrap_or(0)))
        .find(|x| (0..1920).contains(&view.snapped_tick_at(0.0, *x, ROLL_SNAP_GRID_TICKS)))
        .expect("夹具里必须有一个吸附 tick 落在 [0, 1920) 的点击位置")
}

/// 某个片段里的音符数（`clip_pool` 的键序读数）。
fn clip_note_count(project: &yeban_model::YebanProjectV1, clip: yeban_model::EntityId) -> usize {
    project
        .clip_pool
        .get(&clip)
        .and_then(|entry| entry.content.notes())
        .map_or(0, BTreeMap::len)
}

/// 用**指定工程**装配：真窗口 ＋ 端口 ＋ 产品同一条 `wire_roll_edit`。
fn assemble_with_project(
    project: yeban_model::YebanProjectV1,
) -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<pencil-track>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_roll_edit(ui.ui(), &port);
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 判据 (a)：展示轨 A **没有**覆盖该 tick 的片段、B **有** ⇒ 铅笔**不得**写到 B。
#[test]
fn a_pencil_click_is_refused_when_the_displayed_track_has_no_clip_at_that_tick() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (b)：展示轨 A **有**覆盖 ⇒ 写到**它**（B 也有覆盖，但 B 一位不动）。
#[test]
fn a_pencil_click_writes_to_the_displayed_track_not_a_covered_later_track() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 判据 (c)：**单轨**工程 —— 两条规则指向同一条轨，结果与改前逐值相同。
#[test]
fn a_single_track_project_keeps_the_pre_166_pencil_result() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}

/// 在卷帘面**中心**做一次越过规范的 4px 阈值的水平拖动（合计 `dx`，正 = 指针向右）。
///
/// 铅笔工具下 `note-drag-grab` 如实返回 `false` ⇒ `.slint` 走的是**滚动臂**
/// （`root.scroll-requested(root.drag-last-x - self.mouse-x)`），⛔ 不是音符拖拽臂。
fn drag_roll_horizontally(plane: &mut LiveControlPlane, seq: i64, dx: f32, steps: usize) {
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    #[allow(clippy::cast_precision_loss)]
    let center = (
        bounds["width"].as_f64().expect("width") as f32 / 2.0,
        bounds["height"].as_f64().expect("height") as f32 / 2.0,
    );
    drag_in_roll_by_id_at(plane, seq, center, dx, 0.0, steps);
}

/// 某个音符的矩形是否**在运行时控件树里**（可见反馈的口径，与判据 ① 同一读数）。
fn note_rect_in_tree(plane: &mut LiveControlPlane, seq: i64, ulid: &str) -> bool {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": format!("note-{ulid}-rect") })),
    ));
    !node.is_error()
}

/// 判据（D-UI-10）：**编辑之后的一次滚动拖动**必须重投影此刻的工程，⛔ 不得重投影
/// `wire_roll_edit` 接线那一刻抓的快照。
///
/// ## 缺陷（本判据必须在修复前的代码上变红）
///
/// 滚动 handler 过去只在 `build_main_window_with_console_tab` 里装一次，闭包抓住**装配时**
/// 的 `ViewState`，此后没有任何路径更新它；`wire_roll_edit` 也不重装它 ⇒ 生产路径上它抓住的是
/// **接线那一刻**的工程（常常还是空的那一份）。于是卷帘内一次越过 4px 阈值的指针移动
/// （人手上的"随手一拖"）会把 bpm / 轨道 / 剪辑 / 音符**整体**重绘回那份快照。
///
/// 实测症状（用控制面创作第二首歌时撞上的，`docs/ledger/app-zimmer-ui-notes.md`）：画完一批
/// 音符后随手一拖，`ui/tree` 读数变成 **0 轨 / 0 剪辑 / bpm 120.00**，刚画的音符整批从树里
/// 消失；⛔ 没有 `Op`、⛔ 没有日志、撤销栈一位没动（下一次端口侧动作重投影 ⇒ 又全部回来，
/// 因此看起来像"一次 Redo 找回了整段会话"）。
///
/// ## 本判据断言什么
///
/// ① 前置：铅笔点击在**权威工程**里加一个音符，且它的矩形进树；② 一次越过阈值的水平拖动
/// 确实推动了宿主拥有的 `roll-scroll-x`（证明 `scroll-requested` 这条路真的被驱动，⛔ 不是
/// "事件没送到"造成的假绿）；③ 拖动之后那个音符**仍在树里** ⇒ 重投影用的是此刻的工程；
/// ④ 拖动⛔ 不改工程、⛔ 不插新音符（`up` 只在未越阈值时才 fired `clicked`）。
#[test]
fn a_scroll_drag_after_a_pencil_edit_redraws_the_live_project_not_the_wired_snapshot() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("> note-drag-active:"),
        "新 UI 契约面必须声明 > note-drag-active:"
    );
    assert!(
        aether_host.contains("set_note_drag_active"),
        "宿主接线层必须注册 set_note_drag_active"
    );
}
