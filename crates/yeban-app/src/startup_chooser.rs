//! 启动工程选择器 —— 学 Logic Pro「Choose a Project」的内容与思想
//! （Wang 2026-11-17 令），布局按夜半自己的设计语言（对话框组件与
//! 主题令牌），⛔ 不照搬 Logic 外观。
//!
//! ## 它解决什么
//!
//! 此前无 `--open` 启动 = 直接拿内置样本工程进主窗口，用户没有
//! 「这次要开哪个工程」的选择点。本模块是选择器的**纯逻辑层**
//! （无 Slint、无对话框）：分类、行模型、数据汇集、新建工程构造、
//! Tap Tempo 测速都在这里，界面接线住在
//! [`crate::host::wire_startup_chooser`]，路由判定住在
//! [`crate::cli::startup_route`]。
//!
//! ## 五个分类（Logic 侧栏去掉 Live Loops，其余都要）
//!
//! | 分类 | 内容来源 | 打开语义 |
//! | :--- | :--- | :--- |
//! | 新建工程 | 空工程 + Details（真实字段） | 未命名新工程 |
//! | 最近 | 设置里的最近清单（[`crate::settings`]） | 按路径打开原文件 |
//! | 演示工程 | 内置样本（`default` / `filled`） | 未命名副本（样本无磁盘文件） |
//! | 工程模板 | 工厂模板目录（[`crate::templates`]） | 未命名副本，永不回写 |
//! | 我的模板 | 用户模板目录（[`crate::templates`]） | 未命名副本，永不回写 |
//!
//! ## Details 只放真实字段（普查结论）
//!
//! 工程模型 [`YebanProjectV1`] 里真实存在且落盘的字段：`bpm`
//! （20.0..=999.0）、`time_signature`（分子 1..=32、分母
//! ∈ {1,2,4,8,16,32}）、`key_signature`（升降号 −7..=7 + 大/小调）、
//! `audio_config.sample_rate`（五档枚举）。**输入/输出设备不在
//! 工程模型里**（设备是运行时由 `yeban_engine::device` 协商的），
//! ⛔ 不摆假字段。默认值取规范空工程 `samples::default_project()`
//! 自身的字段值 —— 不改 Details 建出来的工程与既有空工程逐字段
//! 一致（除创建时间）。
//!
//! ## 宿主写入的闭集标签（九语）
//!
//! 行模型里有几处宿主生成的常驻显示文本（最近条目的相对时间与
//! 「文件已不存在」、演示条目的轨道数、调号名）：与模板来源分组
//! 标题（`templates::TemplateSource::group_label_for`）同一口径 ——
//! 按当前语言从九语闭集取名，本模块内单测看守。其余文字（分类名、
//! 字段名、按钮）都在 .slint 侧走 `@tr` 词表。用户内容（文件名、
//! 模板分类名、工程标题）原样显示、不翻译。

use std::path::PathBuf;

use yeban_model::YebanProjectV1;
use yeban_model::project::{KeySignature, SampleRate, TimeSignature};

use crate::cli::{Language, Sample};
use crate::settings::{RecentProject, Settings};
use crate::templates::{TemplateEntry, TemplateRoots, TemplateSource, list_templates};

/// 选择器一次最多画多少行（与模板选择器同一上限纪律：超出部分
/// 由宿主如实报数，⛔ 不静默截断）。
pub const CHOOSER_MAX_ROWS: usize = 64;

/// 最近清单灌进选择器时一并算出的条目（存在性在汇集那一刻查）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    /// 显示名（路径的文件名去后缀派生，⛔ 不另存一份）。
    pub name: String,
    /// 工程文件路径。
    pub path: PathBuf,
    /// 最近一次打开时刻（Unix 毫秒）。
    pub opened_at_unix_ms: u64,
    /// 汇集时文件还在不在。不在 ⇒ 条目置灰、不可点（见模块文档）。
    pub exists: bool,
}

