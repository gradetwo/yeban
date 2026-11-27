// T-FIX-207：编排多选剪辑驱动编辑路径（删除/复制/剪切/静音/分割/拖拽）。
//
// 夹具是 `yeban_model::samples::filled_project()`，它恰好有两条摆放（均未静音）：
//   * MIDI 剪辑 `01J8ZQ00000000000000000050`，轨道 slot 0，[0, 3840)
//   * 音频剪辑 `01J8ZQ00000000000000000051`，轨道 slot 1，[1920, 2880)
// 简报验收写的是“框选 3 条”——该夹具只有 2 条，故验收按“框选 2 条”执行；
// 编辑臂遍历的是整个选中集合，条数由夹具决定，不由实现决定（见 RECEIPT）。
//
// 每条行为测试单独组装一个窗口（slint 平台是线程局部的）。
// 头部说明：按仓库 AGENTS.md §5，Slint 测试本地不编译，以 CI 为准；
// 本地能跑就跑，跑出的结果只是“本地参考”。
//
// 装配与注入口径与 `tests/delete_selected_clip.rs` / `tests/clip_drag.rs` 相同：
// 真窗口 + 撤销端口当装配选项（与产品进程同一形态），指针经 Tier-1 执行面注入。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `tests/delete_selected_clip.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::ComponentHandle as _;
use slint::Model as _;
use yeban_app::host::{
    clip_delete_ops_for_many, clip_mute_ops_for_many, clip_split_ops_for, clip_split_ops_for_many,
    duplicate_clip_placement_ops_for,
};
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_engine::audio_clip::{AudioPcm, mix_into, project_audio_clips};
use yeban_model::ops::Op;
use yeban_model::project::ClipContent;
use yeban_model::samples::filled_project;
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::{
    METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE, METHOD_DISPATCH_POINTER_UP,
    METHOD_NODE,
};
use yeban_ui_test_port::port::Permission;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 框选矩形。x 轴跨度足够覆盖两条剪辑的全部时间线范围
/// （`roll_ticks_per_pixel` 默认 30，3840 ticks ≈ 128px）。
const MARQUEE: (f32, f32, f32, f32) = (0.0, 0.0, 100_000.0, 100_000.0);

/// 一次拖动注入的水平位移（窗口逻辑像素，向右为正）。
/// 40px × 默认缩放 30 = 1200 tick（与 `tests/clip_drag.rs` 同一口径）。
const DRAG_DX_PX: f64 = 40.0;

/// 拖动分几步注入（每步 10px ⇒ 第一步就越过规范的 4px 阈值）。
const DRAG_STEPS: usize = 4;

/// 一个合法但不存在的摆放身份（夹具 ULID 方案：`01J8ZQ` ＋ 20 位十进制序号）。
fn stale_clip_ulid() -> String {
    EntityId::from_str(&format!("01J8ZQ{:020}", 999_999_999_u128))
        .expect("canonical fixture ulid")
        .to_canonical_string()
}

/// 装配：真窗口 ＋ 撤销端口当装配选项传进去（`host::wire_keys` 从第一刻起就握着端口，
/// 与产品进程 `main.rs` 同一形态）＋ 与生产同一条 `wire_clip_scope` / `wire_clip_drag`。
fn assemble() -> (
    MainWindow,
    Rc<UndoPort>,
    LiveControlPlane,
    Rc<std::cell::RefCell<yeban_app::input::InputContext>>,
) {
    let project = filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<clip-multiselect-edit>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开"),
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

/// 项目中全部摆放（id、轨道、摆放），按 start_tick 升序。
fn placements(project: &YebanProjectV1) -> Vec<(String, EntityId, ClipPlacement)> {
    let mut out: Vec<(String, EntityId, ClipPlacement)> = Vec::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            out.push((placement.id.to_canonical_string(), *track_id, *placement));
        }
    }
    out.sort_by_key(|(_, _, p)| p.start_tick);
    out
}

