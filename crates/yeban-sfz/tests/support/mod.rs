//! 测试支持代码（std-only）。
//!
//! 刻意**不**使用 `tempfile`：本 crate 的依赖预算只允许 `thiserror`
//! （见 `docs/ledger/sfz-core-notes.md`「依赖纪律」）。这里用
//! `std::env::temp_dir()` + 进程号 + 原子计数器保证唯一性，并在 `Drop` 里清理。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 一个自动清理的临时目录。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// 创建一个唯一命名的临时目录。
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yeban-sfz-test-{}-{tag}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// 临时目录路径。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
