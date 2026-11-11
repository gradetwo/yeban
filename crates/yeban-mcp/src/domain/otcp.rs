//! `yeban_correct_pitch` / `yeban_correct_pitch_job` / `yeban_correct_pitch_notes`
//! —— OTCP 音高校正的 MCP 适配层（`ADR-0070`）。
//!
//! ## plan / apply 分工
//!
//! - `yeban_correct_pitch` 的 plan：校验实参 → 定位片段 → 查双重校正 →
//!   解码/重采样源音频 → 写源 WAV → 写片段→任务索引 → **派生后台线程**
//!   （`OtcpDriver::correct_clip` 在线程里跑，收据是唯一的会合点）→
//!   回 `Plan::OtcpSubmit`（提交回执）。提交本身是**外部**副作用（启动
//!   OpenTune 子进程建任务），不改工程文档、不落盘工程；`dryRun`（通用预览
//!   键）只校验参数、不提交。
//! - `yeban_correct_pitch_job` 的 plan：读收据。`running` / `failed` /
//!   `cancelled` 只回状态数据（`Plan::OtcpJob`，只读）；`succeeded` 且未
//!   交付且非干跑 ⇒ 建交付计划（`Plan::OtcpDeliver`：`put_asset` +
//!   `apply_propose`，默认只出提案，提交时的 `autoMerge` 当场合并）。
//! - `yeban_correct_pitch_notes` 的 plan：只读收据里的音符表（`Plan::OtcpNotes`），
//!   ⛔ 不启动服务端现算音符。
//!
//! ## 失败语义（响亮，不静默空成功）
//!
//! - 所有 OTCP `reason`（`otcpRenderFailed` 等 19 个）**只**从 [`fault_for`]
//!   发出：一个 reason 一个 `match` 臂、一个固定的载荷键集（见 `REASONS`
//!   与 `tests/fault_vocabulary.rs` 的黄金表）。驱动侧的 `OtcpError`
//!   只带 `reason` + `message` + `data`，⛔ 不在适配层之外另造 reason。
//! - `RENDER_FAILED` 时任务收据里已有 §6.2 结构化错误（失败块 + 音符间隙 +
//!   自测记录）；任务工具原样回传它，⛔ 不回落干声、⛔ 不写资产。
//! - 交付时片段资产已变 ⇒ `otcpRevisionConflict`；片段已删 ⇒ `otcpContentGone`。
//! - `predictOnly`（真跑一遍导入→F0→调音→读谱，只预测）与通用 `dryRun`
//!   （只校验参数、不提交）是两回事：前者是真实任务，后者是预览。
//!   通用 `dryRun` 由 [`crate::dispatch`] 拦截，plan 只出预览、永不派生线程。

use std::fs;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::thread;

use serde_json::Value;
use yeban_decode::{DecodeOptions, resample_asset};
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, DeviceKind, EntityId, MediaKind, Op, TrackV3,
};
use yeban_otcp::wav::write_wav_mono_pcm16;
use yeban_otcp::{
    CorrectionRequest, JobReceipt, Key, Note, OtcpConfig, OtcpDriver, OtcpError, VALID_SCALES,
    mark_delivered, write_initial_receipt,
};

use super::error::{self, Fault};
use super::proposal::ProposalDraft;
use super::{Domain, Plan, require_active};
use crate::tools::{ErrorCode, ToolCall, ToolResponse};

// ===========================================================================
// plan 入口（由 `domain::plan` 路由；`apply` 在 `domain::apply_plan_model`）
// ===========================================================================

