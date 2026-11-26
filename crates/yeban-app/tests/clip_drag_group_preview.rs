//! **T-FIX-491**：编曲区**整组拖动预览**必须跟着走，⛔ 不只是按下的那一条。
//!
//! ## 修前的病
//!
//! 松手提交早已是整组的（`host::commit_clip_drag`：组里每条各发一条
//! `MoveClipPlacement`，**同一 delta**、一次提交、一步撤销），但**拖动期的预览**
//! 只认 `clip_ulid == clip-drag-ulid` 那一条（`ui/arrangement.slint` 的块 x 与拖动面
//! 反向抵消两处）⇒ 组内其他成员停在原位，松手才跳变，预览与提交不是一回事。
//!
//! ## 判据
//!
//! | # | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① 整组横移 | 选中两条（起点差 960 tick）拖 40px（=1200 tick，5 个吸附格）⇒ 拖动期**两条**包头都右移同一量 | 预览不再是"只动按下的那条" |
//! | ② 组内相对位置不变 | 两条包头 x 的**差**在拖动期与拖动前逐值相同 | 同 delta（⛔ 不是逐条吸附把组拉散） |
//! | ③ 松手与预览一致 | 松手后两条包头 x == 拖动期最后一次预览的 x；工程里两条起点都 +1200 | 预览不是"画着玩"，提交落在看到的位置 |
//! | ④ 单条不回归 | 只选一条拖 ⇒ `clip-drag-is-group == false`，另一条包头 x 一位不动 | 单条拖动的旧行为逐像素不变 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部且每线程只能装一次 ⇒ 每个 `#[test]` 在自己的
//! 线程里各装配一次（与 `tests/clip_drag.rs` / `tests/clip_selection.rs` 同款）。
//!
//! ⚠ 票面举例的 "间距 960 / 拖 200 tick" 是巡查侧的口径；本仓默认横向缩放
//! `DEFAULT_TPP = 30`、吸附网格 `ROLL_SNAP_GRID_TICKS = 240` ⇒ 判据用 **40px = 1200 tick**
//! （5 格，手算可核），间距取 **960 tick**（两条相邻片段，前一条 960 长）。结论与票面同构：
//! 组内同 delta、间距保持、预览与提交一致。

#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, build_live_ui};
use serde_json::json;
use slint::Model as _;
use yeban_app::host::{apply_clip_selection, wire_clip_drag};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_model::{ClipPlacement, EntityId, LoopConfig, TrackKind, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_700_000_000_000;
/// 一次拖动注入的水平位移（窗口逻辑像素）。40px × 30 tick/px = 1200 tick = 5 格。
const DRAG_DX_PX: f64 = 40.0;
const DRAG_STEPS: usize = 4;
/// 两条片段的起点差（tick），与票面同量级：前一条 960 长 ⇒ 首尾相接。
const GAP_TICKS: u64 = 960;
const CLIP_TICKS: u64 = 960;
/// 期望的提交位移（tick）：40px × 30 = 1200。
const EXPECTED_DELTA_TICKS: u64 = 1200;

/// 夹具：Lead（MIDI 轨）上放两条首尾相接的片段（起点 0 与 960，各 960 tick），
/// 复用工程里已有的 `clip_id`（⛔ 不造假资产）。
fn fixture() -> (YebanProjectV1, String, String) {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    let lead_id = project
        .tracks
        .iter()
        .find(|(_, track)| track.kind == TrackKind::Midi)
        .map(|(id, _)| *id)
        .expect("filled 样本必须有 MIDI 轨");
    let clip_id = project.tracks[&lead_id]
        .clips
        .values()
        .next()
        .map(|placement| placement.clip_id)
        .expect("MIDI 轨必须已有片段（复用它的 clip）");
    let track = project.tracks.get_mut(&lead_id).expect("MIDI 轨");
    track.clips.clear();
    let mut make = |start_tick: u64| {
        let id = EntityId::new();
        track.clips.insert(
            id,
            ClipPlacement {
                id,
                clip_id,
                start_tick,
                duration_ticks: CLIP_TICKS,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
                fade_in_ticks: None,
                fade_out_ticks: None,
            },
        );
        id.to_canonical_string()
    };
    let first = make(0);
    let second = make(GAP_TICKS);
    (project, first, second)
}

fn assemble(
    project: YebanProjectV1,
) -> (yeban_app::ui::MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let ui = build_live_ui(&project, Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<clip-drag-group>", "yeban-app", project, NOW_MS).expect("打开"),
    ));
    wire_clip_drag(ui.ui(), &port);
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 语义 ID（`[UI-TEST-001]`）在运行时控件树里的包围盒 `(x, y, w, h)`。
fn element_bounds(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
) -> (i32, i32, u32, u32) {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时控件树里: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    (
        i32::try_from(bounds["x"].as_i64().expect("x")).expect("x 在 i32 内"),
        i32::try_from(bounds["y"].as_i64().expect("y")).expect("y 在 i32 内"),
        u32::try_from(bounds["width"].as_u64().expect("width")).expect("width 在 u32 内"),
        u32::try_from(bounds["height"].as_u64().expect("height")).expect("height 在 u32 内"),
    )
}

fn header_x(plane: &mut LiveControlPlane, seq: i64, ulid: &str) -> i32 {
    element_bounds(plane, seq, &format!("clip-{ulid}-header")).0
}

/// 在拖动面中心按下，返回绝对窗口按下点。
fn pointer_down_at_body(plane: &mut LiveControlPlane, seq: i64, ulid: &str) -> (f64, f64) {
    let id = format!("clip-{ulid}-body");
    let (x, y, width, height) = element_bounds(plane, seq, &id);
    let x_offset = f64::from(width) / 2.0;
    let y_offset = f64::from(height) / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    (f64::from(x) + x_offset, f64::from(y) + y_offset)
}

fn pointer_drag_right(plane: &mut LiveControlPlane, seq: i64, origin: (f64, f64)) {
    for step in 1..=DRAG_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f64 / DRAG_STEPS as f64;
        let x = origin.0 + DRAG_DX_PX * fraction;
        let moved = plane.plane().try_line(&request_line(
            seq + i64::try_from(step).expect("步号"),
            METHOD_DISPATCH_POINTER_MOVE,
            Some(json!({ "x": x, "y": origin.1 })),
        ));
        assert!(
            !moved.is_error(),
            "第 {step} 步移动必须落到真实窗口: {moved:?}"
        );
    }
}

