//! `get_song_overview` 的**真实 provider**（设计稿 `yeban-songmap-design.md`
//! §8，Phase 1）：把 B 线管线（`yeban_render::song_overview::cached_overview`：
//! 谱面映射 → 离线渲染 → dsp 四层分析 → 整曲哈希缓存）的输出映射成
//! MCP 的 §8.2 JSON structs。
//!
//! ## 为什么住在这里（`yeban-mcp`，两种形态共用同一份实现）
//!
//! 本模块原住 `yeban-app`，于是只有形态 A（进程内控制面）能注入它，
//! 形态 B（独立 stdio 二进制）里 `get_song_overview` 恒回
//! `providerUnavailable`（清单 `05_yeban.md` #3 / #4）。迁入 `yeban-mcp`
//! 后果：两个形态构造的是**同一个** provider，测量语义只有一份。
//!
//! 迁入时依赖方向逐条对过守卫
//! `scripts/gates/check_mcp_dependency_direction.py`（`FORBIDDEN` 逐字含
//! `yeban-dsp` / `yeban-engine` 的**直接**边）：
//!
//! - 分析类型与入口（`AnalysisOptions` / `TensionProfile` /
//!   `analyze_song_overview_with_options`）经 `yeban-render` 的**再导出**
//!   取得 —— `yeban-mcp → yeban-render` 是已登记例外那条边，再导出不新增
//!   依赖边（`yeban_render::song_overview` 模块头有同款先例）；
//! - SFZ 武装不再走 `yeban-engine` 的 `SfzInstrument` + app 私有的
//!   `sfz_load`，改走 [`crate::domain::sfz`] 的装载实现
//!   （[`load_with`](crate::domain::sfz::load_with) /
//!   [`load_with_embedded`](crate::domain::sfz::load_with_embedded)）——
//!   与 `yeban_render_master` 的 `prepare_sampler_track` **同一份**装载：
//!   同一条路径解析（[`crate::domain::render::resolve_sampler_path`]：
//!   工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd）、同一份 `#include` 展开、
//!   同一条样本解码分派（WAV 走 hound 腿、其余走 `yeban-decode`）。
//!   两处与渲染路径的**刻意**差别：⛔ 不带需求过滤（`wanted: None` ——
//!   总览/测量要的是整份乐器的提供表，与旧 app provider 的 `arm_all`
//!   全集口径一致），⛔ 不带样本预算上限（预算是 `yeban_render_master`
//!   的实参语义，不是分析的语义）。
//!
//! ## 上下文（工程目录 + CAS 池）从哪来：[`ProviderContext`]
//!
//! provider 的磁盘腿要工程目录、内嵌腿要会话 CAS 池 —— 两者都是
//! **会话状态**，不是构造期常量。宿主两种接法：
//!
//! - **固定快照**：形态 A 挂载时按当次工程构造一份
//!   [`ProviderContext::shared`] 交进来（会话与工程一对一，快照不会过期）；
//!   `--export-song-overview` 批处理同款（一次性进程）；
//! - **Domain 托管**：形态 B 在启动时调
//!   [`crate::domain::Domain::install_real_providers`]，provider 读的是
//!   `Domain` 的那份共享单元；`Domain` 在每次工具取 provider 时按**当前
//!   活跃工程**同步它（路径 + 资产键集变化才真换池），因此 stdio 会话里
//!   开/换工程不需要重装 provider。
//!
//! ## 预检为什么必须在 `cached_overview` 之前
//!
//! `EngineTimbre::from_project_armed` 的契约是"**provider 是可靠的**"：
//! 它不返回 `Result`，失败由调用方**在调用之前**用自己的错误类型响亮报出。
//! `cached_overview` 会把算出的结果**存进缓存** —— 若先让它渲染、事后再发现
//! 某份 SFZ 读不到而报错，缓存里已经躺着一份用空采样表算出的静音总览，
//! 下一次调用直接命中缓存、静默返回错误数据。因此预检先行：
//! 每一份待摊平的 SFZ 都在渲染之前解析 + 解码，任一失败 ⇒ 整个调用
//! `RENDER_FAILED` 响亮失败，缓存里什么都不留。
//!
//! 预检的枚举与武装步**同一份集合**：待摊平项 = `track.sampler.is_some()`
//! 的轨（`EngineSnapshot::from_project` 原样抄进 `sfz_pending` 的规则），
//! 因此"预检过了、武装时又冒出一轨"不可能发生。
//!
//! ## SFZ 两条腿（与 MCP `prepare_sampler_track` 同口径）
//!
//! - **磁盘腿**：[`crate::domain::render::resolve_sampler_path`] 解析绑定
//!   路径 → [`crate::domain::sfz::load_with`] 一次解析同时产出装载读数与
//!   提供表 → 读不到的样本不进提供表（引擎标 `NO_SAMPLE`），SFZ 文件
//!   本身读不到 / 文本非法 ⇒ 响亮失败（`samplerUnreadable`）。
//! - **内嵌腿**：绑定登记在 `project.embedded_samples` 里 ⇒ SFZ 文本与
//!   样本都从会话 CAS 池取（[`crate::domain::sfz::load_with_embedded`]）；
//!   含 `#include` 的内嵌文本响亮拒绝（保存侧从不内嵌这类 SFZ，
//!   静默忽略会改区域表）。
//!
//! **缺样本不是错误**（两条腿一致）：某份 `sample=` 读不到 ⇒ 不进提供表，
//! 引擎把该区域标 `NO_SAMPLE`（触发时不建声部）＋计数。
//!
//! ## 无音频（纯谱面）
//!
//! 渲染失败与无音频是两回事：工程能渲染但某轨没出声、或整曲无音频时，
//! dsp 按约定出哨兵值（LUFS −70、频段 −120 dB），调用正常返回。
//! 只有**渲染本身**失败（缺 SFZ、无内容、采样率不一致）才响亮报错。
//!
//! ## 拍级展开（Phase 1 的诚实口径，Phase 2 未变）
//!
//! dsp 只输出**小节级**序列。`resolution: "beat"` 时每行继承所属小节的值，
//! `time_sec` 按 `60 / base_bpm` 步进。逐拍重算需要 dsp 的拍级输出，
//! 不在 Phase 2 核心三项内 —— 这里不假装算过。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{Value, json};

