//! **T-FIX-216 / ISS-MWVUAFHB-F805C4** 回归判据：`Cmd/Ctrl+A` 全选必须尊重
//! 卷帘当前的**纵向音高窗口**（16 条泳道），不能把视野外的音高也选进去。
//!
//! 现场：同一片段放 pitch 36 / 60 / 84 三颗，卷帘基音 = 60 ⇒ 窗口 60..=75。
//! 全选后只能有 1 颗（60）；随后一次 Delete 只能删掉那一颗，36 / 84 必须留在工程里。
//!
//! ⚠ 单 `#[test]`：Slint 平台每线程只能装一次。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model;

use yeban_app::bridge::ViewState;
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

/// 夹具：清空所有 MIDI 片段的音符，只在同一个片段里放 36 / 60 / 84 三颗。
/// 返回工程与 pitch 60 那颗的身份（全选唯一应命中的那颗）。
fn three_octave_project() -> (YebanProjectV1, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    let midi_clips: Vec<EntityId> = project
        .clip_pool
        .iter()
        .filter(|(_, entry)| matches!(entry.content, ClipContent::Midi { .. }))
        .map(|(id, _)| *id)
        .collect();
    let clip_id = midi_clips.first().copied().expect("夹具必须有 MIDI 片段");
    for id in &midi_clips {
        let entry = project.clip_pool.get_mut(id).expect("片段池条目");
        if let ClipContent::Midi { notes } = &mut entry.content {
            notes.clear();
        }
    }
    let ClipContent::Midi { notes } = &mut project
        .clip_pool
        .get_mut(&clip_id)
        .expect("片段池条目")
        .content
    else {
        unreachable!();
    };
    let mut id_60 = None;
    for (start, pitch) in [(0_u64, 36_u8), (960, 60), (1920, 84)] {
        let id = EntityId::new();
        if pitch == 60 {
            id_60 = Some(id);
        }
        notes.insert(id, MidiNote::new(id, start, pitch, 480));
    }
    (project, id_60.expect("夹具必须有 pitch 60"))
}

fn midi_pitches(project: &YebanProjectV1) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .clip_pool
        .values()
        .flat_map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.values().map(|note| note.pitch).collect(),
            ClipContent::Audio { .. } => Vec::new(),
        })
        .collect();
    pitches.sort_unstable();
    pitches
}

#[test]
fn select_all_only_takes_notes_inside_the_sixteen_lane_pitch_window() {
    let (project, id_60) = three_octave_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<select-all-pitch-window>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台");
    let ui = live.ui();
    host::wire_roll_edit(ui, &port);
    host::wire_keys(
        ui,
        Rc::new(RefCell::new(InputContext::new())),
        Some(Rc::clone(&port)),
        None,
        Some(Rc::new(project.clone())),
    );

    // 把卷帘纵向窗口固定在默认基音 60 ⇒ 16 条泳道对应 60..=75。
    ui.set_roll_base_pitch(60);
    assert_eq!(ui.get_roll_base_pitch(), 60, "夹具窗口基音必须是 60");

    // ① Cmd/Ctrl+A：36 与 84 在窗口外，只能选中 60 那一颗。
    ui.invoke_key_action("a".into(), false, true, false, false);
    assert_eq!(
        ui.get_selected_note_count(),
        1,
        "全选必须只取 60..=75 窗口内的音符（36/84 不得被选中）"
    );
    let selected: Vec<String> = ui
        .get_selected_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        selected,
        vec![id_60.to_string()],
        "选中的身份必须正是 pitch 60 那颗"
    );

    // ② Delete：只删可见窗口里那颗；视野外的 36 / 84 必须留在工程里。
    ui.invoke_key_action("\u{7f}".into(), false, false, false, false);
    assert_eq!(
        midi_pitches(port.try_project().as_ref().expect("工程")),
        vec![36, 84],
        "Delete 不得作用到当前音高窗口之外的音符"
    );
}
