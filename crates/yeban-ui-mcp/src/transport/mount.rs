//! **产品形态的挂载**：把 UI 控制面挂成一个"只绑环回 + Bearer 令牌 + 可停可释放"的
//! 环回服务，由**宿主线程**一拍一拍地泵（[`UiHttpMount::pump`]）。
//!
//! 规范来源 (Normative)：`[ARCH-SEC-002]`、`[MUST-GATE-009]`、`[ARCH-UI-004]`、`[ROAD-M4-001]`。
//!
//! ## 它为什么存在（而不是让宿主自己拼）
//!
//! `crates/yeban-app/src/mcp_mount.rs` 的 `InProcessMcp` 已经把**领域** MCP 的环回控制面
//! 的每一条安全性质证明过一遍：只绑 `127.0.0.1:0`（绑后回读断言）、256-bit 令牌、
//! `~/.yeban/session.token` 恰好 `0600`、令牌写不进去就**停机并释放监听口**。
//! 本类型是同一组性质在 **UI 控制面**上的**装配点**：它不重写任何判定，只把
//! `yeban-mcp` 的既有公开项（[`yeban_mcp::security::TokenFile`] /
//! `BearerToken` / `assert_loopback` / 报文层）与 [`UiHttpServer`] 串成一条可被
//! 宿主持有、被宿主泵、被宿主释放的东西。
//!
//! ## 与 `InProcessMcp` 的**唯一**结构性差别：谁来跑 accept 循环
//!
//! | | 领域 MCP（`InProcessMcp`） | UI 控制面（本模块） |
//! | :--- | :--- | :--- |
//! | 分发器 | `yeban_mcp::Dispatcher`（`Send`） | [`crate::service::UiService`]（**`!Send`**：执行面持 Slint 组件） |
//! | accept 循环 | **自己的工作线程** `serve_loop` | **宿主线程**：宿主调 [`UiHttpMount::pump`] |
//! | 停机 | 停标志 + 唤醒连接 + 有界 `join` | 消费掉挂载（`Drop` ⇒ `close()`） |
//! | 令牌 / 落盘 / 路径 / 权限 | `TokenFile`，`~/.yeban/session.token`，`0600` | **同一份**（[`UiHttpMount::publish_token`]） |
//! | 绑定 | `HttpServer::bind_loopback`（`127.0.0.1:0` + 回读断言） | `UiHttpServer::bind_loopback`（**同款**：同一份 `assert_loopback`） |
//!
//! 所以"为什么不是一个线程"的答案是**类型事实**而不是偏好：`UiService` 里的执行面
//! （`LivePort<MainWindow>`）是 `Rc` 语义的 Slint 组件，`Send` 不成立，把它搬进线程
//! 需要复制窗口状态 —— 那会造出**第二份事实源**，是本仓库最不愿意付的代价
//! （见 `crates/yeban-app/src/live_surface.rs` 的 §`Send` 边界说明）。
//!
//! ## 边界（**明确没做**，不是"忘了"）
//!
//! - 没有慢速攻击防护：泵形态靠 [`UiHttpServer::try_serve_once`] 的**每连接读/写超时**
//!   （2 秒）兜底，而不是靠一条专门的看门狗线程；
//! - 一个连接处理期间，宿主的其它节拍（界面重绘 / 空闲循环）会等它 —— 这是
//!   "服务与界面同线程"的固有代价，写在这里而不是留给读者去猜。

use std::net::SocketAddr;
use std::path::PathBuf;

use yeban_mcp::security::{BearerToken, SecurityError, TokenFile};
use yeban_mcp::transport::http::HttpError;

use crate::service::UiService;
use crate::transport::UI_MCP_PATH;
use crate::transport::http::UiHttpServer;

