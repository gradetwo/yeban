//! MCP 失败信封的恢复字段（T-FIX-561）。
//!
//! 保留 `code/message` 兼容形状，只往 `data` 里补：reason、定位字段、
//! received/expected、accepted/candidates、stateChanged、retryable、nextStep。

use serde_json::{Map, Value, json};

use crate::domain::Domain;
use crate::jsonrpc::{ErrorObject, INVALID_PARAMS, TOOL_NOT_FOUND};
use crate::tools::{ToolCall, ToolCallError, ToolSpec, tool, tool_names};

/// 候选清单上限。
pub const MAX_CANDIDATES: usize = 5;
/// `accepted` 值清单上限。
pub const MAX_ACCEPTED: usize = 8;

/// `tools/call` 的请求层失败 → 带恢复字段的 JSON-RPC 错误。
#[must_use]
pub fn tool_call_error(
    error: &ToolCallError,
    params: Option<&Value>,
    domain: &Domain,
) -> ErrorObject {
    let (code, message) = match error {
        ToolCallError::UnknownTool { name } => (TOOL_NOT_FOUND, format!("未知工具 `{name}`")),
        other => (INVALID_PARAMS, other.to_string()),
    };
    let mut data = Map::new();
    data.insert("detail".to_owned(), Value::from(message.clone()));
    data.insert("stateChanged".to_owned(), Value::Bool(false));
    data.insert("retryable".to_owned(), Value::Bool(false));

    let params_object = params.and_then(Value::as_object);
    let tool_name = params_object
        .and_then(|object| object.get("name"))
        .and_then(Value::as_str);
    let arguments = params_object
        .and_then(|object| object.get("arguments"))
        .and_then(Value::as_object);
    let spec = tool_name.and_then(tool);

    match error {
        ToolCallError::ParamsNotAnObject => {
            reason(&mut data, "paramsMustBeObject");
            location(&mut data, "params");
            data.insert("received".to_owned(), type_name(params));
            data.insert("expected".to_owned(), Value::from("object"));
            data.insert(
                "nextStep".to_owned(),
                json!({"action": "把 params 改成 {\"name\":\"<tool>\",\"arguments\":{...}} 再调用"}),
            );
        }
        ToolCallError::MissingName => {
            reason(&mut data, "missingToolName");
            location(&mut data, "params.name");
            data.insert("expected".to_owned(), Value::from("string"));
            data.insert(
                "nextStep".to_owned(),
                json!({"action": "在 params.name 填一个工具名；不确定时先调 tools/list"}),
            );
        }
        ToolCallError::MissingArguments => {
            reason(&mut data, "missingArguments");
            location(&mut data, "params.arguments");
            data.insert("expected".to_owned(), Value::from("object"));
            let template = spec.map_or_else(Map::new, |spec| minimal_arguments(spec, None, domain));
            data.insert(
                "nextStep".to_owned(),
                next_step_with_arguments(tool_name.unwrap_or(""), template),
            );
        }
        ToolCallError::ArgumentsNotAnObject => {
            reason(&mut data, "argumentsMustBeObject");
            location(&mut data, "params.arguments");
            data.insert(
                "received".to_owned(),
                type_name(params_object.and_then(|object| object.get("arguments"))),
            );
            data.insert("expected".to_owned(), Value::from("object"));
            let template = spec.map_or_else(Map::new, |spec| minimal_arguments(spec, None, domain));
            data.insert(
                "nextStep".to_owned(),
                next_step_with_arguments(tool_name.unwrap_or(""), template),
            );
        }
        ToolCallError::UnknownTool { name } => {
            reason(&mut data, "unknownTool");
            location(&mut data, "params.name");
            data.insert("received".to_owned(), Value::from(name.as_str()));
            let names = tool_names();
            let candidates = nearest(name, &names, MAX_CANDIDATES);
            data.insert(
                "candidates".to_owned(),
                Value::Array(
                    candidates
                        .iter()
                        .map(|candidate| Value::from(*candidate))
                        .collect(),
                ),
            );
            data.insert(
                "expected".to_owned(),
                Value::from("one of the registered yeban_* tool names"),
            );
            data.insert(
                "nextStep".to_owned(),
                candidates.first().map_or_else(
                    || {
                        json!({
                            "action": "调 tools/list 取注册工具名，再从 candidates 里选一个重试",
                            "tool": "tools/list",
                            "arguments": {}
                        })
                    },
                    |candidate| {
                        json!({
                            "action": "用最近邻工具重试；参数先按该工具的 inputSchema 补齐",
                            "tool": candidate,
                            "arguments": {}
                        })
                    },
                ),
            );
        }
        ToolCallError::MissingParam { name } => {
            reason(&mut data, "missingParam");
            location(&mut data, format!("arguments.{name}"));
            data.insert(
                "expected".to_owned(),
                Value::from(expected_type(spec, name)),
            );
            let template =
                spec.map_or_else(Map::new, |spec| minimal_arguments(spec, arguments, domain));
            data.insert(
                "nextStep".to_owned(),
                next_step_with_arguments(tool_name.unwrap_or(""), template),
            );
        }
        ToolCallError::InvalidParam { name, expected } => {
            reason(&mut data, "invalidParamType");
            location(&mut data, format!("arguments.{name}"));
            data.insert(
                "received".to_owned(),
                type_name(arguments.and_then(|object| object.get(name))),
            );
            data.insert("expected".to_owned(), Value::from(*expected));
            let template = spec.map_or_else(Map::new, |spec| {
                corrected_argument(spec, arguments, name, domain)
            });
            data.insert(
                "nextStep".to_owned(),
                next_step_with_arguments(tool_name.unwrap_or(""), template),
            );
        }
        ToolCallError::ParamLengthOutOfRange { name, min, max } => {
            reason(&mut data, "paramLengthOutOfRange");
            location(&mut data, format!("arguments.{name}"));
            data.insert(
                "received".to_owned(),
                Value::from(
                    arguments
                        .and_then(|object| object.get(name))
                        .and_then(Value::as_str)
                        .map_or(0, |text| text.chars().count()),
                ),
            );
            data.insert(
                "expected".to_owned(),
                json!({"minLength": min, "maxLength": max}),
            );
            let template = spec.map_or_else(Map::new, |spec| {
                corrected_length_argument(spec, arguments, name, *min, domain)
            });
            data.insert(
                "nextStep".to_owned(),
                next_step_with_arguments(tool_name.unwrap_or(""), template),
            );
        }
        ToolCallError::ConflictingParamAlias { name, alias } => {
            reason(&mut data, "conflictingParamAlias");
            location(&mut data, format!("arguments.{alias}"));
            data.insert("received".to_owned(), Value::from(alias.as_str()));
            data.insert(
                "expected".to_owned(),
                Value::from(format!("只给 `{name}` 或 `{alias}` 其中一个")),
            );
            data.insert(
                "nextStep".to_owned(),
                json!({
                    "action": format!("删掉 `{alias}` 或 `{name}` 中一个；两者值不同时不猜"),
                }),
            );
        }
        ToolCallError::UnknownParam { name } => {
            reason(&mut data, "unknownParam");
            location(&mut data, format!("arguments.{name}"));
            data.insert("received".to_owned(), Value::from(name.as_str()));
            let accepted = spec.map_or_else(Vec::new, |spec| {
                spec.all_params()
                    .iter()
                    .flat_map(|param| {
                        std::iter::once(param.name.to_owned())
                            .chain(param.aliases.iter().map(|alias| (*alias).to_owned()))
                    })
                    .collect::<Vec<_>>()
            });
            let accepted_refs: Vec<&str> = accepted.iter().map(String::as_str).collect();
            let candidates = nearest(name, &accepted_refs, MAX_CANDIDATES);
            data.insert(
                "accepted".to_owned(),
                Value::Array(
                    accepted
                        .iter()
                        .take(MAX_ACCEPTED)
                        .map(|value| Value::from(value.as_str()))
                        .collect(),
                ),
            );
            data.insert(
                "candidates".to_owned(),
                Value::Array(
                    candidates
                        .iter()
                        .map(|candidate| Value::from(*candidate))
                        .collect(),
                ),
            );
            data.insert(
                "expected".to_owned(),
                Value::from("one of accepted argument names"),
            );
            let template = spec.map_or_else(Map::new, |spec| {
                minimal_arguments_without(spec, arguments, name, domain)
            });
            data.insert(
                "nextStep".to_owned(),
                next_step_with_arguments(tool_name.unwrap_or(""), template),
            );
        }
    }

    ErrorObject::new(code, message).with_data(Value::Object(data))
}

