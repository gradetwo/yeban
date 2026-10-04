//! JSON-RPC 2.0 的最小实现（手写枚举 + 手写 `Serialize`）[MCP-TOOL-001..010 的传输外壳]。
//!
//! 为什么手写而不是引入 `jsonrpc-core` 之类的 crate（`docs/adr/ADR-0001` D20/D21）：
//!
//! 1. **形状是规范**。MCP 客户端对 `jsonrpc` 字段的字面值、`id` 的**回显**、
//!    以及"`result` 与 `error` 恰好出现一个"都有硬要求。手写 `Serialize` 能把这些
//!    不变量钉在类型上，而不是依赖第三方 crate 的默认行为（它可能多写一个
//!    `"error": null`，也可能把 `id` 丢掉）。
//! 2. **零新增依赖**。本 crate 只需要 `serde_json` 的 `Value`。
//!
//! 覆盖范围（刻意最小）：
//!
//! | 能力 | 状态 |
//! | :--- | :--- |
//! | 单个请求 / 响应 | 支持 |
//! | `id` 回显（number / string / null） | 支持 |
//! | 错误对象 `code` / `message` / `data` | 支持 |
//! | notification（**没有** `id` 键） | 支持（**不产生响应**，符合规范） |
//! | 批处理（顶层 JSON 数组） | **不支持**，明确返回 `-32600` |
//! | 位置参数（`params` 是数组） | 解析放行、由 [`crate::dispatch`] 判 `-32602` |

use std::fmt;

use serde::ser::SerializeMap as _;
use serde_json::Value;

/// 规范要求的协议版本字面值。
pub const JSONRPC_VERSION: &str = "2.0";

/// 解析错误（不是合法 JSON）。
pub const PARSE_ERROR: i64 = -32700;
/// 非法请求（是 JSON，但不是合法 JSON-RPC 2.0 请求）。
pub const INVALID_REQUEST: i64 = -32600;
/// 方法不存在。
pub const METHOD_NOT_FOUND: i64 = -32601;
/// 参数非法。
pub const INVALID_PARAMS: i64 = -32602;
/// 服务内部错误。
pub const INTERNAL_ERROR: i64 = -32603;
/// 未鉴权（缺 token / 错 token）。属于 `-32000..=-32099` 的服务自定义区。
pub const UNAUTHORIZED: i64 = -32001;
/// 已鉴权但未授权（scope 不足 / 生产模式硬禁）。
pub const FORBIDDEN: i64 = -32003;
/// 工具不存在。
pub const TOOL_NOT_FOUND: i64 = -32004;
/// 工具已注册但**尚未实现**（本轮的诚实状态，见 `docs/ledger/mcp-core-notes.md`）。
pub const NOT_IMPLEMENTED: i64 = -32005;

/// JSON-RPC 请求 / 响应标识。
///
/// 规范只允许 string / number / null 三种。这里手写 `Serialize` / `Deserialize`，
/// 把"回显"这件事变成类型行为。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Id {
    /// 整数标识。
    Number(i64),
    /// 字符串标识（MCP 客户端常用）。
    Text(String),
    /// `null` 标识（显式出现，或请求没有 `id` 时的响应占位）。
    Null,
}

impl Id {
    /// 由 JSON 值解析；`None` 表示这个值不是合法的 `id`。
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_i64().map(Self::Number),
            Value::String(text) => Some(Self::Text(text.clone())),
            Value::Null => Some(Self::Null),
            _ => None,
        }
    }

    /// 转成 JSON 值。
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Number(number) => Value::from(*number),
            Self::Text(text) => Value::from(text.clone()),
            Self::Null => Value::Null,
        }
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(f, "{number}"),
            Self::Text(text) => write!(f, "{text}"),
            Self::Null => f.write_str("null"),
        }
    }
}

impl serde::Serialize for Id {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Number(number) => serializer.serialize_i64(*number),
            Self::Text(text) => serializer.serialize_str(text),
            Self::Null => serializer.serialize_unit(),
        }
    }
}

/// 错误对象：`code` + `message` 必填，`data` 可选。
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorObject {
    /// 数值错误码。规范内置码是 `-32700..=-32600`，服务自定义码是 `-32000..=-32099`。
    pub code: i64,
    /// 人话信息（不参与机器判定）。
    pub message: String,
    /// 结构化补充（机器判定放这里）。
    pub data: Option<Value>,
}

