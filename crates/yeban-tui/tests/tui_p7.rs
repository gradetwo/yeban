//! T-FIX-86：四方对齐 TUI 补齐批 4 的判据。
//!
//! | 项 | 判据 | 钉住的事实 |
//! | :--- | :--- | :--- |
//! | ① 960 PPQ 时钟 + EntityId | `info_shows_the_clock_and_entity_ids_from_the_model` | 时钟数取自 `yeban_model::ids::PPQ`，身份取自工程/轨道自己的 ULID |
//! | ② 容器契约 | `contract_roundtrips_and_rejects_a_newer_schema_loudly` | 存/开保真；schema 过新 ⇒ 打开响亮拒绝（同一道 `check_readable` 门） |
//! | ③ 无障碍语义 | `semantic_rows_use_gui_roles_and_match_the_visible_labels` | 角色词取自 GUI `ElementKind`，序连续、标签与界面同源 |
//! | ④ 合成态 / 修饰键泄漏 | `ime_and_super_combos_never_reach_shortcuts_or_text_buffers` | 合成中只剩 F5/F6；SUPER 组合不进撤销、不进命令行 |
//! | ⑤ 全键盘音符操控 | `shift_arrows_resize_note_duration_through_the_shared_recipe` | `Shift+←/→` = 一个吸附网格，`SetNoteTiming` 保身份、1 tick 下限 |

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_app::elements::ElementKind;
use yeban_app::input::View;
use yeban_model::ids::EntityId;
use yeban_model::project::ClipContent;
use yeban_tui::app::{App, ListView};
use yeban_tui::keymap::Pane;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn last_log(app: &App) -> String {
    app.log.last().cloned().unwrap_or_default()
}

