//! 伶人 SVS 工具（`synthesize_singing` / `svs_job`）的端到端判据。
//!
//! 全部网络交互走本文件内的桩服务（`127.0.0.1` 临时端口，`std::net`）：
//! 提交、状态、回执、音频、失败路径（未知任务、任务失败、未完成、门耗尽）
//! 各一例；SoulX 与 VocalRender 两种放置语义各一例；`dryRun` 不提交、
//! 不取 wav、不写暂存各一例。⛔ 不依赖任何外部服务。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, TrackKind, YebanProjectV1};

// ---------------------------------------------------------------------------
// 脚手架（与 `tests/extension_tools.rs` 同款，不跨测试文件引用）
// ---------------------------------------------------------------------------

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
            "yeban-mcp-svs-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的 `ToolResponse`（`result` 里的 JSON 对象；领域失败走带内错误）。
fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
    let line = json!({
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
    outcome.response.expect("响应").result.expect("result")
}

/// 一次 `tools/call` 的原始读数（参数层失败用它：走 JSON-RPC `-32602`）。
fn call_raw(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> (u16, Option<i64>) {
    let auth = bearer(dispatcher);
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let status = outcome.http_status;
    let code = outcome.error_code();
    (status, code)
}

fn assert_in_band_error(result: &Value, code: &str, reason: &str) -> Value {
    assert_eq!(result["status"], "error", "必须是带内错误: {result}");
    assert_eq!(result["error"]["code"], code, "错误码: {result}");
    assert_eq!(
        result["error"]["data"]["reason"], reason,
        "reason: {result}"
    );
    result.clone()
}

/// 工程的规范化字节（`undo_session` 的同一口径；状态不变判据用它）。
fn project_bytes(dispatcher: &Dispatcher) -> Vec<u8> {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    yeban_mcp::undo_session::canonical_project_bytes(project).expect("规范化字节")
}

/// 摆放目标轨：优先 Audio 轨（样本里没有则退到第一条轨；落点只要求轨存在）。
fn placement_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .values()
        .find(|track| matches!(track.kind, TrackKind::Audio))
        .or_else(|| project.tracks.values().next())
        .expect("样本里必须有音轨")
        .id
}

// ---------------------------------------------------------------------------
// 桩服务
// ---------------------------------------------------------------------------

const JOB_OK: &str = "abcdef0123456789";
const JOB_VR: &str = "vrjob000000000001";
/// 干运行测试专用的 VR 任务（暂存路径与落轨测试不共享，免得并行测试互相写同一文件）。
const JOB_VR_DRY: &str = "vrjob000000000002";
const JOB_FAILED: &str = "failedjob00000001";
const JOB_RUNNING: &str = "runningjob000001";
const JOB_UNKNOWN: &str = "unknownjob0000001";
const GATE_ERROR: &str =
    "part lead take 1: takes gate exhausted after 3 attempts; last: envelope correlation 0.995";

/// 一份最小的 16-bit 单声道 WAV（24 kHz，1 秒 ⇒ 24000 帧）。
fn wav_fixture() -> Vec<u8> {
    let samples = vec![0i16; 24_000];
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24_000u32.to_le_bytes());
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn soulx_receipt() -> Value {
    json!({
        "job_id": JOB_OK,
        "status": "completed",
        "parts": {
            "lead": {
                "model": "soulx",
                "takes": [{
                    "take": 0,
                    "seed": 0,
                    "attempts": 1,
                    "audio_file": "lead_take0.wav",
                    "duration_s": 1.0,
                    "sample_rate": 24000,
                    "alignment": [
                        {"score_onset_s": 0.0, "measured_onset_s": 0.01, "deviation_ms": 10.0, "lyric": "la"},
                        {"score_onset_s": 0.5, "measured_onset_s": 0.85, "deviation_ms": 350.0, "lyric": "ti"},
                    ],
                }],
                "gate_pairwise": [],
            },
        },
    })
}

fn vr_receipt_for(job_id: &str) -> Value {
    json!({
        "job_id": job_id,
        "status": "completed",
        "parts": {
            "lead": {
                "model": "vocalrender",
                "takes": [{
                    "take": 0,
                    "seed": 0,
                    "attempts": 1,
                    "audio_file": "lead_take0.wav",
                    "duration_s": 1.0,
                    "sample_rate": 24000,
                    "alignment": [
                        {"score_onset_s": 0.0, "measured_onset_s": 0.0, "deviation_ms": 0.0, "lyric": "<SP>"},
                        {"score_onset_s": 0.5, "measured_onset_s": 0.5, "deviation_ms": 0.0, "lyric": "la"},
                        {"score_onset_s": 1.5, "measured_onset_s": 1.6, "deviation_ms": 100.0, "lyric": "ti"},
                    ],
                }],
                "gate_pairwise": [],
            },
        },
    })
}

fn status_json(job_id: &str, state: &str, error: Value) -> Value {
    json!({
        "job_id": job_id,
        "status": state,
        "parts_total": 1,
        "takes_total": 1,
        "parts_completed": if state == "completed" { 1 } else { 0 },
        "takes_completed": if state == "completed" { 1 } else { 0 },
        "error": error,
    })
}

fn http_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// 按请求行路由的桩服务（`Connection: close`，一次请求一个连接）。
fn route(method: &str, path: &str, wav: &[u8]) -> Vec<u8> {
    let (path, _query) = match path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path, None),
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match (method, segments.as_slice()) {
        ("POST", ["v1", "synthesize"]) => http_response(
            200,
            "application/json",
            serde_json::to_string(&json!({
                "job_id": JOB_OK,
                "status": "queued",
                "parts_total": 1,
                "takes_total": 1,
            }))
            .expect("json")
            .as_bytes(),
        ),
        ("GET", ["v1", "jobs", job_id]) => {
            let body = match *job_id {
                JOB_OK | JOB_VR | JOB_VR_DRY => status_json(job_id, "completed", Value::Null),
                JOB_FAILED => status_json(job_id, "failed", Value::from(GATE_ERROR)),
                JOB_RUNNING => status_json(job_id, "running", Value::Null),
                _ => {
                    return http_response(404, "application/json", b"{\"detail\":\"unknown job\"}");
                }
            };
            http_response(
                200,
                "application/json",
                serde_json::to_string(&body).expect("json").as_bytes(),
            )
        }
        ("GET", ["v1", "jobs", job_id, "receipt"]) => match *job_id {
            JOB_OK => http_response(
                200,
                "application/json",
                serde_json::to_string(&soulx_receipt())
                    .expect("json")
                    .as_bytes(),
            ),
            JOB_VR | JOB_VR_DRY => http_response(
                200,
                "application/json",
                serde_json::to_string(&vr_receipt_for(job_id))
                    .expect("json")
                    .as_bytes(),
            ),
            JOB_FAILED => http_response(
                200,
                "application/json",
                serde_json::to_string(&json!({
                    "job_id": JOB_FAILED,
                    "status": "failed",
                    "error": GATE_ERROR,
                    "parts": {},
                }))
                .expect("json")
                .as_bytes(),
            ),
            JOB_RUNNING => http_response(
                409,
                "application/json",
                b"{\"detail\":\"job still running\"}",
            ),
            _ => http_response(404, "application/json", b"{\"detail\":\"unknown job\"}"),
        },
        ("GET", ["v1", "jobs", job_id, "audio"]) => match *job_id {
            JOB_OK | JOB_VR | JOB_VR_DRY => http_response(200, "audio/wav", wav),
            _ => http_response(404, "application/json", b"{\"detail\":\"audio not found\"}"),
        },
        _ => http_response(404, "application/json", b"{\"detail\":\"not found\"}"),
    }
}

