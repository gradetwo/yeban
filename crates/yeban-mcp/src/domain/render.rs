//! `yeban_render_master` 的实现：参数校验 **+ 真渲染** [MCP-TOOL-008] [ROAD-M4-005]。
//!
//! ## 这一半现在是真的
//!
//! 上一线（`line/tools-domain`）只接了参数校验这一半：好参数会在校验通过之后返回
//! JSON-RPC `-32005 NOT_IMPLEMENTED`。现在这一半**接线到 `yeban-render`**：
//!
//! ```text
//! 工程 + 路由图
//!   ├─ track_latencies(DeviceDefinition::latency_samples)   [ARCH-PDC-001] 唯一延迟来源
//!   ├─ RenderPlan::compile_with_latencies(...)              拓扑分层 + PDC 关键路径
//!   ├─ 每个源节点注入一个 AudioSource                       MIDI 音符 → 确定性合成源
//!   ├─ RenderPlan::execute(...)                             Rayon 并行 + 固定顺序归约 [ARCH-DET-002]
//!   ├─ 母带增益 + (可选) 峰值归一化
//!   ├─ TPDF 抖动 → 24-bit PCM                              [ARCH-FMT-001]
//!   └─ RIFF / RF64 / BW64 容器 + bext 元数据               [ARCH-FMT-001]
//!        └─ 原子落盘: 同目录 .tmp + sync_all + rename      [ARCH-SEC-004]
//! ```
//!
//! 渲染发生在 [`build`]（**只读计算**，`dryRun` 也走它，因此预览里的字节数、
//! SHA-256、帧数都是**实测值**而不是估算），落盘发生在 `domain::apply`。
//! 这个切分保住了"`dryRun` 不改状态"的类型系统级保证：[`super::plan`] 只拿 `&Domain`。
//!
//! ## 真的渲染了什么 / 明确没渲染什么
//!
//! **真做**：MIDI 音符（起止 tick、音高、力度、微时值）、placement 的
//! `start_tick`（缺省 0）、`muted`、音轨 `volume_db` / `pan`（等功率 −3 dB）/
//! `mute` / `solo`（见下）、**未旁通设备的 `latency_samples`**（PDC 对齐）、
//! 边增益 `gain_db`、Master 轨的 `volume_db`、确定性 TPDF 抖动与 24-bit 量化、
//! RIFF/RF64/BW64 容器与 `bext` 元数据、峰值归一化。
//!
//! **明确没做**（只要工程里真的出现，就会同时出现在响应的 `unsupported` 与
//! `unsupportedCounts` 里，**绝不静默**）：
//!
//! | 键 | 含义 |
//! | :--- | :--- |
//! | `audioClips` | `ClipContent::Audio` 的片段内容没有解码（CAS 资产池的字节未被消费），当作静音 |
//! | `deviceChainDsp` | 设备链的**参数**（滤波器/音色）没有求值；只有 `latency_samples` 进了 PDC |
//! | `externalPlugins` | `DeviceKind::ExternalInstrument/ExternalEffect` 没有宿主 |
//! | `automationLanes` | 自动化曲线没有求值（静态值也不代偿） |
//! | `clipLoopRepetition` | `loop_config` 的**重复**没有渲染（只渲染第一遍） |
//! | `noteProbability` | `probability < 1.0` 的音符被当作必然触发（没有引入熵源） |
//! | `noteRatchet` | `ratchet > 1` 的连击没有展开 |
//! | `noteSlide` | 滑音没有实现 |
//! | `notePitchBend` | 弯音曲线没有求值 |
//! | `noteLyrics` | 歌词/音素没有歌声合成 |
//! | `busTrackClips` | 非源节点（总线/主轨）上的片段没有渲染（只有源节点有音源） |
//! | `sidechainRouting` | 侧链边按普通音频边处理（`yeban-render` 的既有边界） |
//! | `masterPan` | Master 轨的 `pan` 没有应用（母带输出的声相由总线求和决定） |
//! | `sfzSampler` | 本模型版本（`DeviceKind`）里**没有** SFZ 设备变体；该能力不在本切片内 |
//!
//! "工程里有 MIDI 音符"这一条是渲染的**前提**：完全没有可渲染内容（0 帧）时返回
//! `RENDER_FAILED`，而不是写一个 0 帧的文件冒充成功。
//!
//! ## 输出路径规则（确定性，`dryRun` 与真调用共用同一份实现）
//!
//! ```text
//! arguments.path 存在        → 原样使用（拒绝空串）
//! 否则                       → <工程文件所在目录>/<工程文件 stem>.master.<format>
//!                              例: /x/demo.yeban + format=wav ⇒ /x/demo.master.wav
//!                              没有 stem 时用 "master": /x/.master.wav
//! ```
//!
//! 两条**安全护栏**（都返回契约内的 `INVALID_PARAMETER_RANGE`）：
//! 输出路径不得等于工程文件本身、也不得等于 `.yeban.lock`。
//!
//! ## `normalize` 的口径（含"全零信号"边界）
//!
//! **峰值归一化**，作用域是**抖动/量化之前的浮点母带**（也就是 `RenderOutput.samples`
//! 乘以母带增益之后的那一份）：
//!
//! ```text
//! peak = max |s[i]|          (非有限样本不计入)
//! gain = 1.0 / peak          (peak > 0)
//! 目标 = 1.0 (满量程), 因此归一化后峰值 == 满刻度
//! ```
//!
//! - **全零信号**（`peak == 0`）：`gain = 1.0`，即**不改动任何样本**，响应里
//!   `normalize.applied = false` + `normalize.note` 说明原因。把 0 放大到任何目标都是
//!   在制造一个不存在的信号，因此这里刻意不"成功"。
//! - 容差：f32 的除法与乘法各一次舍入，实测 `|peak_after − 1.0| < 1e-6`
//!   （相对误差上界 ~2⁻²⁴）。判据在 `tests/render_master.rs`。
//! - 归一化发生在抖动**之前**：抖动是"降位深"这一步的伴生 [ARCH-FMT-001]，
//!   对还没定标的信号抖动等于提前污染母带。
//!
//! ## 可观测性（响应 `data` 里的数字全部是实测值）
//!
//! `frames` / `channels` / `sampleRate` / `bytes`（容器总字节）/ `headerBytes` /
//! `payloadBytes` / `sha256`（整份文件）/ `masterDigest`（母带样本的位级 SHA-256）/
//! `blocks` / `longestPathFrames` / `peak.before` / `peak.after` / `sourceNodes` /
//! `unsupported`。`dryRun` 返回同一批数字 + `wouldWrite`，**不落盘**。
//!
//! ## 错误映射（不发明新码，`ADR-0001 D25`）
//!
//! | 状况 | 出口 |
//! | :--- | :--- |
//! | `format` 不在白名单 / `sampleRate` 不在模型集合 / `normalize` 非布尔 / `path` 非字符串 / 输出路径为工程或锁文件 | `INVALID_PARAMETER_RANGE`（带内） |
//! | 没有活跃工程 | `NO_ACTIVE_PROJECT`（带内） |
//! | 采样率与工程不一致（重采样器未接线）、0 帧、超出帧数上限、路由图非法、缺音轨/片段 | `RENDER_FAILED`（带内，`data` 带原因与规范 ID） |
//! | 输出目录不可写 / 磁盘满 / 目标父目录不存在 | `IO_ERROR` / `DISK_FULL`（带内，来自 `store` 的既有映射） |
//!
//! `BUSY` 在当前架构下**仍然不可达**：领域状态单线程同步，一次 `tools/call` 完整跑完
//! 才返回，不存在"已经有一个渲染在跑"的窗口。这是登记，不是遗漏。
//!
//! ## 幂等
//!
//! 不在这里实现第二套：[`crate::dispatch`] 的幂等缓存查询在工具执行**之前**，
//! 相同 `idempotencyKey` 的第二次调用根本到不了 `domain::execute` ⇒ 不重复渲染。
//! 判据用**输出文件的 inode** 与"删掉文件后重放同键不会重建"两个可观测量证明。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use yeban_model::{
    AssetHash, ClipContent, DeviceKind, EntityId, PPQ, RoutingKind, SampleRate, TrackV3,
    YebanProjectV1,
};
use yeban_render::dither::{BitDepth, quantize};
use yeban_render::render::{
    AudioSource, BlockContext, L1_BLOCK_SIZE, RenderError, RenderOptions, RenderPlan, db_to_linear,
    track_latencies,
};
use yeban_render::rf64::{Bext, ContainerKind, ContainerPlan, PcmFormat, write_container};
use yeban_render::rng::dither_rng_for;

