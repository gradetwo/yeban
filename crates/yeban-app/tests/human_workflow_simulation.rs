//! 真人创作流程无 MCP 端到端 UI 仿真测试（LivePort + 全量生产接线）。
//!
//! 严禁依赖 `yeban-mcp` / `yeban-ui-mcp`：仅使用 `yeban-app` 自身提供的
//! `LivePort`、`host::wire_*` 生产接线、钢琴卷帘/编排/混音/设备机架/发送路由
//! 与 `export_wav` 导出能力，完整模拟人类作曲与混音的真实操作链路：
//! 1. 从空白工程起步新建 MIDI / Audio / Aux 轨并重命名；
//! 2. 设定老上海 1930s 怀旧节奏（BPM 78）与 C 小调；
//! 3. 指派合成器音色、插入 EQ / Reverb / Compressor，并用多参数
//!    `device-param-changed` 精确调节目标设备参数（覆盖多设备不误伤 slot 0）；
//! 4. 建立到 Aux Return 的 Send 发送链路并拖动发送增益；
//! 5. 切换控制台页签、二维滚轮滚动编排窗与钢琴卷帘、缩放视口；
//! 6. 琴键试听、自建/跨隙新建片段并画音符、调力度、拆分、人性化、量化；
//! 7. 推子/声像/静音独奏混音、撤销/重做回放，最后导出 48kHz WAV 验收音能且不削波。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model;
use yeban_app::bridge::{
    DEFAULT_TICKS_PER_PIXEL, ViewState, host_to_painted, pitch_lane_y, tick_to_px,
};
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_model::project::{DeviceKind, RoutingKind, TrackKind};
use yeban_render::wav::read_plain_wav_mono_f32;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_760_100_000_000;

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

fn ui_track_pos(ui: &yeban_app::ui::MainWindow, track_id: yeban_model::EntityId) -> usize {
    let target = track_id.to_canonical_string();
    ui.get_track_ids()
        .iter()
        .position(|s| s.as_str() == target)
        .expect("目标轨道必须存在于 UI track-ids 数组中")
}

