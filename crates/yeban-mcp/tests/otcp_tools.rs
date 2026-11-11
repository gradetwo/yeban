//! `yeban_correct_pitch` / `yeban_correct_pitch_job` / `yeban_correct_pitch_notes`
//! 的承重判据（`ADR-0070`）。
//!
//! ## 覆盖
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | 坏实参 ⇒ 既有 `INVALID_PARAMETER_RANGE`（不发明新码） | `correct_pitch_rejects_bad_arguments` |
//! | ② | `dryRun` ⇒ 只预览，状态一位不变 | `correct_pitch_dry_run_previews_without_side_effects` |
//! | ③ | 二进制未配置 ⇒ 响亮 `otcpNotConfigured`，不派生线程 | `correct_pitch_without_binary_fails_loudly` |
//! | ④ | 立体声源 ⇒ 响亮 `otcpStereoUnsupported` | `correct_pitch_refuses_stereo_source` |
//! | ⑤ | 轨道已挂 OpenTune 效果 ⇒ 响亮 `otcpDoubleCorrection` | `correct_pitch_refuses_double_correction` |
//! | ⑥ | 未知任务 / 无音符表 ⇒ 响亮 `ENTITY_NOT_FOUND` + 固定 reason | `correct_pitch_job_reports_unknown_job`、`correct_pitch_notes_reports_unavailable` |
//! | ⑦ | 成功任务经提案交付（默认只出提案，工程一位不变；重复轮询不重复建提案） | `correct_pitch_job_delivers_a_succeeded_job_through_a_proposal` |
//! | ⑧ | `autoMerge` ⇒ 当场合并，片段资产换新、旧资产保留 | `correct_pitch_job_auto_merge_applies_at_once` |
//! | ⑨ | 音符表工具读收据（片段寻址与内容寻址两条路） | `correct_pitch_notes_reads_notes_from_a_receipt` |
//! | ⑩ | 19 个 OTCP reason → MCP Fault 的映射全覆盖 | `otcp_fault_mapping_covers_every_reason` |
//!
//! ## 本机 vs 真实二进制
//!
//! 本文件**不**启动 OpenTune 服务端：提交的正向通路（真跑校正）在
//! `crates/yeban-otcp/tests/mock_server.rs`（mock 服务端）与
//! `crates/yeban-otcp/tests/real_binary.rs`（T1–T13，需二进制，
//! 见 `REAL_BINARY_HANDOVER.md`）覆盖。这里只测 MCP 适配层：
//! 实参关、预览、二进制缺失、源形状、双重校正、收据读写与交付。

use std::path::PathBuf;
use std::str::FromStr as _;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::otcp::fault_for;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::ErrorCode;
use yeban_model::samples::filled_project;
use yeban_model::{
    AssetHash, ClipContent, DeviceDefinition, DeviceKind, EntityId, PluginBinding, YebanProjectV1,
};
use yeban_otcp::OtcpError;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个新的分发器（内存工程，不碰文件系统）。
fn dispatcher_with_project(project: YebanProjectV1) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("fixture"), project, false)
        .expect("注入规范工程");
    dispatcher
}

fn unique_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-mcp-otcp-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的 `ToolResponse`（`result` 里的 JSON 对象）。
fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误（领域失败走 ToolResponse）: {outcome:?}"
    );
    let response = outcome.response.expect("响应");
    response.result.expect("result")
}

/// 工程的**规范化字节**（`undo_session` 的同一口径；逐字节回退判据用它）。
fn project_bytes(dispatcher: &Dispatcher) -> Vec<u8> {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    yeban_mcp::undo_session::canonical_project_bytes(project).expect("规范化字节")
}

/// 一份最小的 16-bit 单声道 WAV。
fn wav_s16(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &36_u32
            .saturating_add(samples.len() as u32 * 2)
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// 一份最小的 16-bit 立体声 WAV（左右声道同值）。
fn wav_s16_stereo(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &36_u32
            .saturating_add(samples.len() as u32 * 4)
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2_u16.to_le_bytes()); // 立体声
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32 * 4).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// 一份最小的 32-bit 浮点单声道 WAV（校正导出的形状：44.1 kHz float32）。
fn wav_f32(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &36_u32
            .saturating_add(samples.len() as u32 * 4)
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&32_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32 * 4).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// 一份独占临时目录里的音频文件（返回路径；调用方负责清理父目录）。
fn write_audio_fixture(tag: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-mcp-otcp-audio-{}-{tag}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建目录");
    let path = dir.join("fixture.wav");
    std::fs::write(&path, bytes).expect("写音频夹具");
    path
}

