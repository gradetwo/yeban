//! 工程文件读写（`.yeban` **ZIP 容器**是**唯一**工程格式）、**原子落盘**与
//! `.yeban.lock` 排他锁 [ARCH-SEC-001, ARCH-SEC-003, ARCH-SEC-004, ADR-0001 D43]。
//!
//! ## 落盘格式：`ARCH-SEC-003` 的容器（本模块的字节来源）
//!
//! 规范 §5.3 要求 `.yeban` 是 **ZIP 容器**：`project.json` + `history.dag` +
//! `assets/{sha256}`。字节由 [`yeban_model::container::write_project_container`] 产出
//! （只写 `stored` 条目的标准 ZIP，含 Zip-Slip 与解压炸弹两条 MUST 防御），
//! 本模块**不自己拼 ZIP**，只负责"把哪份工程 / 哪份 DAG / 哪些 CAS 资产交给它"。
//!
//! | 容器条目 | 内容 | 口径 |
//! | :--- | :--- | :--- |
//! | `project.json` | `serde_json::to_vec(YebanProjectV1)`（**紧凑** JSON，`BTreeMap` 键序） | `schemas/project.schema.json` 的实例 |
//! | `history.dag` | `serde_json::to_vec(CommitGraph)` | 模型层 `CommitGraph` 已有 `Serialize` ⇒ 不另造私有格式；空图谱写 `{}` |
//! | `assets/{sha256}` | 会话 CAS 池里的原始字节 | 条目名 = `AssetHash` 的 64 位小写十六进制；写出与读回**双向**校验 SHA-256 |
//!
//! 工程**内容摘要**（`digest_of` / `saved_digest` / 未保存标记）仍然以
//! [`serialize_project`] 的规范化 JSON 为准 —— 它是"工程内容"的稳定指纹，
//! 与落盘容器字节解耦。这样"容器字节变了"与"工程内容变了"是两个可分别讨论的量。
//!
//! ## 读：`.yeban` 容器是**唯一**格式（`ADR-0001 D43`）
//!
//! 本模块曾经有一条"裸 JSON 兼容读路径"：字节没有 ZIP 魔数就按 UTF-8 +
//! `serde_json` 读。D43 明确"1.0.0 之前没有历史包袱与兼容需求，发现问题或更优解
//! **直接推翻**"，于是它连同形态枚举、错误分类与"看起来像 ZIP 就绝不掉进 JSON 分支"
//! 的补丁式判断**整段删除**（判据**反转**，见 `tests/container_store.rs`）。
//! `yeban-app` 侧的同一条路径已先删（`OpenError::NotAYebanContainer`，台账
//! `docs/ledger/app-no-compat-notes.md`）—— 两侧语义一致：**非容器文件 = 明确的
//! 拒绝 + 精确原因**，既不是"打开成空工程"，也不是泛化的"未知格式"。
//!
//! ```text
//! load_project(path)
//!   ├─ read_project_container(bytes) 成功 ⇒ 版本门 + validate()      【唯一被接受的形态】
//!   ├─ 失败且字节有 ZIP 结构（PK\x03\x04 / PK\x05\x06 / PK\x07\x08）
//!   │                                     ⇒ container_fault          【"你的 .yeban 坏了"】
//!   └─ 失败且连 ZIP 结构都没有             ⇒ not_a_container_fault    【"你给的不是 .yeban 容器"】
//! ```
//!
//! 分档只服务**诊断**，不服务兼容：截断的 `.yeban` 前 4 字节仍是 `PK\x03\x04`，
//! 因此落在"坏了"那一档，拿到精确的容器裁决。**没有任何路径会返回空工程。**
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
//! `tests/container_store.rs::save_into_a_read_only_directory_keeps_the_original_container_bytes`
//! 用一个只读父目录 + 一份预先存在的容器证明这件事（注入"直接原地写"会让它变红）。
//! 落盘入口只有 [`write_project_atomic`] 一个 —— 换了写入内容（JSON → 容器字节）
//! 也没有换掉协议，见该函数的文档。
//!
//! ## 容器错误 → 契约错误码：**只用一个已存在的码**
//!
//! `ADR-0001 D25` 的 20 值联集是工具级错误码的**唯一**来源。容器层的全部
//! [`ContainerError`] 变体（以及"这份字节根本不是 `.yeban` 容器"这件事）都映射到
//! [`ErrorCode::IoError`]（`IO_ERROR`），分类（路径穿越 / 解压炸弹 / 不支持的压缩法 /
//! 结构畸形 / 布局不符）与规范 ID 走 `error.data.{category, specId, containerError}`，
//! **不发明新码**（见 [`container_fault`] / [`not_a_container_fault`] 与
//! [`container_rejection`] 的穷举匹配）。
//!
//! ## 锁：本模块的职责边界
//!
//! **锁的机制全部住在 [`super::lock`]**（`MUST-GATE-008`）：原子创建 + `flock(2)`
//! 建议锁 + 崩溃遗留接管 + 平台矩阵。本模块只保留**两个**职责：
//!
//! 1. 把锁文件路径/内容协议**再导出**（历史调用点 `store::lock_path` 等不变）；
//! 2. 把 [`super::lock::LockError`] **唯一地**映射到契约错误码 ——
//!    `WouldBlock`（含跨进程与同进程另一个 fd）→ `PROJECT_LOCKED`；
//!    平台无建议锁 → JSON-RPC 实现级 `-32005`（**不发明新错误码**）。

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use yeban_model::container::{
    ContainerError, ContainerLimits, MAX_ENTRY_NAME_BYTES, read_project_container,
    write_project_container,
};
use yeban_model::{AssetHash, CommitGraph, EntityId, YebanProjectV1};

use super::MAIN_BRANCH;
use super::error::{Fault, from_io};
use super::lock::{self, LockError};
use crate::jsonrpc::{ErrorObject, NOT_IMPLEMENTED};
use crate::tools::ErrorCode;

pub use super::lock::{
    HEARTBEAT_INTERVAL_SECS, LOCK_SUFFIX, LockFileSnapshot, LockGuard, LockMetadata, LockMode,
    STALE_HEARTBEAT_SECS, hostname, lock_path, read_metadata, read_snapshot,
};

/// 临时文件名的前缀字符（隐藏文件，且带 `.tmp-` 标记）。
pub const TEMP_INFIX: &str = ".tmp-";