fn pointer_up(plane: &mut LiveControlPlane, seq: i64) {
    let up = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

fn start_tick(project: &YebanProjectV1, ulid: &str) -> Option<u64> {
    let id = ulid.parse::<EntityId>().ok()?;
    project
        .tracks
        .values()
        .find_map(|track| track.clips.get(&id).map(|placement| placement.start_tick))
}

/// ⭐ 主判据：整组拖动时**两条都**预览到位，间距不变，松手落点与预览一致。
#[test]
fn a_group_drag_previews_every_member_and_lands_where_it_previewed() {
    let (project, first, second) = fixture();
    let (ui, port, mut plane) = assemble(project);

    // 选区 = 两条（主选中取第一条，与单击/框选同一条落点函数）。
    apply_clip_selection(&ui, &[first.clone(), second.clone()], &first);
    assert!(
        !ui.get_clip_drag_is_group(),
        "没有手势时整组标志必须是 false（⛔ 不许残留上一次的组）"
    );

    let first_before = header_x(&mut plane, 1, &first);
    let second_before = header_x(&mut plane, 2, &second);
    let gap_before = second_before - first_before;
    assert!(
        gap_before > 0,
        "夹具两条片段必须按起点排序（first 在前）：{first_before} / {second_before}"
    );

    let origin = pointer_down_at_body(&mut plane, 10, &first);
    pointer_drag_right(&mut plane, 20, origin);

    // ① 拖动期：整组标志为真 + 两条都移动。
    assert!(
        ui.get_clip_drag_is_group(),
        "选区含按下项 ⇒ 这一拖必须是整组（clip-drag-is-group 必须为 true）"
    );
    assert!(
        ui.get_clip_drag_preview_dx() > 0.0,
        "拖动期必须有非零预览位移（读到 {}）",
        ui.get_clip_drag_preview_dx()
    );
    let first_during = header_x(&mut plane, 30, &first);
    let second_during = header_x(&mut plane, 31, &second);

    // ② 组内相对位置不变（同 delta）。
    assert_eq!(
        second_during - first_during,
        gap_before,
        "拖动期两条的间距必须保持（{first_during} / {second_during}，拖动前差 {gap_before}）"
    );
    // ① 两条都右移同一量（40px）。允许 ±1px：投影/取整各一次。
    let moved_first = first_during - first_before;
    let moved_second = second_during - second_before;
    assert!(
        (moved_first - 40).abs() <= 1,
        "按下那条只右移了 {moved_first}px（应为 40px）"
    );
    assert!(
        (moved_second - 40).abs() <= 1,
        "组内另一条只右移了 {moved_second}px（应为 40px）：预览仍只动按下的那一条（T-FIX-491 回退）"
    );

    // ③ 松手：落点与预览一致。
    pointer_up(&mut plane, 40);
    assert!(
        !ui.get_clip_drag_is_group(),
        "松手后整组标志必须清零（草稿与标志同生同灭）"
    );
    assert_eq!(ui.get_clip_drag_preview_dx(), 0.0, "松手后预览位移必须清零");
    let first_after = header_x(&mut plane, 41, &first);
    let second_after = header_x(&mut plane, 42, &second);
    assert_eq!(
        first_after, first_during,
        "松手后按下那条的 x 必须等于拖动期最后预览的 x（预览与提交一致）"
    );
    assert_eq!(
        second_after, second_during,
        "松手后组内另一条的 x 必须等于拖动期最后预览的 x（预览与提交一致）"
    );

    let project = port.project();
    let first_start = start_tick(&project, &first).expect("第一条仍在");
    let second_start = start_tick(&project, &second).expect("第二条仍在");
    assert_eq!(
        first_start, EXPECTED_DELTA_TICKS,
        "按下那条的起点必须 +{EXPECTED_DELTA_TICKS} tick"
    );
    assert_eq!(
        second_start,
        GAP_TICKS + EXPECTED_DELTA_TICKS,
        "组内另一条的起点必须**同 delta** +{EXPECTED_DELTA_TICKS} tick（{GAP_TICKS}+{EXPECTED_DELTA_TICKS}）"
    );
    assert_eq!(
        second_start - first_start,
        GAP_TICKS,
        "提交后两条间距必须仍是 {GAP_TICKS} tick"
    );
}

/// ⭐ 反向腿（同一文件内的行为对照）：只选一条 ⇒ 整组标志为假，另一条一位不动。
#[test]
fn a_single_clip_drag_still_previews_and_lands_alone() {
    let (project, first, second) = fixture();
    let (ui, port, mut plane) = assemble(project);

    apply_clip_selection(&ui, std::slice::from_ref(&first), &first);
    let first_before = header_x(&mut plane, 1, &first);
    let second_before = header_x(&mut plane, 2, &second);

    let origin = pointer_down_at_body(&mut plane, 10, &first);
    pointer_drag_right(&mut plane, 20, origin);

    assert!(
        !ui.get_clip_drag_is_group(),
        "只选一条 ⇒ 不是整组拖动（clip-drag-is-group 必须为 false）"
    );
    let first_during = header_x(&mut plane, 30, &first);
    let second_during = header_x(&mut plane, 31, &second);
    assert!(
        (first_during - first_before - 40).abs() <= 1,
        "单条拖动的按下那一条必须照旧右移 40px（实测 {}px）",
        first_during - first_before
    );
    assert_eq!(
        second_during, second_before,
        "单条拖动时**另一条**必须一位不动（实测 {} → {}）",
        second_before, second_during
    );

    pointer_up(&mut plane, 40);
    let project = port.project();
    assert_eq!(
        start_tick(&project, &first),
        Some(EXPECTED_DELTA_TICKS),
        "单条拖动仍要提交（起点 +{EXPECTED_DELTA_TICKS}）"
    );
    assert_eq!(
        start_tick(&project, &second),
        Some(GAP_TICKS),
        "单条拖动不许动到别的片段（第二条起点必须原样 {GAP_TICKS}）"
    );
}

/// 从按下点分 `DRAG_STEPS` 步向下拖到目标 y（**绝对窗口坐标**注入）。
fn pointer_drag_down(plane: &mut LiveControlPlane, seq: i64, origin: (f64, f64), target_y: f64) {
    for step in 1..=DRAG_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f64 / DRAG_STEPS as f64;
        let y = origin.1 + (target_y - origin.1) * fraction;
        let moved = plane.plane().try_line(&request_line(
            seq + i64::try_from(step).expect("步号"),
            METHOD_DISPATCH_POINTER_MOVE,
            Some(json!({ "x": origin.0, "y": y })),
        ));
        assert!(
            !moved.is_error(),
            "第 {step} 步纵向移动必须落到真实窗口: {moved:?}"
        );
    }
}

