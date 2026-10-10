//! 规范样本导出（跨语言契约对账的**非测试**入口）[MUST-GATE-010, TEST-SPEC-005, MCP-TOOL-001..010]。
//!
//! 与 `crates/yeban-model/src/samples.rs` 同一套做法：样本由 **Rust serde** 写出，
//! 再由 `python3 scripts/gates/validate_schemas.py --samples-dir <dir>` 交给
//! **Python jsonschema** 逐份对账。两个独立实现互相钉住，任意一边漂移都会变红。
//!
//! 文件名前缀 `mcp-tools.` 已映射到 `schemas/mcp-tools.schema.json`
//! （见 `validate_schemas.py` 的 `SAMPLE_SCHEMA_MAP`）。
//!
//! ## 导出什么（份数由注册表派生：每工具一份实例 + 2 份 ToolResponse 实例 + 2 份文档样本）
//!
//! | 文件 | 内容 | 为什么 |
//! | :--- | :--- | :--- |
//! | `mcp-tools.registry.meta.json`（文档样本，非契约实例） | 全部工具的注册表快照（含 scope / 副作用 / 参数 / 错误码） | 工具名集合、`dryRun`、`idempotencyKey` 的机器可读清单 |
//! | `mcp-tools.error-codes.meta.json`（文档样本，非契约实例） | 错误码全集 + **契约缺口清单** | 让"schema 的 7 个 enum 装不下表格的 16 个错误码"这件事可被机器读到 |
//! | `mcp-tools.response.dry-run.json` | **真实管线**产出的 `ToolResponse`（dryRun 结果） | 覆盖根 `oneOf` 的**第二个分支**：否则契约定义了没人用的类型 |
//! | `mcp-tools.response.replayed.json` | **真实管线**产出的 `ReplayedToolResponse`（同键第二次调用） | 覆盖根 `oneOf` 的**第三个分支**：幂等重放的 `result` 不是裸 `ToolResponse`，这条路径此前完全不在契约里 |
//! | `mcp-tools.call.<tool>.json` ×`TOOLS.len()` | 每个工具一份**规范 `ToolCall`** | 每个工具名都要被契约的 enum 认下来 |
//!
//! ## 契约实例 vs 文档样本（`.meta.` 约定 + 承重的根）
//!
//! ### 1) 契约现在是**承重**的（ADR-0001 D25）
//!
//! 第一轮实测出来的事实是：本 schema 的根当时只有 `{"type":"object","definitions":{…}}`，
//! 没有 `properties` / `$ref` / `allOf` —— Draft 2020-12 下"任意对象"都通过根校验，
//! `definitions.*` **从未被引用**，`--samples-dir` 对账是空转的。
//! 集成者按 ADR-0001 D25 把根改成 `oneOf($ref ToolCall, $ref ToolResponse)`。
//!
//! ### 2) 但"清单/快照"**不是契约实例**
//!
//! 承重的根立刻暴露了一个**类型错误**：注册表快照与错误码清单本来就不是 `ToolCall` /
//! `ToolResponse` 的实例，拿 `oneOf` 根去校验它们属于用错类型。
//! 因此有了 `.meta.` 命名约定（`validate_schemas.py` 的
//! `SAMPLE_NAMING_CONVENTION`）：
//!
//! - `mcp-tools.<name>.json` —— **契约实例**，必须通过 `mcp-tools.schema.json` 的根；
//! - `mcp-tools.<name>.meta.json` —— **文档样本**，显式 `[skip]`，不对账 schema。
//!
//! 本模块导出的样本因此分成两侧：**契约实例**（每个工具一份规范 `ToolCall`，
//! 外加两份**真实管线**产出的 `ToolResponse` / `ReplayedToolResponse`）+ **2 份文档样本**。
//!
//! 为什么非要那份 `ToolResponse` 实例：根是 `oneOf(ToolCall, ToolResponse)`，
//! 全是 `ToolCall` 的话，第二个分支**从未被任何样本覆盖** —— 那正是"契约定义了
//! 没人用的类型"。这份样本由 [`crate::dispatch`] 的 `dryRun` 短路真实产出，
//! 因此它同时钉住"实现产出的 `ToolResponse` 必须被契约接受"。
//!
//! 同理，那份 `ReplayedToolResponse` 实例由**同一个** `Dispatcher` 上的同键两次
//! `tools/call` 真实产出：幂等命中的 `result` 是信封而不是裸 `ToolResponse`，
//! 没有这份样本时第三个分支同样"定义了没人用的类型"。
//!
//! ### 3) `.meta.` 是命名约定，不是 schema 能力 —— 所以要两道守卫
//!
//! 谁都能把一份**本该对账的实例**改名成 `.meta.` 来逃逸。防线有两道：
//!
//! 1. **脚本侧**（`scripts/gates/validate_schemas.py`，集成者的文件）：每个前缀
//!    至少要有 1 份真实例，否则报"该契约等于没有对账（全是 meta 就是假绿）"。
//!    它挡得住**整段逃逸**，挡不住**部分逃逸**（契约实例里混 1 份 meta）。
//! 2. **本 crate 侧**：`tests/contract.rs::exported_instance_set_is_exactly_the_tool_set`
//!    用**双射**断言 —— 非 meta 的实例文件名集合必须**恰好等于**
//!    `{mcp-tools.call.<tool>.json}`，少一份、多一份、或者把一份改名成 `.meta.` 都会红。
//!    另有 `document_samples_must_not_look_like_contract_instances`：
//!    文档样本顶层**不许**出现 `name` / `arguments` / `status`，免得用 `.meta.` 藏实例。
//!
//! `tests/contract.rs::contract_rejects_a_deliberately_invalid_sample` 把"契约承重"
//! 这件事本身也钉成了判据（故意违法的样本必须让 `--samples-dir` 变红）。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::dispatch::Dispatcher;
use crate::security::Channel;
use crate::tools::{ErrorCode, ParamSpec, TOOLS, ToolSpec};

