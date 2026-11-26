//! T-FIX-490：整组拖动的边界夹紧必须**按整组**算，⛔ 不能逐成员饱和。
//!
//! ## 缺陷（issue `ISS-MWWUPCRT-666BD9`）
//!
//! 旧实现在 `commit_clip_drag` 里对每个成员各算各的
//! `member_old.saturating_add_signed(delta_ticks)`：触到时间轴 0 的成员被夹住、
//! 没触到的照常移 ⇒ 组内相对间距被**压扁**；纵向同理（越界成员留原轨、其余换轨）
//! ⇒ 松手后整组变形。
//!
//! ## 本文件钉住的四句话
//!
//! | # | 场景 | 断言 |
//! | :--- | :--- | :--- |
//! | ① | 起点 0/960，抓**右边那条**往左拽 −200px（远超 0） | 结果是 **0/960**（间距 960 保持），⛔ 不是 0/0 |
//! | ② | 起点 400/1360，同样往左拽 −200px | 整组**一起**夹到 **0/960**：能移就移，间距仍 960 |
//! | ③ | 两条在 1/2 行，抓右边那条往上拖两行（最左成员会越出第 0 行） | 整组**一条都不换轨**、起点不变，且有可见回执 |
//! | ④ | 单条拖动（选中集只有它） | 起点照常 +1200 tick（组逻辑不回归单条） |
//!
//! ## 为什么用「抓右边那条」
//!
//! 位移是从**按下那条**的吸附落点反推的：抓最左那条时它自己先被 `clip_drag_target_tick`
//! 夹到 0，delta 已经合法，整组自然同步 —— 旧实现也"看起来正确"。
//! 缺陷只在**按下那条不是最左**时暴露：它自己能移到 0，左边那条却被饱和夹在原地。
//! 所以 ①② 都抓右边的 bass。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，`Tier1Window::install` 每线程只能成功一次
//! ⇒ 每个 `#[test]` 在自己的线程里各装一次（与 `tests/non_utf8_project_paths.rs` 同款）。
//! 行几何（`track-ys` / `track-heights`）必须由**真实布局**填出来，所以用 Tier-1 真窗口 +
//! 一次 `resize`，⛔ 不手写几何。

use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host::{self, roll_ticks_per_pixel};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;

/// filled 样本里两条摆放的身份（`samples.rs` 的 `fixture_id(50)` / `fixture_id(51)`）。
fn lead_placement(project: &YebanProjectV1) -> (yeban_model::EntityId, String) {
    let (id, _) = project
        .tracks
        .values()
        .find_map(|track| track.clips.keys().next().map(|id| (*id, track.id)))
        .expect("filled 样本必须有 lead 摆放");
    (id, id.to_canonical_string())
}

/// 第二条摆放（bass 轨那条）。
fn bass_placement(project: &YebanProjectV1) -> (yeban_model::EntityId, String) {
    let mut found = project
        .tracks
        .values()
        .filter_map(|track| track.clips.keys().next().map(|id| (*id, track.id)))
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 2, "filled 样本应恰有两条摆放");
    let (id, _) = found.pop().expect("第二条");
    (id, id.to_canonical_string())
}

fn set_start(project: &mut YebanProjectV1, placement: yeban_model::EntityId, start_tick: u64) {
    for track in project.tracks.values_mut() {
        if let Some(placement) = track.clips.get_mut(&placement) {
            placement.start_tick = start_tick;
            return;
        }
    }
    panic!("找不到摆放");
}

fn start_of(project: &YebanProjectV1, placement: yeban_model::EntityId) -> u64 {
    for track in project.tracks.values() {
        if let Some(placement) = track.clips.get(&placement) {
            return placement.start_tick;
        }
    }
    panic!("找不到摆放");
}

/// 某条摆放当前所在的行号（行序 = 界面 `track-ids` 的顺序）。
fn row_of(
    ui: &yeban_app::ui::MainWindow,
    project: &YebanProjectV1,
    placement_id: yeban_model::EntityId,
) -> usize {
    let track_id = project
        .tracks
        .iter()
        .find_map(|(track_id, track)| track.clips.contains_key(&placement_id).then_some(*track_id))
        .expect("摆放必须有宿主轨");
    let ids: Vec<String> = ui.get_track_ids().iter().map(|id| id.to_string()).collect();
    ids.iter()
        .position(|id| id == &track_id.to_canonical_string())
        .unwrap_or_else(|| panic!("轨道 {track_id} 必须出现在界面 track-ids 里：{ids:?}"))
}