/// `yeban_correct_pitch` 的 plan：校验 → 备源音频 → 派生后台线程。
///
/// # Errors
///
/// 参数非法 / 无活跃工程 / 片段不存在 / 非音频片段 / 立体声源 / 双重校正 /
/// 源资产无许可 / 二进制未配置 / 线程派生失败，见模块头失败语义。
pub fn plan_correct_pitch(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let track_id = super::arg_id(call, "trackId")?;
    let clip_id = super::arg_id(call, "clipId")?;
    let key_root = parse_key_root(call)?;
    let scale = parse_scale(call)?;
    let mode = parse_mode(call)?;
    let tuning_hz = parse_tuning_hz(call)?;
    let auto_merge = super::arg_bool(call, "autoMerge", false);
    let predict_only = super::arg_bool(call, "predictOnly", false);
    let note_ops = parse_note_ops(call)?;
    let project = require_active(domain)?;
    let (source_asset, license) = resolve_clip(project, &track_id, &clip_id)?;
    check_double_correction(project, &track_id)?;
    if call.is_dry_run() {
        return Ok(Plan::OtcpSubmit {
            data: serde_json::json!({
                "wouldSubmit": true,
                "trackId": track_id.to_canonical_string(),
                "clipId": clip_id.to_canonical_string(),
                "keyRoot": key_root,
                "scale": scale,
                "mode": mode,
                "tuningHz": tuning_hz,
                "autoMerge": auto_merge,
                "predictOnly": predict_only,
                "note": "dryRun：只校验参数，未启动服务端、未写任务目录；真提交会派生后台线程跑校正",
            }),
        });
    }
    let job_id = EntityId::new().to_canonical_string();
    let config = OtcpConfig::default();
    // 源音频制备在二进制检查之前：源问题（非音频 / 立体声 / 解码失败）
    // 先于"二进制未配置"报出，且立体声判据无需二进制在位。
    let (source_wav, source_duration_secs) = prepare_source_wav(
        domain,
        &config,
        &job_id,
        project,
        &source_asset,
        &track_id,
        &clip_id,
    )?;
    let status = OtcpDriver::new(config.clone()).status();
    if !status.binary_present {
        return Err(fault_for(&OtcpError::new(
            "otcpNotConfigured",
            format!(
                "未配置 OpenTune 二进制（YEBAN_OPENTUNE_BIN / ~/.local/bin/OpenTune，当前 {})",
                status.binary_path.display()
            ),
        )));
    };
    let request = CorrectionRequest {
        job_id: job_id.clone(),
        source_wav,
        source_duration_secs,
        key: Key {
            root: key_root,
            scale: scale.clone(),
        },
        mode: mode.clone(),
        tuning_hz,
        dry_run: predict_only,
        note_ops,
        track_id: track_id.to_canonical_string(),
        clip_id: clip_id.to_canonical_string(),
        auto_merge,
        source_asset_hash: source_asset.as_str().to_owned(),
        source_license: license,
    };
    // 先派生线程（worker 第一件事就是写 `running` 收据），再尽力补写初始收据
    // （把"提交响应已回、轮询却短暂报 otcpJobNotFound"的窗口关到最小；
    // 补写失败也无妨 —— worker 自己会写）。
    let worker_config = config.clone();
    let worker_request = request.clone();
    if thread::Builder::new()
        .name(format!("otcp-{job_id}"))
        .spawn(move || {
            let driver = OtcpDriver::new(worker_config);
            let _ = driver.correct_clip(worker_request);
        })
        .is_err()
    {
        let _ = fs::remove_dir_all(config.job_dir(&job_id));
        return Err(fault_for(&OtcpError::with_data(
            "otcpServerGone",
            "派生校正线程失败（系统线程资源耗尽），任务未建".to_owned(),
            serde_json::json!({ "job_id": job_id }),
        )));
    }
    let _ = write_initial_receipt(&config, &request);
    if let Err(error) = write_clip_index(&config, &track_id, &clip_id, &job_id) {
        return Err(fault_for(&error));
    }
    Ok(Plan::OtcpSubmit {
        data: serde_json::json!({
            "jobId": job_id,
            "status": "running",
            "dryRun": predict_only,
            "server": server_block(&status),
            "notesBefore": { "count": Value::Null, "notes": [] },
            "receiptPath": config.receipt_path(&job_id).display().to_string(),
            "note": "提交成功只是任务已入队；用 yeban_correct_pitch_job 轮询完成情况",
        }),
    })
}

/// `yeban_correct_pitch_job` 的 plan：读收据；成功且未交付 ⇒ 建交付计划。
///
/// # Errors
///
/// 任务不存在 / 收据损坏 / 导出文件丢失 / 片段已删 / 资产已变，见模块头失败语义。
/// 无活跃工程时**不**报错 —— 只回"交付待定"状态，打开工程后重轮询即可交付。
pub fn plan_correct_pitch_job(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let job_id = parse_job_id(call)?;
    let config = OtcpConfig::default();
    let driver = OtcpDriver::new(config.clone());
    let receipt = driver
        .read_receipt(&job_id)
        .map_err(|error| fault_for(&error))?;
    match receipt.status.as_str() {
        "succeeded" => plan_succeeded_job(domain, &config, &receipt),
        _ => Ok(Plan::OtcpJob {
            data: job_summary(&config, &receipt),
        }),
    }
}

/// `yeban_correct_pitch_notes` 的 plan：只读收据里的音符表。
///
/// # Errors
///
/// 寻址非法（既没给片段、也没给内容；或两者都给了）/ 无任务收据，见模块头失败语义。
pub fn plan_correct_pitch_notes(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let _ = domain;
    let config = OtcpConfig::default();
    let track_id = match call.arguments.get("trackId") {
        None => None,
        Some(value) => Some(parse_id_arg(value, "trackId")?),
    };
    let clip_id = match call.arguments.get("clipId") {
        None => None,
        Some(value) => Some(parse_id_arg(value, "clipId")?),
    };
    let content_id = call
        .arguments
        .get("contentId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let receipt = match (track_id, clip_id, content_id) {
        (Some(track), Some(clip), None) => {
            let job_id = read_clip_index(&config, &track, &clip)?;
            OtcpDriver::new(config.clone())
                .read_receipt(&job_id)
                .map_err(|error| fault_for(&error))?
        }
        (None, None, Some(content)) => find_receipt_by_content(&config, &content)?,
        _ => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                "须给 trackId + clipId 或 contentId，二选一（两者都给 / 都不给都不接受）",
            ));
        }
    };
    // 音符表 = 校正后的表（有）；否则 = 校正前的检测表（在途任务）。
    let notes = if receipt.notes_after.is_empty() {
        &receipt.notes_before
    } else {
        &receipt.notes_after
    };
    Ok(Plan::OtcpNotes {
        data: serde_json::json!({
            "jobId": receipt.job_id,
            "contentId": receipt.content_id,
            "tuningHz": receipt.tuning_hz,
            "totalCount": notes.len(),
            "notes": notes_json(notes),
            "note": "这是 OpenTune 检测到的音符，不是夜半乐谱（会过分割）；无任务时报 otcpNotesUnavailable，不返回空表",
        }),
    })
}

// ===========================================================================
// apply（`Plan::OtcpDeliver` 的唯一施加；`Plan::OtcpSubmit/Job/Notes` 在
// `domain::apply_plan_model` 里原样回传 `data`）
// ===========================================================================

