//! 保存一个 `.yeban` 工程 —— **原子落盘**（`[ARCH-SEC-004]`）的最小实现。
//!
//! ## 规范来源 (Normative)
//!
//! `[ARCH-SEC-004]`「原子落盘与崩溃安全保存策略 (Atomic Temp-File Replace)」原文三步：
//!
//! 1. 完整 ZIP 容器与资产首先写入**同目录临时文件**（`.yeban.tmp-{ulid}`）；
//! 2. 针对临时文件执行操作系统级物理刷盘（`File::sync_all()` / `fsync`）；
//! 3. 执行操作系统级**原子重命名替换**（Unix `rename` / Windows `MoveFileEx`），
//!    确保即便在写入瞬间断电，旧工程文件依然 100% 完整可用。
//!
//! `[ARCH-SEC-003]` 的容器字节由 `yeban_model::container::write_project_container` 产出
//! （**本模块不重写**容器的任何一条规则；红线 6 的 Zip-Slip / 炸弹防御在 `yeban-model` 里）。
//!
//! ## 为什么这一段住在 `yeban-app`（以及它的边界）
//!
//! ADR-0001 **D30** 明确写着："下一个能力切片：`yeban-mcp/src/domain/store.rs`
//! （`ARCH-SEC-004` 原子落盘）目前**不调用**本模块"。也就是说 **D30 指认的权威实现点在
//! `yeban-mcp`**，而那里现在还只有字节层。本切片需要"`ui/force_save` 真的写了一个文件"
//! 这条可判据的事实，因此在这里写一个**最小**实现，并把两者关系登记清楚：
//!
//! - 本模块**只**做"字节 → 同目录临时文件 → `sync_all` → `rename`"，不碰
//!   `history.dag` 的语义、不碰资产池；
//! - [`save_project_file`] 把 `history.dag` 以**空字节**写出（工程容器布局要求该条目存在；
//!   提交图谱的权威内容属 `yeban-model::commit`，`ui/force_save` 这条切片没有提交可写）；
//! - [`save_archive_file`] 则把调用方给的归档**原样保真**写回（`history.dag` + `assets/`），
//!   命令行的 `--save-as` 用它 ⇒ "打开再保存"不会静默丢掉资产池；
//! - [`write_file_atomically`] 是上面两条**唯一**的落盘实现，也是命令行
//!   `--export-elements` 的落盘实现 —— 原子替换只有一份代码；
//! - 一旦 `yeban-mcp` 的 store 落地，应当把本模块换成对它的调用（needs 已登记）。
//!
//! ## 锁：**两条工程保存入口都取 `.yeban.lock`**（`ROAD-M4-008` 选项 (a) 第三片）
//!
//! 上面那句"不碰锁"曾经是事实，也是问题 6 的缺口：控制面的 `yeban_save_project`
//! 在会话持有的 `.yeban.lock` 下写盘，而 app 的保存路径（`ui/force_save` / `--save-as`）
//! **一把锁都不取** ⇒ 两条路径会同时写同一个工程文件（选项 (c) 被拒的"影子写者"）。
//! 现在两条入口各取**排他写**建议锁，且用的是**与控制面同一份源码**
//! （[`crate::project_lock`]，`#[path]` 共享 `yeban-mcp/src/domain/lock.rs`）。
//! 拿不到锁 ⇒ [`SaveError::Locked`]，**一个字节都不写**（`MUST-GATE-008`）。
//!
//! [`write_file_atomically`] **仍然不取锁**：它写的是任意产物（元素清单 / `.mid` / `.als`），
//! 不是工程文档，锁对它没有语义（见该函数的文档）。
//!
//! ## 平台差异（**如实登记，不写没验证过的代码**）
//!
//! - 临时文件与 `rename` 是跨平台的（`std::fs`）。
//! - **目录**刷盘（`File::open(dir)?.sync_all()`）只在 Unix 分支执行：Windows 上打开目录
//!   需要 `FILE_FLAG_BACKUP_SEMANTICS`，而本仓库没有 `windows-sys` 依赖，**不猜**。
//!   本机（macOS）与 CI（Linux）两侧都会真的执行这段；Windows 手动档只跑
//!   `yeban-model` / `yeban-mcp`，不覆盖本文件。
//!
//! ## 本模块**零 Slint、零引擎**依赖
//!
//! 因此它在本机可以用 `rustc --edition 2024 --test` 真跑（见
//! `docs/ledger/app-mixer-notes.md` §5）。

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use yeban_model::container::{ContainerError, ProjectArchive, write_project_container};
use yeban_model::ids::{AssetHash, EntityId};
use yeban_model::project::YebanProjectV1;

