//! 广播级 RF64 / BW64 容器与 BEXT 元数据 [ARCH-FMT-001]。
//!
//! ## 核验过的上游事实（字段表与出处见 `docs/ledger/render-master-notes.md` §1）
//!
//! | 事实 | 核验来源（已实际抓取） |
//! | :--- | :--- |
//! | RF64 顶层 `RF64` 取代 `RIFF`, 大小字段写 `0xFFFF_FFFF` | FFmpeg `libavformat/wavenc.c` `wav_write_header` / `wav_write_trailer` |
//! | `ds64` 紧随 `WAVE` 之后、**在 `fmt ` 之前** | 同上；libsndfile `src/rf64.c` `rf64_write_header` |
//! | `ds64` chunk 大小 = **28**: `u64 riffSize` + `u64 dataSize` + `u64 sampleCount` + `u32 tableLength` | 同两个实现的写入语句逐字对照 |
//! | `ds64` 的 `sampleCount` 是**帧数**（每声道采样数）, 不是"总采样数" | libsndfile 写 `psf->sf.frames`; FFmpeg 由 `maxpts-minpts+last_duration` 换算成帧 |
//! | `data` chunk 的 32 位大小字段写 `0xFFFF_FFFF`, 真实长度在 `ds64` | FFmpeg `avio_wl32(pb, -1)`；libsndfile `BHW4 (0xffffffff)` |
//! | `bext` 固定前缀 **602 字节**: 256+32+32+10+8+8+2+64+190 | ExifTool RIFF 标签表给出的偏移 `0/256/288/320/338/346/348/602` |
//! | `bext` v2 在 UMID 之后插入 5 个 `i16`（响度/真峰/短时/瞬时）, 保留区由 190 缩为 **180**, 总固定长度仍 602 | EBU Tech 3285 的公开检索片段 "180 bytes reserved for extension" |
//! | `WAVE_FORMAT_EXTENSIBLE` 的 `fmt ` 负载 = 40 字节（`cbSize`=22 + validBits + channelMask + 16 字节 GUID） | hound `read_wave_format_extensible`（要求 >=40 且 `cbSize == 22`）; libsndfile `rf64_write_fmt_chunk` |
//! | `KSDATAFORMAT_SUBTYPE_PCM` / `..._IEEE_FLOAT` 的 16 字节 GUID | hound `src/lib.rs` 常量逐字节读取 |
//! | RIFF chunk 长度不含偶数对齐补位字节 | libsndfile `rf64_write_tailer`: `if (psf->dataend & 1) write 1 pad` |
//! | **非 PCM（IEEE 浮点）容器必须带 `fact` chunk**: `u32` 帧数, 位置在 `fmt ` 之后、`bext` 之前 | FFmpeg `libavformat/wavenc.c`: `codecpar->codec_tag != 0x01`（即"非 PCM"）时写 `fact`, `wav->fact_pos` 紧跟 `fmt ` 且在 `bext` 之前, 结尾回填帧数; hound `src/read.rs` 的 `ChunkKind::Fact` 分支引 Rev.3 原文 "All (compressed) non-PCM formats must have a fact chunk. The chunk contains at least one value, the number of samples in the file." |
//!
//! 上面最后一行是**本模块此前的缺口**: [`write_container`] 对浮点负载不写 `fact`。
//! 仓库内还有第二条**独立**证据说明这条 chunk 该写: `examples/support/l1_digest_record.rs`
//! 的 f32 WAV 编码器（本 crate 的 L1 摘要落点, 与 `rf64` 是两份独立实现）写出的头恒定
//! **56 字节** = `RIFF(12) + fmt (8+16) + fact(8+4) + data(8)`, 其中 `fact` 的负载就是帧数。
//! 计数依据: 那条判据的 `wav_bytes = 65592`、`frames = 8192`、`channels = 2`、32f
//! ⇒ 负载 `8192 × 2 × 4 = 65536` 字节, 头部 `65592 − 65536 = 56` 字节。
//!
//! **写 `fact` 不会让它变陈旧**: hound `src/write.rs` 警告"追加写时不会更新 `fact`"。
//! 本模块的写入器是**一次性**的（`for_payload` 在写之前就拿到总帧数, 头部只写一遍,
//! 之后不再 seek 回去改长度）, 因此那条警告在这里不适用。
//!
//! ## 本模块零第三方依赖
//!
//! 只用 `std::io`。因此:
//!
//! - 本机可用 `rustc --edition 2024 --test` 单独验证（含 chunk 顺序、字段值、
//!   `>4GB` 分支、BEXT 往返、读取器回读）;
//! - 普通 RIFF WAV 的**独立第三方读取器验证**放在 `wav.rs`（用 `hound` 读回本模块
//!   写出的文件）, 那是 CI 判据。
//!
//! ## 已知边界（如实登记）
//!
//! - **读取器对任意字节输入都是全函数**（上一轮加固）：chunk 循环的"起点 + 声明长度"
//!   一律 `checked_add`，`PcmFormat::block_align` / `byte_rate` 一律饱和，
//!   `parse_fmt_payload` 另外把放不进 `nBlockAlign` 的声道布局判为
//!   [`Rf64Error::UnrepresentableBlockAlign`]。判据是破坏扫描（三种容器各一遍）与
//!   三条超大声明长度。
//! - **写入器只写自己读得回的容器**（本轮加固）：[`ContainerPlan::validate`] 把两类
//!   "读取器必然拒绝"的计划挡在**任何字节落盘之前** —— `fmt ` 的声道数为 0、帧对齐放不
//!   进 `u16`（判定与读取器**共用** [`PcmFormat::block_align_fits_u16`]），以及 `bext`
//!   文本字段含 NUL（读取器在第一个 NUL 处截断）。修复前 `write_container` 对这两类计划
//!   返回 `Ok(())` 并写出完整字节, 调用方拿到的是一个本 crate 自己读不回来的交付物;
//!   判据是 `a_format_the_writer_accepts_the_reader_reads_back_identically` 与
//!   `a_bext_text_field_that_cannot_round_trip_is_refused_before_any_byte`。
//!   [`ContainerPlan::header_bytes`] 仍然信任计划（它没有 `Result` 出口）, 因此直接用它
//!   拼文件的调用方要先过 `validate()`。
//! - **写入点还要把负载长度与声明的 `data` 长度对齐**（本轮新增）：这两条长度有两个
//!   独立来源（计划里的 [`Rf64Sizes::data_size`] 与 `payload.len()`），不一致时
//!   修复前的 `write_container` 照样返回 `Ok(())` —— 声明偏长则本 crate 的读取器
//!   报 `Truncated`，声明偏短则读取器**静默丢掉尾巴**。判据是
//!   `the_payload_length_must_match_the_declared_data_length`。
//! - **`sample_rate == 0` 也被拒绝**（本轮新增，来源是 crate 内与 [`crate::wav`] 的
//!   分歧）：本模块的读取器**读得回**零采样率 —— 它产出的是外部解码器打不开的
//!   `nAvgBytesPerSec = 0` 容器，而同一个 crate 走 `hound` 的写入器
//!   （[`crate::wav::check_container_fields`]）早已拒绝它。判据是
//!   `a_zero_sample_rate_is_refused_before_any_byte`。
//! - **`bits_per_sample == 0` 也被拒绝**（本轮新增，与上一行同族的第三个"零分母字段"）：
//!   `PcmFormat::bytes_per_sample` 对 0 位给 0，于是 `fmt ` 会同时声明
//!   `nBlockAlign = 0` 与 `nAvgBytesPerSec = 0` —— 合规解码器按 `nBlockAlign` 求帧数
//!   就是除零。同一个 crate 走 `hound` 的写入器（[`crate::wav::check_container_fields`]）
//!   早已把它判为 `UnsupportedDepth(0)`。本模块的**读取器**此前也接受它，并把 `data` 的
//!   **字节数**当成帧数（`data_size` 除以被 `max(1)` 兜住的 `nBlockAlign`）；两侧现在
//!   共用同一个拒绝 [`Rf64Error::ZeroBitsPerSample`]。判据是
//!   `a_zero_bit_depth_is_refused_before_any_byte`（容器侧）与
//!   `the_two_writers_in_this_crate_agree_on_a_zero_bit_depth`（两条写入器之间）。
//! - 只实现 `bext` 版本 1 与 2 的读写; 1997 年的 v0 布局未核验, 读到即返回
//!   [`Rf64Error::UnsupportedBextVersion`], 登记为 `pending`。**写入器与读取器的
//!   接受集现在是同一个** `{1, 2}`: [`Bext::to_bytes`] 对 v0 **与 v≥3** 都拒绝
//!   （修复前只有下界, 于是 `version = 3` 会被照写进文件, 而本 crate 自己读不回来;
//!   判据是 `every_version_the_writer_accepts_round_trips` 与
//!   `the_writer_refuses_a_version_the_reader_cannot_read`）。**并且这个接受集要在
//!   [`ContainerPlan::validate`] 这一层也成立**（本轮补上）: 一个带着 v≥3、或版本与
//!   响度块矛盾（v2 缺响度 / v1 带响度）的 `bext` 的计划, 此前 `validate` 返回 `Ok(())`
//!   而 [`write_container`] 在 `Bext::to_bytes` 的 `assert!` 上 **panic**
//!   （release 与本机实测的字面读数: `validate() = Ok(()) -> write_container = PANIC`,
//!   写出 0 字节; debug 构建下连 [`ContainerPlan::for_payload`] 末尾的自洽检查都会
//!   panic）。判据是 `every_bext_shape_the_writer_accepts_the_reader_reads_back`。
//!   **文本字段的取值也是同一个
//!   接受集**: 含 NUL 的 `Description` / `Originator` / `OriginatorReference` /
//!   `OriginationDate` / `OriginationTime` 与以 NUL 结尾的 `CodingHistory` 都被拒绝
//!   （读取器读回的不是同一个字符串, 见 [`Bext::field_that_does_not_round_trip`]）。
//! - BW64 的 `axml`/`bxml`/`sxml`/`chna` 四个 XML chunk 未实现（[ARCH-FMT-001]
//!   只要求 RF64/BW64 容器 + `bext`）, `ContainerKind::Bw64` 产出的是
//!   "BW64 标识 + `ds64` + `fmt ` + （浮点时 `fact`）+ `bext`"这一子集, 不是完整
//!   BS.2088 文件。

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::ops::Range;

/// RF64 的"真实长度在 ds64 里"哨兵值。
pub const SENTINEL_U32: u32 = 0xFFFF_FFFF;

/// `bext` v1/v2 的固定前缀长度（字节）。
pub const BEXT_FIXED_LEN: usize = 602;

/// `bext` v2 在固定前缀里额外的响度字段长度（5 × `i16`）。
pub const BEXT_V2_LOUDNESS_LEN: usize = 10;

/// `fact` chunk 的负载长度（字节）: 一个 `u32` 帧数。
///
/// FFmpeg 与 hound 的实现里这个字段都是一个 `u32`（`avio_wl32` / `read_le_u32`）。
pub const FACT_PAYLOAD_LEN: usize = 4;

/// 写进 `fact` chunk 的 `u32` 帧数 [ARCH-FMT-001]。
///
/// `sample_count` 是**帧数**（每声道采样数）, 与 [`Rf64Sizes::sample_count`] 同口径。
/// 严格小于 [`SENTINEL_U32`] 时直接写原值。
///
/// 取到 [`SENTINEL_U32`] 或更大时改写哨兵: 那个值在 `fact` 里已经是"未知/看 `ds64`"的
/// 记号, 把它**同时**当成一个精确计数会让两种含义无法区分。代价是"帧数恰好等于
/// `0xFFFF_FFFF`"这一格与哨兵重合 —— 这是 `u32` 字段借满值当哨兵的固有代价,
/// 不是本函数的取舍（FFmpeg 的判据是 `number_of_samples > UINT32_MAX`, 在那一格
/// 同样写出 `0xFFFF_FFFF`）。
///
/// 能取到哨兵分支只可能是 `RF64`/`BW64` —— `sample_count ≥ 0xFFFF_FFFF` 且浮点每样本
/// 至少 1 字节 ⇒ 负载**至少** 4 294 967 295 字节, 加上任何头部都超过 `data` 的 32 位
/// 上限, `for_payload` 必然升级到带 `ds64` 的容器, 而真值就写在 `ds64.sampleCount` 里。
const fn fact_frame_count(sample_count: u64) -> u32 {
    if sample_count < SENTINEL_U32 as u64 {
        sample_count as u32
    } else {
        SENTINEL_U32
    }
}

/// PDC/音频容器通用的小端读写工具。
mod le {
    /// 小端 `u16` -> 2 字节。
    pub fn u16(value: u16) -> [u8; 2] {
        value.to_le_bytes()
    }

    /// 小端 `u32` -> 4 字节。
    pub fn u32(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    /// 小端 `u64` -> 8 字节。
    pub fn u64(value: u64) -> [u8; 8] {
        value.to_le_bytes()
    }

    /// 从 `bytes` 的 `at..at+2` 读小端 `u16`。
    pub fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
        let slice = bytes.get(at..at + 2)?;
        Some(u16::from_le_bytes([slice[0], slice[1]]))
    }

    /// 从 `bytes` 的 `at..at+4` 读小端 `u32`。
    pub fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
        let slice = bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    /// 从 `bytes` 的 `at..at+8` 读小端 `u64`。
    pub fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
        let slice = bytes.get(at..at + 8)?;
        Some(u64::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
        ]))
    }
}

/// 容器种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    /// 标准 RIFF/WAVE（上限 4 GiB − 1）。
    Riff,
    /// EBU Tech 3306 的 RF64。
    Rf64,
    /// ITU-R BS.2088 的 BW64（本实现只产出其 `ds64` + `fmt ` + `bext` 子集）。
    Bw64,
}

impl ContainerKind {
    /// 顶层 fourcc。
    #[must_use]
    pub const fn fourcc(self) -> [u8; 4] {
        match self {
            Self::Riff => *b"RIFF",
            Self::Rf64 => *b"RF64",
            Self::Bw64 => *b"BW64",
        }
    }

    /// 由顶层 fourcc 判定容器。
    #[must_use]
    pub const fn from_fourcc(fourcc: [u8; 4]) -> Option<Self> {
        match &fourcc {
            b"RIFF" => Some(Self::Riff),
            b"RF64" => Some(Self::Rf64),
            b"BW64" => Some(Self::Bw64),
            _ => None,
        }
    }

    /// `true` 表示必须写 `ds64` 且把长度哨兵化。
    #[must_use]
    pub const fn uses_ds64(self) -> bool {
        matches!(self, Self::Rf64 | Self::Bw64)
    }
}

/// PCM 格式描述（`fmt ` chunk 的语义）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    /// 声道数。
    pub channels: u16,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 每样本**有效**位深 (16 / 24 / 32)。
    pub bits_per_sample: u16,
    /// `true` = IEEE 浮点容器 (`WAVE_FORMAT_IEEE_FLOAT`), `false` = 整数 PCM。
    pub is_float: bool,
    /// `WAVE_FORMAT_EXTENSIBLE` 的声道掩码; `None` 表示按声道数取标准掩码。
    pub channel_mask: Option<u32>,
}

impl PcmFormat {
    /// 构造一个整数 PCM 格式。
    #[must_use]
    pub const fn integer(channels: u16, sample_rate: u32, bits_per_sample: u16) -> Self {
        Self {
            channels,
            sample_rate,
            bits_per_sample,
            is_float: false,
            channel_mask: None,
        }
    }

    /// 构造一个 IEEE 浮点格式（本工程内部精度, 对应 32f）。
    #[must_use]
    pub const fn float(channels: u16, sample_rate: u32, bits_per_sample: u16) -> Self {
        Self {
            channels,
            sample_rate,
            bits_per_sample,
            is_float: true,
            channel_mask: None,
        }
    }

    /// 每样本占用字节数（容器字节数, 24-bit 是 3）。
    #[must_use]
    pub const fn bytes_per_sample(&self) -> u16 {
        self.bits_per_sample.div_ceil(8)
    }

    /// 帧对齐字节数（`nBlockAlign`）。
    ///
    /// **饱和**而不是回绕：`nBlockAlign` 在 `fmt ` 里是 `u16` 字段，而 `channels`
    /// 与 `bits_per_sample` 都来自文件或调用方，未受约束。本机实测：不饱和时
    /// `channels = 0xFFFF` 与 32-bit 的乘积在 debug 下 panic（`attempt to multiply
    /// with overflow`）。解析器另外会把"放不下"的布局判为
    /// [`Rf64Error::UnrepresentableBlockAlign`]，因此饱和值只会出现在**构造期**
    /// 由调用方给出的畸形格式上。
    #[must_use]
    pub const fn block_align(&self) -> u16 {
        self.channels.saturating_mul(self.bytes_per_sample())
    }

    /// 真实的帧对齐（`channels × bytes_per_sample`）能否装进 `fmt ` 的 `u16`
    /// `nBlockAlign` 字段。
    ///
    /// 这是**读取器与写入器共用的唯一谓词**：读取器
    /// （`parse_fmt_payload`）对"装不下"的布局返回
    /// [`Rf64Error::UnrepresentableBlockAlign`]，写入器
    /// （[`ContainerPlan::validate`]）对同一个谓词为假的计划在写任何字节之前拒绝。
    /// 两处各写一遍乘法就会漂移 —— 而漂移的后果正是"写入器写出一个读取器读不回的
    /// 容器"（见 [`ContainerPlan::validate`] 的文档）。
    ///
    /// 与 [`Self::block_align`] 的分工：那个是**写入用的饱和值**（永远不会回绕），
    /// 这个是**可表示性判定**（饱和恰恰说明不可表示）。
    #[must_use]
    pub const fn block_align_fits_u16(&self) -> bool {
        let exact = (self.channels as u32) * (self.bytes_per_sample() as u32);
        exact <= u16::MAX as u32
    }

    /// 每秒字节数（`nAvgBytesPerSec`）。与 [`Self::block_align`] 同源，同样**饱和**：
    /// `nAvgBytesPerSec` 是 `u32` 字段，而采样率与帧对齐都未受约束。
    #[must_use]
    pub const fn byte_rate(&self) -> u32 {
        self.sample_rate.saturating_mul(self.block_align() as u32)
    }

    /// 是否必须使用 `WAVE_FORMAT_EXTENSIBLE`。
    ///
    /// 单/双声道且未指定掩码时用普通的 `WAVE_FORMAT_PCM` /
    /// `WAVE_FORMAT_IEEE_FLOAT`（16 字节 `fmt `）; 其余情况用 EXTENSIBLE
    /// （40 字节 `fmt `）, 与 libsndfile/hound 的选择一致。
    #[must_use]
    pub const fn is_extensible(&self) -> bool {
        self.channels > 2 || self.channel_mask.is_some()
    }

    /// 规范格式标签。
    #[must_use]
    pub const fn format_tag(&self) -> u16 {
        if self.is_extensible() {
            0xFFFE
        } else if self.is_float {
            0x0003
        } else {
            0x0001
        }
    }

    /// `fmt ` chunk 的负载（16 或 40 字节）。
    #[must_use]
    pub fn fmt_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(40);
        out.extend_from_slice(&le::u16(self.format_tag()));
        out.extend_from_slice(&le::u16(self.channels));
        out.extend_from_slice(&le::u32(self.sample_rate));
        out.extend_from_slice(&le::u32(self.byte_rate()));
        out.extend_from_slice(&le::u16(self.block_align()));
        out.extend_from_slice(&le::u16(self.bits_per_sample));
        if self.is_extensible() {
            // WAVEFORMATEXTENSIBLE 的其余部分。
            out.extend_from_slice(&le::u16(22)); // cbSize
            out.extend_from_slice(&le::u16(self.bits_per_sample)); // wValidBitsPerSample
            let mask = self
                .channel_mask
                .unwrap_or_else(|| default_channel_mask(self.channels));
            out.extend_from_slice(&le::u32(mask));
            out.extend_from_slice(&subformat_guid(self.is_float));
        }
        out
    }
}

/// 按声道数给出常用声道掩码（与 libsndfile 的默认分配一致）。
///
/// 未列出的声道数返回 0, 即"无映射"（direct out）—— 这是**回退**, 不是猜测:
/// libsndfile 对未列出的声道数同样写 0。
#[must_use]
pub const fn default_channel_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,                                               // FC
        2 => 0x1 | 0x2,                                         // FL FR
        4 => 0x1 | 0x2 | 0x10 | 0x20,                           // FL FR BL BR
        6 => 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20,               // 5.1
        8 => 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x80, // 7.1
        _ => 0,
    }
}

/// `KSDATAFORMAT_SUBTYPE_PCM` / `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` 的 16 字节形式。
///
/// 字节序列取自 hound `src/lib.rs` 的同名常量（已逐字节核对）。
#[must_use]
pub const fn subformat_guid(is_float: bool) -> [u8; 16] {
    let first = if is_float { 0x03 } else { 0x01 };
    [
        first, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
        0x71,
    ]
}

/// `bext` 的 EBU R128 响度字段（版本 2）[ARCH-FMT-001]。
///
/// 单位: 除 `max_true_peak_level` 是 0.01 dBTP 外, 其余均为 **0.01 LUFS**。
/// 缩放由 [`Self::from_lufs`] / [`Self::from_dbtp`] 固定, 不由调用方手算。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loudness {
    /// 节目响度 `LoudnessValue`（0.01 LUFS）。
    pub loudness_value: i16,
    /// 响度范围 `LoudnessRange`（0.01 LU）。
    pub loudness_range: i16,
    /// 最大真峰值 `MaxTruePeakLevel`（0.01 dBTP）。
    pub max_true_peak_level: i16,
    /// 最大瞬时响度 `MaxMomentaryLoudness`（0.01 LUFS）。
    pub max_momentary_loudness: i16,
    /// 最大短时响度 `MaxShortTermLoudness`（0.01 LUFS）。
    pub max_short_term_loudness: i16,
}

impl Loudness {
    /// `bext` 里表示"该响度参数未知"的哨兵值。
    ///
    /// # 规范缺口（已在 notes 登记为 needs）
    ///
    /// EBU Tech 3285 的 PDF 在本机不可机读（`web_fetch` 拒绝 `application/pdf`）,
    /// 因此 v2 响度字段的"未知"哨兵没有从权威原文核验到。这里取 `i16::MIN`
    /// （`0x8000`）—— 它在 0.01 LUFS 刻度上远离任何真实测量值, 且不会被误读成
    /// 一个合法读数。**需要人类按 EBU Tech 3285 s5 附录裁决**后再定稿。
    pub const UNKNOWN: i16 = i16::MIN;

    /// 由 LUFS 值构造（×100, 四舍五入到 `i16`）。
    #[must_use]
    pub fn from_lufs(lufs: f32) -> i16 {
        scale_hundredths(lufs)
    }

    /// 由 dBTP 值构造（×100）。
    #[must_use]
    pub fn from_dbtp(dbtp: f32) -> i16 {
        scale_hundredths(dbtp)
    }

    /// 编码为 5 × `i16` 小端（10 字节）。
    #[must_use]
    pub fn to_bytes(self) -> [u8; BEXT_V2_LOUDNESS_LEN] {
        let mut out = [0u8; BEXT_V2_LOUDNESS_LEN];
        for (index, value) in [
            self.loudness_value,
            self.loudness_range,
            self.max_true_peak_level,
            self.max_momentary_loudness,
            self.max_short_term_loudness,
        ]
        .into_iter()
        .enumerate()
        {
            out[index * 2..index * 2 + 2].copy_from_slice(&le::u16(value as u16));
        }
        out
    }

    /// 从 10 字节解码。
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < BEXT_V2_LOUDNESS_LEN {
            return None;
        }
        let read = |index: usize| -> i16 { le::read_u16(bytes, index * 2).unwrap_or(0) as i16 };
        Some(Self {
            loudness_value: read(0),
            loudness_range: read(1),
            max_true_peak_level: read(2),
            max_momentary_loudness: read(3),
            max_short_term_loudness: read(4),
        })
    }
}

/// 把浮点值按 0.01 单位缩放到 `i16`, 越界饱和, `NaN` 归到 [`Loudness::UNKNOWN`]。
fn scale_hundredths(value: f32) -> i16 {
    if value.is_nan() {
        return Loudness::UNKNOWN;
    }
    let scaled = (value * 100.0).round();
    if scaled >= f32::from(i16::MAX) {
        i16::MAX
    } else if scaled <= f32::from(i16::MIN + 1) {
        i16::MIN + 1
    } else {
        scaled as i16
    }
}

/// Broadcast Extension (`bext`) 元数据块 [ARCH-FMT-001]。
///
/// 文字字段是定长 NUL 填充的字节串（规范未规定字符集; 这里按 ASCII/UTF-8 写入）。
/// 超长时**丢弃**多余部分, 不 panic; 但截断点落在**字符边界**上, 以免把一个多字节
/// 字符切成两半 —— 见 `push_fixed`。代价是最多少写一个字符, 字段的其余字节仍补 0。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bext {
    /// `Description`（256 字节）。
    pub description: String,
    /// `Originator`（32 字节）。
    pub originator: String,
    /// `OriginatorReference`（32 字节）。本工程用它承载工程 ULID（26 字符）。
    pub originator_reference: String,
    /// `OriginationDate`, ASCII `YYYY-MM-DD`（10 字节）。
    pub origination_date: String,
    /// `OriginationTime`, ASCII `HH:MM:SS`（8 字节）。
    pub origination_time: String,
    /// `TimeReference`（`Low` + `High` 两个 `u32` 拼成的 `u64`）: 首个采样点的
    /// 当日采样计数。
    pub time_reference: u64,
    /// `Version`。
    pub version: u16,
    /// `UMID`（64 字节原始）。
    pub umid: [u8; 64],
    /// 版本 2 的 EBU R128 响度字段; 版本 1 必须为 `None`。
    pub loudness: Option<Loudness>,
    /// `CodingHistory`（变长, 可以为空）。
    pub coding_history: String,
}

impl Default for Bext {
    fn default() -> Self {
        Self {
            description: String::new(),
            originator: String::new(),
            originator_reference: String::new(),
            origination_date: "1970-01-01".to_owned(),
            origination_time: "00:00:00".to_owned(),
            time_reference: 0,
            version: 1,
            umid: [0u8; 64],
            loudness: None,
            coding_history: String::new(),
        }
    }
}

/// `CodingHistory` 的字面内容。
///
/// BWF 的编码历史是一串逗号分隔的 `键=值`（`A=` 算法、`F=` 采样率 Hz、
/// `W=` 字长 bit、`M=` 声道、`T=` 自由文本）。**每一项都是可选的** —— 因此
/// "构造时还不知道的项"的正确写法是**省略**, 绝不是写一个 `<sample_rate>`
/// 这样的**占位符字面量**: 那会让文件声称它的采样率是一个尖括号标记。
///
/// 本 crate 里 `A=PCM` 与 `M=stereo` 是工程级选择（母带导出恒为立体声, 见
/// [`crate::mastering::MasterExportError::NotStereo`]）, `T=Yeban` 是发起者标记;
/// 只有 `F=` 与 `W=` 依赖导出格式, 因此在拿到格式前省略。
fn coding_history(sample_rate: Option<u32>, bits_per_sample: Option<u16>) -> String {
    let mut out = String::from("A=PCM");
    if let Some(sample_rate) = sample_rate {
        out.push_str(&format!(",F={sample_rate}"));
    }
    if let Some(bits_per_sample) = bits_per_sample {
        out.push_str(&format!(",W={bits_per_sample}"));
    }
    out.push_str(",M=stereo,T=Yeban");
    out
}

/// 把 `HH:MM:SS` 的起始时间码换成 `bext` 的 `TimeReference`（**采样数**）。
///
/// # 口径（核验过的上游事实）
///
/// ExifTool 的 RIFF 标签表把 `TimeReference`（`bext` 偏移 338）标为
/// "(first sample count since midnight)" —— 即"从当日零点到**首个采样点**之间的
/// **采样数**"。因此换算只有一次乘法:
///
/// ```text
/// TimeReference = (时·3600 + 分·60 + 秒) · 采样率
/// ```
///
/// 这里的"秒"是**整秒**。`bext` 没有小数秒字段, 而它的兄弟字段
/// `OriginationTime` 也是 8 字符的 `HH:MM:SS`, 所以本函数接受什么粒度就写什么粒度,
/// **不**把毫秒偷偷乘进去。
///
/// # 为什么返回 `Option` 而不是回落到 0
///
/// `TimeReference = 0` 是一个**合法值**, 它的含义是"首个采样点在当日零点"。
/// 因此"时间码写错了"绝不能回落成 0 —— 那正是本模块已在 `CodingHistory` 上禁止过的
/// 占位符做法（见私有函数 `coding_history` 的注释）: 文件会声称一件没发生的事。
/// 时间码不合语法时本函数返回 `None`, 由调用方决定拒绝还是显式用 0。
///
/// 小时 / 分钟必须 `< 60`, 秒必须 `< 60`, **小时还必须 `< 24`** —— 参照点是"当日
/// 零点", 而一天只有 24 小时, 因此 `25:00:00` 不是时刻（它是 1 天又 1 小时, 写进
/// `TimeReference` 就与"当日零点"这个定义自相矛盾）。
///
/// # 整串必须恰好是 `HH:MM:SS`: 每个字段**恰好两位 ASCII 数字**
///
/// 这条比"能算出一个数"严, 而它的来源是**返回值的去处**: 同一个字符串还会被
/// [`Bext::for_project_with_timecode`] 原样写进 `OriginationTime`（偏移 330 的
/// **8 字节** `HH:MM:SS` 字段）。`"1:2:3"` 能算出一个数（`1·3600 + 2·60 + 3` = 3723 秒）,
/// 但它写进那个字段后留下的字节是 `31 3A 32 3A 33 00 00 00` —— 一个 NUL 补位的
/// 5 字符串, 不是 ASCII `HH:MM:SS`。本机实测（修复前）:
///
/// ```text
/// time_reference_samples("1:2:3",      48_000) -> Some(178704000)
/// time_reference_samples("1:02:03",    48_000) -> Some(178704000)
/// time_reference_samples("+1:02:03",   48_000) -> Some(178704000)
/// time_reference_samples("001:02:03",  48_000) -> Some(178704000)
/// ```
///
/// 四者都被接受。因此本函数按 ASCII 的十进制数字**逐字节**解析（私有函数
/// `two_digit_field`）, 而不是交给 `str::parse`：后者接受 `1`、`+1`、`001`
/// 这些写法, 它们都算得出一个秒数, 却都不是一个合法的 `HH:MM:SS` 字段。判据是
/// `a_timecode_field_must_be_exactly_two_ascii_digits`（它同时钉住"写进
/// `OriginationTime` 的串恒为 8 字节 `HH:MM:SS`"这条后果）。
///
/// 本函数是纯整数运算（IEEE 精确类, 见 `docs/adr/ADR-0001` 的裁决口径）, 不碰浮点,
/// 因此跨架构逐位相同。
#[must_use]
pub fn time_reference_samples(start_timecode: &str, sample_rate: u32) -> Option<u64> {
    let mut fields = start_timecode.split(':');
    let hours = two_digit_field(fields.next()?)?;
    let minutes = two_digit_field(fields.next()?)?;
    let seconds = two_digit_field(fields.next()?)?;
    if fields.next().is_some() || hours >= 24 || minutes >= 60 || seconds >= 60 {
        return None;
    }
    let rate = u64::from(sample_rate);
    (hours * 3600 + minutes * 60 + seconds).checked_mul(rate)
}

