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
        anchor: "getting-started-onboarding",
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
    // -----------------------------------------------------------------
    // 五册手册节级锚点（349 条，外部文档）。T-FIX-694：
    // 手册未入 main，统一登记 `External`；接入后逐条转 `Present`。
    // `getting-started-onboarding` 取注册表侧改名，由上面 Present 行承载。
    HelpAnchor {
        anchor: "arrangement-arpeggio",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-bass",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-before-mixing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-brass",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-chapter-map",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-common-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-comping",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-contour",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-conventions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-counterpoint",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-drums",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-electronic-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-electronic-layers",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-electronic-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-electronic-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-energy",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-energy-curve",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-folk-demo",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-frequency-roles",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-glossary",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-harmonic-rhythm",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-harmony-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-harmony-job",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-harmony-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-harmony-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-instrumentation-budget",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-lead-vocal",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-learning-paths",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-melody-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-melody-job",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-melody-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-melody-rhythm",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-melody-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-motif",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-overview-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-pad",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-phrase",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-plan",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-pop-demo",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-pop-layers",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-rhythm-align",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-rhythm-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-rhythm-job",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-rhythm-layers",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-rhythm-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-rhythm-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-role-card",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-sample-loop",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-section-functions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-bowing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-ranges",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-section",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-skeleton",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-spacing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-string-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-structure-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-structure-map",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-structure-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-structure-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-synth-params",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-textures",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-three-pass",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-timbre-layering",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-transitions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-double",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-harmony",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-roles",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-stack",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-vocal-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-voicing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-walkthrough-checklist",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-walkthrough-delivery",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-walkthrough-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-walkthrough-goal",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-walkthrough-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-walkthrough-steps",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-what-is-arrangement",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-winds-combination",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-winds-exercise",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-winds-jobs",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-winds-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-winds-ranges",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-winds-yeban",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-woodwinds",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "arrangement-yeban-demo",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sample-terms",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "drum-current-limits",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "drum-dialog",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "drum-families",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "drum-presets-generation",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "drum-step-sequencing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "drum-voice-synthesis",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "exercise-bass",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "exercise-common-base",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "exercise-lead",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "exercise-pad",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "exercise-three-tracks",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "exercise-troubleshooting",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "factory-synth-presets",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "instrument-signal-flow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modal-basics",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modal-current-status",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modal-excitation-voices",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modal-four-models",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modal-shared-params",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modal-use-cases",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-ara-boundary",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-execution",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-formats",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-limits",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-load-contract",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-scan",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "plugin-workflow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "preset-save-load",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-entry-points",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-file-fallback",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-library-management",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-troubleshooting",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-what-it-is",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-zones-velocity",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "synth-factory-presets",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "synth-parameters",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "synth-signal-chain",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "track-instrument-routing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-choir-pad",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-common-base",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-fm-bell",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-glass-pluck",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-hollow-lead",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-parameter-tables",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-preset-boundary",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-sub-bass",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "voice-warm-organ",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-device-param",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-gaps",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-lanes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-layers",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-pan",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-points-curves",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-read-write",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-record-one-point",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-section-plan",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-send",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-volume",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "aux-return-sends",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "channel-strip-width",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "dyn-compressor-params",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "dyn-compressor-ui",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "dyn-four-kinds",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "dyn-limiter-gate-status",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "dyn-workflow-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "eq-boundaries",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "eq-common-bands",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "eq-curve-drag",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "eq-frequency-map",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "eq-subtract-first",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "eq-three-band",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-checklist",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-options-loudness",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "gain-channel-strip",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "gain-check-workflow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "gain-common-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "gain-level-fader",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "gain-structure-principles",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "loudness-readings",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "master-bus-chain",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "master-limiters",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "master-signal-flow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mastering-gaps",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixing-goals",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixing-monitoring",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixing-reference-track",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixing-signal-flow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixing-workflow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-adjacent-devices",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-clipper-crusher",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-coloration-chain",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-gaps",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-tape",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-three-principles",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mod-workflow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mono-compatibility",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mono-maker",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "pan-center-identity",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "sfz-pan-family",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-convolution-params",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-delay-params",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-devices-status",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-insert-vs-send",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-reverb-basics",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-reverb-params",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "space-workflow",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "stereo-layers",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "stereo-mistakes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "track-pan-laws",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-automation",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-balance-gain",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-boundaries",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-bus-export",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-compressor",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-console",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-eq-low",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-export",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-export-check",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-prep",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "walkthrough-processing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-aaba",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-arrangement-structure",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-bass-voicing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-borrowed-secondary",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-cadences",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch1-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch2-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch3-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch4-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch5-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch6-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch7-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch8-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-ch9-practice",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-chord-inversion",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-chord-root",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-chord-symbols",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-consonance-dissonance",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-dynamics",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-equal-temperament",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-form",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-functional-progressions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-groove-tempo",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-harmonic-function",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-interval-ear-training",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-interval-quantity-quality",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-inversion-enharmonic",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-jianpu-tab",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-key-signature-circle",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-major-scale",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-melody-harmony",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-minor-scales",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-modes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-modulation",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-motive",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-note-names-keyboard",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-note-values",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-octave-semitone",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-pentatonic",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-phrase",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-chords",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-intervals",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-melody",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-notation",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-pitch",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-progressions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-piano-roll-scales",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-progressions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-pulse-beat-tempo",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-relative-keys",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-repetition-variation",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-rests-weak-beats",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-rhythm-motive",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-scale-degrees",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-scientific-pitch-midi",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-semitone-distance",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-sequence",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-sevenths",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-solfege-numbered-notation",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-sound-dimensions",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-staff",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-staff-note-values",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-syncopation-backbeat",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-time-signature",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-timeline-grid",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-transition",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-triads",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-verse-chorus",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "music-theory-voice-leading",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "audio-asset-pool",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "audio-import-place",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "audio-receipt-honesty",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "audio-trim-fade",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-edit-curve",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-show-lanes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-target-binding",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "automation-write-modes",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "clips-create",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "clips-cross-track",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "clips-launch-matrix",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "clips-merge-multiselect",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "clips-move-resize",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "entry-cli-features",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "entry-cli-switches",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "entry-mcp-forms",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "entry-mcp-tools",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "entry-tui",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-project-archive",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-stems-purpose",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-stems-receipt",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-wav-dialog",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "export-wav-receipt",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "getting-started-install",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "interface-arrangement",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "interface-console",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "interface-inspector-library",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "interface-transport",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixer-channel-strip",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixer-control-room",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixer-insert-chain",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixer-manual-division",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "mixer-sends-buses",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "modulation-lfo-boundary",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-atomic-save",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-container-assets",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-file-menu",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-first-arrange",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-new-demo",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-recent-recovery",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-save-export",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-settings",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-startup-chooser",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-template-new",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-template-save",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "project-undo-tree",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "record-landing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "record-panel-input",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-note-drag",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-note-entry",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-open-view",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-quantize-humanize",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-scale-step",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-velocity-lane",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "roll-zoom-framing",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "shortcuts-arrange",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "shortcuts-ime-exception",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "shortcuts-panels",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "shortcuts-policy-table",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "shortcuts-project-edit",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "shortcuts-record-misc",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "tracks-types-header",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "troubleshoot-devices",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "troubleshoot-performance",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "troubleshoot-project-open",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "troubleshoot-silence",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
    },
    HelpAnchor {
        anchor: "troubleshoot-verification",
        status: HelpTopicStatus::External,
        chapter: None,
        entry: None,
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
