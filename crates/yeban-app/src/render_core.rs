//! 渲染核心（**新增项的独立小模块**）：把工程渲染成交错 `f32` 样本。
//!
//! ## 为什么单独一个文件
//!
//! ⚠ 本会话的机械教训：把新增的公开项**追加在 `#[cfg(test)] mod tests` 之后**会触发
//! `clippy::items_after_test_module`（已咬过两次）。把新增项放进**独立模块**可以一次性避开该形态。
//!
//! ## ⭐ 插入链（任务单 `#244`）：导出与 MCP 走**同一条**生产装配
//!
//! 修前本节目的计划编译是 `RenderPlan::compile(...)` —— 它内部把**延迟表与插入表都传空**
//! （`crates/yeban-render/src/render.rs:439`）⇒ `TrackV3.devices` 里的 EQ / Freeverb / 卷积
//! 一个都不进导出（`#226`／`#240` 已登记的口径）。修法**不是**在这里写一份"设备链 → 插入链"
//! 的转换，而是复用两条既有设施：
//!
//! - [`track_latencies`]：`DeviceDefinition::latency_samples` 的唯一累加处（[ARCH-PDC-001]，
//!   旁通设备不计入）；
//! - [`project_inserts_with_forced_strips`]：设备链 → 插入链的唯一转换器（`yeban-render/tests/insert_chain.rs`
//!   与 MCP 的 `domain/render.rs` 都在用它）。
//!
//! ⇒ 计划编译改调 `RenderPlan::compile_with_latencies_and_inserts`。无设备的工程
//! （两张表都为空）逐字节不变；有设备的工程才会变（本单的判据在
//! `tests/export_wav_inserts.rs`）。
//!
//! ## 与 `export_wav.rs` 的关系（**去重已完成**）
//!
//! 本函数是导出与录音的**单一真源**（同一批步骤：慢路径校验 → 计划编译 → 音源装配 → 执行 →
//! digest）。`export_wav.rs` 的四个入口 `export_wav` / `export_wav_with_sfz` /
//! `export_wav_with_sfz_file` / `export_wav_with_sfz_file_and_assets` 逐级收束，末端那个
//! **已改为调用本模块的** [`render_project_f32_with_assets`]（`export_wav.rs:414-417`）；
//! `record_wav::record_project_under` 也走同一条 [`render_project_f32_with_assets_under`]
//! （`record_wav.rs` 的 `record_project` 委托 `project_dir = None` ⇒ 与 `#168` 之前逐位相同）。
//! ⇒ ⭐ "真正的去重（让导出也调用本函数）"**已经落地**，不再是"下一步"。
//! 判据 `recording_the_project_is_bit_identical_to_exporting_it` 仍是这条"两侧逐位一致"契约的
//! **回归网**：一旦漂移，该判据立刻红 ✓
//!
//! ## ⭐ 模型 SFZ 链条（任务单 `#79`）与 `--sfz` 旁路的关系
//!
//! | 入口 | 绑定范围 | 走哪条音源 | 谁读盘 |
//! | :--- | :--- | :--- | :--- |
//! | **`TrackV3.sampler`**（模型链条，本单接线） | **按轨** | 引擎 SFZ 分支（`render_track` 第 3 参 = 快照里的摊平表） | 本模块的构造期（[`crate::sfz_load::arm_pending_sfz`]） |
//! | **`--sfz <文件>`**（旁路，保留） | **整工程** | [`crate::sfz_load::SfzTrackSource`]（整个替换引擎音源） | `sfz_load` 的 SFZ 源构造期 |
//!
//! 两者**同时给** ⇒ **旁路优先**（"整工程用这一份 SFZ 试听"是显式请求，本单⛔ 不改它的语义）；
//! `plans` 注入与模型链条都只在**用引擎音源**时生效（见 [`render_project_f32_with_plans`]）。
//! ⭐ 两条入口都**不**合并：旁路是"整工程试听一份 SFZ"的便捷路径，模型链条是"按轨绑定"的
//! 产品语义（模型侧形状见 `TrackV3.sampler`，登记在 `R731`）。
//!
//! ⚠ **SFZ 文件缺失 ⇒ 明确错误**（[`WavExportError::Sfz`]），缺**样本** ⇒ 该区域静音 ＋ 计数
//! （引擎口径；见 `sfz_load` 与台账 `R711`）—— 两者都**不静默**。
//!
//! ## ⭐ 轨增益（推子／静音／独奏）：三条支路**同径**（本单闭合 `R713` 登记的边界）
//!
//! | 支路 | 音源 | 轨增益从哪来 |
//! | :--- | :--- | :--- |
//! | 引擎乐器（默认） | [`EngineTrackSource`] → `SynthEngine::render_track` | ⭐ 引擎**内部**：鼓机读 `ScheduledNote::gain`、复音读同一个 `note.gain`、SFZ 声部读 `NoteSchedule::track_gain` —— 三者都来自 `snapshot::project_schedules` 里**同一次** [`yeban_engine::synth::track_gain_scale`] |
//! | **SFZ 旁路**（`--sfz`） | [`crate::sfz_load::SfzTrackSource`] | ⭐ **本模块**在构造期逐轨调**同一个** [`track_gain_of`]（⇒ 同一个 `track_gain_scale`）再经 `with_track_gain` 注入 |
//! | 最小音源（`use_engine_source = false`） | [`NoteToneSource`] | ⭐ **本模块**在构造期逐轨调**同一个** [`track_gain_of`]（⇒ 同一个 `track_gain_scale`）再经 `with_track_gain` 注入（与 SFZ 旁路**同形**） |
//!
//! ⚠ **为什么两条旁路原先吃不到**：SFZ 与最小音源都**整个替换**引擎音源，且本模块在带 SFZ 时
//! **不建引擎快照**（上面的 `snapshot` 判据）⇒ 那份"投影期已乘好的"轨增益不存在。
//! 修法**不是**在这里写一份 `volume_db ⇒ 线性`（那会与引擎分叉），而是调引擎的
//! **同一个**算子：两条旁路都在构造期经 `with_track_gain(track_gain_of(...))` 注入。
//!
//! ⭐ **缺口已闭合**（本单落地 `#69` 登记的边界）：最小音源 [`NoteToneSource`] 现在也吃轨增益
//! —— 此前它只作判据对照与极端回退（产品路径 `--export-wav` / `record_wav` 都给
//! `use_engine_source = true` ⇒ ⛔ 产品字节一位没变）。⚠ 唯一受影响的是 `export_wav.rs` 的
//! `the_engine_source_is_not_silent_and_differs_from_the_fallback_tone`：它只断言"两版**逐字节
//! 不同**"＋来源文案，**不钉**回退版的字节；夹具那条 MIDI 轨 `volume_db = -3.0`
//! ⇒ 回退版整体乘 `track_gain(-3.0)`，而**没有一条断言期望值**因此改变。
//!
//! ⚠ **音频片段**（[`crate::audio_clip`]）另有一套自己的"片段增益 ＋ 轨音量 ＋ 声相"
//! 组合（`audio_clip.rs:282` 的 `db_to_linear(clip_gain_db + track_volume_db)`）——
//! 它**不是** `track_gain_scale`，本单如实登记，⛔ 不在此处改（会动既有音频片段判据）。
//!
//! ## ⚠ 红线 7 与**调用方约束**
//!
//! 本函数**允许分配**（解析 SFZ／装载快照／建缓冲）⇒ ⭐ **两侧调用方（导出与录音）都必须是
//! 离线／控制侧**：`--export-wav` 是离线批处理，`record_wav::record_project` 只在**喂环之前**
//! 调它。⛔ **绝不可**在音频回调里调用；录音侧只有 `push_frame` 那一段是"禁止分配／加锁／I/O"的。
//!
//! # Errors
//!
//! 与 [`crate::export_wav::export_wav`] 相同（选项非法 / 计划编译 / 引擎装载 / 执行失败）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use yeban_model::{AssetHash, EntityId, YebanProjectV1};
use yeban_render::automation::{
    TrackGainAutomation, TrackPanAutomation, device_param_automations, forced_insert_instances,
    macro_cascade_reaches, pan_automations, send_gain_automations,
};
use yeban_render::inserts::project_inserts_with_forced_instances;
use yeban_render::render::{
    AudioSource, BlockContext, OutputStage, RenderError, RenderOptions, RenderOutput, RenderPlan,
    db_to_linear, track_latencies,
};

