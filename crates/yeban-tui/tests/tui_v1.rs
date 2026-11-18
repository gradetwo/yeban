//! yeban-tui v1 集成测试：布局快照（TestBackend）、按键分发（GUI 共享键 + Vi 层）、
//! 走带墙钟推进、导出 WAV、保存/重开往返。贯穿夹具 = `yeban_model::samples::demo_project()`
//! （6 条内容轨 + 主总线，含真实合成音频资产）。

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use yeban_engine::transport::TransportState;
use yeban_tui::app::App;
use yeban_tui::keymap::Mode;
use yeban_tui::ui;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn char_key(ch: char) -> KeyEvent {
    key(KeyCode::Char(ch))
}

fn temp_path(tag: &str, ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yeban-tui-test-{}-{tag}.{ext}", std::process::id()))
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

/// 布局：顶栏/四栏/底栏齐全，演示工程的标题与轨道名都渲染出来，
/// 播放头标记 ▼ 出现在片段网格内。
#[test]
fn layout_renders_demo_project_panes() {
    let mut app = App::with_demo().expect("demo app");
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(&terminal);
    for needle in [
        "夜半 Yeban",
        "轨道",
        "片段",
        "检查器",
        "鼓",
        "贝斯",
        "Master",
        "NORMAL",
        "001.1",
        "▼",
        "乐器",
        "音量",
    ] {
        assert!(text.contains(needle), "布局缺少「{needle}」:\n{text}");
    }
    // 轨道行数 = 6 条内容轨 + 主总线 = 7。
    assert_eq!(app.track_ids.len(), 7);
}

/// 按键分发：共享动作走 GUI 键，导航走 Vi 键，两层不串。
#[test]
fn key_dispatch_gui_shared_and_vi_layers() {
    let mut app = App::with_demo().expect("demo app");
    assert_eq!(app.selected, 0);

    // Vi 计数 + 移动：5j 下移 5 条。
    app.handle_key(char_key('5'));
    app.handle_key(char_key('j'));
    assert_eq!(app.selected, 5);
    // gg 回首、G 到尾。
    app.handle_key(char_key('g'));
    app.handle_key(char_key('g'));
    assert_eq!(app.selected, 0);
    app.handle_key(key_with(KeyCode::Char('G'), KeyModifiers::SHIFT));
    assert_eq!(app.selected, 6);

    // GUI 共享键：Space 播放/暂停（走 GUI 解析器）。
    app.handle_key(char_key(' '));
    assert!(app.is_playing(), "Space 后应处于播放态");
    app.handle_key(char_key(' '));
    app.tick(0.05);
    assert!(!app.is_playing(), "再按 Space 应暂停/停止");

    // GUI 共享键：m 静音选中轨（主总线初始未静音）。
    let master_id = app.selected_track_id().expect("selected");
    assert!(!app.project.tracks[&master_id].mute);
    app.handle_key(char_key('m'));
    assert!(app.project.tracks[&master_id].mute, "m 应切换静音");
    assert!(app.dirty, "编辑后应置脏");

    // GUI 共享键：Ctrl+Alt+↑ 音量 +1.0 dB（K2：不另发明 +/- 音量键）。
    let before = app.project.tracks[&master_id].volume_db;
    app.handle_key(key_with(
        KeyCode::Up,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ));
    let after = app.project.tracks[&master_id].volume_db;
    assert!(
        (after - before - 1.0).abs() < 0.001,
        "音量应 +1.0 dB：{before} → {after}"
    );

    // Insert 改名：i 进、输入、Enter 提交。
    app.handle_key(char_key('g'));
    app.handle_key(char_key('g'));
    app.handle_key(char_key('i'));
    assert_eq!(app.mode, Mode::Insert);
    app.handle_key(key_with(KeyCode::Char('X'), KeyModifiers::SHIFT));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.mode, Mode::Normal);
    let first_id = app.track_ids[0];
    assert!(
        app.project.tracks[&first_id].name.ends_with('X'),
        "改名应生效：{}",
        app.project.tracks[&first_id].name
    );

    // 命令行：:q 在脏状态下被拦，:q! 强退。
    app.handle_key(key_with(KeyCode::Char(':'), KeyModifiers::SHIFT));
    assert_eq!(app.mode, Mode::Command);
    app.handle_key(char_key('q'));
    app.handle_key(key(KeyCode::Enter));
    assert!(!app.quit, "脏状态下 :q 应被拦");
    app.execute_command("q!");
    assert!(app.quit, ":q! 应退出");
}

/// 走带：控制面墙钟泵送下播放头按真实时间前进；停止后状态回 Stopped。
#[test]
fn transport_advances_under_wall_clock_pump() {
    let mut app = App::with_demo().expect("demo app");
    app.toggle_play();
    assert!(app.is_playing());
    // 墙钟泵送读真实时钟：给它真实时间推进。
    let mut advanced = 0_u64;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        app.tick(0.05);
        advanced = app.transport().position_ticks;
        if advanced > 0 {
            break;
        }
    }
    assert!(
        advanced > 0,
        "播放 2 秒内播放头应前进，实际 position_ticks = {advanced}"
    );
    app.stop();
    app.tick(0.05);
    assert_eq!(app.transport().state, TransportState::Stopped);
}

/// 导出 WAV：走 render_core 真渲染，文件落盘且字节数与帧数对得上。
#[test]
fn export_wav_writes_real_audio_file() {
    let mut app = App::with_demo().expect("demo app");
    let path = temp_path("export", "wav");
    let receipt = app.export_wav(&path).expect("export");
    assert_eq!(receipt.sample_rate, 48_000);
    assert_eq!(receipt.bits, 24);
    // 默认导出 8 秒 × 48 kHz = 384,000 帧。
    assert_eq!(receipt.frames, 384_000);
    let bytes = std::fs::read(&path).expect("read wav");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(receipt.bytes, bytes.len() as u64);
    // 24-bit 立体声：数据量 ≈ 帧数 × 2 声道 × 3 字节 + WAV 头。
    let expected_min = 384_000_u64 * 2 * 3;
    assert!(
        receipt.bytes >= expected_min && receipt.bytes < expected_min + 4096,
        "WAV 字节数异常：{}",
        receipt.bytes
    );
    let _ = std::fs::remove_file(&path);
}

/// 保存/重开往返：演示工程经全保真归档写出后能原样读回。
#[test]
fn save_and_reopen_roundtrip() {
    let mut app = App::with_demo().expect("demo app");
    let path = temp_path("roundtrip", "yeban");
    app.save_to(&path).expect("save");
    let meta = std::fs::metadata(&path).expect("metadata");
    assert!(meta.len() > 10_000, "归档过小：{} 字节", meta.len());
    let reopened = App::open_file(&path).expect("reopen");
    assert_eq!(reopened.project.title, app.project.title);
    assert_eq!(reopened.track_ids.len(), app.track_ids.len());
    assert_eq!(reopened.project.bpm, app.project.bpm);
    let _ = std::fs::remove_file(&path);
}