/// 工程里第一条音轨的 id。
fn first_track_id(project: &YebanProjectV1) -> EntityId {
    project.tracks.values().next().expect("样本里必须有音轨").id
}

/// 经 `yeban_import_audio` 真导入一份音频（返回片段 id 与资产哈希）。
fn import_audio_clip(dispatcher: &mut Dispatcher, bytes: &[u8]) -> (String, String) {
    let path = write_audio_fixture("import", bytes);
    let imported = call_tool(
        dispatcher,
        "yeban_import_audio",
        json!({
            "license": "CC0-1.0",
            "name": "TestClip",
            "path": path.display().to_string(),
            "gainDb": 0.0,
        }),
    );
    assert_eq!(imported["status"], "success", "{imported}");
    let clip_id = imported["data"]["clip"]["clipId"]
        .as_str()
        .expect("clipId")
        .to_owned();
    let asset = imported["data"]["clip"]["asset"]
        .as_str()
        .expect("asset")
        .to_owned();
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent).ok();
    }
    (clip_id, asset)
}

/// 把 `YEBAN_OPENTUNE_BIN` 指到一个**一定不存在**的路径（提交侧"二进制未配置"判据用）。
///
/// # Safety
///
/// 调用方只在本测试进程里用：`set_var` 是进程级环境；读它的只有 OTCP 适配层
/// （`OtcpConfig::default`），且本文件的用例对它的取值**一致**
/// （都指不存在的路径），并行跑也不存在可观测的竞态。
fn point_bin_at_missing_path() {
    let missing = std::env::temp_dir()
        .join("yeban-otcp-no-such-bin")
        .join("OpenTune");
    // SAFETY: 见本函数文档 —— 进程级环境，单进程测试二进制内取值一致。
    unsafe {
        std::env::set_var("YEBAN_OPENTUNE_BIN", &missing);
    }
}

/// OTCP 默认控制根（与 `OtcpConfig::default` 同一款：`temp_dir/yeban-otcp`）。
fn control_root() -> PathBuf {
    std::env::temp_dir().join("yeban-otcp")
}

/// 往默认控制根里写一份伪造的成功收据（交付 / 音符表判据用；不启动服务端）。
#[allow(clippy::too_many_arguments)]
fn write_receipt(
    job_id: &str,
    track_id: &str,
    clip_id: &str,
    source_asset_hash: &str,
    export_path: &str,
    export_byte_len: u64,
    auto_merge: bool,
    dry_run: bool,
    notes_after: Value,
) {
    let dir = control_root().join("jobs").join(job_id);
    std::fs::create_dir_all(&dir).expect("建任务目录");
    let receipt = json!({
        "job_id": job_id,
        "status": "succeeded",
        "dry_run": dry_run,
        "content_id": format!("c:{job_id}"),
        "notes_before": [
            {"index": 0, "start_content_sec": 0.0, "end_content_sec": 0.5,
             "detected_midi": 69.4, "target_midi": 69.0, "offset_cents": -40.0, "corrected": false},
            {"index": 1, "start_content_sec": 0.5, "end_content_sec": 1.0,
             "detected_midi": 71.2, "target_midi": 71.0, "offset_cents": -20.0, "corrected": false}
        ],
        "notes_after": notes_after,
        "measurement": {
            "median_cents": 3.0, "p90_cents": 8.0, "detection_rate": 0.95,
            "voiced_frames": 95, "total_frames": 100,
            "passed": true, "hybrid_mode": true, "backend": "cpu"
        },
        "export_path": export_path,
        "export_byte_len": export_byte_len,
        "export_duration_secs": 0.1,
        "error": Value::Null,
        "server": {"build": "mock-build-001", "instanceId": "mock-instance"},
        "processing": {"vocoderWeight": "hifigan.onnx", "hybridMode": true, "backend": "cpu", "tuningHz": 440},
        "editing_scheme": "openDyne",
        "mode": "snapNotes",
        "key": {"root": 9, "scale": "major"},
        "track_id": track_id,
        "clip_id": clip_id,
        "auto_merge": auto_merge,
        "source_asset_hash": source_asset_hash,
        "source_license": "CC0-1.0",
        "tuning_hz": 440.0,
        "delivered_proposal_id": Value::Null
    });
    std::fs::write(dir.join("receipt.json"), receipt.to_string()).expect("写收据");
}

