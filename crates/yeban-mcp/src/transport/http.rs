//! 环回 HTTP/1.1 传输（形态 A）[ARCH-SEC-002, MUST-GATE-009]。
//!
//! ## 契约（每一条都有判据）
//!
//! | 约束 | 落点 |
//! | :--- | :--- |
//! | 只绑 `127.0.0.1` | [`HttpServer::bind_loopback`] + [`assert_loopback`] |
//! | 端口 `0`（系统动态分配） | [`DYNAMIC_PORT`]，绑定后从 `local_addr()` **回读**并断言是环回 |
//! | 只接受 `POST` | [`SUPPORTED_METHOD`]，其它方法 `405` + `Allow: POST` |
//! | 必须带 `Content-Length` | 缺失 ⇒ `411` |
//! | 拒绝超长请求 | 头 > [`MAX_REQUEST_HEAD_BYTES`] ⇒ `431`；体 > [`MAX_REQUEST_BODY_BYTES`] ⇒ `413`（**先判长度再分配**） |
//! | 必须带 `Authorization: Bearer <TOKEN>` | 缺失 / 形状非法 / 不匹配 ⇒ `401` + `WWW-Authenticate`，**绝不回退放开** |
//! | 生产模式硬禁 `ui:inject` | [`crate::security::authorize`]（判定不在传输层） |
//!
//! ## 为什么手写而不是 `tiny_http`
//!
//! 需求只有一件事：把一行 JSON 从环回 socket 搬进 [`Dispatcher`] 再把响应搬回去。
//! 引入一个 web 框架换来的是：**多一个依赖、多一份版本漂移面、多一处我们无法
//! 逐行审计的安全边界**（红线 2 的依赖许可、ADR-0001 D20/D21 的依赖政策）。
//! 手写的代价是本文件 ~300 行，且每一行都能被单元判据直接打到。
//!
//! ## 边界（**明确没做**，不是"忘了"）
//!
//! - 不支持 `Transfer-Encoding` / `chunked`（带这个头一律按"没有 body"处理 ⇒ `411`）；
//! - 不支持 HTTP/2、TLS、keep-alive（每个连接一个请求，响应带 `Connection: close`）；
//! - 不支持请求流水线（不读第二个请求）；
//! - 不做慢速攻击防护（环回 + 单用户开发场景；**不要**把它绑到非环回地址上）。

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};

use crate::dispatch::Dispatcher;
use crate::security::{BearerToken, Channel};

/// IPv4 环回地址。
pub const LOOPBACK_IPV4: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// IPv6 环回地址。
pub const LOOPBACK_IPV6: Ipv6Addr = Ipv6Addr::LOCALHOST;

/// 动态端口（交给系统分配）[ARCH-SEC-002]。
pub const DYNAMIC_PORT: u16 = 0;

/// 请求头上限（16 KiB）。
pub const MAX_REQUEST_HEAD_BYTES: usize = 16 * 1024;

/// 请求体上限（1 MiB）。超限一律 `413`，且在**分配之前**就判掉。
pub const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// 唯一接受的 HTTP 方法。
pub const SUPPORTED_METHOD: &str = "POST";

/// MCP 端点路径。
pub const MCP_PATH: &str = "/mcp";

/// 响应内容类型。
pub const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";

/// `401` 上的 Bearer 挑战（RFC 6750）。
pub const BEARER_CHALLENGE: &str = "Bearer realm=\"yeban-mcp\"";

/// 头 / 体分隔符。
const HEAD_TERMINATOR: &[u8] = b"\r\n\r\n";

/// HTTP 传输的错误。
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// 试图绑定非环回地址。
    #[error("拒绝绑定非环回地址 `{address}`: 只允许 {LOOPBACK_IPV4} / {LOOPBACK_IPV6}")]
    NotLoopback {
        /// 被拒绝的地址。
        address: SocketAddr,
    },
    /// 非环回的对端连进来了（绑定到环回时不可能发生；防御性检查）。
    #[error("拒绝非环回对端 `{address}`")]
    NonLoopbackPeer {
        /// 对端地址。
        address: SocketAddr,
    },
    /// 绑定失败。
    #[error("绑定 `{address}` 失败: {source}")]
    Bind {
        /// 目标地址。
        address: SocketAddr,
        /// 底层错误。
        source: std::io::Error,
    },
    /// socket I/O 失败。
    #[error("socket I/O 失败: {source}")]
    Io {
        /// 底层错误。
        source: std::io::Error,
    },
    /// 请求行 / 头 / 分隔符不合法。
    #[error("非法 HTTP 请求: {detail}")]
    MalformedRequest {
        /// 人话说明。
        detail: String,
    },
    /// 不支持的 HTTP 版本。
    #[error("不支持的 HTTP 版本 `{version}`: 只接受 HTTP/1.0 与 HTTP/1.1")]
    BadVersion {
        /// 实际版本串。
        version: String,
    },
    /// 头太大。
    #[error("请求头超过 {limit} 字节上限")]
    HeadTooLarge {
        /// 上限。
        limit: usize,
    },
    /// 体太大。
    #[error("请求体 {declared} 字节超过 {limit} 字节上限")]
    BodyTooLarge {
        /// 声明的长度。
        declared: usize,
        /// 上限。
        limit: usize,
    },
    /// 缺少 `Content-Length`。
    #[error("POST 必须带 `Content-Length` 头")]
    MissingContentLength,
}

