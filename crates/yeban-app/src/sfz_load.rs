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
//! 1. **样本文件会被真读**：`sample=` 指到的文件经 [`read_sample_file`] 解码成单声道 `f32`
//!    （⭐ 任务单 `#85` 起按**后缀分派**：WAV 走 `yeban_render::wav::read_plain_wav_mono_f32`，
//!    其余走 `yeban_decode::decode_path` —— 见下面的 ④），报告行里写出
//!    `file(<path> frames=… rate=…)`。⚠ 仍然成立的两条限制：**WAV 那条腿**只吃 `hound`
//!    能读的 PCM／float WAV（16/24-bit 整数、32-bit 浮点）；任何一条腿读不到时都**如实回退**
//!    到内存正弦 [`InMemorySample::sine`] 并把原因写进来源文案
//!    （探测那一条 [`probe_note_energy`] 本身就用内存正弦 —— 它不解析 `sample=`）。
//!    ⭐ **"读不到"自任务单 `#83` 起是可判别的**：见下面的 ④。
//! 2. **红线 7**：`render_note_into` **每次调用都解析 ⇒ 分配** ⇒ 只能**离线/控制侧**调用，
//!    ⛔ 绝不可在音频回调里调用（与 `arm_from_snapshot` 同一条边）。
//! 3. **已接到导出/录音路径**：`--sfz` ＋ `--export-wav` / `--record-wav` 时，SFZ 音色经
//!    [`SfzTrackSource`] 进入音源链（报告里的 `wired_to_export=yes`）。单独给 `--sfz` 时仍是
//!    `wired_to_export=false` —— 那是"**没请求**导出/录音"，⛔ 不是"没接线"。
//!
//! ## ⭐ ④ **非 PCM-WAV 样本**（⭐ FLAC／OGG）：**已接线**（任务单 `#85` ⇒ 收窄 `#83` 的乙口径）
//!
//! ⭐ **接线后的格式面**（按**后缀**分派，见 [`decode_sample_classified`]）：
//!
//! | `sample=` 后缀 | 解码器 | 口径 |
//! | :--- | :--- | :--- |
//! | `.wav` / `.wave`（大小写不敏感） | `yeban_render::wav::read_plain_wav_mono_f32`（hound） | **逐位与接线前相同** —— 那条腿一个字节都没动 |
//! | **其余**（`.flac` / `.ogg` / …） | `yeban_decode::decode_path`（symphonia：wav／ogg／flac／vorbis／adpcm） | ⭐ **显式折单声道**（取声道 0，与 hound 那条腿**同一口径**） |
//!
//! ⚠ **口径差（⛔ 别踩）**：`DecodedAsset::samples()` 是**交织**多声道，而 hound 那条腿按
//! `step_by(channels)` 取**声道 0** ⇒ 新腿必须做**同一件事**，⛔ 不能把交织数据当单声道
//! （否则立体声样本会以 2× 长度、交替声道进入引擎）。两条腿因此对**同一份 PCM** 给出
//! **逐位相同**的单声道 `f32`（端到端判据见 `tests/sfz_decoded_formats.rs`）。
//!
//! ⚠ **采样率不匹配**：`DecodedAsset::sample_rate()` 与工程采样率不同时**不重采样** ——
//! 它原样进 [`InMemorySample::sample_hz`]，由引擎既有的 `RenderRates::new(sample_hz, engine_hz)`
//! 在**播放速率**上换算（WAV 那条腿从来就是这个口径，⛔ 本单不新造第二条）。理由与
//! "为什么不接 rubato"见 [`decode_sample_classified`]。
//!
//! ⭐ **`#83` 的 `UnsupportedFormat` 因此收窄**：从前"⛔ 不是 PCM WAV"＝ `UnsupportedFormat`；
//! 现在只有**解码器真的不认识**（symphonia 探测失败 / 本构建没启用那个编解码器）才算。
//! 一份**损坏的 FLAC**（`fLaC` 魔数 ＋ 垃圾体）现在归 [`SampleReadFailure::Unreadable`] ——
//! 那是"文件在、也像那种容器，但读不动"，与"格式不受支持"是**两回事**。
//!
//! ⚠ **依赖边**：本单由**人类批文**（"加 `yeban-decode`"）在
//! `crates/yeban-app/Cargo.toml` 新增一条**工作区内部**边，并重生成
//! `docs/ledger/dependency-licenses.md`。⛔ 没有引入任何新的第三方包
//! （`symphonia`／`rubato` 早由 `yeban-mcp` 那条边进了 `Cargo.lock`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_decode::{DecodeError, DecodeOptions, decode_path};
use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::EntityId;
use yeban_render::wav::WavError;
use yeban_sfz::parser::ParseLimits;

