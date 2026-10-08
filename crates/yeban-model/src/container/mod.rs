//! `.yeban` 归档容器（规范 §5.3 / `ARCH-SEC-003`）—— 标准 ZIP 子集 + 两条 MUST 防御。
//!
//! # 规范来源
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3 `[ARCH-SEC-003]`：标准 ZIP 容器
//!   存储 `project.json`（`BTreeMap` 保证键序确定）、`history.dag`（版本提交树）、
//!   `assets/{sha256}`（CAS 资产池）；Zip-Slip 防御 (MUST)；解压炸弹防御 (MUST)。
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `[MUST-GATE-006]` / `[MUST-GATE-007]`。
//! - `docs/DEV_WORKFLOW.md` §3（需求 ID 闭环）与 `AGENTS.md` §2 红线 2/4/8。
//!
//! # 设计裁决（由集成者裁定，本模块照此执行）
//!
//! **不引入 `zip` / `flate2` 依赖。** 手写最小 ZIP 读写：
//!
//! - **只写 `stored`（method 0，不压缩）** 条目，但产出的是**标准 ZIP**
//!   （`unzip -l` / `unzip -p` 能读、能解出正确字节）。资产池里是已压缩的音频
//!   （FLAC/WAV），deflate 收益很小；换来的是 `yeban-model` **不新增任何第三方依赖**，
//!   于是本线的全部判据（含对抗性归档）都能在本机
//!   `bash scripts/gates/run-gates.sh crate yeban-model` 真跑，而不必等 CI。
//! - **读**到 `deflate`(method 8) 或任何其它压缩法一律 [`ContainerError::UnsupportedCompression`]，
//!   绝不静默跳过、也绝不猜测内容。
//!
//! # 两条 MUST 防御的落点
//!
//! | 门禁 | 实现 | 判据 |
//! | :--- | :--- | :--- |
//! | `MUST-GATE-006` Zip-Slip | [`normalize_entry_name`]：拒绝 `..` 段、绝对路径/盘符、`\`、`:`、NUL 与控制字符、空段、尾随点/空格、Windows 设备名、目录条目、符号链接条目；大小写折叠重名 | `tests/container_adversarial.rs` §Zip-Slip |
//! | `MUST-GATE-007` 解压炸弹 | [`ContainerLimits`] 四道上限（单条声明值、单条**实际**字节数、全归档实际字节数、整体膨胀比率）+ 条目数上限 | `tests/container_adversarial.rs` §炸弹 |
//!
//! **"实际写入量"是防谎报声明的关键**：限制判定既看 ZIP 里**声明**的 `uncompressed_size`
//! （fail-fast），也看读取器**真的materialize**的字节数（兜底）。谎报 `uncompressed = 1`
//! 的 2 MiB 条目会在"实际字节数"这一道被拦下（[`ContainerError::EntryActualTooLarge`]）。
//!
//! # 与 `[ARCH-SEC-004]` 的关系（本模块**不**做的事）
//!
//! `ARCH-SEC-004`（原子落盘：`.yeban.tmp-{ulid}` + `fsync` + 原子重命名）已由
//! `crates/yeban-mcp/src/domain/store.rs` 实现，属于 **I/O 层**责任。本模块是**纯字节层**：
//! 输入 `&[u8]`、输出 `Vec<u8>`，不碰文件系统、不碰 `unsafe`、不做权限判定。
//! 两者拼起来才是完整的"安全保存 + 安全加载"：本模块保证**字节流本身**不可越界，
//! `store.rs` 保证**落盘那一刻**不可撕裂。

mod crc32;
mod path;
mod zip;

pub use path::{MAX_ENTRY_NAME_BYTES, normalize_entry_name};

use std::collections::BTreeMap;

use thiserror::Error;

use crate::ids::AssetHash;
use crate::project::YebanProjectV1;

