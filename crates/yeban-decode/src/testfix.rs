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

/// 在 `fmt ` **之前**插入任意个未知块，再写标准的 `fmt ` + `data`。
///
/// 存在的理由：`crate::decode` 的 RIFF 预检是一台**块网格**走查器，它的前进量一旦算错，
/// 就只在"`fmt ` 恰好是第一个块"这一种布局下还看得见 `fmt `。真实 WAV 在 `fmt ` 之前
/// 常有 `JUNK` / `LIST` / `bext` 这些块，所以"只有最小 44 字节头"的夹具**测不出**那类
/// 错位。本构造器因此允许把块放在 `fmt ` 之前，并且：
/// - 块体长度为奇数时补 1 个 RIFF 约定的填充字节（与 `wav_with_declared_len` 同规则）；
/// - RIFF 尺寸按**最终真实长度**回填（`文件长度 - 8`），因此当 `fmt ` 合法时，
///   生成的文件是上游能正常解析的合法文件（"闸门不得误拒"那一侧的对照物）。
///
/// `junk` 是 `(块标签, 块体)` 序列。`data` 是 `data` 块的块体。
#[must_use]
pub fn wav_with_chunks_before_fmt(
    spec: &WavSpec,
    junk: &[([u8; 4], &[u8])],
    data: &[u8],
) -> Vec<u8> {
    let format_tag: u16 = match spec.format {
        WavFormat::Integer => 1, // WAVE_FORMAT_PCM
        WavFormat::Float => 3,   // WAVE_FORMAT_IEEE_FLOAT
    };
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes()); // 占位，最后回填真实长度
    out.extend_from_slice(b"WAVE");
    for (tag, body) in junk {
        out.extend_from_slice(tag);
        out.extend_from_slice(
            &u32::try_from(body.len())
                .expect("junk body fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
    }
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format_tag.to_le_bytes());
    out.extend_from_slice(&spec.channels.to_le_bytes());
    out.extend_from_slice(&spec.sample_rate.to_le_bytes());
    out.extend_from_slice(&spec.byte_rate().to_le_bytes());
    out.extend_from_slice(&spec.block_align().to_le_bytes());
    out.extend_from_slice(&spec.bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(
        &u32::try_from(data.len())
            .expect("fixture data fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    let riff_len = u32::try_from(out.len() - 8).expect("fixture fits u32");
    out[4..8].copy_from_slice(&riff_len.to_le_bytes());
    out
}

/// `fmt ` 块**块体**在 [`wav_with_chunks_before_fmt`] 产物里的偏移。
///
/// 判据要靠它去改写 `num_channels`（"32769 声道"这种畸形声明不能在构造期写进
/// [`WavSpec`] —— 那会让夹具自己先溢出）。返回 `None` 表示前 12 字节不是 RIFF/WAVE。
#[must_use]
pub fn fmt_body_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let size = usize::try_from(u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]))
        .ok()?;
        if &bytes[offset..offset + 4] == b"fmt " {
            return Some(offset + 8);
        }
        offset = offset
            .checked_add(8)?
            .checked_add(size.checked_add(size & 1)?)?;
    }
    None
}

/// `WAVE_FORMAT_EXTENSIBLE`（tag `0xFFFE`）WAV 的参数：40 字节 `fmt ` 块体。
///
/// 存在的理由：上游 `read_ext_fmt` 之后调用 `fix_wave_channel_mask`，其中
/// `1u32 << channel_diff` 在 `channel_diff >= 32` 时移位溢出。这与 [`WavSpec`] 的 tag 1
/// 路径是**两处**不同的未检查算术，因此夹具必须能单独构造这条形状 —— 而
/// `num_channels = 33` 之类的声明不能写进 [`WavSpec`]（那会让 `block_align` 先回绕）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavExtensibleSpec {
    /// 声道数（`wFormatTag` 之后的第一个 `u16`）。
    pub channels: u16,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// `wBitsPerSample`。
    pub bits: u16,
    /// `wValidBitsPerSample`。
    pub valid_bits: u16,
    /// `dwChannelMask`。
    pub channel_mask: u32,
    /// `SubFormat` 的 16 字节 GUID（调用方从 `decode` 的常量取，避免两处各写一份）。
    pub sub_format: [u8; 16],
}

/// 生成一个 `WAVE_FORMAT_EXTENSIBLE` 的 RIFF/WAVE 文件：`fmt ` 声明 **40** 字节。
///
/// 结构：12 字节 RIFF 头 + `fmt ` (8 + 40) + `data` (8 + `data.len()`)；RIFF 尺寸按最终
/// 真实长度回填，`data` 长度为奇数时补 1 个填充字节（与 [`wav_with_declared_len`] 同规则）。
/// 生成的文件在"上游能解析"的形状下是合法 WAV：判据
/// `decode::tests::a_legal_extensible_wav_still_decodes` 用它做"闸门不得误拒"的对照物。
#[must_use]
pub fn wav_extensible(spec: &WavExtensibleSpec, data: &[u8]) -> Vec<u8> {
    wav_extensible_with_junk(spec, &[], data)
}

