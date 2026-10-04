//! 控制面管线：**方法解析 → 授权（生产硬禁 → token → scope）→ 参数校验 → 执行 → 返回**。
//!
//! `[UI-MCP-001]` §12.3 / `[ARCH-UI-004]` / `[ARCH-SEC-002]`。
//!
//! ## 管线顺序本身就是判据
//!
//! ```text
//! handle_line (原始文本入口, stdio 与 HTTP body 共用)
//!   0. token 鉴权        缺 / 形状非法 / 不匹配 → 401 (id=null, **请求体一个字节都不解析**)
//!   1. JSON 解析         非法 → -32700
//!   ──> handle (已解析)
//!        2. 方法解析      不在注册表 → -32601
//!        3. authorize     ui:inject 生产硬禁(403) → token(401) → scope(403)
//!        4. 参数校验      未知/缺失/类型/白名单 → -32602
//!        5. 执行          经 [`UiSurface`] 落到 `yeban-ui-test-port`
//! ```
//!
//! ### 为什么 `authorize` 是**同一个函数**而不是"照抄一遍"
//!
//! 第 3 步调用的是 `yeban_mcp::security::authorize` —— `mcp-core` 那条线用的**同一个**
//! 纯函数。它内部的三步顺序（生产硬禁 → 鉴权 → scope）就是 `ARCH-SEC-002` 的
//! "生产环境硬编码封禁 `ui:inject`"的落点。**不重写**是刻意的：
//! 两套 MCP 若各写一遍顺序，"顺手把硬禁挪到 token 后面"这种改动只会在一侧发生，
//! 而另一侧看起来仍然是对的。
//!
//! ### 判据 ③"注入硬禁先于 token 校验"落在哪
//!
//! 在**已解析**入口上，`ui:inject` 在生产模式下即便是**没有 token**的请求也会拿到
//! `403 forbidden-in-production`（而不是 `401`）—— 因为 `authorize` 的第一件事就是硬禁。
//! 判据 `event_injection_is_hard_denied_in_production_before_token_check` 用三种凭据
//! （无 / 错 / 合法）逐一钉住这件事。
//!
//! ### 两道入口的差异（**这是有意的**，不是不一致）
//!
//! | 入口 | 未鉴权调用方能得到什么 |
//! | :--- | :--- |
//! | [`UiService::handle_line`]（stdio / HTTP，外部可达） | 只有 `401`。方法名不会被解析，因此**没有方法集探测面**（与 `mcp-core` 的 M3 同口径） |
//! | [`UiService::handle`]（进程内 / 库形态） | `403`/`404` 都可能 —— 调用方**已经在本进程里**，信息面不是安全边界；换来的是"硬禁先于 token 校验"这条可判定的性质 |
//!
//! 判据 `raw_entry_point_never_leaks_the_method_set` 把第一条钉住：
//! 不带 token 调一个**不存在**的方法，必须拿到 `401` 而不是 `-32601`。
//!
//! ## 错误码（哪些是复用的、哪些是本线新增的）
//!
//! 复用 `yeban_mcp::jsonrpc` 的常量（**不重新定义**，否则两套 MCP 会漂移）：
//! `-32700` / `-32600` / `-32601` / `-32602` / `-32603` / `-32001`（未鉴权）/
//! `-32003`（已鉴权未授权）/ `-32005`（能力尚未接线）。
//! 本线新增三个（JSON-RPC 的 `-32000..=-32099` 是服务自定义区）：
//!
//! | 码 | 常量 | 含义 | HTTP |
//! | :--- | :--- | :--- | ---: |
//! | `-32006` | [`ELEMENT_NOT_FOUND`] | 语义 ID 不在控件树里（§12.2：找不到就是找不到） | 400 |
//! | `-32008` | [`CAPTURE_FAILED`] | Tier-1 光栅化/编码失败（含 `[MUST-GATE-015]` 的尺寸非零且非全黑门槛） | 500 |
//! | `-32009` | [`GEOMETRY_UNAVAILABLE`] | 元素存在但没有几何包围盒（`[UI-MCP-002]` 的遮罩无从执行） | 400 |
//!
//! **绝不**把这三个塞进 `schemas/mcp-tools.schema.json` 的 `ToolResponse.error.code`：
//! 那是**领域**契约的闭合 enum（ADR-0001 D25），UI 控制面的实现级状况不属于它
//! （与 `mcp-core` notes §2 M10 同一口径）。

use serde_json::{Map, Value};

use yeban_mcp::dispatch::Outcome;
use yeban_mcp::jsonrpc::{
    self, ErrorObject, FORBIDDEN, INVALID_PARAMS, Id, NOT_IMPLEMENTED, Request, Response,
};
use yeban_mcp::security::{
    AuthContext, BearerToken, Channel, Denial, RunMode, ScopeSet, authenticate, authorize,
};
use yeban_ui_test_port::port::{KeyCode, PointerButton, PortError};
use yeban_ui_test_port::{image::Rect, mask};

use crate::methods::{self, MethodSpec};
use crate::surface::{DEFAULT_MAX_PNG_BYTES, ShotEvidence, UiSurface, encode_with_evidence};
use crate::tree::{Coverage, UiTree};

/// 语义 ID 不在控件树里（本线新增；见模块文档的错误码表）。
pub const ELEMENT_NOT_FOUND: i64 = -32006;
/// Tier-1 抓帧 / PNG 编码失败（本线新增）。
pub const CAPTURE_FAILED: i64 = -32008;
/// 元素存在但没有几何包围盒（本线新增）。
pub const GEOMETRY_UNAVAILABLE: i64 = -32009;

/// 服务标识（进 `ui/methods` 与日志）。
pub const SERVICE_NAME: &str = "yeban-ui-mcp";

/// UI 控制面服务：一个执行面 + 一套凭据/作用域/模式。
pub struct UiService {
    surface: Box<dyn UiSurface>,
    expected_token: BearerToken,
    granted: ScopeSet,
    mode: RunMode,
    handled: u64,
    denied: u64,
}

impl UiService {
    /// 构造。
    #[must_use]
    pub fn new(
        expected_token: BearerToken,
        granted: ScopeSet,
        mode: RunMode,
        surface: Box<dyn UiSurface>,
    ) -> Self {
        Self {
            surface,
            expected_token,
            granted,
            mode,
            handled: 0,
            denied: 0,
        }
    }

    /// 期望的令牌（**唯一**的凭据比对对象）。
    #[must_use]
    pub fn expected_token(&self) -> &BearerToken {
        &self.expected_token
    }

