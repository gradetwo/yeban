//! 测试夹具构造器：**用代码生成最小 WAV / FLAC 字节**，仓库里不放任何真实音频文件。
//!
//! 为什么不用真实样本（AGENTS.md §2 红线 9 + 多线纪律）：
//! 1. 二进制样本要进 `assets/manifest.json` 登记许可与 SHA-256，成本远大于收益；
//! 2. 真实样本的字节是"黑盒"，判据只能断言"能解出来"；而构造的字节让判据能断言
//!    **每一个采样点的期望值**、每一位深、每一种声道布局。
//!
//! 本模块同样**零第三方依赖**，因此可以用 `rustc --edition 2024 --test` 单独编译执行，
//! 从而在本机就把"夹具本身是否合法"钉住（见 `docs/ledger/decode-core-notes.md` §7）。
//! 这一点很重要：如果夹具是错的，CI 上的解码失败会被误读成"解码器坏了"。

#![allow(dead_code)]

// ---------------------------------------------------------------------------
// WAV (RIFF) 夹具
// ---------------------------------------------------------------------------

/// WAV 采样格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormat {
    /// 整数 PCM（8-bit 无符号，16/24/32-bit 小端二进制补码）。
    Integer,
    /// IEEE 754 单精度浮点。
    Float,
}

/// WAV 夹具参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavSpec {
    /// 声道数。
    pub channels: u16,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 每个样本的位数 (8/16/24/32)。
    pub bits: u16,
    /// 整数还是浮点。
    pub format: WavFormat,
}

impl WavSpec {
    /// 每帧字节数（`block_align`）。
    #[must_use]
    pub fn block_align(&self) -> u16 {
        self.channels * (self.bits / 8)
    }

    /// 每秒字节数（`byte_rate`）。
    #[must_use]
    pub fn byte_rate(&self) -> u32 {
        self.sample_rate * u32::from(self.block_align())
    }
}

/// 把小端字节序的原始 PCM 数据装进一个标准 44 字节头的 RIFF/WAVE 文件。
///
/// `declared_data_len` 允许故意与 `data.len()` 不一致 —— 用来构造"声明与真实长度不符"
/// 的畸形输入。
#[must_use]
pub fn wav_with_declared_len(spec: &WavSpec, data: &[u8], declared_data_len: u32) -> Vec<u8> {
    let format_tag: u16 = match spec.format {
        WavFormat::Integer => 1, // WAVE_FORMAT_PCM
        WavFormat::Float => 3,   // WAVE_FORMAT_IEEE_FLOAT
    };
    // RIFF 的块体必须按偶字节对齐: 长度为奇数的块后面要跟一个**不计入块长度**的填充字节。
    // 这不是可选的装饰 —— symphonia 的 `ChunksReader::next` 在 `consumed & 1 == 1` 时
    // 会真的去读那一个字节 (源码 `symphonia-format-riff-0.6.1/src/common.rs:75-79`),
    // 少了它就会在文件末尾吃一个 UnexpectedEof。8-bit 与 24-bit 的样本数很容易触发奇数长度,
    // 所以这里按**声明的**长度算填充。
    let pad = usize::from((declared_data_len % 2) as u8);
    let mut out = Vec::with_capacity(44 + data.len() + pad);
    out.extend_from_slice(b"RIFF");
    // RIFF 尺寸 = 4 ("WAVE") + 8 + 16 (fmt) + 8 + data + pad
    let riff_len = (4u64 + 8 + 16 + 8 + u64::from(declared_data_len) + pad as u64)
        .min(u64::from(u32::MAX)) as u32;
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format_tag.to_le_bytes());
    out.extend_from_slice(&spec.channels.to_le_bytes());
    out.extend_from_slice(&spec.sample_rate.to_le_bytes());
    out.extend_from_slice(&spec.byte_rate().to_le_bytes());
    out.extend_from_slice(&spec.block_align().to_le_bytes());
    out.extend_from_slice(&spec.bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&declared_data_len.to_le_bytes());
    out.extend_from_slice(data);
    out.resize(out.len() + pad, 0);
    out
}

