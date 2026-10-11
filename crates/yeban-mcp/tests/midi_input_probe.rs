//! `yeban_midi_input_probe` 的承重判据（`ADR-0006`，feature `midi-input` 默认关）。
//!
//! ## 为什么这份判据在**没有 MIDI 硬件**的机器上（含 CI）也成立
//!
//! 工具面被刻意切成两半：
//!
//! * **与硬件无关**的那一半（实参形状：空端口名、`seconds` 区间、类型）在
//!   `domain::midi_input::parse_arguments` 里 ⇒ **纯函数**，本文件完整覆盖；
//! * **与硬件有关**的那一半（枚举 / 打开 / 收窗口）只能在这台机器**有**端口时跑到，
//!   因此本文件对它只断言**结构性质**：要么是结构化不可用（`CONFLICT` ＋ 固定
//!   `data.reason`），要么是结构化参数错误 ⇒ ⛔ **永远不会**是
//!   "成功但 `events: 0`"（那会把"这台机器没有 MIDI 输入"谎报成"有端口但静默"）。
//!
//! ## 两档（`--no-default-features` 与 `--features midi-input`）都必须绿
//!
//! 期望值按 `cfg!(feature = "midi-input")` **分档**给出，而不是"随便接受一个错误"
//! —— 默认档必须**恰好**是 `midiInputFeatureDisabled`；特性档必须是
//! `midiInputNoPorts` 或 `midiInputHostUnavailable`（两者都说明"这台机器现在
//! 收不到硬件事件"，而后者是宿主自身不可用）。

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{self, ErrorCode, SideEffect, TOOL_COUNT};

/// 无工程的**新分发器**：本工具不要求活跃工程（硬件探测与工程状态无关）。
fn dispatcher() -> Dispatcher {
    let token = BearerToken::generate().token;
    Dispatcher::new(token, ScopeSet::all(), RunMode::Production)
}

fn bearer(dispatcher: &Dispatcher) -> String {
    format!("Bearer {}", dispatcher.expected_token().expose())
}

/// 一次 `tools/call` 的**带内** `ToolResponse`（成功或领域失败都走 `result`）。
fn call(dispatcher: &mut Dispatcher, arguments: Value) -> Value {
    let auth = bearer(dispatcher);
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "yeban_midi_input_probe", "arguments": arguments },
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "本工具不得产出 JSON-RPC 层错误（不可用是**带内**领域结果）: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 一次 `tools/call` 的**契约层**读数：`(http 状态, JSON-RPC 错误码)`。
///
/// 类型不符（例如 `port` 给了 JSON 数字）在**契约层**就被拒（`-32602`），
/// ⛔ 走不到领域代码 —— 这是 D46 那一课的同一条口径（类型违规不是带内领域失败）。
fn call_raw(dispatcher: &mut Dispatcher, arguments: Value) -> (u16, Option<i64>) {
    let auth = bearer(dispatcher);
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "yeban_midi_input_probe", "arguments": arguments },
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let status = outcome.http_status;
    let code = outcome.error_code();
    (status, code)
}

/// 带内失败的 `data.reason`（不是失败 ⇒ `None`）。
fn reason_of(response: &Value) -> Option<&str> {
    response
        .get("error")
        .and_then(|error| error.get("data"))
        .and_then(|data| data.get("reason"))
        .and_then(Value::as_str)
}