use yeban_model::song_overview::score_input_from_project;
use yeban_model::{AssetHash, EntityId, YebanProjectV1};
use yeban_render::engine_source::{ArmedSfzInput, ArmedSfzSample};
use yeban_render::song_overview::{
    AnalysisOptions, ClashSeverity as DspClashSeverity, NO_ENVELOPE, RenderSettings,
    SongOverview as DspSongOverview, SongOverviewCache, TensionProfile,
    analyze_song_overview_with_options, cached_overview, song_overview_hash,
};

use crate::domain::error::Fault;
use crate::domain::render::resolve_sampler_path;
use crate::domain::sfz::{
    SampleLimits, SampleStore, SampledProviderTable, TrackSampleCache, load_with,
    load_with_embedded,
};
use crate::domain::song_overview::{
    Profile as McpProfile, Resolution, SongOverviewProvider, SongOverviewRequest,
};
use crate::output_schema::{
    BAND_DEFINITIONS_HZ, BarOverlap, BarRow, BestMatch, ClashTrackRef, DetectedAnomaly,
    FrequencyClashEntry, FrequencyClashSummary, RepeatedPassage, RepetitionSummary, SongMeta,
    SongOverview as McpSongOverview, StructuralSummary,
};
use crate::tools::ErrorCode;

/// 真实 provider 的自述名（进响应 `data.provider`；stub 的是 `"stub"`）。
pub const PROVIDER_NAME: &str = "real";

/// provider 的会话上下文：磁盘腿的工程目录 + 内嵌腿的会话 CAS 池。
///
/// 它是**会话运行态**（`MODEL-ISO-001` 第 2 层）：不进 `project.json`、
/// 不进 `history.dag`。宿主经 [`ProviderContext::shared`] 造一份共享单元
/// 交给 provider；`Domain` 托管形态下（见模块文档）由 `Domain` 在每次取
/// provider 时调 [`ProviderContext::sync_from`] 按活跃工程刷新 ——
/// 刷新按（工程路径、资产键集）判定，键没变就不动池（⛔ 不逐调用深拷 CAS）。
#[derive(Debug)]
pub struct ProviderContext {
    /// 磁盘腿 SFZ 解析的工程目录（`None` = 内存会话 / 无磁盘对应物）。
    project_dir: Option<PathBuf>,
    /// 会话 CAS 池（内嵌腿的 SFZ 文本与样本由此取；没有内嵌绑定时一次都不读）。
    assets: Arc<BTreeMap<AssetHash, Vec<u8>>>,
    /// 上一次 [`ProviderContext::sync_from`] 记下的同步键
    /// （活跃工程路径 + 资产键集）；固定快照形态下恒为 `None`（从不步）。
    synced_key: Option<(Option<PathBuf>, Vec<AssetHash>)>,
}

/// [`ProviderContext`] 的共享单元（宿主与 provider 各持一份句柄）。
pub type SharedProviderContext = Arc<RwLock<ProviderContext>>;

impl ProviderContext {
    /// 造一份固定快照的共享上下文（形态 A 挂载 / 批处理导出用）。
    #[must_use]
    pub fn shared(
        project_dir: Option<PathBuf>,
        assets: Arc<BTreeMap<AssetHash, Vec<u8>>>,
    ) -> SharedProviderContext {
        Arc::new(RwLock::new(Self {
            project_dir,
            assets,
            synced_key: None,
        }))
    }

    /// 磁盘腿 SFZ 解析的工程目录。
    #[must_use]
    pub fn project_dir(&self) -> Option<&Path> {
        self.project_dir.as_deref()
    }

    /// 会话 CAS 池（共享句柄；克隆只加引用计数）。
    #[must_use]
    pub fn assets(&self) -> Arc<BTreeMap<AssetHash, Vec<u8>>> {
        Arc::clone(&self.assets)
    }

    /// 按**当前活跃工程**同步（`Domain` 托管形态专用）：工程路径或资产
    /// 键集变了才真换目录与池；没变就原样返回（⛔ 不深拷）。
    ///
    /// 工程目录的推导与形态 A 挂载时同一条：工程文件路径的父目录，
    /// 空父目录（纯文件名）视为没有目录。
    pub(crate) fn sync_from(
        &mut self,
        active_path: Option<&Path>,
        assets: Option<&BTreeMap<AssetHash, Vec<u8>>>,
    ) {
        let key = (
            active_path.map(Path::to_path_buf),
            assets
                .map(|pool| pool.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        if self.synced_key.as_ref() == Some(&key) {
            return;
        }
        self.project_dir = active_path
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf);
        self.assets = Arc::new(assets.cloned().unwrap_or_default());
        self.synced_key = Some(key);
    }
}

/// 武装时的样本预算：不限（`0` = 不限，见 [`SampleLimits`] 的字段口径）。
///
/// 总览/测量的武装与 `yeban_render_master` 不同：渲染的预算是**调用方
/// 实参**，分析没有这个实参，旧 app provider 的 `arm_all` 也从不带预算 ——
/// 迁入后保持同一口径，⛔ 不在这里发明一个上限。
const UNLIMITED_SAMPLE_LIMITS: SampleLimits = SampleLimits {
    max_files: 0,
    max_frames: 0,
};

/// 整曲总览图的真实 provider：B 线管线 + §8.2 映射 + 整曲哈希缓存。
///
/// `Debug` 是 trait 约束的一部分（`Domain` 派生 `Debug`）。
#[derive(Debug)]
pub struct RealSongOverviewProvider {
    /// 整曲哈希缓存（`cached_overview` 的那一份；命中时不渲染、不解码）。
    cache: Mutex<SongOverviewCache>,
    /// 会话上下文（工程目录 + CAS 池；见 [`ProviderContext`]）。
    context: SharedProviderContext,
}

impl RealSongOverviewProvider {
    /// 造一个真实 provider（上下文的两种接法见 [`ProviderContext`]）。
    #[must_use]
    pub fn new(context: SharedProviderContext) -> Self {
        Self {
            cache: Mutex::new(SongOverviewCache::new()),
            context,
        }
    }

    /// 缓存命中次数（判据用：第二次同一调用不重算的证据）。
    #[must_use]
    pub fn cache_hits(&self) -> u64 {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .hits()
    }

    /// 缓存未命中次数。
    #[must_use]
    pub fn cache_misses(&self) -> u64 {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .misses()
    }

    /// 此刻的上下文快照（工程目录 + CAS 池句柄；只克隆句柄，不克隆字节）。
    fn context_snapshot(&self) -> (Option<PathBuf>, Arc<BTreeMap<AssetHash, Vec<u8>>>) {
        let context = self
            .context
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            context.project_dir().map(Path::to_path_buf),
            context.assets(),
        )
    }