use crate::project_lock;

/// 临时文件名里的固定中缀（`[ARCH-SEC-004]` 的 `.yeban.tmp-{ulid}` 形态）。
pub const TEMP_INFIX: &str = ".tmp-";

/// 一次成功保存的读数（`ui/force_save` 的结构化回执就用它）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// 最终落点（调用方给的那个路径）。
    pub path: PathBuf,
    /// 写出的容器字节数。
    pub bytes: usize,
    /// 用过的临时文件名（**保存成功后它不应再存在**；判据会去查）。
    pub temp_name: String,
}

/// 保存失败的原因（**每一种都如实上报**，绝不"写了个空文件也算成功"）。
#[derive(Debug)]
pub enum SaveError {
    /// 目标路径没有文件名（例如以 `/` 结尾）⇒ 无法构造同目录临时文件。
    NoFileName {
        /// 调用方给的路径。
        path: PathBuf,
    },
    /// 容器序列化被拒绝（`yeban-model` 的裁决，原样携带）。
    Container(ContainerError),
    /// I/O 失败（哪个路径、哪个动作、底层错误）。
    Io {
        /// 出问题的路径。
        path: PathBuf,
        /// 正在做什么（`"写临时文件"` / `"刷盘"` / `"原子重命名"` / `"刷盘目录"`）。
        action: &'static str,
        /// 底层错误。
        source: std::io::Error,
    },
    /// 目标工程文件的 `.yeban.lock` **排他写**建议锁被别的活着的持有者占用 ⇒
    /// **拒绝写入**（`[ARCH-SEC-001]` / `[MUST-GATE-008]`；`ROAD-M4-008` 选项 (a) 第三片）。
    ///
    /// 存在的理由：控制面的 `yeban_save_project` 与 app 的保存路径必须争**同一把**锁。
    /// 没有这一条时，`--save-as` / `ui/force_save` 会在别人持锁时直接覆盖工程文件 ——
    /// 那正是问题 6 选项 (c) 被拒的"影子写者"。
    ///
    /// 说明：诊断载荷装在一个 `Box` 里（[`LockHold`]）。这不是洁癖 ——
    /// `SaveError` 会进 `CliError`，而 `clippy::result_large_err`（`-D warnings` 下）对
    /// `Result<_, CliError>` 的尺寸有上限；把 `PathBuf`/`String` 摊在变体里会让整个
    /// crate 的 `Result` 都变大。
    Locked(Box<LockHold>),
    /// 本平台没有可用的 OS 建议锁 ⇒ **显式**失败，绝不静默放行并发写。
    LockUnsupported {
        /// 企图写入的工程路径。
        path: PathBuf,
        /// `std::env::consts::OS`。
        os: &'static str,
        /// 规范 ID（`ARCH-SEC-001` / `MUST-GATE-008`）。
        spec_id: &'static str,
    },
}

/// [`SaveError::Locked`] 的诊断载荷（谁占着、占用的是哪个锁文件）。
#[derive(Debug)]
pub struct LockHold {
    /// 企图写入的工程路径。
    pub path: PathBuf,
    /// 被占用的锁文件路径（`<工程>.lock`）。
    pub lock_file: PathBuf,
    /// 持有者文本（读不到时给出**原因**，而不是空串）。
    pub holder: String,
    /// 持有者元数据的可读性口径（`available` / `unavailable-on-this-platform` / `unavailable`）。
    pub holder_metadata: &'static str,
    /// 持有者的锁模式（元数据读不到时为 `None`）。
    pub holder_mode: Option<String>,
}