use super::error::Fault;
use super::render_math as math;
use super::store;
use crate::tools::ErrorCode;

/// 允许的输出容器/编码。
pub const FORMATS: [&str; 3] = ["wav", "rf64", "bw64"];

/// 母带声道数（规范把 Master 定义为立体声总线；工程模型里没有声道数字段）。
pub const MASTER_CHANNELS: usize = 2;

/// 导出位深：**恒为 24-bit 整数 PCM**（[ARCH-FMT-001] 的内置高质量 TPDF 抖动路径）。
pub const OUTPUT_BIT_DEPTH: u16 = 24;

/// 归一化目标（满量程）。
pub const NORMALIZE_TARGET: f32 = 1.0;

/// 起音时长（毫秒）。
pub const ATTACK_MS: u32 = 5;

/// 释音时长（毫秒）。
pub const RELEASE_MS: u32 = 10;

/// 单次渲染的帧数上限：1 小时 @ 48 kHz。
///
/// 不是性能目标，而是一道**防呆闸门**：工程里一个错误的 `duration_ticks`
/// （或极慢的 BPM）可以在几秒内申请几十 GB 缓冲。超过即 `RENDER_FAILED`
/// 并如实报出请求帧数与上限。
pub const MAX_RENDER_FRAMES: u64 = 48_000 * 3_600;

/// 源节点没有可识别名字时的文件名 stem。
pub const DEFAULT_STEM: &str = "master";

/// 音频源种类（响应 `sources[].kind`）。
pub const SOURCE_KIND: &str = "midi-synth-osc";

/// 一次已校验的渲染请求（**还没有渲染**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRequest {
    /// 输出格式下标（[`FORMATS`]）。
    pub format_index: usize,
    /// 采样率（模型层枚举）。
    pub sample_rate: SampleRate,
    /// 是否做峰值归一化。
    pub normalize: bool,
    /// 显式输出路径（`None` = 按缺省规则从工程路径派生）。
    pub path: Option<PathBuf>,
}

impl RenderRequest {
    /// 输出格式字符串。
    #[must_use]
    pub fn format(&self) -> &'static str {
        FORMATS.get(self.format_index).copied().unwrap_or("wav")
    }

    /// 诚实描述这次请求（`dryRun` 与响应共用的 `request` 字段）。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "format": self.format(),
            "sampleRate": self.sample_rate.hz(),
            "normalize": self.normalize,
            "path": self.path.as_ref().map(|path| path.display().to_string()),
            "pathRule": if self.path.is_some() {
                "显式 path 参数"
            } else {
                "<工程文件 stem>.master.<format>, 与工程同目录"
            },
            "bitDepth": OUTPUT_BIT_DEPTH,
            "channels": MASTER_CHANNELS,
        })
    }

    /// 解析输出路径（确定性；`dryRun` 与真调用共用）。
    ///
    /// # Errors
    ///
    /// 显式 `path` 为空（`validate` 已拦，这里是第二道）。
    pub fn output_path(&self, project_path: &Path) -> Result<PathBuf, Fault> {
        if let Some(path) = &self.path {
            if path.as_os_str().is_empty() {
                return Err(Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`path` 不能是空字符串",
                ));
            }
            return Ok(path.clone());
        }
        let stem = project_path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .filter(|stem| !stem.is_empty())
            .unwrap_or(DEFAULT_STEM);
        Ok(store::parent_dir(project_path)
            .join(math::default_output_file_name(stem, self.format())))
    }
}

