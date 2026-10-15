//! SFZ 乐器**装载**与**离线音符探测**（用户："加载一个 SFZ ⇒ 出声"）。
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! 文件文本 ──▶ SfzInstrument::load(text, &ParseLimits)   // 引擎的闭包式装载器（控制侧）
//!          ──▶ region_count()                            // 报告里的"区域数"
//!          ──▶ InMemorySample::sine(440 Hz, engine_hz, n) // ⚠ 样本数据是**内存正弦**
//!          ──▶ render_note_into(&sample, note, vel, &mut out, engine_hz)
//!          ──▶ 帧数 ＋ 能量（Σx²）                        // 可机械判定的"非静音"证据
//! ```
//!
//! ## ⚠ 三条诚实登记（⛔ 不假装）
//!
//! 1. **样本文件会被真读**：`sample=` 指到的文件经 [`read_sample_file`]（→
//!    `yeban_render::wav::read_plain_wav_mono_f32`）解码成单声道 `f32`，报告行里写出
//!    `file(<path> frames=… rate=…)`。⚠ 两条**仍然成立**的限制：只支持该读取器能读的 PCM WAV；
//!    读不到时**如实回退**到内存正弦 [`InMemorySample::sine`] 并把原因写进来源文案
//!    （探测那一条 [`probe_note_energy`] 本身就用内存正弦 —— 它不解析 `sample=`）。
//! 2. **红线 7**：`render_note_into` **每次调用都解析 ⇒ 分配** ⇒ 只能**离线/控制侧**调用，
//!    ⛔ 绝不可在音频回调里调用（与 `arm_from_snapshot` 同一条边）。
//! 3. **已接到导出/录音路径**：`--sfz` ＋ `--export-wav` / `--record-wav` 时，SFZ 音色经
//!    [`SfzTrackSource`] 进入音源链（报告里的 `wired_to_export=yes`）。单独给 `--sfz` 时仍是
//!    `wired_to_export=false` —— 那是"**没请求**导出/录音"，⛔ 不是"没接线"。

use std::path::{Path, PathBuf};

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument};
use yeban_sfz::parser::ParseLimits;

/// 探测用的样本源（440 Hz 内存正弦）的文案——报告行里逐字使用。
pub const SAMPLE_SOURCE: &str = "in-memory-sine(440Hz)";

/// 探测音符的默认音高（A4）。
pub const DEFAULT_PROBE_NOTE: u8 = 60;

/// 探测用的力度。
pub const DEFAULT_PROBE_VELOCITY: u8 = 100;

/// 装载失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SfzLoadError {
    /// 读文件失败。
    Read(String),
    /// SFZ 文本非法（语法/超限）—— 引擎原样透出的裁决。
    Parse(String),
    /// 样本文件读取/解码失败（本仓只支持 `read_plain_wav` 能读的 PCM WAV）。
    Sample(String),
}

impl core::fmt::Display for SfzLoadError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Read(reason) => write!(formatter, "读 SFZ 失败: {reason}"),
            Self::Parse(reason) => write!(formatter, "SFZ 非法: {reason}"),
            Self::Sample(reason) => write!(formatter, "样本读取失败: {reason}"),
        }
    }
}

impl core::error::Error for SfzLoadError {}

/// 一次音符探测的读数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzProbeReading {
    /// 探测音高。
    pub note: u8,
    /// 探测力度。
    pub velocity: u8,
    /// `render_note_into` 写入的帧数（`0` = 没有匹配区域，**合法**：该音符不发声）。
    pub frames: usize,
    /// 输出能量 `Σx²`（`0.0` = 静音）。
    pub energy: f32,
}

impl SfzProbeReading {
    /// `true` 表示这次探测**真的出了声**（有帧且能量大于 0）。
    #[must_use]
    pub fn is_audible(&self) -> bool {
        self.frames > 0 && self.energy > 0.0
    }
}

/// 装载报告（形态照 `MidiImportReport`：路径／字节／区域数／探测读数／诚实字段）。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzLoadReport {
    /// SFZ 文件路径。
    pub path: PathBuf,
    /// 文件字节数。
    pub bytes: u64,
    /// 区域数（`SfzInstrument::region_count`）。
    pub regions: usize,
    /// 探测读数（`probe_note = None` 时为 `None`）。
    pub probe: Option<SfzProbeReading>,
    /// **探测那一步**的样本来源文案（恒为 [`SAMPLE_SOURCE`] —— `probe_note_energy` 不解析
    /// `sample=`）。导出/录音真正用的样本来源由 `export_wav` 报告行的 `source=` 给出。
    pub sample_source: String,
    /// 是否已经接到导出/录音的音源上：默认 `false`（本次装载**没有请求**导出/录音），
    /// 调用方（CLI 的 `run_batch`）在真的要用它导出/录音时改写为 `true`。
    pub wired_to_export: bool,
}

