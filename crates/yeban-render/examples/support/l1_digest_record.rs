//! # `MUST-GATE-002` 的 **L1 摘要记录**（可存档、可跨机对照的产物）
//!
//! ## 为什么还需要它（根因）
//!
//! `MUST-GATE-003` 的收据（`examples/support/l1_receipt.rs`）证明的是"**一次进程内**的
//! 渲染是确定的"：它携带全部样本位型，因此可以在**两份同时存在的收据**之间做逐样本比较。
//! 但它回答不了 `MUST-GATE-002` 真正问的那个问题：
//!
//! > 换一台同平台机器、同一个种子，**WAV 文件的 SHA-256 是否一模一样**？
//!
//! 因为收据的时间尺度是"一次 `cargo test` / 一次 CI job"。**跨机器**要求的是
//! **一条可以被记录、存档、并在另一台机器上重新比对的基准**。本模块定义那条基准 ——
//! 它的字段、它的**归一化口径**（哪些字段参与比对、哪些只做记录）、以及判决规则。
//!
//! ## 摘要口径（`digest_scope` / `digest_input` / `wav_encoding`）
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:378`）要求的是
//! "**WAV 文件 SHA-256 哈希**"。但"WAV 文件的哈希"在容器层是不稳定的 —— `bext` 里的
//! 起始时间戳、RIFF 块顺序、填充字节、`LIST/INFO` 里的软件名都会改变文件字节，而它们
//! **与声学无关**。因此本模块把哈希的**输入**与**容器**分开写明，不许含糊：
//!
//! | 字段 | 取值 | 含义 |
//! | :--- | :--- | :--- |
//! | `digest_input` | `canonical-pcm-bits` | 摘要覆盖的字节流 = **交错 PCM 有效位**（见下），逐流确定 |
//! | `digest_scope` | `pcm-payload-only` \| `whole-file` | 上面那串字节**就是** WAV 的 `data` 载荷（编码无损时）／就是整个文件 |
//! | `sample_format` | `f32-le` | 每个样本 4 字节小端 IEEE-754 位型 |
//! | `wav_encoding` | `pcm-f32-le` | 容器里的编码；满足"无损"条件，才允许 `pcm-payload-only` |
//! | `wav_bytes` | 字节数 | 存档证据（可由 `digest_input` + 固定头布局复算） |
//!
//! **`pcm-payload-only` 是允许的，但必须自证无损**：当 `sample_format` 是 `f32-le` 且
//! `wav_encoding` 是 `pcm-f32-le` 时，`data` 块载荷**逐字节等于**位型串接，于是
//! "PCM 载荷的 SHA-256"与"整个无损 WAV 文件的 SHA-256"是同一个数。`null` 样本哨兵值
//! 只可能出现在 `pcm-f32-le` 里，因此"哨兵恒不出现"不需要额外条件。
//!
//! 任何**有损**容器（`pcm-s16-le` / `pcm-s24-le` / `pcm-s32-le` / `float64` …）都
//! **必须**用 `whole-file`：那时 `data` 载荷不是位型串接，用载荷哈希会静默丢掉量化信息。
//! [`DigestRecord::validate`] 把这条规则做成硬判据（见 `validate-wav-lossless-rule`）。
//!
//! ## 归一化口径：哪些字段参与比对、哪些只做记录
//!
//! 这是本模块存在的一半理由。它是一张**显式的表**（[`FIELD_TABLE`]），不是文档里的散文，
//! 因为判据要**注入**一个非参与字段的变化并断言"仍然绿"（`ARCH-DET-*` 的"参与/仅记录"
//! 必须可被机器检验，而不是靠人读）。
//!
//! | 字段 | 参与比对 | 理由 |
//! | :--- | :--- | :--- |
//! | `schema` | 否 | 兼容性检查，不是读数 |
//! | `fixture` | 是 | 换了工程就没有"同一个种子渲染"可谈 |
//! | `seed` / `sample_rate` / `frames` / `channels` / `tracks` / `block_size` / `gain_db` / `latency` | 是 | 渲染参数：任一不同 ⇒ 两份摘要不可比 |
//! | `target_arch` / `target_os` / `target_triple` / `target_env` / `target_endian` / `target_pointer_width` | 是 | **平台/ISA 身份**；这是"同平台"的定义所在 |
//! | `rustc_release` | 是 | **锁定工具链**：发布号是锁的钥匙 |
//! | `rustc_host` | 是 | 主机三元组；与 `target_triple` 一起构成"同工具链" |
//! | `isa_features` | 是 | 基准指令集（如 `+fma`）；指令选择直接改变浮点结果 |
//! | `digest_algorithm` / `digest_input` / `digest_scope` / `sample_format` / `wav_encoding` / `wav_bytes` | 是 | 口径本身不同 ⇒ 两个数不可比（比的是口径，不是"差不多"） |
//! | `digest` | 是 | **被比对的读数** |
//! | `sample_digest` | 是 | 独立交叉校验（`digest` 是它的函数） |
//! | `rustc_version` / `rustc_commit` / `rustc_commit_date` | 否 | **仅记录**：同一发布号的构建元数据会随 runner 镜像漂移（实测：本机与 CI 的 `rustc -vV` 逐字不同），而它**不**改变本仓库渲染路径的浮点结果 |
//! | `host_name` / `host_os_version` / `generated_at_utc` / `notes` | 否 | **仅记录**：宿主名、时间戳、路径 —— 刻意让它们可自由变化，判据 ⑥ 据此证明"真的不参与" |
//!
//! ## 判决：三种结论，**箭头不许含糊**
//!
//! | 结论 | 退出码 | 何时 |
//! | :--- | :--- | :--- |
//! | [`Verdict::Pass`] | `0` | 平台同 + 工具链锁同 + 全部参与字段逐字段相同 ⇒ **SHA-256 全同** |
//! | [`Verdict::Fail`] | `1` | **平台相同（同 ISA + 同 OS）、工具链锁相同、参与字段却不同 ⇒ 硬红** |
//! | [`Verdict::Skip`] | `2` | 平台不同（跨 ISA / 跨 OS）⇒ **不可比**，如实 SKIP 并打印原因 |
//!
//! **两条最容易撒谎的地方，本模块用类型把它们堵掉**：
//!
//! 1. **不许把"跳过"写成"通过"**：`Skip` 有自己的 `Verdict` 变体与退出码 `2`，
//!    报告行是 `VERDICT SKIP` + `reason=…`，**不是** `VERDICT PASS`；
//! 2. **不许把"平台不同"当成"渲染不确定"**：跨 ISA 的哈希差异是**预期**的（`MUST-GATE-003`
//!    的场景），因此它是 SKIP 而不是 FAIL。反过来，**同平台**下的差异没有借口 ⇒ FAIL。
//!
//! 工具链锁不同（同平台、发布号不同）走 **SKIP**，`reason=toolchain-not-locked`：
//! 规范要求的是"**锁定**工具链下"的确定性，未锁定的两份读数说明不了任何事 ——
//! 但它是**与平台不匹配不同的原因码**，报告里一眼可辨（不许混为一谈）。
//!
//! ## 本模块零第三方依赖
//!
//! [`sha256`] 是本文件自带的一份实现（`sha2` 是 `yeban-render` 的依赖，但本模块刻意
//! 不引用任何 crate），WAV 编码器也自足。因此它可以脱离 `rayon`/`hound` 用
//! `rustc --edition 2024 --test` 在本机真跑：见 `examples/support/l1_digest_record_tests.rs`。
//! 真实渲染侧的端到端判据在 `tests/l1_digest_parity.rs`。

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// 摘要记录的 schema 标识（第一个字段）。
pub const SCHEMA: &str = "yeban-l1-digest/1";

/// **本模块认识的** schema 列表。
///
/// ⚠ 与 [`DigestRecord::validate`] 的分工：解析器**只认列表里的版本**（不认识的版本无法
/// 逐字段读懂，"猜着解析"必然出错）；而自洽性检查**不**要求 schema 等于当前版本，
/// 否则一份未来版本的记录连"不可比"这个结论都说不出口（那正是本仓库最忌讳的假红）。
pub const SUPPORTED_SCHEMAS: &[&str] = &[SCHEMA];

/// 固定的 WAV 容器字节数：`RIFF(12) + fmt (8+16) + fact(8+4) + data(8)`。
///
/// `data` 载荷长度 = `frames * channels * 4`（`f32-le`）。这个头布局是**规范的一部分**：
/// 改动它就会改变 `whole-file` 口径下的 `wav_bytes`，因此必须与 schema 一起升版本。
pub const WAV_HEADER_BYTES: u64 = 12 + 24 + 12 + 8;

/// 每个 `f32` 样本在 `f32-le` 口径下占的字节数。
pub const SAMPLE_BYTES: u64 = 4;

/// 生成器写入的 `digest_algorithm`。
pub const DIGEST_ALGORITHM: &str = "sha256";

/// 生成器写入的 `digest_input`。
pub const DIGEST_INPUT: &str = "canonical-pcm-bits";

/// 生成器写入的 `sample_format`。
pub const SAMPLE_FORMAT: &str = "f32-le";

/// 生成器写入的 `wav_encoding`。
pub const WAV_ENCODING: &str = "pcm-f32-le";

/// 写一行到 `String`；写 `String` 不会失败，因此这里的 `expect` 不可能触发。
macro_rules! emitln {
    ($out:expr, $($arg:tt)*) => {{
        writeln!($out, $($arg)*).expect("写入 String 不会失败")
    }};
}

/// 写一段（**不**换行）到 `String`。
macro_rules! emit {
    ($out:expr, $($arg:tt)*) => {{
        write!($out, $($arg)*).expect("写入 String 不会失败")
    }};
}

/// 字段的参与身份。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Participation {
    /// 参与比对：两份摘要不同 ⇒ 不可比（不是"差异报告"）。
    Compared,
    /// 只做记录：宿主名 / 时间戳 / 路径 / 构建元数据。它变了**仍然应当绿**。
    RecordedOnly,
}

impl Participation {
    /// 报告里的记号。
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Compared => "compared",
            Self::RecordedOnly => "recorded-only",
        }
    }
}

/// 一个摘要字段的（JSON 路径，参与身份）描述。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// JSON 里的字段名（本记录的字段是平的，没有嵌套）。
    pub name: &'static str,
    /// 参与比对还是只做记录。
    pub participation: Participation,
}

/// **归一化口径的单一事实源**：摘要有哪些字段、哪些参与比对。
///
/// 顺序即文档顺序（与提交进仓库的参考摘要的字段顺序一致，便于人工 diff）。
pub const FIELD_TABLE: &[FieldSpec] = &[
    FieldSpec {
        name: "schema",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "fixture",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "seed",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "sample_rate",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "channels",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "frames",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "tracks",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "block_size",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "gain_db",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "latency",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "threads",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "target_arch",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "target_os",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "target_env",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "target_endian",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "target_pointer_width",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "target_triple",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "rustc_release",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "rustc_host",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "isa_features",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "digest_algorithm",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "digest_input",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "sample_format",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "wav_encoding",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "digest_scope",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "wav_bytes",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "digest",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "sample_digest",
        participation: Participation::Compared,
    },
    FieldSpec {
        name: "rustc_version",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "rustc_commit",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "rustc_commit_date",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "host_name",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "host_os_version",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "generated_at_utc",
        participation: Participation::RecordedOnly,
    },
    FieldSpec {
        name: "notes",
        participation: Participation::RecordedOnly,
    },
];

/// 按字段名查它的参与身份；未知字段返回 `None`。
#[must_use]
pub fn participation_of(name: &str) -> Option<Participation> {
    FIELD_TABLE
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.participation)
}

/// 摘要覆盖的字节流（决定 `digest` 到底哈希了什么）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestScope {
    /// 只哈希 WAV 的 `data` 块载荷（`digest_input = canonical-pcm-bits`）。
    ///
    /// 仅当编码**无损**时可接受（见模块文档的"自证无损"）。
    PcmPayloadOnly,
    /// 哈希整个 WAV 文件（含 RIFF 头）。
    WholeFile,
}

impl DigestScope {
    /// JSON 里的记号。
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::PcmPayloadOnly => "pcm-payload-only",
            Self::WholeFile => "whole-file",
        }
    }

    /// 从 JSON 记号还原。
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "pcm-payload-only" => Some(Self::PcmPayloadOnly),
            "whole-file" => Some(Self::WholeFile),
            _ => None,
        }
    }
}

/// 渲染参数（= 除工具链/平台身份之外的全部输入）。
#[derive(Clone, Debug, PartialEq)]
pub struct RenderParams {
    /// 夹具名（本线固定 `reference-a`）。
    pub fixture: String,
    /// 确定性种子 `[ARCH-DET-001]`。
    pub seed: u64,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 总帧数。
    pub frames: u64,
    /// 轨道数。
    pub tracks: u32,
    /// 处理块大小（帧）。
    pub block_size: usize,
    /// 边增益：`none` 或十进制 dB。
    pub gain_db: String,
    /// 线程策略：`auto` 或十进制。**不参与比对**（`[ARCH-DET-002]` 的实测结论）。
    pub threads: String,
    /// 每节点注入的 PDC 延迟（节点 ULID → 帧），按键升序。
    pub latency: BTreeMap<String, u32>,
}

/// 平台与工具链身份（= "同平台 / 锁工具链"这条判据的全部输入）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformIdentity {
    /// `cfg!(target_arch)`。
    pub target_arch: String,
    /// `cfg!(target_os)`。
    pub target_os: String,
    /// `cfg!(target_env)`（空值写 `-`）。
    pub target_env: String,
    /// `cfg!(target_endian)`。
    pub target_endian: String,
    /// `cfg!(target_pointer_width)`。
    pub target_pointer_width: String,
    /// 完整目标三元组（`rustc -vV` 的 `host:`）。
    pub target_triple: String,
    /// `rustc -vV` 的 `release:`（**锁定工具链的钥匙**）。
    pub rustc_release: String,
    /// `rustc -vV` 的 `host:`。
    pub rustc_host: String,
    /// `rustc -vV` 的 `commit-hash:`（仅记录）。
    pub rustc_commit: String,
    /// `rustc -vV` 的 `commit-date:`（仅记录）。
    pub rustc_commit_date: String,
    /// 基准指令集：如 `baseline` 或 `x86-64-v3+fma,+avx2`（逗号分隔、无空格）。
    pub isa_features: String,
}

/// 摘要口径（= "这个数是怎么算出来的"）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestEnvelope {
    /// 哈希算法（本线固定 `sha256`）。
    pub algorithm: String,
    /// 哈希输入（本线固定 `canonical-pcm-bits`）。
    pub input: String,
    /// 样本格式（本线固定 `f32-le`）。
    pub sample_format: String,
    /// 容器编码（如 `pcm-f32-le`）。
    pub wav_encoding: String,
    /// 覆盖范围。
    pub scope: DigestScope,
    /// WAV 文件的字节数（存档证据）。
    pub wav_bytes: u64,
}

/// **一份可存档的 L1 摘要记录**。
#[derive(Clone, Debug, PartialEq)]
pub struct DigestRecord {
    /// schema 标识。
    pub schema: String,
    /// 渲染参数。
    pub params: RenderParams,
    /// 平台与工具链身份。
    pub platform: PlatformIdentity,
    /// 摘要口径。
    pub envelope: DigestEnvelope,
    /// 被比对的读数：WAV 有效位流的 SHA-256（64 位小写十六进制）。
    pub digest: String,
    /// 独立交叉校验：位型串接的 SHA-256（`digest` 是它的函数）。
    pub sample_digest: String,
    /// `rustc -vV` 的 `release:` 全文（仅记录）。
    pub rustc_version: String,
    /// 宿主名（仅记录；刻意允许自由变化）。
    pub host_name: String,
    /// 宿主 OS 版本（仅记录）。
    pub host_os_version: String,
    /// 生成时刻（仅记录；`SOURCE_DATE_EPOCH` 可覆盖，便于复现）。
    pub generated_at_utc: String,
    /// 备注（仅记录）。
    pub notes: String,
}

/// 参与比对的字段名清单（供报告与文档使用）。
#[must_use]
pub fn compared_fields() -> Vec<&'static str> {
    FIELD_TABLE
        .iter()
        .filter(|spec| spec.participation == Participation::Compared)
        .map(|spec| spec.name)
        .collect()
}

/// 仅记录的字段名清单（供报告与文档使用）。
#[must_use]
pub fn recorded_only_fields() -> Vec<&'static str> {
    FIELD_TABLE
        .iter()
        .filter(|spec| spec.participation == Participation::RecordedOnly)
        .map(|spec| spec.name)
        .collect()
}

// ---------------------------------------------------------------------------
// SHA-256（自带实现：本模块零第三方依赖）
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 增量 SHA-256（`std` 只，零第三方依赖）。
///
/// 自带实现而不是引 `sha2` 的理由：本模块必须能在**零依赖**的 `rustc --test` 脚手架里
/// 本机真跑（见模块文档末节）。正确性由 NIST/FIPS 已知向量与"与 `render.rs` 的
/// `sha2` 结果逐字节相同"两条判据钉住。
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// 新建一个空的哈希器。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buffered: 0,
            length: 0,
        }
    }

    /// 喂入一段字节。
    pub fn update(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(bytes.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&bytes[..take]);
            self.buffered += take;
            bytes = &bytes[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while bytes.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&bytes[..64]);
            self.compress(&block);
            bytes = &bytes[64..];
        }
        if !bytes.is_empty() {
            self.buffer[..bytes.len()].copy_from_slice(bytes);
            self.buffered = bytes.len();
        }
    }

    /// 收尾并给出 32 字节摘要。
    #[must_use]
    pub fn finish(mut self) -> [u8; 32] {
        let bit_length = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0x00]);
        }
        // 长度字段本身不能让 `update` 再改变 `length` 的语义（已经被读进 `bit_length`）。
        let mut tail = [0u8; 64];
        tail[..self.buffered].copy_from_slice(&self.buffer[..self.buffered]);
        tail[56..].copy_from_slice(&bit_length.to_be_bytes());
        self.compress(&tail);
        let mut out = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut schedule = [0u32; 64];
        for (index, slot) in schedule.iter_mut().take(16).enumerate() {
            *slot = u32::from_be_bytes([
                block[index * 4],
                block[index * 4 + 1],
                block[index * 4 + 2],
                block[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        self.state = [
            self.state[0].wrapping_add(a),
            self.state[1].wrapping_add(b),
            self.state[2].wrapping_add(c),
            self.state[3].wrapping_add(d),
            self.state[4].wrapping_add(e),
            self.state[5].wrapping_add(f),
            self.state[6].wrapping_add(g),
            self.state[7].wrapping_add(h),
        ];
    }
}

/// 一次性 SHA-256。
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finish()
}

/// `[u8; 32]` → 64 位小写十六进制。
#[must_use]
pub fn hex_lower(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        write!(out, "{byte:02x}").expect("写入 String 不会失败");
    }
    out
}

/// 位型串接的 SHA-256（= `RenderOutput::digest_of` 的口径，供独立交叉校验）。
#[must_use]
pub fn sample_digest_of(samples: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for sample in samples {
        hasher.update(&sample.to_bits().to_le_bytes());
    }
    hex_lower(&hasher.finish())
}

// ---------------------------------------------------------------------------
// WAV 容器（`pcm-f32-le`，固定头布局）
// ---------------------------------------------------------------------------

/// 把交错 `f32` 样本编成一个**固定布局**的普通 RIFF/WAVE（`WAVE_FORMAT_IEEE_FLOAT`）。
///
/// 布局（[`WAV_HEADER_BYTES`] = 56 字节头）刻意不含任何可变内容：
///
/// ```text
/// offset  0  "RIFF" <u32 36+payload> "WAVE"
/// offset 12  "fmt " <u32 16> <u16 3=IEEE_FLOAT> <u16 channels> <u32 sample_rate>
///            <u32 byte_rate> <u16 block_align> <u16 32>
/// offset 36  "fact" <u32 4> <u32 frames>
/// offset 48  "data" <u32 payload>
/// offset 56  <payload: frames*channels 个 f32 小端位型>
/// ```
///
/// **没有** `bext`/`LIST/INFO`/时间戳 —— 那些会让"同一份音频"在不同时刻产出不同字节，
/// 从而把 `whole-file` 口径的哈希变成一个与声学无关的随机数。
#[must_use]
pub fn encode_wav_f32_le(samples: &[f32], channels: u16, sample_rate: u32) -> Vec<u8> {
    let payload = samples.len() * 4;
    let mut out = Vec::with_capacity(WAV_HEADER_BYTES as usize + payload);
    let u16le = |out: &mut Vec<u8>, value: u16| out.extend_from_slice(&value.to_le_bytes());
    let u32le = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
    out.extend_from_slice(b"RIFF");
    u32le(&mut out, 36 + payload as u32);
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    u32le(&mut out, 16);
    u16le(&mut out, 3);
    u16le(&mut out, channels);
    u32le(&mut out, sample_rate);
    let block_align =
        u16::try_from(4usize.saturating_mul(usize::from(channels))).unwrap_or(u16::MAX);
    u32le(&mut out, sample_rate.saturating_mul(u32::from(block_align)));
    u16le(&mut out, block_align);
    u16le(&mut out, 32);
    out.extend_from_slice(b"fact");
    u32le(&mut out, 4);
    if channels == 0 {
        u32le(&mut out, 0);
    } else {
        u32le(&mut out, (samples.len() / usize::from(channels)) as u32);
    }
    out.extend_from_slice(b"data");
    u32le(&mut out, payload as u32);
    for sample in samples {
        out.extend_from_slice(&sample.to_bits().to_le_bytes());
    }
    out
}