/// 夹具中两条摆放（按 start_tick 升序），供纯函数测试使用。
fn fixture_two(project: &YebanProjectV1) -> Vec<(String, EntityId, ClipPlacement)> {
    let all = placements(project);
    assert_eq!(all.len(), 2, "夹具必须恰好 2 条摆放");
    assert_eq!(all[0].2.start_tick, 0, "第一条摆放从 0 开始");
    assert_eq!(all[1].2.start_tick, 1920, "第二条摆放在 1920 开始");
    all
}

/// 通过 `port.project()` 读回权威工程中的摆放（id、轨道、摆放）。
fn port_placements(port: &UndoPort) -> Vec<(String, EntityId, ClipPlacement)> {
    placements(&port.project())
}

/// 选中集合：`selected-clip-ulids` 属性里的 id 集合。
fn selected(window: &MainWindow) -> Vec<String> {
    window
        .get_selected_clip_ulids()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// 选中主 id。
fn primary(window: &MainWindow) -> String {
    window.get_selected_clip_ulid().to_string()
}

/// 框选两条剪辑，返回它们在夹具中的顺序（id、轨道、摆放）。
fn box_select_two(window: &MainWindow) -> Vec<(String, EntityId, ClipPlacement)> {
    window.invoke_clip_marquee_select(MARQUEE.0, MARQUEE.1, MARQUEE.2, MARQUEE.3, false, false);
    let got = selected(window);
    assert_eq!(got.len(), 2, "框选必须选中 2 条剪辑");
    let want: Vec<String> = placements(&filled_project())
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(got, want, "框选选中的必须是夹具的 2 条摆放");
    assert_eq!(primary(window), want[0], "主选中是集合第一条");
    placements(&filled_project())
}

/// 给窗口发一个带 Ctrl/Cmd 修饰的字母键（`d`/`m`/`t`/`c`/`v`/`x`/`z` 的绑定都要求修饰键）。
fn key_ctrl(window: &MainWindow, text: &str) {
    let handled = window.invoke_key_action(text.into(), false, true, false, false);
    assert!(handled, "窗口应处理按键 {text:?}");
}

/// 给窗口发一个无修饰的 Delete 键（文本 `\u{7f}`）。
fn key_delete(window: &MainWindow) {
    let handled = window.invoke_key_action("\u{7f}".into(), false, false, false, false);
    assert!(handled, "窗口应处理 Delete 键");
}

/// 取某个语义 ID 在运行时控件树里的包围盒（每次都重读 —— 端口里那份会过期）。
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
        "`{element_id}` 必须在运行时控件树里（[UI-TEST-001] 语义 ID 寻址）: {node:?}"
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

/// 按语义 ID 在包围盒中心按下；返回拖动面的绝对窗口原点（后续 `move` 用绝对坐标）。
fn pointer_down_at_center(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
    bounds: (i32, i32, u32, u32),
) -> (f64, f64) {
    let (x, y, width, height) = bounds;
    let x_offset = f64::from(width) / 2.0;
    let y_offset = f64::from(height) / 2.0;
    let down = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    (f64::from(x) + x_offset, f64::from(y) + y_offset)
}

/// 从按下点分 `DRAG_STEPS` 步向右拖 `DRAG_DX_PX`（绝对窗口坐标注入）。
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

/// 松手（`[UI-TEST-002]` §12.4 的口径：位置复用最后一次按下 / 移动）。
fn pointer_up(plane: &mut LiveControlPlane, seq: i64) {
    let up = plane.plane().try_line(&request_line(
        seq,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({ "button": "left" })),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
}

/// 夹具前提：恰好 2 条摆放，起点分别是 0 与 1920。
#[test]
fn the_fixture_has_two_placements_with_known_starts() {
    let project = filled_project();
    let all = fixture_two(&project);
    assert_eq!(all[0].0, "01J8ZQ00000000000000000050", "第一条是 MIDI 剪辑");
    assert_eq!(all[1].0, "01J8ZQ00000000000000000051", "第二条是音频剪辑");
    assert_eq!(all[0].2.duration_ticks, 3840, "第一条时长 3840");
    assert_eq!(all[1].2.duration_ticks, 960, "第二条时长 960");
}

/// ① 框选 2 条 → Delete 删掉 2 条、只提交一次、选中清空；
///    一次 Ctrl+Z 恢复全部（按摆放逐字比对）。
#[test]
fn box_select_two_then_delete_removes_two_and_one_undo_restores_them() {
    let (window, port, _plane, _input) = assemble();
    let before = port_placements(&port);
    assert_eq!(before.len(), 2, "夹具必须恰好 2 条摆放");
    box_select_two(&window);

    let commits_before = port.display().commit_count;
    key_delete(&window);
    let after = port_placements(&port);
    assert_eq!(after.len(), 0, "Delete 必须删掉框选的 2 条摆放");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "删除 2 条只提交一次"
    );
    assert!(selected(&window).is_empty(), "删除后选中集合清空");
    assert!(primary(&window).is_empty(), "删除后主选中清空");

    key_ctrl(&window, "z");
    let restored = port_placements(&port);
    assert_eq!(restored, before, "一次撤销恢复全部 2 条摆放（逐字比对）");
}

/// ② 选中 2 条 → Cmd+D 新增 2 个摆放、只提交一次；复制摆放落在原摆放之后，
///    名称带 ` (Copy)`；选中移到新摆放上，主选中是第一个新摆放。
#[test]
fn duplicate_two_selected_clips_adds_two_placements_in_one_commit() {
    let (window, port, _plane, _input) = assemble();
    let all = box_select_two(&window);
    let commits_before = port.display().commit_count;

    key_ctrl(&window, "d");
    let after = port_placements(&port);
    assert_eq!(after.len(), 4, "复制 2 条必须新增 2 个 placement");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "复制 2 条只提交一次"
    );
    let copies: Vec<(String, EntityId, ClipPlacement)> = after
        .iter()
        .filter(|(id, _, _)| !all.iter().any(|(old, _, _)| old == id))
        .cloned()
        .collect();
    assert_eq!(copies.len(), 2, "新增的 2 个摆放就是复制件");
    let mut starts: Vec<u64> = copies.iter().map(|(_, _, p)| p.start_tick).collect();
    starts.sort_unstable();
    assert_eq!(starts, vec![2880, 3840], "复制摆放落在原摆放之后");
    let project_now = port.project();
    for (_, _, placement) in &copies {
        let entry = project_now
            .clip_pool
            .get(&placement.clip_id)
            .expect("复制件的 clip 必须在池里");
        assert!(
            entry.name.ends_with(" (Copy)"),
            "复制件名称带 ` (Copy)`，实际 {:?}",
            entry.name
        );
    }
    // 选中集合等于 2 个新摆放（比较时排一下序：臂按 ULID 序产新 id，
    // 而 `copies` 是按 start_tick 排的，两边顺序不同）。
    let selected_now = selected(&window);
    assert_eq!(selected_now.len(), 2, "复制后选中移到 2 个新摆放上");
    let mut selected_sorted = selected_now.clone();
    selected_sorted.sort();
    let mut copy_ids: Vec<String> = copies.iter().map(|(id, _, _)| id.clone()).collect();
    copy_ids.sort();
    assert_eq!(selected_sorted, copy_ids, "选中集合等于 2 个新摆放");
    assert_eq!(primary(&window), selected_now[0], "主选中是第一个新摆放");
}