/// 默认样本目录名（相对工作区 `target/`）。
pub const SAMPLES_DIR_NAME: &str = "schema-samples";

/// 文件名前缀（决定用哪份 schema）。
pub const FILE_PREFIX: &str = "mcp-tools";

/// 注册表样本文件名。
pub const REGISTRY_FILE: &str = "mcp-tools.registry.meta.json";

/// 错误码样本文件名。
pub const ERROR_CODES_FILE: &str = "mcp-tools.error-codes.meta.json";

/// `ToolCall` 样本的文件名前缀。
pub const CALL_FILE_PREFIX: &str = "mcp-tools.call.";

/// **`ToolResponse` 契约实例**样本：真实管线产出的 `dryRun` 结果。
///
/// 为什么必须有它：契约的根是 `oneOf($ref ToolCall, $ref ToolResponse)`。
/// 如果样本目录里全是 `ToolCall`，那个 `oneOf` 的**第二个分支从未被任何样本覆盖** ——
/// 正是集成者在 `ci.yml` 里警告的"契约定义了没人用的类型"。
/// 这份样本走的是**真实分发管线**（[`crate::dispatch`] 的 `dryRun` 短路），
/// 不是手写的形状，因此它同时钉住了"实现产出的 `ToolResponse` 必须被契约接受"。
pub const RESPONSE_DRY_RUN_FILE: &str = "mcp-tools.response.dry-run.json";

/// **`ReplayedToolResponse` 契约实例**样本：真实管线产出的幂等重放信封。
///
/// 为什么必须有它：在它之前，根 `oneOf` 的第三个分支（`ReplayedToolResponse`）
/// **没有**任何样本覆盖，而这条路径是**真实存在**的 —— `arguments.idempotencyKey`
/// 命中缓存时 `tools/call` 的 `result` 就是那个信封（`crates/yeban-mcp/src/dispatch.rs`
/// 的 `replay_value`）。没有这份样本时，"信封不在契约里"这件事对
/// `validate_schemas.py` 是**不可见的**（2026-10-07 实测：`grep -c 'replayed'
/// schemas/mcp-tools.schema.json` ⇒ 0）。
///
/// 这份样本同样走**真实分发管线**（同一个 `Dispatcher` 上同键调两次，取第二次），
/// 因此它钉住"实现产出的重放信封必须被契约接受"；重放路径若改回裸 `ToolResponse`，
/// [`check_replayed_response_instance`] 在写盘前就会变红。
pub const RESPONSE_REPLAYED_FILE: &str = "mcp-tools.response.replayed.json";

