//! `--export-song-overview <path.json>` —— 整曲总览图的**批处理出口**。
//!
//! ## 它补的是哪一格（这是本模块存在的全部理由）
//!
//! 总览图的真实算法住在 `yeban-dsp`，而 `yeban-mcp` **不允许**依赖 `yeban-dsp`
//! （依赖方向守卫）：provider 由宿主注入。于是今天要拿到一份总览 JSON，只有一条路 ——
//! **起一个 GUI 进程**（`--enable-mcp-http`，形态 A），连上去 `tools/call get_song_overview`。
//! 在没有显示器的机器上这条路还要先挂 `xvfb`，并且**一次只能服务一个工程**：
//! 批量检查四个工程 = 起停四次进程、四次等面板首算。
//!
//! 本模块把同一份算法接到**批处理**路径上：一次 `yeban-app --open X.yeban
//! --export-song-overview X.json`，不开窗口、不建 MCP 会话、不碰 `~/.yeban` 的令牌。
//!
//! ## 与工具出口的**逐字节**关系（这一条是契约，不是"应该差不多"）
//!
//! 输出文件的内容 = `tools/call get_song_overview` 响应里 `data` 的 JSON（*原样*，不换字段名、
//! 不增不减），只差**四**件事，每件都是**刻意**的：
//!
//! | 差异 | 为什么 |
//! | :--- | :--- |
//! | 缩进成 2 空格的 pretty 形态 ＋ 末尾换行 | 落盘给人看 / 给 `diff` 看；JSON 语义不变 |
//! | 不带 `pngPath` / `pngNote` | 那是 `include_png_path: true` 才加的 Phase 2 说明；CLI 没有这个参数 |
//! | 不带 `commitModel` | 那是**协议层**的回执装饰（`ADR-0032` D1：只读工具恒为 `"none"`），不属于总览数据；把它写进交付文件等于让这份数据多背一个与它无关的协议字段 |
//! | **多**一个 `renderMode` | 只有这条出口有 `score-only` 这一档，读文件的人不必去翻命令行就知道这份数是渲染过的还是只有谱面的 |
//! | —— | 取整走 `yeban_mcp::domain::song_overview::rounded`，**同一个函数** |
//!
//! ⇒ 同一份工程，把这两处差异去掉（`del(.commitModel, .renderMode)`）之后 `jq -S`，
//! 两边的 `data` 必须**逐字节相同**。这条由 `crates/yeban-app/tests/song_overview_export.rs`
//! 钉住（它同时跑两条路：真 socket 上的 `tools/call` 与真二进制的批处理出口）。
//!
//! ## 失败语义（与其它导出族同款）
//!
//! * 采样器读不到 ⇒ **响亮失败**（`samplerUnreadable`），⛔ 不悄悄降级成纯谱面；
//!   要把"能算多少算多少"写出来，请显式给 `--overview-score-only`（见下）。
//! * 渲染失败 / 落盘失败 ⇒ 非零退出码，**不留半成品**（落盘走
//!   [`crate::save::write_file_atomically`]：同目录临时文件 → rename）。
//! * **静音不是失败，但必须自己说出来**：全量渲染却一个小节都没能量时（缺样本在
//!   provider 里是"跳过该样本"、引擎标 `NO_SAMPLE` ⇒ **不报错**），回执会多一行
//!   `overview-silent:`，把"这份数是静音的"变成可 grep 的事实并列出三个排查方向。
//!   退出码仍是 0 —— 空工程、无音源、真的写了一首全休止的曲子都是合法状态。
//!
//! ## `--overview-score-only`：显式要"只有谱面"
//!
//! GUI 面板在渲染失败时**回退**到纯谱面（面板总得画点什么，而且要在回执里说清回退原因）。
//! 批处理不吃这一套：一个"因为渲染失败所以数字全是哨兵值"的文件，如果和正常文件长得一样，
//! 就是一份**假的**交付物。因此：
//!
//! * 缺省 = 渲染失败即失败（`render-failed`，退出码 5）；
//! * 给 `--overview-score-only` = 跳过整曲渲染，直接出谱面分析（`render=score-only` 会写在
//!   回执行与 JSON 的 `renderMode` 字段里）—— 名字里带 `score-only`，谁也误会不了。
//!
//! ## `--export-song-overview-html`：同一份总览的 HTML 报告
//!
//! 与 JSON 出口**同一份"算总览"步骤**（provider / ScoreOnly → `rounded`，见
//! [`compute_overview`]），只差最后一步：JSON 换成 `yeban-mcp` 的转置 +
//! `yeban-dsp` 唯一渲染器的单文件自包含 HTML 字符串（与
//! `tools/call get_song_overview {"format": "html"}` 的 `data` 逐字节相同）。
//! 报告标题取 `project.title`（与工具侧同一来源 ⇒ 逐字节一致仍成立）。
//! 回执前缀是 `exported-song-overview-html:`；失败语义与 JSON 出口同款（见上）。
//! HTML 报告本身不写 `renderMode`（它不是 JSON），口径只写在回执行 `render=` 里。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::song_overview::{
    Profile, Resolution, SongOverviewProvider, SongOverviewRequest,
    overview_html_report_with_title, rounded,
};
use yeban_model::{AssetHash, YebanProjectV1};

