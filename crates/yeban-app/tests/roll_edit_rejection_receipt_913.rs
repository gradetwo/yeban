//! T-FIX-913：选区/音符拖动三条拒绝的运行时落点（运行时 UI 读数，不 grep 源码）。
//!
//! 只读 `MainWindow` 的运行时属性，证明三件事：
//! - 拒绝文案含原因＋工程未变＋可执行下一步（中文句逐字钉在文件头常量里）；
//! - 落点是卷帘状态行＋模块回执行，提示条按**错误级**常驻（`toast-is-error` 真）；
//! - 拒绝不改工程（提交计数 / 音符集合 / 序列化字节三路对账）。
//!
//! 三条臂的触发方式：
//! - 身份臂：拖动中途删掉指针下那颗音符再松手 ⇒ `note_drag_move_ops_for` 空集；
//! - 边界臂：**指针拖动到不了** —— `note_move_delta_for` 把纵向坐标夹到 0..15 号泳道，
//!   单颗拖动能达到的最高音高是 `roll-base-pitch + 15 ≤ 127`（基音本身钳在 0..112），
//!   最低是基音 `≥ 0`；组拖动另有 `clamp_pitch_delta` 组夹紧。边界文案的**落点**
//!   由直接调 `report_roll_edit_rejected` 钉住（与微移臂同一条），其模型前提
//!   （`Op::MoveNote` 越界 ⇒ `commit_ops` 拒）由直接提交一条越界 op 证明；
//!   另有一条判据把\"顶格音符再往上拖\"钉成**空操作**（无提交、无拒绝、工程不动），
//!   免得后人误以为这条臂能被拖出来。
//! - 微移臂：键盘微移的 op 构造自带组夹紧、提交紧跟构造，`commit_ops` 的 err 分支
//!   在 UI 路径上不可达（见回执 honest gaps）—— 本文件只覆盖它的**落点函数**
//!   （直接调公开的 `report_roll_edit_rejected`），触发路径由 581 契约判据钉住。
//!
//! 绘制坐标的基音口径：`ViewState::from_project` 的 `note.y` 是按默认基音 60
//! 算的，但运行时 `roll-base-pitch` 由工程音高范围对齐（夹具工程是 58），
//! 命中测试与界面绘制都用**实时基音**。靶子中心必须用
//! `pitch_lane_y_with_base(pitch, ui.get_roll_base_pitch())` 算，
//! 用 `note.y` 会偏两条泳道（28px），拖动根本抓不到音符。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use yeban_app::bridge::{ViewState, host_to_painted, pitch_lane_y_with_base};
use yeban_app::host::{self, delete_ops_for, wire_roll_edit};
use yeban_app::input::Tool;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 当前语言是**进程级**原子量：同一测试二进制里的判据并行跑，一个判据
/// 切英文会污染另一个判据的中文断言。语言敏感的判据全程持这把锁，
/// 并在开头显式切到自己要的语言，结尾切回中文 —— 决定性，不靠执行顺序。
static LANGUAGE_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 持语言锁并把界面切到指定语言；返回的 guard 必须活到判据结束。
fn lock_language(
    ui: &yeban_app::ui::MainWindow,
    lang: yeban_app::cli::Language,
) -> std::sync::MutexGuard<'static, ()> {
    let guard = match LANGUAGE_GUARD.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    host::apply_language(ui, lang);
    guard
}
const SCROLL_AREA: &str = "piano-roll-scroll-area";

/// 音符矩形的高度（与 `host::NOTE_HEIGHT_PX` / `piano_roll.slint` 的 `12px` 同一读数）。
const NOTE_HEIGHT: f32 = 12.0;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 拖动分几步注入。
const DRAG_STEPS: usize = 4;

/// 一格泳道的宿主高度（`PITCH_LANE_HEIGHT_PX`）。
const LANE_HEIGHT: f32 = 14.0;

/// 边界臂的基音：`roll_base_pitch` 把原始值钳在 0..112（`base + 15 ≤ 127`），
/// 112 是能让 127 音高落进可见窗口的最大基音 —— 此时 127 在第 0 泳道（顶格）。
const BOUNDS_BASE_PITCH: i32 = 112;

