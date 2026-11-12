//! `--export-stems <dir>` —— 分轨导出的**批处理**出口。
//!
//! ## 它补的是哪一格（这是本模块存在的全部理由）
//!
//! `yeban_export_stems`（`ADR-0030`）早就一次调用把每条轨弹成一个 32-bit Float
//! WAV，但它只住在 MCP 里：要取分轨必须起 GUI 挂 `xvfb`、连上控制面、一次调一条。
//! 9 轨工程的分轨因此是一条条手工拿的。
//!
//! 本模块把**同一份**后端（`yeban_mcp::domain::stems`，`N + 1` 次渲染、逐轨
//! 32-bit Float、序号前缀文件名）接到批处理路径上：一次
//! `yeban-app --open X.yeban --export-stems stems/`，不开窗口、不建 MCP 会话、
//! 不碰 `~/.yeban` 的令牌。
//!
//! ## 口径（全是后端的既有口径，本模块不另立一张表）
//!
//! * 产物：每条**源节点**轨一个 WAV（32-bit Float，工程采样率）；文件名
//!   `{序号}.{清洗后的轨名}.wav`（序号最小 4 位，`stem_file_name` 的规则；
//!   保留 Unicode 字母/数字与 `._-`（汉字原样保留），其余逐个换 `_`，
//!   总长封顶 255 字节）。
//! * 主总线轨与非源节点轨（总线/分组轨）**不出文件**，但在 `skippedTracks` 里
//!   点名（⛔ 不静默丢轨，D4）；回执里每条跳过的轨各有一行 `stem-skipped:`。
//! * 输出目录**必须已存在且是目录**（`stems::validate` 的规则，⛔ 不自动创建：
//!   分轨是写一堆文件，目录建错位置比单文件更贵）。
//! * 目标文件已存在 ⇒ 整批在渲染之前就拒绝（⛔ 不覆盖、不跳过、不改名）。
//! * 落盘逐条原子；第 k 条失败时前 k−1 条**保留不删**，回 `IO_ERROR` +
//!   `errorKind:"stemWriteFailed"`，`data` 里给 `completedStems` /
//!   `failedTrackId` / `orphanedFiles` / `resumeFrom`（D5 的部分成功是一等状态）。
//! * 渲染失败 ⇒ `RENDER_FAILED`；`trackIds` 点名不存在的轨 ⇒ `TRACK_NOT_FOUND`。
//!   本开关不设 `trackIds` 实参：导出的恒是**全部**非 master 源节点轨。
//!
//! ## 工程路径
//!
//! `stems::export` 要一份工程文件路径（SFZ 相对路径的解析根＋输出护栏的比对
//! 基准）：`--open` 时取那个文件；样本工程（没有 `--open`）时取当前工作目录
//! （与 `--export-wav` 的"样本形态解析从仓库根开始"的旧口径同档，只是这里
//! 落到 `Path` 上）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::stems::{self, StemBatch};
use yeban_model::{AssetHash, YebanProjectV1};

/// 回执前缀（稳定前缀，判据与脚本据此切片）。
pub const RECEIPT_PREFIX: &str = "exported-stems:";
/// 每条已写分轨的回执前缀。
pub const STEM_LINE_PREFIX: &str = "stem:";
/// 每条跳过轨的回执前缀。
pub const SKIPPED_LINE_PREFIX: &str = "stem-skipped:";

/// 一条已写分轨的实测读数（回执行的数据来源：`StemRow::to_value` 的同一批字段）。
#[derive(Debug)]
pub struct StemLine {
    /// 轨名（原样回显）。
    pub track: String,
    /// 落点文件名（`{序号}.{清洗后的轨名}.wav`）。
    pub file_name: String,
    /// 落点完整路径。
    pub path: PathBuf,
    /// 文件字节数。
    pub bytes: usize,
    /// 位深（恒为 32，`render::STEM_BIT_DEPTH`）。
    pub bit_depth: u16,
    /// 声道数。
    pub channels: u16,
}

/// 一条跳过轨的实测读数（`SkippedTrack::to_value` 的同一批字段）。
#[derive(Debug)]
pub struct SkippedLine {
    /// 轨名（原样回显）。
    pub track: String,
    /// 跳过原因（`masterBusTrack` / `notASourceNode`）。
    pub reason: &'static str,
}

/// 一次分轨导出的实测读数（回执行的**唯一**来源：⛔ 不从入参抄）。
#[derive(Debug)]
pub struct ExportStemsReport {
    /// 输出目录（原样回显）。
    pub dir: PathBuf,
    /// 已写的分轨（工程内轨顺序）。
    pub stems: Vec<StemLine>,
    /// 跳过的轨（主总线 + 非源节点轨）。
    pub skipped: Vec<SkippedLine>,
    /// 产物字节合计。
    pub bytes_total: usize,
    /// 渲染次数（实测：基线 1 次 + 每条分轨 1 次）。
    pub renders: usize,
    /// 基线渲染的帧数（每条分轨被钉死的帧数）。
    pub basis_frames: u64,
    /// 采样率（Hz，工程采样率）。
    pub sample_rate: u32,
}

