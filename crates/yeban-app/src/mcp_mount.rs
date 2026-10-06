//! **形态 A 的运行态挂载**（`[ROAD-M4-001]`）：`yeban-app` 进程内托管领域 MCP
//! （`yeban-mcp`）的**环回 HTTP JSON-RPC 控制面**。
//!
//! 在这条线之前，仓库里有两件彼此对不上的事实：
//!
//! 1. 传输层是真的 —— `yeban_mcp::transport::http::HttpServer::bind_loopback`
//!    （绑 `127.0.0.1:0` 之后**回读 `local_addr()` 断言是环回**）+ `BearerToken`；
//! 2. 但 `yeban-app` **不依赖** `yeban-mcp`（`grep -n "yeban-mcp" crates/yeban-app/Cargo.toml`
//!    命中 0）⇒「app 进程内嵌入」这句话**没有依赖边支撑**，运行期挂载无从谈起。
//!
//! 本模块补的就是第 2 件：把服务循环**真的**跑在 app 进程的一个工作线程里。
//!
//! ## 两道开关（缺一不可，这是 `[MUST-GATE-009]` 的落点）
//!
//! | 层 | 开关 | 默认 |
//! | :--- | :--- | :--- |
//! | 编译期 | `--features in-process-mcp`（它不在 `default` 里，见 `Cargo.toml`） | **关** |
//! | 运行期 | `--enable-mcp-http`（[`CLI_SWITCH`]）/ `YEBAN_MCP_HTTP=1`（[`ENV_SWITCH`]） | **关** |
//!
//! 两道是**刻意冗余**的：编译进来了不等于要在生产里开，运行期打开了也不等于这个二进制里
//! 真的有那段代码。运行期的判定走 [`plan`]，它**复用** `yeban-mcp` 的
//! `transport::plan_http_startup` —— 不在这里写第二套判定。
//!
//! ## 安全边界（一条都不许放松）
//!
//! - **只绑环回**：地址由 `HttpServer::bind_loopback` 决定（`127.0.0.1:0`），本模块
//!   **不**接受任何地址参数 ⇒ 没有"改成 0.0.0.0"的入口；
//! - **端口 0**：由系统动态分配，绝不抢固定端口；
//! - **必须带 Bearer Token**：256-bit 高熵令牌由 [`BearerToken::generate`] 生成，
//!   鉴权判定在 `yeban-mcp` 的 `security` 模块里（本模块不碰判定）；
//! - **令牌不进日志**：`BearerToken` 的 `Debug` 是脱敏的；要交给外部 Agent 时用
//!   [`publish_token`] 落到 `~/.yeban/session.token`（`0600`），只报告**路径**；
//! - **`ui:inject` 仍然硬禁**：分发器一律以 [`RunMode::Production`] 构造，
//!   生产模式下 `ui:inject` 由 `security::authorize` 硬拒（本模块无法放松它）。
//!
//! ## 会话：**只读**打开的那份工程
//!
//! [`start_for_project`] 把调用方交进来的 `YebanProjectV1` 经 `Domain::open_in_memory`
//! 注入控制面会话，并且**固定 `read_only = true`**。为什么只读：
//!
//! - app 侧**没有**把这份投影与界面共享同一个可变实例（那是 `ROAD-M4-008` 的缺口），
//!   一个可写的**影子副本**会让 Agent 以为自己改了用户的工程；
//! - 只读还意味着控制面**不会**成为同一工程文件的第二个写者 ——
//!   `[MUST-GATE-008]` / `ROAD-M0-007` 的「跨形态互斥」缺口**不因本帖而被扩大**。
//!
//! 想真的改工程，走的仍是已完成的形态 B（`yeban-mcp` stdio CLI + `.yeban.lock`）。
//!
//! ## 明确**没做**（不是"忘了"）
//!
//! - **没有**把控制面会话与界面投影接成同一实例（`ROAD-M4-008` 的域侧缺口）；
//! - **没有**取 `.yeban.lock` ⇒ 跨形态互斥未被验证（`ROAD-M0-007` 的缺口）；
//! - **没有**停机信号的传输层原语：`HttpServer` 只有阻塞的 `serve_once` / `serve_forever`。
//!   本模块用一个 stop 标志 + 一次**环回唤醒连接**让阻塞中的 `accept` 返回（见 [`InProcessMcp::stop`]）。
//!   代价如实登记：一个连上却不发请求的慢客户端会推迟停机（传输层文件头已声明"不做慢速攻击防护"）。
//!
//! ## 判据
//!
//! `crates/yeban-app/tests/in_process_mcp.rs`（只在 `--features in-process-mcp` 下存在）：
//! 开关关着 ⇒ 什么都不建；开关打开 ⇒ 真环回 socket 上 `tools/list` / `yeban_query_project`
//! 拿到 200 与真实载荷；**无令牌 ⇒ 401**；**停机之后同一地址连不上**。
//! 运行：
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-app --features in-process-mcp --tests
//! ```