/// 校验渲染参数。
///
/// # Errors
///
/// - 未知 `format` → `INVALID_PARAMETER_RANGE`（带白名单）；
/// - `sampleRate` 不在模型层允许集合 → `INVALID_PARAMETER_RANGE`；
/// - `normalize` 不是布尔 / `path` 不是非空字符串 → `INVALID_PARAMETER_RANGE`。
pub fn validate(arguments: &Map<String, Value>) -> Result<RenderRequest, Fault> {
    let format = arguments
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let format_index = FORMATS
        .iter()
        .position(|known| *known == format)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("不支持的输出格式 `{format}`"),
                serde_json::json!({ "supportedFormats": FORMATS }),
            )
        })?;
    let sample_rate_hz = arguments
        .get("sampleRate")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`sampleRate` 必须是 32 位无符号整数",
            )
        })?;
    let sample_rate = SampleRate::from_hz(sample_rate_hz)
        .map_err(|error| super::error::from_model("采样率校验", &error))?;
    let normalize = match arguments.get("normalize") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`normalize` 必须是布尔值, 实际收到 {other}"),
            ));
        }
    };
    let path = match arguments.get("path") {
        None => None,
        Some(Value::String(text)) if !text.trim().is_empty() => Some(PathBuf::from(text)),
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`path` 必须是非空字符串, 实际收到 {other}"),
            ));
        }
    };
    Ok(RenderRequest {
        format_index,
        sample_rate,
        normalize,
        path,
    })
}

/// 渲染请求的"工程前提"检查（无活跃工程 → `NO_ACTIVE_PROJECT`）。
///
/// 单独成函数是为了让"参数校验"与"会话前提"两条判据可以分别变红。
///
/// # Errors
///
/// 没有活跃工程。
pub fn require_project(project: Option<&YebanProjectV1>) -> Result<(), Fault> {
    if project.is_none() {
        return Err(Fault::domain(
            ErrorCode::NoActiveProject,
            "没有活跃工程, 无法渲染母带",
        ));
    }
    Ok(())
}

/// 一个源节点的渲染报告（进响应 `data.sources`）。
#[derive(Clone, Debug)]
pub struct SourceReport {
    /// 路由图节点身份（= 音轨身份）。
    pub node: EntityId,
    /// 音轨名（人眼可读）。
    pub track: String,
    /// 音频源种类。
    pub kind: &'static str,
    /// 是否可闻（`mute` / `solo` 判定之后的结论）。
    pub audible: bool,
    /// 排程的音符数。
    pub notes: u64,
    /// 该源贡献的时间轴末端（tick）。
    pub end_tick: u64,
}

impl SourceReport {
    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "node": self.node.to_canonical_string(),
            "track": self.track.clone(),
            "kind": self.kind,
            "audible": self.audible,
            "notes": self.notes,
            "endTick": self.end_tick,
        })
    }
}

/// 渲染产物：**已经编码好的容器字节** + 全部实测数字。
///
/// 它由 [`build`] 产出（只读计算），由 `domain::apply` 落盘。
/// 刻意**手动实现 `Debug`**：`bytes` 有几十万字节，`#[derive(Debug)]` 会在任何
/// `{:?}` 里把日志淹掉。
pub struct RenderArtifact {
    /// 已校验的请求（响应里原样回显）。
    pub request: RenderRequest,
    /// 产物路径（原子落盘的目标）。
    pub path: PathBuf,
    /// 实际容器种类（`wav` 在超过 4 GiB 时会被 `yeban-render` 升级为 RF64）。
    pub container: ContainerKind,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 位深。
    pub bit_depth: u16,
    /// 帧数。
    pub frames: u64,
    /// 处理块数。
    pub blocks: u64,
    /// 处理块大小。
    pub block_size: usize,
    /// 头部字节数（含容器前缀与全部 chunk 头）。
    pub header_bytes: usize,
    /// 音频负载字节数（`frames * channels * bit_depth / 8`）。
    pub payload_bytes: usize,
    /// 整份文件字节（头部 + 负载 + 偶数补位）。
    pub bytes: Vec<u8>,
    /// 整份文件的 SHA-256（十六进制）。
    pub sha256: String,
    /// 母带**样本**的位级 SHA-256（对每个 `f32` 的 IEEE-754 位型）。
    pub master_digest: String,
    /// 最长延迟关键路径（帧）[ARCH-PDC-001]。
    pub longest_path_frames: u32,
    /// 渲染计划里的节点数（剪枝后）。
    pub node_count: usize,
    /// 源节点。
    pub source_nodes: Vec<EntityId>,
    /// 每个源节点的报告。
    pub sources: Vec<SourceReport>,
    /// 归一化前的峰值。
    pub peak_before: f32,
    /// 归一化后的峰值（未请求归一化时等于 `peak_before`）。
    pub peak_after: f32,
    /// 归一化是否真的改变了样本。
    pub normalize_applied: bool,
    /// 归一化的补充说明（全零信号等边界）。
    pub normalize_note: Option<&'static str>,
    /// `bext` 的 `OriginatorReference`（工程 ULID，`ARCH-FMT-001` 要求的映射）。
    pub originator_reference: String,
    /// `bext` 的 `OriginationDate`。
    pub origination_date: String,
    /// `bext` 的 `OriginationTime`。
    pub origination_time: String,
    /// `bext` 的 `CodingHistory`（写进文件的那一份）。
    pub coding_history: String,
    /// 工程内容摘要（渲染输入的指纹，用于溯源）。
    pub project_digest: String,
    /// 渲染时刻（注入时钟的 Unix 毫秒）。
    pub rendered_at_unix_ms: u64,
    /// 明确没渲染的部分（键序字典序，来自 `BTreeSet`）。
    pub unsupported: BTreeSet<&'static str>,
    /// 每个"没渲染"部分的出现次数。
    pub unsupported_counts: BTreeMap<&'static str, u64>,
}

impl core::fmt::Debug for RenderArtifact {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderArtifact")
            .field("path", &self.path)
            .field("container", &self.container)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames)
            .field("blocks", &self.blocks)
            .field("header_bytes", &self.header_bytes)
            .field("payload_bytes", &self.payload_bytes)
            .field("file_bytes", &self.bytes.len())
            .field("sha256", &self.sha256)
            .field("unsupported", &self.unsupported)
            .finish_non_exhaustive()
    }
}

