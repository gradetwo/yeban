//! T-FIX-579：量化对话框的**音乐单位 + 摇摆**真实链路。
//!
//! 走界面回调：打开量化 → 写摇摆 50% → 应用 → 权威工程里的后格音符被推后。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use std::rc::Rc;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_760_000_000_000;

fn first_midi_clip(project: &YebanProjectV1) -> (String, yeban_model::EntityId) {
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                && entry.content.notes().is_some_and(|notes| !notes.is_empty())
            {
                return (placement.id.to_canonical_string(), placement.clip_id);
            }
        }
    }
    panic!("filled_project 必须含至少一个带音符的 MIDI 片段");
}

#[test]
fn quantize_dialog_swing_fifty_percent_delays_the_offbeat() {
    let mut project = filled_project();
    let (clip_ulid, clip_id) = first_midi_clip(&project);
    let note_id = {
        let entry = project.clip_pool.get_mut(&clip_id).expect("片段在池里");
        let notes = entry.content.notes_mut().expect("MIDI 片段");
        let (note_id, note) = notes.iter_mut().next().expect("非空夹具");
        note.start_tick = 240;
        *note_id
    };

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<quantize-swing-ui>", "yeban-app", project, NOW_MS).expect("打开会话"),
    ));
    let ui = build_live_ui_with(
        &filled_project(),
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + 接线");
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    window.invoke_clip_select(clip_ulid.clone().into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        clip_ulid,
        "夹具前提：宿主接受这个剪辑身份"
    );
    window.invoke_open_quantize();
    window.set_quantize_swing("50".into());
    // 明确 1/16 均分（与缺省一致，避免依赖装配态）。
    window.invoke_roll_grid_note_picked(4);
    window.invoke_roll_grid_mode_picked(0);
    window.invoke_apply_quantize();

    let after = port.try_project().expect("权威工程");
    let notes = after
        .clip_pool
        .get(&clip_id)
        .and_then(|entry| entry.content.notes())
        .expect("片段仍在");
    assert_eq!(
        notes.get(&note_id).expect("音符仍在").start_tick,
        300,
        "1/16 · 50% 摇摆：240 的后格音符应推到 300"
    );
}

#[test]
fn quantize_dialog_swing_zero_keeps_the_old_240_tick_result() {
    let mut project = filled_project();
    let (clip_ulid, clip_id) = first_midi_clip(&project);
    let note_id = {
        let entry = project.clip_pool.get_mut(&clip_id).expect("片段在池里");
        let notes = entry.content.notes_mut().expect("MIDI 片段");
        let (note_id, note) = notes.iter_mut().next().expect("非空夹具");
        // 离 240 网格最近的落点是 120：旧量化在 50% 强度下应得 180。
        note.start_tick = 120;
        *note_id
    };

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<quantize-swing-zero-ui>", "yeban-app", project, NOW_MS)
            .expect("打开会话"),
    ));
    let ui = build_live_ui_with(
        &filled_project(),
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + 接线");
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    window.invoke_clip_select(clip_ulid.clone().into());
    window.invoke_open_quantize();
    window.set_quantize_swing("0".into());
    window.set_humanize_strength("50".into());
    window.invoke_roll_grid_note_picked(4);
    window.invoke_roll_grid_mode_picked(0);
    window.invoke_apply_quantize();

    let after = port.try_project().expect("权威工程");
    let notes = after
        .clip_pool
        .get(&clip_id)
        .and_then(|entry| entry.content.notes())
        .expect("片段仍在");
    assert_eq!(
        notes.get(&note_id).expect("音符仍在").start_tick,
        180,
        "swing 0% 必须与旧 240 tick 量化逐位一致（120 → 0 与 240 的中点 → 180）"
    );
}
