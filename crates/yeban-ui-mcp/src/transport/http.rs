//! 环回 HTTP/1.1 传输（联网形态）[ARCH-UI-004, ARCH-SEC-002, MUST-GATE-009]。
//!
//! ## 契约（每一条都有判据）
//!
//! | 约束 | 落点 |
//! | :--- | :--- |
//! | 只绑 `127.0.0.1` | [`UiHttpServer::bind_loopback`] + 复用 `yeban_mcp` 的 `assert_loopback` |
//! | 端口 `0`（系统动态分配） | `SocketAddr::new(LOOPBACK_IPV4, 0)`，绑定后**回读** `local_addr()` 再断言 |
//! | 只接受 `POST` | 复用 `SUPPORTED_METHOD`；其它方法 `405` + `Allow: POST` |
//! | 必须带 `Content-Length` | 缺失 ⇒ `411` |
//! | 拒绝超长请求 | 头 > `MAX_REQUEST_HEAD_BYTES` ⇒ `431`；体 > `MAX_REQUEST_BODY_BYTES` ⇒ `413`（**先判长度再分配**） |
//! | 必须带 `Authorization: Bearer <TOKEN>` | 缺失 / 形状非法 / 不匹配 ⇒ `401` + `WWW-Authenticate`，**绝不回退放开** |
//! | 生产模式硬禁 `ui:inject` | [`crate::service::UiService`]（判定不在传输层） |
//!
//! ## 与 `yeban-mcp` 的关系（**复用清单**）
//!
//! 报文层全部复用（见 [`crate::transport`] 的模块文档）：`assert_loopback` /
//! `assert_loopback_peer` / `parse_head` / `split_message` / `HttpRequest` / `HttpResponse` /
//! `HttpError` / 限额常量。本文件只有"驱动"：accept、从 `TcpStream` 读头/读体、
//! 把响应写回去。之所以不能直接复用 `yeban_mcp::transport::http::HttpServer`：
//! 它把 `Mutex<yeban_mcp::dispatch::Dispatcher>` 钉在结构体里，而本线的分发器是
//! [`crate::service::UiService`]。**唯一**因此必须自己写的一小块是
//! [`outcome_of`]（那边的 `HttpError::outcome()` 是私有方法，四行）。
//!
//! ## 边界（**明确没做**，不是"忘了"）
//!
//! 不支持 `Transfer-Encoding` / `chunked`、HTTP/2、TLS、keep-alive（每连接一请求，
//! 响应带 `Connection: close`）、请求流水线、慢速攻击防护。这些都是 `yeban-mcp` 的
//! `http.rs` 文件头明确列出的同款边界 —— 环回 + 单用户开发场景，
//! **不要**把它绑到非环回地址上。

use std::io::{Read as _, Write as _};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};

use yeban_mcp::security::{BearerToken, Channel};
use yeban_mcp::transport::http::{
    BEARER_CHALLENGE, DYNAMIC_PORT, HttpError, HttpRequest, HttpResponse, LOOPBACK_IPV4,
    MAX_REQUEST_BODY_BYTES, MAX_REQUEST_HEAD_BYTES, SUPPORTED_METHOD, assert_loopback,
    assert_loopback_peer, parse_head, split_message,
};

use crate::service::UiService;
use crate::transport::UI_MCP_PATH;

/// 头 / 体分隔符。
const HEAD_TERMINATOR: &[u8] = b"\r\n\r\n";

/// 环回 HTTP 服务。
pub struct UiHttpServer {
    listener: TcpListener,
    service: Mutex<UiService>,
}