    /// 预检：把工程里全部待摊平的 SFZ 武装成 `ArmedSfzInput`。
    ///
    /// # Errors
    ///
    /// 任一 SFZ 文件读不到 / 文本非法 / 内嵌资产缺失 ⇒ `RENDER_FAILED`
    ///（`reason: "samplerUnreadable"`，含绑定路径与试过的根）。
    /// 缺**样本**不是错误（见模块文档：不进提供表，引擎标 `NO_SAMPLE`）。
    pub(crate) fn arm_all(
        &self,
        project: &YebanProjectV1,
        engine_hz: f32,
    ) -> Result<BTreeMap<EntityId, ArmedSfzInput>, Fault> {
        let (project_dir, assets) = self.context_snapshot();
        // 一次武装共享一个 SampleStore：同一份样本被多条轨引用时只解码一次
        // （与 `yeban_render_master` 的渲染期口径相同）。
        let mut store = SampleStore::new();
        let mut armed = BTreeMap::new();
        for (track, binding) in project
            .tracks
            .iter()
            .filter_map(|(id, track)| track.sampler.as_ref().map(|sampler| (id, sampler)))
        {
            let input = self.arm_track(
                &binding.path,
                project,
                engine_hz,
                &mut store,
                project_dir.as_deref(),
                &assets,
            )?;
            armed.insert(*track, input);
        }
        Ok(armed)
    }

    /// 武装一条轨的 SFZ：内嵌腿与磁盘腿按绑定登记分流。
    fn arm_track(
        &self,
        raw: &str,
        project: &YebanProjectV1,
        engine_hz: f32,
        store: &mut SampleStore,
        project_dir: Option<&Path>,
        assets: &BTreeMap<AssetHash, Vec<u8>>,
    ) -> Result<ArmedSfzInput, Fault> {
        if project.embedded_samples.contains_key(raw) {
            self.arm_embedded(raw, project, engine_hz, assets)
        } else {
            self.arm_disk(raw, engine_hz, store, project_dir)
        }
    }

    /// 磁盘腿：解析 → 一次装载（解析 + 逐样本解码）→ 提供表。
    fn arm_disk(
        &self,
        raw: &str,
        engine_hz: f32,
        store: &mut SampleStore,
        project_dir: Option<&Path>,
    ) -> Result<ArmedSfzInput, Fault> {
        // `resolve_sampler_path` 收的是**工程文件**路径、内部取 `parent()`
        // 当工程目录 ⇒ 这里按 `domain::sfz::resolve_load_path` 同一条补锚点：
        // 有目录时补一个文件名；没有时给不存在的相对名（`parent()` 为空
        // ⇒ 工程目录档整档跳过）。
        let anchor = match project_dir {
            Some(dir) => dir.join("__active_project__.yeban"),
            None => PathBuf::from("__no_project__"),
        };
        // 解析失败的 Fault 本身就是 `RENDER_FAILED` + `samplerUnreadable`
        // + `tried` 全集（`domain::render` 的唯一实现），原样透出。
        let resolved = resolve_sampler_path(raw, &anchor)?;
        let mut cache = TrackSampleCache::with_limits(store, UNLIMITED_SAMPLE_LIMITS);
        let (load, table) = load_with(
            &resolved,
            engine_hz,
            0,
            false,
            &mut cache,
            None,
            &BTreeMap::new(),
        )
        .map_err(|fault| {
            sampler_fault(
                raw,
                format!("采样器定义文件读不到或解析不了: {}", fault_message(&fault)),
                Some(resolved.display().to_string()),
            )
        })?;
        Ok(ArmedSfzInput::new(load.path, samples_from_table(table)))
    }

    /// 内嵌腿：SFZ 文本与样本全从会话 CAS 池取，⛔ 不读盘。
    fn arm_embedded(
        &self,
        raw: &str,
        project: &YebanProjectV1,
        engine_hz: f32,
        assets: &BTreeMap<AssetHash, Vec<u8>>,
    ) -> Result<ArmedSfzInput, Fault> {
        let hash_text = project
            .embedded_samples
            .get(raw)
            .expect("调用方已确认绑定登记在 embedded_samples 里");
        let hash = AssetHash::parse(hash_text).map_err(|error| {
            sampler_fault(
                raw,
                format!("内嵌采样器绑定 `{raw}` 的哈希 `{hash_text}` 非法: {error}"),
                None,
            )
        })?;
        let bytes = assets.get(&hash).ok_or_else(|| {
            sampler_fault(
                raw,
                format!("内嵌采样器定义文件 `{raw}` 的资产 `{hash}` 不在会话 CAS 池里"),
                None,
            )
        })?;
        let text = String::from_utf8(bytes.clone()).map_err(|error| {
            sampler_fault(
                raw,
                format!("内嵌采样器定义文件 `{raw}` 不是合法 UTF-8: {error}"),
                None,
            )
        })?;
        // `load_with_embedded` 同样会拒绝 `#include`（它走真实的 include
        // 探测）；这里先按行扫一遍，是为了给出与旧 app provider 逐字相同的
        // 报文（保存侧从不内嵌这类 SFZ，静默忽略会改区域表）。
        if text
            .lines()
            .any(|line| line.trim_start().starts_with("#include"))
        {
            return Err(sampler_fault(
                raw,
                "内嵌采样器定义文件含 `#include`，Phase 1 不展开（请用磁盘腿）".to_owned(),
                None,
            ));
        }
        let (load, table) = load_with_embedded(
            text.clone(),
            hash_text.clone(),
            raw,
            &project.embedded_samples,
            assets,
            engine_hz,
            0,
            false,
            UNLIMITED_SAMPLE_LIMITS,
            None,
        )
        .map_err(|fault| {
            sampler_fault(
                raw,
                format!(
                    "内嵌采样器定义文件 `{raw}` 文本非法: {}",
                    fault_message(&fault)
                ),
                None,
            )
        })?;
        Ok(ArmedSfzInput::new(load.path, samples_from_table(table)).with_sfz_text(text))
    }
}