/// 交付一次校正：先登记 CAS 字节，再走现有 `apply_propose`（建提案；
/// `autoMerge` 时同一把锁内背靠背合并），最后给收据记交付。
///
/// # Errors
///
/// 无活跃工程（`put_asset` 的 `NO_ACTIVE_PROJECT`）/ 提案创建失败 /
/// 交付收据写失败（提案已创建时，错误信息里带提案 id，⛔ 不假装回滚）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_deliver(
    domain: &mut Domain,
    draft: ProposalDraft,
    include_ops: bool,
    warnings: &[Value],
    auto_merge: bool,
    merge_commit_message: Option<String>,
    asset_bytes: Vec<u8>,
    job_id: &str,
) -> Result<ToolResponse, Fault> {
    let config = OtcpConfig::default();
    let expected = AssetHash::of_bytes(&asset_bytes);
    let hash = domain.put_asset(asset_bytes)?;
    if hash != expected {
        return Err(Fault::domain(
            ErrorCode::IoError,
            "CAS 哈希不一致（内部错误：登记返回的哈希与计划时的哈希不同）",
        ));
    }
    let mut response = super::apply_propose(
        domain,
        draft,
        include_ops,
        warnings,
        auto_merge,
        merge_commit_message,
    )?;
    let proposal_id = match &response {
        ToolResponse::Success { data } => data
            .get("proposal")
            .and_then(|proposal| proposal.get("proposalId"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    };
    let Some(proposal_id) = proposal_id else {
        return Err(Fault::domain(
            ErrorCode::IoError,
            "提案已创建，但响应里缺 proposalId（内部错误），交付收据未记",
        ));
    };
    if let Err(error) = mark_delivered(&config, job_id, &proposal_id) {
        return Err(Fault::domain_with_data(
            ErrorCode::IoError,
            format!(
                "提案 {proposal_id} 已创建，但交付收据写失败（{}）；提案保持 Open，下次轮询会凭标题认出它并补记收据",
                error.message
            ),
            serde_json::json!({
                "reason": "otcpServerGone",
                "job_id": job_id,
            }),
        ));
    }
    if let ToolResponse::Success { data } = &mut response
        && let Some(object) = data.as_object_mut()
    {
        object.insert("jobId".to_owned(), Value::from(job_id));
        object.insert(
            "receiptPath".to_owned(),
            Value::from(config.receipt_path(job_id).display().to_string()),
        );
        // 交付响应同时带任务摘要（状态 / 资产 / 量测 / 音符表）：
        // 只回提案响应会丢 `status` 与 `asset`，调用方轮询就看不到
        // "任务成功 + 提案已建" 的完整画面。键不冲突（提案响应是
        // created / pendingMerge / proposal…，摘要是 status / asset…）。
        // 收据读失败时只回提案响应（交付本身已成功，不因此报错）。
        if let Ok(receipt) = OtcpDriver::new(config.clone()).read_receipt(job_id)
            && let Ok(summary) = succeeded_data(&config, &receipt)
            && let Value::Object(fields) = summary
        {
            for (key, value) in fields {
                object.insert(key, value);
            }
        }
    }
    Ok(response)
}

// ===========================================================================
// `fault_for`：OTCP reason → MCP Fault 的**唯一**发出点
// ===========================================================================

/// `OtcpError` → [`Fault`]。
///
/// 一个 reason 一个臂、一个固定的载荷键集（值缺失时记 `null`，键**恒在**）：
/// 判据 `tests/fault_vocabulary.rs` 的黄金表（`GOLDEN_REASON_PAYLOADS`）
/// 钉的就是这里发出的键集 —— 改任一臂的键集必须同步改黄金表。
#[must_use]
pub fn fault_for(error: &OtcpError) -> Fault {
    let data = &error.data;
    let get = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
    let message = error.message.clone();
    match error.reason {
        "otcpRenderFailed" => Fault::domain_with_data(
            ErrorCode::RenderFailed,
            message,
            serde_json::json!({
                "reason": "otcpRenderFailed",
                "job_id": get("job_id"),
                "block": get("block"),
                "note_gap": get("note_gap"),
                "self_test": get("self_test"),
            }),
        ),
        "otcpModalBlocked" => Fault::domain_with_data(
            ErrorCode::Busy,
            message,
            serde_json::json!({
                "reason": "otcpModalBlocked",
                "job_id": get("job_id"),
                "dialogs": get("dialogs"),
            }),
        ),
        "otcpDialogCommits" => Fault::domain_with_data(
            ErrorCode::Conflict,
            message,
            serde_json::json!({
                "reason": "otcpDialogCommits",
                "job_id": get("job_id"),
                "dialogs": get("dialogs"),
            }),
        ),
        "otcpRevisionConflict" => Fault::domain_with_data(
            ErrorCode::Conflict,
            message,
            serde_json::json!({
                "reason": "otcpRevisionConflict",
                "job_id": get("job_id"),
            }),
        ),
        "otcpNotReady" => Fault::domain_with_data(
            ErrorCode::IoError,
            message,
            serde_json::json!({
                "reason": "otcpNotReady",
                "job_id": get("job_id"),
            }),
        ),
        "otcpContentGone" => Fault::domain_with_data(
            ErrorCode::ClipNotFound,
            message,
            serde_json::json!({
                "reason": "otcpContentGone",
                "job_id": get("job_id"),
                "content_id": get("content_id"),
            }),
        ),
        "otcpMeasureFailed" => Fault::domain_with_data(
            ErrorCode::RenderFailed,
            message,
            serde_json::json!({
                "reason": "otcpMeasureFailed",
                "job_id": get("job_id"),
                "measurement": data.clone(),
            }),
        ),
        "otcpNoEditor" => Fault::domain_with_data(
            ErrorCode::IoError,
            message,
            serde_json::json!({
                "reason": "otcpNoEditor",
                "job_id": get("job_id"),
            }),
        ),
        "otcpUnsupportedBuild" => Fault::domain_with_data(
            ErrorCode::IoError,
            message,
            serde_json::json!({
                "reason": "otcpUnsupportedBuild",
                "job_id": get("job_id"),
                "build_commit": get("build_commit"),
                "missing_capabilities": get("missing_capabilities"),
            }),
        ),
        "otcpDoubleCorrection" => Fault::domain_with_data(
            ErrorCode::Conflict,
            message,
            serde_json::json!({
                "reason": "otcpDoubleCorrection",
                "track_id": get("track_id"),
                "device_id": get("device_id"),
            }),
        ),
        "otcpStereoUnsupported" => Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            message,
            serde_json::json!({
                "reason": "otcpStereoUnsupported",
                "track_id": get("track_id"),
                "clip_id": get("clip_id"),
                "channels": get("channels"),
            }),
        ),
        "otcpServerGone" => Fault::domain_with_data(
            ErrorCode::IoError,
            message,
            serde_json::json!({
                "reason": "otcpServerGone",
                "job_id": get("job_id"),
            }),
        ),
        "otcpBusy" => Fault::domain_with_data(
            ErrorCode::Busy,
            message,
            serde_json::json!({
                "reason": "otcpBusy",
                "job_id": get("job_id"),
            }),
        ),
        "otcpJobNotFound" => Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            message,
            serde_json::json!({
                "reason": "otcpJobNotFound",
                "job_id": get("job_id"),
            }),
        ),
        "otcpNotesUnavailable" => Fault::domain_with_data(
            ErrorCode::EntityNotFound,
            message,
            serde_json::json!({
                "reason": "otcpNotesUnavailable",
                "job_id": get("job_id"),
                "content_id": get("content_id"),
            }),
        ),
        "otcpNotConfigured" => Fault::domain_with_data(
            ErrorCode::IoError,
            message,
            serde_json::json!({
                "reason": "otcpNotConfigured",
            }),
        ),
        "otcpSourceNotWav" => Fault::domain_with_data(
            ErrorCode::IoError,
            message,
            serde_json::json!({
                "reason": "otcpSourceNotWav",
                "track_id": get("track_id"),
                "clip_id": get("clip_id"),
                "detail": get("detail"),
            }),
        ),
        "otcpSourceLicenseMissing" => Fault::domain_with_data(
            ErrorCode::Conflict,
            message,
            serde_json::json!({
                "reason": "otcpSourceLicenseMissing",
                "track_id": get("track_id"),
                "clip_id": get("clip_id"),
                "asset": get("asset"),
            }),
        ),
        "otcpTimeGridUnsupported" => Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            message,
            serde_json::json!({
                "reason": "otcpTimeGridUnsupported",
                "job_id": get("job_id"),
            }),
        ),
        // 驱动的 `REASONS` 只有上面 19 个；走到这里说明驱动发了未知 reason，
        // 按"服务端已不可信"处理（原 reason 进 message，不进载荷键集）。
        _ => Fault::domain_with_data(
            ErrorCode::IoError,
            format!("未知 OTCP 错误（{}）：{message}", error.reason),
            serde_json::json!({
                "reason": "otcpServerGone",
                "job_id": get("job_id"),
            }),
        ),
    }
}

