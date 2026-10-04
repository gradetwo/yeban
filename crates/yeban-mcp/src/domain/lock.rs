//! **操作系统级建议锁**：`.yeban.lock` 的跨平台封装 [ARCH-SEC-001, MUST-GATE-008]。
//!
//! ## 规范原文与实现事实（先核验，后写码）
//!
//! `ARCH-SEC-001` 第 2 条的字面要求是 "Unix 调 `fcntl(fd, F_SETLK, &fl)`，
//! Windows 调 `LockFileEx`"。本机对 **stable Rust 1.99.0** 的 std 做了实测核验
//! （rustdoc 随工具链自带，见 `docs/ledger/lock-advisory-notes.md` §1），事实是：
//!
//! | 事实 | 出处 |
//! | :--- | :--- |
//! | `File::try_lock() -> Result<(), TryLockError>`、`File::try_lock_shared()` **1.89.0 稳定** | `rustdoc std::fs::struct.File` |
//! | `File::lock()` / `File::lock_shared()`（阻塞版）与 `File::unlock()` **同为 1.89.0 稳定** | 同上 |
//! | Unix 上它**就是** `flock(2)`（`LOCK_EX`/`LOCK_SH` + `LOCK_NB`），**不是** `fcntl(F_SETLK)` | rustdoc "Platform-specific behavior" |
//! | Windows 上它**就是** `LockFileEx`（`LOCKFILE_EXCLUSIVE_LOCK` + `LOCKFILE_FAIL_IMMEDIATELY`） | 同上 |
//! | `TryLockError::{WouldBlock, Error}` —— `WouldBlock` 明确表示"被别的句柄/进程持有" | rustdoc `std::fs::enum.TryLockError` |
//!
//! ⇒ **零新增依赖**：不需要 `libc`，也不需要 `windows-sys`。本 crate 的
//! `Cargo.toml` 只多了一行 `rust-version` 已经满足的事实（工具链 1.99.0 ≥ 1.89.0），
//! 没有动依赖图，因此 `docs/ledger/dependency-licenses.md` 不变。
//!
//! ### 为什么 `flock(2)` 比 `fcntl(F_SETLK)` 更符合本门禁的意图
//!
//! 两者都是建议锁，但在**崩溃自愈**这一点上语义不同（`MUST-GATE-008` 的
//! "崩溃留下永久锁"正是要修的病）：
//!
//! | | `fcntl(F_SETLK)` (POSIX record lock) | `flock(2)`（本实现） |
//! | :--- | :--- | :--- |
//! | 进程死亡（含 `SIGKILL`）时释放 | 是 | 是 |
//! | **关闭 fd** 时释放 | **否** —— 锁属于 (进程, inode)，`close` 不放 | 是（`LOCK_UN` / 最后一个 fd 关闭） |
//! | 进程内多个 fd 互相争用 | **否** —— 同进程对同一 inode 的 fcntl 锁互不冲突 | **是**（实测 S1） |
//!
//! 第二行是决定性的：`fcntl` 的"同进程 fd 之间不冲突"意味着**同一进程开两次**
//! 同一个工程拦不住（任务书场景 ①）；第三行则是 `MUST-GATE-008` 的
//! "并发读写立即抛出 `PROJECT_LOCKED`" 的前提。本实现的平台层因此是 `flock`，
//! `fcntl` 只作为**记录在案的偏差**（见 §"与规范的偏差"）。
//!
//! ## 三个场景的语义（判据见 `tests/lock_advisory.rs`）
//!
//! 1. **同一进程两个打开者**：`flock` 在 fd 粒度上判所有权，两个独立
//!    `File` 句柄（即两个 `Domain`）第二个必得 `WouldBlock` ⇒ `PROJECT_LOCKED`。
//!    *本线实测*：同一个进程内 `try_lock` 两个 fd，第二个 `Err(WouldBlock)`。
//! 2. **跨进程**：内核按 inode 仲裁，另一进程的另一 fd 同样 `WouldBlock`。
//!    *本线实测*：子进程（`std::process::Command` 跑同一个测试二进制 +
//!    隐藏参数）在父进程持锁时拿到 `WouldBlock`。
//! 3. **崩溃遗留陈旧锁**：进程死亡（含 `SIGKILL`）时内核**自动释放** `flock`，
//!    但锁文件**留在磁盘上**。 ⇒ **"文件存在"绝不是"被占用"的证据**，
//!    "能拿到建议锁"才是"没有活着的持有者"的证据。
//!    拿到锁之后立刻**原子地重写**锁内容（PID/时间戳/工程路径）。
//!    *本线实测*：`SIGKILL` 子进程后父进程 `try_lock` 立刻 `Ok`，
//!    且锁文件确实还在（这正是旧实现会永久锁死的那一步）。
//!
//! ## 原子创建与建议锁的先后（崩溃/竞态矩阵）
//!
//! 做法：**先 `create_new(true)` 原子创建**，**再**对 fd 加建议锁，**两者都成功**
//! 才算拿到锁。谁先谁后在崩溃点上的差别（这张表是设计的承重部分）：
//!
//! | 崩溃/竞态点 | 磁盘状态 | 下一个打开者看到什么 | 判定 |
//! | :--- | :--- | :--- | :--- |
//! | `create_new` 之前 | 无锁文件 | 自己创建 + 加锁 | 可打开 |
//! | `create_new` 成功，`try_lock` 之前 | **空锁文件**（0 字节） | 能拿到锁（无持有者） | 可打开并接管 — **正确** |
//! | `try_lock` 成功，写元数据之前 | 空锁文件 + 活锁 | `WouldBlock` | `PROJECT_LOCKED`（持有者活着） |
//! | 写元数据中（截断后 / 写完前） | **半截 JSON** | `WouldBlock` | `PROJECT_LOCKED`；元数据仅供人眼，**不参与判定** |
//! | 持有中崩溃（含 `SIGKILL`） | 完整元数据 + **无锁** | 能拿到锁 | 可打开并**接管重写** |
//! | 正常 `Drop`（排他） | 文件被删除 | 无锁文件 | 可打开 |
//!
//! 关键性质：**任何一行都不会永久锁死**。"空文件/半截 JSON/完整元数据但无人持有"
//! 三种残骸的处置完全相同 —— 拿得到建议锁就接管。旧实现（存在即拒）在三行的
//! 后两类上会永久拒绝打开，这就是 `MUST-GATE-008` 要修的病。
//!
//! 反过来，"存在即占用"被**显式否决**的判据是
//! `tests/lock_advisory.rs::crashed_holder_leaves_a_lock_file_that_is_not_occupied`
//! （造一份无人持有的锁文件 ⇒ 必须能打开；把实现改回"看一眼文件是否存在"它就变红）。
//!
//! ## 心跳与陈旧阈值（规范 §0.2 第 5 条的处理）
//!
//! 规范要求"每 3 秒刷新 `last_heartbeat`，超过 15 秒或 `kill(pid, 0)` 失败则允许接管"。
//! 本实现**按单调性收窄**了它，理由是一条硬性质：
//!
//! > 内核在进程死亡时释放 `flock` ⇒ **"建议锁被持有"是"持有者活着"的充分证据**；
//! > 反之不成立（持有者可能活着但心跳陈旧）。
//!
//! 因此本实现里**心跳不参与任何判定**，`last_heartbeat` 只是写给人看的诊断字段：
//!
//! - **锁被持有** ⇒ 一律 `PROJECT_LOCKED`，**无论**心跳多陈旧。
//!   （规范允许"心跳 > 15s ⇒ 接管"，但那需要 `kill(pid, 0)` 二次确认；
//!   心跳本身是可在崩溃前就已经陈旧的弱证据 —— 若只凭心跳接管，
//!   一个被 `SIGSTOP` 挂起或调度被饿死的活持有者会被强行抢锁。
//!   本线选择**不实现**基于心跳的抢占：宁可拒绝，也不误抢。
//!   这是**收窄**，不是遗漏，登记为 pending P2。）
//! - **锁没有被持有** ⇒ 一律可接管，**无论**锁文件里写着多新鲜的心跳。
//!   即 `STALE_HEARTBEAT_SECS` 描述的阈值在**当前实现里不是判据**。
//!
//! 为了让规范里的 15 秒仍然有一个可核对的出口，本模块提供
//! [`LockMetadata::heartbeat_age_secs`] 与常量 [`HEARTBEAT_INTERVAL_SECS`] /
//! [`STALE_HEARTBEAT_SECS`]：它们只在**诊断载荷**（`PROJECT_LOCKED` 的 `data`）里
//! 出现，供 UI 提示"持有者可能已卡死"，不改变判定。
//!
//! ## 与规范的偏差（登记，不隐藏）
//!
//! | 偏差 | 规范原文 | 本实现 | 理由 |
//! | :--- | :--- | :--- | :--- |
//! | 锁原语是 `flock(2)` 而不是 `fcntl(F_SETLK)` | §0.2 第 2 条 | `std::fs::File::try_lock` ⇒ `flock` | `fcntl` 同进程 fd 不互相冲突（实测），拦不住场景 ① |
//! | `F_GETLK` 风格的"查出被谁持有" | §0.2 第 2 条 | `flock` 无此查询；从**锁文件内容**读出持有者元数据 | 语义等价（谁持有 + 什么模式），且不需要 `unsafe`/`libc` |
//! | 心跳不参与判定 | §0.2 第 5 条（3s/15s/`kill(pid,0)`） | 见上（收窄） | 建议锁是单调的活体证据 |
//! | `--force-unlock` | §0.2 第 5 条 | 未实现（接管是**隐式**的：拿到锁即接管） | 接管不需要"强制"开关，残留锁自动可接管 |
//!
//! ## 平台矩阵
//!
//! | 平台 | 行为 |
//! | :--- | :--- |
//! | Unix（macOS/Linux，CI 的 x86_64 + aarch64） | `flock(2)` 真锁，**本机实测 + CI 编译** |
//! | Windows | `LockFileEx`（std 内部），**从未在本机编译过/跑过** ⇒ 登记 pending P1 |
//! | 其它（wasm 等） | **显式** [`LockError::UnsupportedPlatform`] ⇒ JSON-RPC 实现级 `-32005`；**绝不静默放过** |

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// 锁文件后缀（`demo.yeban` → `demo.yeban.lock`）。
pub const LOCK_SUFFIX: &str = ".lock";