/// 领域失败（`ToolResponse.status == "error"`）→ 就地补恢复字段。
pub fn enrich_tool_value(mut value: Value, call: &ToolCall, domain: &Domain) -> Value {
    if value.get("status").and_then(Value::as_str) != Some("error") {
        return value;
    }
    let Some(error) = value.get_mut("error").and_then(Value::as_object_mut) else {
        return value;
    };
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let data_value = error
        .entry("data")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(data) = data_value.as_object_mut() else {
        return value;
    };
    data.insert("detail".to_owned(), Value::from(message));
    data.entry("reason".to_owned())
        .or_insert_with(|| Value::from(reason_for_code(&code)));
    data.entry("stateChanged".to_owned())
        .or_insert(Value::Bool(false));
    data.entry("retryable".to_owned())
        .or_insert(Value::Bool(false));

    let path = infer_path(&code, data, call);
    if !data.contains_key("path")
        && !data.contains_key("field")
        && let Some(path) = path.as_deref()
    {
        location(data, path);
    }
    let received = data
        .get("received")
        .cloned()
        .or_else(|| data.get("value").cloned())
        .or_else(|| {
            path.as_deref()
                .and_then(|path| argument_at(call, path))
                .cloned()
        })
        .unwrap_or(Value::Null);
    data.entry("received".to_owned()).or_insert(received);
    if !data.contains_key("expected")
        && let Some(expected) = expected_for(&code, data, call, path.as_deref())
    {
        data.insert("expected".to_owned(), expected);
    }

    let accepted = accepted_values(data, call, path.as_deref());
    if !accepted.is_empty() && !data.contains_key("accepted") {
        data.insert(
            "accepted".to_owned(),
            Value::Array(accepted.iter().take(MAX_ACCEPTED).cloned().collect()),
        );
    }
    let candidates = entity_candidates(&code, data, domain);
    if !candidates.is_empty() && !data.contains_key("candidates") {
        data.insert(
            "candidates".to_owned(),
            Value::Array(candidates.into_iter().take(MAX_CANDIDATES).collect()),
        );
    }
    if !data.contains_key("nextStep") {
        data.insert(
            "nextStep".to_owned(),
            next_step_for_domain(call, &code, data, path.as_deref(), &accepted, domain),
        );
    }
    value
}

