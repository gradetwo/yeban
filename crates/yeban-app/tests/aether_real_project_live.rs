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

use slint::Model;
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

    // ---- 生产同款接线（逐条对齐 `src/main.rs` 的 wire_* 清单）----
    // 报告 Q2：本判据此前只接 `wire_mixer_edit` + `wire_aether_actions`，于是
    // 轨道重排 / 设备旁通在缩减装配下静默无效 —— 判据全绿也不能证明生产接线没断。
    // 这里把产品装配清单补齐，下面 ⑥/⑦ 直接断言这两条链路真的改工程。
    host::wire_transport(
        ui,
        Rc::clone(&engine),
        Some(Rc::clone(&project_rc)),
        Some(Rc::clone(&undo)),
    );
    host::wire_view_callbacks(ui);
    host::wire_keys(
        ui,
        Rc::new(RefCell::new(yeban_app::input::InputContext::default())),
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
    host::wire_track_reorder(ui, &undo);
    host::wire_track_height_drag(ui, &undo);
    host::wire_zoom_controls(ui, &undo);
    host::wire_arrangement_wheel(ui, &undo);
    host::wire_arrangement_hscroll(ui, &undo);
    host::wire_track_rename(ui, &undo);
    host::wire_automation_lane_click(ui, &undo);
    host::wire_clip_create(ui, &undo);
    host::wire_clip_drag(ui, &undo);
    host::wire_send_strip(ui, &undo);
    host::wire_macro_panel(ui, &undo);
    let _ = host::wire_device_rack(ui, &undo);
    host::wire_scene_launch(ui, Rc::clone(&engine), Rc::clone(&view_cell));
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_sidebar_contents(ui, Rc::clone(&view_cell));
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );

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
}
