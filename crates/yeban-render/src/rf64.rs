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
//! - 只实现 `bext` 版本 1 与 2 的读写; 1997 年的 v0 布局未核验, 读到即返回
//!   [`Rf64Error::UnsupportedBextVersion`], 登记为 `pending`。
//! - BW64 的 `axml`/`bxml`/`sxml`/`chna` 四个 XML chunk 未实现（[ARCH-FMT-001]
//!   只要求 RF64/BW64 容器 + `bext`）, `ContainerKind::Bw64` 产出的是
//!   "BW64 标识 + `ds64` + `fmt ` + `bext`"这一子集, 不是完整 BS.2088 文件。

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::ops::Range;

/// RF64 的"真实长度在 ds64 里"哨兵值。
pub const SENTINEL_U32: u32 = 0xFFFF_FFFF;

/// `bext` v1/v2 的固定前缀长度（字节）。
pub const BEXT_FIXED_LEN: usize = 602;

/// `bext` v2 在固定前缀里额外的响度字段长度（5 × `i16`）。
pub const BEXT_V2_LOUDNESS_LEN: usize = 10;

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
    #[must_use]
    pub const fn block_align(&self) -> u16 {
        self.channels * self.bytes_per_sample()
    }

    /// 每秒字节数（`nAvgBytesPerSec`）。
    #[must_use]
    pub const fn byte_rate(&self) -> u32 {
        self.sample_rate * (self.block_align() as u32)
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
/// 文字字段是定长 NUL 填充的字节串（规范未规定字符集; 这里按 ASCII/UTF-8 写入并
/// 截断到字段长度, 超出部分**丢弃而不是 panic**, 与 FFmpeg/libndsfile 的行为一致）。
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

    /// 编码为 `bext` chunk 的负载（固定前缀 + coding history）。
    ///
    /// # Panics
    ///
    /// `version == 0` 或 `version >= 2 && loudness.is_none()`, 以及
    /// `version == 1 && loudness.is_some()` 时 panic —— 这些是**调用方的构造错误**,
    /// 静默写一个自相矛盾的 chunk 会让下游解析器读到垃圾。
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        assert!(
            self.version >= 1,
            "bext 版本 0 的字段表未核验, 拒绝写入 (见 render-master-notes needs)"
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

        let mut out = Vec::with_capacity(self.fixed_len() + self.coding_history.len());
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
        debug_assert_eq!(out.len(), self.fixed_len() + self.coding_history.len());
        out
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

/// 把 `value` 写进 `width` 字节的定长字段, 超出即截断, 其余补 0。
///
/// 按**字节**截断而不是按字符: 规范字段是字节串, 按字符截断可能把多字节 UTF-8
/// 从中间切断, 写出的就不再是合法文本。
fn push_fixed(out: &mut Vec<u8>, value: &str, width: usize) {
    let raw = value.as_bytes();
    let take = raw.len().min(width);
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
        let bext_total = bext
            .as_ref()
            .map_or(0, |block| chunk_total(block.to_bytes().len()));
        let header_no_ds64 = 12 + fmt_total + bext_total + 8;
        // RIFF 的长度字段包含 chunk 之间的偶数补位字节, 因此 data 负载的补位也要算进去。
        let payload_total = payload_len + (payload_len % 2);
        let projected = header_no_ds64 as u64 + payload_total;
        let kind = if projected > u64::from(SENTINEL_U32) && !preferred.uses_ds64() {
            ContainerKind::Rf64
        } else {
            preferred
        };
        let header_len = header_no_ds64 + if kind.uses_ds64() { 36 } else { 0 };
        let riff_size = header_len as u64 + payload_total - 8;
        debug_assert_eq!(
            Self {
                kind,
                format,
                sizes: Rf64Sizes {
                    riff_size,
                    data_size: payload_len,
                    sample_count: frame_count
                },
                bext: bext.clone(),
            }
            .header_bytes()
            .len(),
            header_len,
            "头部长度计算必须与实际写出的字节数一致"
        );
        Self {
            kind,
            format,
            sizes: Rf64Sizes {
                riff_size,
                data_size: payload_len,
                sample_count: frame_count,
            },
            bext,
        }
    }

    /// 构造头部字节（`data` chunk 头之后、音频负载之前的一切）。
    ///
    /// 这是个**纯函数** —— 大尺寸场景的全部判据都打在它身上, 因此不需要真的写
    /// 4 GiB。
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
    /// `bext` 版本不受支持（本实现只支持 1 与 2）。
    UnsupportedBextVersion(u16),
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
            Self::UnsupportedBextVersion(version) => {
                write!(f, "不受支持的 bext 版本: {version}")
            }
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
/// # Errors
///
/// 底层写入失败。
pub fn write_container<W: Write>(
    out: &mut W,
    plan: &ContainerPlan,
    payload: &[u8],
) -> Result<(), Rf64Error> {
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

    let mut chunks = Vec::new();
    let mut ds64: Option<Rf64Sizes> = None;
    let mut format: Option<PcmFormat> = None;
    let mut bext: Option<Bext> = None;
    let mut data: Option<(usize, usize)> = None;

    let mut cursor = 12usize;
    while cursor + 8 <= bytes.len() {
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
                    .get(payload_offset..payload_offset + effective)
                    .ok_or(Rf64Error::Truncated {
                        what: "ds64 负载",
                        got: bytes.len(),
                    })?;
                ds64 = Rf64Sizes::from_bytes(payload);
            }
            b"fmt " => {
                let payload = bytes
                    .get(payload_offset..payload_offset + effective)
                    .ok_or(Rf64Error::Truncated {
                        what: "fmt 负载",
                        got: bytes.len(),
                    })?;
                format = Some(parse_fmt_payload(payload, declared)?);
            }
            b"bext" => {
                let payload = bytes
                    .get(payload_offset..payload_offset + effective)
                    .ok_or(Rf64Error::Truncated {
                        what: "bext 负载",
                        got: bytes.len(),
                    })?;
                bext = Some(Bext::from_bytes(payload)?);
            }
            b"data" => {
                if payload_offset + effective > bytes.len() {
                    return Err(Rf64Error::Truncated {
                        what: "data 负载",
                        got: bytes.len(),
                    });
                }
                data = Some((payload_offset, payload_offset + effective));
            }
            _ => {}
        }

        // 前进: chunk 头 8 字节 + 负载 + 偶数补位。
        cursor = payload_offset + effective + (effective % 2);
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
    Ok(PcmFormat {
        channels,
        sample_rate,
        bits_per_sample: valid_bits,
        is_float,
        channel_mask,
    })
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
}
