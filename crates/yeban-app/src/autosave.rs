//! 自动保存草稿与崩溃恢复的**纯逻辑层**（ARCH-SYS-001 / T-FIX-796）。
//!
//! 与 `dirty_guard.rs` 同纪律：本模块不依赖 Slint，只做路径/间隔/扫描/分类/
//! 隔离，真写盘复用 [`crate::save`] 的原子落盘原语，定时器与弹窗接线在 `host`。
//! 默认不能静默：坏间隔、残缺草稿、残留临时文件都返回可被宿主响亮上报的信息。

use std::path::{Path, PathBuf};

/// 规范默认间隔（秒）：ARCH-SYS-001 的「每 60 秒」。
pub const DEFAULT_INTERVAL_SECS: u64 = 60;
/// 下限：避免误配成 0 造成忙循环。
pub const MIN_INTERVAL_SECS: u64 = 5;
/// 上限：避免误配成天文数字导致实质不保存。
pub const MAX_INTERVAL_SECS: u64 = 3600;
/// 间隔配置环境变量（显式覆盖，非法值响亮告警后回落默认）。
pub const INTERVAL_ENV: &str = "YEBAN_AUTOSAVE_INTERVAL_SECS";
/// 草稿扩展名：与正式 `.yeban` 明确区分，绝不覆盖正式工程。
pub const DRAFT_EXT: &str = "yeban.autosave";
/// 隔离残缺草稿时追加的扩展名。
pub const CORRUPT_EXT: &str = "corrupt";

/// 读取自动保存间隔（秒）。
///
/// 未设置 ⇒ 60；越界 ⇒ 钳到 `[5, 3600]`；不可解析 ⇒ 响亮 `eprintln!` 后回落 60。
/// 非法值绝不静默吞掉。
#[must_use]
pub fn interval_secs() -> u64 {
    let Ok(raw) = std::env::var(INTERVAL_ENV) else {
        return DEFAULT_INTERVAL_SECS;
    };
    match raw.trim().parse::<u64>() {
        Ok(value) if (MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&value) => value,
        Ok(value) => {
            let clamped = value.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS);
            eprintln!(
                "[autosave] {INTERVAL_ENV}={value} 超出 [{MIN_INTERVAL_SECS}, {MAX_INTERVAL_SECS}]，钳到 {clamped}"
            );
            clamped
        }
        Err(error) => {
            eprintln!(
                "[autosave] {INTERVAL_ENV}={raw:?} 不是合法整数（{error}），回落 {DEFAULT_INTERVAL_SECS} 秒"
            );
            DEFAULT_INTERVAL_SECS
        }
    }
}

/// 由正式工程路径推导草稿路径（同目录、不同扩展名）。
#[must_use]
pub fn draft_path(project_path: &Path) -> PathBuf {
    project_path.with_extension(DRAFT_EXT)
}

/// 是否是自动保存草稿文件。
#[must_use]
pub fn is_draft(path: &Path) -> bool {
    // 扩展名是复合名（`yeban.autosave`），`Path::extension()` 只返回最后一段
    // （`autosave`）⇒ 必须按整个文件名后缀判定。
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.len() > DRAFT_EXT.len() && name.ends_with(&format!(".{DRAFT_EXT}"))
        })
}

/// 是否是保存写出残留的临时文件（与正式工程同目录的 `.yeban.tmp-*`）。
#[must_use]
pub fn is_stale_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(crate::save::TEMP_INFIX))
}

/// 启动期扫描结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// 可解析的草稿（给恢复入口）。
    pub complete_drafts: Vec<PathBuf>,
    /// 残缺草稿（显式告警并隔离）。
    pub corrupt_drafts: Vec<PathBuf>,
    /// 残留临时文件（显式告警并清理）。
    pub stale_temps: Vec<PathBuf>,
}

impl SweepReport {
    /// 本次扫描是否发现任何需要处理的文件。
    #[must_use]
    pub fn has_findings(&self) -> bool {
        !(self.complete_drafts.is_empty()
            && self.corrupt_drafts.is_empty()
            && self.stale_temps.is_empty())
    }
}

/// ZIP 容器的 magic：`yeban-model` 的容器是 ZIP，完整草稿至少应以 PK 开头且有内容。
fn looks_like_container(bytes: &[u8]) -> bool {
    bytes.len() > 22 && bytes.starts_with(b"PK\x03\x04")
}

/// 扫描工程目录下的草稿与残留临时文件，并按「完整/残缺」分类。
///
/// 只读：不删除、不改名；处置由宿主写入回执后执行，保持默认不静默。
#[must_use]
pub fn sweep(dir: &Path) -> SweepReport {
    let mut report = SweepReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if is_stale_temp(&path) {
            report.stale_temps.push(path);
        } else if is_draft(&path) {
            let bytes = std::fs::read(&path).unwrap_or_default();
            if looks_like_container(&bytes) {
                report.complete_drafts.push(path);
            } else {
                report.corrupt_drafts.push(path);
            }
        }
    }
    report
}

/// 隔离残缺草稿：改名为 `<name>.corrupt.<n>`，不静默丢弃。
///
/// # Errors
/// 改名失败时原样返回 I/O 错误，由宿主响亮上报。
pub fn quarantine(path: &Path) -> std::io::Result<PathBuf> {
    let mut target = path.to_path_buf();
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("draft");
    target.set_file_name(format!("{stem}.{CORRUPT_EXT}"));
    let mut counter = 1u32;
    while target.exists() {
        target.set_file_name(format!("{stem}.{CORRUPT_EXT}.{counter}"));
        counter += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yeban-autosave-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn default_interval_is_60_seconds() {
        // 环境变量由 CI 控制；若显式设置则以配置值为准，本条只钉默认常量。
        assert_eq!(DEFAULT_INTERVAL_SECS, 60);
        const { assert!(MIN_INTERVAL_SECS <= DEFAULT_INTERVAL_SECS) };
        const { assert!(DEFAULT_INTERVAL_SECS <= MAX_INTERVAL_SECS) };
    }

    #[test]
    fn draft_path_never_overwrites_official_project() {
        let official = Path::new("/tmp/song.yeban");
        let draft = draft_path(official);
        assert_ne!(draft, official);
        assert!(is_draft(&draft));
        assert!(!is_draft(official));
    }

    #[test]
    fn sweep_classifies_complete_corrupt_and_temp() {
        let dir = scratch("sweep");
        std::fs::write(
            dir.join("song.yeban.autosave"),
            b"PK\x03\x04rest-of-zip-container-minimum-length",
        )
        .unwrap();
        std::fs::write(dir.join("bad.yeban.autosave"), b"not a zip").unwrap();
        std::fs::write(dir.join("song.yeban.tmp-ULID"), b"partial").unwrap();
        let report = sweep(&dir);
        assert_eq!(report.complete_drafts.len(), 1);
        assert_eq!(report.corrupt_drafts.len(), 1);
        assert_eq!(report.stale_temps.len(), 1);
        assert!(report.has_findings());
    }

    #[test]
    fn quarantine_does_not_silently_discard() {
        let dir = scratch("quarantine");
        let bad = dir.join("bad.yeban.autosave");
        std::fs::write(&bad, b"garbage").unwrap();
        let target = quarantine(&bad).expect("rename");
        assert!(!bad.exists(), "原残缺草稿应被移走");
        assert!(target.exists(), "必须留下可追溯的 .corrupt 文件");
        assert!(target.to_string_lossy().contains(CORRUPT_EXT));
    }
}
