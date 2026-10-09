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
    /// 格式整体不可用：声道数为 0、采样率为 0，或缓冲里有越界样本。
    RejectedFormat {
        /// 被拒绝的字段或量（人读）。
        field: &'static str,
        /// 被拒绝的值或原因（人读）。
        detail: String,
    },
}

impl core::fmt::Display for WavError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Hound(message) => write!(f, "hound 失败: {message}"),
            Self::FormatMismatch { expected, got } => {
                write!(f, "格式不匹配: 期望 {expected}, 实际 {got}")
            }
            Self::UnsupportedDepth(bits) => write!(f, "不支持的位深: {bits}"),
            Self::RejectedFormat { field, detail } => write!(f, "格式不可用: {field} = {detail}"),
        }
    }
}

impl std::error::Error for WavError {}

impl From<hound::Error> for WavError {
    fn from(error: hound::Error) -> Self {
        Self::Hound(error.to_string())
    }
}

/// 校验格式的**容器级**约束：`hound` 只接受非零声道数与非零采样率。
///
/// # 为什么必须由这里挡（实测，两处都是 `hound` 的 panic，不是 `Result`）
///
/// - `channels == 0` ⇒ hound 的 `WavWriter::update_header` **现位于第 502 行**做
///   `data_bytes_written % spec.channels`，除数为 0 ⇒ **panic**。本机实测：
///   `check_match` 与 `hound_spec` 都返回 `Ok`，`write_plain_wav` 在被拒之前已经
///   建好文件（实测留下 68 字节的残缺头）。
/// - `sample_rate == 0` ⇒ hound 写 `nBlockAlign` 时 **现位于第 332 行**做
///   `bytes_per_sec / spec.sample_rate`，除数为 0 ⇒ **panic**（空缓冲也一样）。
///
/// 因此这一层必须在**创建文件之前**说"不"：`write_plain_wav` 拒绝时**不得**留下
/// 任何字节，这与 [`crate::rf64`] 那条"绝不产出声明了不存在音频的容器"同一条纪律。
///
/// # Errors
///
/// 声道数为 0、采样率为 0，或位深不在 16 / 24 / 32。
fn check_container_fields(format: &PcmFormat) -> Result<(), WavError> {
    if format.channels == 0 {
        return Err(WavError::RejectedFormat {
            field: "channels",
            detail: "0".into(),
        });
    }
    if format.sample_rate == 0 {
        return Err(WavError::RejectedFormat {
            field: "sample_rate",
            detail: "0".into(),
        });
    }
    if !matches!(format.bits_per_sample, 16 | 24 | 32) {
        return Err(WavError::UnsupportedDepth(format.bits_per_sample));
    }
    Ok(())
}

