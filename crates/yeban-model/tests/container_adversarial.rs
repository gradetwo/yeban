//! 对抗性 `.yeban` 容器判据：`MUST-GATE-006`（Zip-Slip）/ `MUST-GATE-007`（解压炸弹）+ 结构畸形。
//!
//! 规范来源：
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3 `[ARCH-SEC-003]`
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `[MUST-GATE-006]` / `[MUST-GATE-007]`
//!
//! 每个恶意归档都由 [`container_support`] 先写出**完全合法**的容器，再定点改写**一个**字段，
//! 因此"变红"必然归因于被改写的那一处。

mod container_support;

use container_support::{
    CENTRAL_CRC, CENTRAL_EXTERNAL_ATTRIBUTES, CENTRAL_FLAGS, CENTRAL_LOCAL_OFFSET, CENTRAL_METHOD,
    CENTRAL_UNCOMPRESSED, EOCD_CENTRAL_OFFSET, EOCD_CENTRAL_SIZE, EOCD_DISK, EOCD_ENTRIES_ON_DISK,
    EOCD_TOTAL_ENTRIES, LOCAL_FLAGS, LOCAL_METHOD, LOCAL_NAME, LOCAL_UNCOMPRESSED, container,
    data_start, declare_sizes, find_central, find_eocd, find_local, limits, patch_all, put_u16,
    put_u32, read_u32, small_limits,
};
use yeban_model::container::{
    ContainerEntry, ContainerError, ContainerLimits, DEFAULT_MAX_ENTRY_BYTES, DEFAULT_MAX_RATIO,
    read_container, write_container,
};

/// 把合法容器里长度为 `len` 的占位名改成恶意名（等长定点改写）。
fn with_name(len: usize, evil: &str) -> Vec<u8> {
    assert_eq!(len, evil.len(), "定点改写必须等长");
    let placeholder = "a".repeat(len);
    let mut bytes = container(&[(placeholder.as_str(), b"\x00\x01\x02")]);
    patch_all(&mut bytes, placeholder.as_bytes(), evil.as_bytes());
    bytes
}

/// 断言归档被拒绝，并逐字段检查错误内容。
fn assert_rejected(bytes: &[u8], limits: &ContainerLimits, expected: ContainerError) {
    match read_container(bytes, limits) {
        Err(actual) => assert_eq!(actual, expected),
        Ok(archive) => panic!("期望被拒绝，实际读出了 {} 个条目", archive.len()),
    }
}

// ======================================================================
// MUST-GATE-006：Zip-Slip 路径穿越防御
// ======================================================================

/// `../evil`（规范点名的第一条）。
#[test]
fn dotdot_relative_path_is_rejected() {
    assert_rejected(
        &with_name(7, "../evil"),
        &small_limits(),
        ContainerError::ParentDirSegment {
            name: "../evil".into(),
        },
    );
}

/// `../../etc/passwd`：多层穿越。
#[test]
fn deep_dotdot_etc_passwd_is_rejected() {
    assert_rejected(
        &with_name(16, "../../etc/passwd"),
        &small_limits(),
        ContainerError::ParentDirSegment {
            name: "../../etc/passwd".into(),
        },
    );
}

/// 裸 `..`（整个条目名就是父目录）。
#[test]
fn bare_dotdot_name_is_rejected() {
    assert_rejected(
        &with_name(2, ".."),
        &small_limits(),
        ContainerError::ParentDirSegment { name: "..".into() },
    );
}

/// 段中间的 `..`（`a/../../b`）：只查前缀/后缀的实现会漏掉这一条。
#[test]
fn interior_dotdot_segment_is_rejected() {
    assert_rejected(
        &with_name(9, "a/../../b"),
        &small_limits(),
        ContainerError::ParentDirSegment {
            name: "a/../../b".into(),
        },
    );
}

/// 绝对根路径 `/abs/path`。
#[test]
fn absolute_unix_path_is_rejected() {
    assert_rejected(
        &with_name(9, "/abs/path"),
        &small_limits(),
        ContainerError::AbsoluteEntryPath {
            name: "/abs/path".into(),
        },
    );
}

