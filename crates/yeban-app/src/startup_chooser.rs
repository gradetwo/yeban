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
//! | 演示工程 | 文件型 demo（见下；内置样本已按 Wang 2026-11-23 裁决移出清单） | 未命名副本 |
//! | 工程模板 | 工厂模板目录（[`crate::templates`]） | 未命名副本，永不回写 |
//! | 我的模板 | 用户模板目录（[`crate::templates`]） | 未命名副本，永不回写 |
//!
//! ## 文件型 demo（实验，`YEBAN_DEMOS_DIR`）
//!
//! 环境变量 [`DEMOS_DIR_ENV`] 指向一个目录时，`gather` 扫描该
//! 目录作为演示清单的唯一来源：每个 `*.yeban` 文件是一条文件型 demo，
//! 标题、BPM 与跨度从容器真实读取（[`read_project_container`]），
//! 读不出来的文件**响亮报错并跳过**（⛔ 不编造标题）；封面是与工程
//! 同名的 `.png` / `.jpg`，或同名加 `-cover` 后缀的 `.png` / `.jpg`。
//! 曲风等专辑页字段容器里没有，由同名旁挂 `<stem>.meta.json`
//! 提供（`genre` / `released` / `description` / `credits` /
//! `copyright` / `license` / `website`，全可选，缺失即不列该行，
//! ⛔ 不编造）。
//! 环境变量未设 ⇒ 演示清单为空（内置样本已移出清单，见上）。
//! 文件型 demo 打开时
//! 从磁盘真实加载归档，同样走未命名副本口径（不记最近、不回写源
//! 文件）。
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
//! 「文件已不存在」、演示条目「详情」里的轨道数/小节数/时长、调号
//! 名）：与模板来源分组
//! 标题（`templates::TemplateSource::group_label_for`）同一口径 ——
//! 按当前语言从九语闭集取名，本模块内单测看守。其余文字（分类名、
//! 字段名、按钮）都在 .slint 侧走 `@tr` 词表。用户内容（文件名、
//! 模板分类名、工程标题）原样显示、不翻译。

use std::path::{Path, PathBuf};

use yeban_model::YebanProjectV1;
use yeban_model::container::{ContainerLimits, read_project_container};
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

/// 文件型 demo 目录的环境变量名（实验开关，见模块文档）。
pub const DEMOS_DIR_ENV: &str = "YEBAN_DEMOS_DIR";

/// 一条演示工程的来源：内置样本，或磁盘上的工程文件（实验）。
/// 演示清单自 Wang 2026-11-23 裁决后只由文件型条目构成（见
/// [`gather`]），`Builtin` 变体保留在闭集里、宿主打开臂照常处理
/// 它，只是不再有产出方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoSource {
    /// 内置样本（打开时按它构造归档）。
    Builtin(Sample),
    /// 磁盘上的 `.yeban` 文件（打开时从磁盘真实加载归档）。
    File(PathBuf),
}

/// 一条演示工程。
#[derive(Debug, Clone, PartialEq)]
pub struct DemoEntry {
    /// 来源（打开时按它取归档）。
    pub source: DemoSource,
    /// 工程标题（样本工程自带 / 文件型从容器读出，原样显示）。
    pub title: String,
    /// 非主总线轨道数（只进卡片「详情」，不上卡片主面）。
    pub track_count: usize,
    /// 作者（容器 `author` 字段实读，经 [`author_name`] 切出作者
    /// 名；空 ⇒ `None`，详情不列作者行）。
    pub author: Option<String>,
    /// 曲风/风格（文件型 demo 读同名旁挂 `<stem>.meta.json` 的
    /// `genre`；容器本身没有曲风字段，缺失即 `None`，卡片不编造）。
    pub genre: Option<String>,
    /// 旁挂 meta（专辑页字段：发布日期 / 简介 / 制作名单 / 版权 /
    /// 许可 / 作者网站；只有文件型 demo 有旁挂文件，无旁挂时恒
    /// 全缺失。曲风也在旁挂里，与 `genre` 字段同源）。
    pub meta: DemoMeta,
    /// 工程速度（样本工程 / 容器自带 `bpm`，实读）。
    pub bpm: f64,
    /// 全曲小节数（按片段末端 tick 与拍号实算，只进「详情」）。
    pub bars: usize,
    /// 全曲时长（秒，按片段末端 tick 与 bpm 实算，只进「详情」）。
    pub duration_secs: f64,
    /// 封面图路径（文件型 demo 的同名图片；无同名图片时 `None`）。
    pub cover: Option<PathBuf>,
}

