//! `#339` 判据：快照的 tick → 帧投影真的读 `project.tempo_map`
//! [MODEL-AST-002, ARCH-DET-001]。
//!
//! 接线前的缺陷：`tempo_map` / `time_signature_map` 都能经 `SetTempoPoint` 与
//! `yeban_edit_notes` 写入，但 `project_schedules` 与 `project_audio_clips` 只乘
//! `project.bpm` ⇒ 变速在母带里**静默无效**。本文件的四条判据把"读到了"钉住：
//!
//! | 编号 | 判据 | 怎么变红 |
//! | :--- | :--- | :--- |
//! | T1 | 空 `tempo_map` 的 `from_project_with_tempo` 与 `from_project` 逐项相同 | 空表路径漂移（"变速表为空也改了旧行为"） |
//! | T2 | 变速点之后落的音符按分段常速后移（tick 1920 ⇒ 36 000 帧） | 仍乘 `project.bpm` |
//! | T3 | 音频摆放的帧区间同样按分段常速换算 | 音频路径漏接线 |
//! | T4 | 端到端：变速后的音符在 36 000 帧之前**完全静音**，之后出声 | 只有表变了、渲染没变 |
//!
//! 夹具栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 25 帧每 tick；变速点
//! `{960: 240 BPM}` 把后半段每 tick 减半到 12.5 帧。

mod support;

use support::{NoteSpec, audio_clip_project_at, note_project, render_snapshot};
use yeban_engine::snapshot::EngineSnapshot;

/// 夹具：120 BPM 起，tick 960 之后 240 BPM。
fn slowed_project(project: &mut yeban_model::YebanProjectV1) {
    project.tempo_map.insert(960, 240.0);
}

/// T1：空表**逐项**等于 `from_project`（调度表音符 + 音频片段表都算）。
#[test]
fn an_empty_tempo_map_is_bit_identical_to_from_project() {
    let fixture = note_project(&[
        NoteSpec::at(0, 480, 60, 100),
        NoteSpec::at(960, 960, 64, 100),
        NoteSpec::at(2400, 480, 67, 100),
    ]);
    let baseline = EngineSnapshot::from_project(&fixture.project, 1).expect("基线快照");
    let explicit =
        EngineSnapshot::from_project_with_tempo(&fixture.project, 1, &fixture.project.tempo())
            .expect("显式空表快照");

    let baseline_notes = baseline
        .schedule(&fixture.track)
        .expect("基线调度表")
        .notes();
    let explicit_notes = explicit
        .schedule(&fixture.track)
        .expect("显式调度表")
        .notes();
    assert_eq!(baseline_notes.len(), explicit_notes.len(), "音符数必须相同");
    for (left, right) in baseline_notes.iter().zip(explicit_notes) {
        assert_eq!(
            (
                left.start_sample(),
                left.end_sample(),
                left.pitch(),
                left.gain().to_bits()
            ),
            (
                right.start_sample(),
                right.end_sample(),
                right.pitch(),
                right.gain().to_bits()
            ),
            "空 tempo_map 不许改动任何音符的帧区间或增益位型"
        );
    }
    assert_eq!(
        baseline.audio_clips().len(),
        explicit.audio_clips().len(),
        "空表的音频片段表也必须相同"
    );
}

/// T2：变速点之后落的音符按**分段常速**后移。
///
/// `[0, 960)` @120 BPM = 24 000 帧；`[960, 1920)` @240 BPM = 12 000 帧
/// ⇒ tick 1920 的起点是 **36 000** 帧（恒定 120 BPM 下会是 48 000）。
#[test]
fn a_tempo_point_after_the_note_moves_its_onset() {
    let fixture = note_project(&[NoteSpec::at(1920, 480, 64, 100)]);
    let constant = EngineSnapshot::from_project(&fixture.project, 1).expect("恒定快照");
    let constant_start = constant
        .schedule(&fixture.track)
        .expect("恒定调度表")
        .notes()[0]
        .start_sample();
    assert_eq!(constant_start, 48_000, "恒定 120 BPM 下 tick 1920 = 48 000");

    let mut project = fixture.project.clone();
    slowed_project(&mut project);
    let mapped =
        EngineSnapshot::from_project_with_tempo(&project, 1, &project.tempo()).expect("变速快照");
    let note = &mapped.schedule(&fixture.track).expect("变速调度表").notes()[0];
    assert_eq!(note.start_sample(), 36_000, "分段常速：24 000 + 12 000");
    // 时值 480 tick 全落在 240 BPM 段 ⇒ 6 000 帧。
    assert_eq!(note.end_sample(), 42_000, "终点 = 36 000 + 6 000");
}

/// T3：音频摆放在变速点之后同样按分段常速换算（不是只改了 MIDI）。
#[test]
fn an_audio_placement_after_the_tempo_point_moves_too() {
    let fixture = audio_clip_project_at(1920, 960, 0.0);
    let mut project = fixture.project.clone();
    slowed_project(&mut project);
    let mapped =
        EngineSnapshot::from_project_with_tempo(&project, 1, &project.tempo()).expect("变速快照");
    let schedule = mapped
        .audio_clip_schedule(&fixture.track)
        .expect("夹具应有一条音频轨");
    let span = &schedule.spans()[0];
    assert_eq!(span.start(), 36_000, "音频起点同样走分段常速");
    // 960 tick @240 BPM = 12 000 帧。
    assert_eq!(span.end(), 48_000, "终点 = 36 000 + 12 000");
}

/// T4：端到端 —— 变速后的音符在 36 000 帧之前**完全静音**，之后出声。
#[test]
fn rendering_a_tempo_mapped_snapshot_starts_sound_at_the_mapped_frame() {
    let fixture = note_project(&[NoteSpec::at(1920, 480, 64, 100)]);
    let mut project = fixture.project.clone();
    slowed_project(&mut project);
    let snapshot =
        EngineSnapshot::from_project_with_tempo(&project, 1, &project.tempo()).expect("变速快照");
    // 38400 帧 = 300 量子，覆盖 36000 起点后的 2400 帧（50 ms）。
    let rendered = render_snapshot(snapshot, 300);
    assert!(
        rendered.left[35_000..36_000].iter().all(|&s| s == 0.0),
        "36 000 帧之前不许有声音（恒定 BPM 的 48 000 起点会在这里漏音）"
    );
    assert!(
        rendered.left[36_000..].iter().any(|&s| s != 0.0),
        "36 000 帧之后必须出声"
    );
}
