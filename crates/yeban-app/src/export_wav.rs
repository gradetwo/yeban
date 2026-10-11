//! `--export-wav`：把工程**离线渲染**成一段可播放的 WAV（`[ROAD-M4-*]` 的导出族，与
//! `export_midi`／`export_als`／`export_logic` 并列）。
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! YebanProjectV1 ──▶ RenderPlan::compile(&project.routing_graph, project.master_bus_track_id, opts)
//!                 ──▶ RenderPlan::execute(sources: BTreeMap<EntityId, Box<dyn AudioSource>>)
//!                 ──▶ RenderOutput { samples: 交错 f32, frames, channels, digest }
//!                 ──▶ 逐个样本 quantize_i24(_, &mut DeterministicDitherRng)   ← TPDF 抖动
//!                 ──▶ PcmBuffer::Int24  ──▶  yeban_render::wav::write_plain_wav
//! ```
//!
//! ⛔ 本文件**不写 RIFF 头**（`write_plain_wav` 是唯一落盘点），⛔ 也**不另造渲染路径**
//! （一律 `RenderPlan`，与 `yeban-render` 的基准/门禁同源）。
//!
//! ## 音源：**引擎乐器**（默认），最小音源只作回退与对照
//!
//! 默认路径是**引擎乐器**，与实时路径**同一条装配序列**：
//!
//! ```text
//! EngineSnapshot::from_project(&project, 1)        // 工程 ⇒ 快照（既有）
//!       ⇒ SynthEngine::new(snapshot.sample_rate())
//!       ⇒ engine.arm_from_snapshot(&snapshot)      // ⭐ `mod-engine` 新增的公开入口
//!       ⇒ 每轨一个源：engine.render_track(track, snapshot.schedules().get(&track), &mut mono)
//! ```
//!
//! ⚠ **红线 7**：`arm_from_snapshot` **会分配**（`begin_snapshot` 增长区间表）⇒ 它的允许调用点是
//! **离线导出／控制线程装载工程／测试**，⛔ **绝不可**在音频回调里调用。本文件是离线路径 ✓。
//! ⚠ 没有主总线时 `arm_from_snapshot` 返回 `SnapshotError::NoMasterBus` ⇒ 本文件在**落盘之前**
//! 如实失败（⛔ 不 `unwrap`、⛔ 不留半成品）。
//!
//! ⭐ **SFZ 音源**（`export_wav_with_sfz` 的 `Some(sfz)`）：带 `--sfz` 时每条可渲染轨用
//! [`crate::sfz_load::SfzTrackSource`] ⇒ 导出的声音来自该 SFZ 的区域；样本数据是**内存正弦**
//! （⛔ 本仓无采样文件读取）⇒ 报告行 `source=` 明说。
//!
//! `WavExportOptions::use_engine_source = false` 时才用**最小音源**（确定性方波 ＋ 首尾包络，
//! 相位由整数帧号派生）：它是**判据的对照**与极端回退，报告行里的 `source=` 字段**明说用了哪一个**
//! （⛔ 不让人误以为听到了乐器）。
//!
//! ## "拒绝时不留半成品"
//!
//! `write_plain_wav` 自带该纪律（先 `check_match` 校验，再建文件）。本文件在**调用顺序**上
//! 不破坏它：**全部渲染与量化都在内存里完成**，只在最后一步调用一次 `write_plain_wav`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, SynthEngine};
use yeban_model::{EntityId, YebanProjectV1};
use yeban_render::dither::{PcmBuffer, quantize_i24};
use yeban_render::render::{AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan};
use yeban_render::rf64::PcmFormat;
use yeban_render::rng::DeterministicDitherRng;
use yeban_render::wav::write_plain_wav;

/// 导出参数（秒数／采样率／声道／种子）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavExportOptions {
    /// 总帧数。
    pub frames: u64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 确定性种子（[`ARCH-DET-001]`：渲染不引入真熵源）。
    pub seed: u64,
    /// `true`（默认）= 用**引擎乐器**（`SynthEngine::arm_from_snapshot`）；
    /// `false` = 用**最小音源**回退（方波；判据用它做对照）。
    pub use_engine_source: bool,
}