/// 从整份 WAV 字节里取出 `data` 块的**载荷**（不做任何解码）。
///
/// # Errors
///
/// 找不到 `RIFF`/`WAVE`/`data`、载荷被截断、`data` 长度字段与实际不符时返回说明。
pub fn wav_data_payload(bytes: &[u8]) -> Result<&[u8], String> {
    if bytes.len() < WAV_HEADER_BYTES as usize {
        return Err(format!(
            "WAV 太短: {} 字节 (至少 {WAV_HEADER_BYTES})",
            bytes.len()
        ));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("不是 RIFF/WAVE 容器".to_owned());
    }
    let mut cursor = 12usize;
    while cursor + 8 <= bytes.len() {
        let fourcc = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(size)
            .ok_or_else(|| "块长度溢出".to_owned())?;
        if end > bytes.len() {
            return Err(format!(
                "块 `{}` 声明 {size} 字节, 但文件只剩 {} 字节",
                String::from_utf8_lossy(fourcc),
                bytes.len() - start
            ));
        }
        if fourcc == b"data" {
            return Ok(&bytes[start..end]);
        }
        // RIFF 块按偶数字节对齐。
        cursor = end + (size % 2);
    }
    Err("容器里没有 `data` 块".to_owned())
}

/// 把交错 `f32` 位型串接成字节流（`canonical-pcm-bits`）。
#[must_use]
pub fn pcm_bits_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        out.extend_from_slice(&sample.to_bits().to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// 校验
// ---------------------------------------------------------------------------

/// 摘要记录自相矛盾（不是"比对结果不同"，而是"这份记录本身不可用"）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordError {
    /// 机器可读的原因码。
    pub code: &'static str,
    /// 人读说明。
    pub message: String,
}

