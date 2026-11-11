//! 校正流水线：导入 → F0 → 读谱 → snap → 验谱 → 导出 → 验导出 → 量测 → 交付。
//!
//! 步骤（设计稿 §3.1）：
//! 1. MCP 层已把源音频写成任务目录下的 WAV（48 kHz 工程采样率，单声道）；
//! 2. `import.start`（`idempotencyKey` = yeban 任务 id）→ 轮询 `job.get` 至终态；
//! 3. `project.get` → 取**最后**一个 content（应用自动打开的 demo 工程在下，
//!    导入落在上层）；记 `contentId` 与 placement 列表；
//! 4. 等 F0：轮询 `content.getDetails` 直到 `analysis.f0State == "ready"` ——
//!    只认 `f0State`；冷启动后首次轮询可花 ~60 s；`f0_failed` / `f0_empty`
//!    为终态；`NOT_READY` 继续轮询（遵守 `retryAfterMs`）；
//! 5. `content.getNotes`（游标分页；游标绑定 content 修订 + 音高移位），
//!    记 index / startContentSec / endContentSec / detectedMidi / targetMidi /
//!    offsetCents / corrected + 音符参数；修订令牌取最新 `asOf.contents[]`；
//! 6. `content.autoTune {mode: "snapNotes", expectedRevision（必填）, key}`；
//!    `REVISION_CONFLICT` → 重读令牌重试一次，再失败；
//! 7. 重读音符：每个被改的音符须 `corrected: true` 且 `targetMidi` 落在调性音阶上；
//! 8. `export.start {kind: "clip", target: {placementId}, allowDry: false}` →
//!    轮询 120 s；`export_wait_timeout` → 只重导一次；
//! 9. 验导出结果：44100 Hz / 1 通道 / float32 / contents 修订与第 7 步一致 /
//!    **`dryContents: []`**（非空 = 整次导出失败）；记 `processing`；
//! 10. 量测导出文件（`measure` 模块），过门才交付。
//!
//! 失败语义（设计稿 §4.1 / §6.2）：`RENDER_FAILED` 大声失败、
//! 什么都不写、⛔ 不重试、不回落干声；模态框先走 `recovery` 循环再上报。
//!
//! ⚠ `Key` / `Note` / `JobReceipt` 的形状是本 crate 与 `yeban-mcp`
//! 适配层的契约；改动须同步 `crates/yeban-mcp/src/domain/otcp.rs`。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::OtcpClient;
use crate::config::{DriverStatus, OtcpConfig, SelfTestRecord};
use crate::discovery::now_ms;
use crate::error::OtcpError;
use crate::measure::{Measurement, measure_export};
use crate::recovery::recover_modal;
use crate::supervisor::{ServerSession, shutdown_server, start_server};
use crate::wav::{read_wav, write_wav_mono_pcm16};

/// 调性（`content.autoTune` 的 `key`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    /// 根音（0–11，C=0）。
    pub root: u8,
    /// 音阶名（见 [`VALID_SCALES`]）。
    pub scale: String,
}

/// `key.scale` 的合法取值（OTCP `key.schema.json`）。
pub const VALID_SCALES: &[&str] = &[
    "major",
    "minor",
    "chromatic",
    "harmonicMinor",
    "dorian",
    "mixolydian",
    "pentatonicMajor",
    "pentatonicMinor",
];

impl Key {
    /// 校验根音与音阶名。
    pub fn validate(&self) -> Result<(), OtcpError> {
        if self.root > 11 {
            return Err(OtcpError::new(
                "otcpNotReady",
                format!("调性根音 {} 超出 0–11", self.root),
            ));
        }
        if !VALID_SCALES.contains(&self.scale.as_str()) {
            return Err(OtcpError::new(
                "otcpNotReady",
                format!("未知音阶 {}", self.scale),
            ));
        }
        Ok(())
    }

    /// 该音阶的音级集合（相对根音的半音偏移）。
    #[must_use]
    pub fn pitch_classes(&self) -> &'static [u8] {
        match self.scale.as_str() {
            "major" => &[0, 2, 4, 5, 7, 9, 11],
            "minor" => &[0, 2, 3, 5, 7, 8, 10],
            "chromatic" => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            "harmonicMinor" => &[0, 2, 3, 5, 7, 8, 11],
            "dorian" => &[0, 2, 3, 5, 7, 9, 10],
            "mixolydian" => &[0, 2, 4, 5, 7, 9, 10],
            "pentatonicMajor" => &[0, 2, 4, 7, 9],
            "pentatonicMinor" => &[0, 3, 5, 7, 10],
            _ => &[],
        }
    }

    /// `targetMidi` 是否落在该调性音阶上（取整后按半音判）。
    #[must_use]
    pub fn contains_midi(&self, target_midi: f32) -> bool {
        let pitch = target_midi.round() as i64;
        let pc = (pitch - i64::from(self.root)).rem_euclid(12) as u8;
        self.pitch_classes().contains(&pc)
    }
}

/// 检测到的音符（`content.getNotes` 的条目子集）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// 音符下标（content 内）。
    pub index: usize,
    /// 起始（content 内秒）。
    pub start_content_sec: f64,
    /// 结束（content 内秒）。
    pub end_content_sec: f64,
    /// 检测到的 MIDI 音高（可带小数）。
    pub detected_midi: f32,
    /// 目标 MIDI 音高。
    pub target_midi: f32,
    /// 相对目标的偏移（音分）。
    pub offset_cents: f32,
    /// 是否已被校正。
    pub corrected: bool,
}