/// 样本导出失败。
#[derive(Debug, thiserror::Error)]
pub enum SampleExportError {
    /// 文件系统错误。
    #[error("写入样本失败: {0}")]
    Io(#[from] std::io::Error),
    /// JSON 序列化错误。
    #[error("序列化样本失败: {0}")]
    Serialize(#[from] serde_json::Error),
    /// 样本自身不满足契约（写盘前的 Rust 侧自检）。
    #[error("样本 `{file}` 未通过契约自检: {detail}")]
    InvalidSample {
        /// 文件名。
        file: String,
        /// 人话说明。
        detail: String,
    },
}

/// `<tool>` 的 `ToolCall` 样本文件名。
#[must_use]
pub fn call_file(tool_name: &str) -> String {
    format!("{CALL_FILE_PREFIX}{tool_name}.json")
}

/// 全部样本文件名（顺序固定，逐字节稳定）。
#[must_use]
pub fn sample_file_names() -> Vec<String> {
    let mut names = vec![
        REGISTRY_FILE.to_owned(),
        ERROR_CODES_FILE.to_owned(),
        RESPONSE_DRY_RUN_FILE.to_owned(),
        RESPONSE_REPLAYED_FILE.to_owned(),
    ];
    names.extend(TOOLS.iter().map(|spec| call_file(spec.name)));
    names
}

/// 注册表样本：工具名集合 / 参数 / 作用域 / 副作用 / 错误码的机器可读快照。
///
/// 外层是 `ToolResponse`（`{"status":"success","data":{…}}`）—— 契约的根是
/// `oneOf(ToolCall, ToolResponse)`，裸对象会被拒（ADR-0001 D25）。
#[must_use]
pub fn registry_sample() -> Value {
    let mut root = Map::new();
    root.insert("count".to_owned(), Value::from(TOOLS.len()));
    root.insert(
        "dryRunParam".to_owned(),
        Value::from(crate::tools::DRY_RUN_PARAM),
    );
    root.insert(
        "idempotencyKeyParam".to_owned(),
        Value::from(crate::tools::IDEMPOTENCY_KEY_PARAM),
    );
    root.insert(
        "tools".to_owned(),
        Value::Array(
            TOOLS
                .iter()
                .map(|spec| {
                    let mut tool = Map::new();
                    tool.insert("specId".to_owned(), Value::from(spec.spec_id));
                    tool.insert("name".to_owned(), Value::from(spec.name));
                    tool.insert("requiredScope".to_owned(), Value::from(spec.scope.as_str()));
                    tool.insert(
                        "sideEffect".to_owned(),
                        Value::from(spec.side_effect.as_str()),
                    );
                    tool.insert(
                        "sideEffecting".to_owned(),
                        Value::from(spec.side_effect.is_side_effecting()),
                    );
                    tool.insert(
                        "supportsDryRun".to_owned(),
                        Value::from(spec.param(crate::tools::DRY_RUN_PARAM).is_some()),
                    );
                    tool.insert(
                        "supportsIdempotencyKey".to_owned(),
                        Value::from(spec.param(crate::tools::IDEMPOTENCY_KEY_PARAM).is_some()),
                    );
                    tool.insert(
                        "params".to_owned(),
                        Value::Array(spec.all_params().into_iter().map(param_value).collect()),
                    );
                    tool.insert(
                        "errorCodes".to_owned(),
                        Value::Array(
                            spec.errors
                                .iter()
                                .map(|code| Value::from(code.as_str()))
                                .collect(),
                        ),
                    );
                    Value::Object(tool)
                })
                .collect(),
        ),
    );
    Value::Object(root)
}

/// 一个参数的样本形状。
fn param_value(param: &ParamSpec) -> Value {
    let mut map = Map::new();
    map.insert("name".to_owned(), Value::from(param.name));
    map.insert("type".to_owned(), Value::from(param.json_type));
    map.insert("required".to_owned(), Value::from(param.required));
    Value::Object(map)
}

/// 错误码样本：契约联集（20）/ 规范表格（16）/ schema 原有（4）/ 实现级（1）。
///
/// 历史：本线第一轮实测出"schema 的 7 值 enum 装不下表格的 16 个错误码"（缺口 13 个），
/// 该缺口已由 **ADR-0001 D25** 关闭（契约改成联集 20 值）。
/// 缺口清单作为方法论留痕保存在 `docs/ledger/mcp-core-notes.md` §3.1；
/// 这里给出的是**修复后**的四个集合，判据要求它们逐一对上。
#[must_use]
pub fn error_codes_sample() -> Value {
    let schema_only: Vec<Value> = ErrorCode::SCHEMA_ONLY
        .iter()
        .map(|code| Value::from(code.as_str()))
        .collect();
    let implementation_only: Vec<Value> = ErrorCode::ALL
        .iter()
        .filter(|code| !code.is_schema_contract() && !code.is_documented_tool_code())
        .map(|code| Value::from(code.as_str()))
        .collect();
    let mut root = Map::new();
    root.insert(
        "schemaContractEnum".to_owned(),
        Value::Array(
            ErrorCode::SCHEMA_CONTRACT
                .iter()
                .map(|code| Value::from(code.as_str()))
                .collect(),
        ),
    );
    root.insert(
        "documentedToolCodes".to_owned(),
        Value::Array(
            ErrorCode::DOCUMENTED_TOOL_CODES
                .iter()
                .map(|code| Value::from(code.as_str()))
                .collect(),
        ),
    );
    root.insert("schemaOnlyCodes".to_owned(), Value::Array(schema_only));
    root.insert(
        "implementationOnly".to_owned(),
        Value::Array(implementation_only),
    );
    root.insert(
        "all".to_owned(),
        Value::Array(
            ErrorCode::ALL
                .iter()
                .map(|code| Value::from(code.as_str()))
                .collect(),
        ),
    );
    Value::Object(root)
}

/// **真实管线**产出的 `ToolResponse`（`yeban_query_project` 的 `dryRun` 结果）。
///
/// 走的是 [`crate::dispatch::Dispatcher::handle_line`]，与线上完全同一条路径；
/// 令牌在响应里不出现，工程由 [`crate::domain::Domain::open_in_memory`] 注入
/// （不碰文件系统、不用随机 ULID 装配载荷），因此样本逐字节稳定。
///
/// 为什么是"只读工具 + 内存工程"：
///
/// - `dryRun` 现在会**真的做领域合法性校验**（规范对它的定义是
///   "只做参数与领域合法性校验"）。没有活跃工程时 `yeban_save_project` 的
///   `dryRun` 会如实返回 `NO_ACTIVE_PROJECT` —— 那也是一份合法的 `ToolResponse`，
///   但用它当样本会让"成功形状"从未被契约覆盖；
/// - 因此注入一份确定性的规范工程，再对只读工具做 `dryRun`：
///   成功形状 + 真实差异预览（`projectDigestBefore/After`、`commitCount*`）都被覆盖。
#[must_use]
pub fn dry_run_response_sample() -> Value {
    let (mut dispatcher, authorization) = sample_dispatcher();
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "sample-dry-run",
        "method": "tools/call",
        "params": {
            "name": "yeban_query_project",
            "arguments": {
                "limit": 3,
                "offset": 0,
                "fields": ["title", "bpm", "tracks.name"],
                "dryRun": true
            }
        }
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&authorization), &line);
    let response = outcome.response.expect("dryRun 调用必须产生响应");
    response.result.expect("dryRun 必须成功并返回 ToolResponse")
}

/// 一份**确定性的**样本会话：规范工程 + 全作用域 + 时钟钉在 0。
///
/// 两份 `ToolResponse` 契约实例（dryRun 与幂等重放）共用它，于是两者的
/// 逐字节稳定性只取决于各自的 `tools/call` 载荷。
fn sample_dispatcher() -> (Dispatcher, String) {
    use crate::security::{BearerToken, RunMode, ScopeSet};

    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(0);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-sample/demo.yeban"),
            yeban_model::samples::filled_project(),
            true,
        )
        .expect("规范样本工程必须通过结构校验");
    (dispatcher, authorization)
}

