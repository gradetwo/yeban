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
//! - 本模块**只**做"字节 → 同目录临时文件 → `sync_all` → `rename`"，不碰锁、不碰
//!   `history.dag` 的语义、不碰资产池；
//! - `history.dag` 以**空字节**写出（工程容器布局要求该条目存在；提交图谱的权威内容
//!   属 `yeban-model::commit`，本切片没有提交可写）；
//! - 一旦 `yeban-mcp` 的 store 落地，应当把本模块换成对它的调用（needs 已登记）。
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

use yeban_model::container::{ContainerError, write_project_container};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;

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
}

impl SaveError {
    /// 如果这次失败来自容器层，借出它的精确变体（压缩法 / 路径攻击 / CRC …）。
    #[must_use]
    pub const fn container(&self) -> Option<&ContainerError> {
        match self {
            Self::Container(error) => Some(error),
            Self::NoFileName { .. } | Self::Io { .. } => None,
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
        }
    }
}

impl core::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Container(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::NoFileName { .. } => None,
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
/// # Errors
///
/// 路径无法构造临时文件（[`SaveError::NoFileName`]）、容器写出被拒
/// （[`SaveError::Container`]）、或任一步 I/O 失败（[`SaveError::Io`]）。
pub fn save_project_file(
    project: &YebanProjectV1,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref().to_path_buf();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| SaveError::NoFileName { path: path.clone() })?;

    // 第 0 步：先把**字节**全部算出来（容器写出失败时一个文件都还没碰）。
    // `history.dag` 以空字节写出：容器布局要求该条目存在，而提交图谱的权威内容属
    // `yeban-model::commit`（本切片没有提交可写，见模块文档的边界）。
    let bytes = write_project_container(project, &[], &BTreeMap::new())?;

    // 第 1 步：同目录临时文件（`[ARCH-SEC-004]` 的 `.yeban.tmp-{ulid}` 形态）。
    // `EntityId::new()` 生成一个 ULID，取它的规范文本做尾段 —— 与规范的字面形态一致，
    // 而且**不需要**在 app 侧新引入 `ulid` 依赖（`yeban-model` 已把它封装成 `EntityId`）。
    let temp_name = format!(
        "{file_name}{TEMP_INFIX}{}",
        EntityId::new().to_canonical_string()
    );
    let temp_path = path.with_file_name(&temp_name);

    let write_result = write_temp(&temp_path, &bytes);
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
    }
}
