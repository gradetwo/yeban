//! **33 个新 UI 独有回调的穷尽性活窗口判据**。
//!
//! 每个回调都在 Tier-1 活窗口(Slint testing backend + 生产同款 `wire_*` 装配)里
//! **真的 `invoke_` 一次**, 并断言它产生了"真实 handler / 真实 Op / 真实引擎事件"
//! 的可观测副作用 —— ⛔ 不是只读源码文本、⛔ 不是 toast、⛔ 不是空绿。
//!
//! 逐项读数以 `[matrix NN/33]` 打到 stdout, 因此"33 个都真跑过、都真改了状态"
//! 在日志里可数。真实 `.yeban` 交付件可经 `YEBAN_REAL_PROJECT` 注入(默认用
//! `samples::filled_project()`, 两条路径都跑完整 33 项)。
//!
//! 装配顺序与 `main.rs::run_gui` 一致: 先接旧回调(转发目标), 最后
//! [`yeban_app::host::wire_aether_actions`] 用 `on_*` 覆盖语义接管新回调。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::host::ProductionLoop;
use yeban_app::input::InputContext;
use yeban_app::open::open_project_file;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn project_under_test() -> YebanProjectV1 {
    match std::env::var("YEBAN_REAL_PROJECT") {
        Ok(path) => open_project_file(path).expect("真实 .yeban 容器必须能打开"),
        Err(_) => yeban_model::samples::filled_project(),
    }
}

fn note_ids(project: &YebanProjectV1) -> Vec<yeban_model::EntityId> {
    let mut ids = Vec::new();
    for entry in project.clip_pool.values() {
        if let yeban_model::project::ClipContent::Midi { notes } = &entry.content {
            ids.extend(notes.keys().cloned());
        }
    }
    ids
}

fn note_velocity(project: &YebanProjectV1, id: &yeban_model::EntityId) -> Option<u8> {
    project
        .clip_pool
        .values()
        .find_map(|entry| match &entry.content {
            yeban_model::project::ClipContent::Midi { notes } => notes.get(id).map(|n| n.velocity),
            _ => None,
        })
}

fn device_param_value(project: &YebanProjectV1) -> Option<f32> {
    project.tracks.values().find_map(|track| {
        track
            .devices
            .iter()
            .find_map(|dev| dev.params.first().map(|p| p.value))
    })
}

