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
//! ## ⭐ 导出音源上的**轨增益**（归属裁决 `R711`；⛔ 不重复计）
//!
//! ```text
//! TrackV3{mute, solo, solo_safe, volume_db} ──▶ yeban_engine::synth::track_gain_scale(...)
//!          ──▶ SfzTrackSource::with_track_gain(门)      // ⭐ 与引擎侧**同一个算子**
//!          ──▶ 乘在 render_note_into 的输出之外          // 乐器层 × 轨增益，⛔ 不重复
//! ```
//!
//! 值由 [`crate::render_core`] 逐轨算出（⛔ 本模块不写第二份 `volume_db ⇒ 线性` 换算）。
//!
//! ## ⭐ 模型链条：`TrackV3.sampler` ⇒ 真的发声的摊平表（任务单 `#79`）
//!
//! ```text
//! EngineSnapshot::sfz_pending()   // ⭐ #78：引擎只带"这条轨有一份 SFZ"这一事实（⛔ 不读盘）
//!    ──▶ decode_sfz_plan(path)    // ⭐ 本模块：读盘 → 解析 → 路径全集 → 逐个解码 → prepare_with_samples
//!    ──▶ EngineSnapshot::with_sfz(track, plan)   // ⭐ 取代待摊平项（引擎侧既有入口）
//!    ──▶ SynthEngine::render_track(…, Some(plan.prepared()), …)  // 实时侧第三条音源支路
//! ```
//!
//! [`arm_pending_sfz`] 把这条链**一次走完**（`render_core` 在导出/录音的构造期调它）。
//! ⚠ 它与 `--sfz` **旁路**（[`SfzTrackSource`]）是两条**不同**的入口，关系与理由见
//! [`arm_pending_sfz`] 与 `render_core` 的模块文档：
//!
//! * **`--sfz <文件>`**：**整工程**用同一份 SFZ 替换引擎音源（旁路；不进引擎 SFZ 分支）；
//! * **`TrackV3.sampler`**：**按轨**绑定（模型链条；引擎 SFZ 分支真的发声）。
//!   两者同时给 ⇒ **旁路优先**（"整工程试听一份 SFZ"是显式请求）。
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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::EntityId;
use yeban_sfz::parser::ParseLimits;

/// ⭐ **控制侧读盘计数器**（任务单 `#80` 的缓存判据的**可观测量**）。
///
/// ## 数什么（对象 ＋ 单位）
///
/// | 读数 | 对象 | 单位 |
/// | :--- | :--- | :--- |
/// | `sfz_text_reads` | **本线程**对 SFZ 文本文件发起的 `read_to_string` | 次 |
/// | `sample_decodes` | **本线程**对 `sample=` 指向的 PCM WAV 发起的解码尝试 | 次 |
///
/// ⚠ 计的是**发起次数**（失败也算一次：读盘动作已经发生），⛔ 不是"成功次数"。
///
/// ## 为什么按线程计（⛔ 不用进程级 `AtomicU64`）
///
/// 与 `yeban-engine` 的 `release_probe` 同一条纪律：跨线程的窗口必须**按线程归账**，
/// 否则同一进程里并行的判据会互相污染读数（本仓实测过这类假红：目录计数被并行活动扰动，
/// 见 `sfz_load` 测试 c 的注释）。控制侧的装载/解码都发生在**调用它的那一条线程**上
/// （GUI 主线程 / MCP 控制线程 / 判据线程），因此按线程计恰好覆盖真实调用面。
///
/// ## 红线 7（实时线程零可见性）
///
/// 它只在**控制侧**（[`load_sfz_with_instrument`] 与 [`read_sample_file`]）自增；
/// 实时侧不读盘（引擎快照只带已摊平的表）⇒ ⛔ 音频回调一个字都不碰它。
pub mod read_probe {
    use std::cell::Cell;

    thread_local! {
        static SFZ_TEXT_READS: Cell<u64> = const { Cell::new(0) };
        static SAMPLE_DECODES: Cell<u64> = const { Cell::new(0) };
    }

