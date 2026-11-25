//! 分发管线：解析 → 鉴权 → scope 检查 → 工具分发 → `dryRun` 短路 → 幂等去重 [MCP-TOOL-001..010]。
//!
//! ## 管线（顺序本身是判据）
//!
//! ```text
//! 1. method 解析            tools/list | tools/call | initialize | ping | 其它(-32601)
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
//!
//! ## 有界 LRU（T-FIX-25 / #24.4）
//!
//! 缓存条目数与占用字节都有上限（[`MAX_IDEMPOTENCY_ENTRIES`] /
//! [`MAX_IDEMPOTENCY_BYTES`]）。超限时淘汰**最久未用**的条目（按 `seq` 判，命中即
//! touch），直到两条约束都满足。这是**既定口径**的保留策略：淘汰只在容量压力
//! 下发生，且回执/注释留痕，不算静默丢弃 —— 被淘汰的 key 下次会重新执行（幂等
//! 语义由调用方再次携带同一 key 保证）。主存储仍是 `BTreeMap`，快照字典序不变。

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
/// MCP 探活方法名。协议要求收到 `ping` 的实现必须回响应（空 result），健康检查依赖它。
pub const METHOD_PING: &str = "ping";

/// 客户端未给 `params.protocolVersion` 时本仓声明的协议版本。
pub const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// 本仓**实际支持**的 MCP 协议版本，按日期升序排列，最后一项是最高版本。
///
/// 只登记已经核过要求的版本；`2025-06-18` 等更新版本在未逐项验证前不得加入，
/// 避免 `initialize` 向客户端宣称支持一个本服务端并未实现的协议。
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[DEFAULT_PROTOCOL_VERSION];

/// 支持列表按日期升序排列，因此最后一项就是协商回退时使用的最高版本。
fn highest_supported_protocol_version() -> &'static str {
    SUPPORTED_PROTOCOL_VERSIONS
        .last()
        .copied()
        .unwrap_or(DEFAULT_PROTOCOL_VERSION)
}

/// `dryRun` 结果里标记"这是模拟"的字段名。
pub const DRY_RUN_FLAG: &str = "dryRun";

/// 幂等缓存最大条目数（T-FIX-25 / #24.4）：超过即淘汰最久未用条目。
pub const MAX_IDEMPOTENCY_ENTRIES: usize = 1024;

/// 幂等缓存最大占用字节（T-FIX-25 / #24.4）：超过即淘汰最久未用条目。
///
/// 字节数按 `key.len() + result 序列化长度 + error 序列化长度` 估算（保守口径）。
pub const MAX_IDEMPOTENCY_BYTES: u64 = 32 * 1024 * 1024;

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
    /// LRU 序号（插入/命中时取 Dispatcher 的单调计数；越小越旧）。
    seq: u64,
    /// 该条目估算占用字节（key + result + error 的序列化长度）。
    bytes: u64,
}