impl SongOverviewProvider for RealSongOverviewProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    fn overview(
        &self,
        project: &YebanProjectV1,
        request: &SongOverviewRequest,
    ) -> Result<McpSongOverview, Fault> {
        let sample_rate_hz = project.audio_config.sample_rate.hz();
        let settings = RenderSettings::new(sample_rate_hz, 0);
        // Phase 2（设计稿 §3.5 / §3.6）：请求里的曲风档与期望等级表进分析
        // 选项；档名是 MCP 侧的词，这里映射成 dsp 的档（两档同名同义）。
        let options = AnalysisOptions {
            profile: match request.profile {
                McpProfile::Default => TensionProfile::Default,
                McpProfile::Orchestral => TensionProfile::Orchestral,
                McpProfile::Electronic => TensionProfile::Electronic,
            },
            expected_levels: request.expected_levels.clone(),
        };
        // 缓存命中时不预检、不渲染、不解码（预检的 I/O 只在未命中时付一次）。
        // 注意：`SongOverviewCache::get` 不计命中数（见 render 侧文档），命中
        // 计数只能走 `cached_overview`；所以这里只做一次不计数的预判，真正的
        // 取值与计数都交给下面那次 `cached_overview` 调用（命中时它不调闭包、
        // 不渲染，直接计一次命中返回克隆结果）。
        let hash = song_overview_hash(project, &settings, &options);
        let peek_hit = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&hash)
            .is_some();
        // 预检先行（见模块文档）：任一 SFZ 失败 ⇒ 响亮失败，缓存里不留脏结果。
        // 命中预判为真时跳过预检；`cached_overview` 命中时不调闭包，`armed`
        // 为空也不影响结果。
        let armed = if peek_hit {
            None
        } else {
            Some(self.arm_all(project, sample_rate_hz as f32)?)
        };
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (dsp_overview, _hit) =
            cached_overview(&mut cache, project, &settings, &options, |track, raw| {
                armed
                    .as_ref()
                    .and_then(|armed| armed.get(&track).cloned())
                    .unwrap_or_else(|| ArmedSfzInput::empty(raw))
            })
            .map_err(|error| {
                Fault::domain_with_data(
                    ErrorCode::RenderFailed,
                    format!("总览渲染失败: {error}"),
                    json!({
                        "reason": "renderFailed",
                        "cause": error.to_string(),
                    }),
                )
            })?;
        let beats = score_input_from_project(project).beats_per_bar();
        Ok(map_song_overview(&dsp_overview, request.resolution, beats))
    }
}

/// **纯谱面**总览：不渲染、不解码、不读盘 —— 只有谱面分析与它产生的哨兵。
///
/// ## 谁需要它
///
/// `--export-song-overview --overview-score-only`（批处理）要一份"只有谱面"的总览，
/// 且要它在 JSON 上与全量那一份**同构**（同一套字段、同一套取整）。GUI 面板的回退分支
/// （`yeban-app` 的 `song_overview::refresh_song_overview` 的 `Err(_)` 一支）算的也是
/// 同一件事 —— 但那一支住在 UI 侧（它要画图），批处理够不着，因此这里给它一个
/// **纯函数**出口。
///
/// ⛔ 它不是"渲染失败时偷偷用一下"的兜底：调用方必须**显式**要它
/// （见 `yeban-app` 的 `song_overview_export` 模块文档"失败语义"）。
///
/// # Errors
///
/// 当前实现不会失败（纯内存计算）；返回 `Result` 是为了与 provider 的口径一致，
/// 将来真的接上"谱面读不出来"时不必改签名。
pub fn score_only_overview(project: &YebanProjectV1) -> Result<McpSongOverview, Fault> {
    let score = score_input_from_project(project);
    let beats = score.beats_per_bar();
    let options = AnalysisOptions::default();
    let dsp = analyze_song_overview_with_options(&score, None, &options);
    Ok(map_song_overview(&dsp, Resolution::Bar, beats))
}

/// 采样器失败的结构化错误（`RENDER_FAILED` + `reason: "samplerUnreadable"`，
/// 与 MCP `prepare_sampler_track` 的同一枚 reason）。
fn sampler_fault(raw: &str, detail: String, resolved: Option<String>) -> Fault {
    let mut data = json!({
        "reason": "samplerUnreadable",
        "path": raw,
    });
    if let Some(resolved) = resolved
        && let Some(object) = data.as_object_mut()
    {
        object.insert("resolvedPath".to_owned(), Value::from(resolved));
    }
    Fault::domain_with_data(
        ErrorCode::RenderFailed,
        format!("总览渲染失败：采样器 `{raw}`：{detail}"),
        data,
    )
}

/// 一份 [`Fault`] 的人话信息（领域失败取 `message`；实现级状况取错误对象
/// 的 `message`）—— 只用于把装载层的失败**转述**进 [`sampler_fault`] 的
/// detail，⛔ 不改变失败的码与 reason（那由 `sampler_fault` 统一给）。
fn fault_message(fault: &Fault) -> String {
    match fault {
        Fault::Domain { message, .. } => message.clone(),
        Fault::Impl { error } => error.message.clone(),
    }
}

/// 提供表 → 引擎武装样本：读到的进表（`Arc` 共享同一份 PCM），读不到的
/// 跳过（缺样本不是错误，见模块文档：引擎标 `NO_SAMPLE` ＋ 计数）。
fn samples_from_table(table: SampledProviderTable) -> Vec<ArmedSfzSample> {
    table
        .into_iter()
        .filter_map(|(raw_path, outcome)| {
            outcome
                .ok()
                .map(|reading| (raw_path, (Arc::clone(&reading.pcm), reading.sample_hz)))
        })
        .collect()
}