/// 一条演示工程（内置样本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoEntry {
    /// 样本身份（打开时按它构造归档）。
    pub sample: Sample,
    /// 工程标题（样本工程自带，原样显示）。
    pub title: String,
    /// 非主总线轨道数。
    pub track_count: usize,
}

/// 选择器开屏时汇集的全部数据（汇集一次，分类切换只读不重扫 ——
/// 与模板选择器「两跳之间清单只存一份」同一条纪律，防下标漂移）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChooserData {
    /// 最近清单（最近在前）。
    pub recent: Vec<RecentEntry>,
    /// 演示工程（固定两条：`default` 与 `filled`）。
    pub demos: Vec<DemoEntry>,
    /// 全部模板（工厂在前，与 [`list_templates`] 同序）。
    pub templates: Vec<TemplateEntry>,
}

/// 汇集选择器数据：最近清单来自设置（存在性当场查），演示来自
/// 内置样本，模板来自两个模板根目录的当场扫描。
#[must_use]
pub fn gather(settings: &Settings, roots: &TemplateRoots) -> ChooserData {
    let recent = settings
        .recent_projects
        .iter()
        .map(|entry: &RecentProject| RecentEntry {
            name: entry
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned(),
            exists: entry.path.is_file(),
            path: entry.path.clone(),
            opened_at_unix_ms: entry.opened_at_unix_ms,
        })
        .collect();
    let demos = [Sample::Default, Sample::Filled]
        .into_iter()
        .map(|sample| {
            let project = sample.project();
            DemoEntry {
                sample,
                title: project.title.clone(),
                track_count: project
                    .tracks
                    .keys()
                    .filter(|id| **id != project.master_bus_track_id)
                    .count(),
            }
        })
        .collect();
    ChooserData {
        recent,
        demos,
        templates: list_templates(roots),
    }
}

// ---------------------------------------------------------------------------
// 行模型（分组列表形态，与模板选择器同款：标题行与条目行同列）
// ---------------------------------------------------------------------------

/// 选择器内容区的一行种类。`code()` 是 .slint 侧的整数编码
/// （`StartupChooserRow.kind`），两处必须同值，判据对账。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooserRowKind {
    /// 一级分组标题（模板的曲风）。
    StyleHeader,
    /// 二级分组标题（模板的子曲风）。
    SubstyleHeader,
    /// 条目行（可点；`entry_index` 有效）。
    Entry,
}

impl ChooserRowKind {
    /// .slint 侧的整数编码。
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            ChooserRowKind::StyleHeader => 0,
            ChooserRowKind::SubstyleHeader => 1,
            ChooserRowKind::Entry => 2,
        }
    }
}

/// 选择器内容区的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserRow {
    /// 行种类。
    pub kind: ChooserRowKind,
    /// 主文字（条目名 / 分组名）。
    pub label: String,
    /// 副文字（路径＋时间 / 轨道数；标题行为空）。
    pub detail: String,
    /// 条目行指向其分类数据表的下标；标题行为 `None`。
    pub entry_index: Option<usize>,
    /// 条目是否可点（最近条目文件已不存在 ⇒ false；标题行恒 false）。
    pub enabled: bool,
}

fn header(kind: ChooserRowKind, label: String) -> ChooserRow {
    ChooserRow {
        kind,
        label,
        detail: String::new(),
        entry_index: None,
        enabled: false,
    }
}

/// 「最近」分类的行模型：一条一工程，副文字 = 路径 + 相对时间；
/// 文件已不存在的条目副文字换成「文件已不存在」，且不可点。
#[must_use]
pub fn recent_rows(entries: &[RecentEntry], lang: Language, now_ms: u64) -> Vec<ChooserRow> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let detail = if entry.exists {
                format!(
                    "{} · {}",
                    entry.path.display(),
                    relative_time_label(lang, now_ms, entry.opened_at_unix_ms)
                )
            } else {
                format!("{} · {}", entry.path.display(), missing_file_label(lang))
            };
            ChooserRow {
                kind: ChooserRowKind::Entry,
                label: entry.name.clone(),
                detail,
                entry_index: Some(index),
                enabled: entry.exists,
            }
        })
        .collect()
}

