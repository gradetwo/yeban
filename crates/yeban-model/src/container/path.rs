//! `MUST-GATE-006`：归档条目目标路径的规范化与拒绝判定。
//!
//! # 规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3 / `ARCH-SEC-003`）
//!
//! > **Zip-Slip 路径遍历防御 (MUST)** ：解包与导入 `.yeban` 归档时，必须对每个 Zip 内部条目的
//! > 目标路径进行 `canonicalize()` 规范化检查，严禁包含 `..`、绝对根路径或跨卷符号链接，
//! > 杜绝路径穿越任意文件覆盖漏洞。
//!
//! # 本实现如何把"canonicalize()"落成可判定的纯函数
//!
//! 真正的 `std::fs::canonicalize()` 要求路径**已经存在于文件系统上**，因此在"决定是否解包"
//! 这一步根本不可用（解包前的目标文件还不存在），而且它把判定交给了 OS 的具体规范化规则
//! （macOS 的 HFS+/APFS 会做 Unicode 分解与忽略大小写，Windows 会剥掉尾随点与空格）——
//! 三个平台会给出三个不同的答案。安全边界不能是"看平台"。
//!
//! 因此这里做**纯语法层**的规范化判定，并且采用比"改写"更强的策略：
//!
//! **接受 == 原样，拒绝 == 报错；绝不静默重写危险名字。**
//!
//! 为什么不"修正"（例如把 `a/../../b` 折叠成 `b`）：把恶意条目重写成合法条目，等于把一个
//! 攻击信号变成一个**看起来正常**的条目——调用方再也无法区分"归档本来就是这样"和"我们
//! 改过它"。规范要求"100% 拦截"，拦截的定义就是拒绝。这也让本函数成为一个**全函数**：
//! 它的接受集合是安全的、封闭的（每个被接受的输出都等于输入，且不含任何危险构造）。
//!
//! # 判定清单（每一条都有对应判据）
//!
//! | 构造 | 判据 | 依据 |
//! | :--- | :--- | :--- |
//! | 空名字 | [`ContainerError::EmptyEntryName`] | 不是合法文件名 |
//! | 超过 4096 字节 | [`ContainerError::EntryNameTooLong`] | 内存放大防护 |
//! | 以 `/` 开头（含 `//server/share`） | [`ContainerError::AbsoluteEntryPath`] | 绝对根路径 |
//! | 含 `..` 段 | [`ContainerError::ParentDirSegment`] | Zip-Slip 本体 |
//! | 含 `.` 段 | [`ContainerError::CurrentDirSegment`] | 规范化歧义 |
//! | 含空段（`a//b`、`a/`、`/a`） | [`ContainerError::EmptyPathSegment`] | 空段在不同平台被折叠 |
//! | 以 `/` 结尾（目录条目） | [`ContainerError::DirectoryEntryUnsupported`] | `.yeban` 只存文件 |
//! | 含 `\` | [`ContainerError::BackslashInEntryName`] | Windows 上是分隔符 |
//! | 含 `:` | [`ContainerError::ColonInEntryName`] | `C:\`、`C:rel`、NTFS 数据流 |
//! | 含 NUL | [`ContainerError::NulInEntryName`] | C 字符串截断 |
//! | 含其它 C0 控制字符 / DEL | [`ContainerError::ControlCharInEntryName`] | 终端与 shell 注入 |
//! | 段以 `.` 或空格结尾 | [`ContainerError::TrailingDotOrSpaceSegment`] | Windows 会剥离 ⇒ `".. "` → `..` |
//! | 段是 Windows 设备名（`CON`/`NUL`/`COM1`…） | [`ContainerError::WindowsReservedName`] | 写设备而非文件 |
//! | 大小写变体重名 | [`ContainerError::DuplicateEntryName`] | APFS/NTFS 上互相覆盖 |
//!
//! # 明确**不**做的事（见 `docs/ledger/container-notes.md` §未实现项）
//!
//! - 不做百分号解码：`..%2f` 是**字面文件名**，不是路径。ZIP 规范没有百分号解码这一步，
//!   任何"先解码再判定"的读者才会中招。本函数把它当普通名字接受（判据钉住这一点）。
//! - 不做 Unicode 规范化（NFC/NFD）与全角字符映射：`．` (U+FF0E) 不是 `.`，它是**字面字符**。
//!   我们没有引入 `unicode-normalization`（零依赖目标），因此把"两个不同码位在 APFS 上折叠成
//!   同一个文件"记为 `pending`（见笔记），而不是假装已覆盖。
//! - 不处理 Windows 上标数字别名（`COM¹` ≡ `COM1`）：`.yeban` 写入器只产出 ASCII 名字，
//!   这条留作 `pending`。