/// UNC / 跨卷前缀 `//server/share/x`。
#[test]
fn unc_path_is_rejected() {
    assert_rejected(
        &with_name(16, "//server/share/x"),
        &small_limits(),
        ContainerError::AbsoluteEntryPath {
            name: "//server/share/x".into(),
        },
    );
}

/// Windows 盘符 + 反斜杠 `C:\abs`。
#[test]
fn windows_drive_absolute_path_is_rejected() {
    assert_rejected(
        &with_name(6, "C:\\abs"),
        &small_limits(),
        ContainerError::BackslashInEntryName {
            name: "C:\\abs".into(),
        },
    );
}

/// Windows 盘符相对路径 `C:rel`（不是绝对路径，但同样能写到别处）。
#[test]
fn windows_drive_relative_path_is_rejected() {
    assert_rejected(
        &with_name(5, "C:rel"),
        &small_limits(),
        ContainerError::ColonInEntryName {
            name: "C:rel".into(),
        },
    );
}

/// NTFS 数据流 `a:stream!`（同名文件挂隐藏流）。
#[test]
fn ntfs_alternate_data_stream_is_rejected() {
    assert_rejected(
        &with_name(9, "a:stream!"),
        &small_limits(),
        ContainerError::ColonInEntryName {
            name: "a:stream!".into(),
        },
    );
}

/// 名字里含 NUL 字节。
#[test]
fn nul_byte_in_name_is_rejected() {
    assert_rejected(
        &with_name(4, "a\0b\u{1}"),
        &small_limits(),
        ContainerError::NulInEntryName {
            name: "a\0b\u{1}".into(),
        },
    );
}

/// 名字里含其它控制字符（`\n`）。
#[test]
fn control_character_in_name_is_rejected() {
    assert_rejected(
        &with_name(4, "a\nb!"),
        &small_limits(),
        ContainerError::ControlCharInEntryName {
            name: "a\nb!".into(),
        },
    );
}

/// 空段 `a//b`。
#[test]
fn empty_path_segment_is_rejected() {
    assert_rejected(
        &with_name(4, "a//b"),
        &small_limits(),
        ContainerError::EmptyPathSegment {
            name: "a//b".into(),
        },
    );
}

/// 目录条目 `assets/`（`.yeban` 只存文件）。
#[test]
fn directory_entry_is_rejected() {
    assert_rejected(
        &with_name(7, "assets/"),
        &small_limits(),
        ContainerError::DirectoryEntryUnsupported {
            name: "assets/".into(),
        },
    );
}

/// `.` 段：不同平台折叠规则不同，属于规范化歧义，一律拒绝。
#[test]
fn current_dir_segment_is_rejected() {
    assert_rejected(
        &with_name(4, "./a!"),
        &small_limits(),
        ContainerError::CurrentDirSegment {
            name: "./a!".into(),
        },
    );
}

/// Windows 会剥掉段尾的点/空格 ⇒ `"a/.. "` 在 Windows 上等价于 `a/..`，也就是穿越。
///
/// 这是最隐蔽的一类绕过：朴素实现只比对 `segment == ".."`，于是把 `".. "` 当普通段放行，
/// 而 Windows 的文件系统在打开时会把它变成 `..`。
#[test]
fn windows_trailing_dot_or_space_bypass_is_rejected() {
    assert_rejected(
        &with_name(5, "a/.. "),
        &small_limits(),
        ContainerError::TrailingDotOrSpaceSegment {
            name: "a/.. ".into(),
        },
    );
    assert_rejected(
        &with_name(3, ".. "),
        &small_limits(),
        ContainerError::TrailingDotOrSpaceSegment { name: ".. ".into() },
    );
}

/// Windows 保留设备名（写的是设备而不是文件）。
#[test]
fn windows_reserved_device_name_is_rejected() {
    assert_rejected(
        &with_name(3, "NUL"),
        &small_limits(),
        ContainerError::WindowsReservedName { name: "NUL".into() },
    );
}

