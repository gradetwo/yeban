//! ⭐ 任务单 `#271`：采样器路径的**根**推导 —— 与 app 的 `sfz_load.rs` / `samples.rs`
//! **同口径**。
//!
//! ## 为什么这里有一份"重算"而不是 `use yeban_app`
//!
//! 依赖方向门 `scripts/gates/check_mcp_dependency_direction.py` 的 `FORBIDDEN` 逐字含
//! `yeban-app`（带 GUI），因此 `yeban-mcp` **不能**调 app 的实现。两条候选路径：
//!
//! 1. **下移**到双方都能依赖的低层（`yeban-sfz`）：MCP 已经依赖它，不新增边；但要
//!    真正消灭第二份真相，**app 也必須**改成调用它 —— app 不在本单可改面内
//!    （`crates/yeban-mcp/**` ＋「若下移」`crates/yeban-sfz/**`），只下移一半等于把
//!    重复从两份变成两份＋一份公共 API，收益为零；
//! 2. **在 MCP 内重算**：两份实现并存的代价由判据兜住（见下）。
//!
//! 本单选 2。**同口径**不是"看起来一样"，判据有三条：
//!
//! - 三平台字面量：本模块的 `preset_library_root` 与 app 的
//!   `samples::preset_library_root` 入参一一对应（platform ＋ home/xdg/appdata 三个
//!   `Option`），`tests/sampler_path_roots.rs` 钉住与 app 的
//!   `tests/sample_library.rs` **同一组**期望字面量；
//! - 环境变量覆盖：`YEBAN_SAMPLE_LIBRARY` 非空 ⇒ 原样返回（⛔ 不规范化），
//!   `tests/sampler_path_roots.rs` 的谓词路径逐字等于该变量值；
//! - 运行期对拍：同一 `YEBAN_SAMPLE_LIBRARY` 下 `yeban-app --probe-instruments` 打印的
//!   `lib: <库根>` 与 MCP 的库根候选逐字相同（读数见任务报告）。
//!
//! ## 与 app 逐条对应的来源
//!
//! | 本模块 | app 的唯一实现 |
//! | :--- | :--- |
//! | [`library_root`] | `crates/yeban-app/src/samples.rs:294` 的 `library_root()` |
//! | [`preset_library_root`] | `crates/yeban-app/src/samples.rs:262` 的 `preset_library_root(..)` |
//! | [`repo_root`] | `crates/yeban-app/src/sfz_load.rs:984` 的 `repo_root()` |
//!
//! ⚠ **每次调用都重读环境变量**（与 app 同一条：判据在同一进程里换临时库根，
//! 且用户在会话中改环境变量不该被上一份缓存盖住）；`repo_root` 用 `OnceLock`
//! 缓存（与 app 同一条：它只由编译期常量决定，不会随环境变量变）。

use std::path::{Path, PathBuf};

/// 采样库根的环境变量名（与 app 的 `samples::SAMPLE_LIBRARY_ENV` 逐字相同）。
pub(crate) const SAMPLE_LIBRARY_ENV: &str = "YEBAN_SAMPLE_LIBRARY";

/// 预设库根的**平台形状**（判据要在同一台机器上读三档读数）。
///
/// 与 app 的 `samples::LibraryPlatform` 一一对应；`cfg!` 只出现在 [`Self::current`] 一处。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryPlatform {
    /// macOS：`<home>/Library/Application Support/dev.yeban.app/Samples`。
    MacOs,
    /// Linux：`$XDG_DATA_HOME/yeban/samples`，缺省 `<home>/.local/share/yeban/samples`。
    Linux,
    /// Windows：`%APPDATA%\yeban\samples`。
    Windows,
}