/// `.yeban` 容器的 ZIP 本地文件头签名（`PK\x03\x04`）。
///
/// 这是"本实现写出的容器长什么样"的**唯一**判据：不用扩展名、不用文件大小、
/// 不用试探解析。它是 [`has_zip_signature`] 三签名里的第一个。
pub const CONTAINER_MAGIC: [u8; 4] = *b"PK\x03\x04";

/// 内容摘要（SHA-256 十六进制）—— 直接复用 `yeban-model` 的 CAS 哈希实现。
#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    AssetHash::of_bytes(bytes).as_str().to_owned()
}

/// `yeban_open_project` / `yeban_save_project` 响应里 `format` 的**唯一**取值。
///
/// `ADR-0001 D43` 之后 `.yeban` 容器是唯一的工程格式，这里刻意**没有**一个
/// "文档形态"枚举：一个只剩单个变体的枚举只会邀请人再往里面塞一个变体回来。
/// 常量把"读法只有一条"写死成事实 —— 判据据此断言报告里**不可能**出现第二个值。
/// `yeban-app` 侧的同名常量 `open::DOCUMENT_FORMAT` 是同一件事
/// （见 `docs/ledger/app-no-compat-notes.md`）。
pub const DOCUMENT_FORMAT: &str = "yeban-container";

/// 一次加载的完整结果（工程 + 容器里另外两类条目）。
#[derive(Debug, Clone)]
pub struct LoadedProject {
    /// 已通过版本门与 `validate()` 的工程文档。
    pub project: YebanProjectV1,
    /// 文件字节数（**容器字节**，不是 `project.json` 的长度）。
    pub bytes: u64,
    /// `history.dag` 解析出的提交图谱；空图谱为 `None`。
    pub graph: Option<CommitGraph>,
    /// `assets/{sha256}` 解出的 CAS 资产池（键序确定）；没有资产时为空。
    pub assets: BTreeMap<AssetHash, Vec<u8>>,
}

/// 这份字节有没有 ZIP 结构（三种签名：local header / EOCD / data descriptor）。
///
/// **不是格式探测，也不是为兼容而写的护栏**：旧实现用它决定"要不要掉进裸 JSON
/// 分支"，那条分支已按 `ADR-0001 D43` 删除。它现在只服务一件事 —— 把拒绝分成
/// "**你给的不是 `.yeban` 容器**"（[`not_a_container_fault`]）与"**你的 `.yeban`
/// 坏了**"（[`container_fault`]）两档，而不是把两者糊成一句"无法识别的文件"。
/// 截断的 `.yeban` 前 4 字节仍是 [`CONTAINER_MAGIC`]，因此落在"坏了"那一档，
/// 拿到的是精确的容器裁决 —— 这与 `yeban-app` 的 `has_zip_signature` 逐条同义。
#[must_use]
pub fn has_zip_signature(bytes: &[u8]) -> bool {
    const SIGNATURES: [&[u8; 4]; 3] = [&CONTAINER_MAGIC, b"PK\x05\x06", b"PK\x07\x08"];
    SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature.as_slice()))
}

/// 容器层拒绝的五种性质（**全部**出口都是 [`ErrorCode::IoError`]）。
///
/// 分类不是为了产生新错误码，而是为了让 `IO_ERROR` 的载荷携带可诊断信息：
/// "为什么拒绝"在安全事件里与"拒绝了"同等重要。分类与规范 ID 的对应：
///
/// | 分类 | 规范 ID | 覆盖 |
/// | :--- | :--- | :--- |
/// | [`ContainerRejection::PathTraversal`] | `MUST-GATE-006` | 条目名安全（`..`/绝对路径/UNC/跨卷符号链接/控制字符……） |
/// | [`ContainerRejection::ArchiveBomb`] | `MUST-GATE-007` | 四道体积/比率闸门 + 条目数 |
/// | [`ContainerRejection::Unsupported`] | `ARCH-SEC-003` | deflate / ZIP64 / 加密 / data descriptor / 多卷 |
/// | [`ContainerRejection::Malformed`] | `ARCH-SEC-003` | 结构畸形、CRC 不符、local↔central 不一致 |
/// | [`ContainerRejection::Layout`] | `ARCH-SEC-003` | §5.3 布局不符（缺条目 / 资产哈希不符 / 工程 JSON 非法） |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRejection {
    /// Zip-Slip 一族（`MUST-GATE-006`）。
    PathTraversal,
    /// 解压炸弹一族（`MUST-GATE-007`）。
    ArchiveBomb,
    /// 本实现明确不支持的归档特性（压缩法 / ZIP64 / 加密……）。
    Unsupported,
    /// 归档结构本身畸形或内容被篡改。
    Malformed,
    /// 归档合法，但不满足 §5.3 的内容布局。
    Layout,
}

impl ContainerRejection {
    /// 规范字符串（进 `error.data.category`）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PathTraversal => "path-traversal",
            Self::ArchiveBomb => "archive-bomb",
            Self::Unsupported => "unsupported-container-feature",
            Self::Malformed => "malformed-container",
            Self::Layout => "container-layout",
        }
    }

    /// 这一分类的规范 ID。
    #[must_use]
    pub const fn spec_id(self) -> &'static str {
        match self {
            Self::PathTraversal => "MUST-GATE-006",
            Self::ArchiveBomb => "MUST-GATE-007",
            Self::Unsupported | Self::Malformed | Self::Layout => "ARCH-SEC-003",
        }
    }
}

