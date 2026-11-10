//! `--slice-audio` —— 音频切片的批处理出口。
//!
//! ## 它补的是哪一格
//!
//! Wang 2026-11-10 的缺口分级（C 级第 2 条）：`chop.py` 在仓库外做
//! "onset 检测切到字 → tape（变速变调）/shift/stretch 三变形 → 4/10 ms
//! 淡入淡出"，而 yeban 里没有"切成采样器 region"这个动作 ——
//! `import_audio` 只能整条导入（见 [`crate::import_audio`]），
//! `transient` 是瞬态塑形效果器、`anatomy.rs` 是底鼓合成器，
//! 全仓没有任何音频 onset 检测。本模块就是那个动作的编排层：
//! 读 WAV → 切片（onset 检测或显式切点）→ 每片三变形 → 4/10 ms
//! 淡入淡出 → 写成采样器 region（`<stem>.sfz` ＋ 每片一段 WAV，
//! 可被 yeban 的采样器直接加载）。
//!
//! 切片核（onset 检测／tape／淡入淡出）在
//! [`yeban_dsp::slicer`]；shift 与 stretch 复用既有离线器件
//! （[`yeban_dsp::vocal_pitch_shift::PitchShifter`]、
//! [`yeban_dsp::vocal_timemap::render_continuous_timemap`]）。
//!
//! ## 流程口径（与 chop.py 的对应关系）
//!
//! `chop.py` 本身已随 2026-11-09 的清理从工作区删除，流程以 Wang
//! 2026-11-10 报告的一行为准（"onset 检测切到字 → tape（变速变调）/
//! shift/stretch 三变形 → 4/10 ms 淡入淡出"）：
//!
//! 1. 读 WAV：与 `import_audio` 同一条 64 MiB 门（
//!    [`crate::import_audio::MAX_AUDIO_FILE_BYTES`]），超限拒绝不截断；
//! 2. 切点：缺省按 onset 检测（[`yeban_dsp::slicer::detect_onsets`]），
//!    第一个 onset 之前的音频丢弃（收据 `droppedLeadingFrames` 如实报）；
//!    零 onset ⇒ 整条为一片。`--slice-cuts` 给显式切点（秒）时覆盖检测，
//!    第 0 帧起切，什么都不丢；
//! 3. 三变形（三选一，缺省不变形）：`tape` 变速变调（时长与音高同变）、
//!    `shift` 移调（时长不变）、`stretch` 拉伸（音高不变）；
//! 4. 淡入淡出：每片变形**之后**施加，线性淡入 4 ms、淡出 10 ms
//!    （[`yeban_dsp::slicer::apply_fades`]）；
//! 5. 落盘：先写全部切片 WAV（临时文件 → 改名），再原子写 SFZ 文本 ——
//!    中途失败不留 SFZ 指向缺失的 WAV。
//!
//! ## 出口决策
//!
//! * **CLI 无窗口批量**（`--slice-audio`，与 `--export-song-overview`
//!   同类）：切片是批文件操作，不依赖工程／会话状态，CLI 是自然出口。
//! * **不补 MCP 工具**：MCP 工具是工程作用域的（`TOOL_COUNT 38`、
//!   schema 枚举、fault 词表都要跟着动），切片不读工程、不改工程，
//!   加 MCP 工具只会给普查加计数、不给用户加能力。
//! * **不做 GUI**：同上，批操作不需要界面。
//!
//! ## 失败语义
//!
//! 任何一步失败 ⇒ [`AudioSlicerError`]（CLI 侧以 `EXIT_EXPORT` 大声报，
//! 不写部分产物：切片 WAV 写一半时已写好的会被删掉，SFZ 不写）。
//! 输入文件本身不动（只读）。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use yeban_dsp::slicer::{OnsetOptions, SlicerError, apply_fades, detect_onsets, tape_varispeed};
use yeban_dsp::vocal_pitch_shift::{PitchShiftConfig, PitchShifter};
use yeban_dsp::vocal_timemap::{PiecewiseTimeMap, StretchConfig, render_continuous_timemap};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav, write_plain_wav};