use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::SynthEngine;

use crate::audio_clip;
use crate::export_wav::{NoteToneSource, WavExportError, WavExportOptions, track_notes_in_frames};
// ⭐ `ADR-0011`：引擎音色源已搬到 `yeban-render`；与 MCP 共用同一份实现。
use yeban_render::engine_source::EngineTrackSource;

/// 一条轨的复合源：**先 MIDI 基座、后音频片段**（顺序与 MCP 的 `TrackSource` 相同，
/// 且与块边界无关 ⇒ 逐位可复现）。
struct TrackSource {
    /// 既有音源（引擎乐器 / SFZ / 最小音源）。
    base: Box<dyn AudioSource>,
    /// 音频片段源（按资产哈希升序，每个素材一个）。
    clips: Vec<audio_clip::AudioClipSource>,
}

impl AudioSource for TrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // 基座会清零（三个既有源都 `out.fill(0.0)`）；音频片段只**累加**。
        self.base.render_block(context, out)?;
        for clip in &mut self.clips {
            clip.render_block(context, out)?;
        }
        Ok(())
    }
}

/// ⭐ 本条任务的**音量自动化门**：在源输出之后乘 R55 比率（含 MIDI 基座与音频片段）。
///
/// 与 MCP 的 `TrackSource` 里的 `automation` 字段**同一份**实现
/// （[`yeban_render::automation::TrackGainAutomation`]）：块级求值 + 逐样本 5 ms 平滑。
/// 只对**非恒等**的轨包这一层；恒等的轨直接返回原源，一个样本都不碰。
struct AutomationGain {
    /// 既有音源（引擎乐器 / SFZ / 最小音源 ＋ 音频片段）。
    inner: Box<dyn AudioSource>,
    /// 本轨的 `TrackVolume` 自动化（构造期已排除恒等快路径）。
    automation: TrackGainAutomation,
}

impl AudioSource for AutomationGain {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        self.inner.render_block(context, out)?;
        self.automation.apply_ratio_block(
            out,
            context.first_frame,
            context.frames,
            context.channels,
        );
        Ok(())
    }
}

/// ⭐ 任务单 `#257`：**声相自动化门** —— 在源输出之后乘自动化位置对应的绝对左右增益。
///
/// 与 MCP 的 `TrackSource::pan_automation` **同一份**实现
/// （[`yeban_render::automation::TrackPanAutomation`]）：块级求值 + 逐样本 5 ms
/// 平滑，曲线由 [`yeban_engine::mixer::pan_gains`] 给出（与静态声相**同一条**：
/// `crate::audio_clip::track_pan_gains` 内部就是它）。只对**非恒等**的轨包这一层；
/// 恒等的轨源侧静态声相原样保留，一个样本都不碰。
///
/// ⚠ 源侧静态声相在**构造期**已经关掉（见 [`track_pan_of`] 的
/// `static_pan_suppressed`）⇒ 这里乘的是**全部**声相，⛔ 不是第二次乘。
struct AutomationPan {
    /// 既有音源（引擎乐器 / SFZ / 最小音源 ＋ 音频片段）。
    inner: Box<dyn AudioSource>,
    /// 本轨的 `TrackPan` 自动化（构造期已排除恒等快路径）。
    automation: TrackPanAutomation,
    /// 工程的声相衰减律（与静态路径同一个 `PanLaw::from_model`）。
    law: PanLaw,
}

impl AudioSource for AutomationPan {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        self.inner.render_block(context, out)?;
        let law = self.law;
        self.automation.apply_pan_block(
            out,
            context.first_frame,
            context.frames,
            context.channels,
            |pan| pan_gains(pan, law),
        );
        Ok(())
    }
}

/// ⭐ 任务单 `#226`：**逐源峰值探针**（只读）。
///
/// 与 MCP 的 `PeakTap`（`crates/yeban-mcp/src/domain/render.rs`）**同一形态、同一测点**：
/// 每块渲染之后扫一遍已经写进 `out` 的样本，取 `max |sample|`，存进 `AtomicU32`
/// （非负有限 `f32` 的位型与数值同序 ⇒ `fetch_max` 就是"取更大"）。
///
/// ## 它为什么**不改音频**
///
/// - 只调一次 `inner.render_block`，写进去的字节一个不改；
/// - 只读 `out`，⛔ 不写；
/// - 峰值⛔ 不参与增益／归一化／量化。
///
/// 判据：`reporting_readings_does_not_change_a_single_byte` 用**加读数之前**的整文件
/// `sha256` 常量钉住（实测于 `main` @ `95bdfec3`）。
struct PeakTap {
    /// 真正的音源（引擎乐器 / SFZ / 最小音源 ＋ 音频片段）。
    inner: Box<dyn AudioSource>,
    /// 峰值位型（见类型文档）。
    peak_bits: Arc<AtomicU32>,
}

impl AudioSource for PeakTap {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        self.inner.render_block(context, out)?;
        let mut peak = 0.0_f32;
        for sample in out.iter() {
            if sample.is_finite() {
                peak = peak.max(sample.abs());
            }
        }
        if peak > 0.0 {
            self.peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
        }
        Ok(())
    }
}

/// 一条轨的**引擎层轨增益门**（可闻门 ＝ 静音/独奏 × 推子 × …）——⭐ **引擎的同一个算子**。
///
/// ⛔ 本函数**不**自己换算 dB：它只把工程的 `mute` / `solo` / `solo_safe` / `volume_db`
/// 原样交给引擎的 [`yeban_engine::synth::track_gain_scale`] —— 那个函数是引擎快照投影
/// （`snapshot::project_schedules`）与声部槽（`SynthEngine::set_track_gain_scales`）
/// 共用的**唯一**实现（⛔ 不是"这里再写一遍再对齐"）。
///
/// ⚠ `any_solo` 与引擎**同口径**：扫**全部**轨道（含主总线那一份），见
/// `crates/yeban-engine/src/snapshot.rs:907`。
///
/// ⚠ 找不到该轨（正常不会发生：`renderable` 已按 `project.tracks` 过滤）⇒ `0.0`
/// （静音）—— 宁可无声，⛔ 不是"忘了乘 ⇒ 满增益"。
fn track_gain_of(project: &YebanProjectV1, track_id: EntityId, any_solo: bool) -> f32 {
    project.tracks.get(&track_id).map_or(0.0, |track| {
        yeban_engine::synth::track_gain_scale(
            track.mute,
            track.solo,
            track.solo_safe,
            any_solo,
            track.volume_db,
        )
    })
}

/// ⭐ `#296`：同一条轨在**源侧**只乘"可闻门"（`mute`/`solo`/`solo_safe`）、
/// ⛔ 不乘推子的增益。
///
/// 用途：总线轨（有入边）的推子搬到 `RenderPlan` 的输出级，源侧必须把
/// `volume_db` 置 0，否则"自有音源"会被乘两次。可闻门仍留在源侧（静音的轨
/// 连自己的源都不发），与 [`track_gain_of`] 是**同一个** `track_gain_scale`。
fn track_gain_of_without_fader(
    project: &YebanProjectV1,
    track_id: EntityId,
    any_solo: bool,
) -> f32 {
    project.tracks.get(&track_id).map_or(0.0, |track| {
        yeban_engine::synth::track_gain_scale(
            track.mute,
            track.solo,
            track.solo_safe,
            any_solo,
            0.0,
        )
    })
}