/// 往默认控制根里写片段→任务索引（音符表工具的片段寻址用）。
fn write_clip_index(track_id: &str, clip_id: &str, job_id: &str) {
    let dir = control_root().join("clip-jobs");
    std::fs::create_dir_all(&dir).expect("建索引目录");
    let path = dir.join(format!("{track_id}-{clip_id}.json"));
    std::fs::write(path, json!({ "job_id": job_id }).to_string()).expect("写片段索引");
}

/// 0.1 秒 440 Hz 正弦（44.1 kHz float32，校正导出的形状）。
fn corrected_export_bytes() -> Vec<u8> {
    let samples: Vec<f32> = (0..4410)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin() * 0.5)
        .collect();
    wav_f32(44_100, &samples)
}

/// 伪造一份成功任务：写导出文件 + 收据，返回（任务 id，导出哈希）。
fn fabricate_succeeded_job(
    track_id: &str,
    clip_id: &str,
    source_asset_hash: &str,
    auto_merge: bool,
) -> (String, String) {
    let job_id = EntityId::new().to_canonical_string();
    let export_bytes = corrected_export_bytes();
    let export_hash = AssetHash::of_bytes(&export_bytes).as_str().to_owned();
    let dir = control_root().join("jobs").join(&job_id);
    std::fs::create_dir_all(&dir).expect("建任务目录");
    let export_path = dir.join("export.wav");
    std::fs::write(&export_path, &export_bytes).expect("写导出文件");
    let notes_after = json!([
        {"index": 0, "start_content_sec": 0.0, "end_content_sec": 0.5,
         "detected_midi": 69.4, "target_midi": 69.0, "offset_cents": 0.0, "corrected": true},
        {"index": 1, "start_content_sec": 0.5, "end_content_sec": 1.0,
         "detected_midi": 71.2, "target_midi": 71.0, "offset_cents": 0.0, "corrected": true}
    ]);
    write_receipt(
        &job_id,
        track_id,
        clip_id,
        source_asset_hash,
        &export_path.display().to_string(),
        export_bytes.len() as u64,
        auto_merge,
        false,
        notes_after,
    );
    (job_id, export_hash)
}

// ---------------------------------------------------------------------------
// ① 坏实参
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_rejects_bad_arguments() {
    let project = filled_project();
    let track = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, _) = import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let base = |overrides: Value| {
        let mut args = json!({
            "trackId": track,
            "clipId": clip_id,
            "keyRoot": 9,
            "scale": "major",
        });
        if let (Value::Object(base), Value::Object(extra)) = (&mut args, overrides) {
            for (key, value) in extra {
                base.insert(key, value);
            }
        }
        args
    };
    for (label, args) in [
        ("keyRoot 越界", base(json!({ "keyRoot": 12 }))),
        ("scale 非法", base(json!({ "scale": "lydian" }))),
        ("mode 非法", base(json!({ "mode": "warp" }))),
        ("mode 未开放", base(json!({ "mode": "regenerate" }))),
        ("trackId 非 ULID", base(json!({ "trackId": "not-a-ulid" }))),
    ] {
        let result = call_tool(&mut dispatcher, "yeban_correct_pitch", args);
        assert_eq!(result["status"], "error", "{label}: {result}");
        assert_eq!(
            result["error"]["code"], "INVALID_PARAMETER_RANGE",
            "{label}: {result}"
        );
    }
}

// ---------------------------------------------------------------------------
// ② dryRun 预览
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_dry_run_previews_without_side_effects() {
    let project = filled_project();
    let track = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, _) = import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let before = project_bytes(&dispatcher);
    // `dryRun` 由 dispatch 层拦截：响应是 dispatch 信封
    //（`dryRun` / `stateUnchanged` / `preview`），plan 的 data 展平在 `preview` 里。
    let preview = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch",
        json!({
            "trackId": track,
            "clipId": clip_id,
            "keyRoot": 9,
            "scale": "major",
            "dryRun": true,
        }),
    );
    assert_eq!(preview["status"], "success", "{preview}");
    assert_eq!(preview["data"]["dryRun"], true);
    assert_eq!(preview["data"]["stateUnchanged"], true);
    assert_eq!(preview["data"]["preview"]["wouldSubmit"], true);
    assert_eq!(preview["data"]["preview"]["keyRoot"], 9);
    assert_eq!(preview["data"]["preview"]["scale"], "major");
    assert_eq!(
        project_bytes(&dispatcher),
        before,
        "dryRun 预览必须一位都不改"
    );
}

