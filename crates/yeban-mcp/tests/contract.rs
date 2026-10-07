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
    // 10 = 规范表格的 MCP-TOOL-001..010；+EXTENSION_TOOL_COUNT = ADR-0001 D45/D46 的扩展
    // （yeban_undo / yeban_redo + D46 的三类能力）。
    assert_eq!(
        contract.len(),
        tools::DOCUMENTED_TOOL_COUNT + tools::EXTENSION_TOOL_COUNT,
        "规范十个工具 + D45/D46 的扩展"
    );
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
fn implementation_error_codes_equal_the_contract_enum_exactly() {
    // ADR-0001 D25 把 ToolResponse.error.code 从 7 值扩成**联集 20 值**。
    // 第一轮这条判据是"钉住 13 个码的缺口"; 缺口关闭后它升级成**集合相等**:
    // 少一个 (契约新增而实现没跟上) 或多一个 (实现自创了契约里没有的码) 都会红。
    let contract = sorted(contract_error_codes());
    let implemented = sorted(
        ErrorCode::SCHEMA_CONTRACT
            .iter()
            .map(|code| code.as_str().to_owned())
            .collect(),
    );
    assert_eq!(contract.len(), 20, "联集 20 值");
    assert_eq!(
        implemented, contract,
        "ErrorCode::SCHEMA_CONTRACT 必须与契约 enum 集合完全相等"
    );
    assert_eq!(
        ErrorCode::SCHEMA_ONLY.len(),
        4,
        "schema 原有而表格未列的 4 个"
    );
    assert_eq!(ErrorCode::DOCUMENTED_TOOL_CODES.len(), 16, "架构 §7.2 表格");

    // 表格的 16 个全部落在契约里 (缺口已关闭)。
    for code in ErrorCode::DOCUMENTED_TOOL_CODES {
        assert!(
            code.is_schema_contract(),
            "表格错误码 {code} 不在契约 enum 里 —— 缺口回来了"
        );
    }
    // 契约减去表格 == 那 4 个 schema 原有码。
    let schema_only: Vec<String> = contract
        .iter()
        .filter(|name| {
            !ErrorCode::DOCUMENTED_TOOL_CODES
                .iter()
                .any(|code| code.as_str() == name.as_str())
        })
        .cloned()
        .collect();
    assert_eq!(
        schema_only,
        vec![
            "ENTITY_NOT_FOUND".to_owned(),
            "INVALID_PARAMETER_RANGE".to_owned(),
            "PERMISSION_DENIED".to_owned(),
            "ROUTING_CYCLE_DETECTED".to_owned(),
        ]
    );
    // 实现级码 (NOT_IMPLEMENTED) 绝不能被塞进契约路径。
    assert!(
        !contract.contains(&ErrorCode::NotImplemented.as_str().to_owned()),
        "NOT_IMPLEMENTED 是实现级码, 不许进 ToolResponse.error.code"
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
    let mut response_samples = 0;
    let mut meta_samples = 0;
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

        let looks_like_call = value.get("name").is_some() || value.get("arguments").is_some();
        let looks_like_response = value.get("status").is_some();
        let looks_like_replay = value.get("replayed").is_some() && value.get("response").is_some();

        if name.contains(".meta.") {
            // 文档样本: 顶层是清单/快照, **不许**长得像契约实例 (否则就是在用 .meta. 藏实例)。
            assert!(
                !looks_like_call && !looks_like_response && !looks_like_replay,
                "文档样本 `{name}` 顶层出现了契约实例的判别键"
            );
            meta_samples += 1;
            continue;
        }

        // 根的 oneOf 语义: 契约实例必须**恰好**是三种形状之一, 不能两边都像。
        let matched = [looks_like_call, looks_like_response, looks_like_replay]
            .iter()
            .filter(|flag| **flag)
            .count();
        assert_eq!(
            matched, 1,
            "样本 `{name}` 必须恰好是 ToolCall / ToolResponse / ReplayedToolResponse 之一"
        );

        if name.starts_with(yeban_mcp::samples::CALL_FILE_PREFIX) {
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
        } else if name == yeban_mcp::samples::RESPONSE_REPLAYED_FILE {
            response_samples += 1;
            assert_eq!(
                value["replayed"], true,
                "重放样本的 `replayed` 必须恰好是 true"
            );
            assert_eq!(value["response"]["jsonrpc"], "2.0");
            assert!(
                value["response"]["result"]["status"].is_string(),
                "重放样本的内层 result 必须是 ToolResponse"
            );
        } else {
            response_samples += 1;
            let status = value["status"].as_str().expect("ToolResponse.status");
            assert!(
                ["success", "error"].contains(&status),
                "样本 `{name}` 的 status `{status}` 不在契约 enum 中"
            );
            assert!(
                value["data"].is_object(),
                "样本 `{name}` 的 data 必须是对象 (契约要求)"
            );
        }
    }
    assert_eq!(
        call_samples,
        tools::TOOL_COUNT,
        "每个工具一份 ToolCall 契约实例"
    );
    assert_eq!(
        response_samples, 2,
        "根 oneOf 的第二、第三个分支各须至少一份真实例覆盖"
    );
    assert_eq!(meta_samples, 2, "注册表与错误码目录各一份 .meta. 文档样本");
    std::fs::remove_dir_all(&dir).ok();
}