/// ③ 选中 2 条 → Cmd+M 全部静音、再按一次全部取消静音，每次只提交一次。
#[test]
fn mute_two_selected_clips_toggles_both_in_one_commit() {
    let (window, port, _plane, _input) = assemble();
    box_select_two(&window);

    let commits_before = port.display().commit_count;
    key_ctrl(&window, "m");
    let after = port_placements(&port);
    assert_eq!(after.len(), 2, "静音不增减摆放");
    assert!(
        after.iter().all(|(_, _, p)| p.muted),
        "Cmd+M 必须静音选中的 2 条"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "静音 2 条只提交一次"
    );
    assert_eq!(selected(&window).len(), 2, "静音后选中不变");

    key_ctrl(&window, "m");
    let after = port_placements(&port);
    assert_eq!(after.len(), 2, "取消静音不增减摆放");
    assert!(
        after.iter().all(|(_, _, p)| !p.muted),
        "再按 Cmd+M 必须取消静音"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 2,
        "取消静音再提交一次"
    );
}

/// ④ 播放头 2400 + 选中 2 条 → Cmd+T 把播放头穿过的 2 条都切开
///    （两条都覆盖 2400）；左半保留原 id，右半从 2400 开始，只提交一次。
#[test]
fn split_two_selected_clips_at_a_shared_playhead() {
    let (window, port, _plane, _input) = assemble();
    window.set_playhead_ticks(2400);
    let all = box_select_two(&window);
    let commits_before = port.display().commit_count;

    key_ctrl(&window, "t");
    let after = port_placements(&port);
    assert_eq!(after.len(), 4, "切开 2 条必须新增 2 个右半");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "分割 2 条只提交一次"
    );
    let lefts: Vec<&(String, EntityId, ClipPlacement)> = after
        .iter()
        .filter(|(id, _, _)| all.iter().any(|(old, _, _)| old == id))
        .collect();
    let rights: Vec<&(String, EntityId, ClipPlacement)> = after
        .iter()
        .filter(|(id, _, _)| !all.iter().any(|(old, _, _)| old == id))
        .collect();
    assert_eq!(lefts.len(), 2, "左半保留原 id");
    assert_eq!(rights.len(), 2, "右半是 2 个新摆放");
    assert!(
        rights.iter().all(|(_, _, p)| p.start_tick == 2400),
        "右半都从播放头 2400 开始"
    );
    let mut durations: Vec<u64> = rights.iter().map(|(_, _, p)| p.duration_ticks).collect();
    durations.sort_unstable();
    assert_eq!(durations, vec![480, 1440], "右半时长分别是 480 与 1440");
    let selected_now = selected(&window);
    assert_eq!(selected_now.len(), 2, "分割后选中移到 2 个右半上");
    assert_eq!(primary(&window), selected_now[0], "主选中是第一个右半");
}