/// 选择器开屏时汇集的全部数据（汇集一次，分类切换只读不重扫 ——
/// 与模板选择器「两跳之间清单只存一份」同一条纪律，防下标漂移）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChooserData {
    /// 最近清单（最近在前）。
    pub recent: Vec<RecentEntry>,
    /// 演示工程（只有 [`DEMOS_DIR_ENV`] 扫描出的文件型 demo；
    /// 环境变量未设时为空。内置样本已按 Wang 2026-11-23 裁决
    /// 移出本清单）。
    pub demos: Vec<DemoEntry>,
    /// 全部模板（工厂在前，与 [`list_templates`] 同序）。
    pub templates: Vec<TemplateEntry>,
}

/// 汇集选择器数据：最近清单来自设置（存在性当场查），演示来自
/// 文件型 demo 目录的当场扫描，模板来自两个模板根目录的当场扫描。
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
    // 演示清单只剩文件型 demo（Wang 2026-11-23 15:11 终定）：两个
    // 系统内置样本（`default` / `filled`）不再进选择页清单。样本
    // 本体不受影响——CLI `--sample` 与 `ProjectSource::Sample` 仍可
    // 打开它们；`DemoSource::Builtin` 变体与宿主打开臂保留（类型
    // 闭集不缩），只是本函数不再产出。环境变量未设 ⇒ 清单为空，
    // 演示分类如实显示空清单，⛔ 不拿内置样本回填。
    let mut demos: Vec<DemoEntry> = Vec::new();
    if let Some(dir) = std::env::var_os(DEMOS_DIR_ENV)
        && !dir.is_empty()
    {
        demos.extend(scan_file_demos(Path::new(&dir)));
    }
    ChooserData {
        recent,
        demos,
        templates: list_templates(roots),
    }
}

/// 扫描文件型 demo 目录（实验）：每个 `*.yeban` 文件一条，标题与
/// 轨道数从容器真实读取；读不出的文件**响亮报错并跳过**（⛔ 不
/// 编造）；与工程同名的 `.png` / `.jpg` 是封面。条目按路径排序，
/// ⛔ 不依赖 `read_dir` 的文件系统顺序。
fn scan_file_demos(dir: &Path) -> Vec<DemoEntry> {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            eprintln!(
                "startup-chooser: 文件型 demo 目录 `{}` 读不了: {error}",
                dir.display()
            );
            return Vec::new();
        }
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "yeban") {
                    paths.push(path);
                }
            }
            Err(error) => {
                eprintln!(
                    "startup-chooser: 文件型 demo 目录 `{}` 有条目读不了: {error}",
                    dir.display()
                );
            }
        }
    }
    paths.sort();
    let mut demos = Vec::new();
    for path in paths {
        match load_demo_entry(&path) {
            Ok(entry) => demos.push(entry),
            Err(message) => {
                eprintln!("startup-chooser: 文件型 demo 跳过: {message}");
            }
        }
    }
    demos
}

