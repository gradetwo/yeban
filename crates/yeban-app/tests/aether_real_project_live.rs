//! 真实 `.yeban` 工程在**活窗口**里的导入 / 编辑 / 混音端到端判据。
//!
//! 交付件 `/home/crow/work/qoder/夜半霓虹_v5_母带交付.zip` 内的 `.yeban`
//! （17 轨 / 2867 音符 / 10 段落 / 6336 语义元素）经环境变量 `YEBAN_REAL_PROJECT`
//! 注入：`YEBAN_REAL_PROJECT=<path> cargo test -p yeban-app --test aether_real_project_live`。
//!
//! CI 上没有这件 120MB 资产 ⇒ 退回 `samples::filled_project()`（必要时补一个 MIDI
//! 片段）走**完全相同的链路**。⛔ 这不是跳过：两种输入下每一条断言都真跑、真改工程、
//! 真读回 UI 属性；区别只在工程规模。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use slint::{ComponentHandle, Model};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::ProductionLoop;
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

/// 生产装配的同一条路径：`build_main_window` + `src/main.rs` 的完整 `wire_*` 清单
/// （走带 / 键位 / 卷帘 / 编排滚轮 / 移轨 / 设备架 / 侧栏 / 发送 / 宏 / 撤销）+
fn wire_full_production_surface(
    ui: &yeban_app::ui::MainWindow,
    undo: &Rc<UndoPort>,
    engine: &Rc<RefCell<yeban_app::engine_host::EngineHost>>,
    project_rc: &Rc<YebanProjectV1>,
    view_cell: &Rc<RefCell<ViewState>>,
) {
    let input = Rc::new(RefCell::new(yeban_app::input::InputContext::default()));
    host::wire_input(ui, Rc::clone(&input));
    host::wire_transport(
        ui,
        Rc::clone(engine),
        Some(Rc::clone(project_rc)),
        Some(Rc::clone(undo)),
    );
    host::wire_view_callbacks(ui);
    host::wire_keys(
        ui,
        input,
        Some(Rc::clone(undo)),
        Some(Rc::clone(engine)),
        Some(Rc::clone(project_rc)),
    );
    host::wire_bpm_commit(ui, undo);
    host::wire_mixer_edit(ui, undo);
    host::wire_track_arm(ui);
    host::wire_track_select(ui, undo);
    host::wire_tool_select(ui, Some(Rc::clone(undo)));
    host::wire_track_strip(ui, undo);
    host::wire_undo(ui, undo);
    host::wire_track_reorder(ui, undo);
    host::wire_track_height_drag(ui, undo);
    host::wire_zoom_controls(ui, undo);
    host::wire_arrangement_wheel(ui, undo);
    host::wire_arrangement_hscroll(ui, undo);
    host::wire_track_rename(ui, undo);
    host::wire_automation_lane_click(ui, undo);
    host::wire_clip_create(ui, undo);
    host::wire_clip_drag(ui, undo);
    host::wire_roll_edit(ui, undo);
    host::wire_humanize_dialog(ui, Some(Rc::clone(undo)));
    host::wire_send_strip(ui, undo);
    host::wire_macro_panel(ui, undo);
    let _ = host::wire_device_rack(ui, undo);
    host::wire_scene_launch(ui, Rc::clone(engine), Rc::clone(view_cell));
    host::wire_proposal_drawer(ui, yeban_app::proposals::ProposalSource::none());
    host::wire_clip_scope(ui, Rc::clone(view_cell), Some(Rc::clone(undo)));
    host::wire_ruler_seek(ui, Rc::clone(engine), Rc::clone(view_cell));
    let _overview_state = yeban_app::song_overview::wire_song_overview(
        ui,
        Rc::clone(engine),
        Rc::clone(view_cell),
        Some(Rc::clone(undo)),
    );
    host::wire_section_seek(ui, Rc::clone(engine), Rc::clone(view_cell));
    host::wire_sidebar_contents(ui, Rc::clone(view_cell));
    host::wire_aether_actions(ui, Rc::clone(undo), Rc::clone(engine), Rc::clone(view_cell));
}