struct Stub {
    port: u16,
    requests: Arc<AtomicUsize>,
}

fn start_stub() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 桩服务");
    let port = listener.local_addr().expect("读端口").port();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let wav = wav_fixture();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            counter.fetch_add(1, Ordering::SeqCst);
            if handle_connection(&mut stream, &wav).is_err() {
                continue;
            }
        }
    });
    Stub { port, requests }
}

fn handle_connection(stream: &mut TcpStream, wav: &[u8]) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
        if head.len() > 8192 {
            return Ok(());
        }
    }
    let head_text = String::from_utf8_lossy(&head);
    let mut lines = head_text.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("").to_owned();
    let content_length: usize = lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body)?;
    }
    let response = route(method, &path, wav);
    stream.write_all(&response)?;
    stream.flush()?;
    Ok(())
}

fn service_url(stub: &Stub) -> String {
    format!("http://127.0.0.1:{}", stub.port)
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 环回端口");
    listener.local_addr().expect("读端口").port()
}

fn valid_parts() -> Value {
    json!([{
        "label": "lead",
        "score": {
            "bpm": 120.0,
            "notes": [
                {"lyric": "la", "midi": 69, "start_s": 0.0, "dur_s": 1.0},
            ],
        },
        "voice_id": "v1",
    }])
}

// ---------------------------------------------------------------------------
// 提交
// ---------------------------------------------------------------------------