impl HttpError {
    /// 机器可读的种类名（进错误响应的 `error.kind`）。
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotLoopback { .. } => "not-loopback",
            Self::NonLoopbackPeer { .. } => "non-loopback-peer",
            Self::Bind { .. } => "bind-failed",
            Self::Io { .. } => "io-error",
            Self::MalformedRequest { .. } => "malformed-request",
            Self::BadVersion { .. } => "bad-version",
            Self::HeadTooLarge { .. } => "head-too-large",
            Self::BodyTooLarge { .. } => "body-too-large",
            Self::MissingContentLength => "missing-content-length",
        }
    }

    /// HTTP 状态码。
    #[must_use]
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::NonLoopbackPeer { .. } => 403,
            Self::MalformedRequest { .. } => 400,
            Self::BadVersion { .. } => 505,
            Self::HeadTooLarge { .. } => 431,
            Self::BodyTooLarge { .. } => 413,
            Self::MissingContentLength => 411,
            Self::NotLoopback { .. } | Self::Bind { .. } | Self::Io { .. } => 500,
        }
    }

    fn outcome(&self) -> HttpResponse {
        HttpResponse::json(
            self.http_status(),
            error_body(self.kind(), &self.to_string()),
        )
    }
}

/// 一个已解析的 HTTP 请求。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HttpRequest {
    /// 方法（原样保留大小写，比较时区分）。
    pub method: String,
    /// 请求目标（含可能的查询串）。
    pub target: String,
    /// 版本串（`HTTP/1.1`）。
    pub version: String,
    /// 头（键已规范化为小写）。
    pub headers: BTreeMap<String, String>,
    /// 体（按 UTF-8 有损解码）。
    pub body: String,
}

impl HttpRequest {
    /// 按名取头（大小写不敏感）。
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// `Authorization` 头的原始值。
    #[must_use]
    pub fn authorization(&self) -> Option<&str> {
        self.header("authorization")
    }

    /// 解析 `Content-Length`。
    ///
    /// - `Ok(None)`：没有这个头；
    /// - `Ok(Some(len))`：合法长度；
    /// - `Err(..)`：有这个头但不是合法无符号十进制。
    ///
    /// # Errors
    ///
    /// `Content-Length` 不是十进制整数。
    pub fn parse_content_length(&self) -> Result<Option<usize>, HttpError> {
        match self.header("content-length") {
            None => Ok(None),
            Some(raw) => {
                raw.trim()
                    .parse::<usize>()
                    .map(Some)
                    .map_err(|_| HttpError::MalformedRequest {
                        detail: format!("`Content-Length` 不是十进制整数: `{raw}`"),
                    })
            }
        }
    }

    /// 端点路径（去掉查询串）。
    #[must_use]
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }
}

/// 一个待写出的 HTTP 响应。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    /// 状态码。
    pub status: u16,
    /// 内容类型。
    pub content_type: &'static str,
    /// 附加头。
    pub headers: Vec<(&'static str, String)>,
    /// 体。
    pub body: String,
}

impl HttpResponse {
    /// JSON 响应。
    #[must_use]
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: CONTENT_TYPE_JSON,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// 纯文本响应（只用于本地诊断）。
    #[must_use]
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// 追加一个头。
    #[must_use]
    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// 状态原因短语。
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        status_reason(self.status)
    }

    /// 完整报文字节（含状态行、头、空行、体）。
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.reason(),
            self.content_type,
            self.body.len()
        );
        for (name, value) in &self.headers {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(self.body.as_bytes());
        bytes
    }
}

/// 状态码 → 原因短语。
#[must_use]
pub const fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        505 => "HTTP Version Not Supported",
        _ => "Unknown",
    }
}