/// **`ReplayedToolResponse` 契约实例**样本：真实管线产出的幂等重放信封。
///
/// 做法与 [`dry_run_response_sample`] 同一形态（**真实分发管线**，不是手写形状）：
/// 对只读的 `yeban_query_project` 用**同一个** `idempotencyKey` 调**两次**，
/// 返回第二次的 `result` —— 那就是 [`crate::dispatch`] 的 `replay_value` 信封。
///
/// 确定性：键与请求都固定，时钟钉在 0，因此两次导出逐字节相同。
#[must_use]
pub fn replay_response_sample() -> Value {
    let (mut dispatcher, authorization) = sample_dispatcher();
    let call = |id: &str| {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "yeban_query_project",
                "arguments": {
                    "limit": 3,
                    "offset": 0,
                    "fields": ["title", "bpm", "tracks.name"],
                    "idempotencyKey": "sample-replayed"
                }
            }
        })
        .to_string()
    };
    let first = dispatcher.handle_line(
        Channel::Http,
        Some(&authorization),
        &call("sample-replayed"),
    );
    let first = first.response.expect("首次调用必须产生响应");
    let first_result = first.result.expect("首次调用必须返回裸 ToolResponse");
    assert!(
        first_result.get("status").is_some() && first_result.get("replayed").is_none(),
        "首次调用必须是裸 ToolResponse"
    );
    let second = dispatcher.handle_line(
        Channel::Http,
        Some(&authorization),
        &call("sample-replayed"),
    );
    assert_eq!(dispatcher.replayed(), 1, "第二次必须真的命中幂等缓存");
    let second = second.response.expect("重放调用必须产生响应");
    let envelope = second.result.expect("重放必须成功并返回信封");
    assert_eq!(
        envelope["response"]["result"], first_result,
        "重放的主体必须与首次逐字节相同 (信封里只有 id 换成当前请求)"
    );
    envelope
}

/// `ToolResponse` **契约实例**的 Rust 侧自检。
///
/// 判据口径与 `schemas/mcp-tools.schema.json` 的 `definitions.ToolResponse` 对齐：
/// `status` 必须在 `{success, error}` 里；`data` 若出现必须是对象；
/// `error.code` 若出现必须在 [`ErrorCode::ALL`] 目录里（且**不能**是实现级的
/// `NOT_IMPLEMENTED` —— 那是 JSON-RPC 层的码）。
///
/// # Errors
///
/// 违反上述任一条。
pub fn check_tool_response_instance(file: &str, sample: &Value) -> Result<(), SampleExportError> {
    let invalid = |detail: String| SampleExportError::InvalidSample {
        file: file.to_owned(),
        detail,
    };
    let object = sample
        .as_object()
        .ok_or_else(|| invalid("ToolResponse 必须是 JSON 对象".to_owned()))?;
    match object.get("status").and_then(Value::as_str) {
        Some("success") | Some("error") => {}
        Some(other) => return Err(invalid(format!("`status` 只能是 success/error: {other}"))),
        None => return Err(invalid("缺少字符串 `status`".to_owned())),
    }
    if let Some(data) = object.get("data")
        && !data.is_object()
    {
        return Err(invalid("`data` 必须是对象".to_owned()));
    }
    if let Some(error) = object.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("`error.code` 必须是字符串".to_owned()))?;
        if error.get("message").and_then(Value::as_str).is_none() {
            return Err(invalid("`error.message` 是契约必填项".to_owned()));
        }
        let known = ErrorCode::SCHEMA_CONTRACT
            .iter()
            .any(|known| known.as_str() == code);
        if !known {
            return Err(invalid(format!(
                "`error.code` `{code}` 不在契约 enum 里 (NOT_IMPLEMENTED 属于 JSON-RPC 层, 不许混进来)"
            )));
        }
    }
    Ok(())
}