/// 实现级 JSON-RPC 失败 → 也补最小恢复字段（不伪装成领域失败）。
#[must_use]
pub fn enrich_impl_error(mut error: ErrorObject, call: &ToolCall) -> ErrorObject {
    let mut data = error
        .data
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    data.entry("reason".to_owned())
        .or_insert(Value::from("implementationError"));
    data.entry("stateChanged".to_owned())
        .or_insert(Value::Bool(false));
    data.entry("retryable".to_owned())
        .or_insert(Value::Bool(true));
    data.entry("detail".to_owned())
        .or_insert(Value::from(error.message.clone()));
    data.entry("nextStep".to_owned()).or_insert(json!({
        "action": format!("先查 error.data.detail；确认宿主/平台条件后重试 {}", call.tool.name)
    }));
    error.with_data(Value::Object(data))
}

fn reason(data: &mut Map<String, Value>, value: &str) {
    data.insert("reason".to_owned(), Value::from(value));
}

fn location(data: &mut Map<String, Value>, path: impl Into<String>) {
    let path = path.into();
    data.insert("field".to_owned(), Value::from(path.clone()));
    data.insert("path".to_owned(), Value::from(path));
}

fn reason_for_code(code: &str) -> String {
    match code {
        "NO_ACTIVE_PROJECT" => "noActiveProject".to_owned(),
        "TRACK_NOT_FOUND" => "trackNotFound".to_owned(),
        "CLIP_NOT_FOUND" => "clipNotFound".to_owned(),
        "PROPOSAL_NOT_FOUND" => "proposalNotFound".to_owned(),
        "STYLE_NOT_FOUND" => "styleNotFound".to_owned(),
        "ENTITY_NOT_FOUND" => "entityNotFound".to_owned(),
        "INVALID_PARAMETER_RANGE" => "invalidParameterRange".to_owned(),
        "OUT_OF_RANGE" => "outOfRange".to_owned(),
        "CONFLICT" => "conflict".to_owned(),
        "PERMISSION_DENIED" => "permissionDenied".to_owned(),
        "BUSY" => "busy".to_owned(),
        other => {
            let mut result = String::new();
            for (index, word) in other.split('_').enumerate() {
                if index == 0 {
                    result.push_str(&word.to_ascii_lowercase());
                } else {
                    let mut chars = word.chars();
                    if let Some(first) = chars.next() {
                        result.push(first);
                        result.push_str(&chars.as_str().to_ascii_lowercase());
                    }
                }
            }
            result
        }
    }
}

