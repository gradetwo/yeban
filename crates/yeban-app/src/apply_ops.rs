//! `--apply-ops <ops.json>` —— 批量编辑（ops）的**批处理**出口。
//!
//! ## 它补的是哪一格（这是本模块存在的全部理由）
//!
//! 53 种 ops（`yeban-mcp` 的 `OP_KINDS`，`addTrack` / `insertDevice` / `setParam` /
//! `humanizeClip` / `quantizeClip` 全在内）今天只有两条执行入口：MCP 的
//! `propose_section`（提案）与 `yeban_edit_notes`（直接编辑），以及 GUI 里对提案的
//! 接受 —— 三条都要一个活着的 MCP 会话或一个窗口。在没有显示器的机器上批量改一批
//! 工程，等于先起 GUI 挂 `xvfb` 再连 MCP。
//!
//! 本模块把**同一条**执行管线接到批处理路径上：一次
//! `yeban-app --open X.yeban --apply-ops ops.json --save-as Y.yeban`，不开窗口、
//! 不建 MCP 会话、不碰 `~/.yeban` 的令牌。
//!
//! ## 文件格式：就是 `yeban_edit_notes` 的实参对象
//!
//! ```json
//! {"trackId": "<ULID>", "clipId": "<ULID>", "ops": [{"kind": "setTrackName", "trackId": "<ULID>", "name": "新名"}, ...]}
//! ```
//!
//! 顶层必须是对象（⛔ 裸数组不接受：两种形状就是两个真相源）；`ops` 必填，
//! `trackId` / `clipId` 按 `yeban_edit_notes` 同一条口径条件必需 ——
//! 批里全是自带寻址的形态（`addTrack` / 设备形态 / 逐条给了 `trackId` 的
//! `setParam` / `setTrackMute` / `setTrackSolo`）时两者都可省，其余形态缺失即
//! 响亮拒绝。判定住在 `yeban_mcp::domain::notes::NoteOp` 的谓词里，与 `compile`
//! 同一份真相（⛔ 本模块不另写一张"谁需要顶层身份"的表）。
//!
//! ## 管线（与 `yeban_edit_notes` 同一条，不抄第二份语义）
//!
//! 读文件 → [`notes::parse_ops`]（53 种 kind 的校验、未知键拒绝、空数组拒绝）→
//! 信封身份解析 → [`notes::compile`]（批内投影、整片段形态就地展开、撤销载荷取
//! 当时值）→ 一次提交（`UndoPort::commit_ops_with_origin`，来源
//! [`OpOrigin::Import`]，**一条** `Op::Batch` = 一步撤销）→ 回执。
//! 先全部解析、再一次性提交：任何一步失败都在**提交之前**返回 ⇒ 不会留下半批
//! （与 `cli::apply_track_samplers` 同一条纪律）。
//!
//! ## `--dry-run`：模拟，不是"假装成功"
//!
//! `--dry-run` 只修饰 `--apply-ops`：解析与编译照旧跑，提交改成在**克隆**上
//! `Op::Batch.apply` + `validate`。前置不满足（陈旧载荷、目标不存在）时模拟
//! 照样失败 —— 干跑**报告**领域失败，不伪造成功（与 MCP 分发层
//! `dry_run_reports_domain_failures_in_band_instead_of_faking_success` 同一条）。
//! 干跑不碰 `loaded`、不写盘、不要求 `--save-as`；`--dry-run` 与 `--save-as`
//! 同给 = 用法错误（保存的会是**没改过**的工程，那是静默的假交付）。

use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use serde_json::Value;
use yeban_mcp::domain::error::{Fault, from_model};
use yeban_mcp::domain::notes::{self, NoteOp};
use yeban_mcp::tools::ErrorCode;
use yeban_model::{EntityId, Op, OpOrigin, YebanProjectV1};

/// 真执行的回执前缀（稳定前缀，判据与脚本据此切片）。
pub const RECEIPT_PREFIX: &str = "applied-ops:";
/// 干跑的回执前缀。
pub const DRY_RUN_RECEIPT_PREFIX: &str = "applied-ops-dryrun:";

