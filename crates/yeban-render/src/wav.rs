//! 普通 RIFF WAV 的读写, 走 `hound` 作为**独立第三方实现**。
//!
//! ## 为什么还要一个用 `hound` 的模块
//!
//! 自研的 RF64/BW64 写入器在 [`crate::rf64`]（零第三方依赖）。如果只用自己的
//! 写入器 + 自己的读取器做往返测试, 那只证明"我的两个实现互相同意"——对"两个都
//! 不符合规范"完全无感。
//!
//! `hound` 在这里的价值正是**当裁判**:
//!
//! - [`read_plain_wav`]: 用 `hound` 读回 [`crate::rf64`] 写出的普通 RIFF WAV,
//!   逐样本对账;
//! - [`write_plain_wav`]: 用 `hound` 写一份普通 WAV, 再用
//!   [`crate::rf64::parse_container`] 解析它, 核对 `fmt `/`data` 的字节与格式;
//! - [`ChunkLayout`]: 直接断言 WAV 的 `fmt ` 与 `data` chunk 长度。
//!
//! ## 边界
//!
//! - 只做**普通 RIFF WAV**; RF64/BW64 的 4 GiB 突破由 [`crate::rf64`] 负责
//!   （`hound` 自己不支持 >4 GiB, 也不支持 `ds64`）。
//! - `hound` 不读 `bext` 的语义内容（它跳过未知 chunk）, 因此 `bext` 的正确性由
//!   [`crate::rf64`] 自己的读取器核验。

use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use crate::dither::{BitDepth, PcmBuffer};
use crate::rf64::PcmFormat;

/// `hound` 桥接错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavError {
    /// `hound` 报的错。
    Hound(String),
    /// 位深/样本格式与缓冲不匹配。
    FormatMismatch {
        /// 期望的格式。
        expected: String,
        /// 实际拿到的。
        got: String,
    },
    /// `hound` 不支持的位深。
    UnsupportedDepth(u16),
}

impl core::fmt::Display for WavError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Hound(message) => write!(f, "hound 失败: {message}"),
            Self::FormatMismatch { expected, got } => {
                write!(f, "格式不匹配: 期望 {expected}, 实际 {got}")
            }
            Self::UnsupportedDepth(bits) => write!(f, "不支持的位深: {bits}"),
        }
    }
}

impl std::error::Error for WavError {}

impl From<hound::Error> for WavError {
    fn from(error: hound::Error) -> Self {
        Self::Hound(error.to_string())
    }
}

/// 把本 crate 的 [`PcmFormat`] 转成 `hound::WavSpec`。
///
/// # Errors
///
/// `bits_per_sample` 不是 16 / 24 / 32 时返回
/// [`WavError::UnsupportedDepth`] —— `hound` 的 `WavSpec` 没有这些位深的写入路径。
pub fn hound_spec(format: &PcmFormat) -> Result<WavSpec, WavError> {
    if !matches!(format.bits_per_sample, 16 | 24 | 32) {
        return Err(WavError::UnsupportedDepth(format.bits_per_sample));
    }
    Ok(WavSpec {
        channels: format.channels,
        sample_rate: format.sample_rate,
        bits_per_sample: format.bits_per_sample,
        sample_format: if format.is_float {
            SampleFormat::Float
        } else {
            SampleFormat::Int
        },
    })
}

/// 由缓冲反推它**必须**配套的格式。
#[must_use]
pub fn format_of(buffer: &PcmBuffer, channels: u16, sample_rate: u32) -> PcmFormat {
    match buffer.depth() {
        BitDepth::Int16 => PcmFormat::integer(channels, sample_rate, 16),
        BitDepth::Int24 => PcmFormat::integer(channels, sample_rate, 24),
        BitDepth::Float32 => PcmFormat::float(channels, sample_rate, 32),
    }
}

