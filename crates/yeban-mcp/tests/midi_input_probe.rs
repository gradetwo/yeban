//! `yeban_midi_input_probe` 的承重判据（`ADR-0006`，feature `midi-input` 默认关）。
//!
//! ## 为什么这份判据在**没有 MIDI 硬件**的机器上（含 CI）也成立
//!
//! 工具面被刻意切成**三层**，前两层本文件完整覆盖，第三层如实登记"无硬件不可判"：
//!
//! * **与硬件无关的实参形状**（空端口名、`seconds` 区间、类型）在
//!   `domain::midi_input::parse_arguments` 里 ⇒ **纯函数**；
//! * **与硬件无关的载荷组装**（`ProbeReading` → `probe_payload` →
//!   `ProbeReport::from_reading`）⇒ **纯函数**，⛔ 不挂 feature、⛔ 不碰 `open_input`。
//!   判据 ⑤⑥ 直接构造任意读数（含 `u64::MAX` 这种真机跑不到的输入）调它
//!   ⇒ 把此前**零覆盖**的 `unsupportedMessages`（类型 + 取值口径）真正钉住；
//! * **与硬件有关的那一半**（枚举 / 打开 / 收窗口）只能在这台机器**有**端口时跑到，
//!   因此本文件对它只断言**结构性质**：要么是结构化不可用（`CONFLICT` ＋ 固定
//!   `data.reason`），要么是结构化参数错误 ⇒ ⛔ **永远不会**是
//!   "成功但 `events: 0`"（那会把"这台机器没有 MIDI 输入"谎报成"有端口但静默"）。
//!
//! ## 为什么"载荷层"值得单独判（这是本轮补的那处零覆盖）
//!
//! 载荷原先与"收窗口"写在**同一个** `plan()` 里 ⇒ 那个函数要 `open_input` ⇒
//! 无硬件的 CI 根本走不到 ⇒ `unsupportedMessages` 的**类型**（数字 vs `null`）与
//! **取值口径**（等于传入值 vs 写死 0）此前**没有任何判据**。
//! 解耦之后，"序列化这一层"成为一个可被独立验证的真实单元。
//!
//! ## 两档（`--no-default-features` 与 `--features midi-input`）都必须绿
//!
//! 期望值按 `cfg!(feature = "midi-input")` **分档**给出，而不是"随便接受一个错误"
//! —— 默认档必须**恰好**是 `midiInputFeatureDisabled`；特性档必须是
//! `midiInputNoPorts` 或 `midiInputHostUnavailable`（两者都说明"这台机器现在
//! 收不到硬件事件"，而后者是宿主自身不可用）。
//!
//! ⚠ 判据 ⑤⑥ 的期望值与 feature **无关**（载荷函数两档都编译）⇒ 它们在任何一档
//! CI 腿里都会真的执行，⛔ 不会"绿着跳过"。

use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::extension_audit::path_ends_with;
use yeban_mcp::domain::midi_input::{
    JITTER_CALIBER, JitterNumbers, PortEntry, ProbeEvent, ProbeReading, ProbeReport,
    UNSUPPORTED_MESSAGES_CALIBER, probe_payload,
};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{self, ErrorCode, SideEffect, TOOL_COUNT};
use yeban_mcp::undo_session::{production_region, read_rust_sources};

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

// ---------------------------------------------------------------------------
// 判据 ⑤：载荷组装是**纯函数** ⇒ `unsupportedMessages` **无声卡可判**
// ---------------------------------------------------------------------------

/// 构造一个"除 `unsupportedMessages` 以外全部固定"的**纯读数**。
///
/// ⛔ 不碰 `open_input`、⛔ 不碰 `midir`、⛔ 不读时钟、⛔ 不读环境 ⇒ 因此在
/// **没有 MIDI 硬件**的机器上（含 CI）也能构造出任意输入，包括真机上要跑
/// 几百年才可能出现的 `u64::MAX`。
fn reading_with_unsupported(unsupported_messages: u64) -> ProbeReading {
    ProbeReading {
        port_index: 3,
        port_name: "Yeban Test Port".to_owned(),
        ports: vec![
            PortEntry {
                index: 3,
                name: "Yeban Test Port".to_owned(),
            },
            PortEntry {
                index: 5,
                name: "Other Port".to_owned(),
            },
        ],
        seconds: 1.5,
        events: 4,
        first_events: vec![ProbeEvent {
            timestamp_us: 1_000,
            status: 0x90,
            data1: 60,
            data2: 100,
        }],
        dropped_by_probe: 3,
        jitter: JitterNumbers {
            n: 3,
            p50: 900,
            p99: 1_200,
            max: 1_200,
        },
        unsupported_messages,
    }
}