/// 三条拒绝的中文规范句（与 `roll_edit_rejection_copy_913.rs` 的表逐字同一份）。
const NUDGE_ZH: &str =
    "模型拒绝：选区移动越界或身份已失效，工程没有变化。请重新选择后再移动，或缩小移动幅度后重试。";
const IDENTITY_ZH: &str =
    "音符拖动被拒：音符身份已失效（可能已被删除），工程没有变化。请重新选择音符后再拖动。";
const BOUNDS_ZH: &str = "音符拖动被拒：已到音高上限/下限或已到时间轴边界，工程没有变化。请缩小拖动幅度，或把音符拖回边界内再松手。";
const BOUNDS_EN: &str = "Note drag rejected: the pitch limit or the timeline edge was reached, and the project is unchanged. Drag a shorter distance, or drag the note back inside the limits before releasing.";
const IDENTITY_EN: &str = "Note drag rejected: the note no longer exists (it may have been deleted), and the project is unchanged. Select the note again and drag it.";

fn build_ui() -> LivePort<yeban_app::ui::MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("LivePort 必须装配 MainWindow")
}

/// 装配：真窗口 ＋ 端口 ＋ **与产品同一条** `wire_roll_edit`。
fn assemble() -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    assemble_with_project(yeban_model::samples::filled_project())
}

fn assemble_with_project(
    project: yeban_model::YebanProjectV1,
) -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<roll-edit-913>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_roll_edit(ui.ui(), &port);
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 工程里的音符数（与 `live_ui_mcp.rs` 的 `model_note_count` 同一口径）。
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

/// 按语义 ID 在**绘制帧** `from` 处按下；返回 `piano-roll-scroll-area` 的绝对窗口原点。
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

/// 松手（位置复用最后一次按下 / 移动）。
fn pointer_up_in_roll(plane: &mut LiveControlPlane, seq: i64) {
    let up = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
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

/// 靶子音符的一切读数（判据共用的装配段）。
struct RollTarget {
    ulid: String,
    host_center: (f32, f32),
    painted_center: (f32, f32),
    before: (u64, u8, u64),
}

/// **宿主帧**下"点在音符中心"：`view.notes` 的 x ＋ 宽一半，y ＋半个音符高。
fn note_center_in_host_view(
    view: &yeban_app::bridge::ViewState,
    index: usize,
    scroll: f32,
    base: u8,
) -> (f32, f32) {
    let note = &view.notes[index];
    (
        note.x - scroll + note.width / 2.0,
        pitch_lane_y_with_base(note.pitch, base) + NOTE_HEIGHT / 2.0,
    )
}

/// **绘制帧**下"点在音符中心"：宿主中心经 [`host_to_painted`] 搬到绘制坐标。
fn note_center_in_painted_view(
    view: &yeban_app::bridge::ViewState,
    index: usize,
    scroll: f32,
    base: u8,
) -> (f32, f32) {
    let (x, y) = note_center_in_host_view(view, index, scroll, base);
    host_to_painted(x, y)
}

/// 找一个**人真的点得到**的音符（默认基音 60）：宿主帧中心被 `hit_test_visible`
/// 解析到它自己，且绘制帧中心落在卷帘元素矩形内。
fn a_painted_note(
    view: &yeban_app::bridge::ViewState,
    viewport_width: f32,
    viewport_height: f32,
    scroll: f32,
    base: u8,
) -> usize {
    (0..view.notes.len())
        .find(|&index| {
            let (hx, hy) = note_center_in_host_view(view, index, scroll, base);
            if view.hit_test_visible_scoped_with_base(
                scroll,
                viewport_width,
                hx,
                hy,
                NOTE_HEIGHT,
                None,
                base,
            ) != Some(index)
            {
                return false;
            }
            let (px, py) = note_center_in_painted_view(view, index, scroll, base);
            (0.0..=viewport_width).contains(&px) && (0.0..=viewport_height).contains(&py)
        })
        .expect("夹具里必须有一个宿主命中的音符，其绘制帧中心也落在卷帘元素矩形内")
}

/// 装配一个"人真的拖得动"的靶子音符（用窗口的**实时基音**算中心，见文件头）。
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
        "夹具前提：窗口的缩放就是投影的默认缩放"
    );
    assert_eq!(
        window.get_active_tool(),
        i32::from(Tool::Select.digit()),
        "夹具前提：默认工具必须是选择箭头"
    );
    let base = u8::try_from(window.get_roll_base_pitch()).expect("基音是合法 u8");
    let bounds = element_bounds(plane, seq, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let index = a_painted_note(&view, viewport_width, viewport_height, scroll, base);
    let ulid = view.notes[index].id.clone();
    RollTarget {
        before: note_by_ulid(project, &ulid),
        host_center: note_center_in_host_view(&view, index, scroll, base),
        painted_center: note_center_in_painted_view(&view, index, scroll, base),
        ulid,
    }
}

