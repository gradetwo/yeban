//! 打开一个真实的 `.yeban` 文件 —— **纯函数**（字节 → 工程）+ **一个文件入口**。
//!
//! 规范来源 (Normative)：
//! - `[ARCH-SEC-003]` / `[MUST-GATE-006]` / `[MUST-GATE-007]`：`.yeban` 是标准 ZIP 子集，
//!   两条 MUST 防御（Zip-Slip、解压炸弹）由 [`yeban_model::container`] 执行 —— 本模块
//!   **不重写**任何一条，只做"把裁决**如实**转达给调用方"。
//! - `[MODEL-AST-002]`：`project.json` 反序列化出的 `YebanProjectV1` 是唯一权威结构。
//! - `[MODEL-ISO-001]`：`history.dag`（提交图谱）与 `assets/{sha256}`（CAS 资产池）
//!   是**另外两层**状态，不属于工程文档。本模块因此提供两条返回形态：
//!   只要工程的 [`open_project_file`]，以及**全保真**的 [`open_project_archive_file`]
//!   （历史 / 资产不会被静默丢弃，见 [`open_project_bytes`] 的文档）。
//!
//! ## 为什么这一层是**纯函数 + 薄 I/O**，而且零 Slint 依赖
//!
//! 容器读取全部逻辑都在 `yeban-model::container` 里，本模块只加三件事：
//!
//! 1. **文件尺寸上限**（[`MAX_PROJECT_FILE_BYTES`]）：在 `read` **之前**按 metadata 判定，
//!    避免"先读进内存再拒绝"；
//! 2. **错误如实映射**（[`OpenError`]）：容器拒绝的压缩法 / 炸弹 / 路径攻击 ⇒
//!    [`OpenError::Container`] 携带**原样的** [`ContainerError`] 变体，
//!    绝不退化成 `Ok(空工程)`；
//! 3. **可注入的上限**（[`ProjectOpenOptions`]）：CI 判据能在几十字节的数据上触发上限。
//!
//! 零 Slint 依赖带来两件事：本机可以 `rustc --edition 2024 --test` **真跑**全部判据
//! （见 notes §4），以及"打开文件"这件事与界面**没有任何耦合** ——
//! 它将来接进 `main.rs` 的命令行 / 会话层时，不需要碰事件循环（本线**没有**改 `main.rs`）。
//!
//! ## 与 `main.rs` 的关系（边界）
//!
//! 本模块交付"可被调用的入口"（[`open_project_file`] / [`open_project_archive_file`] /
//! [`open_project_document_file`]），**不**决定命令行语法、不打印、不碰事件循环。
//! `--open` 的语法与输出属 `src/cli.rs`；把工程注入界面属 `host.rs`（D28 的唯一注入点）。
//! 这条边界让本模块保持零 Slint，从而可被判据在本机用 `rustc --edition 2024 --test` 真跑。
//!
//! ## 容器是**唯一**工程格式（ADR-0001 **D43**）
//!
//! 本模块曾经有一条"裸 `project.json` 兼容读路径"（把裸 JSON 在内存里包成最小容器再读）。
//! D43 明确：1.0.0 之前没有历史包袱与兼容需求 ⇒ **直接推翻**。那条路径连同它的
//! 形态枚举、错误分类与"像 ZIP 就绝不掉进 JSON 分支"的补丁式判断已经**整段删除**。
//!
//! 删掉兼容之后，打开一个非容器文件（裸 JSON / 随机字节 / 空文件）得到的是
//! **一个精确的错误**（[`OpenError::NotAYebanContainer`]，携带容器层的原裁决），
//! 既不是"打开成空工程"，也不是泛化的"未知格式" —— 见 [`open_project_document_file`]。
//! 报告里的 `format=` 只剩一个取值（[`DOCUMENT_FORMAT`]）。

use std::fmt;
use std::path::{Path, PathBuf};

use std::collections::{BTreeMap, BTreeSet};

use yeban_model::container::{
    ContainerError, ContainerLimits, ProjectArchive, read_project_container,
};
use yeban_model::project::{AssetMetadata, ClipContent, MediaKind, YebanProjectV1};

/// 装载时补登的索引项用的许可标识（`[MUST-GATE-014]` 要求逐条可查）。
///
/// 补登的条目**不知道**原始许可（文件里根本没有这条记录），所以写 `unknown` 而不是
/// 编一个看起来合理的值：可查 ≠ 看起来完整，`unknown` 是诚实的读数。
const BACKFILLED_ASSET_LICENSE: &str = "unknown";

/// 合法 `.yeban` 文件的字节上限：**4 GiB**（`1 << 32`）。
///
/// 取值不是拍脑袋：`.yeban` 是 **ZIP32** 容器（`yeban-model` 明确拒绝 ZIP64 与多卷 ——
/// 见 `ContainerError::UnsupportedZip64` / `UnsupportedMultiDisk`），
/// 因此偏移与尺寸字段都是 `u32`；一个超过 4 GiB 的文件不可能是一个合法 `.yeban`。
/// 有了这条，`open_project_file` 就不必把任意大的文件读进内存再让容器去拒绝。
pub const MAX_PROJECT_FILE_BYTES: u64 = 1 << 32;

/// 打开一个 `.yeban` 文件时的可注入上限。
///
/// 为什么把上限做成结构体而不是常量：判据要在**几十字节**的数据上触发"文件过大 / 炸弹"，
/// 不可能真去造一个 4 GiB 的文件。生产调用点用 [`ProjectOpenOptions::default`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectOpenOptions {
    /// 容器层的四道上限 + 条目数上限（`[MUST-GATE-007]`）。
    pub limits: ContainerLimits,
    /// 文件字节上限（默认 [`MAX_PROJECT_FILE_BYTES`]）。
    pub max_file_bytes: u64,
    /// T-FIX-330：资产引用缺口是否升级为**打开失败**。
    ///
    /// `false`（兼容默认）：打开成功，缺口写进 [`ProjectArchive::load_notes`] 与
    /// [`OpenedProject::asset_audit`]，由调用方显示；`true`（生产 UI / 严格入口）：
    /// 首个缺口返回 [`OpenError::MissingAsset`]，⛔ 不给"有片段但无声"的工程开门。
    pub strict_missing_assets: bool,
    /// 预制引用缺口是否升级为**打开失败**（T-FIX-806）。
    ///
    /// `false`（兼容默认）：沿既有行为；`true`（生产 GUI / CLI）：先解析内嵌
    /// 用户预制、再校验全部 `track.preset` 引用（系统预制查发行表，用户预制
    /// 查内嵌副本或用户库），首个缺口返回 [`OpenError::PresetReference`]。
    /// 与 MCP `plan_open` 的 `parse_embedded_presets` + `validate_preset_references`
    /// 同一段口径，⛔ 不静默打开一个缺预制的工程。
    pub strict_preset_references: bool,
}

impl Default for ProjectOpenOptions {
    /// 生产口径：容器默认上限 + [`MAX_PROJECT_FILE_BYTES`]。
    fn default() -> Self {
        Self {
            limits: ContainerLimits::default(),
            max_file_bytes: MAX_PROJECT_FILE_BYTES,
            strict_missing_assets: false,
            strict_preset_references: false,
        }
    }
}

/// 打开 `.yeban` 失败的原因。
///
/// **变体是契约**：容器拒绝的每一种原因都原样保留在 [`Self::Container`] 里
/// （压缩法 / ZIP64 / 炸弹 / 路径穿越 / CRC …），调用方可以据此给出精确的用户提示，
/// 也可以写精确判据。这里**刻意没有**"打开成空工程"这条路径 ——
/// 打不开就是打不开（那是 `[ARCH-SEC-003]` 要的语义）。
#[derive(Debug)]
pub enum OpenError {
    /// 读文件失败（不存在 / 权限 / 是目录 …）。
    Io {
        /// 出问题的路径（原样回显给调用方）。
        path: PathBuf,
        /// 底层 I/O 错误。
        source: std::io::Error,
    },
    /// 文件字节数超过上限（`metadata` 判定或读回后二次判定）。
    FileTooLarge {
        /// 出问题的路径。
        path: PathBuf,
        /// 实际（或声明）字节数。
        len: u64,
        /// 上限。
        max: u64,
    },
    /// 工程引用了容器资产池里**不存在**的资产（T-FIX-330）。
    ///
    /// 典型成因：`project.json` 的 Audio 片段 / `project.assets` 索引 /
    /// `embedded_samples` / 可解析历史指向了 `assets/{sha256}` 里没有的哈希。
    /// 旧行为会把它当成"打开成功"、之后引擎注入时才静默跳过 ⇒ 时间轴有片段但播放无声。
    /// 这里在**打开**这一跳就拒绝，并把首个哈希与引用定位串带进 UI 回执。
    MissingAsset {
        /// 首个缺失的资产哈希。
        hash: yeban_model::AssetHash,
        /// 引用该哈希的定位串（已排序）。
        referenced_by: Vec<String>,
        /// 本次打开一共缺多少个不同资产（首个之外还有几个）。
        missing_count: usize,
    },
    /// 工程轨引用了**无法解析**的预制（T-FIX-806）。
    ///
    /// 与 MCP 打开路径同段校验：内嵌用户预制先解析，再逐轨 `track.preset`
    /// 解析；系统预制必须在发行表里，用户预制必须在内嵌副本或用户库里。
    /// 首个缺口即拒绝打开，⛔ 不静默回干声。
    PresetReference {
        /// 给用户看的完整原因（含缺失标识与查找位置）。
        detail: String,
    },
    /// 文件有 ZIP 结构、但被容器层拒绝：**原样**携带 `yeban-model` 的裁决。
    ///
    /// 这一档的意思是"这**看起来是**一个 `.yeban`，但它坏了 / 不合法"（截断 / CRC /
    /// Zip-Slip / 炸弹 / 缺件 / 非法 `project.json` …）。
    Container(ContainerError),
    /// 文件**不是** `.yeban` 容器（连 ZIP 结构都没有：裸 `project.json` / 随机字节 / 空文件）。
    ///
    /// 携带**容器层的原裁决**（通常 [`ContainerError::EocdNotFound`]）：这正是本变体
    /// 存在的意义 —— "不是容器"这件事本身也有一个精确原因，把它吞掉退化成
    /// "无法识别的文件"、或者更糟地"打开成空工程"，都是在丢信息。
    NotAYebanContainer {
        /// 调用方给的路径。
        path: PathBuf,
        /// 容器读取器给出的原始裁决。
        container: ContainerError,
    },
}

