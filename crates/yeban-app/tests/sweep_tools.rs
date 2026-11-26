//! T-FIX-768：剪刀拖拽切分 / 橡皮擦拖拽擦除的**活窗口**判据。
//!
//! 两条判据都从**真实指针回调**（`note-drag-grab/move/release`）进入，读回**权威工程**：
//! - 剪刀横拖过一颗音符 ⇒ 原文消失、两颗新音符、一次撤销回原文；
//! - 橡皮擦横拖过相邻两颗音符 ⇒ 两颗都没了、**一次**撤销把两颗都恢复（身份级比对）。
//!
//! 红对照：本文件在实现前先跑一遍，`grab` 返回 `false` / 音符数不变即红。

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use yeban_app::bridge::{NewTrackKind, ViewState, host_to_painted, track_create_ops};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::YebanProjectV1;
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::project::{ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

struct Fixture {
    live: LivePort<yeban_app::ui::MainWindow>,
    undo: Rc<UndoPort>,
    first: EntityId,
    second: EntityId,
}

fn notes(project: &YebanProjectV1) -> BTreeMap<EntityId, MidiNote> {
    project
        .clip_pool
        .values()
        .find_map(|entry| match &entry.content {
            ClipContent::Midi { notes } => Some(notes.clone()),
            _ => None,
        })
        .expect("夹具必须有一个 MIDI 片段")
}

fn build_fixture() -> Fixture {
    let base = yeban_model::samples::default_project();
    let seed_ops = track_create_ops(&base, NewTrackKind::Midi).expect("创建 MIDI 轨");
    let track_id = seed_ops
        .iter()
        .find_map(|op| match op {
            Op::AddTrack { track } => Some(track.id),
            _ => None,
        })
        .expect("必须返回新建轨道 ID");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<sweep-tools>", "yeban-app", base, NOW_MS).expect("撤销会话"),
    ));
    undo.commit_ops(NOW_MS + 1, "test: add track", seed_ops)
        .expect("提交新建轨道");

    let clip_id = EntityId::new();
    let placement_id = EntityId::new();
    let first = EntityId::new();
    let second = EntityId::new();
    let mut clip_notes = BTreeMap::new();
    // 两颗同音高、首尾相接的音符：剪刀只碰第一颗，橡皮擦横拖会同时扫过两颗。
    clip_notes.insert(first, MidiNote::new(first, 0, 60, 960));
    clip_notes.insert(second, MidiNote::new(second, 960, 60, 960));
    let clip = ClipPoolEntry {
        id: clip_id,
        name: "sweep-fixture".into(),
        content: ClipContent::Midi { notes: clip_notes },
    };
    let placement = ClipPlacement {
        id: placement_id,
        clip_id,
        start_tick: 0,
        duration_ticks: 3840,
        loop_config: LoopConfig::default(),
        muted: false,
        offset_samples: None,
        fade_in_ticks: None,
        fade_out_ticks: None,
        source_start_samples: None,
    };
    undo.commit_ops(
        NOW_MS + 2,
        "test: add sweep clip",
        vec![
            Op::AddClip { clip },
            Op::AddClipPlacement {
                track_id,
                placement,
            },
        ],
    )
    .expect("提交夹具片段");

    let project = undo.try_project().expect("权威工程");
    let view = ViewState::from_project(&project).expect("工程投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    ui.set_selected_clip_ulid(placement_id.to_string().into());
    ui.set_roll_scroll_x(0.0);
    ui.set_active_tool(3); // 剪刀；橡皮擦用例里再改成 5
    host::wire_roll_edit(ui, &undo);

    Fixture {
        live,
        undo,
        first,
        second,
    }
}

/// 取第一颗音符在**绘制帧**里的左沿内 2px / 行中心。
fn first_anchor(fixture: &Fixture) -> (f32, f32, f32) {
    let project = fixture.undo.try_project().expect("权威工程");
    let view = host::project_with_view_state(&project, fixture.live.ui()).expect("重投影");
    let note = view
        .notes
        .iter()
        .find(|note| note.id == fixture.first.to_string())
        .expect("第一颗音符必须在投影里");
    let scroll = fixture.live.ui().get_roll_scroll_x();
    let start = host_to_painted(note.x - scroll + 2.0, note.y + 4.0);
    let end = host_to_painted(note.x - scroll + note.width - 2.0, note.y + 4.0);
    (start.0, start.1, end.0)
}

