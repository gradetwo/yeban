//! **引擎音色源**：把 `yeban-engine` 的 `SynthEngine` 接成 [`AudioSource`]
//! （`ADR-0011`；本文件是 `yeban-render` 里**唯一**碰 `yeban-engine` 的模块）。
//!
//! ## 为什么这一层存在（`ADR-0011` D2）
//!
//! 同一份工程原来有两条 MIDI 音源：CLI / 界面走 `SynthEngine`（加法波表 `HOLLOW`
//! ＋ ADSR ＋ 双振荡器），MCP 走自己的正弦振荡器。负责人 2026-10-21 逐字裁决
//! 「统一」⇒ 本模块把 CLI 那条 [`EngineTrackSource`] **原样搬进** `yeban-render`，
//! MCP 经**已登记的** `(yeban-mcp, yeban-render)` 例外使用它
//! ⇒ ⛔ 没有新增任何依赖例外，也 ⛔ 没有第二份音源。
//!
//! ## `ADR-0011` 的三条未决（本单的裁决，逐条给理由）
//!
//! ### ① 落哪一层：**新模块**，不是 `render.rs` 内
//!
//! - `render.rs` 是调度 / 归约 / 位级摘要的**唯一实现**，它的判据（`render::tests`）
//!   不应当为了"离线导出要引擎音色"而把 `yeban-engine` 的构造拖进每个测试二进制；
//! - 这条依赖边（`yeban-render -> yeban-engine`，`default-features = false`）在
//!   `Cargo.toml` 里**只有一处注释**，在本 crate 内也**只有一个模块**引用它 ⇒
//!   将来若 `ADR-0011` 被推翻，删除面是一个文件而不是散落在 2300 行的调度器里；
//! - 本模块 ⛔ 不复制 `render.rs` 的任何逻辑：`AudioSource` / `BlockContext` /
//!   `RenderError` 全部从 [`crate::render`] 导入。
//!
//! ### ② `advance(frames)` 的顺序契约：**有状态，且由本模块强制**
//!
//! 今天的引擎音源不是"绝对帧的纯函数"：它内部有一个播放头（`SynthEngine::position`），
//! `render_track` 从**当前**播放头渲染、`advance(frames)` 负责推进。于是"块无关"
//! 从**构造性**（正弦源的相位由绝对帧号算出）降级为**契约性**：
//!
//! > 调用方必须按**从第 0 帧开始、连续升序、不重不漏**的顺序调用 [`render_block`]；
//! > 每次调用的 `context.first_frame` 必须等于引擎当前播放头，返回后播放头前进
//! > `context.frames` 帧。
//!
//! 本模块把这条契约**写成代码**而不是只写成文档：`context.first_frame` 与
//! `engine.position()` 不符时返回 [`RenderError::Source`]（指名轨道与两个帧号），
//! ⛔ 不静默地从错误位置继续渲染（那会产出"听起来对、位级不可复现"的母带）。
//! 判据：本文件 `tests::the_advance_contract_is_enforced`。
//!
//! ### ③ 块长：**不是** `L1_BLOCK_SIZE` 与引擎 `DEFAULT_BLOCK_FRAMES` 的巧合
//!
//! 本源**不读**任何引擎的块长常量：它只按调用方给的 `context.frames` 渲染并 `advance`
//! 同样多帧。引擎的 `SynthEngine::render_track` 与 `PolySynth::render` 都按**绝对样本
//! 位置**定位（声部只在 `now >= start_sample` 时发声），因此换块长**不改样本**。
//! 本文件把这条判据钉成读数：同一个源用 64 / 128 / 256 帧三种切法渲染同一段总帧数，
//! 三个位级摘要**必须相同**（`tests::the_block_split_does_not_change_a_single_sample`）。
//! 于是"块长一致"不再是"两个 128 今天碰巧相等"，而是"任意块长逐位相同"——
//! 比 `ADR-0011` 要求的更强，且**不依赖** `L1_BLOCK_SIZE` 的取值。
//!
//! ## 与 CLI 的关系（⛔ 不是第二份实现）
//!
//! 本类型就是 `crates/yeban-app/src/export_wav.rs` 里那个 `pub(crate)` 的
//! `EngineTrackSource`：字段、`new`、`render_block`、声相口径逐字搬过来，
//! app 侧改为 `use yeban_render::engine_source::EngineTrackSource`。
//! 判据（同一份工程、同一份快照 ⇒ 逐位相同的母带）见 `yeban-app` 的 `mcp_mount`
//! 里 `C1` 一族；本文件的判据只钉"推进契约 + 块长"。
//!
//! ## 红线 7（[ARCH-RT-001]）
//!
//! [`EngineTrackSource::new`] 会 `arm_from_snapshot`（分配 / 增长区间表）⇒ 它的
//! 允许调用点是**离线导出 / 录音的控制侧**，⛔ 绝不在音频回调里。本 crate 是离线层
//! （模块文档的第一条契约），因此这里安全；`render_block` 本身**零分配**
//! （`mono` 缓冲只在不足时增长，构造后稳定）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotError};
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::{EntityId, YebanProjectV1};
use yeban_sfz::parser::ParseLimits;

use crate::render::{AudioSource, BlockContext, RenderError};

/// 引擎音源装配失败（快照投影 / `arm_from_snapshot`）。
///
/// 调用方把它映射成自己的错误类型：`yeban-app` ⇒ `WavExportError::Engine`，
/// `yeban-mcp` ⇒ `RENDER_FAILED`（带 `reason: "engineSource"`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSourceError {
    message: String,
}

