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
//! 本线只交付"可被调用的入口"。把它接进 CLI（例如 `--open <path>`）属于**控制面 / 事件循环**
//! 的独立裁决（红线 6 + live-port 线对 `main.rs` 的处置），已登记为 needs。

use std::fmt;
use std::path::{Path, PathBuf};

use yeban_model::container::{
    ContainerError, ContainerLimits, ProjectArchive, read_project_container,
};
use yeban_model::project::YebanProjectV1;

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
}

impl Default for ProjectOpenOptions {
    /// 生产口径：容器默认上限 + [`MAX_PROJECT_FILE_BYTES`]。
    fn default() -> Self {
        Self {
            limits: ContainerLimits::default(),
            max_file_bytes: MAX_PROJECT_FILE_BYTES,
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
    /// 容器层拒绝：**原样**携带 `yeban-model` 的裁决。
    Container(ContainerError),
}

impl OpenError {
    /// 如果这次失败是**容器层**的裁决，借出它的精确变体（压缩法 / ZIP64 / 炸弹 / 路径 …）。
    ///
    /// 存在的意义：调用方要能对"容器拒绝"做**精确**分支（而不是匹配一个字符串），
    /// 判据也要能比较精确错误码 —— [`OpenError`] 本身不能 `PartialEq`（`io::Error` 不是），
    /// 但 [`ContainerError`] 是，于是比较走这个访问器。
    #[must_use]
    pub const fn container(&self) -> Option<&ContainerError> {
        match self {
            Self::Container(error) => Some(error),
            Self::Io { .. } | Self::FileTooLarge { .. } => None,
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
            Self::Container(error) => write!(formatter, "容器被拒绝: {error}"),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Container(error) => Some(error),
            Self::FileTooLarge { .. } => None,
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
    Ok(read_project_container(bytes, limits)?)
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
    let metadata = std::fs::metadata(path).map_err(|source| OpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > options.max_file_bytes {
        return Err(OpenError::FileTooLarge {
            path: path.to_path_buf(),
            len: metadata.len(),
            max: options.max_file_bytes,
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
    if len > options.max_file_bytes {
        return Err(OpenError::FileTooLarge {
            path: path.to_path_buf(),
            len,
            max: options.max_file_bytes,
        });
    }
    open_project_archive(&bytes, &options.limits)
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

    /// 造一个真容器（`project.json` + `history.dag` + 一个 CAS 资产）。
    fn container_bytes() -> (Vec<u8>, YebanProjectV1) {
        let project = demo_project();
        let asset = b"yeban-open-test-asset".to_vec();
        let hash = AssetHash::of_bytes(&asset);
        let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
        assets.insert(hash, asset);
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
        assert_eq!(archive.assets[0].1, b"yeban-open-test-asset");

        let only_project = open_project_bytes(&bytes, &limits).expect("只要工程的形态");
        assert_eq!(only_project, project);
        assert_eq!(only_project.title, "夜半 Yeban");
        assert!(!only_project.tracks.is_empty());
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

        // (f) 膨胀比率：把 `project.json` **声明**的解压体积改成 1 MB（压缩后只有几百字节）
        //     ⇒ `ExpansionRatioExceeded`（fail-fast 用声明值，见 zip.rs 的判定顺序注释）。
        let mut ratio_bomb = bytes.clone();
        let central = central_record(&ratio_bomb, 0);
        let compressed = u32_at(&ratio_bomb, central + 20);
        patch_u32(&mut ratio_bomb, central + 24, 1_000_000);
        match container_err(open_project_bytes(&ratio_bomb, &limits)) {
            Some(ContainerError::ExpansionRatioExceeded {
                uncompressed,
                compressed: seen,
                max_ratio,
            }) => {
                assert_eq!(uncompressed, 1_000_000);
                assert_eq!(seen, u64::from(compressed));
                assert_eq!(max_ratio, 100);
            }
            other => panic!("膨胀比率必须报 ExpansionRatioExceeded, 实测 {other:?}"),
        }

        // (g) ZIP64 哨兵：把 `uncompressed` 改成 `0xFFFFFFFF` ⇒ 明确拒绝（不支持 ZIP64）。
        let mut zip64 = bytes.clone();
        let central = central_record(&zip64, 0);
        patch_u32(&mut zip64, central + 24, u32::MAX);
        assert_eq!(
            container_err(open_project_bytes(&zip64, &limits)),
            Some(ContainerError::UnsupportedZip64)
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
}