impl RenderArtifact {
    /// 容器种类的规范字符串。
    #[must_use]
    pub const fn container_name(&self) -> &'static str {
        match self.container {
            ContainerKind::Riff => "RIFF",
            ContainerKind::Rf64 => "RF64",
            ContainerKind::Bw64 => "BW64",
        }
    }

    /// 实测数字（`dryRun` 预览与真调用响应共用的主体）。
    #[must_use]
    pub fn details(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert(
            "path".to_owned(),
            Value::from(self.path.display().to_string()),
        );
        map.insert("format".to_owned(), Value::from(self.request.format()));
        map.insert("container".to_owned(), Value::from(self.container_name()));
        map.insert("sampleRate".to_owned(), Value::from(self.sample_rate));
        map.insert("channels".to_owned(), Value::from(self.channels));
        map.insert("bitDepth".to_owned(), Value::from(self.bit_depth));
        map.insert("frames".to_owned(), Value::from(self.frames));
        map.insert(
            "durationSeconds".to_owned(),
            Value::from(self.frames as f64 / f64::from(self.sample_rate)),
        );
        map.insert("blocks".to_owned(), Value::from(self.blocks));
        map.insert("blockSize".to_owned(), Value::from(self.block_size));
        map.insert("bytes".to_owned(), Value::from(self.bytes.len()));
        map.insert("headerBytes".to_owned(), Value::from(self.header_bytes));
        map.insert("payloadBytes".to_owned(), Value::from(self.payload_bytes));
        map.insert("sha256".to_owned(), Value::from(self.sha256.clone()));
        map.insert(
            "masterDigest".to_owned(),
            Value::from(self.master_digest.clone()),
        );
        map.insert(
            "longestPathFrames".to_owned(),
            Value::from(self.longest_path_frames),
        );
        map.insert(
            "latencySource".to_owned(),
            Value::from("DeviceDefinition::latency_samples (track_latencies)"),
        );
        map.insert("nodeCount".to_owned(), Value::from(self.node_count));
        map.insert(
            "sourceNodes".to_owned(),
            Value::Array(
                self.source_nodes
                    .iter()
                    .map(|node| Value::from(node.to_canonical_string()))
                    .collect(),
            ),
        );
        map.insert(
            "sources".to_owned(),
            Value::Array(self.sources.iter().map(SourceReport::to_value).collect()),
        );
        map.insert(
            "peak".to_owned(),
            serde_json::json!({
                "before": self.peak_before,
                "after": self.peak_after,
                "target": NORMALIZE_TARGET,
                "unit": "full-scale",
            }),
        );
        map.insert(
            "normalize".to_owned(),
            serde_json::json!({
                "requested": self.request.normalize,
                "applied": self.normalize_applied,
                "target": NORMALIZE_TARGET,
                "domain": "抖动/量化之前的浮点母带",
                "note": self.normalize_note,
            }),
        );
        map.insert("atomic".to_owned(), Value::from(true));
        map.insert(
            "strategy".to_owned(),
            Value::from("同目录 .tmp-{ulid} + File::sync_all + rename (ARCH-SEC-004)"),
        );
        map.insert("dither".to_owned(), Value::from("TPDF (ARCH-FMT-001)"));
        map.insert(
            "bwf".to_owned(),
            serde_json::json!({
                "originator": "Yeban DAW",
                "originatorReference": self.originator_reference.clone(),
                "originationDate": self.origination_date.clone(),
                "originationTime": self.origination_time.clone(),
                "codingHistory": self.coding_history.clone(),
                // EBU R128 响度没有测量 ⇒ 写哨兵 `UNKNOWN`, 绝不写一个假的 LUFS 读数。
                "loudness": "unknown (bext v2 哨兵)",
            }),
        );
        map.insert(
            "unsupported".to_owned(),
            Value::Array(
                self.unsupported
                    .iter()
                    .map(|key| Value::from(*key))
                    .collect(),
            ),
        );
        map.insert(
            "unsupportedCounts".to_owned(),
            Value::Object(
                self.unsupported_counts
                    .iter()
                    .map(|(key, count)| ((*key).to_owned(), Value::from(*count)))
                    .collect(),
            ),
        );
        map.insert(
            "projectDigest".to_owned(),
            Value::from(self.project_digest.clone()),
        );
        map.insert(
            "renderedAtUnixMs".to_owned(),
            Value::from(self.rendered_at_unix_ms),
        );
        map
    }

    /// `dryRun` 预览：同一批实测数字 + `wouldWrite`（**不落盘**）。
    #[must_use]
    pub fn preview(&self) -> Value {
        let mut map = self.details();
        map.insert("renderer".to_owned(), Value::from("yeban-render"));
        map.insert("wired".to_owned(), Value::from(true));
        map.insert("writesFile".to_owned(), Value::from(true));
        map.insert("request".to_owned(), self.request.to_value());
        map.insert(
            "wouldWrite".to_owned(),
            serde_json::json!({
                "path": self.path.display().to_string(),
                "format": self.request.format(),
                "container": self.container_name(),
                "frames": self.frames,
                "bytes": self.bytes.len(),
                "sha256": self.sha256.clone(),
            }),
        );
        Value::Object(map)
    }

    /// 真调用响应的 `data`。
    #[must_use]
    pub fn response_data(&self) -> Value {
        let mut map = self.details();
        map.insert("rendered".to_owned(), Value::from(true));
        map.insert("request".to_owned(), self.request.to_value());
        Value::Object(map)
    }
}

