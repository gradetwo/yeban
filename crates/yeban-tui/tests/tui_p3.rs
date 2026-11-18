//! yeban-tui 三期集成测试：片段切分 / 复制剪切粘贴 / 片段改名 /
//! 音符视图编辑 / 片段池视图删条目。贯穿夹具 = `yeban_model::samples::demo_project()`
//! （池内 2 条目：MIDI「夜色铺底」6 音符、音频「909 鼓组」；摆放：首轨 MIDI
//! [0,3840)、二轨音频 [1920,2880)、三轨音频 [5760,9600)；120 BPM 4/4，
//! 一小节 3840 tick、一拍 960 tick）。
//! 每笔编辑都经 `UndoPort`，测试逐项断言「改 → 撤销回原状」。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use yeban_model::ids::EntityId;
use yeban_model::project::ClipContent;
use yeban_tui::app::{App, ListView};
use yeban_tui::keymap::Pane;
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

fn ctrl(ch: char) -> KeyEvent {
    key_with(KeyCode::Char(ch), KeyModifiers::CONTROL)
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

/// 选中放有指定池条目的那条轨，并把焦点切到片段网格、光标移到摆放起点小节。
/// 返回 (轨道, 摆放)。
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
    assert_eq!(app.focus, Pane::Grid);
    app.cursor_bar = usize::try_from(start / app.bar_ticks()).unwrap_or(0);
    (track_id, placement_id)
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            out.push_str(symbol);
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

/// 切分（音频片段，`Ctrl+T`）：播放头落在摆放内部时一分为二，
/// 右半是新池条目；撤销后还原。切分点不在片段内时如实拒绝、工程不动。
#[test]
fn split_clip_at_playhead_and_refusal() {
    let mut app = App::with_demo().expect("demo app");
    // 三轨音频摆放 [5760, 9600)：找它所在轨。
    let audio_id = app
        .project
        .clip_pool
        .values()
        .find(|entry| matches!(entry.content, ClipContent::Audio { .. }))
        .map(|entry| entry.id)
        .expect("音频条目");
    // 该条目有两处摆放；focus_clip 拿到第一处（二轨 [1920,2880)）。
    // 本测试要的是三轨那处 [5760,9600)：按起点定位。
    let mut target = None;
    for (index, track_id) in app.track_ids.iter().enumerate() {
        let track = &app.project.tracks[track_id];
        if let Some(placement) = track
            .clips
            .values()
            .find(|p| p.clip_id == audio_id && p.start_tick == 5760)
        {
            target = Some((index, *track_id, placement.id));
            break;
        }
    }
    let (index, track_id, placement_id) = target.expect("三轨音频摆放");
    app.selected = index;
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Grid);
    let pool_before = app.project.clip_pool.len();

    // 拒绝臂：播放头在 0，不在 [5760,9600) 内（光标先移到该摆放）。
    app.cursor_bar = usize::try_from(5760 / app.bar_ticks()).unwrap_or(0);
    app.handle_key(ctrl('t'));
    assert_eq!(
        app.project.tracks[&track_id].clips.len(),
        1,
        "切分点不在片段内应拒绝"
    );
    assert_eq!(app.project.clip_pool.len(), pool_before);
    assert!(
        app.log
            .last()
            .is_some_and(|line| line.contains("切分点不在片段内部")),
        "拒绝应如实记日志：{:?}",
        app.log.last()
    );

    // 成功臂：:seek 2 把播放头与光标同步移到 7680（严格落在摆放内）。
    app.execute_command("seek 2");
    app.tick();
    assert_eq!(app.transport().position_ticks, 2 * app.bar_ticks());
    app.handle_key(ctrl('t'));
    let track = &app.project.tracks[&track_id];
    assert_eq!(track.clips.len(), 2, "切分后应有两处摆放");
    let mut starts: Vec<(u64, u64)> = track
        .clips
        .values()
        .map(|p| (p.start_tick, p.duration_ticks))
        .collect();
    starts.sort_unstable();
    assert_eq!(
        starts,
        vec![(5760, 1920), (7680, 1920)],
        "两半应恰好铺满原摆放"
    );
    assert_eq!(
        app.project.clip_pool.len(),
        pool_before + 1,
        "右半应是新池条目"
    );
    assert!(
        app.project
            .clip_pool
            .values()
            .any(|entry| entry.name.ends_with("(Part 2)")),
        "右半条目名应带 (Part 2) 后缀"
    );
    let _ = placement_id;

    // 撤销：回到一处摆放、池大小复原。
    app.handle_key(char_key('u'));
    assert_eq!(app.project.tracks[&track_id].clips.len(), 1);
    assert_eq!(app.project.clip_pool.len(), pool_before);
}

