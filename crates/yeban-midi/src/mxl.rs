//! `.mxl`（**压缩** MusicXML，OPC/ZIP 容器）的**只读**导入：零新依赖。
//!
//! ## 这一票是什么
//!
//! 上一票（`a29d280`）把 `.mxl` 的**拒绝与代价**钉成了字面判据，并登记了四条路线；
//! 集成者裁决 R1 把"`.mxl`（inflate）"判为**另立票**
//! （`docs/ledger/integration-rulings-notes.md:37`）。**本模块就是那一票**。
//!
//! 路线 **B**（手写 raw-DEFLATE inflate，零依赖）被采用。⛔ 没走路线 A（`flate2`）的原因
//! **不是**偏好，而是本票的改动范围：`flate2` 虽已在根清单登记（根 `Cargo.toml:79`），
//! 但给本 crate 加上这条依赖边会**同时**要求改 `Cargo.lock`（根级文件）并让
//! `scripts/gates/license_inventory.py --check` 变红（该守卫读
//! `docs/ledger/dependency-licenses.md` 的"直接依赖"列）。两者都在本票的允许改动范围之外，
//! 且后者是 `docs/**`。**本机实测**（2026-10-08，临时改回后已还原）：
//! `cargo metadata --locked` ⇒ rc **101**（`cannot update the lock file … because --locked was passed`）；
//! `python3 -B scripts/gates/license_inventory.py --check` ⇒ rc **2**。
//!
//! ## 做什么（白名单）
//!
//! 输入 `.mxl` 字节，输出与 [`parse_musicxml`](crate::musicxml::parse_musicxml) **同一个**
//! [`MusicXmlScore`](crate::musicxml::MusicXmlScore)：
//!
//! | 步骤 | 行为 |
//! | :--- | :--- |
//! | End of Central Directory（EOCD） | 从文件尾向前找签名 `PK\x05\x06`，取中央目录的偏移 / 尺寸 / 条目数 |
//! | 中央目录 | 逐条读签名 `PK\x01\x02` 的固定 46 字节 + 条目名（尺寸与 CRC 以**中央目录**为准） |
//! | 根文件定位 | 解压 `META-INF/container.xml`，取**第一个** `<rootfile full-path="…">` 的属性值 |
//! | 条目解码 | 压缩法 **0**（stored）原样取；**8**（deflate）走 `inflate::inflate_raw`；其余明确 `Err` |
//! | 完整性 | 膨胀后长度必须等于声明的未压缩长度；CRC-32（IEEE 802.3）必须等于条目的 CRC 字段 |
//! | 载荷 | 直接喂 [`parse_musicxml`](crate::musicxml::parse_musicxml)（⛔ 不复制第二份 XML 解析器） |
//!
//! ZIP 的字段布局出自 APPNOTE（PKWARE `.ZIP` File Format Specification）4.3.x 的
//! local file header / central directory 两节 —— 那是**外部**规范，⛔ 本仓库不含其正文。
//! raw DEFLATE 出自 RFC 1951，同样是外部规范，见 `src/mxl/inflate.rs` 的模块文档。
//!
//! ## 未实现清单（⛔ 不许把这些读成"已支持"）
//!
//! 1. ⛔ **ZIP64**：条目数 `0xFFFF` 或尺寸/偏移 `0xFFFFFFFF` ⇒ 明确
//!    [`MxlError::UnsupportedZip64`](crate::mxl::MxlError::UnsupportedZip64)。因此 **>4 GiB 的容器、>65535 个条目的容器读不了**。
//!    "尺寸/偏移 `0xFFFFFFFF`" 有**五处**，全部点名（APPNOTE 4.4.8 / 4.4.9 / 4.4.16 与
//!    4.5.3 的 ZIP64 扩展信息 extra field `0x0001`）：EOCD 的中央目录**尺寸**与**偏移**
//!    两条（APPNOTE 4.3.16），以及**每个条目**的压缩长度 / 未压缩长度 / 本地头偏移三条。
//!    ⚠️ 最后三条是**本票补的**：在补之前，同一个 ZIP64 条目按调用方给的
//!    `MxlLimits::max_entry_bytes` 报出**三种不同**的读数（默认上界 ⇒ `LimitExceeded`；
//!    上界放到 `usize::MAX` ⇒ `SizeMismatch`；压缩长度为标记 ⇒ `Malformed`）
//!    ⇒ 把"格式不支持"说成了"调用方的上界太小"。判据
//!    `mxl_zip64_markers_are_named_not_blamed_on_the_limit` 钉住五个标记
//!    在**两种**上界下给出**同一个** `UnsupportedZip64`。
//! 2. ⛔ **加密**：general purpose flag 的 bit 0 ⇒ 明确 [`MxlError::Encrypted`](crate::mxl::MxlError::Encrypted)。
//! 3. ⛔ **其它压缩法**：只认 0 与 8（deflate）。`bzip2`(12) / `lzma`(14) / `zstd`(93) 等明确
//!    [`MxlError::UnsupportedCompression`](crate::mxl::MxlError::UnsupportedCompression)。
//! 4. ⛔ **写出**：本模块只读。`.mxl` 打包不存在。
//! 5. ⛔ **不做 XML 解析器**：`META-INF/container.xml` 只用一个小扫描器取 `<rootfile>`
//!    标签的 `full-path` 属性；属性值里只解 5 个预定义实体（`&amp;` `&lt;` `&gt;`
//!    `&quot;` `&apos;`），**数字字符引用原样保留**（⇒ 路径对不上条目名时是明确的
//!    [`MxlError::MissingRootFile`](crate::mxl::MxlError::MissingRootFile)，⛔ 不会静默读错文件）。
//!    ⚠️ 那条"⛔ 不会静默读错文件"原来**只靠实体那一半**成立：属性扫描器不按引号走 ⇒
//!    另一个属性的**值**里出现 `full-path='other.xml'` 时，容器里**存在的** `other.xml`
//!    会被静默当成根文件（**同一份字节**，本模块与符合规范的读取器读出**不同**的乐谱）。
//!    本票把扫描器改成按 XML 的引号规则走（`"…"` / `'…'` 之间不作属性名起点），
//!    判据 `mxl_rootfile_attribute_is_not_read_from_another_attribute_value` 钉住两条方向：
//!    值里那段伪属性指向的条目**存在** ⇒ 仍然读**真**属性指向的那一份；
//!    标签里**真的**没有 `full-path`（只有值里那段伪属性）⇒ 明确的
//!    [`MxlError::NoRootFile`](crate::mxl::MxlError::NoRootFile)
//!    （修之前，前一条报 `MissingRootFile { path: "other.xml" }`、后一条报
//!    `MissingRootFile { path: "absent.xml" }` —— 两条都是"值被当成属性"的读数）。
//! 6. ⛔ **无 `META-INF/container.xml` 时不猜**：即使容器里只有一个 XML 条目也不回退
//!    （OPC 要求根文件由 `container.xml` 指定）⇒ 明确 [`MxlError::NoContainer`](crate::mxl::MxlError::NoContainer)。
//! 7. ⛔ **data descriptor**（general purpose flag bit 3）本身不需要额外代码：尺寸与 CRC 全部
//!    取**中央目录**的值 ⇒ 声明写在数据之后的容器也能读。判据
//!    `mxl_data_descriptor_container_is_read_from_the_central_directory` 用一份由 CPython
//!    `zipfile` 在**不可 seek** 的输出上写的已提交夹具（两份头都置 bit 3、本地头的
//!    CRC / 两个长度字段为 0、数据区之后是 16 字节 `PK\x07\x08` 描述符）钉住这条：
//!    **接受**，且描述符里的字段**不是**权威读数（中央目录才是）。
//!    本机 8 个 `.mxl`（2 份已提交夹具 + 6 个真文件）的 **16/16** 个条目的 bit 3 都是 0
//!    （单位 = **条目**）⇒ 这条路径只能靠自造夹具，语料碰不到。
//! 8. ⛔ **接线**：引擎 / MCP / 界面**都不**调用本模块（与 `musicxml` 同口径）。
//! 9. ⛔ **多块 DEFLATE**：`inflate::inflate_raw` 按 RFC 1951 §3.2.3 的 `BFINAL` 链读**全部**块，
//!    但本机 8 个 `.mxl` 的 **16/16** 个 DEFLATE 流都是**单块**（首块 `BFINAL=1`；单位 = **流**）
//!    ⇒ 已提交判据碰不到多块链。该形状由判据
//!    `mxl_multiblock_deflate_stream_is_read_to_its_last_block` 与
//!    `tests/fixtures/README.md` 第 8 节的自造夹具钉住（3 块，中间那块是 `stored`）。
//! 10. **滑动窗口**：`inflate::MAX_WINDOW` 是 RFC 1951 的完整 32 KiB。⛔ 本节之前那 4 份 `.mxl`
//!     夹具的 **8 个** DEFLATE 流的最远匹配距离只有 **1881 / 1881 / 1881 / 1187**（`score.xml`）
//!     与 4×**95**（`META-INF/container.xml`）字节（单位 = 字节）⇒ 单靠它们钉不住这个常量
//!     （把它降到 `2048` 也不会让任何判据变红）。证据是
//!     `tests/fixtures/handmade_mvp_partwise_long_match.mxl`（`score.xml` 里有一次距离
//!     **32506** 字节的匹配，生产者 = CPython `zlib`）与单元判据
//!     `full_window_match_is_accepted_and_the_history_check_runs_after_it`（手写固定 Huffman 流，
//!     钉住 RFC 的**精确**上界 **32768** —— `zlib` 的 `MAX_DIST` 只到 `32506`，所以两条都需要）。
//!
//! ## 分配（MusicXML **不在**音频线程 ⇒ 零分配不适用，但必须有界）
//!
//! - 输入是**借用**的切片；只在解压一个条目时分配。
//! - 上界：声明长度 > [`MxlLimits::max_entry_bytes`](crate::mxl::MxlLimits::max_entry_bytes) ⇒ **解压前**就
//!   [`MxlError::LimitExceeded`](crate::mxl::MxlError::LimitExceeded)；inflate 的**每次**写入也查同一个上界 ⇒ 声明小、实际膨胀大的
//!   容器在上界处 [`MxlError::InflatedTooLarge`](crate::mxl::MxlError::InflatedTooLarge)（一个 `max_entry_bytes` 同时挡住两种炸弹，
//!   而两种拒绝**分开报**：前者看的是**声明**，后者看的是**实际**）。
//! - 条目名字节数、条目数各有一个上界（[`MxlLimits`](crate::mxl::MxlLimits)）。
//!
//! ## 确定性 [ARCH-DET-001]
//!
//! 同一份字节 ⇒ 逐位相同的结果：容器解析是纯函数，唯一的结果由 `container.xml` 的**第一个**
//! `<rootfile` 决定（⛔ 不依赖哈希表的迭代顺序，本模块不用 `HashMap` / `HashSet`）。
//!
//! ## 不 panic 的承诺
//!
//! 任意字节输入只产生 `Ok` 或 [`MxlError`](crate::mxl::MxlError)：没有 `unwrap` / `expect` / 索引恐慌 /
//! 算术溢出（长度相加一律先查上界或用 `checked_add`）。
//! 判据 `mxl_container_fuzz_never_panics` 对本目录的 **5** 个 `.mxl` 夹具与 1 个判据自造的容器
//! 做**截断 / 翻转 / 插入**（三种变形都做；跑的次数由那条判据自己数出来并打印）。

