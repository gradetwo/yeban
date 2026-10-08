//! **`[BASELINE-006]` 载荷判据**：MCP 往返的序列化 JSON 字节数必须在上限之内。
//!
//! ## 规范原文
//!
//! `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:361`：
//!
//! > `[BASELINE-006]` | **AI 交互效率** | 单次段落生成数据负载与 Token 开销 |
//! > 统计生成 16 小节段落的完整 MCP 工具往返载荷 |
//! > **序列化 JSON 载荷 ≤ 4 KB，结构化字段传输**，Token 开销中位数 ≤ 600 Tokens ...
//!
//! ## 这个文件量什么、单位是什么
//!
//! - **对象**：一次 `tools/call` 的**线上 JSON 文本行**（请求 + 响应）；
//! - **单位**：UTF-8 **字节**（口径与计数函数见 [`yeban_mcp::payload`]）；
//! - **场景**：`[BASELINE-006]` 自己点名的"生成 **16 小节**段落"
//!   （[`yeban_mcp::payload::baseline_006_section_line`]）。
//!
//! `docs/ledger/gate-status.md` 记 `BASELINE-006` 为 PENDING，理由是"需要生成 16 小节
//! 段落的完整 MCP 往返统计；十个工具已能真做事，但**载荷统计未接**"。本文件接上
//! **JSON 那半边**。Token 那半边需要分词器口径（人类已明示延后），本文件**不**
//! 用"字节 ÷ 常数"冒充它（见 `crate::payload` 的文件头）。
//!
//! 复跑读数：`cargo test -p yeban-mcp --test payload_budget -- --nocapture`。

use std::collections::BTreeSet;

use serde_json::{Value, json};

use yeban_mcp::dispatch::{Dispatcher, Outcome};
use yeban_mcp::payload::{self, RoundTripPayload};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个带**活跃工程**的分发器：规范样本工程 + 固定时钟 + 全作用域。
///
/// 工程是 `yeban_model::samples::filled_project()`（有可用的 MIDI 材料与主总线），
/// 因此 `yeban_propose_section` 是**可成功执行**的真实调用，不是一个必然报错的空壳。
fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::path::PathBuf::from("/tmp/yeban-mcp-payload-budget.yeban"),
            yeban_model::samples::filled_project(),
            false,
        )
        .expect("注入规范工程");
    (dispatcher, authorization)
}

/// 一条 `tools/call` 请求行（与线上同形）。
fn call_line(name: &str, arguments: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": "budget-probe",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string()
}

/// 响应里的 `ToolResponse` 本体（`result.data`）。
fn data_of(outcome: &Outcome) -> Value {
    outcome
        .response
        .as_ref()
        .expect("这条请求必须产生响应")
        .result
        .clone()
        .expect("这条请求必须成功")
        .get("data")
        .cloned()
        .expect("成功响应必须带 data")
}

/// 一个对象值的键集合（用来断言"只多了一个键"，而不是"看起来没变"）。
fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("必须是对象")
        .keys()
        .cloned()
        .collect()
}