/// 规范 §0.2 第 5 条的续约间隔（秒）。当前实现不发送心跳，常量在此把规范值钉住。
pub const HEARTBEAT_INTERVAL_SECS: u64 = 3;

/// 规范 §0.2 第 5 条的陈旧阈值（秒）。**当前实现里不是接管判据**（见模块头）。
pub const STALE_HEARTBEAT_SECS: u64 = 15;

/// 锁模式（`ARCH-SEC-001` 第 3 条的双模式）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockMode {
    /// `EXCLUSIVE_WRITE`：交互式主会话、破坏性编辑 CLI ⇒ 排他写锁。
    ExclusiveWrite,
    /// `SHARED_READ`：离线分析、批处理导出、多 Agent 并行只读审查 ⇒ 共享读锁。
    SharedRead,
}

impl LockMode {
    /// 写进锁元数据 `lock_mode` 字段的规范字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExclusiveWrite => "ExclusiveWrite",
            Self::SharedRead => "SharedRead",
        }
    }

    /// 排他模式。
    #[must_use]
    pub const fn is_exclusive(self) -> bool {
        matches!(self, Self::ExclusiveWrite)
    }
}

/// 工程文件的锁文件路径（`<工程文件名>.lock`，见 `docs/ledger/tools-domain-notes.md` needs-7）。
#[must_use]
pub fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || std::ffi::OsString::from("project"),
        std::ffi::OsStr::to_os_string,
    );
    name.push(LOCK_SUFFIX);
    path.with_file_name(name)
}