/// 把夹具工程的第一颗 MIDI 音符调到 127 音高（其余不动），返回工程与该音符 ULID。
///
/// 127 是可见窗口的顶格音高（基音 112 时在第 0 泳道）：往上拖一格在几何上
/// 是空操作（`note_move_delta_for` 把纵向坐标夹到泳道内），不是一次模型拒绝 ——
/// 这正是边界臂端到端判据要钉住的事实，见
/// `dragging_the_top_note_further_up_is_a_no_op_by_lane_clamp`。
fn project_with_top_note() -> (yeban_model::YebanProjectV1, String) {
    let mut project = yeban_model::samples::filled_project();
    let mut target: Option<String> = None;
    for entry in project.clip_pool.values_mut() {
        let Some(map) = entry.content.notes_mut() else {
            continue;
        };
        for (id, note) in map.iter_mut() {
            if target.is_none() {
                note.pitch = 127;
                target = Some(id.to_canonical_string());
            }
        }
        if target.is_some() {
            break;
        }
    }
    let ulid = target.expect("夹具工程里必须有一颗 MIDI 音符可调到 127");
    (project, ulid)
}

/// 在给定基音下找一颗**人真的点得到**的指定音符：宿主帧中心用该基音的泳道换算，
/// `hit_test_visible_scoped_with_base` 解析到它自己，绘制帧中心落在元素矩形内。
fn a_painted_note_at_base(
    view: &yeban_app::bridge::ViewState,
    viewport_width: f32,
    viewport_height: f32,
    scroll: f32,
    base: u8,
    ulid: &str,
) -> (f32, f32) {
    let index = view
        .notes
        .iter()
        .position(|note| note.id == ulid)
        .expect("目标音符必须在投影里");
    let note = &view.notes[index];
    let hx = note.x - scroll + note.width / 2.0;
    let hy = pitch_lane_y_with_base(note.pitch, base) + NOTE_HEIGHT / 2.0;
    assert_eq!(
        view.hit_test_visible_scoped_with_base(
            scroll,
            viewport_width,
            hx,
            hy,
            NOTE_HEIGHT,
            None,
            base
        ),
        Some(index),
        "目标音符必须在基音 {base} 下被命中（宿主帧中心 {hx:.1},{hy:.1}）"
    );
    let (px, py) = host_to_painted(hx, hy);
    assert!(
        (0.0..=viewport_width).contains(&px) && (0.0..=viewport_height).contains(&py),
        "目标音符的绘制帧中心必须落在卷帘元素矩形内：{px:.1},{py:.1}"
    );
    (px, py)
}

/// 三条拒绝的**落点**必须同时成立：卷帘状态行与模块回执行是同一句人话＋下一步，
/// 提示条按错误级常驻，成功标志位不得沿用上一条。
fn assert_actionable_error_landing(ui: &yeban_app::ui::MainWindow, expected: &str) {
    assert_eq!(
        ui.get_roll_edit_status().as_str(),
        expected,
        "卷帘状态行必须是完整人话＋下一步"
    );
    assert_eq!(
        ui.get_module_status_text().as_str(),
        expected,
        "模块回执行必须是同一句人话＋下一步"
    );
    assert_eq!(
        ui.get_toast_message().as_str(),
        expected,
        "提示条必须是同一句人话＋下一步"
    );
    assert!(
        ui.get_toast_is_error(),
        "拒绝提示条必须是错误档（常驻），不得几秒自动收走"
    );
    assert!(ui.get_show_floating_toast(), "拒绝必须在浮位可见");
    assert!(!ui.get_module_status_succeeded(), "拒绝不得标成成功");
}

