//! 新 UI 钢琴卷帘的**真实数据与编辑**判据（报告反馈 ①③④）。
//!
//! 旧实现是 16×64 的演示网格：音符来自 `root.notes`（所有轨/所有片段），
//! 点击映射到"第一条轨/第一个 MIDI 片段"，因此：
//! * 卷帘内容与选中轨/片段不一致；
//! * 点击无法落到真实工程（等于不能编辑）。
//!
//! 本判据把`MainWindow`的钢琴卷帘替换成 `console/piano_roll.slint` 之后钉住：
//! 1. 卷帘音符 = **选中片段**的音符（不是全工程）；
//! 2. 铅笔点击提交 `Op::AddNote` 到**该片段**，工程音符 +1；
//! 3. 横向缩放 / 滚动改的是同一条投影（`ticks-per-pixel` / `roll-scroll-x`）。

use std::cell::RefCell;
use std::rc::Rc;

use slint::{LogicalPosition, Model, platform::WindowEvent};
use yeban_app::bridge::{
    NOTE_INSET_X_PX, NOTE_INSET_Y_PX, PITCH_LANE_HEIGHT_PX, ROLL_HEADER_HEIGHT_PX,
    ROLL_KEY_COLUMN_WIDTH_PX, ViewState,
};
use yeban_app::host;
use yeban_app::host::ProductionLoop;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Size, UiTestPort};

const NOW_MS: u64 = 1_700_000_000_000;

fn note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum()
}

fn first_midi_clip(project: &YebanProjectV1) -> (EntityId, std::collections::BTreeSet<EntityId>) {
    for (id, entry) in &project.clip_pool {
        if let ClipContent::Midi { notes } = &entry.content {
            return (*id, notes.keys().copied().collect());
        }
    }
    panic!("夹具必须有 MIDI 片段");
}

#[test]
fn piano_roll_shows_the_selected_clip_and_edits_reach_the_project() {
    let project = yeban_model::samples::filled_project();
    let (clip_id, clip_notes) = first_midi_clip(&project);
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-roll>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::without_engine();
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_zoom_controls(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), engine, view_cell);

    // ① 选中那个片段（与用户点时间线块 / 片段头同一条入口）⇒ 卷帘只画它的音符。
    let placement = yeban_app::bridge::ViewState::from_project(&project)
        .unwrap()
        .clips
        .iter()
        .find(|clip| clip.clip_id == clip_id.to_canonical_string())
        .map(|clip| (clip.placement_id.clone(), clip.track_index))
        .expect("片段必须有摆放");
    ui.invoke_clip_select(placement.0.clone().into());
    let drawn = ui.get_note_ulids();
    let drawn_ids: Vec<String> = drawn.iter().map(|id| id.to_string()).collect();
    assert_eq!(
        drawn_ids.len(),
        clip_notes.len(),
        "卷帘音符数必须等于选中片段的音符数（不是全工程 {}）",
        note_count(&project)
    );
    for id in &drawn_ids {
        assert!(
            clip_notes
                .iter()
                .any(|note| note.to_canonical_string() == *id),
            "卷帘出现了片段外的音符 {id}"
        );
    }
    eprintln!(
        "[roll/scope] clip {} 条音符 -> 卷帘 {} 条",
        clip_notes.len(),
        drawn_ids.len()
    );

    // ①b 没有选中片段时，兜底作用域 = **选中轨**（不是全工程）。
    ui.set_selected_clip_ulid("".into());
    let selected_track = view.tracks[placement.1].id.clone();
    ui.set_selected_track_id(selected_track.clone().into());
    let track_view = host::project_with_view_state(&project, ui).expect("按选中轨重投影");
    host::apply_view(ui, &track_view, 1440.0, 0.0);
    let track_clip_ids: std::collections::BTreeSet<String> = view
        .clips
        .iter()
        .filter(|clip| clip.track_index == placement.1)
        .map(|clip| clip.clip_id.clone())
        .collect();
    let track_notes: usize = project
        .clip_pool
        .iter()
        .filter(|(id, _)| track_clip_ids.contains(&id.to_canonical_string()))
        .map(|(_, entry)| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(
        ui.get_note_ulids().row_count(),
        track_notes,
        "没有选中片段时，卷帘必须只画选中轨的音符（不是全工程 {}）",
        note_count(&project)
    );
    eprintln!(
        "[roll/track] 选中轨 {track_notes} 条音符 -> 卷帘 {} 条",
        ui.get_note_ulids().row_count()
    );
    // 回到片段作用域继续编辑断言。
    ui.invoke_clip_select(placement.0.clone().into());

    // ② 铅笔点击：坐标按卷帘的键盘列 / 行头 / 泳道换算；提交到同一片段。
    let notes_before = note_count(&undo.try_project().unwrap());
    ui.set_active_tool(2);
    let x = ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX + 40.0;
    let y = ROLL_HEADER_HEIGHT_PX + NOTE_INSET_Y_PX + PITCH_LANE_HEIGHT_PX * 3.0 + 4.0;
    ui.invoke_clicked(x, y);
    let notes_after = note_count(&undo.try_project().unwrap());
    assert!(
        notes_after > notes_before,
        "铅笔点击必须提交真实 Op::AddNote（{notes_before} -> {notes_after}）"
    );
    let edited = undo.try_project().unwrap();
    let in_clip = edited
        .clip_pool
        .get(&clip_id)
        .and_then(|entry| entry.content.notes())
        .map_or(0, std::collections::BTreeMap::len);
    assert_eq!(
        in_clip,
        clip_notes.len() + 1,
        "新音符必须落在选中片段里，实测 {in_clip}"
    );
    eprintln!("[roll/edit] project {notes_before} -> {notes_after}, clip = {in_clip}");

    // ③ 横向缩放 / 滚动：同一份投影（ticks-per-pixel / roll-scroll-x）。
    let tpp_before = ui.get_roll_ticks_per_pixel();
    ui.invoke_zoom_h_step(true);
    let tpp_after = ui.get_roll_ticks_per_pixel();
    assert!(
        tpp_after < tpp_before,
        "H+ 必须放大卷帘：{tpp_before} -> {tpp_after}"
    );
    // 夹具只有 1 小节：先连续放大，让内容宽超过视口，滚动才有可移动的余量。
    for _ in 0..6 {
        ui.invoke_zoom_h_step(true);
    }
    let scroll_before = ui.get_roll_scroll_x();
    ui.invoke_scroll_requested(160.0);
    let scroll_after = ui.get_roll_scroll_x();
    eprintln!(
        "[roll/zoom] tpp={} content={} viewport=1440 scroll {scroll_before}->{scroll_after}",
        ui.get_roll_ticks_per_pixel(),
        ui.get_arrangement_content_px()
    );
    assert!(
        scroll_after > scroll_before,
        "滚轮 / 拖动必须横向滚动卷帘：{scroll_before} -> {scroll_after}"
    );
    eprintln!("[roll/zoom] tpp {tpp_before}->{tpp_after}, scroll {scroll_before}->{scroll_after}");
}