/// 剪刀横拖第一颗音符：起点在音符内，终点也在音符内（越过 4px 阈值 ⇒ 拖拽臂）。
#[test]
fn knife_drag_across_a_note_splits_it_once() {
    let fixture = build_fixture();
    let ui = fixture.live.ui();
    let before = notes(&fixture.undo.try_project().expect("权威工程"));
    assert_eq!(before.len(), 2, "夹具必须恰好两颗音符");
    let (start_x, start_y, end_x) = first_anchor(&fixture);
    let mid_x = start_x + 12.0;
    assert!(
        (mid_x - start_x) > 4.0 && mid_x < end_x,
        "拖拽必须越过 4px 阈值且仍落在音符内"
    );

    assert!(
        ui.invoke_note_drag_grab(start_x, start_y),
        "剪刀拖拽必须由宿主接管（不得静默落到滚动臂）"
    );
    ui.invoke_note_drag_move(mid_x, start_y);
    ui.invoke_note_drag_release(mid_x, start_y);

    let after = notes(&fixture.undo.try_project().expect("权威工程"));
    assert_eq!(after.len(), 3, "剪刀拖拽后原文应变成两颗半边：2 → 3");
    assert!(
        !after.contains_key(&fixture.first),
        "原文身份必须被 DeleteNote 移除，而不是原地改"
    );
    let halves: Vec<_> = after
        .values()
        .filter(|note| note.pitch == 60 && note.velocity == before[&fixture.first].velocity)
        .collect();
    assert_eq!(halves.len(), 3, "三颗音符都应保持 pitch/velocity");
    let split = after
        .values()
        .find(|note| note.start_tick == 0)
        .expect("左半必须从 0 开始");
    let right = after
        .values()
        .find(|note| note.start_tick > 0 && note.start_tick < 960)
        .expect("必须有一颗从切点开始的新音符");
    assert_eq!(
        split.duration_ticks + right.duration_ticks,
        960,
        "两半必须恰好铺满原文"
    );
    assert_eq!(ui.get_roll_scroll_x(), 0.0, "剪刀拖拽不得静默变成滚动");
    assert!(
        ui.get_roll_edit_status().contains("切分"),
        "剪刀拖拽必须给出切分回执，实际 = {:?}",
        ui.get_roll_edit_status()
    );

    fixture.undo.perform(UiAction::Undo);
    let restored = notes(&fixture.undo.try_project().expect("权威工程"));
    assert_eq!(restored.len(), 2, "一次撤销必须回到切分前的音符数");
    assert!(
        restored.contains_key(&fixture.first),
        "一次撤销必须恢复原文身份（证明是一步批撤销）"
    );
}

/// 橡皮擦横拖过相邻两颗音符：一次手势删两颗，一次撤销全部恢复。
#[test]
fn eraser_drag_over_two_notes_deletes_both_in_one_undo_step() {
    let fixture = build_fixture();
    let ui = fixture.live.ui();
    ui.set_active_tool(5);
    let before = notes(&fixture.undo.try_project().expect("权威工程"));
    let ids: BTreeSet<EntityId> = before.keys().copied().collect();
    assert_eq!(ids.len(), 2, "夹具必须恰好两颗音符");

    let project = fixture.undo.try_project().expect("权威工程");
    let view = host::project_with_view_state(&project, ui).expect("重投影");
    let left = view
        .notes
        .iter()
        .find(|note| note.id == fixture.first.to_string())
        .expect("第一颗");
    let right = view
        .notes
        .iter()
        .find(|note| note.id == fixture.second.to_string())
        .expect("第二颗");
    let scroll = ui.get_roll_scroll_x();
    let start = host_to_painted(left.x - scroll + 2.0, left.y + 4.0);
    let finish = host_to_painted(right.x - scroll + right.width - 2.0, right.y + 4.0);
    assert!(
        finish.0 > start.0 + 4.0,
        "橡皮拖拽必须越过 4px 阈值并覆盖第二颗"
    );

    assert!(
        ui.invoke_note_drag_grab(start.0, start.1),
        "橡皮擦拖拽必须由宿主接管（不得静默落到滚动臂）"
    );
    ui.invoke_note_drag_move(finish.0, finish.1);
    ui.invoke_note_drag_release(finish.0, finish.1);

    assert!(
        notes(&fixture.undo.try_project().expect("权威工程")).is_empty(),
        "橡皮擦横拖必须删除被扫过的两颗音符"
    );
    assert_eq!(ui.get_roll_scroll_x(), 0.0, "橡皮擦拖拽不得静默变成滚动");
    assert!(
        ui.get_roll_edit_status().contains("删除"),
        "橡皮擦拖拽必须给出删除回执，实际 = {:?}",
        ui.get_roll_edit_status()
    );

    fixture.undo.perform(UiAction::Undo);
    let restored: BTreeSet<EntityId> = notes(&fixture.undo.try_project().expect("权威工程"))
        .keys()
        .copied()
        .collect();
    assert_eq!(
        restored, ids,
        "一次撤销必须把两颗音符按原身份全部恢复（证明一次手势 = 一步批撤销）"
    );
}

/// 空白处剪刀拖拽：必须接管并给明确回执，不能静默滚动。
#[test]
fn knife_drag_on_blank_is_loud_instead_of_scrolling() {
    let fixture = build_fixture();
    let ui = fixture.live.ui();
    let (start_x, start_y, _) = first_anchor(&fixture);
    // 垂直移到琴键/空白区之外仍算网格，但 x 远离音符：用音符左侧 40px。
    let blank_x = (start_x - 40.0).max(20.0);
    assert!(ui.invoke_note_drag_grab(blank_x, start_y));
    ui.invoke_note_drag_move(blank_x + 30.0, start_y);
    ui.invoke_note_drag_release(blank_x + 30.0, start_y);
    assert_eq!(
        notes(&fixture.undo.try_project().expect("权威工程")).len(),
        2
    );
    assert_eq!(ui.get_roll_scroll_x(), 0.0);
    assert!(
        !ui.get_roll_edit_status().is_empty(),
        "空白剪刀拖拽必须给明确回执，不得静默"
    );
}