fn infer_path(code: &str, data: &Map<String, Value>, call: &ToolCall) -> Option<String> {
    if let Some(path) = data
        .get("path")
        .or_else(|| data.get("field"))
        .and_then(Value::as_str)
    {
        return Some(path.to_owned());
    }
    let reason = data.get("reason").and_then(Value::as_str).unwrap_or("");
    match code {
        "TRACK_NOT_FOUND" => Some("trackId".to_owned()),
        "CLIP_NOT_FOUND" => Some("clipId".to_owned()),
        "PROPOSAL_NOT_FOUND" => Some("proposalId".to_owned()),
        "ENTITY_NOT_FOUND" => match reason {
            "trackNotFound" => Some("trackId".to_owned()),
            "clipNotFound" => Some("clipId".to_owned()),
            "sceneNotFound" => Some("sceneId".to_owned()),
            "sectionNotFound" => Some("sectionId".to_owned()),
            _ => None,
        },
        "STYLE_NOT_FOUND" => Some("stylePreset".to_owned()),
        "NO_ACTIVE_PROJECT" => Some("project".to_owned()),
        _ => {
            let key = data
                .get("key")
                .and_then(Value::as_str)
                .or_else(|| data.get("parameter").and_then(Value::as_str))?;
            if call.arguments.contains_key(key) {
                Some(key.to_owned())
            } else {
                None
            }
        }
    }
}

fn expected_type(spec: Option<&ToolSpec>, name: &str) -> &'static str {
    spec.and_then(|spec| spec.param(name))
        .map_or("value of the declared type", |param| param.json_type)
}

fn expected_for(
    code: &str,
    data: &Map<String, Value>,
    call: &ToolCall,
    path: Option<&str>,
) -> Option<Value> {
    if let Some(expected) = data.get("expected") {
        return Some(expected.clone());
    }
    match code {
        "TRACK_NOT_FOUND" | "CLIP_NOT_FOUND" | "PROPOSAL_NOT_FOUND" | "ENTITY_NOT_FOUND" => {
            Some(Value::from("id of an existing entity"))
        }
        "STYLE_NOT_FOUND" => Some(Value::from("one of accepted stylePreset values")),
        "NO_ACTIVE_PROJECT" => Some(Value::from("an opened project")),
        "OUT_OF_RANGE" => {
            let min = data.get("min").cloned().unwrap_or(Value::Null);
            let max = data.get("max").cloned().unwrap_or(Value::Null);
            Some(json!({"min": min, "max": max}))
        }
        "INVALID_PARAMETER_RANGE" => {
            let accepted = accepted_values(data, call, path);
            if accepted.is_empty() {
                Some(Value::from("a value that satisfies the tool schema"))
            } else {
                Some(json!({"oneOf": accepted.into_iter().take(MAX_ACCEPTED).collect::<Vec<_>>()}))
            }
        }
        _ => None,
    }
}