/// [`ContainerError`] → 分类。
///
/// **穷举匹配，没有 `_` 兜底**（与 [`super::error::code_for_model`] 同一条纪律）：
/// 容器层新增一个变体时这里会**编译失败**，而不是悄悄落进某个默认分类。
#[must_use]
pub const fn container_rejection(error: &ContainerError) -> ContainerRejection {
    use ContainerError as E;
    match error {
        // ---- MUST-GATE-006 路径安全 ----
        E::EmptyEntryName
        | E::EntryNameTooLong { .. }
        | E::AbsoluteEntryPath { .. }
        | E::ParentDirSegment { .. }
        | E::CurrentDirSegment { .. }
        | E::EmptyPathSegment { .. }
        | E::DirectoryEntryUnsupported { .. }
        | E::BackslashInEntryName { .. }
        | E::ColonInEntryName { .. }
        | E::NulInEntryName { .. }
        | E::ControlCharInEntryName { .. }
        | E::TrailingDotOrSpaceSegment { .. }
        | E::WindowsReservedName { .. }
        | E::DuplicateEntryName { .. }
        | E::SymlinkEntryUnsupported { .. } => ContainerRejection::PathTraversal,

        // ---- MUST-GATE-007 解压炸弹 ----
        E::EntryTooLarge { .. }
        | E::EntryActualTooLarge { .. }
        | E::ArchiveTooLarge { .. }
        | E::ExpansionRatioExceeded { .. }
        | E::TooManyEntries { .. } => ContainerRejection::ArchiveBomb,

        // ---- 明确不支持的归档特性 ----
        E::UnsupportedMultiDisk
        | E::UnsupportedZip64
        | E::UnsupportedCompression { .. }
        | E::UnsupportedDataDescriptor { .. }
        | E::EncryptedEntryUnsupported { .. } => ContainerRejection::Unsupported,

        // ---- 结构畸形 / 篡改 ----
        E::EocdNotFound
        | E::TruncatedArchive
        | E::CentralDirectoryOutOfBounds
        | E::CentralDirectorySizeMismatch { .. }
        | E::TruncatedCentralDirectory { .. }
        | E::BadCentralDirectorySignature { .. }
        | E::EntryNameNotUtf8 { .. }
        | E::LocalHeaderOutOfBounds { .. }
        | E::BadLocalHeaderSignature { .. }
        | E::LocalCentralMismatch { .. }
        | E::TruncatedEntryData { .. }
        | E::StoredSizeMismatch { .. }
        | E::CrcMismatch { .. } => ContainerRejection::Malformed,

        // ---- §5.3 布局 / CAS 完整性 ----
        E::MissingProjectJson
        | E::MissingHistoryDag
        | E::InvalidProjectJson { .. }
        | E::UnexpectedContainerEntry { .. }
        | E::InvalidAssetName { .. }
        | E::AssetHashMismatch { .. }
        | E::ContainerSerialization { .. } => ContainerRejection::Layout,
    }
}

/// [`ContainerError`] → 契约 [`Fault`]（**唯一**映射；出口一律 `IO_ERROR`）。
///
/// 为什么一律 `IO_ERROR`：`ADR-0001 D25` 的联集是工具级错误码的唯一来源，
/// 里面**没有**也不该有"Zip-Slip"/"解压炸弹"/"不支持的压缩法"这些**容器内部**的码。
/// 把归档问题伪装成某个更"贴切"的领域码（例如 `PERMISSION_DENIED`）会让调用方
/// 按领域语义重试一件永远不会成功的事。诊断信息走 `data`，不挤进 `code`。
#[must_use]
pub fn container_fault(path: &Path, error: &ContainerError) -> Fault {
    let rejection = container_rejection(error);
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!(
            "`{}` 不是可接受的 .yeban 容器 [{} / {}]: {error}",
            path.display(),
            rejection.as_str(),
            rejection.spec_id()
        ),
        serde_json::json!({
            "specId": rejection.spec_id(),
            "category": rejection.as_str(),
            "containerError": format!("{error:?}"),
            "path": path.display().to_string(),
        }),
    )
}

/// **不是 `.yeban` 容器**的精确拒绝（`ADR-0001 D43`）。
///
/// 语义与 `yeban-app` 的 `OpenError::NotAYebanContainer` **一致**
/// （见 `docs/ledger/app-no-compat-notes.md`）：裸 `project.json`、随机字节、空文件
/// 都走这一支，**一个都不会被"顺手当成工程打开"**，也绝不退化成"打开成空工程"。
///
/// 载荷携带容器层的**原裁决**（通常是 [`ContainerError::EocdNotFound`]）与分类：
/// "不是容器"这件事本身也有一个精确原因，把它吞掉换成一句"无法识别的文件"
/// 就是在丢信息。错误码仍然只能是 [`ErrorCode::IoError`]（`ADR-0001 D25` 的联集里
/// 没有"非容器"这种容器内部码，诊断走 `data`）。
#[must_use]
pub fn not_a_container_fault(path: &Path, error: &ContainerError) -> Fault {
    let rejection = container_rejection(error);
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!(
            "`{}` 不是 `.yeban` 容器 (容器裁决: {error})",
            path.display()
        ),
        serde_json::json!({
            "specId": rejection.spec_id(),
            "category": rejection.as_str(),
            "containerError": format!("{error:?}"),
            "path": path.display().to_string(),
        }),
    )
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
///
/// `WouldBlock` **带走加锁前读到的**锁文件快照：诊断持有者时**不能**在失败后再去读一次
/// —— Windows 的 `LockFileEx` 是强制锁，那一次读必然失败（见 `lock.rs` 的平台矩阵）。
#[must_use]
pub fn lock_fault(project_path: &Path, error: LockError) -> Fault {
    match error {
        LockError::WouldBlock { snapshot } => locked_fault_with_snapshot(project_path, &snapshot),
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

/// `PROJECT_LOCKED` 的载荷（best-effort 读一次持有者元数据）。
///
/// ⚠ 这个便利入口在 Windows 上读不到持有者内容（强制锁）—— 需要精确口径时用
/// [`locked_fault_with_snapshot`]，它接受**加锁前**读到的快照。
#[must_use]
pub fn locked_fault(project_path: &Path) -> Fault {
    locked_fault_with_snapshot(project_path, &read_snapshot(project_path))
}

/// `PROJECT_LOCKED` 的载荷（带持有者元数据 + 建议锁诊断，便于人眼排查）。
///
/// `holderMetadata` 明确区分三种情况，**不把"读不到"糊成"没有持有者"**：
///
/// | 值 | 含义 |
/// | :--- | :--- |
/// | `available` | 读到了锁文件内容（Unix 恒成立） |
/// | `unavailable-on-this-platform` | 平台不允许看：Windows 的 `LockFileEx` 是强制锁 |
/// | `unavailable` | 其它读失败（权限、文件刚被删） |
#[must_use]
pub fn locked_fault_with_snapshot(project_path: &Path, snapshot: &LockFileSnapshot) -> Fault {
    let lock = lock_path(project_path);
    let parsed = snapshot.metadata();
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
            "holder": snapshot.holder_text(),
            // 诊断信息的**可读性口径**（平台事实, 不是错误）:
            // "我看不到持有者" 与 "没有持有者" 是两件事, 不能糊成一个 null。
            "holderMetadata": snapshot.availability(),
            "holderPid": parsed.as_ref().map(|meta| meta.pid),
            "holderMode": parsed.as_ref().map(|meta| meta.lock_mode.clone()),
            "heartbeatAgeSecs": heartbeat_age,
            "staleHeartbeatSecs": STALE_HEARTBEAT_SECS,
            "advisoryLockHeld": true,
        }),
    )
}

