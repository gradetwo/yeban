//! 真实 `.yeban` 工程在**活窗口**里的导入 / 编辑 / 混音端到端判据。
//!
//! 交付件 `/home/crow/work/qoder/夜半霓虹_v5_母带交付.zip` 内的 `.yeban`
//! （17 轨 / 2867 音符 / 10 段落 / 6336 语义元素）经环境变量 `YEBAN_REAL_PROJECT`
//! 注入：`YEBAN_REAL_PROJECT=<path> cargo test -p yeban-app --test aether_real_project_live`。
//!
//! CI 上没有这件 120MB 资产 ⇒ 退回 `samples::filled_project()`（必要时补一个 MIDI
//! 片段）走**完全相同的链路**。⛔ 这不是跳过：两种输入下每一条断言都真跑、真改工程、
//! 真读回 UI 属性；区别只在工程规模。

use std::collections::BTreeMap;
use std::rc::Rc;

use slint::Model;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::open::open_project_file;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{ClipContent, ClipPoolEntry};
use yeban_model::{EntityId, MidiNote, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn midi_clip_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .filter(|entry| matches!(entry.content, ClipContent::Midi { .. }))
        .count()
}

fn midi_note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum()
}

fn project_under_test() -> YebanProjectV1 {
    let mut project = match std::env::var("YEBAN_REAL_PROJECT") {
        Ok(path) => open_project_file(path).expect("真实 .yeban 容器必须能打开"),
        Err(_) => yeban_model::samples::filled_project(),
    };
    // 编辑判据需要一个 MIDI 片段作为落点；真工程自带，等价夹具可能不带。
    if midi_clip_count(&project) == 0 {
        let clip_id = EntityId::new();
        let note_id = EntityId::new();
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: "aether-live-clip".to_owned(),
                content: ClipContent::Midi {
                    notes: BTreeMap::from([(note_id, MidiNote::new(note_id, 0, 60, 960))]),
                },
            },
        );
    }
    project
}

/// 生产装配的同一条路径：`build_main_window` + `wire_mixer_edit` + `wire_aether_actions`。
#[test]
fn real_project_import_edit_and_mix_in_a_live_window() {
    let project = project_under_test();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-live>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口必须装配成功");
    let ui = live.ui();
    host::wire_mixer_edit(ui, &undo);
    host::wire_aether_actions(ui, Rc::clone(&undo));

    // ① 导入：真工程投影必须注入新 UI 的结构数组（不是 .slint 里的硬编码演示轨道）。
    assert!(view.tracks.len() >= 2, "工程必须有多轨，否则判据是空绿");
    assert_eq!(
        ui.get_tracks().row_count(),
        view.tracks.len(),
        "导入: tracks"
    );
    assert_eq!(ui.get_notes().row_count(), view.notes.len(), "导入: notes");
    assert_eq!(
        ui.get_track_mutes().row_count(),
        view.tracks.len(),
        "导入: 混音数组等长"
    );
    eprintln!(
        "[import] tracks={} clips={} notes={} ui-tracks={} ui-notes={}",
        view.tracks.len(),
        view.clips.len(),
        view.notes.len(),
        ui.get_tracks().row_count(),
        ui.get_notes().row_count()
    );

    let first_track = *undo
        .try_project()
        .expect("撤销端口有活跃工程")
        .tracks
        .keys()
        .next()
        .expect("工程至少一条轨道");

    // ② 混音 - 静音：新 UI 的 track-toggle-mute 必须提交真实 Op::SetTrackMute。
    // 注意: UI 的 track-ids 是显示序, 工程的 tracks 是按 EntityId 排序的 BTreeMap,
    // 两者不必同序 ⇒ 断言"工程里某一条轨的 mute 翻转"而不是钉死某一条。
    let mute_snapshot =
        |p: &YebanProjectV1| -> Vec<bool> { p.tracks.values().map(|track| track.mute).collect() };
    let mutes_before = mute_snapshot(&undo.try_project().unwrap());
    ui.invoke_track_toggle_mute(1);
    let mutes_after = mute_snapshot(&undo.try_project().unwrap());
    assert_ne!(mutes_before, mutes_after, "静音必须改工程，不是只改视图态");
    eprintln!("[mix/mute] {mutes_before:?} -> {mutes_after:?}");

    // ③ 混音 - 推子：track-volume-changed 必须提交 Op::SetParam(TrackVolume)。
    ui.invoke_track_volume_changed(1, -6.0);
    let volumes: Vec<f32> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|track| track.volume_db)
        .collect();
    assert!(
        volumes.iter().any(|db| (db - (-6.0)).abs() < 1e-3),
        "推子 dB 必须落到工程某条轨，实测 {volumes:?}"
    );
    eprintln!("[mix/fader] volumes={volumes:?}");

    // ④ 混音 - 声相：Op::SetParam(TrackPan) 落到工程，重投影后 UI 的 track-pans 跟上。
    let pan_before = undo.try_project().unwrap().tracks[&first_track].pan;
    let new_pan = if pan_before > 0.0 { -0.5 } else { 0.5 };
    let pan_op = yeban_model::Op::SetParam {
        target: yeban_model::AutomationTarget::TrackPan {
            track_id: first_track,
        },
        old_val: pan_before,
        new_val: new_pan,
    };
    undo.commit_ops(NOW_MS + 1, "aether: 声相", vec![pan_op])
        .expect("声相 Op 必须被接受");
    let pan_after = undo.try_project().unwrap().tracks[&first_track].pan;
    assert!((pan_after - new_pan).abs() < 1e-3, "声相必须落到工程");
    let panned_view = ViewState::from_project(&undo.try_project().unwrap()).expect("重投影");
    host::apply_view(ui, &panned_view, 1440.0, 0.0);
    assert_eq!(
        ui.get_track_pans().row_count(),
        view.tracks.len(),
        "重投影后声相数组必须等长"
    );
    eprintln!("[mix/pan] {pan_before} -> {pan_after}");

    // ⑤ 编辑 - 新建音符：piano-note-create 必须提交 Op::AddNote 并重投影 UI。
    let notes_before = midi_note_count(&undo.try_project().unwrap());
    let ui_notes_before = ui.get_notes().row_count();
    ui.invoke_piano_note_create(3, 8);
    let notes_after = midi_note_count(&undo.try_project().unwrap());
    assert_eq!(notes_after, notes_before + 1, "新建音符必须改工程");
    assert_eq!(
        ui.get_notes().row_count(),
        ui_notes_before + 1,
        "重投影后 UI 的 notes 数组必须跟上工程"
    );
    eprintln!(
        "[edit/note] project {notes_before} -> {notes_after}, ui {ui_notes_before} -> {}",
        ui.get_notes().row_count()
    );
}
