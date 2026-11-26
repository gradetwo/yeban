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

/// 按 id 取章节下标（`open_help_topic` 落章用）。
#[must_use]
pub fn chapter_index(id: &str) -> Option<usize> {
    CHAPTERS.iter().position(|chapter| chapter.id == id)
}

// --------------------------------------------------------------------- 帮助锚点注册表（T-FIX-487）

/// 帮助锚点的覆盖状态：只有 `Present` 算已覆盖；`Missing` / `External`
/// 与表外锚点一律不算覆盖（⛔ 不许用全文字符串同名匹配冒充落点）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpTopicStatus {
    /// 应用内可打开（手册章节或既有帮助入口）。
    Present,
    /// 已知主题但正文未入 main（五册手册规划锚点等）：有名字、没落点。
    Missing,
    /// 主题在应用外（外部文档/网站）：应用内无落点。
    External,
}

/// 帮助锚点注册表的一行：锚点名 → 落点或显式状态。
///
/// 锚点名是命令 `help-topic:<anchor>` 的参数；解析只做精确匹配，
/// ⛔ 不做子串/前缀/模糊匹配——同名 UI id（如 `export-wav-dialog`）
/// 不在表里就是未覆盖。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpAnchor {
    /// 锚点名。
    pub anchor: &'static str,
    /// 覆盖状态。
    pub status: HelpTopicStatus,
    /// 落点章节 id；仅 `Present` 的章节锚点有值。
    pub chapter: Option<&'static str>,
    /// 落点既有帮助入口命令名（`help-manual` 等）；仅 `Present` 的入口锚点有值。
    pub entry: Option<&'static str>,
}

impl HelpAnchor {
    /// 是否算已覆盖：只有 `Present` 行算数。
    #[must_use]
    pub const fn is_covered(&self) -> bool {
        matches!(self.status, HelpTopicStatus::Present)
    }
}

/// 帮助锚点注册表：当前 main 可打开的 6 章 + 四个既有帮助入口。
///
/// 五册手册的 270 锚点清单在 main 不可达（`docs/manual-latex` 不存在，
/// 五册 LaTeX 正文尚未合入 main），本表不登记、不断言 270 已覆盖；
/// 清单可达时逐条登记 `Present` / `Missing`，缺口见 T-FIX-487 回执。
/// `Missing` / `External` 状态位已预留（[`HelpTopicStatus`]），供登记时用。
pub const HELP_ANCHORS: &[HelpAnchor] = &[
    // 章节锚点：锚点名即章节 id。
    HelpAnchor {
        anchor: "01-overview",
        status: HelpTopicStatus::Present,
        chapter: Some("01-overview"),
        entry: None,
    },
    HelpAnchor {
        anchor: "02-projects",
        status: HelpTopicStatus::Present,
        chapter: Some("02-projects"),
        entry: None,
    },
    HelpAnchor {
        anchor: "03-tracks-clips",
        status: HelpTopicStatus::Present,
        chapter: Some("03-tracks-clips"),
        entry: None,
    },
    HelpAnchor {
        anchor: "04-piano-roll",
        status: HelpTopicStatus::Present,
        chapter: Some("04-piano-roll"),
        entry: None,
    },
    HelpAnchor {
        anchor: "05-mixer-effects",
        status: HelpTopicStatus::Present,
        chapter: Some("05-mixer-effects"),
        entry: None,
    },
    HelpAnchor {
        anchor: "06-export",
        status: HelpTopicStatus::Present,
        chapter: Some("06-export"),
        entry: None,
    },
    // 入口锚点：锚点名即既有帮助命令名。
    HelpAnchor {
        anchor: "help-manual",
        status: HelpTopicStatus::Present,
        chapter: None,
        entry: Some("help-manual"),
    },
    HelpAnchor {
        anchor: "help-shortcuts",
        status: HelpTopicStatus::Present,
        chapter: None,
        entry: Some("help-shortcuts"),
    },
    HelpAnchor {
        anchor: "help-onboarding",
        status: HelpTopicStatus::Present,
        chapter: None,
        entry: Some("help-onboarding"),
    },
    HelpAnchor {
        anchor: "help-lessons",
        status: HelpTopicStatus::Present,
        chapter: None,
        entry: Some("help-lessons"),
    },
];

/// 精确匹配查注册表；表外锚点返回 `None`（未覆盖，不做任何猜测）。
#[must_use]
pub fn resolve_help_anchor(anchor: &str) -> Option<&'static HelpAnchor> {
    HELP_ANCHORS.iter().find(|row| row.anchor == anchor)
}

/// 已覆盖的锚点：仅 `Present` 行；`Missing` / `External` 行不在其中。
pub fn covered_anchors() -> impl Iterator<Item = &'static HelpAnchor> {
    HELP_ANCHORS.iter().filter(|row| row.is_covered())
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
