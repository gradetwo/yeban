//! 整曲总览图的单文件自包含 HTML 报告（渲染器，**唯一** HTML 实现）。
//!
//! 输入是 [`SongOverview`]（`yeban-dsp` 的列式总览，见 [`crate::song_overview`]）。
//! 本模块**只渲染、不新增分析、不改任何数值**：
//!
//! - 数值按设计稿 §8.2 的量化口径显示（归一化指标 2 位小数，响度与频段能量 1 位小数），
//!   与 JSON 总览的取整口径一致；
//! - 哨兵值原样显示（LUFS −70、频段 −120 dB、期望带缺失时显示 `—`），并在表下注明含义，
//!   ⛔ 不把哨兵值伪装成测量值；
//! - 输出是确定性的：无时间戳、无随机 id，同一份输入恒产出同一份 HTML ——
//!   MCP 的 `format: "html"` 与 CLI 的 `--export-song-overview-html` 才能逐字节一致。
//!
//! 形态（借 `director-desk` 联系表的“单文件自包含 HTML”，不抄业务内容）：
//! 内联 CSS + 内联 SVG，零外部资源、零 CDN、不写 `<script>`（不要任何 JS），可打印。
//! 来自数据的**一切**字符串（段落名、调号、曲风档、异常描述等）进 HTML 前都过
//! [`escape_html`]；转义表与联系表的 `contactEscape` 同款（`& < > " '`）。
//!
//! 调用方：MCP 的 `get_song_overview`（`format: "html"`）与 CLI 的
//! `--export-song-overview-html` 都调这里 —— 两边共享行→列转置后调
//! [`render_html_report_with_title`]，⛔ 不各写一份 HTML 生成代码（转置住在 `yeban-mcp` 的
//! `domain::song_overview`，渲染只住在这里）。标题取工程的 `title`
//! （`YebanProjectV1.title`，顶层唯一权威）；无标题时调 [`render_html_report`]，
//! 它委托给带标题入口并用通用标题。

use crate::song_overview::{BAND_DB_FLOOR, NO_ENVELOPE, NO_LOUDNESS_READING_LUFS, SongOverview};

/// HTML 转义（与联系表 `contactEscape` 同款）：`& < > " '` → 实体。
///
/// 顺序重要：`&` 必须最先转，否则后转出的实体里的 `&` 会被二次转义。
#[must_use]
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// 2 位小数（归一化指标，§8.2 口径）；非有限值显示 `—`（不编造数字）。
fn fmt2(value: f32) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value:.2}")
}

/// 1 位小数（响度 LUFS 与频段能量 dB，§8.2 口径）；非有限值显示 `—`。
fn fmt1(value: f32) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value:.1}")
}

/// 期望带端点：哨兵 [`NO_ENVELOPE`]（无等级）显示 `—`，否则 2 位小数。
fn fmt_band_edge(value: f32) -> String {
    if value == NO_ENVELOPE {
        "—".to_owned()
    } else {
        fmt2(value)
    }
}

/// 内联 CSS（单文件自包含；打印时每节尽量不跨页断开）。
const STYLE: &str = "body{margin:24px;background:#fff;color:#171717;font:14px/1.5 system-ui,sans-serif}h1{font-size:24px;margin:0 0 4px}h2{font-size:18px;margin:24px 0 8px}h3{font-size:14px;margin:16px 0 4px}p{margin:6px 0}table{border-collapse:collapse;width:100%;font-size:12px}th,td{border:1px solid #ccc;padding:4px 6px;text-align:right}th:first-child,td:first-child{text-align:left}th{background:#f0f0f0}section{break-inside:avoid}svg{display:block;max-width:100%;height:auto;background:#fafafa;border:1px solid #ddd}.legend{font-size:12px;color:#444}.nodata{color:#666;font-style:italic}.meta{color:#444}.note{color:#444;font-size:12px}footer{margin-top:24px;border-top:1px solid #ccc;padding-top:8px;color:#666;font-size:12px}@media print{body{margin:0}section{break-inside:avoid}}@page{size:A4;margin:12mm}";