/// 百分号编码**不是** ZIP 的路径语法：`..%2f` 是字面文件名，必须原样接受。
///
/// 这条判据存在的意义：它把"本实现不做百分号解码"从一句注释变成可执行断言 ——
/// 任何日后加上"先解码再判定"的改动都会让它变红。
#[test]
fn percent_encoded_dotdot_is_a_literal_name_not_a_path() {
    let bytes = container(&[("..%2f", b"literal")]);
    let archive = read_container(&bytes, &small_limits()).expect("字面名字必须被接受");
    assert_eq!(archive.get("..%2f").expect("条目必须存在").data, b"literal");
}

/// Unicode 全角句点 `．`(U+FF0E) **不是** `.`：它是字面字符，必须原样接受。
#[test]
fn unicode_fullwidth_dot_is_a_literal_name_not_a_segment() {
    let name = "\u{FF0E}\u{FF0E}/x";
    let bytes = container(&[(name, b"wide")]);
    let archive = read_container(&bytes, &small_limits()).expect("全角字符名必须被接受");
    assert_eq!(archive.get(name).expect("条目必须存在").data, b"wide");
}

/// 大小写折叠后重名（APFS/NTFS 上会互相覆盖）必须拒绝。
#[test]
fn case_variant_duplicate_names_are_rejected() {
    let mut bytes = container(&[("aaa", b"\x01"), ("bbb", b"\x02")]);
    patch_all(&mut bytes, b"bbb", b"AAA");
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::DuplicateEntryName { name: "AAA".into() },
    );
}

/// 完全同名的重复条目必须拒绝。
#[test]
fn exact_duplicate_names_are_rejected() {
    let mut bytes = container(&[("aaa", b"\x01"), ("bbb", b"\x02")]);
    patch_all(&mut bytes, b"bbb", b"aaa");
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::DuplicateEntryName { name: "aaa".into() },
    );
}

/// 写入口同样拒绝危险名字（否则我们能写出自己读不回来的容器）。
#[test]
fn write_container_rejects_unsafe_names() {
    let entry = ContainerEntry::new("../evil", b"x".to_vec());
    assert_eq!(
        write_container(&[entry]),
        Err(ContainerError::ParentDirSegment {
            name: "../evil".into()
        })
    );
}

/// 写入口拒绝大小写折叠后的重名。
#[test]
fn write_container_rejects_case_insensitive_duplicates() {
    let entries = vec![
        ContainerEntry::new("a/project.json", b"1".to_vec()),
        ContainerEntry::new("A/PROJECT.JSON", b"2".to_vec()),
    ];
    assert_eq!(
        write_container(&entries),
        Err(ContainerError::DuplicateEntryName {
            name: "A/PROJECT.JSON".into()
        })
    );
}

/// 符号链接条目（`ARCH-SEC-003` 的"跨卷符号链接"）：Unix 模式下 `S_IFLNK`。
#[test]
fn symlink_entry_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let central = find_central(&bytes, "aaaa");
    put_u32(
        &mut bytes,
        central + CENTRAL_EXTERNAL_ATTRIBUTES,
        0xA1FF_0000,
    );
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::SymlinkEntryUnsupported {
            index: 0,
            name: "aaaa".into(),
        },
    );
}

// ======================================================================
// MUST-GATE-007：解压炸弹防御
// ======================================================================

/// 规范默认阈值必须被钉住（2 GB / 100:1），且一律取"更紧"的一侧。
#[test]
fn spec_defaults_are_pinned() {
    assert_eq!(DEFAULT_MAX_ENTRY_BYTES, 2_000_000_000);
    assert_eq!(DEFAULT_MAX_RATIO, 100);
    let defaults = ContainerLimits::default();
    assert_eq!(defaults.max_entry_bytes, DEFAULT_MAX_ENTRY_BYTES);
    assert_eq!(defaults.max_ratio, DEFAULT_MAX_RATIO);
    assert!(
        defaults.max_entry_bytes <= 2_000_000_000,
        "规范要求单条目 ≤ 2GB，默认值不得放宽"
    );
}