impl Default for WavExportOptions {
    fn default() -> Self {
        Self {
            // 8 秒 48 kHz 立体声：足以听见演示工程的内容，且导出足够快。
            frames: 48_000 * 8,
            sample_rate: 48_000,
            channels: 2,
            seed: 1,
            use_engine_source: true,
        }
    }
}

/// 导出报告（形态照 `MidiExportReport`：路径／帧数／采样率／声道／位深／字节数）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WavExportReport {
    /// 落盘路径。
    pub path: PathBuf,
    /// 总帧数。
    pub frames: u64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 位深（bit）。
    pub bits: u16,
    /// 文件字节数。
    pub bytes: u64,
    /// 渲染输出的位级摘要（十六进制；来自 `RenderOutput::digest`）。
    pub digest: String,
    /// 本次导出用的音源名（如实登记；文件样本会写出路径／帧数／采样率）。
    pub source: String,
}

impl WavExportReport {
    /// 一行可读摘要（CLI 打印它）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "WAV 已写出: {}（{} 帧 / {} Hz / {} 声道 / {} bit / {} 字节 / 音源 {} / digest {}）",
            self.path.display(),
            self.frames,
            self.sample_rate,
            self.channels,
            self.bits,
            self.bytes,
            self.source,
            self.digest
        )
    }
}

/// 导出失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavExportError {
    /// 渲染计划编译或执行失败（图不自洽 / 端点缺失 / 选项非法 ……）。
    Render(String),
    /// WAV 写出失败（含"拒绝时不落字节"的那类拒绝）。
    Write(String),
    /// 报告阶段读取文件元数据失败。
    Io(String),
    /// 请求了 0 帧（没有可导出的内容）。
    Empty(String),
    /// 引擎装载失败（快照投影 / `arm_from_snapshot`；例如工程没有主总线 ⇒ `NoMasterBus`）。
    Engine(String),
}

impl core::fmt::Display for WavExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Render(reason) => write!(formatter, "渲染失败: {reason}"),
            Self::Write(reason) => write!(formatter, "写出 WAV 失败: {reason}"),
            Self::Io(reason) => write!(formatter, "读取导出结果失败: {reason}"),
            Self::Empty(reason) => write!(formatter, "没有可导出的内容: {reason}"),
            Self::Engine(reason) => write!(formatter, "引擎装载失败: {reason}"),
        }
    }
}

impl core::error::Error for WavExportError {}

/// **最小音源**：按音符起止发出廉价方波（首尾各 64 帧线性包络），相位由**整数帧号**派生。
///
/// ⛔ 这不是引擎乐器（见模块文档的诚实登记）；它的职责是"把导出管路打通且**可听**"，
/// 并保持确定性：同一工程 ＋ 同一选项 ⇒ 逐位相同的输出。
struct NoteToneSource {
    /// `(绝对起始帧, 时值帧, 音高, 力度 0..=127)`。
    notes: Vec<(u64, u64, u8, u8)>,
    sample_rate: u32,
}

/// 首尾包络长度（帧）：避免方波起止的爆音。
const TONE_FADE_FRAMES: u64 = 64;

