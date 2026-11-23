//! **卷帘多音符选区入口**回归判据（T-FIX-204 / ISS-MWTFLI5C-10E44F）。
//!
//! 三条入口：① 空白拖拽框选（选择工具）；② Shift 单击 toggle；③ Cmd/Ctrl+A 全选。
//! 判据只看可观测读数：`selected-note-count` / `selected-ulids`，再用 Delete 键验证
//! “框选 3 颗 ⇒ 一次删 3 颗 ⇒ 一次撤销全部回来”。
//!
//! ⚠ 单 `#[test]`：Slint 平台每线程只能装一次。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model;
use yeban_app::bridge::{ViewState, host_to_painted};
use yeban_app::host;
use yeban_app::input::InputContext;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, MidiNote, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;

/// 夹具：`filled_project` 的 Lead 片段里放 3 颗音符（60/62/64，tick 0/960/1920）。
fn three_note_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    let mut lead_id = None;
    let mut clip_id = None;
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            if matches!(
                &project.clip_pool[&placement.clip_id].content,
                ClipContent::Midi { .. }
            ) {
                lead_id = Some(track.id);
                clip_id = Some(placement.clip_id);
            }
        }
    }
    let lead_id = lead_id.expect("夹具必须有 MIDI 轨");
    let clip_id = clip_id.expect("夹具必须有 MIDI 片段");
    let ClipContent::Midi { notes } = &mut project
        .clip_pool
        .get_mut(&clip_id)
        .expect("片段池条目")
        .content
    else {
        unreachable!();
    };
    notes.clear();
    for (start, pitch) in [(0_u64, 60_u8), (960, 62), (1920, 64)] {
        let id = EntityId::new();
        notes.insert(id, MidiNote::new(id, start, pitch, 480));
    }
    (project, lead_id, clip_id)
}

fn midi_note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            ClipContent::Audio { .. } => 0,
        })
        .sum()
}

#[test]
fn marquee_selects_three_notes_and_delete_removes_them_in_one_undo_step() {
    let (project, _lead_id, _clip_id) = three_note_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<roll-marquee>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台");
    let ui = live.ui();
    // 生产接线：点击/marquee 带端口重装一次；键盘（Delete / Cmd+Z）接同一端口。
    host::wire_roll_edit(ui, &port);
    host::wire_keys(
        ui,
        Rc::new(RefCell::new(InputContext::new())),
        Some(Rc::clone(&port)),
        None,
        Some(Rc::new(project.clone())),
    );

    // ── ① 框选：把三颗音符全包进矩形（宿主坐标 → 绘制帧）。
    let ui_base = ui.get_roll_base_pitch().clamp(0, 112) as u8;
    let scroll = ui.get_roll_scroll_x();
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    let mut visible = view.visible_notes_scoped(scroll, width, None);
    view.reproject_note_ys_with_base(&visible.ulids, &mut visible.ys, ui_base);
    assert_eq!(visible.ulids.len(), 3, "夹具必须 3 颗可见音符");
    let min_x = visible.positions.iter().copied().fold(f32::MAX, f32::min) - 8.0;
    let max_x = visible
        .positions
        .iter()
        .zip(&visible.widths)
        .map(|(x, w)| x + w)
        .fold(f32::MIN, f32::max)
        + 8.0;
    let min_y = visible.ys.iter().copied().fold(f32::MAX, f32::min) - 8.0;
    let max_y = visible.ys.iter().copied().fold(f32::MIN, f32::max) + 18.0;
    let (x1, y1) = host_to_painted(min_x, min_y);
    let (x2, y2) = host_to_painted(max_x, max_y);
    ui.invoke_roll_marquee(x1, y1, x2, y2, false);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "框选必须选中 3 颗（矩形 {x1},{y1} → {x2},{y2}）"
    );
    assert_eq!(ui.get_selected_ulids().iter().count(), 3);

    // ── ② Shift 单击第 4 次? 没有第 4 颗：用 Shift 单击**已选**的一颗 ⇒ 取消它（2 颗）。
    let (cx, cy) = host_to_painted(
        visible.positions[0] + visible.widths[0] / 2.0,
        visible.ys[0] + 3.0,
    );
    ui.invoke_clicked(cx, cy, true);
    assert_eq!(
        ui.get_selected_note_count(),
        2,
        "Shift 单击已选音符必须取消它"
    );
    // 再 Shift 一次 ⇒ 加回来（3 颗）。
    ui.invoke_clicked(cx, cy, true);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "Shift 单击未选音符必须加选"
    );

    // ── ③ Cmd/Ctrl+A 全选（作用域 = 全部可见）⇒ 仍是 3 颗（幂等）。
    ui.invoke_key_action("a".into(), false, true, false, false);
    assert_eq!(
        ui.get_selected_note_count(),
        3,
        "Cmd/Ctrl+A 必须全选作用域内音符"
    );

    // ── ④ Delete：一次删 3 颗；一次撤销全部回来。
    assert_eq!(
        midi_note_count(port.try_project().as_ref().expect("工程")),
        3
    );
    ui.invoke_key_action("\u{7f}".into(), false, false, false, false);
    assert_eq!(
        midi_note_count(port.try_project().as_ref().expect("工程")),
        0,
        "Delete 必须一次删掉 3 颗选中音符"
    );
    ui.invoke_key_action("z".into(), false, true, false, false);
    assert_eq!(
        midi_note_count(port.try_project().as_ref().expect("工程")),
        3,
        "一次撤销必须把 3 颗全部恢复（原子 Batch）"
    );
}

/// 界面侧的接线是“空白拖拽 = 框选”的**唯一**手势来源：这条判据钉住三件事——
/// ① 选择工具（active-tool == 1）且宿主没接管音符拖动时才进入 marquee；
/// ② 松手拖过阈值上抛 `roll-marquee`，没拖过阈值仍是一次空白单击；
/// ③ cancel 会丢弃框选草稿（不改选区）。
#[test]
fn slint_wires_blank_drag_to_marquee_only_for_the_select_tool() {
    let source = include_str!("../ui/console/piano_roll.slint");
    assert!(
        source.contains("if !root.note-dragging && root.active-tool == 1 {"),
        "空白拖拽必须限定在选择工具且宿主未接管音符拖动"
    );
    assert!(
        source.contains("root.roll-marquee(x1, y1, self.mouse-x, self.mouse-y, additive);"),
        "拖过阈值必须上抛 roll-marquee"
    );
    assert!(
        source.contains("root.clicked(self.mouse-x, self.mouse-y, event.modifiers.shift);"),
        "单击必须把 Shift 状态一起上报（toggle 语义）"
    );
    assert!(
        source.contains("if root.marquee-active {"),
        "cancel 分支必须丢弃框选草稿"
    );
}