/// dsp 列式总览 → MCP 行式 §8.2 structs。
///
/// 取整**不在**这里做：`domain::song_overview` 的 handler 在序列化前按 §8.2
/// 的口径取整（归一化 2 位、dB 1 位），stub 与真实 provider 共用那一条。
/// `beats_per_bar` 只用于拍级展开（`resolution: "beat"` 时每行继承所属小节的值，
/// `time_sec` 按 `60 / base_bpm` 步进；见模块文档）。
#[must_use]
pub fn map_song_overview(
    overview: &DspSongOverview,
    resolution: Resolution,
    beats_per_bar: u32,
) -> McpSongOverview {
    let total_bars = overview.song_meta.total_bars as usize;
    let series = &overview.bar_series;
    // dsp 保证各列长度恒等于总小节数；不一致是实现级错误，响亮 panic，
    // ⛔ 不静默截断到最短列。
    debug_assert_eq!(series.bar.len(), total_bars, "dsp bar 列长度");
    debug_assert_eq!(series.time_sec.len(), total_bars, "dsp time_sec 列长度");
    debug_assert_eq!(series.section.len(), total_bars, "dsp section 列长度");
    debug_assert_eq!(series.tension.len(), total_bars, "dsp tension 列长度");
    debug_assert_eq!(
        series.valence_hue.len(),
        total_bars,
        "dsp valence_hue 列长度"
    );
    debug_assert_eq!(series.lufs_short.len(), total_bars, "dsp lufs_short 列长度");
    debug_assert_eq!(
        series.lufs_momentary_max.len(),
        total_bars,
        "dsp lufs_momentary_max 列长度"
    );
    debug_assert_eq!(series.score_dyn.len(), total_bars, "dsp score_dyn 列长度");
    debug_assert_eq!(
        series.envelope_lo.len(),
        total_bars,
        "dsp envelope_lo 列长度"
    );
    debug_assert_eq!(
        series.envelope_hi.len(),
        total_bars,
        "dsp envelope_hi 列长度"
    );
    debug_assert_eq!(
        series.target_deviation.len(),
        total_bars,
        "dsp target_deviation 列长度"
    );
    debug_assert_eq!(
        series.density_pitched_onsets_per_beat.len(),
        total_bars,
        "dsp density_pitched 列长度"
    );
    debug_assert_eq!(
        series.density_pitched_duty.len(),
        total_bars,
        "dsp density_pitched_duty 列长度"
    );
    debug_assert_eq!(
        series.density_perc_onsets_per_beat.len(),
        total_bars,
        "dsp density_perc 列长度"
    );
    debug_assert_eq!(series.bands_tilted_db.len(), total_bars, "dsp bands 列长度");

    let mut bar_series: Vec<BarRow> = Vec::with_capacity(total_bars);
    for index in 0..total_bars {
        // 期望带（Phase 2）：dsp 的哨兵 −1（`NO_ENVELOPE`）= 无等级 ⇒
        // MCP 侧为 `None`（不序列化）；有等级时两端按 f32 原值转 f64。
        let envelope_lo = f64::from(series.envelope_lo[index]);
        let envelope_hi = f64::from(series.envelope_hi[index]);
        bar_series.push(BarRow {
            bar: series.bar[index] as u64,
            beat: None,
            time_sec: f64::from(series.time_sec[index]),
            section: series.section[index].clone(),
            tension: f64::from(series.tension[index]),
            valence_hue: f64::from(series.valence_hue[index]),
            lufs_short: f64::from(series.lufs_short[index]),
            lufs_momentary_max: f64::from(series.lufs_momentary_max[index]),
            score_dyn: f64::from(series.score_dyn[index]),
            envelope_lo: if envelope_lo == f64::from(NO_ENVELOPE) {
                None
            } else {
                Some(envelope_lo)
            },
            envelope_hi: if envelope_hi == f64::from(NO_ENVELOPE) {
                None
            } else {
                Some(envelope_hi)
            },
            target_deviation: f64::from(series.target_deviation[index]),
            density_pitched_onsets_per_beat: f64::from(
                series.density_pitched_onsets_per_beat[index],
            ),
            density_pitched_duty: f64::from(series.density_pitched_duty[index]),
            density_perc_onsets_per_beat: f64::from(series.density_perc_onsets_per_beat[index]),
            bands_tilted_db: series.bands_tilted_db[index].map(f64::from),
        });
    }
    if resolution == Resolution::Beat {
        bar_series = expand_to_beats(
            bar_series,
            beats_per_bar,
            f64::from(overview.song_meta.base_bpm),
        );
    }
    McpSongOverview {
        schema_version: overview.schema_version.to_owned(),
        song_meta: SongMeta {
            total_bars: overview.song_meta.total_bars as u64,
            duration_sec: f64::from(overview.song_meta.duration_sec),
            base_bpm: f64::from(overview.song_meta.base_bpm),
            key_signature: overview.song_meta.key_signature.clone(),
            profile: overview.song_meta.profile.clone(),
        },
        band_definitions_hz: BAND_DEFINITIONS_HZ,
        structural_summary: StructuralSummary {
            climax_bar: overview.structural_summary.climax_bar as u64,
            climax_tension: f64::from(overview.structural_summary.climax_tension),
            dynamic_range_lu: f64::from(overview.structural_summary.dynamic_range_lu),
            detected_anomalies: overview
                .structural_summary
                .detected_anomalies
                .iter()
                .map(|anomaly| DetectedAnomaly {
                    bars: [u64::from(anomaly.bars[0]), u64::from(anomaly.bars[1])],
                    anomaly_type: anomaly.anomaly_type.as_str().to_owned(),
                    description: anomaly.description.clone(),
                })
                .collect(),
        },
        // schema 1.1：段内自相似度摘要原样搬运（只做 f32 → f64；取整不在这里，
        // 见 `domain::song_overview::rounded`）。
        repetition: RepetitionSummary {
            best_matches: overview
                .repetition
                .best_matches
                .iter()
                .map(|best| BestMatch {
                    bar_a: u64::from(best.bar_a),
                    bar_b: u64::from(best.bar_b),
                    similarity: f64::from(best.similarity),
                })
                .collect(),
            mechanical_index: f64::from(overview.repetition.mechanical_index),
            repeated_passages: overview
                .repetition
                .repeated_passages
                .iter()
                .map(|passage| RepeatedPassage {
                    bars: [u64::from(passage.bars[0]), u64::from(passage.bars[1])],
                    source_bars: [
                        u64::from(passage.source_bars[0]),
                        u64::from(passage.source_bars[1]),
                    ],
                    mean_similarity: f64::from(passage.mean_similarity),
                })
                .collect(),
        },
        // schema 1.2：分轨频率打架摘要原样搬运（只做 f32 → f64；取整不在这里，
        // 见 `domain::song_overview::rounded`；严重度分档枚举 → 字符串）。
        frequency_clash: FrequencyClashSummary {
            total_groups: overview.frequency_clash.total_groups as u64,
            overall_overlap: f64::from(overview.frequency_clash.overall_overlap),
            bar_overlaps: overview
                .frequency_clash
                .bar_overlaps
                .iter()
                .map(|bar| BarOverlap {
                    bar: u64::from(bar.bar),
                    overlap: f64::from(bar.overlap),
                    max_overlap: f64::from(bar.max_overlap),
                })
                .collect(),
            entries: overview
                .frequency_clash
                .entries
                .iter()
                .map(|entry| FrequencyClashEntry {
                    bar: u64::from(entry.bar),
                    band: entry.band,
                    track_a: ClashTrackRef {
                        track_id: entry.track_a.track_id.clone(),
                        track_name: entry.track_a.track_name.clone(),
                    },
                    track_b: ClashTrackRef {
                        track_id: entry.track_b.track_id.clone(),
                        track_name: entry.track_b.track_name.clone(),
                    },
                    overlap: f64::from(entry.overlap),
                    mean_overlap: f64::from(entry.mean_overlap),
                    gap_db: f64::from(entry.gap_db),
                    severity: match entry.severity {
                        DspClashSeverity::Severe => "severe".to_owned(),
                        DspClashSeverity::Moderate => "moderate".to_owned(),
                        DspClashSeverity::Mild => "mild".to_owned(),
                    },
                    beats: u64::from(entry.beats),
                })
                .collect(),
        },
        bar_series,
    }
}

