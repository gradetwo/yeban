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
use yeban_tui::keymap::{Mode, Pane};
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
        let mut x = 0;
        while x < buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            out.push_str(symbol);
            // 双宽字符（中文等）的后继格是占位空格，跳过它文本才连读。
            x += if unicode_width::UnicodeWidthStr::width(symbol) >= 2 {
                2
            } else {
                1
            };
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
        "000.0",
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
    app.tick();
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
        app.tick();
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
    app.tick();
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
    // 导出帧数取工程时长：演示工程内容 5.0 秒 × 48 kHz = 240,000 帧。
    assert_eq!(receipt.frames, 240_000);
    let bytes = std::fs::read(&path).expect("read wav");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(receipt.bytes, bytes.len() as u64);
    // 24-bit 立体声：数据量 ≈ 帧数 × 2 声道 × 3 字节 + WAV 头。
    let expected_min = 240_000_u64 * 2 * 3;
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

/// K3 撤销/重做接线：GUI 键（Ctrl+Z / Ctrl+Shift+Z）与 TUI 自加的 Vi 键
/// （`u` / Ctrl+R）落到同一个撤销端口；计数撤销一次退多步。
#[test]
fn undo_redo_wired_through_gui_and_vi_keys() {
    let mut app = App::with_demo().expect("demo app");
    let track_id = app.track_ids[0];
    assert!(!app.project.tracks[&track_id].mute);

    // GUI 共享键 m 静音 → 进提交史。
    app.handle_key(char_key('m'));
    assert!(app.project.tracks[&track_id].mute, "m 应切换静音");
    // Vi `u` 撤销。
    app.handle_key(char_key('u'));
    assert!(!app.project.tracks[&track_id].mute, "u 应撤销静音");
    // Vi `Ctrl+R` 重做。
    app.handle_key(key_with(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert!(app.project.tracks[&track_id].mute, "Ctrl+R 应重做静音");
    // GUI `Ctrl+Z` 撤销。
    app.handle_key(key_with(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert!(!app.project.tracks[&track_id].mute, "Ctrl+Z 应撤销静音");
    // GUI `Ctrl+Shift+Z` 重做。
    app.handle_key(key_with(
        KeyCode::Char('z'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(
        app.project.tracks[&track_id].mute,
        "Ctrl+Shift+Z 应重做静音"
    );

    // 计数撤销：再叠两笔编辑（第二条轨静音 + 本轨独奏切换），3u 一次全退。
    // 注意：演示工程首轨的初始独奏状态由夹具决定（实测为开），只断言「切换」。
    let was_solo = app.project.tracks[&track_id].solo;
    let other_id = app.track_ids[1];
    app.handle_key(char_key('j'));
    app.handle_key(char_key('m'));
    assert!(app.project.tracks[&other_id].mute);
    app.handle_key(char_key('k'));
    app.handle_key(char_key('s'));
    assert_eq!(
        app.project.tracks[&track_id].solo, !was_solo,
        "s 应切换独奏"
    );
    app.handle_key(char_key('3'));
    app.handle_key(char_key('u'));
    assert!(!app.project.tracks[&track_id].mute, "3u 应退掉第一笔静音");
    assert!(!app.project.tracks[&other_id].mute, "3u 应退掉第二笔静音");
    assert_eq!(
        app.project.tracks[&track_id].solo, was_solo,
        "3u 应把独奏退回初始态"
    );
    // 史已空：再按 u 只如实被拒，工程不动。
    app.handle_key(char_key('u'));
    assert!(!app.project.tracks[&track_id].mute);
}

/// K6：`Tab` 在三个窗格间循环焦点（轨道→片段→检查器→轨道），
/// 不改模式、不改选轨。
#[test]
fn tab_cycles_pane_focus() {
    let mut app = App::with_demo().expect("demo app");
    assert_eq!(app.focus, Pane::Tracks);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Grid, "Tab 1 应到片段网格");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Inspector, "Tab 2 应到检查器");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Tracks, "Tab 3 应回到轨道");
    assert_eq!(app.mode, Mode::Normal, "Tab 不应改模式");
    assert_eq!(app.selected, 0, "Tab 不应改选轨");
}

/// 编辑一轮：片段网格焦点下 `L` 平移片段（走 MoveClipPlacement）、
/// GUI `Delete` 删摆放、`:tempo` 改速度、`:seek` 定位播放头；
/// 每笔文档编辑都可撤销。
#[test]
fn grid_clip_move_delete_tempo_and_seek() {
    let mut app = App::with_demo().expect("demo app");
    // 找第一条带摆放的轨与其起点最早的摆放。
    let mut found = None;
    for (index, id) in app.track_ids.iter().enumerate() {
        if let Some(track) = app.project.tracks.get(id)
            && let Some(placement) = track.clips.values().min_by_key(|p| p.start_tick)
        {
            found = Some((index, *id, placement.id, placement.start_tick));
            break;
        }
    }
    let (index, track_id, placement_id, old_start) = found.expect("演示工程应有片段摆放");
    app.selected = index;
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Grid);
    app.cursor_bar = usize::try_from(old_start / app.bar_ticks()).unwrap_or(0);

    // L：右移一小节（TUI 自加键，计数前缀 = 小节数）。
    app.handle_key(key_with(KeyCode::Char('L'), KeyModifiers::SHIFT));
    let moved = app.project.tracks[&track_id].clips[&placement_id].start_tick;
    assert_eq!(moved, old_start + app.bar_ticks(), "L 应右移一小节");
    // u 撤销平移。
    app.handle_key(char_key('u'));
    assert_eq!(
        app.project.tracks[&track_id].clips[&placement_id].start_tick, old_start,
        "u 应撤销平移"
    );
    // 光标还停在平移后的位置（撤销不挪光标，与 GUI 同）：先把光标移回片段起点。
    app.cursor_bar = usize::try_from(old_start / app.bar_ticks()).unwrap_or(0);

    // Delete：删光标处摆放（GUI 画布键 DeleteSelection 的落点）。
    app.handle_key(key(KeyCode::Delete));
    assert!(
        !app.project.tracks[&track_id]
            .clips
            .contains_key(&placement_id),
        "Delete 应删掉摆放"
    );
    // Ctrl+Z 恢复摆放。
    app.handle_key(key_with(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert!(
        app.project.tracks[&track_id]
            .clips
            .contains_key(&placement_id),
        "Ctrl+Z 应恢复摆放"
    );

    // :tempo 改速度（经撤销端口），u 恢复。
    let old_bpm = app.project.bpm;
    app.execute_command("tempo 90");
    assert_eq!(app.project.bpm, 90.0, ":tempo 应改工程速度");
    app.handle_key(char_key('u'));
    assert_eq!(app.project.bpm, old_bpm, "u 应恢复速度");

    // :seek 定位播放头（走带状态，不进撤销史）。
    app.execute_command("seek 2");
    app.tick();
    assert_eq!(app.cursor_bar, 2);
    assert_eq!(
        app.transport().position_ticks,
        2 * app.bar_ticks(),
        ":seek 应把播放头定位到第 2 小节起点"
    );
}