/// 工程文档在容器内的固定条目名（规范 §5.3 原文）。
pub const PROJECT_JSON_NAME: &str = "project.json";
/// 提交树在容器内的固定条目名（规范 §5.3 原文）。
pub const HISTORY_DAG_NAME: &str = "history.dag";
/// CAS 资产池在容器内的固定目录前缀（规范 §5.3 的 `assets/{sha256}`）。
pub const ASSETS_DIR: &str = "assets";
/// `assets/` 前缀（含分隔符），拼接与剥离都用它，避免两处字面量漂移。
const ASSETS_PREFIX: &str = "assets/";

/// 单条目解压后体积上限的默认值：2 GB。
///
/// 规范原文是"≤ 2 GB"。这里取**十进制 2×10⁹**而不是 2 GiB（2×2³⁰）：后者比 2 GB 宽 7.4%，
/// 会在"GB 到底怎么算"的争议里站到放宽的一侧。安全上限一律往**紧**的一侧取。
pub const DEFAULT_MAX_ENTRY_BYTES: u64 = 2_000_000_000;

/// 全归档解压后总体积上限的默认值：8 GB。
///
/// 规范只钉住单条目与比率，这是本实现**额外**加的一道（防"每条都合法、合起来耗尽内存"）。
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 8_000_000_000;

/// 整体压缩膨胀比率的默认上限：`100:1`（规范 §5.3 原文）。
pub const DEFAULT_MAX_RATIO: u64 = 100;

/// 条目数上限的默认值。
///
/// ZIP32 的结构上限是 65534（`0xFFFF` 是 ZIP64 哨兵），但真实 `.yeban` 工程的条目数
/// 是"2 + 资产数"。4096 足够大，又能拦住"用海量条目做元数据放大"。
pub const DEFAULT_MAX_ENTRIES: usize = 4096;

/// `MUST-GATE-007` 的四道可注入上限（外加条目数）。
///
/// **阈值必须可注入**：判据要在几十字节的数据上触发上限，不可能真去造一个 2 GB 的归档。
/// 生产调用点用 [`ContainerLimits::default`]，判据用结构体字面量把上限压到极小的值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerLimits {
    /// 单条目解压后体积上限（字节）。规范要求 ≤ 2 GB，默认 [`DEFAULT_MAX_ENTRY_BYTES`]。
    pub max_entry_bytes: u64,
    /// 全归档解压后总体积上限（字节），默认 [`DEFAULT_MAX_TOTAL_BYTES`]。
    pub max_total_bytes: u64,
    /// 整体压缩膨胀比率上限（`解压后 / 压缩后`），默认 [`DEFAULT_MAX_RATIO`]，即 `100:1`。
    pub max_ratio: u64,
    /// 条目数上限，默认 [`DEFAULT_MAX_ENTRIES`]。
    pub max_entries: usize,
}

impl Default for ContainerLimits {
    /// 规范 §5.3 的上限（`2 GB` / `100:1`）+ 本实现额外加的两道（总体积 / 条目数）。
    fn default() -> Self {
        Self {
            max_entry_bytes: DEFAULT_MAX_ENTRY_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_ratio: DEFAULT_MAX_RATIO,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

/// 容器内的一个条目：规范化后的名字 + 原始字节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEntry {
    /// 已通过 [`normalize_entry_name`] 的安全相对路径（`/` 分隔，无 `..`）。
    pub name: String,
    /// 条目内容。
    pub data: Vec<u8>,
}

impl ContainerEntry {
    /// 构造一个条目。
    ///
    /// 这里**不**做路径校验：校验发生在 [`write_container`] / [`read_container`]，
    /// 因为构造一个"待校验的原始条目"是合法的中间状态（对抗性判据正是这么造恶意归档的）。
    pub fn new(name: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
        }
    }
}

/// 读取成功后的容器视图。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContainerArchive {
    /// 按 central directory 顺序存放的条目。
    entries: Vec<ContainerEntry>,
}

impl ContainerArchive {
    /// 全部条目（central directory 顺序）。
    #[must_use]
    pub fn entries(&self) -> &[ContainerEntry] {
        &self.entries
    }

    /// 按名字取条目。
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ContainerEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// 条目数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空容器。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 全部条目名。
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }

    /// 取出全部条目。
    #[must_use]
    pub fn into_entries(self) -> Vec<ContainerEntry> {
        self.entries
    }
}

