//! 整曲多层可视化总览图（设计稿 `yeban-songmap-design.md` Phase 1）：
//!
//! 工程 → 离线总线音频（内存缓冲）→ dsp 四层分析（[`SongOverview`]）的
//! Phase 1 管线 + 整曲哈希缓存。
//!
//! ## 三条入口
//!
//! - [`render_project_to_buffer`]：给定工程，离线渲染整曲总线音频到内存交错
//!   立体声 `f32` 缓冲（[`AudioInput`] 可直接用的形状）+ 采样率。
//! - [`render_stems_to_buffers`]：逐源轨 solo 渲染（设计稿 §8.2 追加，schema 1.2），
//!   分轨频率打架检测的音频来源；复用分轨导出的 clone-mute-re-render 招数。
//! - [`cached_overview`]：同一哈希直接返回缓存结果（不重算）；工程或渲染设置
//!   任一变化即失效（见 [`song_overview_hash`]）。
//!
//! ## 复用，不另写
//!
//! 渲染复用现有管线，不写第二份合成器：快照投影 + SFZ 武装走
//! [`EngineTimbre`]，调度走 [`RenderPlan`]，插入链走 [`project_inserts`]，
//! PDC 延迟走 [`track_latencies`]，声相走 [`centered_pan_gains`] +
//! `yeban_engine::mixer::pan_gains`（与 CLI 离线导出同一条曲线）。
//! 谱面映射走 `yeban_model::song_overview::score_input_from_project`，
//! 分析走 [`yeban_dsp::song_overview::analyze_song_overview_with_stems`]
//! （分轨版；无分轨时退化为旧行为）。
//!
//! ## 帧数口径（与 `yeban_render_master` 同一条公式）
//!
//! 先以 `frames = 1` 编译一份探针计划取保留子图（剪枝后的节点集），再取
//! 保留节点上全部非静音摆放的末端（摆放末端 + 微时值正偏移的音符尾），
//! 经工程速度图换算成帧。未触发的音符一位都不贡献；音频片段摆放的末端
//! 同样计入（即使该轨的音频本身 Phase 1 不渲染 —— 见下）。
//!
//! ## Phase 1 明确不做的事（不是静默缺失，都记在收据里）
//!
//! - **自动化泳道**（`TrackVolume` / `TrackPan` / `SendGain` /
//!   `DeviceParam` / `Macro`）：不挂接。静态推子与声相走引擎快照与源侧，
//!   插入链走静态投影。带非恒等泳道的工程渲染结果与 `yeban_render_master`
//!   不一致 —— 泳道接线是 Phase 2 的工作。
//! - **音频片段轨**：本 crate 没有音频解码器（`yeban-decode` 刻意不下移，
//!   见 `#270` 的禁区），因此音频片段不出声；相关轨进收据的
//!   [`SkippedTrack`]（[`SkippedReason::AudioClipsOnly`]），源位填静音。
//! - **外置乐器轨**（`ExternalInstrument` 设备）：没有插件宿主实例可接，
//!   同样进收据（[`SkippedReason::PluginInstrument`]），源位填静音。
//! - **外置效果器**：不实例化，相关轨按干声渲染（收据不另记 ——
//!   `project_inserts` 的静态投影同样不含外置效果器实例）。
//! - **混响 / 释放尾巴**：帧数只到谱面末端，不加尾巴（与 `yeban_render_master`
//!   的缺省口径一致）。
//!
//! ## 缓存
//!
//! 整曲哈希 = 工程 JSON（确定性序列化）+ 渲染设置（采样率、种子）+
//! 分析选项（曲风档、期望等级表，Phase 2）的 SHA-256。
//! 哈希只认**内容**：工程里任何一位变化（音符、速度、调号、采样器绑定、
//! 设备参数、元数据时间戳）或渲染设置 / 分析选项变化 ⇒ 哈希变 ⇒ 旧结果失效。
//! 缓存是进程内内存（[`SongOverviewCache`]），不写盘、不碰持久化 AST。
//! 采样率必须与工程 `audio_config` 一致（引擎快照按工程采样率投影），
//! 否则 [`SongOverviewError::SampleRateMismatch`] 响亮失败。

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use yeban_dsp::song_overview::{
    AnalysisOptions, AudioInput, SongOverview, StemInput, analyze_song_overview_with_stems,
};
use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_model::{
    ClipContent, DeviceKind, EntityId, PPQ, RoutingKind, YebanProjectV1,
    song_overview::score_input_from_project,
};

