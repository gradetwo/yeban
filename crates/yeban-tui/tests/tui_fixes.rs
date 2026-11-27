//! 真实 TTY 测试抓到的 TUI 缺口：回归判据
//! （seek 钳位 / export 无参 / tempo 范围 / 改名提示 / 音频字节接线）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_model::ids::EntityId;
use yeban_model::project::TrackKind;
use yeban_tui::app::App;
use yeban_tui::keymap::Pane;

fn app() -> App {
    App::with_demo().expect("demo app")
}

#[test]
fn seek_is_clamped_to_project_length() {
    let mut app = app();
    let total = app.total_bars();
    app.seek_to_bar(99);
    assert_eq!(app.cursor_bar, total - 1, "越界必须钳到最后一小节");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(last.contains("工程只有"), "钳位必须说人话：{last}");
}

#[test]
fn export_without_path_is_a_usage_error() {
    let mut app = app();
    app.execute_command("export wav");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(
        last.starts_with("用法：:export"),
        "必须报用法而不是写 `wav` 文件：{last}"
    );
    assert!(app.last_export.is_none(), "⛔ 不许真的导出");
}

#[test]
fn tempo_out_of_range_explains_the_range() {
    let mut app = app();
    app.execute_command("tempo 0");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(last.contains("20..=999"), "越界必须给合法范围：{last}");
}

#[test]
fn rename_mode_mentions_ctrl_u() {
    let mut app = app();
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(last.contains("Ctrl+U"), "改名入口必须提示清空键：{last}");
}

#[test]
fn demo_audio_assets_reach_the_engine() {
    let app = app();
    let (injected, skipped) = app.audio_injection_counts();
    assert_eq!(skipped, 0, "演示工程的音频片段不该再被跳过");
    assert!(injected >= 1, "至少注入一条音频素材（实测 {injected}）");
}

#[test]
fn notes_key_outside_grid_says_where_to_go() {
    let mut app = app();
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(last.contains("Tab 到片段网格"), "导航键必须给落点：{last}");
}

#[test]
fn track_command_creates_a_track_and_selects_it() {
    let mut app = app();
    let before = app.track_ids.len();
    app.execute_command("track midi");
    assert_eq!(
        app.track_ids.len(),
        before + 1,
        "`:track midi` 必须真的多出一条轨"
    );
    let selected = app.track_ids[app.selected];
    let track = app.project.tracks.get(&selected).expect("selected track");
    assert_eq!(track.kind, TrackKind::Midi, "新建的必须是 MIDI 轨");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(last.contains("已新建"), "建完必须说人话：{last}");
}

#[test]
fn clip_command_places_a_clip_at_the_cursor_bar() {
    let mut app = app();
    app.execute_command("seek 3");
    let track_id = app.selected_track_id().expect("a selected track");
    let before: Vec<EntityId> = app
        .project
        .tracks
        .get(&track_id)
        .expect("track")
        .clips
        .keys()
        .copied()
        .collect();
    app.execute_command("clip");
    let track = app.project.tracks.get(&track_id).expect("track");
    let fresh = track
        .clips
        .iter()
        .find(|(id, _)| !before.contains(id))
        .map(|(_, placement)| placement)
        .expect("`:clip` 必须新增一条摆放");
    assert_eq!(
        fresh.start_tick,
        u64::try_from(app.cursor_bar).unwrap() * app.bar_ticks(),
        "片段必须落在光标所在小节，⛔ 不是固定第 1 小节"
    );
}

#[test]
fn track_command_rejects_unknown_kind_with_usage() {
    let mut app = app();
    let before = app.track_ids.len();
    app.execute_command("track drums");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(
        last.starts_with("用法：:track"),
        "未知类型必须报用法：{last}"
    );
    assert_eq!(app.track_ids.len(), before, "⛔ 不许悄悄建轨");
}