use crate::musicxml::{MusicXmlError, MusicXmlScore, parse_musicxml};

mod inflate;

pub use inflate::{InflateError, InflateErrorKind};

/// ZIP local file header 的固定部分（APPNOTE 4.3.7；单位：**字节**）。
const LOCAL_FIXED: usize = 30;
/// ZIP central directory header 的固定部分（APPNOTE 4.3.12；单位：**字节**）。
const CENTRAL_FIXED: usize = 46;
/// EOCD（End of Central Directory）的固定部分（APPNOTE 4.3.16；单位：**字节**）。
const EOCD_FIXED: usize = 22;
/// EOCD 注释的最大长度（APPNOTE 4.3.16；单位：**字节**）—— 也是向前搜索的窗口上界。
const EOCD_MAX_COMMENT: usize = 0xffff;

const LOCAL_SIGNATURE: [u8; 4] = *b"PK\x03\x04";
const CENTRAL_SIGNATURE: [u8; 4] = *b"PK\x01\x02";
const EOCD_SIGNATURE: [u8; 4] = *b"PK\x05\x06";

/// `META-INF/container.xml` 的条目名（OPC 规定，逐字节比较）。
const CONTAINER_NAME: &[u8] = b"META-INF/container.xml";

/// 容器层与 inflate 层的上界（单位见各字段）。
///
/// 这是**工程选择**（规范未定义 `.mxl` 的上界）：默认值只求"比真文件宽、又不无界"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MxlLimits {
    /// 单个条目**膨胀后**字节数的上界。
    ///
    /// 默认 **64 MiB**：本机 12 个真 MusicXML（`/tmp/musicxml/**`，**未提交**）里最大的
    /// 纯文本是 635518 字节（`stat -f%z`）⇒ 默认值比它宽 **约 105 倍**。
    pub max_entry_bytes: usize,
    /// 中央目录允许的条目数上界。
    pub max_entries: usize,
    /// 单个条目名允许的字节数上界。
    pub max_name_bytes: usize,
}