impl RecordError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for RecordError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for RecordError {}

/// 编码是否**无损**（`data` 载荷 == 位型串接）。
#[must_use]
pub fn encoding_is_lossless(sample_format: &str, wav_encoding: &str) -> bool {
    sample_format == SAMPLE_FORMAT
        && matches!(
            wav_encoding,
            "pcm-f32-le" | "float32-le" | "ieee-float32-le"
        )
}

impl DigestRecord {
    /// 逐字段自洽性检查（**不是**格式检查，那是解析器的事）。
    ///
    /// # Errors
    ///
    /// 见 [`RecordError`] 的原因码。
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.schema.is_empty() {
            return Err(RecordError::new("schema-empty", "schema 不能为空"));
        }
        if self.params.fixture.is_empty() {
            return Err(RecordError::new("fixture-empty", "fixture 不能为空"));
        }
        if self.params.channels == 0 {
            return Err(RecordError::new("channels-zero", "channels 必须 >= 1"));
        }
        if self.params.frames == 0 {
            return Err(RecordError::new("frames-zero", "frames 必须 >= 1"));
        }
        if self.params.sample_rate == 0 {
            return Err(RecordError::new(
                "sample-rate-zero",
                "sample_rate 必须 >= 1",
            ));
        }
        if self.params.block_size == 0 {
            return Err(RecordError::new("block-size-zero", "block_size 必须 >= 1"));
        }
        if self.params.tracks == 0 {
            return Err(RecordError::new("tracks-zero", "tracks 必须 >= 1"));
        }
        if self.envelope.algorithm != DIGEST_ALGORITHM {
            return Err(RecordError::new(
                "digest-algorithm-unsupported",
                format!(
                    "digest_algorithm 必须是 `{DIGEST_ALGORITHM}`, 实际 `{}`",
                    self.envelope.algorithm
                ),
            ));
        }
        if self.envelope.input != DIGEST_INPUT {
            return Err(RecordError::new(
                "digest-input-unsupported",
                format!(
                    "digest_input 必须是 `{DIGEST_INPUT}`, 实际 `{}`",
                    self.envelope.input
                ),
            ));
        }
        if !is_lower_hex_64(&self.digest) {
            return Err(RecordError::new(
                "digest-not-hex64",
                format!("digest 必须是 64 位小写十六进制: `{}`", self.digest),
            ));
        }
        if !is_lower_hex_64(&self.sample_digest) {
            return Err(RecordError::new(
                "sample-digest-not-hex64",
                format!(
                    "sample_digest 必须是 64 位小写十六进制: `{}`",
                    self.sample_digest
                ),
            ));
        }
        // 口径与字节数必须自洽。
        let samples = self
            .params
            .frames
            .checked_mul(self.params.channels as u64)
            .ok_or_else(|| RecordError::new("sample-count-overflow", "frames * channels 溢出"))?;
        let payload = samples
            .checked_mul(SAMPLE_BYTES)
            .ok_or_else(|| RecordError::new("payload-overflow", "payload 字节数溢出"))?;
        match self.envelope.scope {
            DigestScope::PcmPayloadOnly => {
                // `wav_bytes` 永远是**整个文件**的字节数（口径不改变文件大小）；
                // `scope` 只说明 SHA-256 覆盖了其中哪一段。两者必须自洽。
                if self.envelope.wav_bytes != WAV_HEADER_BYTES + payload {
                    return Err(RecordError::new(
                        "wav-bytes-mismatch",
                        format!(
                            "wav_bytes 必须是整个文件的字节数 {} + {payload} = {}, 实际 {}",
                            WAV_HEADER_BYTES,
                            WAV_HEADER_BYTES + payload,
                            self.envelope.wav_bytes
                        ),
                    ));
                }
                if !encoding_is_lossless(&self.envelope.sample_format, &self.envelope.wav_encoding)
                {
                    return Err(RecordError::new(
                        "validate-wav-lossless-rule",
                        format!(
                            "编码 `{}` / `{}` 是有损的, 不许用 pcm-payload-only 口径 \
                             (载荷哈希会静默丢掉量化误差); 必须改用 whole-file",
                            self.envelope.sample_format, self.envelope.wav_encoding
                        ),
                    ));
                }
                // 无损 ⇒ 载荷哈希与位型哈希必须**逐字节相同**。
                if self.digest != self.sample_digest {
                    return Err(RecordError::new(
                        "payload-and-bits-digest-differ",
                        format!(
                            "无损编码下 data 载荷与位型串接必须给出同一个 SHA-256, \
                             实际 digest={} sample_digest={}",
                            self.digest, self.sample_digest
                        ),
                    ));
                }
            }
            DigestScope::WholeFile => {
                if self.envelope.wav_bytes != WAV_HEADER_BYTES + payload {
                    return Err(RecordError::new(
                        "wav-bytes-mismatch",
                        format!(
                            "scope=whole-file 时 wav_bytes 必须等于 {} + {payload} = {}, 实际 {}",
                            WAV_HEADER_BYTES,
                            WAV_HEADER_BYTES + payload,
                            self.envelope.wav_bytes
                        ),
                    ));
                }
            }
        }
        if self.platform.target_arch.is_empty() {
            return Err(RecordError::new(
                "target-arch-empty",
                "target_arch 不能为空",
            ));
        }
        if self.platform.target_triple.is_empty() {
            return Err(RecordError::new(
                "target-triple-empty",
                "target_triple 不能为空",
            ));
        }
        if self.platform.rustc_release.is_empty() {
            return Err(RecordError::new(
                "rustc-release-empty",
                "rustc_release 不能为空 (锁定工具链的钥匙)",
            ));
        }
        if self.platform.rustc_host.is_empty() {
            return Err(RecordError::new(
                "rustc-host-empty",
                "rustc_host 不能为空 (来自 `rustc -vV` 的 host)",
            ));
        }
        if self.envelope.wav_encoding.is_empty() {
            return Err(RecordError::new(
                "wav-encoding-empty",
                "wav_encoding 不能为空",
            ));
        }
        Ok(())
    }

    /// 记录的"平台身份"（用于判定同平台 / 跨平台）。
    #[must_use]
    pub fn platform_of(&self) -> &PlatformIdentity {
        &self.platform
    }
}

