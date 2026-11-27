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
//! * 主总线轨**不出文件**，但在 `skippedTracks` 里点名（⛔ 不静默丢轨，D4）；
//!   有喂入且能到达主总线的总线/分组轨出**总线 stem**（喂入求和经其自身通路
//!   在主总线上的实际贡献，线 G ⑪）；无通路/无内容的轨同样点名。
//!   回执里每条跳过的轨各有一行 `stem-skipped:`。
//! * 输出目录不存在时**在写盘时创建**（`stems::export_streaming` 开头的
//!   `create_dir_all`，B6 修订 2026-11-14，与 `--export-wav` 口径对齐）；
//!   已存在但不是目录 ⇒ `stems::validate` 响亮拒绝。
//! * 目标文件已存在 ⇒ 整批在渲染之前就拒绝（⛔ 不覆盖、不跳过、不改名）。
//! * 落盘逐条原子；第 k 条失败时前 k−1 条**保留不删**，回 `IO_ERROR` +
//!   `errorKind:"stemWriteFailed"`，`data` 里给 `completedStems` /
//!   `failedTrackId` / `orphanedFiles` / `resumeFrom`（D5 的部分成功是一等状态）。
//! * 渲染失败 ⇒ `RENDER_FAILED`；`trackIds` 点名不存在的轨 ⇒ `TRACK_NOT_FOUND`。
//! * 选轨：`--stems-track-ids <ULID,ULID,...>`（`cli::Options::export_stems_track_ids`）
//!   透传成后端的 `trackIds` 实参，只导点名的轨；不给 = 全部非 master 源节点轨。
//!   空清单在 CLI 解析期就被拒绝，非法/未知身份由后端响亮拒绝。
//! * manifest：成功时同时把机器可读的批次索引落到 `<dir>/stems-manifest.json`
//!   （`stems::write_manifest`，与 MCP 腿同一份）：每条 stem 的 `trackId` / 轨名 /
//!   文件名 / `sha256` / 字节数等都在里面，进程退出后可据它核对与续跑判定。
//!   回执汇总行以 `manifest=` 报它的落点。
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
use yeban_mcp::domain::stems::{self, StemBatch, StemProgress};
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
    /// 轨身份（ULID 文本，与 manifest 的 `trackId` 同值）。
    pub track_id: String,
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
    /// 本条分轨里**真的参与渲染**的音频片段摆放数（T-FIX-418：
    /// `StemRow::audio_clips_rendered` 的同一批读数）。
    pub audio_clips_rendered: u64,
    /// 本条分轨里因为资产字节不在会话池里而**没有**渲染的音频片段摆放数
    /// （T-FIX-418：`StemRow::audio_clips_skipped` 的同一批读数 —— 文件照常
    /// 落盘、却是静音的；toast 必须从这个数看出来）。
    pub audio_clips_skipped: u64,
}

/// 一条跳过轨的实测读数（`SkippedTrack::to_value` 的同一批字段）。
#[derive(Debug)]
pub struct SkippedLine {
    /// 轨身份（ULID 文本，与 manifest 的 `trackId` 同值）。
    pub track_id: String,
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
    /// 本批**真的参与渲染**的音频片段摆放数合计（T-FIX-418：
    /// `StemBatch::audio_clips_rendered` 的同一批读数）。
    pub audio_clips_rendered: u64,
    /// 本批因为资产字节不在会话池里而**没有**渲染的音频片段摆放数合计
    /// （T-FIX-418：`StemBatch::audio_clips_skipped` 的同一批读数 —— 口径与
    /// 轨级 `skipped`（整轨不出文件）**不同**，不重复计数；一条轨只要出了文件
    /// 就不进 `skipped`，哪怕它的音频片段全是这个口径）。
    pub audio_clips_skipped: u64,
    /// manifest 的落点（`<dir>/stems-manifest.json`，已落盘）。
    pub manifest_path: PathBuf,
    /// T-FIX-691：本批落盘的持久化警告（分轨文件 + manifest 自身；`rename` 成功但
    /// 刷目录失败 = 已提交、持久化未确认），每条是一份文件的警告文案（同文案去重）。
    ///
    /// 完全落盘时为空；有警告**仍是**一次成功的导出，调用方（CLI）必须把它如实带进
    /// 回执。与 app 侧 `SaveReport.durability`（T-FIX-324）同口径。
    pub durability_warnings: Vec<String>,
}