use crate::automation::centered_pan_gains;
use crate::engine_source::{ArmedSfzInput, EngineSourceError, EngineTimbre};
use crate::inserts::project_inserts;
use crate::render::{
    AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan, track_latencies,
};

/// 渲染设置（整曲哈希的一半；另一半是工程内容）。
///
/// 采样率必须等于工程 `audio_config.sample_rate`（见模块文档）；线程数不进
/// 哈希 —— 多线程与单线程输出逐位相同（L1 bit-exact 判据守着）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSettings {
    /// 采样率（Hz）。
    pub sample_rate_hz: u32,
    /// 确定性随机种子（透传给渲染器与样本源）。
    pub seed: u64,
}

impl RenderSettings {
    /// 构造一份渲染设置。
    #[must_use]
    pub const fn new(sample_rate_hz: u32, seed: u64) -> Self {
        Self {
            sample_rate_hz,
            seed,
        }
    }
}

impl Default for RenderSettings {
    /// 48 kHz、种子 0。
    fn default() -> Self {
        Self {
            sample_rate_hz: 48_000,
            seed: 0,
        }
    }
}

/// 本管线的失败原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SongOverviewError {
    /// 引擎快照投影 / SFZ 武装失败。
    Engine(EngineSourceError),
    /// 渲染计划编译 / 执行失败。
    Render(RenderError),
    /// 工程里没有可渲染的内容（帧数为 0）。
    EmptyProject,
    /// 请求的采样率与工程 `audio_config` 不一致。
    SampleRateMismatch {
        /// 工程的采样率（Hz）。
        expected_hz: u32,
        /// 请求的采样率（Hz）。
        got_hz: u32,
    },
}

impl std::fmt::Display for SongOverviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Engine(error) => write!(f, "引擎快照投影/武装失败: {error}"),
            Self::Render(error) => write!(f, "渲染失败: {error}"),
            Self::EmptyProject => f.write_str("工程里没有可渲染的内容 (0 帧)"),
            Self::SampleRateMismatch {
                expected_hz,
                got_hz,
            } => write!(
                f,
                "采样率与工程不一致: 工程是 {expected_hz} Hz, 请求是 {got_hz} Hz"
            ),
        }
    }
}

impl std::error::Error for SongOverviewError {}

impl From<EngineSourceError> for SongOverviewError {
    fn from(error: EngineSourceError) -> Self {
        Self::Engine(error)
    }
}

impl From<RenderError> for SongOverviewError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

/// 一条被跳过的轨（出了静音源、没出声音，原因见 [`SkippedReason`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedTrack {
    /// 轨道身份。
    pub track: EntityId,
    /// 跳过原因。
    pub reason: SkippedReason,
}

/// 跳过一轨的原因（Phase 1 明确不做的两类，见模块文档）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkippedReason {
    /// 只有音频片段、没有可排程的 MIDI 音符：本 crate 没有音频解码器。
    AudioClipsOnly,
    /// 挂了外置乐器设备：本入口不接插件宿主实例。
    PluginInstrument,
}

/// 一次整曲离线渲染的产物：内存交错立体声 `f32` 缓冲 + 收据。
#[derive(Clone, Debug)]
pub struct RenderedAudio {
    /// 交错立体声采样（L, R, L, R, …）。
    pub samples: Vec<f32>,
    /// 采样率（Hz；恒等于工程 `audio_config.sample_rate`）。
    pub sample_rate: u32,
    /// 总帧数（= `samples.len() / 2`）。
    pub frames: u64,
    /// 被跳过的轨（出了静音源，原因见 [`SkippedReason`]）。
    pub skipped_tracks: Vec<SkippedTrack>,
}

impl RenderedAudio {
    /// 峰值（最大绝对值；空缓冲为 0）。
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.samples
            .iter()
            .fold(0.0f32, |peak, &sample| peak.max(sample.abs()))
    }

    /// 以 [`AudioInput`] 的形状借出本缓冲（可直接喂
    /// [`analyze_song_overview`]）。
    #[must_use]
    pub fn as_audio_input(&self) -> AudioInput<'_> {
        AudioInput {
            samples: &self.samples,
            sample_rate: self.sample_rate as f32,
        }
    }
}