#[test]
fn simulated_human_ui_composition_mixing_and_wav_export_without_mcp() {
    let base = yeban_model::samples::default_project();
    let initial_view = ViewState::from_project(&base).expect("空白默认工程必须能投影");
    let scene = DemoScene::from_view(&initial_view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<human-sim>", "yeban-app", base.clone(), NOW_MS)
            .expect("打开权威撤销会话"),
    ));
    let production = ProductionLoop::start(&base, 0).expect("启动引擎");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(initial_view.clone()));
    let project_rc = Rc::new(base);

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&initial_view, &scene)
    })
    .expect("构建 LivePort 活窗口");
    let ui = live.ui();
    wire_full_production_surface(ui, &undo, &engine, &project_rc, &view_cell);

    // 1. 建轨：2 条 MIDI 轨 + 1 条 Audio 轨 + 1 条 Aux Return 混响总线。
    ui.invoke_track_add_requested();
    ui.invoke_track_create_midi();
    ui.invoke_track_create_audio();
    ui.invoke_track_create_aux();

    let after_tracks = undo.try_project().expect("建轨后工程存在");
    let midi_ids: Vec<_> = after_tracks
        .tracks
        .values()
        .filter(|t| t.kind == TrackKind::Midi)
        .map(|t| t.id)
        .collect();
    let audio_id = after_tracks
        .tracks
        .values()
        .find(|t| t.kind == TrackKind::Audio)
        .expect("新建音频轨存在")
        .id;
    let aux_id = after_tracks
        .tracks
        .values()
        .find(|t| t.kind == TrackKind::AuxReturn)
        .expect("新建 Aux Return 轨存在")
        .id;
    assert_eq!(midi_ids.len(), 2, "应成功建立 2 条 MIDI 轨");
    let lead_id = midi_ids[0];
    let piano_id = midi_ids[1];

    // 重命名各轨（UI track-rename 采用 0-based 下标）
    ui.invoke_track_rename(ui_track_pos(ui, lead_id) as i32, "导唱长笛".into());
    ui.invoke_track_rename(ui_track_pos(ui, piano_id) as i32, "立式钢琴".into());
    ui.invoke_track_rename(ui_track_pos(ui, audio_id) as i32, "霞飞路雨声".into());
    ui.invoke_track_rename(ui_track_pos(ui, aux_id) as i32, "老唱片大厅混响".into());

    let renamed = undo.try_project().expect("重命名后工程");
    assert_eq!(renamed.tracks[&lead_id].name, "导唱长笛");
    assert_eq!(renamed.tracks[&piano_id].name, "立式钢琴");
    assert_eq!(renamed.tracks[&audio_id].name, "霞飞路雨声");
    assert_eq!(renamed.tracks[&aux_id].name, "老唱片大厅混响");

    // 2. 速度与调号（1930s 老上海 78 BPM C 小调）
    ui.invoke_transport_set_bpm(78);
    ui.invoke_transport_set_scale("C minor".into());
    let timed = undo.try_project().expect("速度/调号提交");
    assert!((timed.bpm - 78.0).abs() < 0.01, "BPM 必须同步更新为 78.0");
    assert_eq!(
        timed.key_signature,
        Some(yeban_model::KeySignature {
            sharps_flats: -3,
            is_minor: true,
        }),
        "C minor 必须正确解析为 3 个降号的小调"
    );
    assert_eq!(ui.get_scale_text().as_str(), "C minor");
    let lead_track_row = ui
        .get_tracks()
        .row_data(ui_track_pos(ui, lead_id))
        .expect("导唱轨 UI 行存在");
    assert_eq!(
        lead_track_row.bus.as_str(),
        "Master",
        "直连主总线的轨道在 UI 中必须真实投影为 Master，而非硬编码 Bus 1 (Sum)"
    );

    // 3. 选中“导唱长笛”指派合成器，再插 EQ、Reverb 与 Compressor，检验多设备精确改参
    let lead_pos = ui_track_pos(ui, lead_id);
    ui.invoke_track_selected((lead_pos + 1) as i32);
    ui.invoke_track_instrument_change((lead_pos + 1) as i32, "synth:hollow".into());
    ui.invoke_device_insert_eq();
    ui.invoke_device_insert_reverb();
    ui.invoke_compressor_cycle_ratio();

    // 切到第 2 条钢琴轨也装上 synth:organ（验证 device-param-changed 精确按设备+参数名解析，不误伤他轨也不误覆盖 slot 0）
    let piano_pos = ui_track_pos(ui, piano_id);
    ui.invoke_track_selected((piano_pos + 1) as i32);
    ui.invoke_track_instrument_change((piano_pos + 1) as i32, "synth:organ".into());

    // 切回导唱长笛，通过 device-param-changed 精确调节 EQ 的 eq_high_gain 与 Reverb 的 reverb_wet
    let lead_pos = ui_track_pos(ui, lead_id);
    ui.invoke_track_selected((lead_pos + 1) as i32);
    let before_fx = undo.try_project().expect("加效果器后工程");
    let before_cutoff = before_fx.tracks[&lead_id]
        .devices
        .iter()
        .find(|d| d.kind == DeviceKind::InternalInstrument)
        .and_then(|d| d.params.iter().find(|p| p.name == "cutoff_hz"))
        .map(|p| p.value)
        .expect("导唱长笛合成器含 cutoff_hz");

    ui.invoke_device_param_changed("eq".into(), "eq_high_gain".into(), -2.5);
    ui.invoke_device_param_changed("reverb".into(), "reverb_wet".into(), 0.35);

    // 拖动 EQ 低架增益滑块手势
    let eq_dev_id = ui.get_device_eq_device_id();
    assert!(
        !eq_dev_id.is_empty(),
        "插 EQ 后必须投影出 device-eq-device-id"
    );
    ui.invoke_device_eq_grab(eq_dev_id, 0.5);
    ui.invoke_device_eq_drag(0.58);
    ui.invoke_device_eq_release();

    // 4. 在钢琴轨选中的状态下调用 Hollow Lead 的 cutoff（亦检验 app.slint "cutoff" -> "cutoff_hz" 别名，应精确命中导唱长笛上的 Hollow Lead）
    ui.invoke_track_selected((ui_track_pos(ui, piano_id) + 1) as i32);
    ui.invoke_device_param_changed("Hollow Lead".into(), "cutoff".into(), 1950.0);

    let after_fx = undo.try_project().expect("调参后工程");
    let lead_devs = &after_fx.tracks[&lead_id].devices;
    let lead_cutoff = lead_devs
        .iter()
        .find(|d| d.name == "Hollow Lead")
        .and_then(|d| d.params.iter().find(|p| p.name == "cutoff_hz"))
        .map(|p| p.value)
        .expect("Hollow Lead cutoff_hz 存在");
    let eq_high = lead_devs
        .iter()
        .find(|d| d.name == "EQ")
        .and_then(|d| d.params.iter().find(|p| p.name == "eq_high_gain"))
        .map(|p| p.value)
        .expect("EQ eq_high_gain 存在");
    let eq_low = lead_devs
        .iter()
        .find(|d| d.name == "EQ")
        .and_then(|d| d.params.iter().find(|p| p.name == "eq_low_gain"))
        .map(|p| p.value)
        .expect("EQ eq_low_gain 存在");
    let rev_wet = lead_devs
        .iter()
        .find(|d| d.name == "Reverb")
        .and_then(|d| d.params.iter().find(|p| p.name == "reverb_wet"))
        .map(|p| p.value)
        .expect("Reverb reverb_wet 存在");

    assert!(
        (before_cutoff - 2400.0).abs() < 0.01 && (lead_cutoff - 1950.0).abs() < 0.01,
        "Hollow Lead cutoff 必须从 2400 精准改到 1950 且不被 EQ 误覆盖: before={before_cutoff}, after={lead_cutoff}"
    );
    assert!((eq_high - (-2.5)).abs() < 0.01, "EQ eq_high_gain={eq_high}");
    assert!(
        eq_low.abs() > 0.5,
        "EQ 低架拖动应真实改动 eq_low_gain={eq_low}"
    );
    assert!((rev_wet - 0.35).abs() < 0.01, "Reverb reverb_wet={rev_wet}");

    // 5. 建 Aux 发送边并推子拉发送量
    ui.invoke_track_selected((ui_track_pos(ui, lead_id) + 1) as i32);
    ui.invoke_send_create();
    let after_send = undo.try_project().expect("建立 Send 后工程");
    let send_edge_id = after_send
        .routing_graph
        .edges
        .values()
        .find(|e| e.kind == RoutingKind::SendToAux)
        .expect("UI send-create 必须建出 SendToAux 边")
        .id;
    let send_edge_str = send_edge_id.to_canonical_string();
    ui.invoke_send_gain_grab(send_edge_str.clone().into(), 0.72);
    ui.invoke_send_gain_drag(send_edge_str.clone().into(), 0.65);
    ui.invoke_send_gain_release(send_edge_str.into());
    let after_send_gain = undo.try_project().expect("调 Send 增益后");
    let send_gain = after_send_gain.routing_graph.edges[&send_edge_id]
        .gain_db
        .expect("松手后 Send 增益必须写回模型");
    assert!(send_gain.is_finite(), "Send gain_db={send_gain}");

    // 6. 切换停靠栏 + 编排/卷帘二维滚轮 + 缩放
    // (注：app.slint apply-dock-tab 定义 pianoroll=0, mixer=1, devices=2；与 host::on_dock_tab_selected 的旧 ConsoleTabs 顺序不一致，报告已记录)
    ui.invoke_apply_dock_tab("mixer".into());
    assert_eq!(ui.get_console_tab(), 1);
    ui.invoke_apply_dock_tab("devices".into());
    assert_eq!(ui.get_console_tab(), 2);
    ui.invoke_apply_dock_tab("pianoroll".into());
    assert_eq!(ui.get_console_tab(), 0);

    ui.invoke_arrangement_wheel(-64.0, -48.0, false, false);
    assert!(
        ui.get_roll_scroll_x() > 0.0,
        "编排横向手势滚轮必须改变 roll_scroll_x: {}",
        ui.get_roll_scroll_x()
    );
    ui.invoke_arrangement_wheel(64.0, 48.0, false, false);

    ui.invoke_piano_roll_wheel(-96.0, -32.0, false, false);
    assert!(ui.get_roll_scroll_x() > 0.0, "卷帘横向滚轮必须响应");
    let scrolled_x = ui.get_roll_scroll_x();
    ui.invoke_normalize_track_heights();
    assert!(
        (ui.get_roll_scroll_x() - scrolled_x).abs() < 0.01,
        "归一化轨高不得意外清零当前横向滚动偏移: expected={scrolled_x}, got={}",
        ui.get_roll_scroll_x()
    );
    ui.invoke_piano_roll_wheel(96.0, 32.0, false, false);
    ui.invoke_zoom_h_step(true);
    ui.invoke_zoom_h_step(false);

    // 7. 琴键试听 + 旋律创作（首拍 + 空白建段 + 跨隙后段 + 调力度 + 拆分 + 人性化 + 量化）
    ui.invoke_piano_key_pressed(24);
    ui.invoke_piano_key_released(24);
    ui.invoke_chord_audition("Cm7".into());

    // 在导唱长笛上建空片段并用画笔填入 C4 - Eb4 - G4 - F4 老上海五声拐弯旋律
    ui.invoke_track_selected((ui_track_pos(ui, lead_id) + 1) as i32);
    ui.invoke_clip_create();
    ui.invoke_edit_tool_selected("pencil".into());

    let melody_pitches = [60u8, 63, 67, 65, 63, 60];
    let melody_ticks = [0u64, 480, 960, 1440, 1920, 2400];
    ui.set_roll_scroll_x(0.0);
    for (&pitch, &tick) in melody_pitches.iter().zip(melody_ticks.iter()) {
        let host_x = tick_to_px(tick, DEFAULT_TICKS_PER_PIXEL).expect("tick px") as f32 + 1.0;
        let host_y = pitch_lane_y(pitch) + 6.0;
        let (px, py) = host_to_painted(host_x, host_y);
        ui.invoke_clicked(px, py);
    }

    // 选中钢琴轨同样自动建段并填入柱式和弦低音+和声
    ui.invoke_track_selected((ui_track_pos(ui, piano_id) + 1) as i32);
    ui.invoke_piano_note_create(36, 0); // 自动新建片段写根音
    ui.invoke_piano_note_create(24, 4); // 第 2 拍和弦
    ui.invoke_piano_note_create(21, 8); // 第 3 拍和弦

    // 选中导唱片段并调力度 + 拆分 + 人性化 + 量化
    let mid_proj = undo.try_project().expect("填入音符后工程");
    let lead_placement = mid_proj.tracks[&lead_id]
        .clips
        .values()
        .next()
        .expect("导唱片段存在");
    let lead_clip_ulid = lead_placement.id.to_canonical_string();
    let lead_clip_id = lead_placement.clip_id;
    let pre_count = mid_proj.clip_pool[&lead_clip_id]
        .content
        .notes()
        .expect("MIDI notes")
        .len();
    assert_eq!(pre_count, melody_pitches.len(), "导唱片段应有 6 颗音符");

    ui.invoke_track_selected((ui_track_pos(ui, lead_id) + 1) as i32);
    ui.set_selected_clip_ulid(lead_clip_ulid.into());
    ui.invoke_piano_note_select(1);
    ui.invoke_piano_note_velocity(1, 108);
    ui.invoke_piano_note_split(1);

    let after_split = undo.try_project().expect("拆分后工程");
    assert_eq!(
        after_split.clip_pool[&lead_clip_id]
            .content
            .notes()
            .unwrap()
            .len(),
        pre_count + 1,
        "拆分音符必须多出 1 颗音符"
    );

    // 打开人性化对话框并应用，接着量化
    ui.invoke_open_humanize();
    assert!(ui.get_humanize_open());
    ui.set_humanize_seed("1936".into());
    ui.set_humanize_time_jitter("18".into());
    ui.set_humanize_velocity_jitter("10".into());
    ui.invoke_apply_humanize();
    assert!(!ui.get_humanize_open(), "应用人性化成功应关闭弹窗");

    ui.invoke_open_quantize();
    ui.set_humanize_grid("1/16".into());
    ui.set_humanize_strength("40".into());
    ui.invoke_apply_quantize();

    // 8. 混音台推子/声像/静音/独奏 & 撤销重做
    let piano_num = (ui_track_pos(ui, piano_id) + 1) as i32;
    let lead_num = (ui_track_pos(ui, lead_id) + 1) as i32;
    ui.invoke_track_volume_changed(piano_num, -4.5);
    ui.invoke_track_pan_changed(piano_num, -0.25);
    ui.invoke_track_pan_changed(lead_num, 0.12);
    ui.invoke_track_toggle_mute(piano_num);
    assert!(undo.try_project().unwrap().tracks[&piano_id].mute);
    ui.invoke_undo_step();
    assert!(
        !undo.try_project().unwrap().tracks[&piano_id].mute,
        "撤销应恢复未静音"
    );
    ui.invoke_redo_step();
    assert!(
        undo.try_project().unwrap().tracks[&piano_id].mute,
        "重做应重新静音"
    );
    ui.invoke_track_toggle_mute(piano_num); // 恢复打开以便导出合奏

    // 9. 导出 48kHz 24-bit WAV 验收真实有声输出且零削波
    let final_proj = undo.try_project().expect("最终工程");
    let wav_path = std::env::temp_dir().join(format!("yeban_human_sim_{}.wav", std::process::id()));
    let opts = WavExportOptions {
        sample_rate: 48_000,
        frames: 48_000 * 3,
        limiter: true,
        ..Default::default()
    };
    export_wav(&final_proj, &wav_path, opts).expect("仿真作品 WAV 导出必须成功");
    let (mono, rate, _) = read_plain_wav_mono_f32(&wav_path).expect("读取导出 WAV");
    let _ = std::fs::remove_file(&wav_path);
    assert_eq!(rate, 48_000);
    let peak = mono.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        peak > 0.01 && peak <= 1.0,
        "导出母带必须有明显音能且不削波: peak={peak}"
    );
}
