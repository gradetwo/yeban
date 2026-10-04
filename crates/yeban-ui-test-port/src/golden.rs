//! Golden 图目录约定与**分平台严禁混用**检查 —— `[UI-MCP-003]`。
//!
//! 规范原文（UI/UX §12.5 第 3 条）：Linux (FreeType)、macOS (CoreText) 与
//! Windows (DirectWrite) 的底层字体光栅化与亚像素抗锯齿存在微弱差异，
//! CI 视觉回归测试**严禁跨平台混用同一张 Golden 图**，必须按操作系统独立维护基准图集。
//!
//! 因此本模块把"约定"变成两件**机械可判定**的东西：
//! 1. 路径形状：`tests/golden/<platform>/<name>.png`；
//! 2. 校验函数 [`check_platform_tag`]：把"这张基准图属于哪个平台"与"当前跑在哪个平台"
//!    对上，不一致就**报错**（而不是"差不多也能过"）。
//!
//! ## 诚实边界
//!
//! 本模块只做**命名与归属**的检查，不做像素比对本身（那是 `ssim.rs` + `mask.rs` 的事）。
//! 它证明不了"基准图内容是对的" —— 那需要有人真的在对应平台上生成并提交基准图，
//! 属于 needs（见 `docs/ledger/ui-test-port-notes.md`）。

use std::path::{Component, Path, PathBuf};

/// Golden 图库的根目录（相对仓库根）。
pub const GOLDEN_ROOT: &str = "tests/golden";

/// Golden 图所属平台。取值即目录名。
///
/// 只登记三个有规范依据的平台（§12.5 逐字点名 Linux / macOS / Windows）。
/// `target_os` 的其它取值（如 `freebsd`）会落到 [`PlatformTag::Other`]，
/// 它**没有**独立目录 —— 在这种平台上跑视觉回归必须显式失败，而不是借用别的平台的图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlatformTag {
    /// Linux：字体走 FreeType/fontconfig。
    Linux,
    /// macOS：字体走 CoreText。
    Macos,
    /// Windows：字体走 DirectWrite。
    Windows,
    /// 其它平台：按 §12.5 没有基准图集，视觉回归不得静默通过与跳过。
    Other,
}

impl PlatformTag {
    /// 当前编译目标所属的平台标签。
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }

    /// 目录名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Other => "other",
        }
    }

    /// 从目录名解析。
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "linux" => Some(Self::Linux),
            "macos" => Some(Self::Macos),
            "windows" => Some(Self::Windows),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    /// 该平台是否有规范意义上的独立基准图集。
    #[must_use]
    pub const fn has_reference_library(self) -> bool {
        matches!(self, Self::Linux | Self::Macos | Self::Windows)
    }
}

/// 目录 / 归属检查的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoldenError {
    /// 文件名不合法（空、含路径分隔符、非 `.png`）。
    BadName {
        /// 原始输入。
        name: String,
    },
    /// 路径不符合 `tests/golden/<platform>/<name>.png` 的形状。
    NotAPlatformScopedPath {
        /// 出错的路径。
        path: String,
    },
    /// 基准图所属平台与当前平台不一致 —— `[UI-MCP-003]` 明文禁止跨平台混用。
    CrossPlatformGolden {
        /// 基准图所属平台。
        golden: PlatformTag,
        /// 当前平台。
        current: PlatformTag,
    },
    /// 当前平台没有规范基准图集（例如 `Other`）。
    NoReferenceLibraryForPlatform {
        /// 当前平台。
        current: PlatformTag,
    },
}

impl core::fmt::Display for GoldenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadName { name } => write!(
                f,
                "Golden 名非法: `{name}` (只允许不含路径分隔符的 `<name>.png`)"
            ),
            Self::NotAPlatformScopedPath { path } => write!(
                f,
                "Golden 路径必须形如 `{GOLDEN_ROOT}/<platform>/<name>.png`, 实际 `{path}` \
                 [UI-MCP-003]"
            ),
            Self::CrossPlatformGolden { golden, current } => write!(
                f,
                "跨平台 Golden 混用: 基准图属于 `{}`, 当前平台是 `{}` —— \
                 [UI-MCP-003] 严禁跨平台混用同一张基准图 (字体光栅化差异)",
                golden.as_str(),
                current.as_str()
            ),
            Self::NoReferenceLibraryForPlatform { current } => write!(
                f,
                "平台 `{}` 没有规范基准图集, 视觉回归不得静默跳过 [UI-MCP-003]",
                current.as_str()
            ),
        }
    }
}

impl core::error::Error for GoldenError {}

/// 组装某个 Golden 图的**仓库相对路径**：`tests/golden/<platform>/<name>.png`。
///
/// `platform` 通常传 [`PlatformTag::current`]；显式传入是为了让"故意要另一平台"的场景
/// 只能走显式调用，而不是靠默认值漂移。
pub fn golden_path(platform: PlatformTag, name: &str) -> Result<PathBuf, GoldenError> {
    validate_name(name)?;
    Ok(Path::new(GOLDEN_ROOT)
        .join(platform.as_str())
        .join(format!("{name}.png")))
}

/// 从既有路径里读出它所属的平台标签。
pub fn platform_of(path: &Path) -> Result<PlatformTag, GoldenError> {
    let components: Vec<&str> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    // 期望结尾是 [..., "golden", "<platform>", "<name>.png"]
    if components.len() < 3 {
        return Err(GoldenError::NotAPlatformScopedPath {
            path: display(path),
        });
    }
    let platform_at = components.len() - 2;
    if components[platform_at - 1] != "golden" {
        return Err(GoldenError::NotAPlatformScopedPath {
            path: display(path),
        });
    }
    let tag = PlatformTag::parse(components[platform_at]).ok_or_else(|| {
        GoldenError::NotAPlatformScopedPath {
            path: display(path),
        }
    })?;
    let file = components[components.len() - 1];
    if !file.ends_with(".png") {
        return Err(GoldenError::NotAPlatformScopedPath {
            path: display(path),
        });
    }
    Ok(tag)
}