/// `ReplayedToolResponse` **契约实例**的 Rust 侧自检。
///
/// 判据口径与 `schemas/mcp-tools.schema.json` 的 `definitions.ReplayedToolResponse`
/// （+ `definitions.JsonRpcResponse`）对齐：
///
/// - 顶层是对象，且 `replayed` **恰好** `true`、`response` 是对象；
/// - `response.jsonrpc` **恰好** `"2.0"`，`response.id` 是整数 / 字符串 / `null`；
/// - `response` 恰好有 `result` 或 `error` 之一；
/// - 若 `response.result` 在场，它必须是一份合法 `ToolResponse`
///   （复用 [`check_tool_response_instance`] 的口径）；
/// - 若 `response.error` 在场，`code` 必须是整数、`message` 必须是字符串。
///
/// 这条自检是"重放路径改回裸 `ToolResponse`"的**红线**：那时 `replayed` 不在场，
/// [`export_all`] 会在写盘前返回 [`SampleExportError::InvalidSample`]。
///
/// # Errors
///
/// 违反上述任一条。
pub fn check_replayed_response_instance(
    file: &str,
    sample: &Value,
) -> Result<(), SampleExportError> {
    let invalid = |detail: String| SampleExportError::InvalidSample {
        file: file.to_owned(),
        detail,
    };
    let object = sample
        .as_object()
        .ok_or_else(|| invalid("ReplayedToolResponse 必须是 JSON 对象".to_owned()))?;
    if object.get("replayed") != Some(&Value::Bool(true)) {
        return Err(invalid(
            "`replayed` 必须**恰好**是 `true` —— 裸 `ToolResponse` 不是重放信封".to_owned(),
        ));
    }
    let response = object
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("`response` 必须是对象 (完整 JSON-RPC 响应)".to_owned()))?;
    if response.get("jsonrpc") != Some(&Value::from("2.0")) {
        return Err(invalid(
            "`response.jsonrpc` 必须恰好是 `\"2.0\"`".to_owned(),
        ));
    }
    match response.get("id") {
        Some(Value::Number(number)) if number.is_i64() => {}
        Some(Value::String(_) | Value::Null) => {}
        _ => {
            return Err(invalid(
                "`response.id` 必须是整数 / 字符串 / null".to_owned(),
            ));
        }
    }
    let has_result = response.contains_key("result");
    let has_error = response.contains_key("error");
    if has_result == has_error {
        return Err(invalid(
            "`response` 必须恰好有 `result` 或 `error` 之一".to_owned(),
        ));
    }
    if let Some(result) = response.get("result") {
        let inner = format!("{file}#response.result");
        check_tool_response_instance(&inner, result)?;
    }
    if let Some(error) = response.get("error") {
        if !error.get("code").is_some_and(Value::is_i64) {
            return Err(invalid("`response.error.code` 必须是整数".to_owned()));
        }
        if error.get("message").and_then(Value::as_str).is_none() {
            return Err(invalid("`response.error.message` 必须是字符串".to_owned()));
        }
    }
    Ok(())
}

/// 一个工具的规范 `ToolCall` 样本（含全部必填实参 + 两个公共参数）。
#[must_use]
pub fn call_sample(spec: &ToolSpec) -> Value {
    let mut arguments = Map::new();
    for param in spec.all_params() {
        if param.required {
            arguments.insert(param.name.to_owned(), fixture_argument(param));
        }
    }
    arguments.insert(crate::tools::DRY_RUN_PARAM.to_owned(), Value::from(true));
    arguments.insert(
        crate::tools::IDEMPOTENCY_KEY_PARAM.to_owned(),
        Value::from(format!("sample-{}", spec.name)),
    );
    let mut root = Map::new();
    root.insert("name".to_owned(), Value::from(spec.name));
    root.insert("arguments".to_owned(), Value::Object(arguments));
    Value::Object(root)
}

/// 参数的确定性夹具值（样本必须逐字节稳定，因此不用随机值）。
fn fixture_argument(param: &ParamSpec) -> Value {
    match param.name {
        "path" => Value::from("~/Music/yeban/demo.yeban"),
        "format" => Value::from("wav"),
        "sampleRate" => Value::from(48_000),
        "bars" => Value::from(8),
        "limit" => Value::from(16),
        "offset" => Value::from(0),
        "macroIndex" => Value::from(0),
        "sectionName" => Value::from("Chorus"),
        "stylePreset" => Value::from("orchestral_film_score"),
        "scale" => Value::from("C minor"),
        "commitMessage" => Value::from("AI 提案: 副歌加一层八度"),
        "reason" => Value::from("织体过密"),
        "fields" => Value::Array(vec![Value::from("tracks"), Value::from("sections")]),
        "ops" => Value::Array(vec![Value::Object(Map::new())]),
        "value" => Value::from(0.5),
        // ADR-0001 D46 的三个扩展工具（词汇表与 `project.json` 逐字相同）。
        "lane" => Value::from("TrackVolume"),
        "name" => Value::from("AI 导入的底鼓"),
        "ticks" => Value::Array(vec![Value::from(0), Value::from(960)]),
        "gainDb" => Value::from(-1.5),
        "slotIndex" | "paramIndex" => Value::from(0),
        "point" => serde_json::json!({"tick": 0, "value": -6.0, "curve": "Linear"}),
        other if other.ends_with("Id") => Value::from("01J8ZQ00000000000000000001"),
        _ => match param.json_type {
            "string" => Value::from("01J8ZQ00000000000000000001"),
            "integer" => Value::from(1),
            "number" => Value::from(1.0),
            "boolean" => Value::from(true),
            "array" => Value::Array(Vec::new()),
            _ => Value::Object(Map::new()),
        },
    }
}

/// 默认样本目录：`<repo>/target/schema-samples`。
///
/// 遵循 Cargo 的 `CARGO_TARGET_DIR`（若设置），否则用 `<crate>/../../target`。
#[must_use]
pub fn default_out_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || manifest_dir.join("..").join("..").join("target"),
        PathBuf::from,
    );
    target.join(SAMPLES_DIR_NAME)
}