/// 「演示工程」分类的行模型：标题是工程自带标题，副文字是轨道数。
#[must_use]
pub fn demo_rows(entries: &[DemoEntry], lang: Language) -> Vec<ChooserRow> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| ChooserRow {
            kind: ChooserRowKind::Entry,
            label: entry.title.clone(),
            detail: tracks_label(lang, entry.track_count),
            entry_index: Some(index),
            enabled: true,
        })
        .collect()
}

/// 模板分类（工厂 / 我的）的行模型：只取该来源的条目，按
/// 曲风 → 子曲风 分组；`entry_index` 是**全量模板清单**的下标
/// （与 [`crate::templates::build_chooser_rows`] 同口径，宿主持
/// 有的那一份清单就是全量清单）。
#[must_use]
pub fn template_rows(entries: &[TemplateEntry], source: TemplateSource) -> Vec<ChooserRow> {
    let mut rows = Vec::new();
    let mut current_style: Option<&str> = None;
    let mut current_substyle: Option<&str> = None;
    for (index, entry) in entries.iter().enumerate() {
        if entry.source != source {
            continue;
        }
        if current_style != Some(entry.style.as_str()) {
            rows.push(header(ChooserRowKind::StyleHeader, entry.style.clone()));
            current_style = Some(entry.style.as_str());
            current_substyle = None;
        }
        if current_substyle != Some(entry.substyle.as_str()) {
            rows.push(header(
                ChooserRowKind::SubstyleHeader,
                entry.substyle.clone(),
            ));
            current_substyle = Some(entry.substyle.as_str());
        }
        rows.push(ChooserRow {
            kind: ChooserRowKind::Entry,
            label: entry.name.clone(),
            detail: String::new(),
            entry_index: Some(index),
            enabled: true,
        });
    }
    rows
}

// ---------------------------------------------------------------------------
// 新建工程（Empty Project + Details）
// ---------------------------------------------------------------------------

/// 新建工程的 Details（全部是工程模型里的真实字段，见模块文档）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewProjectDetails {
    /// 速度（BPM，20.0..=999.0）。
    pub bpm: f64,
    /// 调号；`None` = 无调号（规范空工程的出厂态）。
    pub key_signature: Option<KeySignature>,
    /// 拍号。
    pub time_signature: TimeSignature,
    /// 采样率。
    pub sample_rate: SampleRate,
}

impl NewProjectDetails {
    /// 从一份工程读出 Details 初值（生产装配喂规范空工程，于是
    /// 「不改 Details」与既有空工程逐字段一致）。
    #[must_use]
    pub fn from_project(project: &YebanProjectV1) -> Self {
        Self {
            bpm: project.bpm,
            key_signature: project.key_signature,
            time_signature: project.time_signature,
            sample_rate: project.audio_config.sample_rate,
        }
    }

    /// 把字段钳回模型合法域（界面输入是自由文本/步进，越界值在
    /// 这里收口，⛔ 不把非法值写进工程）。
    #[must_use]
    pub fn sanitized(self) -> Self {
        let mut out = self;
        out.bpm = if out.bpm.is_finite() {
            out.bpm.clamp(20.0, 999.0)
        } else {
            120.0
        };
        out.time_signature.numerator = out.time_signature.numerator.clamp(1, 32);
        if ![1, 2, 4, 8, 16, 32].contains(&out.time_signature.denominator) {
            out.time_signature.denominator = 4;
        }
        if let Some(key) = &mut out.key_signature {
            key.sharps_flats = key.sharps_flats.clamp(-7, 7);
        }
        out
    }