impl NoteToneSource {
    /// 从一条轨道的全部片段收集音符（绝对帧 = 片段起点 ＋ 音符起点，960 PPQ 换算）。
    fn from_track(project: &YebanProjectV1, track_id: EntityId, sample_rate: u32) -> Self {
        let mut notes = Vec::new();
        if let Some(track) = project.tracks.get(&track_id) {
            for placement in track.clips.values() {
                let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                    continue;
                };
                let Some(map) = entry.content.notes() else {
                    continue;
                };
                for note in map.values() {
                    // 960 PPQ ⇒ 帧：`tick * sample_rate / 960`（整数运算，⛔ 不引入浮点漂移）。
                    let start_tick = placement.start_tick + note.start_tick;
                    let start = start_tick.saturating_mul(u64::from(sample_rate)) / 960;
                    let duration =
                        (note.duration_ticks.max(1)).saturating_mul(u64::from(sample_rate)) / 960;
                    notes.push((start, duration.max(1), note.pitch, note.velocity));
                }
            }
        }
        notes.sort_unstable();
        Self { notes, sample_rate }
    }

    /// 该帧上所有正在发声的音符之和（`[-1, 1]` 之外的削波交给渲染层的母线）。
    fn sample_at(&self, frame: u64) -> f32 {
        let mut value = 0.0_f32;
        for (start, duration, pitch, velocity) in &self.notes {
            let end = start + duration;
            if frame < *start || frame >= end {
                continue;
            }
            let offset = frame - start;
            // 十二平均律：一个八度 12 半音，A4 = 69 = 440 Hz（幂运算只算一次/音符/帧）。
            let frequency = 440.0_f32 * 2.0_f32.powf((f32::from(*pitch) - 69.0) / 12.0);
            let period = (f64::from(self.sample_rate) / f64::from(frequency)).max(2.0);
            let phase = (offset as f64 % period) / period;
            let square = if phase < 0.5 { 1.0_f32 } else { -1.0_f32 };
            let fade_in = offset.min(TONE_FADE_FRAMES) as f32 / TONE_FADE_FRAMES as f32;
            let fade_out = (end - frame).min(TONE_FADE_FRAMES) as f32 / TONE_FADE_FRAMES as f32;
            value += square * fade_in.min(fade_out) * (f32::from(*velocity) / 127.0) * 0.2;
        }
        value
    }
}

/// **引擎乐器音源**：每条轨道一个 [`SynthEngine`]，用**同一条离线装配路径**
/// （`SynthEngine::arm_from_snapshot`）武装，再按轨渲染（`render_track`）。
///
/// ⚠ 每个源持有**自己的**引擎：`RenderPlan::execute` 会把源分发到工作线程（`threads`），
/// 共享一个 `&mut SynthEngine` 需要锁，而"一个源一个引擎"没有锁也没有跨线程共享。
/// ⚠ 红线 7：`arm_from_snapshot` **会分配**（`begin_snapshot` 增长区间表）⇒ 只在**离线导出**
/// 里调用，⛔ 绝不在音频回调里调用（本文件是离线路径）。
struct EngineTrackSource {
    /// 已武装的合成器（本源独占）。
    engine: SynthEngine,
    /// 本源负责的轨道。
    track: EntityId,
    /// 该轨的调度表（来自同一份快照；`None` ⇒ 该轨静音，与 `render_track` 的文档一致）。
    schedule: Option<NoteSchedule>,
    /// 单声道暂存（`render_track` 输出"声相之前"的单声道）。
    mono: Vec<f32>,
}

impl EngineTrackSource {
    /// 从快照武装一个轨道源。
    ///
    /// # Errors
    ///
    /// `arm_from_snapshot` 的拒绝（含 `NoMasterBus`）。
    fn new(snapshot: &EngineSnapshot, track: EntityId) -> Result<Self, WavExportError> {
        let mut engine = SynthEngine::new(snapshot.sample_rate());
        // ⛔ 不 `unwrap`：没有主总线时如实失败（调用方据此在**落盘之前**退出）。
        engine
            .arm_from_snapshot(snapshot)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        Ok(Self {
            engine,
            track,
            schedule: snapshot.schedules().get(&track).cloned(),
            mono: Vec::new(),
        })
    }
}

impl AudioSource for EngineTrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        if self.mono.len() < frames {
            self.mono.resize(frames, 0.0);
        }
        // `render_track` 复用内部游标推进（与实时路径同一个函数）。
        self.engine
            .render_track(self.track, self.schedule.as_ref(), &mut self.mono[..frames]);
        out.fill(0.0);
        for index in 0..frames {
            let value = self.mono[index];
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        Ok(())
    }
}

impl AudioSource for NoteToneSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        out.fill(0.0);
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        for index in 0..frames {
            let frame = context.first_frame + index as u64;
            let value = self.sample_at(frame);
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        Ok(())
    }
}