/// 第 `row` 行的纵向中心（真实布局给的值）。
fn row_center(ui: &yeban_app::ui::MainWindow, row: usize) -> f32 {
    let tops: Vec<f32> = ui.get_track_ys().iter().collect();
    let heights: Vec<f32> = ui.get_track_heights().iter().collect();
    assert_eq!(tops.len(), heights.len(), "行顶与行高必须成对");
    assert!(row < tops.len(), "行号 {row} 越界（共 {} 行）", tops.len());
    tops[row] + heights[row] / 2.0
}

/// `[string]` 属性的入参（与 `host::strings` 同一种形状）。
fn ulids(values: &[String]) -> slint::ModelRc<slint::SharedString> {
    slint::ModelRc::new(slint::VecModel::from(
        values
            .iter()
            .map(|value| slint::SharedString::from(value.as_str()))
            .collect::<Vec<_>>(),
    ))
}

fn assemble(
    project: YebanProjectV1,
) -> (
    LivePort<yeban_app::ui::MainWindow>,
    yeban_app::ui::MainWindow,
    Rc<UndoPort>,
) {
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<group-drag>", "yeban-app", project, NOW_MS).expect("打开会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let ui = slint::ComponentHandle::clone_strong(live.ui());
    host::wire_clip_drag(&ui, &port);
    (live, ui, port)
}

/// 一次「按下 → 松手」：`dx_px` / `dy_px` 是相对按下点的位移（逻辑像素）。
fn drag(ui: &yeban_app::ui::MainWindow, ulid: &str, x: f32, y: f32, dx_px: f32, dy_px: f32) {
    let grabbed = ui.invoke_clip_drag_grab(ulid.into(), x, y);
    assert!(grabbed, "按下必须被接受（{ulid}）");
    ui.invoke_clip_drag_release(ulid.into(), x + dx_px, y + dy_px);
    assert!(ui.get_clip_drag_ulid().is_empty(), "松手后草稿身份必须清零");
}

/// ⭐ ①：起点 0/960，抓右边那条往左拽 −200px ⇒ 间距 960 保持（0/960）。
#[test]
fn a_group_drag_that_hits_the_timeline_start_keeps_its_spacing() {
    let mut project = yeban_model::samples::filled_project();
    let (lead_id, _) = lead_placement(&project);
    let (bass_id, bass_ulid) = bass_placement(&project);
    set_start(&mut project, lead_id, 0);
    set_start(&mut project, bass_id, 960);

    let (_live, ui, port) = assemble(project);
    let row = row_of(&ui, &port.project(), bass_id);
    let y = row_center(&ui, row);
    ui.set_selected_clip_ulids(ulids(&[lead_id.to_canonical_string(), bass_ulid.clone()]));

    // −200px × 30 tick/px = −6000 tick：远超 960 的可移动量 ⇒ 旧实现会把左边那条饱和在 0。
    drag(&ui, &bass_ulid, 500.0, y, -200.0, 0.0);

    let after = port.project();
    assert_eq!(
        start_of(&after, lead_id),
        0,
        "左边那条仍在 0（它本来就贴着边界）"
    );
    assert_eq!(
        start_of(&after, bass_id),
        960,
        "间距必须保持 960：整组被夹在边界外 ⇒ **一条都不动**，⛔ 不是 0/0"
    );
}

/// ⭐ ②：起点 400/1360，同样拽 −200px ⇒ 整组一起夹到 0/960（能移就移，间距不变）。
#[test]
fn a_group_drag_is_clamped_as_one_rigid_body() {
    let mut project = yeban_model::samples::filled_project();
    let (lead_id, _) = lead_placement(&project);
    let (bass_id, bass_ulid) = bass_placement(&project);
    set_start(&mut project, lead_id, 400);
    set_start(&mut project, bass_id, 1360);

    let (_live, ui, port) = assemble(project);
    let row = row_of(&ui, &port.project(), bass_id);
    let y = row_center(&ui, row);
    ui.set_selected_clip_ulids(ulids(&[lead_id.to_canonical_string(), bass_ulid.clone()]));

    let commits_before = port.display().commit_count;
    drag(&ui, &bass_ulid, 500.0, y, -200.0, 0.0);

    let after = port.project();
    let lead_after = start_of(&after, lead_id);
    let bass_after = start_of(&after, bass_id);
    assert_eq!(
        (lead_after, bass_after),
        (0, 960),
        "整组一起夹到时间轴起点：间距仍是 960，⛔ 不是逐成员饱和后的 (0, 0)"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次拖动恰好一次提交"
    );
}

