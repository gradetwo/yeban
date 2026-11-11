//! `get_song_overview` —— 整曲多层可视化总览图的 MCP 工具
//! （设计稿 `~/workspace/yeban-songmap-design.md` §8，Phase 1）。
//!
//! ## 架构落点：`SongOverviewProvider` trait + 宿主注入
//!
//! 证据（先查再定，不凭假设）：
//!
//! 1. `cargo tree -p yeban-mcp -e normal` 里 **`yeban-dsp` 只出现在
//!    `yeban-render` 的传递依赖里**（`yeban-mcp → yeban-render → yeban-dsp`），
//!    `yeban-mcp/Cargo.toml` 的 `[dependencies]` 里**没有**它；
//! 2. `scripts/gates/check_mcp_dependency_direction.py` 的 `FORBIDDEN` 逐字含
//!    `"yeban-dsp"`（音频栈）、`"yeban-render"`、`"yeban-engine"`，且
//!    `yeban-render` 那条边只是**已登记例外**（`EXCEPTIONS` 里
//!    `(yeban-mcp, yeban-render)` 一条）—— 再加一条 `yeban-dsp`
//!    **直接**依赖边就是第二条未登记违规，守卫会红；
//! 3. `crates/yeban-app/src/mcp_mount.rs` 的既有口径是**宿主注入镜像**
//!    （`Domain::set_engine_readings` + `EngineReadingsHandle`）：
//!    宿主在挂载后把读数交给正在服务的那个 `Domain`，工具只读镜像。
//!
//! 因此分析（STFT / LUFS / 情绪公式 / 异常检测）**不住**在 `yeban-mcp` 里：
//! 本模块只定义
//!
//! - 输出形状（[`crate::output_schema::SongOverview`]，§8.2），
//! - [`SongOverviewProvider`] trait（方法返回那份形状），
//! - [`plan_song_overview`]（参数校验 + 取 provider + 按 §8.2 量化取整 + 组装）。
//!
//! 真实现由 `yeban-app`（挂载时经 [`super::Domain::set_song_overview_provider`]
//! 注入；app 能同时依赖 `yeban-dsp` 与 `yeban-mcp`，负责把 dsp 输出映射成
//! §8.2 的 JSON）提供；dsp 线落地后由协调员派人接真实 provider，
//! 本轮不新建那条依赖边、也不等它。
//!
//! ## provider 怎么拿到工程（round 2 的接线口径）
//!
//! [`SongOverviewProvider::overview`] 的第一个参数就是 [`Domain`] 当前的活跃工程
//! —— `plan_song_overview` 里 `require_active` 取到的那一份**原样传进**，
//! provider 不自己缓存工程快照。三个被否掉的方案：① provider 在构造时领一份
//! 快照 ⇒ 工具改过工程后它看到的还是旧谱面（过期）；② provider 持共享 cell、
//! 宿主在修订号前进时刷新 ⇒ 工具调用与刷新之间有竞态窗口；③ provider 回头读
//! `Domain` ⇒ 调用时正持着分发器的锁，重入即死锁。当前方案三者都没有：
//! 每一次调用都拿到调用那一刻的活跃工程。
//!
//! ## `diagnostics` 的归属
//!
//! 异常检测逻辑（`low_mid_buildup`、`compression_squash` 等）由 dsp 线实现并由
//! 两边共用；本 handler **只消费 provider 已算好的序列与异常条目做映射**，
//! ⛔ 不在 MCP 里复制一份检测算法（阈值扫描、频段超量判定都不出现在这里）。
//! 下面的 stub 只在测试里用：它的异常条目是**手算的固定夹具值**（算术写在注释里），
//! 不是一套检测算法 —— 换真实 provider 时整段 stub 被替掉，handler 一位不动。
//!
//! ## 参数与失败语义
//!
//! - `resolution`：`"bar"`（缺省）或 `"beat"`；其它值 ⇒ `INVALID_PARAMETER_RANGE`
//!   （`reason: "unknownResolution"`）。
//! - `profile`（Phase 2，设计稿 §3.6）：`"default"`（缺省）/ `"orchestral"` /
//!   `"electronic"`；其它值 ⇒ `INVALID_PARAMETER_RANGE`（`reason: "unknownProfile"`）。
//! - `expected_levels`（Phase 2，设计稿 §3.5）：段落名 → 期望张力等级（1–5 级）
//!   的对象，如 `{"Chorus": 5}`；**非对象** ⇒ 协议层 `-32602`（注册表类型
//!   校验先于领域层）；对象内非整数值、越界等级 ⇒
//!   `INVALID_PARAMETER_RANGE`（`reason: "invalidExpectedLevels"`）。空对象
//!   `{}` 合法 = 无标注。等级只活在本次分析请求里，不写工程。
//! - `include_png_path`：缺省 `false`。Phase 1 **只接受参数、不产 PNG**：
//!   给 `true` 时响应带 `pngPath: null` + 一句 `pngNote`（Phase 2 再接 Slint
//!   离屏渲染，设计稿 §8.1），⛔ 不声称已实现未实现的功能。
//! - `format`：`"json"`（缺省，现有行为一字不动）或 `"html"`（返回完整 HTML 报告
//!   字符串，见 [`overview_html_report`]）；其它值 ⇒ `INVALID_PARAMETER_RANGE`
//!   （`reason: "unknownFormat"`）。`format: "html"` 与 `include_png_path: true`
//!   同给 ⇒ 同码（`reason: "includePngPathRequiresJson"`）：HTML 报告没有
//!   `pngPath` 槽位，⛔ 不静默忽略。
//! - 没有活跃工程 ⇒ `NO_ACTIVE_PROJECT`（与其它只读工具同一口径）。
//! - 宿主没有注入 provider ⇒ `CONFLICT` + `reason: "providerUnavailable"`
//!   （与 `yeban_launch_scene` 的 `engineUnavailable` 同一款"响亮拒绝"，
//!   ⛔ 不返回空总览冒充成功）。
//! - 只读：不改工程、不落盘、不进提交图谱（`Plan::SongOverview` 的
//!   `commit_delta` 为 0，`commitModel` 为 `"none"`）。