fn is_lower_hex_64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

// ---------------------------------------------------------------------------
// 判决
// ---------------------------------------------------------------------------

/// 比对结论。**三个变体，语义不许互相冒充**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 同平台 + 锁工具链 + 全部参与字段逐字段相同 ⇒ SHA-256 全同。
    Pass,
    /// 同平台 + 锁工具链，但参与字段/读数不同 ⇒ 硬红。
    Fail,
    /// 平台不同（或工具链未锁定）⇒ **不可比**。既不是通过也不是失败。
    Skip,
    /// **显式策略**下允许跨平台比较，且被比对的读数（`digest` + `sample_digest`）
    /// 在全同的渲染参数上**逐字节相同**。
    ///
    /// 它**不是** `MUST-GATE-002` 的"通过"（规范要求的是同平台），而是**比同平台更强**的一条
    /// 观测：连 ISA 都不同的两台机器都给出同一个 WAV SHA-256。因此它的记号是
    /// `PASS-CROSS-PLATFORM`（刻意与 `PASS` 区分），并且**只有在调用方显式选择这条策略时**才可能
    /// 出现（见 [`judge_policy`] / [`CrossPlatform`]）—— 默认策略永远给 `Skip`。
    PassCrossPlatform,
}

/// 跨平台比较的策略。默认是**规范口径**（同平台才有结论）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrossPlatform {
    /// 规范口径：平台不同 ⇒ [`Verdict::Skip`]（"不可比"，不是通过也不是失败）。
    #[default]
    Skip,
    /// 更强的观测口径：平台不同时，若被比对的读数**逐字节相同** ⇒
    /// [`Verdict::PassCrossPlatform`]；不同 ⇒ 仍然 `Skip`（**不许**判 `Fail` ——
    /// 跨平台的差异是 `MUST-GATE-003` 的领域，判红就是假红）。
    DigestParity,
}