/// ⭐ **控制侧读盘计数器**（任务单 `#80` 的缓存判据的**可观测量**）。
///
/// ## 数什么（对象 ＋ 单位）
///
/// | 读数 | 对象 | 单位 |
/// | :--- | :--- | :--- |
/// | `sfz_text_reads` | **本线程**对 SFZ 文本文件发起的 `read_to_string` | 次 |
/// | `sample_decodes` | **本线程**对 `sample=` 指向的样本（两条解码腿：WAV 走 hound／其余走 `yeban_decode`）发起的解码尝试 | 次 |
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
        /// 样本（WAV 走 hound；其余走 `yeban_decode`，任务单 `#85`）的解码尝试次数。
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
    /// 样本文件读取/解码失败（⭐ 任务单 `#85` 起格式面 = PCM／float WAV **＋ FLAC／OGG**）。
    ///
    /// ⭐ 需要区分"文件不存在 / 格式不受支持 / 文件在但读不动"时走 [`decode_sfz_plan`]：
    /// 它把同一次失败同时填进 [`SfzDecodeReport::unreadable_sample_reasons`]。
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

/// 把一个 SFZ 样本读成 [`InMemorySample`]（⭐ 任务单 `#85` 起支持 **FLAC／OGG**）。
///
/// 多声道样本取**声道 0**（本仓的 SFZ 播放按单声道叠加；⛔ 不自己发明下混规则，文档写明）。
/// **两条解码腿用同一口径**：WAV 走 `hound`（既有路径，逐位不变），其余走 `yeban_decode`。
///
/// # Errors
///
/// 读文件 / 解码失败 ⇒ [`SfzLoadError::Sample`]。
/// ⭐ 若调用方需要**可判别理由**（文件不存在 / 格式不受支持 / 文件在但读不动），
/// 走 [`decode_sfz_plan`] —— 它用 [`decode_sample_classified`] 并填
/// [`SfzDecodeReport::unreadable_sample_reasons`]。
pub fn read_sample_file(path: &Path) -> Result<(InMemorySample, u64, String), SfzLoadError> {
    decode_sample_classified(path).map_err(|failure| SfzLoadError::Sample(failure.detail))
}

/// [`read_sample_file`] 的实现体：**按后缀分派**两条解码腿，失败时**带上**可判别理由。
///
/// ## 分派（任务单 `#85`；⛔ 唯一判据是**文件后缀**）
///
/// | 后缀 | 腿 | 依据 |
/// | :--- | :--- | :--- |
/// | `wav` / `wave`（大小写不敏感） | [`decode_with_hound`] | ⭐ **保持既有 WAV 路径逐位不变**（`BASELINE_WAV_READ` 的指纹就是它） |
/// | **其余**（含 `.flac` / `.ogg` / 未知 / 无后缀） | [`decode_with_yeban_decode`] | 通用入口：symphonia 按**内容**探测容器 |
///
/// ⚠ **为什么按后缀而不是按内容嗅探**：内容嗅探会为 `.wav` 增加一次读盘/探测，并且
/// "哪个读取器先认领"变成一条隐含规则；按后缀分派让"WAV 走老腿"这条**可判据的**承诺
/// 逐字成立（既有判据 `sfz_sample_formats::the_wav_sample_path_is_bit_identical_...` 断言
/// 的正是它）。代价如实登记：**无后缀／冷门后缀的 WAV** 现在走通用腿（symphonia 也解 WAV，
/// 结果同口径），`.wav` 但 hound 读不动的位深（32-bit 整数）**仍然读不动**（⛔ 不被新腿救回）。
///
/// ## 采样率（任务单 `#85` 第 4 条：**不重采样**）
///
/// 两条腿都**原样上报文件自己的采样率**（[`InMemorySample::sample_hz`]），由引擎既有的
/// `RenderRates::new(source.sample_hz, engine_hz)` 在**播放速率**上换算。
/// ⛔ 本层**不**调 `rubato` —— 那会把"引擎已经处理好的事"做成第二条换算，
/// 且 WAV 那条腿从来就是这个口径（44.1 kHz 样本在 48 kHz 工程里一直靠播放速率换算）。
///
/// ⚠ 读盘计数（任务单 `#80`）在**这里**自增：**两条腿都会先经过它** ⇒ 两条入口
/// （[`read_sample_file`] 与 [`decode_sfz_plan`]）各调一次都恰好记**一次**，与接线前**逐次相同**
/// （⛔ 不多不少；⛔ 也没有因为分派而多记一次）。
fn decode_sample_classified(
    path: &Path,
) -> Result<(InMemorySample, u64, String), UnreadableSample> {
    read_probe::note_sample_decode();
    if is_wav_suffix(path) {
        decode_with_hound(path)
    } else {
        decode_with_yeban_decode(path)
    }
}