/// 一条轨道的音符，按 960 PPQ 换算成 `(起始帧, 时值帧, 音高, 力度)`（⛔ 与最小音源同一口径）。
fn track_notes_in_frames(
    project: &YebanProjectV1,
    track_id: EntityId,
    sample_rate: u32,
) -> Vec<(u64, u64, u8, u8)> {
    let mut notes = Vec::new();
    if let Some(track) = project.tracks.get(&track_id) {
        for placement in track.clips.values() {
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let Some(map) = entry.content.notes() else {
                continue;
            };
            for note in map.values() {
                let start_tick = placement.start_tick + note.start_tick;
                let start = start_tick.saturating_mul(u64::from(sample_rate)) / 960;
                let duration =
                    (note.duration_ticks.max(1)).saturating_mul(u64::from(sample_rate)) / 960;
                notes.push((start, duration.max(1), note.pitch, note.velocity));
            }
        }
    }
    notes.sort_unstable();
    notes
}

/// 把工程离线渲染成 WAV（**先全量渲染与量化，最后一步才落盘**）。
///
/// # Errors
///
/// - [`WavExportError::Empty`]：`options.frames == 0`；
/// - [`WavExportError::Render`]：计划编译/执行失败；
/// - [`WavExportError::Write`]：`write_plain_wav` 拒绝（此时**不留任何字节**）。
pub fn export_wav(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
) -> Result<WavExportReport, WavExportError> {
    export_wav_with_sfz(project, path, options, None)
}

/// 与 [`export_wav`] 相同，但可以指定一份**已装载的 SFZ 乐器**（`--sfz <path>` 的接线）。
///
/// 给了 SFZ ⇒ 每条**可渲染**轨用 [`crate::sfz_load::SfzTrackSource`]（SFZ 区域发声，
/// 样本是内存正弦）；⛔ 否则与 [`export_wav`] 逐位相同。
///
/// # Errors
///
/// 与 [`export_wav`] 相同。
pub fn export_wav_with_sfz(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
    sfz: Option<std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>>,
) -> Result<WavExportReport, WavExportError> {
    export_wav_with_sfz_file(project, path, options, sfz.map(|i| (i, None)))
}