impl Verdict {
    /// 报告里的机器可读记号。
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Skip => "SKIP",
            Self::PassCrossPlatform => "PASS-CROSS-PLATFORM",
        }
    }

    /// 退出码：`0` = 通过（含显式策略下的跨平台读数一致），`1` = 硬红，`2` = 跳过（**不是通过**）。
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Pass | Self::PassCrossPlatform => 0,
            Self::Fail => 1,
            Self::Skip => 2,
        }
    }
}

/// 一个参与字段的差异。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDiff {
    /// 字段名。
    pub field: &'static str,
    /// 参考摘要里的值。
    pub reference: String,
    /// 本机重算的值。
    pub local: String,
}

/// 结构化判决（纯数据，可被任何调用方重新格式化）。
#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    /// 结论。
    pub verdict: Verdict,
    /// 机器可读的原因码。
    pub reason: &'static str,
    /// 该判决服务哪条门禁。
    pub gate: &'static str,
    /// 参考摘要的平台三元组。
    pub reference_triple: String,
    /// 本机重算摘要的平台三元组。
    pub local_triple: String,
    /// 是否同平台（`target_arch` + `target_os` + `target_triple` 全同）。
    pub same_platform: bool,
    /// 工具链发布号是否锁定相同。
    pub toolchain_locked: bool,
    /// 两个 `digest` 是否相同。
    pub digest_equal: bool,
    /// `sample_digest` 是否相同。
    pub sample_digest_equal: bool,
    /// 参与字段的差异（按 [`FIELD_TABLE`] 顺序，全部列出，不截断）。
    pub diffs: Vec<FieldDiff>,
    /// 被比对的参与字段数。
    pub compared_fields: usize,
    /// 仅记录字段里**取值不同**的那些（如实报告，**不影响**判决）。
    pub recorded_only_differences: Vec<FieldDiff>,
}

/// "不可比"的两种情形（调用方应打印 SKIP 而不是 PASS/FAIL）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JudgeError {
    /// 参考摘要本身不可用。
    ReferenceMalformed(RecordError),
    /// 本机摘要本身不可用。
    LocalMalformed(RecordError),
    /// 参考与本机的 schema 不同 ⇒ 无法逐字段比对。
    SchemaMismatch {
        /// 参考摘要的 schema。
        reference: String,
        /// 本机摘要的 schema。
        local: String,
    },
}

impl core::fmt::Display for JudgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ReferenceMalformed(error) => write!(f, "参考摘要不可用: {error}"),
            Self::LocalMalformed(error) => write!(f, "本机摘要不可用: {error}"),
            Self::SchemaMismatch { reference, local } => write!(
                f,
                "schema 不同, 无法逐字段比对: 参考 `{reference}` / 本机 `{local}`"
            ),
        }
    }
}

impl std::error::Error for JudgeError {}

/// 同平台的定义：`target_arch` + `target_os` + `target_triple` 全同。
#[must_use]
pub fn same_platform(a: &PlatformIdentity, b: &PlatformIdentity) -> bool {
    a.target_arch == b.target_arch
        && a.target_os == b.target_os
        && a.target_triple == b.target_triple
}

/// 工具链锁：`rustc_release` + `rustc_host` 全同。
///
/// **刻意不用** `rustc_version` 全文：那是**仅记录**字段（同一发布号的构建元数据会随
/// runner 镜像漂移 —— 实测：本机 `rustc -vV` 的 `rustc 1.99.0 (b940084d7 2026-09-28)`
/// 与 CI 归档日志里的那行逐字不同）。用全文当锁会把"同工具链"变成永远不成立的判据。
#[must_use]
pub fn toolchain_locked(a: &PlatformIdentity, b: &PlatformIdentity) -> bool {
    a.rustc_release == b.rustc_release && a.rustc_host == b.rustc_host
}

/// 逐字段比对参考摘要与本机重算摘要。
///
/// 判决顺序（每一步都对应模块文档里的一条规则）：
///
/// 1. 两份摘要分别自洽（否则 [`JudgeError`]，调用方打印"不可用"）；
/// 2. schema 不同 ⇒ [`JudgeError::SchemaMismatch`]；
/// 3. **平台不同** ⇒ [`Verdict::Skip`]，`reason=cross-platform`（跨 ISA/OS 的差异是
///    `MUST-GATE-003` 的场景，不是本门禁的失败）；
/// 4. **工具链未锁定** ⇒ [`Verdict::Skip`]，`reason=toolchain-not-locked`；
/// 5. 参与字段有差异（含 `digest`）⇒ [`Verdict::Fail`]，`reason=digest-mismatch`
///    或 `reason=compared-field-mismatch`；
/// 6. 否则 ⇒ [`Verdict::Pass`]，`reason=digest-identical`。
///
/// # Errors
///
/// 见 [`JudgeError`]。
#[allow(clippy::too_many_lines)]
pub fn judge(reference: &DigestRecord, local: &DigestRecord) -> Result<Judgement, JudgeError> {
    judge_policy(reference, local, CrossPlatform::Skip)
}

