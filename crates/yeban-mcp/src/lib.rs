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
//! | [`security`] | Bearer Token、`0600` 令牌文件、六级 scope、`ui:inject` 硬禁 | `ARCH-SEC-002`, `MUST-GATE-009` |
//! | [`jsonrpc`] | JSON-RPC 2.0 请求 / 响应 / 错误对象（手写 `Serialize`） | `MCP-TOOL-001..010` |
//! | [`dispatch`] | 解析 → 鉴权 → scope → 分发 → `dryRun` 短路 → 幂等去重 | `MCP-TOOL-001..010` |
//! | [`transport`] | [`transport::stdio`]（默认）与 [`transport::http`]（环回、显式开关） | `ARCH-SEC-002` |
//! | [`samples`] | 规范样本导出（跨语言契约对账的输入） | `MUST-GATE-010`, `TEST-SPEC-005` |
//!
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
//! - **两份契约在错误码上冲突**：schema 的 `ToolResponse.error.code` 是闭合的 7 值 enum，
//!   表格里有 13 个错误码不在其中。详见 [`tools::ErrorCode`] 与
//!   `docs/ledger/mcp-core-notes.md` §3.1（本线不擅自改 `schemas/**`）。
//! - `schemas/mcp-tools.schema.json` 的**根没有引用 `definitions`**，因此
//!   `validate_schemas.py --samples-dir` 在本文件上目前是空转的；
//!   承重的判据是 `tests/contract.rs` 里直接读 enum 的集合相等断言。
//!
//! ## 本轮的实现状态
//!
//! **分发 / 鉴权 / `dryRun` / 幂等 / 传输已经是真实现 + 真判据**；
//! 十个工具的**领域实现**尚未接线，一律返回 JSON-RPC `-32005 NOT_IMPLEMENTED`
//! （`data.code = "NOT_IMPLEMENTED"`、`data.detail` 带工具名与规范 ID）。
//! 这是"如实报未实现"，不是"假装成功"。
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-mcp
//! bash scripts/gates/run-gates.sh crate yeban-mcp
//! bash scripts/dev/cargo-local.sh test -p yeban-mcp --features mcp-http
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod dispatch;
pub mod jsonrpc;
pub mod samples;
pub mod security;
pub mod tools;
pub mod transport;

pub use dispatch::{Dispatcher, Outcome};
pub use jsonrpc::{Request, Response};
pub use security::{AuthContext, BearerToken, RunMode, Scope, ScopeSet, TokenFile};
pub use tools::{TOOLS, ToolSpec};