#[test]
fn piano_roll_live_hold_release_octave_shift_and_audibility_warnings() {
    let mut project = yeban_model::samples::filled_project();
    // Ensure track 0 has no solo/mute blocking audition initially.
    for track in project.tracks.values_mut() {
        track.solo = false;
        track.mute = false;
        track.volume_db = 0.0;
    }
    let (clip_id, _) = first_midi_clip(&project);
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-roll-live>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("启动实时引擎");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_mixer_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), Rc::clone(&engine), view_cell);

    let (placement_id, target_track_idx) = view
        .clips
        .iter()
        .find(|clip| clip.clip_id == clip_id.to_canonical_string())
        .map(|clip| (clip.placement_id.clone(), clip.track_index))
        .expect("片段必须有摆放");
    ui.set_selected_track_id(view.tracks[target_track_idx].id.clone().into());
    ui.invoke_clip_select(placement_id.into());

    // 1. Real hold-and-release piano key audition while transport is stopped.
    ui.invoke_piano_key_pressed(15); // C4 (60)
    assert!(ui.get_module_status_succeeded());
    assert!(ui.get_module_status().as_str().contains("C4"));

    let mut held_buf = vec![0.0_f32; 512 * 2];
    engine.borrow_mut().render_into(&mut held_buf, 2, 4);
    let held_peak = held_buf
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        held_peak > 1e-4,
        "按住琴键时必须持续出声，实际峰值={held_peak}"
    );

    // Sustained hold stays audible across additional blocks before release.
    let mut sustain_buf = vec![0.0_f32; 512 * 2];
    engine.borrow_mut().render_into(&mut sustain_buf, 2, 8);
    let sustain_peak = sustain_buf
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(sustain_peak > 1e-4, "持续按住尚未松手时仍应有音频输出");

    ui.invoke_piano_key_released(15);
    let tail_quanta =
        ((48_000 * 2) / yeban_app::engine_host::EngineHost::quantum_frames().max(1)) as u64;
    engine.borrow_mut().pump(tail_quanta);
    let mut tail_buf = vec![0.0_f32; 512 * 2];
    engine.borrow_mut().render_into(&mut tail_buf, 2, 8);
    let tail_end_peak = tail_buf
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        tail_end_peak < 1e-4,
        "松手释放 NoteOff 后音尾应衰减归零，实际={tail_end_peak}"
    );

    // 2. Octave shift changes pitch labels, range readout, note Y projections, and pencil pitch.
    let ys_c4: Vec<f32> = ui.get_note_ys().iter().collect();
    ui.invoke_piano_roll_octave_shift(-12);
    assert_eq!(ui.get_roll_base_pitch(), 48);
    assert_eq!(ui.get_roll_pitch_range_label().as_str(), "C3 – Eb4");
    assert_eq!(
        ui.get_roll_lane_key_labels().row_data(15).unwrap().as_str(),
        "C3"
    );
    let ys_c3: Vec<f32> = ui.get_note_ys().iter().collect();
    if !ys_c4.is_empty() && !ys_c3.is_empty() {
        assert_ne!(ys_c4, ys_c3, "切换八度后可见音符的 Y 坐标必须同步重投影");
    }

    // Clamping down/up stays in 24..=96 (C1..C7).
    for _ in 0..8 {
        ui.invoke_piano_roll_octave_shift(-12);
    }
    assert_eq!(ui.get_roll_base_pitch(), 24);
    for _ in 0..10 {
        ui.invoke_piano_roll_octave_shift(12);
    }
    assert_eq!(ui.get_roll_base_pitch(), 96);
    // Reset back to C3 (48) and pencil-draw on lane 15 -> must add pitch 48.
    for _ in 0..4 {
        ui.invoke_piano_roll_octave_shift(-12);
    }
    assert_eq!(ui.get_roll_base_pitch(), 48);
    ui.set_active_tool(2);
    let draw_x = ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX + 20.0;
    let draw_y = ROLL_HEADER_HEIGHT_PX + NOTE_INSET_Y_PX + PITCH_LANE_HEIGHT_PX * 15.0 + 4.0;
    ui.invoke_clicked(draw_x, draw_y);
    let updated = undo.try_project().expect("工程");
    let has_c3 = updated
        .clip_pool
        .get(&clip_id)
        .and_then(|e| e.content.notes())
        .is_some_and(|notes| notes.values().any(|n| n.pitch == 48));
    assert!(
        has_c3,
        "切换到 C3 音域后用铅笔点最底行必须写入 MIDI 48 (C3)"
    );

    // 3. Muted track emits visible audibility warning on piano key press, then unmuting clears it.
    ui.invoke_track_toggle_mute(target_track_idx as i32 + 1);
    ui.invoke_piano_key_pressed(15);
    assert!(ui.get_module_status_succeeded());
    assert!(
        ui.get_roll_edit_status().as_str().contains("静音"),
        "静音轨道试听必须在状态提示用户，实际={}",
        ui.get_roll_edit_status()
    );
    ui.invoke_piano_key_released(15);

    ui.invoke_track_toggle_mute(target_track_idx as i32 + 1);
    ui.invoke_piano_key_pressed(15);
    assert_eq!(ui.get_roll_edit_status().as_str(), "");
    ui.invoke_piano_key_released(15);
}