use serde_json::Value;

use std::collections::{BTreeMap, BTreeSet};

use yeban_model::YebanProjectV1;
use yeban_render::song_overview::{
    Anomaly as DspAnomaly, AnomalyType as DspAnomalyType, BarSeries as DspBarSeries,
    BestMatch as DspBestMatch, DEFAULT_REPORT_TITLE, NO_ENVELOPE,
    RepeatedPassage as DspRepeatedPassage, RepetitionSummary as DspRepetitionSummary,
    SCHEMA_VERSION, SongMeta as DspSongMeta, SongOverview as DspSongOverview,
    StructuralSummary as DspStructuralSummary, render_html_report_with_title,
};

use crate::output_schema::{
    BAND_DEFINITIONS_HZ, BarRow, DetectedAnomaly, RepetitionSummary, SongMeta, SongOverview,
    StructuralSummary,
};
use crate::tools::{ErrorCode, ToolCall};

use super::error::Fault;
use super::{Domain, arg_bool, arg_str, require_active};

/// 宿主没有注入 provider 时的 `data.reason`（与 `engineUnavailable` 同款命名）。
pub const PROVIDER_UNAVAILABLE_REASON: &str = "providerUnavailable";

/// 曲风预设档（Phase 2，设计稿 §3.6；MCP 侧的档名表）。
///
/// 权重本身住在 `yeban-dsp` 的 `TensionProfile::weights`（唯一事实源）；
/// 这里只定档名与解析（MCP crate 禁依赖 dsp，见模块头）。真实 provider
/// （`yeban-app`）把本档名映射成 dsp 档；stub 用同一组数字手算夹具张力
/// （见下面的 `profile_weights`，与 dsp 逐位一致）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Profile {
    /// 均衡流行摇滚（默认）。
    #[default]
    Default,
    /// 管弦 / 原声叙事。
    Orchestral,
    /// 电子 / 律动。
    Electronic,
}

impl Profile {
    /// 档名（进响应 `song_meta.profile`，与 dsp 的档名同一组词）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Orchestral => "orchestral",
            Self::Electronic => "electronic",
        }
    }

    /// 按档名解析；未知档名返回 `None`（handler 响亮拒绝，不静默回退）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "default" => Some(Self::Default),
            "orchestral" => Some(Self::Orchestral),
            "electronic" => Some(Self::Electronic),
            _ => None,
        }
    }
}

/// 时间分辨率（设计稿 §8.2：`resolution` 参数，`bar` | `beat`，缺省 `bar`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// 小节级（缺省）：`bar_series` 一小节一行。
    Bar,
    /// 拍级：`bar_series` 一拍一行（行里多一个 `beat` 键）。
    Beat,
}

impl Resolution {
    /// 响应里的自述词。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Beat => "beat",
        }
    }
}

/// 传给 provider 的请求（分辨率 + Phase 2 的曲风档与期望等级表；
/// `include_png_path` 是 handler 层的呈现开关，不进 provider —— provider
/// 只算数，不产图）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SongOverviewRequest {
    /// 时间分辨率。
    pub resolution: Resolution,
    /// 曲风预设档（缺省 [`Profile::Default`]）。
    pub profile: Profile,
    /// 按段落名给的期望张力等级（段落名 → 1–5 级；空表 = 无标注）。
    pub expected_levels: BTreeMap<String, u8>,
}

impl SongOverviewRequest {
    /// 只有分辨率的请求（缺省档、无期望等级表；测试与旧调用点的简写）。
    #[must_use]
    pub fn with_resolution(resolution: Resolution) -> Self {
        Self {
            resolution,
            profile: Profile::Default,
            expected_levels: BTreeMap::new(),
        }
    }
}

/// 整曲总览分析的**唯一**入口。
///
/// 实现方（`yeban-dsp` 侧的真实分析，或测试里的 stub）只做一件事：对当前活跃
/// 工程算出 §8.2 形状的 [`SongOverview`]。`Debug` 是 trait 约束的一部分 ——
/// [`Domain`] 派生 `Debug`，provider 以 `Arc<dyn SongOverviewProvider>` 住在
/// 会话运行态里，`Arc` 的 `Debug` 要求 trait 本身是 `Debug`。
pub trait SongOverviewProvider: std::fmt::Debug + Send + Sync {
    /// 自述名（进响应 `data.provider`，供调用方与判据区分 stub 与真实实现）。
    fn name(&self) -> &'static str;

    /// 对给定的活跃工程做一次总览分析。
    ///
    /// `project` 是 [`Domain`] 当前的活跃工程（调用那一刻的版本，见模块头的
    /// 接线口径）—— provider 不缓存它、不回头读它。
    ///
    /// # Errors
    ///
    /// 分析不可用（无渲染音频、无谱面）时响亮失败，⛔ 不返回空总览。
    fn overview(
        &self,
        project: &YebanProjectV1,
        request: &SongOverviewRequest,
    ) -> Result<SongOverview, Fault>;
}

