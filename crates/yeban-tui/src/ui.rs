//! 呈现层：ratatui 平铺布局（顶栏 / 左轨道 / 中片段网格 / 右检查器 / 底栏）。
//!
//! 本模块只读 `App` 状态画图（外加网格横向窗口跟随播放头/光标这一处视图态调整），
//! 不改工程、不碰引擎。颜色全部来自 [`crate::theme`]（GUI tokens 取样）。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine, Points};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use yeban_app::input::View;
use yeban_engine::transport::TransportState;
use yeban_model::project::{SceneV3, TrackV3};

use crate::app::{App, ListView, track_kind_label};
use crate::keymap::{Mode, Pane};
use crate::theme;

/// 画一帧。
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let root = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(4),
    ])
    .split(frame.area());
    draw_top_bar(frame, root[0], app);
    let body = Layout::horizontal([
        Constraint::Length(30),
        Constraint::Min(24),
        Constraint::Length(32),
    ])
    .split(root[1]);
    draw_tracks(frame, body[0], app);
    // 主视图切换（T-FIX-76 第 3 项）：Session 画场景 × 轨道矩阵，
    // Arrangement 画线性片段网格 —— 同一枚举 `yeban_app::input::View`。
    match app.view {
        View::Arrangement => draw_grid(frame, body[1], app),
        View::Session => draw_session(frame, body[1], app),
    }
    draw_inspector(frame, body[2], app);
    draw_bottom(frame, root[2], app);
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let color = if focused { theme::GOLD } else { theme::LINE };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(theme::BG_PANEL))
        .title(title.to_owned())
}

fn draw_top_bar(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let reading = app.transport();
    let (bar, beat, tick) = app.position_bar_beat();
    let (glyph, glyph_color) = match reading.state {
        TransportState::Playing => ("▶", theme::PLAYING),
        TransportState::Recording => ("●", theme::RECORD_RED),
        TransportState::Stopped => ("■", theme::INK1),
    };
    let mode_badge = match app.mode {
        Mode::Normal => (" NORMAL ", theme::GOLD_DEEP),
        Mode::Insert => (" INSERT ", theme::RECORD_RED),
        Mode::Command => (" COMMAND ", theme::GOLD),
    };
    let mut spans = vec![
        Span::styled(
            mode_badge.0,
            Style::default().fg(theme::BG_VOID).bg(mode_badge.1),
        ),
        Span::styled(
            format!(" {} ", app.project.title),
            Style::default()
                .fg(theme::INK0)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{glyph} "), Style::default().fg(glyph_color)),
        Span::styled(
            format!("{bar:03}.{beat}.{tick:03} / {} 小节", app.total_bars()),
            Style::default().fg(theme::GOLD_BRIGHT),
        ),
        Span::styled(
            format!(
                "  {:.0} BPM  {}/{}  {} PPQ",
                app.project.bpm,
                app.project.time_signature.numerator,
                app.project.time_signature.denominator,
                yeban_model::ids::PPQ
            ),
            Style::default().fg(theme::INK1),
        ),
    ];
    spans.push(Span::styled(
        format!("  [{}]", view_name(app.view)),
        Style::default().fg(theme::GOLD_DEEP),
    ));
    if app.dirty {
        spans.push(Span::styled(
            "  ● 未保存",
            Style::default().fg(theme::RECORD_RED),
        ));
    }
    if let Some(path) = &app.project_path {
        spans.push(Span::styled(
            format!("  {}", path.display()),
            Style::default().fg(theme::INK2),
        ));
    }
    let bar_widget = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(theme::BG_PANEL).fg(theme::INK0));
    frame.render_widget(bar_widget, area);
}

/// 主视图短名（顶栏徽标）。
fn view_name(view: View) -> &'static str {
    match view {
        View::Session => "SESSION",
        View::Arrangement => "ARRANGE",
    }
}

/// 按显示宽度补齐（中文名按 2 列计）。
fn pad_to(text: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(text);
    if current >= width {
        return text.to_owned();
    }
    format!("{}{}", text, " ".repeat(width - current))
}