/// 静音源：给"没有可排程音符的叶节点"占位（`RenderPlan::execute` 要求每个
/// 无入边节点都有源；静音是如实上报的 [`SkippedTrack`] 的另一半）。
struct Silence;

impl AudioSource for Silence {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        out.fill(0.0);
        let _ = context;
        Ok(())
    }
}

/// 工程 + 渲染设置 + 分析选项的整曲哈希（SHA-256）。
///
/// 工程侧是 `serde_json` 确定性序列化（`BTreeMap` 键升序、结构体字段定序）；
/// 设置侧是采样率与种子的定长小端字节；选项侧是曲风档名与期望等级表
/// （`BTreeMap` 升序：段落名 + 等级字节）。任一一位变化 ⇒ 哈希变 ⇒ 缓存失效。
/// Phase 2 加选项侧：同一工程换档 / 改期望等级必须是不命中的另一条缓存项，
/// ⛔ 不许返回 Default 档的旧结果。
#[must_use]
pub fn song_overview_hash(
    project: &YebanProjectV1,
    settings: &RenderSettings,
    options: &AnalysisOptions,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    // `YebanProjectV1` 的 `Serialize` 是确定性的（见上）；序列化失败只可能
    // 是内存分配失败 ⇒ 退回 `Debug` 文本（同样确定），⛔ 不 panic。
    match serde_json::to_vec(project) {
        Ok(bytes) => hasher.update(bytes),
        Err(_) => hasher.update(format!("{project:?}").into_bytes()),
    }
    hasher.update(settings.sample_rate_hz.to_le_bytes());
    hasher.update(settings.seed.to_le_bytes());
    hasher.update(options.profile.as_str().as_bytes());
    for (section, level) in &options.expected_levels {
        hasher.update(section.as_bytes());
        hasher.update([*level]);
    }
    let output = hasher.finalize();
    let mut digest = [0u8; 32];
    for (slot, byte) in digest.iter_mut().zip(output.iter()) {
        *slot = *byte;
    }
    digest
}

/// 整曲哈希缓存（Phase 1）：整曲哈希 → [`SongOverview`]。
///
/// 进程内内存，不写盘、不碰持久化 AST。命中计数（[`Self::hits`] /
/// [`Self::misses`]）是"第二次调用不重算"的可测证据。
#[derive(Clone, Debug, Default)]
pub struct SongOverviewCache {
    entries: BTreeMap<[u8; 32], SongOverview>,
    hits: u64,
    misses: u64,
}

impl SongOverviewCache {
    /// 空缓存。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 按哈希查（不计命中数；计数只走 [`Self::get_or_compute`]）。
    #[must_use]
    pub fn get(&self, hash: &[u8; 32]) -> Option<&SongOverview> {
        self.entries.get(hash)
    }

    /// 存入一条结果（同哈希覆盖旧值）。
    pub fn insert(&mut self, hash: [u8; 32], overview: SongOverview) {
        self.entries.insert(hash, overview);
    }

    /// 查缓存，命中返回克隆结果（`hit = true`），未命中跑 `compute` 后存入
    /// 再返回（`hit = false`）。命中/未命中计数各加一。
    pub fn get_or_compute(
        &mut self,
        hash: [u8; 32],
        compute: impl FnOnce() -> SongOverview,
    ) -> (SongOverview, bool) {
        if let Some(cached) = self.entries.get(&hash) {
            self.hits = self.hits.saturating_add(1);
            return (cached.clone(), true);
        }
        self.misses = self.misses.saturating_add(1);
        let overview = compute();
        self.entries.insert(hash, overview.clone());
        (overview, false)
    }

    /// 缓存命中次数。
    #[must_use]
    pub const fn hits(&self) -> u64 {
        self.hits
    }

    /// 缓存未命中次数。
    #[must_use]
    pub const fn misses(&self) -> u64 {
        self.misses
    }