/// 保留 2 位小数（归一化指标，§8.2）。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// 保留 1 位小数（响度与频段能量 dB，§8.2）。
fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// 对 provider 返回的总览按 §8.2 的量化口径取整（handler 的唯一数值动作）。
///
/// ⭐ 公开的理由（`--export-song-overview`）：批处理出口要与 `tools/call get_song_overview`
/// 的 `data` **逐字节**可比 —— 那意味着取整必须走**同一个**函数，⛔ 不是在 app 侧再写一份
/// "看起来一样"的 `round1`/`round2`（两份取整迟早会漂移，而漂移表现为"同一份工程，
/// CLI 的文件与工具的回包差 0.1"）。
///
/// 调用它的**唯一**义务：`provider.overview()` 之后、序列化之前。
pub fn rounded(mut overview: SongOverview) -> SongOverview {
    overview.song_meta.duration_sec = round1(overview.song_meta.duration_sec);
    overview.song_meta.base_bpm = round1(overview.song_meta.base_bpm);
    overview.structural_summary.climax_tension = round2(overview.structural_summary.climax_tension);
    overview.structural_summary.dynamic_range_lu =
        round1(overview.structural_summary.dynamic_range_lu);
    // schema 1.1 的 `repetition`：归一化指标保留 2 位小数（§8.2 同口径）。
    for best in &mut overview.repetition.best_matches {
        best.similarity = round2(best.similarity);
    }
    overview.repetition.mechanical_index = round2(overview.repetition.mechanical_index);
    for passage in &mut overview.repetition.repeated_passages {
        passage.mean_similarity = round2(passage.mean_similarity);
    }
    for row in &mut overview.bar_series {
        row.time_sec = round2(row.time_sec);
        row.tension = round2(row.tension);
        row.valence_hue = round2(row.valence_hue);
        row.lufs_short = round1(row.lufs_short);
        row.lufs_momentary_max = round1(row.lufs_momentary_max);
        row.score_dyn = round2(row.score_dyn);
        row.envelope_lo = row.envelope_lo.map(round2);
        row.envelope_hi = row.envelope_hi.map(round2);
        row.target_deviation = round2(row.target_deviation);
        row.density_pitched_onsets_per_beat = round2(row.density_pitched_onsets_per_beat);
        row.density_pitched_duty = round2(row.density_pitched_duty);
        row.density_perc_onsets_per_beat = round2(row.density_perc_onsets_per_beat);
        for band in &mut row.bands_tilted_db {
            *band = round1(*band);
        }
    }
    overview
}

/// `format` 参数的响应形态（缺省 `"json"`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverviewFormat {
    /// §8.2 JSON（既有行为，一字不动）。
    Json,
    /// 单文件 HTML 报告（[`overview_html_report`]）。
    Html,
}

/// `u64` 小节号 → dsp 的 `u32`（饱和截断；小节号是小整数，截断臂不可达）。
fn to_u32(value: u64) -> u32 {
    value.min(u32::MAX as u64) as u32
}

/// 把已取整的 §8.2 总览（行式）转置成 dsp 列式并渲染成单文件 HTML 报告（通用标题口径）。
///
/// 唯一 HTML 实现住在 `yeban-dsp` 的 `song_overview_report`（经 `yeban-render` 的
/// 再导出调用，见本文件头的依赖说明）；这里只做机械转置，⛔ 不另写 HTML。
/// CLI 的 `--export-song-overview-html` 调同一函数 ⇒ 两边逐字节一致。
/// 标题口径见 [`overview_html_report_with_title`]：两边都传当前工程的
/// `project.title`，同一工程 ⇒ 同一标题 ⇒ 逐字节一致仍成立。
///
/// 转置口径：
/// - 数值已由 handler 按 §8.2 取整（归一化 2 位、dB 1 位），HTML 显示的正是 JSON 的数；
/// - `envelope_lo` / `envelope_hi` 的 `None`（无等级）→ dsp 哨兵 [`NO_ENVELOPE`]；
/// - `resolution: "beat"` 的拍行按小节去重（取每小节首行；拍行值继承所属小节，
///   逐位相同，去重不丢信息），报告恒为小节级；
/// - `effective_weights` 不在 §8.2 形状里：填所选曲风档的档位权重（[`profile_weights`]），
///   报告不展示该字段（只为凑齐 dsp 结构）；自适应权重只活在 dsp 侧，不经 MCP 形状回传；
/// - `schema_version` 恒为 [`SCHEMA_VERSION`]：§8.2 形状只携带这一版；
/// - 未知异常类型（§8.2 契约外的值）→ `LowMidBuildup` 兜底：类型名不在契约值域里时
///   描述文本仍原样保留在报告里（信息不丢）；两条 in-tree provider 只产出契约内的 4 种，
///   该臂不可达（对照见 `tests/song_overview.rs` 的 `html_report_keeps_unknown_anomaly_text`）。
#[must_use]
pub fn overview_html_report(overview: &SongOverview) -> String {
    overview_html_report_with_title(overview, DEFAULT_REPORT_TITLE)
}

