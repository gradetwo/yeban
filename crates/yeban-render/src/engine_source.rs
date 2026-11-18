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

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotError};
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::{EntityId, LaneChainDef, YebanProjectV1};
use yeban_sfz::parser::ParseLimits;

use crate::automation::{CcProbe, centered_pan_gains};
use crate::inserts::{InsertChain, InsertRuntime};
use crate::render::{AudioSource, BlockContext, L1_BLOCK_SIZE, RenderError};

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
/// * `pcm`：归一化到 `[-1, 1]` 的样本帧，**按 `channels` 交错存储**
///   （立体声样本通路片起：立体声 PCM 左右交错原样交出，⛔ 不再折单声道）；
///   ⭐ `#279` 起是 `Arc<Vec<f32>>` —— provider 直接交出它自己那份已解码 PCM
///   的 `Arc::clone`（⛔ 不复制元素），引擎摊平表也只 `Arc::clone` 它
///   ⇒ **同一份 PCM 只有一份字节**；
/// * `channels`：`pcm` 的声道数（`1` ＝ 单声道、`2` ＝ 立体声；provider 的
///   解码若仍是单声道口径就如实填 `1`，⛔ 不许拿交错长度冒充）；
/// * `sample_hz`：该 PCM 的**原始采样率**（Hz）—— 播放速率换算交给引擎的
///   `RenderRates`（⛔ 本层不重采样）。
pub type ArmedSfzSample = (String, (Arc<Vec<f32>>, u16, f32));

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
    /// 预解析用的 SFZ 文本（内嵌腿）：`Some` ⇒ 引擎直接解析这份文本
    /// （[`SfzInstrument::load`]，⛔ 不读盘 —— 无采样库的机器上 `resolved_path`
    /// 指的绑定路径在盘上不存在）；`None` ⇒ 按 `resolved_path` 读盘（旧腿原样）。
    ///
    /// 内嵌文本含 `#include` 时调用方必须**不**走这条路（保存侧从不内嵌这类 SFZ，
    /// 装载侧 `load_with_embedded` 也响亮拒绝 —— `SfzInstrument::load` 不展开 include）。
    pub sfz_text: Option<String>,
}

impl ArmedSfzInput {
    /// 造一份输入。
    #[must_use]
    pub fn new(resolved_path: PathBuf, samples: Vec<ArmedSfzSample>) -> Self {
        Self {
            resolved_path,
            samples,
            sfz_text: None,
        }
    }

    /// 造一份带 SFZ 文本的输入（内嵌腿）：引擎解析这份文本，不读 `resolved_path`。
    #[must_use]
    pub fn with_sfz_text(mut self, text: String) -> Self {
        self.sfz_text = Some(text);
        self
    }