/// 判据 ①：工具**注册**且**只读**，工具集规模三处一致（`TOOL_COUNT` / 清单 / 表）。
#[test]
fn the_probe_is_registered_as_a_read_only_extension_with_its_two_parameters() {
    // R119/R120：非真空下界（工具数下界 + 扩展段下界）。
    // ⚠ 用**运行时**计数（`iter().count()`）而不是直接断常量 —— 后者会被
    // `clippy::assertions_on_constants` 判成"恒真断言"（那正是 R119/R120 的形状）。
    let registered = tools::TOOLS.iter().count();
    let extensions = tools::EXTENSION_NAMES.iter().count();
    assert_eq!(registered, TOOL_COUNT, "注册表长度必须等于 TOOL_COUNT");
    assert!(registered >= 21, "工具集规模下界不成立: {registered}");
    assert!(extensions >= 11, "扩展段下界不成立: {extensions}");
    assert_eq!(
        tools::TOOL_COUNT,
        tools::DOCUMENTED_TOOL_COUNT + tools::EXTENSION_TOOL_COUNT,
        "规范表 + 扩展段必须等于总数（三处读数机械一致）"
    );
    assert!(
        tools::EXTENSION_NAMES.contains(&"yeban_midi_input_probe"),
        "扩展名清单必须点名本工具（它是扩展段的唯一清单）"
    );
    let spec = tools::TOOLS
        .iter()
        .find(|spec| spec.name == "yeban_midi_input_probe")
        .expect("工具必须在注册表里: 工具集固定, 不可用只是**执行**时的结果");
    assert_eq!(spec.side_effect, SideEffect::ReadOnly, "本工具只读");
    let names: Vec<&str> = spec.params.iter().map(|param| param.name).collect();
    assert_eq!(names, vec!["port", "seconds"], "参数面就是这两个");
    // `port` 不得被写成必填：缺省在"恰好一个端口"时合法（规则写在工具文档里）。
    let port = spec.param("port").expect("port 参数");
    assert!(!port.required, "`port` 不得是必填");
    assert!(!spec.param("seconds").expect("seconds 参数").required);
    // 错误码必须落在契约联集里（⛔ 不发明新码）。
    for code in spec.errors {
        assert!(
            ErrorCode::SCHEMA_CONTRACT.contains(code),
            "`{code:?}` 不在契约 enum 里"
        );
    }
}

/// 判据 ②：**没有硬件 ⇒ 结构化不可用**；⛔ 不 panic、⛔ 不假装收到 0 条事件。
///
/// 两档的期望值**不同**（默认档 = feature 未启用；特性档 = 零端口或宿主不可用），
/// 但**共同**的断言是"这不是一个成功的 `events: 0`"。
#[test]
fn without_hardware_the_probe_reports_a_structured_unavailability_and_never_a_fake_zero() {
    let mut dispatcher = dispatcher();
    let response = call(&mut dispatcher, json!({}));
    assert_eq!(
        response["status"], "error",
        "没有硬件时不得回成功: {response}"
    );
    assert_eq!(
        response["error"]["code"], "CONFLICT",
        "不可用必须是 CONFLICT（契约码，⛔ 不发明新码）: {response}"
    );
    let reason = reason_of(&response).expect("不可用必须带结构化 data.reason");
    if cfg!(feature = "midi-input") {
        assert!(
            matches!(reason, "midiInputNoPorts" | "midiInputHostUnavailable"),
            "特性档的不可用只允许这两种原因（零端口 / 宿主不可用），实际 {reason}"
        );
    } else {
        assert_eq!(
            reason, "midiInputFeatureDisabled",
            "默认档必须**恰好**报 feature 未启用，实际 {reason}"
        );
        assert_eq!(
            response["error"]["data"]["feature"], "midi-input",
            "必须点名是哪个 feature"
        );
    }
    // ⛔ 核心否定断言：不可用**不得**被伪装成"收到 0 条事件"的成功响应。
    assert!(
        response.get("data").is_none() || response["data"].get("events").is_none(),
        "不可用响应里⛔ 不得出现 `events` 读数（那会把\"没有硬件\"谎报成\"有端口但静默\"）: {response}"
    );
    // 且工具**仍在** `tools/list` 里（工具集固定，⛔ 不因不可用而消失）。
    let auth = bearer(&dispatcher);
    let list = dispatcher.handle_line(
        Channel::Http,
        Some(&auth),
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string(),
    );
    let names: Vec<String> = list
        .response
        .expect("tools/list 响应")
        .result
        .expect("result")["tools"]
        .as_array()
        .expect("tools 数组")
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect();
    assert!(
        names.iter().any(|name| name == "yeban_midi_input_probe"),
        "不可用也要在 tools/list 里可见（工具集固定）"
    );
    assert_eq!(names.len(), TOOL_COUNT, "tools/list 必须与 TOOL_COUNT 一致");
}