/// 从工程构造一份**已编码**的母带产物（只读、不落盘）。
///
/// 这是本模块唯一的入口：`plan_render_master` 调它拿 [`RenderArtifact`]，
/// `domain::apply` 只负责把 `artifact.bytes` 原子写到 `artifact.path`。
///
/// # Errors
///
/// - `RENDER_FAILED`：采样率不一致（重采样未接线）、0 帧、超出帧数上限、
///   路由图非法、源节点没有对应音轨、摆放引用了不存在的片段、容器编码失败；
/// - `INVALID_PARAMETER_RANGE`：输出路径是工程文件或锁文件。
pub fn build(
    project: &YebanProjectV1,
    project_path: &Path,
    request: &RenderRequest,
    now_ms: u64,
) -> Result<RenderArtifact, Fault> {
    let sample_rate = request.sample_rate.hz();
    let project_rate = project.audio_config.sample_rate.hz();
    // 1. 采样率必须与工程一致: 重采样器 (ARCH-DSP-002 / D26) 未接线, 猜一个会产出
    //    音高错误的母带 —— 那是"假成功", 必须报错。
    if sample_rate != project_rate {
        return Err(render_failed(
            "输出采样率与工程采样率不一致, 而重采样器尚未接线 (ARCH-DSP-002)",
            serde_json::json!({
                "requestedSampleRate": sample_rate,
                "projectSampleRate": project_rate,
                "unwired": "resampler",
                "specId": "ARCH-DSP-002",
            }),
        ));
    }
    // 2. 输出路径 + 两条安全护栏。
    let path = request.output_path(project_path)?;
    guard_output_path(&path, project_path)?;
    // 3. 延迟表: 唯一来源是 DeviceDefinition::latency_samples [ARCH-PDC-001]。
    let latencies = track_latencies(&project.tracks);
    // 4. 探针编译: 用 frames=1 取得"剪枝后的可达子图 + 源节点集合"。
    //    刻意复用渲染器的剪枝/分层逻辑, 而不是在这里复制一份图算法(两份必然漂移)。
    let probe = RenderOptions::l1(1, MASTER_CHANNELS, sample_rate, project.rng_seed);
    let probe_plan = RenderPlan::compile_with_latencies(
        &project.routing_graph,
        project.master_bus_track_id,
        probe,
        &latencies,
    )
    .map_err(|error| render_error("渲染计划探针编译", &error))?;
    let nodes = probe_plan.nodes();
    let source_nodes: Vec<EntityId> = nodes
        .iter()
        .copied()
        .filter(|node| is_source_node(&probe_plan, *node))
        .collect();
    // 5. 组装音源 + 采集"明确没渲染"的部分 + 得到时间轴末端。
    let mut unsupported: BTreeSet<&'static str> = BTreeSet::new();
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let any_solo = project.tracks.values().any(|track| track.solo);
    if project
        .routing_graph
        .edges
        .values()
        .any(|edge| edge.kind == RoutingKind::Sidechain)
    {
        note_unsupported(&mut unsupported, &mut counts, "sidechainRouting");
    }
    let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let mut reports: Vec<SourceReport> = Vec::new();
    let mut end_tick = 0u64;
    for &node in &source_nodes {
        let track = project.tracks.get(&node).ok_or_else(|| {
            render_failed(
                "路由图里的源节点没有对应的音轨, 无法知道该渲染什么",
                serde_json::json!({ "node": node.to_canonical_string() }),
            )
        })?;
        let (source, report) = build_source(
            project,
            track,
            sample_rate,
            any_solo,
            &mut unsupported,
            &mut counts,
        )?;
        end_tick = end_tick.max(report.end_tick);
        reports.push(report);
        // 显式标注类型: 让 `Box<MidiSynthSource> -> Box<dyn AudioSource>` 的强制转换
        // 出现在插槽类型已知的位置, 而不是依赖 `insert` 的参数推断。
        let source: Box<dyn AudioSource> = Box::new(source);
        registry.insert(node, source);
    }
    // 6. 非源节点(总线/主轨)上的片段: 不渲染, 但**明确登记**, 不静默丢。
    for &node in &nodes {
        if source_nodes.contains(&node) {
            continue;
        }
        if let Some(track) = project.tracks.get(&node)
            && audible_end_tick(track).is_some()
        {
            note_unsupported(&mut unsupported, &mut counts, "busTrackClips");
        }
    }
    // 7. 帧数: 由时间轴末端与工程量纲唯一决定。
    let frames = math::ticks_to_frames(end_tick, PPQ, project.bpm, sample_rate);
    if frames == 0 {
        return Err(render_failed(
            "工程里没有可渲染的 MIDI 内容 (0 帧)",
            serde_json::json!({
                "endTick": end_tick,
                "sourceNodes": source_nodes
                    .iter()
                    .map(|node| node.to_canonical_string())
                    .collect::<Vec<_>>(),
                "hint": "需要至少一个未静音的 MIDI 片段摆放 (ClipContent::Midi)",
            }),
        ));
    }
    if frames > MAX_RENDER_FRAMES {
        return Err(render_failed(
            "渲染帧数超出单次上限 (1 小时 @ 48 kHz)",
            serde_json::json!({ "frames": frames, "maxFrames": MAX_RENDER_FRAMES }),
        ));
    }
    // 8. 真编译 + 真执行。
    let options = RenderOptions::l1(frames, MASTER_CHANNELS, sample_rate, project.rng_seed);
    let mut plan = RenderPlan::compile_with_latencies(
        &project.routing_graph,
        project.master_bus_track_id,
        options,
        &latencies,
    )
    .map_err(|error| render_error("渲染计划编译", &error))?;
    let mut output = plan
        .execute(registry)
        .map_err(|error| render_error("渲染执行", &error))?;
    // 9. Master 轨增益 (在源侧无法表达: 它是总线节点)。pan 无法应用 ⇒ 登记。
    if let Some(master) = project.tracks.get(&project.master_bus_track_id) {
        let master_gain = if master.mute {
            0.0
        } else {
            db_to_linear(master.volume_db)
        };
        if master_gain != 1.0 {
            math::scale_in_place(&mut output.samples, master_gain);
        }
        if master.pan != 0.0 {
            note_unsupported(&mut unsupported, &mut counts, "masterPan");
        }
    }
    // 10. 峰值归一化(抖动之前)。
    let peak_before = math::peak_of(&output.samples);
    let mut normalize_applied = false;
    let mut normalize_note = None;
    if request.normalize {
        if peak_before > 0.0 {
            let gain = math::normalize_gain(peak_before, NORMALIZE_TARGET);
            math::scale_in_place(&mut output.samples, gain);
            normalize_applied = true;
        } else {
            normalize_note = Some("全零信号: 峰值为 0, 归一化不改动任何样本");
        }
    }
    let peak_after = math::peak_of(&output.samples);
    // 11. TPDF 抖动 → 24-bit PCM → 容器。
    let mut rng = dither_rng_for(project.rng_seed, project.master_bus_track_id);
    let pcm = quantize(&output.samples, BitDepth::Int24, &mut rng);
    let payload = pcm.to_le_bytes();
    let (origination_date, origination_time) = math::bext_stamp(now_ms);
    let originator_reference = project.id.to_canonical_string();
    let coding_history = format!("A=PCM,F={sample_rate},W={OUTPUT_BIT_DEPTH},M=stereo,T=Yeban-MCP");
    let mut bext = Bext::for_project(&originator_reference, &origination_date, &origination_time);
    // `Bext::for_project` 的 coding history 是模板字符串(带 `<sample_rate>` 字面量);
    // 母带导出必须写**真实**参数, 否则那是文件里的假元数据。
    bext.coding_history.clone_from(&coding_history);
    let preferred = match request.format() {
        "rf64" => ContainerKind::Rf64,
        "bw64" => ContainerKind::Bw64,
        _ => ContainerKind::Riff,
    };
    let container_plan = ContainerPlan::for_payload(
        preferred,
        PcmFormat::integer(
            u16::try_from(MASTER_CHANNELS).unwrap_or(u16::MAX),
            sample_rate,
            OUTPUT_BIT_DEPTH,
        ),
        payload.len() as u64,
        frames,
        Some(bext),
    );
    let header_bytes = container_plan.header_bytes().len();
    let mut bytes = Vec::with_capacity(header_bytes + payload.len() + 1);
    write_container(&mut bytes, &container_plan, &payload).map_err(|error| {
        render_failed(
            "容器编码失败",
            serde_json::json!({ "rf64Error": format!("{error:?}") }),
        )
    })?;
    let project_digest = store::digest_of(store::serialize_project(project)?.as_bytes());
    Ok(RenderArtifact {
        request: request.clone(),
        path,
        container: container_plan.kind,
        sample_rate,
        channels: MASTER_CHANNELS,
        bit_depth: OUTPUT_BIT_DEPTH,
        frames: output.frames,
        blocks: output.blocks,
        block_size: L1_BLOCK_SIZE,
        header_bytes,
        payload_bytes: payload.len(),
        sha256: AssetHash::of_bytes(&bytes).as_str().to_owned(),
        master_digest: hex(&output.digest),
        longest_path_frames: output.longest_path_frames,
        bytes,
        node_count: nodes.len(),
        source_nodes,
        sources: reports,
        peak_before,
        peak_after,
        normalize_applied,
        normalize_note,
        originator_reference,
        origination_date,
        origination_time,
        coding_history,
        project_digest,
        rendered_at_unix_ms: now_ms,
        unsupported,
        unsupported_counts: counts,
    })
}