fn clip_track_of(project: &YebanProjectV1, ulid: &str) -> Option<EntityId> {
    let id = ulid.parse::<EntityId>().ok()?;
    project
        .tracks
        .iter()
        .find_map(|(track_id, track)| track.clips.contains_key(&id).then_some(*track_id))
}

/// ⭐ 跨轨：整组**共用**一条落点金线，松手后两条一起换到那一行（组语义与提交一致）。
///
/// ⚠ 如实：块本体在拖动期不做纵向位移预览（活视图的块嵌在各自轨道子树里，
/// 纵向目标一直由 T-FIX-209 的落点金线表达）；本判据钉的是"金线指示的目标行 ==
/// 松手后两条落到的行"，即**纵向预览与提交一致**。
#[test]
fn a_group_drag_across_tracks_targets_one_line_and_lands_both_members_there() {
    let (project, first, second) = fixture();
    let (ui, port, mut plane) = assemble(project);

    apply_clip_selection(&ui, &[first.clone(), second.clone()], &first);

    let track_ids: Vec<String> = ui.get_track_ids().iter().map(|id| id.to_string()).collect();
    let project_before = port.project();
    let from_track = clip_track_of(&project_before, &first).expect("第一条有轨");
    let from_row = track_ids
        .iter()
        .position(|id| id == &from_track.to_canonical_string())
        .expect("第一条的轨在 track-ids 里");
    assert!(
        from_row + 1 < track_ids.len(),
        "夹具下面必须还有一轨（否则没有'跨轨'可拖）"
    );
    let to_row = from_row + 1;
    let target_track = track_ids[to_row]
        .parse::<EntityId>()
        .expect("目标轨身份可解析");

    let target_header = element_bounds(&mut plane, 11, &format!("track-{to_row}-header"));
    let target_center_y = f64::from(target_header.1) + f64::from(target_header.3) / 2.0;
    let second_before = header_x(&mut plane, 12, &second);

    let origin = pointer_down_at_body(&mut plane, 20, &first);
    pointer_drag_down(&mut plane, 30, origin, target_center_y);

    // ① 拖动期：整组标志为真、落点行 = 下一轨、金线在该行顶沿。
    assert!(ui.get_clip_drag_is_group(), "两条选中 ⇒ 这一拖必须是整组");
    assert_eq!(
        ui.get_clip_drag_preview_track_index(),
        i32::try_from(to_row).expect("行号在 i32 内"),
        "落点行必须是下一轨"
    );
    let tops: Vec<f32> = ui.get_track_ys().iter().collect();
    assert!(
        (ui.get_clip_drag_preview_track_y() - tops[to_row]).abs() < 1e-3,
        "落点 y 必须是目标行顶沿（track-ys[{to_row}] = {}），实测 {}",
        tops[to_row],
        ui.get_clip_drag_preview_track_y()
    );
    let line = element_bounds(&mut plane, 60, "clip-drag-target-line");
    assert_eq!(line.3, 2, "落点线必须是 2px 金线: {line:?}");
    // ② 拖动期不提交（草稿期不碰工程）。
    let during = port.project();
    assert_eq!(
        clip_track_of(&during, &first),
        Some(from_track),
        "拖动期工程里的第一条**不得**换轨"
    );
    // ③ 横向整组预览照旧（纵向拖不改这条口径）。
    let second_during = header_x(&mut plane, 31, &second);
    assert_eq!(
        second_during, second_before,
        "纵向拖动不带横向位移 ⇒ 第二条的 x 一位不动"
    );

    // ④ 松手：两条**一起**落到金线指示的那一行。
    pointer_up(&mut plane, 40);
    let after = port.project();
    assert_eq!(
        clip_track_of(&after, &first),
        Some(target_track),
        "松手后第一条必须落在金线指示的轨道"
    );
    assert_eq!(
        clip_track_of(&after, &second),
        Some(target_track),
        "松手后组内第二条必须**一起**落在同一轨道（整组语义）"
    );
    assert_eq!(
        start_tick(&after, &first),
        Some(0),
        "纵向拖动不带横向位移 ⇒ 起点 tick 不变"
    );
    assert_eq!(
        start_tick(&after, &second),
        Some(GAP_TICKS),
        "纵向拖动不改起点 ⇒ 第二条起点原样"
    );
}