/// 一条折线（`svg_chart` 的输入）：图例名 + 每小节一个值 + 颜色。
struct ChartLine<'a> {
    label: &'a str,
    values: &'a [f32],
    color: &'a str,
    dashed: bool,
}

/// 折线图（内联 SVG）：`lines` 各画一条折线，`extra` 是画在折线**之下**的附加 SVG
/// 片段（如期望带的多边形），`unit` 是纵轴单位（画在纵轴上下两端）。
///
/// 空序列 / 全空序列 ⇒ 一段“无数据”文字，不画空图。纵轴范围由调用方给
/// （各层自己的量纲）；范围退化（上下相等）时向两边各扩 0.5 防除零。
/// 非有限值断开折线（该点不画、不连线），⛔ 不把坏值画成 0。
fn svg_chart(lines: &[ChartLine<'_>], min: f32, max: f32, unit: &str, extra: &str) -> String {
    let mut point_count = 0usize;
    for line in lines {
        point_count += line.values.len();
    }
    if point_count == 0 {
        return "<p class=\"nodata\">无数据</p>".to_owned();
    }
    let (mut lo, mut hi) = (min, max);
    if !lo.is_finite() || !hi.is_finite() || (hi - lo).abs() < 1e-6 {
        let mid = if lo.is_finite() && hi.is_finite() {
            (lo + hi) / 2.0
        } else {
            0.0
        };
        lo = mid - 0.5;
        hi = mid + 0.5;
    }
    const WIDTH: f32 = 720.0;
    const HEIGHT: f32 = 150.0;
    const PAD_LEFT: f32 = 52.0;
    const PAD_RIGHT: f32 = 8.0;
    const PAD_TOP: f32 = 8.0;
    const PAD_BOTTOM: f32 = 20.0;
    let plot_w = WIDTH - PAD_LEFT - PAD_RIGHT;
    let plot_h = HEIGHT - PAD_TOP - PAD_BOTTOM;
    let x_of = |index: usize, count: usize| -> f32 {
        if count <= 1 {
            PAD_LEFT + plot_w / 2.0
        } else {
            PAD_LEFT + plot_w * index as f32 / (count - 1) as f32
        }
    };
    let y_of = |value: f32| -> f32 { PAD_TOP + plot_h * (1.0 - (value - lo) / (hi - lo)) };
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg viewBox=\"0 0 {WIDTH:.0} {HEIGHT:.0}\" role=\"img\" aria-label=\"折线图\">"
    ));
    svg.push_str(extra);
    // 纵轴上下限 + 单位。
    let y_top = PAD_TOP + 4.0;
    let y_bottom = PAD_TOP + plot_h;
    let y_unit = HEIGHT - 4.0;
    let hi_label = fmt_axis(hi);
    let lo_label = fmt_axis(lo);
    svg.push_str(&format!(
        "<text x=\"48\" y=\"{y_top}\" text-anchor=\"end\" font-size=\"11\">{hi_label}</text>"
    ));
    svg.push_str(&format!(
        "<text x=\"48\" y=\"{y_bottom}\" text-anchor=\"end\" font-size=\"11\">{lo_label}</text>"
    ));
    svg.push_str(&format!(
        "<text x=\"48\" y=\"{y_unit}\" text-anchor=\"end\" font-size=\"11\">{}</text>",
        escape_html(unit)
    ));
    for line in lines {
        // 连续有限值成段画；断点处另起一段，单点画圆。
        let mut segment: Vec<(f32, f32)> = Vec::new();
        let flush = |segment: &mut Vec<(f32, f32)>, svg: &mut String| {
            if segment.len() == 1 {
                let (x, y) = segment[0];
                let color = line.color;
                svg.push_str(&format!(
                    "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3\" fill=\"{color}\"/>"
                ));
            } else if segment.len() > 1 {
                let points: Vec<String> = segment
                    .iter()
                    .map(|(x, y)| format!("{x:.1},{y:.1}"))
                    .collect();
                let points = points.join(" ");
                let color = line.color;
                let dash = if line.dashed {
                    " stroke-dasharray=\"6 4\""
                } else {
                    ""
                };
                svg.push_str(&format!(
                    "<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"{dash}/>"
                ));
            }
            segment.clear();
        };
        for (index, &value) in line.values.iter().enumerate() {
            if value.is_finite() {
                segment.push((x_of(index, line.values.len()), y_of(value)));
            } else {
                flush(&mut segment, &mut svg);
            }
        }
        flush(&mut segment, &mut svg);
    }
    svg.push_str("</svg>");
    // 图例（颜色块 + 名）。
    let mut legend = String::from("<p class=\"legend\">");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            legend.push('　');
        }
        let color = line.color;
        let label = escape_html(line.label);
        legend.push_str(&format!("<span style=\"color:{color}\">■</span> {label}"));
    }
    legend.push_str("</p>");
    svg.push_str(&legend);
    svg
}