/// `[UI-MCP-003]` 的核心判据：基准图的平台标签必须等于当前平台。
///
/// 当前平台没有基准图集时（[`PlatformTag::Other`]）也返回错误：
/// "这个平台不跑视觉回归"必须是**显式**的失败，而不是静默通过。
pub fn check_platform_tag(path: &Path) -> Result<PlatformTag, GoldenError> {
    let current = PlatformTag::current();
    if !current.has_reference_library() {
        return Err(GoldenError::NoReferenceLibraryForPlatform { current });
    }
    let golden = platform_of(path)?;
    if golden != current {
        return Err(GoldenError::CrossPlatformGolden { golden, current });
    }
    Ok(golden)
}

/// 完整的"取基准图 + 归属校验"入口：任何调用方都不必记得先校验再使用。
pub fn reference_for_current_platform(name: &str) -> Result<PathBuf, GoldenError> {
    let path = golden_path(PlatformTag::current(), name)?;
    check_platform_tag(&path)?;
    Ok(path)
}

fn validate_name(name: &str) -> Result<(), GoldenError> {
    let bad = name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.ends_with(".png")
        || name.contains("..")
        || name.contains(char::is_whitespace);
    if bad {
        return Err(GoldenError::BadName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 1: 路径形状必须严格是 `tests/golden/<platform>/<name>.png`。
    #[test]
    fn golden_path_has_the_documented_shape() {
        assert_eq!(
            golden_path(PlatformTag::Linux, "arrangement-1920x1080"),
            Ok(PathBuf::from(
                "tests/golden/linux/arrangement-1920x1080.png"
            ))
        );
        assert_eq!(
            golden_path(PlatformTag::current(), "piano-roll"),
            Ok(Path::new(GOLDEN_ROOT)
                .join(PlatformTag::current().as_str())
                .join("piano-roll.png"))
        );
    }

    /// 判据 2: 非法名字必须被拒绝（否则会写到别的平台上、或写出非 PNG）。
    #[test]
    fn bad_names_are_rejected() {
        for bad in [
            "",
            "../escape",
            "sub/dir",
            "sub\\dir",
            "already.png",
            "with space",
        ] {
            assert_eq!(
                golden_path(PlatformTag::Linux, bad),
                Err(GoldenError::BadName {
                    name: bad.to_owned()
                }),
                "非法名字 `{bad}`"
            );
        }
    }

    /// 判据 3: **`[UI-MCP-003]` 的核心** —— 别的平台的基准图必须被拒绝，本平台的必须通过。
    ///
    /// 变异测试：把 [`check_platform_tag`] 的 `golden != current` 改成 `false`，本判据变红。
    #[test]
    fn cross_platform_goldens_are_rejected() {
        let current = PlatformTag::current();
        assert!(
            current.has_reference_library(),
            "本机/CI 必须是三大平台之一"
        );

        let own = golden_path(current, "arrangement").expect("合法路径");
        assert_eq!(check_platform_tag(&own), Ok(current));

        for other in [PlatformTag::Linux, PlatformTag::Macos, PlatformTag::Windows] {
            let path = golden_path(other, "arrangement").expect("合法路径");
            if other == current {
                assert_eq!(check_platform_tag(&path), Ok(current));
            } else {
                assert_eq!(
                    check_platform_tag(&path),
                    Err(GoldenError::CrossPlatformGolden {
                        golden: other,
                        current
                    }),
                    "`{}` 的基准图不得在当前平台 `{}` 上使用",
                    other.as_str(),
                    current.as_str()
                );
            }
        }
    }

    /// 判据 4: 不符合形状的路径必须报错，而不是被"猜"成某个平台。
    #[test]
    fn malformed_paths_are_rejected() {
        for bad in [
            "arrangement.png",
            "tests/golden/arrangement.png",
            "tests/goldens/linux/arrangement.png",
            "tests/golden/bsd/arrangement.png",
            "tests/golden/linux/arrangement.jpg",
        ] {
            assert!(
                matches!(
                    platform_of(Path::new(bad)),
                    Err(GoldenError::NotAPlatformScopedPath { .. })
                ),
                "`{bad}` 必须被拒绝"
            );
        }
        assert_eq!(
            platform_of(Path::new("tests/golden/linux/a.png")),
            Ok(PlatformTag::Linux)
        );
        assert_eq!(
            platform_of(Path::new("tests/golden/macos/a.png")),
            Ok(PlatformTag::Macos)
        );
        assert_eq!(
            platform_of(Path::new("tests/golden/windows/a.png")),
            Ok(PlatformTag::Windows)
        );
    }

    /// 判据 5: `other` 平台没有基准图集 —— 视觉回归必须显式失败，不得静默跳过。
    #[test]
    fn platforms_without_a_reference_library_fail_explicitly() {
        assert!(!PlatformTag::Other.has_reference_library());
        assert_eq!(PlatformTag::parse("other"), Some(PlatformTag::Other));
        assert_eq!(PlatformTag::parse("freebsd"), None);

        // 通过 reference_for_current_platform 走完整条路径：本平台上必须拿到本平台的路径。
        let path = reference_for_current_platform("smoke").expect("当前平台应当有基准图集");
        assert_eq!(platform_of(&path), Ok(PlatformTag::current()));
    }
}