/// 一次真实执行的实测读数（回执行的**唯一**来源：⛔ 不从入参抄）。
#[derive(Debug)]
pub struct ApplyOpsReport {
    /// ops 文件路径（原样回显）。
    pub file: PathBuf,
    /// 输入的 NoteOp 条数（`parse_ops` 之后）。
    pub note_ops: usize,
    /// 编译出的模型 `Op` 条数。
    pub model_ops: usize,
    /// 去重后的 kind 列表（首次出现序）。
    pub kinds: Vec<&'static str>,
    /// 提交后的提交总数（`UndoDisplay::commit_count`，模型读数）。
    pub commit_count: usize,
    /// 还能撤销的步数（`UndoDisplay::undoable`）。
    pub undoable: usize,
}

impl ApplyOpsReport {
    /// 回执行（**一行**，字段顺序即契约）。
    #[must_use]
    pub fn line(&self) -> String {
        let kinds = self.kinds.join(",");
        if self.model_ops == 0 {
            return format!(
                "{RECEIPT_PREFIX} file={} noteOps={} modelOps=0 kinds={kinds} \
                 commit_count={} undoable={} (编译出 0 条模型操作, 工程未改, 未提交)",
                self.file.display(),
                self.note_ops,
                self.commit_count,
                self.undoable,
            );
        }
        format!(
            "{RECEIPT_PREFIX} file={} noteOps={} modelOps={} kinds={kinds} \
             commit_count={} undoable={}",
            self.file.display(),
            self.note_ops,
            self.model_ops,
            self.commit_count,
            self.undoable,
        )
    }
}

/// 一次干跑的实测读数（**没有**提交，因此没有 `commit_count` / `undoable`）。
#[derive(Debug)]
pub struct ApplyOpsDryRunReport {
    /// ops 文件路径（原样回显）。
    pub file: PathBuf,
    /// 输入的 NoteOp 条数（`parse_ops` 之后）。
    pub note_ops: usize,
    /// 编译出的模型 `Op` 条数。
    pub model_ops: usize,
    /// 去重后的 kind 列表（首次出现序）。
    pub kinds: Vec<&'static str>,
}

impl ApplyOpsDryRunReport {
    /// 回执行（**一行**，字段顺序即契约；`written=no` 是干跑的全部保证）。
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{DRY_RUN_RECEIPT_PREFIX} file={} noteOps={} modelOps={} kinds={} written=no",
            self.file.display(),
            self.note_ops,
            self.model_ops,
            self.kinds.join(","),
        )
    }
}

/// `--apply-ops` / `--dry-run` 的失败（退出码由 `cli::CliError` 统一给）。
#[derive(Debug)]
pub enum ApplyOpsError {
    /// ops 文件读不到或不是合法 JSON（落盘层之外唯一的人话档）。
    Io {
        /// ops 文件路径。
        path: PathBuf,
        /// 读失败 / 解析失败的原文。
        source: String,
    },
    /// 领域失败（`parse_ops` / `compile` / 模拟 / 提交的 `Fault` 原文）。
    Fault(Box<Fault>),
}

impl ApplyOpsError {
    /// 稳定错误码（`CliError::ApplyOps::code` 的**唯一**来源，⛔ 不从文案里正则抠）。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "IO_ERROR",
            Self::Fault(fault) => match fault.as_ref() {
                Fault::Domain { code, .. } => code.as_str(),
                Fault::Impl { .. } => "INTERNAL_ERROR",
            },
        }
    }

    /// 人话原因（`CliError::ApplyOps::reason` 的**唯一**来源：原文逐字带出去）。
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Io { source, .. } => source.clone(),
            Self::Fault(fault) => match fault.as_ref() {
                Fault::Domain { message, .. } => message.clone(),
                Fault::Impl { error } => format!("{error:?}"),
            },
        }
    }
}

impl core::fmt::Display for ApplyOpsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "读 ops 文件 `{}` 失败: {source}", path.display())
            }
            Self::Fault(fault) => match fault.as_ref() {
                Fault::Domain { message, .. } => write!(formatter, "{message}"),
                Fault::Impl { error } => write!(formatter, "ops 执行失败（实现级）: {error:?}"),
            },
        }
    }
}

