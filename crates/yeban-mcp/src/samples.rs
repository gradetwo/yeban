//! 规范样本导出（跨语言契约对账的**非测试**入口）[MUST-GATE-010, TEST-SPEC-005, MCP-TOOL-001..010]。
//!
//! 与 `crates/yeban-model/src/samples.rs` 同一套做法：样本由 **Rust serde** 写出，
//! 再由 `python3 scripts/gates/validate_schemas.py --samples-dir <dir>` 交给
//! **Python jsonschema** 逐份对账。两个独立实现互相钉住，任意一边漂移都会变红。
//!
//! 文件名前缀 `mcp-tools.` 已映射到 `schemas/mcp-tools.schema.json`
//! （见 `validate_schemas.py` 的 `SAMPLE_SCHEMA_MAP`）。
//!
//! ## 导出什么（12 份）
//!
//! | 文件 | 内容 | 为什么 |
//! | :--- | :--- | :--- |
//! | `mcp-tools.registry.meta.json`（文档样本，非契约实例） | 十个工具的注册表快照（含 scope / 副作用 / 参数 / 错误码） | 工具名集合、`dryRun`、`idempotencyKey` 的机器可读清单 |
//! | `mcp-tools.error-codes.meta.json`（文档样本，非契约实例） | 错误码全集 + **契约缺口清单** | 让"schema 的 7 个 enum 装不下表格的 16 个错误码"这件事可被机器读到 |
//! | `mcp-tools.call.<tool>.json` ×10 | 每个工具一份**规范 `ToolCall`** | 每个工具名都要被契约的 enum 认下来 |
//!
//! ## 契约现在是**承重**的（ADR-0001 D25）
//!
//! 第一轮实测出来的事实是：本 schema 的根当时只有 `{"type":"object","definitions":{…}}`，
//! 没有 `properties` / `$ref` / `allOf` —— Draft 2020-12 下"任意对象"都通过根校验，
//! `definitions.*` **从未被引用**，`--samples-dir` 对账是空转的。
//!
//! 集成者按 **ADR-0001 D25** 把根改成 `oneOf($ref ToolCall, $ref ToolResponse)`，
//! 并顺手把错误码 enum 扩成联集 20 值。于是**每一个样本都必须真的是 `ToolCall` 或
//! `ToolResponse`**，否则 `validate_schemas.py --samples-dir` 立刻变红：
//!
//! - 10 份 `mcp-tools.call.<tool>.json` 是 `ToolCall`；
//! - 注册表与错误码目录装进 `ToolResponse.data`（`{"status":"success","data":{…}}`）——
//!   它们的原始形状（裸对象）在新根下**会被拒**，这是契约变承重后的直接后果。
//!
//! `tests/contract.rs::contract_rejects_a_deliberately_invalid_sample` 把"契约承重"
//! 这件事本身也钉成了判据（故意违法的样本必须让 `--samples-dir` 变红）。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

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
    let mut names = vec![REGISTRY_FILE.to_owned(), ERROR_CODES_FILE.to_owned()];
    names.extend(TOOLS.iter().map(|spec| call_file(spec.name)));
    names
}

/// 注册表样本：工具名集合 / 参数 / 作用域 / 副作用 / 错误码的机器可读快照。
///
/// 外层是 `ToolResponse`（`{"status":"success","data":{…}}`）—— 契约的根是
/// `oneOf(ToolCall, ToolResponse)`，裸对象会被拒（ADR-0001 D25）。
#[must_use]
pub fn registry_sample() -> Value {
    tool_response(registry_payload())
}

/// 注册表的载荷本体。
fn registry_payload() -> Value {
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
    tool_response(error_codes_payload())
}