/// 一条轨的**声相增益** `(左, 右)` —— ⭐ `#251`：MIDI 路径与音频片段路径的**同一个**入口。
///
/// 唯一实现住在 [`crate::audio_clip::track_pan_gains`]（`pan == 0` ⇒ `(1.0, 1.0)`，
/// 否则走引擎的等功率曲线）。本函数只补"轨道不在表里 ⇒ 居中"的兜底 ——
/// `renderable` 已按 `project.tracks` 过滤，正常不会走到。
///
/// ⭐ 任务单 `#257`：`static_pan_suppressed` 为真 ⇒ 这条轨有**非恒等**的 `TrackPan`
/// 自动化，源侧静态声相关掉（返回 `(1.0, 1.0)`），改由源输出之后的
/// [`TrackPanAutomation`] 乘**同一条曲线**算出的绝对左右增益。⛔ 不是乘第二次：
/// 自动化的值就是位置的唯一来源。
fn track_pan_of(
    project: &YebanProjectV1,
    track_id: EntityId,
    static_pan_suppressed: bool,
) -> (f32, f32) {
    if static_pan_suppressed {
        return (1.0, 1.0);
    }
    project.tracks.get(&track_id).map_or((1.0, 1.0), |track| {
        crate::audio_clip::track_pan_gains(project, track)
    })
}

/// 一次离线渲染的**全部读数**（含音频片段的渲染 / 跳过计数）。
#[derive(Clone, Debug, PartialEq)]
pub struct RenderProjectOutput {
    /// 交错 `f32` 样本。
    pub samples: Vec<f32>,
    /// 帧数。
    pub frames: u64,
    /// 音源文案（含音频片段计数）。
    pub source_label: String,
    /// 渲染输出的位级摘要（十六进制）。
    pub digest: String,
    /// 真的进了母带的音频摆放**个数**（不是素材个数、⛔ 也不是 span 数）。
    pub audio_clips_rendered: u64,
    /// ⭐ `#249`：循环展开后实际搬进母带的 **span 数**（与上面的摆放数分开报）。
    ///
    /// 非循环工程恒等于 `audio_clips_rendered`；一个 `0..960 / 摆放 1920` 的
    /// 循环摆放 ⇒ `audio_clips_rendered = 1`、`audio_clip_spans = 2`。
    pub audio_clip_spans: u64,
    /// 被**如实跳过**的音频片段原因（例如声道无法映射、坏 WAV、重采样失败）。
    /// ⚠ 采样率不符**不再**进这里：`#141` 起离线导出会重采样到导出率。
    pub audio_clips_skipped: Vec<String>,
    /// ⭐ 控制侧 SFZ 解码读数（任务单 `#79`；键 = 轨）。
    ///
    /// 空 = 本次渲染**没有**走模型 SFZ 链条（没有 `TrackV3.sampler`，或走了 `--sfz` 旁路 /
    /// 最小音源）。有内容 = 每条 `PendingSfz` 都真的读了盘并摊平（缺样本也**如实登记**）。
    pub sfz_decoded: Vec<(EntityId, crate::sfz_load::SfzDecodeReport)>,
    /// ⭐ 任务单 `#226`：**逐源峰值**（键 = 源节点 = 轨，值 = 该源输出的 `max |sample|`），
    /// 按 `EntityId` 升序（与计划节点同序 ⇒ 同一工程每次同一顺序）。
    ///
    /// 口径与 MCP 的 `sources[].peak` **逐条相同**：测点是**源节点输出**（含轨音量／声像／
    /// 采样器与片段增益），⛔ **不含**该轨插入链与路由边增益，也⛔ **不是**母带峰值。
    ///
    /// ⚠ 这是**只读**读数：探针只读 `out`，⛔ 不写样本、⛔ 不参与增益 ⇒ 产物逐字节不变
    /// （判据 `reporting_readings_does_not_change_a_single_byte` 用加读数之前的
    /// 整文件 `sha256` 常量钉住）。
    pub source_peaks: Vec<(EntityId, f32)>,
}

/// 把工程渲染成交错 `f32`：`(样本, 帧数, 音源文案, digest 十六进制)`。
///
/// ⚠ 本入口**没有**资产池、也**不知道工程文件在哪** ⇒ 音频片段被如实跳过并计数
/// （既有判据仍用它；⭐ `#170` 起 `record_wav` 改走 [`render_project_f32_with_assets_under`]，
/// `project_dir = None` 时两者**逐位相同**）。
/// 生产导出（`--export-wav`）走 [`render_project_f32_with_assets`]。
pub fn render_project_f32(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
) -> Result<(Vec<f32>, u64, String, String), WavExportError> {
    let output = render_project_f32_with_assets(project, options, sfz, &[])?;
    Ok((
        output.samples,
        output.frames,
        output.source_label,
        output.digest,
    ))
}

/// 与 [`render_project_f32`] 相同，但**带上归档资产池** ⇒ `ClipContent::Audio` 真的发声。
///
/// `assets` 的形状就是 `ProjectArchive.assets`。给空切片时与 [`render_project_f32`]
/// 逐位相同（音频片段被跳过并计数）。
///
/// # Errors
///
/// 与 [`render_project_f32_with_plans`] 相同。
pub fn render_project_f32_with_assets(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(AssetHash, Vec<u8>)],
) -> Result<RenderProjectOutput, WavExportError> {
    render_project_f32_with_assets_under(project, options, sfz, assets, None)
}

/// 与 [`render_project_f32_with_assets`] 相同，但**知道工程文件所在目录**
/// （任务单 `#168`：相对 SFZ 路径的第一个候选根）。
///
/// `project_dir = None` ⇒ 与 [`render_project_f32_with_assets`] **逐位相同**
/// （相对路径按仓库根 ⇒ cwd 解析）。生产 `--export-wav` 从 `--open` 的路径取父目录传进来，
/// 于是"样本放在工程旁边"在**任意 cwd** 下都读得到。
///
/// # Errors
///
/// 与 [`render_project_f32_with_plans`] 相同。
pub fn render_project_f32_with_assets_under(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(AssetHash, Vec<u8>)],
    project_dir: Option<&Path>,
) -> Result<RenderProjectOutput, WavExportError> {
    render_project_f32_with_plans_under(project, options, sfz, assets, BTreeMap::new(), project_dir)
}

/// 与 [`render_project_f32_with_assets`] 相同，但可**额外注入已摊平的 SFZ 表**（任务单 `#79`）。
///
/// ## 两个来源，同一个注入点
///
/// * ⭐ **模型链条**（`plans` 为空时的默认行为）：快照里每条 `PendingSfz` 由
///   [`crate::sfz_load::arm_pending_sfz`] 走"读盘 → 解码 → `SfzPlan::prepare_with_samples` →
///   [`EngineSnapshot::with_sfz`]"，然后交给 `render_track` 的第三条音源支路；
/// * ⭐ **`plans`**：调用方**已经**持有摊平表（缓存/判据）时的注入入口 —— 与模型链条**同一个**
///   `with_sfz`（因此也**取代**该轨的待摊平项，⛔ 不重复解码）。
///
/// ⚠ `plans` 只在"用引擎音源"（`options.use_engine_source` 且 `sfz = None`）时有效：
/// `--sfz` 旁路与最小音源都**整个替换**引擎音源（见模块文档的音源表），与本入口无关。
///
/// # Errors
///
/// 与 [`render_project_f32`] 相同，外加：
/// * [`WavExportError::Sfz`]：模型链条里某个 SFZ 文件**读不到 / 文本非法**（⛔ 不静默回退）。
///   缺**样本**不算失败（该区域静音并计数，见 [`crate::sfz_load::decode_sfz_plan`]）。
pub fn render_project_f32_with_plans(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(AssetHash, Vec<u8>)],
    plans: BTreeMap<EntityId, yeban_engine::sfz_instrument::SfzPlan>,
) -> Result<RenderProjectOutput, WavExportError> {
    render_project_f32_with_plans_under(project, options, sfz, assets, plans, None)
}

