//! `tools/list` 分页与按名拉取（T-FIX-832）。
//!
//! ## 这个文件量什么、单位是什么
//!
//! - **对象**：`tools/list` 的**响应体**（`result`，不含 JSON-RPC 信封），即分发层
//!   `tools/list` 分支返回的 JSON；单位：UTF-8 **字节**（`serde_json::to_string`，
//!   无空白；token 估算用字节 / 4，与 `tools_list_budget.rs` 同口径，**近似**）；
//! - **改动前**（基线 c3065090，实测）：整表 `catalog()` 序列化
//!   **310,171 字节**（简报上报人 318,144 字节是在证据尖端 bb84a640 上量的，
//!   尖端已前进，差值来自后续改动；本单 before 以本尖端实测为准）；
//! - **改动后**：默认首响应（前 5 条）约 32,220 字节，约改动前的 **10%**；
//!   整表仍可翻页取全（10 页），单工具可按名取全 schema。
//!
//! ## 为什么页大小是 5
//!
//! `yeban_edit_notes` 一个描述符就 141,496 字节（占整表 46%），且在表里排第 6
//! （下标 5）：10 条一页的首响应仍有 215,689 字节，5 条一页的首响应只有
//! 32,179 字节。页大小见 `tools::TOOLS_LIST_PAGE_SIZE`。
//!
//! ## 红对照（规则 30）
//!
//! 任一条把 `tools_list` 改回整表返回（或把 `TOOLS_LIST_PAGE_SIZE` 改大）⇒
//! 首响应判据变红；把游标校验删掉 ⇒ 恶形游标判据变红；把 `name` 分支删掉 ⇒
//! 按名拉取判据变红。复现：`cargo test -p yeban-mcp --test tools_list_pagination -- --nocapture`。

use serde_json::{Value, json};

use yeban_mcp::dispatch::{Dispatcher, Outcome};
use yeban_mcp::jsonrpc::INVALID_PARAMS;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{self, TOOL_COUNT, TOOLS_LIST_PAGE_SIZE};

// ---------------------------------------------------------------------------
// 脚手架（与 `tests/compare_song_snapshots.rs` 同款，不跨测试文件引用）
// ---------------------------------------------------------------------------

fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    (dispatcher, auth)
}

/// 一次 `tools/list` 调用，返回原始 [`Outcome`]（成功与失败都不断言，调用方自判）。
fn list_raw(dispatcher: &mut Dispatcher, auth: &str, params: Option<Value>) -> Outcome {
    let mut line = json!({
        "jsonrpc": "2.0",
        "id": "r",
        "method": "tools/list",
    });
    if let Some(params) = params {
        line["params"] = params;
    }
    dispatcher.handle_line(Channel::Http, Some(auth), &line.to_string())
}