/// 带**显式跨平台策略**的判决（`judge` 就是它的 `CrossPlatform::Skip` 形态）。
///
/// 为什么要一个显式参数而不是一个布尔：跨平台比较是**另一条口径**（比同平台更强，
/// 但不属于 `MUST-GATE-002` 的字面要求）。把它做成具名枚举 ⇒ 调用点必须写清楚自己在比什么，
/// 且默认值永远是规范口径。
///
/// # Errors
///
/// 见 [`JudgeError`]。
#[allow(clippy::too_many_lines)]
pub fn judge_policy(
    reference: &DigestRecord,
    local: &DigestRecord,
    policy: CrossPlatform,
) -> Result<Judgement, JudgeError> {
    reference
        .validate()
        .map_err(JudgeError::ReferenceMalformed)?;
    local.validate().map_err(JudgeError::LocalMalformed)?;
    if reference.schema != local.schema {
        return Err(JudgeError::SchemaMismatch {
            reference: reference.schema.clone(),
            local: local.schema.clone(),
        });
    }

    // ⚠ 每个字段都必须出现在这张表里（顺序即 `FIELD_TABLE`），否则"字段表"就与实际
    // 判决脱节 —— 那正是本文件开头承诺要避免的事。下面的自检把这条钉住。
    let values = |record: &DigestRecord| -> BTreeMap<&'static str, String> {
        let mut map = BTreeMap::new();
        map.insert("schema", record.schema.clone());
        map.insert("fixture", record.params.fixture.clone());
        map.insert("seed", record.params.seed.to_string());
        map.insert("sample_rate", record.params.sample_rate.to_string());
        map.insert("channels", record.params.channels.to_string());
        map.insert("frames", record.params.frames.to_string());
        map.insert("tracks", record.params.tracks.to_string());
        map.insert("block_size", record.params.block_size.to_string());
        map.insert("gain_db", record.params.gain_db.clone());
        map.insert("latency", latency_token(&record.params.latency));
        map.insert("threads", record.params.threads.clone());
        map.insert("target_arch", record.platform.target_arch.clone());
        map.insert("target_os", record.platform.target_os.clone());
        map.insert("target_env", record.platform.target_env.clone());
        map.insert("target_endian", record.platform.target_endian.clone());
        map.insert(
            "target_pointer_width",
            record.platform.target_pointer_width.clone(),
        );
        map.insert("target_triple", record.platform.target_triple.clone());
        map.insert("rustc_release", record.platform.rustc_release.clone());
        map.insert("rustc_host", record.platform.rustc_host.clone());
        map.insert("isa_features", record.platform.isa_features.clone());
        map.insert("digest_algorithm", record.envelope.algorithm.clone());
        map.insert("digest_input", record.envelope.input.clone());
        map.insert("sample_format", record.envelope.sample_format.clone());
        map.insert("wav_encoding", record.envelope.wav_encoding.clone());
        map.insert("digest_scope", record.envelope.scope.token().to_owned());
        map.insert("wav_bytes", record.envelope.wav_bytes.to_string());
        map.insert("digest", record.digest.clone());
        map.insert("sample_digest", record.sample_digest.clone());
        map.insert("rustc_version", record.rustc_version.clone());
        map.insert("rustc_commit", record.platform.rustc_commit.clone());
        map.insert(
            "rustc_commit_date",
            record.platform.rustc_commit_date.clone(),
        );
        map.insert("host_name", record.host_name.clone());
        map.insert("host_os_version", record.host_os_version.clone());
        map.insert("generated_at_utc", record.generated_at_utc.clone());
        map.insert("notes", record.notes.clone());
        map
    };
    let reference_values = values(reference);
    let local_values = values(local);
    // 自检：字段表与判决读取的字段**必须完全一致**（多一个少一个都是脱节）。
    for spec in FIELD_TABLE {
        assert!(
            reference_values.contains_key(spec.name),
            "字段表里的 `{}` 没有被判决读取 —— 归一化口径与实现脱节",
            spec.name
        );
    }
    assert_eq!(
        reference_values.len(),
        FIELD_TABLE.len(),
        "判决读取的字段数与字段表长度不同, 归一化口径与实现脱节"
    );

    let mut diffs = Vec::new();
    let mut recorded = Vec::new();
    for spec in FIELD_TABLE {
        let a = &reference_values[spec.name];
        let b = &local_values[spec.name];
        if a == b {
            continue;
        }
        let diff = FieldDiff {
            field: spec.name,
            reference: a.clone(),
            local: b.clone(),
        };
        match spec.participation {
            Participation::Compared => diffs.push(diff),
            Participation::RecordedOnly => recorded.push(diff),
        }
    }

    let platform_match = same_platform(&reference.platform, &local.platform);
    let locked = toolchain_locked(&reference.platform, &local.platform);
    let digest_equal = reference.digest == local.digest;
    let sample_equal = reference.sample_digest == local.sample_digest;
    let (verdict, reason) = if !platform_match {
        // 跨平台的哈希差异是**预期**的（MUST-GATE-003 的领域），默认策略下本门禁比不了。
        if policy == CrossPlatform::DigestParity && digest_equal && sample_equal {
            // 显式策略下的**更强观测**：ISA 都不同却给出同一个 WAV SHA-256。
            (
                Verdict::PassCrossPlatform,
                "cross-platform-digest-identical",
            )
        } else {
            (Verdict::Skip, "cross-platform")
        }
    } else if !locked {
        // 规范要求的是"**锁定**工具链下"的确定性；未锁定的两份读数说明不了任何事。
        (Verdict::Skip, "toolchain-not-locked")
    } else if !digest_equal || !sample_equal {
        (Verdict::Fail, "digest-mismatch")
    } else if !diffs.is_empty() {
        (Verdict::Fail, "compared-field-mismatch")
    } else {
        (Verdict::Pass, "digest-identical")
    };

    Ok(Judgement {
        verdict,
        reason,
        gate: "MUST-GATE-002",
        reference_triple: reference.platform.target_triple.clone(),
        local_triple: local.platform.target_triple.clone(),
        same_platform: platform_match,
        toolchain_locked: locked,
        digest_equal,
        sample_digest_equal: sample_equal,
        diffs,
        compared_fields: compared_fields().len(),
        recorded_only_differences: recorded,
    })
}

/// 延迟表的确定性文本（键升序，`ULID:帧` 以 `,` 连接；空表写 `none`）。
#[must_use]
pub fn latency_token(latency: &BTreeMap<String, u32>) -> String {
    if latency.is_empty() {
        return "none".to_owned();
    }
    let mut out = String::new();
    for (index, (node, frames)) in latency.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write!(out, "{node}:{frames}").expect("写入 String 不会失败");
    }
    out
}

/// 人读 + 机器可读的判决报告（首行是 `VERDICT <记号>`，便于 `grep`）。
#[must_use]
pub fn report(judgement: &Judgement) -> String {
    let mut out = String::new();
    emitln!(out, "VERDICT {}", judgement.verdict.token());
    emitln!(out, "reason={}", judgement.reason);
    emitln!(out, "gate={}", judgement.gate);
    emitln!(out, "exit_code={}", judgement.verdict.exit_code());
    emitln!(out, "same_platform={}", judgement.same_platform);
    emitln!(out, "toolchain_locked={}", judgement.toolchain_locked);
    emitln!(out, "reference_target={}", judgement.reference_triple);
    emitln!(out, "local_target={}", judgement.local_triple);
    emitln!(out, "digest_equal={}", judgement.digest_equal);
    emitln!(out, "sample_digest_equal={}", judgement.sample_digest_equal);
    emitln!(out, "compared_fields={}", judgement.compared_fields);
    emitln!(out, "compared_field_differences={}", judgement.diffs.len());
    for diff in &judgement.diffs {
        emitln!(
            out,
            "diff field={} reference={} local={}",
            diff.field,
            diff.reference,
            diff.local
        );
    }
    emitln!(
        out,
        "recorded_only_differences={}",
        judgement.recorded_only_differences.len()
    );
    for diff in &judgement.recorded_only_differences {
        emitln!(
            out,
            "recorded-only field={} reference={} local={}",
            diff.field,
            diff.reference,
            diff.local
        );
    }
    if judgement.verdict == Verdict::Skip {
        emitln!(
            out,
            "note=SKIP 既不是通过也不是失败: {}",
            skip_explanation(judgement.reason)
        );
    }
    if judgement.verdict == Verdict::PassCrossPlatform {
        emitln!(
            out,
            "note=PASS-CROSS-PLATFORM 是**更强**的观测, 但**不是 MUST-GATE-002 的通过**: {}",
            skip_explanation(judgement.reason)
        );
    }
    out
}

/// SKIP 原因码的人读解释（**必须**让"跳过"与"通过"在文本上不可混淆）。
#[must_use]
pub fn skip_explanation(reason: &str) -> &'static str {
    match reason {
        "cross-platform" => {
            "参考摘要与本机不是同一个平台 (target_arch/target_os/target_triple 不同); \
             跨平台的哈希差异是 MUST-GATE-003 的领域, 本门禁**没有**判定, 请勿记为通过"
        }
        "toolchain-not-locked" => {
            "同平台但 rustc 发布号/主机不同; 规范要求的是锁定工具链下的确定性, \
             因此本轮**没有**判定, 请勿记为通过"
        }
        "cross-platform-digest-identical" => {
            "跨平台, 但两份读数逐字节相同 —— 这是比同平台更强的一条观测; \
             它仍**不是** MUST-GATE-002 的通过(规范要求同平台), 要闭环还需同平台的第二台机器"
        }
        _ => "原因码未知 (这本身就是一个缺陷)",
    }
}

/// 把摘要记录渲染成**单行** JSON（带换行结尾，可直接 `>>` 进归档文件）。
#[must_use]
pub fn to_json_line(record: &DigestRecord) -> String {
    let mut out = to_json(record);
    out.push('\n');
    out
}