/// 带标题的转置 + 渲染：标题取当前活跃工程的 `project.title`
/// （`YebanProjectV1.title`，顶层唯一权威；调用点见 `plan_song_overview` 的
/// HTML 分支，`project` 就是 `require_active` 取到的活跃工程）。
/// 空标题时渲染器回退到通用标题。
#[must_use]
pub fn overview_html_report_with_title(overview: &SongOverview, title: &str) -> String {
    // 拍级行按小节去重（取每小节首行）。
    let mut seen = BTreeSet::new();
    let mut rows: Vec<&BarRow> = Vec::new();
    for row in &overview.bar_series {
        if seen.insert(row.bar) {
            rows.push(row);
        }
    }
    let dsp = DspSongOverview {
        schema_version: SCHEMA_VERSION,
        song_meta: DspSongMeta {
            total_bars: to_u32(overview.song_meta.total_bars),
            duration_sec: overview.song_meta.duration_sec as f32,
            base_bpm: overview.song_meta.base_bpm as f32,
            key_signature: overview.song_meta.key_signature.clone(),
            profile: overview.song_meta.profile.clone(),
        },
        band_definitions_hz: overview
            .band_definitions_hz
            .map(|[lo, hi]| [lo as f32, hi as f32]),
        effective_weights: profile_weights(
            Profile::from_name(&overview.song_meta.profile).unwrap_or_default(),
        )
        .map(|weight| weight as f32),
        structural_summary: DspStructuralSummary {
            climax_bar: to_u32(overview.structural_summary.climax_bar),
            climax_tension: overview.structural_summary.climax_tension as f32,
            dynamic_range_lu: overview.structural_summary.dynamic_range_lu as f32,
            detected_anomalies: overview
                .structural_summary
                .detected_anomalies
                .iter()
                .map(|anomaly| DspAnomaly {
                    bars: [to_u32(anomaly.bars[0]), to_u32(anomaly.bars[1])],
                    anomaly_type: match anomaly.anomaly_type.as_str() {
                        "low_mid_buildup" => DspAnomalyType::LowMidBuildup,
                        "compression_squash" => DspAnomalyType::CompressionSquash,
                        "envelope_deviation" => DspAnomalyType::EnvelopeDeviation,
                        "arrangement_repetition" => DspAnomalyType::ArrangementRepetition,
                        _ => DspAnomalyType::LowMidBuildup,
                    },
                    description: anomaly.description.clone(),
                })
                .collect(),
        },
        repetition: DspRepetitionSummary {
            best_matches: overview
                .repetition
                .best_matches
                .iter()
                .map(|best| DspBestMatch {
                    bar_a: to_u32(best.bar_a),
                    bar_b: to_u32(best.bar_b),
                    similarity: best.similarity as f32,
                })
                .collect(),
            mechanical_index: overview.repetition.mechanical_index as f32,
            repeated_passages: overview
                .repetition
                .repeated_passages
                .iter()
                .map(|passage| DspRepeatedPassage {
                    bars: [to_u32(passage.bars[0]), to_u32(passage.bars[1])],
                    source_bars: [
                        to_u32(passage.source_bars[0]),
                        to_u32(passage.source_bars[1]),
                    ],
                    mean_similarity: passage.mean_similarity as f32,
                })
                .collect(),
        },
        bar_series: DspBarSeries {
            bar: rows.iter().map(|row| to_u32(row.bar)).collect(),
            time_sec: rows.iter().map(|row| row.time_sec as f32).collect(),
            section: rows.iter().map(|row| row.section.clone()).collect(),
            tension: rows.iter().map(|row| row.tension as f32).collect(),
            valence_hue: rows.iter().map(|row| row.valence_hue as f32).collect(),
            lufs_short: rows.iter().map(|row| row.lufs_short as f32).collect(),
            lufs_momentary_max: rows
                .iter()
                .map(|row| row.lufs_momentary_max as f32)
                .collect(),
            score_dyn: rows.iter().map(|row| row.score_dyn as f32).collect(),
            envelope_lo: rows
                .iter()
                .map(|row| row.envelope_lo.map_or(NO_ENVELOPE, |value| value as f32))
                .collect(),
            envelope_hi: rows
                .iter()
                .map(|row| row.envelope_hi.map_or(NO_ENVELOPE, |value| value as f32))
                .collect(),
            target_deviation: rows.iter().map(|row| row.target_deviation as f32).collect(),
            density_pitched_onsets_per_beat: rows
                .iter()
                .map(|row| row.density_pitched_onsets_per_beat as f32)
                .collect(),
            density_pitched_duty: rows
                .iter()
                .map(|row| row.density_pitched_duty as f32)
                .collect(),
            density_perc_onsets_per_beat: rows
                .iter()
                .map(|row| row.density_perc_onsets_per_beat as f32)
                .collect(),
            bands_tilted_db: rows
                .iter()
                .map(|row| row.bands_tilted_db.map(|value| value as f32))
                .collect(),
        },
    };
    render_html_report_with_title(&dsp, title)
}

/// 读 `expected_levels` 实参：缺席 ⇒ 空表；对象 ⇒ 逐项校验（值须为 1–5 级
/// 的整数）；其它形状 ⇒ `INVALID_PARAMETER_RANGE`（`reason: "invalidExpectedLevels"`）。
fn arg_expected_levels(call: &ToolCall) -> Result<BTreeMap<String, u8>, Fault> {
    let Some(value) = call.arguments.get("expected_levels") else {
        return Ok(BTreeMap::new());
    };
    let object = value.as_object().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`expected_levels` 必须是段落名到期望等级（1–5）的对象，如 {\"Chorus\": 5}",
            serde_json::json!({
                "reason": "invalidExpectedLevels",
                "received": value,
            }),
        )
    })?;
    let mut levels = BTreeMap::new();
    for (section, level) in object {
        let level = level
            .as_u64()
            .filter(|&v| (1..=5).contains(&v))
            .ok_or_else(|| {
                Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!("`expected_levels[\"{section}\"]` 必须是 1–5 的整数，实际收到 {level}"),
                    serde_json::json!({
                        "reason": "invalidExpectedLevels",
                        "section": section,
                        "received": level,
                    }),
                )
            })?;
        levels.insert(section.clone(), u8::try_from(level).expect("1–5 在 u8 内"));
    }
    Ok(levels)
}