impl OpenError {
    /// 如果这次失败是**容器层**的裁决，借出它的精确变体（压缩法 / ZIP64 / 炸弹 / 路径 …）。
    ///
    /// 存在的意义：调用方要能对"容器拒绝"做**精确**分支（而不是匹配一个字符串），
    /// 判据也要能比较精确错误码 —— [`OpenError`] 本身不能 `PartialEq`（`io::Error` 不是），
    /// 但 [`ContainerError`] 是，于是比较走这个访问器。
    ///
    /// [`Self::NotAYebanContainer`] 也算容器层的裁决（它携带原裁决），因此也返回 `Some`。
    #[must_use]
    pub const fn container(&self) -> Option<&ContainerError> {
        match self {
            Self::Container(error)
            | Self::NotAYebanContainer {
                container: error, ..
            } => Some(error),
            Self::Io { .. }
            | Self::FileTooLarge { .. }
            | Self::MissingAsset { .. }
            | Self::PresetReference { .. } => None,
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "无法读取 `{}`: {source}", path.display())
            }
            Self::FileTooLarge { path, len, max } => write!(
                formatter,
                "`{}` 有 {len} 字节, 超过 `.yeban` 文件上限 {max} 字节 (ZIP32 边界)",
                path.display()
            ),
            Self::MissingAsset {
                hash,
                referenced_by,
                missing_count,
            } => write!(
                formatter,
                "工程引用了 {missing_count} 个缺失资产：首个 `{hash}`（引用: {}）",
                referenced_by.join(", ")
            ),
            Self::PresetReference { detail } => write!(formatter, "预制引用校验失败: {detail}"),
            Self::Container(error) => write!(formatter, "容器被拒绝: {error}"),
            Self::NotAYebanContainer { path, container } => write!(
                formatter,
                "`{}` 不是 `.yeban` 容器 (容器裁决: {container})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Container(error)
            | Self::NotAYebanContainer {
                container: error, ..
            } => Some(error),
            Self::FileTooLarge { .. }
            | Self::MissingAsset { .. }
            | Self::PresetReference { .. } => None,
        }
    }
}

impl From<ContainerError> for OpenError {
    fn from(value: ContainerError) -> Self {
        Self::Container(value)
    }
}

/// **纯函数**：容器字节 → 全保真的 [`ProjectArchive`]（工程 + `history.dag` + 资产池）。
///
/// 这是本模块的核心：无 I/O、无 Slint、无隐藏状态，因此判据可以用
/// `write_project_container` 造一个真容器再用它读回来。
///
/// # Errors
///
/// 容器层的**全部**拒绝原因（路径穿越 / 炸弹 / ZIP64 / 压缩法 / CRC / 缺条目 / 非法工程 JSON …）
/// 原样出现在 [`OpenError::Container`] 里。
pub fn open_project_archive(
    bytes: &[u8],
    limits: &ContainerLimits,
) -> Result<ProjectArchive, OpenError> {
    let mut archive = read_project_container(bytes, limits)?;
    // T-FIX-329 的写前核对要求 Audio 片段的哈希**同时**在索引与池里；而今天 main 产出的
    // 归档只有池、没有索引（`--import-audio` / 录音落地都只写池）。装载时按池字节把索引
    // 补登回去，"打开→保存"才能闭环 —— 否则 329 一进 main，所有存量带音频工程都存不回去。
    // 补登了什么必须能从装载结果里看见（本模块的既有约定），所以同时写一条 `load_notes`。
    let backfilled = backfill_audio_index_from_pool(&mut archive);
    if backfilled > 0 {
        archive.load_notes.push(format!(
            "资产索引补登：{backfilled} 个音频片段引用的资产在 `assets/` 池里有字节、但 \
             `project.assets` 缺索引项；已按池字节补登（`byte_len` 取自池、`original_path` \
             留空、`license` = `{BACKFILLED_ASSET_LICENSE}`）。本条只改内存中的装载结果，\
             文件未被改写；下次保存会把补登后的索引一并写出。"
        ));
    }
    // T-FIX-330：容器读成功只证明 `assets/{sha256}` 的**字节与名字**自洽；
    // 它不证明工程 / 历史引用的哈希真的在池里。默认形态把缺口写进 `load_notes`
    // 并随 `OpenedProject.asset_audit` 交给 UI；严格形态（`OpenProjectOptions::
    // strict_missing_assets`）在 `open_project_archive_file` / `open_project_document_file`
    // 把它升级为 `OpenError::MissingAsset`。
    let audit = audit_project_assets(&archive);
    if !audit.missing.is_empty() {
        let first = &audit.missing[0];
        archive.load_notes.push(format!(
            "资产缺口：工程 / 历史引用了 {} 个不在资产池里的资产；首个 `{}`（引用: {}）",
            audit.missing.len(),
            first.hash,
            first.referenced_by.join(", ")
        ));
    }
    Ok(archive)
}

/// 按资产池字节补登 `project.assets` 缺失的索引项，返回补登条数。
///
/// ## 为什么需要（T-FIX-329 的兼容面）
///
/// 329 让保存入口拒绝"音频片段引用的哈希缺索引或缺池字节"。而**今天 main 产出的归档
/// 恰恰缺索引**：`--import-audio` 与录音落地都只往 CAS 池写字节。于是 329 一合入，
/// 存量带音频工程就会"能打开、存不回去"。修"以后不再产生"（384）治不了已有文件；
/// 这条修的是"已有文件还能不能救"。
///
/// ## 边界（刻意窄）
///
/// - **只补被 `ClipContent::Audio` 引用、且池里真有字节**的哈希。池里也没有的（真缺口）
///   一律不补 —— 留给 [`audit_project_assets`] 的 `missing` 去报，本函数⛔ 不制造"看着有、
///   其实没有"的条目。
/// - **只补索引，不动池**：字节是权威，本函数一个字节都不改。
/// - 池里没被任何片段引用的多余资产**不补** —— 它们必须继续出现在
///   `AssetAudit::unindexed_pool` 里（T-FIX-330 的读数语义）。
///
/// ## 确定性
///
/// 合成元数据只取输入里已有的信息：`byte_len` 取自池（重复哈希按写出端的"最后一次胜出"
/// 取值，与 [`crate::save`] 的去重语义一致）、`media_kind` 恒 `Audio`（引用它的就是音频
/// 片段）、`original_path` 留空（`AssetMetadata` 文档写明它只留痕、不参与寻址）、
/// `license` 用 [`BACKFILLED_ASSET_LICENSE`]。⛔ 不写机器相关路径，同一份字节在任何机器上
/// 补登结果相同。
fn backfill_audio_index_from_pool(archive: &mut ProjectArchive) -> usize {
    let mut missing_index = BTreeSet::new();
    for entry in archive.project.clip_pool.values() {
        if let ClipContent::Audio { asset, .. } = &entry.content
            && !archive.project.assets.contains_key(asset)
        {
            missing_index.insert(asset.clone());
        }
    }
    if missing_index.is_empty() {
        return 0;
    }

    let mut pool_lens: BTreeMap<yeban_model::AssetHash, u64> = BTreeMap::new();
    for (hash, data) in &archive.assets {
        // 倒序不需要：`BTreeMap::insert` 覆盖语义与"最后一次出现胜出"一致。
        pool_lens.insert(hash.clone(), u64::try_from(data.len()).unwrap_or(u64::MAX));
    }

    let mut backfilled = 0;
    for hash in missing_index {
        let Some(byte_len) = pool_lens.get(&hash).copied() else {
            continue; // 池里也没有 ⇒ 真缺口，留给 audit.missing
        };
        archive.project.assets.insert(
            hash.clone(),
            AssetMetadata {
                hash,
                original_path: String::new(),
                byte_len,
                media_kind: MediaKind::Audio,
                license: BACKFILLED_ASSET_LICENSE.to_owned(),

                frames: None,
                sample_rate: None,
            },
        );
        backfilled += 1;
    }
    backfilled
}