use super::ContainerError;

/// 单个条目名的字节上限。
///
/// ZIP32 的文件名字段本身可到 65535 字节，但 `.yeban` 只用 `project.json`、`history.dag`、
/// `assets/<64 hex>` 三种形状（最长的 `assets/` + 64 = 71 字节）。4096 是一个远大于真实需求、
/// 又足以拦住"用超长名字做内存放大"的上限。
pub const MAX_ENTRY_NAME_BYTES: usize = 4096;

/// Windows 保留设备名（大小写不敏感；含扩展名时按主干匹配，因为 `CON.txt` 同样是设备）。
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 把 ZIP 条目的原始名字判定为**安全相对路径**，或返回拒绝原因。
///
/// 返回的 `String` 保证：
/// 1. 非空、长度 ≤ [`MAX_ENTRY_NAME_BYTES`]；
/// 2. 只含 `/` 作为分隔符，且不含空段、`.` 段、`..` 段；
/// 3. 不以 `/` 开头（相对路径），不含 `\`、`:`、NUL 或任何控制字符；
/// 4. 每个段都不以 `.` / 空格结尾，且不是 Windows 保留设备名。
///
/// 当前实现对**被接受**的输入返回与输入逐字节相同的字符串（见模块文档"接受 == 原样"）。
///
/// # Errors
///
/// 上述任一构造命中时返回对应的 [`ContainerError`] 变体。
pub fn normalize_entry_name(raw: &str) -> Result<String, ContainerError> {
    if raw.is_empty() {
        return Err(ContainerError::EmptyEntryName);
    }
    if raw.len() > MAX_ENTRY_NAME_BYTES {
        return Err(ContainerError::EntryNameTooLong {
            len: raw.len(),
            max: MAX_ENTRY_NAME_BYTES,
        });
    }
    if raw.starts_with('/') {
        return Err(ContainerError::AbsoluteEntryPath {
            name: raw.to_owned(),
        });
    }
    if raw.ends_with('/') {
        return Err(ContainerError::DirectoryEntryUnsupported {
            name: raw.to_owned(),
        });
    }
    if raw.contains('\0') {
        return Err(ContainerError::NulInEntryName {
            name: raw.to_owned(),
        });
    }
    if raw.contains('\\') {
        return Err(ContainerError::BackslashInEntryName {
            name: raw.to_owned(),
        });
    }
    if raw.contains(':') {
        return Err(ContainerError::ColonInEntryName {
            name: raw.to_owned(),
        });
    }
    if raw.chars().any(char::is_control) {
        return Err(ContainerError::ControlCharInEntryName {
            name: raw.to_owned(),
        });
    }

    let mut normalized = String::with_capacity(raw.len());
    for segment in raw.split('/') {
        if segment.is_empty() {
            return Err(ContainerError::EmptyPathSegment {
                name: raw.to_owned(),
            });
        }
        if segment == "." {
            return Err(ContainerError::CurrentDirSegment {
                name: raw.to_owned(),
            });
        }
        if segment == ".." {
            return Err(ContainerError::ParentDirSegment {
                name: raw.to_owned(),
            });
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err(ContainerError::TrailingDotOrSpaceSegment {
                name: raw.to_owned(),
            });
        }
        if is_windows_reserved_device(segment) {
            return Err(ContainerError::WindowsReservedName {
                name: raw.to_owned(),
            });
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(segment);
    }
    Ok(normalized)
}

/// 段的主干（第一个 `.` 之前的部分）是否为 Windows 保留设备名。
fn is_windows_reserved_device(segment: &str) -> bool {
    let stem = match segment.split('.').next() {
        Some(stem) => stem,
        None => segment,
    };
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    use super::{MAX_ENTRY_NAME_BYTES, normalize_entry_name};
    use crate::container::ContainerError;

    fn accept(name: &str) {
        assert_eq!(
            normalize_entry_name(name).as_deref(),
            Ok(name),
            "应当被接受: {name:?}"
        );
    }

    fn reject(name: &str, expected: ContainerError) {
        assert_eq!(
            normalize_entry_name(name),
            Err(expected.clone()),
            "应当被拒绝: {name:?}"
        );
    }

    #[test]
    fn accepts_the_three_container_shapes() {
        accept("project.json");
        accept("history.dag");
        accept("assets/0000000000000000000000000000000000000000000000000000000000000000");
    }

    #[test]
    fn rejects_the_classic_zip_slip_shapes() {
        reject(
            "../evil",
            ContainerError::ParentDirSegment {
                name: "../evil".into(),
            },
        );
        reject(
            "../../etc/passwd",
            ContainerError::ParentDirSegment {
                name: "../../etc/passwd".into(),
            },
        );
        reject("..", ContainerError::ParentDirSegment { name: "..".into() });
        reject(
            "a/../../b",
            ContainerError::ParentDirSegment {
                name: "a/../../b".into(),
            },
        );
    }

    #[test]
    fn rejects_absolute_and_cross_volume_paths() {
        reject(
            "/abs/path",
            ContainerError::AbsoluteEntryPath {
                name: "/abs/path".into(),
            },
        );
        reject(
            "//server/share/x",
            ContainerError::AbsoluteEntryPath {
                name: "//server/share/x".into(),
            },
        );
        reject(
            "C:\\abs",
            ContainerError::BackslashInEntryName {
                name: "C:\\abs".into(),
            },
        );
        reject(
            "C:rel",
            ContainerError::ColonInEntryName {
                name: "C:rel".into(),
            },
        );
        reject(
            "/etc/shadow",
            ContainerError::AbsoluteEntryPath {
                name: "/etc/shadow".into(),
            },
        );
    }

    #[test]
    fn rejects_empty_segments_and_directory_entries() {
        reject(
            "a//b",
            ContainerError::EmptyPathSegment {
                name: "a//b".into(),
            },
        );
        reject(
            "assets/",
            ContainerError::DirectoryEntryUnsupported {
                name: "assets/".into(),
            },
        );
        reject(
            "./project.json",
            ContainerError::CurrentDirSegment {
                name: "./project.json".into(),
            },
        );
        reject(
            "a/./b",
            ContainerError::CurrentDirSegment {
                name: "a/./b".into(),
            },
        );
    }

    #[test]
    fn rejects_nul_and_control_characters() {
        reject(
            "a\0b",
            ContainerError::NulInEntryName {
                name: "a\0b".into(),
            },
        );
        reject(
            "a\nb",
            ContainerError::ControlCharInEntryName {
                name: "a\nb".into(),
            },
        );
        reject(
            "a\u{7f}b",
            ContainerError::ControlCharInEntryName {
                name: "a\u{7f}b".into(),
            },
        );
    }

    #[test]
    fn rejects_windows_trailing_dot_or_space_bypass() {
        // Windows 会剥掉段尾的点与空格 ⇒ `".. "` 在 Windows 上等价于 `..`。
        reject(
            ".. ",
            ContainerError::TrailingDotOrSpaceSegment { name: ".. ".into() },
        );
        reject(
            "a/.. ",
            ContainerError::TrailingDotOrSpaceSegment {
                name: "a/.. ".into(),
            },
        );
        reject(
            "...",
            ContainerError::TrailingDotOrSpaceSegment { name: "...".into() },
        );
        reject(
            "a./b",
            ContainerError::TrailingDotOrSpaceSegment {
                name: "a./b".into(),
            },
        );
        reject(
            "a /b",
            ContainerError::TrailingDotOrSpaceSegment {
                name: "a /b".into(),
            },
        );
    }

    #[test]
    fn rejects_windows_reserved_device_names() {
        for name in ["NUL", "nul", "Con", "COM1", "lpt9", "NUL.txt", "aux.json"] {
            assert!(
                matches!(
                    normalize_entry_name(name),
                    Err(ContainerError::WindowsReservedName { .. })
                ),
                "应当按设备名拒绝: {name:?}"
            );
        }
        // 前缀相近但不是设备名的，必须放行（否则判据会变成"什么都拒绝"的假绿）。
        accept("CONSOLE.txt");
        accept("COM10");
        accept("nullable");
    }

    #[test]
    fn rejects_over_long_names() {
        let long = "a".repeat(MAX_ENTRY_NAME_BYTES + 1);
        assert_eq!(
            normalize_entry_name(&long),
            Err(ContainerError::EntryNameTooLong {
                len: MAX_ENTRY_NAME_BYTES + 1,
                max: MAX_ENTRY_NAME_BYTES,
            })
        );
        accept(&"a".repeat(MAX_ENTRY_NAME_BYTES));
    }
}
