//! 最小 ZIP 读写器（ZIP32 子集）—— 零第三方依赖、无 `unsafe`、无 panic。
//!
//! # 实现的 ZIP 子集与依据
//!
//! 依据 **PKWARE APPNOTE.TXT 6.3.10**（`.ZIP` File Format Specification）的以下结构：
//!
//! | 结构 | 签名 | 本实现 | 依据 |
//! | :--- | :--- | :--- | :--- |
//! | Local file header | `0x04034B50` | 读 + 写 | APPNOTE 4.3.7 |
//! | Central directory file header | `0x02014B50` | 读 + 写 | APPNOTE 4.3.12 |
//! | End of central directory (EOCD) | `0x06054B50` | 读 + 写 | APPNOTE 4.3.16 |
//! | 压缩法 `stored` (method 0) | — | 读 + 写 | APPNOTE 4.4.5 |
//! | 压缩法 `deflate` (method 8) | — | **拒绝**（`UnsupportedCompression`） | 本线裁决：不引入 `flate2` |
//! | ZIP64 | — | **拒绝**（`UnsupportedZip64`） | 单条目上限 2 GB ⇒ ZIP32 足够 |
//! | 加密 | — | **拒绝**（`EncryptedEntryUnsupported`） | 本线范围外 |
//! | Data descriptor (flag bit 3) | — | **拒绝**（`UnsupportedDataDescriptor`） | 写入器永远先写尺寸 |
//! | 多卷 / 跨盘 | — | **拒绝**（`UnsupportedMultiDisk`） | 归档必须是单文件 |
//! | 符号链接条目 | — | **拒绝**（`SymlinkEntryUnsupported`） | `ARCH-SEC-003` "跨卷符号链接" |
//!
//! # 确定性（`ARCH-DET-001`）
//!
//! 写入器只有一个输入自由度（条目序列），它把**其余全部字段钉成常量**：
//! 固定的 DOS 时间戳 `1980-01-01 00:00:00`、固定的 `version made by`（Unix 3.0）、
//! 固定的外部属性（`0o100644`）、固定置位的 UTF-8 标志位 `0x0800`。
//! 因此"同一输入两次写出逐字节相同"就是这条常量化的直接推论，并由判据钉住。
//!
//! # 读路径的核心裁决：**central directory 是权威，且必须与 local header 一致**
//!
//! ZIP 允许 local header 与 central directory 对同一字段给出不同的值（历史上这是
//! "不同解包器读出不同文件"的真实攻击面）。本读取器**两处都读、逐字段比对、任何不一致
//! 直接拒绝**（[`ContainerError::LocalCentralMismatch`]）。这样就没有"以谁为准"的歧义：
//! 歧义本身就是漏洞，消除歧义的方式是拒绝而不是选一个。

use std::collections::BTreeSet;

use super::crc32::crc32;
use super::{ContainerEntry, ContainerError, ContainerLimits};

/// Local file header 签名。
pub(crate) const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4B50;
/// Central directory file header 签名。
pub(crate) const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4B50;
/// End of central directory 签名。
pub(crate) const EOCD_SIGNATURE: u32 = 0x0605_4B50;

/// Local file header 的定长部分（不含名字/额外字段）。
const LOCAL_HEADER_LEN: usize = 30;
/// Central directory file header 的定长部分（不含名字/额外字段/注释）。
const CENTRAL_HEADER_LEN: usize = 46;
/// EOCD 的定长部分（不含注释）。
const EOCD_LEN: usize = 22;
/// EOCD 注释的最大长度（EOCD 注释长度字段是 `u16`）。
const EOCD_MAX_COMMENT: usize = 65_535;