/// 与 [`render_project_f32_with_plans`] 相同，但**知道工程文件所在目录**
/// （任务单 `#168`：相对 SFZ 路径的第一个候选根）。
///
/// `project_dir = None` ⇒ 与 [`render_project_f32_with_plans`] **逐位相同**。
///
/// # Errors
///
/// 与 [`render_project_f32_with_plans`] 相同，外加"相对路径在全部候选根下都找不到"时的
/// [`WavExportError::Sfz`]（原文逐字带"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"）。
pub fn render_project_f32_with_plans_under(
    project: &YebanProjectV1,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
    assets: &[(AssetHash, Vec<u8>)],
    plans: BTreeMap<EntityId, yeban_engine::sfz_instrument::SfzPlan>,
    project_dir: Option<&Path>,
) -> Result<RenderProjectOutput, WavExportError> {
    // ⭐ `#168`：唯一的解析上下文（工程目录 ⇒ 仓库根 ⇒ cwd）。
    let sfz_roots = match project_dir {
        Some(dir) => crate::sfz_load::SfzRoots::under_project(dir),
        None => crate::sfz_load::SfzRoots::none(),
    };
    if options.frames == 0 {
        return Err(WavExportError::Empty("请求的帧数为 0".to_owned()));
    }
    // ⭐ `#296`：**总线节点**（路由图里**有入边**的非主轨）。它们的推子从
    // **源侧**搬到 `RenderPlan` 的**输出级**（插入链之后），这样推子乘的是
    // "自有音源 ＋ 入边求和"整条通道，而不是只乘自己的那一份。
    //
    // 口径与计划的剪枝同解：任何一条入边的源只要能到达这个节点，就能经它到达
    // master ⇒ 两端都会被保留。因此"图里有入边"＝"计划里 `incoming` 非空"。
    let bus_nodes: BTreeSet<EntityId> = project
        .routing_graph
        .edges
        .values()
        .map(|edge| edge.destination_node)
        .filter(|node| *node != project.master_bus_track_id && project.tracks.contains_key(node))
        .collect();
    // ---- 0. 引擎装载（**慢路径校验先行**：引擎文档要求先做慢路径校验，再进入渲染循环）----
    //
    // 为什么提前：`arm_from_snapshot` 在**没有可武装的轨/音色/鼓**时返回 `NoMasterBus`；
    // 它是"这个工程能不能用引擎渲染"的判定，理应在计划编译与任何 I/O 之前发生
    // ⇒ 失败时一个字节都不会落盘，也不会先被计划层的次生错误（例如"没有注册样本源"）掩盖。
    // 带 SFZ ⇒ 走 SFZ 音源（⛔ 不需要引擎快照；SFZ 自己的解析在离线路径里做）。
    let mut sfz_decoded: Vec<(EntityId, crate::sfz_load::SfzDecodeReport)> = Vec::new();
    let snapshot = if options.use_engine_source && sfz.is_none() {
        // ⭐ `#296`：总线轨的推子改由计划的**输出级**乘 ⇒ 快照投影时把它们的
        // `volume_db` 置 0（⛔ 不动 `mute`/`solo`：可闻门仍在源侧），否则自有音源
        // 会被乘两次。没有总线节点的工程**不 clone**（默认路径零改动）。
        let bus_snapshot_project;
        let snapshot_source = if bus_nodes.is_empty() {
            project
        } else {
            bus_snapshot_project = {
                let mut copy = project.clone();
                for node in &bus_nodes {
                    if let Some(track) = copy.tracks.get_mut(node) {
                        track.volume_db = 0.0;
                    }
                }
                copy
            };
            &bus_snapshot_project
        };
        let snapshot = EngineSnapshot::from_project(snapshot_source, 1)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        // ⭐ ⭐ **本单接线（`#79`）**：`TrackV3.sampler` 已经在快照里是"待摊平项"
        //    （`#78`，引擎⛔ 不读盘）⇒ 控制侧（**这里**，允许 I/O 与分配）把它走完六步，
        //    `with_sfz` 注入摊平表（并取代待摊平项）。失败 ⇒ `Sfz` 错误（⛔ 不静默）。
        let (snapshot, decoded) = crate::sfz_load::arm_pending_sfz(
            snapshot,
            options.sample_rate as f32,
            plans,
            sfz_roots,
        )
        .map_err(|error| WavExportError::Sfz(error.to_string()))?;
        sfz_decoded = decoded;
        // ⭐ 快照以 `Arc` 共享给逐轨音源：`render_track` 只**借用**里面的摊平表
        //    （⛔ 不 clone PCM —— 与实时路径同一条纪律）。
        let snapshot = std::sync::Arc::new(snapshot);
        // ⭐ **慢路径校验只做一次**，且必须在这里做：武装是**逐源**发生的，
        // 而"没有任何可渲染轨"时一个源都不会被造出来 ⇒ 引擎的 `NoMasterBus` 裁决会被
        // 计划层的次生错误（"没有注册样本源"）掩盖（本机实测：清空轨道表时正是这样）。
        // ⇒ 用一个**探针引擎**先武装一次，让"能不能用引擎渲染这个工程"的判定发生在
        //   计划编译与任何 I/O **之前**（引擎文档要求先做慢路径校验）。
        let mut probe = SynthEngine::new(snapshot.sample_rate());
        probe
            .arm_from_snapshot(&snapshot)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        Some(snapshot)
    } else {
        None
    };

    let options_render = RenderOptions {
        frames: options.frames,
        sample_rate: options.sample_rate,
        channels: options.channels,
        seed: options.seed,
        ..RenderOptions::default()
    };

    // ---- 1. 编译计划（图与 master 都来自工程本身，⛔ 不猜）----
    //
    // ⭐ 任务单 `#244`：离线导出与 MCP（`yeban-mcp/src/domain/render.rs:2020`）走
    // **同一条生产装配** —— 延迟表与插入链都从同一份 `TrackV3.devices` 投影：
    //
    // - [`track_latencies`] 是 `DeviceDefinition::latency_samples` 的**唯一累加处**
    //   （[ARCH-PDC-001]；旁通设备不计入 ⇒ 旁通 = 没有那台设备）；
    // - [`project_inserts_with_forced_instances`] 是"设备链 → 插入链"的**唯一转换器**
    //   （EQ / Freeverb / 卷积；`#266` 起带"强制实例化通道条"、`#293` 起带"强制实例化
    //   Freeverb/卷积"的编译期开关），`yeban-render/tests/insert_chain.rs` 与 MCP 都在用它
    //   ⇒ ⛔ 这里不写第二份。
    //
    // 修前这里调 `RenderPlan::compile(...)`，它内部等价于传**两张空表**
    // （`crates/yeban-render/src/render.rs:439` 的 `&BTreeMap::new()`）⇒ 用户装的
    // EQ / 混响 / 卷积**完全不进导出**（`#226`／`#240` 登记的口径）。
    let latencies = track_latencies(&project.tracks);
    // ⭐ 任务单 `#266`/`#293`：有**非恒等** `DeviceParam` 泳道的轨，对应器件被**强制
    // 实例化**（通道条：`effective_strip` 在全恒等参数下也返回 `Some`；Freeverb/卷积：
    // 静态 `wet = 0` 时 `is_active()` 为假，但块边界仍需有器件可改）。扫描器与 MCP 的
    // `yeban_render_master` 共用同一份（`yeban_render::automation`），
    // ⛔ 本层不复制"哪些泳道/哪一族"的判定。
    let device_automations = device_param_automations(project, options.sample_rate);
    let forced = forced_insert_instances(&device_automations);
    let inserts =
        project_inserts_with_forced_instances(&project.tracks, options.sample_rate, &forced);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &project.routing_graph,
        project.master_bus_track_id,
        options_render,
        &latencies,
        &inserts,
    )
    .map_err(|error| WavExportError::Render(error.to_string()))?;

    // ---- 1b. ⭐ 发送增益自动化（`SendGain`，任务单 `#256`）----
    //
    // 落点是**这条路由边的入边归约**（`RenderPlan::render_node`），因此与 MCP 的
    // `yeban_render_master` 共用同一份求值/平滑实现（`yeban_render::automation`）。
    // 没有 `SendGain` 泳道的工程一个计划槽都不改 ⇒ 产物逐字节不变。
    for automation in send_gain_automations(project, options.sample_rate).into_values() {
        plan.attach_send_automation(automation);
    }

    // ---- 1c. ⭐ 设备参数自动化（`DeviceParam`，任务单 `#266`）----
    //
    // 落点是**该节点的插入链运行态**（`InsertRuntime::apply_device_params`，块边界
    // 一次对应器件的 `set_params`）。恒等泳道也挂（`value_at_block` 返回 `None`，
    // 一个参数都不改）；没有设备参数泳道的工程一个槽都不改 ⇒ 产物逐字节不变。
    for automation in device_automations.into_values().flatten() {
        plan.attach_device_param_automation(automation);
    }

    // ---- 1d. ⭐ 输出级（任务单 `#296`）：总线推子 ＋ 主总线声相 ----
    //
    // 与 MCP 的 `yeban_render_master` **同一份** `RenderPlan::attach_output_stage`：
    // - **总线轨**（有入边的非主轨）挂推子（静态 `track_gain_of` ＋ 可选的
    //   `TrackVolume` 自动化），落点是插入链之后；源侧推子已在 `snapshot` 投影／
    //   三个源分支里被置成 1.0（`mute`/`solo` 门保留），因此**不会乘两次**；
    // - **主总线**挂声相（既有等功率曲线 ＋ `centered_pan_gains` 的中心规则）；
    //   `TrackPan` 非恒等时静态声相被关掉，改由执行之后乘绝对左右增益（`#257` 纪律）。
    //
    // 没有总线 / `master.pan == 0` 的工程：两个 `OutputStage` 都是恒等 ⇒
    // `render_node` 的最后一级一个样本都不碰 ⇒ 默认路径逐位不变。
    let any_solo = project.tracks.values().any(|track| track.solo);
    for &node in &bus_nodes {
        let Some(track) = project.tracks.get(&node) else {
            continue;
        };
        let mut stage = OutputStage::new().with_gain(track_gain_of(project, node, any_solo));
        if track
            .automation_lanes
            .keys()
            .any(|target| matches!(target, yeban_model::AutomationTarget::TrackVolume { .. }))
            || macro_cascade_reaches(
                project,
                &yeban_model::AutomationTarget::TrackVolume { track_id: node },
            )
        {
            stage = stage.with_gain_automation(TrackGainAutomation::new(
                project,
                node,
                options.sample_rate,
            ));
        }
        // 不在保留子图（不可达）⇒ 返回 false；本层不改任何东西。
        plan.attach_output_stage(node, stage);
    }
    let pan_law = PanLaw::from_model(project.audio_config.pan_law);
    let mut master_pan_automation: Option<TrackPanAutomation> = None;
    if let Some(master) = project.tracks.get(&project.master_bus_track_id) {
        let automation = TrackPanAutomation::new(project, master.id, options.sample_rate);
        let static_pan = if automation.is_identity() {
            master.pan
        } else {
            0.0
        };
        plan.attach_output_stage(
            master.id,
            OutputStage::new().with_pan(static_pan, |pan| pan_gains(pan, pan_law)),
        );
        if !automation.is_identity() {
            master_pan_automation = Some(automation);
        }
    }

    // ---- 2. 每条非 master 轨道一个源 ----
    //
    // 默认走**引擎乐器**：先投影快照（`EngineSnapshot::from_project`），再按轨武装
    // （`arm_from_snapshot`，与实时路径同一条装配序列）。任何一步失败都在**落盘之前**返回
    // ⇒ 不留半成品。`use_engine_source = false` 时用最小音源（判据的对照 + 极端回退）。
    // 计划**自己**的节点表是唯一权威：`RenderPlan::nodes()`（图里存在但不可达的节点会被剪枝，
    // 给它塞源会得到"节点 … 不在渲染计划里（未知或被剪枝）" —— 本机实测踩过两次）。
    //
    // ⭐ 任务单 `#240`（Logic Pro 的通道条语义）：**每一条非主总线轨都注册自己的源**，
    // 包括有入边的 `AuxReturn`。Logic 的通道条只有一种形状：
    // `输入端求和（送进来的 send ＋ 自己的音源输出）→ 插入链 → 输出`
    // ⇒ "总线既有自己的音源、又接别处的发送"是常态，两路相加。
    //
    // 历史：`#237` 曾把"有入边的 `AuxReturn`"从源表里**拿掉**（`aux_return_is_a_bus`），
    // 因为当时的 `RenderPlan::execute` 拒绝"源 ＋ 入边"（`RenderError::SourceOnBusNode`
    // ⇒ 逐字"节点 … 有入边, 不能注册样本源"），于是 aux 的采样器被整个跳过。
    // 本单在渲染器里定义了那个缺失的语义（源进输入端、与入边同权相加），因此这层
    // 绕法**不再需要**：aux 既注册源，也仍然接受入边。
    //
    // ⚠ `Sidechain`：渲染器一直把它当**普通音频边**处理（`render.rs` 模块头已登记
    // "侧链键控语义未实现"）。因此侧链的目标节点现在也能有自己的源 —— 两路会在输入端
    // 相加，而不是像修前那样报"有入边, 不能注册样本源"。**键控**仍然未实现，
    // 这条边界没有变化；没有把任何一路**丢掉**。
    let renderable: Vec<EntityId> = plan
        .nodes()
        .into_iter()
        .filter(|id| *id != project.master_bus_track_id && project.tracks.contains_key(id))
        .collect();
    // ---- 2a-pre. ⭐ `TrackPan` 自动化（任务单 `#257`）----
    //
    // 两个导出的**同一份**扫描器（`yeban_render::automation::pan_automations`）。
    // 有**非恒等**泳道的轨才进这张表：源侧静态声相在下面三个分支与音频片段里被
    // 关掉（`pan = 0` ⇒ `(1, 1)`，乘上去逐位恒等），改由一个
    // [`AutomationPan`] 门在**源输出之后**乘自动化位置对应的绝对左右增益 ——
    // 与静态声相**同一条**曲线（[`crate::audio_clip::track_pan_gains`]）。
    // 恒等（没有泳道 / 关掉 / 空泳道 / 处处等于静态 `pan`）⇒ 这张表里没有它，
    // 源侧静态声相原样保留 ⇒ 产物逐字节不变。
    //
    // ⚠ 有入边的总线轨**不进**这张表：总线声相是 `#296` B2（本单不做），与 MCP
    // 的 `busTrack` 残差同口径。app 侧不发明第三种落点。
    let mut pan_gates: BTreeMap<EntityId, TrackPanAutomation> =
        pan_automations(project, options.sample_rate);
    pan_gates.retain(|track_id, _| !bus_nodes.contains(track_id));
    pan_gates.retain(|_, automation| !automation.is_identity());
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let mut source_label: String = if let Some((instrument, sfz_path)) = sfz.as_ref() {
        let mut labels: Vec<String> = Vec::new();
        // ⭐ **轨增益门**（推子 × 静音/独奏）：`any_solo` 与引擎同口径（含主总线那一份）。
        // ⚠ 这段必须在这里算：SFZ 分支**不建引擎快照**（见上面的 `snapshot` 判据），
        // 因此拿不到 `NoteSchedule::track_gain`；改调**同一个** `track_gain_scale` 就不会分叉。
        let any_solo = project.tracks.values().any(|track| track.solo);
        for track_id in renderable.clone() {
            let notes = track_notes_in_frames(project, track_id, options.sample_rate);
            // ⭐ `#296`：总线轨的推子改由输出级乘 ⇒ 源侧只保留 `mute`/`solo` 门
            // （`volume_db = 0` 的同一个 `track_gain_scale`），⛔ 不乘两次。
            let track_gain = if bus_nodes.contains(&track_id) {
                track_gain_of_without_fader(project, track_id, any_solo)
            } else {
                track_gain_of(project, track_id, any_solo)
            };
            let track_source = match sfz_path {
                Some(sfz_path) => crate::sfz_load::SfzTrackSource::new_from_sfz_path(
                    instrument.clone(),
                    sfz_path,
                    &notes,
                    options.sample_rate as f32,
                    options.channels,
                ),
                None => crate::sfz_load::SfzTrackSource::new(
                    instrument.clone(),
                    &notes,
                    options.sample_rate as f32,
                    options.channels,
                ),
            }
            // ⭐ 与引擎侧同形（`NoteSchedule::with_track_gain`）：轨增益乘在**乐器层输出之外**。
            // ⭐ `#255`：`--sfz` 旁路也吃**轨声相** —— 值来自 [`crate::audio_clip::track_pan_gains`]
            //    （与引擎乐器路径／MCP 的采样器轨同一条等功率曲线）。
            //    `pan == 0` ⇒ `(1.0, 1.0)` ⇒ 导出字节逐位不变。
            .with_track_gain(track_gain)
            .with_pan(track_pan_of(
                project,
                track_id,
                pan_gates.contains_key(&track_id),
            ));
            labels.push(track_source.sample_source());
            sources.insert(track_id, Box::new(track_source));
        }
        labels
            .first()
            .map_or_else(|| "sfz(无区域)".to_owned(), |label| format!("sfz({label})"))
    } else if let Some(snapshot) = snapshot.as_ref() {
        for track_id in renderable.clone() {
            // ⭐ `#255`：**采样器轨也乘 pan**。修前这里按 `snapshot.sfz().contains_key`
            // 分流、把绑了采样器的轨钉成 `(1.0, 1.0)`（对齐当时**不应用**声相的 MCP
            // 采样器源）。⭐ `#274`：MCP 的采样器轨也走同一个 `EngineTrackSource` ⇒
            // 分流取消，全部走 [`track_pan_of`]（它内部是 `audio_clip::track_pan_gains`）。
            // `render_track` 输出的是**声相之前**的单声道 ⇒ 在这里注入 pan（`#251` 的落点不变）。
            let pan = track_pan_of(project, track_id, pan_gates.contains_key(&track_id));
            sources.insert(
                track_id,
                // ⭐ `Arc::clone` 是**指针复制**（ADR：实时侧只借快照）：摊平表的 PCM 留在
                //    快照里，⛔ 不按源再 clone 一份（见 `EngineTrackSource` 的字段文档）。
                // ⭐ `#251`：`render_track` 输出的是**声相之前**的单声道 ⇒ 在这里注入 pan。
                Box::new(
                    EngineTrackSource::new(std::sync::Arc::clone(snapshot), track_id)?
                        .with_pan(pan),
                ),
            );
        }
        let mut label = "引擎乐器（SynthEngine::arm_from_snapshot + render_track）".to_owned();
        if !sfz_decoded.is_empty() {
            label.push_str(&format!(" + {}", sfz_decode_label(&sfz_decoded)));
        }
        label
    } else {
        // ⭐ **轨增益门**（推子 × 静音/独奏）：与 SFZ 分支**同一个** [`track_gain_of`]
        // （⇒ 同一个 `track_gain_scale`）。⚠ 这段必须在这里算：最小音源同样是**整体替换**
        // 引擎音源的旁路（见上面 `snapshot` 判据），拿不到投影期已乘好的那一份。
        let any_solo = project.tracks.values().any(|track| track.solo);
        for track_id in renderable.clone() {
            // ⭐ `#296`：与 SFZ 分支同一条 —— 总线轨的推子改由输出级乘。
            let track_gain = if bus_nodes.contains(&track_id) {
                track_gain_of_without_fader(project, track_id, any_solo)
            } else {
                track_gain_of(project, track_id, any_solo)
            };
            sources.insert(
                track_id,
                Box::new(
                    NoteToneSource::from_track(project, track_id, options.sample_rate)
                        // ⭐ 与 SFZ 侧同形（`SfzTrackSource::with_track_gain`）：轨增益乘在
                        //    **本源输出之外**；`1.0` ⇒ 跳过乘法、读数逐位不变。
                        .with_track_gain(track_gain)
                        // ⭐ `#251`：与音频片段**同一条**等功率曲线（`pan = 0` ⇒ 恒等）。
                        .with_pan(track_pan_of(
                            project,
                            track_id,
                            pan_gates.contains_key(&track_id),
                        )),
                ),
            );
        }
        "最小音源（方波 + 首尾包络；回退）".to_owned()
    };

    // ---- 2b. 音频片段（`ClipContent::Audio`）叠加 ----
    //
    // 引擎快照把音频片段当静音（`snapshot.rs` 的 `project_schedules` 第 5 条登记），
    // 因此音频摆放必须由**本层**叠加；语义与 MCP 的 `AudioClipSource` 逐条对齐
    // （见 `crate::audio_clip` 的模块文档：帧区间 / 硬切 / 累加 / 增益 / 声道布局）。
    let mut audio_cache = BTreeMap::new();
    let mut audio_skipped: Vec<String> = Vec::new();
    let mut audio_rendered = 0_u64;
    // `#249`：`audio_rendered` 保持 `#241` 的**摆放数**口径；循环展开出的
    // span 数单独计（⛔ 两个口径不混）。
    let mut audio_clip_spans = 0_u64;
    for track_id in renderable.clone() {
        let clips = audio_clip::resolve_track_clips(
            project,
            track_id,
            options.sample_rate,
            options.channels,
            assets,
            &mut audio_cache,
            &mut audio_skipped,
            pan_gates.contains_key(&track_id),
            // ⭐ `#296`：总线轨的推子改由输出级乘 ⇒ 片段增益里不带 track volume。
            bus_nodes.contains(&track_id),
        );
        if clips.sources.is_empty() {
            continue;
        }
        audio_rendered = audio_rendered.saturating_add(clips.placements);
        audio_clip_spans = audio_clip_spans.saturating_add(clips.span_count);
        if let Some(base) = sources.remove(&track_id) {
            sources.insert(
                track_id,
                Box::new(TrackSource {
                    base,
                    clips: clips.sources,
                }),
            );
        }
    }
    if audio_rendered > 0 || !audio_skipped.is_empty() {
        source_label.push_str(&format!(
            " + audio-clips(rendered={audio_rendered} skipped={}) spans={audio_clip_spans}",
            audio_skipped.len()
        ));
    }

    // ---- 2b-bis. ⭐ 音量自动化（`TrackVolume`）：每条可渲染轨一个门 ----
    //
    // 挂点与 MCP 的 `TrackSource` 相同：**源输出之后**（引擎乐器 / SFZ / 最小音源
    // ＋ 音频片段三路都已相加），因此自动化对这条轨的全部贡献一视同仁。
    // 恒等的轨**不包**这一层（没有泳道 / 关掉 / 空泳道 / 曲线处处等于静态推子）。
    // ⭐ `#296`：`TrackVolume` 现在有三个落点（与 MCP 同口径）—— 源节点（没有入边，
    // 就是这一层）、总线节点（`RenderPlan` 的输出级，见上面 1d）与主总线（执行之后）。
    // 因此总线轨**不**在这里包门：它的自动化已挂到输出级，包这里会把推子只乘到
    // "自己的源"上（⛔ 漏掉入边求和）。
    // ⭐ `#257`：`TrackPan` 的门**先于**音量门（与静态路径同序：源里的声相 → 音量），
    // 且只在 `pan_gates` 里（非恒等 + 非总线）时才包。
    let mut automated: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    for (track_id, source) in std::mem::take(&mut sources) {
        let source: Box<dyn AudioSource> = if let Some(automation) = pan_gates.remove(&track_id) {
            Box::new(AutomationPan {
                inner: source,
                automation,
                law: pan_law,
            })
        } else {
            source
        };
        let automation = TrackGainAutomation::new(project, track_id, options.sample_rate);
        if automation.is_identity() || bus_nodes.contains(&track_id) {
            automated.insert(track_id, source);
        } else {
            automated.insert(
                track_id,
                Box::new(AutomationGain {
                    inner: source,
                    automation,
                }),
            );
        }
    }
    let mut sources = automated;

    // ---- 2c. 逐源峰值探针（⭐ `#226`；只读）----
    //
    // ⚠ 必须在**全部源都装配完**（含音频片段）之后套：峰值要涵盖这一条轨真正进母带的
    // 全部贡献。⛔ 探针只读 `out`、不改一个样本（见 [`PeakTap`] 的文档）。
    let mut peak_bits: BTreeMap<EntityId, Arc<AtomicU32>> = BTreeMap::new();
    let mut tapped: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    for (track_id, source) in std::mem::take(&mut sources) {
        let bits = Arc::new(AtomicU32::new(0));
        peak_bits.insert(track_id, Arc::clone(&bits));
        tapped.insert(
            track_id,
            Box::new(PeakTap {
                inner: source,
                peak_bits: bits,
            }),
        );
    }

    // ---- 3. 执行（全部在内存里）----
    let mut output = plan
        .execute(tapped)
        .map_err(|error| WavExportError::Render(error.to_string()))?;

    // ---- 3b. ⭐ 主总线的音量（含 `TrackVolume` 自动化）----
    //
    // 主总线没有源：它的静态推子在任何源侧都无法表达，只能在执行之后乘。这是音量
    // 自动化的**第二个站点**（MCP 的 `build` 第 9 步是同一条口径）：
    //   - 恒等（没有可求值点 / 曲线处处等于静态值）⇒ 保留原来的“乘静态主推子”，
    //     `1.0` 时一个样本都不碰 ⇒ 没有自动化的工程逐字节不变；
    //   - 非恒等 ⇒ 按 128 帧块求值 + 逐样本平滑，乘 `静态主推子 × R55 比率`。
    // ⚠ `mute` 与 MCP 同口径：主总线静音 ⇒ 静态主推子 `0.0`（全静音）。
    if let Some(master) = project.tracks.get(&project.master_bus_track_id) {
        let master_gain = if master.mute {
            0.0
        } else {
            db_to_linear(master.volume_db)
        };
        let mut automation = TrackGainAutomation::new(project, master.id, options.sample_rate);
        if automation.is_identity() {
            if master_gain != 1.0 {
                for sample in &mut output.samples {
                    *sample *= master_gain;
                }
            }
        } else {
            let frames = output.samples.len() / options.channels.max(1);
            automation.apply_gain_block(
                &mut output.samples,
                0,
                frames,
                options.channels,
                master_gain,
            );
        }
        // ⭐ `#296`：主总线的 `TrackPan` 自动化（推子之后、归一化之前）。静态声相
        // 已在计划的输出级里乘过；非恒等时它被关掉，这里乘的是**绝对**左右增益。
        if let Some(automation) = master_pan_automation.as_mut() {
            let channels = options.channels.max(1);
            let frames = output.samples.len() / channels;
            automation.apply_pan_block(&mut output.samples, 0, frames, channels, |pan| {
                pan_gains(pan, pan_law)
            });
        }
    }

    // 主总线增益/自动化是**执行之后**才乘的，而 `RenderOutput::digest` 在 `execute`
    // 里就按当时（未乘主推子）的样本算好了。`RenderProjectOutput::digest` 的文档是
    // “渲染输出的位级摘要” ⇒ 对**最终**样本重算，否则摘要与 `samples` 分叉。
    output.digest = RenderOutput::digest_of(&output.samples);

    // 渲染结束后取回读数：⛔ 此后一个样本都不再改（只生成报告字段）。
    let source_peaks: Vec<(EntityId, f32)> = peak_bits
        .iter()
        .map(|(track_id, bits)| (*track_id, f32::from_bits(bits.load(Ordering::Relaxed))))
        .collect();

    // ---- 4. 逐个样本量化（TPDF 抖动；位深 24 bit 整数 PCM）----
    let mut digest = String::with_capacity(output.digest.len() * 2);
    for byte in &output.digest {
        use core::fmt::Write;
        let _ = write!(digest, "{byte:02x}");
    }
    Ok(RenderProjectOutput {
        samples: output.samples,
        frames: output.frames,
        source_label,
        digest,
        audio_clips_rendered: audio_rendered,
        audio_clip_spans,
        audio_clips_skipped: audio_skipped,
        sfz_decoded,
        source_peaks,
    })
}