/// 实例文件名 ↔ 契约实例集合的**双射**校验。
///
/// `.meta.` 是"不对账 schema"的**命名约定**, 不是 schema 能力 —— 谁都能把一份本该对账的
/// 实例改名成 `.meta.` 来逃逸。脚本侧的守卫只挡得住"整段逃逸"(每个前缀至少一份真实例);
/// 这条纯函数挡住"部分逃逸": 非 meta 的实例集合必须**恰好**是
/// `{mcp-tools.call.<tool>.json | tool ∈ 契约工具集} ∪ {mcp-tools.response.dry-run.json,
/// mcp-tools.response.replayed.json}`。
fn check_instance_set(names: &[String]) -> Result<(), String> {
    let mut expected: Vec<String> = tools::TOOLS
        .iter()
        .map(|spec| yeban_mcp::samples::call_file(spec.name))
        .collect();
    expected.push(yeban_mcp::samples::RESPONSE_DRY_RUN_FILE.to_owned());
    expected.push(yeban_mcp::samples::RESPONSE_REPLAYED_FILE.to_owned());

    let mut instances: Vec<String> = names
        .iter()
        .filter(|name| !name.contains(".meta."))
        .cloned()
        .collect();
    instances.sort();
    expected.sort();
    if instances != expected {
        return Err(format!(
            "契约实例集合与工具集合不是双射: 实例={instances:?}, 期望={expected:?}"
        ));
    }
    Ok(())
}