impl SaveError {
    /// 如果这次失败来自容器层，借出它的精确变体（压缩法 / 路径攻击 / CRC …）。
    #[must_use]
    pub const fn container(&self) -> Option<&ContainerError> {
        match self {
            Self::Container(error) => Some(error),
            Self::NoFileName { .. }
            | Self::Io { .. }
            | Self::Locked(_)
            | Self::LockUnsupported { .. } => None,
        }
    }
}

impl core::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoFileName { path } => {
                write!(
                    formatter,
                    "`{}` 没有文件名, 无法构造同目录临时文件",
                    path.display()
                )
            }
            Self::Container(error) => write!(formatter, "容器写出被拒绝: {error}"),
            Self::Io {
                path,
                action,
                source,
            } => write!(formatter, "{action} `{}` 失败: {source}", path.display()),
            Self::Locked(hold) => write!(
                formatter,
                "`{}` 被 `.yeban.lock` 建议锁占用 ⇒ 拒绝写入 (绝不绕过锁覆盖别人的工程): \
                 锁文件={} 持有者={} holderMetadata={} holderMode={}",
                hold.path.display(),
                hold.lock_file.display(),
                hold.holder,
                hold.holder_metadata,
                hold.holder_mode.as_deref().unwrap_or("<unknown>"),
            ),
            Self::LockUnsupported { path, os, spec_id } => write!(
                formatter,
                "平台 `{os}` 没有实现 OS 建议锁 ({spec_id}) ⇒ 拒绝写入 `{}` \
                 (绝不静默放任并发写)",
                path.display()
            ),
        }
    }
}

impl core::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Container(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::NoFileName { .. } | Self::Locked(_) | Self::LockUnsupported { .. } => None,
        }
    }
}

impl From<ContainerError> for SaveError {
    fn from(value: ContainerError) -> Self {
        Self::Container(value)
    }
}

/// 把工程写成一个 `.yeban` 容器并**原子替换**到 `path`。
///
/// 步骤严格按 `[ARCH-SEC-004]` 的三阶段：临时文件 → `sync_all` → `rename`。
/// 失败时临时文件会被尽力删除（`remove_file` 的错误被忽略 —— 它与"保存失败"这个主因
/// 相比是次要信息，而且上报它会盖住主因）。
///
/// `history.dag` 以**空字节**写出、资产池为空：这条入口只拿得到一个 `YebanProjectV1`。
/// 需要"把打开的东西原样存回去"（保真 `history.dag` 与资产池）的调用方用
/// [`save_archive_file`]。
///
/// # Errors
///
/// 路径无法构造临时文件（[`SaveError::NoFileName`]）、容器写出被拒
/// （[`SaveError::Container`]）、`.yeban.lock` 排他写建议锁拿不到（[`SaveError::Locked`] /
/// [`SaveError::LockUnsupported`]）、或任一步 I/O 失败（[`SaveError::Io`]）。
pub fn save_project_file(
    project: &YebanProjectV1,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref();
    // 第 0 步：先把**字节**全部算出来（容器写出失败时一个文件都还没碰）。
    // `history.dag` 以空字节写出：容器布局要求该条目存在，而提交图谱的权威内容属
    // `yeban-model::commit`（本切片没有提交可写，见模块文档的边界）。
    let bytes = write_project_container(project, &[], &BTreeMap::new())?;
    // 第 0.5 步：**取排他写建议锁并随本次写入持有**（`MUST-GATE-008`）。
    // 顺序是刻意的：字节先算完（容器被拒时一个文件都不碰，连锁文件都不建），
    // 锁在**任何**文件系统写入之前拿到 ⇒ 拿不到就一个字节都不写。
    let _lock = acquire_write_lock(path)?;
    write_file_atomically(&bytes, path)
}