/// 错误码目录的载荷本体。
fn error_codes_payload() -> Value {
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

/// 把一个载荷包成 `ToolResponse`（契约根要求的两种形状之一）。
fn tool_response(data: Value) -> Value {
    let mut root = Map::new();
    root.insert("status".to_owned(), Value::from("success"));
    root.insert("data".to_owned(), data);
    Value::Object(root)
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
        "stylePreset" => Value::from("cinematic-orchestral"),
        "scale" => Value::from("C minor"),
        "commitMessage" => Value::from("AI 提案: 副歌加一层八度"),
        "reason" => Value::from("织体过密"),
        "fields" => Value::Array(vec![Value::from("tracks"), Value::from("sections")]),
        "ops" => Value::Array(vec![Value::Object(Map::new())]),
        "value" => Value::from(0.5),
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

/// 把 12 份样本写到 `out_dir`，返回实际写出的路径（顺序固定）。
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

    let registry = registry_sample();
    check_tool_response(REGISTRY_FILE, &registry)?;
    written.push(write_json(out_dir, REGISTRY_FILE, &registry)?);
    let error_codes = error_codes_sample();
    check_tool_response(ERROR_CODES_FILE, &error_codes)?;
    written.push(write_json(out_dir, ERROR_CODES_FILE, &error_codes)?);
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

/// `ToolResponse` 样本的 Rust 侧自检：形状必须与 `definitions.ToolResponse` 一致。
///
/// # Errors
///
/// 缺少 `status`、`status` 不在枚举里、`data` 不是对象，或错误响应的 `code` 不在目录里。
pub fn check_tool_response(file: &str, sample: &Value) -> Result<(), SampleExportError> {
    let invalid = |detail: String| SampleExportError::InvalidSample {
        file: file.to_owned(),
        detail,
    };
    let object = sample
        .as_object()
        .ok_or_else(|| invalid("样本必须是 JSON 对象".to_owned()))?;
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
    if let Some(code) = object
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
    {
        let known = ErrorCode::ALL.iter().any(|known| known.as_str() == code);
        if !known {
            return Err(invalid(format!("`error.code` `{code}` 不在错误码目录里")));
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
    fn export_writes_twelve_byte_stable_samples() {
        let dir = temp_dir("stable");
        let written = export_all(&dir).expect("导出样本");
        let names: Vec<String> = written
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, sample_file_names());
        assert_eq!(names.len(), crate::tools::TOOL_COUNT + 2);
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
        assert_eq!(registry["status"], "success", "必须是 ToolResponse 形状");
        let registry = &registry["data"];
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
        // 契约根是 oneOf(ToolCall, ToolResponse): 两份目录样本都必须是 ToolResponse 形状。
        assert_eq!(sample["status"], "success");
        let data = &sample["data"];
        assert_eq!(
            data["schemaContractEnum"].as_array().expect("enum").len(),
            20,
            "ADR-0001 D25 的联集 20 值"
        );
        assert_eq!(
            data["documentedToolCodes"].as_array().expect("enum").len(),
            16,
            "架构 §7.2 表格"
        );
        let schema_only = data["schemaOnlyCodes"].as_array().expect("schema 原有码");
        assert_eq!(schema_only.len(), 4);
        assert!(schema_only.contains(&Value::from("PERMISSION_DENIED")));
        assert_eq!(
            data["implementationOnly"],
            serde_json::json!(["NOT_IMPLEMENTED"])
        );
        assert_eq!(
            data["all"].as_array().expect("all").len(),
            ErrorCode::ALL.len()
        );
        // 字段名里不再有"缺口": 缺口已由 D25 关闭。
        assert!(data.get("missingFromSchema").is_none());
    }

    #[test]
    fn both_catalogue_samples_are_tool_responses() {
        for (file, sample) in [
            (REGISTRY_FILE, registry_sample()),
            (ERROR_CODES_FILE, error_codes_sample()),
        ] {
            check_tool_response(file, &sample).expect("样本必须自洽");
            assert!(sample.get("name").is_none(), "不得同时长得像 ToolCall");
            assert!(sample["data"].is_object());
        }
        // 反例: 坏 status / 坏 code 必须被自检拦下。
        assert!(check_tool_response("x.json", &serde_json::json!({"status": "maybe"})).is_err());
        assert!(
            check_tool_response(
                "x.json",
                &serde_json::json!({"status": "error", "error": {"code": "NOT_A_CODE"}})
            )
            .is_err()
        );
        assert!(
            check_tool_response(
                "x.json",
                &serde_json::json!({"status": "success", "data": []})
            )
            .is_err()
        );
        assert!(check_tool_response("x.json", &serde_json::json!({})).is_err());
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
        // 供 `validate_schemas.py --samples-dir` 对账 (CI 的接线归集成者)。
        let written = export_to_default_dir().expect("导出到 target/schema-samples");
        assert_eq!(written.len(), crate::tools::TOOL_COUNT + 2);
    }
}
