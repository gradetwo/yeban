//! **T-FIX-561 判据**：所有 `tools/call` 失败都带可直接执行的恢复信封。
//!
//! 兼容形状（`code`/`message`）不变；`error.data` 补 reason、field/path、
//! received、expected/accepted、candidates、stateChanged、retryable、nextStep。
//! 每条判据都断言 `nextStep` 指向**真实存在的工具**且参数是对象；
//! 实体不存在时必须给有界候选。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::jsonrpc::ErrorObject;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::TOOLS;

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-recovery-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn project_path(&self) -> PathBuf {
        let project = yeban_model::samples::filled_project();
        let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱");
        let assets: std::collections::BTreeMap<_, _> =
            yeban_model::samples::filled_project_assets()
                .into_iter()
                .collect();
        let bytes = yeban_model::container::write_project_container(&project, &history, &assets)
            .expect("写真容器");
        let path = self.dir.join("demo.yeban");
        fs::write(&path, bytes).expect("写容器");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

fn call_raw(
    dispatcher: &mut Dispatcher,
    auth: &str,
    name: &str,
    arguments: Value,
) -> (u16, Result<Value, ErrorObject>) {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    match (response.result, response.error) {
        (Some(result), None) => (outcome.http_status, Ok(result)),
        (None, Some(error)) => (outcome.http_status, Err(error)),
        other => panic!("result 与 error 必须恰好一个: {other:?}"),
    }
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let (status, outcome) = call_raw(dispatcher, auth, name, arguments);
    assert_eq!(status, 200, "{name} 的领域失败必须走带内 ToolResponse");
    outcome.unwrap_or_else(|error| panic!("{name} 不该有 JSON-RPC 层错误: {error:?}"))
}

fn open(scratch: &Scratch, dispatcher: &mut Dispatcher, auth: &str) -> Value {
    let path = scratch.project_path();
    let opened = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    opened
}

fn domain_data(value: &Value) -> &Value {
    assert_eq!(value["status"], "error", "必须是领域失败: {value}");
    &value["error"]["data"]
}

fn assert_envelope(data: &Value, expected_reason: &str) {
    assert_eq!(data["reason"], expected_reason, "{data}");
    assert!(
        data.get("field").and_then(Value::as_str).is_some()
            || data.get("path").and_then(Value::as_str).is_some(),
        "必须能定位字段: {data}"
    );
    assert_eq!(
        data["stateChanged"], false,
        "失败前不得声称改了工程: {data}"
    );
    let next = data
        .get("nextStep")
        .unwrap_or_else(|| panic!("缺 nextStep: {data}"));
    let tool = next["tool"]
        .as_str()
        .unwrap_or_else(|| panic!("nextStep 缺 tool: {next}"));
    assert!(
        TOOLS.iter().any(|spec| spec.name == tool),
        "nextStep.tool 必须是真实工具: {tool}"
    );
    assert!(
        next["arguments"].is_object(),
        "nextStep.arguments 必须是可执行参数模板: {next}"
    );
}

fn assert_candidates_include(data: &Value, id: &str) {
    let candidates = data["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("缺 candidates: {data}"));
    assert!(!candidates.is_empty(), "候选不得为空: {data}");
    assert!(
        candidates.iter().any(|candidate| candidate["id"] == id),
        "候选必须包含真实 id {id}: {data}"
    );
}

/// 真的按 `nextStep` 再调一次，证明它不是摆设。
fn assert_next_step_runs(dispatcher: &mut Dispatcher, auth: &str, data: &Value) {
    let next = &data["nextStep"];
    let tool = next["tool"].as_str().expect("nextStep.tool");
    let arguments = next["arguments"].clone();
    let follow = call(dispatcher, auth, tool, arguments);
    assert_eq!(
        follow["status"], "success",
        "nextStep 必须可直接执行: {tool} {next} => {follow}"
    );
}

fn ids(dispatcher: &Dispatcher) -> (String, String) {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let track = project.tracks.values().next().expect("至少一条音轨");
    let track_id = track.id.to_canonical_string();
    let clip_id = project
        .tracks
        .values()
        .find_map(|track| track.clips.values().next())
        .expect("至少一处摆放")
        .clip_id
        .to_canonical_string();
    (track_id, clip_id)
}

fn macro_track_id(dispatcher: &Dispatcher) -> String {
    dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .tracks
        .values()
        .find(|track| !track.macros.is_empty())
        .expect("带宏的音轨")
        .id
        .to_canonical_string()
}

fn bogus_id() -> String {
    // 合法 ULID 形状，但 fixture 里不存在。
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned()
}

#[test]
fn unknown_tool_gets_nearest_tool_and_tool_next_step() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let (status, outcome) = call_raw(&mut dispatcher, &auth, "yeban_query_projet", json!({}));
    assert_eq!(status, 400, "未知工具仍是 JSON-RPC 请求层失败");
    let error = outcome.expect_err("未知工具必须报 JSON-RPC 错误");
    assert_eq!(error.code, -32004, "{error:?}");
    let data = error.data.as_ref().expect("必须带 data");
    assert_eq!(data["reason"], "unknownTool", "{data}");
    assert_eq!(data["stateChanged"], false);
    let candidates = data["candidates"].as_array().expect("候选");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate == "yeban_query_project"),
        "最近邻必须包含 yeban_query_project: {data}"
    );
    assert_eq!(data["nextStep"]["tool"], "yeban_query_project");
    assert!(data["nextStep"]["arguments"].is_object());
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn unknown_parameter_gets_nearest_parameter_and_clean_template() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let track_id = macro_track_id(&dispatcher);
    let (status, outcome) = call_raw(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({"trackId": track_id, "macroIndex": 0, "macroIndx": 0, "value": 0.5}),
    );
    assert_eq!(status, 400);
    let error = outcome.expect_err("未知参数必须拒绝");
    let data = error.data.as_ref().expect("必须带 data");
    assert_envelope(data, "unknownParam");
    assert!(
        data["candidates"]
            .as_array()
            .expect("候选")
            .iter()
            .any(|candidate| candidate == "macroIndex"),
        "最近邻必须包含 macroIndex: {data}"
    );
    assert_eq!(data["nextStep"]["tool"], "yeban_set_macro");
    assert!(
        data["nextStep"]["arguments"].get("macroIndx").is_none(),
        "修正模板必须去掉未知键: {data}"
    );
    assert!(data["nextStep"]["arguments"].get("macroIndex").is_some());
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn missing_parameter_gets_minimal_legal_template() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let track_id = macro_track_id(&dispatcher);
    let (status, outcome) = call_raw(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({"trackId": track_id, "value": 0.5}),
    );
    assert_eq!(status, 400);
    let error = outcome.expect_err("缺必填必须拒绝");
    let data = error.data.as_ref().expect("必须带 data");
    assert_envelope(data, "missingParam");
    let template = &data["nextStep"]["arguments"];
    assert!(
        template.get("macroIndex").is_some(),
        "必须补 macroIndex: {data}"
    );
    assert!(template.get("trackId").is_some());
    assert!(template.get("value").is_some());
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn wrong_type_reports_received_and_expected() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let track_id = macro_track_id(&dispatcher);
    let (status, outcome) = call_raw(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({"trackId": track_id, "macroIndex": "zero", "value": 0.5}),
    );
    assert_eq!(status, 400);
    let error = outcome.expect_err("类型错误必须拒绝");
    let data = error.data.as_ref().expect("必须带 data");
    assert_envelope(data, "invalidParamType");
    assert_eq!(data["received"], "string", "{data}");
    assert_eq!(data["expected"], "integer", "{data}");
    assert!(data["nextStep"]["arguments"].get("macroIndex").is_some());
    assert_eq!(data["nextStep"]["tool"], "yeban_set_macro");
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn enum_error_lists_accepted_values_and_corrected_kind() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let value = call(
        &mut dispatcher,
        &auth,
        "yeban_list_proposals",
        json!({"kind": "song"}),
    );
    let data = domain_data(&value);
    assert_envelope(data, "unknownProposalKind");
    let accepted = data["accepted"].as_array().expect("accepted");
    for kind in ["section", "notes", "macro", "plugin"] {
        assert!(
            accepted.iter().any(|candidate| candidate == kind),
            "accepted 必须含 {kind}: {data}"
        );
    }
    assert_eq!(data["received"], "song");
    assert_eq!(data["nextStep"]["tool"], "yeban_list_proposals");
    assert_eq!(data["nextStep"]["arguments"]["kind"], "section");
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn missing_track_gets_real_track_candidates_and_query_next_step() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let (real_track, _) = ids(&dispatcher);
    let value = call(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({"trackId": bogus_id(), "macroIndex": 0, "value": 0.5}),
    );
    let data = domain_data(&value);
    assert_envelope(data, "trackNotFound");
    assert_candidates_include(data, &real_track);
    assert_eq!(data["nextStep"]["tool"], "yeban_query_project");
    assert_eq!(data["received"], bogus_id());
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn missing_clip_gets_real_clip_candidates_and_query_next_step() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let (track_id, real_clip) = ids(&dispatcher);
    let value = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id,
            "clipId": bogus_id(),
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}],
        }),
    );
    let data = domain_data(&value);
    assert_envelope(data, "clipNotFound");
    assert_candidates_include(data, &real_clip);
    assert_eq!(data["nextStep"]["tool"], "yeban_query_project");
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn missing_proposal_gets_open_proposal_candidate_and_list_next_step() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let (track_id, clip_id) = ids(&dispatcher);
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id,
            "clipId": clip_id,
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 64, "durationTicks": 240}}],
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let value = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": bogus_id()}),
    );
    let data = domain_data(&value);
    assert_envelope(data, "proposalNotFound");
    assert_candidates_include(data, &proposal_id);
    assert_eq!(data["nextStep"]["tool"], "yeban_list_proposals");
    assert_next_step_runs(&mut dispatcher, &auth, data);
}