/// ⑤ Ctrl+C 复制 2 条 → Ctrl+V 在播放头处粘贴 2 条，保持相对偏移
///    （第一条落在播放头，第二条偏移 +1920），只提交一次。
#[test]
fn copy_two_clips_and_paste_preserves_relative_offsets() {
    let (window, port, _plane, _input) = assemble();
    let all = box_select_two(&window);
    key_ctrl(&window, "c");

    // 粘贴要先选中目标轨（产品行为：未选轨时粘贴拒绝并提示）。
    // 目标轨取第一条剪辑所在的轨；两条复制件都落在同一目标轨上。
    window.set_selected_track_id(all[0].1.to_canonical_string().into());
    window.set_playhead_ticks(10_000);
    let commits_before = port.display().commit_count;
    key_ctrl(&window, "v");
    let after = port_placements(&port);
    assert_eq!(after.len(), 4, "粘贴 2 条必须新增 2 个 placement");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "粘贴 2 条只提交一次"
    );
    let pasted: Vec<u64> = after
        .iter()
        .filter(|(_, _, p)| p.start_tick >= 10_000)
        .map(|(_, _, p)| p.start_tick)
        .collect();
    assert_eq!(pasted.len(), 2, "粘贴的 2 条都在播放头之后");
    let mut starts = pasted;
    starts.sort_unstable();
    assert_eq!(
        starts,
        vec![10_000, 11_920],
        "粘贴保持相对偏移：第一条在播放头，第二条 +1920"
    );
}

/// ⑥ Ctrl+X 剪切 2 条（复制 + 删除一次提交）→ Ctrl+V 恢复 2 条。
#[test]
fn cut_two_clips_copies_both_and_removes_both_then_paste_restores() {
    let (window, port, _plane, _input) = assemble();
    let all = box_select_two(&window);

    let commits_before = port.display().commit_count;
    key_ctrl(&window, "x");
    assert_eq!(port_placements(&port).len(), 0, "剪切必须删掉选中的 2 条");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "剪切（复制+删除）只提交一次"
    );
    assert!(selected(&window).is_empty(), "剪切后选中清空");

    // 粘贴要先选中目标轨（产品行为：未选轨时粘贴拒绝并提示）。
    window.set_selected_track_id(all[0].1.to_canonical_string().into());
    window.set_playhead_ticks(20_000);
    key_ctrl(&window, "v");
    let after = port_placements(&port);
    assert_eq!(after.len(), 2, "粘贴恢复 2 条");
    let mut starts: Vec<u64> = after.iter().map(|(_, _, p)| p.start_tick).collect();
    starts.sort_unstable();
    assert_eq!(starts, vec![20_000, 21_920], "粘贴恢复后保持相对偏移");
}

