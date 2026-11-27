//! T-FIX-801 红对照：Ctrl+A 后 Delete 必须删光当前 MIDI 片段的全部音符。
//! 基线只有单选，Ctrl+A 无 dispatch 分支 ⇒ 本测试应失败。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_model::ids::EntityId;
use yeban_model::project::ClipContent;
use yeban_tui::app::{App, ListView};
use yeban_tui::keymap::Pane;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn char_key(ch: char) -> KeyEvent {
    key(KeyCode::Char(ch))
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn focus_clip(app: &mut App, clip_id: EntityId) {
    let mut found = None;
    for (index, track_id) in app.track_ids.iter().enumerate() {
        let track = &app.project.tracks[track_id];
        if let Some(placement) = track.clips.values().find(|p| p.clip_id == clip_id) {
            found = Some((index, placement.start_tick));
            break;
        }
    }
    let (index, start) = found.expect("应有引用该条目的摆放");
    app.selected = index;
    if app.focus != Pane::Grid {
        app.handle_key(key(KeyCode::Tab));
    }
    assert_eq!(app.focus, Pane::Grid);
    app.cursor_bar = usize::try_from(start / app.bar_ticks()).unwrap_or(0);
}

fn open_notes(app: &mut App) -> yeban_model::ids::EntityId {
    let clip_id = app
        .project
        .clip_pool
        .values()
        .find(|entry| matches!(entry.content, ClipContent::Midi { .. }))
        .map(|entry| entry.id)
        .expect("演示工程应有 MIDI 片段");
    focus_clip(app, clip_id);
    app.handle_key(char_key('n'));
    assert!(matches!(app.list_view, Some(ListView::Notes { .. })));
    clip_id
}

#[test]
fn ctrl_a_then_delete_removes_all_notes() {
    let mut app = App::with_demo().expect("demo app");
    let clip_id = open_notes(&mut app);
    let before = match &app.project.clip_pool[&clip_id].content {
        ClipContent::Midi { notes } => notes.len(),
        ClipContent::Audio { .. } => 0,
    };
    assert_eq!(before, 6, "演示 MIDI 片段应有 6 个音符");

    app.handle_key(ctrl('a'));
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(
        app.view_notes().len(),
        0,
        "Ctrl+A 后 Delete 必须删光当前片段的全部音符"
    );
}

#[test]
fn ctrl_a_then_nudge_moves_every_selected_note() {
    let mut app = App::with_demo().expect("demo app");
    open_notes(&mut app);
    let before: Vec<u64> = app
        .view_notes()
        .iter()
        .map(|note| note.start_tick)
        .collect();
    app.handle_key(ctrl('a'));
    app.handle_key(char_key('l'));
    let after: Vec<u64> = app
        .view_notes()
        .iter()
        .map(|note| note.start_tick)
        .collect();
    assert_eq!(before.len(), 6);
    assert_eq!(
        after,
        before.iter().map(|tick| tick + 240).collect::<Vec<_>>()
    );
}