/// 后缀是不是**既有 hound 路径**的那一族（`wav` / `wave`，大小写不敏感）。
///
/// ⚠ 只决定**走哪条腿**，⛔ 不决定成功与否：腿自己失败时按 [`classify_sample_failure`] /
/// [`classify_decode_failure`] 分类。
fn is_wav_suffix(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("wave")
        })
}

/// **WAV 腿**（⛔ 逐字保留接线前的实现，任务单 `#79`／`#83`）。
///
/// ⭐ **共享实现**（`yeban-render`；MCP 线把它唯一化了）与本地那份**逐行同口径**：
/// `Float32` 直用、`Int16`/`Int24` 除 `full_scale_max().max(1)`、`step_by(channels)` 取单声道，
/// 且它的 `describe` 文案与本函数原来**一字不差**（`file(<path> frames=… rate=…)`）。
/// ⇒ 适配只做**形状转换**（`InMemorySample` ＋ `u64` 帧数），⛔ 不隐藏任何语义差异。
fn decode_with_hound(path: &Path) -> Result<(InMemorySample, u64, String), UnreadableSample> {
    match yeban_render::wav::read_plain_wav_mono_f32(path) {
        Ok((pcm, sample_rate, describe)) => {
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
        Err(error) => {
            let reason = classify_sample_failure(path, &error);
            Err(UnreadableSample {
                path: path.display().to_string(),
                reason,
                // ⚠ 负载与接线前**逐字相同**（`SfzLoadError::Sample` 的构造式没变）
                // ⇒ `read_sample_file` 透出的错误文案、`region_sample_for` 的回退文案、
                //   以及 `unreadable_samples` 那一行都⛔ 不漂移。
                detail: format!("`{}`: {error}", path.display()),
            })
        }
    }
}

/// ⭐ **通用解码腿**（任务单 `#85`）：`yeban_decode::decode_path` ⇒ 显式折单声道。
///
/// ## 折单声道（⚠ 本函数存在的**唯一**理由）
///
/// [`yeban_decode::DecodedAsset::samples`] 是**交织**样本（`frames × channels`），
/// 而本仓 SFZ 播放的口径是**单声道声道 0**（与 [`decode_with_hound`] 的
/// `step_by(channels)` 一字不差）⇒ 这里按**同一个** `step_by(channels)` 取声道 0。
/// ⛔ 若直接交出交织缓冲，立体声样本会以 2× 长度、左右交替进入引擎（判据
/// `sfz_decoded_formats::a_stereo_flac_is_folded_to_channel_zero_like_the_wav_leg` 会红）。
///
/// ## 与 WAV 腿的语义对齐
///
/// * `sample_hz` ← [`yeban_decode::DecodedAsset::sample_rate`]（原样，⛔ 不重采样）；
/// * `describe` ← 同一形状 `file(<path> frames=<折后帧数> rate=<文件采样率>)`；
/// * 帧数 ← 折单声道**之后**的长度（= 每声道帧数，与 WAV 腿的 `samples.len()` 同义）。
fn decode_with_yeban_decode(
    path: &Path,
) -> Result<(InMemorySample, u64, String), UnreadableSample> {
    match decode_path(path, &DecodeOptions::default()) {
        Ok(asset) => {
            // `decode_path` 保证 `channels >= 1`（0 声道的布局在闸门处就被拒），
            // `max(1)` 只是让"除零"在这条路径上不可能发生。
            let channels = usize::from(asset.channels().max(1));
            let pcm: Vec<f32> = asset.samples().iter().step_by(channels).copied().collect();
            let sample_rate = asset.sample_rate();
            let describe = format!(
                "file({} frames={} rate={})",
                path.display(),
                pcm.len(),
                sample_rate
            );
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
        Err(error) => {
            let reason = classify_decode_failure(path, &error);
            Err(UnreadableSample {
                path: path.display().to_string(),
                reason,
                // ⚠ 与 WAV 腿**同一个形状**（`` `<path>`: <读取器原文> ``）⇒
                // `UnreadableSample::row` / `message` 不需要第二条格式化。
                detail: format!("`{}`: {error}", path.display()),
            })
        }
    }
}

/// ⭐ 一份**读不到的样本**为什么读不到（任务单 `#83` 的**乙**口径：可判别，⛔ 不再只是一句
/// 混在一起的错误文案）。
///
/// ⚠ 本枚举只描述**已经读失败**的那一类。它⛔ 不参与任何成功路径，也⛔ 不改变
/// [`read_sample_file`] 的成功结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SampleReadFailure {
    /// **文件不存在**：`sample=` 解析出来的路径在盘上没有（资源没随 SFZ 分发，或路径写错）。
    NotFound,
    /// **格式不受支持**：文件在，但**两条解码腿都不认识它**。
    ///
    /// ⭐ **本档自任务单 `#85` 起收窄**（`#83` 的乙口径 ⇒ 甲口径）：
    ///
    /// | 情形 | `#83`（接线前） | `#85`（接线后） |
    /// | :--- | :--- | :--- |
    /// | `.flac` / `.ogg` 的**真文件** | `UnsupportedFormat` | **读成功**（⛔ 不再落这一档） |
    /// | 解码器**认不出**的容器（如 `.xyz` 文本） | `UnsupportedFormat` | `UnsupportedFormat` |
    /// | 本构建**没启用**那个编解码器 | `UnsupportedFormat` | `UnsupportedFormat` |
    /// | 是 RIFF/WAVE 但位深不受支持（如 32-bit 整数） | `UnsupportedFormat` | `UnsupportedFormat` |
    ///
    /// ⚠ **为什么收窄是对的**：`UnsupportedFormat` 的语义是"**这个格式**不受支持"，
    /// 而 FLAC／OGG 现在**受支持** ⇒ 再把它们记成"格式不受支持"就是一条**假读数**
    /// （用户会据此去转格式，而真正的问题是别的）。反过来，一份**损坏的 FLAC**
    /// （`fLaC` 魔数 ＋ 垃圾体）落 [`SampleReadFailure::Unreadable`] —— 容器认得、内容坏，
    /// 与"格式不支持"分得开，正是 `#83` 建立可判别性时要的那个区分。
    UnsupportedFormat,
    /// **文件在、容器也（可能）认得，但读不动**：截断／容器字段非法（声道或采样率为 0）／
    /// 样本越界／坏帧／声明时长与实际不符／其它 I/O 失败。
    ///
    /// ⚠ 对**通用腿**（`yeban_decode`）而言，本档还涵盖"像某种容器但内容坏"——
    /// 例如 `fLaC` 魔数 ＋ 垃圾体（见上表）。
    Unreadable,
}

impl core::fmt::Display for SampleReadFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "文件不存在"),
            Self::UnsupportedFormat => write!(
                formatter,
                "格式不受支持（本入口读 PCM／float WAV 与 FLAC／OGG；这条读失败是解码器\
                 认不出容器，或本构建没启用那个编解码器）"
            ),
            Self::Unreadable => write!(
                formatter,
                "文件在、但读取/解析失败（截断／字段非法／样本越界／坏帧／I/O）"
            ),
        }
    }
}