impl EngineSourceError {
    /// 造一条报文（本模块内部的装载/摊平失败用）。
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// 报文（给调用方拼进 `data.reason` / 错误文本）。
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for EngineSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineSourceError {}

impl From<SnapshotError> for EngineSourceError {
    fn from(error: SnapshotError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

/// ⭐ `#274`：**控制侧交回的一条样本**（SFZ 的 `sample=` 原样字符串 ＋ 已解码 PCM）。
///
/// 单位与口径：
/// * `path`：`region.sample_path()` 的**原样**字符串（⚠ 可能已拼上
///   `default_path=` 前缀）—— 它同时是 `SfzPlan::prepare_with_samples` 的**去重键**，
///   ⛔ 本层不做任何路径规范化（与引擎的 `PreparedSfz::prepare_with_samples` 同一句）；
/// * `pcm`：单声道、归一化到 `[-1, 1]` 的样本帧；⭐ `#279` 起是 `Arc<Vec<f32>>`
///   —— provider 直接交出它自己那份已解码 PCM 的 `Arc::clone`（⛔ 不复制元素），
///   引擎摊平表也只 `Arc::clone` 它 ⇒ **同一份 PCM 只有一份字节**；
/// * `sample_hz`：该 PCM 的**原始采样率**（Hz）—— 播放速率换算交给引擎的
///   `RenderRates`（⛔ 本层不重采样）。
pub type ArmedSfzSample = (String, (Arc<Vec<f32>>, f32));

/// ⭐ `#274`：一条轨的**武装输入**（provider 的返回值）。
///
/// `resolved_path` 由调用方**解析**（绝对原样 ⇒ 工程目录 ⇒ …）：本层只负责按这份
/// 路径读文本、摊平、注入快照 —— 路径解析的候选顺序属于各入口自己的口径
/// （CLI 的 `SfzRoots` / MCP 的 `resolve_sampler_path`），⛔ 不由本层发明第三份。
#[derive(Clone, Debug, Default)]
pub struct ArmedSfzInput {
    /// 解析后的 SFZ 文件路径（本层按它读盘）。
    pub resolved_path: PathBuf,
    /// [`ArmedSfzSample`] 的列表（去重由引擎的 `prepare_with_samples` 做）。
    ///
    /// ⭐ `#279`：列表只搬 `Arc` 指针（构造/搬动都是常数级）；PCM 元素不在这里被复制。
    pub samples: Vec<ArmedSfzSample>,
}

impl ArmedSfzInput {
    /// 造一份输入。
    #[must_use]
    pub fn new(resolved_path: PathBuf, samples: Vec<ArmedSfzSample>) -> Self {
        Self {
            resolved_path,
            samples,
        }
    }

    /// 空输入（provider 失败时的占位：调用方随后会用自己捕获的错误**响亮失败**，
    /// ⛔ 不会拿这份空表去渲染）。
    #[must_use]
    pub fn empty(resolved_path: impl Into<PathBuf>) -> Self {
        Self {
            resolved_path: resolved_path.into(),
            samples: Vec::new(),
        }
    }
}

/// ⭐ `#274`：武装之后的**逐轨读数**（来自引擎摊平表本身，⛔ 不是另算一遍）。
///
/// 它们是 `data.sources[].sampler` 的承重数字：`regions`／`silent`／`missing` 与
/// CLI 的 `sfz(...)` 行**同一个来源**（[`yeban_engine::sfz_instrument::PreparedSfz`]），
/// 因此两侧读数不可能漂移。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmedSfzSummary {
    /// 模型里登记的路径（**逐字**，⛔ 未解析）。
    pub sfz_path: String,
    /// 实际读的那份文件（provider 解析的路径）。
    pub resolved_path: PathBuf,
    /// 摊平后的区域数。
    pub regions: usize,
    /// materialize 的**去重**样本数（＝提供表里真的解码成功的那几份）。
    pub samples: usize,
    /// 不产生采样输出的区域数（缺样本 ＋ 内建静音，引擎口径）。
    pub silent_regions: usize,
    /// 缺样本的路径（升序去重；`silent_regions` 里除内建静音以外的那一半）。
    pub missing_samples: Vec<String>,
}

/// 一次工程快照 ＋ 逐轨引擎源的工厂。
///
/// 存在的理由：`EngineSnapshot::from_project` 会把**全部**轨道（调度表 / 音色 /
/// 鼓机 / SFZ 待摊平 / 音频摆放）投影一遍，逐轨各建一份是纯浪费；
/// MCP 一次渲染要建 N 个源 ⇒ 快照只投影**一次**、`Arc` 共享（与 CLI 的
/// `render_core` 同一手法：`Arc::clone` 是指针复制，⛔ 不 clone PCM）。
pub struct EngineTimbre {
    snapshot: Arc<EngineSnapshot>,
    /// ⭐ `#274`：已武装的 SFZ 轨读数（键 = 轨）。空 = 这次快照没有摊平任何 SFZ
    /// （没有 `TrackV3.sampler`，或调用方走的是非武装入口）。
    armed: BTreeMap<EntityId, ArmedSfzSummary>,
}

impl EngineTimbre {
    /// 从工程投影一份快照（`revision = 1`，与 CLI 的离线装配同口径）。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：工程没有可用的主总线 / 路由图非法 / PDC 失败。
    pub fn from_project(project: &YebanProjectV1) -> Result<Self, EngineSourceError> {
        Ok(Self {
            snapshot: Arc::new(EngineSnapshot::from_project_with_tempo(
                project,
                1,
                &project.tempo(),
            )?),
            armed: BTreeMap::new(),
        })
    }