impl Note {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            index: value.get("index")?.as_u64()? as usize,
            start_content_sec: value.get("startContentSec")?.as_f64()?,
            end_content_sec: value.get("endContentSec")?.as_f64()?,
            detected_midi: value.get("detectedMidi")?.as_f64()? as f32,
            target_midi: value.get("targetMidi")?.as_f64()? as f32,
            offset_cents: value
                .get("offsetCents")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32,
            corrected: value
                .get("corrected")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// 校正请求（MCP 适配层构造）。
#[derive(Clone, Debug)]
pub struct CorrectionRequest {
    /// yeban 任务 id（同时作 `import.start` 的 `idempotencyKey`）。
    pub job_id: String,
    /// 源 WAV（绝对路径，PCM16 单声道，工程采样率；MCP 层已备好）。
    pub source_wav: PathBuf,
    /// 源音频时长（秒，供导出时长校验 ±10 ms）。
    pub source_duration_secs: f64,
    /// 目标调性。
    pub key: Key,
    /// 模式：v1 只执行 `snapNotes`；其它值在驱动入口即拒。
    pub mode: String,
    /// A4 频率（Hz）。
    pub tuning_hz: f32,
    /// 干跑：只做音符预测，⛔ 不导出、不交付。
    pub dry_run: bool,
    /// `content.editNotes` 的 op 列表（JSON 原样透传；空 = 不做）。
    pub note_ops: Vec<Value>,
    /// 目标音轨 id（MCP 适配层填；收据带走，供交付时定位片段）。
    pub track_id: String,
    /// 目标片段 id（MCP 适配层填；收据带走，供交付时定位片段）。
    pub clip_id: String,
    /// 交付时是否立即合并提案（提交时记录，交付时沿用）。
    pub auto_merge: bool,
    /// 源资产哈希（十六进制；交付时核对片段资产未变）。
    pub source_asset_hash: String,
    /// 源资产许可（交付时新资产沿用）。
    pub source_license: String,
}

/// 任务收据（任务目录下的 `receipt.json`；MCP 适配层读它做交付）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobReceipt {
    /// yeban 任务 id。
    pub job_id: String,
    /// `running` | `succeeded` | `failed` | `cancelled`。
    pub status: String,
    /// 是否干跑。
    pub dry_run: bool,
    /// OpenTune content id（导入成功后才有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    /// 校正前音符表。
    #[serde(default)]
    pub notes_before: Vec<Note>,
    /// 校正后音符表（干跑时为预测值）。
    #[serde(default)]
    pub notes_after: Vec<Note>,
    /// 量测结果（导出成功后才有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<Measurement>,
    /// 导出文件路径（任务目录下，成功后才有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_path: Option<String>,
    /// 导出文件字节数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_byte_len: Option<u64>,
    /// 导出文件时长（秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_duration_secs: Option<f64>,
    /// 失败信息 `{reason, message, data}`（失败后才有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    /// 服务端块 `{build, instanceId, self_test}`。
    #[serde(default)]
    pub server: Value,
    /// `session.info` 的 `processing` 块（原样）。
    #[serde(default)]
    pub processing: Value,
    /// `session.info` 的 `editingScheme`。
    #[serde(default)]
    pub editing_scheme: String,
    /// 执行的模式。
    #[serde(default)]
    pub mode: String,
    /// 目标调性。
    pub key: Key,
    /// 目标音轨 id（提交时记录；交付时定位片段）。
    #[serde(default)]
    pub track_id: String,
    /// 目标片段 id（提交时记录；交付时定位片段）。
    #[serde(default)]
    pub clip_id: String,
    /// 交付时是否立即合并提案（提交时记录）。
    #[serde(default)]
    pub auto_merge: bool,
    /// 源资产哈希（十六进制；交付时核对片段资产未变）。
    #[serde(default)]
    pub source_asset_hash: String,
    /// 源资产许可（交付时新资产沿用）。
    #[serde(default)]
    pub source_license: String,
    /// A4 频率（Hz；提交时记录，供音符表工具回显）。
    #[serde(default = "default_tuning_hz")]
    pub tuning_hz: f32,
    /// 已交付的提案 id（交付后才有；重复轮询不再重复交付）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_proposal_id: Option<String>,
}

/// 老收据缺 `tuning_hz` 时的缺省（440 Hz）。
fn default_tuning_hz() -> f32 {
    440.0
}

impl JobReceipt {
    fn new_running(request: &CorrectionRequest) -> Self {
        Self {
            job_id: request.job_id.clone(),
            status: "running".to_owned(),
            dry_run: request.dry_run,
            content_id: None,
            notes_before: Vec::new(),
            notes_after: Vec::new(),
            measurement: None,
            export_path: None,
            export_byte_len: None,
            export_duration_secs: None,
            error: None,
            server: Value::Null,
            processing: Value::Null,
            editing_scheme: String::new(),
            mode: request.mode.clone(),
            key: request.key.clone(),
            track_id: request.track_id.clone(),
            clip_id: request.clip_id.clone(),
            auto_merge: request.auto_merge,
            source_asset_hash: request.source_asset_hash.clone(),
            source_license: request.source_license.clone(),
            tuning_hz: request.tuning_hz,
            delivered_proposal_id: None,
        }
    }

    fn failed(&mut self, error: &OtcpError) {
        self.status = "failed".to_owned();
        self.error = Some(json!({
            "reason": error.reason,
            "message": error.message,
            "data": error.data,
        }));
    }
}

/// 驱动入口。
pub struct OtcpDriver {
    config: OtcpConfig,
}

impl OtcpDriver {
    /// 新建驱动。
    #[must_use]
    pub fn new(config: OtcpConfig) -> Self {
        Self { config }
    }

    /// 驱动状态（不启动服务端）：二进制在位 + 自测记录。
    #[must_use]
    pub fn status(&self) -> DriverStatus {
        let self_test =
            read_self_test_record(&self.config).unwrap_or_else(|_| SelfTestRecord::unknown());
        DriverStatus {
            binary_present: self.config.opentune_bin.is_file(),
            binary_path: self.config.opentune_bin.clone(),
            self_test,
        }
    }

    /// 读任务收据；无收据 ⇒ `otcpJobNotFound`。
    pub fn read_receipt(&self, job_id: &str) -> Result<JobReceipt, OtcpError> {
        read_receipt(&self.config, job_id)
    }

