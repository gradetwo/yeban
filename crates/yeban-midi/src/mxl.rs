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
//! 2. ⛔ **加密**：general purpose flag 的 bit 0 ⇒ 明确 [`MxlError::Encrypted`](crate::mxl::MxlError::Encrypted)。
//! 3. ⛔ **其它压缩法**：只认 0 与 8（deflate）。`bzip2`(12) / `lzma`(14) / `zstd`(93) 等明确
//!    [`MxlError::UnsupportedCompression`](crate::mxl::MxlError::UnsupportedCompression)。
//! 4. ⛔ **写出**：本模块只读。`.mxl` 打包不存在。
//! 5. ⛔ **不做 XML 解析器**：`META-INF/container.xml` 只用一个小扫描器取 `<rootfile>`
//!    标签的 `full-path` 属性；属性值里只解 5 个预定义实体（`&amp;` `&lt;` `&gt;`
//!    `&quot;` `&apos;`），**数字字符引用原样保留**（⇒ 路径对不上条目名时是明确的
//!    [`MxlError::MissingRootFile`](crate::mxl::MxlError::MissingRootFile)，⛔ 不会静默读错文件）。
//! 6. ⛔ **无 `META-INF/container.xml` 时不猜**：即使容器里只有一个 XML 条目也不回退
//!    （OPC 要求根文件由 `container.xml` 指定）⇒ 明确 [`MxlError::NoContainer`](crate::mxl::MxlError::NoContainer)。
//! 7. ⛔ **data descriptor**（general purpose flag bit 3）本身不需要额外代码：尺寸与 CRC 全部
//!    取**中央目录**的值 ⇒ 声明写在数据之后的容器也能读。但**没有**针对它的独立判据。
//! 8. ⛔ **接线**：引擎 / MCP / 界面**都不**调用本模块（与 `musicxml` 同口径）。
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
//! 判据 `mxl_container_fuzz_never_panics` 对本目录的 4 个容器夹具做截断 / 翻转 / 插入。

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
        entries.push(CentralEntry {
            name,
            flags: u16_at(bytes, pos + 8)?,
            method: u16_at(bytes, pos + 10)?,
            crc32: u32_at(bytes, pos + 16)?,
            compressed: u32_at(bytes, pos + 20)?,
            uncompressed: u32_at(bytes, pos + 24)?,
            local_offset: u32_at(bytes, pos + 42)?,
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
fn attribute(tag: &[u8], key: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0usize;
    while let Some(found) = find(&tag[pos..], key) {
        let at = pos + found;
        let after = at + key.len();
        // 属性名必须是独立词：前一个字节不能是名字字符。
        let boundary = at == 0 || !is_name_byte(tag[at - 1]);
        let mut cursor = after;
        while tag.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if boundary && tag.get(cursor) == Some(&b'=') {
            cursor += 1;
            while tag.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let quote = *tag.get(cursor)?;
            if quote == b'"' || quote == b'\'' {
                let start = cursor + 1;
                let end = start + memchr(quote, tag.get(start..)?)?;
                return tag.get(start..end).map(<[u8]>::to_vec);
            }
        }
        pos = at + 1;
    }
    None
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
}