/// 落点函数本身（不经手势触发）：三条文案逐条写入同一套错误级落点。
#[test]
fn roll_rejection_lands_as_actionable_error_in_status_and_receipt() {
    let port = build_ui();
    let ui = port.ui();
    let _lang = lock_language(ui, yeban_app::cli::Language::ZhCn);

    // 先造一个"成功/信息"旧态，证明拒绝写点会重置分级，不沿用上一条。
    ui.set_module_status_succeeded(true);
    ui.set_toast_is_error(false);
    ui.set_toast_message("上一步完成".into());
    ui.set_roll_edit_status("上一步完成".into());

    for expected in [NUDGE_ZH, IDENTITY_ZH, BOUNDS_ZH] {
        host::report_roll_edit_rejected(ui, expected);
        assert_actionable_error_landing(ui, expected);
    }
}

/// 边界臂（端到端，几何事实）：127 音高的音符已在第 0 泳道（顶格），
/// 再向上拖一格 ⇒ `note_move_delta_for` 把纵向坐标夹回泳道内，增量为 0 ⇒
/// 既不提交也不出声：状态行留空、提交 0 次、工程一位不动。
/// 这条判据钉住的是\"边界臂拖不出来\"本身（见文件头），不是边界文案的落点 ——
/// 落点由 `roll_rejection_lands_as_actionable_error_in_status_and_receipt` 覆盖。
#[test]
fn dragging_the_top_note_further_up_is_a_no_op_by_lane_clamp() {
    let (project, ulid) = project_with_top_note();
    let (window, port, mut plane) = assemble_with_project(project);
    let before = port.project();
    let before_bytes = serde_json::to_vec(&before).expect("序列化起点工程");
    let commits_before = port.display().commit_count;
    assert_eq!(
        note_by_ulid(&before, &ulid).1,
        127,
        "夹具前提：靶子音符确为 127 音高"
    );

    window.set_roll_base_pitch(BOUNDS_BASE_PITCH);
    let view = yeban_app::bridge::ViewState::from_project(&before).expect("投影");
    let bounds = element_bounds(&mut plane, 1, SCROLL_AREA);
    let viewport_width = bounds["width"].as_f64().expect("width") as f32;
    let viewport_height = bounds["height"].as_f64().expect("height") as f32;
    let painted_center = a_painted_note_at_base(
        &view,
        viewport_width,
        viewport_height,
        0.0,
        BOUNDS_BASE_PITCH as u8,
        &ulid,
    );

    // 向上拖恰好一格泳道（-14px）：已在顶格，夹紧后增量为 0。
    drag_in_roll_by_id_at(
        &mut plane,
        10,
        painted_center,
        0.0,
        -LANE_HEIGHT,
        DRAG_STEPS,
    );
    assert_eq!(
        window.get_roll_edit_status().as_str(),
        "",
        "顶格再往上拖不得出声（增量为 0 的空手势）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "空手势不得留提交"
    );
    let after = port.project();
    assert_eq!(
        serde_json::to_vec(&after).expect("序列化事后工程"),
        before_bytes,
        "空手势不得改工程"
    );
    assert_eq!(note_by_ulid(&after, &ulid).1, 127, "靶子音符仍在 127 音高");
}