impl SfzLoadReport {
    /// 一行可读摘要（CLI 打印它）。
    #[must_use]
    pub fn summary(&self) -> String {
        let probe = match self.probe {
            Some(reading) => format!(
                "{}:{}:{}:{}",
                reading.note, reading.velocity, reading.frames, reading.energy
            ),
            None => "-".to_owned(),
        };
        format!(
            "loaded-sfz: path={} bytes={} regions={} sample_source={} probe(note:vel:frames:energy)={probe} wired_to_export={}",
            self.path.display(),
            self.bytes,
            self.regions,
            self.sample_source,
            if self.wired_to_export { "yes" } else { "no" },
        )
    }
}

/// 装载一份 SFZ 并（可选）探测一个音符。
///
/// # Errors
///
/// 读文件失败 / SFZ 文本非法。⛔ 本函数**不写任何文件** ⇒ 失败时不会留下半成品。
pub fn load_sfz(
    path: &Path,
    probe_note: Option<u8>,
    engine_hz: f32,
) -> Result<SfzLoadReport, SfzLoadError> {
    load_sfz_with_instrument(path, probe_note, engine_hz).map(|(report, _instrument)| report)
}

/// 与 [`load_sfz`] 相同，但**同时交出乐器本体**（`--sfz` ＋ `--export-wav` 接线要用它发声）。
///
/// # Errors
///
/// 与 [`load_sfz`] 相同。
pub fn load_sfz_with_instrument(
    path: &Path,
    probe_note: Option<u8>,
    engine_hz: f32,
) -> Result<(SfzLoadReport, SfzInstrument), SfzLoadError> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| SfzLoadError::Read(format!("读 `{}` 失败: {error}", path.display())))?;
    let bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    // 控制侧专用（会分配）—— 见模块文档 ②。
    let instrument = SfzInstrument::load(text, &ParseLimits::default())
        .map_err(|error| SfzLoadError::Parse(error.to_string()))?;
    let probe = probe_note
        .map(|note| probe_note_energy(&instrument, note, engine_hz))
        .transpose()?;
    Ok((
        SfzLoadReport {
            path: path.to_path_buf(),
            bytes,
            regions: instrument.region_count(),
            probe,
            sample_source: SAMPLE_SOURCE.to_owned(),
            // 默认 `false`（本次装载没有请求导出/录音）；调用方（CLI）在真的要用它导出/录音时改写为 `true`。
            wired_to_export: false,
        },
        instrument,
    ))
}

/// 用一个内存正弦样本触发 `note`，返回帧数与能量（离线/控制侧）。
///
/// # Errors
///
/// 文本在此期间变得非法时透出解析错误（正常路径不会发生）。
pub fn probe_note_energy(
    instrument: &SfzInstrument,
    note: u8,
    engine_hz: f32,
) -> Result<SfzProbeReading, SfzLoadError> {
    // 0.5 秒的样本数据足够覆盖一次探测（引擎按 `RenderRates` 换算播放速率）。
    let frames = usize::try_from(engine_hz.max(1.0) as u64 / 2)
        .unwrap_or(1)
        .max(1);
    let source = InMemorySample::sine(440.0, engine_hz, frames);
    let mut out = vec![0.0_f32; frames];
    let written = instrument
        .render_note_into(&source, note, DEFAULT_PROBE_VELOCITY, &mut out, engine_hz)
        .map_err(|error| SfzLoadError::Parse(error.to_string()))?;
    let energy: f32 = out.iter().map(|sample| sample * sample).sum();
    Ok(SfzProbeReading {
        note,
        velocity: DEFAULT_PROBE_VELOCITY,
        frames: written,
        energy,
    })
}