/// 切分组合（MIDI 片段）直测共享构造点 `clip_split_ops_for`：
/// 跨点后的音符移交右半并重新对齐，op 组合一步可撤销的形状齐全。
#[test]
fn split_ops_for_midi_clip_hands_notes_to_right_half() {
    let app = App::with_demo().expect("demo app");
    let clip_id = midi_clip_id(&app);
    let placement_id = app
        .project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .find(|p| p.clip_id == clip_id)
        .map(|p| p.id)
        .expect("MIDI 摆放");
    // 在 1920 处切：音符起点 0/480/960/1440 留左，1920/2400 移右。
    let (ops, _right_id) =
        yeban_app::host::clip_split_ops_for(&app.project, &placement_id.to_string(), 1920)
            .expect("切分点在片段内应有组合");
    use yeban_model::ops::Op;
    let deletes = ops
        .iter()
        .filter(|op| matches!(op, Op::DeleteNote { .. }))
        .count();
    assert_eq!(deletes, 2, "两颗音符应移交右半");
    let add_clip = ops.iter().find_map(|op| match op {
        Op::AddClip { clip } => Some(clip),
        _ => None,
    });
    let right = add_clip.expect("组合应含 AddClip");
    let ClipContent::Midi { notes } = &right.content else {
        panic!("右半应是 MIDI 内容");
    };
    let mut right_starts: Vec<u64> = notes.values().map(|n| n.start_tick).collect();
    right_starts.sort_unstable();
    assert_eq!(right_starts, vec![0, 480], "右半音符应从 0 重新对齐");
    assert!(
        ops.iter()
            .any(|op| matches!(op, Op::AddClipPlacement { .. })),
        "组合应含 AddClipPlacement"
    );
    // 退化切分：切分点在边界上回 None。
    assert!(
        yeban_app::host::clip_split_ops_for(&app.project, &placement_id.to_string(), 0).is_none()
    );
    assert!(
        yeban_app::host::clip_split_ops_for(&app.project, &placement_id.to_string(), 3840)
            .is_none()
    );
}

/// 复制 / 粘贴 / 剪切：剪贴板快照 → 粘贴出新身份副本（音符身份全换新）；
/// 剪切 = 复制 + 删摆放在同一步撤销内。
#[test]
fn copy_paste_cut_clipboard() {
    let mut app = App::with_demo().expect("demo app");
    let clip_id = midi_clip_id(&app);
    let (track_id, _placement_id) = focus_clip(&mut app, clip_id);
    let source_notes: Vec<EntityId> = match &app.project.clip_pool[&clip_id].content {
        ClipContent::Midi { notes } => notes.keys().copied().collect(),
        ClipContent::Audio { .. } => unreachable!(),
    };
    let pool_before = app.project.clip_pool.len();

    // 复制：工程不动，剪贴板有货。
    app.handle_key(ctrl('c'));
    assert!(app.clipboard.is_some(), "Ctrl+C 后剪贴板应有快照");
    assert_eq!(app.project.clip_pool.len(), pool_before);

    // 粘贴到另一条轨的光标处（第 3 小节）。
    let other_index = app
        .track_ids
        .iter()
        .position(|id| *id != track_id && *id != app.project.master_bus_track_id)
        .expect("另一条内容轨");
    app.selected = other_index;
    let other_track = app.track_ids[other_index];
    let other_clips_before = app.project.tracks[&other_track].clips.len();
    app.cursor_bar = 3;
    app.handle_key(ctrl('v'));
    assert_eq!(
        app.project.clip_pool.len(),
        pool_before + 1,
        "粘贴应新增池条目"
    );
    let other = &app.project.tracks[&other_track];
    assert_eq!(other.clips.len(), other_clips_before + 1);
    let pasted = other
        .clips
        .values()
        .find(|p| p.start_tick == 3 * app.bar_ticks())
        .expect("粘贴摆放应落在光标处");
    assert_ne!(pasted.clip_id, clip_id, "粘贴片段应是新身份");
    let ClipContent::Midi { notes } = &app.project.clip_pool[&pasted.clip_id].content else {
        panic!("粘贴条目应是 MIDI 内容");
    };
    assert_eq!(notes.len(), source_notes.len(), "音符数应与源一致");
    for note in notes.values() {
        assert!(!source_notes.contains(&note.id), "粘贴音符身份必须全换新");
    }
    // 撤销粘贴。
    app.handle_key(char_key('u'));
    assert_eq!(app.project.clip_pool.len(), pool_before);
    assert_eq!(
        app.project.tracks[&other_track].clips.len(),
        other_clips_before
    );

    // 剪切：摆放消失、剪贴板仍有快照、池条目留下；撤销一次全回来。
    app.selected = app
        .track_ids
        .iter()
        .position(|id| *id == track_id)
        .expect("源轨下标");
    app.cursor_bar = 0;
    app.handle_key(ctrl('x'));
    assert!(
        app.project.tracks[&track_id].clips.is_empty(),
        "剪切后源轨摆放应消失"
    );
    assert!(app.clipboard.is_some(), "剪切后剪贴板应有快照");
    assert_eq!(
        app.project.clip_pool.len(),
        pool_before,
        "剪切只删摆放，池条目留下"
    );
    app.handle_key(char_key('u'));
    assert_eq!(
        app.project.tracks[&track_id].clips.len(),
        1,
        "撤销应恢复摆放"
    );
}

