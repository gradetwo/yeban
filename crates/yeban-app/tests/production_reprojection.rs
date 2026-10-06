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
    // 本进程唯一的平台安装（见文件头的"为什么只有一个 #[test]"）。
    yeban_ui_test_port::inspect::install_testing_backend_with_event_loop();

    // ---- 1. 真控制面 + 生产构造入口建窗口（工程取自权威，不是随手一份） ----
    let project = yeban_model::samples::filled_project();
    let mount = InProcessMcp::start_for_project(
        true,
        project,
        SessionSource::InMemory(std::path::PathBuf::from("sample:m4-008-reproject")),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let authority = mount.project_authority();
    let bearer = format!("Bearer {}", mount.token().expose());

    let authority_project = authority.project().expect("权威有活跃工程");
    let view = ViewState::from_project(&authority_project).expect("权威工程的投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");

    let lead_index = view.tracks[0].index;
    let lead_id = view.tracks[0].id.clone();
    let lane_id = format!("track-{lead_index}-automation-pan-lane");
    assert!(
        !view.automation_lane_element_ids().contains(&lane_id),
        "起点：`{lane_id}` 不该存在 —— 第 3 步要新建的就是它"
    );

    // 与生产一致：窗口尺寸定下来之后用**同一条注入**再走一次（`apply_view` 幂等）。
    slint::ComponentHandle::window(&ui).set_size(slint::PhysicalSize::new(1440, 900));
    host::apply_view(&ui, &view, 1440.0, 0.0);
    let labels_before = lane_labels(&ui);
    assert!(
        !labels_before.iter().any(|label| label.contains(&lane_id)),
        "起点：窗口的自动化泳道里不该有 `{lane_id}`: {labels_before:?}"
    );

    // ---- 2. 装钩子：把修订号观察者装到控制面上 ----
    let mirror = AuthorityMirror::install(&ui, &mount).expect("服务活着 ⇒ 必须装上");
    let revision_before = authority.apply_revision();
    assert_eq!(
        mirror.projected_revision(),
        revision_before,
        "起点：窗口画的这一版 == 权威当前这一版（同一个 Domain 建出来的）"
    );

    // ---- 3. 会话侧：真 socket 上的一次**会改工程**的工具调用 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"yeban_edit_automation","arguments":{{"trackId":"{lead_id}","lane":"TrackPan","point":{{"tick":0,"value":0.25,"curve":"Linear"}}}}}}}}"#
        ),
    );
    assert_eq!(
        reply["result"]["status"], "success",
        "写类工具必须成功: {reply}"
    );
    let revision_after = authority.apply_revision();
    assert_eq!(
        revision_after,
        revision_before + 1,
        "一次改工程的施加必须恰好推进一个修订号"
    );

    // ---- 4. 钩子还没跑：窗口**一位不动**（会话侧的改动没有同步溜进 UI 线程） ----
    let labels_pending = lane_labels(&ui);
    assert_eq!(
        labels_pending, labels_before,
        "钩子跑之前生产窗口必须还是旧投影（否则说明有人在服务线程上直接碰了 UI）"
    );

    // ---- 5. 同一时刻权威**已经**有那条新泳道 ⇒ 第 4 步是真的缺口 ----
    let authority_view = ViewState::from_project(&authority.project().expect("权威有活跃工程"))
        .expect("权威工程的投影");
    assert!(
        authority_view
            .automation_lane_element_ids()
            .contains(&lane_id),
        "权威工程里必须出现 `{lane_id}`: {:?}",
        authority_view.automation_lane_element_ids()
    );
    let expected_label = authority_view
        .automation_lanes
        .iter()
        .find(|lane| lane.element_id == lane_id)
        .expect("新泳道的投影")
        .label
        .clone();

    // ---- 6. 跑掉事件循环里排队的那一条调用（钩子在 UI 线程上执行） ----
    drain_event_loop();

    // ---- 7. 窗口真的变了：标签与**权威工程的投影**逐字相等 ----
    let labels_after = lane_labels(&ui);
    assert!(
        labels_after.contains(&expected_label),
        "钩子跑过之后生产窗口里必须有 `{lane_id}` 的标签 `{expected_label}`: {labels_after:?}"
    );
    assert_eq!(
        labels_after,
        authority_view.automation_lane_labels(),
        "窗口的泳道标签必须逐字等于权威工程的投影"
    );
    assert_eq!(
        mirror.projected_revision(),
        revision_after,
        "重投影成功之后投影游标必须跟上权威修订号"
    );

    // ---- 8. 只读调用不刷新界面：修订号不动 ⇒ 观察者根本没被调用 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"yeban_query_project","arguments":{"limit":2}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert_eq!(
        authority.apply_revision(),
        revision_after,
        "只读工具不得推进施加修订号"
    );
    drain_event_loop();
    assert_eq!(
        lane_labels(&ui),
        labels_after,
        "一次只读调用不得让窗口重投影"
    );
    assert_eq!(
        mirror.projected_revision(),
        revision_after,
        "只读调用不推进投影游标"
    );

    // ---- 9. 钩子本体单独再驱动一次：幂等（没有新版本 ⇒ `Unchanged`） ----
    assert_eq!(
        mirror.sync_now(&ui),
        ProjectionOutcome::Unchanged,
        "已经投影过这一版 ⇒ 直接驱动钩子也不得重投影"
    );

    report_line(&format!(
        "[m4-008-b] 生产窗口 + 会话侧改动 ⇒ 钩子把它投影上去: 环回 socket 上 `yeban_edit_automation` \
         新建 `{lane_id}` ⇒ 权威修订号 {revision_before}→{revision_after}; 钩子跑之前窗口泳道 \
         {} 条（无新泳道）、跑之后 {} 条且标签 = `{expected_label}`",
        labels_before.len(),
        labels_after.len(),
    ));

    // ---- 10. 停机：钩子持的是弱句柄 ⇒ 监听口真的关了（没有引用环） ----
    //
    // `drop(authority)` 是这条断言的**前提**：判据自己也持着一个强句柄
    // （[`ProjectAuthorityHandle`]，内含 `Arc<HttpServer>`），不放开它的话
    // "还在监听"量到的是判据自己，而不是生产代码里的引用环。
    drop(authority);
    let address = mount.address();
    mount.stop().expect("停机必须成功（有 5 秒上限）");
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_err(),
        "停机之后不该还有东西在监听 {address}：钩子若持有强句柄就会形成 \
         HttpServer → 观察者 → Arc<HttpServer> 的引用环，这里会变红"
    );
}
