//! 分发管线：解析 → 鉴权 → scope 检查 → 工具分发 → `dryRun` 短路 → 幂等去重 [MCP-TOOL-001..010]。
//!
//! ## 管线（顺序本身是判据）
//!
//! ```text
//! 1. method 解析            tools/list | tools/call | 其它(-32601)
//! 2. token 鉴权             缺 token / 错 token / 形状非法 → 401 (硬拒, 不回退放开)
//! 3. 工具解析               name 不在契约枚举 → -32004
//! 4. scope 检查 + 生产硬禁   ui:inject 在生产模式 → 403 (且**不依赖** token 是否正确)
//! 5. dryRun 短路            走 domain::plan(&Domain) —— 只读, 不改状态、不记幂等
//! 6. 幂等去重               相同 idempotencyKey → 复用首次结果 (换当前 id 回显)
//! 7. 工具执行               domain::execute(&mut Domain) —— 真实现
//! ```
//!
//! 第 2 步在第 3 步之前是**刻意的**：未鉴权的调用方不该能通过错误码差异
//! （`-32004` vs `-32001`）探测出本机注册了哪些工具。
//!
//! ## 领域状态住在 [`Dispatcher`] 里
//!
//! [`Dispatcher`] 持有一个 [`crate::domain::Domain`]（活跃工程 + 提交图谱 + 提案记录）。
//! 形态 A 的内嵌使用方通过 [`Dispatcher::domain_mut`] 注入已经打开的工程
//! （[`crate::domain::Domain::open_in_memory`]），形态 B 的二进制走 `yeban_open_project`。
//!
//! ## 两种失败，两条出口
//!
//! `schemas/mcp-tools.schema.json` 的 `ToolResponse.error.code` 是一个**闭合**
//! enum（ADR-0001 D25 之后是联集 20 值），**领域失败**才走 `ToolResponse`。
//! **实现级**状况（如渲染器尚未接线）走 JSON-RPC 错误对象 `-32005`，
//! 绝不伪造一个契约里不存在的 `ToolResponse.error.code`
//! （见 [`crate::domain::error::Fault`] 与 `docs/ledger/mcp-core-notes.md` §2 M10）。
//!
//! ## 幂等去重
//!
//! 键是 `arguments.idempotencyKey`（空字符串按"未提供"处理 —— 契约没有给它
//! `minLength`，把一个空串当成"必须去重"会让所有老客户端莫名其妙命中同一条缓存）。
//! 缓存是 `BTreeMap`（红线 4 的确定性精神），因此缓存快照的顺序逐字节稳定。
//! 幂等缓存存的是**首次执行的完整结果**，因此"同 key 不重复施加"是结构性的：
//! 第 6 步在第 7 步之前，命中缓存就永远走不到 `domain::execute`。
//! 命中时返回的是 `ReplayedToolResponse` 信封（契约
//! `definitions.ReplayedToolResponse`，见 [`replay_value`]）：`replayed: true` +
//! 用当前 `id` 重新组装的那次响应。副作用、`commitCount`、`projectDigest`
//! 都停在首次的值上（缓存里就是首次的载荷，重放不改任何状态）。

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::domain::{self, Domain};
use crate::jsonrpc::{self, ErrorObject, Id, Request, Response};
use crate::security::{
    AuthContext, BearerToken, Channel, Denial, RunMode, ScopeSet, authenticate, authorize,
};
use crate::tools::{self, ToolCall, ToolCallError};

/// MCP 方法名：列出工具。
pub const METHOD_TOOLS_LIST: &str = "tools/list";

/// MCP 方法名：调用工具。
pub const METHOD_TOOLS_CALL: &str = "tools/call";
/// MCP 握手方法名（`[ROAD-M4-002]`）。协议要求客户端的第一条请求就是它。
pub const METHOD_INITIALIZE: &str = "initialize";

/// 客户端未给 `params.protocolVersion` 时本仓声明的协议版本。
pub const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

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

/// 分发器：持有期望的 Bearer Token、授予的作用域、运行模式、**领域会话状态**
/// 与幂等缓存。
///
/// **刻意不实现 `Clone`**：领域会话（活跃工程 + 提交图谱 + 提案记录 + `.yeban.lock`
/// RAII 守卫）一旦被克隆就是两个会各自释放锁、各自累积幂等缓存的"会话"，
/// 那不是复制，那是分叉。需要一个干净的会话就 `Dispatcher::new`。
#[derive(Debug)]
pub struct Dispatcher {
    expected_token: BearerToken,
    granted: ScopeSet,
    mode: RunMode,
    idempotency: BTreeMap<String, CachedOutcome>,
    handled: u64,
    replayed: u64,
    domain: Domain,
}