/// 片段改名：片段网格焦点下 `i` 改的是池条目名（`Op::SetClipName`），
/// 轨道名不动；撤销可回。
#[test]
fn rename_clip_in_grid_focus() {
    let mut app = App::with_demo().expect("demo app");
    let clip_id = midi_clip_id(&app);
    let (track_id, _placement_id) = focus_clip(&mut app, clip_id);
    let old_name = app.project.clip_pool[&clip_id].name.clone();
    let track_name_before = app.project.tracks[&track_id].name.clone();

    app.handle_key(char_key('i'));
    app.handle_key(key_with(KeyCode::Char('X'), KeyModifiers::SHIFT));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        app.project.clip_pool[&clip_id].name,
        format!("{old_name}X"),
        "片段名应被改"
    );
    assert_eq!(
        app.project.tracks[&track_id].name, track_name_before,
        "轨道名不应被动"
    );
    app.handle_key(char_key('u'));
    assert_eq!(
        app.project.clip_pool[&clip_id].name, old_name,
        "撤销应恢复片段名"
    );
}

/// 音符视图：开视图 → 移时（h/l 与方向键）→ 移高（J/K 与方向键）→
/// 加音符 → 改时值 → 改力度 → 删音符 → 逐笔撤销回原状 → Esc 关视图。
#[test]
fn notes_view_edit_cycle() {
    let mut app = App::with_demo().expect("demo app");
    let clip_id = midi_clip_id(&app);
    let (_track_id, _placement_id) = focus_clip(&mut app, clip_id);

    app.handle_key(char_key('n'));
    assert!(
        matches!(app.list_view, Some(ListView::Notes { .. })),
        "n 应打开音符视图"
    );
    // 渲染：标题与音名出现。
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(&terminal);
    assert!(text.contains("片段 · 音符"), "视图标题缺失：\n{text}");
    assert!(text.contains("C4"), "音名 C4 缺失：\n{text}");

    let initial: Vec<(EntityId, u64, u8, u64, u8)> = app
        .view_notes()
        .iter()
        .map(|n| (n.id, n.start_tick, n.pitch, n.duration_ticks, n.velocity))
        .collect();
    assert_eq!(initial.len(), 6, "演示 MIDI 片段应有 6 个音符");
    let first = initial[0];
    assert_eq!((first.1, first.2), (0, 60), "首音符应是 0 tick / 音高 60");

    // l：右移一格（240 tick）。
    app.handle_key(char_key('l'));
    assert_eq!(app.view_notes()[0].start_tick, 240);
    // →：GUI 微移 Grid 档，再 +240。
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.view_notes()[0].start_tick, 480);
    // J：音高 +1。
    app.handle_key(key_with(KeyCode::Char('J'), KeyModifiers::SHIFT));
    assert_eq!(app.view_notes()[0].pitch, 61, "J 应升高半音");
    // ↑：GUI 微移 Semitone 档，再 +1。
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.view_notes()[0].pitch, 62, "↑ 应升高半音");

    // a：在光标处（第 0 小节 = 片段内 0 tick）加音符，音高随选中音符。
    app.handle_key(char_key('a'));
    let notes = app.view_notes();
    assert_eq!(notes.len(), 7, "a 应加一个音符");
    let added = notes
        .iter()
        .find(|n| !initial.iter().any(|(id, ..)| *id == n.id))
        .expect("新音符");
    assert_eq!(added.start_tick, 0);
    assert_eq!(added.pitch, 62, "新音符音高应随选中音符");
    assert_eq!(
        added.duration_ticks,
        app.ticks_per_beat(),
        "新音符时值应为一拍"
    );
    assert_eq!(added.velocity, 100);

    // >：时值 +一拍；-：力度 -10。
    app.handle_key(key_with(KeyCode::Char('>'), KeyModifiers::SHIFT));
    let added_now = app
        .view_notes()
        .into_iter()
        .find(|n| n.id == added.id)
        .expect("新音符仍在");
    assert_eq!(added_now.duration_ticks, 2 * app.ticks_per_beat());
    app.handle_key(char_key('-'));
    let added_now = app
        .view_notes()
        .into_iter()
        .find(|n| n.id == added.id)
        .expect("新音符仍在");
    assert_eq!(added_now.velocity, 90);

    // Delete：删选中音符（加音符后选中已回填到新音符）。
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(app.view_notes().len(), 6, "Delete 应删掉选中音符");
    assert!(
        app.view_notes().iter().all(|n| n.id != added.id),
        "被删的应是新音符"
    );

    // 逐笔撤销（共 8 笔：移时×2、移高×2、加、时值、力度、删）回原状。
    for _ in 0..8 {
        app.handle_key(char_key('u'));
    }
    let restored: Vec<(EntityId, u64, u8, u64, u8)> = app
        .view_notes()
        .iter()
        .map(|n| (n.id, n.start_tick, n.pitch, n.duration_ticks, n.velocity))
        .collect();
    assert_eq!(restored, initial, "8 笔撤销后音符应逐项复原");

    // Esc 关视图。
    app.handle_key(key(KeyCode::Esc));
    assert!(app.list_view.is_none(), "Esc 应关掉音符视图");
}