/// 边界臂的模型前提（不经手势）：一条 `Op::MoveNote` 把 127 音符再推高一格 ⇒
/// 模型前置条件 `PitchOutOfRange` 拒，提交返回 Err，工程一位不动。
/// 这是 `commit_note_drag` 里边界臂那句 `report_roll_edit_rejected` 的触发条件；
/// 指针拖动本身到不了这里（见上一条判据）。
#[test]
fn an_out_of_range_move_note_is_rejected_by_the_model() {
    let (project, ulid) = project_with_top_note();
    let (window, port, _plane) = assemble_with_project(project);
    let before = port.project();
    let before_bytes = serde_json::to_vec(&before).expect("序列化起点工程");
    let commits_before = port.display().commit_count;

    let (track_id, clip_id) = before
        .tracks
        .iter()
        .flat_map(|(track_id, track)| track.clips.values().map(move |p| (*track_id, p.clip_id)))
        .next()
        .expect("夹具工程里必须有一条摆放");
    let note_id: yeban_model::EntityId = ulid.parse().expect("ULID 可解析");
    let op = yeban_model::ops::Op::MoveNote {
        track_id,
        clip_id,
        note_id,
        delta_tick: 0,
        delta_pitch: 1,
    };
    let result = port.commit_ops(NOW_MS, "probe: move 127 note up one", vec![op]);
    assert!(
        result.is_err(),
        "127 音符再 +1 半音必须被模型拒绝，实际提交成功了"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "被拒提交不得留提交"
    );
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化事后工程"),
        before_bytes,
        "被拒提交不得改工程"
    );
    assert_eq!(
        window.get_roll_edit_status().as_str(),
        "",
        "直接经端口提交不走 UI 拒绝落点（宿主只在拖动提交臂里写状态行）"
    );
}

/// 身份臂（端到端）：拖动中途删掉指针下那颗音符再松手 ⇒ 身份解析不到 ⇒
/// 身份拒绝文案进错误级落点；只有删除那一次提交，拖动本身 0 提交。
#[test]
fn dragging_a_note_deleted_mid_drag_reports_the_identity_next_step() {
    let (window, port, mut plane) = assemble();
    let _lang = lock_language(&window, yeban_app::cli::Language::ZhCn);
    let before = port.project();
    let commits_before = port.display().commit_count;
    let target = target_note(&window, &mut plane, &before, 1, 0.0);
    let before_notes = notes(&before);

    let origin = pointer_down_in_roll(&mut plane, 10, target.painted_center);
    // 拖动中途：经端口直接删掉指针下那颗音符（一次提交），拖动状态留在 UI 属性里。
    let ops = delete_ops_for(&port.project(), std::slice::from_ref(&target.ulid));
    assert_eq!(ops.len(), 1, "删除夹具音符必须恰好产一条 op");
    port.commit_ops(NOW_MS, "delete test note", ops)
        .expect("删除夹具音符");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "中途删除恰好是一次提交"
    );
    assert!(
        !notes(&port.project())
            .iter()
            .any(|(id, _, _)| id == &target.ulid),
        "靶子音符必须已被删掉"
    );
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        origin,
        target.painted_center,
        16.0,
        -LANE_HEIGHT * 2.0,
        DRAG_STEPS,
    );
    pointer_up_in_roll(&mut plane, 30);
    assert_actionable_error_landing(&window, IDENTITY_ZH);
    assert!(
        window
            .get_module_status_text()
            .contains("重新选择音符后再拖动"),
        "身份文案必须给重新选择的下一步"
    );

    // 拖动本身 0 提交：提交计数仍是删除那一次；其余音符一位不动。
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "被拒拖动不得再留提交"
    );
    let after_notes = notes(&port.project());
    assert_eq!(
        after_notes.len(),
        before_notes.len() - 1,
        "只少删掉的那一颗"
    );
    for note in &after_notes {
        assert!(
            before_notes.contains(note),
            "其余音符必须一位不动: {note:?}"
        );
    }
}

