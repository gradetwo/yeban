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

/// 内联 CSS（单文件自包含；屏幕深色专业报告风，打印切浅色；每节尽量不跨页断开）。
const STYLE: &str = r#"
:root{--bg:#0f1115;--panel:#171a21;--panel2:#1d2230;--line:#2a2f3a;--line-soft:#232838;--text:#e8eaf0;--muted:#9aa3b2;--faint:#6b7280;--accent:#6ea8fe;--red:#e5534b;--amber:#e8a13a;--teal:#58a6a0;--green:#5fb878;--purple:#b678e6}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:14px/1.55 system-ui,-apple-system,"Segoe UI",sans-serif}
.wrap{max-width:960px;margin:0 auto;padding:28px 24px 48px}
.cover{background:linear-gradient(135deg,#1b2a4a 0%,#141a28 55%,#171a21 100%);border:1px solid var(--line);border-radius:12px;padding:28px 28px 22px;margin-bottom:20px}
h1{font-size:28px;line-height:1.2;margin:0 0 8px;letter-spacing:.2px}
h2{font-size:18px;margin:0 0 12px;padding-left:10px;border-left:4px solid var(--accent)}
h3{font-size:14px;margin:20px 0 8px}
p{margin:8px 0}
.meta{color:var(--muted);margin:4px 0}
.chips{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0 4px}
.chip{display:inline-block;padding:3px 10px;border:1px solid var(--line);border-radius:999px;background:#1d2230;font-size:12px;white-space:nowrap}
.chip b{color:var(--accent);font-weight:600}
.note{color:var(--muted);font-size:12px}
.kpi-row{display:grid;grid-template-columns:repeat(6,1fr);gap:10px;margin:18px 0 4px}
.kpi{background:var(--panel2);border:1px solid var(--line);border-radius:10px;padding:12px 14px}
.kpi-label{font-size:11px;letter-spacing:.06em;color:var(--muted);text-transform:uppercase}
.kpi-value{font-size:26px;font-weight:700;line-height:1.25;font-variant-numeric:tabular-nums;white-space:nowrap}
.kpi-sub{font-size:11px;color:var(--faint)}
.toc{position:sticky;top:0;z-index:5;background:#0f1115;border:1px solid var(--line);border-radius:10px;padding:10px 14px;margin:0 0 20px;display:flex;flex-wrap:wrap;gap:6px 16px}
.toc a{color:var(--accent);text-decoration:none;font-size:13px}
.toc a:hover{text-decoration:underline}
section{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:20px 22px;margin-bottom:18px;break-inside:avoid;scroll-margin-top:64px}
#overview h2{border-left-color:var(--accent)}
#energy h2{border-left-color:var(--red)}
#frequency h2{border-left-color:var(--accent)}
#density h2{border-left-color:var(--green)}
#emotion h2{border-left-color:var(--purple)}
#clash h2{border-left-color:var(--amber)}
#anomalies h2{border-left-color:var(--red)}
#repetition h2{border-left-color:var(--teal)}
table{border-collapse:collapse;width:100%;font-size:12px}
th,td{border:1px solid var(--line);padding:5px 8px;text-align:right;font-variant-numeric:tabular-nums}
th:first-child,td:first-child{text-align:left}
thead th{background:#232838;position:sticky;top:48px}
tbody tr:nth-child(even){background:#1b1f2a}
svg{display:block;width:100%;height:auto;background:#141821;border:1px solid var(--line);border-radius:8px}
.legend{font-size:12px;color:var(--muted)}
.nodata{color:var(--muted);font-style:italic}
footer{margin-top:8px;border-top:1px solid var(--line);padding-top:10px;color:var(--faint);font-size:12px}
.sev{display:inline-block;min-width:34px;text-align:center;padding:2px 8px;border-radius:999px;font-size:11px;font-weight:600}
.sev-severe{background:rgba(229,83,75,.18);color:#ff8a80;border:1px solid #e5534b}
.sev-moderate{background:rgba(232,161,58,.15);color:#f2b95c;border:1px solid #e8a13a}
.sev-mild{background:rgba(88,166,160,.15);color:#7cc9c2;border:1px solid #58a6a0}
.type-low_mid_buildup{color:#f2b95c;border-color:#e8a13a}
.type-compression_squash{color:#ff8a80;border-color:#e5534b}
.type-envelope_deviation{color:#d3a5f5;border-color:#b678e6}
.type-arrangement_repetition{color:#9cc2ff;border-color:#6ea8fe}
.wbar{display:flex;align-items:center;gap:10px;margin:6px 0;font-size:12px}
.wbar-label{width:44px;color:var(--muted)}
.wbar-track{flex:1;height:8px;background:#232838;border-radius:4px;overflow:hidden}
.wbar-fill{height:100%;background:var(--accent);border-radius:4px}
.wbar-value{width:36px;text-align:right;font-variant-numeric:tabular-nums}
.heat{display:flex;align-items:center;gap:10px;margin:6px 0;font-size:12px}
.heat-label{width:170px;color:var(--muted)}
.heat-track{flex:1;height:10px;background:#232838;border-radius:5px;overflow:hidden}
.heat-fill{height:100%;border-radius:5px}
.heat-value{width:64px;text-align:right;font-variant-numeric:tabular-nums}
.obar{width:110px;height:10px;background:#232838;border-radius:5px;overflow:hidden;display:inline-block;vertical-align:middle;margin-right:8px}
.obar-fill{display:block;height:100%;border-radius:5px}
@media (max-width:720px){.kpi-row{grid-template-columns:repeat(3,1fr)}}
@media print{body{background:#fff;color:#111}.wrap{padding:0}.cover{background:#fff;border-color:#bbb;color:#111}.toc{display:none}section{background:#fff;border-color:#bbb;break-inside:avoid}th,td{border-color:#bbb;color:#111}thead th{background:#eee}tbody tr:nth-child(even){background:#f7f7f7}svg{background:#fff;border-color:#bbb}.kpi{background:#fff;border-color:#bbb}.kpi-value{color:#111}.chip{background:#fff;color:#111;border-color:#999}.chip b{color:#1a56db}.meta,.note,.legend,.kpi-sub{color:#444}footer{color:#555;border-color:#bbb}.wbar-track,.heat-track,.obar{background:#e5e5e5}.sev-severe{color:#b3261e}.sev-moderate{color:#8a5a00}.sev-mild{color:#2e7d6b}.type-low_mid_buildup{color:#8a5a00}.type-compression_squash{color:#b3261e}.type-envelope_deviation{color:#6b3fa0}.type-arrangement_repetition{color:#1a56db}}
@page{size:A4;margin:12mm}
"#;

/// 图表画布（`svg_chart` 与期望带多边形共用；SVG 用 viewBox 缩放，CSS 宽度 100%）。
const CHART_WIDTH: f32 = 960.0;
const CHART_HEIGHT: f32 = 180.0;
const CHART_PAD_LEFT: f32 = 56.0;
const CHART_PAD_RIGHT: f32 = 12.0;
const CHART_PAD_TOP: f32 = 10.0;
const CHART_PAD_BOTTOM: f32 = 26.0;

/// 一条折线（`svg_chart` 的输入）：图例名 + 每小节一个值 + 颜色；
/// `area` 为真时在折线下铺半透明面积（只给每层的主线，避免多线叠色）。
struct ChartLine<'a> {
    label: &'a str,
    values: &'a [f32],
    color: &'a str,
    dashed: bool,
    area: bool,
}

/// 折线图（内联 SVG）：`lines` 各画一条折线，`extra` 是画在折线**之下**的附加 SVG
/// 片段（如期望带的多边形），`unit` 是纵轴单位（画在右上角）。
///
/// 空序列 / 全空序列 ⇒ 一段“无数据”文字，不画空图。纵轴范围由调用方给
/// （各层自己的量纲）；范围退化（上下相等）时向两边各扩 0.5 防除零。
/// 非有限值断开折线（该点不画、不连线），⛔ 不把坏值画成 0。
/// 图带 5 档横向网格线 + 纵轴刻度、至多 8 个 x 轴小节刻度。
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
    let plot_w = CHART_WIDTH - CHART_PAD_LEFT - CHART_PAD_RIGHT;
    let plot_h = CHART_HEIGHT - CHART_PAD_TOP - CHART_PAD_BOTTOM;
    let x_of = |index: usize, count: usize| -> f32 {
        if count <= 1 {
            CHART_PAD_LEFT + plot_w / 2.0
        } else {
            CHART_PAD_LEFT + plot_w * index as f32 / (count - 1) as f32
        }
    };
    let y_of = |value: f32| -> f32 { CHART_PAD_TOP + plot_h * (1.0 - (value - lo) / (hi - lo)) };
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg viewBox=\"0 0 {CHART_WIDTH:.0} {CHART_HEIGHT:.0}\" role=\"img\" aria-label=\"折线图\">"
    ));
    // 横向网格线（5 档）+ 纵轴刻度。
    for step in 0..=4 {
        let value = lo + (hi - lo) * step as f32 / 4.0;
        let y = y_of(value);
        let label = fmt_axis(value);
        svg.push_str(&format!(
            "<line x1=\"{CHART_PAD_LEFT:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"#2a2f3a\" stroke-width=\"1\"/>",
            CHART_WIDTH - CHART_PAD_RIGHT
        ));
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"11\" fill=\"#9aa3b2\">{label}</text>",
            CHART_PAD_LEFT - 6.0,
            y + 4.0
        ));
    }
    // x 轴小节刻度（至多 8 个，1-based 小节号）+ 纵向浅网格。
    let max_count = lines
        .iter()
        .map(|line| line.values.len())
        .max()
        .unwrap_or(0);
    if max_count > 1 {
        let step = (max_count - 1).div_ceil(7).max(1);
        let mut index = 0usize;
        while index < max_count {
            let x = x_of(index, max_count);
            svg.push_str(&format!(
                "<line x1=\"{x:.1}\" y1=\"{CHART_PAD_TOP:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\" stroke=\"#232838\" stroke-width=\"1\"/>",
                CHART_PAD_TOP + plot_h
            ));
            svg.push_str(&format!(
                "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"11\" fill=\"#9aa3b2\">{}</text>",
                CHART_HEIGHT - 8.0,
                index + 1
            ));
            index += step;
        }
        // 末小节号（步进未必落在末点时补一个）。
        if (max_count - 1) % step != 0 {
            let x = x_of(max_count - 1, max_count);
            svg.push_str(&format!(
                "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"11\" fill=\"#9aa3b2\">{max_count}</text>",
                CHART_HEIGHT - 8.0
            ));
        }
    }
    // 纵轴单位（右上角）。
    svg.push_str(&format!(
        "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"11\" fill=\"#9aa3b2\">{}</text>",
        CHART_WIDTH - CHART_PAD_RIGHT,
        CHART_PAD_TOP + 10.0,
        escape_html(unit)
    ));
    svg.push_str(extra);
    for line in lines {
        // 连续有限值成段画；断点处另起一段，单点画圆；主线铺面积。
        let mut segment: Vec<(f32, f32)> = Vec::new();
        let flush = |segment: &mut Vec<(f32, f32)>, svg: &mut String| {
            if segment.len() == 1 {
                let (x, y) = segment[0];
                let color = line.color;
                svg.push_str(&format!(
                    "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3\" fill=\"{color}\"/>"
                ));
            } else if segment.len() > 1 {
                let color = line.color;
                if line.area {
                    let base = y_of(lo);
                    let mut points: Vec<String> = segment
                        .iter()
                        .map(|(x, y)| format!("{x:.1},{y:.1}"))
                        .collect();
                    let (first_x, _) = segment[0];
                    let (last_x, _) = segment[segment.len() - 1];
                    points.push(format!("{last_x:.1},{base:.1}"));
                    points.push(format!("{first_x:.1},{base:.1}"));
                    svg.push_str(&format!(
                        "<polygon points=\"{}\" fill=\"{color}\" fill-opacity=\"0.10\" stroke=\"none\"/>",
                        points.join(" ")
                    ));
                }
                let points: Vec<String> = segment
                    .iter()
                    .map(|(x, y)| format!("{x:.1},{y:.1}"))
                    .collect();
                let points = points.join(" ");
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

/// 逐小节柱状图（分轨频率重叠节）：`values` 每小节一个重叠度（0–1），
/// `labels` 为小节号；柱按 `max_overlap` 的严重度着色（>0.5 重 / 0.25–0.5 中 / 其余轻）。
/// 空序列 ⇒ “无数据”文字。纵轴恒为 [0, 1]。
fn svg_bar_chart(values: &[f32], max_values: &[f32], labels: &[u32]) -> String {
    if values.is_empty() {
        return "<p class=\"nodata\">无数据</p>".to_owned();
    }
    let count = values.len();
    let plot_w = CHART_WIDTH - CHART_PAD_LEFT - CHART_PAD_RIGHT;
    let plot_h = CHART_HEIGHT - CHART_PAD_TOP - CHART_PAD_BOTTOM;
    let slot = plot_w / count as f32;
    let bar_w = (slot * 0.62).max(2.0);
    let y_of = |value: f32| -> f32 { CHART_PAD_TOP + plot_h * (1.0 - value.clamp(0.0, 1.0)) };
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg viewBox=\"0 0 {CHART_WIDTH:.0} {CHART_HEIGHT:.0}\" role=\"img\" aria-label=\"逐小节重叠度柱状图\">"
    ));
    for step in 0..=4 {
        let value = step as f32 / 4.0;
        let y = y_of(value);
        let label = fmt_axis(value);
        svg.push_str(&format!(
            "<line x1=\"{CHART_PAD_LEFT:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"#2a2f3a\" stroke-width=\"1\"/>",
            CHART_WIDTH - CHART_PAD_RIGHT
        ));
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"11\" fill=\"#9aa3b2\">{label}</text>",
            CHART_PAD_LEFT - 6.0,
            y + 4.0
        ));
    }
    for (index, &value) in values.iter().enumerate() {
        let x = CHART_PAD_LEFT + slot * index as f32 + (slot - bar_w) / 2.0;
        let y = y_of(value);
        let height = (CHART_PAD_TOP + plot_h - y).max(0.0);
        let max_value = max_values.get(index).copied().unwrap_or(value);
        let color = if max_value > 0.5 {
            "#e5534b"
        } else if max_value >= 0.25 {
            "#e8a13a"
        } else {
            "#58a6a0"
        };
        svg.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bar_w:.1}\" height=\"{height:.1}\" fill=\"{color}\" fill-opacity=\"0.85\"/>"
        ));
        // x 轴小节号（至多 12 个）。
        let step = count.div_ceil(11).max(1);
        if index % step == 0 || index == count - 1 {
            let cx = CHART_PAD_LEFT + slot * index as f32 + slot / 2.0;
            let label = labels.get(index).copied().unwrap_or(index as u32 + 1);
            svg.push_str(&format!(
                "<text x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"11\" fill=\"#9aa3b2\">{label}</text>",
                CHART_HEIGHT - 8.0
            ));
        }
    }
    svg.push_str(&format!(
        "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"11\" fill=\"#9aa3b2\">重叠度</text>",
        CHART_WIDTH - CHART_PAD_RIGHT,
        CHART_PAD_TOP + 10.0
    ));
    svg.push_str("</svg>");
    svg.push_str(
        "<p class=\"legend\"><span style=\"color:#e5534b\">■</span> 重（&gt;0.5）　\
         <span style=\"color:#e8a13a\">■</span> 中（0.25–0.5）　\
         <span style=\"color:#58a6a0\">■</span> 轻（&lt;0.25）</p>",
    );
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
    let plot_w = CHART_WIDTH - CHART_PAD_LEFT - CHART_PAD_RIGHT;
    let plot_h = CHART_HEIGHT - CHART_PAD_TOP - CHART_PAD_BOTTOM;
    let count = series.len();
    if count == 0 {
        return String::new();
    }
    let x_of = |index: usize| -> f32 {
        if count <= 1 {
            CHART_PAD_LEFT + plot_w / 2.0
        } else {
            CHART_PAD_LEFT + plot_w * index as f32 / (count - 1) as f32
        }
    };
    // 张力域恒为 [0, 1]，与 svg_chart 的情绪层纵轴同域。
    let y_of = |value: f32| -> f32 { CHART_PAD_TOP + plot_h * (1.0 - value.clamp(0.0, 1.0)) };
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
            "<polygon points=\"{points}\" fill=\"#7d7bff\" fill-opacity=\"0.22\" stroke=\"none\"/>"
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

/// 分轨频率重叠严重度 → CSS 类（分档见 [`ClashSeverity`]；Wang 2026-11-11 定）。
fn clash_severity_class(severity: crate::song_overview::ClashSeverity) -> &'static str {
    use crate::song_overview::ClashSeverity;
    match severity {
        ClashSeverity::Severe => "sev-severe",
        ClashSeverity::Moderate => "sev-moderate",
        ClashSeverity::Mild => "sev-mild",
    }
}

/// 分轨频率重叠严重度 → 中文名。
fn clash_severity_label(severity: crate::song_overview::ClashSeverity) -> &'static str {
    use crate::song_overview::ClashSeverity;
    match severity {
        ClashSeverity::Severe => "重",
        ClashSeverity::Moderate => "中",
        ClashSeverity::Mild => "轻",
    }
}

/// 异常类型 → CSS 类（只按类型着色，不发明严重度）。
fn anomaly_type_class(anomaly_type: crate::song_overview::AnomalyType) -> &'static str {
    use crate::song_overview::AnomalyType;
    match anomaly_type {
        AnomalyType::LowMidBuildup => "type-low_mid_buildup",
        AnomalyType::CompressionSquash => "type-compression_squash",
        AnomalyType::EnvelopeDeviation => "type-envelope_deviation",
        AnomalyType::ArrangementRepetition => "type-arrangement_repetition",
    }
}

/// KPI 卡（标签 + 大值 + 小字说明；值须已格式化 / 已转义）。
fn kpi_card(label: &str, value: &str, sub: &str) -> String {
    format!(
        "<div class=\"kpi\"><div class=\"kpi-label\">{}</div><div class=\"kpi-value\">{}</div><div class=\"kpi-sub\">{}</div></div>",
        escape_html(label),
        value,
        escape_html(sub)
    )
}

/// 分析权重条（`effective_weights` 的一项：标签 + 0–1 值 + 比例条）。
fn weight_bar(label: &str, value: f32) -> String {
    let pct = (value.clamp(0.0, 1.0) * 100.0).round();
    format!(
        "<div class=\"wbar\"><span class=\"wbar-label\">{}</span><div class=\"wbar-track\"><div class=\"wbar-fill\" style=\"width:{pct:.0}%\"></div></div><span class=\"wbar-value\">{}</span></div>",
        escape_html(label),
        fmt2(value)
    )
}

/// 频段热力条（频段名 + 全曲均值 dB + 按 [−120, 0] dB 映射的条）。
fn band_heat_bar(label: &str, mean_db: f32, color: &str) -> String {
    let pct = ((mean_db + 120.0) / 120.0 * 100.0).clamp(0.0, 100.0);
    format!(
        "<div class=\"heat\"><span class=\"heat-label\">{}</span><div class=\"heat-track\"><div class=\"heat-fill\" style=\"width:{pct:.0}%;background:{color}\"></div></div><span class=\"heat-value\">{} dB</span></div>",
        escape_html(label),
        fmt1(mean_db)
    )
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
        "<title>{title} · {} · {}</title><style>{STYLE}</style></head><body><div class=\"wrap\">",
        escape_html(&meta.key_signature),
        escape_html(&meta.profile),
    ));

    // ---- 封面标题带 ----
    html.push_str("<header class=\"cover\"><h1>");
    html.push_str(&title);
    html.push_str("</h1>");
    let schema = escape_html(overview.schema_version);
    let key = escape_html(&meta.key_signature);
    let profile = escape_html(&meta.profile);
    let total_bars = meta.total_bars;
    let duration = fmt1(meta.duration_sec);
    let bpm = fmt1(meta.base_bpm);
    html.push_str(&format!(
        "<p class=\"meta\">schema {schema} · 调号 {key} · 曲风 {profile} · {total_bars} 小节 · {duration} 秒 · {bpm} BPM</p>",
    ));
    html.push_str(&format!(
        "<div class=\"chips\"><span class=\"chip\">调号 <b>{key}</b></span><span class=\"chip\">曲风 <b>{profile}</b></span><span class=\"chip\"><b>{total_bars}</b> 小节</span><span class=\"chip\">时长 <b>{duration} 秒</b></span><span class=\"chip\">速度 <b>{bpm} BPM</b></span><span class=\"chip\">schema <b>{schema}</b></span></div>",
    ));
    // KPI 卡行（无数据不显该卡、不编造：无小节时高潮/张力/动态范围显 —；
    // 总体重叠只在参评组数 > 0 时显）。
    html.push_str("<div class=\"kpi-row\">");
    let (climax_bar, climax_tension, dynamic_range) = if bars == 0 {
        ("—".to_owned(), "—".to_owned(), "—".to_owned())
    } else {
        (
            summary.climax_bar.to_string(),
            fmt2(summary.climax_tension),
            format!("{} LU", fmt1(summary.dynamic_range_lu)),
        )
    };
    html.push_str(&kpi_card("高潮小节", &climax_bar, "张力峰值所在小节"));
    html.push_str(&kpi_card("最高张力", &climax_tension, "0–1 归一化"));
    html.push_str(&kpi_card("动态范围", &dynamic_range, "短时 LUFS 极差"));
    html.push_str(&kpi_card(
        "异常",
        &summary.detected_anomalies.len().to_string(),
        "见异常诊断节",
    ));
    let clash = &overview.frequency_clash;
    if clash.total_groups > 0 {
        html.push_str(&kpi_card(
            "总体频谱重叠",
            &fmt2(clash.overall_overlap),
            &format!("共 {} 组", clash.total_groups),
        ));
    }
    html.push_str(&kpi_card(
        "机械指数",
        &fmt2(repetition.mechanical_index),
        "≥0.95 匹配占比",
    ));
    html.push_str("</div>");
    html.push_str(
        "<p class=\"note\">生成说明：本报告由 yeban-dsp 渲染器自整曲总览（SongOverview）直接生成；\
         数值与 JSON 总览一致（归一化指标 2 位小数，响度与频段能量 1 位小数）。\
         单文件自包含，无外部资源，无脚本，可打印。</p></header>",
    );

    // ---- 页内目录（纯 CSS 锚点，无 JS） ----
    html.push_str(
        "<nav class=\"toc\" aria-label=\"目录\"><a href=\"#overview\">总览</a><a href=\"#energy\">能量层</a><a href=\"#frequency\">频率层</a><a href=\"#density\">疏密层</a><a href=\"#emotion\">情绪层</a><a href=\"#clash\">分轨频率重叠</a><a href=\"#anomalies\">异常诊断</a><a href=\"#repetition\">自相似度</a></nav>",
    );

    // ---- 总览摘要 ----
    html.push_str("<section id=\"overview\"><h2>总览</h2>");
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
    // 方差自适应后的实际权重 [响度, 密度, 音区, 和声, 配器]（和恒为 1.0）。
    html.push_str("<h3>分析权重</h3>");
    const WEIGHT_LABELS: [&str; 5] = ["响度", "密度", "音区", "和声", "配器"];
    for (index, label) in WEIGHT_LABELS.iter().enumerate() {
        html.push_str(&weight_bar(label, overview.effective_weights[index]));
    }
    html.push_str("<p class=\"note\">权重为方差自适应后的实际权重（和为 1.0）；有死特征时按设计稿 §3.6 衰减并重分配。</p>");
    html.push_str("</section>");

    // ---- 能量层 ----
    html.push_str("<section id=\"energy\"><h2>能量层（LUFS）</h2>");
    html.push_str(&svg_chart(
        &[
            ChartLine {
                label: "短时 LUFS",
                values: &series.lufs_short,
                color: "#e5534b",
                dashed: false,
                area: true,
            },
            ChartLine {
                label: "瞬时 LUFS 最大值",
                values: &series.lufs_momentary_max,
                color: "#e67e22",
                dashed: true,
                area: false,
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
    html.push_str("<section id=\"frequency\"><h2>频率层（8 频段，dB）</h2>");
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
            area: false,
        })
        .collect();
    html.push_str(&svg_chart(&band_lines, BAND_DB_FLOOR, 0.0, "dB", ""));
    // 频段全曲均值能量热力条（均值 = 该频段各小节能量算术均值）。
    if bars > 0 {
        html.push_str("<h3>频段均值能量</h3>");
        for band in 0..8 {
            let mean_db: f32 =
                band_columns[band].iter().sum::<f32>() / band_columns[band].len() as f32;
            html.push_str(&band_heat_bar(
                &band_labels[band],
                mean_db,
                BAND_COLORS[band],
            ));
        }
    }
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
    html.push_str("<section id=\"density\"><h2>疏密层（触发率）</h2>");
    html.push_str(&svg_chart(
        &[
            ChartLine {
                label: "旋律组触发率 (次/拍)",
                values: &series.density_pitched_onsets_per_beat,
                color: "#5fb878",
                dashed: false,
                area: true,
            },
            ChartLine {
                label: "打击组触发率 (次/拍)",
                values: &series.density_perc_onsets_per_beat,
                color: "#2e86c1",
                dashed: true,
                area: false,
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
    html.push_str("<section id=\"emotion\"><h2>情绪层（张力 / 效价）</h2>");
    let band_polygons = envelope_band_polygons(series);
    html.push_str(&svg_chart(
        &[
            ChartLine {
                label: "张力",
                values: &series.tension,
                color: "#b678e6",
                dashed: false,
                area: true,
            },
            ChartLine {
                label: "谱面力度",
                values: &series.score_dyn,
                color: "#e8c53a",
                dashed: true,
                area: false,
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

    // ---- 分轨频率重叠（schema 1.2；此前报告未渲染本节数据） ----
    html.push_str("<section id=\"clash\"><h2>分轨频率重叠</h2>");
    if clash.entries.is_empty() {
        html.push_str("<p>未检出分轨频率重叠（无参评轨对）。</p>");
    } else {
        html.push_str(&format!(
            "<p>总体重叠度 <b>{}</b>（共 {} 组，取重叠度最高的前 {} 条；重叠度 = 第二强轨能量 / 最强轨能量，1 = 两轨等能）。</p>",
            fmt2(clash.overall_overlap),
            clash.total_groups,
            clash.entries.len(),
        ));
        // 逐小节总体重叠度柱状图（柱按该小节最大重叠度的严重度着色）。
        if !clash.bar_overlaps.is_empty() {
            let overlaps: Vec<f32> = clash.bar_overlaps.iter().map(|bar| bar.overlap).collect();
            let max_overlaps: Vec<f32> = clash
                .bar_overlaps
                .iter()
                .map(|bar| bar.max_overlap)
                .collect();
            let bar_labels: Vec<u32> = clash.bar_overlaps.iter().map(|bar| bar.bar).collect();
            html.push_str(&svg_bar_chart(&overlaps, &max_overlaps, &bar_labels));
        }
        let clash_rows: Vec<Vec<String>> = clash
            .entries
            .iter()
            .map(|entry| {
                let band_no = entry.band as usize + 1;
                let [lo, hi] = overview
                    .band_definitions_hz
                    .get(entry.band as usize)
                    .copied()
                    .unwrap_or([0.0, 0.0]);
                let overlap_pct = (entry.overlap.clamp(0.0, 1.0) * 100.0).round();
                let class = clash_severity_class(entry.severity);
                let severity = clash_severity_label(entry.severity);
                vec![
                    entry.bar.to_string(),
                    format!("Band {band_no} ({lo}–{hi} Hz)"),
                    escape_html(&entry.track_a.track_name),
                    escape_html(&entry.track_b.track_name),
                    format!(
                        "<span class=\"obar\"><span class=\"obar-fill {class}\" style=\"width:{overlap_pct:.0}%\"></span></span>{}",
                        fmt2(entry.overlap)
                    ),
                    fmt2(entry.mean_overlap),
                    fmt1(entry.gap_db),
                    format!("<span class=\"sev {class}\">{severity}</span>"),
                    entry.beats.to_string(),
                ]
            })
            .collect();
        html.push_str(&table(
            &[
                "小节",
                "频段",
                "轨 A（最响）",
                "轨 B（次响）",
                "重叠度",
                "均值",
                "差距 (dB)",
                "严重度",
                "拍数",
            ],
            &clash_rows,
        ));
        html.push_str("<p class=\"note\">严重度分档：重 &gt; 0.5（差距 &lt; 3 dB）、中 0.25–0.5（差距 3–6 dB）、轻 &lt; 0.25（差距 &gt; 6 dB）；gap_db 只作参考，不定性（overlap = 10^(−gap_db / 10)）。</p>");
    }
    html.push_str("</section>");

    // ---- 异常诊断 ----
    html.push_str("<section id=\"anomalies\"><h2>异常诊断</h2>");
    if summary.detected_anomalies.is_empty() {
        html.push_str("<p>未检出异常。</p>");
    } else {
        let anomaly_rows: Vec<Vec<String>> = summary
            .detected_anomalies
            .iter()
            .map(|anomaly| {
                let (start, end) = (anomaly.bars[0], anomaly.bars[1]);
                let type_name = anomaly.anomaly_type.as_str();
                let type_class = anomaly_type_class(anomaly.anomaly_type);
                vec![
                    format!("{start}–{end}"),
                    format!("<span class=\"chip {type_class}\">{type_name}</span>"),
                    escape_html(&anomaly.description),
                ]
            })
            .collect();
        html.push_str(&table(&["小节区间", "类型", "描述"], &anomaly_rows));
    }
    html.push_str("</section>");

    // ---- 自相似度 ----
    html.push_str("<section id=\"repetition\"><h2>自相似度（repetition）</h2>");
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
        "<footer><p>schema 版本 {} · 整曲总览 HTML 报告（yeban-dsp 渲染器）</p></footer></div></body></html>",
        escape_html(overview.schema_version),
    ));
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song_overview::{
        Anomaly, AnomalyType, BAND_DEFINITIONS_HZ, BarOverlap, BarSeries, BestMatch, ClashSeverity,
        ClashTrackRef, FrequencyClashEntry, FrequencyClashSummary, RepeatedPassage,
        RepetitionSummary, SongMeta, StructuralSummary,
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
            frequency_clash: FrequencyClashSummary {
                total_groups: 2,
                overall_overlap: 0.42,
                bar_overlaps: vec![
                    BarOverlap {
                        bar: 1,
                        overlap: 0.35,
                        max_overlap: 0.62,
                    },
                    BarOverlap {
                        bar: 2,
                        overlap: 0.42,
                        max_overlap: 0.80,
                    },
                ],
                entries: vec![
                    FrequencyClashEntry {
                        bar: 2,
                        band: 2,
                        track_a: ClashTrackRef {
                            track_id: "t-drums".to_owned(),
                            track_name: "鼓组".to_owned(),
                        },
                        track_b: ClashTrackRef {
                            track_id: "t-bass".to_owned(),
                            track_name: "贝斯".to_owned(),
                        },
                        overlap: 0.62,
                        mean_overlap: 0.48,
                        gap_db: 2.1,
                        severity: ClashSeverity::Severe,
                        beats: 4,
                    },
                    FrequencyClashEntry {
                        bar: 1,
                        band: 4,
                        track_a: ClashTrackRef {
                            track_id: "t-vox".to_owned(),
                            track_name: "主唱".to_owned(),
                        },
                        track_b: ClashTrackRef {
                            track_id: "t-guitar".to_owned(),
                            track_name: "吉他".to_owned(),
                        },
                        overlap: 0.35,
                        mean_overlap: 0.30,
                        gap_db: 4.6,
                        severity: ClashSeverity::Moderate,
                        beats: 4,
                    },
                ],
            },
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
        // SVG 图：四层折线图各一 + 分轨频率重叠逐小节柱状图一。
        assert_eq!(html.matches("<svg").count(), 5);
        assert!(html.contains("<polyline"));
        // 高潮小节与动态范围。
        assert!(html.contains("10.0"));
    }

    #[test]
    fn clash_section_renders_entries_weights_and_heat() {
        let html = render_html_report(&two_bar_overview());
        // 分轨频率重叠：总体重叠度、轨名、严重度色卡、数值。
        assert!(html.contains("分轨频率重叠"));
        assert!(html.contains("鼓组"));
        assert!(html.contains("贝斯"));
        assert!(html.contains("主唱"));
        assert!(html.contains("吉他"));
        assert!(html.contains("sev-severe"));
        assert!(html.contains("sev-moderate"));
        assert!(html.contains("0.62"));
        assert!(html.contains("0.48"));
        assert!(html.contains("2.1"));
        assert!(html.contains("4.6"));
        assert!(html.contains("Band 5 (1000–2000 Hz)"));
        // 分析权重与频段均值能量（此前未渲染的字段）。
        assert!(html.contains("分析权重"));
        assert!(html.contains("0.30"));
        assert!(html.contains("频段均值能量"));
        // 页内目录锚点。
        assert!(html.contains("href=\"#clash\""));
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
        assert!(html.contains("未检出分轨频率重叠"));
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