/// 判据 ⑤-1（**这一条正是那处零覆盖的空格**）：`unsupportedMessages` 是**数字**，
/// ⛔ **不是** `null`、⛔ 也不是"缺字段"。
///
/// ⚠ 断言的是 `Value::Number` 这个**类型**，不是"值不为 null"这种可以被
/// 别的类型（字符串 `"7"`、布尔 `true`）蒙混过去的性质 —— 契约之外的类型会让
/// 调用方在解析时才发现问题。
#[test]
fn unsupported_messages_is_a_number_and_never_null() {
    for input in [0_u64, 7, u64::MAX] {
        let payload = probe_payload(&reading_with_unsupported(input));
        let field = payload
            .get("unsupportedMessages")
            .unwrap_or_else(|| panic!("字段必须存在（⛔ 不许缺字段）: {payload}"));
        assert!(
            matches!(field, Value::Number(_)),
            "必须是 `Value::Number`（输入 {input}），实际 `{field}`（{}）",
            match field {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
                Value::Number(_) => "number",
            }
        );
        assert!(field.is_number(), "`is_number()` 必须为真: {field}");
        assert_ne!(
            field,
            &Value::Null,
            "⛔ 不得是 null（这是真读数，不是占位）"
        );
        assert!(!field.is_null(), "⛔ 不得是 null");
        // 与生产端**同一条**载荷路径（`ProbeReport` 的唯一构造入口）。
        let via_report = ProbeReport::from_reading(&reading_with_unsupported(input)).data();
        assert_eq!(
            via_report["unsupportedMessages"], *field,
            "`probe_payload` 与 `ProbeReport::from_reading` 必须是同一条路径"
        );
    }
}

/// 判据 ⑤-2 ＋ ⑤-3：**等于传入值**（不是写死 `0`／`1`），且**跟着输入变**。
///
/// 三组输入都要断言（`0` / `7` / `u64::MAX`），因为"非 null"单独成立只能说明
/// "不再是 null"，说不出它是不是**真的数** —— 反向对照必须证明它随输入变。
///
/// ⚠ **数值口径（如实断言，⛔ 不假设会被截断）**：生产端的计数器是 `AtomicU64`
/// ⇒ 载荷里的 `unsupportedMessages` 一路走 `u64`，**全宽、不截断**。
/// 因此 `u64::MAX` 的序列化文本是 `18446744073709551615`（20 位）。
#[test]
fn unsupported_messages_equals_the_input_value() {
    let cases: [(u64, &str); 3] = [(0, "0"), (7, "7"), (u64::MAX, "18446744073709551615")];
    let mut observed: Vec<u64> = Vec::new();
    for (input, expected_text) in cases {
        let payload = probe_payload(&reading_with_unsupported(input));
        let field = &payload["unsupportedMessages"];
        assert_eq!(
            field.as_u64(),
            Some(input),
            "必须等于**传入值本身**（⛔ 不是写死的常量）: 输入 {input} ⇒ {field}"
        );
        assert_eq!(field, &Value::from(input), "必须等于 `Value::from(输入)`");
        assert_eq!(
            field.to_string(),
            expected_text,
            "序列化文本口径（u64 全宽, ⛔ 不截断）: 输入 {input}"
        );
        // 口径文案必须与常量**同源**（⛔ 不是判据里手抄的第二份）。
        assert_eq!(
            payload["unsupportedMessagesCaliber"], UNSUPPORTED_MESSAGES_CALIBER,
            "口径文案必须来自 `UNSUPPORTED_MESSAGES_CALIBER`"
        );
        assert_eq!(payload["jitter"]["caliber"], JITTER_CALIBER);
        observed.push(field.as_u64().expect("u64"));
    }
    // ③ 反向对照：三组不同输入必须得到三个**不同**输出。
    //    若实现把该字段写死（例如恒 `0`、恒 `1`、或仍是 `null`），这条会红。
    let mut distinct = observed.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        cases.len(),
        "输出必须**跟着输入变**（写死值 / 恒 null 会在这里被抓到）: {observed:?}"
    );
}