/// 把一个**全保真归档**（工程 + `history.dag` + 资产池）写成一个 `.yeban` 容器并
/// **原子替换**到 `path`。
///
/// 为什么需要它：`--open <a> --save-as <b>` 这类"另存为"必须**无损** —— 否则一个带
/// 资产池的工程被打开再保存一次，`assets/{sha256}` 与 `history.dag` 会**静默消失**，
/// 而用户看到的只是"保存成功"。这正是本文件头等忌讳的失败模式（见 `open.rs` 的
/// "绝不退化成空工程"）。
///
/// 与 [`save_project_file`] 的关系：后者 = 本函数 + 一个"历史空 / 资产空"的归档。
/// 落盘手法（临时文件 → `sync_all` → `rename` → 刷目录）**只有一份实现**
/// （[`write_file_atomically`]），不存在两条会漂移的原子写入路径。
///
/// `history.dag` 与资产池的**内容**由调用方决定（本模块不解释它们，也不替它们做取舍）。
///
/// 已知代价（如实登记）：`write_project_container` 的签名要 `BTreeMap<AssetHash, Vec<u8>>`，
/// 而 `ProjectArchive::assets` 是 `Vec<(AssetHash, Vec<u8>)>`，因此这里要重建一个
/// `BTreeMap` —— 大资产池会多一次内存拷贝。要消掉它需要 `yeban-model` 暴露一个
/// `&[(AssetHash, &[u8])]` 形态的写出面（本条已登记为 notes 的 needs）。
///
/// # Errors
///
/// 同 [`save_project_file`]，外加资产字节与其 CAS 键不符
/// （`ContainerError::AssetHashMismatch`，原样上报）。
pub fn save_archive_file(
    archive: &ProjectArchive,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref();
    let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    for (hash, data) in &archive.assets {
        assets.insert(hash.clone(), data.clone());
    }
    let bytes = write_project_container(&archive.project, &archive.history_dag, &assets)?;
    // 与 [`save_project_file`] 同一条门：`--save-as` 因此与 `ui/force_save`、与
    // 控制面的 `yeban_save_project` 争**同一把** `.yeban.lock`。
    let _lock = acquire_write_lock(path)?;
    write_file_atomically(&bytes, path)
}

/// 取目标工程文件的**排他写**建议锁，随这次写入的生命周期持有
/// （`[ARCH-SEC-001]` / `[MUST-GATE-008]`；`ROAD-M4-008` 选项 (a) 第三片）。
///
/// ## 为什么是"取锁"而不是"看一眼锁文件在不在"
///
/// `lock.rs` 的模块文档已经把这条钉死：**"文件存在"绝不是"被占用"的证据**
/// （崩溃会留下无持有者的锁文件，内核在进程死亡时释放 `flock`）；
/// "能拿到建议锁"才是"没有活着的持有者"的证据。因此这里直接用
/// [`crate::project_lock::lock::acquire`] —— 与控制面会话**同一个函数体**。
///
/// ## 为什么拿不到就**拒绝写入**
///
/// 保存路径是 `.yeban` 工程的**磁盘写者**。别人（另一个 app 实例 / stdio `yeban-mcp` /
/// 本进程的只读控制面会话）持着同一把锁时，唯一不制造第二个写者的行为就是拒绝 ——
/// 这正是问题 6 选项 (c) 被拒绝时点名的"影子写者"。
///
/// ## 归还方式
///
/// `LockGuard` 是 RAII：成功路径在 `write_file_atomically` 返回后随函数退出释放；
/// 失败路径（容器被拒之前就已经算完字节，之后任何 I/O 失败）同样随作用域释放。
/// **排他**持有者 `Drop` 时删除锁文件（共享读者不删，见 `lock.rs`），
/// 因此一次成功的保存**不会**在工程旁留下 `.yeban.lock`。
fn acquire_write_lock(path: &Path) -> Result<project_lock::lock::LockGuard, SaveError> {
    use project_lock::lock::{LockError, LockMode, lock_path};

    // 先做与写入端**同一个**文件名判定：`/` 这类路径连"同目录临时文件"都构造不出来，
    // 若先试着建锁文件，就会把"路径非法"报成 `/project.lock` 的 I/O 失败
    // （`a_path_without_a_file_name_is_rejected` 会红）。⇒ 非法路径**一个文件都不碰**。
    file_name_of(path)?;
    project_lock::lock::acquire(path, LockMode::ExclusiveWrite).map_err(|error| match error {
        LockError::WouldBlock { snapshot } => SaveError::Locked(Box::new(LockHold {
            path: path.to_path_buf(),
            lock_file: lock_path(path),
            holder: snapshot.holder_text(),
            holder_metadata: snapshot.availability(),
            holder_mode: snapshot.metadata().map(|metadata| metadata.lock_mode),
        })),
        LockError::UnsupportedPlatform { os, spec_id } => SaveError::LockUnsupported {
            path: path.to_path_buf(),
            os,
            spec_id,
        },
        // I/O 失败仍然是 I/O 失败（拿不到锁文件与"写不进目录"是同一类事实）：
        // 归到 [`SaveError::Io`] 而不是新造一个语义，调用方的退出码也因此不变。
        LockError::Io(source) => SaveError::Io {
            path: lock_path(path),
            action: "获取排他写建议锁",
            source,
        },
    })
}

