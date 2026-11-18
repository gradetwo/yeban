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

use yeban_engine::transport::TransportState;
use yeban_model::project::TrackV3;

use crate::app::{App, track_kind_label};
use crate::keymap::{Mode, Pane};
use crate::theme;

/// 画一帧。
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let root = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(3),
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
    draw_grid(frame, body[1], app);
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
                "  {:.0} BPM  {}/{}",
                app.project.bpm,
                app.project.time_signature.numerator,
                app.project.time_signature.denominator
            ),
            Style::default().fg(theme::INK1),
        ),
    ];
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
        let name = pad_to(&truncate_to(&track.name, 14), 14);
        let text = format!("{cursor}{name} {mute}  {solo}  {:+5.1}", track.volume_db);
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
        items.push(ListItem::new(Line::from(Span::styled(text, style))));
    }
    let list = List::new(items).block(pane_block("轨道", focused));
    let mut state = ListState::default();
    state.select(Some(app.selected + 1));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_grid(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let focused = app.focus == Pane::Grid;
    let block = pane_block("片段", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let bar_cols = app.bar_cols.max(1);
    let visible = usize::from(inner.width) / bar_cols;
    if visible == 0 {
        return;
    }
    let total = app.total_bars();
    let (play_bar, _, _) = app.position_bar_beat();
    let play_bar = usize::try_from(play_bar).unwrap_or(0);
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
        lines.push(Line::from(spans));
    }
    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme::BG_PANEL));
    frame.render_widget(paragraph, inner);
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

fn draw_bottom(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    // 第一行：Canvas 盲文库点阵 —— 位置进度 + 主总线电平。
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
    frame.render_widget(canvas, rows[0]);
    // 第二行：日志（最后一条）。
    let log_text = app.log.last().cloned().unwrap_or_default();
    frame.render_widget(
        Paragraph::new(log_text).style(Style::default().fg(theme::INK1).bg(theme::BG_PANEL)),
        rows[1],
    );
    // 第三行：命令行 / 状态行。
    let status = match app.mode {
        Mode::Command => Line::from(vec![
            Span::styled(":", Style::default().fg(theme::GOLD_BRIGHT)),
            Span::styled(app.cmdline.clone(), Style::default().fg(theme::INK0)),
            Span::styled("▌", Style::default().fg(theme::GOLD_BRIGHT)),
        ]),
        Mode::Insert => Line::from(Span::styled(
            "-- INSERT 改名中：Enter 提交 · Esc 取消 --",
            Style::default().fg(theme::RECORD_RED),
        )),
        Mode::Normal => {
            let count = app
                .vi_count()
                .map(|count| format!(" 计数 {count}"))
                .unwrap_or_default();
            Line::from(Span::styled(
                format!(
                    "-- NORMAL -- 焦点 {} ·{}{} · Space 播放 · M 静音 · S 独奏 · : 命令 · i 改名",
                    app.focus.label(),
                    app.selected_track_name().unwrap_or_default(),
                    count
                ),
                Style::default().fg(theme::INK2),
            ))
        }
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().bg(theme::BG_PANEL)),
        rows[2],
    );
}
