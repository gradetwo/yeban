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
//!
//! ### 官方 demo 的独立口径（T-FIX-641）
//!
//! [`crate::demo_download::DEMOS`] 是官方 8 首 demo 的**固定清单**
//! （名字 / 工程文件名 / 封面文件名）。下载落地目录里的 `.yeban`
//! 文件名命中固定清单 ⇒ 这条是**官方 demo**：卡片从固定清单取标题、
//! 从同名旁挂 `<stem>.meta.json` 取作者 / 轨道数 / BPM / 小节 /
//! 时长 / 曲风等展示字段，**⛔ 不读整包**，也不受
//! [`DEMO_LIST_MAX_BYTES`] 约束（官方 demo 23–269 MB，全部超过该闸）。
//! 不在固定清单里的 `.yeban` 是用户手工放入的文件：照旧走体积闸，
//! 超过 [`DEMO_LIST_MAX_BYTES`] 就只进 [`ChooserData::blocked_demos`]
//! 并写明原因（置灰行对用户可见，可从「打开工程」直接打开）。
//! 官方 demo 的旁挂元数据缺失 ⇒ 对应字段留空/为 0，⛔ 不猜。
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

/// 选择器行族在 [`crate::elements`] 注册表里登记的**代表窗口**（`startup-row-{0..N}` 等）。
///
/// T-FIX-293 起行模型**无界**：宿主把全部行写进模型，界面用 `Flickable` 纵向滚动
/// 到达任意一行，`hidden_rows` 恒为 0（⛔ 不再按此截断）。这个常数不再限制行数，
/// 只给注册表一个枚举窗口 —— 覆盖判据按 `.slint` 的 `"startup-row-" + row-index`
/// 等模板匹配，与窗口大小无关；运行时第 65 行起与前 64 行共享同一模板、同一 `kind`
/// 与同一 `accessible-action-default` 落点。⛔ 残留缺口（登记在 T-FIX-293 回执）：
/// 若将来有判据对打开的、超过 64 行的选择器做运行时 `coverage_against` 断言，
/// 第 65 行起会报 `unknown_at_runtime` —— 今天没有这样的判据。
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
    /// 封面三态（文件型 demo 的同名图片；T-FIX-541）。
    pub cover: DemoCover,
    /// 官方 demo 的下载态（T-FIX-951）；本地 demo 为 `None`。
    pub download: Option<DemoDownloadState>,
}

/// 官方 demo 的下载态（T-FIX-951）：只有命中
/// [`crate::demo_download::DEMOS`] 固定清单的条目才有值；用户自己放进
/// `YEBAN_DEMOS_DIR` 的普通 demo 恒为 `None`（它们是本地文件，没有下载这一步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoDownloadState {
    /// 未下载：可点「下载」。
    NotDownloaded,
    /// 下载中：已收字节 / 总量（`None` = 服务端没给 Content-Length）。
    Downloading {
        /// 已收字节数。
        received: u64,
        /// 总字节数（`None` = 服务端没给 Content-Length）。
        total: Option<u64>,
    },
    /// 失败：人读原因，可重试。
    Failed(String),
    /// 已下载：正式文件就位，可直接打开播放。
    Downloaded,
}

impl DemoDownloadState {
    /// .slint 侧整数编码：0 = 无下载概念，1 = 可下载，2 = 下载中，
    /// 3 = 失败（可重试），4 = 已下载。
    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            Self::NotDownloaded => 1,
            Self::Downloading { .. } => 2,
            Self::Failed(_) => 3,
            Self::Downloaded => 4,
        }
    }

    /// 进度 / 回执原文（总量未知时明说，⛔ 不编一个假数）。
    #[must_use]
    pub fn progress_text(&self) -> String {
        fn bytes(value: u64) -> String {
            const KB: f64 = 1024.0;
            const MB: f64 = 1024.0 * 1024.0;
            if value >= 1024 * 1024 {
                format!("{:.1} MB", value as f64 / MB)
            } else if value >= 1024 {
                format!("{:.1} KB", value as f64 / KB)
            } else {
                format!("{value} B")
            }
        }
        match self {
            Self::Downloading { received, total } => match total {
                Some(total) => format!("{} / {}", bytes(*received), bytes(*total)),
                None => format!("{} / 总量未知", bytes(*received)),
            },
            Self::Failed(message) => message.clone(),
            _ => String::new(),
        }
    }
}

/// 演示封面的三态（T-FIX-541）：没有封面文件 / 有且解码成功 / 有但解码失败。
///
/// 两态（`Option<PathBuf>`）只能表达"有没有文件"，不能表达"解没解开" ——
/// 坏图与"本来没封面"在界面上长得一模一样。失败态带原因，详情里必须看得见。
#[derive(Debug, Clone, PartialEq)]
pub enum DemoCover {
    /// 没有同名图片。
    None,
    /// 有同名图片，宿主侧解码成功。
    Ready(PathBuf),
    /// 有同名图片，但解码失败（坏图 / 格式不支持 / 读不了）；`reason` 是原样原因。
    Failed {
        /// 解码失败的图片路径。
        path: PathBuf,
        /// 失败原因（解码器原文，⛔ 不吞、⛔ 不编造友好文案）。
        reason: String,
    },
}