#[test]
fn piano_roll_supports_2d_trackpad_vertical_pitch_and_horizontal_timeline_scroll() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-roll-2d-scroll>", "yeban-app", project, NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let mut live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("主窗口装配成功");
    {
        let ui = live.ui();
        ui.set_show_lower_dock(true);
        ui.invoke_apply_dock_tab("pianoroll".into());
        host::wire_roll_edit(ui, &undo);
        host::apply_view(ui, &view, 1440.0, 0.0);
    }

    live.refresh_tree(None).expect("刷新控件树");
    let ui = live.ui();
    let roll_bounds = live
        .tree()
        .find_by_id("piano-roll-scroll-area")
        .expect("piano-roll-scroll-area 必须在树里")
        .bounds
        .expect("piano-roll-scroll-area 必须有几何");
    let grid_pt = LogicalPosition::new(
        roll_bounds.x as f32 + (roll_bounds.width as f32 * 0.5),
        roll_bounds.y as f32 + (roll_bounds.height as f32 * 0.5),
    );
    let key_col_pt = LogicalPosition::new(
        roll_bounds.x as f32 + 24.0,
        roll_bounds.y as f32 + (roll_bounds.height as f32 * 0.5),
    );

    let target_note_idx = (0..ui.get_note_positions().row_count())
        .find(|&idx| ui.get_note_positions().row_data(idx).unwrap_or(0.0) >= 96.0)
        .expect("filled_project 至少应有一个 x>=96px 的音符");
    let target_note_ulid = ui
        .get_note_ulids()
        .row_data(target_note_idx)
        .expect("target_note_ulid");
    let target_note_x_before = ui
        .get_note_positions()
        .row_data(target_note_idx)
        .unwrap_or(0.0);
    let ys_before: Vec<f32> = ui.get_note_ys().iter().collect();
    assert_eq!(ui.get_roll_base_pitch(), 60);
    assert_eq!(ui.get_roll_scroll_x(), 0.0);

    // 1. 双指左右滑动（delta_x != 0，无修饰）平滑滚动时间轴
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: -84.0,
        delta_y: 0.0,
    });
    assert!(
        (ui.get_roll_scroll_x() - 84.0).abs() < 0.5,
        "双指右滑 delta_x=-84 应滚动 roll_scroll_x 至 84，实际={}",
        ui.get_roll_scroll_x()
    );
    let scrolled_idx = (0..ui.get_note_ulids().row_count())
        .find(|&idx| ui.get_note_ulids().row_data(idx).as_ref() == Some(&target_note_ulid))
        .expect("滚动后目标音符仍应在可见视口内");
    let target_note_x_after = ui
        .get_note_positions()
        .row_data(scrolled_idx)
        .unwrap_or(0.0);
    assert!(
        (target_note_x_after - (target_note_x_before - 84.0)).abs() < 1.0,
        "滚动后目标音符绘制 x 必须左移 84px: before={target_note_x_before}, after={target_note_x_after}"
    );

    live.window().dispatch(WindowEvent::PointerScrolled {
        position: key_col_pt,
        delta_x: 40.0,
        delta_y: 0.0,
    });
    assert!(
        (ui.get_roll_scroll_x() - 44.0).abs() < 0.5,
        "琴键列区域向左滑动 delta_x=+40 应回退至 44，实际={}",
        ui.get_roll_scroll_x()
    );

    // 2. 双指上下滑动（delta_y != 0，无修饰）纵向移动音高视口并同步重投影 note_ys
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: 0.0,
        delta_y: -28.0,
    });
    assert_eq!(
        ui.get_roll_base_pitch(),
        58,
        "向下滚动 28px (=2*14px) 必须使基音下降 2 半音 (60->58)"
    );
    let ys_after: Vec<f32> = ui.get_note_ys().iter().collect();
    if !ys_before.is_empty() && !ys_after.is_empty() {
        assert_ne!(
            ys_before, ys_after,
            "纵向滚动音高视野后可见音符的 Y 坐标必须同步重投影"
        );
    }

    // 3. 精细触控板连续微小增量累积跨过 14px 时触发 1 半音步进
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: 0.0,
        delta_y: -6.0,
    });
    assert_eq!(ui.get_roll_base_pitch(), 58);
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: 0.0,
        delta_y: -10.0,
    });
    assert_eq!(ui.get_roll_base_pitch(), 57);

    // 4. 对角线双指手势同时推进横向与纵向
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: -20.0,
        delta_y: 28.0,
    });
    assert!((ui.get_roll_scroll_x() - 64.0).abs() < 0.5);
    assert_eq!(ui.get_roll_base_pitch(), 59);

    // 5. 覆盖全 MIDI 0..=127 音域（base_pitch 夹紧在 0..=112）
    for _ in 0..10 {
        live.window().dispatch(WindowEvent::PointerScrolled {
            position: grid_pt,
            delta_x: 0.0,
            delta_y: -140.0,
        });
    }
    assert_eq!(ui.get_roll_base_pitch(), 0);
    assert_eq!(
        ui.get_roll_lane_key_labels().row_data(15).as_deref(),
        Some("C-1")
    );

    for _ in 0..20 {
        live.window().dispatch(WindowEvent::PointerScrolled {
            position: grid_pt,
            delta_x: 0.0,
            delta_y: 140.0,
        });
    }
    assert_eq!(ui.get_roll_base_pitch(), 112);
    assert_eq!(
        ui.get_roll_lane_key_labels().row_data(0).as_deref(),
        Some("G9")
    );
}