/// 声明的解压体积超过 `max_entry_bytes` ⇒ 拒绝（不截断、不跳过）。
#[test]
fn declared_entry_size_over_limit_is_rejected() {
    let mut bytes = container(&[("aaaa", &[0u8; 64])]);
    declare_sizes(&mut bytes, "aaaa", 64, 1_000_000);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::EntryTooLarge {
            declared: 1_000_000,
            max: 1024,
        },
    );
}

/// 规范上界在**默认阈值**下生效：声明 2 GB + 1 字节、实际只有 8 字节。
///
/// 判据不 materialize 任何 2 GB 数据 —— 上限判定发生在分配之前（这正是重点）。
#[test]
fn declared_size_just_over_two_gigabytes_is_rejected_with_default_limits() {
    let mut bytes = container(&[("aaaa", &[0u8; 8])]);
    declare_sizes(&mut bytes, "aaaa", 8, 2_000_000_001);
    assert_rejected(
        &bytes,
        &ContainerLimits::default(),
        ContainerError::EntryTooLarge {
            declared: 2_000_000_001,
            max: 2_000_000_000,
        },
    );
}

/// **声明撒谎**：声明的解压体积只有 1 字节，但数据区实际有 4096 字节。
///
/// 只看声明值的实现会放行这个条目（1 ≤ 上限），因此这条判据钉住的是"按**实际写入量**
/// 再判一次"那一行代码；去掉它，判据变红（错误码退化成 `StoredSizeMismatch`）。
#[test]
fn lying_declared_size_is_caught_by_actual_bytes() {
    let mut bytes = container(&[("aaaa", &[0u8; 4096])]);
    declare_sizes(&mut bytes, "aaaa", 4096, 1);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::EntryActualTooLarge {
            actual: 4096,
            max: 1024,
        },
    );
}

/// 累计实际体积超过 `max_total_bytes` ⇒ 拒绝（每条都合法、合起来超限）。
#[test]
fn total_actual_bytes_over_limit_is_rejected() {
    let bytes = container(&[
        ("aaaa", &[1u8; 100]),
        ("bbbb", &[2u8; 100]),
        ("cccc", &[3u8; 100]),
    ]);
    assert_rejected(
        &bytes,
        &limits(200, 250, 100, 64),
        ContainerError::ArchiveTooLarge {
            actual: 300,
            max: 250,
        },
    );
}

/// 整体压缩膨胀比率超过 `max_ratio` ⇒ 拒绝（`MUST-GATE-007` 的第二道）。
#[test]
fn expansion_ratio_over_limit_is_rejected() {
    let mut bytes = container(&[("aaaa", &[0u8; 8])]);
    declare_sizes(&mut bytes, "aaaa", 8, 100_000);
    assert_rejected(
        &bytes,
        &limits(1_000_000, 1_000_000, 100, 64),
        ContainerError::ExpansionRatioExceeded {
            uncompressed: 100_000,
            compressed: 8,
            max_ratio: 100,
        },
    );
}

/// 比率边界必须**精确**：恰好 100:1 放行（随后被"stored 尺寸不一致"拦下），101:1 才由比率闸门拦下。
///
/// 这一对判据把"≤ 100:1"与"> 100:1"的分界线钉在两边的错误码上：
/// 若把比较写成 `>=`，第一条会变成 `ExpansionRatioExceeded`；
/// 若把比率检查删掉，第二条会变成 `StoredSizeMismatch`。
#[test]
fn expansion_ratio_boundary_is_exactly_one_hundred() {
    let mut at_limit = container(&[("aaaa", &[0u8; 8])]);
    declare_sizes(&mut at_limit, "aaaa", 8, 800);
    assert_rejected(
        &at_limit,
        &limits(1_000_000, 1_000_000, 100, 64),
        ContainerError::StoredSizeMismatch {
            index: 0,
            name: "aaaa".into(),
            compressed: 8,
            uncompressed: 800,
        },
    );

    let mut over_limit = container(&[("aaaa", &[0u8; 8])]);
    declare_sizes(&mut over_limit, "aaaa", 8, 801);
    assert_rejected(
        &over_limit,
        &limits(1_000_000, 1_000_000, 100, 64),
        ContainerError::ExpansionRatioExceeded {
            uncompressed: 801,
            compressed: 8,
            max_ratio: 100,
        },
    );
}

