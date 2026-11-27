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