/// ⑦ 组拖拽：选中 2 条后按住第一条拖 +40px（1200 ticks），
///    两条摆放都移动相同增量，只提交一次。
#[test]
fn dragging_a_group_moves_both_members_by_the_same_delta() {
    let (window, port, mut plane, _input) = assemble();
    let all = box_select_two(&window);
    assert_eq!(all.len(), 2, "夹具必须恰好 2 条摆放");
    let commits_before = port.display().commit_count;

    let first_ulid = &all[0].0;
    let body_id = format!("clip-{first_ulid}-body");
    let bounds = element_bounds(&mut plane, 10, &body_id);
    assert!(
        bounds.2 > 0 && bounds.3 > 0,
        "拖动面必须有可见几何: {bounds:?}"
    );
    let origin = pointer_down_at_center(&mut plane, 20, &body_id, bounds);
    pointer_drag_right(&mut plane, 30, origin);
    pointer_up(&mut plane, 40);

    let after = port_placements(&port);
    let mut starts: Vec<u64> = after.iter().map(|(_, _, p)| p.start_tick).collect();
    starts.sort_unstable();
    assert_eq!(starts, vec![1200, 3120], "组内两条都移动 +1200 ticks");
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "组拖拽只提交一次"
    );
}

/// ⑧ 纯函数：`clip_delete_ops_for_many` 为每条可解析摆放各生成一条删除 op。
#[test]
fn clip_delete_ops_for_many_builds_one_op_per_resolvable_placement() {
    let project = filled_project();
    let all = fixture_two(&project);
    let ids: Vec<String> = all.iter().map(|(id, _, _)| id.clone()).collect();
    let ops = clip_delete_ops_for_many(&project, &ids);
    assert_eq!(ops.len(), 2, "2 条摆放各生成一条删除 op");
    assert!(
        ops.iter()
            .all(|op| matches!(op, Op::RemoveClipPlacement { .. })),
        "全部是 RemoveClipPlacement"
    );
    // 掺入一条解析不到的 id：跳过，不影响其余 op。
    let mut ids_with_stale = ids.clone();
    ids_with_stale.push(stale_clip_ulid());
    let ops = clip_delete_ops_for_many(&project, &ids_with_stale);
    assert_eq!(ops.len(), 2, "解析不到的 id 被跳过");
}

/// ⑨ 纯函数：`clip_mute_ops_for_many` 各自按当前值取反。
#[test]
fn clip_mute_ops_for_many_toggles_each_from_its_own_value() {
    let project = filled_project();
    let all = fixture_two(&project);
    let ids: Vec<String> = all.iter().map(|(id, _, _)| id.clone()).collect();
    let ops = clip_mute_ops_for_many(&project, &ids);
    assert_eq!(ops.len(), 2, "2 条摆放各生成一条静音 op");
    assert!(
        ops.iter().all(|op| matches!(
            op,
            Op::SetClipPlacementMuted {
                new_muted: true,
                ..
            }
        )),
        "未静音的摆放取反为静音"
    );
}

/// ⑩ 纯函数：`clip_split_ops_for_many` 在共享播放头切开两条，返回新右半 id；
///    应用后共 4 条摆放，右半都从 2400 开始。
///    （不钉 op 总数：MIDI 剪辑切分时还要搬运穿过切分点的音符，op 数随内容变。）
#[test]
fn clip_split_ops_for_many_splits_both_at_a_shared_playhead() {
    let project = filled_project();
    let all = fixture_two(&project);
    let ids: Vec<String> = all.iter().map(|(id, _, _)| id.clone()).collect();
    let (ops, new_ids) = clip_split_ops_for_many(&project, &ids, 2400);
    assert_eq!(new_ids.len(), 2, "返回 2 个新右半 id");
    assert!(!ops.is_empty(), "切分必须生成 op");
    let mut project = project;
    for op in &ops {
        op.apply(&mut project).expect("分割 op 必须能应用");
    }
    let after = placements(&project);
    assert_eq!(after.len(), 4, "应用后共 4 条摆放");
    for id in &new_ids {
        let (_, _, placement) = after
            .iter()
            .find(|(pid, _, _)| pid == id)
            .expect("新右半必须存在");
        assert_eq!(placement.start_tick, 2400, "右半从播放头开始");
    }
}