impl DemoCover {
    /// 有封面文件时返回它的路径（失败态也给，便于日志/回执指认）。
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            DemoCover::None => None,
            DemoCover::Ready(path) | DemoCover::Failed { path, .. } => Some(path),
        }
    }

    /// 解码失败时返回原因（成功 / 无封面 ⇒ `None`）。
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        match self {
            DemoCover::Failed { reason, .. } => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// 演示清单阶段单文件**整体读盘**的上限（字节）。
///
/// 列清单只需要标题 / BPM / 轨道数 / 跨度，⛔ 不该为它把整包读进内存：
/// 超过本上限的 `.yeban` 不进 [`ChooserData::demos`]，而是带着原因进
/// [`ChooserData::blocked_demos`]，在选择器里显示成一条置灰行（用户仍可
/// 从「打开工程」按路径直接打开它）。上限取 8 MiB，与模板预检同一数量级
/// —— 只覆盖真正的"小 demo"，带音频资产的归档一律不在清单阶段读。
///
/// T-FIX-641：**官方 demo**（文件名命中 [`crate::demo_download::DEMOS`]
/// 固定清单）不受本闸约束 —— 它们的卡片由固定清单 + 旁挂元数据生成，
/// 清单阶段同样不读整包；本闸只保护用户手工放入目录的任意 `.yeban`。
pub const DEMO_LIST_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 官方 demo 的固定清单（见 [`crate::demo_download::DEMOS`]）：按文件名
/// 认领，大小写敏感、只认 `.yeban` 的文件名本身（不认目录层级）。
///
/// 为什么按文件名而不是内容哈希：清单阶段不许读整包（官方 demo 最大
/// 269 MB），而下载落地时文件名就是 [`crate::demo_download::DemoSpec::file`]
/// （下载执行器用同一份固定清单拼目标路径），所以文件名是清单阶段唯一
/// 不需要读内容就能拿到的身份。用户把手放同名文件也只是得到"官方卡片"
/// 这一条外观，打开时仍走真实归档解析、坏容器照旧响亮报错。
fn official_demo_spec(path: &Path) -> Option<&'static crate::demo_download::DemoSpec> {
    if path.extension().is_none_or(|ext| ext != "yeban") {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    crate::demo_download::DEMOS
        .iter()
        .find(|spec| spec.file == name)
}

/// 官方 demo 的封面：固定清单里的封面文件名优先（下载落地时与工程同
/// 目录）；文件不在 ⇒ 回落到与普通 demo 同款的同名图规则，⛔ 不编造路径。
fn official_demo_cover(path: &Path, spec: &crate::demo_download::DemoSpec) -> Option<PathBuf> {
    let cover = path.with_file_name(spec.cover);
    if cover.is_file() {
        return Some(cover);
    }
    same_stem_cover(path)
}

/// 与文件同名的封面候选（先到先得）：同名图、同名 + `-cover` 后缀图。
fn same_stem_cover(path: &Path) -> Option<PathBuf> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    ["png", "jpg"]
        .iter()
        .flat_map(|ext| {
            [
                path.with_extension(ext),
                path.with_file_name(format!("{stem}-cover.{ext}")),
            ]
        })
        .find(|candidate| candidate.is_file())
}

/// 一条没能进演示清单的条目（路径 + 显示名 + 原因）。
///
/// 存在理由：旧行为是 `eprintln!` 后 `continue` —— 用户只看得到"演示工程为空"
/// 或条目凭空消失，分不清是文件坏了、目录没权限，还是本来就没有 demo。这里把
/// 每条失败都留在结构化结果里，选择器用它渲染置灰行（`enabled = false`，
/// 点不动），⛔ 不再只写 stderr。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedDemo {
    /// 出问题的路径（文件或目录）。
    pub path: PathBuf,
    /// 置灰行的主文字：文件名（目录级失败时是目录名）。
    /// ⚠ 不是从容器读出的标题 —— 读不到元数据时只能显示文件名，行副文字
    /// 会同时写明原因，⛔ 不把它当成工程标题冒充。
    pub label: String,
    /// 为什么没进清单（读不了 / 超过列表上限 / 不是可读容器 / 目录不可读）。
    pub reason: String,
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
    /// 没能进演示清单的条目（每条带路径与原因，选择器渲染成置灰行）。
    /// ⛔ 不是"静默跳过"的等价物：这些行对用户可见、可读原因。
    pub blocked_demos: Vec<BlockedDemo>,
    /// 全部模板（工厂在前，与 [`list_templates`] 同序）。
    pub templates: Vec<TemplateEntry>,
    /// 模板扫描被跳过的路径 / 原因（T-ISS-MWXZ1PQZ）。
    ///
    /// 非空 ⇒ 启动选择器的模板分类必须把它们渲染成置灰警告行，不能只取
    /// `templates` 把诊断丢掉。
    pub template_diagnostics: Vec<String>,
}

/// 汇集选择器数据：最近清单来自设置（存在性当场查），演示来自
/// 文件型 demo 目录的当场扫描，模板来自两个模板根目录的当场扫描。
#[must_use]
pub fn gather(settings: &Settings, roots: &TemplateRoots) -> ChooserData {
    let recent = settings
        .recent_projects
        .iter()
        // ⭐ T-FIX-312：死行不进选择器。设置层 `load()` 会在启动时 prune ＋
        // 写回；这里再 filter 一次是第二道闸（防止调用方直接传未经 load 的
        // Settings，或文件在两次读之间被删/移动）。
        .filter(|entry: &&RecentProject| entry.path.is_file())
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
    let mut blocked_demos: Vec<BlockedDemo> = Vec::new();
    if let Some(dir) = std::env::var_os(DEMOS_DIR_ENV)
        && !dir.is_empty()
    {
        let scan = scan_file_demos(Path::new(&dir));
        demos.extend(scan.entries);
        blocked_demos.extend(scan.blocked);
    }
    // ⭐ T-FIX-951：官方 8 首 demo 常驻清单 —— 即使 `YEBAN_DEMOS_DIR` 没设、
    // 本地一首都没有，演示分类也显示这 8 条带下载入口的卡片，⛔ 不再是
    // "还没有演示工程。" 的空态。已下载的走官方元数据卡片（不读整包），
    // 没下载的走固定清单占位卡片；两条路的 `source` 都指向下载落地路径。
    let demo_root = crate::demo_download::cache_root();
    for spec in crate::demo_download::DEMOS {
        let path = crate::demo_download::cached_project(&demo_root, &spec);
        if demos
            .iter()
            .any(|entry| matches!(&entry.source, DemoSource::File(existing) if *existing == path))
        {
            continue;
        }
        if path.is_file() {
            demos.push(load_official_demo_entry(&path, &spec));
        } else {
            demos.push(pending_official_demo_entry(&spec));
        }
    }
    let templates = list_templates(roots);
    ChooserData {
        recent,
        demos,
        blocked_demos,
        templates: templates.entries,
        template_diagnostics: templates.diagnostics,
    }
}

/// [`scan_file_demos`] 的结构化结果：进清单的条目 + 没进的（含原因）。
#[derive(Debug, Clone, PartialEq)]
struct DemoScan {
    entries: Vec<DemoEntry>,
    blocked: Vec<BlockedDemo>,
}