/// 从磁盘真实读取一条文件型 demo 的标题与轨道数（整包读容器 ——
/// 实验口径，见回执）。读不出 ⇒ `Err` 写明原因，调用方响亮跳过。
fn load_demo_entry(path: &Path) -> Result<DemoEntry, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("`{}` 读不了: {error}", path.display()))?;
    let archive = read_project_container(&bytes, &ContainerLimits::default())
        .map_err(|error| format!("`{}` 不是可读的工程容器: {error}", path.display()))?;
    let project = &archive.project;
    // 封面候选（先到先得）：同名图、同名 + `-cover` 后缀图。
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let cover = ["png", "jpg"]
        .iter()
        .flat_map(|ext| {
            [
                path.with_extension(ext),
                path.with_file_name(format!("{stem}-cover.{ext}")),
            ]
        })
        .find(|candidate| candidate.is_file());
    let (bars, duration_secs) = demo_span(project);
    let meta = read_sidecar_meta(path, stem);
    Ok(DemoEntry {
        source: DemoSource::File(path.to_path_buf()),
        title: project.title.clone(),
        track_count: project
            .tracks
            .keys()
            .filter(|id| **id != project.master_bus_track_id)
            .count(),
        author: author_name(&project.author),
        genre: meta.genre.clone(),
        meta,
        bpm: project.bpm,
        bars,
        duration_secs,
        cover,
    })
}

/// 全曲跨度实算：取全部轨道片段末端 tick 的最大值，按拍号换
/// 小节、按 bpm 换秒。无片段 ⇒ (0, 0.0)，「详情」里不列这两项。
fn demo_span(project: &YebanProjectV1) -> (usize, f64) {
    let end_tick = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .map(|clip| clip.start_tick.saturating_add(clip.duration_ticks))
        .max()
        .unwrap_or(0);
    let ticks_per_bar =
        yeban_model::ids::PPQ.saturating_mul(u64::from(project.time_signature.numerator.max(1)));
    let bars = if ticks_per_bar == 0 {
        0
    } else {
        usize::try_from(end_tick.div_ceil(ticks_per_bar)).unwrap_or(usize::MAX)
    };
    let duration_secs = if project.bpm > 0.0 {
        end_tick as f64 / yeban_model::ids::PPQ as f64 / project.bpm * 60.0
    } else {
        0.0
    };
    (bars, duration_secs)
}

/// 作者名切法：容器 `author` 是自由文本注记，常把作者名与风格 /
/// 版本注记写在同一段里（《雨声太吵》实测为
/// `雨窗 · Rain-window Grunge-Pop（4:24 版）`）。专辑页的「作者」
/// 只取首个 `·` 之前的段并去两端空白；整段为空 ⇒ `None`
/// （详情不列作者行），⛔ 不从注记段里猜作者。
fn author_name(author: &str) -> Option<String> {
    author
        .split('·')
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// 旁挂 meta（实验）：容器没有的专辑页字段由工程同目录的
/// `<stem>.meta.json` 提供（`genre` / `released` / `description` /
/// `credits` / `copyright` / `license` / `website`，全部可选）。
/// 文件缺失 ⇒ 全 `None`（对应详情行不出现）；文件在但解析失败
/// ⇒ **响亮**报错并按全缺失处理，⛔ 不编造。字段不是非空字符串
/// ⇒ 按缺失处理。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DemoMeta {
    /// 曲风/风格（同时是卡片主面的曲风行来源）。
    pub genre: Option<String>,
    /// 发布日期（原文显示，如 `2026-11-20`）。
    pub released: Option<String>,
    /// 简介。
    pub description: Option<String>,
    /// 制作名单。
    pub credits: Option<String>,
    /// 版权声明。
    pub copyright: Option<String>,
    /// 许可协议。
    pub license: Option<String>,
    /// 作者网站（只展示 URL 文本）。
    pub website: Option<String>,
}

fn read_sidecar_meta(path: &Path, stem: &str) -> DemoMeta {
    let meta_path = path.with_file_name(format!("{stem}.meta.json"));
    let Some(text) = std::fs::read_to_string(&meta_path).ok() else {
        return DemoMeta::default();
    };
    let field = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => DemoMeta {
            genre: field(&value, "genre"),
            released: field(&value, "released"),
            description: field(&value, "description"),
            credits: field(&value, "credits"),
            copyright: field(&value, "copyright"),
            license: field(&value, "license"),
            website: field(&value, "website"),
        },
        Err(error) => {
            eprintln!(
                "startup-chooser: 旁挂 meta `{}` 解析失败: {error}",
                meta_path.display()
            );
            DemoMeta::default()
        }
    }
}