    /// 授予的作用域。
    #[must_use]
    pub fn granted(&self) -> &ScopeSet {
        &self.granted
    }

    /// 运行模式（生产是默认）。
    #[must_use]
    pub const fn mode(&self) -> RunMode {
        self.mode
    }

    /// 已处理的请求数。
    #[must_use]
    pub const fn handled(&self) -> u64 {
        self.handled
    }

    /// 被拒的请求数。
    #[must_use]
    pub const fn denied(&self) -> u64 {
        self.denied
    }

    /// 执行面标识。
    #[must_use]
    pub fn surface_name(&self) -> &'static str {
        self.surface.surface_name()
    }

    /// 借出执行面（只读判据与嵌入式用法）。
    #[must_use]
    pub fn surface(&self) -> &dyn UiSurface {
        self.surface.as_ref()
    }

    /// 借出执行面（可变）。
    pub fn surface_mut(&mut self) -> &mut dyn UiSurface {
        self.surface.as_mut()
    }

    /// **原始文本入口**（stdio 与 HTTP body 共用）：鉴权**先于**解析。
    ///
    /// 未鉴权的调用方不是 JSON-RPC 对端，因此拿到的 `401` 里 `id` 一定是 `null`
    /// （无从回显），而且请求体**一个字节都不会被解释**。这一条把"用错误码差异探测
    /// 本机注册了哪些方法"堵死（判据 `raw_entry_point_never_leaks_the_method_set`）。
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
            self.denied += 1;
            let (status, payload) = denied(&denial);
            return outcome(status, Id::Null, payload);
        }
        match Request::parse(line) {
            Ok(request) => {
                // 不要在两条入口上把同一个请求数成两次。
                self.handled -= 1;
                self.handle(channel, authorization, &request)
            }
            Err(error) => {
                self.denied += 1;
                let status = http_status_for(&error);
                outcome(status, Id::Null, Err(error))
            }
        }
    }

    /// **已解析入口**：方法解析 → `authorize` → 参数 → 执行。
    pub fn handle(
        &mut self,
        channel: Channel,
        authorization: Option<&str>,
        request: &Request,
    ) -> Outcome {
        self.handled += 1;
        let context =
            AuthContext::from_headers(channel, authorization, self.granted.clone(), self.mode);
        let (status, payload) = self.evaluate(&context, request);
        if request.is_notification() {
            // notification: 规范要求不产生响应。副作用照常发生 (JSON-RPC 的语义)。
            return Outcome::silent(status);
        }
        if payload.is_err() {
            self.denied += 1;
        }
        outcome(status, request.response_id(), payload)
    }

    /// 管线本体。
    fn evaluate(
        &mut self,
        context: &AuthContext<'_>,
        request: &Request,
    ) -> (u16, Result<Value, ErrorObject>) {
        // 2. 方法解析。
        let Some(spec) = methods::method(&request.method) else {
            let error = ErrorObject::method_not_found(&request.method);
            return (http_status_for(&error), Err(error));
        };
        // 3. 授权: 生产硬禁 → token → scope（`yeban_mcp::security::authorize` 的内部顺序）。
        if let Err(denial) = authorize(&self.expected_token, context, spec.scope) {
            return denied(&denial);
        }
        // 4. 参数校验。
        let params = match methods::validate_params(spec, request.params.as_ref()) {
            Ok(params) => params,
            Err(error) => {
                let error = ErrorObject::invalid_params(error.detail);
                return (http_status_for(&error), Err(error));
            }
        };
        // 5. 执行。
        let payload = self.execute(spec, &params);
        let status = match &payload {
            Ok(_) => 200,
            Err(error) => http_status_for(error),
        };
        (status, payload)
    }

    /// 方法实现（每一条都必须落到 [`UiSurface`]，**不允许**在这里编造数据）。
    fn execute(
        &mut self,
        spec: &'static MethodSpec,
        params: &Map<String, Value>,
    ) -> Result<Value, ErrorObject> {
        match spec.name {
            methods::METHOD_METHODS => Ok(self.describe()),
            methods::METHOD_TREE => {
                let projection = UiTree::from_runtime(self.surface.tree());
                let prefix = text_param(params, "prefix");
                let dynamic_only = bool_param(params, "dynamicOnly").unwrap_or(false);
                let total = projection.count;
                let filtered = projection.filtered(prefix, dynamic_only);
                let mut filter = Map::new();
                filter.insert("prefix".to_owned(), prefix.map_or(Value::Null, Value::from));
                filter.insert("dynamicOnly".to_owned(), Value::from(dynamic_only));
                filter.insert("totalCount".to_owned(), Value::from(total));
                let mut root = Map::new();
                root.insert("filter".to_owned(), Value::Object(filter));
                root.insert("tree".to_owned(), json_of(&filtered));
                Ok(Value::Object(root))
            }
            methods::METHOD_NODE => {
                let id = required_text(params, "elementId");
                let projection = UiTree::from_runtime(self.surface.tree());
                let node = projection.find(&id).ok_or_else(|| element_not_found(&id))?;
                let mut root = Map::new();
                root.insert("node".to_owned(), json_of(node));
                Ok(Value::Object(root))
            }
            methods::METHOD_PROPERTY => {
                let id = required_text(params, "elementId");
                let name = required_text(params, "name");
                let value = self
                    .surface
                    .read_property(&id, &name)
                    .map_err(|error| port_error(PortContext::Read, error))?;
                let mut root = Map::new();
                root.insert("id".to_owned(), Value::from(id));
                root.insert("name".to_owned(), Value::from(name));
                root.insert("value".to_owned(), Value::from(value));
                Ok(Value::Object(root))
            }
            methods::METHOD_DYNAMIC_REGIONS => {
                let projection = UiTree::from_runtime(self.surface.tree());
                let rects = projection
                    .mask_rects()
                    .map_err(|error| geometry_unavailable(&error.to_string()))?;
                let regions = rects
                    .into_iter()
                    .map(|(id, bounds)| {
                        let mut entry = Map::new();
                        entry.insert("id".to_owned(), Value::from(id));
                        entry.insert("bounds".to_owned(), json_of(&bounds));
                        entry.insert("maskable".to_owned(), Value::from(true));
                        Value::Object(entry)
                    })
                    .collect::<Vec<_>>();
                let mut root = Map::new();
                root.insert("count".to_owned(), Value::from(regions.len()));
                root.insert("regions".to_owned(), Value::Array(regions));
                Ok(Value::Object(root))
            }
            methods::METHOD_SCREENSHOT => self.screenshot(params),
            methods::METHOD_COVERAGE => {
                let ids = registry_ids(params)?;
                let projection = UiTree::from_runtime(self.surface.tree());
                Ok(json_of(&Coverage::between(&projection, &ids)))
            }
            methods::METHOD_DISPATCH_POINTER_DOWN => {
                let id = required_text(params, "elementId");
                let x = required_number(params, "xOffset");
                let y = required_number(params, "yOffset");
                let button = button_param(params)?;
                self.surface
                    .dispatch_pointer_down(&id, x, y, button)
                    .map_err(|error| port_error(PortContext::Inject, error))?;
                let mut root = Map::new();
                root.insert("dispatched".to_owned(), Value::from("pointer_down"));
                root.insert("elementId".to_owned(), Value::from(id));
                root.insert("xOffset".to_owned(), Value::from(x));
                root.insert("yOffset".to_owned(), Value::from(y));
                root.insert("button".to_owned(), Value::from(button.as_str()));
                Ok(Value::Object(root))
            }
            methods::METHOD_DISPATCH_POINTER_MOVE => {
                let x = required_number(params, "x");
                let y = required_number(params, "y");
                self.surface
                    .dispatch_pointer_move(x, y)
                    .map_err(|error| port_error(PortContext::Inject, error))?;
                let mut root = Map::new();
                root.insert("dispatched".to_owned(), Value::from("pointer_move"));
                root.insert("x".to_owned(), Value::from(x));
                root.insert("y".to_owned(), Value::from(y));
                Ok(Value::Object(root))
            }
            methods::METHOD_DISPATCH_POINTER_UP => {
                let button = button_param(params)?;
                self.surface
                    .dispatch_pointer_up(button)
                    .map_err(|error| port_error(PortContext::Inject, error))?;
                let mut root = Map::new();
                root.insert("dispatched".to_owned(), Value::from("pointer_up"));
                root.insert("button".to_owned(), Value::from(button.as_str()));
                Ok(Value::Object(root))
            }
            methods::METHOD_DISPATCH_KEY_PRESS => {
                let raw = required_text(params, "keyCode");
                let key =
                    KeyCode::parse(&raw).map_err(|error| port_error(PortContext::Inject, error))?;
                self.surface
                    .dispatch_key_press(key)
                    .map_err(|error| port_error(PortContext::Inject, error))?;
                let mut root = Map::new();
                root.insert("dispatched".to_owned(), Value::from("key_press"));
                root.insert("keyCode".to_owned(), Value::from(key.as_str()));
                Ok(Value::Object(root))
            }
            methods::METHOD_SWITCH_MAIN_VIEW => {
                let view = required_text(params, "view");
                self.surface
                    .switch_main_view(&view)
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                let mut root = Map::new();
                root.insert("accepted".to_owned(), Value::from(true));
                root.insert("operation".to_owned(), Value::from("switch_main_view"));
                root.insert("view".to_owned(), Value::from(view));
                Ok(Value::Object(root))
            }
            methods::METHOD_FORCE_SAVE => {
                self.surface
                    .force_save()
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                Ok(accepted("force_save"))
            }
            methods::METHOD_RELOAD_ENGINE => {
                self.surface
                    .reload_engine()
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                Ok(accepted("reload_engine"))
            }
            // 注册表是 `const`，不可能走到这里；真走到了要**响亮报错**而不是返回空成功。
            other => Err(ErrorObject::new(
                jsonrpc::INTERNAL_ERROR,
                format!("方法 `{other}` 在注册表里但没有实现分支 (注册表与 execute 漂移)"),
            )),
        }
    }

    /// `ui/screenshot`：抓帧 → （可选）遮罩 → 证据 → PNG。
    fn screenshot(&mut self, params: &Map<String, Value>) -> Result<Value, ErrorObject> {
        // 默认 **遮罩**：§12.5 把动态区遮罩写成 MUST，而本方法的唯一用途就是视觉回归比对
        // (AI Agent 拿它做断言)。要原始帧请显式 `maskDynamic: false`。
        let mask_dynamic = bool_param(params, "maskDynamic").unwrap_or(true);
        let max_bytes =
            integer_param(params, "maxBytes").map_or(DEFAULT_MAX_PNG_BYTES, |value| value as usize);

        let projection = UiTree::from_runtime(self.surface.tree());
        let mut image = self
            .surface
            .capture_image()
            .map_err(|error| port_error(PortContext::Capture, error))?;

        let mut masked_regions = 0_usize;
        let mut mask_effective = false;
        if mask_dynamic {
            // 动态区缺包围盒 ⇒ **报错**。静默跳过就等于把假阳性放进 CI
            // ([UI-MCP-002] 的 MUST 要求"强制遮罩", 做不到就必须说做不到)。
            let rects = projection
                .mask_rects()
                .map_err(|error| geometry_unavailable(&error.to_string()))?
                .into_iter()
                .map(|(_, rect)| rect)
                .collect::<Vec<Rect>>();
            masked_regions = rects.len();
            image = mask::masked(&image, &rects);
            // "遮罩是否真的生效"必须在**置黑之后**判定（`mask_is_effective` 检查的正是
            // 矩形内像素是否全是纯黑）—— 顺序反了会得到一个恒为 false 的假自检。
            mask_effective = mask::mask_is_effective(&image, &rects);
        }

        let (bytes, evidence) = encode_with_evidence(&image, max_bytes)
            .map_err(|error| port_error(PortContext::Capture, error))?;
        Ok(shot_payload(
            mask_dynamic,
            masked_regions,
            mask_effective,
            &evidence,
            &bytes,
        ))
    }

    /// `ui/methods` 的载荷：方法注册表 + 本进程的运行态（不含任何秘密）。
    fn describe(&self) -> Value {
        let mut payload = match methods::catalogue() {
            Value::Object(map) => map,
            other => {
                // `catalogue()` 恒为对象; 这条分支只是不给 panic 留位置。
                let mut map = Map::new();
                map.insert("catalogue".to_owned(), other);
                map
            }
        };
        payload.insert("service".to_owned(), Value::from(SERVICE_NAME));
        payload.insert("mode".to_owned(), Value::from(self.mode.as_str()));
        payload.insert(
            "grantedScopes".to_owned(),
            Value::from(self.granted.to_spec_string()),
        );
        payload.insert("surface".to_owned(), Value::from(self.surface_name()));
        payload.insert(
            "injectAllowed".to_owned(),
            Value::from(
                !self.mode.is_production()
                    && self.granted.grants(yeban_mcp::security::Scope::UiInject),
            ),
        );
        Value::Object(payload)
    }
}

