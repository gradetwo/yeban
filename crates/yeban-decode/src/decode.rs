//! symphonia 解码前端：把任意支持的输入变成 [`DecodedAsset`]。
//!
//! ## 支持的格式（**明确清单**，见 `Cargo.toml` 的 feature 列表与 notes §3）
//!
//! | 容器 | symphonia feature | 是否启用 | 依据 |
//! | :--- | :--- | :--- | :--- |
//! | RIFF/WAVE | `wav` | ✅ | `ROAD-M-1-004` 点名的标准 PCM WAV |
//! | FLAC (native) | `flac` | ✅ | `ROAD-M-1-004` 点名的 FLAC |
//! | OGG | `ogg` | ✅ | symphonia 上游默认集里的开放标准容器 |
//! | Matroska/WebM | `mkv` | ❌ | 视频容器；缩小不可信输入的解析面 |
//! | AIFF | `aiff` | ❌ | 上游默认关闭，规范未点名 |
//! | CAF / ISO-MP4 | `caf` / `isomp4` | ❌ | 上游默认关闭，规范未点名 |
//! | MP1/MP2/MP3 | `mp3` / `mpa` | ❌ | 有损且需要额外 feature，规范未点名 |
//! | AAC-LC / ALAC | `aac` / `alac` | ❌ | 上游默认关闭，规范未点名 |
//!
//! 编解码器：`pcm` / `adpcm` / `flac` / `vorbis` 已启用；元数据读取器
//! `ape` / `id3v1` / `id3v2` 保持与上游默认集一致。
//! **SIMD 优化 feature（`opt-simd-*`）一律不启用**：它会拉入 `rustfft`，并让
//! 解码路径依赖运行时 CPU 探测 —— 后者与 [ARCH-DET-001] 的 L1/L2 确定性目标相悖。
//!
//! ## 边界（[ARCH-TOP-002] / [ARCH-RT-001]）
//!
//! 本模块**只**在后台线程/池里被调用。它按设计会分配、会做阻塞式文件 I/O —— 这正是
//! 它不能出现在 cpal 实时回调路径上的原因。实时线程只读已经就绪的
//! [`DecodedAsset`]（不可变资产），不做解码、不做重采样。
//!
//! ## 不可信字节的两条"内容级"拒绝
//!
//! 除了尺寸/布局闸门，本模块还拒绝两类**内容**本身不可信的输入：
//!
//! 1. 会让上游 RIFF 解析器整型溢出的 `fmt ` 声明（见 [`precheck_riff_wave_fmt`]）；
//! 2. **浮点容器里的非有限样本（NaN / ±∞）** —— IEEE float 的 `data` 块体是任意字节，
//!    而含 NaN 的资产会毒化整条混音总线，下游没有任何一处能把它变回有限值。判据
//!    `tests::a_float_container_with_non_finite_samples_is_refused`。整型格式不可能
//!    产出非有限值，因此这一条只对浮点容器付出一次扫描的代价。

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

use crate::asset::{DecodeFacts, DecodedAsset, PcmFormat};
use crate::duration;
use crate::error::{DecodeError, DecodeResult};
use crate::limits::{self, LimitViolation, PcmBudget};

/// 一次解码的预算与严格度。
///
/// 资源上限**不再**是散落在常量里的写死数字：全部收进 [`PcmBudget`]（`HD-24`）。
/// 调用方可以只改 `.budget` 一项来收紧/放宽，其余严格度旋钮保持默认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// 全部资源上限（输入字节 / PCM 字节 / 声道数 / 采样率 / 时长）。
    ///
    /// 默认 [`PcmBudget::default`]：96 kHz 立体声 3 小时（= 96 kHz 8 声道 45 分钟）
    /// 的 PCM 预算，推导依据见 [`limits`] 的模块文档。
    pub budget: PcmBudget,
    /// 声明帧数与解出帧数允许的差。默认 [`limits::DURATION_TOLERANCE_FRAMES`]（0）。
    pub duration_tolerance_frames: u64,
    /// 是否把声明/解出的不一致当成**错误**（而不是只记进元数据）。
    ///
    /// 默认 `true`：时长是资产元数据的一部分，猜出来的时长会一路传到卷帘视口与
    /// 剪辑边界。把它设成 `false` 只有一个正当用途 —— 让调用方先看清双方数字。
    pub verify_declared_duration: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            budget: PcmBudget::default(),
            duration_tolerance_frames: limits::DURATION_TOLERANCE_FRAMES,
            verify_declared_duration: true,
        }
    }
}

/// 解码磁盘上的文件。
///
/// 先看文件长度是否在预算内（**在打开/解码之前**），再用 `File` 直接流式解码，
/// 因此不会把整个文件读进内存。
///
/// # Errors
///
/// 见 [`DecodeError`]：I/O、格式不支持、畸形流、超预算、声明时长不一致等。
pub fn decode_path(path: &Path, options: &DecodeOptions) -> DecodeResult<DecodedAsset> {
    let len = std::fs::metadata(path)?.len();
    limits::check_input_len(len, &options.budget)?;
    let file = File::open(path)?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    decode_source(Box::new(file), &hint, options)
}

/// 解码内存里的一段字节（不复制输入，借用即可）。
///
/// # Errors
///
/// 同 [`decode_path`]。
pub fn decode_bytes(bytes: &[u8], options: &DecodeOptions) -> DecodeResult<DecodedAsset> {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    limits::check_input_len(len, &options.budget)?;
    decode_source(Box::new(Cursor::new(bytes)), &Hint::new(), options)
}

/// 解码任意 `Read + Seek` 输入。
///
/// symphonia 只为 `File` 与 `Cursor<_>` 提供 `MediaSource` 实现，因此任意 reader 会被
/// 包进 [`MeasuredSource`]：它在构造时量出流长度（供容器解析"总帧数/总字节数"），
/// 并原样转发 `read`/`seek`。
///
/// # Errors
///
/// 同 [`decode_path`]；reader 自身的 I/O 失败也会原样上报。
pub fn decode_reader<R>(reader: R, options: &DecodeOptions) -> DecodeResult<DecodedAsset>
where
    R: Read + Seek + Send + Sync,
{
    let source = MeasuredSource::new(reader)?;
    if let Some(len) = source.byte_len() {
        limits::check_input_len(len, &options.budget)?;
    }
    decode_source(Box::new(source), &Hint::new(), options)
}

/// 把一个已经擦除类型的媒体源解码为资产。
///
/// 这是所有入口的公共下游；`hint` 用于给探测器一点容器线索（可为空）。
///
/// 这里**显式写出 `'s`**，不让 `Box<dyn MediaSource>` 落到类型默认的 `'static`：
/// 内存入口传进来的是 `Cursor<&[u8]>`，它借用调用方的切片，不可能是 `'static`。
///
/// **不可回退的源**（`is_seekable() == false`）会先被整份读进内存，再当成可回退的源
/// 解码。理由与代价见下面的注释；上限就是调用方的 [`PcmBudget::max_input_bytes`]。
///
/// 第五道闸门（输入容器字节）由本函数**自己**对每个源判一次：凡是 [`MediaSource::byte_len`]
/// 报得出长度的源，超预算都在探测之前返回 [`LimitViolation::InputTooLarge`]。
/// [`decode_path`] / [`decode_bytes`] / [`decode_reader`] 在调用本函数之前已经对同一个数字
/// 判过一次（因此对它们是一个恒真的重复判定），但本函数是**公共**入口 ——
/// 直接调用它的调用方不会经过那三次预检。
///
/// # Errors
///
/// 见 [`DecodeError`]。
pub fn decode_source<'s>(
    mut source: Box<dyn MediaSource + 's>,
    hint: &Hint,
    options: &DecodeOptions,
) -> DecodeResult<DecodedAsset> {
    // 预检与 symphonia 的解封装都要"先读一段、再从头读一遍"（[`precheck_riff_wave_fmt`]
    // 扫完块头会把游标还给起点）。对**不可回退**的源，旧实现的选择是"预检直接放弃"
    // （`!is_seekable` ⇒ `Ok(())`），于是上游那两处未检查算术完全不受本条闸门约束。
    //
    // 2026-10-09 实测：那个 fail-open 是**可达**的，而且不是"理论上的第三方源" ——
    // symphonia 的 `impl MediaSource for std::fs::File` 把 `is_seekable()` 定义为
    // `metadata().is_file()`，所以一条 **FIFO** 路径（`decode_path` / `import_path` 收
    // 路径，`metadata().len()` 为 0 因而输入字节闸门放行）就落在这条分支上：
    //   · 合法 WAV（44 字节）经不可回退源 ⇒ `Ok(frames=2)`（**今天能解**）；
    //   · 33 声道 `WAVE_FORMAT_EXTENSIBLE` 经不可回退源 ⇒ panic（`wave/chunks.rs` 690）；
    //   · 32769 声道 tag 1 经不可回退源 ⇒ panic（`wave/chunks.rs` 100）。
    // 因此这里不能 fail closed（那是对第一行能力的真实回归），改为把输入读进内存：
    // 读完之后源就是可回退的，预检与解封装都恢复原状，而且这条路径**第一次**受输入字节
    // 预算约束（此前它绕过了 [`limits::check_input_len`]）。
    //
    // 代价（诚实声明）：不可回退的源的峰值内存多了**一份输入容器字节**，上限是预算里的
    // `max_input_bytes`。可回退的源（本 crate 的三个入口都是）一个字节都不多读。
    let mut source: Box<dyn MediaSource + 's> = if source.is_seekable() {
        source
    } else {
        let bytes = slurp_unseekable(&mut *source, options.budget.max_input_bytes)?;
        Box::new(Cursor::new(bytes))
    };
    // 第五道闸门（输入容器字节）：**取到源之后**在这里判一次 —— 对可回退的源，这是它
    // 唯一的一次判定（`decode_source` 是公共入口，直接调用它的调用方不经过
    // [`decode_path`] / [`decode_bytes`] / [`decode_reader`] 的那三次预检）。少了这一判，
    // 同一份 52 字节的 WAV 经 `decode_bytes` 会被 [`LimitViolation::InputTooLarge`] 挡下、
    // 经 `decode_source` 却能解出资产（实测读数见判据
    // `a_seekable_source_obeys_the_input_byte_budget_through_decode_source`）。
    // `byte_len()` 是可选能力（管道常常报 `None`），因此这里只判"报得出长度的源"；
    // 不可回退的源由 [`slurp_unseekable`] **边读边判**，本行对它是恒真的重复判定
    // （slurp 之后是 `Cursor`，它报的长度已经在上限之内）。
    if let Some(len) = source.byte_len() {
        limits::check_input_len(len, &options.budget)?;
    }
    // 上游的 RIFF/WAVE 解析器有两处畸形声明会整型溢出并 panic（`u16` 乘法与 `u32` 移位）。
    // 本 crate 的契约是"不可信输入只返回 DecodeError"，而我们不能改上游，因此在探测之前
    // 先把 `fmt ` 块走一遍。
    precheck_riff_wave_fmt(&mut *source)?;
    let stream = MediaSourceStream::new(source, Default::default());
    let mut reader = symphonia::default::get_probe()
        .probe(
            hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|err| match err {
            SymphoniaError::Unsupported(_) => DecodeError::UnsupportedFormat,
            other => DecodeError::from_symphonia(&other),
        })?;

    // 把一个轨道需要的一切**拷贝出来**，从而立刻结束对 `reader` 的不可变借用 ——
    // 后面的 `next_packet` 需要 `&mut reader`。
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let declared_frames = track.num_frames;
    let encoder_delay_frames = track.delay;
    let encoder_padding_frames = track.padding;
    let codec_params = track
        .codec_params
        .clone()
        .ok_or(DecodeError::MissingCodecParameters)?;

    let audio_params = codec_params.audio().ok_or(DecodeError::NoAudioTrack)?;
    let sample_rate = audio_params
        .sample_rate
        .ok_or(DecodeError::MissingSampleRate)?;
    let declared_bit_depth = audio_params
        .bits_per_sample
        .and_then(|b| u16::try_from(b).ok());
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|err| DecodeError::from_symphonia(&err))?;

    // 时长闸门在开始解码之前**按声明**判一次（畸形文件常在头里就声称几小时）。传
    // `channels = 1` 是有意的 —— 这一层只判"每声道帧数 × 4 字节"是否超预算，
    // 声道数还不知道（要等第一个解码缓冲），因此不会在这里误判声道相关的分支。
    // 时长闸门与声道数无关（`frames / sample_rate`），因此在这里判它是准确的。
    //
    // ⚠ 只判这一层是不够的：`declared_frames` 可以是 `None`（FLAC 的
    // `STREAMINFO.total_samples == 0` 就是"未知"的规范写法，实测读数见判据
    // `the_duration_gate_fires_on_undeclared_frames_while_decoding`）。那种输入会
    // 绕过上面这次预检，而**下面那个循环只按字节预算逐包判** —— 于是"6 小时"这道闸门
    // 在整段解码期间不生效，峰值缓冲由 `max_pcm_bytes` 而不是时长预算决定（8 kHz
    // 单声道下字节预算允许约 75 小时，是时长上界的 12.6×）。这正是
    // `docs/ledger/decode-limits-notes.md` §2.2 说的"低采样率 × 少声道：字节便宜、
    // 时间昂贵"那一格，只是它当时假设声明帧数总是存在。因此循环里用**同一条**公式
    // （[`PcmBudget::max_duration_frames`]）逐包判它。
    //
    // ⚠ 同一处注释里的另一半：这道预检**不能**挂在 `if let Some(frames) = declared_frames`
    // 下面。`declared_frames == None` 时那一次跳过会把"采样率是 0"的声明原样放进循环，
    // 而循环的时长闸门要拿 `projected_frames / sample_rate` 去组错误文案 —— 0 Hz 下
    // `max_duration_frames` 恒为 0，于是**第一个**非空包就进错误分支，撞上那次除零
    // （实测读数 `attempt to divide by zero`，判据
    // `a_zero_sample_rate_stream_is_refused_not_a_panic`）。0 Hz 是可达的：WAV 的
    // `fmt ` 把 `sampleRate` 原样交上来、`data` 块声明 `0xFFFF_FFFF` 又让帧数变成
    // "未知"。因此这里无条件判一次，`unwrap_or(0)` 只把"未知帧数"折算成 0 帧 ——
    // 时长与 PCM 字节两道闸门对 0 帧恒真，采样率与声道数两道照常生效。
    limits::check_layout(
        1,
        sample_rate,
        declared_frames.unwrap_or(0),
        &options.budget,
    )?;
    let max_duration_frames = options.budget.max_duration_frames(sample_rate);

    let sample_budget = options.budget.interleaved_samples_limit();
    let mut samples: Vec<f32> = Vec::new();
    let mut layout: Option<(u16, PcmFormat)> = None;
    // 已解出的**帧数**（每声道样本数）。时长闸门以帧为单位，因此这里显式记账，
    // 而不是每轮用 `samples.len() / channels` 反算（除法在循环里，且 `channels`
    // 要到第一个缓冲才知道）。
    let mut decoded_frames: u64 = 0;

    // [MUST-GATE-011] 不推进闸门 —— 见 [`limits::MAX_IDLE_PACKETS`] 的完整实测理由。
    let mut idle_packets = limits::IdleGuard::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(err) => return Err(DecodeError::from_symphonia(&err)),
        };
        if packet.track_id != track_id {
            bump_idle(&mut idle_packets)?;
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // 与 symphonia 官方示例一致：单个坏 packet 跳过而不是整段失败。
            // 这条宽容只作用于**包内**，不改动任何已解出的样本，因此不破坏确定性；
            // 它也不会变成挂死 —— 所有"这一轮没出样本"的路径都过 `bump_idle`。
            Err(SymphoniaError::DecodeError(_)) => {
                bump_idle(&mut idle_packets)?;
                continue;
            }
            Err(err) => return Err(DecodeError::from_symphonia(&err)),
        };

        let frames_in_buffer = buffer.frames();
        let planes = buffer.num_planes();
        let total = buffer.samples_interleaved();
        if frames_in_buffer == 0 || total == 0 {
            bump_idle(&mut idle_packets)?;
            continue;
        }
        // 缓冲自述的形状先自洽，再谈预算：一个自相矛盾的缓冲不该被拿去算时长。
        if planes == 0 || frames_in_buffer.checked_mul(planes) != Some(total) {
            return Err(DecodeError::InconsistentLayout {
                detail: format!(
                    "buffer reports {total} interleaved samples but {frames_in_buffer} frames \
                     x {planes} planes"
                ),
            });
        }
        // 时长闸门逐包生效（数值与终检同源）。用投影值判定：这一包真的加进去之后会到
        // 多少帧。判定放在**分配之前**，因此"没有声明帧数"的输入不会先把缓冲涨到字节
        // 预算才被拒。
        let projected_frames =
            decoded_frames.saturating_add(u64::try_from(frames_in_buffer).unwrap_or(u64::MAX));
        if projected_frames > max_duration_frames {
            return Err(DecodeError::Budget(LimitViolation::DurationTooLong {
                frames: projected_frames,
                sample_rate,
                seconds: projected_frames / u64::from(sample_rate),
                limit_secs: options.budget.max_duration_secs,
            }));
        }
        let channels = u16::try_from(planes).map_err(|_| {
            DecodeError::Budget(LimitViolation::TooManyChannels {
                channels: u16::MAX,
                limit: options.budget.max_channels,
            })
        })?;
        let buffer_format = PcmFormat::from_buffer(&buffer);
        match layout {
            None => {
                limits::check_layout(channels, sample_rate, 0, &options.budget)?;
                layout = Some((channels, buffer_format));
            }
            Some((locked_channels, locked_format)) => {
                if locked_channels != channels {
                    return Err(DecodeError::InconsistentLayout {
                        detail: format!(
                            "channel count changed mid-stream: {locked_channels} -> {channels}"
                        ),
                    });
                }
                if locked_format != buffer_format {
                    return Err(DecodeError::InconsistentLayout {
                        detail: format!(
                            "sample format changed mid-stream: {locked_format:?} -> {buffer_format:?}"
                        ),
                    });
                }
            }
        }

        // [ARCH-SEC-003] 尺寸上限发生在**分配之前**，并且用 `try_reserve` 把
        // "分配器说不" 变成错误而不是 abort。
        let projected =
            samples
                .len()
                .checked_add(total)
                .ok_or(DecodeError::InconsistentLayout {
                    detail: "decoded sample count overflowed usize".to_owned(),
                })?;
        let projected = u64::try_from(projected).unwrap_or(u64::MAX);
        if projected > sample_budget {
            return Err(DecodeError::Budget(LimitViolation::PcmBudgetExceeded {
                frames: projected / u64::from(channels),
                channels,
                samples: projected,
                limit_samples: sample_budget,
            }));
        }
        samples.try_reserve(total).map_err(|_| {
            DecodeError::Budget(LimitViolation::AllocationRefused {
                samples: u64::try_from(total).unwrap_or(u64::MAX),
            })
        })?;
        let start = samples.len();
        samples.resize(start + total, 0.0);
        buffer.copy_to_slice_interleaved(&mut samples[start..]);
        // [ARCH-SEC-003] 浮点样本必须是**有限**的。一个 IEEE 754 位型可以编码 NaN 或
        // ±∞，而"解出一份含非有限样本的资产"会把 NaN 顺着渲染链带进整条混音总线 ——
        // 本 crate 的产物是不可变资产，下游（重采样、量化、电平表、母带导出）没有任何
        // 一处能把它变回有限值。整数格式（PCM / FLAC）不可能产出非有限值，因此这道
        // 检查只对浮点容器（IEEE float WAV、Vorbis）付出一次扫描的代价。
        //
        // 判定紧跟拷贝：这一包的样本已经在缓冲里，因此报错能精确指出帧号与声道号，
        // 而返回值仍然只是一个 `DecodeError`（不可信输入零 panic）。`planes == 0` 已在
        // 上面被拒，所以 `channels >= 1`；`max(1)` 只是让这条除法在任何情况下都不可能
        // 除零。
        //
        // ⚠ 2026-10-10 更正（由本线新增的判据
        // `a_non_finite_float_sample_past_the_first_packet_is_refused_and_named` 的**首次
        // 运行**读出来）：上面那句"精确指出帧号与声道号"此前**只在第一个缓冲里成立**。
        // `position` 返回的是包内下标，而这一包在 `samples` 里的起点是 `start`；上游
        // RIFF 读端每包 1152 帧，于是"第三包的样本 1391"被报成 `frame 695 channel 1`，
        // 而它真实的帧号是 2999。判据把两个数字都钉住 ⇒ 这里必须把 `start` 加回去。
        if buffer_format.is_float()
            && let Some(offset) = samples[start..].iter().position(|s| !s.is_finite())
        {
            let offset = u64::try_from(start.saturating_add(offset)).unwrap_or(u64::MAX);
            let per_frame = u64::from(channels).max(1);
            return Err(DecodeError::Malformed {
                detail: format!(
                    "decoded float sample {offset} is not finite (NaN or ±inf): frame {} \
                     channel {}; a non-finite sample would poison every downstream stage",
                    offset / per_frame,
                    offset % per_frame
                ),
            });
        }
        decoded_frames = projected_frames;
        // 这一轮真的推进了，闸门清零。
        idle_packets.reset();
    }

    let (channels, pcm_format) = layout.ok_or(DecodeError::EmptyStream)?;
    let frames = u64::try_from(samples.len() / usize::from(channels)).unwrap_or(u64::MAX);
    if frames == 0 {
        return Err(DecodeError::EmptyStream);
    }
    limits::check_layout(channels, sample_rate, frames, &options.budget)?;

    let outcome = duration::reconcile(declared_frames, frames, options.duration_tolerance_frames);
    if options.verify_declared_duration {
        outcome.into_result()?;
    }

    Ok(DecodedAsset::new(
        DecodeFacts {
            channels,
            sample_rate,
            pcm_format,
            declared_bit_depth,
            declared_frames,
            encoder_delay_frames,
            encoder_padding_frames,
            duration: outcome,
        },
        samples,
    ))
}

/// RIFF 块头的扫描预算（畸形文件可以声明无数个 0 长度块）。
///
/// 语义：这是**预算**，不是"超过就放行"。预算用尽仍未找到 `fmt ` 时，
/// [`scan_riff_for_fmt`] **保守拒绝**该文件 —— 预算的存在理由就是"不能再往下走"，
/// 而把没看过的字节交给上游解析器，等于把 [`precheck_riff_wave_fmt`] 要挡的那次
/// `u16` 溢出乘法重新暴露出来。代价写在 [`precheck_riff_wave_fmt`] 的文档里。
const RIFF_PRECHECK_MAX_CHUNKS: u32 = 4_096;

/// RIFF/WAVE 的 `fmt ` 块里 `WAVE_FORMAT_PCM` 的格式标签（`wFormatTag = 0x0001`）。
///
/// 上游那次 `num_channels * (bits_per_sample / 8)` 的 `u16` 乘法只有本标签会走到。
const WAVE_FORMAT_PCM: u16 = 0x0001;

/// [`slurp_unseekable`] 一次从源里读入的字节数：64 KiB。
///
/// 与 [`crate::asset`] 的摘要缓冲同一个量级：决定的是 **I/O 次数**，与被读的容器长度无关。
const SLURP_CHUNK_BYTES: usize = 64 * 1024;

/// RIFF/WAVE 的 `fmt ` 块里 `WAVE_FORMAT_EXTENSIBLE` 的格式标签（`wFormatTag = 0xFFFE`）。
///
/// 单独命名，是因为这个标签有**自己的**一处上游未检查算术：`read_ext_fmt` 之后调用
/// `fix_wave_channel_mask`，其中 `1u32 << (num_channels - channel_mask.count_ones())` 在
/// 差值达到 32 时移位溢出（现位于 `wave/chunks.rs` 第 690 行）。tag 1 的那次 `u16` 乘法
/// 与它是**两个**不同的落点，因此必须分开判。
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// `fmt ` 块里 `WAVE_FORMAT_EXTENSIBLE` 的 `sub_format_guid`：会用**声道掩码**定位声道的
/// 四个取值。
///
/// 上游 `read_ext_fmt` 只对这四个 GUID 调用 `fix_wave_channel_mask`（也就是唯一会移位
/// 溢出的地方）；另外两个 Ambisonic GUID 走 `map_amb_channel_count`（无移位），其余 GUID
/// 直接 `unsupported_error`。这四个常量因此必须是**逐字节**正确的：认错一个方向就会
/// 要么漏判（放行 ⇒ 上游 panic），要么误拒（Ambisonic 文件被当成掩码文件）。
/// 漏判那一侧由判据
/// `a_wave_extensible_fmt_that_shift_overflows_is_refused_not_a_panic` 逐个 GUID 钉住
/// （四个 GUID 各要被拒一次）；误拒那一侧由
/// `the_extensible_gate_refuses_only_the_shape_that_really_shifts_over` 的 Ambisonic 与
/// 未知 GUID 用例钉住。
const WAVE_SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

/// `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`（同 [`WAVE_SUBTYPE_PCM`] 的说明）。
const WAVE_SUBTYPE_IEEE_FLOAT: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

/// `KSDATAFORMAT_SUBTYPE_ALAW`（同 [`WAVE_SUBTYPE_PCM`] 的说明）。
const WAVE_SUBTYPE_ALAW: [u8; 16] = [
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

/// `KSDATAFORMAT_SUBTYPE_MULAW`（同 [`WAVE_SUBTYPE_PCM`] 的说明）。
const WAVE_SUBTYPE_MULAW: [u8; 16] = [
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

/// `KSDATAFORMAT_SUBTYPE_AMBISONIC_B_FORMAT_PCM`：**不**走声道掩码那条路（无移位）。
///
/// 只出现在判据里：本闸门对它与"未知 GUID"是同一个结论，因此非测试代码不认它（见
/// [`reaches_the_wave_channel_mask_fix`] 的文档）。判据把它写出来，是为了钉住"闸门认的
/// 那四个 GUID 与 Ambisonic 不是同一份字节"。
#[cfg(test)]
const WAVE_SUBTYPE_AMBISONIC_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x21, 0x07, 0xd3, 0x11, 0x86, 0x44, 0xc8, 0xc1, 0xca, 0x00, 0x00, 0x00,
];

/// `KSDATAFORMAT_SUBTYPE_AMBISONIC_B_FORMAT_IEEE_FLOAT`：**不**走声道掩码那条路。
#[cfg(test)]
const WAVE_SUBTYPE_AMBISONIC_IEEE_FLOAT: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x21, 0x07, 0xd3, 0x11, 0x86, 0x44, 0xc8, 0xc1, 0xca, 0x00, 0x00, 0x00,
];

/// 在探测之前拒掉会让上游 RIFF 解析器整型溢出的 `fmt ` 声明。
///
/// 上游事实（逐行读源码得到；本 crate **不能**修改上游）——**两处**未检查算术：
///
/// 1. `symphonia-format-riff-0.6.1/src/wave/chunks.rs:100`
///    `let expected_block_align = num_channels * (bits_per_sample / 8);`
///    两个操作数都是 `u16`。乘积大于 `u16::MAX` 时，debug 与 release 都会 panic ——
///    根 `Cargo.toml` 的 `[profile.release] overflow-checks = true` 让它在生产构建里也炸。
///    那次乘法只有**一个**调用点：`WAVE_FORMAT_PCM`（tag `0x0001`，同文件 `chunks.rs:438-440`）。
///    复现基线：44 字节 WAV 头 + 2 个 16-bit 样本（共 48 字节），`num_channels = 32769` ⇒
///    `32769 × 2 = 65538 > 65535`。
/// 2. `fix_wave_channel_mask`（同文件第 682 行起）的 `1u32 << channel_diff`（第 690 行）：
///    tag `0xFFFE`（`WAVE_FORMAT_EXTENSIBLE`）走 `read_ext_fmt` 之后调用它，
///    `num_channels - channel_mask.count_ones() >= 32` 即移位溢出。这条与第 1 条**不是**
///    同一个落点：tag 1 的那次 `u16` 乘法挡不住它。复现基线：**134 字节**、
///    `fmt ` 声明 40 字节、`num_channels = 33`、`channel_mask = 0`、PCM 子格式 GUID ⇒
///    `attempt to shift left with overflow`。判据与读数见
///    `decode::tests::a_wave_extensible_fmt_that_shift_overflows_is_refused_not_a_panic`。
///
/// 这道闸门是**只拒**的：它只在"上游确实会执行那次乘法/移位"时报错。上游能正常解析的文件
/// 一个都不会被它拒掉 —— **唯一**的例外是块数超过 [`RIFF_PRECHECK_MAX_CHUNKS`] 的文件
/// （那种文件的块结构已经不可信，宁可立刻报错也不放行；见 [`scan_riff_for_fmt`]）。
/// 扫描不到 `fmt ` 块时它什么都不做，把判定留给探测器。
///
/// ⚠ 2026-10-09 实测更正：改建前这里的扫描**算错过前进量**，于是本闸门在
/// "`fmt ` 前面有非零长度块"的布局上完全失效（放行 ⇒ 上游 panic）。反例与读数见
/// `decode::tests::a_wave_fmt_behind_other_chunks_is_refused_not_a_panic`。
fn precheck_riff_wave_fmt(source: &mut dyn MediaSource) -> DecodeResult<()> {
    if !source.is_seekable() {
        return Ok(());
    }
    // `MediaSource` 在这里是 trait 对象，而 `Seek::stream_position` 要求 `Self: Sized`
    // （clippy 的建议在此不可用），因此只能用 `SeekFrom::Current(0)` 取当前位置。
    #[allow(clippy::seek_from_current)]
    let Ok(start) = source.seek(SeekFrom::Current(0)) else {
        return Ok(());
    };
    let outcome = scan_riff_for_fmt(source);
    // 无论扫描结果如何，都把位置还给探测器 —— 后面的 `probe` 必须从头读。
    source.seek(SeekFrom::Start(start))?;
    outcome
}