/// `get_song_overview` 的只读规划（`plan` / `apply` 分工见 `domain/mod.rs` 模块头）。
///
/// # Errors
///
/// 无活跃工程（`NO_ACTIVE_PROJECT`）、`resolution` 非法
/// （`INVALID_PARAMETER_RANGE`）、`profile` 非法（同码 +
/// `reason: "unknownProfile"`）、`expected_levels` 非法（同码 +
/// `reason: "invalidExpectedLevels"`）、宿主未注入 provider（`CONFLICT` +
/// `providerUnavailable`），或 provider 自身响亮失败（原样转发）。
pub fn plan_song_overview(domain: &Domain, call: &ToolCall) -> Result<super::Plan, Fault> {
    let project = require_active(domain)?;
    let resolution = match arg_str(call, "resolution") {
        None | Some("bar") => Resolution::Bar,
        Some("beat") => Resolution::Beat,
        Some(other) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`resolution` 只接受 \"bar\" 或 \"beat\"，实际收到 \"{other}\""),
                serde_json::json!({
                    "reason": "unknownResolution",
                    "accepted": ["bar", "beat"],
                }),
            ));
        }
    };
    let profile = match arg_str(call, "profile") {
        None => Profile::Default,
        Some(name) => Profile::from_name(name).ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`profile` 只接受 \"default\"、\"orchestral\" 或 \"electronic\"，实际收到 \"{name}\""
                ),
                serde_json::json!({
                    "reason": "unknownProfile",
                    "accepted": ["default", "orchestral", "electronic"],
                }),
            )
        })?,
    };
    let format = match arg_str(call, "format") {
        None | Some("json") => OverviewFormat::Json,
        Some("html") => OverviewFormat::Html,
        Some(other) => {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`format` 只接受 \"json\" 或 \"html\"、实际收到 \"{other}\""),
                serde_json::json!({
                    "reason": "unknownFormat",
                    "accepted": ["json", "html"],
                }),
            ));
        }
    };
    let expected_levels = arg_expected_levels(call)?;
    let include_png_path = arg_bool(call, "include_png_path", false);
    if format == OverviewFormat::Html && include_png_path {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`include_png_path` 只对 `format: \"json\"` 生效：HTML 报告没有 `pngPath` 槽位",
            serde_json::json!({
                "reason": "includePngPathRequiresJson",
            }),
        ));
    }
    let provider = domain.song_overview_provider().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::Conflict,
            "本形态没有总览分析 provider：独立 stdio 进程（形态 B）不托管 yeban-dsp 分析，\
             get_song_overview 无法计算总览。",
            serde_json::json!({
                "reason": PROVIDER_UNAVAILABLE_REASON,
                "form": "stdio",
                "providerAttached": false,
                "hint": "请在形态 A（yeban-app 的进程内控制面）里调用；\
                         真实 provider 由 yeban-dsp 线提供，经 Domain::set_song_overview_provider 注入",
            }),
        )
    })?;
    let request = SongOverviewRequest {
        resolution,
        profile,
        expected_levels,
    };
    let overview = provider.overview(project, &request)?;
    let overview = rounded(overview);
    if format == OverviewFormat::Html {
        // HTML 形态：`data` 就是报告字符串本身（不是对象；`commitModel` 只注入对象，
        // 见 `ToolResponse::with_commit_model` 的第二条口径）。
        // 标题取当前活跃工程的 `project.title`：`project` 是本函数开头
        // `require_active(domain)` 取到的那一份，即 `domain.active_project()`；
        // 没有活跃工程时上面那一步已 `NO_ACTIVE_PROJECT` 报错，不必另设回退。
        let html = overview_html_report_with_title(&overview, &project.title);
        return Ok(super::Plan::SongOverview {
            data: Value::String(html),
        });
    }
    let mut data = serde_json::to_value(&overview).map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!("总览序列化失败（实现级状况，不应发生）: {error}"),
        )
    })?;
    let object = data.as_object_mut().expect("SongOverview 序列化后是对象");
    object.insert("provider".to_owned(), Value::from(provider.name()));
    object.insert("resolution".to_owned(), Value::from(resolution.as_str()));
    if include_png_path {
        // Phase 1 只接受参数、不产 PNG（设计稿 §8.1 的 PNG 是 Phase 2 的 Slint
        // 离屏渲染）：`pngPath: null` + 一句如实说明，⛔ 不编造路径。
        object.insert("pngPath".to_owned(), Value::Null);
        object.insert(
            "pngNote".to_owned(),
            Value::from("Phase 2 再接 Slint 离屏渲染（设计稿 §8.1）：本轮只接受参数、不产 PNG。"),
        );
    }
    Ok(super::Plan::SongOverview { data })
}

// ===========================================================================
// 测试替身：StubSongOverviewProvider
//
// 8 小节、120 BPM、4/4、D minor 的固定夹具。张力按设计稿 §3.2 的公式从每小节
// 的分量算出（不是手抄的"看起来对"的数），异常条目是手算的固定值。
// 独立重算的对照在 `crates/yeban-mcp/tests/song_overview.rs` 里：
// 那里的重算函数与这里的公式是**两份独立实现**，同一组分量必须逐位一致。
// ===========================================================================

/// 一小节的 stub 分量（§3.2 的五个张力分量 + §8.2 的行字段）。
///
/// 字段顺序 = 下面 `FIXTURE_BARS` 逐行的顺序：
/// `lufs_short` / `lufs_momentary_max` / `score_dyn` /
/// `density_pitched_onsets_per_beat` / `density_pitched_duty` /
/// `density_perc_onsets_per_beat` / `pitch_center_midi` / `harmony` /
/// `active_tracks` / `octave_bandwidth` / `valence_hue` / `density` / `section`。
#[derive(Clone, Copy, Debug)]
struct FixtureBar {
    lufs_short: f64,
    lufs_momentary_max: f64,
    score_dyn: f64,
    density_pitched_onsets_per_beat: f64,
    density_pitched_duty: f64,
    density_perc_onsets_per_beat: f64,
    pitch_center_midi: f64,
    harmony: f64,
    active_tracks: f64,
    octave_bandwidth: f64,
    valence_hue: f64,
    /// 预计算归一化密度 `D̂ = clamp(ln(1+d)/ln(17))`（`d` = 每秒等效触发数，
    /// 设计稿 §3.2；6 位小数，与现场算的 2 位小数取整逐位相同）。
    /// 夹具数据固定 ⇒ 密度是常量，直接写死；生产区不得调用超越函数
    /// （`probe_authenticity` R56：`.ln()` 等会让位级硬断言变成跨平台假设）。
    density: f64,
    section: &'static str,
}