/// [`wav_extensible`] 的完整版：允许在 `fmt ` **之前**插入任意个未知块。
///
/// 为什么需要块网格上的第二个位置：本 crate 的 RIFF 预检是一台块网格走查器，`fmt ` 在
/// 别的块之后时它必须仍然看得见这块 `fmt `（否则第二处未检查移位会重新暴露）。
pub fn wav_extensible_with_junk(
    spec: &WavExtensibleSpec,
    junk: &[([u8; 4], &[u8])],
    data: &[u8],
) -> Vec<u8> {
    let bytes_per_sample = u32::from(spec.bits / 8);
    // 这两个字段上游只用于日志/推算，畸形形状下可以饱和；夹具**不**据此判任何东西。
    let block_align =
        u16::try_from(u32::from(spec.channels) * bytes_per_sample).unwrap_or(u16::MAX);
    let byte_rate =
        u32::try_from(u64::from(spec.sample_rate) * u64::from(block_align)).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes()); // 占位，最后回填真实长度
    out.extend_from_slice(b"WAVE");
    for (tag, body) in junk {
        out.extend_from_slice(tag);
        out.extend_from_slice(
            &u32::try_from(body.len())
                .expect("junk body fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
    }
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
    out.extend_from_slice(&spec.channels.to_le_bytes());
    out.extend_from_slice(&spec.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&spec.bits.to_le_bytes());
    out.extend_from_slice(&22u16.to_le_bytes()); // cbSize = sizeof(WAVEFORMATEXTENSIBLE) - 18
    out.extend_from_slice(&spec.valid_bits.to_le_bytes());
    out.extend_from_slice(&spec.channel_mask.to_le_bytes());
    out.extend_from_slice(&spec.sub_format);
    out.extend_from_slice(b"data");
    out.extend_from_slice(
        &u32::try_from(data.len())
            .expect("fixture data fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    let riff_len = u32::try_from(out.len() - 8).expect("fixture fits u32");
    out[4..8].copy_from_slice(&riff_len.to_le_bytes());
    out
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
                // 24 位是**有符号**的 3 字节：越界值必须在这里就炸，不能靠"取低 3 字节"
                // 静默截断 —— 那会让判据拿到一份**悄悄错了**的夹具，而失败点出现在别处
                // （与 8/16 位同一条约定，见 `integer_fixture_range_is_pinned_…`）。
                assert!(
                    (-8_388_608..=8_388_607).contains(&v),
                    "24-bit sample in -8388608..=8388607, got {v}"
                );
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
// Ogg Vorbis 夹具
// ---------------------------------------------------------------------------

/// 最小合法 Ogg Vorbis 夹具**解出的帧数**。
///
/// 它同时是最后一个 Ogg 页的 `granule position`：本 crate 的对账容差是 0
/// （[`crate::limits::DURATION_TOLERANCE_FRAMES`]），而 Ogg/Vorbis 读端把最后那个 granule
/// 当作"容器声明的总帧数"。两者不等就会被 `duration::reconcile` 拒绝。
pub const OGG_FIXTURE_FRAMES: u64 = 128;

/// Vorbis 的位打包是 **LSB-first**（symphonia 用 `BitReaderRtl`）。
struct LsbBits {
    bytes: Vec<u8>,
    acc: u8,
    filled: u32,
}

impl LsbBits {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            acc: 0,
            filled: 0,
        }
    }

    fn put(&mut self, value: u64, count: u32) {
        for index in 0..count {
            if (value >> index) & 1 == 1 {
                self.acc |= 1 << self.filled;
            }
            self.filled += 1;
            if self.filled == 8 {
                self.bytes.push(self.acc);
                self.acc = 0;
                self.filled = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.filled > 0 {
            self.bytes.push(self.acc);
        }
        self.bytes
    }
}

/// **R177**：数表必须**按变体名定位**（⛔ 不按位置对齐）。
///
/// `pairs` 是（表里**声明的**名字，该值自己的 `Debug` 呈现）。断言两件事：
/// ① 第 `i` 行声明的名字 == 第 `i` 个值**自己的**变体名（`Debug` 的前缀，去掉尾部空格）；
/// ② 名字互不重复。
///
/// 调用方**另外**断言行数（`cases.len() == N`）⇒ 合起来是**双向**的：
/// 表必须覆盖每个变体，且每一行都必须靠**名字**对上号，而不是靠顺序。
///
/// # Panics
///
/// 名字与变体名不符、或出现重复名字时 panic。
pub(crate) fn assert_rows_are_name_located(pairs: &[(&str, String)]) {
    let mut seen = std::collections::BTreeSet::new();
    for (index, (declared, debug)) in pairs.iter().enumerate() {
        let actual = debug
            .split(['(', '{'])
            .next()
            .unwrap_or(debug.as_str())
            .trim();
        assert_eq!(
            *declared, actual,
            "row {index}: the table's declared name must be the variant's own name"
        );
        assert!(
            seen.insert(*declared),
            "row {index}: duplicate variant name `{declared}`"
        );
    }
    assert_eq!(
        seen.len(),
        pairs.len(),
        "every row must carry a distinct variant name"
    );
}

/// Ogg 页 CRC-32：多项式 `0x04C11DB7`、初值 0、**不反射**、不异或输出；CRC 字段先置零。
#[must_use]
pub fn ogg_crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for byte in data {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// 一个 Ogg 页：27 字节页头 ＋ 段表 ＋ 体。
///
/// **包按 Ogg 的 lacing 规则切段**：每个包写成 `len / 255` 个 `255` 段，再写一个
/// `len % 255` 的收尾段（`len` 恰为 255 的倍数时收尾段是 `0`）。一个包因此**可以超过
/// 255 字节** —— 这是必需的：`residue` 的一个 partition 要读 `partition_size / dimensions`
/// 个码字，很容易上百字节。
///
/// # Panics
///
/// 整页的段数超过 255 时 panic（Ogg 的 `page_segments` 是 1 字节）。
#[must_use]
pub fn ogg_page(
    header_type: u8,
    granule: u64,
    serial: u32,
    sequence: u32,
    packets: &[Vec<u8>],
) -> Vec<u8> {
    let mut lacing = Vec::new();
    for packet in packets {
        let mut remaining = packet.len();
        while remaining >= 255 {
            lacing.push(255u8);
            remaining -= 255;
        }
        lacing.push(u8::try_from(remaining).expect("the remainder is below 255"));
    }
    assert!(
        lacing.len() <= 255,
        "a page holds at most 255 segments; split the packet across pages"
    );
    let mut page = Vec::new();
    page.extend_from_slice(b"OggS");
    page.push(0);
    page.push(header_type);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&[0u8; 4]);
    page.push(u8::try_from(lacing.len()).expect("checked above"));
    page.extend_from_slice(&lacing);
    for packet in packets {
        page.extend_from_slice(packet);
    }
    let crc = ogg_crc32(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// Vorbis 识别头（30 字节）：1 声道、44.1 kHz、blocksize 2^8 / 2^11。
#[must_use]
pub fn ogg_vorbis_ident() -> Vec<u8> {
    let mut packet = vec![1];
    packet.extend_from_slice(b"vorbis");
    packet.extend_from_slice(&0u32.to_le_bytes()); // version
    packet.push(1); // channels
    packet.extend_from_slice(&44_100u32.to_le_bytes());
    packet.extend_from_slice(&0i32.to_le_bytes()); // bitrate max
    packet.extend_from_slice(&0i32.to_le_bytes()); // bitrate nominal
    packet.extend_from_slice(&0i32.to_le_bytes()); // bitrate min
    packet.push(0xB8); // blocksize_0 = 2^8, blocksize_1 = 2^11
    packet.push(1); // framing
    packet
}

/// Vorbis 注释头（带一个 vendor 串、零条注释）。
#[must_use]
pub fn ogg_vorbis_comment() -> Vec<u8> {
    let mut packet = vec![3];
    packet.extend_from_slice(b"vorbis");
    packet.extend_from_slice(&5u32.to_le_bytes());
    packet.extend_from_slice(b"yeban");
    packet.extend_from_slice(&0u32.to_le_bytes()); // 注释条数
    packet.push(1); // framing
    packet
}

/// 最小 Vorbis setup 头。逐字段取值与理由：
///
/// - **每个计数字段都是"读出的值 ＋ 1"**（`codebook_count` 是 8 位＋1，`time`/`floor`/
///   `residue`/`mapping`/`mode` 的 count 都是 6 位＋1，`floor0_number_of_books` 是 4 位＋1，
///   `residue_partition_size` 是 24 位＋1，`residue_classifications` 是 6 位＋1）
///   ⇒ **floor/residue/mapping/mode 各至少 1 个**，写 `0` 表示"1 个"。
/// - 码本：`ordered = 0`（无序）之后还有 1 位 **`is_sparse`**；`code_len` 存的是
///   **长度 − 1**；`lookup_type = 0`（无 VQ lookup）。
/// - floor0：`order = 0`、`amplitude_bits = 0` ⇒ 音频包里的 `amplitude` 必为 0
///   ⇒ 全部声道 `do_not_decode` ⇒ **静音**（本夹具要的就是静音）。
///   `order` 必须为 0：一旦 > 0 就要走 `codebook.read_vq()`，而 `lookup_type = 0` 的码本
///   在那一步返回 `vorbis: not a vq codebook`。
/// - residue：`begin = end = 0` ⇒ 空区间，一个样本都不读；`is_used = 0` ⇒ 不读书号。
#[must_use]
pub fn ogg_vorbis_setup() -> Vec<u8> {
    let mut bits = LsbBits::new();
    bits.put(5, 8);
    for byte in b"vorbis" {
        bits.put(u64::from(*byte), 8);
    }
    bits.put(0, 8); // codebook_count - 1
    bits.put(0x56_43_42, 24); // codebook sync
    bits.put(1, 16); // dimensions
    bits.put(1, 24); // entries
    bits.put(0, 1); // ordered = 0
    bits.put(0, 1); // is_sparse = 0（稠密）
    bits.put(0, 5); // code_len - 1
    bits.put(0, 4); // lookup_type = 0
    bits.put(0, 6); // time count - 1
    bits.put(0, 16); // time type
    bits.put(0, 6); // floor count - 1
    bits.put(0, 16); // floor_type = 0
    bits.put(0, 8); // floor0_order = 0
    bits.put(44_100, 16); // floor0_rate
    bits.put(16, 16); // floor0_bark_map_size
    bits.put(0, 6); // floor0_amplitude_bits = 0
    bits.put(0, 8); // floor0_amplitude_offset
    bits.put(0, 4); // number_of_books - 1
    bits.put(0, 8); // book_list[0]
    bits.put(0, 6); // residue count - 1
    bits.put(0, 16); // residue_type = 0
    bits.put(0, 24); // begin
    bits.put(0, 24); // end
    bits.put(0, 24); // partition_size - 1
    bits.put(0, 6); // classifications - 1
    bits.put(0, 8); // classbook
    bits.put(0, 3); // low_bits
    bits.put(0, 1); // high_bits 标志 ⇒ is_used = 0
    bits.put(0, 6); // mapping count - 1
    bits.put(0, 16); // mapping_type = 0
    bits.put(0, 1); // num_submaps 标志 = 0 ⇒ 1 个子映射
    bits.put(0, 1); // coupling 标志 = 0
    bits.put(0, 2); // reserved
    bits.put(0, 8); // submap unused
    bits.put(0, 8); // submap floor = 0
    bits.put(0, 8); // submap residue = 0
    bits.put(0, 6); // mode count - 1
    bits.put(0, 1); // block_flag = 0（短块）
    bits.put(0, 16); // window_type
    bits.put(0, 16); // transform_type
    bits.put(0, 8); // mapping = 0
    bits.put(1, 1); // framing
    bits.finish()
}

/// 一个**可解码**的最小 Ogg Vorbis 流（1 声道、44.1 kHz、静音）。
///
/// 页序：BOS(识别头) → 注释头 → setup 头 → `packets` 个数据页（最后一个带 EOS 与
/// `granule = `[`OGG_FIXTURE_FRAMES`]）。
///
/// 两条实测约束（都不是可选的）：
/// 1. **至少 2 个数据页**：`AudioDecoderOptions::default().gapless == true`，而 gapless
///    会把"解码器重置后的第一个包"静音（`buf.clear()`）。
/// 2. EOS 页的 **granule 必须等于解出的帧数**（本 crate 的对账容差是 0）。
///
/// 音频包取 `0x12`：位序（LSB-first）为 `[0 = 音频包][amplitude = 1][floor_book_idx = 0]
/// …`；`amplitude_bits = 0` 时这个位不会被读，因此包内容对静音夹具不敏感 ——
/// 这也是为什么夹具只用它来占位。
///
/// # Panics
///
/// `packets < 2` 时 panic：少于两个数据页解不出帧（见上）。
#[must_use]
pub fn ogg_vorbis_silence(packets: u32) -> Vec<u8> {
    assert!(
        packets >= 2,
        "gapless trimming silences the first data page"
    );
    let mut out = ogg_page(0x02, 0, 7, 0, &[ogg_vorbis_ident()]);
    out.extend_from_slice(&ogg_page(0, 0, 7, 1, &[ogg_vorbis_comment()]));
    out.extend_from_slice(&ogg_page(0, 0, 7, 2, &[ogg_vorbis_setup()]));
    for index in 0..packets {
        let last = index + 1 == packets;
        out.extend_from_slice(&ogg_page(
            if last { 0x04 } else { 0x00 },
            if last { OGG_FIXTURE_FRAMES } else { 0 },
            7,
            3 + index,
            &[vec![0x12]],
        ));
    }
    out
}

/// `lookup_type = 1` 的 VQ 码本 ＋ **被使用**的 floor0（`order = 1`）。
///
/// 与 [`ogg_vorbis_setup`] 的差别（逐条都是实测出来的）：
/// - 码本带 VQ lookup（`lookup_type = 1`）：`minimum_value = 0`、`delta_value` 由 `zero_coefficients`
///   决定、`value_bits = 1`、`sequence_p = 0`，`lookup_values` **不是读出来的**，而是
///   `lookup1_values(entries = 1, dimensions = 1) = 1`，因此只读 1 个 1 位的 multiplicand（写 1）。
/// - `delta_value = 1.0` 的 Vorbis 位型是 **`0x62A0_0000`**（`float32_unpack`：`mantissa = 1 << 21`、
///   `exponent = 788`）；写 0 时 VQ 向量全零。
/// - floor0 的 `order = 1` ⇒ 音频包里要读一个码字，`coeffs` 来自 VQ 向量。
///
/// **`amplitude_bits = 8` 与 `= 1` 不同**（实测）：`amplitude_bits = 1` 时本夹具仍解不出帧，
/// `= 8` 时解出 128 帧。因此调用方要"被使用的 floor"就传 8。
///
/// 零系数（`zero_coefficients = true`）会让 `Floor0::synthesis` 走到
/// `if p + q == 0.0 { decode_error("vorbis: invalid floor0 coefficients") }`
/// （`symphonia-codec-vorbis/src/floor.rs:326`），也就是说那个包被拒 ⇒ 整份资产解码失败。
#[must_use]
pub fn ogg_vorbis_vq_setup(amplitude_bits: u64, zero_coefficients: bool) -> Vec<u8> {
    let mut bits = LsbBits::new();
    bits.put(5, 8);
    for byte in b"vorbis" {
        bits.put(u64::from(*byte), 8);
    }
    bits.put(0, 8); // codebook_count - 1
    bits.put(0x56_43_42, 24);
    bits.put(1, 16); // dimensions
    bits.put(1, 24); // entries
    bits.put(0, 1); // ordered
    bits.put(0, 1); // is_sparse
    bits.put(0, 5); // code_len - 1
    bits.put(1, 4); // lookup_type = 1
    bits.put(0, 32); // minimum_value = 0.0
    bits.put(if zero_coefficients { 0 } else { 0x62A0_0000 }, 32); // delta_value
    bits.put(0, 4); // value_bits - 1
    bits.put(0, 1); // sequence_p
    bits.put(1, 1); // multiplicand[0]
    bits.put(0, 6);
    bits.put(0, 16);
    bits.put(0, 6); // floor count - 1
    bits.put(0, 16); // floor_type = 0
    bits.put(1, 8); // floor0_order = 1
    bits.put(44_100, 16);
    bits.put(16, 16); // bark_map_size
    bits.put(amplitude_bits, 6);
    bits.put(0, 8);
    bits.put(0, 4); // number_of_books - 1
    bits.put(0, 8); // book_list[0]
    bits.put(0, 6);
    bits.put(0, 16);
    bits.put(0, 24);
    bits.put(0, 24);
    bits.put(0, 24);
    bits.put(0, 6);
    bits.put(0, 8);
    bits.put(0, 3);
    bits.put(0, 1);
    bits.put(0, 6);
    bits.put(0, 16);
    bits.put(0, 1);
    bits.put(0, 1);
    bits.put(0, 2);
    bits.put(0, 8);
    bits.put(0, 8);
    bits.put(0, 8);
    bits.put(0, 6);
    bits.put(0, 1);
    bits.put(0, 16);
    bits.put(0, 16);
    bits.put(0, 8);
    bits.put(1, 1);
    bits.finish()
}

/// 用 [`ogg_vorbis_vq_setup`] 组装的流（`serial` 可指定，便于串联）。
///
/// 数据页里的音频包是 `0x12`：位序（LSB-first）`[0 = 音频][amplitude = 1][floor_book_idx = 0]
/// [码字 = 0][framing = 1]`。
#[must_use]
pub fn ogg_vorbis_vq_stream(
    serial: u32,
    channels: u8,
    amplitude_bits: u64,
    zero_coefficients: bool,
    packets: u32,
) -> Vec<u8> {
    assert!(
        packets >= 2,
        "gapless trimming silences the first data page"
    );
    let mut ident = ogg_vorbis_ident();
    ident[11] = channels;
    let mut out = ogg_page(0x02, 0, serial, 0, &[ident]);
    out.extend_from_slice(&ogg_page(0, 0, serial, 1, &[ogg_vorbis_comment()]));
    out.extend_from_slice(&ogg_page(
        0,
        0,
        serial,
        2,
        &[ogg_vorbis_vq_setup(amplitude_bits, zero_coefficients)],
    ));
    for index in 0..packets {
        let last = index + 1 == packets;
        out.extend_from_slice(&ogg_page(
            if last { 0x04 } else { 0x00 },
            if last { OGG_FIXTURE_FRAMES } else { 0 },
            serial,
            3 + index,
            &[vec![0x12]],
        ));
    }
    out
}

/// **两条物理流串联**的 Ogg：第二条用不同的 `serial`（并可选不同声道数）。
///
/// 实测：读端对"中途出现新的逻辑流"一律报
/// `stream requires a decoder reset mid-decode (chained stream)`（[`crate::DecodeError::ResetRequired`]），
/// **与第二条流的声道数是否相同无关**。⇒ 本 crate 的 `decode_source` 循环里那条
/// "中途换布局"的分支（`locked_channels != channels`）**不能**由串联 Ogg 到达。
#[must_use]
pub fn ogg_vorbis_chained(packets_each: u32, second_channels: u8) -> Vec<u8> {
    let mut out = ogg_vorbis_vq_stream(7, 1, 8, false, packets_each);
    out.extend_from_slice(&ogg_vorbis_vq_stream(
        8,
        second_channels,
        8,
        false,
        packets_each,
    ));
    out
}

/// Vorbis 的 `float32` 位型里 `delta_value = 1.0` 的正确编码。
///
/// `float32_unpack(x)` 的口径是 `mantissa × 2^(exponent − 788)`，其中
/// `mantissa = x & 0x1FFFFF`、`exponent = (x & 0x7FE00000) >> 21`。因此 `1.0` 要的是
/// **`mantissa = 1`、`exponent = 788`**：`0x6280_0001`。
///
/// ⚠ **登记一次算错**：第八/九/十批我一直用 `0x62A0_0000`，它的 `mantissa` 是 **0**
/// （`0x62A0_0000 & 0x1FFFFF == 0`）⇒ 它编码的是 **0.0**，不是 1.0。后果是 floor 的 VQ 向量
/// 全零（见 [`ogg_vorbis_nonzero_setup`] 的说明）。
pub const OGG_VORBIS_DELTA_ONE: u64 = 0x6280_0001;

/// 逐字段构造 setup 的位写入器（`amplitude_bits` 与 `delta` 可配）。
fn nonzero_setup_bits(amplitude_bits: u64, delta: u64) -> LsbBits {
    let mut bits = LsbBits::new();
    bits.put(5, 8);
    for byte in b"vorbis" {
        bits.put(u64::from(*byte), 8);
    }
    bits.put(2, 8); // codebook_count - 1 = 2 ⇒ 3 本码本
    // 码本 0：floor 的 VQ 书（entries = 1，lookup_type = 1）
    bits.put(0x56_43_42, 24);
    bits.put(1, 16);
    bits.put(1, 24);
    bits.put(0, 1);
    bits.put(0, 1);
    bits.put(0, 5);
    bits.put(1, 4);
    bits.put(0, 32); // minimum_value = 0.0
    bits.put(delta, 32);
    bits.put(0, 4); // value_bits - 1
    bits.put(0, 1); // sequence_p
    bits.put(1, 1); // multiplicand[0]（lookup_values = lookup1_values(1, 1) = 1）
    // 码本 1：residue 的 classbook（entries = 1，无 VQ lookup）
    bits.put(0x56_43_42, 24);
    bits.put(1, 16);
    bits.put(1, 24);
    bits.put(0, 1);
    bits.put(0, 1);
    bits.put(0, 5);
    bits.put(0, 4);
    // 码本 2：residue 的 VQ 书（entries = 2，lookup_values = lookup1_values(2, 1) = 2）
    bits.put(0x56_43_42, 24);
    bits.put(1, 16);
    bits.put(2, 24);
    bits.put(0, 1);
    bits.put(0, 1);
    bits.put(0, 5);
    bits.put(0, 5);
    bits.put(1, 4);
    bits.put(0, 32);
    bits.put(delta, 32);
    bits.put(0, 4);
    bits.put(0, 1);
    bits.put(1, 1);
    bits.put(1, 1);
    bits.put(0, 6); // time count - 1
    bits.put(0, 16);
    // floor：1 个，type 0，order = 1（要读一个 VQ 码字）
    bits.put(0, 6);
    bits.put(0, 16);
    bits.put(1, 8); // floor0_order
    bits.put(44_100, 16);
    bits.put(16, 16); // bark_map_size
    bits.put(amplitude_bits, 6);
    bits.put(0, 8);
    bits.put(0, 4); // number_of_books - 1
    bits.put(0, 8); // book_list[0] = 0（用码本 0）
    // residue：1 个，type 0；区间 [0, 128)、partition_size = 128 ⇒ **恰好 1 个 partition**
    bits.put(0, 6);
    bits.put(0, 16);
    bits.put(0, 24); // begin
    bits.put(128, 24); // end
    bits.put(127, 24); // partition_size - 1
    bits.put(0, 6); // classifications - 1 = 0 ⇒ 1 个分类
    bits.put(1, 8); // classbook = 1
    bits.put(1, 3); // low_bits = 1 ⇒ is_used = 1（只用到 j = 0）
    bits.put(0, 1); // high_bits 标志 = 0
    bits.put(2, 8); // j = 0 的 VQ 书 = 2
    // mapping：1 个，type 0
    bits.put(0, 6);
    bits.put(0, 16);
    bits.put(0, 1);
    bits.put(0, 1);
    bits.put(0, 2);
    bits.put(0, 8);
    bits.put(0, 8);
    bits.put(0, 8);
    // mode：1 个
    bits.put(0, 6);
    bits.put(0, 1);
    bits.put(0, 16);
    bits.put(0, 16);
    bits.put(0, 8);
    bits.put(1, 1); // framing
    bits
}

/// **非零谱**的 setup：3 本码本 ＋ 一个可用的 residue partition。
///
/// 构造过程（每一段的来源）：
/// 1. **码本 0**（floor 的 VQ 书）：`entries = 1`、`lookup_type = 1`、`delta_value` = `delta`、
///    一个 `multiplicand = 1`。`lookup_values` **不是读出来的**，是
///    `lookup1_values(entries, dimensions)` 算出来的（本处 = 1）。
/// 2. **码本 1**（classbook）：`entries = 1`、无 VQ lookup。每个 partition 的"分类号"从它读。
/// 3. **码本 2**（residue 的 VQ 书）：`entries = 2`、`lookup_values = lookup1_values(2, 1) = 2`，
///    两个 `multiplicand` 都写 1 ⇒ 任何码字的 VQ 向量都是 `1 × delta + 0`。
/// 4. **residue**：`begin = 0`、`end = 128`、`partition_size = 128` ⇒
///    `parts_to_read = 128 / 128 = 1`；`classifications = 1`、`classbook = 1`、
///    分类 0 的 `is_used = 1`（`low_bits = 1`）且 `j = 0` 用书 2。
/// 5. **音频包**（见 [`ogg_vorbis_nonzero_stream`]）：`[0 音频][amplitude（amplitude_bits 位）]
///    [floor_book_idx 1 位][floor 的 VQ 码字 1 位][classbook 码字 1 位]
///    [128 个 residue 码字 ×1 位][framing 1 位]`。
///    `read_residue_partition_format0` 的 `step = partition_size / dimensions = 128`，
///    因此它**恰好读 128 个码字**，每个给 `out[i]` 加 `delta`。
///
/// ⇒ 谱非零 ⇒ `floor × residue ≠ 0` ⇒ **输出样本非全零**（实测 128/128 非零）。
///
/// **`delta = 0`（`zero_coefficients`）为什么会被拒**（机械解释，回答了两轮"读到但构造不出"）：
/// VQ 向量全零 ⇒ `coeffs[0] = 2·cos(0) = 2`；floor0 合成时
/// `p + q == 0` 当且仅当 `sin²ω == 0` **且** `(coeffs[0] − 2cos ω)² == 0`，也就是
/// `ω ≡ 0 (mod 2π)` —— 那只发生在 bark 映射里出现 **0** 的位置上。命中即
/// `vorbis: invalid floor0 coefficients`（`symphonia-codec-vorbis/src/floor.rs:326`）。
#[must_use]
pub fn ogg_vorbis_nonzero_setup(amplitude_bits: u64, zero_coefficients: bool) -> Vec<u8> {
    let delta = if zero_coefficients {
        0
    } else {
        OGG_VORBIS_DELTA_ONE
    };
    nonzero_setup_bits(amplitude_bits, delta).finish()
}

/// **非零谱**的音频包（字段顺序见 [`ogg_vorbis_nonzero_setup`] 的第 5 条）。
#[must_use]
pub fn ogg_vorbis_nonzero_packet(amplitude_bits: u64) -> Vec<u8> {
    let mut bits = LsbBits::new();
    bits.put(0, 1); // 音频包（第 1 位必须是 0）
    bits.put(
        1,
        u32::try_from(amplitude_bits).expect("amplitude_bits fits u32"),
    );
    bits.put(0, 1); // floor_book_idx
    bits.put(0, 1); // floor 的 VQ 码字 ⇒ 系数 = delta
    bits.put(0, 1); // classbook 码字 ⇒ 分类 0
    for _ in 0..128 {
        bits.put(0, 1); // 码本 2 的码字 ⇒ VQ 向量 = delta
    }
    bits.put(1, 1); // framing
    bits.finish()
}

/// **非零谱**的完整流（1 声道、44.1 kHz、`OGG_FIXTURE_FRAMES` 帧、样本非零）。
#[must_use]
pub fn ogg_vorbis_nonzero_stream(packets: u32, zero_coefficients: bool) -> Vec<u8> {
    assert!(
        packets >= 2,
        "gapless trimming silences the first data page"
    );
    let packet = ogg_vorbis_nonzero_packet(8);
    let mut out = ogg_page(0x02, 0, 7, 0, &[ogg_vorbis_ident()]);
    out.extend_from_slice(&ogg_page(0, 0, 7, 1, &[ogg_vorbis_comment()]));
    out.extend_from_slice(&ogg_page(
        0,
        0,
        7,
        2,
        &[ogg_vorbis_nonzero_setup(8, zero_coefficients)],
    ));
    for index in 0..packets {
        let last = index + 1 == packets;
        out.extend_from_slice(&ogg_page(
            if last { 0x04 } else { 0x00 },
            if last { OGG_FIXTURE_FRAMES } else { 0 },
            7,
            3 + index,
            std::slice::from_ref(&packet),
        ));
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
/// 帧号写成 FLAC 规范的 **UTF-8 编码数**（见 [`push_utf8_number`]），因此帧数**没有**
/// "≤ 127"这个上界；真正的上界是 `STREAMINFO.total_samples` 的 36 位字段。
///
/// # Panics
///
/// 参数超出本夹具支持的最小合法子集时 panic（夹具错误必须在测试里立刻可见，
/// 不能悄悄生成一个非法文件让 CI 去猜）。
#[must_use]
pub fn flac_constant(spec: &FlacSpec, frames: u16, constant: i32) -> Vec<u8> {
    flac_constant_with_frame_spans(spec, frames, constant).0
}

/// [`flac_constant`] 的完整版：同时返回每个帧在字节串里的 `(起点, 长度)`。
///
/// 存在理由：判据 `the_idle_guard_trips_on_1025_consecutive_bad_packets` 必须**逐帧**
/// 破坏 CRC-16（让解封装器照常产出包、而每个包都解码失败），因此它需要帧边界。
/// 把边界从夹具里交出来，胜过在判据里按 UTF-8 帧号长度重算一遍帧长 —— 那是同一件事
/// 写在两个地方，任何一处改动都会让另一处静默失效。
#[must_use]
pub fn flac_constant_with_frame_spans(
    spec: &FlacSpec,
    frames: u16,
    constant: i32,
) -> (Vec<u8>, Vec<(usize, usize)>) {
    // FLAC 的声道域是 **1..=8**，由两处字段各自钉死：`STREAMINFO` 的声道字段是 3 位
    // （存 `channels - 1`，因此 1..=8），帧头的声道赋值是 4 位（`0b0000..=0b0111` 表示
    // 1..=8 个**独立**声道；`0b1000..=0b1010` 是三种立体声去相关，仍是 2 声道）。
    // 因此本夹具能构造 1..=8，**构造不出** 9 声道 —— 这不是夹具的取舍，是容器的上界。
    assert!(
        (1..=8).contains(&spec.channels),
        "FLAC's channel domain is 1..=8 (3-bit STREAMINFO field / 4-bit frame field)"
    );
    assert_eq!(spec.bits, 16, "fixture only packs 16-bit samples");
    assert_eq!(
        spec.block_frames, 256,
        "fixture pins the 256-sample block code"
    );
    assert!(frames > 0, "the fixture needs at least one frame");
    assert!(
        u64::from(frames) * u64::from(spec.block_frames) < (1u64 << 36),
        "total_samples is a 36-bit STREAMINFO field"
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
    let mut spans = Vec::with_capacity(encoded_frames.len());
    for frame in &encoded_frames {
        spans.push((out.len(), frame.len()));
        out.extend_from_slice(frame);
    }
    (out, spans)
}

/// 与 [`flac_constant`] 相同，但把**每一个**帧的 CRC-16 都写成不可能匹配的值。
///
/// 破坏方式是把原 CRC 按位取反：取反值必然与原值不同，而原值就是正确的那个，
/// 因此校验必然失败（不需要假设"某个具体值不会被撞上"）。
///
/// ⚠ **它产出不了"到达解码器的坏包"**（2026-10-10 实测）：symphonia 的 FLAC **读端**
/// 自己会校验帧尾 CRC-16，不符时**跳过整个帧**、一个包都不产出。因此
/// `flac_constant_with_broken_frame_crc(spec, 1025, _)` 的读数是 `EmptyStream`
/// （`bump_idle` 一次都没跑），而不是"1025 次不推进"。
///
/// 本函数因此只用于**反向对照**：判据
/// `crate::decode::tests::the_idle_guard_trips_on_1025_consecutive_bad_packets` 用它钉住
/// "破坏 CRC 走不到解码器"这件事。要构造"读端收下、解码器拒收"的包，请用
/// [`flac_constant_with_reserved_subframes`]。
#[must_use]
pub fn flac_constant_with_broken_frame_crc(spec: &FlacSpec, frames: u16, constant: i32) -> Vec<u8> {
    let (mut bytes, spans) = flac_constant_with_frame_spans(spec, frames, constant);
    for (start, len) in spans {
        let crc_at = start + len - 2;
        let original = u16::from_be_bytes([bytes[crc_at], bytes[crc_at + 1]]);
        bytes[crc_at..crc_at + 2].copy_from_slice(&(!original).to_be_bytes());
    }
    bytes
}

/// 与 [`flac_constant`] 相同，但把**每一个**帧的子帧类型都改成 FLAC 规范里的**保留**值，
/// 并用**正确的** CRC-16 覆盖帧尾。
///
/// 存在的理由：这是"解封装器照常产出包、而解码器逐个失败"的**唯一**可达形状，也就是
/// `crate::decode` 循环里 `Err(SymphoniaError::DecodeError(_))` 那条容忍分支的唯一入口。
/// 乍看之下"破坏帧尾 CRC-16"更直接，但实测（2026-10-10）读端自己会校验帧尾 CRC 并跳过
/// 整个帧 ⇒ 一个包都不产出。因此必须让帧**结构自洽**（帧头 CRC-8 与帧尾 CRC-16 都正确），
/// 只让**子帧语义**非法：读端不解析子帧，解码器才解析。
///
/// 子帧类型字节是 8 位 `[零填充 1][类型 6][浪费位标志 1]`，本函数写 `0b0_001101_0`
/// （类型 `001101` 在规范里是保留值）。
///
/// 判据 `crate::decode::tests::the_idle_guard_trips_on_1025_consecutive_bad_packets`
/// 用它把 [`crate::limits::MAX_IDLE_PACKETS`] 的**端到端**行为钉住。
#[must_use]
pub fn flac_constant_with_reserved_subframes(
    spec: &FlacSpec,
    frames: u16,
    constant: i32,
) -> Vec<u8> {
    let (mut bytes, spans) = flac_constant_with_frame_spans(spec, frames, constant);
    for (start, len) in spans {
        // 帧长 = 11 + 帧号字节数 ⇒ 子帧起点 = `start + len - 5`
        // （4 字节帧头 + 帧号 + 1 字节块大小 + 1 字节 CRC-8）。
        let subframe_at = start + len - 5;
        let crc_at = start + len - 2;
        bytes[subframe_at] = 0b0001_1010;
        // 改完子帧必须重算 CRC-16，否则读端会把整帧丢掉（见上）。
        let crc = crc16(&bytes[start..crc_at]);
        bytes[crc_at..crc_at + 2].copy_from_slice(&crc.to_be_bytes());
    }
    bytes
}

/// 把 FLAC 的 **UTF-8 编码数**推进位写入器。
///
/// FLAC 复用 UTF-8 的**位型**来编码最长 36 位的无符号数（帧号或样本号）。编码规则：
/// 1 字节承载 7 位，2 字节承载 11 位，3 字节 16 位，4 字节 21 位，5 字节 26 位，6 字节
/// 31 位；续字节一律 `10xxxxxx`。
///
/// 存在理由：本夹具此前只写单字节帧号，于是"帧数 ≤ 127"成了**夹具**的硬上界。判据
/// `crate::decode::tests::the_idle_guard_trips_on_1025_consecutive_bad_packets` 需要
/// 1025 个包（逐个来自 1025 个帧），因此必须按规范编码多字节帧号。
///
/// 自检：本函数由 `testfix` 自己的判据
/// `flac_frame_numbers_use_the_spec_utf8_encoding` 在 127/128 与 2047/2048 两个边界上
/// 逐字节钉住（与 UTF-8 自身的编码结果相同）。
///
/// # Panics
///
/// 值超出 31 位时 panic：那超出本夹具支持的最小合法子集。
fn push_utf8_number(w: &mut BitWriter, value: u64) {
    let bits = 64 - value.leading_zeros();
    // (总字节数, 首字节前缀, 首字节载荷位数)
    let (len, prefix, first_bits) = match bits {
        0..=7 => (1u32, 0x00u8, 7u32),
        8..=11 => (2, 0xC0, 5),
        12..=16 => (3, 0xE0, 4),
        17..=21 => (4, 0xF0, 3),
        22..=26 => (5, 0xF8, 2),
        27..=31 => (6, 0xFC, 1),
        other => panic!("a FLAC UTF-8 number carries at most 31 bits, got {other}"),
    };
    let first = prefix
        | u8::try_from((value >> (6 * (len - 1))) & ((1u64 << first_bits) - 1))
            .expect("the first byte payload fits u8");
    w.push_bits(u64::from(first), 8);
    for index in (0..len - 1).rev() {
        let continuation = 0x80
            | u8::try_from((value >> (6 * index)) & 0x3F).expect("a continuation payload fits u8");
        w.push_bits(u64::from(continuation), 8);
    }
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
    push_utf8_number(&mut header, u64::from(index)); // FLAC 的 UTF-8 编码帧号
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
    fn wav_fixture_places_chunks_before_fmt_and_backfills_the_riff_size() {
        // 判据 (夹具自检): `fmt ` 之前确实有那些块，偏移与填充字节都与 RIFF 约定一致，
        // 且 RIFF 尺寸等于"文件长度 - 8"。没有这条判据，用本夹具写出来的"错位"判据
        // 会把夹具自身的布局错误读成解码器的缺陷。
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits: 16,
            format: WavFormat::Integer,
        };
        let four = [0xEEu8; 4];
        let three = [0xEEu8; 3];
        let data = [0u8; 4];
        let junk: [([u8; 4], &[u8]); 3] = [(*b"JUNK", &four), (*b"LIST", &[]), (*b"bext", &three)];
        let bytes = wav_with_chunks_before_fmt(&spec, &junk, &data);

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize,
            bytes.len() - 8,
            "RIFF size must be backfilled from the real length"
        );
        // 块链：JUNK(8+4) | LIST(8+0) | bext(8+3+1 pad) | fmt(8+16) | data(8+4)
        let mut offset = 12usize;
        assert_eq!(&bytes[offset..offset + 4], b"JUNK");
        assert_eq!(
            u32::from_le_bytes([
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7]
            ]),
            4
        );
        offset += 8 + 4;
        assert_eq!(&bytes[offset..offset + 4], b"LIST");
        offset += 8;
        assert_eq!(&bytes[offset..offset + 4], b"bext");
        assert_eq!(
            u32::from_le_bytes([
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7]
            ]),
            3
        );
        offset += 8;
        assert_eq!(&bytes[offset..offset + 3], &three);
        assert_eq!(bytes[offset + 3], 0, "odd chunk must be padded to even");
        offset += 4;
        assert_eq!(&bytes[offset..offset + 4], b"fmt ");
        assert_eq!(
            fmt_body_offset(&bytes),
            Some(offset + 8),
            "the helper must find the very fmt chunk the constructor wrote"
        );
        assert_eq!(bytes.len(), offset + 8 + 16 + 8 + 4);

        // 没有 `fmt ` 的结构必须报 `None`，而不是猜一个偏移。
        assert_eq!(fmt_body_offset(b"not a riff file"), None);
        assert_eq!(
            fmt_body_offset(&bytes[..12]),
            None,
            "a header-only RIFF has no fmt chunk"
        );
    }

    #[test]
    fn extensible_fixture_writes_a_40_byte_fmt_body_at_the_chunk_grid() {
        // 判据 (夹具自检): `WAVE_FORMAT_EXTENSIBLE` 夹具的 40 字节块体必须逐字段落在
        // 上游 `read_ext_fmt` 读的位置上（tag / 声道 / 位深 / `cbSize` / valid bits /
        // 声道掩码 / GUID）。夹具错了，判据会把"位移没有溢出"读成"闸门漏判"。
        let spec = WavExtensibleSpec {
            channels: 33,
            sample_rate: 8_000,
            bits: 8,
            valid_bits: 8,
            channel_mask: 0,
            sub_format: [0xA5; 16],
        };
        let junk: [([u8; 4], &[u8]); 1] = [(*b"JUNK", &[0xEEu8; 4])];
        let bytes = wav_extensible_with_junk(&spec, &junk, &[0u8; 4]);
        let body = fmt_body_offset(&bytes).expect("fixture must contain a fmt chunk");
        let at = |offset: usize| bytes[body + offset];
        assert_eq!(
            u32::from_le_bytes([
                bytes[body - 4],
                bytes[body - 3],
                bytes[body - 2],
                bytes[body - 1]
            ]),
            40,
            "the fmt chunk must declare exactly 40 bytes"
        );
        assert_eq!(u16::from_le_bytes([at(0), at(1)]), 0xFFFE);
        assert_eq!(u16::from_le_bytes([at(2), at(3)]), 33);
        assert_eq!(
            u32::from_le_bytes([at(4), at(5), at(6), at(7)]),
            spec.sample_rate
        );
        assert_eq!(u16::from_le_bytes([at(14), at(15)]), 8, "wBitsPerSample");
        assert_eq!(u16::from_le_bytes([at(16), at(17)]), 22, "cbSize");
        assert_eq!(
            u16::from_le_bytes([at(18), at(19)]),
            8,
            "wValidBitsPerSample"
        );
        assert_eq!(u32::from_le_bytes([at(20), at(21), at(22), at(23)]), 0);
        assert_eq!(
            &bytes[body + 24..body + 40],
            &[0xA5u8; 16],
            "SubFormat GUID"
        );
        // `data` 紧跟在 40 字节块体之后（`fmt ` 的长度是偶数，因此没有填充字节）。
        assert_eq!(&bytes[body + 40..body + 44], b"data");
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize,
            bytes.len() - 8,
            "RIFF size must be backfilled from the real length"
        );
        // 块链 = 12 字节 RIFF 头 + JUNK(8+4) + fmt(8+40) + data(8+4)。
        assert_eq!(bytes.len(), 12 + 12 + 48 + 12);
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

    /// 判据（夹具自检 / 压缩容器的帧号编码）：FLAC 的帧号是 **UTF-8 编码数**，因此
    /// 0…127 占 1 字节、128…2047 占 2 字节、2048… 占 3 字节。
    ///
    /// 量什么：[`flac_constant_with_frame_spans`] 返回的帧跨度与帧头里的帧号字节。
    /// 怎么量：在 127/128 与 2047/2048 两个边界上逐字节比对；并把跨度与帧区做无缝覆盖核对。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 帧号 | 帧号字节 | 帧长（字节） |
    /// | :--- | :--- | ---: |
    /// | 0 | `00` | 12 |
    /// | 127 | `7F` | 12 |
    /// | 128 | `C2 80` | 13 |
    /// | 2047 | `DF BF` | 13 |
    /// | 2048 | `E0 A0 80` | 14 |
    ///
    /// 帧长 = 11 + 帧号字节数（4 字节头 + 帧号 + 1 字节块大小 + 1 字节 CRC-8 +
    /// 3 字节单声道 16 位 CONSTANT 子帧 + 2 字节 CRC-16）。
    ///
    /// 为什么需要它：既有判据 `flac_fixture_has_a_valid_streaminfo_and_frame_chain` 把
    /// `header_len` 写死成 6（= 4 + 1 + 1），那个假设只在帧号 < 128 时成立。本档帧号
    /// 改成多字节编码之后，必须有一条判据把"哪一档用几个字节"钉住，否则夹具错了会把
    /// 解码失败误读成"解码器坏了"。
    ///
    /// 注入（实测）：把 `push_utf8_number` 的第二档前缀由 `0xC0` 改成 `0xE0` ⇒ 本条在
    /// 帧 128 的字节比对上红；把 `first_bits` 的对应档由 5 改成 4 ⇒ 同上。
    #[test]
    fn flac_frame_numbers_use_the_spec_utf8_encoding() {
        let spec = FlacSpec::default();

        let (one, spans) = flac_constant_with_frame_spans(&spec, 1, 0);
        assert_eq!(spans, vec![(42usize, 12usize)]);
        assert_eq!(one.len(), 42 + 12);
        assert_eq!(one[42 + 4], 0x00, "frame 0 is a single 0x00 byte");

        let (edge, spans) = flac_constant_with_frame_spans(&spec, 129, 0);
        assert_eq!(spans.len(), 129);
        assert_eq!(spans[127], (spans[126].0 + 12, 12), "frame 127: one byte");
        assert_eq!(spans[128].1, 13, "frame 128: two bytes");
        let f128 = spans[128].0;
        assert_eq!(
            &edge[f128 + 4..f128 + 6],
            &[0xC2, 0x80],
            "frame 128 must be UTF-8 C2 80"
        );

        let (big, spans) = flac_constant_with_frame_spans(&spec, 2049, 0);
        assert_eq!(spans[2047].1, 13, "frame 2047: still two bytes");
        assert_eq!(spans[2048].1, 14, "frame 2048: three bytes");
        let f2048 = spans[2048].0;
        assert_eq!(
            &big[f2048 + 4..f2048 + 7],
            &[0xE0, 0xA0, 0x80],
            "frame 2048 must be UTF-8 E0 A0 80"
        );

        // 跨度必须无缝覆盖整个帧区：既有"帧号恒为 1 字节"的算法错一处就会在这里露出来。
        for (bytes, spans) in [
            (&edge, &flac_constant_with_frame_spans(&spec, 129, 0).1),
            (&big, &flac_constant_with_frame_spans(&spec, 2049, 0).1),
        ] {
            let mut cursor = 42usize;
            for (start, len) in spans {
                assert_eq!(*start, cursor, "frame spans must be contiguous");
                cursor += len;
            }
            assert_eq!(
                cursor,
                bytes.len(),
                "the spans must cover the whole frame area"
            );
        }
    }

    /// 判据（夹具自检 / UTF-8 编码数的位打包）：`push_utf8_number` 在**每一档的两个端点**
    /// 与"载荷最高位为 1"的点上逐字节正确。
    ///
    /// 量什么：`push_utf8_number(value)` 写出的字节（单位：字节）。怎么量：直接调用私有的
    /// 位写入器（本模块的判据可以访问它），逐值比对期望字节。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 值 | 字节 | 档 |
    /// | ---: | :--- | :--- |
    /// | `0x00` | `00` | 1 字节 |
    /// | `0x7F` | `7F` | 1 字节上端 |
    /// | `0x80` | `C2 80` | 2 字节下端 |
    /// | `0x7FF` | `DF BF` | 2 字节上端 |
    /// | `0x800` | `E0 A0 80` | 3 字节下端 |
    /// | `0x8000` | `E8 80 80` | 3 字节，**载荷最高位为 1** |
    /// | `0xFFFF` | `EF BF BF` | 3 字节上端 |
    /// | `0x1_0000` | `F0 90 80 80` | 4 字节下端 |
    /// | `0x1F_FFFF` | `F7 BF BF BF` | 4 字节上端 |
    ///
    /// 为什么需要它：**只测每一档的起始值抓不到载荷位宽写错** —— 例如把 3 字节档的首字节
    /// 载荷位数由 4 写成 3，在 `0x800`（载荷 `0`）上读数完全一样，只有 `0x8000`（载荷 `8`）
    /// 才露出来。本批注入实测正是这样：`12..=16 => (3, 0xE0, 4)` 改成 `(3, 0xE0, 3)`
    /// 时，既有判据全绿。
    ///
    /// 注入（实测）：把 `12..=16 => (3, 0xE0, 4)` 改成 `(3, 0xE0, 3)` ⇒ 本条以
    /// `value 0x8000` 红。
    #[test]
    fn push_utf8_number_packs_every_band_at_both_ends() {
        fn encode(value: u64) -> Vec<u8> {
            let mut writer = BitWriter::new();
            push_utf8_number(&mut writer, value);
            writer.align();
            writer.into_bytes()
        }
        let cases: [(u64, &[u8]); 9] = [
            (0x00, &[0x00]),
            (0x7F, &[0x7F]),
            (0x80, &[0xC2, 0x80]),
            (0x7FF, &[0xDF, 0xBF]),
            (0x800, &[0xE0, 0xA0, 0x80]),
            (0x8000, &[0xE8, 0x80, 0x80]),
            (0xFFFF, &[0xEF, 0xBF, 0xBF]),
            (0x1_0000, &[0xF0, 0x90, 0x80, 0x80]),
            (0x1F_FFFF, &[0xF7, 0xBF, 0xBF, 0xBF]),
        ];
        for (value, expected) in cases {
            assert_eq!(encode(value), expected, "value {value:#x}");
        }
        // 5 字节与 6 字节档只钉长度（本 crate 的夹具用不到那么大的帧号）。
        assert_eq!(encode(0x20_0000).len(), 5);
        assert_eq!(encode(0x400_0000).len(), 6);
    }

    /// 判据（夹具自检 / 整数与浮点编码器的端序与量程）：三种位宽的**字节序**逐个钉住，
    /// 且 24 位的量程两端都要**在夹具层**报错。
    ///
    /// 量什么：`encode_int_samples` 与 `encode_f32_samples` 的**逐字节**输出。
    /// 怎么量：取每个位深的边界值，与手写的小端字节串比对。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 调用 | 字节 |
    /// | :--- | :--- |
    /// | `(16, &[0x1234])` | `34 12` |
    /// | `(24, &[0x12_3456])` | `56 34 12` |
    /// | `(24, &[-8_388_608])` | `00 00 80` |
    /// | `(24, &[8_388_607])` | `FF FF 7F` |
    /// | `(32, &[0x1234_5678])` | `78 56 34 12` |
    /// | `encode_f32_samples(&[1.0])` | `00 00 80 3F` |
    ///
    /// 为什么需要它：既有 `integer_fixture_range_is_pinned_…` 只对 24/32 位断言了**长度**，
    /// 字节序无人看管；`encode_f32_samples` 完全没有自检。
    ///
    /// 注入（实测）：把 24 位那支的 `raw[..3]` 改成 `raw[1..]` ⇒ 本条在 24 位三行红。
    #[test]
    fn the_sample_encoders_are_little_endian_at_every_depth() {
        assert_eq!(encode_int_samples(16, &[0x1234]), vec![0x34, 0x12]);
        assert_eq!(encode_int_samples(24, &[0x12_3456]), vec![0x56, 0x34, 0x12]);
        assert_eq!(
            encode_int_samples(24, &[-8_388_608]),
            vec![0x00, 0x00, 0x80],
            "the 24-bit lower bound must sign-extend into the top byte"
        );
        assert_eq!(encode_int_samples(24, &[8_388_607]), vec![0xFF, 0xFF, 0x7F]);
        assert_eq!(
            encode_int_samples(32, &[0x1234_5678]),
            vec![0x78, 0x56, 0x34, 0x12]
        );
        assert_eq!(encode_f32_samples(&[1.0]), 1.0f32.to_le_bytes().to_vec());
        assert_eq!(
            encode_f32_samples(&[-2.5]),
            (-2.5f32).to_le_bytes().to_vec()
        );
        // `-0.0` 与 `+0.0` 的位型不同，夹具必须原样搬运（摘要判据依赖这一点）。
        assert_ne!(
            encode_f32_samples(&[0.0]),
            encode_f32_samples(&[-0.0]),
            "the encoder must preserve the sign bit of zero"
        );
    }

    /// 判据（夹具自检）：24 位越界值必须在**夹具层**炸，而不是被静默截断成低 3 字节。
    ///
    /// 为什么需要它：改建前 24 位那支是 `v.to_le_bytes()` 取 `raw[..3]` —— **没有任何量程
    /// 检查**，因此传 `8_388_608`（上界加一）会得到一份"看起来合法、值却错"的夹具，失败点
    /// 出现在别的判据里。8 位与 16 位本来就有这道检查（各自的 `should_panic` 判据），24 位
    /// 是唯一漏掉的位深。
    ///
    /// 注入（实测）：把新加的 `assert!((-8_388_608..=8_388_607).contains(&v), …)` 删掉 ⇒
    /// 本条以"应当 panic 却没有"红。
    #[test]
    #[should_panic(expected = "24-bit sample in -8388608..=8388607")]
    fn twenty_four_bit_fixture_rejects_a_value_that_does_not_fit_i24() {
        let _ = encode_int_samples(24, &[8_388_608]);
    }

    /// 判据（夹具自检）：`WavSpec` 的两个派生字段按定义算。
    ///
    /// 量什么：`block_align()`（字节/帧）与 `byte_rate()`（字节/秒）。
    /// 怎么量：直接算，并与手写值比对。
    ///
    /// 读数（本机、debug 构建）：2 声道 16 位 ⇒ `block_align = 4`；48 kHz ⇒ `byte_rate = 192000`。
    ///
    /// 为什么需要它：这两个字段要写进 `fmt ` 块体，而 symphonia 会用 `byte_rate` 推算每包
    /// 帧数。它们错了，`data` 块的切包就会跟着错，失败会表现成"解码器读少了几个样本"。
    ///
    /// 注入（实测）：把 `block_align` 的 `self.channels * (self.bits / 8)` 改成
    /// `self.channels * self.bits` ⇒ 本条红。
    #[test]
    fn the_wav_spec_derives_block_align_and_byte_rate() {
        let stereo = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits: 16,
            format: WavFormat::Integer,
        };
        assert_eq!(stereo.block_align(), 4);
        assert_eq!(stereo.byte_rate(), 192_000);
        let mono24 = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits: 24,
            format: WavFormat::Integer,
        };
        assert_eq!(mono24.block_align(), 3);
        assert_eq!(mono24.byte_rate(), 24_000);
        let float = WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits: 32,
            format: WavFormat::Float,
        };
        assert_eq!(float.block_align(), 8);
        assert_eq!(float.byte_rate(), 352_800);
    }

    /// 判据（夹具自检）：Ogg 夹具的**页结构与 CRC**必须自洽，且 EOS 页的 granule 恰为
    /// [`OGG_FIXTURE_FRAMES`]。
    ///
    /// 量什么：各页的 `OggS` 魔术、CRC 自洽性（把 CRC 字段清零后重算）、页序号序列、
    /// EOS 标志与 granule，以及"数据页数 ≥ 2"这条硬约束。
    /// 怎么量：逐页走查 —— 27 字节页头 ＋ 段表，按段表长度推进。
    ///
    /// 读数（本机、debug 构建）：`ogg_vorbis_silence(2)` = 5 页（BOS / 注释 / setup / 数据 /
    /// EOS 数据），页序号 0..4，最后一个 granule = 128，每页 CRC 与重算值相同。
    ///
    /// 为什么需要它：夹具错了会把"解码器坏了"读成真。本夹具的字节全部手写（没有外部样本），
    /// 因此必须有一条判据把页边界与 CRC 钉住。
    #[test]
    fn ogg_fixture_pages_are_well_formed_and_the_last_granule_is_the_frame_count() {
        let bytes = ogg_vorbis_silence(2);
        let mut offset = 0usize;
        let mut pages = Vec::new();
        while offset < bytes.len() {
            assert_eq!(&bytes[offset..offset + 4], b"OggS", "page {offset} magic");
            let granule =
                u64::from_le_bytes(bytes[offset + 6..offset + 14].try_into().expect("8 bytes"));
            let sequence =
                u32::from_le_bytes(bytes[offset + 18..offset + 22].try_into().expect("4 bytes"));
            let header_type = bytes[offset + 5];
            let segments = usize::from(bytes[offset + 26]);
            let mut body = 0usize;
            for index in 0..segments {
                body += usize::from(bytes[offset + 27 + index]);
            }
            let end = offset + 27 + segments + body;
            // CRC 自洽：把 CRC 字段清零后重算。
            let mut copy = bytes[offset..end].to_vec();
            copy[22..26].fill(0);
            let recomputed = ogg_crc32(&copy);
            let stored = u32::from_le_bytes(bytes[offset + 22..offset + 26].try_into().unwrap());
            assert_eq!(recomputed, stored, "page {sequence} CRC");
            pages.push((sequence, header_type, granule));
            offset = end;
        }
        assert_eq!(offset, bytes.len(), "the pages must tile the whole fixture");
        assert_eq!(pages.len(), 5, "BOS + comment + setup + 2 data pages");
        assert_eq!(
            pages.iter().map(|page| page.0).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4],
            "page sequence numbers must be 0..N"
        );
        assert_eq!(pages[0].1 & 0x02, 0x02, "the first page must be a BOS page");
        assert_eq!(pages[4].1 & 0x04, 0x04, "the last page must be an EOS page");
        assert_eq!(
            pages[3].2, 0,
            "non-final data pages carry granule 0 in this fixture"
        );
        assert_eq!(
            pages[4].2, OGG_FIXTURE_FRAMES,
            "the EOS granule is the declared frame count and must match what decodes"
        );
        let one = ogg_vorbis_silence(3);
        assert!(
            one.len() > bytes.len(),
            "more data pages must grow the fixture"
        );
    }

    /// 判据（夹具自检）：[`ogg_page`] 按 Ogg 的 lacing 规则把**大包**切成多段，且 CRC 覆盖段表。
    ///
    /// 量什么：一个 600 字节的包在页里占的段数、段值序列与整页长度（单位：段／字节）。
    /// 怎么量：直接构造一页，逐字节核对段表。
    ///
    /// 读数（本机、debug 构建）：600 字节 ⇒ 段值 `255, 255, 90`（3 段）、`page_segments == 3`、
    /// 整页 = 27 + 3 + 600 = 630 字节；恰好 255 字节 ⇒ 段值 `255, 0`（2 段，收尾 0 表示包在此结束）。
    ///
    /// 为什么需要它：这是**非静音** Ogg 夹具的前置条件 —— 一个 residue partition 要读
    /// `partition_size / dimensions` 个码字，包很容易超过 255 字节；改建前 [`ogg_page`] 只写
    /// 单段并在 ≥ 256 字节时 panic（第十批实测：`part=1` 那一格需要 2048 字节的包 ⇒ 夹具 panic）。
    ///
    /// 注入（实测）：把 lacing 的 `while remaining >= 255` 改成 `while remaining > 255` ⇒
    /// 恰好 255 字节的包会被写成单段 `255`（读端会当成"包未结束"）⇒ 本条红。
    #[test]
    fn ogg_page_splits_large_packets_across_lacing_segments() {
        let page = ogg_page(0, 0, 7, 0, &[vec![0u8; 600]]);
        assert_eq!(page[26], 3, "600 bytes need three lacing values");
        assert_eq!(&page[27..30], &[255, 255, 90]);
        assert_eq!(page.len(), 27 + 3 + 600);
        let exact = ogg_page(0, 0, 7, 0, &[vec![0u8; 255]]);
        assert_eq!(exact[26], 2, "exactly 255 bytes needs a terminating 0");
        assert_eq!(&exact[27..29], &[255, 0]);
        assert_eq!(exact.len(), 27 + 2 + 255);
        // CRC 必须覆盖段表：改一个段值后重算，值应当变。
        let mut copy = page.clone();
        copy[28] ^= 0x01;
        copy[22..26].fill(0);
        assert_ne!(
            ogg_crc32(&copy),
            u32::from_le_bytes(page[22..26].try_into().expect("4 bytes"))
        );
    }

    /// 判据（**R183 的终点形态**：把"注入"收进判据内的绿/红两臂 ⇒ 那几种形态的**外部注入分母归 0**）。
    ///
    /// 核的是 [`assert_rows_are_name_located`] 这个**下界函数**本身：喂正确表必须过、喂坏表必须被拒。
    /// ⛔ 不假装有牙：四条臂都用 `catch_unwind` 真跑一遍，**被拒**是断言出来的事实。
    ///
    /// ⭐ **R187**：每条样本都带 `// R187 sample: …` 标记，便于与"被测形态"分开。
    /// ⭐ **R191（配对对照必须排除平凡性）**：只给"坏表被拒"是**平凡**的（任何会 panic 的东西都满足）。
    /// 因此**必须再给一条"只差无关维度的表照样通过"** —— 下面 arm-4（名字相同、值不同）与
    /// arm-1（名字相同、值相同）成对，说明这条判据区分的是**名字**而不是"有没有值"。
    ///
    /// 读数（本机、debug 构建）：
    ///
    /// | 臂 | 样本 | 期望 | 实测 |
    /// | :--- | :--- | :--- | :--- |
    /// | arm-1 | 名字与 `Debug` 一致 | 通过 | 通过 |
    /// | arm-2 | 某行名字改错（`S16x`） | **被拒** | 被拒（panic 被捕获） |
    /// | arm-3 | 两行**重名** | **被拒** | 被拒 |
    /// | arm-4 | 名字相同、**值不同**（只差无关维度） | 通过 | 通过 |
    #[test]
    fn name_located_rows_reject_a_mislabelled_table_and_accept_an_unrelated_difference() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        eprintln!("[R187-PROBE name-located-1] arms start: {N} arms", N = 4);
        // R187 sample: arm-1 绿（名字与 Debug 一致）
        let good: Vec<(&str, String)> = vec![("U8", "U8".to_owned()), ("S16", "S16".to_owned())];
        eprintln!("[R187-PROBE name-located-arm1] good table accepted");
        assert_rows_are_name_located(&good);

        eprintln!("[R187-PROBE name-located-arm2] mislabelled row: expecting rejection");
        // R187 sample: arm-2 红（名字改错）
        let mislabelled: Vec<(&str, String)> =
            vec![("U8", "U8".to_owned()), ("S16x", "S16".to_owned())];
        assert!(
            catch_unwind(AssertUnwindSafe(|| assert_rows_are_name_located(
                &mislabelled
            )))
            .is_err(),
            "[R187-PROBE name-located-arm2] a mislabelled row must be rejected"
        );

        eprintln!("[R187-PROBE name-located-arm3] duplicate names: expecting rejection");
        // R187 sample: arm-3 红（两行重名）
        let duplicated: Vec<(&str, String)> =
            vec![("U8", "U8".to_owned()), ("U8", "U8".to_owned())];
        assert!(
            catch_unwind(AssertUnwindSafe(|| assert_rows_are_name_located(
                &duplicated
            )))
            .is_err(),
            "[R187-PROBE name-located-arm3] duplicate names must be rejected"
        );

        eprintln!("[R187-PROBE name-located-arm4] unrelated difference: expecting acceptance");
        // R187 sample: arm-4 绿（**只差无关维度**：名字相同、`Debug` 呈现不同）—— R191 的反平凡臂。
        let unrelated_difference: Vec<(&str, String)> =
            vec![("U8", "U8".to_owned()), ("S16", "S16(99)".to_owned())];
        assert_rows_are_name_located(&unrelated_difference);
        // ⭐ **反平凡性（R191）再钉一次** —— 两条对照各自"只差一个维度"：
        // arm-2 与 arm-1 **只差声明的名字**（`Debug` 相同）⇒ 必须被拒；
        assert_ne!(
            good[1].0, mislabelled[1].0,
            "arm-2 differs only in the name"
        );
        assert_eq!(
            good[1].1, mislabelled[1].1,
            "arm-2 keeps the same Debug text"
        );
        // arm-4 与 arm-1 **只差 `Debug` 文本**（名字相同）⇒ 必须通过。
        assert_eq!(
            good[1].0, unrelated_difference[1].0,
            "arm-4 keeps the same name"
        );
        assert_ne!(
            good[1].1, unrelated_difference[1].1,
            "arm-4 differs only in the Debug text"
        );
    }
}