#[test]
fn submit_returns_the_server_receipt_stub() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({"parts": valid_parts(), "service_url": service_url(&stub)}),
    );
    assert_eq!(result["status"], "success");
    let data = &result["data"];
    assert_eq!(data["jobId"], JOB_OK);
    assert_eq!(data["status"], "queued");
    assert_eq!(data["partsTotal"], 1);
    assert_eq!(data["takesTotal"], 1);
    assert_eq!(data["serviceUrl"], service_url(&stub));
    // API key 绝不出现在响应里。
    assert!(!result.to_string().contains("YEBAN_SVS_API_KEY"));
}

#[test]
fn submit_to_an_unreachable_service_fails_loudly_in_band() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({
            "parts": valid_parts(),
            "service_url": format!("http://127.0.0.1:{}", closed_port()),
        }),
    );
    assert_in_band_error(&result, "IO_ERROR", "serviceUnreachable");
}

#[test]
fn submit_dry_run_validates_but_never_submits() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let before = project_bytes(&dispatcher);
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({
            "parts": valid_parts(),
            "service_url": service_url(&stub),
            "dryRun": true,
        }),
    );
    assert_eq!(result["status"], "success");
    let data = &result["data"];
    assert_eq!(data["sideEffect"], "external");
    assert_eq!(data["preview"]["wouldSubmit"], true);
    assert_eq!(data["preview"]["partsTotal"], 1);
    assert_eq!(stub.requests.load(Ordering::SeqCst), 0, "dryRun 不得发请求");
    assert_eq!(project_bytes(&dispatcher), before, "dryRun 不得改工程");
}

#[test]
fn submit_rejects_bad_parts_shapes_in_band() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let url = service_url(&stub);
    // 空 parts。
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({"parts": [], "service_url": url}),
    );
    assert_in_band_error(&result, "INVALID_PARAMETER_RANGE", "partsEmpty");
    // takes 越界。
    let mut parts = valid_parts();
    parts[0]["takes"] = json!(9);
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({"parts": parts, "service_url": url}),
    );
    assert_in_band_error(&result, "INVALID_PARAMETER_RANGE", "partTakesInvalid");
    // 未知 part 键。
    let mut parts = valid_parts();
    parts[0]["surprise"] = json!(1);
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({"parts": parts, "service_url": url}),
    );
    assert_in_band_error(&result, "INVALID_PARAMETER_RANGE", "unknownPartField");
    // https 响亮拒绝（配置解析时，不发请求）。
    let before = stub.requests.load(Ordering::SeqCst);
    let result = call_tool(
        &mut dispatcher,
        "synthesize_singing",
        json!({"parts": valid_parts(), "service_url": "https://example.com"}),
    );
    assert_in_band_error(&result, "IO_ERROR", "httpsNotSupported");
    assert_eq!(stub.requests.load(Ordering::SeqCst), before);
}

#[test]
fn missing_required_params_fail_at_the_tool_call_layer() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let (status, code) = call_raw(
        &mut dispatcher,
        "synthesize_singing",
        json!({"service_url": service_url(&stub)}),
    );
    assert_eq!(status, 400);
    assert_eq!(code, Some(-32602));
    let (status, code) = call_raw(
        &mut dispatcher,
        "svs_job",
        json!({"service_url": service_url(&stub)}),
    );
    assert_eq!(status, 400);
    assert_eq!(code, Some(-32602));
}

// ---------------------------------------------------------------------------
// 状态与回执
// ---------------------------------------------------------------------------

#[test]
fn status_reports_progress_and_carries_a_failed_error_verbatim() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let url = service_url(&stub);
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({"job_id": JOB_OK, "service_url": url}),
    );
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"]["status"], "completed");
    assert_eq!(result["data"]["partsCompleted"], 1);
    assert_eq!(result["data"]["takesCompleted"], 1);
    // 失败任务的状态查询本身是成功响应，error 原文照带（⛔ 不是带内错误）。
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({"job_id": JOB_FAILED, "service_url": url}),
    );
    assert_eq!(result["status"], "success");
    assert_eq!(result["data"]["status"], "failed");
    assert_eq!(result["data"]["error"], GATE_ERROR);
}

#[test]
fn status_of_an_unknown_job_is_entity_not_found() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({"job_id": JOB_UNKNOWN, "service_url": service_url(&stub)}),
    );
    assert_in_band_error(&result, "ENTITY_NOT_FOUND", "svsJobNotFound");
}