impl Default for MxlLimits {
    fn default() -> Self {
        Self {
            max_entry_bytes: 64 * 1024 * 1024,
            max_entries: 1024,
            max_name_bytes: 4096,
        }
    }
}

/// `.mxl` 导入失败的字面读数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MxlError {
    /// 文件尾找不到 EOCD 签名 ⇒ 不是 ZIP 容器。
    NotZip,
    /// 结构被截断或字段自相矛盾。
    Malformed {
        /// 出问题的字节偏移。
        offset: usize,
        /// 一句话说明。
        detail: &'static str,
    },
    /// 条目**声明**的未压缩长度超过 [`MxlLimits::max_entry_bytes`] ⇒ **解压前**拒绝。
    LimitExceeded {
        /// 上界的名字（`"entries"` / `"name_bytes"` / `"entry_bytes"`）。
        limit: &'static str,
        /// 实际读数。
        value: usize,
        /// 允许的最大值。
        max: usize,
    },
    /// 条目**膨胀后**的输出在上界处被截停（声明的长度没超界 —— 声明可以撒谎）。
    InflatedTooLarge {
        /// 条目名。
        name: String,
        /// 允许的最大值（[`MxlLimits::max_entry_bytes`]）。
        max: usize,
    },
    /// 容器带 ZIP64 标记（条目数 `0xFFFF` 或尺寸/偏移 `0xFFFFFFFF`）。
    UnsupportedZip64,
    /// 条目的压缩法不是 0（stored）或 8（deflate）。
    UnsupportedCompression {
        /// 条目名（非 UTF-8 时按替换字符显示）。
        name: String,
        /// 压缩法编号。
        method: u16,
    },
    /// 条目带加密位（general purpose flag 的 bit 0）。
    Encrypted {
        /// 条目名（非 UTF-8 时按替换字符显示）。
        name: String,
    },
    /// 容器里没有 `META-INF/container.xml`。
    NoContainer,
    /// `META-INF/container.xml` 里没有 `<rootfile full-path="…">`。
    NoRootFile,
    /// `full-path` 指向的条目在中央目录里不存在。
    MissingRootFile {
        /// `full-path` 的值（已解 5 个预定义实体）。
        path: String,
    },
    /// raw DEFLATE 流非法（偏移是**该条目数据区**内的字节偏移）。
    InvalidDeflate {
        /// 出错时已消费的字节偏移（相对条目数据区）。
        offset: usize,
        /// 一句话说明。
        detail: &'static str,
    },
    /// 膨胀后的长度不等于中央目录声明的未压缩长度。
    SizeMismatch {
        /// 条目名。
        name: String,
        /// 中央目录声明的未压缩长度。
        declared: usize,
        /// 实际膨胀长度。
        actual: usize,
    },
    /// CRC-32 与中央目录的字段不符。
    CrcMismatch {
        /// 条目名。
        name: String,
        /// 中央目录声明的 CRC-32。
        declared: u32,
        /// 实际算出的 CRC-32。
        actual: u32,
    },
    /// 载荷不是本 crate 能读的 MusicXML（文本层的错误原样上传）。
    MusicXml(MusicXmlError),
}

impl core::fmt::Display for MxlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotZip => f.write_str("文件尾没有 ZIP 的 EOCD 签名 ⇒ 不是 .mxl 容器"),
            Self::Malformed { offset, detail } => {
                write!(f, "偏移 {offset} 处容器结构非法: {detail}")
            }
            Self::LimitExceeded { limit, value, max } => {
                write!(f, "{limit} = {value} 超过上界 {max}")
            }
            Self::InflatedTooLarge { name, max } => {
                write!(f, "条目 {name} 膨胀后超过上界 {max} 字节（已在上界处截停）")
            }
            Self::UnsupportedZip64 => f.write_str("容器带 ZIP64 标记（本模块不支持）"),
            Self::UnsupportedCompression { name, method } => {
                write!(f, "条目 {name} 的压缩法是 {method}（只认 0 与 8）")
            }
            Self::Encrypted { name } => write!(f, "条目 {name} 带加密位"),
            Self::NoContainer => f.write_str("容器里没有 META-INF/container.xml"),
            Self::NoRootFile => f.write_str("container.xml 里没有 <rootfile full-path=…>"),
            Self::MissingRootFile { path } => write!(f, "rootfile 指向的条目不存在: {path}"),
            Self::InvalidDeflate { offset, detail } => {
                write!(f, "偏移 {offset} 处 DEFLATE 流非法: {detail}")
            }
            Self::SizeMismatch {
                name,
                declared,
                actual,
            } => write!(
                f,
                "条目 {name} 膨胀后 {actual} 字节，中央目录声明 {declared} 字节"
            ),
            Self::CrcMismatch {
                name,
                declared,
                actual,
            } => write!(
                f,
                "条目 {name} 的 CRC-32 是 {actual:#010x}，中央目录声明 {declared:#010x}"
            ),
            Self::MusicXml(error) => write!(f, "载荷不是可读的 MusicXML: {error}"),
        }
    }
}