/// 标准 RIFF/WAVE 文件（声明长度与真实长度一致）。
#[must_use]
pub fn wav(spec: &WavSpec, data: &[u8]) -> Vec<u8> {
    let len = u32::try_from(data.len()).expect("fixture data fits in u32");
    wav_with_declared_len(spec, data, len)
}

/// 把整数样本编码为 `bits` 位小端字节流（8-bit 为无符号偏移 128 的约定）。
#[must_use]
pub fn encode_int_samples(bits: u16, values: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * usize::from(bits / 8));
    for &v in values {
        match bits {
            8 => out.push(u8::try_from(v + 128).expect("8-bit sample in -128..=127")),
            16 => out.extend_from_slice(&i16::try_from(v).expect("fits i16").to_le_bytes()),
            24 => {
                let raw = v.to_le_bytes();
                out.extend_from_slice(&raw[..3]);
            }
            32 => out.extend_from_slice(&v.to_le_bytes()),
            other => panic!("unsupported integer bit depth {other}"),
        }
    }
    out
}

/// 把 `f32` 样本编码为小端 IEEE 754 字节流。
#[must_use]
pub fn encode_f32_samples(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// FLAC 夹具
// ---------------------------------------------------------------------------

/// FLAC 夹具参数（只覆盖 CONSTANT 子帧的最小合法子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlacSpec {
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 声道数（只支持 1 或 2 —— 独立声道编码）。
    pub channels: u16,
    /// 每样本位数（只支持 16）。
    pub bits: u16,
    /// 每帧的采样数（固定块大小流；只支持 256）。
    pub block_frames: u16,
    /// 故意把 `STREAMINFO.total_samples` 写成别的值（构造"声明时长不一致"的输入）。
    pub total_samples_override: Option<u64>,
}

impl Default for FlacSpec {
    fn default() -> Self {
        Self {
            sample_rate: 8_000,
            channels: 1,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        }
    }
}

/// 生成一个只含 CONSTANT 子帧的合法 FLAC 流。
///
/// 结构：`fLaC` + 最后一个 STREAMINFO 元数据块 + `frames` 个固定块大小帧。
/// 每个声道写 CONSTANT 子帧（值 `constant`），因此音频是直流 —— 对"位深/声道/长度"
/// 这些判据而言足够，而且字节量最小、最容易核对。
///
/// # Panics
///
/// 参数超出本夹具支持的最小合法子集时 panic（夹具错误必须在测试里立刻可见，
/// 不能悄悄生成一个非法文件让 CI 去猜）。
#[must_use]
pub fn flac_constant(spec: &FlacSpec, frames: u16, constant: i32) -> Vec<u8> {
    assert!(
        spec.channels == 1 || spec.channels == 2,
        "only 1 or 2 channels"
    );
    assert_eq!(spec.bits, 16, "fixture only packs 16-bit samples");
    assert_eq!(
        spec.block_frames, 256,
        "fixture pins the 256-sample block code"
    );
    assert!(
        frames > 0 && frames < 128,
        "frame numbers must stay single-byte UTF-8"
    );
    assert!(
        spec.sample_rate > 0 && spec.sample_rate < (1 << 20),
        "sample rate must fit the 20-bit STREAMINFO field"
    );

    let total_samples = spec
        .total_samples_override
        .unwrap_or(u64::from(frames) * u64::from(spec.block_frames));

    // 先构造全部帧，才能填 STREAMINFO 里的 min/max framesize。
    let mut encoded_frames: Vec<Vec<u8>> = Vec::new();
    for index in 0..frames {
        encoded_frames.push(flac_frame(spec, index, constant));
    }
    let min_frame = encoded_frames.iter().map(Vec::len).min().unwrap_or(0);
    let max_frame = encoded_frames.iter().map(Vec::len).max().unwrap_or(0);

    let mut out = Vec::new();
    out.extend_from_slice(b"fLaC");
    // METADATA_BLOCK_HEADER: last-block flag (1) + type 0 (STREAMINFO), 24-bit length = 34.
    out.push(0x80);
    out.extend_from_slice(&[0x00, 0x00, 34]);
    out.extend_from_slice(&stream_info(spec, total_samples, min_frame, max_frame));
    for frame in encoded_frames {
        out.extend_from_slice(&frame);
    }
    out
}

