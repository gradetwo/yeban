//! 用户手册：章节表 + Markdown 源（`docs/manual/`）的只读投影。
//!
//! 手册的唯一事实源是仓里的 Markdown 文件，本模块用 `include_str!` 把它们
//! 编进二进制，应用内「帮助 → 用户手册」按行渲染。章节清单与
//! `docs/manual/README.md` 的登记表一一对应，判据逐项对账：
//! 章节文件缺失 / 解析不出标题 / 正文过短都会变红。
//!
//! 渲染子集（与手册写作约束一致）：`#` / `##` / `###` 标题、`-` 列表项、
//! 代码围栏、普通段落；其余 Markdown 语法按原文呈现，不猜测渲染。

/// 一行已解析的手册内容。`level`：0 = 正文，1/2/3 = 标题层级，
/// 4 = 列表项，5 = 代码行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualLine {
    /// 行文本（已去掉 Markdown 结构标记）。
    pub text: String,
    /// 行种类，见类型注释。
    pub level: u8,
}

/// 一章手册。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManualChapter {
    /// 章节 id（与文件名去扩展名一致）。
    pub id: &'static str,
    /// 章节标题（目录与窗口标题用）。
    pub title: &'static str,
    /// 章节 Markdown 全文。
    pub markdown: &'static str,
}

/// 章节表：顺序即手册目录顺序，与 `docs/manual/README.md` 一致。
pub const CHAPTERS: &[ManualChapter] = &[
    ManualChapter {
        id: "01-overview",
        title: "界面总览",
        markdown: include_str!("../../../docs/manual/01-overview.md"),
    },
    ManualChapter {
        id: "02-projects",
        title: "新建与保存工程",
        markdown: include_str!("../../../docs/manual/02-projects.md"),
    },
    ManualChapter {
        id: "03-tracks-clips",
        title: "轨道与片段",
        markdown: include_str!("../../../docs/manual/03-tracks-clips.md"),
    },
    ManualChapter {
        id: "04-piano-roll",
        title: "钢琴卷帘编辑",
        markdown: include_str!("../../../docs/manual/04-piano-roll.md"),
    },
    ManualChapter {
        id: "05-mixer-effects",
        title: "混音台与效果",
        markdown: include_str!("../../../docs/manual/05-mixer-effects.md"),
    },
    ManualChapter {
        id: "06-export",
        title: "导出音频",
        markdown: include_str!("../../../docs/manual/06-export.md"),
    },
];

/// 按 id 取章节。
#[must_use]
pub fn chapter(id: &str) -> Option<&'static ManualChapter> {
    CHAPTERS.iter().find(|chapter| chapter.id == id)
}

/// 把一章 Markdown 解析成渲染行。规则见模块头；空行不产生行
/// （段落间距由渲染层统一给），代码围栏内的行原样保留。
#[must_use]
pub fn parse_chapter(markdown: &str) -> Vec<ManualLine> {
    let mut lines = Vec::new();
    let mut in_code = false;
    for raw in markdown.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(ManualLine {
                text: line.to_owned(),
                level: 5,
            });
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("### ") {
            lines.push(ManualLine {
                text: text.trim().to_owned(),
                level: 3,
            });
        } else if let Some(text) = trimmed.strip_prefix("## ") {
            lines.push(ManualLine {
                text: text.trim().to_owned(),
                level: 2,
            });
        } else if let Some(text) = trimmed.strip_prefix("# ") {
            lines.push(ManualLine {
                text: text.trim().to_owned(),
                level: 1,
            });
        } else if let Some(text) = trimmed.strip_prefix("- ") {
            lines.push(ManualLine {
                text: text.trim().to_owned(),
                level: 4,
            });
        } else {
            lines.push(ManualLine {
                text: trimmed.to_owned(),
                level: 0,
            });
        }
    }
    lines
}