/// 一次 `tools/list` 成功调用，返回 `result`。
fn list_ok(dispatcher: &mut Dispatcher, auth: &str, params: Option<Value>) -> Value {
    let outcome = list_raw(dispatcher, auth, params);
    assert_eq!(
        outcome.error_code(),
        None,
        "tools/list 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    outcome.response.expect("响应").result.expect("result")
}

/// 断言一次 `tools/list` 调用以 `-32602` 响亮拒绝，且 `data.detail` 点名原因。
fn assert_invalid_params(outcome: Outcome, needle: &str) {
    assert_eq!(outcome.http_status, 400, "非法参数必须走 HTTP 400");
    assert_eq!(
        outcome.error_code(),
        Some(INVALID_PARAMS),
        "非法参数必须是 -32602"
    );
    let detail = outcome
        .response
        .expect("响应")
        .error_object()
        .expect("错误对象")
        .data
        .clone()
        .expect("错误必须带 data")
        .get("detail")
        .and_then(Value::as_str)
        .expect("data.detail 必须是字符串")
        .to_owned();
    assert!(
        detail.contains(needle),
        "错误 detail 必须点名 `{needle}`，实际：{detail}"
    );
}

/// 整表（改动前行为的参照）：`catalog()` 序列化字节数。
fn full_catalog_bytes() -> usize {
    serde_json::to_string(&tools::catalog())
        .expect("清单必须可序列化")
        .len()
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// 默认（无参 / `{}` / `cursor: null`）只回第一页 + `nextCursor`，且首响应
/// 远小于整表（改动前 310,171 字节 → 改动后约 32,220 字节）。
#[test]
fn default_tools_list_returns_first_page_with_next_cursor() {
    let (mut dispatcher, auth) = dispatcher();
    let full_bytes = full_catalog_bytes();
    let full_tools = tools::catalog()["tools"]
        .as_array()
        .expect("tools 是数组")
        .clone();

    // 三种"缺省"写法必须同形：无 params 键、`{}`、`{"cursor": null}`。
    let mut first_pages = Vec::new();
    first_pages.push(list_ok(&mut dispatcher, &auth, None));
    first_pages.push(list_ok(&mut dispatcher, &auth, Some(json!({}))));
    first_pages.push(list_ok(
        &mut dispatcher,
        &auth,
        Some(json!({"cursor": null})),
    ));
    for (i, page) in first_pages.iter().enumerate() {
        let page_tools = page["tools"].as_array().expect("tools 是数组");
        assert_eq!(
            page_tools.len(),
            TOOLS_LIST_PAGE_SIZE,
            "第 {i} 种缺省写法：首响应必须是恰好一页（{TOOLS_LIST_PAGE_SIZE} 条）"
        );
        assert_eq!(
            *page_tools,
            full_tools[..TOOLS_LIST_PAGE_SIZE],
            "第 {i} 种缺省写法：首响应必须是整表前 {TOOLS_LIST_PAGE_SIZE} 条，顺序一致"
        );
        assert_eq!(
            page["nextCursor"], "tools-list:5",
            "第 {i} 种缺省写法：首响应必须带 nextCursor 指向下一页"
        );
    }

    // --- 字节读数（eprintln：--nocapture 下可见，回执的 before/after 即取自这里）---
    let first_bytes = serde_json::to_string(&first_pages[0])
        .expect("首响应必须可序列化")
        .len();
    eprintln!("### tools/list 分页字节读数");
    eprintln!("before(full catalog)={full_bytes} bytes | after(first page)={first_bytes} bytes");
    eprintln!(
        "first page tools={} | nextCursor={}",
        first_pages[0]["tools"].as_array().unwrap().len(),
        first_pages[0]["nextCursor"]
    );
    assert!(
        first_bytes <= 40_000,
        "首响应 {first_bytes} 字节超过 40,000 上限（实测约 32,220）：页太大就失去分页意义"
    );
    assert!(
        first_bytes * 5 < full_bytes,
        "首响应 {first_bytes} 字节必须不到整表 {full_bytes} 字节的五分之一"
    );
}

/// 按 `nextCursor` 翻页：10 页拼回的整表与 `catalog()` 逐字节相同
///（顺序、内容全等；每页 ≤ 5 条；最后一页 1 条且无 `nextCursor`）。
#[test]
fn paging_through_all_pages_reconstructs_the_full_catalog() {
    let (mut dispatcher, auth) = dispatcher();
    let full_tools = tools::catalog()["tools"]
        .as_array()
        .expect("tools 是数组")
        .clone();

    let mut collected: Vec<Value> = Vec::new();
    let mut pages = 0usize;
    let mut cursor: Option<Value> = None;
    loop {
        let params = match &cursor {
            None => json!({}),
            Some(cursor) => json!({"cursor": cursor}),
        };
        let page = list_ok(&mut dispatcher, &auth, Some(params));
        let page_tools = page["tools"].as_array().expect("tools 是数组");
        assert!(
            !page_tools.is_empty() && page_tools.len() <= TOOLS_LIST_PAGE_SIZE,
            "每页必须 1..={TOOLS_LIST_PAGE_SIZE} 条，实际 {} 条",
            page_tools.len()
        );
        collected.extend(page_tools.iter().cloned());
        pages += 1;
        match page.get("nextCursor") {
            Some(next) => cursor = Some(next.clone()),
            None => break,
        }
        assert!(pages <= TOOL_COUNT, "翻页不得死循环（页数已超工具数）");
    }
    assert_eq!(pages, 10, "46 个工具、每页 5 条，必须恰好 10 页");
    assert_eq!(
        collected.len(),
        TOOL_COUNT,
        "翻页收齐的工具数必须等于 TOOL_COUNT"
    );
    assert_eq!(
        collected, full_tools,
        "翻页拼回的整表必须与 catalog() 逐字节相同（顺序、内容全等）"
    );
}

/// 按名拉取：`{"name": …}` 只回该工具的完整描述符（与 `catalog()` 里那一条
/// 逐字节相同）；未知名与 `name` + `cursor` 同给 ⇒ `-32602` 响亮拒绝。
#[test]
fn tools_list_by_name_returns_exactly_that_tool() {
    let (mut dispatcher, auth) = dispatcher();
    let full_tools = tools::catalog()["tools"]
        .as_array()
        .expect("tools 是数组")
        .clone();

    // 巨型工具与小工具各取一次：都必须是完整描述符，不带 nextCursor。
    for name in ["yeban_edit_notes", "yeban_close_project"] {
        let result = list_ok(&mut dispatcher, &auth, Some(json!({"name": name})));
        let got = result["tools"].as_array().expect("tools 是数组");
        assert_eq!(got.len(), 1, "按名拉取必须恰好回 1 条");
        let want = full_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("整表里必须有该工具");
        assert_eq!(
            &got[0], want,
            "按名拉取的描述符必须与 catalog() 里那一条逐字节相同"
        );
        assert!(
            result.get("nextCursor").is_none(),
            "按名拉取不得带 nextCursor"
        );
    }

    // 未知工具名 ⇒ -32602 且点名。
    let outcome = list_raw(
        &mut dispatcher,
        &auth,
        Some(json!({"name": "yeban_no_such_tool"})),
    );
    assert_invalid_params(outcome, "yeban_no_such_tool");

    // name 与 cursor 同给 ⇒ -32602。
    let outcome = list_raw(
        &mut dispatcher,
        &auth,
        Some(json!({"name": "yeban_edit_notes", "cursor": "tools-list:5"})),
    );
    assert_invalid_params(outcome, "不可同传");
}

/// 恶形 `cursor` / `params` 一律 `-32602` 响亮拒绝：伪造前缀、非页边界、
/// 越界、非数字、非字符串、数组 params、未知键。
#[test]
fn tools_list_rejects_malformed_cursors_and_params_loudly() {
    let (mut dispatcher, auth) = dispatcher();
    let cases: Vec<(Value, &str)> = vec![
        (json!({"cursor": "bogus"}), "非法"),
        (json!({"cursor": "tools-list:7"}), "页边界"),
        (json!({"cursor": "tools-list:46"}), "越界"),
        (json!({"cursor": "tools-list:-1"}), "非法"),
        (json!({"cursor": "tools-list:abc"}), "非法"),
        (json!({"cursor": 123}), "字符串"),
        (json!(["tools-list:5"]), "对象"),
        (json!({"pad": "x"}), "不认识参数"),
        (json!({"name": 123}), "字符串"),
        (json!({"name": ""}), "未知工具名"),
    ];
    for (params, needle) in cases {
        let outcome = list_raw(&mut dispatcher, &auth, Some(params.clone()));
        assert_invalid_params(outcome, needle);
    }
}

/// `initialize` / `tools/call` 语义不变：握手照旧回协议版本与服务信息；
/// 一次 `tools/call` 照旧路由执行（无活跃工程时带内 `NO_ACTIVE_PROJECT`，
/// 不是协议层错误）。
#[test]
fn initialize_and_tools_call_semantics_unchanged() {
    let (mut dispatcher, auth) = dispatcher();

    // initialize：协议版本 + 能力 + 服务信息一个不少。
    let line = json!({
        "jsonrpc": "2.0",
        "id": "init",
        "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(outcome.error_code(), None, "initialize 不得产出协议层错误");
    let result = outcome.response.expect("响应").result.expect("result");
    assert_eq!(result["serverInfo"]["name"], "yeban-mcp");
    assert!(
        result["capabilities"]["tools"].is_object(),
        "initialize 的 capabilities.tools 必须在"
    );
    assert!(
        result["protocolVersion"].is_string(),
        "initialize 必须回协议版本"
    );

    // tools/call：照旧路由执行（无活跃工程 ⇒ 带内失败，不是协议层错误）。
    let line = json!({
        "jsonrpc": "2.0",
        "id": "call",
        "method": "tools/call",
        "params": {"name": "yeban_query_project", "arguments": {}},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(outcome.http_status, 200);
    assert_eq!(
        outcome.error_code(),
        None,
        "tools/call 不得产出协议层错误: {outcome:?}"
    );
}