    /// 一次读盘读数的**快照**（单位：次）。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct SfzReadCounts {
        /// SFZ 文本文件的读盘尝试次数。
        pub sfz_text_reads: u64,
        /// 样本（PCM WAV）的解码尝试次数。
        pub sample_decodes: u64,
    }

    impl SfzReadCounts {
        /// 两个分量之和（仍是一个"次数"）。
        #[must_use]
        pub const fn total(self) -> u64 {
            self.sfz_text_reads + self.sample_decodes
        }

        /// 相对 `earlier` 的**增量**（`self − earlier`，逐分量）。
        ///
        /// ⚠ 前一个读数更大时**饱和到 0**（⛔ 不 panic、⛔ 不绕回）。
        #[must_use]
        pub const fn since(self, earlier: Self) -> Self {
            Self {
                sfz_text_reads: self.sfz_text_reads.saturating_sub(earlier.sfz_text_reads),
                sample_decodes: self.sample_decodes.saturating_sub(earlier.sample_decodes),
            }
        }
    }

    /// 此刻**本线程**的读数。
    #[must_use]
    pub fn counts() -> SfzReadCounts {
        SfzReadCounts {
            sfz_text_reads: SFZ_TEXT_READS.with(Cell::get),
            sample_decodes: SAMPLE_DECODES.with(Cell::get),
        }
    }

    /// 记一次 SFZ 文本读盘尝试（[`super::load_sfz_with_instrument`] 唯一调用点）。
    pub(super) fn note_sfz_text_read() {
        SFZ_TEXT_READS.with(|counter| counter.set(counter.get().saturating_add(1)));
    }

    /// 记一次样本解码尝试（[`super::read_sample_file`] 唯一调用点）。
    pub(super) fn note_sample_decode() {
        SAMPLE_DECODES.with(|counter| counter.set(counter.get().saturating_add(1)));
    }
}

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
    // ⭐ 读盘计数器（任务单 `#80`）：只在这里自增 —— 本函数是 SFZ 文本**唯一**的读入口。
    read_probe::note_sfz_text_read();
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
    // ⭐ 读盘计数器（任务单 `#80`）：只在这里自增 —— 本函数是样本 PCM **唯一**的解码入口。
    read_probe::note_sample_decode();
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
    let resolved = resolve_sample_path(sfz_path, raw);
    match read_sample_file(&resolved) {
        Ok((in_memory, _frames, describe)) => Some((in_memory, describe)),
        Err(error) => {
            // 如实回退：读不到 ⇒ 内存正弦，并把原因写进来源文案（⛔ 不假装读到了文件）。
            let in_memory = InMemorySample::sine(440.0, 48_000.0, 48_000);
            Some((in_memory, format!("in-memory-sine(440Hz, 回退: {error})")))
        }
    }
}

/// 解析一个 `sample=` 的**真路径**（⛔ 唯一实现：新解码管线与 [`region_sample_for`] 共用）。
///
/// ⚠ 路径语义（SFZ 惯例）：相对路径**相对 SFZ 文件所在目录**；绝对路径原样使用。
/// `sample=` 里已经带了 `default_path` 前缀（见 `Region::sample_path`）。
fn resolve_sample_path(sfz_path: &Path, sample: &Path) -> PathBuf {
    if sample.is_absolute() {
        sample.to_path_buf()
    } else {
        sfz_path
            .parent()
            .map_or_else(|| sample.to_path_buf(), |parent| parent.join(sample))
    }
}

/// ⭐ **一次控制侧解码的读数**（任务单 `#79`；形状照 [`SfzLoadReport`]：路径／字节／区域数／
/// **诚实字段**）。
///
/// 它是"⭐ **缺样本要如实**"的载体：`decoded_samples` / `missing_sample_paths` /
/// `silent_region_count` / `unreadable_samples` 四条一起回答"哪一份读了、哪一份没读、为什么"。
/// ⚠ 三个缺样本字段都是**引擎口径**（直接读 [`yeban_engine::sfz_instrument::PreparedSfz`] 的
/// 同名方法），⛔ 本层**不另定义**一套。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SfzDecodeReport {
    /// SFZ 文件路径（逐字来自调用方；⛔ 不规范化）。
    pub path: PathBuf,
    /// 文件字节数。
    pub bytes: u64,
    /// 区域数（[`SfzInstrument::region_count`]）。
    pub regions: usize,
    /// ⭐ [`SfzInstrument::referenced_sample_paths`] 的读数（排序 ＋ 去重；`sample=` 原样字符串）。
    pub referenced_sample_paths: Vec<String>,
    /// 真的解码成功、进了提供表的样本**份数**。
    pub decoded_samples: usize,
    /// ⭐ 引擎口径的缺样本路径（[`yeban_engine::sfz_instrument::PreparedSfz::missing_sample_paths`]；
    /// 排序 ＋ 去重，元素是 `sample=` 原样字符串）。
    pub missing_sample_paths: Vec<String>,
    /// ⭐ 缺样本 ⇒ `NO_SAMPLE`（**不产生采样输出**）的**区域数**（引擎口径）。
    pub silent_region_count: usize,
    /// 读盘失败的样本（`"<解析后路径>: <原因>"` 逐条登记；⛔ 不静默）。
    pub unreadable_samples: Vec<String>,
    /// 摊平表占用的字节数（[`SfzPlan::memory_bytes`]）。
    pub plan_bytes: usize,
}