#[test]
fn piano_roll_auditions_and_inserts_notes_when_track_has_no_midi_clip_no_synth_or_is_audio() {
    // 1. Startup demo_project(): Track 0 ("鼓", Midi) has solo=true; Track 1 ("贝斯") is Audio with no synth;
    //    Track 3 ("主音") is Midi with 0 clips and 0 devices.
    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("demo_project 投影成功");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<aether-roll-no-midi>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("启动实时引擎");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_undo(ui, &undo);
    host::wire_mixer_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), Rc::clone(&engine), view_cell);

    let empty_midi_track_id = view
        .tracks
        .iter()
        .find(|t| t.name == "主音")
        .map(|t| t.id.clone())
        .expect("demo_project 必须有 Track 3 主音");
    let audio_track_id = view
        .tracks
        .iter()
        .find(|t| t.name == "贝斯")
        .map(|t| t.id.clone())
        .expect("demo_project 必须有 Track 1 贝斯");

    // 1a) Empty MIDI track (no clip, no synth device, blocked by Track 0 solo):
    //     pressing key auto-installs instrument on Track 3 AND produces real audible output.
    ui.set_selected_track_id(empty_midi_track_id.clone().into());
    ui.invoke_piano_key_pressed(15);
    assert!(ui.get_module_status_succeeded());
    let mut empty_midi_buf = vec![0.0_f32; 512 * 2];
    engine.borrow_mut().render_into(&mut empty_midi_buf, 2, 4);
    let empty_midi_peak = empty_midi_buf
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        empty_midi_peak > 1e-4,
        "无片段/无合成器设备且存在其它轨 Solo 时，空 MIDI 轨琴键仍应发声，实际={empty_midi_peak}"
    );
    ui.invoke_piano_key_released(15);
    let tail_quanta =
        ((48_000 * 2) / yeban_app::engine_host::EngineHost::quantum_frames().max(1)) as u64;
    engine.borrow_mut().pump(tail_quanta);

    let after_key = undo.try_project().expect("工程存在");
    let empty_midi_entity = empty_midi_track_id
        .parse::<EntityId>()
        .expect("合法 track id");
    assert!(
        !after_key.tracks[&empty_midi_entity].devices.is_empty(),
        "空 MIDI 轨试听后应自动装载默认内置合成器"
    );

    // 1b) Drawing on the Piano Roll grid on empty MIDI track automatically creates MIDI clip + inserts note.
    let notes_before = note_count(&after_key);
    let commits_before = undo.display().commit_count;
    ui.set_active_tool(2);
    let draw_x = ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX + 24.0;
    let draw_y = ROLL_HEADER_HEIGHT_PX + NOTE_INSET_Y_PX + PITCH_LANE_HEIGHT_PX * 10.0 + 4.0;
    ui.invoke_clicked(draw_x, draw_y);
    let after_draw = undo.try_project().expect("工程存在");
    assert_eq!(note_count(&after_draw), notes_before + 1);
    assert_eq!(
        undo.display().commit_count,
        commits_before + 1,
        "画音符+建片段应是单步可撤销"
    );
    assert!(
        !after_draw.tracks[&empty_midi_entity].clips.is_empty(),
        "空 MIDI 轨点击网格后应自动创建 MIDI 片段摆放"
    );
    assert!(
        ui.get_note_ulids().row_count() >= 1,
        "自动创建的 MIDI 片段与音符必须立即投影到卷帘"
    );

    // 2) Audio track selected ("贝斯"):
    //    pressing piano keys falls back to an audible synth and produces sound;
    //    double-clicking / drawing inserts a new MIDI track + clip + note atomically (and undoes cleanly).
    ui.set_selected_track_id(audio_track_id.into());
    ui.set_selected_clip_ulid("".into());
    ui.invoke_piano_key_pressed(10);
    assert!(ui.get_module_status_succeeded());
    let mut audio_sel_buf = vec![0.0_f32; 512 * 2];
    engine.borrow_mut().render_into(&mut audio_sel_buf, 2, 4);
    let audio_sel_peak = audio_sel_buf
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        audio_sel_peak > 1e-4,
        "选中音频轨时点击钢琴卷帘琴键必须回退到可听合成器发声，实际={audio_sel_peak}"
    );
    ui.invoke_piano_key_released(10);
    engine.borrow_mut().pump(tail_quanta);

    let tracks_before = undo.try_project().unwrap().tracks.len();
    let notes_before_audio = note_count(&undo.try_project().unwrap());
    let commits_before_audio = undo.display().commit_count;
    ui.set_active_tool(1); // Pointer tool double-click inserts a default note too
    ui.invoke_double_clicked(draw_x + 48.0, draw_y);
    let after_audio_insert = undo.try_project().expect("工程存在");
    assert_eq!(
        after_audio_insert.tracks.len(),
        tracks_before + 1,
        "选中音频轨在卷帘写音符时必须自动创建新的 MIDI 轨"
    );
    assert_eq!(note_count(&after_audio_insert), notes_before_audio + 1);
    assert_eq!(undo.display().commit_count, commits_before_audio + 1);

    // Single undo rolls back the entire track + clip + note cleanly.
    ui.invoke_undo_step();
    let after_undo = undo.try_project().expect("工程存在");
    assert_eq!(after_undo.tracks.len(), tracks_before);
    assert_eq!(note_count(&after_undo), notes_before_audio);
}

#[test]
fn piano_roll_auditions_and_inserts_notes_in_empty_default_project() {
    // 3) Empty default_project() (only Master track, 0 non-master tracks):
    //    key audition auto-creates a MIDI track + synth and renders audio;
    //    piano-note-create inserts clip + note cleanly.
    let empty_proj = yeban_model::samples::default_project();
    let empty_view = ViewState::from_project(&empty_proj).expect("default_project 投影成功");
    let empty_scene = DemoScene::from_view(&empty_view);
    let empty_undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<aether-roll-default>",
            "yeban-app",
            empty_proj.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let empty_prod = ProductionLoop::start(&empty_proj, 0).expect("空工程引擎可启动");
    let empty_engine = empty_prod.engine_handle();
    let empty_view_cell = Rc::new(RefCell::new(empty_view.clone()));
    let empty_live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&empty_view, &empty_scene)
    })
    .expect("Tier-1 活窗口");
    let empty_ui = empty_live.ui();
    host::wire_roll_edit(empty_ui, &empty_undo);
    host::wire_track_select(empty_ui, &empty_undo);
    host::wire_clip_scope(
        empty_ui,
        Rc::clone(&empty_view_cell),
        Some(Rc::clone(&empty_undo)),
    );
    host::wire_aether_actions(
        empty_ui,
        Rc::clone(&empty_undo),
        Rc::clone(&empty_engine),
        empty_view_cell,
    );

    empty_ui.invoke_piano_key_pressed(15);
    assert!(empty_ui.get_module_status_succeeded());
    let mut default_buf = vec![0.0_f32; 512 * 2];
    empty_engine
        .borrow_mut()
        .render_into(&mut default_buf, 2, 4);
    let default_peak = default_buf
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        default_peak > 1e-4,
        "0 音轨工程按钢琴卷帘琴键必须自动建 MIDI 轨并立即发布发声，实际={default_peak}"
    );
    empty_ui.invoke_piano_key_released(15);

    empty_ui.invoke_piano_note_create(3, 4);
    let default_after_note = empty_undo.try_project().expect("工程存在");
    assert_eq!(note_count(&default_after_note), 1);
    assert!(
        empty_ui.get_note_ulids().row_count() >= 1,
        "0 音轨工程新建音符后必须立即投影到卷帘"
    );
}