/// 与 [`export_wav_with_sfz`] 相同，但可同时给出 **SFZ 文件路径** ⇒ `sample=` 会被**真读**。
///
/// 给了路径 ⇒ 每条音符按它的区域解析 `sample=`（**只支持 `read_plain_wav` 能读的 PCM WAV**）；
/// 读不到 ⇒ 如实回退到内存正弦并把原因写进 `source=`。⛔ 给 `None` 时与
/// [`export_wav_with_sfz`] 逐位相同（内存正弦）。
///
/// # Errors
///
/// 与 [`export_wav`] 相同。
pub fn export_wav_with_sfz_file(
    project: &YebanProjectV1,
    path: &Path,
    options: WavExportOptions,
    sfz: Option<(
        std::sync::Arc<yeban_engine::sfz_instrument::SfzInstrument>,
        Option<PathBuf>,
    )>,
) -> Result<WavExportReport, WavExportError> {
    if options.frames == 0 {
        return Err(WavExportError::Empty("请求的帧数为 0".to_owned()));
    }
    // ---- 0. 引擎装载（**慢路径校验先行**：引擎文档要求先做慢路径校验，再进入渲染循环）----
    //
    // 为什么提前：`arm_from_snapshot` 在**没有可武装的轨/音色/鼓**时返回 `NoMasterBus`；
    // 它是"这个工程能不能用引擎渲染"的判定，理应在计划编译与任何 I/O 之前发生
    // ⇒ 失败时一个字节都不会落盘，也不会先被计划层的次生错误（例如"没有注册样本源"）掩盖。
    // 带 SFZ ⇒ 走 SFZ 音源（⛔ 不需要引擎快照；SFZ 自己的解析在离线路径里做）。
    let snapshot = if options.use_engine_source && sfz.is_none() {
        let snapshot = EngineSnapshot::from_project(project, 1)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
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
    let mut plan = RenderPlan::compile(
        &project.routing_graph,
        project.master_bus_track_id,
        options_render,
    )
    .map_err(|error| WavExportError::Render(error.to_string()))?;

    // ---- 2. 每条非 master 轨道一个源 ----
    //
    // 默认走**引擎乐器**：先投影快照（`EngineSnapshot::from_project`），再按轨武装
    // （`arm_from_snapshot`，与实时路径同一条装配序列）。任何一步失败都在**落盘之前**返回
    // ⇒ 不留半成品。`use_engine_source = false` 时用最小音源（判据的对照 + 极端回退）。
    // 计划**自己**的节点表是唯一权威：`RenderPlan::nodes()`（图里存在但不可达的节点会被剪枝，
    // 给它塞源会得到"节点 … 不在渲染计划里（未知或被剪枝）" —— 本机实测踩过两次）。
    let renderable: Vec<EntityId> = plan
        .nodes()
        .into_iter()
        .filter(|id| *id != project.master_bus_track_id && project.tracks.contains_key(id))
        .collect();
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let source_label: String = if let Some((instrument, sfz_path)) = sfz.as_ref() {
        let mut labels: Vec<String> = Vec::new();
        for track_id in renderable.clone() {
            let notes = track_notes_in_frames(project, track_id, options.sample_rate);
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
            };
            labels.push(track_source.sample_source());
            sources.insert(track_id, Box::new(track_source));
        }
        labels
            .first()
            .map_or_else(|| "sfz(无区域)".to_owned(), |label| format!("sfz({label})"))
    } else if let Some(snapshot) = snapshot.as_ref() {
        for track_id in renderable.clone() {
            sources.insert(
                track_id,
                Box::new(EngineTrackSource::new(snapshot, track_id)?),
            );
        }
        "引擎乐器（SynthEngine::arm_from_snapshot + render_track）".to_owned()
    } else {
        for track_id in renderable.clone() {
            sources.insert(
                track_id,
                Box::new(NoteToneSource::from_track(
                    project,
                    track_id,
                    options.sample_rate,
                )),
            );
        }
        "最小音源（方波 + 首尾包络；回退）".to_owned()
    };

    // ---- 3. 执行（全部在内存里）----
    let output = plan
        .execute(sources)
        .map_err(|error| WavExportError::Render(error.to_string()))?;

    // ---- 4. 逐个样本量化（TPDF 抖动；位深 24 bit 整数 PCM）----
    let mut rng =
        DeterministicDitherRng::new(u32::try_from(options.seed & u64::from(u32::MAX)).unwrap_or(1));
    let samples: Vec<i32> = output
        .samples
        .iter()
        .map(|sample| quantize_i24(*sample, &mut rng))
        .collect();
    let buffer = PcmBuffer::Int24(samples);
    // 用**构造器**而不是逐字段字面量：`PcmFormat` 将来加字段时这里不会漏（本机实测踩过一次）。
    let format = PcmFormat::integer(
        u16::try_from(options.channels).unwrap_or(u16::MAX),
        options.sample_rate,
        24,
    );

    // ---- 5. 最后一步：一次落盘（拒绝 ⇒ 不留半成品）----
    write_plain_wav(path, &format, &buffer)
        .map_err(|error| WavExportError::Write(error.to_string()))?;

    let bytes = std::fs::metadata(path)
        .map_err(|error| WavExportError::Io(error.to_string()))?
        .len();
    let digest = output
        .digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(WavExportReport {
        path: path.to_path_buf(),
        frames: output.frames,
        sample_rate: format.sample_rate,
        channels: options.channels,
        bits: format.bits_per_sample,
        bytes,
        digest,
        source: source_label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 a（**真的换了音源**）：引擎乐器路径非静音，且与最小音源路径**逐字节不同**。
    ///
    /// ⚠ 为什么断言"逐字节不同"而不是"更好听"：这是**可机械判定**的判别量；
    /// 两个不同音源在同一工程上产出同一串 24-bit 抖动样本的概率可忽略（若真相同，判据会红并要求解释）。
    #[test]
    fn the_engine_source_is_not_silent_and_differs_from_the_fallback_tone() {
        let project = yeban_model::samples::filled_project();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-engine-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let engine_path = dir.join("engine.wav");
        let tone_path = dir.join("tone.wav");

        let engine = export_wav(&project, &engine_path, WavExportOptions::default())
            .expect("引擎乐器导出必须成功");
        let fallback = export_wav(
            &project,
            &tone_path,
            WavExportOptions {
                use_engine_source: false,
                ..WavExportOptions::default()
            },
        )
        .expect("回退音源导出必须成功");

        assert!(
            engine.source.contains("引擎乐器"),
            "报告必须明说音源: {}",
            engine.source
        );
        assert!(
            fallback.source.contains("最小音源"),
            "报告必须明说回退: {}",
            fallback.source
        );

        let (_, engine_buffer) =
            yeban_render::wav::read_plain_wav(&engine_path).expect("引擎版可回读");
        let engine_nonzero = match &engine_buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => {
                samples.iter().filter(|sample| **sample != 0).count()
            }
            other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
        };
        assert!(
            engine_nonzero > 0,
            "引擎乐器路径必须非静音（非零样本数 = {engine_nonzero}）"
        );
        assert_ne!(
            std::fs::read(&engine_path).expect("读引擎版"),
            std::fs::read(&tone_path).expect("读回退版"),
            "引擎乐器与最小音源必须产出不同的字节（否则本判据没有判别力）"
        );
        println!(
            "[wav-engine] 引擎版非零样本 {engine_nonzero}；两版字节数 {} vs {}",
            engine.bytes, fallback.bytes
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据（`sample=` **真读文件**）：把一条**常量**样本（`0.25` 的 24-bit PCM）当区域样本，
    /// 导出的声音**确实来自该文件** —— 判定量 = 该音符前 2048 个样本的**极差**（max−min）：
    /// 常量样本 ⇒ 极差**极小**；内存正弦 ⇒ 极差**大**。
    ///
    /// ⚠ 这样命名判定量，是为了同时排除"换了但没声音"（极差为 0 且全 0 ⇒ 另断言峰值 > 0）
    /// 与"其实还是正弦"（极差会大）两种假象。
    #[test]
    fn a_sample_file_named_by_the_region_is_really_read_and_drives_the_output() {
        let project = yeban_model::samples::filled_project();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-sample-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        // ① 造一条**常量** 24-bit 单声道样本（值 = 满量程的 1/4 ⇒ 归一化 0.25）
        let scale = yeban_render::dither::BitDepth::Int24.full_scale_max() / 4;
        let sample_path = dir.join("const.wav");
        let buffer = yeban_render::dither::PcmBuffer::Int24(vec![scale; 4_800]);
        let format = yeban_render::rf64::PcmFormat::integer(1, 48_000, 24);
        yeban_render::wav::write_plain_wav(&sample_path, &format, &buffer).expect("写样本");
        // ② SFZ：每个音高一条区域，sample 指向该文件
        let mut pitches: Vec<u8> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        pitches.sort_unstable();
        pitches.dedup();
        let text: String = pitches
            .iter()
            .map(|pitch| format!("<region> sample=const.wav key={pitch}\n"))
            .collect();
        let sfz_path = dir.join("keys.sfz");
        std::fs::write(&sfz_path, &text).expect("写 SFZ");
        let instrument = yeban_engine::sfz_instrument::SfzInstrument::load(
            text,
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("装载");
        // ③ 导两次：带文件样本 vs 只有内存正弦
        let file_path = dir.join("file.wav");
        let memory_path = dir.join("memory.wav");
        let with_file = export_wav_with_sfz_file(
            &project,
            &file_path,
            WavExportOptions::default(),
            Some((
                std::sync::Arc::new(instrument.clone()),
                Some(sfz_path.clone()),
            )),
        )
        .expect("带文件样本的导出");
        let with_memory = export_wav_with_sfz(
            &project,
            &memory_path,
            WavExportOptions::default(),
            Some(std::sync::Arc::new(instrument)),
        )
        .expect("内存正弦的导出");
        println!(
            "[wav-file-sample] file source={} / memory source={}",
            with_file.source, with_memory.source
        );
        assert!(
            with_file.source.contains("file("),
            "来源必须写出文件路径与帧数: {}",
            with_file.source
        );
        assert!(
            with_memory.source.contains("in-memory-sine"),
            "内存版来源必须明说: {}",
            with_memory.source
        );

        let spread_of = |path: &std::path::Path| -> (f32, f32) {
            let (format, buffer) = yeban_render::wav::read_plain_wav(path).expect("可回读");
            let channels = usize::from(format.channels.max(1));
            let samples: Vec<f32> = match &buffer {
                yeban_render::dither::PcmBuffer::Int24(values) => values
                    .iter()
                    .step_by(channels)
                    .map(|value| (*value as f32) / 8_388_608.0)
                    .collect(),
                other => panic!("24-bit 整数: {other:?}"),
            };
            let window = &samples[..samples.len().min(2048)];
            let peak = window
                .iter()
                .fold(0.0_f32, |acc, value| acc.max(value.abs()));
            let (min, max) = window.iter().fold((f32::MAX, f32::MIN), |(lo, hi), value| {
                (lo.min(*value), hi.max(*value))
            });
            (max - min, peak)
        };
        let (file_spread, file_peak) = spread_of(&file_path);
        let (memory_spread, _) = spread_of(&memory_path);
        println!(
            "[wav-file-sample] 文件样本极差 {file_spread:.6}（峰值 {file_peak:.6}）ｖｓ 内存正弦极差 {memory_spread:.6}"
        );
        assert!(file_peak > 0.0, "文件样本版不许是静音（峰值 {file_peak}）");
        assert!(
            file_spread < 0.01,
            "常量样本 ⇒ 前 2048 样本极差必须极小（实际 {file_spread}）"
        );
        assert!(
            memory_spread > file_spread * 10.0,
            "内存正弦的极差必须显著更大（正弦 {memory_spread} ｖｓ 常量 {file_spread}）"
        );
        assert_ne!(
            std::fs::read(&file_path).expect("读文件版"),
            std::fs::read(&memory_path).expect("读内存版"),
            "文件样本与内存正弦必须逐字节不同"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据（`--sfz` 接线）：**同一工程**带 SFZ 与不带 SFZ 导出的 WAV **逐字节不同**，
    /// 且 SFZ 版**非静音**（⛔ 不是"换了但没声音"）。
    ///
    /// ⚠ 为什么这份 SFZ 按**工程里实际出现的音高**逐键写区域：区域不匹配就会是静音，
    /// 那样"可区分"就变成"一个响一个哑"，测不出"真的用 SFZ 发声"（本机刻意这样构造）。
    #[test]
    fn an_sfz_export_differs_from_the_default_export_and_is_not_silent() {
        let project = yeban_model::samples::filled_project();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-sfz-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        // 工程里的每个音高一条区域 ⇒ 保证 SFZ 真的会发声。
        let mut pitches: Vec<u8> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values().map(|note| note.pitch))
            .collect();
        pitches.sort_unstable();
        pitches.dedup();
        assert!(!pitches.is_empty(), "夹具必须有音符");
        let text: String = pitches
            .iter()
            .map(|pitch| format!("<region> sample=a.wav key={pitch}\n"))
            .collect();
        let instrument = yeban_engine::sfz_instrument::SfzInstrument::load(
            text,
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("夹具 SFZ 必须可装载");
        assert_eq!(instrument.region_count(), pitches.len(), "区域数 == 音高数");

        let plain_path = dir.join("plain.wav");
        let sfz_path = dir.join("sfz.wav");
        let plain =
            export_wav(&project, &plain_path, WavExportOptions::default()).expect("默认导出");
        let with_sfz = export_wav_with_sfz(
            &project,
            &sfz_path,
            WavExportOptions::default(),
            Some(std::sync::Arc::new(instrument)),
        )
        .expect("带 SFZ 导出");
        println!(
            "[wav-sfz] 默认 source={} / 字节 {}；SFZ source={} / 字节 {}",
            plain.source, plain.bytes, with_sfz.source, with_sfz.bytes
        );
        assert!(
            with_sfz.source.contains("sfz"),
            "报告必须明说走了 SFZ: {}",
            with_sfz.source
        );
        assert!(
            with_sfz.source.contains("in-memory-sine"),
            "必须明说样本来源（内存正弦）: {}",
            with_sfz.source
        );
        let (_, buffer) = yeban_render::wav::read_plain_wav(&sfz_path).expect("可回读");
        let nonzero = match &buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => {
                samples.iter().filter(|sample| **sample != 0).count()
            }
            other => panic!("24-bit 整数: {other:?}"),
        };
        assert!(nonzero > 0, "SFZ 版必须非静音（非零样本 {nonzero}）");
        assert_ne!(
            std::fs::read(&plain_path).expect("读默认版"),
            std::fs::read(&sfz_path).expect("读 SFZ 版"),
            "带 SFZ 与不带 SFZ 必须逐字节不同（否则接线没生效）"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b（⭐ **语义已按引擎侧修复更新**）：主总线**不在轨道表**里、但工程**仍有可武装的轨道**
    /// ⇒ 导出**应当成功**（⛔ 不再期望 `NoMasterBus`）。
    ///
    /// ## 为什么这条判据的期望被反转过（第 617 轮更正）
    ///
    /// 我最初把它写成"主总线不在 `tracks()` ⇒ `NoMasterBus` ⇒ 失败"。`mod-engine` 随后用一个
    /// 最小夹具发现 `arm_from_snapshot` 的 master 守卫**与 `rt.rs` 的语义不一致**：实时侧只把 master
    /// 当**过滤器**（`id != master`），所以"要求 master 出现在 `tracks()` 里"会**误拒一个合法的最小工程**；
    /// 它已改成"只拒绝**没有可武装的轨/音色/鼓**的快照"（`6b0ef4f9`）。
    /// ⇒ 本判据当时断言的是**被修掉的错误行为**，**它变红正是修复生效的证据**；现在把期望改成新语义。
    #[test]
    fn a_project_whose_master_is_absent_from_the_track_table_still_renders() {
        let mut project = yeban_model::samples::filled_project();
        // 路由图里保留主总线节点（计划仍可编译），只把主总线**轨道**从轨道表里删掉。
        project.tracks.remove(&project.master_bus_track_id);
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-nomaster-track-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join("still-renders.wav");

        let report = export_wav(&project, &target, WavExportOptions::default())
            .expect("仍有可武装的轨道 ⇒ 必须成功（新语义）");
        assert!(
            report.source.contains("引擎乐器"),
            "音源: {}",
            report.source
        );
        assert!(target.exists(), "成功 ⇒ 文件必须存在");
        let (_, buffer) = yeban_render::wav::read_plain_wav(&target).expect("可回读");
        let nonzero = match &buffer {
            yeban_render::dither::PcmBuffer::Int24(samples) => {
                samples.iter().filter(|sample| **sample != 0).count()
            }
            other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
        };
        assert!(
            nonzero > 0,
            "仍有可武装的轨道 ⇒ 不许是静音（非零样本 {nonzero}）"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 c（**失败路径的正确触发条件** ⇒ 失败且不留半成品）：工程里**没有可武装的轨/音色/鼓**
    /// 时，`arm_from_snapshot` 返回 `NoMasterBus`，导出在**落盘之前**失败，目标文件**不存在**。
    ///
    /// ⚠ **重叠声明**：`mod-engine` 的夹具已经断言过 `NoMasterBus` **返回值**本身；
    /// 本判据保留的理由是它覆盖的是**导出这一侧的后果**（`WavExportError::Engine` ＋ **一个字节都不落盘**），
    /// ⛔ 不是重复断言返回值。
    #[test]
    fn a_project_with_nothing_armable_reports_no_master_bus_and_writes_nothing() {
        let mut project = yeban_model::samples::filled_project();
        // 清空**全部**轨道（含主总线）：快照里没有任何可武装的轨/音色/鼓。
        // ⚠ 实测更正：只留主总线**不够** —— 引擎的守卫把主总线自己也算"可武装"，
        // 于是它不再拒绝（随后才在计划执行时因缺源而失败）⇒ 触发条件必须是"一个可武装项都没有"。
        // 路由图保持原样 ⇒ 计划仍可编译（⛔ 不靠改坏图来制造失败）。
        project.tracks.clear();
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-nothing-armable-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join("never.wav");

        let error = export_wav(&project, &target, WavExportOptions::default())
            .expect_err("没有可武装的轨 ⇒ 必须失败");
        assert!(
            matches!(error, WavExportError::Engine(_)),
            "必须是引擎装载失败（NoMasterBus）: {error:?}"
        );
        assert!(!target.exists(), "失败 ⇒ 一个字节都不许写出去");
        std::fs::remove_dir_all(&dir).ok();
    }
}
