//! `compare_song_snapshots`（Songmap A/B 快照 Diff，设计稿 §8.3，Phase 2）
//! 的端到端判据。
//!
//! 全部断言都以本文件内手写的**两小节快照 JSON**为被测对象（不读
//! `domain::compare_snapshots` 的夹具常量、不依赖 stub provider）：
//! - 注册：`tools/list` 里可发现，`inputSchema` 要求两个对象实参；
//! - 正向：两份合法快照 ⇒ `success` + 逐位手算的差值（张力 / 响度 / 频段）；
//! - 不要求活跃工程：不开工程也成功（Diff 是入参的纯函数）；
//! - 失败：形状非法 ⇒ 带内 `INVALID_PARAMETER_RANGE` + `invalidSnapshot`；
//!   版本不等 ⇒ 同码 + `snapshotMismatch`；非对象实参 ⇒ 协议层 `-32602`。
//!   ⛔ 不依赖任何外部服务；不碰真实渲染音频；不重录金图。

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::TOOL_COUNT;

// ---------------------------------------------------------------------------
// 脚手架（与 `tests/song_overview.rs` 同款，不跨测试文件引用）
// ---------------------------------------------------------------------------

fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    (dispatcher, auth)
}

/// 一次 `tools/call` 的 `ToolResponse`（`result` 里的 JSON 对象；领域失败走带内错误）。
fn call_tool(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误（领域失败走 ToolResponse）: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 一次 JSON-RPC 方法调用（不是 `tools/call`）—— `tools/list` 用。
fn request(dispatcher: &mut Dispatcher, auth: &str, method: &str, params: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "r",
        "method": method,
        "params": params,
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{method}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

// ---------------------------------------------------------------------------
// 夹具：两小节快照 JSON（手写，不读实现侧常量）
// ---------------------------------------------------------------------------

fn row_json(bar: u64, tension: f64, lufs_short: f64, band0: f64) -> Value {
    json!({
        "bar": bar,
        // 120 BPM、4/4 下每小节 2 秒（只为夹具自洽；Diff 不比时间坐标）。
        "time_sec": (bar - 1) as f64 * 2.0,
        "section": "Verse",
        "tension": tension,
        "valence_hue": 0.5,
        "lufs_short": lufs_short,
        "lufs_momentary_max": -10.0,
        "score_dyn": 0.5,
        "envelope_lo": 0.2,
        "envelope_hi": 0.8,
        "target_deviation": 0.0,
        "density_pitched_onsets_per_beat": 1.0,
        "density_pitched_duty": 0.5,
        "density_perc_onsets_per_beat": 1.0,
        "bands_tilted_db": [band0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    })
}

fn snapshot_json(tension_1: f64, tension_2: f64, lufs_1: f64, band0_1: f64) -> Value {
    json!({
        "schema_version": "1.2",
        "song_meta": {
            "total_bars": 2,
            "duration_sec": 10.0,
            "base_bpm": 120.0,
            "key_signature": "C major",
            "profile": "pop",
        },
        "band_definitions_hz": [[20, 60], [60, 250], [250, 500], [500, 2000], [2000, 4000], [4000, 6000], [6000, 10000], [10000, 20000]],
        "structural_summary": {
            "climax_bar": 1,
            "climax_tension": 0.9,
            "dynamic_range_lu": 8.0,
            "detected_anomalies": [],
        },
        "repetition": {
            "mechanical_index": 0.3,
            "best_matches": [],
            "repeated_passages": [],
        },
        // 夹具无分轨打架：空摘要（schema 1.2 起 `frequency_clash` 必填）。
        "frequency_clash": {
            "total_groups": 0,
            "overall_overlap": 0.0,
            "bar_overlaps": [],
            "entries": [],
        },
        "bar_series": [
            row_json(1, tension_1, lufs_1, band0_1),
            row_json(2, tension_2, -12.0, 1.0),
        ],
    })
}

/// 判据 ①：注册可发现 —— `tools/list` 里有本工具，`inputSchema` 要求两个对象实参。
#[test]
fn compare_is_discoverable_in_tools_list() {
    let (mut dispatcher, auth) = dispatcher();
    let listed = request(&mut dispatcher, &auth, "tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), TOOL_COUNT, "工具面条数必须等于 TOOL_COUNT");
    let tool = tools
        .iter()
        .find(|tool| tool["name"] == "compare_song_snapshots")
        .expect("tools/list 里必须有 compare_song_snapshots");
    let required = tool["inputSchema"]["required"]
        .as_array()
        .expect("inputSchema.required");
    assert_eq!(
        required,
        &[Value::from("snapshot_a"), Value::from("snapshot_b"),],
        "两个快照实参都必填"
    );
    assert_eq!(
        tool["inputSchema"]["properties"]["snapshot_a"]["type"], "object",
        "snapshot_a 必须是 object"
    );
    assert_eq!(
        tool["inputSchema"]["properties"]["snapshot_b"]["type"], "object",
        "snapshot_b 必须是 object"
    );
}

/// 判据 ②：正向通路 —— 两份合法快照 ⇒ `success` + 逐位手算的差值。
/// 本判据不开工程：Diff 是入参的纯函数，不要求活跃工程。
#[test]
fn compare_returns_hand_computed_diff_without_active_project() {
    let (mut dispatcher, auth) = dispatcher();
    let result = call_tool(
        &mut dispatcher,
        &auth,
        "compare_song_snapshots",
        json!({
            "snapshot_a": snapshot_json(0.3, 0.5, -14.0, -2.0),
            "snapshot_b": snapshot_json(0.5, 0.5, -13.0, -1.0),
        }),
    );
    assert_eq!(result["status"], "success", "合法快照对必须成功: {result}");
    let data = &result["data"];
    assert_eq!(data["schema_version"], "1.2");
    assert_eq!(data["summary"]["bars_compared"], 2);
    assert_eq!(data["summary"]["bars_changed"], 1);
    assert_eq!(data["summary"]["max_abs_tension_delta"], 0.2);
    assert_eq!(data["summary"]["max_abs_lufs_delta"], 1.0);
    assert_eq!(data["summary"]["max_abs_band_delta_db"], 1.0);
    assert_eq!(data["bar_diffs"][0]["tension_delta"], 0.2);
    assert_eq!(data["bar_diffs"][0]["lufs_short_delta"], 1.0);
    assert_eq!(data["bar_diffs"][0]["bands_tilted_db_delta"][0], 1.0);
    assert_eq!(data["bar_diffs"][0]["changed"], true);
    assert_eq!(data["bar_diffs"][1]["changed"], false);
    assert_eq!(
        data["commitModel"], "none",
        "只读工具的 commitModel 必须是 none"
    );
}

/// 判据 ③：形状非法 ⇒ 带内 `INVALID_PARAMETER_RANGE` + `invalidSnapshot`。
#[test]
fn malformed_snapshot_is_invalid_snapshot() {
    let (mut dispatcher, auth) = dispatcher();
    let result = call_tool(
        &mut dispatcher,
        &auth,
        "compare_song_snapshots",
        json!({
            "snapshot_a": {"schema_version": "1.1"},
            "snapshot_b": snapshot_json(0.5, 0.5, -13.0, -1.0),
        }),
    );
    assert_eq!(result["status"], "error", "缺字段的快照必须失败: {result}");
    assert_eq!(result["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert_eq!(result["error"]["data"]["reason"], "invalidSnapshot");
    assert_eq!(result["error"]["data"]["snapshot"], "snapshot_a");
}

/// 判据 ④：版本不等 ⇒ 带内 `INVALID_PARAMETER_RANGE` + `snapshotMismatch`。
#[test]
fn version_mismatch_is_snapshot_mismatch() {
    let (mut dispatcher, auth) = dispatcher();
    let mut b = snapshot_json(0.5, 0.5, -13.0, -1.0);
    b["schema_version"] = Value::from("1.0");
    let result = call_tool(
        &mut dispatcher,
        &auth,
        "compare_song_snapshots",
        json!({
            "snapshot_a": snapshot_json(0.3, 0.5, -14.0, -2.0),
            "snapshot_b": b,
        }),
    );
    assert_eq!(
        result["status"], "error",
        "版本不等的快照必须失败: {result}"
    );
    assert_eq!(result["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert_eq!(result["error"]["data"]["reason"], "snapshotMismatch");
    assert_eq!(result["error"]["data"]["field"], "schema_version");
}

/// 判据 ⑤：拍级快照 ⇒ 带内 `INVALID_PARAMETER_RANGE` + `snapshotMismatch`
/// （`field: "resolution"`；Diff 只比小节级，设计稿 §11）。
#[test]
fn beat_level_snapshot_is_snapshot_mismatch() {
    let (mut dispatcher, auth) = dispatcher();
    let mut b = snapshot_json(0.5, 0.5, -13.0, -1.0);
    b["bar_series"][0]["beat"] = Value::from(1);
    let result = call_tool(
        &mut dispatcher,
        &auth,
        "compare_song_snapshots",
        json!({
            "snapshot_a": snapshot_json(0.3, 0.5, -14.0, -2.0),
            "snapshot_b": b,
        }),
    );
    assert_eq!(result["status"], "error", "拍级快照必须失败: {result}");
    assert_eq!(result["error"]["code"], "INVALID_PARAMETER_RANGE");
    assert_eq!(result["error"]["data"]["reason"], "snapshotMismatch");
    assert_eq!(result["error"]["data"]["field"], "resolution");
}

/// 判据 ⑥：非对象实参 ⇒ 协议层 `-32602`（注册表类型校验先于领域层）。
#[test]
fn non_object_snapshot_is_protocol_rejection() {
    let (mut dispatcher, auth) = dispatcher();
    let line = json!({
        "jsonrpc": "2.0",
        "id": "non-object",
        "method": "tools/call",
        "params": {
            "name": "compare_song_snapshots",
            "arguments": {
                "snapshot_a": "not-an-object",
                "snapshot_b": snapshot_json(0.5, 0.5, -13.0, -1.0),
            },
        },
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let response = outcome.response.expect("必须有响应");
    let error = response.error.expect("非对象实参必须是 JSON-RPC 层错误");
    assert_eq!(error.code, -32602, "非对象实参必须是 INVALID_PARAMS");
    assert!(response.result.is_none(), "非对象实参不得产生带内成功结果");
}