/// 判据 ⑤ 的**整条载荷**复核（同一层里的其它字段也一并钉住，证明这一层真的被判了）。
#[test]
fn the_whole_payload_shape_is_judged_without_hardware() {
    let payload = probe_payload(&reading_with_unsupported(7));
    assert_eq!(payload["port"]["index"], 3);
    assert_eq!(payload["port"]["name"], "Yeban Test Port");
    assert_eq!(payload["ports"].as_array().expect("ports").len(), 2);
    assert_eq!(payload["ports"][1]["name"], "Other Port");
    assert_eq!(payload["seconds"], 1.5);
    assert_eq!(
        payload["events"], 4,
        "`events` 是总数, ⛔ 不是 firstEvents 的长度"
    );
    assert_eq!(payload["firstEvents"][0]["status"], 0x90);
    assert_eq!(payload["firstEvents"][0]["data2"], 100);
    assert_eq!(payload["droppedByProbe"], 3);
    assert_eq!(payload["jitter"]["n"], 3);
    assert_eq!(payload["jitter"]["p99"], 1_200);
    assert_eq!(payload["readOnly"], true);
    // `dryRun` 与真调用共用这一份载荷（`apply` 原样回传），因此把构造入口也钉一下。
    assert_eq!(
        ProbeReport::from_reading(&reading_with_unsupported(7)).data(),
        payload
    );
}

// ---------------------------------------------------------------------------
// 判据 ⑥：**非真空守卫** —— 载荷只在**一个**地方组装
// ---------------------------------------------------------------------------

/// 在源码清单里按后缀找文件。
///
/// ⚠ **归一化分隔符**是承重的：`read_rust_sources` 给的是 `Path::display()`，
/// 在 Windows 上用 `\` 分隔 ⇒ 裸 `ends_with("domain/x.rs")` 会**恒 false**。
/// 抽成函数是为了让它**可以被合成清单单独判**（见下一条判据）——
/// 否则"查找点是否跨平台"只能靠 Windows 腿才发现（本轮就是这么红的）。
fn find_source<'a>(sources: &'a [(String, String)], suffix: &str) -> Option<&'a (String, String)> {
    sources
        .iter()
        .find(|(path, _)| path_ends_with(path, suffix))
}