/// 条目数超过 `max_entries` ⇒ 拒绝。
#[test]
fn entry_count_over_limit_is_rejected() {
    let bytes = container(&[
        ("a1", b"\x01"),
        ("a2", b"\x02"),
        ("a3", b"\x03"),
        ("a4", b"\x04"),
        ("a5", b"\x05"),
    ]);
    assert_rejected(
        &bytes,
        &limits(1024, 4096, 100, 4),
        ContainerError::TooManyEntries { found: 5, max: 4 },
    );
}

/// 四道可注入上限都是**闭区间**：`== 上限` 放行、`上限 + 1` 拒绝。
///
/// 为什么需要：`src/container/mod.rs` 的模块文档写明"判据用结构体字面量把上限压到
/// 极小的值"，但改动前本文件**全部**上限判据用的都是"超限"一侧（例如 300 > 250）或
/// `ContainerLimits::default()`（2 GB / 8 GB / 4096 条），因此 `>` 与 `>=` 在实现里
/// 不可区分。实测：把 `count > limits.max_entries`、`declared > limits.max_entry_bytes`、
/// `actual > limits.max_entry_bytes`、`actual_total > limits.max_total_bytes` 四条里的
/// 任意一条改成 `>=`，全仓判据保持全绿 —— 四道上限各自被**少放行一格**。
#[test]
fn the_injectable_container_limits_are_inclusive() {
    let sized = |count: usize, size: usize| -> Vec<ContainerEntry> {
        (0..count)
            .map(|index| ContainerEntry::new(format!("e{index}"), vec![0x5a; size]))
            .collect()
    };

    // 条目数：恰好等于上限放行，多一条拒绝。
    let two = write_container(&sized(2, 1)).expect("写两条");
    assert!(
        read_container(&two, &limits(64, 64, 100, 2)).is_ok(),
        "条目数 == max_entries 必须放行"
    );
    assert_rejected(
        &two,
        &limits(64, 64, 100, 1),
        ContainerError::TooManyEntries { found: 2, max: 1 },
    );

    // 单条目字节：恰好等于上限放行（`declared` 与 `actual` 两道都走这一格），
    // 少一个字节就拒绝。
    let one = write_container(&sized(1, 8)).expect("写一条 8 字节");
    assert!(
        read_container(&one, &limits(8, 64, 100, 4)).is_ok(),
        "单条字节 == max_entry_bytes 必须放行"
    );
    assert_rejected(
        &one,
        &limits(7, 64, 100, 4),
        ContainerError::EntryTooLarge {
            declared: 8,
            max: 7,
        },
    );

    // 全归档总体积：恰好等于上限放行，少一个字节就拒绝。
    let three = write_container(&sized(3, 4)).expect("写三条 4 字节");
    assert!(
        read_container(&three, &limits(64, 12, 100, 8)).is_ok(),
        "总体积 == max_total_bytes 必须放行"
    );
    assert_rejected(
        &three,
        &limits(64, 11, 100, 8),
        ContainerError::ArchiveTooLarge {
            actual: 12,
            max: 11,
        },
    );
}

/// 阈值**可注入**：同一个 64 字节的归档，默认阈值放行，注入极小阈值后逐条触发。
#[test]
fn limits_are_injectable_on_tiny_data() {
    let bytes = container(&[("aaaa", &[0u8; 64])]);
    assert!(
        read_container(&bytes, &ContainerLimits::default()).is_ok(),
        "默认阈值下必须放行"
    );
    assert_rejected(
        &bytes,
        &limits(32, 4096, 100, 64),
        ContainerError::EntryTooLarge {
            declared: 64,
            max: 32,
        },
    );
    assert_rejected(
        &bytes,
        &limits(1024, 4096, 100, 0),
        ContainerError::TooManyEntries { found: 1, max: 0 },
    );
    assert_rejected(
        &bytes,
        &limits(1024, 32, 100, 64),
        ContainerError::ArchiveTooLarge {
            actual: 64,
            max: 32,
        },
    );
}