/// 容器**文件**大小的上限（`MUST-GATE-007` 在 I/O 层的落点）。
///
/// `ContainerLimits` 全部是"**解压后**体积 / 比率"的闸门，它们只在**字节已经进了内存**
/// 之后才生效（`read_container` 的入参是 `&[u8]`）。于是存在一个明显的绕过：
/// 先给一个 500 GB 的 `.yeban`，`fs::read` 会在任何闸门生效之前把进程 OOM 掉。
/// 防炸弹不该有一个"比你想象的更早"的入口，因此在读盘之前先按**文件字节数**拦一道。
///
/// 上界取"解压总量上限 + 全部元数据开销的宽松上界"：
///
/// ```text
/// max_total_bytes
///   + max_entries × (MAX_ENTRY_NAME_BYTES + 128) × 2   // local header 一份 + central directory 一份
///   + 4096                                             // EOCD（本实现不写注释字段）
/// ```
///
/// `stored` 子集里"压缩后字节 == 数据区长度"，所以合法容器的文件大小确实落在
/// `max_total_bytes` 附近，这个上界不会误伤任何**本实现写得出**的容器。
///
/// 固定余量刻意取得**小**（4 KiB 而不是 1 MiB）：判据要在可注入的紧上限下用几十 KB
/// 的文件触发这道闸门（`oversized_files_are_refused_before_they_are_read_into_memory`），
/// 余量一大就只能靠真造 8 GB 文件来测 —— 那就等于测不了。
#[must_use]
pub fn max_container_file_bytes(limits: &ContainerLimits) -> u64 {
    let entries = u64::try_from(limits.max_entries).unwrap_or(u64::MAX);
    let per_entry_metadata = (MAX_ENTRY_NAME_BYTES as u64 + 128) * 2;
    limits
        .max_total_bytes
        .saturating_add(entries.saturating_mul(per_entry_metadata))
        .saturating_add(4_096)
}

/// 文件**太大，连读都不读**（`MUST-GATE-007` 的 I/O 层闸门）。
fn oversized_file_fault(path: &Path, len: u64, max: u64) -> Fault {
    Fault::domain_with_data(
        ErrorCode::IoError,
        format!(
            "`{}` 有 {len} 字节, 超过容器文件上限 {max} 字节 —— 在读进内存之前就拒绝 (MUST-GATE-007)",
            path.display()
        ),
        serde_json::json!({
            "specId": "MUST-GATE-007",
            "category": ContainerRejection::ArchiveBomb.as_str(),
            "fileBytes": len,
            "maxFileBytes": max,
            "path": path.display().to_string(),
        }),
    )
}

/// **读取并校验**一个工程文件（只接受 `.yeban` 容器），使用规范默认上限。
///
/// `ADR-0001 D43` 之后容器是**唯一**工程格式，判定顺序是契约的一部分
/// （见 [`has_zip_signature`] 与 [`not_a_container_fault`]）：
///
/// ```text
/// read_project_container(bytes) 成功 ⇒ 版本门 + validate()        【唯一被接受的形态】
/// 失败且字节有 ZIP 结构              ⇒ container_fault           【"你的 .yeban 坏了"】
/// 失败且连 ZIP 结构都没有            ⇒ not_a_container_fault     【"你给的不是 .yeban 容器"】
/// ```
///
/// 版本门与结构校验的顺序（与错误码）没有变：
///
/// 1. [`YebanProjectV1::check_readable`] → `CONFLICT` 等（`from_model` 的唯一映射）；
/// 2. [`YebanProjectV1::validate`] → `OUT_OF_RANGE` / `CONFLICT` 等。
///
/// **任何失败路径都不返回空工程 / 默认工程。**
///
/// # Errors
///
/// - 不是普通文件 → `FILE_NOT_FOUND`；
/// - 读失败 → `IO_ERROR`（`DISK_FULL` 等由 [`super::error::code_for_io`] 判定）；
/// - 文件超过 [`max_container_file_bytes`] → `IO_ERROR`（`category = archive-bomb`）；
/// - 容器被拒（Zip-Slip / 炸弹 / 不支持的压缩法 / 篡改 / 布局不符）→ `IO_ERROR`
///   （载荷带 `category` / `specId`，见 [`container_fault`]）；
/// - 不是 `.yeban` 容器（裸 JSON / 随机字节 / 空文件）→ `IO_ERROR`
///   （载荷携带容器原裁决，见 [`not_a_container_fault`]）；
/// - 容器内工程的版本门 / 结构校验失败 → `CONFLICT` / `OUT_OF_RANGE` 等。
pub fn load_project(path: &Path) -> Result<LoadedProject, Fault> {
    load_project_with_limits(path, &ContainerLimits::default())
}

/// 同 [`load_project`]，但上限**可注入**（判据要在小文件上触发"文件过大"这一道）。
///
/// # Errors
///
/// 同 [`load_project`]。
pub fn load_project_with_limits(
    path: &Path,
    limits: &ContainerLimits,
) -> Result<LoadedProject, Fault> {
    if !path.is_file() {
        return Err(Fault::domain(
            ErrorCode::FileNotFound,
            format!("工程文件不存在或不是普通文件: {}", path.display()),
        ));
    }
    // 第一道：声明大小（fail-fast，不碰磁盘内容）。
    let max_file = max_container_file_bytes(limits);
    let declared = fs::metadata(path)
        .map_err(|error| from_io(&format!("读取元数据 {}", path.display()), &error))?
        .len();
    if declared > max_file {
        return Err(oversized_file_fault(path, declared, max_file));
    }
    let bytes =
        fs::read(path).map_err(|error| from_io(&format!("读取 {}", path.display()), &error))?;
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    // 第二道：实际读到的字节数（`metadata` 与 `read` 之间有 TOCTOU 窗口，文件可以变大）。
    if len > max_file {
        return Err(oversized_file_fault(path, len, max_file));
    }
    load_container(path, &bytes, len, limits)
}