use crate::import_audio::MAX_AUDIO_FILE_BYTES;
use crate::save::write_file_atomically;

/// 收据行前缀（CLI 批输出的一行，见 [`SliceReport::line`]）。
pub const RECEIPT_PREFIX: &str = "sliced-audio:";

/// 缺省根音（MIDI 60 = C4）：第 `i` 片的根音是 `base_key + i`。
pub const DEFAULT_BASE_KEY: u8 = 60;

/// 缺省最小切点间隔（毫秒）—— [`yeban_dsp::slicer::DEFAULT_MIN_GAP_MS`] 的重导出。
pub use yeban_dsp::slicer::DEFAULT_MIN_GAP_MS;

/// 缺省检测底线（dBFS）—— [`yeban_dsp::slicer::DEFAULT_FLOOR_DB`] 的重导出。
pub use yeban_dsp::slicer::DEFAULT_FLOOR_DB as DEFAULT_THRESHOLD_DB;

/// tape / stretch 比值合法域（与 [`yeban_dsp::slicer`] 同值）。
pub const RATE_MIN: f32 = yeban_dsp::slicer::TAPE_RATE_MIN;
/// tape / stretch 比值合法域（与 [`yeban_dsp::slicer`] 同值）。
pub const RATE_MAX: f32 = yeban_dsp::slicer::TAPE_RATE_MAX;

/// shift 音分合法域（±1200 音分 = ±一个八度）。
pub const SHIFT_CENTS_MIN: f32 = -1200.0;
/// shift 音分合法域（±1200 音分 = ±一个八度）。
pub const SHIFT_CENTS_MAX: f32 = 1200.0;

/// 切片的三变形（三选一；`None` = 不变形，只切）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SliceTransform {
    /// 不变形。
    #[default]
    None,
    /// 变速变调：`rate` = 输出速率 / 输入速率（`> 1` 变快变高）。
    Tape {
        /// 变速比，[`RATE_MIN`]..=[`RATE_MAX`]。
        rate: f32,
    },
    /// 移调：时长不变，音高变 `cents` 音分。
    Shift {
        /// 移调量（音分），[`SHIFT_CENTS_MIN`]..=[`SHIFT_CENTS_MAX`]。
        cents: f32,
    },
    /// 拉伸：音高不变，时长变 `rate` 倍。
    Stretch {
        /// 拉伸比，[`RATE_MIN`]..=[`RATE_MAX`]。
        rate: f32,
    },
}

impl SliceTransform {
    /// 解析 `--slice-transform` 的规格串：
    /// `none` | `tape:<rate>` | `shift:<cents>` | `stretch:<rate>`。
    ///
    /// # Errors
    ///
    /// 规格串不是上述四形、数值解析失败、数值非有限或越界时返回
    /// 形如 `"tape 的比值 9 越界（合法域 0.25..=4）"` 的原因串
    /// （CLI 侧以 exit 2 报用法错）。
    pub fn parse(spec: &str) -> Result<Self, String> {
        if spec == "none" {
            return Ok(Self::None);
        }
        let (kind, value) = spec.split_once(':').ok_or_else(|| {
            format!("变形规格 `{spec}` 须是 none | tape:<比值> | shift:<音分> | stretch:<比值>")
        })?;
        let number: f32 = value
            .parse()
            .map_err(|_| format!("变形规格 `{spec}` 的数值 `{value}` 不是数字"))?;
        if !number.is_finite() {
            return Err(format!("变形规格 `{spec}` 的数值 `{value}` 不是有限数"));
        }
        match kind {
            "tape" | "stretch" => {
                if !(RATE_MIN..=RATE_MAX).contains(&number) {
                    return Err(format!(
                        "{kind} 的比值 {number} 越界（合法域 {RATE_MIN}..={RATE_MAX}）"
                    ));
                }
                Ok(if kind == "tape" {
                    Self::Tape { rate: number }
                } else {
                    Self::Stretch { rate: number }
                })
            }
            "shift" => {
                if !(SHIFT_CENTS_MIN..=SHIFT_CENTS_MAX).contains(&number) {
                    return Err(format!(
                        "shift 的音分 {number} 越界（合法域 {SHIFT_CENTS_MIN}..={SHIFT_CENTS_MAX}）"
                    ));
                }
                Ok(Self::Shift { cents: number })
            }
            _ => Err(format!(
                "变形规格 `{spec}` 的种类 `{kind}` 未知（只认 tape / shift / stretch）"
            )),
        }
    }

