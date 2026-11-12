//! `yeban_auto_align_tracks` 的承重判据（自动相位对齐，手动层第二层）。
//!
//! ## 覆盖
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | lag > 0 ⇒ 延基准轨（先到者），走提案、工程一位不变 | `positive_lag_delays_reference_through_proposal` |
//! | ② | `autoMerge` ⇒ 当场合并，轨延时真正落盘 | `auto_merge_applies_delay_at_once` |
//! | ③ | lag < 0 ⇒ 延目标轨 |lag| | `negative_lag_delays_target` |
//! | ④ | 极性反转检出 ⇒ 翻转目标轨 `invert_polarity`（lag 0 也建提案） | `polarity_inversion_flips_target` |
//! | ⑤ | 低相关 + `apply` ⇒ `CONFLICT` + `lowCorrelation` 响亮拒绝 | `low_correlation_refuses_to_apply` |
//! | ⑥ | 低相关 + `apply: false` ⇒ 只测量、如实返回、不建提案 | `measure_only_returns_low_correlation` |
//! | ⑦ | 同轨 ⇒ `INVALID_PARAMETER_RANGE` + `sameReferenceAndTarget` | `same_track_is_rejected` |
//! | ⑧ | 补偿后超 ±4800 ⇒ `CONFLICT` + `compensationOutOfRange`，不部分施加 | `compensation_out_of_range_is_rejected` |
//! | ⑨ | `dryRun` ⇒ 只预览，不调 provider 语义下的提案、工程不变 | `dry_run_previews_without_proposal` |
//! | ⑩ | 无 provider ⇒ `CONFLICT` + `providerUnavailable`，不返回假测量 | `missing_provider_fails_loudly` |
//!
//! 测量本身（互相关精度）由 `yeban-dsp` 的 `phase_align_auto` 单元测试与
//! `yeban-render` 的两轨渲染测试覆盖；这里只测 MCP 适配层：补偿方向、
//! 提案纪律、拒绝语义。provider 用定值 stub（与生产 trait 同一个）。

use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::phase_align_auto::{
    MeasurementConfidence, PhaseDelayMeasurement, StubPhaseAlignAutoProvider,
};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};

/// `filled_project` 的 lead 轨（`fixture_id(2)`）。
const LEAD: &str = "01J8ZQ00000000000000000002";
/// `filled_project` 的 bass 轨（`fixture_id(3)`）。
const BASS: &str = "01J8ZQ00000000000000000003";

// ---------------------------------------------------------------------------
// 夹具
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
            "yeban-mcp-phase-align-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

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

/// 定值测量（stub provider 的返回值）。
fn measurement(lag: i32, correlation: f32, polarity_inverted: bool) -> PhaseDelayMeasurement {
    PhaseDelayMeasurement {
        lag_samples: f64::from(lag),
        lag_samples_rounded: lag,
        correlation,
        polarity_inverted,
        confidence: MeasurementConfidence::from_correlation(correlation),
        window_start_frame: 0,
        window_frames: 48_000,
        sample_rate_hz: 48_000,
    }
}

fn dispatcher_with_stub(m: PhaseDelayMeasurement) -> Dispatcher {
    let mut dispatcher = dispatcher_with_project(filled_project());
    dispatcher
        .domain_mut()
        .set_phase_align_auto_provider(Arc::new(StubPhaseAlignAutoProvider::new(m)));
    dispatcher
}

fn align_args(extra: Value) -> Value {
    let mut args = json!({
        "referenceTrackId": LEAD,
        "targetTrackId": BASS,
    });
    args.as_object_mut()
        .expect("对象")
        .extend(extra.as_object().expect("对象").clone());
    args
}

fn track_delay(dispatcher: &Dispatcher, track: &str) -> Option<i32> {
    let id = EntityId::from_str(track).expect("ULID");
    dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .tracks
        .get(&id)
        .expect("轨")
        .delay_samples
}

fn track_invert(dispatcher: &Dispatcher, track: &str) -> Option<bool> {
    let id = EntityId::from_str(track).expect("ULID");
    dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .tracks
        .get(&id)
        .expect("轨")
        .invert_polarity
}