/// 截断到显示宽度内（不切半个宽字符）。
fn truncate_to(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0_usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

fn draw_tracks(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.focus == Pane::Tracks;
    let mut items: Vec<ListItem<'_>> = Vec::with_capacity(app.track_ids.len() + 1);
    items.push(ListItem::new(Line::from(Span::styled(
        "  名称            静 独  音量",
        Style::default().fg(theme::INK2),
    ))));
    for (index, id) in app.track_ids.iter().enumerate() {
        let Some(track) = app.project.tracks.get(id) else {
            continue;
        };
        let selected = index == app.selected;
        let cursor = if selected { "▸" } else { " " };
        let mute = if track.mute { "M" } else { "·" };
        let solo = if track.solo { "S" } else { "·" };
        // 通道电平（T-FIX-81 第 2 项）：同一份 SPSC 链抽干出的逐轨峰值，
        // 4 格条 + 保持衰减（`App::tick` 每跳衰减一次，与主总线同一个系数）。
        let meter = meter_bar(app.track_peak(id));
        let name = pad_to(&truncate_to(&track.name, 10), 10);
        let text = format!(
            "{cursor}{name} {mute}{solo} {meter} {:+5.1}",
            track.volume_db
        );
        let mut style = Style::default()
            .fg(if selected { theme::INK0 } else { theme::INK1 })
            .bg(if index % 2 == 0 {
                theme::BG_LANE
            } else {
                theme::BG_LANE2
            });
        if selected {
            style = style.bg(theme::BG_RAISED).add_modifier(Modifier::BOLD);
        }
        // 轨道高度（T-FIX-76 第 2 项）：一行轨占 `track_rows` 个终端行；
        // 续行只画一条纵向连接符，⛔ 不重复读数（避免"看起来像多了一条轨"）。
        let rows = app.track_rows(id);
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(usize::from(rows));
        lines.push(Line::from(Span::styled(text, style)));
        for _ in 1..rows {
            lines.push(Line::from(Span::styled(
                "  │",
                Style::default()
                    .fg(theme::LINE_STRONG)
                    .bg(style.bg.unwrap_or(theme::BG_LANE)),
            )));
        }
        items.push(ListItem::new(lines));
    }
    let list = List::new(items).block(pane_block("轨道", focused));
    let mut state = ListState::default();
    state.select(Some(app.selected + 1));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_grid(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let focused = app.focus == Pane::Grid;
    // 三期：列表视图（音符 / 片段池）是本窗格的内容变体，标题随视图变。
    let title = match app.list_view {
        Some(ListView::Notes { .. }) => "片段 · 音符",
        Some(ListView::Pool { .. }) => "片段 · 池",
        Some(ListView::Automation { .. }) => "片段 · 自动化",
        None => "片段",
    };
    let block = pane_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    match app.list_view {
        Some(ListView::Notes { .. }) => {
            draw_notes_list(frame, inner, app);
            return;
        }
        Some(ListView::Pool { .. }) => {
            draw_pool_list(frame, inner, app);
            return;
        }
        Some(ListView::Automation { .. }) => {
            draw_automation_list(frame, inner, app);
            return;
        }
        None => {}
    }
    let bar_cols = app.bar_cols.max(1);
    let visible = usize::from(inner.width) / bar_cols;
    if visible == 0 {
        return;
    }
    let total = app.total_bars();
    let play_bar = app.playhead_bar_index();
    // 横向窗口跟随：播放时跟播放头，平时跟光标。
    let anchor = if app.is_playing() {
        play_bar
    } else {
        app.cursor_bar
    };
    if anchor < app.bar_offset {
        app.bar_offset = anchor;
    } else if anchor >= app.bar_offset + visible {
        app.bar_offset = anchor + 1 - visible;
    }
    let offset = app.bar_offset.min(total.saturating_sub(1));
    let bar_ticks = app.bar_ticks().max(1);

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(app.track_ids.len() + 1);
    // 标尺行：小节号 + 播放头 ▼ + 光标下划线。
    let mut ruler: Vec<Span<'_>> = Vec::new();
    for i in 0..visible {
        let bar = offset + i;
        if bar >= total {
            ruler.push(Span::raw(" ".repeat(bar_cols)));
            continue;
        }
        let label = if bar == play_bar {
            format!("▼{}", bar + 1)
        } else {
            format!("{}", bar + 1)
        };
        let cell = format!(
            "{}{}",
            label,
            " ".repeat(bar_cols.saturating_sub(label.chars().count()))
        );
        let mut style = Style::default().fg(theme::INK2);
        if bar == play_bar {
            style = Style::default()
                .fg(theme::GOLD_BRIGHT)
                .add_modifier(Modifier::BOLD);
        }
        if bar == app.cursor_bar {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        ruler.push(Span::styled(cell, style));
    }
    lines.push(Line::from(ruler));

    for (index, id) in app.track_ids.iter().enumerate() {
        let Some(track) = app.project.tracks.get(id) else {
            continue;
        };
        let color = theme::track_color(track.color.as_deref(), index);
        let row_bg = if index % 2 == 0 {
            theme::BG_LANE
        } else {
            theme::BG_LANE2
        };
        let mut spans: Vec<Span<'_>> = Vec::new();
        for i in 0..visible {
            let bar = offset + i;
            if bar >= total {
                spans.push(Span::styled(
                    " ".repeat(bar_cols),
                    Style::default().bg(row_bg),
                ));
                continue;
            }
            let bar_start = u64::try_from(bar).unwrap_or(0) * bar_ticks;
            let bar_end = bar_start + bar_ticks;
            let clip = track.clips.values().find(|clip| {
                clip.start_tick < bar_end
                    && clip.start_tick.saturating_add(clip.duration_ticks) > bar_start
            });
            let mut style = Style::default().bg(row_bg);
            if bar == play_bar {
                style = style.add_modifier(Modifier::BOLD);
            }
            if bar == app.cursor_bar {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            let (cell, cell_style) = match clip {
                Some(clip) if clip.muted => ("░".repeat(bar_cols), style.fg(theme::INK2)),
                Some(_) => ("▓".repeat(bar_cols), style.fg(color)),
                None => (
                    format!("·{}", " ".repeat(bar_cols.saturating_sub(1))),
                    style.fg(theme::INK2),
                ),
            };
            spans.push(Span::styled(cell, cell_style));
        }
        // 轨道高度（T-FIX-76 第 2 项）：一条轨按 `track_rows` 个终端行重复画，
        // 于是"整体放大一档"在网格里也看得见（同一份有效像素读数）。
        for _ in 0..app.track_rows(id) {
            lines.push(Line::from(spans.clone()));
        }
    }
    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme::BG_PANEL));
    frame.render_widget(paragraph, inner);
}

/// MIDI 音高 → IPN 音名（C4 = 60，与工程内部记法同一标准；仅显示用）。
fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let name = NAMES[usize::from(pitch % 12)];
    let octave = i32::from(pitch / 12) - 1;
    format!("{name}{octave}")
}

/// 音符列表视图（三期）：对象片段的音符按 (起点, 音高) 排序逐行列出，
/// 位置是片段内相对 tick 换算的小节.拍+余数。
fn draw_notes_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(ListView::Notes {
        clip_id, anchor, ..
    }) = app.list_view
    else {
        return;
    };
    let clip_name = app
        .project
        .clip_pool
        .get(&clip_id)
        .map(|entry| entry.name.clone())
        .unwrap_or_default();
    let notes = app.view_notes();
    let bar_ticks = app.bar_ticks().max(1);
    let beat_ticks = app.ticks_per_beat().max(1);
    let mut items: Vec<ListItem<'_>> = Vec::with_capacity(notes.len() + 1);
    items.push(ListItem::new(Line::from(Span::styled(
        format!(
            "  「{clip_name}」共 {} 个音符 · 选中 {}",
            notes.len(),
            app.note_selection.len()
        ),
        Style::default().fg(theme::INK2),
    ))));
    for note in &notes {
        let is_selected = app.note_selection.contains(&note.id);
        let cursor = if is_selected { "▸" } else { " " };
        let bar = note.start_tick / bar_ticks;
        let beat = (note.start_tick % bar_ticks) / beat_ticks;
        let rem = note.start_tick % beat_ticks;
        let text = format!(
            "{cursor}{}.{}+{rem:03}  {:>4}({:>3})  时值 {:>5}  力度 {:>3}",
            bar + 1,
            beat + 1,
            pitch_name(note.pitch),
            note.pitch,
            note.duration_ticks,
            note.velocity,
        );
        let mut style = Style::default().fg(if is_selected {
            theme::INK0
        } else {
            theme::INK1
        });
        if is_selected {
            style = style.bg(theme::BG_RAISED).add_modifier(Modifier::BOLD);
        }
        items.push(ListItem::new(Line::from(Span::styled(text, style))));
    }
    if notes.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  （无音符：按 a 在光标处加一个）",
            Style::default().fg(theme::INK2),
        ))));
    }
    let list = List::new(items);
    let mut state = ListState::default();
    let row = if notes.is_empty() {
        0
    } else {
        anchor.min(notes.len().saturating_sub(1)) + 1
    };
    state.select(Some(row));
    frame.render_stateful_widget(list, area, &mut state);
}

