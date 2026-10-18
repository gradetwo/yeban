//! `--export-logic` 的**消费侧**：映射与编码已住在 `yeban-render` 的 `logic` 模块，
//! 本模块只做"建目录 + 落盘 + 把读数与损失表交回 CLI"这一件事。
//!
//! ## 为什么它存在（`ADR-0001 D47` 的直接后果）
//!
//! `D47` 把导出的**唯一出口**定为 app CLI（离线批处理语义），并明令**不允许**两侧各造
//! 一份导出实现。因此这里与 [`crate::export_als`] 是同一套做法：
//!
//! - 真正的映射 / 编码只有一份，住在 `crates/yeban-render/src/logic.rs`
//!   （`[ARCH-FMT-002]` / `[ROAD-M4-007]` 的交付物）。本 crate **不**认识 `ProjectData`
//!   的分块布局，也不认识 bplist00；
//! - 本 crate 只把 `YebanProjectV1` 交给那个导出器拿到**路径 → 字节**的映射，
//!   建出 bundle 目录，并对每个文件走**同一份**原子落盘实现
//!   （[`crate::save::write_file_atomically`]，与 `--save-as` / `--export-elements` /
//!   `--export-midi` / `--export-als` 同源）；
//! - 导出器同时返回的 [`yeban_render::logic::LogicLoss`] 表原样上交给
//!   [`crate::cli::run_batch`] —— `D47` 的价值就是让这张表**对用户可见**。
//!   本模块**不**过滤、不截断、不重排它。
//!
//! ## 一个 `.logicx` 是目录
//!
//! 与前面几个导出不同，本开关的取值是一个**目录**（bundle），因此本模块会
//! `create_dir_all`（含父目录）—— 那是本特性的语义（"create the directory and write"），
//! 不是"凭空造出目录"的意外。目标路径被一个**普通文件**挡住时照样失败（退出码 5），
//! 且不写任何文件。
//!
//! ## feature 门（**非默认**）
//!
//! 整个可执行路径挂在非默认 feature `experimental-logic-export` 后面（AGENTS.md §2 红线 6）：
//! 默认构建里 `yeban-render` 的 `logic` 模块根本不在，`--export-logic` 是一次点名 feature 的
//! **用法错误**（`src/cli.rs` 的 `ParseError::LogicNotCompiled`）。
//!
//! 与 `export_als` 不同，[`LogicExportError`] 的两档都是 I/O 错误（映射/编码是无错的纯函数）,
//! 因此本模块**总是编译**（默认构建里 `export_project_to_bundle` 被 `cfg` 掉，
//! 但错误类型在，`CliError::ExportLogic` 才能引用它）。

#![allow(unused_imports)] // 宽集合：缺失由编译器点名，多余由本行放行（账本第 339-341 轮）
use std::path::{Path, PathBuf};

/// 写进目录名的 alternative 编号（三位十进制）。
///
/// 真实工程的编号**不是**永远 `000`（`Swing!` 是 `004`、`ocean eyes` 是 `001`），
/// 而 `Resources/ProjectInformation.plist` 的 `ActiveVariant` 必须与目录名指同一个编号 ——
/// 本切片固定写 `000`，并把这条限制写进导出器的损失表。
pub const LOGIC_ALTERNATIVE: &str = "000";

/// `.logicx` bundle 的**扩展名**（含点）。
///
/// `#200`：`open -a "Logic Pro" hello` 在 macOS 上**报错** —— LaunchServices 靠**扩展名**
/// 认 bundle，不是靠目录内容。于是"用户给了一个目录名、我们写出一个 bundle"这件事
/// 在**打开那一步**才失败，而失败点离原因很远（用户的命令行里根本没有 `.logicx` 这个词）。
/// 因此本模块把后缀当成**导出的组成部分**而不是用户的义务，见
/// [`logic_bundle_directory`]。
pub const LOGIC_BUNDLE_EXTENSION: &str = ".logicx";