impl std::error::Error for MxlError {}

impl From<MusicXmlError> for MxlError {
    fn from(error: MusicXmlError) -> Self {
        Self::MusicXml(error)
    }
}

/// 中央目录里一个条目的**权威**字段（尺寸与 CRC 一律以中央目录为准，不看 local header）。
#[derive(Debug, Clone, Copy)]
struct CentralEntry<'a> {
    name: &'a [u8],
    flags: u16,
    method: u16,
    crc32: u32,
    compressed: u32,
    uncompressed: u32,
    local_offset: u32,
}

impl CentralEntry<'_> {
    fn display_name(&self) -> String {
        String::from_utf8_lossy(self.name).into_owned()
    }
}

/// 解析 `.mxl` 字节（**只读**），返回与 [`parse_musicxml`] 同一个 [`MusicXmlScore`]（默认上界）。
///
/// # Errors
///
/// 见 [`MxlError`]。任意字节输入都**不**会 panic。
pub fn parse_mxl(bytes: &[u8]) -> Result<MusicXmlScore, MxlError> {
    parse_mxl_with_limits(bytes, &MxlLimits::default())
}

/// 与 [`parse_mxl`] 相同，但由调用方给定上界（判据用它证明上界真的会开火）。
///
/// # Errors
///
/// 见 [`MxlError`]。任意字节输入都**不**会 panic。
pub fn parse_mxl_with_limits(bytes: &[u8], limits: &MxlLimits) -> Result<MusicXmlScore, MxlError> {
    let entries = central_directory(bytes, limits)?;
    let container = find_entry(&entries, CONTAINER_NAME).ok_or(MxlError::NoContainer)?;
    let container_xml = read_entry(bytes, &container, limits)?;
    let path = rootfile_path(&container_xml)?;
    let root = find_entry(&entries, path.as_bytes())
        .ok_or_else(|| MxlError::MissingRootFile { path: path.clone() })?;
    let payload = read_entry(bytes, &root, limits)?;
    Ok(parse_musicxml(&payload)?)
}

/// 中央目录里的全部条目，按它们在目录里的顺序。
fn central_directory<'a>(
    bytes: &'a [u8],
    limits: &MxlLimits,
) -> Result<Vec<CentralEntry<'a>>, MxlError> {
    let eocd = find_eocd(bytes)?;
    let total = usize::from(u16_at(bytes, eocd + 10)?);
    let size = u32_at(bytes, eocd + 12)?;
    let offset = u32_at(bytes, eocd + 16)?;
    if total == usize::from(u16::MAX) || size == u32::MAX || offset == u32::MAX {
        return Err(MxlError::UnsupportedZip64);
    }
    if total > limits.max_entries {
        return Err(MxlError::LimitExceeded {
            limit: "entries",
            value: total,
            max: limits.max_entries,
        });
    }
    let offset = offset as usize;
    let end = offset
        .checked_add(size as usize)
        .ok_or(MxlError::Malformed {
            offset: eocd,
            detail: "中央目录的偏移 + 尺寸溢出",
        })?;
    if end > bytes.len() {
        return Err(MxlError::Malformed {
            offset: eocd,
            detail: "中央目录越过文件尾",
        });
    }

    let mut entries = Vec::with_capacity(total);
    let mut pos = offset;
    for _ in 0..total {
        if bytes.get(pos..pos + 4) != Some(CENTRAL_SIGNATURE.as_slice()) {
            return Err(MxlError::Malformed {
                offset: pos,
                detail: "中央目录条目的签名不是 PK\\x01\\x02",
            });
        }
        let name_len = usize::from(u16_at(bytes, pos + 28)?);
        let extra_len = usize::from(u16_at(bytes, pos + 30)?);
        let comment_len = usize::from(u16_at(bytes, pos + 32)?);
        // APPNOTE 4.4.8 / 4.4.9 / 4.4.16：这三条 32 位字段取 `0xFFFFFFFF` 是 **ZIP64 标记**，
        // 真值在 ZIP64 扩展信息 extra field（APPNOTE 4.5.3，ID `0x0001`）里 ⇒ 本模块不支持。
        // ⛔ 若不在这里点名，同一个容器会按调用方给的 `max_entry_bytes` / `max_name_bytes`
        // 报出**不同**的读数（默认上界 ⇒ `LimitExceeded`，上界放到 `usize::MAX` ⇒
        // `SizeMismatch` / `Malformed`）⇒ 那是"把**格式**问题说成**策略**问题"。
        //
        // ⭐ [R54] 这三条读取的偏移（`pos + 20` / `pos + 24` / `pos + 42`）与 `name_len`
        // **无关** ⇒ 必须放在**任何**基于调用方上界的检查（含 `max_name_bytes`）**之前**：
        // 格式问题优先于策略问题。连带效果：一个**被截断**的中央目录条目（`pos + 46`
        // 越过文件尾）现在先在这里报 `Malformed`（越界读 32 位字段），⛔ 不再是
        // `LimitExceeded { limit: "name_bytes" }` —— 截断是格式问题，这个读数更诚实。
        // 判据 `mxl_zip64_markers_are_named_not_blamed_on_the_limit` 与
        // `the_zip64_marker_fires_before_the_entry_name_limit` 钉住这条。
        let compressed = u32_at(bytes, pos + 20)?;
        let uncompressed = u32_at(bytes, pos + 24)?;
        let local_offset = u32_at(bytes, pos + 42)?;
        if compressed == u32::MAX || uncompressed == u32::MAX || local_offset == u32::MAX {
            return Err(MxlError::UnsupportedZip64);
        }
        if name_len > limits.max_name_bytes {
            return Err(MxlError::LimitExceeded {
                limit: "name_bytes",
                value: name_len,
                max: limits.max_name_bytes,
            });
        }
        let name_start = pos + CENTRAL_FIXED;
        let name = bytes
            .get(name_start..name_start + name_len)
            .ok_or(MxlError::Malformed {
                offset: name_start,
                detail: "条目名越过文件尾",
            })?;
        let flags = u16_at(bytes, pos + 8)?;
        let method = u16_at(bytes, pos + 10)?;
        let crc32 = u32_at(bytes, pos + 16)?;
        entries.push(CentralEntry {
            name,
            flags,
            method,
            crc32,
            compressed,
            uncompressed,
            local_offset,
        });
        pos = name_start + name_len + extra_len + comment_len;
        if pos > bytes.len() {
            return Err(MxlError::Malformed {
                offset: pos,
                detail: "中央目录条目越过文件尾",
            });
        }
    }
    Ok(entries)
}