    /// ⭐ `#274`：**武装入口** —— 投影快照，再把 `sfz_pending()` 里每条轨的样本摊平
    /// 进快照（`SfzPlan::prepare_with_samples` ＋ `with_sfz`）。
    ///
    /// ## 分工（与引擎 `PendingSfz` 的分工图**同一条**）
    ///
    /// ```text
    /// from_project(project)            ⇒ 快照（含 sfz_pending：轨 → 路径）
    /// provider(track, raw_path)        ⇒ ArmedSfzInput { resolved_path, samples }
    ///                                      ⚠ 读盘／解析／解码都在 provider 里（控制侧，允许 I/O）
    /// SfzInstrument::load*(文本)        ⇒ 区域表（⛔ 引擎不读盘，本层只读 provider 指的那份文件）
    /// SfzPlan::prepare_with_samples    ⇒ 摊平表（去重 ＋ 缺样本静音 ＋ 计数）
    /// with_sfz                         ⇒ 摊平表**取代**待摊平项
    /// ```
    ///
    /// ⚠ **provider 是可靠的**：它不返回 `Result` —— 失败（文件读不到 / 文本非法 /
    /// 样本读不动）由调用方在**调用本函数之前**用自己的错误类型响亮报出（MCP 的
    /// `RENDER_FAILED` / CLI 的 `SfzLoadError` 都在各自那一侧）。provider 失败时
    /// 返回 [`ArmedSfzInput::empty`] 占位，调用方随即丢弃本次 `EngineTimbre`。
    ///
    /// ⚠ `engine_hz`：摊平期采样率（进 `rate_per_note` / 包络换算）。CLI 传的是
    /// **导出率**（`WavExportOptions::sample_rate`），MCP 传的是**请求率**
    /// ⇒ 两侧同一份工程、同一个率时逐位相同。
    ///
    /// ⚠ ⛔ **本层不解码**：`samples` 里是**已经解码**的 PCM
    /// （`yeban-decode`／`symphonia` 留在调用方），因此 `yeban-render` 的依赖闭包
    /// ⛔ 不新增任何音频解码器（`#270` 的禁区）。
    ///
    /// # Errors
    ///
    /// 快照投影失败 / SFZ 文件读不到 / SFZ 文本非法 / 摊平失败 ⇒ [`EngineSourceError`]
    /// （报文带具体路径）。
    pub fn from_project_armed<F>(
        project: &YebanProjectV1,
        engine_hz: f32,
        provider: F,
    ) -> Result<Self, EngineSourceError>
    where
        F: FnMut(EntityId, &str) -> ArmedSfzInput,
    {
        // `#339`：离线渲染读 `tempo_map`（空表与 `from_project` 逐位相同）。
        let snapshot = EngineSnapshot::from_project_with_tempo(project, 1, &project.tempo())?;
        Self::from_snapshot_armed(snapshot, engine_hz, provider)
    }

    /// 与 [`Self::from_project_armed`] 相同，但快照由调用方给（已有快照时省一次投影）。
    ///
    /// # Errors
    ///
    /// 与 [`Self::from_project_armed`] 相同。
    pub fn from_snapshot_armed<F>(
        snapshot: EngineSnapshot,
        engine_hz: f32,
        mut provider: F,
    ) -> Result<Self, EngineSourceError>
    where
        F: FnMut(EntityId, &str) -> ArmedSfzInput,
    {
        // 先收集（键升序）再逐条摊平：`with_sfz` 吃掉 `self`，⛔ 不能边借边改。
        let pending: Vec<(EntityId, String)> = snapshot
            .sfz_pending()
            .iter()
            .map(|(track, entry)| (*track, entry.path().to_owned()))
            .collect();
        let mut snapshot = snapshot;
        let mut armed: BTreeMap<EntityId, ArmedSfzSummary> = BTreeMap::new();
        for (track, raw_path) in pending {
            let input = provider(track, &raw_path);
            let instrument = load_sfz_instrument(&input.resolved_path)?;
            let provided: Vec<(String, InMemorySample)> = input
                .samples
                .into_iter()
                .map(|(path, (pcm, sample_hz))| (path, InMemorySample { pcm, sample_hz }))
                .collect();
            let plan = SfzPlan::prepare_with_samples(&instrument, &provided, engine_hz).map_err(
                |error| {
                    EngineSourceError::new(format!(
                        "SFZ 摊平失败 ({}): {error}",
                        input.resolved_path.display()
                    ))
                },
            )?;
            let prepared = plan.prepared();
            armed.insert(
                track,
                ArmedSfzSummary {
                    sfz_path: raw_path,
                    resolved_path: input.resolved_path,
                    regions: prepared.region_count(),
                    samples: prepared.sample_count(),
                    silent_regions: prepared.silent_region_count(),
                    missing_samples: prepared.missing_sample_paths().to_vec(),
                },
            );
            snapshot = snapshot.with_sfz(track, plan);
        }
        Ok(Self {
            snapshot: Arc::new(snapshot),
            armed,
        })
    }

    /// 直接包一份快照（app 的导出 / 录音路径已有快照时用它）。
    #[must_use]
    pub fn from_snapshot(snapshot: Arc<EngineSnapshot>) -> Self {
        Self {
            snapshot,
            armed: BTreeMap::new(),
        }
    }

    /// ⭐ `#274`：该轨的武装读数（`None` = 这次快照没给它摊平 SFZ）。
    #[must_use]
    pub fn armed_sfz(&self, track: EntityId) -> Option<&ArmedSfzSummary> {
        self.armed.get(&track)
    }

    /// 快照的采样率（Hz）—— 源的构造与音频配置必须用**同一份**快照的值。
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.snapshot.sample_rate()
    }

    /// 该轨在快照里的**已排程音符数**（0 = 该轨没有可发声的 MIDI 内容）。
    ///
    /// MCP 的 `sources[].notes` 读数用它，⛔ 不用自己再排一遍 —— 否则"报出来的
    /// 音符数"与"引擎真的会触发的音符数"会变成两条可以漂移的口径。
    #[must_use]
    pub fn scheduled_notes(&self, track: EntityId) -> usize {
        self.snapshot
            .schedules()
            .get(&track)
            .map_or(0, NoteSchedule::len)
    }

