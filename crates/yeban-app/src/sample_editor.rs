//! `--edit-samples <input>` —— 采样库编辑的**批处理出口**。
//!
//! ## 它补的是哪一格（这是本模块存在的全部理由）
//!
//! Wang 2026-11-10 缺口报告 C-1：`build_erhu2.py` 逐采样 FFT 取基频 → 音准校正到
//! ±0.5 cent，用 `sus` 的 >2.2 kHz 分量伪造 mf/f 力度层，再 RMS 归一；284 个采样，
//! 手工不可能。yeban 有 SFZ 解析与播放，没有采样编辑器 —— 这就是"二胡难听"折腾
//! 三轮的真正原因。本模块把这三件事接到批处理路径上：一次
//! `yeban-app --edit-samples <采样目录或 .sfz> --sample-edit-out <目录>`，不开窗口、
//! 不建 MCP 会话、不碰工程。
//!
//! ## 三件工具（DSP 件住在 `yeban_dsp::sample_edit`，本模块只做编排与落盘）
//!
//! * **音准校正**：每采样测基频 → 与目标（SFZ `pitch_keycenter`；目录模式取最近半音）
//!   的偏差 → PSOLA 移调（保时长，循环点与时值不变）；|偏差| ≤ 0.5 音分不动样本；
//!   |偏差| > 1200 音分（±一个八度）**拒绝改这个采样** —— 那不是跑调，是 SFZ 映射错了
//!   （例如文件名八度与 `pitch_keycenter` 差两个八度），记为 skip，不静默硬改。
//! * **力度层合成**：调好音的样本过 2200 Hz 四阶高通，取高频分量作 mf/f 层，再按
//!   同一目标 RMS 归一。层是音色素材：响度区分由 SFZ 力度映射给，不是把增益烤进文件。
//! * **RMS 归一**：全部输出缩放到目标 RMS（缺省 −18 dBFS）；峰值顶过 1.0 时报告里
//!   写出来，⛔ 不静默削波。
//!
//! ## 输入两种形态
//!
//! * **采样目录**：递归取 `.wav` / `.wave` / `.flac` / `.ogg`（排序确定）；目标音高
//!   = 测得基频的最近半音（目录里没有 keycenter，这是唯一诚实的默认 —— 报告里
//!   每采样都写明测得值与目标值）。
//! * **`.sfz` 文件**：`yeban-sfz` 解析（含 `#include` 沙箱）→ 每个 region 的
//!   `sample`（`\` 先归一化为 `/` 再拼 SFZ 所在目录）与 `pitch_keycenter` 作目标；
//!   同一文件被多个 region 引用只处理一次。另出一份扁平重发的
//!   `<stem>_edited.sfz`（见下）。
//!
//! ## 扁平重发的 SFZ（只在 SFZ 模式）
//!
//! `<out>/<stem>_edited.sfz` 是**扁平重发**，不是原文改写：原文件的 `#include`
//! 结构、注释与本工具不认识的 opcode（包络、滤波、颤音控制等）**没有**带入 ——
//! 带入需要全量 opcode 重发，那是另一项工作。文件头注释把这一点写明。每个 region
//! 发一条调好音的 region（`sample`/`lokey`/`hikey`/`pitch_keycenter`/`lovel`/
//! `hivel`，循环 region 另带 `loop_mode`/`loop_start`/`loop_end`），再按力度
//! 三等分发 mf/f 层 region（`lovel`/`hivel` 三等分；范围窄到分不出三段时不发层）。
//!
//! ## 失败语义（与 `--export-song-overview` 同款）
//!
//! * 输入读不到 / 一个采样都没有 / 输出目录建不起来 ⇒ **响亮失败**（退出码 5），
//!   ⛔ 不产出一份半残的报告冒充成功；
//! * 单个采样失败（解码失败、测不出基频、移调超限、静音）⇒ 记为 skip、stderr
//!   一行 `sample-edit-skipped:`、报告里留 `reason`，其余采样继续；**零个**采样
//!   成功才算整体失败；
//! * 落盘走原子实现（WAV：同目录临时文件 → rename；JSON/SFZ：
//!   [`crate::save::write_file_atomically`]），失败不留半成品；
//! * 重复跑同一输入 ⇒ 同一份报告、同一批输出字节（DSP 件确定性判据钉住；
//!   报告里不写绝对时间戳，只写输入与输出路径）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use yeban_dsp::math::note_to_hz;
use yeban_dsp::sample_edit::{
    TUNE_DEADBAND_CENTS, cents_between, correct_pitch, measure_fundamental, normalize_rms, rms,
    synthesize_layer,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav, write_plain_wav};