/// 解析 `HH:MM:SS` 的三个字段之一: **恰好两位 ASCII 数字**（`00`..=`99`）。
///
/// 为什么不是 `field.parse::<u64>()`: 它接受 `"1"` / `"+1"` / `"001"` 这些
/// **不是** `OriginationTime` 那种 8 字节 ASCII `HH:MM:SS` 的写法。本函数的返回值会
/// 被 [`Bext::for_project_with_timecode`] 写进定长字段, 因此"能算出一个数"不等于
/// "写出来是一个合法字段"（见 [`time_reference_samples`] 的实测表）。
///
/// 位宽检查先于数值检查: 返回的值因此恒在 `0..=99`, 调用方不需要再防溢出。
fn two_digit_field(field: &str) -> Option<u64> {
    let bytes = field.as_bytes();
    if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let tens = u64::from(bytes[0] - b'0');
    let ones = u64::from(bytes[1] - b'0');
    Some(tens * 10 + ones)
}

impl Bext {
    /// 为一次母带导出构造 `bext` 块: 工程 ULID 写进 `OriginatorReference`。
    ///
    /// # 映射裁决（不是发明的字段）
    ///
    /// [ARCH-FMT-001] 要求 `bext` 内嵌"工程 ULID 全局唯一标识", 但 EBU Tech 3285
    /// 的 `bext` **没有** ULID 字段。可用的 32 字节 `OriginatorReference` 是唯一
    /// 长度放得下 26 字符 ULID 的自由文本字段, 因此选它承载, 并登记在 notes 的
    /// `needs`: 若后续引入 `axml`（BS.2088 的 `<ULID>` 元素是更规范的落点），
    /// 应改为写 `axml` 同时保留 `OriginatorReference` 以向后兼容。
    ///
    /// # `CodingHistory`
    ///
    /// 本函数在构造时**不知道**采样率与字长, 因此它的编码历史是
    /// `A=PCM,M=stereo,T=Yeban` —— 只写已知项, **不写占位符**。
    /// 需要把 `F=`/`W=` 一并写进交付文件的调用方用
    /// [`Self::for_project_with_format`]。
    ///
    /// # `TimeReference`（本函数的默认值是 0, 且这是一个**声明**）
    ///
    /// 本函数把 `TimeReference`（首个采样点距当日零点的**采样数**）留在
    /// [`Self::default`] 的 `0`。0 在 BWF 里是一个合法读数, 它的含义是
    /// "首个采样点在当日零点" —— 对一个从工程起点（tick 0）导出的母带而言,
    /// 这正是本 crate 现有调用方的情形（数据模型里没有会话起始时间码字段,
    /// 见 `docs/ledger/render-master-notes.md` 的 `needs`）。
    ///
    /// **调用方若知道起始时间码, 不要用本函数再手改字段** —— 用
    /// [`Self::for_project_with_timecode`], 它把时间码、`OriginationTime` 与
    /// `TimeReference` 一起写, 三者不会互相矛盾。
    #[must_use]
    pub fn for_project(ulid: &str, origination_date: &str, origination_time: &str) -> Self {
        Self {
            originator: "Yeban DAW".to_owned(),
            originator_reference: ulid.to_owned(),
            origination_date: origination_date.to_owned(),
            origination_time: origination_time.to_owned(),
            version: 2,
            // 导出时尚未测量响度: 按哨兵 `UNKNOWN` 写入, 而不是写一个 0.0 LUFS 的假读数。
            // 母带链测出真值后应改写这些字段 (见 `Loudness::from_lufs` / `from_dbtp`)。
            loudness: Some(Loudness {
                loudness_value: Loudness::UNKNOWN,
                loudness_range: Loudness::UNKNOWN,
                max_true_peak_level: Loudness::UNKNOWN,
                max_momentary_loudness: Loudness::UNKNOWN,
                max_short_term_loudness: Loudness::UNKNOWN,
            }),
            coding_history: coding_history(None, None),
            ..Self::default()
        }
    }

    /// 与 [`Self::for_project`] 相同, 但 `CodingHistory` 携带**真实的**导出格式参数。
    ///
    /// # 与 `for_project` 的唯一差别
    ///
    /// `A=PCM,F=<sampling frequency>,W=<word length>,M=stereo,T=Yeban`。
    /// `F=` 与 `W=` 取自**调用方实际会写出的**格式（`sample_rate` 是 Hz,
    /// `bits_per_sample` 是位）。模板函数 `for_project` 在构造时并不知道这两项,
    /// 因此它只能给出不带 `F=`/`W=` 的诚实子集; 想要完整编码历史的调用方用本函数。
    ///
    /// # 为什么要单独一个入口（实测的缺口）
    ///
    /// 导出路径上唯一知道格式的地方就是导出调用本身。本 crate 的两条下游都因此
    /// 被迫在**别处**手工拼这条字符串, 例如
    /// `crates/yeban-mcp/src/domain/render.rs:1279` 自己 `format!` 了一份
    /// （并在注释里点名 `for_project` 的编码历史"是模板字符串(带 `<sample_rate>`
    /// 字面量)"）。把"真实参数"这一档做成 crate 自己的构造器, 下游就不必各自复刻。
    ///
    /// 参数**不做校验**: 本函数无法知道 `bits_per_sample` 是否是一个本 crate 能写出的
    /// 位深, 它只如实转录调用方声称的格式（与 [`PcmFormat`] 一样不设白名单）。
    #[must_use]
    pub fn for_project_with_format(
        ulid: &str,
        origination_date: &str,
        origination_time: &str,
        sample_rate: u32,
        bits_per_sample: u16,
    ) -> Self {
        Self {
            coding_history: coding_history(Some(sample_rate), Some(bits_per_sample)),
            ..Self::for_project(ulid, origination_date, origination_time)
        }
    }

    /// 与 [`Self::for_project_with_format`] 相同, 但把**起始时间码**一并写成
    /// `TimeReference`。
    ///
    /// `start_timecode` 是 `HH:MM:SS`。它同时被写进两个字段, 因此不可能互相矛盾:
    ///
    /// - `OriginationTime`（偏移 330, 8 字符 ASCII）—— `bext` 的**定长**字段;
    /// - `TimeReference`（偏移 338, `u64`）—— 换算见 [`time_reference_samples`]。
    ///
    /// # 这是 [ARCH-FMT-001] 三项里唯一没有**构造器落点**的那一项
    ///
    /// > 完整内嵌广播级 `bext` (Broadcast Extension) 元数据块（**录制起始时间码**、
    /// > 响度元数据 EBU R128、工程 ULID 全局唯一标识）
    /// > —— `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.2（现位于第 461 行）
    ///
    /// 三项里"响度"由 [`Self::for_project`] 的响度哨兵 + 母带链的实测改写承载,
    /// "ULID"由 `OriginatorReference` 承载（映射裁决见 [`Self::for_project`]）,
    /// 而"录制起始时间码"此前**没有构造器写过**: [`Self::for_project`] 与
    /// [`Self::for_project_with_format`] 都把 [`Self::time_reference`] 留在
    /// [`Self::default`] 的 `0`, 于是文件一边声称发起时间是 `13:37:00`、一边声称
    /// 首个采样点在当日零点。本函数补上这个落点。
    ///
    /// **本函数不改写既有构造器**（那会改变下游交付文件的字节, 而"起始时间码"至今没有
    /// 数据模型字段, 见 `docs/ledger/render-master-notes.md` 的 `needs`）。想知道起始
    /// 时间码的调用方换用本函数; 仍用 `for_project` 的调用方继续得到 `0`（它也是
    /// [`Self::for_project`] 文档里那条**声明**）。
    ///
    /// # Errors
    ///
    /// `start_timecode` 不合 `HH:MM:SS` 语法（或时/分/秒越界）⇒
    /// [`Rf64Error::BadStartTimecode`]。**不**回落成 0: 见
    /// [`time_reference_samples`] 的"为什么返回 `Option` 而不是回落到 0"。
    /// `sample_rate == 0` 是合法的（换算结果 0）, 由调用方按格式约束负责。
    pub fn for_project_with_timecode(
        ulid: &str,
        origination_date: &str,
        start_timecode: &str,
        sample_rate: u32,
        bits_per_sample: u16,
    ) -> Result<Self, Rf64Error> {
        let time_reference = time_reference_samples(start_timecode, sample_rate)
            .ok_or_else(|| Rf64Error::BadStartTimecode(start_timecode.to_owned()))?;
        Ok(Self {
            time_reference,
            ..Self::for_project_with_format(
                ulid,
                origination_date,
                start_timecode,
                sample_rate,
                bits_per_sample,
            )
        })
    }

    /// 固定前缀长度。
    ///
    /// **版本 1 与版本 2 都是 602 字节**: 版本 2 把 190 字节保留区缩为 180 字节,
    /// 腾出的 10 字节正好放 5 个 `i16` 响度字段, 总长不变。这一点由两条独立事实
    /// 互证 —— ExifTool 的 RIFF 表把 `CodingHistory` 恒定标在偏移 602,
    /// 而 EBU Tech 3285 的补充说明写的是 "180 bytes reserved for extension"。
    #[must_use]
    pub const fn fixed_len(&self) -> usize {
        BEXT_FIXED_LEN
    }

    /// [`Self::to_bytes`] 会写出的负载长度（`fixed_len() + CodingHistory` 的字节数）。
    ///
    /// 这是**纯算术**的读数, 与 `to_bytes` 末尾的 `debug_assert_eq!` 同一口径。
    /// [`ContainerPlan::for_payload`] 用它算头部长度, 因此**不需要**在那里调用
    /// `to_bytes` —— 后者对畸形的块会 panic, 而"头部有多长"这个问题不该顺带回答
    /// "这个块能不能写"（那由 [`ContainerPlan::validate`] 回答, 出口是 `Result`）。
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        self.fixed_len() + self.coding_history.len()
    }

    /// 编码为 `bext` chunk 的负载（固定前缀 + coding history）。
    ///
    /// # Panics
    ///
    /// `version` 不在 `{1, 2}` 里、`version == 2 && loudness.is_none()`, 以及
    /// `version == 1 && loudness.is_some()` 时 panic —— 这些是**调用方的构造错误**,
    /// 静默写一个自相矛盾的 chunk 会让下游解析器读到垃圾。
    ///
    /// # 写入器只写 [`Self::from_bytes`] 读得回的版本
    ///
    /// 上界与下界都是必需的, 而且必须是**同一个**集合 `{1, 2}`。修复前只有下界
    /// （`version >= 1`）, 于是 `version = 3` / `4` 会被照原样写进文件 —— 本机实测:
    /// `to_bytes` 写出的 `version` 字段就是 3, 而同一份字节交给 [`Self::from_bytes`]
    /// 得到 [`Rf64Error::UnsupportedBextVersion`]。导出器因此能产出一个**本 crate
    /// 自己读不回来**的容器, 而调用方拿到的是成功。判据是
    /// `the_writer_refuses_a_version_the_reader_cannot_read`; 导出路径上的对应落点是
    /// [`crate::mastering::MasterExportError::UnsupportedBextVersion`]。
    ///
    /// # 文本字段里含 NUL 也 panic（写入器只写读取器读得回同一个值的块）
    ///
    /// 五个定长文本字段是 **NUL 补位**的定长字段, `CodingHistory` 是变长字段而读取器
    /// 会裁掉它的尾随 NUL（RIFF 的偶数字节补位就是那个 `0`, 两者在字节层无法区分）。
    /// 因此一个**含 NUL** 的取值在这个编码里**无法表示**: [`Self::from_bytes`] 的读取在
    /// 第一个 NUL 处停止, 于是"写出去的值"与"读回来的值"不是同一个字符串。
    /// 本机实测（修复前, 六种形态各自都是这一结果）:
    ///
    /// ```text
    /// Description      "a\0b"        -> 读回 "a"      (往返不等)
    /// Originator       "Yeban\0DAW"  -> 读回 "Yeban"  (往返不等)
    /// OriginationTime  "13\0:37:00"  -> 读回 "13"     (往返不等)
    /// CodingHistory    "A=PCM\0"     -> 读回 "A=PCM"  (往返不等, 尾随 NUL 被裁掉)
    /// ```
    ///
    /// 判定落在纯函数 [`Self::field_that_does_not_round_trip`]（判据可以直接读它）;
    /// 导出路径上的落点是
    /// [`crate::mastering::MasterExportError::UnrepresentableBextField`]。
    /// 这是私有函数 `push_fixed` 那条"截断点必须落在字符边界上"的同一条纪律的另一半:
    /// **写入器不静默改写调用方给的字段值**。
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        assert!(
            matches!(self.version, 1 | 2),
            "bext 版本 {} 的字段表未核验, 拒绝写入: 读取器只接受 1 与 2, \
             写出去就是一个本 crate 读不回来的容器 (见 render-master-notes needs)",
            self.version
        );
        if self.version >= 2 {
            assert!(
                self.loudness.is_some(),
                "bext 版本 {version} 必须提供 EBU R128 响度字段",
                version = self.version
            );
        } else {
            assert!(
                self.loudness.is_none(),
                "bext 版本 1 没有响度字段, 不应提供"
            );
        }
        if let Some(field) = self.field_that_does_not_round_trip() {
            panic!(
                "bext 的 {field} 取值含 NUL: 字段是 NUL 终止的, 读取器会在第一个 NUL 处截断, \
                 写出去的值与读回来的值不是同一个字符串 (拒绝写入)"
            );
        }

        let mut out = Vec::with_capacity(self.encoded_len());
        push_fixed(&mut out, &self.description, 256);
        push_fixed(&mut out, &self.originator, 32);
        push_fixed(&mut out, &self.originator_reference, 32);
        push_fixed(&mut out, &self.origination_date, 10);
        push_fixed(&mut out, &self.origination_time, 8);
        out.extend_from_slice(&le::u64(self.time_reference));
        out.extend_from_slice(&le::u16(self.version));
        out.extend_from_slice(&self.umid);
        if let Some(loudness) = self.loudness {
            out.extend_from_slice(&loudness.to_bytes());
            out.extend_from_slice(&[0u8; 180]);
        } else {
            out.extend_from_slice(&[0u8; 190]);
        }
        out.extend_from_slice(self.coding_history.as_bytes());
        debug_assert_eq!(out.len(), self.encoded_len());
        out
    }

    /// 第一个**读不回同一个值**的文本字段名; 全部字段都能往返 ⇒ `None`。
    ///
    /// # 判定规则（与 [`Self::from_bytes`] 的读取规则一一对应）
    ///
    /// | 字段 | 不能往返的取值 |
    /// | :--- | :--- |
    /// | `Description` / `Originator` / `OriginatorReference` / `OriginationDate` / `OriginationTime` | 含 NUL（读取器在第一个 NUL 处停止, 其余字节读不回来） |
    /// | `CodingHistory` | **以 NUL 结尾**（读取器的 `trim_end_matches('\0')` 会裁掉它; 中间的 NUL 不受影响） |
    ///
    /// 除这些文本字段之外的字节（`TimeReference` / `Version` / `UMID` / 响度块）都是
    /// 二进制字段, 不存在这条问题。
    ///
    /// # 为什么只查"读取器真的会读丢"的形态
    ///
    /// 定长字段的**超长截断**是**有意的**（见私有函数 `push_fixed`: 字段宽 256/32/32/10/8 字节,
    /// 写不下的部分按字符边界丢掉）, 因此它不在这里; 而 `CodingHistory` 的**尾随** NUL
    /// 是另一个来源: 读取器必须裁掉它, 因为奇数长度的 `bext` 负载会被 RIFF 的偶数字节
    /// 补位补一个 `0` —— 那个补位字节落在负载里。于是"调用方给的尾随 NUL"与"补位字节"
    /// 在字节层无法区分, 只能拒绝前者。
    ///
    /// 返回**字段名**而不是 `bool`: 调用方（导出错误、判据）要能点名是哪个字段。
    #[must_use]
    pub fn field_that_does_not_round_trip(&self) -> Option<&'static str> {
        [
            ("Description", self.description.as_str()),
            ("Originator", self.originator.as_str()),
            ("OriginatorReference", self.originator_reference.as_str()),
            ("OriginationDate", self.origination_date.as_str()),
            ("OriginationTime", self.origination_time.as_str()),
        ]
        .into_iter()
        .find(|(_, value)| value.contains('\0'))
        .map(|(name, _)| name)
        .or_else(|| {
            self.coding_history
                .ends_with('\0')
                .then_some("CodingHistory")
        })
    }

    /// 从 `bext` chunk 负载解码。
    ///
    /// 只接受版本 1 与 2; 其他版本返回 [`Rf64Error::UnsupportedBextVersion`]。
    ///
    /// # Errors
    ///
    /// 负载过短或版本不受支持。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Rf64Error> {
        if bytes.len() < BEXT_FIXED_LEN {
            return Err(Rf64Error::Truncated {
                what: "bext 固定前缀 (需要 602 字节)",
                got: bytes.len(),
            });
        }
        let text = |at: usize, len: usize| -> String {
            let slice = &bytes[at..at + len];
            let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
            String::from_utf8_lossy(&slice[..end]).into_owned()
        };
        let version = le::read_u16(bytes, 346).unwrap_or(0);
        let mut umid = [0u8; 64];
        umid.copy_from_slice(&bytes[348..412]);
        let (loudness, coding_history_at) = match version {
            1 => (None, BEXT_FIXED_LEN),
            2 => {
                if bytes.len() < BEXT_FIXED_LEN {
                    return Err(Rf64Error::Truncated {
                        what: "bext v2 前缀",
                        got: bytes.len(),
                    });
                }
                let loudness = Loudness::from_bytes(&bytes[412..422]);
                (loudness, BEXT_FIXED_LEN)
            }
            other => return Err(Rf64Error::UnsupportedBextVersion(other)),
        };
        let coding_history =
            String::from_utf8_lossy(bytes.get(coding_history_at..).unwrap_or_default())
                .trim_end_matches('\0')
                .to_owned();

        Ok(Self {
            description: text(0, 256),
            originator: text(256, 32),
            originator_reference: text(288, 32),
            origination_date: text(320, 10),
            origination_time: text(330, 8),
            time_reference: le::read_u64(bytes, 338).unwrap_or(0),
            version,
            umid,
            loudness,
            coding_history,
        })
    }
}

/// 把 `value` 写进 `width` 字节的定长字段, 超长即截断, 其余补 0。
///
/// 截断点必须落在**字符边界**上, 不能落在任意字节位置上。字段宽的单位是字节, 而按
/// 字节硬截会把一个多字节 UTF-8 序列从中间切断 —— 实测: `"→"`（`E2 86 92`, 3 字节）
/// 重复 200 次写进 256 字节的 `Description` 时, 第 256 字节是**孤立的前导字节**
/// `E2`, 那一段字节不是合法文本; 读回来时 [`Bext::from_bytes`] 的 `from_utf8_lossy`
/// 只能把它换成 U+FFFD, 于是"写出去的名字"与"读回来的名字"不是同一个字符串。
///
/// 因此这里取**不超过 `width` 的最大字符边界**: 代价是最多少写一个字符（剩下的字节
/// 补 0）, 换来字段的有效区始终是合法 UTF-8。纯 ASCII 输入不受影响 —— 每个字节都是
/// 字符边界 —— 因此既有的 ASCII 产物**逐字节不变**, 仍恰好占满 `width`。
fn push_fixed(out: &mut Vec<u8>, value: &str, width: usize) {
    let raw = value.as_bytes();
    let mut take = raw.len().min(width);
    while take > 0 && !value.is_char_boundary(take) {
        take -= 1;
    }
    out.extend_from_slice(&raw[..take]);
    out.resize(out.len() + (width - take), 0);
}

/// `ds64` chunk 的三个 64 位长度 [ARCH-FMT-001]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rf64Sizes {
    /// `riffSize`: 整个文件长度减去顶层 8 字节头。
    pub riff_size: u64,
    /// `dataSize`: `data` chunk 的负载长度。
    pub data_size: u64,
    /// `sampleCount`: **帧数**（每声道采样数, 与 `fact` chunk 的口径一致）。
    pub sample_count: u64,
}

impl Rf64Sizes {
    /// `ds64` chunk 的固定负载长度: 8 + 8 + 8 + 4。
    pub const PAYLOAD_LEN: usize = 28;

    /// 编码为 28 字节（`tableLength` 恒为 0）。
    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::PAYLOAD_LEN] {
        let mut out = [0u8; Self::PAYLOAD_LEN];
        out[0..8].copy_from_slice(&le::u64(self.riff_size));
        out[8..16].copy_from_slice(&le::u64(self.data_size));
        out[16..24].copy_from_slice(&le::u64(self.sample_count));
        out[24..28].copy_from_slice(&le::u32(0));
        out
    }

    /// 从 28 字节解码。
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::PAYLOAD_LEN {
            return None;
        }
        Some(Self {
            riff_size: le::read_u64(bytes, 0)?,
            data_size: le::read_u64(bytes, 8)?,
            sample_count: le::read_u64(bytes, 16)?,
        })
    }
}

/// 写出一个容器所需的全部信息（不含音频负载本身）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerPlan {
    /// 容器种类。
    pub kind: ContainerKind,
    /// PCM 格式。
    pub format: PcmFormat,
    /// `ds64` 的长度三元组（RIFF 容器下仍会被计算, 只是不写进文件）。
    pub sizes: Rf64Sizes,
    /// 可选的 `bext` 块。
    pub bext: Option<Bext>,
}

impl ContainerPlan {
    /// 按负载长度与帧数构造计划, 并在超过 4 GiB 时自动切到 RF64。
    ///
    /// `preferred` 是"如果放得下就用它"; 放不下时升到 [`ContainerKind::Rf64`]
    /// （RF64 比 RIFF 多 36 字节头, 因此升级不会让长度回落到 4 GiB 以内）。
    ///
    /// 这就是 [ROAD-M4-006] "超过 4GB 时走 RF64 分支"的可测形式: 传入一个
    /// **假想**的大 `payload_len` 即可, 不需要真的写 4 GiB。
    #[must_use]
    pub fn for_payload(
        preferred: ContainerKind,
        format: PcmFormat,
        payload_len: u64,
        frame_count: u64,
        bext: Option<Bext>,
    ) -> Self {
        // chunk 的总占用 = 8 (头) + 负载 + 必要的偶数补位字节。
        let fmt_total = chunk_total(format.fmt_payload().len());
        // 非 PCM（浮点）必须带 `fact` —— 见模块头的核验表。整数 PCM 不写它:
        // FFmpeg 的条件就是 `codec_tag != 0x01`, 而 PCM 的 `fact` 是冗余的
        // （hound `src/write.rs` 的原话: "For all the formats that Hound can write,
        // the fact chunk is redundant"）。
        let fact_total = if format.is_float {
            chunk_total(FACT_PAYLOAD_LEN)
        } else {
            0
        };
        // 头部长度用**算术**算, 不调 `Bext::to_bytes` —— 后者对畸形的 `bext` 块会
        // panic（见它的 `# Panics`）。头部长度算术不该把"这个块能不能写"提前引爆:
        // 一个不可写的块现在能构造出计划, 再由 [`Self::validate`] 在**写任何字节之前**
        // 明确拒绝。`Bext::encoded_len` 与 `to_bytes` 的 `debug_assert_eq!` 同一口径。
        let bext_total = bext
            .as_ref()
            .map_or(0, |block| chunk_total(block.encoded_len()));
        let header_no_ds64 = 12 + fmt_total + fact_total + bext_total + 8;
        // RIFF 的长度字段包含 chunk 之间的偶数补位字节, 因此 data 负载的补位也要算进去。
        //
        // 与读取器同一族的纪律: 这一段全部用**饱和**加法。`payload_len` 是调用方给的
        // `u64`, `u64::MAX` 附近的输入会让 `payload_len + (payload_len % 2)` 在 debug 下
        // panic、在 release 下回绕。饱和之后 `for_payload` 对**任意** `u64` 都是全函数
        // （判据 `an_impossible_payload_length_saturates_instead_of_panicking`）;
        // 真实调用只传得出实际负载长度, 因此这条只影响畸形输入。
        let payload_total = payload_len.saturating_add(payload_len % 2);
        let projected = (header_no_ds64 as u64).saturating_add(payload_total);
        let kind = if projected > u64::from(SENTINEL_U32) && !preferred.uses_ds64() {
            ContainerKind::Rf64
        } else {
            preferred
        };
        let header_len = header_no_ds64 + if kind.uses_ds64() { 36 } else { 0 };
        let riff_size = (header_len as u64)
            .saturating_add(payload_total)
            .saturating_sub(8);
        let plan = Self {
            kind,
            format,
            sizes: Rf64Sizes {
                riff_size,
                data_size: payload_len,
                sample_count: frame_count,
            },
            bext,
        };
        // 头部长度自洽性只在**可写**的计划上要求: 一个 [`Self::validate`] 拒绝的计划
        // 会被 `write_container` 在写任何字节之前拒绝, 它的头部长度算得对不对无关紧要
        // —— 而 `header_bytes` 对畸形 `bext` 字段会 panic（`to_bytes` 的既有纪律）,
        // 那条 panic 不该在这里被当成"长度算错了"。
        debug_assert!(
            plan.validate().is_err() || plan.header_bytes().len() == header_len,
            "头部长度计算必须与实际写出的字节数一致"
        );
        plan
    }