use crate::save::{SaveError, SaveReport, write_file_atomically};
use yeban_mcp::song_overview_provider::{ProviderContext, RealSongOverviewProvider};

/// 导出成功后回执行的第一段（稳定前缀，判据与脚本据此切片）。
pub const RECEIPT_PREFIX: &str = "exported-song-overview:";

/// HTML 报告出口的回执前缀（与 JSON 出口的唯一区别，判据据此切片）。
pub const RECEIPT_HTML_PREFIX: &str = "exported-song-overview-html:";

/// 导出形态（回执前缀的唯一来源：`line()` 不手写前缀）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// JSON（`exported-song-overview:`）。
    Json,
    /// HTML 报告（`exported-song-overview-html:`）。
    Html,
}

impl ExportFormat {
    /// 回执里的稳定前缀。
    #[must_use]
    pub const fn receipt_prefix(self) -> &'static str {
        match self {
            Self::Json => RECEIPT_PREFIX,
            Self::Html => RECEIPT_HTML_PREFIX,
        }
    }
}

/// 两种渲染口径（写进回执 `render=` 与 JSON 的 `renderMode`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// 全量：真的把整曲渲染一遍再分析（与 GUI / 工具同一条路）。
    Full,
    /// 只有谱面：`--overview-score-only`，跳过渲染。
    ScoreOnly,
}

impl RenderMode {
    /// 回执 / JSON 里的稳定词。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ScoreOnly => "score-only",
        }
    }
}

/// 一次导出的实测读数（回执行的**唯一**来源：⛔ 不从入参抄）。
#[derive(Debug)]
pub struct OverviewExportReport {
    /// 实际落点（`write_file_atomically` 报的那个路径）。
    pub path: PathBuf,
    /// 实际字节数。
    pub bytes: usize,
    /// 小节数（`bar_series` 的条数）。
    pub bars: usize,
    /// 行粒度（当前恒为 `bar`：批处理没有 `--overview-resolution`，见模块文档"未做"）。
    pub resolution: &'static str,
    /// 曲风档（当前恒为 `default`）。
    pub profile: &'static str,
    /// provider 自述名（`real`；这个出口**永远**是真 provider，⛔ 没有 stub 档）。
    pub provider: &'static str,
    /// `render=full|score-only`。
    pub mode: RenderMode,
    /// 导出形态（决定回执前缀；JSON 行为一字不动）。
    pub format: ExportFormat,
    /// 有声小节的 `lufs_short` 范围（全静音 ⇒ `None`，⛔ 不写成 −70）。
    pub lufs: Option<(f64, f64)>,
    /// 高潮小节（`structural_summary.climax_bar`，§8.2 里它是 `u64` 的小节号）。
    pub climax_bar: u64,
    /// 自动诊断出的异常条数。
    pub anomalies: usize,
    /// 缓存命中（批处理里恒为 `false`：一次性进程、新缓存）。
    pub cache_hit: bool,
}