/// 把全部样本写到 `out_dir`，返回实际写出的路径（顺序固定；份数由 `TOOLS` 派生）。
///
/// 写盘**之前**先做 Rust 侧自检（[`check_sample`]），因此磁盘上不会出现
/// 一份连自己都不合法的样本。
///
/// # Errors
///
/// 目录创建 / 写入失败、序列化失败，或任一样本未通过自检。
pub fn export_all(out_dir: &Path) -> Result<Vec<PathBuf>, SampleExportError> {
    std::fs::create_dir_all(out_dir)?;
    let mut written = Vec::new();

    // 两份**文档样本** (.meta., 不对账 schema)。
    let registry = registry_sample();
    check_document_sample(REGISTRY_FILE, &registry)?;
    written.push(write_json(out_dir, REGISTRY_FILE, &registry)?);
    let error_codes = error_codes_sample();
    check_document_sample(ERROR_CODES_FILE, &error_codes)?;
    written.push(write_json(out_dir, ERROR_CODES_FILE, &error_codes)?);
    // 一份 ToolResponse **契约实例**: 真实管线产出的 dryRun 结果。
    let response = dry_run_response_sample();
    check_tool_response_instance(RESPONSE_DRY_RUN_FILE, &response)?;
    written.push(write_json(out_dir, RESPONSE_DRY_RUN_FILE, &response)?);
    // 一份 ReplayedToolResponse **契约实例**: 真实管线产出的幂等重放信封。
    let replay = replay_response_sample();
    check_replayed_response_instance(RESPONSE_REPLAYED_FILE, &replay)?;
    written.push(write_json(out_dir, RESPONSE_REPLAYED_FILE, &replay)?);
    // 十份 ToolCall **契约实例** (每个工具一份规范 ToolCall)。
    for spec in &TOOLS {
        let file = call_file(spec.name);
        let sample = call_sample(spec);
        check_sample(&file, &sample)?;
        written.push(write_json(out_dir, &file, &sample)?);
    }
    Ok(written)
}

/// 写到 [`default_out_dir`]。
///
/// # Errors
///
/// 同 [`export_all`]。
pub fn export_to_default_dir() -> Result<Vec<PathBuf>, SampleExportError> {
    export_all(&default_out_dir())
}

/// `ToolCall` 样本的 Rust 侧自检：形状必须与 `definitions.ToolCall` 一致。
///
/// # Errors
///
/// 缺少 `name` / `arguments`、工具名不在注册表里、或缺少公共参数。
pub fn check_sample(file: &str, sample: &Value) -> Result<(), SampleExportError> {
    let invalid = |detail: String| SampleExportError::InvalidSample {
        file: file.to_owned(),
        detail,
    };
    let object = sample
        .as_object()
        .ok_or_else(|| invalid("样本必须是 JSON 对象".to_owned()))?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("缺少字符串 `name`".to_owned()))?;
    if crate::tools::tool(name).is_none() {
        return Err(invalid(format!("`{name}` 不在注册表里")));
    }
    let arguments = object
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("缺少对象 `arguments`".to_owned()))?;
    for key in [
        crate::tools::DRY_RUN_PARAM,
        crate::tools::IDEMPOTENCY_KEY_PARAM,
    ] {
        if !arguments.contains_key(key) {
            return Err(invalid(format!("`arguments` 缺少契约要求的 `{key}`")));
        }
    }
    Ok(())
}

/// **文档样本**的 Rust 侧自检。
///
/// `.meta.` 是"不对账 schema"的通行证，因此这里必须挡住"拿它藏实例"：
/// 文档样本顶层**不许**出现 `name` / `arguments` / `status` 这三个契约实例的判别键。
///
/// # Errors
///
/// 不是 JSON 对象，或者顶层出现了契约实例的判别键。
pub fn check_document_sample(file: &str, sample: &Value) -> Result<(), SampleExportError> {
    let invalid = |detail: String| SampleExportError::InvalidSample {
        file: file.to_owned(),
        detail,
    };
    let object = sample
        .as_object()
        .ok_or_else(|| invalid("文档样本必须是 JSON 对象".to_owned()))?;
    for key in ["name", "arguments", "status"] {
        if object.contains_key(key) {
            return Err(invalid(format!(
                "文档样本顶层出现了契约实例的判别键 `{key}`: \
                 要么它是实例(请去掉 .meta.), 要么它不该有这个名字"
            )));
        }
    }
    Ok(())
}