/// 从磁盘真实加载一份文件型 demo 的完整归档（打开语义用）。
///
/// # Errors
///
/// 文件读不了或容器不合法时返回写明原因的错误，调用方走既有
/// fail 回执路径响亮报错。
pub fn load_file_demo_archive(
    path: &Path,
) -> Result<yeban_model::container::ProjectArchive, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("`{}` 读不了: {error}", path.display()))?;
    read_project_container(&bytes, &ContainerLimits::default())
        .map_err(|error| format!("`{}` 不是可读的工程容器: {error}", path.display()))
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

/// 演示卡片「详情」展开区里的一行：标签（弱化显示）+ 值。
/// 只由 [`demo_detail_rows`] 产出（专辑页顺序、有数据才有行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoDetailRow {
    /// 字段标签（九语闭集，见 [`detail_field_label`]）。
    pub label: String,
    /// 字段值（实读实算 / 旁挂原文）。
    pub value: String,
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
    /// 曲风/风格（只演示条目有；其余行为空串）。
    pub genre: String,
    /// 弱化速度行（如 `129 BPM`；只演示条目有，其余行为空串）。
    pub bpm_label: String,
    /// 「详情」展开文本（`details` 行的纯文本拼法，供无障碍标签；
    /// 只演示条目有，其余行为空串）。
    pub details: String,
    /// 「详情」展开区的带标签行（专辑页构成，只演示条目有）。
    pub detail_rows: Vec<DemoDetailRow>,
    /// 展开区高度估算（px，界面层据此把卡片长高；非演示行 0）。
    pub details_height_px: u32,
    /// 条目行指向其分类数据表的下标；标题行为 `None`。
    pub entry_index: Option<usize>,
    /// 条目是否可点（最近条目文件已不存在 ⇒ false；标题行恒 false）。
    pub enabled: bool,
    /// 封面图路径（只有文件型 demo 条目有；其余行一律 `None`）。
    pub cover: Option<PathBuf>,
}

fn header(kind: ChooserRowKind, label: String) -> ChooserRow {
    ChooserRow {
        kind,
        label,
        detail: String::new(),
        genre: String::new(),
        bpm_label: String::new(),
        details: String::new(),
        detail_rows: Vec::new(),
        details_height_px: 0,
        entry_index: None,
        enabled: false,
        cover: None,
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
                genre: String::new(),
                bpm_label: String::new(),
                details: String::new(),
                detail_rows: Vec::new(),
                details_height_px: 0,
                entry_index: Some(index),
                enabled: entry.exists,
                cover: None,
            }
        })
        .collect()
}

/// 「演示工程」分类的行模型（唱片架卡片口径，Wang 2026-11-23
/// 定）：主面只留标题、曲风与弱化的 BPM；专辑页字段只进「详情」
/// 展开区（带标签行，见 [`demo_detail_rows`]），⛔ 不上主面。
#[must_use]
pub fn demo_rows(entries: &[DemoEntry], lang: Language) -> Vec<ChooserRow> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let detail_rows = demo_detail_rows(lang, entry);
            ChooserRow {
                kind: ChooserRowKind::Entry,
                label: entry.title.clone(),
                detail: String::new(),
                genre: entry.genre.clone().unwrap_or_default(),
                bpm_label: bpm_label(entry.bpm),
                details: detail_rows
                    .iter()
                    .map(|row| format!("{} {}", row.label, row.value))
                    .collect::<Vec<_>>()
                    .join("\n"),
                details_height_px: detail_rows_height_px(&detail_rows),
                detail_rows,
                entry_index: Some(index),
                enabled: true,
                cover: entry.cover.clone(),
            }
        })
        .collect()
}

