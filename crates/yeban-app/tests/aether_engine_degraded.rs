//! 引擎不可用时的**降级**行为判据（报告 Q1 的界面侧后果）。
//!
//! 真实工程可能引用本机不存在的样本库（或机器没有声卡）⇒ `ProductionLoop::start`
//! 失败、产品退回 `without_engine()`。此时：
//!
//! * 工程照常打开、投影照常注入；
//! * 编辑（静音 / 推子 / 新建音符）**仍然提交真实 `Op`** —— ⛔ 不因引擎缺席而全停；
//! * 试听 / 监听如实报告“引擎未接线”，⛔ 不假装注入成功。
//!
//! 这是一条**界面侧**判据：它不改变引擎层“缺样本 ⇒ 启动失败”的既有语义
//! （该行为与主干共有，见报告的 Q1/Q5 归属）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::ProductionLoop;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{ClipContent, ClipPoolEntry};
use yeban_model::{EntityId, MidiNote, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

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

fn project_with_midi_clip() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    if midi_note_count(&project) == 0 {
        let clip_id = EntityId::new();
        let note_id = EntityId::new();
        project.clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: "engine-degraded-clip".to_owned(),
                content: ClipContent::Midi {
                    notes: BTreeMap::from([(note_id, MidiNote::new(note_id, 0, 60, 960))]),
                },
            },
        );
    }
    project
}

#[test]
fn edits_keep_working_when_the_engine_is_unavailable() {
    let project = project_with_midi_clip();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-degraded>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));

    // 产品在引擎启动失败时退到的同一条路（main.rs 的 `unwrap_or_else`）。
    let production = ProductionLoop::without_engine();
    assert!(!production.has_engine(), "本判据必须真的处在无引擎状态");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_mixer_edit(ui, &undo);
    host::wire_aether_actions(ui, Rc::clone(&undo), Rc::clone(&engine), view_cell);

    // ① 混音编辑仍然落工程（Op::SetTrackMute）。
    let mutes_before: Vec<bool> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|track| track.mute)
        .collect();
    ui.invoke_track_toggle_mute(1);
    let mutes_after: Vec<bool> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|track| track.mute)
        .collect();
    assert_ne!(
        mutes_before, mutes_after,
        "无引擎时静音仍然必须改工程（Q1：编辑不许跟着引擎停）"
    );
    eprintln!("[degraded/mute] {mutes_before:?} -> {mutes_after:?}");

    // ② 音符编辑仍然落工程（Op::AddNote）。
    let notes_before = midi_note_count(&undo.try_project().unwrap());
    ui.invoke_piano_note_create(3, 8);
    let notes_after = midi_note_count(&undo.try_project().unwrap());
    assert_eq!(
        notes_after,
        notes_before + 1,
        "无引擎时新建音符仍然必须提交 Op::AddNote"
    );
    eprintln!("[degraded/note] {notes_before} -> {notes_after}");

    // ③ 试听如实失败（不是假装注入了 0 条还报成功）。
    ui.invoke_chord_audition("Cmaj".into());
    assert!(
        !ui.get_module_status_succeeded(),
        "无引擎时试听必须报失败，实测 status={}",
        ui.get_module_status()
    );
    assert!(
        ui.get_module_status().contains("引擎未接线"),
        "试听回执必须点名引擎未接线，实测 {}",
        ui.get_module_status()
    );
    eprintln!("[degraded/audition] {}", ui.get_module_status());
}