    /// 整 clip 校正（阻塞当前线程；MCP 层在后台线程调用）。
    ///
    /// 成功 ⇒ `Ok(收据)`（`status == "succeeded"`）；失败 ⇒ `Err`
    /// 且收据已写 `failed`（MCP 层读收据拿结构化错误）。
    pub fn correct_clip(&self, request: CorrectionRequest) -> Result<JobReceipt, OtcpError> {
        request.key.validate()?;
        if request.mode != "snapNotes" {
            return Err(OtcpError::new(
                "otcpNotReady",
                format!(
                    "模式 {} 未在 v1 开放（只执行 snapNotes；regenerate / notesOnly 须先在无头环境探针验证，见设计稿 §9）",
                    request.mode
                ),
            ));
        }
        if !request.source_wav.is_absolute() {
            return Err(OtcpError::new(
                "otcpNotConfigured",
                format!("源 WAV 须为绝对路径：{}", request.source_wav.display()),
            ));
        }
        if request.source_wav.extension().and_then(|e| e.to_str()) != Some("wav") {
            return Err(OtcpError::new(
                "otcpNotConfigured",
                format!("源文件须为 .wav：{}", request.source_wav.display()),
            ));
        }
        let mut receipt = JobReceipt::new_running(&request);
        write_receipt(&self.config, &receipt);
        let result = self.correct_clip_inner(&request, &mut receipt);
        match result {
            Ok(()) => {
                receipt.status = "succeeded".to_owned();
                write_receipt(&self.config, &receipt);
                Ok(receipt)
            }
            Err(error) => {
                receipt.failed(&error);
                write_receipt(&self.config, &receipt);
                Err(error)
            }
        }
    }

    fn correct_clip_inner(
        &self,
        request: &CorrectionRequest,
        receipt: &mut JobReceipt,
    ) -> Result<(), OtcpError> {
        let config = &self.config;
        let job_id = &request.job_id;
        let deadline = Instant::now() + Duration::from_secs(config.timeouts.job_budget_secs);
        let mut session = start_server(config, job_id)?;
        receipt.server = json!({
            "build": session.build_commit,
            "instanceId": session.instance_id(),
        });
        receipt.processing = session.processing.clone();
        receipt.editing_scheme = session.editing_scheme.clone();
        write_receipt(config, receipt);

        let outcome = self.run_with_session(&mut session, request, receipt, deadline);
        // 关闭（每条路径都走）：先解模态框再退出。
        // 注：OTCP 的 import/export 任务 id 未在收据里跟踪，取消只对
        // yeban 任务 id 做 best-effort；服务端退出本身（quit/kill）
        // 已保证无残留进程。
        let job_ids = vec![job_id.clone()];
        shutdown_server(&mut session, config, &job_ids, &|client, id| {
            recover_modal(client, id, 3)
        });
        outcome
    }

    fn run_with_session(
        &self,
        session: &mut ServerSession,
        request: &CorrectionRequest,
        receipt: &mut JobReceipt,
        deadline: Instant,
    ) -> Result<(), OtcpError> {
        let config = &self.config;
        let job_id = &request.job_id;

        // 自测门（设计稿 §6.3）：该构建无记录时先跑一次再接任务；
        // fail 只注解错误，⛔ 不拦任务。
        let self_test = ensure_self_test(config, session, job_id)?;
        receipt.server["self_test"] = serde_json::to_value(&self_test).unwrap_or(Value::Null);
        write_receipt(config, receipt);

        let client = &mut session.client;

        // 2. 导入。
        let import_job = client.call(
            "import.start",
            json!({
                "items": [{"path": request.source_wav.display().to_string()}],
                "layout": "single",
                "trackId": 0,
                "startTimelineSec": 0,
                "generateNotes": true,
                "focus": false,
                "idempotencyKey": job_id,
            }),
            job_id,
        )?;
        let import_job_id = import_job
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        wait_job_terminal(client, job_id, &import_job_id, deadline)?;

        // 3. 取最后的 content。
        let (content_id, _placements) = last_content_id(client, job_id)?;
        receipt.content_id = Some(content_id.clone());
        write_receipt(config, receipt);

        // 4. 等 F0。
        let placement_id = wait_f0_ready(
            client,
            job_id,
            &content_id,
            Duration::from_secs(config.timeouts.f0_wait_secs),
            deadline,
        )?;

        // 5. 读谱（校正前）。
        let (notes_before, revision) = read_notes(client, job_id, &content_id)?;
        receipt.notes_before = notes_before.clone();
        write_receipt(config, receipt);
        let mut revision =
            revision.ok_or_else(|| OtcpError::protocol("getNotes 未返回修订令牌"))?;

        // 6. snap。
        revision = snap_notes(client, job_id, &content_id, &revision, &request.key)?;

        // 7. 重读并验谱。
        let (notes_after, fresh_revision) = read_notes(client, job_id, &content_id)?;
        verify_notes(&notes_before, &notes_after, &request.key)?;
        revision = fresh_revision.unwrap_or(revision);
        receipt.notes_after = notes_after.clone();
        write_receipt(config, receipt);

        // 7b. 可选 noteOps（editNotes；干跑时 dryRun: true 只预测）。
        if !request.note_ops.is_empty() {
            let new_revision = apply_note_ops(
                client,
                job_id,
                &content_id,
                &revision,
                &request.note_ops,
                request.dry_run,
            )?;
            revision = new_revision;
            let (notes_after_ops, fresh) = read_notes(client, job_id, &content_id)?;
            receipt.notes_after = notes_after_ops;
            if let Some(fresh) = fresh {
                revision = fresh;
            }
            write_receipt(config, receipt);
        }

        // 干跑：不导出、不交付；预测值已在收据里。
        if request.dry_run {
            return Ok(());
        }

        // 8. 导出（超时只重导一次）。
        let export_path = config.job_dir(job_id).join("export.wav");
        let export = export_clip(
            client,
            job_id,
            &placement_id,
            &export_path,
            Duration::from_secs(config.timeouts.export_wait_secs),
            deadline,
        )
        .map_err(|error| {
            enrich_render_failed(
                error,
                &notes_after,
                job_id,
                &content_id,
                &placement_id,
                &self_test,
                &session.build_commit,
            )
        })?;

        // 9. 验导出结果。
        let (hybrid_mode, backend) = verify_export(
            &export,
            &content_id,
            &revision,
            request.source_duration_secs,
        )?;

        // 10. 量测导出文件。
        let wav = read_wav(&export_path)?;
        let measurement = measure_export(
            &wav,
            &receipt.notes_after,
            config.measure_gate_cents,
            hybrid_mode,
            &backend,
        )?;
        receipt.measurement = Some(measurement);
        receipt.export_path = Some(export_path.display().to_string());
        receipt.export_byte_len = fs::metadata(&export_path).ok().map(|m| m.len());
        receipt.export_duration_secs = Some(wav.duration_secs());
        write_receipt(config, receipt);
        Ok(())
    }
}