impl ErrorObject {
    /// 只要 `code` + `message`。
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// 带上结构化 `data`。
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// 解析错误（`id` 一定是 `null`）。
    #[must_use]
    pub fn parse_error(detail: impl Into<String>) -> Self {
        Self::new(PARSE_ERROR, "JSON 解析失败").with_data(json_data(detail.into(), None))
    }

    /// 非法请求。
    #[must_use]
    pub fn invalid_request(detail: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, "非法 JSON-RPC 2.0 请求")
            .with_data(json_data(detail.into(), None))
    }

    /// 参数非法。
    #[must_use]
    pub fn invalid_params(detail: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, "参数非法").with_data(json_data(detail.into(), None))
    }

    /// 方法不存在。
    #[must_use]
    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("方法 `{method}` 不存在"))
            .with_data(json_data(format!("未知方法 `{method}`"), None))
    }

    /// 工具不存在。
    #[must_use]
    pub fn tool_not_found(tool: &str) -> Self {
        Self::new(TOOL_NOT_FOUND, format!("工具 `{tool}` 不在契约工具集中"))
            .with_data(json_data(format!("未知工具 `{tool}`"), None))
    }

    /// 尚未实现。
    #[must_use]
    pub fn not_implemented(tool: &str, spec_id: &str) -> Self {
        Self::new(
            NOT_IMPLEMENTED,
            format!("工具 `{tool}` 的分发链路已就绪, 领域实现尚未接线"),
        )
        .with_data(json_data(
            format!("`{tool}` ({spec_id}) 的真实领域实现待 MCP-TOOL 能力切片接线"),
            Some("NOT_IMPLEMENTED"),
        ))
    }
}

/// 构造错误对象的 `data`：`{"detail": ..., ["code": ...]}`。
fn json_data(detail: String, code: Option<&str>) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("detail".to_owned(), Value::from(detail));
    if let Some(code) = code {
        map.insert("code".to_owned(), Value::from(code));
    }
    Value::Object(map)
}

impl serde::Serialize for ErrorObject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("code", &self.code)?;
        map.serialize_entry("message", &self.message)?;
        if let Some(data) = &self.data {
            map.serialize_entry("data", data)?;
        }
        map.end()
    }
}

impl fmt::Display for ErrorObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

/// 一个已解析的 JSON-RPC 2.0 请求。
///
/// `id == None` 表示这是 **notification**：规范要求在 notification 上**不产生任何响应**。
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// `id`；`None` 表示 notification。
    pub id: Option<Id>,
    /// 方法名。
    pub method: String,
    /// 参数（对象或数组）。
    pub params: Option<Value>,
}

impl Request {
    /// 解析一行 / 一帧 JSON 文本。
    ///
    /// # Errors
    ///
    /// 文本不是合法 JSON ⇒ [`ErrorObject::parse_error`]；是 JSON 但不是合法请求
    /// ⇒ [`ErrorObject::invalid_request`]。
    pub fn parse(text: &str) -> Result<Self, ErrorObject> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| ErrorObject::parse_error(error.to_string()))?;
        Self::from_value(value)
    }

    /// 由 JSON 值解析（严格校验 `jsonrpc` 字面值、`method` 与 `id` 的类型）。
    ///
    /// # Errors
    ///
    /// 见 [`Request::parse`]。
    pub fn from_value(value: Value) -> Result<Self, ErrorObject> {
        if value.is_array() {
            return Err(ErrorObject::invalid_request(
                "本实现不支持 JSON-RPC 批处理 (顶层数组)",
            ));
        }
        let Value::Object(mut object) = value else {
            return Err(ErrorObject::invalid_request("顶层必须是 JSON 对象"));
        };

        match object.get("jsonrpc") {
            Some(Value::String(version)) if version == JSONRPC_VERSION => {}
            Some(Value::String(version)) => {
                return Err(ErrorObject::invalid_request(format!(
                    "jsonrpc 必须是 `{JSONRPC_VERSION}`, 实际 `{version}`"
                )));
            }
            _ => {
                return Err(ErrorObject::invalid_request(format!(
                    "缺少字符串字段 `jsonrpc` (必须恰好是 `{JSONRPC_VERSION}`)"
                )));
            }
        }

        let method = match object.remove("method") {
            Some(Value::String(method)) if !method.is_empty() => method,
            Some(Value::String(_)) => {
                return Err(ErrorObject::invalid_request("`method` 不能是空字符串"));
            }
            _ => return Err(ErrorObject::invalid_request("缺少字符串字段 `method`")),
        };

        let id =
            match object.remove("id") {
                None => None,
                Some(value) => Some(Id::from_value(&value).ok_or_else(|| {
                    ErrorObject::invalid_request("`id` 只能是字符串 / 整数 / null")
                })?),
            };

        let params = match object.remove("params") {
            None => None,
            Some(value @ (Value::Object(_) | Value::Array(_))) => Some(value),
            Some(_) => {
                return Err(ErrorObject::invalid_request("`params` 只能是对象或数组"));
            }
        };

        Ok(Self { id, method, params })
    }

    /// `true` 表示这是 notification（没有 `id` 键），规范要求不回复。
    #[must_use]
    pub const fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// 响应用的 `id`：没有 `id` 时用 [`Id::Null`]。
    #[must_use]
    pub fn response_id(&self) -> Id {
        self.id.clone().unwrap_or(Id::Null)
    }

    /// 对象形式的 `params`（MCP 的 `tools/call` 形状）。
    #[must_use]
    pub fn params_object(&self) -> Option<&serde_json::Map<String, Value>> {
        self.params.as_ref().and_then(Value::as_object)
    }
}