/// 建议锁的失败分类。
///
/// 刻意**不**在这里直接产出契约错误码：本模块只管"OS 层面发生了什么"，
/// 到 `PROJECT_LOCKED` 的映射是 [`crate::domain::store`] 的唯一职责
/// （`ADR-0001 D25` 的 20 值联集）。
#[derive(Debug)]
pub enum LockError {
    /// 锁已被**别的活着的持有者**占用（内核仲裁，含跨进程与同进程另一个 fd）。
    WouldBlock,
    /// 本平台没有实现建议锁 ⇒ 显式失败，**绝不静默放过**。
    UnsupportedPlatform {
        /// `std::env::consts::OS`。
        os: &'static str,
        /// 规范 ID，便于在 JSON-RPC `data` 里定位。
        spec_id: &'static str,
    },
    /// 真实的 I/O 失败（权限、磁盘、坏 fd……）。
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WouldBlock => write!(f, "文件建议锁已被其它持有者占用"),
            Self::UnsupportedPlatform { os, spec_id } => write!(
                f,
                "平台 `{os}` 没有实现 OS 建议锁 ({spec_id})；为避免静默放任并发写, 拒绝打开"
            ),
            Self::Io(error) => write!(f, "文件建议锁系统调用失败: {error}"),
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::WouldBlock | Self::UnsupportedPlatform { .. } => None,
        }
    }
}