use crate::save::write_file_atomically;

/// 导出成功后回执行的第一段（稳定前缀，判据与脚本据此切片）。
pub const RECEIPT_PREFIX: &str = "edited-samples:";

/// `--edit-samples` 的字面值（采样库编辑的**批处理**出口）。
pub const SAMPLE_EDIT_SWITCH: &str = "--edit-samples";

/// 缺省目标 RMS（dBFS）：−18 dBFS 是本人的选择（脚本已佚，无从考证其目标值），
///
/// 不是脚本的原值 —— 报告与 `--help` 都写明这一点。
pub const DEFAULT_TARGET_RMS_DBFS: f32 = -18.0;

/// 要合成的伪造力度层（`--sample-edit-layers` 的取值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerSet {
    /// 合成 mf 层（中段力度）。
    pub mf: bool,
    /// 合成 f 层（高段力度）。
    pub f: bool,
}

impl Default for LayerSet {
    /// 缺省 = [`LayerSet::all`]（mf 与 f 都合成，与 `--sample-edit-layers` 缺省一致）。
    fn default() -> Self {
        Self::all()
    }
}

impl LayerSet {
    /// 缺省：mf 与 f 都合成。
    #[must_use]
    pub const fn all() -> Self {
        Self { mf: true, f: true }
    }

    /// 一层都不合成。
    #[must_use]
    pub const fn none() -> Self {
        Self {
            mf: false,
            f: false,
        }
    }

    /// 解析 `--sample-edit-layers` 的取值（`none` / `mf` / `f` / `mf,f`）。
    ///
    /// # Errors
    ///
    /// 取值不在白名单里时返回原文（调用方转用法错误）。
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "none" => Ok(Self::none()),
            "mf" => Ok(Self { mf: true, f: false }),
            "f" => Ok(Self { mf: false, f: true }),
            "mf,f" | "f,mf" => Ok(Self::all()),
            _ => Err(value.to_owned()),
        }
    }
}

/// 一次编辑的入参（`cli::Options` 的投影：⛔ 不从入参抄数字进报告）。
#[derive(Debug, Clone)]
pub struct SampleEditCliOptions {
    /// 输入：采样目录或 `.sfz` 文件。
    pub input: PathBuf,
    /// 输出目录（报告与编辑后样本的落点；不存在即创建）。
    pub out_dir: PathBuf,
    /// 目标 RMS（dBFS）。
    pub target_rms_dbfs: f32,
    /// 要合成的伪造力度层。
    pub layers: LayerSet,
    /// 跳过音准校正（只做层合成与归一）。
    pub no_tune: bool,
    /// 只测算、只写报告，不写音频（预览口径）。
    pub report_only: bool,
}

/// 一个采样（或一个 region）的处理行（报告 JSON 的**唯一**来源，见 [`row_json`]）。
#[derive(Debug)]
pub struct SampleRow {
    /// 源文件（SFZ 模式：region 的 `sample=` 解析后路径；目录模式：输入路径）。
    pub source: String,
    /// 调好音的输出文件名（skip 时为 `null`）。
    pub out_tuned: Option<String>,
    /// 测得基频（Hz，测不出时为 `null`）。
    pub measured_hz: Option<f64>,
    /// 目标频率（Hz，SFZ `pitch_keycenter` 或最近半音）。
    pub target_hz: Option<f64>,
    /// 校正前的偏差（音分，正值 = 测得的比目标高）。
    pub cents_before: Option<f32>,
    /// 实际施加的移调量（音分；死区内 / `--sample-edit-no-tune` 时为 0）。
    pub cents_applied: f32,
    /// 校正前 RMS（dBFS；skip 时为 `None` —— 没测到就没有读数，不写 −∞ 冒充）。
    pub rms_before_dbfs: Option<f32>,
    /// 归一后 RMS（dBFS，skip 时为 `null`）。
    pub rms_after_dbfs: Option<f32>,
    /// 归一后峰值（线性；skip 时为 `null`）。
    pub peak: Option<f32>,
    /// 合成的层文件名（`mf` / `f`，未合成时为空）。
    pub layers: Vec<String>,
    /// `ok` 或 `skipped`。
    pub status: &'static str,
    /// skip 的原因（`ok` 时为 `null`）。
    pub reason: Option<String>,
}

