//! 跨语言契约对账（**承重**的那一半）[MCP-TOOL-001..010, MUST-GATE-010, TEST-SPEC-005]。
//!
//! `scripts/gates/validate_schemas.py --samples-dir` 把 Rust 写出的样本交给
//! Python `jsonschema`，那是跨实现交叉验证。**但对 `schemas/mcp-tools.schema.json`
//! 而言那条对账目前是空转的**：该 schema 的根只有 `{"type":"object","definitions":{…}}`，
//! 没有 `properties` / `$ref` / `allOf` —— Draft 2020-12 下"任意对象"都能过根校验，
//! `definitions.ToolCall` 从未被引用（`--samples-dir` 只能证明"我们写出了一个对象"）。
//!
//! 所以真正承重的判据在这里：**直接读契约文件里的 enum 做集合相等断言**。
//! 新增或漏掉一个工具、把 `dryRun` / `idempotencyKey` 从 `arguments` 里删掉、
//! 或者有人静悄悄改小错误码 enum，这几条都会红。
//!
//! 本文件同时把"契约缺口"（13 个表格错误码不在 schema enum 里）钉成**实测常量**，
//! 于是它要么保持原样，要么有人必须来解释为什么变了。

use std::path::PathBuf;

use serde_json::Value;
use yeban_mcp::tools::{self, ErrorCode};

/// `schemas/mcp-tools.schema.json` 的路径（`<crate>/../../schemas/...`）。
fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schemas")
        .join("mcp-tools.schema.json")
}

/// 读契约文件。
fn contract() -> Value {
    let text = std::fs::read_to_string(schema_path()).expect("读 schemas/mcp-tools.schema.json");
    serde_json::from_str(&text).expect("契约必须是合法 JSON")
}

/// `definitions.ToolCall.properties.name.enum`（契约的工具名枚举）。
fn contract_tool_names() -> Vec<String> {
    contract()["definitions"]["ToolCall"]["properties"]["name"]["enum"]
        .as_array()
        .expect("definitions.ToolCall.properties.name.enum 必须是数组")
        .iter()
        .map(|value| value.as_str().expect("enum 元素是字符串").to_owned())
        .collect()
}

/// `definitions.ToolCall.properties.arguments.properties` 的键集合。
fn contract_argument_keys() -> Vec<String> {
    contract()["definitions"]["ToolCall"]["properties"]["arguments"]["properties"]
        .as_object()
        .expect("arguments.properties 必须是对象")
        .keys()
        .cloned()
        .collect()
}