use std::fmt;
use std::io::Write as _;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use yeban_mcp::Dispatcher;
use yeban_mcp::security::{BearerToken, RunMode, ScopeSet, TokenFile};
use yeban_mcp::transport::http::{HttpError, HttpServer, MCP_PATH};
use yeban_mcp::transport::{HttpStartup, TransportError, plan_http_startup};
use yeban_model::YebanProjectV1;

// ---------------------------------------------------------------------------
// 两道开关（第二道）与时间常量
// ---------------------------------------------------------------------------

/// 运行期开关的**命令行**形态。
///
/// 字面值与权威定义（`yeban-mcp` 的 `transport::ENABLE_HTTP_FLAG`，形态 A 与形态 B
/// 共用的那个词）**逐字节相同**这件事由判据钉住：
/// `crates/yeban-app/tests/in_process_mcp.rs` 断言
/// `CLI_SWITCH == yeban_mcp::transport::ENABLE_HTTP_FLAG`。
/// 字面值本体住在 [`crate::cli::MCP_HTTP_SWITCH`] —— 默认构建里没有 `yeban-mcp`，
/// 而用法文本在默认构建里也要打得出来。
pub const CLI_SWITCH: &str = crate::cli::MCP_HTTP_SWITCH;

/// 运行期开关的**环境变量**形态；取 `1` / `true` / `yes` / `on`（大小写不敏感、两侧空白忽略）。
pub const ENV_SWITCH: &str = crate::cli::MCP_HTTP_ENV;

/// 唤醒阻塞中的 `accept` 用的报文体：一次合法的 `POST /mcp`（**不带**令牌）。
///
/// 它只会拿到一个 `401`，本模块**不读**这个响应 —— 目的仅仅是让 `accept` 返回。
/// 无令牌是刻意的：唤醒连接不能顺便获得任何能力。
const WAKE_REQUEST: &str =
    "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// 唤醒连接的连接超时。
const WAKE_TIMEOUT: Duration = Duration::from_secs(1);

/// `accept` 出错后的退避：避免"accept 坏掉"变成把机器一个核打满的忙等。
const ACCEPT_RETRY_BACKOFF: Duration = Duration::from_millis(10);

/// 停机的上限：超过它 [`InProcessMcp::stop`] **响亮地失败**，而不是让调用方永远等下去。
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// 纯函数：运行期开关的判定
// ---------------------------------------------------------------------------

/// **纯函数**：命令行开关与环境变量合成一个布尔判定。
///
/// 语义（表就是判据）：
///
/// | `cli_flag` | `env_value` | 结果 |
/// | :--- | :--- | :--- |
/// | `false` | `None` / 非真值串 | `false`（**默认**） |
/// | `false` | `"1"` / `"true"` / `"yes"` / `"on"`（任意大小写、可带空白） | `true` |
/// | `true` | 任意 | `true` |
///
/// 没有"反向覆盖"：本模块的默认态就是关，因此不存在需要 `=0` 去关的东西。
#[must_use]
pub fn switch_requested(cli_flag: bool, env_value: Option<&str>) -> bool {
    if cli_flag {
        return true;
    }
    env_value.is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// 从**真实环境**读第二道开关（`main.rs` 用；判据用 [`switch_requested`] 直接喂表）。
#[must_use]
pub fn switch_from_env() -> bool {
    switch_requested(false, std::env::var(ENV_SWITCH).ok().as_deref())
}

/// 运行期启动决策：**复用** `yeban-mcp` 的 `transport::plan_http_startup`。
///
/// 本模块特意不自己判"feature 有没有编译进来" —— 那是传输层 crate 的判定，抄一份就是
/// 第二个事实源。
///
/// # Errors
///
/// 运行期要求开、但这次构建没有把 `yeban-mcp/mcp-http` 编译进来。
/// （开 `in-process-mcp` 就一定会带上它，因此这条在实践中不可达；保留它是为了**不吞掉**
/// 底层判定。）
pub fn plan(requested: bool) -> Result<HttpStartup, MountError> {
    plan_http_startup(requested).map_err(MountError::Transport)
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// 挂载过程中的失败（刻意手写，不引 `thiserror` —— app 的默认依赖图不因本帖变大）。
#[derive(Debug)]
pub enum MountError {
    /// 传输层的启动决策失败（见 [`plan`]）。
    Transport(TransportError),
    /// 绑定 / 取址失败（含"回读地址不是环回"这一条红线断言）。
    Http(HttpError),
    /// 工程无法注入控制面会话（版本门 / 结构校验不通过）。
    Domain(String),
    /// 工作线程创建失败。
    Thread(std::io::Error),
    /// 停机超时：工作线程在 [`STOP_TIMEOUT`] 内没有退出。
    StopTimeout(Duration),
    /// 工作线程 panic。
    WorkerPanicked,
}

impl fmt::Display for MountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "传输层启动决策失败: {error}"),
            Self::Http(error) => write!(formatter, "环回绑定失败: {error}"),
            Self::Domain(detail) => {
                write!(formatter, "工程无法注入控制面会话: {detail}")
            }
            Self::Thread(error) => write!(formatter, "控制面工作线程创建失败: {error}"),
            Self::StopTimeout(limit) => {
                write!(formatter, "控制面在 {limit:?} 内没有停机")
            }
            Self::WorkerPanicked => formatter.write_str("控制面工作线程 panic"),
        }
    }
}