/// `version needed to extract`：2.0（ZIP32 基线）。
const VERSION_NEEDED: u16 = 20;
/// `version made by`：高字节 3 = Unix，低字节 30 = 规范 3.0。
const VERSION_MADE_BY_UNIX: u16 = 0x031E;
/// 通用位标志 bit 11：文件名为 UTF-8。
const FLAG_UTF8: u16 = 0x0800;
/// 通用位标志 bit 0：条目已加密。
const FLAG_ENCRYPTED: u16 = 0x0001;
/// 通用位标志 bit 3：尺寸写在 data descriptor 里。
const FLAG_DATA_DESCRIPTOR: u16 = 0x0008;

/// 压缩法 `stored`（APPNOTE 4.4.5）。方法 8 = `deflate` 与其余一切取值都在读取路径上
/// 被**明确拒绝**（[`ContainerError::UnsupportedCompression`]），绝不静默跳过或猜测。
const METHOD_STORED: u16 = 0;

/// Unix 常规文件 `0o100644`，放在外部属性的高 16 位。
const UNIX_MODE_REGULAR_FILE: u32 = (0o100_644u32) << 16;
/// Unix 文件类型掩码 `S_IFMT`。
const UNIX_S_IFMT: u32 = 0o170_000;
/// Unix 符号链接 `S_IFLNK`。
const UNIX_S_IFLNK: u32 = 0o120_000;

/// 固定 DOS 时间 `00:00:00`（见模块文档"确定性"）。
const DOS_TIME: u16 = 0;
/// 固定 DOS 日期 `1980-01-01`（DOS 日期的纪元起点）。
const DOS_DATE: u16 = 0x0021;

/// ZIP32 的 `u16` 饱和值：出现在 EOCD 条目数上表示"见 ZIP64"。
const ZIP64_SENTINEL_U16: u16 = 0xFFFF;
/// ZIP32 的 `u32` 饱和值：出现在尺寸/偏移上表示"见 ZIP64"。
const ZIP64_SENTINEL_U32: u32 = 0xFFFF_FFFF;

/// 只前向推进、越界即 `Err` 的 panic-free 切片游标。
///
/// 为什么不用 `bytes[i]`：解析路径上的下标 panic 就是 `MUST-GATE-011`（格式解析零崩溃）
/// 要拦的东西。所有越界都变成 [`ContainerError::TruncatedArchive`]，调用方再用更精确的
/// 前置长度检查给出具体错误。
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// 在 `bytes` 的 `pos` 处建立游标。
    const fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    /// 取走 `len` 字节；越界返回 `Err`。
    fn take(&mut self, len: usize) -> Result<&'a [u8], ContainerError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(ContainerError::TruncatedArchive)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(ContainerError::TruncatedArchive)?;
        self.pos = end;
        Ok(slice)
    }

    /// 读一个小端 `u16`。
    fn u16(&mut self) -> Result<u16, ContainerError> {
        let raw = self.take(2)?;
        let array: [u8; 2] = raw
            .try_into()
            .map_err(|_| ContainerError::TruncatedArchive)?;
        Ok(u16::from_le_bytes(array))
    }

    /// 读一个小端 `u32`。
    fn u32(&mut self) -> Result<u32, ContainerError> {
        let raw = self.take(4)?;
        let array: [u8; 4] = raw
            .try_into()
            .map_err(|_| ContainerError::TruncatedArchive)?;
        Ok(u32::from_le_bytes(array))
    }
}

/// 在 `offset` 处读一个小端 `u16`（越界返回 `None`）。
fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw = bytes.get(offset..offset.checked_add(2)?)?;
    let array: [u8; 2] = raw.try_into().ok()?;
    Some(u16::from_le_bytes(array))
}

/// 在 `offset` 处读一个小端 `u32`（越界返回 `None`）。
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset.checked_add(4)?)?;
    let array: [u8; 4] = raw.try_into().ok()?;
    Some(u32::from_le_bytes(array))
}