    /// 收据里的变形标签：`none` | `tape:2` | `shift:1200` | `stretch:0.5`。
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::None => "none".to_string(),
            Self::Tape { rate } => format!("tape:{rate}"),
            Self::Shift { cents } => format!("shift:{cents}"),
            Self::Stretch { rate } => format!("stretch:{rate}"),
        }
    }
}

/// `--slice-audio` 的选项（CLI 解析后拼出；缺省见各字段）。
#[derive(Clone, Debug, PartialEq)]
pub struct SliceOptions {
    /// 三变形（缺省不变形）。
    pub transform: SliceTransform,
    /// 第 0 片的根音（MIDI 0..=127，缺省 60）。
    pub base_key: u8,
    /// 显式切点（秒，升序去重后用）：给定时覆盖 onset 检测。
    pub cuts: Option<Vec<f64>>,
    /// onset 检测的最小切点间隔（毫秒，缺省 80）。
    pub min_gap_ms: f32,
    /// onset 检测的绝对底线（dBFS，缺省 −50）。
    pub threshold_db: f32,
}

impl Default for SliceOptions {
    fn default() -> Self {
        Self {
            transform: SliceTransform::None,
            base_key: DEFAULT_BASE_KEY,
            cuts: None,
            min_gap_ms: DEFAULT_MIN_GAP_MS,
            threshold_db: DEFAULT_THRESHOLD_DB,
        }
    }
}

