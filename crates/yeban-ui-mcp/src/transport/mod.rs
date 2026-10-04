//! 两种传输形态：stdio（进程边界）与环回 HTTP（联网形态）[ARCH-UI-004, MUST-GATE-009]。
//!
//! | 形态 | 通道 | 鉴权 | 默认 |
//! | :--- | :--- | :--- | :--- |
//! | stdio | [`stdio`]：逐行 JSON-RPC | 进程边界即能力边界（`Credential::LocalProcess`） | **开** |
//! | 环回 HTTP | [`http`]：`127.0.0.1:0` | `Authorization: Bearer <TOKEN>` | **关** |
//!
//! ## 两道开关（缺一不可）
//!
//! HTTP 形态要同时满足：
//!
//! 1. **编译期** `--features ui-mcp-http`（它**不**在 `default` 里，见 `Cargo.toml`）；
//! 2. **运行期** `--enable-ui-mcp-http` 显式开关（见 [`plan_http_startup`]）。
//!
//! 这是刻意的冗余（与 `yeban-mcp` 的 `transport` 完全同款）：编译进去了不等于要在生产里开，
//! 运行期打开了也不等于这个二进制里真的有那段代码。[`plan_http_startup`] 把两件事合成一个
//! 纯函数，判据可以分别钉住四种组合。
//!
//! ## 复用而不是重写（**这一条要读清楚**）
//!
//! `[ARCH-SEC-002]` 的安全约束不是"每个 crate 各写一遍"，而是"只有一处能判"。
//! 因此本线的 HTTP 报文层**直接复用** `yeban-mcp` 的公开项：
//!
//! | 复用项 | 干什么 | 不重写的理由 |
//! | :--- | :--- | :--- |
//! | `yeban_mcp::transport::http::assert_loopback` / `assert_loopback_peer` | 只允许 `127.0.0.1` | 环回判定是安全边界，两份实现就会有两套漏洞面 |
//! | `...::parse_head` / `split_message` | 请求行/头解析、头体切分 | 报文的边界条件（非法版本 505、头太大 431）已经在那边有 21 条判据 |
//! | `...::HttpResponse` / `status_reason` | 响应序列化 | 状态行/头/`Connection: close` 的形状必须与领域 MCP 完全一致 |
//! | `...::HttpError` | 协议层错误 → 状态码 | 同上 |
//! | `...::MAX_REQUEST_BODY_BYTES` / `SUPPORTED_METHOD` / `BEARER_CHALLENGE` | 上限与常量 | 两条 MCP 的限额不该不同 |
//!
//! **仍然重写的那一小块**：accept 循环与"从 `TcpStream` 读头/读体"（约 70 行）。
//! 原因不是偏好：`yeban-mcp` 的 `HttpServer` 把 `Mutex<yeban_mcp::dispatch::Dispatcher>`
//! 钉在结构体里（`http.rs:454`），而 UI 控制面的分发器是 [`crate::service::UiService`]
//! —— 两者不是同一个类型，改那边的结构体会越过本线的文件边界（`crates/yeban-mcp/**`
//! 属于 `line/mcp-core`）。因此这里只重写"驱动"部分，**协议语义与安全判定全部复用**。
//! 判据 `http_is_off_unless_explicitly_enabled_at_runtime` 与
//! `http::tests::bind_is_loopback_only_and_uses_a_dynamic_port` 分别钉住两道开关与绑定地址。

pub mod stdio;

#[cfg(any(feature = "ui-mcp-http", test))]
pub mod http;

/// 本服务的端点路径（**与领域 MCP 的 `/mcp` 刻意不同**，见 [`http`] 的文档）。
///
/// 这里而不是 `http` 模块里定义，是因为 `http` 只在 feature/`cfg(test)` 下编译，
/// 而样本导出（`crate::samples`）在默认构建里也要能引用它。
pub const UI_MCP_PATH: &str = "/ui-mcp";

/// 运行期开启 HTTP 传输的显式开关。
pub const ENABLE_HTTP_FLAG: &str = "--enable-ui-mcp-http";

/// 编译期开启 HTTP 传输的 cargo feature 名。
pub const HTTP_FEATURE_NAME: &str = "ui-mcp-http";

/// `true` 表示当前构建把 HTTP 传输编译进来了。
#[must_use]
pub const fn http_feature_compiled() -> bool {
    cfg!(feature = "ui-mcp-http")
}

/// HTTP 传输的启动决策。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpStartup {
    /// 不启动（**默认**）。
    Disabled,
    /// 启动。
    Enabled,
}

/// 启动决策失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// 运行期要求开 HTTP，但这个二进制没有把 `ui-mcp-http` 编译进来。
    #[error("本次构建未编译 `ui-mcp-http` feature: 请用 `--features ui-mcp-http` 重新构建")]
    HttpFeatureNotCompiled,
}

/// **纯函数**：由"运行期是否显式给了开关"推出启动决策。
///
/// | `explicitly_enabled` | `feature = "ui-mcp-http"` | 结果 |
/// | :--- | :--- | :--- |
/// | `false` | 任意 | [`HttpStartup::Disabled`] |
/// | `true` | 否 | `Err(HttpFeatureNotCompiled)` |
/// | `true` | 是 | [`HttpStartup::Enabled`] |
///
/// # Errors
///
/// 运行期要求开、但编译期没有这个 feature。
pub fn plan_http_startup(explicitly_enabled: bool) -> Result<HttpStartup, TransportError> {
    if !explicitly_enabled {
        return Ok(HttpStartup::Disabled);
    }
    if http_feature_compiled() {
        Ok(HttpStartup::Enabled)
    } else {
        Err(TransportError::HttpFeatureNotCompiled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_is_off_unless_explicitly_enabled_at_runtime() {
        assert_eq!(plan_http_startup(false), Ok(HttpStartup::Disabled));
        assert_eq!(ENABLE_HTTP_FLAG, "--enable-ui-mcp-http");
        assert_eq!(HTTP_FEATURE_NAME, "ui-mcp-http");
    }

    #[test]
    fn enabling_http_without_the_feature_is_an_explicit_error() {
        if http_feature_compiled() {
            assert_eq!(plan_http_startup(true), Ok(HttpStartup::Enabled));
        } else {
            assert_eq!(
                plan_http_startup(true),
                Err(TransportError::HttpFeatureNotCompiled),
                "运行期开关不能凭空造出编译期没进来的代码"
            );
        }
    }
}