/// ⭐ 一条**读不到的样本**的登记：路径 ＋ **可判别理由** ＋ 读取器原文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableSample {
    /// [`resolve_sample_path`] 解析后的路径（`Display` 形态）。
    pub path: String,
    /// ⭐ 可判别理由（[`SampleReadFailure`]）。
    pub reason: SampleReadFailure,
    /// 读取器原文：`SfzLoadError::Sample` 的**负载**（形如 `` `<path>`: <读取器错误> ``；
    /// WAV 腿是 `WavError`，通用腿（`#85`）是 `DecodeError`）。
    pub detail: String,
}

impl UnreadableSample {
    /// 与接线前**逐字相同**的那一行 `"<路径>: <错误>"`（既有判据按它断言，⛔ 不改）。
    #[must_use]
    pub fn row(&self) -> String {
        format!("{}: {}", self.path, self.message())
    }

    /// [`SfzLoadError::Sample`] 的 `Display`（`row` 与错误透出共用**同一份**格式化）。
    #[must_use]
    pub fn message(&self) -> String {
        SfzLoadError::Sample(self.detail.clone()).to_string()
    }
}

/// 文件头是不是 WAV 家族（`RIFF` 或 `RF64` ＋ `WAVE`）。
///
/// ⚠ 只读**前 12 个字节**，且只用于**已经读失败**的文件分类：用它区分"根本不是 WAV"
/// （⇒ 别的容器/编码）与"是 WAV 但读不动"。⛔ 不参与成功路径（那条路走 `hound` 的判定）。
fn looks_like_wav_family(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 12];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    let riff = header[0..4] == *b"RIFF" || header[0..4] == *b"RF64";
    let wave = header[8..12] == *b"WAVE";
    riff && wave
}