impl ExportStemsReport {
    /// 回执行：**一行汇总** + 持久化警告行（有才打）+ 每条分轨一行 + 每条跳过轨一行。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{RECEIPT_PREFIX} dir={} stems={} skipped={} bytes={} renders={} sampleRate={} bitDepth=32 audioClipsRendered={} audioClipsSkipped={} manifest={}",
            self.dir.display(),
            self.stems.len(),
            self.skipped.len(),
            self.bytes_total,
            self.renders,
            self.sample_rate,
            self.audio_clips_rendered,
            self.audio_clips_skipped,
            self.manifest_path.display(),
        )];
        // T-FIX-691：持久化警告紧跟汇总行（与 `--export-midi` 等的
        // `committed=true durability=warning` 同一条文案口径）；无警告时一行都不多。
        for warning in &self.durability_warnings {
            lines.push(format!("committed=true durability=warning {warning}"));
        }
        for stem in &self.stems {
            lines.push(format!(
                "{STEM_LINE_PREFIX} path={} track={} trackId={} fileName={} bytes={} bitDepth={} channels={} audioClipsRendered={} audioClipsSkipped={}",
                stem.path.display(),
                stem.track,
                stem.track_id,
                stem.file_name,
                stem.bytes,
                stem.bit_depth,
                stem.channels,
                stem.audio_clips_rendered,
                stem.audio_clips_skipped,
            ));
        }
        for skipped in &self.skipped {
            lines.push(format!(
                "{SKIPPED_LINE_PREFIX} track={} trackId={} reason={}",
                skipped.track, skipped.track_id, skipped.reason,
            ));
        }
        lines
    }
}