fn accepted_values(data: &Map<String, Value>, call: &ToolCall, path: Option<&str>) -> Vec<Value> {
    for key in [
        "accepted",
        "allowed",
        "availablePresets",
        "allowedLicenses",
        "availableSelectors",
        "supportedValues",
        "validValues",
    ] {
        if let Some(value) = data.get(key) {
            if let Some(array) = value.as_array() {
                return array.iter().take(MAX_ACCEPTED).cloned().collect();
            }
            if value.is_string() || value.is_number() {
                return vec![value.clone()];
            }
        }
    }
    let Some(path) = path else {
        return Vec::new();
    };
    let field = path.rsplit('.').next().unwrap_or(path);
    call.tool
        .param(field)
        .and_then(|param| param.enum_values)
        .map(|values| values().into_iter().map(Value::from).collect())
        .unwrap_or_default()
}

fn entity_candidates(code: &str, data: &Map<String, Value>, domain: &Domain) -> Vec<Value> {
    if let Some(candidates) = data.get("candidates").and_then(Value::as_array) {
        return candidates.iter().take(MAX_CANDIDATES).cloned().collect();
    }
    if let Some(known) = data.get("knownProposals").and_then(Value::as_array) {
        return known
            .iter()
            .take(MAX_CANDIDATES)
            .map(|proposal| {
                json!({
                    "id": proposal.get("proposalId").cloned().unwrap_or(Value::Null),
                    "name": proposal.get("title").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
    }
    let Some(project) = domain.active_project() else {
        return Vec::new();
    };
    match code {
        "TRACK_NOT_FOUND" => project
            .tracks
            .values()
            .take(MAX_CANDIDATES)
            .map(|track| {
                json!({
                    "id": track.id.to_canonical_string(),
                    "name": track.name,
                })
            })
            .collect(),
        "CLIP_NOT_FOUND" => project
            .tracks
            .values()
            .flat_map(|track| {
                track.clips.values().map(|placement| {
                    json!({
                        "id": placement.clip_id.to_canonical_string(),
                        "name": format!("{} · {}", track.name, placement.start_tick),
                    })
                })
            })
            .take(MAX_CANDIDATES)
            .collect(),
        "PROPOSAL_NOT_FOUND" => domain
            .open_proposals()
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|proposal| {
                json!({
                    "id": proposal.id.to_canonical_string(),
                    "name": proposal.title,
                })
            })
            .collect(),
        "ENTITY_NOT_FOUND" => match data.get("reason").and_then(Value::as_str) {
            Some("trackNotFound") => project
                .tracks
                .values()
                .take(MAX_CANDIDATES)
                .map(|track| {
                    json!({
                        "id": track.id.to_canonical_string(),
                        "name": track.name,
                    })
                })
                .collect(),
            Some("sceneNotFound") => project
                .scenes
                .values()
                .take(MAX_CANDIDATES)
                .map(|scene| {
                    json!({
                        "id": scene.id.to_canonical_string(),
                        "name": scene.name,
                    })
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// 从缺字段的 `detail` 文案里取 `(字段名, 期望类型)`。
///
/// 领域层的 `notes::missing()` 刻意**不带 data**（`Fault` 的既有形状不能为一个
/// 恢复提示改契约）；`enrich_tool_value` 已把同一条 message 写进 `data.detail`，
/// 恢复层因此只读文案、不反向依赖领域层加字段。文案由 `notes::missing()`
/// 单点生成：`缺少 `{field}`（期望 {expected}）`。
fn missing_field_from_detail(detail: &str) -> Option<(String, String)> {
    let rest = detail.strip_prefix("缺少 `")?;
    let (field, tail) = rest.split_once("`（期望 ")?;
    let expected = tail.strip_suffix('）')?;
    if field.is_empty() || expected.is_empty() {
        return None;
    }
    Some((field.to_owned(), expected.to_owned()))
}

fn next_step_for_domain(
    call: &ToolCall,
    code: &str,
    data: &Map<String, Value>,
    path: Option<&str>,
    accepted: &[Value],
    domain: &Domain,
) -> Value {
    let missing = data
        .get("missingField")
        .and_then(Value::as_str)
        .map(|field| {
            let expected = data
                .get("missingFieldExpected")
                .and_then(Value::as_str)
                .unwrap_or("该字段声明的类型");
            (field.to_owned(), expected.to_owned())
        })
        .or_else(|| {
            data.get("detail")
                .and_then(Value::as_str)
                .and_then(missing_field_from_detail)
        });
    if let Some((field, expected)) = missing {
        let template = minimal_arguments(call.tool, Some(&call.arguments), domain);
        // 操作层缺字段的模板**不能**假装可直接执行：`minimal_arguments` 只补顶层
        // 必填，`ops` 元素里缺的键仍原样回显。这里把「手补哪个键」写进 action，
        // 与工具参数层 `missingParam` 的“最小合法模板可直接执行”明确分口径。
        return json!({
            "tool": call.tool.name,
            "arguments": template,
            "action": format!(
                "在 `ops` 元素中手补缺失键 `{field}`（期望 {expected}）后重试；不要原样重放本次模板"
            )
        });
    }
    if data.get("reason").and_then(Value::as_str) == Some("opsValidationFailed") {
        let mut template = minimal_arguments(call.tool, Some(&call.arguments), domain);
        template.insert("ops".to_owned(), json!([{"kind": "setTempo", "bpm": 120}]));
        return json!({
            "tool": call.tool.name,
            "arguments": template,
            "action": "按 error.data.errors[] 的 opIndex/field 修正每一条后重试"
        });
    }
    match code {
        "TRACK_NOT_FOUND" | "CLIP_NOT_FOUND" | "ENTITY_NOT_FOUND" => json!({
            "tool": "yeban_query_project",
            "arguments": {},
            "action": "先读当前工程的 entities/tracks/clips，确认目标 id 后重试"
        }),
        "PROPOSAL_NOT_FOUND" => json!({
            "tool": "yeban_list_proposals",
            "arguments": {},
            "action": "先列未合并提案，确认 proposalId 后重试"
        }),
        "NO_ACTIVE_PROJECT" => json!({
            "tool": "yeban_open_project",
            "arguments": {"path": "<project path>"},
            "action": "先打开目标工程再调用"
        }),
        "STYLE_NOT_FOUND" => {
            if accepted.is_empty() {
                json!({"tool": "yeban_query_project", "arguments": {}, "action": "先查工程可用样式"})
            } else {
                next_step_with_arguments(
                    call.tool.name,
                    corrected_with_value(call, path, accepted[0].clone()),
                )
            }
        }
        "INVALID_PARAMETER_RANGE" if !accepted.is_empty() && path.is_some() => {
            next_step_with_arguments(
                call.tool.name,
                corrected_with_value(call, path, accepted[0].clone()),
            )
        }
        _ => {
            let template = minimal_arguments(call.tool, Some(&call.arguments), domain);
            json!({
                "tool": call.tool.name,
                "arguments": template,
                "action": "按 error.data 的 reason/field/received/expected 修正后重试"
            })
        }
    }
}

fn next_step_with_arguments(tool_name: &str, arguments: Map<String, Value>) -> Value {
    json!({
        "tool": tool_name,
        "arguments": Value::Object(arguments),
    })
}

fn corrected_with_value(call: &ToolCall, path: Option<&str>, value: Value) -> Map<String, Value> {
    let mut arguments = call.arguments.clone();
    if let Some(path) = path {
        let key = path
            .strip_prefix("arguments.")
            .unwrap_or(path)
            .rsplit('.')
            .next()
            .unwrap_or(path);
        arguments.insert(key.to_owned(), value);
    }
    arguments
}

fn corrected_argument(
    spec: &ToolSpec,
    arguments: Option<&Map<String, Value>>,
    name: &str,
    domain: &Domain,
) -> Map<String, Value> {
    let mut template = minimal_arguments(spec, arguments, domain);
    if let Some(param) = spec.param(name) {
        // `placeholder` 对 array 返回 `[]`；ops 必须换成可执行最小模板，
        // 否则 corrected_argument 会把 minimal_arguments 刚补好的模板又覆写回空数组。
        let value = if spec.name == "yeban_edit_notes" && name == "ops" {
            minimal_ops_argument()
        } else {
            placeholder(param, domain)
        };
        template.insert(name.to_owned(), value);
    }
    template
}

fn corrected_length_argument(
    spec: &ToolSpec,
    arguments: Option<&Map<String, Value>>,
    name: &str,
    min: Option<usize>,
    domain: &Domain,
) -> Map<String, Value> {
    let mut template = minimal_arguments(spec, arguments, domain);
    template.insert(
        name.to_owned(),
        Value::from("x".repeat(min.unwrap_or(1).max(1))),
    );
    template
}

fn minimal_arguments_without(
    spec: &ToolSpec,
    arguments: Option<&Map<String, Value>>,
    unknown: &str,
    domain: &Domain,
) -> Map<String, Value> {
    let mut template = minimal_arguments(spec, arguments, domain);
    template.remove(unknown);
    template
}

/// `yeban_edit_notes` 的 `ops` 最小可执行模板：一条合法的 `add` 音符操作。
/// ⛔ 不能用空数组——领域层 `parse_ops` 对 `ops:[]` 响亮拒绝，
/// 而 nextStep 原样重放空数组会形成「错误→按提示重试→再错误」的自愈死循环。
fn minimal_ops_argument() -> Value {
    json!([{"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}])
}

/// 「没填」与「填了但为空数组」都要视为待补：只判断 key 是否存在会把 `ops:[]`
/// 当成已填，从而让 nextStep 携带一个原样重放必败的空数组。
fn ops_is_unfilled(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::Array(items)) => items.is_empty(),
        _ => false,
    }
}

fn minimal_arguments(
    spec: &ToolSpec,
    arguments: Option<&Map<String, Value>>,
    domain: &Domain,
) -> Map<String, Value> {
    let mut template = Map::new();
    if let Some(arguments) = arguments {
        for (key, value) in arguments {
            if spec.param(key).is_some() {
                template.insert(key.clone(), value.clone());
            }
        }
    }
    for param in spec.required_params() {
        if !template.contains_key(param.name) {
            template.insert(param.name.to_owned(), placeholder(param, domain));
        }
    }
    if spec.name == "yeban_edit_notes" && ops_is_unfilled(template.get("ops")) {
        template.insert("ops".to_owned(), minimal_ops_argument());
    }
    template
}

fn placeholder(param: &crate::tools::ParamSpec, domain: &Domain) -> Value {
    if let Some(values) = param.enum_values
        && let Some(first) = values().into_iter().next()
    {
        return Value::from(first);
    }
    if let Some(project) = domain.active_project() {
        match param.name {
            "trackId" => {
                if let Some(track) = project.tracks.values().next() {
                    return Value::from(track.id.to_canonical_string());
                }
            }
            "clipId" => {
                if let Some(placement) = project
                    .tracks
                    .values()
                    .find_map(|track| track.clips.values().next())
                {
                    return Value::from(placement.id.to_canonical_string());
                }
            }
            "sceneId" => {
                if let Some(scene) = project.scenes.values().next() {
                    return Value::from(scene.id.to_canonical_string());
                }
            }
            _ => {}
        }
    }
    if param.name == "proposalId"
        && let Some(proposal) = domain.open_proposals().into_iter().next()
    {
        return Value::from(proposal.id.to_canonical_string());
    }
    match param.json_type {
        "boolean" => Value::Bool(false),
        "integer" | "number" => Value::from(0),
        "array" => json!([]),
        "object" => json!({}),
        _ => Value::from("<replace>"),
    }
}

fn type_name(value: Option<&Value>) -> Value {
    Value::from(match value {
        None | Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    })
}

fn argument_at<'a>(call: &'a ToolCall, path: &str) -> Option<&'a Value> {
    let key = path
        .strip_prefix("arguments.")
        .unwrap_or(path)
        .rsplit('.')
        .next()
        .unwrap_or(path);
    call.arguments.get(key)
}

fn nearest<'a>(needle: &str, candidates: &'a [&str], limit: usize) -> Vec<&'a str> {
    let mut scored: Vec<(usize, &str)> = candidates
        .iter()
        .map(|candidate| (edit_distance(needle, candidate), *candidate))
        .filter(|(distance, _)| *distance <= 4)
        .collect();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, candidate)| candidate)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (i, left_char) in left.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, right_char) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            let insertion = current[j] + 1;
            let deletion = previous[j + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}
