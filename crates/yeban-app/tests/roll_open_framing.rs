//! **打开工程 → 钢琴卷帘取景/选中**回归判据（T-FIX-193 的 D2/D3/D4）。
//!
//! 三条缺陷都发生在「打开一份工程」这一跳：
//!
//! - **D2**：卷帘取景键只有 `track:{id}` / `clip:{id}`，不带工程身份；demo 与仓库夹具
//!   共用同一批 `01J8ZQ…` 身份序 ⇒ 换工程后键相同，音高窗口停在上一个工程的值
//!   （实测 58，应为 31）。修法：`open_archive_runtime` 换工程时重置 `roll-framed-scope`。
//! - **D3**：默认选中轨按身份序取首条非主总线轨；民谣三首首轨是音频轨「人声·参考」
//!   ⇒ 打开第一眼卷帘必然空。修法：默认选中**有 MIDI 音符**的轨（没有才回落旧口径）。
//! - **D4**：选轨横向落点用摆放 `start_tick`；尼龙·扫弦摆放 start=0 而首音符在
//!   tick 80636 ⇒ 滚到 0 之后窗口内音符数仍是 0。修法：落点取**首个音符** tick。
//!
//! 判据只测这三个可观测读数（窗口 base / 选中轨身份 / 横向 scroll），不碰 D1（投影钳位）
//! 与 D5（网格高度）—— 那两条在 T-FIX-192。
//!
//! ⚠ 本文件只有一个 `#[test]`：`slint::platform::set_platform` 是线程局部的，
//! `Tier1Window::install` 每线程只能成功一次（与 `tests/open_project_ui.rs` 同一条约束）。

use std::path::PathBuf;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::host::{self, OpenProjectRuntime};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{ClipContent, TrackKind};
use yeban_model::{EntityId, MidiNote, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const WINDOW: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_760_000_000_000;

fn track_named(project: &YebanProjectV1, name: &str) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| track.name == name)
        .unwrap_or_else(|| panic!("夹具必须有 {name} 轨"))
        .id
}

/// 把一条 MIDI 轨的**第一个** MIDI 片段替换成 `notes`（清空其余摆放）。
fn set_midi_notes(project: &mut YebanProjectV1, track_id: EntityId, notes: &[(u64, u8)]) {
    let mut clip_id = None;
    for placement in project.tracks.get(&track_id).expect("轨").clips.values() {
        if matches!(
            &project.clip_pool[&placement.clip_id].content,
            ClipContent::Midi { .. }
        ) {
            clip_id = Some(placement.clip_id);
            break;
        }
    }
    let clip_id = clip_id.expect("夹具必须已有 MIDI 片段（身份保持不变）");
    let ClipContent::Midi { notes: slot } = &mut project
        .clip_pool
        .get_mut(&clip_id)
        .expect("片段池条目")
        .content
    else {
        unreachable!("上面已判过是 MIDI");
    };
    slot.clear();
    for (start_tick, pitch) in notes {
        let note_id = EntityId::new();
        slot.insert(note_id, MidiNote::new(note_id, *start_tick, *pitch, 480));
    }
}

/// 把 Lead 的 MIDI 摆放起点改成 `start_tick`（D4 的「摆放从 0 起」）。
fn set_lead_placement_start(project: &mut YebanProjectV1, track_id: EntityId, start_tick: u64) {
    for placement in project
        .tracks
        .get_mut(&track_id)
        .expect("轨")
        .clips
        .values_mut()
    {
        placement.start_tick = start_tick;
    }
}