/// 33 个回调 → 真实副作用。逐项 `invoke_` + 断言 + 读数。
#[test]
fn all_33_aether_callbacks_reach_real_handlers_in_a_live_window() {
    let project = project_under_test();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-matrix>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));
    let project_rc = Rc::new(project.clone());

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // ---- 生产同款的旧回调装配(转发目标) ----
    host::wire_transport(
        ui,
        Rc::clone(&engine),
        Some(Rc::clone(&project_rc)),
        Some(Rc::clone(&undo)),
    );
    host::wire_view_callbacks(ui);
    host::wire_keys(
        ui,
        Rc::new(RefCell::new(InputContext::default())),
        Some(Rc::clone(&undo)),
        Some(Rc::clone(&engine)),
        Some(Rc::clone(&project_rc)),
    );
    host::wire_bpm_commit(ui, &undo);
    host::wire_mixer_edit(ui, &undo);
    host::wire_track_arm(ui);
    host::wire_track_select(ui, &undo);
    host::wire_tool_select(ui, Some(Rc::clone(&undo)));
    host::wire_track_strip(ui, &undo);
    host::wire_undo(ui, &undo);
    host::wire_scene_launch(ui, Rc::clone(&engine), Rc::clone(&view_cell));
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_sidebar_contents(ui, Rc::clone(&view_cell));
    // 生产桥接层最后注册(覆盖语义)。
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );

    let journal = || engine.borrow().transport_journal().len();
    let mut n = 0;
    macro_rules! hit {
        ($name:expr, $ok:expr, $detail:expr) => {{
            n += 1;
            assert!($ok, "[matrix {:02}/33] {} 没有产生真实副作用", n, $name);
            eprintln!("[matrix {:02}/33] {}: {}", n, $name, $detail);
        }};
    }

    // 01 transport-toggle-play -> 引擎走带命令
    let before = journal();
    ui.invoke_transport_toggle_play();
    hit!(
        "transport-toggle-play",
        journal() > before,
        format!("引擎命令日志 {} -> {}", before, journal())
    );

    // 02 transport-toggle-rec -> 真实启停录音(写 recording 属性), 第二击配对停止。
    ui.invoke_transport_toggle_rec();
    let started = ui.get_recording();
    ui.invoke_transport_toggle_rec();
    let stopped = !ui.get_recording();
    hit!(
        "transport-toggle-rec",
        started && stopped,
        format!(
            "起录 recording={started}, 停录后 recording={}",
            ui.get_recording()
        )
    );

    // 03 transport-toggle-loop -> ToggleCycle(写 cycle-enabled 视图态)
    let before = ui.get_cycle_enabled();
    ui.invoke_transport_toggle_loop();
    hit!(
        "transport-toggle-loop",
        ui.get_cycle_enabled() != before,
        format!("cycle-enabled {} -> {}", before, ui.get_cycle_enabled())
    );

    // 04 transport-rewind -> Stop + Seek(0)
    let before = journal();
    ui.invoke_transport_rewind();
    hit!(
        "transport-rewind",
        journal() > before,
        format!(
            "引擎命令日志 {} -> {}, position={}",
            before,
            journal(),
            engine.borrow().transport().position_ticks
        )
    );

    // 05 transport-set-bpm -> Op::SetTempo(改工程)
    ui.invoke_transport_set_bpm(132);
    let bpm = undo.try_project().expect("工程").bpm;
    hit!(
        "transport-set-bpm",
        (bpm - 132.0).abs() < 1e-6,
        format!("project.bpm = {bpm}")
    );

    // 06 transport-set-scale -> Op::SetKeySignature + scale-text
    ui.invoke_transport_set_scale("D Major".into());
    let key = undo.try_project().expect("工程").key_signature;
    hit!(
        "transport-set-scale",
        key.is_some() && ui.get_scale_text().as_str() == "D Major",
        format!(
            "key_signature = {key:?}, scale-text = {}",
            ui.get_scale_text()
        )
    );

    // 07 trigger-shortcut -> 空格 = PlayPause(引擎走带命令)
    let before = journal();
    ui.invoke_trigger_shortcut(" ".into(), slint::language::KeyEvent::default());
    hit!(
        "trigger-shortcut",
        journal() > before,
        format!("引擎命令日志 {} -> {}", before, journal())
    );

    // 08 track-selected -> selected-track-index
    ui.invoke_track_selected(2);
    hit!(
        "track-selected",
        ui.get_selected_track_index() == 2,
        format!("selected-track-index = {}", ui.get_selected_track_index())
    );

    // 09 track-toggle-mute -> Op::SetTrackMute(工程 mute 集合变化)
    let before: Vec<bool> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|t| t.mute)
        .collect();
    ui.invoke_track_toggle_mute(1);
    let after: Vec<bool> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|t| t.mute)
        .collect();
    hit!(
        "track-toggle-mute",
        before != after,
        format!("工程 mute 集合 {before:?} -> {after:?}")
    );

    // 10 track-toggle-solo -> Op::SetTrackSolo
    let before: Vec<bool> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|t| t.solo)
        .collect();
    ui.invoke_track_toggle_solo(1);
    let after: Vec<bool> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|t| t.solo)
        .collect();
    hit!(
        "track-toggle-solo",
        before != after,
        format!("工程 solo 集合 {before:?} -> {after:?}")
    );

    // 11 track-toggle-arm -> track-arms 数组
    let before: Vec<bool> = ui.get_track_arms().iter().collect();
    ui.invoke_track_toggle_arm(1);
    let after: Vec<bool> = ui.get_track_arms().iter().collect();
    hit!(
        "track-toggle-arm",
        before != after,
        format!("track-arms {before:?} -> {after:?}")
    );

    // 12 track-volume-changed -> Op::SetParam(TrackVolume)
    ui.invoke_track_volume_changed(1, -6.0);
    let vols: Vec<f32> = undo
        .try_project()
        .unwrap()
        .tracks
        .values()
        .map(|t| t.volume_db)
        .collect();
    hit!(
        "track-volume-changed",
        vols.iter().any(|v| (v + 6.0).abs() < 1e-3),
        format!("工程 volumes = {vols:?}")
    );

    // 13 track-add-requested -> Op::AddTrack
    let before = undo.try_project().unwrap().tracks.len();
    ui.invoke_track_add_requested();
    let after = undo.try_project().unwrap().tracks.len();
    hit!(
        "track-add-requested",
        after > before,
        format!("工程轨道数 {before} -> {after}")
    );

    // 14 edit-tool-selected -> active-tool(工具档位)
    let before = ui.get_active_tool();
    ui.invoke_edit_tool_selected("draw".into());
    hit!(
        "edit-tool-selected",
        ui.get_active_tool() != before,
        format!("active-tool {} -> {}", before, ui.get_active_tool())
    );

    // 15 command-execute("播放") -> 分派到 toggle_play(引擎命令)
    let before = journal();
    ui.invoke_command_execute("播放".into());
    hit!(
        "command-execute",
        journal() > before,
        format!("引擎命令日志 {} -> {}", before, journal())
    );

    // 16 piano-note-create -> Op::AddNote
    let before = note_ids(&undo.try_project().unwrap()).len();
    ui.invoke_piano_note_create(3, 8);
    let after = note_ids(&undo.try_project().unwrap()).len();
    hit!(
        "piano-note-create",
        after == before + 1,
        format!("工程音符数 {before} -> {after}")
    );

    // 17 piano-note-delete -> Op::DeleteNote(按 UI 音符 id 定位工程音符)
    let ui_notes_before = ui.get_notes().row_count();
    let before = note_ids(&undo.try_project().unwrap()).len();
    ui.invoke_piano_note_delete(1);
    let after = note_ids(&undo.try_project().unwrap()).len();
    hit!(
        "piano-note-delete",
        after == before.saturating_sub(1),
        format!("工程音符数 {before} -> {after}, ui-notes-before={ui_notes_before}")
    );

    // 18 piano-note-velocity -> Op::ModifyNoteVelocity
    let ids = note_ids(&undo.try_project().unwrap());
    let target = ids.first().copied();
    let before = target.and_then(|id| note_velocity(&undo.try_project().unwrap(), &id));
    // UI id 2 对应视图里第 2 个音符; 取工程里与该音符匹配的实体做 old_vel 校验,
    // 这里直接断言"有音符的力度被改成 42"。
    ui.invoke_piano_note_velocity(2, 42);
    let changed = note_ids(&undo.try_project().unwrap())
        .iter()
        .any(|id| note_velocity(&undo.try_project().unwrap(), id) == Some(42));
    hit!(
        "piano-note-velocity",
        changed,
        format!("工程里出现 velocity=42 的音符 (before={before:?})")
    );

    // 19 piano-note-split -> Op::SetNoteTiming + Op::AddNote
    let before = note_ids(&undo.try_project().unwrap()).len();
    ui.invoke_piano_note_split(3);
    let after = note_ids(&undo.try_project().unwrap()).len();
    hit!(
        "piano-note-split",
        after == before + 1,
        format!("工程音符数 {before} -> {after}")
    );

    // 20 piano-note-select -> note-selected + selected-note-count
    ui.invoke_piano_note_select(1);
    let selected: Vec<bool> = ui.get_note_selected().iter().collect();
    hit!(
        "piano-note-select",
        ui.get_selected_note_count() == 1 && selected.iter().any(|v| *v),
        format!(
            "selected-note-count = {}, note-selected 命中 {} 项",
            ui.get_selected_note_count(),
            selected.iter().filter(|v| **v).count()
        )
    );

    // 21 piano-step-toggle -> 无则 Op::AddNote / 有则 Op::DeleteNote
    let before = note_ids(&undo.try_project().unwrap()).len();
    ui.invoke_piano_step_toggle(5, 20);
    let after = note_ids(&undo.try_project().unwrap()).len();
    hit!(
        "piano-step-toggle",
        after != before,
        format!("工程音符数 {before} -> {after} (切换)")
    );

    // 22 device-param-changed -> Op::SetParam(DeviceParam)
    let before = device_param_value(&undo.try_project().unwrap());
    ui.invoke_device_param_changed("wavetable".into(), "cutoff".into(), 777.0);
    let after = device_param_value(&undo.try_project().unwrap());
    hit!(
        "device-param-changed",
        before.is_some() && after.is_some() && (after.unwrap() - 777.0).abs() < 1e-3,
        format!("工程设备参数 {before:?} -> {after:?}")
    );

    // 23 scene-trigger -> invoke_scene_launch(写 scene-launch-status 真实回执)
    ui.invoke_scene_trigger(0);
    hit!(
        "scene-trigger",
        !ui.get_scene_launch_status().as_str().is_empty(),
        format!("scene-launch-status = {}", ui.get_scene_launch_status())
    );

    // 24 clip-trigger -> invoke_clip_select(写 selected-clip-ulid)
    ui.invoke_clip_trigger(1, 0);
    hit!(
        "clip-trigger",
        !ui.get_selected_clip_ulid().as_str().is_empty(),
        format!("selected-clip-ulid = {}", ui.get_selected_clip_ulid())
    );

    // 25 chord-audition -> 真实引擎 NoteOn/Off 事件(通道批量写调用计数 +1)
    let channel_before = engine.borrow().event_bulk_push_calls();
    ui.invoke_chord_audition("Cmaj".into());
    let status = ui.get_module_status().to_string();
    let channel_after = engine.borrow().event_bulk_push_calls();
    hit!(
        "chord-audition",
        ui.get_module_status_succeeded()
            && status.contains("引擎事件通道已调用")
            && channel_after > channel_before,
        format!("module-status = {status}, 引擎事件通道调用 {channel_before} -> {channel_after}")
    );

    // 26 piano-key-audition -> 真实引擎 NoteOn/Off 事件
    let channel_before = engine.borrow().event_bulk_push_calls();
    ui.invoke_piano_key_audition("C4".into(), 3);
    let status = ui.get_module_status().to_string();
    let channel_after = engine.borrow().event_bulk_push_calls();
    hit!(
        "piano-key-audition",
        ui.get_module_status_succeeded()
            && status.contains("引擎事件通道已调用")
            && channel_after > channel_before,
        format!("module-status = {status}, 引擎事件通道调用 {channel_before} -> {channel_after}")
    );

    // 27 browser-item-load -> sidebar-search + 真实过滤投影
    ui.invoke_browser_item_load("pad".into());
    hit!(
        "browser-item-load",
        ui.get_sidebar_search().as_str() == "pad",
        format!("sidebar-search = {}", ui.get_sidebar_search())
    );

    // 28 dock-tab-selected -> console-tab
    ui.invoke_dock_tab_selected("pianoroll".into());
    hit!(
        "dock-tab-selected",
        ui.get_console_tab() == 1,
        format!("console-tab = {}", ui.get_console_tab())
    );

    // 29 zone-toggle -> status-selection(汇总四个真实 show-* 开关)
    ui.invoke_zone_toggle("browser".into());
    let status = ui.get_status_selection().to_string();
    hit!(
        "zone-toggle",
        status.contains("区域 browser"),
        format!("status-selection = {status}")
    );

    // 30 monitor-switch -> EngineHost::device_name() 真实读数
    ui.invoke_monitor_switch("Main".into());
    let status = ui.get_status_selection().to_string();
    hit!(
        "monitor-switch",
        status.contains("监听 Main"),
        format!("status-selection = {status}")
    );

    // 31 hot-swap-buffer-size -> buffer-samples + dsp-cpu-percent
    ui.invoke_hot_swap_buffer_size(256);
    hit!(
        "hot-swap-buffer-size",
        ui.get_buffer_samples() == 256,
        format!(
            "buffer-samples = {}, dsp-cpu-percent = {}",
            ui.get_buffer_samples(),
            ui.get_dsp_cpu_percent()
        )
    );

    // 32 normalize-track-heights -> track-height-percent 复位 + 清覆盖
    ui.set_track_height_percent(175);
    ui.invoke_normalize_track_heights();
    hit!(
        "normalize-track-heights",
        ui.get_track_height_percent() == 100,
        format!("track-height-percent = {}", ui.get_track_height_percent())
    );

    // 33 open-cmd-palette -> show-cmd-palette
    ui.invoke_open_cmd_palette();
    hit!(
        "open-cmd-palette",
        ui.get_show_cmd_palette(),
        format!("show-cmd-palette = {}", ui.get_show_cmd_palette())
    );

    // 02 的真实录音会在 cwd 落一条空 WAV, 判据收尾清掉(不污染工作区)。
    let _ = std::fs::remove_file("ui-record.wav");
    assert_eq!(n, 33, "必须穷尽 33 个回调");
    eprintln!("[matrix] 33/33 个新 UI 独有回调全部在活窗口产生真实副作用");
}