impl OverviewExportReport {
    /// 回执行（**一行**，字段顺序即契约）。
    #[must_use]
    pub fn line(&self) -> String {
        let lufs = self.lufs.map_or_else(
            || "lufs=none".to_owned(),
            |(low, high)| format!("lufs={low:.1}..{high:.1}"),
        );
        let prefix = self.format.receipt_prefix();
        format!(
            "{prefix} path={} bytes={} bars={} resolution={} profile={} provider={} \
             render={} {lufs} climaxBar={} anomalies={} cacheHit={}",
            self.path.display(),
            self.bytes,
            self.bars,
            self.resolution,
            self.profile,
            self.provider,
            self.mode.as_str(),
            self.climax_bar,
            self.anomalies,
            self.cache_hit,
        )
    }

    /// 全量渲染却**一个小节都没出声**吗？
    ///
    /// 只有 [`RenderMode::Full`] 才有资格叫"静音"：`ScoreOnly` 本来就没有音频。
    #[must_use]
    pub const fn is_silent(&self) -> bool {
        matches!(self.mode, RenderMode::Full) && self.lufs.is_none()
    }

    /// 静音时的**诊断行**（`overview-silent:` 开头；不静音 ⇒ `None`，一行都不多）。
    ///
    /// ## 它补的是哪一种失败
    ///
    /// 实测（2026-11-10，`L_F_A_nv_安和桥风_女声.yeban` / 57 小节）：整曲渲染出来
    /// **一个小节都没有能量**（所有 `lufs_short` 都是 −70 哨兵），而这次导出
    /// **没有失败** —— 缺样本在 provider 里是"跳过该样本"（引擎标 `NO_SAMPLE`），
    /// 不是错误。于是旧行为给出的是：一个**成功**的退出码 ＋ 一份数字全是哨兵的 JSON，
    /// 没有一个字提示"这一份是静音的"。
    ///
    /// ⛔ 这一行**不改**退出码（静音不是失败：空工程、纯 MIDI 无音源、或真的写了一首
    /// 全休止的曲子都是合法状态）；它只把"你看的这份数是静音的"变成可 grep 的一行，
    /// 并把排查方向写出来。
    #[must_use]
    pub fn silent_note(&self) -> Option<String> {
        if !self.is_silent() {
            return None;
        }
        Some(format!(
            "overview-silent: {} 小节全部没有能量（`lufs_short` 全是 −70 哨兵）—— \
             本次导出**成功**但这份总览是静音的。先查三件事：\
             ① 轨上的采样器绑定是否指向读得到的 SFZ（`--export-wav` 的 `source=` 会说明实际音源）；\
             ② SFZ 里引用的样本是否真的存在（缺样本不会让导出失败，只会让该区域静音）；\
             ③ 素材是否挂在主总线轨上（按设计不参与渲染）。",
            self.bars
        ))
    }
}

/// 导出失败的两种来源（退出码由 `cli::CliError` 统一给：都是 `EXIT_EXPORT`）。
#[derive(Debug)]
pub enum OverviewExportError {
    /// 总览算不出来（provider 的 `Fault` 原文：采样器读不到 / 渲染失败）。
    ///
    /// 带 `Fault` 而不是一句字符串：`reason` 与 `data` 是**契约**（`samplerUnreadable` 等），
    /// 换成人话字符串会让"哪一类失败"重新变成字符串匹配。
    Render(Box<Fault>),
    /// 落盘失败。
    Save {
        /// 目标路径。
        path: PathBuf,
        /// 落盘层原样裁决。
        source: SaveError,
    },
}