    /// 为一条轨道武装一个引擎源。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：`arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    pub fn track_source(&self, track: EntityId) -> Result<EngineTrackSource, EngineSourceError> {
        EngineTrackSource::new(Arc::clone(&self.snapshot), track)
    }
}

/// 控制侧**读盘 ＋ 解析**一份 SFZ（⭐ `#274`；与 CLI 的
/// `yeban_app::sfz_load::load_sfz_with_instrument` **同一份** include 口径）。
///
/// * 根文本在这里读（⛔ 引擎不读盘）；
/// * `ParseLimits::default()` 与 CLI / `SfzLoad::with_instrument` 同一个缺省；
/// * 有 UTF-8 文件名 ⇒ `load_with_includes`（`#include` 展开，基准目录 = 根文件所在目录）；
///   否则退回纯文本 `load`（既有行为）。
fn load_sfz_instrument(path: &Path) -> Result<SfzInstrument, EngineSourceError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        EngineSourceError::new(format!("读 SFZ 文件失败 ({}): {error}", path.display()))
    })?;
    let limits = ParseLimits::default();
    let base_dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let entry = path.file_name().and_then(|name| name.to_str());
    match entry {
        Some(entry) if !entry.is_empty() => {
            SfzInstrument::load_with_includes(text, base_dir, entry, &limits)
        }
        _ => SfzInstrument::load(text, &limits),
    }
    .map_err(|error| EngineSourceError::new(format!("SFZ 文本非法 ({}): {error}", path.display())))
}

/// **引擎乐器音源**：每条轨道一个 [`SynthEngine`]，用**单轨武装**
/// （`SynthEngine::arm_track_from_snapshot`）装进合成器，再按轨渲染（`render_track`）。
///
/// ⚠ 单轨武装不是优化而是**正确性**：本源只渲染自己那条轨，全量武装会让工程里
/// 第 17 条起的源拿不到槽位（16 槽上限）而**静音**，见 `arm_track_from_snapshot`。
///
/// ⚠ 每个源持有**自己的**引擎：`RenderPlan::execute` 会把源分发到工作线程（`threads`），
/// 共享一个 `&mut SynthEngine` 需要锁，而"一个源一个引擎"没有锁也没有跨线程共享。
/// ⚠ 红线 7：`arm_from_snapshot` **会分配**（`begin_snapshot` 增长区间表）⇒ 只在**离线导出**
/// 里调用，⛔ 绝不在音频回调里调用（本 crate 是离线路径）。
pub struct EngineTrackSource {
    /// 已武装的合成器（本源独占）。
    ///
    /// ⭐ **为什么是 `Box<SynthEngine>` 而不是内联的 `SynthEngine`**（`#72` 的修法）：
    /// `size_of::<SynthEngine>()` 实测 **109152 B**（`synth.rs` 的字段文档是同一读数）。
    /// 内联时本结构体就是 `EngineTrackSource = 109232 B`，于是**每一次按值搬动**
    /// （`new()` 的返回位、`?` 的中间位、`Box::new` 的装箱源、`Box<T> → Box<dyn Trait>`
    /// 的 unsize 强制转换）都要在栈上留一份 10 万字节级的副本 —— 实测
    /// `render_project_f32_with_assets` 的栈帧因此达 **582480 B**、
    /// `EngineTrackSource::new` 达 **327904 B**。`Box` 之后本结构体只剩一个指针（8 B）
    /// ＋轨道 ID ＋调度表 ＋暂存 `Vec`，搬动是**常数级**。
    /// ⚠ 这只是**把大对象从栈挪到堆**：分配点仍只有"离线导出/录音的构造期"（[`Self::new`]），
    /// ⛔ 不进实时路径（红线 7）。
    engine: Box<SynthEngine>,
    /// 本源负责的轨道。
    track: EntityId,
    /// 该轨的调度表（来自同一份快照；`None` ⇒ 该轨静音，与 `render_track` 的文档一致）。
    schedule: Option<NoteSchedule>,
    /// ⭐ **武装用的那份快照**（`#79`）：`render_block` 从它**借用**该轨的 SFZ 摊平表
    /// （[`EngineSnapshot::sfz`]）交给 `render_track` 的第 3 参。
    ///
    /// ⭐ 为什么是 `Arc<EngineSnapshot>` 而**不是** clone 一份 `SfzPlan`：摊平表里带着
    /// **区域表**与 PCM（[`yeban_engine::sfz_instrument::PreparedSample`]）；⭐ `#279` 起
    /// PCM 自身已 `Arc` 共享（clone 不再复制样本字节），但 clone 一份 `SfzPlan` 仍会按轨
    /// 复制**区域表** ⇒ 快照仍按 `Arc` 共享。`Arc` clone 只是**指针复制** ⇒ 与实时路径
    /// 同一条纪律（"实时侧只**借用**快照，⛔ 不 clone 那些 `Vec`"，见 `sfz_instrument.rs`
    /// 的模块文档）。
    snapshot: Arc<EngineSnapshot>,
    /// 单声道暂存（`render_track` 输出"声相之前"的单声道）。
    mono: Vec<f32>,
    /// ⭐ **轨声相**（`(左, 右)`；默认 **`(1.0, 1.0)`＝居中**，`#251`）。
    ///
    /// `render_track` 产出的是**声相之前**的单声道 ⇒ 声相必须在本源补上，否则
    /// 导出把用户拉过的 pan 完全丢掉（`#250` 勘察的实测缺陷）。
    /// 值来自调用方的 `track_pan_gains`（与音频片段同一条曲线）。
    pan: (f32, f32),
    /// ⭐ `#274`：**声部窃取**读数（引擎 SFZ 声部池 16 槽，池满 ⇒ 窃取最老的声部）。
    ///
    /// 为什么是 `Arc<AtomicU64>`：本值在**渲染之后**才有意义，而渲染期间本结构体被
    /// `Box<dyn AudioSource>` 吃掉（`RenderPlan::execute`）⇒ 调用方必须在装箱**之前**
    /// 留一个句柄（[`Self::steal_counter`]）。写入只发生在 `render_block` 末尾
    /// （一次 relaxed `store`），⛔ 不影响音频。
    steals: Arc<AtomicU64>,
}