/// 一个保留节点是不是**源节点**（没有入边）。
///
/// `bus_reduction_order` 对源节点返回 `Some(vec![])`（空入边列表），对总线返回非空列表。
fn is_source_node(plan: &RenderPlan, node: EntityId) -> bool {
    plan.bus_reduction_order(node)
        .is_none_or(|inputs| inputs.is_empty())
}

/// 音轨上**可闻内容**的时间轴末端（没有内容时为 `None`）。
///
/// 只做"有没有/到哪"的判定，不做排程 —— 它服务于"总线轨上的片段必须被登记为
/// 不支持"这一条，以及防止"没有内容却渲染出 0 帧"。
fn audible_end_tick(track: &TrackV3) -> Option<u64> {
    let mut end: Option<u64> = None;
    for placement in track.clips.values() {
        if placement.muted {
            continue;
        }
        let candidate = placement
            .start_tick
            .saturating_add(placement.duration_ticks);
        end = Some(end.map_or(candidate, |current| current.max(candidate)));
    }
    end
}

/// 一个已排程的音符（发音所需的全部信息，与模型层解耦）。
///
/// 声相/音轨增益**不在**这里：一个源就是一条音轨，因此这两个量在整个源上恒定，
/// 放在 [`MidiSynthSource`] 上（每音符复制一遍只会制造漂移面）。
#[derive(Clone, Copy, Debug)]
struct ScheduledNote {
    /// 起始帧（含）。
    start: i64,
    /// 结束帧（不含）。
    end: i64,
    /// 频率 (Hz)。
    frequency: f32,
    /// 幅度（力度 / 127）。
    amplitude: f32,
}

/// 确定性 MIDI 合成源：**纯函数式**振荡器（相位由绝对帧号算出，不做累加）。
///
/// 相位 `= (global_frame - note_start) * frequency / sample_rate`，
/// 因此输出只依赖**绝对帧号**，与块边界、块大小、线程数完全无关 ——
/// 这比"在块之间累加相位"更强：累加只要错一次（丢块/重复块）就会漂移。
struct MidiSynthSource {
    notes: Vec<ScheduledNote>,
    /// 左声道增益（音轨音量 × 等功率声相）。
    gain_l: f32,
    /// 右声道增益。
    gain_r: f32,
    attack_frames: i64,
    release_frames: i64,
}

impl AudioSource for MidiSynthSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        out.fill(0.0);
        let channels = context.channels;
        if channels == 0 || self.notes.is_empty() {
            return Ok(());
        }
        let sample_rate = f64::from(context.sample_rate);
        if sample_rate <= 0.0 {
            return Ok(());
        }
        for index in 0..context.frames {
            let global = i64::try_from(context.first_frame + index as u64).unwrap_or(i64::MAX);
            let mut value = 0.0f32;
            for note in &self.notes {
                if global < note.start || global >= note.end {
                    continue;
                }
                let offset = global - note.start;
                let envelope = math::envelope(
                    offset,
                    note.end - note.start,
                    self.attack_frames,
                    self.release_frames,
                );
                if envelope <= 0.0 {
                    continue;
                }
                let phase = (offset as f64) * f64::from(note.frequency) / sample_rate;
                let fraction = (phase - phase.floor()) as f32;
                value += note.amplitude * envelope * libm::sinf(fraction * core::f32::consts::TAU);
            }
            if value == 0.0 {
                continue;
            }
            let base = index * channels;
            for (channel, slot) in out[base..base + channels].iter_mut().enumerate() {
                let gain = match channel {
                    0 => self.gain_l,
                    1 => self.gain_r,
                    _ => 0.0,
                };
                *slot += value * gain;
            }
        }
        Ok(())
    }
}