/// **纯函数**：容器字节 → `YebanProjectV1`（只要工程文档）。
///
/// 刻意**只**返回工程：`[MODEL-ISO-001]` 说得很清楚，`history.dag` 与资产池是另外两层状态。
/// 需要它们的调用方用 [`open_project_archive`] —— 这不是"静默丢弃"，而是两个显式的返回形态：
/// 名字与文档都写明了各自的取舍，判据也分别覆盖（`container_history_and_assets_survive_the_round_trip`）。
///
/// # Errors
///
/// 同 [`open_project_archive`]。
pub fn open_project_bytes(
    bytes: &[u8],
    limits: &ContainerLimits,
) -> Result<YebanProjectV1, OpenError> {
    Ok(open_project_archive(bytes, limits)?.project)
}

/// **入口**：从一个真实路径打开 `.yeban`，返回全保真的归档（含工程 / 历史 / 资产）。
///
/// 上限可用 [`ProjectOpenOptions`] 注入（判据与将来的受限环境用）。
///
/// # Errors
///
/// 见 [`OpenError`]：读失败 / 文件超上限 / 容器拒绝。
pub fn open_project_archive_file(
    path: impl AsRef<Path>,
    options: &ProjectOpenOptions,
) -> Result<ProjectArchive, OpenError> {
    let path = path.as_ref();
    let bytes = read_capped(path, options.max_file_bytes)?;
    let archive = open_project_archive(&bytes, &options.limits)?;
    if options.strict_preset_references {
        validate_preset_references(&archive)?;
    }
    if options.strict_missing_assets
        && let Some(error) = strict_missing_asset_error(&archive)
    {
        return Err(error);
    }
    Ok(archive)
}

/// 带尺寸闸门的读文件（`metadata` 判一次、读回后再判一次）。
///
/// 为什么要有这条独立函数：`[ARCH-SEC-003]` 的"读之前先按 metadata 判上限"与
/// 容器层防御是**两件事**，而"打开一个文档"（[`open_project_document_file`]）与
/// "打开一个容器"（[`open_project_archive_file`]）都必须走**同一份**闸门 ——
/// 两份就会漂移（其中一份迟早会忘了第二次判定）。
///
/// # Errors
///
/// [`OpenError::Io`]（不存在 / 权限 / 是目录）或 [`OpenError::FileTooLarge`]。
fn read_capped(path: &Path, max_file_bytes: u64) -> Result<Vec<u8>, OpenError> {
    let metadata = std::fs::metadata(path).map_err(|source| OpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > max_file_bytes {
        return Err(OpenError::FileTooLarge {
            path: path.to_path_buf(),
            len: metadata.len(),
            max: max_file_bytes,
        });
    }
    let bytes = std::fs::read(path).map_err(|source| OpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    // `metadata` 与 `read` 之间文件可能变长（TOCTOU）：用**实际读到的**长度再判一次。
    // 这一条不是为了"防攻击"（本地文件本来就归用户所有），而是为了让上限的语义
    // 与"我们真的没有 materialize 超过上限的字节"一致。
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > max_file_bytes {
        return Err(OpenError::FileTooLarge {
            path: path.to_path_buf(),
            len,
            max: max_file_bytes,
        });
    }
    Ok(bytes)
}

/// `--open` 报告里 `format=` 的**唯一**取值。
///
/// 这里刻意**没有**一个"文档形态"枚举：ADR-0001 **D43** 之后 `.yeban` 容器是唯一的
/// 工程格式，一个只剩单个变体的枚举只会邀请人再往里面塞一个变体回来。常量把
/// "读法只有一条"这件事写死成事实 —— 判据也据此断言报告里**不可能**出现第二个值。
pub const DOCUMENT_FORMAT: &str = "yeban-container";

/// 一个**被引用却不在资产池**里的资产（T-FIX-330）。
///
/// `referenced_by` 是给人看的定位串：`track:<id>/placement:<id>/clip:<id>`、
/// `project.assets`、`embedded_samples:<path>`、`history:<commit>/<op>` 等。
/// 同一哈希的多个引用合并进同一条，不重复报。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingAssetReference {
    /// 缺失的 CAS 哈希。
    pub hash: yeban_model::AssetHash,
    /// 谁引用了它（已排序，便于稳定比较与回执）。
    pub referenced_by: Vec<String>,
}

/// 一次打开时的**跨层资产核对**读数（T-FIX-330）。
///
/// `missing` 非空时打开会失败（[`OpenError::MissingAsset`]）；成功打开时它恒为空，
/// 其余字段仍会给出「资产池 / 工程索引 / 引用」三边的多出与缺少，供 UI 与判据读数。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssetAudit {
    /// 被引用到的不同资产哈希数（音轨摆放 / `project.assets` / `embedded_samples` / 历史）。
    pub referenced: usize,
    /// 容器资产池里的不同资产哈希数。
    pub pool: usize,
    /// `project.assets` 索引条目数。
    pub project_index: usize,
    /// 被引用但池里没有字节的资产；严格模式下非空 ⇒ 打开失败，默认模式写 `load_notes`。
    pub missing: Vec<MissingAssetReference>,
    /// `project.assets` 有索引、但池里没有字节的资产（索引多出，不致命，只作读数）。
    pub index_without_pool: Vec<yeban_model::AssetHash>,
    /// 池里有字节、但 `project.assets` 没有登记索引的资产（池多出/索引缺少，不致命）。
    pub unindexed_pool: Vec<yeban_model::AssetHash>,
    /// 池里有字节、但当前工程与可解析历史都没有引用的资产（多出，不致命）。
    pub unreferenced_pool: Vec<yeban_model::AssetHash>,
}

/// 一次成功的"打开一个文档"：全保真归档 + 文件字节数。
///
/// `file_bytes` 是**从磁盘读到的**字节数（不是 `metadata` 声明值）—— 命令行的
/// "打开了哪个文件、多少字节"必须是实测值。
#[derive(Debug, Clone, PartialEq)]
pub struct OpenedProject {
    /// 打开结果（含 `history.dag` / 资产池）。
    pub archive: ProjectArchive,
    /// 实际读入的字节数。
    pub file_bytes: u64,
    /// T-FIX-330：打开时的跨层资产核对读数（严格模式下成功时 `missing` 必为空）。
    pub asset_audit: AssetAudit,
}

/// 后台打开的**阶段**（工作线程向 UI 线程报告进度的唯一口径）。
///
/// 每个阶段只在**进入**时报告一次：`fraction` 是该阶段在整次打开里的锚点
/// （读盘 0.0→0.3、解码 0.55、核对 0.85、完成由排空那一跳写 1.0），⛔ 不是
/// 实测字节比 —— 解码 / 核对是整段调用，中间没有字节级进度可报（T-FIX-428）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenLoadPhase {
    /// 正在读盘（`read_capped` 整文件读；`file_bytes` 只有读完那一刻是 `Some`）。
    Reading,
    /// 正在解码容器（ZIP 解包 + `project.json` 解析）。
    Decoding,
    /// 正在核对资产（严格模式下缺资产直接拒绝）。
    Auditing,
}

/// **命令行的打开入口**：只接受 `.yeban` 容器。
///
/// ## 判定顺序（顺序是契约的一部分）
///
/// 1. 按**容器**读。成功 ⇒ [`Ok`]，`history.dag` / 资产池保真。
/// 2. 失败且文件**有 ZIP 结构**（前 4 字节是 `PK\x03\x04` / `PK\x05\x06` / `PK\x07\x08`）
///    ⇒ **原样上报容器裁决**（[`OpenError::Container`]）：截断 / 被篡改的 `.yeban`
///    因此永远拿到精确的容器错误码。
/// 3. 否则 ⇒ [`OpenError::NotAYebanContainer`]（携带容器原裁决，通常是
///    `EocdNotFound`）：**"你给的文件不是 `.yeban` 容器"** —— 裸 `project.json`、
///    随机字节、空文件都走这一支，一个都不会被"顺手当成工程打开"。
///
/// 任何一条失败路径都**不会**返回默认 / 空工程。
///
/// # Errors
///
/// 见 [`OpenError`]：读失败 / 超上限 / 容器拒绝 / 不是 `.yeban` 容器。
pub fn open_project_document_file(
    path: impl AsRef<Path>,
    options: &ProjectOpenOptions,
) -> Result<OpenedProject, OpenError> {
    let path = path.as_ref();
    // ⛔ 单调用入口没有取消源：取消标志永不置位，分步的 `Ok(None)` 不可能到达这里。
    // 组合只留一份（见 [`open_project_document_file_stepwise`]），⛔ 不复制第二份。
    let never_cancelled = std::sync::atomic::AtomicBool::new(false);
    match open_project_document_file_stepwise(path, options, &never_cancelled, &mut |_, _, _| {}) {
        Ok(Some(opened)) => Ok(opened),
        Ok(None) => unreachable!("单调用入口的取消标志永不置位，分步返回 None 不可能到达这里"),
        Err(error) => Err(error),
    }
}