/// ⑪ 纯函数：`duplicate_clip_placement_ops_for` 复制单条摆放
///    （旧臂提取出的构造体，行为与旧臂一致）。
#[test]
fn duplicate_clip_placement_ops_for_duplicates_one_clip() {
    let project = filled_project();
    let all = fixture_two(&project);
    let (ops, new_id) =
        duplicate_clip_placement_ops_for(&project, &all[0].0).expect("第一条摆放必须能复制");
    assert_eq!(ops.len(), 2, "复制一条生成 AddClip + AddClipPlacement");
    assert!(
        ops.iter().any(|op| matches!(op, Op::AddClip { .. })),
        "含 AddClip"
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, Op::AddClipPlacement { .. })),
        "含 AddClipPlacement"
    );
    let mut project = project;
    for op in &ops {
        op.apply(&mut project).expect("复制 op 必须能应用");
    }
    let after = placements(&project);
    let (_, _, placement) = after
        .iter()
        .find(|(id, _, _)| id == &new_id)
        .expect("复制件必须存在");
    assert_eq!(placement.start_tick, 3840, "复制件落在原摆放之后");
    let entry = project
        .clip_pool
        .get(&placement.clip_id)
        .expect("复制件的 clip 必须在池里");
    assert!(entry.name.ends_with(" (Copy)"), "复制件名称带后缀");
}

/// ⑫ 纪律：`RemoveClipPlacement` / `SetClipPlacementMuted` /
///    `MoveClipPlacement` 在 host.rs 里各只有一个构造点。
#[test]
fn op_construction_sites_stay_single() {
    let source = include_str!("../src/host.rs");
    let remove_sites = source.matches("Op::RemoveClipPlacement {").count();
    let mute_sites = source.matches("Op::SetClipPlacementMuted {").count();
    let move_sites = source.matches("Op::MoveClipPlacement {").count();
    assert_eq!(remove_sites, 1, "RemoveClipPlacement 必须只有一个构造点");
    assert_eq!(mute_sites, 1, "SetClipPlacementMuted 必须只有一个构造点");
    assert_eq!(move_sites, 1, "MoveClipPlacement 必须只有一个构造点");
}

/// ⑬ 纪律：删除/静音/分割/复制/粘贴/拖拽臂都经由统一的选中集合读取点。
#[test]
fn edit_arms_read_the_selection_set_through_one_helper() {
    let source = include_str!("../src/host.rs");
    assert!(
        source.contains("fn selected_clip_ulids_or_primary"),
        "统一读取点必须存在"
    );
    let calls = source.matches("selected_clip_ulids_or_primary(ui)").count();
    assert!(
        calls >= 5,
        "删除/复制/剪切/静音/分割臂都应调用统一读取点，实际 {calls} 处"
    );
    assert!(source.contains("CLIP_DRAG_GROUP"), "组拖拽快照必须存在");
}