#[test]
fn ops_second_and_third_errors_are_reported_together() {
    let scratch = Scratch::new();
    let (mut dispatcher, auth) = dispatcher();
    let _ = open(&scratch, &mut dispatcher, &auth);
    let (track_id, clip_id) = ids(&dispatcher);
    let before = dispatcher.domain().open_proposals().len();
    let value = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id,
            "clipId": clip_id,
            "ops": [
                {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}},
                {"kind": "noSuchKind"},
                {"kind": "add"},
            ],
        }),
    );
    let data = domain_data(&value);
    assert_envelope(data, "opsValidationFailed");
    let errors = data["errors"].as_array().expect("errors[]");
    assert_eq!(errors.len(), 2, "必须一次报全第 2、3 条: {data}");
    let indices: Vec<u64> = errors
        .iter()
        .map(|entry| entry["opIndex"].as_u64().expect("opIndex"))
        .collect();
    assert_eq!(indices, vec![1, 2], "{data}");
    for entry in errors {
        assert!(
            entry.get("field").and_then(Value::as_str).is_some()
                || entry.get("path").and_then(Value::as_str).is_some(),
            "逐条错误必须可定位: {entry}"
        );
    }
    assert_eq!(data["nextStep"]["tool"], "yeban_edit_notes");
    assert!(data["nextStep"]["arguments"]["ops"].is_array());
    assert_eq!(
        dispatcher.domain().open_proposals().len(),
        before,
        "校验失败不得建提案"
    );
    assert_next_step_runs(&mut dispatcher, &auth, data);
}
