//! # yeban-mcp — 领域 MCP / Intent API v2
//!
//! 夜半 (Yeban) 的 `yeban-mcp` 兼具库与独立可执行文件**双形态**：
//!
//! - **库形态**（形态 A）：内嵌进 `yeban-app` 进程，供 AI Agent 在运行中的 DAW 上工作；
//! - **二进制形态**（形态 B）：`yeban-mcp` 走 stdio 批处理，适用于无 GUI 环境的
//!   算法编曲流水线与 CI 自动化母带渲染（`ROAD-M4-002`）。
//!
//! 两个形态共享同一套工具注册表、同一套鉴权与同一套分发管线 —— 不存在
//! "库里那套"与"CLI 里那套"两份实现。
//!
//! ## 模块地图
//!
//! | 模块 | 职责 | 规范 |
//! | :--- | :--- | :--- |
//! | [`tools`] | 十个 `yeban_*` 工具的注册表、参数 schema、错误码 | `MCP-TOOL-001..010`, `ROAD-M4-003` |
//! | [`domain`] | **十个工具的领域实现**：`Op` 驱动的可逆变更、原子落盘、稀疏视图、提案 | `MCP-TOOL-001..010`, `ARCH-OPS-001/002`, `ARCH-SEC-004` |
//! | [`security`] | Bearer Token、`0600` 令牌文件、六级 scope、`ui:inject` 硬禁 | `ARCH-SEC-002`, `MUST-GATE-009` |
//! | [`jsonrpc`] | JSON-RPC 2.0 请求 / 响应 / 错误对象（手写 `Serialize`） | `MCP-TOOL-001..010` |
//! | [`dispatch`] | 解析 → 鉴权 → scope → 分发 → `dryRun` 短路 → 幂等去重 | `MCP-TOOL-001..010` |
//! | [`transport`] | [`transport::stdio`]（默认）与 [`transport::http`]（环回、显式开关） | `ARCH-SEC-002` |
//! | [`samples`] | 规范样本导出（跨语言契约对账的输入） | `MUST-GATE-010`, `TEST-SPEC-005` |
//! | [`payload`] | MCP 往返**载荷统计**（字节；Token 半边等口径裁决） | `BASELINE-006` |
//!
//! ## `dryRun` 与 `idempotencyKey` 是怎么被保证的
//!
//! 这两条语义**不靠自觉**，各有一层结构性保证 + 一层运行期判据：
//!
//! | 语义 | 结构性保证 | 运行期判据 |
//! | :--- | :--- | :--- |
//! | `dryRun` 不改状态 | [`domain::plan`] 只接受 `&Domain`（共享引用）⇒ 借用检查器不允许它改任何东西 | `tests/tools_e2e.rs`：前后 `YebanProjectV1` 序列化**逐字节相同** + `CommitGraph` 提交数不变 |
//! | 同 `idempotencyKey` 不重复施加 | 幂等缓存查询（`dispatch` 第 6 步）在工具执行（第 7 步）**之前**；命中就永远到不了 `domain::execute` | `tests/tools_e2e.rs`：同键两次调用后项目哈希与提交数只前进一次 |
//!
//! ## 领域失败的两种出口（`ADR-0001 D25` 之后仍然闭合）
//!
//! - **领域失败**（工程锁被占、片段不存在、提案已被拒绝、渲染失败……）走
//!   `ToolResponse{status:"error", error:{code}}`，`code` 必须落在
//!   `schemas/mcp-tools.schema.json` 的**20 值** enum 里
//!   （[`tools::ErrorCode::SCHEMA_CONTRACT`]）；
//! - **实现级状况**（目前只剩"本平台没有 OS 建议锁"这一条）走 JSON-RPC `-32005`，
//!   [`domain::error::Fault`] 是这两条出口的唯一分叉点。
//!
//! 判据 `tests/tools_e2e.rs::every_emitted_error_code_is_inside_the_contract_enum`
//! 穷举本 crate 能产出的每一个错误码并要求它是契约 enum 的成员。
//!
//! ## 本轮的实现状态（逐工具如实标注）
//!
//! 十个工具**全部接了真实现**，包括 `yeban_render_master` 的渲染本体：
//! 它真的调用 `yeban-render`（拓扑分层并行 + 固定顺序归约 + TPDF 抖动 +
//! RIFF/RF64/BW64 容器 + BWF `bext`），并把产物按 `ARCH-SEC-004` 的
//! 同目录临时文件 + `fsync` + `rename` 原子落盘。明确的**能力边界**（音频片段、
//! 设备链 DSP、自动化曲线、连击/概率/弯音/歌词、循环重复……）同时写进
//! `docs/ledger/mcp-render-notes.md` 与每次响应的 `unsupported` 载荷。
//! 逐工具的"真做 / 半做 / 未接线"表在 `docs/ledger/tools-domain-notes.md`。
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-mcp
//! bash scripts/gates/run-gates.sh crate yeban-mcp
//! bash scripts/dev/cargo-local.sh test -p yeban-mcp --features mcp-http
//! ```
//! ## 默认安全模型（**这是本 crate 存在的第一理由**）
//!
//! | 红线 | 默认状态 | 落点 |
//! | :--- | :--- | :--- |
//! | 网络监听 | **关**（编译期 + 运行期两道开关） | `Cargo.toml` 的 `default = []`；[`transport::plan_http_startup`] |
//! | 监听地址 | 只有 `127.0.0.1`，端口 `0` 动态分配 | [`transport::http::HttpServer::bind_loopback`] + [`transport::http::assert_loopback`] |
//! | Bearer Token | 256-bit，落 `~/.yeban/session.token`，权限恰好 `0600` | [`security::BearerToken::generate`]、[`security::TokenFile`] |
//! | 请求鉴权 | 缺失 / 形状非法 / 不匹配 ⇒ `401`，**不回退放开** | [`security::authenticate`] |
//! | 权限作用域 | 六级 RBAC，`app:admin` **不**隐含任何 `ui:*` | [`security::Scope`]、[`security::ScopeSet::grants`] |
//! | `ui:inject` | 生产模式硬禁（且判定不依赖 token 是否正确） | [`security::authorize`] |
//!
//! ### 为什么 HTTP 传输默认关闭
//!
//! `MUST-GATE-009` 的原文是"发布版默认关闭网络监听"。这里把它做成**两道独立开关**：
//!
//! 1. **编译期** `--features mcp-http`（它不在 `default` 里）—— 默认 release 构建里
//!    连监听循环都不存在，攻击面为零；
//! 2. **运行期** `--enable-mcp-http` —— 编译进去了也不代表要在生产里开。
//!
//! 判据 `transport::tests::http_is_off_unless_explicitly_enabled_at_runtime` 与
//! `tests/contract.rs::manifest_default_features_do_not_enable_mcp_http` 分别钉住这两道。
//!
//! ### 唯一的诚实缺口
//!
//! `transport::http` 模块在 `cfg(test)` 下也会编译（不只是 `feature = "mcp-http"`）。
//! 理由：纯 feature 门控会让这条线最危险的一段代码在 CI 里**从未被编译过**，
//! 而未编译的代码不是判据（`docs/DEVELOPMENT_LEDGER.md` 的 L6 教训）。
//! 真正的监听循环 `HttpServer::serve_forever` 仍然只在 feature 下存在。
//!
//! ## 契约对账的状态（**读这一节再改代码**）
//!
//! - 工具名集合 / `dryRun` / `idempotencyKey` 的权威定义在
//!   `schemas/mcp-tools.schema.json` 的 `definitions.ToolCall`；
//! - 每个工具的参数与错误码在 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 的表格；
//! - **错误码取联集 20 值**（[`tools::ErrorCode::SCHEMA_CONTRACT`]）。第 1 轮本线实测出
//!   "schema 原来的 7 值 enum 装不下规范表格的 16 个错误码（13 个没有家）"，
//!   已由 **ADR-0001 D25** 修好；判据随之从"钉住缺口"升级成"实现集合 == 契约集合"
//!   （`tests/contract.rs::implementation_error_codes_equal_the_contract_enum_exactly`）。
//!   历史留痕见 `docs/ledger/mcp-core-notes.md` §3.1。
//! - **契约是承重的**：D25 把 schema 的根改成 `oneOf($ref ToolCall, $ref ToolResponse)`，
//!   因此每一份导出样本都必须真的是两种形状之一，`validate_schemas.py --samples-dir`
//!   在故意违法的样本上会真变红（判据
//!   `tests/contract.rs::contract_rejects_a_deliberately_invalid_sample`）。
//!   在 D25 之前那个根是"空对象"，任意对象都通过 —— 详见 notes §3.2。
//!
//! ## 本轮的实现状态
//!
//! **分发 / 鉴权 / `dryRun` / 幂等 / 传输 / 十个工具的领域实现都是真的** ——
//! 包括 `yeban_render_master` 的**渲染本体**：它真的从活跃工程构造 `RoutingGraph`
//! 与音源、编译并执行 `yeban_render::RenderPlan`、写出 24-bit RIFF/RF64/BW64 母带
//! （TPDF 抖动 + BWF `bext`），并在响应里给出实测的帧数/字节数/SHA-256/块数/
//! 最长延迟路径。`dryRun` 走同一条**只读**渲染路径，因此预览里的数字是实测值，
//! 且一个字节都不落盘。
//!
//! 明确的**能力边界**（设备链 DSP、自动化曲线、`ratchet`/
//! 弯音/歌词、循环重复、侧链键控……；本模型版本里没有 SFZ 设备变体）逐条写在
//! `docs/ledger/mcp-render-notes.md`，只要工程里真的出现就会出现在响应的
//! `unsupported` / `unsupportedCounts` 载荷里 —— 不声称渲染了没渲染的东西。
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-mcp
//! bash scripts/gates/run-gates.sh crate yeban-mcp
//! bash scripts/dev/cargo-local.sh test -p yeban-mcp --features mcp-http
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod dispatch;
pub mod domain;
pub mod jsonrpc;
pub mod payload;
pub mod samples;
pub mod security;
pub mod tools;
pub mod transport;
pub mod undo_session;

pub use dispatch::{Dispatcher, Outcome};
pub use domain::Domain;
pub use jsonrpc::{Request, Response};
pub use security::{AuthContext, BearerToken, RunMode, Scope, ScopeSet, TokenFile};
pub use tools::{TOOLS, ToolSpec};