#[test]
fn piano_roll_preserves_pitch_bounds_and_octave_direction_across_horizontal_scroll_and_edits() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-roll-pitch-bounds>", "yeban-app", project, NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let mut live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("主窗口装配成功");
    let prod = ProductionLoop::without_engine();
    let engine = prod.engine_handle();
    {
        let ui = live.ui();
        ui.set_show_lower_dock(true);
        ui.invoke_apply_dock_tab("pianoroll".into());
        host::wire_roll_edit(ui, &undo);
        host::wire_aether_actions(
            ui,
            Rc::clone(&undo),
            Rc::clone(&engine),
            Rc::new(RefCell::new(view.clone())),
        );
        host::apply_view(ui, &view, 1440.0, 0.0);
    }

    live.refresh_tree(None).expect("刷新控件树");
    let roll_bounds = live
        .tree()
        .find_by_id("piano-roll-scroll-area")
        .expect("piano-roll-scroll-area 必须在树里")
        .bounds
        .expect("piano-roll-scroll-area 必须有几何");
    let grid_bounds = live
        .tree()
        .find_by_id("piano-roll-grid")
        .expect("piano-roll-grid 必须在树里")
        .bounds
        .expect("piano-roll-grid 必须有几何");
    let grid_pt = LogicalPosition::new(
        roll_bounds.x as f32 + (roll_bounds.width as f32 * 0.5),
        roll_bounds.y as f32 + (roll_bounds.height as f32 * 0.5),
    );

    // 1. Scroll down 12 semitones to base_pitch = 48 (C3..D#4)
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: 0.0,
        delta_y: -168.0,
    });
    assert_eq!(live.ui().get_roll_base_pitch(), 48);
    assert_eq!(live.ui().get_roll_min_pitch(), 48);
    assert_eq!(live.ui().get_roll_max_pitch(), 63);
    let frame_before_hscroll = live.window().capture().expect("横向滚动前抓帧");

    // 2. Subsequent horizontal wheel/trackpad scroll and note creation must keep roll_min_pitch / roll_max_pitch intact
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: -60.0,
        delta_y: 0.0,
    });
    assert!((live.ui().get_roll_scroll_x() - 60.0).abs() < 0.5);
    assert_eq!(live.ui().get_roll_min_pitch(), 48);
    assert_eq!(live.ui().get_roll_max_pitch(), 63);

    // Verify rendered grid lines and notes in piano-roll-grid actually shift horizontally
    let frame_after_hscroll = live.window().capture().expect("卷帘横向滚动后抓帧");
    let mut grid_diffs = 0usize;
    let gx0 = (grid_bounds.x + 4).max(0) as u32;
    let gx1 = (grid_bounds.x + i32::try_from(grid_bounds.width).unwrap() - 4).max(0) as u32;
    let gy0 = (grid_bounds.y + 4).max(0) as u32;
    let gy1 = (grid_bounds.y + i32::try_from(grid_bounds.height).unwrap() - 4).max(0) as u32;
    for py in gy0..gy1 {
        for px in gx0..gx1 {
            if frame_before_hscroll.pixel(px, py) != frame_after_hscroll.pixel(px, py) {
                grid_diffs += 1;
            }
        }
    }
    assert!(
        grid_diffs > 100,
        "卷帘横向滚动后网格线与音符绘制必须发生真实位移 (grid_diffs={grid_diffs})"
    );

    live.ui().invoke_piano_note_create(20, 4);
    assert_eq!(live.ui().get_roll_base_pitch(), 48);
    assert_eq!(live.ui().get_roll_min_pitch(), 48);
    assert_eq!(live.ui().get_roll_max_pitch(), 63);

    // 3. Scroll down below 24 (base_pitch = 10), then press 八- (must NOT jump up to 24) and 八+ (clean +12 -> 22)
    live.window().dispatch(WindowEvent::PointerScrolled {
        position: grid_pt,
        delta_x: 0.0,
        delta_y: -(38.0 * 14.0),
    });
    assert_eq!(live.ui().get_roll_base_pitch(), 10);
    live.ui().invoke_piano_roll_octave_shift(-12);
    assert_eq!(
        live.ui().get_roll_base_pitch(),
        10,
        "base_pitch < 24 时点 八- 不得反方向跳回 24"
    );
    live.ui().invoke_piano_roll_octave_shift(12);
    assert_eq!(live.ui().get_roll_base_pitch(), 22);
    assert_eq!(live.ui().get_roll_min_pitch(), 22);
    assert_eq!(live.ui().get_roll_max_pitch(), 37);
}

fn assert_no_overlapping_clips(track: &yeban_model::project::TrackV3) {
    let mut ranges: Vec<(u64, u64)> = track
        .clips
        .values()
        .map(|p| (p.start_tick, p.start_tick.saturating_add(p.duration_ticks)))
        .collect();
    ranges.sort_unstable();
    assert!(!ranges.is_empty(), "断言轨道必须至少有一个片段摆放");
    for pair in ranges.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "自动创建/顺延片段不得与同轨已有片段重叠: {ranges:?}"
        );
    }
}

fn add_empty_midi_placement(
    port: &UndoPort,
    track_id: EntityId,
    name: &str,
    start_tick: u64,
    duration_ticks: u64,
) {
    let clip_id = EntityId::new();
    let placement_id = EntityId::new();
    let clip = yeban_model::project::ClipPoolEntry {
        id: clip_id,
        name: name.to_owned(),
        content: ClipContent::Midi {
            notes: std::collections::BTreeMap::new(),
        },
    };
    let placement = yeban_model::project::ClipPlacement {
        id: placement_id,
        clip_id,
        start_tick,
        duration_ticks,
        loop_config: yeban_model::project::LoopConfig::default(),
        muted: false,
        offset_samples: None,
    };
    port.commit_ops(
        NOW_MS,
        "test: seed midi clip",
        vec![
            yeban_model::ops::Op::AddClip { clip },
            yeban_model::ops::Op::AddClipPlacement {
                track_id,
                placement,
            },
        ],
    )
    .expect("测试预置 MIDI 片段必须提交成功");
}