/// 以"美化 JSON + 结尾换行"写出一个样本。
fn write_json(out_dir: &Path, file: &str, value: &Value) -> Result<PathBuf, SampleExportError> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    let path = out_dir.join(file);
    std::fs::write(&path, json)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::Scope;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yeban-mcp-samples-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn export_writes_the_registry_derived_sample_set() {
        let dir = temp_dir("stable");
        let written = export_all(&dir).expect("导出样本");
        let names: Vec<String> = written
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, sample_file_names());
        assert_eq!(
            names.len(),
            crate::tools::TOOL_COUNT + 4,
            "每个工具一份 ToolCall 实例 + 2 份 ToolResponse 实例 + 2 份文档样本"
        );
        for name in &names {
            assert!(
                name.starts_with(FILE_PREFIX),
                "前缀必须是 mcp-tools: {name}"
            );
        }

        let before: Vec<String> = written
            .iter()
            .map(|path| std::fs::read_to_string(path).expect("读样本"))
            .collect();
        let again = export_all(&dir).expect("再次导出");
        let after: Vec<String> = again
            .iter()
            .map(|path| std::fs::read_to_string(path).expect("读样本"))
            .collect();
        assert_eq!(before, after, "样本导出必须逐字节稳定");
        for content in &before {
            assert!(content.ends_with('\n'), "样本必须以换行结尾");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn registry_sample_lists_every_tool_and_the_two_common_params() {
        let registry = registry_sample();
        // 文档样本: 顶层就是快照本身, **不是**契约实例 (所以名字里有 .meta.)。
        assert!(registry.get("status").is_none());
        assert!(registry.get("name").is_none());
        assert_eq!(registry["count"], crate::tools::TOOL_COUNT);
        assert_eq!(registry["dryRunParam"], crate::tools::DRY_RUN_PARAM);
        assert_eq!(
            registry["idempotencyKeyParam"],
            crate::tools::IDEMPOTENCY_KEY_PARAM
        );
        let tools = registry["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), crate::tools::TOOL_COUNT);
        for (index, spec) in TOOLS.iter().enumerate() {
            assert_eq!(tools[index]["name"], spec.name);
            assert_eq!(tools[index]["specId"], spec.spec_id);
            assert_eq!(tools[index]["supportsDryRun"], true);
            assert_eq!(tools[index]["supportsIdempotencyKey"], true);
            assert!(
                Scope::parse(tools[index]["requiredScope"].as_str().expect("scope")).is_ok(),
                "作用域必须是六级之一"
            );
        }
    }

    #[test]
    fn error_codes_sample_matches_the_union_contract() {
        let sample = error_codes_sample();
        // 文档样本 (`.meta.`): 顶层就是清单本身, 不装进 ToolResponse。
        assert!(sample.get("status").is_none(), "文档样本不是契约实例");
        assert_eq!(
            sample["schemaContractEnum"].as_array().expect("enum").len(),
            20,
            "ADR-0001 D25 的联集 20 值"
        );
        assert_eq!(
            sample["documentedToolCodes"]
                .as_array()
                .expect("enum")
                .len(),
            16,
            "架构 §7.2 表格"
        );
        let schema_only = sample["schemaOnlyCodes"].as_array().expect("schema 原有码");
        assert_eq!(schema_only.len(), 4);
        assert!(schema_only.contains(&Value::from("PERMISSION_DENIED")));
        assert_eq!(
            sample["implementationOnly"],
            serde_json::json!(["NOT_IMPLEMENTED"])
        );
        assert_eq!(
            sample["all"].as_array().expect("all").len(),
            ErrorCode::ALL.len()
        );
        // 字段名里不再有"缺口": 缺口已由 D25 关闭。
        assert!(sample.get("missingFromSchema").is_none());
    }

    #[test]
    fn dry_run_response_sample_is_a_contract_valid_tool_response() {
        let sample = dry_run_response_sample();
        check_tool_response_instance(RESPONSE_DRY_RUN_FILE, &sample).expect("必须自洽");
        assert_eq!(sample["status"], "success");
        assert_eq!(sample["data"]["dryRun"], true);
        assert_eq!(sample["data"]["tool"], "yeban_query_project");
        assert_eq!(sample["data"]["specId"], "MCP-TOOL-004");
        assert_eq!(
            sample["data"]["wouldChangeState"], false,
            "只读工具的 dryRun 不改变状态"
        );
        assert_eq!(sample["data"]["stateUnchanged"], true);
        assert!(sample["data"]["arguments"].is_object());
        assert!(sample["data"]["preview"]["projectDigestBefore"].is_string());
        assert_eq!(
            sample["data"]["preview"]["commitCountBefore"],
            sample["data"]["preview"]["commitCountAfter"],
            "只读工具不得改变提交数"
        );
        // 逐字节稳定: 令牌不参与响应, 两次生成完全一致。
        assert_eq!(sample, dry_run_response_sample());

        // 反例: 坏 status / 实现级码混进 error.code / data 不是对象, 都必须被拦下。
        assert!(
            check_tool_response_instance("x.json", &serde_json::json!({"status": "ok"})).is_err()
        );
        assert!(
            check_tool_response_instance(
                "x.json",
                &serde_json::json!({
                    "status": "error",
                    "error": {"code": "NOT_IMPLEMENTED", "message": "x"}
                })
            )
            .is_err(),
            "NOT_IMPLEMENTED 属于 JSON-RPC 层, 不许混进 ToolResponse.error.code"
        );
        assert!(
            check_tool_response_instance(
                "x.json",
                &serde_json::json!({"status": "success", "data": []})
            )
            .is_err()
        );
        // 正例: 领域失败码是合法的。
        check_tool_response_instance(
            "x.json",
            &serde_json::json!({
                "status": "error",
                "error": {"code": "DISK_FULL", "message": "磁盘写满"}
            }),
        )
        .expect("DISK_FULL 在契约 enum 里");
    }

    /// 幂等重放信封的样本自检 —— "重放路径改回裸 `ToolResponse`"的红线在这里。
    #[test]
    fn replay_response_sample_is_a_contract_valid_envelope() {
        let sample = replay_response_sample();
        check_replayed_response_instance(RESPONSE_REPLAYED_FILE, &sample).expect("必须自洽");
        assert_eq!(sample["replayed"], true);
        let response = &sample["response"];
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], "sample-replayed", "id 必须回显当前请求");
        assert_eq!(response["result"]["status"], "success");
        assert!(response.get("error").is_none(), "成功重放不该有 error");
        assert!(response["result"]["data"].is_object());
        // 逐字节稳定: 键与请求都固定, 两次生成完全一致。
        assert_eq!(sample, replay_response_sample());

        // 反例 1 (**承重的那一条**): 重放路径改回裸 ToolResponse ⇒ 必须被拦下。
        let bare = sample["response"]["result"].clone();
        assert!(bare.get("status").is_some(), "夹具取的就是裸 ToolResponse");
        let error = check_replayed_response_instance(RESPONSE_REPLAYED_FILE, &bare)
            .expect_err("裸 ToolResponse 不是重放信封");
        assert!(format!("{error}").contains("replayed"), "{error}");

        // 反例 2: `replayed` 不是恰好 true。
        let mut wrong = sample.clone();
        wrong["replayed"] = Value::from(false);
        assert!(check_replayed_response_instance("x.json", &wrong).is_err());

        // 反例 3: `response` 同时有 result 与 error。
        let mut both = sample.clone();
        both["response"]["error"] = serde_json::json!({"code": -32005, "message": "x"});
        assert!(check_replayed_response_instance("x.json", &both).is_err());

        // 反例 4: `response.id` 类型非法 (浮点不是合法的 JSON-RPC id)。
        let mut bad_id = sample.clone();
        bad_id["response"]["id"] = Value::from(1.5);
        assert!(check_replayed_response_instance("x.json", &bad_id).is_err());

        // 反例 5: 实现了级错误的重放 (result 不在场, error 在场) 是合法的。
        check_replayed_response_instance(
            "x.json",
            &serde_json::json!({
                "replayed": true,
                "response": {
                    "jsonrpc": "2.0",
                    "id": 7,
                    "error": {"code": -32005, "message": "尚未接线"}
                }
            }),
        )
        .expect("实现级失败也会被幂等缓存, 它的重放信封必须合法");
    }

    #[test]
    fn catalogue_samples_are_documents_not_instances() {
        for (file, sample) in [
            (REGISTRY_FILE, registry_sample()),
            (ERROR_CODES_FILE, error_codes_sample()),
        ] {
            assert!(file.contains(".meta."), "文档样本的名字必须带 .meta.");
            check_document_sample(file, &sample).expect("文档样本必须自洽");
            assert!(sample.is_object());
        }
        // 反例: 拿 .meta. 藏实例必须被拦下。
        for hidden in [
            serde_json::json!({"name": "yeban_save_project", "arguments": {}}),
            serde_json::json!({"status": "success", "data": {}}),
            serde_json::json!({"arguments": {}}),
        ] {
            assert!(
                check_document_sample("x.meta.json", &hidden).is_err(),
                "文档样本顶层不得出现契约实例的判别键: {hidden}"
            );
        }
        // 反例: 根本不是对象。
        assert!(check_document_sample("x.meta.json", &serde_json::json!([1, 2])).is_err());
    }

    #[test]
    fn every_call_sample_passes_the_rust_side_self_check() {
        for spec in &TOOLS {
            let sample = call_sample(spec);
            let file = call_file(spec.name);
            check_sample(&file, &sample).expect("样本必须自洽");
            assert_eq!(sample["name"], spec.name);
            // 必填参数都在 (dryRun 为 true, 因为 dryRun 是只读模拟)。
            for param in spec.required_params() {
                assert!(
                    sample["arguments"].get(param.name).is_some(),
                    "{} 的必填参数 {} 没进样本",
                    spec.name,
                    param.name
                );
            }
            assert_eq!(sample["arguments"][crate::tools::DRY_RUN_PARAM], true);
            assert_eq!(
                sample["arguments"][crate::tools::IDEMPOTENCY_KEY_PARAM],
                format!("sample-{}", spec.name)
            );
        }
    }

    #[test]
    fn self_check_rejects_a_broken_sample() {
        let error = check_sample(
            "mcp-tools.call.x.json",
            &serde_json::json!({"name": "yeban_nope", "arguments": {}}),
        )
        .expect_err("未知工具必须被自检拦下");
        assert!(error.to_string().contains("不在注册表里"));

        let error = check_sample(
            "mcp-tools.call.x.json",
            &serde_json::json!({"name": "yeban_save_project", "arguments": {}}),
        )
        .expect_err("缺公共参数必须被拦下");
        assert!(error.to_string().contains(crate::tools::DRY_RUN_PARAM));
    }

    #[test]
    fn default_out_dir_is_the_workspace_target() {
        let dir = default_out_dir();
        assert!(dir.ends_with(SAMPLES_DIR_NAME));
        assert!(dir.is_absolute());
    }

    #[test]
    fn export_to_target_feeds_the_python_reconciliation() {
        // 与 yeban-model 同一做法: 本机门禁顺带把样本落到 target/schema-samples,
        // 供 `validate_schemas.py --samples-dir` 对账 (CI 的 checks job 已接线)。
        let written = export_to_default_dir().expect("导出到 target/schema-samples");
        assert_eq!(written.len(), crate::tools::TOOL_COUNT + 4);
    }
    /// 样本目录名是**跨语言契约**（Python 探针与 CI 都按字面值找它）。
    ///
    /// 为什么不能用常量自比：既有判据 `default_out_dir_is_the_workspace_target` 写的是
    /// `dir.ends_with(SAMPLES_DIR_NAME)` ⇒ 把常量改成别的值时两边一起变、判据恒真，
    /// 而 `verify/e2e_audio_clip_seed.py` / `verify/idempotency_replay_schema.py`
    /// 与 `scripts/gates/validate_schemas.py --samples-dir target/schema-samples`
    /// 仍然按**字面值** `schema-samples` 找文件 ⇒ 它们会静默找不到（不是红）。
    ///
    /// 注入（实测红）：`SAMPLES_DIR_NAME` "schema-samples" → "samples" ⇒ 既有判据全绿；
    /// 本判据红。
    #[test]
    fn the_sample_directory_name_is_the_cross_language_literal() {
        assert_eq!(SAMPLES_DIR_NAME, "schema-samples");
        let dir = default_out_dir();
        assert!(
            dir.ends_with("schema-samples"),
            "缺省输出目录必须以字面值结尾: {}",
            dir.display()
        );
        assert!(dir.ends_with("target/schema-samples") || dir.ends_with("target\\schema-samples"));
    }
}