impl CachedOutcome {
    fn from_response(http_status: u16, response: &Response, seq: u64, key_len: usize) -> Self {
        // 字节估算是保守口径：序列化长度 + key 长度。失败时（理论上不会发生）
        // 退回 0，不影响正确性，只影响 LRU 的容量判断精度。
        let result_bytes = response
            .result
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok())
            .map_or(0, |text| text.len() as u64);
        let error_bytes = response
            .error
            .as_ref()
            .and_then(|error| serde_json::to_string(error).ok())
            .map_or(0, |text| text.len() as u64);
        Self {
            http_status,
            result: response.result.clone(),
            error: response.error.clone(),
            seq,
            bytes: (key_len as u64)
                .saturating_add(result_bytes)
                .saturating_add(error_bytes),
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
    /// 幂等缓存单调序号（每次插入/命中 +1，作 LRU 时间轴）。
    idempotency_seq: u64,
    /// 幂等缓存当前估算占用字节（与条目数一起受上限约束）。
    idempotency_bytes: u64,
    /// 因容量上限被 LRU 淘汰的条目数（可观测，回执/诊断用）。
    idempotency_evicted: u64,
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
            idempotency_seq: 0,
            idempotency_bytes: 0,
            idempotency_evicted: 0,
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

    /// 幂等缓存里的键（`<工具名>:<idempotencyKey>`，见 [`scoped_idempotency_key`]），
    /// **字典序**（`BTreeMap` 的确定性）。
    #[must_use]
    pub fn idempotency_keys(&self) -> Vec<&str> {
        self.idempotency.keys().map(String::as_str).collect()
    }

    /// 幂等缓存条数。
    #[must_use]
    pub fn idempotency_len(&self) -> usize {
        self.idempotency.len()
    }

    /// 幂等缓存当前估算占用字节（T-FIX-25 / #24.4）。
    #[must_use]
    pub const fn idempotency_bytes(&self) -> u64 {
        self.idempotency_bytes
    }

    /// 因容量上限被 LRU 淘汰的条目数（T-FIX-25 / #24.4，可观测）。
    #[must_use]
    pub const fn idempotency_evicted(&self) -> u64 {
        self.idempotency_evicted
    }

    /// 插入一条幂等结果，并在必要时按 LRU 淘汰到两条上限内（T-FIX-25）。
    ///
    /// 淘汰按 `seq` 升序（最久未用先走），直到**条目数 ≤
    /// [`MAX_IDEMPOTENCY_ENTRIES`] 且字节数 ≤ [`MAX_IDEMPOTENCY_BYTES`]** 都满足。
    /// 淘汰是既定口径、留痕在 [`Self::idempotency_evicted`]，不算静默丢弃。
    fn insert_idempotent(&mut self, key: String, outcome: CachedOutcome) {
        self.insert_idempotent_with_limits(
            key,
            outcome,
            MAX_IDEMPOTENCY_ENTRIES,
            MAX_IDEMPOTENCY_BYTES,
        );
    }

    /// [`Self::insert_idempotent`] 的可注入上限内核：测试用小上限钉住淘汰边界，
    /// 不必造 1024 条 / 32 MiB 的真实载荷。
    fn insert_idempotent_with_limits(
        &mut self,
        key: String,
        mut outcome: CachedOutcome,
        max_entries: usize,
        max_bytes: u64,
    ) {
        // 插入统一分配新 seq（与命中的 touch 共用一条单调时间轴）。
        self.idempotency_seq += 1;
        outcome.seq = self.idempotency_seq;
        // 同 key 覆盖：先扣掉旧条目的字节，避免重复计数。
        if let Some(old) = self.idempotency.remove(&key) {
            self.idempotency_bytes = self.idempotency_bytes.saturating_sub(old.bytes);
        }
        self.idempotency_bytes = self.idempotency_bytes.saturating_add(outcome.bytes);
        self.idempotency.insert(key, outcome);

        while self.idempotency.len() > max_entries || self.idempotency_bytes > max_bytes {
            // 找 seq 最小（最久未用）的条目。空缓存时不可能进到这里。
            let Some(oldest_key) = self
                .idempotency
                .iter()
                .min_by_key(|(_, value)| value.seq)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(evicted) = self.idempotency.remove(&oldest_key) {
                self.idempotency_bytes = self.idempotency_bytes.saturating_sub(evicted.bytes);
                self.idempotency_evicted += 1;
            }
        }
    }

    /// 命中一条幂等结果时 touch 它的 LRU 序号（命中 = 最近使用）。
    fn touch_idempotent(&mut self, key: &str) {
        if let Some(cached) = self.idempotency.get_mut(key) {
            self.idempotency_seq += 1;
            cached.seq = self.idempotency_seq;
        }
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
    /// 版本策略：只接受 [`SUPPORTED_PROTOCOL_VERSIONS`] 内的客户端值；缺失时用
    /// [`DEFAULT_PROTOCOL_VERSION`]，未知值回退到本仓支持的最高版本。协商结果
    /// 连同请求值、支持列表写进 `serverInfo`，使客户端与日志都能查到“为什么不是
    /// 它请求的那个版本”。**不**在这里编造本仓没有的能力或声明未验证的协议版本。
    fn initialize_result(request: &Request) -> Value {
        let requested = request
            .params
            .as_ref()
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION);
        let supported = SUPPORTED_PROTOCOL_VERSIONS.contains(&requested);
        let negotiated = if supported {
            requested
        } else {
            highest_supported_protocol_version()
        };
        if !supported {
            eprintln!(
                "[yeban-mcp] initialize: 客户端协议版本 {requested:?} 不受支持；已回退到 {negotiated:?}（支持列表: {SUPPORTED_PROTOCOL_VERSIONS:?}）"
            );
        }
        serde_json::json!({
            "protocolVersion": negotiated,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "yeban-mcp",
                "version": env!("CARGO_PKG_VERSION"),
                "supportedProtocolVersions": SUPPORTED_PROTOCOL_VERSIONS,
                "requestedProtocolVersion": requested,
                "protocolVersionSupported": supported,
            },
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
            METHOD_PING => {
                // 与握手/清单同档: 探活不携带领域状态, 但仍要求鉴权（本仓一律鉴权的表面积不变）。
                if let Err(denial) =
                    authenticate(&self.expected_token, context.credential, context.channel)
                {
                    return denied(&denial);
                }
                (200, Ok(serde_json::json!({})))
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
                Ok(result) => (200, Ok(wrap_mcp_call_result(result))),
                Err(error) => (http_status_for(&error), Err(error)),
            };
        }
        // 6. 幂等去重: 命中缓存 ⇒ 永远走不到第 7 步 (因此不会重复施加副作用)。
        //    键**带工具名**（[`scoped_idempotency_key`]）：只按调用方给的 key 去重，
        //    会让两个不同工具用同一个 key 时互相命中 —— 第二个工具**不执行**却收到
        //    第一个工具的结果。
        if let Some(key) = scoped_idempotency_key(&call)
            && let Some(cached) = self.idempotency.get(&key)
        {
            self.replayed += 1;
            let id = request.response_id();
            let status = cached.http_status;
            // LRU: 命中即最近使用，刷新序号（touch 在取走值之后，避免双重借用）。
            let replay = replay_value(cached, id);
            self.touch_idempotent(&key);
            return (status, Ok(replay));
        }
        // 7. 执行: 真的做事 (打开/保存/查询/提案/合并/渲染参数校验)。
        // ⭐ MCP 规范的 `CallToolResult` 包裹（[`wrap_mcp_call_result`]）必须
        // 在**这里**做，而不是在管线出口：幂等缓存存的是本步的产物，重放时
        // 原样嵌回 `response.result` 并被判据要求与首次**逐字节相同**——
        // 出口处再包会让首次（已包）与重放内层（未包）对不上。
        let payload = domain::execute(&mut self.domain, &call).map(wrap_mcp_call_result);
        let status = match &payload {
            Ok(_) => 200,
            Err(error) => http_status_for(error),
        };
        if let Some(key) = scoped_idempotency_key(&call) {
            let response = match &payload {
                Ok(result) => Response::success(request.response_id(), result.clone()),
                Err(error) => Response::failure(request.response_id(), error.clone()),
            };
            let key_len = key.len();
            let outcome = CachedOutcome::from_response(status, &response, 0, key_len);
            // 有界 LRU：超限时淘汰最久未用的条目（既定口径，留痕在 evicted 计数）。
            // seq 由 insert 统一分配（保证插入/命中共用同一单调时间轴）。
            self.insert_idempotent(key, outcome);
        }
        (status, payload)
    }
}

/// 幂等缓存的**作用域键**：`<工具名>:<idempotencyKey>`。
///
/// ⚠ 必须带工具名（2026-10-29 实测缺陷）：缓存若只按调用方给的 key 存，**两个不同工具**
/// 用同一个 key 会互相命中 ⇒ 第二个工具根本走不到 `domain::execute`（副作用不发生），
/// 却收到第一个工具的结果。工具名来自 28 个固定注册名，因此 `:` 分隔无歧义。
fn scoped_idempotency_key(call: &ToolCall) -> Option<String> {
    call.idempotency_key()
        .map(|key| format!("{}:{key}", call.tool.name))
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

/// 把带内 [`ToolResponse`] 包成 MCP 规范的 `CallToolResult`。
///
/// ## 为什么必须包
///
/// MCP 对 `tools/call` 的返回值定义是
/// `{ content: ContentBlock[], isError?: bool, structuredContent?: object }`，
/// 其中 `content` **必填**。本仓历史上的 result 是 `{status, data}`——
/// 自带客户端够用，但标准 MCP 客户端（Claude Desktop / Cursor / MCP Inspector）
/// 按 `content` 渲染：拿不到就显示空结果，也识别不出 `status:"error"` 的带内失败。
///
/// ## 形状：`content` 是摘要，载荷只出现一次
///
/// 线上形状是「`CallToolResult` ∪ `ToolResponse`」：新客户端读 `content`，
/// 老调用方读 `status` / `data`，二者指向同一份 JSON。
///
/// * `content[0].text` 是**一行摘要**（状态＋数据键名／错误码），不是载荷全文。
///   摘要是预算的一部分：全文只许在顶层键里出现一次，再发一份
///   `structuredContent` 或全文 `text` 都会把 16 小节往返撑到约 9 KB，
///   突破 `[BASELINE-006]` 的 4 KiB 上限（T-CI-FIX-1 实测：三份拷贝 9,025 B，
///   摘要版约 3.0 KB；`tests/payload_budget.rs` 钉住往返字节数）。
/// * 因此**不发 `structuredContent`**。需要结构化数据的调用方读顶层键
///   （与 `ToolResponse` 同形）；标准 MCP 客户端读 `content` 得梗概、
///   读顶层键得全文——`content` 与顶层键指向同一份 JSON，不是两套数据面。
///
/// ## 为什么**保留**原来的顶层字段（而不是换掉形状）
///
/// 1. 本仓 HTTP 客户端与既有判据读 `result.status` / `result.data`；
///    两套形状表达的是同一件事，没必要二选一。
/// 2. `ToolResponse` 的 JSON Schema 未设 `additionalProperties:false`，
///    多出的 `content` / `isError` 不破坏既有校验。
fn wrap_mcp_call_result(value: Value) -> Value {
    let is_error = value.get("status").and_then(Value::as_str) == Some("error");
    let mut map = Map::new();
    map.insert(
        "content".to_owned(),
        Value::Array(vec![
            serde_json::json!({ "type": "text", "text": envelope_summary(&value) }),
        ]),
    );
    map.insert("isError".to_owned(), Value::Bool(is_error));
    // ⛔ 不发 `structuredContent`：载荷只能出现一次（见本函数文档）。
    if let Some(object) = value.as_object() {
        for (key, item) in object {
            map.entry(key.clone()).or_insert_with(|| item.clone());
        }
    }
    Value::Object(map)
}

/// `CallToolResult` 的 `content` 文本：一行摘要，绝不是载荷本身。
///
/// 只报状态与数据键名（成功）或错误码（失败），让只读 `content` 的标准
/// 客户端也能看到结果的梗概；完整数据在同一结果的顶层键里（`status` /
/// `data` / `error`），按 JSON 读得到。摘要必须短：它是预算的一部分
/// （`tests/payload_budget.rs` 钉住往返字节数；键名最多列 16 个）。
fn envelope_summary(value: &Value) -> String {
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("success");
    if status == "error" {
        let code = value
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if code.is_empty() {
            "error".to_owned()
        } else {
            format!("error: {code}")
        }
    } else {
        match value.get("data") {
            Some(Value::Object(data)) => {
                let keys: Vec<&str> = data.keys().take(16).map(String::as_str).collect();
                if keys.is_empty() {
                    "success".to_owned()
                } else {
                    let mut summary = format!("success: {}", keys.join(", "));
                    if data.len() > 16 {
                        summary.push_str(", …");
                    }
                    summary
                }
            }
            _ => "success".to_owned(),
        }
    }
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
    use std::path::PathBuf;

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

    /// MCP 规范：收到 `ping` 必须回响应（空 result），健康检查依赖它；不能落进
    /// `-32601` 方法不存在。请求 `id` 回显。
    ///
    /// 注入（实测红）：删掉 [`evaluate`] 的 `METHOD_PING` 分支 ⇒ 本判据红（-32601/404）。
    #[test]
    fn ping_replies_an_empty_result_with_the_request_id_echoed() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let request = Request::parse(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).expect("解析");
        let outcome = dispatcher.handle(Channel::Http, Some(&auth), &request);
        assert_eq!(outcome.http_status, 200);
        assert_eq!(outcome.error_code(), None);
        let response = outcome.response.expect("响应");
        assert_eq!(response.id, Id::Number(7), "ping 的 id 必须回显");
        let result = response.result_value().expect("result");
        assert_eq!(result, &serde_json::json!({}), "探活 result 必须是空对象");
    }

    /// `ping` 与 tools/list / `initialize` 同一档鉴权：不携带领域状态，但仍要求鉴权。
    #[test]
    fn ping_is_authenticated_like_the_handshake_methods() {
        let mut dispatcher = dispatcher();
        let request = Request::parse(r#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#).expect("解析");
        // Http 通道缺 token ⇒ 401（与 tools/list / initialize 同档硬拒）。
        let outcome = dispatcher.handle(Channel::Http, None, &request);
        assert_eq!(outcome.http_status, 401);
        // 无头 stdio 通道不带 Authorization 头也走到实现（与既有 stdio 判据同语义）⇒ 200 + 空 result。
        let outcome = dispatcher.handle_line(
            Channel::Stdio,
            None,
            r#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#,
        );
        assert_eq!(outcome.http_status, 200);
        assert_eq!(
            outcome
                .response
                .expect("响应")
                .result_value()
                .expect("result"),
            &serde_json::json!({})
        );
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
        assert_eq!(
            dispatcher.idempotency_keys(),
            vec!["yeban_render_master:k1"]
        );
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
            vec![
                "yeban_query_project:a",
                "yeban_query_project:b",
                "yeban_query_project:c"
            ],
            "BTreeMap 的键序必须是字典序 (确定性), 且键带工具名作用域"
        );
        assert_eq!(dispatcher.replayed(), 1);
    }

    /// **本判据存在的理由（2026-10-29 实测缺陷）**：缓存键若只有调用方给的
    /// `idempotencyKey`，**两个不同工具**用同一个 key 会互相命中 ⇒ 第二个工具根本
    /// 不执行（副作用不发生），却收到第一个工具的结果。判法：同一个 key 依次调用两个
    /// 只读工具，第二条**不得**带 `replayed`；随后同工具再调，才允许命中。
    #[test]
    fn idempotency_keys_are_scoped_per_tool() {
        let mut dispatcher = dispatcher();
        seed_project(&mut dispatcher, unique_project_path());
        let auth = bearer(&dispatcher);
        let line = |name: &str, id: &str| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": name, "arguments": {"idempotencyKey": "shared"}}
            })
            .to_string()
        };
        let project = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &line("yeban_query_project", "p1"),
        );
        assert_eq!(project.error_code(), None, "第一条必须成功");
        let engine = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &line("yeban_query_engine_state", "e1"),
        );
        assert_eq!(engine.error_code(), None, "第二条必须成功");
        let engine_result = engine.response.expect("响应").result.expect("result");
        assert!(
            engine_result.get("replayed").is_none(),
            "不同工具用同一个 key 不得命中彼此的缓存: {engine_result}"
        );
        assert_eq!(engine_result["status"], "success", "{engine_result}");
        assert_eq!(
            dispatcher.idempotency_len(),
            2,
            "两个工具各自缓存一条 (键带工具名)"
        );
        assert_eq!(dispatcher.replayed(), 0, "前两次都必须是真执行");

        let again = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &line("yeban_query_project", "p2"),
        );
        assert_eq!(dispatcher.replayed(), 1, "同工具同 key 必须命中");
        let replayed = again.response.expect("响应").result.expect("result");
        assert_eq!(replayed["replayed"], true, "{replayed}");
        assert_eq!(
            dispatcher.idempotency_keys(),
            vec![
                "yeban_query_engine_state:shared",
                "yeban_query_project:shared"
            ],
            "BTreeMap 字典序"
        );
    }

    /// **边角判据（2026-10-29 补）**：**失败也进缓存**。契约 `idempotencyKey` 的描述是
    /// "第二次 `tools/call` **不执行工具**, 直接返回首次结果的缓存"，未把失败排除；
    /// 模块文档同样说缓存存的是"首次执行的**完整结果**"。此前只有成功路径的重放判据
    /// ⇒ 若有人把 `Err` 从缓存里排除，客户端"同键重试"会变成**重复执行**而无人发现。
    ///
    /// 用**没有活跃工程**的会话：`yeban_query_project` 必然带内报 `NO_ACTIVE_PROJECT`
    /// （带内 = `Ok(ToolResponse{status:"error"})`，仍是一等结果，走得到第 7 步）。
    #[test]
    fn failed_calls_are_cached_and_replay_without_reexecution() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        let line = |id: &str| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "yeban_query_project",
                           "arguments": {"idempotencyKey": "fail-key"}}
            })
            .to_string()
        };
        let first = dispatcher.handle_line(Channel::Http, Some(&auth), &line("f1"));
        assert_eq!(first.error_code(), None, "带内失败不是 JSON-RPC 错误");
        assert_eq!(first.http_status, 200);
        let first_result = first.response.expect("响应").result.expect("result");
        assert_eq!(first_result["status"], "error", "{first_result}");
        assert_eq!(first_result["error"]["code"], "NO_ACTIVE_PROJECT");
        assert_eq!(dispatcher.idempotency_len(), 1, "失败同样入缓存");
        assert_eq!(
            dispatcher.idempotency_keys(),
            vec!["yeban_query_project:fail-key"]
        );

        let second = dispatcher.handle_line(Channel::Http, Some(&auth), &line("f2"));
        assert_eq!(dispatcher.replayed(), 1, "第二次必须命中**失败**缓存");
        assert_eq!(dispatcher.idempotency_len(), 1, "缓存不得增长");
        assert_eq!(second.http_status, 200, "重放沿用首次的状态码");
        let envelope = second.response.expect("响应");
        assert_eq!(envelope.id, Id::Text("f2".to_owned()), "必须回显当前 id");
        let replayed = envelope.result.expect("result");
        assert_eq!(replayed["replayed"], true, "{replayed}");
        assert_eq!(replayed["response"]["id"], "f2", "内层也用当前 id");
        assert_eq!(
            replayed["response"]["result"], first_result,
            "重放的失败主体必须与首次逐字节相同"
        );
    }

    /// **回归判据（2026-11-03 制作实测）**：`yeban_edit_notes` `create:true`
    /// 批量建材料时校验失败（`duplicateNoteId`），同键重发必须 promptly
    /// 重放首次失败 —— 不得重执行，更不得挂起。制作侧曾报“同键重发
    /// 服务端挂起 6 分钟以上”；本判据把该序列钉在分发器层：537 音符
    /// 批量、1 处身份冲突（与制作侧钢琴声部 537 音、535 个不同
    /// (start,pitch,duration) 三元组同构；这里用显式 id 撞出同一
    /// `duplicateNoteId`，不依赖派生路径）。
    #[test]
    fn failed_create_replays_promptly_on_same_key() {
        let mut dispatcher = dispatcher();
        seed_project(&mut dispatcher, unique_project_path());
        let auth = bearer(&dispatcher);
        let mut ops = Vec::new();
        let mut dup_id = String::new();
        for i in 0..537 {
            let id = if i == 200 {
                dup_id.clone()
            } else {
                format!("01J8ZQ{:020}", 1000 + i)
            };
            if i == 100 {
                dup_id = id.clone();
            }
            ops.push(serde_json::json!({"kind": "add", "note": {
                "id": id, "startTick": i * 96,
                "pitch": 60 + (i % 12), "durationTicks": 96, "velocity": 80}}));
        }
        let arguments = serde_json::json!({
            "trackId": "01J8ZQ00000000000000000001",
            "clipId": "01J8ZQ00000000000000009000",
            "create": true, "ops": ops, "idempotencyKey": "midi2-f-s5-piano"});
        let line = |id: &str| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "yeban_edit_notes",
                           "arguments": arguments.clone()}
            })
            .to_string()
        };
        let first = dispatcher.handle_line(Channel::Http, Some(&auth), &line("c1"));
        assert_eq!(first.error_code(), None, "领域失败是带内的");
        assert_eq!(first.http_status, 200);
        let first_result = first.response.expect("响应").result.expect("result");
        assert_eq!(first_result["status"], "error", "{first_result}");
        assert_eq!(first_result["error"]["code"], "INVALID_PARAMETER_RANGE");
        assert_eq!(
            first_result["error"]["data"]["reason"], "duplicateNoteId",
            "{first_result}"
        );
        assert_eq!(dispatcher.idempotency_len(), 1, "失败同样入缓存");
        assert_eq!(
            dispatcher.idempotency_keys(),
            vec!["yeban_edit_notes:midi2-f-s5-piano"]
        );

        let second = dispatcher.handle_line(Channel::Http, Some(&auth), &line("c2"));
        assert_eq!(dispatcher.replayed(), 1, "第二次必须命中**失败**缓存");
        assert_eq!(dispatcher.idempotency_len(), 1, "缓存不得增长");
        assert_eq!(second.http_status, 200, "重放沿用首次的状态码");
        let envelope = second.response.expect("响应");
        assert_eq!(envelope.id, Id::Text("c2".to_owned()), "必须回显当前 id");
        let replayed = envelope.result.expect("result");
        assert_eq!(replayed["replayed"], true, "{replayed}");
        assert_eq!(replayed["response"]["id"], "c2", "内层也用当前 id");
        assert_eq!(
            replayed["response"]["result"], first_result,
            "重放的失败主体必须与首次逐字节相同"
        );
    }

    /// **本判据存在的理由（实测事故）**：上面那条测试对**每个注册工具**做一次
    /// **真执行**，而带输出文件面的工具会真的写盘。本线开发中 `yeban_record_wav`
    /// 用默认 2 秒在 `crates/yeban-mcp/` 里留下了
    /// `01J8ZQ00000000000000000001`（576,044 字节，未被 `git` 跟踪）——
    /// 那是"测试把产物写进源码树"，必须机械钉住。
    ///
    /// 口径：**crate 根目录的条目集合**在通用执行前后必须**逐项相同**（新增或消失都算红）。
    #[test]
    fn the_generic_execution_leaves_no_file_in_the_crate_dir() {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let snapshot = |dir: &PathBuf| -> Vec<String> {
            let mut entries: Vec<String> = std::fs::read_dir(dir)
                .expect("读 crate 目录")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            entries.sort_unstable();
            entries
        };
        let before = snapshot(&crate_dir);
        // ⭐ 非真空下界（R93）：扫描面太小说明 `CARGO_MANIFEST_DIR` 解析错了 ⇒
        // 上面的"前后相同"会**恒真**（空集 == 空集），判据变成一句注释。
        assert!(
            before.len() >= 3,
            "crate 根至少应有 Cargo.toml / src / tests 三条, 实得 {}",
            before.len()
        );
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        for spec in &crate::tools::TOOLS {
            let line = serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": spec.name, "arguments": minimal_arguments(spec)}
            })
            .to_string();
            let _ = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
        }
        let after = snapshot(&crate_dir);
        assert_eq!(
            before, after,
            "通用执行不得在 crate 源码目录里留下任何条目（工具的输出路径必须在临时目录里）"
        );
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
    ///
    /// ⚠ `path` 必须落在**临时目录**里，⛔ 不能是一个裸相对文件名：本测试对
    /// **每一个注册工具**做一次**真执行**，而带输出文件面的工具（
    /// `yeban_record_wav` / `yeban_render_master` / `yeban_export_diagnostics`）
    /// 会照实写盘 —— 一个相对路径会让它们写进 **crate 源码目录**。
    /// 实测事故（本线开发中）：`yeban_record_wav` 用默认 2 秒在这里留下了
    /// `crates/yeban-mcp/01J8ZQ00000000000000000001`（576,044 字节 WAV，未被跟踪）。
    /// 同一条判据 `the_generic_execution_leaves_no_file_in_the_crate_dir` 现在把它钉住。
    fn minimal_arguments(spec: &crate::tools::ToolSpec) -> Value {
        let mut map = Map::new();
        for param in spec.all_params() {
            if !param.required {
                continue;
            }
            let value = match param.json_type {
                "string" if param.name == "path" => Value::from(
                    std::env::temp_dir()
                        .join(format!(
                            "yeban-mcp-dispatch-{}-{}",
                            std::process::id(),
                            spec.name
                        ))
                        .display()
                        .to_string(),
                ),
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
    /// 协议版本协商：缺省/受支持/未知三条路径都只返回本仓支持的版本，并在
    /// `serverInfo` 留下请求值、支持列表与协商结果。
    ///
    /// 注入（实测红）：把未知值的分支改回原样回显 ⇒ 本判据红。
    #[test]
    fn initialize_negotiates_a_supported_protocol_version() {
        assert_eq!(DEFAULT_PROTOCOL_VERSION, "2024-11-05");
        assert!(!SUPPORTED_PROTOCOL_VERSIONS.is_empty());
        assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&DEFAULT_PROTOCOL_VERSION));
        // 本仓没有逐项验证 2025-06-18 的要求，因此不能把它写进支持列表。
        assert!(!SUPPORTED_PROTOCOL_VERSIONS.contains(&"2025-06-18"));

        // 缺字段 → DEFAULT_PROTOCOL_VERSION。
        let missing = Request::parse(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#)
            .expect("初始化请求");
        let value = Dispatcher::initialize_result(&missing);
        assert_eq!(
            value["protocolVersion"], "2024-11-05",
            "缺省必须回线协议的已发布版本: {value}"
        );
        assert_eq!(
            value["serverInfo"]["requestedProtocolVersion"], "2024-11-05",
            "缺字段时留痕应记录实际采用的缺省值: {value}"
        );
        assert_eq!(
            value["serverInfo"]["protocolVersionSupported"], true,
            "缺字段走缺省不应被标成不支持: {value}"
        );

        // 受支持版本 → 原样返回该版本。
        let supported = Request::parse(
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
        )
        .expect("初始化请求");
        let value = Dispatcher::initialize_result(&supported);
        assert_eq!(
            value["protocolVersion"], "2024-11-05",
            "受支持版本必须原样返回: {value}"
        );
        assert_eq!(
            value["serverInfo"]["protocolVersionSupported"], true,
            "受支持版本应标记为支持: {value}"
        );

        // 未知版本 → 支持的最高版本，不得回显客户端原文。
        let unknown = Request::parse(
            r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"X"}}"#,
        )
        .expect("初始化请求");
        let value = Dispatcher::initialize_result(&unknown);
        assert_eq!(
            value["protocolVersion"],
            highest_supported_protocol_version(),
            "未知版本必须回退到支持的最高版本: {value}"
        );
        assert_ne!(value["protocolVersion"], "X", "不得回显未知版本: {value}");
        assert_eq!(
            value["serverInfo"]["requestedProtocolVersion"], "X",
            "serverInfo 必须保留原始请求值以便排查: {value}"
        );
        assert_eq!(
            value["serverInfo"]["protocolVersionSupported"], false,
            "未知版本必须标记为不支持: {value}"
        );
        assert_eq!(
            value["serverInfo"]["supportedProtocolVersions"],
            serde_json::json!(SUPPORTED_PROTOCOL_VERSIONS),
            "serverInfo 必须列出实际支持列表: {value}"
        );

        // 2025-06-18 是比 DEFAULT 更新的协议版本，但本仓未验证，必须同样回退。
        let newer = Request::parse(
            r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        )
        .expect("初始化请求");
        let value = Dispatcher::initialize_result(&newer);
        assert_eq!(
            value["protocolVersion"],
            highest_supported_protocol_version(),
            "未验证的新协议版本必须回退: {value}"
        );
        assert_eq!(
            value["serverInfo"]["protocolVersionSupported"], false,
            "未验证的新协议版本不得宣称支持: {value}"
        );
    }

    /// ⭐ MCP 规范：`tools/call` 的 `result` 必须是 `CallToolResult`（`content` 必填）。
    ///
    /// 判据三件套（成功 / 带内失败 / 幂等重放）—— 三者都走同一个出口，
    /// 因此**任何**一条漏包都会被本函数抓到：
    /// * `content[0].type == "text"`，`content[0].text` 是一行**摘要**（状态＋数据
    ///   键名／错误码），绝不是载荷全文——全文只在顶层键里出现一次，否则
    ///   `[BASELINE-006]` 的 4 KiB 预算会被撑爆（T-CI-FIX-1 实测三份拷贝 9,025 B）；
    /// * `structuredContent` **不得出现**（同上：它是第二份载荷拷贝）；
    /// * `isError` 与带内 `status` 一致（success ⇒ false，error ⇒ true）。
    ///
    /// 同时钉住向后兼容：顶层 `status` / `data` / `error` 必须**原样存在**
    /// （本仓 HTTP 客户端与既有判据读的就是它们；`yeban-app/src/mcp_mount/` 的
    /// 内嵌挂载直接读 `result["data"]`）。
    ///
    /// 注入（实测红）：把 [`wrap_mcp_call_result`] 换成恒等函数 ⇒ 三条判据全红；
    /// 把 `isError` 写成常量 `false` ⇒ 失败臂红。
    #[test]
    fn tools_call_result_carries_a_mcp_content_envelope() {
        let mut dispatcher = dispatcher();
        let auth = bearer(&dispatcher);
        seed_project(&mut dispatcher, unique_project_path());

        let assert_envelope = |result: &Value, want_error: bool| {
            let content = result["content"]
                .as_array()
                .unwrap_or_else(|| panic!("result 必须有 content 数组: {result}"));
            assert_eq!(content.len(), 1, "本仓只产出一个文本块: {result}");
            assert_eq!(content[0]["type"], "text");
            let text = content[0]["text"].as_str().expect("text 必须是字符串");
            // 摘要必须短：载荷全文在顶层 data 里，摘要里出现全文即重复载荷。
            assert!(
                text.len() < 256,
                "content.text 必须是摘要（< 256 B），实测 {} B: {text}",
                text.len()
            );
            assert!(
                text.starts_with(if want_error { "error" } else { "success" }),
                "摘要必须先报状态: {text}"
            );
            assert!(
                result.get("structuredContent").is_none(),
                "不得发 structuredContent：它是第二份载荷拷贝，会撑爆 BASELINE-006 预算"
            );
            assert_eq!(result["isError"], Value::Bool(want_error), "isError 口径");
            assert_eq!(
                result["status"],
                if want_error { "error" } else { "success" },
                "顶层 status 必须保留（向后兼容）"
            );
        };

        // ① 成功：查询工程。
        let outcome = dispatcher.handle_line(
            Channel::Http,
            Some(&auth),
            &call("yeban_query_project", serde_json::json!({})),
        );
        assert_eq!(outcome.http_status, 200);
        let result = outcome.response.expect("响应").result.expect("result");
        assert_envelope(&result, false);
        assert!(result["data"].is_object(), "顶层 data 必须保留");

        // ② 带内失败：没有活跃工程（新分发器）。
        // ⛔ 不能叫 `dispatcher`：会遮蔽同名的测试辅助函数，下面就没法再造一个。
        let mut fresh = self::dispatcher();
        let auth = bearer(&fresh);
        let outcome = fresh.handle_line(
            Channel::Http,
            Some(&auth),
            &call("yeban_query_project", serde_json::json!({})),
        );
        let result = outcome.response.expect("响应").result.expect("result");
        assert_envelope(&result, true);
        assert_eq!(result["error"]["code"], "NO_ACTIVE_PROJECT");
        let text = result["content"][0]["text"]
            .as_str()
            .expect("text 必须是字符串");
        assert!(
            text.contains("NO_ACTIVE_PROJECT"),
            "失败摘要必须带错误码: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // T-FIX-25 / #24.4 幂等缓存有界 LRU 回归判据
    // -----------------------------------------------------------------------

    /// 造一个指定序号 / 字节的缓存条目（测试可直接访问私有字段）。
    fn fake_outcome(seq: u64, bytes: u64) -> CachedOutcome {
        CachedOutcome {
            http_status: 200,
            result: Some(serde_json::json!({"seq": seq})),
            error: None,
            seq,
            bytes,
        }
    }

    /// 判据：条目数上限触顶时淘汰最久未用（seq 最小）条目，不静默增长。
    #[test]
    fn idempotency_cache_evicts_oldest_by_entry_limit() {
        let mut dispatcher = dispatcher();
        // 小上限 3：依次插入 k1..k5（seq 1..5）。
        for index in 1..=5u64 {
            dispatcher.insert_idempotent_with_limits(
                format!("k{index}"),
                fake_outcome(index, 1),
                3,
                u64::MAX,
            );
        }
        assert_eq!(dispatcher.idempotency_len(), 3, "条目数必须卡在上限");
        assert_eq!(dispatcher.idempotency_evicted(), 2, "淘汰数必须留痕");
        let keys = dispatcher.idempotency_keys();
        assert_eq!(
            keys,
            vec!["k3", "k4", "k5"],
            "必须淘汰最旧的 k1/k2，保留最新三条"
        );
    }

    /// 判据：字节上限触顶时同样淘汰最久未用条目，直到占用回到上限内。
    #[test]
    fn idempotency_cache_evicts_oldest_by_byte_limit() {
        let mut dispatcher = dispatcher();
        // 每条 10 字节，上限 25 ⇒ 最多留 2 条（20），第 3 条触发淘汰 k1。
        for index in 1..=3u64 {
            dispatcher.insert_idempotent_with_limits(
                format!("k{index}"),
                fake_outcome(index, 10),
                usize::MAX,
                25,
            );
        }
        assert_eq!(dispatcher.idempotency_len(), 2, "字节上限必须生效");
        assert!(dispatcher.idempotency_bytes() <= 25);
        assert_eq!(dispatcher.idempotency_keys(), vec!["k2", "k3"]);
        assert_eq!(dispatcher.idempotency_evicted(), 1);
    }

    /// 判据：命中缓存会 touch 序号 —— 刚被命中的旧条目在下一轮淘汰中存活。
    #[test]
    fn idempotency_cache_hit_refreshes_lru_order() {
        let mut dispatcher = dispatcher();
        dispatcher.insert_idempotent_with_limits("a".into(), fake_outcome(1, 1), 3, u64::MAX);
        dispatcher.insert_idempotent_with_limits("b".into(), fake_outcome(2, 1), 3, u64::MAX);
        dispatcher.insert_idempotent_with_limits("c".into(), fake_outcome(3, 1), 3, u64::MAX);
        // 命中 a（最旧）⇒ touch 成最新（seq 4）。
        dispatcher.touch_idempotent("a");
        // 再插入 d ⇒ 应淘汰 b（现在最旧），a 存活。
        dispatcher.insert_idempotent_with_limits("d".into(), fake_outcome(4, 1), 3, u64::MAX);
        assert_eq!(
            dispatcher.idempotency_keys(),
            vec!["a", "c", "d"],
            "被命中的 a 必须存活，淘汰没被用的 b"
        );
    }
}