/// T-FIX-488：音频片段在中点切分后，右半段必须从切点对应的素材帧继续读。
///
/// 夹具：`filled_project` 的音频摆放改成 `[0, 1920)`，素材 PCM 是前半 `-0.5`、
/// 后半 `+0.5` 的方波；切点帧由工程 BPM/采样率现算（本夹具 BPM=128 ⇒ 960 tick =
/// 22 500 帧，⛔ 不写死 24 000）。在 960 tick 切分：
/// - 右半 `source_start_samples = Some(切点帧)`；
/// - 右半首帧（绝对帧 = 切点帧）应读 `+0.5`；
/// - 把右半的素材内起点强制回 `None`（旧公式）⇒ 读素材第 0 帧 `-0.5` ⇒ 旧实现红。
#[test]
fn splitting_audio_clip_resumes_from_cut_frame_not_zero() {
    let mut project = filled_project();
    let (track_id, placement_id, _clip_id, asset) = project
        .tracks
        .iter()
        .find_map(|(track_id, track)| {
            track.clips.iter().find_map(|(placement_id, placement)| {
                let entry = project.clip_pool.get(&placement.clip_id)?;
                match &entry.content {
                    ClipContent::Audio { asset, .. } => {
                        Some((*track_id, *placement_id, placement.clip_id, asset.clone()))
                    }
                    ClipContent::Midi { .. } => None,
                }
            })
        })
        .expect("filled_project 必须有一条音频摆放");

    let placement = project
        .tracks
        .get_mut(&track_id)
        .expect("音频轨必须存在")
        .clips
        .get_mut(&placement_id)
        .expect("音频摆放必须存在");
    placement.start_tick = 0;
    placement.duration_ticks = 1_920;
    placement.loop_config = yeban_model::LoopConfig::default();
    placement.offset_samples = None;
    placement.fade_in_ticks = None;
    placement.fade_out_ticks = None;
    placement.source_start_samples = None;

    let (ops, right_id) = clip_split_ops_for(&project, &placement_id.to_canonical_string(), 960)
        .expect("中点切分必须可构造");
    for op in &ops {
        op.apply(&mut project).expect("切分 op 必须可应用");
    }
    let right_id: EntityId = right_id.parse().expect("右半身份必须是合法 ULID");
    let split_frame = yeban_model::time_math::ticks_to_frames(
        960,
        project.bpm,
        yeban_model::PPQ,
        project.audio_config.sample_rate.hz(),
    )
    .expect("合法工程必须能算切点帧");
    let right = project
        .tracks
        .get(&track_id)
        .expect("音频轨必须存在")
        .clips
        .get(&right_id)
        .expect("右半摆放必须存在");
    assert_eq!(right.start_tick, 960, "右半从切点开始");
    assert_eq!(
        right.source_start_samples,
        Some(split_frame),
        "右半素材内起点 = 切点相对摆放起点的 {split_frame} 帧"
    );

    // 双倍切点长度的单声道：前半 -0.5、后半 +0.5，切点处符号翻转。
    let frames = usize::try_from(split_frame.saturating_mul(2)).expect("帧数必须放得进 usize");
    let samples: Vec<f32> = (0..frames)
        .map(|frame| {
            if u64::try_from(frame).unwrap_or(u64::MAX) < split_frame {
                -0.5
            } else {
                0.5
            }
        })
        .collect();
    let pcm = std::sync::Arc::new(
        AudioPcm::new(
            std::sync::Arc::from(samples),
            1,
            project.audio_config.sample_rate.hz(),
        )
        .expect("PCM 必须合法"),
    );
    let mut schedules = project_audio_clips(&project);
    for schedule in schedules.values_mut() {
        schedule.attach_pcm(&asset, &pcm);
    }
    let schedule = schedules.get(&track_id).expect("音频轨必须有 span 表");
    let mut out = [0.0_f32; 1];
    let mut out_r = [0.0_f32; 1];
    mix_into(schedule, split_frame, &mut out, &mut out_r);
    assert!(
        out[0] > 0.01,
        "切分右半首帧应读切点后的 +0.5，而不是素材第 0 帧的 -0.5；实际 {}",
        out[0]
    );

    // 红对照：旧实现没有素材内起点字段（等价于 None）⇒ 同一帧读成 -0.5。
    let mut old_formula = project.clone();
    old_formula
        .tracks
        .get_mut(&track_id)
        .expect("音频轨必须存在")
        .clips
        .get_mut(&right_id)
        .expect("右半摆放必须存在")
        .source_start_samples = None;
    let mut old_schedules = project_audio_clips(&old_formula);
    for schedule in old_schedules.values_mut() {
        schedule.attach_pcm(&asset, &pcm);
    }
    let old_schedule = old_schedules.get(&track_id).expect("音频轨必须有 span 表");
    let mut old_out = [0.0_f32; 1];
    let mut old_out_r = [0.0_f32; 1];
    mix_into(old_schedule, split_frame, &mut old_out, &mut old_out_r);
    assert!(
        old_out[0] < -0.01,
        "旧公式（source_start=None）必须读素材第 0 帧 -0.5；实际 {}",
        old_out[0]
    );
}