// ===========================================================================
// 实参解析（`validate_arguments` 只把类型关；值域关在这里）
// ===========================================================================

/// `keyRoot`：整数 0–11（C=0）。
fn parse_key_root(call: &ToolCall) -> Result<u8, Fault> {
    let value = call
        .arguments
        .get("keyRoot")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`keyRoot` 必须是 0–11 的整数（C=0）",
            )
        })?;
    if value > 11 {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`keyRoot` 须在 0–11 之间，实际 {value}"),
        ));
    }
    Ok(u8::try_from(value).expect("已验 ≤ 11"))
}

/// `scale`：须在 `VALID_SCALES` 里。
fn parse_scale(call: &ToolCall) -> Result<String, Fault> {
    let scale = super::arg_str(call, "scale").ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`scale` 必须是音阶名（major | minor | chromatic | harmonicMinor | dorian | mixolydian | pentatonicMajor | pentatonicMinor）",
        )
    })?;
    if VALID_SCALES.contains(&scale) {
        Ok(scale.to_owned())
    } else {
        Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!(
                "`scale` 非法：{scale}（须为 major | minor | chromatic | harmonicMinor | dorian | mixolydian | pentatonicMajor | pentatonicMinor）"
            ),
        ))
    }
}

/// `mode`：缺省 `snapNotes`；v1 只执行它。
fn parse_mode(call: &ToolCall) -> Result<String, Fault> {
    let mode = super::arg_str(call, "mode").unwrap_or("snapNotes");
    match mode {
        "snapNotes" => Ok(mode.to_owned()),
        "regenerate" | "notesOnly" => Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!(
                "模式 {mode} 未在 v1 开放（只执行 snapNotes；regenerate / notesOnly 须先在无头环境探针验证，见设计稿 §9）"
            ),
        )),
        other => Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`mode` 非法：{other}（须为 snapNotes | regenerate | notesOnly）"),
        )),
    }
}