/// 挂载 / 落令牌过程中的失败。
///
/// 变体只区分"**哪个**既有实现的哪一步失败了"，不发明新的错误分类：
/// 绑定错误就是报文层的 [`HttpError`]，令牌错误就是安全层的
/// [`yeban_mcp::security::SecurityError`] —— 两层都**原样**透出，
/// 因此宿主打印的话与领域 MCP 那句是同一个口径。
#[derive(Debug, thiserror::Error)]
pub enum MountError {
    /// 绑定 / 回读地址 / 泵一个连接失败（[`UiHttpServer`] 的既有错误）。
    #[error("UI 控制面环回传输失败: {0}")]
    Http(#[from] HttpError),
    /// 令牌无法按 `0600` 落到目标路径（家目录不可用 / 非 Unix / I/O）。
    ///
    /// 落到这个变体时宿主**必须**释放监听口（fail closed）：一个外部 Agent 拿不到
    /// 令牌的监听口只是多出来的攻击面。
    #[error("会话令牌落盘失败: {0}")]
    Token(#[from] SecurityError),
}

/// 一个**已经绑好、还没开始服务**的 UI 控制面环回挂载。
///
/// 它持有三样东西：监听 socket、回读到的**环回**地址、以及这个会话期望的令牌。
/// **服务不由它自己驱动** —— 宿主线程调 [`Self::pump`]（见模块文档的差别表）。
///
/// 释放监听口的方式只有一条：**消费掉它**（[`Self::stop`] 或直接 `drop`）。
/// 于是"没有东西在监听"是 `TcpStream::connect` 可观察到的事实，而不只是一句声明。
pub struct UiHttpMount {
    /// 监听 socket（`bind_loopback` 已经回读断言过它是环回，并且切成了非阻塞）。
    server: UiHttpServer,
    /// 绑定后回读到的地址（便于报错时点名，也便于宿主打印端点）。
    address: SocketAddr,
    /// 这个会话期望的令牌（`Debug` 是脱敏的；**不要**把它写进日志）。
    token: BearerToken,
}

/// 手写而不是 `derive`：`UiService` 不是 `Debug`（它内含执行面 trait 对象），
/// 而这里真正需要被打印的只有**地址**与**脱敏令牌**。
impl std::fmt::Debug for UiHttpMount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UiHttpMount")
            .field("address", &self.address)
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl UiHttpMount {
    /// 绑定 `127.0.0.1:0`（**只绑环回**、端口由系统分配、绑后回读断言），并把监听
    /// socket 切成**非阻塞**（泵形态的前提）。
    ///
    /// 这一步**不**写任何文件、**不**开任何线程。令牌落盘是**另一件**可分别观测的事
    /// （[`Self::publish_token`]）—— 于是"绑好了但令牌没落盘"这个中间态是可被宿主
    /// 处理的，而不是被藏进构造函数里。
    ///
    /// # Errors
    ///
    /// 绑定失败、回读到的地址不是环回（红线，不可发生但必须判），或设置非阻塞失败。
    pub fn bind_loopback(service: UiService) -> Result<Self, MountError> {
        let server = UiHttpServer::bind_loopback(service)?;
        let address = server.local_addr()?;
        let token = server.token();
        // 泵形态：宿主在自己的循环里调 `pump()`，因此监听 socket 必须非阻塞。
        // （漏了这一步 `pump()` 会在空队列上阻塞 —— 也就是把界面线程冻住。）
        server.set_nonblocking(true)?;
        Ok(Self {
            server,
            address,
            token,
        })
    }

    /// 实际监听地址（**环回** + 系统动态分配的端口）。
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// 外部调用方要用的端点 URL（`http://127.0.0.1:<port>/ui-mcp`）。
    ///
    /// 路径是 [`UI_MCP_PATH`]（`/ui-mcp`）—— 与领域 MCP 的 `/mcp` **刻意不同**：
    /// 接错端口要能一眼看出来。
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("http://{}{UI_MCP_PATH}", self.address)
    }

    /// 期望的会话令牌。**不要**把它写进日志 / 报告行（[`Self::publish_token`] 只报路径）。
    #[must_use]
    pub fn token(&self) -> &BearerToken {
        &self.token
    }

    /// **泵一拍**：非阻塞地服务至多一个待处理连接。
    ///
    /// | 返回值 | 含义 |
    /// | :--- | :--- |
    /// | `Ok(true)` | 服务了一个连接（队列里可能还有 ⇒ 宿主应当立刻再调一次） |
    /// | `Ok(false)` | 此刻没有连接（宿主可以去干别的） |
    ///
    /// # Errors
    ///
    /// 见 [`UiHttpServer::try_serve_once`]：accept 失败，或该连接的 socket I/O 失败。
    /// 宿主**不应**把一次连接级失败当成停机理由（领域 MCP 的工作线程同款：
    /// 退避一下继续），但也不该把它吞成一个"服务正常"的假象。
    pub fn pump(&self) -> Result<bool, MountError> {
        Ok(self.server.try_serve_once()?)
    }

    /// **默认落点**的令牌落盘：`~/.yeban/session.token`，权限恰好 `0600`，返回**路径**。
    ///
    /// 为什么不打印令牌本身：令牌进日志就等于进 CI 制品与终端回滚缓冲。
    /// 这是 `yeban-mcp` 形态 B 与 `yeban-app` 的形态 A 已经在用的做法 ——
    /// 本方法**复用同一个** [`TokenFile`]，因此两条控制面的令牌文件语义不可能漂移。
    ///
    /// # Errors
    ///
    /// 定位不到家目录、非 Unix 平台（`0600` 无法校验 ⇒ 明确拒绝而不是静默放过）、或 I/O 失败。
    /// 落到这个结果时宿主**必须**释放监听口（fail closed）。
    pub fn publish_token(&self) -> Result<PathBuf, MountError> {
        self.publish_token_at(TokenFile::at_default_path()?)
    }

    /// 把令牌写到**指定** `TokenFile`（判据用它在一个受控目录上验证 `0600` 与"只报路径"）。
    ///
    /// 生产路径走 [`Self::publish_token`]；两者是**同一条**实现，不是两份。
    ///
    /// # Errors
    ///
    /// 同 [`TokenFile::save`]。
    pub fn publish_token_at(&self, file: TokenFile) -> Result<PathBuf, MountError> {
        file.save(&self.token)?;
        Ok(file.path().to_path_buf())
    }

    /// 停机：**消费掉**挂载 ⇒ 监听 socket 被真的关掉（`TcpListener` 的 `Drop`）。
    ///
    /// 消费 `self` 与 `InProcessMcp::stop` 的签名同款：停止之后"还在监听"在类型上
    /// 就不可能被误用。
    pub fn stop(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeSurface, fixture_tree, shared};
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use yeban_mcp::security::{RunMode, ScopeSet};
    use yeban_ui_test_port::port::Permission;

    /// 一个只读的 UI 服务（与 `transport::http` 的判据装置同款）。
    fn service() -> UiService {
        UiService::new(
            BearerToken::generate().token,
            ScopeSet::all(),
            RunMode::Production,
            Box::new(FakeSurface {
                state: shared(Permission::Administrative),
                tree: fixture_tree(),
            }),
        )
    }

    fn mount() -> UiHttpMount {
        UiHttpMount::bind_loopback(service()).expect("绑环回")
    }

    /// 判据: 只绑环回 + 动态端口 + 端点路径与领域 MCP 不同。
    #[test]
    fn bind_is_loopback_only_and_the_endpoint_is_not_the_domain_one() {
        let mount = mount();
        let address = mount.address();
        assert!(address.ip().is_loopback(), "必须绑在环回上: {address}");
        assert_ne!(address.port(), 0, "端口 0 是**请求**，系统分配后必须非 0");
        assert_eq!(
            mount.endpoint(),
            format!("http://{address}{UI_MCP_PATH}"),
            "端点必须由回读到的地址拼出来"
        );
        assert!(
            mount.endpoint().ends_with("/ui-mcp"),
            "UI 控制面的路径必须与领域 MCP 的 `/mcp` 不同: {}",
            mount.endpoint()
        );
        // 令牌内容不进 Debug（脱敏）—— 报告行只能报路径。
        let debug = format!("{:?}", mount.token());
        assert!(
            !debug.contains(mount.token().expose()),
            "令牌的 Debug 必须脱敏: {debug}"
        );
    }

    /// 判据: 泵形态真的在**真环回 socket** 上服务一个请求，且空队列返回 `Ok(false)`。
    #[test]
    fn pump_serves_a_real_request_and_reports_an_idle_queue() {
        let mount = mount();
        let address = mount.address();
        assert!(!mount.pump().expect("空队列"), "没有连接时必须是 Ok(false)");

        let body = r#"{"jsonrpc":"2.0","id":7,"method":"ui/tree"}"#;
        let raw = format!(
            "POST {UI_MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            mount.token().expose(),
            body.len()
        );
        let mut stream = TcpStream::connect(address).expect("连环回");
        stream.write_all(raw.as_bytes()).expect("写请求");
        stream.flush().expect("刷");
        assert!(
            mount.pump().expect("服务一个连接"),
            "排队中的连接必须被服务"
        );
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("读响应");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(
            response.contains("\"track-0-fader\""),
            "响应当携带运行时控件树: {response}"
        );
        assert!(
            !mount.pump().expect("空队列"),
            "服务完之后队列又空了 ⇒ Ok(false)"
        );
    }

    /// 判据: 令牌落盘**只报路径**、权限恰好 `0600`、内容可回读（而报告行里没有它）。
    #[test]
    fn publish_token_writes_0600_and_returns_only_the_path() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/ui-mcp-mount-test")
            .join(format!("{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建测试目录");
        let path = dir.join("session.token");
        let _ = std::fs::remove_file(&path);

        let mount = mount();
        let reported = mount
            .publish_token_at(TokenFile::new(path.clone()))
            .expect("落令牌");
        assert_eq!(reported, path, "报告行里只能出现**路径**");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, yeban_mcp::security::TOKEN_FILE_MODE, "必须是 0600");
        }
        let content = std::fs::read_to_string(&path).expect("读回");
        assert_eq!(content.trim(), mount.token().expose(), "文件内容 = 令牌");
        assert!(
            !reported.to_string_lossy().contains(mount.token().expose()),
            "返回值里不能夹带令牌"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// 判据: **停止即释放** —— 消费掉挂载之后同一个地址连不上。
    #[test]
    fn stop_releases_the_listener() {
        let mount = mount();
        let address = mount.address();
        assert!(
            TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_ok(),
            "停止之前必须连得上（否则这条判据证明不了任何事）"
        );
        mount.stop();
        assert!(
            TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_err(),
            "停止之后同一个地址必须连不上（监听 socket 真的 close 了）"
        );
    }

    /// 连接探测的超时（判据用；与生产路径无关）。
    const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
}