impl UiHttpServer {
    /// 绑定 `127.0.0.1:0`，并**回读 `local_addr()` 断言是环回**。
    ///
    /// # Errors
    ///
    /// 绑定失败，或（不可能发生的）回读地址不是环回。
    pub fn bind_loopback(service: UiService) -> Result<Self, HttpError> {
        let address = SocketAddr::new(IpAddr::V4(LOOPBACK_IPV4), DYNAMIC_PORT);
        let listener =
            TcpListener::bind(address).map_err(|source| HttpError::Bind { address, source })?;
        let server = Self {
            listener,
            service: Mutex::new(service),
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

    /// 期望的 Bearer Token（只用于启动日志与判据）。
    #[must_use]
    pub fn token(&self) -> BearerToken {
        self.lock().expected_token().clone()
    }

    /// 取服务（锁被毒化时仍然取内层 —— 环回开发服务宁可继续回答并如实报错，
    /// 也不要在 handler 里 panic）。
    fn lock(&self) -> MutexGuard<'_, UiService> {
        match self.service.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// 处理一个已解析的请求。
    fn respond(&self, request: &HttpRequest) -> HttpResponse {
        if request.method != SUPPORTED_METHOD {
            return method_not_allowed(&request.method);
        }
        if !matches!(request.path(), UI_MCP_PATH | "/") {
            return unknown_path(&request.target);
        }
        let declared = match request.parse_content_length() {
            Ok(Some(length)) => length,
            Ok(None) => return outcome_of(&HttpError::MissingContentLength),
            Err(error) => return outcome_of(&error),
        };
        if declared > MAX_REQUEST_BODY_BYTES || request.body.len() > MAX_REQUEST_BODY_BYTES {
            return outcome_of(&HttpError::BodyTooLarge {
                declared: declared.max(request.body.len()),
                limit: MAX_REQUEST_BODY_BYTES,
            });
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
            Err(error) => outcome_of(&error),
        }
    }

    /// 从一个已连接的 socket 上读完一个请求并回答。
    ///
    /// # Errors
    ///
    /// socket I/O 失败（协议层错误会被翻译成错误响应，不在此返回）。
    pub fn handle_stream(&self, stream: &mut TcpStream) -> Result<(), HttpError> {
        let response = match read_head_request(stream) {
            // 方法与端点先判: 否则一个不带 `Content-Length` 的 `GET` 会先撞上 411,
            // 让人以为"服务器要求 body"而不是"服务器不接受这个方法"。
            Ok(mut request) => {
                if request.method != SUPPORTED_METHOD {
                    method_not_allowed(&request.method)
                } else if !matches!(request.path(), UI_MCP_PATH | "/") {
                    unknown_path(&request.target)
                } else {
                    match read_body(stream, &mut request) {
                        Ok(()) => self.respond(&request),
                        Err(error) => outcome_of(&error),
                    }
                }
            }
            Err(error) => outcome_of(&error),
        };
        stream
            .write_all(&response.to_bytes())
            .and_then(|()| stream.flush())
            .map_err(|source| HttpError::Io { source })
    }

    /// 接受**一个**连接并处理它（判据与嵌入式用法）。
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

    /// 永久服务（**只在 `ui-mcp-http` feature 下存在**）。
    ///
    /// ## 为什么是**单线程串行**，而不是每连接一线程
    ///
    /// `yeban-mcp` 的同名方法用 `std::thread::scope` 每连接开一条线程。本线**刻意不这么做**：
    /// 执行面的真实实现（`yeban_ui_test_port::render::LivePort`）持有 Slint 组件与
    /// `Rc<MinimalSoftwareWindow>`，它**不是 `Send`**（Slint 的组件句柄基于 `Rc`）。
    /// 要求 `Send` 会直接把真实接线排除掉，那比"串行 accept"糟得多。
    ///
    /// 代价与边界（写清楚，不含糊）：同一时刻只服务一个请求；一个**卡住**的连接会阻塞
    /// 后续连接。环回 + 单用户开发场景可以接受；这也意味着**不要**把它暴露到环回之外。
    ///
    /// # Errors
    ///
    /// accept 失败。
    #[cfg(feature = "ui-mcp-http")]
    pub fn serve_forever(&self) -> Result<(), HttpError> {
        loop {
            let (mut stream, peer) = self
                .listener
                .accept()
                .map_err(|source| HttpError::Io { source })?;
            if assert_loopback_peer(peer).is_err() {
                continue;
            }
            // 单连接失败不影响服务其它连接。
            let _ = self.handle_stream(&mut stream);
        }
    }
}

/// 协议层错误 → 响应。
///
/// `yeban_mcp::transport::http::HttpError::outcome()` 是**私有**的，所以这四行必须自己写；
/// 状态码与错误种类仍然来自那边的公开访问器（`http_status()` / `kind()`），
/// 因此两条 MCP 的错误码形状不会漂移。
fn outcome_of(error: &HttpError) -> HttpResponse {
    HttpResponse::json(
        error.http_status(),
        error_body(error.kind(), &error.to_string()),
    )
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
            &format!("未知端点 `{target}` (只服务 `{UI_MCP_PATH}`)"),
        ),
    )
}

/// 错误响应的 JSON 体。
fn error_body(kind: &str, detail: &str) -> String {
    serde_json::json!({"error": {"kind": kind, "detail": detail}}).to_string()
}