/// 由一条音轨构造音频源 + 报告，并把"没渲染的部分"登记进 `unsupported`。
fn build_source(
    project: &YebanProjectV1,
    track: &TrackV3,
    sample_rate: u32,
    any_solo: bool,
    unsupported: &mut BTreeSet<&'static str>,
    counts: &mut BTreeMap<&'static str, u64>,
) -> Result<(MidiSynthSource, SourceReport), Fault> {
    // solo 语义（只作用于源轨；辅助返回总线不参与判定 —— 登记在 notes 的边界里）:
    // 只要工程里有任一音轨 solo, 未 solo 且非 solo-safe 的源轨就不发声。
    let audible = !track.mute && (!any_solo || track.solo || track.solo_safe);
    let gain = if audible {
        db_to_linear(track.volume_db)
    } else {
        0.0
    };
    let (gain_l, gain_r) = if track.pan.is_finite() && track.pan != 0.0 {
        let (left, right) = pan_gains(track.pan);
        (gain * left, gain * right)
    } else {
        (gain, gain)
    };
    // 设备链: 只有 latency_samples 进了 PDC; 参数求值没有实现 —— 出现即登记。
    if track.devices.iter().any(|device| !device.bypassed) {
        note_unsupported(unsupported, counts, "deviceChainDsp");
    }
    if track.devices.iter().any(|device| {
        !device.bypassed
            && matches!(
                device.kind,
                DeviceKind::ExternalInstrument | DeviceKind::ExternalEffect
            )
    }) {
        note_unsupported(unsupported, counts, "externalPlugins");
    }
    if !track.automation_lanes.is_empty() {
        note_unsupported(unsupported, counts, "automationLanes");
    }
    let attack_frames = math::ms_to_frames(ATTACK_MS, sample_rate);
    let release_frames = math::ms_to_frames(RELEASE_MS, sample_rate);
    let mut scheduled: Vec<ScheduledNote> = Vec::new();
    let mut end_tick = 0u64;
    for placement in track.clips.values() {
        if placement.muted {
            continue;
        }
        let clip = project.clip_pool.get(&placement.clip_id).ok_or_else(|| {
            render_failed(
                "摆放引用了片段池里不存在的片段 (工程结构已失效)",
                serde_json::json!({
                    "placementId": placement.id.to_canonical_string(),
                    "clipId": placement.clip_id.to_canonical_string(),
                }),
            )
        })?;
        end_tick = end_tick.max(
            placement
                .start_tick
                .saturating_add(placement.duration_ticks),
        );
        if placement.loop_config.enabled
            && placement.loop_config.end_tick > placement.duration_ticks
        {
            note_unsupported(unsupported, counts, "clipLoopRepetition");
        }
        let ClipContent::Midi { notes } = &clip.content else {
            note_unsupported(unsupported, counts, "audioClips");
            continue;
        };
        for note in notes.values() {
            if note
                .probability
                .is_some_and(|probability| probability < 1.0)
            {
                note_unsupported(unsupported, counts, "noteProbability");
            }
            if note.ratchet.is_some_and(|ratchet| ratchet > 1) {
                note_unsupported(unsupported, counts, "noteRatchet");
            }
            if note.slide.is_some() {
                note_unsupported(unsupported, counts, "noteSlide");
            }
            if !note.pitch_bend_curve.is_empty() {
                note_unsupported(unsupported, counts, "notePitchBend");
            }
            if note.syllable.is_some() || !note.phonemes.is_empty() {
                note_unsupported(unsupported, counts, "noteLyrics");
            }
            let micro = i64::from(note.micro_timing_ticks.unwrap_or(0));
            let (start, end) = math::note_frame_span(
                placement.start_tick.saturating_add(note.start_tick),
                note.duration_ticks,
                micro,
                PPQ,
                project.bpm,
                sample_rate,
            );
            scheduled.push(ScheduledNote {
                start,
                end,
                frequency: pitch_to_hz(note.pitch),
                amplitude: f32::from(note.velocity) / 127.0,
            });
            let shifted_micro = u64::try_from(micro.max(0)).unwrap_or(0);
            end_tick = end_tick.max(
                placement
                    .start_tick
                    .saturating_add(note.start_tick)
                    .saturating_add(shifted_micro)
                    .saturating_add(note.duration_ticks),
            );
        }
    }
    // 浮点求和不满足结合律 [ARCH-DET-002]: 排程顺序必须是**全序**且与输入顺序无关。
    // 键取到"两个音符完全相同则顺序无影响"的深度即可。
    scheduled.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(a.end.cmp(&b.end))
            .then(a.frequency.to_bits().cmp(&b.frequency.to_bits()))
            .then(a.amplitude.to_bits().cmp(&b.amplitude.to_bits()))
    });
    let report = SourceReport {
        node: track.id,
        track: track.name.clone(),
        kind: SOURCE_KIND,
        audible,
        notes: u64::try_from(scheduled.len()).unwrap_or(u64::MAX),
        end_tick,
    };
    Ok((
        MidiSynthSource {
            notes: scheduled,
            gain_l,
            gain_r,
            attack_frames,
            release_frames,
        },
        report,
    ))
}

/// 等程律频率：`440 * 2^((pitch - 69) / 12)`，走 `libm::powf` [ARCH-DET-001]。
fn pitch_to_hz(pitch: u8) -> f32 {
    440.0 * libm::powf(2.0, (f32::from(pitch) - 69.0) / 12.0)
}

/// 等功率（−3 dB）声相增益：`θ = (pan + 1) * π/4`，`(cos θ, sin θ)`。
///
/// 与模型层 `PanLaw::ConstantPowerMinus3dB` 的口径一致：居中的 `pan = 0`
/// 给出 `(√2/2, √2/2)`（即每个声道 −3 dB）。
fn pan_gains(pan: f32) -> (f32, f32) {
    let clamped = pan.clamp(-1.0, 1.0);
    let angle = (clamped + 1.0) * core::f32::consts::FRAC_PI_4;
    (libm::cosf(angle), libm::sinf(angle))
}

/// 登记一条"明确没渲染"的部分（键 + 出现次数）。
fn note_unsupported(
    unsupported: &mut BTreeSet<&'static str>,
    counts: &mut BTreeMap<&'static str, u64>,
    key: &'static str,
) {
    unsupported.insert(key);
    let counter = counts.entry(key).or_insert(0);
    *counter = counter.saturating_add(1);
}

/// `[u8; 32]` → 64 位小写十六进制。
fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `RENDER_FAILED` 的简写（带结构化原因，便于调用方自纠）。
fn render_failed(message: impl Into<String>, data: Value) -> Fault {
    Fault::domain_with_data(ErrorCode::RenderFailed, message, data)
}

/// [`RenderError`] → `RENDER_FAILED`。
fn render_error(context: &str, error: &RenderError) -> Fault {
    render_failed(
        format!("{context}: {error}"),
        serde_json::json!({
            "renderError": format!("{error:?}"),
            "context": context,
            "specId": "ROAD-M4-004",
        }),
    )
}

/// 输出路径的两条安全护栏：不得是工程文件本身、不得是 `.yeban.lock`。
fn guard_output_path(output: &Path, project_path: &Path) -> Result<(), Fault> {
    if same_path(output, project_path) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "输出路径不能是工程文件本身 (会毁掉工程)",
            serde_json::json!({
                "outputPath": output.display().to_string(),
                "projectPath": project_path.display().to_string(),
            }),
        ));
    }
    let lock = store::lock_path(project_path);
    if same_path(output, &lock) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "输出路径不能是工程的锁文件 (.yeban.lock)",
            serde_json::json!({
                "outputPath": output.display().to_string(),
                "lockFile": lock.display().to_string(),
            }),
        ));
    }
    Ok(())
}