/// `tuningHz`：有限正数，缺省 440。
fn parse_tuning_hz(call: &ToolCall) -> Result<f32, Fault> {
    let value = call
        .arguments
        .get("tuningHz")
        .and_then(Value::as_f64)
        .unwrap_or(440.0);
    if !value.is_finite() || value <= 0.0 || value > 20_000.0 {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`tuningHz` 须为 (0, 20000] 内的有限数，实际 {value}"),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as f32)
}

/// `noteOps`：对象数组（JSON 原样透传给 `content.editNotes`）。
fn parse_note_ops(call: &ToolCall) -> Result<Vec<Value>, Fault> {
    let Some(raw) = call.arguments.get("noteOps") else {
        return Ok(Vec::new());
    };
    let items = raw.as_array().ok_or_else(|| {
        Fault::domain(ErrorCode::InvalidParameterRange, "`noteOps` 必须是对象数组")
    })?;
    for item in items {
        if !item.is_object() {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`noteOps` 的每一项都必须是对象",
            ));
        }
    }
    Ok(items.clone())
}

/// `job_id`：必填；只接受 `[A-Za-z0-9_-]{1,64}`（收据路径的目录名，⛔ 不接受 `/` 与 `..`）。
fn parse_job_id(call: &ToolCall) -> Result<String, Fault> {
    let job_id = super::arg_str(call, "job_id").ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            "`job_id` 必须是字符串（提交时返回的 job_id）",
        )
    })?;
    if job_id.is_empty()
        || job_id.len() > 64
        || !job_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`job_id` 非法：{job_id}（只接受 [A-Za-z0-9_-]，至多 64 字符）"),
        ));
    }
    Ok(job_id.to_owned())
}