/// 8 小节夹具分量（`bands_tilted_db` 另见 [`FIXTURE_BANDS`]）。
const FIXTURE_BARS: [FixtureBar; 8] = [
    FixtureBar {
        lufs_short: -22.4,
        lufs_momentary_max: -19.8,
        score_dyn: 0.28,
        density_pitched_onsets_per_beat: 1.5,
        density_pitched_duty: 0.65,
        density_perc_onsets_per_beat: 0.5,
        pitch_center_midi: 57.0,
        harmony: 0.20,
        active_tracks: 2.0,
        octave_bandwidth: 1.5,
        valence_hue: -0.62,
        density: 0.568061,
        section: "Intro",
    },
    FixtureBar {
        lufs_short: -20.1,
        lufs_momentary_max: -17.5,
        score_dyn: 0.34,
        density_pitched_onsets_per_beat: 2.0,
        density_pitched_duty: 0.68,
        density_perc_onsets_per_beat: 1.0,
        pitch_center_midi: 59.0,
        harmony: 0.25,
        active_tracks: 3.0,
        octave_bandwidth: 2.0,
        valence_hue: -0.58,
        density: 0.686821,
        section: "Intro",
    },
    FixtureBar {
        lufs_short: -18.6,
        lufs_momentary_max: -16.0,
        score_dyn: 0.42,
        density_pitched_onsets_per_beat: 2.5,
        density_pitched_duty: 0.70,
        density_perc_onsets_per_beat: 1.0,
        pitch_center_midi: 60.0,
        harmony: 0.30,
        active_tracks: 3.0,
        octave_bandwidth: 2.0,
        valence_hue: -0.55,
        density: 0.733952,
        section: "Verse",
    },
    FixtureBar {
        lufs_short: -16.2,
        lufs_momentary_max: -13.6,
        score_dyn: 0.48,
        density_pitched_onsets_per_beat: 3.0,
        density_pitched_duty: 0.72,
        density_perc_onsets_per_beat: 2.0,
        pitch_center_midi: 62.0,
        harmony: 0.35,
        active_tracks: 4.0,
        octave_bandwidth: 2.5,
        valence_hue: -0.50,
        density: 0.846352,
        section: "Verse",
    },
    FixtureBar {
        lufs_short: -13.8,
        lufs_momentary_max: -11.2,
        score_dyn: 0.52,
        density_pitched_onsets_per_beat: 3.5,
        density_pitched_duty: 0.75,
        density_perc_onsets_per_beat: 2.5,
        pitch_center_midi: 64.0,
        harmony: 0.42,
        active_tracks: 5.0,
        octave_bandwidth: 3.0,
        valence_hue: -0.44,
        density: 0.905315,
        section: "Chorus",
    },
    FixtureBar {
        lufs_short: -14.0,
        lufs_momentary_max: -11.4,
        score_dyn: 0.40,
        density_pitched_onsets_per_beat: 2.0,
        density_pitched_duty: 0.60,
        density_perc_onsets_per_beat: 1.0,
        pitch_center_midi: 60.0,
        harmony: 0.30,
        active_tracks: 3.0,
        octave_bandwidth: 2.0,
        valence_hue: -0.52,
        density: 0.686821,
        section: "Chorus",
    },
    FixtureBar {
        lufs_short: -13.6,
        lufs_momentary_max: -11.0,
        score_dyn: 0.54,
        density_pitched_onsets_per_beat: 4.0,
        density_pitched_duty: 0.78,
        density_perc_onsets_per_beat: 3.0,
        pitch_center_midi: 65.0,
        harmony: 0.48,
        active_tracks: 6.0,
        octave_bandwidth: 3.5,
        valence_hue: -0.40,
        density: 0.955823,
        section: "Chorus",
    },
    FixtureBar {
        lufs_short: -12.4,
        lufs_momentary_max: -9.8,
        score_dyn: 0.58,
        density_pitched_onsets_per_beat: 4.5,
        density_pitched_duty: 0.80,
        density_perc_onsets_per_beat: 3.5,
        pitch_center_midi: 67.0,
        harmony: 0.55,
        active_tracks: 7.0,
        octave_bandwidth: 4.0,
        valence_hue: -0.35,
        density: 1.0,
        section: "Outro",
    },
];

/// 8 小节夹具的 8 频段倾斜能量（dB，§4.3；列序 = [`BAND_DEFINITIONS_HZ`]）。
///
/// 第 5、6 小节的 Band 3（250–500 Hz）为 −6.8 dB，其余小节约 −12.0 dB ⇒
/// 超量 **+5.2 dB**（异常条目 `low_mid_buildup` 的算术，见 [`fixture_anomalies`]）。
const FIXTURE_BANDS: [[f64; 8]; 8] = [
    [-18.2, -14.5, -12.0, -11.4, -15.0, -19.2, -24.0, -31.5],
    [-17.0, -13.8, -12.1, -11.0, -14.6, -18.8, -23.5, -31.0],
    [-16.2, -13.2, -11.9, -10.6, -14.0, -18.0, -22.8, -30.2],
    [-15.0, -12.4, -12.2, -10.2, -13.5, -17.4, -22.0, -29.5],
    [-14.2, -11.8, -6.8, -10.0, -13.2, -17.0, -21.6, -29.0],
    [-14.6, -12.0, -6.8, -10.4, -13.6, -17.5, -22.2, -29.6],
    [-13.8, -11.2, -12.0, -9.8, -13.0, -16.8, -21.2, -28.6],
    [-13.0, -10.6, -11.8, -9.4, -12.6, -16.2, -20.8, -28.0],
];