#[test]
fn piano_roll_auto_clip_insert_never_overlaps_existing_placements() {
    let base = yeban_model::samples::default_project();
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<aether-roll-no-overlap>",
            "yeban-app",
            base.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let create_a =
        yeban_app::bridge::track_create_ops(&base, yeban_app::bridge::NewTrackKind::Midi)
            .expect("可建 MIDI 轨 A");
    let track_a_id = create_a
        .iter()
        .find_map(|op| match op {
            yeban_model::ops::Op::AddTrack { track } => Some(track.id),
            _ => None,
        })
        .expect("含 Track A");
    undo.commit_ops(NOW_MS, "test: add track A", create_a)
        .expect("Track A 提交成功");
    // Case (a): Track A has only [7680..11520)
    add_empty_midi_placement(&undo, track_a_id, "LateClip", 7680, 3840);
    // Case (b): Track B has [0..3840) and [7680..11520)
    let create_b = yeban_app::bridge::track_create_ops(
        &undo.try_project().unwrap(),
        yeban_app::bridge::NewTrackKind::Midi,
    )
    .expect("可建 MIDI 轨 B");
    let track_b_id = create_b
        .iter()
        .find_map(|op| match op {
            yeban_model::ops::Op::AddTrack { track } => Some(track.id),
            _ => None,
        })
        .expect("含 Track B");
    undo.commit_ops(NOW_MS, "test: add track B", create_b)
        .expect("Track B 提交成功");
    add_empty_midi_placement(&undo, track_b_id, "EarlyB", 0, 3840);
    add_empty_midi_placement(&undo, track_b_id, "LateB", 7680, 3840);

    let seeded = undo.try_project().expect("工程存在");
    let view = ViewState::from_project(&seeded).expect("投影成功");
    let scene = DemoScene::from_view(&view);
    let production = ProductionLoop::without_engine();
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_undo(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), Rc::clone(&engine), view_cell);

    // Subtest (a): Track A [7680..11520) + click at tick 240 -> creates [0..7680), no overlap.
    ui.set_selected_track_id(track_a_id.to_canonical_string().into());
    ui.set_selected_clip_ulid("".into());
    ui.set_roll_scroll_x(0.0);
    ui.set_active_tool(2);
    let click_x = ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX + 12.0;
    let click_y = ROLL_HEADER_HEIGHT_PX + NOTE_INSET_Y_PX + PITCH_LANE_HEIGHT_PX * 4.0 + 4.0;
    ui.invoke_clicked(click_x, click_y);
    let after_a = undo.try_project().expect("工程存在");
    let track_a = &after_a.tracks[&track_a_id];
    assert_eq!(track_a.clips.len(), 2, "Track A 应新建 1 条前置片段");
    assert_no_overlapping_clips(track_a);
    let ranges_a: Vec<(u64, u64)> = track_a
        .clips
        .values()
        .map(|p| (p.start_tick, p.start_tick + p.duration_ticks))
        .collect();
    assert!(
        ranges_a.contains(&(0, 7680)),
        "前置新建片段必须截断在 7680 不越界重叠: {ranges_a:?}"
    );

    // Subtest (b): Track B [0..3840) + [7680..11520) ->
    // 1) step 17 (tick 4080) extends [0..3840) to [0..7680) cleanly;
    // 2) step 49 (tick 11760) creates [11520..26880) cleanly without overlapping [7680..11520).
    ui.set_selected_track_id(track_b_id.to_canonical_string().into());
    ui.set_selected_clip_ulid("".into());
    ui.invoke_piano_note_create(24, 17);
    let after_b1 = undo.try_project().expect("工程存在");
    let track_b1 = &after_b1.tracks[&track_b_id];
    assert_eq!(track_b1.clips.len(), 2, "4080 应顺延既有 [0..3840) 片段");
    assert_no_overlapping_clips(track_b1);
    let mut ranges_b1: Vec<(u64, u64)> = track_b1
        .clips
        .values()
        .map(|p| (p.start_tick, p.start_tick + p.duration_ticks))
        .collect();
    ranges_b1.sort_unstable();
    assert_eq!(ranges_b1, vec![(0, 7680), (7680, 11520)]);

    ui.invoke_piano_note_create(24, 49);
    let after_b2 = undo.try_project().expect("工程存在");
    let track_b2 = &after_b2.tracks[&track_b_id];
    assert_eq!(
        track_b2.clips.len(),
        3,
        "11760 应在 [7680..11520) 之后新建独立片段"
    );
    assert_no_overlapping_clips(track_b2);
    let mut ranges_b2: Vec<(u64, u64)> = track_b2
        .clips
        .values()
        .map(|p| (p.start_tick, p.start_tick + p.duration_ticks))
        .collect();
    ranges_b2.sort_unstable();
    assert_eq!(ranges_b2, vec![(0, 7680), (7680, 11520), (11520, 26880)]);
}

