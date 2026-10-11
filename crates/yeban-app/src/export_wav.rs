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
//! ## ⚠ 诚实登记：音源是**最小音源**，⛔ 不是引擎乐器
//!
//! 本切片之前 `yeban-app` **从未渲染过音频**（`RenderPlan`／`RenderOptions` 在 app 里 0 命中）。
//! 而"设备 → `AudioSource`"这条桥今天**只能用最小音源**打通，理由是一条**实测**：
//!
//! - `yeban-engine` 的 `SynthEngine::render_track(track, schedule, out)` 虽是 `pub`，
//!   但 `SynthEngine` 的**公开方法只有三个**（`new`／`voice_steals`／`set_steal_fade_frames`）
//!   ⇒ **"把快照装进合成器"的 arming 是 crate 内部的** ⇒ app 侧**够不着**引擎乐器。
//! - 引擎的**离线**入口（`device::render_callback` / `NullBackend::render`）确实与 cpal 同路径、
//!   无需声卡，但它的构造要 engine-host 机制（快照槽 ＋ 退役队列 ＋ 事件通道），
//!   且**样本只从回调缓冲取**（app 侧今天没有公开的取回路径）。
//!
//! ⇒ 本文件因此提供一个**确定性、按音符起止**的最小音源（廉价方波 ＋ 首尾线性包络，
//! 相位由**整数帧号**派生，⛔ 不用浮点累加），把**导出管路**先打通；
//! 引擎乐器的接入**另需一条引擎公开 API**，签名与理由写在下面（⛔ 本线不改 `yeban-engine`）：
//!
//! ```text
//! // 写给 `mod-engine` 线：
//! impl SynthEngine {
//!     /// 用一份工程快照武装/更新本合成器（轨道槽、鼓机、参数、调度表）。
//!     pub fn arm_from_snapshot(&mut self, snapshot: &EngineSnapshot) -> Result<(), SnapshotError>;
//! }
//! // `EngineSnapshot::from_project(&project, revision)` 已存在（`snapshot.rs:435`）。
//! ```
//!
//! ## "拒绝时不留半成品"
//!
//! `write_plain_wav` 自带该纪律（先 `check_match` 校验，再建文件）。本文件在**调用顺序**上
//! 不破坏它：**全部渲染与量化都在内存里完成**，只在最后一步调用一次 `write_plain_wav`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
}

impl Default for WavExportOptions {
    fn default() -> Self {
        Self {
            // 8 秒 48 kHz 立体声：足以听见演示工程的内容，且导出足够快。
            frames: 48_000 * 8,
            sample_rate: 48_000,
            channels: 2,
            seed: 1,
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
    /// 本次导出用的音源名（如实登记，⛔ 不假装是引擎乐器）。
    pub source: &'static str,
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
}

impl core::fmt::Display for WavExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Render(reason) => write!(formatter, "渲染失败: {reason}"),
            Self::Write(reason) => write!(formatter, "写出 WAV 失败: {reason}"),
            Self::Io(reason) => write!(formatter, "读取导出结果失败: {reason}"),
            Self::Empty(reason) => write!(formatter, "没有可导出的内容: {reason}"),
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
    if options.frames == 0 {
        return Err(WavExportError::Empty("请求的帧数为 0".to_owned()));
    }
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

    // ---- 2. 每条非 master 轨道一个源（无音符的轨道 ⇒ 静音源，保证每节点都有源）----
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    for track_id in project.tracks.keys() {
        if *track_id == project.master_bus_track_id {
            continue;
        }
        sources.insert(
            *track_id,
            Box::new(NoteToneSource::from_track(
                project,
                *track_id,
                options.sample_rate,
            )),
        );
    }

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
        source: "最小音源（方波 + 首尾包络；⛔ 非引擎乐器，见模块文档）",
    })
}
