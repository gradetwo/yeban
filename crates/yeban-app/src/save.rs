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

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use yeban_model::container::{ContainerError, ProjectArchive, write_project_container};
use yeban_model::ids::{AssetHash, EntityId};
use yeban_model::project::{ClipContent, YebanProjectV1};

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
    /// 持久化读数（T-FIX-324）：文件内容是否连同**目录项**一起落盘。
    pub durability: Durability,
}

/// 一次成功保存的持久化读数（T-FIX-324）。
///
/// `rename` 成功 ⇒ 目标文件**已经**换成了新字节 —— 这件事不会因为随后的目录刷盘
/// 失败而回退（磁盘上旧内容已经不存在）。因此“rename 成功、刷目录失败”**不是**
/// 保存失败（那意味着“什么都没写”），而是“已提交、持久化未确认”：调用方应当
/// 把它当成一次成功的保存并更新自己的落点状态，同时把警告如实转给用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Durability {
    /// 文件内容与目录项都已落盘（rename 后的刷目录成功；`Windows` 分支不刷目录，
    /// 也归这里 —— 与模块文档“目录刷盘只在 Unix 执行”一致）。
    Durable,
    /// 文件内容已落盘（`rename` 成功），但**目录项**持久化未能确认（Unix 刷目录失败）。
    ///
    /// 断电后文件可能回退到旧内容 —— 这是“持久化警告”，不是保存失败；调用方
    /// （界面 / `CLI`）必须把它与真正的“什么都没写”分开显示。
    Warning {
        /// 给用户看的一句话（带目录路径与底层错误，说清“文件已替换”这个事实）。
        message: String,
    },
}

impl Durability {
    /// 警告文案（[`Durability::Durable`] 恒为 [`Option::None`]）。
    ///
    /// 不是 `const fn`：`Some(message)` 要把 `&String` 解引用成 `&str`，而
    /// `String` 的 deref 转换在常量求值里不可用（E0015）。调用方全在运行时上下文
    /// （`run_batch` 的批处理行、`save_action` 的落点映射、`host` 的回执文案），
    /// 没有任何 `const` 初始化点依赖它。
    #[must_use]
    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::Durable => None,
            Self::Warning { message } => Some(message),
        }
    }
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
    /// ⭐ **目标已存在**（T-FIX-308）：`save_new_archive_file` 的"不覆盖"语义。
    ///
    /// 判定发生在**拿到 `.yeban.lock` 之后**（同一个临界区内），因此两个守规矩的
    /// 写者不会同时通过检查；`write_file_atomically` 的 `rename` 仍是覆盖式的，
    /// 但本变体保证"新建"这条路**从不**走到它。
    TargetExists {
        /// 已经存在的目标路径。
        path: PathBuf,
    },
    /// 本平台没有可用的 OS 建议锁 ⇒ **显式**失败，绝不静默放行并发写。
    LockUnsupported {
        /// 企图写入的工程路径。
        path: PathBuf,
        /// `std::env::consts::OS`。
        os: &'static str,
        /// 规范 ID（`ARCH-SEC-001` / `MUST-GATE-008`）。
        spec_id: &'static str,
    },
    /// ⭐ **工程含音频片段 ⇒ 裸工程入口拒绝写**（T-FIX-329 的 API 硬化）。
    ///
    /// [`save_project_file`] 的文档契约是“history.dag 空 + 资产池空”；含 `Audio`
    /// 片段的工程走这条入口必然被写成缺素材坏文件（片段引用的资产字节不在容器里）。
    /// 因此在写任何字节之前响亮拒绝：请改用 [`save_archive_file`]（保真归档入口）。
    RequiresArchive {
        /// 工程里 `ClipContent::Audio` 片段的条数。
        audio_clips: usize,
    },
    /// ⭐ **音频片段引用的资产缺条目**（T-FIX-329 的 API 硬化）：归档入口在写盘前
    /// 做跨层核对 —— `ClipContent::Audio.asset` 引用的哈希必须**同时**在
    /// `project.assets` 索引与本次写出的 CAS 字节池（`ProjectArchive::assets`）里；
    /// 缺任何一边 ⇒ 本变体，**一个字节都不写**。
    MissingAsset {
        /// 缺失的资产哈希（去重升序；装在 `Box` 里控制错误值尺寸，
        /// 理由同 [`SaveError::Locked`]）。
        missing: Box<BTreeSet<AssetHash>>,
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
            | Self::TargetExists { .. }
            | Self::Io { .. }
            | Self::Locked(_)
            | Self::LockUnsupported { .. }
            | Self::RequiresArchive { .. }
            | Self::MissingAsset { .. } => None,
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
            Self::TargetExists { path } => write!(
                formatter,
                "目标工程已存在 `{}` ⇒ 新建工程拒绝覆盖（要覆盖请走显式 overwrite 入口）",
                path.display()
            ),
            Self::LockUnsupported { path, os, spec_id } => write!(
                formatter,
                "平台 `{os}` 没有实现 OS 建议锁 ({spec_id}) ⇒ 拒绝写入 `{}` \
                 (绝不静默放任并发写)",
                path.display()
            ),
            Self::RequiresArchive { audio_clips } => write!(
                formatter,
                "工程含 {audio_clips} 个音频片段, 裸工程入口只写空资产池 ⇒ 拒绝写出 \
                 (T-FIX-329: 会写成缺素材坏文件) —— 请改用保真归档入口 save_archive_file"
            ),
            Self::MissingAsset { missing } => write!(
                formatter,
                "归档缺 {} 项音频资产 (clip_pool 引用的哈希在索引/字节池缺条目) ⇒ 拒绝写盘: {:?}",
                missing.len(),
                missing
            ),
        }
    }
}