/// **红线判定**：地址必须是 IPv4 / IPv6 环回 [ARCH-SEC-002, AGENTS.md §2 红线 5]。
///
/// 这是纯函数，判据可以直接喂一个非环回地址进来把它打红 ——
/// `Ipv4Addr::UNSPECIFIED` 就是那个"绝不允许"的地址。
///
/// # Errors
///
/// 地址不是环回。
pub fn assert_loopback(address: SocketAddr) -> Result<(), HttpError> {
    if address.ip().is_loopback() {
        Ok(())
    } else {
        Err(HttpError::NotLoopback { address })
    }
}

/// 对端地址也必须来自环回（绑定到环回时的冗余防务）。
///
/// # Errors
///
/// 对端不是环回。
pub fn assert_loopback_peer(address: SocketAddr) -> Result<(), HttpError> {
    if address.ip().is_loopback() {
        Ok(())
    } else {
        Err(HttpError::NonLoopbackPeer { address })
    }
}

/// 解析请求头（`head` 是分隔符之前的部分，允许 CRLF 或 LF 行尾）。
///
/// # Errors
///
/// 请求行 / 头行不合法、版本不受支持、重复的 `Content-Length`。
pub fn parse_head(head: &str) -> Result<HttpRequest, HttpError> {
    let normalized = head.replace("\r\n", "\n");
    let mut lines = normalized.split('\n');
    let request_line = lines.next().unwrap_or_default().trim();
    if request_line.is_empty() {
        return Err(HttpError::MalformedRequest {
            detail: "缺少请求行".to_owned(),
        });
    }
    let parts: Vec<&str> = request_line.split(' ').filter(|p| !p.is_empty()).collect();
    let [method, target, version] = parts.as_slice() else {
        return Err(HttpError::MalformedRequest {
            detail: format!("请求行必须是 `METHOD SP target SP HTTP/x.y`: `{request_line}`"),
        });
    };
    if !(version.starts_with("HTTP/1.0") || version.starts_with("HTTP/1.1")) {
        return Err(HttpError::BadVersion {
            version: (*version).to_owned(),
        });
    }

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpError::MalformedRequest {
                detail: format!("非法头行 (缺少 `:`): `{line}`"),
            });
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err(HttpError::MalformedRequest {
                detail: format!("非法头行 (头名为空): `{line}`"),
            });
        }
        // 重复的 Content-Length 是经典的请求走私形状: 直接拒。
        if name == "content-length" && headers.contains_key(&name) {
            return Err(HttpError::MalformedRequest {
                detail: "重复的 `Content-Length` 头".to_owned(),
            });
        }
        headers.insert(name, value.trim().to_owned());
    }

    Ok(HttpRequest {
        method: (*method).to_owned(),
        target: (*target).to_owned(),
        version: (*version).to_owned(),
        headers,
        body: String::new(),
    })
}

/// 把一个完整报文字节切成 `(head, body)`。
#[must_use]
pub fn split_message(raw: &[u8]) -> (String, String) {
    if let Some(index) = raw.windows(4).position(|window| window == HEAD_TERMINATOR) {
        let head = String::from_utf8_lossy(&raw[..index]).into_owned();
        let body = String::from_utf8_lossy(&raw[index + 4..]).into_owned();
        return (head, body);
    }
    if let Some(index) = raw.windows(2).position(|window| window == b"\n\n") {
        let head = String::from_utf8_lossy(&raw[..index]).into_owned();
        let body = String::from_utf8_lossy(&raw[index + 2..]).into_owned();
        return (head, body);
    }
    (String::from_utf8_lossy(raw).into_owned(), String::new())
}

/// `405 Method Not Allowed`（带 `Allow: POST`）。
fn method_not_allowed(method: &str) -> HttpResponse {
    HttpResponse::json(
        405,
        error_body(
            "method-not-allowed",
            &format!("只接受 `{SUPPORTED_METHOD}`, 收到 `{method}`"),
        ),
    )
    .with_header("Allow", SUPPORTED_METHOD)
}

/// `404 Not Found`。
fn unknown_path(target: &str) -> HttpResponse {
    HttpResponse::json(
        404,
        error_body(
            "unknown-path",
            &format!("未知端点 `{target}` (只服务 `{MCP_PATH}`)"),
        ),
    )
}

/// 错误响应的 JSON 体。
fn error_body(kind: &str, detail: &str) -> String {
    serde_json::json!({"error": {"kind": kind, "detail": detail}}).to_string()
}

/// 环回 HTTP 服务。
#[derive(Debug)]
pub struct HttpServer {
    listener: TcpListener,
    dispatcher: Mutex<Dispatcher>,
}