/// 一个已解析的 central directory 记录（后续再做防御判定）。
#[derive(Debug, Clone)]
struct CentralRecord {
    /// 在 central directory 中的序号（错误信息定位用）。
    index: usize,
    /// 条目名（已通过 UTF-8 校验，尚未通过路径校验）。
    name: String,
    /// 通用位标志。
    flags: u16,
    /// 压缩法。
    method: u16,
    /// 声明的 CRC-32。
    crc: u32,
    /// 声明的压缩后大小。
    compressed: u32,
    /// 声明的解压后大小。
    uncompressed: u32,
    /// `version made by`（高字节 = 宿主系统）。
    version_made_by: u16,
    /// 外部属性（Unix 模式下高 16 位是 `st_mode`）。
    external_attributes: u32,
    /// local file header 的偏移。
    local_header_offset: u32,
}

/// 把**借用**形态的条目序列 `(名字, 字节)` 写成标准 ZIP 字节流。
///
/// 这是写入路径的**唯一**实现：拥有所有权的 [`ContainerEntry`] 形态（[`write_zip`]）
/// 只是本函数的一层适配。借用形态让"工程容器写出"不必为每一条资产先复制一份
/// `Vec<u8>`（见 [`super::write_project_container_borrowed`] 的文档）。
///
/// # Errors
///
/// - 条目名不安全（见 [`super::normalize_entry_name`]）；
/// - 条目名大小写折叠后重复（[`ContainerError::DuplicateEntryName`]）；
/// - 条目数超过 ZIP32 上限 65535（[`ContainerError::TooManyEntries`]）；
/// - 单条目或全归档超过 ZIP32 的 `u32` 上限（[`ContainerError::EntryTooLarge`] /
///   [`ContainerError::ArchiveTooLarge`]）。
pub(crate) fn write_zip_borrowed(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, ContainerError> {
    // EOCD 的条目数字段是 `u16`，而 `0xFFFF` 是"见 ZIP64"的哨兵 ⇒ 本写入器最多 65534 条，
    // 否则它自己写出的归档会被本读取器按 ZIP64 拒绝（"写得出、读不回"是必须拦住的失配）。
    let max_zip32_entries = usize::from(ZIP64_SENTINEL_U16) - 1;
    if entries.len() > max_zip32_entries {
        return Err(ContainerError::TooManyEntries {
            found: entries.len(),
            max: max_zip32_entries,
        });
    }

    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut total: u64 = 0;

    for (raw_name, data) in entries {
        let name = super::normalize_entry_name(raw_name)?;
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(ContainerError::DuplicateEntryName { name });
        }
        if name.len() > usize::from(u16::MAX) {
            return Err(ContainerError::EntryNameTooLong {
                len: name.len(),
                max: usize::from(u16::MAX),
            });
        }
        let size = u32::try_from(data.len()).map_err(|_| ContainerError::EntryTooLarge {
            declared: data.len() as u64,
            max: u64::from(ZIP64_SENTINEL_U32),
        })?;
        total += u64::from(size);
        if total > u64::from(ZIP64_SENTINEL_U32) {
            return Err(ContainerError::ArchiveTooLarge {
                actual: total,
                max: u64::from(ZIP64_SENTINEL_U32),
            });
        }
        let offset = u32::try_from(out.len()).map_err(|_| ContainerError::ArchiveTooLarge {
            actual: out.len() as u64,
            max: u64::from(ZIP64_SENTINEL_U32),
        })?;
        let signature = crc32(data);
        let name_len = name.len() as u16;

        // --- local file header (APPNOTE 4.3.7) ---
        out.extend_from_slice(&LOCAL_FILE_HEADER_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
        out.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        out.extend_from_slice(&METHOD_STORED.to_le_bytes());
        out.extend_from_slice(&DOS_TIME.to_le_bytes());
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&signature.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        // --- central directory file header (APPNOTE 4.3.12) ---
        central.extend_from_slice(&CENTRAL_DIRECTORY_SIGNATURE.to_le_bytes());
        central.extend_from_slice(&VERSION_MADE_BY_UNIX.to_le_bytes());
        central.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
        central.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        central.extend_from_slice(&METHOD_STORED.to_le_bytes());
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&signature.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&UNIX_MODE_REGULAR_FILE.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }

    let central_offset = u32::try_from(out.len()).map_err(|_| ContainerError::ArchiveTooLarge {
        actual: out.len() as u64,
        max: u64::from(ZIP64_SENTINEL_U32),
    })?;
    let central_size =
        u32::try_from(central.len()).map_err(|_| ContainerError::ArchiveTooLarge {
            actual: central.len() as u64,
            max: u64::from(ZIP64_SENTINEL_U32),
        })?;
    out.extend_from_slice(&central);

    // --- end of central directory (APPNOTE 4.3.16) ---
    let count = entries.len() as u16;
    out.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    Ok(out)
}

/// 把拥有所有权的 [`ContainerEntry`] 序列写成标准 ZIP 字节流。
///
/// 这是 [`write_zip_borrowed`] 的适配层：它只借用每个条目的名字与字节，
/// 因此**不再复制**条目内容。判据 `write_zip_and_write_zip_borrowed_agree_byte_for_byte`
/// 钉住两条入口逐字节一致。
///
/// # Errors
///
/// 同 [`write_zip_borrowed`]。
pub(crate) fn write_zip(entries: &[ContainerEntry]) -> Result<Vec<u8>, ContainerError> {
    let borrowed: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.data.as_slice()))
        .collect();
    write_zip_borrowed(&borrowed)
}