/// 判据 ⑥ 的**跨平台臂**：查找函数对两种分隔符都必须命中。
///
/// 用**纯反斜杠的合成清单**（而不是直接调 `path_ends_with` 传字面量）审查查找点本身 ——
/// 这是本轮从 `38153839076` 的 Windows 失败里学到的：
/// 只钉助手不钉查找点，等于没钉（实测：把查找改回裸 `ends_with` 时，
/// "只钉助手"的版本在 Linux 上**仍然 8 passed**）。
#[test]
fn the_source_lookup_is_separator_agnostic() {
    let target = "domain/midi_input.rs";

    // 合成清单：只有**反斜杠**路径（Windows 形态）。
    let windows_style: Vec<(String, String)> = vec![
        (
            r"crates\yeban-mcp\src\domain\other.rs".to_owned(),
            "other".to_owned(),
        ),
        (
            r"crates\yeban-mcp\src\domain\midi_input.rs".to_owned(),
            "midi_input".to_owned(),
        ),
    ];
    assert_eq!(
        find_source(&windows_style, target).map(|(_, text)| text.as_str()),
        Some("midi_input"),
        "反斜杠路径必须命中（Windows 腿的回归：裸 `ends_with` 在这里会返回 None）"
    );

    // 合成清单：只有**正斜杠**路径（Unix 形态）。
    let unix_style: Vec<(String, String)> = vec![(
        "crates/yeban-mcp/src/domain/midi_input.rs".to_owned(),
        "midi_input".to_owned(),
    )];
    assert_eq!(
        find_source(&unix_style, target).map(|(_, text)| text.as_str()),
        Some("midi_input"),
        "正斜杠路径必须命中"
    );

    // 反例：后缀相近的文件 ⛔ 不得命中（防止"匹配太宽"把判据变成空转）。
    for other in [
        r"crates\yeban-mcp\src\domain\midi_input.rs.bak",
        "src/domain/midi_input_extra.rs",
    ] {
        assert!(
            find_source(&[(other.to_owned(), "x".to_owned())], target).is_none(),
            "`{other}` ⛔ 不得命中 `{target}`"
        );
    }

    // 与真实清单对照（非真空：合成的两条臂都通过时，真清单也必须真的找得到）。
    let real = read_rust_sources(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")]);
    assert!(real.len() >= 20, "真实清单太小: {}", real.len());
    assert!(
        find_source(&real, target).is_some(),
        "真实源码清单里必须能找到 `{target}`"
    );
}

/// 判据 ⑥：生产区里 `"unsupportedMessages"` 这个 JSON 键**恰好出现一次**，
/// 且那一处落在 [`probe_payload`] 体内。
///
/// 为什么需要它：判据 ⑤ 直接调 `probe_payload`。若日后有人在别处**再**拼一份
/// 带这个键的 `json!`（例如把 `plan()` 改回自己拼），判据 ⑤ 会**继续绿**
/// 但线上走的是**另一份**载荷 ⇒ 判据变成空转。
/// 这条守卫把"载荷只在一处组装"变成机械可判的事实。
///
/// ## ⚠ 跨平台（Windows 腿换来的教训）
///
/// 第一版写的是 `path.ends_with("domain/midi_input.rs")`。在 **Linux/macOS** 上绿，
/// 在 **Windows** 上 `read_rust_sources` 给的路径用 `\` 分隔 ⇒ 该比较**恒 false** ⇒
/// 判据 panic（实测：run `38153839076` 的 `windows` 腿，
/// `panicked at ...: 必须扫到 domain/midi_input.rs（扫描到 38 个 .rs 文件）`）。
/// 这与本仓库 `non_vacuity_guard` 记为 R93 的那条是同一个陷阱
///（`path.contains("/src/")` 在 Windows 上恒 false）。
///
/// ⚠ **第一版"修法"是假的**：我把比较换成共享助手 [`path_ends_with`]，
/// 又加了反斜杠正例 —— 但那些正例直接调**助手**，用的是**字面量**，
/// 于是"把查找改回裸 `ends_with`"这个注入在 Linux 上**依然全绿**（实测：exit 0, 8 passed）。
/// 也就是说那条对照只钉住了助手，**没有**钉住查找点。
///
/// 真正的修法是把查找本身变成**可测的单元**：见 [`find_source`] 与
/// `the_source_lookup_is_separator_agnostic` —— 后者用**纯反斜杠的合成清单**调查找函数，
/// 因此在 **Linux 上**把 `find_source` 改回裸 `ends_with` 就会红（已验证）。
///
/// [`probe_payload`]: yeban_mcp::domain::midi_input::probe_payload
/// [`path_ends_with`]: yeban_mcp::domain::extension_audit::path_ends_with
#[test]
fn the_payload_is_built_in_exactly_one_place() {
    // 先命名指标：数的是 `crates/yeban-mcp/src/` 下**几个 `.rs` 文件**（单位 = 文件数）。
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let sources = read_rust_sources(&[manifest_dir.join("src")]);
    assert!(
        sources.len() >= 20,
        "扫描面太小（单位 = 文件数）: {} —— 说明扫描没生效",
        sources.len()
    );

    let target = "domain/midi_input.rs";
    let (_, text) = find_source(&sources, target).unwrap_or_else(|| {
        panic!(
            "必须扫到 `{target}`（扫描到 {} 个 .rs 文件）",
            sources.len()
        )
    });
    let production = production_region(text);
    let key = "\"unsupportedMessages\"";
    let occurrences = production.matches(key).count();
    assert_eq!(
        occurrences, 1,
        "生产区里 `{key}` 这个 JSON 键必须**恰好出现一次**（实际 {occurrences} 次）—— \
         多一处就意味着可能存在第二份载荷、判据 ⑤ 会空转"
    );

    // 且那一处必须落在 `probe_payload` 的**函数体内**（不是被搬到别处）。
    let fn_start = production
        .find("pub fn probe_payload")
        .expect("`probe_payload` 必须在生产区里");
    let body_end = production[fn_start..]
        .find("\n}\n")
        .map(|offset| fn_start + offset + 1)
        .expect("必须能找到 `probe_payload` 的函数体结束（列 0 的 `}}`）");
    let key_at = production.find(key).expect("键的那一处出现");
    assert!(
        key_at > fn_start && key_at < body_end,
        "键必须落在 `probe_payload` 体内（偏移 {key_at} 不在 ({fn_start}, {body_end}) 之间）\
         —— 否则载荷是在别处拼的, 判据 ⑤ 判的不是线上那条路径"
    );

    // 反向对照：这个扫描器**真的**会数（把键塞进一段合成文本 ⇒ 计数变 2）。
    let synthetic = format!("{production}\nfn extra() {{ let _ = json!({{ {key}: 1 }}); }}\n");
    assert_eq!(
        synthetic.matches(key).count(),
        2,
        "扫描器必须能数出第二处（否则上面的 `== 1` 是恒真的空判据）"
    );
}
