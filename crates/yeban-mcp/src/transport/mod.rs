//! 两种传输形态：stdio（形态 B）与环回 HTTP（形态 A）[ARCH-SEC-002, MUST-GATE-009]。
//!
//! | 形态 | 通道 | 鉴权 | 默认 |
//! | :--- | :--- | :--- | :--- |
//! | B（无头 CLI） | [`stdio`]：逐行 JSON-RPC | 进程边界即能力边界 | **开** |
//! | A（内嵌 app） | [`http`]：`127.0.0.1:0` | `Authorization: Bearer <TOKEN>` | **关** |
//!
//! ## 两道开关（缺一不可）
//!
//! HTTP 形态要同时满足：
//!
//! 1. **编译期**：`--features mcp-http`（它**不**在 `default` 里，见 `Cargo.toml`）；
//! 2. **运行期**：`--enable-mcp-http` 显式开关（见 [`plan_http_startup`]）。
//!
//! 这是刻意的冗余：编译进去了不等于要在生产里开，运行期打开了也不等于这个
//! 二进制里真的有那段代码。[`plan_http_startup`] 把两件事合成一个纯函数，
//! 判据可以分别钉住四种组合。
//!
//! ## 为什么 `http` 模块的 `cfg` 里有一个 `test`
//!
//! `#[cfg(feature = "mcp-http")]` 单独一门是不够的：CI 默认档位不传
//! `--all-features`，那段代码就会**从未被编译过**。未编译的代码不是判据 ——
//! 它连"语法错"都能躲过门禁（`docs/DEVELOPMENT_LEDGER.md` 的 L6 教训：
//! 那正是 `crates/yeban-app/src/test_port_adapter.rs` 曾经的状态）。
//!
//! 因此 `http` 模块在 `cfg(test)` 下也编译：单元判据与 `clippy --all-targets`
//! 都能看到它；而**真正的监听循环** [`http::HttpServer::serve_forever`] 仍然只在
//! `feature = "mcp-http"` 下存在。默认 release 构建里既没有监听循环，
//! 也没有任何 HTTP 代码被链接进来。

pub mod stdio;

#[cfg(any(feature = "mcp-http", test))]
pub mod http;

/// 运行期开启 HTTP 传输的显式开关。
pub const ENABLE_HTTP_FLAG: &str = "--enable-mcp-http";

/// 编译期开启 HTTP 传输的 cargo feature 名。
pub const HTTP_FEATURE_NAME: &str = "mcp-http";

/// `true` 表示当前构建把 HTTP 传输编译进来了。
#[must_use]
pub const fn http_feature_compiled() -> bool {
    cfg!(feature = "mcp-http")
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
    /// 运行期要求开 HTTP，但这个二进制没有把 `mcp-http` 编译进来。
    #[error("本次构建未编译 `mcp-http` feature: 请用 `--features mcp-http` 重新构建")]
    HttpFeatureNotCompiled,
}

/// **纯函数**：由"运行期是否显式给了开关"推出启动决策。
///
/// | `explicitly_enabled` | `feature = "mcp-http"` | 结果 |
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
        assert_eq!(ENABLE_HTTP_FLAG, "--enable-mcp-http");
        assert_eq!(HTTP_FEATURE_NAME, "mcp-http");
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