impl core::fmt::Display for OverviewExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Render(fault) => match fault.as_ref() {
                Fault::Domain { message, .. } => write!(formatter, "{message}"),
                Fault::Impl { error } => write!(formatter, "总览计算失败（实现级）: {error:?}"),
            },
            Self::Save { path, source } => {
                write!(formatter, "写入 `{}` 失败: {source}", path.display())
            }
        }
    }
}

/// 算一份总览并**原子**落盘成 JSON，返回实测读数。
///
/// # 参数
///
/// * `project`：要分析的工程（`--open` 的那份，或 `--project-sample`）；
/// * `path`：JSON 落点（覆盖已有文件；失败不留半成品）；
/// * `project_dir`：工程文件所在目录 —— 磁盘腿 SFZ 的相对路径第一个候选根。
///   `None`（内存会话 / 样本工程）⇒ 只有内嵌腿可用；
/// * `assets`：归档资产池（`assets/{sha256}`）—— 内嵌腿的 SFZ 文本与样本从这里取；
/// * `mode`：[`RenderMode::Full`] 走真实渲染，[`RenderMode::ScoreOnly`] 跳过。
///
/// # Errors
///
/// [`OverviewExportError::Render`]（provider 的 `Fault` 原文）或
/// [`OverviewExportError::Save`]。
pub fn export_song_overview(
    project: &YebanProjectV1,
    path: &Path,
    project_dir: Option<&Path>,
    assets: &[(AssetHash, Vec<u8>)],
    mode: RenderMode,
) -> Result<OverviewExportReport, OverviewExportError> {
    let (overview, provider, request) = compute_overview(project, project_dir, assets, mode)?;
    let mut data = serde_json::to_value(&overview).map_err(|error| {
        OverviewExportError::Render(Box::new(Fault::domain(
            yeban_mcp::tools::ErrorCode::IoError,
            format!("总览序列化失败（实现级状况，不应发生）: {error}"),
        )))
    })?;
    let object = data.as_object_mut().expect("SongOverview 序列化后是对象");
    object.insert("provider".to_owned(), serde_json::Value::from(provider));
    object.insert(
        "resolution".to_owned(),
        serde_json::Value::from(request.resolution.as_str()),
    );
    // ⭐ 只在这一条出口里有的字段：读文件的人**不必**去查命令行才知道这份数是"渲染过的"
    // 还是"只有谱面的"。工具出口不带它（工具没有 score-only 这一档）。
    object.insert(
        "renderMode".to_owned(),
        serde_json::Value::from(mode.as_str()),
    );

    // 终端用户会 `head` 这个文件，也会把它塞进 `jq`：pretty + 末尾换行。
    let mut text = serde_json::to_string_pretty(&data).map_err(|error| {
        OverviewExportError::Render(Box::new(Fault::domain(
            yeban_mcp::tools::ErrorCode::IoError,
            format!("总览序列化失败（实现级状况，不应发生）: {error}"),
        )))
    })?;
    text.push('\n');

    let report: SaveReport = write_file_atomically(text.as_bytes(), path).map_err(|source| {
        OverviewExportError::Save {
            path: path.to_path_buf(),
            source,
        }
    })?;

    Ok(OverviewExportReport {
        path: report.path,
        bytes: report.bytes,
        bars: overview.bar_series.len(),
        resolution: request.resolution.as_str(),
        profile: "default",
        provider,
        mode,
        format: ExportFormat::Json,
        lufs: lufs_range(&overview),
        climax_bar: overview.structural_summary.climax_bar,
        anomalies: overview.structural_summary.detected_anomalies.len(),
        cache_hit: false,
    })
}