/// 纵轴刻度：整数不带小数位，其余 1 位小数。
fn fmt_axis(value: f32) -> String {
    if value == value.trunc() {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// 表格（表头 + 行；单元格须已转义 / 已格式化）。无行 ⇒ “无数据”文字。
fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return "<p class=\"nodata\">无数据</p>".to_owned();
    }
    let mut out = String::from("<table><thead><tr>");
    for header in headers {
        out.push_str(&format!("<th>{}</th>", escape_html(header)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            out.push_str(&format!("<td>{cell}</td>"));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// 期望带多边形（情绪层折线图之下）：有期望等级的小节连成一段半透明带，
/// 无等级的小节断开（不跨无等级小节连带）。
fn envelope_band_polygons(series: &crate::song_overview::BarSeries) -> String {
    const WIDTH: f32 = 720.0;
    const HEIGHT: f32 = 150.0;
    const PAD_LEFT: f32 = 52.0;
    const PAD_RIGHT: f32 = 8.0;
    const PAD_TOP: f32 = 8.0;
    const PAD_BOTTOM: f32 = 20.0;
    let plot_w = WIDTH - PAD_LEFT - PAD_RIGHT;
    let plot_h = HEIGHT - PAD_TOP - PAD_BOTTOM;
    let count = series.len();
    if count == 0 {
        return String::new();
    }
    let x_of = |index: usize| -> f32 {
        if count <= 1 {
            PAD_LEFT + plot_w / 2.0
        } else {
            PAD_LEFT + plot_w * index as f32 / (count - 1) as f32
        }
    };
    // 张力域恒为 [0, 1]，与 svg_chart 的情绪层纵轴同域。
    let y_of = |value: f32| -> f32 { PAD_TOP + plot_h * (1.0 - value.clamp(0.0, 1.0)) };
    let mut out = String::new();
    let mut run: Vec<usize> = Vec::new();
    let flush = |run: &mut Vec<usize>, out: &mut String| {
        if run.len() < 2 {
            run.clear();
            return;
        }
        let mut points: Vec<String> = Vec::new();
        for &index in run.iter() {
            let x = x_of(index);
            let y_hi = y_of(series.envelope_hi[index]);
            points.push(format!("{x:.1},{y_hi:.1}"));
        }
        for &index in run.iter().rev() {
            let x = x_of(index);
            let y_lo = y_of(series.envelope_lo[index]);
            points.push(format!("{x:.1},{y_lo:.1}"));
        }
        let points = points.join(" ");
        out.push_str(&format!(
            "<polygon points=\"{points}\" fill=\"#8888ff\" fill-opacity=\"0.18\" stroke=\"none\"/>"
        ));
        run.clear();
    };
    for index in 0..count {
        let has_band =
            series.envelope_lo[index] != NO_ENVELOPE && series.envelope_hi[index] != NO_ENVELOPE;
        if has_band {
            run.push(index);
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// 报告无标题时的通用标题（空标题回退到它，不得输出空 `<h1>`）。
pub const DEFAULT_REPORT_TITLE: &str = "整曲总览报告";

/// 把 [`SongOverview`] 渲染成单文件自包含 HTML 报告字符串（通用标题口径）。
///
/// 只渲染、不分析、不改数值；数值显示口径见模块文档。输出确定性：
/// 同一份输入恒产出同一份 HTML（无时间戳、无随机 id）。
#[must_use]
pub fn render_html_report(overview: &SongOverview) -> String {
    render_html_report_with_title(overview, DEFAULT_REPORT_TITLE)
}

/// 带标题的渲染入口：报告头 `<h1>` 与 `<title>` 都用传入的标题（[`escape_html`] 后）；
/// 空标题（或全空白）回退到通用标题 [`DEFAULT_REPORT_TITLE`]，不得输出空 `<h1>`。
#[must_use]
pub fn render_html_report_with_title(overview: &SongOverview, title: &str) -> String {
    let title = if title.trim().is_empty() {
        DEFAULT_REPORT_TITLE
    } else {
        title
    };
    let title = escape_html(title);
    let series = &overview.bar_series;
    let meta = &overview.song_meta;
    let summary = &overview.structural_summary;
    let repetition = &overview.repetition;
    let bars = series.len();

    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    html.push_str(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">",
    );
    html.push_str(&format!(
        "<title>{title} · {} · {}</title><style>{STYLE}</style></head><body>",
        escape_html(&meta.key_signature),
        escape_html(&meta.profile),
    ));

    // ---- 文件头 ----
    html.push_str(&format!("<header><h1>{title}</h1>"));
    let schema = escape_html(overview.schema_version);
    let key = escape_html(&meta.key_signature);
    let profile = escape_html(&meta.profile);
    let total_bars = meta.total_bars;
    let duration = fmt1(meta.duration_sec);
    let bpm = fmt1(meta.base_bpm);
    html.push_str(&format!(
        "<p class=\"meta\">schema {schema} · 调号 {key} · 曲风 {profile} · {total_bars} 小节 · {duration} 秒 · {bpm} BPM</p>",
    ));
    html.push_str(
        "<p class=\"note\">生成说明：本报告由 yeban-dsp 渲染器自整曲总览（SongOverview）直接生成；\
         数值与 JSON 总览一致（归一化指标 2 位小数，响度与频段能量 1 位小数）。\
         单文件自包含，无外部资源，无脚本，可打印。</p></header>",
    );

    // ---- 总览摘要 ----
    html.push_str("<section><h2>总览</h2>");
    let summary_rows = vec![vec![
        summary.climax_bar.to_string(),
        fmt2(summary.climax_tension),
        fmt1(summary.dynamic_range_lu),
        summary.detected_anomalies.len().to_string(),
    ]];
    html.push_str(&table(
        &["高潮小节", "最高张力", "动态范围 (LU)", "异常条数"],
        &summary_rows,
    ));
    html.push_str("</section>");

    // ---- 能量层 ----
    html.push_str("<section><h2>能量层（LUFS）</h2>");
    html.push_str(&svg_chart(
        &[
            ChartLine {
                label: "短时 LUFS",
                values: &series.lufs_short,
                color: "#c0392b",
                dashed: false,
            },
            ChartLine {
                label: "瞬时 LUFS 最大值",
                values: &series.lufs_momentary_max,
                color: "#e67e22",
                dashed: true,
            },
        ],
        NO_LOUDNESS_READING_LUFS,
        0.0,
        "LUFS",
        "",
    ));
    let energy_rows: Vec<Vec<String>> = (0..bars)
        .map(|index| {
            vec![
                series.bar[index].to_string(),
                fmt2(series.time_sec[index]),
                escape_html(&series.section[index]),
                fmt1(series.lufs_short[index]),
                fmt1(series.lufs_momentary_max[index]),
            ]
        })
        .collect();
    html.push_str(&table(
        &["小节", "起始 (秒)", "段落", "短时 LUFS", "瞬时 LUFS 最大值"],
        &energy_rows,
    ));
    html.push_str("<p class=\"note\">−70.0 LUFS 为无有效读数哨兵（窗口未满 / 静音 / 无音频），不是测量值。</p>");
    html.push_str("</section>");

    // ---- 频率层 ----
    html.push_str("<section><h2>频率层（8 频段，dB）</h2>");
    const BAND_COLORS: [&str; 8] = [
        "#1f77b4", "#2ca02c", "#d62728", "#ff7f0e", "#9467bd", "#8c564b", "#e377c2", "#7f7f7f",
    ];
    let band_labels: Vec<String> = overview
        .band_definitions_hz
        .iter()
        .enumerate()
        .map(|(index, [lo, hi])| {
            let band_no = index + 1;
            format!("Band {band_no} ({lo}–{hi} Hz)")
        })
        .collect();
    let band_columns: Vec<Vec<f32>> = (0..8)
        .map(|band| series.bands_tilted_db.iter().map(|row| row[band]).collect())
        .collect();
    let band_lines: Vec<ChartLine<'_>> = (0..8)
        .map(|band| ChartLine {
            label: &band_labels[band],
            values: &band_columns[band],
            color: BAND_COLORS[band],
            dashed: false,
        })
        .collect();
    html.push_str(&svg_chart(&band_lines, BAND_DB_FLOOR, 0.0, "dB", ""));
    let mut freq_headers: Vec<&str> = vec!["小节", "起始 (秒)", "段落"];
    let freq_band_headers: Vec<String> = band_labels.clone();
    let mut freq_header_refs: Vec<&str> = freq_headers.to_vec();
    for header in &freq_band_headers {
        freq_header_refs.push(header.as_str());
    }
    freq_headers = freq_header_refs;
    let freq_rows: Vec<Vec<String>> = (0..bars)
        .map(|index| {
            let mut row = vec![
                series.bar[index].to_string(),
                fmt2(series.time_sec[index]),
                escape_html(&series.section[index]),
            ];
            for band in 0..8 {
                row.push(fmt1(series.bands_tilted_db[index][band]));
            }
            row
        })
        .collect();
    html.push_str(&table(&freq_headers, &freq_rows));
    html.push_str("<p class=\"note\">频段能量为倾斜补偿后能量（+4.5 dB/oct 粉红倾斜补偿）；−120.0 dB 为频段能量下限（该频段无可测能量）。</p>");
    html.push_str("</section>");

    // ---- 疏密层 ----
    html.push_str("<section><h2>疏密层（触发率）</h2>");
    html.push_str(&svg_chart(
        &[
            ChartLine {
                label: "旋律组触发率 (次/拍)",
                values: &series.density_pitched_onsets_per_beat,
                color: "#2e86c1",
                dashed: false,
            },
            ChartLine {
                label: "打击组触发率 (次/拍)",
                values: &series.density_perc_onsets_per_beat,
                color: "#28b463",
                dashed: true,
            },
        ],
        0.0,
        8.0,
        "次/拍",
        "",
    ));
    let density_rows: Vec<Vec<String>> = (0..bars)
        .map(|index| {
            vec![
                series.bar[index].to_string(),
                fmt2(series.time_sec[index]),
                escape_html(&series.section[index]),
                fmt2(series.density_pitched_onsets_per_beat[index]),
                fmt2(series.density_pitched_duty[index]),
                fmt2(series.density_perc_onsets_per_beat[index]),
            ]
        })
        .collect();
    html.push_str(&table(
        &[
            "小节",
            "起始 (秒)",
            "段落",
            "旋律组触发率 (次/拍)",
            "旋律组占空比",
            "打击组触发率 (次/拍)",
        ],
        &density_rows,
    ));
    html.push_str("</section>");

    // ---- 情绪层 ----
    html.push_str("<section><h2>情绪层（张力 / 效价）</h2>");
    let band_polygons = envelope_band_polygons(series);
    html.push_str(&svg_chart(
        &[
            ChartLine {
                label: "张力",
                values: &series.tension,
                color: "#7d3c98",
                dashed: false,
            },
            ChartLine {
                label: "谱面力度",
                values: &series.score_dyn,
                color: "#b7950b",
                dashed: true,
            },
        ],
        0.0,
        1.0,
        "张力",
        &band_polygons,
    ));
    let emotion_rows: Vec<Vec<String>> = (0..bars)
        .map(|index| {
            vec![
                series.bar[index].to_string(),
                fmt2(series.time_sec[index]),
                escape_html(&series.section[index]),
                fmt2(series.tension[index]),
                fmt2(series.valence_hue[index]),
                fmt2(series.score_dyn[index]),
                fmt_band_edge(series.envelope_lo[index]),
                fmt_band_edge(series.envelope_hi[index]),
                fmt2(series.target_deviation[index]),
            ]
        })
        .collect();
    html.push_str(&table(
        &[
            "小节",
            "起始 (秒)",
            "段落",
            "张力",
            "效价色相",
            "谱面力度",
            "期望带下沿",
            "期望带上沿",
            "期望带偏离",
        ],
        &emotion_rows,
    ));
    html.push_str("<p class=\"note\">期望带缺失的小节显示 —（该段落未标注期望张力等级）；偏离超过 0.15 的小节进异常诊断。</p>");
    html.push_str("</section>");

    // ---- 异常诊断 ----
    html.push_str("<section><h2>异常诊断</h2>");
    if summary.detected_anomalies.is_empty() {
        html.push_str("<p>未检出异常。</p>");
    } else {
        let anomaly_rows: Vec<Vec<String>> = summary
            .detected_anomalies
            .iter()
            .map(|anomaly| {
                let (start, end) = (anomaly.bars[0], anomaly.bars[1]);
                vec![
                    format!("{start}–{end}"),
                    escape_html(anomaly.anomaly_type.as_str()),
                    escape_html(&anomaly.description),
                ]
            })
            .collect();
        html.push_str(&table(&["小节区间", "类型", "描述"], &anomaly_rows));
    }
    html.push_str("</section>");

    // ---- 自相似度 ----
    html.push_str("<section><h2>自相似度（repetition）</h2>");
    html.push_str(&format!(
        "<p>机械指数：{}（最佳非邻匹配 ≥ 0.95 的小节占比）</p>",
        fmt2(repetition.mechanical_index)
    ));
    html.push_str("<h3>最相似小节对</h3>");
    if repetition.best_matches.is_empty() {
        html.push_str("<p>无。</p>");
    } else {
        let best_rows: Vec<Vec<String>> = repetition
            .best_matches
            .iter()
            .map(|best| {
                vec![
                    best.bar_a.to_string(),
                    best.bar_b.to_string(),
                    fmt2(best.similarity),
                ]
            })
            .collect();
        html.push_str(&table(&["较早小节", "较晚小节", "余弦相似度"], &best_rows));
    }
    html.push_str("<h3>成段重复</h3>");
    if repetition.repeated_passages.is_empty() {
        html.push_str("<p>无。</p>");
    } else {
        let passage_rows: Vec<Vec<String>> = repetition
            .repeated_passages
            .iter()
            .map(|passage| {
                let (start, end) = (passage.bars[0], passage.bars[1]);
                let (src_start, src_end) = (passage.source_bars[0], passage.source_bars[1]);
                vec![
                    format!("{start}–{end}"),
                    format!("{src_start}–{src_end}"),
                    fmt2(passage.mean_similarity),
                ]
            })
            .collect();
        html.push_str(&table(
            &["重复段小节区间", "源段小节区间", "平均相似度"],
            &passage_rows,
        ));
    }
    html.push_str("</section>");

    // ---- 页脚 ----
    html.push_str(&format!(
        "<footer><p>schema 版本 {} · 整曲总览 HTML 报告（yeban-dsp 渲染器）</p></footer></body></html>",
        escape_html(overview.schema_version),
    ));
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song_overview::{
        Anomaly, AnomalyType, BAND_DEFINITIONS_HZ, BarSeries, BestMatch, FrequencyClashSummary,
        RepeatedPassage, RepetitionSummary, SongMeta, StructuralSummary,
    };

    /// 两小节手算夹具（列式；与 `song_overview_provider` 的映射夹具同源的数值，
    /// 但这里直接写 dsp 列，不读映射代码）。
    /// 段落名与异常描述含 `<>&"'`，钉转义；第 2 小节无期望带（哨兵）。
    fn two_bar_overview() -> SongOverview {
        SongOverview {
            schema_version: "1.1",
            song_meta: SongMeta {
                total_bars: 2,
                duration_sec: 4.0,
                base_bpm: 120.0,
                key_signature: "D minor".to_owned(),
                profile: "default".to_owned(),
            },
            band_definitions_hz: BAND_DEFINITIONS_HZ,
            effective_weights: [0.30, 0.20, 0.20, 0.15, 0.15],
            structural_summary: StructuralSummary {
                climax_bar: 2,
                climax_tension: 0.79,
                dynamic_range_lu: 10.0,
                detected_anomalies: vec![Anomaly {
                    bars: [1, 2],
                    anomaly_type: AnomalyType::LowMidBuildup,
                    description: "Band 3 (250-500Hz) exceeds tilted balance by +5.2 dB <>&\"'"
                        .to_owned(),
                }],
            },
            repetition: RepetitionSummary {
                best_matches: vec![BestMatch {
                    bar_a: 1,
                    bar_b: 5,
                    similarity: 0.97,
                }],
                mechanical_index: 0.5,
                repeated_passages: vec![RepeatedPassage {
                    bars: [5, 8],
                    source_bars: [1, 4],
                    mean_similarity: 0.97,
                }],
            },
            frequency_clash: FrequencyClashSummary::default(),
            bar_series: BarSeries {
                bar: vec![1, 2],
                time_sec: vec![0.0, 2.0],
                section: vec!["Intro <&>".to_owned(), "Verse".to_owned()],
                tension: vec![0.42, 0.79],
                valence_hue: vec![-0.62, -0.5],
                lufs_short: vec![-22.4, -12.4],
                lufs_momentary_max: vec![-19.8, -9.8],
                score_dyn: vec![0.28, 0.58],
                envelope_lo: vec![0.35, NO_ENVELOPE],
                envelope_hi: vec![0.65, NO_ENVELOPE],
                target_deviation: vec![0.0, 0.0],
                density_pitched_onsets_per_beat: vec![1.5, 4.5],
                density_pitched_duty: vec![0.65, 0.8],
                density_perc_onsets_per_beat: vec![0.5, 3.5],
                bands_tilted_db: vec![
                    [-18.2, -14.5, -12.0, -11.4, -15.0, -19.2, -24.0, -31.5],
                    [-13.0, -10.6, -11.8, -9.4, -12.6, -16.2, -20.8, -28.0],
                ],
            },
        }
    }

    fn empty_overview() -> SongOverview {
        SongOverview {
            schema_version: "1.1",
            song_meta: SongMeta {
                total_bars: 0,
                duration_sec: 0.0,
                base_bpm: 120.0,
                key_signature: "C major".to_owned(),
                profile: "default".to_owned(),
            },
            band_definitions_hz: BAND_DEFINITIONS_HZ,
            effective_weights: [0.30, 0.20, 0.20, 0.15, 0.15],
            structural_summary: StructuralSummary {
                climax_bar: 0,
                climax_tension: 0.0,
                dynamic_range_lu: 0.0,
                detected_anomalies: Vec::new(),
            },
            repetition: RepetitionSummary::default(),
            frequency_clash: FrequencyClashSummary::default(),
            bar_series: BarSeries {
                bar: Vec::new(),
                time_sec: Vec::new(),
                section: Vec::new(),
                tension: Vec::new(),
                valence_hue: Vec::new(),
                lufs_short: Vec::new(),
                lufs_momentary_max: Vec::new(),
                score_dyn: Vec::new(),
                envelope_lo: Vec::new(),
                envelope_hi: Vec::new(),
                target_deviation: Vec::new(),
                density_pitched_onsets_per_beat: Vec::new(),
                density_pitched_duty: Vec::new(),
                density_perc_onsets_per_beat: Vec::new(),
                bands_tilted_db: Vec::new(),
            },
        }
    }

    #[test]
    fn escape_html_covers_all_five_characters() {
        assert_eq!(
            escape_html("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(escape_html("plain"), "plain");
        // `&` 最先转：后转出的实体不被二次转义。
        assert_eq!(escape_html("&lt;"), "&amp;lt;");
    }

    #[test]
    fn report_contains_every_section_with_fixture_values() {
        let html = render_html_report(&two_bar_overview());
        // 文件头与页脚。
        assert!(html.contains("<h1>整曲总览报告</h1>"));
        assert!(html.contains("schema 1.1"));
        assert!(html.contains("D minor"));
        assert!(html.contains("schema 版本 1.1"));
        // 四层标题与数值（§8.2 口径：归一化 2 位、响度/频段 1 位）。
        assert!(html.contains("能量层"));
        assert!(html.contains("-22.4"));
        assert!(html.contains("-12.4"));
        assert!(html.contains("频率层"));
        assert!(html.contains("-18.2"));
        assert!(html.contains("Band 3 (250–500 Hz)"));
        assert!(html.contains("疏密层"));
        assert!(html.contains("1.50"));
        assert!(html.contains("情绪层"));
        assert!(html.contains("0.42"));
        assert!(html.contains("0.79"));
        assert!(html.contains("-0.62"));
        // 期望带：第 1 小节有带（0.35/0.65），第 2 小节无带（—）。
        assert!(html.contains("0.35"));
        assert!(html.contains("0.65"));
        assert!(html.contains("—"));
        // 异常诊断与自相似度。
        assert!(html.contains("异常诊断"));
        assert!(html.contains("low_mid_buildup"));
        assert!(html.contains("+5.2 dB"));
        assert!(html.contains("自相似度"));
        assert!(html.contains("0.50"));
        assert!(html.contains("0.97"));
        // 每层一条 SVG 迷你折线图（能量/频率/疏密/情绪各一）。
        assert_eq!(html.matches("<svg").count(), 4);
        assert!(html.contains("<polyline"));
        // 高潮小节与动态范围。
        assert!(html.contains("10.0"));
    }

    #[test]
    fn data_strings_are_escaped_before_they_reach_html() {
        let html = render_html_report(&two_bar_overview());
        // 段落名里的 `<&>` 必须转义，不得原样出现。
        assert!(html.contains("Intro &lt;&amp;&gt;"));
        assert!(!html.contains("Intro <&>"));
        // 异常描述里的 `<>&\"'` 必须全转义。
        assert!(html.contains("&lt;&gt;&amp;&quot;&#39;"));
        assert!(!html.contains("by +5.2 dB <>&\"'"));
    }

    #[test]
    fn report_has_no_external_references_and_no_script() {
        let html = render_html_report(&two_bar_overview());
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("<link"));
        assert!(!html.contains("url("));
        assert!(html.contains("<style>"));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn rendering_is_deterministic() {
        let overview = two_bar_overview();
        assert_eq!(render_html_report(&overview), render_html_report(&overview));
    }

    #[test]
    fn empty_overview_renders_without_panicking() {
        let html = render_html_report(&empty_overview());
        assert!(html.contains("<h1>整曲总览报告</h1>"));
        assert!(html.contains("未检出异常"));
        assert!(html.contains("无数据"));
        assert!(!html.contains("<polyline"));
    }

    #[test]
    fn report_title_is_escaped_in_h1_and_title_tag() {
        let title = "夜半 <>&\"' 试听";
        let html = render_html_report_with_title(&two_bar_overview(), title);
        let escaped = "夜半 &lt;&gt;&amp;&quot;&#39; 试听";
        assert!(
            html.contains(&format!("<h1>{escaped}</h1>")),
            "h1 用转义后的标题"
        );
        assert!(
            html.contains(&format!("<title>{escaped} · ")),
            "title 用转义后的标题"
        );
        assert!(!html.contains(title), "原始标题不得原样出现");
    }

    #[test]
    fn empty_title_falls_back_to_the_generic_title() {
        for title in ["", "   "] {
            let html = render_html_report_with_title(&two_bar_overview(), title);
            assert!(
                html.contains("<h1>整曲总览报告</h1>"),
                "空标题回退到通用标题：{title:?}"
            );
            assert!(!html.contains("<h1></h1>"), "不得输出空 h1");
            assert!(!html.contains("<h1>   </h1>"), "不得输出空白 h1");
        }
    }
}
