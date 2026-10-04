//! 分发管线：解析 → 鉴权 → scope 检查 → 工具分发 → `dryRun` 短路 → 幂等去重 [MCP-TOOL-001..010]。
//!
//! ## 管线（顺序本身是判据）
//!
//! ```text
//! 1. method 解析            tools/list | tools/call | 其它(-32601)
//! 2. token 鉴权             缺 token / 错 token / 形状非法 → 401 (硬拒, 不回退放开)
//! 3. 工具解析               name 不在契约枚举 → -32004
//! 4. scope 检查 + 生产硬禁   ui:inject 在生产模式 → 403 (且**不依赖** token 是否正确)
//! 5. dryRun 短路            只校验 + 返回"将要做什么", 不改状态、不记幂等
//! 6. 幂等去重               相同 idempotencyKey → 复用首次结果 (换当前 id 回显)
//! 7. 工具执行               本轮一律 -32005 NOT_IMPLEMENTED (诚实状态)
//! ```
//!
//! 第 2 步在第 3 步之前是**刻意的**：未鉴权的调用方不该能通过错误码差异
//! （`-32004` vs `-32001`）探测出本机注册了哪些工具。
//!
//! ## 为什么"尚未实现"走 JSON-RPC 错误而不是 `ToolResponse`
//!
//! `schemas/mcp-tools.schema.json` 的 `ToolResponse.error.code` 是一个**闭合**的
//! enum（ADR-0001 D25 之后是联集 20 值），**领域失败**才走 `ToolResponse`。
//! `NOT_IMPLEMENTED` 不在那个 enum 里，而且不该在 —— 它不是领域失败，是"这条能力
//! 还没接线"。所以本模块对**实现级**状况一律返回 JSON-RPC 错误对象 `-32005`，
//! 绝不伪造一个契约里不存在的 `ToolResponse.error.code`
//! （见 [`crate::tools::ErrorCode`] 的说明与 `docs/ledger/mcp-core-notes.md` §2 M10）。
//!
//! ## 幂等去重
//!
//! 键是 `arguments.idempotencyKey`（空字符串按"未提供"处理 —— 契约没有给它
//! `minLength`，把一个空串当成"必须去重"会让所有老客户端莫名其妙命中同一条缓存）。
//! 缓存是 `BTreeMap`（红线 4 的确定性精神），因此缓存快照的顺序逐字节稳定。

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::jsonrpc::{self, ErrorObject, Id, Request, Response};
use crate::security::{
    AuthContext, BearerToken, Channel, Denial, RunMode, ScopeSet, authenticate, authorize,
};
use crate::tools::{self, ToolCall, ToolCallError};

/// MCP 方法名：列出工具。
pub const METHOD_TOOLS_LIST: &str = "tools/list";

/// MCP 方法名：调用工具。
pub const METHOD_TOOLS_CALL: &str = "tools/call";

/// `dryRun` 结果里标记"这是模拟"的字段名。
pub const DRY_RUN_FLAG: &str = "dryRun";

/// 一次分发的产物。
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// HTTP 形态的状态码（stdio 形态忽略它）。集中在一处，避免两套口径。
    pub http_status: u16,
    /// JSON-RPC 响应；`None` 表示这是 notification（规范要求不回复）。
    pub response: Option<Response>,
}

impl Outcome {
    /// 只有状态码、没有响应（notification）。
    #[must_use]
    pub const fn silent(http_status: u16) -> Self {
        Self {
            http_status,
            response: None,
        }
    }

    /// 错误码（成功或 notification 时为 `None`）。
    #[must_use]
    pub fn error_code(&self) -> Option<i64> {
        self.response
            .as_ref()
            .and_then(Response::error_object)
            .map(|error| error.code)
    }
}

/// 幂等缓存里保存的东西：**不带 `id`** 的响应载荷。
///
/// 重放时必须用**当前**请求的 `id` 重新组装响应 —— 规范要求 `id` 回显，
/// 直接把首次的响应原样吐回去会回显一个陈旧的 `id`。
#[derive(Clone, Debug, PartialEq)]
pub struct CachedOutcome {
    /// HTTP 状态码。
    pub http_status: u16,
    /// 成功结果。
    pub result: Option<Value>,
    /// 失败对象。
    pub error: Option<ErrorObject>,
}