/// 把摘要记录渲染成 JSON 对象（**单行**，`{...}`）。
#[must_use]
pub fn to_json(record: &DigestRecord) -> String {
    let mut out = String::new();
    out.push('{');
    let mut first = true;
    let mut field = |out: &mut String, name: &str, value: String| {
        if !first {
            out.push(',');
        }
        first = false;
        emit!(out, "\"{name}\":{value}");
    };
    let quoted = |value: &str| json_string(value);
    field(&mut out, "schema", quoted(&record.schema));
    field(&mut out, "fixture", quoted(&record.params.fixture));
    field(&mut out, "seed", record.params.seed.to_string());
    field(
        &mut out,
        "sample_rate",
        record.params.sample_rate.to_string(),
    );
    field(&mut out, "channels", record.params.channels.to_string());
    field(&mut out, "frames", record.params.frames.to_string());
    field(&mut out, "tracks", record.params.tracks.to_string());
    field(&mut out, "block_size", record.params.block_size.to_string());
    field(&mut out, "gain_db", quoted(&record.params.gain_db));
    field(&mut out, "latency", latency_json(&record.params.latency));
    field(&mut out, "threads", quoted(&record.params.threads));
    field(
        &mut out,
        "target_arch",
        quoted(&record.platform.target_arch),
    );
    field(&mut out, "target_os", quoted(&record.platform.target_os));
    field(
        &mut out,
        "target_env",
        quoted(empty_as_dash(&record.platform.target_env)),
    );
    field(
        &mut out,
        "target_endian",
        quoted(&record.platform.target_endian),
    );
    field(
        &mut out,
        "target_pointer_width",
        quoted(&record.platform.target_pointer_width),
    );
    field(
        &mut out,
        "target_triple",
        quoted(&record.platform.target_triple),
    );
    field(
        &mut out,
        "rustc_release",
        quoted(&record.platform.rustc_release),
    );
    field(&mut out, "rustc_host", quoted(&record.platform.rustc_host));
    field(
        &mut out,
        "isa_features",
        quoted(&record.platform.isa_features),
    );
    field(
        &mut out,
        "digest_algorithm",
        quoted(&record.envelope.algorithm),
    );
    field(&mut out, "digest_input", quoted(&record.envelope.input));
    field(
        &mut out,
        "sample_format",
        quoted(&record.envelope.sample_format),
    );
    field(
        &mut out,
        "wav_encoding",
        quoted(&record.envelope.wav_encoding),
    );
    field(
        &mut out,
        "digest_scope",
        quoted(record.envelope.scope.token()),
    );
    field(&mut out, "wav_bytes", record.envelope.wav_bytes.to_string());
    field(&mut out, "digest", quoted(&record.digest));
    field(&mut out, "sample_digest", quoted(&record.sample_digest));
    field(&mut out, "rustc_version", quoted(&record.rustc_version));
    field(
        &mut out,
        "rustc_commit",
        quoted(&record.platform.rustc_commit),
    );
    field(
        &mut out,
        "rustc_commit_date",
        quoted(&record.platform.rustc_commit_date),
    );
    field(&mut out, "host_name", quoted(&record.host_name));
    field(&mut out, "host_os_version", quoted(&record.host_os_version));
    field(
        &mut out,
        "generated_at_utc",
        quoted(&record.generated_at_utc),
    );
    field(&mut out, "notes", quoted(&record.notes));
    out.push('}');
    out
}