/// 锁文件里那份**给人看的**元数据协议（`ARCH-SEC-001` 第 4 条）。
///
/// 它**不参与任何判定**（判定只看建议锁），只用于 `PROJECT_LOCKED` 的
/// 诊断载荷与 `--force-unlock` 式的人眼排查。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockMetadata {
    /// 持有者 PID。
    pub pid: u32,
    /// 持有者主机名。
    pub hostname: String,
    /// 持有者应用版本。
    pub app_version: String,
    /// `ExclusiveWrite` / `SharedRead`。
    pub lock_mode: String,
    /// 加锁时刻（Unix 秒）。
    pub started_at: u64,
    /// 最近心跳（Unix 秒）。当前实现只在获取锁时写一次。
    pub last_heartbeat: u64,
    /// 被锁的工程文件路径（规范 §0.2 示例里没有，但它是接管审计的关键字段）。
    pub project_path: String,
}

impl LockMetadata {
    /// 为一次成功的加锁构造元数据。
    #[must_use]
    pub fn new(project_path: &Path, mode: LockMode) -> Self {
        let now = unix_secs_now();
        Self {
            pid: std::process::id(),
            hostname: hostname(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            lock_mode: mode.as_str().to_owned(),
            started_at: now,
            last_heartbeat: now,
            project_path: project_path.display().to_string(),
        }
    }

    /// 解析锁文件内容。坏 JSON / 半截文件 ⇒ `None`（**不是**错误：
    /// 元数据只是给人看的，损坏的元数据不改变"锁是否被持有"的判定）。
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        Some(Self {
            pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
            hostname: value
                .get("hostname")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            app_version: value
                .get("app_version")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            lock_mode: value
                .get("lock_mode")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            started_at: value.get("started_at").and_then(Value::as_u64).unwrap_or(0),
            last_heartbeat: value
                .get("last_heartbeat")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            project_path: value
                .get("project_path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        })
    }

    /// 心跳距当前时刻的秒数（`0` 表示"就在刚刚"；时钟回拨按 `0` 处理）。
    ///
    /// 只用于诊断载荷 —— **当前实现不用它做接管判定**（见模块头）。
    #[must_use]
    pub fn heartbeat_age_secs(&self) -> u64 {
        unix_secs_now().saturating_sub(self.last_heartbeat)
    }

    /// 序列化成落盘文本（美化 JSON + 结尾换行）。
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut json = serde_json::to_string_pretty(&serde_json::json!({
            "pid": self.pid,
            "hostname": self.hostname,
            "app_version": self.app_version,
            "lock_mode": self.lock_mode,
            "started_at": self.started_at,
            "last_heartbeat": self.last_heartbeat,
            "project_path": self.project_path,
        }))
        .unwrap_or_else(|_| String::from("{}"));
        json.push('\n');
        json
    }
}

/// 主机名（读不到就用 `unknown` —— 锁元数据不是承重信息）。
#[must_use]
pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| String::from("unknown"))
}