/// 扫描文件型 demo 目录（实验）：每个 `*.yeban` 文件一条，标题与
/// 轨道数从容器真实读取；读不出的文件**响亮报错并进 `blocked`**（⛔ 不
/// 编造、⛔ 不只写 stderr）；与工程同名的 `.png` / `.jpg` 是封面。条目按
/// 路径排序，⛔ 不依赖 `read_dir` 的文件系统顺序。
///
/// 两处**不读整包**的闸：目录读不了 ⇒ 一条目录级 `BlockedDemo`；单文件
/// 体积超过 [`DEMO_LIST_MAX_BYTES`] ⇒ 不进清单、不读内容，只留一条写明
/// 上限的 `BlockedDemo`（读整包的时机是用户点选打开，不是列清单）。
fn scan_file_demos(dir: &Path) -> DemoScan {
    let mut blocked: Vec<BlockedDemo> = Vec::new();
    let read_dir = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            let reason = format!("演示目录读不了: {error}");
            eprintln!(
                "startup-chooser: 文件型 demo 目录 `{}` {reason}",
                dir.display()
            );
            blocked.push(BlockedDemo {
                path: dir.to_path_buf(),
                label: dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned(),
                reason,
            });
            return DemoScan {
                entries: Vec::new(),
                blocked,
            };
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
                let reason = format!("演示目录有条目读不了: {error}");
                eprintln!(
                    "startup-chooser: 文件型 demo 目录 `{}` {reason}",
                    dir.display()
                );
                blocked.push(BlockedDemo {
                    path: dir.to_path_buf(),
                    label: dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_owned(),
                    reason,
                });
            }
        }
    }
    paths.sort();
    let mut entries = Vec::new();
    for path in paths {
        // T-FIX-641：官方 demo 先按固定清单认领 —— 命中就不进整包读盘，
        // 也不受体积闸约束（见 [`official_demo_spec`]）。
        if let Some(spec) = official_demo_spec(&path) {
            entries.push(load_official_demo_entry(&path, spec));
            continue;
        }
        match load_demo_entry(&path) {
            Ok(entry) => entries.push(entry),
            Err(message) => {
                eprintln!("startup-chooser: 文件型 demo 未进清单: {message}");
                blocked.push(BlockedDemo {
                    path: path.clone(),
                    label: path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or_default()
                        .to_owned(),
                    reason: message,
                });
            }
        }
    }
    DemoScan { entries, blocked }
}

/// 从磁盘真实读取一条文件型 demo 的标题与轨道数（整包读容器 ——
/// 实验口径，见回执）。读不出 ⇒ `Err` 写明原因，调用方把它转成
/// [`BlockedDemo`]（置灰行）而不是静默跳过。体积超过
/// [`DEMO_LIST_MAX_BYTES`] 的文件在这里就被拒，⛔ 不进 `fs::read`。
fn load_demo_entry(path: &Path) -> Result<DemoEntry, String> {
    // 列清单前先量体积：超过上限的归档**一个字节都不读**（否则一个坏/巨
    // 文件就能把 UI 线程占住，容器 limits 也拦不住"已经读进内存"这件事）。
    let len = std::fs::metadata(path)
        .map_err(|error| format!("`{}` 读不了: {error}", path.display()))?
        .len();
    if len > DEMO_LIST_MAX_BYTES {
        return Err(format!(
            "`{}` 有 {len} 字节，超过清单阶段读盘上限 {DEMO_LIST_MAX_BYTES} 字节，\
             未读取元数据；请从「打开工程」直接打开它",
            path.display()
        ));
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("`{}` 读不了: {error}", path.display()))?;
    let archive = read_project_container(&bytes, &ContainerLimits::default())
        .map_err(|error| format!("`{}` 不是可读的工程容器: {error}", path.display()))?;
    let project = &archive.project;
    // 封面候选（先到先得）：同名图、同名 + `-cover` 后缀图。
    let cover = same_stem_cover(path);
    let (bars, duration_secs) = demo_span(project);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
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
        cover: cover.map_or(DemoCover::None, DemoCover::Ready),
        download: None,
    })
}

/// 官方 demo 的清单条目（T-FIX-641）：**不读工程包**，卡片字段全部来自
/// 固定清单（标题 / 封面文件名）与同名旁挂 `<stem>.meta.json`
/// （作者 / 轨道数 / BPM / 小节 / 时长 / 曲风 / 专辑页字段）。
///
/// 与 [`load_demo_entry`] 的差别只有"元数据从哪来"：官方 demo 是
/// 23–269 MB 的整包，清单阶段读不起（见 [`DEMO_LIST_MAX_BYTES`] 的
/// 文档）。旁挂缺字段 ⇒ 留空/0，⛔ 不从文件名或封面猜内容。
fn load_official_demo_entry(path: &Path, spec: &crate::demo_download::DemoSpec) -> DemoEntry {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let meta = read_sidecar_meta(path, stem);
    let cover = official_demo_cover(path, spec);
    DemoEntry {
        source: DemoSource::File(path.to_path_buf()),
        title: spec.title.to_owned(),
        track_count: meta.track_count.unwrap_or(0),
        author: meta.author.clone(),
        genre: meta.genre.clone(),
        bpm: meta.bpm.unwrap_or(0.0),
        bars: meta.bars.unwrap_or(0),
        duration_secs: meta.duration_secs.unwrap_or(0.0),
        meta,
        cover: cover.map_or(DemoCover::None, DemoCover::Ready),
        download: Some(DemoDownloadState::Downloaded),
    }
}

