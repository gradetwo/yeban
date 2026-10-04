//! 工程文件的读取、**原子落盘**与 `.yeban.lock` 排他锁 [ARCH-SEC-001, ARCH-SEC-004]。
//!
//! ## 原子落盘的三阶段（`ARCH-SEC-004` 的原文）
//!
//! 1. 完整内容先写到**同目录**的临时文件 `.{name}.tmp-{ulid}`；
//! 2. 对临时文件 `File::sync_all()`（`fsync`），确保字节真的落到非易失介质；
//! 3. `std::fs::rename` 原子替换 —— POSIX `rename(2)` 与 Windows
//!    `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` 都保证"要么旧文件完好，要么新文件生效"。
//!
//! **任何一步失败都不得破坏原文件**：失败路径统一 `remove_file(tmp)`，
//! 而不是把目标文件截断重写。判据
//! `save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original` 用一个
//! 只读父目录 + 一份预先存在的"原文件"证明这件事（注入"直接截断写"会让它变红）。
//!
//! ## 与规范的已知偏差（登记，不隐藏）
//!
//! `ARCH-SEC-003` 要求 `.yeban` 是 **ZIP 容器**（`project.json` + `history.dag` +
//! `assets/{sha256}`）。本线按工作线任务书用**裸 `YebanProjectV1` JSON**
//! （`schemas/project.schema.json` 是权威契约）—— ZIP 容器（含 Zip-Slip 与解压炸弹防御）
//! 属于容器/存储拥有者的资产。见 `docs/ledger/tools-domain-notes.md` 的未接线清单。
//!
//! ## 锁：实现了什么、没实现什么
//!
//! - **实现了**：原子创建（`O_CREAT | O_EXCL` 语义的 `create_new(true)`）+
//!   排他语义（锁存在即 `PROJECT_LOCKED`）+ 内容元数据协议 + 关闭时释放；
//! - **没实现**：`fcntl(F_SETLK)` / `LockFileEx` 的 **OS 级建议锁**、
//!   `SHARED_READ` 多读者共存、心跳与陈旧锁抢占（`kill(pid, 0)` 探测需要 `libc`，
//!   而本 crate 的新增依赖面被刻意压到最小）。
//!   只读打开的处置是**观察**排他锁并拒绝，自己不建锁 —— 比"静默忽略锁"安全。

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use yeban_model::{AssetHash, EntityId, YebanProjectV1};

use super::error::{Fault, from_io};
use crate::tools::ErrorCode;

/// 锁文件后缀（`demo.yeban` → `demo.yeban.lock`）。
pub const LOCK_SUFFIX: &str = ".lock";

/// 临时文件名的前缀字符（隐藏文件，且带 `.tmp-` 标记）。
pub const TEMP_INFIX: &str = ".tmp-";

/// 内容摘要（SHA-256 十六进制）—— 直接复用 `yeban-model` 的 CAS 哈希实现。
#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    AssetHash::of_bytes(bytes).as_str().to_owned()
}

/// 工程文件的锁文件路径。
#[must_use]
pub fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || std::ffi::OsString::from("project"),
        std::ffi::OsStr::to_os_string,
    );
    name.push(LOCK_SUFFIX);
    path.with_file_name(name)
}

/// 持有中的排他锁；`Drop` 时删除锁文件（"释放 `.yeban.lock`"）。
///
/// **刻意不实现 `Clone`**：克隆一个守卫等于克隆一个"谁先 `Drop` 谁删锁文件"的
/// 双重所有权。`Domain` 因此也不实现 `Clone`（见 `crate::dispatch::Dispatcher` 的说明）。
#[derive(Debug, PartialEq, Eq)]
pub struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    /// 锁文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // 释放失败不做任何事: `Drop` 里 panic 会让"关闭工程"变成崩溃。
        let _ = fs::remove_file(&self.path);
    }
}

/// 锁文件的元数据协议（`ARCH-SEC-001` 第 4 条）。
#[must_use]
pub fn lock_metadata(pid: u32, lock_mode: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |delta| delta.as_secs());
    let mut json = serde_json::to_string_pretty(&serde_json::json!({
        "pid": pid,
        "hostname": hostname(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "lock_mode": lock_mode,
        "started_at": now,
        "last_heartbeat": now,
    }))
    .unwrap_or_else(|_| String::from("{}"));
    json.push('\n');
    json
}

/// 主机名（读不到就用 `unknown` —— 锁元数据不是承重信息）。
#[must_use]
pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| String::from("unknown"))
}