impl Dispatcher {
    /// 构造（领域会话为空：没有活跃工程）。
    #[must_use]
    pub fn new(expected_token: BearerToken, granted: ScopeSet, mode: RunMode) -> Self {
        Self {
            expected_token,
            granted,
            mode,
            idempotency: BTreeMap::new(),
            handled: 0,
            replayed: 0,
            domain: Domain::new(),
        }
    }

    /// 领域会话状态（只读）。
    #[must_use]
    pub const fn domain(&self) -> &Domain {
        &self.domain
    }

    /// 领域会话状态（可变）：形态 A 的内嵌使用方用它注入已打开的工程 /
    /// 注入时钟（`Domain::open_in_memory`、`Domain::set_now_ms`）。
    pub const fn domain_mut(&mut self) -> &mut Domain {
        &mut self.domain
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

    /// `[ROAD-M4-002]` MCP 握手响应：协议版本 + 能力 + 服务器自述。
    ///
    /// 版本策略：客户端给了 `params.protocolVersion` 就**回显**它（协议允许服务端在回应里确定版本），
    /// 否则用 [`DEFAULT_PROTOCOL_VERSION`]。**不**在这里编造本仓没有的能力。
    fn initialize_result(request: &Request) -> Value {
        let version = request
            .params
            .as_ref()
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION);
        serde_json::json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "yeban-mcp", "version": env!("CARGO_PKG_VERSION") },
        })
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
            METHOD_INITIALIZE => {
                // 与 tools/list 同一档: 握手不携带领域状态, 但仍要求鉴权。
                if let Err(denial) =
                    authenticate(&self.expected_token, context.credential, context.channel)
                {
                    return denied(&denial);
                }
                (200, Ok(Self::initialize_result(request)))
            }
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
        // 5. dryRun 短路: 只读规划, 不执行、不记幂等。
        //
        // `domain::preview` 拿到的是 `&Domain`（共享引用）—— "dryRun 不改状态"
        // 因此是借用检查器保证的, 不靠自觉。运行期判据在 `tests/tools_e2e.rs`。
        if call.is_dry_run() {
            return match dry_run_result(&self.domain, &call) {
                Ok(result) => (200, Ok(result)),
                Err(error) => (http_status_for(&error), Err(error)),
            };
        }
        // 6. 幂等去重: 命中缓存 ⇒ 永远走不到第 7 步 (因此不会重复施加副作用)。
        if let Some(key) = call.idempotency_key()
            && let Some(cached) = self.idempotency.get(key)
        {
            self.replayed += 1;
            let id = request.response_id();
            return (cached.http_status, Ok(replay_value(cached, id)));
        }
        // 7. 执行: 真的做事 (打开/保存/查询/提案/合并/渲染参数校验)。
        let payload = domain::execute(&mut self.domain, &call);
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
///
/// 信封是**契约的一部分**：`schemas/mcp-tools.schema.json` 的
/// `definitions.ReplayedToolResponse`（根 `oneOf` 的第三个分支），因此会校验 schema
/// 的客户端不会拒收它。`response` 装的是**完整 JSON-RPC 响应**（`jsonrpc` / `id` /
/// `result` 或 `error`），用**当前** `id` 重新组装；那个 `result` 才是首次执行产出的
/// `ToolResponse`。真实管线样本 = `mcp-tools.response.replayed.json`。
fn replay_value(cached: &CachedOutcome, id: Id) -> Value {
    let replayed = cached.to_response(id);
    let mut map = Map::new();
    map.insert("replayed".to_owned(), Value::from(true));
    map.insert("response".to_owned(), replayed.to_value());
    Value::Object(map)
}

