//! # yeban-ui-mcp — UI 控制面（**不是** Intent API）
//!
//! 夜半 (Yeban) 的**双 MCP** 里，这一半负责"看界面"与（仅测试模式下）"动界面"：
//!
//! ```text
//! +---------------------- 双 MCP (MCP-DUAL-001 / ARCH-SEC-002 §7) ----------------------+
//! |                                                                                      |
//! |  AI Agent ──┬── 领域 MCP:  yeban-mcp        (Intent API v2, 十个 yeban_* 工具)        |
//! |             │     乐理意图 / 工程落盘 / 渲染导出    crates/yeban-mcp                  |
//! |             │                                                                        |
//! |             └── UI 控制面: yeban-ui-mcp      (本 crate)                               |
//! |                   语义控件树 / 属性读取 / Tier-1 截图 / 事件注入(硬禁)                |
//! |                              │                                                        |
//! |                              └── 进程内调用 ──> yeban-ui-test-port                    |
//! |                                                 (UiTestPort + 三级权限)                |
//! +--------------------------------------------------------------------------------------+
//! ```
//!
//! ## 本 crate **不是** Intent API（分工说清楚）
//!
//! | 问题 | 领域 MCP（`yeban-mcp`） | UI 控制面（本 crate） |
//! | :--- | :--- | :--- |
//! | 改什么 | `yeban-model` 的工程状态（音符、轨道、提案） | **界面**：只读内省 + 模拟输入 |
//! | 方法名 | `tools/list` / `tools/call`（MCP 标准），工具名 `yeban_*` | `ui/*`（§12 的能力名，见 [`methods`]） |
//! | 契约 | `schemas/mcp-tools.schema.json`（`oneOf(ToolCall, ToolResponse)`） | **无 schema**：JSON-RPC 2.0，结果是自由对象（见 [`samples`]） |
//! | 默认 | stdio 开、HTTP 关 | 同上（两道开关，见 [`transport`]） |
//! | 生产硬禁 | `ui:inject` scope（`Scope::is_production_forbidden`） | **同一个** scope、**同一个** `authorize` 函数 |
//!
//! 两侧**共用**同一套安全模型 —— 这不是巧合，是刻意的：`yeban_mcp::security` 的
//! 256-bit Token、`~/.yeban/session.token` 的 `0600`、六级 scope、`ui:inject` 的
//! 生产硬禁与"硬禁先于 token 校验"的判定顺序，本 crate **一行都不重写**。
//!
//! ## 模块地图
//!
//! | 模块 | 依赖 Slint? | 职责 | 规范 |
//! | :--- | :--- | :--- | :--- |
//! | [`base64`] | 否 | 零依赖 Base64（JSON 里传 PNG 字节的唯一办法） | — |
//! | [`tree`] | 否 | 语义控件树的 **JSON 投影**（稳定键序 / `visible` 的证据语义 / 动态区矩形） | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` |
//! | [`methods`] | 否 | 14 条方法的名字 / 参数 / scope / 规范出处 | `UI-MCP-001` `UI-TEST-002` `ARCH-UI-004` |
//! | [`service`] | 否 | 管线：方法解析 → 授权（硬禁→token→scope）→ 参数 → 执行 | `UI-MCP-001` `ARCH-SEC-002` `MUST-GATE-009` |
//! | [`live`] | 否 | **真实界面上的控制面装配**（`ControlPlane`）与端到端读数（`ui/tree` → `ui/node` → `ui/screenshot`） | `ARCH-UI-004` `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` `MUST-GATE-015` |
//! | [`surface`] | 否 | 执行面 `UiSurface`（= `UiTestPort` + Tier-1 像素）、像素证据、`PortAdapter` | `MUST-GATE-015` |
//! | [`transport`] | 否 | stdio（默认开）与环回 HTTP（默认关，两道开关） | `ARCH-UI-004` `MUST-GATE-009` `ROAD-M4-002` |
//! | [`samples`] | 否 | 3 份 `.meta.` 文档样本 + 跨语言对账入口 | `MUST-GATE-010` `TEST-SPEC-005` |
//!
//! **本 crate 自己的代码零 Slint 依赖**（Slint 只经 `yeban-ui-test-port` 间接进入依赖图）。
//! 这不是巧合，是纪律换来的：本机禁止编译 Slint，而"零 Slint 的模块"可以用
//! `rustc --edition 2024 --test -D warnings` 在**本机真跑**（做法见
//! `docs/ledger/ui-mcp-notes.md`）。真实窗口的接线由 [`surface::PortAdapter`] 承担 ——
//! 它把"怎么拿像素"变成调用方注入的闭包，于是本 crate 不必写 `slint::ComponentHandle`。
//!
//! ## 默认安全模型（**这是本 crate 存在的第一理由**）
//!
//! | 红线 | 默认状态 | 落点 |
//! | :--- | :--- | :--- |
//! | 网络监听 | **关**（编译期 + 运行期两道开关） | `Cargo.toml` 的 `default = []`；[`transport::plan_http_startup`] |
//! | 监听地址 | 只有 `127.0.0.1`，端口 `0` 动态分配 | [`transport::http::UiHttpServer::bind_loopback`] + 复用 `assert_loopback` + 绑定后回读 |
//! | Bearer Token | 256-bit，落 `~/.yeban/session.token`，权限恰好 `0600` | 复用 `yeban_mcp::security::TokenFile` |
//! | 请求鉴权 | 缺失 / 形状非法 / 不匹配 ⇒ `401`，**不回退放开** | 复用 `yeban_mcp::security::authenticate` |
//! | 权限作用域 | 六级 RBAC，`app:admin` **不**隐含任何 `ui:*` | 复用 `yeban_mcp::security::ScopeSet::grants` |
//! | `ui:inject` | 生产模式硬禁，且判定**先于** token 校验 | 复用 `yeban_mcp::security::authorize` |
//!
//! ## 边界（本 crate 明确不做什么）
//!
//! - **不实现 Tier-1 光栅化**：那是 `yeban-ui-test-port::render`（`SoftwareRenderer` +
//!   `MinimalSoftwareWindow`）的事。本 crate 只消费一张已经画好的 `Rgb8Image`。
//! - **不实现控件树模型**：模型在 `yeban_ui_test_port::tree`，本 crate 只投影（见 [`tree`]）。
//! - **不做 PNG 解码 / SSIM 比对**：`ui/screenshot` 给的是 base64 PNG + 像素证据；
//!   跨进程的图-图比对需要解码器，本线**没有**引入（`yeban-ui-test-port` 手写 PNG 时
//!   明确只做编码）。进程内的遮罩 + SSIM 仍由那边的 `mask` / `ssim` 提供。
//! - **不碰领域状态**：`ui/force_save` 等管理动作在本 crate 里只做**授权 → 转发 → 转达回执**
//!   （[`AdminReport`]）。真正的落盘 / 引擎重建需要 `yeban-model` / `yeban-engine` 句柄，
//!   那属于 `yeban-app` 的接线（`crates/yeban-app/src/live_surface.rs`）—— 本 crate **不**
//!   反向依赖 `yeban-app`（会成环），所以事实由执行面交出来、本 crate 原样放进 `result.report`。
//!
//! ## 怎么跑
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-ui-mcp                       # 含 Slint, 只在 CI 上真跑
//! bash scripts/gates/run-gates.sh crate yeban-ui-mcp                         # 见到 slint 会自动 SKIP 本机档位
//! bash scripts/dev/cargo-local.sh run -p yeban-ui-mcp --example export_ui_samples -- --out target/schema-samples
//! python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples
//! ```