/// 校验格式与缓冲互相匹配。
///
/// # Errors
///
/// 位深或整数/浮点类别不一致。
pub fn check_match(format: &PcmFormat, buffer: &PcmBuffer) -> Result<(), WavError> {
    let expected_depth = buffer.depth();
    let ok = format.bits_per_sample == expected_depth.bits()
        && format.is_float == matches!(expected_depth, BitDepth::Float32);
    if ok {
        Ok(())
    } else {
        Err(WavError::FormatMismatch {
            expected: format!("{expected_depth:?} ({} 位)", expected_depth.bits()),
            got: format!(
                "{} 位, {}",
                format.bits_per_sample,
                if format.is_float { "浮点" } else { "整数" }
            ),
        })
    }
}

/// 用 `hound` 写一份普通 RIFF WAV。
///
/// # Errors
///
/// 格式与缓冲不匹配、位深不受支持、或 `hound` I/O 失败。
pub fn write_plain_wav(
    path: impl AsRef<Path>,
    format: &PcmFormat,
    buffer: &PcmBuffer,
) -> Result<(), WavError> {
    check_match(format, buffer)?;
    let spec = hound_spec(format)?;
    let mut writer = WavWriter::create(path, spec)?;
    match buffer {
        PcmBuffer::Int16(samples) => {
            for &sample in samples {
                writer.write_sample(sample)?;
            }
        }
        PcmBuffer::Int24(samples) => {
            for &sample in samples {
                writer.write_sample(sample)?;
            }
        }
        PcmBuffer::Float32(samples) => {
            for &sample in samples {
                writer.write_sample(sample)?;
            }
        }
    }
    writer.finalize()?;
    Ok(())
}

/// 用 `hound` 读回一份 WAV, 返回 `(格式, 缓冲)`。
///
/// 整数样本会被规范成 16-bit 或 24-bit 的 `PcmBuffer` 变体（`hound` 读 24-bit
/// 整数时给出符号扩展后的 `i32`, 与 [`crate::dither::PcmBuffer::Int24`] 的口径一致）。
///
/// # Errors
///
/// 位深不受支持或 `hound` I/O 失败。
pub fn read_plain_wav(path: impl AsRef<Path>) -> Result<(PcmFormat, PcmBuffer), WavError> {
    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();
    let format = PcmFormat {
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        bits_per_sample: spec.bits_per_sample,
        is_float: spec.sample_format == SampleFormat::Float,
        channel_mask: None,
    };
    let buffer = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 16) => {
            PcmBuffer::Int16(reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?)
        }
        (SampleFormat::Int, 24) => {
            PcmBuffer::Int24(reader.samples::<i32>().collect::<Result<Vec<_>, _>>()?)
        }
        (SampleFormat::Float, 32) => {
            PcmBuffer::Float32(reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?)
        }
        (_, bits) => return Err(WavError::UnsupportedDepth(bits)),
    };
    Ok((format, buffer))
}

/// WAV 里的 chunk 布局（fourcc + 负载长度, 按文件顺序）。
///
/// 用 [`crate::rf64::chunk_order`] 与 [`crate::rf64::chunk_lengths`] 取得,
/// 于是"普通 WAV 的 chunk 顺序与长度"这条判据**不依赖 hound 的解析器**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkLayout {
    /// `(fourcc, 负载长度)`。
    pub entries: Vec<([u8; 4], usize)>,
}

impl ChunkLayout {
    /// 顺序化的 fourcc 列表。
    #[must_use]
    pub fn order(&self) -> Vec<[u8; 4]> {
        self.entries.iter().map(|(fourcc, _)| *fourcc).collect()
    }

    /// 某个 chunk 的负载长度。
    #[must_use]
    pub fn len_of(&self, fourcc: &[u8; 4]) -> Option<usize> {
        self.entries
            .iter()
            .find(|(id, _)| id == fourcc)
            .map(|(_, len)| *len)
    }

