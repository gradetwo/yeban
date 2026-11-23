//! **卷帘音高窗口按作用域取景**回归判据（2026-11-23 Wang 实证）。
//!
//! 缺陷：音高窗口（`roll-base-pitch`）是粘性状态——只有「点轨道头选一条
//! MIDI 轨」一条路会按内容重取景；打开工程、选音频轨都让它停在历史位置。
//! 《雨声太吵》贝斯/鼓音高 28–50 在默认窗口 60–75 之外 ⇒ 选轨后卷帘全空；
//! 《失语》全音频轨时窗口停在 C1 一带，看起来与选中轨毫无关系。
//!
//! 本判据钉住 `apply_view` 的作用域取景（`host::reframe_roll_pitch_if_scope_changed`）：
//! ① 打开工程即按当前作用域（选中轨兜底）内容取景，低音轨音符落在窗内；
//! ② 同一作用域内手动改窗口（滚轮/八度平移）后重投影不回弹；
//! ③ 切到音频轨（作用域无音符）⇒ 窗口回中性默认，⛔ 不留上一轨的陈旧窗口；
//! ④ 切回 MIDI 轨 ⇒ 再次按其内容取景。
//!
//! 反向腿：删掉 `apply_view` 里的取景调用 ⇒ ① 红（base 停在默认 60，
//! 28–50 的音符全在窗外）；让取景无条件每帧重算 ⇒ ② 红（手动窗口被回弹）；
//! 让无音符作用域不复位 ⇒ ③ 红（base 留在上一轨的 31）。

use std::rc::Rc;

use slint::Model;
use yeban_app::bridge::{PITCH_LANE_COUNT, PITCH_LANE_HEIGHT_PX, ViewState};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, MidiNote, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;

/// 夹具：`filled_project` 的 Lead（MIDI）片段音符换成低音区 28/39/50
/// （跨度 22，照《雨声太吵》贝斯 28–50）；Bass 保持音频轨。
/// 返回（工程，Lead 轨身份，Bass 轨身份）。
fn low_bass_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    let mut lead_id = None;
    let mut bass_id = None;
    let mut lead_clip = None;
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            match &project.clip_pool[&placement.clip_id].content {
                ClipContent::Midi { .. } => {
                    lead_id = Some(track.id);
                    lead_clip = Some(placement.clip_id);
                }
                ClipContent::Audio { .. } => {
                    bass_id = Some(track.id);
                }
            }
        }
    }
    let lead_id = lead_id.expect("夹具必须有 MIDI 轨");
    let bass_id = bass_id.expect("夹具必须有音频轨");
    let lead_clip = lead_clip.expect("夹具必须有 MIDI 片段");
    let entry = project.clip_pool.get_mut(&lead_clip).expect("片段池条目");
    let ClipContent::Midi { notes } = &mut entry.content else {
        panic!("Lead 片段必须是 MIDI");
    };
    notes.clear();
    for (start, pitch) in [(0_u64, 28_u8), (960, 39), (1920, 50)] {
        let note_id = EntityId::new();
        notes.insert(note_id, MidiNote::new(note_id, start, pitch, 480));
    }
    (project, lead_id, bass_id)
}

/// 非主总线轨在界面下标序中的位置（`invoke_track_select` 的下标）。
fn track_index(project: &YebanProjectV1, track_id: EntityId) -> i32 {
    let mut ids: Vec<EntityId> = project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id)
        .map(|track| track.id)
        .collect();
    ids.sort();
    ids.iter()
        .position(|id| *id == track_id)
        .expect("轨必须在下标序里") as i32
}

fn live_with_port(project: &YebanProjectV1) -> (LivePort<MainWindow>, Rc<UndoPort>, ViewState) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<roll-scope-framing>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_track_select(live.ui(), &port);
    (live, port, view)
}

#[test]
fn roll_pitch_window_frames_the_scope_and_follows_selection() {
    let (project, lead_id, bass_id) = low_bass_project();
    let (live, _port, view) = live_with_port(&project);
    let ui = live.ui();

    // ① 打开工程：作用域（兜底选中 Lead）音高 28–50，跨度 22 ≥ 16 ⇒ 居中取景
    // base = 28 + 22/2 − 16/2 = 31，窗口 31–46；三条音符中 39 落在窗内、
    // 28/50 按「两端各露一半」在窗外（可纵向滚动），但窗口必须已离开默认 60。
    assert_eq!(ui.get_roll_base_pitch(), 31, "打开工程应按 Lead 音区取景");
    assert_eq!(ui.get_roll_min_pitch(), 31);
    assert_eq!(ui.get_roll_max_pitch(), 46);
    let ys: Vec<f32> = ui.get_note_ys().iter().collect();
    assert!(
        ys.iter()
            .any(|y| *y >= 0.0 && *y < PITCH_LANE_COUNT as f32 * PITCH_LANE_HEIGHT_PX),
        "取景后至少窗内音符的 y 必须落在 16 行网格内，实际 ys = {ys:?}"
    );

    // ② 同一作用域内手动把窗口移到 20，重投影不得回弹。
    ui.set_roll_base_pitch(20);
    host::apply_view(ui, &view, 1440.0, 0.0);
    assert_eq!(
        ui.get_roll_base_pitch(),
        20,
        "同一作用域内手动窗口必须保持（取景只在作用域变化时发生）"
    );

    // ③ 切到音频轨 Bass：作用域无音符 ⇒ 窗口回中性默认 60，音符集为空。
    ui.invoke_track_select(track_index(&project, bass_id));
    assert_eq!(
        ui.get_roll_base_pitch(),
        60,
        "选音频轨后窗口应回中性默认，不留 Lead 的陈旧窗口"
    );
    assert_eq!(ui.get_note_ulids().row_count(), 0, "音频轨作用域无音符");

    // ④ 切回 Lead：再次按 28–50 取景到 31。
    ui.invoke_track_select(track_index(&project, lead_id));
    assert_eq!(ui.get_roll_base_pitch(), 31, "切回低音轨应重新按其音区取景");
    let framed = ui.get_roll_framed_scope().to_string();
    assert!(
        framed.starts_with("track:"),
        "取景键应记为选中轨作用域，实际 {framed}"
    );
}