/// [`precheck_riff_wave_fmt`] 的扫描主体。
///
/// 走法与上游 `symphonia-format-riff-0.6.1` 的 `ChunksReader::next` +
/// `WavReader::try_new` **逐条对齐**。对齐不是洁癖：本闸门的判据是"只拒上游真的会
/// panic 的那一份输入"，走法一旦与上游不同，两侧都会出错 ——
/// 走得**少**是漏洞（放行 ⇒ panic），走得**多**是误拒（上游根本不解析的字节被本闸门判死）。
///
/// | 上游行为 | 本函数的镜像 |
/// | :--- | :--- |
/// | `riff_len < 4` ⇒ `wav: invalid riff length`（报错，不 panic） | 直接 `Ok(())`，把报错留给上游 |
/// | `riff_len == u32::MAX`（流式 WAV）⇒ 父块长度未知 | `parent_len = None` |
/// | 否则父块上界 = `riff_len - 4`（`ChunksReader::new(Some(riff_len - 4), ..)`，`wave/mod.rs`） | 同值；越界即 `Ok(())` |
/// | 每个块头之前按 2 字节对齐（`consumed & 1`） | 同 |
/// | 未知块的块体用 `ignore_bytes(chunk_len)` 跳过 | `seek(size + (size & 1))` |
///
/// # Errors
///
/// 只在"上游那次 `u16` 乘法确实会溢出"、"上游那次 `u32` 移位确实会溢出"
/// （见 [`refuse_extensible_fmt_shift_overflow`]），或扫描预算用尽时返回
/// [`DecodeError::Malformed`]。
fn scan_riff_for_fmt(source: &mut dyn MediaSource) -> DecodeResult<()> {
    let mut header = [0u8; 12];
    if source.read_exact(&mut header).is_err() {
        return Ok(());
    }
    if header[0..4] != b"RIFF"[..] || header[8..12] != b"WAVE"[..] {
        return Ok(());
    }
    // 上游只走"RIFF 声明的那段块区"，所以本扫描也必须停在那里。
    let riff_len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if riff_len < 4 {
        return Ok(());
    }
    let parent_len = if riff_len == u32::MAX {
        None
    } else {
        Some(u64::from(riff_len) - 4)
    };

    // 父块内已消耗的字节数（含填充字节），口径与上游 `ChunksReader.consumed` 相同。
    let mut consumed: u64 = 0;
    for _ in 0..RIFF_PRECHECK_MAX_CHUNKS {
        // 上游的顺序：先判"到父块末尾了吗"，再对齐，再判"还够一个 8 字节块头吗"。
        if parent_len.is_some_and(|limit| consumed >= limit) {
            return Ok(());
        }
        if consumed & 1 == 1 {
            let mut pad = [0u8; 1];
            if source.read_exact(&mut pad).is_err() {
                return Ok(());
            }
            consumed += 1;
        }
        if parent_len.is_some_and(|limit| consumed + 8 > limit) {
            return Ok(());
        }
        let mut chunk = [0u8; 8];
        if source.read_exact(&mut chunk).is_err() {
            return Ok(());
        }
        consumed += 8;
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        if chunk[0..4] == b"fmt "[..] {
            // 上游不看 `fmt ` 的声明长度，一律先读 16 字节字段（`chunks.rs:415-424`），
            // 所以这里也读 16 字节，而不是相信 `size`。
            let mut body = [0u8; 16];
            if source.read_exact(&mut body).is_err() {
                return Ok(());
            }
            let format_tag = u16::from_le_bytes([body[0], body[1]]);
            let num_channels = u16::from_le_bytes([body[2], body[3]]);
            let bits_per_sample = u16::from_le_bytes([body[14], body[15]]);
            if format_tag == WAVE_FORMAT_PCM {
                // 只有 tag 1 + 上游接受的位深才会走到那次乘法；其余形状上游自己会报错。
                let bytes_per_sample = match bits_per_sample {
                    8 | 16 | 24 | 32 => u32::from(bits_per_sample / 8),
                    _ => return Ok(()),
                };
                if u32::from(num_channels) * bytes_per_sample > u32::from(u16::MAX) {
                    return Err(DecodeError::Malformed {
                        detail: format!(
                            "WAV fmt declares {num_channels} channels at {bits_per_sample} bits \
                             per sample: the RIFF parser computes \
                             num_channels * (bits_per_sample / 8) in u16 and would overflow \
                             (symphonia-format-riff src/wave/chunks.rs:100)"
                        ),
                    });
                }
            } else if format_tag == WAVE_FORMAT_EXTENSIBLE {
                // 第二处上游未检查算术（`fix_wave_channel_mask` 的 `u32` 移位）。tag 1 的
                // 那次 `u16` 乘法挡不住它，所以这里必须单独判。
                refuse_extensible_fmt_shift_overflow(source, size, num_channels, bits_per_sample)?;
            }
            return Ok(());
        }
        // 未知块。上游用 `ignore_bytes(chunk_len)` 跳过块体，**不会**再多吃一个块头：
        // 块头那 8 字节已经在上面读掉了，所以这里只前进"块体 + 奇数长度的填充字节"。
        //
        // ⚠ 2026-10-09 实测更正：改建前这里写的是 `8 + size + (size & 1)`，即每跳一个块
        // 就多走 8 字节，扫描从此脱离块网格。后果是只要 `fmt ` 前面有任何**非零长度**
        // 的块（真实 WAV 里的 `JUNK` / `LIST` / `bext` 很常见），或者有**奇数个** 0 长度
        // 块，扫描就再也找不到 `fmt `，于是放行给上游，而上游那次 `u16` 乘法
        // （现位于 `wave/chunks.rs` 第 100 行）把进程 panic 掉 —— 这道闸门存在的
        // 唯一理由正好被它自己的前进量抵消了。
        let skip = u64::from(size).saturating_add(u64::from(size & 1));
        consumed = consumed.saturating_add(skip);
        let advance = i64::try_from(skip).unwrap_or(i64::MAX);
        if source.seek(SeekFrom::Current(advance)).is_err() {
            return Ok(());
        }
    }
    // 扫描预算用尽：**保守拒绝**。此时的块结构已经不可信，放行等于把没看过的字节
    // 交给上游那个会 panic 的乘法（[ARCH-SEC-003]、不可信输入零 panic）。
    Err(DecodeError::Malformed {
        detail: format!(
            "RIFF/WAVE has more than {RIFF_PRECHECK_MAX_CHUNKS} chunks before its fmt chunk; \
             refusing to hand the file to the parser"
        ),
    })
}

/// 上游 `read_ext_fmt` → `fix_wave_channel_mask` 的**未检查移位**：只拒这一份输入。
///
/// 上游事实（逐行读源码得到；本 crate **不能**修改上游）：
/// `fix_wave_channel_mask`（现位于 `wave/chunks.rs` 第 682 行）先算
/// `channel_diff = num_channels as i32 - channel_mask.count_ones() as i32`，在
/// `channel_diff > 0` 时执行 `1u32 << channel_diff`（第 690 行）。`channel_diff >= 32`
/// 就是一次移位溢出 —— debug 构建与 `overflow-checks = true` 的 release 构建都会
/// panic，而**134 字节**的畸形文件就足以触发（实测读数见判据
/// `a_wave_extensible_fmt_that_shift_overflows_is_refused_not_a_panic`）。
///
/// 本函数**只**在"上游真的会走到那次移位"时说拒绝：它在移位之前的分支逐条镜像上游
/// （声明长度 < 40、`cbSize != 22`、位深不是 8 的倍数、GUID 是 Ambisonic 或未知、
/// 各子格式自己的位深约束）。任何一条不满足时上游都会先报错、不会 panic，此时本函数
/// 返回 `Ok(())`，把判定留给上游 —— 这与"只拒"的取舍一致：宁可让上游去报自己的错，
/// 也不用一条**不成立**的理由拒绝一个文件。
///
/// # Errors
///
/// 上游那次移位确实会溢出时返回 [`DecodeError::Malformed`]。
fn refuse_extensible_fmt_shift_overflow(
    source: &mut dyn MediaSource,
    size: u32,
    num_channels: u16,
    bits_per_sample: u16,
) -> DecodeResult<()> {
    // 上游 `read_ext_fmt` 的第一条：声明长度 < 40 直接 `decode_error`，走不到移位。
    if size < 40 {
        return Ok(());
    }
    // 40 字节块体里剩下的 24 字节，顺序与上游读取顺序一致：
    // `cbSize(2)` → `valid_bits_per_sample(2)` → `channel_mask(4)` → `sub_format_guid(16)`。
    let mut ext = [0u8; 24];
    if source.read_exact(&mut ext).is_err() {
        return Ok(());
    }
    // `cbSize` 必须是 22（`WaveFormatEx` 的 22 字节扩展），否则上游报错。
    if u16::from_le_bytes([ext[0], ext[1]]) != 22 {
        return Ok(());
    }
    let valid_bits = u16::from_le_bytes([ext[2], ext[3]]);
    // 上游在读到 `cbSize`/`valid_bits` 之后、读声道掩码之前就拒绝非 8 倍数位深。
    if !bits_per_sample.is_multiple_of(8) {
        return Ok(());
    }
    let channel_mask = u32::from_le_bytes([ext[4], ext[5], ext[6], ext[7]]);
    let sub_format = &ext[8..24];
    if !reaches_the_wave_channel_mask_fix(sub_format, bits_per_sample, valid_bits) {
        return Ok(());
    }
    if !would_channel_mask_fix_overflow(num_channels, channel_mask) {
        return Ok(());
    }
    Err(DecodeError::Malformed {
        detail: format!(
            "WAV fmt (WAVE_FORMAT_EXTENSIBLE) declares {num_channels} channels at \
             {bits_per_sample} bits per sample with channel mask {channel_mask:#010x}: the RIFF \
             parser's channel-mask fix-up would shift-overflow (symphonia-format-riff \
             wave/chunks.rs line 690)"
        ),
    })
}

/// 上游 `read_ext_fmt` 读完 40 字节块体之后，会不会走到 `fix_wave_channel_mask`。
///
/// 只有**声道掩码定位**的四个子格式会走到那里（`sub_format_guid` 为 PCM / IEEE_FLOAT /
/// ALAW / MULAW）。上游另外还认两个 Ambisonic B-format GUID，它们走
/// `map_amb_channel_count`（没有移位），其余 GUID 直接 `unsupported_error` —— 这两类对
/// 本闸门是**同一个**结论（`false`），因此这里只认那四个，不给 Ambisonic 单开分支：
/// 开了也观察不到差别（判据 `the_extensible_gate_refuses_only_the_shape_that_really_
/// shifts_over` 的 Ambisonic 用例与"未知 GUID"用例走的就是同一条返回路径）。
///
/// 每个子格式在移位之前还有自己的位深约束（不满足时上游报错），这里一并镜像：位深不合法
/// 的输入由上游报它自己的错，不由本条闸门冒充"移位会溢出"。
fn reaches_the_wave_channel_mask_fix(
    sub_format: &[u8],
    bits_per_sample: u16,
    valid_bits: u16,
) -> bool {
    if sub_format == WAVE_SUBTYPE_PCM {
        // 上游：位深 ∈ {8,16,24,32}（0 与 > 32 先报错），且 `valid <= bits`。
        matches!(bits_per_sample, 8 | 16 | 24 | 32) && valid_bits <= bits_per_sample
    } else if sub_format == WAVE_SUBTYPE_IEEE_FLOAT {
        // 上游：`valid == bits`，且位深 ∈ {32,64}。
        matches!(bits_per_sample, 32 | 64) && valid_bits == bits_per_sample
    } else if sub_format == WAVE_SUBTYPE_ALAW || sub_format == WAVE_SUBTYPE_MULAW {
        // 上游：a-law / mu-law 只接受 8 位。
        bits_per_sample == 8
    } else {
        // Ambisonic 与未知 GUID：上游不调用那次移位。把这两类当成"会走到移位"的代价是
        // **误拒**（用一个不成立的理由拒绝文件），因此判据里各有一条对照。
        false
    }
}

/// 上游 `fix_wave_channel_mask` 里那次移位会不会 panic。
///
/// 上游（现位于 `wave/chunks.rs` 第 690 行）算的是：
/// `channel_diff = num_channels - channel_mask.count_ones()`，只在 `channel_diff > 0` 时执行
/// `channel_mask |= ((1 << channel_diff) - 1) << shift`，移位量
/// `shift = 32 - (!channel_mask).leading_ones()` 是 mask 的**最高零位**。
///
/// 这次构造有**两个**独立的 panic 来源，缺一不可：
///
/// 1. **内移** `(1 << channel_diff) - 1`：移位量 `channel_diff >= 32` 时这次左移即溢出
///    （实测的 panic 文本是 `attempt to shift left with overflow`）。这正是改建前唯一的
///    条件 `num_channels >= popcount + 32`；
/// 2. **外移** `… << shift`：移位量 `shift >= 32` 时同样溢出。`leading_ones()` 恒 ≤ 32，
///    因此 `shift >= 32` 等价于 `(!channel_mask).leading_ones() == 0`，也就是
///    **mask 的第 31 位是 1**。
///
/// 改建前只判第一条，于是漏掉第二条的**全部**形状 —— 例如 `33` 声道 +
/// `channel_mask = 0xAAAA_AAAA`（`channel_diff = 17`、`shift = 32`）会直接 panic 在
/// 上游第 690 行。这一族不是边角料：`channel_mask = 0x8000_0000` 只有一个掩码位，
/// 只要声道数 > 1 就落入其中。
///
/// 等价性有一条**穷举交叉验证**（`/tmp` 探针，逐字复制上游函数并用 `catch_unwind`
/// 当基准）：在 2 184 组固定 `(channels, mask)` 与 300 000 组伪随机对上，这条条件
/// **漏判 0 组、误拒 0 组**；改建前的条件在同一批上漏判 279 + 63 组。
/// 判据 `a_wave_extensible_fmt_whose_channel_mask_shift_overflows_is_refused_not_a_panic`
/// 给出其中的具体读数。
///
/// 另外记一笔给下一位：**建模这条闸门时不要假设那次构造按统一的 u32 宽度求值**。
/// 实测（同一支探针）上游在这一侧对参数比 u32 更宽容 —— 只有内移的移位量
/// `channel_diff >= 32`、以及外移的移位量 `shift >= 32` 这两种情形真的 panic。
/// 按"u32 位跨度 `shift + channel_diff > 32`"去推会算出一个**不存在的** panic 集合，
/// 让闸门白白多拒 203 组：这 203 组的上游结果与逐位正确的 u32 结果完全一致
/// （探针逐组比对过），因此它们既不是畸形文件，也不是上游会拒绝的文件。
/// 上面两条是唯一判据。
fn would_channel_mask_fix_overflow(num_channels: u16, channel_mask: u32) -> bool {
    // 内移：`channel_diff >= 32` ⇒ 移位量非法。
    if u32::from(num_channels) >= channel_mask.count_ones() + 32 {
        return true;
    }
    // `channel_diff == 0` 时上游不进移位分支（`else` 那条 `while` 的循环条件一开始就为假）。
    if channel_mask.count_ones() >= u32::from(num_channels) {
        return false;
    }
    // 外移：`shift >= 32` ⇔ `(!mask).leading_ones() == 0` ⇔ mask 的第 31 位是 1。
    (!channel_mask).leading_ones() == 0
}

/// 不可回退源允许的**连续无进展读取**次数（[`std::io::ErrorKind::Interrupted`] 重试的上界）。
///
/// 单位：次。这个上界**不是**一个新的魔数，而是从调用方的输入字节预算
/// （[`crate::limits::PcmBudget::max_input_bytes`]）推导出来的：预算每容纳一个
/// [`SLURP_CHUNK_BYTES`] 读取分块就多允许一次无进展重试，另加**一次保底** ——
/// 一次合法的 `EINTR` 必须能被容忍，即使预算是 0（否则零预算的源会在被判定"输入过大"
/// 之前先报一个不成理由的 I/O 错误）。
///
/// 为什么要上界：`Interrupted` 的语义是"这次读**没有**交出任何字节"，重试它不消耗任何
/// 资源。于是一个**永远**报 `Interrupted` 的源会让 [`slurp_unseekable`] 的循环永不返回
/// （挂死），而本 crate 的契约是不可信输入只返回 [`DecodeError`]（`MUST-GATE-011`）。
///
/// 上界取"预算能容纳的分块数 + 1"而不是写死的数字，是为了让它随预算伸缩：默认预算
/// （约 8.3 GiB）允许约 126 500 次，预算 0 只允许 1 次。判据
/// `consecutive_interrupts_are_bounded_by_the_input_byte_budget` 逐档复算这条公式，
/// 并钉住"上界加一即返回源自己的 I/O 错误"。
fn unseekable_retry_allowance(max_input_bytes: u64) -> u64 {
    (max_input_bytes / SLURP_CHUNK_BYTES as u64).saturating_add(1)
}

