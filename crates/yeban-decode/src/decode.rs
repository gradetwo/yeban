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
/// # Errors
///
/// - 输入超过 `max_input_bytes` ⇒ [`LimitViolation::InputTooLarge`]；
/// - 分配器拒绝 ⇒ [`LimitViolation::AllocationRefused`]（该变体的计数**在这里的单位是
///   容器字节**，因为被增长的缓冲装的是容器字节，不是音频样本）；
/// - 源自身的 I/O 失败原样上报。
fn slurp_unseekable(source: &mut dyn MediaSource, max_input_bytes: u64) -> DecodeResult<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = [0u8; SLURP_CHUNK_BYTES];
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
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
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
        flac_constant, fmt_body_offset, wav, wav_extensible, wav_extensible_with_junk,
        wav_with_chunks_before_fmt, wav_with_declared_len,
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
        let budget = PcmBudget::new(limit, !3u64, 64, 768_000, 60);
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
            budget: PcmBudget::new(limit, !3u64, 64, 768_000, 60),
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
}