/// `PortError` 出现的上下文 —— 同一个 `Rejected` 在只读路径上是"参数问题"，
/// 在管理路径上是"这条能力尚未接线"，两者不该共用一个错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortContext {
    /// 只读（树 / 属性）。
    Read,
    /// 截图。
    Capture,
    /// 事件注入。
    Inject,
    /// 管理动作。
    Admin,
}

/// `PortError` → JSON-RPC 错误对象。
fn port_error(context: PortContext, error: PortError) -> ErrorObject {
    match error {
        PortError::PermissionDenied {
            operation,
            required,
            actual,
        } => {
            let mut data = Map::new();
            data.insert("kind".to_owned(), Value::from("port-permission-denied"));
            data.insert("operation".to_owned(), Value::from(operation.as_str()));
            data.insert("required".to_owned(), Value::from(required.as_str()));
            data.insert("actual".to_owned(), Value::from(actual.as_str()));
            ErrorObject::new(
                FORBIDDEN,
                format!(
                    "进程内权限不足: 操作 `{operation}` 需要 {required}, 当前 {actual} \
                     ([UI-MCP-001] §12.3 的三级模型)"
                ),
            )
            .with_data(Value::Object(data))
        }
        PortError::UnknownElement { id } => element_not_found(&id),
        PortError::MissingGeometry { id } => {
            geometry_unavailable(&format!("元素 `{id}` 没有几何包围盒"))
        }
        PortError::UnknownKey { key } => ErrorObject::invalid_params(format!(
            "无法解析的 `keyCode`: `{key}` (§12.4 的 key_code)"
        )),
        PortError::Capture { message } => {
            let mut data = Map::new();
            data.insert("kind".to_owned(), Value::from("capture-failed"));
            data.insert("detail".to_owned(), Value::from(message.clone()));
            ErrorObject::new(CAPTURE_FAILED, format!("Tier-1 截图失败: {message}"))
                .with_data(Value::Object(data))
        }
        PortError::Rejected { message } => match context {
            PortContext::Read => ErrorObject::invalid_params(message),
            _ => {
                let mut data = Map::new();
                data.insert("kind".to_owned(), Value::from("not-implemented"));
                data.insert("detail".to_owned(), Value::from(message.clone()));
                ErrorObject::new(NOT_IMPLEMENTED, message).with_data(Value::Object(data))
            }
        },
    }
}