impl LibraryPlatform {
    /// 当前进程所在平台的形状（`cfg!` 只在这里出现一次，与 app 同一条）。
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// ⭐ **预设库根的唯一公式**（纯函数：判据可在任意平台上读三档读数）。
///
/// 逐条照 `yeban-app/src/samples.rs:262` 的 `preset_library_root`：
///
/// | 平台 | 主变量 | 缺省 |
/// | :--- | :--- | :--- |
/// | macOS | `home` | ——（没有 home ⇒ 相对路径 `<cwd>/Library/Application Support/dev.yeban.app/Samples`） |
/// | Linux | `xdg_data_home` | `<home>/.local/share`；没有 home ⇒ `yeban/samples` |
/// | Windows | `appdata` | ——（没有 `%APPDATA%` ⇒ 相对路径 `yeban/samples`） |
///
/// ⚠ 返回值**不创建目录**、⛔ 不查存在性：它只是路径公式。
pub(crate) fn preset_library_root(
    platform: LibraryPlatform,
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    appdata: Option<&Path>,
) -> PathBuf {
    match platform {
        LibraryPlatform::MacOs => home
            .map_or_else(PathBuf::new, Path::to_path_buf)
            .join("Library/Application Support/dev.yeban.app/Samples"),
        LibraryPlatform::Linux => {
            if let Some(xdg) = xdg_data_home {
                xdg.join("yeban/samples")
            } else if let Some(home) = home {
                home.join(".local/share/yeban/samples")
            } else {
                PathBuf::from("yeban/samples")
            }
        }
        LibraryPlatform::Windows => appdata
            .map_or_else(PathBuf::new, Path::to_path_buf)
            .join("yeban/samples"),
    }
}

/// ⭐ **采样库根**（MCP 侧的同一口径）：`YEBAN_SAMPLE_LIBRARY` 非空 ⇒ 用它；否则按平台惯例。
///
/// 逐条照 `yeban-app/src/samples.rs:294` 的 `library_root()`：
/// 先读 `YEBAN_SAMPLE_LIBRARY`（非空才用，⛔ 不规范化/不改写），再取
/// `HOME` → `USERPROFILE`、`XDG_DATA_HOME`、`APPDATA` 三个环境事实交给
/// [`preset_library_root`]。
///
/// ⚠ **每次调用都重读环境变量**（⛔ 不 `OnceLock` 缓存）：判据在同一进程里换临时库根，
/// 且"用户在会话中改环境变量"不该被上一份缓存盖住。
/// ⚠ 本函数**不创建目录**、⛔ 不查存在性。
pub(crate) fn library_root() -> PathBuf {
    if let Some(raw) = std::env::var_os(SAMPLE_LIBRARY_ENV)
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    preset_library_root(
        LibraryPlatform::current(),
        home.as_deref(),
        xdg.as_deref(),
        appdata.as_deref(),
    )
}

/// ⭐ **仓库根目录**（`CARGO_MANIFEST_DIR/../..`）—— 内置 SFZ 清单相对路径的契约根。
///
/// 逐条照 `yeban-app/src/sfz_load.rs:984` 的 `repo_root()`：
/// 优先 `canonicalize`（错误信息里读得到真实的根），失败时退回**未规范化**的那条路径
/// （宁可给出带 `..` 的根，也不静默换一个根）。`crates/yeban-mcp/../..` 与
/// `crates/yeban-app/../..` 是**同一个**仓库根。
pub(crate) fn repo_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        raw.canonicalize().unwrap_or(raw)
    })
    .as_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⭐ 与 app `tests/sample_library.rs` **同一组**期望字面量（判据 6 的第一条）。
    #[test]
    fn the_platform_formula_matches_the_app_literals() {
        let home = Path::new("/Users/crow");
        let xdg = Path::new("/home/crow/.local/share");
        let appdata = Path::new(r"C:\Users\crow\AppData\Roaming");
        let macos =
            preset_library_root(LibraryPlatform::MacOs, Some(home), Some(xdg), Some(appdata));
        let linux_xdg =
            preset_library_root(LibraryPlatform::Linux, Some(home), Some(xdg), Some(appdata));
        let linux_default =
            preset_library_root(LibraryPlatform::Linux, Some(home), None, Some(appdata));
        let linux_none = preset_library_root(LibraryPlatform::Linux, None, None, None);
        let windows = preset_library_root(
            LibraryPlatform::Windows,
            Some(home),
            Some(xdg),
            Some(appdata),
        );
        assert_eq!(
            macos,
            PathBuf::from("/Users/crow/Library/Application Support/dev.yeban.app/Samples")
        );
        assert_eq!(
            linux_xdg,
            PathBuf::from("/home/crow/.local/share/yeban/samples")
        );
        assert_eq!(
            linux_default,
            PathBuf::from("/Users/crow/.local/share/yeban/samples")
        );
        assert_eq!(linux_none, PathBuf::from("yeban/samples"));
        assert_eq!(
            windows,
            PathBuf::from(r"C:\Users\crow\AppData\Roaming").join("yeban/samples")
        );
    }

    /// 仓库根是 `crates/yeban-mcp/../..`（与 app 的 `crates/yeban-app/../..` 同一处）。
    #[test]
    fn the_repository_root_is_the_workspace_root() {
        let root = repo_root();
        assert!(
            root.join("Cargo.toml").is_file(),
            "仓库根必须有根清单: {}",
            root.display()
        );
        assert!(
            root.join("crates/yeban-mcp").is_dir(),
            "仓库根必须是工作区根: {}",
            root.display()
        );
    }
}