/// 片段池视图：被引用的条目删除被模型拒绝（工程不动、日志如实）；
/// 无引用的条目可删、可撤销恢复。
#[test]
fn pool_view_delete_entry_and_in_use_refusal() {
    let mut app = App::with_demo().expect("demo app");
    let clip_id = midi_clip_id(&app);
    let pool_before = app.project.clip_pool.len();

    app.execute_command("pool");
    assert!(matches!(app.list_view, Some(ListView::Pool { .. })));
    // 渲染：池视图标题出现。
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    assert!(buffer_text(&terminal).contains("片段 · 池"));

    // 拒绝臂：两个条目都有引用（MIDI 1 处、音频 2 处），删任一都被拒。
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(
        app.project.clip_pool.len(),
        pool_before,
        "被引用的条目不应被删"
    );
    assert!(
        app.log.last().is_some_and(|line| line.contains("被拒绝")),
        "拒绝应如实记日志：{:?}",
        app.log.last()
    );
    app.handle_key(key(KeyCode::Esc));
    assert!(app.list_view.is_none());

    // 造无引用条目：剪切 MIDI 摆放（池条目留下、引用归零）。
    let (_track_id, _placement_id) = focus_clip(&mut app, clip_id);
    app.handle_key(ctrl('x'));
    let rows = app.pool_rows();
    let (row_index, refs) = rows
        .iter()
        .enumerate()
        .find(|(_, (entry, _))| entry.id == clip_id)
        .map(|(index, (_, refs))| (index, *refs))
        .expect("MIDI 条目应在池中");
    assert_eq!(refs, 0, "剪切后该条目应无引用");

    app.execute_command("pool");
    app.list_view = Some(ListView::Pool {
        selected: row_index,
    });
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(
        app.project.clip_pool.len(),
        pool_before - 1,
        "无引用条目应被删掉"
    );
    assert!(!app.project.clip_pool.contains_key(&clip_id));
    // 撤销：条目回来（摆放仍是剪切掉的状态，那是另一笔）。
    app.handle_key(char_key('u'));
    assert_eq!(app.project.clip_pool.len(), pool_before);
    assert!(app.project.clip_pool.contains_key(&clip_id));
}

/// `App::with_project`：以任意工程起会话（测试/嵌入入口）。
#[test]
fn with_project_opens_filled_fixture() {
    let app = App::with_project(yeban_model::samples::filled_project()).expect("filled app");
    assert!(!app.track_ids.is_empty(), "filled 工程应有轨道");
    assert!(!app.project.clip_pool.is_empty(), "filled 工程应有片段池");
}