/// 把一个 PCM WAV 读成 [`InMemorySample`]（**只支持 `read_plain_wav` 能读的格式**）。
///
/// 多声道样本取**声道 0**（本仓的 SFZ 播放按单声道叠加；⛔ 不自己发明下混规则，文档写明）。
///
/// # Errors
///
/// 读文件 / 解码失败 ⇒ [`SfzLoadError::Sample`]。
pub fn read_sample_file(path: &Path) -> Result<(InMemorySample, u64, String), SfzLoadError> {
    // ⭐ **共享实现**（`yeban-render`；MCP 线把它唯一化了）与本地那份**逐行同口径**：
    // `Float32` 直用、`Int16`/`Int24` 除 `full_scale_max().max(1)`、`step_by(channels)` 取单声道，
    // 且它的 `describe` 文案与本函数原来**一字不差**（`file(<path> frames=… rate=…)`）。
    // ⇒ 适配只做**形状转换**（`InMemorySample` ＋ `u64` 帧数），⛔ 不隐藏任何语义差异。
    let (pcm, sample_rate, describe) = yeban_render::wav::read_plain_wav_mono_f32(path)
        .map_err(|error| SfzLoadError::Sample(format!("`{}`: {error}", path.display())))?;
    let frames = u64::try_from(pcm.len()).unwrap_or(u64::MAX);
    Ok((
        InMemorySample {
            pcm,
            sample_hz: sample_rate as f32,
        },
        frames,
        describe,
    ))
}

/// 为一个音符**解析它的区域样本**：优先读 `sample=` 指到的文件，读不到则**如实回退**到内存正弦。
///
/// ⚠ 路径语义：`sample=` 相对路径按 SFZ 惯例**相对 SFZ 文件所在目录**解析（`sample_path()`
/// 已含 `default_path` 前缀）；绝对路径原样使用。
///
/// 返回 `(样本, 来源文案)`；`None` = 该音符没有匹配区域（合法：不发声）。
///
/// # Errors
///
/// 只在该音符**有**匹配区域且样本文件**存在但读不了**时返回错误？⛔ 不 —— 本函数**不**返回错误：
/// 读不到就回退（并要求调用方把回退写进输出行）。⇒ 实际上返回 `Result` 是为了**路径不可解析**这类
/// 需要调用方知道的失败；样本不可读按"回退"处理。
#[must_use]
pub fn region_sample_for(
    instrument: &SfzInstrument,
    sfz_path: &Path,
    note: u8,
    velocity: u8,
) -> Option<(InMemorySample, String)> {
    let sample = instrument
        .with_instrument(|parsed| {
            parsed
                .regions()
                .iter()
                .find(|region| region.matches_key(note) && region.matches_velocity(velocity))
                .map(|region| region.sample_path().into_owned())
        })
        .ok()
        .flatten()?;
    let raw = Path::new(&sample);
    let resolved = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        sfz_path
            .parent()
            .map_or_else(|| raw.to_path_buf(), |parent| parent.join(raw))
    };
    match read_sample_file(&resolved) {
        Ok((in_memory, _frames, describe)) => Some((in_memory, describe)),
        Err(error) => {
            // 如实回退：读不到 ⇒ 内存正弦，并把原因写进来源文案（⛔ 不假装读到了文件）。
            let in_memory = InMemorySample::sine(440.0, 48_000.0, 48_000);
            Some((in_memory, format!("in-memory-sine(440Hz, 回退: {error})")))
        }
    }
}

/// **SFZ 音源**：让 `--export-wav` 用 SFZ 的区域/样本发声（**离线**路径）。
///
/// ## 怎么发声（复用既有设施，⛔ 无旁路）
///
/// 每个音符在**第一次需要时整段渲染一次**（`SfzInstrument::render_note_into`）到自己的 scratch，
/// 之后按块把该块对应的窗口**叠加**进 `out` ⇒ 音符可以跨块延续，且每条音符只解析渲染一次。
/// 样本数据：用 [`SfzTrackSource::new_from_sfz_path`] 构造时逐音符真读 `sample=` 指向的
/// PCM WAV；读不到（或用 [`SfzTrackSource::new`]）才用**共享的一份内存正弦**
/// （[`InMemorySample::sine`]）。
///
/// ## ⚠ 红线 7
///
/// `render_note_into` **每次调用都解析（⇒ 分配）** ⇒ 本类型**只**用于**离线导出**，
/// ⛔ 绝不进音频回调（与 `arm_from_snapshot` 同一条边）。
pub struct SfzTrackSource {
    /// 已装载的 SFZ（多轨可共享同一份 ⇒ 用 `Arc`，⛔ 不重复解析文本）。
    instrument: std::sync::Arc<SfzInstrument>,
    /// 共享的样本数据（**未指定 SFZ 路径**时的回退：内存正弦）。
    sample: InMemorySample,
    /// 每个音符自己的样本与来源文案（`None` = 用共享的内存正弦）。
    per_note: Vec<Option<(InMemorySample, String)>>,
    /// 引擎采样率。
    engine_hz: f32,
    /// 声道数。
    channels: usize,
    /// 每个音符的渲染状态。
    notes: Vec<SfzNoteState>,
}