/// 一次编辑的实测读数（回执行的**唯一**来源）。
#[derive(Debug)]
pub struct SampleEditReport {
    /// 输入（原样）。
    pub input: PathBuf,
    /// 输出目录（原样）。
    pub out_dir: PathBuf,
    /// 实际落点（报告 JSON 的路径）。
    pub report_path: PathBuf,
    /// 发现的采样数（去重后）。
    pub samples: usize,
    /// 调好音并落盘的采样数。
    pub tuned: usize,
    /// 合成并落盘的层文件数。
    pub layers: usize,
    /// skip 的采样数。
    pub skipped: usize,
    /// 目标 RMS（dBFS，实测入参）。
    pub target_rms_dbfs: f32,
    /// 是否报告模式（只写报告，未写音频）。
    pub report_only: bool,
    /// 逐采样行（与报告 JSON 同一份数据）。
    pub rows: Vec<SampleRow>,
}

impl SampleEditReport {
    /// 回执行（**一行**，字段顺序即契约）。
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{RECEIPT_PREFIX} input={} out={} samples={} tuned={} layers={} skipped={} \
             rms={:.1}dBFS report={}",
            self.input.display(),
            self.out_dir.display(),
            self.samples,
            self.tuned,
            self.layers,
            self.skipped,
            self.target_rms_dbfs,
            self.report_path.display(),
        )
    }
}

/// 编辑失败的稳定码（回执与 JSON 里只认码，不从文案里正则抠）。
#[derive(Debug)]
pub enum SampleEditError {
    /// 输入路径不存在 / 不是目录也不是 `.sfz`。
    InputUnreadable {
        /// 输入路径。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
    /// 输入里一个可处理的采样都没有。
    NoSamples {
        /// 输入路径。
        path: PathBuf,
    },
    /// 输出目录建不起来 / 报告落不下去。
    Output {
        /// 输出路径。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
    /// SFZ 解析失败（`yeban-sfz` 的原文）。
    Sfz {
        /// SFZ 路径。
        path: PathBuf,
        /// 解析器原文。
        reason: String,
    },
}

impl SampleEditError {
    /// 稳定错误码。
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InputUnreadable { .. } => "INPUT_UNREADABLE",
            Self::NoSamples { .. } => "NO_SAMPLES",
            Self::Output { .. } => "OUTPUT_FAILED",
            Self::Sfz { .. } => "SFZ_PARSE_FAILED",
        }
    }
}

impl core::fmt::Display for SampleEditError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InputUnreadable { path, reason } => {
                write!(formatter, "输入 `{}` 读不到: {reason}", path.display())
            }
            Self::NoSamples { path } => {
                write!(
                    formatter,
                    "输入 `{}` 里一个可处理的采样都没有",
                    path.display()
                )
            }
            Self::Output { path, reason } => {
                write!(formatter, "输出 `{}` 失败: {reason}", path.display())
            }
            Self::Sfz { path, reason } => {
                write!(formatter, "SFZ `{}` 解析失败: {reason}", path.display())
            }
        }
    }
}

/// 一个待处理采样：解出的单声道 PCM ＋ 目标频率 ＋ 输出词干。
struct SampleJob {
    /// 源文件（显示与去重用）。
    source: PathBuf,
    /// 解出的单声道 PCM。
    samples: Vec<f32>,
    /// 采样率（Hz）。
    sample_rate: u32,
    /// 输出格式（WAV 输入：原位深；其余：16-bit）。
    format: OutputFormat,
    /// 目标频率（Hz；目录模式由最近半音算出，SFZ 模式由 `pitch_keycenter` 算出）。
    target_hz: Option<f64>,
    /// 输出词干（去重后）。
    stem: String,
}

/// 输出 WAV 的格式（只记"写回时用什么位深"，不记声道：输出恒为单声道）。
#[derive(Debug, Clone, Copy)]
struct OutputFormat {
    /// 采样率（Hz）。
    sample_rate: u32,
    /// 每样本有效位深（16 / 24 / 32）。
    bits_per_sample: u16,
    /// `true` = IEEE 浮点容器。
    is_float: bool,
}