/// 小节行 → 拍行：每小节展开成 `beats_per_bar` 行，值继承所属小节，
/// `time_sec` 按每拍 `60 / base_bpm` 秒步进。
fn expand_to_beats(bar_series: Vec<BarRow>, beats_per_bar: u32, base_bpm: f64) -> Vec<BarRow> {
    let beats = beats_per_bar.max(1);
    let beat_sec = if base_bpm > 0.0 && base_bpm.is_finite() {
        60.0 / base_bpm
    } else {
        0.5
    };
    let mut beat_series: Vec<BarRow> = Vec::with_capacity(bar_series.len() * beats as usize);
    for row in bar_series {
        for beat in 1..=beats {
            let mut beat_row = row.clone();
            beat_row.beat = Some(u64::from(beat));
            beat_row.time_sec = row.time_sec + f64::from(beat - 1) * beat_sec;
            beat_series.push(beat_row);
        }
    }
    beat_series
}

// ===========================================================================
// 单元判据（与 `tests/song_overview.rs` 的 stub 判据同款：期望值走本文件内
// 的手算夹具，不读被测的映射代码）
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::SamplerBinding;
    use yeban_model::samples::demo_project;
    use yeban_render::song_overview::{
        Anomaly, AnomalyType, BAND_DEFINITIONS_HZ as DSP_BAND_DEFINITIONS_HZ,
        BarOverlap as DspBarOverlap, BarSeries, BestMatch as DspBestMatch,
        ClashSeverity as DspClashSeverity, ClashTrackRef as DspClashTrackRef,
        FrequencyClashEntry as DspFrequencyClashEntry,
        FrequencyClashSummary as DspFrequencyClashSummary, RepeatedPassage as DspRepeatedPassage,
        RepetitionSummary as DspRepetitionSummary, SongMeta as DspSongMeta,
        StructuralSummary as DspStructuralSummary,
    };

    /// 两小节的手算 dsp 夹具（列式；与 `map_song_overview` 的输入形状一致）。
    fn two_bar_dsp_overview() -> DspSongOverview {
        DspSongOverview {
            schema_version: "1.2",
            song_meta: DspSongMeta {
                total_bars: 2,
                duration_sec: 4.0,
                base_bpm: 120.0,
                key_signature: "D minor".to_owned(),
                profile: "default".to_owned(),
            },
            band_definitions_hz: DSP_BAND_DEFINITIONS_HZ,
            // Phase 2：自适应权重（本夹具无死特征 = 缺省档原权重）与包络列
            // （第 1 小节 Intro L3 带 [0.35, 0.65]、张力 0.42 带内偏离 0；
            // 第 2 小节 Verse 无等级 = 哨兵、偏离 0）。
            effective_weights: [0.30, 0.20, 0.20, 0.15, 0.15],
            structural_summary: DspStructuralSummary {
                climax_bar: 2,
                climax_tension: 0.79,
                dynamic_range_lu: 10.0,
                detected_anomalies: vec![Anomaly {
                    bars: [1, 2],
                    anomaly_type: AnomalyType::LowMidBuildup,
                    description: "Band 3 (250-500Hz) exceeds tilted balance by +5.2 dB".to_owned(),
                }],
            },
            // schema 1.1：两小节成不了段，重复摘要为空。
            repetition: DspRepetitionSummary::default(),
            // schema 1.2：两小节里贝斯与键盘在 Band 3 重叠 1 组（差距 0.5 dB、
            // 重叠度 0.89 = 10^(-0.5/10)、重档；映射只做 f32 → f64，不取整）。
            frequency_clash: DspFrequencyClashSummary {
                total_groups: 1,
                overall_overlap: 0.87,
                bar_overlaps: vec![DspBarOverlap {
                    bar: 1,
                    overlap: 0.87,
                    max_overlap: 0.89,
                }],
                entries: vec![DspFrequencyClashEntry {
                    bar: 1,
                    band: 2,
                    track_a: DspClashTrackRef {
                        track_id: "track-bass".to_owned(),
                        track_name: "Bass".to_owned(),
                    },
                    track_b: DspClashTrackRef {
                        track_id: "track-keys".to_owned(),
                        track_name: "Keys".to_owned(),
                    },
                    overlap: 0.89,
                    mean_overlap: 0.85,
                    gap_db: 0.5,
                    severity: DspClashSeverity::Severe,
                    beats: 2,
                }],
            },
            bar_series: BarSeries {
                bar: vec![1, 2],
                time_sec: vec![0.0, 2.0],
                section: vec!["Intro".to_owned(), "Verse".to_owned()],
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

    fn bar_request() -> SongOverviewRequest {
        SongOverviewRequest {
            resolution: Resolution::Bar,
            profile: McpProfile::Default,
            expected_levels: BTreeMap::new(),
        }
    }

    fn provider() -> RealSongOverviewProvider {
        RealSongOverviewProvider::new(ProviderContext::shared(None, Arc::new(BTreeMap::new())))
    }

    #[test]
    fn mapping_turns_columns_into_rows() {
        let mapped = map_song_overview(&two_bar_dsp_overview(), Resolution::Bar, 4);
        assert_eq!(mapped.schema_version, "1.2");
        assert_eq!(mapped.frequency_clash.total_groups, 1);
        assert_eq!(mapped.frequency_clash.overall_overlap, f64::from(0.87f32));
        assert_eq!(mapped.frequency_clash.bar_overlaps.len(), 1);
        assert_eq!(mapped.frequency_clash.bar_overlaps[0].bar, 1);
        assert_eq!(
            mapped.frequency_clash.bar_overlaps[0].overlap,
            f64::from(0.87f32)
        );
        assert_eq!(mapped.frequency_clash.entries.len(), 1);
        let clash = &mapped.frequency_clash.entries[0];
        assert_eq!(clash.bar, 1);
        assert_eq!(clash.band, 2);
        assert_eq!(clash.track_a.track_id, "track-bass");
        assert_eq!(clash.track_b.track_name, "Keys");
        assert_eq!(clash.overlap, f64::from(0.89f32));
        assert_eq!(clash.mean_overlap, f64::from(0.85f32));
        assert_eq!(clash.gap_db, 0.5);
        assert_eq!(clash.severity, "severe");
        assert_eq!(clash.beats, 2);
        assert_eq!(mapped.song_meta.total_bars, 2);
        assert_eq!(mapped.song_meta.duration_sec, 4.0);
        assert_eq!(mapped.song_meta.base_bpm, 120.0);
        assert_eq!(mapped.song_meta.key_signature, "D minor");
        assert_eq!(mapped.song_meta.profile, "default");
        assert_eq!(mapped.band_definitions_hz, BAND_DEFINITIONS_HZ);
        assert_eq!(mapped.structural_summary.climax_bar, 2);
        assert_eq!(mapped.structural_summary.climax_tension, 0.79f32 as f64);
        assert_eq!(mapped.structural_summary.dynamic_range_lu, 10.0);
        assert_eq!(mapped.structural_summary.detected_anomalies.len(), 1);
        assert_eq!(mapped.structural_summary.detected_anomalies[0].bars, [1, 2]);
        assert_eq!(
            mapped.structural_summary.detected_anomalies[0].anomaly_type,
            "low_mid_buildup"
        );
        assert_eq!(mapped.bar_series.len(), 2);
        let first = &mapped.bar_series[0];
        assert_eq!(first.bar, 1);
        assert_eq!(first.beat, None);
        assert_eq!(first.time_sec, 0.0);
        assert_eq!(first.section, "Intro");
        assert_eq!(first.tension, 0.42f32 as f64);
        assert_eq!(first.valence_hue, -0.62f32 as f64);
        assert_eq!(first.lufs_short, -22.4f32 as f64);
        assert_eq!(first.lufs_momentary_max, -19.8f32 as f64);
        assert_eq!(first.score_dyn, 0.28f32 as f64);
        assert_eq!(first.density_pitched_onsets_per_beat, 1.5);
        assert_eq!(first.density_pitched_duty, 0.65f32 as f64);
        assert_eq!(first.density_perc_onsets_per_beat, 0.5);
        assert_eq!(
            first.bands_tilted_db,
            [
                -18.2f32 as f64,
                -14.5f32 as f64,
                -12.0f32 as f64,
                -11.4f32 as f64,
                -15.0f32 as f64,
                -19.2f32 as f64,
                -24.0f32 as f64,
                -31.5f32 as f64,
            ]
        );
        let second = &mapped.bar_series[1];
        assert_eq!(second.bar, 2);
        assert_eq!(second.time_sec, 2.0);
        assert_eq!(second.section, "Verse");
        assert_eq!(second.tension, 0.79f32 as f64);
        // Phase 2：包络列映射（哨兵 −1 ⇒ None；有等级的行带值、偏离原值）。
        assert_eq!(first.envelope_lo, Some(0.35f32 as f64));
        assert_eq!(first.envelope_hi, Some(0.65f32 as f64));
        assert_eq!(first.target_deviation, 0.0);
        assert_eq!(second.envelope_lo, None);
        assert_eq!(second.envelope_hi, None);
        assert_eq!(second.target_deviation, 0.0);
    }

    #[test]
    fn mapping_copies_a_non_empty_repetition_summary() {
        // schema 1.1：非空重复摘要原样搬运（f32 → f64 逐位；取整不在映射层）。
        // 值取 8 小节歌的结构（[5, 8] 重复 [1, 4]），只测映射、不测检测。
        let mut dsp = two_bar_dsp_overview();
        dsp.repetition = DspRepetitionSummary {
            best_matches: vec![
                DspBestMatch {
                    bar_a: 1,
                    bar_b: 5,
                    similarity: 0.97,
                },
                DspBestMatch {
                    bar_a: 2,
                    bar_b: 6,
                    similarity: 0.5,
                },
            ],
            mechanical_index: 0.5,
            repeated_passages: vec![DspRepeatedPassage {
                bars: [5, 8],
                source_bars: [1, 4],
                mean_similarity: 0.97,
            }],
        };
        let mapped = map_song_overview(&dsp, Resolution::Bar, 4);
        assert_eq!(mapped.repetition.best_matches.len(), 2);
        assert_eq!(mapped.repetition.best_matches[0].bar_a, 1);
        assert_eq!(mapped.repetition.best_matches[0].bar_b, 5);
        assert_eq!(mapped.repetition.best_matches[0].similarity, 0.97f32 as f64);
        assert_eq!(mapped.repetition.mechanical_index, 0.5f32 as f64);
        assert_eq!(mapped.repetition.repeated_passages.len(), 1);
        assert_eq!(mapped.repetition.repeated_passages[0].bars, [5, 8]);
        assert_eq!(mapped.repetition.repeated_passages[0].source_bars, [1, 4]);
        assert_eq!(
            mapped.repetition.repeated_passages[0].mean_similarity,
            0.97f32 as f64
        );
    }

    #[test]
    fn beat_resolution_expands_bars_and_steps_by_beat_seconds() {
        let mapped = map_song_overview(&two_bar_dsp_overview(), Resolution::Beat, 4);
        assert_eq!(mapped.bar_series.len(), 8, "2 小节 × 4 拍 = 8 行");
        for (index, row) in mapped.bar_series.iter().enumerate() {
            let bar = (index / 4) as u64 + 1;
            let beat = (index % 4) as u64 + 1;
            assert_eq!(row.bar, bar);
            assert_eq!(row.beat, Some(beat));
            assert_eq!(row.time_sec, index as f64 * 0.5, "120 BPM ⇒ 0.5 秒一拍");
            // 拍行继承所属小节的值（Phase 1 口径，见模块文档）。
            // 包络列同样继承（Phase 2：拍级不重算期望带，沿用小节值）。
            assert_eq!(
                row.envelope_lo,
                if bar == 1 { Some(0.35f32 as f64) } else { None }
            );
            assert_eq!(
                row.envelope_hi,
                if bar == 1 { Some(0.65f32 as f64) } else { None }
            );
            assert_eq!(row.target_deviation, 0.0);
            assert_eq!(
                row.tension,
                if bar == 1 {
                    0.42f32 as f64
                } else {
                    0.79f32 as f64
                }
            );
            assert_eq!(
                row.lufs_short,
                if bar == 1 {
                    -22.4f32 as f64
                } else {
                    -12.4f32 as f64
                }
            );
        }
        // 结构摘要仍是小节级。
        assert_eq!(mapped.structural_summary.climax_bar, 2);
    }

    #[test]
    fn mapping_an_empty_overview_stays_empty() {
        let empty = DspSongOverview {
            schema_version: "1.2",
            song_meta: DspSongMeta {
                total_bars: 0,
                duration_sec: 0.0,
                base_bpm: 120.0,
                key_signature: "C major".to_owned(),
                profile: "default".to_owned(),
            },
            band_definitions_hz: DSP_BAND_DEFINITIONS_HZ,
            effective_weights: [0.30, 0.20, 0.20, 0.15, 0.15],
            structural_summary: DspStructuralSummary {
                climax_bar: 0,
                climax_tension: 0.0,
                dynamic_range_lu: 0.0,
                detected_anomalies: Vec::new(),
            },
            repetition: DspRepetitionSummary::default(),
            frequency_clash: DspFrequencyClashSummary::default(),
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
        };
        let mapped = map_song_overview(&empty, Resolution::Bar, 4);
        assert_eq!(mapped.song_meta.total_bars, 0);
        assert!(mapped.bar_series.is_empty());
        assert_eq!(mapped.structural_summary.climax_bar, 0);
        assert_eq!(mapped.frequency_clash.total_groups, 0);
        assert_eq!(mapped.frequency_clash.overall_overlap, 0.0);
        assert!(mapped.frequency_clash.bar_overlaps.is_empty());
        assert!(mapped.frequency_clash.entries.is_empty());
    }

    #[test]
    fn demo_project_overview_matches_the_pipeline_fixture() {
        let project = demo_project();
        let real = provider();
        let first = real
            .overview(&project, &bar_request())
            .expect("演示工程必须渲染通过");
        // B 线夹具的已知答案（`yeban-render/tests/song_overview_pipeline.rs`）：
        // 谱面 4 拍、音频 10 拍 ⇒ 3 小节；5 秒 @ 120 BPM。
        assert_eq!(first.song_meta.total_bars, 3);
        assert!((first.song_meta.duration_sec - 5.0).abs() < 1e-6);
        assert_eq!(first.song_meta.base_bpm, 120.0);
        assert_eq!(first.bar_series.len(), 3);
        // 演示工程只有一条轨出声（鼓），无重叠。
        assert_eq!(first.frequency_clash.total_groups, 0);
        assert_eq!(first.frequency_clash.overall_overlap, 0.0);
        assert_eq!(first.bar_series[0].section, "Intro");
        for row in &first.bar_series {
            assert!((0.0..=1.0).contains(&row.tension), "张力越界");
            assert!(row.bands_tilted_db.iter().all(|db| db.is_finite()));
        }
        assert!(first.bar_series[0].score_dyn > 0.0, "第一小节必须有起音");
        // 第二次同一调用：缓存命中，结果逐位相同，且不再算一次。
        assert_eq!((real.cache_hits(), real.cache_misses()), (0, 1));
        let second = real.overview(&project, &bar_request()).expect("第二次调用");
        assert_eq!(first, second, "缓存结果必须与现算逐位相同");
        assert_eq!((real.cache_hits(), real.cache_misses()), (1, 1));
    }

    #[test]
    fn missing_sfz_is_a_loud_render_failure_not_silence() {
        let mut project = demo_project();
        let first_track = *project.tracks.keys().next().expect("演示工程有轨");
        project.tracks.get_mut(&first_track).expect("轨在").sampler = Some(SamplerBinding {
            path: "assets/samples/does-not-exist/missing.sfz".to_owned(),
        });
        let real = provider();
        let error = real
            .overview(&project, &bar_request())
            .expect_err("缺失的 SFZ 必须响亮失败，⛔ 不静默出静音总览");
        match &error {
            Fault::Domain {
                code,
                message,
                data,
            } => {
                assert_eq!(*code, ErrorCode::RenderFailed);
                assert!(
                    message.contains("missing.sfz"),
                    "错误消息必须点名缺失的绑定路径: {message}"
                );
                let data = data.as_ref().expect("结构化补充");
                assert_eq!(data["reason"], "samplerUnreadable");
                assert_eq!(data["path"], "assets/samples/does-not-exist/missing.sfz");
            }
            other => panic!("失败必须是领域错误，实际是 {other:?}"),
        }
        // 失败的调用不留缓存：命中/未命中计数都不动；绑定修好后同一 provider
        // 必须能算（失败没有毒化缓存，也没有把"缺 SFZ"那次的结果存进去）。
        assert_eq!((real.cache_hits(), real.cache_misses()), (0, 0));
        project.tracks.get_mut(&first_track).expect("轨在").sampler = None;
        let recovered = real
            .overview(&project, &bar_request())
            .expect("绑定修好后必须能算");
        assert_eq!(recovered.song_meta.total_bars, 3);
        assert_eq!((real.cache_hits(), real.cache_misses()), (0, 1));
    }

    #[test]
    fn provider_name_is_real_not_stub() {
        assert_eq!(provider().name(), "real");
        assert_eq!(PROVIDER_NAME, "real");
    }

    #[test]
    fn context_sync_follows_the_active_project_and_asset_keys() {
        // Domain 托管形态的同步口径：路径/键集没变 ⇒ 不换池（同一 Arc）；
        // 路径变 ⇒ 工程目录跟随；键集变 ⇒ 换池。
        let mut context = ProviderContext {
            project_dir: None,
            assets: Arc::new(BTreeMap::new()),
            synced_key: None,
        };
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        context.sync_from(Some(Path::new("/music/song/song.yeban")), Some(&pool));
        assert_eq!(context.project_dir(), Some(Path::new("/music/song")));
        let first_pool = context.assets();
        context.sync_from(Some(Path::new("/music/song/song.yeban")), Some(&pool));
        assert!(
            Arc::ptr_eq(&first_pool, &context.assets()),
            "同步键没变时不得换池"
        );
        context.sync_from(Some(Path::new("/music/other/other.yeban")), Some(&pool));
        assert_eq!(context.project_dir(), Some(Path::new("/music/other")));
        context.sync_from(None, None);
        assert_eq!(context.project_dir(), None);
        assert!(context.assets().is_empty());
    }
}