    /// 从一个完整 WAV 的字节里读出 chunk 布局。
    ///
    /// # Errors
    ///
    /// 容器结构非法（转发 [`crate::rf64::parse_container`] 的错误）。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::rf64::Rf64Error> {
        let parsed = crate::rf64::parse_container(bytes)?;
        Ok(Self {
            entries: parsed
                .chunks
                .iter()
                .map(|chunk| (chunk.fourcc, chunk.payload_len))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dither::quantize;
    use crate::rng::DeterministicDitherRng;

    fn ramp(len: usize) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let t = index as f32 / len as f32;
                t * 1.6 - 0.8
            })
            .collect()
    }

    /// 判据 1: `hound` 能读回本 crate 写出的普通 RIFF WAV（逐样本一致）。
    #[test]
    fn hound_reads_the_wav_written_by_our_rf64_module() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("ours.wav");

        let input = ramp(512);
        let mut rng = DeterministicDitherRng::new(0x1234_5678);
        let pcm = quantize(&input, BitDepth::Int24, &mut rng);
        let format = PcmFormat::integer(2, 48_000, 24);
        let payload = pcm.to_le_bytes();

        let plan = crate::rf64::ContainerPlan::for_payload(
            crate::rf64::ContainerKind::Riff,
            format,
            512 * 2 * 3,
            512,
            None,
        );
        let mut file = std::fs::File::create(&path).expect("创建文件");
        crate::rf64::write_container(&mut file, &plan, &payload).expect("写入");
        drop(file);

        let (read_format, read_pcm) = read_plain_wav(&path).expect("hound 读回");
        assert_eq!(read_format.channels, 2);
        assert_eq!(read_format.sample_rate, 48_000);
        assert_eq!(read_format.bits_per_sample, 24);
        assert!(!read_format.is_float);
        assert_eq!(read_pcm, pcm, "hound 读到的样本必须与我们写的一致");
    }

    /// 判据 2: 本 crate 的读取器能解析 `hound` 写出的 WAV, 且 `fmt `/`data` 长度正确。
    #[test]
    fn our_reader_parses_the_wav_written_by_hound() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("hound16.wav");

        let samples: Vec<f32> = ramp(64);
        let pcm = PcmBuffer::Int16(
            samples
                .iter()
                .map(|&sample| (sample * 32_768.0).clamp(-32_768.0, 32_767.0) as i16)
                .collect(),
        );
        let format = PcmFormat::integer(1, 44_100, 16);
        write_plain_wav(&path, &format, &pcm).expect("hound 写入");

        let bytes = std::fs::read(&path).expect("读文件");
        let parsed = crate::rf64::parse_container(&bytes).expect("我校的读取器解析");
        assert_eq!(parsed.kind, crate::rf64::ContainerKind::Riff);
        assert_eq!(parsed.format.channels, 1);
        assert_eq!(parsed.format.sample_rate, 44_100);
        assert_eq!(parsed.format.bits_per_sample, 16);
        assert_eq!(&bytes[parsed.data.clone()], pcm.to_le_bytes().as_slice());

        let layout = ChunkLayout::from_bytes(&bytes).expect("chunk 布局");
        assert_eq!(
            layout.len_of(b"fmt "),
            Some(16),
            "单声道 16-bit 用 16 字节 fmt"
        );
        assert_eq!(layout.len_of(b"data"), Some(64 * 2));
        assert!(layout.order().contains(&*b"data"));
    }

    /// 判据 3: 16 / 24 / 32f 三种位深经 `hound` 往返都逐位一致。
    #[test]
    fn every_bit_depth_round_trips_through_hound() {
        let directory = tempfile::tempdir().expect("临时目录");
        let input = ramp(256);
        let cases: [(BitDepth, PcmFormat); 3] = [
            (BitDepth::Int16, PcmFormat::integer(2, 48_000, 16)),
            (BitDepth::Int24, PcmFormat::integer(2, 48_000, 24)),
            (BitDepth::Float32, PcmFormat::float(2, 48_000, 32)),
        ];
        for (depth, format) in cases {
            let path = directory.path().join(format!("depth{}.wav", depth.bits()));
            let mut rng = DeterministicDitherRng::new(9);
            let pcm = quantize(&input, depth, &mut rng);
            write_plain_wav(&path, &format, &pcm).expect("hound 写入");
            let (read_format, read_pcm) = read_plain_wav(&path).expect("hound 读回");
            assert_eq!(read_format, format, "{depth:?} 的格式应该一致");
            assert_eq!(read_pcm, pcm, "{depth:?} 的样本应该逐位一致");
        }
    }

    /// 判据 4: 6 声道 `WAVE_FORMAT_EXTENSIBLE` 能被独立的第三方读取器接受。
    /// 这条把 `rf64` 的 EXTENSIBLE 头部（40 字节 `fmt `、`cbSize` 22、标准 GUID）
    /// 交给 `hound` 当裁判。
    #[test]
    fn hound_accepts_our_extensible_six_channel_header() {
        let directory = tempfile::tempdir().expect("临时目录");
        let format = PcmFormat {
            channel_mask: Some(0x3F),
            ..PcmFormat::float(6, 48_000, 32)
        };
        let payload = vec![0u8; 6 * 4 * 8];
        // `hound` 不支持 RF64 顶层 fourcc, 因此把同一份 `fmt ` 负载放进 RIFF 容器
        // 再交给它 —— 被验证的是 EXTENSIBLE 头部本身。
        let riff_path = directory.path().join("surround.riff.wav");
        let riff_plan = crate::rf64::ContainerPlan::for_payload(
            crate::rf64::ContainerKind::Riff,
            format,
            payload.len() as u64,
            8,
            None,
        );
        let mut riff_file = std::fs::File::create(&riff_path).expect("创建文件");
        crate::rf64::write_container(&mut riff_file, &riff_plan, &payload).expect("写入");
        drop(riff_file);

        let (read_format, read_pcm) = read_plain_wav(&riff_path).expect("hound 读回");
        assert_eq!(read_format.channels, 6);
        assert_eq!(read_format.bits_per_sample, 32);
        assert!(read_format.is_float);
        assert_eq!(read_pcm.len(), 48);
        assert!(read_pcm.to_le_bytes().iter().all(|&byte| byte == 0));
    }

    /// 判据 5: 格式与缓冲不匹配、以及不支持的位深都被明确拒绝。
    #[test]
    fn mismatched_format_and_buffer_are_rejected() {
        let format = PcmFormat::integer(2, 48_000, 16);
        let buffer = PcmBuffer::Int24(vec![0, 1, 2]);
        assert!(matches!(
            check_match(&format, &buffer),
            Err(WavError::FormatMismatch { .. })
        ));

        let float = PcmFormat::float(2, 48_000, 32);
        assert!(matches!(
            check_match(&float, &buffer),
            Err(WavError::FormatMismatch { .. })
        ));

        let odd = PcmFormat::integer(1, 48_000, 8);
        assert_eq!(hound_spec(&odd), Err(WavError::UnsupportedDepth(8)));

        let directory = tempfile::tempdir().expect("临时目录");
        assert!(write_plain_wav(directory.path().join("x.wav"), &format, &buffer).is_err());
    }

    /// 判据 6: 空缓冲也能往返（0 帧文件是合法的）。
    #[test]
    fn empty_buffer_round_trips() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("empty.wav");
        let format = PcmFormat::integer(1, 48_000, 16);
        let pcm = PcmBuffer::Int16(Vec::new());
        write_plain_wav(&path, &format, &pcm).expect("写入");
        let (read_format, read_pcm) = read_plain_wav(&path).expect("读回");
        assert_eq!(read_format, format);
        assert!(read_pcm.is_empty());
    }

    /// 判据 7: `format_of` 与 `check_match` 互相自洽（否则判据 5 会自相矛盾）。
    #[test]
    fn format_of_agrees_with_check_match() {
        let buffer = PcmBuffer::Int24(vec![0; 6]);
        let format = format_of(&buffer, 2, 48_000);
        assert!(check_match(&format, &buffer).is_ok());
        assert_eq!(format.block_align(), 6);
    }
}