/// 从后往前定位 EOCD，并校验注释长度与文件长度相符。
fn find_eocd(bytes: &[u8]) -> Result<usize, ContainerError> {
    if bytes.len() < EOCD_LEN {
        return Err(ContainerError::EocdNotFound);
    }
    let lowest = bytes.len().saturating_sub(EOCD_LEN + EOCD_MAX_COMMENT);
    let mut cursor = bytes.len() - EOCD_LEN;
    loop {
        if u32_at(bytes, cursor) == Some(EOCD_SIGNATURE) {
            let comment_len = usize::from(u16_at(bytes, cursor + 20).unwrap_or(0));
            // 只有"注释长度恰好补齐到文件末尾"的签名才算真 EOCD，否则是注释里的假签名。
            if cursor + EOCD_LEN + comment_len == bytes.len() {
                return Ok(cursor);
            }
        }
        if cursor == lowest {
            return Err(ContainerError::EocdNotFound);
        }
        cursor -= 1;
    }
}

/// 解析 central directory 的全部记录（结构层，尚未做安全判定）。
fn parse_central_directory(
    bytes: &[u8],
    cd_start: usize,
    cd_end: usize,
    count: usize,
) -> Result<Vec<CentralRecord>, ContainerError> {
    let mut records = Vec::with_capacity(count);
    let mut reader = Reader::new(bytes, cd_start);
    for index in 0..count {
        let header_end = reader
            .pos
            .checked_add(CENTRAL_HEADER_LEN)
            .ok_or(ContainerError::TruncatedCentralDirectory { index })?;
        if header_end > cd_end {
            return Err(ContainerError::TruncatedCentralDirectory { index });
        }
        let signature = reader.u32()?;
        if signature != CENTRAL_DIRECTORY_SIGNATURE {
            return Err(ContainerError::BadCentralDirectorySignature {
                index,
                found: signature,
            });
        }
        let version_made_by = reader.u16()?;
        let _version_needed = reader.u16()?;
        let flags = reader.u16()?;
        let method = reader.u16()?;
        let _time = reader.u16()?;
        let _date = reader.u16()?;
        let crc = reader.u32()?;
        let compressed = reader.u32()?;
        let uncompressed = reader.u32()?;
        let name_len = usize::from(reader.u16()?);
        let extra_len = usize::from(reader.u16()?);
        let comment_len = usize::from(reader.u16()?);
        let disk_start = reader.u16()?;
        let _internal = reader.u16()?;
        let external_attributes = reader.u32()?;
        let local_header_offset = reader.u32()?;
        if disk_start != 0 {
            return Err(ContainerError::UnsupportedMultiDisk);
        }
        let name_bytes = reader.take(name_len)?;
        let _extra = reader.take(extra_len)?;
        let _comment = reader.take(comment_len)?;
        if reader.pos > cd_end {
            return Err(ContainerError::TruncatedCentralDirectory { index });
        }
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| ContainerError::EntryNameNotUtf8 { index })?
            .to_owned();
        records.push(CentralRecord {
            index,
            name,
            flags,
            method,
            crc,
            compressed,
            uncompressed,
            version_made_by,
            external_attributes,
            local_header_offset,
        });
    }
    if reader.pos != cd_end {
        return Err(ContainerError::CentralDirectorySizeMismatch {
            declared: cd_end - cd_start,
            actual: reader.pos - cd_start,
        });
    }
    Ok(records)
}