#![deny(missing_docs)]
#![deny(rust_2018_idioms)]
#![forbid(unsafe_code)]

pub mod base64;
pub mod live;
pub mod methods;
pub mod samples;
pub mod service;
pub mod surface;
pub mod transport;
pub mod tree;

#[cfg(test)]
mod testing;

pub use live::{
    CallResult, ControlPlane, LiveProbe, ProbeError, ProbeOptions, ScreenshotProbe,
    find_family_member, find_semantic_node, scopes_for_permission,
};
pub use methods::{METHOD_COUNT, MethodSpec, ParamSpec};
pub use service::{
    CAPTURE_FAILED, ELEMENT_NOT_FOUND, GEOMETRY_UNAVAILABLE, SERVICE_NAME, UiService,
    http_status_for,
};
pub use surface::{
    AdminReport, DEFAULT_MAX_PNG_BYTES, PortAdapter, ReportValue, ShotEvidence, UiSurface,
};
pub use transport::{ENABLE_HTTP_FLAG, HTTP_FEATURE_NAME, HttpStartup};
pub use tree::{Coverage, TreeSource, UiNode, UiTree};

/// 本 crate 覆盖的规范 ID（供审计脚本与 notes 引用）。
///
/// 测法：这些 ID 必须都能在 `docs/YEBAN_*.md` 里 `grep` 到（判据
/// `crate_level_contract_holds` 逐条断言形状；具体出处写在 [`methods`] 的每条方法上）。
pub const IMPLEMENTED_SPEC_IDS: &[&str] = &[
    "ARCH-SEC-002",
    "ARCH-SLINT-001",
    "ARCH-UI-004",
    "ARCH-UI-005",
    "MCP-DUAL-001",
    "MUST-GATE-009",
    "MUST-GATE-010",
    "MUST-GATE-015",
    "ROAD-M4-001",
    "ROAD-M4-002",
    "ROAD-M4-008",
    "TEST-SPEC-005",
    "UI-MCP-001",
    "UI-MCP-002",
    "UI-MCP-003",
    "UI-TEST-001",
    "UI-TEST-002",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据: crate 级契约 —— 规范 ID 清单无重复、形状合法；方法集与错误码常量稳定。
    #[test]
    fn crate_level_contract_holds() {
        let unique: std::collections::BTreeSet<&str> =
            IMPLEMENTED_SPEC_IDS.iter().copied().collect();
        assert_eq!(unique.len(), IMPLEMENTED_SPEC_IDS.len(), "规范 ID 不得重复");
        for id in IMPLEMENTED_SPEC_IDS {
            let (family, number) = id.split_once('-').expect("规范 ID 必须含 `-`");
            assert!(!family.is_empty() && !number.is_empty(), "{id}");
            assert!(
                family.chars().all(|ch| ch.is_ascii_uppercase()),
                "族名必须大写: {id}"
            );
        }

        // 三个新增错误码都落在 JSON-RPC 的服务自定义区 (`-32000..=-32099`),
        // 且**不**与复用的那些撞号。
        for code in [ELEMENT_NOT_FOUND, CAPTURE_FAILED, GEOMETRY_UNAVAILABLE] {
            assert!((-32099..=-32000).contains(&code), "{code} 越出自定义区");
        }
        let reused = [
            yeban_mcp::jsonrpc::PARSE_ERROR,
            yeban_mcp::jsonrpc::INVALID_REQUEST,
            yeban_mcp::jsonrpc::METHOD_NOT_FOUND,
            yeban_mcp::jsonrpc::INVALID_PARAMS,
            yeban_mcp::jsonrpc::INTERNAL_ERROR,
            yeban_mcp::jsonrpc::UNAUTHORIZED,
            yeban_mcp::jsonrpc::FORBIDDEN,
            yeban_mcp::jsonrpc::NOT_IMPLEMENTED,
        ];
        for code in [ELEMENT_NOT_FOUND, CAPTURE_FAILED, GEOMETRY_UNAVAILABLE] {
            assert!(!reused.contains(&code), "{code} 与复用的错误码撞号");
        }
        assert_eq!(
            [ELEMENT_NOT_FOUND, CAPTURE_FAILED, GEOMETRY_UNAVAILABLE],
            [-32006, -32008, -32009]
        );

        assert_eq!(METHOD_COUNT, 14);
        assert_eq!(SERVICE_NAME, "yeban-ui-mcp");
        assert_eq!(ENABLE_HTTP_FLAG, "--enable-ui-mcp-http");
        assert_eq!(HTTP_FEATURE_NAME, "ui-mcp-http");
        assert_eq!(transport::UI_MCP_PATH, "/ui-mcp");
        assert_ne!(
            transport::UI_MCP_PATH,
            "/mcp",
            "双 MCP 的端点路径必须不同 (接错端口要能看出来)"
        );
    }

    /// 判据: 方法的 scope 表**恰好**覆盖六级 scope（没有哪一级是空的）。
    #[test]
    fn every_scope_has_at_least_one_method() {
        use yeban_mcp::security::Scope;
        for scope in Scope::ALL {
            assert!(
                methods::METHODS.iter().any(|spec| spec.scope == scope),
                "scope `{scope}` 没有任何方法 —— 六级分层不该有空洞"
            );
        }
    }
}