/// 把 `--export-logic` 的取值补成**以 `.logicx` 结尾**的目录（`#200`）。
///
/// 三条语义，逐条都有判据钉住：
///
/// 1. **缺后缀 ⇒ 补**：`hello` → `hello.logicx`。理由见 [`LOGIC_BUNDLE_EXTENSION`]：
///    没有这个后缀，`open -a "Logic Pro" hello` 会报错，而报错点离"用户少打了一个后缀"
///    很远 —— 用户不该为它踩坑。
/// 2. **已有后缀 ⇒ **不**重复加**：`foo.logicx` 仍是 `foo.logicx`（⛔ 不是
///    `foo.logicx.logicx`）。比较是 **ASCII 大小写不敏感**的：macOS 的 LaunchServices
///    按大小写不敏感匹配扩展名，`Foo.LOGICX` 一样能被认出来，补成
///    `Foo.LOGICX.logicx` 只会让名字变丑而不增加任何兼容性。
/// 3. **幂等**：对返回值再调一次不变 —— 这条让调用方（CLI 与库内两条路径）都能安全地补。
///
/// 名字里**已有的点不会丢**：`foo.bar` → `foo.bar.logicx`（⛔ 不用 `set_extension`，
/// 那会把 `.bar` 换成 `.logicx`，等于替用户改名）。
///
/// 末尾带分隔符的路径（`hello.logicx/`）按**最后一个组件**判断，因此不会补成
/// `hello.logicx/.logicx`。没有任何文件名的路径（`/`、`..`）原样返回 —— 那种取值会在
/// 建目录那一步失败，用不着在这里替它编一个名字。
#[must_use]
pub fn logic_bundle_directory(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let Some(name) = path.file_name() else {
        return path.to_path_buf();
    };
    if name
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(LOGIC_BUNDLE_EXTENSION)
    {
        return path.to_path_buf();
    }
    let mut suffixed = name.to_os_string();
    suffixed.push(LOGIC_BUNDLE_EXTENSION);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(suffixed),
        _ => PathBuf::from(suffixed),
    }
}

/// `.logicx` 导出失败的原因。
///
/// 映射 / 编码（`yeban-render` 的 `logic` 模块）是**纯函数、无错**，因此这里只有 I/O 两档。
#[derive(Debug)]
pub enum LogicExportError {
    /// bundle 目录建不出来（路径被普通文件挡住 / 权限）。
    Directory {
        /// 建目录失败的那一层。
        path: PathBuf,
        /// I/O 原因。
        source: std::io::Error,
    },
    /// 原子落盘失败（临时文件 / 刷盘 / 原子重命名任一步）。
    Save(crate::save::SaveError),
}

impl core::fmt::Display for LogicExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Directory { path, source } => {
                write!(
                    formatter,
                    "创建 bundle 目录 `{}` 失败: {source}",
                    path.display()
                )
            }
            Self::Save(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for LogicExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Directory { source, .. } => Some(source),
            Self::Save(error) => Some(error),
        }
    }
}

impl From<crate::save::SaveError> for LogicExportError {
    fn from(error: crate::save::SaveError) -> Self {
        Self::Save(error)
    }
}

/// 一次成功导出的读数（bundle 目录 + 每个文件实际写出的字节数 + 映射计数 + **完整的**损失表）。
#[cfg(feature = "experimental-logic-export")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicExportReport {
    /// **实际落盘**的 bundle 目录 —— 用户给的值缺 `.logicx` 时这里是**补过**的路径
    /// （[`logic_bundle_directory`]，`#200`）。回执的 `path=` 打印的就是它。
    pub directory: PathBuf,
    /// 包内相对路径 → 实际落盘的字节数（按 `BTreeMap` 键序）。
    pub files: Vec<(String, usize)>,
    /// 写进 `ProjectData` 的 region 条数。
    pub mapped_regions: usize,
    /// 写进 `ProjectData` 的音符事件数。
    pub mapped_notes: usize,
    /// 映射损失表（与写进 `MetaData.plist` 的 `YebanMappingLosses` 同一份产物）。
    pub losses: Vec<yeban_render::logic::LogicLoss>,
}