/// 一个音符的离线渲染状态。
struct SfzNoteState {
    /// 起始帧。
    start_frame: u64,
    /// 时值（帧；上限）。
    target_frames: u64,
    /// 音高。
    pitch: u8,
    /// 力度。
    velocity: u8,
    /// 整段渲染结果（单声道）。
    scratch: Vec<f32>,
    /// `scratch` 里有效样本数（`render_note_into` 的返回值）。
    rendered: usize,
}

impl SfzTrackSource {
    /// 用一份已装载的 SFZ 造一条轨道源。
    ///
    /// `notes` 为 `(起始帧, 时值帧, 音高, 力度)`；帧由调用方按 960 PPQ 换算好。
    #[must_use]
    pub fn new(
        instrument: std::sync::Arc<SfzInstrument>,
        notes: &[(u64, u64, u8, u8)],
        engine_hz: f32,
        channels: usize,
    ) -> Self {
        let sample_frames = usize::try_from(engine_hz.max(1.0) as u64)
            .unwrap_or(48_000)
            .max(1);
        Self {
            instrument,
            sample: InMemorySample::sine(440.0, engine_hz, sample_frames),
            engine_hz,
            channels: channels.max(1),
            notes: notes
                .iter()
                .map(|(start, duration, pitch, velocity)| SfzNoteState {
                    start_frame: *start,
                    target_frames: (*duration).max(1),
                    pitch: *pitch,
                    velocity: *velocity,
                    scratch: Vec::new(),
                    rendered: 0,
                })
                .collect(),
            per_note: Vec::new(),
        }
    }

    /// 与 [`Self::new`] 相同，但**按 SFZ 文件路径解析每条音符的区域样本**（`sample=` 真读文件）。
    ///
    /// ⚠ 读不到（不存在／截断／非 PCM WAV）⇒ **如实回退**到内存正弦，并把原因写进来源文案
    /// （⛔ 不假装读到了文件）。⚠ 红线 7：读文件与解析都发生在**构造期**（离线/控制侧）。
    #[must_use]
    pub fn new_from_sfz_path(
        instrument: std::sync::Arc<SfzInstrument>,
        sfz_path: &Path,
        notes: &[(u64, u64, u8, u8)],
        engine_hz: f32,
        channels: usize,
    ) -> Self {
        let mut source = Self::new(instrument.clone(), notes, engine_hz, channels);
        source.per_note = notes
            .iter()
            .map(|(_, _, pitch, velocity)| {
                region_sample_for(&instrument, sfz_path, *pitch, *velocity)
            })
            .collect();
        source
    }

    /// 本次导出用的样本来源文案（**文件样本优先**，取第一条；否则内存正弦）。
    #[must_use]
    pub fn sample_source(&self) -> String {
        self.per_note
            .iter()
            .flatten()
            .map(|(_, describe)| describe.clone())
            .next()
            .unwrap_or_else(|| SAMPLE_SOURCE.to_owned())
    }

    /// 保证第 `index` 个音符**整段**渲染完成（只做一次）。
    fn ensure_rendered(&mut self, index: usize) {
        if self.notes[index].rendered > 0 {
            return;
        }
        let target = usize::try_from(self.notes[index].target_frames).unwrap_or(usize::MAX);
        let pitch = self.notes[index].pitch;
        let velocity = self.notes[index].velocity;
        let mut scratch = vec![0.0_f32; target.max(1)];
        // 该音符的区域样本（文件样本或回退的内存正弦）。
        let sample = self
            .per_note
            .get(index)
            .and_then(|entry| entry.as_ref())
            .map_or(&self.sample, |(in_memory, _)| in_memory);
        let written = self
            .instrument
            .render_note_into(sample, pitch, velocity, &mut scratch, self.engine_hz)
            .unwrap_or(0);
        let state = &mut self.notes[index];
        state.scratch = scratch;
        state.rendered = written;
    }
}