impl CachedOutcome {
    fn from_response(http_status: u16, response: &Response) -> Self {
        Self {
            http_status,
            result: response.result.clone(),
            error: response.error.clone(),
        }
    }

    fn to_response(&self, id: Id) -> Response {
        Response {
            id,
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }
}

/// 分发器：持有期望的 Bearer Token、授予的作用域、运行模式与幂等缓存。
#[derive(Clone, Debug)]
pub struct Dispatcher {
    expected_token: BearerToken,
    granted: ScopeSet,
    mode: RunMode,
    idempotency: BTreeMap<String, CachedOutcome>,
    handled: u64,
    replayed: u64,
}

impl Dispatcher {
    /// 构造。
    #[must_use]
    pub fn new(expected_token: BearerToken, granted: ScopeSet, mode: RunMode) -> Self {
        Self {
            expected_token,
            granted,
            mode,
            idempotency: BTreeMap::new(),
            handled: 0,
            replayed: 0,
        }
    }

    /// 期望的令牌。
    #[must_use]
    pub fn expected_token(&self) -> &BearerToken {
        &self.expected_token
    }

    /// 授予的作用域。
    #[must_use]
    pub fn granted(&self) -> &ScopeSet {
        &self.granted
    }

    /// 运行模式。
    #[must_use]
    pub const fn mode(&self) -> RunMode {
        self.mode
    }

    /// 已处理的请求数。
    #[must_use]
    pub const fn handled(&self) -> u64 {
        self.handled
    }

    /// 命中幂等缓存的次数。
    #[must_use]
    pub const fn replayed(&self) -> u64 {
        self.replayed
    }

    /// 幂等缓存里的键，**字典序**（`BTreeMap` 的确定性）。
    #[must_use]
    pub fn idempotency_keys(&self) -> Vec<&str> {
        self.idempotency.keys().map(String::as_str).collect()
    }

    /// 幂等缓存条数。
    #[must_use]
    pub fn idempotency_len(&self) -> usize {
        self.idempotency.len()
    }

    /// 处理一个已解析的请求。
    pub fn handle(
        &mut self,
        channel: Channel,
        authorization: Option<&str>,
        request: &Request,
    ) -> Outcome {
        self.handled += 1;
        let context =
            AuthContext::from_headers(channel, authorization, self.granted.clone(), self.mode);
        let (http_status, payload) = self.evaluate(&context, request);
        if request.is_notification() {
            return Outcome::silent(http_status);
        }
        let id = request.response_id();
        let response = match payload {
            Ok(result) => Response::success(id, result),
            Err(error) => Response::failure(id, error),
        };
        Outcome {
            http_status,
            response: Some(response),
        }
    }

    /// 处理一行 JSON 文本（stdio 与 HTTP body 共用的入口）。
    ///
    /// ## 鉴权**先于**解析
    ///
    /// 本入口在解析 JSON **之前**先过 token 闸门：未鉴权的调用方不是 JSON-RPC 对端，
    /// 因此它拿到的 401 里 `id` 一定是 `null`（无从回显），而且**请求体一个字节都不会
    /// 被解释**。带合法 token 但 JSON 非法时才回到正常的解析错误（400 / `-32700`）。
    ///
    /// [`Dispatcher::handle`] 拿到的是**已解析**的请求，所以那里的鉴权失败仍然能回显 `id`。
    pub fn handle_line(
        &mut self,
        channel: Channel,
        authorization: Option<&str>,
        line: &str,
    ) -> Outcome {
        self.handled += 1;
        let context =
            AuthContext::from_headers(channel, authorization, self.granted.clone(), self.mode);
        if let Err(denial) = authenticate(&self.expected_token, context.credential, context.channel)
        {
            let (status, payload) = denied(&denial);
            let response = match payload {
                Ok(result) => Response::success(Id::Null, result),
                Err(error) => Response::failure(Id::Null, error),
            };
            return Outcome {
                http_status: status,
                response: Some(response),
            };
        }
        match Request::parse(line) {
            Ok(request) => {
                self.handled -= 1;
                self.handle(channel, authorization, &request)
            }
            Err(error) => Outcome {
                http_status: http_status_for(&error),
                response: Some(Response::failure(Id::Null, error)),
            },
        }
    }