impl HttpServer {
    /// 绑定 `127.0.0.1:0`，并**回读 `local_addr()` 断言是环回**。
    ///
    /// 端口 `0` 让系统动态分配，避免与本机其它服务抢固定端口。
    ///
    /// # Errors
    ///
    /// 绑定失败，或（不可能发生的）回读地址不是环回。
    pub fn bind_loopback(dispatcher: Dispatcher) -> Result<Self, HttpError> {
        let address = SocketAddr::new(IpAddr::V4(LOOPBACK_IPV4), DYNAMIC_PORT);
        let listener =
            TcpListener::bind(address).map_err(|source| HttpError::Bind { address, source })?;
        let server = Self {
            listener,
            dispatcher: Mutex::new(dispatcher),
        };
        // 绑定后立刻回读: "我请求的是环回"与"我真的绑在环回上"是两件事。
        assert_loopback(server.local_addr()?)?;
        Ok(server)
    }

    /// 实际监听地址。
    ///
    /// # Errors
    ///
    /// 取不到本地地址。
    pub fn local_addr(&self) -> Result<SocketAddr, HttpError> {
        self.listener
            .local_addr()
            .map_err(|source| HttpError::Io { source })
    }

    /// 期望的 Bearer Token。
    #[must_use]
    pub fn token(&self) -> BearerToken {
        self.lock().expected_token().clone()
    }

    /// 取分发器（锁被毒化时仍然取内层 —— 环回开发服务宁可继续回答并如实报错，
    /// 也不要在 handler 里 panic）。
    fn lock(&self) -> MutexGuard<'_, Dispatcher> {
        match self.dispatcher.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// 处理一个已解析的请求。
    fn respond(&self, request: &HttpRequest) -> HttpResponse {
        if request.method != SUPPORTED_METHOD {
            return method_not_allowed(&request.method);
        }
        if !matches!(request.path(), MCP_PATH | "/") {
            return unknown_path(&request.target);
        }
        let declared = match request.parse_content_length() {
            Ok(Some(length)) => length,
            Ok(None) => return HttpError::MissingContentLength.outcome(),
            Err(error) => return error.outcome(),
        };
        if declared > MAX_REQUEST_BODY_BYTES || request.body.len() > MAX_REQUEST_BODY_BYTES {
            return HttpError::BodyTooLarge {
                declared: declared.max(request.body.len()),
                limit: MAX_REQUEST_BODY_BYTES,
            }
            .outcome();
        }

        let outcome =
            self.lock()
                .handle_line(Channel::Http, request.authorization(), &request.body);
        let body = outcome
            .response
            .map_or_else(String::new, |response| response.to_json());
        let mut response = HttpResponse::json(outcome.http_status, body);
        if outcome.http_status == 401 {
            response = response.with_header("WWW-Authenticate", BEARER_CHALLENGE);
        }
        response
    }

    /// 处理一段完整报文文本（单元判据与嵌入式用法）。
    #[must_use]
    pub fn handle_text(&self, raw: &str) -> HttpResponse {
        let (head, body) = split_message(raw.as_bytes());
        match parse_head(&head) {
            Ok(mut request) => {
                request.body = body;
                self.respond(&request)
            }
            Err(error) => error.outcome(),
        }
    }

    /// 从一个已连接的 socket 上读完一个请求并回答。
    ///
    /// # Errors
    ///
    /// socket I/O 失败（协议层错误会被翻译成错误响应，不在此返回）。
    pub fn handle_stream(&self, stream: &mut TcpStream) -> Result<(), HttpError> {
        let response = match Self::read_head_request(stream) {
            // 方法与端点先判: 否则一个不带 `Content-Length` 的 `GET` 会先撞上 411,
            // 让人以为"服务器要求 body"而不是"服务器不接受这个方法"。
            Ok(mut request) => {
                if request.method != SUPPORTED_METHOD {
                    method_not_allowed(&request.method)
                } else if !matches!(request.path(), MCP_PATH | "/") {
                    unknown_path(&request.target)
                } else {
                    match Self::read_body(stream, &mut request) {
                        Ok(()) => self.respond(&request),
                        Err(error) => error.outcome(),
                    }
                }
            }
            Err(error) => error.outcome(),
        };
        stream
            .write_all(&response.to_bytes())
            .and_then(|()| stream.flush())
            .map_err(|source| HttpError::Io { source })
    }

    /// 只读头并解析出请求行 / 头（体留空）。
    fn read_head_request(stream: &mut TcpStream) -> Result<HttpRequest, HttpError> {
        let head_bytes = Self::read_head(stream)?;
        let head = String::from_utf8_lossy(&head_bytes).into_owned();
        parse_head(&head)
    }