/// 还没下载的官方 demo 卡片（T-FIX-951）：只来自固定清单，**不读包**。
///
/// `source` 指向下载落地后的正式路径，因此下载成功回调把 `download` 改成
/// `Downloaded` 之后，现有的「打开」臂无需改动就能直接打开播放；未下载时
/// 卡片带 `NotDownloaded`，界面给出下载入口而不是一个点了就报错的路径。
fn pending_official_demo_entry(spec: &crate::demo_download::DemoSpec) -> DemoEntry {
    let path = crate::demo_download::cached_project(&crate::demo_download::cache_root(), spec);
    DemoEntry {
        source: DemoSource::File(path),
        title: spec.title.to_owned(),
        track_count: 0,
        author: None,
        genre: None,
        meta: DemoMeta::default(),
        bpm: 0.0,
        bars: 0,
        duration_secs: 0.0,
        cover: DemoCover::None,
        download: Some(DemoDownloadState::NotDownloaded),
    }
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
    // T-FIX-168：每小节 tick = PPQ × 4 × 分子 ÷ 分母。分母不是固定 4 —— 6/8 的一小节
    // 是 6 个八分音符（3 拍 × 2），按旧式「只乘分子」会算成 6 个四分音符 ⇒ 小节数减半。
    let numerator = u64::from(project.time_signature.numerator.max(1));
    let denominator = u64::from(project.time_signature.denominator.max(1));
    let ticks_per_bar = yeban_model::ids::PPQ
        .saturating_mul(4)
        .saturating_mul(numerator)
        / denominator;
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
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// 作者名（官方 demo 清单阶段不读容器，作者由此提供）。
    pub author: Option<String>,
    /// 非主总线轨道数（官方 demo 的卡片「详情」用）。
    pub track_count: Option<usize>,
    /// 工程速度（官方 demo 清单阶段不读容器，速度由此提供）。
    pub bpm: Option<f64>,
    /// 全曲小节数（官方 demo 清单阶段不读容器，跨度由此提供）。
    pub bars: Option<usize>,
    /// 全曲时长（秒）。
    pub duration_secs: Option<f64>,
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
    let integer = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|number| usize::try_from(number).ok())
    };
    let number = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .filter(|number| number.is_finite() && *number >= 0.0)
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
            author: field(&value, "author").and_then(|name| author_name(&name)),
            track_count: integer(&value, "trackCount"),
            bpm: number(&value, "bpm"),
            bars: integer(&value, "bars"),
            duration_secs: number(&value, "durationSecs"),
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
    /// 官方 demo 的下载态（T-FIX-951），随行灌给 Slint 卡片/面板；其余行 `None`。
    pub download: Option<DemoDownloadState>,
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
        download: None,
    }
}

/// 搜索串的子串匹配（T-FIX-293）：空串命中全部；非空时不区分大小写、子串匹配。
#[must_use]
fn query_matches(haystack: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || haystack.to_lowercase().contains(&query)
}

/// 「最近」分类的行模型：一条一工程，副文字 = 路径 + 相对时间；
/// 文件已不存在的条目副文字换成「文件已不存在」，且不可点。
#[must_use]
pub fn recent_rows(entries: &[RecentEntry], lang: Language, now_ms: u64) -> Vec<ChooserRow> {
    recent_rows_matching(entries, lang, now_ms, "")
}

/// [`recent_rows`] 的带搜索过滤版（T-FIX-293）：只放名字或路径命中 `query` 的条目；
/// `entry_index` 仍是**全量最近清单**的下标（宿主按它打开文件），⛔ 不因过滤重排。
#[must_use]
pub fn recent_rows_matching(
    entries: &[RecentEntry],
    lang: Language,
    now_ms: u64,
    query: &str,
) -> Vec<ChooserRow> {
    let mut rows = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if !query_matches(&format!("{} {}", entry.name, entry.path.display()), query) {
            continue;
        }
        let detail = if entry.exists {
            format!(
                "{} · {}",
                entry.path.display(),
                relative_time_label(lang, now_ms, entry.opened_at_unix_ms)
            )
        } else {
            format!("{} · {}", entry.path.display(), missing_file_label(lang))
        };
        rows.push(ChooserRow {
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
            download: None,
        });
    }
    rows
}

/// 「演示工程」分类的行模型（唱片架卡片口径，Wang 2026-11-23
/// 定）：主面只留标题、曲风与弱化的 BPM；专辑页字段只进「详情」
/// 展开区（带标签行，见 [`demo_detail_rows`]），⛔ 不上主面。
#[must_use]
pub fn demo_rows(entries: &[DemoEntry], lang: Language) -> Vec<ChooserRow> {
    demo_rows_matching(entries, lang, "")
}

/// [`demo_rows`] 的带搜索过滤版（T-FIX-293）：只放标题或曲风命中 `query` 的条目；
/// `entry_index` 仍是**全量演示清单**的下标（宿主按它打开文件），⛔ 不因过滤重排。
#[must_use]
pub fn demo_rows_matching(entries: &[DemoEntry], lang: Language, query: &str) -> Vec<ChooserRow> {
    demo_rows_matching_with_blocked(entries, &[], lang, query)
}

/// [`demo_rows_matching`] 的带"未进清单条目"版：正常条目之后追加每条
/// [`BlockedDemo`] 的置灰行（`enabled = false` ⇒ 界面层点不动、不给打开
/// 回调），副文字写原因，`entry_index = None`（没有可打开的下标）。
/// 搜索同样过滤这些行（按显示名 + 原因），⛔ 不让坏条目从搜索里消失。
#[must_use]
pub fn demo_rows_matching_with_blocked(
    entries: &[DemoEntry],
    blocked: &[BlockedDemo],
    lang: Language,
    query: &str,
) -> Vec<ChooserRow> {
    let mut rows = demo_rows_matching_entries(entries, lang, query);
    for blocked_entry in blocked {
        if !query_matches(
            &format!("{} {}", blocked_entry.label, blocked_entry.reason),
            query,
        ) {
            continue;
        }
        rows.push(ChooserRow {
            kind: ChooserRowKind::Entry,
            label: blocked_entry.label.clone(),
            detail: blocked_entry.reason.clone(),
            genre: String::new(),
            bpm_label: String::new(),
            details: blocked_entry.reason.clone(),
            detail_rows: Vec::new(),
            details_height_px: 0,
            entry_index: None,
            enabled: false,
            cover: None,
            download: None,
        });
    }
    rows
}