/// 目标路径的**文件名字段**（`NoFileName` 的**唯一**判定）。
///
/// 锁与写入共用它，顺序因此只有一种：**非法路径 ⇒ 一个文件都不碰**
/// （既不建锁文件，也不建临时文件）。
fn file_name_of(path: &Path) -> Result<String, SaveError> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| SaveError::NoFileName {
            path: path.to_path_buf(),
        })
}

/// 把一段已经算好的字节**原子**写到 `path`（`[ARCH-SEC-004]` 的第 1~3.5 步）。
///
/// 为什么把它抽成公开函数：工程保存与"导出元素清单"必须共用**同一份**原子写入实现。
/// 两条各写一遍的原子替换，迟早有一条会退化成"直接 create + write"——那时
/// **失败现场的旧文件已经被截断**，而调用方只会看到一句"保存失败"。
///
/// ## 它**不取** `.yeban.lock`（刻意的）
///
/// 锁的语义单位是"**一份工程文档**"。这条入口写的是任意字节的任意目标
/// （元素清单 / `.mid` / `.als`），对它取工程锁会在 `song.mid` 旁边凭空造一个
/// `song.mid.lock`，而且与任何工程的锁都不互斥 —— 那是假保护。
/// 工程保存的两条入口（[`save_project_file`] / [`save_archive_file`]）各自在外面
/// 取锁，因此"写工程"这件事**只有一个**取锁位置（不存在"某条保存路径忘了取"的形态）。
///
/// # Errors
///
/// 目标路径没有文件名（[`SaveError::NoFileName`]）或任一步 I/O 失败（[`SaveError::Io`]）。
pub fn write_file_atomically(
    bytes: &[u8],
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref().to_path_buf();
    let file_name = file_name_of(&path)?;

    // 第 1 步：同目录临时文件（`[ARCH-SEC-004]` 的 `.yeban.tmp-{ulid}` 形态）。
    // `EntityId::new()` 生成一个 ULID，取它的规范文本做尾段 —— 与规范的字面形态一致，
    // 而且**不需要**在 app 侧新引入 `ulid` 依赖（`yeban-model` 已把它封装成 `EntityId`）。
    let temp_name = format!(
        "{file_name}{TEMP_INFIX}{}",
        EntityId::new().to_canonical_string()
    );
    let temp_path = path.with_file_name(&temp_name);

    let write_result = write_temp(&temp_path, bytes);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    // 第 3 步：原子重命名替换（Unix `rename` 覆盖已存在目标；Windows 由 `std` 映射到
    // `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`）。
    if let Err(source) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(SaveError::Io {
            path: path.clone(),
            action: "原子重命名",
            source,
        });
    }

    // 第 3.5 步（Unix 追加）：把**目录项**也刷下去，否则"文件在了"这件事本身可能还在
    // 页缓存里。Windows 分支不猜（见模块文档）。
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let Err(source) = File::open(dir).and_then(|handle| handle.sync_all()) {
            return Err(SaveError::Io {
                path: dir.to_path_buf(),
                action: "刷盘目录",
                source,
            });
        }
    }

    Ok(SaveReport {
        path,
        bytes: bytes.len(),
        temp_name,
    })
}