/// 容器形态的加载 —— **唯一**的加载路径（`ARCH-SEC-003`）。
///
/// 失败分两档（`ADR-0001 D43`）：有 ZIP 结构 ⇒ 原样上报容器裁决（截断 / CRC /
/// Zip-Slip / 炸弹 / 缺件 / 容器内坏 JSON 都拿到精确分类）；连 ZIP 结构都没有
/// ⇒ [`not_a_container_fault`]（"你给的不是 `.yeban` 容器"）。
fn load_container(
    path: &Path,
    bytes: &[u8],
    len: u64,
    limits: &ContainerLimits,
) -> Result<LoadedProject, Fault> {
    let archive = match read_project_container(bytes, limits) {
        Ok(archive) => archive,
        Err(error) if !has_zip_signature(bytes) => {
            return Err(not_a_container_fault(path, &error));
        }
        Err(error) => return Err(container_fault(path, &error)),
    };
    archive
        .project
        .check_readable()
        .map_err(|error| super::error::from_model("工程版本门", &error))?;
    archive
        .project
        .validate()
        .map_err(|error| super::error::from_model("工程结构校验", &error))?;
    let graph = decode_history_dag(path, &archive.history_dag)?;
    Ok(LoadedProject {
        project: archive.project,
        bytes: len,
        graph,
        assets: archive.assets.into_iter().collect(),
    })
}

/// `history.dag` 的读取口径（`ARCH-OPS-002`）。
///
/// - 空条目 / 空图谱（`{}`，零提交） ⇒ `None` ⇒ 打开时建新的根提交；
/// - 合法 `CommitGraph` JSON ⇒ `Some(graph)`，**并且**必须有 `main` 分支：
///   没有主分支的 DAG 会让后续每一次 `yeban_propose_*` 都撞 `CONFLICT`，
///   这种"半个历史"必须在打开的那一刻就被拒绝，而不是留给下一个工具去踩。
/// - 非法 JSON ⇒ `IO_ERROR`（**不是**静默忽略：忽略等于把用户的历史悄悄丢掉）。
fn decode_history_dag(path: &Path, raw: &[u8]) -> Result<Option<CommitGraph>, Fault> {
    if raw.is_empty() {
        return Ok(None);
    }
    let graph: CommitGraph = serde_json::from_slice(raw).map_err(|error| {
        Fault::domain_with_data(
            ErrorCode::IoError,
            format!(
                "`history.dag` 不是合法的 CommitGraph JSON ({}): {error}",
                path.display()
            ),
            serde_json::json!({
                "specId": "ARCH-OPS-002",
                "containerEntry": yeban_model::container::HISTORY_DAG_NAME,
            }),
        )
    })?;
    if graph.commits.is_empty() {
        return Ok(None);
    }
    if !graph.branches.contains_key(MAIN_BRANCH) {
        return Err(Fault::domain_with_data(
            ErrorCode::IoError,
            format!(
                "`history.dag` 有 {} 条提交但没有 `{MAIN_BRANCH}` 分支 ({})",
                graph.commits.len(),
                path.display()
            ),
            serde_json::json!({
                "specId": "ARCH-OPS-002",
                "containerEntry": yeban_model::container::HISTORY_DAG_NAME,
                "branches": graph.branches.keys().collect::<Vec<_>>(),
            }),
        ));
    }
    Ok(Some(graph))
}

/// 把工程序列化成**规范化文本**（美化 JSON + 结尾换行）。
///
/// 这是 `projectDigest` / `saved_digest` / 未保存标记的**指纹口径**，
/// **不是**落盘字节 —— 落盘走 [`container_bytes`]（`ARCH-SEC-003`）。
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

/// 产出 `.yeban` 容器的**字节**（`ARCH-SEC-003`）：工程 + 提交图谱 + CAS 资产池。
///
/// `history.dag` 的口径是 `serde_json::to_vec(CommitGraph)` —— 模型层已有
/// `Serialize`/`Deserialize`，因此不另造一套私有格式（两份格式必然漂移）。
/// 资产来自会话 CAS 池：容器条目名由 [`AssetHash`] 决定，写出前
/// `write_project_container` 会**逐条重算 SHA-256** 并要求等于条目名。
///
/// # Errors
///
/// 图谱序列化失败、资产哈希与键不符、或容器层拒绝（条目名/条目数/ZIP32 上限）→ `IO_ERROR`。
pub fn container_bytes(
    path: &Path,
    project: &YebanProjectV1,
    graph: &CommitGraph,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
) -> Result<Vec<u8>, Fault> {
    let history_dag = serde_json::to_vec(graph).map_err(|error| {
        Fault::domain_with_data(
            ErrorCode::IoError,
            format!("序列化提交图谱失败: {error}"),
            serde_json::json!({ "specId": "ARCH-OPS-002" }),
        )
    })?;
    write_project_container(project, &history_dag, assets)
        .map_err(|error| container_fault(path, &error))
}