impl core::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Container(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::NoFileName { .. }
            | Self::TargetExists { .. }
            | Self::Locked(_)
            | Self::LockUnsupported { .. }
            | Self::RequiresArchive { .. }
            | Self::MissingAsset { .. } => None,
        }
    }
}

impl From<ContainerError> for SaveError {
    fn from(value: ContainerError) -> Self {
        Self::Container(value)
    }
}

/// 数一数工程里 `ClipContent::Audio` 片段的条数（T-FIX-329 的判定读数）。
fn audio_clip_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .filter(|entry| matches!(entry.content, ClipContent::Audio { .. }))
        .count()
}

/// 归档入口的跨层核对（T-FIX-329）：Audio 片段引用的哈希必须同时在
/// `project.assets` 索引与本次写出的 CAS 字节池里。返回缺条目的哈希集合
/// （去重升序 —— 错误文案因此也是确定性的，`[ARCH-DET-001]`）。
fn missing_audio_assets(archive: &ProjectArchive) -> BTreeSet<AssetHash> {
    let cas: BTreeSet<&AssetHash> = archive.assets.iter().map(|(hash, _)| hash).collect();
    let mut missing = BTreeSet::new();
    for entry in archive.project.clip_pool.values() {
        if let ClipContent::Audio { asset, .. } = &entry.content {
            let in_index = archive.project.assets.contains_key(asset);
            let in_cas = cas.contains(asset);
            if !in_index || !in_cas {
                missing.insert(asset.clone());
            }
        }
    }
    missing
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
/// ⭐ T-FIX-329：这条契约从文档约定升级成了 **API 检查** —— 工程含 `Audio` 片段时
/// 本入口直接报 [`SaveError::RequiresArchive`]，不再靠"调用方自己知道别用这条"续命。
///
/// # Errors
///
/// 路径无法构造临时文件（[`SaveError::NoFileName`]）、**工程含音频片段**
/// （[`SaveError::RequiresArchive`]）、容器写出被拒（[`SaveError::Container`]）、
/// `.yeban.lock` 排他写建议锁拿不到（[`SaveError::Locked`] /
/// [`SaveError::LockUnsupported`]）、或任一步 I/O 失败（[`SaveError::Io`]）。
pub fn save_project_file(
    project: &YebanProjectV1,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref();
    // ⭐ T-FIX-329（API 硬化）：本入口的文档契约是"history.dag 空 + 资产池空"。
    // 含 Audio 片段的工程走到这里必然被写成缺素材坏文件 —— 在写任何字节
    // （包括容器字节本身）之前响亮拒绝。检查只数 clip_pool，O(n) 且零副作用。
    let audio_clips = audio_clip_count(project);
    if audio_clips > 0 {
        return Err(SaveError::RequiresArchive { audio_clips });
    }
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
/// 资产池**不再**重建 `BTreeMap`（T-FIX-460）：`yeban-model` 已有借用形态的写出面
/// （`write_project_container_with_proposals_borrowed`），本函数经
/// [`save_archive_parts_file_with`] 直接借用输入切片；重复哈希在借用层按
/// `BTreeMap` 的语义去重（升序、最后一次出现的值胜出），不复制资产字节。
/// （旧"已知代价"登记已随本次改动删除；台账 `docs/ledger/app-cli-notes.md` §8
/// needs-2 的"加一个借用形态写出面"一项现已满足，待集成者划掉。）
///
/// # Errors
///
/// 同 [`save_project_file`]，外加资产字节与其 CAS 键不符
/// （`ContainerError::AssetHashMismatch`，原样上报）。
pub fn save_archive_file(
    archive: &ProjectArchive,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    save_archive_file_with(archive, path.as_ref(), true)
}

/// 与 [`save_archive_file`] 同一条原子写，但**工程只借用**（T-FIX-457）。
///
/// 调用方手里只有共享句柄（`Rc<RefCell<YebanProjectV1>>`）时用它：序列化只读
/// 借用里的工程，⛔ 不再为"凑一个 `ProjectArchive`"整工程深拷贝一次。
/// 写出的容器字节与 [`save_archive_file`] 逐字节相同（同一条
/// `write_project_container_with_proposals_borrowed` 调用，同一把
/// `.yeban.lock`，同一条 `write_file_atomically`）。
///
/// `load_notes` 不在参数里：它是装载留痕，不进容器（见
/// `yeban_model::container::ProjectArchive::load_notes` 的文档）。
///
/// # Errors
///
/// 同 [`save_archive_file`]。
pub(crate) fn save_archive_parts_file(
    project: &YebanProjectV1,
    history_dag: &[u8],
    assets: &[(AssetHash, Vec<u8>)],
    proposals: Option<&[u8]>,
    embedded_presets: &[(String, Vec<u8>)],
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    save_archive_parts_file_with(
        project,
        history_dag,
        assets,
        proposals,
        embedded_presets,
        path.as_ref(),
        true,
    )
}

/// ⭐ **新建专用**（T-FIX-308）：与 [`save_archive_file`] 同一条原子写，但
/// **不覆盖已存在的目标** —— 拿到 `.yeban.lock` 之后先查存在性，已存在 ⇒
/// [`SaveError::TargetExists`]，**一个字节都不写**。
///
/// 为什么检查必须在这里（而不是调用方）：存在性判定与写入之间若不是同一个
/// 临界区，两个守规矩的窗口都能通过 `path.exists()` 然后先后 `rename` 覆盖。
/// 本函数把判定放在 `acquire_write_lock` 之后（同一把锁内）⇒ 合作写者之间
/// 不存在该窗口。⚠ 不合作的外部写者仍可与 `rename` 竞争（OS 级覆盖是原子
/// 替换）—— 那是"独占创建"要另做跨平台系统调用才能关掉，本单不扩围。
///
/// # Errors
///
/// 同 [`save_archive_file`]，外加目标已存在 ⇒ [`SaveError::TargetExists`]。
pub fn save_new_archive_file(
    archive: &ProjectArchive,
    path: impl AsRef<Path>,
) -> Result<SaveReport, SaveError> {
    save_archive_file_with(archive, path.as_ref(), false)
}

/// [`save_archive_file`] 与 [`save_new_archive_file`] 的**唯一**实现体
/// （归档形态；借用形态见 [`save_archive_parts_file_with`]）。
fn save_archive_file_with(
    archive: &ProjectArchive,
    path: &Path,
    overwrite: bool,
) -> Result<SaveReport, SaveError> {
    // ⭐ T-FIX-329（API 硬化）：写盘前的跨层核对 —— Audio 片段引用的资产哈希
    // 必须**同时**在 project.assets 索引与本次写出的 CAS 字节池里；缺一边 ⇒
    // [`SaveError::MissingAsset`]，**一个字节都不写**。检查放在字节计算之前
    // （与"容器被拒时一个文件都不碰"同一哲学，且更早：连容器字节都不算）。
    let missing = missing_audio_assets(archive);
    if !missing.is_empty() {
        return Err(SaveError::MissingAsset {
            missing: Box::new(missing),
        });
    }
    save_archive_parts_file_with(
        &archive.project,
        &archive.history_dag,
        &archive.assets,
        archive.proposals.as_deref(),
        &archive.embedded_presets,
        path,
        overwrite,
    )
}

/// 把 `&[(AssetHash, Vec<u8>)]` 转成写出端要的 `&[(&AssetHash, &[u8])]`（T-FIX-460）。
///
/// 只搬运引用，⛔ 不复制资产字节。语义与旧实现（重建成 `BTreeMap`）逐条对齐：
/// - 按哈希升序；
/// - 重复哈希只留**最后一次**出现的值（`BTreeMap::insert` 的覆盖语义）；
/// - 输入顺序不影响输出（写出端还会再排一次，这里排好是让借用层本身就有确定的顺序）。
///
/// [`ProjectArchive::assets`] 的文档只要求"按哈希升序"，没有在类型上禁掉重复哈希；
/// 旧实现对重复输入是"静默取最后一次"，本函数保持同一行为 —— 绝不在这里把
/// "本可以保存的输入"改成报错（写出端对真正的重名条目仍有重名拒绝兜底，但去重
/// 之后它到不了那里）。
fn borrowed_assets_deduped(assets: &[(AssetHash, Vec<u8>)]) -> Vec<(&AssetHash, &[u8])> {
    let mut seen: BTreeSet<&AssetHash> = BTreeSet::new();
    let mut borrowed: Vec<(&AssetHash, &[u8])> = Vec::with_capacity(assets.len());
    // 倒序遍历 ⇒ 每个哈希第一次被 `seen` 接受的，就是它在输入里的最后一次出现。
    for (hash, data) in assets.iter().rev() {
        if seen.insert(hash) {
            borrowed.push((hash, data.as_slice()));
        }
    }
    borrowed.sort_by(|left, right| left.0.cmp(right.0));
    borrowed
}

/// [`save_archive_file_with`] 的借用形态：**唯一**实现体，归档形态只是转发。
///
/// 资产池**直接借用**输入切片（T-FIX-460）：[`borrowed_assets_deduped`] 只搬运引用，
/// ⛔ 不复制资产字节；重复哈希按 `BTreeMap` 的语义去重（升序、最后一次出现的值胜出），
/// 因此同一输入的写出字节与旧实现（重建 `BTreeMap`）逐字节相同。工程本身只借用，⛔ 不克隆。
fn save_archive_parts_file_with(
    project: &YebanProjectV1,
    history_dag: &[u8],
    assets: &[(AssetHash, Vec<u8>)],
    proposals: Option<&[u8]>,
    embedded_presets: &[(String, Vec<u8>)],
    path: &Path,
    overwrite: bool,
) -> Result<SaveReport, SaveError> {
    let borrowed_assets = borrowed_assets_deduped(assets);
    let borrowed_presets: Vec<(&str, &[u8])> = embedded_presets
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    // 提案段（`proposals.json`，T-FIX-43）随归档原样带出：本层与
    // `history_dag` 一样只搬运字节、不解读 —— 打开时读到的段若在这里被
    // 丢掉，界面保存一次就会把 MCP 的未合并提案悄悄抹除。
    let bytes = yeban_model::container::write_project_container_with_proposals_borrowed(
        project,
        history_dag,
        &borrowed_assets,
        &borrowed_presets,
        proposals,
    )?;
    // 与 [`save_project_file`] 同一条门：`--save-as` 因此与 `ui/force_save`、与
    // 控制面的 `yeban_save_project` 争**同一把** `.yeban.lock`。
    let _lock = acquire_write_lock(path)?;
    if !overwrite && path.exists() {
        return Err(SaveError::TargetExists {
            path: path.to_path_buf(),
        });
    }
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
    //
    // ⭐ T-FIX-324：这一步失败**不再**报 `SaveError::Io { action: "刷盘目录" }`。
    // `rename` 已经成功 ⇒ 目标文件已经是新字节，旧文件已经不存在 —— 这不是
    // "保存失败"（那意味着"什么都没写"，调用方会回滚界面 / 保留旧落点，与磁盘分叉），
    // 而是"已提交但目录项未确认持久化"：照常返回 [`Ok`]，把警告装进
    // [`SaveReport::durability`]，由调用方（界面 / CLI）如实显示。
    #[cfg(unix)]
    let mut durability = Durability::Durable;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let Err(source) = sync_directory(dir) {
            durability = Durability::Warning {
                message: format!(
                    "文件已替换，但目录项未确认持久化（刷目录 `{}` 失败: {source}）",
                    dir.display()
                ),
            };
        }
    }

    Ok(SaveReport {
        path,
        bytes: bytes.len(),
        temp_name,
        #[cfg(unix)]
        durability,
        #[cfg(not(unix))]
        durability: Durability::Durable,
    })
}

/// 第 3.5 步的本体（Unix）：打开目录并 `sync_all`。
///
/// 抽成函数只为判据可注入（T-FIX-324：rename 成功 + 刷目录失败的用例）；
/// 生产路径行为不变。
#[cfg(unix)]
fn sync_directory(dir: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    {
        if DIR_SYNC_INJECTION.with(std::cell::Cell::get) {
            return Err(std::io::Error::other("注入的目录刷盘失败（判据）"));
        }
    }
    File::open(dir).and_then(|handle| handle.sync_all())
}

// 判据注入缝（仅测试构建）：置 `true` 后 [`sync_directory`] 直接返回错误。
// 线程局部 ⇒ 并行测试互不干扰。
#[cfg(all(test, unix))]
thread_local! {
    pub(crate) static DIR_SYNC_INJECTION: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
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
    use yeban_model::samples::demo_project_assets;

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

    /// 演示工程的**无音频片段**变体（判据夹具）。
    ///
    /// `demo_project()` 自带一个引用真实 kick 字节的 `Audio` 片段（任务单 `#142`）；
    /// 裸工程入口按 T-FIX-329 必须**拒绝**它（写出来就是缺素材坏文件）。本文件
    /// 判据 1~7 要验证的落盘语义与音频片段无关 ⇒ 剥掉 `Audio` 片段再走保存，
    /// ⛔ 不改 `demo_project()` 本身（`samples` 的字节契约另有判据钉住）。
    fn demo_project_without_audio_clips() -> YebanProjectV1 {
        let mut project = demo_project();
        project
            .clip_pool
            .retain(|_, entry| !matches!(entry.content, ClipContent::Audio { .. }));
        project
    }

    /// 判据 1：写出的文件**真的能被容器读回来**，且内容就是那个工程。
    #[test]
    fn a_saved_project_reads_back_as_the_same_project() {
        let project = demo_project_without_audio_clips();
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
        let project = demo_project_without_audio_clips();
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

        let report =
            save_project_file(&demo_project_without_audio_clips(), &path).expect("覆盖保存");
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
        let error = save_project_file(&demo_project_without_audio_clips(), &path)
            .expect_err("目录不存在必须报错");
        assert!(matches!(error, SaveError::Io { .. }), "实际: {error:?}");
        assert!(!path.exists());
        assert!(!dir.join("nope").exists());
    }

    /// 判据 5：没有文件名的目标路径 ⇒ [`SaveError::NoFileName`]（不 panic、不写到别处）。
    #[test]
    fn a_path_without_a_file_name_is_rejected() {
        let error = save_project_file(&demo_project_without_audio_clips(), "/")
            .expect_err("没有文件名必须报错");
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

    /// 判据 6：**归档保真保存** —— `history.dag`、资产池与提案段
    /// （`proposals.json`，T-FIX-43）必须原样活过一轮
    /// "打开 → 另存"（这是 `--save-as` 的语义，见模块文档）。
    #[test]
    fn an_archive_save_preserves_history_and_the_asset_pool() {
        use yeban_model::container::read_project_container;
        use yeban_model::ids::AssetHash;

        // 判据 6 的工程同样剥掉 Audio 片段：本判据要证明的是 history/资产池保真，
        // 资产池里只有判据自己放进去的那一项（与 clip 引用无关）。
        let project = demo_project_without_audio_clips();
        let asset = b"yeban-save-archive-asset".to_vec();
        let hash = AssetHash::of_bytes(&asset);
        let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        assets.insert(hash, asset.clone());
        let source = ProjectArchive {
            project: project.clone(),
            history_dag: b"commit-graph-bytes".to_vec(),
            proposals: Some(b"unmerged-proposals-bytes".to_vec()),
            assets: vec![(AssetHash::of_bytes(&asset), asset.clone())],
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
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
        assert_eq!(
            read_back.proposals.as_deref(),
            Some(b"unmerged-proposals-bytes".as_slice()),
            "proposals.json 必须在 --save-as 之后仍然存在（界面保存不得抹除未合并提案）"
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
        save_project_file(&demo_project_without_audio_clips(), &path).expect("先放一个真容器");
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
            let error = save_project_file(&demo_project_without_audio_clips(), &path)
                .expect_err("只读目录必须报错");
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
        save_project_file(&demo_project_without_audio_clips(), &path).expect("先放一个真容器");
        let before = std::fs::read(&path).expect("读原文");

        // 与保存路径**同一个实现、同一把锁**（这正是 `#[path]` 共享源码的可观测后果）。
        let guard = acquire(&path, LockMode::ExclusiveWrite).expect("取排他写建议锁");
        let error = save_project_file(&demo_project_without_audio_clips(), &path)
            .expect_err("持锁时保存必须被拒");
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
            save_project_file(&demo_project_without_audio_clips(), &path).is_err(),
            "锁还在时第二次也必须被拒（不是只有第一次）"
        );
        drop(guard);

        // 释放之后同一份保存必须成功 ⇒ 第 1 步拒绝的原因只能是那把锁。
        save_project_file(&demo_project_without_audio_clips(), &path).expect("锁释放后必须能保存");
        assert!(
            !lock_path(&path).exists(),
            "成功的排他保存必须把自己的锁文件收走（不留残留锁）: {}",
            lock_path(&path).display()
        );
    }

    /// 判据 9（T-FIX-460）：**字节同一性** —— 同一份归档输入，新借用路径写出的容器
    /// 字节与旧实现（重建成 `BTreeMap` 再借出）**逐字节相同**。
    ///
    /// 对照直接用 `yeban-model` 仍保留的 `write_project_container`（`BTreeMap`
    /// 形态）算出：输入故意**按哈希降序**排列，证明顺序不影响输出；提案段与预制
    /// 都为空，两条路径调的是同一条写出实现，只差资产形态。
    #[test]
    fn borrowed_save_writes_byte_identical_containers_to_the_btreemap_form() {
        use yeban_model::container::write_project_container;
        use yeban_model::ids::AssetHash;

        let project = demo_project();
        let history_dag = b"commit-graph-bytes";
        let first = b"first-asset-bytes".to_vec();
        let second = b"second-asset-bytes".to_vec();
        let third = b"third-asset-bytes".to_vec();
        let hash_first = AssetHash::of_bytes(&first);
        let hash_second = AssetHash::of_bytes(&second);
        let hash_third = AssetHash::of_bytes(&third);
        assert!(
            hash_first != hash_second && hash_second != hash_third && hash_first != hash_third,
            "三个资产的哈希必须互不相同，否则降序排列无意义"
        );
        // 故意按哈希降序排列（与写出顺序相反），证明输入顺序不影响输出字节。
        let mut assets = vec![
            (hash_first.clone(), first.clone()),
            (hash_second.clone(), second.clone()),
            (hash_third.clone(), third.clone()),
        ];
        assets.sort_by(|left, right| right.0.cmp(&left.0));

        let mut map: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        for (hash, data) in &assets {
            map.insert(hash.clone(), data.clone());
        }
        let old_bytes =
            write_project_container(&project, history_dag, &map).expect("旧 BTreeMap 形态写出");

        let dir = scratch_dir("borrowed-identical");
        let path = dir.join("borrowed.yeban");
        let report =
            save_archive_parts_file_with(&project, history_dag, &assets, None, &[], &path, true)
                .expect("借用路径保存");
        let new_bytes = std::fs::read(&path).expect("读回");
        assert_eq!(report.bytes, new_bytes.len());
        assert_eq!(
            new_bytes, old_bytes,
            "借用路径与 BTreeMap 形态的写出字节必须逐字节相同"
        );
    }

    /// 判据 10（T-FIX-460）：**重复哈希** —— 输入里同一哈希出现两次时，借用层按
    /// `BTreeMap` 的语义只留**最后一次**出现的值，保存成功且读回只有一条资产。
    ///
    /// 归档类型只在文档里要求"按哈希升序"，没有在类型上禁掉重复；旧实现对此是
    /// "静默取最后一次"，本判据把这条行为钉住（而不是改成报错）。红对照：
    /// 去掉去重步骤后，写出端因重名条目拒绝写入，本判据红掉（见回执 §3 的红对照记录）。
    #[test]
    fn duplicate_asset_hash_keeps_the_last_value() {
        use yeban_model::container::read_project_container;
        use yeban_model::ids::AssetHash;

        let project = demo_project_without_audio_clips();
        let kept = b"duplicate-hash-last-value".to_vec();
        let dropped = b"duplicate-hash-first-value".to_vec();
        let hash = AssetHash::of_bytes(&kept);
        // 两条数据的哈希键相同（都是 `kept` 的哈希），但字节不同 —— 只有最后一次
        // 出现的值会进容器，写出端的哈希校验才通过。
        let archive = ProjectArchive {
            project: project.clone(),
            history_dag: b"commit-graph-bytes".to_vec(),
            proposals: None,
            assets: vec![(hash.clone(), dropped), (hash.clone(), kept.clone())],
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        };

        let dir = scratch_dir("duplicate-hash");
        let path = dir.join("dup.yeban");
        save_archive_file(&archive, &path).expect("重复哈希的保存必须成功（取最后一次）");

        let bytes = std::fs::read(&path).expect("读回");
        let read_back =
            read_project_container(&bytes, &yeban_model::container::ContainerLimits::default())
                .expect("读回容器");
        assert_eq!(read_back.assets.len(), 1, "重复哈希必须去重成一条资产");
        assert_eq!(read_back.assets[0].0, hash);
        assert_eq!(read_back.assets[0].1, kept, "留下的必须是最后一次出现的值");
    }

    /// 判据 11（T-FIX-460）：借用层去重函数本身 —— 乱序输入 + 重复哈希 ⇒
    /// 输出按哈希升序、每哈希一条、重复取最后一次；且全程只借用（返回值借自输入）。
    #[test]
    fn borrowed_assets_deduped_sorts_and_keeps_the_last_value() {
        use yeban_model::ids::AssetHash;

        let first = b"asset-first".to_vec();
        let second = b"asset-second".to_vec();
        let third = b"asset-third".to_vec();
        let hash_first = AssetHash::of_bytes(&first);
        let hash_second = AssetHash::of_bytes(&second);
        let hash_third = AssetHash::of_bytes(&third);
        // 同一哈希出现两次，第二次的值不同但键相同 ⇒ 只留第二次。
        let first_again = b"asset-first-second-occurrence".to_vec();
        let input = vec![
            (hash_third.clone(), third.clone()),
            (hash_first.clone(), first.clone()),
            (hash_second.clone(), second.clone()),
            (hash_first.clone(), first_again.clone()),
        ];
        let out = borrowed_assets_deduped(&input);
        assert_eq!(out.len(), 3, "四个输入、三个不同哈希 ⇒ 三条输出");
        let hashes: Vec<&AssetHash> = out.iter().map(|(hash, _)| *hash).collect();
        let mut sorted = hashes.clone();
        sorted.sort();
        assert_eq!(hashes, sorted, "输出必须按哈希升序");
        let kept = out
            .iter()
            .find(|(hash, _)| **hash == hash_first)
            .expect("hash_first 在输出里");
        assert_eq!(kept.1, first_again.as_slice(), "重复哈希留最后一次出现的值");
    }

    /// 判据 8（T-FIX-324）：`rename` 成功、**刷目录失败** ⇒ 返回 `Ok` + 持久化警告，
    /// 目标文件字节已经是新内容；**不是** `SaveError`（"保存失败"意味着什么都没写，
    /// 而此刻磁盘上已经是新文件 —— 归成失败会让调用方把落点状态留在旧值）。
    #[cfg(unix)]
    #[test]
    fn a_failed_directory_sync_is_committed_with_a_warning() {
        let dir = scratch_dir("dirsync");
        let path = dir.join("warned.yeban");
        std::fs::write(&path, b"old bytes that must be replaced").expect("先放旧内容");

        DIR_SYNC_INJECTION.with(|cell| cell.set(true));
        let result = save_project_file(&demo_project_without_audio_clips(), &path);
        DIR_SYNC_INJECTION.with(|cell| cell.set(false));

        let report = result.expect("rename 已成功 ⇒ 保存已提交，不是失败");
        let Durability::Warning { message } = &report.durability else {
            panic!("必须是 Durability::Warning, 实际: {:?}", report.durability);
        };
        assert!(
            message.contains("目录项未确认持久化"),
            "警告必须说清'文件已替换、目录项未确认': {message}"
        );

        // 目标文件字节已经更新（旧内容不在了，且是可读的合法容器）。
        let bytes = std::fs::read(&path).expect("读回");
        assert_ne!(
            bytes,
            b"old bytes that must be replaced".as_slice(),
            "目录刷盘失败不得回滚已替换的文件内容"
        );
        assert!(
            open_project_file(&path).is_ok(),
            "替换后的文件必须是可读的容器"
        );
        // rename 已发生 ⇒ 临时文件不该留下。
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(TEMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "目录里残留了临时文件: {leftovers:?}");
    }

    /// 判据 8 的反面（T-FIX-324）：注入关掉时同一目标保存必须仍是 `Durable`。
    #[cfg(unix)]
    #[test]
    fn a_normal_save_reports_durable() {
        let dir = scratch_dir("durable");
        let path = dir.join("quiet.yeban");
        let report = save_project_file(&demo_project_without_audio_clips(), &path).expect("保存");
        assert_eq!(report.durability, Durability::Durable);
        assert!(report.durability.warning().is_none());
    }

    /// 判据 9（T-FIX-329）：含 `Audio` 片段的工程走**裸工程入口**必须被拒绝
    /// （[`SaveError::RequiresArchive`]），且目标文件一个字节都不变。
    /// 夹具就是 `demo_project()` 原件 —— 它自带一个引用真实 kick 字节的音频片段，
    /// 正是"会被写成缺素材坏文件"的那类输入。
    #[test]
    fn a_project_with_audio_clips_is_rejected_by_the_project_entry() {
        let dir = scratch_dir("requires-archive");
        let path = dir.join("audio.yeban");
        std::fs::write(&path, b"old bytes that must survive").expect("先放旧内容");
        let before = std::fs::read(&path).expect("读旧内容");

        let error = save_project_file(&demo_project(), &path).expect_err("必须拒绝");
        let SaveError::RequiresArchive { audio_clips } = &error else {
            panic!("必须是 RequiresArchive, 实际: {error:?}");
        };
        assert!(*audio_clips >= 1, "必须点名音频片段条数: {error}");
        assert!(
            error.to_string().contains("save_archive_file"),
            "错误文案必须指路保真归档入口: {error}"
        );
        assert_eq!(
            std::fs::read(&path).expect("旧文件仍在"),
            before,
            "被拒绝的保存绝不能碰旧文件"
        );
    }

    /// 判据 10（T-FIX-329）：归档入口的**跨层核对** —— 音频片段引用的哈希缺索引
    /// 或缺 CAS 字节池条目 ⇒ [`SaveError::MissingAsset`]，目标文件一个字节都不写。
    /// 两条腿各红一次：腿 A 引用在索引里但字节池缺条目；腿 B 反过来。
    #[test]
    fn an_archive_with_dangling_audio_assets_is_rejected() {
        let kick_hash = demo_project_assets()[0].0.clone();

        // 腿 A：project.assets 索引里有 kick 元数据，但归档字节池是空的。
        let dir = scratch_dir("missing-cas");
        let path = dir.join("a.yeban");
        let archive = ProjectArchive {
            project: demo_project(),
            history_dag: b"history".to_vec(),
            proposals: None,
            assets: Vec::new(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        };
        let error = save_archive_file(&archive, &path).expect_err("字节池缺条目必须拒绝");
        let SaveError::MissingAsset { missing } = &error else {
            panic!("必须是 MissingAsset, 实际: {error:?}");
        };
        assert_eq!(
            missing.iter().next().map(|hash| hash.0.clone()),
            Some(kick_hash.0.clone()),
            "缺失清单必须点名 kick 的哈希: {error}"
        );
        assert!(!path.exists(), "⛔ 一个字节都不写");

        // 腿 B：字节池里有真实 kick 字节，但索引被清空（引用无索引可查）。
        let dir = scratch_dir("missing-index");
        let path = dir.join("b.yeban");
        let mut project = demo_project();
        project.assets.clear();
        let archive = ProjectArchive {
            project,
            history_dag: b"history".to_vec(),
            proposals: None,
            assets: demo_project_assets(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        };
        let error = save_archive_file(&archive, &path).expect_err("索引缺条目必须拒绝");
        assert!(
            matches!(error, SaveError::MissingAsset { .. }),
            "实际: {error:?}"
        );
        assert!(!path.exists(), "⛔ 一个字节都不写");
    }

    /// 判据 11（T-FIX-329）：**完整**归档（音频片段 + 索引 + CAS 三者同源）必须
    /// 照常保真写出 —— API 硬化不得拦住合法保存。
    #[test]
    fn a_complete_archive_with_audio_clips_still_saves_byte_identical() {
        use yeban_model::container::read_project_container;

        let dir = scratch_dir("complete");
        let path = dir.join("kept.yeban");
        let archive = ProjectArchive {
            project: demo_project(),
            history_dag: b"tfix-329-history".to_vec(),
            proposals: Some(b"unmerged-proposals".to_vec()),
            assets: demo_project_assets(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        };
        save_archive_file(&archive, &path).expect("完整归档必须照常写出");

        let bytes = std::fs::read(&path).expect("读回");
        let read_back =
            read_project_container(&bytes, &yeban_model::container::ContainerLimits::default())
                .expect("读回容器");
        assert_eq!(
            read_back, archive,
            "归档必须逐字段等价（含音频片段与资产池）"
        );
        // 音频片段引用的哈希必须仍在读回的容器里（引用不再悬空）。
        let audio_ref = archive
            .project
            .clip_pool
            .values()
            .find_map(|entry| match &entry.content {
                ClipContent::Audio { asset, .. } => Some(asset.0.clone()),
                _ => None,
            })
            .expect("demo_project 必须有一个音频片段");
        assert!(
            read_back
                .assets
                .iter()
                .any(|(hash, _)| *hash.0 == audio_ref),
            "音频引用的资产字节必须真的在容器里"
        );
    }
}
