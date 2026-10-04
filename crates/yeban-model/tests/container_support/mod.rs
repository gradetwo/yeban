//! 对抗性 `.yeban` 判据共用的夹具工具：写一个合法容器，然后**只改一个字段**把它变成恶意归档。
//!
//! 为什么用"改字节"而不是"手搓一个 ZIP"：手搓的恶意归档很容易同时踩中好几条防御，
//! 于是判据变红时分不清是哪一条在起作用。这里先让 `write_container` 产出一个**完全合法**的
//! 归档，再定点改写一个字段 —— 归档里除了那一个字段之外处处合法，于是"变红"必然归因于
//! 我们想测的那一条防御。
//!
//! 所有偏移量来自 PKWARE APPNOTE 6.3.10 的结构定义（见 `src/container/zip.rs` 模块文档）。
#![allow(dead_code)]

use yeban_model::container::{ContainerEntry, ContainerLimits, write_container};

/// Local file header 签名。
pub const LOCAL_SIGNATURE: u32 = 0x0403_4B50;
/// Central directory file header 签名。
pub const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;
/// End of central directory 签名。
pub const EOCD_SIGNATURE: u32 = 0x0605_4B50;

/// Local header：通用位标志。
pub const LOCAL_FLAGS: usize = 6;
/// Local header：压缩法。
pub const LOCAL_METHOD: usize = 8;
/// Local header：CRC-32。
pub const LOCAL_CRC: usize = 14;
/// Local header：压缩后大小。
pub const LOCAL_COMPRESSED: usize = 18;
/// Local header：解压后大小。
pub const LOCAL_UNCOMPRESSED: usize = 22;
/// Local header：名字起始偏移。
pub const LOCAL_NAME: usize = 30;

/// Central header：`version made by`。
pub const CENTRAL_VERSION_MADE_BY: usize = 4;
/// Central header：通用位标志。
pub const CENTRAL_FLAGS: usize = 8;
/// Central header：压缩法。
pub const CENTRAL_METHOD: usize = 10;
/// Central header：CRC-32。
pub const CENTRAL_CRC: usize = 16;
/// Central header：压缩后大小。
pub const CENTRAL_COMPRESSED: usize = 20;
/// Central header：解压后大小。
pub const CENTRAL_UNCOMPRESSED: usize = 24;
/// Central header：外部属性。
pub const CENTRAL_EXTERNAL_ATTRIBUTES: usize = 38;
/// Central header：local header 偏移。
pub const CENTRAL_LOCAL_OFFSET: usize = 42;
/// Central header：名字起始偏移。
pub const CENTRAL_NAME: usize = 46;

/// EOCD：磁盘号。
pub const EOCD_DISK: usize = 4;
/// EOCD：central directory 所在磁盘号。
pub const EOCD_CENTRAL_DISK: usize = 6;
/// EOCD：本盘条目数。
pub const EOCD_ENTRIES_ON_DISK: usize = 8;
/// EOCD：总条目数。
pub const EOCD_TOTAL_ENTRIES: usize = 10;
/// EOCD：central directory 长度。
pub const EOCD_CENTRAL_SIZE: usize = 12;
/// EOCD：central directory 偏移。
pub const EOCD_CENTRAL_OFFSET: usize = 16;

/// 写一个合法容器。
pub fn container(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let owned: Vec<ContainerEntry> = entries
        .iter()
        .map(|(name, data)| ContainerEntry::new(*name, data.to_vec()))
        .collect();
    write_container(&owned).expect("夹具必须是可写出的合法容器")
}

/// 小数据上就能触发的注入阈值（判据不可能真去造 2 GB）。
pub fn small_limits() -> ContainerLimits {
    ContainerLimits {
        max_entry_bytes: 1024,
        max_total_bytes: 4096,
        max_ratio: 100,
        max_entries: 64,
    }
}

