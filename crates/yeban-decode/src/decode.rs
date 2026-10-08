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
/// # Errors
///
/// 见 [`DecodeError`]。
pub fn decode_source<'s>(
    mut source: Box<dyn MediaSource + 's>,
    hint: &Hint,
    options: &DecodeOptions,
) -> DecodeResult<DecodedAsset> {
    // 上游的 RIFF/WAVE 解析器在一条畸形声明上会整型溢出并 panic。本 crate 的契约是
    // "不可信输入只返回 DecodeError"，而我们不能改上游，因此在探测之前先走一遍块头。
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

    // 预算：先按"声明"做一次廉价的前置检查（畸形文件常在头里就声称几小时）。
    // 传 `channels = 1` 是有意的 —— 这一层只判"每声道帧数 × 4 字节"是否超预算，
    // 声道数还不知道（要等第一个解码缓冲），因此不会在这里误判声道相关的分支。
    // 时长闸门与声道数无关（`frames / sample_rate`），因此在这里判它是准确的。
    if let Some(frames) = declared_frames {
        limits::check_layout(1, sample_rate, frames, &options.budget)?;
    }

    let sample_budget = options.budget.interleaved_samples_limit();
    let mut samples: Vec<f32> = Vec::new();
    let mut layout: Option<(u16, PcmFormat)> = None;

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
        if planes == 0 || frames_in_buffer.checked_mul(planes) != Some(total) {
            return Err(DecodeError::InconsistentLayout {
                detail: format!(
                    "buffer reports {total} interleaved samples but {frames_in_buffer} frames \
                     x {planes} planes"
                ),
            });
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

/// RIFF 块头的扫描上界（畸形文件可以声明无数个 0 长度块）。
const RIFF_PRECHECK_MAX_CHUNKS: u32 = 4_096;

/// 在探测之前拒掉会让上游 RIFF 解析器整型溢出的 `fmt ` 声明。
///
/// 上游事实（逐行读源码得到；本 crate **不能**修改上游）：
/// `symphonia-format-riff-0.6.1/src/wave/chunks.rs:100`
/// `let expected_block_align = num_channels * (bits_per_sample / 8);`
/// 两个操作数都是 `u16`。乘积大于 `u16::MAX` 时，debug 与 release 都会 panic ——
/// 根 `Cargo.toml` 的 `[profile.release] overflow-checks = true` 让它在生产构建里也炸。
/// 那次乘法只有**一个**调用点：`WAVE_FORMAT_PCM`（tag `0x0001`，同文件 `chunks.rs:438-440`）。
/// 复现基线：44 字节 WAV 头 + 2 个 16-bit 样本（共 48 字节），`num_channels = 32769` ⇒
/// `32769 × 2 = 65538 > 65535`。
///
/// 这道闸门是**只拒**的：它只在"上游那次乘法确实会溢出"时报错。上游能正常解析的文件
/// 一个都不会被它拒掉。扫描不到 `fmt ` 块时它什么都不做，把判定留给探测器。
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
fn scan_riff_for_fmt(source: &mut dyn MediaSource) -> DecodeResult<()> {
    let mut header = [0u8; 12];
    if source.read_exact(&mut header).is_err() {
        return Ok(());
    }
    if header[0..4] != b"RIFF"[..] || header[8..12] != b"WAVE"[..] {
        return Ok(());
    }
    for _ in 0..RIFF_PRECHECK_MAX_CHUNKS {
        let mut chunk = [0u8; 8];
        if source.read_exact(&mut chunk).is_err() {
            return Ok(());
        }
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
            // 只有 tag 1 + 上游接受的位深才会走到那次乘法；其余形状上游自己会报错。
            let bytes_per_sample = match bits_per_sample {
                8 | 16 | 24 | 32 => u32::from(bits_per_sample / 8),
                _ => return Ok(()),
            };
            if format_tag == 1 && u32::from(num_channels) * bytes_per_sample > u32::from(u16::MAX) {
                return Err(DecodeError::Malformed {
                    detail: format!(
                        "WAV fmt declares {num_channels} channels at {bits_per_sample} bits per \
                         sample: the RIFF parser computes num_channels * (bits_per_sample / 8) in \
                         u16 and would overflow (symphonia-format-riff src/wave/chunks.rs:100)"
                    ),
                });
            }
            return Ok(());
        }
        // 下一个块 = 8 字节块头 + 块体 + 奇数长度块的填充字节。
        let advance = i64::try_from(
            8u64.saturating_add(u64::from(size))
                .saturating_add(u64::from(size & 1)),
        )
        .unwrap_or(i64::MAX);
        if source.seek(SeekFrom::Current(advance)).is_err() {
            return Ok(());
        }
    }
    Ok(())
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
        FlacSpec, WavFormat, WavSpec, encode_f32_samples, encode_int_samples, flac_constant, wav,
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
}