/// `ELEMENT_NOT_FOUND`（§12.2：按语义 ID 找不到就是找不到，**不返回空成功**）。
fn element_not_found(id: &str) -> ErrorObject {
    let mut data = Map::new();
    data.insert("kind".to_owned(), Value::from("element-not-found"));
    data.insert("id".to_owned(), Value::from(id));
    ErrorObject::new(
        ELEMENT_NOT_FOUND,
        format!("控件树里没有语义 ID `{id}` ([UI-TEST-001] §12.2 只允许按语义 ID 寻址)"),
    )
    .with_data(Value::Object(data))
}

/// `GEOMETRY_UNAVAILABLE`。
fn geometry_unavailable(detail: &str) -> ErrorObject {
    let mut data = Map::new();
    data.insert("kind".to_owned(), Value::from("geometry-unavailable"));
    data.insert("detail".to_owned(), Value::from(detail));
    ErrorObject::new(
        GEOMETRY_UNAVAILABLE,
        format!("{detail} ([UI-MCP-002] 的遮罩/寻址需要真实几何)"),
    )
    .with_data(Value::Object(data))
}

/// 截图响应载荷（键序固定；`pngBase64` 放最后，免得日志里一大坨 base64 挡在前面）。
fn shot_payload(
    mask_dynamic: bool,
    masked_regions: usize,
    mask_effective: bool,
    evidence: &ShotEvidence,
    bytes: &[u8],
) -> Value {
    let mut root = Map::new();
    root.insert("maskDynamic".to_owned(), Value::from(mask_dynamic));
    root.insert("maskedRegions".to_owned(), Value::from(masked_regions));
    root.insert("maskEffective".to_owned(), Value::from(mask_effective));
    root.insert("width".to_owned(), Value::from(evidence.width));
    root.insert("height".to_owned(), Value::from(evidence.height));
    root.insert(
        "nonBlackPixels".to_owned(),
        Value::from(evidence.non_black_pixels),
    );
    root.insert(
        "distinctColors".to_owned(),
        Value::from(evidence.distinct_colors),
    );
    root.insert("pngBytes".to_owned(), Value::from(evidence.png_bytes));
    root.insert(
        "fingerprint".to_owned(),
        Value::from(format!("{:016x}", evidence.fingerprint)),
    );
    root.insert(
        "pngBase64".to_owned(),
        Value::from(crate::base64::encode(bytes)),
    );
    Value::Object(root)
}

/// 管理动作的通用"已接受"载荷。
fn accepted(operation: &str) -> Value {
    let mut root = Map::new();
    root.insert("accepted".to_owned(), Value::from(true));
    root.insert("operation".to_owned(), Value::from(operation));
    Value::Object(root)
}

/// `serde_json::to_value` 的**不 panic** 包装。
fn json_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// 可选字符串参数。
fn text_param<'a>(params: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    params.get(name).and_then(Value::as_str)
}

/// 必填字符串参数（校验已过；`unwrap_or_default` 是"不给 panic 留位置"的兜底）。
fn required_text(params: &Map<String, Value>, name: &str) -> String {
    text_param(params, name).unwrap_or_default().to_owned()
}

/// 可选布尔参数。
fn bool_param(params: &Map<String, Value>, name: &str) -> Option<bool> {
    params.get(name).and_then(Value::as_bool)
}

/// 可选整型参数。
fn integer_param(params: &Map<String, Value>, name: &str) -> Option<u64> {
    params.get(name).and_then(Value::as_u64)
}

/// 必填数值参数。
fn required_number(params: &Map<String, Value>, name: &str) -> f64 {
    params.get(name).and_then(Value::as_f64).unwrap_or_default()
}

/// `ui/coverage` 的参数：注册表 ID 清单（数组里必须全是字符串）。
fn registry_ids(params: &Map<String, Value>) -> Result<Vec<String>, ErrorObject> {
    let raw = params
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ErrorObject::invalid_params("`ids` 必须是数组"))?;
    let mut ids = Vec::with_capacity(raw.len());
    for (index, value) in raw.iter().enumerate() {
        let text = value.as_str().ok_or_else(|| {
            ErrorObject::invalid_params(format!(
                "`ids[{index}]` 必须是字符串, 收到 {}",
                methods::type_name(value)
            ))
        })?;
        ids.push(text.to_owned());
    }
    Ok(ids)
}