/// 卡片上的弱化速度行（语言中立写法：整数不带小数尾巴）。
#[must_use]
pub fn bpm_label(bpm: f64) -> String {
    if bpm.fract().abs() < f64::EPSILON {
        format!("{bpm:.0} BPM")
    } else {
        format!("{bpm:.1} BPM")
    }
}

/// 详情字段（专辑详情页的构成与顺序，Wang 2026-11-23 定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailField {
    Author,
    Genre,
    Released,
    Duration,
    Bars,
    Tracks,
    Description,
    Credits,
    Copyright,
    License,
    Website,
}

/// 详情字段标签（九语闭集；时长 / 小节 / 轨道的词根与
/// [`duration_label`] / [`bars_label`] / [`tracks_label`] 同源）。
/// ⚠ 非中文译文按既有词汇拼成，未经专业复核（实验口径）。
fn detail_field_label(lang: Language, field: DetailField) -> &'static str {
    use DetailField as F;
    use Language as L;
    match (lang, field) {
        (L::ZhCn, F::Author) => "作者",
        (L::ZhCn, F::Genre) => "曲风",
        (L::ZhCn, F::Released) => "发布日期",
        (L::ZhCn, F::Duration) => "时长",
        (L::ZhCn, F::Bars) => "小节数",
        (L::ZhCn, F::Tracks) => "轨道数",
        (L::ZhCn, F::Description) => "简介",
        (L::ZhCn, F::Credits) => "制作名单",
        (L::ZhCn, F::Copyright) => "版权声明",
        (L::ZhCn, F::License) => "许可协议",
        (L::ZhCn, F::Website) => "作者网站",
        (L::EnUs, F::Author) => "Author",
        (L::EnUs, F::Genre) => "Genre",
        (L::EnUs, F::Released) => "Release date",
        (L::EnUs, F::Duration) => "Length",
        (L::EnUs, F::Bars) => "Bars",
        (L::EnUs, F::Tracks) => "Tracks",
        (L::EnUs, F::Description) => "About",
        (L::EnUs, F::Credits) => "Credits",
        (L::EnUs, F::Copyright) => "Copyright",
        (L::EnUs, F::License) => "License",
        (L::EnUs, F::Website) => "Website",
        (L::FrFr, F::Author) => "Artiste",
        (L::FrFr, F::Genre) => "Genre",
        (L::FrFr, F::Released) => "Date de sortie",
        (L::FrFr, F::Duration) => "Durée",
        (L::FrFr, F::Bars) => "Mesures",
        (L::FrFr, F::Tracks) => "Pistes",
        (L::FrFr, F::Description) => "Présentation",
        (L::FrFr, F::Credits) => "Crédits",
        (L::FrFr, F::Copyright) => "Droits d'auteur",
        (L::FrFr, F::License) => "Licence",
        (L::FrFr, F::Website) => "Site web",
        (L::EsEs, F::Author) => "Artista",
        (L::EsEs, F::Genre) => "Género",
        (L::EsEs, F::Released) => "Fecha de lanzamiento",
        (L::EsEs, F::Duration) => "Duración",
        (L::EsEs, F::Bars) => "Compases",
        (L::EsEs, F::Tracks) => "Pistas",
        (L::EsEs, F::Description) => "Sinopsis",
        (L::EsEs, F::Credits) => "Créditos",
        (L::EsEs, F::Copyright) => "Derechos de autor",
        (L::EsEs, F::License) => "Licencia",
        (L::EsEs, F::Website) => "Sitio web",
        (L::PtBr, F::Author) => "Artista",
        (L::PtBr, F::Genre) => "Gênero",
        (L::PtBr, F::Released) => "Data de lançamento",
        (L::PtBr, F::Duration) => "Duração",
        (L::PtBr, F::Bars) => "Compassos",
        (L::PtBr, F::Tracks) => "Faixas",
        (L::PtBr, F::Description) => "Sinopse",
        (L::PtBr, F::Credits) => "Créditos",
        (L::PtBr, F::Copyright) => "Direitos autorais",
        (L::PtBr, F::License) => "Licença",
        (L::PtBr, F::Website) => "Site",
        (L::JaJp, F::Author) => "アーティスト",
        (L::JaJp, F::Genre) => "ジャンル",
        (L::JaJp, F::Released) => "リリース日",
        (L::JaJp, F::Duration) => "長さ",
        (L::JaJp, F::Bars) => "小節",
        (L::JaJp, F::Tracks) => "トラック",
        (L::JaJp, F::Description) => "紹介",
        (L::JaJp, F::Credits) => "クレジット",
        (L::JaJp, F::Copyright) => "著作権",
        (L::JaJp, F::License) => "ライセンス",
        (L::JaJp, F::Website) => "ウェブサイト",
        (L::KoKr, F::Author) => "아티스트",
        (L::KoKr, F::Genre) => "장르",
        (L::KoKr, F::Released) => "발매일",
        (L::KoKr, F::Duration) => "길이",
        (L::KoKr, F::Bars) => "마디",
        (L::KoKr, F::Tracks) => "트랙",
        (L::KoKr, F::Description) => "소개",
        (L::KoKr, F::Credits) => "크레딧",
        (L::KoKr, F::Copyright) => "저작권",
        (L::KoKr, F::License) => "라이선스",
        (L::KoKr, F::Website) => "웹사이트",
        (L::RuRu, F::Author) => "Автор",
        (L::RuRu, F::Genre) => "Жанр",
        (L::RuRu, F::Released) => "Дата выпуска",
        (L::RuRu, F::Duration) => "Длительность",
        (L::RuRu, F::Bars) => "Такты",
        (L::RuRu, F::Tracks) => "Дорожки",
        (L::RuRu, F::Description) => "Аннотация",
        (L::RuRu, F::Credits) => "Создатели",
        (L::RuRu, F::Copyright) => "Авторские права",
        (L::RuRu, F::License) => "Лицензия",
        (L::RuRu, F::Website) => "Сайт",
        (L::DeDe, F::Author) => "Künstler",
        (L::DeDe, F::Genre) => "Genre",
        (L::DeDe, F::Released) => "Veröffentlichungsdatum",
        (L::DeDe, F::Duration) => "Dauer",
        (L::DeDe, F::Bars) => "Takte",
        (L::DeDe, F::Tracks) => "Spuren",
        (L::DeDe, F::Description) => "Kurzbeschreibung",
        (L::DeDe, F::Credits) => "Mitwirkende",
        (L::DeDe, F::Copyright) => "Urheberrecht",
        (L::DeDe, F::License) => "Lizenz",
        (L::DeDe, F::Website) => "Website",
    }
}