/// 夹具的异常条目（**手算固定值**，不是检测算法；算术逐条写明）：
///
/// - `low_mid_buildup` [5, 6]：Band 3 在这两小节为 −6.8 dB，其余小节约
///   −12.0 dB ⇒ 超量 **+5.2 dB**；
/// - `compression_squash` [7, 8]：谱面力度从第 6 小节 0.40 涨到第 7 小节
///   0.54（**+35.0%**），而渲染 LUFS 只从 −14.0 变到 −13.6（**+0.4 LU**）。
///
/// 偏离告警阈值（Phase 2）：实际张力超出期望带边界超过 0.15 即告警。
/// 与 dsp 的 `ENVELOPE_DEVIATION_ALERT` 同值（dsp 是唯一事实源；stub 侧
/// 的复制值由 mcp 测试钉死，见 `envelope_deviation_anomaly_appears`）。
pub const ENVELOPE_DEVIATION_ALERT: f64 = 0.15;

/// 期望带偏离异常（Phase 2）：把连续超阈的小节合并成一条
/// `envelope_deviation` 异常（`bars: [起, 止]`，含端点；描述带最大偏离量）。
/// 输入是小节级 `(bar, section, deviation)`，须按小节升序。
fn envelope_anomalies(deviations: &[(u64, String, f64)]) -> Vec<DetectedAnomaly> {
    let mut anomalies = Vec::new();
    let mut run: Option<(u64, u64, String, f64)> = None;
    let flush = |run: &mut Option<(u64, u64, String, f64)>,
                 anomalies: &mut Vec<DetectedAnomaly>| {
        if let Some((start, end, section, max_deviation)) = run.take() {
            anomalies.push(DetectedAnomaly {
                bars: [start, end],
                anomaly_type: "envelope_deviation".to_owned(),
                description: format!(
                    "Section \"{section}\" tension deviates from its expected band by up to {max_deviation:.2}"
                ),
            });
        }
    };
    for &(bar, ref section, deviation) in deviations {
        match &mut run {
            Some((_, end, run_section, max_deviation))
                if bar == *end + 1 && section == run_section =>
            {
                *end = bar;
                *max_deviation = max_deviation.max(deviation);
            }
            _ => {
                flush(&mut run, &mut anomalies);
                run = Some((bar, bar, section.clone(), deviation));
            }
        }
    }
    flush(&mut run, &mut anomalies);
    anomalies
}

fn fixture_anomalies() -> Vec<DetectedAnomaly> {
    vec![
        DetectedAnomaly {
            bars: [5, 6],
            anomaly_type: "low_mid_buildup".to_owned(),
            description: "Band 3 (250-500Hz) exceeds tilted balance by +5.2 dB".to_owned(),
        },
        DetectedAnomaly {
            bars: [7, 8],
            anomaly_type: "compression_squash".to_owned(),
            description: "Score velocity rises +35% but rendered LUFS changes only +0.4 LU"
                .to_owned(),
        },
    ]
}

/// §3.2 的张力公式（固定心理声学锚点，⛔ 不用全曲 Min-Max）。
///
/// `T = w_L·L̂ + w_D·D̂ + w_P·P̂ + w_H·Ĥ + w_O·Ô`，其中
/// `L̂ = clamp((LUFS + 36) / 28)`（锚点 −36 / −8 LUFS）、
/// `D̂ = clamp(ln(1 + d) / ln(17))`（每秒 16 音符满分）、
/// `P̂ = clamp((p − 36) / 48)`（锚点 C2 / C6）、
/// `Ô = 0.5·min(N/8, 1) + 0.5·min(B/5, 1)`。
/// 密度项 `D̂ = clamp(ln(1+d)/ln(1+16))` 里 `d` 是**每秒等效触发数**
/// （设计稿 §3.2：速度是密度计算的时基乘数）。stub 的夹具只有"每拍触发数"，
/// 故 `d = (旋律起音/拍 + 打击起音/拍) × 拍/秒`（120 BPM ⇒ 2 拍/秒）。
/// ⚠ 两处与真实 provider 的差别（stub 口径，如实写明）：
/// ① 真实 provider 用 dsp 的聚类后 Onset 数（同拍 30 ms 内柱式和弦记 1 次），
/// stub 没有逐音时序、聚不了类，用未聚类的两组之和（聚类数的上界）；
/// ② 占空比 `density_pitched_duty` 不进密度公式（只作行字段回传，与 dsp 同口径）。
/// 权重按请求的曲风档取（Phase 2）：唯一事实源是 dsp 的
/// `TensionProfile::weights`，这里的表与之逐位一致（见 `profile_weights`）。
/// 密度项 `D̂` 直接读 `FixtureBar::density` 预计算值（现场算含 `ln`，
/// 生产区调用超越函数会红 `probe_authenticity`（R56）；夹具数据固定，
/// 预计算值与公式的 2 位小数取整逐位相同）。
fn tension_of(bar: &FixtureBar, weights: [f64; 5]) -> f64 {
    let loudness = ((bar.lufs_short + 36.0) / 28.0).clamp(0.0, 1.0);
    let pitch = ((bar.pitch_center_midi - 36.0) / 48.0).clamp(0.0, 1.0);
    let orchestration =
        0.5 * (bar.active_tracks / 8.0).min(1.0) + 0.5 * (bar.octave_bandwidth / 5.0).min(1.0);
    weights[0] * loudness
        + weights[1] * bar.density
        + weights[2] * pitch
        + weights[3] * bar.harmony
        + weights[4] * orchestration
}

/// 曲风档权重（Phase 2）：与 dsp `TensionProfile::weights` 逐位一致。
/// Default [0.30, 0.20, 0.20, 0.15, 0.15]；Orchestral [0.25, 0.15, 0.30, 0.20, 0.10]；
/// Electronic [0.35, 0.30, 0.10, 0.10, 0.15]。
/// ⚠ 方差自适应**不在** stub 里：夹具是 8 行固定值、无逐拍分量，算不出 σ；
/// stub 只证“档位换权重 ⇒ 张力跟着换”这一层，自适应由 dsp 专测钉死。
fn profile_weights(profile: Profile) -> [f64; 5] {
    match profile {
        Profile::Default => [0.30, 0.20, 0.20, 0.15, 0.15],
        Profile::Orchestral => [0.25, 0.15, 0.30, 0.20, 0.10],
        Profile::Electronic => [0.35, 0.30, 0.10, 0.10, 0.15],
    }
}

