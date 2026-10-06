//! `[ROAD-M4-001]` 形态 A 的**运行态挂载**判据：`yeban-app` 进程内托管领域 MCP 的
//! 环回 HTTP JSON-RPC 控制面。
//!
//! ## 这条判据要证的句子
//!
//! > 打开编译期与运行期两道开关之后，**一个真实客户端能在一个真实的环回 socket 上**
//! > 用 256-bit 令牌驱动 app 进程内的控制面；**关掉任何一道**都不可能发生；
//! > 停掉之后，那个地址上**没有任何东西**在接受连接。
//!
//! ## 为什么它必须真的走 socket（而不是调 `handle_text`）
//!
//! `crates/yeban-mcp/src/transport/http.rs` 的单元判据已经把**协议层**钉死了
//! （405 / 411 / 413 / 431 / 401 / 200 都在那里，走的是 `handle_text` 与一次
//! `serve_once`）。本文件**不重复**那些断言 —— 它证的是另一件事：
//! **挂载**是真的（依赖边在、线程在、监听 socket 在、停机真的关掉了它）。
//! 因此这里每一次往返都是 `TcpStream` 上的一次真 HTTP/1.1。
//!
//! ## 边界（这三条都是刻意的）
//!
//! - **不碰 `~/.yeban`**：令牌落盘（`mcp_mount::publish_token`）只被 `main.rs` 的 GUI
//!   路径调用；判据直接读内存里的 `mount.token()`，因此不会覆盖用户或 CLI 的会话令牌；
//! - **不占固定端口**：控制面自己绑 `127.0.0.1:0`（系统动态分配），并回读断言是环回；
//! - **不会挂住**：每一次 socket 往返都有 5 秒读写超时，停机也有 5 秒上限
//!   （`mcp_mount` 的 `STOP_TIMEOUT`）⇒ 坏回归是一次**响亮的失败**，不是一次挂到 CI 上限。
//!
//! ## 判据有牙（怎么把它弄红）
//!
//! | 改哪里 | 哪一条会红 |
//! | :--- | :--- |
//! | `mcp_mount::start_for_project` 在 `Disabled` 分支也建会话 | `the_switch_off_builds_nothing_at_all`（不可读工程会立刻 `Err`） |
//! | `InProcessMcp::stop` 只置标志、不关 socket | `the_round_trip_stops_leaving_nothing_listening` 的最后一条断言 |
//! | 令牌校验被放松 | `..._rejects_a_request_without_the_token` |
//! | `CLI_SWITCH` 被改成别的拼写 | `the_runtime_switches_are_the_documented_ones` |
//!
//! 运行：
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-app --features in-process-mcp --tests
//! ```
#![cfg(feature = "in-process-mcp")]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use yeban_app::mcp_mount::{self, InProcessMcp, MountError};
use yeban_mcp::security::{AuthContext, Denial, RunMode, Scope, ScopeSet, authorize};
use yeban_mcp::tools::TOOL_COUNT;
use yeban_mcp::transport::http::{BEARER_CHALLENGE, MCP_PATH};
use yeban_mcp::transport::{ENABLE_HTTP_FLAG, HttpStartup};
use yeban_model::YebanProjectV1;
use yeban_model::project::READER_SCHEMA_VERSION;

/// 一次 socket 往返（或一次停机等待）允许的最长时间。
///
/// 它不是"性能阈值"而是**护栏**：没有它，一个把响应写一半的回归会让 `cargo test` 永远等下去。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 一个已解析的 HTTP 响应（判据只关心状态码、几个头、体）。
#[derive(Debug)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    /// 按名取头（大小写不敏感）。
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// 响应体解析成 JSON（**不是**字符串包含断言：载荷形状是判据的一部分）。
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("响应体不是 JSON ({error}): {}", self.body))
    }
}