impl std::error::Error for MountError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Http(error) => Some(error),
            Self::Thread(error) => Some(error),
            Self::Domain(_) | Self::StopTimeout(_) | Self::WorkerPanicked => None,
        }
    }
}

impl From<HttpError> for MountError {
    fn from(error: HttpError) -> Self {
        Self::Http(error)
    }
}

// ---------------------------------------------------------------------------
// 运行态挂载
// ---------------------------------------------------------------------------

/// 一个**正在跑**的进程内控制面。
///
/// 它持有三样东西：监听 socket、工作线程句柄、停机标志。**停止 = 消费掉它**
/// （[`Self::stop`] 或 `Drop`）：停机之后监听 socket 被真的关掉，因此"没有东西在监听"
/// 是可被 `TcpStream::connect` 观察到的事实，而不只是一句声明。
#[derive(Debug)]
pub struct InProcessMcp {
    /// 监听 socket（`None` = 已经停机、socket 已关闭）。
    server: Option<Arc<HttpServer>>,
    /// 停机标志（工作线程每轮 accept 前后各看一次）。
    stop: Arc<AtomicBool>,
    /// 工作线程句柄（`None` = 已经回收）。
    worker: Option<JoinHandle<()>>,
    /// 绑定后**回读**到的环回地址（启动时确定，停机后仍可读，便于报错时点名）。
    address: SocketAddr,
    /// 期望的会话令牌（启动时从分发器取出；`Debug` 是脱敏的）。
    token: BearerToken,
}

impl InProcessMcp {
    /// 真的绑环回、建线程、开始服务。
    ///
    /// 调用方负责两件事：把**令牌**交给外部 Agent（见 [`publish_token`]），以及在该停的
    /// 时候 [`Self::stop`]。
    ///
    /// # Errors
    ///
    /// 绑定失败、回读地址不是环回、或工作线程创建失败。
    pub fn start(dispatcher: Dispatcher) -> Result<Self, MountError> {
        let server = Arc::new(HttpServer::bind_loopback(dispatcher)?);
        let address = server.local_addr()?;
        let token = server.token();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let server = Arc::clone(&server);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("yeban-mcp-http".to_owned())
                .spawn(move || serve_loop(&server, &stop))
                .map_err(MountError::Thread)?
        };
        Ok(Self {
            server: Some(server),
            stop,
            worker: Some(worker),
            address,
            token,
        })
    }

    /// **开关关着就什么都不建**：返回 `Ok(None)`，连分发器都不构造
    /// （没有 socket、没有令牌、没有线程、没有工程克隆）。
    ///
    /// 工程固定以**只读**方式注入控制面会话（理由见模块文档的"会话"一节）——
    /// 这里**没有** `read_only` 参数，因为"要不要交出第二个写者"不该由调用点随手决定。
    ///
    /// # Errors
    ///
    /// 决策失败、工程无法注入、或绑定/线程失败。
    pub fn start_for_project(
        requested: bool,
        project: YebanProjectV1,
        path: impl Into<PathBuf>,
    ) -> Result<Option<Self>, MountError> {
        match plan(requested)? {
            HttpStartup::Disabled => Ok(None),
            HttpStartup::Enabled => {
                let dispatcher = session_dispatcher(&project, &path.into())?;
                Self::start(dispatcher).map(Some)
            }
        }
    }

    /// 实际监听地址（**环回** + 系统动态分配的端口）。
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// 外部 Agent 要用的端点 URL（`http://127.0.0.1:<port>/mcp`）。
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("http://{}{MCP_PATH}", self.address)
    }

    /// 期望的会话令牌。**不要**把它写进日志（`Debug` 已脱敏，`expose()` 不会）。
    #[must_use]
    pub fn token(&self) -> &BearerToken {
        &self.token
    }

    /// 停机：置标志 → 唤醒阻塞中的 `accept` → **有界地**等工作线程退出 → 关掉监听 socket。
    ///
    /// 消费 `self` 是刻意的：停止之后"还在监听"在类型上就不可能被误用。
    ///
    /// # Errors
    ///
    /// 停机超时（[`MountError::StopTimeout`]）或工作线程 panic。
    pub fn stop(mut self) -> Result<(), MountError> {
        self.shutdown()
    }

    /// 停机的唯一实现（`stop` 与 `Drop` 共用；幂等）。
    fn shutdown(&mut self) -> Result<(), MountError> {
        self.stop.store(true, Ordering::SeqCst);
        self.wake();
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + STOP_TIMEOUT;
            while !worker.is_finished() {
                if Instant::now() >= deadline {
                    // 把句柄放回去，让 `Drop` 还能再试一次（而不是把它丢成一个泄漏的线程）。
                    self.worker = Some(worker);
                    return Err(MountError::StopTimeout(STOP_TIMEOUT));
                }
                thread::sleep(ACCEPT_RETRY_BACKOFF);
            }
            worker.join().map_err(|_| MountError::WorkerPanicked)?;
        }
        // 关掉监听 socket —— "停止"必须意味着"没有任何东西在监听", 而不只是"没人 accept"。
        // 工作线程已经退出, 它的 `Arc` 克隆已经释放, 因此这里一 drop 就是真的 `close()`。
        if let Some(server) = self.server.take() {
            drop(server);
        }
        Ok(())
    }

    /// 用一次**环回连接**把阻塞在 `accept` 里的那一次 `serve_once` 叫回来。
    ///
    /// 失败被刻意忽略：唤醒失败时 `shutdown` 会在 `STOP_TIMEOUT` 内响亮地失败，
    /// 而不是静默地留下一个"以为停了"的控制面。
    fn wake(&self) {
        let Some(server) = self.server.as_ref() else {
            return;
        };
        let Ok(address) = server.local_addr() else {
            return;
        };
        let Ok(mut stream) = TcpStream::connect_timeout(&address, WAKE_TIMEOUT) else {
            return;
        };
        let _ = stream.write_all(WAKE_REQUEST.as_bytes());
        let _ = stream.flush();
        let _ = stream.shutdown(Shutdown::Both);
    }
}