#[test]
fn receipt_returns_the_server_receipt_verbatim() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_OK,
            "want": "receipt",
            "service_url": service_url(&stub),
        }),
    );
    assert_eq!(result["status"], "success");
    let receipt = &result["data"]["receipt"];
    assert_eq!(receipt["parts"]["lead"]["takes"][0]["seed"], 0);
    assert_eq!(receipt["parts"]["lead"]["model"], "soulx");
}

// ---------------------------------------------------------------------------
// 取音频：不落轨 / SoulX 落轨 / VR 落轨 / 失败路径
// ---------------------------------------------------------------------------

#[test]
fn audio_without_a_track_returns_a_stable_cache_path_and_changes_nothing() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let before = project_bytes(&dispatcher);
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_OK,
            "want": "audio",
            "part": "lead",
            "take": 0,
            "service_url": service_url(&stub),
        }),
    );
    assert_eq!(result["status"], "success");
    let data = &result["data"];
    assert_eq!(data["placed"], false);
    assert_eq!(data["wavBytes"], wav_fixture().len());
    let path = data["wavPath"].as_str().expect("wavPath");
    assert!(path.contains("yeban-svs"), "暂存路径: {path}");
    assert!(
        path.ends_with("abcdef0123456789_lead_take0.wav"),
        "暂存路径: {path}"
    );
    assert_eq!(
        std::fs::read(path).expect("暂存 wav 必须存在"),
        wav_fixture(),
        "暂存文件必须是服务返回的原字节"
    );
    assert_eq!(project_bytes(&dispatcher), before, "不落轨不得改工程");
}

#[test]
fn audio_soulx_placement_uses_the_score_clock_and_is_idempotent() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let track = placement_track(dispatcher.domain().active_project().expect("活跃工程"));
    let track_id = track.to_canonical_string();
    let url = service_url(&stub);
    let args = json!({
        "job_id": JOB_OK,
        "want": "audio",
        "part": "lead",
        "take": 0,
        "trackId": track_id,
        "startTick": 960,
        "license": "CC0-1.0",
        "service_url": url,
    });
    let result = call_tool(&mut dispatcher, "svs_job", args.clone());
    assert_eq!(result["status"], "success", "{result}");
    let data = &result["data"];
    assert_eq!(data["placementModel"], "score");
    assert_eq!(data["placementStartTick"], 960);
    assert_eq!(data["clip"]["name"], "svs lead take 0");
    assert_eq!(data["placement"]["startTick"], 960);
    // 1 秒 @ 24 kHz、工程 bpm 128 ⇒ 全长 2048 tick。
    assert_eq!(data["placement"]["durationTicks"], 2048);
    // 超阈值偏差（ti 的 350 ms）进警告；SoulX 不移位。
    assert_eq!(data["deviationWarnings"].as_array().expect("数组").len(), 1);
    assert_eq!(data["deviationWarnings"][0]["lyric"], "ti");
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let placement = project
        .tracks
        .get(&track)
        .expect("轨")
        .clips
        .values()
        .find(|placement| placement.start_tick == 960)
        .expect("摆放必须落在轨上");
    assert_eq!(placement.duration_ticks, 2048);
    // 重复取同一遍 ⇒ 幂等命中（一位都不改）。
    let again = call_tool(&mut dispatcher, "svs_job", args);
    assert_eq!(again["status"], "success");
    assert_eq!(again["data"]["unchanged"], true);
    assert_eq!(again["data"]["applied"], false);
}

#[test]
fn audio_vocalrender_placement_anchors_the_first_sung_syllable() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let track = placement_track(dispatcher.domain().active_project().expect("活跃工程"));
    let url = service_url(&stub);
    // 工程 bpm 128、PPQ 960 ⇒ 1 秒 = 2048 tick；首音节实测 0.5 秒 ⇒ 移 1024 tick。
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_VR,
            "want": "audio",
            "part": "lead",
            "take": 0,
            "trackId": track.to_canonical_string(),
            "startTick": 2048,
            "license": "CC0-1.0",
            "service_url": url,
        }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let data = &result["data"];
    assert_eq!(data["placementModel"], "measured");
    assert_eq!(data["placementStartTick"], 1024);
    assert_eq!(data["firstSyllableLandsAtTick"], 2048);
    // SP 条目已忽略（说明里有点名）；100 ms 恰好不超阈值 ⇒ 无警告。
    assert!(data["placementNotes"].to_string().contains("SP"));
    assert_eq!(data["deviationWarnings"].as_array().expect("数组").len(), 0);
}