/// 读取并校验一个 `.yeban` 容器，应用 `MUST-GATE-006` / `MUST-GATE-007` 两条防御。
///
/// # 判定顺序（**顺序本身是契约的一部分**，判据断言的是精确错误码）
///
/// 1. EOCD 定位与卷/条目数/ZIP64 哨兵检查；
/// 2. central directory 的边界、签名、逐条解析与"声明长度 == 实际消耗"核对；
/// 3. 逐条：
///    1. 压缩法（`deflate` ⇒ `UnsupportedCompression`）；
///    2. 加密标志、data descriptor 标志；
///    3. ZIP64 哨兵；
///    4. **路径规范化（`MUST-GATE-006`）**；
///    5. 符号链接条目；
///    6. 声明解压体积 > `max_entry_bytes`；
///    7. **实际读取体积 > `max_entry_bytes`（防谎报声明）**；
///    8. 累计实际体积 > `max_total_bytes`；
///    9. **累计膨胀比率 > `max_ratio`（`MUST-GATE-007`，fail-fast 用声明值）**；
///    10. `stored` 的压缩前后尺寸必须相等（谎报声明的第二道拦截）；
///    11. local header 与 central directory 逐字段一致；
///    12. 数据区边界、CRC 校验。
/// 4. 名字大小写折叠后的重复检查。
///
/// 第 9 步刻意排在"尺寸一致性"之前：炸弹**声明**本身就足以拒绝，不应该等我们把结构
/// 核对完再拒绝（fail-fast）；而第 7/8 步用**实际字节数**兜底，防止"声明撒谎绕过上限"。
/// 两者覆盖不同的攻击（前者防真炸弹，后者防假声明），判据分别钉住。
pub(crate) fn read_zip(
    bytes: &[u8],
    limits: &ContainerLimits,
) -> Result<Vec<ContainerEntry>, ContainerError> {
    let eocd = find_eocd(bytes)?;
    let mut tail = Reader::new(bytes, eocd);
    let _signature = tail.u32()?;
    let disk_number = tail.u16()?;
    let central_disk = tail.u16()?;
    let entries_on_disk = tail.u16()?;
    let total_entries = tail.u16()?;
    let central_size = tail.u32()?;
    let central_offset = tail.u32()?;
    let _comment_len = tail.u16()?;

    if disk_number != 0 || central_disk != 0 || entries_on_disk != total_entries {
        return Err(ContainerError::UnsupportedMultiDisk);
    }
    if total_entries == ZIP64_SENTINEL_U16
        || central_size == ZIP64_SENTINEL_U32
        || central_offset == ZIP64_SENTINEL_U32
    {
        return Err(ContainerError::UnsupportedZip64);
    }

    let count = usize::from(total_entries);
    if count > limits.max_entries {
        return Err(ContainerError::TooManyEntries {
            found: count,
            max: limits.max_entries,
        });
    }

    let cd_start = central_offset as usize;
    let cd_len = central_size as usize;
    let cd_end = cd_start
        .checked_add(cd_len)
        .ok_or(ContainerError::CentralDirectoryOutOfBounds)?;
    if cd_end > eocd {
        return Err(ContainerError::CentralDirectoryOutOfBounds);
    }

    let records = parse_central_directory(bytes, cd_start, cd_end, count)?;

    let mut entries: Vec<ContainerEntry> = Vec::with_capacity(records.len());
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut actual_total: u64 = 0;
    let mut declared_uncompressed_total: u64 = 0;
    let mut declared_compressed_total: u64 = 0;

    for record in &records {
        let index = record.index;
        let name = record.name.as_str();

        // (3.1) 压缩法：明确报错，绝不静默跳过或猜测。
        if record.method != METHOD_STORED {
            return Err(ContainerError::UnsupportedCompression {
                index,
                method: record.method,
            });
        }
        // (3.2) 加密与 data descriptor（我们既不实现也不猜）。
        if record.flags & FLAG_ENCRYPTED != 0 {
            return Err(ContainerError::EncryptedEntryUnsupported {
                index,
                name: record.name.clone(),
            });
        }
        if record.flags & FLAG_DATA_DESCRIPTOR != 0 {
            return Err(ContainerError::UnsupportedDataDescriptor { index });
        }
        // (3.3) ZIP64 哨兵。
        if record.compressed == ZIP64_SENTINEL_U32
            || record.uncompressed == ZIP64_SENTINEL_U32
            || record.local_header_offset == ZIP64_SENTINEL_U32
        {
            return Err(ContainerError::UnsupportedZip64);
        }

        // (3.4) MUST-GATE-006：路径规范化。
        let safe_name = super::normalize_entry_name(name)?;

        // (3.5) 符号链接条目（ARCH-SEC-003 的"跨卷符号链接"）。
        if is_unix_symlink(record) {
            return Err(ContainerError::SymlinkEntryUnsupported {
                index,
                name: record.name.clone(),
            });
        }

        // (3.6) 声明解压体积上限（规范 §5.3：单条目 ≤ 2 GB）。
        let declared = u64::from(record.uncompressed);
        if declared > limits.max_entry_bytes {
            return Err(ContainerError::EntryTooLarge {
                declared,
                max: limits.max_entry_bytes,
            });
        }

        // (3.7) 实际会materialize的字节数：`stored` 下就是压缩后大小（数据区长度）。
        // 这一条是"声明撒谎"的兜底：谎报 `uncompressed = 1` 也拿不走 2 MiB 的写入量。
        let actual = u64::from(record.compressed);
        if actual > limits.max_entry_bytes {
            return Err(ContainerError::EntryActualTooLarge {
                actual,
                max: limits.max_entry_bytes,
            });
        }
        // (3.8) 累计实际体积。
        actual_total += actual;
        if actual_total > limits.max_total_bytes {
            return Err(ContainerError::ArchiveTooLarge {
                actual: actual_total,
                max: limits.max_total_bytes,
            });
        }
        // (3.9) MUST-GATE-007：整体膨胀比率（fail-fast，用声明值）。
        declared_uncompressed_total += declared;
        declared_compressed_total += actual;
        if ratio_exceeded(
            declared_uncompressed_total,
            declared_compressed_total,
            limits.max_ratio,
        ) {
            return Err(ContainerError::ExpansionRatioExceeded {
                uncompressed: declared_uncompressed_total,
                compressed: declared_compressed_total,
                max_ratio: limits.max_ratio,
            });
        }
        // (3.10) `stored` 的压缩前后尺寸必须相等。
        if record.compressed != record.uncompressed {
            return Err(ContainerError::StoredSizeMismatch {
                index,
                name: record.name.clone(),
                compressed: record.compressed,
                uncompressed: record.uncompressed,
            });
        }

        // (3.11) local header 与 central directory 逐字段一致。
        let data = verify_local_header_and_locate_data(bytes, record, &safe_name)?;

        // (3.12) CRC 校验。
        let actual_crc = crc32(data);
        if actual_crc != record.crc {
            return Err(ContainerError::CrcMismatch {
                index,
                name: record.name.clone(),
                declared: record.crc,
                actual: actual_crc,
            });
        }

        // 名字重复（大小写折叠后）会让大小写不敏感的文件系统互相覆盖。
        if !seen.insert(safe_name.to_ascii_lowercase()) {
            return Err(ContainerError::DuplicateEntryName { name: safe_name });
        }

        entries.push(ContainerEntry {
            name: safe_name,
            data: data.to_vec(),
        });
    }

    Ok(entries)
}