    /// 按 Details 构造一份新工程：以规范空工程为底（0 轨 + 主
    /// 总线），覆盖四个字段，记下创建时间。其余字段与空工程
    /// 逐一相同（标题、种子、元数据口径都不另发明）。
    #[must_use]
    pub fn build_project(&self, now_ms: u64) -> YebanProjectV1 {
        let details = self.sanitized();
        let mut project = yeban_model::samples::default_project();
        project.bpm = details.bpm;
        project.key_signature = details.key_signature;
        project.time_signature = details.time_signature;
        project.audio_config.sample_rate = details.sample_rate;
        project.record_creation_time(now_ms);
        project
    }
}

/// 调号步进表的长度：`None`（无调号）+ 大调 15 + 小调 15。
pub const KEY_CHOICE_COUNT: i32 = 31;

/// 调号步进下标 → 调号：0 = 无调号；1..=15 = 大调（升降号
/// −7..=7）；16..=30 = 小调（升降号 −7..=7）。界外落回 `None`。
#[must_use]
pub fn key_choice(index: i32) -> Option<KeySignature> {
    match index {
        1..=15 => Some(KeySignature {
            sharps_flats: (index - 8) as i8,
            is_minor: false,
        }),
        16..=30 => Some(KeySignature {
            sharps_flats: (index - 23) as i8,
            is_minor: true,
        }),
        _ => None,
    }
}

/// 调号 → 步进下标（[`key_choice`] 的逆）。
#[must_use]
pub fn key_choice_index(key: Option<KeySignature>) -> i32 {
    match key {
        None => 0,
        Some(key) if key.is_minor => i32::from(key.sharps_flats.clamp(-7, 7)) + 23,
        Some(key) => i32::from(key.sharps_flats.clamp(-7, 7)) + 8,
    }
}

/// 大调根音名（按升降号数查表，标准调号表）。
const MAJOR_ROOTS: [&str; 15] = [
    "C♭", "G♭", "D♭", "A♭", "E♭", "B♭", "F", "C", "G", "D", "A", "E", "B", "F♯", "C♯",
];
/// 小调根音名（按升降号数查表，标准调号表）。
const MINOR_ROOTS: [&str; 15] = [
    "A♭", "E♭", "B♭", "F", "C", "G", "D", "A", "E", "B", "F♯", "C♯", "G♯", "D♯", "A♯",
];

/// 调号的显示名（九语闭集）：无调号显示「—」；有调号显示
/// 「根音 + 大调/小调词」。根音名是记谱符号（各语言同形），只有
/// 大/小调词随语言变。
#[must_use]
pub fn key_signature_label(key: Option<KeySignature>, lang: Language) -> String {
    let Some(key) = key else {
        return "—".to_owned();
    };
    let slot = usize::try_from(i32::from(key.sharps_flats.clamp(-7, 7)) + 7).unwrap_or(7);
    let (root, word) = if key.is_minor {
        (MINOR_ROOTS[slot], minor_word(lang))
    } else {
        (MAJOR_ROOTS[slot], major_word(lang))
    };
    format!("{root} {word}")
}

fn major_word(lang: Language) -> &'static str {
    match lang {
        Language::ZhCn => "大调",
        Language::EnUs => "Major",
        Language::FrFr => "majeur",
        Language::EsEs => "mayor",
        Language::PtBr => "maior",
        Language::JaJp => "長調",
        Language::KoKr => "장조",
        Language::RuRu => "мажор",
        Language::DeDe => "Dur",
    }
}

fn minor_word(lang: Language) -> &'static str {
    match lang {
        Language::ZhCn => "小调",
        Language::EnUs => "Minor",
        Language::FrFr => "mineur",
        Language::EsEs => "menor",
        Language::PtBr => "menor",
        Language::JaJp => "短調",
        Language::KoKr => "단조",
        Language::RuRu => "минор",
        Language::DeDe => "Moll",
    }
}

// ---------------------------------------------------------------------------
// Tap Tempo（点按测速，Logic 的做法）
// ---------------------------------------------------------------------------