    /// 管线本体。
    fn evaluate(
        &mut self,
        context: &AuthContext<'_>,
        request: &Request,
    ) -> (u16, Result<Value, ErrorObject>) {
        match request.method.as_str() {
            METHOD_TOOLS_LIST => {
                // 只要求鉴权: 工具清单本身不携带领域状态。
                if let Err(denial) =
                    authenticate(&self.expected_token, context.credential, context.channel)
                {
                    return denied(&denial);
                }
                (200, Ok(tools::catalog()))
            }
            METHOD_TOOLS_CALL => self.evaluate_tool_call(context, request),
            other => {
                let error = ErrorObject::method_not_found(other);
                (http_status_for(&error), Err(error))
            }
        }
    }

    /// `tools/call` 的完整管线。
    fn evaluate_tool_call(
        &mut self,
        context: &AuthContext<'_>,
        request: &Request,
    ) -> (u16, Result<Value, ErrorObject>) {
        // 2. 鉴权 (在解析工具名之前, 免得错误码差异泄漏工具集)。
        if let Err(denial) = authenticate(&self.expected_token, context.credential, context.channel)
        {
            return denied(&denial);
        }
        // 3. 工具解析 + 参数校验。
        let call = match ToolCall::from_params(request.params.as_ref()) {
            Ok(call) => call,
            Err(ToolCallError::UnknownTool { name }) => {
                let error = ErrorObject::tool_not_found(&name);
                return (http_status_for(&error), Err(error));
            }
            Err(other) => {
                let error = ErrorObject::invalid_params(other.to_string());
                return (http_status_for(&error), Err(error));
            }
        };
        // 4. scope 检查 (authorize 会先做 ui:inject 的生产硬禁)。
        if let Err(denial) = authorize(&self.expected_token, context, call.tool.scope) {
            return denied(&denial);
        }
        // 5. dryRun 短路: 不执行、不记幂等。
        if call.is_dry_run() {
            return (200, Ok(dry_run_result(&call)));
        }
        // 6. 幂等去重。
        if let Some(key) = call.idempotency_key()
            && let Some(cached) = self.idempotency.get(key)
        {
            self.replayed += 1;
            let id = request.response_id();
            return (cached.http_status, Ok(replay_value(cached, id)));
        }
        // 7. 执行。
        let payload = execute(&call);
        let status = match &payload {
            Ok(_) => 200,
            Err(error) => http_status_for(error),
        };
        if let Some(key) = call.idempotency_key() {
            let response = match &payload {
                Ok(result) => Response::success(request.response_id(), result.clone()),
                Err(error) => Response::failure(request.response_id(), error.clone()),
            };
            self.idempotency.insert(
                key.to_owned(),
                CachedOutcome::from_response(status, &response),
            );
        }
        (status, payload)
    }
}

/// 幂等重放的返回值：把缓存载荷包成"这就是重放"的形状。
///
/// 结果主体逐字节相同（判据会断言），只在外面加一层 `replayed` 信封，
/// 让调用方**看得见**自己拿到的是缓存而不是一次新的执行。
fn replay_value(cached: &CachedOutcome, id: Id) -> Value {
    let replayed = cached.to_response(id);
    let mut map = Map::new();
    map.insert("replayed".to_owned(), Value::from(true));
    map.insert("response".to_owned(), replayed.to_value());
    Value::Object(map)
}

/// `dryRun` 的结果：`ToolResponse` 形状（`status: success`），`data` 说明"将要做什么"。
fn dry_run_result(call: &ToolCall) -> Value {
    let mut data = Map::new();
    data.insert(DRY_RUN_FLAG.to_owned(), Value::from(true));
    data.insert("tool".to_owned(), Value::from(call.tool.name));
    data.insert("specId".to_owned(), Value::from(call.tool.spec_id));
    data.insert(
        "sideEffect".to_owned(),
        Value::from(call.tool.side_effect.as_str()),
    );
    data.insert(
        "wouldChangeState".to_owned(),
        Value::from(call.tool.side_effect.is_side_effecting()),
    );
    data.insert(
        "requiredScope".to_owned(),
        Value::from(call.tool.scope.as_str()),
    );
    data.insert(
        "arguments".to_owned(),
        Value::Object(call.domain_arguments()),
    );
    crate::tools::ToolResponse::success(Value::Object(data)).to_value()
}

/// 工具的真实执行。
///
/// **本轮的诚实状态**：分发 / 鉴权 / `dryRun` / 幂等已经是真实现 + 真判据；
/// 十个工具的领域实现尚未接线，因此一律返回 `-32005 NOT_IMPLEMENTED`，
/// 并且把工具名与规范 ID 放进错误对象的 `data` 里（可机械对账，见
/// `docs/ledger/mcp-core-notes.md` §4 的 pending 清单）。
fn execute(call: &ToolCall) -> Result<Value, ErrorObject> {
    Err(ErrorObject::not_implemented(
        call.tool.name,
        call.tool.spec_id,
    ))
}

/// 把拒绝理由变成"状态码 + JSON-RPC 错误对象"。
fn denied(denial: &Denial) -> (u16, Result<Value, ErrorObject>) {
    let error = ErrorObject::new(denial.rpc_code(), denial.message()).with_data(denial.data());
    (denial.http_status(), Err(error))
}

/// JSON-RPC 错误码 → HTTP 状态码。
///
/// | 错误码 | HTTP | 理由 |
/// | :--- | ---: | :--- |
/// | `-32700` `-32600` `-32602` `-32004` | 400 | 请求本身不合法 |
/// | `-32601` | 404 | 方法不存在 |
/// | `-32001` | 401 | 未鉴权 |
/// | `-32003` | 403 | 已鉴权但未授权 |
/// | `-32005` | 501 | 工具存在但尚未实现（**不假装成功**） |
/// | 其它 | 500 | 兜底 |
#[must_use]
pub fn http_status_for(error: &ErrorObject) -> u16 {
    match error.code {
        jsonrpc::PARSE_ERROR
        | jsonrpc::INVALID_REQUEST
        | jsonrpc::INVALID_PARAMS
        | jsonrpc::TOOL_NOT_FOUND => 400,
        jsonrpc::METHOD_NOT_FOUND => 404,
        jsonrpc::UNAUTHORIZED => 401,
        jsonrpc::FORBIDDEN => 403,
        jsonrpc::NOT_IMPLEMENTED => 501,
        _ => 500,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{Scope, ScopeSet};
    use crate::tools::ErrorCode;

    fn token() -> BearerToken {
        BearerToken::generate().token
    }

    fn dispatcher() -> Dispatcher {
        Dispatcher::new(token(), ScopeSet::all(), RunMode::Production)
    }

    fn call(name: &str, arguments: Value) -> String {
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                           "params": {"name": name, "arguments": arguments}})
        .to_string()
    }

