//! `--export-lyrics <path.lrc>` —— 歌词时间轴的**批处理出口**。
//!
//! ## 它补的是哪一格（这是本模块存在的全部理由）
//!
//! 工程里的歌词时间轴（`YebanProjectV1::lyrics`，见 [`yeban_model::lyrics`]）
//! 只有一条出路是把它变成文件：本模块。没有它，词只能躺在 `project.json` 里，
//! 而 Wang 2026-11-10 实测：切片双版那两份 LRC 是比对 ASR 音频 + 编曲事件
//! 硬算出来的 —— 歌词轨的全部意义就是让这份手工消失。
//!
//! 一次 `yeban-app --open X.yeban --export-lyrics X.lrc`，不开窗口、不建 MCP 会话、
//! 不碰 `~/.yeban` 的令牌。格式口径（`[ti/ar/by/length/offset]` 头 +
//! `[mm:ss.cc]行`）由 [`yeban_model::lyrics::lyrics_to_lrc`] 钉住，本模块只负责落盘与回执。
//!
//! ## 失败语义（与其它导出族同款）
//!
//! * 落盘失败 ⇒ 非零退出码，**不留半成品**（落盘走 [`crate::save::write_file_atomically`]：
//!   同目录临时文件 → rename）；
//! * 无词不是失败：空工程、纯 MIDI 无歌词都是合法状态 —— 回执写 `lines=0`，
//!   文件只有头标签（与 `--export-song-overview` 的静音自报同款纪律：
//!   “成功但空”必须自己说出来，⛔ 不悄悄给一个看起来有内容的空文件 ——
//!   这里的空文件一眼就是空的，头标签之后一行词都没有）。

use std::path::{Path, PathBuf};

use yeban_model::YebanProjectV1;

use crate::save::{SaveError, SaveReport, write_file_atomically};

/// 导出成功后回执行的第一段（稳定前缀，判据与脚本据此切片）。
pub const RECEIPT_PREFIX: &str = "exported-lyrics:";

/// 一次导出的实测读数（回执行的**唯一**来源：⛔ 不从入参抄）。
#[derive(Debug)]
pub struct LyricsExportReport {
    /// 实际落点（`write_file_atomically` 报的那个路径）。
    pub path: PathBuf,
    /// 实际字节数。
    pub bytes: usize,
    /// 实际写出的歌词行数（无词工程 ⇒ 0）。
    pub lines: usize,
}

impl LyricsExportReport {
    /// 回执行（**一行**，字段顺序即契约）。
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{RECEIPT_PREFIX} path={} bytes={} lines={}",
            self.path.display(),
            self.bytes,
            self.lines,
        )
    }
}

/// 导出失败的两种来源（退出码由 `cli::CliError` 统一给：都是 `EXIT_EXPORT`）。
#[derive(Debug)]
pub enum LyricsExportError {
    /// LRC 文本算不出来（病态 BPM 等 —— 已校验的工程不会走到这里）。
    Convert {
        /// 模型层的原样裁决。
        reason: String,
    },
    /// 落盘失败。
    Save {
        /// 目标路径。
        path: PathBuf,
        /// 落盘层原样裁决。
        source: SaveError,
    },
}

impl core::fmt::Display for LyricsExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Convert { reason } => write!(formatter, "{reason}"),
            Self::Save { path, source } => {
                write!(formatter, "写入 `{}` 失败: {source}", path.display())
            }
        }
    }
}

/// 把工程的歌词时间轴算成 LRC 并**原子**落盘，返回实测读数。
///
/// # 参数
///
/// * `project`：要导出的工程（`--open` 的那份，或 `--project-sample`）；
/// * `path`：LRC 落点（覆盖已有文件；失败不留半成品）。
///
/// # Errors
///
/// [`LyricsExportError::Convert`]（LRC 文本算不出来）或
/// [`LyricsExportError::Save`]（落盘失败）。
pub fn export_lyrics(
    project: &YebanProjectV1,
    path: &Path,
) -> Result<LyricsExportReport, LyricsExportError> {
    let lrc = yeban_model::lyrics::lyrics_to_lrc(project, None).map_err(|error| {
        LyricsExportError::Convert {
            reason: error.to_string(),
        }
    })?;
    let lines = project.lyrics.len();
    let report: SaveReport =
        write_file_atomically(lrc.as_bytes(), path).map_err(|source| LyricsExportError::Save {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(LyricsExportReport {
        path: report.path,
        bytes: report.bytes,
        lines,
    })
}