#[test]
fn exported_instance_set_is_exactly_the_tool_set() {
    let dir = std::env::temp_dir().join(format!("yeban-mcp-bij-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let written = yeban_mcp::samples::export_all(&dir).expect("导出样本");
    let names: Vec<String> = written
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    check_instance_set(&names).expect("非 meta 的实例集合必须穷举且与工具集合一一对应");

    // 文档样本集合也穷举 (多一份 .meta. 可能是逃逸的前奏)。
    let mut meta: Vec<String> = names
        .iter()
        .filter(|name| name.contains(".meta."))
        .cloned()
        .collect();
    meta.sort();
    assert_eq!(
        meta,
        vec![
            yeban_mcp::samples::ERROR_CODES_FILE.to_owned(),
            yeban_mcp::samples::REGISTRY_FILE.to_owned(),
        ],
        "文档样本只允许这两份"
    );

    // 反例 1: 把一份实例改名成 .meta. —— 脚本侧的"每个前缀至少一份实例"守卫**挡不住**
    // 这种部分逃逸(还剩 10 份实例)。这正是本判据存在的理由。
    let victim = names
        .iter()
        .position(|name| name.starts_with(yeban_mcp::samples::CALL_FILE_PREFIX))
        .expect("至少有一份 ToolCall 实例");
    let mut escaped = names.clone();
    escaped[victim] = escaped[victim].replace(".json", ".meta.json");
    let error = check_instance_set(&escaped).expect_err("改名逃逸必须被双射守卫抓到");
    assert!(error.contains("双射"), "{error}");

    // 反例 2: 少一份实例。
    let mut missing = names.clone();
    missing.remove(victim);
    assert!(check_instance_set(&missing).is_err());

    // 反例 3: 多一份"契约里没有的工具"的实例。
    let mut extra = names.clone();
    extra.push("mcp-tools.call.yeban_not_a_tool.json".to_owned());
    assert!(check_instance_set(&extra).is_err());

    // 反例 4: 把 ToolResponse 实例删掉 ⇒ 根 oneOf 的第二个分支就没人覆盖了。
    let mut no_response = names.clone();
    no_response.retain(|name| name != yeban_mcp::samples::RESPONSE_DRY_RUN_FILE);
    assert!(check_instance_set(&no_response).is_err());

    // 反例 5: 把 ReplayedToolResponse 实例删掉 ⇒ 第三个分支就没人覆盖了。
    let mut no_replay = names.clone();
    no_replay.retain(|name| name != yeban_mcp::samples::RESPONSE_REPLAYED_FILE);
    assert!(check_instance_set(&no_replay).is_err());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn contract_root_is_one_of_tool_call_and_tool_response() {
    // ADR-0001 D25: 根从"空对象"改成 oneOf($ref ToolCall, $ref ToolResponse)。
    // 之后新增第三个分支 $ref ReplayedToolResponse —— 幂等重放的 `result` 不是裸
    // `ToolResponse`, 把信封放在契约外会让会校验 schema 的客户端拒收重放响应。
    // 这条判据钉住"契约引用 definitions"这个**结构事实**;
    // 下一条判据钉住"这个引用真的在判定"。
    let root = contract();
    let branches = root["oneOf"]
        .as_array()
        .expect("根的 oneOf 必须是数组 (ADR-0001 D25)");
    assert_eq!(
        branches.len(),
        3,
        "oneOf(ToolCall, ToolResponse, ReplayedToolResponse)"
    );
    let refs: Vec<&str> = branches
        .iter()
        .map(|branch| branch["$ref"].as_str().expect("每个分支必须是 $ref"))
        .collect();
    assert!(refs.contains(&"#/definitions/ToolCall"));
    assert!(refs.contains(&"#/definitions/ToolResponse"));
    assert!(refs.contains(&"#/definitions/ReplayedToolResponse"));
    for key in ["properties", "allOf", "anyOf"] {
        assert!(
            root.get(key).is_none(),
            "根不应该再有 `{key}` —— 契约的判定入口应当只有 oneOf"
        );
    }

    // 信封的判据口径必须与实现一致: `replayed` 恒 true, `response` 是完整 JSON-RPC 响应。
    let envelope = &root["definitions"]["ReplayedToolResponse"];
    assert_eq!(
        envelope["required"],
        serde_json::json!(["replayed", "response"]),
        "信封必填 `replayed` + `response`"
    );
    assert_eq!(envelope["properties"]["replayed"]["const"], true);
    assert_eq!(
        envelope["properties"]["response"]["$ref"],
        "#/definitions/JsonRpcResponse"
    );
    let jsonrpc = &root["definitions"]["JsonRpcResponse"];
    assert_eq!(jsonrpc["properties"]["jsonrpc"]["const"], "2.0");
    assert_eq!(
        jsonrpc["properties"]["result"]["$ref"], "#/definitions/ToolResponse",
        "信封内层的 result 是 ToolResponse, 不是 JSON-RPC 响应"
    );
    assert_eq!(
        jsonrpc["properties"]["error"]["$ref"],
        "#/definitions/JsonRpcError"
    );
}

#[test]
fn contract_rejects_a_deliberately_invalid_sample() {
    // 把"契约承重"这件事本身钉成判据: 故意违法的样本必须让 --samples-dir 变红。
    //
    // 这条判据需要 python3 + jsonschema。CI 的 **rust 矩阵腿**不装 jsonschema
    // (只有 checks job 装), 所以缺依赖时打印**响亮的 SKIP** 而不是伪造绿;
    // 真正的跨语言对账在 checks job 里跑 (见 docs/ledger/mcp-core-notes.md §4.2)。
    if !python_jsonschema_available() {
        eprintln!(
            "SKIP[contract_rejects_a_deliberately_invalid_sample]: python3/jsonschema 不可用。\
             这条判据的**真跑**在 CI 的 checks job (那里装了 jsonschema 且已接线 export_mcp_samples)。"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("yeban-mcp-live-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    yeban_mcp::samples::export_all(&dir).expect("导出样本");

    // (1) 全部合法样本必须**全部通过**根校验（每个工具一份 ToolCall + 一份 ToolResponse）。
    let clean = run_validate_schemas(&dir);
    assert!(
        clean.status.success(),
        "合法样本必须全部通过根校验。stdout={}\nstderr={}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );

    // (2) 混进一份**故意违法**的样本之后必须变红。
    let bogus = dir.join("mcp-tools.bogus.json");
    std::fs::write(&bogus, "{\"anything\": [1, 2, 3]}\n").expect("写违法样本");
    let dirty = run_validate_schemas(&dir);
    assert!(
        !dirty.status.success(),
        "故意违法的样本必须让契约变红 —— 否则这条对账是空转的。stdout={}",
        String::from_utf8_lossy(&dirty.stdout)
    );
    assert!(
        String::from_utf8_lossy(&dirty.stderr).contains("mcp-tools.bogus.json"),
        "变红的原因必须指向那份样本: {}",
        String::from_utf8_lossy(&dirty.stderr)
    );

    // (3) "工具名不在 enum 里"的 ToolCall 也必须被拒。
    std::fs::remove_file(&bogus).ok();
    let bad_tool = dir.join("mcp-tools.call.not_a_tool.json");
    std::fs::write(
        &bad_tool,
        "{\"name\": \"yeban_not_a_tool\", \"arguments\": {\"dryRun\": true}}\n",
    )
    .expect("写违法 ToolCall");
    let dirty = run_validate_schemas(&dir);
    assert!(
        !dirty.status.success(),
        "未知工具名必须让契约变红。stdout={}",
        String::from_utf8_lossy(&dirty.stdout)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `python3 -c "import jsonschema"` 是否可用。
fn python_jsonschema_available() -> bool {
    std::process::Command::new("python3")
        .args(["-c", "import jsonschema"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// 跑一次 `validate_schemas.py --samples-dir <dir>`。
fn run_validate_schemas(dir: &std::path::Path) -> std::process::Output {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let script = repo
        .join("scripts")
        .join("gates")
        .join("validate_schemas.py");
    std::process::Command::new("python3")
        .arg(script)
        .arg("--samples-dir")
        .arg(dir)
        .current_dir(repo)
        .output()
        .expect("跑 validate_schemas.py")
}

// ---------------------------------------------------------------------------
// 判据 7: 红线自身的机器化自查
// ---------------------------------------------------------------------------

/// 扩展工具的实参约束必须与 Rust 注册表**逐字段一致**，且**既有十工具不得出现在本节**。
///
/// 为什么需要这条：`definitions.ExtensionToolArguments` 是契约文件里唯一一节
/// **手写**的逐工具参数约束（其余工具的 `inputSchema` 由注册表派生）。
/// 手写就有漂移风险 —— 于是这条判据把两边的声明逐个字段对账，
/// 让"注册表改了、契约没跟上"立刻变红。
///
/// ADR-0001 **D46** 还给了本线一条**特别授权/限制**：只允许**新增**工具定义，
/// **不得改动既有十工具的参数语义**。这条判据把那个限制也机械化了：
/// 规范十工具在契约里**没有**任何 `$defs` 条目、也**没有**任何 `if/then` 分支
/// （它们的参数只存在于注册表派生的 `inputSchema` 里），因此"偷偷给某个老工具
/// 加一个 if/then 约束"会红。
#[test]
fn extension_argument_constraints_match_the_registry() {
    let root = contract();
    let wiring = root["definitions"]["ToolCall"]["allOf"]
        .as_array()
        .expect("ToolCall.allOf 必须存在（扩展工具的实参约束靠 if/then 接线）");
    assert_eq!(
        wiring.len(),
        tools::EXTENSION_TOOL_COUNT,
        "恰好一条 if/then 对应一个扩展工具"
    );
    let defs = &root["definitions"]["ExtensionToolArguments"]["$defs"];

    // (1) 既有十工具**绝不**出现在本节（D46 的"不得改动既有参数语义"）。
    for spec in tools::TOOLS.iter().take(tools::DOCUMENTED_TOOL_COUNT) {
        assert!(
            defs.get(spec.name).is_none(),
            "规范工具 `{}` 不得出现在 ExtensionToolArguments 里",
            spec.name
        );
        assert!(
            !wiring.iter().any(|branch| {
                branch["if"]["properties"]["name"]["const"].as_str() == Some(spec.name)
            }),
            "规范工具 `{}` 不得有 if/then 实参约束",
            spec.name
        );
    }

    // (2) 每个扩展工具：分支 ⇄ $defs ⇄ 注册表 三者逐字段一致。
    let extension_names: Vec<&str> = tools::TOOLS
        .iter()
        .skip(tools::DOCUMENTED_TOOL_COUNT)
        .map(|spec| spec.name)
        .collect();
    assert_eq!(extension_names, tools::EXTENSION_NAMES, "扩展清单顺序");
    for (index, tool) in extension_names.iter().enumerate() {
        let branch = &wiring[index];
        assert_eq!(
            branch["if"]["properties"]["name"]["const"], *tool,
            "第 {index} 条 if 必须判 `{tool}`（顺序 = 注册顺序）"
        );
        let reference = branch["then"]["properties"]["arguments"]["$ref"]
            .as_str()
            .expect("then 必须 $ref 到扩展参数定义");
        assert_eq!(
            reference,
            format!("#/definitions/ExtensionToolArguments/$defs/{tool}")
        );

        let spec = tools::tool(tool).unwrap_or_else(|| panic!("`{tool}` 必须注册"));
        let declared = defs[*tool]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("`{tool}` 的 $defs 必须有 properties"));

        // (2a) 属性集合**恰好**等于注册表的全部参数（公共两个 + 特有）。
        let mut declared_keys: Vec<&str> = declared.keys().map(String::as_str).collect();
        declared_keys.sort_unstable();
        let mut registry_keys: Vec<&str> =
            spec.all_params().iter().map(|param| param.name).collect();
        registry_keys.sort_unstable();
        assert_eq!(
            declared_keys, registry_keys,
            "`{tool}` 的实参集合必须与注册表逐项相等"
        );

        // (2b) 每个参数的 JSON 类型一致。
        for param in spec.all_params() {
            let property = &declared[param.name];
            assert_eq!(
                property["type"], param.json_type,
                "`{tool}.{}` 的 JSON 类型必须与注册表一致",
                param.name
            );
        }

        // (2c) 必填集合一致 —— 口径是手写 schema 的 `required` 数组。
        let mut registry_required: Vec<String> = spec
            .required_params()
            .iter()
            .map(|param| param.name.to_owned())
            .collect();
        registry_required.sort();
        let schema_required: Vec<String> = defs[*tool]
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.as_str().expect("字符串").to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let mut schema_required_sorted = schema_required.clone();
        schema_required_sorted.sort();
        assert_eq!(
            schema_required_sorted, registry_required,
            "`{tool}` 的必填集合必须与注册表一致 (手写 schema 的 required 数组)"
        );
        // 反向：注册表说可选的那些参数，schema 里不得列进 required。
        for param in spec.all_params() {
            if !param.required {
                assert!(
                    !schema_required.contains(&param.name.to_owned()),
                    "`{tool}.{}` 在注册表里是可选参数, schema 不得把它列成必填",
                    param.name
                );
            }
        }

        // (2d) `dryRun` 的缺省值口径与公共声明一致（false）。
        assert_eq!(
            declared[tools::DRY_RUN_PARAM]["default"],
            serde_json::Value::from(false),
            "`{tool}.dryRun` 的 default 必须是 false"
        );
        assert_eq!(
            declared[tools::IDEMPOTENCY_KEY_PARAM]["type"],
            "string",
            "`{tool}.idempotencyKey` 必须是字符串"
        );
    }

    // (3) 三个 D46 工具的名字必须真的在契约 enum 里（可发现性的一半；另一半是 tools/list）。
    let names = contract_tool_names();
    for tool in [
        "yeban_edit_automation",
        "yeban_query_engine_state",
        "yeban_import_audio",
    ] {
        assert!(names.contains(&tool.to_owned()), "契约 enum 缺少 `{tool}`");
    }

    // (4) 扩展工具声明的错误码必须落在 D25 的 20 值联集里。
    let contract_codes = contract_error_codes();
    for spec in tools::TOOLS.iter().skip(tools::DOCUMENTED_TOOL_COUNT) {
        for code in spec.errors {
            assert!(
                contract_codes.contains(&code.as_str().to_owned()),
                "`{}` 声明了契约 enum 之外的错误码 {code}",
                spec.name
            );
        }
    }
}

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