/// ⭐ 给一份**读不到的 WAV 样本**分类（任务单 `#83`；⛔ 只服务 `wav` / `wave` 那条腿）。
///
/// ⚠ 任务单 `#85` 之后，本函数**只**由 [`decode_with_hound`] 调用；非 WAV 后缀走
/// [`classify_decode_failure`]。两条腿共用同一个 [`SampleReadFailure`] 词表
/// （⇒ 报告里的 `unreadable_kind(...)` 计数对两条腿**同一口径**）。
///
/// 顺序：① 盘上有没有这个路径 ⇒ [`SampleReadFailure::NotFound`]；
/// ② 读取器的错误种类 ＋ 文件头是不是 WAV 家族 ⇒ 格式面 / 内容面。
#[must_use]
pub fn classify_sample_failure(path: &Path, error: &WavError) -> SampleReadFailure {
    if std::fs::metadata(path).is_err() {
        return SampleReadFailure::NotFound;
    }
    match error {
        // 位深不受支持 ⇒ **格式面**（文件本身没坏，是本读取器不吃它）。
        WavError::UnsupportedDepth(_) => SampleReadFailure::UnsupportedFormat,
        // `hound` 把"根本不是 WAV"与"WAV 但内容坏"报成同一类错 ⇒ 用**内容**分开。
        WavError::Hound(_) if !looks_like_wav_family(path) => SampleReadFailure::UnsupportedFormat,
        // 剩下的是"文件在、也像 WAV，但读不动"。
        WavError::Hound(_) | WavError::FormatMismatch { .. } | WavError::RejectedFormat { .. } => {
            SampleReadFailure::Unreadable
        }
    }
}

/// ⭐ 给一份**读不到的通用腿样本**分类（任务单 `#85`）。
///
/// ## 三档怎么分（⛔ 不是"非 WAV 就不支持"）
///
/// | `DecodeError` | 档 | 为什么 |
/// | :--- | :--- | :--- |
/// | [`DecodeError::UnsupportedFormat`] | [`SampleReadFailure::UnsupportedFormat`] | 探测器**认不出容器** ⇒ 真的是格式问题（`.xyz` 落这里） |
/// | [`DecodeError::UnsupportedCodec`] | [`SampleReadFailure::UnsupportedFormat`] | 容器认得，但**本构建没启用**那个编解码器 ⇒ 对用户同样是"格式不受支持" |
/// | [`DecodeError::Io`] 且 `kind == NotFound` | [`SampleReadFailure::NotFound`] | 竞态下文件在 `metadata` 之后消失 ⇒ 仍是"不存在"，⛔ 不冒充"读不动" |
/// | 其余（畸形／无音轨／缺编解码参数／空流／超预算／重采样…） | [`SampleReadFailure::Unreadable`] | 文件在、内容坏 ⇒ 与"格式不受支持"分得开 |
///
/// ⚠ 顺序与 [`classify_sample_failure`] **同款**：先判盘上有没有（`metadata`），
/// 再判错误种类 —— 两条腿因此对"文件不存在"给出**同一个**读数。
#[must_use]
pub fn classify_decode_failure(path: &Path, error: &DecodeError) -> SampleReadFailure {
    if std::fs::metadata(path).is_err() {
        return SampleReadFailure::NotFound;
    }
    match error {
        DecodeError::UnsupportedFormat | DecodeError::UnsupportedCodec { .. } => {
            SampleReadFailure::UnsupportedFormat
        }
        DecodeError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            SampleReadFailure::NotFound
        }
        _ => SampleReadFailure::Unreadable,
    }
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
/// `silent_region_count` / `unreadable_samples` 四条一起回答"哪一份读了、哪一份没读"，
/// ⭐ 再加上 `unreadable_sample_reasons` 回答"**为什么**没读"（任务单 `#83`：把
/// "文件不存在"与"格式不受支持（FLAC／OGG）"分开）。
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
    ///
    /// ⚠ **逐字与接线前相同**（既有判据按它断言）。结构化视图见
    /// [`Self::unreadable_sample_reasons`] —— 后者是**唯一来源**，本字段由它导出。
    pub unreadable_samples: Vec<String>,
    /// ⭐ 同样那些读失败的样本，但**带可判别理由**（任务单 `#83` 的乙口径：
    /// [`SampleReadFailure::NotFound`] 与 [`SampleReadFailure::UnsupportedFormat`] 分得开）。
    ///
    /// ⚠ 与 [`Self::unreadable_samples`] **逐条同序同源**（后者每一行都是本字段的
    /// [`UnreadableSample::row`]）⇒ 两者⛔ 不可能不一致。
    pub unreadable_sample_reasons: Vec<UnreadableSample>,
    /// 摊平表占用的字节数（[`SfzPlan::memory_bytes`]）。
    pub plan_bytes: usize,
}