/// 切片失败的原因（调用方如实上报，不静默降级、不写部分产物）。
#[derive(Debug)]
pub enum AudioSlicerError {
    /// 输入不是文件（不存在／是目录）。
    NotAFile {
        /// 输入路径。
        path: PathBuf,
    },
    /// 输入超过 64 MiB 门（与 `import_audio` 同门，拒绝不截断）。
    TooLarge {
        /// 输入路径。
        path: PathBuf,
        /// 输入字节数。
        bytes: u64,
        /// 门限字节数。
        max_bytes: u64,
    },
    /// 读输入文件失败（I/O）。
    Read {
        /// 输入路径。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
    /// WAV 解码失败。
    Decode {
        /// 输入路径。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
    /// 输入是 0 帧（无音频可切）。
    EmptyAudio {
        /// 输入路径。
        path: PathBuf,
    },
    /// 变形失败（`tape`／`shift`／`stretch` 的原因原样透传）。
    Transform {
        /// 原因。
        reason: String,
    },
    /// 显式切点非法（越界／非有限／与总长相等）。
    InvalidCut {
        /// 切点（秒）。
        seconds: f64,
        /// 原因。
        reason: &'static str,
    },
    /// 根音 + 片数超出 MIDI 0..=127（第 `i` 片根音 `base_key + i` 须 ≤ 127）。
    BaseKeyOverflow {
        /// 根音。
        base_key: u8,
        /// 片数。
        slices: usize,
    },
    /// 写切片 WAV 或 SFZ 失败（I/O）。
    Write {
        /// 目标路径。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
}

impl std::fmt::Display for AudioSlicerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFile { path } => write!(formatter, "`{}` 不是文件", path.display()),
            Self::TooLarge {
                path,
                bytes,
                max_bytes,
            } => write!(
                formatter,
                "`{}` 是 {bytes} 字节，超过 64 MiB 门（{max_bytes} 字节），拒绝不截断",
                path.display()
            ),
            Self::Read { path, reason } => {
                write!(formatter, "读 `{}` 失败: {reason}", path.display())
            }
            Self::Decode { path, reason } => {
                write!(formatter, "解码 `{}` 失败: {reason}", path.display())
            }
            Self::EmptyAudio { path } => {
                write!(formatter, "`{}` 是 0 帧，无音频可切", path.display())
            }
            Self::Transform { reason } => write!(formatter, "变形失败: {reason}"),
            Self::InvalidCut { seconds, reason } => {
                write!(formatter, "切点 {seconds} 秒非法: {reason}")
            }
            Self::BaseKeyOverflow { base_key, slices } => write!(
                formatter,
                "根音 {base_key} + {slices} 片超出 MIDI 0..=127（第 {slices} 片根音超限）"
            ),
            Self::Write { path, reason } => {
                write!(formatter, "写 `{}` 失败: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for AudioSlicerError {}

impl From<SlicerError> for AudioSlicerError {
    fn from(error: SlicerError) -> Self {
        Self::Transform {
            reason: error.to_string(),
        }
    }
}

/// 切片收据（CLI 批输出的一行，见 [`SliceReport::line`]）。
#[derive(Clone, Debug, PartialEq)]
pub struct SliceReport {
    /// 输入 WAV 路径。
    pub input: PathBuf,
    /// 切片输出目录。
    pub out_dir: PathBuf,
    /// 生成的 SFZ 路径。
    pub sfz_path: PathBuf,
    /// 切片数。
    pub slices: usize,
    /// 施加的变形。
    pub transform: SliceTransform,
    /// 第 0 片的根音。
    pub base_key: u8,
    /// 丢弃的首段帧数（onset 检测模式下第一个 onset 之前的音频；显式切点模式恒为 0）。
    pub dropped_leading_frames: u64,
}

impl SliceReport {
    /// 收据行：`sliced-audio: input=… out=… sfz=… slices=N transform=… baseKey=… droppedLeadingFrames=…`。
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{RECEIPT_PREFIX} input={} out={} sfz={} slices={} transform={} baseKey={} droppedLeadingFrames={}",
            self.input.display(),
            self.out_dir.display(),
            self.sfz_path.display(),
            self.slices,
            self.transform.label(),
            self.base_key,
            self.dropped_leading_frames
        )
    }
}

/// 切一片音频：读 WAV → 切点 → 每片变形＋淡入淡出 → 写切片 WAV＋SFZ。
///
/// 产物：`<stem>_<nnn>.wav`（32 位浮点 WAV，与输入同采样率同声道）＋
/// `<stem>.sfz`（每片一个 `<region>`，`lokey=hikey=pitch_keycenter=base_key+i`）。
///
/// # Errors
///
/// 见 [`AudioSlicerError`]：输入不是文件／超 64 MiB 门／解码失败／0 帧／
/// 切点非法／根音溢出／变形失败／写盘失败。任何失败都不留部分产物
/// （已写的切片 WAV 会被删掉，SFZ 不写）。
pub fn slice_audio_file(
    input: &Path,
    out_dir: &Path,
    options: &SliceOptions,
) -> Result<SliceReport, AudioSlicerError> {
    let metadata = std::fs::metadata(input).map_err(|error| AudioSlicerError::Read {
        path: input.to_path_buf(),
        reason: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(AudioSlicerError::NotAFile {
            path: input.to_path_buf(),
        });
    }
    if metadata.len() > MAX_AUDIO_FILE_BYTES {
        return Err(AudioSlicerError::TooLarge {
            path: input.to_path_buf(),
            bytes: metadata.len(),
            max_bytes: MAX_AUDIO_FILE_BYTES,
        });
    }
    // 读：与 `import_audio` 同一条路 —— 先看 `metadata.len()` 再决定读不读，
    // 再走 `read_plain_wav` 的路径腿（它自己开文件，不先整份读进内存）。
    let (format, buffer) = read_plain_wav(input).map_err(|error| AudioSlicerError::Decode {
        path: input.to_path_buf(),
        reason: error.to_string(),
    })?;
    let channels = pcm_to_f32_channels(&format, &buffer);
    let frames = channels.first().map_or(0, Vec::len);
    if frames == 0 {
        return Err(AudioSlicerError::EmptyAudio {
            path: input.to_path_buf(),
        });
    }
    let sample_rate = format.sample_rate;

    // 切点（帧下标，严格升序）：显式切点覆盖检测。
    let (boundaries, dropped_leading_frames) = if let Some(cuts) = &options.cuts {
        (explicit_boundaries(cuts, frames, sample_rate)?, 0u64)
    } else {
        let mono = mixdown_mono(&channels);
        let onset_options = OnsetOptions {
            min_gap_ms: options.min_gap_ms,
            floor_db: options.threshold_db,
            ..OnsetOptions::default()
        };
        let onsets = detect_onsets(&mono, sample_rate, &onset_options);
        if onsets.is_empty() {
            (vec![0, frames], 0u64)
        } else {
            // 先取首个 onset（丢弃的首段帧数），再把 `frames` 推进边界表：
            // 元组里"移动 `boundaries`＋借用 `boundaries`"是同一表达式里的
            // 借用冲突，分两步写才合法。
            let dropped = u64::try_from(onsets.first().copied().unwrap_or(0)).unwrap_or(u64::MAX);
            let mut boundaries = onsets;
            boundaries.push(frames);
            (boundaries, dropped)
        }
    };
    let slices = boundaries.len() - 1;
    if options.base_key as usize + slices - 1 > 127 {
        return Err(AudioSlicerError::BaseKeyOverflow {
            base_key: options.base_key,
            slices,
        });
    }

    std::fs::create_dir_all(out_dir).map_err(|error| AudioSlicerError::Write {
        path: out_dir.to_path_buf(),
        reason: error.to_string(),
    })?;
    let stem = input.file_stem().map_or("slice".to_string(), |stem| {
        stem.to_string_lossy().into_owned()
    });

    // 先写全部切片 WAV（临时文件 → 改名），再写 SFZ：中途失败不留
    // SFZ 指向缺失的 WAV；已写的切片 WAV 会被删掉。
    let mut written: Vec<PathBuf> = Vec::new();
    if let Err(error) = write_slice_wavs(
        &boundaries,
        &channels,
        sample_rate,
        options.transform,
        &stem,
        out_dir,
        &mut written,
    ) {
        for path in &written {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }

    // SFZ 文本（每片一个 region；注释行可被 `yeban-sfz` 的解析器剥掉）。
    let mut sfz = String::new();
    let _ = writeln!(
        sfz,
        "// {stem}.sfz — `--slice-audio` 生成：{}，{slices} 片，变形 {}，根音 {} 起",
        input
            .file_name()
            .map_or("input.wav".to_string(), |name| name
                .to_string_lossy()
                .into_owned()),
        options.transform.label(),
        options.base_key
    );
    for (index, path) in written.iter().enumerate() {
        let key = options.base_key as usize + index;
        let file_name = path.file_name().map_or("slice.wav".to_string(), |name| {
            name.to_string_lossy().into_owned()
        });
        let _ = writeln!(
            sfz,
            "<region> sample={file_name} lokey={key} hikey={key} pitch_keycenter={key}"
        );
    }
    let sfz_path = out_dir.join(format!("{stem}.sfz"));
    if let Err(error) = write_file_atomically(sfz.as_bytes(), &sfz_path) {
        for path in &written {
            let _ = std::fs::remove_file(path);
        }
        return Err(AudioSlicerError::Write {
            path: sfz_path,
            reason: error.to_string(),
        });
    }

    Ok(SliceReport {
        input: input.to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        sfz_path,
        slices,
        transform: options.transform,
        base_key: options.base_key,
        dropped_leading_frames,
    })
}

/// 写全部切片 WAV（每片：变形 → 淡入淡出 → 32 位浮点 WAV，临时文件 → 改名）。
/// 成功时 `written` 里是全部切片路径；失败时调用方删掉 `written` 里的已写文件。
#[allow(clippy::too_many_arguments)]
fn write_slice_wavs(
    boundaries: &[usize],
    channels: &[Vec<f32>],
    sample_rate: u32,
    transform: SliceTransform,
    stem: &str,
    out_dir: &Path,
    written: &mut Vec<PathBuf>,
) -> Result<(), AudioSlicerError> {
    for (index, window) in boundaries.windows(2).enumerate() {
        let (start, end) = (window[0], window[1]);
        let mut transformed: Vec<Vec<f32>> = Vec::new();
        for channel in channels {
            let piece = transform_slice(&channel[start..end], sample_rate, transform)?;
            transformed.push(piece);
        }
        for piece in transformed.iter_mut() {
            apply_fades(piece, sample_rate);
        }
        let interleaved = interleave(&transformed);
        let out_format = PcmFormat::float(
            u16::try_from(transformed.len()).unwrap_or(u16::MAX),
            sample_rate,
            32,
        );
        let name = format!("{stem}_{index:03}.wav");
        let target = out_dir.join(&name);
        let temp = out_dir.join(format!("{name}.tmp-{}", std::process::id()));
        // `write_plain_wav` 直接写盘（路径腿）：先写临时文件，再改名 ——
        // 校验在 `hound` 建文件之前完成，被拒的缓冲不留字节；写一半失败时
        // 临时文件随手删掉，不留半成品。
        write_plain_wav(&temp, &out_format, &PcmBuffer::Float32(interleaved)).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            AudioSlicerError::Write {
                path: temp.clone(),
                reason: error.to_string(),
            }
        })?;
        std::fs::rename(&temp, &target).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            AudioSlicerError::Write {
                path: target.clone(),
                reason: error.to_string(),
            }
        })?;
        written.push(target);
    }
    Ok(())
}

/// 显式切点（秒）→ 帧边界 `[0] + 切点 + [frames]`（严格升序去重）。
fn explicit_boundaries(
    cuts: &[f64],
    frames: usize,
    sample_rate: u32,
) -> Result<Vec<usize>, AudioSlicerError> {
    let duration_s = frames as f64 / sample_rate as f64;
    let mut points: Vec<usize> = Vec::new();
    for &seconds in cuts {
        if !seconds.is_finite() {
            return Err(AudioSlicerError::InvalidCut {
                seconds,
                reason: "不是有限数",
            });
        }
        if seconds <= 0.0 {
            return Err(AudioSlicerError::InvalidCut {
                seconds,
                reason: "须 > 0（0 秒即文件起点，不是切点）",
            });
        }
        if seconds >= duration_s {
            return Err(AudioSlicerError::InvalidCut {
                seconds,
                reason: "须 < 总时长（总时长处即文件终点，不是切点）",
            });
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = (seconds * sample_rate as f64).round() as usize;
        if at == 0 || at >= frames {
            return Err(AudioSlicerError::InvalidCut {
                seconds,
                reason: "换算后落在文件两端",
            });
        }
        points.push(at);
    }
    points.sort_unstable();
    points.dedup();
    let mut boundaries = vec![0];
    boundaries.extend(points);
    boundaries.push(frames);
    Ok(boundaries)
}

/// PCM 缓冲 → 每声道 `f32`（交错展开；Int16/Int24 按满幅归一化）。
fn pcm_to_f32_channels(format: &PcmFormat, buffer: &PcmBuffer) -> Vec<Vec<f32>> {
    let channels = usize::from(format.channels);
    let mut out: Vec<Vec<f32>> = vec![Vec::new(); channels.max(1)];
    // 声道数先取出来：`out[index % out.len()]` 是"借 `out` 不可变（取长）＋
    // 借 `out` 可变（索引写）"的同一表达式借用冲突，分开写才合法。
    let n = out.len();
    match buffer {
        PcmBuffer::Int16(samples) => {
            for (index, sample) in samples.iter().enumerate() {
                out[index % n].push(f32::from(*sample) / 32768.0);
            }
        }
        PcmBuffer::Int24(samples) => {
            for (index, sample) in samples.iter().enumerate() {
                out[index % n].push(*sample as f32 / 8_388_608.0);
            }
        }
        PcmBuffer::Float32(samples) => {
            for (index, sample) in samples.iter().enumerate() {
                out[index % n].push(*sample);
            }
        }
    }
    out
}

/// 多声道混单声道（逐帧取均值，只用于 onset 检测）。
fn mixdown_mono(channels: &[Vec<f32>]) -> Vec<f32> {
    let frames = channels.first().map_or(0, Vec::len);
    let mut mono = vec![0.0f32; frames];
    for channel in channels {
        for (index, sample) in channel.iter().enumerate() {
            mono[index] += *sample;
        }
    }
    let count = channels.len().max(1) as f32;
    for sample in mono.iter_mut() {
        *sample /= count;
    }
    mono
}

/// 每声道样本 → 交错（帧主序），与输入声道数一致。
fn interleave(channels: &[Vec<f32>]) -> Vec<f32> {
    let frames = channels.first().map_or(0, Vec::len);
    let mut out = Vec::with_capacity(frames * channels.len());
    for frame in 0..frames {
        for channel in channels {
            out.push(channel[frame]);
        }
    }
    out
}

/// 对一片单声道音频施加三变形（`None` = 原样返回；`shift`/`stretch`
/// 的器件只用于离线，见各模块文档）。
fn transform_slice(
    samples: &[f32],
    sample_rate: u32,
    transform: SliceTransform,
) -> Result<Vec<f32>, AudioSlicerError> {
    match transform {
        SliceTransform::None => Ok(samples.to_vec()),
        SliceTransform::Tape { rate } => Ok(tape_varispeed(samples, rate)?),
        SliceTransform::Shift { cents } => {
            // `PitchShifter::new` 在采样率 < 1000 Hz 时 panic：先拦，不靠 panic。
            if sample_rate < 1000 {
                return Err(AudioSlicerError::Transform {
                    reason: format!("采样率 {sample_rate} Hz < 1000 Hz，shift 器件拒绝"),
                });
            }
            let config = PitchShiftConfig {
                shift_cents: cents,
                // 切片本身就是唱出的音节（整段浊音），且片尾另有 4/10 ms
                // 淡入淡出：浊音／清音过渡淡入淡出（`transition_fade_s`）关掉，
                // 不在器件里再叠一层淡入淡出。
                transition_fade_s: 0.0,
                ..PitchShiftConfig::default()
            };
            let mut shifter = PitchShifter::new(sample_rate as f32, config);
            let n_chunks = samples.len().div_ceil(config.chunk_samples);
            let voiced = vec![true; n_chunks];
            let (out, _) = shifter.process(samples, &voiced);
            Ok(out)
        }
        SliceTransform::Stretch { rate } => {
            // 均匀映射（单段直线）：输出时间 `out_s` 读输入时间 `in_s`。
            // `render_continuous_timemap` 用固定种子（`STRETCH_SEED`），确定。
            let in_s = samples.len() as f64 / sample_rate as f64;
            let out_s = in_s * f64::from(rate);
            let map = PiecewiseTimeMap::new(vec![(0.0, 0.0), (out_s, in_s)]);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let target_len = (samples.len() as f64 * f64::from(rate)).round() as usize;
            let (out, _) = render_continuous_timemap(
                samples,
                sample_rate,
                &map,
                target_len,
                StretchConfig::PresetDefault,
                None,
            );
            Ok(out)
        }
    }
}