// ---------------------------------------------------------------------------
// ③ 二进制未配置
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_without_binary_fails_loudly() {
    point_bin_at_missing_path();
    let project = filled_project();
    let track = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, _) = import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let result = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch",
        json!({
            "trackId": track,
            "clipId": clip_id,
            "keyRoot": 9,
            "scale": "major",
        }),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "IO_ERROR", "{result}");
    assert_eq!(
        result["error"]["data"]["reason"], "otcpNotConfigured",
        "{result}"
    );
}

// ---------------------------------------------------------------------------
// ④ 立体声源
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_refuses_stereo_source() {
    let project = filled_project();
    let track = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, _) = import_audio_clip(
        &mut dispatcher,
        &wav_s16_stereo(48_000, &[0, 1_000, -1_000, 0]),
    );
    let result = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch",
        json!({
            "trackId": track,
            "clipId": clip_id,
            "keyRoot": 9,
            "scale": "major",
        }),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(
        result["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{result}"
    );
    assert_eq!(
        result["error"]["data"]["reason"], "otcpStereoUnsupported",
        "{result}"
    );
    assert_eq!(result["error"]["data"]["channels"], 2);
}

// ---------------------------------------------------------------------------
// ⑤ 双重校正
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_refuses_double_correction() {
    let mut project = filled_project();
    let track_id = first_track_id(&project);
    let device = DeviceDefinition {
        id: EntityId::new(),
        name: "OpenTune".to_owned(),
        kind: DeviceKind::ExternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: 0,
        plugin: Some(PluginBinding {
            path: "OpenTune.vst3".to_owned(),
            class: None,
        }),
    };
    let device_id = device.id.to_canonical_string();
    project
        .tracks
        .get_mut(&track_id)
        .expect("首轨")
        .devices
        .push(device);
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, _) = import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let result = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id,
            "keyRoot": 9,
            "scale": "major",
        }),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "CONFLICT", "{result}");
    assert_eq!(
        result["error"]["data"]["reason"], "otcpDoubleCorrection",
        "{result}"
    );
    assert_eq!(result["error"]["data"]["device_id"], device_id);
}

// ---------------------------------------------------------------------------
// ⑥ 未知任务 / 无音符表
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_job_reports_unknown_job() {
    let project = filled_project();
    let mut dispatcher = dispatcher_with_project(project);
    let result = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_job",
        json!({ "job_id": "otcp-no-such-job-0001" }),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "ENTITY_NOT_FOUND", "{result}");
    assert_eq!(
        result["error"]["data"]["reason"], "otcpJobNotFound",
        "{result}"
    );
}

#[test]
fn correct_pitch_notes_reports_unavailable() {
    let project = filled_project();
    let mut dispatcher = dispatcher_with_project(project);
    let by_clip = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_notes",
        json!({
            "trackId": EntityId::new().to_canonical_string(),
            "clipId": EntityId::new().to_canonical_string(),
        }),
    );
    assert_eq!(by_clip["status"], "error", "{by_clip}");
    assert_eq!(by_clip["error"]["code"], "ENTITY_NOT_FOUND", "{by_clip}");
    assert_eq!(
        by_clip["error"]["data"]["reason"], "otcpNotesUnavailable",
        "{by_clip}"
    );
    let by_content = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_notes",
        json!({ "contentId": "no-such-content" }),
    );
    assert_eq!(by_content["status"], "error", "{by_content}");
    assert_eq!(
        by_content["error"]["data"]["reason"], "otcpNotesUnavailable",
        "{by_content}"
    );
    // 寻址非法：两者都给 / 都不给 ⇒ INVALID_PARAMETER_RANGE。
    let both = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_notes",
        json!({
            "trackId": EntityId::new().to_canonical_string(),
            "clipId": EntityId::new().to_canonical_string(),
            "contentId": "c:x",
        }),
    );
    assert_eq!(both["error"]["code"], "INVALID_PARAMETER_RANGE", "{both}");
}