/// 把单行 JSON **按字段顺序重排成多行**（只用于提交进仓库的参考摘要的存档形态）。
///
/// 它刻意不做通用 JSON 美化：解析 → 重新序列化 ⇒ 多行文本与单行文本在**数据模型**层面
/// 相等（判据 ⑦ 的形态之一）。这样"人类可 diff 的存档"与"机器可解析"不必二选一。
#[must_use]
pub fn to_pretty_json(record: &DigestRecord) -> String {
    let compact = to_json(record);
    let inner = compact
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(compact.as_str());
    let pairs = split_top_level(inner);
    let mut out = String::from("{\n");
    for (index, pair) in pairs.iter().enumerate() {
        out.push_str("  ");
        out.push_str(pair);
        if index + 1 < pairs.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// 按顶层 `,` 切分（本记录的字段值里不含裸 `,`：`latency` 里的分隔符在字符串里）。
fn split_top_level(inner: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in inner.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b',' => {
                parts.push(&inner[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        parts.push(&inner[start..]);
    }
    parts
}

/// `latency` 映射的 JSON 对象（键升序 ⇒ 确定）。
fn latency_json(latency: &BTreeMap<String, u32>) -> String {
    let mut out = String::from("{");
    for (index, (node, frames)) in latency.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        emit!(out, "{}:{}", json_string(node), frames);
    }
    out.push('}');
    out
}

/// JSON 字符串字面量（转义 `"`、`\` 与控制字符；非 ASCII 原样保留）。
#[must_use]
pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                emit!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn empty_as_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

// ---------------------------------------------------------------------------
// 锁定工具链的解析（`rustc -vV` → 发布号 / commit / 日期）
// ---------------------------------------------------------------------------

/// 把 `rustc -vV` 的版本行拆成 `(发布号, commit-hash, commit-date)`。
///
/// 输入形如 `1.99.0 (b940084d7eb6a299eb4bfeb8e34901bc051e7ac4 2026-09-28)`。
/// **缺任何一段就返回空串 —— 绝不编造**。发布号是"锁定工具链"的钥匙（参与比对），
/// commit 与日期只做记录（同一发布号的构建元数据会随 runner 镜像漂移，本机与 CI 实测如此）。
#[must_use]
pub fn split_rustc_version(version: Option<&str>) -> (String, String, String) {
    let Some(version) = version else {
        return (String::new(), String::new(), String::new());
    };
    let release = version
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    let inside = version
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(inner, _)| inner)
        .unwrap_or_default();
    let mut meta = inside.split_whitespace();
    let commit = meta.next().unwrap_or_default().to_owned();
    let commit_date = meta.next().unwrap_or_default().to_owned();
    (release, commit, commit_date)
}

// ---------------------------------------------------------------------------
// 时间戳（**仅记录**字段；ISO-8601 UTC，秒精度）
// ---------------------------------------------------------------------------

/// `i64` Unix 秒 → `YYYY-MM-DDTHH:MM:SSZ`（公历投影，零依赖）。
///
/// 它只服务 `generated_at_utc` 这个**仅记录**字段：判据 ⑥ 靠它证明"时间戳变化不影响
/// 摘要比对"。因此这里不需要时区数据库、不需要闰秒表 —— 需要的是**确定**。
///
/// 1970..=2100 的闰年规则就是"能被 4 整除"，因此不需要 100/400 年的例外分支。
#[must_use]
pub fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let hour = remainder / 3600;
    let minute = (remainder % 3600) / 60;
    let second = remainder % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// 天数（1970-01-01 起）→ `(年, 月, 日)`。Howard Hinnant 的 `civil_from_days`。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

// ---------------------------------------------------------------------------
// 解析（严格；不 panic）
// ---------------------------------------------------------------------------

/// JSON 解析失败（行号 1 起；`0` 表示整份记录层面的问题）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 行号，`0` = 整份记录。
    pub line: usize,
    /// 说明。
    pub message: String,
}

impl ParseError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.line == 0 {
            write!(f, "摘要记录非法: {}", self.message)
        } else {
            write!(f, "摘要记录第 {} 行非法: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ParseError {}

/// 严格的 JSON 解析：只接受本记录需要的 JSON 子集 + 拒绝未知字段。
///
/// 拒绝未知字段是刻意的：摘要记录是**判据的输入**，一个拼错的字段名会静默地让
/// "参与比对"的字段少一个 —— 那正是本仓库最忌讳的假绿。
///
/// # Errors
///
/// 见 [`ParseError`]。
#[allow(clippy::too_many_lines)]
pub fn parse(text: &str) -> Result<DigestRecord, ParseError> {
    let mut parser = JsonParser::new(text);
    let value = parser.parse_value()?;
    let object = match value {
        Json::Object(map) => map,
        other => {
            return Err(ParseError::new(
                1,
                format!("顶层必须是 JSON 对象, 实际是 {}", other.kind()),
            ));
        }
    };
    if let Some(Json::String(schema)) = object.get("schema")
        && !SUPPORTED_SCHEMAS.contains(&schema.as_str())
    {
        return Err(ParseError::new(
            1,
            format!(
                "不认识的 schema `{schema}` (本解析器只认 {:?}): \
                 跨版本的记录必须由**认识它的**解析器读, 不许猜",
                SUPPORTED_SCHEMAS
            ),
        ));
    }
    for key in object.keys() {
        if participation_of(key).is_none() {
            return Err(ParseError::new(
                1,
                format!("未知字段 `{key}`（拼写错误? 或是本解析器不认识的版本）"),
            ));
        }
    }
    let mut latency = BTreeMap::new();
    if let Some(Json::Object(map)) = object.get("latency") {
        for (node, frames) in map {
            let Json::Number(frames) = frames else {
                return Err(ParseError::new(1, format!("latency.{node} 必须是整数")));
            };
            let frames: u32 = frames
                .parse()
                .map_err(|_| ParseError::new(1, format!("latency.{node} 非法: `{frames}`")))?;
            latency.insert(node.clone(), frames);
        }
    } else if object.contains_key("latency") {
        return Err(ParseError::new(1, "latency 必须是 JSON 对象".to_owned()));
    }

    let string = |name: &str| -> Result<String, ParseError> {
        match object.get(name) {
            Some(Json::String(value)) => Ok(value.clone()),
            Some(other) => Err(ParseError::new(
                1,
                format!("`{name}` 必须是字符串, 实际是 {}", other.kind()),
            )),
            None => Err(ParseError::new(1, format!("缺少必填字段 `{name}`"))),
        }
    };
    let number = |name: &str| -> Result<u64, ParseError> {
        match object.get(name) {
            Some(Json::Number(value)) => value
                .parse()
                .map_err(|_| ParseError::new(1, format!("`{name}` 不是合法整数: `{value}`"))),
            Some(other) => Err(ParseError::new(
                1,
                format!("`{name}` 必须是整数, 实际是 {}", other.kind()),
            )),
            None => Err(ParseError::new(1, format!("缺少必填字段 `{name}`"))),
        }
    };
    let usize_of = |name: &str| -> Result<usize, ParseError> {
        let value = number(name)?;
        usize::try_from(value)
            .map_err(|_| ParseError::new(1, format!("`{name}` 超出 usize 范围: {value}")))
    };

    let scope_token = string("digest_scope")?;
    let scope = DigestScope::from_token(&scope_token).ok_or_else(|| {
        ParseError::new(
            1,
            format!("digest_scope 只认 pcm-payload-only / whole-file, 实际 `{scope_token}`"),
        )
    })?;
    let target_env = {
        let raw = string("target_env")?;
        if raw == "-" { String::new() } else { raw }
    };
    let record = DigestRecord {
        schema: string("schema")?,
        params: RenderParams {
            fixture: string("fixture")?,
            seed: number("seed")?,
            sample_rate: u32::try_from(number("sample_rate")?)
                .map_err(|_| ParseError::new(1, "`sample_rate` 超出 u32 范围".to_owned()))?,
            channels: usize_of("channels")?,
            frames: number("frames")?,
            tracks: u32::try_from(number("tracks")?)
                .map_err(|_| ParseError::new(1, "`tracks` 超出 u32 范围".to_owned()))?,
            block_size: usize_of("block_size")?,
            gain_db: string("gain_db")?,
            threads: string("threads")?,
            latency,
        },
        platform: PlatformIdentity {
            target_arch: string("target_arch")?,
            target_os: string("target_os")?,
            target_env,
            target_endian: string("target_endian")?,
            target_pointer_width: string("target_pointer_width")?,
            target_triple: string("target_triple")?,
            rustc_release: string("rustc_release")?,
            rustc_host: string("rustc_host")?,
            rustc_commit: string("rustc_commit")?,
            rustc_commit_date: string("rustc_commit_date")?,
            isa_features: string("isa_features")?,
        },
        envelope: DigestEnvelope {
            algorithm: string("digest_algorithm")?,
            input: string("digest_input")?,
            sample_format: string("sample_format")?,
            wav_encoding: string("wav_encoding")?,
            scope,
            wav_bytes: number("wav_bytes")?,
        },
        digest: string("digest")?,
        sample_digest: string("sample_digest")?,
        rustc_version: string("rustc_version")?,
        host_name: string("host_name")?,
        host_os_version: string("host_os_version")?,
        generated_at_utc: string("generated_at_utc")?,
        notes: string("notes")?,
    };
    record.validate().map_err(|error| {
        ParseError::new(
            1,
            format!("记录自相矛盾 ({}): {}", error.code, error.message),
        )
    })?;
    Ok(record)
}

/// JSON 值（只覆盖本记录用到的三种）。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Json {
    String(String),
    Number(String),
    Object(BTreeMap<String, Json>),
}

impl Json {
    const fn kind(&self) -> &'static str {
        match self {
            Self::String(_) => "字符串",
            Self::Number(_) => "数字",
            Self::Object(_) => "对象",
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> JsonParser<'a> {
    const fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    fn parse_value(&mut self) -> Result<Json, ParseError> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'"') => Ok(Json::String(self.parse_string()?)),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.parse_number(),
            Some(byte) => Err(ParseError::new(
                1,
                format!("不支持的 JSON 值起始字符 `{}`", char::from(byte)),
            )),
            None => Err(ParseError::new(1, "JSON 内容为空".to_owned())),
        }
    }

    fn parse_object(&mut self) -> Result<Json, ParseError> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            if map.insert(key.clone(), value).is_some() {
                return Err(ParseError::new(1, format!("字段 `{key}` 重复")));
            }
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                }
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(map));
                }
                _ => return Err(ParseError::new(1, "对象里期望 `,` 或 `}`".to_owned())),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| ParseError::new(1, "字符串未闭合".to_owned()))?;
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or_else(|| ParseError::new(1, "转义序列被截断".to_owned()))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'u' => {
                            let hex = self
                                .bytes
                                .get(self.at..self.at + 4)
                                .ok_or_else(|| ParseError::new(1, "\\u 转义被截断".to_owned()))?;
                            let text = core::str::from_utf8(hex)
                                .map_err(|_| ParseError::new(1, "\\u 转义非 UTF-8".to_owned()))?;
                            let code = u32::from_str_radix(text, 16).map_err(|_| {
                                ParseError::new(1, format!("\\u 转义非法: `{text}`"))
                            })?;
                            self.at += 4;
                            let character = char::from_u32(code).ok_or_else(|| {
                                ParseError::new(1, format!("\\u{code:04x} 不是合法字符"))
                            })?;
                            out.push(character);
                        }
                        other => {
                            return Err(ParseError::new(
                                1,
                                format!("未知转义 `\\{}`", char::from(other)),
                            ));
                        }
                    }
                }
                _ => {
                    // 直接按 UTF-8 续进（本记录的字符串都是 ASCII/UTF-8 单字节安全）。
                    let start = self.at - 1;
                    let mut end = self.at;
                    while end < self.bytes.len() && self.bytes[end] & 0xC0 == 0x80 {
                        end += 1;
                    }
                    let slice = &self.bytes[start..end];
                    let text = core::str::from_utf8(slice)
                        .map_err(|_| ParseError::new(1, "字符串不是合法 UTF-8".to_owned()))?;
                    out.push_str(text);
                    self.at = end;
                }
            }
        }
    }

    fn parse_number(&mut self) -> Result<Json, ParseError> {
        let start = self.at;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit()
                || byte == b'-'
                || byte == b'+'
                || byte == b'.'
                || byte == b'e'
                || byte == b'E'
            {
                self.at += 1;
            } else {
                break;
            }
        }
        let text = core::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| ParseError::new(1, "数字不是合法 UTF-8".to_owned()))?;
        if text.is_empty() {
            return Err(ParseError::new(1, "数字为空".to_owned()));
        }
        Ok(Json::Number(text.to_owned()))
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(ParseError::new(
                1,
                format!(
                    "期望 `{}`, 实际 {}",
                    char::from(byte),
                    self.peek().map_or_else(
                        || "内容结束".to_owned(),
                        |found| format!("`{}`", char::from(found)),
                    )
                ),
            ))
        }
    }
}