/// 把本 crate 的 [`PcmFormat`] 转成 `hound::WavSpec`。
///
/// # Errors
///
/// `bits_per_sample` 不是 16 / 24 / 32 时返回 [`WavError::UnsupportedDepth`];
/// 声道数或采样率为 0 时返回 [`WavError::RejectedFormat`] —— 这两个值会让 `hound`
/// 的写入器**panic**（见 [`check_container_fields`]），不是 `Result`。
pub fn hound_spec(format: &PcmFormat) -> Result<WavSpec, WavError> {
    check_container_fields(format)?;
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
/// 三项一起查，因此**任一**不匹配时返回的都是一个明确的错误，不会走到 `hound`：
///
/// 1. 容器字段可用（声道数 ≠ 0、采样率 ≠ 0、位深 ∈ {16, 24, 32}，
///    [`WavError::RejectedFormat`] / [`WavError::UnsupportedDepth`]）；
/// 2. 位深与整数/浮点类别一致（[`WavError::FormatMismatch`]）；
/// 3. 缓冲里的每个样本都在自己位深的合法区间里（[`WavError::RejectedFormat`]）。
///
/// 注意顺序：容器字段在**位深匹配之前**查。理由是位深先查会让
/// `channels == 0` + 位深不匹配的组合报出"位深"这个**次要**原因，而真正会让
/// `hound` panic 的是声道数。判据 `a_zero_channel_count_is_refused_before_any_file_exists`
/// 直读这一点。
///
/// 第 3 项的来源是 [`PcmBuffer::is_in_range`]：整数变体是公开的，`Int24` 里可以装
/// 装不下 24 位的值，而越界样本会被[`PcmBuffer::to_le_bytes`]静默丢高位。
///
/// # Errors
///
/// 见上面三条。
pub fn check_match(format: &PcmFormat, buffer: &PcmBuffer) -> Result<(), WavError> {
    check_container_fields(format)?;
    let expected_depth = buffer.depth();
    if format.bits_per_sample != expected_depth.bits()
        || format.is_float != matches!(expected_depth, BitDepth::Float32)
    {
        let expected_bits = expected_depth.bits();
        let actual_bits = format.bits_per_sample;
        let actual_kind = if format.is_float { "浮点" } else { "整数" };
        return Err(WavError::FormatMismatch {
            expected: format!("{expected_depth:?} ({expected_bits} 位)"),
            got: format!("{actual_bits} 位, {actual_kind}"),
        });
    }
    if !buffer.is_in_range() {
        return Err(WavError::RejectedFormat {
            field: "samples",
            detail: format!(
                "越界样本: {:?} 的合法区间是 [{}, {}]",
                expected_depth,
                expected_depth.full_scale_min(),
                expected_depth.full_scale_max()
            ),
        });
    }
    Ok(())
}

/// 用 `hound` 写一份普通 RIFF WAV。
///
/// **拒绝时不留字节**：全部校验（格式、容器字段、样本范围）都在
/// [`WavWriter::create`] 之前完成，因此一个被拒的缓冲不会留下残缺文件。
/// 这条纪律的来源是实测：此前越界样本会在 `hound` 的第 3 个样本处报错，
/// 而文件已经存在（本机实测 68 字节的 `RIFF`/`fmt `/`data` 头）。
///
/// # Errors
///
/// 格式与缓冲不匹配、容器字段不可用、有越界样本、位深不受支持，或 `hound` I/O 失败。
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
/// 位深不受支持、**读到的格式整体不可用（声道数为 0 / 采样率为 0 / 有越界样本）**，
/// 或 `hound` I/O 失败。第三类不是多余的：读回的值来自**外部文件的内容**，
/// 不校验就等于把"文件说了什么"当成自己的数据。这条与 [`check_match`] 同源。
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
    check_container_fields(&format)?;
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
    if !buffer.is_in_range() {
        return Err(WavError::RejectedFormat {
            field: "samples",
            detail: format!("读到的样本越界: {:?}", buffer.depth()),
        });
    }
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
            payload.len() as u64,
            (input.len() / 2) as u64,
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
        assert!(layout.order().contains(b"data"));
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
            let depth_bits = depth.bits();
            let path = directory.path().join(format!("depth{depth_bits}.wav"));
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
    /// 判据 4b: 我们写出的**浮点** RIFF 现在带 `fact` chunk（非 PCM 的必填项,
    /// 见 [`crate::rf64`] 模块头的核验表）, 而独立的第三方读取器 `hound` **仍然**
    /// 能逐位读回 —— 这是"新增的 chunk 没有把文件读坏"的第三方见证。
    ///
    /// 同时本 crate 的读取器必须报告 `fact` 的载荷长度 = 4 字节（`u32` 帧数）。
    ///
    /// **两份写入器在这里刻意不同**（如实登记）: [`write_plain_wav`] 走 `hound`,
    /// 而 hound **从不写** `fact` —— 它 `src/write.rs` 的原话是 "Hound never writes a
    /// fact chunk. For all the formats that Hound can write, the fact chunk is
    /// redundant." 本仓库不改第三方实现 ⇒ 浮点 + `fact` 只由 [`crate::rf64`] 的
    /// 生产写入器提供; 本判据覆盖的也是那一条路径。
    #[test]
    fn hound_reads_the_float_riff_that_carries_our_fact_chunk() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("float-fact.wav");

        let format = PcmFormat::float(2, 48_000, 32);
        let samples = ramp(64);
        let payload: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frames = (samples.len() / 2) as u64;
        let plan = crate::rf64::ContainerPlan::for_payload(
            crate::rf64::ContainerKind::Riff,
            format,
            payload.len() as u64,
            frames,
            None,
        );
        let mut file = std::fs::File::create(&path).expect("创建文件");
        crate::rf64::write_container(&mut file, &plan, &payload).expect("写入");
        drop(file);

        // 第三方裁判: `hound` 读回的格式与样本必须逐位一致。
        let (read_format, read_pcm) = read_plain_wav(&path).expect("hound 读回");
        assert_eq!(read_format, format);
        assert_eq!(read_pcm, PcmBuffer::Float32(samples));

        // 本 crate 的读取器: `fact` 必须存在, 长度是 4 字节。
        let bytes = std::fs::read(&path).expect("读文件");
        let layout = ChunkLayout::from_bytes(&bytes).expect("chunk 布局");
        assert_eq!(layout.len_of(b"fact"), Some(crate::rf64::FACT_PAYLOAD_LEN));
    }

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
        let rejected = directory.path().join("x.wav");
        assert!(write_plain_wav(&rejected, &format, &buffer).is_err());
        assert!(
            !rejected.exists(),
            "被拒的写入**不得**留下任何字节; 实测此前会留下 68 字节的残缺头"
        );
    }

    /// 判据 8: `PcmBuffer::is_in_range` 的读数与规范端点**逐点**一致。
    ///
    /// 这是"位深边界"那条契约的机械读数：24-bit 的合法上界是 `2^23 − 1`
    /// （[`BitDepth::full_scale_max`] 的文档），**不是** `2^23`。
    /// 判据同时把两个端点（合法）与四个越界值（不合法）都走一遍，
    /// 因此"把上界改成 `2^23`"这一类注入会让它变红。
    #[test]
    fn pcm_buffer_knows_its_legal_sample_range() {
        // 三个变体的合法端点。
        assert!(PcmBuffer::Int16(vec![i16::MIN, i16::MAX, 0]).is_in_range());
        assert!(PcmBuffer::Int24(vec![-8_388_608, 8_388_607, 0]).is_in_range());
        assert!(PcmBuffer::Float32(vec![-1.0, 1.0, 0.0, f32::MIN, f32::MAX]).is_in_range());
        assert!(PcmBuffer::Int16(Vec::new()).is_in_range(), "空缓冲合法");

        // 越界样本：24-bit 的上界再加一个 LSB。
        assert!(!PcmBuffer::Int24(vec![8_388_608]).is_in_range());
        assert!(!PcmBuffer::Int24(vec![-8_388_609]).is_in_range());
        assert!(!PcmBuffer::Int24(vec![i32::MAX]).is_in_range());
        assert!(!PcmBuffer::Int24(vec![0, 1, i32::MIN]).is_in_range());

        // 浮点的"合法"就是有限。
        assert!(!PcmBuffer::Float32(vec![f32::NAN]).is_in_range());
        assert!(!PcmBuffer::Float32(vec![0.0, f32::INFINITY]).is_in_range());
        assert!(!PcmBuffer::Float32(vec![f32::NEG_INFINITY]).is_in_range());

        // 越界值的字节读数是**丢高位**, 不是截断到合法范围 —— 这条是"为什么要拦"的
        // 字面证据: `8_388_608` 的三个字节读回来是 −8_388_608（符号翻转）。
        assert_eq!(
            PcmBuffer::Int24(vec![8_388_608]).to_le_bytes(),
            [0x00, 0x00, 0x80]
        );
        assert_eq!(
            PcmBuffer::Int24(vec![i32::MAX]).to_le_bytes(),
            [0xFF, 0xFF, 0xFF]
        );
    }

    /// 判据 9: **零声道数**与**零采样率**在创建文件之前被明确拒绝 —— 不是 panic。
    ///
    /// 注入证明：改回旧行为（`check_match` 只看位深匹配）会让这条**以 panic 结束**：
    /// hound 的 `update_header` **现位于第 502 行**对 `spec.channels` 取模（除数为 0）；
    /// 写 `nBlockAlign` 的那处 **现位于第 332 行**对 `spec.sample_rate` 做除法。
    /// 两处都是**除数为 0 的 panic**，因此这条判据不是风格检查，是崩溃闸门。
    ///
    /// 同时钉住"拒绝时不留字节"：被拒之后路径上**没有**文件。
    #[test]
    fn a_zero_channel_count_is_refused_before_any_file_exists() {
        let directory = tempfile::tempdir().expect("临时目录");
        let mono = PcmBuffer::Int16(vec![0, 1, -1, 0]);

        // 零声道：格式校验、spec 转换、写入三处都拒绝，且都没有副作用。
        let zero_channels = PcmFormat::integer(0, 48_000, 16);
        assert!(matches!(
            check_match(&zero_channels, &mono),
            Err(WavError::RejectedFormat {
                field: "channels",
                ..
            })
        ));
        assert!(matches!(
            hound_spec(&zero_channels),
            Err(WavError::RejectedFormat {
                field: "channels",
                ..
            })
        ));
        let no_channels_path = directory.path().join("zero_channels.wav");
        assert!(write_plain_wav(&no_channels_path, &zero_channels, &mono).is_err());
        assert!(!no_channels_path.exists(), "被拒时不得留下文件");

        // 零采样率：空缓冲也一样（旧行为在空缓冲上也 panic）。
        let zero_rate = PcmFormat::integer(1, 0, 16);
        assert!(matches!(
            check_match(&zero_rate, &mono),
            Err(WavError::RejectedFormat {
                field: "sample_rate",
                ..
            })
        ));
        assert!(matches!(
            hound_spec(&zero_rate),
            Err(WavError::RejectedFormat {
                field: "sample_rate",
                ..
            })
        ));
        let no_rate_path = directory.path().join("zero_rate.wav");
        assert!(write_plain_wav(&no_rate_path, &zero_rate, &mono).is_err());
        assert!(
            write_plain_wav(
                directory.path().join("zero_rate_empty.wav"),
                &zero_rate,
                &PcmBuffer::Int16(Vec::new())
            )
            .is_err()
        );
        assert!(!no_rate_path.exists(), "被拒时不得留下文件");

        // 越界样本也走同一个"留不下文件"的出口。
        let over = PcmBuffer::Int24(vec![0, 8_388_608]);
        let int24 = PcmFormat::integer(1, 48_000, 24);
        assert!(matches!(
            check_match(&int24, &over),
            Err(WavError::RejectedFormat {
                field: "samples",
                ..
            })
        ));
        let over_path = directory.path().join("over.wav");
        assert!(write_plain_wav(&over_path, &int24, &over).is_err());
        assert!(!over_path.exists(), "越界样本被拒时不得留下文件");

        // 合法端点必须仍然写得进去（否则这条判据只是把功能关掉）。
        let edge = PcmBuffer::Int24(vec![8_388_607, -8_388_608]);
        let edge_path = directory.path().join("edge.wav");
        write_plain_wav(&edge_path, &int24, &edge).expect("合法端点必须能写");
        let (read_format, read_pcm) = read_plain_wav(&edge_path).expect("读回");
        assert_eq!(read_format, int24);
        assert_eq!(read_pcm, edge);
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

    /// 判据 (**类别 6: 缓冲变体 / 类别一致性**): [`format_of`] 对**三个**缓冲变体都反推出
    /// 同类别的格式 —— 浮点缓冲必须给出 `is_float = true` 的格式, 否则它自己反推出来的
    /// 格式会被自己的 [`check_match`] 拒绝。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 `BitDepth::Float32 => PcmFormat::float(channels, sample_rate, 32)` 注入成
    /// `PcmFormat::integer(channels, sample_rate, 32)` 时, 全量
    /// `cargo test -p yeban-render` **全绿**（`test result: ok. 176 passed; 0 failed`）——
    /// 既有的 `format_of_agrees_with_check_match` 只走了 `Int24` 这**一个**变体,
    /// 而 `mismatched_format_and_buffer_are_rejected` 里的浮点格式是**手写**的,
    /// 不经过 `format_of`。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 三个变体各一份**合法**缓冲（`Int16` / `Int24` / `Float32`, 声道数 2,
    /// 采样率 48 kHz）。单位: 位深是**位/样本**。读数: `format_of` 的
    /// `bits_per_sample` 与 `is_float`, 以及 `check_match` 的判决。
    ///
    /// # 非空证明
    ///
    /// 三个变体覆盖了 `is_float` 的**两个**取值（`false` × 2 与 `true` × 1）——
    /// 只断言 `Int16`/`Int24` 的话, `is_float` 这一位根本没有判别力。
    #[test]
    fn format_of_covers_every_buffer_variant() {
        let cases = [
            (PcmBuffer::Int16(vec![0, 1]), 16u16, false),
            (PcmBuffer::Int24(vec![0, 1]), 24, false),
            (PcmBuffer::Float32(vec![0.0, 1.0]), 32, true),
        ];
        let mut float_cases = 0usize;
        for (buffer, bits, is_float) in cases {
            let format = format_of(&buffer, 2, 48_000);
            assert_eq!(format.channels, 2);
            assert_eq!(format.sample_rate, 48_000);
            assert_eq!(
                format.bits_per_sample, bits,
                "{buffer:?}: format_of 反推的位深"
            );
            assert_eq!(
                format.is_float, is_float,
                "{buffer:?}: 类别必须与缓冲的变体一致"
            );
            assert!(
                check_match(&format, &buffer).is_ok(),
                "{buffer:?}: format_of 与 check_match 必须自洽"
            );
            if is_float {
                float_cases += 1;
            }
        }
        assert!(float_cases > 0, "必须至少覆盖一个浮点变体");
    }

    /// 判据 10（**跨模块**）: 同一个 `PcmFormat` 在本 crate 的**两条写入器**上必须得到
    /// 同一个判决 —— 这里取 `bits_per_sample == 0` 这一格。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: **一个**格式 `channels = 2, sample_rate = 48_000, bits_per_sample = 0`
    /// （整数 PCM），分别交给 ① 本模块走 `hound` 的 [`write_plain_wav`],
    /// ② [`crate::rf64::ContainerPlan`] + [`crate::rf64::write_container`]。
    /// 单位: 判决（接受 / 拒绝）与 `out` / 磁盘上的**字节数**（都必须是 0）。
    ///
    /// # 为什么这条不属于本模块
    ///
    /// 本模块的拒绝理由在 [`check_container_fields`]（`bits ∉ {16, 24, 32}` ⇒
    /// [`WavError::UnsupportedDepth`]）, 而 0 位深在**另一条**写入器上修复前是**接受**的:
    /// `PcmFormat::bytes_per_sample` 对 0 位给 0, 于是那份 `fmt ` 声明
    /// `nBlockAlign = 0` / `nAvgBytesPerSec = 0` —— 合规解码器按 `nBlockAlign` 求帧数就是
    /// 除零。两条写入器对同一个显然不可用的格式给出两个判决, 与 `sample_rate == 0`
    /// 那一格是同一条纪律（那一格由 `crate::rf64::ContainerPlan::validate` 补上, 判据
    /// `crate::rf64::tests::a_zero_sample_rate_is_refused_before_any_byte`）。
    ///
    /// 容器侧的完整判据在 `crate::rf64::tests::a_zero_bit_depth_is_refused_before_any_byte`;
    /// 本判据只钉"两条写入器一致"这一个读数, 因此它**不**重复那边的网格。
    #[test]
    fn the_two_writers_in_this_crate_agree_on_a_zero_bit_depth() {
        let zero_bits = PcmFormat::integer(2, 48_000, 0);
        let buffer = PcmBuffer::Int16(vec![0, 1, -1, 0]);

        // ① 走 `hound` 的写入器: 拒绝, 且理由是位深（不是别的字段）。
        assert_eq!(
            hound_spec(&zero_bits),
            Err(WavError::UnsupportedDepth(0)),
            "0 位深必须由 check_container_fields 拒绝"
        );
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("zero_bits.wav");
        assert!(write_plain_wav(&path, &zero_bits, &buffer).is_err());
        assert!(!path.exists(), "被拒的写入不得留下文件（实测零字节）");

        // ② 自研容器写入器: 必须给出**同一个**判决（修复前是 `Ok` + 写出 52 字节）。
        let data = buffer.to_le_bytes();
        let plan = crate::rf64::ContainerPlan::for_payload(
            crate::rf64::ContainerKind::Riff,
            zero_bits,
            data.len() as u64,
            (data.len() / 4) as u64,
            None,
        );
        assert_eq!(
            plan.validate(),
            Err(crate::rf64::Rf64Error::ZeroBitsPerSample),
            "两条写入器必须对 0 位深给出同一个判决"
        );
        let mut file = Vec::new();
        assert_eq!(
            crate::rf64::write_container(&mut file, &plan, &data),
            Err(crate::rf64::Rf64Error::ZeroBitsPerSample)
        );
        assert!(file.is_empty(), "被拒的写入不得留下任何字节");

        // 防空判据: 只把位深换成 16, **同一个**声道布局/采样率/负载必须两条路都通。
        let good = PcmFormat::integer(2, 48_000, 16);
        let good_path = directory.path().join("good.wav");
        write_plain_wav(&good_path, &good, &buffer).expect("16 位必须写得出去");
        let (read_format, read_pcm) = read_plain_wav(&good_path).expect("读回");
        assert_eq!(read_format, good);
        assert_eq!(read_pcm, buffer);
        let plan = crate::rf64::ContainerPlan::for_payload(
            crate::rf64::ContainerKind::Riff,
            good,
            data.len() as u64,
            (data.len() / 4) as u64,
            None,
        );
        assert_eq!(plan.validate(), Ok(()), "16 位必须可写");
    }

    /// 判据 11（**读取器侧**）：声明了 0 采样率的外部文件必须被 [`read_plain_wav`] 拒绝。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一份**合法写出**的 16-bit 单声道 WAV, 只把 `fmt ` 负载里的两个字段改成 0 ——
    /// `nSamplesPerSec`（负载偏移 4, 单位 Hz）与 `nAvgBytesPerSec`（负载偏移 8, 单位
    /// 字节/秒）。两个都要改: `hound` 的读取器校验
    /// `nAvgBytesPerSec == nBlockAlign × nSamplesPerSec`, 只改一个会先被它拒
    /// （实测是 `Error::FormatError("inconsistent fmt chunk")`）, 那就测不到本模块的判决。
    /// 偏移从**同一份合法文件**的 chunk 表里取, 不写死常量。单位: 偏移是字节。
    ///
    /// # 为什么这一条不能由写入器侧的判据代替
    ///
    /// 读回的值来自**外部文件的内容**（[`read_plain_wav`] 的文档写明这一点）。写入器侧的
    /// `a_zero_channel_count_is_refused_before_any_file_exists` 只证明"我们不写这种文件",
    /// 证明不了"别人写的这种文件我们拒绝"。
    ///
    /// 注入证明（本机实测）: 删掉 [`read_plain_wav`] 里那句 `check_container_fields(&format)?`
    /// 之后本轮 90 次注入里的**这一次全绿**：当时全量 192 条判据（lib 168 ＋ 集成 10 ＋ 13 ＋
    /// doc 1）的 `test result` 全是 `ok`
    /// —— 读取器会把 `sample_rate = 0` 的容器当成合法读数交出去。本判据在那条注入下变红。
    #[test]
    fn the_reader_refuses_a_file_that_declares_a_zero_sample_rate() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("zero_rate_from_disk.wav");
        let format = PcmFormat::integer(1, 48_000, 16);
        write_plain_wav(&path, &format, &PcmBuffer::Int16(vec![0, 1, -1, 0]))
            .expect("先写一份合法文件");

        let mut bytes = std::fs::read(&path).expect("读文件");
        let parsed = crate::rf64::parse_container(&bytes).expect("自研读取器解析");
        let fmt = parsed
            .chunks
            .iter()
            .find(|chunk| &chunk.fourcc == b"fmt ")
            .expect("容器必须有 fmt ");
        let rate = fmt.payload_offset + 4;
        let byte_rate = fmt.payload_offset + 8;
        bytes[rate..rate + 4].copy_from_slice(&0u32.to_le_bytes());
        bytes[byte_rate..byte_rate + 4].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(&path, &bytes).expect("写回");

        // 前提自证: 这份文件真的被独立的第三方读取器接受了 —— 否则测到的是它的拒绝。
        assert!(
            hound::WavReader::open(&path).is_ok(),
            "本判据要的是'hound 接受、我们拒绝'这一格"
        );
        assert!(
            matches!(
                read_plain_wav(&path),
                Err(WavError::RejectedFormat {
                    field: "sample_rate",
                    ..
                })
            ),
            "读取器必须拒绝声明 0 采样率的外部文件"
        );
    }

    /// 判据 12（**读取器侧**）：从外部文件读到的**非有限浮点样本**必须被拒绝。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一份 32-bit 浮点单声道 WAV, 载荷里第 2 个样本的位型被改成 `NaN`
    /// （`0x7FC0_0000`）。单位: 样本是 `f32` 位型, 偏移是字节。
    ///
    /// # 为什么这一格可达且必须拦住
    ///
    /// [`PcmBuffer::is_in_range`] 对浮点的定义就是**有限**（`NaN` / `±inf` 非法）。
    /// 整数变体由元素宽度自己保证落在合法区间里, 因此"读到的样本越界"这条拒绝
    /// **只可能由浮点文件触发** —— 它是那条判据唯一的可达入口。
    ///
    /// 注入证明（本机实测）: 删掉 [`read_plain_wav`] 里那句 `if !buffer.is_in_range()`
    /// 之后本轮注入里的这一次**全绿**（192 条判据无一变红）; 本判据在那条注入下变红。
    #[test]
    fn the_reader_refuses_a_file_that_carries_a_non_finite_float_sample() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("nan_from_disk.wav");
        let format = PcmFormat::float(1, 48_000, 32);
        let payload_samples = [0.25f32, 0.5, -0.75, 1.0];
        let mut payload: Vec<u8> = payload_samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        // 把第 2 个样本改成 `NaN`（IEEE-754 单精度静默 NaN 的规范位型）。
        payload[4..8].copy_from_slice(&0x7FC0_0000u32.to_le_bytes());
        let plan = crate::rf64::ContainerPlan::for_payload(
            crate::rf64::ContainerKind::Riff,
            format,
            payload.len() as u64,
            payload_samples.len() as u64,
            None,
        );
        let mut file = std::fs::File::create(&path).expect("创建文件");
        crate::rf64::write_container(&mut file, &plan, &payload).expect("写入");
        drop(file);

        // 前提自证: 容器本身是合法的（第三方读取器能打开）, 被拒的是那一格样本。
        let mut reader = hound::WavReader::open(&path).expect("hound 必须能打开这份容器");
        let raw: Vec<f32> = reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .expect("读样本");
        assert!(raw[1].is_nan(), "前提: 文件里那个样本真的是 NaN");
        assert!(
            matches!(
                read_plain_wav(&path),
                Err(WavError::RejectedFormat {
                    field: "samples",
                    ..
                })
            ),
            "读取器必须拒绝携带 NaN 的外部文件"
        );
    }

    /// 判据 13：`format_of` 对**三个**位深都必须给出通过 [`check_match`] 的格式。
    ///
    /// # 为什么需要单独一条
    ///
    /// `format_of_agrees_with_check_match` 只用 `Int24` 一个缓冲。把 `Float32` 那一支
    /// 改写成整数格式之后它仍然全绿（本机实测: 注入那次全量 192 条判据的 `test result` 都是 `ok`）——
    /// 而那时 [`format_of`] 给出的格式会被同一个 [`check_match`] 拒绝, 也就是
    /// "由缓冲反推的格式与校验器自相矛盾"。
    ///
    /// 本判据对三个位深各跑一遍 `format_of` → `check_match` 的往返, 因此三支都必须自洽。
    #[test]
    fn format_of_pairs_every_depth_with_the_checker() {
        let channels = 2u16;
        let sample_rate = 48_000u32;
        let mut rng = DeterministicDitherRng::new(0x1234_5678);
        let samples = [0.0f32, 0.25, -0.25, 0.5, -0.5, 1.0];
        for depth in [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32] {
            let buffer = quantize(&samples, depth, &mut rng);
            let format = format_of(&buffer, channels, sample_rate);
            assert_eq!(
                format.bits_per_sample,
                depth.bits(),
                "{depth:?}: format_of 必须报告缓冲自己的位深"
            );
            assert_eq!(
                format.is_float,
                matches!(depth, BitDepth::Float32),
                "{depth:?}: format_of 必须报告缓冲自己的类别"
            );
            assert!(
                check_match(&format, &buffer).is_ok(),
                "{depth:?}: format_of 的产物必须自己能过 check_match"
            );
        }
    }

    /// 判据 14：**同一个位深、不同类别**的格式与缓冲必须被拒 —— 判定的两半各自有判别力。
    ///
    /// # 为什么需要单独一条
    ///
    /// `mismatched_format_and_buffer_are_rejected` 里两个格式的**位深都与缓冲不同**
    /// （16 对 24、32 对 24）, 因此它只钉住"位深那一半"; 把类别那一半（`is_float`）
    /// **整条删掉**之后它仍然全绿（本机实测: 注入那次全量 192 条判据无一变红）。
    /// 这一格是能让两半分离的输入: `PcmFormat::integer(2, 48_000, 32)` 与
    /// `PcmBuffer::Float32` 的位深都是 32, 只有类别不同。
    ///
    /// 反向（浮点格式 + 32 位**整数**缓冲）在本 crate 的 [`PcmBuffer`] 里**不可达**
    /// （没有 `Int32` 变体）, 因此本判据只钉可达的那一个方向, 并如实说明。
    #[test]
    fn a_thirty_two_bit_integer_format_does_not_accept_a_float_buffer() {
        let int32 = PcmFormat::integer(2, 48_000, 32);
        let buffer = PcmBuffer::Float32(vec![0.0, 0.5, -0.5, 1.0]);
        assert_eq!(
            int32.bits_per_sample,
            buffer.depth().bits(),
            "本判据的前提是'两边的位深相同', 只有类别不同"
        );
        assert!(
            matches!(
                check_match(&int32, &buffer),
                Err(WavError::FormatMismatch { .. })
            ),
            "32 位整数格式不得接受浮点缓冲"
        );
        // 防空判据: 换成浮点格式, 同一个缓冲必须通过。
        let float32 = PcmFormat::float(2, 48_000, 32);
        assert!(check_match(&float32, &buffer).is_ok());
    }
}