// ---------------------------------------------------------------------------
// ⑦ 交付（默认只出提案）
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_job_delivers_a_succeeded_job_through_a_proposal() {
    let project = filled_project();
    let track_id = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, source_asset) =
        import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let (job_id, export_hash) = fabricate_succeeded_job(&track_id, &clip_id, &source_asset, false);
    let before = project_bytes(&dispatcher);
    let delivered = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_job",
        json!({ "job_id": job_id }),
    );
    assert_eq!(delivered["status"], "success", "{delivered}");
    let data = &delivered["data"];
    assert_eq!(data["status"], "succeeded");
    assert_eq!(data["asset"]["hash"], export_hash);
    assert_eq!(data["pendingMerge"], true);
    let proposal_id = data["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    assert_eq!(
        project_bytes(&dispatcher),
        before,
        "默认只出提案：工程一位不变"
    );
    // 收据已记交付；重复轮询不再重复建提案。
    let receipt_text = std::fs::read_to_string(
        control_root()
            .join("jobs")
            .join(&job_id)
            .join("receipt.json"),
    )
    .expect("读收据");
    let receipt: Value = serde_json::from_str(&receipt_text).expect("收据 JSON");
    assert_eq!(receipt["delivered_proposal_id"], proposal_id);
    let again = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_job",
        json!({ "job_id": job_id }),
    );
    assert_eq!(again["status"], "success", "{again}");
    assert_eq!(again["data"]["proposal"]["proposalId"], proposal_id);
    let listed = call_tool(&mut dispatcher, "yeban_list_proposals", json!({}));
    let proposals = listed["data"]["proposals"].as_array().expect("proposals");
    assert_eq!(proposals.len(), 1, "重复轮询不得重复建提案: {proposals:?}");
    assert_eq!(proposals[0]["proposalId"], proposal_id);
}

// ---------------------------------------------------------------------------
// ⑧ autoMerge 当场合并
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_job_auto_merge_applies_at_once() {
    let project = filled_project();
    let track_id = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, source_asset) =
        import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let (job_id, export_hash) = fabricate_succeeded_job(&track_id, &clip_id, &source_asset, true);
    let merged = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_job",
        json!({ "job_id": job_id }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    assert_eq!(merged["data"]["autoMerged"], true);
    assert_eq!(merged["data"]["pendingMerge"], false);
    // 片段资产已换新；旧资产仍在索引里（历史仍引用它）。
    let clip_entity = EntityId::from_str(&clip_id).expect("ULID");
    let clip = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .clip_pool
        .get(&clip_entity)
        .expect("片段")
        .clone();
    match &clip.content {
        ClipContent::Audio { asset, .. } => {
            assert_eq!(asset.as_str(), export_hash);
        }
        ClipContent::Midi { .. } => panic!("必须是音频片段"),
    }
    let old_hash = AssetHash::parse(source_asset).expect("旧哈希");
    assert!(
        dispatcher
            .domain()
            .active_project()
            .expect("活跃工程")
            .assets
            .contains_key(&old_hash),
        "旧资产索引条目必须保留"
    );
}

// ---------------------------------------------------------------------------
// ⑨ 音符表工具读收据
// ---------------------------------------------------------------------------

#[test]
fn correct_pitch_notes_reads_notes_from_a_receipt() {
    let project = filled_project();
    let track_id = first_track_id(&project).to_canonical_string();
    let mut dispatcher = dispatcher_with_project(project);
    let (clip_id, source_asset) =
        import_audio_clip(&mut dispatcher, &wav_s16(48_000, &[0, 1_000, -1_000, 0]));
    let (job_id, _) = fabricate_succeeded_job(&track_id, &clip_id, &source_asset, false);
    write_clip_index(&track_id, &clip_id, &job_id);
    let by_clip = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_notes",
        json!({ "trackId": track_id, "clipId": clip_id }),
    );
    assert_eq!(by_clip["status"], "success", "{by_clip}");
    assert_eq!(by_clip["data"]["totalCount"], 2);
    // f32 往返：69.4 存成 f32 是 69.4000015…，容差比较。
    let detected = by_clip["data"]["notes"][0]["detectedMidi"]
        .as_f64()
        .expect("detectedMidi");
    assert!((detected - 69.4).abs() < 1e-4, "{detected}");
    assert_eq!(by_clip["data"]["notes"][0]["corrected"], true);
    assert_eq!(by_clip["data"]["tuningHz"], 440.0);
    let by_content = call_tool(
        &mut dispatcher,
        "yeban_correct_pitch_notes",
        json!({ "contentId": format!("c:{job_id}") }),
    );
    assert_eq!(by_content["status"], "success", "{by_content}");
    assert_eq!(by_content["data"]["totalCount"], 2);
    assert_eq!(by_content["data"]["jobId"], job_id);
}