/// 单个固定块大小帧：4 字节帧头 + UTF-8 帧号 + 8 位块大小 + CRC-8 + 子帧 + CRC-16。
fn flac_frame(spec: &FlacSpec, index: u16, constant: i32) -> Vec<u8> {
    let mut header = BitWriter::new();
    header.push_bits(0b11_1111_1111_1110, 14); // sync
    header.push_bits(0, 1); // reserved
    header.push_bits(0, 1); // blocking strategy = fixed
    header.push_bits(0b0110, 4); // block size = 8-bit value - 1 at end of header
    header.push_bits(0b0000, 4); // sample rate = from STREAMINFO
    header.push_bits(u64::from(spec.channels - 1), 4); // independent channels
    header.push_bits(0b000, 3); // sample size = from STREAMINFO
    header.push_bits(0, 1); // reserved
    header.push_bits(u64::from(index), 8); // UTF-8 frame number (0..=127)
    header.push_bits(u64::from(spec.block_frames - 1), 8);
    header.align();
    let mut frame = header.into_bytes();
    frame.push(crc8(&frame));

    let mut subframes = BitWriter::new();
    for _ in 0..spec.channels {
        subframes.push_bits(0b0000_0000, 8); // zero pad + CONSTANT + no wasted bits
        subframes.push_bits(
            u64::from(constant as u32) & ((1u64 << spec.bits) - 1),
            u32::from(spec.bits),
        );
    }
    subframes.align();
    frame.extend_from_slice(&subframes.into_bytes());

    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// 34 字节 `STREAMINFO`。
fn stream_info(spec: &FlacSpec, total_samples: u64, min_frame: usize, max_frame: usize) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.push_bits(u64::from(spec.block_frames), 16); // min blocksize
    w.push_bits(u64::from(spec.block_frames), 16); // max blocksize
    w.push_bits(min_frame as u64, 24); // min framesize
    w.push_bits(max_frame as u64, 24); // max framesize
    w.push_bits(u64::from(spec.sample_rate), 20);
    w.push_bits(u64::from(spec.channels - 1), 3);
    w.push_bits(u64::from(spec.bits - 1), 5);
    w.push_bits(total_samples, 36);
    w.align();
    // 全零 MD5 = "未计算"（FLAC 规范的合法取值）。
    for _ in 0..16 {
        w.push_bits(0, 8);
    }
    let bytes = w.into_bytes();
    assert_eq!(bytes.len(), 34, "STREAMINFO must be exactly 34 bytes");
    bytes
}