impl Drop for InProcessMcp {
    fn drop(&mut self) {
        // 忘了 `stop()` 也不会留下监听的东西 —— 这是线程/套接字安全的最后一道。
        let _ = self.shutdown();
    }
}

/// 工作线程：一次一个连接地服务，直到 stop 标志被看见。
///
/// 为什么用 `serve_once` 而不是 `serve_forever`：后者没有停机入口（它 `loop { accept }`），
/// 而本模块必须能证明"停止之后什么都不在监听"。
fn serve_loop(server: &HttpServer, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        if server.serve_once().is_err() {
            // 连接级 I/O 失败不是停机理由；`accept` 真坏了会立刻再进这里，因此退避一下。
            thread::sleep(ACCEPT_RETRY_BACKOFF);
        }
    }
}

/// 构造一个**生产模式**的分发器，并把工程注入它的**只读**会话。
///
/// 高熵令牌在**这里**生成（[`BearerToken::generate`]，256 bit）—— 没有第二个生成点。
fn session_dispatcher(project: &YebanProjectV1, path: &Path) -> Result<Dispatcher, MountError> {
    let token = BearerToken::generate().token;
    // scope 给全：`ui:inject` 的硬禁**不在这里**判，而在 `security::authorize`
    // 按 `RunMode::Production` 判 —— 少给 scope 不等于更安全，只会让别的工具也失灵。
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher
        .domain_mut()
        .open_in_memory(path.to_path_buf(), project.clone(), true)
        .map_err(|fault| MountError::Domain(format!("{fault:?}")))?;
    Ok(dispatcher)
}

/// 把会话令牌以 `0600` 落到 `~/.yeban/session.token`（`[ARCH-SEC-002]` 的既有落点），
/// 返回**路径**。
///
/// 为什么不打印令牌本身：令牌进日志就等于进 CI 制品与终端回滚缓冲。落盘 + 只报路径
/// 是 `yeban-mcp` 形态 B 已经在用的做法，这里**复用同一个** `TokenFile`。
///
/// # Errors
///
/// 定位不到家目录、非 Unix 平台（`0600` 无法校验 ⇒ 明确拒绝而不是静默放过）、或 I/O 失败。
pub fn publish_token(token: &BearerToken) -> Result<PathBuf, MountError> {
    let file =
        TokenFile::at_default_path().map_err(|error| MountError::Domain(error.to_string()))?;
    file.save(token)
        .map_err(|error| MountError::Domain(error.to_string()))?;
    Ok(file.path().to_path_buf())
}