/// §12.4 的 `button` 参数。
fn button_param(params: &Map<String, Value>) -> Result<PointerButton, ErrorObject> {
    match required_text(params, "button").as_str() {
        "left" => Ok(PointerButton::Left),
        "middle" => Ok(PointerButton::Middle),
        "right" => Ok(PointerButton::Right),
        "other" => Ok(PointerButton::Other),
        // `validate_params` 的白名单已经拦过一遍; 这里是**第二道**,
        // 免得将来有人放宽白名单却忘了这里的 match。
        other => Err(ErrorObject::invalid_params(format!(
            "`button` 取值非法: `{other}`"
        ))),
    }
}

/// 把拒绝理由变成"状态码 + JSON-RPC 错误对象"（与 `mcp-core` 的 `denied` 同形状）。
fn denied(denial: &Denial) -> (u16, Result<Value, ErrorObject>) {
    let error = ErrorObject::new(denial.rpc_code(), denial.message()).with_data(denial.data());
    (denial.http_status(), Err(error))
}

/// 组装一个 `Outcome`（避免在几条返回路径上各写一遍）。
fn outcome(status: u16, id: Id, payload: Result<Value, ErrorObject>) -> Outcome {
    let response = match payload {
        Ok(result) => Response::success(id, result),
        Err(error) => Response::failure(id, error),
    };
    Outcome {
        http_status: status,
        response: Some(response),
    }
}