/// D3 夹具：身份序第一条非主总线轨是**音频轨**，MIDI 音符在身份序第二条。
fn audio_first_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    let lead_id = track_named(&project, "Lead");
    let bass_id = track_named(&project, "Bass");
    let lead_clips = project.tracks.get(&lead_id).expect("Lead").clips.clone();
    let bass_clips = project.tracks.get(&bass_id).expect("Bass").clips.clone();
    {
        let lead = project.tracks.get_mut(&lead_id).expect("Lead");
        lead.kind = TrackKind::Audio;
        lead.clips = bass_clips;
    }
    {
        let bass = project.tracks.get_mut(&bass_id).expect("Bass");
        bass.kind = TrackKind::Midi;
        bass.clips = lead_clips;
    }
    (project, lead_id, bass_id)
}

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-roll-open-framing-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn write_container(dir: &std::path::Path, name: &str, project: &YebanProjectV1) -> PathBuf {
    let path = dir.join(name);
    // T-FIX-330：这些工程来自 `filled_project`，音频片段引用运行期合成的 kick。
    // 打开已改为严格资产核对 ⇒ 夹具必须带上**同一份**资产字节，否则 UI 会正确地
    // 报 `MissingAsset` / 回执失败（旧夹具正是"缺资产仍静默打开"的温床）。
    let archive = yeban_model::container::ProjectArchive {
        project: project.clone(),
        history_dag: Vec::new(),
        proposals: None,
        assets: yeban_model::samples::filled_project_assets(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };
    yeban_app::save::save_archive_file(&archive, &path).expect("写出一份真容器");
    path
}

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

#[test]
fn opening_a_project_reframes_the_roll_selects_a_midi_track_and_scrolls_to_the_first_note() {
    let dir = scratch_dir();

    // ── A / B：同一批轨道身份（= demo 与夹具共用 id 序），只有音高不同 ──
    let mut a = yeban_model::samples::filled_project();
    let lead_id = track_named(&a, "Lead");
    set_midi_notes(&mut a, lead_id, &[(0, 58)]);
    let mut b = a.clone();
    set_midi_notes(&mut b, lead_id, &[(0, 31)]);

    // ── C：首轨音频、MIDI 在第二轨（D3） ──
    let (mut c, c_audio_id, c_midi_id) = audio_first_project();
    set_midi_notes(&mut c, c_midi_id, &[(0, 31)]);
    assert!(c_audio_id < c_midi_id, "夹具：音频轨必须身份序在前");

    // ── D：摆放 start=0，首个音符在 tick 80636（D4） ──
    let mut d = yeban_model::samples::filled_project();
    let d_lead = track_named(&d, "Lead");
    set_midi_notes(&mut d, d_lead, &[(80_636, 60)]);
    set_lead_placement_start(&mut d, d_lead, 0);

    let path_a = write_container(&dir, "a.yeban", &a);
    let path_b = write_container(&dir, "b.yeban", &b);
    let path_c = write_container(&dir, "c.yeban", &c);
    let path_d = write_container(&dir, "d.yeban", &d);

    // ── 装配：真实窗口 + 真实端口 + 生产形态的「打开」实现 ──
    let view = ViewState::from_project(&a).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<roll-open-framing>", "yeban-app", a.clone(), NOW_MS).expect("打开会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let ui = live.ui();
    let after: host::OpenProjectAfter = Rc::new(|_, _, _, _| Ok(()));
    let runtime = host::PortOpenProject {
        port: Rc::clone(&port),
        production: None,
        quanta: 0,
        after,
    };
    // 选轨联动（`roll_focus_on_track`）挂在 `on_track_select` 上；与
    // `tests/roll_scope_framing.rs` 同一条装配。
    host::wire_track_select(ui, &port);

    // ① 打开 A：单音 58（跨度 0）⇒ base = 58 − 2 = 56。
    runtime.open_runtime(ui, &path_a).expect("打开 A");
    assert_eq!(ui.get_roll_base_pitch(), 56, "打开 A 后应按 58 取景");
    let scope_a = ui.get_roll_framed_scope().to_string();
    assert!(scope_a.starts_with("track:"), "A 的取景键: {scope_a}");

    // ② D2：打开 B（同一批轨道身份、音符 31）⇒ 必须重取景到 29，⛔ 不停在 56。
    runtime.open_runtime(ui, &path_b).expect("打开 B");
    assert_eq!(
        ui.get_roll_base_pitch(),
        29,
        "换工程后取景键相同也必须按新工程内容重取景（D2）"
    );

    // ③ D3：打开 C（首轨音频）⇒ 默认选中第二轨 MIDI，而不是身份序首条音频轨。
    runtime.open_runtime(ui, &path_c).expect("打开 C");
    assert_eq!(
        ui.get_selected_track_id().to_string(),
        c_midi_id.to_canonical_string(),
        "打开工程默认选中轨必须优先有 MIDI 音符的轨（D3）"
    );
    assert_eq!(ui.get_roll_base_pitch(), 29, "C 的 MIDI 轨是 31 ⇒ base 29");

    // ④ D4：打开 D 后选轨 ⇒ 横向滚到首音符 tick 80636（⛔ 不是摆放 0）。
    runtime.open_runtime(ui, &path_d).expect("打开 D");
    ui.invoke_track_select(track_index(&d, d_lead));
    let tpp = host::roll_ticks_per_pixel(ui);
    let expected = yeban_app::bridge::tick_to_px(80_636, tpp).expect("像素换算");
    let expected = (expected as f32 - 16.0).max(0.0);
    let scroll = ui.get_roll_scroll_x();
    assert!(
        (scroll - expected).abs() < 1.0,
        "选轨横向落点必须取首个音符（期望 {expected}，实际 {scroll}）（D4）"
    );
    assert!(
        scroll > 0.0,
        "摆放 start=0 但首音符在 80636 ⇒ 滚动量必须 > 0"
    );

    // ⑤ ⭐ T-FIX-2392：**打开工程**的横向落点也要落在首音符（《回头路》鼓刷军鼓 89280）。
    let mut e = yeban_model::samples::filled_project();
    let e_lead = track_named(&e, "Lead");
    set_midi_notes(&mut e, e_lead, &[(89_280, 60)]);
    set_lead_placement_start(&mut e, e_lead, 0);
    let path_e = write_container(&dir, "e.yeban", &e);
    runtime.open_runtime(ui, &path_e).expect("打开 E");
    let tpp = host::roll_ticks_per_pixel(ui);
    let expected =
        (yeban_app::bridge::tick_to_px(89_280, tpp).expect("像素换算") as f32 - 16.0).max(0.0);
    assert!(
        (ui.get_roll_scroll_x() - expected).abs() < 1.0,
        "打开工程后横向落点必须是默认轨首音符（期望 {expected}，实际 {}）（T-FIX-2392）",
        ui.get_roll_scroll_x()
    );

    // ⑥ 对照：没有任何 MIDI 音符的工程 ⇒ 横向落 0（不报错、不 panic）。
    let mut f = yeban_model::samples::filled_project();
    // `filled_project` 里只有 Lead 有 MIDI 片段（Bass 是音频轨、Aux 为空）⇒ 清空它就够了。
    let f_lead = track_named(&f, "Lead");
    set_midi_notes(&mut f, f_lead, &[]);
    let path_f = write_container(&dir, "f.yeban", &f);
    runtime
        .open_runtime(ui, &path_f)
        .expect("打开无音符工程不得失败");
    assert_eq!(
        ui.get_roll_scroll_x(),
        0.0,
        "无 MIDI 音符时横向落点必须是 0"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