/// 分轨导出 toast 的**纯文案**（T-FIX-418：GUI 与 CLI 共用同一条判定，判据不
/// 用起 UI 就能钉死 —— 见 `tests/export_stems_clip_counts.rs`）。
///
/// 判定只看**音频片段维度**的未渲染数（`report.audio_clips_skipped`：资产字节
/// 不在会话池里 ⇒ 登记 + 静音的摆放；文件照常落盘、却是静音的）：
///
/// * `skipped == 0` ⇒ 沿用原来的"完成"文案，**一字不改**（`host.rs` 的旧 toast
///   是 `"[导出] 分轨完成：{} 条（跳过 {}）→ {}"`，这里逐字复述）。
/// * `skipped > 0` ⇒ 降级为警告：前缀改成"分轨警告"，并在括号里点名未渲染的
///   片段数 —— 一条分轨的音频片段全被跳过时，用户再也不会只看到"完成"。
///   括号里仍保留轨级 `skipped` 数（整轨不出文件的轨），两者口径不同，⛔ 不合并。
#[must_use]
pub fn toast_message(report: &ExportStemsReport, dir: &std::path::Path) -> String {
    // T-FIX-990：持久化警告也是"分轨警告"（与未渲染片段同档）：警告子句只追加，
    // ⛔ 不改既有两条臂的文案 —— 零警告时逐字沿用旧文案（既有判据钉死）。
    // 多条警告时只报首条原文（与保存"首条原因"、回执首条口径一致），条数照实报。
    let mut clauses: Vec<String> = Vec::new();
    if report.audio_clips_skipped > 0 {
        clauses.push(format!("{} 个音频片段未渲染", report.audio_clips_skipped));
    }
    if !report.durability_warnings.is_empty() {
        clauses.push(format!(
            "持久化警告 {} 条，首条：{}",
            report.durability_warnings.len(),
            report.durability_warnings[0]
        ));
    }
    if clauses.is_empty() {
        format!(
            "[导出] 分轨完成：{} 条（跳过 {}）→ {}",
            report.stems.len(),
            report.skipped.len(),
            dir.display(),
        )
    } else {
        format!(
            "[导出] 分轨警告：{} 条（{}；跳过 {}）→ {}",
            report.stems.len(),
            clauses.join("；"),
            report.skipped.len(),
            dir.display(),
        )
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
    track_ids: Option<&[String]>,
    assets: &[(AssetHash, Vec<u8>)],
) -> Result<ExportStemsReport, ExportStemsError> {
    export_stems_with_progress(
        project,
        project_path,
        out_dir,
        track_ids,
        assets,
        &mut |_| {},
    )
}

/// 带进度的分轨导出（T-FIX-936）。
///
/// `progress` 在逐轨循环起时与每条分轨落盘后各报一次（见
/// [`StemProgress`]）；本函数是 [`export_stems`] 的带进度版（历史调用点不动）。
/// manifest 写盘在流式导出返回后发生，那一步没有单独的进度采样 —— 它是一次
/// 小 JSON 写盘，终局回执紧随其后。
pub fn export_stems_with_progress(
    project: &YebanProjectV1,
    project_path: &Path,
    out_dir: &Path,
    track_ids: Option<&[String]>,
    assets: &[(AssetHash, Vec<u8>)],
    progress: &mut dyn FnMut(&StemProgress),
) -> Result<ExportStemsReport, ExportStemsError> {
    let mut arguments = Map::new();
    arguments.insert(
        "outDir".to_owned(),
        Value::from(out_dir.display().to_string()),
    );
    if let Some(ids) = track_ids {
        arguments.insert(
            "trackIds".to_owned(),
            Value::Array(ids.iter().map(|id| Value::from(id.clone())).collect()),
        );
    }
    let request = stems::validate(&arguments)?;
    let pool: BTreeMap<AssetHash, Vec<u8>> = assets.iter().cloned().collect();
    // ⭐ **流式**（渲一条、写一条、释放一条）而不是 `export` + `write`：
    //   批处理没有 `dryRun`，没有理由把 N 条分轨全攒在内存里再统一写盘。
    //   实测读数不变（同一条渲染链、同一条容器编码），内存峰值从
    //   `N × 单轨字节` 降到 `1 × 单轨字节`。
    let batch: StemBatch = stems::export_streaming_with_progress(
        project,
        project_path,
        &request,
        crate::host::now_ms(),
        &pool,
        progress,
    )?;
    // manifest 在分轨全部落盘后写（失败 ⇒ 整批以 `Fault` 响亮失败，⛔ 不假装
    // 索引已落盘）；内容与 MCP 腿的 `stems::write` 写的那份同一实现。
    // T-FIX-691：manifest 自身的持久化警告同样收进回执（行级警告已在流式落盘时
    // 收进 `batch.durability_warnings`；同文案去重）。
    let mut durability_warnings = batch.durability_warnings.clone();
    let (manifest_path, manifest_warning) =
        stems::write_manifest_with_durability(&batch, &durability_warnings)?;
    if let Some(warning) = manifest_warning.as_deref()
        && !durability_warnings
            .iter()
            .any(|existing| existing == warning)
    {
        durability_warnings.push(warning.to_owned());
    }
    let mut stems_out = Vec::with_capacity(batch.stems.len());
    for row in &batch.stems {
        stems_out.push(StemLine {
            track_id: row.track_id.to_canonical_string(),
            track: row.name.clone(),
            file_name: row.file_name.clone(),
            path: row.path.clone(),
            bytes: row.bytes_len,
            bit_depth: row.artifact.bit_depth,
            channels: row.artifact.channels as u16,
            audio_clips_rendered: row.audio_clips_rendered,
            audio_clips_skipped: row.audio_clips_skipped,
        });
    }
    let mut skipped_out = Vec::with_capacity(batch.skipped.len());
    for skipped in &batch.skipped {
        // `reason` 的字面量（`masterBusTrack` / `notASourceNode` / `emptyTrack`）是
        // `fault_vocabulary` 黄金表里的冻结词，这里只复述，不另造。
        let reason = match skipped.reason {
            stems::SkipReason::MasterBusTrack => "masterBusTrack",
            stems::SkipReason::NotASourceNode => "notASourceNode",
            stems::SkipReason::EmptyTrack => "emptyTrack",
        };
        skipped_out.push(SkippedLine {
            track_id: skipped.track_id.to_canonical_string(),
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
        audio_clips_rendered: batch.audio_clips_rendered(),
        audio_clips_skipped: batch.audio_clips_skipped(),
        manifest_path,
        durability_warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(warnings: Vec<String>) -> ExportStemsReport {
        ExportStemsReport {
            dir: PathBuf::from("/tmp/stems-out"),
            stems: Vec::new(),
            skipped: Vec::new(),
            bytes_total: 0,
            renders: 1,
            basis_frames: 48_000,
            sample_rate: 48_000,
            audio_clips_rendered: 0,
            audio_clips_skipped: 0,
            manifest_path: PathBuf::from("/tmp/stems-out/stems-manifest.json"),
            durability_warnings: warnings,
        }
    }

    /// 判据（T-FIX-691）：有持久化警告时汇总行后紧跟
    /// `committed=true durability=warning` 行（与 `--export-midi` 等同一口径）。
    #[test]
    fn lines_carry_durability_warnings_right_after_the_summary() {
        let report = report_with(vec![
            "文件已替换，但目录项未确认持久化（刷目录 `/tmp/stems-out` 失败: 注入）".to_owned(),
        ]);
        let lines = report.lines();
        assert_eq!(lines.len(), 2, "汇总行 + 1 条警告行: {lines:?}");
        assert!(
            lines[0].starts_with(RECEIPT_PREFIX),
            "首行必须是汇总行: {}",
            lines[0]
        );
        assert!(
            lines[1].contains("committed=true durability=warning"),
            "警告行必须明写 committed=true / durability=warning: {}",
            lines[1]
        );
        assert!(
            lines[1].contains("目录项未确认持久化"),
            "警告必须说清'文件已替换、目录项未确认': {}",
            lines[1]
        );
    }

    /// 上条判据的反面（T-FIX-691）：无警告时回执形状与从前逐行一致（只剩汇总行）。
    #[test]
    fn lines_without_warnings_are_unchanged() {
        let lines = report_with(Vec::new()).lines();
        assert_eq!(lines.len(), 1, "无警告时只剩汇总行: {lines:?}");
        assert!(
            !lines[0].contains("durability"),
            "健康路径不得凭空多出 durability 字样: {}",
            lines[0]
        );
    }

    /// 判据（T-FIX-990）：有持久化警告时 toast 必须降级为警告并点名条数与首条原文；
    /// 无未渲染片段时也不得再报"分轨完成"。
    #[test]
    fn toast_message_names_a_durability_warning() {
        let report = report_with(vec![
            "文件已替换，但目录项未确认持久化（刷目录 `/tmp/stems-out` 失败: 注入）".to_owned(),
        ]);
        let toast = toast_message(&report, Path::new("/tmp/stems-out"));
        assert!(
            toast.starts_with("[导出] 分轨警告"),
            "有持久化警告时 toast 必须降级为警告，实得 {toast}"
        );
        assert!(
            toast.contains("持久化警告 1 条"),
            "警告里必须点名条数，实得 {toast}"
        );
        assert!(
            toast.contains("目录项未确认持久化"),
            "警告里必须带首条原文，实得 {toast}"
        );
        assert!(
            !toast.contains("分轨完成"),
            "警告文案里不得再出现\"分轨完成\"，实得 {toast}"
        );
    }

    /// 上条判据的反面（T-FIX-990）：无持久化警告时 toast 与从前逐字一致。
    #[test]
    fn toast_message_without_warnings_is_unchanged() {
        let report = report_with(Vec::new());
        let toast = toast_message(&report, Path::new("/tmp/stems-out"));
        assert_eq!(
            toast, "[导出] 分轨完成：0 条（跳过 0）→ /tmp/stems-out",
            "零警告时 toast 必须逐字沿用旧文案"
        );
    }
}