    /// 这份计划写出的容器, 本 crate 的读取器是否读得回来。
    ///
    /// # 为什么写入器需要这一条（实测的缺口）
    ///
    /// [`Self::for_payload`] 对格式**不做校验**（它的返回值不是 `Result`）, 于是
    /// 修复前 [`write_container`] 会为一个读取器**必然拒绝**的 `fmt ` 写出完整的容器
    /// 并返回 `Ok(())` —— 调用方拿到成功, 交付物却是一个本 crate 自己读不回来的文件。
    /// 本机实测（判据
    /// `a_format_the_writer_accepts_the_reader_reads_back_identically` 的输入网格,
    /// 每种格式都真的调 `write_container` 再调 `parse_container`）:
    ///
    /// ```text
    /// channels = 0    bits = 16 -> write_container Ok(写出 52 字节), parse_container Err(ZeroChannels)
    /// channels = 0xFFFF bits = 32 -> write_container Ok(写出 76 字节), parse_container Err(UnrepresentableBlockAlign { .. })
    /// bext Description = "a\0b" -> write_container Ok(写出 662 字节), 读回的 Description 是 "a"
    /// ```
    ///
    /// 两类拒绝**就是**读取器的两个既有变体（当初为读取器写的）:
    ///
    /// 1. `channels == 0` ⇒ [`Rf64Error::ZeroChannels`];
    /// 2. 帧对齐放不进 `u16` ⇒ [`Rf64Error::UnrepresentableBlockAlign`], 判定走
    ///    [`PcmFormat::block_align_fits_u16`] —— 与读取器**共用同一个谓词**;
    /// 3. `bext` 文本字段含 NUL ⇒ [`Rf64Error::UnrepresentableBextField`]
    ///    （读取器读回的不是同一个字符串, 见
    ///    [`Bext::field_that_does_not_round_trip`]）。
    ///
    /// # 第四类: `sample_rate == 0`（本轮补上, 来源是 crate 内两条写入器的分歧）
    ///
    /// 前三类都是"本 crate 的读取器会拒绝"。第四类不同, 它的来源是 [`crate::wav`]:
    /// 同一个 crate 的**另一条**写入器 `write_plain_wav` 走 `hound`, 而
    /// [`crate::wav::check_container_fields`] 明确拒绝 `sample_rate == 0`, 文档里写明
    /// 理由是 "hound 写 `nBlockAlign` 时 **现位于第 332 行**做
    /// `bytes_per_sec / spec.sample_rate`, 除数为 0 ⇒ panic"。本模块的
    /// [`Self::byte_rate`] 是**饱和**乘法而不是除法, 因此不会 panic —— 它写出
    /// `nAvgBytesPerSec = 0` 的 `fmt `, 一个任何符合规范的解码器都打不开的容器。
    /// 本机实测（修复前, 判据
    /// `a_zero_sample_rate_is_refused_before_any_byte`）:
    ///
    /// ```text
    /// RIFF ch=2 bits=16 rate=0 -> validate Ok(()), write_container Ok(写出 60 字节), parse_container Ok(sample_rate = 0)
    /// ```
    ///
    /// 即本 crate 的读取器**读得回来**, 所以这一条不是对称性判据, 而是"同一 crate 的
    /// 两条写入器不得对同一个显然不可用的格式给出两个判决"。
    ///
    /// # 第五类: `bits_per_sample == 0`（本轮补上, 与上一类同一个来源）
    ///
    /// [`PcmFormat::bytes_per_sample`] 对 0 位给 0, 于是 `nBlockAlign` 与
    /// `nAvgBytesPerSec` 都被写成 0 —— 又是一个"合规解码器按 `nBlockAlign` 求帧数就是
    /// 除零"的容器。同一个 crate 走 `hound` 的写入器
    /// （[`crate::wav::check_container_fields`]）早已把它判为 `UnsupportedDepth(0)`。
    /// 与上一类不同的是: 本模块的**读取器也在本轮同步拒绝**它
    /// （[`Rf64Error::ZeroBitsPerSample`]）, 因为它此前会把 `data` 的**字节数**当成帧数
    /// —— `data_size` 除以被 `max(1)` 兜住的 `nBlockAlign`, 于是 8 字节负载被报告成
    /// **8 帧**。本机实测（修复前, 判据
    /// `a_zero_bit_depth_is_refused_before_any_byte`）:
    ///
    /// ```text
    /// RIFF ch=2 bits=0 rate=48000 -> validate Ok(()), write_container Ok(写出 52 字节), nBlockAlign = 0, nAvgBytesPerSec = 0, parse_container Ok(bits_per_sample = 0, sample_count = 8) —— 负载只有 8 字节
    /// ```
    ///
    /// 这与 [`Bext::to_bytes`] 那条"写入器只写 [`Bext::from_bytes`] 读得回的版本"
    /// 是同一条纪律: **写入器与读取器的接受集必须是同一个**。
    ///
    /// # 第六类: `bext` 的版本与响度块矛盾, 或版本根本不受支持（本轮补上）
    ///
    /// 前五类都是"本 crate 的读取器会**返回 `Err`**"或"外部解码器打不开"。第六类不同:
    /// 读取器对这两种形态根本读不出一个值, 而 [`Bext::to_bytes`] 对它们**直接 panic**
    /// （那是该函数文档写明的 `# Panics`）。因此修复前 [`write_container`] 不是"写出坏
    /// 字节", 而是**在写出任何字节之前 panic** —— 一个返回 `Result` 的写入器不该 panic。
    ///
    /// 判定与 [`Bext::to_bytes`] 的三条 `assert!` 一一对应, 因此
    /// `validate() == Ok(())` **蕴含** [`Self::header_bytes`] 不会 panic
    /// （`header_bytes` 的文档正是这句"它信任计划合法"）:
    ///
    /// 1. `version ∉ {1, 2}` ⇒ [`Rf64Error::UnsupportedBextVersion`] —— 与
    ///    [`Bext::from_bytes`] 用的是**同一个**变体, 因此"写入器与读取器的接受集是同一个
    ///    `{1, 2}`"在**计划**这一层也成立（此前只在 [`Bext`] 这一层成立）;
    /// 2. `version >= 2` 却 `loudness.is_none()`, 或 `version == 1` 却
    ///    `loudness.is_some()` ⇒ [`Rf64Error::BextLoudnessVersionMismatch`] ——
    ///    读取器对版本恒给出固定的响度取值, 这两种形态读不回同一个值。
    ///
    /// 顺序放在前五条**之后**: 一个同时含 NUL 字段与坏版本的块仍然先报字段那条（既有的
    /// 判据因此逐条不变）。
    ///
    /// # Errors
    ///
    /// 上面六类。`Ok(())` ⇒ [`write_container`] 写出的字节能被 [`parse_container`]
    /// 读回, 且 `fmt ` 的四个字段逐字段相同。
    pub fn validate(&self) -> Result<(), Rf64Error> {
        if self.format.channels == 0 {
            return Err(Rf64Error::ZeroChannels);
        }
        if self.format.sample_rate == 0 {
            return Err(Rf64Error::ZeroSampleRate);
        }
        // 与上面两条同一个位置（写任何字节之前）。判定与读取器侧的
        // `parse_fmt_payload` 用的是**同一个字段**, 因此两条路的接受集不会漂移。
        if self.format.bits_per_sample == 0 {
            return Err(Rf64Error::ZeroBitsPerSample);
        }
        if !self.format.block_align_fits_u16() {
            return Err(Rf64Error::UnrepresentableBlockAlign {
                channels: self.format.channels,
                bytes_per_sample: self.format.bytes_per_sample(),
            });
        }
        if let Some(field) = self
            .bext
            .as_ref()
            .and_then(Bext::field_that_does_not_round_trip)
        {
            return Err(Rf64Error::UnrepresentableBextField { field });
        }
        if let Some(block) = self.bext.as_ref() {
            if !matches!(block.version, 1 | 2) {
                return Err(Rf64Error::UnsupportedBextVersion(block.version));
            }
            if block.loudness.is_some() != (block.version >= 2) {
                return Err(Rf64Error::BextLoudnessVersionMismatch {
                    version: block.version,
                    has_loudness: block.loudness.is_some(),
                });
            }
        }
        Ok(())
    }

    /// 构造头部字节（`data` chunk 头之后、音频负载之前的一切）。
    ///
    /// chunk 顺序: 可选 `ds64` → `fmt ` → 可选 `fact`（**仅非 PCM**）→ 可选 `bext`
    /// → `data` 头。`fact` 的位置与"仅非 PCM"这两个约束都来自模块头核验表里的
    /// FFmpeg 语句。
    ///
    /// 这是个**纯函数** —— 大尺寸场景的全部判据都打在它身上, 因此不需要真的写
    /// 4 GiB。
    ///
    /// **它信任计划合法**: 返回值不是 `Result`, 因此调用方在直接用它拼文件之前要先过
    /// [`ContainerPlan::validate`]。走 [`write_container`] 的调用方已经过了。
    ///
    /// # "信任计划合法"是一句**可执行**的话（本轮补上）
    ///
    /// 上面那句话此前有一个反例: 一个 `bext` 版本不受支持（或版本与响度块矛盾）的计划
    /// 会让 [`ContainerPlan::validate`] 返回 `Ok(())`, 于是本函数在这里 `panic` ——
    /// 调用方**过了** `validate` 却仍然被 panic 打中。现在 `validate` 查了那三条,
    /// 因此 `validate() == Ok(())` 蕴含本函数不会 panic。
    /// 判据 `every_bext_shape_the_writer_accepts_the_reader_reads_back` 对
    /// `version × loudness × 容器` 的 18 格逐格断言这一点（而不是只断言"不 panic"：
    /// 它同时对被接受的格断言往返、对被拒的格断言错误值与 0 字节）。
    #[must_use]
    pub fn header_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.kind.fourcc());
        if self.kind.uses_ds64() {
            out.extend_from_slice(&le::u32(SENTINEL_U32));
        } else {
            out.extend_from_slice(&le::u32(self.sizes.riff_size as u32));
        }
        out.extend_from_slice(b"WAVE");
        if self.kind.uses_ds64() {
            push_chunk(&mut out, b"ds64", &self.sizes.to_bytes());
        }
        push_chunk(&mut out, b"fmt ", &self.format.fmt_payload());
        // `fact` 紧跟 `fmt `、在 `bext` 之前（FFmpeg 的 `wav->fact_pos` 就是写在这里）。
        // 只有非 PCM 写它; 载荷是帧数, 不是"总采样数"。
        if self.format.is_float {
            push_chunk(
                &mut out,
                b"fact",
                &le::u32(fact_frame_count(self.sizes.sample_count)),
            );
        }
        if let Some(bext) = &self.bext {
            push_chunk(&mut out, b"bext", &bext.to_bytes());
        }
        // data chunk: RF64 下长度字段哨兵化, 真实长度在 ds64 里。
        out.extend_from_slice(b"data");
        if self.kind.uses_ds64() {
            out.extend_from_slice(&le::u32(SENTINEL_U32));
        } else {
            out.extend_from_slice(&le::u32(self.sizes.data_size as u32));
        }
        out
    }
}

/// 一个 chunk 在文件里占用的总字节数（含 8 字节头与偶数补位）。
#[must_use]
const fn chunk_total(payload_len: usize) -> usize {
    8 + payload_len + (payload_len % 2)
}

/// 写出一个 chunk: fourcc + 小端 32 位长度 + 负载 + 必要的偶数补位。
fn push_chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&le::u32(payload.len() as u32));
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

/// `>4 GiB` 判定的独立入口（供调用方在拿到负载前预判）。
#[must_use]
pub const fn requires_rf64(projected_file_size: u64) -> bool {
    projected_file_size > SENTINEL_U32 as u64
}

/// 一次解析出的 chunk 位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkInfo {
    /// fourcc。
    pub fourcc: [u8; 4],
    /// 负载起始偏移。
    pub payload_offset: usize,
    /// 负载长度（不含补位）。
    pub payload_len: usize,
}

/// 解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedContainer {
    /// 容器种类。
    pub kind: ContainerKind,
    /// PCM 格式。
    pub format: PcmFormat,
    /// `ds64` 的长度三元组（RIFF 下由实际长度推导）。
    pub sizes: Rf64Sizes,
    /// `bext` 块（若存在）。
    pub bext: Option<Bext>,
    /// `data` 负载在文件里的字节范围。
    pub data: Range<usize>,
    /// 按文件顺序出现的全部 chunk（判"chunk 顺序正确"用）。
    pub chunks: Vec<ChunkInfo>,
}

/// RF64/BW64 读写错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rf64Error {
    /// 底层 I/O 失败。
    Io(String),
    /// 顶层 fourcc 不是 `RIFF` / `RF64` / `BW64`, 或 `WAVE` 缺失。
    NotWaveContainer,
    /// 文件比声明的长度短。
    Truncated {
        /// 期望的东西。
        what: &'static str,
        /// 实际可用的字节数。
        got: usize,
    },
    /// `ds64` chunk 的长度字段不是 28。
    BadDs64Len(u32),
    /// `RF64`/`BW64` 文件缺少 `ds64`。
    MissingDs64,
    /// 缺少 `fmt `。
    MissingFmt,
    /// 缺少 `data`。
    MissingData,
    /// `fmt ` chunk 长度不受支持。
    BadFmtLen(u32),
    /// 格式标签不受支持。
    UnsupportedFormatTag(u16),
    /// 声道数为 0。
    ZeroChannels,
    /// 采样率为 0。
    ///
    /// 与 [`Self::ZeroChannels`] 是同一条纪律, 只是这次**本 crate 的读取器读得回来**:
    /// 产出的文件声称"每秒 0 个采样"、`nAvgBytesPerSec` 也是 0, 是**外部**解码器打不开
    /// 的容器。同一份字节交给 `hound`（本 crate 在 [`crate::wav`] 里当裁判的第三方实现）
    /// 时, `hound` 写 `nBlockAlign` 的那一处 **现位于第 332 行**对 `spec.sample_rate`
    /// 做除法 ⇒ 除数为 0 的 panic。[`crate::wav::check_container_fields`] 因此早已把
    /// `sample_rate == 0` 列进"必须在创建文件之前说'不'"。
    ZeroSampleRate,
    /// 位深为 0。
    ///
    /// 与 [`Self::ZeroChannels`] / [`Self::ZeroSampleRate`] 是同一条纪律的第三个成员:
    /// `fmt ` 的这三个字段里任何一个取 0, 容器就**声明了一个零分母**。
    /// [`PcmFormat::bytes_per_sample`] 对 0 位给 0, 于是 `nBlockAlign` 与
    /// `nAvgBytesPerSec` 都是 0 —— 合规解码器按 `nBlockAlign` 求帧数就是除零。
    /// 本 crate 的**另一条**写入器（[`crate::wav::check_container_fields`]）早已把
    /// 0 位深判为 `UnsupportedDepth(0)`（那里连 `hound` 都进不去）; 而本模块的旧读取器
    /// 读得回它, 并把 `data` 的**字节数**当成帧数（`data_size` 除以被 `max(1)` 兜住的
    /// `nBlockAlign`）—— 数目是编出来的, 不是文件里写的。
    /// 判定在写入器侧（[`ContainerPlan::validate`]）与读取器侧
    /// （`parse_fmt_payload`）**各一处**, 两边共用本变体。
    ZeroBitsPerSample,
    /// [`write_container`] 拿到的负载长度与计划里声明的 `data` 长度不一致。
    ///
    /// 声明值落在 `data` chunk 的 32 位长度字段（`RIFF`）或 `ds64.dataSize`
    /// （`RF64`/`BW64`）里, 而真实字节数是 `payload.len()`。两者不一致时写入器的既有
    /// 保证不成立: 声明得比实际长 ⇒ 本 crate 的读取器返回
    /// [`Self::Truncated`]; 声明得比实际短 ⇒ 读取器只交出前 `declared` 字节,
    /// **其余字节被静默丢掉**。判定见 [`write_container`]。
    DataSizeMismatch {
        /// 计划声明的 `data` 负载长度（字节）。
        declared: u64,
        /// 实际交进来的负载长度（字节）。
        actual: u64,
    },
    /// `fmt ` 的 `nBlockAlign`（声道数 × 每样本容器字节数）放不进 `u16`。
    ///
    /// 这是一个**无法在 WAV 里表示**的声道布局（两个因子都直接来自文件字节）,
    /// 因此拒绝, 而不是让 [`PcmFormat::block_align`] 的乘法溢出。
    UnrepresentableBlockAlign {
        /// 声明的声道数。
        channels: u16,
        /// 每个样本占用的容器字节数。
        bytes_per_sample: u16,
    },
    /// `bext` 版本不受支持（本实现只支持 1 与 2）。
    UnsupportedBextVersion(u16),
    /// `bext` 的 `Version` 字段与它的 EBU R128 响度块**互相矛盾**：
    /// 版本 2 少了响度块, 或版本 1 却带着响度块。
    ///
    /// # 为什么这是一个必须由**计划**挡住的形态（实测）
    ///
    /// 读取器对版本**恒**给出一个确定的响度取值: [`Bext::from_bytes`] 对版本 1 给
    /// `None`、对版本 2 给 `Some(..)`（现位于第 912 行起的那个 `match`）。因此上面两种
    /// 形态**读不回同一个值**, 而 [`Bext::to_bytes`] 对它们直接 `panic`
    /// （"bext 版本 2 必须提供 EBU R128 响度字段" / "bext 版本 1 没有响度字段"）。
    ///
    /// 修复前 [`ContainerPlan::validate`] **不查**这两条, 于是同一份探针的字面读数
    /// 分成两半（本机实测; 判据
    /// `every_bext_shape_the_writer_accepts_the_reader_reads_back` 现在把两半都钉住）:
    ///
    /// ```text
    /// release 构建: validate() = Ok(()) -> write_container = PANIC（写出 0 字节）
    /// debug   构建: ContainerPlan::for_payload 自己 PANIC（它末尾的 debug_assert 会调 header_bytes）
    /// ```
    ///
    /// 也就是说一个返回 `Result` 的写入器在**两种构建下都 panic**, 而
    /// [`ContainerPlan::header_bytes`] 的文档写的是"走 [`write_container`] 的调用方
    /// 已经过了 `validate`"。这个变体就是那条文档的落地。
    BextLoudnessVersionMismatch {
        /// 块声明的 `Version` 字段。
        version: u16,
        /// 块是否带 EBU R128 响度块（[`Bext::loudness`]）。
        has_loudness: bool,
    },
    /// `bext` 的某个文本字段取值在字段编码里**无法表示**（含 NUL, 读取器会截断它）。
    ///
    /// 与 [`Self::UnrepresentableBlockAlign`] 同一条纪律的另一半: 写入器只写
    /// 读取器读得回**同一个值**的块。判定见 [`Bext::field_that_does_not_round_trip`]。
    UnrepresentableBextField {
        /// 出问题的字段名（BWF 的字段名, 如 `Description`）。
        field: &'static str,
    },
    /// 起始时间码不是 `HH:MM:SS`（或时/分/秒越界）。
    ///
    /// 这是**构造期的拒绝**, 不是"解析时宽容一下": `TimeReference` 写错会产出一个
    /// 声称首个采样点在某个不存在时刻的文件（见 [`time_reference_samples`]）。
    BadStartTimecode(String),
}

impl core::fmt::Display for Rf64Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "I/O 失败: {message}"),
            Self::NotWaveContainer => f.write_str("不是 RIFF/RF64/BW64 的 WAVE 容器"),
            Self::Truncated { what, got } => write!(f, "{what} 被截断 (只有 {got} 字节)"),
            Self::BadDs64Len(len) => write!(f, "ds64 chunk 长度应为 28, 实际 {len}"),
            Self::MissingDs64 => f.write_str("RF64/BW64 文件缺少 ds64 chunk"),
            Self::MissingFmt => f.write_str("缺少 fmt chunk"),
            Self::MissingData => f.write_str("缺少 data chunk"),
            Self::BadFmtLen(len) => write!(f, "fmt chunk 长度不受支持: {len}"),
            Self::UnsupportedFormatTag(tag) => write!(f, "不受支持的格式标签: {tag:#06X}"),
            Self::ZeroChannels => f.write_str("声道数为 0"),
            Self::ZeroSampleRate => f.write_str("采样率为 0"),
            Self::ZeroBitsPerSample => f.write_str("位深为 0"),
            Self::DataSizeMismatch { declared, actual } => write!(
                f,
                "声明的 data 负载长度是 {declared} 字节, 实际交进来的是 {actual} 字节"
            ),
            Self::UnrepresentableBlockAlign {
                channels,
                bytes_per_sample,
            } => write!(
                f,
                "声道布局无法表示: {channels} 声道 × {bytes_per_sample} 字节/样本 超过 nBlockAlign 的 u16 上限"
            ),
            Self::UnsupportedBextVersion(version) => {
                write!(f, "不受支持的 bext 版本: {version}")
            }
            Self::BextLoudnessVersionMismatch {
                version,
                has_loudness,
            } => {
                let expected = if *version >= 2 {
                    "必须带"
                } else {
                    "不得带"
                };
                let actual = if *has_loudness {
                    "却带着"
                } else {
                    "却没有"
                };
                write!(
                    f,
                    "bext 版本 {version} {expected} EBU R128 响度字段, 实际{actual}: \
                     读取器对这个版本恒给出同一个取值, 写出去就是一个往返不等的容器"
                )
            }
            Self::UnrepresentableBextField { field } => write!(
                f,
                "bext 的 {field} 字段取值含 NUL: 读取器在 NUL 处停止（或裁掉尾随 NUL）, \
                 写出去的值与读回来的值不是同一个字符串"
            ),
            Self::BadStartTimecode(value) => write!(
                f,
                "起始时间码 {value:?} 不是 HH:MM:SS（时分秒必须各自在合法范围内）"
            ),
        }
    }
}

impl std::error::Error for Rf64Error {}

impl From<io::Error> for Rf64Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// 把计划与负载写进 `out`。
///
/// 头部完全由 [`ContainerPlan::header_bytes`] 决定, 因此"假想大尺寸"的头部判据
/// 与真实写入走的是同一条代码路径 —— 不存在"测试测的是另一段代码"。
///
/// # 校验先于任何字节
///
/// 本函数先调 [`ContainerPlan::validate`], 因此一个"读取器必然拒绝 / 必然读回不同值"
/// 的计划会**在写出第一个字节之前**返回 `Err` —— `out` 里不会留下半个容器。
/// 这条顺序是有意的: `out` 可以是真实文件, 先写头再报错会留下一个残缺的交付物
/// （与 [`crate::wav::write_plain_wav`] 的"拒绝时不留字节"同一条纪律）。
///
/// # 负载长度必须等于计划声明的 `data` 长度（本轮补上）
///
/// `data` 的长度**有两条独立的来源**, 而它们只在调用方守规矩时才一致:
///
/// - **声明值** —— 计划里 [`Rf64Sizes::data_size`]。它由
///   [`ContainerPlan::for_payload`] 的 `payload_len` 参数算出, 落在 `RIFF` 的
///   `data` 32 位长度字段或 `RF64`/`BW64` 的 `ds64.dataSize` 里;
/// - **真实值** —— `payload.len()`, 即本函数真的写出去的字节数。
///
/// 下面这些类型都是 `pub` 字段: [`ContainerPlan`]、[`Rf64Sizes`], 而
/// [`ContainerPlan::for_payload`] 也不校验这两个参数彼此是否一致。于是一个把
/// `payload_len` 与真正交进来的切片写岔的调用方（或直接拼 `ContainerPlan` 字面量的
/// 调用方）能让本函数在**修复前**返回 `Ok(())`, 而交付物是:
///
/// ```text
/// 声明 34 字节, 实际交 32 字节 -> write_container Ok(写出 76 字节), parse_container Err(Truncated { what: "data 负载" })
/// 声明 30 字节, 实际交 32 字节 -> write_container Ok(写出 76 字节), parse_container Ok 但只交出 30 字节, 尾巴 2 字节被静默丢掉
/// 声明  0 字节, 实际交 32 字节 -> 同上, 读取器报告 0 帧
/// ```
///
/// （本机实测, 三种容器 `RIFF`/`RF64`/`BW64` 逐格同形; 判据
/// `the_payload_length_must_match_the_declared_data_length`。）
///
/// 两种形态都让本函数的既有保证 "`Ok(())` ⇒ 写出的字节能被 [`parse_container`]
/// 读回" 不成立, 因此它们在写任何字节之前都返回
/// [`Rf64Error::DataSizeMismatch`]。这与 [`ContainerPlan::validate`] 是同一个位置、
/// 同一条纪律: **写入器只写自己的读取器读得回全部字节的容器**。
///
/// [`ContainerPlan::header_bytes`] 自己**做不了**这项检查 —— 它的签名里没有负载,
/// 这正是这条校验落在**写入点**而不是头部构造点的原因。
///
/// # Errors
///
/// 计划不可写（见 [`ContainerPlan::validate`]）、负载长度与声明的 `data` 长度不符
/// （[`Rf64Error::DataSizeMismatch`]）, 或底层写入失败。
pub fn write_container<W: Write>(
    out: &mut W,
    plan: &ContainerPlan,
    payload: &[u8],
) -> Result<(), Rf64Error> {
    plan.validate()?;
    let actual = payload.len() as u64;
    if actual != plan.sizes.data_size {
        return Err(Rf64Error::DataSizeMismatch {
            declared: plan.sizes.data_size,
            actual,
        });
    }
    out.write_all(&plan.header_bytes())?;
    out.write_all(payload)?;
    if payload.len() % 2 == 1 {
        out.write_all(&[0])?;
    }
    Ok(())
}

/// 解析一个完整容器的字节。
///
/// 判据要点: `RF64`/`BW64` 下 `data` 的真实长度**只能**来自 `ds64`, 因为 32 位
/// 长度字段是哨兵; 若 `ds64` 声明的长度超出实际文件, 返回 [`Rf64Error::Truncated`]
/// 而不是"假装读到了"。
///
/// **本函数对任意字节输入都是全函数**: chunk 循环里所有"起点 + 声明长度"的算术
/// 都是 `checked_add`（声明长度既可能来自 chunk 头的 32 位字段, 也可能来自 `ds64`
/// 的 64 位字段），因此畸形长度只会得到一个 `Err`，不会在 debug 下 panic、也不会在
/// release 下回绕。判据是 `every_single_byte_mutation_returns_a_result_instead_of_panicking`
/// 与 `an_oversized_declared_length_is_rejected_instead_of_overflowing`。
///
/// # Errors
///
/// 容器结构非法、缺 chunk、或声明的长度与实际字节不符。
pub fn parse_container(bytes: &[u8]) -> Result<ParsedContainer, Rf64Error> {
    let fourcc: [u8; 4] = bytes
        .get(0..4)
        .ok_or(Rf64Error::Truncated {
            what: "顶层 fourcc",
            got: bytes.len(),
        })?
        .try_into()
        .expect("4 字节");
    let kind = ContainerKind::from_fourcc(fourcc).ok_or(Rf64Error::NotWaveContainer)?;
    if bytes.get(8..12) != Some(&b"WAVE"[..]) {
        return Err(Rf64Error::NotWaveContainer);
    }

    // 统一的"声明长度超出文件"出口: 只差一个说明字段, 实际字节数恒为文件长度。
    let truncated = |what: &'static str| Rf64Error::Truncated {
        what,
        got: bytes.len(),
    };
    let mut chunks = Vec::new();
    let mut ds64: Option<Rf64Sizes> = None;
    let mut format: Option<PcmFormat> = None;
    let mut bext: Option<Bext> = None;
    let mut data: Option<(usize, usize)> = None;

    let mut cursor = 12usize;
    // 循环条件本身也要 checked: `cursor` 由下面的声明长度算出, 在对齐前不得回绕。
    while cursor
        .checked_add(8)
        .is_some_and(|chunk_header_end| chunk_header_end <= bytes.len())
    {
        let id: [u8; 4] = bytes[cursor..cursor + 4].try_into().expect("4 字节");
        let declared = le::read_u32(bytes, cursor + 4).ok_or(Rf64Error::Truncated {
            what: "chunk 长度字段",
            got: bytes.len(),
        })?;
        let payload_offset = cursor + 8;
        // 哨兵只允许出现在 data chunk 上 (RF64/BW64); 其余 chunk 用它就是损坏。
        let effective = if declared == SENTINEL_U32 && &id == b"data" {
            ds64.ok_or(Rf64Error::MissingDs64)?
                .data_size
                .try_into()
                .map_err(|_| Rf64Error::Truncated {
                    what: "data 负载长度超出本机 usize",
                    got: bytes.len(),
                })?
        } else {
            declared as usize
        };

        // `payload_end = 起始偏移 + 声明长度`。这一步**必须**是 checked 加法:
        // `data` 的长度来自文件里的 64 位 `ds64.data_size` 字段, 一个损坏或恶意的值
        // 可以让这个和溢出 `usize`。溢出意味着"声明的长度比任何可能的文件都长",
        // 与"文件比声明的短"是同一件事, 因此走同一个 `Truncated` 出口 ——
        // 既不是 debug 下的 `attempt to add with overflow` panic, 也不是 release 下
        // 的回绕 (回绕会让 `cursor` 倒退, 同一个 chunk 被反复解析)。
        let payload_end = payload_offset
            .checked_add(effective)
            .ok_or(Rf64Error::Truncated {
                what: "chunk 负载长度 (起始偏移 + 声明长度超出 usize)",
                got: bytes.len(),
            })?;

        chunks.push(ChunkInfo {
            fourcc: id,
            payload_offset,
            payload_len: effective,
        });

        match &id {
            b"ds64" => {
                if declared != Rf64Sizes::PAYLOAD_LEN as u32 {
                    return Err(Rf64Error::BadDs64Len(declared));
                }
                let payload = bytes
                    .get(payload_offset..payload_end)
                    .ok_or_else(|| truncated("ds64 负载"))?;
                ds64 = Rf64Sizes::from_bytes(payload);
            }
            b"fmt " => {
                let payload = bytes
                    .get(payload_offset..payload_end)
                    .ok_or_else(|| truncated("fmt 负载"))?;
                format = Some(parse_fmt_payload(payload, declared)?);
            }
            b"bext" => {
                let payload = bytes
                    .get(payload_offset..payload_end)
                    .ok_or_else(|| truncated("bext 负载"))?;
                bext = Some(Bext::from_bytes(payload)?);
            }
            b"data" => {
                if payload_end > bytes.len() {
                    return Err(Rf64Error::Truncated {
                        what: "data 负载",
                        got: bytes.len(),
                    });
                }
                data = Some((payload_offset, payload_end));
            }
            _ => {}
        }

        // 前进: chunk 头 8 字节 + 负载 + 偶数补位。
        cursor = payload_end
            .checked_add(effective % 2)
            .ok_or(Rf64Error::Truncated {
                what: "chunk 负载长度 (补位字节超出 usize)",
                got: bytes.len(),
            })?;
    }

    let format = format.ok_or(Rf64Error::MissingFmt)?;
    let (data_start, data_end) = data.ok_or(Rf64Error::MissingData)?;
    let sizes = ds64.unwrap_or(Rf64Sizes {
        riff_size: bytes.len().saturating_sub(8) as u64,
        data_size: (data_end - data_start) as u64,
        sample_count: ((data_end - data_start) as u64) / u64::from(format.block_align().max(1)),
    });
    if kind.uses_ds64() && ds64.is_none() {
        return Err(Rf64Error::MissingDs64);
    }

    Ok(ParsedContainer {
        kind,
        format,
        sizes,
        bext,
        data: data_start..data_end,
        chunks,
    })
}

/// 解析 `fmt ` 负载（16 或 40 字节）。
fn parse_fmt_payload(payload: &[u8], declared_len: u32) -> Result<PcmFormat, Rf64Error> {
    if payload.len() < 16 {
        return Err(Rf64Error::BadFmtLen(declared_len));
    }
    let tag = le::read_u16(payload, 0).ok_or(Rf64Error::BadFmtLen(declared_len))?;
    let channels = le::read_u16(payload, 2).ok_or(Rf64Error::BadFmtLen(declared_len))?;
    let sample_rate = le::read_u32(payload, 4).ok_or(Rf64Error::BadFmtLen(declared_len))?;
    let bits_per_sample = le::read_u16(payload, 14).ok_or(Rf64Error::BadFmtLen(declared_len))?;
    if channels == 0 {
        return Err(Rf64Error::ZeroChannels);
    }
    // 0 位深与 0 声道同族: `bytes_per_sample()` 会是 0, 于是 `nBlockAlign` 也是 0,
    // 而下面算 `sample_count` 的那次除法只能靠 `max(1)` 兜住 —— 结果是**把负载的字节数
    // 当成帧数**。这是编出来的读数, 不是文件里写的, 因此整条拒绝。
    // 写入器侧 [`ContainerPlan::validate`] 用同一个字段做同一个判决。
    if bits_per_sample == 0 {
        return Err(Rf64Error::ZeroBitsPerSample);
    }
    let (is_float, channel_mask) = match tag {
        0x0001 => (false, None),
        0x0003 => (true, None),
        0xFFFE => {
            if payload.len() < 40 {
                return Err(Rf64Error::BadFmtLen(declared_len));
            }
            let mask = le::read_u32(payload, 20).ok_or(Rf64Error::BadFmtLen(declared_len))?;
            let guid: [u8; 16] = payload[24..40].try_into().expect("16 字节");
            let is_float = if guid == subformat_guid(true) {
                true
            } else if guid == subformat_guid(false) {
                false
            } else {
                return Err(Rf64Error::UnsupportedFormatTag(tag));
            };
            (is_float, Some(mask))
        }
        other => return Err(Rf64Error::UnsupportedFormatTag(other)),
    };
    let valid_bits = if tag == 0xFFFE {
        le::read_u16(payload, 18)
            .filter(|&bits| bits > 0)
            .unwrap_or(bits_per_sample)
    } else {
        bits_per_sample
    };
    let format = PcmFormat {
        channels,
        sample_rate,
        bits_per_sample: valid_bits,
        is_float,
        channel_mask,
    };
    // `nBlockAlign` 是 `fmt ` 里的 `u16` 字段, 而它必须容纳"声道数 × 每样本字节数"。
    // 两个因子都直接来自文件字节, 因此这一步必须显式检查: 放不下的布局在 WAV 里
    // **无法表示**。本机实测（判据 `unrepresentable_block_align_is_rejected`）:
    // 不查这一条时, `channels = 0xFFFF` + 32-bit 会让 `PcmFormat::block_align`
    // 的乘法在 debug 下 panic。
    //
    // 判定走 `PcmFormat::block_align_fits_u16` —— 写入器（`ContainerPlan::validate`）
    // 用的是**同一个**谓词, 因此"读取器拒绝的布局"与"写入器拒绝的计划"不会漂移。
    if !format.block_align_fits_u16() {
        return Err(Rf64Error::UnrepresentableBlockAlign {
            channels: format.channels,
            bytes_per_sample: format.bytes_per_sample(),
        });
    }
    Ok(format)
}