/// 两次点按间隔超过这个值（毫秒）视为新一轮测速（旧样本作废）。
pub const TAP_TEMPO_RESET_MS: u64 = 2000;

/// 由一串点按时刻（Unix 毫秒，升序）算 BPM：先按
/// [`TAP_TEMPO_RESET_MS`] 切出最后一轮连续点按，轮内不足两次
/// ⇒ `None`（还测不出）；否则按轮内平均间隔换算并钳回
/// 20..=999。纯函数：宿主持有点按时刻表，点击时追加、算完
/// 把结果写回速度字段。
#[must_use]
pub fn tap_tempo_bpm(taps_ms: &[u64]) -> Option<f64> {
    let mut run_start = 0usize;
    for i in 1..taps_ms.len() {
        if taps_ms[i].saturating_sub(taps_ms[i - 1]) > TAP_TEMPO_RESET_MS {
            run_start = i;
        }
    }
    let run = &taps_ms[run_start..];
    if run.len() < 2 {
        return None;
    }
    let span = run[run.len() - 1].saturating_sub(run[0]);
    if span == 0 {
        return None;
    }
    let mean_interval = span as f64 / (run.len() - 1) as f64;
    Some((60_000.0 / mean_interval).clamp(20.0, 999.0))
}

// ---------------------------------------------------------------------------
// 宿主闭集显示标签（九语）
// ---------------------------------------------------------------------------

/// 「文件已不存在」（最近条目置灰时的副文字后缀，九语闭集）。
#[must_use]
pub fn missing_file_label(lang: Language) -> &'static str {
    match lang {
        Language::ZhCn => "文件已不存在",
        Language::EnUs => "file no longer exists",
        Language::FrFr => "le fichier n'existe plus",
        Language::EsEs => "el archivo ya no existe",
        Language::PtBr => "o arquivo não existe mais",
        Language::JaJp => "ファイルは存在しません",
        Language::KoKr => "파일이 더 이상 없습니다",
        Language::RuRu => "файл больше не существует",
        Language::DeDe => "Datei existiert nicht mehr",
    }
}

/// 「N 条轨道」（演示条目副文字，九语闭集）。
#[must_use]
pub fn tracks_label(lang: Language, count: usize) -> String {
    match lang {
        Language::ZhCn => format!("{count} 条轨道"),
        Language::EnUs => format!("{count} tracks"),
        Language::FrFr => format!("{count} pistes"),
        Language::EsEs => format!("{count} pistas"),
        Language::PtBr => format!("{count} faixas"),
        Language::JaJp => format!("{count} トラック"),
        Language::KoKr => format!("트랙 {count}개"),
        Language::RuRu => format!("дорожек: {count}"),
        Language::DeDe => format!("{count} Spuren"),
    }
}

/// 最近打开时间的相对说法（九语闭集）：刚刚 / N 分钟前 /
/// N 小时前 / N 天前。相对说法不需要时区，⛔ 不拿 UTC 冒充
/// 本地时间。`opened_ms` 晚于 `now_ms`（时钟回拨）按「刚刚」。
#[must_use]
pub fn relative_time_label(lang: Language, now_ms: u64, opened_ms: u64) -> String {
    let diff = now_ms.saturating_sub(opened_ms);
    if diff < 60_000 {
        return just_now_label(lang).to_owned();
    }
    if diff < 3_600_000 {
        return minutes_ago_label(lang, diff / 60_000);
    }
    if diff < 86_400_000 {
        return hours_ago_label(lang, diff / 3_600_000);
    }
    days_ago_label(lang, diff / 86_400_000)
}

fn just_now_label(lang: Language) -> &'static str {
    match lang {
        Language::ZhCn => "刚刚",
        Language::EnUs => "just now",
        Language::FrFr => "à l'instant",
        Language::EsEs => "ahora mismo",
        Language::PtBr => "agora mesmo",
        Language::JaJp => "たった今",
        Language::KoKr => "방금",
        Language::RuRu => "только что",
        Language::DeDe => "gerade eben",
    }
}

