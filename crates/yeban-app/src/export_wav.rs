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
    let source_label: &'static str;
    if options.use_engine_source {
        let snapshot = EngineSnapshot::from_project(project, 1)
            .map_err(|error| WavExportError::Engine(error.to_string()))?;
        for track_id in renderable.clone() {
            sources.insert(
                track_id,
                Box::new(EngineTrackSource::new(&snapshot, track_id)?),
            );
        }
        source_label = "引擎乐器（SynthEngine::arm_from_snapshot + render_track）";
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
        source_label = "最小音源（方波 + 首尾包络；回退）";
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

    /// 判据 b（`NoMasterBus` ⇒ **失败且不留半成品**）：工程的主总线不在轨道表里时，
    /// 引擎装载在**落盘之前**失败，目标文件**不存在**。
    #[test]
    fn a_project_without_a_master_bus_fails_before_any_byte_is_written() {
        let mut project = yeban_model::samples::filled_project();
        // ⚠ 必须让**计划编译通过**、而**引擎装载失败**：路由图里保留主总线节点，
        // 但把主总线**轨道**从轨道表里删掉 ⇒ `arm_from_snapshot` 才会返回 `NoMasterBus`
        // （若把 master 改成不存在的 id，先失败的是计划编译，测不到本判据要测的那条路 —— 本机实测）。
        project.tracks.remove(&project.master_bus_track_id);
        let dir = std::env::temp_dir().join(format!(
            "yeban-wav-nomaster-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join("never.wav");

        let error = export_wav(&project, &target, WavExportOptions::default())
            .expect_err("没有主总线必须失败");
        assert!(
            matches!(error, WavExportError::Engine(_)),
            "必须是引擎装载失败（NoMasterBus）: {error:?}"
        );
        assert!(!target.exists(), "失败 ⇒ 一个字节都不许写出去");
        std::fs::remove_dir_all(&dir).ok();
    }
}