/// 写临时文件 + `sync_all`（`[ARCH-SEC-004]` 的第 1、2 步）。
///
/// `create_new(true)` 而不是 `create(true)`：临时名里带 ULID，重名意味着"同一纳秒里
/// 撞了两次"，那时**报错**比覆盖一个别人正在写的文件安全。
fn write_temp(temp_path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let mut handle = File::options()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .map_err(|source| SaveError::Io {
            path: temp_path.to_path_buf(),
            action: "写临时文件",
            source,
        })?;
    handle.write_all(bytes).map_err(|source| SaveError::Io {
        path: temp_path.to_path_buf(),
        action: "写临时文件",
        source,
    })?;
    handle.sync_all().map_err(|source| SaveError::Io {
        path: temp_path.to_path_buf(),
        action: "刷盘",
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::demo_project;
    use crate::open::open_project_file;

    /// 一个一次性的临时目录（**不用外部 crate**：`tempfile` 不在本 crate 的依赖里）。
    ///
    /// 目录落在 `std::env::temp_dir()` 下，名字带进程 id 与一个 ULID；测试结束时不删
    /// （`Drop` 里删要考虑失败路径的清理，收益不如"让失败现场留着可查"）。
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-app-save-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 判据 1：写出的文件**真的能被容器读回来**，且内容就是那个工程。
    #[test]
    fn a_saved_project_reads_back_as_the_same_project() {
        let project = demo_project();
        let dir = scratch_dir("roundtrip");
        let path = dir.join("demo.yeban");

        let report = save_project_file(&project, &path).expect("保存");
        assert_eq!(report.path, path);
        assert!(report.bytes > 0, "容器不能是 0 字节");
        assert_eq!(
            std::fs::metadata(&path).expect("文件在").len(),
            report.bytes as u64,
            "落盘字节数必须等于报告里的数字"
        );
        let read_back = open_project_file(&path).expect("读回");
        assert_eq!(read_back, project, "读回的工程必须与写出的逐字段相同");
        // 临时文件不许留下。
        assert!(
            !dir.join(&report.temp_name).exists(),
            "临时文件 `{}` 必须已被重命名掉",
            report.temp_name
        );
    }

    /// 判据 2：同一工程两次写出的**字节完全相同**（`[ARCH-DET-001]` 的确定性要求
    /// 由 `write_project_container` 保证：`BTreeMap` 顺序 + stored 压缩）。
    #[test]
    fn two_saves_of_the_same_project_are_byte_identical() {
        let project = demo_project();
        let dir = scratch_dir("stable");
        let first = save_project_file(&project, dir.join("a.yeban")).expect("第一次");
        let second = save_project_file(&project, dir.join("b.yeban")).expect("第二次");
        // 路径不同 ⇒ 容器字节相同（容器里没有路径）。
        assert_eq!(first.bytes, second.bytes);
        let left = std::fs::read(dir.join("a.yeban")).expect("读 a");
        let right = std::fs::read(dir.join("b.yeban")).expect("读 b");
        assert_eq!(left, right, "同一工程两次写出的字节必须相同");
    }

    /// 判据 3：**替换**已有文件（而不是追加/失败），且不留临时文件。
    #[test]
    fn saving_replaces_an_existing_file_atomically() {
        let dir = scratch_dir("replace");
        let path = dir.join("over.yeban");
        std::fs::write(&path, b"stale bytes that must disappear").expect("先放一个旧文件");

        let report = save_project_file(&demo_project(), &path).expect("覆盖保存");
        let bytes = std::fs::read(&path).expect("读回");
        assert_eq!(bytes.len(), report.bytes);
        assert!(
            open_project_file(&path).is_ok(),
            "替换后的文件必须是可读的容器"
        );
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(TEMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "目录里残留了临时文件: {leftovers:?}");
    }

    /// 判据 4：目标目录不存在 ⇒ **明确的 I/O 错误**（不是静默成功），且不留下垃圾。
    #[test]
    fn a_missing_directory_is_an_explicit_io_error() {
        let dir = scratch_dir("missing");
        let path = dir.join("nope").join("x.yeban");
        let error = save_project_file(&demo_project(), &path).expect_err("目录不存在必须报错");
        assert!(matches!(error, SaveError::Io { .. }), "实际: {error:?}");
        assert!(!path.exists());
        assert!(!dir.join("nope").exists());
    }

    /// 判据 5：没有文件名的目标路径 ⇒ [`SaveError::NoFileName`]（不 panic、不写到别处）。
    #[test]
    fn a_path_without_a_file_name_is_rejected() {
        let error = save_project_file(&demo_project(), "/").expect_err("没有文件名必须报错");
        assert!(
            matches!(error, SaveError::NoFileName { .. }),
            "实际: {error:?}"
        );
        // 通用原子写入入口同一条契约（否则导出会绕过它）。
        let error = write_file_atomically(b"x", "/").expect_err("没有文件名必须报错");
        assert!(
            matches!(error, SaveError::NoFileName { .. }),
            "实际: {error:?}"
        );
    }

    /// 判据 6：**归档保真保存** —— `history.dag` 与资产池必须原样活过一轮
    /// "打开 → 另存"（这是 `--save-as` 的语义，见模块文档）。
    #[test]
    fn an_archive_save_preserves_history_and_the_asset_pool() {
        use yeban_model::container::read_project_container;
        use yeban_model::ids::AssetHash;

        let project = demo_project();
        let asset = b"yeban-save-archive-asset".to_vec();
        let hash = AssetHash::of_bytes(&asset);
        let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        assets.insert(hash, asset.clone());
        let source = ProjectArchive {
            project: project.clone(),
            history_dag: b"commit-graph-bytes".to_vec(),
            assets: vec![(AssetHash::of_bytes(&asset), asset.clone())],
        };

        let dir = scratch_dir("archive");
        let path = dir.join("kept.yeban");
        let report = save_archive_file(&source, &path).expect("保真保存");
        assert_eq!(
            report.bytes,
            std::fs::metadata(&path).expect("文件在").len() as usize
        );

        let bytes = std::fs::read(&path).expect("读回");
        let read_back =
            read_project_container(&bytes, &yeban_model::container::ContainerLimits::default())
                .expect("读回容器");
        assert_eq!(read_back.project, project);
        assert_eq!(
            read_back.history_dag, b"commit-graph-bytes",
            "history.dag 必须在 --save-as 之后仍然存在"
        );
        assert_eq!(read_back.assets.len(), 1, "资产池不得静默消失");
        assert_eq!(read_back.assets[0].1, asset);
        assert_eq!(read_back, source, "归档必须逐字段等价");
    }

    /// 判据 7：通用原子写入在**只读目录**下必须失败，且**旧文件一个字节都没变**
    /// （`[ARCH-SEC-004]` 的可观测后果 —— 这正是"直接 create+write"会红掉的那条）。
    ///
    /// 构造的关键：目标文件**本身可写**，只有**目录**不可写。于是"就地覆盖"会成功、
    /// 而"同目录临时文件 + rename"必须失败 —— 这条判据因此真的能区分两种实现，
    /// 而不是只证明"写不进去"。
    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_never_touches_the_existing_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch_dir("readonly");
        let path = dir.join("protected.yeban");
        save_project_file(&demo_project(), &path).expect("先放一个真容器");
        let before = std::fs::read(&path).expect("读原文");

        let mut permissions = std::fs::metadata(&dir).expect("目录元数据").permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&dir, permissions).expect("降权");

        // 权限对特权进程无效（root）：响亮地跳过，而不是把"没测到"记成"通过"。
        let probe = dir.join(".probe");
        let writable = std::fs::write(&probe, b"x").is_ok();
        let _ = std::fs::remove_file(&probe);

        if writable {
            eprintln!("[yeban-app/save] 只读目录仍可写 (特权进程?), 本条判据无从判定 —— 响亮跳过");
        } else {
            let error = save_project_file(&demo_project(), &path).expect_err("只读目录必须报错");
            assert!(matches!(error, SaveError::Io { .. }), "实际: {error:?}");
            assert_eq!(
                std::fs::read(&path).expect("旧文件仍在"),
                before,
                "失败的保存绝不能碰旧文件"
            );
            let leftovers: Vec<String> = std::fs::read_dir(&dir)
                .expect("列目录")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains(TEMP_INFIX))
                .collect();
            assert!(leftovers.is_empty(), "失败后残留临时文件: {leftovers:?}");
        }

        let mut permissions = std::fs::metadata(&dir).expect("目录元数据").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&dir, permissions).expect("还原权限");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据 7 的非 Unix 占位：Windows 的"只读目录"是 ACL 语义，本仓库没有可移植的
    /// 构造手法，**不猜**。这里响亮地说明"本平台没测"，而不是静默通过。
    #[cfg(not(unix))]
    #[test]
    fn a_read_only_directory_never_touches_the_existing_file() {
        eprintln!(
            "[yeban-app/save] 非 Unix 平台: 只读目录语义是 ACL, 本仓库不构造 —— 该判据只在 Unix 腿有效"
        );
    }

    /// 判据 8（`ROAD-M4-008` 选项 (a) 第三片）：**别的持有者持着 `.yeban.lock` 时，
    /// 工程保存必须拒绝写入**（`MUST-GATE-008`）。
    ///
    /// 这条判据的牙在"三步都断言"：
    /// 1. 持锁期间保存 ⇒ `SaveError::Locked`（点名目标与锁文件）；
    /// 2. 拒绝之后旧文件**逐字节未变**（不是"写坏了才知道"）；
    /// 3. 释放锁之后**同一份保存必须成功**，且排他持有者把自己的锁文件收走
    ///    —— 这证明第 1 步拒绝的原因就是那把锁，而不是别的偶发失败。
    #[test]
    fn a_held_project_lock_refuses_the_save_without_touching_the_file() {
        use crate::project_lock::lock::{LockMode, acquire, lock_path};

        let dir = scratch_dir("locked");
        let path = dir.join("locked.yeban");
        save_project_file(&demo_project(), &path).expect("先放一个真容器");
        let before = std::fs::read(&path).expect("读原文");

        // 与保存路径**同一个实现、同一把锁**（这正是 `#[path]` 共享源码的可观测后果）。
        let guard = acquire(&path, LockMode::ExclusiveWrite).expect("取排他写建议锁");
        let error = save_project_file(&demo_project(), &path).expect_err("持锁时保存必须被拒");
        match &error {
            SaveError::Locked(hold) => {
                assert_eq!(hold.path, path, "拒绝必须点名企图写入的工程");
                assert_eq!(
                    hold.lock_file,
                    lock_path(&path),
                    "拒绝必须点名被占用的锁文件"
                );
            }
            other => panic!("必须是 SaveError::Locked, 实际: {other:?}"),
        }
        assert_eq!(
            std::fs::read(&path).expect("旧文件仍在"),
            before,
            "被拒绝的保存绝不能碰旧文件"
        );
        assert!(
            save_project_file(&demo_project(), &path).is_err(),
            "锁还在时第二次也必须被拒（不是只有第一次）"
        );
        drop(guard);

        // 释放之后同一份保存必须成功 ⇒ 第 1 步拒绝的原因只能是那把锁。
        save_project_file(&demo_project(), &path).expect("锁释放后必须能保存");
        assert!(
            !lock_path(&path).exists(),
            "成功的排他保存必须把自己的锁文件收走（不留残留锁）: {}",
            lock_path(&path).display()
        );
    }
}