/// `[BASELINE-006]` 场景：16 小节段落生成。
fn section_arguments(include_ops: Option<bool>) -> Value {
    let mut arguments = json!({
        "sectionName": "Chorus",
        "stylePreset": payload::BASELINE_006_STYLE_PRESET,
        "bars": payload::BASELINE_006_SECTION_BARS,
        "scale": "C minor",
    });
    if let Some(include_ops) = include_ops {
        arguments["includeOps"] = Value::from(include_ops);
    }
    arguments
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// **判据 ①（承重）**：16 小节段落生成的完整往返 ≤ 4 KB，且响应是**结构化字段**。
///
/// 变红的方式（都是实测过的注入，见文件末的 §注入证据）：
/// 把缺省形状改回"回传完整 op 载荷" ⇒ 这条判据的"缺省不得回传 ops"先红；
/// 把 `BASELINE_006_JSON_LIMIT_BYTES` 改小 ⇒ 预算断言红，字面读数是
/// `必须 ≤ 1024 B, 实测 request 196 B + response 2747 B = 2943 B`。
#[test]
fn sixteen_bar_section_round_trip_fits_the_baseline_006_json_budget() {
    let (mut dispatcher, authorization) = dispatcher();
    let line = payload::baseline_006_section_line(payload::BASELINE_006_SECTION_BARS);
    let (reading, outcome) =
        payload::measure(&mut dispatcher, Channel::Stdio, Some(&authorization), &line);
    println!(
        "BASELINE-006 · 16 小节 yeban_propose_section（缺省形状）: {reading} · 上限 {} B",
        payload::BASELINE_006_JSON_LIMIT_BYTES
    );
    assert_eq!(outcome.http_status, 200, "这条调用必须成功: {reading}");

    let data = data_of(&outcome);
    // "结构化字段传输"：结构化清单在，逐条 op 载荷不在。
    assert!(
        data["willCreate"]["opKinds"]
            .as_array()
            .is_some_and(|kinds| !kinds.is_empty()),
        "结构化清单 willCreate.opKinds 必须存在: {data}"
    );
    assert_eq!(
        data["proposal"]["opCount"], data["willCreate"]["opCount"],
        "提案摘要与派生清单的 op 数必须一致: {data}"
    );
    assert!(
        data["proposal"]["ops"].is_null(),
        "缺省形状**不得**回传完整 op 载荷: {data}"
    );

    assert!(
        reading.within_baseline_006(),
        "16 小节段落生成的往返 JSON 必须 ≤ {} B, 实测 {reading}",
        payload::BASELINE_006_JSON_LIMIT_BYTES
    );
    // 钉住**实测读数**（单位：字节）。它变红意味着载荷形状变了 ⇒ 必须重新量一次
    // 再决定新的期望值，不许顺手改数字（AGENTS.md §6「逐位一致判据」的精神）。
    assert_eq!(
        reading,
        RoundTripPayload {
            request: 196,
            response: 2747
        },
        "BASELINE-006 的 16 小节缺省读数变了"
    );
}

/// **判据 ②**：完整 op 载荷是**显式可选**的，且它只往响应里**加一个键** `ops`。
///
/// 变量：同一场景的两次调用（缺省 / `includeOps: true`），两个独立会话。
/// 变红的方式：让 `includeOps` 无效 ⇒ `left: Null / right: [...]`；或者让缺省也带
/// `ops` ⇒ 键集合断言红。
#[test]
fn full_op_payloads_are_opt_in_and_only_add_the_ops_key() {
    let (mut plain, plain_auth) = dispatcher();
    let (mut full, full_auth) = dispatcher();

    let (plain_reading, plain_outcome) = payload::measure(
        &mut plain,
        Channel::Stdio,
        Some(&plain_auth),
        &call_line("yeban_propose_section", section_arguments(None)),
    );
    let (full_reading, full_outcome) = payload::measure(
        &mut full,
        Channel::Stdio,
        Some(&full_auth),
        &call_line("yeban_propose_section", section_arguments(Some(true))),
    );

    let plain_proposal = &data_of(&plain_outcome)["proposal"];
    let full_proposal = &data_of(&full_outcome)["proposal"];
    println!("BASELINE-006 · 缺省形状: {plain_reading}");
    println!("BASELINE-006 · includeOps=true: {full_reading}");

    assert!(
        plain_proposal["ops"].is_null(),
        "缺省不得带 ops: {plain_proposal}"
    );
    let ops = full_proposal["ops"]
        .as_array()
        .expect("includeOps=true 必须回传逐条 ops");
    assert_eq!(
        ops.len(),
        full_proposal["opCount"].as_u64().expect("opCount") as usize,
        "ops 条数必须等于 opCount"
    );
    assert_eq!(
        ops[0]["op"]["SetSection"]["new_section"]["name"], "Chorus",
        "逐条 op 载荷必须是真的 op 本体: {ops:?}"
    );

    // 精确的形状差异：`ops` 是**唯一**多出来的键。
    let plain_keys = keys(plain_proposal);
    let full_keys = keys(full_proposal);
    let added: Vec<&String> = full_keys.difference(&plain_keys).collect();
    assert_eq!(added, vec![&"ops".to_owned()], "只许多出 ops 这一个键");
    assert_eq!(
        plain_keys.difference(&full_keys).count(),
        0,
        "缺省形状不得比重载荷少键"
    );

    assert!(
        full_reading.total() > plain_reading.total(),
        "回传完整 op 载荷必须更大: 缺省 {plain_reading} / 完整 {full_reading}"
    );
}

/// **判据 ③**：`includeOps` **只**改响应的形状，不改被创建的提案与工程。
///
/// 两侧各用一份全新会话（同一个固定时钟与同一份样本工程），逐项核对结构性事实：
/// 工程摘要、提交数、提案数、op 序列（`opKinds`）与结构化清单。实体身份（ULID）
/// 逐次随机，因此**不**比身份字符串，比的是"同一件事发生了没有"。
#[test]
fn include_ops_only_changes_the_response_shape() {
    let (mut plain, plain_auth) = dispatcher();
    let (mut full, full_auth) = dispatcher();

    let plain_outcome = plain.handle_line(
        Channel::Stdio,
        Some(&plain_auth),
        &call_line("yeban_propose_section", section_arguments(None)),
    );
    let full_outcome = full.handle_line(
        Channel::Stdio,
        Some(&full_auth),
        &call_line("yeban_propose_section", section_arguments(Some(true))),
    );

    let plain_data = data_of(&plain_outcome);
    let full_data = data_of(&full_outcome);

    for key in ["created", "projectUnchanged", "projectDigest", "unwired"] {
        assert_eq!(plain_data[key], full_data[key], "`{key}` 必须两侧相同");
    }
    for key in ["kind", "title", "status", "opCount", "opKinds"] {
        assert_eq!(
            plain_data["proposal"][key], full_data["proposal"][key],
            "提案的 `{key}` 必须两侧相同"
        );
    }
    assert_eq!(
        plain_data["willCreate"], full_data["willCreate"],
        "派生清单必须两侧相同（它由同一份 op 序列数出来）"
    );

    assert_eq!(
        plain.domain().commit_count(),
        full.domain().commit_count(),
        "提交数必须两侧相同"
    );
    assert_eq!(
        plain.domain().proposal_count(),
        full.domain().proposal_count(),
        "提案数必须两侧相同"
    );
    assert_eq!(
        serde_json::to_string(plain.domain().active_project().expect("工程")).expect("序列化"),
        serde_json::to_string(full.domain().active_project().expect("工程")).expect("序列化"),
        "活跃工程必须逐字节相同（提案不改工程内容）"
    );
}

/// **判据 ④**：`dryRun` 预览也受同一条预算约束，且同样尊重 `includeOps`。
///
/// 预览是 Agent 在真调用之前的一次往返，因此它也在 `[BASELINE-006]` 的"往返载荷"里。
#[test]
fn the_dry_run_preview_is_also_inside_the_budget_and_honours_include_ops() {
    let (mut plain, plain_auth) = dispatcher();
    let mut dry = section_arguments(None);
    dry["dryRun"] = Value::from(true);
    let (plain_reading, plain_outcome) = payload::measure(
        &mut plain,
        Channel::Stdio,
        Some(&plain_auth),
        &call_line("yeban_propose_section", dry.clone()),
    );
    println!("BASELINE-006 · 16 小节 dryRun 预览（缺省形状）: {plain_reading}");
    let plain_preview = data_of(&plain_outcome)["preview"].clone();
    assert!(
        plain_preview["willCreate"]["opKinds"]
            .as_array()
            .is_some_and(|kinds| !kinds.is_empty()),
        "预览必须给出结构化清单: {plain_preview}"
    );
    assert!(
        plain_preview["ops"].is_null(),
        "缺省预览不得回传完整 op 载荷: {plain_preview}"
    );
    assert!(
        plain_reading.within_baseline_006(),
        "dryRun 往返也必须 ≤ {} B, 实测 {plain_reading}",
        payload::BASELINE_006_JSON_LIMIT_BYTES
    );

    let (mut full, full_auth) = dispatcher();
    let mut dry_full = section_arguments(Some(true));
    dry_full["dryRun"] = Value::from(true);
    let full_outcome = full.handle_line(
        Channel::Stdio,
        Some(&full_auth),
        &call_line("yeban_propose_section", dry_full),
    );
    let full_preview = data_of(&full_outcome)["preview"].clone();
    let ops = full_preview["ops"]
        .as_array()
        .expect("includeOps=true 的预览必须回传逐条 ops");
    assert_eq!(
        ops.len(),
        full_preview["opCount"].as_u64().expect("opCount") as usize,
        "预览的 ops 条数必须等于 opCount"
    );
}

/// **判据 ⑤**：统计器按**线上文本**计数，`notification` 没有响应 ⇒ 响应 0 字节。
///
/// 这条判据钉的是统计口径本身：把"没有响应"记成"响应的字节数"会让读数虚高，
/// 把"请求字节"记成"值里的字段数"会让读数虚低。
#[test]
fn notification_round_trip_counts_zero_response_bytes() {
    let (mut dispatcher, authorization) = dispatcher();
    let notification = json!({"jsonrpc": "2.0", "method": "tools/list"}).to_string();
    let (reading, outcome) = payload::measure(
        &mut dispatcher,
        Channel::Stdio,
        Some(&authorization),
        &notification,
    );
    assert!(
        outcome.response.is_none(),
        "notification 不得产生响应: {reading}"
    );
    assert_eq!(reading.request, notification.len(), "请求按字节计");
    assert_eq!(reading.response, 0, "notification 的响应字节数必须是 0");
    assert_eq!(reading.total(), notification.len());
    assert!(reading.within_baseline_006());
}

/// **判据 ⑥**：单位是 **UTF-8 字节**，不是字符数。
///
/// 测法：用一条**含非 ASCII** 的请求（章节名 `副歌`）跑真实调用，此时
/// `line.len() != line.chars().count()`，两种口径会给出不同的数。
/// 变红的方式：把 [`yeban_mcp::payload::measure`] 的请求计数改成 `chars().count()`
/// ⇒ `left: … / right: …`（字节数 > 字符数）。
#[test]
fn the_reading_counts_utf8_bytes_not_characters() {
    let (mut dispatcher, authorization) = dispatcher();
    let mut arguments = section_arguments(None);
    arguments["sectionName"] = Value::from("副歌");
    let line = call_line("yeban_propose_section", arguments);
    let (reading, outcome) =
        payload::measure(&mut dispatcher, Channel::Stdio, Some(&authorization), &line);

    assert!(
        line.chars().count() < line.len(),
        "这条请求必须真的含多字节字符, 否则本判据测不到东西"
    );
    assert_eq!(
        reading.request,
        line.as_bytes().len(),
        "请求计数必须是 UTF-8 字节数"
    );

    let response = outcome.response.as_ref().expect("响应").clone();
    let text = serde_json::to_string(&response).expect("序列化");
    assert!(
        text.chars().count() < text.len(),
        "响应里必须带着那个多字节章节名: {text}"
    );
    assert_eq!(
        reading.response,
        payload::response_wire_bytes(&response),
        "响应计数必须与序列化文本的字节数一致"
    );
}

// ---------------------------------------------------------------------------
// §注入证据（先红后还原；刻度说明见 `docs/DEVELOPMENT_LEDGER.md` 的纪律）
// ---------------------------------------------------------------------------
//
// 五条注入逐个施加在**生产代码**上，读回**字面**红行，再用 `cp` 还原并用
// `cmp` + sha256 证明逐字节相同。注入期写在源码里的临时标记注释（形如 `INJECT-I1`）
// **已随还原全部消失**：还原后 `grep -rn 'INJECT-I[1-5]' crates/` 只命中本段文档
// （即这两处提到标记名字的地方），源码与测试代码里一处都不剩。
// 逐次还原的 `cmp` 与 sha256 读数见下表最后一列。
//
// | # | 注入（生产代码） | 字面红行 | 还原证明 |
// | --- | :--- | :--- | :--- |
// | I1 | `domain::include_ops` 缺省 `false` → `true` | `缺省形状**不得**回传完整 op 载荷` @ 本文件:138；`缺省预览不得回传完整 op 载荷` @ :299；`缺省不得带 ops` @ :188；`3 passed; 3 failed` | `cmp` 相同 / sha256 `f23f2b8c…` |
// | I2 | `apply_propose` 忽略 `includeOps`（恒回 `summary`） | `includeOps=true 必须回传逐条 ops` @ :194；`5 passed; 1 failed` | `cmp` 相同 / sha256 `f23f2b8c…` |
// | I3 | `payload::measure` 的请求计数改用 `chars().count()` | `请求计数必须是 UTF-8 字节数` @ :371，`left: 192 / right: 196`；`5 passed; 1 failed` | `cmp` 相同 / sha256 `7b5b343f…` |
// | I4 | `RoundTripPayload::total` 去掉饱和加法 | `attempt to add with overflow` @ `src/payload.rs:81`；`4 passed; 1 failed` | `cmp` 相同 / sha256 `7b5b343f…` |
// | I5 | `BASELINE_006_JSON_LIMIT_BYTES` `4096` → `1024` | `16 小节段落生成的往返 JSON 必须 ≤ 1024 B, 实测 request 196 B + response 2747 B = 2943 B` @ :143 与 `dryRun 往返也必须 ≤ 1024 B, 实测 … 2918 B` @ :303；`4 passed; 2 failed`；lib 侧 `left: 1024 / right: 4096` | `cmp` 相同 / sha256 `7b5b343f…` |
//
// **I3 值得单独说**：只有它变红，其余五条判据全绿 —— 因为 BASELINE-006 的
// 场景载荷全是 ASCII，此时"字节数"与"字符数"相等。判据 ⑥ 用一条含 CJK 的请求
// 把这个区别变成可观测的，否则"单位是字节"这句话在判据里是**空的**。