/// 一个 JSON-RPC 2.0 响应。`result` 与 `error` **恰好一个**存在。
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// 被回显的请求 `id`。
    pub id: Id,
    /// 成功结果。
    pub result: Option<Value>,
    /// 失败对象。
    pub error: Option<ErrorObject>,
}

impl Response {
    /// 成功响应。
    #[must_use]
    pub fn success(id: Id, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// 失败响应。
    #[must_use]
    pub fn failure(id: Id, error: ErrorObject) -> Self {
        Self {
            id,
            result: None,
            error: Some(error),
        }
    }

    /// 解析错误响应（`id = null`）。
    #[must_use]
    pub fn parse_error(detail: impl Into<String>) -> Self {
        Self::failure(Id::Null, ErrorObject::parse_error(detail))
    }

    /// 是否失败。
    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// 错误对象。
    #[must_use]
    pub const fn error_object(&self) -> Option<&ErrorObject> {
        self.error.as_ref()
    }

    /// 成功结果。
    #[must_use]
    pub const fn result_value(&self) -> Option<&Value> {
        self.result.as_ref()
    }

    /// 序列化成规范 JSON 文本。
    ///
    /// # Panics
    ///
    /// `serde_json` 序列化 `Value` 不会失败；失败只可能来自我们自己的 `Serialize`
    /// 实现（本类型内不可能）。这里用 `expect` 而不是静默吐一个错误串。
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Response 的 Serialize 不会失败")
    }

    /// 序列化成 JSON 值。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("Response 的 Serialize 不会失败")
    }
}