/// 该记录是否声明为 Unix 符号链接。
fn is_unix_symlink(record: &CentralRecord) -> bool {
    let host = record.version_made_by >> 8;
    if host != 3 {
        return false;
    }
    (record.external_attributes >> 16) & UNIX_S_IFMT == UNIX_S_IFLNK
}

/// 校验 local header 与 central directory 完全一致，并返回条目数据切片。
fn verify_local_header_and_locate_data<'a>(
    bytes: &'a [u8],
    record: &CentralRecord,
    safe_name: &str,
) -> Result<&'a [u8], ContainerError> {
    let index = record.index;
    let start = record.local_header_offset as usize;
    if start
        .checked_add(LOCAL_HEADER_LEN)
        .is_none_or(|end| end > bytes.len())
    {
        return Err(ContainerError::LocalHeaderOutOfBounds { index });
    }
    let mut reader = Reader::new(bytes, start);
    let signature = reader.u32()?;
    if signature != LOCAL_FILE_HEADER_SIGNATURE {
        return Err(ContainerError::BadLocalHeaderSignature {
            index,
            found: signature,
        });
    }
    let _version_needed = reader.u16()?;
    let flags = reader.u16()?;
    let method = reader.u16()?;
    let _time = reader.u16()?;
    let _date = reader.u16()?;
    let crc = reader.u32()?;
    let compressed = reader.u32()?;
    let uncompressed = reader.u32()?;
    let name_len = usize::from(reader.u16()?);
    let extra_len = usize::from(reader.u16()?);
    if reader
        .pos
        .checked_add(name_len + extra_len)
        .is_none_or(|end| end > bytes.len())
    {
        return Err(ContainerError::LocalHeaderOutOfBounds { index });
    }
    let local_name = reader.take(name_len)?;
    let _extra = reader.take(extra_len)?;

    if flags != record.flags
        || method != record.method
        || crc != record.crc
        || compressed != record.compressed
        || uncompressed != record.uncompressed
        || local_name != record.name.as_bytes()
        || local_name != safe_name.as_bytes()
    {
        return Err(ContainerError::LocalCentralMismatch {
            index,
            name: record.name.clone(),
        });
    }

    let data_start = reader.pos;
    let data_end = data_start
        .checked_add(record.compressed as usize)
        .ok_or(ContainerError::TruncatedEntryData { index })?;
    bytes
        .get(data_start..data_end)
        .ok_or(ContainerError::TruncatedEntryData { index })
}