/// `dryRun` 的结果：`ToolResponse` 形状（`status: success`），`data` 说明"将要做什么"。
///
/// 信封里的 `dryRun` / `tool` / `specId` / `sideEffect` / `wouldChangeState` /
/// `requiredScope` / `arguments` 全部从**注册表**派生（唯一事实源 = [`crate::tools::TOOLS`]）；
/// 真正的差异预览来自 [`crate::domain::preview`]（只读计算，改不了状态）。
///
/// 领域合法性不通过时返回**带内的** `ToolResponse{status:"error"}` ——
/// "只做参数与领域合法性校验"是 `dryRun` 的规范定义
/// （[`crate::tools::COMMON_PARAMS`] 里 `dryRun` 的 `doc`），
/// 因此"没有活跃工程"这类领域失败必须如实报出来，而不是伪造一个成功预览。
///
/// # Errors
///
/// 只有实现级状况（`Fault::Impl`）才返回 `Err`。
fn dry_run_result(domain: &Domain, call: &ToolCall) -> Result<Value, ErrorObject> {
    let preview = match domain::preview(domain, call) {
        Ok(preview) => preview,
        Err(fault) => return fault.into_result(),
    };
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
    data.insert("stateUnchanged".to_owned(), Value::from(true));
    data.insert("preview".to_owned(), preview);
    Ok(crate::tools::ToolResponse::success(Value::Object(data)).to_value())
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

    /// 一个**保证不存在**的工程路径（父目录也不建）—— 用来把"真落盘"变成
    /// 一个确定性的 `IO_ERROR`，而不是依赖 `/tmp` 是否可写。
    fn unique_project_path() -> std::path::PathBuf {
        use std::sync::OnceLock;
        static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            std::env::temp_dir()
                .join(format!(
                    "yeban-mcp-dispatch-{}-{}",
                    std::process::id(),
                    crate::security::BearerToken::generate().token.expose()
                ))
                .join("demo.yeban")
        })
        .clone()
    }

    /// 往分发器里注入一份确定性的规范工程（不碰文件系统）。
    fn seed_project(dispatcher: &mut Dispatcher, path: std::path::PathBuf) {
        dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
        dispatcher
            .domain_mut()
            .open_in_memory(path, yeban_model::samples::filled_project(), false)
            .expect("注入规范工程");
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
        // 有权限 ⇒ 真的走到领域实现。没有活跃工程 ⇒ **带内** ToolResponse 领域失败
        // （不是 JSON-RPC 错误：契约要求领域失败走 `ToolResponse.error.code`）。
        assert_eq!(save.http_status, 200);
        assert_eq!(save.error_code(), None, "领域失败不得伪装成 JSON-RPC 错误");
        let result = save.response.expect("响应").result.expect("result");
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"]["code"], "NO_ACTIVE_PROJECT");

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
        seed_project(&mut dispatcher, unique_project_path());
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
        assert_eq!(result["data"]["stateUnchanged"], true);
        assert_eq!(result["data"]["preview"]["atomic"], true);
        assert_eq!(result["data"]["preview"]["wouldSkip"], false, "force=true");
        // 领域实参保留, 公共参数被剥掉。
        assert_eq!(result["data"]["arguments"]["force"], true);
        assert!(result["data"]["arguments"].get("dryRun").is_none());
        assert!(result["data"]["arguments"].get("idempotencyKey").is_none());
        // 关键: dryRun **不写**幂等缓存。
        assert_eq!(dispatcher.idempotency_len(), 0);
        assert_eq!(dispatcher.replayed(), 0);

        // 同键的真调用必须真的执行 (不被 dryRun 的缓存顶掉)。
        // 目标父目录不存在 ⇒ 领域失败 IO_ERROR —— 这条路径只有真的走到
        // `domain::execute` 才可能产生。
        let real = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_save_project",
                serde_json::json!({"force": true, "idempotencyKey": "k-dry"}),
            ),
        );
        assert_eq!(real.error_code(), None);
        let result = real.response.expect("响应").result.expect("result");
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"]["code"], "IO_ERROR");
        assert_eq!(dispatcher.idempotency_len(), 1);
    }

    #[test]
    fn dry_run_reports_domain_failures_in_band_instead_of_faking_success() {
        // 没有活跃工程时, `save_project` 的 dryRun 必须如实报 NO_ACTIVE_PROJECT ——
        // 规范对 dryRun 的定义是"只做参数与领域合法性校验"。
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_save_project",
                serde_json::json!({"dryRun": true, "idempotencyKey": "k"}),
            ),
        );
        assert_eq!(outcome.http_status, 200);
        assert_eq!(outcome.error_code(), None);
        let result = outcome.response.expect("响应").result.expect("result");
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"]["code"], "NO_ACTIVE_PROJECT");
        assert_eq!(dispatcher.idempotency_len(), 0, "dryRun 仍然不写缓存");
    }

    #[test]
    fn read_only_tool_dry_run_says_state_is_unchanged() {
        let mut dispatcher = dispatcher();
        seed_project(&mut dispatcher, unique_project_path());
        let auth = bearer(&dispatcher);
        let commits_before = dispatcher.domain().commit_count();
        let digest_before = dispatcher.domain().project_digest();
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
        assert_eq!(result["data"]["stateUnchanged"], true);
        assert!(result["data"]["preview"]["result"]["project"].is_object());
        // 只读工具: dryRun 之后状态必须**逐项**不变。
        assert_eq!(dispatcher.domain().commit_count(), commits_before);
        assert_eq!(dispatcher.domain().project_digest(), digest_before);
    }

    /// 一个**保证可写**的渲染输出路径（独占临时目录里的一个文件名）。
    ///
    /// 目录真的被创建：渲染现在会**真的落盘**，因此这个判据需要一个存在的父目录。
    fn render_output_path(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-dispatch-render-{}-{tag}-{}",
            std::process::id(),
            crate::security::BearerToken::generate().token.expose()
        ));
        std::fs::create_dir_all(&dir).expect("建渲染输出目录");
        dir.join("master.wav")
    }

    #[test]
    fn idempotency_replays_the_same_result_without_rendering_twice() {
        let mut dispatcher = dispatcher();
        seed_project(&mut dispatcher, unique_project_path());
        let auth = bearer(&dispatcher);
        let output = render_output_path("idem");
        let arguments = serde_json::json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": output.display().to_string(),
            "idempotencyKey": "k1",
        });
        let line = |id: &str| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "yeban_render_master", "arguments": arguments}
            })
            .to_string()
        };
        let first_line = line("first");
        let first = dispatcher.handle_line(Channel::Http, Some(&auth), &first_line);
        assert_eq!(first.error_code(), None, "渲染必须带内成功");
        assert_eq!(dispatcher.idempotency_len(), 1);
        assert_eq!(dispatcher.idempotency_keys(), vec!["k1"]);
        let first_response = first.response.expect("响应");
        let first_result = first_response.result.clone().expect("result");
        assert_eq!(first_result["status"], "success", "{first_result}");
        assert!(output.is_file(), "第一次调用必须真的写出母带");
        let modified_first = std::fs::metadata(&output)
            .expect("元数据")
            .modified()
            .expect("mtime");

        let second_line = line("second");
        let second = dispatcher.handle_line(Channel::Http, Some(&auth), &second_line);
        assert_eq!(dispatcher.replayed(), 1, "第二次必须命中缓存");
        assert_eq!(dispatcher.idempotency_len(), 1, "缓存不得增长");
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
        assert_eq!(
            inner["result"], first_result,
            "重放的主体必须与首次逐字节相同"
        );
        assert_eq!(
            std::fs::metadata(&output)
                .expect("元数据")
                .modified()
                .expect("mtime"),
            modified_first,
            "命中幂等缓存 ⇒ 输出文件不得被重写"
        );

        // 最强的一条: 把产物删掉再重放同键 —— 文件**不会**被重建,
        // 因此第二次调用确实没有进入渲染路径(而不是"渲染出一样的字节")。
        std::fs::remove_file(&output).expect("删掉产物");
        let third = dispatcher.handle_line(Channel::Http, Some(&auth), &line("third"));
        let third_result = third.response.expect("响应").result.expect("result");
        assert_eq!(third_result["replayed"], true);
        assert!(!output.exists(), "同键重放不得重新渲染 (产物不应被重建)");
        assert_eq!(dispatcher.replayed(), 2);
        if let Some(parent) = output.parent() {
            std::fs::remove_dir_all(parent).ok();
        }
    }

    #[test]
    fn render_master_is_wired_and_writes_a_real_master() {
        let mut dispatcher = dispatcher();
        seed_project(&mut dispatcher, unique_project_path());
        let auth = bearer(&dispatcher);
        let output = render_output_path("wired");
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_render_master",
                serde_json::json!({
                    "format": "wav",
                    "sampleRate": 48000,
                    "path": output.display().to_string(),
                }),
            ),
        );
        assert_eq!(outcome.http_status, 200, "不再有实现级 -32005");
        assert_eq!(outcome.error_code(), None);
        let result = outcome.response.expect("响应").result.expect("result");
        assert_eq!(result["status"], "success", "{result}");
        assert_eq!(result["data"]["rendered"], true);
        assert_eq!(result["data"]["format"], "wav");
        assert_eq!(result["data"]["bitDepth"], 24);
        assert_eq!(result["data"]["frames"], 90_000);
        let written = std::fs::read(&output).expect("产物");
        assert_eq!(
            written.len(),
            usize::try_from(result["data"]["bytes"].as_u64().expect("bytes")).expect("小尺寸")
        );
        // 落盘是原子的: 目录里不许留下临时文件。
        let leftovers: Vec<String> = std::fs::read_dir(output.parent().expect("父目录"))
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(crate::domain::store::TEMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
        if let Some(parent) = output.parent() {
            std::fs::remove_dir_all(parent).ok();
        }
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
    fn all_ten_tools_reach_the_domain_and_no_blanket_not_implemented_remains() {
        // 本轮之前: 十个工具一律 -32005。现在: 每一个都进**领域实现**,
        // 结果要么是带内 ToolResponse（成功或领域失败）, 要么是 -32005 那条
        // **参数校验已通过**的渲染器（唯一未接线的一半）。
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
                None,
                "{} 不得再返回 JSON-RPC 层错误 (领域失败走 ToolResponse)",
                spec.name
            );
            let result = outcome
                .response
                .clone()
                .expect("响应")
                .result
                .expect("必须是 result 而不是 JSON-RPC error");
            assert!(
                matches!(result["status"].as_str(), Some("success" | "error")),
                "{} 必须产出契约形状的 ToolResponse: {result}",
                spec.name
            );
            // 领域失败时错误码必须落在契约 enum 内。
            if let Some(error) = result.get("error") {
                let code = error["code"].as_str().unwrap_or_default();
                assert!(
                    crate::tools::ErrorCode::SCHEMA_CONTRACT
                        .iter()
                        .any(|known| known.as_str() == code),
                    "{} 的错误码必须落在契约 enum 里: {error}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn no_tool_is_left_at_an_implementation_level_error() {
        // 上一线唯一的实现级出口是"参数校验通过之后的渲染器"。
        // 渲染器接线之后, 这个出口在本 crate 的工具路径上**不再存在**:
        // 好参数 ⇒ 真渲染 ⇒ 带内 ToolResponse; 坏参数 ⇒ 带内领域失败。
        let mut dispatcher = dispatcher();
        seed_project(&mut dispatcher, unique_project_path());
        let auth = bearer(&dispatcher);
        let output = render_output_path("no-impl");
        let good = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_render_master",
                serde_json::json!({
                    "format": "wav",
                    "sampleRate": 48000,
                    "path": output.display().to_string(),
                }),
            ),
        );
        assert_eq!(good.http_status, 200);
        assert_eq!(good.error_code(), None);
        let result = good.response.expect("响应").result.expect("result");
        assert_eq!(result["status"], "success", "{result}");
        assert!(
            result["data"].get("validated").is_none(),
            "旧的 `validated` 披露字段属于 -32005 那一版, 不该再出现"
        );
        // 坏参数仍然先于渲染被拦下（参数校验先于渲染）。
        let bad = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call(
                "yeban_render_master",
                serde_json::json!({"format": "mp3", "sampleRate": 48000}),
            ),
        );
        assert_eq!(bad.http_status, 200);
        let bad_result = bad.response.expect("响应").result.expect("result");
        assert_eq!(bad_result["status"], "error");
        assert_eq!(bad_result["error"]["code"], "INVALID_PARAMETER_RANGE");
        if let Some(parent) = output.parent() {
            std::fs::remove_dir_all(parent).ok();
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
        // 无头 stdio 通道不带 Authorization 头也走到领域实现；
        // 没有活跃工程 ⇒ 带内 NO_ACTIVE_PROJECT（不是 401、不是 -32005）。
        assert_eq!(outcome.http_status, 200);
        assert_eq!(outcome.error_code(), None);
        let result = outcome.response.expect("响应").result.expect("result");
        assert_eq!(result["error"]["code"], "NO_ACTIVE_PROJECT");
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