/// JSON-RPC 错误码 → HTTP 状态码。
///
/// 复用 `yeban_mcp::dispatch::http_status_for` 的既有映射，再补本线新增的三个码 ——
/// 两条 MCP 对 `-32700` / `-32601` / `-32001` / `-32003` / `-32005` 的状态码因此**逐字节相同**。
#[must_use]
pub fn http_status_for(error: &ErrorObject) -> u16 {
    match error.code {
        ELEMENT_NOT_FOUND | GEOMETRY_UNAVAILABLE => 400,
        CAPTURE_FAILED => 500,
        _ => yeban_mcp::dispatch::http_status_for(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;
    use yeban_mcp::security::Scope;
    use yeban_ui_test_port::image::{Rgb8Image, Size};
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::tree::ControlTree;

    /// 判据 ①: 未鉴权一律被拒；**原始入口**上鉴权先于解析（不含 token 时拿不到任何解析反馈）。
    ///
    /// 注入验证（把 `handle_line` 里的 `authenticate` 挪到 `Request::parse` 之后）会让
    /// 本判据的第 2 条变红（会变成 `-32700`）。
    #[test]
    fn unauthenticated_calls_are_rejected_and_the_raw_entry_parses_nothing() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);

        // 1) 原始入口: 完全没有 Authorization。
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"ui/tree"}"#;
        let outcome = service.handle_line(Channel::Http, None, line);
        assert_eq!(outcome.http_status, 401);
        let response = outcome.response.expect("有响应");
        assert_eq!(response.id, Id::Null, "未鉴权时无从回显 id");
        assert_eq!(
            response.error_object().expect("有 error").code,
            jsonrpc::UNAUTHORIZED
        );

        // 2) 原始入口: 错 token + **非法 JSON** ⇒ 401 而不是 -32700
        //    (请求体一个字节都没被解释)。
        let wrong = format!("Bearer {}", "0".repeat(64));
        let outcome = service.handle_line(Channel::Http, Some(&wrong), "{oops");
        assert_eq!(outcome.http_status, 401);
        assert_eq!(
            outcome
                .response
                .expect("有响应")
                .error_object()
                .expect("有 error")
                .code,
            jsonrpc::UNAUTHORIZED
        );

        // 3) 合法 token + 非法 JSON ⇒ 这才轮到解析错误。
        let header = authorization(&token);
        let outcome = service.handle_line(Channel::Http, Some(&header), "{oops");
        assert_eq!(outcome.http_status, 400);
        assert_eq!(
            outcome
                .response
                .expect("有响应")
                .error_object()
                .expect("有 error")
                .code,
            jsonrpc::PARSE_ERROR
        );

        // 4) 已解析入口: 错 token 也必须拒。
        let other = BearerToken::parse(&"1".repeat(64)).expect("夹具令牌");
        let error = error_of(
            &mut service,
            &other,
            &request(methods::METHOD_TREE, Value::Null),
        );
        assert_eq!(error.code, jsonrpc::UNAUTHORIZED);
        assert_eq!(service.denied(), 4, "四条被拒路径都要计数");
    }

    /// 判据 ①′: **原始入口不泄漏方法集** —— 未鉴权调一个不存在的方法，拿到 401 而不是 -32601。
    #[test]
    fn raw_entry_point_never_leaks_the_method_set() {
        let state = shared(Permission::ReadOnly);
        let (mut service, _) = build_read_service(&state);
        let outcome = service.handle_line(
            Channel::Http,
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"ui/definitely-not-a-method"}"#,
        );
        assert_eq!(outcome.http_status, 401, "未鉴权不该看到 -32601");
        assert_eq!(
            outcome
                .response
                .expect("有响应")
                .error_object()
                .expect("有 error")
                .code,
            jsonrpc::UNAUTHORIZED
        );

        // 已解析入口上未知方法才是 -32601（调用方已经在进程边界之内）。
        let (mut service, token) = build_read_service(&state);
        let error = error_of(&mut service, &token, &request("ui/nope", Value::Null));
        assert_eq!(error.code, jsonrpc::METHOD_NOT_FOUND);
        assert_eq!(http_status_for(&error), 404);
    }

    /// 判据 ②: scope 不足被拒，且拒绝里**点名**要哪一个、现在有什么；
    /// `app:admin` **不**隐含任何 `ui:*`。
    #[test]
    fn insufficient_scope_names_the_required_scope() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(
            &state,
            ScopeSet::from_scopes([Scope::UiRead]),
            RunMode::Production,
        );
        let error = error_of(
            &mut service,
            &token,
            &request(methods::METHOD_SCREENSHOT, serde_json::json!({})),
        );
        assert_eq!(error.code, FORBIDDEN);
        let data = error.data.expect("有 data");
        assert_eq!(data["kind"], "insufficient-scope");
        assert_eq!(data["requiredScope"], "ui:screenshot");
        assert_eq!(data["grantedScopes"], "ui:read");

        // `app:admin` 是"全量领域权限", 但界面控制是另一条授权线。
        let (mut admin, admin_token) = build_service(
            &state,
            ScopeSet::from_scopes([Scope::AppAdmin]),
            RunMode::Production,
        );
        for method in [
            methods::METHOD_TREE,
            methods::METHOD_SCREENSHOT,
            methods::METHOD_DISPATCH_KEY_PRESS,
        ] {
            let params = if method == methods::METHOD_DISPATCH_KEY_PRESS {
                serde_json::json!({"keyCode": "Tab"})
            } else {
                serde_json::json!({})
            };
            let error = error_of(&mut admin, &admin_token, &request(method, params));
            assert_eq!(
                error.code, FORBIDDEN,
                "`app:admin` 不得隐含 `{method}` 需要的 ui scope"
            );
        }

        // 反过来: 管理动作要求 app:* —— 只有 ui:* 的调用方拿不到。
        let (mut ui_only, ui_token) = build_service(
            &state,
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot, Scope::UiInject]),
            RunMode::Test,
        );
        let error = error_of(
            &mut ui_only,
            &ui_token,
            &request(methods::METHOD_FORCE_SAVE, serde_json::json!({})),
        );
        assert_eq!(error.data.expect("有 data")["requiredScope"], "app:save");
    }

    /// 判据 ③（**承重**）: 事件注入在生产模式被硬拒，且**先于 token 校验**。
    ///
    /// 三种凭据（无 / 错 / 合法）都必须拿到 `forbidden-in-production`（403）而不是 401。
    /// 注入验证 B（删掉生产硬禁 / 把顺序改成先鉴权）会让本判据变红。
    #[test]
    fn event_injection_is_hard_denied_in_production_before_token_check() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Production);
        let wrong = BearerToken::parse(&"2".repeat(64)).expect("夹具令牌");
        let injection = request(
            methods::METHOD_DISPATCH_KEY_PRESS,
            serde_json::json!({"keyCode": "Tab"}),
        );

        for (label, header) in [
            ("无凭据", None),
            ("错凭据", Some(authorization(&wrong))),
            ("合法凭据", Some(authorization(&token))),
        ] {
            let outcome = service.handle(Channel::Http, header.as_deref(), &injection);
            assert_eq!(outcome.http_status, 403, "{label} 必须被硬拒");
            let error = outcome
                .response
                .expect("有响应")
                .error_object()
                .expect("有 error")
                .clone();
            assert_eq!(error.code, FORBIDDEN, "{label}");
            let data = error.data.expect("有 data");
            assert_eq!(data["kind"], "forbidden-in-production", "{label}");
            assert_eq!(data["scope"], "ui:inject", "{label}");
            assert_eq!(data["mode"], "production", "{label}");
        }
        assert!(calls(&state).is_empty(), "硬拒时执行面一次都不该被调用");

        // 只读方法在生产模式下照样工作（硬禁只针对 ui:inject）。
        let _ = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, serde_json::json!({})),
        );

        // 测试模式 + 显式授予 ui:inject ⇒ 真的落到执行面。
        let (mut test_mode, test_token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let result = result_of(
            &mut test_mode,
            &test_token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Tab"}),
            ),
        );
        assert_eq!(result["dispatched"], "key_press");
        assert_eq!(result["keyCode"], "Tab");
        assert_eq!(calls(&state), ["key_press:Tab".to_owned()]);

        // 测试模式但没有 ui:inject 授予 ⇒ 仍然拒（硬禁 ≠ 自动授权）。
        let (mut test_no_inject, no_inject_token) = build_service(
            &state,
            ScopeSet::from_scopes([Scope::UiRead]),
            RunMode::Test,
        );
        let error = error_of(
            &mut test_no_inject,
            &no_inject_token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Tab"}),
            ),
        );
        assert_eq!(error.data.expect("有 data")["kind"], "insufficient-scope");
    }

    /// 判据 ④: `ui/tree` 的**结果字节**两次调用逐字节相同，节点按语义 ID 升序。
    ///
    /// 注入验证 C（把投影的节点顺序改成依赖插入顺序 / 逆序）会让本判据变红。
    #[test]
    fn tree_method_result_is_byte_stable_across_two_calls() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let first = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, serde_json::json!({})),
        );
        let second = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, serde_json::json!({})),
        );
        assert_eq!(first, second);
        assert_eq!(first.to_string(), second.to_string(), "逐字节相同");

        let nodes = first["tree"]["nodes"].as_array().expect("数组");
        let ids = nodes
            .iter()
            .map(|node| node["id"].as_str().expect("字符串"))
            .collect::<Vec<_>>();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "节点必须按语义 ID 升序");
        assert_eq!(first["tree"]["source"], "runtime");
        assert_eq!(first["tree"]["count"], 3);
        assert_eq!(first["filter"]["totalCount"], 3);

        // 逆序插入的同一棵树 ⇒ 结果字节必须相同。
        let mut reversed = ControlTree::new();
        for id in fixture_tree().ids().collect::<Vec<_>>().into_iter().rev() {
            let source = fixture_tree();
            reversed
                .insert(source.find_by_id(id).expect("存在").clone())
                .expect("插入");
        }
        let (mut service_b, token_b) = build_service_with_tree(
            &state,
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot]),
            RunMode::Production,
            reversed,
        );
        let reordered = result_of(
            &mut service_b,
            &token_b,
            &request(methods::METHOD_TREE, serde_json::json!({})),
        );
        assert_eq!(first.to_string(), reordered.to_string(), "不得依赖插入顺序");

        // 过滤: 前缀 + 只看动态区。
        let filtered = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_TREE,
                serde_json::json!({"prefix": "mixer-", "dynamicOnly": true}),
            ),
        );
        assert_eq!(filtered["tree"]["count"], 1);
        assert_eq!(filtered["tree"]["nodes"][0]["id"], "mixer-vu-track-0");
        assert_eq!(filtered["filter"]["prefix"], "mixer-");
        assert_eq!(filtered["filter"]["dynamicOnly"], true);

        // 空过滤结果是**合法成功**（它确实查到了 0 个节点），与 ⑤ 的"按 ID 查不到"要分开。
        let none = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, serde_json::json!({"prefix": "nope-"})),
        );
        assert_eq!(none["tree"]["count"], 0);
    }

    /// 判据 ⑤: 按不存在的语义 ID 查节点 → **明确错误**（不是空成功）；属性同理。
    #[test]
    fn unknown_semantic_id_is_an_explicit_error_not_an_empty_success() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_NODE,
                serde_json::json!({"elementId": "track-9-fader"}),
            ),
        );
        assert_eq!(error.code, ELEMENT_NOT_FOUND);
        assert_eq!(error.data.as_ref().expect("有 data")["id"], "track-9-fader");
        assert_eq!(http_status_for(&error), 400);

        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_PROPERTY,
                serde_json::json!({"elementId": "track-9-fader", "name": "value"}),
            ),
        );
        assert_eq!(error.code, ELEMENT_NOT_FOUND);

        // 不支持的属性名在只读路径上是**参数问题**（INVALID_PARAMS），不是"未实现"。
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_PROPERTY,
                serde_json::json!({"elementId": "track-0-fader", "name": "nope"}),
            ),
        );
        assert_eq!(error.code, INVALID_PARAMS);

        // 存在的 ID 正常返回; 无几何 ⇒ `visible` 是 null（不可断定）。
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_NODE,
                serde_json::json!({"elementId": "track-0-fader"}),
            ),
        );
        assert_eq!(result["node"]["role"], "slider");
        assert_eq!(result["node"]["visible"], Value::Null);
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_PROPERTY,
                serde_json::json!({"elementId": "track-0-fader", "name": "value"}),
            ),
        );
        assert_eq!(result["value"], "value=1");
    }

    /// 判据 ⑥: 截图是**真实像素** —— 尺寸非零、非全黑、PNG 魔数、证据与字节一致。
    ///
    /// 这里渲染的是本机零 Slint 的假执行面（真 `Rgb8Image` + 真 PNG 编码器）；
    /// Tier-1 的**真实**光栅化由 `yeban-ui-test-port` / `yeban-app` 的 CI 判据证明
    /// （见 `docs/ledger/ui-mcp-notes.md` 的"本机验证 vs 交给 CI"）。
    #[test]
    fn screenshot_returns_a_real_non_black_png_with_evidence() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SCREENSHOT,
                serde_json::json!({"maskDynamic": false}),
            ),
        );
        assert_eq!(result["width"], 200);
        assert_eq!(result["height"], 120);
        assert!(result["nonBlackPixels"].as_u64().expect("数字") > 0);
        assert!(result["distinctColors"].as_u64().expect("数字") >= 2);
        assert_eq!(result["maskedRegions"], 0);
        assert_eq!(result["maskDynamic"], false);

        let encoded = result["pngBase64"].as_str().expect("字符串");
        let bytes = crate::base64::decode(encoded).expect("base64 必须能解回来");
        assert_eq!(
            bytes.len(),
            result["pngBytes"].as_u64().expect("数字") as usize
        );
        assert_eq!(&bytes[..8], &yeban_ui_test_port::png::PNG_SIGNATURE);
        assert_eq!(
            result["fingerprint"].as_str().expect("字符串"),
            format!("{:016x}", crate::surface::fnv1a64(&bytes))
        );

        // 全黑帧 → CAPTURE_FAILED（[MUST-GATE-015]）。
        state.borrow_mut().image = Rgb8Image::new(Size::new(200, 120));
        let error = error_of(
            &mut service,
            &token,
            &request(methods::METHOD_SCREENSHOT, serde_json::json!({})),
        );
        assert_eq!(error.code, CAPTURE_FAILED);
        assert_eq!(error.data.expect("有 data")["kind"], "capture-failed");

        // 超上限也是显式错误（不是发出去一个会被仓库红线拒的文件）。
        state.borrow_mut().image = fixture_image();
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SCREENSHOT,
                serde_json::json!({"maskDynamic": false, "maxBytes": 16}),
            ),
        );
        assert_eq!(error.code, CAPTURE_FAILED);
    }

    /// 判据 ⑦: 动态区来自**运行时包围盒**；默认遮罩真的把那些矩形置黑。
    ///
    /// 注入验证 D（把矩形写成硬编码坐标）会让本判据的"挪走包围盒"一段变红。
    #[test]
    fn dynamic_regions_are_masked_from_the_runtime_bounds() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let regions = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_DYNAMIC_REGIONS, serde_json::json!({})),
        );
        assert_eq!(regions["count"], 2);
        assert_eq!(regions["regions"][0]["id"], "mixer-vu-track-0");
        assert_eq!(regions["regions"][0]["bounds"]["x"], 40);
        assert_eq!(regions["regions"][0]["bounds"]["y"], 60);
        assert_eq!(regions["regions"][0]["bounds"]["width"], 8);
        assert_eq!(regions["regions"][0]["bounds"]["height"], 64);
        assert_eq!(regions["regions"][1]["id"], "transport-timecode");

        // 默认遮罩: 遮罩区数 == 动态区数, 且置黑真的生效。
        let masked = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_SCREENSHOT, serde_json::json!({})),
        );
        assert_eq!(masked["maskDynamic"], true);
        assert_eq!(masked["maskedRegions"], 2);
        assert_eq!(masked["maskEffective"], true);

        // 未遮罩的帧必须与遮罩帧**不同**（否则"遮罩"就是一句空话）。
        let raw = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SCREENSHOT,
                serde_json::json!({"maskDynamic": false}),
            ),
        );
        assert_ne!(raw["fingerprint"], masked["fingerprint"]);

        // 包围盒挪走 ⇒ 遮罩矩形跟着挪（证明它不是硬编码坐标）。
        let (mut moved, moved_token) = build_service_with_tree(
            &state,
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot]),
            RunMode::Production,
            tree_with_bounds("mixer-vu-track-0", Some(Rect::new(7, 9, 8, 64))),
        );
        let regions = result_of(
            &mut moved,
            &moved_token,
            &request(methods::METHOD_DYNAMIC_REGIONS, serde_json::json!({})),
        );
        assert_eq!(regions["regions"][0]["bounds"]["x"], 7);
        assert_eq!(regions["regions"][0]["bounds"]["y"], 9);

        // 动态区没有几何 ⇒ 必须**报错**（做不到 MUST 就说做不到）。
        let (mut broken, broken_token) = build_service_with_tree(
            &state,
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot]),
            RunMode::Production,
            tree_with_bounds("mixer-vu-track-0", None),
        );
        let error = error_of(
            &mut broken,
            &broken_token,
            &request(methods::METHOD_SCREENSHOT, serde_json::json!({})),
        );
        assert_eq!(error.code, GEOMETRY_UNAVAILABLE);
        assert_eq!(error.data.expect("有 data")["kind"], "geometry-unavailable");
        // ...但显式要求原始帧时不遮罩, 因此不该报错（调用方的显式选择）。
        let _ = result_of(
            &mut broken,
            &broken_token,
            &request(
                methods::METHOD_SCREENSHOT,
                serde_json::json!({"maskDynamic": false}),
            ),
        );
        // `ui/dynamic_regions` 也必须报错而不是少报一个区。
        let error = error_of(
            &mut broken,
            &broken_token,
            &request(methods::METHOD_DYNAMIC_REGIONS, serde_json::json!({})),
        );
        assert_eq!(error.code, GEOMETRY_UNAVAILABLE);
    }

    /// 判据 ⑧: 参数校验的错误码与 `mcp-core` 同口径（`-32602`），未知参数被拒。
    #[test]
    fn malformed_params_are_invalid_params() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        for params in [
            serde_json::json!({}),
            serde_json::json!({"id": "track-0-fader"}),
            serde_json::json!({"elementId": 5}),
        ] {
            let error = error_of(&mut service, &token, &request(methods::METHOD_NODE, params));
            assert_eq!(error.code, INVALID_PARAMS);
            assert_eq!(http_status_for(&error), 400);
        }
        let error = error_of(
            &mut service,
            &token,
            &request(methods::METHOD_METHODS, serde_json::json!({"nope": 1})),
        );
        assert_eq!(error.code, INVALID_PARAMS);
    }

    /// 判据 ⑨: **纵深防御** —— 进程内三级权限是第二道闸门。
    #[test]
    fn in_process_permission_is_a_second_independent_gate() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_POINTER_DOWN,
                serde_json::json!({
                    "elementId": "track-0-fader", "xOffset": 1.0, "yOffset": 2.0, "button": "left"
                }),
            ),
        );
        assert_eq!(error.code, FORBIDDEN);
        let data = error.data.expect("有 data");
        assert_eq!(data["kind"], "port-permission-denied");
        assert_eq!(data["operation"], "dispatch_pointer");
        assert_eq!(data["required"], "Interactive");
        assert_eq!(data["actual"], "ReadOnly");
        assert!(calls(&state).is_empty(), "越权时实现侧不得被调用");

        // scope 够、端口权限也够, 但**端点没有接线** ⇒ 端口返回 Rejected ⇒ -32005 如实报未实现。
        let state2 = shared(Permission::Administrative);
        state2.borrow_mut().reject_admin = true;
        let (mut interactive, interactive_token) =
            build_service(&state2, ScopeSet::all(), RunMode::Test);
        let error = error_of(
            &mut interactive,
            &interactive_token,
            &request(methods::METHOD_FORCE_SAVE, serde_json::json!({})),
        );
        assert_eq!(error.code, NOT_IMPLEMENTED);
        assert_eq!(
            error.data.as_ref().expect("有 data")["kind"],
            "not-implemented"
        );
        assert_eq!(http_status_for(&error), 501);
    }

    /// 判据 ⑩: `ui/methods` 是能力发现 —— 生产模式下 `injectAllowed=false`，
    /// 列出全部 14 条方法与各自的 scope。
    #[test]
    fn methods_discovery_reports_scopes_and_inject_availability() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let result = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_METHODS, serde_json::json!({})),
        );
        assert_eq!(result["service"], SERVICE_NAME);
        assert_eq!(result["mode"], "production");
        assert_eq!(result["grantedScopes"], "ui:read,ui:screenshot");
        assert_eq!(result["injectAllowed"], false);
        assert_eq!(result["surface"], "fake-surface");
        let listed = result["methods"].as_array().expect("数组");
        assert_eq!(listed.len(), methods::METHOD_COUNT);
        assert_eq!(listed[0]["name"], methods::METHOD_METHODS);

        let (mut test_mode, test_token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let result = result_of(
            &mut test_mode,
            &test_token,
            &request(methods::METHOD_METHODS, serde_json::json!({})),
        );
        assert_eq!(result["injectAllowed"], true);
        assert_eq!(result["mode"], "test");
    }

    /// 判据 ⑪: `ui/coverage` 双向核对；数组里出现非字符串必须报参数错。
    #[test]
    fn coverage_reports_both_directions() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_COVERAGE,
                serde_json::json!({"ids": ["track-0-fader", "tab-mixer-button"]}),
            ),
        );
        assert_eq!(
            result["missingAtRuntime"],
            serde_json::json!(["tab-mixer-button"])
        );
        assert_eq!(
            result["unknownAtRuntime"],
            serde_json::json!(["mixer-vu-track-0", "transport-timecode"])
        );
        assert_eq!(result["runtimeCount"], 3);
        assert_eq!(result["registryCount"], 2);

        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_COVERAGE,
                serde_json::json!({"ids": ["track-0-fader", 7]}),
            ),
        );
        assert_eq!(error.code, INVALID_PARAMS);
        // 细节在 `data.detail`（`message` 是稳定的种类说明"参数非法"）。
        assert!(
            error.data.as_ref().expect("有 data")["detail"]
                .as_str()
                .expect("字符串")
                .contains("ids[1]"),
            "{}",
            error.data.as_ref().expect("有 data")
        );
    }

    /// 判据 ⑫: notification 不产生响应（副作用照常发生）；管理动作的端口拒绝走 `-32005`。
    #[test]
    fn notifications_are_silent_and_administration_is_honestly_reported() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let header = authorization(&token);
        let notification = Request::parse(
            &serde_json::json!({"jsonrpc": "2.0", "method": methods::METHOD_FORCE_SAVE})
                .to_string(),
        )
        .expect("构造");
        let outcome = service.handle(Channel::Http, Some(&header), &notification);
        assert_eq!(outcome.http_status, 200);
        assert!(outcome.response.is_none(), "notification 不得有响应");
        assert_eq!(calls(&state), ["force_save".to_owned()]);

        // §12.4 的"元素内偏移"链路: 服务只负责把 elementId + 偏移交给执行面,
        // 换算成窗口坐标是执行面的事（`LivePort::dispatch_pointer_down_impl`）。
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_POINTER_DOWN,
                serde_json::json!({
                    "elementId": "track-0-fader", "xOffset": 3.5, "yOffset": 4.5, "button": "middle"
                }),
            ),
        );
        assert_eq!(result["button"], "middle");
        assert_eq!(result["xOffset"], 3.5);
        assert!(
            calls(&state)
                .iter()
                .any(|call| call == "pointer_down:track-0-fader:3.5:4.5:middle")
        );

        // selector 与 key 的解析: 规范点名的热键必须能过线, 拼错的必须拒。
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Shift+Enter"}),
            ),
        );
        assert_eq!(result["keyCode"], "Shift+Enter");
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Tab+"}),
            ),
        );
        assert_eq!(error.code, INVALID_PARAMS);
    }
}