/// 累计膨胀比率是否超过上限。
///
/// 用乘法而不是除法：`compressed == 0` 时除法会 panic 或产出 `inf`，而乘法只要
/// 一边溢出就 `saturating`（`max_ratio = u64::MAX` 因此等价于"关闭这一条"，这正是
/// 注入判据用来把本防御"关掉"的方式）。
fn ratio_exceeded(uncompressed: u64, compressed: u64, max_ratio: u64) -> bool {
    if compressed == 0 {
        return uncompressed > 0;
    }
    uncompressed > compressed.saturating_mul(max_ratio)
}

#[cfg(test)]
mod tests {
    use super::{EOCD_SIGNATURE, find_eocd, ratio_exceeded, u32_at, write_zip, write_zip_borrowed};
    use crate::container::{ContainerEntry, ContainerError};

    /// `MUST-GATE-007` 的核心算术：`≤ 100:1` 放行、`> 100:1` 拒绝（边界必须精确）。
    #[test]
    fn ratio_boundary_is_exact() {
        assert!(!ratio_exceeded(0, 0, 100), "空归档必须放行");
        assert!(!ratio_exceeded(100, 1, 100), "恰好 100:1 必须放行");
        assert!(ratio_exceeded(101, 1, 100), "101:1 必须拒绝");
        assert!(
            ratio_exceeded(1, 0, 100),
            "无压缩字节却有解压字节 ⇒ 比率无穷大"
        );
        assert!(
            !ratio_exceeded(u64::MAX, 1, u64::MAX),
            "max_ratio = u64::MAX 等价于关闭该条（饱和乘法，不 panic）"
        );
        assert!(ratio_exceeded(1, 1, 0), "max_ratio = 0 时任何膨胀都被拒绝");
    }