/// 解析好的编译产物（真执行与干跑共用：⛔ 两条路不各走一遍解析）。
struct CompiledBatch {
    /// 输入的 NoteOp 条数。
    note_ops: usize,
    /// 去重后的 kind 列表（首次出现序）。
    kinds: Vec<&'static str>,
    /// 编译出的模型 `Op`（按批内投影顺序）。
    model_ops: Vec<Op>,
}

fn fault(code: ErrorCode, message: String) -> ApplyOpsError {
    ApplyOpsError::Fault(Box::new(Fault::domain(code, message)))
}

/// 从 JSON 对象里读一个可选的 ULID 身份（缺席 / `null` ⇒ `None`）。
fn read_optional_id(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<EntityId>, ApplyOpsError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => EntityId::from_str(text).map(Some).map_err(|error| {
            fault(
                ErrorCode::InvalidParameterRange,
                format!("`{field}` 不是合法 ULID: {error}"),
            )
        }),
        Some(other) => Err(fault(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是 ULID 字符串, 收到: {other}"),
        )),
    }
}

/// 读文件 → 解析 → 编译（真执行与干跑的**公共前缀**）。
fn load_and_compile(project: &YebanProjectV1, path: &Path) -> Result<CompiledBatch, ApplyOpsError> {
    let text = std::fs::read_to_string(path).map_err(|error| ApplyOpsError::Io {
        path: path.to_path_buf(),
        source: error.to_string(),
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| ApplyOpsError::Io {
        path: path.to_path_buf(),
        source: format!("ops JSON 解析失败: {error}"),
    })?;
    let object = value.as_object().ok_or_else(|| {
        fault(
            ErrorCode::InvalidParameterRange,
            "ops 文件顶层必须是对象（`yeban_edit_notes` 的实参形状：{\"trackId\"?, \"clipId\"?, \"ops\": [...]}），不是数组".to_owned(),
        )
    })?;
    let raw_ops = object.get("ops").ok_or_else(|| {
        fault(
            ErrorCode::InvalidParameterRange,
            "ops 文件缺少 `ops`（必须是 `yeban_edit_notes` 的实参对象，含 `ops` 数组）".to_owned(),
        )
    })?;
    let note_ops =
        notes::parse_ops(raw_ops).map_err(|error| ApplyOpsError::Fault(Box::new(error)))?;
    let mut kinds: Vec<&'static str> = Vec::new();
    for op in &note_ops {
        let kind = op.kind_name();
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    // 信封身份：与 `domain::plan_edit_notes` 同一条口径 —— 批里每条都自带寻址时
    // 顶层身份取占位 nil（`compile` 不读它们）；否则 `trackId` 必填，`clipId`
    // 只在批里有形态读它时必填（`NoteOp::envelope_clip_required` 是唯一真相）。
    // 判定住在共享谓词 `NoteOp::all_ops_address_their_own_track`（`removeTrack` /
    // 设备 / `addTrack` / 逐条点名的属性形态）：`compile` 与 MCP 入口用同一份真相，
    // 这里不另立谓词。`addTrack` 算自带寻址 —— 它只建新轨，身份就是操作对象自带的
    // `trackId`，顶层 `trackId` 一个字节都不读（见该谓词的文档）。
    let (track_id, clip_id) = if NoteOp::all_ops_address_their_own_track(&note_ops) {
        (EntityId::default(), EntityId::default())
    } else {
        let track_id = read_optional_id(object, "trackId")?.ok_or_else(|| {
            fault(
                ErrorCode::InvalidParameterRange,
                "缺少 `trackId`：这批里有形态需要顶层 trackId（目标片段所在的音轨）；全是自带寻址的形态时才可省".to_owned(),
            )
        })?;
        let clip_id = if NoteOp::envelope_clip_required(&note_ops) {
            read_optional_id(object, "clipId")?.ok_or_else(|| {
                fault(
                    ErrorCode::InvalidParameterRange,
                    "缺少 `clipId`：这批里有音符级形态需要顶层 clipId（目标片段）".to_owned(),
                )
            })?
        } else {
            EntityId::default()
        };
        (track_id, clip_id)
    };
    let model_ops = notes::compile(project, &track_id, &clip_id, &note_ops)
        .map_err(|error| ApplyOpsError::Fault(Box::new(error)))?;
    Ok(CompiledBatch {
        note_ops: note_ops.len(),
        kinds,
        model_ops,
    })
}

/// 真执行：解析 → 编译 → **一次提交**，返回提交后的工程与实测读数。
///
/// # 参数
///
/// * `project`：`--open` 的那份工程（或 `--project-sample`）；
/// * `path`：ops JSON 文件路径。
///
/// 提交是一条 `Op::Batch`（来源 [`OpOrigin::Import`]，消息固定为
/// `cli --apply-ops`），因此本次编辑在撤销栈里是**一步**。调用方负责把返回的
/// 工程落回归档（`--save-as` 那一步）；本函数**不**写盘。
///
/// # Errors
///
/// [`ApplyOpsError::Io`]（文件读不到 / 不是合法 JSON）或
/// [`ApplyOpsError::Fault`]（`parse_ops` / `compile` / 建会话 / 提交被拒）。
pub fn apply_ops(
    project: &YebanProjectV1,
    path: &Path,
) -> Result<(YebanProjectV1, ApplyOpsReport), ApplyOpsError> {
    let compiled = load_and_compile(project, path)?;
    if compiled.model_ops.is_empty() {
        // 编译出 0 条 ⇒ ⛔ 不制造"什么也没改"的提交（与 `apply_track_samplers`
        // 的 `pending == 0` 分支同一条纪律）。
        return Ok((
            project.clone(),
            ApplyOpsReport {
                file: path.to_path_buf(),
                note_ops: compiled.note_ops,
                model_ops: 0,
                kinds: compiled.kinds,
                commit_count: 0,
                undoable: 0,
            },
        ));
    }
    let model_ops_len = compiled.model_ops.len();
    let now = crate::host::now_ms();
    let session =
        crate::undo::UndoSession::open("<cli:apply-ops>", "yeban-app", project.clone(), now)
            .map_err(|error| fault(ErrorCode::Conflict, format!("建撤销会话失败: {error}")))?;
    let port = crate::undo::UndoPort::new(session);
    port.commit_ops_with_origin(now, OpOrigin::Import, "cli --apply-ops", compiled.model_ops)
        .map_err(|error| fault(ErrorCode::Conflict, format!("提交被拒: {error}")))?;
    let display = port.display();
    Ok((
        port.project(),
        ApplyOpsReport {
            file: path.to_path_buf(),
            note_ops: compiled.note_ops,
            model_ops: model_ops_len,
            kinds: compiled.kinds,
            commit_count: display.commit_count,
            undoable: display.undoable,
        },
    ))
}

/// 干跑：解析 → 编译 → 在**克隆**上模拟施加 + 校验，不碰原工程、不写盘。
///
/// # Errors
///
/// 与 [`apply_ops`] 同两档，外加模拟施加 / 结果校验的领域失败（前置不满足、
/// 陈旧载荷都在这里响亮失败，⛔ 不伪造成功）。
pub fn dry_run_ops(
    project: &YebanProjectV1,
    path: &Path,
) -> Result<ApplyOpsDryRunReport, ApplyOpsError> {
    let compiled = load_and_compile(project, path)?;
    let mut simulated = project.clone();
    Op::Batch {
        ops: compiled.model_ops.clone(),
        description: "cli --apply-ops dry-run".to_owned(),
    }
    .apply(&mut simulated)
    .map_err(|error| ApplyOpsError::Fault(Box::new(from_model("ops 模拟", &error))))?;
    simulated
        .validate()
        .map_err(|error| ApplyOpsError::Fault(Box::new(from_model("ops 结果校验", &error))))?;
    Ok(ApplyOpsDryRunReport {
        file: path.to_path_buf(),
        note_ops: compiled.note_ops,
        model_ops: compiled.model_ops.len(),
        kinds: compiled.kinds,
    })
}