/// 构造自定义阈值。
pub fn limits(
    max_entry_bytes: u64,
    max_total_bytes: u64,
    max_ratio: u64,
    max_entries: usize,
) -> ContainerLimits {
    ContainerLimits {
        max_entry_bytes,
        max_total_bytes,
        max_ratio,
        max_entries,
    }
}

/// 小端读 `u16`。
pub fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// 小端读 `u32`。
pub fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// 小端写 `u16`。
pub fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// 小端写 `u32`。
pub fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// 找出 `needle` 在 `haystack` 中的全部起始偏移。
pub fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut hits = Vec::new();
    if needle.is_empty() || needle.len() > haystack.len() {
        return hits;
    }
    for start in 0..=(haystack.len() - needle.len()) {
        if &haystack[start..start + needle.len()] == needle {
            hits.push(start);
        }
    }
    hits
}

/// 把归档中所有出现的 `from` 替换成 `to`（要求等长，避免改动任何长度字段）。
pub fn patch_all(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len(), "定点改写必须等长");
    let hits = find_all(bytes, from);
    assert!(!hits.is_empty(), "夹具里没有找到待改写的字节");
    for start in hits {
        bytes[start..start + to.len()].copy_from_slice(to);
    }
}

/// 按签名 + 名字定位 local file header。
pub fn find_local(bytes: &[u8], name: &str) -> usize {
    for start in find_all(bytes, &LOCAL_SIGNATURE.to_le_bytes()) {
        let name_len = usize::from(read_u16(bytes, start + 26));
        let name_start = start + LOCAL_NAME;
        if name_start + name_len <= bytes.len()
            && &bytes[name_start..name_start + name_len] == name.as_bytes()
        {
            return start;
        }
    }
    panic!("找不到 local header: {name}");
}

/// 按签名 + 名字定位 central directory 记录。
pub fn find_central(bytes: &[u8], name: &str) -> usize {
    for start in find_all(bytes, &CENTRAL_SIGNATURE.to_le_bytes()) {
        let name_len = usize::from(read_u16(bytes, start + 28));
        let name_start = start + CENTRAL_NAME;
        if name_start + name_len <= bytes.len()
            && &bytes[name_start..name_start + name_len] == name.as_bytes()
        {
            return start;
        }
    }
    panic!("找不到 central record: {name}");
}

/// 定位 EOCD。
pub fn find_eocd(bytes: &[u8]) -> usize {
    for start in find_all(bytes, &EOCD_SIGNATURE.to_le_bytes()) {
        if start + 22 <= bytes.len() {
            let comment_len = usize::from(read_u16(bytes, start + 20));
            if start + 22 + comment_len == bytes.len() {
                return start;
            }
        }
    }
    panic!("找不到 EOCD");
}

/// 条目数据区的起始偏移（依赖写入器把 extra field 写成 0 长度）。
pub fn data_start(bytes: &[u8], name: &str) -> usize {
    let local = find_local(bytes, name);
    let name_len = usize::from(read_u16(bytes, local + 26));
    let extra_len = usize::from(read_u16(bytes, local + 28));
    local + LOCAL_NAME + name_len + extra_len
}

/// 同时改写 local header 与 central directory 的"声明尺寸"（两处保持一致）。
///
/// 用于造出"结构自洽、但声明值与实际字节不符"的归档 —— 这正是"谎报声明"攻击的形状。
pub fn declare_sizes(bytes: &mut [u8], name: &str, compressed: u32, uncompressed: u32) {
    let local = find_local(bytes, name);
    put_u32(bytes, local + LOCAL_COMPRESSED, compressed);
    put_u32(bytes, local + LOCAL_UNCOMPRESSED, uncompressed);
    let central = find_central(bytes, name);
    put_u32(bytes, central + CENTRAL_COMPRESSED, compressed);
    put_u32(bytes, central + CENTRAL_UNCOMPRESSED, uncompressed);
}
