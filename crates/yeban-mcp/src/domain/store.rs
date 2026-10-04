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
//! ## 锁：本模块的职责边界
//!
//! **锁的机制全部住在 [`super::lock`]**（`MUST-GATE-008`）：原子创建 + `flock(2)`
//! 建议锁 + 崩溃遗留接管 + 平台矩阵。本模块只保留**两个**职责：
//!
//! 1. 把锁文件路径/内容协议**再导出**（历史调用点 `store::lock_path` 等不变）；
//! 2. 把 [`super::lock::LockError`] **唯一地**映射到契约错误码 ——
//!    `WouldBlock`（含跨进程与同进程另一个 fd）→ `PROJECT_LOCKED`；
//!    平台无建议锁 → JSON-RPC 实现级 `-32005`（**不发明新错误码**，
//!    `ADR-0001 D25` 的联集是 20 值，锁相关的领域码只有 `PROJECT_LOCKED`）。
//!
//! 历史包袱的处置：旧实现是"锁文件存在即占用"，因此进程崩溃会**永久锁死**工程
//! （`docs/ledger/tools-domain-notes.md` boundary-3）。新实现里"文件存在"**不是**
//! 占用证据 —— 证据是"建议锁被内核持有"。见 [`super::lock`] 的崩溃/竞态矩阵。

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use yeban_model::{AssetHash, EntityId, YebanProjectV1};

use super::error::{Fault, from_io};
use super::lock::{self, LockError};
use crate::jsonrpc::{ErrorObject, NOT_IMPLEMENTED};
use crate::tools::ErrorCode;

pub use super::lock::{
    HEARTBEAT_INTERVAL_SECS, LOCK_SUFFIX, LockGuard, LockMetadata, LockMode, STALE_HEARTBEAT_SECS,
    hostname, lock_path, read_metadata,
};

/// 临时文件名的前缀字符（隐藏文件，且带 `.tmp-` 标记）。
pub const TEMP_INFIX: &str = ".tmp-";

/// 内容摘要（SHA-256 十六进制）—— 直接复用 `yeban-model` 的 CAS 哈希实现。
#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    AssetHash::of_bytes(bytes).as_str().to_owned()
}

/// 一次成功的加锁（守卫 + 可观察的结果面）。
///
/// 存在理由：`PROJECT_LOCKED` 只看 [`Fault`]，但"**接管了崩溃遗留的陈旧锁**"
/// 这件事必须能被调用方（和判据）看见 —— [`LockGuard::took_over_stale_lock`]。
#[derive(Debug)]
pub struct AcquiredLock {
    /// RAII 守卫（`Drop` 释放建议锁）。
    pub guard: LockGuard,
}

impl AcquiredLock {
    /// 是否接管了一份崩溃遗留的陈旧锁文件。
    #[must_use]
    pub const fn took_over_stale_lock(&self) -> bool {
        self.guard.took_over_stale_lock()
    }
}

/// **拿到锁**（原子创建 + OS 建议锁）：`MUST-GATE-008` 的唯一入口。
///
/// # Errors
///
/// - 锁被别的活着的持有者占用 → `PROJECT_LOCKED`（载荷含锁文件路径 + 持有者元数据
///   + 建议锁诊断）；
/// - 平台无建议锁 → JSON-RPC 实现级 `-32005`（**不是**契约错误码，见 [`Fault::Impl`]）；
/// - 文件系统失败 → `IO_ERROR` / `FILE_NOT_FOUND` / `DISK_FULL`。
pub fn lock(project_path: &Path, mode: LockMode) -> Result<AcquiredLock, Fault> {
    lock::acquire(project_path, mode)
        .map(|guard| AcquiredLock { guard })
        .map_err(|error| lock_fault(project_path, error))
}

/// `read_only: bool` 形态的兼容入口（`domain/mod.rs` 的历史调用点）。
///
/// **共享读锁不再返回 `None`**：锁文件会被创建（0 字节或含元数据），
/// 因为共享锁必须锁在**同一个 inode** 上才能与排他写锁互斥。
/// 旧实现返回 `None`（"只读打开不建锁文件"）在语义上等于"只读打开不受保护"。
///
/// # Errors
///
/// 同 [`lock`]。
pub fn acquire_lock(project_path: &Path, read_only: bool) -> Result<AcquiredLock, Fault> {
    let mode = if read_only {
        LockMode::SharedRead
    } else {
        LockMode::ExclusiveWrite
    };
    lock(project_path, mode)
}