/// 从文件尾向前找 EOCD（APPNOTE 4.3.16）；注释最长 65535 字节 ⇒ 回到该窗口之外就放弃。
fn find_eocd(bytes: &[u8]) -> Result<usize, MxlError> {
    if bytes.len() < EOCD_FIXED {
        return Err(MxlError::NotZip);
    }
    let lowest = bytes.len().saturating_sub(EOCD_FIXED + EOCD_MAX_COMMENT);
    let mut pos = bytes.len() - EOCD_FIXED;
    loop {
        if bytes.get(pos..pos + 4) == Some(EOCD_SIGNATURE.as_slice()) {
            let comment = usize::from(u16_at(bytes, pos + 20)?);
            if pos + EOCD_FIXED + comment == bytes.len() {
                return Ok(pos);
            }
        }
        if pos == lowest {
            return Err(MxlError::NotZip);
        }
        pos -= 1;
    }
}

/// 按**条目名逐字节**查找（⛔ 不做大小写折叠或路径归一化：那是猜测，不是 OPC 语义）。
fn find_entry<'a>(entries: &'a [CentralEntry<'a>], name: &[u8]) -> Option<CentralEntry<'a>> {
    entries.iter().copied().find(|entry| entry.name == name)
}

/// 取本地文件头之后的数据区（⛔ 尺寸用中央目录的值 ⇒ data descriptor 容器也能读）。
fn entry_data<'a>(bytes: &'a [u8], entry: &CentralEntry<'_>) -> Result<&'a [u8], MxlError> {
    let offset = entry.local_offset as usize;
    if bytes.get(offset..offset + 4) != Some(LOCAL_SIGNATURE.as_slice()) {
        return Err(MxlError::Malformed {
            offset,
            detail: "local file header 的签名不是 PK\\x03\\x04",
        });
    }
    let name_len = usize::from(u16_at(bytes, offset + 26)?);
    let extra_len = usize::from(u16_at(bytes, offset + 28)?);
    let start = offset
        .checked_add(LOCAL_FIXED)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len))
        .ok_or(MxlError::Malformed {
            offset,
            detail: "local file header 的长度字段溢出",
        })?;
    let end = start
        .checked_add(entry.compressed as usize)
        .ok_or(MxlError::Malformed {
            offset: start,
            detail: "条目数据区长度溢出",
        })?;
    bytes.get(start..end).ok_or(MxlError::Malformed {
        offset: start,
        detail: "条目数据区越过文件尾",
    })
}

/// 解出一个条目的**未压缩**内容，并核对长度与 CRC-32。
fn read_entry(
    bytes: &[u8],
    entry: &CentralEntry<'_>,
    limits: &MxlLimits,
) -> Result<Vec<u8>, MxlError> {
    let name = entry.display_name();
    if entry.flags & 0x0001 != 0 {
        return Err(MxlError::Encrypted { name });
    }
    let declared = entry.uncompressed as usize;
    if declared > limits.max_entry_bytes {
        return Err(MxlError::LimitExceeded {
            limit: "entry_bytes",
            value: declared,
            max: limits.max_entry_bytes,
        });
    }
    let data = entry_data(bytes, entry)?;
    let actual = match entry.method {
        0 => data.to_vec(),
        8 => inflate::inflate_raw(data, limits.max_entry_bytes).map_err(|error| {
            match error.kind {
                InflateErrorKind::Limit => MxlError::InflatedTooLarge {
                    name: entry.display_name(),
                    max: limits.max_entry_bytes,
                },
                InflateErrorKind::Malformed => MxlError::InvalidDeflate {
                    offset: error.offset,
                    detail: error.detail,
                },
            }
        })?,
        method => {
            return Err(MxlError::UnsupportedCompression { name, method });
        }
    };
    if actual.len() != declared {
        return Err(MxlError::SizeMismatch {
            name,
            declared,
            actual: actual.len(),
        });
    }
    let actual_crc = crc32(&actual);
    if actual_crc != entry.crc32 {
        return Err(MxlError::CrcMismatch {
            name,
            declared: entry.crc32,
            actual: actual_crc,
        });
    }
    Ok(actual)
}