    /// 缓存条目数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否空缓存。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 端到端 Phase 1 管线（带缓存）：谱面映射 → 渲染 → 分析。
///
/// 命中缓存时不渲染、不分析，直接返回克隆结果（`hit = true`）。
/// 未命中时全量跑一遍后把结果存入缓存（`hit = false`）。
/// `options` 是 Phase 2 分析选项（曲风档 / 期望等级表）：进哈希（见
/// [`song_overview_hash`]），也进分析（[`analyze_song_overview_with_stems`]）。
///
/// # Errors
///
/// [`SongOverviewError`]：采样率不一致 / 工程无内容 / 快照武装失败 /
/// 渲染计划编译或执行失败。
pub fn cached_overview<F>(
    cache: &mut SongOverviewCache,
    project: &YebanProjectV1,
    settings: &RenderSettings,
    options: &AnalysisOptions,
    mut sfz_provider: F,
) -> Result<(SongOverview, bool), SongOverviewError>
where
    F: FnMut(EntityId, &str) -> ArmedSfzInput,
{
    let hash = song_overview_hash(project, settings, options);
    if let Some(cached) = cache.get(&hash).cloned() {
        cache.hits = cache.hits.saturating_add(1);
        return Ok((cached, true));
    }
    cache.misses = cache.misses.saturating_add(1);
    let rendered = render_project_to_buffer(project, settings, &mut sfz_provider)?;
    let stems = render_stems_to_buffers(project, settings, &mut sfz_provider)?;
    let score = score_input_from_project(project);
    let audio = rendered.as_audio_input();
    let stem_ids: Vec<String> = stems
        .iter()
        .map(|stem| stem.track.to_canonical_string())
        .collect();
    let stem_inputs: Vec<StemInput<'_>> = stems
        .iter()
        .zip(&stem_ids)
        .map(|(stem, id)| StemInput {
            track_id: id,
            track_name: &stem.name,
            audio: stem.audio.as_audio_input(),
        })
        .collect();
    let overview = analyze_song_overview_with_stems(&score, Some(&audio), &stem_inputs, options);
    cache.insert(hash, overview.clone());
    Ok((overview, false))
}

/// 给定工程，离线渲染整曲总线音频到内存交错立体声 `f32` 缓冲。
///
/// 复用现有渲染管线（见模块文档）：快照 + SFZ 武装 + `RenderPlan` 执行。
/// SFZ 采样解析走调用方提供的 `sfz_provider`（与
/// [`EngineTimbre::from_project_armed`] 同一形状）：provider 负责
/// embedded_samples 优先、磁盘回退的解析口径（MCP / CLI 各自的既有实现），
/// 本函数只按它返回的 [`ArmedSfzInput`] 武装，不发明第三份解析规则。
/// 没有采样器绑定的工程不会调用 provider。
///
/// # Errors
///
/// [`SongOverviewError`]：采样率不一致 / 工程无内容 / 快照武装失败 /
/// 渲染计划编译或执行失败。
pub fn render_project_to_buffer<F>(
    project: &YebanProjectV1,
    settings: &RenderSettings,
    sfz_provider: F,
) -> Result<RenderedAudio, SongOverviewError>
where
    F: FnMut(EntityId, &str) -> ArmedSfzInput,
{
    let sample_rate = project.audio_config.sample_rate.hz();
    if settings.sample_rate_hz != sample_rate {
        return Err(SongOverviewError::SampleRateMismatch {
            expected_hz: sample_rate,
            got_hz: settings.sample_rate_hz,
        });
    }
    // 1. 探针计划（frames = 1）取保留子图 —— 与 MCP 的探针同一招数：
    //    帧数推导只看能到达 Master 的节点，不看悬挂轨。
    let probe_options = RenderOptions::l1(1, 2, sample_rate, settings.seed);
    let probe = RenderPlan::compile(
        &project.routing_graph,
        project.master_bus_track_id,
        probe_options,
    )?;
    let kept: BTreeSet<EntityId> = probe.nodes().into_iter().collect();
    let frames = frames_for_project(project, &kept, sample_rate);
    if frames == 0 {
        return Err(SongOverviewError::EmptyProject);
    }

    // 2. 快照 + SFZ 武装（一次投影，供全部轨共用）。
    let timbre = EngineTimbre::from_project_armed(project, sample_rate as f32, sfz_provider)?;

    // 3. 正式计划：延迟 + 插入链。
    let latencies = track_latencies(&project.tracks);
    let inserts = project_inserts(&project.tracks, sample_rate, project.bpm);
    let options = RenderOptions::l1(frames, 2, sample_rate, settings.seed);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &project.routing_graph,
        project.master_bus_track_id,
        options,
        &latencies,
        &inserts,
    )?;