impl yeban_render::render::AudioSource for SfzTrackSource {
    fn render_block(
        &mut self,
        context: yeban_render::render::BlockContext,
        out: &mut [f32],
    ) -> Result<(), yeban_render::render::RenderError> {
        out.fill(0.0);
        let channels = self.channels;
        let frames = context.frames.min(out.len() / channels);
        let block_start = context.first_frame;
        let block_end = block_start + frames as u64;
        for index in 0..self.notes.len() {
            let note_start = self.notes[index].start_frame;
            if block_end <= note_start {
                continue;
            }
            let window_start = block_start.saturating_sub(note_start);
            let window_end = ((block_end - note_start).min(self.notes[index].target_frames))
                .min(u64::from(u32::MAX));
            if window_end <= window_start {
                continue;
            }
            self.ensure_rendered(index);
            let state = &self.notes[index];
            let limit = state.rendered.min(state.scratch.len());
            let lo = usize::try_from(window_start).unwrap_or(usize::MAX);
            let hi = usize::try_from(window_end).unwrap_or(usize::MAX).min(limit);
            for offset in lo..hi {
                let frame_in_block = if note_start >= block_start {
                    usize::try_from(note_start - block_start).unwrap_or(usize::MAX) + offset
                } else {
                    offset - lo
                };
                if frame_in_block >= frames {
                    break;
                }
                let value = state.scratch[offset];
                for channel in 0..channels {
                    out[frame_in_block * channels + channel] += value;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-sfz-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 判据 a：最小 SFZ（两条区域）⇒ 区域数与文件内容一致。
    #[test]
    fn the_region_count_matches_the_file() {
        let dir = temp_dir("count");
        let path = dir.join("two.sfz");
        std::fs::write(
            &path,
            "<region> sample=a.wav key=60\n<region> sample=b.wav key=64\n",
        )
        .expect("写夹具");
        let report = load_sfz(&path, None, 48_000.0).expect("装载");
        assert_eq!(report.regions, 2, "两条 <region> ⇒ 2");
        assert!(report.probe.is_none(), "没要求探测 ⇒ 无探测读数");
        assert!(
            report.summary().contains(SAMPLE_SOURCE),
            "报告必须如实说明样本来源（⛔ 不是 wav 文件名）"
        );
        assert!(!report.wired_to_export, "今天必须如实说未接到导出");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 b：**触发某音符 ⇒ 非静音**（帧数 > 0 且能量 > 0），且与**不触发**（不匹配的键）可区分。
    #[test]
    fn a_matching_key_is_audible_and_an_unmatched_key_is_silent() {
        let dir = temp_dir("probe");
        let path = dir.join("one.sfz");
        std::fs::write(&path, "<region> sample=a.wav key=60\n").expect("写夹具");
        let instrument = {
            let text = std::fs::read_to_string(&path).expect("读");
            SfzInstrument::load(text, &ParseLimits::default()).expect("装载")
        };
        let hit = probe_note_energy(&instrument, 60, 48_000.0).expect("探测");
        let miss = probe_note_energy(&instrument, 127, 48_000.0).expect("探测");
        println!(
            "[sfz-probe] key=60: {} 帧，能量 {:.6}；key=127: {} 帧，能量 {:.6}",
            hit.frames, hit.energy, miss.frames, miss.energy
        );
        assert!(hit.is_audible(), "匹配键必须非静音: {hit:?}");
        assert!(
            !miss.is_audible(),
            "不匹配键必须静音（合法的不发声）: {miss:?}"
        );
        assert_eq!(miss.frames, 0, "没有匹配区域 ⇒ 写入 0 帧");
        assert_eq!(miss.energy, 0.0, "没有匹配区域 ⇒ 能量 0");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 判据 c：文件不存在 ⇒ `Read` 错误（⛔ 不 panic），且**不新增任何文件**。
    #[test]
    fn a_missing_file_is_an_error_and_writes_nothing() {
        let dir = temp_dir("missing");
        let before = std::fs::read_dir(&dir).expect("列目录").count();
        let error =
            load_sfz(&dir.join("nope.sfz"), Some(60), 48_000.0).expect_err("文件不存在必须失败");
        assert!(matches!(error, SfzLoadError::Read(_)), "实际: {error:?}");
        assert_eq!(std::fs::read_dir(&dir).expect("列目录").count(), before);
        std::fs::remove_dir_all(&dir).ok();
    }
}