/// 期望等级 → 期望带（Phase 2，dsp `envelope_band` 的 f64 复算）：
/// 中心 `0.1 + 0.2 × (L − 1)`，半宽 0.15，夹到 [0, 1]。
fn envelope_band_f64(level: u8) -> (f64, f64) {
    let center = 0.1 + 0.2 * f64::from(level - 1);
    ((center - 0.15).max(0.0), (center + 0.15).min(1.0))
}

/// 测试替身 provider：对夹具工程返回上面那组固定序列。
///
/// ⚠ 只供测试与本地开发用：真实 provider 由 dsp 线提供（见模块头）。
#[derive(Clone, Copy, Debug, Default)]
pub struct StubSongOverviewProvider;

impl SongOverviewProvider for StubSongOverviewProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn overview(
        &self,
        _project: &YebanProjectV1,
        request: &SongOverviewRequest,
    ) -> Result<SongOverview, Fault> {
        let weights = profile_weights(request.profile);
        let mut bar_series: Vec<BarRow> = Vec::new();
        // 小节级偏离（拍级展开前收集：异常按小节合并，与 dsp 同口径）。
        let mut envelope_deviations: Vec<(u64, String, f64)> = Vec::new();
        for (index, bar) in FIXTURE_BARS.iter().enumerate() {
            let bar_no = index as u64 + 1;
            let bar_start_sec = index as f64 * 2.0;
            let tension = tension_of(bar, weights);
            // 期望带（Phase 2）：按段落名查期望等级表；有等级时填带与偏离量，
            // 无等级时两端为 None、偏离为 0.0（与 dsp 的 −1 哨兵对应，见 app 映射）。
            let (envelope_lo, envelope_hi, target_deviation) =
                match request.expected_levels.get(bar.section) {
                    Some(&level) => {
                        let (lo, hi) = envelope_band_f64(level);
                        let deviation = if tension < lo {
                            lo - tension
                        } else if tension > hi {
                            tension - hi
                        } else {
                            0.0
                        };
                        (Some(lo), Some(hi), deviation)
                    }
                    None => (None, None, 0.0),
                };
            if target_deviation > ENVELOPE_DEVIATION_ALERT {
                envelope_deviations.push((bar_no, bar.section.to_owned(), target_deviation));
            }
            let row = BarRow {
                bar: bar_no,
                beat: None,
                time_sec: bar_start_sec,
                section: bar.section.to_owned(),
                tension,
                valence_hue: bar.valence_hue,
                lufs_short: bar.lufs_short,
                lufs_momentary_max: bar.lufs_momentary_max,
                score_dyn: bar.score_dyn,
                envelope_lo,
                envelope_hi,
                target_deviation,
                density_pitched_onsets_per_beat: bar.density_pitched_onsets_per_beat,
                density_pitched_duty: bar.density_pitched_duty,
                density_perc_onsets_per_beat: bar.density_perc_onsets_per_beat,
                bands_tilted_db: FIXTURE_BANDS[index],
            };
            match request.resolution {
                Resolution::Bar => bar_series.push(row),
                // stub 的拍级展开（文档化的替身行为）：4/4 拍，一小节 4 拍，
                // 每拍 0.5 s，值沿用所属小节。真实 provider 在 Phase 1 也走同一条
                // 继承口径：dsp 只输出小节级序列，逐拍重算需要 dsp 的拍级输出，
                // 那是 Phase 2 的事（见 `yeban-app` 的 `song_overview_provider`）。
                Resolution::Beat => {
                    for beat in 1..=4u64 {
                        let mut beat_row = row.clone();
                        beat_row.beat = Some(beat);
                        beat_row.time_sec = bar_start_sec + (beat - 1) as f64 * 0.5;
                        bar_series.push(beat_row);
                    }
                }
            }
        }
        let climax = bar_series
            .iter()
            .max_by(|left, right| {
                left.tension
                    .partial_cmp(&right.tension)
                    .expect("张力是有限数")
            })
            .expect("夹具至少有一行");
        let loudest = bar_series
            .iter()
            .map(|row| row.lufs_short)
            .fold(f64::NEG_INFINITY, f64::max);
        let quietest = bar_series
            .iter()
            .map(|row| row.lufs_short)
            .fold(f64::INFINITY, f64::min);
        let mut detected_anomalies = fixture_anomalies();
        detected_anomalies.extend(envelope_anomalies(&envelope_deviations));
        Ok(SongOverview {
            schema_version: "1.1".to_owned(),
            song_meta: SongMeta {
                total_bars: FIXTURE_BARS.len() as u64,
                duration_sec: 16.0,
                base_bpm: 120.0,
                key_signature: "D minor".to_owned(),
                profile: request.profile.as_str().to_owned(),
            },
            band_definitions_hz: BAND_DEFINITIONS_HZ,
            structural_summary: StructuralSummary {
                climax_bar: climax.bar,
                climax_tension: climax.tension,
                dynamic_range_lu: loudest - quietest,
                detected_anomalies,
            },
            // stub 不算重复检测（检测在 dsp；见模块头"diagnostics 的归属"）：
            // 8 行夹具不声称重复与否，字段只保形状完整，值为空摘要。
            repetition: RepetitionSummary {
                best_matches: Vec::new(),
                mechanical_index: 0.0,
                repeated_passages: Vec::new(),
            },
            bar_series,
        })
    }
}