/// 卡片「详情」展开区的带标签行（专辑详情页口径，Wang
/// 2026-11-23 定）：按 作者 / 曲风 / 发布日期 / 时长 / 小节数 /
/// 轨道数 / 简介 / 制作名单 / 版权声明 / 许可协议 / 作者网站 的
/// 顺序排列，**有数据才出现该行**，无数据整行不出现、不留空
/// 占位。数据来源：作者与时长 / 小节数 / 轨道数是容器实读实算；
/// 其余字段容器没有，统一走旁挂 meta（见 [`DemoMeta`]）。
#[must_use]
pub fn demo_detail_rows(lang: Language, entry: &DemoEntry) -> Vec<DemoDetailRow> {
    let mut rows: Vec<DemoDetailRow> = Vec::new();
    let mut push = |field: DetailField, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            rows.push(DemoDetailRow {
                label: detail_field_label(lang, field).to_owned(),
                value,
            });
        }
    };
    push(DetailField::Author, entry.author.clone());
    push(DetailField::Genre, entry.genre.clone());
    push(DetailField::Released, entry.meta.released.clone());
    if entry.duration_secs > 0.0 {
        push(
            DetailField::Duration,
            Some(duration_clock(entry.duration_secs)),
        );
    }
    if entry.bars > 0 {
        push(DetailField::Bars, Some(entry.bars.to_string()));
    }
    push(DetailField::Tracks, Some(entry.track_count.to_string()));
    push(DetailField::Description, entry.meta.description.clone());
    push(DetailField::Credits, entry.meta.credits.clone());
    push(DetailField::Copyright, entry.meta.copyright.clone());
    push(DetailField::License, entry.meta.license.clone());
    push(DetailField::Website, entry.meta.website.clone());
    rows
}