/// FLAC 头 CRC-8：多项式 `x^8 + x^2 + x + 1`，初值 0，不反射，不异或输出。
#[must_use]
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// FLAC 帧 CRC-16：多项式 `x^16 + x^15 + x^2 + 1`，初值 0，不反射，不异或输出。
#[must_use]
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// 大端/MSB-first 的位写入器（FLAC 的字段是位打包的）。
struct BitWriter {
    bytes: Vec<u8>,
    acc: u8,
    filled: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            acc: 0,
            filled: 0,
        }
    }

    fn push_bit(&mut self, bit: bool) {
        self.acc = (self.acc << 1) | u8::from(bit);
        self.filled += 1;
        if self.filled == 8 {
            self.bytes.push(self.acc);
            self.acc = 0;
            self.filled = 0;
        }
    }

    fn push_bits(&mut self, value: u64, count: u32) {
        for shift in (0..count).rev() {
            self.push_bit((value >> shift) & 1 == 1);
        }
    }

    fn align(&mut self) {
        while self.filled != 0 {
            self.push_bit(false);
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        assert_eq!(self.filled, 0, "BitWriter used before align()");
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_algorithms_match_the_published_check_values() {
        // 判据 (夹具自检): "123456789" 的标准校验值 —— CRC-8 (poly 0x07) = 0xF4,
        // CRC-16/UMTS 即非反射 poly 0x8005 = 0xFEE8。
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    #[test]
    fn wav_fixture_is_a_well_formed_44_byte_header_riff() {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 16,
            format: WavFormat::Integer,
        };
        let data = encode_int_samples(16, &[1, -1, 2, -2]);
        let bytes = wav(&spec, &data);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(bytes.len(), 44 + data.len());
        // RIFF 尺寸 = 文件长度 - 8
        let riff_len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        assert_eq!(riff_len as usize, bytes.len() - 8);
        // fmt 块字段
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1);
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            48_000
        );
        assert_eq!(
            u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
            48_000 * 4
        );
        assert_eq!(u16::from_le_bytes([bytes[32], bytes[33]]), 4);
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16);
        let declared = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(declared as usize, data.len());
    }

    #[test]
    fn integer_fixture_range_is_pinned_so_out_of_range_data_fails_loudly_here() {
        // 判据 (夹具自检): CI 第 2 轮就是栽在这上面 —— 某个集成判据传了 `32_768`（i16 放不下），
        // 于是 panic 出现在**别的测试**的调用栈里（`testfix.rs:106` 的 `expect("fits i16")`），
        // 定位成本全落在读日志的人身上。这里把每个位深的合法区间显式钉住：
        // 越界值必须在**夹具层**就炸，而且 `#[should_panic]` 让它成为一条本机可跑的判据。
        assert_eq!(encode_int_samples(8, &[127]), vec![255]);
        assert_eq!(encode_int_samples(16, &[32_767]), vec![0xFF, 0x7F]);
        assert_eq!(encode_int_samples(16, &[-32_768]), vec![0x00, 0x80]);
        assert_eq!(encode_int_samples(24, &[8_388_607]).len(), 3);
        assert_eq!(encode_int_samples(32, &[i32::MAX]).len(), 4);
    }

    #[test]
    #[should_panic(expected = "fits i16")]
    fn sixteen_bit_fixture_rejects_a_value_that_does_not_fit_i16() {
        let _ = encode_int_samples(16, &[32_768]);
    }

    #[test]
    #[should_panic(expected = "8-bit sample in -128..=127")]
    fn eight_bit_fixture_rejects_a_value_outside_the_unsigned_offset_range() {
        let _ = encode_int_samples(8, &[128]);
    }

    #[test]
    fn wav_fixture_pads_odd_length_chunks_to_the_next_word_boundary() {
        // 判据 (夹具自检): 长度为奇数的 data 块后面必须有 1 个不计入块长度的填充字节,
        // 否则 symphonia 的 ChunksReader 会在对齐那一步读越界。
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits: 8,
            format: WavFormat::Integer,
        };
        let odd = wav(&spec, &[0x80, 0x80, 0x80]);
        assert_eq!(odd.len(), 44 + 3 + 1);
        assert_eq!(*odd.last().unwrap(), 0, "pad byte must be zero");
        let riff_len = u32::from_le_bytes([odd[4], odd[5], odd[6], odd[7]]);
        assert_eq!(riff_len as usize, odd.len() - 8);
        let declared = u32::from_le_bytes([odd[40], odd[41], odd[42], odd[43]]);
        assert_eq!(
            declared, 3,
            "the pad byte is not part of the declared length"
        );

        // 偶数长度不能再多出一个字节。
        let even = wav(&spec, &[0x80, 0x80]);
        assert_eq!(even.len(), 44 + 2);
    }

    #[test]
    fn wav_fixture_can_lie_about_its_data_length() {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits: 8,
            format: WavFormat::Integer,
        };
        let bytes = wav_with_declared_len(&spec, &[0x80, 0x80], 4_096);
        let declared = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(declared, 4_096);
        assert_eq!(bytes.len() - 44, 2);
    }

    #[test]
    fn eight_bit_integer_encoding_uses_the_unsigned_offset_convention() {
        assert_eq!(encode_int_samples(8, &[0, 127, -128]), vec![128, 255, 0]);
    }

    #[test]
    fn flac_fixture_has_a_valid_streaminfo_and_frame_chain() {
        let spec = FlacSpec {
            sample_rate: 8_000,
            channels: 1,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        };
        let bytes = flac_constant(&spec, 2, 0);
        assert_eq!(&bytes[0..4], b"fLaC");
        assert_eq!(bytes[4], 0x80, "STREAMINFO must be the last metadata block");
        assert_eq!(&bytes[5..8], &[0x00, 0x00, 34]);
        let info = &bytes[8..42];
        assert_eq!(u16::from_be_bytes([info[0], info[1]]), 256);
        assert_eq!(u16::from_be_bytes([info[2], info[3]]), 256);
        // 20-bit sample rate + 3-bit channels + 5-bit depth 位于 info[10..18]。
        let packed = u64::from_be_bytes([
            info[10], info[11], info[12], info[13], info[14], info[15], info[16], info[17],
        ]);
        assert_eq!(packed >> 44, 8_000, "sample rate");
        assert_eq!((packed >> 41) & 0b111, 0, "channels - 1");
        assert_eq!((packed >> 36) & 0b1_1111, 15, "bits per sample - 1");
        assert_eq!(packed & 0xF_FFFF_FFFF, 512, "total samples");
        // 每一帧的 CRC-8 / CRC-16 必须自洽 —— 否则 symphonia 会拒收，而失败原因
        // 会被误读成"解码器坏了"。
        let mut offset = 42;
        for _ in 0..2 {
            let header_len = 6; // 4 字节帧头 + 1 字节帧号 + 1 字节块大小
            let header = &bytes[offset..offset + header_len];
            assert_eq!(bytes[offset + header_len], crc8(header), "frame CRC-8");
            // 单声道 CONSTANT 子帧 = 1 字节头 + 16 位值 = 3 字节
            let body_len = header_len + 1 + 3;
            let body = &bytes[offset..offset + body_len];
            let crc = crc16(body);
            let stored =
                u16::from_be_bytes([bytes[offset + body_len], bytes[offset + body_len + 1]]);
            assert_eq!(stored, crc, "frame CRC-16");
            offset += body_len + 2;
        }
        assert_eq!(offset, bytes.len(), "fixture has no trailing bytes");
    }

    #[test]
    fn flac_fixture_records_a_constant_non_zero_dc_level() {
        let spec = FlacSpec {
            sample_rate: 44_100,
            channels: 2,
            bits: 16,
            block_frames: 256,
            total_samples_override: None,
        };
        let bytes = flac_constant(&spec, 1, -3);
        assert_eq!(&bytes[0..4], b"fLaC");
        // 立体声 CONSTANT：每声道 1 字节头 + 16 位值 = 3 字节 => 子帧区 6 字节。
        let header_len = 6;
        let subframe = header_len + 1; // 帧头 + CRC-8
        let body_len = subframe + 6; // 每声道 1 字节子帧头 + 16 位常量值
        let frame = &bytes[42..42 + body_len];
        assert_eq!(
            frame[subframe], 0x00,
            "subframe header: CONSTANT, 0 wasted bits"
        );
        assert_eq!(
            u16::from_be_bytes([frame[subframe + 1], frame[subframe + 2]]),
            0xFFFD
        );
    }

    #[test]
    fn flac_fixture_can_lie_about_total_samples() {
        let spec = FlacSpec {
            total_samples_override: Some(511),
            ..FlacSpec::default()
        };
        let bytes = flac_constant(&spec, 2, 0);
        let info = &bytes[8..42];
        let packed = u64::from_be_bytes([
            info[10], info[11], info[12], info[13], info[14], info[15], info[16], info[17],
        ]);
        assert_eq!(packed & 0xF_FFFF_FFFF, 511);
    }
}