    /// 空输入（provider 失败时的占位：调用方随后会用自己捕获的错误**响亮失败**，
    /// ⛔ 不会拿这份空表去渲染）。
    #[must_use]
    pub fn empty(resolved_path: impl Into<PathBuf>) -> Self {
        Self {
            resolved_path: resolved_path.into(),
            samples: Vec::new(),
            sfz_text: None,
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
            // 内嵌腿：provider 已把 SFZ 文本从 CAS 池取出来 ⇒ 直接解析文本，
            // ⛔ 不读盘（`resolved_path` 只是绑定路径的留痕）；旧腿（`None`）原样读盘。
            let instrument = match input.sfz_text {
                Some(text) => {
                    let display = input.resolved_path.display().to_string();
                    SfzInstrument::load(text, &ParseLimits::default()).map_err(|error| {
                        EngineSourceError::new(format!("SFZ 文本非法 ({display}): {error}"))
                    })?
                }
                None => load_sfz_instrument(&input.resolved_path)?,
            };
            let provided: Vec<(String, InMemorySample)> = input
                .samples
                .into_iter()
                .map(|(path, (pcm, channels, sample_hz))| {
                    (
                        path,
                        InMemorySample {
                            pcm,
                            channels,
                            sample_hz,
                            loop_points: None,
                        },
                    )
                })
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

    /// 该轨的调度表克隆（ADR-0069：插件源与引擎源读**同一份**快照投影，
    /// 不各自重算；`None` = 该轨在快照里没有调度表，插件源按静音推进处理）。
    ///
    /// 与 [`EngineTrackSource::new`] 的取法同一行（`schedules().get(&track).cloned()`），
    /// 只是不附带武装一台 109 KB 引擎 —— 插件轨不需要引擎声，建它纯属浪费。
    #[must_use]
    pub fn schedule(&self, track: EntityId) -> Option<NoteSchedule> {
        self.snapshot.schedules().get(&track).cloned()
    }

    /// 为一条轨道武装一个引擎源。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：`arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    pub fn track_source(&self, track: EntityId) -> Result<EngineTrackSource, EngineSourceError> {
        EngineTrackSource::new(Arc::clone(&self.snapshot), track)
    }

    /// ⭐ 逐件渲染（逐件处理线期 3）：为一条带 `lane_chains` 的轨道武装
    /// 一个**分组源**（[`LaneSplitSource`]：调度表按键组切分、逐组引擎
    /// 渲染、逐组插入链与组声相、最后按组首键升序＋余量求和）。
    ///
    /// `bpm` 只用于 lane 链里乒乓延迟的音符同步换算（与轨链投影同一口径，
    /// 调用方传 `project.bpm`）。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：任一组引擎武装失败，或 SFZ 独占组被 lane
    /// 分组拆开（choke 双保险，见 [`LaneSplitSource::new`]）。
    pub fn lane_track_source(
        &self,
        track: EntityId,
        chains: &[LaneChainDef],
        bpm: f64,
    ) -> Result<LaneSplitSource, EngineSourceError> {
        LaneSplitSource::new(Arc::clone(&self.snapshot), track, chains, bpm)
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
    /// ⭐ `#341`：本轨的 **MIDI CC 探针**（`MidiCc` 泳道 ⇒ 逐块 `[u8; 128]`）。
    ///
    /// `None` ⇒ 走 `render_track`（无探针），与接线前**逐位相同**；
    /// `Some(_)` ⇒ 走 `render_track_with_cc`：SFZ 分支的 `loccN`/`hiccN` 门控与
    /// `*_onccN` 调制才有裁决依据。⚠ 只有 `with_cc` 会填它 —— 因此
    /// **没接 `with_cc` 的调用点，CC 泳道不生效**（这是有意的：探针要工程，
    /// 而 `new()` 只拿得到快照）。
    cc: Option<CcProbe>,
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
        //
        // ⭐⭐ **槽数 = 1**（不是 `DEFAULT_TRACK_SLOTS` = 1024）。
        //
        // 本源只渲染**一条**轨（下面 `arm_track_from_snapshot` 逐轨武装），1024 个槽里
        // 只有一个会用上，其余 1023 个是纯浪费 —— 而每个槽带着一台
        // `DrumMachine<16>`（≈ 23.9 KiB）＋ `PolySynth` ＋ `SfzVoicePool`，一整台默认
        // 引擎 ≈ 32 MiB。导出链**每条轨一个源** ⇒ 工程里有多少条轨就乘多少份：
        // 实测 79 轨工程（其中 71 条空轨）单次整曲渲染峰值 3.33 GB，
        // 而 7 条有声轨的同一份工程只要 1.05 GB —— 差的 2.28 GB 全是空轨那 71 台
        // 1024 槽引擎。`--export-stems` / `--export-song-overview-html` 要在**一个进程**
        // 里渲染 N+1 次，于是双双在 8 GB 的 cgroup 上限被 OOM 杀掉（exit=137）。
        //
        // 1 槽不会掉音：`arm_track_from_snapshot` 的契约就是"一个源一个槽"（见
        // `synth.rs` 该函数文档：单轨武装**正是**为了让槽位上限与工程规模解耦），
        // 而且 `begin_snapshot` 里 `tracks` 迭代器只有本轨一项 ⇒ `track_drops` 恒 0。
        let mut engine = Box::new(SynthEngine::with_track_slots(snapshot.sample_rate(), 1));
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
            // ⭐ `#341`：默认**没有** CC 探针 ⇒ `render_track`（逐位等于接线前）。
            cc: None,
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

    /// ⭐ `#341`：把本轨的 `MidiCc` 泳道接成**离线 CC 探针**。
    ///
    /// 没有 `MidiCc` 泳道 ⇒ 内部仍是 `None` ⇒ `render_block` 走 `render_track`
    /// ⇒ 输出与接线前**逐位相同**（判据 `cc_probe_is_bit_identical_when_unused`）。
    ///
    /// ⭐ set_ccN 离线播种（路线 a，推翻 `#112` 等待状态）：本轨在快照里有 SFZ
    /// 摊平表时，把摊平表自带的种子表（`PreparedSfz::cc_seeds()`，源头是文件
    /// `<control> set_ccN`）注入探针作**底层**，泳道覆于其上（泳道赢，见
    /// [`CcProbe`] 的种子层文档）。种子来源与 `render_block` 的 SFZ 分派读的是
    /// **同一张**快照表 ⇒ 不存在"探针的乐器"与"发声的乐器"不是同一份的窗口。
    /// 没有摊平表、或种子表为空 ⇒ 探针与接线前逐位相同。
    #[must_use]
    pub fn with_cc(mut self, project: &YebanProjectV1) -> Self {
        let sample_rate = self.snapshot.sample_rate();
        let mut probe = CcProbe::new(project, self.track, sample_rate);
        if let Some(plan) = self.snapshot.sfz().get(&self.track) {
            probe = probe.with_seeds(*plan.prepared().cc_seeds());
        }
        self.cc = (!probe.is_empty()).then_some(probe);
        self
    }

    /// ⭐ 逐件渲染（期 3）：把本源的调度表换成调用方给的**过滤副本**。
    ///
    /// [`LaneSplitSource`] 用它把整轨调度表按键组切开：每组一个源、各自
    /// 只留本组的音符。引擎本身不存调度表（`render_track` 每块从本源的
    /// `schedule` 字段读），因此换表即换发声集合，⛔ 不需要第二种武装。
    /// 音高形状旁表必须**整张**跟着走（下标语义，见 [`Self::seek_to`] 的
    /// 同一条纪律）—— 过滤由 [`filter_schedule`] 统一做，本方法只换表。
    #[must_use]
    pub fn with_schedule(mut self, schedule: Option<NoteSchedule>) -> Self {
        self.schedule = schedule;
        self
    }

    /// 被 CC 自动化的控制器个数（读数用；没接 `with_cc` 恒 `0`）。
    #[must_use]
    pub fn cc_controllers(&self) -> usize {
        self.cc.as_ref().map_or(0, CcProbe::controllers)
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

    /// 本源那台引擎的**轨道槽数**（构造期定死）。
    ///
    /// 回归判据的读点：它必须是 **`1`**，⛔ 不是 `DEFAULT_TRACK_SLOTS`（1024）——
    /// 见 [`Self::new`] 的内存注释（一个源只渲染一条轨，多出来的 1023 个槽
    /// 每个都带着一台 `DrumMachine<16>`，合计 ≈ 32 MiB **一条轨**的纯浪费）。
    #[must_use]
    pub fn track_slots(&self) -> usize {
        self.engine.track_slots()
    }

    /// 本源的调度表（加法访问器，无行为变化；供 MCP 插件源构造时复用——
    /// 插件源与引擎源读同一份调度表，不各自重算）。
    #[must_use]
    pub fn schedule(&self) -> Option<&NoteSchedule> {
        self.schedule.as_ref()
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
    /// ⭐ 第五份报告 B-1：冷启动定位。
    ///
    /// ⚠ 引擎自己的 `seek` 是**走带语义**：它复位声部并跳播放头，但
    /// 触发循环对"跨过定位点的音符"（`end_sample > position`）会
    /// **补触发**（跳进一个正被按住的音符要能听见它）—— 那是实时
    /// 播放要的，⛔ 不是区间渲染要的。故本函数在 `seek` 之外再把
    /// 本源的调度表换成只含 `start_sample >= frame` 的音符的副本：
    /// 跨起点的音符整体不在表里，三条支路（复音/SFZ/鼓机）都无从
    /// 补触发。音高形状旁表**整张保留**（音符的 `pitch_shape` 是
    /// 它的下标，压缩会让下标失效）；轨增益门与调律偏移照抄。
    ///
    /// 前提：定位发生在任何一块渲染之前（引擎槽位游标仍为 0，
    /// 与过滤后的表对齐）—— `RenderPlan::execute` 的调用点满足。
    fn seek_to(&mut self, frame: u64) -> Result<(), String> {
        self.engine.seek(frame);
        if let Some(schedule) = &self.schedule {
            let kept: Vec<yeban_engine::synth::ScheduledNote> = schedule
                .notes()
                .iter()
                .copied()
                .filter(|note| note.start_sample() >= frame)
                .collect();
            if kept.len() != schedule.notes().len() {
                self.schedule = Some(
                    NoteSchedule::from_sorted(kept)
                        .with_track_gain(schedule.track_gain())
                        .with_tune_cents(schedule.tune_cents())
                        .with_pitch_shapes(schedule.pitch_shapes().to_vec()),
                );
            }
        }
        Ok(())
    }

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
        // ⭐ `#341`：本块的 CC 表（`None` ＝ 本轨没有 CC 自动化 ⇒ 下面走无探针入口，
        //    与接线前逐位相同）。表是 `[u8; 128]` 的**按值副本** ⇒ 探针闭包只借它，
        //    不与 `self.engine` / `self.mono` 的借用相交。
        let cc_table = self
            .cc
            .as_mut()
            .and_then(|cc| cc.table_at(context.first_frame));
        // ⭐ **stage 2**：这张摊平表里有**非中性**区域 `pan` ⇒ 走分声道通路
        //    （`render_track_stereo`），源侧声相按区域施加；否则走原来的单声道通路
        //    （`pan` 全 0 的语料 ⇒ 与接线前逐位相同）。
        // ⚠ stage 3 起还要看**逐块可变**的声相：静态 `pan=0` ＋ `pan_onccN` 的库，
        //   只有在分声道通路上才听得见调制（同样条数为 0 时不切，逐位不变）。
        // ⭐ **立体声样本通路补门**：样本本身是多声道时，即使声相全中性也必须
        //   走分声道通路 —— 单声道通路只取声道 0，右声道会整段丢失（stage 2
        //   立门时样本全是单声道，这条前提随立体声样本通路落地已不成立）。
        let source_panned = channels == 2
            && sfz.is_some_and(|plan| {
                plan.regions_with_pan() > 0
                    || plan.regions_with_cc_pan() > 0
                    || plan.has_multichannel_samples()
            });
        if source_panned {
            // 栈上定长分块：⛔ 不在渲染中分配（与 `SfzVoicePool` 的分声道入口同一条纪律）。
            const CHUNK: usize = 128;
            let mut done = 0usize;
            while done < frames {
                let count = (frames - done).min(CHUNK);
                let mut left = [0.0f32; CHUNK];
                let mut right = [0.0f32; CHUNK];
                match cc_table {
                    Some(table) => {
                        let probe = |controller: u8| {
                            table.get(usize::from(controller)).copied().unwrap_or(0)
                        };
                        self.engine.render_track_stereo_with_cc(
                            self.track,
                            self.schedule.as_ref(),
                            sfz,
                            &mut left[..count],
                            &mut right[..count],
                            Some(&probe),
                        );
                    }
                    None => self.engine.render_track_stereo(
                        self.track,
                        self.schedule.as_ref(),
                        sfz,
                        &mut left[..count],
                        &mut right[..count],
                    ),
                }
                let (gain_l, gain_r) = self.pan;
                for index in 0..count {
                    let base = (done + index) * channels;
                    out[base] = left[index] * gain_l;
                    out[base + 1] = right[index] * gain_r;
                }
                done += count;
            }
        } else {
            match cc_table {
                Some(table) => {
                    let probe =
                        |controller: u8| table.get(usize::from(controller)).copied().unwrap_or(0);
                    self.engine.render_track_with_cc(
                        self.track,
                        self.schedule.as_ref(),
                        sfz,
                        &mut self.mono[..frames],
                        Some(&probe),
                    );
                }
                None => self.engine.render_track(
                    self.track,
                    self.schedule.as_ref(),
                    sfz,
                    &mut self.mono[..frames],
                ),
            }
        }
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
        if !source_panned {
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
        }
        Ok(())
    }
}

/// 按音高谓词过滤一张调度表（逐件渲染期 3 的切分核）。
///
/// 旁表纪律与 [`EngineTrackSource::seek_to`] 的过滤**同一条**：音高形状
/// 旁表整张保留（音符的 `pitch_shape` 是它的下标，压缩会让下标失效）；
/// 轨增益门与调律偏移照抄。过滤保序 ⇒ 结果仍满足 `from_sorted` 的前提。
fn filter_schedule(schedule: &NoteSchedule, keep: impl Fn(u8) -> bool) -> NoteSchedule {
    let kept: Vec<yeban_engine::synth::ScheduledNote> = schedule
        .notes()
        .iter()
        .copied()
        .filter(|note| keep(note.pitch()))
        .collect();
    NoteSchedule::from_sorted(kept)
        .with_track_gain(schedule.track_gain())
        .with_tune_cents(schedule.tune_cents())
        .with_pitch_shapes(schedule.pitch_shapes().to_vec())
}

/// 组声相的增益换算：与 MCP 轨声相**同一条**等功率（−3 dB）曲线
/// （`θ = (pan + 1) * π/4`，`(cos θ, sin θ)`），外层走
/// [`centered_pan_gains`] 的中心规则（`pan == 0` ⇒ `(1.0, 1.0)`，
/// ⛔ 不是 0.707）。
fn lane_pan_gains(pan: f32) -> (f32, f32) {
    centered_pan_gains(pan, |position| {
        let clamped = position.clamp(-1.0, 1.0);
        let angle = (clamped + 1.0) * core::f32::consts::FRAC_PI_4;
        (libm::cosf(angle), libm::sinf(angle))
    })
}

/// ⭐ **choke 双保险**（逐件渲染期 3）：SFZ 独占组不得被 lane 分组拆开。
///
/// 语义（`yeban-engine` `#94`）：区域触发时按自己的 `off_by` 去关断
/// 「`group == off_by` 的正在发声声部」。触发键与受害键若落在不同的
/// lane 分区（不同组、或一组在链内一组在余量），它们会被**不同的引擎
/// 实例**渲染，关断无从发生 ⇒ choke 静默失效。因此构造期按摊平表再核
/// 一遍（生成侧已核过一次，这是第二道）：任一独占关系的触发键集与
/// 受害键集只要有**一对**键分区不同，整源响亮失败、点名键与组号。
///
/// 合成鼓机侧没有跨音色 choke（各音色独立声部），无摊平表的轨不核。
fn check_choke_groups_not_split(
    prepared: &PreparedSfz,
    chains: &[LaneChainDef],
    track: EntityId,
) -> Result<(), EngineSourceError> {
    let assignment = |key: u8| chains.iter().position(|chain| chain.keys.contains(&key));
    let keys_of = |mask: [u64; 2]| -> Vec<u8> {
        let mut keys = Vec::new();
        for (word, bits) in mask.iter().enumerate() {
            for bit in 0..64 {
                if bits & (1u64 << bit) != 0 {
                    keys.push(u8::try_from(word * 64 + bit).unwrap_or(u8::MAX));
                }
            }
        }
        keys
    };
    for region in prepared.regions() {
        let off_by = region.off_by();
        if off_by == 0 {
            continue;
        }
        let trigger_keys = keys_of(region.key_mask());
        let mut victim_keys: Vec<u8> = Vec::new();
        for victim in prepared.regions() {
            if victim.group() == off_by {
                victim_keys.extend(keys_of(victim.key_mask()));
            }
        }
        if trigger_keys.is_empty() || victim_keys.is_empty() {
            continue;
        }
        for &trigger_key in &trigger_keys {
            for &victim_key in &victim_keys {
                if assignment(trigger_key) != assignment(victim_key) {
                    return Err(EngineSourceError::new(format!(
                        "逐件分组拆开了 SFZ 独占组: 轨道 {} 的区域以 off_by={off_by} 关断 \
                         group={off_by}，但触发键 {trigger_key} 与受害键 {victim_key} \
                         落在不同的 lane 分区（choke 会失效）；请把同一独占组的键放进同一条 lane 链",
                        track.to_canonical_string()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// [`LaneSplitSource`] 的一个 lane 组：过滤调度表的引擎源 ＋ 本组链的
/// 运行态（无可执行器件 ⇒ `None` ⇒ 整级不执行）＋ 组声相增益。
struct LaneGroup {
    /// 本组的引擎源（调度表只含本组键的音符；声相恒 `(1.0, 1.0)`，
    /// 轨声相与组声相都由 [`LaneSplitSource`] 在组外施加）。
    source: EngineTrackSource,
    /// 本组插入链运行态（由 `InsertChain::from_devices` 投影构造，
    /// 与轨链同一投影、同一运行态实现）。
    runtime: Option<InsertRuntime>,
    /// 组声相增益（`LaneChainDef::pan` 经 [`lane_pan_gains`] 换算；
    /// `None` = 预置未给声相 ⇒ 不乘）。
    pan: Option<(f32, f32)>,
    /// 本组过滤后的调度表是否有音符。`false` ⇒ 组链**整段不执行**
    /// （设计 §5.1 断言 1：无本组音符时组链不得改动任何样本）。
    has_notes: bool,
    /// 本组引擎的声部窃取句柄（构造期留存，渲染后读）。
    steals: Arc<AtomicU64>,
}

impl LaneGroup {
    /// 把 `MidiCc` 探针接到本组源上（与 [`LaneSplitSource::with_cc`] 同步）。
    fn with_cc(self, project: &YebanProjectV1) -> Self {
        Self {
            source: self.source.with_cc(project),
            ..self
        }
    }
}

/// ⭐ **逐件（逐 lane）分组源**（逐件处理线期 3，离线发声落点）。
///
/// 一条带 `lane_chains` 的轨在离线渲染里不再走单个 [`EngineTrackSource`]，
/// 而是走本源：整轨调度表按 lane 组的键集切成「组调度表 × N ＋ 余量
/// 调度表」，每份各武装一台 1 槽引擎（与 [`EngineTrackSource::new`]
/// 同款武装、同一份 `Arc` 快照，⛔ 不复制 PCM）；逐块渲染时按
/// **组首键升序（＝ `lane_chains` 的校验序）、余量最后**的固定顺序求和
/// 进节点缓冲（[ARCH-DET-002]：顺序固定 ⇒ 确定性）。每组在求和之前先过
/// 本组的插入链运行态（与轨链同一份 [`InsertRuntime`] 投影）与组声相；
/// 余量不加链、不加组声相。轨声相（[`Self::with_pan`]，调用方注入，
/// 与单源同一口径）在求和之后乘一次 —— 与单源路径里「源侧声相在轨链
/// 之前」的相对次序一致（轨链在 `render_node` 里跑，位于源之后）。
///
/// ## 契约与边界
///
/// * **无组音符 ⇒ 组链不执行**：某组过滤后的调度表为空时，它的引擎照常
///   推进（推进契约要求每块恰好一次），但链运行态整段跳过 ⇒ 该组的链
///   对本次渲染的任何样本**一位都不改**（设计 §5.1 断言 1 的落点）。
/// * **choke 双保险**：构造期核对 SFZ 独占组未被分组拆开，拆开响亮
///   失败（[`check_choke_groups_not_split`]）。
/// * **声部池按组隔离**（设计 R2 的已知行为差）：每组一台引擎 ⇒ SFZ
///   声部池按组各 16 槽，整轨可用声部数上升、窃取只会 ≤ 单源；
///   [`Self::steal_counter`] 报的是各组与余量的**总和**，与单源读数同口径。
/// * **lane 器件不可自动化**（设计 §2.5 既定边界）：`DeviceParam` 泳道
///   寻址是 (track, slot)，lane 器件不在此寻址空间 ⇒ 本源的链是静态的。
/// * **声道**：组链运行态只支持立体声（与轨链的
///   `RenderError::UnsupportedInsertChannels` 同一条）；非立体声渲染里
///   只要有任一组链可执行就响亮失败，⛔ 不静默跳过链。
pub struct LaneSplitSource {
    /// 本源负责的轨道。
    track: EntityId,
    /// lane 组（`lane_chains` 原序 ＝ 组首键升序，模型校验已钉）。
    groups: Vec<LaneGroup>,
    /// 余量源（不属于任何 lane 组的键；不加链）。
    remainder: EngineTrackSource,
    /// 余量源的声部窃取句柄。
    remainder_steals: Arc<AtomicU64>,
    /// 累加用的暂存缓冲（第一份直接写入 `out`，其余份先写这里再累加）。
    scratch: Vec<f32>,
    /// 轨声相（`(左, 右)`；默认 `(1.0, 1.0)` ＝ 居中，与单源同一缺省）。
    pan: (f32, f32),
    /// 全部子源窃取数之和的句柄（每块末尾由各子源句柄汇总写回）。
    steals: Arc<AtomicU64>,
}

impl std::fmt::Debug for LaneSplitSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaneSplitSource")
            .field("track", &self.track)
            .field("groups", &self.groups.len())
            .field("pan", &self.pan)
            .finish_non_exhaustive()
    }
}

impl LaneSplitSource {
    /// 从快照与该轨的 `lane_chains` 武装一个分组源。
    ///
    /// `bpm` 只用于 lane 链里乒乓延迟的音符同步换算（与轨链投影同口径）。
    ///
    /// # Errors
    ///
    /// [`EngineSourceError`]：任一子源武装失败，或 SFZ 独占组被分组
    /// 拆开（见 [`check_choke_groups_not_split`]）。
    pub fn new(
        snapshot: Arc<EngineSnapshot>,
        track: EntityId,
        chains: &[LaneChainDef],
        bpm: f64,
    ) -> Result<Self, EngineSourceError> {
        if let Some(plan) = snapshot.sfz().get(&track) {
            check_choke_groups_not_split(plan.prepared(), chains, track)?;
        }
        let sample_rate = snapshot.sample_rate();
        let schedule = snapshot.schedules().get(&track).cloned();
        let mut groups = Vec::with_capacity(chains.len());
        for chain in chains {
            let filtered = schedule
                .as_ref()
                .map(|full| filter_schedule(full, |pitch| chain.keys.contains(&pitch)));
            let has_notes = filtered
                .as_ref()
                .is_some_and(|table| !table.notes().is_empty());
            let source =
                EngineTrackSource::new(Arc::clone(&snapshot), track)?.with_schedule(filtered);
            let steals = source.steal_counter();
            let projection = InsertChain::from_devices(&chain.devices, sample_rate, bpm);
            let runtime = (!projection.is_empty())
                .then(|| InsertRuntime::new(&projection, sample_rate, L1_BLOCK_SIZE));
            groups.push(LaneGroup {
                source,
                runtime,
                pan: chain.pan.map(lane_pan_gains),
                has_notes,
                steals,
            });
        }
        let remainder_filtered = schedule.as_ref().map(|full| {
            filter_schedule(full, |pitch| {
                !chains.iter().any(|chain| chain.keys.contains(&pitch))
            })
        });
        let remainder =
            EngineTrackSource::new(Arc::clone(&snapshot), track)?.with_schedule(remainder_filtered);
        let remainder_steals = remainder.steal_counter();
        Ok(Self {
            track,
            groups,
            remainder,
            remainder_steals,
            scratch: Vec::new(),
            pan: (1.0, 1.0),
            steals: Arc::new(AtomicU64::new(0)),
        })
    }

    /// 写入**轨声相**（与 [`EngineTrackSource::with_pan`] 同一口径：
    /// 调用方给等功率增益对；缺省 `(1.0, 1.0)` ⇒ 与接线前逐位相同）。
    #[must_use]
    pub const fn with_pan(mut self, pan: (f32, f32)) -> Self {
        self.pan = pan;
        self
    }

    /// 把本轨的 `MidiCc` 泳道接成离线 CC 探针（逐组与余量各接一份
    /// 同内容的探针：探针按绝对帧查表，组间读数一致；没有 CC 泳道时
    /// 每份探针内部仍是 `None` ⇒ 与接线前逐位相同）。
    #[must_use]
    pub fn with_cc(self, project: &YebanProjectV1) -> Self {
        Self {
            groups: self
                .groups
                .into_iter()
                .map(|group| group.with_cc(project))
                .collect(),
            remainder: self.remainder.with_cc(project),
            ..self
        }
    }

    /// 本源负责的轨道 ID。
    #[must_use]
    pub const fn track(&self) -> EntityId {
        self.track
    }

    /// 全部子源的已排程音符数之和（＝ 整轨调度表的音符数）。
    #[must_use]
    pub fn scheduled_notes(&self) -> usize {
        self.groups
            .iter()
            .map(|group| group.source.scheduled_notes())
            .sum::<usize>()
            + self.remainder.scheduled_notes()
    }

    /// 声部窃取总数的句柄（各组 ＋ 余量之和；渲染后读，口径与
    /// [`EngineTrackSource::steal_counter`] 相同）。
    #[must_use]
    pub fn steal_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.steals)
    }

    /// 渲染一份（组或余量）到 `buf`，并**原地**过它的组链与组声相。
    ///
    /// `runtime` 为 `None`（余量、或组链投影为空）⇒ 只渲染不处理；
    /// `has_notes` 为假 ⇒ 链整段跳过（设计 §5.1 断言 1）。
    /// 组链按 [`L1_BLOCK_SIZE`] 帧分块喂运行态（运行态的内部暂存按
    /// 构造块长分配，块可以更小、⛔ 不能更大）。
    fn render_part(
        source: &mut EngineTrackSource,
        runtime: Option<&mut InsertRuntime>,
        pan: Option<(f32, f32)>,
        has_notes: bool,
        context: BlockContext,
        buf: &mut [f32],
    ) -> Result<(), RenderError> {
        source.render_block(context, buf)?;
        let channels = context.channels.max(1);
        let frames = context.frames.min(buf.len() / channels);
        if frames == 0 {
            return Ok(());
        }
        if has_notes && let Some(runtime) = runtime {
            let mut offset = 0usize;
            while offset < frames {
                let chunk = (frames - offset).min(L1_BLOCK_SIZE);
                runtime.process(&mut buf[offset * channels..(offset + chunk) * channels]);
                offset += chunk;
            }
        }
        if let Some((gain_l, gain_r)) = pan
            && (gain_l != 1.0 || gain_r != 1.0)
        {
            for frame in 0..frames {
                let base = frame * channels;
                buf[base] *= gain_l;
                if channels > 1 {
                    buf[base + 1] *= gain_r;
                }
            }
        }
        Ok(())
    }
}

impl AudioSource for LaneSplitSource {
    fn seek_to(&mut self, frame: u64) -> Result<(), String> {
        for group in &mut self.groups {
            group.source.seek_to(frame)?;
        }
        self.remainder.seek_to(frame)
    }

    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        let width = frames * channels;
        // 组链运行态只支持立体声（与轨链同一条响亮口径）：非立体声且
        // 任一组链可执行 ⇒ 指名失败，⛔ 不静默跳过链照常出声。
        if channels != 2 && self.groups.iter().any(|group| group.runtime.is_some()) {
            return Err(RenderError::UnsupportedInsertChannels(channels));
        }
        if self.scratch.len() < width {
            self.scratch.resize(width, 0.0);
        }
        // 固定求和顺序（[ARCH-DET-002]）：组按 `lane_chains` 序（组首键
        // 升序）、余量最后。第一份直接写进 `out`（子源的 `render_block`
        // 本来就整段覆写），其余份经暂存累加。
        let mut started = false;
        for group in &mut self.groups {
            if started {
                Self::render_part(
                    &mut group.source,
                    group.runtime.as_mut(),
                    group.pan,
                    group.has_notes,
                    context,
                    &mut self.scratch[..width],
                )?;
                for (slot, sample) in out[..width].iter_mut().zip(&self.scratch[..width]) {
                    *slot += *sample;
                }
            } else {
                Self::render_part(
                    &mut group.source,
                    group.runtime.as_mut(),
                    group.pan,
                    group.has_notes,
                    context,
                    &mut out[..width],
                )?;
                started = true;
            }
        }
        if started {
            self.remainder
                .render_block(context, &mut self.scratch[..width])?;
            for (slot, sample) in out[..width].iter_mut().zip(&self.scratch[..width]) {
                *slot += *sample;
            }
        } else {
            self.remainder.render_block(context, &mut out[..width])?;
        }
        // 轨声相在求和之后乘一次（与单源「源侧声相」的相对次序一致：
        // 都在轨链之前）。`(1.0, 1.0)` 时一个样本都不碰。
        let (gain_l, gain_r) = self.pan;
        if gain_l != 1.0 || gain_r != 1.0 {
            for frame in 0..frames {
                let base = frame * channels;
                out[base] *= gain_l;
                if channels > 1 {
                    out[base + 1] *= gain_r;
                }
            }
        }
        // 窃取读数汇总（只写一个 relaxed 原子，⛔ 不改音频）。
        let total: u64 = self
            .groups
            .iter()
            .map(|group| group.steals.load(Ordering::Relaxed))
            .sum::<u64>()
            + self.remainder_steals.load(Ordering::Relaxed);
        self.steals.store(total, Ordering::Relaxed);
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
            note_octave_override: None,
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
                    (Arc::new(vec![0.25_f32; 4_800]), 1, 48_000.0),
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

    /// ⭐ set_ccN 离线播种（路线 a）：`with_cc` 把武装乐器的种子表接进离线探针。
    ///
    /// 三条腿（同一单轨夹具：音高 48 的一个音符；区域 `key=48 locc1=64 hicc1=127`）：
    /// 1. SFZ 带 `<control>set_cc1=64` ⇒ 门控区域离线**发声**（峰值 > 0）；
    /// 2. 同一 SFZ 去掉 `set_cc` 行 ⇒ 门把区域挡住 ⇒ **峰值恰为 0**（`#112` 的旧行为）；
    /// 3. 带 `set_cc` ＋ 工程给 CC1 加一条值为 0 的 `MidiCc` 泳道 ⇒ **泳道赢** ⇒ 峰值恰为 0。
    ///
    /// 引擎侧零改动：三条腿走的都是既有 `render_track_with_cc` 入口，
    ///差别只在控制侧探针的组表（见 [`CcProbe`] 的种子层文档）。
    #[test]
    fn the_offline_probe_seeds_set_cc_gates_from_the_armed_instrument() {
        use std::str::FromStr as _;

        let dir = tempfile::tempdir().expect("临时目录");
        let sfz_path = dir.path().join("gated.sfz");

        let render = |sfz_text: &str, lane_zero: bool| -> f32 {
            std::fs::write(&sfz_path, sfz_text).expect("写 SFZ");
            let (mut project, ids) = many_track_project(1);
            let track = ids[0];
            project.tracks.get_mut(&track).expect("轨").sampler =
                Some(yeban_model::SamplerBinding {
                    note_octave_override: None,
                    path: sfz_path.display().to_string(),
                });
            if lane_zero {
                let target = yeban_model::AutomationTarget::MidiCc {
                    track_id: track,
                    controller: 1,
                };
                let point_id = EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P201").expect("ULID");
                let mut points = BTreeMap::new();
                points.insert(
                    point_id,
                    yeban_model::AutomationPoint {
                        id: point_id,
                        tick: 0,
                        value: 0.0,
                        curve: yeban_model::CurveType::Linear,
                    },
                );
                project
                    .tracks
                    .get_mut(&track)
                    .expect("轨")
                    .automation_lanes
                    .insert(
                        target,
                        yeban_model::AutomationLane {
                            target,
                            points,
                            read_enabled: true,
                            write_mode: yeban_model::AutomationWriteMode::Off,
                            domain: None,
                        },
                    );
            }
            project.validate().expect("播种夹具必须合法");
            let timbre = EngineTimbre::from_project_armed(&project, 48_000.0, |_, _| {
                ArmedSfzInput::new(
                    sfz_path.clone(),
                    vec![(
                        "tone.wav".to_owned(),
                        (Arc::new(vec![0.25_f32; 4_800]), 1, 48_000.0),
                    )],
                )
            })
            .expect("武装必须成功");
            let mut source = timbre
                .track_source(track)
                .expect("必须能武装")
                .with_cc(&project);
            render_peak(&mut source, 48_000)
        };

        let seeded = "<control>set_cc1=64\n<region> sample=tone.wav key=48 \
                      amp_veltrack=0 locc1=64 hicc1=127\n";
        let unseeded = "<region> sample=tone.wav key=48 amp_veltrack=0 locc1=64 hicc1=127\n";

        let peak_seeded = render(seeded, false);
        let peak_unseeded = render(unseeded, false);
        let peak_lane = render(seeded, true);
        println!(
            "### [setcc-seed] seeded={peak_seeded:.6} unseeded={peak_unseeded:.6} \
             lane0={peak_lane:.6}"
        );
        assert!(
            peak_seeded > 0.0,
            "set_cc1=64 必须让 locc1=64 的区域在离线渲染里发声"
        );
        assert_eq!(peak_unseeded, 0.0, "无 set_ccN 时门控区域必须仍被挡住");
        assert_eq!(peak_lane, 0.0, "MidiCc 泳道值 0 必须覆写种子 64（泳道赢）");
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
                    offset_samples: None,
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

    /// ⭐ **render 层选路证据**：单声道 SFZ 语料经**分声道通路**渲染，与经
    /// **单声道通路**渲染**逐位一致**（引擎层 S124 钉的是单声道出口对立体声
    /// 样本取声道 0；本条钉的是离线源在双声道出口上两条通路的合流）。
    ///
    /// 三个渲染（同一工程、同一音符、同一份单声道样本，只差 SFZ 文本）：
    /// 1. 纯区域（声相全中性、样本单声道）⇒ 选路走单声道通路；
    /// 2. 区域带 `pan_oncc10`（选路的 CC 声相一路命中）但不接 CC 探针
    ///    ⇒ 走分声道通路、有效声相仍为中性 ⇒ 必须与 1 **逐位相同**；
    /// 3. 同 2 的区域再加 `<control>set_cc10=127` 并接上离线探针 ⇒ 声相被
    ///    推满 +100 ⇒ 输出必须与 1 不同、左声道塌到近零 —— 这是 2 真的走了
    ///    分声道通路的**见证**（否则 1＝2 可能只是两边都走了单声道通路）。
    #[test]
    fn a_mono_sfz_corpus_renders_bit_identically_through_the_stereo_path() {
        fn render_stereo(source: &mut EngineTrackSource, frames: u64) -> Vec<f32> {
            let mut block = vec![0.0_f32; 256];
            let mut all = Vec::new();
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
                            channels: 2,
                            sample_rate: 48_000,
                            seed: 1,
                        },
                        &mut block[..len * 2],
                    )
                    .expect("连续升序的块必须被接受");
                all.extend_from_slice(&block[..len * 2]);
                first += len as u64;
                index += 1;
            }
            all
        }

        let dir = tempfile::tempdir().expect("临时目录");
        let sfz_path = dir.path().join("gate.sfz");
        let render = |sfz_text: &str, with_probe: bool| -> Vec<f32> {
            std::fs::write(&sfz_path, sfz_text).expect("写 SFZ");
            let (mut project, ids) = many_track_project(1);
            let track = ids[0];
            project.tracks.get_mut(&track).expect("轨").sampler =
                Some(yeban_model::SamplerBinding {
                    note_octave_override: None,
                    path: sfz_path.display().to_string(),
                });
            project.validate().expect("选路夹具必须合法");
            let pcm: Vec<f32> = (0..4_800)
                .map(|i| (((i * 37) % 401) as f32) / 401.0 - 0.5)
                .collect();
            let timbre = EngineTimbre::from_project_armed(&project, 48_000.0, |_, _| {
                ArmedSfzInput::new(
                    sfz_path.clone(),
                    vec![("tone.wav".to_owned(), (Arc::new(pcm.clone()), 1, 48_000.0))],
                )
            })
            .expect("武装必须成功");
            let mut source = timbre.track_source(track).expect("必须能武装");
            if with_probe {
                source = source.with_cc(&project);
            }
            render_stereo(&mut source, 4_800)
        };

        let plain = "<region> sample=tone.wav key=48 amp_veltrack=0\n";
        let gated = "<region> sample=tone.wav key=48 amp_veltrack=0 pan_oncc10=100\n";
        let seeded = "<control>set_cc10=127\n<region> sample=tone.wav key=48 amp_veltrack=0 pan_oncc10=100\n";

        let mono = render(plain, false);
        let stereo_neutral = render(gated, false);
        let bits = |buf: &[f32]| buf.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
        assert_eq!(
            bits(&stereo_neutral),
            bits(&mono),
            "⭐ 单声道语料经分声道通路（声相中性）必须与单声道通路逐位一致"
        );

        let panned = render(seeded, true);
        assert_ne!(
            bits(&panned),
            bits(&mono),
            "⭐ set_cc10=127 把 pan_oncc 推满后输出必须改变（分声道通路的见证）"
        );
        let left_peak = panned
            .iter()
            .step_by(2)
            .fold(0.0_f32, |acc, v| acc.max(v.abs()));
        let right_peak = panned
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0.0_f32, |acc, v| acc.max(v.abs()));
        assert!(
            right_peak > 0.1,
            "⭐ 声相推满后右声道必须带满信号（实得峰值 {right_peak}）"
        );
        assert!(
            left_peak <= right_peak * 1e-6,
            "⭐ 声相推满 +100 后左声道必须塌到近零（实得峰值 {left_peak}，右 {right_peak}）"
        );
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

    /// **实测缺陷回归（G1）**：工程源数超过实时槽位上限（`MAX_TRACK_SLOTS` = 2048）时，
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
        // 槽位容量现已可配（P2），默认 1024 条轨装得下 ⇒ 这里显式用 16 槽建引擎，
        // 牙齿咬的仍是“超容武装丢轨、丢轨静音”这条语义，不随默认值漂移。
        let mut engine = Box::new(SynthEngine::with_track_slots(snapshot.sample_rate(), 16));
        engine
            .arm_from_snapshot(&snapshot)
            .expect("全量武装本身必须成功（它只是丢槽）");
        assert!(
            engine.track_drops() > 0,
            "{TRACKS} 条轨全量武装必须触发槽位耗尽计数（否则这条牙齿没咬住）"
        );
        let mut legacy = EngineTrackSource {
            cc: None,
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

    /// ⭐⭐ **内存判据**：一个离线源只武装**一个**轨道槽。
    ///
    /// # 量什么
    ///
    /// `EngineTrackSource::new` 建的那台 `SynthEngine` 的 `track_slots()`。修前是
    /// `DEFAULT_TRACK_SLOTS` = 1024，修后是 1。
    ///
    /// # 为什么这条判据值钱（实测读数）
    ///
    /// 每个槽带着一台 `DrumMachine<16>`（≈ 23.9 KiB）＋ `PolySynth` ＋ `SfzVoicePool`
    /// ⇒ 一整台默认引擎 ≈ **32 MiB**。导出链**每条轨一个源**，于是"轨数 × 32 MiB"：
    ///
    /// | 工程 | 非 master 节点 | 单次整曲（266 s）渲染峰值 |
    /// |:--|--:|--:|
    /// | 79 轨（7 条有声） | 78 | 3.33 GB（修前）→ **841 MB**（修后） |
    /// | 同工程剪掉 71 条空轨 | 7 | 1.05 GB |
    ///
    /// 修前 `--export-stems` / `--export-song-overview-html`（一个进程里渲染 N+1 次）
    /// 双双在 8 GB cgroup 上限被 OOM 杀（`exit=137`，无 traceback）。
    ///
    /// # 怎么变红
    ///
    /// 把 `new` 里的 `with_track_slots(_, 1)` 换回 `SynthEngine::new(_)`
    /// ⇒ `track_slots() == 1024` ⇒ 本条红。
    #[test]
    fn an_offline_source_arms_exactly_one_track_slot() {
        let project = yeban_model::samples::demo_project();
        let (timbre, track) = source(&project);
        let source = timbre.track_source(track).expect("必须能武装");
        assert_eq!(
            source.track_slots(),
            1,
            "离线源只渲染一条轨 ⇒ 只该占一个槽（每多一个槽 ≈ 32 KiB 的鼓机 + 声部池）"
        );
    }

    // -----------------------------------------------------------------
    // ⭐ 逐件渲染（期 3）判据组：设计 §5.1 的离线验收落到源级。
    // -----------------------------------------------------------------

    /// 逐件夹具的通道条设备（参数名与轨链同一张拼写表）。
    fn lane_strip(params: &[(&str, f32)]) -> yeban_model::DeviceDefinition {
        yeban_model::DeviceDefinition {
            id: EntityId::new(),
            name: "Lane Strip".to_owned(),
            kind: yeban_model::DeviceKind::InternalEffect,
            bypassed: false,
            params: params
                .iter()
                .map(|(name, value)| yeban_model::ParameterValue {
                    name: (*name).to_owned(),
                    value: *value,
                    unit: None,
                })
                .collect(),
            latency_samples: 0,
            plugin: None,
        }
    }

    /// 单轨多音符夹具：音符由 `(start_tick, pitch, duration_ticks)` 给定，
    /// 可选给该轨挂逐件链。工程 120 BPM / 960 PPQ / 48 kHz ⇒ 1 tick = 25 帧。
    fn lane_project(
        notes: &[(u64, u8, u64)],
        chains: Vec<LaneChainDef>,
    ) -> (YebanProjectV1, EntityId) {
        use std::str::FromStr as _;

        use yeban_model::{
            ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, MidiNote, RoutingEdge,
            RoutingKind, SampleRate, TrackKind, TrackV3,
        };

        let id = |seed: usize| {
            EntityId::from_str(&format!("01J8ZN{seed:020}")).expect("夹具 ULID 必须合法")
        };
        let master = id(1);
        let track = id(100);
        let clip = id(1_000);
        let placement = id(2_000);
        let edge = id(4_000);
        let mut project = YebanProjectV1 {
            title: "lane fixture".to_owned(),
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
        let mut track_params = TrackV3 {
            id: track,
            name: "Drums".to_owned(),
            kind: TrackKind::Midi,
            lane_chains: chains,
            ..TrackV3::default()
        };
        track_params.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: 0,
                duration_ticks: 3_840,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
            },
        );
        project.tracks.insert(track, track_params);
        let mut note_map = BTreeMap::new();
        for (index, &(start, pitch, duration)) in notes.iter().enumerate() {
            let note = id(3_000 + index);
            note_map.insert(note, MidiNote::new(note, start, pitch, duration));
        }
        project.clip_pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: "Clip".to_owned(),
                content: ClipContent::Midi { notes: note_map },
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
        project
            .validate()
            .unwrap_or_else(|error| panic!("逐件夹具必须合法: {error}"));
        (project, track)
    }

    /// 把任意源按 128 帧立体声块渲染 `frames` 帧，返回交错样本。
    fn render_stereo_any(source: &mut impl AudioSource, frames: u64) -> Vec<f32> {
        let mut block = vec![0.0_f32; 256];
        let mut all = Vec::new();
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
                        channels: 2,
                        sample_rate: 48_000,
                        seed: 1,
                    },
                    &mut block[..len * 2],
                )
                .expect("连续升序的块必须被接受");
            all.extend_from_slice(&block[..len * 2]);
            first += len as u64;
            index += 1;
        }
        all
    }

    /// 某帧区间、某声道（`None` = 两声道合并）的 RMS（线性）。
    fn segment_rms(samples: &[f32], frames: std::ops::Range<usize>, channel: Option<usize>) -> f32 {
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for frame in frames {
            for ch in 0..2 {
                if channel.is_some_and(|wanted| wanted != ch) {
                    continue;
                }
                let value = f64::from(samples[frame * 2 + ch]);
                sum += value * value;
                count += 1;
            }
        }
        if count == 0 {
            return 0.0;
        }
        (sum / count as f64).sqrt() as f32
    }

    fn db(ratio: f32) -> f32 {
        20.0 * ratio.max(1.0e-9).log10()
    }

    /// 三件套音符：kick(36) @0、hat(42) @960、snare(38) @1920（tick）。
    const KIT_NOTES: [(u64, u8, u64); 3] = [(0, 36, 240), (960, 42, 240), (1_920, 38, 240)];
    /// 三个音符的测量窗（帧）：起音后 +500 .. +5000（避开起音瞬态与尾段）。
    const KICK_WIN: std::ops::Range<usize> = 500..5_000;
    const HAT_WIN: std::ops::Range<usize> = 24_500..29_000;
    const SNARE_WIN: std::ops::Range<usize> = 48_500..53_000;

    /// §5.1 断言 1（逐位隔离）：只有 kick 音符的工程挂一条**可执行的**
    /// hats 组链，分组渲染必须与单源渲染**逐位相同**——无 hat 音符 ⇒
    /// hat 组链整段不执行；若实现把组链跑在求和后的缓冲上（或空组也跑
    /// 链并留下状态痕迹），kick 会被 −48 dB 压掉、本条立即变红。
    /// 对称地再测只有 snare 音符的一份。
    #[test]
    fn a_lane_chain_for_an_absent_key_group_changes_not_one_bit() {
        let chains = vec![LaneChainDef {
            keys: vec![42, 46],
            name: "hats".to_owned(),
            devices: vec![lane_strip(&[("output_gain_db", -48.0)])],
            pan: None,
        }];
        for notes in [&[(0u64, 36u8, 240u64)][..], &[(0, 38, 240)][..]] {
            let (project, track) = lane_project(notes, chains.clone());
            let timbre = EngineTimbre::from_project(&project).expect("夹具必须成快照");
            let mut plain = timbre.track_source(track).expect("必须能武装");
            let plain_out = render_stereo_any(&mut plain, 24_000);
            let mut lane = timbre
                .lane_track_source(track, &project.tracks[&track].lane_chains, project.bpm)
                .expect("分组源必须能武装");
            let lane_out = render_stereo_any(&mut lane, 24_000);
            assert!(
                segment_rms(&plain_out, KICK_WIN, None) > 0.01,
                "夹具音符必须真的出声（否则逐位相同是空判据）"
            );
            assert_eq!(
                RenderOutput::digest_of(&lane_out),
                RenderOutput::digest_of(&plain_out),
                "无本组音符时组链不得改动任何一位（音高集 {notes:?}）"
            );
        }
    }

    /// §5.1 断言 2（隔离量级）：hats 组链 −48 dB 只许压低 hat 段；
    /// kick 段与 snare 段的 RMS 差必须在容差内（链间不许串扰）。
    /// 容差逐字取设计 §5.1：kick 段 ≤ 0.1 dB、snare 段 ≤ 0.25 dB；
    /// hat 段以 −48 dB 增益链断言压低 ≥ 20 dB（强于设计的 HPF 频带
    /// ≥ 10 dB 口径——源级夹具是时段隔离的合成音色，用时段 RMS
    /// 代频带能量，量法差异已在期 3 回执登记）。
    #[test]
    fn a_lane_chain_gain_hits_only_its_own_key_group() {
        let chains = vec![LaneChainDef {
            keys: vec![42, 46],
            name: "hats".to_owned(),
            devices: vec![lane_strip(&[("output_gain_db", -48.0)])],
            pan: None,
        }];
        let (project, track) = lane_project(&KIT_NOTES, chains);
        let timbre = EngineTimbre::from_project(&project).expect("夹具必须成快照");
        let mut plain = timbre.track_source(track).expect("必须能武装");
        let plain_out = render_stereo_any(&mut plain, 72_000);
        let mut lane = timbre
            .lane_track_source(track, &project.tracks[&track].lane_chains, project.bpm)
            .expect("分组源必须能武装");
        let lane_out = render_stereo_any(&mut lane, 72_000);

        let hat_drop = db(segment_rms(&plain_out, HAT_WIN.clone(), None))
            - db(segment_rms(&lane_out, HAT_WIN.clone(), None));
        let kick_diff = (db(segment_rms(&plain_out, KICK_WIN, None))
            - db(segment_rms(&lane_out, KICK_WIN, None)))
        .abs();
        let snare_diff = (db(segment_rms(&plain_out, SNARE_WIN, None))
            - db(segment_rms(&lane_out, SNARE_WIN, None)))
        .abs();
        println!(
            "### [lane-gain] hat_drop={hat_drop:.2} dB kick_diff={kick_diff:.3} dB \
             snare_diff={snare_diff:.3} dB"
        );
        assert!(
            segment_rms(&plain_out, HAT_WIN, None) > 0.01,
            "hat 段基线必须出声"
        );
        assert!(
            hat_drop >= 20.0,
            "hat 段必须被组链压低 ≥ 20 dB（实测 {hat_drop:.2}）"
        );
        assert!(
            kick_diff <= 0.1,
            "kick 段不得被 hat 组链影响（实测差 {kick_diff:.3} dB）"
        );
        assert!(
            snare_diff <= 0.25,
            "snare 段不得被 hat 组链影响（实测差 {snare_diff:.3} dB）"
        );
    }

    /// §5.1 断言 3（组声相）：hats 组 pan = −1（硬左）⇒ hat 段右声道塌陷、
    /// 左声道保留；kick 段左右平衡不动。
    #[test]
    fn a_lane_group_pan_moves_only_its_own_group() {
        let chains = vec![LaneChainDef {
            keys: vec![42, 46],
            name: "hats".to_owned(),
            devices: Vec::new(),
            pan: Some(-1.0),
        }];
        let (project, track) = lane_project(&KIT_NOTES, chains);
        let timbre = EngineTimbre::from_project(&project).expect("夹具必须成快照");
        let mut lane = timbre
            .lane_track_source(track, &project.tracks[&track].lane_chains, project.bpm)
            .expect("分组源必须能武装");
        let out = render_stereo_any(&mut lane, 72_000);
        let hat_l = segment_rms(&out, HAT_WIN, Some(0));
        let hat_r = segment_rms(&out, HAT_WIN, Some(1));
        let kick_l = segment_rms(&out, KICK_WIN, Some(0));
        let kick_r = segment_rms(&out, KICK_WIN, Some(1));
        println!("### [lane-pan] hat L={hat_l:.5} R={hat_r:.6} kick L={kick_l:.5} R={kick_r:.5}");
        assert!(hat_l > 0.01, "hat 段左声道必须保留（实测 {hat_l:.5}）");
        assert!(
            hat_r <= hat_l * 0.01,
            "hat 段右声道必须塌陷到左的 1% 以下（实测 R={hat_r:.6} L={hat_l:.5}）"
        );
        assert!(
            (db(kick_l) - db(kick_r)).abs() <= 0.5,
            "kick 段声像不得被 hat 组声相移动（L/R 差 {:.3} dB）",
            (db(kick_l) - db(kick_r)).abs()
        );
    }

    /// §5.1 断言 5（确定性）：同一工程两份分组源渲染摘要一致。
    #[test]
    fn lane_split_rendering_is_deterministic() {
        let chains = vec![LaneChainDef {
            keys: vec![42, 46],
            name: "hats".to_owned(),
            devices: vec![lane_strip(&[("output_gain_db", -6.0), ("hpf_hz", 400.0)])],
            pan: Some(-0.3),
        }];
        let (project, track) = lane_project(&KIT_NOTES, chains);
        let timbre = EngineTimbre::from_project(&project).expect("夹具必须成快照");
        let render = || {
            let mut lane = timbre
                .lane_track_source(track, &project.tracks[&track].lane_chains, project.bpm)
                .expect("分组源必须能武装");
            RenderOutput::digest_of(&render_stereo_any(&mut lane, 72_000))
        };
        assert_eq!(render(), render(), "分组渲染必须逐位可复现");
    }

    /// §5.1 断言 4（choke 回归）＋拆组响亮失败（双保险）：
    /// 开镲（46, group=1）长音后接闭镲（42, off_by=1）——hats 同组
    /// [42,46] 时分组渲染的掐尾时刻与单源渲染差 ≤ 1 块；把 42/46 拆到
    /// 不同分区时，分组源构造必须响亮失败。
    #[test]
    fn choke_survives_lane_grouping_and_split_groups_fail_loudly() {
        let dir = tempfile::tempdir().expect("临时目录");
        let sfz_path = dir.path().join("kit.sfz");
        let sfz_text = "<region> sample=open.wav key=46 group=1 amp_veltrack=0\n\
                        <region> sample=closed.wav key=42 group=2 off_by=1 amp_veltrack=0\n";
        std::fs::write(&sfz_path, sfz_text).expect("写 SFZ");
        let notes = [(0u64, 46u8, 1_920u64), (960, 42, 120)];
        let hats_chain = |keys: Vec<u8>| LaneChainDef {
            keys,
            name: "hats".to_owned(),
            devices: vec![lane_strip(&[("output_gain_db", -6.0)])],
            pan: None,
        };
        let build = |chains: Vec<LaneChainDef>| {
            let (mut project, track) = lane_project(&notes, chains);
            project.tracks.get_mut(&track).expect("轨").sampler =
                Some(yeban_model::SamplerBinding {
                    note_octave_override: None,
                    path: sfz_path.display().to_string(),
                });
            project.validate().expect("choke 夹具必须合法");
            let timbre = EngineTimbre::from_project_armed(&project, 48_000.0, |_, _| {
                ArmedSfzInput::new(
                    sfz_path.clone(),
                    vec![
                        (
                            "open.wav".to_owned(),
                            (Arc::new(vec![0.4_f32; 96_000]), 1, 48_000.0),
                        ),
                        (
                            "closed.wav".to_owned(),
                            (Arc::new(vec![0.3_f32; 4_800]), 1, 48_000.0),
                        ),
                    ],
                )
            })
            .expect("武装必须成功");
            (project, track, timbre)
        };
        let last_audible = |samples: &[f32]| -> usize {
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .rposition(|frame| frame[0].abs() > 0.05 || frame[1].abs() > 0.05)
                .unwrap_or(0)
        };

        // 单源基线与同组分组渲染的掐尾时刻对账。
        let (project, track, timbre) = build(vec![hats_chain(vec![42, 46])]);
        let mut plain = timbre.track_source(track).expect("必须能武装");
        let plain_out = render_stereo_any(&mut plain, 72_000);
        let mut lane = timbre
            .lane_track_source(track, &project.tracks[&track].lane_chains, project.bpm)
            .expect("同组分组源必须能武装");
        let lane_out = render_stereo_any(&mut lane, 72_000);
        let plain_last = last_audible(&plain_out);
        let lane_last = last_audible(&lane_out);
        println!("### [lane-choke] plain_last={plain_last} lane_last={lane_last}");
        assert!(
            plain_last.abs_diff(lane_last) <= 128,
            "choke 掐尾时刻差不得超过 1 块（单源 {plain_last} vs 分组 {lane_last}）"
        );
        // 非空证明：没有闭镲时开镲长音必须响得久得多（掐尾真的发生过）。
        let (mut open_project, open_track) = lane_project(&[(0, 46, 1_920)], Vec::new());
        open_project
            .tracks
            .get_mut(&open_track)
            .expect("轨")
            .sampler = Some(yeban_model::SamplerBinding {
            note_octave_override: None,
            path: sfz_path.display().to_string(),
        });
        open_project.validate().expect("开镲夹具必须合法");
        let open_timbre = EngineTimbre::from_project_armed(&open_project, 48_000.0, |_, _| {
            ArmedSfzInput::new(
                sfz_path.clone(),
                vec![
                    (
                        "open.wav".to_owned(),
                        (Arc::new(vec![0.4_f32; 96_000]), 1, 48_000.0),
                    ),
                    (
                        "closed.wav".to_owned(),
                        (Arc::new(vec![0.3_f32; 4_800]), 1, 48_000.0),
                    ),
                ],
            )
        })
        .expect("武装必须成功");
        let mut open_source = open_timbre.track_source(open_track).expect("必须能武装");
        let open_out = render_stereo_any(&mut open_source, 72_000);
        let open_last = last_audible(&open_out);
        println!("### [lane-choke] open_last={open_last}");
        assert!(
            open_last > plain_last + 8_000,
            "无闭镲时开镲必须多响至少 8000 帧（open {open_last} vs choked {plain_last}）"
        );
        // 窃取对账（§2.5）：两边都不得发生声部窃取。
        assert_eq!(plain.steal_counter().load(Ordering::Relaxed), 0);
        assert_eq!(lane.steal_counter().load(Ordering::Relaxed), 0);

        // 拆组 ⇒ 构造响亮失败（触发键 42 在组内、受害键 46 在余量）。
        let (_project2, track2, timbre2) = build(vec![hats_chain(vec![42])]);
        let error = timbre2
            .lane_track_source(track2, &[hats_chain(vec![42])], 120.0)
            .expect_err("拆开独占组的分组必须失败");
        assert!(
            error.message().contains("off_by=1"),
            "失败报文必须点名独占组: {}",
            error.message()
        );
    }
}
