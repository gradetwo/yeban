//! T-FIX-81：四方对齐 TUI 补齐批 2 的判据。
//!
//! 四项各一组"红→绿"：
//!
//! | 项 | 判据 | 钉住的事实 |
//! | :--- | :--- | :--- |
//! | 引擎状态面 | `engine_status_reads_the_existing_sources` | 采样率/帧数/通道/设备/走带都从既有读数来 |
//! | 通道电平 | `track_meters_share_the_master_spsc_chain` | 逐轨峰值与主总线同一份抽干结果，且按跳衰减 |
//! | 60Hz 心跳 | `meter_heartbeat_matches_the_gui_period` | 16 ms / 62.5 Hz，与 GUI `HEARTBEAT_PERIOD_MS` 同口径 |
//! | 强制保存 | `force_save_requires_a_real_path_and_really_writes` | 无路径响亮报错；有路径写出的文件能被 `open_file` 读回 |
//! | 重载引擎 | `reload_engine_rebuilds_and_reports_readings` | 落点是 `EngineHost::reload`，回执带代数/版本/轨数 |

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_engine::transport::TransportState;
use yeban_tui::app::{App, METER_HEARTBEAT_MS};

fn demo_app() -> App {
    App::with_demo().expect("演示工程")
}

fn last_log(app: &App) -> String {
    app.log.last().cloned().unwrap_or_default()
}

#[test]
fn engine_status_reads_the_existing_sources() {
    let app = demo_app();
    let status = app.engine_status();

    // 工程采样率：`project.audio_config.sample_rate` 的唯一换算入口。
    assert_eq!(
        status.project_sample_rate,
        yeban_tui::app::sample_rate_hz(app.project.audio_config.sample_rate),
        "工程采样率必须走 `sample_rate_hz`"
    );
    // 引擎侧读数来自快照自己（不是从工程再推一遍）。
    let (snapshot_rate, _) = app.snapshot_readings();
    assert_eq!(
        status.engine_sample_rate, snapshot_rate,
        "引擎采样率必须来自 `EngineSnapshot::sample_rate()`"
    );
    assert!(status.block_frames.is_some(), "有引擎 ⇒ 有每块帧数");
    assert!(status.channels.is_some(), "有引擎 ⇒ 有通道数");

    // TUI 形态：不开设备腿（墙钟泵送），如实说"无实时转换"。
    assert!(!status.device_active, "TUI 默认不开设备腿");
    assert!(status.device_name.is_none());
    assert!(
        status.conversion.contains("未开设备腿"),
        "转换状态必须如实说明没有设备腿：{}",
        status.conversion
    );

    // 走带读数与 `App::transport()` 同一口径。
    assert_eq!(status.transport_state, app.transport().state);
    assert_eq!(status.position_ticks, app.transport().position_ticks);
    assert_ne!(status.transport_state, TransportState::Recording);
}

#[test]
fn track_meters_share_the_master_spsc_chain() {
    let mut app = demo_app();
    let tracks = app.project.display_track_ids();
    assert!(tracks.len() >= 2, "演示工程至少两条轨");
    for id in &tracks {
        assert_eq!(app.track_peak(id), 0.0, "开局没有电平读数");
    }

    // 播放并推量子：同一份 SPSC 队列抽干后，主总线与逐轨都要有读数。
    app.toggle_play();
    app.drive_meters_for_test(8);
    assert!(app.master_peak > 0.0, "主总线电平应有读数");
    let loudest = tracks
        .iter()
        .map(|id| app.track_peak(id))
        .fold(0.0_f32, f32::max);
    assert!(loudest > 0.0, "逐轨电平应有读数（同一份抽干结果）");

    // 不再喂音频：下一跳必须衰减（保持不是"卡住上一帧"）。
    let before = loudest;
    app.drive_meters_for_test(0);
    let after = tracks
        .iter()
        .map(|id| app.track_peak(id))
        .fold(0.0_f32, f32::max);
    assert!(
        after < before,
        "没有新帧时电平必须衰减：{before} -> {after}"
    );
}

#[test]
fn meter_heartbeat_matches_the_gui_period() {
    // GUI：`crates/yeban-app/src/main.rs` 的 `HEARTBEAT_PERIOD_MS = 16` ⇒ 62.5 Hz。
    // `black_box` 让这两个值在运行时才求值（判据本身不是常量折叠的废话）。
    let period = std::hint::black_box(METER_HEARTBEAT_MS);
    let gui_period = std::hint::black_box(yeban_tui::app::HEARTBEAT_PERIOD_MS_REFERENCE);
    assert_eq!(period, gui_period, "与 GUI 心跳同一个周期");
    assert!(
        1000 / period >= 60,
        "跳频必须不低于 60 Hz（实际 {} Hz）",
        1000 / period
    );
}

#[test]
fn force_save_requires_a_real_path_and_really_writes() {
    let mut app = demo_app();
    app.execute_command("w!");
    assert!(
        last_log(&app).contains("尚无文件路径"),
        "无路径必须响亮报错（与 ui/force_save 同一条）：{}",
        last_log(&app)
    );

    let dir = std::env::temp_dir().join(format!("yeban-tui-fix81-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录");
    let path = dir.join("force-save.yeban");
    app.save_to(&path).expect("先落一次文件拿到路径");
    app.execute_command("w!");
    assert!(
        last_log(&app).contains("强制保存"),
        "强制保存要有明确回执：{}",
        last_log(&app)
    );
    assert!(path.is_file(), "真的写盘");

    // 写出来的文件必须能被生产打开路径读回（不是假成功）。
    let reopened = App::open_file(&path).expect("读回强制保存的文件");
    assert_eq!(reopened.project.title, app.project.title);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn reload_engine_rebuilds_and_reports_readings() {
    let mut app = demo_app();
    let revision_before = app
        .snapshot_readings()
        .1
        .expect("开局应有一代引擎的快照版本");
    app.execute_command("reload");
    let line = last_log(&app);
    assert!(line.contains("重载音频引擎：第"), "回执应带代数：{line}");
    assert!(!line.contains("失败"), "演示工程重载不该失败：{line}");
    let revision_after = app.snapshot_readings().1.expect("重载后仍有快照");
    assert!(
        revision_after > revision_before,
        "重载必须换代（revision {revision_before} -> {revision_after}）"
    );
    // 重载后引擎状态面仍可读（不是"重建把读数弄丢了"）。
    assert!(app.engine_status().engine_sample_rate.is_some());
}

#[test]
fn reload_and_force_save_are_reachable_by_keys_and_commands() {
    let mut app = demo_app();
    // 命令动词两种写法都认（`:reload` / `:reload-engine`）。
    app.execute_command("reload-engine");
    assert!(
        last_log(&app).contains("重载音频引擎"),
        "{}",
        last_log(&app)
    );
    // `w!` 无路径时的失败不会把 TUI 弄崩，也不写脏标记。
    let dirty_before = app.dirty;
    app.execute_command("w!");
    assert_eq!(app.dirty, dirty_before, "失败路径不得改脏标记");
    // Esc / 普通按键路径不受影响（这里只确认按键仍可分发）。
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}