/// 当前 Unix 秒。
fn unix_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |delta| delta.as_secs())
}

// ---------------------------------------------------------------------------
// 平台层：std 的 `File::try_lock*`（Unix = flock(2)，Windows = LockFileEx）
// ---------------------------------------------------------------------------

/// Unix（macOS / Linux）：`std::fs::File::try_lock` 就是 `flock(2)`。
#[cfg(unix)]
mod platform {
    use super::LockError;

    /// 尝试取排他建议锁（非阻塞）。
    pub(super) fn try_lock_exclusive(file: &std::fs::File) -> Result<(), LockError> {
        file.try_lock().map_err(convert)
    }

    /// 尝试取共享建议锁（非阻塞）。
    pub(super) fn try_lock_shared(file: &std::fs::File) -> Result<(), LockError> {
        file.try_lock_shared().map_err(convert)
    }

    /// 释放本 fd 上的建议锁。
    pub(super) fn release(file: &std::fs::File) -> Result<(), LockError> {
        file.unlock().map_err(LockError::Io)
    }

    fn convert(error: std::fs::TryLockError) -> LockError {
        match error {
            std::fs::TryLockError::WouldBlock => LockError::WouldBlock,
            std::fs::TryLockError::Error(error) => LockError::Io(error),
        }
    }
}

/// Windows：std 内部走 `LockFileEx`。
///
/// **本机（macOS）从未编译过这个分支** —— 它登记为 pending P1，
/// 由 CI 的 `windows-latest`（若启用）或人工核验后才算"验过"。
/// 注意 std 的告警：**只以 append 方式打开的文件锁不住**，
/// 因此 [`LockGuard::open_lock_file`] 一律用 `read(true).write(true)` 打开。
#[cfg(windows)]
mod platform {
    use super::LockError;

    /// 尝试取排他建议锁（非阻塞）。
    pub(super) fn try_lock_exclusive(file: &std::fs::File) -> Result<(), LockError> {
        file.try_lock().map_err(convert)
    }

    /// 尝试取共享建议锁（非阻塞）。
    pub(super) fn try_lock_shared(file: &std::fs::File) -> Result<(), LockError> {
        file.try_lock_shared().map_err(convert)
    }

    /// 释放本句柄上的建议锁。
    pub(super) fn release(file: &std::fs::File) -> Result<(), LockError> {
        file.unlock().map_err(LockError::Io)
    }

    fn convert(error: std::fs::TryLockError) -> LockError {
        match error {
            std::fs::TryLockError::WouldBlock => LockError::WouldBlock,
            std::fs::TryLockError::Error(error) => LockError::Io(error),
        }
    }
}

/// 其它平台（wasm 等）：**显式**不支持，绝不静默放过并发写。
#[cfg(not(any(unix, windows)))]
mod platform {
    use super::LockError;

    /// 规范 ID（`ARCH-SEC-001`）+ `MUST-GATE-008`。
    const SPEC_ID: &str = "ARCH-SEC-001/MUST-GATE-008";

    fn unsupported() -> LockError {
        LockError::UnsupportedPlatform {
            os: std::env::consts::OS,
            spec_id: SPEC_ID,
        }
    }

    pub(super) fn try_lock_exclusive(_file: &std::fs::File) -> Result<(), LockError> {
        Err(unsupported())
    }

    pub(super) fn try_lock_shared(_file: &std::fs::File) -> Result<(), LockError> {
        Err(unsupported())
    }