/// 成功清理：一次拒绝之后再拖成功一颗 ⇒ 卷帘状态行收掉拒绝文案（中文前缀分支）。
#[test]
fn a_successful_drag_after_a_rejection_clears_the_status_row() {
    let (window, port, mut plane) = assemble();
    let _lang = lock_language(&window, yeban_app::cli::Language::ZhCn);
    let before = port.project();
    let target = target_note(&window, &mut plane, &before, 1, 0.0);

    // 先造一次身份拒绝（与身份臂同一手法）。
    let origin = pointer_down_in_roll(&mut plane, 10, target.painted_center);
    let ops = delete_ops_for(&port.project(), std::slice::from_ref(&target.ulid));
    port.commit_ops(NOW_MS, "delete test note", ops)
        .expect("删除夹具音符");
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        origin,
        target.painted_center,
        16.0,
        -LANE_HEIGHT * 2.0,
        DRAG_STEPS,
    );
    pointer_up_in_roll(&mut plane, 30);
    assert_eq!(
        window.get_roll_edit_status().as_str(),
        IDENTITY_ZH,
        "拒绝后状态行必须挂着身份文案"
    );

    // 再拖成功另一颗（向下半音，合法）：状态行收掉，工程多一步成功提交。
    let after_delete = port.project();
    let second = target_note(&window, &mut plane, &after_delete, 40, 0.0);
    assert_ne!(second.ulid, target.ulid, "第二颗靶子不得是已删的那颗");
    let commits_before_success = port.display().commit_count;
    drag_in_roll_by_id_at(
        &mut plane,
        50,
        second.painted_center,
        0.0,
        LANE_HEIGHT,
        DRAG_STEPS,
    );
    assert_eq!(
        port.display().commit_count,
        commits_before_success + 1,
        "成功拖动必须留一次提交"
    );
    assert_eq!(
        note_by_ulid(&port.project(), &second.ulid).1,
        second.before.1 - 1,
        "第二颗音符必须真的向下挪了一格"
    );
    assert_eq!(
        window.get_roll_edit_status().as_str(),
        "",
        "成功后拒绝文案必须从状态行收掉"
    );
}

/// 成功清理（非中文）：英文界面上一次身份拒绝之后再拖成功一颗 ⇒
/// 状态行同样收掉（九语文案的逐字比对分支，不是中文前缀分支）。
#[test]
fn a_successful_drag_after_a_rejection_clears_the_status_row_in_english() {
    use yeban_app::cli::Language;
    let (window, port, mut plane) = assemble();
    let _lang = lock_language(&window, Language::EnUs);
    assert_eq!(
        host::current_language(),
        Language::EnUs,
        "判据前提：界面语言必须真的切到英文"
    );

    let before = port.project();
    let target = target_note(&window, &mut plane, &before, 1, 0.0);

    // 先造一次身份拒绝（与身份臂同一手法），文案必须是英文。
    let origin = pointer_down_in_roll(&mut plane, 10, target.painted_center);
    let ops = delete_ops_for(&port.project(), std::slice::from_ref(&target.ulid));
    port.commit_ops(NOW_MS, "delete test note", ops)
        .expect("删除夹具音符");
    pointer_drag_steps_in_roll(
        &mut plane,
        20,
        origin,
        target.painted_center,
        16.0,
        -LANE_HEIGHT * 2.0,
        DRAG_STEPS,
    );
    pointer_up_in_roll(&mut plane, 30);
    assert_eq!(
        window.get_roll_edit_status().as_str(),
        IDENTITY_EN,
        "英文界面上的拒绝必须是英文文案"
    );

    // 再拖成功另一颗（向下半格，合法）：成功，状态行收掉。
    let after_delete = port.project();
    let second = target_note(&window, &mut plane, &after_delete, 40, 0.0);
    assert_ne!(second.ulid, target.ulid, "第二颗靶子不得是已删的那颗");
    let commits_before_success = port.display().commit_count;
    drag_in_roll_by_id_at(
        &mut plane,
        50,
        second.painted_center,
        0.0,
        LANE_HEIGHT,
        DRAG_STEPS,
    );
    assert_eq!(
        port.display().commit_count,
        commits_before_success + 1,
        "成功拖动必须留一次提交"
    );
    assert_eq!(
        note_by_ulid(&port.project(), &second.ulid).1,
        second.before.1 - 1,
        "第二颗音符必须真的向下挪了一格"
    );
    assert_eq!(
        window.get_roll_edit_status().as_str(),
        "",
        "英文拒绝文案也必须在成功后从状态行收掉"
    );
    host::apply_language(&window, Language::ZhCn);
}