fn temp_path(tag: &str, ext: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("yeban-tui-p7-{tag}-{}.{ext}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// 演示工程里 MIDI 池条目的身份。
fn midi_clip_id(app: &App) -> EntityId {
    app.project
        .clip_pool
        .values()
        .find(|entry| matches!(entry.content, ClipContent::Midi { .. }))
        .map(|entry| entry.id)
        .expect("演示工程应有 MIDI 片段")
}

/// 选中放有指定池条目的那条轨，焦点切到片段网格，光标移到摆放起点小节。
fn focus_clip(app: &mut App, clip_id: EntityId) -> (EntityId, EntityId) {
    let mut found = None;
    for (index, track_id) in app.track_ids.iter().enumerate() {
        let track = &app.project.tracks[track_id];
        if let Some(placement) = track.clips.values().find(|p| p.clip_id == clip_id) {
            found = Some((index, *track_id, placement.id, placement.start_tick));
            break;
        }
    }
    let (index, track_id, placement_id, start) = found.expect("应有引用该条目的摆放");
    app.selected = index;
    if app.focus != Pane::Grid {
        app.handle_key(key(KeyCode::Tab));
    }
    app.cursor_bar = usize::try_from(start / app.bar_ticks()).unwrap_or(0);
    (track_id, placement_id)
}

#[test]
fn info_shows_the_clock_and_entity_ids_from_the_model() {
    let mut app = App::with_demo().expect("演示工程");
    let project_id = app.project.id.to_canonical_string();
    let track_id = app
        .selected_track_id()
        .expect("演示工程有选中轨")
        .to_canonical_string();

    app.execute_command("info");
    let text = app
        .log
        .iter()
        .rev()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("960 PPQ"), "时钟必须点名 PPQ：{text}");
    assert!(
        text.contains("每拍 960 tick") && text.contains("每小节 3840 tick"),
        "tick 换算必须可核（4/4）：{text}"
    );
    assert!(text.contains("schema 1"), "契约版本必须可见：{text}");
    assert!(
        text.contains("minReader 1"),
        "最低读取器版本必须可见：{text}"
    );
    assert!(
        text.contains("check_readable 通过"),
        "可读性必须可核：{text}"
    );
    assert!(
        text.contains(&project_id),
        "工程身份（EntityId）必须可见：{project_id} 不在 {text}"
    );
    assert!(
        text.contains(&track_id),
        "选中轨身份必须可见：{track_id} 不在 {text}"
    );
    // 模型常量是唯一事实源。
    assert_eq!(app.project_info().ppq, yeban_model::ids::PPQ);
    assert_eq!(app.project_info().ticks_per_beat, 960);
    assert_eq!(app.project_info().bar_ticks, 3840);
    // `:clock` 是同一落点（别名；四行里第三行是时钟行）。
    let before = app.log.len();
    app.execute_command("clock");
    let alias = app.log[before..].join("\n");
    assert!(alias.contains("960 PPQ"), "别名词必须同样可达：{alias}");
}

#[test]
fn contract_roundtrips_and_rejects_a_newer_schema_loudly() {
    let mut app = App::with_demo().expect("演示工程");
    let path = temp_path("contract", "yeban");
    app.save_to(&path).expect("保存");
    let reopened = App::open_file(&path).expect("重开");
    assert_eq!(reopened.project.schema_version, app.project.schema_version);
    assert_eq!(
        reopened.project.min_reader_version,
        app.project.min_reader_version
    );
    assert_eq!(reopened.project.writer_version, app.project.writer_version);
    assert_eq!(reopened.project.id, app.project.id, "身份必须保真");
    assert!(reopened.project.check_readable().is_ok());

    // schema 过新 ⇒ 打开必须响亮拒绝（同一道 check_readable 门）。
    let mut archive = yeban_app::open::open_project_archive_file(
        &path,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("读归档");
    archive.project.schema_version = yeban_model::project::READER_SCHEMA_VERSION + 1;
    let bumped = temp_path("contract-newer", "yeban");
    yeban_app::save::save_archive_file(&archive, &bumped).expect("写出过新 schema 的归档");
    let error = match App::open_file(&bumped) {
        Ok(_) => panic!("schema 过新必须拒绝"),
        Err(error) => error,
    };
    assert!(
        error.contains("schema_version") && error.contains("打开失败"),
        "拒绝文案必须带契约原因：{error}"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&bumped);
}

#[test]
fn semantic_rows_use_gui_roles_and_match_the_visible_labels() {
    let mut app = App::with_demo().expect("演示工程");
    let rows = app.semantic_rows();
    let known = [
        ElementKind::Main,
        ElementKind::List,
        ElementKind::ListItem,
        ElementKind::Region,
        ElementKind::GroupBox,
        ElementKind::Text,
        ElementKind::ContentInfo,
    ]
    .map(ElementKind::accessible_role);
    for row in &rows {
        assert!(
            known.contains(&row.role),
            "角色必须来自 GUI 词表：{} / {}",
            row.role,
            row.id
        );
        assert!(!row.label.is_empty(), "标签不得为空：{}", row.id);
    }
    assert_eq!(rows[0].role, "main", "首行必须是主工作区地标");

    // 轨道行的序连续、标签与轨道名同源、选中标记落在选中轨上。
    let track_rows: Vec<_> = rows.iter().filter(|row| row.role == "list-item").collect();
    assert_eq!(track_rows.len(), app.track_ids.len());
    for (index, row) in track_rows.iter().enumerate() {
        assert_eq!(row.index, index, "序必须连续：{}", row.id);
        let name = &app.project.tracks[&app.track_ids[index]].name;
        assert!(row.label.contains(name), "标签必须含轨道名：{}", row.label);
    }
    assert!(
        track_rows[app.selected].label.contains("（选中）"),
        "选中标记必须在选中轨上"
    );

    // 检查器文本与界面同一份读数（含身份）。
    let id_text = app.project.tracks[&app.track_ids[app.selected]]
        .id
        .to_canonical_string();
    assert!(
        rows.iter()
            .any(|row| row.role == "text" && row.label.contains(&id_text)),
        "检查器语义必须含轨身份"
    );
    // `:a11y` 导出与内存里的同一份清单。
    app.execute_command("a11y");
    let exported = app
        .log
        .iter()
        .rev()
        .take(rows.len())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(exported.len(), rows.len());
    assert!(
        exported.iter().any(|line| line.starts_with("role=main ")),
        "导出必须带角色：{exported:?}"
    );
}

#[test]
fn ime_and_super_combos_never_reach_shortcuts_or_text_buffers() {
    let mut app = App::with_demo().expect("演示工程");
    let track_id = app.track_ids[0];
    assert!(!app.project.tracks[&track_id].mute);

    // 合成态：裸 `m`（静音）必须被吞掉；F5（全局视图键）仍然放行。
    app.set_ime_composing(true);
    app.handle_key(key(KeyCode::Char('m')));
    assert!(
        !app.project.tracks[&track_id].mute,
        "合成态下的 m 不得改工程"
    );
    app.handle_key(key(KeyCode::F(5)));
    assert_eq!(app.view(), View::Session, "F5 仍然直达");
    app.set_ime_composing(false);
    app.handle_key(key(KeyCode::Char('m')));
    assert!(
        app.project.tracks[&track_id].mute,
        "退出合成态后 m 恢复原语义"
    );

    // SUPER 组合（macOS Cmd / Win 键）：既不做撤销，也不当裸键。
    app.handle_key(key_with(KeyCode::Char('z'), KeyModifiers::SUPER));
    assert!(
        app.project.tracks[&track_id].mute,
        "Cmd+Z 不得被当成撤销（撤销会回退静音）"
    );

    // 命令行缓冲：Ctrl/Alt/SUPER 的字符组合一个都不进缓冲。
    app.handle_key(key(KeyCode::Char(':')));
    assert_eq!(app.mode, yeban_tui::keymap::Mode::Command);
    for event in [
        key_with(KeyCode::Char('v'), KeyModifiers::CONTROL),
        key_with(KeyCode::Char('z'), KeyModifiers::ALT),
        key_with(KeyCode::Char('v'), KeyModifiers::SUPER),
    ] {
        app.handle_key(event);
    }
    assert!(
        app.cmdline.is_empty(),
        "修饰键组合不得进命令行：{:?}",
        app.cmdline
    );
    app.handle_key(key(KeyCode::Char('i')));
    assert_eq!(app.cmdline, "i", "裸字符照常输入");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode, yeban_tui::keymap::Mode::Normal);
}

#[test]
fn shift_arrows_resize_note_duration_through_the_shared_recipe() {
    let mut app = App::with_demo().expect("演示工程");
    let clip_id = midi_clip_id(&app);
    focus_clip(&mut app, clip_id);
    app.handle_key(key(KeyCode::Char('n')));
    assert!(matches!(app.list_view, Some(ListView::Notes { .. })));

    let before = app.view_notes()[0].clone();
    let note_id = before.id;
    let grid = i64::try_from(yeban_app::host::ROLL_SNAP_GRID_TICKS).expect("网格常量");

    // Shift+→：时值 +一个吸附网格（与无修饰 ←/→ 的平移同一个常量）。
    app.handle_key(key_with(KeyCode::Right, KeyModifiers::SHIFT));
    let after = app
        .view_notes()
        .into_iter()
        .find(|note| note.id == note_id)
        .expect("音符仍在");
    assert_eq!(
        after.duration_ticks,
        before.duration_ticks + u64::try_from(grid).expect("正值"),
        "Shift+→ 应加一个吸附网格"
    );
    assert_eq!(after.start_tick, before.start_tick, "时值编辑不得改起点");

    // Shift+← 连按：夹在 1 tick 下限，不越界、不 panic。
    for _ in 0..40 {
        app.handle_key(key_with(KeyCode::Left, KeyModifiers::SHIFT));
    }
    let floor = app
        .view_notes()
        .into_iter()
        .find(|note| note.id == note_id)
        .expect("音符仍在");
    assert_eq!(floor.duration_ticks, 1, "时值下限是 1 tick");
    // 已在谷底再按：响亮说明、不写空 op。
    app.handle_key(key_with(KeyCode::Left, KeyModifiers::SHIFT));
    assert!(
        last_log(&app).contains("1 tick 下限"),
        "谷底必须响亮说明：{}",
        last_log(&app)
    );

    // 阴影：视图外按 Shift+←/→ 不得改任何音符。
    app.handle_key(key(KeyCode::Esc));
    assert!(app.list_view.is_none());
    let snapshot = app.project.clip_pool[&clip_id].clone();
    app.handle_key(key_with(KeyCode::Right, KeyModifiers::SHIFT));
    assert_eq!(
        app.project.clip_pool[&clip_id], snapshot,
        "视图外不得改音符"
    );
    assert!(
        last_log(&app).contains("音符视图内"),
        "视图外必须如实说明落点：{}",
        last_log(&app)
    );
}