/// 片段池列表视图（三期）：每行一个池条目（名 / 内容 / 引用数）。
/// 引用数 > 0 的条目删除会被模型拒绝（`ClipInUse`），行内先把数亮出来。
fn draw_pool_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(ListView::Pool { selected }) = app.list_view else {
        return;
    };
    let rows = app.pool_rows();
    let mut items: Vec<ListItem<'_>> = Vec::with_capacity(rows.len() + 1);
    items.push(ListItem::new(Line::from(Span::styled(
        format!("  共 {} 个片段池条目", rows.len()),
        Style::default().fg(theme::INK2),
    ))));
    for (index, (entry, refs)) in rows.iter().enumerate() {
        let is_selected = index == selected;
        let cursor = if is_selected { "▸" } else { " " };
        let kind = match &entry.content {
            yeban_model::project::ClipContent::Midi { notes } => {
                format!("MIDI {} 音符", notes.len())
            }
            yeban_model::project::ClipContent::Audio { .. } => "音频".to_owned(),
        };
        let name = pad_to(&truncate_to(&entry.name, 24), 24);
        let text = format!("{cursor}{name} {kind}  引用 {refs} 处");
        let mut style = Style::default().fg(if is_selected {
            theme::INK0
        } else {
            theme::INK1
        });
        if is_selected {
            style = style.bg(theme::BG_RAISED).add_modifier(Modifier::BOLD);
        }
        items.push(ListItem::new(Line::from(Span::styled(text, style))));
    }
    let list = List::new(items);
    let mut state = ListState::default();
    state.select(Some(selected + 1));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Session 视图（T-FIX-76 第 3 项）：场景（行）× 轨道（列）矩阵。