/// 取排他锁。
///
/// - `read_only == false`：原子创建锁文件；已存在 → `PROJECT_LOCKED`；
/// - `read_only == true`：锁文件已存在 → `PROJECT_LOCKED`（观察而不创建）。
///
/// # Errors
///
/// 锁被占用 → `PROJECT_LOCKED`；创建锁文件本身失败 → `IO_ERROR`。
pub fn acquire_lock(path: &Path, read_only: bool) -> Result<Option<LockGuard>, Fault> {
    let lock = lock_path(path);
    if read_only {
        if lock.exists() {
            return Err(locked_fault(&lock));
        }
        return Ok(None);
    }
    // 原子创建: 检查与创建之间没有竞态窗口 (TOCTOU)。
    match OpenOptions::new().write(true).create_new(true).open(&lock) {
        Ok(mut file) => {
            let metadata = lock_metadata(std::process::id(), "ExclusiveWrite");
            let write = file
                .write_all(metadata.as_bytes())
                .and_then(|()| file.sync_all());
            if let Err(error) = write {
                drop(file);
                let _ = fs::remove_file(&lock);
                return Err(from_io(&format!("写入锁元数据 {}", lock.display()), &error));
            }
            Ok(Some(LockGuard { path: lock }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(locked_fault(&lock)),
        Err(error) => Err(from_io(&format!("创建锁文件 {}", lock.display()), &error)),
    }
}

/// `PROJECT_LOCKED` 的载荷（带持有者元数据，便于人眼排查）。
fn locked_fault(lock: &Path) -> Fault {
    let holder = fs::read_to_string(lock).unwrap_or_else(|_| String::from("<不可读>"));
    Fault::domain_with_data(
        ErrorCode::ProjectLocked,
        format!("工程已被排他锁占用: {}", lock.display()),
        serde_json::json!({ "lockFile": lock.display().to_string(), "holder": holder }),
    )
}

/// 读取并校验一个工程文件。
///
/// 返回 `(工程, 磁盘上的原始文本, 字节数)`；文本与字节数是 `dryRun` 与
/// "未保存标记"判定的依据（避免重新序列化后误判"没变"）。
///
/// # Errors
///
/// - 不是普通文件 → `FILE_NOT_FOUND`；
/// - 读失败 → `IO_ERROR`（`DISK_FULL` 等由 [`super::error::code_for_io`] 判定）；
/// - UTF-8 / JSON / 版本门 / 结构校验失败 → `IO_ERROR` 或 `CONFLICT`。
pub fn read_project(path: &Path) -> Result<(YebanProjectV1, String, u64), Fault> {
    if !path.is_file() {
        return Err(Fault::domain(
            ErrorCode::FileNotFound,
            format!("工程文件不存在或不是普通文件: {}", path.display()),
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| from_io(&format!("读取 {}", path.display()), &error))?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        Fault::domain(
            ErrorCode::IoError,
            format!("工程文件不是合法 UTF-8: {}", path.display()),
        )
    })?;
    let project: YebanProjectV1 = serde_json::from_str(&text).map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!(
                "工程文件不是合法的 YebanProjectV1 JSON ({}): {error}",
                path.display()
            ),
        )
    })?;
    project
        .check_readable()
        .map_err(|error| super::error::from_model("工程版本门", &error))?;
    project
        .validate()
        .map_err(|error| super::error::from_model("工程结构校验", &error))?;
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    Ok((project, text, len))
}

/// 把工程序列化成落盘文本（美化 JSON + 结尾换行）。
///
/// # Errors
///
/// 序列化失败（理论上不会：`YebanProjectV1` 的字段全部可序列化）。
pub fn serialize_project(project: &YebanProjectV1) -> Result<String, Fault> {
    let mut json = serde_json::to_string_pretty(project)
        .map_err(|error| Fault::domain(ErrorCode::IoError, format!("序列化工程失败: {error}")))?;
    json.push('\n');
    Ok(json)
}