/// `definitions.ToolResponse.properties.error.properties.code.enum`。
fn contract_error_codes() -> Vec<String> {
    contract()["definitions"]["ToolResponse"]["properties"]["error"]["properties"]["code"]["enum"]
        .as_array()
        .expect("ToolResponse.error.code.enum 必须是数组")
        .iter()
        .map(|value| value.as_str().expect("enum 元素是字符串").to_owned())
        .collect()
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

// ---------------------------------------------------------------------------
// 判据 1..3: 工具名集合与契约枚举**完全一致**
// ---------------------------------------------------------------------------

#[test]
fn tool_name_count_equals_the_contract_enum_length() {
    let contract = contract_tool_names();
    assert_eq!(
        contract.len(),
        tools::TOOL_COUNT,
        "契约枚举有 {} 个工具, 注册表有 {} 个",
        contract.len(),
        tools::TOOL_COUNT
    );
    assert_eq!(contract.len(), 10, "MCP-TOOL-001..010 是十个工具");
    assert_eq!(tools::TOOLS.len(), contract.len());
}

#[test]
fn every_registered_tool_name_is_in_the_contract_enum() {
    let contract = contract_tool_names();
    for spec in &tools::TOOLS {
        assert!(
            contract.contains(&spec.name.to_owned()),
            "注册表里的 `{}` ({}) 不在契约 enum 中",
            spec.name,
            spec.spec_id
        );
    }
}

#[test]
fn every_contract_enum_name_is_registered_in_the_registry() {
    let registered: Vec<String> = tools::tool_names().into_iter().map(str::to_owned).collect();
    for name in contract_tool_names() {
        assert!(
            registered.contains(&name),
            "契约里的 `{name}` 没有在注册表中实现"
        );
    }
}

#[test]
fn tool_name_sets_are_equal_and_in_the_same_order() {
    // 最强的一条: 双向包含 + 顺序一致 (顺序决定 tools/list 的逐字节输出)。
    let expected = contract_tool_names();
    let actual: Vec<String> = tools::tool_names().into_iter().map(str::to_owned).collect();
    assert_eq!(
        sorted(expected.clone()),
        sorted(actual.clone()),
        "工具名集合必须完全相等"
    );
    assert_eq!(expected, actual, "注册表顺序必须与契约 enum 顺序一致");
}

// ---------------------------------------------------------------------------
// 判据 4: dryRun / idempotencyKey
// ---------------------------------------------------------------------------

#[test]
fn contract_declares_dry_run_and_idempotency_key_on_arguments() {
    let keys = contract_argument_keys();
    for expected in [tools::DRY_RUN_PARAM, tools::IDEMPOTENCY_KEY_PARAM] {
        assert!(
            keys.contains(&expected.to_owned()),
            "契约的 arguments.properties 必须声明 `{expected}`, 实际 {keys:?}"
        );
    }
    // 且每个工具都真的提供了它们 (不是只写在 schema 里)。
    for spec in &tools::TOOLS {
        for expected in [tools::DRY_RUN_PARAM, tools::IDEMPOTENCY_KEY_PARAM] {
            assert!(
                spec.param(expected).is_some(),
                "{} 没有实现契约要求的 `{expected}`",
                spec.name
            );
        }
    }
}

#[test]
fn dry_run_defaults_to_false_in_the_contract_and_in_the_registry() {
    assert_eq!(
        contract()["definitions"]["ToolCall"]["properties"]["arguments"]["properties"]
            [tools::DRY_RUN_PARAM]["default"],
        Value::from(false),
        "契约把 dryRun 的默认值钉成 false"
    );
    // 注册表侧: 不带 dryRun 的调用必须被当成非 dryRun (fail-closed 到"真的执行")。
    let call = tools::ToolCall::from_params(Some(&serde_json::json!({
        "name": "yeban_save_project",
        "arguments": {}
    })))
    .expect("合法调用");
    assert!(!call.is_dry_run());
}

// ---------------------------------------------------------------------------
// 判据 5: 错误码
// ---------------------------------------------------------------------------

#[test]
fn every_contract_error_code_is_implemented() {
    for name in contract_error_codes() {
        let found = ErrorCode::ALL
            .iter()
            .any(|code| code.as_str() == name.as_str());
        assert!(found, "契约错误码 `{name}` 没有被 ErrorCode 覆盖");
    }
}

#[test]
fn the_error_code_gap_between_the_two_contracts_is_pinned() {
    let contract = contract_error_codes();
    assert_eq!(contract.len(), 7, "schema enum 是 7 个错误码");

    let documented: Vec<String> = ErrorCode::DOCUMENTED_TOOL_CODES
        .iter()
        .map(|code| code.as_str().to_owned())
        .collect();
    assert_eq!(documented.len(), 16, "架构 §7.2 表格是 16 个错误码");

    let missing: Vec<&str> = documented
        .iter()
        .filter(|name| !contract.contains(*name))
        .map(String::as_str)
        .collect();
    assert_eq!(
        missing,
        vec![
            "FILE_NOT_FOUND",
            "DISK_FULL",
            "NO_ACTIVE_PROJECT",
            "INVALID_FIELD_SELECTOR",
            "STYLE_NOT_FOUND",
            "CYCLE_DETECTED",
            "CLIP_NOT_FOUND",
            "OUT_OF_RANGE",
            "TRACK_NOT_FOUND",
            "INDEX_OUT_OF_BOUNDS",
            "RENDER_FAILED",
            "BUSY",
            "CONFLICT",
        ],
        "schema 装不下的 13 个错误码是**实测**的; 变化需要有人解释 (见 notes §3.1)"
    );
    // 两个契约共有的 3 个。
    let shared: Vec<&str> = documented
        .iter()
        .filter(|name| contract.contains(*name))
        .map(String::as_str)
        .collect();
    assert_eq!(
        shared,
        vec!["PROJECT_LOCKED", "IO_ERROR", "PROPOSAL_NOT_FOUND"]
    );
}

// ---------------------------------------------------------------------------
// 判据 6: 样本与契约逐份对账 (Rust 侧)
// ---------------------------------------------------------------------------

#[test]
fn exported_call_samples_are_contract_shaped_tool_calls() {
    let dir = std::env::temp_dir().join(format!("yeban-mcp-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let written = yeban_mcp::samples::export_all(&dir).expect("导出样本");
    let contract_names = contract_tool_names();
    let argument_keys = contract_argument_keys();

    let mut call_samples = 0;
    for path in &written {
        let name = path
            .file_name()
            .expect("文件名")
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(yeban_mcp::samples::FILE_PREFIX),
            "样本前缀必须映射到 mcp-tools.schema.json: {name}"
        );
        let text = std::fs::read_to_string(path).expect("读样本");
        let value: Value = serde_json::from_str(&text).expect("样本必须是 JSON");
        assert!(value.is_object(), "schema 的根要求 object: {name}");
        if !name.starts_with(yeban_mcp::samples::CALL_FILE_PREFIX) {
            continue;
        }
        call_samples += 1;
        let tool = value["name"].as_str().expect("ToolCall.name");
        assert!(
            contract_names.contains(&tool.to_owned()),
            "样本 `{name}` 的工具名 `{tool}` 不在契约 enum 中"
        );
        for key in &argument_keys {
            assert!(
                value["arguments"].get(key).is_some(),
                "样本 `{name}` 的 arguments 缺少契约键 `{key}`"
            );
        }
    }
    assert_eq!(
        call_samples,
        tools::TOOL_COUNT,
        "每个工具一份 ToolCall 样本"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn schema_root_references_the_tool_call_definition() {
    // 现状: 根没有 $ref / properties / allOf, 因此 --samples-dir 对账对本文件是空转的。
    // 契约被修好时这条会**变红**, 提醒把样本对账升级成真判据 (并更新 notes §3.2)。
    let root = contract();
    let has_reference = root.get("$ref").is_some()
        || root.get("properties").is_some()
        || root.get("allOf").is_some()
        || root.get("oneOf").is_some()
        || root.get("anyOf").is_some();
    assert!(
        !has_reference,
        "schemas/mcp-tools.schema.json 的根开始引用 definitions 了 —— \
         请把 scripts/gates 的 --samples-dir 对账接进 CI, 并更新 docs/ledger/mcp-core-notes.md §3.2"
    );
}

// ---------------------------------------------------------------------------
// 判据 7: 红线自身的机器化自查
// ---------------------------------------------------------------------------

#[test]
fn manifest_default_features_do_not_enable_mcp_http() {
    let manifest =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("读 Cargo.toml");
    let mut in_features = false;
    let mut default_line: Option<String> = None;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && trimmed.starts_with("default") {
            default_line = Some(trimmed.to_owned());
        }
    }
    let default_line = default_line.expect("[features] 必须显式声明 default");
    assert!(
        !default_line.contains("mcp-http"),
        "红线 6 [MUST-GATE-009]: `mcp-http` 绝不能进 default, 实际: {default_line}"
    );
    assert!(
        default_line.replace(' ', "").starts_with("default=[]"),
        "default 必须是空集: {default_line}"
    );
    assert!(manifest.contains("mcp-http = []"), "feature 必须显式登记");
    assert!(
        !manifest.contains("tiny_http"),
        "本线坚持零新增依赖: HTTP 传输是手写的 (见 src/transport/http.rs 的说明)"
    );
}

#[test]
fn crate_sources_never_contain_the_unspecified_bind_address() {
    // 守卫 G04 [ARCH-SEC-002] 已经在仓库级别查这件事; 这里是 crate 级的自查,
    // 让"这条红线"在本 crate 的判据列表里也可见 (由 concat! 拼出字面量, 免得自己犯规)。
    let forbidden = concat!("0.0.", "0.0");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut visited = 0_usize;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("读目录") {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                continue;
            }
            visited += 1;
            let text = std::fs::read_to_string(&path).expect("读源文件");
            for (lineno, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                assert!(
                    !line.contains(forbidden),
                    "{}:{} 出现禁止的监听字面量",
                    path.display(),
                    lineno + 1
                );
            }
        }
    }
    assert!(visited >= 7, "应当扫描到本 crate 的源文件, 实际 {visited}");
}

#[test]
fn only_the_loopback_and_dynamic_port_are_used_for_binding() {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/transport/http.rs"),
    )
    .expect("读 http.rs");
    assert!(
        text.contains("SocketAddr::new(IpAddr::V4(LOOPBACK_IPV4), DYNAMIC_PORT)"),
        "绑定地址必须是 `Ipv4Addr::LOCALHOST` + 动态端口 0"
    );
    assert_eq!(
        text.matches("TcpListener::bind(").count(),
        1,
        "整个 crate 只允许一处 bind —— 多一处就意味着多一个没被审过的监听面"
    );
    assert!(
        text.contains("assert_loopback(server.local_addr()?)"),
        "绑定之后必须**回读** local_addr() 并断言它是环回, 而不是相信自己的入参"
    );
}