    pub(super) fn release(_file: &std::fs::File) -> Result<(), LockError> {
        Err(unsupported())
    }
}

// ---------------------------------------------------------------------------
// 守卫
// ---------------------------------------------------------------------------

/// 持有中的 `.yeban.lock`（**原子创建 + OS 建议锁**，`Drop` 释放）。
///
/// RAII 语义：`file` 字段的存在就是"建议锁活着"的物理证据 ——
/// `flock` 在最后一个指向该 inode 的 fd 关闭时（含进程死亡）释放。
/// 因此**销毁守卫 = 释放锁**，连 `Drop` 都不需要显式 `unlock`
/// （本实现仍然显式 `unlock`，让"释放"在代码里可见，而不是一个隐式副作用）。
///
/// **刻意不实现 `Clone`**：克隆守卫等于克隆一个"谁先 `Drop` 谁删锁文件"的
/// 双重所有权。`Domain` 因此也不实现 `Clone`。
#[derive(Debug)]
pub struct LockGuard {
    /// 加锁的目标（工程文件路径，不是锁文件路径）。
    project_path: PathBuf,
    /// 锁文件路径。
    path: PathBuf,
    /// 锁模式。
    mode: LockMode,
    /// **持有建议锁的句柄**。字段顺序保证它在 `Drop` 之前一直活着。
    ///
    /// `Option` 只为 `Drop` 能 `take` 出来释放；正常情况下永远是 `Some`。
    file: Option<File>,
    /// 本次加锁是否是"接管崩溃遗留"。
    took_over_stale_lock: bool,
}

impl LockGuard {
    /// 锁文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 被锁的工程文件路径。
    #[must_use]
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    /// 锁模式。
    #[must_use]
    pub const fn mode(&self) -> LockMode {
        self.mode
    }

    /// 本次加锁是否接管了一份**崩溃遗留的陈旧锁文件**。
    ///
    /// 语义：锁文件本来就在，但**没有任何活着的持有者**（内核里没有对应的
    /// 建议锁），因此本进程拿到了建议锁并重写了它的内容。
    #[must_use]
    pub const fn took_over_stale_lock(&self) -> bool {
        self.took_over_stale_lock
    }

    /// 持有中的建议锁句柄（只读；用于让"锁一直活着"有可观察出口）。
    #[must_use]
    pub fn file(&self) -> Option<&File> {
        self.file.as_ref()
    }