/// **原子**把 `json` 落盘到 `path`（同目录临时文件 + `fsync` + `rename`）。
///
/// 失败时原文件**逐字节不变**，且临时文件被清理。
///
/// # Errors
///
/// 父目录不存在、无写权限、磁盘写满、`rename` 失败 → 对应契约错误码
/// （`IO_ERROR` / `DISK_FULL`）。
pub fn write_project_atomic(path: &Path, json: &str) -> Result<(), Fault> {
    let parent = parent_dir(path);
    if !parent.is_dir() {
        return Err(Fault::domain(
            ErrorCode::IoError,
            format!("目标父目录不存在: {}", parent.display()),
        ));
    }
    let stem = path.file_name().map_or_else(
        || String::from("project"),
        |name| name.to_string_lossy().into_owned(),
    );
    // 临时文件与目标**同目录**: 跨目录 rename 不是原子替换, 甚至可能 EXDEV 失败。
    let temp = parent.join(format!(
        ".{stem}{TEMP_INFIX}{}",
        EntityId::new().to_canonical_string()
    ));

    let outcome = write_then_replace(&temp, path, json);
    if outcome.is_err() {
        let _ = fs::remove_file(&temp);
    }
    outcome.map_err(|error| {
        from_io(
            &format!("原子保存 {} (临时文件 {})", path.display(), temp.display()),
            &error,
        )
    })
}

/// 三阶段写入本体；失败时不留半成品（调用方负责清理临时文件）。
fn write_then_replace(temp: &Path, target: &Path, json: &str) -> std::io::Result<()> {
    {
        let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(temp, target)
}

/// 父目录（没有父目录时用 `.`，而不是空 `Path`）。
#[must_use]
pub fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-store-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn lock_path_appends_the_suffix() {
        assert_eq!(
            lock_path(Path::new("/tmp/demo.yeban")),
            PathBuf::from("/tmp/demo.yeban.lock")
        );
    }

    #[test]
    fn exclusive_lock_is_atomic_and_released_on_drop() {
        let dir = scratch("lock");
        let project = dir.join("demo.yeban");
        fs::write(&project, "{}").expect("占位");
        let guard = acquire_lock(&project, false)
            .expect("首次加锁")
            .expect("持有");
        assert!(guard.path().is_file());
        let holder = fs::read_to_string(guard.path()).expect("读锁元数据");
        assert!(
            holder.contains("\"lock_mode\": \"ExclusiveWrite\""),
            "{holder}"
        );
        // 第二次加锁必须失败。
        let second = acquire_lock(&project, false).expect_err("锁被占用");
        assert_eq!(second.domain_code(), Some(ErrorCode::ProjectLocked));
        // 只读打开也要观察到排他锁。
        let read_only = acquire_lock(&project, true).expect_err("只读也要拒绝");
        assert_eq!(read_only.domain_code(), Some(ErrorCode::ProjectLocked));
        drop(guard);
        assert!(!lock_path(&project).exists(), "Drop 必须释放锁");
        acquire_lock(&project, false)
            .expect("释放后可以重新加锁")
            .expect("持有");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_only_open_does_not_create_a_lock_file() {
        let dir = scratch("readonly");
        let project = dir.join("demo.yeban");
        fs::write(&project, "{}").expect("占位");
        assert!(acquire_lock(&project, true).expect("可读").is_none());
        assert!(!lock_path(&project).exists(), "只读打开不得创建锁文件");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_replaces_the_target_and_leaves_no_temp_files() {
        let dir = scratch("atomic");
        let project = dir.join("demo.yeban");
        fs::write(&project, "OLD\n").expect("原文件");
        write_project_atomic(&project, "NEW\n").expect("保存");
        assert_eq!(fs::read_to_string(&project).expect("读"), "NEW\n");
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(TEMP_INFIX))
            .collect();
        assert!(
            leftovers.is_empty(),
            "临时文件必须被 rename 掉: {leftovers:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_parent_directory_is_an_io_error() {
        let dir = scratch("noparent");
        let missing = dir.join("nope").join("demo.yeban");
        let fault = write_project_atomic(&missing, "x").expect_err("父目录不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn serialize_read_round_trip_is_byte_stable() {
        let dir = scratch("roundtrip");
        let project = dir.join("demo.yeban");
        let model = yeban_model::samples::filled_project();
        let json = serialize_project(&model).expect("序列化");
        write_project_atomic(&project, &json).expect("保存");
        let (read, text, len) = read_project(&project).expect("读回");
        assert_eq!(read, model);
        assert_eq!(text, json, "读回的文本必须逐字节相同");
        assert_eq!(len as usize, json.len());
        assert_eq!(digest_of(json.as_bytes()).len(), 64);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_json_is_refused_with_an_io_error() {
        let dir = scratch("corrupt");
        let project = dir.join("demo.yeban");
        fs::write(&project, "{not json").expect("写坏文件");
        let fault = read_project(&project).expect_err("必须拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn directory_target_is_file_not_found() {
        let dir = scratch("isdir");
        let fault = read_project(&dir).expect_err("目录不是工程文件");
        assert_eq!(fault.domain_code(), Some(ErrorCode::FileNotFound));
        fs::remove_dir_all(&dir).ok();
    }
}