/// 自测门：该构建无记录时跑一次（同一服务端上），记录后返回。
///
/// 自测夹具是合成信号（两段 440 Hz 正弦 + 0.5 s 无声间隙），测的是
/// 「无声间隙起始的渲染块」这一机制，不是原版 Shiguang 人声切片——
/// 真实切片只作 `T2` 真实二进制验收（见 `tests/real_binary.rs`），
/// 驱动⛔ 不把真实人声录音当运行时资产打包。
/// `fail` 只注解错误，⛔ 不拦任务；自测本身跑不起来 ⇒ 返回 `Err`。
fn ensure_self_test(
    config: &OtcpConfig,
    session: &mut ServerSession,
    job_id: &str,
) -> Result<SelfTestRecord, OtcpError> {
    let build_commit = session.build_commit.clone();
    let record = read_self_test_record(config)
        .ok()
        .filter(|record| record.matches_build(&build_commit));
    if let Some(record) = record {
        return Ok(record);
    }
    let record = run_self_test(config, session, job_id)?;
    write_self_test_record(config, &record);
    Ok(record)
}

/// 跑一次自测（导入合成夹具 → snap 到 A 大调 → 导出）。
fn run_self_test(
    config: &OtcpConfig,
    session: &mut ServerSession,
    job_id: &str,
) -> Result<SelfTestRecord, OtcpError> {
    let at = iso_now();
    let fixture_path = config.job_tmp_dir(job_id).join("self-test-source.wav");
    write_self_test_fixture(&fixture_path)?;
    let client = &mut session.client;
    let deadline = Instant::now() + Duration::from_secs(config.timeouts.job_budget_secs);
    let import_job = client.call(
        "import.start",
        json!({
            "items": [{"path": fixture_path.display().to_string()}],
            "layout": "single",
            "trackId": 0,
            "startTimelineSec": 0,
            "generateNotes": true,
            "focus": false,
            "idempotencyKey": format!("{job_id}-selftest"),
        }),
        job_id,
    )?;
    let import_job_id = import_job
        .get("jobId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    wait_job_terminal(client, job_id, &import_job_id, deadline)?;
    let (content_id, _) = last_content_id(client, job_id)?;
    let placement_id = wait_f0_ready(
        client,
        job_id,
        &content_id,
        Duration::from_secs(config.timeouts.f0_wait_secs),
        deadline,
    )?;
    let (_, revision) = read_notes(client, job_id, &content_id)?;
    let revision = revision.ok_or_else(|| OtcpError::protocol("自测 getNotes 未返回修订令牌"))?;
    let key = Key {
        root: 9,
        scale: "major".to_owned(),
    };
    let _ = snap_notes(client, job_id, &content_id, &revision, &key)?;
    let export_path = config.job_tmp_dir(job_id).join("self-test-export.wav");
    let export_result = export_clip(
        client,
        job_id,
        &placement_id,
        &export_path,
        Duration::from_secs(config.timeouts.export_wait_secs),
        deadline,
    );
    let unvoiced_gap = match export_result {
        Ok(_) => "pass".to_owned(),
        Err(error) if error.reason == "otcpRenderFailed" => "fail".to_owned(),
        Err(error) => return Err(error),
    };
    Ok(SelfTestRecord {
        build_commit: session.build_commit.clone(),
        unvoiced_gap,
        at,
    })
}

/// 自测夹具：两段 440 Hz 正弦（各 1.0 s）+ 0.5 s 无声间隙，48 kHz 单声道。
fn write_self_test_fixture(path: &Path) -> Result<(), OtcpError> {
    let sample_rate = 48_000u32;
    let mut samples = Vec::with_capacity(48_000 * 3);
    let tone =
        |i: usize| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin() * 0.5;
    for i in 0..48_000 {
        samples.push(tone(i));
    }
    samples.extend(std::iter::repeat_n(0.0, 24_000));
    for i in 0..48_000 {
        samples.push(tone(i));
    }
    write_wav_mono_pcm16(path, sample_rate, &samples)
}

/// 当前 UTC 时间（ISO 8601，秒精度；std-only 手算）。
fn iso_now() -> String {
    let secs = now_ms() / 1000;
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Unix 天数 → 民用日期（Howard Hinnant 算法）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 轮询 `job.get` 至终态；`failed` ⇒ 按服务端错误映射后返回 `Err`。
/// 返回终态任务的 `result`。
fn wait_job_terminal(
    client: &mut OtcpClient,
    job_id: &str,
    otcp_job_id: &str,
    deadline: Instant,
) -> Result<Value, OtcpError> {
    loop {
        if Instant::now() > deadline {
            return Err(OtcpError::new(
                "otcpNotReady",
                format!("等任务 {otcp_job_id} 终态时整任务预算耗尽"),
            ));
        }
        let state = client.call(
            "job.get",
            json!({"jobId": otcp_job_id, "waitMs": 30_000}),
            job_id,
        )?;
        let job = state.get("job").cloned().unwrap_or(Value::Null);
        let state_name = job
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        match state_name.as_str() {
            "succeeded" => {
                return Ok(job.get("result").cloned().unwrap_or(Value::Null));
            }
            "failed" => {
                let error = job.get("error").cloned().unwrap_or(Value::Null);
                let kind = error
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("INTERNAL");
                let reason = error.get("reason").and_then(Value::as_str).unwrap_or("");
                let detail = error.get("detail").and_then(Value::as_str).unwrap_or("");
                let mut driver_error = OtcpError::from_otcp(kind, reason, detail, job_id);
                // 失败块等结构化上下文原样透传进 data（供 §6.2 上报）。
                if let Value::Object(map) = &mut driver_error.data {
                    for (key, value) in error.as_object().map(|o| o.iter()).into_iter().flatten() {
                        if !matches!(key.as_str(), "kind" | "reason" | "detail") {
                            map.insert(key.clone(), value.clone());
                        }
                    }
                }
                return Err(driver_error);
            }
            "cancelled" => {
                return Err(OtcpError::new(
                    "otcpServerGone",
                    format!("任务 {otcp_job_id} 被取消"),
                ));
            }
            "queued" | "running" | "" => {
                std::thread::sleep(Duration::from_millis(200));
            }
            other => {
                return Err(OtcpError::protocol(format!("未知的任务状态 {other}")));
            }
        }
    }
}

/// `project.get` → 取最后的 content id。
fn last_content_id(
    client: &mut OtcpClient,
    job_id: &str,
) -> Result<(String, Vec<Value>), OtcpError> {
    let project = client.call("project.get", json!({}), job_id)?;
    let contents = project
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(id) = contents
        .last()
        .and_then(|last| last.get("contentId"))
        .and_then(Value::as_str)
    {
        return Ok((id.to_owned(), contents));
    }
    if let Some(id) = project.get("contentId").and_then(Value::as_str) {
        return Ok((id.to_owned(), contents));
    }
    Err(OtcpError::protocol(
        "project.get 未返回任何 content（导入可能未落盘）",
    ))
}

/// 等 F0 就绪：轮询 `content.getDetails` 直到 `analysis.f0State == "ready"`。
/// 返回 `viaPlacementId`（导出用）。
fn wait_f0_ready(
    client: &mut OtcpClient,
    job_id: &str,
    content_id: &str,
    budget: Duration,
    deadline: Instant,
) -> Result<String, OtcpError> {
    let start = Instant::now();
    loop {
        if Instant::now() > deadline || start.elapsed() > budget {
            return Err(OtcpError::new(
                "otcpNotReady",
                format!(
                    "等 content {content_id} 的 F0 超出预算（{} 秒）",
                    budget.as_secs()
                ),
            ));
        }
        match client.call(
            "content.getDetails",
            json!({"contentId": content_id}),
            job_id,
        ) {
            Ok(details) => {
                let f0_state = details
                    .get("analysis")
                    .and_then(|a| a.get("f0State"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match f0_state {
                    "ready" => {
                        let placement = details
                            .get("viaPlacementId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        if placement.is_empty() {
                            return Err(OtcpError::protocol("getDetails 未返回 viaPlacementId"));
                        }
                        return Ok(placement);
                    }
                    "f0_failed" | "f0_empty" => {
                        return Err(OtcpError::with_data(
                            "otcpNotReady",
                            format!("F0 分析终态失败：{f0_state}"),
                            json!({"content_id": content_id, "f0State": f0_state}),
                        ));
                    }
                    _ => {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            Err(error) if error.reason == "otcpNotReady" => {
                let wait_ms = error
                    .data
                    .get("retryAfterMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(500);
                std::thread::sleep(Duration::from_millis(wait_ms));
            }
            Err(error) => return Err(error),
        }
    }
}

/// 读全部音符（游标分页）；返回（音符，最新修订令牌）。
fn read_notes(
    client: &mut OtcpClient,
    job_id: &str,
    content_id: &str,
) -> Result<(Vec<Note>, Option<String>), OtcpError> {
    let mut notes = Vec::new();
    let mut cursor: Option<String> = None;
    let mut revision: Option<String> = None;
    loop {
        let mut params = json!({"contentId": content_id, "limit": 200});
        if let Some(cursor_value) = &cursor {
            params["cursor"] = Value::String(cursor_value.clone());
        }
        let result = client.call("content.getNotes", params, job_id)?;
        if let Some(token) = revision_for(content_id, &result) {
            revision = Some(token);
        }
        if let Some(items) = result.get("notes").and_then(Value::as_array) {
            for item in items {
                if let Some(note) = Note::from_value(item) {
                    notes.push(note);
                }
            }
        }
        match result.get("nextCursor").and_then(Value::as_str) {
            Some(next) if !next.is_empty() => cursor = Some(next.to_owned()),
            _ => break,
        }
    }
    Ok((notes, revision))
}

/// 从 `asOf.contents[]` 或 `tokens.contents[]` 取某 content 的修订令牌。
fn revision_for(content_id: &str, value: &Value) -> Option<String> {
    for key in ["asOf", "tokens"] {
        if let Some(contents) = value
            .get(key)
            .and_then(|v| v.get("contents"))
            .and_then(Value::as_array)
        {
            for entry in contents {
                let matches = entry.get("contentId").and_then(Value::as_str) == Some(content_id);
                if !matches {
                    continue;
                }
                if let Some(revision) = entry.get("revision").and_then(Value::as_str) {
                    return Some(revision.to_owned());
                }
            }
        }
    }
    None
}

/// `content.autoTune`（snapNotes）：`REVISION_CONFLICT` → 重读令牌重试一次；
/// `BUSY(modal_open)` → 走恢复循环重试一次。
/// 返回新的修订令牌。
fn snap_notes(
    client: &mut OtcpClient,
    job_id: &str,
    content_id: &str,
    revision: &str,
    key: &Key,
) -> Result<String, OtcpError> {
    let params = json!({
        "contentId": content_id,
        "mode": "snapNotes",
        "expectedRevision": revision,
        "key": {"root": key.root, "scale": key.scale},
    });
    match client.call("content.autoTune", params.clone(), job_id) {
        Ok(result) => Ok(revision_for(content_id, &result).unwrap_or_else(|| revision.to_owned())),
        Err(error) if error.reason == "otcpRevisionConflict" => {
            let (_, fresh) = read_notes(client, job_id, content_id)?;
            let fresh = fresh.ok_or_else(|| OtcpError::protocol("冲突重试时未读到新修订令牌"))?;
            let mut retry_params = params.clone();
            retry_params["expectedRevision"] = Value::String(fresh.clone());
            let result = client.call("content.autoTune", retry_params, job_id)?;
            Ok(revision_for(content_id, &result).unwrap_or(fresh))
        }
        Err(error) if error.reason == "otcpModalBlocked" => {
            recover_modal(client, job_id, 3)?;
            let result = client.call("content.autoTune", params, job_id)?;
            Ok(revision_for(content_id, &result).unwrap_or_else(|| revision.to_owned()))
        }
        Err(error) => Err(error),
    }
}

/// 验谱：每个被改的音符须 `corrected: true` 且 `targetMidi` 落在调性音阶上。
fn verify_notes(before: &[Note], after: &[Note], key: &Key) -> Result<(), OtcpError> {
    for (before_note, after_note) in before.iter().zip(after.iter()) {
        let changed = (before_note.target_midi - after_note.target_midi).abs() > f32::EPSILON
            || (before_note.offset_cents - after_note.offset_cents).abs() > f32::EPSILON;
        if !changed {
            continue;
        }
        if !after_note.corrected {
            return Err(OtcpError::with_data(
                "otcpNotReady",
                format!("音符 {} 被改后 corrected 仍为 false", after_note.index),
                json!({"note_index": after_note.index}),
            ));
        }
        if !key.contains_midi(after_note.target_midi) {
            return Err(OtcpError::with_data(
                "otcpNotReady",
                format!(
                    "音符 {} 的 targetMidi {} 不在调性音阶上",
                    after_note.index, after_note.target_midi
                ),
                json!({"note_index": after_note.index, "targetMidi": after_note.target_midi}),
            ));
        }
    }
    Ok(())
}

/// `content.editNotes`（noteOps 透传；干跑时 `dryRun: true` 只预测）。
/// 返回新的修订令牌。
fn apply_note_ops(
    client: &mut OtcpClient,
    job_id: &str,
    content_id: &str,
    revision: &str,
    ops: &[Value],
    dry_run: bool,
) -> Result<String, OtcpError> {
    let result = client.call(
        "content.editNotes",
        json!({
            "contentId": content_id,
            "expectedRevision": revision,
            "ops": ops,
            "dryRun": dry_run,
        }),
        job_id,
    )?;
    Ok(revision_for(content_id, &result).unwrap_or_else(|| revision.to_owned()))
}

/// 导出 clip（`export.start` + 轮询）；`export_wait_timeout` → 只重导一次。
/// 返回导出任务的 `result`。
fn export_clip(
    client: &mut OtcpClient,
    job_id: &str,
    placement_id: &str,
    export_path: &Path,
    budget: Duration,
    deadline: Instant,
) -> Result<Value, OtcpError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let start = client.call(
            "export.start",
            json!({
                "kind": "clip",
                "target": {"placementId": placement_id},
                "path": export_path.display().to_string(),
                "ifExists": "error",
                "allowDry": false,
            }),
            job_id,
        )?;
        let export_job_id = start
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let export_deadline = Instant::now() + budget;
        let export_deadline = export_deadline.min(deadline);
        match wait_job_terminal(client, job_id, &export_job_id, export_deadline) {
            Ok(result) => return Ok(result),
            Err(error) if error.reason == "otcpNotReady" && attempt == 1 => {
                // 导出等待超时（预算内未终态）：重导一次。
                continue;
            }
            Err(error) => return Err(error),
        }
    }
}

/// `RENDER_FAILED` 的结构化上报（设计稿 §6.2）：失败块 + 音符间隙 +
/// 构建提交 + 自测记录 + 提示。⛔ 不写任何文件、不重试。
fn enrich_render_failed(
    error: OtcpError,
    notes_after: &[Note],
    job_id: &str,
    content_id: &str,
    placement_id: &str,
    self_test: &SelfTestRecord,
    build_commit: &str,
) -> OtcpError {
    if error.reason != "otcpRenderFailed" {
        return error;
    }
    let data = &error.data;
    let block = data.get("block").cloned().unwrap_or(Value::Null);
    let start_sample = block
        .get("startSample")
        .and_then(Value::as_u64)
        .or_else(|| data.get("startSample").and_then(Value::as_u64));
    let start_sec = start_sample.map(|s| s as f64 / 44_100.0);
    let note_gap = start_sec.map(|sec| gap_around(notes_after, sec));
    let mut enriched = json!({
        "job_id": job_id,
        "content_id": content_id,
        "placement_id": placement_id,
        "block": block,
        "note_gap": note_gap,
        "otcp_build": build_commit,
        "self_test": {
            "build_commit": self_test.build_commit,
            "unvoiced_gap": self_test.unvoiced_gap,
            "at": self_test.at,
        },
        "hint": "unvoiced-gap pitch-shift failure; see DESIGN-otcp-driver.md §6",
    });
    if let Value::Object(map) = &mut enriched {
        for (key, value) in data.as_object().map(|o| o.iter()).into_iter().flatten() {
            if !map.contains_key(key) {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    OtcpError::with_data(
        "otcpRenderFailed",
        format!(
            "OpenTune 渲染失败（{}）：{}",
            error
                .data
                .get("otcpReason")
                .and_then(Value::as_str)
                .unwrap_or("render_chunk_failed"),
            error.message
        ),
        enriched,
    )
}

/// 找包含 `sec` 的音符间隙：`{prev_note_end_sec, next_note_start_sec}`。
fn gap_around(notes: &[Note], sec: f64) -> Value {
    let mut prev_end: Option<f64> = None;
    let mut next_start: Option<f64> = None;
    for note in notes {
        if note.end_content_sec <= sec {
            prev_end = Some(note.end_content_sec);
        } else if note.start_content_sec >= sec {
            next_start = Some(note.start_content_sec);
            break;
        }
    }
    json!({
        "prev_note_end_sec": prev_end,
        "next_note_start_sec": next_start,
    })
}

/// 验导出结果（设计稿 §3.1 第 9 步）。
/// 返回 `(hybrid_mode, backend)`（上报值，原样进收据）。
fn verify_export(
    export: &Value,
    content_id: &str,
    revision: &str,
    source_duration_secs: f64,
) -> Result<(bool, String), OtcpError> {
    let sample_rate = export
        .get("sampleRateHz")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if sample_rate != 44_100 {
        return Err(OtcpError::protocol(format!(
            "导出采样率 {sample_rate} Hz，不是 44100 Hz"
        )));
    }
    let channels = export
        .get("channelCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if channels != 1 {
        return Err(OtcpError::protocol(format!(
            "导出通道数 {channels}，不是 1"
        )));
    }
    let format = export
        .get("sampleFormat")
        .and_then(Value::as_str)
        .unwrap_or("");
    if format != "float32" {
        return Err(OtcpError::protocol(format!(
            "导出采样格式 {format}，不是 float32"
        )));
    }
    let dry = export
        .get("dryContents")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if dry != 0 {
        return Err(OtcpError::with_data(
            "otcpRenderFailed",
            "导出结果含 dry 内容（整次导出失败）",
            json!({"dryContents": export.get("dryContents")}),
        ));
    }
    let contents = export
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let matched = contents.iter().any(|c| {
        c.get("contentId").and_then(Value::as_str) == Some(content_id)
            && c.get("revision").and_then(Value::as_str) == Some(revision)
    });
    if !matched {
        return Err(OtcpError::protocol(format!(
            "导出 contents 的修订与验谱后不一致（content {content_id}）"
        )));
    }
    let duration = export
        .get("durationSec")
        .and_then(Value::as_f64)
        .unwrap_or(-1.0);
    if (duration - source_duration_secs).abs() > 0.010 {
        return Err(OtcpError::protocol(format!(
            "导出时长 {duration:.3} s 与源 {source_duration_secs:.3} s 相差超 10 ms"
        )));
    }
    let processing = export.get("processing").cloned().unwrap_or(Value::Null);
    let hybrid_mode = processing
        .get("hybridMode")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let backend = processing
        .get("backend")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok((hybrid_mode, backend))
}

/// 写收据（临时文件 + 重命名，原子落盘）。
pub fn write_receipt(config: &OtcpConfig, receipt: &JobReceipt) {
    let dir = config.job_dir(&receipt.job_id);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = dir.join("receipt.json.tmp");
    let final_path = dir.join("receipt.json");
    let Ok(text) = serde_json::to_string_pretty(receipt) else {
        return;
    };
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &final_path);
    }
}

/// 写初始 `running` 收据（MCP 提交侧在派生后台线程**之前**落盘，⛔ 不等 worker；
/// 这样提交响应里的 `receipt_path` 立即可读，轮询不会在最初几毫秒撞上
/// `otcpJobNotFound`）。写失败 ⇒ `Err`（提交侧响亮拒绝，不派生线程）。
///
/// # Errors
///
/// 任务目录建不出 / 收据写不出 ⇒ `otcpServerGone`（本地 IO 失败）。
pub fn write_initial_receipt(
    config: &OtcpConfig,
    request: &CorrectionRequest,
) -> Result<(), OtcpError> {
    let receipt = JobReceipt::new_running(request);
    let dir = config.job_dir(&receipt.job_id);
    fs::create_dir_all(&dir)
        .map_err(|error| OtcpError::io(format!("建任务目录 {} 失败: {error}", dir.display())))?;
    let text = serde_json::to_string_pretty(&receipt)
        .map_err(|error| OtcpError::io(format!("序列化初始收据失败: {error}")))?;
    let tmp = dir.join("receipt.json.tmp");
    let final_path = dir.join("receipt.json");
    fs::write(&tmp, text)
        .map_err(|error| OtcpError::io(format!("写初始收据 {} 失败: {error}", tmp.display())))?;
    fs::rename(&tmp, &final_path).map_err(|error| {
        OtcpError::io(format!(
            "初始收据落盘 {} 失败: {error}",
            final_path.display()
        ))
    })?;
    Ok(())
}

/// 给收据记交付（`delivered_proposal_id`；重复轮询凭它认出"已交付"，⛔ 不重复建提案）。
///
/// # Errors
///
/// 收据不存在 / 读不出 / 写不出 ⇒ 对应 `OtcpError`。
pub fn mark_delivered(
    config: &OtcpConfig,
    job_id: &str,
    proposal_id: &str,
) -> Result<(), OtcpError> {
    let mut receipt = read_receipt(config, job_id)?;
    receipt.delivered_proposal_id = Some(proposal_id.to_owned());
    let dir = config.job_dir(job_id);
    let text = serde_json::to_string_pretty(&receipt)
        .map_err(|error| OtcpError::io(format!("序列化交付收据失败: {error}")))?;
    let tmp = dir.join("receipt.json.tmp");
    let final_path = dir.join("receipt.json");
    fs::write(&tmp, text)
        .map_err(|error| OtcpError::io(format!("写交付收据 {} 失败: {error}", tmp.display())))?;
    fs::rename(&tmp, &final_path).map_err(|error| {
        OtcpError::io(format!(
            "交付收据落盘 {} 失败: {error}",
            final_path.display()
        ))
    })?;
    Ok(())
}

/// 读收据；无收据 ⇒ `otcpJobNotFound`。
pub fn read_receipt(config: &OtcpConfig, job_id: &str) -> Result<JobReceipt, OtcpError> {
    let path = config.receipt_path(job_id);
    let text = fs::read_to_string(&path).map_err(|_| {
        OtcpError::with_data(
            "otcpJobNotFound",
            format!("任务 {job_id} 无收据（未提交或已被清理）"),
            json!({"job_id": job_id}),
        )
    })?;
    serde_json::from_str::<JobReceipt>(&text)
        .map_err(|error| OtcpError::protocol(format!("解析收据 {} 失败: {error}", path.display())))
}

/// 读自测记录；无记录 ⇒ `Err`（调用方视作 `unknown`）。
fn read_self_test_record(config: &OtcpConfig) -> Result<SelfTestRecord, OtcpError> {
    let text = fs::read_to_string(config.self_test_path())
        .map_err(|error| OtcpError::io(format!("读自测记录失败: {error}")))?;
    serde_json::from_str::<SelfTestRecord>(&text)
        .map_err(|error| OtcpError::protocol(format!("解析自测记录失败: {error}")))
}

/// 写自测记录（原子落盘）。
fn write_self_test_record(config: &OtcpConfig, record: &SelfTestRecord) {
    if fs::create_dir_all(&config.control_root).is_err() {
        return;
    }
    let tmp = config.control_root.join("self-test.json.tmp");
    let final_path = config.self_test_path();
    let Ok(text) = serde_json::to_string_pretty(record) else {
        return;
    };
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &final_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_scale_membership() {
        let key = Key {
            root: 9,
            scale: "major".to_owned(),
        };
        // A 大调：A B C# D E F# G#。
        assert!(key.contains_midi(69.0)); // A4
        assert!(key.contains_midi(71.0)); // B4
        assert!(key.contains_midi(73.0)); // C#5
        assert!(!key.contains_midi(70.0)); // A#4
        assert!(!key.contains_midi(72.0)); // C5
    }

    #[test]
    fn key_validation_rejects_bad_root_and_scale() {
        assert!(
            Key {
                root: 12,
                scale: "major".to_owned()
            }
            .validate()
            .is_err()
        );
        assert!(
            Key {
                root: 0,
                scale: "blues".to_owned()
            }
            .validate()
            .is_err()
        );
        assert!(
            Key {
                root: 0,
                scale: "dorian".to_owned()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn verify_notes_catches_uncorrected_change() {
        let before = vec![Note {
            index: 0,
            start_content_sec: 0.0,
            end_content_sec: 1.0,
            detected_midi: 69.4,
            target_midi: 69.0,
            offset_cents: 40.0,
            corrected: false,
        }];
        let after = vec![Note {
            target_midi: 71.0,
            offset_cents: 0.0,
            corrected: false,
            ..before[0].clone()
        }];
        let key = Key {
            root: 9,
            scale: "major".to_owned(),
        };
        assert!(verify_notes(&before, &after, &key).is_err());
        let after_ok = vec![Note {
            target_midi: 71.0,
            offset_cents: 0.0,
            corrected: true,
            ..before[0].clone()
        }];
        assert!(verify_notes(&before, &after_ok, &key).is_ok());
        // 改到调外音高 ⇒ 拒。
        let after_bad = vec![Note {
            target_midi: 70.0,
            offset_cents: 0.0,
            corrected: true,
            ..before[0].clone()
        }];
        assert!(verify_notes(&before, &after_bad, &key).is_err());
    }

    #[test]
    fn verify_export_rejects_dry_contents() {
        let export = json!({
            "sampleRateHz": 44100,
            "channelCount": 1,
            "sampleFormat": "float32",
            "durationSec": 2.0,
            "contents": [{"contentId": "c1", "revision": "r1"}],
            "dryContents": [{"contentId": "c1"}],
            "processing": {"hybridMode": true, "backend": "cpu"}
        });
        let error = verify_export(&export, "c1", "r1", 2.0).expect_err("dry 应拒");
        assert_eq!(error.reason, "otcpRenderFailed");
    }

    #[test]
    fn verify_export_accepts_clean_result() {
        let export = json!({
            "sampleRateHz": 44100,
            "channelCount": 1,
            "sampleFormat": "float32",
            "durationSec": 2.0,
            "contents": [{"contentId": "c1", "revision": "r1"}],
            "dryContents": [],
            "processing": {"hybridMode": false, "backend": "cpu"}
        });
        let (hybrid, backend) = verify_export(&export, "c1", "r1", 2.0).expect("应过");
        assert!(!hybrid);
        assert_eq!(backend, "cpu");
    }

    #[test]
    fn gap_around_finds_between_notes() {
        let notes = vec![
            Note {
                index: 0,
                start_content_sec: 0.0,
                end_content_sec: 2.84,
                detected_midi: 69.0,
                target_midi: 69.0,
                offset_cents: 0.0,
                corrected: true,
            },
            Note {
                index: 1,
                start_content_sec: 3.02,
                end_content_sec: 4.0,
                detected_midi: 71.0,
                target_midi: 71.0,
                offset_cents: 0.0,
                corrected: true,
            },
        ];
        let gap = gap_around(&notes, 2.902);
        assert_eq!(gap["prev_note_end_sec"], 2.84);
        assert_eq!(gap["next_note_start_sec"], 3.02);
    }

    #[test]
    fn revision_token_read_from_asof() {
        let value = json!({
            "asOf": {"contents": [{"contentId": "c1", "revision": "r42"}]}
        });
        assert_eq!(revision_for("c1", &value).as_deref(), Some("r42"));
        assert_eq!(revision_for("c2", &value), None);
    }

    #[test]
    fn iso_now_is_wellformed() {
        let now = iso_now();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z'));
        assert_eq!(&now[4..5], "-");
    }

    #[test]
    fn receipt_roundtrips() {
        let config = OtcpConfig::default();
        let request = CorrectionRequest {
            job_id: "receipt-test-1".to_owned(),
            source_wav: PathBuf::from("/tmp/x.wav"),
            source_duration_secs: 2.0,
            key: Key {
                root: 9,
                scale: "major".to_owned(),
            },
            mode: "snapNotes".to_owned(),
            tuning_hz: 440.0,
            dry_run: false,
            note_ops: Vec::new(),
            track_id: "t1".to_owned(),
            clip_id: "c1".to_owned(),
            auto_merge: false,
            source_asset_hash: "aa".repeat(32),
            source_license: "CC0-1.0".to_owned(),
        };
        let mut receipt = JobReceipt::new_running(&request);
        receipt.status = "succeeded".to_owned();
        write_receipt(&config, &receipt);
        let read = read_receipt(&config, "receipt-test-1").expect("读回");
        assert_eq!(read.job_id, "receipt-test-1");
        assert_eq!(read.status, "succeeded");
        let _ = std::fs::remove_dir_all(config.job_dir("receipt-test-1"));
    }

    #[test]
    fn missing_receipt_is_job_not_found() {
        let config = OtcpConfig::default();
        let error = read_receipt(&config, "no-such-job-xyz").expect_err("无收据应拒");
        assert_eq!(error.reason, "otcpJobNotFound");
    }
}