    /// 读回锁文件内容（诊断用；坏内容返回原文而不是 `Err`）。
    #[must_use]
    pub fn lock_file_contents(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_else(|_| String::from("<不可读>"))
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // 释放失败不做任何事: `Drop` 里 panic 会让"关闭工程"变成崩溃。
        if let Some(file) = self.file.take() {
            let _ = platform::release(&file);
            drop(file);
        }
        // 只有**排他**持有者删除锁文件。
        //
        // 共享读者**不删**：删文件不能唤醒已经在等别的 inode 的竞争者
        // （典型 check-then-lock 竞态），反而制造"看起来释放了其实没有"的假象。
        // 读者留下的锁文件是**可接管**的（下一次拿排他锁即可接管并重写），
        // 所以它不会变成永久锁 —— 这正是 `MUST-GATE-008` 要的性质。
        if self.mode.is_exclusive() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ---------------------------------------------------------------------------
// 加锁
// ---------------------------------------------------------------------------

/// **原子创建 + OS 建议锁**。
///
/// 语义（`MUST-GATE-008`）：
///
/// 1. `OpenOptions::new().write(true).create_new(true)` **原子创建**锁文件
///    （`O_CREAT | O_EXCL`）；已存在则退化为普通打开（不是错误 —— 见下一步）。
/// 2. 对 fd 施加建议锁：排他模式 `try_lock()`，共享模式 `try_lock_shared()`。
/// 3. **两者都成功**才返回 `Ok(guard)`。
///
/// `took_over_stale_lock` 的判定：锁文件**本来就在**（`create_new` 报 `AlreadyExists`）
/// 而本次仍然拿到了排他建议锁 ⇒ 原持有者已经死了（内核释放了 `flock`），
/// 本进程接管并原子重写锁内容。
///
/// # Errors
///
/// - 锁被别的活着的持有者占用 → [`LockError::WouldBlock`]（调用方映射成 `PROJECT_LOCKED`）；
/// - 平台无建议锁 → [`LockError::UnsupportedPlatform`]；
/// - 文件系统失败 → [`LockError::Io`]。
pub fn acquire(project_path: &Path, mode: LockMode) -> Result<LockGuard, LockError> {
    let lock_file = lock_path(project_path);
    let (file, created) = open_lock_file(&lock_file)?;
    // `created == false` ⇒ 文件本来就在。此刻"是否陈旧"还没有结论 ——
    // 它取决于下一行能不能拿到建议锁。
    let took_over_stale_lock = !created && mode.is_exclusive();

    let locked = if mode.is_exclusive() {
        platform::try_lock_exclusive(&file)
    } else {
        platform::try_lock_shared(&file)
    };
    // 刻意展开成 `if let Err(...) { return Err(...) }` 而不是 `locked?`：
    // 这里有一个**必须写下来**的负向决定 —— 加锁失败时**不删锁文件**
    // （`WouldBlock` 说明别人正持有它，`Io` 说明我们连自己的创建都没落成；
    //  两种情况都不该由失败者去动那个文件。一个 0 字节的残留锁文件是无害的：
    //  它没有任何持有者，下一次打开会拿到建议锁并接管重写）。
    #[allow(clippy::question_mark)]
    if let Err(error) = locked {
        return Err(error);
    }

    let mut guard = LockGuard {
        project_path: project_path.to_path_buf(),
        path: lock_file,
        mode,
        file: Some(file),
        took_over_stale_lock,
    };

    // 排他模式才重写内容：共享读者不能截断别人正在写的元数据。
    if mode.is_exclusive()
        && let Err(error) = rewrite_metadata(guard.file.as_ref(), project_path, mode)
    {
        // 元数据写失败 ⇒ 撤掉这次加锁（不能留下"锁是我的但内容是别人的/半截的"）。
        guard.file = None;
        let _ = std::fs::remove_file(&guard.path);
        return Err(LockError::Io(error));
    }
    Ok(guard)
}

/// 打开（必要时原子创建）锁文件。返回 `(file, 是否由本次创建)`。
fn open_lock_file(lock_file: &Path) -> Result<(File, bool), LockError> {
    // Unix 的 `flock` 与 Windows 的 `LockFileEx` 都要求句柄可读或可写；
    // Windows 的 append-only 句柄**锁不住**，因此一律 read + write。
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(lock_file)
    {
        Ok(file) => Ok((file, true)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_file)
            .map(|file| (file, false))
            .map_err(LockError::Io),
        Err(error) => Err(LockError::Io(error)),
    }
}

/// **原子地**把锁内容重写成当前持有者的元数据（`ftruncate` + `write` + `fsync`）。
fn rewrite_metadata(
    file: Option<&File>,
    project_path: &Path,
    mode: LockMode,
) -> std::io::Result<()> {
    let Some(file) = file else {
        return Err(std::io::Error::other("锁句柄已经不在守卫里"));
    };
    let metadata = LockMetadata::new(project_path, mode);
    file.set_len(0)?;
    (&*file).write_all(metadata.to_json().as_bytes())?;
    file.sync_all()
}

/// 读锁文件里的持有者元数据（诊断用）。返回 `(原文, 解析结果)`。
#[must_use]
pub fn read_metadata(project_path: &Path) -> (String, Option<LockMetadata>) {
    let path = lock_path(project_path);
    match File::open(&path).and_then(|mut file| {
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(text)
    }) {
        Ok(text) => {
            let parsed = LockMetadata::parse(&text);
            (text, parsed)
        }
        Err(_) => (String::from("<不可读>"), None),
    }
}