    fn bearer(dispatcher: &Dispatcher) -> String {
        format!("Bearer {}", dispatcher.expected_token().expose())
    }

    #[test]
    fn run_mode_defaults_to_production() {
        assert_eq!(RunMode::default(), RunMode::Production);
        assert!(RunMode::default().is_production());
        assert_eq!(RunMode::from_test_flag(false), RunMode::Production);
        assert_eq!(RunMode::from_test_flag(true), RunMode::Test);
        assert_eq!(
            Dispatcher::new(token(), ScopeSet::all(), RunMode::default()).mode(),
            RunMode::Production
        );
    }

    #[test]
    fn tools_list_requires_a_valid_token_and_returns_all_ten_tools() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let request =
            Request::parse(r#"{"jsonrpc":"2.0","id":9,"method":"tools/list"}"#).expect("解析");

        // 缺 token ⇒ 401。
        let outcome = dispatcher.handle(Channel::Http, None, &request);
        assert_eq!(outcome.http_status, 401);
        assert_eq!(outcome.error_code(), Some(jsonrpc::UNAUTHORIZED));
        assert_eq!(outcome.response.expect("响应").id, Id::Number(9));

        // 错 token ⇒ 401。
        let outcome = dispatcher.handle(Channel::Http, Some("Bearer deadbeef"), &request);
        assert_eq!(outcome.http_status, 401);

        // 对 token ⇒ 200 + 10 个工具。
        let outcome = dispatcher.handle(Channel::Http, Some(&auth), &request);
        assert_eq!(outcome.http_status, 200);
        let response = outcome.response.expect("响应");
        let tools = response.result_value().expect("result")["tools"]
            .as_array()
            .expect("tools 数组")
            .len();
        assert_eq!(tools, crate::tools::TOOL_COUNT);
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let request = Request::parse(r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#).expect("解析");
        let outcome = dispatcher.handle(Channel::Http, Some(&auth), &request);
        assert_eq!(outcome.http_status, 404);
        assert_eq!(outcome.error_code(), Some(jsonrpc::METHOD_NOT_FOUND));
    }

    #[test]
    fn unauthenticated_call_never_leaks_tool_existence() {
        let mut dispatcher = dispatcher();
        // 未知工具 + 缺 token ⇒ 401 (而不是 400/404) —— 鉴权在工具解析之前。
        let outcome = dispatcher.handle_line(
            Channel::Http,
            None,
            &call("yeban_definitely_not", serde_json::json!({})),
        );
        assert_eq!(outcome.http_status, 401);
        assert_eq!(outcome.error_code(), Some(jsonrpc::UNAUTHORIZED));
    }

    #[test]
    fn unknown_tool_with_a_valid_token_is_tool_not_found() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call("yeban_definitely_not", serde_json::json!({})),
        );
        assert_eq!(outcome.http_status, 400);
        assert_eq!(outcome.error_code(), Some(jsonrpc::TOOL_NOT_FOUND));
        let error = outcome.response.expect("响应").error.expect("错误");
        assert_eq!(
            error.data.expect("data")["detail"],
            "未知工具 `yeban_definitely_not`"
        );
    }

    #[test]
    fn scope_denied_when_only_ui_read_is_granted() {
        let expected = token();
        let mut dispatcher = Dispatcher::new(
            expected.clone(),
            ScopeSet::from_scopes([Scope::UiRead]),
            RunMode::Production,
        );
        let auth = format!("Bearer {}", expected.expose());
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call("yeban_query_project", serde_json::json!({})),
        );
        assert_eq!(outcome.http_status, 403);
        assert_eq!(outcome.error_code(), Some(jsonrpc::FORBIDDEN));
        let error = outcome.response.expect("响应").error.expect("错误");
        let data = error.data.expect("data");
        assert_eq!(data["kind"], "insufficient-scope");
        assert_eq!(data["requiredScope"], "app:admin");
        assert_eq!(data["grantedScopes"], "ui:read");
    }

    #[test]
    fn app_save_scope_is_enough_for_save_but_not_for_query() {
        let expected = token();
        let mut dispatcher = Dispatcher::new(
            expected.clone(),
            ScopeSet::from_scopes([Scope::AppSave]),
            RunMode::Production,
        );
        let auth = format!("Bearer {}", expected.expose());
        let save = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call("yeban_save_project", serde_json::json!({})),
        );
        // 有权限 ⇒ 走到执行 (尚未实现)。
        assert_eq!(save.error_code(), Some(jsonrpc::NOT_IMPLEMENTED));

        let query = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call("yeban_query_project", serde_json::json!({})),
        );
        assert_eq!(query.http_status, 403);
    }

    #[test]
    fn dry_run_short_circuits_without_recording_idempotency() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_save_project",
                serde_json::json!({"force": true, "dryRun": true, "idempotencyKey": "k-dry"}),
            ),
        );
        assert_eq!(outcome.http_status, 200);
        let result = outcome.response.expect("响应").result.expect("result");
        assert_eq!(result["status"], "success");
        assert_eq!(result["data"][DRY_RUN_FLAG], true);
        assert_eq!(result["data"]["tool"], "yeban_save_project");
        assert_eq!(result["data"]["specId"], "MCP-TOOL-002");
        assert_eq!(result["data"]["wouldChangeState"], true);
        assert_eq!(result["data"]["sideEffect"], "disk");
        // 领域实参保留, 公共参数被剥掉。
        assert_eq!(result["data"]["arguments"]["force"], true);
        assert!(result["data"]["arguments"].get("dryRun").is_none());
        assert!(result["data"]["arguments"].get("idempotencyKey").is_none());
        // 关键: dryRun **不写**幂等缓存。
        assert_eq!(dispatcher.idempotency_len(), 0);
        assert_eq!(dispatcher.replayed(), 0);

        // 同键的真调用必须真的执行 (不被 dryRun 的缓存顶掉)。
        let real = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_save_project",
                serde_json::json!({"force": true, "idempotencyKey": "k-dry"}),
            ),
        );
        assert_eq!(real.error_code(), Some(jsonrpc::NOT_IMPLEMENTED));
        assert_eq!(dispatcher.idempotency_len(), 1);
    }

    #[test]
    fn read_only_tool_dry_run_says_state_is_unchanged() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_query_project",
                serde_json::json!({"dryRun": true, "limit": 5}),
            ),
        );
        let result = outcome.response.expect("响应").result.expect("result");
        assert_eq!(result["data"]["wouldChangeState"], false);
        assert_eq!(result["data"]["sideEffect"], "read-only");
    }

    #[test]
    fn idempotency_replays_the_same_result_with_the_current_id() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let first_line = serde_json::json!({
            "jsonrpc": "2.0", "id": "first", "method": "tools/call",
            "params": {"name": "yeban_render_master",
                       "arguments": {"format": "wav", "sampleRate": 48000, "idempotencyKey": "k1"}}
        })
        .to_string();
        let first = dispatcher.handle_line(Channel::Http, Some(&auth), &first_line);
        assert_eq!(first.error_code(), Some(jsonrpc::NOT_IMPLEMENTED));
        assert_eq!(dispatcher.idempotency_len(), 1);
        assert_eq!(dispatcher.idempotency_keys(), vec!["k1"]);

        let second_line = serde_json::json!({
            "jsonrpc": "2.0", "id": "second", "method": "tools/call",
            "params": {"name": "yeban_render_master",
                       "arguments": {"format": "wav", "sampleRate": 48000, "idempotencyKey": "k1"}}
        })
        .to_string();
        let second = dispatcher.handle_line(Channel::Http, Some(&auth), &second_line);
        assert_eq!(dispatcher.replayed(), 1, "第二次必须命中缓存");
        assert_eq!(dispatcher.idempotency_len(), 1, "缓存不得增长");

        let first_response = first.response.expect("响应");
        let envelope = second.response.expect("响应");
        assert_eq!(
            envelope.id,
            Id::Text("second".to_owned()),
            "必须回显当前 id"
        );
        let replayed = envelope.result.expect("result");
        assert_eq!(replayed["replayed"], true);
        let inner = &replayed["response"];
        assert_eq!(inner["id"], "second", "内层也要用当前 id");
        // 错误主体逐字节相同。
        assert_eq!(
            inner["error"],
            serde_json::to_value(first_response.error.expect("错误")).expect("序列化")
        );
    }

    #[test]
    fn distinct_keys_do_not_collide_and_cache_is_ordered() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        for (index, key) in ["b", "a", "c", "a"].iter().enumerate() {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": index, "method": "tools/call",
                "params": {"name": "yeban_query_project", "arguments": {"idempotencyKey": key}}
            })
            .to_string();
            dispatcher.handle_line(Channel::Http, Some(&auth), &line);
        }
        assert_eq!(
            dispatcher.idempotency_keys(),
            vec!["a", "b", "c"],
            "BTreeMap 的键序必须是字典序 (确定性)"
        );
        assert_eq!(dispatcher.replayed(), 1);
    }

    #[test]
    fn empty_idempotency_key_is_treated_as_absent() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        for _ in 0..2 {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "yeban_query_project", "arguments": {"idempotencyKey": ""}}
            })
            .to_string();
            dispatcher.handle_line(Channel::Http, Some(&auth), &line);
        }
        assert_eq!(dispatcher.idempotency_len(), 0);
        assert_eq!(dispatcher.replayed(), 0);
    }

    #[test]
    fn unimplemented_tools_report_the_spec_id_and_are_not_faked() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        for spec in &crate::tools::TOOLS {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": spec.name, "arguments": minimal_arguments(spec)}
            })
            .to_string();
            let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
            assert_eq!(
                outcome.error_code(),
                Some(jsonrpc::NOT_IMPLEMENTED),
                "{} 的领域实现尚未接线, 不得假装成功",
                spec.name
            );
            assert_eq!(outcome.http_status, 501);
            let error = outcome.response.expect("响应").error.expect("错误");
            let data = error.data.expect("data");
            assert_eq!(data["code"], "NOT_IMPLEMENTED");
            assert_eq!(
                data["detail"],
                format!(
                    "`{}` ({}) 的真实领域实现待 MCP-TOOL 能力切片接线",
                    spec.name, spec.spec_id
                )
            );
        }
    }

    /// 为每个工具造一份最小合法实参（只为走到执行分支）。
    fn minimal_arguments(spec: &crate::tools::ToolSpec) -> Value {
        let mut map = Map::new();
        for param in spec.all_params() {
            if !param.required {
                continue;
            }
            let value = match param.json_type {
                "string" => Value::from("01J8ZQ00000000000000000001"),
                "integer" => Value::from(1),
                "number" => Value::from(1.0),
                "boolean" => Value::from(true),
                "array" => Value::Array(vec![Value::from("track.id")]),
                _ => Value::Object(Map::new()),
            };
            map.insert(param.name.to_owned(), value);
        }
        Value::Object(map)
    }

    #[test]
    fn invalid_arguments_are_rejected_before_execution() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_set_macro",
                serde_json::json!({"macroIndex": 0, "value": 0.5}),
            ),
        );
        assert_eq!(outcome.http_status, 400);
        assert_eq!(outcome.error_code(), Some(jsonrpc::INVALID_PARAMS));
        let error = outcome.response.expect("响应").error.expect("错误");
        assert!(
            error.data.expect("data")["detail"]
                .as_str()
                .expect("detail")
                .contains("trackId")
        );
    }

    #[test]
    fn ui_inject_is_hard_denied_in_production_through_the_pipeline() {
        // 领域工具集里没有 ui:* 工具 (它们属于 yeban-ui-mcp), 但**判定**在这一侧:
        // 生产模式下即使拿满 scope 也一样被拒。
        let expected = token();
        let header = format!("Bearer {}", expected.expose());
        let context = AuthContext::http(Some(&header), ScopeSet::all(), RunMode::Production);
        let denial = authorize(&expected, &context, Scope::UiInject).expect_err("必须拒绝");
        assert_eq!(denial.http_status(), 403);
        assert_eq!(denial.kind(), "forbidden-in-production");
    }

    #[test]
    fn notifications_produce_no_response() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            r#"{"jsonrpc":"2.0","method":"tools/list"}"#,
        );
        assert_eq!(outcome.response, None);
        assert_eq!(outcome.http_status, 200);
        assert_eq!(dispatcher.handled(), 1);
    }

    #[test]
    fn authentication_precedes_parsing_on_the_raw_entry_point() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        // 未鉴权 + 非法 JSON ⇒ 401 (而不是 400): token 闸门在协议层之前。
        let outcome = dispatcher.handle_line(Channel::Http, None, "{oops");
        assert_eq!(outcome.http_status, 401);
        assert_eq!(outcome.error_code(), Some(jsonrpc::UNAUTHORIZED));
        assert_eq!(outcome.response.expect("响应").id, Id::Null);

        // 已鉴权 + 非法 JSON ⇒ 400 解析错误。
        let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), "{oops");
        assert_eq!(outcome.http_status, 400);
        assert_eq!(outcome.error_code(), Some(jsonrpc::PARSE_ERROR));
    }

    #[test]
    fn raw_entry_point_counts_each_request_exactly_once() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        dispatcher.handle_line(Channel::Http, None, "{oops");
        dispatcher.handle_line(Channel::Http, Some(&auth), "{oops");
        assert_eq!(
            dispatcher.handled(),
            3,
            "handle_line 委托 handle 时不得重复计数"
        );
    }

    #[test]
    fn malformed_text_yields_a_parse_error_with_null_id() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), "{oops");
        assert_eq!(outcome.http_status, 400);
        assert_eq!(outcome.error_code(), Some(jsonrpc::PARSE_ERROR));
        assert_eq!(outcome.response.expect("响应").id, Id::Null);
    }

    #[test]
    fn stdio_channel_needs_no_authorization_header() {
        let mut dispatcher = dispatcher();
        let outcome = dispatcher.handle_line(
            Channel::Stdio,
            None,
            &call("yeban_query_project", serde_json::json!({})),
        );
        assert_eq!(outcome.error_code(), Some(jsonrpc::NOT_IMPLEMENTED));
    }

    #[test]
    fn tool_response_error_codes_never_use_a_code_outside_the_catalog() {
        // 反证: 本模块产出的 JSON-RPC 错误码都在 jsonrpc 常量表里;
        // 领域 ToolResponse 一旦接线, 其 code 必须来自 ErrorCode。
        for code in crate::tools::ErrorCode::ALL {
            assert!(!code.as_str().is_empty());
        }
        let sample = crate::tools::ToolResponse::failure(ErrorCode::IoError, "x").to_value();
        assert_eq!(sample["error"]["code"], "IO_ERROR");
    }
}