// ---------------------------------------------------------------------------
// ⑩ 19 个 reason → Fault 的映射全覆盖
// ---------------------------------------------------------------------------

#[test]
fn otcp_fault_mapping_covers_every_reason() {
    // （reason，期望码，驱动侧 data，期望载荷键集）
    let cases: &[(&'static str, ErrorCode, Value, &[&str])] = &[
        (
            "otcpRenderFailed",
            ErrorCode::RenderFailed,
            json!({"job_id": "j1", "block": {"startSample": 128000}, "note_gap": {"prev_note_end_sec": 2.84}, "self_test": {"unvoiced_gap": "fail"}}),
            &["reason", "job_id", "block", "note_gap", "self_test"],
        ),
        (
            "otcpModalBlocked",
            ErrorCode::Busy,
            json!({"job_id": "j1", "dialogs": ["renderFailed"]}),
            &["reason", "job_id", "dialogs"],
        ),
        (
            "otcpDialogCommits",
            ErrorCode::Conflict,
            json!({"job_id": "j1", "dialogs": ["save"]}),
            &["reason", "job_id", "dialogs"],
        ),
        (
            "otcpRevisionConflict",
            ErrorCode::Conflict,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
        (
            "otcpNotReady",
            ErrorCode::IoError,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
        (
            "otcpContentGone",
            ErrorCode::ClipNotFound,
            json!({"job_id": "j1", "content_id": "c:1"}),
            &["reason", "job_id", "content_id"],
        ),
        (
            "otcpMeasureFailed",
            ErrorCode::RenderFailed,
            json!({"median_cents": 80.0, "detection_rate": 0.5}),
            &["reason", "job_id", "measurement"],
        ),
        (
            "otcpNoEditor",
            ErrorCode::IoError,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
        (
            "otcpUnsupportedBuild",
            ErrorCode::IoError,
            json!({"job_id": "j1", "build_commit": "abc", "missing_capabilities": ["jobs.v1"]}),
            &["reason", "job_id", "build_commit", "missing_capabilities"],
        ),
        (
            "otcpDoubleCorrection",
            ErrorCode::Conflict,
            json!({"track_id": "t1", "device_id": "d1"}),
            &["reason", "track_id", "device_id"],
        ),
        (
            "otcpStereoUnsupported",
            ErrorCode::InvalidParameterRange,
            json!({"track_id": "t1", "clip_id": "c1", "channels": 2}),
            &["reason", "track_id", "clip_id", "channels"],
        ),
        (
            "otcpServerGone",
            ErrorCode::IoError,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
        (
            "otcpBusy",
            ErrorCode::Busy,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
        (
            "otcpJobNotFound",
            ErrorCode::EntityNotFound,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
        (
            "otcpNotesUnavailable",
            ErrorCode::EntityNotFound,
            json!({"job_id": Value::Null, "content_id": "c:1"}),
            &["reason", "job_id", "content_id"],
        ),
        (
            "otcpNotConfigured",
            ErrorCode::IoError,
            Value::Null,
            &["reason"],
        ),
        (
            "otcpSourceNotWav",
            ErrorCode::IoError,
            json!({"track_id": "t1", "clip_id": "c1", "detail": "bad riff"}),
            &["reason", "track_id", "clip_id", "detail"],
        ),
        (
            "otcpSourceLicenseMissing",
            ErrorCode::Conflict,
            json!({"track_id": "t1", "clip_id": "c1", "asset": "ab12"}),
            &["reason", "track_id", "clip_id", "asset"],
        ),
        (
            "otcpTimeGridUnsupported",
            ErrorCode::InvalidParameterRange,
            json!({"job_id": "j1"}),
            &["reason", "job_id"],
        ),
    ];
    assert_eq!(cases.len(), 19, "OTCP reason 必须是恰好 19 个");
    for (reason, code, data, keys) in cases {
        let error = OtcpError::with_data(reason, format!("测试 {reason}"), data.clone());
        let fault = fault_for(&error);
        assert_eq!(fault.domain_code(), Some(*code), "{reason}");
        let rendered = format!("{fault:?}");
        assert!(
            rendered.contains(reason),
            "Fault 里必须带 reason {reason}: {rendered}"
        );
        for key in *keys {
            assert!(
                rendered.contains(key),
                "Fault 载荷里必须有键 {key}（{reason}）: {rendered}"
            );
        }
    }
}