/// ⭐ ③：组内有成员会越出行范围 ⇒ 整组不换轨、起点不变，且有可见回执。
#[test]
fn a_blocked_row_move_keeps_the_whole_group_in_place_and_says_so() {
    let mut project = yeban_model::samples::filled_project();
    let (lead_id, lead_ulid) = lead_placement(&project);
    let (bass_id, bass_ulid) = bass_placement(&project);
    set_start(&mut project, lead_id, 0);
    set_start(&mut project, bass_id, 960);

    let (_live, ui, port) = assemble(project);
    let lead_row = row_of(&ui, &port.project(), lead_id);
    let bass_row = row_of(&ui, &port.project(), bass_id);
    assert!(bass_row > 0, "夹具前提：bass 必须在第 0 行之下");
    let y = row_center(&ui, bass_row);
    ui.set_selected_clip_ulids(ulids(&[lead_ulid, bass_ulid.clone()]));

    // 往上拖得比"最左成员到第 0 行"还多一行 ⇒ 该成员必然越界。
    let up_rows = i32::try_from(lead_row + 1).expect("行号");
    let target_row = bass_row.saturating_sub(usize::try_from(up_rows).unwrap_or(0));
    let dy = row_center(&ui, target_row.min(bass_row)) - y;
    // `track_index_at_y` 对越界 y 会夹到首行，所以直接用"首行中心"作为落点。
    let dy_to_first_row = row_center(&ui, 0) - y;
    let _ = dy;

    let tracks_before = track_signature(&port.project(), lead_id, bass_id);
    let commits_before = port.display().commit_count;
    drag(&ui, &bass_ulid, 500.0, y, 0.0, dy_to_first_row);

    let after = port.project();
    assert_eq!(
        track_signature(&after, lead_id, bass_id),
        tracks_before,
        "整组必须一条都不换轨（⛔ 不是一条换轨一条不动）"
    );
    assert_eq!(start_of(&after, lead_id), 0, "被拦下时起点不得变");
    assert_eq!(start_of(&after, bass_id), 960, "被拦下时起点不得变");
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "被拦下时不得提交空 op"
    );
    assert!(
        !ui.get_module_status().is_empty() || !ui.get_module_status_text().is_empty(),
        "被拦下必须给可见回执（module-status 三件套），⛔ 不静默"
    );
}

/// 两条摆放各自的宿主轨（用于判"换没换轨"）。
fn track_signature(
    project: &YebanProjectV1,
    first: yeban_model::EntityId,
    second: yeban_model::EntityId,
) -> (Option<yeban_model::EntityId>, Option<yeban_model::EntityId>) {
    let find = |placement: yeban_model::EntityId| {
        project
            .tracks
            .iter()
            .find_map(|(track_id, track)| track.clips.contains_key(&placement).then_some(*track_id))
    };
    (find(first), find(second))
}

/// ⭐ ④：单条拖动不回归（选中集只有它 ⇒ 组逻辑退化成单条）。
#[test]
fn a_single_clip_drag_still_moves_exactly_as_before() {
    let mut project = yeban_model::samples::filled_project();
    let (lead_id, lead_ulid) = lead_placement(&project);
    set_start(&mut project, lead_id, 0);

    let (_live, ui, port) = assemble(project);
    let row = row_of(&ui, &port.project(), lead_id);
    let y = row_center(&ui, row);
    ui.set_selected_clip_ulids(ulids(std::slice::from_ref(&lead_ulid)));

    let tpp = roll_ticks_per_pixel(&ui);
    let expected: u64 = 40 * tpp;
    let commits_before = port.display().commit_count;
    drag(&ui, &lead_ulid, 500.0, y, 40.0, 0.0);

    let after = port.project();
    assert_eq!(
        start_of(&after, lead_id),
        yeban_app::bridge::snap_tick(expected, yeban_app::host::ROLL_SNAP_GRID_TICKS),
        "单条拖动必须照常落在吸附后的目标（40px × {tpp}）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "单条拖动恰好一次提交"
    );
}