/// 从 `META-INF/container.xml` 里取**第一个** `<rootfile full-path="…">` 的值。
fn rootfile_path(xml: &[u8]) -> Result<String, MxlError> {
    let mut pos = 0usize;
    while let Some(open) = memchr(b'<', &xml[pos..]) {
        let open = pos + open;
        let rest = &xml[open..];
        if rest.starts_with(b"<!--") {
            let Some(end) = find(rest, b"-->") else {
                return Err(MxlError::NoRootFile);
            };
            pos = open + end + 3;
            continue;
        }
        if rest.starts_with(b"<?") {
            let Some(end) = find(rest, b"?>") else {
                return Err(MxlError::NoRootFile);
            };
            pos = open + end + 2;
            continue;
        }
        if rest.starts_with(b"<!") {
            let Some(end) = memchr(b'>', rest) else {
                return Err(MxlError::NoRootFile);
            };
            pos = open + end + 1;
            continue;
        }
        let Some(end) = tag_end(rest) else {
            return Err(MxlError::NoRootFile);
        };
        let tag = &rest[1..end];
        if tag_name(tag) == Some(b"rootfile".as_slice())
            && let Some(value) = attribute(tag, b"full-path")
        {
            return Ok(decode_entities(&value));
        }
        pos = open + end + 1;
    }
    Err(MxlError::NoRootFile)
}

/// 找到标签的 `>`（⛔ 跳过引号内的 `>`）。
fn tag_end(rest: &[u8]) -> Option<usize> {
    let mut quote = None;
    for (index, &byte) in rest.iter().enumerate() {
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => return Some(index),
                _ => {}
            },
        }
    }
    None
}

/// 标签名（`<rootfile …>` ⇒ `rootfile`；不含 `<` / `>` / `/` / 空白）。
fn tag_name(tag: &[u8]) -> Option<&[u8]> {
    let start = tag.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let end = tag[start..]
        .iter()
        .position(|byte| byte.is_ascii_whitespace() || *byte == b'/' || *byte == b'>')
        .map_or(tag.len(), |offset| start + offset);
    tag.get(start..end)
}

/// 取标签里某个属性的值（去引号，⛔ 不解实体；调用方决定要不要解）。
///
/// 扫描**按 XML 的引号规则走**：`"…"` / `'…'` 之间的字节一律不作为属性名的起点。
/// 因此 `<rootfile note="full-path='other.xml'" full-path="score.xml"/>` 取到的是
/// `score.xml`，⛔ 不是另一个属性**值**里那段长得很像属性的文本（若不按引号走，
/// 容器里存在的 `other.xml` 会被**静默**当成根文件解析 —— 同一份字节，
/// 本模块与符合规范的读取器会读出**不同**的乐谱）。
/// 判据 `mxl_rootfile_attribute_is_not_read_from_another_attribute_value` 钉住这条。
fn attribute(tag: &[u8], key: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0usize;
    let mut quote: Option<u8> = None;
    while let Some(&byte) = tag.get(pos) {
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
                pos += 1;
            }
            None if byte == b'"' || byte == b'\'' => {
                quote = Some(byte);
                pos += 1;
            }
            None => {
                // 属性名必须是独立词：前一个字节不能是名字字符。
                let boundary = pos == 0 || !is_name_byte(tag[pos - 1]);
                if boundary
                    && tag[pos..].starts_with(key)
                    && let Some(value) = quoted_value(tag, pos + key.len())
                {
                    return Some(value);
                }
                pos += 1;
            }
        }
    }
    None
}

/// `=` 之后被引号包住的值；`cursor` 指向属性名的下一个字节。
///
/// ⛔ 不是任何一步都 `Err`/`panic`：形状不对就 `None`（调用方继续往后找）。
fn quoted_value(tag: &[u8], mut cursor: usize) -> Option<Vec<u8>> {
    while tag.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if tag.get(cursor) != Some(&b'=') {
        return None;
    }
    cursor += 1;
    while tag.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let opening = *tag.get(cursor)?;
    if opening != b'"' && opening != b'\'' {
        return None;
    }
    let start = cursor + 1;
    let end = start + memchr(opening, tag.get(start..)?)?;
    tag.get(start..end).map(<[u8]>::to_vec)
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.' || byte == b':'
}