fn proposal_count(dispatcher: &mut Dispatcher) -> usize {
    let result = call_tool(dispatcher, "yeban_list_proposals", json!({}));
    assert_eq!(result["status"], "success", "{result}");
    result["data"]["proposals"]
        .as_array()
        .expect("提案清单")
        .len()
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

#[test]
fn positive_lag_delays_reference_through_proposal() {
    let mut dispatcher = dispatcher_with_stub(measurement(120, 0.95, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({})),
    );
    assert_eq!(result["status"], "success", "{result}");
    let data = &result["data"];
    assert_eq!(data["lagSamples"], 120, "{data}");
    assert_eq!(data["applied"], true, "{data}");
    // lag > 0 = 目标轨晚到 ⇒ 延先到的基准轨。
    assert_eq!(data["appliedDelay"]["trackId"], LEAD, "{data}");
    assert_eq!(data["appliedDelay"]["delaySamplesBefore"], 0, "{data}");
    assert_eq!(data["appliedDelay"]["delaySamplesAfter"], 120, "{data}");
    assert!(
        data["proposal"]["proposalId"].is_string(),
        "须经提案路径: {data}"
    );
    // 只建提案：工程一位不变。
    assert_eq!(track_delay(&dispatcher, LEAD), None);
    assert_eq!(proposal_count(&mut dispatcher), 1);
}

#[test]
fn auto_merge_applies_delay_at_once() {
    let mut dispatcher = dispatcher_with_stub(measurement(120, 0.95, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({ "autoMerge": true })),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["data"]["autoMerged"], true, "{result}");
    assert_eq!(track_delay(&dispatcher, LEAD), Some(120));
    assert_eq!(track_delay(&dispatcher, BASS), None);
}

#[test]
fn negative_lag_delays_target() {
    let mut dispatcher = dispatcher_with_stub(measurement(-80, 0.9, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({ "autoMerge": true })),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["data"]["appliedDelay"]["trackId"], BASS, "{result}");
    assert_eq!(track_delay(&dispatcher, BASS), Some(80));
    assert_eq!(track_delay(&dispatcher, LEAD), None);
}

#[test]
fn polarity_inversion_flips_target() {
    let mut dispatcher = dispatcher_with_stub(measurement(0, -0.9, true));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({ "autoMerge": true })),
    );
    assert_eq!(result["status"], "success", "{result}");
    let data = &result["data"];
    assert_eq!(data["polarityInverted"], true, "{data}");
    assert_eq!(data["appliedPolarity"]["trackId"], BASS, "{data}");
    assert_eq!(data["appliedPolarity"]["invertAfter"], true, "{data}");
    assert_eq!(track_invert(&dispatcher, BASS), Some(true));
}

#[test]
fn low_correlation_refuses_to_apply() {
    let mut dispatcher = dispatcher_with_stub(measurement(50, 0.2, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({})),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "CONFLICT", "{result}");
    assert_eq!(
        result["error"]["data"]["reason"], "lowCorrelation",
        "{result}"
    );
    assert_eq!(proposal_count(&mut dispatcher), 0);
}

#[test]
fn measure_only_returns_low_correlation() {
    let mut dispatcher = dispatcher_with_stub(measurement(50, 0.2, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({ "apply": false })),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["data"]["applied"], false, "{result}");
    assert_eq!(result["data"]["lagSamples"], 50, "{result}");
    assert_eq!(proposal_count(&mut dispatcher), 0);
}

#[test]
fn same_track_is_rejected() {
    let mut dispatcher = dispatcher_with_stub(measurement(0, 1.0, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        json!({ "referenceTrackId": LEAD, "targetTrackId": LEAD }),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(
        result["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{result}"
    );
    assert_eq!(
        result["error"]["data"]["reason"], "sameReferenceAndTarget",
        "{result}"
    );
}

#[test]
fn compensation_out_of_range_is_rejected() {
    let mut project = filled_project();
    let lead = EntityId::from_str(LEAD).expect("ULID");
    project
        .tracks
        .get_mut(&lead)
        .expect("lead 轨")
        .delay_samples = Some(4_750);
    let mut dispatcher = dispatcher_with_project(project);
    dispatcher
        .domain_mut()
        .set_phase_align_auto_provider(Arc::new(StubPhaseAlignAutoProvider::new(measurement(
            120, 0.95, false,
        ))));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({})),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "CONFLICT", "{result}");
    assert_eq!(
        result["error"]["data"]["reason"], "compensationOutOfRange",
        "{result}"
    );
    // 不部分施加：延时保持原值、无提案。
    assert_eq!(track_delay(&dispatcher, LEAD), Some(4_750));
    assert_eq!(proposal_count(&mut dispatcher), 0);
}

#[test]
fn dry_run_previews_without_proposal() {
    let mut dispatcher = dispatcher_with_stub(measurement(120, 0.95, false));
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({ "dryRun": true })),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(track_delay(&dispatcher, LEAD), None);
    assert_eq!(proposal_count(&mut dispatcher), 0);
}

#[test]
fn missing_provider_fails_loudly() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = call_tool(
        &mut dispatcher,
        "yeban_auto_align_tracks",
        align_args(json!({})),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "CONFLICT", "{result}");
    assert_eq!(
        result["error"]["data"]["reason"], "providerUnavailable",
        "{result}"
    );
}