impl SfzDecodeReport {
    /// 一行可读摘要（导出报告/SFZ 报告行直接用）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "path={} bytes={} regions={} samples={} silent={} missing={:?} unreadable={} \
             unreadable_kind(not_found={},unsupported={},other={}) plan_bytes={}",
            self.path.display(),
            self.bytes,
            self.regions,
            self.decoded_samples,
            self.silent_region_count,
            self.missing_sample_paths,
            self.unreadable_samples.len(),
            self.unreadable_not_found_count(),
            self.unreadable_unsupported_format_count(),
            self.unreadable_other_count(),
            self.plan_bytes,
        )
    }

    /// ⭐ 读失败的样本里，理由 = [`SampleReadFailure::NotFound`] 的**条数**（单位：条）。
    #[must_use]
    pub fn unreadable_not_found_count(&self) -> usize {
        self.count_reason(SampleReadFailure::NotFound)
    }

    /// ⭐ 读失败的样本里，理由 = [`SampleReadFailure::UnsupportedFormat`] 的**条数**（单位：条）。
    ///
    /// ⚠ **这一条就是"SFZ 用了 FLAC／OGG"的读数**：它⛔ 不再与"文件不存在"混在一起。
    #[must_use]
    pub fn unreadable_unsupported_format_count(&self) -> usize {
        self.count_reason(SampleReadFailure::UnsupportedFormat)
    }

    /// 读失败的样本里，其余理由（[`SampleReadFailure::Unreadable`]）的**条数**（单位：条）。
    #[must_use]
    pub fn unreadable_other_count(&self) -> usize {
        self.count_reason(SampleReadFailure::Unreadable)
    }

    fn count_reason(&self, reason: SampleReadFailure) -> usize {
        self.unreadable_sample_reasons
            .iter()
            .filter(|entry| entry.reason == reason)
            .count()
    }
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#168`：SFZ 资产根的**唯一**解析
// ---------------------------------------------------------------------------

/// ⭐ **仓库根目录**（`CARGO_MANIFEST_DIR/../..`）—— 内置 SFZ 清单相对路径的契约根。
///
/// 它是 [`resolve_sampler_path`] 的第二个候选根，也是**唯一**一份"仓库根"取值：
/// 探测（`bridge::sfz_samples_present`）与渲染（[`arm_pending_sfz`]）都经
/// [`resolve_sampler_path`] 取它，⛔ 不各自写 `join("../..")`。
///
/// 优先 `canonicalize`（把 `..` 与符号链接消掉，错误信息里读得到真实的根）；
/// 失败时退回**未规范化**的那条路径（宁可给出带 `..` 的根，也不静默换一个根）。
#[must_use]
pub fn repo_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        raw.canonicalize().unwrap_or(raw)
    })
    .as_path()
}

