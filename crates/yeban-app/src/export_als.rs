//! `--export-als` 的**消费侧**：映射与 Gzip 封装已住在 `yeban-render` 的 `als` 模块，
//! 本模块只做"落盘 + 把读数与损失表交回 CLI"这一件事。
//!
//! ## 为什么它存在（`ADR-0001 D47` 的直接后果）
//!
//! `D47` 把导出的**唯一出口**定为 app CLI（离线批处理语义），并明令**不允许**两侧各造
//! 一份导出实现。因此这里与 [`crate::export_midi`] 是同一套做法：
//!
//! - 真正的映射 / 编码只有一份，住在 `crates/yeban-render/src/als.rs`
//!   （`ROAD-M4-007` 的交付物）。本 crate **不**直接依赖 `flate2`，也不认识 LiveSet XML；
//! - 本 crate 只把 `YebanProjectV1` 交给那个导出器，把返回的字节**原子**落盘
//!   （复用 [`crate::save::write_file_atomically`]，与 `--save-as` / `--export-elements` /
//!   `--export-midi` 同一份实现）；
//! - 导出器同时返回的 [`AlsLoss`](yeban_render::als::AlsLoss) 表原样上交给
//!   [`crate::cli::run_batch`] —— `D47` 的价值就是让这张表**对用户可见**
//!   （见 `docs/ledger/open-questions.md` 问题 3 与 `docs/ledger/phase-status.md` 的
//!   `ROAD-M4-007`）。本模块**不**过滤、不截断、不重排它。
//!
//! ## feature 门（**非默认**）
//!
//! 整个可执行路径挂在非默认 feature `experimental-als-export` 后面（AGENTS.md §2 红线 6）：
//! 默认构建里 `yeban-render` 的 `als` 模块与 `flate2` 都不在依赖图上，`--export-als`
//! 是一次点名 feature 的**用法错误**（`src/cli.rs` 的 `ParseError::AlsNotCompiled`）。
//!
//! 本模块**本体**仍然总是编译：它存在的意义之一是让 [`AlsExportError`] 这个类型在默认
//! 构建里也在（`CliError::ExportAls` 要引用它）。默认构建里那个枚举只剩落盘那一档，
//! 导出那一档与它引用的 `yeban_render::als` 一起被 `cfg` 掉 —— 因此默认依赖图里
//! **一个 `.als`/`flate2` 符号都不会多**。

#![allow(unused_imports)] // 宽集合：缺失由编译器点名，多余由本行放行（账本第 339-341 轮）
use std::path::{Path, PathBuf};

/// `.als` 导出失败的原因。
///
/// 每一档都**原样**保留下层裁决（映射/压缩来自 `yeban-render`，落盘来自
/// [`crate::save`]），不发明新码（ADR-0001 D25 的口径下 CLI 复用既有的"导出失败"档，
/// 见 [`crate::cli::EXIT_EXPORT`]）。
#[derive(Debug)]
pub enum AlsExportError {
    /// 映射 / Gzip 封装失败（领域侧，来自 `yeban-render` 的 `als` 模块）。
    ///
    /// 只在带 `experimental-als-export` 的构建里存在 —— 默认构建里那段代码根本不在。
    #[cfg(feature = "experimental-als-export")]
    Export(yeban_render::als::AlsExportError),
    /// 原子落盘失败（I/O：临时文件 / 刷盘 / 原子重命名任一步）。
    Save(crate::save::SaveError),
}

impl core::fmt::Display for AlsExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(feature = "experimental-als-export")]
            Self::Export(error) => write!(formatter, "{error}"),
            Self::Save(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for AlsExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(feature = "experimental-als-export")]
            Self::Export(error) => Some(error),
            Self::Save(error) => Some(error),
        }
    }
}

#[cfg(feature = "experimental-als-export")]
impl From<yeban_render::als::AlsExportError> for AlsExportError {
    fn from(error: yeban_render::als::AlsExportError) -> Self {
        Self::Export(error)
    }
}

/// 一次成功导出的读数（落点 + 映射计数 + **完整的**损失表）。
///
/// `losses` 是导出器返回的那一份，长度由它决定（本层不裁剪）：呈现侧的截断是
/// [`crate::cli::run_batch`] 的事，而"截断了多少条"必须由那边的行**明写**出来。
#[cfg(feature = "experimental-als-export")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlsExportReport {
    /// 最终落点（调用方给的那个路径）。
    pub path: PathBuf,
    /// 实际落盘的字节数（**读回来的事实**，不是编码器声明的）。
    pub bytes: usize,
    /// 用过的临时文件名（成功之后它不应再存在；判据会去查）。
    pub temp_name: String,
    /// 写进 XML 的轨道数。
    pub mapped_tracks: usize,
    /// 写进 XML 的片段数。
    pub mapped_clips: usize,
    /// 写进 XML 的音符事件数。
    pub mapped_notes: usize,
    /// 映射损失表（与写进 XML 的 `<!-- yeban-loss … -->` 注释同一份产物）。
    pub losses: Vec<yeban_render::als::AlsLoss>,
}

/// 把工程写成 `.als` 文件（Gzip 压缩的 XML），并**同时**返回映射损失表。
///
/// 语义与 `--export-midi` 完全平行：先让导出器产出全部字节（纯内存、确定性），
/// 再走**同一份**原子落盘实现 —— 任何一步失败都不留半个文件。
///
/// # Errors
///
/// [`AlsExportError::Export`]：映射 / Gzip 封装失败；
/// [`AlsExportError::Save`]：原子落盘失败。
#[cfg(feature = "experimental-als-export")]
pub fn export_project_to_file(
    project: &yeban_model::YebanProjectV1,
    path: impl AsRef<Path>,
) -> Result<AlsExportReport, AlsExportError> {
    let export = yeban_render::als::export_project(project)?;
    let saved =
        crate::save::write_file_atomically(&export.bytes, path).map_err(AlsExportError::Save)?;
    Ok(AlsExportReport {
        path: saved.path,
        bytes: saved.bytes,
        temp_name: saved.temp_name,
        mapped_tracks: export.mapped_tracks,
        mapped_clips: export.mapped_clips,
        mapped_notes: export.mapped_notes,
        losses: export.losses,
    })
}