// ======================================================================
// 结构畸形
// ======================================================================

/// 截断的 EOCD。
#[test]
fn truncated_eocd_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    bytes.truncate(bytes.len() - 5);
    assert_rejected(&bytes, &small_limits(), ContainerError::EocdNotFound);
}

/// central directory 偏移越界。
#[test]
fn central_directory_offset_out_of_bounds_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let eocd = find_eocd(&bytes);
    put_u32(&mut bytes, eocd + EOCD_CENTRAL_OFFSET, 1_000_000);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::CentralDirectoryOutOfBounds,
    );
}

/// central directory 声明长度与实际解析消耗不符（在 CD 与 EOCD 之间塞入垃圾字节）。
#[test]
fn central_directory_size_lie_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let eocd = find_eocd(&bytes);
    let declared = read_u32(&bytes, eocd + EOCD_CENTRAL_SIZE);
    bytes.splice(eocd..eocd, [0u8; 4]);
    let eocd = find_eocd(&bytes);
    put_u32(&mut bytes, eocd + EOCD_CENTRAL_SIZE, declared + 4);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::CentralDirectorySizeMismatch {
            declared: declared as usize + 4,
            actual: declared as usize,
        },
    );
}

/// EOCD 的条目数撒谎（声明 3 条，实际只有 2 条）。
#[test]
fn entry_count_lie_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01"), ("bbbb", b"\x02")]);
    let eocd = find_eocd(&bytes);
    put_u16(&mut bytes, eocd + EOCD_ENTRIES_ON_DISK, 3);
    put_u16(&mut bytes, eocd + EOCD_TOTAL_ENTRIES, 3);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::TruncatedCentralDirectory { index: 2 },
    );
}

/// CRC 不匹配（数据被篡改）。
#[test]
fn crc_mismatch_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01\x02\x03\x04")]);
    let data = data_start(&bytes, "aaaa");
    let archived_crc = read_u32(&bytes, find_central(&bytes, "aaaa") + CENTRAL_CRC);
    bytes[data] ^= 0xFF;
    match read_container(&bytes, &small_limits()) {
        Err(ContainerError::CrcMismatch {
            index,
            name,
            declared,
            actual,
        }) => {
            assert_eq!(index, 0);
            assert_eq!(name, "aaaa");
            assert_eq!(declared, archived_crc);
            assert_ne!(declared, actual, "篡改后的字节必须算出不同的 CRC");
        }
        other => panic!("期望 CrcMismatch，实际 {other:?}"),
    }
}

/// `local header` 与 `central directory` 的名字不一致 —— ZIP 的真实攻击面。
///
/// 本实现的裁决是"两处都必须一致"：不一致即拒绝，没有"以谁为准"的歧义。
#[test]
fn local_central_name_mismatch_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let local = find_local(&bytes, "aaaa");
    bytes[local + LOCAL_NAME..local + LOCAL_NAME + 4].copy_from_slice(b"bbbb");
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::LocalCentralMismatch {
            index: 0,
            name: "aaaa".into(),
        },
    );
}

/// `local header` 与 `central directory` 的尺寸不一致（同样按拒绝处理）。
#[test]
fn local_central_size_mismatch_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01\x02")]);
    let local = find_local(&bytes, "aaaa");
    put_u32(&mut bytes, local + LOCAL_UNCOMPRESSED, 4);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::LocalCentralMismatch {
            index: 0,
            name: "aaaa".into(),
        },
    );
}

/// `stored` 条目的压缩前后尺寸自相矛盾。
#[test]
fn stored_size_mismatch_is_rejected() {
    let mut bytes = container(&[("aaaa", &[0u8; 8])]);
    declare_sizes(&mut bytes, "aaaa", 8, 16);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::StoredSizeMismatch {
            index: 0,
            name: "aaaa".into(),
            compressed: 8,
            uncompressed: 16,
        },
    );
}