/// ⭐ 解析一条 SFZ 路径时**按序试**的几个根（任务单 `#168`）。
///
/// | 字段 | 什么时候是 `Some` | 语义 |
/// | :--- | :--- | :--- |
/// | [`Self::project_dir`] | 工程是**从文件打开的** | 工程文件所在目录（用户写的相对路径通常相对它） |
/// | [`Self::cwd`] | 判据要**显式注入** cwd；生产传 `None` | 进程当前目录（兼容旧行为） |
///
/// ⚠ `cwd: None` **不是**"没有 cwd 这个根"，而是"取 `std::env::current_dir()`"——
/// 生产路径因此不必传它；判据传 `Some` 就能在**同一进程**里模拟三种 cwd
/// （`set_current_dir` 是进程级副作用，并行判据会互相污染）。
///
/// ⚠ ⛔ **本结构不是"根的真相源"**：仓库根永远来自 [`repo_root`]，⛔ 调用方不能覆盖它。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SfzRoots<'a> {
    /// 工程文件所在目录（`None` = 工程不是从文件打开的 / 装配方没给）。
    pub project_dir: Option<&'a Path>,
    /// 进程当前目录（`None` = 解析时读 `std::env::current_dir()`）。
    pub cwd: Option<&'a Path>,
}

impl<'a> SfzRoots<'a> {
    /// 只有仓库根 ＋ 进程 cwd（`#164`／`#165` 的旧口径；探测与离线导出用它）。
    #[must_use]
    pub const fn none() -> Self {
        Self {
            project_dir: None,
            cwd: None,
        }
    }

    /// 工程文件所在目录优先，其次仓库根，再次进程 cwd。
    #[must_use]
    pub const fn under_project(project_dir: &'a Path) -> Self {
        Self {
            project_dir: Some(project_dir),
            cwd: None,
        }
    }

    /// 判据用：显式指定 cwd（替代进程真实 cwd，避免并行判据互相污染）。
    #[must_use]
    pub const fn with_cwd(mut self, cwd: &'a Path) -> Self {
        self.cwd = Some(cwd);
        self
    }
}

/// ⭐ 一条相对 SFZ 路径**在所有候选根下都找不到**时的逐个候选（任务单 `#168`）。
///
/// 它存在的唯一理由：⛔ **不许静默静音**。`Display` 逐字列出"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"，
/// 于是用户/判据能从一条错误里读出"我到底在哪些目录下找过"，而不是只看到"加载失败"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SfzPathError {
    /// 模型里逐字存的那条路径（⛔ 不规范化 —— 用户看到的必须是他写下的那句）。
    path: String,
    /// 按**尝试顺序**排的候选绝对路径（去重后；通常 3 条，工程未知时 2 条）。
    tried: Vec<PathBuf>,
}

impl SfzPathError {
    /// 模型里逐字存的那条路径。
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 按尝试顺序排的候选路径（错误信息的可机械断言形态）。
    #[must_use]
    pub fn tried(&self) -> &[PathBuf] {
        &self.tried
    }
}

impl core::fmt::Display for SfzPathError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "SFZ 路径 `{}` 找不到；试过：", self.path)?;
        for (index, candidate) in self.tried.iter().enumerate() {
            if index > 0 {
                write!(formatter, "／")?;
            }
            write!(formatter, "{}", candidate.display())?;
        }
        Ok(())
    }
}

impl core::error::Error for SfzPathError {}

/// ⭐ ⭐ **SFZ 路径的唯一种解析**（任务单 `#168`）：探测与渲染都调它。
///
/// ## 规则（按序，**先命中先返回**）
///
/// 1. **绝对路径 ⇒ 原样返回**（⛔ 不查存在性、⛔ 不改写：模型里写的就是它；
///    文件真的不在时，`decode_sfz_plan` 自己的 `Read` 错误带着这条路径 —— 仍**响亮**）；
/// 2. **相对路径** ⇒ 依次试：
///    ① 工程文件所在目录（若 [`SfzRoots::project_dir`] 给了）；
///    ② **仓库根**（[`repo_root`]，内置清单的契约）；
///    ③ 进程 cwd（[`SfzRoots::cwd`]，缺省读 `std::env::current_dir()`）；
///    第一个 `is_file()` 的候选胜出；
/// 3. **都找不到 ⇒ [`SfzPathError`]**（`Display` 逐字列出全部候选；⛔ 绝不静默静音）。
///
/// ## 为什么这个顺序
///
/// * **工程目录第一**：用户把样本放在工程旁边是**可迁移**的写法，且它比仓库根/ cwd
///   更具体 —— 同一个相对字符串在两种解释下都存在时，选"跟工程走"的那份；
/// * **仓库根第二**：内置清单（[`crate::bridge::BUILTIN_INSTRUMENTS`]）的路径**逐字
///   相对仓库根**写进模型，这是 `#164` 的既有契约；
/// * **cwd 最后**：它只是 `#79` 之前的历史行为，绝不优先于前两者 —— 否则"从别的目录
///   启动"又会改变解释，正是 `#168` 要堵的那条路。
///
/// ⚠ 候选按 `PathBuf` **去重**（工程目录可能就是仓库根）⇒ 错误信息不会有重复行。
/// ⚠ 本函数**只查存在性**（`is_file`），⛔ 不读文件内容、⛔ 不解析 SFZ。
///
/// # Errors
///
/// [`SfzPathError`]：相对路径在全部候选根下都不是一个文件。
pub fn resolve_sampler_path(path: &str, roots: SfzRoots<'_>) -> Result<PathBuf, SfzPathError> {
    let raw = Path::new(path);
    // 规则 1：绝对路径原样（缺文件由 `decode_sfz_plan` 的 Read 错误兜住，仍响亮）。
    if raw.is_absolute() {
        return Ok(raw.to_path_buf());
    }
    // 规则 2：三个候选，按序。
    let mut tried: Vec<PathBuf> = Vec::with_capacity(3);
    if let Some(project_dir) = roots.project_dir {
        push_unique_candidate(&mut tried, project_dir.join(raw));
    }
    push_unique_candidate(&mut tried, repo_root().join(raw));
    let cwd = roots
        .cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    if let Some(cwd) = cwd {
        push_unique_candidate(&mut tried, cwd.join(raw));
    }
    if let Some(found) = tried.iter().find(|candidate| candidate.is_file()) {
        return Ok(found.clone());
    }
    // 规则 3：响亮（逐字列出试过的根）。
    Err(SfzPathError {
        path: path.to_owned(),
        tried,
    })
}