#[test]
fn piano_roll_audio_track_auditions_with_drums_only_or_muted_midi_and_syncs_track_index() {
    let mut project = yeban_model::samples::demo_project();
    let audio_id = project
        .tracks
        .values()
        .find(|t| t.kind == yeban_model::project::TrackKind::Audio)
        .map(|t| t.id)
        .expect("demo_project 含音频轨");
    // Turn Track 0 ("鼓") into a real Drums kit (notes 36/38/39/42/46, not mapping C4=60)
    // and mute every other MIDI track so no existing MIDI track can play C4.
    let mut configured_drums = false;
    let mut muted_non_drum_midi = 0usize;
    for track in project.tracks.values_mut() {
        track.solo = false;
        if track.id != project.master_bus_track_id
            && track.kind == yeban_model::project::TrackKind::Midi
        {
            if !configured_drums {
                track.devices = vec![yeban_model::project::DeviceDefinition {
                    id: EntityId::new(),
                    name: "Yeban Drums".to_owned(),
                    kind: yeban_model::project::DeviceKind::InternalInstrument,
                    bypassed: false,
                    params: vec![
                        ("kick_note", 36.0),
                        ("snare_note", 38.0),
                        ("closed_hat_note", 42.0),
                        ("open_hat_note", 46.0),
                        ("clap_note", 39.0),
                    ]
                    .into_iter()
                    .map(|(name, value)| yeban_model::project::ParameterValue {
                        name: name.to_owned(),
                        value,
                        unit: None,
                    })
                    .collect(),
                    latency_samples: 0,
                    plugin: None,
                }];
                assert!(
                    yeban_engine::drums::DrumsParams::from_devices(&track.devices).is_some(),
                    "鼓机设备必须被引擎识别"
                );
                configured_drums = true;
            } else {
                track.mute = true;
                muted_non_drum_midi += 1;
            }
        }
    }
    assert!(
        configured_drums && muted_non_drum_midi >= 1,
        "demo_project 必须配置一条鼓机轨并静音其余 MIDI 轨"
    );

    let view = ViewState::from_project(&project).expect("投影成功");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<aether-roll-audio-drums-only>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("启动实时引擎");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_undo(ui, &undo);
    host::wire_mixer_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), Rc::clone(&engine), view_cell);

    let tracks_before = undo.try_project().unwrap().tracks.len();
    ui.set_selected_track_id(audio_id.to_canonical_string().into());
    ui.set_selected_clip_ulid("".into());

    // Pitch C4 (lane 15): Drums track doesn't map C4, Track 3 is muted -> must auto-create synth MIDI track and sound.
    ui.invoke_piano_key_pressed(15);
    assert!(ui.get_module_status_succeeded());
    let mut buf = vec![0.0_f32; 512 * 2];
    engine.borrow_mut().render_into(&mut buf, 2, 4);
    let peak = buf.iter().copied().fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        peak > 1e-4,
        "仅有未映射鼓轨与静音 MIDI 轨时，选中音频轨试听琴键必须自动新建合成器轨并发声 (peak={peak})"
    );
    ui.invoke_piano_key_released(15);
    let tail_quanta =
        ((48_000 * 2) / yeban_app::engine_host::EngineHost::quantum_frames().max(1)) as u64;
    engine.borrow_mut().pump(tail_quanta);
    assert_eq!(undo.try_project().unwrap().tracks.len(), tracks_before + 1);

    // Switch back to Audio track and create a note -> auto-created track must sync selected_track_index cleanly.
    ui.set_selected_track_id(audio_id.to_canonical_string().into());
    ui.set_selected_clip_ulid("".into());
    ui.invoke_piano_note_create(24, 1);

    let after_note = undo.try_project().unwrap();
    assert_eq!(after_note.tracks.len(), tracks_before + 2);
    let selected_id = ui.get_selected_track_id().to_string();
    let expected_view = ViewState::from_project(&after_note).expect("重投影成功");
    let expected_idx = expected_view
        .tracks
        .iter()
        .position(|t| t.id == selected_id)
        .expect("新建轨道必须出现在视图轨道列表中")
        + 1;
    assert_eq!(
        ui.get_selected_track_index() as usize,
        expected_idx,
        "自动建轨后 selected_track_index 必须同步为新轨 1-based 下标"
    );
}