/// ⭐ 模型 SFZ 链条的**读数文案**（进 `source_label` ⇒ ⛔ 缺样本不静默）。
///
/// 形态：`sfz(tracks=1 regions=4 samples=1 silent=1 missing=["missing.wav"] unreadable(not_found=0,unsupported=0,other=0))`。
/// ⭐ `unreadable(...)` 是任务单 `#83` 加的可判别计数：**文件不存在**与**格式不受支持**
/// （FLAC／OGG）分开报 —— 否则用户会把"格式不支持"读成"缺文件"。
/// ⚠ 逗号分隔、**无空格**（报告行按空格切词，⛔ 不引入第二种切法）。
fn sfz_decode_label(decoded: &[(EntityId, crate::sfz_load::SfzDecodeReport)]) -> String {
    let regions: usize = decoded.iter().map(|(_, report)| report.regions).sum();
    let samples: usize = decoded
        .iter()
        .map(|(_, report)| report.decoded_samples)
        .sum();
    let silent: usize = decoded
        .iter()
        .map(|(_, report)| report.silent_region_count)
        .sum();
    let missing: Vec<&str> = decoded
        .iter()
        .flat_map(|(_, report)| report.missing_sample_paths.iter().map(String::as_str))
        .collect();
    // ⭐ `#83`：读失败的**可判别**计数（not_found / unsupported / other）。
    let not_found: usize = decoded
        .iter()
        .map(|(_, report)| report.unreadable_not_found_count())
        .sum();
    let unsupported: usize = decoded
        .iter()
        .map(|(_, report)| report.unreadable_unsupported_format_count())
        .sum();
    let other: usize = decoded
        .iter()
        .map(|(_, report)| report.unreadable_other_count())
        .sum();
    format!(
        "sfz(tracks={} regions={regions} samples={samples} silent={silent} missing={missing:?} \
         unreadable(not_found={not_found},unsupported={unsupported},other={other}))",
        decoded.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::TrackV3;

    /// 夹具：`filled_project` 里那条**带 MIDI 音符**的非主总线轨（＝ 工程里唯一会发声的
    /// 可渲染轨：另一条音频轨没有资产池 ⇒ 被如实跳过），把三个自变量写进去。
    fn fixture(volume_db: f32, mute: bool, solo_elsewhere: bool) -> (YebanProjectV1, EntityId) {
        let mut project = yeban_model::samples::filled_project();
        let lead = *project
            .tracks
            .iter()
            .find(|(id, track)| {
                **id != project.master_bus_track_id
                    && track.clips.values().any(|placement| {
                        project
                            .clip_pool
                            .get(&placement.clip_id)
                            .is_some_and(|entry| entry.content.notes().is_some())
                    })
            })
            .map(|(id, _)| id)
            .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨");
        {
            let track = project.tracks.get_mut(&lead).expect("上面刚找到");
            track.volume_db = volume_db;
            track.mute = mute;
        }
        if solo_elsewhere {
            let other = EntityId::new();
            project.tracks.insert(
                other,
                TrackV3 {
                    id: other,
                    name: "Solo".to_owned(),
                    solo: true,
                    ..TrackV3::default()
                },
            );
            project.routing_graph.nodes.push(other);
        }
        (project, lead)
    }

    /// ⭐ 判据（本单新增）：**同一个算子**。
    ///
    /// # 量什么（先说再跑）
    ///
    /// 同一份工程上，[`track_gain_of`]（＝ SFZ 旁路注入的那个标量）与引擎快照投影写进
    /// `NoteSchedule::track_gain` 的那个标量，**逐位相同的 f32**（单位：线性增益）。
    ///
    /// * 对象 ＝ 一条轨的轨增益门（可闻门 × 推子）；
    /// * 单位 ＝ 线性幅度倍数（无量纲，`1.0` ＝ 单位增益、`0.0` ＝ 静音）。
    ///
    /// # 为什么必须逐位比
    ///
    /// 判据是"**与引擎用同一个函数／同一套字段**，⛔ 不另写一份 `volume_db ⇒ 线性` 换算"。
    /// 只比"接近"会放过"两份实现恰好都差不多"的情形；逐位比恰好就是"同一次调用的结果"
    /// （引擎侧注释的原话：`slot.gain_scale` 与 `NoteSchedule::track_gain` **逐位相等**）。
    ///
    /// # 怎么变红
    ///
    /// 把 [`track_gain_of`] 改成自己换算（例如 `db_to_linear(volume_db)` 而丢掉
    /// `track_is_audible` 门）⇒ 静音档 / 别轨独奏档当场不等；换成
    /// `10f32.powf(db / 20.0)` ⇒ 末位与引擎的 `exp2` 口径不同，`to_bits()` 也当场不等。
    #[test]
    fn the_sfz_track_gain_is_bit_identical_to_the_engine_projection() {
        for (volume_db, mute, solo_elsewhere) in [
            (0.0_f32, false, false),
            (-6.0, false, false),
            (-3.5, false, false),
            (0.0, true, false),
            (0.0, false, true),
        ] {
            let (project, lead) = fixture(volume_db, mute, solo_elsewhere);
            let any_solo = project.tracks.values().any(|track| track.solo);
            let ours = track_gain_of(&project, lead, any_solo);
            let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须成快照");
            let theirs = snapshot
                .schedules()
                .get(&lead)
                .expect("本轨必须有调度表")
                .track_gain();
            println!(
                "[app-sfz-gain] volume_db={volume_db} mute={mute} solo_elsewhere={solo_elsewhere}: \
                 app={ours:.9} ({:#010x}) vs 引擎投影={theirs:.9} ({:#010x})",
                ours.to_bits(),
                theirs.to_bits()
            );
            assert_eq!(
                ours.to_bits(),
                theirs.to_bits(),
                "⭐ SFZ 旁路注入的轨增益必须与引擎投影 `NoteSchedule::track_gain` **逐位相同**\
                 （volume_db={volume_db} mute={mute} solo_elsewhere={solo_elsewhere}）"
            );
        }
    }

    /// 峰值（绝对值最大；单位：线性幅度）—— 本模块的两条最小音源判据用它排除"全零也算恒等"。
    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0_f32, |acc, sample| acc.max(sample.abs()))
    }

    /// 一块 8 秒 48 kHz 的单声道上下文（与 `WavExportOptions::default()` 同长）。
    fn block_context(frames: usize) -> BlockContext {
        BlockContext {
            block_index: 0,
            first_frame: 0,
            frames,
            channels: 1,
            sample_rate: 48_000,
            seed: 1,
        }
    }

    /// ⭐ 判据（本单新增，`#71`）：**同一个算子** —— 最小音源注入的那一份 vs 引擎投影。
    ///
    /// # 量什么（先说再跑）
    ///
    /// 同一份工程上，[`NoteToneSource::with_track_gain`] 收到的那个标量
    /// （读回＝[`NoteToneSource::track_gain`]）与引擎快照投影写进
    /// `NoteSchedule::track_gain` 的那个标量，**逐位相同的 f32**（单位：线性增益）。
    ///
    /// 做法与 [`the_sfz_track_gain_is_bit_identical_to_the_engine_projection`] 逐条对齐
    /// —— 两条旁路用的是**同一个** [`track_gain_of`]（⇒ 同一个 `track_gain_scale`）。
    ///
    /// # 怎么变红
    ///
    /// 把 [`NoteToneSource`] 的注入换成自己换算（例如 `10f32.powf(db / 20.0)` 而丢掉
    /// 可闻门）⇒ 静音档 / 别轨独奏档当场不等，其余档末位 `to_bits()` 也不等。
    #[test]
    fn the_minimum_tone_track_gain_is_bit_identical_to_the_engine_projection() {
        for (volume_db, mute, solo_elsewhere) in [
            (0.0_f32, false, false),
            (-6.0, false, false),
            (-3.5, false, false),
            (0.0, true, false),
            (0.0, false, true),
        ] {
            let (project, lead) = fixture(volume_db, mute, solo_elsewhere);
            let any_solo = project.tracks.values().any(|track| track.solo);
            let ours = NoteToneSource::from_track(&project, lead, 48_000)
                .with_track_gain(track_gain_of(&project, lead, any_solo))
                .track_gain();
            let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须成快照");
            let theirs = snapshot
                .schedules()
                .get(&lead)
                .expect("本轨必须有调度表")
                .track_gain();
            println!(
                "[app-tone-gain/operator] volume_db={volume_db} mute={mute} \
                 solo_elsewhere={solo_elsewhere}: 最小音源注入={ours:.9} ({:#010x}) vs \
                 引擎投影={theirs:.9} ({:#010x})",
                ours.to_bits(),
                theirs.to_bits()
            );
            assert_eq!(
                ours.to_bits(),
                theirs.to_bits(),
                "⭐ 最小音源注入的轨增益必须与引擎投影 `NoteSchedule::track_gain` **逐位相同**\
                 （volume_db={volume_db} mute={mute} solo_elsewhere={solo_elsewhere}）"
            );
        }
    }

    /// ⭐ 判据（本单新增，`#71`）：**单位增益（`1.0`）⇒ 跳过乘法 ⇒ 逐位恒等**。
    ///
    /// 判定量：同一条轨、同一段帧区间上，`with_track_gain(1.0)` 的输出与**完全不注入**
    /// 的输出**逐位相同**（单位：线性幅度，单声道 8 秒）；并断言该段**非静音**
    /// （否则"全零 ＝ 全零"也算恒等，判据没有牙）。
    ///
    /// ⚠ 这条腿解释"为什么 `1.0` 那档的字节一位没变"。
    #[test]
    fn the_default_tone_track_gain_is_a_bit_exact_identity() {
        let (project, lead) = fixture(-6.0, false, false);
        let frames = 48_000 * 8;
        let mut implicit = NoteToneSource::from_track(&project, lead, 48_000);
        assert_eq!(
            implicit.track_gain(),
            1.0,
            "缺省必须是**单位增益**，⛔ 不是 0.0"
        );
        let mut explicit = NoteToneSource::from_track(&project, lead, 48_000).with_track_gain(1.0);
        assert_eq!(
            explicit.track_gain().to_bits(),
            1.0_f32.to_bits(),
            "`with_track_gain` 必须原样存下注入的门"
        );
        let mut without = vec![0.0_f32; frames];
        let mut with = vec![0.0_f32; frames];
        implicit
            .render_block(block_context(frames), &mut without)
            .expect("最小音源渲染不得失败");
        explicit
            .render_block(block_context(frames), &mut with)
            .expect("最小音源渲染不得失败");
        assert!(
            peak(&without) > 0.0,
            "该段必须真的出声（否则恒等没有判别力）"
        );
        assert_eq!(
            without
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            with.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
            "⭐ 单位增益 ⇒ 输出必须与**不注入**逐位相同（`1.0` 是 IEEE 恒等、跳过乘法）"
        );
    }

    /// ⭐ 判据（本单新增，`#71`）：**轨增益乘在本源输出之外**（⛔ 不是折进每个音符的幅度）。
    ///
    /// 判定量：`with_track_gain(g)` 的输出与"单位增益输出 × `g`"**逐位相同**
    /// （单位：线性幅度，单声道 8 秒，`g = track_gain(−6 dB)`）。
    ///
    /// ⚠ 与 `SfzTrackSource` 同形：增益乘在**源的输出**上，与引擎侧
    /// `NoteSchedule::track_gain` 一次乘法同口径。
    #[test]
    fn the_tone_source_output_is_the_unit_gain_output_times_the_track_gain() {
        let gain = yeban_engine::synth::track_gain(-6.0);
        assert!(gain > 0.0 && gain < 1.0, "夹具档必须是非恒等衰减");
        let (project, lead) = fixture(-6.0, false, false);
        let frames = 48_000 * 8;
        let mut raw_source = NoteToneSource::from_track(&project, lead, 48_000);
        let mut gated_source =
            NoteToneSource::from_track(&project, lead, 48_000).with_track_gain(gain);
        let mut raw = vec![0.0_f32; frames];
        let mut gated = vec![0.0_f32; frames];
        raw_source
            .render_block(block_context(frames), &mut raw)
            .expect("最小音源渲染不得失败");
        gated_source
            .render_block(block_context(frames), &mut gated)
            .expect("最小音源渲染不得失败");
        let expected: Vec<f32> = raw.iter().map(|value| value * gain).collect();
        println!(
            "[app-tone-gain/shape] 单位增益峰值 {:.9} × {gain:.9} = {:.9}，注入档峰值 {:.9}",
            peak(&raw),
            peak(&raw) * gain,
            peak(&gated)
        );
        assert!(peak(&raw) > 0.0 && peak(&gated) > 0.0, "两臂都必须出声");
        assert_eq!(
            gated
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            "⭐ 注入档必须 ＝ 单位增益输出 × 轨增益（逐位相同）"
        );
    }
}
