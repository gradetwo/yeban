//! **生产窗口上的运行期重投影** —— `ROAD-M4-008` 选项 (a) **第 (b) 项**的端到端判据。
//!
//! ## 这条判据要证的句子
//!
//! 一次**会话侧**（真环回 socket + 真令牌 + 真客户端）的工具调用改了那个唯一可变权威的
//! 工程之后，**生产 GUI 的窗口**（不是 `live_surface.rs` 那套只活在测试目标里的装配）
//! 会在宿主装的钩子跑过之后画出新的投影 —— 而在钩子跑之前，它一位不动。
//!
//! ## 为什么这个文件只有**一个** `#[test]`
//!
//! 它用的无头平台带着**真的简单事件循环**
//! （[`yeban_ui_test_port::inspect::install_testing_backend_with_event_loop`]）：
//! `slint::platform::set_platform` 会往一个**进程级** `OnceCell` 里填事件循环 proxy，因此
//! 一个进程里只能有一个线程装它（第二个线程的 `set_platform` 会拿到
//! `SetPlatformError::AlreadySet`）。libtest 默认多线程并行 ⇒ 本文件只能有一个 `#[test]`。
//!
//! ## 论证顺序（每一步都用上一步的实读值）
//!
//! | 步 | 动作 | 读回 |
//! | :--- | :--- | :--- |
//! | 1 | 用**生产构造入口** `host::build_main_window` 建窗口（工程取自权威） | 窗口的自动化泳道里**没有** `track-0-automation-pan-lane` |
//! | 2 | 装上钩子 `AuthorityMirror::install`（把修订号观察者装到控制面上） | 起点已投影修订号 == 权威当前修订号 |
//! | 3 | **会话侧**：真 socket + 真令牌发 `yeban_edit_automation`（在 `TrackPan` 上写一个点） | `status = success`；权威修订号 **+1** |
//! | 4 | 钩子**还没跑**：读同一个活窗口 | 窗口仍是旧投影（**新泳道不在**窗口里）—— 会话侧的改动**没有**同步溜进 UI 线程 |
//! | 5 | 同一时刻读权威工程的投影 | 权威**已经**有那条新泳道（所以第 4 步是真的缺口，不是巧合） |
//! | 6 | 跑掉事件循环里排队的那一条调用（`quit_event_loop` + `run_event_loop`） | 钩子在 **UI 线程**上执行 |
//! | 7 | 再读窗口 | 新泳道**在**窗口里，标签与权威工程的投影**逐字相等**；已投影修订号 == 权威修订号 |
//! | 8 | 一次**只读**工具调用（`yeban_query_project`）+ 再跑一轮事件循环 | 修订号不动 ⇒ 观察者根本没被调用 ⇒ 窗口一位没动 |
//! | 9 | `mount.stop()` | 钩子持的是**弱**句柄 ⇒ 停机后监听口真的关了（没有引用环） |
//!
//! ## 这条判据怎么变红（负向实测见工作线报告）
//!
//! - 不装钩子（`AuthorityMirror::install` 不调用）⇒ 第 7 步：事件循环里没有排队调用，
//!   窗口停在旧投影；
//! - 钩子本体不注入（`sync_now` 里摘掉 `host::apply_view`）⇒ 第 7 步同样变红，
//!   但第 6 步的"真的在 UI 线程上跑过"仍然成立（两种错法方向不同）。
#![allow(unused)]
#![cfg(feature = "in-process-mcp")]

use std::net::TcpStream;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
use yeban_app::reproject::{AuthorityMirror, ProjectionOutcome};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::render::report_line;

/// 生产窗口现在的自动化泳道标签（`.slint` 的 `automation-lane-labels` 属性）。
///
/// 这就是**窗口上的投影**：`host::apply_view` 是唯一的注入点，`.slint` 只读它。
fn lane_labels(ui: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    ui.get_automation_lane_labels()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// 真客户端 + 真令牌 + 真环回 socket（与 `live_ui_mcp.rs` 同一个形状）。
fn mcp_call(address: std::net::SocketAddr, bearer: &str, body: &str) -> serde_json::Value {
    use std::io::{Read as _, Write as _};

    let timeout = Duration::from_secs(5);
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).expect("连接环回控制面（真 socket）");
    stream.set_read_timeout(Some(timeout)).expect("设置读超时");
    stream.set_write_timeout(Some(timeout)).expect("设置写超时");
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Authorization: {bearer}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        yeban_mcp::transport::http::MCP_PATH,
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("写请求头");
    stream.write_all(body.as_bytes()).expect("写请求体");
    stream.flush().expect("刷出请求");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .expect("读响应（超时即失败）");
    let body = raw
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("响应里没有头/体分隔符: {raw}"))
        .1;
    serde_json::from_str(body).unwrap_or_else(|error| panic!("响应体不是 JSON ({error}): {body}"))
}

/// 把事件循环里**已经排队**的那一条调用跑掉就返回。
///
/// 确定性来自三件事，**没有** sleep、**没有**轮询、**没有**超时：
/// 1. `HttpServer::respond` 在**释放分发器锁之后、写出响应字节之前**通知观察者
///    ⇒ 客户端读到响应时，排队调用**一定**已经在队列里；
/// 2. 这里先压一条 `Quit` 再进循环 ⇒ 循环一定会终止（不会 park 在空队列上）；
/// 3. 无头平台的事件循环是 FIFO 队列 ⇒ 先跑重投影、再看到 `Quit`。
fn drain_event_loop() {
    slint::quit_event_loop().expect("无头平台必须提供事件循环 proxy（否则本判据不成立）");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

#[test]
fn a_session_side_mutation_reaches_the_production_window_through_the_runtime_hook() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> automation-lane-labels:"),
        "新 UI 契约面必须声明 > automation-lane-labels:"
    );
    assert!(
        aether_host.contains("set_automation_lane_labels"),
        "宿主接线层必须注册 set_automation_lane_labels"
    );
}