/// T-FIX-799 红对照：轨道焦点 Delete 必须删选中内容轨，且一步可撤销。
#[test]
fn delete_on_track_focus_removes_selected_track_and_is_undoable() {
    let mut app = app();
    assert_eq!(app.focus, Pane::Tracks, "夹具必须从轨道焦点开始");
    let before = app.track_ids.len();
    let track_id = app.track_ids[0];
    let track_name = app.project.tracks[&track_id].name.clone();

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

    assert!(
        !app.project.tracks.contains_key(&track_id),
        "轨道焦点 Delete 必须删掉选中轨 `{track_name}`"
    );
    assert_eq!(app.track_ids.len(), before - 1, "轨道列表必须同步少一条");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(
        last.contains("删除轨道") && last.contains(&track_name),
        "删轨回执必须点名删了哪条轨：{last}"
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert!(
        app.project.tracks.contains_key(&track_id),
        "Ctrl+Z 必须恢复刚删的轨道（一步撤销）"
    );
}

/// T-FIX-799 红对照：主总线不可删，拒绝理由必须逐字响亮。
#[test]
fn delete_on_master_track_is_loudly_rejected() {
    let mut app = app();
    let master = app.project.master_bus_track_id;
    let master_index = app
        .track_ids
        .iter()
        .position(|id| *id == master)
        .expect("demo 的 track_ids 必须包含主总线");
    app.selected = master_index;
    let before = app.track_ids.len();

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

    assert_eq!(app.track_ids.len(), before, "主总线绝不能被删掉");
    assert!(app.project.tracks.contains_key(&master), "主总线仍在工程里");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(
        last.contains("主总线（Master）不可删除"),
        "拒绝理由必须逐字响亮：{last}"
    );
}

/// T-FIX-799：`:track delete` 与轨道焦点 Delete 同一落点（删选中内容轨）。
#[test]
fn track_delete_command_removes_the_selected_track() {
    let mut app = app();
    let before = app.track_ids.len();
    let track_id = app.track_ids[0];
    let track_name = app.project.tracks[&track_id].name.clone();

    app.execute_command("track delete");

    assert!(
        !app.project.tracks.contains_key(&track_id),
        "`:track delete` 必须删掉选中轨 `{track_name}`"
    );
    assert_eq!(app.track_ids.len(), before - 1, "轨道列表必须同步少一条");
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(
        last.contains("已删除轨道") && last.contains(&track_name),
        "命令回执必须点名删了哪条轨：{last}"
    );
}

/// T-FIX-800：R 走共享 `ToggleRecord`，无设备时起录失败必须响亮、且不进录音态。
struct NoDeviceFactory;

impl yeban_app::record_device::InputSourceFactory for NoDeviceFactory {
    fn open(
        &self,
        _options: &yeban_app::record_wav::RecordOptions,
    ) -> Result<Box<dyn yeban_app::record_wav::InputSource + Send>, String> {
        Err("没有可用音频输入设备".to_owned())
    }

    fn label(&self) -> &'static str {
        "no-device-test"
    }
}

fn record_temp_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("yeban-tui-record-{tag}-{}.wav", std::process::id()))
}

fn select_audio_track(app: &mut App) {
    let audio = app
        .track_ids
        .iter()
        .copied()
        .find(|id| {
            app.project
                .tracks
                .get(id)
                .is_some_and(|track| track.kind == TrackKind::Audio)
        })
        .expect("demo 至少一条音频轨");
    app.selected = app
        .track_ids
        .iter()
        .position(|id| *id == audio)
        .expect("音频轨在轨道条里");
}

/// 无设备：R 不落“未实现”，而是 `起录失败`，状态保持 Stopped。
#[test]
fn record_key_without_input_device_is_loud() {
    let mut app = app();
    let path = record_temp_path("no-device");
    app.set_record_seam(std::rc::Rc::new(NoDeviceFactory), path.clone());

    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));

    assert!(!app.is_recording(), "起录失败时不许置录音态");
    assert_eq!(
        app.engine_status().transport_state,
        yeban_engine::transport::TransportState::Stopped,
        "起录失败时走带必须仍是 Stopped"
    );
    let last = app.log.last().cloned().unwrap_or_default();
    assert!(
        last.contains("起录失败") && last.contains("没有可用音频输入设备"),
        "无设备必须响亮报错：{last}"
    );
    let _ = std::fs::remove_file(path);
}

/// R 进入/退出录音态：静音源注入（不依赖声卡），状态投影与引擎走带一致。
#[test]
fn record_key_enters_and_exits_recording_state() {
    let mut app = app();
    select_audio_track(&mut app);
    let path = record_temp_path("key");
    app.set_record_seam(
        std::rc::Rc::new(yeban_app::record_device::SilenceFactory),
        path.clone(),
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.is_recording(), "第一次 R 必须进入录音态");
    assert_eq!(
        app.engine_status().transport_state,
        yeban_engine::transport::TransportState::Recording,
        "录音态必须投影到走带读数"
    );
    let started = app.log.last().cloned().unwrap_or_default();
    assert!(
        started.contains("开始录音") && started.contains(&path.display().to_string()),
        "起录回执必须点名落盘路径：{started}"
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(!app.is_recording(), "第二次 R 必须退出录音态");
    assert_eq!(
        app.engine_status().transport_state,
        yeban_engine::transport::TransportState::Stopped,
        "停录后走带必须回到 Stopped"
    );
    let stopped = app.log.last().cloned().unwrap_or_default();
    assert!(
        stopped.contains("录音"),
        "停录必须给出落盘/空帧/失败回执：{stopped}"
    );
    let _ = std::fs::remove_file(path);
}

/// `:record` 与 R 共用同一落点。
#[test]
fn record_command_enters_and_exits_recording_state() {
    let mut app = app();
    select_audio_track(&mut app);
    let path = record_temp_path("command");
    app.set_record_seam(
        std::rc::Rc::new(yeban_app::record_device::SilenceFactory),
        path.clone(),
    );

    app.execute_command("record");
    assert!(app.is_recording(), "`:record` 第一次必须进入录音态");
    app.execute_command("record");
    assert!(!app.is_recording(), "`:record` 第二次必须退出录音态");
    let _ = std::fs::remove_file(path);
}