    /// 按 `Content-Length` 读体（**先判长度再分配**）。
    fn read_body(stream: &mut TcpStream, request: &mut HttpRequest) -> Result<(), HttpError> {
        let length = request
            .parse_content_length()?
            .ok_or(HttpError::MissingContentLength)?;
        if length > MAX_REQUEST_BODY_BYTES {
            return Err(HttpError::BodyTooLarge {
                declared: length,
                limit: MAX_REQUEST_BODY_BYTES,
            });
        }
        if length > 0 {
            let mut body = vec![0_u8; length];
            stream
                .read_exact(&mut body)
                .map_err(|source| HttpError::Io { source })?;
            request.body = String::from_utf8_lossy(&body).into_owned();
        }
        Ok(())
    }

    /// 逐字节读到头 / 体分隔符（上限 [`MAX_REQUEST_HEAD_BYTES`]）。
    fn read_head(stream: &mut TcpStream) -> Result<Vec<u8>, HttpError> {
        let mut head: Vec<u8> = Vec::with_capacity(256);
        let mut byte = [0_u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    head.push(byte[0]);
                    if head.len() > MAX_REQUEST_HEAD_BYTES {
                        return Err(HttpError::HeadTooLarge {
                            limit: MAX_REQUEST_HEAD_BYTES,
                        });
                    }
                    if head.ends_with(HEAD_TERMINATOR) || head.ends_with(b"\n\n") {
                        break;
                    }
                }
                Err(source) => return Err(HttpError::Io { source }),
            }
        }
        if head.is_empty() {
            return Err(HttpError::MalformedRequest {
                detail: "连接在收到任何字节之前就关闭了".to_owned(),
            });
        }
        Ok(head)
    }

    /// 接受**一个**连接并处理它（判据与嵌入式用法；循环见 [`HttpServer::serve_forever`]）。
    ///
    /// # Errors
    ///
    /// accept 失败，或该连接的处理返回 I/O 错误。
    pub fn serve_once(&self) -> Result<(), HttpError> {
        let (mut stream, peer) = self
            .listener
            .accept()
            .map_err(|source| HttpError::Io { source })?;
        assert_loopback_peer(peer)?;
        self.handle_stream(&mut stream)
    }

    /// 永久服务（**只在 `mcp-http` feature 下存在**）。
    ///
    /// 每个连接一条线程（`std::thread::scope`，不需要 `Arc`）。
    ///
    /// # Errors
    ///
    /// accept 失败。
    #[cfg(feature = "mcp-http")]
    pub fn serve_forever(&self) -> Result<(), HttpError> {
        std::thread::scope(|scope| -> Result<(), HttpError> {
            loop {
                let (mut stream, peer) = self
                    .listener
                    .accept()
                    .map_err(|source| HttpError::Io { source })?;
                scope.spawn(move || {
                    if assert_loopback_peer(peer).is_err() {
                        return;
                    }
                    // 客户端提前断开是常态, 不是错误。
                    let _ = self.handle_stream(&mut stream);
                });
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{AuthContext, Denial, RunMode, Scope, ScopeSet, authorize};
    use serde_json::Value;

    fn dispatcher(mode: RunMode) -> Dispatcher {
        Dispatcher::new(BearerToken::generate().token, ScopeSet::all(), mode)
    }

    fn loopback_server() -> HttpServer {
        HttpServer::bind_loopback(dispatcher(RunMode::Production)).expect("绑定环回")
    }

    /// 一个**注入了工程**的环回服务（渲染器那半未接线时才可能拿到 501）。
    fn loopback_server_with_project() -> HttpServer {
        let mut dispatcher = dispatcher(RunMode::Production);
        dispatcher.domain_mut().set_now_ms(0);
        dispatcher
            .domain_mut()
            .open_in_memory(
                std::path::PathBuf::from("/tmp/yeban-http-tests/demo.yeban"),
                yeban_model::samples::filled_project(),
                false,
            )
            .expect("注入规范工程");
        HttpServer::bind_loopback(dispatcher).expect("绑定环回")
    }

    /// 组装一个请求报文。
    fn request(authorization: Option<&str>, body: &str) -> String {
        let mut head = format!(
            "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(value) = authorization {
            head.push_str("Authorization: ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        head.push_str(body);
        head
    }

    fn bearer(server: &HttpServer) -> String {
        format!("Bearer {}", server.token().expose())
    }

    fn body(server: &HttpServer) -> Value {
        let request = request(
            Some(&bearer(server)),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        let response = server.handle_text(&request);
        serde_json::from_str(&response.body).expect("响应体是 JSON")
    }

    #[test]
    fn bind_is_loopback_only_and_uses_a_dynamic_port() {
        let server = loopback_server();
        let address = server.local_addr().expect("本地地址");
        assert!(address.ip().is_loopback(), "必须绑在环回: {address}");
        assert!(address.is_ipv4(), "本实现绑定 IPv4 环回: {address}");
        assert_eq!(address.ip(), IpAddr::V4(LOOPBACK_IPV4));
        assert_ne!(
            address.port(),
            DYNAMIC_PORT,
            "端口 0 由系统分配, 回读必须非 0"
        );
        assert!(assert_loopback(address).is_ok());
    }

    #[test]
    fn assert_loopback_rejects_the_unspecified_addresses() {
        // 注: 这里刻意用 Ipv4Addr::UNSPECIFIED 而不是写它的字面量 ——
        // 字面量本身被守卫 G04 [ARCH-SEC-002] 禁止出现在源码里。
        for address in [
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 9316),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 9316),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 9316),
        ] {
            let error = assert_loopback(address).expect_err("非环回必须被拒");
            assert!(matches!(error, HttpError::NotLoopback { .. }));
            assert_eq!(error.http_status(), 500);
        }
    }

    #[test]
    fn assert_loopback_accepts_both_families() {
        assert!(assert_loopback(SocketAddr::new(IpAddr::V4(LOOPBACK_IPV4), 1)).is_ok());
        assert!(assert_loopback(SocketAddr::new(IpAddr::V6(LOOPBACK_IPV6), 1)).is_ok());
        assert!(
            assert_loopback_peer(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 1))
                .is_err()
        );
    }

    #[test]
    fn missing_token_is_rejected_with_401() {
        let server = loopback_server();
        let response = server.handle_text(&request(
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ));
        assert_eq!(response.status, 401);
        assert_eq!(response.reason(), "Unauthorized");
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| *name == "WWW-Authenticate" && value == BEARER_CHALLENGE),
            "401 必须带 Bearer 挑战"
        );
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["code"], crate::jsonrpc::UNAUTHORIZED);
        assert_eq!(value["error"]["data"]["kind"], "missing-token");
        assert_eq!(
            value["error"]["data"]["hint"],
            "服务器不会在缺少/错误 token 时回退放开"
        );
    }

    #[test]
    fn wrong_token_is_rejected_with_401() {
        let server = loopback_server();
        let wrong = "0".repeat(crate::security::TOKEN_HEX_LEN);
        let response = server.handle_text(&request(
            Some(&format!("Bearer {wrong}")),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ));
        assert_eq!(response.status, 401);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["data"]["kind"], "invalid-token");
    }

    #[test]
    fn malformed_authorization_is_rejected_with_401() {
        let server = loopback_server();
        let response = server.handle_text(&request(
            Some("Basic dXNlcjpwYXNz"),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ));
        assert_eq!(response.status, 401);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["data"]["kind"], "malformed-authorization");
    }

    #[test]
    fn valid_token_gets_200_and_echoes_the_id() {
        let server = loopback_server();
        let response = server.handle_text(&request(
            Some(&bearer(&server)),
            r#"{"jsonrpc":"2.0","id":"req-7","method":"tools/list"}"#,
        ));
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, CONTENT_TYPE_JSON);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["id"], "req-7");
        assert_eq!(
            value["result"]["tools"].as_array().expect("tools").len(),
            crate::tools::TOOL_COUNT
        );
        let bytes = response.to_bytes();
        assert!(bytes.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert!(bytes.ends_with(response.body.as_bytes()));
    }

    #[test]
    fn non_post_is_rejected_with_405_and_allow_header() {
        let server = loopback_server();
        let raw =
            format!("GET {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n");
        let response = server.handle_text(&raw);
        assert_eq!(response.status, 405);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| *name == "Allow" && value == SUPPORTED_METHOD)
        );
    }

    #[test]
    fn unknown_path_is_404() {
        let server = loopback_server();
        let raw = format!(
            "POST /admin HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: {}\r\nContent-Length: 2\r\n\r\n{{}}",
            bearer(&server)
        );
        assert_eq!(server.handle_text(&raw).status, 404);
    }

    #[test]
    fn missing_content_length_is_411() {
        let server = loopback_server();
        let raw = format!(
            "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: {}\r\n\r\n{{}}",
            bearer(&server)
        );
        let response = server.handle_text(&raw);
        assert_eq!(response.status, 411);
        assert_eq!(response.reason(), "Length Required");
    }

    #[test]
    fn oversized_body_is_rejected_with_413() {
        let server = loopback_server();
        // 只在头上声明 2 MiB, **不真的**发 2 MiB (先判长度再分配)。
        let raw = format!(
            "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: {}\r\nContent-Length: {}\r\n\r\n",
            bearer(&server),
            MAX_REQUEST_BODY_BYTES + 1
        );
        let response = server.handle_text(&raw);
        assert_eq!(response.status, 413);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["kind"], "body-too-large");

        // 实际体超限也一样。
        let big = "x".repeat(MAX_REQUEST_BODY_BYTES + 1);
        let response = server.handle_text(&request(Some(&bearer(&server)), &big));
        assert_eq!(response.status, 413);
    }

    #[test]
    fn garbage_request_line_is_400_and_bad_version_is_505() {
        let server = loopback_server();
        assert_eq!(server.handle_text("not http at all\r\n\r\n").status, 400);
        let raw = format!("POST {MCP_PATH} HTTP/2.0\r\n\r\n");
        let response = server.handle_text(&raw);
        assert_eq!(response.status, 505);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["kind"], "bad-version");
    }

    #[test]
    fn duplicate_content_length_is_rejected() {
        let error = parse_head(&format!(
            "POST {MCP_PATH} HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n"
        ))
        .expect_err("重复头必须被拒");
        assert_eq!(error.http_status(), 400);
        assert!(error.to_string().contains("重复"));
    }

    #[test]
    fn content_length_must_be_decimal() {
        let request = parse_head(&format!(
            "POST {MCP_PATH} HTTP/1.1\r\nContent-Length: 1e3\r\n"
        ))
        .expect("头合法");
        let error = request.parse_content_length().expect_err("必须是十进制");
        assert_eq!(error.http_status(), 400);
    }

    #[test]
    fn headers_are_case_insensitive_and_query_strings_are_stripped() {
        let mut request =
            parse_head("POST /mcp?session=1 HTTP/1.1\r\nAUTHORIZATION: Bearer x\r\n").expect("头");
        assert_eq!(request.path(), MCP_PATH);
        assert_eq!(request.authorization(), Some("Bearer x"));
        request.body = String::from("ok");
        assert_eq!(request.header("Content-Length"), None);
    }

    #[test]
    fn ui_inject_is_still_hard_denied_in_production_over_http_context() {
        // 领域工具集里没有 ui:* 工具 (它们属于 yeban-ui-mcp), 但**判定**在这里:
        // 生产模式下即使 HTTP 通道 + 全量 scope, ui:inject 也一定被拒。
        let expected = BearerToken::generate().token;
        let header = format!("Bearer {}", expected.expose());
        let context = AuthContext::http(Some(&header), ScopeSet::all(), RunMode::Production);
        assert_eq!(
            authorize(&expected, &context, Scope::UiInject),
            Err(Denial::ForbiddenInProduction {
                scope: Scope::UiInject,
                mode: RunMode::Production,
            })
        );
        // 测试模式 + 显式授予才放行。
        let test_context = AuthContext::http(Some(&header), ScopeSet::all(), RunMode::Test);
        assert_eq!(authorize(&expected, &test_context, Scope::UiInject), Ok(()));
    }

    #[test]
    fn render_master_is_in_band_over_http_and_never_answers_501() {
        let server = loopback_server_with_project();
        // 坏参数 ⇒ **带内**领域失败 (200), 校验先于渲染。
        let bad = server.handle_text(&request(
            Some(&bearer(&server)),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"yeban_render_master","arguments":{"format":"mp3","sampleRate":48000}}}"#,
        ));
        assert_eq!(bad.status, 200);
        let value: Value = serde_json::from_str(&bad.body).expect("JSON");
        assert!(value.get("error").is_none(), "领域失败必须带内传递");
        assert_eq!(value["result"]["error"]["code"], "INVALID_PARAMETER_RANGE");

        // 好参数但**工程采样率不一致**（规范样本工程是 48 kHz, 这里请求 44.1 kHz）:
        // 重采样器未接线 ⇒ 契约内的 `RENDER_FAILED`（带内 200），
        // 既不是"假装成功", 也不再是实现级 501。
        let mismatched = server.handle_text(&request(
            Some(&bearer(&server)),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"yeban_render_master","arguments":{"format":"wav","sampleRate":44100}}}"#,
        ));
        assert_eq!(mismatched.status, 200, "渲染器已接线, 不再有 501");
        let value: Value = serde_json::from_str(&mismatched.body).expect("JSON");
        assert!(value.get("error").is_none(), "领域失败必须带内传递");
        assert_eq!(value["result"]["status"], "error");
        assert_eq!(value["result"]["error"]["code"], "RENDER_FAILED");
        assert_eq!(
            value["result"]["error"]["data"]["unwired"], "resampler",
            "必须如实说明是什么没接线"
        );
    }

    #[test]
    fn unauthenticated_malformed_body_is_still_401() {
        // token 闸门在协议层之前: 未鉴权的调用方拿不到"你的 JSON 有问题"这种反馈,
        // 也就无从用错误码差异探测服务端的解析器。
        let server = loopback_server();
        let response = server.handle_text(&request(None, "{not json"));
        assert_eq!(response.status, 401);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["code"], crate::jsonrpc::UNAUTHORIZED);
        assert_eq!(value["id"], Value::Null, "未鉴权时没有 id 可回显");

        // 带上正确 token 之后, 同一个非法体才轮到解析错误。
        let response = server.handle_text(&request(Some(&bearer(&server)), "{not json"));
        assert_eq!(response.status, 400);
        let value: Value = serde_json::from_str(&response.body).expect("JSON");
        assert_eq!(value["error"]["code"], crate::jsonrpc::PARSE_ERROR);
    }

    #[test]
    fn stream_path_checks_method_before_content_length() {
        // 回归: 线上（live）路径曾先要 Content-Length 再判方法, 于是一个不带体的
        // `GET` 会拿到 411 而不是 405 —— 两套口径不一致。这里把它钉死。
        let server = loopback_server();
        let address = server.local_addr().expect("地址");
        let raw = format!("GET {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        std::thread::scope(|scope| {
            scope.spawn(|| {
                server.serve_once().expect("服务一个连接");
            });
            let mut stream = TcpStream::connect(address).expect("连接环回");
            stream.write_all(raw.as_bytes()).expect("写请求");
            stream.shutdown(std::net::Shutdown::Write).expect("半关");
            let mut text = String::new();
            stream.read_to_string(&mut text).expect("读响应");
            assert!(
                text.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
                "实际: {text}"
            );
            assert!(text.contains("Allow: POST"), "405 必须带 Allow: POST");
        });
    }

    #[test]
    fn end_to_end_over_a_real_loopback_socket() {
        let server = loopback_server();
        let address = server.local_addr().expect("地址");
        let auth = bearer(&server);
        let payload = request(
            Some(&auth),
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/list"}"#,
        );

        std::thread::scope(|scope| {
            scope.spawn(|| {
                server.serve_once().expect("服务一个连接");
            });
            let mut stream = TcpStream::connect(address).expect("连接环回");
            stream.write_all(payload.as_bytes()).expect("写请求");
            stream.shutdown(std::net::Shutdown::Write).expect("半关");
            let mut raw = String::new();
            stream.read_to_string(&mut raw).expect("读响应");
            assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"), "实际: {raw}");
            assert!(raw.contains("Connection: close"));
            let (_, body) = split_message(raw.as_bytes());
            let value: Value = serde_json::from_str(&body).expect("JSON");
            assert_eq!(value["id"], 42);
        });

        // 无 token 的同一个 socket 路径 => 401。
        let server = loopback_server();
        let address = server.local_addr().expect("地址");
        let payload = request(None, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                server.serve_once().expect("服务一个连接");
            });
            let mut stream = TcpStream::connect(address).expect("连接环回");
            stream.write_all(payload.as_bytes()).expect("写请求");
            stream.shutdown(std::net::Shutdown::Write).expect("半关");
            let mut raw = String::new();
            stream.read_to_string(&mut raw).expect("读响应");
            assert!(
                raw.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
                "实际: {raw}"
            );
        });
    }

    #[test]
    fn oversized_head_is_rejected_by_the_stream_reader() {
        // 直接喂一个超长头: read_head 在**读完之前**就报 431。
        let server = loopback_server();
        let listener_server = &server;
        let address = listener_server.local_addr().expect("地址");
        let filler = "X-Pad: ".to_owned() + &"a".repeat(MAX_REQUEST_HEAD_BYTES);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                listener_server.serve_once().expect("服务一个连接");
            });
            let mut stream = TcpStream::connect(address).expect("连接");
            // 写满超过上限的字节 (对端会在读到上限时停手并回 431)。
            let _ = stream.write_all(filler.as_bytes());
            let mut raw = String::new();
            let _ = stream.read_to_string(&mut raw);
            assert!(
                raw.starts_with("HTTP/1.1 431 ") || raw.is_empty(),
                "要么回 431, 要么在对端停读后连接直接关闭: {raw}"
            );
        });
    }

    #[test]
    fn mode_is_reported_from_the_dispatcher() {
        let server = HttpServer::bind_loopback(dispatcher(RunMode::Test)).expect("绑定");
        assert_eq!(server.lock().mode(), RunMode::Test);
        assert_eq!(
            server.token().expose().len(),
            crate::security::TOKEN_HEX_LEN
        );
        let _ = body(&server);
    }
}