/// 分步打开（后台线程用）：与 [`open_project_document_file`] **同一段**组合，
///
/// 只是每步之间向调用方报告进度并给取消留出检查点 —— 读盘 / 解码 / 资产核对
/// 全在调用方的线程上跑（T-FIX-428：`wire_open_project` 的工作线程），⛔ 不再
/// 占着 UI 线程。
///
/// `cancelled` 被置位时返回 `Ok(None)`：⛔ 不是错误 —— 取消不是失败，调用方按
/// "用户取消、当前工程一位不动"回执。检查点在每步**之间**（读盘 / 解码 / 核对
/// 各自是整段调用，段内不停 —— 取消总在下一段开始前生效）。
///
/// `on_phase` 在每步**进入**时调一次（`fraction` 是 [`OpenLoadPhase`] 的锚点；
/// `file_bytes` 只有读完那一刻是 `Some`，之前是 `None`）。
///
/// # Errors
///
/// 与 [`open_project_document_file`] 逐字相同。
pub(crate) fn open_project_document_file_stepwise(
    path: &Path,
    options: &ProjectOpenOptions,
    cancelled: &std::sync::atomic::AtomicBool,
    on_phase: &mut dyn FnMut(OpenLoadPhase, f32, Option<u64>),
) -> Result<Option<OpenedProject>, OpenError> {
    on_phase(OpenLoadPhase::Reading, 0.0, None);
    let bytes = read_capped(path, options.max_file_bytes)?;
    let file_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    on_phase(OpenLoadPhase::Reading, 0.3, Some(file_bytes));
    if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(None);
    }
    on_phase(OpenLoadPhase::Decoding, 0.55, Some(file_bytes));
    let archive = match open_project_archive(&bytes, &options.limits) {
        Ok(archive) => archive,
        Err(OpenError::Container(container)) if !has_zip_signature(&bytes) => {
            return Err(OpenError::NotAYebanContainer {
                path: path.to_path_buf(),
                container,
            });
        }
        // ZIP 结构在 ⇒ "容器坏了"，原样转达；`open_project_archive` 只剩容器裁决与
        // 超限两支，而超限已被上面的 `read_capped` 拦下。
        Err(other) => return Err(other),
    };
    if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(None);
    }
    on_phase(OpenLoadPhase::Auditing, 0.85, Some(file_bytes));
    // 与 MCP `plan_open` 同序：先预制引用，再资产缺口（两者都缺时先报预制）。
    if options.strict_preset_references {
        validate_preset_references(&archive)?;
    }
    if options.strict_missing_assets
        && let Some(error) = strict_missing_asset_error(&archive)
    {
        return Err(error);
    }
    let asset_audit = audit_project_assets(&archive);
    Ok(Some(OpenedProject {
        archive,
        file_bytes,
        asset_audit,
    }))
}

/// 这份字节有没有 ZIP 结构（三种签名：local header / EOCD / data descriptor）。
///
/// **这不是格式探测，也不是为兼容而写的护栏**（旧版用它来防止"看起来像 ZIP 的东西掉进
/// 裸 JSON 分支"，那条分支已经删掉）。它现在只服务一件事：把诊断分成
/// "你给的不是 `.yeban`"与"你的 `.yeban` 坏了"两档，而不是把两者糊成一句
/// "无法识别的文件"。截断的 `.yeban` 前 4 字节仍是 `PK\x03\x04`，因此它落在
/// "坏了"那一档，拿到的是精确的容器错误码（判据 `truncated_containers_keep_their_precise_container_verdict`）。
fn has_zip_signature(bytes: &[u8]) -> bool {
    const SIGNATURES: [&[u8; 4]; 3] = [b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"];
    SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature.as_slice()))
}

/// 打开时校验 `track.preset` 的全部引用（T-FIX-806）。
///
/// 默认构建直接复用 `yeban-mcp` 的 `parse_embedded_presets` +
/// `validate_preset_references`（与 MCP `yeban_open_project` **同一段**实现）：
/// 先解析 `presets/user/<id>.json` 内嵌副本，再逐轨解析预制标识；系统预制
/// 必须在发行表里，用户预制必须在内嵌副本或用户库里。首个缺口即返回
/// [`OpenError::PresetReference`]，⛔ 不静默打开一个缺预制的工程。
///
/// `--no-default-features` 构建没有 `yeban-mcp` 依赖边，因此保留一份**逐条同义**
/// 的本地校验（同一条 `Preset::parse` / `is_factory_preset` / 用户库路径），
/// 不让"未编译 MCP"退化成静默放行。
#[cfg(feature = "in-process-mcp")]
fn validate_preset_references(
    archive: &yeban_model::container::ProjectArchive,
) -> Result<(), OpenError> {
    use yeban_mcp::domain::error::Fault;
    fn fault_message(fault: Fault) -> String {
        match fault {
            Fault::Domain { message, .. } => message,
            Fault::Impl { error } => error.message,
        }
    }
    let embedded = yeban_mcp::domain::preset::parse_embedded_presets(&archive.embedded_presets)
        .map_err(|fault| OpenError::PresetReference {
            detail: fault_message(fault),
        })?;
    yeban_mcp::domain::preset::validate_preset_references(&archive.project, &embedded).map_err(
        |fault| OpenError::PresetReference {
            detail: fault_message(fault),
        },
    )?;
    Ok(())
}

#[cfg(not(feature = "in-process-mcp"))]
fn validate_preset_references(
    archive: &yeban_model::container::ProjectArchive,
) -> Result<(), OpenError> {
    use std::collections::BTreeMap;
    use yeban_model::preset::{
        Preset, is_factory_preset, preset_id_from_embedded_entry, user_preset_library_dir,
        validate_preset_id,
    };
    const USER_PRESET_READ_MAX_BYTES: u64 = 16 * 1024 * 1024;
    let mut embedded: BTreeMap<String, Preset> = BTreeMap::new();
    for (entry_name, bytes) in &archive.embedded_presets {
        let Some(id) = preset_id_from_embedded_entry(entry_name) else {
            return Err(OpenError::PresetReference {
                detail: format!("内嵌预制条目名非法：`{entry_name}`"),
            });
        };
        let preset = Preset::parse(bytes).map_err(|error| OpenError::PresetReference {
            detail: format!("内嵌预制 `{id}` 解析失败：{error}"),
        })?;
        if preset.id != id {
            return Err(OpenError::PresetReference {
                detail: format!(
                    "内嵌预制条目 `{entry_name}` 里的标识是 `{}`，与条目名不符",
                    preset.id
                ),
            });
        }
        embedded.insert(id, preset);
    }
    for track in archive.project.tracks.values() {
        let Some(state) = track.preset.as_ref() else {
            continue;
        };
        let Some(id) = state.preset_id.as_deref() else {
            continue;
        };
        validate_preset_id(id).map_err(|error| OpenError::PresetReference {
            detail: error.to_string(),
        })?;
        if is_factory_preset(id) || embedded.contains_key(id) {
            continue;
        }
        let file = id.strip_prefix("user:").unwrap_or(id);
        let path = user_preset_library_dir()
            .map(|dir| dir.join(format!("{file}.json")))
            .map_err(|detail| OpenError::PresetReference { detail })?;
        let too_large =
            std::fs::metadata(&path).is_ok_and(|meta| meta.len() > USER_PRESET_READ_MAX_BYTES);
        let bytes = if too_large {
            Err(OpenError::PresetReference {
                detail: format!("用户预制 `{id}` 超过读取上限 {USER_PRESET_READ_MAX_BYTES} 字节"),
            })
        } else {
            std::fs::read(&path).map_err(|_| OpenError::PresetReference {
                detail: format!(
                    "缺用户预制 `{id}`（用户库里没有 {}），拒绝静默回干声",
                    path.display()
                ),
            })
        }?;
        Preset::parse(&bytes).map_err(|error| OpenError::PresetReference {
            detail: format!("用户预制 `{id}` 损坏：{error}"),
        })?;
    }
    Ok(())
}

/// **任务书点名的入口**：`open_project_file(path) -> Result<YebanProjectV1, OpenError>`。
///
/// 用生产默认上限（[`ProjectOpenOptions::default`]）；只要工程文档。
/// 需要 `history.dag` / 资产池的调用方改用 [`open_project_archive_file`]。
///
/// # Errors
///
/// 同 [`open_project_archive_file`]。
pub fn open_project_file(path: impl AsRef<Path>) -> Result<YebanProjectV1, OpenError> {
    let archive = open_project_archive_file(path, &ProjectOpenOptions::default())?;
    Ok(archive.project)
}

/// 收集一个归档里所有**必须能从资产池取到字节**的引用。
///
/// 口径（T-FIX-330，逐条可复核）：
/// - `clip_pool` 的每个 `ClipContent::Audio`；
/// - `tracks[*].clips[*]` 的摆放（给 `track/placement/clip` 定位串，同一片段多摆一处就多一条引用）；
/// - `project.embedded_samples` 的哈希值（SFZ / 样本内嵌索引）；
/// - `history.dag` 中**可解析**的 `AddClip` / `SetClipAsset` / `SetAsset`：历史里的旧资产
///   仍要能取到字节才能重放撤销；解不开的历史**不阻断打开**（它与现有
///   `decode_history_dag` 的消费者同口径，坏历史由那条线响亮报错）。
///
/// `project.assets` 的索引键**不算**硬引用：它可能保留已删除资产的元数据；
/// 与池的差集分别记进 [`AssetAudit::index_without_pool`] / [`AssetAudit::unindexed_pool`]。
fn collect_asset_references(
    archive: &ProjectArchive,
) -> BTreeMap<yeban_model::AssetHash, BTreeSet<String>> {
    let mut refs: BTreeMap<yeban_model::AssetHash, BTreeSet<String>> = BTreeMap::new();
    let mut add = |hash: &yeban_model::AssetHash, label: String| {
        refs.entry(hash.clone()).or_default().insert(label);
    };

    for (clip_id, clip) in &archive.project.clip_pool {
        if let ClipContent::Audio { asset, .. } = &clip.content {
            add(asset, format!("clip:{clip_id}"));
        }
    }
    for (track_id, track) in &archive.project.tracks {
        for (placement_id, placement) in &track.clips {
            let Some(clip) = archive.project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            if let ClipContent::Audio { asset, .. } = &clip.content {
                add(
                    asset,
                    format!(
                        "track:{track_id}/placement:{placement_id}/clip:{}",
                        placement.clip_id
                    ),
                );
            }
        }
    }
    for (path, hash) in &archive.project.embedded_samples {
        if let Ok(hash) = yeban_model::AssetHash::parse(hash.clone()) {
            add(&hash, format!("embedded_samples:{path}"));
        }
    }

    if let Ok(Some(graph)) = yeban_model::decode_history_dag(&archive.history_dag) {
        for (commit_id, commit) in &graph.commits {
            for (index, stamped) in commit.ops.iter().enumerate() {
                let label = format!("history:{commit_id}/op{index}");
                match &stamped.op {
                    yeban_model::Op::AddClip { clip } => {
                        if let ClipContent::Audio { asset, .. } = &clip.content {
                            add(asset, format!("{label}/AddClip"));
                        }
                    }
                    yeban_model::Op::SetClipAsset {
                        old_asset,
                        new_asset,
                        ..
                    } => {
                        add(old_asset, format!("{label}/SetClipAsset.old"));
                        add(new_asset, format!("{label}/SetClipAsset.new"));
                    }
                    yeban_model::Op::SetAsset { asset_hash, .. } => {
                        add(asset_hash, format!("{label}/SetAsset"));
                    }
                    _ => {}
                }
            }
        }
    }
    refs
}