    // 4. 逐轨装源：有排程音符的轨走引擎源（声相 + CC 探针）；外置乐器轨与
    //    纯音频轨进收据、源位填静音；无音符的叶节点填静音；无音符的总线
    //    节点不需要源（`execute` 只要求无入边节点有源）。
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let mut skipped_tracks = Vec::new();
    for node in plan.nodes() {
        let Some(track) = project.tracks.get(&node) else {
            continue;
        };
        let has_incoming = has_audio_incoming(project, &kept, node);
        if is_plugin_track(track) {
            skipped_tracks.push(SkippedTrack {
                track: node,
                reason: SkippedReason::PluginInstrument,
            });
            if !has_incoming {
                sources.insert(node, Box::new(Silence));
            }
            continue;
        }
        if timbre.scheduled_notes(node) > 0 {
            let pan = centered_pan_gains(track.pan, |pan| {
                pan_gains(pan, PanLaw::from_model(project.audio_config.pan_law))
            });
            let source = timbre.track_source(node)?.with_pan(pan).with_cc(project);
            sources.insert(node, Box::new(source));
            continue;
        }
        if track_has_audio_clips(project, track) {
            skipped_tracks.push(SkippedTrack {
                track: node,
                reason: SkippedReason::AudioClipsOnly,
            });
        }
        if !has_incoming {
            sources.insert(node, Box::new(Silence));
        }
    }

    // 5. 执行：产物即整曲总线音频。
    let output = plan.execute(sources)?;
    Ok(RenderedAudio {
        samples: output.samples,
        sample_rate,
        frames: output.frames,
        skipped_tracks,
    })
}

/// 一条源轨 solo 渲染出的音频（分轨频率打架检测的输入，见 [`render_stems_to_buffers`]）。
#[derive(Clone, Debug)]
pub struct StemBuffer {
    /// 轨 id。
    pub track: EntityId,
    /// 轨名。
    pub name: String,
    /// 该轨 solo 渲染的音频（与混音同采样率、同帧数）。
    pub audio: RenderedAudio,
}

/// 逐源轨 solo 渲染（设计稿 §8.2 追加，schema 1.2；分轨频率打架检测的音频来源）。
///
/// 做法与 MCP 的分轨导出（`yeban_mcp::domain::stems::export`）同一招数，
/// 不另建渲染器：每条源轨各 clone 一份工程 → 试听复位（mute/solo/solo_safe
/// 全清，与 `stems.rs` 的 `project_without_audition` 同语义）→ 清主总线
/// devices（与分轨一致：主总线处理是共用的，分轨各涂一遍会扭曲各轨频段
/// 相对关系）→ 除目标轨外的其余**源节点** mute → [`render_project_to_buffer`]。
/// mute 经引擎快照生效（静音轨渲染为静音）。总线与 master **不动** —— 它们
/// 是目标轨信号的通路，静音它们会连目标一起消掉（`stems.rs` 同款注释；
/// 本函数初版误 mute 全轨，演示工程 4 条 MIDI 分轨只剩鼓轨出声，遂改）。
///
/// 源节点 = 探针计划里无入边（`bus_reduction_order` 为 None 或空）的节点，
/// 主总线除外；与 MCP 的 `probe_subgraph` / `is_source_node` 同规则（那侧
/// 是 `pub(super)`，本侧自己实现）。有入边 + 自带 clips 的总线源轨跳过，
/// 是 v1 的已知局限（与分轨后端的 `NotASourceNode` 同规则）。
///
/// 成本：N 次渲染（N = 源轨数），与 `--export-stems` 同量级；调用方
/// （[`cached_overview`]）只在缓存未命中时跑。
///
/// # Errors
///
/// [`SongOverviewError`]：采样率不一致 / 工程无内容 / 快照武装失败 /
/// 渲染计划编译或执行失败。
pub fn render_stems_to_buffers<F>(
    project: &YebanProjectV1,
    settings: &RenderSettings,
    mut sfz_provider: F,
) -> Result<Vec<StemBuffer>, SongOverviewError>
where
    F: FnMut(EntityId, &str) -> ArmedSfzInput,
{
    let sample_rate = project.audio_config.sample_rate.hz();
    if settings.sample_rate_hz != sample_rate {
        return Err(SongOverviewError::SampleRateMismatch {
            expected_hz: sample_rate,
            got_hz: settings.sample_rate_hz,
        });
    }
    // 探针计划取保留子图 + 源节点（与 `render_project_to_buffer` 同一招数）。
    let probe_options = RenderOptions::l1(1, 2, sample_rate, settings.seed);
    let probe = RenderPlan::compile(
        &project.routing_graph,
        project.master_bus_track_id,
        probe_options,
    )?;
    let sources: Vec<EntityId> = probe
        .nodes()
        .into_iter()
        .filter(|node| {
            *node != project.master_bus_track_id
                && probe
                    .bus_reduction_order(*node)
                    .is_none_or(|inputs| inputs.is_empty())
        })
        .collect();
    let mut stems = Vec::with_capacity(sources.len());
    for &target in &sources {
        let mut clone = project.clone();
        for track in clone.tracks.values_mut() {
            track.mute = false;
            track.solo = false;
            track.solo_safe = false;
        }
        if let Some(master) = clone.tracks.get_mut(&clone.master_bus_track_id) {
            master.devices.clear();
        }
        // 只 mute 其余源节点：总线与 master 是目标信号的通路，不动。
        for &node in &sources {
            if node != target {
                if let Some(track) = clone.tracks.get_mut(&node) {
                    track.mute = true;
                }
            }
        }
        let audio = render_project_to_buffer(&clone, settings, &mut sfz_provider)?;
        let name = project
            .tracks
            .get(&target)
            .map(|track| track.name.clone())
            .unwrap_or_default();
        stems.push(StemBuffer {
            track: target,
            name,
            audio,
        });
    }
    Ok(stems)
}