/// 只读头并解析出请求行 / 头（体留空）。
fn read_head_request(stream: &mut TcpStream) -> Result<HttpRequest, HttpError> {
    let head_bytes = read_head(stream)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ELEMENT_NOT_FOUND;
    use crate::testing::{FakeSurface, fixture_tree, shared};
    use yeban_mcp::security::{RunMode, ScopeSet};
    use yeban_ui_test_port::port::Permission;

    fn server() -> UiHttpServer {
        server_with(
            Permission::Administrative,
            ScopeSet::all(),
            RunMode::Production,
        )
    }

    fn server_with(permission: Permission, granted: ScopeSet, mode: RunMode) -> UiHttpServer {
        let state = shared(permission);
        let service = UiService::new(
            BearerToken::generate().token,
            granted,
            mode,
            Box::new(FakeSurface {
                state,
                tree: fixture_tree(),
            }),
        );
        UiHttpServer::bind_loopback(service).expect("绑环回")
    }

    fn bearer(server: &UiHttpServer) -> String {
        format!("Bearer {}", server.token().expose())
    }

    fn request(authorization: Option<&str>, body: &str) -> String {
        let mut head = format!(
            "POST {UI_MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(value) = authorization {
            head.push_str(&format!("Authorization: {value}\r\n"));
        }
        head.push_str("\r\n");
        head.push_str(body);
        head
    }

    fn parse(response: &HttpResponse) -> serde_json::Value {
        serde_json::from_str(&response.body).expect("响应体必须是 JSON")
    }

    /// 判据: 只绑环回 + 动态端口（绑定后**回读**地址再断言）。
    #[test]
    fn bind_is_loopback_only_and_uses_a_dynamic_port() {
        let server = server();
        let address = server.local_addr().expect("回读地址");
        assert!(address.ip().is_loopback(), "必须绑在环回上: {address}");
        assert_ne!(address.port(), 0, "端口 0 是**请求**, 系统分配后必须非 0");

        // 绝不允许的地址：用 `Ipv4Addr::UNSPECIFIED` 构造，**不写那个字面量**
        // （守卫 G04 是文本守卫，本判据不依赖它；见 mcp-core notes §3.3）。
        for bad in [
            SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 1234),
            SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 1234),
        ] {
            assert!(
                assert_loopback(bad).is_err(),
                "非环回地址必须被 assert_loopback 拒绝: {bad}"
            );
        }
        assert!(assert_loopback(SocketAddr::new(IpAddr::V4(LOOPBACK_IPV4), 1)).is_ok());
    }

    /// 判据: 缺 / 错 / 形状非法的凭据一律 401 + `WWW-Authenticate`，**不回退放开**。
    #[test]
    fn missing_and_wrong_tokens_are_rejected_with_401() {
        let server = server();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ui/methods"}"#;

        for (label, header) in [
            ("缺凭据", None),
            (
                "错凭据",
                Some("Bearer 0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            ("形状非法", Some("Basic dXNlcjpwYXNz")),
        ] {
            let response = server.handle_text(&request(header, body));
            assert_eq!(response.status, 401, "{label}");
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(name, value)| *name == "WWW-Authenticate" && value.contains("Bearer")),
                "{label} 必须带 Bearer 挑战"
            );
            let json = parse(&response);
            assert_eq!(json["error"]["code"], yeban_mcp::jsonrpc::UNAUTHORIZED);
            assert_eq!(json["id"], serde_json::Value::Null);
        }
    }

    /// 判据: 对 token 才 200，且 `id` **原样回显**。
    #[test]
    fn valid_token_gets_200_and_echoes_the_id() {
        let server = server();
        let header = bearer(&server);
        let response = server.handle_text(&request(
            Some(&header),
            r#"{"jsonrpc":"2.0","id":"live","method":"ui/tree"}"#,
        ));
        assert_eq!(response.status, 200);
        let json = parse(&response);
        assert_eq!(json["id"], "live");
        assert_eq!(json["result"]["tree"]["count"], 3);
    }

    /// 判据: 生产模式下，**即便 token 合法**，事件注入也在这条链路上被硬拒。
    ///
    /// 这是判据 ③ 的 HTTP 侧端到端版本 —— 它证明硬禁不在传输层被绕过。
    #[test]
    fn ui_inject_is_still_hard_denied_in_production_over_http() {
        let server = server();
        let header = bearer(&server);
        let response = server.handle_text(&request(
            Some(&header),
            r#"{"jsonrpc":"2.0","id":5,"method":"ui/dispatch_key_press","params":{"keyCode":"Tab"}}"#,
        ));
        assert_eq!(response.status, 403);
        let json = parse(&response);
        assert_eq!(json["error"]["data"]["kind"], "forbidden-in-production");
        assert_eq!(json["error"]["data"]["scope"], "ui:inject");

        // 测试模式下同一条请求才放行（且执行面真的被调用）。
        let server = server_with(Permission::Interactive, ScopeSet::all(), RunMode::Test);
        let header = bearer(&server);
        let response = server.handle_text(&request(
            Some(&header),
            r#"{"jsonrpc":"2.0","id":5,"method":"ui/dispatch_key_press","params":{"keyCode":"Tab"}}"#,
        ));
        assert_eq!(response.status, 200);
        assert_eq!(parse(&response)["result"]["keyCode"], "Tab");
    }

    /// 判据: 传输层的协议错误各有明确状态码，且**方法先于 Content-Length** 判。
    #[test]
    fn protocol_errors_have_explicit_statuses() {
        let server = server();
        let header = bearer(&server);

        // 方法不对 ⇒ 405 + Allow（即使没有 Content-Length）。
        let get = format!("GET {UI_MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        let response = server.handle_text(&get);
        assert_eq!(response.status, 405);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| *name == "Allow" && value == SUPPORTED_METHOD)
        );

        // 端点不对 ⇒ 404。
        let wrong_path = request(Some(&header), "{}").replace(UI_MCP_PATH, "/nope");
        assert_eq!(server.handle_text(&wrong_path).status, 404);

        // 没有 Content-Length ⇒ 411。
        let no_length = format!("POST {UI_MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        assert_eq!(server.handle_text(&no_length).status, 411);

        // 体超上限 ⇒ 413（**先判长度再分配**）。
        let oversized = format!(
            "POST {UI_MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: {header}\r\nContent-Length: {}\r\n\r\n",
            MAX_REQUEST_BODY_BYTES + 1
        );
        assert_eq!(server.handle_text(&oversized).status, 413);

        // 请求行/版本不合法 ⇒ 400 / 505（复用 `yeban-mcp` 的报文层）。
        assert_eq!(server.handle_text("garbage\r\n\r\n").status, 400);
        let bad_version = format!("POST {UI_MCP_PATH} HTTP/2.0\r\nHost: 127.0.0.1\r\n\r\n");
        assert_eq!(server.handle_text(&bad_version).status, 505);
    }

    /// 判据: 找不到的语义 ID 在 HTTP 上映射到 400（`-32006` 的映射有判据）。
    #[test]
    fn element_not_found_maps_to_400() {
        let server = server();
        let header = bearer(&server);
        let response = server.handle_text(&request(
            Some(&header),
            r#"{"jsonrpc":"2.0","id":2,"method":"ui/node","params":{"elementId":"track-9-fader"}}"#,
        ));
        assert_eq!(response.status, 400);
        let json = parse(&response);
        assert_eq!(json["error"]["code"], ELEMENT_NOT_FOUND);
        assert_eq!(
            crate::service::http_status_for(&yeban_mcp::jsonrpc::ErrorObject::new(
                ELEMENT_NOT_FOUND,
                "x"
            )),
            400
        );
    }

    /// 判据: 真环回 socket 上的端到端（`serve_once` + `std::net::TcpStream`）。
    ///
    /// 端到端**不需要**第二个线程：`connect` 会把请求排进 listen backlog，
    /// 随后同一线程里 `serve_once` 处理它，再去读响应。这也顺带证明"执行面不必是 `Send`"
    /// —— 真实 Slint 窗口正是不 `Send` 的（见 [`UiHttpServer::serve_forever`] 的说明）。
    #[test]
    fn end_to_end_over_a_real_loopback_socket() {
        let server = server();
        let address = server.local_addr().expect("地址");
        let header = bearer(&server);
        let body = r#"{"jsonrpc":"2.0","id":42,"method":"ui/dynamic_regions"}"#;
        let raw = request(Some(&header), body);

        let mut stream = TcpStream::connect(address).expect("连环回");
        stream
            .write_all(raw.as_bytes())
            .and_then(|()| stream.flush())
            .expect("写请求");
        server.serve_once().expect("服务一个连接");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("读响应");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let json: serde_json::Value =
            serde_json::from_str(response.split_once("\r\n\r\n").expect("有头体分隔").1)
                .expect("响应体是 JSON");
        assert_eq!(json["id"], 42);
        assert_eq!(json["result"]["count"], 2);
    }
}