impl SfzDecodeReport {
    /// 一行可读摘要（导出报告/SFZ 报告行直接用）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "path={} bytes={} regions={} samples={} silent={} missing={:?} unreadable={} plan_bytes={}",
            self.path.display(),
            self.bytes,
            self.regions,
            self.decoded_samples,
            self.silent_region_count,
            self.missing_sample_paths,
            self.unreadable_samples.len(),
            self.plan_bytes,
        )
    }
}

/// ⭐ ⭐ **控制侧解码管线**（任务单 `#79`）：SFZ 文件路径 ⇒ 可发声的 [`SfzPlan`]。
///
/// ## 六步（每一步都**复用既有形状**，⛔ 不另写一份解码）
///
/// ```text
/// ① 读盘 ＋ 解析   load_sfz_with_instrument(path, None, hz)   // 与 --sfz 装载同一个函数
/// ② 区域数/字节    ← 同一个 SfzLoadReport 的字段（⛔ 不再数一遍）
/// ③ 路径全集       SfzInstrument::referenced_sample_paths()   // 引擎给的路径全集
/// ④ 逐个解码       read_sample_file(resolve_sample_path(…))   // 与 region_sample_for 同一条解析
/// ⑤ 摊平           SfzPlan::prepare_with_samples()            // 引擎口径：去重 ＋ 缺样本 NO_SAMPLE
/// ⑥ 读数           plan.prepared() 的 missing/silent ＋ 本函数登记的 unreadable
/// ```
///
/// ⚠ **缺样本 ⇒ 如实**：读不到的 `sample=` **⛔ 不放进提供表** ⇒
/// [`yeban_engine::sfz_instrument::PreparedSfz::prepare_with_samples`] 把该区域标
/// `NO_SAMPLE`（触发时**不建声部**）＋ 计数 ⇒ 路径进 `missing_sample_paths`。
/// ⛔ 本函数**不发明替代内容**（不回退内存正弦、不去播别人的采样）—— 归属裁决见台账 `R711`。
///
/// ⚠ **红线 7**：本函数读盘 ＋ 分配（解析、PCM）⇒ **只**用于离线/控制侧（导出、装载、
/// 判据），⛔ 绝不在音频回调里调用。
///
/// # Errors
///
/// * [`SfzLoadError::Read`]：SFZ 文件读不到（**明确错误**，⛔ 不静默回退）；
/// * [`SfzLoadError::Parse`]：SFZ 文本非法，或区域表无法摊平。
///   ⚠ 样本读不到**不是**错误（走"缺样本 ⇒ 静音 ＋ 计数"那条既有口径，见上）。
pub fn decode_sfz_plan(
    sfz_path: &Path,
    engine_hz: f32,
) -> Result<(SfzPlan, SfzDecodeReport), SfzLoadError> {
    // ① ＋ ②：读盘与解析走 `--sfz` 装载的**同一个**函数（⛔ 不复制读文件/解析那两行）。
    let (load, instrument) = load_sfz_with_instrument(sfz_path, None, engine_hz)?;
    // ③：引擎给的路径全集（排序 ＋ 去重 ⇒ 确定性顺序）。
    let referenced = instrument
        .referenced_sample_paths()
        .map_err(|error| SfzLoadError::Parse(error.to_string()))?;
    // ④：逐个真读（复用 `read_sample_file`；解析规则与 `region_sample_for` 共用）。
    let mut provided: Vec<(String, InMemorySample)> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for raw in &referenced {
        let resolved = resolve_sample_path(sfz_path, Path::new(raw));
        match read_sample_file(&resolved) {
            Ok((sample, _frames, _describe)) => provided.push((raw.clone(), sample)),
            // ⭐ 读不到 ⇒ **不放进提供表**（⇒ 引擎按缺样本处理），但把原因**如实登记**。
            Err(error) => unreadable.push(format!("{}: {error}", resolved.display())),
        }
    }
    // ⑤：引擎口径的摊平（去重键 = `sample_path` 原样字符串；缺样本 ⇒ `NO_SAMPLE` ＋ 计数）。
    let plan = SfzPlan::prepare_with_samples(&instrument, &provided, engine_hz)
        .map_err(|error| SfzLoadError::Parse(error.to_string()))?;
    let prepared = plan.prepared();
    let report = SfzDecodeReport {
        path: sfz_path.to_path_buf(),
        bytes: load.bytes,
        regions: load.regions,
        referenced_sample_paths: referenced,
        decoded_samples: provided.len(),
        missing_sample_paths: prepared.missing_sample_paths().to_vec(),
        silent_region_count: prepared.silent_region_count(),
        unreadable_samples: unreadable,
        plan_bytes: plan.memory_bytes(),
    };
    Ok((plan, report))
}