/// [`LockError`] → [`Fault`] 的**唯一**映射。
///
/// | OS 层事实 | 契约出口 | 为什么 |
/// | :--- | :--- | :--- |
/// | `WouldBlock` | `ToolResponse.error.code = PROJECT_LOCKED` | 唯一的锁领域码（`ADR-0001 D25` 联集） |
/// | `UnsupportedPlatform` | JSON-RPC `-32005`（实现级） | 不是领域失败；契约 enum 里**没有**也不该有"平台不支持" |
/// | `Io` | `IO_ERROR` / `FILE_NOT_FOUND` / `DISK_FULL` | 复用既有 `io::ErrorKind` 映射 |
#[must_use]
pub fn lock_fault(project_path: &Path, error: LockError) -> Fault {
    match error {
        LockError::WouldBlock => locked_fault(project_path),
        LockError::UnsupportedPlatform { os, spec_id } => Fault::implementation(
            ErrorObject::new(
                NOT_IMPLEMENTED,
                format!("平台 `{os}` 没有可用的 OS 建议锁, 拒绝打开以防并发写坏工程"),
            )
            .with_data(serde_json::json!({
                "code": ErrorCode::NotImplemented.as_str(),
                "specId": spec_id,
                "os": os,
                "detail": "ARCH-SEC-001 的 OS 建议锁在本平台没有实现; 本实现刻意不静默放行",
                "lockFile": lock_path(project_path).display().to_string(),
            })),
        ),
        LockError::Io(error) => from_io(
            &format!("获取建议锁 {}", lock_path(project_path).display()),
            &error,
        ),
    }
}

/// `PROJECT_LOCKED` 的载荷（带持有者元数据 + 建议锁诊断，便于人眼排查）。
#[must_use]
pub fn locked_fault(project_path: &Path) -> Fault {
    let lock = lock_path(project_path);
    let (holder, parsed) = read_metadata(project_path);
    let heartbeat_age = parsed
        .as_ref()
        .map(LockMetadata::heartbeat_age_secs)
        .unwrap_or(0);
    Fault::domain_with_data(
        ErrorCode::ProjectLocked,
        format!(
            "工程已被 OS 建议锁占用 (建议锁由内核持有, 持有者进程死亡时会自动释放): {}",
            lock.display()
        ),
        serde_json::json!({
            "lockFile": lock.display().to_string(),
            "holder": holder,
            "holderPid": parsed.as_ref().map(|meta| meta.pid),
            "holderMode": parsed.as_ref().map(|meta| meta.lock_mode.clone()),
            "heartbeatAgeSecs": heartbeat_age,
            "staleHeartbeatSecs": STALE_HEARTBEAT_SECS,
            "advisoryLockHeld": true,
        }),
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
        let guard = lock(&project, LockMode::ExclusiveWrite)
            .expect("首次加锁")
            .guard;
        assert!(guard.path().is_file());
        let holder = fs::read_to_string(guard.path()).expect("读锁元数据");
        assert!(
            holder.contains("\"lock_mode\": \"ExclusiveWrite\""),
            "{holder}"
        );
        assert_eq!(
            read_metadata(&project).1.map(|meta| meta.pid),
            Some(std::process::id()),
            "锁内容必须记录真实持有者 PID"
        );
        // 第二次加锁必须被内核拦下 (不是"文件存在" —— 是建议锁)。
        let second = acquire_lock(&project, false).expect_err("锁被占用");
        assert_eq!(second.domain_code(), Some(ErrorCode::ProjectLocked));
        // 只读打开也要观察到排他锁。
        let read_only = acquire_lock(&project, true).expect_err("只读也要拒绝");
        assert_eq!(read_only.domain_code(), Some(ErrorCode::ProjectLocked));
        drop(guard);
        assert!(!lock_path(&project).exists(), "排他 Drop 必须释放锁文件");
        let again = lock(&project, LockMode::ExclusiveWrite).expect("释放后可以重新加锁");
        assert!(
            !again.took_over_stale_lock(),
            "正常释放之后重新加锁不是接管"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_only_open_takes_a_shared_lock_and_peers_coexist() {
        // 旧实现的行为是"只读打开不建锁文件、什么都不锁" —— 那等于只读打开
        // **完全不受保护**（写者照样能改同一个工程）。新实现让读者锁在**同一个
        // inode**（锁文件）上取共享锁：读者之间共存，读者与写者互斥。
        let dir = scratch("readonly");
        let project = dir.join("demo.yeban");
        fs::write(&project, "{}").expect("占位");
        let first = acquire_lock(&project, true).expect("第一个读者");
        assert_eq!(first.guard.mode(), LockMode::SharedRead);
        let second = acquire_lock(&project, true).expect("第二个读者必须共存");
        assert_eq!(second.guard.mode(), LockMode::SharedRead);
        // 写者必须在读者持有期间被拒绝（否则"只读"只是口号）。
        let writer = acquire_lock(&project, false).expect_err("写者必须被读者挡住");
        assert_eq!(writer.domain_code(), Some(ErrorCode::ProjectLocked));
        drop(first);
        drop(second);
        // 读者不删锁文件（删了会制造 check-then-lock 竞态窗口），
        // 但它**没有持有者** ⇒ 可被接管，不是永久锁。
        let writer = lock(&project, LockMode::ExclusiveWrite).expect("读者退出后写者可接管");
        assert!(
            writer.took_over_stale_lock(),
            "残留的读锁文件必须被判为可接管"
        );
        drop(writer);
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