#[test]
fn audio_dry_run_with_a_track_previews_placement_without_fetching() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let track = placement_track(dispatcher.domain().active_project().expect("活跃工程"));
    let before = project_bytes(&dispatcher);
    let cache_path = std::env::temp_dir()
        .join("yeban-svs")
        .join(format!("{JOB_VR_DRY}_lead_take0.wav"));
    let _ = std::fs::remove_file(&cache_path);
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_VR_DRY,
            "want": "audio",
            "part": "lead",
            "take": 0,
            "trackId": track.to_canonical_string(),
            "startTick": 2048,
            "license": "CC0-1.0",
            "service_url": service_url(&stub),
            "dryRun": true,
        }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let preview = &result["data"]["preview"];
    assert_eq!(preview["wouldPlace"], true);
    assert_eq!(preview["placementStartTick"], 1024);
    assert!(!cache_path.exists(), "dryRun 不得写暂存文件");
    assert_eq!(project_bytes(&dispatcher), before, "dryRun 不得改工程");
}

#[test]
fn audio_of_a_failed_job_fails_loudly_with_the_gate_text_verbatim() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_FAILED,
            "want": "audio",
            "part": "lead",
            "take": 1,
            "service_url": service_url(&stub),
        }),
    );
    let error = assert_in_band_error(&result, "IO_ERROR", "svsJobFailed");
    assert_eq!(error["error"]["data"]["error"], GATE_ERROR);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("takes gate exhausted"),
        "失败信息必须带服务端原文: {error}"
    );
}

#[test]
fn audio_of_a_running_job_is_not_ready_and_receipt_conflicts() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let url = service_url(&stub);
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_RUNNING,
            "want": "audio",
            "part": "lead",
            "take": 0,
            "service_url": url,
        }),
    );
    assert_in_band_error(&result, "IO_ERROR", "jobNotCompleted");
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({"job_id": JOB_RUNNING, "want": "receipt", "service_url": url}),
    );
    assert_in_band_error(&result, "IO_ERROR", "receiptNotReady");
}

#[test]
fn audio_argument_mismatches_are_rejected_before_any_network() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let url = service_url(&stub);
    let before = stub.requests.load(Ordering::SeqCst);
    // 缺 part。
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({"job_id": JOB_OK, "want": "audio", "take": 0, "service_url": url}),
    );
    assert_in_band_error(&result, "INVALID_PARAMETER_RANGE", "missingField");
    // 落轨缺 license。
    let track = placement_track(dispatcher.domain().active_project().expect("活跃工程"));
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_OK,
            "want": "audio",
            "part": "lead",
            "take": 0,
            "trackId": track.to_canonical_string(),
            "service_url": url,
        }),
    );
    assert_in_band_error(&result, "INVALID_PARAMETER_RANGE", "licenseRequired");
    // 没给 trackId 却给了 startTick。
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_OK,
            "want": "audio",
            "part": "lead",
            "take": 0,
            "startTick": 960,
            "service_url": url,
        }),
    );
    assert_in_band_error(&result, "INVALID_PARAMETER_RANGE", "placementWithoutTrack");
    assert_eq!(
        stub.requests.load(Ordering::SeqCst),
        before,
        "参数错误不得发请求"
    );
}

#[test]
fn audio_with_an_unknown_take_is_entity_not_found() {
    let stub = start_stub();
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "svs_job",
        json!({
            "job_id": JOB_OK,
            "want": "audio",
            "part": "lead",
            "take": 5,
            "service_url": service_url(&stub),
        }),
    );
    assert_in_band_error(&result, "ENTITY_NOT_FOUND", "svsTakeNotInReceipt");
}

#[test]
fn both_new_tools_are_discoverable_in_tools_list() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let auth = bearer(&dispatcher);
    let line = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#;
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), line);
    let listed = outcome
        .response
        .expect("响应")
        .result_value()
        .expect("result")["tools"]
        .as_array()
        .expect("tools 数组")
        .clone();
    assert_eq!(listed.len(), yeban_mcp::tools::TOOL_COUNT);
    for name in ["synthesize_singing", "svs_job"] {
        let descriptor = listed
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("tools/list 里找不到 `{name}`"));
        let spec = yeban_mcp::tools::tool(name).expect("注册表");
        assert_eq!(descriptor["annotations"]["specId"], spec.spec_id);
        assert_eq!(descriptor["annotations"]["dryRunSupported"], true);
        assert_eq!(descriptor["annotations"]["idempotent"], true);
    }
    let submit = listed
        .iter()
        .find(|entry| entry["name"] == "synthesize_singing")
        .expect("提交工具");
    assert_eq!(submit["annotations"]["sideEffect"], "external");
}