/// ⭐ **把快照里的待摊平项全部变成摊平表**（任务单 `#79` 的接线：`PendingSfz` ⇒ `with_sfz`）。
///
/// 逐条（`BTreeMap` ⇒ 键升序、确定性）调 [`decode_sfz_plan`]，再用
/// [`EngineSnapshot::with_sfz`] 注入 —— 后者会把该轨的待摊平项**移除**（"已经有摊平表了"）。
///
/// `prebuilt`：调用方**已经**持有摊平表时的注入入口（缓存/判据；⛔ 不重复解码该轨）。
/// 它先注入，因此也享受 `with_sfz` 的"取代待摊平项"语义。
///
/// ⚠ **SFZ 文件本身缺失 ⇒ 明确错误**（`Err`，消息里带路径），⛔ 不静默回退到复音合成器：
/// 模型说"这条轨用这份文件"，而文件读不到时仍出一版**不是它**的声音，是用户听不出、
/// 报告里也看不见的内容错。缺**样本**（文件在、`sample=` 读不到）仍走既有口径
/// （区域静音 ＋ 计数，见 [`decode_sfz_plan`]）。
///
/// # Errors
///
/// 任一 SFZ 文件读不到 / 文本非法 ⇒ 透出 [`SfzLoadError`]（消息含具体路径）。
pub fn arm_pending_sfz(
    snapshot: EngineSnapshot,
    engine_hz: f32,
    prebuilt: BTreeMap<EntityId, SfzPlan>,
) -> Result<(EngineSnapshot, Vec<(EntityId, SfzDecodeReport)>), SfzLoadError> {
    let mut snapshot = snapshot;
    for (track, plan) in prebuilt {
        snapshot = snapshot.with_sfz(track, plan);
    }
    // 先收集（键升序）再逐条解码：`with_sfz` 需要 `&mut self`，⛔ 不能边借边改。
    let pending: Vec<(EntityId, String)> = snapshot
        .sfz_pending()
        .iter()
        .map(|(track, entry)| (*track, entry.path().to_owned()))
        .collect();
    let mut reports = Vec::with_capacity(pending.len());
    for (track, path) in pending {
        let (plan, report) = decode_sfz_plan(Path::new(&path), engine_hz)?;
        snapshot = snapshot.with_sfz(track, plan);
        reports.push((track, report));
    }
    Ok((snapshot, reports))
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
    /// ⭐ **引擎层的轨增益门**（可闻门 ＝ 静音/独奏 × 推子；默认 **`1.0`＝单位增益**）。
    ///
    /// ## 归属裁决 `R711`（⛔ 不重复计）
    ///
    /// **乐器层**拥有乐器内增益（`volume_gain` / `velocity_gain[128]` / `key_crossfade` /
    /// `velocity_fade_*` / `amp_veltrack`），**引擎层**拥有轨增益（推子 / 静音 / 独奏）⇒
    /// ⭐ **两者相乘、⛔ 不重复**：`render_block` 把本标量乘在
    /// [`SfzInstrument::render_note_into`] 的**输出之外** ⇒ 与引擎侧
    /// （`synth::SfzVoicePool::trigger_note` 的 `region.gain_at(...) * track_gain`）
    /// 同一个形状。⛔ 力度那一半属于乐器层，本类型**不碰**。
    ///
    /// ## 值从哪来（⭐ 同一个算子，⛔ 不是这里自己换算）
    ///
    /// 调用方（[`crate::render_core`]）用引擎的
    /// [`yeban_engine::synth::track_gain_scale`] 逐轨算出同一个值再经
    /// [`Self::with_track_gain`] 注入 —— 那个函数正是引擎快照投影
    /// （`snapshot::project_schedules`）与声部槽（`SynthEngine::set_track_gain_scales`）
    /// 共用的**唯一**实现。⛔ 本类型**不**自己写一份 `volume_db ⇒ 线性` 的换算。
    track_gain: f32,
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
            // ⭐ 默认**单位增益**（⛔ 不是 `0.0`：默认不是静音）—— 与引擎侧
            // `NoteSchedule::from_sorted` 的默认 `track_gain = 1.0` 同口径。
            track_gain: 1.0,
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

    /// ⭐ 写入**引擎层的轨增益门**（推子 × 静音/独奏；构造期一次）。
    ///
    /// ⚠ 值必须来自引擎的**同一个算子** [`yeban_engine::synth::track_gain_scale`]
    /// （见字段文档）—— ⛔ 本类型不接受"这里自己换算"的第二份实现。
    /// 与引擎侧 [`yeban_engine::synth::NoteSchedule::with_track_gain`] 同名同形。
    #[must_use]
    pub const fn with_track_gain(mut self, track_gain: f32) -> Self {
        self.track_gain = track_gain;
        self
    }

    /// ⭐ 本源的**轨增益门**（与力度无关：可闻门 × 推子）。
    ///
    /// 产品路径下它逐位等于引擎调度表上 [`yeban_engine::synth::NoteSchedule::track_gain`]
    /// 的那一份（同一次 [`yeban_engine::synth::track_gain_scale`] 的值）。
    #[must_use]
    pub const fn track_gain(&self) -> f32 {
        self.track_gain
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
        // ⭐ **引擎层的轨增益门**乘在**乐器层输出之外**（归属裁决 `R711`：两者相乘、
        //    ⛔ 不重复）。乐器层（`render_note_into`）已经含它自己的 `volume_gain` /
        //    `velocity_gain` / 交叉淡化等 —— 这里**只**乘轨增益这一半。
        //    `1.0` 是 IEEE 恒等 ⇒ 跳过乘法，让"调用方没给轨参数"的读数逐位不变。
        let track_gain = self.track_gain;
        if track_gain != 1.0 {
            let limit = written.min(scratch.len());
            for sample in &mut scratch[..limit] {
                *sample *= track_gain;
            }
        }
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

    /// 判据 c：文件不存在 ⇒ `Read` 错误（⛔ 不 panic），且**不写出任何文件**。
    ///
    /// ⚠ **为何不直接比较目录计数**（2026-10-16 修，见台账 R709／R710）：本判据原以
    /// `read_dir(..).count()` 前后相等为准，而在 Windows runner 上**偶发变红**（同一提交
    /// 时红时绿、本机 macOS 复现不出）。目录计数是**间接量**：它会被与该判据无关的
    /// 文件系统活动扰动（另一个并行测试、杀软/索引器、临时目录的惰性回收）。
    /// 把断言改成**直接量**——"目标路径不存在" ＋ "目录里没有 `.sfz`"——才是它真正要证的
    /// 事（"⛔ 不写出东西"），且不再受无关活动影响。
    #[test]
    fn a_missing_file_is_an_error_and_writes_nothing() {
        let dir = temp_dir("missing");
        let missing = dir.join("nope.sfz");
        let error = load_sfz(&missing, Some(60), 48_000.0).expect_err("文件不存在必须失败");
        assert!(matches!(error, SfzLoadError::Read(_)), "实际: {error:?}");
        // ⭐ 直接量之一：目标文件⛔ 必须不存在（"writes nothing" 的字面意思）。
        assert!(!missing.exists(), "load_sfz 失败时⛔ 不得写出目标文件");
        // ⭐ 直接量之二：目录里⛔ 不得出现任何 `.sfz`（间接量的窄化版，仍不依赖裸计数）。
        let sfz_files: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".sfz"))
            .collect();
        assert!(
            sfz_files.is_empty(),
            "load_sfz 失败时⛔ 不得写出 .sfz：{sfz_files:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