/// 真实 `.yeban`（或等价 `filled_project()`）在活窗口里走通全链（含 Q2 回归）与
/// `wire_aether_actions`。报告 Q2 的教训：判据若只接一小撮接线，轨道重排与设备旁通
/// 会在“缩减装配”下静默无效，全绿也证明不了产品接线没断。
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
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));
    let project_rc = Rc::new(project.clone());

    wire_full_production_surface(ui, &undo, &engine, &project_rc, &view_cell);

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

    // ①a 片段块：身份 = 摆放身份，宽度 = 真实时值 / 当前缩放（报告反馈②）。
    let blocks = ui.get_timeline_blocks();
    let tpp = u32::try_from(ui.get_roll_ticks_per_pixel())
        .unwrap_or(1)
        .max(1);
    assert_eq!(blocks.row_count(), view.clips.len(), "片段块数量");
    for (index, clip) in view.clips.iter().enumerate() {
        let block = blocks.row_data(index).expect("块与投影同序等长");
        assert_eq!(
            block.clip_ulid.as_str(),
            clip.placement_id,
            "块身份必须是摆放身份（点击靠它选中片段）"
        );
        let expected = (clip.duration_ticks as f32 / tpp as f32).max(12.0);
        assert!(
            (block.width_px - expected).abs() < 1.5,
            "片段 {} 的块宽 {} 与真实时值换算的 {expected} 不一致",
            clip.clip_name,
            block.width_px
        );
    }
    eprintln!(
        "[clips] {} 个块的宽度/身份与投影一致 (ticks-per-pixel={tpp})",
        view.clips.len()
    );

    // ①b [Q3] 走带 TEMPO 读数 = 工程 BPM（`.slint` 默认 124 只能在没有工程时出现）。
    let expected_bpm = i32::try_from((view.bpm_millis + 500) / 1000).unwrap_or(0);
    assert_eq!(
        ui.get_bpm_val(),
        expected_bpm,
        "TEMPO 读数必须等于工程 BPM（投影 {:.2}）",
        view.bpm
    );
    eprintln!(
        "[transport] bpm-val={} project={:.2}",
        ui.get_bpm_val(),
        view.bpm
    );

    // ①c [Q4] 通道检查器的插入槽 = 该轨**真实设备链**（空链时为空槽 / SFZ 路径）。
    let ui_tracks = ui.get_tracks();
    for (index, track) in view.tracks.iter().enumerate() {
        let row = ui_tracks.row_data(index).expect("ui tracks 与投影同序等长");
        let expected = [
            track
                .devices
                .first()
                .map(String::as_str)
                .or(track.sampler_path.as_deref())
                .unwrap_or(""),
            track.devices.get(1).map_or("", String::as_str),
            track.devices.get(2).map_or("", String::as_str),
        ];
        let actual = [
            row.insert1.as_str(),
            row.insert2.as_str(),
            row.insert3.as_str(),
        ];
        assert_eq!(
            actual, expected,
            "轨道 {} 的插入槽必须是真实设备链 {:?}",
            track.name, expected
        );
        for fabricated in [
            "Pro-QLinear 8",
            "Glue Comp VCA",
            "Wavetable MPE",
            "Audio Clip",
        ] {
            assert!(
                !actual.contains(&fabricated) || expected.contains(&fabricated),
                "轨道 {} 出现了工程里不存在的插入设备 {fabricated}",
                track.name
            );
        }
    }
    eprintln!(
        "[inserts] {} 条轨的插入槽均来自真实设备链",
        view.tracks.len()
    );

    // ①d [Q6] 标尺 / 网格的刻度与片段块同源：小节宽 = 相邻小节线之差，
    // 第 N 小节的标尺 x = `bar_positions[N]`（旧版写死 1..36 / 64px）。
    assert!(view.bar_positions.len() >= 2, "工程至少要有两小节");
    let spacing = view.bar_positions[1] - view.bar_positions[0];
    assert!(
        (ui.get_bar_width_px() - spacing).abs() < 1e-3,
        "小节宽必须等于投影间距 {spacing}，实测 {}",
        ui.get_bar_width_px()
    );
    let bar_count = usize::try_from(ui.get_bar_count()).expect("正整数");
    assert!(
        bar_count >= 2 && bar_count <= view.bar_positions.len(),
        "可见小节数 {bar_count} 必须在 2..={} 内",
        view.bar_positions.len()
    );
    for index in 0..bar_count {
        let ruler_x = index as f32 * ui.get_bar_width_px();
        assert!(
            (ruler_x - view.bar_positions[index]).abs() < 1e-3,
            "第 {} 小节的标尺 x={ruler_x} 与投影 {} 不一致",
            index + 1,
            view.bar_positions[index]
        );
    }
    eprintln!(
        "[ruler] {bar_count}/{} 小节可见, 小节宽 {}px, 与 bar_positions 逐项一致",
        view.bar_positions.len(),
        ui.get_bar_width_px()
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

    // ⑥ [Q2] 轨道重排：`track-reorder-step` 必须提交 `Op::MoveTrack`。
    // 这条链路只有生产装配（`wire_track_reorder`）才存在 ⇒ 断言它真改工程顺序。
    let order_before = undo.try_project().unwrap().track_order.clone();
    ui.invoke_track_reorder_step(0, 1);
    let order_after = undo.try_project().unwrap().track_order.clone();
    assert_ne!(
        order_before, order_after,
        "轨道重排必须改工程的显示顺序，不是只改视图态"
    );
    assert_eq!(
        ui.get_track_ids().row_count(),
        view.tracks.len(),
        "重排后 UI 的 track-ids 必须重投影"
    );
    eprintln!(
        "[reorder] order {} -> {}",
        order_before.len(),
        order_after.len()
    );

    // ⑦ [Q2] 设备旁通：`device-bypass-toggle` 必须提交真实 Op（工程里的 bypassed 翻转）。
    // 只从**非主总线**轨里选目标（设备架的可编辑面就是展示轨）——主总线不在
    // UI 的 `track-ids` 里，选不到它。
    let current_project = undo.try_project().unwrap();
    let bypass_target = view
        .tracks
        .iter()
        .find_map(|track| {
            let track_id = track.id.parse::<EntityId>().ok()?;
            let device = current_project.tracks.get(&track_id)?.devices.first()?;
            Some((track_id, device.id, device.bypassed))
        })
        .expect("工程至少有一台展示轨设备，否则本项是空绿");
    let canonical = bypass_target.0.to_canonical_string();
    let track_ids = ui.get_track_ids();
    let ui_index = (0..track_ids.row_count())
        .find(|index| {
            track_ids
                .row_data(*index)
                .is_some_and(|id| id.as_str() == canonical)
        })
        .expect("设备所在轨必须在 UI 的 track-ids 里");
    ui.invoke_track_select(i32::try_from(ui_index).expect("轨道下标"));
    ui.invoke_device_bypass_toggle(bypass_target.1.to_string().into());
    let bypassed_after = undo
        .try_project()
        .unwrap()
        .tracks
        .get(&bypass_target.0)
        .and_then(|track| {
            track
                .devices
                .iter()
                .find(|device| device.id == bypass_target.1)
        })
        .map(|device| device.bypassed)
        .expect("设备必须在工程里");
    assert_ne!(
        bypass_target.2, bypassed_after,
        "设备旁通必须改工程里的 bypassed 位"
    );
    eprintln!("[device] bypass {} -> {bypassed_after}", bypass_target.2);

    // ⑧ [N1-N9 + Pan/Instrument 回归] 在同一个活窗口里逐项验证：
    // [N8] TimelineBlockData.note_count 反映真实音符密度
    let blocks = ui.get_timeline_blocks();
    assert!(blocks.row_count() >= 2);
    assert!(
        blocks.iter().any(|b| !b.is_audio && b.note_count > 0),
        "[N8] MIDI 时间块必须带出真实音符数 note_count > 0"
    );
    assert!(
        blocks
            .iter()
            .filter(|b| b.is_audio)
            .all(|b| b.note_count == 0),
        "[N8] 音频时间块的 note_count 必须为 0"
    );

    // [N7] 双击片段与 apply-dock-tab 必须双向同步而不回弹
    let first_clip_ulid = ui.get_clip_ulids().row_data(0).expect("至少一个片段");
    ui.invoke_clip_double_click(first_clip_ulid);
    assert!(ui.get_show_lower_dock(), "[N7] 双击片段必须展开底部面板");
    assert_eq!(ui.get_console_tab(), 0, "[N7] 双击片段应切至钢琴卷帘 tab=0");
    assert_eq!(
        ui.global::<yeban_app::ui::Theme<'_>>()
            .get_active_dock_tab()
            .as_str(),
        "pianoroll",
        "[N7] Theme.active-dock-tab 必须保持为 pianoroll"
    );
    ui.invoke_apply_dock_tab("mixer".into());
    assert_eq!(ui.get_console_tab(), 1);
    assert_eq!(
        ui.global::<yeban_app::ui::Theme<'_>>()
            .get_active_dock_tab()
            .as_str(),
        "mixer"
    );
    ui.invoke_apply_dock_tab("pianoroll".into());

    // [N6 + P1] 动态定位当前带设备的 MIDI 轨(第⑥步重排可能移动了它的下标)
    let (midi_ui_idx, track0_id) = {
        let proj = undo.try_project().unwrap();
        let ids = ui.get_track_ids();
        (0..ids.row_count())
            .find_map(|idx| {
                let s = ids.row_data(idx)?;
                let tid = s.as_str().parse::<EntityId>().ok()?;
                let track = proj.tracks.get(&tid)?;
                (track.kind == yeban_model::project::TrackKind::Midi && !track.devices.is_empty())
                    .then_some((i32::try_from(idx).unwrap(), tid))
            })
            .expect("工程至少一条带设备的 MIDI 轨")
    };
    ui.invoke_track_select(midi_ui_idx);
    assert_eq!(
        ui.get_selected_track_index(),
        midi_ui_idx + 1,
        "[N6] 选中 MIDI 轨同步"
    );
    ui.invoke_track_pan_changed(midi_ui_idx + 1, -0.5);
    assert!(
        (undo.try_project().unwrap().tracks[&track0_id].pan + 0.5).abs() < 1e-3,
        "track-pan-changed 必须提交 Op::SetParam(TrackPan)"
    );
    assert_eq!(
        ui.get_track_pans()
            .row_data(usize::try_from(midi_ui_idx).unwrap())
            .as_ref()
            .map(slint::SharedString::as_str),
        Some("L50")
    );
    ui.invoke_track_instrument_change(midi_ui_idx + 1, "synth:metallic".into());
    assert_eq!(
        undo.try_project().unwrap().tracks[&track0_id].devices[0].name,
        "FM Bell",
        "track-instrument-change 必须更新内置合成器音色预设"
    );

    // [N3] 新建一条无片段的 MIDI 轨并选中：写音符不得静默吞掉，必须置可见状态提示
    let track0_midi_clip_id = undo.try_project().unwrap().tracks[&track0_id]
        .clips
        .values()
        .find(|p| {
            undo.try_project()
                .unwrap()
                .clip_pool
                .get(&p.clip_id)
                .is_some_and(|e| e.content.notes().is_some())
        })
        .map(|p| p.clip_id);
    let track0_notes_before = track0_midi_clip_id
        .map(|cid| {
            undo.try_project().unwrap().clip_pool[&cid]
                .content
                .notes()
                .unwrap()
                .len()
        })
        .unwrap_or(0);

    ui.invoke_track_create_midi();
    let new_idx = i32::try_from(ui.get_track_ids().row_count() - 1).expect("新轨下标");
    let new_track_id = ui
        .get_track_ids()
        .row_data(usize::try_from(new_idx).unwrap())
        .unwrap()
        .as_str()
        .parse::<EntityId>()
        .unwrap();
    ui.invoke_track_select(new_idx);
    assert_eq!(
        ui.get_selected_track_index(),
        new_idx + 1,
        "[N6] 选中新轨同步"
    );
    ui.set_selected_clip_ulid("".into());
    ui.set_roll_edit_status("".into());
    let count_before_n3 = midi_note_count(&undo.try_project().unwrap());
    ui.invoke_piano_note_create(2, 0);
    assert_eq!(
        midi_note_count(&undo.try_project().unwrap()),
        count_before_n3,
        "[N3] 空 MIDI 轨未建片段前不应凭空写入音符"
    );
    assert!(
        !ui.get_roll_edit_status().is_empty() && !ui.get_toast_message().is_empty(),
        "[N3] 无片段写音符必须在 roll-edit-status 与 toast 明确反馈"
    );

    // [N2 & N4] 在新选中轨建片段后写越界低音 row=48 (pitch=36) / step=80 (start_tick=19200)
    ui.invoke_clip_create();
    let new_clip_id = undo.try_project().unwrap().tracks[&new_track_id]
        .clips
        .values()
        .next()
        .expect("新建轨应刚创建片段")
        .clip_id;
    ui.invoke_piano_note_create(48, 80);
    let proj_after_n2 = undo.try_project().unwrap();
    if let Some(cid0) = track0_midi_clip_id {
        assert_eq!(
            proj_after_n2.clip_pool[&cid0]
                .content
                .notes()
                .unwrap()
                .len(),
            track0_notes_before,
            "[N2] 不得误写入已有 MIDI 轨片段"
        );
    }
    let created_notes: Vec<_> = proj_after_n2.clip_pool[&new_clip_id]
        .content
        .notes()
        .unwrap()
        .values()
        .collect();
    assert_eq!(created_notes.len(), 1, "[N2] 必须写入当前选中轨新建片段");
    let created_note_id = created_notes[0].id;
    assert_eq!(
        created_notes[0].pitch, 36,
        "[N4] 应支持低于 69 的低音区 pitch=36"
    );
    assert_eq!(
        created_notes[0].start_tick, 19200,
        "[N4] 应支持超过 64 步的 step=80"
    );

    // [N1] 同会话内不重开直接改力度 / 删除刚刚新建的音符
    let live_after_create = host::project_with_view_state(&proj_after_n2, ui).expect("重投影失败");
    let created_note_str = created_note_id.to_string();
    let created_ui_id = live_after_create
        .notes
        .iter()
        .find(|n| n.id == created_note_str)
        .map(|n| i32::try_from(n.index + 1).unwrap())
        .expect("新建音符必须在实时投影中");
    ui.invoke_piano_note_velocity(created_ui_id, 55);
    assert_eq!(
        undo.try_project().unwrap().clip_pool[&new_clip_id]
            .content
            .notes()
            .unwrap()[&created_note_id]
            .velocity,
        55,
        "[N1] 同会话内新建音符必须能被力度调整命中"
    );
    let count_before_del = midi_note_count(&undo.try_project().unwrap());
    ui.invoke_piano_note_delete(created_ui_id);
    assert_eq!(
        midi_note_count(&undo.try_project().unwrap()),
        count_before_del - 1,
        "[N1] 同会话内新建音符必须能被删除"
    );
    assert!(
        !undo.try_project().unwrap().clip_pool[&new_clip_id]
            .content
            .notes()
            .unwrap()
            .contains_key(&created_note_id),
        "[N1] 删除命中的必须是刚才新建的那颗音符"
    );

    // [N9] 工具=铅笔(digit=2 -> ToolDrag::ResizeDuration) 拖拽改变音符时值
    ui.invoke_track_select(midi_ui_idx);
    ui.invoke_tool_selected(2);
    let live_view = host::project_with_view_state(&undo.try_project().unwrap(), ui).expect("投影");
    let scroll_x = ui.get_roll_scroll_x().max(0.0);
    let target_note = live_view
        .notes
        .iter()
        .find(|n| n.x >= scroll_x && n.x < scroll_x + 900.0)
        .or_else(|| live_view.notes.first())
        .expect("至少一个可见音符")
        .clone();
    let note_entity = target_note.id.parse::<EntityId>().unwrap();
    let dur_before = target_note.duration_ticks;
    let (px, py) =
        yeban_app::bridge::host_to_painted((target_note.x - scroll_x) + 2.0, target_note.y + 4.0);
    assert!(
        ui.invoke_note_drag_grab(px, py),
        "[N9] ToolDrag::ResizeDuration 必须能抓起音符"
    );
    ui.invoke_note_drag_move(px + 48.0, py);
    ui.invoke_note_drag_release(px + 48.0, py);
    let dur_after = undo
        .try_project()
        .unwrap()
        .clip_pool
        .values()
        .find_map(|entry| {
            entry
                .content
                .notes()?
                .get(&note_entity)
                .map(|n| n.duration_ticks)
        })
        .expect("音符必须在工程里");
    assert!(
        dur_after > dur_before,
        "[N9] ToolDrag::ResizeDuration 松手必须增加 duration_ticks ({dur_before} -> {dur_after})"
    );
}