#[test]
fn piano_roll_post_gap_auto_clip_aligns_ui_projection_split_and_audio() {
    use std::collections::BTreeMap;
    use yeban_app::bridge::{DEFAULT_TICKS_PER_PIXEL, host_to_painted, pitch_lane_y, tick_to_px};
    use yeban_app::export_wav::{WavExportOptions, export_wav};
    use yeban_render::wav::read_plain_wav_mono_f32;

    let mut base = yeban_model::samples::demo_project();
    base.bpm = 120.0;
    let master = base.master_bus_track_id;
    for track in base.tracks.values_mut() {
        if track.id != master {
            track.mute = true;
            track.solo = false;
        }
    }
    let seed_ops =
        yeban_app::bridge::track_create_ops(&base, yeban_app::bridge::NewTrackKind::Midi)
            .expect("创建测试 MIDI 轨");
    let target_track_id = seed_ops
        .iter()
        .find_map(|op| match op {
            yeban_model::ops::Op::AddTrack { track } => Some(track.id),
            _ => None,
        })
        .expect("必须拿到新建 MIDI 轨 ID");

    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-roll-gap-sync>", "yeban-app", base, NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    undo.commit_ops(NOW_MS, "test: add post-gap midi track", seed_ops)
        .expect("提交新建 MIDI 轨");
    // 在 0..1920 与 1920..3840 放两段空 MIDI 片段，制造 [3840..9600) 间隔后的跨隙新建片段场景。
    add_empty_midi_placement(&undo, target_track_id, "Bar1Empty", 0, 1920);
    add_empty_midi_placement(&undo, target_track_id, "Bar2Empty", 1920, 1920);

    let project = undo.try_project().expect("权威工程");
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let production = ProductionLoop::without_engine();
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_zoom_controls(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), engine, view_cell);

    let track_ulid = target_track_id.to_canonical_string();
    ui.set_selected_track_id(track_ulid.into());
    ui.set_selected_clip_ulid("".into());
    ui.invoke_apply_dock_tab("pianoroll".into());

    let scroll_tick: u64 = 7_680;
    let target_tick: u64 = 9_600;
    let scroll_px = tick_to_px(scroll_tick, DEFAULT_TICKS_PER_PIXEL).expect("scroll px") as f32;
    let target_px = tick_to_px(target_tick, DEFAULT_TICKS_PER_PIXEL).expect("target px") as f32;
    let host_x = target_px - scroll_px + 1.0;
    let host_y = pitch_lane_y(64) + 6.0;
    let (painted_x, painted_y) = host_to_painted(host_x, host_y);

    ui.set_roll_scroll_x(scroll_px);
    ui.set_active_tool(2); // Pencil
    ui.invoke_clicked(painted_x, painted_y);

    let after_insert = undo.try_project().expect("落笔后工程");
    let track = after_insert
        .tracks
        .get(&target_track_id)
        .expect("目标轨存在");
    let post_gap_placement = track
        .clips
        .values()
        .find(|p| target_tick >= p.start_tick && target_tick < p.start_tick + p.duration_ticks)
        .expect("必须在 9600 位置自动建出覆盖它的非零起点片段");
    assert_eq!(
        post_gap_placement.start_tick, 3_840,
        "空隙自动新建片段必须紧接前一片段末端 3840 开始"
    );

    let clip_id_str = post_gap_placement.clip_id.to_canonical_string();
    let clip_notes = after_insert
        .clip_pool
        .get(&post_gap_placement.clip_id)
        .and_then(|e| e.content.notes())
        .expect("新建片段必须是 MIDI 片段");
    assert_eq!(clip_notes.len(), 1, "跨隙落笔应精确插入 1 颗音符");
    let inserted_note = clip_notes.values().next().unwrap();
    assert_eq!(
        inserted_note.start_tick,
        target_tick - post_gap_placement.start_tick,
        "模型里的音符起点必须是剪辑局部 tick (9600 - 3840 = 5760)"
    );
    assert_eq!(inserted_note.pitch, 64);
    assert_eq!(inserted_note.duration_ticks, 960);

    // UI 投影侧：NoteView 与 note-positions 必须在时间线 9600 处精确可见且可命中。
    let proj_view = ViewState::from_project(&after_insert).expect("重投影");
    let note_view = proj_view
        .notes
        .iter()
        .find(|n| n.clip_id == clip_id_str)
        .expect("ViewState 必须投影出新建片段里的音符");
    assert_eq!(note_view.start_tick, target_tick);
    assert_eq!(note_view.end_tick, target_tick + 960);
    assert_eq!(note_view.x, target_px);

    host::apply_view(ui, &proj_view, 1440.0, scroll_px);
    let ui_positions: Vec<f32> = ui.get_note_positions().iter().collect();
    assert!(
        ui_positions
            .iter()
            .any(|&p| (p - (target_px - scroll_px)).abs() < 0.01),
        "视口滚动到 {scroll_px} 后，UI note-positions 必须包含相对坐标 {}，实际={ui_positions:?}",
        target_px - scroll_px
    );
    let hit_idx = proj_view.hit_test_visible_scoped_with_base(
        scroll_px,
        1440.0,
        host_x + 1.0,
        host_y,
        400.0,
        Some(&clip_id_str),
        60,
    );
    assert_eq!(
        hit_idx.map(|i| proj_view.notes[i].id.as_str()),
        Some(inserted_note.id.to_canonical_string().as_str()),
        "作用域命中测试必须命中该跨隙音符"
    );

    // 音频导出侧：120 BPM 下 tick 9600 = 5.000s = 240_000 帧。
    // 5760 局部 tick (= 3.000s = 144_000 帧) 处及之前必须严格静音，5.00s..5.50s 必须有真实音能。
    let wav_path = std::env::temp_dir().join(format!(
        "yeban_post_gap_roll_sync_{}.wav",
        std::process::id()
    ));
    let wav_opts = WavExportOptions {
        sample_rate: 48_000,
        frames: 48_000 * 6,
        ..Default::default()
    };
    export_wav(&after_insert, &wav_path, wav_opts).expect("WAV 导出必须成功");
    let (mono, rate, _) = read_plain_wav_mono_f32(&wav_path).expect("读取导出 WAV");
    let _ = std::fs::remove_file(&wav_path);
    assert_eq!(rate, 48_000);
    let pre_note_peak = mono[..235_000].iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        pre_note_peak < 1e-5,
        "9600 tick (240000 帧) 之前（含局部 tick 5760 对应的 144000 帧处）必须静音（仅允许 24-bit 量化底噪 <= 1e-5），实测={pre_note_peak}"
    );
    let note_peak = mono[240_000..264_000]
        .iter()
        .fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        note_peak > 0.005,
        "9600..10560 tick (240000..264000 帧) 区间必须发声，实测峰值={note_peak}"
    );

    // 剪刀切分：在时间线 10080 tick 处落刀，应把 [5760..6720) 切成 [5760..6240) 与 [6240..6720)。
    let split_px = tick_to_px(10_080, DEFAULT_TICKS_PER_PIXEL).expect("split px") as f32;
    let (split_painted_x, split_painted_y) = host_to_painted(split_px - scroll_px + 1.0, host_y);
    ui.set_selected_clip_ulid(post_gap_placement.id.to_canonical_string().into());
    ui.set_active_tool(3); // Knife
    ui.invoke_clicked(split_painted_x, split_painted_y);

    let after_split = undo.try_project().expect("切分后工程");
    let split_notes = after_split
        .clip_pool
        .get(&post_gap_placement.clip_id)
        .and_then(|e| e.content.notes())
        .expect("MIDI 片段");
    assert_eq!(
        split_notes.len(),
        2,
        "剪刀在 clip_start>0 音符中点切分后必须变成 2 颗音符"
    );
    let mut pairs: Vec<(u64, u64)> = split_notes
        .values()
        .map(|n| (n.start_tick, n.duration_ticks))
        .collect();
    pairs.sort_unstable();
    assert_eq!(pairs, vec![(5_760, 480), (6_240, 480)]);

    // 撤销切分回到 1 颗 [5760..6720) 音符后，step-toggle(84-64=20, step=40 -> 9600) 必须识别并删除它而非重复插入。
    let undo_outcome = undo.perform(yeban_app::undo::UiAction::Undo);
    assert!(
        matches!(undo_outcome, yeban_app::undo::ActionOutcome::Changed { .. }),
        "撤销切分必须改变工程回到 1 颗音符，实际={undo_outcome:?}"
    );
    ui.invoke_piano_step_toggle(84 - 64, 40);
    let after_toggle = undo.try_project().expect("步进切换后工程");
    let toggled_notes: &BTreeMap<_, _> = after_toggle
        .clip_pool
        .get(&post_gap_placement.clip_id)
        .and_then(|e| e.content.notes())
        .expect("MIDI 片段");
    assert!(
        toggled_notes.is_empty(),
        "piano-step-toggle 落在 clip_start>0 已有音符的时间线 step=40 (9600) 上时必须删除而非叠加重复音符"
    );
}