impl ExportStemsReport {
    /// 回执行：**一行汇总** + 每条分轨一行 + 每条跳过轨一行。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{RECEIPT_PREFIX} dir={} stems={} skipped={} bytes={} renders={} sampleRate={} bitDepth=32",
            self.dir.display(),
            self.stems.len(),
            self.skipped.len(),
            self.bytes_total,
            self.renders,
            self.sample_rate,
        )];
        for stem in &self.stems {
            lines.push(format!(
                "{STEM_LINE_PREFIX} path={} track={} bytes={} bitDepth={} channels={}",
                stem.path.display(),
                stem.track,
                stem.bytes,
                stem.bit_depth,
                stem.channels,
            ));
        }
        for skipped in &self.skipped {
            lines.push(format!(
                "{SKIPPED_LINE_PREFIX} track={} reason={}",
                skipped.track, skipped.reason,
            ));
        }
        lines
    }
}

/// `--export-stems` 的失败（退出码由 `cli::CliError` 统一给：都是 `EXIT_EXPORT`）。
#[derive(Debug)]
pub enum ExportStemsError {
    /// 领域失败（`validate` / `export` / `write` 的 `Fault` 原文）。
    Fault(Box<Fault>),
}

impl ExportStemsError {
    /// 稳定错误码（`CliError::ExportStems::code` 的**唯一**来源，⛔ 不从文案里正则抠）。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Fault(fault) => match fault.as_ref() {
                Fault::Domain { code, .. } => code.as_str(),
                Fault::Impl { .. } => "INTERNAL_ERROR",
            },
        }
    }

    /// 人话原因（`CliError::ExportStems::reason` 的**唯一**来源：原文逐字带出去。
    /// 落盘失败时这里带 D5 的部分成功回执——`Fault::domain_with_data` 的 `message`
    /// 里已含 `completedStems` / `failedTrackId` / `orphanedFiles` / `resumeFrom`）。
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Fault(fault) => match fault.as_ref() {
                Fault::Domain { message, .. } => message.clone(),
                Fault::Impl { error } => format!("{error:?}"),
            },
        }
    }
}

impl core::fmt::Display for ExportStemsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Fault(fault) => match fault.as_ref() {
                Fault::Domain { message, .. } => write!(formatter, "{message}"),
                Fault::Impl { error } => write!(formatter, "分轨导出失败（实现级）: {error:?}"),
            },
        }
    }
}

impl From<Fault> for ExportStemsError {
    fn from(fault: Fault) -> Self {
        Self::Fault(Box::new(fault))
    }
}

/// 导出全部非 master 源节点轨到已存在的目录，返回实测读数。
///
/// # 参数
///
/// * `project`：要分轨的工程（`--open` 的那份，或 `--project-sample`）；
/// * `project_path`：工程文件路径（SFZ 解析根＋输出护栏基准；样本工程时取 cwd）；
/// * `out_dir`：输出目录（**必须已存在**且是目录）；
/// * `assets`：归档资产池（`assets/{sha256}`，SFZ 文本与样本从这里取）。
///
/// # Errors
///
/// [`ExportStemsError::Fault`]（目录不存在/不是目录、目标已存在、渲染失败、
/// 落盘失败——落盘失败时 `Fault` 的 `data` 里有 D5 的部分成功回执）。
pub fn export_stems(
    project: &YebanProjectV1,
    project_path: &Path,
    out_dir: &Path,
    assets: &[(AssetHash, Vec<u8>)],
) -> Result<ExportStemsReport, ExportStemsError> {
    let mut arguments = Map::new();
    arguments.insert(
        "outDir".to_owned(),
        Value::from(out_dir.display().to_string()),
    );
    let request = stems::validate(&arguments)?;
    let pool: BTreeMap<AssetHash, Vec<u8>> = assets.iter().cloned().collect();
    let batch: StemBatch = stems::export(
        project,
        project_path,
        &request,
        crate::host::now_ms(),
        &pool,
    )?;
    let _ = stems::write(&batch)?;
    let mut stems_out = Vec::with_capacity(batch.stems.len());
    for row in &batch.stems {
        stems_out.push(StemLine {
            track: row.name.clone(),
            file_name: row.file_name.clone(),
            path: row.path.clone(),
            bytes: row.artifact.bytes.len(),
            bit_depth: row.artifact.bit_depth,
            channels: row.artifact.channels as u16,
        });
    }
    let mut skipped_out = Vec::with_capacity(batch.skipped.len());
    for skipped in &batch.skipped {
        // `reason` 的字面量（`masterBusTrack` / `notASourceNode`）是
        // `fault_vocabulary` 黄金表里的冻结词，这里只复述，不另造。
        let reason = match skipped.reason {
            stems::SkipReason::MasterBusTrack => "masterBusTrack",
            stems::SkipReason::NotASourceNode => "notASourceNode",
        };
        skipped_out.push(SkippedLine {
            track: skipped.name.clone(),
            reason,
        });
    }
    Ok(ExportStemsReport {
        dir: out_dir.to_path_buf(),
        stems: stems_out,
        skipped: skipped_out,
        bytes_total: batch.bytes_total(),
        renders: batch.renders,
        basis_frames: batch.basis_frames,
        sample_rate: batch.sample_rate,
    })
}
