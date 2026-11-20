//! 真实 TTY 测试抓到的 TUI 缺口：回归判据
//! （seek 钳位 / export 无参 / tempo 范围 / 改名提示 / 音频字节接线）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_model::ids::EntityId;
use yeban_model::project::TrackKind;
use yeban_tui::app::App;

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