fn demo_rows_matching_entries(
    entries: &[DemoEntry],
    lang: Language,
    query: &str,
) -> Vec<ChooserRow> {
    let mut rows = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if !query_matches(
            &format!(
                "{} {}",
                entry.title,
                entry.genre.clone().unwrap_or_default()
            ),
            query,
        ) {
            continue;
        }
        let detail_rows = demo_detail_rows(lang, entry);
        rows.push(ChooserRow {
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
            cover: match &entry.cover {
                DemoCover::Ready(path) => Some(path.clone()),
                _ => None,
            },
            download: entry.download.clone(),
        });
    }
    rows
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
    /// 封面读不了（T-FIX-541；只在解码失败时出现，⛔ 没有封面不列）。
    CoverError,
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
        (L::ZhCn, F::CoverError) => "封面读不了",
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
        (L::EnUs, F::CoverError) => "Cover unreadable",
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
        (L::FrFr, F::CoverError) => "Couverture illisible",
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
        (L::EsEs, F::CoverError) => "Portada ilegible",
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
        (L::PtBr, F::CoverError) => "Capa ilegível",
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
        (L::JaJp, F::CoverError) => "カバーを読み込めません",
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
        (L::KoKr, F::CoverError) => "커버를 읽을 수 없음",
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
        (L::RuRu, F::CoverError) => "Обложку не удалось прочитать",
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
        (L::DeDe, F::CoverError) => "Cover nicht lesbar",
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
    // ⭐ T-FIX-541：封面解码失败必须在详情里看得见（⛔ 只写默认关闭的 module 回执）。
    push(
        DetailField::CoverError,
        entry.cover.failure().map(str::to_owned),
    );
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
    // 空串不过滤，`lang` 不会被查（见 `entry_matches_query` 的短路）。
    template_rows_matching(entries, source, Language::ZhCn, "")
}

/// [`template_rows`] 的带搜索过滤版（T-FIX-293）：只放来源 / 曲风 / 子曲风 / 模板名
/// 命中 `query` 的条目（匹配口径与模板选择器同款，见
/// [`crate::templates::entry_matches_query`]）；分组标题只在它名下出现第一条
/// 命中条目时才出现；`entry_index` 仍是**全量模板清单**的下标，⛔ 不因过滤重排。
#[must_use]
pub fn template_rows_matching(
    entries: &[TemplateEntry],
    source: TemplateSource,
    lang: Language,
    query: &str,
) -> Vec<ChooserRow> {
    let mut rows = Vec::new();
    let mut current_style: Option<&str> = None;
    let mut current_substyle: Option<&str> = None;
    for (index, entry) in entries.iter().enumerate() {
        if entry.source != source {
            continue;
        }
        if !crate::templates::entry_matches_query(entry, lang, query) {
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
            download: None,
        });
    }
    rows
}

/// [`template_rows_matching`] ＋ 模板扫描诊断行（T-ISS-MWXZ1PQZ）。
///
/// 诊断行是置灰的普通条目行：`entry_index = None`（不可打开），`detail`
/// 原样写扫描器给的路径 / 原因。正常条目仍按原序在前。
#[must_use]
pub fn template_rows_matching_with_diagnostics(
    entries: &[TemplateEntry],
    diagnostics: &[String],
    source: TemplateSource,
    lang: Language,
    query: &str,
) -> Vec<ChooserRow> {
    let mut rows = template_rows_matching(entries, source, lang, query);
    for diagnostic in diagnostics {
        rows.push(ChooserRow {
            kind: ChooserRowKind::Entry,
            label: template_scan_warning_label(lang).to_owned(),
            detail: diagnostic.clone(),
            genre: String::new(),
            bpm_label: String::new(),
            details: String::new(),
            detail_rows: Vec::new(),
            details_height_px: 0,
            entry_index: None,
            enabled: false,
            cover: None,
            download: None,
        });
    }
    rows
}