/// 按 `PathBuf` 逐字去重地追加一个候选（顺序 = 尝试顺序）。
fn push_unique_candidate(tried: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !tried.contains(&candidate) {
        tried.push(candidate);
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
    // ④：逐个真读（复用 `decode_sample_classified`；解析规则与 `region_sample_for` 共用）。
    let mut provided: Vec<(String, InMemorySample)> = Vec::new();
    let mut unreadable_sample_reasons: Vec<UnreadableSample> = Vec::new();
    for raw in &referenced {
        let resolved = resolve_sample_path(sfz_path, Path::new(raw));
        match decode_sample_classified(&resolved) {
            Ok((sample, _frames, _describe)) => provided.push((raw.clone(), sample)),
            // ⭐ 读不到 ⇒ **不放进提供表**（⇒ 引擎按缺样本处理），但把原因**如实登记**
            //    ＋ **可判别理由**（任务单 `#83`：文件不存在 / 格式不受支持 / 读不动）。
            Err(failure) => unreadable_sample_reasons.push(failure),
        }
    }
    // ⭐ 旧的字符串形态由结构化登记**导出**（⛔ 不写第二份 ⇒ 两者不可能不一致）。
    let unreadable_samples: Vec<String> = unreadable_sample_reasons
        .iter()
        .map(UnreadableSample::row)
        .collect();
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
        unreadable_samples,
        unreadable_sample_reasons,
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
/// ⭐ **任务单 `#168`**：每条待摊平项的路径都先经 [`resolve_sampler_path`]（**唯一**解析：
/// 绝对原样 ⇒ 工程目录 ⇒ 仓库根 ⇒ cwd）。相对路径在全部候选根下都找不到时，错误原文
/// 逐字带"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"（⛔ 不静默静音）。
///
/// # Errors
///
/// 任一 SFZ 文件读不到 / 文本非法 ⇒ 透出 [`SfzLoadError`]（消息含具体路径与试过的根）。
pub fn arm_pending_sfz(
    snapshot: EngineSnapshot,
    engine_hz: f32,
    prebuilt: BTreeMap<EntityId, SfzPlan>,
    roots: SfzRoots<'_>,
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
        // ⭐ `#168`：唯一解析（绝对原样 ⇒ 工程目录 ⇒ 仓库根 ⇒ cwd）；找不到 ⇒ 响亮。
        let resolved = resolve_sampler_path(&path, roots)
            .map_err(|error| SfzLoadError::Read(error.to_string()))?;
        let (plan, report) = decode_sfz_plan(&resolved, engine_hz)?;
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
/// PCM WAV／FLAC／OGG；读不到（或用 [`SfzTrackSource::new`]）才用**共享的一份内存正弦**
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
    /// ⚠ 读不到（不存在／截断／格式不受支持）⇒ **如实回退**到内存正弦，并把原因写进来源文案
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