/// **原子**把 `bytes` 落盘到 `path`（同目录临时文件 + `fsync` + `rename`）。
///
/// 这是 `ARCH-SEC-004` 的**唯一**落盘入口 —— 写入内容从"JSON 文本"换成
/// "容器字节"时**协议一字未改**：临时文件名、同目录约束、`create_new`、
/// `sync_all`、`rename`、失败清理全部保持原样。
///
/// 失败时原文件**逐字节不变**，且临时文件被清理。
///
/// # Errors
///
/// 父目录不存在、无写权限、磁盘写满、`rename` 失败 → 对应契约错误码
/// （`IO_ERROR` / `DISK_FULL`）。
pub fn write_project_atomic(path: &Path, bytes: &[u8]) -> Result<(), Fault> {
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

    let outcome = write_then_replace(&temp, path, bytes);
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
fn write_then_replace(temp: &Path, target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    {
        let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
        file.write_all(bytes)?;
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
        // 持有者元数据必须**从守卫本身**读（内存里那一份）：
        // Windows 的 `LockFileEx` 是**强制**锁，"持锁后再读锁文件"必然失败
        // （`os error 33`），Unix 的 `flock` 是建议锁所以能读 —— 这条差异有专门判据
        // （`tests/lock_advisory.rs::holder_metadata_while_locked_is_platform_specific`），
        // 这里只断言**两个平台都成立**的那一半。
        let holder = guard.holder().expect("排他守卫必须携带自己写入的元数据");
        assert_eq!(holder.pid, std::process::id());
        assert_eq!(holder.lock_mode, "ExclusiveWrite");
        assert_eq!(holder.project_path, project.display().to_string());
        // Unix 附加：建议锁不阻止其它句柄 ⇒ 磁盘内容**也**必须是我们的
        // （证明"守卫携带的那一份"确实被写进了文件，而不是只活在内存里）。
        #[cfg(unix)]
        {
            let on_disk = fs::read_to_string(guard.path()).expect("flock 建议锁: 持锁时仍可读");
            assert_eq!(holder.to_json(), on_disk, "磁盘内容必须与守卫携带的一致");
            assert_eq!(
                read_metadata(&project).1.map(|meta| meta.pid),
                Some(std::process::id()),
                "锁内容必须记录真实持有者 PID"
            );
        }
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
        fs::write(&project, b"OLD\n").expect("原文件");
        write_project_atomic(&project, b"NEW\n").expect("保存");
        assert_eq!(fs::read(&project).expect("读"), b"NEW\n");
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
        let fault = write_project_atomic(&missing, b"x").expect_err("父目录不存在");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn container_round_trip_preserves_the_project_and_the_history() {
        let dir = scratch("roundtrip");
        let path = dir.join("demo.yeban");
        let model = yeban_model::samples::filled_project();
        let graph = CommitGraph::new();
        let assets = BTreeMap::new();
        let bytes = container_bytes(&path, &model, &graph, &assets).expect("容器字节");
        assert_eq!(
            &bytes[..4],
            CONTAINER_MAGIC.as_slice(),
            "落盘字节必须从 ZIP 本地文件头开始"
        );
        write_project_atomic(&path, &bytes).expect("保存");

        let loaded = load_project(&path).expect("读回");
        assert_eq!(loaded.project, model);
        assert_eq!(loaded.bytes as usize, bytes.len());
        assert!(loaded.graph.is_none(), "空图谱写出的 DAG 读回是 None");
        assert!(loaded.assets.is_empty());
        assert_eq!(
            digest_of(serialize_project(&model).unwrap().as_bytes()).len(),
            64
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// `ADR-0001 D43`：裸 JSON 不再有读路径 —— 必须被**精确拒绝**，
    /// 而**同一份 JSON 包进真容器仍然能读**（证明拒绝的是容器边界，不是内容）。
    #[test]
    fn bare_json_is_refused_as_not_a_container_and_the_same_json_in_a_container_still_opens() {
        let dir = scratch("no-compat");
        let path = dir.join("demo.yeban");
        let model = yeban_model::samples::filled_project();
        let text = serialize_project(&model).expect("序列化");
        fs::write(&path, &text).expect("写裸 JSON");

        let fault = load_project(&path).expect_err("裸 JSON 必须被明确拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        let value = fault.into_result().expect("领域失败是带内响应");
        assert_eq!(value["error"]["code"], "IO_ERROR");
        assert_eq!(value["error"]["data"]["category"], "malformed-container");
        assert_eq!(value["error"]["data"]["specId"], "ARCH-SEC-003");
        assert_eq!(value["error"]["data"]["containerError"], "EocdNotFound");
        let message = value["error"]["message"].as_str().expect("message");
        assert!(message.contains("不是 `.yeban` 容器"), "{message}");
        assert!(message.contains("end-of-central-directory"), "{message}");

        // **强对照**：同一份 JSON 字节原样包进真容器 ⇒ 能打开。
        let bytes = yeban_model::container::write_container(&[
            yeban_model::container::ContainerEntry::new(
                yeban_model::container::PROJECT_JSON_NAME,
                text.clone().into_bytes(),
            ),
            yeban_model::container::ContainerEntry::new(
                yeban_model::container::HISTORY_DAG_NAME,
                serde_json::to_vec(&CommitGraph::new()).expect("空图谱"),
            ),
        ])
        .expect("写真容器");
        write_project_atomic(&path, &bytes).expect("保存容器");
        let loaded = load_project(&path).expect("容器里的同一份工程必须能读");
        assert_eq!(loaded.project, model);
        assert_eq!(loaded.bytes as usize, bytes.len());
        fs::remove_dir_all(&dir).ok();
    }

    /// 空文件 / 随机字节 / 垃圾文本 / 坏 JSON：**各有**明确且精确的错误
    /// （不是泛化的"未知格式"，更不是"打开成空工程"）。
    #[test]
    fn empty_random_and_garbage_files_are_refused_precisely() {
        let dir = scratch("non-container");
        let model = yeban_model::samples::filled_project();
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty.yeban", Vec::new()),
            ("random.yeban", vec![0xAB; 64]),
            ("garbage.yeban", b"not a zip at all".to_vec()),
            ("corrupt.yeban", b"{not json".to_vec()),
        ];
        for (name, bytes) in cases {
            let path = dir.join(name);
            fs::write(&path, &bytes).expect("写非容器");
            let fault = load_project(&path).expect_err("非容器必须被拒绝");
            assert_eq!(fault.domain_code(), Some(ErrorCode::IoError), "{name}");
            let value = fault.into_result().expect("带内");
            assert_eq!(value["error"]["code"], "IO_ERROR", "{name}");
            assert_eq!(
                value["error"]["data"]["category"], "malformed-container",
                "{name}: {value}"
            );
            assert_eq!(
                value["error"]["data"]["specId"], "ARCH-SEC-003",
                "{name}: {value}"
            );
            let message = value["error"]["message"].as_str().expect("message");
            assert!(message.contains("不是 `.yeban` 容器"), "{name}: {message}");
        }
        // 反面对照：真容器仍然能读（拒绝不是"这个目录里的东西都读不了"）。
        let path = dir.join("good.yeban");
        let bytes = container_bytes(&path, &model, &CommitGraph::new(), &BTreeMap::new())
            .expect("容器字节");
        fs::write(&path, &bytes).expect("写容器");
        assert!(load_project(&path).is_ok());
        fs::remove_dir_all(&dir).ok();
    }

    /// 容器侧的**炸弹 / 上限闸门**在删掉兼容路径之后一个都没松：
    /// 真容器 + 可注入的紧上限 ⇒ 命中**容器层**（不是 I/O 层）的 `MUST-GATE-007`。
    #[test]
    fn container_bomb_gates_still_fire_after_the_compat_path_is_gone() {
        let dir = scratch("bomb");
        let path = dir.join("bomb.yeban");
        let model = yeban_model::samples::filled_project();
        let bytes = container_bytes(&path, &model, &CommitGraph::new(), &BTreeMap::new())
            .expect("容器字节");
        fs::write(&path, &bytes).expect("写容器");

        // `max_entry_bytes = 64`：`project.json` 远大于它 ⇒ 条目声明的解压体积越界。
        // 文件大小闸门刻意放宽（`max_total_bytes` 远大于容器），所以命中的只能是
        // **容器层**那一道 —— 载荷里没有 `fileBytes` 就是证据。
        let tight = ContainerLimits {
            max_entry_bytes: 64,
            max_total_bytes: 1 << 20,
            max_ratio: 1_000,
            max_entries: 8,
        };
        assert!(
            max_container_file_bytes(&tight) > u64::try_from(bytes.len()).unwrap(),
            "本判据必须让文件大小闸门放行, 才能证明容器层闸门真的在"
        );
        let fault = load_project_with_limits(&path, &tight).expect_err("容器层炸弹闸门必须生效");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["category"], "archive-bomb");
        assert_eq!(value["error"]["data"]["specId"], "MUST-GATE-007");
        assert!(
            value["error"]["data"]["fileBytes"].is_null(),
            "拒绝必须来自容器层的条目体积闸门, 不是 I/O 层的文件大小闸门: {value}"
        );
        assert!(
            value["error"]["data"]["containerError"]
                .as_str()
                .is_some_and(|detail| detail.contains("EntryTooLarge")),
            "诊断必须指名是条目解压体积: {value}"
        );

        // 同一份字节在默认上限下能读 ⇒ 上面拒绝的确实是**注入的紧上限**。
        assert!(load_project(&path).is_ok());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn directory_target_is_file_not_found() {
        let dir = scratch("isdir");
        let fault = load_project(&dir).expect_err("目录不是工程文件");
        assert_eq!(fault.domain_code(), Some(ErrorCode::FileNotFound));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn oversized_files_are_refused_before_they_are_read_into_memory() {
        // `MUST-GATE-007` 在 **I/O 层**的落点：`ContainerLimits` 全是"解压后体积"的闸门，
        // 它们只在字节已经进了内存之后才生效 —— 所以"先给一个巨大文件"曾是一个绕过窗口。
        // 判据用**可注入的紧上限**在几十 KB 的文件上触发它（不可能真造 8 GB）。
        let dir = scratch("oversized");
        let path = dir.join("huge.yeban");
        let tight = ContainerLimits {
            max_entry_bytes: 8,
            max_total_bytes: 8,
            max_ratio: 1,
            max_entries: 1,
        };
        let cap = max_container_file_bytes(&tight);
        assert!(
            cap < 64 * 1024,
            "紧上限必须小到能用小文件触发闸门, 实际 {cap}"
        );
        fs::write(&path, vec![b'x'; usize::try_from(cap).unwrap() + 1]).expect("写超限文件");

        let fault = load_project_with_limits(&path, &tight).expect_err("超限文件必须被拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        let value = fault.into_result().expect("领域失败是带内响应");
        assert_eq!(value["error"]["data"]["category"], "archive-bomb");
        assert_eq!(value["error"]["data"]["specId"], "MUST-GATE-007");
        assert_eq!(value["error"]["data"]["fileBytes"], cap + 1);

        // 同一份字节在**宽松**上限下不会被这道闸门拦：它会落到"不是 `.yeban` 容器"
        // 那一条路上，载荷里**没有** `fileBytes` ⇒ 证明上面的拒绝确实来自**文件大小**
        // 这一道，而不是"这个文件反正会失败"。
        let loose = load_project(&path).expect_err("不是合法工程");
        let loose_value = loose.into_result().expect("带内");
        assert_eq!(
            loose_value["error"]["data"]["category"], "malformed-container",
            "{loose_value}"
        );
        assert!(
            loose_value["error"]["data"]["fileBytes"].is_null(),
            "宽松上限下不该命中文件大小闸门: {loose_value}"
        );
        assert!(
            loose_value["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("不是 `.yeban` 容器")),
            "{loose_value}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// **第一道闸门是 fail-fast（不碰磁盘内容）**：声明大小超限时必须在 `metadata`
    /// 之后立刻拒绝，而不是先 `fs::read` 再拒绝。
    ///
    /// 既有判据只断言"被拒绝"的载荷，而"读了内容再拒"与"没读就拒"对同一个**普通**
    /// 文件产出**同一份**载荷（`declared == len`，`oversized_file_fault` 的两个入参
    /// 相同）⇒ 两道闸门在那里同解。本条用一个**不可读**的文件把两道分开：
    /// 权限位 `0000` 时 `metadata` 仍然成功（`stat` 只看目录的执行位），
    /// `fs::read` 会 `EACCES`。第一道在 `metadata` 上就拒绝（`archive-bomb` +
    /// `fileBytes`）；放宽它就会掉进 `fs::read` 的 `IO_ERROR`（载荷里是 `kind`/`osError`）。
    ///
    /// ⚠ 这是一条 `#[cfg(unix)]` 的判据，且前提是**不以 root 跑**（root 会绕过读权限位）。
    #[test]
    #[cfg(unix)]
    fn the_declared_size_gate_refuses_before_reading_the_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch("declared-fail-fast");
        let path = dir.join("unreadable.yeban");
        let tight = ContainerLimits {
            max_entry_bytes: 8,
            max_total_bytes: 8,
            max_ratio: 1,
            max_entries: 1,
        };
        let cap = max_container_file_bytes(&tight);
        fs::write(&path, vec![b'x'; usize::try_from(cap).unwrap() + 1]).expect("写超限文件");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");

        // 夹具前提: `metadata` 读得到大小, 但内容读不出来。
        assert_eq!(
            fs::metadata(&path).expect("stat").len(),
            cap + 1,
            "夹具前提: 声明大小必须可读"
        );
        assert!(
            fs::read(&path).is_err(),
            "夹具前提: 这个文件必须读不出来（否则本条区分不了两道闸门）"
        );

        let fault = load_project_with_limits(&path, &tight).expect_err("超限文件必须被拒绝");
        let value = fault.into_result().expect("领域失败是带内响应");
        assert_eq!(
            value["error"]["data"]["category"], "archive-bomb",
            "必须在读内容之前就按声明大小拒绝: {value}"
        );
        assert_eq!(value["error"]["data"]["fileBytes"], cap + 1);
        assert!(
            value["error"]["data"]["kind"].is_null(),
            "载荷不得来自 fs::read 的 I/O 错误（那说明第一道闸门被绕过了）: {value}"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).ok();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zip_signature_tiers_non_containers_from_broken_containers() {
        // `has_zip_signature` 只服务**诊断分档**（D43 之后不再服务兼容分支）：
        // 有 ZIP 结构的失败报容器裁决，没有的报"不是 `.yeban` 容器"。
        assert!(has_zip_signature(&CONTAINER_MAGIC));
        assert!(has_zip_signature(b"PK\x05\x06"));
        assert!(has_zip_signature(b"PK\x07\x08"));
        assert!(!has_zip_signature(&[]));
        assert!(!has_zip_signature(b"{\n  \"bpm\": 120.0\n}"));
        assert!(!has_zip_signature(b"not a zip at all"));
    }

    #[test]
    fn truncating_a_container_is_an_io_error_not_a_half_project() {
        let dir = scratch("truncated");
        let path = dir.join("demo.yeban");
        let model = yeban_model::samples::filled_project();
        let bytes = container_bytes(&path, &model, &CommitGraph::new(), &BTreeMap::new())
            .expect("容器字节");
        fs::write(&path, &bytes[..bytes.len() / 2]).expect("写半截容器");
        let fault = load_project(&path).expect_err("截断的容器必须被拒绝");
        assert_eq!(fault.domain_code(), Some(ErrorCode::IoError));
        let value = fault.into_result().expect("带内");
        // 截断的容器前 4 字节仍是 `PK\x03\x04` ⇒ 它落在"**你的 `.yeban` 坏了**"
        // 那一档（精确容器裁决），而不是"你给的不是 `.yeban` 容器"。
        assert_eq!(value["error"]["data"]["category"], "malformed-container");
        assert_eq!(value["error"]["data"]["specId"], "ARCH-SEC-003");
        assert!(
            value["error"]["data"]["containerError"].is_string(),
            "必须携带精确的容器裁决: {value}"
        );
        assert!(
            value["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("不是可接受的 .yeban 容器")),
            "截断的 `.yeban` 必须报容器裁决, 而不是「不是我们的文件」: {value}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn container_rejections_are_classified_without_inventing_error_codes() {
        // 每个分类都映射到**契约内**的 IO_ERROR, 分类本身进 data。
        let cases: Vec<(ContainerError, ContainerRejection)> = vec![
            (
                ContainerError::ParentDirSegment {
                    name: "../evil".to_owned(),
                },
                ContainerRejection::PathTraversal,
            ),
            (
                ContainerError::EntryTooLarge {
                    declared: 10,
                    max: 1,
                },
                ContainerRejection::ArchiveBomb,
            ),
            (
                ContainerError::UnsupportedCompression {
                    index: 0,
                    method: 8,
                },
                ContainerRejection::Unsupported,
            ),
            (
                ContainerError::CrcMismatch {
                    index: 0,
                    name: "assets/x".to_owned(),
                    declared: 1,
                    actual: 2,
                },
                ContainerRejection::Malformed,
            ),
            (
                ContainerError::MissingHistoryDag,
                ContainerRejection::Layout,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(container_rejection(&error), expected, "{error:?}");
            let fault = container_fault(Path::new("/tmp/demo.yeban"), &error);
            assert_eq!(fault.domain_code(), Some(ErrorCode::IoError), "{error:?}");
            let value = fault.into_result().expect("领域失败是带内响应");
            assert_eq!(value["error"]["code"], "IO_ERROR");
            assert_eq!(value["error"]["data"]["category"], expected.as_str());
            assert_eq!(value["error"]["data"]["specId"], expected.spec_id());
        }

        // "不是 `.yeban` 容器"这一档同样只能用一个**已存在的**契约码，
        // 并且必须携带容器的原裁决（`ADR-0001 D25` + D43）。
        let not_a_container =
            not_a_container_fault(Path::new("/tmp/bare.yeban"), &ContainerError::EocdNotFound);
        assert_eq!(
            not_a_container.domain_code(),
            Some(ErrorCode::IoError),
            "非容器拒绝不许发明新码"
        );
        let value = not_a_container.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "IO_ERROR");
        assert_eq!(value["error"]["data"]["category"], "malformed-container");
        assert_eq!(value["error"]["data"]["specId"], "ARCH-SEC-003");
        assert_eq!(value["error"]["data"]["containerError"], "EocdNotFound");
        assert_eq!(value["error"]["data"]["path"], "/tmp/bare.yeban");
        assert!(
            value["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("不是 `.yeban` 容器")),
            "{value}"
        );
    }
    /// 容器文件大小上界的**公式**按字面值钉住（含固定的 4 KiB 余量与逐条目元数据项）。
    ///
    /// 注入（实测红）：把 `.saturating_add(4_096)` 改成 `.saturating_add(0)` ⇒
    /// 既有判据 `the_declared_size_gate_refuses_before_reading_the_file` **全绿**
    /// —— 它拿 `max_container_file_bytes(&tight)` 的返回值当夹具输入（常量自比）；
    /// 本判据红。
    #[test]
    fn the_container_size_bound_is_the_published_formula() {
        // 0 个条目 ⇒ 只剩固定余量。
        assert_eq!(
            max_container_file_bytes(&ContainerLimits {
                max_entry_bytes: 0,
                max_total_bytes: 0,
                max_ratio: 1,
                max_entries: 0,
            }),
            4_096,
            "固定余量"
        );
        // 1 个条目 ⇒ 余量 + (名字上限 + 128) × 2（local header 一份 + central directory 一份）。
        // ⚠ 名字上限是**另一个 crate 的公开常量**（`yeban_model::container::MAX_ENTRY_NAME_BYTES`），
        // 因此这里也把它的**取值**按字面值钉住 —— 它变了本判据要红（公式跟着变）。
        assert_eq!(MAX_ENTRY_NAME_BYTES, 4_096);
        assert_eq!(
            max_container_file_bytes(&ContainerLimits {
                max_entry_bytes: 0,
                max_total_bytes: 0,
                max_ratio: 1,
                max_entries: 1,
            }),
            4_096 + (4_096 + 128) * 2
        );
        // 总量项按原样加上去。
        assert_eq!(
            max_container_file_bytes(&ContainerLimits {
                max_entry_bytes: 0,
                max_total_bytes: 1_024,
                max_ratio: 1,
                max_entries: 0,
            }),
            1_024 + 4_096
        );
    }
}