/// 读工程容器时的结构化结果。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectArchive {
    /// `project.json` 反序列化出的工程文档。
    pub project: YebanProjectV1,
    /// `history.dag` 的原始字节（本层**不**解读：编解码口径见
    /// [`crate::commit::encode_history_dag`] / [`crate::commit::decode_history_dag`]）。
    pub history_dag: Vec<u8>,
    /// `assets/{sha256}` 资产池，按哈希升序（= 容器内顺序 = `BTreeMap` 顺序）。
    pub assets: Vec<(AssetHash, Vec<u8>)>,
}

/// `.yeban` 容器层的错误类型。
///
/// 全部变体都**只**描述"为什么拒绝"，不带任何平台相关细节：同一条恶意归档在三个平台上
/// 必须得到同一个错误码（这是判据能跨机复现的前提）。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContainerError {
    // ------------------------------------------------------------------
    // MUST-GATE-006：路径安全
    // ------------------------------------------------------------------
    /// 条目名为空。
    #[error("entry name is empty")]
    EmptyEntryName,
    /// 条目名超过字节上限。
    #[error("entry name is {len} bytes, over the {max}-byte limit")]
    EntryNameTooLong {
        /// 实际长度（字节）。
        len: usize,
        /// 上限（字节）。
        max: usize,
    },
    /// 条目名是绝对路径（以 `/` 开头，含 `//server/share` 形式的 UNC 路径）。
    #[error("entry name `{name}` is an absolute path")]
    AbsoluteEntryPath {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名含 `..` 段（Zip-Slip 本体）。
    #[error("entry name `{name}` contains a `..` parent-directory segment")]
    ParentDirSegment {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名含 `.` 段（规范化歧义：不同解包器折叠规则不同）。
    #[error("entry name `{name}` contains a `.` segment")]
    CurrentDirSegment {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名含空段（`a//b`、`/a`、`a/`）。
    #[error("entry name `{name}` contains an empty path segment")]
    EmptyPathSegment {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名以 `/` 结尾（目录条目）。`.yeban` 容器只存文件。
    #[error("entry name `{name}` is a directory entry; the container stores files only")]
    DirectoryEntryUnsupported {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名含反斜杠（Windows 上它是路径分隔符）。
    #[error("entry name `{name}` contains a backslash")]
    BackslashInEntryName {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名含冒号（`C:\` / `C:rel` 盘符，或 NTFS 数据流 `file:stream`）。
    #[error("entry name `{name}` contains a colon (drive letter or NTFS data stream)")]
    ColonInEntryName {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名含 NUL 字节。
    #[error("entry name `{name}` contains a NUL byte")]
    NulInEntryName {
        /// 被拒绝的原始名字（调试形式）。
        name: String,
    },
    /// 条目名含 C0 控制字符或 DEL。
    #[error("entry name `{name}` contains a control character")]
    ControlCharInEntryName {
        /// 被拒绝的原始名字（调试形式）。
        name: String,
    },
    /// 条目名的某个段以 `.` 或空格结尾（Windows 会剥离它们 ⇒ `".. "` 等价于 `..`）。
    #[error("entry name `{name}` has a segment ending in `.` or space")]
    TrailingDotOrSpaceSegment {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名的某个段是 Windows 保留设备名（`CON` / `NUL` / `COM1` …）。
    #[error("entry name `{name}` uses a reserved Windows device name")]
    WindowsReservedName {
        /// 被拒绝的原始名字。
        name: String,
    },
    /// 条目名与另一条目重复（大小写折叠后）。会互相覆盖的归档一律拒绝。
    #[error("entry name `{name}` appears more than once (case-insensitive)")]
    DuplicateEntryName {
        /// 重复的名字。
        name: String,
    },
    /// 条目被声明为 Unix 符号链接（`ARCH-SEC-003` 的"跨卷符号链接"）。
    #[error("entry #{index} `{name}` is a symbolic link; the container stores regular files only")]
    SymlinkEntryUnsupported {
        /// central directory 中的序号。
        index: usize,
        /// 条目名。
        name: String,
    },

    // ------------------------------------------------------------------
    // MUST-GATE-007：解压炸弹
    // ------------------------------------------------------------------
    /// 条目**声明**的解压体积超过 `max_entry_bytes`。
    #[error("entry declares {declared} uncompressed bytes, over the {max}-byte per-entry limit")]
    EntryTooLarge {
        /// 声明的解压体积。
        declared: u64,
        /// 上限。
        max: u64,
    },
    /// 条目**实际**需要读取的字节数超过 `max_entry_bytes`（声明撒谎时的兜底）。
    #[error("entry actually materializes {actual} bytes, over the {max}-byte per-entry limit")]
    EntryActualTooLarge {
        /// 实际字节数。
        actual: u64,
        /// 上限。
        max: u64,
    },
    /// 全归档**实际**累计体积超过 `max_total_bytes`。
    #[error("archive actually materializes {actual} bytes in total, over the {max}-byte limit")]
    ArchiveTooLarge {
        /// 累计实际字节数。
        actual: u64,
        /// 上限。
        max: u64,
    },
    /// 整体压缩膨胀比率超过 `max_ratio`。
    #[error(
        "expansion ratio {uncompressed}:{compressed} exceeds the {max_ratio}:1 limit \
         (uncompressed {uncompressed} bytes / compressed {compressed} bytes)"
    )]
    ExpansionRatioExceeded {
        /// 累计声明的解压体积。
        uncompressed: u64,
        /// 累计压缩后体积。
        compressed: u64,
        /// 比率上限。
        max_ratio: u64,
    },
    /// 条目数超过 `max_entries`。
    #[error("archive has {found} entries, over the {max}-entry limit")]
    TooManyEntries {
        /// 实际条目数。
        found: usize,
        /// 上限。
        max: usize,
    },

    // ------------------------------------------------------------------
    // 结构畸形
    // ------------------------------------------------------------------
    /// 找不到合法的 EOCD。
    #[error("end-of-central-directory record not found")]
    EocdNotFound,
    /// 读越界（某一结构的长度声明超出了输入字节流）。
    #[error("archive is truncated")]
    TruncatedArchive,
    /// 多卷 / 跨盘归档（`.yeban` 必须是单文件）。
    #[error("multi-disk archives are not supported")]
    UnsupportedMultiDisk,
    /// ZIP64 结构（`.yeban` 单条目上限 2 GB ⇒ ZIP32 足够）。
    #[error("ZIP64 archives are not supported")]
    UnsupportedZip64,
    /// 不支持的压缩法（含 `deflate`）。明确报错，绝不静默跳过。
    #[error("entry #{index} uses compression method {method}; only stored (0) is supported")]
    UnsupportedCompression {
        /// central directory 中的序号。
        index: usize,
        /// 实际的压缩法编号。
        method: u16,
    },
    /// 条目使用 data descriptor（尺寸写在数据之后）。
    #[error("entry #{index} uses a data descriptor")]
    UnsupportedDataDescriptor {
        /// central directory 中的序号。
        index: usize,
    },
    /// 条目已加密。
    #[error("entry #{index} `{name}` is encrypted")]
    EncryptedEntryUnsupported {
        /// central directory 中的序号。
        index: usize,
        /// 条目名。
        name: String,
    },
    /// central directory 的偏移/长度越界（或落在 EOCD 之后）。
    #[error("central directory offset/length is out of bounds")]
    CentralDirectoryOutOfBounds,
    /// central directory 的声明长度与实际解析消耗的字节数不符。
    #[error("central directory declares {declared} bytes but {actual} bytes were parsed")]
    CentralDirectorySizeMismatch {
        /// EOCD 声明的长度。
        declared: usize,
        /// 实际解析消耗的长度。
        actual: usize,
    },
    /// central directory 在某条记录中间就结束了。
    #[error("central directory record #{index} is truncated")]
    TruncatedCentralDirectory {
        /// central directory 中的序号。
        index: usize,
    },
    /// central directory 记录签名不对。
    #[error("central directory record #{index} has signature {found:#010X}")]
    BadCentralDirectorySignature {
        /// central directory 中的序号。
        index: usize,
        /// 实际读到的签名。
        found: u32,
    },
    /// 条目名不是合法 UTF-8（CP437 名字本实现不解码）。
    #[error("entry #{index} has a non-UTF-8 name")]
    EntryNameNotUtf8 {
        /// central directory 中的序号。
        index: usize,
    },
    /// local file header 的偏移越界。
    #[error("local file header of entry #{index} is out of bounds")]
    LocalHeaderOutOfBounds {
        /// central directory 中的序号。
        index: usize,
    },
    /// local file header 签名不对。
    #[error("local file header of entry #{index} has signature {found:#010X}")]
    BadLocalHeaderSignature {
        /// central directory 中的序号。
        index: usize,
        /// 实际读到的签名。
        found: u32,
    },
    /// local file header 与 central directory 对同一字段给出不同值。
    ///
    /// ZIP 的历史攻击面：不同解包器"以谁为准"不同 ⇒ 同一归档读出不同文件。
    /// 本实现的裁决是**两处都必须一致**，不一致即拒绝。
    #[error("entry #{index} `{name}`: local file header disagrees with the central directory")]
    LocalCentralMismatch {
        /// central directory 中的序号。
        index: usize,
        /// 条目名。
        name: String,
    },
    /// 条目数据区越界。
    #[error("data of entry #{index} is truncated")]
    TruncatedEntryData {
        /// central directory 中的序号。
        index: usize,
    },
    /// `stored` 条目的压缩前后尺寸不一致（声明自相矛盾）。
    #[error(
        "entry #{index} `{name}` is stored but declares compressed {compressed} != \
         uncompressed {uncompressed}"
    )]
    StoredSizeMismatch {
        /// central directory 中的序号。
        index: usize,
        /// 条目名。
        name: String,
        /// 声明的压缩后大小。
        compressed: u32,
        /// 声明的解压后大小。
        uncompressed: u32,
    },
    /// CRC-32 校验失败（内容被篡改或损坏）。
    #[error(
        "entry #{index} `{name}` CRC mismatch: header says {declared:#010X}, data is {actual:#010X}"
    )]
    CrcMismatch {
        /// central directory 中的序号。
        index: usize,
        /// 条目名。
        name: String,
        /// header 声明的 CRC-32。
        declared: u32,
        /// 实际算出的 CRC-32。
        actual: u32,
    },

    // ------------------------------------------------------------------
    // 容器内容布局（§5.3 的三种条目）
    // ------------------------------------------------------------------
    /// 缺少 `project.json`。
    #[error("container has no `project.json` entry")]
    MissingProjectJson,
    /// 缺少 `history.dag`。
    #[error("container has no `history.dag` entry")]
    MissingHistoryDag,
    /// `project.json` 不是合法的工程文档。
    #[error("`project.json` is not a valid YebanProjectV1: {detail}")]
    InvalidProjectJson {
        /// `serde_json` 的错误描述。
        detail: String,
    },
    /// 容器里出现了 §5.3 未定义的条目。
    #[error("container has an unexpected entry `{name}`")]
    UnexpectedContainerEntry {
        /// 条目名。
        name: String,
    },
    /// `assets/` 下的条目名不是 64 位小写十六进制 SHA-256。
    #[error("asset entry `{name}` does not carry a canonical SHA-256 digest")]
    InvalidAssetName {
        /// 条目名。
        name: String,
    },
    /// 资产字节的 SHA-256 与条目名（CAS 键）不符。
    #[error("asset `{declared}` hashes to {actual} instead")]
    AssetHashMismatch {
        /// 条目名携带的（或调用方声明的）摘要。
        declared: String,
        /// 对实际字节算出的摘要。
        actual: String,
    },
    /// 序列化工程文档失败。
    #[error("failed to serialize the project document: {detail}")]
    ContainerSerialization {
        /// `serde_json` 的错误描述。
        detail: String,
    },
}

/// `assets/{sha256}` 的条目名（规范 §5.3 的 CAS 布局）。
#[must_use]
pub fn asset_entry_name(hash: &AssetHash) -> String {
    format!("{ASSETS_PREFIX}{hash}")
}

/// 把条目序列写成标准 ZIP 字节流（method 0 `stored`）。
///
/// # 与任务书的签名差异（有意为之）
///
/// 任务书建议 `write_container(entries) -> Vec<u8>`。这里多一个 `Result`：写入口
/// **同样**做路径与重复名校验，否则我们能写出自己读不回来的容器
/// （"写得出、读不回"就是最坏的一种失配，必须让它变成编译期可见的 `Result`）。
///
/// # Errors
///
/// 条目名不安全（见 [`normalize_entry_name`]）、大小写折叠后重名、条目数或体积超出 ZIP32 上限。
pub fn write_container(entries: &[ContainerEntry]) -> Result<Vec<u8>, ContainerError> {
    zip::write_zip(entries)
}

/// 解析并校验 `.yeban` 容器，应用 `MUST-GATE-006` / `MUST-GATE-007` 两条防御。
///
/// 判定顺序（**顺序本身是契约的一部分**，判据断言精确错误码）见 `src/container/zip.rs`
/// 中 `read_zip` 的文档注释（私有模块，不经 rustdoc 暴露）。
///
/// # Errors
///
/// 路径穿越、解压炸弹、结构畸形、CRC 不匹配等任一命中时返回对应的 [`ContainerError`]。
pub fn read_container(
    bytes: &[u8],
    limits: &ContainerLimits,
) -> Result<ContainerArchive, ContainerError> {
    let entries = zip::read_zip(bytes, limits)?;
    Ok(ContainerArchive { entries })
}

/// 按 §5.3 的布局写出一个工程容器：`project.json` + `history.dag` + `assets/{sha256}`。
///
/// 资产以调用方给的 `BTreeMap` 顺序写出（**键序确定**，[`crate::project::YebanProjectV1`]
/// 的 `assets` 索引也是 `BTreeMap`）⇒ 同一工程内容两次写出的字节完全相同
/// （`ARCH-DET-001`，由判据 `project_container_ignores_btreemap_insertion_order` 钉住）。
///
/// 本函数是 [`write_project_container_borrowed`] 的适配层：它只借用 `BTreeMap` 里的
/// 资产字节 ⇒ 写出路径**不再为任何一条资产复制一份 `Vec<u8>`**。
///
/// # Errors
///
/// - 工程文档序列化失败（[`ContainerError::ContainerSerialization`]）；
/// - 资产字节的 SHA-256 与其 CAS 键不符（[`ContainerError::AssetHashMismatch`]）；
/// - 以及 [`write_container`] 的全部错误。
pub fn write_project_container(
    project: &YebanProjectV1,
    history_dag: &[u8],
    assets: &BTreeMap<AssetHash, Vec<u8>>,
) -> Result<Vec<u8>, ContainerError> {
    let borrowed: Vec<(&AssetHash, &[u8])> = assets
        .iter()
        .map(|(hash, data)| (hash, data.as_slice()))
        .collect();
    write_project_container_borrowed(project, history_dag, &borrowed)
}

/// 按 §5.3 的布局写出工程容器，资产以**借用**的 `(哈希, 字节)` 切片给出。
///
/// 存在的理由（`docs/ledger/app-cli-notes.md` §8 needs-2）：调用方通常已经持有
/// `Vec<(AssetHash, Vec<u8>)>`（[`ProjectArchive::assets`] 就是这个形状）。把它重建成
/// [`write_project_container`] 要的 `BTreeMap` 会对**每一条资产**多复制一份字节；
/// 本入口不要求所有权 ⇒ 除最终归档缓冲本身之外，写出路径不再复制资产内容。
///
/// 顺序：资产按哈希**升序**写出（与 `BTreeMap` 的迭代顺序一致）⇒
/// **输入切片本身的顺序不影响归档字节**（`ARCH-DET-001`）。
///
/// # Errors
///
/// - 工程文档序列化失败（[`ContainerError::ContainerSerialization`]）；
/// - 资产字节的 SHA-256 与其 CAS 键不符（[`ContainerError::AssetHashMismatch`]）；
/// - 两个资产的哈希相同（[`ContainerError::DuplicateEntryName`] —— 同名条目由
///   [`write_container`] 拒绝，与 `BTreeMap` 形态"键不可能重复"的语义对齐）；
/// - 以及 [`write_container`] 的全部错误。
pub fn write_project_container_borrowed(
    project: &YebanProjectV1,
    history_dag: &[u8],
    assets: &[(&AssetHash, &[u8])],
) -> Result<Vec<u8>, ContainerError> {
    let json =
        serde_json::to_vec(project).map_err(|error| ContainerError::ContainerSerialization {
            detail: error.to_string(),
        })?;

    // 按哈希升序（= `BTreeMap` 键序 = 归档确定性）。排序只搬运引用，不碰资产字节。
    let mut ordered: Vec<(&AssetHash, &[u8])> = assets.to_vec();
    ordered.sort_by(|left, right| left.0.cmp(right.0));

    let mut names: Vec<String> = Vec::with_capacity(ordered.len());
    for &(hash, data) in &ordered {
        let actual = AssetHash::of_bytes(data);
        if &actual != hash {
            return Err(ContainerError::AssetHashMismatch {
                declared: hash.to_string(),
                actual: actual.to_string(),
            });
        }
        names.push(asset_entry_name(hash));
    }

    let mut entries: Vec<(&str, &[u8])> = Vec::with_capacity(2 + ordered.len());
    entries.push((PROJECT_JSON_NAME, json.as_slice()));
    entries.push((HISTORY_DAG_NAME, history_dag));
    for (index, &(_, data)) in ordered.iter().enumerate() {
        entries.push((names[index].as_str(), data));
    }
    zip::write_zip_borrowed(&entries)
}

/// 读取一个按 §5.3 布局写出的工程容器，并校验 CAS 完整性。
///
/// # Errors
///
/// - 缺少 `project.json` / `history.dag`（[`ContainerError::MissingProjectJson`] /
///   [`ContainerError::MissingHistoryDag`]）；
/// - 出现 §5.3 未定义的条目（[`ContainerError::UnexpectedContainerEntry`]）；
/// - 资产名不是规范 SHA-256（[`ContainerError::InvalidAssetName`]）或内容哈希不符
///   （[`ContainerError::AssetHashMismatch`]）；
/// - `project.json` 反序列化失败（[`ContainerError::InvalidProjectJson`]）；
/// - 以及 [`read_container`] 的全部错误（两条 MUST 防御先于内容解读生效）。
pub fn read_project_container(
    bytes: &[u8],
    limits: &ContainerLimits,
) -> Result<ProjectArchive, ContainerError> {
    let archive = read_container(bytes, limits)?;
    let mut project_json: Option<&[u8]> = None;
    let mut history_dag: Option<Vec<u8>> = None;
    let mut assets: Vec<(AssetHash, Vec<u8>)> = Vec::new();
    for entry in archive.entries() {
        if entry.name == PROJECT_JSON_NAME {
            project_json = Some(&entry.data);
        } else if entry.name == HISTORY_DAG_NAME {
            history_dag = Some(entry.data.clone());
        } else if let Some(digest) = entry.name.strip_prefix(ASSETS_PREFIX) {
            let hash = AssetHash::parse(digest).map_err(|_| ContainerError::InvalidAssetName {
                name: entry.name.clone(),
            })?;
            let actual = AssetHash::of_bytes(&entry.data);
            if actual != hash {
                return Err(ContainerError::AssetHashMismatch {
                    declared: hash.to_string(),
                    actual: actual.to_string(),
                });
            }
            assets.push((hash, entry.data.clone()));
        } else {
            return Err(ContainerError::UnexpectedContainerEntry {
                name: entry.name.clone(),
            });
        }
    }
    let project = serde_json::from_slice(project_json.ok_or(ContainerError::MissingProjectJson)?)
        .map_err(|error| ContainerError::InvalidProjectJson {
        detail: error.to_string(),
    })?;
    Ok(ProjectArchive {
        project,
        history_dag: history_dag.ok_or(ContainerError::MissingHistoryDag)?,
        assets,
    })
}