/// 从容器字节里抽出每个 chunk 的 fourcc（顺序敏感）。
///
/// 这是"chunk 顺序正确"判据的读侧入口: 调用方可以直接断言
/// `["ds64", "fmt ", "bext", "data"]`。
///
/// # Errors
///
/// 与 [`parse_container`] 相同。
pub fn chunk_order(bytes: &[u8]) -> Result<Vec<[u8; 4]>, Rf64Error> {
    Ok(parse_container(bytes)?
        .chunks
        .into_iter()
        .map(|c| c.fourcc)
        .collect())
}

/// 把 `(fourcc, payload_len)` 汇总成便于断言的映射。
#[must_use]
pub fn chunk_lengths(chunks: &[ChunkInfo]) -> BTreeMap<String, usize> {
    chunks
        .iter()
        .map(|chunk| {
            (
                String::from_utf8_lossy(&chunk.fourcc).into_owned(),
                chunk.payload_len,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_16bit() -> PcmFormat {
        PcmFormat::integer(2, 48_000, 16)
    }

    /// 交错立体声 16-bit 负载, 每个样本都是可预测的值。
    fn payload(frames: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(frames * 4);
        for frame in 0..frames {
            let left = (frame as i16).wrapping_mul(3);
            let right = (frame as i16).wrapping_mul(-5);
            out.extend_from_slice(&left.to_le_bytes());
            out.extend_from_slice(&right.to_le_bytes());
        }
        out
    }

    /// 判据 1: RF64 头部逐字段正确 —— fourcc、两处 `0xFFFFFFFF` 哨兵、
    /// `ds64` 的 chunk 长度 = 28、三个 64 位长度、`tableLength` = 0、chunk 顺序。
    #[test]
    fn rf64_header_fields_and_chunk_order_are_correct() {
        let frames = 1000u64;
        let data = payload(frames as usize);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Rf64,
            stereo_16bit(),
            data.len() as u64,
            frames,
            None,
        );
        let header = plan.header_bytes();

        assert_eq!(&header[0..4], b"RF64");
        assert_eq!(
            le::read_u32(&header, 4),
            Some(SENTINEL_U32),
            "RIFF 长度哨兵"
        );
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(&header[12..16], b"ds64", "ds64 必须紧随 WAVE");
        assert_eq!(le::read_u32(&header, 16), Some(28), "ds64 chunk 长度");
        let sizes = Rf64Sizes::from_bytes(&header[20..48]).expect("28 字节");
        assert_eq!(sizes.data_size, data.len() as u64);
        assert_eq!(sizes.sample_count, frames, "sampleCount 是帧数");
        assert_eq!(le::read_u32(&header, 44), Some(0), "tableLength = 0");
        assert_eq!(
            sizes.riff_size,
            header.len() as u64 + data.len() as u64 - 8,
            "riffSize = 文件长度 - 8"
        );

        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        assert_eq!(
            chunk_order(&file).expect("解析"),
            vec![*b"ds64", *b"fmt ", *b"data"],
            "chunk 顺序"
        );
        // data chunk 的 32 位长度字段必须是哨兵
        let data_at = file.len() - data.len() - 8;
        assert_eq!(&file[data_at..data_at + 4], b"data");
        assert_eq!(le::read_u32(&file, data_at + 4), Some(SENTINEL_U32));
        assert_eq!(&file[data_at + 8..], data.as_slice());
    }

    /// 判据 2: `data` 的 32 位长度字段在 RF64/BW64 下是哨兵, 在 RIFF 下是真实长度。
    #[test]
    fn data_chunk_size_field_is_sentinel_only_for_ds64_containers() {
        let data = payload(8);
        for (kind, expected) in [
            (ContainerKind::Riff, data.len() as u32),
            (ContainerKind::Rf64, SENTINEL_U32),
            (ContainerKind::Bw64, SENTINEL_U32),
        ] {
            let plan = ContainerPlan::for_payload(kind, stereo_16bit(), data.len() as u64, 8, None);
            let header = plan.header_bytes();
            let data_at = header.len() - 8;
            assert_eq!(&header[data_at..data_at + 4], b"data");
            assert_eq!(
                le::read_u32(&header, data_at + 4),
                Some(expected),
                "{kind:?} 的 data 长度字段错误"
            );
        }
    }

    /// 判据 3 **>4 GiB 走 RF64**: 用**假想**的大负载长度构造计划, 一个字节的
    /// 4 GiB 也不用写。
    #[test]
    fn payload_beyond_four_gib_switches_to_rf64() {
        let huge: u64 = 5_000_000_000;
        let frames = huge / 4;
        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, stereo_16bit(), huge, frames, None);
        assert_eq!(plan.kind, ContainerKind::Rf64, "必须升级到 RF64");
        assert_eq!(plan.sizes.data_size, huge);

        let header = plan.header_bytes();
        assert_eq!(&header[0..4], b"RF64");
        assert_eq!(le::read_u32(&header, 4), Some(SENTINEL_U32));
        let sizes = Rf64Sizes::from_bytes(&header[20..48]).expect("28 字节");
        assert_eq!(sizes.data_size, huge);
        assert_eq!(sizes.sample_count, frames);
        assert!(sizes.riff_size > u64::from(SENTINEL_U32));
        assert!(requires_rf64(sizes.riff_size + 8));
    }

    /// 判据 4: 4 GiB 边界是**精确**的 —— 刚好放得下仍用 RIFF, 多 2 字节就升 RF64;
    /// 而且边界判定把 data 负载的偶数补位算在内。
    #[test]
    fn four_gib_boundary_is_exact() {
        // 立体声 16-bit: 头部无 ds64 时 = 12 + (8+16) + 0 + 8 = 44。
        const HEADER_WITHOUT_DS64: u64 = 44;
        let limit = u64::from(SENTINEL_U32) - HEADER_WITHOUT_DS64;
        let at_limit = limit - (limit % 2); // 偶数负载没有补位
        assert_eq!(at_limit % 2, 0);

        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, stereo_16bit(), at_limit, 1, None);
        assert_eq!(plan.kind, ContainerKind::Riff, "恰好放得下时不该升级");

        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, stereo_16bit(), at_limit + 2, 1, None);
        assert_eq!(plan.kind, ContainerKind::Rf64, "多 2 字节必须升级");

        // `limit` 本身是奇数: 负载没有超, 但补位字节把它顶了出去 —— 这正是"补位字节
        // 计入 RIFF 长度字段"的直接后果。
        assert_eq!(limit % 2, 1);
        let plan = ContainerPlan::for_payload(ContainerKind::Riff, stereo_16bit(), limit, 1, None);
        assert_eq!(
            plan.kind,
            ContainerKind::Rf64,
            "奇数负载的补位字节也算进 RIFF 长度, 因此同样要升级"
        );

        assert!(!requires_rf64(u64::from(SENTINEL_U32)));
        assert!(requires_rf64(u64::from(SENTINEL_U32) + 1));
    }

    /// 判据 4b: `for_payload` 对**任意** `u64` 负载长度都是全函数 —— 一个不可能的
    /// `u64::MAX` 只让长度**饱和**, 不让算术在 debug 下溢出（release 下则回绕）。
    ///
    /// 注入证明（本机实测）：把 `payload_len.saturating_add(payload_len % 2)` 换回
    /// `payload_len + (payload_len % 2)`，本判据以 `attempt to add with overflow` 结束。
    /// 与读取器那两条判据（12b / 12d）是同一族纪律：**声明的长度算术不得 panic**。
    #[test]
    fn an_impossible_payload_length_saturates_instead_of_panicking() {
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            stereo_16bit(),
            u64::MAX,
            u64::MAX,
            None,
        );
        assert_eq!(
            plan.kind,
            ContainerKind::Rf64,
            "饱和后的长度必然超过 4 GiB, 因此必须升级"
        );
        assert_eq!(plan.sizes.data_size, u64::MAX, "负载长度本身原样保留");
        assert_eq!(
            plan.sizes.riff_size,
            u64::MAX - 8,
            "riffSize = 头部 + 饱和后的负载总量 − 8"
        );
        assert!(!plan.header_bytes().is_empty(), "头部仍然必须写得出来");
    }

    /// 判据 5: 三种容器写出的文件都能被**自己的**读取器读回, 且长度三元组一致。
    #[test]
    fn every_container_kind_round_trips_through_our_reader() {
        let data = payload(300);
        let frames = 300u64;
        for kind in [
            ContainerKind::Riff,
            ContainerKind::Rf64,
            ContainerKind::Bw64,
        ] {
            let plan =
                ContainerPlan::for_payload(kind, stereo_16bit(), data.len() as u64, frames, None);
            let mut file = Vec::new();
            write_container(&mut file, &plan, &data).expect("写入");
            let parsed = parse_container(&file).expect("解析");
            assert_eq!(parsed.kind, kind);
            assert_eq!(parsed.format, stereo_16bit());
            assert_eq!(parsed.sizes.data_size, data.len() as u64);
            assert_eq!(parsed.sizes.sample_count, frames);
            assert_eq!(&file[parsed.data.clone()], data.as_slice());
            assert_eq!(parsed.sizes.riff_size, file.len() as u64 - 8);
        }
    }

    /// 判据 6: BEXT 版本 2 往返, 固定前缀 612 字节, 响度字段偏移与 0.01 单位缩放正确。
    #[test]
    fn bext_version_two_round_trips() {
        let block = Bext {
            description: "Yeban reference project A master".to_owned(),
            originator: "Yeban DAW".to_owned(),
            originator_reference: "01J8ZK9WQ7F5N2V4B6C8D0E1F2".to_owned(),
            origination_date: "2026-10-05".to_owned(),
            origination_time: "13:37:00".to_owned(),
            time_reference: 48_000 * 3600,
            version: 2,
            umid: [0xAB; 64],
            loudness: Some(Loudness {
                loudness_value: Loudness::from_lufs(-14.0),
                loudness_range: Loudness::from_lufs(7.5),
                max_true_peak_level: Loudness::from_dbtp(-1.0),
                max_momentary_loudness: Loudness::from_lufs(-12.25),
                max_short_term_loudness: Loudness::UNKNOWN,
            }),
            coding_history: "A=PCM,F=48000,W=24,M=stereo".to_owned(),
        };
        assert_eq!(
            block.fixed_len(),
            BEXT_FIXED_LEN,
            "v1 与 v2 的固定前缀都是 602"
        );
        let bytes = block.to_bytes();
        assert_eq!(bytes.len(), BEXT_FIXED_LEN + block.coding_history.len());
        assert_eq!(le::read_u16(&bytes, 346), Some(2), "Version 字段偏移 346");
        assert_eq!(&bytes[348..412], &[0xAB; 64], "UMID 偏移 348..412");
        assert_eq!(
            le::read_u64(&bytes, 338),
            Some(48_000 * 3600),
            "TimeReference"
        );
        // 响度字段紧接 UMID: 偏移 412..422; 之后是缩短为 180 字节的保留区。
        let loud = Loudness::from_bytes(&bytes[412..422]).expect("10 字节");
        assert_eq!(loud.loudness_value, -1400);
        assert_eq!(loud.loudness_range, 750);
        assert_eq!(loud.max_true_peak_level, -100);
        assert_eq!(loud.max_short_term_loudness, Loudness::UNKNOWN);
        assert_eq!(&bytes[422..602], &[0u8; 180], "v2 保留区 180 字节");

        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert_eq!(decoded, block);
    }

    /// 判据 7: BEXT 版本 1 固定前缀 602 字节, 保留区 190 字节, 无响度字段。
    #[test]
    fn bext_version_one_has_602_byte_prefix() {
        let block = Bext {
            version: 1,
            description: "v1".to_owned(),
            ..Bext::default()
        };
        let bytes = block.to_bytes();
        assert_eq!(bytes.len(), BEXT_FIXED_LEN);
        assert_eq!(le::read_u16(&bytes, 346), Some(1));
        assert_eq!(&bytes[412..602], &[0u8; 190]);
        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.loudness, None);
        assert_eq!(decoded.description, "v1");
    }

    /// 判据 8: 未核验的 `bext` 版本 0 必须被**拒绝**, 而不是写一段猜的字节。
    #[test]
    fn bext_version_zero_is_refused() {
        let valid = Bext {
            version: 1,
            ..Bext::default()
        };
        let mut bytes = valid.to_bytes();
        bytes[346..348].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            Bext::from_bytes(&bytes),
            Err(Rf64Error::UnsupportedBextVersion(0))
        );
        // 版本 3 同样拒绝。
        bytes[346..348].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            Bext::from_bytes(&bytes),
            Err(Rf64Error::UnsupportedBextVersion(3))
        );
    }

    /// 判据 9: `bext` 必须排在 `fmt ` 之后、`data` 之前, 且 ULID 落进
    /// `OriginatorReference`（[ARCH-FMT-001] 要求的"工程 ULID"映射）。
    #[test]
    fn bext_sits_between_fmt_and_data_and_carries_the_ulid() {
        let data = payload(16);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Rf64,
            stereo_16bit(),
            data.len() as u64,
            16,
            Some(Bext::for_project(
                "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
                "2026-10-05",
                "13:37:00",
            )),
        );
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        assert_eq!(
            chunk_order(&file).expect("解析"),
            vec![*b"ds64", *b"fmt ", *b"bext", *b"data"]
        );
        let parsed = parse_container(&file).expect("解析");
        let block = parsed.bext.expect("有 bext");
        assert_eq!(block.originator_reference, "01J8ZK9WQ7F5N2V4B6C8D0E1F2");
        assert_eq!(block.version, 2);
        assert_eq!(block.origination_date, "2026-10-05");
        assert_eq!(block.origination_time, "13:37:00");
    }

    /// 从 `header_bytes()` 里取出某个 chunk 的**负载**（走到 `data` 头就停）。
    ///
    /// 为什么不能用 [`parse_container`]: 大尺寸场景只有头部、没有负载（写不出
    /// 5 GB 的判据数据）, 而 `parse_container` 要求 `data` 负载真的存在。
    /// 头部是刚由 [`ContainerPlan::header_bytes`] 写出来的, 因此它的长度字段可信。
    /// `data` 的长度字段在 RF64 下是哨兵 —— 所以循环必须在读它**之前**停。
    fn header_chunk_payload(header: &[u8], fourcc: &[u8; 4]) -> Option<Vec<u8>> {
        let mut cursor = 12usize;
        while cursor + 8 <= header.len() {
            let id: [u8; 4] = header[cursor..cursor + 4].try_into().expect("4 字节");
            if &id == b"data" {
                return None;
            }
            let declared = le::read_u32(header, cursor + 4).expect("长度字段") as usize;
            let payload = header.get(cursor + 8..cursor + 8 + declared)?;
            if &id == fourcc {
                return Some(payload.to_vec());
            }
            cursor += 8 + declared + (declared % 2);
        }
        None
    }

    /// 判据 9b: **非 PCM（IEEE 浮点）容器必须带 `fact` chunk**, 位置与载荷都正确;
    /// 同一条判据还钉住**反方向** —— 整数 PCM 不得写它。
    ///
    /// 每个读数的来源与单位:
    /// - **顺序**（chunk 序列）: 模块头核验表里 FFmpeg 的 `wav->fact_pos` —— 紧跟
    ///   `fmt `、在 `bext` 之前;
    /// - **载荷长度**（字节）: [`FACT_PAYLOAD_LEN`] = 4（一个 `u32`）;
    /// - **载荷数值**（帧）: 必须等于传给 `for_payload` 的 `frame_count`, 不是"总采样数";
    /// - **`riffSize`**（字节）: 仍然等于文件长度减 8。
    ///
    /// 注入（先红后还原）: 把 [`ContainerPlan::header_bytes`] 里的
    /// `if self.format.is_float` 改成 `false` ⇒ 第一段红; 改成 `true`
    /// ⇒ 第二段（整数不得有 `fact`）红。
    #[test]
    fn non_pcm_containers_carry_the_mandatory_fact_chunk() {
        const FRAMES: u64 = 480;
        let block = Bext {
            version: 1,
            coding_history: "CH".to_owned(),
            ..Bext::default()
        };

        // 浮点: 三种容器都必须带 `fact`, 且就在 `fmt ` 之后。
        for kind in [
            ContainerKind::Riff,
            ContainerKind::Rf64,
            ContainerKind::Bw64,
        ] {
            let format = PcmFormat::float(2, 48_000, 32);
            let data = vec![0u8; FRAMES as usize * 2 * 4];
            let plan = ContainerPlan::for_payload(
                kind,
                format,
                data.len() as u64,
                FRAMES,
                Some(block.clone()),
            );
            let mut file = Vec::new();
            write_container(&mut file, &plan, &data).expect("写入");

            let expected: Vec<[u8; 4]> = if kind.uses_ds64() {
                vec![*b"ds64", *b"fmt ", *b"fact", *b"bext", *b"data"]
            } else {
                vec![*b"fmt ", *b"fact", *b"bext", *b"data"]
            };
            assert_eq!(
                chunk_order(&file).expect("解析"),
                expected,
                "{kind:?}: 浮点容器必须带 `fact` 且排在 `fmt ` 之后"
            );

            let parsed = parse_container(&file).expect("解析");
            let fact = parsed
                .chunks
                .iter()
                .find(|chunk| &chunk.fourcc == b"fact")
                .expect("浮点容器必须有 fact chunk");
            assert_eq!(
                fact.payload_len, FACT_PAYLOAD_LEN,
                "{kind:?}: fact 的声明长度(字节)"
            );
            assert_eq!(
                le::read_u32(&file, fact.payload_offset),
                Some(FRAMES as u32),
                "{kind:?}: fact 的 u32 必须等于帧数(帧)"
            );
            assert_eq!(
                parsed.sizes.riff_size,
                file.len() as u64 - 8,
                "{kind:?}: 加了 fact 之后 riffSize 仍必须是文件长度 - 8"
            );
        }

        // 整数 PCM: 一个 `fact` 都不许有（PCM 的 `fact` 是冗余的）。
        for format in [
            PcmFormat::integer(2, 48_000, 16),
            PcmFormat::integer(2, 48_000, 24),
        ] {
            let bytes_per_sample = usize::from(format.bits_per_sample) / 8;
            let data = vec![0u8; FRAMES as usize * 2 * bytes_per_sample];
            let plan = ContainerPlan::for_payload(
                ContainerKind::Riff,
                format,
                data.len() as u64,
                FRAMES,
                Some(block.clone()),
            );
            let header = plan.header_bytes();
            assert!(
                !header.windows(4).any(|window| window == b"fact".as_slice()),
                "整数 PCM 不得带 fact chunk: {format:?}"
            );
        }
    }

    /// 判据 9c: `fact` 的数值**只在放不进 `u32` 时**才退化为哨兵; 退化时真值在
    /// `ds64.sampleCount` 里（因此那个容器必然是带 `ds64` 的）。
    ///
    /// 大尺寸用 [`ContainerPlan::for_payload`] 的**假想**尺寸参数化 —— 与
    /// `payload_beyond_four_gib_switches_to_rf64` 同一手法, 不写任何大文件。
    /// 单位: `payload_len` 是**字节**, `frame_count` 是**帧**。
    ///
    /// 注入（先红后还原）:
    /// ① 把 `fact_frame_count` 的条件 `sample_count < SENTINEL_U32 as u64` 改成
    /// `false` ⇒ 第 1、2 段红（能表示的帧数也被写成哨兵）;
    /// ② 把 `else` 分支的 `SENTINEL_U32` 改成 `0` ⇒ 第 3 段红;
    /// ③ 把整个函数体换成恒真的 `sample_count as u32` ⇒ 第 3 段红, 靠的是
    /// `0x1_0000_0000` 那一格（它的低 32 位是 `0`）。**只取 `0xFFFF_FFFF` 的版本抓不到
    /// 这条注入** —— 那一格与哨兵逐位重合（见 `fact_frame_count` 的文档）; 本判据因此
    /// 取两个值, 而不是一个。
    #[test]
    fn fact_degrades_to_the_sentinel_only_when_the_frame_count_does_not_fit() {
        let format = PcmFormat::float(2, 48_000, 32);

        // 1) 放得下的 RIFF: 原样写帧数。头部长度(字节) = 12 + (8+16) + (8+4) + 8 = 56。
        let frames = 250_000u64;
        let payload_len = frames * 2 * 4;
        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, format, payload_len, frames, None);
        assert_eq!(plan.kind, ContainerKind::Riff, "2 MB 不该升级");
        assert_eq!(
            plan.header_bytes().len(),
            56,
            "RIFF 立体声 32f 且无 bext 的头部长度(字节)"
        );
        assert_eq!(
            header_chunk_payload(&plan.header_bytes(), b"fact").expect("有 fact"),
            (frames as u32).to_le_bytes().to_vec()
        );

        // 2) 负载 > 4 GiB 但帧数仍放得进 u32: 容器升级到 RF64, 帧数**照原样**写。
        let big_payload: u64 = 5_000_000_000;
        let big_frames = big_payload / 8;
        assert!(big_frames < u64::from(SENTINEL_U32));
        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, format, big_payload, big_frames, None);
        assert_eq!(plan.kind, ContainerKind::Rf64, "必须升级");
        assert_eq!(
            header_chunk_payload(&plan.header_bytes(), b"fact").expect("有 fact"),
            (big_frames as u32).to_le_bytes().to_vec(),
            "帧数放得进 u32 时不许写哨兵(帧)"
        );

        // 3) 帧数本身放不进 u32: `fact` 写哨兵, 真值在 `ds64.sampleCount`。
        //
        // 取**两个**值: `0xFFFF_FFFF` 这一格与哨兵逐位重合（见 `fact_frame_count` 的
        // 文档, 那是 `u32` 字段借满值当哨兵的固有代价）; `0x1_0000_0000` 这一格是用来
        // **区分**"真的走哨兵分支"与"无脑 `sample_count as u32` 截断"的 —— 后者的低
        // 32 位是 `0`, 因此只有真的走哨兵分支才会得到 `0xFFFF_FFFF`。
        for overflow in [u64::from(SENTINEL_U32), u64::from(SENTINEL_U32) + 1] {
            let plan = ContainerPlan::for_payload(
                ContainerKind::Riff,
                format,
                overflow * 8,
                overflow,
                None,
            );
            assert!(
                plan.kind.uses_ds64(),
                "这个尺寸必须带 ds64, 否则真值无处可写"
            );
            assert_eq!(
                header_chunk_payload(&plan.header_bytes(), b"fact").expect("有 fact"),
                SENTINEL_U32.to_le_bytes().to_vec(),
                "放不进 u32 的帧数在 fact 里必须是哨兵 (帧数 {overflow})"
            );
            assert_eq!(
                plan.sizes.sample_count, overflow,
                "ds64.sampleCount 必须写真值(帧)"
            );
        }
    }

    /// 判据 10: 单/双声道用 16 字节 `fmt `（tag 1/3）; 多声道自动切到 40 字节
    /// `WAVE_FORMAT_EXTENSIBLE`, 且 `cbSize`/`validBits`/声道掩码/GUID 正确。
    #[test]
    fn fmt_chunk_switches_to_extensible_for_multichannel() {
        let stereo_int = PcmFormat::integer(2, 48_000, 24);
        assert_eq!(stereo_int.fmt_payload().len(), 16);
        assert_eq!(stereo_int.format_tag(), 0x0001);
        assert_eq!(stereo_int.block_align(), 6);
        assert_eq!(stereo_int.byte_rate(), 288_000);

        let stereo_float = PcmFormat::float(2, 48_000, 32);
        assert_eq!(stereo_float.fmt_payload().len(), 16);
        assert_eq!(stereo_float.format_tag(), 0x0003);

        let surround = PcmFormat::float(6, 48_000, 32);
        let body = surround.fmt_payload();
        assert_eq!(body.len(), 40);
        assert_eq!(surround.format_tag(), 0xFFFE);
        assert_eq!(le::read_u16(&body, 16), Some(22), "cbSize");
        assert_eq!(le::read_u16(&body, 18), Some(32), "wValidBitsPerSample");
        assert_eq!(le::read_u32(&body, 20), Some(0x3F), "5.1 声道掩码");
        assert_eq!(&body[24..40], &subformat_guid(true));
        assert_eq!(
            default_channel_mask(3),
            0,
            "未列出的声道数回退为 0 (无映射)"
        );
    }

    /// 判据 11: 多声道 EXTENSIBLE 文件也能被自己的读取器读回。
    #[test]
    fn extensible_container_round_trips() {
        // 显式给出掩码, 这样写出的 `fmt ` 与读回的 `PcmFormat` 逐字段相等。
        // 掩码为 `None` 时会被物化成默认掩码 —— 那是**有意的**回退, 见下面的断言。
        let format = PcmFormat {
            channel_mask: Some(0x3F),
            ..PcmFormat::float(6, 48_000, 32)
        };
        let data = vec![0u8; 6 * 4 * 10];
        let plan =
            ContainerPlan::for_payload(ContainerKind::Bw64, format, data.len() as u64, 10, None);
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(parsed.format, format);
        assert_eq!(parsed.kind, ContainerKind::Bw64);
        assert_eq!(parsed.sizes.sample_count, 10);
    }

    /// `WAVE_FORMAT_EXTENSIBLE` 的 `wValidBitsPerSample = 0` 必须**回退**到容器的
    /// `wBitsPerSample` —— 不得让 `PcmFormat::bits_per_sample` 变成 0。
    ///
    /// # 量的是什么
    ///
    /// 一个 40 字节的 `fmt ` 负载: `wBitsPerSample` = 32、`wValidBitsPerSample` = 0、
    /// 6 声道、掩码 `0x3F`、整数 PCM GUID。量的是 [`parse_container`] 读回来的三个
    /// **整数**: `bits_per_sample`、`block_align()` 与 `sizes.sample_count`。
    ///
    /// # 为什么这条契约必须钉住
    ///
    /// `wValidBitsPerSample = 0` 被当成真值时会写进 `bits_per_sample`, 于是
    /// `bytes_per_sample()` 与 `block_align()` 也都是 0, 而 `parse_container` 求帧数的
    /// 那次除法只能靠 `max(1)` 兜住 —— 结果是**把 `data` 负载的字节数当成帧数**。
    /// 那是编出来的读数, 不是文件里写的。（`wBitsPerSample = 0` 走的是另一条路:
    /// 在建 `PcmFormat` 之前就拒绝, 见 `Rf64Error::ZeroBitsPerSample`。）
    ///
    /// # 注入实测（本机, 先全绿后补本判据）
    ///
    /// 在本判据落地**之前**, 把 `parse_fmt_payload` 里 `le::read_u16(payload, 18)`
    /// 之后的 `.filter(|&bits| bits > 0)` 删掉（那一版现位于第 1781 行）⇒ 本 crate 的
    /// `--lib --tests` 全量表读数是 **211 passed; 0 failed**, 一条都不红;
    /// 把回退值 `.unwrap_or(bits_per_sample)` 改成 `.unwrap_or(0)` 同样全绿。
    /// 两种注入在本判据下各自变红。
    #[test]
    fn a_zero_valid_bits_per_sample_falls_back_to_the_container_bit_depth() {
        // 读侧入口是 `parse_container`, 因此手搭容器: 12 字节顶层头 + `fmt ` + `data`。
        // 顶层长度字段写 0（`parse_container` 不读它）。
        let container = |valid_bits: u16, data_len: usize| -> Vec<u8> {
            let mut body = PcmFormat::integer(6, 48_000, 32).fmt_payload();
            // 前提: 这个形状必须真的走 EXTENSIBLE 分支, 否则本判据没有射程。
            assert_eq!(body.len(), 40, "EXTENSIBLE 负载长度");
            assert_eq!(le::read_u16(&body, 0), Some(0xFFFE), "格式标签");
            assert_eq!(le::read_u16(&body, 14), Some(32), "容器 wBitsPerSample");
            body[18..20].copy_from_slice(&le::u16(valid_bits));
            let data = vec![0u8; data_len];
            let mut raw = Vec::new();
            raw.extend_from_slice(b"RIFF");
            raw.extend_from_slice(&0u32.to_le_bytes());
            raw.extend_from_slice(b"WAVE");
            push_chunk(&mut raw, b"fmt ", &body);
            push_chunk(&mut raw, b"data", &data);
            raw
        };

        // 负载 = 16 帧 × 6 声道 × 4 字节 = 384 字节。除数 24 与除数 1 的差别在这里
        // 是 16 与 384 —— 后者正是"把负载字节数当成帧数"。
        let parsed = parse_container(&container(0, 24 * 16)).expect("0 是回退, 不是拒绝");
        assert_eq!(
            parsed.format.bits_per_sample, 32,
            "wValidBitsPerSample=0 必须回退到容器的 wBitsPerSample; 实际读到 {}",
            parsed.format.bits_per_sample
        );
        assert_eq!(parsed.format.block_align(), 24, "6 声道 × 4 字节");
        assert_eq!(parsed.format.byte_rate(), 1_152_000, "48000 × 24");
        assert_eq!(
            parsed.sizes.sample_count, 16,
            "帧数 = 负载字节数 ÷ nBlockAlign; 实际读到 {} (384 是负载字节数)",
            parsed.sizes.sample_count
        );
        assert_eq!(
            parsed.format.channel_mask,
            Some(0x3F),
            "EXTENSIBLE 分支真的走到了"
        );
        assert!(!parsed.format.is_float, "整数 PCM GUID");
    }

    /// `wValidBitsPerSample` **小于**容器的 `wBitsPerSample` 时必须被**采纳** ——
    /// 读取器读的是这个字段, 不是"永远用容器的值"。
    ///
    /// 本判据是上面那条的**反面**: 少了它, "读取器一律忽略 `wValidBitsPerSample`"
    /// 这种改法也能让上面那条全绿。本机注入实测（在本判据落地之前）: 把
    /// `let valid_bits = if tag == 0xFFFE` 的谓词改成 `0x0001`（那一版现位于第 1780 行）
    /// ⇒ 全量表 **211 passed; 0 failed**。本判据在该注入下变红。
    ///
    /// 读数: 6 声道 / 容器 32 位 / 有效 24 位 ⇒ `block_align` 是 **18**（不是 24）,
    /// `byte_rate` 是 864000, 负载 16 帧 × 18 字节 = 288 字节 ⇒ 帧数 16。
    #[test]
    fn a_valid_bits_per_sample_below_the_container_is_adopted() {
        let mut body = PcmFormat::integer(6, 48_000, 32).fmt_payload();
        assert_eq!(body.len(), 40, "EXTENSIBLE 负载长度");
        assert_eq!(le::read_u16(&body, 14), Some(32), "容器 wBitsPerSample");
        body[18..20].copy_from_slice(&le::u16(24)); // wValidBitsPerSample = 24
        let data = vec![0u8; 18 * 16];
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &body);
        push_chunk(&mut raw, b"data", &data);

        let parsed = parse_container(&raw).expect("24 位有效位深是合法的");
        assert_eq!(parsed.format.bits_per_sample, 24, "有效位深必须被采纳");
        assert_eq!(parsed.format.block_align(), 18, "6 声道 × 3 字节");
        assert_eq!(parsed.format.byte_rate(), 864_000, "48000 × 18");
        assert_eq!(parsed.sizes.sample_count, 16, "288 ÷ 18");
    }

    /// 判据 12b: `ds64` 里那个 **64 位** `dataSize` 可以让"负载起点 + 声明长度"溢出
    /// `usize`。这种输入必须走 `Truncated` 出口 —— **不得 panic, 也不得回绕**。
    ///
    /// 注入证明（本机实测）：把本轮引入的 `checked_add` 换回普通的 `+`，本判据在
    /// debug 下以 `attempt to add with overflow` 结束（实测位置是读 `data` 负载长度
    /// 的那次加法），而不是返回 `Err`。release 下同一处会**回绕**：`cursor` 因此可以
    /// 倒退到已经解析过的位置，同一个 chunk 被反复解析。两种行为都不是本模块的契约。
    ///
    /// 断言只钉"`Err(Truncated)`"而不是某一个具体字节数：这条判据管的是算术的
    /// **定义域**，与文件长度无关。
    #[test]
    fn an_oversized_declared_length_is_rejected_instead_of_overflowing() {
        // 每个值都让 `data` 的"负载起点 + 声明长度"越过 `usize::MAX`（本机 64 位；
        // 32 位下 `usize::MAX` 那一项也越界，因此这组值不是平台相关的）。
        let extremes = [
            u64::MAX,
            u64::MAX - 1,
            u64::MAX - 7,
            usize::MAX as u64,
            (usize::MAX - 32) as u64,
        ];
        for declared in extremes {
            let plan = ContainerPlan::for_payload(ContainerKind::Rf64, stereo_16bit(), 16, 4, None);
            let mut file = plan.header_bytes();
            // ds64 的负载从偏移 20 开始: riffSize @20, dataSize @28, sampleCount @36。
            file[28..36].copy_from_slice(&declared.to_le_bytes());
            file.extend_from_slice(&[0u8; 16]);
            assert!(
                matches!(parse_container(&file), Err(Rf64Error::Truncated { .. })),
                "ds64 dataSize = {declared} 必须被拒绝, 不是 panic/回绕"
            );
        }
    }

    /// 判据 12c: **非 `data`** 的 chunk 把 32 位长度字段顶到 `0xFFFFFFFF` 时同样只报错。
    ///
    /// 这条与 12b 是两条不同的路径：那里的和来自 `ds64`（64 位字段），这里的和来自
    /// chunk 头自己的 32 位长度。在 32 位宿主上 `payload_offset + effective` 同样可能
    /// 溢出，因此两条都必须走 checked 加法。
    #[test]
    fn a_chunk_that_declares_the_max_u32_length_is_rejected() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        raw.extend_from_slice(b"fmt ");
        // 哨兵在非 `data` chunk 上就是"声明了 4 GiB 负载"的损坏文件。
        raw.extend_from_slice(&SENTINEL_U32.to_le_bytes());
        raw.extend_from_slice(&stereo_16bit().fmt_payload());
        assert!(matches!(
            parse_container(&raw),
            Err(Rf64Error::Truncated { .. })
        ));
    }

    /// 判据 12e: `fmt ` 声明的声道布局放不进 `nBlockAlign` 的 `u16` 时被**拒绝**,
    /// 而不是让帧对齐的乘法溢出。
    ///
    /// 注入证明（两条都变红，实测其一）：把 `block_align` 改回普通乘法并删掉
    /// `parse_fmt_payload` 里那条上限检查 ⇒ 本判据以 `attempt to multiply with
    /// overflow` 结束；只把 `block_align` 改成饱和而**保留**检查 ⇒ 解析会成功，
    /// 本判据在那条 `Err` 断言上变红。
    #[test]
    fn unrepresentable_block_align_is_rejected() {
        let mut over = stereo_16bit().fmt_payload();
        over[2..4].copy_from_slice(&0xFFFFu16.to_le_bytes()); // 声道数
        over[14..16].copy_from_slice(&32u16.to_le_bytes()); // 每样本位数
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &over);
        push_chunk(&mut raw, b"data", &payload(2));
        assert_eq!(
            parse_container(&raw),
            Err(Rf64Error::UnrepresentableBlockAlign {
                channels: 0xFFFF,
                bytes_per_sample: 4
            })
        );

        // 合法端点必须仍然能解析（否则这条判据只是把功能关掉）:
        // 8191 声道 × 4 字节 = 32764 ≤ u16::MAX。
        let mut edge = stereo_16bit().fmt_payload();
        edge[2..4].copy_from_slice(&8191u16.to_le_bytes());
        edge[14..16].copy_from_slice(&32u16.to_le_bytes());
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &edge);
        push_chunk(&mut raw, b"data", &payload(2));
        let parsed = parse_container(&raw).expect("合法端点必须能解析");
        assert_eq!(parsed.format.channels, 8191);
        assert_eq!(parsed.format.block_align(), 32_764);
    }

    /// 手搭一个 `RIFF` 容器: 12 字节头 + 一个 `fmt ` chunk + 可选的 `data` chunk。
    ///
    /// `fmt_declared_len` 与 `fmt_body.len()` 可以**不一致** —— 那正是本判据要构造的
    /// 畸形形状。顶层大小字段写 0: [`parse_container`] 不读它。
    fn raw_riff(fmt_declared_len: u32, fmt_body: &[u8], with_data: bool) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        raw.extend_from_slice(b"fmt ");
        raw.extend_from_slice(&fmt_declared_len.to_le_bytes());
        raw.extend_from_slice(fmt_body);
        if fmt_body.len() % 2 == 1 {
            raw.push(0);
        }
        if with_data {
            push_chunk(&mut raw, b"data", &payload(2));
        }
        raw
    }

    /// 判据 12f: 读取器**拒绝族**里此前一条判据都没有的三个成员各自被明确拒绝。
    ///
    /// # 量的是什么
    ///
    /// `Rf64Error` 在**本判据落地时**共 13 个变体。把这 13 个变体各自在**本模块的判据
    /// 代码**里出现的次数数一遍（数法: 在 `#[cfg(test)] mod tests` 的字节范围里对每个变体
    /// `grep -c`; 用 `git show` 取本判据落地**之前**的那一版）: **9 个变体 ≥ 1 次,
    /// 4 个是 0 次**。4 个里有一个不在本判据的射程内 —— `Io` 是底层 I/O 的透传包装
    /// （只能由真实的读写失败产生, 不是对字节输入的判决）。剩下这 3 个都**从字节
    /// 输入可达**, 却一条判据也没有:
    ///
    /// | 变体 | 生产代码里的落点 | 加本判据之前的判据数 |
    /// | :--- | :--- | :--- |
    /// | `BadFmtLen` | `fmt ` 负载短于 16 字节, 或 EXTENSIBLE 标签的负载短于 40 字节 | 0 |
    /// | `MissingData` | 容器里没有 `data` chunk | 0 |
    /// | `ZeroChannels` | `fmt ` 里的声道数为 0 | 0 |
    ///
    /// > ⚠️ **变体计数已同步（新增成员, 不是弱化）**: 本判据落地之后 `Rf64Error` 又多了
    /// > 4 个变体 —— `ZeroSampleRate`、`DataSizeMismatch`、`BadStartTimecode` 与
    /// > `ZeroBitsPerSample`, 因此**现在是 17 个**。其中 `ZeroBitsPerSample` 是**读取器侧
    /// > 可达的字节判决**, 已按本判据的形式补进下面的 `cases`（`[Case; 4]` → `[Case; 5]`,
    /// > 正文说明由"四种畸形形状"改为"五种"）。另外三个是**写入器侧 / 构造期**的拒绝
    /// > （`DataSizeMismatch` 与 `BadStartTimecode` 在写入点, `ZeroSampleRate` 两处都有
    /// > 但不在本判据的射程内）, 各自有自己的判据:
    /// > `the_payload_length_must_match_the_declared_data_length`、
    /// > `a_timecode_field_must_be_exactly_two_ascii_digits`、
    /// > `a_zero_sample_rate_is_refused_before_any_byte`。
    ///
    /// 本判据把每一个都喂给 [`parse_container`] 并断言**具体的**变体, 而不是
    /// "返回了 `Err` 就行"。
    ///
    /// # 为什么"返回了 `Err` 就行"不够（实测的盲区, 三种注入全绿）
    ///
    /// 同模块的 `every_single_byte_mutation_returns_a_result_instead_of_panicking`
    /// **明确接受 `Ok`**（它的文档写了理由: 破坏可能落在不被读取的字节上）, 因此它
    /// 抓不到下面三种注入。三者各自单独施加在**加本判据之前**的判据集上, 本机实测
    /// 读数都是 `85 passed; 0 failed` —— 整个拒绝族对这三条语义是**盲**的:
    ///
    /// 1. 删掉 `parse_fmt_payload` 里的声道数检查（现位于第 1330 行）⇒ 声道数为 0 的
    ///    `fmt ` 被解析**成功**, 调用方拿到一个 `block_align()` 为 0 的格式;
    /// 2. 把 `data` 缺失的出口换成"空区间"（现位于第 1301 行）⇒ 一个声称了 `fmt `
    ///    却没有任何音频的容器被当成成功;
    /// 3. 把短 `fmt ` 那个出口的错误换成 `MissingFmt`（现位于第 1324 行）⇒ 调用方读到
    ///    错误的诊断（"缺 fmt chunk"，而文件里有一个坏的 `fmt ` chunk）。
    ///
    /// 三种注入在本判据下各自变红，红行点名下面表里的 `case`。
    #[test]
    fn every_member_of_the_reader_rejection_family_is_pinned() {
        // 五种畸形形状。每个闭包都**不捕获**环境（因此能放进 `fn` 指针表里）
        // 并现场造出容器, 使每行自证其输入。
        //
        // 这个别名不是风格: 把三元素元组直接写成数组的元素类型会触发
        // `clippy::type_complexity`（本项目 `clippy::all` 是 `deny`）, 而加
        // `#[allow]` 是弱化门禁。别名让类型仍被写清楚, 同时满足 lint。
        type Case = (&'static str, fn() -> Vec<u8>, Rf64Error);
        let cases: [Case; 5] = [
            (
                "fmt 声明长度 10 (< 16)",
                || raw_riff(10, &stereo_16bit().fmt_payload()[..10], true),
                Rf64Error::BadFmtLen(10),
            ),
            (
                "fmt 声明长度 16 但 tag 是 EXTENSIBLE (需要 40)",
                || {
                    let mut body = stereo_16bit().fmt_payload();
                    body[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
                    raw_riff(16, &body, true)
                },
                Rf64Error::BadFmtLen(16),
            ),
            (
                "容器里没有 data chunk",
                || raw_riff(16, &stereo_16bit().fmt_payload(), false),
                Rf64Error::MissingData,
            ),
            (
                "fmt 的声道数为 0",
                || {
                    let mut body = stereo_16bit().fmt_payload();
                    body[2..4].copy_from_slice(&0u16.to_le_bytes());
                    raw_riff(16, &body, true)
                },
                Rf64Error::ZeroChannels,
            ),
            (
                "fmt 的位深为 0",
                || {
                    let mut body = stereo_16bit().fmt_payload();
                    body[14..16].copy_from_slice(&0u16.to_le_bytes());
                    raw_riff(16, &body, true)
                },
                Rf64Error::ZeroBitsPerSample,
            ),
        ];
        for (case, build, expected) in cases {
            assert_eq!(parse_container(&build()), Err(expected), "{case}");
        }

        // 基准（防空判据）: 同一个构造函数 + 一个合法 `fmt ` + `data` ⇒ 必须成功。
        // 少了这一条, "五种形状全都 `Err`"也可能只是因为这个构造器造出来的容器
        // 本来就坏, 而与被点名的字段无关。
        assert!(
            parse_container(&raw_riff(16, &stereo_16bit().fmt_payload(), true)).is_ok(),
            "基准形状必须能解析, 否则上面的五行没有区分力"
        );
    }

    /// `WAVE_FORMAT_EXTENSIBLE` 的 `fmt ` 负载在 **16..39 字节**之间时必须**干净拒绝**,
    /// 不得靠切片越界 panic。
    ///
    /// 40 字节是 18 字节的 `WAVEFORMATEX` 加 22 字节的扩展部分（`cbSize` = 22 时
    /// GUID 落在 24..40）。长度不足 40 时偏移 24..40 的 GUID 读取会越界, 因此
    /// `payload.len() < 40` 那条检查**不是**冗余 —— 它是这条路径唯一的守卫。
    ///
    /// # 注入实测（本机, 在本判据落地之前）
    ///
    /// 把那条检查的上界从 40 放宽到 24 ⇒ 全量表读数是 **211 passed; 0 failed**。
    /// 原因是既有的拒绝族判据只喂了**16 字节**的 EXTENSIBLE 负载（16 < 24, 仍然被
    /// 拦下）, 24..39 这一段没有任何判据走过。放宽后本判据以 **panic**
    /// （`range end index 40 out of range`）变红。
    ///
    /// # 读数
    ///
    /// 20 / 24 / 30 / 39 四个长度都必须是 `Err(BadFmtLen(长度))`; 同一个形状补齐到
    /// 40 字节必须能解析（否则本判据只是把这个分支关掉）。
    #[test]
    fn an_extensible_fmt_payload_shorter_than_forty_bytes_is_rejected() {
        let full = PcmFormat::integer(6, 48_000, 32).fmt_payload();
        assert_eq!(full.len(), 40, "EXTENSIBLE 负载长度");
        assert_eq!(le::read_u16(&full, 0), Some(0xFFFE), "格式标签");
        for len in [20usize, 24, 30, 39] {
            assert_eq!(
                parse_container(&raw_riff(len as u32, &full[..len], true)),
                Err(Rf64Error::BadFmtLen(len as u32)),
                "{len} 字节的 EXTENSIBLE 负载"
            );
        }
        // 基准: 补齐到 40 字节必须能解析。
        assert!(
            parse_container(&raw_riff(40, &full, true)).is_ok(),
            "40 字节的 EXTENSIBLE 负载必须能解析, 否则上面的四行没有区分力"
        );
    }

    /// 无 `ds64` 的 `RIFF` 里帧数 = `data` 负载字节数 ÷ `nBlockAlign` ——
    /// `nBlockAlign` = 1 的 8 位单声道也按 **1** 除（帧数就是字节数）。
    ///
    /// `parse_container` 的除法写作 `block_align().max(1)`。`max(1)` 是**除零兜底**:
    /// `block_align()` 为 0 需要声道数或位深为 0, 而这两条在更早处已被拒绝, 因此
    /// 解析成功的文件里除数恒 ≥ 1。本判据钉住"除数就是 `nBlockAlign` 本身",
    /// 使兜底值不能悄悄变成别的数。
    ///
    /// # 注入实测（本机, 在本判据落地之前）
    ///
    /// 把 `.max(1)` 改成 `.max(2)` ⇒ 全量表读数是 **211 passed; 0 failed**。
    /// 8 位单声道（`nBlockAlign` = 1）此前没有任何判据走过, 而那正是唯一能分辨
    /// 除数 1 与 2 的输入。本判据在该注入下变红（帧数 64 → 32）。
    #[test]
    fn the_frame_count_divides_by_the_exact_block_align() {
        // 8 位单声道: 1 声道 × 1 字节 ⇒ `nBlockAlign` 1。
        let body = PcmFormat::integer(1, 48_000, 8).fmt_payload();
        assert_eq!(le::read_u16(&body, 12), Some(1), "写出的 nBlockAlign");
        let data = vec![0u8; 64];
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &body);
        push_chunk(&mut raw, b"data", &data);

        let parsed = parse_container(&raw).expect("8 位单声道是合法的");
        assert_eq!(parsed.format.bits_per_sample, 8);
        assert_eq!(parsed.format.block_align(), 1);
        assert_eq!(parsed.format.byte_rate(), 48_000, "48000 × 1");
        assert_eq!(
            parsed.sizes.sample_count, 64,
            "nBlockAlign = 1 ⇒ 帧数 = 负载字节数; 实际读到 {}",
            parsed.sizes.sample_count
        );
    }

    /// 判据 12d: **任意单字节破坏都不 panic**（把每个字节单独置成 `0xFF` 后重解析），
    /// 三种容器各扫一遍：`RIFF`（无 `ds64`、长度字段是真值）与 `RF64` / `BW64`
    /// （有 `ds64`、`data` 长度是哨兵）走的是两条不同的算术路径。
    ///
    /// 这是"损坏输入"那一族的机械读数：本判据不断言"必须是 `Err`"——有些破坏会落在
    /// 不被读取的字节上，于是解析成功是**允许**的；它断言的是**每一种破坏都只能返回
    /// `Ok` 或 `Err`**。`parse_container` 的循环前进量现在全部是 checked 加法，因此
    /// 破坏声明长度也不可能让解析器回绕或死循环。
    ///
    /// **本判据是本轮的探针，不是补充**：它第一次跑就抓到了第二条缺陷 ——
    /// 由文件字节算出的 `channels` 让 `PcmFormat::block_align` 里那次
    /// `channels * bytes_per_sample` 乘法在 debug 下溢出（实测
    /// `attempt to multiply with overflow`）。修法是两侧都做：访问器饱和 +
    /// [`Rf64Error::UnrepresentableBlockAlign`] 拒绝。
    #[test]
    fn every_single_byte_mutation_returns_a_result_instead_of_panicking() {
        let data = payload(8);
        let bext_v2 = Bext {
            version: 2,
            loudness: Some(Loudness {
                loudness_value: Loudness::from_lufs(-14.0),
                loudness_range: 500,
                max_true_peak_level: Loudness::from_dbtp(-1.0),
                max_momentary_loudness: Loudness::from_lufs(-12.0),
                max_short_term_loudness: Loudness::from_lufs(-13.0),
            }),
            coding_history: "A=PCM,F=48000,W=24,M=stereo,T=Yeban".to_owned(),
            ..Bext::default()
        };
        // 三种容器各一份基准：`RIFF` 走"没有 ds64、长度字段是真值"的那条分支,
        // `RF64` / `BW64` 走 `ds64` 与哨兵那条。破坏扫描必须覆盖两条分支。
        let cases: Vec<(&str, ContainerKind, PcmFormat, Option<Bext>)> = vec![
            (
                "riff-int24",
                ContainerKind::Riff,
                PcmFormat::integer(2, 48_000, 24),
                None,
            ),
            (
                "rf64-float32",
                ContainerKind::Rf64,
                PcmFormat::float(2, 48_000, 32),
                Some(bext_v2.clone()),
            ),
            (
                "bw64-v1-bext",
                ContainerKind::Bw64,
                PcmFormat::integer(2, 48_000, 16),
                Some(Bext {
                    version: 1,
                    coding_history: "A=PCM".to_owned(),
                    ..Bext::default()
                }),
            ),
        ];

        for (case, kind, format, bext) in cases {
            let plan = ContainerPlan::for_payload(
                kind,
                format,
                data.len() as u64,
                (data.len() / usize::from(format.block_align())) as u64,
                bext,
            );
            let mut file = Vec::new();
            write_container(&mut file, &plan, &data).expect("写入");
            assert!(parse_container(&file).is_ok(), "{case}: 基准文件必须能解析");

            let mut rejected = 0usize;
            let mut accepted = 0usize;
            for index in 0..file.len() {
                let mut broken = file.clone();
                broken[index] = 0xFF;
                // 只钉"返回了结果"; `Ok` 与 `Err` 都合法 —— 有些破坏落在不被读取的字节上。
                if parse_container(&broken).is_err() {
                    rejected += 1;
                } else {
                    accepted += 1;
                }
            }
            assert_eq!(
                rejected + accepted,
                file.len(),
                "{case}: 每个字节都必须被扫到一次"
            );
            assert!(
                rejected > 0,
                "{case}: 一次破坏都没被拒绝 ⇒ 这条判据没有覆盖到被解析的字段"
            );
            assert!(
                accepted > 0,
                "{case}: 每个字节都被拒绝 ⇒ 扫描可能根本没读到基准文件"
            );
        }
    }

    /// 判据 12: `ds64` 声明的长度超过实际字节时必须报错 —— 不得"假装读到了"。
    #[test]
    fn reader_refuses_a_file_shorter_than_its_ds64_declaration() {
        let huge: u64 = 5_000_000_000;
        let plan =
            ContainerPlan::for_payload(ContainerKind::Rf64, stereo_16bit(), huge, huge / 4, None);
        let mut file = plan.header_bytes();
        file.extend_from_slice(&[0u8; 16]);
        assert!(matches!(
            parse_container(&file),
            Err(Rf64Error::Truncated { .. })
        ));
    }

    /// 判据 13: 奇数长度负载补一个 0 字节, 但 chunk 长度字段**不含**补位;
    /// `riffSize` 含补位。
    #[test]
    fn odd_payload_gets_a_pad_byte_not_counted_in_the_chunk_size() {
        let format = PcmFormat::integer(1, 48_000, 24); // 1 帧 = 3 字节
        let data = vec![1u8, 2, 3];
        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, format, data.len() as u64, 1, None);
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(parsed.sizes.data_size, 3, "chunk 长度不含补位");
        assert_eq!(parsed.data.len(), 3);
        assert_eq!(file.len(), parsed.data.end + 1, "文件末尾有一个补位字节");
        assert_eq!(
            plan.sizes.riff_size,
            file.len() as u64 - 8,
            "riffSize 含补位"
        );
        assert_eq!(file[parsed.data.end], 0, "补位字节必须是 0");
    }

    /// 判据 14: 奇数长度的 `bext` 负载同样被补位, 头部长度计算仍然自洽。
    #[test]
    fn odd_bext_coding_history_is_padded_consistently() {
        let block = Bext {
            version: 1,
            coding_history: "ABC".to_owned(), // 602 + 3 = 605 (奇数)
            ..Bext::default()
        };
        let data = payload(4);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Rf64,
            stereo_16bit(),
            data.len() as u64,
            4,
            Some(block),
        );
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        assert_eq!(plan.sizes.riff_size, file.len() as u64 - 8);
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(
            chunk_order(&file).expect("解析"),
            vec![*b"ds64", *b"fmt ", *b"bext", *b"data"]
        );
        assert_eq!(
            parsed.bext.expect("有 bext").coding_history,
            "ABC",
            "CodingHistory 必须原样读回"
        );
    }

    /// 判据 15: 损坏/非 WAVE 输入被明确拒绝, 不得 panic。
    #[test]
    fn malformed_inputs_are_rejected_without_panicking() {
        assert_eq!(
            parse_container(b""),
            Err(Rf64Error::Truncated {
                what: "顶层 fourcc",
                got: 0
            })
        );
        assert_eq!(
            parse_container(b"XXXXWAVE"),
            Err(Rf64Error::NotWaveContainer)
        );
        assert_eq!(
            parse_container(b"RIFF\x00\x00\x00\x00NOPE"),
            Err(Rf64Error::NotWaveContainer)
        );
        assert_eq!(
            parse_container(b"RIFF\x04\x00\x00\x00WAVE"),
            Err(Rf64Error::MissingFmt)
        );

        // RF64 但缺 ds64（只有 fmt + 哨兵化的 data）
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RF64");
        raw.extend_from_slice(&SENTINEL_U32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &stereo_16bit().fmt_payload());
        raw.extend_from_slice(b"data");
        raw.extend_from_slice(&SENTINEL_U32.to_le_bytes());
        assert_eq!(parse_container(&raw), Err(Rf64Error::MissingDs64));

        // ds64 的 chunk 长度字段错误
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RF64");
        raw.extend_from_slice(&SENTINEL_U32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        raw.extend_from_slice(b"ds64");
        raw.extend_from_slice(&16u32.to_le_bytes());
        raw.extend_from_slice(&[0u8; 16]);
        assert_eq!(parse_container(&raw), Err(Rf64Error::BadDs64Len(16)));

        // 不受支持的格式标签
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        let mut fmt = stereo_16bit().fmt_payload();
        fmt[0..2].copy_from_slice(&0x0002u16.to_le_bytes()); // ADPCM
        push_chunk(&mut raw, b"fmt ", &fmt);
        assert_eq!(
            parse_container(&raw),
            Err(Rf64Error::UnsupportedFormatTag(0x0002))
        );
    }

    #[test]
    fn container_kind_fourcc_round_trips() {
        for kind in [
            ContainerKind::Riff,
            ContainerKind::Rf64,
            ContainerKind::Bw64,
        ] {
            assert_eq!(ContainerKind::from_fourcc(kind.fourcc()), Some(kind));
        }
        assert_eq!(ContainerKind::from_fourcc(*b"WAVE"), None);
        assert!(!ContainerKind::Riff.uses_ds64());
        assert!(!ContainerKind::Rf64.fourcc().is_empty());
        assert!(ContainerKind::Bw64.uses_ds64());
    }

    /// 判据 16: 响度的 0.01 单位缩放、饱和与未知哨兵。
    #[test]
    fn loudness_scaling_is_hundredths_and_saturates() {
        assert_eq!(Loudness::from_lufs(-23.0), -2300);
        assert_eq!(Loudness::from_dbtp(-1.0), -100);
        assert_eq!(Loudness::from_lufs(0.0), 0);
        assert_eq!(Loudness::from_lufs(f32::NAN), Loudness::UNKNOWN);
        assert_eq!(Loudness::from_lufs(1.0e9), i16::MAX);
        assert_eq!(Loudness::from_lufs(-1.0e9), i16::MIN + 1);

        let round_trip = Loudness::from_bytes(
            &Loudness {
                loudness_value: -1400,
                loudness_range: 750,
                max_true_peak_level: -100,
                max_momentary_loudness: 123,
                max_short_term_loudness: Loudness::UNKNOWN,
            }
            .to_bytes(),
        )
        .expect("10 字节");
        assert_eq!(round_trip.loudness_value, -1400);
        assert_eq!(round_trip.max_short_term_loudness, Loudness::UNKNOWN);
    }

    /// 判据 17: 超长文本字段按**字节**截断并 NUL 填充, 不 panic、不越界。
    #[test]
    fn overlong_text_fields_are_truncated_not_panicking() {
        let block = Bext {
            description: "x".repeat(1000),
            originator: "y".repeat(100),
            originator_reference: "z".repeat(100),
            coding_history: String::new(),
            ..Bext::default()
        };
        let bytes = block.to_bytes();
        assert_eq!(bytes.len(), BEXT_FIXED_LEN);
        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert_eq!(decoded.description.len(), 256);
        assert_eq!(decoded.originator.len(), 32);
        assert_eq!(decoded.originator_reference.len(), 32);
    }

    /// 判据 17b: 超长**多字节**文本字段的截断点落在**字符边界**上 —— 字段的有效区必须是
    /// 合法 UTF-8, 往返读回不得出现替换字符 U+FFFD, 且截断点是**最大的**字符边界。
    ///
    /// 这是判据 17 的补集: 那一条只用纯 ASCII（每个字节都是字符边界）, 因此它对
    /// "按字节硬截"与"按字符边界截"**不可分辨**。本条用 3 字节的 `→`（`E2 86 92`）
    /// 让 256 / 32 这两个宽度落在字符的**中间**。
    ///
    /// 注入（先红后还原）: 把 `push_fixed` 的 `take` 换回 `raw.len().min(width)`
    /// （不再回退到字符边界）⇒ 本判据的断言全部变红。
    #[test]
    fn overlong_multibyte_text_fields_are_cut_on_a_character_boundary() {
        const ARROW: &str = "\u{2192}"; // UTF-8: E2 86 92, 3 字节
        let description = ARROW.repeat(200); // 600 字节 > 256
        let originator = ARROW.repeat(20); // 60 字节 > 32
        let reference = ARROW.repeat(20);
        let block = Bext {
            description: description.clone(),
            originator: originator.clone(),
            originator_reference: reference.clone(),
            coding_history: String::new(),
            ..Bext::default()
        };
        let bytes = block.to_bytes();
        assert_eq!(bytes.len(), BEXT_FIXED_LEN);

        for (width, at, value) in [
            (256usize, 0usize, description.as_str()),
            (32, 256, originator.as_str()),
            (32, 288, reference.as_str()),
        ] {
            let field = &bytes[at..at + width];
            let used = field.iter().position(|&byte| byte == 0).unwrap_or(width);
            let text = core::str::from_utf8(&field[..used])
                .unwrap_or_else(|error| panic!("宽度 {width} 的字段有效区不是合法 UTF-8: {error}"));
            assert!(
                value.starts_with(text),
                "有效区 {text:?} 不是输入 {value:?} 的前缀"
            );
            assert!(
                field[used..].iter().all(|&byte| byte == 0),
                "有效区之后的 {} 字节必须全是 NUL 补位",
                width - used
            );
            // 最大性: 有效区里若还能再放一个字符, 截断点就不是最大的字符边界。
            if let Some(next) = value[used..].chars().next() {
                assert!(
                    used + next.len_utf8() > width,
                    "宽度 {width} 里还能再放一个 {next:?}（{} 字节）",
                    next.len_utf8()
                );
            }
        }

        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert!(
            !decoded.description.contains('\u{FFFD}'),
            "回读出现替换字符"
        );
        assert!(!decoded.originator.contains('\u{FFFD}'), "回读出现替换字符");
        assert!(
            !decoded.originator_reference.contains('\u{FFFD}'),
            "回读出现替换字符"
        );
        assert!(description.starts_with(&decoded.description));
        assert!(originator.starts_with(&decoded.originator));

        // 变异敏感度自证: 同一份输入上朴素的"按字节截断"必然产出非法 UTF-8。
        // 没有这条, 上面的"合法 UTF-8"断言可能是空判据（例如输入恰好对齐）。
        assert!(
            core::str::from_utf8(&description.as_bytes()[..256]).is_err(),
            "输入必须让按字节截断落在字符中间, 否则本判据测不到东西"
        );
        assert!(core::str::from_utf8(&originator.as_bytes()[..32]).is_err());
    }

    /// 判据 17c: 放得下的多字节字段**不得**被多截一个字符 —— 截断只发生在超长时,
    /// 且恰好占满一个字段的输入必须逐字符完整保留。
    #[test]
    fn a_multibyte_text_field_that_fits_is_written_whole() {
        const ARROW: &str = "\u{2192}"; // 3 字节
        let fits = ARROW.repeat(85); // 255 字节 < 256
        let exact = format!("{fits}x"); // 255 + 1 = 恰好 256 字节
        for (description, expected) in
            [(exact.clone(), exact.clone()), (fits.clone(), fits.clone())]
        {
            let block = Bext {
                description,
                coding_history: String::new(),
                ..Bext::default()
            };
            let bytes = block.to_bytes();
            let decoded = Bext::from_bytes(&bytes).expect("解码");
            assert_eq!(decoded.description, expected, "放得下的字段必须逐字符保留");
        }
        // `Originator` 宽 32 字节: 10 个 `→`（30 字节）放得下, 11 个（33 字节）放不下,
        // 因此第二个必须截到 30 字节而不是 32 字节。
        let block = Bext {
            originator: ARROW.repeat(11),
            coding_history: String::new(),
            ..Bext::default()
        };
        let decoded = Bext::from_bytes(&block.to_bytes()).expect("解码");
        assert_eq!(decoded.originator, ARROW.repeat(10));
        assert_eq!(decoded.originator.len(), 30);
    }

    /// 判据 18: 带 `bext` 的 RIFF 文件仍能被读回（`riffSize` 含 bext 与补位）。
    #[test]
    fn riff_with_bext_round_trips() {
        let data = payload(64);
        let block = Bext {
            version: 1,
            coding_history: "CH".to_owned(),
            ..Bext::default()
        };
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            stereo_16bit(),
            data.len() as u64,
            64,
            Some(block),
        );
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(parsed.kind, ContainerKind::Riff);
        assert_eq!(parsed.sizes.data_size, data.len() as u64);
        assert_eq!(parsed.bext.expect("有 bext").coding_history, "CH");
        assert_eq!(parsed.sizes.riff_size, file.len() as u64 - 8);
        assert_eq!(
            chunk_order(&file).expect("解析"),
            vec![*b"fmt ", *b"bext", *b"data"]
        );
        let lengths = chunk_lengths(&parsed.chunks);
        assert_eq!(lengths.get("fmt "), Some(&16));
        assert_eq!(lengths.get("data"), Some(&data.len()));
    }

    /// 判据 19: 工程模板的 `CodingHistory` **不含占位符字面量**, 且真实参数入口
    /// 写出的 `F=`/`W=` 就是调用方给的格式。
    ///
    /// 为什么这是一条判据而不是注释: 修复前 `for_project` 的字面量是
    /// `A=PCM,F=<sample_rate>,W=<bits>,M=stereo,T=Yeban` —— 一个交付文件会因此
    /// **声称自己的采样率是尖括号标记**。这条判据同时钉住两档:
    /// 不知道格式时**省略** `F=`/`W=`（BWF 的每一项都可选）, 知道时写**数字**。
    #[test]
    fn project_templates_never_write_placeholder_coding_history() {
        let unknown = Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00");
        assert_eq!(unknown.coding_history, "A=PCM,M=stereo,T=Yeban");
        assert!(
            !unknown.coding_history.contains('<') && !unknown.coding_history.contains('>'),
            "构造期不知道格式 ⇒ 省略 F=/W=, 绝不写占位符: {:?}",
            unknown.coding_history
        );

        let known = Bext::for_project_with_format(
            "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
            "2026-10-08",
            "13:37:00",
            48_000,
            24,
        );
        assert_eq!(known.coding_history, "A=PCM,F=48000,W=24,M=stereo,T=Yeban");
        // 其余字段两档必须完全一致 —— 差别**只有**编码历史这一项。
        assert_eq!(known.originator_reference, unknown.originator_reference);
        assert_eq!(known.originator, unknown.originator);
        assert_eq!(known.version, unknown.version);
        assert_eq!(known.loudness, unknown.loudness);
        assert_eq!(known.origination_date, unknown.origination_date);
        assert_eq!(known.origination_time, unknown.origination_time);

        // 参数如实转录, 不做任何单位换算或白名单。
        let other = Bext::for_project_with_format("u", "2026-10-08", "13:37:00", 44_100, 16);
        assert_eq!(other.coding_history, "A=PCM,F=44100,W=16,M=stereo,T=Yeban");
    }

    /// 判据 20: 两档编码历史都**逐字节落在 `CodingHistory` 偏移**（602）上,
    /// 并能被自己的读取器原样读回。
    ///
    /// 这条判据把"字符串对不对"升级为"字节落点对不对": 即使字符串正确, 若它
    /// 被写进了保留区, 文件依然没有一个可读的 `CodingHistory`。
    #[test]
    fn both_coding_history_flavours_land_at_offset_602_and_round_trip() {
        let cases = [
            (
                Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00"),
                "A=PCM,M=stereo,T=Yeban",
            ),
            (
                Bext::for_project_with_format(
                    "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
                    "2026-10-08",
                    "13:37:00",
                    96_000,
                    32,
                ),
                "A=PCM,F=96000,W=32,M=stereo,T=Yeban",
            ),
        ];
        for (block, expected) in cases {
            assert_eq!(block.coding_history, expected);
            let bytes = block.to_bytes();
            assert_eq!(bytes.len(), BEXT_FIXED_LEN + expected.len());
            assert_eq!(
                &bytes[BEXT_FIXED_LEN..],
                expected.as_bytes(),
                "CodingHistory 必须从固定前缀之后开始"
            );
            let decoded = Bext::from_bytes(&bytes).expect("解码");
            assert_eq!(decoded.coding_history, expected);
            assert_eq!(decoded, block, "带真实编码历史的块必须原样往返");
        }
    }

    /// 判据 21: 起始时间码 ⇒ `TimeReference` 是**采样数**, 且落在 `bext` 的 338 偏移上。
    ///
    /// # 这条判据钉住的是什么
    ///
    /// [ARCH-FMT-001]（现位于 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 第 461 行）
    /// 要求 `bext` 内嵌"录制起始时间码", 而修复前 [`Bext::for_project`] 与
    /// [`Bext::for_project_with_format`] 都把 `TimeReference` 留在 [`Bext::default`]
    /// 的 `0` ⇒ 交付文件一边写着发起时间 `13:37:00`、一边声称首个采样点在当日零点。
    /// 本判据同时钉住三件事:
    ///
    /// 1. 换算口径（ExifTool: "first sample count since midnight"）: `13:37:00` ⇒
    ///    `49_020` 秒 ⇒ ×48000 ⇒ `2_352_960_000` **采样数**（不是秒、不是帧号）;
    /// 2. **字节落点**: `bext` 负载的 338..346 小端 `u64` 就是这个数（字符串对但
    ///    写错偏移的文件依然读不到时间参照）;
    /// 3. 同一时间码同时进 `OriginationTime`（330..338）⇒ 两个字段不会互相矛盾。
    ///
    /// 负向对照也在里面: 0 与"时/分/秒越界"都**被拒绝**, 因为 `TimeReference = 0`
    /// 是一个合法读数（"首个采样点在当日零点"）, 拿它当错误哨兵会让文件说谎。
    #[test]
    fn a_start_timecode_becomes_the_time_reference_in_samples() {
        // 口径: 13:37:00 = 13·3600 + 37·60 = 49_020 秒; ×48000 = 2_352_960_000 采样。
        assert_eq!(
            time_reference_samples("13:37:00", 48_000),
            Some(2_352_960_000)
        );
        assert_eq!(time_reference_samples("00:00:00", 48_000), Some(0));
        assert_eq!(
            time_reference_samples("01:00:00", 44_100),
            Some(158_760_000)
        );
        assert_eq!(time_reference_samples("00:00:01", 96_000), Some(96_000));
        // 不支持的时长/语法**不**回落成 0（0 是合法读数, 不能当错误哨兵）。
        for bad in [
            "25:00:00",
            "00:70:00",
            "00:00:99",
            "13:37",
            "13:37:00:01",
            "-1:00:00",
            "13:37:0x",
            "",
            "a:b:c",
        ] {
            assert_eq!(
                time_reference_samples(bad, 48_000),
                None,
                "{bad:?} 不是 HH:MM:SS, 必须报错而不是写成 0"
            );
        }

        let block = Bext::for_project_with_timecode(
            "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
            "2026-10-08",
            "13:37:00",
            48_000,
            24,
        )
        .expect("13:37:00 是合法时间码");
        assert_eq!(block.time_reference, 2_352_960_000);
        assert_eq!(block.origination_time, "13:37:00");
        assert_eq!(block.coding_history, "A=PCM,F=48000,W=24,M=stereo,T=Yeban");

        // 字节落点: 日期 320..330、时间 330..338、时间参照 338..346。
        let bytes = block.to_bytes();
        assert_eq!(&bytes[330..338], b"13:37:00", "OriginationTime 在偏移 330");
        assert_eq!(
            le::read_u64(&bytes, 338),
            Some(2_352_960_000),
            "TimeReference 必须落在偏移 338"
        );
        // 读回来的是同一个数, 且重新编码逐字节相同。
        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert_eq!(decoded.time_reference, 2_352_960_000);
        assert_eq!(decoded, block);
        assert_eq!(decoded.to_bytes(), bytes);

        // 时间码不合法 ⇒ 构造失败, 且错误里带**原值**（调用方要能定位是哪一串坏的）。
        assert_eq!(
            Bext::for_project_with_timecode("u", "2026-10-08", "25:00:00", 48_000, 24),
            Err(Rf64Error::BadStartTimecode("25:00:00".to_owned()))
        );

        // 两个既有构造器的时间参照仍是 0（它们是**声明**, 不是缺口）—— 这条同时防
        // "把 for_project 的默认值悄悄改掉"。
        assert_eq!(
            Bext::for_project("u", "2026-10-08", "13:37:00").time_reference,
            0
        );
        assert_eq!(
            Bext::for_project_with_format("u", "2026-10-08", "13:37:00", 48_000, 24).time_reference,
            0
        );
    }

    /// 判据 22: 起始时间码的**每个字段**必须恰好两位 ASCII 数字。
    ///
    /// # 这条判据钉住的是什么
    ///
    /// 修复前 [`time_reference_samples`] 用 `str::parse::<u64>()` 解析三个字段,
    /// 于是 `1:2:3` / `1:02:03` / `+1:02:03` / `001:02:03` 全部被接受（本机实测,
    /// 四者都返回 `Some(178704000)`）。它们的公共缺陷不是"算错了数", 而是
    /// **写进 `OriginationTime` 的字节不是 ASCII `HH:MM:SS`** —— 那是偏移 330 的
    /// 8 字节定长字段。因此本判据有两半, 缺一半就会留一条后路:
    ///
    /// 1. **语法半**: 不是恰好两位数字的写法必须报错（`None` / 构造返回
    ///    [`Rf64Error::BadStartTimecode`]）。`"24:00:00"` 也在这一档: 它是"小时
    ///    上界"的**边界值**, 参照点是"当日零点", 而一天只有 24 小时 ⇒ 它不是一个时刻
    ///    （它等于次日零点）。把这一格写进本判据是**实测换来的**: 既有判据只钉了
    ///    `"25:00:00"`, 于是把 `hours >= 24` 写成 `hours > 24` 时**全套判据仍然全绿**
    ///    （本机实测: 85 passed, 而 `time_reference_samples("24:00:00", 48_000)` 给出
    ///    `Some(4147200000)`）;
    /// 2. **后果半**: **凡是**被接受的串, [`Bext::for_project_with_timecode`] 写出来的
    ///    `OriginationTime` 字段都恰好是 8 字节、且逐字节形如 `DD:DD:DD`。
    ///    只查第 1 半, 换一个宽松解析器就会重新溜过去。
    ///
    /// 注入: 把 `two_digit_field` 换回 `field.parse().ok()` ⇒ 第 1 半的
    /// `assert_eq!(.., None)` 立刻红, 并打出 `Some(178704000)`。
    #[test]
    fn a_timecode_field_must_be_exactly_two_ascii_digits() {
        // --- 1. 语法半 ---
        for bad in [
            "1:2:3",
            "1:02:03",
            "01:2:03",
            "01:02:3",
            "+1:02:03",
            "001:02:03",
            "24:00:00",
            " 1:02:03",
            "01:02:03 ",
            "0\u{ff11}:02:03",
            "01:02:0\u{ff13}",
        ] {
            assert_eq!(
                time_reference_samples(bad, 48_000),
                None,
                "{bad:?} 不是恰好两位 ASCII 数字的 HH:MM:SS, 必须报错"
            );
            assert_eq!(
                Bext::for_project_with_timecode("u", "2026-10-08", bad, 48_000, 24),
                Err(Rf64Error::BadStartTimecode(bad.to_owned())),
                "{bad:?} 不得进入 OriginationTime 字段"
            );
        }
        // 口径没有被顺手收紧: 合法写法给出的数一位不变。
        assert_eq!(
            time_reference_samples("01:02:03", 48_000),
            Some(178_704_000)
        );
        assert_eq!(time_reference_samples("00:00:00", 48_000), Some(0));
        assert_eq!(
            time_reference_samples("23:59:59", 48_000),
            Some(4_147_152_000)
        );

        // --- 2. 后果半: 接受 ⇒ 字段逐字节就是 8 字节 `DD:DD:DD` ---
        for good in ["00:00:00", "01:02:03", "13:37:00", "23:59:59"] {
            let block = Bext::for_project_with_timecode(
                "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
                "2026-10-08",
                good,
                48_000,
                24,
            )
            .expect("合法的 HH:MM:SS");
            let bytes = block.to_bytes();
            let field = &bytes[330..338];
            assert_eq!(
                field,
                good.as_bytes(),
                "OriginationTime 必须逐字节就是时间码本身"
            );
            assert!(
                field[2] == b':'
                    && field[5] == b':'
                    && field[0..2].iter().all(u8::is_ascii_digit)
                    && field[3..5].iter().all(u8::is_ascii_digit)
                    && field[6..8].iter().all(u8::is_ascii_digit),
                "OriginationTime 必须形如 DD:DD:DD, 实际 {field:?}"
            );
        }
    }

    /// 判据 23: `to_bytes` **只**写 [`Bext::from_bytes`] 读得回的版本。
    ///
    /// 修复前只有下界检查（`version >= 1`）, 于是版本 3 / 4 会被照原样写进文件:
    /// 本机实测 `version = 3` ⇒ `to_bytes` 写出的 `version` 字段就是 3, 而
    /// `from_bytes` 对同一串字节返回 [`Rf64Error::UnsupportedBextVersion`]。
    /// 导出器因此能产出一个**本 crate 自己读不回来**的容器, 而调用方拿到成功。
    ///
    /// 两半一起钉: 接受集必须往返, 接受集之外必须拒绝。
    #[test]
    fn every_version_the_writer_accepts_round_trips() {
        for version in [1u16, 2] {
            let mut block =
                Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00");
            block.version = version;
            if version == 1 {
                // 版本 1 没有 EBU R128 响度字段, 带上就是一个自相矛盾的块。
                block.loudness = None;
            }
            let bytes = block.to_bytes();
            let decoded = Bext::from_bytes(&bytes).expect("写入器只写读得回的版本");
            assert_eq!(decoded.version, version);
            assert_eq!(decoded, block, "版本 {version} 必须逐字段往返");
        }
    }

    /// 判据 24: 版本 3 不得被写出去 —— 读取器的接受集是 `{1, 2}`, 写入器必须相同。
    ///
    /// 注入: 把 [`Bext::to_bytes`] 的 `matches!(self.version, 1 | 2)` 换回
    /// `self.version >= 1` ⇒ 本判据不再 panic ⇒ 红（`should_panic` 未触发）。
    #[test]
    #[should_panic(expected = "字段表未核验")]
    fn the_writer_refuses_a_version_the_reader_cannot_read() {
        let mut block = Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00");
        block.version = 3;
        // 响度字段**留着**: 版本 3 在修复前正是靠这一格溜过去的
        // （`version >= 2` 的 `loudness.is_some()` 断言满足 ⇒ 整块照写）。
        let _ = block.to_bytes();
    }

    /// 判据 25: **写入器与读取器对 `fmt ` 的接受集是同一个** —— 凡是
    /// [`write_container`] 接受并写出字节的计划, [`parse_container`] 都必须读回同一个
    /// 格式; 凡是不可写的计划, 都必须在**写出任何字节之前**返回 `Err`。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: `channels × bits × is_float` 的格式网格 = 5 × 3 × 2 = **30 个格式组合**。
    /// 单位: "格式组合"。每个组合都真的调一次 [`write_container`]（写进一个 `Vec`）
    /// 再调一次 [`parse_container`]; 两个计数器 `accepted` / `refused` 分别是
    /// "写得出去且读得回来"与"被拒且 0 字节"的组合数。
    ///
    /// # 修复前的字面读数（本机实测: 同一份探针分别编 base 与本轮两版; 探针的负载是
    /// **8 字节**, 因此下面的文件长度比本判据（16 字节负载）的产物各少 8 字节）
    ///
    /// ```text
    /// channels = 0,      bits = 16 -> write_container Ok(写出 52 字节), parse_container Err(ZeroChannels)
    /// channels = 0x8000, bits = 24 -> write_container Ok(写出 76 字节), parse_container Err(UnrepresentableBlockAlign)
    /// channels = 0xFFFF, bits = 32 -> write_container Ok(写出 76 字节), parse_container Err(UnrepresentableBlockAlign)
    /// ```
    ///
    /// 修复后前一行是 `Err(ZeroChannels)` + 0 字节, 后两行是
    /// `Err(UnrepresentableBlockAlign)` + 0 字节; `channels = 2` 与 `channels = 0x3FFF`
    /// （帧对齐恰好 `≤ u16::MAX`）的字节数与判决**一位未变**。
    ///
    /// # 注入（两个方向都要有判别力）
    ///
    /// - 删掉 [`ContainerPlan::validate`] 里 `block_align_fits_u16` 那一支 ⇒
    ///   `0x8000` / `0xFFFF` 的六格回到"写出 76 字节 + 读取器拒绝" ⇒ 红;
    /// - 删掉 `channels == 0` 那一支 ⇒ 三格回到"写出 52 字节" ⇒ 红;
    /// - 把 `validate` 整个换成 `Ok(())` ⇒ 两种红的形态同时出现。
    ///
    /// 反向的防空判据是同一条断言的另一半: `channels = 2` / `0x3FFF` 必须**写得出去
    /// 且读得回来** —— 少了它, "30 格全部 `Err`"也会让本判据变绿。
    #[test]
    fn a_format_the_writer_accepts_the_reader_reads_back_identically() {
        let data = payload(4);
        let mut accepted = 0usize;
        let mut refused = 0usize;
        for channels in [0u16, 2, 0x3FFF, 0x8000, 0xFFFF] {
            for bits in [16u16, 24, 32] {
                for is_float in [false, true] {
                    let format = PcmFormat {
                        channels,
                        sample_rate: 48_000,
                        bits_per_sample: bits,
                        is_float,
                        channel_mask: None,
                    };
                    let plan = ContainerPlan::for_payload(
                        ContainerKind::Riff,
                        format,
                        data.len() as u64,
                        4,
                        None,
                    );
                    let validated = plan.validate();
                    let mut file = Vec::new();
                    let written = write_container(&mut file, &plan, &data);
                    let label = format!("ch={channels} bits={bits} float={is_float}");
                    match validated {
                        Err(expected) => {
                            refused += 1;
                            assert_eq!(
                                written,
                                Err(expected),
                                "{label}: validate 与 write_container 必须是同一个判决"
                            );
                            assert!(file.is_empty(), "{label}: 被拒的计划不得留下任何字节");
                        }
                        Ok(()) => {
                            accepted += 1;
                            written.unwrap_or_else(|error| {
                                panic!("{label}: validate Ok 却写不出去: {error}")
                            });
                            let parsed = parse_container(&file).unwrap_or_else(|error| {
                                panic!("{label}: 写得出去却读不回来: {error}")
                            });
                            assert_eq!(
                                (
                                    parsed.format.channels,
                                    parsed.format.sample_rate,
                                    parsed.format.bits_per_sample,
                                    parsed.format.is_float,
                                ),
                                (channels, 48_000, bits, is_float),
                                "{label}: fmt 的四个字段必须逐字段往返"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            accepted > 0 && refused > 0,
            "网格必须同时覆盖接受与拒绝（accepted={accepted}, refused={refused}）"
        );
    }

    /// 判据 26: **写入器不静默改写 `bext` 的文本字段** —— 一个读取器读不回同一个值的块
    /// 必须在写任何字节之前被拒绝。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: **6 种**"读不回同一个值"的形态 —— 五个定长文本字段各一个含内部 NUL 的
    /// 取值, 加 `CodingHistory` 的尾随 NUL。单位: "字段形态"。每一种都查三件事:
    /// ① 纯函数 [`Bext::field_that_does_not_round_trip`] 点名**该**字段;
    /// ② [`ContainerPlan::validate`] 与 [`write_container`] 返回同一个
    /// [`Rf64Error::UnrepresentableBextField`]; ③ `out` 里 0 字节。
    ///
    /// # 修复前的字面读数（本机实测: 同一份探针编 base 与本轮两版; 探针的负载是
    /// **8 字节** —— 长度读数只用于与修复后对比, 与判据本体的负载无关）
    ///
    /// ```text
    /// Description    = "a\0b"        -> write_container Ok(写出 662 字节), from_bytes 读回 "a"
    /// CodingHistory  = "A=PCM\0"     -> write_container Ok(写出 668 字节), from_bytes 读回 "A=PCM"
    /// ```
    ///
    /// # 注入
    ///
    /// 删掉 `validate` 里 `field_that_does_not_round_trip` 那一支 ⇒ 本判据红。本机实测的
    /// **字面**红形态不是"写出整份容器并返回 `Ok`", 而是 [`Bext::to_bytes`] 的 `panic!`:
    /// 少了这一支之后, `for_payload` 末尾的自洽检查会走到 `header_bytes`, 而后者对畸形
    /// 字段仍然 panic。两条出口都算红, 但它们钉的不是同一件事 —— 因此判据同时断言错误值
    /// **与**"0 字节"。
    ///
    /// 防空判据是后半段: 同一个字段的**合法**取值必须写得出去、读回同一个字符串, 而且
    /// **读取器读出的块永远过得了这道检查**（否则写入器会拒绝自己读取器的产物）。
    #[test]
    fn a_bext_text_field_that_cannot_round_trip_is_refused_before_any_byte() {
        let data = payload(4);
        let format = PcmFormat::integer(2, 48_000, 16);
        let cases: [(&'static str, Bext); 6] = [
            (
                "Description",
                Bext {
                    description: "a\u{0}b".to_owned(),
                    ..Bext::default()
                },
            ),
            (
                "Originator",
                Bext {
                    originator: "Yeban\u{0}DAW".to_owned(),
                    ..Bext::default()
                },
            ),
            (
                "OriginatorReference",
                Bext {
                    originator_reference: "ULID\u{0}X".to_owned(),
                    ..Bext::default()
                },
            ),
            (
                "OriginationDate",
                Bext {
                    origination_date: "2026\u{0}1008".to_owned(),
                    ..Bext::default()
                },
            ),
            (
                "OriginationTime",
                Bext {
                    origination_time: "13\u{0}37:00".to_owned(),
                    ..Bext::default()
                },
            ),
            (
                "CodingHistory",
                Bext {
                    coding_history: "A=PCM\u{0}".to_owned(),
                    ..Bext::default()
                },
            ),
        ];
        for (field, block) in cases {
            assert_eq!(
                block.field_that_does_not_round_trip(),
                Some(field),
                "{field}: 纯函数必须点名这个字段"
            );
            let plan = ContainerPlan::for_payload(
                ContainerKind::Riff,
                format,
                data.len() as u64,
                4,
                Some(block),
            );
            let expected = Rf64Error::UnrepresentableBextField { field };
            assert_eq!(plan.validate(), Err(expected.clone()), "{field}");
            let mut file = Vec::new();
            assert_eq!(
                write_container(&mut file, &plan, &data),
                Err(expected),
                "{field}: 写入器必须拒绝, 而不是截断"
            );
            assert!(file.is_empty(), "{field}: 被拒的计划不得留下任何字节");
        }

        // ---- 防空判据: 合法取值必须往返 ----
        let good = Bext {
            description: "a b".to_owned(),
            coding_history: "A=PCM".to_owned(),
            ..Bext::default()
        };
        assert_eq!(good.field_that_does_not_round_trip(), None);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            format,
            data.len() as u64,
            4,
            Some(good.clone()),
        );
        assert_eq!(plan.validate(), Ok(()));
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("合法字段必须写得出去");
        let decoded = parse_container(&file).expect("读回").bext.expect("有 bext");
        assert_eq!(decoded.description, good.description);
        assert_eq!(decoded.coding_history, good.coding_history);
        // 读取器产出的块**永远**过得了这道检查。
        assert_eq!(decoded.field_that_does_not_round_trip(), None);
    }

    /// 判据 27: 直接调 [`Bext::to_bytes`] 也不能把含 NUL 的字段悄悄写出去。
    ///
    /// 判据 26 管的是**容器**出口（`write_container` 返回 `Err`）; 这一条管**块**出口
    /// （`to_bytes` 是 `# Panics` 文档里点名的第 4 类调用方构造错误）。
    ///
    /// 注入: 删掉 `to_bytes` 里 `field_that_does_not_round_trip` 那段检查 ⇒ 本判据不再
    /// panic ⇒ 红（`should_panic` 未触发）。
    #[test]
    #[should_panic(expected = "含 NUL")]
    fn the_raw_block_writer_refuses_a_field_that_would_be_truncated() {
        let block = Bext {
            originator: "Yeban\u{0}DAW".to_owned(),
            ..Bext::default()
        };
        let _ = block.to_bytes();
    }

    /// 判据 27b: **计划层的接受集与 [`Bext`] 层的接受集是同一个** —— 一个版本不受
    /// 支持、或版本与响度块互相矛盾的 `bext` 必须在 `validate()` 处被拒绝, 而不是让
    /// [`write_container`] 在 [`Bext::to_bytes`] 的 `assert!` 上 **panic**。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: `version ∈ {1, 2, 3}` × `loudness ∈ {无, 有}` = **6 种** `bext` 形态
    /// × **3 种**容器 = **18 格**。单位: "形态格"。每格查四件事:
    /// ① [`ContainerPlan::for_payload`] 不 panic（debug 构建下它末尾的自洽检查会调
    /// `header_bytes`, 于是"计划被接受"与"头部写得出来"在 debug 下是同一件事 ——
    /// 这一格正是修复前 debug 下最先炸的地方）;
    /// ② `validate()` 与 `write_container` 返回**同一个**判决;
    /// ③ 被拒的格在 `out` 里留下 **0 字节**;
    /// ④ 被接受的格必须**往返**: `parse_container` 读回的 `bext` 逐字段等于写进去的那个。
    ///
    /// # 修复前的字面读数（本机实测: 探针把本文件按 `#[path]` 单独编译, release 与
    /// debug 各一份可执行文件; 负载 32 字节）
    ///
    /// ```text
    /// release: bext v3               -> validate() = Ok(()), write_container = PANIC（0 字节）
    /// release: bext v2 loudness=None -> validate() = Ok(()), write_container = PANIC（0 字节）
    /// release: bext v1 loudness=Some -> validate() = Ok(()), write_container = PANIC（0 字节）
    /// debug  : 上面三格都是 ContainerPlan::for_payload 自己 PANIC
    /// ```
    ///
    /// 即: 一个返回 `Result` 的写入器在**两种构建下都 panic**, 而
    /// [`ContainerPlan::header_bytes`] 的文档写的是"走 [`write_container`] 的调用方
    /// 已经过了 `validate`"。
    ///
    /// # 同一个版本号, 读取器与写入器给出同一个变体
    ///
    /// `version ∉ {1, 2}` 那一支不止"被拒绝": 判定用的**就是**读取器的变体。本判据拿
    /// 一份合法 v2 块的字节、只改版本字段（`bext` 的 `Version` 现位于第 346 字节）再交给
    /// [`Bext::from_bytes`], 得到 `Err(UnsupportedBextVersion(3))` —— 与计划侧的判决
    /// 逐字相同。这与 `block_align_fits_u16` / `ZeroBitsPerSample` 是同一个手法:
    /// 两侧共用一个谓词, 接受集不会漂移。
    ///
    /// # 注入
    ///
    /// - 删掉 [`ContainerPlan::validate`] 末尾那整段 `bext` 形态检查 ⇒ 12 个"被拒"格
    ///   全部回到 `validate` = `Ok` 且 `write_container` = `panic` ⇒ 红;
    /// - 只删掉响度那一支（保留版本那一支）⇒ 只有 `v2/无响度` 与 `v1/有响度` 两族红;
    /// - 只删掉版本那一支 ⇒ 只有 `v3` 那两族红 —— 两支各有独立的判别力。
    ///
    /// 防空判据是同一条网格的另一半: `v1/无响度` 与 `v2/有响度` **必须**写得出去、
    /// 逐字段往返, 且计数恰好是 `(接受 6, 拒绝 12)` —— 少了它, "18 格全部 `Err`"
    /// 也会让本判据变绿。
    #[test]
    fn every_bext_shape_the_writer_accepts_the_reader_reads_back() {
        let data = payload(4);
        let format = PcmFormat::integer(2, 48_000, 16);
        let legal = Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00");

        // 读取器对同一个版本号的判决: 改的是合法块的字节, 不是另造一个块。
        let mut raw = legal.to_bytes();
        raw[346..348].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            Bext::from_bytes(&raw),
            Err(Rf64Error::UnsupportedBextVersion(3)),
            "读取器对版本 3 的判决必须就是 UnsupportedBextVersion(3)"
        );

        let mut accepted = 0usize;
        let mut refused = 0usize;
        for version in [1u16, 2, 3] {
            for has_loudness in [false, true] {
                let mut block = legal.clone();
                block.version = version;
                block.loudness = if has_loudness { legal.loudness } else { None };
                for kind in [
                    ContainerKind::Riff,
                    ContainerKind::Rf64,
                    ContainerKind::Bw64,
                ] {
                    let plan = ContainerPlan::for_payload(
                        kind,
                        format,
                        data.len() as u64,
                        4,
                        Some(block.clone()),
                    );
                    let validated = plan.validate();
                    let mut file = Vec::new();
                    let written = write_container(&mut file, &plan, &data);
                    let label = format!("{kind:?}: bext v{version} loudness={has_loudness}");
                    match validated {
                        Err(expected) => {
                            refused += 1;
                            assert_eq!(
                                written,
                                Err(expected),
                                "{label}: validate 与 write_container 必须是同一个判决"
                            );
                            assert!(file.is_empty(), "{label}: 被拒的计划不得留下任何字节");
                        }
                        Ok(()) => {
                            accepted += 1;
                            written.unwrap_or_else(|error| {
                                panic!("{label}: validate Ok 却写不出去: {error}")
                            });
                            let parsed = parse_container(&file).unwrap_or_else(|error| {
                                panic!("{label}: 写得出去却读不回来: {error}")
                            });
                            assert_eq!(
                                parsed.bext,
                                Some(block.clone()),
                                "{label}: bext 必须逐字段往返"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(
            (accepted, refused),
            (6, 12),
            "接受集必须是 {{v1/无响度, v2/有响度}} × 3 种容器 = 6 格, 其余 12 格被拒"
        );
    }

    /// 判据 28: **负载长度必须等于计划声明的 `data` 长度** —— 两条长度来源不一致时,
    /// [`write_container`] 在写任何字节之前返回 [`Rf64Error::DataSizeMismatch`]。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: **6 种**长度岔口 × **3 种**容器 = **18 格**（`RIFF` / `RF64` / `BW64`）。
    /// 单位: 负载长度是**字节**, `data` 的声明长度也是**字节**。每格查三件事:
    /// ① [`write_container`] 给出 `Err(DataSizeMismatch { declared, actual })`,
    /// 且两个字段就是交进去的那两个数; ② `out` 里 **0 字节**;
    /// ③ 反向 —— 一致的那几格必须写得出去, 且 `parse_container` 读回的 `data`
    /// 范围**恰好**是整条负载。
    ///
    /// # 修复前的字面读数（本机实测: 负载 32 字节, 三种容器逐格同形）
    ///
    /// ```text
    /// 声明 34 字节, 实际 32 -> write_container Ok(写出 76 字节), parse_container Err(Truncated { what: "data 负载" })
    /// 声明 32 字节, 实际 30 -> write_container Ok(写出 74 字节), parse_container Err(Truncated { what: "data 负载" })
    /// 声明 30 字节, 实际 32 -> write_container Ok(写出 76 字节), parse_container Ok 但只交出 30 字节
    /// 声明  0 字节, 实际 32 -> write_container Ok(写出 76 字节), parse_container Ok 且报告 0 帧
    /// ```
    ///
    /// 三种容器的真实字节数是: 声明 32/实际 32 → `RIFF` 76 字节（`RF64`/`BW64` 112 字节）;
    /// 声明 34/实际 32 → 同为 76（112）; 声明 32/实际 30 → 74（110）;
    /// 声明 30 或 0/实际 32 → 76（112）。
    ///
    /// # 注入
    ///
    /// 删掉 [`write_container`] 里那段 `actual != plan.sizes.data_size` 的检查
    /// ⇒ 第 1、2 格回到上表（`Ok` 且写出字节）⇒ 红。把比较写成 `actual < declared`
    /// 只会放行"声明偏长"一格, 仍红。
    ///
    /// 防空判据是后半段: 长度一致的 2 格 × 3 种容器必须写得出去并整条读回 ——
    /// 少了它, "18 格全部 `Err`" 也会让本判据变绿。
    #[test]
    fn the_payload_length_must_match_the_declared_data_length() {
        let format = stereo_16bit();
        let data = payload(8);
        let full = data.len() as u64;
        // (声明的负载长度, 实际交进去的字节数)
        let mismatches: [(u64, usize); 4] = [
            (full + 2, data.len()),
            (full - 2, data.len()),
            (full, data.len() - 2),
            (1, data.len()),
        ];
        let matches: [(u64, usize); 2] = [(full, data.len()), (30, 30)];

        for kind in [
            ContainerKind::Riff,
            ContainerKind::Rf64,
            ContainerKind::Bw64,
        ] {
            for (declared, passed) in mismatches {
                let plan = ContainerPlan::for_payload(kind, format, declared, 8, None);
                assert_eq!(
                    plan.sizes.data_size, declared,
                    "{kind:?}: 计划里的声明值必须原样保留"
                );
                let mut file = Vec::new();
                assert_eq!(
                    write_container(&mut file, &plan, &data[..passed]),
                    Err(Rf64Error::DataSizeMismatch {
                        declared,
                        actual: passed as u64,
                    }),
                    "{kind:?}: 声明 {declared} 字节 / 实际 {passed} 字节必须被拒"
                );
                assert!(
                    file.is_empty(),
                    "{kind:?}: 被拒的写入不得留下任何字节（实测零字节）"
                );
            }

            for (declared, passed) in matches {
                let plan = ContainerPlan::for_payload(kind, format, declared, 8, None);
                let mut file = Vec::new();
                write_container(&mut file, &plan, &data[..passed]).unwrap_or_else(|error| {
                    panic!("{kind:?}: 长度一致时必须写得出去, 实际 {error}")
                });
                let parsed = parse_container(&file)
                    .unwrap_or_else(|error| panic!("{kind:?}: 写得出去却读不回来: {error}"));
                assert_eq!(
                    parsed.data.end - parsed.data.start,
                    passed,
                    "{kind:?}: 读取器必须交出**整条**负载, 不得静默丢尾巴"
                );
                assert_eq!(
                    &file[parsed.data.clone()],
                    &data[..passed],
                    "{kind:?}: 读回的负载必须逐字节相同"
                );
            }
        }
    }

    /// 判据 29: `sample_rate == 0` 在**创建任何字节之前**被拒绝 —— 不是写出一个
    /// `nAvgBytesPerSec = 0` 的容器。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: **6 种**格式组合（`1`/`2`/`6` 声道 × `16`/`32` 位）× 三种容器,
    /// 每种在 `rate = 0` 下必须被拒、在 `rate = 48_000` 下必须写得出去。
    /// 单位: 采样率是**Hz**（本判据只用到 `0` 与 `48_000` 两个值）; 拒绝的读数是
    /// [`Rf64Error::ZeroSampleRate`] 与 `out` 的**字节数**（必须是 0）。
    ///
    /// # 为什么这条不是对称性判据（如实说明）
    ///
    /// 本 crate 的读取器**读得回**零采样率的文件 —— 它照样填 `sample_rate = 0`。
    /// 这条的来源是同一个 crate 的**另一条**写入器: [`crate::wav::check_container_fields`]
    /// 拒绝 `sample_rate == 0`, 理由是 `hound` 在写 `nBlockAlign` 的那一处
    /// **现位于第 332 行**对它做除法（除数为 0 ⇒ panic）。因此"同一个显然不可用的格式
    /// 不得有两个判决"。修复前的本机实测:
    ///
    /// ```text
    /// RIFF ch=2 bits=16 rate=0 -> validate Ok(()), write_container Ok(写出 60 字节), parse_container Ok(sample_rate = 0)
    /// ```
    ///
    /// # 注入
    ///
    /// 删掉 [`ContainerPlan::validate`] 里 `sample_rate == 0` 那一支 ⇒ 每格回到
    /// 上表（`Ok` 且写出字节）⇒ 红。
    ///
    /// 防空判据是后半段: 同一个格式在 48 kHz 下必须写得出去、读回 `sample_rate = 48_000`
    /// —— 少了它, "全部 `Err`" 也会让本判据变绿。
    #[test]
    fn a_zero_sample_rate_is_refused_before_any_byte() {
        let data = payload(4);
        for kind in [
            ContainerKind::Riff,
            ContainerKind::Rf64,
            ContainerKind::Bw64,
        ] {
            for channels in [1u16, 2, 6] {
                for bits in [16u16, 32] {
                    let label = format!("{kind:?} ch={channels} bits={bits}");
                    let zero_rate = PcmFormat::integer(channels, 0, bits);
                    let plan =
                        ContainerPlan::for_payload(kind, zero_rate, data.len() as u64, 4, None);
                    assert_eq!(
                        plan.validate(),
                        Err(Rf64Error::ZeroSampleRate),
                        "{label}: 零采样率必须被 validate 拒绝"
                    );
                    let mut file = Vec::new();
                    assert_eq!(
                        write_container(&mut file, &plan, &data),
                        Err(Rf64Error::ZeroSampleRate),
                        "{label}: 零采样率必须被 write_container 拒绝"
                    );
                    assert!(
                        file.is_empty(),
                        "{label}: 被拒的写入不得留下任何字节（实测零字节）"
                    );

                    // 防空判据: 同一个格式在 48 kHz 下必须写得出去并读回同一个采样率。
                    let good = PcmFormat::integer(channels, 48_000, bits);
                    let plan = ContainerPlan::for_payload(kind, good, data.len() as u64, 4, None);
                    assert_eq!(plan.validate(), Ok(()), "{label}: 48 kHz 必须可写");
                    let mut file = Vec::new();
                    write_container(&mut file, &plan, &data)
                        .unwrap_or_else(|error| panic!("{label}: {error}"));
                    let parsed =
                        parse_container(&file).unwrap_or_else(|error| panic!("{label}: {error}"));
                    assert_eq!(
                        parsed.format.sample_rate, 48_000,
                        "{label}: 48 kHz 必须原样读回"
                    );
                }
            }
        }
    }

    /// 判据 30: `bits_per_sample == 0` 在**创建任何字节之前**被拒绝, 读取器同样拒绝
    /// 一个声明了 0 位深的 `fmt ` —— 不是写出一个 `nBlockAlign = 0` 的容器。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: **3 种**声道布局（`1` / `2` / `6` 声道, 位深固定为 0）× 三种容器, 每种查
    /// 三件事: ① [`ContainerPlan::validate`] 的判决; ② [`write_container`] 的判决与 `out`
    /// 的**字节数**（必须是 0）; ③ **同一个 `fmt ` 负载**只把 `wBitsPerSample` 字段
    /// （偏移 14, 单位是**位/样本**）改成 0 之后, [`parse_container`] 的判决。
    /// 单位: 位深是**位/样本**（本判据只用到 `0` 与 `16`/`32`）, 帧数是**帧**。
    ///
    /// # 为什么这一条是跨写入器的（如实说明）
    ///
    /// 本 crate 有**两条**写入器: 本模块的 [`write_container`] 与 [`crate::wav`] 里走
    /// `hound` 的 [`crate::wav::write_plain_wav`]。后者经
    /// [`crate::wav::check_container_fields`] 早已把 0 位深判为 `UnsupportedDepth(0)`;
    /// 修复前本模块却**接受**同一个 `PcmFormat`, 并写出 `nBlockAlign = 0` /
    /// `nAvgBytesPerSec = 0` 的 `fmt `。两条写入器对同一个格式的两个判决由跨模块判据
    /// `crate::wav::tests::the_two_writers_in_this_crate_agree_on_a_zero_bit_depth` 钉住。
    ///
    /// # 修复前的字面读数（本机实测, 三种容器逐格同形, 负载 **8 字节**）
    ///
    /// ```text
    /// Riff  bits=0 bytes_per_sample=0 block_align=0 validate=Ok(()) written=true file_bytes=52 nBlockAlign=0 nAvgBytesPerSec=0 reader=Ok((bits=0, sample_count=8))
    /// Rf64  bits=0 bytes_per_sample=0 block_align=0 validate=Ok(()) written=true file_bytes=88 nBlockAlign=0 nAvgBytesPerSec=0 reader=Ok((bits=0, sample_count=2))
    /// Bw64  bits=0 bytes_per_sample=0 block_align=0 validate=Ok(()) written=true file_bytes=88 nBlockAlign=0 nAvgBytesPerSec=0 reader=Ok((bits=0, sample_count=2))
    /// ```
    ///
    /// `RIFF` 那一格的 `sample_count = 8` 是**编出来的**: 8 字节负载 ÷ `max(nBlockAlign, 1)`
    /// = 8。`RF64`/`BW64` 那一格的 2 是 `ds64.sampleCount` 里的原值, 不是算出来的。
    ///
    /// # 注入
    ///
    /// ① 删掉 [`ContainerPlan::validate`] 里 `bits_per_sample == 0` 那一支 ⇒ 第 ①② 面
    /// 回到上表（`Ok` 且写出字节）⇒ 红; ② 删掉 `parse_fmt_payload` 里的同名检查 ⇒ 第 ③ 面
    /// 回到 `Ok(bits_per_sample = 0)` ⇒ 红。
    ///
    /// 防空判据是后半段: 同一个声道布局在 16 / 32 位下必须写得出去、读回**同一个**位深,
    /// 且读取器报告的全部样本数等于 `data` 负载的长度 —— 少了它, "全部 `Err`" 也会让
    /// 本判据变绿。
    #[test]
    fn a_zero_bit_depth_is_refused_before_any_byte() {
        let data = payload(6);
        for channels in [1u16, 2, 6] {
            for kind in [
                ContainerKind::Riff,
                ContainerKind::Rf64,
                ContainerKind::Bw64,
            ] {
                let label = format!("{kind:?} ch={channels} bits=0");

                // ①② 写入器侧: 同一个声道布局配 0 位深必须被拒, 且不留字节。
                let zero = PcmFormat::integer(channels, 48_000, 0);
                // 帧数取同一个声道布局在 16 位下的**精确**帧数（与负载长度一致）: 本判据
                // 要判的是位深那一格, 不想顺带卷入"帧数与负载长度是否一致"。
                let frames = (data.len() / 2 / usize::from(channels)) as u64;
                let plan = ContainerPlan::for_payload(kind, zero, data.len() as u64, frames, None);
                assert_eq!(
                    plan.validate(),
                    Err(Rf64Error::ZeroBitsPerSample),
                    "{label}: 0 位深必须被 validate 拒绝"
                );
                let mut file = Vec::new();
                assert_eq!(
                    write_container(&mut file, &plan, &data),
                    Err(Rf64Error::ZeroBitsPerSample),
                    "{label}: 0 位深必须被 write_container 拒绝"
                );
                assert!(
                    file.is_empty(),
                    "{label}: 被拒的写入不得留下任何字节（实测零字节）"
                );

                // 后半段（防空判据）: 同一个布局在 16 / 32 位下必须可写、可读回。
                for bits in [16u16, 32] {
                    let good = PcmFormat::integer(channels, 48_000, bits);
                    let frames = (data.len() / usize::from(good.block_align())) as u64;
                    let plan =
                        ContainerPlan::for_payload(kind, good, data.len() as u64, frames, None);
                    assert_eq!(
                        plan.validate(),
                        Ok(()),
                        "{label}: {bits} 位必须可写（否则本条判据只是把功能关掉）"
                    );
                    let mut file = Vec::new();
                    write_container(&mut file, &plan, &data)
                        .unwrap_or_else(|error| panic!("{label}: {bits} 位写不出去: {error}"));
                    let parsed = parse_container(&file)
                        .unwrap_or_else(|error| panic!("{label}: {bits} 位读不回来: {error}"));

                    assert_eq!(
                        parsed.format.bits_per_sample, bits,
                        "{label}: 位深必须原样读回"
                    );
                    assert_eq!(
                        parsed.data.len(),
                        data.len(),
                        "{label}: 读取器必须交出整条负载"
                    );

                    // ③ 读取器侧: 只把 `fmt ` 的 `wBitsPerSample`（负载偏移 14）改成 0。
                    //    偏移从**同一份合法文件**的 chunk 表里取, 因此不靠手算常量。
                    let fmt = parsed
                        .chunks
                        .iter()
                        .find(|chunk| &chunk.fourcc == b"fmt ")
                        .expect("容器必须有 fmt ");
                    let at = fmt.payload_offset + 14;
                    let mut broken = file.clone();
                    broken[at..at + 2].copy_from_slice(&0u16.to_le_bytes());
                    assert_eq!(
                        parse_container(&broken),
                        Err(Rf64Error::ZeroBitsPerSample),
                        "{label}: 声明 0 位深的 fmt 必须被读取器拒绝（字节 {at}..{}）",
                        at + 2
                    );
                }
            }
        }
    }
    /// 判据 (**类别 4: 参数极值 / 边界值**): `fmt ` 的 `nBlockAlign` 字段的可表示上界是
    /// `u16::MAX` **本身**, 不是它减一。
    ///
    /// [`PcmFormat::block_align_fits_u16`] 的文档写的是"真实的帧对齐能否**装进** `fmt `
    /// 的 `u16` `nBlockAlign` 字段" —— "装进"是**闭**区间。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把该谓词的 `exact <= u16::MAX as u32` 注入成 `exact < u16::MAX as u32`,
    /// 全量 `cargo test -p yeban-render` **全绿**（`test result: ok. 170 passed;
    /// 0 failed`）—— 既有的接受网格用的是 `0x3FFF × 4 = 32764`（接受）与
    /// `0x8000 × 2 = 65536`（拒绝）两点, 恰好**跳过 65535 这一格**。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: `channels = 21845` × `bits_per_sample = 24`（`21845 × 3 = 65535`, 恰好是
    /// 闭区间上界）与它的上邻 `channels = 21846`（`65538`）。单位: 帧对齐是**字节/帧**。
    /// 读数: 谓词的布尔、[`ContainerPlan::validate`] / [`write_container`] 的判决、
    /// `fmt ` 负载偏移 12 的 `nBlockAlign` 字段、以及 [`parse_container`] 读回的格式。
    ///
    /// # 非空证明
    ///
    /// `65535 == u16::MAX` 且 `65538 > u16::MAX` —— 两个操作数都不是退化值, 而
    /// [`PcmFormat::block_align`] 的饱和乘法在 65538 那一格读出的仍是 65535,
    /// 因此"饱和值"和"可表示"必须由谓词分开。
    #[test]
    fn the_block_align_boundary_is_u16_max_inclusive() {
        let data = payload(4);
        let at_the_edge = PcmFormat::integer(21_845, 48_000, 24);
        assert_eq!(at_the_edge.block_align(), u16::MAX, "21845 × 3 = 65535");
        assert!(
            at_the_edge.block_align_fits_u16(),
            "65535 恰好装进 u16 的 nBlockAlign 字段"
        );
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            at_the_edge,
            data.len() as u64,
            0,
            None,
        );
        assert_eq!(plan.validate(), Ok(()), "闭区间上界必须可写");
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("65535 必须写得出去");
        let parsed = parse_container(&file).expect("写出去的必须读得回来");
        assert_eq!(parsed.format.channels, 21_845);
        assert_eq!(parsed.format.block_align(), u16::MAX);
        let fmt = parsed
            .chunks
            .iter()
            .find(|chunk| &chunk.fourcc == b"fmt ")
            .expect("容器必须有 fmt ");
        assert_eq!(
            le::read_u16(&file, fmt.payload_offset + 12),
            Some(u16::MAX),
            "nBlockAlign 字段必须恰好是 65535（字节/帧）"
        );

        // 上邻一格: 65538 装不进 ⇒ 两侧一起拒绝, 且不留字节。
        let over_the_edge = PcmFormat::integer(21_846, 48_000, 24);
        assert_eq!(
            over_the_edge.block_align(),
            u16::MAX,
            "饱和乘法把 65538 也读成 65535 —— 这正是本条要分辨的"
        );
        assert!(!over_the_edge.block_align_fits_u16(), "65538 > u16::MAX");
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            over_the_edge,
            data.len() as u64,
            0,
            None,
        );
        assert_eq!(
            plan.validate(),
            Err(Rf64Error::UnrepresentableBlockAlign {
                channels: 21_846,
                bytes_per_sample: 3,
            })
        );
        let mut file = Vec::new();
        assert_eq!(
            write_container(&mut file, &plan, &data),
            Err(Rf64Error::UnrepresentableBlockAlign {
                channels: 21_846,
                bytes_per_sample: 3,
            })
        );
        assert!(file.is_empty(), "被拒的写入不得留下任何字节");
    }

    /// 判据 (**类别 7: 尺寸 / 边界值**): [`PcmFormat::bytes_per_sample`] 是**向上取整**
    /// 的容器字节数, 不是 `bits / 8` 的地板除。
    ///
    /// 位深不是 8 的倍数时两条式子差一, 而 `nBlockAlign` 与 `nAvgBytesPerSec` 都是从
    /// 它算出来的 —— 差一意味着写出去的文件声明了一个**装不下一个样本**的帧对齐。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 `self.bits_per_sample.div_ceil(8)` 注入成 `self.bits_per_sample / 8`,
    /// 全量 `cargo test -p yeban-render` **全绿**（`test result: ok. 170 passed;
    /// 0 failed`）—— 既有判据的位深网格（`16` / `24` / `32` / `0`）**全是 8 的倍数**,
    /// 两条式子在那些点上逐位相同。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一串位深 `1 / 8 / 9 / 16 / 17 / 20 / 24 / 25 / 32`（单位: **位/样本**）
    /// 与一个 20 位立体声容器。读数: [`PcmFormat::bytes_per_sample`] 的**字节/样本**、
    /// 写出的 `fmt ` 负载偏移 12 的 `nBlockAlign`（字节/帧）与偏移 8 的
    /// `nAvgBytesPerSec`（字节/秒）。
    ///
    /// # 非空证明
    ///
    /// `20 % 8 == 4` —— 至少一个取值的向上取整与地板除**不同**; 后半段的
    /// `16 / 24 / 32` 三格同时钉住"是 8 的倍数时两条式子确实一致"。
    #[test]
    fn a_bit_depth_that_is_not_a_multiple_of_eight_rounds_up() {
        for (bits, bytes) in [
            (1u16, 1u16),
            (8, 1),
            (9, 2),
            (16, 2),
            (17, 3),
            (20, 3),
            (24, 3),
            (25, 4),
            (32, 4),
        ] {
            assert_eq!(
                PcmFormat::integer(2, 48_000, bits).bytes_per_sample(),
                bytes,
                "{bits} 位的容器字节数（字节/样本）"
            );
        }
        assert_ne!(20 % 8, 0, "本条必须至少含一个 8 的非倍数");

        // 20 位立体声: nBlockAlign = ceil(20/8) × 2 = 6, nAvgBytesPerSec = 6 × 48000。
        let data = payload(6);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            PcmFormat::integer(2, 48_000, 20),
            data.len() as u64,
            1,
            None,
        );
        assert_eq!(plan.validate(), Ok(()));
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("20 位立体声必须写得出去");
        let parsed = parse_container(&file).expect("写出去的必须读得回来");
        assert_eq!(parsed.format.bits_per_sample, 20, "位深必须原样读回");
        let fmt = parsed
            .chunks
            .iter()
            .find(|chunk| &chunk.fourcc == b"fmt ")
            .expect("容器必须有 fmt ");
        assert_eq!(
            le::read_u16(&file, fmt.payload_offset + 12),
            Some(6),
            "nBlockAlign(字节/帧) 必须是 ceil(20/8) × 2 = 6"
        );
        assert_eq!(
            le::read_u32(&file, fmt.payload_offset + 8),
            Some(288_000),
            "nAvgBytesPerSec(字节/秒) 必须是 6 × 48000"
        );
    }

    /// 判据 (**类别 1: 非平凡输入 / 字段边界**): `CodingHistory` 是**变长**字段, 只有它的
    /// **尾随** NUL 不可表示; **内部** NUL 逐字节往返。
    ///
    /// [`Bext::field_that_does_not_round_trip`] 的判定表把这两个形态分开写:
    /// `CodingHistory` 那一格是"**以 NUL 结尾**", 与五个定长字段的"含 NUL"不同 ——
    /// 变长字段的中间字节不是终止符。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把该函数末尾的 `.ends_with('\0')` 注入成 `.contains('\0')`,
    /// 全量 `cargo test -p yeban-render` **全绿**（`test result: ok. 170 passed;
    /// 0 failed`）—— 既有的 6 个"读不回同一个值"的形态里, `CodingHistory` 那一格用的是
    /// **尾随** NUL（两种写法都命中）, 没有一格含**内部** NUL。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一条含内部 NUL 的编码历史（`"A=PCM\0M=stereo,T=Yeban"`, 单位: 字节）。
    /// 读数: [`Bext::field_that_does_not_round_trip`] 的判决、[`Bext::to_bytes`] 的
    /// **长度**、[`Bext::from_bytes`] 读回的字符串与重新编码的逐字节相同。
    ///
    /// # 非空证明
    ///
    /// 探针串同时满足"含内部 NUL"与"不以 NUL 结尾" ⇒ 它落在**可往返**这一侧;
    /// 后半段用同一族的**尾随** NUL 形态钉住另一侧（两半一起, 才不是把整族判宽）。
    #[test]
    fn an_interior_nul_in_the_coding_history_still_round_trips() {
        let probe = "A=PCM\u{0}M=stereo,T=Yeban";
        assert!(
            probe.contains('\u{0}') && !probe.ends_with('\u{0}'),
            "探针串必须含内部 NUL 且不以 NUL 结尾"
        );
        let block = Bext {
            version: 1,
            coding_history: probe.to_owned(),
            ..Bext::default()
        };
        assert_eq!(
            block.field_that_does_not_round_trip(),
            None,
            "内部 NUL 不影响往返"
        );

        let bytes = block.to_bytes();
        assert_eq!(bytes.len(), BEXT_FIXED_LEN + probe.len());
        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert_eq!(decoded.coding_history, probe, "内部 NUL 必须逐字节读回");
        assert_eq!(decoded, block);
        assert_eq!(decoded.to_bytes(), bytes, "重新编码必须逐字节相同");

        // 另一半: **尾随** NUL 仍然不可表示（读取器必须裁掉它, 那是 RIFF 补位的同一形态）。
        let trailing = Bext {
            version: 1,
            coding_history: "A=PCM\u{0}".to_owned(),
            ..Bext::default()
        };
        assert_eq!(
            trailing.field_that_does_not_round_trip(),
            Some("CodingHistory")
        );
    }

    /// 判据 (**类别 1: 非平凡输入 / 字节边界**): 读取器**只**裁掉 `CodingHistory` 的
    /// **尾随** NUL; **前导** NUL 是数据, 必须逐字节读回。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 [`Bext::from_bytes`] 的 `.trim_end_matches('\0')` 注入成
    /// `.trim_matches('\0')`, 全量 `cargo test -p yeban-render` **全绿**
    /// （`test result: ok. 170 passed; 0 failed`）—— 既有的编码历史全部以可打印字符
    /// 开头, 两种写法在那些输入上给出同一个字符串。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一条**前导** NUL 的编码历史（`"\0A=PCM,M=stereo,T=Yeban"`）。
    /// 读数: [`Bext::from_bytes`] 读回的字符串、`Bext` 的整块相等、以及给同一份字节
    /// **追加**一个尾随 NUL 后的读数（尾随那一侧必须被裁掉）。
    ///
    /// # 非空证明
    ///
    /// 探针串以 NUL 开头且**不**以 NUL 结尾; 后半段在同一个串后面追加一个 NUL。
    /// 两半的期望值**相同**, 因此"裁掉尾随"与"裁掉两端"无法同时满足。
    #[test]
    fn a_leading_nul_in_the_coding_history_is_data_not_padding() {
        let probe = "\u{0}A=PCM,M=stereo,T=Yeban";
        assert!(
            probe.starts_with('\u{0}') && !probe.ends_with('\u{0}'),
            "探针串必须以 NUL 开头且不以 NUL 结尾"
        );
        let block = Bext {
            version: 1,
            coding_history: probe.to_owned(),
            ..Bext::default()
        };
        assert_eq!(block.field_that_does_not_round_trip(), None);
        let bytes = block.to_bytes();
        let decoded = Bext::from_bytes(&bytes).expect("解码");
        assert_eq!(
            decoded.coding_history, probe,
            "前导 NUL 是数据, 不得被当成补位裁掉"
        );
        assert_eq!(decoded, block);

        // 尾随 NUL 仍然被裁掉: 同一个串后面追加一个 NUL, 读数必须**不变**。
        let mut with_padding = bytes.clone();
        with_padding.push(0);
        let decoded = Bext::from_bytes(&with_padding).expect("解码");
        assert_eq!(
            decoded.coding_history, probe,
            "尾随 NUL（RIFF 的偶数字节补位）必须被裁掉"
        );
    }

    /// 判据 (**类别 6: 多声道一致性**): `default_channel_mask(8)` 是 7.1 的**八个**声道位
    /// `0xFF`, 而且这个掩码要真的落进 `fmt ` 的 `dwChannelMask` 字段。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把该 match 臂里的 `0x40` 那一位取掉时, 全量 `cargo test -p yeban-render`
    /// **全绿**（`test result: ok. 176 passed; 0 failed`）—— 既有的声道掩码判据只钉了
    /// 5.1 那一档（`default_channel_mask(6) == 0x3F` 与 6 声道 `fmt ` 的偏移 20）,
    /// **8 声道那一档没有任何判据, 也没有一条 8 声道的容器**。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 8 声道 / 48 kHz / 24 位的 [`PcmFormat`]（掩码留 `None`, 走默认分配）。
    /// 单位: 掩码是**声道位集合**（bit 0 = FL … bit 7 = SR 一族）。
    /// 读数: [`default_channel_mask`] 的 `u32` 与写出的 `fmt ` 负载**偏移 20** 的
    /// `dwChannelMask`。
    ///
    /// # 非空证明
    ///
    /// `0xFF != 0` 且它与 5.1 的 `0x3F` **不同** —— 因此本条不是"任何声道数都给同一个
    /// 掩码"的空判据; 后半段同时钉住 5.1 那一档不受影响。
    #[test]
    fn the_seven_point_one_default_mask_has_all_eight_bits() {
        assert_eq!(
            default_channel_mask(8),
            0xFF,
            "7.1 = FL FR FC LFE BL BR SL SR"
        );
        assert_ne!(
            default_channel_mask(8),
            default_channel_mask(6),
            "7.1 与 5.1 不是同一个掩码"
        );
        assert_eq!(default_channel_mask(6), 0x3F, "5.1 那一档不受影响");

        let data = payload(8 * 3 * 2);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            PcmFormat::integer(8, 48_000, 24),
            data.len() as u64,
            2,
            None,
        );
        assert_eq!(plan.validate(), Ok(()));
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("8 声道必须写得出去");
        let parsed = parse_container(&file).expect("写出去的必须读得回来");
        assert_eq!(parsed.format.channel_mask, Some(0xFF), "读回的掩码");
        let fmt = parsed
            .chunks
            .iter()
            .find(|chunk| &chunk.fourcc == b"fmt ")
            .expect("容器必须有 fmt ");
        assert_eq!(
            le::read_u32(&file, fmt.payload_offset + 20),
            Some(0xFF),
            "dwChannelMask 字段必须逐位是 0xFF"
        );
    }

    /// 判据 (**类别 7: 尺寸字段**): RIFF 容器**顶层**的 32 位大小字段必须是
    /// **文件长度 − 8**, 与计划里的 `riffSize` 是同一个读数。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 `header_bytes` 里 RIFF 分支的 `self.sizes.riff_size` 注入成
    /// `self.sizes.data_size` 时, 全量 `cargo test -p yeban-render` **全绿**
    /// （`test result: ok. 176 passed; 0 failed`）—— 既有的 `riff_with_bext_round_trips`
    /// 断言的是 `parsed.sizes.riff_size == file.len() - 8`, 而 RIFF 分支下
    /// `parse_container` 的 `sizes` 是**从实际字节数推导**的, 根本不读那个字段。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个 RIFF + `bext` 的容器。单位: 长度是**字节**。读数: 文件偏移 4 处的
    /// 32 位字段、文件总长度、计划里的 `riff_size` 与读取器推导的 `riff_size`。
    ///
    /// # 非空证明
    ///
    /// `data` 长度（6 字节）与 `riff_size`（654 字节）**不同** —— 因此"写成 data 长度"
    /// 与"写成 riff_size"在这条判据上是两个不同的读数。
    #[test]
    fn the_riff_top_level_size_field_is_the_file_length_minus_eight() {
        let data = payload(6);
        let block = Bext {
            version: 1,
            coding_history: "CH".to_owned(),
            ..Bext::default()
        };
        let plan = ContainerPlan::for_payload(
            ContainerKind::Riff,
            stereo_16bit(),
            data.len() as u64,
            2,
            Some(block),
        );
        assert_eq!(plan.kind, ContainerKind::Riff);
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        let field = u64::from(le::read_u32(&file, 4).expect("顶层大小字段"));
        assert_eq!(
            field,
            file.len() as u64 - 8,
            "RIFF 顶层大小字段是文件长度 − 8 (字节)"
        );
        assert_ne!(field, plan.sizes.data_size, "它不是 data 的长度");
        assert_eq!(field, plan.sizes.riff_size, "它必须等于计划里的 riffSize");
        assert_eq!(
            parse_container(&file).expect("解析").sizes.riff_size,
            field,
            "读取器推导的 riffSize 必须与字段一致"
        );
    }

    /// 判据 (**类别 7: 尺寸下界**): 一个**短于** 602 字节固定前缀的 `bext` chunk 必须
    /// 返回 `Err(Truncated)`, 而不是让读取器在定长字段上越界 panic。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 [`Bext::from_bytes`] 开头的 `if bytes.len() < BEXT_FIXED_LEN` 改成恒假的
    /// 比较, 全量 `cargo test -p yeban-render` **全绿**（`test result: ok. 170 passed;
    /// 0 failed`）—— 既有的畸形输入判据里, `bext` chunk 要么声明长度足以容纳固定前缀,
    /// 要么被 chunk 循环的 `bytes.get(payload_offset..payload_end)` 先裁到文件边界,
    /// 因此"**负载本身**短于前缀"这一格没有落点。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 手搭的 `RIFF` 容器 + 一个**声明并真的只有 100 字节**的 `bext` chunk
    /// （单位: 字节）。读数: [`parse_container`] 的判决
    /// （`Truncated { what: "bext 固定前缀 (需要 602 字节)", got: 100 }`）。
    ///
    /// # 非空证明
    ///
    /// `100 < 602`; 后半段把同一个 chunk 换成一个合法块（`602 + 2` 字节）后必须解析成功,
    /// 因此上面那条不是"这个容器本来就坏"。
    #[test]
    fn a_bext_payload_shorter_than_the_fixed_prefix_is_a_clean_error() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &stereo_16bit().fmt_payload());
        push_chunk(&mut raw, b"bext", &[0u8; 100]);
        push_chunk(&mut raw, b"data", &payload(2));
        assert_eq!(
            parse_container(&raw),
            Err(Rf64Error::Truncated {
                what: "bext 固定前缀 (需要 602 字节)",
                got: 100,
            }),
            "短于 602 的 bext 负载必须是 Err, 不是 panic"
        );

        // 非空对照: 同一个容器把 bext 换成一个合法块之后必须解析成功。
        let good = Bext {
            version: 1,
            coding_history: "CH".to_owned(),
            ..Bext::default()
        };
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&0u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        push_chunk(&mut raw, b"fmt ", &stereo_16bit().fmt_payload());
        push_chunk(&mut raw, b"bext", &good.to_bytes());
        push_chunk(&mut raw, b"data", &payload(2));
        let parsed = parse_container(&raw).expect("合法 bext 必须能解析");
        assert_eq!(parsed.bext.expect("有 bext").coding_history, "CH");
    }

    /// 判据 (**类别 1/4: 字段边界与零值回退**): `WAVE_FORMAT_EXTENSIBLE` 的
    /// `wValidBitsPerSample`（偏移 18）取 0 时, 读取器必须回退到 `fmt ` 里声明的
    /// **容器位深**（偏移 14）。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// `parse_fmt_payload` 用
    /// `le::read_u16(payload, 18).filter(|&bits| bits > 0).unwrap_or(bits_per_sample)`
    /// 决定读回来的 `bits_per_sample`。本机把那个谓词整支删掉（等价于直接取偏移 18 的值）
    /// —— 全量 `cargo test -p yeban-render --no-default-features --lib --tests`
    /// **全绿**（`test result: ok. 179 passed; 0 failed` + 10 + 13）。
    ///
    /// 原因是既有的 EXTENSIBLE 判据里偏移 18 与偏移 14 **恒等**: 写入器把两者写成同一个
    /// 值（见 `PcmFormat::fmt_payload`）, 而 `extensible_container_round_trips` 断言的是
    /// "读回的结构体等于输入的结构体" —— 两者同时是 32, 分不出读取器读的是哪一格。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个由本模块写入器产出的 `RF64` + `WAVE_FORMAT_EXTENSIBLE` 头部
    /// （单位: 字节）, 其 `fmt ` 负载的偏移 18 被改成 0。读数: [`parse_container`] 的
    /// `format.bits_per_sample`（单位: 位）。
    ///
    /// # 非空证明
    ///
    /// 偏移 14 与偏移 18 在写入时相等（都是 32）, 因此本判据**先**断言这两格确实相等,
    /// **再**把偏移 18 改成 0, 最后断言读回的仍是偏移 14 的那一个值。没有前半段,
    /// "读到 32" 就可能只是"文件里本来就是 32"。
    ///
    /// # 运算类别（ADR-0001 的 D32）
    ///
    /// 只用到整数读写与切片比较 —— IEEE 精确类, 不含超越函数, 因此可以逐位断言。
    #[test]
    fn a_zero_valid_bits_field_falls_back_to_the_container_depth() {
        let format = PcmFormat {
            channel_mask: Some(0x3F),
            ..PcmFormat::float(6, 48_000, 32)
        };
        let data = vec![0u8; 6 * 4 * 4];
        let plan =
            ContainerPlan::for_payload(ContainerKind::Rf64, format, data.len() as u64, 4, None);
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");

        // `fmt ` 在 RF64 下紧跟 36 字节的 ds64: 12 + 36 = 48 是它的 chunk 头。
        let fmt_at = 12 + 36;
        assert_eq!(&file[fmt_at..fmt_at + 4], b"fmt ");
        let body = fmt_at + 8;
        assert_eq!(le::read_u16(&file, body), Some(0xFFFE), "EXTENSIBLE 标签");
        assert_eq!(le::read_u16(&file, body + 14), Some(32), "容器位深");
        assert_eq!(
            le::read_u16(&file, body + 18),
            Some(32),
            "写入器把 validBits 与容器位深写成同一个值 (本判据的前提)"
        );

        file[body + 18..body + 20].copy_from_slice(&0u16.to_le_bytes());
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(
            parsed.format.bits_per_sample, 32,
            "validBits = 0 必须回退到 fmt 里声明的容器位深(32), 不是 0"
        );
        assert_eq!(parsed.format.channels, 6);
        assert_eq!(parsed.format.sample_rate, 48_000);
        assert!(parsed.format.is_float);
        assert_eq!(parsed.format.channel_mask, Some(0x3F));
    }

    /// 判据 (**类别 6: 多声道一致性**): [`default_channel_mask`] 对**每一个**列出的
    /// 声道数给出的都是 libsndfile 那一档掩码 —— `1` / `2` / `4` 三格此前没有判据。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `1 => 0x4` 改成 `1 => 0x1`（FC → FL）后, 全量
    /// `cargo test -p yeban-render --no-default-features --lib --tests --no-fail-fast`
    /// **全绿**（`190 passed; 0 failed` + 10 + 17）。同样的注入对
    /// `2 => 0x1 | 0x2` 与 `4 => 0x1 | 0x2 | 0x10 | 0x20` 各一次, 也都是全绿。
    /// 既有的三处掩码断言只覆盖 `default_channel_mask(6) == 0x3F`、
    /// `default_channel_mask(8) == 0xFF` 与 `default_channel_mask(3) == 0`。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: [`default_channel_mask`] 的输入（单位: 声道数）与输出
    /// （单位: 一个 `u32` 声道位掩码）。读数: 10 个 `u32`, 加上"掩码真的落到
    /// `fmt ` 偏移 20 并被读回"的字节读数（单位: 字节偏移）。
    ///
    /// # 非空证明
    ///
    /// 五格列出的掩码**互不相同**（`0x4` / `0x3` / `0x33` / `0x3F` / `0xFF`）, 与回退那一档
    /// 的 `0` 也不同 ⇒ 本条不是"所有输入都给同一个值"的退化判据（下面直接断言两对不等）。
    #[test]
    fn every_documented_default_channel_mask_is_pinned() {
        let documented = [
            (1u16, 0x4u32),                                         // FC
            (2, 0x1 | 0x2),                                         // FL FR
            (4, 0x1 | 0x2 | 0x10 | 0x20),                           // FL FR BL BR
            (6, 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20),               // 5.1
            (8, 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x80), // 7.1
        ];
        for (channels, expected) in documented {
            assert_eq!(
                default_channel_mask(channels),
                expected,
                "{channels} 声道的默认掩码（一个 u32 声道位掩码）"
            );
        }
        assert_ne!(default_channel_mask(1), default_channel_mask(2));
        assert_ne!(default_channel_mask(2), default_channel_mask(4));
        // 未列出的声道数回退为 0（direct out）—— 这一档与上面五格都不同。
        for unlisted in [0u16, 3, 5, 7, 9] {
            assert_eq!(
                default_channel_mask(unlisted),
                0,
                "{unlisted} 声道必须回退为 0"
            );
        }

        // 掩码真的落到 `fmt ` 的偏移 20, 并且能被本 crate 的读取器读回。
        let format = PcmFormat::integer(4, 48_000, 16);
        let body = format.fmt_payload();
        assert_eq!(body.len(), 40, "4 声道必须走 EXTENSIBLE（字节）");
        assert_eq!(
            le::read_u32(&body, 20),
            Some(default_channel_mask(4)),
            "fmt 偏移 20 的 dwChannelMask"
        );
        let data = vec![0u8; 8];
        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, format, data.len() as u64, 1, None);
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(parsed.format.channel_mask, Some(default_channel_mask(4)));
    }

    /// 判据 (**类别 4: 参数极值**): [`PcmFormat::byte_rate`] 与
    /// [`PcmFormat::block_align`] 一样是**饱和**乘法 —— 采样率顶到 `u32::MAX` 时
    /// `nAvgBytesPerSec` 饱和, **不得**在 debug 下 panic。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `self.sample_rate.saturating_mul(self.block_align() as u32)` 换回普通的 `*`
    /// 之后, 全量判据**全绿**。既有的每一处 `byte_rate()` 断言（`288_000`、`1_152_000`、
    /// `864_000`、`48000 × 24`）的采样率都是 48 kHz, 乘积远在 `u32` 之内。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: `sample_rate = u32::MAX`、8 声道、32 位的 [`PcmFormat`]。
    /// 读数: `block_align()`（字节/帧）、`byte_rate()`（字节/秒, 期望饱和到 `u32::MAX`）、
    /// `fmt_payload()` 的长度（字节）与它偏移 8 的 `nAvgBytesPerSec` 字段, 以及
    /// [`ContainerPlan::validate`] 的判决。
    ///
    /// # 非空证明
    ///
    /// 真实乘积 `u32::MAX × 32` 在 `u64` 口径下**超过 `u32::MAX`**（下面直接断言）,
    /// 而 `block_align` 本身是 `32`（不是 0）—— 因此这一格既不是"0 × 0"也不是"没溢出"。
    #[test]
    fn a_byte_rate_that_overflows_saturates_instead_of_panicking() {
        let format = PcmFormat::integer(8, u32::MAX, 32);
        assert_eq!(format.block_align(), 32, "8 声道 × 4 字节（字节/帧）");
        assert!(
            u64::from(format.block_align()) * u64::from(u32::MAX) > u64::from(u32::MAX),
            "这一格的真实乘积必须超过 u32, 否则本判据测不到溢出"
        );
        assert_eq!(
            format.byte_rate(),
            u32::MAX,
            "nAvgBytesPerSec 必须饱和到 u32::MAX, 不得回绕也不得 panic"
        );

        let body = format.fmt_payload();
        assert_eq!(body.len(), 40, "8 声道必须走 EXTENSIBLE（字节）");
        assert_eq!(
            le::read_u32(&body, 8),
            Some(u32::MAX),
            "fmt 偏移 8 的 nAvgBytesPerSec"
        );
        // 头部算术也要对这一个格式是全函数（`fmt_payload` 在里面被调用）。
        let plan = ContainerPlan::for_payload(ContainerKind::Riff, format, 0, 0, None);
        assert_eq!(plan.validate(), Ok(()), "饱和的 byte_rate 不该让计划不可写");
        assert!(!plan.header_bytes().is_empty(), "头部仍然必须写得出来");
    }

    /// 判据 (**类别 6: 多声道一致性**): `channel_mask` 只要**显式给出**, 双声道也必须走
    /// 40 字节 `WAVE_FORMAT_EXTENSIBLE` —— [`PcmFormat::is_extensible`] 的两个析取项
    /// 都要有判据。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `self.channels > 2 || self.channel_mask.is_some()` 削成 `self.channels > 2` 之后,
    /// 全量判据**全绿**。既有的 EXTENSIBLE 夹具（6 声道、掩码 `0x3F`）同时满足两个析取项,
    /// 因此分不出"只看声道数"与"两个都看"。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 双声道 + 显式掩码 `0x4` 的 [`PcmFormat`]（单位: 声道数 / 位掩码）。
    /// 读数: `is_extensible()`（布尔）、`format_tag()`（规范标签）、`fmt_payload()` 的
    /// 长度（字节）与它偏移 16 / 18 / 20 的三个字段, 以及往返读回的 `PcmFormat`。
    ///
    /// # 非空证明
    ///
    /// 后半段用**同一个格式去掉掩码**作对照: 它必须**不是** EXTENSIBLE（16 字节 `fmt `,
    /// 标签 `0x0001`）。少了这一半, "一律 EXTENSIBLE"也会让前半段全绿。
    #[test]
    fn an_explicit_channel_mask_forces_extensible_on_a_stereo_format() {
        let masked = PcmFormat {
            channel_mask: Some(0x4),
            ..PcmFormat::integer(2, 48_000, 16)
        };
        assert!(masked.is_extensible(), "显式掩码必须触发 EXTENSIBLE");
        assert_eq!(masked.format_tag(), 0xFFFE);
        let body = masked.fmt_payload();
        assert_eq!(body.len(), 40, "EXTENSIBLE 负载是 40 字节");
        assert_eq!(le::read_u16(&body, 0), Some(0xFFFE), "格式标签");
        assert_eq!(le::read_u16(&body, 16), Some(22), "cbSize");
        assert_eq!(le::read_u16(&body, 18), Some(16), "wValidBitsPerSample");
        assert_eq!(le::read_u32(&body, 20), Some(0x4), "dwChannelMask");

        let data = vec![0u8; 8];
        let plan =
            ContainerPlan::for_payload(ContainerKind::Riff, masked, data.len() as u64, 2, None);
        let mut file = Vec::new();
        write_container(&mut file, &plan, &data).expect("写入");
        let parsed = parse_container(&file).expect("解析");
        assert_eq!(parsed.format, masked, "掩码必须逐字段往返");

        // 对照: 同一个格式**不**给掩码 ⇒ 16 字节的普通 PCM。
        let plain = PcmFormat::integer(2, 48_000, 16);
        assert!(!plain.is_extensible());
        assert_eq!(plain.format_tag(), 0x0001);
        assert_eq!(plain.fmt_payload().len(), 16);
    }

    /// 判据 (**公开 `Default` 的字段普查**): [`Bext::default`] 的 **10 个字段**逐个钉死。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `origination_date: "1970-01-01"` 改成 `"1970-01-02"`、或把 `umid: [0u8; 64]`
    /// 改成 `[1u8; 64]` 之后, 全量判据**全绿** —— 既有的 `Bext` 夹具要么**显式覆盖**
    /// 这些字段（构造字面量写了它们）, 要么只做**往返**比较（读回来的就是写出去的,
    /// 改了默认值两边同时变, 因此自洽）。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: [`Bext::default`] 的返回值（单位: 一个 `bext` 块）。读数: 5 个字符串字段
    /// （字符）、`time_reference`（采样数）、`version`（版本号）、`umid`（64 字节）、
    /// `loudness`（`Option`）、`coding_history`（字符）, 加上编码长度（字节）。
    ///
    /// # 非空证明
    ///
    /// 10 个字段里有 3 格是**非空字面量**（`"1970-01-01"` / `"00:00:00"` / `version = 1`）,
    /// 与另外 7 格的"空/零"可区分; 而且那两个日期字段的长度是 `bext` 定长字段的宽度
    /// （10 / 8 字节）—— 一句 `assert_eq!` 同时钉住"内容"与"字段宽"。
    #[test]
    fn the_public_bext_default_is_pinned_field_by_field() {
        let default = Bext::default();
        assert_eq!(default.description, "");
        assert_eq!(default.originator, "");
        assert_eq!(default.originator_reference, "");
        assert_eq!(default.origination_date, "1970-01-01");
        assert_eq!(default.origination_time, "00:00:00");
        assert_eq!(default.time_reference, 0);
        assert_eq!(default.version, 1, "默认版本是 1（版本 2 必须有响度块）");
        assert_eq!(default.umid, [0u8; 64]);
        assert_eq!(default.loudness, None);
        assert_eq!(default.coding_history, "");

        // 定长字段的宽度: 一句断言同时钉住内容与字段宽。
        assert_eq!(
            default.origination_date.len(),
            10,
            "OriginationDate 宽 10 字节"
        );
        assert_eq!(
            default.origination_time.len(),
            8,
            "OriginationTime 宽 8 字节"
        );
        // 没有编码历史 ⇒ 编码长度恰好是固定前缀。
        assert_eq!(default.encoded_len(), BEXT_FIXED_LEN);
        assert_eq!(default.to_bytes().len(), BEXT_FIXED_LEN);
    }

    /// 判据 (**未实现的 chunk 族**): 读取器对**不认识的** chunk 是**跳过**, 不是报错,
    /// 而且跳过之后后面的 chunk 仍要正确解析（含奇数负载的补位字节）。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `parse_container` 的 `_ => {}` 换成 `_ => return Err(Rf64Error::NotWaveContainer)`
    /// 之后, 全量判据**全绿** —— 既有的每一个夹具只写 `ds64`/`fmt `/`fact`/`bext`/`data`,
    /// **没有一格带未知 chunk**。而模块头的"已知边界"写明 BW64 的
    /// `axml`/`bxml`/`sxml`/`chna` 未实现 ⇒ 真实 BW64 交付物**必然**带这些 chunk。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个 BW64 容器, 在 `fmt ` 与 `data` 之间插入两个未实现的 chunk:
    /// `axml`（18 字节负载）与 `JUNK`（**3** 字节负载 ⇒ 需要 1 字节补位）。
    /// 读数: `parse_container` 的判决、`chunk_order` 的 fourcc 序列（4 字节 × N）、
    /// `chunk_lengths` 的两个负载长度（字节）与 `chunks.len()`。
    ///
    /// # 非空证明
    ///
    /// `axml` 与 `JUNK` 的**负载长度不同**（18 与 3）且都非零, 其中 `JUNK` 是奇数
    /// ⇒ "补位没跳过"会让后面的 `data` 解析失败; 而 `chunks.len()` 必须恰好 +2。
    #[test]
    fn an_unimplemented_chunk_is_skipped_not_rejected() {
        let data = payload(4);
        let plan = ContainerPlan::for_payload(
            ContainerKind::Bw64,
            stereo_16bit(),
            data.len() as u64,
            4,
            None,
        );
        let mut file = plan.header_bytes();
        file.extend_from_slice(&data);
        let before = chunk_order(&file).expect("原始容器可解析").len();
        assert_eq!(before, 3, "BW64 无 bext 时是 ds64 / fmt / data");

        // 插在 `data` 的 chunk 头**之前**。
        let data_header_at = file.len() - data.len() - 8;
        let mut inserted = Vec::new();
        let axml = b"<ebucore:coreMetadata/>";
        push_chunk(&mut inserted, b"axml", axml);
        push_chunk(&mut inserted, b"JUNK", &[0xAB; 3]);
        file.splice(data_header_at..data_header_at, inserted);

        let parsed = parse_container(&file).expect("未实现的 chunk 不得让解析失败");
        assert_eq!(
            &file[parsed.data.clone()],
            data.as_slice(),
            "data 负载仍要读对"
        );
        assert_eq!(parsed.format, stereo_16bit());
        assert_eq!(parsed.sizes.sample_count, 4);
        assert_eq!(
            chunk_order(&file).expect("解析"),
            vec![*b"ds64", *b"fmt ", *b"axml", *b"JUNK", *b"data"],
            "未实现的 chunk 必须出现在 chunk 序列里, 而且顺序不变"
        );
        assert_eq!(parsed.chunks.len(), before + 2, "恰好多了两个 chunk");
        let lengths = chunk_lengths(&parsed.chunks);
        assert_eq!(lengths.get("axml"), Some(&axml.len()), "axml 负载长度");
        assert_eq!(lengths.get("JUNK"), Some(&3usize), "JUNK 负载长度（奇数）");
        // 非空证明: `JUNK` 的负载是奇数 ⇒ 少了补位 `data` 就会错位。
        assert_eq!(3 % 2, 1, "JUNK 必须是奇数负载");
    }

    /// 判据 (**未实现的 chunk 族, 表驱动**): 8 个未实现的 chunk 名 × 4 个负载长度
    /// （0 / 1 / 2 / 3 字节）× **连续三个**同一 chunk, 每一个形状都必须被跳过,
    /// 而且跳过之后 `data` 仍逐字节读对。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 上一条判据只用了两个 chunk（`axml` 18 字节 / `JUNK` 3 字节）**各一个**。
    /// 把"已出现过的 fourcc 不再入表"（去重）注入进去之后, 上一条判据仍然全绿 ——
    /// 它的两个 fourcc 不同; 只有**连续三个同名** chunk 才会让去重后的 `chunks.len()`
    /// 对不上。此外本表覆盖了**带空格**的 fourcc（`cue ` / `r64m`）与**零长度**负载
    /// （`0 % 2 == 0` ⇒ 不能无条件补位）。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 8 个 fourcc × 4 个负载长度（单位: 字节）× 连续 3 个 chunk。读数:
    /// `parse_container` 的判决、`data` 区间的内容、`chunks` 的条数（3 + 3 个已知 chunk）
    /// 与每个 fourcc 的负载长度。
    ///
    /// # 非空证明
    ///
    /// 4 个长度里既有偶数（0 / 2）也有奇数（1 / 3）⇒ "无条件补位"与"从不补位"两种改法
    /// 都会在这张表上红; 8 个名字里既有带空格的也有不带的, 且**连续三个同名**
    /// ⇒ "去重"与"前缀匹配"两种改法也会红。
    #[test]
    fn every_unimplemented_chunk_shape_is_skipped() {
        let data = payload(4);
        let names: [[u8; 4]; 8] = [
            *b"axml", *b"bxml", *b"sxml", *b"chna", *b"iXML", *b"cue ", *b"r64m", *b"JUNK",
        ];
        let mut shapes = 0usize;
        for fourcc in names {
            for len in [0usize, 1, 2, 3] {
                let plan = ContainerPlan::for_payload(
                    ContainerKind::Bw64,
                    stereo_16bit(),
                    data.len() as u64,
                    4,
                    None,
                );
                let mut file = plan.header_bytes();
                file.extend_from_slice(&data);
                let before = chunk_order(&file).expect("原始容器可解析").len();
                let data_header_at = file.len() - data.len() - 8;
                let mut inserted = Vec::new();
                for _ in 0..3 {
                    push_chunk(&mut inserted, &fourcc, &vec![0x5A; len]);
                }
                file.splice(data_header_at..data_header_at, inserted);

                let label = format!(
                    "{} 连续 3 个 × {len} 字节",
                    String::from_utf8_lossy(&fourcc)
                );
                let parsed = parse_container(&file)
                    .unwrap_or_else(|error| panic!("{label}: 未实现的 chunk 让解析失败: {error}"));
                assert_eq!(
                    &file[parsed.data.clone()],
                    data.as_slice(),
                    "{label}: data 负载"
                );
                assert_eq!(parsed.format, stereo_16bit(), "{label}: fmt 必须读对");
                assert_eq!(
                    parsed.chunks.len(),
                    before + 3,
                    "{label}: 连续三个同名 chunk 不得被合并"
                );
                let order = chunk_order(&file).expect("解析");
                assert_eq!(
                    &order[before - 1..],
                    &[fourcc, fourcc, fourcc, *b"data"],
                    "{label}: 三个未知 chunk 必须按原顺序出现, 后面紧跟 data"
                );
                let lengths = chunk_lengths(&parsed.chunks);
                assert_eq!(
                    lengths.get(&String::from_utf8_lossy(&fourcc).into_owned()),
                    Some(&len),
                    "{label}: 负载长度"
                );
                shapes += 1;
            }
        }
        assert_eq!(shapes, 32, "8 个名字 × 4 个长度");
        // 非空证明: 长度里既有偶数也有奇数; 名字里既有带空格的也有不带的。
        assert_eq!(
            [0usize, 1, 2, 3]
                .iter()
                .filter(|len| **len % 2 == 1)
                .count(),
            2
        );
        assert_eq!(names.iter().filter(|name| name[3] == b' ').count(), 1);
    }
}
