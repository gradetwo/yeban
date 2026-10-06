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
    /// 目标 bundle 目录（调用方给的那个路径）。
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
/// # Errors
///
/// [`LogicExportError::Directory`]：建目录失败；
/// [`LogicExportError::Save`]：原子落盘失败。
#[cfg(feature = "experimental-logic-export")]
pub fn export_project_to_bundle(
    project: &yeban_model::YebanProjectV1,
    directory: impl AsRef<Path>,
) -> Result<LogicExportReport, LogicExportError> {
    let directory = directory.as_ref().to_path_buf();
    let variant = if project.title.trim().is_empty() {
        "Yeban Arrangement".to_owned()
    } else {
        project.title.clone()
    };
    let bundle = yeban_render::logic::build_bundle(project, LOGIC_ALTERNATIVE, variant.as_str());

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