/// 解 XML 的 5 个预定义实体；其它 `&…` 原样保留（见模块文档的边界第 5 条）。
fn decode_entities(value: &[u8]) -> String {
    let text = String::from_utf8_lossy(value);
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// CRC-32（IEEE 802.3，反射多项式 `0xEDB88320`）—— 与 ZIP 条目的完整性字段同一算法。
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|byte| *byte == needle)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, MxlError> {
    let slice = bytes.get(offset..offset + 2).ok_or(MxlError::Malformed {
        offset,
        detail: "读 16 位字段时越过文件尾",
    })?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, MxlError> {
    let slice = bytes.get(offset..offset + 4).ok_or(MxlError::Malformed {
        offset,
        detail: "读 32 位字段时越过文件尾",
    })?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_published_check_value() {
        // CRC-32 的标准自检向量（IEEE 802.3 / PNG / ZIP 用的是同一个）。
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn attribute_values_do_not_masquerade_as_attributes() {
        // 引号状态由扫描器自己跟：另一个属性的**值**里那段 `full-path='…'` 不是属性。
        let score = Some(b"score.xml".to_vec());
        assert_eq!(
            attribute(
                br#"rootfile note="full-path='other.xml'" full-path="score.xml""#,
                b"full-path"
            ),
            score,
            "双引号值里的单引号伪属性必须被跳过"
        );
        assert_eq!(
            attribute(
                br#"rootfile note='full-path="other.xml"' full-path="score.xml""#,
                b"full-path"
            ),
            score,
            "单引号值里的双引号伪属性必须被跳过"
        );
        // 名字字符边界照旧：`data-full-path` / `xfull-path` 都不是 `full-path`。
        assert_eq!(
            attribute(br#"rootfile data-full-path="other.xml""#, b"full-path"),
            None
        );
        assert_eq!(
            attribute(br#"rootfile xfull-path="other.xml""#, b"full-path"),
            None
        );
        // 形状不对（没有 `=`）⇒ 继续往后找，⛔ 不是整个标签放弃。
        assert_eq!(
            attribute(br#"rootfile full-path full-path="score.xml""#, b"full-path"),
            score
        );
    }

    #[test]
    fn rootfile_path_follows_the_first_rootfile_tag() {
        let xml = br#"<?xml version="1.0"?>
            <!-- <rootfile full-path="commented.xml"/> -->
            <container><rootfiles>
              <rootfile full-path="score.xml" media-type="application/vnd.recordare.musicxml"/>
            </rootfiles></container>"#;
        assert_eq!(rootfile_path(xml).as_deref(), Ok("score.xml"));
    }

    #[test]
    fn rootfile_path_decodes_the_five_predefined_entities() {
        let xml = br#"<container><rootfiles><rootfile full-path="a&amp;b/score.xml"/></rootfiles></container>"#;
        assert_eq!(rootfile_path(xml).as_deref(), Ok("a&b/score.xml"));
    }

    #[test]
    fn rootfile_path_is_an_error_when_absent() {
        assert_eq!(
            rootfile_path(b"<container><rootfiles/></container>"),
            Err(MxlError::NoRootFile)
        );
        // 单数/复数不混淆：`<rootfiles>`（父元素）不是 `<rootfile>`。
        assert_eq!(
            rootfile_path(b"<rootfiles></rootfiles>"),
            Err(MxlError::NoRootFile)
        );
    }

    #[test]
    fn eocd_search_rejects_a_file_that_is_too_short() {
        assert_eq!(find_eocd(b"PK\x03\x04"), Err(MxlError::NotZip));
        assert_eq!(find_eocd(b""), Err(MxlError::NotZip));
    }

    /// 判据 (类别③ 明确 Err / 错误文案黄金表): `MxlError` 的 **14** 个变体各有一个
    /// **字面** `Display` 读数。
    ///
    /// 补的是哪个缺口（本票注入实测）：把每一个变体的文案各改坏一次（注入 R01..R14），
    /// **14 次全部全绿** ⇒ 在本 crate 的 `tests/` 与 `src/` 里对 `MxlError` 的
    /// `to_string()` / `format!` / `Display` 引用次数此前是 **0**。
    #[test]
    fn mxl_error_display_text_is_pinned_for_every_variant() {
        let cases: Vec<(MxlError, &str)> = vec![
            (
                MxlError::NotZip,
                "文件尾没有 ZIP 的 EOCD 签名 ⇒ 不是 .mxl 容器",
            ),
            (
                MxlError::Malformed {
                    offset: 4,
                    detail: "中央目录越过文件尾",
                },
                "偏移 4 处容器结构非法: 中央目录越过文件尾",
            ),
            (
                MxlError::LimitExceeded {
                    limit: "entries",
                    value: 2,
                    max: 1,
                },
                "entries = 2 超过上界 1",
            ),
            (
                MxlError::InflatedTooLarge {
                    name: "score.xml".to_owned(),
                    max: 64,
                },
                "条目 score.xml 膨胀后超过上界 64 字节（已在上界处截停）",
            ),
            (
                MxlError::UnsupportedZip64,
                "容器带 ZIP64 标记（本模块不支持）",
            ),
            (
                MxlError::UnsupportedCompression {
                    name: "a.bin".to_owned(),
                    method: 12,
                },
                "条目 a.bin 的压缩法是 12（只认 0 与 8）",
            ),
            (
                MxlError::Encrypted {
                    name: "a.bin".to_owned(),
                },
                "条目 a.bin 带加密位",
            ),
            (MxlError::NoContainer, "容器里没有 META-INF/container.xml"),
            (
                MxlError::NoRootFile,
                "container.xml 里没有 <rootfile full-path=…>",
            ),
            (
                MxlError::MissingRootFile {
                    path: "score.xml".to_owned(),
                },
                "rootfile 指向的条目不存在: score.xml",
            ),
            (
                MxlError::InvalidDeflate {
                    offset: 7,
                    detail: "输入在块中途结束",
                },
                "偏移 7 处 DEFLATE 流非法: 输入在块中途结束",
            ),
            (
                MxlError::SizeMismatch {
                    name: "score.xml".to_owned(),
                    declared: 100,
                    actual: 2716,
                },
                "条目 score.xml 膨胀后 2716 字节，中央目录声明 100 字节",
            ),
            (
                MxlError::CrcMismatch {
                    name: "score.xml".to_owned(),
                    declared: 1,
                    actual: 2,
                },
                "条目 score.xml 的 CRC-32 是 0x00000002，中央目录声明 0x00000001",
            ),
            (
                MxlError::MusicXml(MusicXmlError::Empty),
                "载荷不是可读的 MusicXML: 输入里没有任何元素",
            ),
        ];
        assert_eq!(cases.len(), 14, "MxlError 的变体数");
        assert_every_mxl_error_arm_is_covered(&cases);
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?} 的 Display 文案");
        }

        // ⚠️ **登记（未被注入验证）**：`MxlError` 的 `Error::source()` 走 std 的默认实现
        // ⇒ 即使 `MusicXml(..)` 包着一个 `MusicXmlError` 也恒 `None`。本批没有能打它的
        // 字面注入。⛔ 不计入"已注入验证"。
        assert!(
            std::error::Error::source(&MxlError::MusicXml(MusicXmlError::Empty)).is_none(),
            "MxlError 的 source() 当前是 std 默认的 None（连 MusicXml 臂也一样）"
        );
    }

    /// 判据 (类别④ 参数极值 / 默认值): `MxlLimits::default()` 的三个上界是**字面值**。
    ///
    /// 补的是哪个缺口（本票注入实测）：把三个默认值各砍半（注入 V01/V02/V03）后
    /// 全部判据**保持绿** —— 已有的判据把 `MxlLimits::default().max_entry_bytes`
    /// **当作期望值自己用**（`mxl_zip64_markers_are_named_not_blamed_on_the_limit`
    /// 的 `max:` 字段），那是"常量自比"：默认值改了，期望值跟着改，恒真。
    #[test]
    fn mxl_limits_defaults_are_literal() {
        let limits = MxlLimits::default();
        assert_eq!(
            limits.max_entry_bytes, 67108864,
            "默认条目上界 = 64 MiB（字面值，⛔ 不用 64 * 1024 * 1024）"
        );
        assert_eq!(limits.max_entries, 1024, "默认条目数（字面值）");
        assert_eq!(limits.max_name_bytes, 4096, "默认条目名上界（字面值）");
    }

    /// **编译期穷举探针**：`MxlError` 的每个变体一个唯一编号 ⇒ 新增变体会让这个 `match`
    /// 非穷举、**编译失败**（`cases.len() == 14` 只自校验表的长度）。
    /// ⛔ **不许给这个 `match` 加 `_ =>` 通配臂**（R51）：加了以后新增变体也能编译过，
    /// 探针立刻**静默失效**，而**所有判据仍然全绿**。
    fn mxl_error_arm(error: &MxlError) -> u8 {
        match error {
            MxlError::NotZip => 0,
            MxlError::Malformed { .. } => 1,
            MxlError::LimitExceeded { .. } => 2,
            MxlError::InflatedTooLarge { .. } => 3,
            MxlError::UnsupportedZip64 => 4,
            MxlError::UnsupportedCompression { .. } => 5,
            MxlError::Encrypted { .. } => 6,
            MxlError::NoContainer => 7,
            MxlError::NoRootFile => 8,
            MxlError::MissingRootFile { .. } => 9,
            MxlError::InvalidDeflate { .. } => 10,
            MxlError::SizeMismatch { .. } => 11,
            MxlError::CrcMismatch { .. } => 12,
            MxlError::MusicXml(_) => 13,
        }
    }

    /// 黄金表必须**逐臂恰好一次**。
    fn assert_every_mxl_error_arm_is_covered(cases: &[(MxlError, &str)]) {
        let mut arms: Vec<u8> = cases
            .iter()
            .map(|(error, _)| mxl_error_arm(error))
            .collect();
        arms.sort_unstable();
        assert_eq!(
            arms,
            (0..14).collect::<Vec<u8>>(),
            "黄金表必须逐臂恰好一次（缺一臂或重复都红）"
        );
    }

    /// 判据 (类别: 公开 `Debug` 形状，续): 两个**错误枚举**的派生 `Debug` 输出逐字面钉住
    /// （`MxlError` / `MusicXmlError`）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把这两个枚举的 `#[derive(Debug)]` 各顶成一个写死的
    /// 手写 `impl Debug`（注入 `b9:DBG13` / `b9:DBG14`）后全部判据**保持绿**
    /// —— 这两个类型是 `assert_eq!` 失败消息里最常出现的两个，而它们的 `Debug` 形状
    /// 此前没有判据（第八批钉的是**结果**类型，本批补**错误**类型）。
    #[test]
    fn error_enum_debug_shapes_are_pinned() {
        assert_eq!(format!("{:?}", MxlError::NotZip), "NotZip");
        assert_eq!(
            format!("{:?}", MxlError::UnsupportedZip64),
            "UnsupportedZip64"
        );
        assert_eq!(
            format!(
                "{:?}",
                MxlError::Malformed {
                    offset: 4,
                    detail: "x",
                }
            ),
            "Malformed { offset: 4, detail: \"x\" }"
        );
        assert_eq!(
            format!(
                "{:?}",
                MxlError::LimitExceeded {
                    limit: "entries",
                    value: 2,
                    max: 1,
                }
            ),
            "LimitExceeded { limit: \"entries\", value: 2, max: 1 }"
        );
        assert_eq!(
            format!("{:?}", crate::musicxml::MusicXmlError::Empty),
            "Empty"
        );
        assert_eq!(
            format!(
                "{:?}",
                crate::musicxml::MusicXmlError::InvalidUtf8 { offset: 17 }
            ),
            "InvalidUtf8 { offset: 17 }"
        );
        assert_eq!(
            format!(
                "{:?}",
                crate::musicxml::MusicXmlError::UnsupportedRoot {
                    root: "score-timewise".to_owned(),
                }
            ),
            "UnsupportedRoot { root: \"score-timewise\" }"
        );
    }

    /// 判据 (R183: **常驻判据自带绿/红两臂** ⇒ ⭐ **外部注入分母归 0**): 上面那条真表判据是
    /// **绿臂**（真表必须通过）；本条是**红臂** —— 把**坏表**喂给同一个 checker，它必须**有牙**。
    /// ⭐ 下界早已抽成函数 `assert_every_mxl_error_arm_is_covered`（R183 的第一半），所以可以直接喂坏输入。
    /// ⭐ R180: 自检**不用集合大小界**，而是喂**具体坏表**再看它是否被拒。
    #[test]
    fn the_arm_coverage_checker_rejects_broken_tables() {
        eprintln!("[R187-PROBE b23:the_arm_coverage_checker_rejects_broken_tables] ran");
        // ⚠️ 这些错误枚举**不含 `Copy`**（有 `String` 载荷）⇒ 用工厂闭包，⛔ 不能移动同一个值两次。
        let real = || MxlError::NotZip;
        // 红臂①（缺臂）：只有一行 ⇒ 编号集合不完整。
        let too_short = [(real(), "不是 ZIP")];
        assert!(
            std::panic::catch_unwind(|| assert_every_mxl_error_arm_is_covered(&too_short)).is_err(),
            "缺臂的坏表必须被拒（本 crate 有 14 个变体）"
        );
        // 红臂②（重复臂）：同一行两遍 ⇒ 集合有多余。
        let duplicated = [(real(), "不是 ZIP"), (real(), "不是 ZIP")];
        assert!(
            std::panic::catch_unwind(|| assert_every_mxl_error_arm_is_covered(&duplicated))
                .is_err(),
            "重复臂的坏表必须被拒"
        );
    }
}