/// 展开区高度估算（px，界面层据此把卡片长高、本行架板与下方
/// 专辑下移）：与 .slint 侧展开区的版式常数对账——分界线 1 +
/// 线后间距 4 + 每行（估算行数 × 18）+ 行间距 4 × (n − 1) +
/// 底部留白 10。值列可用宽 = 卡内宽 248 − 标签列 72 − 间距 8
/// = 168px；12px 字体的字宽估算：宽字（CJK 等）12px、窄字
/// 6px，即每行约 28 个窄字单位。估算偏大只多留白、偏小会裁
/// 字，宁可略偏大。
fn detail_rows_height_px(rows: &[DemoDetailRow]) -> u32 {
    if rows.is_empty() {
        return 0;
    }
    let mut total: u32 = 1 + 4 + 10;
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            total += 4;
        }
        let units: u32 = row
            .value
            .chars()
            .map(|c| if is_wide_char(c) { 2 } else { 1 })
            .sum();
        let lines = units.max(1).div_ceil(28);
        total += lines * 18;
    }
    total
}

/// 字宽估算用的宽字判定（CJK 表意 / 假名 / 谚文 / 全角段）。
fn is_wide_char(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}'   // 谚文字母
        | '\u{2E80}'..='\u{9FFF}' // CJK 部首 / 假名 / 表意
        | '\u{A000}'..='\u{A4CF}' // 彝文
        | '\u{AC00}'..='\u{D7A3}' // 谚文音节
        | '\u{F900}'..='\u{FAFF}' // CJK 兼容表意
        | '\u{FF00}'..='\u{FF60}' // 全角形
        | '\u{20000}'..='\u{2FA1F}')
}

/// 时长时钟写法（`4:32`，语言中立；带标签的整句见
/// [`duration_label`]）。
#[must_use]
pub fn duration_clock(secs: f64) -> String {
    let total = secs.round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// 「N 小节」（演示条目「详情」，九语闭集）。
#[must_use]
pub fn bars_label(lang: Language, count: usize) -> String {
    match lang {
        Language::ZhCn => format!("{count} 小节"),
        Language::EnUs => format!("{count} bars"),
        Language::FrFr => format!("{count} mesures"),
        Language::EsEs => format!("{count} compases"),
        Language::PtBr => format!("{count} compassos"),
        Language::JaJp => format!("{count} 小節"),
        Language::KoKr => format!("{count}마디"),
        Language::RuRu => format!("тактов: {count}"),
        Language::DeDe => format!("{count} Takte"),
    }
}

/// 「时长 M:SS」（演示条目「详情」，九语闭集；时分秒写法语言中立）。
#[must_use]
pub fn duration_label(lang: Language, secs: f64) -> String {
    let clock = duration_clock(secs);
    match lang {
        Language::ZhCn => format!("时长 {clock}"),
        Language::EnUs => format!("Length {clock}"),
        Language::FrFr => format!("Durée {clock}"),
        Language::EsEs => format!("Duración {clock}"),
        Language::PtBr => format!("Duração {clock}"),
        Language::JaJp => format!("長さ {clock}"),
        Language::KoKr => format!("길이 {clock}"),
        Language::RuRu => format!("длительность {clock}"),
        Language::DeDe => format!("Dauer {clock}"),
    }
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
            genre: String::new(),
            bpm_label: String::new(),
            details: String::new(),
            detail_rows: Vec::new(),
            details_height_px: 0,
            entry_index: Some(index),
            enabled: true,
            cover: None,
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
            assert!(!bars_label(lang, 3).is_empty());
            assert!(!duration_label(lang, 90.0).is_empty());
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