impl EngineTrackSource {
    /// 从快照武装一个轨道源。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：`arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    pub fn new(snapshot: Arc<EngineSnapshot>, track: EntityId) -> Result<Self, EngineSourceError> {
        // ⭐ 直接构造**在堆上**：`SynthEngine::new` 的 109152 B 返回位因此只在本帧
        //    存在一次（随后 memcpy 进 `Box`），⛔ 不再有"返回位 ＋ 中间位 ＋ 装箱源"
        //    三份 10 万字节级副本叠在同一帧里（见字段文档的尺寸对照）。
        let mut engine = Box::new(SynthEngine::new(snapshot.sample_rate()));
        // ⛔ 不 `unwrap`：没有主总线时如实失败（调用方据此在**落盘之前**退出）。
        //
        // ⭐ **只武装这一条轨**（`arm_track_from_snapshot`）：本源只渲染 `track`，
        //    而全量武装会把快照里**全部**轨道塞进 16 个槽 —— 工程超过 16 条轨时，
        //    排在第 17 条起的源拿不到槽位 ⇒ 整轨数字静音、回执无提示（实测缺陷：
        //    28 轨工程里 12 条轨道 `peak = 0.0`）。逐轨武装让"一个源一个槽"，
        //    与工程规模解耦；顺带让 `track_drops` 在这条路径上恒为 0。
        engine.arm_track_from_snapshot(&snapshot, track)?;
        Ok(Self {
            engine,
            track,
            schedule: snapshot.schedules().get(&track).cloned(),
            snapshot,
            mono: Vec::new(),
            // ⭐ 默认**居中**（`1.0, 1.0`）：没注入时导出字节与接线前逐位相同（`#251`）。
            pan: (1.0, 1.0),
            steals: Arc::new(AtomicU64::new(0)),
        })
    }

    /// ⭐ 写入**轨声相**（`(左, 右)`；构造期一次，`#251`）。
    ///
    /// 值必须来自调用方的 `track_pan_gains`（与音频片段路径**同一个**函数，
    /// 同一条等功率曲线）。缺省 `(1.0, 1.0)` ⇒ 原来的单声道铺放分支。
    #[must_use]
    pub const fn with_pan(mut self, pan: (f32, f32)) -> Self {
        self.pan = pan;
        self
    }

    /// 本源负责的轨道 ID。
    #[must_use]
    pub const fn track(&self) -> EntityId {
        self.track
    }

    /// 从快照投影来的**已排程音符数**（`sources[].notes` 的唯一来源）。
    #[must_use]
    pub fn scheduled_notes(&self) -> usize {
        self.schedule.as_ref().map_or(0, NoteSchedule::len)
    }

    /// 引擎当前播放头（绝对帧）。判据用它证明"每块恰好推进一次"。
    #[must_use]
    pub fn position(&self) -> u64 {
        self.engine.position()
    }

    /// 引擎累计触发音符数（判据读数；⛔ 不参与音频）。
    #[must_use]
    pub fn notes_triggered(&self) -> u64 {
        self.engine.notes_triggered()
    }

    /// ⭐ `#274`：**声部窃取**计数的句柄（渲染结束后读；见字段文档）。
    ///
    /// 口径：`SynthEngine::sfz_voice_steals()` 的**累计**值（该引擎所有轨的 SFZ 声部池
    /// 之和）。MCP 一个源一台引擎且只渲染自己那条轨 ⇒ 读数就是这条轨的窃取数。
    /// ⚠ 与 `sfz_voice_drops()` 配对：`池满的溢出 = 窃取 + 真丢弃`，本读数**只**计窃取。
    #[must_use]
    pub fn steal_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.steals)
    }
}

