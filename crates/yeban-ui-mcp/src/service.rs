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
//!        5. dryRun 短路   只读计划 → dry_run::payload（ADR-0001 D48；**不改一个状态位**）
//!        6. 执行          经 [`UiSurface`] 落到 `yeban-ui-test-port`
//! ```
//!
//! ### `dryRun` 为什么排在**参数校验之后、执行之前**
//!
//! 与领域侧逐字相同的位置（`crates/yeban-mcp/src/dispatch.rs:325-334`：鉴权 → scope →
//! **解析/校验** → dryRun 短路 → 幂等 → 执行）。三条推论，都有判据：
//!
//! - `dryRun` **不绕过授权**：scope 不够时它同样拿 `403`（"先问后做"不是"先问免权限"）；
//! - `dryRun` **共享参数校验**：坏参数拿 `-32602`，不会"因为只是模拟就宽松"；
//! - `dryRun` **不消耗一次性资源**：本服务没有幂等缓存/一次性令牌，唯一的计数是
//!   `handled`（请求数，不是令牌），judged by `dry_run_consumes_no_scope_and_no_token`。
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
use yeban_mcp::jsonrpc::{self, ErrorObject, FORBIDDEN, Id, NOT_IMPLEMENTED, Request, Response};
use yeban_mcp::security::{
    AuthContext, BearerToken, Channel, Denial, RunMode, ScopeSet, authenticate, authorize,
};
use yeban_ui_test_port::port::{KeyCode, PointerButton, PortError};
use yeban_ui_test_port::{image::Rect, mask};