/// `deflate`(method 8) 必须**明确报错**，不许静默跳过或猜测。
#[test]
fn deflate_method_is_rejected_as_unsupported() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let local = find_local(&bytes, "aaaa");
    put_u16(&mut bytes, local + LOCAL_METHOD, 8);
    let central = find_central(&bytes, "aaaa");
    put_u16(&mut bytes, central + CENTRAL_METHOD, 8);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::UnsupportedCompression {
            index: 0,
            method: 8,
        },
    );
}

/// 加密条目（通用位标志 bit 0）。
#[test]
fn encrypted_entry_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let local = find_local(&bytes, "aaaa");
    put_u16(&mut bytes, local + LOCAL_FLAGS, 0x0801);
    let central = find_central(&bytes, "aaaa");
    put_u16(&mut bytes, central + CENTRAL_FLAGS, 0x0801);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::EncryptedEntryUnsupported {
            index: 0,
            name: "aaaa".into(),
        },
    );
}

/// data descriptor（通用位标志 bit 3）：尺寸写在数据之后，本地头部不可信 ⇒ 拒绝。
#[test]
fn data_descriptor_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let local = find_local(&bytes, "aaaa");
    put_u16(&mut bytes, local + LOCAL_FLAGS, 0x0808);
    let central = find_central(&bytes, "aaaa");
    put_u16(&mut bytes, central + CENTRAL_FLAGS, 0x0808);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::UnsupportedDataDescriptor { index: 0 },
    );
}

/// EOCD 里的 ZIP64 哨兵（条目数 `0xFFFF`）。
#[test]
fn zip64_sentinel_in_eocd_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let eocd = find_eocd(&bytes);
    put_u16(&mut bytes, eocd + EOCD_ENTRIES_ON_DISK, 0xFFFF);
    put_u16(&mut bytes, eocd + EOCD_TOTAL_ENTRIES, 0xFFFF);
    assert_rejected(&bytes, &small_limits(), ContainerError::UnsupportedZip64);
}

/// central directory 里的 ZIP64 哨兵（尺寸 `0xFFFFFFFF`）。
#[test]
fn zip64_sentinel_in_central_record_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let central = find_central(&bytes, "aaaa");
    put_u32(&mut bytes, central + CENTRAL_UNCOMPRESSED, 0xFFFF_FFFF);
    assert_rejected(&bytes, &small_limits(), ContainerError::UnsupportedZip64);
}

/// 多卷 / 跨盘归档。
#[test]
fn multi_disk_archive_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let eocd = find_eocd(&bytes);
    put_u16(&mut bytes, eocd + EOCD_DISK, 1);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::UnsupportedMultiDisk,
    );
}

/// central directory 记录签名损坏。
#[test]
fn bad_central_directory_signature_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let central = find_central(&bytes, "aaaa");
    put_u32(&mut bytes, central, 0xDEAD_BEEF);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::BadCentralDirectorySignature {
            index: 0,
            found: 0xDEAD_BEEF,
        },
    );
}

/// local file header 签名损坏。
#[test]
fn bad_local_header_signature_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let local = find_local(&bytes, "aaaa");
    put_u32(&mut bytes, local, 0);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::BadLocalHeaderSignature { index: 0, found: 0 },
    );
}

/// local header 偏移越界。
#[test]
fn local_header_offset_out_of_bounds_is_rejected() {
    let mut bytes = container(&[("aaaa", b"\x01")]);
    let central = find_central(&bytes, "aaaa");
    put_u32(&mut bytes, central + CENTRAL_LOCAL_OFFSET, 1_000_000);
    assert_rejected(
        &bytes,
        &small_limits(),
        ContainerError::LocalHeaderOutOfBounds { index: 0 },
    );
}

/// 条目数据区被截断（声明尺寸大于实际存在的字节）。
#[test]
fn truncated_entry_data_is_rejected() {
    let mut bytes = container(&[("aaaa", &[0u8; 8])]);
    declare_sizes(&mut bytes, "aaaa", 4096, 4096);
    assert_rejected(
        &bytes,
        &limits(1 << 20, 1 << 20, 100, 64),
        ContainerError::TruncatedEntryData { index: 0 },
    );
}