impl AudioSource for EngineTrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // ⭐ `ADR-0011` 未决 ②：把"块无关是契约性的"落成**可失败的前置条件**。
        //    本源的播放头必须与调用方声明的块起点一致；不一致 ⇒ 指名失败，
        //    ⛔ 不静默地按错误位置渲染（那会产出不可复现的母带）。
        if context.first_frame != self.engine.position() {
            return Err(RenderError::Source {
                node: self.track,
                message: format!(
                    "引擎音源的推进契约被违反: `first_frame` = {}，引擎播放头 = {} \
                     （调用方必须从第 0 帧起按连续升序的块渲染，每块恰好一次）",
                    context.first_frame,
                    self.engine.position()
                ),
            });
        }
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        if self.mono.len() < frames {
            self.mono.resize(frames, 0.0);
        }
        // `render_track` 从引擎**当前**播放头渲染，它**不**推进游标（契约见
        // `SynthEngine::render_track` 与 `unknown_track_renders_silence` 的
        // "渲染不推进播放头（由 advance 负责）"）；推进由本函数紧随其后的
        // `advance(frames)` 负责 —— 与实时路径 `rt.rs` 的同一量子同一顺序一致。
        // ⭐ ⭐ 第 3 参（SFZ 摊平表）**从快照借用**（`#79`）：这条轨在快照 `sfz()` 表里
        //    ⇒ 走引擎的第三条音源支路（鼓机 ＞ SFZ ＞ 复音）；不在表里 ⇒ `None` ⇒ 退回复音，
        //    输出与接线前**逐位相同**（`TrackV3.sampler = None` 的反向对照）。
        //    ⛔ 这里**不** clone 摊平表（见字段文档）：只把 `&PreparedSfz` 借出去。
        let sfz = self
            .snapshot
            .sfz()
            .get(&self.track)
            .map(yeban_engine::sfz_instrument::SfzPlan::prepared);
        self.engine.render_track(
            self.track,
            self.schedule.as_ref(),
            sfz,
            &mut self.mono[..frames],
        );
        // ⭐ `#209`：**必须**推进播放头，否则每个块都从位置 0 渲染
        //    ⇒ 只有 tick 0 的音符会被触发一次并一直拖长，其余音符永不发声
        //    （`#207` 实测 `--export-wav` ⇒ `distinct=1`）。
        // ⚠ `render_track` 自带 `out.fill(0.0)` 与内部游标读取，但**不**推进位置：
        //    推进只在 `advance`（`synth.rs` 的契约；实时路径见 `rt.rs` 的
        //    `if running { transport.advance_frames(frames as u64); synth.advance(frames); }`）。
        // ⚠ 本函数每块恰好渲染一次、推进一次 ⇒ 与 `first_frame` 同步，⛔ 不会推进两次。
        self.engine.advance(frames);
        // ⭐ `#274`：**声部窃取**读数（引擎 SFZ 声部池 16 槽；池满 ⇒ 窃取）。
        //    ⚠ 只写一个 relaxed 原子（⛔ 不改音频、⛔ 不分配）；调用方在渲染之后读。
        self.steals
            .store(self.engine.sfz_voice_steals(), Ordering::Relaxed);
        out.fill(0.0);
        let (gain_l, gain_r) = self.pan;
        for index in 0..frames {
            let value = self.mono[index];
            let base = index * channels;
            if gain_l == 1.0 && gain_r == 1.0 {
                // ⭐ 居中（绝大多数工程）：原来的"单声道铺到所有声道"，逐位不变。
                for channel in 0..channels {
                    out[base + channel] = value;
                }
            } else {
                // ⭐ `#251`：等功率声相。第 0 声道左、第 1 声道右，其余保持 0。
                out[base] = value * gain_l;
                if channels > 1 {
                    out[base + 1] = value * gain_r;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use sha2::{Digest, Sha256};

    use crate::render::RenderOutput;

    /// ⭐ `#274`：**武装入口**把 `sfz_pending` 摊平进快照，并把同一份摊平表的数字报回来。
    ///
    /// 三条腿：
    /// 1. provider 被**恰好调用一次**（键 = 那条绑了 sampler 的轨，参数 = **原样**路径）；
    /// 2. 给了样本 ⇒ `regions = samples = 1`、`silent = 0`、`missing = []`；
    /// 3. ⛔ 不给样本 ⇒ 引擎口径**静音 ＋ 计数**（`silent = 1`、`missing = ["tone.wav"]`）
    ///    —— 证明"缺样本"不会被悄悄补成别的内容（`R711`）。
    #[test]
    fn the_armed_entry_flattens_pending_sfz_and_reports_the_same_numbers_as_the_engine() {
        let dir = tempfile::tempdir().expect("临时目录");
        let sfz_path = dir.path().join("t.sfz");
        std::fs::write(&sfz_path, "<region> sample=tone.wav key=60\n").expect("写 SFZ");
        let wav_path = dir.path().join("tone.wav");
        crate::wav::write_plain_wav(
            &wav_path,
            &crate::rf64::PcmFormat::float(1, 48_000, 32),
            &crate::dither::PcmBuffer::Float32(vec![0.5_f32; 4_800]),
        )
        .expect("写样本");

        let mut project = yeban_model::samples::demo_project();
        let track = project
            .tracks
            .values()
            .find(|track| {
                **track != project.tracks[&project.master_bus_track_id]
                    && track.clips.values().any(|placement| {
                        project
                            .clip_pool
                            .get(&placement.clip_id)
                            .is_some_and(|entry| {
                                matches!(entry.content, yeban_model::ClipContent::Midi { .. })
                            })
                    })
            })
            .expect("演示工程必须有 MIDI 轨")
            .id;
        project.tracks.get_mut(&track).expect("轨").sampler = Some(yeban_model::SamplerBinding {
            path: sfz_path.display().to_string(),
        });
        project.validate().expect("武装夹具必须合法");

        let mut calls = 0u32;
        let timbre = EngineTimbre::from_project_armed(&project, 48_000.0, |called, raw| {
            calls = calls.saturating_add(1);
            assert_eq!(called, track, "provider 只应为那条绑了 sampler 的轨调用");
            assert_eq!(
                raw,
                sfz_path.display().to_string(),
                "路径必须**原样**交给 provider"
            );
            ArmedSfzInput::new(
                sfz_path.clone(),
                vec![(
                    "tone.wav".to_owned(),
                    (Arc::new(vec![0.25_f32; 4_800]), 48_000.0),
                )],
            )
        })
        .expect("武装必须成功");
        assert_eq!(calls, 1, "provider 必须恰好调用一次");
        let summary = timbre.armed_sfz(track).expect("必须留下该轨的读数");
        assert_eq!(
            (
                summary.regions,
                summary.samples,
                summary.silent_regions,
                summary.missing_samples.clone(),
            ),
            (1, 1, 0, Vec::new()),
            "一条区域 / 一份样本 / 无静音 / 无缺样本"
        );

        // 反向对照：同样的 SFZ，提供表为空 ⇒ 引擎按缺样本处理（静音 ＋ 计数）。
        let timbre = EngineTimbre::from_project_armed(&project, 48_000.0, |_, _| {
            ArmedSfzInput::new(sfz_path.clone(), Vec::new())
        })
        .expect("空提供表也必须能摊平");
        let summary = timbre.armed_sfz(track).expect("必须留下该轨的读数");
        assert_eq!(
            (summary.regions, summary.samples, summary.silent_regions),
            (1, 0, 1),
            "缺样本 ⇒ 该区域不产生采样输出（引擎口径）"
        );
        assert_eq!(summary.missing_samples, vec!["tone.wav".to_owned()]);
    }

    /// 演示工程的第一个有声轨 + 一份快照（与 app 的离线装配同一条入口）。
    fn source(project: &YebanProjectV1) -> (EngineTimbre, EntityId) {
        let timbre = EngineTimbre::from_project(project).expect("演示工程必须成快照");
        let track = timbre
            .snapshot
            .schedules()
            .iter()
            .find(|(_, schedule)| !schedule.notes().is_empty())
            .map(|(track, _)| *track)
            .expect("演示工程必须至少有一条有声轨");
        (timbre, track)
    }

    /// 把 `frames` 帧按 `block` 切块渲染成单声道，返回整段的位级摘要。
    fn digest_with_block(
        timbre: &EngineTimbre,
        track: EntityId,
        frames: u64,
        block: usize,
    ) -> String {
        let mut source = timbre.track_source(track).expect("必须能武装");
        let mut mono: Vec<f32> = Vec::with_capacity(usize::try_from(frames).unwrap());
        let mut out = vec![0.0_f32; block];
        let mut first = 0u64;
        let mut index = 0u64;
        while first < frames {
            let len = usize::try_from((frames - first).min(block as u64)).unwrap();
            source
                .render_block(
                    BlockContext {
                        block_index: index,
                        first_frame: first,
                        frames: len,
                        channels: 1,
                        sample_rate: 48_000,
                        seed: 1,
                    },
                    &mut out[..len],
                )
                .expect("连续升序的块必须被接受");
            mono.extend_from_slice(&out[..len]);
            first += len as u64;
            index += 1;
        }
        let mut hasher = Sha256::new();
        for sample in &mono {
            hasher.update(sample.to_bits().to_le_bytes());
        }
        // `sha2 0.11` 的 `finalize()` 返回 `hybrid_array::Array`，只实现 `LowerHex`
        // 的旧版本实现不覆盖它 ⇒ 本判据自己拼十六进制（与 `render::digest_of` 同口径：
        // 对**位型**的字节流做 SHA-256）。
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// ⭐ `ADR-0011` 未决 ③：块长**不是**巧合 —— 64 / 128 / 256 三种切法逐位相同。
    ///
    /// 牙齿：把 `render_block` 里的 `self.engine.advance(frames)` 删掉 ⇒ 只有第一块
    /// 有声音，三种切法的摘要会分叉（128 与 64 的块数不同 ⇒ 同一条判据同时变红）。
    #[test]
    fn the_block_split_does_not_change_a_single_sample() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let frames = 48_000u64;
        let reference = digest_with_block(&timbre, track, frames, 128);
        let at_64 = digest_with_block(&timbre, track, frames, 64);
        let at_256 = digest_with_block(&timbre, track, frames, 256);
        println!(
            "### [block-split] 128={reference} 64={at_64} 256={at_256} 相同={}",
            reference == at_64 && reference == at_256
        );
        assert_eq!(
            reference, at_64,
            "64 帧与 128 帧的渲染必须逐位相同（引擎按绝对样本位置起振）"
        );
        assert_eq!(
            reference, at_256,
            "256 帧与 128 帧的渲染必须逐位相同（引擎按绝对样本位置起振）"
        );
    }

    /// ⭐ `ADR-0011` 未决 ②：推进契约被**强制**（不是只写在文档里）。
    ///
    /// 三条读数：从 0 起跳块 ⇒ 接受；重复/跳跃的 `first_frame` ⇒ 指名失败；
    /// 失败信息里带两个帧号（可定位）。
    #[test]
    fn the_advance_contract_is_enforced() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let mut source = timbre.track_source(track).expect("必须能武装");
        let mut out = vec![0.0_f32; 128];
        source
            .render_block(
                BlockContext {
                    block_index: 0,
                    first_frame: 0,
                    frames: 128,
                    channels: 1,
                    sample_rate: 48_000,
                    seed: 1,
                },
                &mut out,
            )
            .expect("从第 0 帧起的第一块必须被接受");
        assert_eq!(source.position(), 128, "每块必须恰好推进一次");
        // 跳跃：跳过第 128 帧那一块 ⇒ 引擎播放头还是 128，调用方声称 256。
        let error = source
            .render_block(
                BlockContext {
                    block_index: 2,
                    first_frame: 256,
                    frames: 128,
                    channels: 1,
                    sample_rate: 48_000,
                    seed: 1,
                },
                &mut out,
            )
            .expect_err("跳跃的块必须被拒绝");
        let message = error.to_string();
        assert!(
            message.contains("256") && message.contains("128"),
            "失败信息必须点名两个帧号: {message}"
        );
        assert_eq!(source.position(), 128, "被拒绝的块不得推进播放头");
    }

    /// 多轨夹具：`n` 条 MIDI 轨（第 i 条音高 `48 + i % 24`）各一个 480-tick 音符，
    /// 全部直连 master。身份按声明顺序升序 ⇒ `BTreeMap` 迭代序 = 声明序。
    fn many_track_project(n: usize) -> (YebanProjectV1, Vec<EntityId>) {
        use std::str::FromStr as _;

        use yeban_model::{
            ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, MidiNote, RoutingEdge,
            RoutingKind, SampleRate, TrackKind, TrackV3,
        };

        let id = |seed: usize| {
            EntityId::from_str(&format!("01J8ZM{seed:020}")).expect("夹具 ULID 必须合法")
        };
        let master = id(1);
        let mut project = YebanProjectV1 {
            title: "multi-source fixture".to_owned(),
            master_bus_track_id: master,
            ..YebanProjectV1::default()
        };
        project.audio_config.sample_rate = SampleRate::Hz48000;
        project.tracks.insert(
            master,
            TrackV3 {
                id: master,
                name: "Master".to_owned(),
                kind: TrackKind::Master,
                ..TrackV3::default()
            },
        );
        project.routing_graph.nodes.push(master);
        let mut ids = Vec::with_capacity(n);
        for index in 0..n {
            let track = id(100 + index);
            let clip = id(1_000 + index);
            let placement = id(2_000 + index);
            let note = id(3_000 + index);
            let edge = id(4_000 + index);
            let mut track_params = TrackV3 {
                id: track,
                name: format!("Track {}", index + 1),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            };
            track_params.clips.insert(
                placement,
                ClipPlacement {
                    id: placement,
                    clip_id: clip,
                    start_tick: 0,
                    duration_ticks: 480,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
            project.tracks.insert(track, track_params);
            let mut notes = BTreeMap::new();
            notes.insert(
                note,
                MidiNote::new(note, 0, 48 + u8::try_from(index % 24).expect("取模"), 480),
            );
            project.clip_pool.insert(
                clip,
                ClipPoolEntry {
                    id: clip,
                    name: format!("Clip {}", index + 1),
                    content: ClipContent::Midi { notes },
                },
            );
            project.routing_graph.nodes.push(track);
            project.routing_graph.edges.insert(
                edge,
                RoutingEdge {
                    id: edge,
                    source_node: track,
                    destination_node: master,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            );
            ids.push(track);
        }
        project
            .validate()
            .unwrap_or_else(|error| panic!("多轨夹具必须合法: {error}"));
        (project, ids)
    }

    /// 把一个源按 128 帧切块渲染 `frames` 帧，返回样本峰值（线性幅度）。
    fn render_peak(source: &mut EngineTrackSource, frames: u64) -> f32 {
        let mut out = vec![0.0_f32; 128];
        let mut peak = 0.0_f32;
        let mut first = 0u64;
        let mut index = 0u64;
        while first < frames {
            let len = usize::try_from((frames - first).min(128)).expect("块长");
            source
                .render_block(
                    BlockContext {
                        block_index: index,
                        first_frame: first,
                        frames: len,
                        channels: 1,
                        sample_rate: 48_000,
                        seed: 1,
                    },
                    &mut out[..len],
                )
                .expect("连续升序的块必须被接受");
            for sample in &out[..len] {
                peak = peak.max(sample.abs());
            }
            first += len as u64;
            index += 1;
        }
        peak
    }

    /// **实测缺陷回归（G1）**：工程源数超过实时槽位上限（`MAX_TRACK_SLOTS` = 16）时，
    /// 第 17 条起的源**不得**静音。
    ///
    /// 缺陷：每个源独享一台引擎却全量武装快照 ⇒ 第 17 条起拿不到槽位、整轨数字静音。
    /// 判据逐条渲染 24 个源，任何一条峰值 `<= 0` 就红。
    #[test]
    fn every_track_source_sounds_beyond_the_sixteen_slot_cap() {
        const TRACKS: usize = 24;
        let (project, ids) = many_track_project(TRACKS);
        let timbre = EngineTimbre::from_project(&project).expect("多轨夹具必须成快照");
        let mut silent = Vec::new();
        for (index, track) in ids.iter().enumerate() {
            let mut source = timbre.track_source(*track).expect("必须能武装");
            let peak = render_peak(&mut source, 4_800);
            println!("### [multi-source] #{} peak={peak:.6}", index + 1);
            if peak <= 0.0 {
                silent.push((index + 1, *track));
            }
        }
        assert!(
            silent.is_empty(),
            "{TRACKS} 条源必须全部出声；静音的源（序号, id）: {silent:?}"
        );
    }

    /// 上一条判据的**牙齿**（反向对照）：旧的**全量武装**路径确实会让第 17 条起的轨静音。
    ///
    /// 这里手工复刻旧行为（`arm_from_snapshot` 装全部轨 + 本源只渲染最后一条），
    /// 断言它峰值恰好为 `0.0`。若哪天 `arm_from_snapshot` 不再受槽位上限约束，
    /// 这条会变红 ⇒ 提醒重写它，而不是让它静悄悄地失去意义。
    #[test]
    fn full_snapshot_arming_is_what_silenced_the_seventeenth_track() {
        const TRACKS: usize = 24;
        let (project, ids) = many_track_project(TRACKS);
        let timbre = EngineTimbre::from_project(&project).expect("多轨夹具必须成快照");
        let snapshot = Arc::clone(&timbre.snapshot);
        let last = *ids.last().expect("夹具必须有轨");
        let mut engine = Box::new(SynthEngine::new(snapshot.sample_rate()));
        engine
            .arm_from_snapshot(&snapshot)
            .expect("全量武装本身必须成功（它只是丢槽）");
        assert!(
            engine.track_drops() > 0,
            "{TRACKS} 条轨全量武装必须触发槽位耗尽计数（否则这条牙齿没咬住）"
        );
        let mut legacy = EngineTrackSource {
            engine,
            track: last,
            schedule: snapshot.schedules().get(&last).cloned(),
            snapshot: Arc::clone(&snapshot),
            mono: Vec::new(),
            pan: (1.0, 1.0),
            steals: Arc::new(AtomicU64::new(0)),
        };
        assert_eq!(
            render_peak(&mut legacy, 4_800),
            0.0,
            "全量武装下第 {TRACKS} 条轨必须（曾经）数字静音 —— 这是 G1 的复现"
        );
    }

    /// 位级摘要的**正对照**：静音源不是"恒等摘要"（证明上面的判据不是空转）。
    #[test]
    fn the_digest_is_not_a_constant() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let a = digest_with_block(&timbre, track, 4_800, 128);
        let b = digest_with_block(&timbre, track, 9_600, 128);
        assert_ne!(a, b, "不同长度的两条渲染不应共享摘要（夹具真的在动）");
        assert_eq!(
            RenderOutput::digest_of(&[]),
            RenderOutput::digest_of(&[]),
            "`digest_of` 对空输入是稳定的（它只是 SHA-256 的包装）"
        );
    }
}