    /// EOCD 从尾部定位；注释长度必须恰好补齐到文件末尾。
    #[test]
    fn eocd_with_comment_is_found_at_the_right_offset() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"PK\x05\x06abcd");
        assert_eq!(find_eocd(&bytes), Ok(0));
    }

    /// 文件中出现**更早**的假 EOCD 签名时，必须继续往前扫到真的那一个。
    #[test]
    fn earlier_fake_eocd_signature_is_skipped() {
        let mut bytes = Vec::new();
        let fake_start = bytes.len();
        bytes.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 18]);
        bytes.extend_from_slice(&[0u8; 4]); // 让假 EOCD 的"注释长度补齐"不成立
        let real_start = bytes.len();
        bytes.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 18]);
        assert_eq!(fake_start, 0);
        assert_eq!(find_eocd(&bytes), Ok(real_start));
    }

    /// 写出的容器能被自己的定位器找到（写入器与读取器的 EOCD 布局必须一致）。
    #[test]
    fn written_container_exposes_a_valid_eocd() {
        let zip = write_zip(&[ContainerEntry::new("project.json", b"{}".to_vec())])
            .expect("写入必须成功");
        let eocd = find_eocd(&zip).expect("必须找到 EOCD");
        assert_eq!(u32_at(&zip, eocd), Some(EOCD_SIGNATURE));
        assert_eq!(eocd + 22, zip.len(), "写入器不写注释");
    }

    /// 截断到不足一个 EOCD 必须报 `EocdNotFound` 而不是 panic。
    #[test]
    fn truncated_eocd_is_an_error_not_a_panic() {
        assert_eq!(find_eocd(&[]), Err(ContainerError::EocdNotFound));
        assert_eq!(find_eocd(&[0u8; 21]), Err(ContainerError::EocdNotFound));
    }

    /// 拥有所有权形态与借用形态**逐字节一致**：借用形态是唯一实现，拥有形态只是适配层。
    #[test]
    fn write_zip_and_write_zip_borrowed_agree_byte_for_byte() {
        let owned = [
            ContainerEntry::new("project.json", b"{}".to_vec()),
            ContainerEntry::new("assets/aa", vec![0u8, 1, 2, 3]),
            ContainerEntry::new("assets/bb", vec![255u8; 64]),
        ];
        let borrowed: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|entry| (entry.name.as_str(), entry.data.as_slice()))
            .collect();

        let from_owned = write_zip(&owned).expect("拥有形态写入必须成功");
        let from_borrowed = write_zip_borrowed(&borrowed).expect("借用形态写入必须成功");
        assert_eq!(
            from_owned, from_borrowed,
            "两条写入入口必须写出同一份字节（否则读者会看到两种归档）"
        );
    }
}