fn minutes_ago_label(lang: Language, n: u64) -> String {
    match lang {
        Language::ZhCn => format!("{n} 分钟前"),
        Language::EnUs => format!("{n} min ago"),
        Language::FrFr => format!("il y a {n} min"),
        Language::EsEs => format!("hace {n} min"),
        Language::PtBr => format!("há {n} min"),
        Language::JaJp => format!("{n} 分前"),
        Language::KoKr => format!("{n}분 전"),
        Language::RuRu => format!("{n} мин назад"),
        Language::DeDe => format!("vor {n} Min."),
    }
}

fn hours_ago_label(lang: Language, n: u64) -> String {
    match lang {
        Language::ZhCn => format!("{n} 小时前"),
        Language::EnUs => format!("{n} h ago"),
        Language::FrFr => format!("il y a {n} h"),
        Language::EsEs => format!("hace {n} h"),
        Language::PtBr => format!("há {n} h"),
        Language::JaJp => format!("{n} 時間前"),
        Language::KoKr => format!("{n}시간 전"),
        Language::RuRu => format!("{n} ч назад"),
        Language::DeDe => format!("vor {n} Std."),
    }
}

fn days_ago_label(lang: Language, n: u64) -> String {
    match lang {
        Language::ZhCn => format!("{n} 天前"),
        Language::EnUs => format!("{n} d ago"),
        Language::FrFr => format!("il y a {n} j"),
        Language::EsEs => format!("hace {n} d"),
        Language::PtBr => format!("há {n} d"),
        Language::JaJp => format!("{n} 日前"),
        Language::KoKr => format!("{n}일 전"),
        Language::RuRu => format!("{n} дн назад"),
        Language::DeDe => format!("vor {n} Tg."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_tempo_measures_average_interval_and_resets_on_gap() {
        // 500 ms 等间隔四击 ⇒ 120 BPM。
        assert_eq!(tap_tempo_bpm(&[0, 500, 1000, 1500]), Some(120.0));
        // 不足两击测不出。
        assert_eq!(tap_tempo_bpm(&[]), None);
        assert_eq!(tap_tempo_bpm(&[42]), None);
        // 间隔 3 秒 > 重置阈值：只有最后一轮（两击 250 ms）算数 ⇒ 240。
        assert_eq!(tap_tempo_bpm(&[0, 500, 3500, 3750]), Some(240.0));
        // 极快点按钳回上限。
        assert_eq!(tap_tempo_bpm(&[0, 10, 20]), Some(999.0));
    }

    #[test]
    fn key_choice_round_trips_all_slots() {
        for index in 0..KEY_CHOICE_COUNT {
            assert_eq!(key_choice_index(key_choice(index)), index, "下标 {index}");
        }
        assert_eq!(key_choice(0), None);
        assert_eq!(
            key_choice(8),
            Some(KeySignature {
                sharps_flats: 0,
                is_minor: false
            }),
            "下标 8 = C 大调"
        );
        assert_eq!(key_choice(31), None, "界外落回无调号");
    }

    #[test]
    fn key_labels_cover_all_nine_languages() {
        let c_major = Some(KeySignature {
            sharps_flats: 0,
            is_minor: false,
        });
        let a_minor = Some(KeySignature {
            sharps_flats: 0,
            is_minor: true,
        });
        for lang in Language::ALL {
            assert!(
                key_signature_label(c_major, lang).starts_with('C'),
                "{lang:?}"
            );
            assert!(
                key_signature_label(a_minor, lang).starts_with('A'),
                "{lang:?}"
            );
            assert_eq!(key_signature_label(None, lang), "—");
            assert!(!missing_file_label(lang).is_empty());
            assert!(!tracks_label(lang, 3).is_empty());
            assert!(!relative_time_label(lang, 10_000, 10_000).is_empty());
        }
        assert_eq!(key_signature_label(c_major, Language::ZhCn), "C 大调");
        assert_eq!(key_signature_label(a_minor, Language::EnUs), "A Minor");
    }

    #[test]
    fn relative_time_buckets() {
        let now = 10 * 86_400_000;
        assert_eq!(relative_time_label(Language::ZhCn, now, now), "刚刚");
        assert_eq!(
            relative_time_label(Language::ZhCn, now, now - 5 * 60_000),
            "5 分钟前"
        );
        assert_eq!(
            relative_time_label(Language::EnUs, now, now - 2 * 3_600_000),
            "2 h ago"
        );
        assert_eq!(
            relative_time_label(Language::ZhCn, now, now - 3 * 86_400_000),
            "3 天前"
        );
        // 时钟回拨（打开时刻在未来）不炸，按刚刚。
        assert_eq!(relative_time_label(Language::ZhCn, now, now + 5000), "刚刚");
    }

    #[test]
    fn build_project_applies_details_and_keeps_empty_shape() {
        let details = NewProjectDetails {
            bpm: 96.0,
            key_signature: Some(KeySignature {
                sharps_flats: 2,
                is_minor: false,
            }),
            time_signature: TimeSignature {
                numerator: 3,
                denominator: 4,
            },
            sample_rate: SampleRate::Hz44100,
        };
        let project = details.build_project(1_700_000_000_000);
        assert_eq!(project.bpm, 96.0);
        assert_eq!(
            project.key_signature,
            Some(KeySignature {
                sharps_flats: 2,
                is_minor: false
            })
        );
        assert_eq!(project.time_signature.numerator, 3);
        assert_eq!(project.audio_config.sample_rate, SampleRate::Hz44100);
        // 空工程形状：只有主总线一条轨。
        assert_eq!(project.tracks.len(), 1);
        assert!(project.tracks.contains_key(&project.master_bus_track_id));
        // 越界 Details 被钳回合法域。
        let wild = NewProjectDetails {
            bpm: 5000.0,
            time_signature: TimeSignature {
                numerator: 99,
                denominator: 7,
            },
            ..details
        };
        let project = wild.build_project(0);
        assert_eq!(project.bpm, 999.0);
        assert_eq!(project.time_signature.numerator, 32);
        assert_eq!(project.time_signature.denominator, 4);
    }

    #[test]
    fn template_rows_group_by_style_and_substyle_per_source() {
        let entries = vec![
            TemplateEntry {
                source: TemplateSource::Factory,
                style: "流行".to_owned(),
                substyle: "抒情".to_owned(),
                name: "甲".to_owned(),
                path: PathBuf::from("/f/流行/抒情/甲.yeban"),
            },
            TemplateEntry {
                source: TemplateSource::Factory,
                style: "流行".to_owned(),
                substyle: "抒情".to_owned(),
                name: "乙".to_owned(),
                path: PathBuf::from("/f/流行/抒情/乙.yeban"),
            },
            TemplateEntry {
                source: TemplateSource::User,
                style: "流行".to_owned(),
                substyle: "抒情".to_owned(),
                name: "丙".to_owned(),
                path: PathBuf::from("/u/流行/抒情/丙.yeban"),
            },
        ];
        let rows = template_rows(&entries, TemplateSource::Factory);
        assert_eq!(rows.len(), 4, "曲风行 + 子曲风行 + 两条模板");
        assert_eq!(rows[0].kind, ChooserRowKind::StyleHeader);
        assert_eq!(rows[1].kind, ChooserRowKind::SubstyleHeader);
        assert_eq!(rows[2].entry_index, Some(0));
        assert_eq!(rows[3].entry_index, Some(1));
        let user_rows = template_rows(&entries, TemplateSource::User);
        assert_eq!(user_rows.len(), 3);
        assert_eq!(user_rows[2].entry_index, Some(2), "下标是全量清单的下标");
    }
}