impl serde::Serialize for Response {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("jsonrpc", JSONRPC_VERSION)?;
        map.serialize_entry("id", &self.id)?;
        if let Some(error) = &self.error {
            map.serialize_entry("error", error)?;
        }
        match &self.result {
            Some(result) => map.serialize_entry("result", result)?,
            // 不变量兜底: 既没有 result 也没有 error 时, 显式写 null 而不是悄悄少一个键。
            None if self.error.is_none() => map.serialize_entry("result", &Value::Null)?,
            None => {}
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_params_and_echoes_string_id() {
        let request = Request::parse(
            r#"{"jsonrpc":"2.0","id":"abc-1","method":"tools/call","params":{"name":"yeban_query_project"}}"#,
        )
        .expect("解析");
        assert_eq!(request.id, Some(Id::Text("abc-1".to_owned())));
        assert_eq!(request.method, "tools/call");
        assert!(!request.is_notification());
        assert_eq!(
            request.params_object().expect("对象参数")["name"],
            "yeban_query_project"
        );

        let response = Response::success(request.response_id(), Value::from(7));
        assert_eq!(
            response.to_json(),
            r#"{"jsonrpc":"2.0","id":"abc-1","result":7}"#
        );
    }

    #[test]
    fn id_is_echoed_verbatim_for_all_three_kinds() {
        for (raw, expected, rendered) in [
            ("7", Id::Number(7), "7"),
            ("\"s\"", Id::Text("s".to_owned()), "\"s\""),
            ("null", Id::Null, "null"),
        ] {
            let text = format!(r#"{{"jsonrpc":"2.0","id":{raw},"method":"ping","params":{{}}}}"#);
            let request = Request::parse(&text).expect("解析");
            assert_eq!(request.id, Some(expected.clone()));
            let response = Response::success(request.response_id(), Value::Null);
            assert_eq!(
                response.to_json(),
                format!(r#"{{"jsonrpc":"2.0","id":{rendered},"result":null}}"#)
            );
            assert!(!response.is_error());
        }
    }

    #[test]
    fn absent_id_is_a_notification() {
        let request =
            Request::parse(r#"{"jsonrpc":"2.0","method":"tools/list","params":{}}"#).expect("解析");
        assert!(request.is_notification());
        assert_eq!(request.response_id(), Id::Null);
        assert_eq!(request.id, None);
    }

    #[test]
    fn parse_error_is_distinguished_from_invalid_request() {
        let parse = Request::parse("{not json").expect_err("必须是解析错误");
        assert_eq!(parse.code, PARSE_ERROR);
        assert_eq!(parse.message, "JSON 解析失败");
        assert!(parse.data.is_some(), "解析错误必须带 data");

        let invalid = Request::parse("[1,2,3]").expect_err("批处理不支持");
        assert_eq!(invalid.code, INVALID_REQUEST);

        let invalid = Request::parse("42").expect_err("顶层必须对象");
        assert_eq!(invalid.code, INVALID_REQUEST);

        let invalid = Request::parse(r#"{"jsonrpc":"2.0","method":""}"#).expect_err("空方法");
        assert_eq!(invalid.code, INVALID_REQUEST);

        let invalid = Request::parse(r#"{"jsonrpc":"2.0","method":"m","id":1.5}"#)
            .expect_err("非整数 id 非法");
        assert_eq!(invalid.code, INVALID_REQUEST);

        let invalid = Request::parse(r#"{"jsonrpc":"2.0","method":"m","params":5}"#)
            .expect_err("params 类型");
        assert_eq!(invalid.code, INVALID_REQUEST);
    }

    #[test]
    fn unsupported_version_is_invalid_request() {
        let error = Request::parse(r#"{"jsonrpc":"1.0","method":"ping"}"#).expect_err("版本");
        assert_eq!(error.code, INVALID_REQUEST);
        assert!(error.message.contains("2.0"));
    }

    #[test]
    fn error_object_carries_code_message_and_data() {
        let error = ErrorObject::not_implemented("yeban_save_project", "MCP-TOOL-002");
        let response = Response::failure(Id::Text("x".to_owned()), error);
        let value = response.to_value();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], "x");
        assert!(value.get("result").is_none(), "失败响应不得带 result");
        assert_eq!(value["error"]["code"], NOT_IMPLEMENTED);
        assert!(
            value["error"]["message"]
                .as_str()
                .expect("message")
                .contains("yeban_save_project")
        );
        assert_eq!(value["error"]["data"]["code"], "NOT_IMPLEMENTED");
        assert!(value["error"]["data"]["detail"].is_string());
    }

    #[test]
    fn exactly_one_of_result_and_error_is_serialized() {
        let ok = Response::success(Id::Number(1), Value::from(true));
        assert!(ok.to_value().get("result").is_some());
        assert!(ok.to_value().get("error").is_none());

        let failed = Response::failure(Id::Number(1), ErrorObject::new(INTERNAL_ERROR, "boom"));
        assert!(failed.to_value().get("error").is_some());
        assert!(failed.to_value().get("result").is_none());

        // 不变量兜底路径: 两个都为空时显式写 result:null。
        let empty = Response {
            id: Id::Null,
            result: None,
            error: None,
        };
        assert_eq!(
            empty.to_json(),
            r#"{"jsonrpc":"2.0","id":null,"result":null}"#
        );
    }

    #[test]
    fn error_object_without_data_omits_the_key() {
        let error = ErrorObject::new(METHOD_NOT_FOUND, "no");
        let value = serde_json::to_value(&error).expect("序列化");
        assert_eq!(value.as_object().expect("对象").len(), 2);
        assert!(value.get("data").is_none());
    }

    #[test]
    fn id_display_and_value_round_trip() {
        for id in [Id::Number(-3), Id::Text("k".to_owned()), Id::Null] {
            assert_eq!(Id::from_value(&id.to_value()), Some(id.clone()));
            let _ = id.to_string();
        }
    }

    #[test]
    fn parse_error_response_uses_null_id() {
        let response = Response::parse_error("bad");
        assert_eq!(response.id, Id::Null);
        assert_eq!(response.to_value()["error"]["code"], PARSE_ERROR);
    }
}