/// 解出一个采样文件：WAV 走 `hound` 腿，其余走 `yeban-decode` 腿。
///
/// 两条腿**同一口径**：单声道取声道 0；整数分母是 `2^(bits-1)`（与
/// `yeban_render::wav` 的 `mono_f32_from_plain` 同一条，#254）。
fn decode_sample(path: &Path) -> Result<(Vec<f32>, u32, OutputFormat), String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "wav" || extension == "wave" {
        let (format, buffer) =
            read_plain_wav(path).map_err(|error| format!("WAV 读失败: {error}"))?;
        let channels = usize::from(format.channels.max(1));
        // ⚠ 分母是 `2^(bits-1)`，⛔ 不是 `2^(bits-1) - 1`（见 wav.rs #254）。
        let scale = match &buffer {
            PcmBuffer::Int16(_) => 1.0_f32 / 32768.0,
            PcmBuffer::Int24(_) => 1.0_f32 / 8_388_608.0,
            PcmBuffer::Float32(_) => 1.0_f32,
        };
        let samples: Vec<f32> = match &buffer {
            PcmBuffer::Int16(values) => values
                .iter()
                .step_by(channels)
                .map(|value| f32::from(*value) * scale)
                .collect(),
            PcmBuffer::Int24(values) => values
                .iter()
                .step_by(channels)
                .map(|value| (*value as f32) * scale)
                .collect(),
            PcmBuffer::Float32(values) => values.iter().step_by(channels).copied().collect(),
        };
        Ok((
            samples,
            format.sample_rate,
            OutputFormat {
                sample_rate: format.sample_rate,
                bits_per_sample: format.bits_per_sample,
                is_float: format.is_float,
            },
        ))
    } else {
        let asset = yeban_decode::decode_path(path, &yeban_decode::DecodeOptions::default())
            .map_err(|error| format!("解码失败: {error:?}"))?;
        let channels = usize::from(asset.channels().max(1));
        let samples: Vec<f32> = asset.samples().iter().step_by(channels).copied().collect();
        Ok((
            samples,
            asset.sample_rate(),
            OutputFormat {
                sample_rate: asset.sample_rate(),
                bits_per_sample: 16,
                is_float: false,
            },
        ))
    }
}