/// 在**真实环回 socket** 上发一次 JSON-RPC 请求，读回完整响应。
///
/// `authorization` 为 `None` ⇒ 刻意**不带** `Authorization` 头（缺令牌那条判据）。
fn call(address: SocketAddr, authorization: Option<&str>, body: &str) -> Reply {
    let mut stream =
        TcpStream::connect_timeout(&address, IO_TIMEOUT).expect("连接环回控制面（真 socket）");
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("设置读超时");
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("设置写超时");

    let mut head = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(value) = authorization {
        head.push_str("Authorization: ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes()).expect("写请求头");
    stream.write_all(body.as_bytes()).expect("写请求体");
    stream.flush().expect("刷出请求");

    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .expect("读响应（超时即失败）");
    let (head, body) = match raw.find("\r\n\r\n") {
        Some(index) => (raw[..index].to_owned(), raw[index + 4..].to_owned()),
        None => panic!("响应里没有头 / 体分隔符: {raw}"),
    };

    let status_line = head.lines().next().unwrap_or_default();
    let status: u16 = status_line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("状态行无法解析: {status_line}"));
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();

    Reply {
        status,
        headers,
        body,
    }
}

/// 规范样本工程（与 `http.rs` 的单元判据、`live_ui_mcp.rs` 用的是同一份）。
fn sample_project() -> YebanProjectV1 {
    yeban_model::samples::filled_project()
}

/// **判据 1**：两道运行期开关的字面值与判定表，以及"**开关关着 ⇒ 一位都不碰**"。
///
/// 后半段用了一个**不可读工程**做差分：`schema_version` 比读者版本新 ⇒
/// `Domain::open_in_memory` 必然拒绝。开关关着时它必须连"注入"这一步都不走到
/// （否则会返回 `Err`），开关打开时它必须**真的**被拒 —— 这才证明关着的那条路
/// 不只是"少绑了一个 socket"。
#[test]
fn the_switch_off_builds_nothing_at_all() {
    // ---- 字面值：运行期开关就是 yeban-mcp 形态 B 的那个词 ----
    assert_eq!(
        mcp_mount::CLI_SWITCH,
        ENABLE_HTTP_FLAG,
        "形态 A 与形态 B 的运行期开关必须是同一个字面值"
    );
    assert_eq!(mcp_mount::CLI_SWITCH, yeban_app::cli::MCP_HTTP_SWITCH);
    assert_eq!(mcp_mount::ENV_SWITCH, yeban_app::cli::MCP_HTTP_ENV);
    assert_eq!(mcp_mount::ENV_SWITCH, "YEBAN_MCP_HTTP");

    // ---- 判定表（纯函数） ----
    assert!(!mcp_mount::switch_requested(false, None), "默认必须是关");
    assert!(mcp_mount::switch_requested(true, None), "命令行开关");
    for truthy in ["1", "true", "TRUE", "Yes", "on", "  ON  "] {
        assert!(
            mcp_mount::switch_requested(false, Some(truthy)),
            "`{truthy}` 应当是真值"
        );
    }
    for falsy in ["", "0", "false", "no", "off", "随便写的"] {
        assert!(
            !mcp_mount::switch_requested(false, Some(falsy)),
            "`{falsy}` 不应当被当成真值"
        );
    }

    // ---- 决策复用的是 yeban-mcp 的判定，不是第二套 ----
    assert_eq!(
        mcp_mount::plan(false).expect("关着不是错误"),
        HttpStartup::Disabled
    );
    assert_eq!(
        mcp_mount::plan(true).expect("这个构建里 mcp-http 一定编译进来了"),
        HttpStartup::Enabled
    );

    // ---- 差分：同一个**不可读**工程，关着被忽略、打开被拒 ----
    let mut unreadable = sample_project();
    unreadable.schema_version = READER_SCHEMA_VERSION + 1;

    let off = InProcessMcp::start_for_project(
        false,
        unreadable.clone(),
        PathBuf::from("sample:unreadable"),
    )
    .expect("运行期开关关着 ⇒ 不是错误");
    assert!(
        off.is_none(),
        "开关关着时必须什么都不建（连工程注入都不做）"
    );

    let error =
        InProcessMcp::start_for_project(true, unreadable, PathBuf::from("sample:unreadable"))
            .expect_err("开关打开时必须真的注入工程 ⇒ 不可读工程要被拒");
    assert!(
        matches!(error, MountError::Domain(_)),
        "拒绝必须来自工程注入这一层: {error:?}"
    );
}

/// **判据 2（本工作线的核心）**：真环回 socket 上的一次完整往返 + 停机之后什么都不在监听。
///
/// 断言顺序就是判据的论证顺序：
/// 1. 监听地址是**环回**且端口是系统动态分配的（非 0）；
/// 2. **没有令牌**的请求 = `401` + Bearer 挑战（令牌闸门在协议层之前）；
/// 3. 带令牌 `tools/list` = `200`，工具条数等于注册表（`TOOL_COUNT`），`id` 原样回显；
/// 4. 带令牌 `tools/call yeban_query_project` = `200`，载荷里是**app 打开的那份工程**
///    （轨道数与样本一致）⇒ 控制面服务的就是本进程的会话，不是一个空壳；
/// 5. 只读会话的牙：`yeban_save_project` 被拒（`IO_ERROR`）⇒ 挂载没有交出第二个写者；
/// 6. `[MUST-GATE-009]` 没有被削弱：即使用**这个**令牌 + 全量 scope，
///    生产模式下 `ui:inject` 仍然硬拒；
/// 7. `stop()` 之后，**同一个地址连不上**。
#[test]
fn the_round_trip_stops_leaving_nothing_listening() {
    let project = sample_project();
    let expected_tracks = project.tracks.len();
    assert!(
        expected_tracks > 0,
        "样本工程必须有轨道（否则第 4 条无意义）"
    );

    let mount = InProcessMcp::start_for_project(true, project, PathBuf::from("sample:filled"))
        .expect("挂载决策")
        .expect("运行期开关打开时必须真的挂载");

    // ---- 1. 只绑环回 + 动态端口 ----
    let address = mount.address();
    assert!(address.ip().is_loopback(), "控制面必须绑在环回: {address}");
    assert_ne!(address.port(), 0, "端口 0 由系统分配, 回读必须非 0");
    assert_eq!(mount.endpoint(), format!("http://{address}{MCP_PATH}"));
    assert_eq!(
        mount.token().expose().len(),
        yeban_mcp::security::TOKEN_HEX_LEN,
        "会话令牌必须是 256-bit（64 位十六进制），不允许第二套方案"
    );
    let bearer = format!("Bearer {}", mount.token().expose());

    // ---- 2. 没有令牌 ⇒ 401（硬拒，不回退放开） ----
    let reply = call(
        address,
        None,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    );
    assert_eq!(reply.status, 401, "缺令牌必须 401; 体={}", reply.body);
    assert_eq!(reply.header("www-authenticate"), Some(BEARER_CHALLENGE));
    let value = reply.json();
    assert_eq!(value["error"]["code"], yeban_mcp::jsonrpc::UNAUTHORIZED);
    assert_eq!(value["error"]["data"]["kind"], "missing-token");

    // ---- 3. 带令牌 tools/list ⇒ 200 + 注册表条数 + id 回显 ----
    let reply = call(
        address,
        Some(&bearer),
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#,
    );
    assert_eq!(reply.status, 200, "带令牌必须 200; 体={}", reply.body);
    let value = reply.json();
    assert_eq!(value["id"], 7, "JSON-RPC 的 id 必须原样回显");
    assert_eq!(
        value["result"]["tools"]
            .as_array()
            .expect("tools 是数组")
            .len(),
        TOOL_COUNT,
        "`tools/list` 必须与注册表逐项对得上"
    );

    // ---- 4. 真调用：控制面服务的就是 app 注入的那份工程 ----
    let reply = call(
        address,
        Some(&bearer),
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"yeban_query_project","arguments":{"limit":2}}}"#,
    );
    assert_eq!(reply.status, 200, "领域调用是带内的; 体={}", reply.body);
    let value = reply.json();
    assert_eq!(value["result"]["status"], "success", "{value}");
    assert_eq!(
        value["result"]["data"]["page"]["limit"], 2,
        "实参必须被解析"
    );
    assert_eq!(
        value["result"]["data"]["project"]["tracks"]
            .as_object()
            .expect("project.tracks 是 BTreeMap<EntityId, TrackV3> ⇒ JSON 对象")
            .len(),
        expected_tracks,
        "控制面注入的必须是 app 交进来的那份工程"
    );

    // ---- 5. 只读会话的牙：不许交出第二个写者 ----
    let reply = call(
        address,
        Some(&bearer),
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"yeban_save_project","arguments":{}}}"#,
    );
    assert_eq!(reply.status, 200, "领域失败是带内的; 体={}", reply.body);
    let value = reply.json();
    assert_eq!(value["result"]["status"], "error", "{value}");
    assert_eq!(
        value["result"]["error"]["code"], "IO_ERROR",
        "只读打开的会话必须拒绝落盘: {value}"
    );

    // ---- 6. `[MUST-GATE-009]` 没有被这次挂载削弱 ----
    let authorization = format!("Bearer {}", mount.token().expose());
    let context = AuthContext::http(Some(&authorization), ScopeSet::all(), RunMode::Production);
    assert!(
        matches!(
            authorize(mount.token(), &context, Scope::UiInject),
            Err(Denial::ForbiddenInProduction { .. })
        ),
        "生产模式下 ui:inject 仍然必须硬拒（即使令牌正确、scope 全给）"
    );

    // ---- 7. 停机 ⇒ 同一地址上没有任何东西在监听 ----
    let endpoint = mount.endpoint();
    mount.stop().expect("停机必须成功（有 5 秒上限）");
    assert!(
        TcpStream::connect_timeout(&address, IO_TIMEOUT).is_err(),
        "停机之后 {endpoint} 上还有东西在接受连接 —— 「停止」不是真的"
    );
}