///
/// 数据只取工程：场景来自 `project.scenes`（与 GUI 投影同序 —— `BTreeMap` 身份
/// 升序，见 `bridge::project_view`），轨迹来自 `display_track_ids()`（主总线不进
/// 矩阵，与移轨 / 显示顺序同一口径）。插槽内容在模型里**没有**表达
/// （GUI `session_view.slint` 同样只画空槽），因此这里不编造"哪个槽装了片段"。
fn draw_session(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.focus == Pane::Grid;
    let block = pane_block("Session", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = usize::from(inner.width);
    let name_w = 20_usize.min(width.saturating_sub(6)).max(6);
    let cell_w = 5_usize;
    let capacity = (width.saturating_sub(name_w) / cell_w).max(1);
    let tracks: Vec<&TrackV3> = app
        .project
        .display_track_ids()
        .iter()
        .filter_map(|id| app.project.tracks.get(id))
        .collect();
    let shown = capacity.min(tracks.len());

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut head = pad_to("  场景", name_w);
    for track in tracks.iter().take(shown) {
        head.push_str(&pad_to(&truncate_to(&track.name, cell_w), cell_w));
    }
    if tracks.len() > shown {
        head.push_str(&format!(" +{} 轨", tracks.len() - shown));
    }
    lines.push(Line::from(Span::styled(
        head,
        Style::default().fg(theme::INK2),
    )));

    let scenes: Vec<&SceneV3> = app.project.scenes.values().collect();
    if scenes.is_empty() {
        lines.push(Line::from(Span::styled(
            "  （本工程没有场景：Session 矩阵为空）",
            Style::default().fg(theme::INK2),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }
    let selected = app.scene_selected.min(scenes.len().saturating_sub(1));
    let visible = usize::from(inner.height).saturating_sub(2).max(1);
    let offset = if selected >= visible {
        selected + 1 - visible
    } else {
        0
    };
    for (index, scene) in scenes.iter().enumerate().skip(offset).take(visible) {
        let cursor = if index == selected { "▸" } else { " " };
        let mut row = format!(
            "{cursor}{}",
            pad_to(
                &truncate_to(&scene.name, name_w.saturating_sub(4)),
                name_w.saturating_sub(2)
            )
        );
        for _ in 0..shown {
            row.push_str(&pad_to("·", cell_w));
        }
        if let Some(tempo) = scene.tempo {
            row.push_str(&format!(" {tempo:.0} BPM"));
        }
        let mut style = Style::default().fg(if index == selected {
            theme::INK0
        } else {
            theme::INK1
        });
        if index == selected {
            style = style.bg(theme::BG_RAISED).add_modifier(Modifier::BOLD);
        }
        lines.push(Line::from(Span::styled(row, style)));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::BG_PANEL)),
        inner,
    );
}

/// tick → 小节.拍+余数（1 起，与顶栏读数同一口径）。
fn tick_label(app: &App, tick: u64) -> String {
    let bar_ticks = app.bar_ticks().max(1);
    let beat_ticks = app.ticks_per_beat().max(1);
    format!(
        "{}.{}+{:03}",
        tick / bar_ticks + 1,
        (tick % bar_ticks) / beat_ticks + 1,
        tick % beat_ticks
    )
}

/// 自动化泳道视图（T-FIX-76 第 4 项）：光标读数 + 采样点列表。
///
/// 读数走 `App::automation_readout`（`YebanProjectV1::automation_value_at`
/// 是唯一求值入口）；列表顺序是模型的 `points_in_tick_order`（`(tick, 身份)`），
/// 这里不重新排序。曲线名用模型枚举的 `Debug` 名（`Linear` / `SCurve` …），
/// 与 `yeban_edit_automation` 回执里的 `curve` 同名。
fn draw_automation_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(ListView::Automation {
        target, selected, ..
    }) = app.list_view
    else {
        return;
    };
    // 泳道名与 GUI 同一口径（设备参数显示参数名、宏显示宏名）。
    let kind = app.automation_lane_name();
    let unit = target.nominal_unit().symbol();
    let domain = app.automation_domain().map_or_else(
        || "值域未知".to_owned(),
        |(min, max)| format!("[{min}, {max}]"),
    );
    let points = app.automation_points();
    let mut items: Vec<ListItem<'_>> = Vec::with_capacity(points.len() + 2);
    items.push(ListItem::new(Line::from(Span::styled(
        format!("  {kind} · 单位 {unit} · {domain} · {} 个点", points.len()),
        Style::default().fg(theme::INK2),
    ))));
    if let Some(readout) = app.automation_readout() {
        let value = readout
            .automation_value
            .map_or_else(|| "—".to_owned(), |value| format!("{value:+.2}"));
        items.push(ListItem::new(Line::from(Span::styled(
            format!(
                "  光标 {} = {value} {unit}（{} · 静态 {:+.2}）",
                tick_label(app, readout.tick),
                readout.source,
                readout.static_value
            ),
            Style::default().fg(theme::GOLD_DEEP),
        ))));
    }
    for (index, point) in points.iter().enumerate() {
        let is_selected = index == selected;
        let cursor = if is_selected { "▸" } else { " " };
        let text = format!(
            "{cursor}{}  {:+8.2} {unit}  {:?}",
            tick_label(app, point.tick),
            point.value,
            point.curve
        );
        let mut style = Style::default().fg(if is_selected {
            theme::INK0
        } else {
            theme::INK1
        });
        if is_selected {
            style = style.bg(theme::BG_RAISED).add_modifier(Modifier::BOLD);
        }
        items.push(ListItem::new(Line::from(Span::styled(text, style))));
    }
    if points.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  （本泳道还没有点：a 在光标处按当前值设一个）",
            Style::default().fg(theme::INK2),
        ))));
    }
    let list = List::new(items);
    let mut state = ListState::default();
    state.select(Some(selected.min(points.len()) + 2));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_inspector(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.focus == Pane::Inspector;
    let block = pane_block("检查器", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(id) = app.selected_track_id() else {
        return;
    };
    let Some(track) = app.project.tracks.get(&id) else {
        return;
    };
    let name_display = match (&app.mode, &app.rename) {
        (Mode::Insert, Some(buffer)) => format!("{buffer}▌"),
        _ => track.name.clone(),
    };
    let instrument = instrument_label(track);
    let (height_base, height_effective) = app.track_height_px(&id);
    let height_rows = app.track_rows(&id);
    let volume_fraction = ((track.volume_db - yeban_app::bridge::FADER_MIN_DB)
        / (yeban_app::bridge::FADER_MAX_DB - yeban_app::bridge::FADER_MIN_DB))
        .clamp(0.0, 1.0);
    let volume_bar = text_bar(volume_fraction, 16);
    let pan_fraction = (track.pan + 1.0) / 2.0;
    let pan_bar = text_bar(pan_fraction, 16);
    let dim = Style::default().fg(theme::INK1);
    let bright = Style::default().fg(theme::INK0);
    let lines = vec![
        Line::from(vec![
            Span::styled("名称 ", dim),
            Span::styled(name_display, Style::default().fg(theme::GOLD_BRIGHT)),
        ]),
        Line::from(vec![
            Span::styled("身份 ", dim),
            Span::styled(
                truncate_to(&track.id.to_canonical_string(), 26),
                Style::default().fg(theme::INK2),
            ),
        ]),
        Line::from(vec![
            Span::styled("类型 ", dim),
            Span::styled(track_kind_label(track.kind), bright),
        ]),
        Line::from(vec![
            Span::styled("音量 ", dim),
            Span::styled(format!("{:+.1} dB", track.volume_db), bright),
        ]),
        Line::from(Span::styled(volume_bar, Style::default().fg(theme::GOLD))),
        Line::from(vec![
            Span::styled("声相 ", dim),
            Span::styled(format!("{:+.1}", track.pan), bright),
        ]),
        Line::from(Span::styled(pan_bar, Style::default().fg(theme::GOLD_DEEP))),
        Line::from(vec![
            Span::styled("乐器 ", dim),
            Span::styled(truncate_to(&instrument, 22), bright),
        ]),
        Line::from(vec![
            Span::styled("片段 ", dim),
            Span::styled(format!("{} 个", track.clips.len()), bright),
        ]),
        Line::from(vec![
            Span::styled("高度 ", dim),
            Span::styled(
                format!("{height_base} px → {height_effective} px · {height_rows} 行"),
                bright,
            ),
        ]),
        Line::from(vec![
            Span::styled("状态 ", dim),
            Span::styled(
                format!(
                    "{} {}",
                    if track.mute { "静音" } else { "发声" },
                    if track.solo { "· 独奏" } else { "" }
                ),
                bright,
            ),
        ]),
        Line::from(Span::styled(
            "Ctrl+Alt+↑/↓ 音量 · ←/→ 声相",
            Style::default().fg(theme::INK2),
        )),
        Line::from(Span::styled(
            "i 改名 · M 静音 · S 独奏",
            Style::default().fg(theme::INK2),
        )),
        Line::from(Span::styled(
            "J/K 移轨 · [/] 高度 · p/P 自动化",
            Style::default().fg(theme::INK2),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// 检查器「乐器」一行：首个设备名 → 采样器文件名 → 占位。
fn instrument_label(track: &TrackV3) -> String {
    if let Some(device) = track.devices.first() {
        return device.name.clone();
    }
    if let Some(sampler) = &track.sampler {
        let path = &sampler.path;
        let file = path.rsplit(['/', '\\']).next().unwrap_or(path.as_str());
        return file.to_owned();
    }
    "—".to_owned()
}

/// 文本电平条：`█████─────`。
fn text_bar(fraction: f32, width: usize) -> String {
    let filled = (fraction.clamp(0.0, 1.0) * width as f32).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled.min(width)),
        "─".repeat(width - filled.min(width))
    )
}

/// 4 格通道电平条：线性峰值 → dB → 与推子同一值域的比例。
///
/// 值域用 GUI 的 `FADER_MIN_DB` / `FADER_MAX_DB`（同一个常量），⛔ 不另造刻度。
fn meter_bar(peak: f32) -> String {
    let fraction = if peak <= 0.0 {
        0.0
    } else {
        let db = 20.0 * peak.log10();
        ((db - yeban_app::bridge::FADER_MIN_DB)
            / (yeban_app::bridge::FADER_MAX_DB - yeban_app::bridge::FADER_MIN_DB))
            .clamp(0.0, 1.0)
    };
    let filled = (fraction * 4.0).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled.min(4)),
        "·".repeat(4 - filled.min(4))
    )
}

/// 引擎状态行：采样率 / 每块帧数 / 通道 / 设备腿（名 + 设备率）/ 转换 / 走带。
fn engine_line(app: &App) -> String {
    let status = app.engine_status();
    let engine = match (
        status.engine_sample_rate,
        status.block_frames,
        status.channels,
    ) {
        (Some(rate), Some(frames), Some(channels)) => {
            format!(
                "引擎 {:.1} kHz / {frames} 帧 / {channels} ch",
                f64::from(rate) / 1000.0
            )
        }
        _ => "引擎 未建".to_owned(),
    };
    let device = match (&status.device_name, status.device_active) {
        (Some(name), true) => match status.device_sample_rate {
            Some(rate) => format!("设备 {name} @ {:.1} kHz", f64::from(rate) / 1000.0),
            None => format!("设备 {name}"),
        },
        _ => "设备 未开".to_owned(),
    };
    let transport = match status.transport_state {
        TransportState::Playing => "播放中",
        TransportState::Recording => "录音中",
        TransportState::Stopped => "停止",
    };
    format!(
        "工程 {:.1} kHz · {engine} · {device} · {} · 走带 {transport} @ {} tick",
        f64::from(status.project_sample_rate) / 1000.0,
        status.conversion,
        status.position_ticks,
    )
}

fn draw_bottom(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    // 第一行：引擎状态（`yeban_query_engine_state` 的 TUI 面）。
    frame.render_widget(
        Paragraph::new(engine_line(app))
            .style(Style::default().fg(theme::INK1).bg(theme::BG_PANEL)),
        rows[0],
    );
    // 第二行：Canvas 盲文库点阵 —— 位置进度 + 主总线电平。
    let total = app.project_len_ticks().max(1) as f64;
    let position = app.transport().position_ticks as f64;
    let peak = app.master_peak.clamp(0.0, 1.0);
    let canvas = Canvas::default()
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([0.0, total])
        .y_bounds([0.0, 4.0])
        .paint(move |ctx| {
            // 轨道线（全长，y=1）。
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: 1.0,
                x2: total,
                y2: 1.0,
                color: theme::LINE_STRONG,
            });
            // 已播部分（金）。
            if position > 0.0 {
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 1.0,
                    x2: position,
                    y2: 1.0,
                    color: theme::GOLD_BRIGHT,
                });
            }
            // 播放头竖线。
            ctx.draw(&CanvasLine {
                x1: position,
                y1: 0.0,
                x2: position,
                y2: 4.0,
                color: theme::GOLD_BRIGHT,
            });
            // 主总线电平（右端竖条，高度 ∝ 峰值）。
            if peak > 0.01 {
                let x = total * 0.995;
                let mut points = Vec::new();
                let steps = (peak * 40.0) as i32;
                for i in 0..=steps {
                    points.push((x, f64::from(i) * 0.1));
                }
                ctx.draw(&Points {
                    coords: &points,
                    color: theme::PLAYING,
                });
            }
        });
    frame.render_widget(canvas, rows[1]);
    // 第二行：日志（最后一条）。
    let log_text = app.log.last().cloned().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(log_text).style(Style::default().fg(theme::INK1).bg(theme::BG_PANEL)),
        rows[2],
    );
    // 第四行：命令行 / 状态行。
    let status = match app.mode {
        Mode::Command => Line::from(vec![
            Span::styled(":", Style::default().fg(theme::GOLD_BRIGHT)),
            Span::styled(app.cmdline.clone(), Style::default().fg(theme::INK0)),
            Span::styled("▌", Style::default().fg(theme::GOLD_BRIGHT)),
        ]),
        Mode::Insert => Line::from(Span::styled(
            format!(
                "-- INSERT 改名中：{}▌ · Ctrl+U 清空 · Enter 提交 · Esc 取消 --",
                app.rename.as_deref().unwrap_or_default()
            ),
            Style::default().fg(theme::RECORD_RED),
        )),
        Mode::Normal => {
            let count = app
                .vi_count()
                .map(|count| format!(" 计数 {count}"))
                .unwrap_or_default();
            let text = match app.list_view {
                Some(ListView::Notes { .. }) => format!(
                    "-- 音符视图 -- j/k 选 · Ctrl+A 全选 · h/l 移时 · J/K 移高 · a 加 · Del 删选区 · <> 时值 · -/+ 力度 · Esc 返回{count}"
                ),
                Some(ListView::Pool { .. }) => format!(
                    "-- 片段池 -- j/k 选 · Del 删条目（被引用会被模型拒绝） · Esc 返回{count}"
                ),
                Some(ListView::Automation { .. }) => format!(
                    "-- 自动化 -- j/k 选点（光标跟随） · a 设点 · -/+ 改值 · Del 删点 · Esc 返回{count}"
                ),
                None if app.view == View::Session => {
                    format!("-- SESSION -- j/k 选场景 · F6 回编曲 · Space 播放 · : 命令{count}")
                }
                None => format!(
                    "-- NORMAL -- 焦点 {} ·{}{} · Space 播放 · M 静音 · S 独奏 · : 命令 · i 改名 · n 音符",
                    app.focus.label(),
                    app.selected_track_name().unwrap_or_default(),
                    count
                ),
            };
            Line::from(Span::styled(text, Style::default().fg(theme::INK2)))
        }
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(theme::BG_PANEL)),
        rows[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                out.push_str(symbol);
                x += if UnicodeWidthStr::width(symbol) >= 2 {
                    2
                } else {
                    1
                };
            }
            out.push('\n');
        }
        out
    }

    fn render_buffer(app: &mut App) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn meter_heartbeat_only_touches_meter_cells() {
        let mut app = App::with_demo().expect("demo app");
        app.execute_command("seek 1");
        // 先让走带进入播放并把电平推到有读数（两帧的顶栏因此一致）。
        app.toggle_play();
        app.drive_meters_for_test(4);
        let before = render_buffer(&mut app);
        app.drive_meters_for_test(8);
        let after = render_buffer(&mut app);

        let mut changed = Vec::new();
        for y in 0..before.area.height {
            for x in 0..before.area.width {
                if before[(x, y)].symbol() != after[(x, y)].symbol() {
                    changed.push((x, y));
                }
            }
        }
        assert!(!changed.is_empty(), "电平跳动必须真的画出来（不是死表）");
        for (x, y) in changed {
            // 允许变的三处：轨道栏（电平列）、底栏（引擎行/电平点阵/日志/状态）、
            // 以及顶栏（走带时间码 —— 它在播放时本来就该跳）。
            let in_track_meter = x < 30;
            let in_bottom = y >= before.area.height - 4;
            let in_top_bar = y == 0;
            assert!(
                in_track_meter || in_bottom || in_top_bar,
                "心跳只能碰电平区，但 ({x},{y}) 变了 —— 说明这一跳重画了别处"
            );
        }
    }

    #[test]
    fn session_view_renders_scene_matrix_instead_of_the_ruler() {
        let mut app = App::with_demo().expect("demo app");
        app.set_view(View::Session);
        let text = render_text(&mut app);
        assert!(
            text.contains("Session"),
            "中间窗格标题应为 Session：\n{text}"
        );
        assert!(text.contains("SESSION"), "顶栏徽标应显示主视图：\n{text}");
        assert!(
            text.contains("Intro") && text.contains("Chorus"),
            "场景名来自工程：\n{text}"
        );
        assert!(
            !text.contains("▼1"),
            "Session 视图不画编曲标尺（视图真的是另一套渲染）：\n{text}"
        );
    }

    #[test]
    fn automation_view_lists_points_with_value_and_curve() {
        let mut app = App::with_demo().expect("demo app");
        // 演示工程第 1 轨（鼓）有音量泳道：-3.2 / -8 / -1。
        app.selected = 0;
        app.open_automation_view(crate::app::AutomationKind::Volume);
        let text = render_text(&mut app);
        assert!(text.contains("片段 · 自动化"), "标题应切到自动化：\n{text}");
        assert!(text.contains("音量"), "泳道名应出现：\n{text}");
        assert!(text.contains("-3.20"), "点值应出现：\n{text}");
        assert!(text.contains("Linear"), "曲线名应出现：\n{text}");
    }

    #[test]
    fn track_height_readout_reaches_the_inspector() {
        let mut app = App::with_demo().expect("demo app");
        app.selected = 0;
        app.set_track_height_px(112);
        let text = render_text(&mut app);
        assert!(
            text.contains("112 px → 112 px · 2 行"),
            "检查器应显示基准 → 有效 → 行数：\n{text}"
        );
    }

    #[test]
    fn ui_top_bar_and_ruler_align_on_one_based_bars() {
        let mut app = App::with_demo().expect("demo app");
        // 墙钟泵送使初始 tick 余数不保证为 0；先 :seek 1 钉位再断言精确读数。
        app.execute_command("seek 1");
        let text0 = render_text(&mut app);
        assert!(
            text0.contains("001.1.000 / 3 小节"),
            "顶栏初始读数应为 1 起：\n{text0}"
        );
        assert!(
            text0.contains("▼1"),
            "标尺初始播放头应在第 1 小节：\n{text0}"
        );
        assert!(
            !text0.contains("▼2"),
            "初始时 ▼ 不应出现在第 2 小节：\n{text0}"
        );

        app.execute_command("seek 2");
        app.tick();
        let text1 = render_text(&mut app);
        assert!(
            text1.contains("002.1.000 / 3 小节"),
            ":seek 2 后顶栏应显示 002.1.000：\n{text1}"
        );
        assert!(
            text1.contains("▼2"),
            ":seek 2 后标尺播放头应在 ▼2：\n{text1}"
        );
        assert!(
            !text1.contains("▼1"),
            ":seek 2 后 ▼ 不应留在第 1 小节：\n{text1}"
        );
    }
}