/// 把审计出的缺口升级成**打开失败**；`None` 表示没有缺口。
fn strict_missing_asset_error(archive: &ProjectArchive) -> Option<OpenError> {
    let audit = audit_project_assets(archive);
    let first = audit.missing.first()?;
    Some(OpenError::MissingAsset {
        hash: first.hash.clone(),
        referenced_by: first.referenced_by.clone(),
        missing_count: audit.missing.len(),
    })
}

/// **跨层资产核对**：枚举工程 / 历史里的可解析引用，与容器资产池做双向差集。
///
/// 返回读数；`missing` 非空时 [`open_project_archive`] 会把它升级为
/// [`OpenError::MissingAsset`]（打开失败），而不是让引擎在注入阶段静默跳过。
#[must_use]
pub fn audit_project_assets(archive: &ProjectArchive) -> AssetAudit {
    let refs = collect_asset_references(archive);
    let pool: BTreeSet<yeban_model::AssetHash> = archive
        .assets
        .iter()
        .map(|(hash, _)| hash.clone())
        .collect();
    let project_index: BTreeSet<yeban_model::AssetHash> =
        archive.project.assets.keys().cloned().collect();

    let mut missing: Vec<MissingAssetReference> = refs
        .iter()
        .filter(|(hash, _)| !pool.contains(*hash))
        .map(|(hash, labels)| MissingAssetReference {
            hash: hash.clone(),
            referenced_by: labels.iter().cloned().collect(),
        })
        .collect();
    missing.sort_by(|left, right| left.hash.cmp(&right.hash));

    AssetAudit {
        referenced: refs.len(),
        pool: pool.len(),
        project_index: project_index.len(),
        missing,
        index_without_pool: project_index.difference(&pool).cloned().collect(),
        unindexed_pool: pool.difference(&project_index).cloned().collect(),
        unreferenced_pool: pool
            .iter()
            .filter(|hash| !refs.contains_key(*hash))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::bridge::demo_project;
    use yeban_model::ids::AssetHash;

    // ------------------------------------------------------------------
    // 一个最小 ZIP 字节修补器：所有"被篡改 / 被拒绝"的判据都作用在**真容器**上，
    // 而不是手写的假字节 —— 这样"容器拒绝的东西"与"app 如实转达的东西"是同一批。
    // ------------------------------------------------------------------

    const EOCD_SIGNATURE: u32 = 0x0605_4b50;
    const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
    const LOCAL_SIGNATURE: u32 = 0x0403_4b50;

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn patch_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn patch_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// EOCD 记录（末尾注释之前的 22 字节固定部分）的偏移。
    fn eocd_offset(bytes: &[u8]) -> usize {
        let floor = bytes.len().saturating_sub(22 + 65_535);
        (floor..=bytes.len() - 22)
            .rev()
            .find(|&at| u32_at(bytes, at) == EOCD_SIGNATURE)
            .expect("真容器必须有 EOCD")
    }

    /// 第 `index` 条 central directory 记录的偏移。
    fn central_record(bytes: &[u8], index: usize) -> usize {
        let eocd = eocd_offset(bytes);
        let mut at = u32_at(bytes, eocd + 16) as usize;
        for _ in 0..index {
            let name_len = u16_at(bytes, at + 28) as usize;
            let extra_len = u16_at(bytes, at + 30) as usize;
            let comment_len = u16_at(bytes, at + 32) as usize;
            at += 46 + name_len + extra_len + comment_len;
        }
        assert_eq!(u32_at(bytes, at), CENTRAL_SIGNATURE, "CD 记录签名");
        at
    }

    /// 第 `index` 条 local file header 的偏移。
    fn local_record(bytes: &[u8], index: usize) -> usize {
        let at = u32_at(bytes, central_record(bytes, index) + 42) as usize;
        assert_eq!(u32_at(bytes, at), LOCAL_SIGNATURE, "local header 签名");
        at
    }

    /// 把第 `index` 条条目的名字**同时**改成 `name`（两处必须一致，否则会先撞
    /// `LocalCentralMismatch` 而不是我们想测的那条防御）。
    fn patch_name(bytes: &mut [u8], index: usize, name: &str) {
        let central = central_record(bytes, index);
        let local = local_record(bytes, index);
        let central_len = u16_at(bytes, central + 28) as usize;
        let local_len = u16_at(bytes, local + 26) as usize;
        assert_eq!(central_len, name.len(), "补丁长度必须与原名字一致");
        assert_eq!(local_len, name.len(), "补丁长度必须与原名字一致");
        bytes[central + 46..central + 46 + central_len].copy_from_slice(name.as_bytes());
        bytes[local + 30..local + 30 + local_len].copy_from_slice(name.as_bytes());
    }

    /// 造一个真容器（`project.json` + `history.dag` + `project` 真正引用的 CAS 资产）。
    ///
    /// T-FIX-330：`demo_project` 的音频片段引用运行期合成的 kick；
    /// 容器必须带上**同一份**字节，否则打开会（正确地）报 `MissingAsset`。
    fn container_bytes() -> (Vec<u8>, YebanProjectV1) {
        let project = demo_project();
        let assets: BTreeMap<AssetHash, Vec<u8>> = yeban_model::samples::demo_project_assets()
            .into_iter()
            .collect();
        let bytes =
            yeban_model::container::write_project_container(&project, b"dag-bytes", &assets)
                .expect("写出真容器");
        (bytes, project)
    }

    /// 把"容器层裁决"取出来做**精确**比较（[`OpenError`] 因 `io::Error` 不能 `PartialEq`，
    /// 但 [`ContainerError`] 可以）。
    fn container_err(result: Result<YebanProjectV1, OpenError>) -> Option<ContainerError> {
        match result {
            Err(OpenError::Container(error)) => Some(error),
            Err(other) => panic!("期望容器裁决, 实测 {other:?}"),
            Ok(_) => None,
        }
    }

    /// 判据 17: **成功路径** —— `write_project_container` 造的真容器能被打开，
    /// 工程逐字段相等，且 `history.dag` / 资产池**全保真**。
    #[test]
    fn real_container_round_trips_through_the_open_entry() {
        let (bytes, project) = container_bytes();
        let limits = ContainerLimits::default();

        let archive = open_project_archive(&bytes, &limits).expect("真容器必须能打开");
        assert_eq!(archive.project, project, "工程文档必须逐字段一致");
        assert_eq!(archive.history_dag, b"dag-bytes", "history.dag 必须保真");
        assert_eq!(archive.assets.len(), 1);
        assert_eq!(
            archive.assets[0].1,
            yeban_model::samples::demo_project_assets()[0].1
        );

        let only_project = open_project_bytes(&bytes, &limits).expect("只要工程的形态");
        assert_eq!(only_project, project);
        assert_eq!(only_project.title, "夜半 Yeban");
        assert!(!only_project.tracks.is_empty());
    }

    /// T-FIX-330：Audio 片段引用的资产不在池里 ⇒ 默认给**可读 warning**，
    /// 严格模式（生产 UI）升级为 `OpenError::MissingAsset`，错误带哈希与引用定位。
    #[test]
    fn audio_clip_without_pool_bytes_is_a_loud_warning_then_a_strict_error() {
        let project = demo_project();
        let asset_hash = project
            .assets
            .keys()
            .next()
            .expect("demo 工程必须有 kick 资产索引")
            .clone();
        let bytes =
            yeban_model::container::write_project_container(&project, b"", &BTreeMap::new())
                .expect("写出缺资产的容器");

        let archive = open_project_archive(&bytes, &ContainerLimits::default())
            .expect("默认模式必须打开并给 warning，而不是只写 stderr");
        let audit = audit_project_assets(&archive);
        assert_eq!(audit.missing.len(), 1);
        let gap = &audit.missing[0];
        assert_eq!(gap.hash, asset_hash);
        assert!(
            gap.referenced_by
                .iter()
                .any(|label| label.starts_with("track:")),
            "引用定位必须能指到轨/摆放: {:?}",
            gap.referenced_by
        );
        assert!(
            gap.referenced_by
                .iter()
                .any(|label| label.starts_with("clip:")),
            "引用定位必须能指到片段: {:?}",
            gap.referenced_by
        );
        assert!(
            archive
                .load_notes
                .iter()
                .any(|note| note.contains("资产缺口") && note.contains(&asset_hash.to_string())),
            "缺口必须进 `load_notes`（UI / CLI 可显示）: {:?}",
            archive.load_notes
        );

        let dir =
            std::env::temp_dir().join(format!("yeban-open-missing-asset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("missing-asset.yeban");
        std::fs::write(&path, &bytes).expect("写临时容器");
        let strict = ProjectOpenOptions {
            strict_missing_assets: true,
            ..ProjectOpenOptions::default()
        };
        match open_project_archive_file(&path, &strict) {
            Err(OpenError::MissingAsset {
                hash,
                referenced_by,
                missing_count,
            }) => {
                assert_eq!(hash, asset_hash);
                assert_eq!(missing_count, 1);
                assert!(
                    referenced_by
                        .iter()
                        .any(|label| label.starts_with("track:"))
                );
            }
            other => panic!("严格模式期望 MissingAsset，实测 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-330：池里多出的资产**不**阻断打开，但必须在审计读数里出现。
    #[test]
    fn extra_pool_asset_is_reported_in_the_audit_not_rejected() {
        let project = demo_project();
        let mut assets: BTreeMap<AssetHash, Vec<u8>> = yeban_model::samples::demo_project_assets()
            .into_iter()
            .collect();
        let extra = b"never-referenced-by-project".to_vec();
        let extra_hash = AssetHash::of_bytes(&extra);
        assets.insert(extra_hash.clone(), extra);
        let bytes = yeban_model::container::write_project_container(&project, b"", &assets)
            .expect("写出带多余资产的容器");

        let archive = open_project_archive(&bytes, &ContainerLimits::default())
            .expect("多余池资产不得阻断打开");
        let audit = audit_project_assets(&archive);
        assert!(audit.missing.is_empty());
        assert_eq!(audit.pool, 2);
        assert_eq!(audit.project_index, 1);
        assert_eq!(audit.unindexed_pool, vec![extra_hash.clone()]);
        assert_eq!(audit.unreferenced_pool, vec![extra_hash]);
    }

    /// T-FIX-330：`project.assets` 比池多出的索引项只作读数，不阻断打开。
    #[test]
    fn project_index_without_pool_is_reported_but_not_rejected() {
        let mut project = yeban_model::samples::default_project();
        let fake = AssetHash::of_bytes(b"index-only-asset");
        project.assets.insert(
            fake.clone(),
            yeban_model::project::AssetMetadata {
                hash: fake.clone(),
                original_path: "samples/index-only.wav".to_owned(),
                byte_len: 0,
                media_kind: yeban_model::project::MediaKind::Audio,
                license: "CC0-1.0".to_owned(),

                frames: None,
                sample_rate: None,
            },
        );
        let bytes =
            yeban_model::container::write_project_container(&project, b"", &BTreeMap::new())
                .expect("写出带孤立索引的容器");

        let archive = open_project_archive(&bytes, &ContainerLimits::default())
            .expect("孤立索引不得阻断打开");
        let audit = audit_project_assets(&archive);
        assert!(audit.missing.is_empty(), "索引项不是硬引用");
        assert_eq!(audit.index_without_pool, vec![fake]);
    }

    /// 判据 18: **截断容器必须报错，绝不"打开成空工程"**。
    ///
    /// 逐个截断长度都断言 `Err`，并额外断言"它不等于空工程的投影" ——
    /// 后者是这条判据的灵魂：把错误吞掉当空工程，正是本线要排除的失败模式。
    #[test]
    fn truncated_containers_error_instead_of_opening_an_empty_project() {
        let (bytes, _) = container_bytes();
        let limits = ContainerLimits::default();
        let empty = YebanProjectV1::default();
        for cut in [
            1_usize,
            2,
            10,
            22,
            bytes.len() / 3,
            bytes.len() / 2,
            bytes.len() - 1,
        ] {
            let truncated = &bytes[..bytes.len() - cut];
            let result = open_project_bytes(truncated, &limits);
            assert!(result.is_err(), "截断 {cut} 字节后必须报错");
            assert_ne!(result.as_ref().ok(), Some(&empty), "错误不得退化成空工程");
        }
        // 完全空 / 完全不是 ZIP 的输入同样是错误。
        assert!(open_project_bytes(&[], &limits).is_err());
        assert!(open_project_bytes(b"not a zip at all", &limits).is_err());
    }

    /// 判据 19: **被篡改的容器** —— 容器层的每一种裁决都被原样转达（精确错误码）。
    ///
    /// 覆盖任务书点名的三类：压缩法（deflate）、路径攻击（`..`）、炸弹（超小上限）；
    /// 外加 CRC 篡改。全部作用在**真容器**的字节上。
    #[test]
    fn tampered_containers_report_the_container_verdict_verbatim() {
        let (bytes, _) = container_bytes();
        let limits = ContainerLimits::default();

        // (a) 压缩法：把 `project.json` 的 method 改成 8（deflate）⇒ 明确拒绝，不猜内容。
        let mut deflate = bytes.clone();
        let central = central_record(&deflate, 0);
        let local = local_record(&deflate, 0);
        patch_u16(&mut deflate, central + 10, 8);
        patch_u16(&mut deflate, local + 8, 8);
        assert_eq!(
            container_err(open_project_bytes(&deflate, &limits)),
            Some(ContainerError::UnsupportedCompression {
                index: 0,
                method: 8
            }),
            "deflate 条目必须报 UnsupportedCompression"
        );

        // (b) 路径攻击：把条目名改成 `../proj.json`（Zip-Slip，`MUST-GATE-006`）。
        let mut slip = bytes.clone();
        patch_name(&mut slip, 0, "../proj.json");
        assert_eq!(
            container_err(open_project_bytes(&slip, &limits)),
            Some(ContainerError::ParentDirSegment {
                name: "../proj.json".to_owned(),
            }),
            "`..` 段必须报 ParentDirSegment"
        );

        // (c) CRC 篡改：翻转 `project.json` 数据区的一个字节。
        let mut corrupted = bytes.clone();
        let local = local_record(&corrupted, 0);
        let name_len = u16_at(&corrupted, local + 26) as usize;
        let extra_len = u16_at(&corrupted, local + 28) as usize;
        let data = local + 30 + name_len + extra_len;
        corrupted[data] ^= 0xff;
        match open_project_bytes(&corrupted, &limits) {
            Err(OpenError::Container(ContainerError::CrcMismatch { index, name, .. })) => {
                assert_eq!(index, 0);
                assert_eq!(name, "project.json");
            }
            other => panic!("CRC 篡改必须报 CrcMismatch, 实测 {other:?}"),
        }

        // (d) 炸弹：把单条目上限压到 4 字节 ⇒ `EntryTooLarge`（用**真容器**触发，
        //     不需要造 2 GB 的归档）。
        let bomb = ContainerLimits {
            max_entry_bytes: 4,
            ..ContainerLimits::default()
        };
        match open_project_bytes(&bytes, &bomb) {
            Err(OpenError::Container(ContainerError::EntryTooLarge { declared, max })) => {
                assert!(declared > 4);
                assert_eq!(max, 4);
            }
            other => panic!("超小上限必须报 EntryTooLarge, 实测 {other:?}"),
        }

        // (e) 条目数上限（同一族防御的另一道）。
        let few = ContainerLimits {
            max_entries: 1,
            ..ContainerLimits::default()
        };
        assert_eq!(
            container_err(open_project_bytes(&bytes, &few)),
            Some(ContainerError::TooManyEntries { found: 3, max: 1 })
        );

        // (f) 声明撒谎：把 `project.json` **声明**的解压体积改成 1 MB（实际只有几百字节）
        //     ⇒ 实际读入量 / 归档总字节远低于比率上限，但 `stored` 的尺寸自相矛盾
        //     ⇒ `StoredSizeMismatch`（比率闸门不再看声明值）。
        let mut ratio_bomb = bytes.clone();
        let central = central_record(&ratio_bomb, 0);
        let local = local_record(&ratio_bomb, 0);
        let compressed = u32_at(&ratio_bomb, central + 20);
        patch_u32(&mut ratio_bomb, central + 24, 1_000_000);
        patch_u32(&mut ratio_bomb, local + 22, 1_000_000);
        match container_err(open_project_bytes(&ratio_bomb, &limits)) {
            Some(ContainerError::StoredSizeMismatch {
                index,
                name,
                compressed: seen,
                uncompressed,
            }) => {
                assert_eq!(index, 0);
                assert_eq!(name, "project.json");
                assert_eq!(seen, compressed);
                assert_eq!(uncompressed, 1_000_000);
            }
            other => panic!("声明尺寸撒谎必须报 StoredSizeMismatch, 实测 {other:?}"),
        }

        // (g) ZIP64 哨兵：把 `uncompressed` 改成 `0xFFFFFFFF` ⇒ 明确拒绝（不支持 ZIP64）。
        let mut zip64 = bytes.clone();
        let central = central_record(&zip64, 0);
        patch_u32(&mut zip64, central + 24, u32::MAX);
        assert_eq!(
            container_err(open_project_bytes(&zip64, &limits)),
            Some(ContainerError::UnsupportedZip64)
        );

        // (h) **中央目录**篡改（与 (c) 的数据区篡改是两处不同的字节）：改 central
        //     directory 里 `project.json` 的 CRC32 ⇒ 容器在比对 local / central 时
        //     立刻报 `LocalCentralMismatch`（**中央目录被改过**这件事本身的精确裁决；
        //     它先于"重算数据 CRC"那一步）。
        let mut central_crc = bytes.clone();
        let central = central_record(&central_crc, 0);
        let original = u32_at(&central_crc, central + 16);
        patch_u32(&mut central_crc, central + 16, original ^ 0xffff_ffff);
        assert_eq!(
            container_err(open_project_bytes(&central_crc, &limits)),
            Some(ContainerError::LocalCentralMismatch {
                index: 0,
                name: "project.json".to_owned(),
            }),
            "中央目录篡改必须被拒绝，且给出精确裁决"
        );
    }

    /// 判据 20: 缺失条目 / 非工程 JSON 也要报**精确**错误（不是空工程）。
    #[test]
    fn structurally_wrong_containers_report_precise_errors() {
        use yeban_model::container::{ContainerEntry, write_container};

        let limits = ContainerLimits::default();
        // `assets/` 下的名字不是规范 SHA-256 ⇒ 容器先报 `InvalidAssetName`
        // （判定顺序：先 `assets/` 前缀 → 再 CAS 名解析 → 才轮到"未定义条目"）。
        let bytes = write_container(&[
            ContainerEntry::new("history.dag", b"x".to_vec()),
            ContainerEntry::new("assets/00", b"y".to_vec()),
        ])
        .expect("写出容器");
        assert_eq!(
            container_err(open_project_bytes(&bytes, &limits)),
            Some(ContainerError::InvalidAssetName {
                name: "assets/00".to_owned(),
            }),
            "非规范资产名必须被拒绝（不是静默跳过）"
        );

        // 既不是三种固定条目、也没有 `assets/` 前缀 ⇒ `UnexpectedContainerEntry`。
        let stray = write_container(&[
            ContainerEntry::new("project.json", b"{}".to_vec()),
            ContainerEntry::new("history.dag", b"x".to_vec()),
            ContainerEntry::new("notes.txt", b"y".to_vec()),
        ])
        .expect("写出容器");
        assert_eq!(
            container_err(open_project_bytes(&stray, &limits)),
            Some(ContainerError::UnexpectedContainerEntry {
                name: "notes.txt".to_owned(),
            }),
            "未定义条目必须被拒绝"
        );

        let only_history = write_container(&[ContainerEntry::new("history.dag", b"x".to_vec())])
            .expect("写出容器");
        assert_eq!(
            container_err(open_project_bytes(&only_history, &limits)),
            Some(ContainerError::MissingProjectJson)
        );

        let bad_json = write_container(&[
            ContainerEntry::new("project.json", b"{ not json".to_vec()),
            ContainerEntry::new("history.dag", b"x".to_vec()),
        ])
        .expect("写出容器");
        match open_project_bytes(&bad_json, &limits) {
            Err(OpenError::Container(ContainerError::InvalidProjectJson { detail })) => {
                assert!(!detail.is_empty(), "必须携带底层解析器的描述");
            }
            other => panic!("非法 project.json 必须报 InvalidProjectJson, 实测 {other:?}"),
        }
    }

    /// 判据 21: **文件入口**的三种形态：成功 / 文件超限 / 不存在。
    ///
    /// 超限用 `ProjectOpenOptions` 注入一个极小上限（而不是造一个 4 GiB 文件）。
    #[test]
    fn file_entry_reports_success_limits_and_missing_files() {
        let (bytes, project) = container_bytes();
        let dir = std::env::temp_dir().join(format!("yeban-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("round-trip.yeban");
        std::fs::write(&path, &bytes).expect("写临时容器");

        assert_eq!(open_project_file(&path).expect("从磁盘打开"), project);

        let strict = ProjectOpenOptions {
            max_file_bytes: 8,
            ..ProjectOpenOptions::default()
        };
        match open_project_archive_file(&path, &strict) {
            Err(OpenError::FileTooLarge { len, max, .. }) => {
                assert!(len > 8);
                assert_eq!(max, 8);
            }
            other => panic!("超限文件必须报 FileTooLarge, 实测 {other:?}"),
        }

        let missing = dir.join("does-not-exist.yeban");
        match open_project_file(&missing) {
            Err(OpenError::Io { path, .. }) => assert_eq!(path, missing),
            other => panic!("缺失文件必须报 Io, 实测 {other:?}"),
        }

        // 上限**不**放宽容器防御：极小上限照样先被文件尺寸拦下（顺序是契约的一部分）。
        let tiny = ProjectOpenOptions {
            limits: ContainerLimits {
                max_entry_bytes: 1,
                ..ContainerLimits::default()
            },
            ..ProjectOpenOptions::default()
        };
        assert!(matches!(
            open_project_archive_file(&path, &tiny),
            Err(OpenError::Container(ContainerError::EntryTooLarge { .. }))
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据 22: 错误的 `Display` 必须**带上**容器的原始描述（人可读、可定位）。
    #[test]
    fn open_errors_are_self_describing() {
        let (bytes, _) = container_bytes();
        let bomb = ContainerLimits {
            max_entry_bytes: 4,
            ..ContainerLimits::default()
        };
        let error = open_project_bytes(&bytes, &bomb).expect_err("必须报错");
        let text = error.to_string();
        assert!(text.contains("容器被拒绝"), "实测: {text}");
        assert!(text.contains("per-entry limit"), "必须带上容器原文: {text}");

        let missing = std::env::temp_dir().join("yeban-definitely-missing.yeban");
        let error = open_project_file(&missing).expect_err("必须报错");
        assert!(error.to_string().contains("无法读取"), "实测: {error}");
        assert!(
            std::error::Error::source(&error).is_some(),
            "Io 必须暴露 source"
        );
    }

    // ------------------------------------------------------------------
    // 文档入口（`open_project_document_file`）—— 命令行的 `--open` 走这条。
    // 容器是**唯一**格式（D43）：这一组判据里"裸 JSON 必须被拒绝"是**反转**来的，
    // 不是新写的 —— 原判据断言它"能打开"。
    // ------------------------------------------------------------------

    /// 判据 25: **真容器**经文档入口打开 ⇒ 成功，归档全保真，报告取值只有一个可能。
    #[test]
    fn document_entry_opens_a_real_container() {
        let (bytes, project) = container_bytes();
        let dir = std::env::temp_dir().join(format!("yeban-doc-container-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("real.yeban");
        std::fs::write(&path, &bytes).expect("写临时容器");

        let opened = open_project_document_file(&path, &ProjectOpenOptions::default())
            .expect("真容器必须能打开");
        assert_eq!(opened.file_bytes, bytes.len() as u64);
        assert_eq!(opened.archive.project, project);
        assert_eq!(opened.archive.history_dag, b"dag-bytes");
        assert_eq!(opened.archive.assets.len(), 1);
        // 报告里 `format=` 的唯一取值：容器。
        assert_eq!(DOCUMENT_FORMAT, "yeban-container");

        // 与"只要归档"的入口逐字段一致（两条路不得漂移）。
        assert_eq!(
            opened.archive,
            open_project_archive_file(&path, &ProjectOpenOptions::default()).expect("归档入口")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据 26（**反转**）: 裸 `project.json` 必须被**明确拒绝**，而且理由精确。
    ///
    /// 旧判据 `document_entry_accepts_a_bare_project_json_document` 断言它"能打开"且形态
    /// 是 `BareProjectJson`。D43 删掉兼容路径之后，同一条输入必须走**拒绝**分支 ——
    /// 这是"我们真的删掉了兼容"的机械证据，而不是"没人测了"。
    ///
    /// 还额外证明"拒绝的是**容器边界**，不是内容"：同一份 JSON 字节包进真容器就能打开。
    #[test]
    fn document_entry_rejects_a_bare_project_json_document() {
        use yeban_model::container::{ContainerEntry, read_container, write_container};

        let (bytes, project) = container_bytes();
        let limits = ContainerLimits::default();
        let json = read_container(&bytes, &limits)
            .expect("真容器")
            .get("project.json")
            .expect("容器必有 project.json")
            .data
            .clone();

        let dir = std::env::temp_dir().join(format!("yeban-doc-bare-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        // 前面加空白：旧判定规则是"首个**非空白**字节是 `{`"。加空白是为了证明
        // 新行为不依赖任何"看起来像 JSON"的启发式 —— 它只看容器。
        let mut padded = b"\n  ".to_vec();
        padded.extend_from_slice(&json);
        let path = dir.join("project.json");
        std::fs::write(&path, &padded).expect("写裸 JSON");

        match open_project_document_file(&path, &ProjectOpenOptions::default()) {
            Err(error @ OpenError::NotAYebanContainer { .. }) => {
                assert_eq!(
                    error.container(),
                    Some(&ContainerError::EocdNotFound),
                    "必须携带容器的精确原裁决"
                );
                let text = error.to_string();
                assert!(text.contains("不是 `.yeban` 容器"), "实测: {text}");
                assert!(
                    text.contains("end-of-central-directory"),
                    "必须带上容器原文: {text}"
                );
            }
            other => panic!("裸 project.json 必须被明确拒绝, 实测 {other:?}"),
        }

        // 拒绝**不是**因为"这份工程内容不行"：同一份 JSON 包进容器就能打开。
        let wrapped = write_container(&[
            ContainerEntry::new("project.json", json),
            ContainerEntry::new("history.dag", Vec::new()),
        ])
        .expect("写出容器");
        let wrapped_path = dir.join("wrapped.yeban");
        std::fs::write(&wrapped_path, &wrapped).expect("写容器");
        let opened = open_project_document_file(&wrapped_path, &ProjectOpenOptions::default())
            .expect("同样的 JSON 在容器里必须能打开");
        assert_eq!(opened.archive.project, project);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据 27: **截断的容器**必须保住它的容器裁决（精确错误码），
    /// 绝不被降级成"不是 `.yeban` 容器"、更不会被顺手当成 JSON 读。
    #[test]
    fn truncated_containers_keep_their_precise_container_verdict() {
        let (bytes, _) = container_bytes();
        let dir = std::env::temp_dir().join(format!("yeban-doc-trunc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("truncated.yeban");
        std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("写截断容器");

        match open_project_document_file(&path, &ProjectOpenOptions::default()) {
            Err(error @ OpenError::Container(_)) => {
                let text = error.to_string();
                assert!(text.contains("容器被拒绝"), "实测: {text}");
                assert!(error.container().is_some(), "必须借出精确裁决");
            }
            other => panic!("截断容器必须报精确容器裁决, 实测 {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据 28: **非容器输入**（垃圾 / 空 / 随机字节 / 裸坏 JSON）各自拿到
    /// "不是 `.yeban` 容器"这个**精确**错误，绝不"打开成空工程"；
    /// 而"有 ZIP 结构但不是合法容器"（多一个条目）走"容器被拒绝"那一档。
    #[test]
    fn non_container_input_is_rejected_precisely_without_an_empty_project() {
        let dir = std::env::temp_dir().join(format!("yeban-doc-junk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");

        // 四种"根本不是容器"的输入：空 / 垃圾文本 / 随机字节 / 看起来像 JSON 的坏 JSON。
        // 全部断言同一个精确出口，并且明确断言**不是** `Ok`（没有任何一条路会打开成空工程）。
        let cases: [(&str, &[u8]); 4] = [
            ("empty.bin", b""),
            ("junk.bin", b"not a zip at all"),
            ("random.bin", &[0xAB; 64]),
            ("broken.json", b"{ not json"),
        ];
        for (name, bytes) in cases {
            let path = dir.join(name);
            std::fs::write(&path, bytes).expect("写输入");
            match open_project_document_file(&path, &ProjectOpenOptions::default()) {
                Err(error @ OpenError::NotAYebanContainer { .. }) => {
                    assert_eq!(
                        error.container(),
                        Some(&ContainerError::EocdNotFound),
                        "{name}: 必须携带精确原裁决"
                    );
                    assert!(
                        error.to_string().contains("不是 `.yeban` 容器"),
                        "{name}: 实测 {error}"
                    );
                }
                other => panic!("{name} 必须报 `不是 .yeban 容器`, 实测 {other:?}"),
            }
        }

        // 有 ZIP 结构 ⇒ 归入"容器被拒绝"，不是"不是容器"（诊断分档，不是糊成一句）。
        let stray = yeban_model::container::write_container(&[
            yeban_model::container::ContainerEntry::new("project.json", b"{}".to_vec()),
            yeban_model::container::ContainerEntry::new("history.dag", b"x".to_vec()),
            yeban_model::container::ContainerEntry::new("notes.txt", b"y".to_vec()),
        ])
        .expect("写出容器");
        let stray_path = dir.join("stray.yeban");
        std::fs::write(&stray_path, &stray).expect("写容器");
        assert_eq!(
            match open_project_document_file(&stray_path, &ProjectOpenOptions::default()) {
                Err(OpenError::Container(error)) => Some(error),
                other => panic!("多余条目必须报容器裁决, 实测 {other:?}"),
            },
            Some(ContainerError::UnexpectedContainerEntry {
                name: "notes.txt".to_owned(),
            })
        );

        // 判据 ⑥：**几乎合法但缺件**的容器（只有 `history.dag`）⇒ 报错，**不是**空工程。
        let missing_project = yeban_model::container::write_container(&[
            yeban_model::container::ContainerEntry::new("history.dag", b"x".to_vec()),
        ])
        .expect("写出容器");
        let missing_path = dir.join("missing-project.yeban");
        std::fs::write(&missing_path, &missing_project).expect("写容器");
        let empty = YebanProjectV1::default();
        match open_project_document_file(&missing_path, &ProjectOpenOptions::default()) {
            Err(OpenError::Container(ContainerError::MissingProjectJson)) => {}
            other => panic!("缺 project.json 必须报 MissingProjectJson, 实测 {other:?}"),
        }
        assert_ne!(
            open_project_document_file(&missing_path, &ProjectOpenOptions::default())
                .ok()
                .map(|opened| opened.archive.project),
            Some(empty),
            "错误不得退化成空工程"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-428：分步打开与单调用入口**同一段**组合 —— 阶段按读盘 / 解码 / 核对
    /// 顺序报告，读到的字节数与工程逐字段一致，且与
    /// [`open_project_document_file`] 的结果逐位相同。
    #[test]
    fn stepwise_reports_phases_in_order_and_matches_the_single_call_entry() {
        let (bytes, project) = container_bytes();
        let dir = std::env::temp_dir().join(format!("yeban-open-stepwise-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("stepwise.yeban");
        std::fs::write(&path, &bytes).expect("写临时容器");

        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let mut seen: Vec<(OpenLoadPhase, u32, Option<u64>)> = Vec::new();
        let opened = open_project_document_file_stepwise(
            &path,
            &ProjectOpenOptions::default(),
            &cancelled,
            &mut |phase, fraction, file_bytes| {
                seen.push((phase, fraction.to_bits(), file_bytes));
            },
        )
        .expect("分步打开必须成功")
        .expect("未取消时分步必须返回 Some");
        assert_eq!(
            seen,
            vec![
                (OpenLoadPhase::Reading, 0.0f32.to_bits(), None),
                (
                    OpenLoadPhase::Reading,
                    0.3f32.to_bits(),
                    Some(bytes.len() as u64)
                ),
                (
                    OpenLoadPhase::Decoding,
                    0.55f32.to_bits(),
                    Some(bytes.len() as u64)
                ),
                (
                    OpenLoadPhase::Auditing,
                    0.85f32.to_bits(),
                    Some(bytes.len() as u64)
                ),
            ],
            "阶段必须按读盘 / 解码 / 核对顺序报告，锚点与字节数逐位一致"
        );
        assert_eq!(opened.archive.project, project, "工程文档必须逐字段一致");
        assert_eq!(opened.file_bytes, bytes.len() as u64);

        let single = open_project_document_file(&path, &ProjectOpenOptions::default())
            .expect("单调用入口必须打开同一份容器");
        assert_eq!(opened, single, "分步与单调用入口的结果必须逐位相同");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-428：读完之后、解码之前取消 ⇒ 返回 `Ok(None)`（⛔ 不是错误），
    /// 且解码 / 核对两个阶段**一个都没跑**。
    ///
    /// 红对照：删掉读盘后的取消检查点 ⇒ 本判据拿到 `Some` 而红。
    #[test]
    fn stepwise_cancel_between_read_and_decode_returns_none() {
        let (bytes, _) = container_bytes();
        let dir =
            std::env::temp_dir().join(format!("yeban-open-stepwise-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("cancel.yeban");
        std::fs::write(&path, &bytes).expect("写临时容器");

        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let mut seen: Vec<OpenLoadPhase> = Vec::new();
        let outcome = open_project_document_file_stepwise(
            &path,
            &ProjectOpenOptions::default(),
            &cancelled,
            &mut |phase, _, _| seen.push(phase),
        )
        .expect("取消不是错误，分步必须返回 Ok");
        assert_eq!(outcome, None, "预先取消 ⇒ 分步返回 None，不打开");
        assert_eq!(
            seen,
            vec![OpenLoadPhase::Reading, OpenLoadPhase::Reading],
            "取消在读盘后生效：解码 / 核对一个都没跑"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-FIX-428：解码开始时才取消 ⇒ 解码跑完、核对**没跑**，返回 `Ok(None)`。
    ///
    /// 红对照：删掉解码后的取消检查点 ⇒ 本判据拿到 `Some` 而红。
    #[test]
    fn stepwise_cancel_between_decode_and_audit_returns_none() {
        let (bytes, _) = container_bytes();
        let dir = std::env::temp_dir().join(format!(
            "yeban-open-stepwise-cancel2-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("cancel2.yeban");
        std::fs::write(&path, &bytes).expect("写临时容器");

        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let mut seen: Vec<OpenLoadPhase> = Vec::new();
        let outcome = open_project_document_file_stepwise(
            &path,
            &ProjectOpenOptions::default(),
            &cancelled,
            &mut |phase, _, _| {
                if phase == OpenLoadPhase::Decoding {
                    cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                seen.push(phase);
            },
        )
        .expect("取消不是错误，分步必须返回 Ok");
        assert_eq!(outcome, None, "解码后取消 ⇒ 分步返回 None，不打开");
        assert_eq!(
            seen,
            vec![
                OpenLoadPhase::Reading,
                OpenLoadPhase::Reading,
                OpenLoadPhase::Decoding
            ],
            "取消在解码后生效：核对没跑"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