/// 把工程写成一个 `.logicx` **bundle 目录**，并**同时**返回映射损失表。
///
/// 语义与 `--export-als` 平行：先让导出器产出全部字节（纯内存、确定性），再逐个文件走
/// **同一份**原子落盘实现 —— 任何一步失败都会让整次导出返回 `Err`（已写出的文件保留，
/// 因为 bundle 是多文件产物；本函数**不**声称整目录的原子性）。
///
/// ⭐ `#200`：目标目录**强制**以 `.logicx` 结尾 —— 入参先过 [`logic_bundle_directory`]
/// （缺后缀补上、已有的不重复加），因此**库这一层**也不可能有调用方绕过它。
/// 返回值里的 [`LogicExportReport::directory`] 是**实际**路径，回执照它打印。
///
/// # Errors
///
/// [`LogicExportError::Directory`]：建目录失败；
/// [`LogicExportError::Save`]：原子落盘失败。
#[cfg(feature = "experimental-logic-export")]
pub fn export_project_to_bundle(
    project: &yeban_model::YebanProjectV1,
    directory: impl AsRef<Path>,
) -> Result<LogicExportReport, LogicExportError> {
    let directory = logic_bundle_directory(directory);
    let variant = if project.title.trim().is_empty() {
        "Yeban Arrangement".to_owned()
    } else {
        project.title.clone()
    };
    // 供体路线：`ProjectData` 以 `jonkubis/logicproformatwriter`（MIT）那份 **Logic 存过的**
    // 夹具为骨架，只把我们自己的拍号 / 速度 / region 名 / 音符写进去。理由与登记见
    // `yeban_render::logic` 的模块头与 `build_bundle_from_donor`，以及
    // `crates/yeban-render/assets/logic-donor/README.md`。
    let bundle =
        yeban_render::logic::build_bundle_from_donor(project, LOGIC_ALTERNATIVE, variant.as_str());

    let mut files = Vec::with_capacity(bundle.files.len());
    for (relative, bytes) in &bundle.files {
        let path = directory.join(relative);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| LogicExportError::Directory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let saved = crate::save::write_file_atomically(bytes, &path)?;
        files.push((relative.clone(), saved.bytes));
    }

    Ok(LogicExportReport {
        directory,
        files,
        mapped_regions: bundle.mapped_regions,
        mapped_notes: bundle.mapped_notes,
        losses: bundle.losses,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    /// ⭐ **判据（`#200`）**：缺后缀 ⇒ 补 `.logicx`；名字里**已有的点不丢**。
    ///
    /// 这张表是"用户会怎么打"的清单：裸名字、相对目录、绝对目录、带点的名字、带空格的名字。
    /// 每一条都断言**整条结果**（不是 `ends_with`），这样"补错位置"（例如补到父目录上）
    /// 也会红。
    #[test]
    fn logic_bundle_directory_appends_the_suffix_when_missing() {
        for (given, expected) in [
            ("hello", "hello.logicx"),
            ("out/Song", "out/Song.logicx"),
            ("/tmp/yeban-x/Song", "/tmp/yeban-x/Song.logicx"),
            ("foo.bar", "foo.bar.logicx"), // ⛔ 不许用 `set_extension` 把 `.bar` 换掉
            ("hello world", "hello world.logicx"),
            ("Song v2.1", "Song v2.1.logicx"),
            ("logicx", "logicx.logicx"), // 只有**扩展名**才算，光有词不算
        ] {
            assert_eq!(
                logic_bundle_directory(given),
                PathBuf::from(expected),
                "`{given}` 必须补成 `{expected}`"
            );
        }
    }

    /// ⭐ **判据（`#200`）**：末尾已经是 `.logicx` ⇒ **不重复加**（`foo.logicx` ⛔ 不变
    /// `foo.logicx.logicx`），且**幂等**（对结果再补一次不变）。
    ///
    /// 大小写不敏感这一条单独钉：macOS 的 LaunchServices 按大小写不敏感匹配扩展名，
    /// 因此 `Foo.LOGICX` 已经是"能被 `open -a` 认出"的名字，补成 `Foo.LOGICX.logicx`
    /// 只会让名字变丑。
    #[test]
    fn logic_bundle_directory_never_doubles_the_suffix() {
        for given in [
            "foo.logicx",
            "out/Song.logicx",
            "/tmp/yeban-x/Song.logicx",
            "Foo.LOGICX",
            "Foo.LogicX",
            ".logicx", // 隐藏文件名，整体就是后缀
        ] {
            let once = logic_bundle_directory(given);
            assert_eq!(once, PathBuf::from(given), "`{given}` 必须原样返回");
            assert_eq!(
                logic_bundle_directory(&once),
                once,
                "幂等：`{given}` 再补一次不许变"
            );
        }

        // 末尾带分隔符时按**最后一个组件**判断（不能变成 `Song.logicx/.logicx`）。
        let trailing = logic_bundle_directory("out/Song.logicx/");
        assert_eq!(
            trailing.file_name(),
            Some(OsStr::new("Song.logicx")),
            "带尾分隔符的路径不许被补成 `Song.logicx/.logicx`"
        );
    }
}