/// 把一份**不可回退**的输入整份读进内存，并在读取过程中施加输入字节闸门。
///
/// 为什么必须边读边查：不可回退的源无法先量长度（[`MediaSource::byte_len`] 对管道可能是
/// `None`），"读完再查"等于"先把内存吃光再报错"。这里的判定与 [`limits::check_input_len`]
/// 同口径：**闭区间**（恰好等于 `max_input_bytes` 通过），超过一个字节即
/// [`LimitViolation::InputTooLarge`]（错误里的 `bytes` 是**投影值**，即"再读这一块会到多少"，
/// 而不是"读到多少才发现"）。
///
/// 读缓冲固定 [`SLURP_CHUNK_BYTES`]：与 [`crate::asset`] 的摘要缓冲同一个量级，与输入长度
/// 无关；增长用 `try_reserve`，因此"分配器说不"是错误而不是 abort。
///
/// **连续无进展的读取有上界**（`2026-10-10` 由裁决 `R33` 加上，见
/// [`unseekable_retry_allowance`]）：`Interrupted` 重试不消耗资源，因此一个永远报
/// `Interrupted` 的源会让本循环永不返回。上界由 `max_input_bytes` 推导，超限时报源自己的
/// I/O 错误。判据 `consecutive_interrupts_are_bounded_by_the_input_byte_budget` 钉住它。
///
/// # Errors
///
/// - 输入超过 `max_input_bytes` ⇒ [`LimitViolation::InputTooLarge`]；
/// - 分配器拒绝 ⇒ [`LimitViolation::AllocationRefused`]（该变体的计数**在这里的单位是
///   容器字节**，因为被增长的缓冲装的是容器字节，不是音频样本）；
/// - 连续无进展的读取超过 [`unseekable_retry_allowance`] ⇒ 源自身的 `Interrupted` 错误；
/// - 源自身的其他 I/O 失败原样上报。
fn slurp_unseekable(source: &mut dyn MediaSource, max_input_bytes: u64) -> DecodeResult<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = [0u8; SLURP_CHUNK_BYTES];
    // 连续无进展读取的次数。有进展就归零，因此它数的是**连续**重试 —— 一份长素材中间
    // 偶发几次 `EINTR` 不受影响。
    let mut retries: u64 = 0;
    let allowance = unseekable_retry_allowance(max_input_bytes);
    loop {
        match source.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(filled) => {
                let projected = u64::try_from(bytes.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(filled).unwrap_or(u64::MAX));
                if projected > max_input_bytes {
                    return Err(DecodeError::Budget(LimitViolation::InputTooLarge {
                        bytes: projected,
                        limit: max_input_bytes,
                    }));
                }
                bytes.try_reserve(filled).map_err(|_| {
                    DecodeError::Budget(LimitViolation::AllocationRefused {
                        samples: u64::try_from(filled).unwrap_or(u64::MAX),
                    })
                })?;
                bytes.extend_from_slice(&chunk[..filled]);
                retries = 0;
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                // 这一次读没有交出任何字节。重试本身不消耗资源，因此必须有上界，否则
                // 一个永远报 `Interrupted` 的源会让本循环**永不返回**（挂死）。
                //
                // 超限报源自己的那个 I/O 错误，而不是 `InputTooLarge`：没有任何字节被
                // 真的消耗，报一个字节数会是假话（真实字节的那道闸门在 `Ok` 分支里，
                // 口径与 [`limits::check_input_len`] 逐字相同，本处不改动它）。
                retries = retries.saturating_add(1);
                if retries > allowance {
                    return Err(err.into());
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
}

/// 记录"这一轮没有产出样本"，超过闸门即报错 [MUST-GATE-011]。
///
/// 单独抽出来是为了让**每一条**"没出样本"的路径都必须经过它 —— 新增一条 `continue`
/// 而忘了记账，在代码审查里会显眼得多。闸门本身的算术住在
/// [`limits::IdleGuard`]（零依赖，本机可单独跑）。
fn bump_idle(idle: &mut limits::IdleGuard) -> DecodeResult<()> {
    if idle.bump() {
        return Err(DecodeError::Malformed {
            detail: format!(
                "demuxer stopped making progress: {} consecutive packets produced no audio",
                limits::MAX_IDLE_PACKETS
            ),
        });
    }
    Ok(())
}

/// 把任意 `Read + Seek` 包装成 symphonia 能吃的 [`MediaSource`]。
///
/// 存在的理由：symphonia 只为 `std::fs::File` 与 `std::io::Cursor<T: AsRef<[u8]>>`
/// 提供实现。构造时量一次流长度（`byte_len`），因为容器的"总帧数"推算依赖它。
pub struct MeasuredSource<R> {
    inner: R,
    len: u64,
}

impl<R> MeasuredSource<R>
where
    R: Read + Seek + Send + Sync,
{
    /// 包一层并量出长度；构造后 reader 的位置与调用前相同。
    ///
    /// # Errors
    ///
    /// 底层 `seek` 失败时原样上报。
    pub fn new(mut inner: R) -> std::io::Result<Self> {
        let start = inner.stream_position()?;
        let end = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(start))?;
        Ok(Self { inner, len: end })
    }
}

impl<R> Read for MeasuredSource<R>
where
    R: Read + Seek + Send + Sync,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R> Seek for MeasuredSource<R>
where
    R: Read + Seek + Send + Sync,
{
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl<R> MediaSource for MeasuredSource<R>
where
    R: Read + Seek + Send + Sync,
{
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfix::{
        FlacSpec, WavExtensibleSpec, WavFormat, WavSpec, encode_f32_samples, encode_int_samples,
        flac_constant, flac_constant_with_broken_frame_crc, flac_constant_with_reserved_subframes,
        fmt_body_offset, wav, wav_extensible, wav_extensible_with_junk, wav_with_chunks_before_fmt,
        wav_with_declared_len,
    };

    fn int_spec(channels: u16, bits: u16) -> WavSpec {
        WavSpec {
            channels,
            sample_rate: 8_000,
            bits,
            format: WavFormat::Integer,
        }
    }

    fn int_wav(channels: u16, bits: u16, values: &[i32]) -> Vec<u8> {
        let spec = int_spec(channels, bits);
        wav(&spec, &encode_int_samples(bits, values))
    }

    /// 声明的样本值 → 期望的归一化 `f32`（容差按源位深的 4 个 LSB 给）。
    fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: got {actual}, expected {expected} (+-{tolerance})"
        );
    }

    #[test]
    fn wav_pcm16_mono_decodes_to_expected_samples() {
        let values = [0i32, 16_384, -16_384, 8_192, -8_192];
        let asset = decode_bytes(&int_wav(1, 16, &values), &DecodeOptions::default()).unwrap();
        assert_eq!(asset.channels(), 1);
        assert_eq!(asset.sample_rate(), 8_000);
        assert_eq!(asset.pcm_format(), PcmFormat::S16);
        assert_eq!(asset.frame_count(), 5);
        let tolerance = 4.0 / 32_768.0;
        for (index, raw) in values.iter().enumerate() {
            assert_close(
                asset.samples()[index],
                *raw as f32 / 32_768.0,
                tolerance,
                &format!("sample {index}"),
            );
        }
    }

    #[test]
    fn wav_pcm16_stereo_keeps_channel_interleaving() {
        // L/R 交替，通道内的顺序必须原样保留。
        let values = [1_000i32, -1_000, 2_000, -2_000, 3_000, -3_000];
        let asset = decode_bytes(&int_wav(2, 16, &values), &DecodeOptions::default()).unwrap();
        assert_eq!(asset.channels(), 2);
        assert_eq!(asset.frame_count(), 3);
        assert_eq!(asset.samples().len(), 6);
        for (index, raw) in values.iter().enumerate() {
            assert_eq!(
                asset.samples()[index].signum(),
                (*raw as f32).signum(),
                "sign flipped at interleaved index {index}"
            );
        }
        // 同相位的两个通道值必须等幅反号。
        assert_close(
            asset.samples()[0],
            -asset.samples()[1],
            1.0 / 32_768.0,
            "L/R mirror",
        );
    }

    #[test]
    fn wav_pcm8_unsigned_is_supported() {
        let values = [0i32, 64, -64, 127, -128];
        let asset = decode_bytes(&int_wav(1, 8, &values), &DecodeOptions::default()).unwrap();
        assert_eq!(asset.pcm_format(), PcmFormat::U8);
        assert_eq!(asset.frame_count(), 5);
        let tolerance = 4.0 / 128.0;
        for (index, raw) in values.iter().enumerate() {
            // 8-bit WAV 是无符号偏移 128 的约定 => 归一化后 0 对应 -1.0。
            assert_close(
                asset.samples()[index],
                *raw as f32 / 128.0,
                tolerance,
                &format!("u8 sample {index}"),
            );
        }
        assert_close(asset.samples()[4], -1.0, tolerance, "u8 min");
    }

    #[test]
    fn wav_pcm24_is_supported() {
        let values = [0i32, 4_194_304, -4_194_304];
        let asset = decode_bytes(&int_wav(1, 24, &values), &DecodeOptions::default()).unwrap();
        assert_eq!(asset.pcm_format(), PcmFormat::S24);
        assert_eq!(asset.frame_count(), 3);
        let tolerance = 4.0 / 8_388_608.0;
        assert_close(asset.samples()[1], 0.5, tolerance, "24-bit half scale");
        assert_close(
            asset.samples()[2],
            -0.5,
            tolerance,
            "24-bit negative half scale",
        );
    }

    #[test]
    fn wav_pcm32_is_supported() {
        let values = [0i32, 1_073_741_824, -1_073_741_824];
        let asset = decode_bytes(&int_wav(1, 32, &values), &DecodeOptions::default()).unwrap();
        assert_eq!(asset.pcm_format(), PcmFormat::S32);
        // 32-bit 整数在 yeban-model 的 BitDepth 里没有对应变体（只有 Int16/Int24/Float32）。
        assert_eq!(asset.model_bit_depth(), None);
        let tolerance = 4.0 / 2_147_483_648.0;
        assert_close(asset.samples()[1], 0.5, tolerance, "32-bit half scale");
    }

    #[test]
    fn wav_float32_is_supported() {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 32,
            format: WavFormat::Float,
        };
        let values = [0.0f32, 0.25, -0.5, 0.75];
        let bytes = wav(&spec, &encode_f32_samples(&values));
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(asset.pcm_format(), PcmFormat::F32);
        assert_eq!(
            asset.model_bit_depth(),
            Some(yeban_model::BitDepth::Float32)
        );
        assert_eq!(asset.samples(), values.as_slice());
    }

    /// 判据（类别①：非有限输入）：浮点容器里的非有限样本必须**拒绝**，不许解成资产。
    ///
    /// 为什么这条闸门的存在理由是输入字节而不是"调用方给错参数"：IEEE float WAV 的
    /// `data` 块体是**任意字节**，`0x7fc00000` 就是一个合法的 f32 载荷。本 crate 的契约是
    /// "不可信输入要么给出自洽的资产、要么给出类型化错误"，而一份含 NaN 的资产既不自洽
    /// （下游没有任何一处能把它变回有限值）也没有别的出口 ⇒ 在这里拒绝是唯一不撒谎的
    /// 处置。整型容器不受影响（`PcmFormat::is_float()` 之外零成本）。
    ///
    /// 判据同时钉住三件事，缺一条都不能算过：
    /// 1. 三个形状（`NaN` / `+∞` / `-∞`）在 F32 与 F64 上都被拒，且是
    ///    [`DecodeError::Malformed`]（不是 panic、不是 Ok）；
    /// 2. **非空洞**：同一个夹具把那个槽位换成有限值就 `Ok`，且逐位相同 —— 证明这条闸门
    ///    拒的是"非有限"而不是"浮点 WAV"或"这个夹具"；
    /// 3. **只拒非有限**：次正规数（`f32::from_bits(1)`，最小的正 f32）与 `-0.0` 都是
    ///    有限值，必须照常解出 —— 否则这条闸门就成了"拒绝一切不寻常的位型"。
    ///
    /// 注入：把 `decode_source` 循环里那一整块
    /// `if buffer_format.is_float() && let Some(offset) = …` 删掉 ⇒ 本判据的第一组断言全红
    /// （实测：`F32 NaN must be refused: DecodedAsset { … samples: [NaN, …] }`），并且整库
    /// **只有本判据**红（lib 116 → 115 passed / 1 failed），因此那个站点此前没有任何判据
    /// 钉着。整型一侧不需要单独的对照注入：`is_float()` 为假时 `position` 那一半根本不会
    /// 被求值，因此"整数样本不可能非有限"是类型事实，不是运行期分支。
    #[test]
    fn a_float_container_with_non_finite_samples_is_refused() {
        let f32_spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 32,
            format: WavFormat::Float,
        };
        let f64_spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 64,
            format: WavFormat::Float,
        };
        // 位置固定在第 0 个样本（帧 0、声道 0），其余样本有限：因此文案里的
        // "frame 0 channel 0" 是可以被断言的具体值，闸门也必须能在第 0 个样本上触发。
        let poisoned = [
            ("NaN", f32::NAN, f64::NAN),
            ("+inf", f32::INFINITY, f64::INFINITY),
            ("-inf", f32::NEG_INFINITY, f64::NEG_INFINITY),
        ];
        for (name, bad32, bad64) in poisoned {
            let values32 = [bad32, -0.25, 0.5, -0.5];
            let bytes = wav(&f32_spec, &encode_f32_samples(&values32));
            let err = decode_bytes(&bytes, &DecodeOptions::default())
                .expect_err(&format!("F32 {name} must be refused"));
            let detail = match &err {
                DecodeError::Malformed { detail } => detail.clone(),
                other => panic!("F32 {name} must be Malformed, got {other:?}"),
            };
            assert!(
                detail.contains("frame 0 channel 0"),
                "F32 {name}: detail must name the offending position, got {detail}"
            );

            let mut data64 = Vec::new();
            for v in [bad64, -0.25, 0.5, -0.5] {
                data64.extend_from_slice(&v.to_le_bytes());
            }
            let bytes64 = wav(&f64_spec, &data64);
            assert!(
                matches!(
                    decode_bytes(&bytes64, &DecodeOptions::default()),
                    Err(DecodeError::Malformed { .. })
                ),
                "F64 {name} must be refused as Malformed"
            );

            // 非空洞对照：同一个夹具、同一个槽位，换成有限值就 Ok，且逐位相同。
            let good32 = [name.len() as f32 * 0.25, -0.25, 0.5, -0.5];
            let good_bytes = wav(&f32_spec, &encode_f32_samples(&good32));
            let asset = decode_bytes(&good_bytes, &DecodeOptions::default())
                .expect("the finite control must decode");
            assert_eq!(asset.samples(), good32.as_slice());
        }

        // 只拒非有限：次正规数与 -0.0 都是有限值，照常解出且逐位保留。
        let odd_but_finite = [f32::from_bits(1), -0.0, f32::MIN_POSITIVE, f32::MAX];
        let bytes = wav(&f32_spec, &encode_f32_samples(&odd_but_finite));
        let asset = decode_bytes(&bytes, &DecodeOptions::default())
            .expect("subnormal / -0.0 / MAX are finite and must decode");
        assert_eq!(
            asset
                .samples()
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>(),
            odd_but_finite
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>()
        );
    }

    /// 判据（类别① 非有限输入）：非有限样本出现在**第一个解码缓冲之后**时同样必须被拒，
    /// 而且错误文案必须点名它**真实**所在的帧与声道。
    ///
    /// 为什么需要它：`a_float_container_with_non_finite_samples_is_refused` 的夹具只有
    /// 4 个样本，在解码循环里只对应**一个**缓冲 —— 那个站点上"每个缓冲都扫一遍"与
    /// "只扫第一个缓冲"给出同一个读数。本判据的夹具是 3 000 帧的 48 kHz 浮点立体声：
    /// 上游 RIFF 读端按每包 1152 帧切分，因此污染点落在**后面的**缓冲里；把扫描限制在
    /// 第一个缓冲（或只扫已经写入的前缀）就会把它整份放行。
    ///
    /// 同一条判据还钉住文案里的帧/声道分解：污染点取最后一个交错下标（帧 2 999、声道 1），
    /// 而"交错下标"与"帧/声道"只有在声道数为 1 时才相同。
    #[test]
    fn a_non_finite_float_sample_past_the_first_packet_is_refused_and_named() {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 32,
            format: WavFormat::Float,
        };
        let frames = 3_000usize;
        let poisoned_frame = frames - 1;
        let poisoned_channel = 1usize;
        let offset = poisoned_frame * 2 + poisoned_channel;
        let mut values = vec![0.25f32; frames * 2];
        values[offset] = f32::NAN;

        let err = decode_bytes(
            &wav(&spec, &encode_f32_samples(&values)),
            &DecodeOptions::default(),
        )
        .expect_err("a NaN past the first packet must still be refused");
        let detail = match &err {
            DecodeError::Malformed { detail } => detail.clone(),
            other => panic!("expected Malformed, got {other:?}"),
        };
        assert!(
            detail.contains("frame 2999 channel 1"),
            "the refusal must name the sample's real frame and channel, got {detail}"
        );

        // 非空洞对照：同一份夹具只把那一个槽位换成有限值 ⇒ 正常解出，且长度逐帧对齐。
        // 这证明上面红的是"非有限"，而不是这份夹具或这个长度本身不可解。
        let mut finite = values.clone();
        finite[offset] = -0.25;
        let asset = decode_bytes(
            &wav(&spec, &encode_f32_samples(&finite)),
            &DecodeOptions::default(),
        )
        .expect("the same fixture with a finite sample must decode");
        assert_eq!(asset.frame_count(), u64::try_from(frames).unwrap());
        assert_eq!(asset.channels(), 2);
    }

    #[test]
    fn flac_constant_block_decodes_with_its_declared_length() {
        let spec = FlacSpec {
            sample_rate: 8_000,
            channels: 1,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        };
        let bytes = flac_constant(&spec, 2, 0);
        let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(asset.channels(), 1);
        assert_eq!(asset.sample_rate(), 8_000);
        assert_eq!(asset.frame_count(), 512);
        assert_eq!(asset.declared_frames(), Some(512));
        assert!(asset.duration_is_reconciled());
        assert!(asset.samples().iter().all(|s| *s == 0.0));
    }

    #[test]
    fn flac_declared_total_samples_mismatch_is_reported_and_rejected() {
        let spec = FlacSpec {
            total_samples_override: Some(511),
            ..FlacSpec::default()
        };
        let bytes = flac_constant(&spec, 2, 0);

        // 宽松档：把双方数字原样交出来，供调用方判断。
        let loose = decode_bytes(
            &bytes,
            &DecodeOptions {
                verify_declared_duration: false,
                ..DecodeOptions::default()
            },
        )
        .unwrap();
        assert_eq!(loose.declared_frames(), Some(511));
        assert_eq!(loose.frame_count(), 512);
        assert!(!loose.duration_is_reconciled());

        // 默认档：明确报错，绝不"取较小者"。
        let err = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(err, DecodeError::DurationMismatch(_)),
            "expected DurationMismatch, got {err}"
        );
        assert!(err.to_string().contains("511"));
    }

    #[test]
    fn truncated_wav_is_an_error_not_a_panic() {
        let bytes = int_wav(1, 16, &[1_000, 2_000, 3_000, 4_000]);
        for cut in [12usize, 20, 30, 40, 44, 45, 47] {
            let truncated = &bytes[..cut.min(bytes.len())];
            let outcome = decode_bytes(truncated, &DecodeOptions::default());
            assert!(
                outcome.is_err(),
                "a {cut}-byte prefix must not decode as a complete asset"
            );
        }
    }

    #[test]
    fn arbitrary_garbage_is_an_error_not_a_panic() {
        let cases: [&[u8]; 5] = [
            b"",
            b"not audio at all, but long enough to be probed as something",
            b"RIFF\x00\x00\x00\x00WAVEjunk",
            &[0xFFu8; 512],
            b"fLaC\x80\x00\x00\x22",
        ];
        for case in cases {
            let outcome = decode_bytes(case, &DecodeOptions::default());
            assert!(
                outcome.is_err(),
                "garbage must be rejected, got {outcome:?}"
            );
        }
    }

    /// 判据 (不可信输入零 panic)：把采样率声明成 **0** 的流必须返回 [`DecodeError`]，
    /// 绝不允许 `decode_source` 的时长闸门在错误文案里做一次 `x / 0`。
    ///
    /// 可达性（实测形状：`wav_with_declared_len(spec, data, u32::MAX)`，48 字节量级）：
    /// - `fmt ` 块把 `sampleRate` 写成 0。上游 `WaveFormatChunk::parse` 只是把那个
    ///   `u32` 原样读进 `AudioCodecParameters`，完全不做非零校验（现位于上游
    ///   `wave/chunks.rs` 第 421 行）；`PcmDecoder::try_new` 也只要求"有值"而不要求非零，
    ///   因此 `sample_rate == 0` 会一路走到本函数；
    /// - `data` 块声明 `0xFFFF_FFFF`。上游把这一取值定义为"长度未知"（`DataChunk::parse`
    ///   做 `Some(len).filter(|&len| len != u32::MAX)`），于是 `append_data_params` 整条不跑
    ///   ⇒ `track.num_frames == None`，`data_end_pos` 也是"未知"。
    ///
    /// 两者合起来正好是缺口的入口：循环之前那次 `check_layout` 挂在
    /// `if let Some(frames) = declared_frames` 下面，`None` 就不跑；而循环里的时长闸门是
    /// `projected_frames > max_duration_frames` —— [`PcmBudget::max_duration_frames`] 在
    /// 0 Hz 下恒为 0，所以**第一个**非空包就进错误分支，那里要算
    /// `projected_frames / sample_rate`。
    ///
    /// 缺陷读数（修复前）：`attempt to divide by zero`（panic，进程被吃）。
    /// 修复后：`Err(Budget(ZeroSampleRate))`，文案点明 "zero sample rate"。
    ///
    /// 注入：把循环之前那次无条件 `check_layout` 改回 `if let Some(frames) = declared_frames`
    /// ⇒ 本判据以 `attempt to divide by zero` 红。
    #[test]
    fn a_zero_sample_rate_stream_is_refused_not_a_panic() {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 0,
            bits: 16,
            format: WavFormat::Integer,
        };
        // `u32::MAX` 的 `data` 声明长度是上游的"流式/未知"约定 ⇒ `track.num_frames == None`。
        let bytes =
            wav_with_declared_len(&spec, &encode_int_samples(16, &[1_000, 2_000]), u32::MAX);
        let err = decode_bytes(&bytes, &DecodeOptions::default())
            .expect_err("a 0 Hz declaration must be refused, not decoded");
        assert!(
            matches!(err, DecodeError::Budget(LimitViolation::ZeroSampleRate)),
            "expected ZeroSampleRate, got {err}"
        );
        assert!(
            err.to_string().contains("zero sample rate"),
            "the refusal must name the cause, got {err}"
        );
        // 同一个夹具只把采样率改成 8 kHz（其余字节不动）：它**不**返回 ZeroSampleRate，
        // 而是以 I/O 错误退出（`data` 声明"未知长度"⇒ 上游解封装一直读到文件尾，而夹具
        // 只有 2 帧真实样本）。这条对照证明上面红的是采样率闸门，而不是"这个形状的输入
        // 在 RIFF 预检就被整类拒掉了"。
        let legal = WavSpec {
            sample_rate: 8_000,
            ..spec
        };
        let ok_bytes =
            wav_with_declared_len(&legal, &encode_int_samples(16, &[1_000, 2_000]), u32::MAX);
        let outcome = decode_bytes(&ok_bytes, &DecodeOptions::default());
        assert!(
            matches!(outcome, Err(DecodeError::Io(_))),
            "an 8 kHz stream with an unknown data length must pass the precheck and the \
             rate gate, failing only on the truncated stream; got {outcome:?}"
        );
    }

    #[test]
    fn the_pcm_budget_is_enforced_before_allocating() {
        let bytes = int_wav(2, 16, &[1_000; 512]);
        // 只有 128 字节 PCM 预算 => 32 个样本 => 16 帧，输入是 256 帧。
        let strict = DecodeOptions {
            budget: PcmBudget {
                max_pcm_bytes: 128,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let err = decode_bytes(&bytes, &strict).unwrap_err();
        assert!(
            matches!(err, DecodeError::Budget(_)),
            "expected a budget refusal, got {err}"
        );
        // 宽松预算下同一份字节必须正常解出 —— 证明上面红的是预算而不是格式。
        assert_eq!(
            decode_bytes(&bytes, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            256
        );
    }

    /// 判据 ([ARCH-SEC-003] 闸门站点)：**声道数闸门必须在第一个解码缓冲处判** ——
    /// 也就是在任何样本缓冲增长之前。声道数超预算时，报出来的必须是
    /// `TooManyChannels`，即使同一份预算的 PCM 字节上限也小到会跳闸。
    ///
    /// 为什么需要它：`the_channel_and_rate_gates_fire_on_their_own` 只把
    /// `max_channels` 压到 1，PCM 字节预算保持默认（不可能跳闸），因此它分不开
    /// "在第一个缓冲处判"与"在整段解完之后的终检处判"—— 两条路径报的是同一个变体、
    /// 同一组字段，而后者会让整段素材先分配出来。本判据把字节上限一起压到刚好够
    /// 单声道版的一帧数：若声道闸门迟到，先跳闸的就是 `PcmBudgetExceeded`（变体不同），
    /// 于是"迟到"第一次变成可观察的。第二段是反向对照：把声道预算放宽、只留字节闸门 ⇒
    /// 那时必须报 `PcmBudgetExceeded`（证明这条判据不是"永远报声道错"）。
    #[test]
    fn the_channel_gate_fires_on_the_first_buffer_before_the_sample_budget() {
        // 256 帧立体声 ⇒ 单声道折算 256 个样本；字节上限给到 256 个样本，
        // 因此循环之前那次按 `channels = 1` 做的预检放行。
        let stereo = int_wav(2, 16, &[1_000; 512]);
        let both_tight = DecodeOptions {
            budget: PcmBudget {
                max_channels: 1,
                max_pcm_bytes: 256 * 4,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert!(
            matches!(
                decode_bytes(&stereo, &both_tight),
                Err(DecodeError::Budget(LimitViolation::TooManyChannels {
                    channels: 2,
                    limit: 1
                }))
            ),
            "the channel gate must be judged on the first buffer, before the sample budget"
        );
        let bytes_only = DecodeOptions {
            budget: PcmBudget {
                max_channels: 2,
                max_pcm_bytes: 256 * 4,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert!(
            matches!(
                decode_bytes(&stereo, &bytes_only),
                Err(DecodeError::Budget(
                    LimitViolation::PcmBudgetExceeded { .. }
                ))
            ),
            "with the channel gate relaxed the sample budget must be the one that trips"
        );
    }

    #[test]
    fn the_input_byte_budget_is_enforced_before_probing() {
        let bytes = int_wav(1, 16, &[7; 8]);
        let strict = DecodeOptions {
            budget: PcmBudget {
                max_input_bytes: 16,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert!(matches!(
            decode_bytes(&bytes, &strict),
            Err(DecodeError::Budget(LimitViolation::InputTooLarge { .. }))
        ));
    }

    #[test]
    fn a_wave_fmt_that_overflows_the_parser_is_rejected_not_a_panic() {
        // 实测缺陷（由 `propcheck::untrusted_bytes_never_panic_and_ok_results_are_self_consistent`
        // 找到）：上游 `symphonia-format-riff-0.6.1/src/wave/chunks.rs:100` 把
        // `num_channels * (bits_per_sample / 8)` 放在 `u16` 里算。`num_channels = 32769`
        // 与 16-bit 样本相乘溢出 ⇒ 整进程 panic（根 `Cargo.toml` 的
        // `[profile.release] overflow-checks = true` 让它在 release 也一样）。
        let spec = int_spec(1, 16);
        let mut bytes = wav(&spec, &encode_int_samples(16, &[0x1234, -0x1234]));
        // 布局：`fmt ` 在偏移 12，`num_channels` 在块体 +2 ⇒ 文件偏移 22。
        bytes[22..24].copy_from_slice(&32_769u16.to_le_bytes());
        let err = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("32769")),
            "expected a typed refusal naming 32769 channels, got {err}"
        );
        // 同一份字节只把声道数改回 1 ⇒ 正常解出。上面红的必须是那道闸门，不是夹具坏了。
        bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_bytes(&bytes, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            2
        );
    }

    /// 把 `fmt ` 块体里的 `num_channels` 改成 `channels`（畸形声明不能在夹具构造期写，
    /// 否则 [`WavSpec::block_align`] 自己会先溢出）。
    fn patch_declared_channels(bytes: &mut [u8], channels: u16) {
        let body = fmt_body_offset(bytes).expect("fixture must contain a fmt chunk");
        bytes[body + 2..body + 4].copy_from_slice(&channels.to_le_bytes());
    }

    /// 判据 ([ARCH-SEC-003] / 不可信输入零 panic)：`fmt ` **前面还有别的块**时，
    /// 上游溢出闸门必须照样在探测之前拦下它。
    ///
    /// 为什么需要它：`precheck_riff_wave_fmt` 是一台**块网格**走查器。改建前它把块头的
    /// 8 字节重复计进 `seek` 的前进量（`8 + size + pad`，而游标本来就在块头之后），
    /// 于是每跳一个块就多走 8 字节、脱离块网格。后果：只要 `fmt ` 前面有**非零长度**的
    /// 块（真实 WAV 里的 `JUNK` / `LIST` / `bext` 很常见），或有**奇数个** 0 长度块，
    /// 扫描就找不到 `fmt ` 而放行 ⇒ 上游 `symphonia-format-riff` 的
    /// `num_channels * (bits_per_sample / 8)` 在 `u16` 里溢出、**进程 panic**。
    /// 既有判据 `a_wave_fmt_that_overflows_the_parser_is_rejected_not_a_panic` 只覆盖
    /// "`fmt ` 是第一个块"这一种布局，因此对此完全免疫。
    ///
    /// 本判据对每种布局都要求**类型化拒绝**（点名 32769 声道）；panic 会让本判据红。
    /// 最后两个用例是"闸门不得误拒"那一侧：同一批布局 + 合法 `fmt ` 必须照常解出。
    #[test]
    fn a_wave_fmt_behind_other_chunks_is_refused_not_a_panic() {
        let spec = int_spec(1, 16);
        let data = encode_int_samples(16, &[0x1234, -0x1234]);
        let zero: &[u8] = &[];
        let four = [0xEEu8; 4];
        let twelve = [0xEEu8; 12];
        let three = [0xEEu8; 3];
        let layouts: [&[([u8; 4], &[u8])]; 8] = [
            &[],
            &[(*b"JUNK", zero)],
            &[(*b"JUNK", zero), (*b"JUNK", zero)],
            &[(*b"JUNK", &four)],
            &[(*b"LIST", &twelve)],
            &[(*b"JUNK", &four), (*b"JUNK", &four), (*b"JUNK", &four)],
            &[(*b"bext", &three)],
            &[(*b"JUNK", &four), (*b"JUNK", zero), (*b"LIST", &twelve)],
        ];
        for junk in layouts {
            let mut bytes = wav_with_chunks_before_fmt(&spec, junk, &data);
            patch_declared_channels(&mut bytes, 32_769);
            let err = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_err();
            assert!(
                matches!(&err, DecodeError::Malformed { detail } if detail.contains("32769")),
                "junk layout {junk:?} must be refused by the precheck, got {err}"
            );

            // 反向对照：同一布局、合法声道数 ⇒ 正常解出（闸门只拒，不误伤）。
            let mut legal = wav_with_chunks_before_fmt(&spec, junk, &data);
            patch_declared_channels(&mut legal, 1);
            assert_eq!(
                decode_bytes(&legal, &DecodeOptions::default())
                    .unwrap()
                    .frame_count(),
                2,
                "junk layout {junk:?} is a legal WAV and must still decode"
            );
        }

        // `riff_len == u32::MAX`：上游把长度当"未知"（流式 WAV），父块**无**上界。
        // 本闸门必须同样不设上界，否则这种布局会漏过去。
        let mut streaming = wav_with_chunks_before_fmt(&spec, &[(*b"JUNK", &four)], &data);
        streaming[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        patch_declared_channels(&mut streaming, 32_769);
        let err = decode_bytes(&streaming, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("32769")),
            "a streaming WAV (riff_len = u32::MAX) must still be refused, got {err}"
        );

        // `riff_len < 4`：上游自己会以 `wav: invalid riff length` 报错（不 panic），
        // 因此这里只钉"有界返回"，不钉具体是谁报的错。
        let mut short_riff = wav_with_chunks_before_fmt(&spec, &[(*b"JUNK", &four)], &data);
        short_riff[4..8].copy_from_slice(&0u32.to_le_bytes());
        patch_declared_channels(&mut short_riff, 32_769);
        assert!(
            decode_bytes(&short_riff, &DecodeOptions::default()).is_err(),
            "a RIFF length below 4 must not decode"
        );
    }

    /// 判据 ([ARCH-SEC-003] / 块网格末端边界)：当 `fmt ` 的 **8 字节块头恰好落在父块
    /// （RIFF 声明的块区）的最后 8 字节**时，预检仍然必须读它的 16 字节块体并判定那次
    /// `u16` 乘法。
    ///
    /// 为什么需要它：`scan_riff_for_fmt` 的"还够一个块头吗"这一步镜像的是上游
    /// `ChunksReader::next` 的条件 `consumed + 8 > len`（**严格大于**）。把它放宽成
    /// `>=` 会在这一格提前返回 `Ok(())`（放行）；而上游在那一格不会被"块长超出父块"
    /// 挡住 —— 声明长度为 0 时 `len - consumed == 0 < 0` 为假，于是 `fmt ` 照样被解析、
    /// 那次乘法照样溢出。既有的块网格判据用的都是"`fmt ` 后面还有块"的布局，
    /// 因此这一格此前是空白。
    ///
    /// 夹具是手工拼的：`N` 个 0 长度 `JUNK` 块（每个恰好 8 字节）加一个 `fmt ` 块头
    /// 构成整个父块区，`riff_len` 只声明到 `fmt ` 头的末尾（比真实文件短 24 字节），
    /// 16 字节块体物理上落在父块区**之外**。
    #[test]
    fn the_riff_precheck_reads_an_fmt_header_that_ends_the_parent_chunk() {
        let junk_chunks = 4usize;
        let total = 12 + junk_chunks * 8 + 8 + 16;
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::try_from(total - 24).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        for _ in 0..junk_chunks {
            bytes.extend_from_slice(b"JUNK");
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 声明的块体长度是 0
        // 16 字节块体：tag 1、32769 声道、16-bit ⇒ 上游那次 `u16` 乘法溢出。
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32_769u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        assert_eq!(bytes.len(), total, "fixture shape");

        let err = scan_precheck(&bytes).expect_err(
            "an fmt block header at the parent chunk's last 8 bytes must still be judged",
        );
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("32769")),
            "got {err}"
        );
    }

    /// 判据 ([ARCH-SEC-003] 的"只拒 WAV"一侧)：`RIFF` 容器但 form type 不是 `WAVE`
    /// 时，预检必须**完全不参与**。上游的 WAV 读端在 form type 那一层就报错，永远走不到
    /// `fmt ` 的解析，因此本闸门不能用一个不成立的理由（"乘法会溢出"）去拒它 ——
    /// 本 crate 的错误文案要进 MCP 响应体，原因必须指对。
    #[test]
    fn the_riff_precheck_stands_down_for_a_riff_container_that_is_not_wave() {
        let spec = int_spec(1, 16);
        let mut bytes = wav_with_chunks_before_fmt(
            &spec,
            &[(*b"JUNK", &[0xEEu8; 4])],
            &encode_int_samples(16, &[0x1234, -0x1234]),
        );
        patch_declared_channels(&mut bytes, 32_769);
        // 同一份字节只把 form type 换成 AVI：它不再是 WAV。
        bytes[8..12].copy_from_slice(b"AVI ");
        assert!(
            scan_precheck(&bytes).is_ok(),
            "a RIFF container that is not WAVE never reaches the WAV fmt parser"
        );
        // 对照：form type 改回 WAVE，同一份字节必须被拒 —— 证明上面绿的是 form type
        // 这一层，而不是这份夹具恰好不会触发那次溢出。
        bytes[8..12].copy_from_slice(b"WAVE");
        assert!(
            scan_precheck(&bytes).is_err(),
            "the same bytes as WAVE must still be refused"
        );
    }

    /// 判据 ([ARCH-SEC-003])：扫描预算用尽时必须**保守拒绝**，不得放行给上游。
    ///
    /// 为什么需要它：块网格修好之后，每个块只前进一个块的距离，于是"预算是多少"真正
    /// 决定了"最多能看穿多少个块"。改建前预算是 `4096`，但走查器每轮跳 2~3 个块，
    /// 到达范围与常量对不上；修好之后 `4096` 个块之后的 `fmt ` 会落在预算之外 ——
    /// 若此时返回"放行"，同一个 panic 就会从另一个门回来。
    ///
    /// 边界是闭区间：`RIFF_PRECHECK_MAX_CHUNKS - 1` 个前导块仍然看得见 `fmt `，
    /// 恰好 `RIFF_PRECHECK_MAX_CHUNKS` 个就用尽预算、保守拒绝。
    #[test]
    fn a_wave_with_more_chunks_than_the_scan_budget_is_refused_not_probed() {
        let spec = int_spec(1, 16);
        let data = encode_int_samples(16, &[0x1234, -0x1234]);
        let zero: &[u8] = &[];
        let cap = RIFF_PRECHECK_MAX_CHUNKS as usize;

        // 恰好用尽预算 ⇒ 保守拒绝（这里用的是**合法** fmt：拒绝的理由是块结构，
        // 不是那个溢出声明 —— 这正是"宁可报错也不放行"的诚实代价）。
        let at_cap = vec![(*b"JUNK", zero); cap];
        let bytes = wav_with_chunks_before_fmt(&spec, &at_cap, &data);
        let err = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("before its fmt chunk")),
            "expected the scan budget to fail closed, got {err}"
        );

        // 预算减一 ⇒ 仍然看得见 `fmt `，照常解出（证明上面红的是预算，不是夹具坏了）。
        let under_cap = vec![(*b"JUNK", zero); cap - 1];
        let bytes = wav_with_chunks_before_fmt(&spec, &under_cap, &data);
        assert_eq!(
            decode_bytes(&bytes, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            2
        );
    }

    /// `WAVE_FORMAT_EXTENSIBLE` 的夹具参数（`sample_rate` 固定 8 kHz：它不参与本判据）。
    fn ext_spec(
        channels: u16,
        bits: u16,
        valid_bits: u16,
        channel_mask: u32,
        sub_format: [u8; 16],
    ) -> WavExtensibleSpec {
        WavExtensibleSpec {
            channels,
            sample_rate: 8_000,
            bits,
            valid_bits,
            channel_mask,
            sub_format,
        }
    }

    /// 跑一遍 RIFF 预检本身（不经过探测器），用来钉"这条闸门**拒什么、不拒什么**"。
    fn scan_precheck(bytes: &[u8]) -> DecodeResult<()> {
        let mut source = Cursor::new(bytes);
        scan_riff_for_fmt(&mut source)
    }

    /// 判据 ([ARCH-SEC-003] / 不可信输入零 panic)：`WAVE_FORMAT_EXTENSIBLE` 的
    /// **声道掩码修正**是上游第二处未检查算术，必须同样在探测之前拦下。
    ///
    /// 上游事实：`fix_wave_channel_mask`（现位于 `wave/chunks.rs` 第 682 行）算
    /// `channel_diff = num_channels - channel_mask.count_ones()`，在 `channel_diff > 0` 时
    /// 执行 `1u32 << channel_diff`（第 690 行）。`channel_diff >= 32` ⇒ 移位溢出 ⇒ debug
    /// 与 `overflow-checks = true` 的 release 都 panic。实测（2026-10-09，把本条闸门去掉
    /// 之后）：**134 字节**的 40 字节 `fmt `（`num_channels = 33`、`channel_mask = 0`、
    /// PCM 子格式 GUID）让 `decode_bytes` 直接 panic 在 `wave/chunks.rs` 第 690 行，
    /// 消息是 `attempt to shift left with overflow`。
    ///
    /// 四个"掩码定位"GUID 都要覆盖：`sub_format_guid` 的字节是这条闸门的开关，认错一个
    /// 就是一条漏判（放行 ⇒ panic）。闭区间在这一侧：`channel_diff = 31`（31 声道 + 空掩码）
    /// 仍然合法（上游随后以 `UnsupportedFormat` 拒绝），`channel_diff = 32` 才溢出。
    #[test]
    fn a_wave_extensible_fmt_that_shift_overflows_is_refused_not_a_panic() {
        let data = encode_int_samples(8, &[0x11, 0x22]);
        // (标签, GUID, 位深, valid bits)：四种掩码定位子格式各一条，位深都取该子格式
        // 合法的最小值，因此上游一定会走到那次移位。
        let sub_formats: [(&str, [u8; 16], u16, u16); 4] = [
            ("pcm", WAVE_SUBTYPE_PCM, 8, 8),
            ("ieee", WAVE_SUBTYPE_IEEE_FLOAT, 32, 32),
            ("alaw", WAVE_SUBTYPE_ALAW, 8, 8),
            ("mulaw", WAVE_SUBTYPE_MULAW, 8, 8),
        ];
        for (label, guid, bits, valid_bits) in sub_formats {
            for channels in [32u16, 33, 1_000, u16::MAX] {
                let bytes = wav_extensible(&ext_spec(channels, bits, valid_bits, 0, guid), &data);
                let err = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_err();
                assert!(
                    matches!(
                        &err,
                        DecodeError::Malformed { detail }
                            if detail.contains("shift-overflow")
                                && detail.contains(&channels.to_string())
                    ),
                    "{label} with {channels} channels and an empty mask must be refused by the \
                     precheck, got {err}"
                );
            }
        }

        // 掩码只置 1 位也一样：`channel_diff = channels - 1`，因此 33 声道仍 ≥ 32。
        let one_bit = wav_extensible(&ext_spec(33, 8, 8, 0x1, WAVE_SUBTYPE_PCM), &data);
        assert!(matches!(
            decode_bytes(&one_bit, &DecodeOptions::default()),
            Err(DecodeError::Malformed { .. })
        ));

        // `fmt ` 前面有别的块时，块网格走查器必须照样看得见这块 `fmt `（第二处算术与第一处
        // 共用同一条扫描路径，因此这条对照必须跟着覆盖）。
        let four = [0xEEu8; 4];
        let junk: [([u8; 4], &[u8]); 2] = [(*b"JUNK", &four), (*b"LIST", &four)];
        let behind =
            wav_extensible_with_junk(&ext_spec(33, 8, 8, 0, WAVE_SUBTYPE_PCM), &junk, &data);
        assert!(
            matches!(
                decode_bytes(&behind, &DecodeOptions::default()),
                Err(DecodeError::Malformed { .. })
            ),
            "the precheck must find the extensible fmt chunk behind other chunks"
        );

        // 闭区间的另一侧：`channel_diff = 31` 时上游那次移位合法，因此本条闸门不许拒它
        // （上游随后会以 `UnsupportedFormat` 拒绝这份掩码，那是上游的判定，不是本闸门的）。
        let boundary = wav_extensible(&ext_spec(31, 8, 8, 0, WAVE_SUBTYPE_PCM), &data);
        match decode_bytes(&boundary, &DecodeOptions::default()) {
            Err(DecodeError::Malformed { detail }) => assert!(
                !detail.contains("shift-overflow"),
                "channel_diff = 31 does not overflow the shift, got {detail}"
            ),
            other => assert!(
                other.is_err(),
                "expected an upstream refusal, got {other:?}"
            ),
        }
    }

    /// 判据 ([ARCH-SEC-003] 的"只拒"一侧)：本条闸门只拒**上游真的会走到那次移位**的形状。
    ///
    /// 为什么需要它：`sub_format_guid` / `cbSize` / 位深这些分支在上游都排在移位**之前**，
    /// 认错一个方向就会把上游本来会正常报错（而不是 panic）的文件改说成"移位会溢出" ——
    /// 错误文案指错原因，而本 crate 的错误文案要进 MCP 响应体。这里直接跑预检本身，
    /// 因此"谁拒的"不靠文案猜。
    #[test]
    fn the_extensible_gate_refuses_only_the_shape_that_really_shifts_over() {
        let data = encode_int_samples(8, &[0x11, 0x22]);

        // 会走到移位 ⇒ 拒绝。
        for channels in [32u16, 33, u16::MAX] {
            let bytes = wav_extensible(&ext_spec(channels, 8, 8, 0, WAVE_SUBTYPE_PCM), &data);
            assert!(
                scan_precheck(&bytes).is_err(),
                "{channels} channels with an empty mask must be refused"
            );
        }

        // 上游不会走到移位 ⇒ 预检必须放行（判定留给上游）。
        let stays_with_upstream = [
            ("ambisonic pcm", WAVE_SUBTYPE_AMBISONIC_PCM, 8u16, 8u16),
            ("ambisonic ieee", WAVE_SUBTYPE_AMBISONIC_IEEE_FLOAT, 32, 32),
            ("unknown guid", [0u8; 16], 8, 8),
        ];
        for (label, guid, bits, valid_bits) in stays_with_upstream {
            let bytes = wav_extensible(&ext_spec(33, bits, valid_bits, 0, guid), &data);
            assert!(
                scan_precheck(&bytes).is_ok(),
                "{label}: upstream never calls the mask fix-up for this GUID, so the precheck \
                 must not refuse it"
            );
        }

        // 移位量 31：合法（`1u32 << 31`）。
        let thirty_one = wav_extensible(&ext_spec(31, 8, 8, 0, WAVE_SUBTYPE_PCM), &data);
        assert!(scan_precheck(&thirty_one).is_ok());

        // 上游在移位**之前**就报错的形状：预检也必须放行（否则错误文案会把原因指错）。
        // ① 声明长度 < 40（上游 `read_ext_fmt` 的第一条判据）。
        let mut short_declared = wav_extensible(&ext_spec(33, 8, 8, 0, WAVE_SUBTYPE_PCM), &data);
        let body = fmt_body_offset(&short_declared).expect("fixture has a fmt chunk");
        short_declared[body - 4..body].copy_from_slice(&17u32.to_le_bytes());
        assert!(
            scan_precheck(&short_declared).is_ok(),
            "a fmt chunk that declares fewer than 40 bytes never reaches the mask fix-up"
        );
        // ② `cbSize != 22`。
        let mut bad_cb_size = wav_extensible(&ext_spec(33, 8, 8, 0, WAVE_SUBTYPE_PCM), &data);
        let body = fmt_body_offset(&bad_cb_size).expect("fixture has a fmt chunk");
        bad_cb_size[body + 16..body + 18].copy_from_slice(&0u16.to_le_bytes());
        assert!(scan_precheck(&bad_cb_size).is_ok());
        // ③ 位深不是 8 的倍数。
        let mut bad_bits = wav_extensible(&ext_spec(33, 12, 12, 0, WAVE_SUBTYPE_PCM), &data);
        let body = fmt_body_offset(&bad_bits).expect("fixture has a fmt chunk");
        bad_bits[body + 14..body + 16].copy_from_slice(&12u16.to_le_bytes());
        assert!(scan_precheck(&bad_bits).is_ok());
        // ④ 子格式自己的位深约束不满足（a-law 只接受 8 位）。
        let alaw_16 = wav_extensible(&ext_spec(33, 16, 16, 0, WAVE_SUBTYPE_ALAW), &data);
        assert!(scan_precheck(&alaw_16).is_ok());
        // ⑤ `valid_bits > bits`（PCM 子格式）。
        let bad_valid = wav_extensible(&ext_spec(33, 8, 9, 0, WAVE_SUBTYPE_PCM), &data);
        assert!(scan_precheck(&bad_valid).is_ok());
    }

    /// 判据 ([ARCH-SEC-003] 的"只拒"一侧 / `channel_diff == 0` 那一格)：声道掩码里
    /// 置位的个数**恰好等于**声道数时，上游 `fix_wave_channel_mask` 的循环条件一开始
    /// 就为假（`channel_diff == 0`），那次移位根本不会执行 —— 因此即使掩码的第 31 位
    /// 是 1，本闸门也必须放行。
    ///
    /// 为什么需要它：既有的"不能拒"对照用的都是 `channel_diff < 0`（2.0 / 5.1 / 7.1
    /// 的掩码位数多于声道数）或空掩码，没有一条落在"`count_ones() == channels` 且最高
    /// 掩码位置位"这一格。把"声道数不多于掩码位数 ⇒ 不进移位分支"这条短路从 `>=`
    /// 放宽成 `>`，这一格就会被"外移会溢出"误拒。
    #[test]
    fn the_extensible_gate_stands_down_when_the_channel_difference_is_zero() {
        let data = encode_int_samples(8, &[0x11, 0x22]);
        for (channels, mask) in [(1u16, 0x8000_0000u32), (3, 0x8000_0003), (32, 0xFFFF_FFFF)] {
            let bytes = wav_extensible(&ext_spec(channels, 8, 8, mask, WAVE_SUBTYPE_PCM), &data);
            assert!(
                scan_precheck(&bytes).is_ok(),
                "{channels} channels with mask {mask:#010x}: the mask has exactly {channels} \
                 bits set, so upstream never enters the shift branch"
            );
        }
    }

    /// 判据 ([ARCH-SEC-003] 的"只拒"一侧)：`sub_format = IEEE_FLOAT` 时上游要求
    /// `valid_bits_per_sample == bits_per_sample`，不满足就在**移位之前**报错 ——
    /// 因此 `valid < bits` 的形状本闸门必须放行，哪怕它的 `channel_diff` 大到会溢出。
    ///
    /// 为什么需要它：既有的位深约束对照只覆盖 PCM 的 `valid > bits`（被上游拒绝）
    /// 那一侧，IEEE_FLOAT 的**相等**约束没有被任何判据走过。
    #[test]
    fn the_extensible_gate_stands_down_for_float_valid_bits_below_the_depth() {
        let data = encode_int_samples(8, &[0x11, 0x22]);
        // 33 声道 + 空掩码：位深与 valid 一致时确实会走到那次溢出移位。
        let overflows_when_reached =
            wav_extensible(&ext_spec(33, 32, 32, 0, WAVE_SUBTYPE_IEEE_FLOAT), &data);
        assert!(
            scan_precheck(&overflows_when_reached).is_err(),
            "the fixture must actually reach the overflowing shift"
        );
        // 同一个形状只把 valid_bits 降到 24：上游在移位之前就报错，本闸门必须放行。
        let valid_below_depth =
            wav_extensible(&ext_spec(33, 32, 24, 0, WAVE_SUBTYPE_IEEE_FLOAT), &data);
        assert!(
            scan_precheck(&valid_below_depth).is_ok(),
            "IEEE_FLOAT requires valid_bits == bits_per_sample; upstream errors before the shift"
        );
    }

    /// 判据 (闸门不得误拒)：合法的 `WAVE_FORMAT_EXTENSIBLE` 素材必须照常解出。
    ///
    /// 没有这条，上面两条判据可以用"拒绝一切 `0xFFFE`"来变绿 —— 而 `WAVE_FORMAT_
    /// EXTENSIBLE` 正是多声道母带最常见的封装，误拒它会让 5.1/7.1 素材整批导入失败。
    #[test]
    fn a_legal_extensible_wav_still_decodes() {
        let cases: [(u16, u32, &[i32]); 2] = [
            (2, 0x3, &[1_000, -1_000, 2_000, -2_000]),
            (
                6,
                0x3F,
                &[
                    100, -100, 200, -200, 300, -300, 400, -400, 500, -500, 600, -600,
                ],
            ),
        ];
        for (channels, mask, values) in cases {
            let data = encode_int_samples(16, values);
            assert_eq!(values.len(), usize::from(channels) * 2, "fixture shape");
            let bytes = wav_extensible(&ext_spec(channels, 16, 16, mask, WAVE_SUBTYPE_PCM), &data);
            let asset = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_or_else(|err| {
                panic!("a legal {channels}-channel extensible WAV must decode, got {err}")
            });
            assert_eq!(asset.channels(), channels);
            assert_eq!(asset.frame_count(), 2);
            assert_eq!(asset.pcm_format(), PcmFormat::S16);
        }
    }

    /// 判据 ([ARCH-SEC-003] / 不可信输入零 panic)：`fix_wave_channel_mask` 里那次移位的
    /// **外移**是第三处未检查算术，必须与内移一起在探测之前拦下。
    ///
    /// 上游事实（现位于 `wave/chunks.rs` 第 690 行）：
    /// `channel_mask |= ((1 << channel_diff) - 1) << shift`，其中
    /// `shift = 32 - (!channel_mask).leading_ones()` 是 mask 的**最高零位**。这次构造有
    /// 两个独立的 panic 来源：
    /// 1. **内移**：`channel_diff >= 32` ⇒ 移位量非法（改建前唯一覆盖的一侧）；
    /// 2. **外移**：`shift >= 32` ⇔ `(!mask).leading_ones() == 0` ⇔ mask 的第 31 位是 1。
    ///
    /// 实测（2026-10-10，本判据在**未改代码**时先跑）：`33` 声道 + `channel_mask =
    /// 0xAAAA_AAAA`（`channel_diff = 17`、`shift = 32`）让 `decode_bytes` 直接 panic 在
    /// `symphonia-format-riff` 的 `wave/chunks.rs` 第 690 行，消息
    /// `attempt to shift left with overflow`。把同一份 `(channels, mask)` 直接喂给逐字复制
    /// 的上游函数（`/tmp` 探针）复现同一读数；那一族里 `channel_mask = 0x8000_0000` 只有
    /// 一个掩码位，只要声道数 > 1 就落入其中，不属于边角形状。
    ///
    /// 闸门与上游的等价性有一条**穷举交叉验证**（同一个探针，用 `catch_unwind` 把逐字复制的
    /// 上游函数当基准）：2 184 组固定 `(channels, mask)` 与 300 000 组伪随机对上，改建后的
    /// 条件**漏判 0 组、误拒 0 组**（改建前的条件漏判 279 + 63 组）。
    ///
    /// 不能拒的对照同样重要：`channel_diff == 0` 的规范布局（2.0 / 5.1 / 7.1）上游根本不进
    /// 移位分支，必须放行 —— 没有这几条，本判据可以用"拒绝一切 `WAVE_FORMAT_EXTENSIBLE`"
    /// 来变绿，而 5.1/7.1 素材正是它最常见的用途。
    #[test]
    fn a_wave_extensible_fmt_whose_channel_mask_shift_overflows_is_refused_not_a_panic() {
        let data = encode_int_samples(16, &[0x1234, -0x1234]);

        // 这五条都必须被**预检**拒绝，逐条对应：
        //   · 前三条只让 `shift = 32`（mask 的第 31 位是 1）、`channel_diff` 都 < 32
        //     ⇒ **旧条件一个都挡不住**（这是本次补的那一族）；
        //   · 第四条走内移一侧（`channel_diff = 29 < 32`，只有 `shift = 32` 让它溢出）；
        //   · 最后一条是空掩码 + 33 声道（`channel_diff = 33 >= 32`，内移先炸），旧条件
        //     本来就挡得住 —— 留着它证明新条件没有漏掉老落点。
        let refused: [(u16, u32, &str); 5] = [
            (33, 0xAAAA_AAAA, "diff 17, shift 32"),
            (33, 0x8000_0000, "diff 32, shift 32"),
            (34, 0xAAAA_AAAA, "diff 18, shift 32"),
            (33, 0xCA00_0000, "diff 29, shift 32"),
            (33, 0x0000_0000, "diff 33, inner shift overflows"),
        ];
        for (channels, mask, why) in refused {
            let bytes = wav_extensible(&ext_spec(channels, 16, 16, mask, WAVE_SUBTYPE_PCM), &data);
            let err = match decode_bytes(&bytes, &DecodeOptions::default()) {
                Err(err) => err,
                Ok(asset) => panic!(
                    "{channels} channels with mask {mask:#010x} ({why}) decoded as \
                     channels={} frames={}, but its mask shift panics upstream",
                    asset.channels(),
                    asset.frame_count()
                ),
            };
            assert!(
                matches!(
                    &err,
                    DecodeError::Malformed { detail }
                        if detail.contains("shift-overflow") && detail.contains(&channels.to_string())
                ),
                "{channels} channels with mask {mask:#010x} ({why}) must be refused by the \
                 precheck, got {err}"
            );
        }

        // 预检本身（不经探测器）给出的也是同一个结论，因此"谁拒的"不靠文案猜。
        for (channels, mask) in [
            (33u16, 0xAAAA_AAAAu32),
            (33, 0x8000_0000),
            (33, 0xCA00_0000),
        ] {
            let bytes = wav_extensible(&ext_spec(channels, 16, 16, mask, WAVE_SUBTYPE_PCM), &data);
            assert!(
                scan_precheck(&bytes).is_err(),
                "{channels} channels with mask {mask:#010x} must be refused by the precheck"
            );
        }

        // 不能拒的对照一：`shift == 0`（mask 的最高位未置位）时上游那次外移不会溢出，
        // 即使 `channel_diff + shift > 32`（这里是 2 + 0）也不算 panic —— 上游随后按
        // "掩码与声道数不符"自行处置，本闸门不许替它拒绝。
        let top_bit_clear =
            wav_extensible(&ext_spec(2, 16, 16, 0x0000_0000, WAVE_SUBTYPE_PCM), &data);
        if let Err(DecodeError::Malformed { detail }) = scan_precheck(&top_bit_clear) {
            panic!("mask 0 has the top mask bit clear, so no shift overflows, got {detail}");
        }
        // 不能拒的对照二：规范布局（2.0 / 5.1 / 7.1）的 `channel_diff == 0`，上游根本不进
        // 移位分支。
        for (channels, mask) in [(2u16, 0x3u32), (6, 0x3F), (8, 0x63F)] {
            let bytes = wav_extensible(&ext_spec(channels, 16, 16, mask, WAVE_SUBTYPE_PCM), &data);
            assert!(
                scan_precheck(&bytes).is_ok(),
                "{channels} channels with the conforming mask {mask:#x} must pass the precheck"
            );
        }
    }

    /// 一个**不可回退**的源：`is_seekable() == false`，且每一次 `seek` 都失败。
    ///
    /// 为什么它代表真实输入：symphonia 把 `impl MediaSource for std::fs::File` 的
    /// `is_seekable()` 定义成 `metadata().is_file()`，所以 `decode_path` / `import_path`
    /// 收下的一条 **FIFO**（或字符设备）路径就落在这条分支上 —— 实测：指向 FIFO 时
    /// `metadata().is_file() == false`、`metadata().len() == 0`，输入字节闸门因此放行。
    /// 判据不真的建 FIFO（那要依赖 `mkfifo` 这个外部程序，且不是每个平台都有），只把
    /// `is_seekable` 报成 `false`：走的是**同一段**代码。
    struct NonSeekableSource {
        inner: Cursor<Vec<u8>>,
    }

    impl NonSeekableSource {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                inner: Cursor::new(bytes),
            }
        }
    }

    impl Read for NonSeekableSource {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Seek for NonSeekableSource {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this source cannot seek",
            ))
        }
    }

    impl MediaSource for NonSeekableSource {
        fn is_seekable(&self) -> bool {
            false
        }
        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// 判据 ([ARCH-SEC-003] / 不可信输入零 panic)：**不可回退的源**也必须过 RIFF 预检。
    ///
    /// 为什么需要它：预检要"扫一遍再把游标还给起点"，所以旧实现见到
    /// `!is_seekable()` 直接返回 `Ok(())` —— 那等于让上游那两处未检查算术完全不受本条
    /// 闸门约束。实测（2026-10-09，修复前）：33 声道 `WAVE_FORMAT_EXTENSIBLE` 与 32769
    /// 声道 tag 1 这两种输入，经不可回退源都**直接 panic**
    /// （`wave/chunks.rs` 第 690 行 / 第 100 行），而合法 WAV 同样经不可回退源却能
    /// `Ok(frames=2)` —— 所以既不能 fail open，也不能 fail closed。
    ///
    /// 本判据同时钉住"没有把不可回退的源变成回归"：第三段要求合法 WAV 经这条路径得到的
    /// 样本与内存入口**逐位相同**。
    #[test]
    fn a_non_seekable_source_is_buffered_so_the_precheck_still_runs() {
        let source =
            |bytes: Vec<u8>| -> Box<dyn MediaSource> { Box::new(NonSeekableSource::new(bytes)) };

        // ① tag 1 的 `u16` 乘法：32769 声道。
        let mut tag1 = int_wav(1, 16, &[0x1234, -0x1234]);
        patch_declared_channels(&mut tag1, 32_769);
        let err = decode_source(source(tag1), &Hint::new(), &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("32769")),
            "a non-seekable source must still be prechecked, got {err}"
        );

        // ② `WAVE_FORMAT_EXTENSIBLE` 的 `u32` 移位：33 声道 + 空掩码。
        let data = encode_int_samples(8, &[0x11, 0x22]);
        let extensible = wav_extensible(&ext_spec(33, 8, 8, 0, WAVE_SUBTYPE_PCM), &data);
        let err =
            decode_source(source(extensible), &Hint::new(), &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("shift-overflow")),
            "a non-seekable source must still be prechecked, got {err}"
        );

        // ③ 反向对照：合法输入经不可回退源必须解出**同一份**样本（缓冲不得改动字节）。
        let legal = int_wav(2, 16, &[1, -2, 3, -4, 5, -6, 7, -8]);
        let buffered = decode_source(
            source(legal.clone()),
            &Hint::new(),
            &DecodeOptions::default(),
        )
        .expect("a legal WAV from a non-seekable source must still decode");
        let direct = decode_bytes(&legal, &DecodeOptions::default()).unwrap();
        assert_eq!(buffered.frame_count(), direct.frame_count());
        assert_eq!(buffered.pcm_hash(), direct.pcm_hash());
        assert_eq!(
            buffered
                .samples()
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>(),
            direct
                .samples()
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>()
        );
    }

    /// 判据 ([ARCH-SEC-003] / `HD-24` 第五道闸门)：不可回退的源也要过**输入字节**闸门。
    ///
    /// 为什么需要它：不可回退的源无法先量长度（`byte_len()` 是 `None`），因此旧实现里
    /// 这类输入**完全绕过**了 `check_input_len`。缓冲之后它在读取过程中边读边判，判定与
    /// [`limits::check_input_len`] 同口径：**闭区间**（恰好等于上限通过），错误里带精确的
    /// 投影字节数与生效上限。
    #[test]
    fn a_non_seekable_source_obeys_the_input_byte_budget() {
        let limit = 256u64;
        let budget = PcmBudget::new(
            limit,
            !3u64,
            64,
            768_000,
            60,
            limits::DEFAULT_MAX_RESAMPLE_RATIO,
        );
        let options = DecodeOptions {
            budget,
            ..DecodeOptions::default()
        };

        // 恰好 `limit` 字节：**不**因输入字节被拒（内容当然是垃圾，因此另有其错）。
        let exact = vec![0u8; usize::try_from(limit).unwrap()];
        if let Err(DecodeError::Budget(LimitViolation::InputTooLarge { bytes, limit: got })) =
            decode_source(
                Box::new(NonSeekableSource::new(exact)),
                &Hint::new(),
                &options,
            )
        {
            panic!("exactly {bytes} bytes must pass the {got}-byte cap (closed interval)");
        }

        // 多一个字节：必须报 `InputTooLarge`，且数字是投影值 `limit + 1`。
        let over = vec![0u8; usize::try_from(limit + 1).unwrap()];
        match decode_source(
            Box::new(NonSeekableSource::new(over)),
            &Hint::new(),
            &options,
        ) {
            Err(DecodeError::Budget(LimitViolation::InputTooLarge { bytes, limit: got })) => {
                assert_eq!(bytes, limit + 1);
                assert_eq!(got, limit);
            }
            other => panic!(
                "expected InputTooLarge for {} bytes, got {other:?}",
                limit + 1
            ),
        }
    }

    /// 判据 ([ARCH-SEC-003] / `HD-24` 第五道闸门)：不可回退的源必须在**读的过程中**就被
    /// 输入字节闸门切断，而不是"整份读完再判"。
    ///
    /// 为什么需要它：`a_non_seekable_source_obeys_the_input_byte_budget` 只钉返回的错误
    /// 变体与数字。缓冲之后 `decode_source` 还会对 `Cursor` 的 `byte_len()` 再判一次，
    /// 因此即使把 `slurp_unseekable` 的上限换成 `u64::MAX`，那条判据读到的错误
    /// （`InputTooLarge { bytes: limit + 1, limit }`）**逐字相同** —— 两道判定的可观察
    /// 结果重合，掩盖了"输入已经被整份读进内存"这件事。本判据改从**源一侧**计量：
    /// 一个能提供 8 个 64 KiB 分块的源，在上限 256 字节下被读走的字节数必须不超过一个
    /// 读分块（`SLURP_CHUNK_BYTES`），且错误里的字节数是第一个分块的投影值。
    #[test]
    fn an_unseekable_source_is_cut_off_while_reading_not_after() {
        struct CountingUnseekable {
            remaining: usize,
            read: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        }

        impl Read for CountingUnseekable {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let filled = buf.len().min(self.remaining);
                buf[..filled].fill(0);
                self.remaining -= filled;
                self.read
                    .fetch_add(filled, std::sync::atomic::Ordering::SeqCst);
                Ok(filled)
            }
        }

        impl Seek for CountingUnseekable {
            fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "this source cannot seek",
                ))
            }
        }

        impl MediaSource for CountingUnseekable {
            fn is_seekable(&self) -> bool {
                false
            }
            fn byte_len(&self) -> Option<u64> {
                None
            }
        }

        let limit = 256u64;
        let options = DecodeOptions {
            budget: PcmBudget::new(
                limit,
                !3u64,
                64,
                768_000,
                60,
                limits::DEFAULT_MAX_RESAMPLE_RATIO,
            ),
            ..DecodeOptions::default()
        };
        let read = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let available = 8 * SLURP_CHUNK_BYTES;
        let source = CountingUnseekable {
            remaining: available,
            read: std::sync::Arc::clone(&read),
        };

        match decode_source(Box::new(source), &Hint::new(), &options) {
            Err(DecodeError::Budget(LimitViolation::InputTooLarge { bytes, limit: got })) => {
                assert_eq!(
                    (bytes, got),
                    (u64::try_from(SLURP_CHUNK_BYTES).unwrap(), limit),
                    "the refusal must carry the projection of the first read chunk"
                );
            }
            other => panic!("expected InputTooLarge after the first chunk, got {other:?}"),
        }
        let consumed = read.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            consumed <= SLURP_CHUNK_BYTES,
            "the gate must fire during the read: {consumed} bytes were consumed out of \
             {available} available"
        );
    }

    /// 判据 ([ARCH-SEC-003] / `HD-24` 第五道闸门)：**可回退**的源经公共入口
    /// [`decode_source`] 也必须过输入字节闸门。
    ///
    /// 为什么需要它：[`decode_path`] / [`decode_bytes`] / [`decode_reader`] 都在调用
    /// [`decode_source`] **之前**各判了一次输入字节，因此走这三个入口一切正常；而
    /// `decode_source` 自己是 `pub` 的，直接用它收下一个可回退的源（`File`、`Cursor`、
    /// [`MeasuredSource`] 都是）时，这道闸门此前**完全没有被求值** —— 同一份 52 字节的
    /// WAV、同一个 16 字节的预算，经 `decode_bytes` 是
    /// `InputTooLarge { bytes: 52, limit: 16 }`，经 `decode_source` 却是 `Ok(frames=4)`。
    ///
    /// 三段断言：① 内存入口拒绝（对照读数）；② 同一个预算下可回退的 `Cursor` 源同样拒绝，
    /// 数字逐字相同；③ 闭区间 —— 上限设成恰好 52 字节时同一个源必须**解出**同一份资产
    /// （证明这条闸门没有变成"见源就拒"）。
    ///
    /// 注入：把 `decode_source` 里那段 `if let Some(len) = source.byte_len()` 删掉（回到
    /// 修复前的形状）⇒ 第二段断言红，字面读数是 `Ok(...)` 而不是 `InputTooLarge`。
    #[test]
    fn a_seekable_source_obeys_the_input_byte_budget_through_decode_source() {
        // 44 字节头 + 4 帧 × 2 字节 = 52 字节。
        let bytes = int_wav(1, 16, &[1, -2, 3, -4]);
        assert_eq!(bytes.len(), 52);
        let capped = |limit: u64| DecodeOptions {
            budget: PcmBudget::new(
                limit,
                !3u64,
                64,
                768_000,
                60,
                limits::DEFAULT_MAX_RESAMPLE_RATIO,
            ),
            ..DecodeOptions::default()
        };

        // ① 内存入口：16 字节上限拒绝 52 字节输入。
        assert!(matches!(
            decode_bytes(&bytes, &capped(16)),
            Err(DecodeError::Budget(LimitViolation::InputTooLarge {
                bytes: 52,
                limit: 16
            }))
        ));

        // ② 可回退的源（`Cursor`，`byte_len() == Some(52)`）：同一个拒绝。
        match decode_source(
            Box::new(Cursor::new(bytes.clone())),
            &Hint::new(),
            &capped(16),
        ) {
            Err(DecodeError::Budget(LimitViolation::InputTooLarge { bytes: got, limit })) => {
                assert_eq!(got, 52);
                assert_eq!(limit, 16);
            }
            other => panic!("a seekable source must obey the input-byte cap, got {other:?}"),
        }

        // ③ 闭区间：上限恰好 52 字节 ⇒ 同一个源必须解出资产（4 帧），不得误拒。
        let exact = decode_source(Box::new(Cursor::new(bytes)), &Hint::new(), &capped(52))
            .expect("exactly 52 bytes must pass a 52-byte cap (closed interval)");
        assert_eq!(exact.frame_count(), 4);
        assert_eq!(exact.channels(), 1);
    }

    #[test]
    fn decoding_the_same_bytes_twice_is_bit_identical() {
        // [ARCH-DET-001] 判据：同输入 → 同输出，逐样本相同、内容摘要相同。
        let bytes = int_wav(2, 16, &[1, -2, 3, -4, 5, -6, 7, -8]);
        let first = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        let second = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(first.samples(), second.samples());
        assert_eq!(first.pcm_hash(), second.pcm_hash());
        assert_eq!(
            first
                .samples()
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>(),
            second
                .samples()
                .iter()
                .map(|s| s.to_bits())
                .collect::<Vec<_>>(),
            "bit pattern (not just value) must match"
        );
    }

    #[test]
    fn a_data_region_mutation_still_decodes_and_changes_the_hash() {
        // 这条是 `propcheck::untrusted_bytes_never_panic_and_ok_results_are_self_consistent`
        // 的**非空洞证据**：那条属性判据的第三族输入（只改 `data` 区）确实会走到 Ok 分支，
        // 所以它的 Ok 不变量不是空判据。同时它把 `pcm_hash` 的敏感性从"手搭的
        // `DecodedAsset`"推到"真实解码出来的样本"。
        let spec = int_spec(1, 16);
        let mut bytes = wav(&spec, &encode_int_samples(16, &[100, -100, 200, -200]));
        let baseline = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let mutated = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(mutated.frame_count(), baseline.frame_count());
        assert_ne!(
            mutated.pcm_hash(),
            baseline.pcm_hash(),
            "改一个真实样本字节必须改内容摘要"
        );
    }

    #[test]
    fn decode_reader_and_decode_bytes_agree() {
        let bytes = int_wav(2, 16, &[10, -10, 20, -20]);
        let from_bytes = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        let from_reader =
            decode_reader(Cursor::new(bytes.clone()), &DecodeOptions::default()).unwrap();
        assert_eq!(from_bytes.samples(), from_reader.samples());
        assert_eq!(from_bytes.pcm_hash(), from_reader.pcm_hash());
    }

    #[test]
    fn decode_reader_restores_the_start_position_of_a_partial_cursor() {
        // 在一个更长的字节数组里嵌入一个 WAV，从中间开始读：MeasuredSource 必须
        // 保留原位置，且 byte_len 反映的是真实文件长度。
        let bytes = int_wav(1, 16, &[100, 200, 300, 400]);
        let mut padded = vec![0u8; 64];
        padded.extend_from_slice(&bytes);
        let mut cursor = Cursor::new(padded);
        cursor.set_position(64);
        let asset = decode_reader(cursor, &DecodeOptions::default()).unwrap();
        assert_eq!(asset.frame_count(), 4);
    }

    #[test]
    fn decode_path_matches_decode_bytes() {
        let bytes = int_wav(1, 16, &[5, -5, 15, -15]);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "yeban-decode-fixture-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, &bytes).unwrap();
        let from_path = decode_path(&path, &DecodeOptions::default()).unwrap();
        let cleanup = std::fs::remove_file(&path);
        let from_bytes = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(from_path.samples(), from_bytes.samples());
        cleanup.unwrap();
    }

    /// 判据 (时长闸门的**逐包**落点)：**没有声明帧数**时，时长闸门必须在解码过程中逐包
    /// 生效，而不是"先把整段解完、再在终检拒掉"。
    ///
    /// 为什么需要它：`decode_source` 只对 `declared_frames` 做预检（`if let Some`），
    /// 而解码循环只按字节预算逐包判。`declared_frames == None` 的输入因此会绕过预检，
    /// 峰值缓冲由 `max_pcm_bytes` 决定 —— 8 kHz 单声道下那是约 75 小时，是 6 小时时长
    /// 上界的 12.6×（`docs/ledger/decode-limits-notes.md` §2.2 的"字节便宜、时间昂贵"格）。
    ///
    /// 实测形状（本判据的基座，FLAC 的 `STREAMINFO.total_samples == 0` 就是"未知"的
    /// 规范写法）：
    /// - 该输入的读数曾经是 `declared_frames=None, frame_count=25600`；
    /// - 预算收到 1 秒时的读数曾经是
    ///   `25600 frames at 8000 Hz is 3 s of audio, over the 1-second duration cap`
    ///   —— 即"整段解完之后才拒"。
    ///
    /// 判据用**报告出来的帧数**把两种实现区分开：逐包判定报的是"越界那一刻的投影帧数"
    /// （8192，第 32 包），终检报的是"整段的总帧数"（25600）。只看"返回了
    /// `DurationTooLong`"是不够的 —— 那个错误在修复前后都会出现。
    #[test]
    fn the_duration_gate_fires_on_undeclared_frames_while_decoding() {
        // FLAC 的 `total_samples` 写 0：规范含义是"未知"，因此 `track.num_frames == None`。
        let spec = FlacSpec {
            total_samples_override: Some(0),
            ..FlacSpec::default()
        };
        let bytes = flac_constant(&spec, 100, 0); // 100 块 × 256 帧 = 25600 帧 @8 kHz
        let total = decode_bytes(&bytes, &DecodeOptions::default())
            .unwrap()
            .frame_count();
        assert_eq!(total, 25_600, "the fixture must be longer than the gate");

        let one_second = DecodeOptions {
            budget: PcmBudget {
                max_duration_secs: 1,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let err = decode_bytes(&bytes, &one_second).unwrap_err();
        let DecodeError::Budget(LimitViolation::DurationTooLong {
            frames,
            sample_rate,
            limit_secs,
            ..
        }) = err
        else {
            panic!("expected DurationTooLong, got {err}");
        };
        assert_eq!(sample_rate, 8_000);
        assert_eq!(limit_secs, 1);
        // 每包 256 帧、上界 8000 帧 ⇒ 前 31 包（7936 帧）合法，第 32 包投影到 8192 帧
        // 时越界。终检则会报整段的 25600 帧。
        assert_eq!(
            frames,
            32 * 256,
            "the gate must fire on the packet that crosses the limit, not after \
             decoding all {total} frames"
        );
        assert!(
            frames < total,
            "reporting {frames} frames proves the loop stopped early; the end-of-stream \
             check would have reported {total}"
        );
        // 恰好 1 秒的上界必须仍然通过（闭区间，逐包判定不能把闸门焊死）。
        let exact_bytes = flac_constant(&spec, 31, 0); // 31 × 256 = 7936 帧 < 8000
        assert_eq!(
            decode_bytes(&exact_bytes, &one_second)
                .unwrap()
                .frame_count(),
            7_936
        );
        let over_bytes = flac_constant(&spec, 33, 0); // 33 × 256 = 8448 帧 > 8000
        assert!(matches!(
            decode_bytes(&over_bytes, &one_second),
            Err(DecodeError::Budget(LimitViolation::DurationTooLong { .. }))
        ));
        // 同一份输入在默认预算下正常解出 ⇒ 上面红的是时长闸门，不是格式。
        assert_eq!(
            decode_bytes(&bytes, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            total
        );
    }

    #[test]
    fn the_duration_gate_fires_independently_of_the_byte_budget() {
        // 8000 Hz 单声道 2 秒 = 16000 帧，PCM 只有 64 KB —— 任何字节预算都拦不住它，
        // 唯一能拦下它的是**时长闸门**（与声道数、字节数都无关的独立闸门）。
        let bytes = int_wav(1, 16, &[0; 16_000]);
        let one_second = DecodeOptions {
            budget: PcmBudget {
                max_duration_secs: 1,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let err = decode_bytes(&bytes, &one_second).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::Budget(LimitViolation::DurationTooLong { .. })
            ),
            "expected DurationTooLong, got {err}"
        );
        assert!(err.to_string().contains("duration cap"), "got {err}");
        // 恰好 1 秒（8000 帧）必须通过 —— 闸门是闭区间。
        let exact = int_wav(1, 16, &[0; 8_000]);
        assert_eq!(
            decode_bytes(&exact, &one_second).unwrap().frame_count(),
            8_000
        );
        // 同一份 2 秒字节在默认预算下正常解出 ⇒ 上面红的确实是时长闸门。
        assert_eq!(
            decode_bytes(&bytes, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            16_000
        );
    }

    /// 判据 ([ARCH-SEC-003] 闸门站点)：容器**声明**的时长超过预算时，必须在开始解码
    /// 之前就被时长闸门挡下 —— 报出来的帧数是声明折算值，而不是"真读到文件尾"的 I/O 错误。
    ///
    /// 为什么需要它：既有的时长判据用的都是"文件里真有那么多帧"的夹具，因此"按声明先判
    /// 一次"与"只逐包判"给出同一个变体与同一个数字；
    /// `a_declared_data_length_far_beyond_the_real_bytes_returns_instead_of_looping`
    /// 只要求 `is_err()`，同样分不开两者。本判据的夹具声明 200 000 字节的 `data`
    /// （16-bit 单声道 ⇒ 100 000 帧）而只有 64 字节真实样本：只有把声明折算成帧数再判，
    /// 才会得到 `DurationTooLong` 与声明值；否则先发生的是上游在真实字节耗尽时的 I/O 错误。
    #[test]
    fn a_declared_duration_over_the_cap_is_refused_before_decoding() {
        let spec = int_spec(1, 16);
        let real_data = encode_int_samples(16, &[1_000; 32]); // 32 帧 = 64 字节
        let declared_bytes = 200_000u32; // 声明的 `data` 长度 ⇒ 100 000 帧
        let bytes = wav_with_declared_len(&spec, &real_data, declared_bytes);
        let one_second = DecodeOptions {
            budget: PcmBudget {
                max_duration_secs: 1,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        match decode_bytes(&bytes, &one_second) {
            Err(DecodeError::Budget(LimitViolation::DurationTooLong {
                frames,
                sample_rate,
                seconds,
                limit_secs,
            })) => {
                assert_eq!(
                    (frames, sample_rate, seconds, limit_secs),
                    (100_000, 8_000, 12, 1),
                    "the refusal must carry the declared frame count, not the decoded one"
                );
            }
            other => panic!(
                "a declared 100 000-frame stream must be refused by the declared-length \
                 pre-check, got {other:?}"
            ),
        }
        // 对照：同一份畸形字节在默认预算下仍然返回错误（它本来就是截断的流），
        // 因此上面红的是那道按声明判的时长闸门，不是这份夹具整类不可解。
        assert!(decode_bytes(&bytes, &DecodeOptions::default()).is_err());
    }

    #[test]
    fn the_channel_and_rate_gates_fire_on_their_own() {
        let stereo = int_wav(2, 16, &[1_000; 64]);

        // 声道闸门：预算只允许 1 声道，而素材是立体声。
        let mono_only = DecodeOptions {
            budget: PcmBudget {
                max_channels: 1,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let err = decode_bytes(&stereo, &mono_only).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::Budget(LimitViolation::TooManyChannels {
                    channels: 2,
                    limit: 1
                })
            ),
            "expected TooManyChannels, got {err}"
        );
        assert!(decode_bytes(&stereo, &DecodeOptions::default()).is_ok());

        // 采样率闸门：素材 8000 Hz，预算只允许 4000 Hz。
        let narrow = DecodeOptions {
            budget: PcmBudget {
                max_sample_rate: 4_000,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let err = decode_bytes(&stereo, &narrow).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::Budget(LimitViolation::SampleRateTooHigh {
                    rate: 8_000,
                    limit: 4_000
                })
            ),
            "expected SampleRateTooHigh, got {err}"
        );
    }

    #[test]
    fn the_pcm_budget_boundary_is_closed_through_the_decoder() {
        // 单声道 64 帧 ⇒ 256 字节交织 f32 PCM。
        let bytes = int_wav(1, 16, &[100; 64]);
        let exact = DecodeOptions {
            budget: PcmBudget {
                max_pcm_bytes: 256,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert_eq!(decode_bytes(&bytes, &exact).unwrap().frame_count(), 64);
        // 少 4 字节（= 少一个样本）就必须被拒：闸门不是 `>=`。
        let short = DecodeOptions {
            budget: PcmBudget {
                max_pcm_bytes: 252,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        let err = decode_bytes(&bytes, &short).unwrap_err();
        assert!(
            matches!(
                err,
                DecodeError::Budget(LimitViolation::PcmBudgetExceeded {
                    frames: 64,
                    channels: 1,
                    samples: 64,
                    limit_samples: 63,
                })
            ),
            "expected PcmBudgetExceeded at the boundary, got {err}"
        );
        assert!(err.to_string().contains("PCM budget"), "got {err}");
    }

    #[test]
    fn the_default_options_carry_the_derived_budget() {
        // 单一事实源：默认预算只在 `PcmBudget::default()` 里推导一次。
        assert_eq!(DecodeOptions::default().budget, PcmBudget::default());
    }

    /// 判据 (MUST-GATE-011 防挂死)：`data` 块**谎报**长度时必须返回类型化错误，而且
    /// **必须返回** —— 这条判据的对照物是"解码循环永远转下去"。
    ///
    /// 为什么需要它：`truncated_wav_is_an_error_not_a_panic` 只截断**真实字节**，
    /// 因此 `data` 头里声明的长度始终等于真实长度；`[limits::MAX_IDLE_PACKETS]` 的
    /// 长注释点名的那一类输入（"`data` 头完整、但数据体被截断"）此前**没有任何判据**
    /// 走过：夹具 `crate::testfix::wav_with_declared_len` 的存在就是为它准备的，
    /// 但它在**解码侧一次也没有被调用过**（只有 `testfix` 自己的夹具判据用它）。
    ///
    /// 本判据把那一类输入钉成"有界返回"：`decode_bytes` 面对"声明 16 MiB / 真实 64 B"
    /// 的文件必须在**有限步**内返回 `Err`。若将来某次重构让解码循环对
    /// `frames_in_buffer == 0` 的包不再返回、也不再记账，这条判据就会挂住 ——
    /// 这正是它存在的意义。
    ///
    /// **诚实声明**：本判据**不能**证明 [`limits::IdleGuard`] 是跳闸的那道闸门。
    /// 实测（见提交信息）是 symphonia 的 RIFF/WAVE 读端在真实字节耗尽时先返回
    /// `UnexpectedEof`：把 `decode.rs` 的 `bump_idle` 记账停掉（临时注入
    /// `if idle.bump() && false`）之后重新编译，本判据与全部既有判据**照样通过**，
    /// 字面读数与基线**完全相同**（`declared=128/4096/65535` 全部
    /// `Err(Io(UnexpectedEof))`）。本判据钉的是不变量（有界返回），不是那条记账路径本身。
    #[test]
    fn a_declared_data_length_far_beyond_the_real_bytes_returns_instead_of_looping() {
        let spec = int_spec(1, 16);
        let real_data = encode_int_samples(16, &[1_000; 32]); // 32 帧 = 64 字节
        for declared in [real_data.len() as u32 + 64, 4_096, 16 * 1024 * 1024] {
            let bytes = wav_with_declared_len(&spec, &real_data, declared);
            // 头里声称的长度是真的，真实字节远少于此。
            assert!(
                u64::from(declared) > real_data.len() as u64,
                "fixture must lie about its data length"
            );
            let outcome = decode_bytes(&bytes, &DecodeOptions::default());
            assert!(
                outcome.is_err(),
                "declared {declared} bytes with {} real bytes must not decode \
                 as a complete asset, got {:?}",
                bytes.len(),
                outcome.map(|asset| asset.frame_count())
            );
        }
    }

    /// 判据（类别⑦ 块长度极值）：1 帧与**非 2 的幂**帧数的流按容器声明逐帧解出；一路信号
    /// 相同的立体声流解出的交织对逐位相同；合法但 0 帧的流按既有口径返回
    /// [`DecodeError::EmptyStream`]。
    ///
    /// 0 帧那半的必要性：[`DecodeError::EmptyStream`] 的口径写在它自己的文档里
    /// （"夜半不导入零长资产"），但此前**没有任何判据**用合法的 0 帧容器走到它 ——
    /// `arbitrary_garbage_is_an_error_not_a_panic` 只要求"是错误"，因此一个把 0 帧流
    /// 当成 `Ok(0 帧)` 的回归在那条判据下仍然全绿。本判据钉的是**具体变体**与**具体文案**。
    ///
    /// 实测读数（本机）：44 字节头 + 0 字节 `data` 的 16-bit 单声道 WAV 在
    /// `decode_bytes` / `decode_reader` / `decode_path` / `import_bytes` 四个入口上全部
    /// 返回 `EmptyStream`（文案 `stream decoded to zero audio frames`）；同一个夹具在
    /// 1 / 2 / 3 / 5 / 1023 / 1024 / 1025 / 2047 帧下解出的帧数逐条等于声明值。
    ///
    /// 注入：把取得布局那一步的 `ok_or(DecodeError::EmptyStream)` 换成 `Malformed` ⇒
    /// 本条以 `decode_bytes: expected EmptyStream, got malformed stream: injected
    /// zero-frame variant` 红，且**只有**本条红（`110 passed / 1 failed`）。
    /// ⚠ 另一条信息给下一位：只把解码循环**之后**那次 `frames == 0` 的返回变体改掉
    /// **不会**变红 —— 0 帧 WAV 一个样本都没解出，因此它在"取得布局"那一步就已是
    /// `None`，根本走不到循环之后的那次判定。注入点位必须选对，否则会误判"判据无效"。
    #[test]
    fn one_frame_and_non_power_of_two_streams_decode_exactly_and_zero_frames_are_refused() {
        let spec = int_spec(1, 16);
        for frames in [1usize, 2, 3, 5, 1_023, 1_024, 1_025, 2_047] {
            let data = encode_int_samples(16, &vec![100i32; frames]);
            let asset = decode_bytes(&wav(&spec, &data), &DecodeOptions::default())
                .unwrap_or_else(|err| panic!("{frames} frames must decode: {err}"));
            assert_eq!(asset.frame_count(), u64::try_from(frames).unwrap());
            assert_eq!(asset.channels(), 1);
            assert_eq!(asset.samples().len(), frames);
        }

        // 同一信号喂两路：交织对必须逐位相同（多声道一致性的解码侧落点）。
        let mut values = Vec::new();
        for index in 0..513usize {
            let value = i32::try_from(index % 101).unwrap() * 100 - 5_000;
            values.push(value);
            values.push(value);
        }
        let stereo = wav(&int_spec(2, 16), &encode_int_samples(16, &values));
        let asset = decode_bytes(&stereo, &DecodeOptions::default()).unwrap();
        assert_eq!(asset.frame_count(), 513);
        let mismatched = asset
            .samples()
            .chunks(2)
            .filter(|pair| pair[0].to_bits() != pair[1].to_bits())
            .count();
        assert_eq!(
            mismatched, 0,
            "the same declared value in both channels must decode to the same bits"
        );

        // 合法但 0 帧：`data` 块声明 0 字节，整份文件是格式完全正确的 WAV。
        let empty = wav(&spec, &[]);
        assert_eq!(empty.len(), 44, "the fixture is a bare 44-byte WAV header");
        let outcomes: [(&str, DecodeResult<DecodedAsset>); 2] = [
            (
                "decode_bytes",
                decode_bytes(&empty, &DecodeOptions::default()),
            ),
            (
                "decode_reader",
                decode_reader(Cursor::new(empty.clone()), &DecodeOptions::default()),
            ),
        ];
        for (label, outcome) in outcomes {
            let err = outcome.expect_err("a zero-frame stream must be refused");
            assert!(
                matches!(err, DecodeError::EmptyStream),
                "{label}: expected EmptyStream, got {err}"
            );
            assert!(
                err.to_string().contains("zero audio frames"),
                "{label}: the refusal must name the cause, got {err}"
            );
        }

        let mut path = std::env::temp_dir();
        path.push(format!("yeban-decode-empty-{}.wav", std::process::id()));
        std::fs::write(&path, &empty).unwrap();
        let from_path = decode_path(&path, &DecodeOptions::default());
        let cleanup = std::fs::remove_file(&path);
        let err = from_path.expect_err("a zero-frame file must be refused");
        assert!(
            matches!(err, DecodeError::EmptyStream),
            "decode_path: expected EmptyStream, got {err}"
        );
        cleanup.unwrap();

        assert!(
            matches!(
                crate::asset::import_bytes(
                    &empty,
                    "empty.wav",
                    "CC0-1.0",
                    &DecodeOptions::default()
                ),
                Err(DecodeError::EmptyStream)
            ),
            "import_bytes must report the same refusal"
        );
    }

    /// 样本的位模式。类别③与类别⑥的判据只承认**逐位**相同，不承认"值相等"
    /// （`-0.0` 与 `+0.0` 值相等而位模式不等，`pcm_hash` 也区分二者）。
    fn sample_bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|sample| sample.to_bits()).collect()
    }

    /// 判据（③-6 类别③ 复位/重新打开后与全新实例一致）：**同一个磁盘路径**关掉再打开，
    /// 与"全新一次打开"逐位一致；两个各自全新的 reader 同样一致；`import_path` 的
    /// **两遍遍历**（摘要一遍 + 解码一遍）也不留任何残余。
    ///
    /// 为什么需要它：本 crate **没有** `open`/`close` 句柄。机械枚举的口径与读数
    /// （全 crate，`grep -rn` 与 `grep -rnw` 两种，单位都是"命中行数"）：
    /// `\bclose\b` = **0** 行（16 行 `close` 全是 `fail closed` / `assert_close` /
    /// `is_closed` 的子串）；`open` 4 行 = `grep -rnw open` 也是 4 行（**没有**假阳性），
    /// 其中 2 行是 `File::open`（现位于第 84 行与第 392 行）、2 行是注释里的 `fail open`。
    /// 因此"open → close → open"在本 crate 的**唯一**可达形态就是"同一个路径被打开两次"，
    /// 而此前没有任何判据打开过同一个路径两次：`decode_path_matches_decode_bytes` 只解一次
    /// 磁盘路径，`importing_the_same_bytes_twice_yields_the_same_keys` 只覆盖内存入口。
    ///
    /// 与既有判据的分工（不重复计量同一件事）：
    /// - `decoding_the_same_bytes_twice_is_bit_identical` 钉**内存**入口的重复施加；
    /// - `decode_path_matches_decode_bytes` 钉"两个**不同**入口对同一份字节给同一结果"；
    /// - 本条钉"同一个路径被**重新打开**"（`metadata` 与 `File::open` 各走两遍）以及
    ///   `import_path` 的**两遍遍历**都不留残余。
    ///
    /// 全 crate 唯一的可变状态对象是 [`limits::IdleGuard`]，它的"复位后 == 全新实例"由
    /// `repeated_resets_leave_the_guard_in_the_same_state_as_a_single_reset` 钉住；
    /// 重采样侧那四个入口每次都**新建**一个重采样器（`Async::new_sinc` 在全 crate 只有
    /// 一个调用点），因此"换主人后与全新实例一致"对它们是构造性质，由
    /// `every_resample_entry_is_idempotent_on_repeated_application` 从外部读数钉住。
    ///
    /// 实测读数（本机）：本判据的全部断言都是**逐位**相等，没有一处需要容差；
    /// 两条路径、两个 reader、两次流式导入的 `pcm_hash` / `asset_hash` 全部相等。
    ///
    /// 注入（证明本条不是空判据）：在 `decode_source` 里放一个调用计数器，让第 2、4、… 次
    /// 调用在终检之前 `samples.pop()` 一次（模拟"重新打开读到了别的状态"）⇒ 本条在第二次
    /// `decode_path` 处以 `DurationMismatch(Mismatch { declared: 1000, decoded: 999,
    /// tolerance: 0, delta: 1 })` 红；只跑本条时读数是 `0 passed / 1 failed`。
    #[test]
    fn reopening_the_same_path_reproduces_a_bit_identical_asset() {
        let spec = int_spec(2, 16);
        let values: Vec<i32> = (0..2_000)
            .map(|index| (index % 97) * 300 - 14_400)
            .collect();
        let bytes = wav(&spec, &encode_int_samples(16, &values));

        let mut path = std::env::temp_dir();
        path.push(format!("yeban-decode-reopen-{}.wav", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();

        // ① 同一个路径打开两次（中间没有别的读者）：样本位模式、事实、内容摘要三者全等。
        let first = decode_path(&path, &DecodeOptions::default()).unwrap();
        let second = decode_path(&path, &DecodeOptions::default()).unwrap();
        assert_eq!(
            sample_bits(first.samples()),
            sample_bits(second.samples()),
            "reopening the same path must restore the same sample bits"
        );
        assert_eq!(first.facts(), second.facts());
        assert_eq!(first.pcm_hash(), second.pcm_hash());
        assert_eq!(first.frame_count(), second.frame_count());

        // ② 重新打开的结果必须等于"同一份字节的内存入口"的结果 —— 通道、采样率与
        //    位模式一起比，因此"第二次打开读到了别的偏移"这类缺陷会在这里红。
        let from_bytes = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(first.pcm_hash(), from_bytes.pcm_hash());
        assert_eq!(
            sample_bits(first.samples()),
            sample_bits(from_bytes.samples())
        );
        assert_eq!(first.channels(), from_bytes.channels());
        assert_eq!(first.sample_rate(), from_bytes.sample_rate());

        // ③ 两个各自全新的 `Read + Seek`（reader 侧的"重新打开"）：`MeasuredSource` 的
        //    定长测量不能留下与调用次数有关的状态。
        let reader_first =
            decode_reader(Cursor::new(bytes.clone()), &DecodeOptions::default()).unwrap();
        let reader_second =
            decode_reader(Cursor::new(bytes.clone()), &DecodeOptions::default()).unwrap();
        assert_eq!(reader_first.pcm_hash(), reader_second.pcm_hash());
        assert_eq!(
            sample_bits(reader_first.samples()),
            sample_bits(reader_second.samples())
        );

        // ④ 两遍遍历的磁盘导入入口：摘要是第一遍、解码是第二遍，两次导入必须给出同一把
        //    CAS 键与同一份 PCM，并且与内存入口的键相同。
        let imported_first =
            crate::asset::import_path(&path, "CC0-1.0", &DecodeOptions::default()).unwrap();
        let imported_second =
            crate::asset::import_path(&path, "CC0-1.0", &DecodeOptions::default()).unwrap();
        let cleanup = std::fs::remove_file(&path);
        assert_eq!(imported_first.asset_hash(), imported_second.asset_hash());
        assert_eq!(imported_first.pcm_hash(), imported_second.pcm_hash());
        assert_eq!(
            imported_first.index.byte_len,
            imported_second.index.byte_len
        );
        assert_eq!(imported_first.index.byte_len, bytes.len() as u64);

        let imported_bytes =
            crate::asset::import_bytes(&bytes, "reopen.wav", "CC0-1.0", &DecodeOptions::default())
                .unwrap();
        assert_eq!(imported_first.asset_hash(), imported_bytes.asset_hash());
        assert_eq!(imported_first.pcm_hash(), imported_bytes.pcm_hash());

        // ⑤ 夹具非空洞：这一路信号不是常数，因此上面的位模式相等不是"两者都为零"的退化。
        assert!(first.samples().iter().any(|sample| sample.abs() > 0.1));
        cleanup.unwrap();
    }

    /// 判据（⑥-8 类别⑥ 多声道一致性 / 声道数极值）：声明的声道数在**极值**上必须被类型化拒绝
    /// 而不是 panic；上游收下的最大声道数（26）此前没有任何判据走过；各声道声明值相同时
    /// 解出的交织帧里各声道**逐位**相同，且声道数与帧数不丢不增。
    ///
    /// 为什么需要它：既有的解码侧声道判据只钉两件事 —— `the_channel_and_rate_gates_fire_on_its_own`
    /// 把预算压到 1 声道再看立体声被拒（**上界** `max_channels` 的闭区间从未在解码侧被测过），
    /// `one_frame_and_non_power_of_two_streams_decode_exactly_and_zero_frames_are_refused`
    /// 只钉 2 声道的 L==R 交织对。以下三类此前是空白：
    ///
    /// 1. **声明的极值**：0 声道、32769 声道与 65535 声道。8-bit 的这两个极值是一个**与既有
    ///    溢出判据不同的形状** —— `num_channels × (bits_per_sample / 8)` 在 8-bit 下是
    ///    `n × 1`，`32769` 与 `65535` 都 **不**溢出 `u16`，因此 RIFF 预检对它们是沉默的，
    ///    拒绝必须来自上游的声道数闸门。既有判据用的是 16-bit 的 32769（那里预检会先跳闸），
    ///    因此覆盖不到这个形状。
    /// 2. **上界 26**：上游 `map_wave_channel_count` 接受 1..=26，27 起拒绝。26 是解码侧
    ///    **真实可达**的最大声道数。
    /// 3. **解码器的声道预算闸门在 WAV 上不可达**：`max_channels` 默认 64，而上游把 PCM/WAV
    ///    的声道数卡在 26，因此解码侧永远走不到 [`LimitViolation::TooManyChannels`]。
    ///    本条把这个事实**钉成断言**（极值拒绝不得是 `Budget` 变体），而不是留成一句注释。
    ///    能超过 26 声道的只有别的容器（Ogg Vorbis 的 `channels` 是 `u8`，可到 255），
    ///    而本 crate 没有 Ogg 的字节级夹具，因此那条路径**没有**判据覆盖，照实登记。
    ///
    /// 另两条**不可达**的守卫也在这里说清（不是本判据的落点）：解码循环里"声道数中途变了"
    /// 与"样本格式中途变了"两处 `InconsistentLayout`（现位于第 331 行与第 338 行）无法由
    /// 公开入口到达 —— WAV/FLAC 的声道数在一条流里恒定，而真正会中途换布局的串联 Ogg
    /// 先撞上 `ResetRequired`（本 crate 明确拒绝），因此那两处是防御性代码。
    ///
    /// 实测读数（本机）：0 声道 ⇒ `malformed stream: riff: invalid channel count`；
    /// 26 声道 ⇒ 正常解出；27 声道 ⇒ 同上那条 invalid channel count；
    /// 32769 / 65535 声道 @8-bit ⇒ 同上（**不是**预算变体，也不是预检的溢出文案）；
    /// 65535 声道 @16-bit ⇒ 预检的溢出文案（`u16` 乘法会溢出）。
    ///
    /// 注入（三条，逐条在本机跑过）：
    /// - 预检对 tag 1 的 `u16` 乘法溢出**不再拒绝**（条件改成恒假）⇒ 本条在上游
    ///   `read_pcm_fmt` 第 100 行的 `attempt to multiply with overflow` 处 panic，
    ///   只跑本条时读数是 `0 passed / 1 failed`（这证明 ⑤ 钉的拒绝是真的在挡 panic）；
    /// - 预检把 8-bit 的每样本字节数误算成 **2**（过度拒绝）⇒ 本条在 ④ 的第一轮以
    ///   `32769 channels at 8 bits: expected the upstream channel-count refusal, got
    ///   malformed stream: WAV fmt declares 32769 channels at 8 bits per sample: the RIFF
    ///   parser computes num_channels * (bits_per_sample / 8) in u16 and would overflow`
    ///   红，只跑本条时读数是 `0 passed / 1 failed`（这证明 ④ 钉的**落点**）；
    /// - `check_layout` 的声道闸门从闭区间改成 `>=` ⇒ 本条在 ⑥ 的"恰好等于上限"处红；
    ///   整库读数是 `108 passed / 6 failed`（另外 5 条是既有的声道/预算判据）。
    #[test]
    fn channel_count_extremes_are_refused_and_equal_channels_stay_bit_identical() {
        // ① 各声道声明值相同 ⇒ 解出的交织帧里各声道逐位相同；声道数与帧数不丢不增。
        let frames = 40usize;
        let values: Vec<i32> = (0..frames)
            .map(|index| (index as i32 % 101) * 100 - 5_000)
            .collect();
        for channels in [1u16, 2, 3, 4, 8, 26] {
            let mut interleaved = Vec::with_capacity(frames * usize::from(channels));
            for value in &values {
                for _ in 0..channels {
                    interleaved.push(*value);
                }
            }
            let asset = decode_bytes(
                &int_wav(channels, 16, &interleaved),
                &DecodeOptions::default(),
            )
            .unwrap_or_else(|err| panic!("{channels} channels must decode: {err}"));
            assert_eq!(
                asset.channels(),
                channels,
                "{channels} channels were not kept"
            );
            assert_eq!(asset.frame_count(), u64::try_from(frames).unwrap());
            assert_eq!(asset.samples().len(), frames * usize::from(channels));
            let mismatched = asset
                .samples()
                .chunks(usize::from(channels))
                .flat_map(|frame| frame[1..].iter().map(move |sample| (frame[0], *sample)))
                .filter(|(first, other)| first.to_bits() != other.to_bits())
                .count();
            assert_eq!(
                mismatched, 0,
                "{channels} channels carrying the same declared value must decode to the same bits"
            );
        }

        // ② 0 声道：类型化拒绝；同一份字节把声明改回 1 声道必须照常解出（证明红的是闸门）。
        let mut zero = int_wav(1, 16, &[100, -100]);
        patch_declared_channels(&mut zero, 0);
        let err = decode_bytes(&zero, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("invalid channel count")),
            "0 declared channels: expected the upstream channel-count refusal, got {err}"
        );
        patch_declared_channels(&mut zero, 1);
        assert_eq!(
            decode_bytes(&zero, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            2,
            "the same bytes with 1 declared channel must decode"
        );

        // ③ 27 声道：上游上界之外，必须与 26 声道形成闭区间的两侧。
        let err =
            decode_bytes(&int_wav(27, 16, &[1_000; 27]), &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("invalid channel count")),
            "27 channels: expected the upstream channel-count refusal, got {err}"
        );

        // ④ 32769 / 65535 声道 @8-bit：`num_channels × 1` 不溢出 `u16`，因此预检沉默 ——
        //    拒绝必须来自上游声道数闸门，而且**不能**是解码预算的声道变体
        //    （那证明解码侧 `max_channels` 在 WAV 上不可达）。
        for channels in [32_769u16, 65_535] {
            let widest = int_wav(channels, 8, &vec![0i32; usize::from(channels)]);
            let err = decode_bytes(&widest, &DecodeOptions::default()).unwrap_err();
            assert!(
                matches!(&err, DecodeError::Malformed { detail }
                    if detail.contains("invalid channel count")),
                "{channels} channels at 8 bits: expected the upstream channel-count refusal, \
                 got {err}"
            );
            assert!(
                !matches!(err, DecodeError::Budget(_)),
                "{channels} channels: the decoder-side channel budget must not be what \
                 refuses a WAV: {err}"
            );
        }

        // ⑤ 对照：同一处算术改成 16-bit ⇒ `num_channels × 2` 溢出 `u16`，预检先跳闸。
        //    两个极值由此分走**两条不同的拒绝路径**（预检 vs 上游声道数闸门）。
        let mut overflowing = int_wav(1, 16, &[0, 0]);
        patch_declared_channels(&mut overflowing, 65_535);
        let err = decode_bytes(&overflowing, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail }
                if detail.contains("65535") && detail.contains("would overflow")),
            "65535 channels at 16 bits: expected the precheck overflow refusal, got {err}"
        );

        // ⑥ 解码侧 `max_channels` 的**闭区间**：恰好等于上限必须通过，少一个必须被拒。
        let stereo = int_wav(2, 16, &[1_000; 64]);
        let at_cap = DecodeOptions {
            budget: PcmBudget {
                max_channels: 2,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert_eq!(
            decode_bytes(&stereo, &at_cap).unwrap().channels(),
            2,
            "exactly max_channels must pass the channel gate"
        );
        let below_cap = DecodeOptions {
            budget: PcmBudget {
                max_channels: 1,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert!(
            matches!(
                decode_bytes(&stereo, &below_cap),
                Err(DecodeError::Budget(LimitViolation::TooManyChannels {
                    channels: 2,
                    limit: 1
                }))
            ),
            "one below max_channels must be refused"
        );
    }

    // -----------------------------------------------------------------------
    // 本批新增判据的辅助：线上原始字节、压缩容器、第三族容器、前缀、不可回退源。
    // -----------------------------------------------------------------------

    /// 把 `fmt ` 的 `sampleRate`（块体 +4）与 `byteRate`（块体 +8）改写成给定的**线上**值。
    ///
    /// 存在理由：`testfix::WavSpec::byte_rate()` 用 `sample_rate * block_align` 算字节率，
    /// 采样率接近 `u32::MAX` 时那次乘法**故意**在夹具里溢出（夹具的约定是"越界立刻炸"，
    /// 见 `testfix::tests`）。因此"线上极大采样率"这类输入不能经 `WavSpec` 构造 ——
    /// 只能先生成一份合法头，再改这两个字段。
    fn patch_declared_rate(bytes: &mut [u8], rate: u32, byte_rate: u32) {
        let body = fmt_body_offset(bytes).expect("the fixture must contain a fmt chunk");
        bytes[body + 4..body + 8].copy_from_slice(&rate.to_le_bytes());
        bytes[body + 8..body + 12].copy_from_slice(&byte_rate.to_le_bytes());
    }

    /// FLAC 夹具里元数据区的末尾偏移：`fLaC`(4) + 元数据块头(4) + `STREAMINFO`(34)。
    ///
    /// 从这一个偏移到文件末尾就是**帧区**。
    const FLAC_METADATA_END: usize = 42;

    /// `STREAMINFO` 的 16 字节 MD5（音频指纹）在文件内的起点。
    ///
    /// `STREAMINFO` 从文件偏移 8 开始：`min/max blocksize`(4 字节) + `min/max framesize`
    /// (6 字节) + `sample_rate`(20 位) + `channels`(3 位) + `bits`(5 位) + `total_samples`
    /// (36 位) 恰好 18 字节，因此 MD5 是文件偏移 26…42。
    const FLAC_MD5_OFFSET: usize = 26;

    /// 固定块大小、单声道、16 位的 FLAC 夹具（直流 1 000，每帧 256 帧）。
    fn flac_fixture(frames: u16) -> Vec<u8> {
        flac_constant(
            &FlacSpec {
                sample_rate: 8_000,
                channels: 1,
                bits: 16,
                block_frames: 256,
                total_samples_override: None,
            },
            frames,
            1_000,
        )
    }

    /// 判据（类别④ 数值极值 / [ARCH-SEC-003]）：采样率闸门在**解码入口**读的是容器线上
    /// 声明的那个 `u32`，判定是闭区间，并且"大到 `u32::MAX`"的声明不会让任何一处算术回绕。
    ///
    /// 量什么：`decode_bytes` 对一串线上采样率取值的返回值，以及错误里回放的 `rate`/`limit`。
    /// 怎么量：先生成合法的 8 kHz 单声道 WAV，再用 [`patch_declared_rate`] 把 `fmt ` 的
    /// `sampleRate` 与 `byteRate` 改写成目标值。既有判据只在"素材 8 kHz 对预算 4 kHz"
    /// 这一格上量过这道闸门（`the_channel_and_rate_gates_fire_on_their_own`），而闸门的
    /// **上边界**、以及 `u32::MAX` 这个可达的畸形声明，在解码入口上没有判据。
    ///
    /// 读数（本机、debug 构建）：768 000 Hz（[`limits::DEFAULT_MAX_SAMPLE_RATE`]）解出
    /// 2 帧、`sample_rate()` 就是 768 000；768 001、1 000 000、`u32::MAX - 1`、`u32::MAX`
    /// 四个取值全部以 `SampleRateTooHigh { rate: <线上值>, limit: 768000 }` 拒绝 ——
    /// 报的是**线上那个数**，不是任何折算值。
    ///
    /// 注入：把 `check_layout` 里 `sample_rate > budget.max_sample_rate` 改成 `>=` ⇒
    /// 本条以"768 000 Hz 必须通过"红。
    #[test]
    fn the_sample_rate_gate_is_closed_at_the_documented_cap_on_the_wire() {
        let cap = limits::DEFAULT_MAX_SAMPLE_RATE;
        let values = [1_000i32, -1_000];

        let mut at_cap = int_wav(1, 16, &values);
        patch_declared_rate(&mut at_cap, cap, cap * 2);
        let asset = decode_bytes(&at_cap, &DecodeOptions::default())
            .expect("exactly the documented cap must pass the sample-rate gate");
        assert_eq!(asset.sample_rate(), cap);
        assert_eq!(asset.frame_count(), 2);

        for over in [cap + 1, 1_000_000, u32::MAX - 1, u32::MAX] {
            let mut bytes = int_wav(1, 16, &values);
            patch_declared_rate(&mut bytes, over, over.wrapping_mul(2));
            match decode_bytes(&bytes, &DecodeOptions::default()) {
                Err(DecodeError::Budget(LimitViolation::SampleRateTooHigh { rate, limit })) => {
                    assert_eq!(
                        (rate, limit),
                        (over, cap),
                        "the gate must replay the wire-declared rate verbatim"
                    );
                }
                other => panic!("{over} Hz must trip the sample-rate gate, got {other:?}"),
            }
        }
    }

    /// 判据（压缩容器的解压边界 / 任意字节不许 panic）：FLAC 的**帧区**是一段被校验和
    /// 保护的字节；改一个字节就必须被拒，而且任何改动都不许 panic。
    ///
    /// 量什么：`decode_bytes` 在"合法 FLAC 的帧区被改写一个字节"上的结果分布
    /// （被接受 / 被拒 / panic 三类）。panic 会让本判据直接失败，因此它是隐式计数。
    /// 怎么量：元数据区是前 [`FLAC_METADATA_END`] 字节（`fLaC` + 块头 + `STREAMINFO`），
    /// 其后到文件末尾是帧区。单帧夹具上逐偏移 × 全部 256 个取值（跳过原值）；三帧夹具上
    /// 逐偏移 × 五个代表值（`0x00`/`0x01`/`0x7F`/`0x80`/`0xFF`，同样跳过原值）。
    ///
    /// 读数（本机、debug 构建）：单帧夹具 12 个帧区偏移 × 255 个非本值 = **3 060** 次改动，
    /// 全部 `Err`、**0** 次被接受；三帧夹具 36 个偏移 × 5 个取值 = 180 次候选改动，其中
    /// 14 次与**原值**相同被跳过，其余 **166** 次全部 `Err`。
    /// 机制：上游按帧头 CRC-8 与帧 CRC-16 逐帧校验；校验不过的帧被 `decode_source` 的宽容
    /// 分支跳过，于是帧数少于声明、[`crate::duration::reconcile`] 的对账失败而整份拒绝。
    ///
    /// 同一份夹具的**元数据区**不是这样（见下一条判据：16 字节 MD5 的 4 080 次改动全部被
    /// 接受且摘要不变），因此本条的边界必须写成"帧区"，不能写成"整个容器"。
    ///
    /// 注入（实测）：把 `decode_source` 里 `if options.verify_declared_duration
    /// { outcome.into_result()?; }` 改成不生效 ⇒ 本条**变红**。机制是把"声明帧数 ↔ 解出
    /// 帧数"的对账拿掉之后，被上游 CRC 丢掉一帧的那份流会解出一份**帧数偏少**的资产而
    /// 变成 `Ok`。因此本条同时钉住两件事：上游逐帧校验的强度，以及本 crate 的对账闸门是
    /// 那种"少一帧也看不出来"的差异的唯一出口。
    #[test]
    fn a_one_byte_change_in_a_flac_frame_region_is_always_detected() {
        let single = flac_fixture(1);
        assert_eq!(
            single.len(),
            FLAC_METADATA_END + 12,
            "one FLAC frame is 12 bytes here"
        );
        let base = decode_bytes(&single, &DecodeOptions::default()).unwrap();
        assert_eq!(base.frame_count(), 256);
        assert_eq!(base.declared_frames(), Some(256));

        let mut cases = 0usize;
        let mut accepted = 0usize;
        for offset in FLAC_METADATA_END..single.len() {
            for value in 0u16..=255 {
                let value = u8::try_from(value).expect("a byte value fits u8");
                if single[offset] == value {
                    continue;
                }
                let mut mutated = single.clone();
                mutated[offset] = value;
                cases += 1;
                if decode_bytes(&mutated, &DecodeOptions::default()).is_ok() {
                    accepted += 1;
                }
            }
        }
        assert_eq!(cases, 12 * 255);
        assert_eq!(cases, 3_060);
        assert_eq!(
            accepted, 0,
            "no single-byte change to a FLAC frame region may decode as audio"
        );

        let triple = flac_fixture(3);
        assert_eq!(
            triple.len(),
            FLAC_METADATA_END + 36,
            "three FLAC frames are 36 bytes"
        );
        assert_eq!(
            decode_bytes(&triple, &DecodeOptions::default())
                .unwrap()
                .frame_count(),
            768
        );
        let mut cases = 0usize;
        let mut accepted = 0usize;
        for offset in FLAC_METADATA_END..triple.len() {
            for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFF] {
                if triple[offset] == value {
                    continue;
                }
                let mut mutated = triple.clone();
                mutated[offset] = value;
                cases += 1;
                if decode_bytes(&mutated, &DecodeOptions::default()).is_ok() {
                    accepted += 1;
                }
            }
        }
        // 逐个取值里与原值相同的那些被跳过。本夹具实测 14 个：每个帧的同步码 `0xFF`、
        // 帧头里的 `0x00` 与子帧类型 `0x00`（各 2 个/帧，共 12 个），加上帧 0 与帧 1 的
        // 帧号字节恰好是 `0x00` 与 `0x01`（2 个）。
        let skipped = (FLAC_METADATA_END..triple.len())
            .filter(|&offset| matches!(triple[offset], 0x00 | 0x01 | 0x7F | 0x80 | 0xFF))
            .count();
        assert_eq!(skipped, 14);
        assert_eq!(cases, 36 * 5 - skipped);
        assert_eq!(cases, 166);
        assert_eq!(accepted, 0);
    }

    /// 判据（压缩容器的元数据边界）：`STREAMINFO` 的 16 字节 MD5（音频指纹）由容器
    /// **声明**，但解码路径不校验它 —— 拒绝决策只依赖帧 CRC 与"声明帧数 ↔ 解出帧数"的对账。
    ///
    /// 量什么：把 [`FLAC_MD5_OFFSET`] 起的 16 个字节逐个改成其余 255 个取值之后，
    /// `decode_bytes` 的返回类别，以及与未改动输入的 `pcm_hash` 是否逐位相同。
    /// 怎么量：三帧 FLAC 夹具；MD5 区间与帧区的分界由 [`FLAC_MD5_OFFSET`] 与
    /// [`FLAC_METADATA_END`] 两个常量写死。
    ///
    /// 读数（本机、debug 构建）：16 × 255 = **4 080** 次改动**全部** `Ok`，并且全部与
    /// 未改动输入的 `pcm_hash` **逐位相同**；同一份夹具的帧区改动 166 次全部 `Err`
    /// （上一条判据）。两个极端的对照就是"这条容器级保证的边界在哪"。
    ///
    /// 把这条边界写下来，是为了让下面两种变化各自有一条判据可以红：升级后 MD5 开始被校验
    /// （4 080 次 `Ok` 变成 `Err`），或者声明帧数被忽略（`Ok` 的摘要改变）。
    ///
    /// 这不是缺陷登记：把 MD5 拉进拒绝决策会改变错误分类（同一份音频会因为"指纹不符"
    /// 而不是"帧数不符"被拒），那需要一次裁决。本 crate 的对账口径写在
    /// [`crate::duration::reconcile`] 上。
    #[test]
    fn the_flac_streaminfo_fingerprint_is_declared_but_never_verified() {
        let bytes = flac_fixture(3);
        let base = decode_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(base.frame_count(), 768);
        let base_hash = base.pcm_hash();

        let mut cases = 0usize;
        let mut same_hash = 0usize;
        let mut other = 0usize;
        for offset in FLAC_MD5_OFFSET..FLAC_METADATA_END {
            for value in 0u16..=255 {
                let value = u8::try_from(value).expect("a byte value fits u8");
                if bytes[offset] == value {
                    continue;
                }
                let mut mutated = bytes.clone();
                mutated[offset] = value;
                cases += 1;
                match decode_bytes(&mutated, &DecodeOptions::default()) {
                    Ok(asset) if asset.pcm_hash() == base_hash => same_hash += 1,
                    _ => other += 1,
                }
            }
        }
        assert_eq!(cases, 16 * 255);
        assert_eq!(cases, 4_080);
        assert_eq!(
            same_hash, cases,
            "the declared MD5 must not take part in the decode decision"
        );
        assert_eq!(other, 0);
    }

    /// 判据（任意字节不许 panic / 第三族容器）：本 crate 启用了 Ogg 容器与 Vorbis 解码器，
    /// 但此前没有任何 Ogg 的字节级判据（`channel_count_extremes_are_refused_...` 的文档
    /// 自己登记了这条空白）。
    ///
    /// 量什么：两份 Ogg 形状的字节串，以及它们"改一个字节 × 五个代表值"的邻居，
    /// `decode_bytes` 的结果分布（被拒 / 被接受 / panic）；panic 会让本判据直接失败。
    /// 怎么量：第一份按 Ogg 页结构手工拼 —— 27 字节页头（`OggS` + 版本 + 头类型 +
    /// 粒度位置 + 序列号 + 页序号 + CRC + 段数）+ 1 字节段表 + 30 字节体（一个 Vorbis
    /// 标识包的开头），共 58 字节；第二份是 `OggS` 后跟 60 个 `0xFF`。两者各自逐偏移 ×
    /// 五个代表值地改一个字节。
    ///
    /// 读数（本机、debug 构建）：两份种子本身都以带 `ogg` 标记的 `Malformed` 拒绝
    /// （本机实测文案分别是 `ogg: crc mismatch` 与 `ogg: invalid ogg version`）；
    /// 全部邻居改动共 **503** 次（58 与 64 个偏移 × 5 个取值 = 610 次候选，减去与原值相同的
    /// 107 次），**0** 次被接受、0 次 panic。
    ///
    /// 覆盖边界（如实登记）：本条只覆盖"Ogg 形状的字节不许 panic"，**不**覆盖"串联 Ogg
    /// 中途换声道布局"那条 [ARCH-DET-001] 路径 —— 走到它需要一份**合法**的多物理流 Ogg，
    /// 本 crate 没有构造它的夹具。
    #[test]
    fn ogg_shaped_bytes_and_their_neighbours_are_errors_not_panics() {
        let mut page = Vec::new();
        page.extend_from_slice(b"OggS");
        page.push(0); // stream_structure_version
        page.push(0); // header_type
        page.extend_from_slice(&[0u8; 8]); // granule position
        page.extend_from_slice(&[0u8; 4]); // bitstream serial number
        page.extend_from_slice(&[0u8; 4]); // page sequence number
        page.extend_from_slice(&[0u8; 4]); // CRC
        page.push(1); // page_segments
        page.push(30); // segment table: one 30-byte packet
        page.extend_from_slice(b"\x01vorbis");
        page.extend_from_slice(&[0u8; 23]);
        assert_eq!(page.len(), 58);

        let mut flooded = Vec::from(*b"OggS");
        flooded.extend_from_slice(&[0xFFu8; 60]);
        assert_eq!(flooded.len(), 64);

        for (label, seed) in [("page", &page), ("flood", &flooded)] {
            let err = decode_bytes(seed, &DecodeOptions::default()).unwrap_err();
            assert!(
                err.to_string().contains("ogg"),
                "{label}: an Ogg-shaped seed must be refused by the Ogg reader, got {err}"
            );
        }

        let mut cases = 0usize;
        let mut accepted = 0usize;
        for seed in [&page, &flooded] {
            for offset in 0..seed.len() {
                for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFF] {
                    if seed[offset] == value {
                        continue;
                    }
                    let mut mutated = seed.clone();
                    mutated[offset] = value;
                    cases += 1;
                    if decode_bytes(&mutated, &DecodeOptions::default()).is_ok() {
                        accepted += 1;
                    }
                }
            }
        }
        let skipped: usize = [&page, &flooded]
            .iter()
            .map(|seed| {
                seed.iter()
                    .filter(|byte| matches!(byte, 0x00 | 0x01 | 0x7F | 0x80 | 0xFF))
                    .count()
            })
            .sum();
        assert_eq!(skipped, 107);
        assert_eq!(cases, (page.len() + flooded.len()) * 5 - skipped);
        assert_eq!(cases, 503);
        assert_eq!(accepted, 0);
    }

    /// 判据（长度闸门 / 任意字节不许 panic）：一份能解出的容器的**每一个真前缀**都必须被拒，
    /// 只有整份才允许解出。
    ///
    /// 量什么：对每个字节长度 `cut ∈ [0, len)` 调用 `decode_bytes(&bytes[..cut])` 的结果
    /// 类别；以及整份输入解出的帧数。panic 会让本判据直接失败。
    /// 怎么量：三份夹具 —— WAV（44 字节头 + 4 个 16-bit 样本 = 52 字节）、单帧 FLAC
    /// （54 字节）、三帧 FLAC（78 字节）。
    ///
    /// 读数（本机、debug 构建）：52 + 54 + 78 = **184** 个真前缀全部 `Err`、0 次 panic；
    /// 三份整份输入分别解出 4 / 256 / 768 帧。
    /// 既有判据 `truncated_wav_is_an_error_not_a_panic` 只取 7 个固定的 WAV 截断点，
    /// 且不覆盖 FLAC；"每一个前缀"是全量读法，因此它能抓到"某个中间长度恰好解成一份短
    /// 资产"这类只在特定偏移暴露的缺陷。
    ///
    /// 注入（实测）：让 `decode_source` 里 `if options.verify_declared_duration
    /// { outcome.into_result()?; }` 不生效 ⇒ 两个 FLAC 夹具在**整帧边界**上的前缀会解出
    /// 比声明少的帧而变成 `Ok`，本条以
    /// `one-frame FLAC: the 54-byte prefix of a 54-byte container must not decode` 一类文案红。
    #[test]
    fn every_proper_prefix_of_a_decodable_container_is_refused() {
        let ogg = crate::testfix::ogg_vorbis_silence(2);
        let ogg_len = ogg.len();
        let cases: [(&str, Vec<u8>, u64); 4] = [
            ("WAV", int_wav(1, 16, &[1_000, -1_000, 2_000, -2_000]), 4),
            ("one-frame FLAC", flac_fixture(1), 256),
            ("three-frame FLAC", flac_fixture(3), 768),
            ("Ogg Vorbis", ogg, crate::testfix::OGG_FIXTURE_FRAMES),
        ];
        let mut refused = 0usize;
        for (label, bytes, frames) in cases {
            for cut in 0..bytes.len() {
                let outcome = decode_bytes(&bytes[..cut], &DecodeOptions::default());
                assert!(
                    outcome.is_err(),
                    "{label}: the {cut}-byte prefix of a {}-byte container must not decode, \
                     got {outcome:?}",
                    bytes.len()
                );
                refused += 1;
            }
            let full = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_or_else(|err| {
                panic!("{label}: the whole container must decode, got {err}")
            });
            assert_eq!(full.frame_count(), frames, "{label}: frame count");
        }
        assert_eq!(refused, 52 + 54 + 78 + ogg_len);
        assert_eq!(refused, 184 + ogg_len);
        assert_eq!(ogg_len, 251, "the Ogg fixture's byte length is pinned");
    }

    /// 一个**不可回退**的源：前 `interrupts` 次 `read` 返回
    /// [`std::io::ErrorKind::Interrupted`]，之后转为正常读取；`seek` 一律失败。
    ///
    /// 存在理由：[`slurp_unseekable`] 有一条 `Err(err) if err.kind() == Interrupted => {}`
    /// 的重试分支，此前**零判据**。真实来源是带 `EINTR` 的读取路径（std 对 `File` 自己会
    /// 重试，但 `MediaSource` 是 trait 对象，调用方的实现可以把 `EINTR` 原样交上来）。
    struct InterruptingSource {
        inner: Cursor<Vec<u8>>,
        interrupts: u32,
    }

    impl InterruptingSource {
        fn new(bytes: Vec<u8>, interrupts: u32) -> Self {
            Self {
                inner: Cursor::new(bytes),
                interrupts,
            }
        }
    }

    impl Read for InterruptingSource {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.interrupts > 0 {
                self.interrupts -= 1;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "interrupted by a signal",
                ));
            }
            self.inner.read(buf)
        }
    }

    impl Seek for InterruptingSource {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this source cannot seek",
            ))
        }
    }

    impl MediaSource for InterruptingSource {
        fn is_seekable(&self) -> bool {
            false
        }

        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// 判据（闸门可达性 / 不可信输入零 panic）：`slurp_unseekable` 的 `Interrupted`
    /// 重试分支**可达**，而且重试之后解出的资产与内存入口**逐位相同**。
    ///
    /// 量什么：一个"前 N 次 `read` 报 `Interrupted`、之后正常"的不可回退源经
    /// `decode_source` 的返回值、帧数与 `pcm_hash`。
    /// 怎么量：N ∈ {1, 3, 64}，输入是一份合法的 2 声道 16-bit WAV。
    ///
    /// 读数（本机、debug 构建）：三档 N 全部解出，帧数与 `pcm_hash` 与 `decode_bytes` 的
    /// 结果逐位相同。
    ///
    /// 注入（实测）：把 `slurp_unseekable` 的
    /// `Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}` 那一行删掉 ⇒
    /// 本条以 `the Interrupted arm must be retried, got io error: interrupted by a signal` 红。
    ///
    /// 覆盖边界（如实登记，待裁决）：本条只证明这个重试分支**存在且可达**，不证明它有界 ——
    /// 一个永远报 `Interrupted` 的源会让这个循环不返回。要判"有界"必须在判据里放一个墙钟
    /// 超时（违反"判据不得依赖墙钟速度"），或者给这个循环加一个重试上限（产线改动，不在
    /// 本批判据的范围）。因此这一格登记为待裁决，不写成"已覆盖"。
    #[test]
    fn an_unseekable_source_that_reports_interrupted_is_retried_not_failed() {
        let legal = int_wav(2, 16, &[1, -2, 3, -4, 5, -6, 7, -8]);
        let direct = decode_bytes(&legal, &DecodeOptions::default()).unwrap();
        for interrupts in [1u32, 3, 64] {
            let asset = decode_source(
                Box::new(InterruptingSource::new(legal.clone(), interrupts)),
                &Hint::new(),
                &DecodeOptions::default(),
            )
            .unwrap_or_else(|err| {
                panic!(
                    "the Interrupted arm must be retried, got {err} after {interrupts} interrupts"
                )
            });
            assert_eq!(asset.frame_count(), direct.frame_count());
            assert_eq!(asset.pcm_hash(), direct.pcm_hash());
        }
    }

    /// 判据（预检的可回退性）：[`precheck_riff_wave_fmt`] 必须在返回前把游标还给调用方给
    /// 它的那个位置 —— 它先读 12 字节 RIFF 头、再扫块网格，最后必须原样还回去。
    ///
    /// 量什么：`Cursor<&[u8]>::position()` 在预检前后是否相同（单位：字节偏移）。
    /// 怎么量：直接调用私有的 [`precheck_riff_wave_fmt`]（本模块的判据可以访问它），
    /// 输入取四类 —— 带 `fmt ` 的合法 WAV、`fmt ` 前面有 `JUNK` 块的布局、不是 RIFF 的
    /// 垃圾、空输入；四格都把起点设在 **3**，用来钉住"还原到 `start`"而不是"还原到 0"。
    ///
    /// 读数（本机、debug 构建）：四格全部 `position() == 3`。
    ///
    /// 为什么需要它：本批注入实测把 `precheck_riff_wave_fmt` 末尾的
    /// `source.seek(SeekFrom::Start(start))?;` 整行删掉之后，**全部 160 条判据照旧通过**
    /// （上游的 `probe` 自己会 seek 回起点，因此从解码结果上看不出差别）。也就是说这条
    /// "位置复原"契约此前没有判据。
    ///
    /// 注入（实测）：删掉那一行 ⇒ 本条以
    /// `a WAV with a fmt chunk: the precheck must leave the cursor where it found it` 红。
    #[test]
    fn the_riff_precheck_restores_the_source_position() {
        let junk = wav_with_chunks_before_fmt(
            &int_spec(1, 16),
            &[(*b"JUNK", &[0xEEu8; 4])],
            &encode_int_samples(16, &[0x1234, -0x1234]),
        );
        let cases: [(&str, Vec<u8>); 4] = [
            ("a WAV with a fmt chunk", int_wav(1, 16, &[0x1234, -0x1234])),
            ("a WAV whose fmt sits behind a JUNK chunk", junk),
            (
                "bytes that are not RIFF at all",
                b"not a riff file at all".to_vec(),
            ),
            ("an empty input", Vec::new()),
        ];
        for (label, bytes) in cases {
            // 起点不是 0：预检必须还原到 `start`，不是还原到 0。
            let mut cursor = Cursor::new(bytes);
            cursor.set_position(3);
            precheck_riff_wave_fmt(&mut cursor).expect("the precheck must return Ok here");
            assert_eq!(
                cursor.position(),
                3,
                "{label}: the precheck must leave the cursor where it found it"
            );
        }
    }

    /// 判据（块扫描预算 / 流式 WAV）：`riff_len == u32::MAX`（上游语义是"长度未知"）时父块
    /// **没有**上界，因此块扫描的唯一上界是 [`RIFF_PRECHECK_MAX_CHUNKS`] —— 超过就必须保守
    /// 拒绝。
    ///
    /// 量什么：把 RIFF 长度字段改写成 `u32::MAX` 之后，`fmt ` 之前的零长度块数 `N` 与
    /// [`RIFF_PRECHECK_MAX_CHUNKS`] 相等 / 少一时的 `decode_bytes` 结果。
    /// 怎么量：`wav_with_chunks_before_fmt` 生成合法的 `fmt ` + `data`，前面插 `N` 个零长度
    /// `JUNK` 块，再把偏移 4…8 的 RIFF 长度改成 `u32::MAX`。
    ///
    /// 读数（本机、debug 构建）：N = 4 096 时以 `Malformed`（文案含 `before its fmt chunk`）
    /// 拒绝；N = 4 095 时解出 2 帧。既有判据
    /// `a_wave_with_more_chunks_than_the_scan_budget_is_refused_not_probed` 用的是**有限**的
    /// RIFF 长度，因此"父块无上界"这一格此前没有判据 —— 而它正是扫描预算唯一还在生效的
    /// 一格：有限长度下 `consumed >= limit` 会先结束扫描。
    ///
    /// 注入（实测）：把 `RIFF_PRECHECK_MAX_CHUNKS` 由 4 096 改成 8 192 ⇒ N = 4 096 的那一格
    /// 会被扫到 `fmt ` 并放行给上游，上游能正常解析这份文件，本条以
    /// `the scan budget must fail closed on a streaming WAV` 红。
    #[test]
    fn a_streaming_wav_obeys_the_scan_budget_because_its_parent_has_no_limit() {
        let spec = int_spec(1, 16);
        let data = encode_int_samples(16, &[0x1234, -0x1234]);
        let zero: &[u8] = &[];
        // 常量本身必须被钉住：只比较"常量与常量减一"的话，判据会跟着常量一起挪，
        // 改常量**不会**变红（本批注入实测：4 096 -> 8 192 时那一对比较全绿）。
        assert_eq!(RIFF_PRECHECK_MAX_CHUNKS, 4_096);
        let cap = RIFF_PRECHECK_MAX_CHUNKS as usize;
        assert_eq!(cap, 4_096);

        let at_cap = vec![(*b"JUNK", zero); 4_096];
        let mut bytes = wav_with_chunks_before_fmt(&spec, &at_cap, &data);
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = decode_bytes(&bytes, &DecodeOptions::default()).unwrap_err();
        assert!(
            matches!(&err, DecodeError::Malformed { detail } if detail.contains("before its fmt chunk")),
            "the scan budget must fail closed on a streaming WAV, got {err}"
        );

        // 预算减一 ⇒ 仍然看得见 `fmt ` 并照常解出（证明上面红的是预算，不是夹具坏了）。
        let under_cap = vec![(*b"JUNK", zero); 4_095];
        let mut bytes = wav_with_chunks_before_fmt(&spec, &under_cap, &data);
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode_bytes(&bytes, &DecodeOptions::default())
                .expect("one chunk below the scan budget is a legal streaming WAV")
                .frame_count(),
            2
        );
    }

    /// 一个**间歇性**中断的不可回退源：每次成功读取之前先报 `interrupts_per_read` 次
    /// `Interrupted`；`seek` 一律失败。
    ///
    /// 存在理由：[`unseekable_retry_allowance`] 量的是**连续**无进展次数。一份长素材中间
    /// 偶发 `EINTR` 不该被拒 —— 只有"连续"才计入。本类型让"总中断次数远超上界、但连续
    /// 次数始终在上界内"变成可构造的输入（判据
    /// `consecutive_interrupts_are_bounded_by_the_input_byte_budget` 的最后一段）。
    struct AlternatingInterruptSource {
        inner: Cursor<Vec<u8>>,
        interrupts_per_read: u32,
        pending: u32,
    }

    impl AlternatingInterruptSource {
        fn new(bytes: Vec<u8>, interrupts_per_read: u32) -> Self {
            Self {
                inner: Cursor::new(bytes),
                interrupts_per_read,
                pending: interrupts_per_read,
            }
        }
    }

    impl Read for AlternatingInterruptSource {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pending > 0 {
                self.pending -= 1;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "interrupted by a signal",
                ));
            }
            let filled = self.inner.read(buf)?;
            self.pending = self.interrupts_per_read;
            Ok(filled)
        }
    }

    impl Seek for AlternatingInterruptSource {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this source cannot seek",
            ))
        }
    }

    impl MediaSource for AlternatingInterruptSource {
        fn is_seekable(&self) -> bool {
            false
        }

        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// 判据（`MUST-GATE-011` 不可信输入不许挂死 / 裁决 `R33`）：`slurp_unseekable` 对
    /// **连续无进展**的读取有**可数的上界**，上界由调用方的输入字节预算推导。
    ///
    /// 量什么：一个"前 N 次 `read` 报 `Interrupted`、之后交出数据"的不可回退源经
    /// `decode_source` 的返回值；以及 [`unseekable_retry_allowance`] 在几档预算上的读数。
    /// 怎么量：预算固定为 4 个读取分块（`4 × SLURP_CHUNK_BYTES` 字节）⇒ 上界是 5。
    /// N = 5 必须解出，N = 6 必须返回**源自己的** I/O 错误。
    /// 判据**不读时钟**：它数的是重试次数，不是耗时（[ARCH-DET-001] 的判据纪律）。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | `max_input_bytes` | `unseekable_retry_allowance`（次） |
    /// | :--- | ---: |
    /// | 0 | 1 |
    /// | `1 × SLURP_CHUNK_BYTES` | 2 |
    /// | `4 × SLURP_CHUNK_BYTES` | 5 |
    /// | `u64::MAX` | `u64::MAX / SLURP_CHUNK_BYTES + 1` |
    ///
    /// 端到端：N = 5 解出 2 帧且 `pcm_hash` 与内存入口逐位相同；N = 6 以 `Err(Io(..))`
    /// 返回，`kind()` 就是 `Interrupted` —— 也就是说改建前的"永不返回"变成了明确错误。
    ///
    /// 最后一段钉住"**连续**"这个口径：每次成功读取前中断 3 次（总量 6 > 上界 5、
    /// 连续量 3 ≤ 5）必须照常解出。
    ///
    /// 注入（实测）：把 `if retries > allowance` 改成 `if false`（等于去掉上界）⇒
    /// 本条以 `the retry cap must refuse the 6th consecutive interrupt` 红。
    #[test]
    fn consecutive_interrupts_are_bounded_by_the_input_byte_budget() {
        let chunk = SLURP_CHUNK_BYTES as u64;
        // 上界是从预算推导的公式，不是魔数：逐档复算并给出闭区间的一侧。
        assert_eq!(unseekable_retry_allowance(0), 1);
        assert_eq!(unseekable_retry_allowance(chunk), 2);
        assert_eq!(unseekable_retry_allowance(4 * chunk), 5);
        assert_eq!(unseekable_retry_allowance(u64::MAX), u64::MAX / chunk + 1);
        // 保底那一次：预算 0 也必须先容忍一次合法的 `EINTR`。
        assert_eq!(unseekable_retry_allowance(0), unseekable_retry_allowance(1));

        let budget = PcmBudget {
            max_input_bytes: 4 * chunk,
            ..PcmBudget::default()
        };
        let options = DecodeOptions {
            budget,
            ..DecodeOptions::default()
        };
        let allowance = unseekable_retry_allowance(budget.max_input_bytes);
        assert_eq!(allowance, 5);

        let legal = int_wav(2, 16, &[1, -2, 3, -4, 5, -6, 7, -8]);
        let direct = decode_bytes(&legal, &options).unwrap();

        // 恰好用尽上界：5 次 `Interrupted` 之后仍然解出，且样本与内存入口逐位相同。
        let buffered = decode_source(
            Box::new(InterruptingSource::new(
                legal.clone(),
                u32::try_from(allowance).unwrap(),
            )),
            &Hint::new(),
            &options,
        )
        .expect("the allowance itself must be retried");
        assert_eq!(buffered.frame_count(), direct.frame_count());
        assert_eq!(buffered.pcm_hash(), direct.pcm_hash());

        // 上界加一：第 6 次 `Interrupted` 必须返回源自己的 I/O 错误，而不是继续转下去。
        match decode_source(
            Box::new(InterruptingSource::new(
                legal.clone(),
                u32::try_from(allowance + 1).unwrap(),
            )),
            &Hint::new(),
            &options,
        ) {
            Err(DecodeError::Io(err)) => assert_eq!(
                err.kind(),
                std::io::ErrorKind::Interrupted,
                "the cap must surface the source's own error"
            ),
            other => {
                panic!("the retry cap must refuse the 6th consecutive interrupt, got {other:?}")
            }
        }

        // "连续"才是被上界计的量：每次成功读取前中断 3 次（总量 6 > 上界 5、连续量 3 ≤ 5）
        // 必须照常解出。把 `retries` 的归零去掉，这一段就会以同一个错误红。
        let alternating = decode_source(
            Box::new(AlternatingInterruptSource::new(legal.clone(), 3)),
            &Hint::new(),
            &options,
        )
        .expect("intermittent interrupts must not accumulate: the cap counts CONSECUTIVE ones");
        assert_eq!(alternating.pcm_hash(), direct.pcm_hash());
    }

    /// 判据（`MUST-GATE-011` "解码不推进"闸门的**端到端**判据）：连续 **1025** 个
    /// "解封装器照常产出、解码器逐个失败"的包必须让解码终止并返回 [`DecodeError::Malformed`]，
    /// 而恰好 **1024** 个必须走完流并以 [`DecodeError::EmptyStream`] 返回。
    ///
    /// 量什么：`decode_bytes` 对"每个帧的 CRC-16 都被破坏的 FLAC"的返回**变体**，
    /// 帧数取 1024 与 1025（单位：帧 = 包）。
    /// 怎么量：用夹具的 [`flac_constant_with_broken_frame_crc`] —— 帧**头**的 CRC-8 保持
    /// 正确，因此解封装器仍然逐个定位帧并逐帧产出包；解码器每次都在帧 CRC-16 上失败 ⇒
    /// 走 `decode_source` 循环里 `Err(SymphoniaError::DecodeError(_))` 那条容忍分支 ⇒
    /// 每一轮都过一次 `bump_idle`。
    ///
    /// 读数（本机、debug 构建）：1025 帧 ⇒ `Malformed`，文案含
    /// `stopped making progress: 1024 consecutive packets produced no audio`；
    /// 1024 帧 ⇒ `EmptyStream`。也就是说这道闸门是**闭区间**：恰好 1024 次不推进不算越界，
    /// 第 1025 次才报错 —— 与 [`limits::IdleGuard::bump`] 的 `idle > MAX_IDLE_PACKETS` 同值。
    ///
    /// 为什么此前没有它：[`limits::IdleGuard`] 的文档自己登记了"本闸门**没有**端到端判据"，
    /// 理由是 WAV"声明长度远超真实字节"那条路径实测不复现（上游报 `UnexpectedEof`），
    /// 而 FLAC 夹具的帧号是**单字节 UTF-8**（`frames < 128`）⇒ 最多 127 个包，够不到 1024。
    /// 本批把夹具的帧号改成 FLAC 规范的 UTF-8 编码数（0…127 一字节、128…2047 两字节），
    /// 这条空白因此被填上；夹具自检见
    /// `testfix::tests::flac_frame_numbers_use_the_spec_utf8_encoding`。
    ///
    /// 注入（实测）：把 `decode_source` 里
    /// `Err(SymphoniaError::DecodeError(_)) => { bump_idle(&mut idle_packets)?; continue; }`
    /// 的 `bump_idle` 去掉 ⇒ 1025 帧那一格变成 `EmptyStream`，本条红；
    /// 把 `IdleGuard::bump` 的 `>` 改成 `>=` ⇒ 1024 帧那一格变成 `Malformed`，本条红。
    #[test]
    fn the_idle_guard_trips_on_1025_consecutive_bad_packets() {
        let cap = limits::MAX_IDLE_PACKETS;
        assert_eq!(cap, 1_024, "the gate's value is pinned as a literal");
        let spec = FlacSpec {
            sample_rate: 8_000,
            channels: 1,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        };

        // 对照：同一批字节在 CRC 不被破坏时照常解出（证明下面红的是 CRC，不是夹具坏了）。
        let legal = flac_constant(&spec, 8, 0);
        assert_eq!(
            decode_bytes(&legal, &DecodeOptions::default())
                .expect("an intact FLAC must decode")
                .frame_count(),
            8 * 256
        );

        // 反向对照：只破坏帧尾 CRC-16 **走不到解码器** —— symphonia 的 FLAC 读端自己会
        // 校验帧尾 CRC，不符时跳过整个帧、一个包都不产出。因此 1025 帧的结果是 EmptyStream
        // （`bump_idle` 一次都没跑），而不是"1025 次不推进"。这一段把"为什么必须用保留子帧
        // 类型"这件事钉住：没有它，下一条断言会被误读成"破坏 CRC 也行"。
        let crc_broken = flac_constant_with_broken_frame_crc(&spec, 1_025, 0);
        match decode_bytes(&crc_broken, &DecodeOptions::default()) {
            Err(DecodeError::EmptyStream) => {}
            other => panic!(
                "a CRC-broken FLAC must be dropped by the READER (no packet ever reaches the \
                 decoder), got {other:?}"
            ),
        }
        // 同一形状只坏一帧时读端会跳过它、其余帧照常解出（证明上面红的确实是"读端跳过"，
        // 不是"整份文件坏了"）。
        // 破坏面用夹具交出来的帧跨度定位，不重算帧长。
        let (mut one_bad, spans) = crate::testfix::flac_constant_with_frame_spans(&spec, 4, 0);
        let (start, len) = spans[2];
        let crc_at = start + len - 2;
        let original = u16::from_be_bytes([one_bad[crc_at], one_bad[crc_at + 1]]);
        one_bad[crc_at..crc_at + 2].copy_from_slice(&(!original).to_be_bytes());
        match decode_bytes(&one_bad, &DecodeOptions::default()) {
            Err(DecodeError::DurationMismatch(_)) => {}
            other => panic!(
                "one dropped frame must surface as a declared/decoded mismatch, got {other:?}"
            ),
        }

        // 恰好用尽：1024 个"读端收下、解码器拒收"的包走完流，一个样本都没解出来 ⇒ EmptyStream。
        let at_cap = flac_constant_with_reserved_subframes(&spec, u16::try_from(cap).unwrap(), 0);
        match decode_bytes(&at_cap, &DecodeOptions::default()) {
            Err(DecodeError::EmptyStream) => {}
            other => panic!(
                "exactly {cap} consecutive bad packets are inside the closed interval and must \
                 end at EmptyStream, got {other:?}"
            ),
        }

        // 多一个：第 1025 次"不推进"必须让循环停下来并报 Malformed。
        let over_cap =
            flac_constant_with_reserved_subframes(&spec, u16::try_from(cap + 1).unwrap(), 0);
        match decode_bytes(&over_cap, &DecodeOptions::default()) {
            Err(DecodeError::Malformed { detail }) => assert!(
                detail.contains("stopped making progress") && detail.contains("1024"),
                "the idle gate must name itself and its cap, got {detail}"
            ),
            other => panic!(
                "{} consecutive bad packets must trip the idle guard, got {other:?}",
                cap + 1
            ),
        }

        // 更多包不会改变结论：循环在第 1025 次就停手，不是走到流末尾才发现。
        let way_over = flac_constant_with_reserved_subframes(&spec, 3_000, 0);
        assert!(matches!(
            decode_bytes(&way_over, &DecodeOptions::default()),
            Err(DecodeError::Malformed { .. })
        ));
    }

    /// 判据（公开面的映射表）：`PcmFormat::from_buffer` 的每一个**可达**分支都随线上位深
    /// 与浮点标签走，且 `PcmFormat::bit_depth` / `PcmFormat::is_float` 与之一致。
    ///
    /// 量什么：`decode_bytes` 对六种 WAV 声明（8/16/24/32 位整数、32/64 位浮点）与一种
    /// FLAC 的 `pcm_format()`、`pcm_format().bit_depth()`、`is_float()`。
    /// 怎么量：每格用 `testfix` 的构造器生成线上字节，再读解出资产的事实。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 容器 | 线上声明 | `pcm_format()` | 位深 | `is_float()` |
    /// | :--- | :--- | :--- | ---: | :--- |
    /// | WAV tag 1 | 8 位 | `U8` | 8 | false |
    /// | WAV tag 1 | 16 位 | `S16` | 16 | false |
    /// | WAV tag 1 | 24 位 | `S24` | 24 | false |
    /// | WAV tag 1 | 32 位 | `S32` | 32 | false |
    /// | WAV tag 3 | 32 位 | `F32` | 32 | true |
    /// | WAV tag 3 | 64 位 | `F64` | 64 | true |
    /// | FLAC | 16 位 | `S32` | 32 | false |
    ///
    /// 覆盖边界（如实登记）：本 crate 启用的编解码器只产出上表这六种。`U16` / `U24` /
    /// `U32` / `S8` 四个变体**没有**可达的线上声明（symphonia 的 PCM 解码器对整数只给
    /// `U8` / `S16` / `S24` / `S32`，FLAC 只给 `S32`），因此那四个分支的 `bit_depth()`
    /// 由纯函数判据覆盖，不由本条覆盖。
    ///
    /// 为什么需要它：本批的**公开面普查**显示 `PcmFormat::from_buffer` 是全部 60 个公开
    /// `fn` 里**唯一**一个在全部判据文本里零引用、且只有一个产线调用点的。它的效果此前
    /// 只被零散的 `pcm_format()` 断言间接覆盖（只有 `S16` 与 `F32` 断言过），映射表本身
    /// 没有判据。
    ///
    /// 注入（实测）：把 `B::F64(_) => Self::F64` 改成 `Self::F32` ⇒ 本条在 64 位浮点那
    /// 一行红；把 `B::U8(_) => Self::U8` 改成 `Self::S8` ⇒ 本条在 8 位那一行红。
    #[test]
    fn the_decoded_pcm_format_follows_the_wire_bit_depth_and_the_float_tag() {
        // 64 位浮点的线上体不能用 `encode_f32_samples`，这里按 `f64` 小端排。
        let mut f64_body = Vec::new();
        for sample in [0.5f64, -0.5, 0.25, -0.25] {
            f64_body.extend_from_slice(&sample.to_le_bytes());
        }
        // 整数体取 8 位量程内的值：`testfix::encode_int_samples` 对超出量程的值会**故意**
        // panic（"夹具错误必须立刻可见"），而本条量的是格式而不是幅度。
        let ints = [0i32, 1, -1, 2];
        let floats = [0.5f32, -0.5, 0.25, -0.25];

        // 一格 = (标签, 线上声明, 线上体, 期望格式, 期望位深, 期望 is_float)。
        type Case = (&'static str, WavSpec, Vec<u8>, PcmFormat, u16, bool);
        let cases: [Case; 6] = [
            (
                "WAV tag 1, 8 bits",
                WavSpec {
                    channels: 1,
                    sample_rate: 8_000,
                    bits: 8,
                    format: WavFormat::Integer,
                },
                encode_int_samples(8, &ints),
                PcmFormat::U8,
                8,
                false,
            ),
            (
                "WAV tag 1, 16 bits",
                WavSpec {
                    channels: 1,
                    sample_rate: 8_000,
                    bits: 16,
                    format: WavFormat::Integer,
                },
                encode_int_samples(16, &ints),
                PcmFormat::S16,
                16,
                false,
            ),
            (
                "WAV tag 1, 24 bits",
                WavSpec {
                    channels: 1,
                    sample_rate: 8_000,
                    bits: 24,
                    format: WavFormat::Integer,
                },
                encode_int_samples(24, &ints),
                PcmFormat::S24,
                24,
                false,
            ),
            (
                "WAV tag 1, 32 bits",
                WavSpec {
                    channels: 1,
                    sample_rate: 8_000,
                    bits: 32,
                    format: WavFormat::Integer,
                },
                encode_int_samples(32, &ints),
                PcmFormat::S32,
                32,
                false,
            ),
            (
                "WAV tag 3, 32 bits",
                WavSpec {
                    channels: 1,
                    sample_rate: 8_000,
                    bits: 32,
                    format: WavFormat::Float,
                },
                encode_f32_samples(&floats),
                PcmFormat::F32,
                32,
                true,
            ),
            (
                "WAV tag 3, 64 bits",
                WavSpec {
                    channels: 1,
                    sample_rate: 8_000,
                    bits: 64,
                    format: WavFormat::Float,
                },
                f64_body,
                PcmFormat::F64,
                64,
                true,
            ),
        ];
        for (label, spec, body, format, depth, is_float) in cases {
            let asset = decode_bytes(&wav(&spec, &body), &DecodeOptions::default())
                .unwrap_or_else(|err| panic!("{label}: {err}"));
            assert_eq!(asset.pcm_format(), format, "{label}: format");
            assert_eq!(asset.pcm_format().bit_depth(), depth, "{label}: bit depth");
            assert_eq!(asset.pcm_format().is_float(), is_float, "{label}: is_float");
            assert_eq!(asset.frame_count(), 4, "{label}: frames");
        }

        // FLAC 的出口是 `S32`（解码器把 16 位整数样本放宽到 32 位容器），与线性的
        // `bits_per_sample` 声明**不同** —— 这正是"位深来自解码器真的吐出了什么"。
        let flac_spec = FlacSpec {
            sample_rate: 8_000,
            channels: 2,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        };
        let flac = decode_bytes(
            &flac_constant(&flac_spec, 2, 500),
            &DecodeOptions::default(),
        )
        .expect("the FLAC fixture must decode");
        assert_eq!(flac.pcm_format(), PcmFormat::S32);
        assert_eq!(flac.pcm_format().bit_depth(), 32);
        assert!(!flac.pcm_format().is_float());
    }

    /// 判据（`HD-24` 第五道闸门 / 公开入口对称性）：`decode_reader` 也必须过**输入字节**闸门。
    ///
    /// 量什么：`decode_reader` 在预算小于真实字节数时的返回值，以及预算恰好等于真实字节数
    /// 时的解出帧数。
    /// 怎么量：[`MeasuredSource`] 在构造时量出流长度，`decode_reader` 用它判
    /// [`limits::check_input_len`]。判据把上限设成"真实长度减一"与"恰好等于真实长度"。
    ///
    /// 读数（本机、debug 构建）：上限 = 长度 − 1 ⇒
    /// `InputTooLarge { bytes: <真实长度>, limit: <长度−1> }`；上限 = 长度 ⇒ 闭区间通过、
    /// 解出 2 帧。
    ///
    /// 为什么需要它：本批的公开面普查显示 `decode_reader` 是四个公开入口里**唯一没有**
    /// 输入字节闸门判据的那个。注入实测坐实了这一点：把 [`MeasuredSource::byte_len`] 改成
    /// `None`（两处 `if let Some(len)` 因此全部跳过）之后，全部既有判据**照旧通过**。
    ///
    /// 注入（实测）：把 `MeasuredSource::byte_len` 改成 `None` ⇒ 本条以
    /// `decode_reader must obey the input-byte gate` 红；把 `check_input_len` 的 `>` 改成
    /// `>=` ⇒ 本条的"恰好等于上限"那一格红。
    #[test]
    fn the_reader_entry_obeys_the_input_byte_budget() {
        let bytes = int_wav(2, 16, &[1, -2, 3, -4]);
        let real = u64::try_from(bytes.len()).expect("the fixture fits u64");

        // 闭区间：恰好等于真实长度必须通过。
        let exact = DecodeOptions {
            budget: PcmBudget {
                max_input_bytes: real,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        assert_eq!(
            decode_reader(Cursor::new(bytes.clone()), &exact)
                .expect("exactly the input length must pass the closed interval")
                .frame_count(),
            2
        );

        // 少一个字节：必须报 InputTooLarge，且数字是真实长度与生效上限。
        let short = DecodeOptions {
            budget: PcmBudget {
                max_input_bytes: real - 1,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        match decode_reader(Cursor::new(bytes.clone()), &short) {
            Err(DecodeError::Budget(LimitViolation::InputTooLarge { bytes: got, limit })) => {
                assert_eq!((got, limit), (real, real - 1));
            }
            other => panic!("decode_reader must obey the input-byte gate, got {other:?}"),
        }
    }

    /// 判据（类别⑥ 声道数极值 / 声道的**容器上界**）：FLAC 的声道域是 **1..=8**，因此
    /// `DEFAULT_MAX_CHANNELS = 64` 这道闸门**不可能被任何容器的内容触发**，它只是 backstop；
    /// 在 FLAC 上能触发它的只有"把预算收紧到 7"。
    ///
    /// 量什么：8 声道 FLAC 解出的 `channels()`、其 `STREAMINFO` 声道字段的**原始位值**，
    /// 以及"预算 `max_channels = 7` + 8 声道内容"的返回值。
    /// 怎么量：夹具构造 8 声道 CONSTANT FLAC（每声道同一个直流值）；位值从
    /// `STREAMINFO[10..18]` 的大端 64 位里按 `>> 41 & 0b111` 取（与
    /// `testfix::tests::flac_fixture_has_a_valid_streaminfo_and_frame_chain` 同一公式）。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 项 | 值 |
    /// | :--- | :--- |
    /// | 8 声道 FLAC 的 `channels()` | 8 |
    /// | `samples().len()`（256 帧 × 8） | 2048 |
    /// | `STREAMINFO` 声道字段 | **7**（= `channels - 1`，3 位字段的**最大值**） |
    /// | `max_channels = 7` 时 | `TooManyChannels { channels: 8, limit: 7 }` |
    ///
    /// **为什么 >8 不可达（机械理由，不是夹具取舍）**：FLAC 用**两处**字段各自钉住声道域 ——
    /// `STREAMINFO` 的声道字段是 **3 位**（存 `channels - 1`，故 1..=8），帧头的声道赋值是
    /// **4 位**（`0b0000..=0b0111` ＝ 1..=8 个独立声道；`0b1000..=0b1010` 是三种立体声去相关，
    /// 仍是 2 声道）。两处都放不下 9。WAV 那一侧上游在 **32 声道**就 `riff: invalid channel
    /// count`（本 crate 的 `>64` 格因此也不可达）；Ogg 需要一份合法 Vorbis 夹具（尚未构造）。
    /// ⇒ 三族容器里**没有任何一族**能用内容触发 64 声道闸门。
    ///
    /// 注入（实测）：把夹具的声道域断言由 `(1..=8).contains(…)` 改成 `spec.channels <= 8`
    /// 之外的形式不会红（那是夹具自检的事）；本条真正钉住的是下面三个读数，把
    /// `check_layout` 的 `channels > budget.max_channels` 改成 `>=` ⇒ 本条的第三行红。
    #[test]
    fn the_channel_gate_cannot_be_tripped_by_content_of_any_enabled_container() {
        let spec = FlacSpec {
            sample_rate: 8_000,
            channels: 8,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        };
        let bytes = flac_constant(&spec, 1, 500);
        let asset = decode_bytes(&bytes, &DecodeOptions::default())
            .expect("an 8-channel FLAC is inside FLAC's channel domain");
        assert_eq!(asset.channels(), 8);
        assert_eq!(asset.frame_count(), 256);
        assert_eq!(asset.samples().len(), 256 * 8);

        // `STREAMINFO` 的声道字段必须读到 3 位字段的最大值 7（= 8 - 1）。
        let info = &bytes[8..42];
        let packed = u64::from_be_bytes([
            info[10], info[11], info[12], info[13], info[14], info[15], info[16], info[17],
        ]);
        assert_eq!(
            (packed >> 41) & 0b111,
            7,
            "the 3-bit channel field saturates at 7 (= 8 channels); 9 would need 8"
        );
        // 编译期断言：FLAC 的声道上界（8）必须低于闸门默认值（64），否则本条的前提
        // （"这道闸门只是 backstop"）就不成立。放成 `const` 断言而不是运行时 `assert!`，
        // 是为了让它在**编译**期生效（也避开 clippy 的 "constant assertion" 告警）。
        const _: () = assert!(
            8u16 < crate::limits::DEFAULT_MAX_CHANNELS,
            "FLAC's channel ceiling must be below the gate's default"
        );

        // 闸门在 FLAC 上只能靠"收紧预算"触发，报的是**内容声明的**声道数。
        let tightened = DecodeOptions {
            budget: PcmBudget {
                max_channels: 7,
                ..PcmBudget::default()
            },
            ..DecodeOptions::default()
        };
        match decode_bytes(&bytes, &tightened) {
            Err(DecodeError::Budget(LimitViolation::TooManyChannels { channels, limit })) => {
                assert_eq!((channels, limit), (8, 7));
            }
            other => {
                panic!("the channel gate must fire on the first decoded buffer, got {other:?}")
            }
        }
    }

    /// 判据（第三族容器**正常解出**）：一个最小但**合法**的 Ogg Vorbis 流必须解出
    /// 1 声道 / 44.1 kHz / [`crate::testfix::OGG_FIXTURE_FRAMES`] 帧的静音资产。
    ///
    /// 量什么：`decode_bytes` 对 [`crate::testfix::ogg_vorbis_silence`] 的 `channels()`、
    /// `sample_rate()`、`frame_count()`、`pcm_format()`、以及全部样本是否为零。
    /// 怎么量：夹具逐字段构造（Ogg 页 CRC-32 ＋ Vorbis 三个头包），不需要任何外部样本。
    ///
    /// 读数（本机、debug 构建）：`channels = 1`、`sample_rate = 44100`、`frames = 128`、
    /// `pcm_format = F32`，全部样本 `== 0.0`。
    ///
    /// 为什么需要它：**这是本 crate 第一个 Ogg 判据**。此前 `decode.rs` 的声道数判据文档
    /// 自己登记了"本 crate 没有 Ogg 的字节级夹具，因此那条路径没有判据覆盖"。构造这个夹具
    /// 的路走得很长，两条约束是**实测出来的、都不是可选的**（写进夹具文档）：
    /// 1. **至少 2 个数据页** —— `AudioDecoderOptions::default().gapless == true`，gapless 会把
    ///    "解码器重置后的第一个包"静音（`buf.clear()`）；
    /// 2. EOS 页的 **granule 必须等于解出的帧数** —— Ogg/Vorbis 读端把最后那个 granule 当作
    ///    "容器声明的总帧数"，而本 crate 的对账容差是 **0**。
    ///
    /// 另有三处 setup 头的位级陷阱写在 [`crate::testfix::ogg_vorbis_setup`] 的文档里：
    /// 每个计数都是"值 ＋ 1"；无序码本在 `ordered` 之后多一位 `is_sparse`；
    /// `code_len` 存的是"长度 − 1"。
    ///
    /// 注入（实测）：把 EOS 页的 granule 由 `OGG_FIXTURE_FRAMES` 改成 `2 * OGG_FIXTURE_FRAMES`
    /// ⇒ 本条以 `declared duration disagrees with decoded frames` 红（**这一条正是夹具能红
    /// 的关键**：granule 与解出帧数不等时，本 crate 的对账闸门会拒绝整份资产）。
    #[test]
    fn a_minimal_ogg_vorbis_stream_decodes_to_silence() {
        let bytes = crate::testfix::ogg_vorbis_silence(2);
        let asset = decode_bytes(&bytes, &DecodeOptions::default())
            .expect("a minimal but legal Ogg Vorbis stream must decode");
        assert_eq!(asset.channels(), 1);
        assert_eq!(asset.sample_rate(), 44_100);
        assert_eq!(asset.pcm_format(), PcmFormat::F32);
        assert_eq!(asset.frame_count(), crate::testfix::OGG_FIXTURE_FRAMES);
        assert!(
            asset.samples().iter().all(|sample| *sample == 0.0),
            "the fixture's floor is unused, so every sample must be exactly zero"
        );
        assert!(asset.samples().len() == usize::try_from(asset.frame_count()).unwrap());
    }

    /// 判据（串联物理流 / [ARCH-SEC-003]）：**两条物理流串联**的 Ogg 必须以
    /// [`DecodeError::ResetRequired`] 拒绝，**与第二条流的声道数是否相同无关**。
    ///
    /// 量什么：`decode_bytes` 对 [`crate::testfix::ogg_vorbis_chained`] 的返回值 ——
    /// 第二条第 1 声道与第 2 声道两种取法。
    /// 怎么量：夹具把同一条最小 Vorbis 流写两遍，`serial` 不同（7 与 8），第二条的识别头声道数
    /// 由参数决定。
    ///
    /// 读数（本机、debug 构建）：两种取法**都**得到
    /// `Err(DecodeError::ResetRequired)`（文案 `stream requires a decoder reset mid-decode
    /// (chained stream)`）。
    ///
    /// 为什么需要它（两条结论）：
    /// 1. [`DecodeError::ResetRequired`] 此前**没有入口判据** —— 它只在
    ///    `symphonia_errors_map_onto_typed_variants` 里被**构造**过一次，而没有任何判据证明
    ///    真实字节能走到它。
    /// 2. ⭐ **"中途换声道布局"那条 `InconsistentLayout` 分支不可达**：实测表明串联给的是
    ///    `ResetRequired`，而不是"第二个缓冲的声道数不同"。⇒ `decode.rs` 里
    ///    `locked_channels != channels` 那一格的机械理由是"容器不支持在同一逻辑流里换布局"，
    ///    而"串联"这条唯一候选路径由读端自己先拒了。
    ///
    /// 注入（实测）：把夹具第二条的 `serial` 与第一条相同（8 → 7）⇒ 本条**仍然全绿**
    /// （两种声道数取法都还是 `ResetRequired`）。也就是说**触发条件不是 serial 的差异**，
    /// 而是"页序列里出现了**第二组 BOS/头页**" —— 读端据此判定"新的逻辑流开始了"。
    /// 这条读数已按 R51 的口径如实登记：本判据钉的是"第二组头页必被拒"，不是"serial 必须不同"。
    #[test]
    fn a_chained_ogg_stream_is_refused_with_a_reset_demand() {
        for second_channels in [1u8, 2] {
            let bytes = crate::testfix::ogg_vorbis_chained(3, second_channels);
            match decode_bytes(&bytes, &DecodeOptions::default()) {
                Err(DecodeError::ResetRequired) => {}
                other => panic!(
                    "a chained Ogg stream (second stream {second_channels} channel(s)) must be \
                     refused with ResetRequired, got {other:?}"
                ),
            }
        }
        // 对照：单条物理流（同样的 setup 与数据页）必须**正常解出** —— 证明上面红的是"串联"
        // 而不是夹具本身坏了。
        let single = crate::testfix::ogg_vorbis_vq_stream(7, 1, 8, false, 3);
        let asset = decode_bytes(&single, &DecodeOptions::default())
            .expect("a single physical stream must still decode");
        assert_eq!(asset.channels(), 1);
        assert_eq!(asset.frame_count(), crate::testfix::OGG_FIXTURE_FRAMES);
    }

    /// 判据（`Floor0` 的**被使用**路径）：`order = 1` ＋ `amplitude_bits = 8` 的 floor0 必须
    /// 合成出 128 帧，而 `amplitude_bits = 1` 的同一份流必须解不出帧。
    ///
    /// 量什么：`decode_bytes` 对 [`crate::testfix::ogg_vorbis_vq_stream`] 在 `amplitude_bits ∈
    /// {1, 8}` 两档下的返回值（其余字段完全相同，包括 `lookup_type = 1` 的 VQ 码本与
    /// `delta_value = 1.0`）。
    /// 怎么量：两条流只在 `amplitude_bits` 那 6 位上不同。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | `amplitude_bits` | 读数 |
    /// | :--- | :--- |
    /// | 8 | `Ok`：1 声道、44100 Hz、**128 帧**、全部样本有限（floor 被真正合成） |
    /// | 1 | 每个音频包都被解码器拒 ⇒ `Err(EmptyStream)` |
    ///
    /// 为什么需要它：静音夹具（`amplitude_bits = 0`）走的是"floor 未使用 ⇒ 全部声道
    /// `do_not_decode`"，**完全不经过** `Floor0::synthesis`。本条让那条路径真的跑起来，
    /// 并把 `amplitude_bits` 的 1 vs 8 这个**实测边界**钉住。
    ///
    /// ⚠ **两档用的是同一个包 `0x12`**（[`crate::testfix::ogg_vorbis_vq_stream`] 写死），
    /// 因此两档之间变化的**不只是** floor 的幅度位宽，还有"这 8 位怎么被消费"。也就是说
    /// 上面那条"1 档被饿死"只是**读数**，不是已证实的机制。第十批实测：把包改成按
    /// `amplitude_bits = 8` **正确对齐**、`amplitude = 1` 时，解码器报的是
    /// `vorbis: invalid floor0 coefficients`（读实现得到的落点是 `floor.rs:326` 的
    /// `p + q == 0.0`）—— 即"被使用的 floor0"在正确对齐的包上**并不**像本条第一格那样顺利
    /// 解出。⛔ 因此本条的名称与文档**只声明实测读数**，不声明机制。
    ///
    /// ⚠ **未诊断的部分（如实登记）**：`Floor0::synthesis` 里有一处
    /// `if p + q == 0.0 { decode_error("vorbis: invalid floor0 coefficients") }`
    /// （`symphonia-codec-vorbis/src/floor.rs:326`）。我**读到了**这个条件，但**没能构造出**
    /// 触发它的输入：把 VQ 系数置零（`delta_value = 0`）在 `amplitude_bits = 8` 下**照常解出**
    /// 128 帧（实测）。`p`／`q` 是 `(coeff - 2cos ω)` 的连乘，置零的系数要撞上
    /// `ω ≡ 0 (mod 2π)` 才会让 `p + q == 0`，那取决于 `bark_map_size` 与 bark 映射的具体取值。
    /// ⇒ 该条件目前**没有判据**，`amplitude_bits = 1` 为何失败也**未诊断**（只钉住读数）。
    ///
    /// 注入（实测）：把 `amplitude_bits` 那 6 位固定成 8（把 `= 1` 的用例也写成 8）⇒
    /// 本条第二格红。
    #[test]
    fn a_used_floor0_synthesises_at_amplitude_bits_eight_but_not_one() {
        let used = crate::testfix::ogg_vorbis_vq_stream(7, 1, 8, false, 3);
        let asset = decode_bytes(&used, &DecodeOptions::default())
            .expect("a used floor0 (order = 1, amplitude_bits = 8) must decode");
        assert_eq!(asset.channels(), 1);
        assert_eq!(asset.sample_rate(), 44_100);
        assert_eq!(asset.frame_count(), crate::testfix::OGG_FIXTURE_FRAMES);
        assert!(
            asset.samples().iter().all(|sample| sample.is_finite()),
            "the synthesised floor must produce finite samples"
        );

        let starved = crate::testfix::ogg_vorbis_vq_stream(7, 1, 1, false, 3);
        match decode_bytes(&starved, &DecodeOptions::default()) {
            Err(DecodeError::EmptyStream) => {}
            other => panic!(
                "amplitude_bits = 1 must starve the floor so every audio packet is refused, \
                 got {other:?}"
            ),
        }
    }

    /// 判据（**非零谱** ⇒ 非静音样本 [ARCH-SEC-003][ARCH-DSP-002]）：一个 residue 区间非空、
    /// 且 VQ 系数非零的 Ogg Vorbis 流必须解出**全部样本非零**的资产；把 VQ 系数置零则必须被拒。
    ///
    /// 量什么：`decode_bytes` 对 [`crate::testfix::ogg_vorbis_nonzero_stream`] 两种取法下的
    /// **非零样本个数**（不是"没崩"）、峰值绝对值、帧数。
    /// 怎么量：夹具的 residue 是 `begin = 0 / end = 128 / partition_size = 128` ⇒ 恰好 1 个
    /// partition ⇒ `read_residue_partition_format0` 读 128 个码字，每个给谱加 `delta`。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | `delta_value` | 读数 |
    /// | :--- | :--- |
    /// | `1.0`（[`crate::testfix::OGG_VORBIS_DELTA_ONE`]） | `Ok`：128 帧、**非零样本 128/128**、峰值 `58.523415` |
    /// | `0`（VQ 向量全零） | 每个音频包被拒 ⇒ `Err(EmptyStream)` |
    ///
    /// 为什么需要它：此前所有 Ogg 夹具的稀疏约束都是 `begin = end = 0` ⇒ 谱恒为零 ⇒
    /// 输出 `floor × 0 = 0` ⇒ **静音**。也就是说 `floor` 是否被使用、`residue` 是否有数据，
    /// 在那批夹具上**都测不出来**。本条第一次让"谱非零"这件事有机械读数（非零样本计数），
    /// 并且用**同一个 `==`** 两侧都断言（`assert_ne!(nonzero, 0)` 落在"非零计数"上）。
    ///
    /// ⚠ 一并登记 `delta` 的算错修正：`float32_unpack` 的口径是 `mantissa × 2^(exponent − 788)`，
    /// 因此 `1.0` 要 `mantissa = 1、exponent = 788` ⇒ **`0x6280_0001`**。第八/九/十批我一直写的
    /// `0x62A0_0000` 的 `mantissa` 是 **0** ⇒ 它编码的是 **0.0**，正是"floor 系数全零 ⇒
    /// `p + q == 0` ⇒ `vorbis: invalid floor0 coefficients`"两轮构造不出非静音的根因。
    ///
    /// 注入（实测）：把 [`crate::testfix::OGG_VORBIS_DELTA_ONE`] 改成 `0x62A0_0000`（回到算错的
    /// 常数）⇒ 本条第一格红（读数变成 `EmptyStream`）。
    #[test]
    fn a_nonzero_residue_spectrum_decodes_to_nonzero_samples() {
        let bytes = crate::testfix::ogg_vorbis_nonzero_stream(2, false);
        let asset = decode_bytes(&bytes, &DecodeOptions::default())
            .expect("a non-zero spectrum must decode");
        assert_eq!(asset.channels(), 1);
        assert_eq!(asset.sample_rate(), 44_100);
        assert_eq!(asset.frame_count(), crate::testfix::OGG_FIXTURE_FRAMES);
        let nonzero = asset.samples().iter().filter(|s| **s != 0.0).count();
        // ⛔ 不用"没崩"当证据：必须数出非零样本，并在**同一个** `!=` 上给出两侧断言。
        assert_ne!(
            nonzero, 0,
            "a non-zero residue must produce non-zero samples"
        );
        assert_eq!(
            nonzero,
            asset.samples().len(),
            "every sample carries the non-zero spectrum"
        );
        assert_eq!(nonzero, 128);
        let peak = asset.samples().iter().fold(0f32, |max, s| max.max(s.abs()));
        assert!(peak > 1.0, "the peak must be well above zero, got {peak}");
        assert!(asset.samples().iter().all(|s| s.is_finite()));

        let zero = crate::testfix::ogg_vorbis_nonzero_stream(2, true);
        match decode_bytes(&zero, &DecodeOptions::default()) {
            Err(DecodeError::EmptyStream) => {}
            other => {
                panic!("zero VQ coefficients make p + q == 0 and must be refused, got {other:?}")
            }
        }
    }

    /// R115 常驻化（第 1/2 件）：**源码文本扫描器**，核两件机械性质。
    ///
    /// 扫描器**自己**是纯函数 `scan_sources`，因此可以按 R56 用**运行期拼出来的**已知红/已知绿
    /// 样本喂它（见下面的自检断言）—— ⛔ 不用"本地全绿"当证据。
    ///
    /// 核的两件性质：
    /// 1. **R102／R109／R118**：每个 `.all(` 的**前 40 行**内必须出现一处**界定被遍历集合大小**
    ///    的界。⚠ R118 的分类：**算**的是 `frame_count` / `.len()` / `.count()` / `is_empty`
    ///    / `!= 0`（它们界定集合**大小**）；**不算**的是元素值界（`peak > 1.0`）与运行期计数器
    ///    （`visited += 1`）—— 后者不界定"被遍历的集合"，`.all()` 在空集上仍然真空为真。
    /// 2. **R100**：每处 `env::temp_dir()` 之后的 60 行内必须有 `.expect(`（判据先写后读、
    ///    写失败**响亮失败**；⛔ 不允许静默跳过 ⇒ 也就没有 `SKIP(vacuous)` 这一支）。
    ///
    /// ⚠ **R119（near-miss）**：`.all(` 的匹配要求**前一个字符是 `.`、后一个字符是 `(`**
    /// ⇒ `small(`/`overall(`/`install(` 不会被误认。
    /// ⚠ **R84／R80**：两个针都在**运行期拼接**（`concat!`），⛔ 不写成整片字面量 —— 否则针
    /// 出现在本判据自己的源码里，检查会**自我满足**。
    /// ⚠ **R122／"可能惰性"**：扫描器最后必须断言**真的扫到了**下界数量的 `.all(` 与
    /// `env::temp_dir()` 命中点，否则"零违规"可能只是"零命中"。
    #[test]
    fn the_crate_source_keeps_a_set_size_bound_before_every_all_and_a_write_before_every_temp_dir()
    {
        // 针：运行期拼接（R84），所以这两片字符串**不是**完整形态。
        // ⚠ R119 的**规格**很关键：针只到 `.all`，**边界检查**才要求后面紧跟 `(`。
        // 若把 `(` 也放进针里、再要求后面还有 `(`，就会**一个都匹配不到**（假清洁）——
        // 本判据第一版就是这么写的，正是**已知红对照**把它抓出来的。
        let all_needle = concat!(".", "all");
        let temp_needle = concat!("env::", "temp_dir()");
        let bound_needles = ["frame_count", ".len()", ".count()", "is_empty", "!= 0"];
        let expect_needles = [concat!(".", "expect("), concat!(".", "unwrap(")];

        /// **保字节数**地把 行注释 与 双引号字符串 的内容换成空格（R113：⛔ 不能改长度，
        /// 否则行号与偏移都会错位 ⇒ 真违规会被静默跳过）。换行符本身保留。
        fn mask(source: &str) -> String {
            let bytes: Vec<char> = source.chars().collect();
            let mut out = bytes.clone();
            let mut i = 0usize;
            while i < bytes.len() {
                // 行注释
                if bytes[i] == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
                    while i < bytes.len() && bytes[i] != '\n' {
                        out[i] = ' ';
                        i += 1;
                    }
                    continue;
                }
                // 双引号字符串（支持 \" 转义；**字节数不变**）
                if bytes[i] == '"' {
                    out[i] = ' ';
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == '\\' {
                            out[i] = ' ';
                            if i + 1 < bytes.len() {
                                out[i + 1] = ' ';
                            }
                            i += 2;
                            continue;
                        }
                        let closing = bytes[i] == '"';
                        out[i] = ' ';
                        i += 1;
                        if closing {
                            break;
                        }
                    }
                    continue;
                }
                i += 1;
            }
            out.into_iter().collect()
        }

        /// 返回 `(行号, 该行)`，只报告窗口内**既没有集合大小界**的 `.all(`。
        fn scan_all(
            source: &str,
            all_needle: &str,
            bound_needles: &[&str],
            window: usize,
        ) -> Vec<(usize, String)> {
            let lines: Vec<&str> = source.lines().collect();
            let mut findings = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                // R119：标识符边界 —— 后一个字符必须是 `(`（前一个字符由针自带 `.`）。
                for (offset, _) in line.match_indices(all_needle) {
                    let after = line[offset + all_needle.len()..].chars().next();
                    if after != Some('(') {
                        continue;
                    }
                    let first = index.saturating_sub(window);
                    let bounded = lines[first..index]
                        .iter()
                        .any(|earlier| bound_needles.iter().any(|needle| earlier.contains(needle)));
                    // 同一行内的界也算（单行写法）。
                    let bounded = bounded
                        || bound_needles
                            .iter()
                            .any(|needle| line[..offset].contains(needle));
                    if !bounded {
                        findings.push((index + 1, (*line).to_owned()));
                    }
                }
            }
            findings
        }

        /// 返回所有 `env::temp_dir()` 之后 `window` 行内**没有** `.expect(` 的行号。
        fn scan_temp_dir(
            source: &str,
            temp_needle: &str,
            expect_needles: &[&str],
            window: usize,
        ) -> Vec<(usize, String)> {
            let lines: Vec<&str> = source.lines().collect();
            let mut findings = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                if !line.contains(temp_needle) {
                    continue;
                }
                let last = (index + window).min(lines.len());
                if !lines[index..last]
                    .iter()
                    .any(|later| expect_needles.iter().any(|needle| later.contains(needle)))
                {
                    findings.push((index + 1, (*line).to_owned()));
                }
            }
            findings
        }

        // ---- R56／R108：先用已知红 ＋ 已知绿喂这两个扫描器（样本在运行期拼出来）----
        let red = concat!(
            "fn f(x: &[f32]) {\n",
            "    assert!(x.iter().",
            "all(|s| s.is_finite()));\n", // ⛔ 前 40 行没有任何集合大小界
            "    let mut p = ",
            "env::temp_dir();\n", // ⛔ 后面没有 .expect(
            "    p.push(\"a\");\n",
            "}\n",
        );
        let green = concat!(
            "fn g(x: &[f32]) {\n",
            "    assert_eq!(x.len(), 4);\n",
            "    assert!(x.iter().",
            "all(|s| s.is_finite()));\n", // ✅ 前 40 行有 .len()
            "    let mut p = ",
            "env::temp_dir();\n",
            "    std::fs::write(&p, b\"x\").",
            "expect(\"writable\");\n", // ✅ 有 .expect(
            "}\n",
        );
        assert_eq!(
            scan_all(red, all_needle, &bound_needles, 40).len(),
            1,
            "known-red: an `.all(` with no set-size bound in its window must be reported"
        );
        assert_eq!(
            scan_temp_dir(&mask(red), temp_needle, &expect_needles, 60).len(),
            1,
            "known-red: a temp_dir without a later .expect( must be reported"
        );
        assert!(
            scan_all(green, all_needle, &bound_needles, 40).is_empty(),
            "known-green: a `.all(` preceded by `.len()` must not be reported"
        );
        assert!(
            scan_temp_dir(&mask(green), temp_needle, &expect_needles, 60).is_empty(),
            "known-green: a temp_dir followed by .expect( must not be reported"
        );
        // R113③：掩码**必须保字节数**（否则行号/偏移错位 ⇒ 真违规会静默跳过）。
        assert_eq!(
            mask(red).len(),
            red.len(),
            "masking must preserve the byte count"
        );
        assert_eq!(
            mask(green).lines().count(),
            green.lines().count(),
            "masking must preserve the line count"
        );
        // 掩码的已知红：字符串里的针必须被掩掉。
        assert!(
            !mask("let s = \"env::temp_dir()\";").contains(temp_needle),
            "known-red for the masker: a needle inside a string literal must be masked out"
        );

        // ---- 真扫本 crate 的四个源文件（`include_str!` 的路径相对本文件所在目录）----
        let sources = [
            ("decode.rs", include_str!("decode.rs")),
            ("resample.rs", include_str!("resample.rs")),
            ("asset.rs", include_str!("asset.rs")),
            ("limits.rs", include_str!("limits.rs")),
        ];
        let mut all_sites = 0usize;
        let mut temp_sites = 0usize;
        for (name, source) in sources {
            // ⚠ 先保字节数掩码（R113）：否则**本判据自己的断言字符串**里出现的针会被当成真命中。
            let masked = mask(source);
            let offenders = scan_all(&masked, all_needle, &bound_needles, 40);
            all_sites += masked.matches(all_needle).count();
            assert!(
                offenders.is_empty(),
                "{name}: every `.all(` must have a set-size bound within 40 lines, offenders: \
                 {offenders:?}"
            );
            let temp_offenders = scan_temp_dir(&masked, temp_needle, &expect_needles, 60);
            temp_sites += masked.matches(temp_needle).count();
            assert!(
                temp_offenders.is_empty(),
                "{name}: every `env::temp_dir()` must be followed by `.expect(` within 60 lines, \
                 offenders: {temp_offenders:?}"
            );
        }
        // R122：证明扫描器**不是惰性的** —— 命中点数必须达到下界。
        assert!(
            all_sites >= 5,
            "the scan must actually match `.all(` sites, matched {all_sites}"
        );
        assert!(
            temp_sites >= 2,
            "the scan must actually match `env::temp_dir()` sites, matched {temp_sites}"
        );
    }
}