/// 判据 ③：**与硬件无关**的参数校验（两档都完整覆盖，⛔ 不碰硬件）。
#[test]
fn bad_arguments_are_rejected_before_any_hardware_is_touched() {
    let cases: [(Value, &str); 4] = [
        (json!({"seconds": 0}), "midiInputSecondsOutOfRange"),
        (json!({"seconds": -1}), "midiInputSecondsOutOfRange"),
        (json!({"seconds": 61}), "midiInputSecondsOutOfRange"),
        (json!({"port": ""}), "midiInputEmptyPortName"),
    ];
    for (arguments, expected) in cases {
        let mut dispatcher = dispatcher();
        let response = call(&mut dispatcher, arguments.clone());
        assert_eq!(
            response["status"], "error",
            "坏实参必须被拒: {arguments} ⇒ {response}"
        );
        assert_eq!(
            response["error"]["code"], "INVALID_PARAMETER_RANGE",
            "坏实参的错误码: {arguments} ⇒ {response}"
        );
        assert_eq!(
            reason_of(&response),
            Some(expected),
            "坏实参的原因: {arguments} ⇒ {response}"
        );
    }
    // ⚠ **类型**不符（`port` 给 JSON 数字）在**契约层**就被拒：`-32602` + HTTP 400。
    // 领域里的 `midiInputPortTypeMismatch` 因此只服务**直接**调领域层的调用方
    //（工具路径上不可达）—— 这条断言把"到底哪一层拒"钉住。
    let (status, code) = call_raw(&mut dispatcher(), json!({"port": 7}));
    assert_eq!(status, 400, "类型不符必须走契约层 400");
    assert_eq!(code, Some(-32602), "类型不符必须是 `-32602`");
    // 边界**内**的实参不得被这一步拒（它们会走到硬件那一步，可能仍不可用，
    // 但⛔ 不许是**参数**错误）。
    for accept in [
        json!({"seconds": 0.5}),
        json!({"seconds": 60}),
        json!({"port": "0", "seconds": 1}),
    ] {
        let mut dispatcher = dispatcher();
        let response = call(&mut dispatcher, accept.clone());
        assert_ne!(
            reason_of(&response).unwrap_or(""),
            "midiInputSecondsOutOfRange",
            "边界内的 seconds 不得被区间校验拒: {accept} ⇒ {response}"
        );
        assert_ne!(
            reason_of(&response).unwrap_or(""),
            "midiInputEmptyPortName",
            "非空 port 不得被空串校验拒: {accept} ⇒ {response}"
        );
    }
}

/// 判据 ④：**不存在的端口名**与**歧义**都必须结构化拒绝，⛔ 不 panic。
///
/// 在**没有**端口的机器上（CI）这条会先撞上"零端口" ⇒ 本判据因此断言
/// "要么是端口选择类错误，要么是不可用类错误"，两者都**不是** panic、⛔ 也都不是成功。
#[test]
fn an_unknown_port_name_never_panics_and_stays_structured() {
    for port in ["no-such-midi-device-xyz", "1", "999999"] {
        let mut dispatcher = dispatcher();
        let response = call(&mut dispatcher, json!({ "port": port, "seconds": 1 }));
        assert_eq!(
            response["status"], "error",
            "端口 `{port}` 不得回成功: {response}"
        );
        let reason = reason_of(&response).expect("必须有结构化 reason");
        assert!(
            reason.starts_with("midiInput"),
            "reason 必须来自本工具的登记集合, 实际 `{reason}`"
        );
        assert!(
            matches!(
                reason,
                "midiInputUnknownPort"
                    | "midiInputAmbiguousPort"
                    | "midiInputNoSuchPortIndex"
                    | "midiInputPortRequired"
                    | "midiInputNoPorts"
                    | "midiInputHostUnavailable"
                    | "midiInputFeatureDisabled"
                    | "midiInputConnectFailed"
            ),
            "reason `{reason}` 不在登记集合里"
        );
    }
}