use crate::dry_run;
use crate::ime;
use crate::methods::{self, MethodSpec};
use crate::surface::{
    AdminReport, DEFAULT_MAX_PNG_BYTES, PreviewArguments, ShotEvidence, UiSurface,
    encode_with_evidence,
};
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

    /// 读方法的新鲜度契约：**进入任何一个读运行时树的分支之前**，先让执行面把树缓存
    /// 更新到"当下"（[`UiSurface::refresh_runtime_tree`]）。
    ///
    /// 三条口径：
    /// 1. 失败的映射走**既有的只读上下文**（`PortContext::Read` ⇒ `-32602`，`D25`：
    ///    不新增 JSON-RPC 错误码）；
    /// 2. **不回退到旧缓存** —— 把一棵已知过期的树当成"当下的界面"发出去是最坏的一种成功；
    /// 3. 没有活窗口的执行面（默认实现）什么都不做，因此"零 Slint 假面"的行为逐字不变。
    fn refresh_before_read(&mut self) -> Result<(), ErrorObject> {
        self.surface
            .refresh_runtime_tree()
            .map_err(|error| port_error(PortContext::Read, error))
    }

    /// 方法实现（每一条都必须落到 [`UiSurface`]，**不允许**在这里编造数据）。
    ///
    /// ## `dryRun`（ADR-0001 **D48**）在这条函数里的位置
    ///
    /// `dryRun=true` 时，**会改状态**的 7 条方法在**调用执行面之前**返回
    /// [`dry_run::payload`]。计划（"将要落到哪个操作、带什么实参"）与真调用
    /// **共用同一段实参提取代码**，因此两者不可能描述出不同的东西
    /// —— 这是"同一个词同一个意思"在实现层的落点，不是靠两处各写一遍。
    ///
    /// 只读的 7 条方法**不接受** `dryRun`（第 4 步的参数校验就会拒绝，`-32602`）：
    /// 只读方法没有副作用可短路，理由见 [`crate::dry_run`] 的模块头。
    fn execute(
        &mut self,
        spec: &'static MethodSpec,
        params: &Map<String, Value>,
    ) -> Result<Value, ErrorObject> {
        // `dryRun` 只在**校验过**的参数上求值（`validate_params` 已把类型钉成 boolean）。
        let dry_run = dry_run::requested(params);
        match spec.name {
            methods::METHOD_METHODS => Ok(self.describe()),
            methods::METHOD_TREE => {
                self.refresh_before_read()?;
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
                self.refresh_before_read()?;
                let projection = UiTree::from_runtime(self.surface.tree());
                let node = projection.find(&id).ok_or_else(|| element_not_found(&id))?;
                let mut root = Map::new();
                root.insert("node".to_owned(), json_of(node));
                Ok(Value::Object(root))
            }
            methods::METHOD_PROPERTY => {
                let id = required_text(params, "elementId");
                let name = required_text(params, "name");
                // 读之前先刷新：`ui/property` 的**值**本来就来自活组件，但它的
                // "这个 ID 在不在树里"那一问走的是同一棵缓存树
                // （`LivePort::read_property` 的第一行）⇒ 不刷新就会把
                // "刚刚出现的元素"报成 `-32006`。
                self.refresh_before_read()?;
                // `[UI-A11Y-002]` 的 IME 合成态是**虚拟属性**：它的载体是执行面的
                // IME 状态机，不是 Slint 的响应式属性，因此不走 `read_property`。
                if ime::is_ime_property(&name) {
                    return self.ime_property(&id, &name);
                }
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
                self.refresh_before_read()?;
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
                self.refresh_before_read()?;
                let projection = UiTree::from_runtime(self.surface.tree());
                Ok(json_of(&Coverage::between(&projection, &ids)))
            }
            methods::METHOD_DISPATCH_POINTER_DOWN => {
                let id = required_text(params, "elementId");
                let x = required_number(params, "xOffset");
                let y = required_number(params, "yOffset");
                let button = button_param(params)?;
                if dry_run {
                    // 只读前置校验（"元素在树里 且 有几何"）：与
                    // `LivePort::dispatch_pointer_down_impl` 的两条前置**同码同句**
                    // （`crates/yeban-ui-test-port/src/render.rs:513-521`），
                    // 因此 `dryRun` 不会把一次注定失败的真调用描述成成功。
                    // 它**只**在 dryRun 路径上跑：真路径仍然把这条判定留给执行面
                    // （纵深防御；既有判据 `in_process_permission_is_a_second_independent_gate`
                    // 钉住"越权时先报端口权限"，不能被这里抢在前面）。
                    self.ensure_geometry(&id)?;
                    return self.preview(spec, params);
                }
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
                if dry_run {
                    return self.preview(spec, params);
                }
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
                if dry_run {
                    return self.preview(spec, params);
                }
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
                // 键名解析（`§12.4` 的 `key_code`）与真调用共用 ⇒ dryRun 对拼错的键名
                // 给出**同一个** `-32602`，不会假装"这一键会被分发"。
                let key =
                    KeyCode::parse(&raw).map_err(|error| port_error(PortContext::Inject, error))?;
                if dry_run {
                    return self.preview(spec, params);
                }
                self.surface
                    .dispatch_key_press(key)
                    .map_err(|error| port_error(PortContext::Inject, error))?;
                let mut root = Map::new();
                root.insert("dispatched".to_owned(), Value::from("key_press"));
                root.insert("keyCode".to_owned(), Value::from(key.as_str()));
                Ok(Value::Object(root))
            }
            methods::METHOD_SET_TRACK_HEIGHT => {
                let id = required_text(params, "elementId");
                // `integer` 类型由第 4 步的校验钉住；这里把**取值范围**也钉住
                // （`as_u64` 对负数 / 超 `u64` 的整数返回 `None`）。
                //
                // 为什么范围检查在执行分支而不是 `validate_params`：`ParamSpec` 没有"数值下界"
                // 这个面（加它要改 `ui/methods` 的载荷形状，只为一个方法），而"非法取值"
                // 仍然用**既有**的 `-32602 INVALID_PARAMS`（D25：不发明新错误码）。
                let height_px = integer_param(params, "heightPx")
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        ErrorObject::invalid_params(format!(
                            "`ui/set_track_height` 的 `heightPx` 必须是 0..=4294967295 的整数 \
                             （`0` = 取消这条覆盖），收到 {}",
                            params.get("heightPx").cloned().unwrap_or(Value::Null)
                        ))
                    })?;
                if dry_run {
                    return self.preview(spec, params);
                }
                self.surface
                    .set_track_height(&id, height_px)
                    .map_err(|error| port_error(port_context(spec), error))?;
                let mut root = Map::new();
                root.insert("accepted".to_owned(), Value::from(true));
                root.insert("operation".to_owned(), Value::from("set_track_height"));
                root.insert("elementId".to_owned(), Value::from(id));
                root.insert("heightPx".to_owned(), Value::from(height_px));
                attach_admin_report(&mut root, self.surface.take_admin_report());
                Ok(Value::Object(root))
            }
            methods::METHOD_SWITCH_MAIN_VIEW => {
                let view = required_text(params, "view");
                if dry_run {
                    return self.preview(spec, params);
                }
                self.surface
                    .switch_main_view(&view)
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                let report = self.surface.take_admin_report();
                let mut root = Map::new();
                root.insert("accepted".to_owned(), Value::from(true));
                root.insert("operation".to_owned(), Value::from("switch_main_view"));
                root.insert("view".to_owned(), Value::from(view));
                attach_admin_report(&mut root, report);
                Ok(Value::Object(root))
            }
            methods::METHOD_FORCE_SAVE => {
                if dry_run {
                    return self.preview(spec, params);
                }
                self.surface
                    .force_save()
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                Ok(accepted_with_report(
                    "force_save",
                    self.surface.take_admin_report(),
                ))
            }
            methods::METHOD_RELOAD_ENGINE => {
                if dry_run {
                    return self.preview(spec, params);
                }
                self.surface
                    .reload_engine()
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                Ok(accepted_with_report(
                    "reload_engine",
                    self.surface.take_admin_report(),
                ))
            }
            methods::METHOD_OPEN_PROJECT => {
                let path = required_text(params, "path");
                // 默认值与 `yeban_close_project` 逐字相同（`saveFirst` 缺省 = true）。
                // 缺省值在**这里**算一次，并与 `dryRun` 的预览共用（下面的 `preview` 走
                // `preview_effect`，而 `preview_effect` 也从实参里取同一个缺省）——
                // 因此"预览说的"与"真做的"不可能对同一个请求给出不同的 `saveFirst`。
                let save_first = bool_param(params, "saveFirst").unwrap_or(true);
                if dry_run {
                    return self.preview(spec, params);
                }
                self.surface
                    .open_project(&path, save_first)
                    .map_err(|error| port_error(PortContext::Admin, error))?;
                let mut root = Map::new();
                root.insert("accepted".to_owned(), Value::from(true));
                root.insert("operation".to_owned(), Value::from("open_project"));
                root.insert("path".to_owned(), Value::from(path));
                root.insert("saveFirst".to_owned(), Value::from(save_first));
                attach_admin_report(&mut root, self.surface.take_admin_report());
                Ok(Value::Object(root))
            }
            // 注册表是 `const`，不可能走到这里；真走到了要**响亮报错**而不是返回空成功。
            other => Err(ErrorObject::new(
                jsonrpc::INTERNAL_ERROR,
                format!("方法 `{other}` 在注册表里但没有实现分支 (注册表与 execute 漂移)"),
            )),
        }
    }

    /// **`dryRun=true` 的响应**（ADR-0001 **D48**）：只回报"将要发生什么"。
    ///
    /// 两件事**必须**同时成立，否则就不是 dryRun：
    ///
    /// 1. **一个状态位都不许变**：本函数拿到的是 `&self`（执行面只能被**只读**地问），
    ///    而且它在**每个** mutating 分支里都排在 `*_impl` 调用**之前**返回 ——
    ///    "执行面一次都没被调用"由判据逐方法钉住（`dry_run_leaves_the_state_untouched_…`）；
    /// 2. **预览要真**：计划（操作 + 归一化实参）来自与真调用**同一段**实参提取代码；
    ///    执行面能报的"影响"由 [`UiSurface::preview_effect`] 只读算出；它报 `Err`
    ///    （"这次真调用一定会失败"）时，这里把**同一个**错误码原样交出去
    ///    （与领域侧"dryRun 只做参数与领域合法性校验、失败如实报"同口径）。
    fn preview(
        &self,
        spec: &'static MethodSpec,
        params: &Map<String, Value>,
    ) -> Result<Value, ErrorObject> {
        let arguments = dry_run::normalized_arguments(params);
        // 实参以 `PreviewArguments`（零 serde_json 的只读视图）交给执行面 —— 接线方
        // 不依赖 serde_json，接口就不该要求它出现。
        let view = PreviewArguments::from_arguments(&arguments);
        let effect = self
            .surface
            .preview_effect(spec.name, &view)
            .map_err(|error| port_error(port_context(spec), error))?
            .map(|effect| effect.to_json());
        // 操作名**从注册表派生**（`port_operation`），不在这里手写第二个名字。
        let operation = spec
            .port_operation
            .map_or("<none>", |operation| operation.as_str());
        Ok(dry_run::payload(spec, operation, arguments, effect))
    }

    /// `ui/property` 的**虚拟属性** `isComposing`（`[UI-A11Y-002]` 的 IME 合成态）。
    ///
    /// 三条口径：
    /// 1. `elementId` 仍然必须**真的在运行树里**（§12.2：只能按语义 ID 寻址，
    ///    找不到就是找不到）—— 拼错 ID 与"没有 IME 状态"是两件事，不能混成一个错误；
    /// 2. `value` 是**原生布尔**（载体是进程内的 IME 状态机，不是 Slint 属性，
    ///    见 [`crate::ime`] 的文件头）；
    /// 3. 执行面没有 IME 状态机时**如实报** `-32005`（`data.kind = not-implemented`），
    ///    **不编一个 `false`** —— 一个恒假的合成态位会让"IME 防护没被触发"变成假绿。
    fn ime_property(&self, id: &str, name: &str) -> Result<Value, ErrorObject> {
        if !self.surface.tree().contains(id) {
            return Err(element_not_found(id));
        }
        let Some(state) = self.surface.ime_state() else {
            let mut data = Map::new();
            data.insert("kind".to_owned(), Value::from("not-implemented"));
            data.insert(
                "detail".to_owned(),
                Value::from(format!(
                    "执行面 `{}` 没有 `[UI-A11Y-002]` 的 IME 状态机 \
                     (`UiSurface::ime_state` 返回 None)",
                    self.surface_name()
                )),
            );
            return Err(ErrorObject::new(
                NOT_IMPLEMENTED,
                format!("`{name}` 在这个执行面上没有载体（不是所有的执行面都有 IME 状态）"),
            )
            .with_data(Value::Object(data)));
        };
        let mut root = Map::new();
        root.insert("id".to_owned(), Value::from(id));
        root.insert("name".to_owned(), Value::from(name));
        root.insert("value".to_owned(), Value::from(state.composing));
        root.insert("focus".to_owned(), Value::from(state.focus_name()));
        root.insert("specId".to_owned(), Value::from(ime::SPEC_ID));
        Ok(Value::Object(root))
    }

    /// 只读前置校验：元素必须在运行树里、且必须有几何包围盒。
    ///
    /// 与 `LivePort::dispatch_pointer_down_impl` 的两条前置
    /// （`crates/yeban-ui-test-port/src/render.rs:513-521`）是**同一个错误码、
    /// 同一句消息**。它只在 `dryRun` 路径上跑，理由有两条：
    ///
    /// 1. `dryRun` 的规范语义是"只做参数与领域合法性校验"（领域侧口径），因此
    ///    "这次真调用一定会失败"必须在预览阶段就如实报出来；
    /// 2. 真调用路径**不**抢在执行面前面判它 —— 端口权限闸门必须仍然是第一道
    ///    能说话的闸门（既有判据 `in_process_permission_is_a_second_independent_gate`）。
    fn ensure_geometry(&self, id: &str) -> Result<(), ErrorObject> {
        let node = self
            .surface
            .tree()
            .find_by_id(id)
            .ok_or_else(|| element_not_found(id))?;
        if node.bounds.is_none() {
            return Err(geometry_unavailable(&format!("元素 `{id}` 没有几何包围盒")));
        }
        Ok(())
    }

    /// `ui/screenshot`：抓帧 → （可选）遮罩 → 证据 → PNG。
    ///
    /// ⚠ **进函数先刷新运行时树**：§12.5 的遮罩矩形来自树（`dynamic_region` + 几何），
    /// 不刷新就会出现"像素是新的、遮罩按旧几何置黑"这种更难查的不一致 ——
    /// 而 `ui/screenshot` 存在的唯一用途就是视觉回归比对。
    fn screenshot(&mut self, params: &Map<String, Value>) -> Result<Value, ErrorObject> {
        // 默认 **遮罩**：§12.5 把动态区遮罩写成 MUST，而本方法的唯一用途就是视觉回归比对
        // (AI Agent 拿它做断言)。要原始帧请显式 `maskDynamic: false`。
        let mask_dynamic = bool_param(params, "maskDynamic").unwrap_or(true);
        let max_bytes =
            integer_param(params, "maxBytes").map_or(DEFAULT_MAX_PNG_BYTES, |value| value as usize);

        self.refresh_before_read()?;
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

/// 从注册表的 scope 推出 [`PortContext`]（`dryRun` 预览里执行面报错时用它选错误码）。
///
/// 为什么按 **scope** 而不是按方法名：方法与 scope 的对应关系是注册表里的**一个字段**
/// （判据 `port_operation_tier_matches_scope` 钉住两张权限表不许漂移），
/// 在这里再抄一张"方法 → 上下文"的表就会多出第二个事实源。
fn port_context(spec: &MethodSpec) -> PortContext {
    match spec.scope {
        yeban_mcp::security::Scope::UiRead => PortContext::Read,
        yeban_mcp::security::Scope::UiScreenshot => PortContext::Capture,
        yeban_mcp::security::Scope::UiInject => PortContext::Inject,
        _ => PortContext::Admin,
    }
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

/// [`accepted`] + 执行面交出来的结构化回执（没有回执时与 [`accepted`] 逐字节相同）。
fn accepted_with_report(operation: &str, report: Option<AdminReport>) -> Value {
    let mut root = match accepted(operation) {
        Value::Object(map) => map,
        // `accepted` 恒为对象；这条分支只是不给 panic 留位置。
        other => return other,
    };
    attach_admin_report(&mut root, report);
    Value::Object(root)
}

/// 把执行面的回执挂到结果载荷上（**唯一**的挂载点）。
///
/// 契约（有判据钉住）：
/// - 没有回执 ⇒ 载荷**一个字节都不变**（既有执行面的行为完全不变）；
/// - 有回执 ⇒ `result.report` 是 [`AdminReport::to_json`] 的原样输出；
/// - 回执里的 `operation` 必须与结果根的 `operation` **同名**（否则说明执行面报的是
///   另一个动作 —— 那种漂移比"没有回执"更危险，见判据
///   `admin_reports_name_the_operation_that_was_actually_run`）。
fn attach_admin_report(root: &mut Map<String, Value>, report: Option<AdminReport>) {
    let Some(report) = report else {
        return;
    };
    debug_assert_eq!(
        root.get("operation").and_then(Value::as_str),
        Some(report.operation),
        "回执的 operation 必须与结果根一致（执行面报错了动作）"
    );
    root.insert("report".to_owned(), report.to_json());
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
    use crate::surface::ReportValue;
    use crate::testing::*;
    use yeban_mcp::jsonrpc::INVALID_PARAMS;
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

    /// 判据 ④′: **读方法在返回前把运行时树刷到"当下"** —— 六条读路径**逐条**钉住。
    ///
    /// ## 判别力（为什么不是"对着 mock 断言 mock"）
    ///
    /// 夹具预置的"下一棵树"里多一个**带序号**的探针节点（`refresh-probe-{n}`，
    /// 动态区 + 有几何）。每一条读方法各自预置**新的一版**，然后要求两件事同时成立：
    ///
    /// 1. 这一版的读数里**有** `refresh-probe-{n}`；
    /// 2. 这一版的读数里**没有** `refresh-probe-{n-1}`。
    ///
    /// ⇒ 把 `refresh_before_read()` 从**任何一条**分支里摘掉，那一条读到的是上一版
    /// （或原始夹具树）⇒ 参数 1 或缺、参数 2 破 ⇒ **那一条**变红。
    ///
    /// ⚠ **序号不是装饰**：第一版探针用固定 ID，实测"只摘掉 `ui/node` 的刷新"**仍然绿** ——
    /// 因为 `ui/tree` 先跑、它刷新过了，后面每一条读的树里都已经有那个固定 ID。
    /// 那种写法只证明"树被刷新过至少一次"，证明不了"每一条都刷新"。序号是那条教训的产物。
    #[test]
    fn read_methods_refresh_the_runtime_tree_before_answering() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);

        // ---- 起点：树里一个探针都没有（否则下面每一条断言都恒真） ----
        let before = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, Value::Null),
        );
        assert_eq!(before["tree"]["count"], 3, "夹具树就是 3 个节点");
        assert!(
            !before.to_string().contains(REFRESH_PROBE_PREFIX),
            "预置之前线上 JSON 里不可能有探针"
        );

        // `ui/tree`：整棵树的节点集合。
        {
            let probe = set_refreshed_tree_with_probe(&state);
            let stale = previous_probe_id(&state);
            let tree = result_of(
                &mut service,
                &token,
                &request(methods::METHOD_TREE, Value::Null),
            );
            assert_eq!(tree["tree"]["count"], 4, "3 个夹具节点 + 1 个探针");
            assert!(tree.to_string().contains(&probe), "{tree}");
            assert!(
                !tree.to_string().contains(&stale),
                "`ui/tree` 不得还在读上一版 `{stale}`: {tree}"
            );
        }

        // `ui/node`：少了刷新就是 `-32006`（`result_of` 会因为 400 直接失败）。
        {
            let probe = set_refreshed_tree_with_probe(&state);
            let stale = previous_probe_id(&state);
            let node = result_of(
                &mut service,
                &token,
                &request(
                    methods::METHOD_NODE,
                    serde_json::json!({"elementId": probe}),
                ),
            );
            assert_eq!(node["node"]["id"], probe);
            assert!(!node.to_string().contains(&stale), "{node}");
        }

        // `ui/property`：存在性一问走同一棵缓存树。
        {
            let probe = set_refreshed_tree_with_probe(&state);
            let stale = previous_probe_id(&state);
            let property = result_of(
                &mut service,
                &token,
                &request(
                    methods::METHOD_PROPERTY,
                    serde_json::json!({"elementId": probe, "name": "width"}),
                ),
            );
            assert_eq!(property["value"], "width=1");
            assert!(!property.to_string().contains(&stale), "{property}");
        }

        // `ui/dynamic_regions`：探针是动态区 ⇒ 必须恰好多一格。
        {
            let probe = set_refreshed_tree_with_probe(&state);
            let stale = previous_probe_id(&state);
            let regions = result_of(
                &mut service,
                &token,
                &request(methods::METHOD_DYNAMIC_REGIONS, Value::Null),
            );
            assert_eq!(
                regions["count"], 3,
                "夹具的 2 个动态区 + 刷新进来的探针: {regions}"
            );
            assert!(regions.to_string().contains(&probe), "{regions}");
            assert!(
                !regions.to_string().contains(&stale),
                "`ui/dynamic_regions` 不得还在读上一版 `{stale}`: {regions}"
            );
        }

        // `ui/coverage`：这一版探针在运行时树里、上一版不在 ⇒ 缺失清单**恰好**是上一版。
        {
            let probe = set_refreshed_tree_with_probe(&state);
            let stale = previous_probe_id(&state);
            let coverage = result_of(
                &mut service,
                &token,
                &request(
                    methods::METHOD_COVERAGE,
                    serde_json::json!({"ids": [probe, stale]}),
                ),
            );
            assert_eq!(coverage["runtimeCount"], 4);
            assert_eq!(
                coverage["missingAtRuntime"],
                serde_json::json!([stale]),
                "只有**上一版**探针该被报成缺失（这一版必须在运行时树里）: {coverage}"
            );
        }

        // `ui/screenshot`：遮罩矩形来自树 ⇒ 多一个动态区就多遮一格
        //（少了刷新会是 2 格）。
        {
            let _probe = set_refreshed_tree_with_probe(&state);
            let shot = result_of(
                &mut service,
                &token,
                &request(
                    methods::METHOD_SCREENSHOT,
                    serde_json::json!({"maskDynamic": true}),
                ),
            );
            assert_eq!(
                shot["maskedRegions"], 3,
                "遮罩必须按**刷新后**的树算（夹具 2 + 探针 1）: {shot}"
            );
            assert_eq!(shot["maskEffective"], true, "{shot}");
        }

        // ---- 反证：刷新是**读**路径的行为，注入路径不刷新 ----
        //
        // 注入那一段用**另一个**夹具（端口权限需要 `Interactive`；上面那个夹具是
        // `ReadOnly`，只够读路径）。它不是"另造一个事实源"：作用域与端口闸门是
        // 两层独立判定，这里的差别只是"这具夹具让不让注入"。
        let inject_state = shared(Permission::Interactive);
        let (mut test_mode, test_token) =
            build_service(&inject_state, ScopeSet::all(), RunMode::Test);
        let seen = refreshes(&inject_state);
        let _ = result_of(
            &mut test_mode,
            &test_token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Tab"}),
            ),
        );
        assert_eq!(
            refreshes(&inject_state),
            seen,
            "`ui/dispatch_key_press` 不是读路径 ⇒ 不得触发刷新（重抓是**读**的新鲜度契约，\
             不是注入的副作用）"
        );
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

    /// 判据（`app-mixer` 工作线）：管理动作的**结构化回执**经 `result.report` 原样转达。
    ///
    /// 两条方向都要钉住：
    /// 1. 执行面**没有**回执 ⇒ 载荷与从前**逐字节相同**（既有假面/`PortAdapter` 行为不变）；
    /// 2. 执行面**有**回执 ⇒ `result.report` 就是它交出来的那一份，且**取走**语义成立
    ///    （下一次调用不再重复报上一次的事）。
    #[test]
    fn admin_reports_are_attached_and_absent_when_the_surface_has_none() {
        // 方向 1：默认假面没有回执。
        let plain = shared(Permission::Administrative);
        let (mut service, token) = build_service(&plain, ScopeSet::all(), RunMode::Test);
        let result = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_FORCE_SAVE, Value::Null),
        );
        assert_eq!(
            result,
            serde_json::json!({"accepted": true, "operation": "force_save"}),
            "没有回执时载荷必须与从前逐字节相同"
        );
        assert!(result.get("report").is_none());

        // 方向 2：带回执的执行面。
        let reporting = shared(Permission::Administrative);
        reporting.borrow_mut().report = Some(AdminReport::new(
            "force_save",
            vec![("bytes", ReportValue::Uint(4096))],
        ));
        let (mut service, token) = build_service(&reporting, ScopeSet::all(), RunMode::Test);
        let result = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_FORCE_SAVE, Value::Null),
        );
        assert_eq!(result["report"]["operation"], "force_save");
        assert_eq!(result["report"]["bytes"], 4096);
        assert_eq!(result["accepted"], true, "回执不替代既有的 accepted 字段");

        // 取走语义：同一次会话的第二次调用不再有上一次的回执。
        let again = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_FORCE_SAVE, Value::Null),
        );
        assert!(again.get("report").is_none(), "回执必须被取走, 不能复用");

        // 三个管理动作都能带回执（`switch_main_view` 的载荷多一个 `view` 字段）。
        let switching = shared(Permission::Administrative);
        switching.borrow_mut().report = Some(AdminReport::new(
            "switch_main_view",
            vec![("arrangementView", ReportValue::Bool(false))],
        ));
        let (mut service, token) = build_service(&switching, ScopeSet::all(), RunMode::Test);
        let result = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "session"}),
            ),
        );
        assert_eq!(result["view"], "session");
        assert_eq!(result["report"]["operation"], "switch_main_view");
        assert_eq!(result["report"]["arrangementView"], false);
        assert!(calls(&switching).contains(&"switch_main_view:session".to_owned()));

        let reloading = shared(Permission::Administrative);
        reloading.borrow_mut().report = Some(AdminReport::new(
            "reload_engine",
            vec![("generation", ReportValue::Uint(2))],
        ));
        let (mut service, token) = build_service(&reloading, ScopeSet::all(), RunMode::Test);
        let result = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_RELOAD_ENGINE, Value::Null),
        );
        assert_eq!(result["report"]["generation"], 2);
    }

    /// 判据（`app-mixer` 工作线）：`ui/switch_main_view` 的 `view` 是**白名单参数** ——
    /// 拼错就是 `-32602`（参数问题），而不是 `-32005`（能力没接线）。
    #[test]
    fn switch_main_view_rejects_a_view_name_outside_the_whitelist() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "mixer"}),
            ),
        );
        assert_eq!(error.code, INVALID_PARAMS, "非法视图名必须是参数错");
        assert!(
            calls(&state).is_empty(),
            "参数校验必须先于执行面: {:?}",
            calls(&state)
        );

        // 白名单里的两个值照常放行。
        for view in ["arrangement", "session"] {
            let result = result_of(
                &mut service,
                &token,
                &request(
                    methods::METHOD_SWITCH_MAIN_VIEW,
                    serde_json::json!({"view": view}),
                ),
            );
            assert_eq!(result["view"], view);
        }
    }

    // ===================================================================================
    // `ui-mcp-dryrun-ime` 工作线（ADR-0001 **D48** + `[UI-A11Y-002]`）
    // ===================================================================================
    //
    // 判据的设计原则（与领域侧 dryRun 的判据同一套）：
    //   1. **逐字段**证明状态没变（不是"我没看到变化"），并且**同一个快照**在
    //      dryRun=false 时必须变（否则快照本身是个常数，第一条就是假绿）；
    //   2. dryRun 说的"将要做的事"必须与真做之后的**实际变化**一致；
    //   3. 词表/默认值靠**跨 crate 的类型与真管线载荷**对齐，不靠人眼比对。

    /// 9 条会改状态的方法 + 它们的一次**合法**调用（`dryRun` 由判据自己加）。
    ///
    /// `pointer_down` 用 `mixer-vu-track-0`（夹具里有几何）：`dryRun` 会做真校验，
    /// 而无几何的 `track-0-fader` 是既有判据用来钉 `visible: null` 的，见 `testing.rs`。
    fn mutating_calls() -> Vec<(&'static str, Value)> {
        vec![
            (
                methods::METHOD_DISPATCH_POINTER_DOWN,
                serde_json::json!({
                    "elementId": "mixer-vu-track-0",
                    "xOffset": 1.5, "yOffset": 2.5, "button": "left"
                }),
            ),
            (
                methods::METHOD_DISPATCH_POINTER_MOVE,
                serde_json::json!({"x": 12.5, "y": 34.5}),
            ),
            (
                methods::METHOD_DISPATCH_POINTER_UP,
                serde_json::json!({"button": "right"}),
            ),
            (
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Space"}),
            ),
            (
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "arrangement"}),
            ),
            (
                methods::METHOD_SET_TRACK_HEIGHT,
                serde_json::json!({"elementId": "track-0-header", "heightPx": 96}),
            ),
            (methods::METHOD_FORCE_SAVE, serde_json::json!({})),
            (methods::METHOD_RELOAD_ENGINE, serde_json::json!({})),
            (
                methods::METHOD_OPEN_PROJECT,
                serde_json::json!({"path": "/tmp/yeban-fixture.yeban", "saveFirst": false}),
            ),
        ]
    }

    /// 把 `dryRun` 加进一份实参（不改原对象）。
    fn with_dry_run(params: &Value, dry_run: bool) -> Value {
        let mut object = params.as_object().cloned().unwrap_or_default();
        object.insert(dry_run::DRY_RUN_PARAM.to_owned(), Value::from(dry_run));
        Value::Object(object)
    }

    /// 执行面**真的**会收到的调用日志条目（与 `FakeSurface` 的格式逐字一致）。
    ///
    /// 它由 dryRun 预览里的 `arguments` **算出来**，因此"预览描述的和真做的不是一回事"
    /// 会让判据 ③ 变红。
    fn expected_call_entry(method: &str, arguments: &Value) -> String {
        let text = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let number = |key: &str| arguments.get(key).cloned().unwrap_or(Value::Null);
        match method {
            methods::METHOD_DISPATCH_POINTER_DOWN => format!(
                "pointer_down:{}:{}:{}:{}",
                text("elementId"),
                number("xOffset"),
                number("yOffset"),
                text("button")
            ),
            methods::METHOD_DISPATCH_POINTER_MOVE => {
                format!("pointer_move:{}:{}", number("x"), number("y"))
            }
            methods::METHOD_DISPATCH_POINTER_UP => format!("pointer_up:{}", text("button")),
            methods::METHOD_DISPATCH_KEY_PRESS => format!("key_press:{}", text("keyCode")),
            methods::METHOD_SET_TRACK_HEIGHT => format!(
                "set_track_height:{}:{}",
                text("elementId"),
                number("heightPx")
            ),
            methods::METHOD_SWITCH_MAIN_VIEW => format!("switch_main_view:{}", text("view")),
            methods::METHOD_FORCE_SAVE => "force_save".to_owned(),
            methods::METHOD_RELOAD_ENGINE => "reload_engine".to_owned(),
            // `saveFirst` 的**缺省**由执行面与服务层各自从实参里取同一个默认值；
            // 这里按归一化实参算，因此"预览报的 saveFirst"与"真做用的 saveFirst"
            // 若不一致，本函数算出的日志就会与执行面收到的不符 ⇒ 判据红。
            methods::METHOD_OPEN_PROJECT => format!(
                "open_project:{}:{}",
                text("path"),
                arguments
                    .get("saveFirst")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            ),
            other => panic!("`{other}` 不是会改状态的方法"),
        }
    }

    /// 判据 ①（D48）: `dryRun=true` 下**状态一位都不许变** —— 对 9 条方法逐一证明。
    ///
    /// **逐字段**比较（`Fixture::snapshot` 的每一个键），并额外钉住三件事：
    /// 执行面的调用日志为空（一次都没被调用）、`ui/tree` 的线上 JSON 逐字节相同、
    /// 响应自证 `dryRun/stateUnchanged/wouldChangeState`。
    ///
    /// 注入验证 A（把 `execute` 里的 `if dry_run { return self.preview(..) }` 删掉，
    /// 让它继续往下走真调用）会让本判据变红（日志非空 + 快照变化）。
    #[test]
    fn dry_run_leaves_the_state_untouched_for_every_mutating_method() {
        for (method, params) in mutating_calls() {
            let state = shared(Permission::Administrative);
            let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);

            let tree_before = result_of(
                &mut service,
                &token,
                &request(methods::METHOD_TREE, Value::Null),
            )
            .to_string();
            let before = snapshot(&state);

            let result = result_of(
                &mut service,
                &token,
                &request(method, with_dry_run(&params, true)),
            );
            let after = snapshot(&state);
            let tree_after = result_of(
                &mut service,
                &token,
                &request(methods::METHOD_TREE, Value::Null),
            )
            .to_string();

            assert_eq!(before, after, "`{method}` 的 dryRun 改了状态");
            assert_eq!(tree_before, tree_after, "`{method}` 的 dryRun 改了控件树");
            assert!(
                calls(&state).is_empty(),
                "`{method}` 的 dryRun 把动作交给了执行面: {:?}",
                calls(&state)
            );
            // 逐字段点名（不是只比两个 JSON blob —— 那样"快照本身写错"看不出来）。
            assert_eq!(after["calls"], serde_json::json!([]));
            assert_eq!(after["arrangementView"], before["arrangementView"]);
            assert_eq!(after["saveEpoch"], before["saveEpoch"]);
            assert_eq!(after["engineGeneration"], before["engineGeneration"]);
            assert_eq!(after["hasReport"], before["hasReport"]);

            // 响应自证（"这是模拟"必须写在里面，调用方不用猜）。
            assert_eq!(result[dry_run::DRY_RUN_FLAG], Value::from(true));
            assert_eq!(result[dry_run::STATE_UNCHANGED], Value::from(true));
            assert_eq!(result[dry_run::WOULD_CHANGE_STATE], Value::from(true));
            assert_eq!(result[dry_run::METHOD], Value::from(method));
            let spec = methods::method(method).expect("注册表里有");
            assert_eq!(result[dry_run::REQUIRED_SCOPE], spec.scope.as_str());
            assert_eq!(
                result[dry_run::PREVIEW][dry_run::OPERATION],
                spec.port_operation
                    .expect("会改状态的方法都有端口操作")
                    .as_str()
            );
            // 实参快照里**不**出现 `dryRun` 自身。
            let arguments = result[dry_run::ARGUMENTS].as_object().expect("对象");
            assert!(!arguments.contains_key(dry_run::DRY_RUN_PARAM));
            for (key, value) in params.as_object().expect("对象") {
                assert_eq!(arguments[key.as_str()], *value, "实参快照漏了 `{key}`");
            }
        }
    }

    /// 判据 ②（D48）: `dryRun=false`（缺省）下**确实**改了状态 —— 证明这个参数不是摆设。
    ///
    /// 与判据 ① 用**同一个** `mutating_calls()` 表、**同一个**快照函数：因此
    /// "快照是个常数"这种情况不可能同时让两条判据变绿。
    ///
    /// 注入验证（把 `dry_run::requested` 改成恒 `true`）会让本判据变红。
    #[test]
    fn the_same_calls_without_dry_run_really_change_the_state() {
        for (method, params) in mutating_calls() {
            let state = shared(Permission::Administrative);
            let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
            let before = snapshot(&state);

            let result = result_of(&mut service, &token, &request(method, params.clone()));
            let after = snapshot(&state);

            assert_ne!(before, after, "`{method}` 的真调用什么都没改");
            assert!(!calls(&state).is_empty(), "`{method}` 的真调用没落到执行面");
            assert!(
                result.get(dry_run::DRY_RUN_FLAG).is_none(),
                "真调用的响应不得带 dryRun 旗标: {result}"
            );
            assert_eq!(
                calls(&state),
                [expected_call_entry(method, &params)],
                "`{method}` 落到执行面的实参与入参不符"
            );
        }

        // 三个可观测状态各自**真的**动了（不只是日志变长）。
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        assert!(!state.borrow().arrangement_view);
        result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "arrangement"}),
            ),
        );
        assert!(state.borrow().arrangement_view, "视图开关必须真的被写");
        assert_eq!(state.borrow().save_epoch, 0);
        result_of(
            &mut service,
            &token,
            &request(methods::METHOD_FORCE_SAVE, Value::Null),
        );
        assert_eq!(state.borrow().save_epoch, 1, "保存轮次必须真的加一");
        result_of(
            &mut service,
            &token,
            &request(methods::METHOD_RELOAD_ENGINE, Value::Null),
        );
        assert_eq!(state.borrow().engine_generation, 1, "引擎代数必须真的加一");
    }

    /// 判据 ③（D48）: `dryRun` 说的"将要做的事"与**真做了之后的实际变化**一致。
    ///
    /// 三个方向：
    /// 1. 预览的 `arguments` 与真调用交给执行面的实参逐字一致（`expected_call_entry`）；
    /// 2. 预览的 `effect`（执行面只读算出的"将要变成什么"）与真做之后的**状态读数**相同；
    /// 3. `wouldChangeState` 为真 ⟺ 真调用后状态快照确实变了。
    ///
    /// 注入验证 D（让 `preview_effect` 返回一份编造的影响，例如恒 `saveEpoch = 42`）
    /// 会让本判据变红。
    #[test]
    fn dry_run_preview_predicts_exactly_what_the_real_call_does() {
        for (method, params) in mutating_calls() {
            let state = shared(Permission::Administrative);
            let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);

            let dry = result_of(
                &mut service,
                &token,
                &request(method, with_dry_run(&params, true)),
            );
            assert_eq!(dry[dry_run::WOULD_CHANGE_STATE], Value::from(true));
            let previewed = dry[dry_run::ARGUMENTS].clone();

            let before = snapshot(&state);
            result_of(&mut service, &token, &request(method, params.clone()));
            let after = snapshot(&state);

            // 方向 1: 预览的实参 == 真调用交给执行面的实参。
            assert_eq!(
                calls(&state),
                [expected_call_entry(method, &previewed)],
                "`{method}`: 预览描述的实参与真做的不是一回事"
            );
            assert_eq!(previewed, params, "`{method}`: 预览的实参就是入参");

            // 方向 3: `wouldChangeState` 与"状态真的变了"一致。
            assert_ne!(before, after, "`{method}`: 报了 wouldChangeState 却没变");

            // 方向 2: 执行面报的影响 == 真做之后的状态读数（逐字段）。
            let effect = &dry[dry_run::PREVIEW][dry_run::EFFECT];
            match method {
                methods::METHOD_SWITCH_MAIN_VIEW => {
                    assert_eq!(effect["arrangementView"], after["arrangementView"]);
                    assert_eq!(effect["view"], Value::from("arrangement"));
                }
                methods::METHOD_FORCE_SAVE => {
                    assert_eq!(effect["saveEpoch"], after["saveEpoch"]);
                }
                methods::METHOD_RELOAD_ENGINE => {
                    assert_eq!(effect["generation"], after["engineGeneration"]);
                }
                methods::METHOD_DISPATCH_KEY_PRESS => {
                    // `[UI-A11Y-002]`: 预览必须说清"这一键会不会被输入法吞掉"，
                    // 而它读的是**同一份** IME 状态（判据 ⑥ 钉方向）。
                    assert_eq!(effect["isComposing"], after["imeComposing"]);
                    assert_eq!(
                        effect["resolution"],
                        Value::from(ime::RESOLUTION_PASS_THROUGH)
                    );
                }
                methods::METHOD_SET_TRACK_HEIGHT => {
                    // 预览报出"将要写进去的基准高"；真做之后夹具里就是它。
                    // `currentBasePx` 是**只读**回读 —— 判据 ① 的"dryRun 一个状态位都不变"
                    // 正是靠它与 `before` 相等来作证。
                    assert_eq!(effect["requestedPx"], after["trackHeightPx"]);
                    assert_eq!(effect["currentBasePx"], before["trackHeightPx"]);
                }
                methods::METHOD_OPEN_PROJECT => {
                    // 预览报的那个路径必须**就是**真做之后夹具打开的那个路径
                    // （`FakeSurface` 把 `path` 记进 `Fixture::opened_path`）。
                    assert_eq!(effect["path"], after["openedPath"]);
                    assert_eq!(effect["path"], previewed["path"]);
                }
                // 指针事件的影响只有窗口自己知道 ⇒ 如实报 null（不编造）。
                _ => assert_eq!(effect, &Value::Null, "`{method}` 不该编造影响预览"),
            }
        }
    }

    /// 判据 ⑥（D48 + `[UI-A11Y-002]`）: IME 观测位读的是**真的那份状态机**，不是影子变量。
    ///
    /// 三件事同时钉住：
    /// 1. 观测位（`ui/property` 的 `isComposing`）跟着**驱动点**变（`begin_composition`
    ///    与 `InputContext::begin_composition` 同名同义）；
    /// 2. 同一时刻，`dryRun` 的按键预览读的是**同一份**状态（说"会被输入法吞掉"）——
    ///    一侧接成常量/影子字段，这两个方向就不可能同时对；
    /// 3. `set_focus` 的既有语义（焦点离开文本域 ⇒ 自动结束合成态）在观测位上可见。
    ///
    /// 注入验证 B（把 `ime_state` 接成常量，或把它接到第二个影子字段）会让本判据变红。
    #[test]
    fn ime_state_reads_the_real_state_machine_not_a_shadow_variable() {
        let state = shared(Permission::Interactive);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let read = |service: &mut UiService, token: &BearerToken| {
            result_of(
                service,
                token,
                &request(
                    methods::METHOD_PROPERTY,
                    serde_json::json!({"elementId": "transport-timecode", "name": ime::IME_FIELD}),
                ),
            )
        };

        // 起点: 画布聚焦、没有合成。
        assert_eq!(read(&mut service, &token)["value"], Value::from(false));

        // 驱动点 1: 切到文本域（`InputContext::set_focus` 的语义）。
        state.borrow_mut().set_focus(ime::ImeFocus::TextInput);
        let focused = read(&mut service, &token);
        assert_eq!(focused["focus"], Value::from("text-input"));
        assert_eq!(focused["value"], Value::from(false), "还没开始合成");

        // 驱动点 2: 开始合成。
        state.borrow_mut().begin_composition();
        let composing = read(&mut service, &token);
        assert_eq!(composing["value"], Value::from(true));
        assert_eq!(composing["specId"], Value::from(ime::SPEC_ID));

        // 同一份状态也被 dryRun 的按键预览读到（"这一键会被怎么处置"）。
        let press = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Space", "dryRun": true}),
            ),
        );
        assert_eq!(
            press[dry_run::PREVIEW][dry_run::EFFECT]["resolution"],
            Value::from(ime::RESOLUTION_CONSUMED_BY_IME),
            "合成态下 Space 必须被输入法吞掉 [UI-A11Y-002] MUST"
        );
        assert_eq!(
            press[dry_run::PREVIEW][dry_run::EFFECT]["isComposing"],
            composing["value"],
            "预览与观测位必须读同一份 IME 状态"
        );

        // 驱动点 3: 焦点离开文本域 ⇒ 状态机自己结束合成态（既有语义），观测位必须跟着回。
        state.borrow_mut().set_focus(ime::ImeFocus::MainCanvas);
        let left = read(&mut service, &token);
        assert_eq!(left["value"], Value::from(false));
        assert_ne!(left["value"], composing["value"], "两个方向都必须可区分");
        let press = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_KEY_PRESS,
                serde_json::json!({"keyCode": "Space", "dryRun": true}),
            ),
        );
        assert_eq!(
            press[dry_run::PREVIEW][dry_run::EFFECT]["resolution"],
            Value::from(ime::RESOLUTION_PASS_THROUGH),
            "非合成态下 Space 不会被输入法吞掉（假面只知道两档, 见 testing.rs）"
        );
    }

    /// 判据 ⑤（`[UI-A11Y-002]`）: 合成中 / 非合成中两种状态的观测值**不同**（两个方向都断言）。
    ///
    /// 与判据 ⑥ 的分工：这一条管"可区分"，那一条管"读的是真状态"。
    #[test]
    fn ime_state_distinguishes_composing_from_not_composing() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let read = |service: &mut UiService, token: &BearerToken| {
            result_of(
                service,
                token,
                &request(
                    methods::METHOD_PROPERTY,
                    serde_json::json!({"elementId": "track-0-fader", "name": ime::IME_FIELD}),
                ),
            )
        };

        // 方向 1: 非合成 → 合成。
        let idle = read(&mut service, &token);
        assert_eq!(idle["name"], Value::from(ime::IME_FIELD));
        assert_eq!(idle["value"], Value::from(false));
        assert_eq!(idle["focus"], Value::from("main-canvas"));
        state.borrow_mut().set_focus(ime::ImeFocus::TextInput);
        state.borrow_mut().begin_composition();
        let composing = read(&mut service, &token);
        assert_eq!(composing["value"], Value::from(true));
        assert_ne!(idle["value"], composing["value"], "方向 1");

        // 方向 2: 合成 → 非合成。
        state.borrow_mut().end_composition();
        let done = read(&mut service, &token);
        assert_eq!(done["value"], Value::from(false));
        assert_ne!(composing["value"], done["value"], "方向 2");
        assert_eq!(done["value"], idle["value"]);
    }

    /// 判据 ⑨（D25 + M9）: `dryRun` 不改任何**错误码**，也不绕过任何一道闸门。
    ///
    /// 每一条都是"既有口径 + dryRun"的对照，且都不发明新码：
    /// `-32601` / `-32602` / `-32003` / `-32006` / `-32009` / `-32005`。
    #[test]
    fn dry_run_keeps_every_existing_error_code() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);

        // ① 未知方法（与 dryRun 无关，只是确认码没漂）。
        let error = error_of(&mut service, &token, &request("ui/nope", Value::Null));
        assert_eq!(error.code, jsonrpc::METHOD_NOT_FOUND);

        // ② `dryRun` 类型不符 ⇒ -32602（不是"静默当 false"）。
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_FORCE_SAVE,
                serde_json::json!({"dryRun": "yes"}),
            ),
        );
        assert_eq!(error.code, INVALID_PARAMS);
        assert!(calls(&state).is_empty());

        // ③ 只读方法收到 dryRun ⇒ -32602（**响亮拒绝**，M9：静默忽略最坏）。
        let error = error_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, serde_json::json!({"dryRun": true})),
        );
        assert_eq!(error.code, INVALID_PARAMS);
        let detail = error.data.as_ref().expect("有 data")["detail"]
            .as_str()
            .expect("字符串")
            .to_owned();
        assert!(detail.contains(dry_run::DRY_RUN_PARAM), "{detail}");

        // ④ scope 不够 ⇒ -32003（dryRun **不**绕过授权）。
        let (mut ui_only, ui_token) = build_service(
            &state,
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot, Scope::UiInject]),
            RunMode::Test,
        );
        let error = error_of(
            &mut ui_only,
            &ui_token,
            &request(
                methods::METHOD_FORCE_SAVE,
                serde_json::json!({"dryRun": true}),
            ),
        );
        assert_eq!(error.code, FORBIDDEN);
        assert!(calls(&state).is_empty());

        // ⑤ dryRun 指向不存在的语义 ID ⇒ -32006（不返回"成功预览"）。
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_POINTER_DOWN,
                serde_json::json!({
                    "elementId": "track-9-fader", "xOffset": 1.0, "yOffset": 2.0,
                    "button": "left", "dryRun": true
                }),
            ),
        );
        assert_eq!(error.code, ELEMENT_NOT_FOUND);
        assert_eq!(
            error.data.as_ref().expect("有 data")["kind"],
            "element-not-found"
        );
        assert!(calls(&state).is_empty());

        // ⑥ dryRun 指向无几何的元素 ⇒ -32009（与真执行面 `MissingGeometry` 同码）。
        let error = error_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_DISPATCH_POINTER_DOWN,
                serde_json::json!({
                    "elementId": "track-0-fader", "xOffset": 1.0, "yOffset": 2.0,
                    "button": "left", "dryRun": true
                }),
            ),
        );
        assert_eq!(error.code, GEOMETRY_UNAVAILABLE);
        assert!(calls(&state).is_empty());

        // ⑦ 执行面判定"这次真调用一定会失败" ⇒ dryRun 与真调用**同码同句**（-32005）。
        let refusing = shared(Permission::Administrative);
        refusing.borrow_mut().reject_admin = true;
        let (mut refusing_service, refusing_token) =
            build_service(&refusing, ScopeSet::all(), RunMode::Test);
        let dry = error_of(
            &mut refusing_service,
            &refusing_token,
            &request(
                methods::METHOD_FORCE_SAVE,
                serde_json::json!({"dryRun": true}),
            ),
        );
        let real = error_of(
            &mut refusing_service,
            &refusing_token,
            &request(methods::METHOD_FORCE_SAVE, Value::Null),
        );
        assert_eq!(dry.code, NOT_IMPLEMENTED);
        assert_eq!(dry.code, real.code, "dryRun 与真调用的错误码必须一致");
        assert_eq!(dry.message, real.message, "连话都得一样");
        assert_eq!(dry.data, real.data);
        assert!(calls(&refusing).is_empty(), "dryRun 不得留下真调用的痕迹");
    }

    /// 判据 ⑩（D48）: `dryRun` 自身**不消耗**任何一次性令牌 / 权限 / 计数语义。
    ///
    /// 本服务没有幂等缓存与一次性令牌（领域侧的 `idempotencyKey` 也没进 UI 控制面），
    /// 唯一的计数是 `handled`（**请求数**，不是令牌）。因此这条判据钉住的是：
    /// 同一个 token 先 dryRun 再真调用都能成功、授权的 scope 集合一位不变。
    #[test]
    fn dry_run_consumes_no_scope_and_no_token() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);
        let granted_before = service.granted().to_spec_string();
        let mode_before = service.mode();

        let dry = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "arrangement", "dryRun": true}),
            ),
        );
        assert_eq!(dry[dry_run::DRY_RUN_FLAG], Value::from(true));
        assert_eq!(service.granted().to_spec_string(), granted_before);
        assert_eq!(service.mode(), mode_before);

        // 同一个 token 紧接着做**真**调用：成功（没被 dryRun 用掉）。
        let real = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "arrangement"}),
            ),
        );
        assert_eq!(real["accepted"], Value::from(true));
        assert!(state.borrow().arrangement_view);
        assert_eq!(
            calls(&state),
            ["switch_main_view:arrangement".to_owned()],
            "dryRun 不得在调用日志里留痕"
        );

        // 第二次 dryRun 仍然可用（没有"一次性"这回事）。
        let dry_again = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_SWITCH_MAIN_VIEW,
                serde_json::json!({"view": "session", "dryRun": true}),
            ),
        );
        assert_eq!(dry_again[dry_run::DRY_RUN_FLAG], Value::from(true));
        assert!(
            state.borrow().arrangement_view,
            "第二次 dryRun 也不许改状态"
        );
    }

    /// 判据 ⑧（D48）: `dryRun` **没有污染真调用的响应形状** —— 既有 14 条方法的线上
    /// 结果与从前逐字节相同（`dryRun` 的三个旗标只出现在 `dryRun=true` 的响应里）。
    #[test]
    fn dry_run_does_not_leak_into_the_real_response_shapes() {
        let state = shared(Permission::Administrative);
        let (mut service, token) = build_service(&state, ScopeSet::all(), RunMode::Test);

        for (method, params) in mutating_calls() {
            let result = result_of(&mut service, &token, &request(method, params));
            for field in [
                dry_run::DRY_RUN_FLAG,
                dry_run::STATE_UNCHANGED,
                dry_run::WOULD_CHANGE_STATE,
                dry_run::PREVIEW,
            ] {
                assert!(
                    result.get(field).is_none(),
                    "`{method}` 的真调用响应里出现了 dryRun 字段 `{field}`: {result}"
                );
            }
        }

        // 只读方法一个字都没变（抽两条有代表性的：形状与字段值）。
        let tree = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_TREE, Value::Null),
        );
        assert!(tree.get("tree").is_some() && tree.get("filter").is_some());
        assert!(tree.get(dry_run::DRY_RUN_FLAG).is_none());
        let property = result_of(
            &mut service,
            &token,
            &request(
                methods::METHOD_PROPERTY,
                serde_json::json!({"elementId": "track-0-fader", "name": "value"}),
            ),
        );
        assert_eq!(property["value"], Value::from("value=1"), "字符串口径没变");
        assert!(property.get("focus").is_none(), "普通属性没有 IME 附加字段");
    }

    /// 判据（可发现性 ⑧⑤的补充）: `ui/methods` 的**运行态**载荷也自报 IME 虚拟属性名 ——
    /// AI 第一站就能知道"这个控制面能不能读合成态"。
    #[test]
    fn methods_discovery_announces_the_ime_property_and_dry_run() {
        let state = shared(Permission::ReadOnly);
        let (mut service, token) = build_read_service(&state);
        let result = result_of(
            &mut service,
            &token,
            &request(methods::METHOD_METHODS, Value::Null),
        );
        assert_eq!(result["dryRunParam"], Value::from(dry_run::DRY_RUN_PARAM));
        assert_eq!(result["imeProperty"], Value::from(ime::IME_FIELD));
        let listed = result["methods"].as_array().expect("数组");
        let supporting: Vec<&str> = listed
            .iter()
            .filter(|entry| entry["dryRunSupported"].as_bool() == Some(true))
            .map(|entry| entry["name"].as_str().expect("字符串"))
            .collect();
        assert_eq!(supporting.len(), 9, "支持 dryRun 的方法: {supporting:?}");
        for method in supporting {
            assert!(
                methods::method(method).expect("注册表里有").mutating,
                "`{method}` 是只读方法却自称支持 dryRun"
            );
        }
    }
}