/// 两条出口共用的"算总览"步骤：provider / ScoreOnly → §8.2 取整。
///
/// 返回（已取整总览，provider 名，本次请求）。JSON 与 HTML 在此之后分叉。
fn compute_overview(
    project: &YebanProjectV1,
    project_dir: Option<&Path>,
    assets: &[(AssetHash, Vec<u8>)],
    mode: RenderMode,
) -> Result<
    (
        yeban_mcp::output_schema::SongOverview,
        &'static str,
        SongOverviewRequest,
    ),
    OverviewExportError,
> {
    // provider 要一份共享池（它内部按需解码，缓存也在它里面）。批处理是一次性的 ⇒
    // 这份 `Arc<BTreeMap>` 只活这一次调用，不存在"多轨共享"之外的复用诉求。
    let pool: Arc<BTreeMap<AssetHash, Vec<u8>>> =
        Arc::new(assets.iter().cloned().collect::<BTreeMap<_, _>>());
    let provider = RealSongOverviewProvider::new(ProviderContext::shared(
        project_dir.map(Path::to_path_buf),
        Arc::clone(&pool),
    ));
    let request = SongOverviewRequest {
        resolution: Resolution::Bar,
        profile: Profile::Default,
        expected_levels: BTreeMap::new(),
    };

    let overview = match mode {
        RenderMode::Full => provider
            .overview(project, &request)
            .map_err(|fault| OverviewExportError::Render(Box::new(fault)))?,
        RenderMode::ScoreOnly => yeban_mcp::song_overview_provider::score_only_overview(project)
            .map_err(|fault| OverviewExportError::Render(Box::new(fault)))?,
    };
    // §8.2 取整：**同一个**函数（见模块文档的逐字节契约）。
    let overview = rounded(overview);
    Ok((overview, provider.name(), request))
}

/// 算一份总览并**原子**落盘成 HTML 报告，返回实测读数。
///
/// 与 [`export_song_overview`] 同一份"算总览"步骤（见 [`compute_overview`]），
/// 只差最后一步：JSON 换成转置 + `yeban-dsp` 唯一渲染器的 HTML 字符串；
/// 回执前缀是 `exported-song-overview-html:`。失败语义与 JSON 出口同款。
///
/// # 参数 / Errors
///
/// 同 [`export_song_overview`]（`path` 是 `.html` 落点）。
pub fn export_song_overview_html(
    project: &YebanProjectV1,
    path: &Path,
    project_dir: Option<&Path>,
    assets: &[(AssetHash, Vec<u8>)],
    mode: RenderMode,
) -> Result<OverviewExportReport, OverviewExportError> {
    let (overview, provider, request) = compute_overview(project, project_dir, assets, mode)?;
    let text = overview_html_report_with_title(&overview, &project.title);
    let report: SaveReport = write_file_atomically(text.as_bytes(), path).map_err(|source| {
        OverviewExportError::Save {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok(OverviewExportReport {
        path: report.path,
        bytes: report.bytes,
        bars: overview.bar_series.len(),
        resolution: request.resolution.as_str(),
        profile: "default",
        provider,
        mode,
        format: ExportFormat::Html,
        lufs: lufs_range(&overview),
        climax_bar: overview.structural_summary.climax_bar,
        anomalies: overview.structural_summary.detected_anomalies.len(),
        cache_hit: false,
    })
}

/// 有声小节的 `lufs_short` 范围。
///
/// 门槛与 GUI 面板 / 既有脚本同一条：`> −69.9` 才算"这一小节有声音"
/// （−70 是 `yeban-dsp` 给"这一段没有能量"的哨兵）。全静音 ⇒ `None` ——
/// 回执写 `lufs=none` 而不是 `lufs=-70.0..-70.0`（后者看起来像一个真实的测量值）。
fn lufs_range(overview: &yeban_mcp::output_schema::SongOverview) -> Option<(f64, f64)> {
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for row in &overview.bar_series {
        if row.lufs_short > -69.9 {
            low = low.min(row.lufs_short);
            high = high.max(row.lufs_short);
        }
    }
    (low <= high).then_some((low, high))
}