/// 两个路径是否指向同一个文件（存在时用 `canonicalize`，否则比字面量）。
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn unknown_format_and_sample_rate_are_invalid_parameter_range() {
        let fault = validate(&args(
            serde_json::json!({"format": "mp3", "sampleRate": 48000}),
        ))
        .expect_err("未知格式");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "INVALID_PARAMETER_RANGE");
        assert!(value["error"]["data"]["supportedFormats"].is_array());

        let fault = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 12345}),
        ))
        .expect_err("采样率集合外");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));

        let fault = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": -1}),
        ))
        .expect_err("负数采样率");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
    }

    #[test]
    fn valid_arguments_produce_a_checked_request() {
        let request = validate(&args(
            serde_json::json!({"format": "rf64", "sampleRate": 96000, "normalize": true}),
        ))
        .expect("合法");
        assert_eq!(request.format(), "rf64");
        assert_eq!(request.sample_rate.hz(), 96000);
        assert!(request.normalize);
        assert_eq!(request.path, None);
        // normalize 缺省为 false。
        let default = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 44100}),
        ))
        .expect("合法");
        assert!(!default.normalize);
        assert_eq!(default.to_value()["format"], "wav");
    }

    #[test]
    fn explicit_path_is_validated_and_used_verbatim() {
        let request = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": "/tmp/x.wav"}),
        ))
        .expect("合法");
        assert_eq!(request.path, Some(PathBuf::from("/tmp/x.wav")));
        assert_eq!(
            request
                .output_path(Path::new("/tmp/demo.yeban"))
                .expect("路径"),
            PathBuf::from("/tmp/x.wav")
        );
        for bad in [
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": ""}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": "   "}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": 7}),
        ] {
            let fault = validate(&args(bad.clone())).expect_err("坏 path");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{bad}"
            );
        }
    }

    #[test]
    fn default_output_path_follows_the_documented_rule() {
        let request = validate(&args(
            serde_json::json!({"format": "rf64", "sampleRate": 48000}),
        ))
        .expect("合法");
        assert_eq!(
            request
                .output_path(Path::new("/music/demo.yeban"))
                .expect("路径"),
            PathBuf::from("/music/demo.master.rf64")
        );
        // 没有扩展名时 stem 就是整个文件名。
        assert_eq!(
            request
                .output_path(Path::new("/music/no-extension"))
                .expect("路径"),
            PathBuf::from("/music/no-extension.master.rf64")
        );
        // 连 stem 都没有（根目录）时回退到 DEFAULT_STEM；此时 `parent_dir` 也在 `"."`。
        assert_eq!(
            request.output_path(Path::new("/")).expect("路径"),
            PathBuf::from("./master.master.rf64")
        );
    }

    #[test]
    fn output_path_may_not_be_the_project_or_the_lock_file() {
        let project = Path::new("/music/demo.yeban");
        let fault = guard_output_path(project, project).expect_err("不得覆盖工程");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let lock = PathBuf::from(format!("{}.lock", project.display()));
        let fault = guard_output_path(&lock, project).expect_err("不得覆盖锁文件");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        guard_output_path(Path::new("/music/demo.master.wav"), project).expect("正常路径");
    }

    #[test]
    fn pitch_and_pan_follow_the_model_conventions() {
        assert!((pitch_to_hz(69) - 440.0).abs() < 1.0e-3, "A4 = 440 Hz");
        assert!((pitch_to_hz(81) - 880.0).abs() < 1.0e-2, "A5 = 880 Hz");
        assert!((pitch_to_hz(57) - 220.0).abs() < 1.0e-2, "A3 = 220 Hz");
        let (left, right) = pan_gains(0.0);
        assert!((left - right).abs() < 1.0e-6, "居中必须对称");
        assert!((left - core::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-6);
        let (left, right) = pan_gains(-1.0);
        assert!((left - 1.0).abs() < 1.0e-6 && right.abs() < 1.0e-6, "全左");
        let (left, right) = pan_gains(1.0);
        assert!((right - 1.0).abs() < 1.0e-6 && left.abs() < 1.0e-6, "全右");
    }

    #[test]
    fn unsupported_keys_are_counted_and_ordered() {
        let mut set = BTreeSet::new();
        let mut counts = BTreeMap::new();
        note_unsupported(&mut set, &mut counts, "audioClips");
        note_unsupported(&mut set, &mut counts, "audioClips");
        note_unsupported(&mut set, &mut counts, "deviceChainDsp");
        assert_eq!(
            set.iter().copied().collect::<Vec<_>>(),
            vec!["audioClips", "deviceChainDsp"],
            "键序必须是字典序 (BTreeSet)"
        );
        assert_eq!(counts.get("audioClips"), Some(&2));
        assert_eq!(counts.get("deviceChainDsp"), Some(&1));
    }

    #[test]
    fn missing_project_is_no_active_project() {
        let fault = require_project(None).expect_err("没有工程");
        assert_eq!(fault.domain_code(), Some(ErrorCode::NoActiveProject));
        let project = YebanProjectV1::default();
        require_project(Some(&project)).expect("有工程");
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        let mut digest = [0u8; 32];
        digest[0] = 0xAB;
        digest[31] = 0x01;
        let text = hex(&digest);
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("ab"));
        assert!(text.ends_with("01"));
        assert!(
            text.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn audible_end_tick_ignores_muted_placements() {
        let mut track = TrackV3::default();
        assert_eq!(audible_end_tick(&track), None);
        let id = EntityId::new();
        track.clips.insert(
            id,
            yeban_model::ClipPlacement {
                id,
                clip_id: EntityId::new(),
                start_tick: 960,
                duration_ticks: 480,
                muted: true,
                ..yeban_model::ClipPlacement::default()
            },
        );
        assert_eq!(audible_end_tick(&track), None, "静音摆放不算内容");
        let live = EntityId::new();
        track.clips.insert(
            live,
            yeban_model::ClipPlacement {
                id: live,
                clip_id: EntityId::new(),
                start_tick: 1_920,
                duration_ticks: 960,
                ..yeban_model::ClipPlacement::default()
            },
        );
        assert_eq!(audible_end_tick(&track), Some(2_880));
    }
}