fn template_scan_warning_label(lang: Language) -> &'static str {
    use Language::{DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
    match lang {
        ZhCn => "模板扫描警告",
        EnUs => "Template scan warning",
        FrFr => "Avertissement d'analyse des modèles",
        EsEs => "Advertencia de escaneo de plantillas",
        PtBr => "Aviso de varredura de modelos",
        JaJp => "テンプレート走査の警告",
        KoKr => "템플릿 스캔 경고",
        RuRu => "Предупреждение сканирования шаблонов",
        DeDe => "Vorlagen-Scanwarnung",
    }
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

    /// 检查 Details 是否在模型合法域内；越界时**响亮**返回原因，
    /// 交给启动选择器留在 Details 页显示，而不是静默钳值。
    ///
    /// 与 [`Self::sanitized`] 的分工：`sanitized` 仍是防御性收口（供
    /// 程序内部兜底），用户输入的路径必须先过这里。
    ///
    /// # Errors
    ///
    /// 速度非有限 / 越出 20–999、拍号越界或分母不在 `1/2/4/8/16/32`、
    /// 调号升降号越出 −7..=7。
    pub fn validate(&self) -> Result<Self, String> {
        if !self.bpm.is_finite() {
            return Err(format!(
                "速度「{}」不是有限数值，请输入 20–999 的数字",
                self.bpm
            ));
        }
        if !(20.0..=999.0).contains(&self.bpm) {
            return Err(format!(
                "速度「{}」超出允许范围 20–999，新工程没有创建",
                self.bpm
            ));
        }
        if !(1..=32).contains(&self.time_signature.numerator) {
            return Err(format!(
                "拍号分子「{}」超出 1–32，新工程没有创建",
                self.time_signature.numerator
            ));
        }
        if ![1, 2, 4, 8, 16, 32].contains(&self.time_signature.denominator) {
            return Err(format!(
                "拍号分母「{}」不在 1/2/4/8/16/32 之内，新工程没有创建",
                self.time_signature.denominator
            ));
        }
        if let Some(key) = &self.key_signature
            && !(-7..=7).contains(&key.sharps_flats)
        {
            return Err(format!(
                "调号升降号「{}」超出 −7..=7，新工程没有创建",
                key.sharps_flats
            ));
        }
        Ok(*self)
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
    /// T-FIX-168：小节数必须按拍号**分母**换算（6/8 末端 6×PPQ ⇒ 2 小节）。
    #[test]
    fn demo_span_counts_bars_with_the_time_signature_denominator() {
        let mut project = yeban_model::samples::filled_project();
        project.time_signature = yeban_model::project::TimeSignature {
            numerator: 6,
            denominator: 8,
        };
        let span = 6 * yeban_model::ids::PPQ;
        for track in project.tracks.values_mut() {
            for clip in track.clips.values_mut() {
                clip.start_tick = 0;
                clip.duration_ticks = span;
            }
        }
        let (bars, _secs) = super::demo_span(&project);
        assert_eq!(bars, 2, "6/8 且末端 6×PPQ 必须是 2 小节");
    }

    /// 4/4 回归：末端 4×PPQ 仍是 1 小节（改动不许动既有口径）。
    #[test]
    fn demo_span_keeps_four_four_bars() {
        let mut project = yeban_model::samples::filled_project();
        project.time_signature = yeban_model::project::TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        let span = 4 * yeban_model::ids::PPQ;
        for track in project.tracks.values_mut() {
            for clip in track.clips.values_mut() {
                clip.start_tick = 0;
                clip.duration_ticks = span;
            }
        }
        let (bars, _) = super::demo_span(&project);
        assert_eq!(bars, 1, "4/4 且末端 4×PPQ 必须仍是 1 小节");
    }

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
    fn validate_rejects_out_of_domain_details_loudly() {
        let base = NewProjectDetails {
            bpm: 120.0,
            key_signature: None,
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 4,
            },
            sample_rate: SampleRate::Hz48000,
        };
        assert!(base.validate().is_ok());
        for bpm in [1000.0, f64::NAN, f64::INFINITY] {
            let error = NewProjectDetails { bpm, ..base }.validate().unwrap_err();
            assert!(
                error.contains("新工程没有创建") || error.contains("不是有限"),
                "{error}"
            );
        }
        let bad_den = NewProjectDetails {
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 3,
            },
            ..base
        };
        let error = bad_den.validate().unwrap_err();
        assert!(error.contains("分母"), "{error}");
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

    /// 构造一条测试用演示条目。
    fn demo_entry(title: &str, genre: Option<&str>) -> DemoEntry {
        DemoEntry {
            source: DemoSource::File(PathBuf::from(format!("{title}.yeban"))),
            title: title.to_owned(),
            track_count: 0,
            author: None,
            genre: genre.map(ToOwned::to_owned),
            meta: DemoMeta::default(),
            bpm: 120.0,
            bars: 0,
            duration_secs: 0.0,
            cover: DemoCover::None,
            download: None,
        }
    }

    /// 构造一条测试用最近条目。
    fn recent_entry(name: &str, path: &str) -> RecentEntry {
        RecentEntry {
            name: name.to_owned(),
            path: PathBuf::from(path),
            opened_at_unix_ms: 1_700_000_000_000,
            exists: true,
        }
    }

    /// T-FIX-293：演示按标题 / 曲风命中；`entry_index` 是全量下标；空串不过滤。
    #[test]
    fn demo_rows_matching_keeps_full_list_entry_index() {
        let entries = vec![
            demo_entry("演示 00", Some("流行")),
            demo_entry("演示 01", Some("古典")),
            demo_entry("演示 02", Some("流行")),
        ];
        assert_eq!(
            demo_rows_matching(&entries, crate::cli::Language::ZhCn, "").len(),
            3,
            "空串不过滤"
        );
        let rows = demo_rows_matching(&entries, crate::cli::Language::ZhCn, "古典");
        assert_eq!(rows.len(), 1, "曲风只命中一条");
        assert_eq!(rows[0].label, "演示 01");
        assert_eq!(rows[0].entry_index, Some(1), "过滤不重排下标");
        let rows = demo_rows_matching(&entries, crate::cli::Language::ZhCn, "流行");
        assert_eq!(rows.len(), 2, "曲风命中两条");
        assert_eq!(rows[0].entry_index, Some(0));
        assert_eq!(rows[1].entry_index, Some(2));
        assert!(
            demo_rows_matching(&entries, crate::cli::Language::ZhCn, "不存在").is_empty(),
            "无命中时行模型为空"
        );
    }

    /// T-FIX-293：最近按名 / 路径命中；`entry_index` 是全量下标。
    #[test]
    fn recent_rows_matching_keeps_full_list_entry_index() {
        let entries = vec![
            recent_entry("最近工程", "/tmp/最近工程.yeban"),
            recent_entry("工程B", "/tmp/工程B.yeban"),
        ];
        let rows = recent_rows_matching(
            &entries,
            crate::cli::Language::ZhCn,
            1_700_000_000_000,
            "工程B",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "工程B");
        assert_eq!(rows[0].entry_index, Some(1), "过滤不重排下标");
        let rows = recent_rows_matching(
            &entries,
            crate::cli::Language::ZhCn,
            1_700_000_000_000,
            "最近工程.yeban",
        );
        assert_eq!(rows.len(), 1, "路径也是匹配字段");
        assert_eq!(rows[0].entry_index, Some(0));
    }

    /// T-FIX-293：启动模板分类按来源 / 曲风 / 子曲风 / 模板名过滤；
    /// 分组标题只在命中组出现；`entry_index` 是全量模板清单下标。
    #[test]
    fn template_rows_matching_keeps_full_list_entry_index() {
        let entries = vec![
            TemplateEntry {
                source: TemplateSource::Factory,
                style: "流行".to_owned(),
                substyle: "摇滚".to_owned(),
                name: "出发".to_owned(),
                path: PathBuf::from("/f/流行/摇滚/出发.yeban"),
            },
            TemplateEntry {
                source: TemplateSource::Factory,
                style: "流行".to_owned(),
                substyle: "抒情".to_owned(),
                name: "归来".to_owned(),
                path: PathBuf::from("/f/流行/抒情/归来.yeban"),
            },
            TemplateEntry {
                source: TemplateSource::User,
                style: "流行".to_owned(),
                substyle: "摇滚".to_owned(),
                name: "我的".to_owned(),
                path: PathBuf::from("/u/流行/摇滚/我的.yeban"),
            },
        ];
        let rows = template_rows_matching(
            &entries,
            TemplateSource::Factory,
            crate::cli::Language::ZhCn,
            "归来",
        );
        let shape: Vec<(ChooserRowKind, String, Option<usize>)> = rows
            .iter()
            .map(|row| (row.kind, row.label.clone(), row.entry_index))
            .collect();
        assert_eq!(
            shape,
            vec![
                (ChooserRowKind::StyleHeader, "流行".to_owned(), None),
                (ChooserRowKind::SubstyleHeader, "抒情".to_owned(), None),
                (ChooserRowKind::Entry, "归来".to_owned(), Some(1)),
            ],
            "过滤后的条目下标必须是全量模板清单下标，⛔ 不重排"
        );
        // 别的来源的同名条目不进工厂分类。
        let rows = template_rows_matching(
            &entries,
            TemplateSource::Factory,
            crate::cli::Language::ZhCn,
            "我的",
        );
        assert!(rows.is_empty(), "用户来源的条目不得进工厂分类");
    }

    /// T-FIX-293：70 条演示的行模型没有 64 上限。
    #[test]
    fn demo_rows_have_no_sixty_four_cap() {
        let entries: Vec<DemoEntry> = (0..70)
            .map(|index| demo_entry(&format!("演示 {index:02}"), None))
            .collect();
        let rows = demo_rows_matching(&entries, crate::cli::Language::ZhCn, "");
        assert_eq!(rows.len(), 70, "70 条演示必须全进模型");
        assert_eq!(
            rows.last().and_then(|row| row.entry_index),
            Some(69),
            "最后一条的下标必须是全量下标 69"
        );
    }

    fn demo_scan_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-demo-scan-{tag}-{}",
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("临时目录");
        dir
    }

    /// T-FIX-641 判据：官方 demo 走固定清单口径 —— 超过 8 MiB 闸的
    /// **官方已下载** demo 仍进清单（标题来自固定清单、细节来自旁挂
    /// 元数据，清单阶段不读整包），且打开时能真实解析；同一目录里任意
    /// 超限用户文件照旧进 blocked 且原因写明上限。
    #[test]
    fn oversized_official_demo_is_listed_and_openable_while_user_file_stays_blocked() {
        let dir = demo_scan_dir("official");
        let spec = crate::demo_download::DEMOS.first().expect("固定清单非空");
        // 官方 demo：真实可解析容器 + 9 MiB 资产 ⇒ 整包 > 8 MiB 闸。
        let project = yeban_model::samples::filled_project();
        let payload = vec![7u8; 9 * 1024 * 1024];
        let hash = yeban_model::AssetHash::of_bytes(&payload);
        let mut assets = std::collections::BTreeMap::new();
        assets.insert(hash, payload);
        let archive = yeban_model::container::write_project_container(&project, &[], &assets)
            .expect("写官方 demo 容器");
        assert!(
            archive.len() as u64 > DEMO_LIST_MAX_BYTES,
            "夹具必须真的超过清单闸：{} 字节",
            archive.len()
        );
        let official = dir.join(spec.file);
        std::fs::write(&official, &archive).expect("写官方 demo");
        let stem = official
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("文件名");
        std::fs::write(
            dir.join(format!("{stem}.meta.json")),
            r#"{"genre":"流行","author":"夜半· Studio","trackCount":5,"bpm":96.0,"bars":32,"durationSecs":180.5}"#,
        )
        .expect("写旁挂");
        // 任意用户文件：稀疏、超限、内容为空 ⇒ 只要读它就会失败。
        let user = dir.join("huge-user.yeban");
        let file = std::fs::File::create(&user).expect("建稀疏文件");
        file.set_len(DEMO_LIST_MAX_BYTES + 1).expect("设置长度");
        drop(file);

        let scan = super::scan_file_demos(&dir);
        let row = scan
            .entries
            .iter()
            .find(|entry| entry.source == super::DemoSource::File(official.clone()))
            .expect("官方已下载 demo 必须进清单");
        assert_eq!(row.title, spec.title, "标题取固定清单的干净歌名");
        assert_eq!(row.track_count, 5, "轨道数取旁挂元数据");
        assert_eq!(row.bars, 32, "小节数取旁挂元数据");
        assert!((row.duration_secs - 180.5).abs() < f64::EPSILON);
        assert!((row.bpm - 96.0).abs() < f64::EPSILON);
        assert_eq!(row.author.as_deref(), Some("夜半"));
        assert_eq!(row.genre.as_deref(), Some("流行"));

        let blocked = scan
            .blocked
            .iter()
            .find(|blocked| blocked.label == "huge-user")
            .expect("任意超限用户文件仍必须进 blocked");
        assert!(
            blocked.reason.contains("超过清单阶段读盘上限")
                && blocked.reason.contains(&DEMO_LIST_MAX_BYTES.to_string()),
            "原因必须写明上限：{}",
            blocked.reason
        );
        assert_eq!(scan.blocked.len(), 1, "官方 demo 不得进置灰行");

        // 「可打开」：同一份官方 demo 走打开路径能真实解析出工程。
        let opened = super::load_file_demo_archive(&official).expect("官方 demo 必须可打开");
        assert_eq!(opened.project.title, project.title);
    }

    /// T-FIX-641 判据：官方 demo 的清单阶段**不读整包** —— 一个超过
    /// 8 MiB、内容为空（不可解析）的官方文件名，加上旁挂元数据，照样
    /// 进清单（读整包就一定失败）。
    #[test]
    fn official_demo_listing_never_reads_the_archive() {
        let dir = demo_scan_dir("official-sparse");
        let spec = crate::demo_download::DEMOS.first().expect("固定清单非空");
        let official = dir.join(spec.file);
        let file = std::fs::File::create(&official).expect("建稀疏文件");
        file.set_len(DEMO_LIST_MAX_BYTES + 1).expect("设置长度");
        drop(file);
        let stem = official
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("文件名");
        std::fs::write(dir.join(format!("{stem}.meta.json")), r#"{"trackCount":4}"#)
            .expect("写旁挂");
        let scan = super::scan_file_demos(&dir);
        assert_eq!(scan.entries.len(), 1, "官方 demo 必须进清单");
        assert!(scan.blocked.is_empty(), "官方 demo 不得被体积闸拦下");
        assert_eq!(scan.entries[0].title, spec.title);
        assert_eq!(scan.entries[0].track_count, 4);
    }

    /// 清单阶段的三道闸：超过上限的文件**一个字节都不读**、坏容器留原因、
    /// 正常 demo 照旧进清单。红对照：去掉 `load_demo_entry` 的体积预检 ⇒
    /// 超大文件会走 `fs::read` 后以"不是可读的工程容器"收场，第三条断言红。
    #[test]
    fn demo_scan_blocks_oversize_without_reading_and_keeps_reasons() {
        let dir = demo_scan_dir("oversize");
        let project = yeban_model::samples::filled_project();
        let good = yeban_model::container::write_project_container(
            &project,
            &[],
            &std::collections::BTreeMap::new(),
        )
        .expect("写容器");
        std::fs::write(dir.join("good.yeban"), good).expect("写合法 demo");
        std::fs::write(dir.join("broken.yeban"), b"x").expect("写坏 demo");
        // 稀疏文件：长度超上限、内容为空 ⇒ 只要实现真的读了它，就会以
        // "不是可读的工程容器"失败；正确实现则应停在体积闸上。
        let huge = dir.join("huge.yeban");
        let file = std::fs::File::create(&huge).expect("建稀疏文件");
        file.set_len(DEMO_LIST_MAX_BYTES + 1).expect("设置长度");
        drop(file);

        let scan = scan_file_demos(&dir);
        assert_eq!(scan.entries.len(), 1, "只有合法 demo 进清单");
        assert_eq!(
            scan.entries[0].title, project.title,
            "合法 demo 的标题必须是容器实读值"
        );
        assert_eq!(scan.blocked.len(), 2, "坏容器 + 超大文件各一条置灰行");
        let huge_row = scan
            .blocked
            .iter()
            .find(|blocked| blocked.label == "huge")
            .expect("超大文件必须有自己的置灰行");
        assert!(
            huge_row.reason.contains("超过清单阶段读盘上限"),
            "超大文件必须死在体积闸上（不是在解容器时失败）：{}",
            huge_row.reason
        );
        assert!(
            huge_row.reason.contains(&DEMO_LIST_MAX_BYTES.to_string()),
            "原因里必须写出上限字节数：{}",
            huge_row.reason
        );
        let broken_row = scan
            .blocked
            .iter()
            .find(|blocked| blocked.label == "broken")
            .expect("坏容器必须有自己的置灰行");
        assert!(
            broken_row.reason.contains("不是可读的工程容器"),
            "坏容器的原因必须具体：{}",
            broken_row.reason
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 目录读不了 ⇒ 一条目录级置灰行（不是空清单），行模型里它置灰、
    /// 无下标、原因可见。
    #[test]
    fn demo_scan_reports_unreadable_dir_and_rows_are_disabled() {
        let missing = demo_scan_dir("missing").join("nope");
        let scan = scan_file_demos(&missing);
        assert!(scan.entries.is_empty());
        assert_eq!(scan.blocked.len(), 1, "目录读不了必须有诊断");
        assert!(
            scan.blocked[0].reason.contains("演示目录读不了"),
            "原因必须写清是目录级失败：{}",
            scan.blocked[0].reason
        );

        let entries = vec![demo_entry("演示 00", Some("流行"))];
        let rows = demo_rows_matching_with_blocked(
            &entries,
            &scan.blocked,
            crate::cli::Language::ZhCn,
            "",
        );
        assert_eq!(rows.len(), 2, "正常条目 + 一条置灰行");
        assert!(rows[0].enabled, "正常条目照旧可点");
        let blocked_row = rows.last().expect("置灰行");
        assert!(!blocked_row.enabled, "未进清单的条目必须点不动");
        assert_eq!(blocked_row.entry_index, None, "置灰行没有可打开的下标");
        assert!(
            blocked_row.detail.contains("演示目录读不了"),
            "置灰行的副文字必须是原因：{}",
            blocked_row.detail
        );
        // 搜索能命中原因里的字 ⇒ 坏条目不会从搜索里消失。
        let rows = demo_rows_matching_with_blocked(
            &entries,
            &scan.blocked,
            crate::cli::Language::ZhCn,
            "读不了",
        );
        assert_eq!(rows.len(), 1, "搜索原因命中时只剩那条置灰行");
        assert!(!rows[0].enabled);
        std::fs::remove_dir_all(missing.parent().expect("父目录")).ok();
    }

    /// ⭐ T-FIX-541：封面解码失败必须在详情里看得见（一行「封面读不了：原因」），
    /// 没有封面 / 封面正常两种状态⛔ 不列该行 —— 三态必须可区分。
    #[test]
    fn a_failed_cover_is_a_visible_detail_row() {
        let mut failed = demo_entry("Bad Cover", None);
        failed.cover = DemoCover::Failed {
            path: PathBuf::from("bad-cover.png"),
            reason: "The image format could not be determined".to_owned(),
        };
        let rows = demo_detail_rows(Language::ZhCn, &failed);
        let row = rows
            .iter()
            .find(|row| row.label == "封面读不了")
            .expect("坏封面必须在详情里有一行");
        assert_eq!(
            row.value, "The image format could not be determined",
            "失败行必须原样带解码器原因（⛔ 不吞、⛔ 不编造友好文案）"
        );
        // 卡片摘要（无障碍 label）与展开区高度也必须算进这一行。
        let demo_rows = demo_rows_matching(std::slice::from_ref(&failed), Language::ZhCn, "");
        let demo_row = demo_rows
            .iter()
            .find(|row| row.entry_index.is_some())
            .expect("演示行必须在");
        assert!(
            demo_row.details.contains("封面读不了"),
            "卡片「详情」拼法里必须带上这一行，实测 {:?}",
            demo_row.details
        );
        assert!(
            demo_row.details_height_px > 0,
            "有失败行时展开区高度必须按行数长出来"
        );
        assert!(
            demo_row.cover.is_none(),
            "失败态没有可显示的图（⛔ 不要拿空图冒充有封面）"
        );

        // 没有封面 ⇒ 不列；封面正常 ⇒ 也不列（那是成功，不是失败）。
        let none = demo_entry("No Cover", None);
        assert!(
            !demo_detail_rows(Language::ZhCn, &none)
                .iter()
                .any(|row| row.label == "封面读不了"),
            "没有封面文件不是失败，⛔ 不许列「封面读不了」"
        );
        let mut ready = demo_entry("Good Cover", None);
        ready.cover = DemoCover::Ready(PathBuf::from("good-cover.png"));
        assert!(
            !demo_detail_rows(Language::ZhCn, &ready)
                .iter()
                .any(|row| row.label == "封面读不了"),
            "封面正常时⛔ 不许列「封面读不了」"
        );
    }
}