/// `f32` 样本按输出格式量化（写回时的唯一换算：读侧分母的逆运算）。
fn quantize(samples: &[f32], format: OutputFormat) -> PcmBuffer {
    if format.is_float {
        PcmBuffer::Float32(samples.to_vec())
    } else if format.bits_per_sample == 24 {
        PcmBuffer::Int24(
            samples
                .iter()
                .map(|x| (x * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32)
                .collect(),
        )
    } else {
        PcmBuffer::Int16(
            samples
                .iter()
                .map(|x| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                .collect(),
        )
    }
}

/// 原子写一份 WAV：同目录临时文件 → rename（失败不留半成品）。
fn write_wav_atomically(path: &Path, samples: &[f32], format: OutputFormat) -> Result<(), String> {
    let pcm_format = if format.is_float {
        PcmFormat::float(1, format.sample_rate, format.bits_per_sample)
    } else {
        PcmFormat::integer(1, format.sample_rate, format.bits_per_sample)
    };
    let buffer = quantize(samples, format);
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = write_plain_wav(&tmp, &pcm_format, &buffer)
        .map_err(|error| format!("WAV 写失败: {error}"))
        .and_then(|()| {
            std::fs::rename(&tmp, path).map_err(|error| format!("WAV 落盘失败: {error}"))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 收集输入：目录（递归，排序确定）或 `.sfz`（解析出 region）。
fn collect_jobs(
    options: &SampleEditCliOptions,
) -> Result<(Vec<SampleJob>, Option<SfzPlan>), SampleEditError> {
    let input = &options.input;
    if !input.exists() {
        return Err(SampleEditError::InputUnreadable {
            path: input.clone(),
            reason: "路径不存在".to_owned(),
        });
    }
    if input.is_dir() {
        let mut paths: Vec<PathBuf> = Vec::new();
        collect_audio_files(input, &mut paths).map_err(|reason| {
            SampleEditError::InputUnreadable {
                path: input.clone(),
                reason,
            }
        })?;
        paths.sort();
        let mut jobs = Vec::new();
        let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("sample")
                .to_owned();
            jobs.push(SampleJob {
                source: path,
                samples: Vec::new(),
                sample_rate: 0,
                format: OutputFormat {
                    sample_rate: 0,
                    bits_per_sample: 16,
                    is_float: false,
                },
                target_hz: None,
                stem,
            });
        }
        Ok((jobs, None))
    } else if input
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sfz"))
    {
        let plan = SfzPlan::load(input).map_err(|reason| SampleEditError::Sfz {
            path: input.clone(),
            reason,
        })?;
        let mut jobs = Vec::new();
        let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
        for region in &plan.regions {
            if !seen.insert(region.sample_path.clone()) {
                continue;
            }
            let stem = region
                .sample_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("sample")
                .to_owned();
            jobs.push(SampleJob {
                source: region.sample_path.clone(),
                samples: Vec::new(),
                sample_rate: 0,
                format: OutputFormat {
                    sample_rate: 0,
                    bits_per_sample: 16,
                    is_float: false,
                },
                target_hz: Some(f64::from(note_to_hz(region.pitch_keycenter as f32))),
                stem,
            });
        }
        Ok((jobs, Some(plan)))
    } else {
        Err(SampleEditError::InputUnreadable {
            path: input.clone(),
            reason: "输入既不是目录也不是 `.sfz` 文件".to_owned(),
        })
    }
}

/// 递归收集目录下的音频文件（`.wav` / `.wave` / `.flac` / `.ogg`）。
fn collect_audio_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("读目录失败: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("读目录项失败: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_audio_files(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("wav")
                    || ext.eq_ignore_ascii_case("wave")
                    || ext.eq_ignore_ascii_case("flac")
                    || ext.eq_ignore_ascii_case("ogg")
            })
        {
            out.push(path);
        }
    }
    Ok(())
}

/// 一个 SFZ 输入的解析结果：region 清单（供重发）＋ 词干去重表。
struct SfzPlan {
    /// 源 SFZ 路径。
    source: PathBuf,
    /// 解析出的 region（文件顺序）。
    regions: Vec<SfzRegion>,
}

/// 解析出的一个 region（只取本工具用的字段，其余 opcode 不重发，见模块文档）。
struct SfzRegion {
    /// 解析后的样本路径（已拼 SFZ 所在目录、`\\` 已归一化）。
    sample_path: PathBuf,
    /// 根音（`pitch_keycenter`）。
    pitch_keycenter: i32,
    /// 音域下界。
    lokey: i32,
    /// 音域上界。
    hikey: i32,
    /// 力度下界。
    lovel: u8,
    /// 力度上界。
    hivel: u8,
    /// 循环模式（`no_loop` 时不写循环 opcode）。
    loop_mode: yeban_sfz::instrument::LoopMode,
    /// 循环起点（采样点）。
    loop_start: u32,
    /// 循环终点（采样点）。
    loop_end: u32,
}

impl SfzPlan {
    /// 解析一份 SFZ（含 `#include` 沙箱），返回 region 清单。
    fn load(path: &Path) -> Result<Self, String> {
        // 先探可读性：读不到就给一句干净的"读 SFZ 失败"，而不是等解析器从 include
        // 沙箱里报出来。真正解析的是 `resolver` 读出的 `sources`。
        std::fs::read_to_string(path).map_err(|error| format!("读 SFZ 失败: {error}"))?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let limits = yeban_sfz::ParseLimits::default();
        let resolver = yeban_sfz::IncludeResolver::new(parent, limits)
            .map_err(|error| format!("SFZ 沙箱失败: {error}"))?;
        let entry = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_owned();
        let sources = resolver
            .resolve(&entry)
            .map_err(|error| format!("SFZ include 展开失败: {error}"))?;
        let instrument = yeban_sfz::parse_sources(&sources, &limits)
            .map_err(|error| format!("SFZ 解析失败: {error}"))?;
        let mut regions = Vec::new();
        for region in instrument.regions() {
            // `sample=` 可能是 Windows 写法（`..\Samples\...`）：先归一化分隔符再拼目录。
            let joined = region.sample_path().replace('\\', "/");
            let sample_path = parent.join(&joined);
            regions.push(SfzRegion {
                sample_path,
                pitch_keycenter: region.pitch_keycenter,
                lokey: region.lokey,
                hikey: region.hikey,
                lovel: region.lovel,
                hivel: region.hivel,
                loop_mode: region.loop_mode,
                loop_start: region.loop_start,
                loop_end: region.loop_end,
            });
        }
        Ok(Self {
            source: path.to_path_buf(),
            regions,
        })
    }
}

/// 力度三等分：`[lovel, hivel]` 切三段（p / mf / f）；范围窄到分不出三段时返回 `None`。
fn split_velocity(lovel: u8, hivel: u8) -> Option<[(u8, u8); 3]> {
    let span = i32::from(hivel) - i32::from(lovel);
    if span < 2 {
        return None;
    }
    let third = span / 3;
    let p_hi = lovel as i32 + third;
    let mf_hi = lovel as i32 + 2 * third;
    Some([
        (lovel, p_hi as u8),
        ((p_hi + 1) as u8, mf_hi as u8),
        ((mf_hi + 1) as u8, hivel),
    ])
}

/// 扁平重发一份 SFZ（只在 SFZ 模式）：调好音的 region ＋ mf/f 层 region。
fn emit_edited_sfz(plan: &SfzPlan, rows: &[SampleRow], out_path: &Path) -> Result<(), String> {
    use yeban_sfz::instrument::LoopMode;
    let mut text = String::new();
    text.push_str("// 由 yeban-app --edit-samples 生成的扁平化重发：只含本工具改写过的 region。\n");
    text.push_str("// 原文件的 #include 结构、注释与本工具不认识的 opcode（包络、滤波、颤音控制等）没有带入。\n");
    text.push_str(&format!("// 来源：{}\n", plan.source.display()));
    // 按源 region 顺序发；`rows` 与 `plan.regions` 的对应关系由"去重后的采样顺序"保证：
    // 同一采样被多个 region 引用时，第一个 region 的行即该采样的行。
    let mut row_by_source: BTreeMap<&str, &SampleRow> = BTreeMap::new();
    for row in rows {
        row_by_source.entry(row.source.as_str()).or_insert(row);
    }
    for region in &plan.regions {
        let Some(row) = row_by_source.get(region.sample_path.to_str().unwrap_or("")) else {
            continue;
        };
        let Some(tuned) = row.out_tuned.as_deref() else {
            continue;
        };
        let mut emit = |sample: &str, lovel: u8, hivel: u8| {
            text.push_str("\n<region>\n");
            text.push_str(&format!("sample={sample}\n"));
            text.push_str(&format!("lokey={}\n", region.lokey));
            text.push_str(&format!("hikey={}\n", region.hikey));
            text.push_str(&format!("pitch_keycenter={}\n", region.pitch_keycenter));
            text.push_str(&format!("lovel={lovel}\n"));
            text.push_str(&format!("hivel={hivel}\n"));
            if !matches!(region.loop_mode, LoopMode::NoLoop) {
                let mode = match region.loop_mode {
                    LoopMode::OneShot => "one_shot",
                    LoopMode::LoopContinuous => "loop_continuous",
                    LoopMode::LoopSustain => "loop_sustain",
                    LoopMode::NoLoop => unreachable!(),
                };
                text.push_str(&format!("loop_mode={mode}\n"));
                text.push_str(&format!("loop_start={}\n", region.loop_start));
                text.push_str(&format!("loop_end={}\n", region.loop_end));
            }
        };
        let layer_kinds: Vec<&str> = row
            .layers
            .iter()
            .map(|name| if name.ends_with("_mf.wav") { "mf" } else { "f" })
            .collect();
        if layer_kinds.is_empty() {
            emit(tuned, region.lovel, region.hivel);
        } else if let Some([(p_lo, p_hi), (mf_lo, mf_hi), (f_lo, f_hi)]) =
            split_velocity(region.lovel, region.hivel)
        {
            emit(tuned, p_lo, p_hi);
            let stem = tuned.strip_suffix("_tuned.wav").unwrap_or(tuned);
            for kind in &layer_kinds {
                let (lo, hi) = if *kind == "mf" {
                    (mf_lo, mf_hi)
                } else {
                    (f_lo, f_hi)
                };
                emit(&format!("{stem}_{kind}.wav"), lo, hi);
            }
        } else {
            // 范围窄到分不出三段：层文件已落盘，但 region 不发（范围重叠会触发歧义）。
            emit(tuned, region.lovel, region.hivel);
        }
    }
    write_file_atomically(text.as_bytes(), out_path)
        .map_err(|error| format!("写 SFZ 失败: {error}"))?;
    Ok(())
}

/// 一行报告转 JSON（手写映射，不用 `serde` derive：yeban-app 不直接依赖 `serde`，
/// 只有 `serde_json` —— 字段名出现在这里，判据按这些键名钉住，改名两边一起改）。
///
/// 非有限浮点（−∞ 之类）在这里没有落点：`SampleRow` 的数值字段要么是实测有限值，
/// 要么是 `None`（→ `null`）—— `serde_json` 对非有限数会整个序列化失败，
/// 一个 skip 行不能拖垮整份报告。
fn row_json(row: &SampleRow) -> serde_json::Value {
    serde_json::json!({
        "source": row.source,
        "out_tuned": row.out_tuned,
        "measured_hz": row.measured_hz,
        "target_hz": row.target_hz,
        "cents_before": row.cents_before,
        "cents_applied": row.cents_applied,
        "rms_before_dbfs": row.rms_before_dbfs,
        "rms_after_dbfs": row.rms_after_dbfs,
        "peak": row.peak,
        "layers": row.layers,
        "status": row.status,
        "reason": row.reason,
    })
}

/// 执行一次采样库编辑（批处理入口）。
///
/// # Errors
///
/// [`SampleEditError::InputUnreadable`]（输入读不到）、
/// [`SampleEditError::NoSamples`]（零个采样成功）、
/// [`SampleEditError::Output`]（输出目录 / 报告落不下去）、
/// [`SampleEditError::Sfz`]（SFZ 解析失败）。
pub fn edit_samples(options: &SampleEditCliOptions) -> Result<SampleEditReport, SampleEditError> {
    std::fs::create_dir_all(&options.out_dir).map_err(|error| SampleEditError::Output {
        path: options.out_dir.clone(),
        reason: format!("建输出目录失败: {error}"),
    })?;
    let (mut jobs, plan) = collect_jobs(options)?;
    if jobs.is_empty() {
        return Err(SampleEditError::NoSamples {
            path: options.input.clone(),
        });
    }
    // 输出词干去重（扁平输出目录里同名会互相覆盖）：按输入顺序，后者加 `_2` / `_3`…。
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for job in jobs.iter_mut() {
        let mut stem = job.stem.clone();
        let mut suffix = 2;
        while !claimed.insert(stem.clone()) {
            stem = format!("{}_{suffix}", job.stem);
            suffix += 1;
        }
        job.stem = stem;
    }
    let target_rms = 10.0_f32.powf(options.target_rms_dbfs / 20.0);
    let mut rows: Vec<SampleRow> = Vec::new();
    let mut tuned_count = 0;
    let mut layers_count = 0;
    for job in jobs.iter_mut() {
        let row = process_one(job, options, target_rms);
        if row.status == "ok" {
            tuned_count += 1;
            layers_count += row.layers.len();
        } else {
            eprintln!(
                "sample-edit-skipped: {} ({})",
                row.source,
                row.reason.as_deref().unwrap_or("")
            );
        }
        rows.push(row);
    }
    if tuned_count == 0 {
        return Err(SampleEditError::NoSamples {
            path: options.input.clone(),
        });
    }
    let report_path = options.out_dir.join("report.json");
    let report_json = serde_json::json!({
        "tool": "edit-samples",
        "input": options.input.display().to_string(),
        "out_dir": options.out_dir.display().to_string(),
        "target_rms_dbfs": options.target_rms_dbfs,
        "report_only": options.report_only,
        "samples": rows.len(),
        "rows": rows.iter().map(row_json).collect::<Vec<_>>(),
    });
    let mut text =
        serde_json::to_string_pretty(&report_json).map_err(|error| SampleEditError::Output {
            path: report_path.clone(),
            reason: format!("报告序列化失败: {error}"),
        })?;
    text.push('\n');
    write_file_atomically(text.as_bytes(), &report_path).map_err(|error| {
        SampleEditError::Output {
            path: report_path.clone(),
            reason: format!("写报告失败: {error}"),
        }
    })?;
    if let Some(plan) = &plan
        && !options.report_only
    {
        let stem = options
            .input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("edited");
        let sfz_out = options.out_dir.join(format!("{stem}_edited.sfz"));
        emit_edited_sfz(plan, &rows, &sfz_out).map_err(|reason| SampleEditError::Output {
            path: sfz_out,
            reason,
        })?;
    }
    Ok(SampleEditReport {
        input: options.input.clone(),
        out_dir: options.out_dir.clone(),
        report_path,
        samples: rows.len(),
        tuned: tuned_count,
        layers: layers_count,
        skipped: rows.len() - tuned_count,
        target_rms_dbfs: options.target_rms_dbfs,
        report_only: options.report_only,
        rows,
    })
}

/// 处理一个采样：解码 → 测基频 → 校正 → 层合成 → 归一 → 落盘。
fn process_one(job: &mut SampleJob, options: &SampleEditCliOptions, target_rms: f32) -> SampleRow {
    let source = job.source.display().to_string();
    // skip 行里测得的值照实留着（`None` = 没测出来，不是"测了个 0"）——
    // 报告的读数永远是实测，不是入参。
    let fail = |measured_hz: Option<f64>, cents_before: Option<f32>, reason: String| SampleRow {
        source: source.clone(),
        out_tuned: None,
        measured_hz,
        target_hz: job.target_hz,
        cents_before,
        cents_applied: 0.0,
        rms_before_dbfs: None,
        rms_after_dbfs: None,
        peak: None,
        layers: Vec::new(),
        status: "skipped",
        reason: Some(reason),
    };
    let (samples, sample_rate, format) = match decode_sample(&job.source) {
        Ok(decoded) => decoded,
        Err(reason) => return fail(None, None, reason),
    };
    job.samples = samples;
    job.sample_rate = sample_rate;
    job.format = format;
    let samples = &job.samples;
    let rms_before = rms(samples);
    let rms_before_dbfs = if rms_before > 0.0 {
        20.0 * rms_before.log10()
    } else {
        f32::NEG_INFINITY
    };
    if rms_before == 0.0 {
        return fail(None, None, "输入全静音".to_owned());
    }
    let measured = measure_fundamental(samples, sample_rate as f32);
    let measured_hz = measured.map(|m| m.hz);
    let target_hz = match job.target_hz {
        Some(target) => Some(target),
        // 目录模式：目标 = 测得基频的最近半音。
        None => measured.map(|m| {
            let midi = (69.0 + 12.0 * (m.hz / 440.0).log2()).round();
            f64::from(note_to_hz(midi as f32))
        }),
    };
    let cents_before = match (measured_hz, target_hz) {
        (Some(measured), Some(target)) => Some(cents_between(measured, target)),
        _ => None,
    };
    // 测不出基频就没法校正（也校无可校）⇒ skip，⛔ 不把"没调过的样本"当 ok 交出去。
    // `--sample-edit-no-tune` 例外：用户明说不要校正，测不测得到都无关。
    if !options.no_tune && cents_before.is_none() {
        return fail(measured_hz, None, "测不出基频".to_owned());
    }
    let mut cents_applied = 0.0_f32;
    let mut tuned: Vec<f32> = samples.clone();
    if !options.no_tune
        && let Some(cents) = cents_before
    {
        let shift = -cents;
        match correct_pitch(samples, sample_rate as f32, shift) {
            Ok((corrected, _)) => {
                tuned = corrected;
                cents_applied = if shift.abs() <= TUNE_DEADBAND_CENTS {
                    0.0
                } else {
                    shift
                };
            }
            Err(error) => return fail(measured_hz, cents_before, format!("{error}")),
        }
    }
    // 层合成：调好音的样本过高通，再按同一目标 RMS 归一。
    let mut layer_files: Vec<(String, Vec<f32>)> = Vec::new();
    if options.layers.mf || options.layers.f {
        match synthesize_layer(&tuned, sample_rate as f32) {
            Ok(layer) => {
                if options.layers.mf {
                    layer_files.push((format!("{}_mf.wav", job.stem), layer.clone()));
                }
                if options.layers.f {
                    layer_files.push((format!("{}_f.wav", job.stem), layer));
                }
            }
            Err(error) => return fail(measured_hz, cents_before, format!("{error}")),
        }
    }
    let (tuned_norm, peak) = match normalize_rms(&tuned, target_rms) {
        Ok(pair) => pair,
        Err(error) => return fail(measured_hz, cents_before, format!("{error}")),
    };
    // 归一后 RMS 写**实测值**，不是目标值 —— 报告的读数永远是实测（量化会让实测
    // 与目标差一点点，差多少由读数自己说，不由入参背书）。
    let rms_after = rms(&tuned_norm);
    let rms_after_dbfs = if rms_after > 0.0 {
        Some(20.0 * rms_after.log10())
    } else {
        None
    };
    let mut layer_rows: Vec<String> = Vec::new();
    if !options.report_only {
        let tuned_name = format!("{}_tuned.wav", job.stem);
        let tuned_path = options.out_dir.join(&tuned_name);
        if let Err(reason) = write_wav_atomically(&tuned_path, &tuned_norm, job.format) {
            return fail(measured_hz, cents_before, reason);
        }
        for (name, layer) in &layer_files {
            let (layer_norm, _) = match normalize_rms(layer, target_rms) {
                Ok(pair) => pair,
                Err(error) => return fail(measured_hz, cents_before, format!("{error}")),
            };
            let layer_path = options.out_dir.join(name);
            if let Err(reason) = write_wav_atomically(&layer_path, &layer_norm, job.format) {
                return fail(measured_hz, cents_before, reason);
            }
            layer_rows.push(name.clone());
        }
    } else {
        // 报告模式：层文件名仍记进报告（"会产出什么"，不写盘）。
        for (name, _) in &layer_files {
            layer_rows.push(name.clone());
        }
    }
    SampleRow {
        source,
        out_tuned: Some(format!("{}_tuned.wav", job.stem)),
        measured_hz,
        target_hz,
        cents_before,
        cents_applied,
        rms_before_dbfs: Some(rms_before_dbfs),
        rms_after_dbfs,
        peak: Some(peak),
        layers: layer_rows,
        status: "ok",
        reason: None,
    }
}