/// 读一个 `EntityId` 实参值（`trackId` / `clipId` 的可选形态）。
fn parse_id_arg(value: &Value, field: &str) -> Result<EntityId, Fault> {
    let text = value.as_str().ok_or_else(|| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 必须是 26 字符 ULID 字符串"),
        )
    })?;
    EntityId::from_str(text).map_err(|error| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{field}` 不是合法 ULID: {error}"),
        )
    })
}

// ===========================================================================
// 片段定位与源音频制备
// ===========================================================================

/// 定位片段：返回（源资产哈希，源资产许可）。
///
/// 许可缺失 ⇒ `otcpSourceLicenseMissing`（⛔ 不编造许可字符串）。
fn resolve_clip(
    project: &yeban_model::YebanProjectV1,
    track_id: &EntityId,
    clip_id: &EntityId,
) -> Result<(AssetHash, String), Fault> {
    project.tracks.get(track_id).ok_or_else(|| {
        Fault::domain(ErrorCode::TrackNotFound, format!("音轨 {track_id} 不存在"))
    })?;
    let clip = project
        .clip_pool
        .get(clip_id)
        .ok_or_else(|| Fault::domain(ErrorCode::ClipNotFound, format!("片段 {clip_id} 不存在")))?;
    let ClipContent::Audio { asset, .. } = &clip.content else {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("片段 {clip_id} 不是音频片段（OTCP 校正只接音频片段）"),
        ));
    };
    let license = project.assets.get(asset).map(|meta| meta.license.clone());
    match license {
        Some(license) => Ok((asset.clone(), license)),
        None => Err(fault_for(&OtcpError::with_data(
            "otcpSourceLicenseMissing",
            format!("源资产 {asset} 在工程 assets 索引里没有许可条目，拒绝校正（不编造许可）"),
            serde_json::json!({
                "track_id": track_id.to_canonical_string(),
                "clip_id": clip_id.to_canonical_string(),
                "asset": asset.as_str(),
            }),
        ))),
    }
}

/// 双重校正检查：轨道已挂 OpenTune 外部效果绑定 ⇒ 拒绝（驱动与效果二选一）。
fn check_double_correction(
    project: &yeban_model::YebanProjectV1,
    track_id: &EntityId,
) -> Result<(), Fault> {
    let Some(track) = project.tracks.get(track_id) else {
        return Ok(());
    };
    if let Some(device) = find_opentune_device(track) {
        return Err(fault_for(&OtcpError::with_data(
            "otcpDoubleCorrection",
            format!(
                "音轨 {} 已挂 OpenTune 外部效果绑定（设备 {}），驱动与效果二选一，拒绝重复校正",
                track_id, device.id
            ),
            serde_json::json!({
                "track_id": track_id.to_canonical_string(),
                "device_id": device.id.to_canonical_string(),
            }),
        )));
    }
    Ok(())
}

/// 轨道上是否挂了 OpenTune 外部效果器（名 / 绑定路径 / 类名含 `opentune`，不分大小写）。
fn find_opentune_device(track: &TrackV3) -> Option<&yeban_model::DeviceDefinition> {
    track.devices.iter().find(|device| {
        device.kind == DeviceKind::ExternalEffect
            && (device.name.to_lowercase().contains("opentune")
                || device.plugin.as_ref().is_some_and(|binding| {
                    binding.path.to_lowercase().contains("opentune")
                        || binding
                            .class
                            .as_ref()
                            .is_some_and(|class| class.to_lowercase().contains("opentune"))
                }))
    })
}

/// 源音频制备：解码 → 立体声拒绝 → 重采样到工程采样率 → 写 PCM16 单声道 WAV。
///
/// 返回（源 WAV 绝对路径，时长秒）。
fn prepare_source_wav(
    domain: &Domain,
    config: &OtcpConfig,
    job_id: &str,
    project: &yeban_model::YebanProjectV1,
    source_asset: &AssetHash,
    track_id: &EntityId,
    clip_id: &EntityId,
) -> Result<(PathBuf, f64), Fault> {
    let bytes = domain.asset(source_asset).ok_or_else(|| {
        fault_for(&OtcpError::with_data(
            "otcpContentGone",
            format!("源资产 {source_asset} 的字节不在会话 CAS 池里（工程未完整加载）"),
            serde_json::json!({
                "job_id": job_id,
                "content_id": clip_id.to_canonical_string(),
            }),
        ))
    })?;
    let source_not_wav = |detail: String| {
        fault_for(&OtcpError::with_data(
            "otcpSourceNotWav",
            format!("源音频解码失败：{detail}"),
            serde_json::json!({
                "track_id": track_id.to_canonical_string(),
                "clip_id": clip_id.to_canonical_string(),
                "detail": detail,
            }),
        ))
    };
    let decoded = yeban_decode::decode_bytes(bytes, &DecodeOptions::default())
        .map_err(|error| source_not_wav(error.to_string()))?;
    if decoded.channels() != 1 {
        return Err(fault_for(&OtcpError::with_data(
            "otcpStereoUnsupported",
            format!(
                "源音频是 {} 通道，v1 只接单声道片段（下混是后续决策，⛔ 不静默下混）",
                decoded.channels()
            ),
            serde_json::json!({
                "track_id": track_id.to_canonical_string(),
                "clip_id": clip_id.to_canonical_string(),
                "channels": decoded.channels(),
            }),
        )));
    }
    let rate = project.audio_config.sample_rate.hz();
    let resampled = resample_asset(&decoded, rate)
        .map_err(|error| source_not_wav(format!("重采样到 {rate} Hz 失败：{error}")))?;
    let samples = resampled.samples();
    #[allow(clippy::cast_precision_loss)]
    let duration_secs = samples.len() as f64 / f64::from(rate);
    let dir = config.job_dir(job_id);
    fs::create_dir_all(&dir).map_err(|error| {
        fault_for(&OtcpError::io(format!(
            "建任务目录 {} 失败：{error}",
            dir.display()
        )))
    })?;
    let path = dir.join("source.wav");
    write_wav_mono_pcm16(&path, rate, samples).map_err(|error| fault_for(&error))?;
    Ok((path, duration_secs))
}

// ===========================================================================
// 任务收据与索引
// ===========================================================================

/// 任务状态数据（`running` / `failed` / `cancelled` 的公共形状）。
fn job_summary(config: &OtcpConfig, receipt: &JobReceipt) -> Value {
    serde_json::json!({
        "jobId": receipt.job_id,
        "status": receipt.status,
        "dryRun": receipt.dry_run,
        "contentId": receipt.content_id,
        "notesBefore": {
            "count": receipt.notes_before.len(),
            "notes": notes_json(&receipt.notes_before),
        },
        "notesAfter": {
            "count": receipt.notes_after.len(),
            "notes": notes_json(&receipt.notes_after),
        },
        "measurement": receipt.measurement,
        "asset": Value::Null,
        "error": receipt.error,
        "server": receipt.server,
        "receiptPath": config.receipt_path(&receipt.job_id).display().to_string(),
    })
}

/// 成功任务的数据：`job_summary` + 导出资产（读导出文件算哈希）。
fn succeeded_data(config: &OtcpConfig, receipt: &JobReceipt) -> Result<Value, Fault> {
    let mut data = job_summary(config, receipt);
    let asset = match &receipt.export_path {
        None => Value::Null,
        Some(path) => {
            let bytes = fs::read(path).map_err(|_| {
                fault_for(&OtcpError::with_data(
                    "otcpContentGone",
                    format!(
                        "任务 {} 已成功，但导出文件 {} 已不在任务目录",
                        receipt.job_id, path
                    ),
                    serde_json::json!({
                        "job_id": receipt.job_id,
                        "content_id": receipt.content_id,
                    }),
                ))
            })?;
            serde_json::json!({
                "hash": AssetHash::of_bytes(&bytes).as_str(),
                "path": path,
                "byteLen": bytes.len(),
            })
        }
    };
    if let Some(object) = data.as_object_mut() {
        object.insert("asset".to_owned(), asset);
    }
    Ok(data)
}

/// 音符表（`Note` 的 JSON 形状与 OTCP `getNotes` 的返回一致）。
/// 音符表 → 响应 JSON（camelCase，与设计稿 §7 的形状一致）。
///
/// `Note` 自身按 serde 默认是 snake_case；响应面统一用 camelCase
///（与 `jobId` / `contentId` 等键同款），不在此展开 `params`——
/// v1 的 `Note` 不带参数字段（见 ADR-0070 偏差 8）。
fn notes_json(notes: &[Note]) -> Value {
    Value::Array(
        notes
            .iter()
            .map(|note| {
                serde_json::json!({
                    "index": note.index,
                    "startContentSec": note.start_content_sec,
                    "endContentSec": note.end_content_sec,
                    "detectedMidi": note.detected_midi,
                    "targetMidi": note.target_midi,
                    "offsetCents": note.offset_cents,
                    "corrected": note.corrected,
                })
            })
            .collect(),
    )
}

/// 提交响应里的服务端块（构建提交 + 自测记录；提交时构建尚未启动，`build` 为 null）。
fn server_block(status: &yeban_otcp::DriverStatus) -> Value {
    serde_json::json!({
        "binaryPresent": status.binary_present,
        "binaryPath": status.binary_path.display().to_string(),
        "build": Value::Null,
        "instanceId": Value::Null,
        "selfTest": status.self_test,
    })
}

/// 片段→任务索引：`<control_root>/clip-jobs/<track>-<clip>.json`。
fn clip_index_path(config: &OtcpConfig, track_id: &EntityId, clip_id: &EntityId) -> PathBuf {
    config.control_root.join("clip-jobs").join(format!(
        "{}-{}.json",
        track_id.to_canonical_string(),
        clip_id.to_canonical_string()
    ))
}

/// 写片段→任务索引（提交时；最新一次提交覆盖旧的）。
fn write_clip_index(
    config: &OtcpConfig,
    track_id: &EntityId,
    clip_id: &EntityId,
    job_id: &str,
) -> Result<(), OtcpError> {
    let path = clip_index_path(config, track_id, clip_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            OtcpError::io(format!("建索引目录 {} 失败：{error}", parent.display()))
        })?;
    }
    fs::write(&path, serde_json::json!({ "job_id": job_id }).to_string())
        .map_err(|error| OtcpError::io(format!("写片段索引 {} 失败：{error}", path.display())))?;
    Ok(())
}

/// 读片段→任务索引；无索引 ⇒ `otcpNotesUnavailable`。
fn read_clip_index(
    config: &OtcpConfig,
    track_id: &EntityId,
    clip_id: &EntityId,
) -> Result<String, Fault> {
    let unavailable = || {
        fault_for(&OtcpError::with_data(
            "otcpNotesUnavailable",
            format!("片段 {clip_id}（音轨 {track_id}）没有校正任务收据"),
            serde_json::json!({
                "job_id": Value::Null,
                "content_id": clip_id.to_canonical_string(),
            }),
        ))
    };
    let text = fs::read_to_string(clip_index_path(config, track_id, clip_id))
        .map_err(|_| unavailable())?;
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .get("job_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(unavailable)
}

/// 按 `contentId` 找收据：扫任务目录（上限 500 条），逐条比 `content_id`。
fn find_receipt_by_content(config: &OtcpConfig, content_id: &str) -> Result<JobReceipt, Fault> {
    let unavailable = || {
        fault_for(&OtcpError::with_data(
            "otcpNotesUnavailable",
            format!("内容 {content_id} 没有校正任务收据"),
            serde_json::json!({
                "job_id": Value::Null,
                "content_id": content_id,
            }),
        ))
    };
    let jobs_dir = config.control_root.join("jobs");
    let entries = fs::read_dir(&jobs_dir).map_err(|_| unavailable())?;
    for (scanned, entry) in entries.flatten().enumerate() {
        if scanned >= 500 {
            break;
        }
        let path = entry.path().join("receipt.json");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(receipt) = serde_json::from_str::<JobReceipt>(&text) else {
            continue;
        };
        if receipt.content_id.as_deref() == Some(content_id) {
            return Ok(receipt);
        }
    }
    Err(unavailable())
}

// ===========================================================================
// 交付计划
// ===========================================================================

/// 交付提案的标题（唯一键：`job_id` 是 ULID，全局唯一）。
fn delivery_title(job_id: &str) -> String {
    format!("音高校正 {job_id}")
}

/// 已有同任务的提案吗（上一轮 `mark_delivered` 失败后的补记口径）。
fn find_existing_proposal(domain: &Domain, job_id: &str) -> Option<String> {
    let title = delivery_title(job_id);
    domain
        .proposals
        .values()
        .find(|proposal| proposal.title == title)
        .map(|proposal| proposal.id.to_canonical_string())
}

/// 提案的摘要投影（与首次交付的响应形状一致）；记录不在内存里时只回 id。
fn proposal_summary(domain: &Domain, proposal_id: &str) -> Value {
    use std::str::FromStr as _;
    EntityId::from_str(proposal_id)
        .ok()
        .and_then(|id| domain.proposals.get(&id))
        .map_or_else(
            || serde_json::json!({ "proposalId": proposal_id }),
            |record| record.summary(),
        )
}

/// 成功任务的 plan：干跑 ⇒ 只回数据；已交付 ⇒ 只回数据；否则建交付计划。
fn plan_succeeded_job(
    domain: &Domain,
    config: &OtcpConfig,
    receipt: &JobReceipt,
) -> Result<Plan, Fault> {
    let mut data = succeeded_data(config, receipt)?;
    if receipt.dry_run {
        if let Some(object) = data.as_object_mut() {
            object.insert(
                "note".to_owned(),
                Value::from("dryRun：只做音符预测，未导出、未交付"),
            );
        }
        return Ok(Plan::OtcpJob { data });
    }
    if let Some(proposal_id) = &receipt.delivered_proposal_id {
        if let Some(object) = data.as_object_mut() {
            object.insert("proposal".to_owned(), proposal_summary(domain, proposal_id));
            object.insert(
                "note".to_owned(),
                Value::from("已交付（重复轮询不再重复建提案）"),
            );
        }
        return Ok(Plan::OtcpJob { data });
    }
    if let Some(proposal_id) = find_existing_proposal(domain, &receipt.job_id) {
        // 上一轮建了提案但收据没记上：补记（写失败也不拦本次轮询，下次再补）。
        let _ = mark_delivered(config, &receipt.job_id, &proposal_id);
        if let Some(object) = data.as_object_mut() {
            object.insert(
                "proposal".to_owned(),
                proposal_summary(domain, &proposal_id),
            );
            object.insert("note".to_owned(), Value::from("已交付（上一轮收据补记）"));
        }
        return Ok(Plan::OtcpJob { data });
    }
    let Ok(project) = require_active(domain) else {
        if let Some(object) = data.as_object_mut() {
            object.insert(
                "delivery".to_owned(),
                serde_json::json!({ "pending": true, "reason": "noActiveProject" }),
            );
            object.insert(
                "note".to_owned(),
                Value::from("任务已成功，但当前无活跃工程，交付待定；打开工程后重轮询即可交付"),
            );
        }
        return Ok(Plan::OtcpJob { data });
    };
    let track_id = EntityId::from_str(&receipt.track_id).map_err(|error| {
        fault_for(&OtcpError::protocol(format!(
            "收据音轨 id 损坏（{}）：{error}",
            receipt.track_id
        )))
    })?;
    let clip_id = EntityId::from_str(&receipt.clip_id).map_err(|error| {
        fault_for(&OtcpError::protocol(format!(
            "收据片段 id 损坏（{}）：{error}",
            receipt.clip_id
        )))
    })?;
    let gone = |detail: String| {
        fault_for(&OtcpError::with_data(
            "otcpContentGone",
            detail,
            serde_json::json!({
                "job_id": receipt.job_id,
                "content_id": clip_id.to_canonical_string(),
            }),
        ))
    };
    let track = project
        .tracks
        .get(&track_id)
        .ok_or_else(|| gone(format!("音轨 {track_id} 已不在工程里，交付无法定位片段")))?;
    let _ = track;
    let clip = project
        .clip_pool
        .get(&clip_id)
        .ok_or_else(|| gone(format!("片段 {clip_id} 已不在工程里，交付无法换资产")))?;
    let ClipContent::Audio { asset: current, .. } = &clip.content else {
        return Err(gone(format!(
            "片段 {clip_id} 已不是音频片段，交付无法换资产"
        )));
    };
    let source = AssetHash::parse(&receipt.source_asset_hash).map_err(|error| {
        fault_for(&OtcpError::protocol(format!(
            "收据源资产哈希损坏（{}）：{error}",
            receipt.source_asset_hash
        )))
    })?;
    if *current != source {
        return Err(fault_for(&OtcpError::with_data(
            "otcpRevisionConflict",
            format!(
                "片段 {clip_id} 的资产自提交后已变（提交时 {source}，现为 {current}），交付拒绝"
            ),
            serde_json::json!({ "job_id": receipt.job_id }),
        )));
    }
    let export_path = receipt.export_path.clone().ok_or_else(|| {
        gone(format!(
            "任务 {} 已成功，但收据里没有导出路径",
            receipt.job_id
        ))
    })?;
    let asset_bytes = fs::read(&export_path).map_err(|_| {
        gone(format!(
            "任务 {} 已成功，但导出文件 {export_path} 已不在任务目录",
            receipt.job_id
        ))
    })?;
    let new_hash = AssetHash::of_bytes(&asset_bytes);
    let new_meta = AssetMetadata {
        hash: new_hash.clone(),
        original_path: format!("otcp-corrected/{}.wav", receipt.job_id),
        byte_len: u64::try_from(asset_bytes.len()).unwrap_or(u64::MAX),
        media_kind: MediaKind::Audio,
        license: receipt.source_license.clone(),
    };
    let ops = vec![
        Op::SetAsset {
            asset_hash: new_hash.clone(),
            old_asset: project.assets.get(&new_hash).cloned(),
            new_asset: new_meta,
        },
        Op::SetClipAsset {
            clip_id,
            old_asset: current.clone(),
            new_asset: new_hash,
        },
    ];
    // 模拟：整批 op 必须能在当前工程上干净地施加（与 `propose_draft` 同一款）。
    let mut simulated = project.clone();
    Op::Batch {
        ops: ops.clone(),
        description: delivery_title(&receipt.job_id),
    }
    .apply(&mut simulated)
    .map_err(|failure| error::from_model("交付模拟", &failure))?;
    simulated
        .validate()
        .map_err(|failure| error::from_model("交付结果校验", &failure))?;
    let base_commit = domain.active_head().ok_or_else(|| {
        Fault::domain(ErrorCode::Conflict, "主分支没有头提交（提交图谱未初始化）")
    })?;
    let base_digest = super::store::digest_of(super::store::serialize_project(project)?.as_bytes());
    let draft = ProposalDraft {
        kind: "otcp-correct",
        title: delivery_title(&receipt.job_id),
        description: format!(
            "OTCP 音高校正：片段 {clip_id}（音轨 {track_id}）换校正后资产；旧资产保留（历史仍引用）"
        ),
        ops,
        base_commit,
        base_digest,
    };
    Ok(Plan::OtcpDeliver {
        draft: Box::new(draft),
        include_ops: false,
        // 交付警告恒为空：`proposal_warnings` 只在设备参数名未知（`effect_param_warnings`）
        // 与设备落静音轨（`device_on_silent_track_warnings`）时出警告，
        // `[SetAsset, SetClipAsset]` 两条 op 皆不触发，空是**实测结果**不是缺省。
        warnings: vec![],
        auto_merge: receipt.auto_merge,
        merge_commit_message: None,
        asset_bytes,
        job_id: receipt.job_id.clone(),
    })
}
