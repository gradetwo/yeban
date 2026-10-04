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
//! - [`save_project_file`] 把 `history.dag` 以**空字节**写出（工程容器布局要求该条目存在；
//!   提交图谱的权威内容属 `yeban-model::commit`，`ui/force_save` 这条切片没有提交可写）；
//! - [`save_archive_file`] 则把调用方给的归档**原样保真**写回（`history.dag` + `assets/`），
//!   命令行的 `--save-as` 用它 ⇒ "打开再保存"不会静默丢掉资产池；
//! - [`write_file_atomically`] 是上面两条**唯一**的落盘实现，也是命令行
//!   `--export-elements` 的落盘实现 —— 原子替换只有一份代码；
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

use yeban_model::container::{ContainerError, ProjectArchive, write_project_container};
use yeban_model::ids::{AssetHash, EntityId};
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
/// `history.dag` 以**空字节**写出、资产池为空：这条入口只拿得到一个 `YebanProjectV1`。
/// 需要"把打开的东西原样存回去"（保真 `history.dag` 与资产池）的调用方用
/// [`save_archive_file`]。
///
/// # Errors
///
/// 路径无法构造临时文件（[`SaveError::NoFileName`]）、容器写出被拒
/// （[`SaveError::Container`]）、或任一步 I/O 失败（[`SaveError::Io`]）。
pub fn save_project_file(
    project: &YebanProjectV1,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    // 第 0 步：先把**字节**全部算出来（容器写出失败时一个文件都还没碰）。
    // `history.dag` 以空字节写出：容器布局要求该条目存在，而提交图谱的权威内容属
    // `yeban-model::commit`（本切片没有提交可写，见模块文档的边界）。
    let bytes = write_project_container(project, &[], &BTreeMap::new())?;
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
    let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    for (hash, data) in &archive.assets {
        assets.insert(hash.clone(), data.clone());
    }
    let bytes = write_project_container(&archive.project, &archive.history_dag, &assets)?;
    write_file_atomically(&bytes, path)
}

/// 把一段已经算好的字节**原子**写到 `path`（`[ARCH-SEC-004]` 的第 1~3.5 步）。
///
/// 为什么把它抽成公开函数：工程保存与"导出元素清单"必须共用**同一份**原子写入实现。
/// 两条各写一遍的原子替换，迟早有一条会退化成"直接 create + write"——那时
/// **失败现场的旧文件已经被截断**，而调用方只会看到一句"保存失败"。
///
/// # Errors
///
/// 目标路径没有文件名（[`SaveError::NoFileName`]）或任一步 I/O 失败（[`SaveError::Io`]）。
pub fn write_file_atomically(
    bytes: &[u8],
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref().to_path_buf();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| SaveError::NoFileName { path: path.clone() })?;

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
}
