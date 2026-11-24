//! ⭐ **T-FIX-302 判据**：实时 MIDI 音符提交被撤销图拒绝时，拒绝数与累计读数
//! **单独**记账，且界面提示函数给出可见提示；⛔ 不混进车道溢出计数。
//!
//! 用一条落点不存在的 `AddNote` 让真 `UndoPort` 返回 `Err`（这是产品路径的拒绝，
//! 不是另写一段假提交）。放在集成测试里而不是 `--lib` 单测里，是为了避开
//! 4 GB cgroup 下 app 单元测试二进制的链接峰值。

use yeban_app::host::{LiveNoteCommit, MidiInState, live_note_commit_status};
use yeban_app::midi_notes::CapturedNote;
use yeban_app::undo::{UndoPort, UndoSession};

fn live_note(pitch: u8, start_tick: u64) -> CapturedNote {
    CapturedNote {
        pitch,
        velocity: 100,
        start_tick,
        end_tick: start_tick + 240,
    }
}

#[test]
fn the_live_note_commit_refusal_is_counted_and_visible() {
    let project = yeban_model::samples::filled_project();
    let port =
        UndoPort::new(UndoSession::open("<判据>", "yeban-app", project.clone(), 0).expect("打开"));
    let state = MidiInState::new();
    assert_eq!(state.note_lane().push(live_note(60, 0)), 1, "车道必须收下");

    let target = LiveNoteCommit {
        port: &port,
        // ⭐ 落点故意不存在 ⇒ 模型校验拒绝这次 `AddNote`。
        track_id: yeban_model::EntityId::new(),
        clip_id: yeban_model::EntityId::new(),
        now_ms: || 0,
    };
    let report = state.commit_live_notes(&target);
    assert_eq!(report.committed, 0, "被拒时不许假装提交成功");
    assert_eq!(report.refused, 1, "本跳必须记下这一次拒绝");
    assert_eq!(report.pending, 0, "被拒的音符不再放回车道");
    assert_eq!(report.refused_cumulative, 1, "提交拒绝累计必须加一");
    assert_eq!(
        report.lane_refused_cumulative, 0,
        "车道没有溢出 ⇒ 溢出计数必须仍是 0"
    );
    assert_eq!(state.note_commit_refused(), 1, "状态里的累计读数必须同步");
    assert_eq!(
        state.note_lane_refused(),
        0,
        "提交拒绝不许写进 `note_lane.refused()`"
    );

    let status = live_note_commit_status(&report).expect("有拒绝就必须有可见提示");
    assert!(status.contains("提交被拒 1 条"), "{status}");
    assert!(status.contains("累计 1"), "{status}");
    assert!(status.contains("车道溢出 0"), "{status}");
    assert!(
        live_note_commit_status(&yeban_app::host::LiveNoteCommitReport::default()).is_none(),
        "没有拒绝时不许刷提示"
    );
}