/// 该节点是否有一条来自保留子图的音频入边（侧链边不算音频边）。
fn has_audio_incoming(project: &YebanProjectV1, kept: &BTreeSet<EntityId>, node: EntityId) -> bool {
    project.routing_graph.edges.values().any(|edge| {
        edge.kind != RoutingKind::Sidechain
            && edge.destination_node == node
            && kept.contains(&edge.source_node)
    })
}

/// 该轨是否挂了外置乐器设备（未旁通的 `ExternalInstrument`）。
fn is_plugin_track(track: &yeban_model::TrackV3) -> bool {
    track
        .devices
        .iter()
        .any(|device| !device.bypassed && device.kind == DeviceKind::ExternalInstrument)
}

/// 该轨是否有非静音的音频片段摆放。
fn track_has_audio_clips(project: &YebanProjectV1, track: &yeban_model::TrackV3) -> bool {
    track.clips.values().any(|placement| {
        !placement.muted
            && project
                .clip_pool
                .get(&placement.clip_id)
                .is_some_and(|entry| matches!(entry.content, ClipContent::Audio { .. }))
    })
}

/// 帧数推导（与 `yeban_render_master` 同一条公式）：保留节点上全部非静音
/// 摆放的末端取最大 —— 摆放末端打底，未触发的音符不贡献，循环展开的音符
/// 尾巴按 `note_copies` + 微时值正偏移计；再经工程速度图换算成帧。
/// 换算失败（非法速度/采样率）⇒ 0 帧（调用方报 [`SongOverviewError::EmptyProject`]）。
fn frames_for_project(
    project: &YebanProjectV1,
    kept: &BTreeSet<EntityId>,
    sample_rate: u32,
) -> u64 {
    let mut end_tick = 0u64;
    for track in project.tracks.values() {
        if !kept.contains(&track.id) {
            continue;
        }
        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            end_tick = end_tick.max(
                placement
                    .start_tick
                    .saturating_add(placement.duration_ticks),
            );
            let ClipContent::Midi { notes } = &entry.content else {
                continue;
            };
            for note in notes.values() {
                if !note.triggers(project.rng_seed) {
                    continue;
                }
                let micro = u64::try_from(note.micro_timing_ticks.unwrap_or(0).max(0)).unwrap_or(0);
                for (copy_start, copy_duration) in
                    placement.note_copies(note.start_tick, note.duration_ticks)
                {
                    end_tick = end_tick.max(
                        placement
                            .start_tick
                            .saturating_add(copy_start)
                            .saturating_add(micro)
                            .saturating_add(copy_duration),
                    );
                }
            }
        }
    }
    project
        .tempo()
        .frames_at_tick(end_tick, PPQ, sample_rate)
        .unwrap_or(0)
}
