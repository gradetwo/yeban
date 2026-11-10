//! **判据**：控制面把工程关掉（`yeban_close_project`）之后，GUI 侧那几条
//! "读当前工程"的路**不得 panic**。
//!
//! ## 这条判据证的是哪一句话
//!
//! > 一个还在跑的 `yeban-app` 进程，在工程被控制面关掉之后，界面上的每一条读工程的路
//! > 都**照常返回**（顶多是一条 `outcome=error` 回执），**不会让整个进程退掉**。
//!
//! ## 缺陷现场（本判据的第一版就是复现，实测）
//!
//! ```text
//! $ yeban-app --open 某工程.yeban --enable-mcp-http
//! $ curl … tools/call yeban_close_project      # 工程关闭，进程应当活着
//! thread 'main' panicked at crates/yeban-app/src/undo.rs:547:
//! 撤销端口必须有一份工程（权威会话此刻没有活跃工程）
//!   …
//!   5: <yeban_app::undo::UndoPort>::project
//!   6: yeban_app::host::wire_module_actions_with_capture::{closure#7}   ← host.rs:9120
//!  11: yeban_app::run_gui                                               ← main.rs:888
//! ```
//!
//! 三件事叠在一起才炸：
//!
//! 1. `--enable-mcp-http` 的控制面**先**绑好 socket，GUI 的装配（`wire_module_actions`）
//!    **后**跑 —— 一个刚连上的客户端完全来得及在装配期里把工程关掉；
//! 2. 装配期那一句是**立即求值**的（实时 MIDI 腿的目标音轨 / 片段要先算好交给工作线程）；
//! 3. 它用的是 `UndoPort::project()`（**记录在案的 panic 入口**）而不是 `try_project()`。
//!
//! 同一个缺陷还有四处**用户按得到**的第二现场（导出 WAV / 导出 MIDI / 导入 MIDI /
//! 实时 MIDI 落地），它们都在同一个"工程可能已经不在"的前提上。
//!
//! ## 修法
//!
//! 五处的读数统一走 `try_project()`；"没有活跃工程"变成一条**有稳定错误码**的回执
//! （[`yeban_app::host::NO_ACTIVE_PROJECT_CODE`]），而不是一次 panic。实时腿的目标解析
//! 收成**一个**函数 [`yeban_app::host::live_note_target_of`]，装配与判据走同一段代码。
//!
//! ## 怎么变红
//!
//! | 改哪里 | 哪一条红 |
//! | :--- | :--- |
//! | `live_note_target_of` 里 `try_project()` 换回 `project()` | `closing_the_project_does_not_panic_the_reader_side` |
//! | `UndoPort::project()` 的 panic 文案被改（契约漂移） | `the_panic_entry_still_says_what_it_documents` |
#![cfg(feature = "in-process-mcp")]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use yeban_app::host;
use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
use yeban_app::undo::UndoPort;
use yeban_mcp::transport::http::MCP_PATH;

/// 一次 socket 往返允许的最长时间（护栏，不是性能阈值）。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 在真环回 socket 上发一次 JSON-RPC，返回解析后的响应体。
fn call(address: SocketAddr, bearer: &str, body: &str) -> Value {
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT).expect("连接环回控制面");
    stream.set_read_timeout(Some(IO_TIMEOUT)).expect("读超时");
    stream.set_write_timeout(Some(IO_TIMEOUT)).expect("写超时");
    let head = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\nAuthorization: {bearer}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("写请求头");
    stream.write_all(body.as_bytes()).expect("写请求体");
    stream.flush().expect("刷出请求");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("读响应");
    let body = match raw.find("\r\n\r\n") {
        Some(index) => &raw[index + 4..],
        None => panic!("响应没有头体分隔: {raw}"),
    };
    serde_json::from_str(body).unwrap_or_else(|error| panic!("响应体不是 JSON ({error}): {body}"))
}

/// 挂载一个**内存会话**控制面，并把它包成 GUI 侧的 `UndoPort`（与 `main.rs` 同一个入口）。
fn mounted() -> (InProcessMcp, String, UndoPort) {
    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::InMemory(PathBuf::from("<test:no-active-project>")),
        Vec::new(),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let bearer = format!("Bearer {}", mount.token().expose());
    let port = UndoPort::from_authority(mount.project_authority());
    (mount, bearer, port)
}

/// **核心判据**：关掉工程之后，实时腿的目标解析**返回 `None`**，而不是把进程带走。
///
/// 论证顺序（每一步都用上一步的实读值）：
/// 1. 关之前：端口有工程，目标能解析出 `(音轨, 片段)` —— 证明这条路径平时**是有值的**，
///    不是在测一个永远为空的函数；
/// 2. 经**真 socket** 调 `yeban_close_project` ⇒ `success`（工程真的关了）；
/// 3. 关之后：`try_project()` 是 `None`（前提读数）；
/// 4. 关之后：`live_note_target_of(Some(&port))` 在 `catch_unwind` 里跑 ⇒ **`Ok(None)`**。
///    修前这里是 `Err(panic)`，栈落在 `UndoPort::project`。
#[test]
fn closing_the_project_does_not_panic_the_reader_side() {
    let (mount, bearer, port) = mounted();

    // ---- 1. 关之前：有工程、也有落点 ----
    assert!(
        port.try_project().is_some(),
        "前提：刚挂载的端口必须有一份工程"
    );
    let before = host::live_note_target_of(Some(&port));
    assert!(
        before.is_some(),
        "前提：样本工程有非 master 音轨 ＋ 摆放 ⇒ 实时腿必须有落点"
    );

    // ---- 2. 经真 socket 关掉工程 ----
    let reply = call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"yeban_close_project","arguments":{"saveFirst":false}}}"#,
    );
    assert_eq!(
        reply["result"]["status"], "success",
        "关工程必须成功（内存会话不可写 ⇒ saveFirst 不落盘）: {reply}"
    );

    // ---- 3. 前提读数：这一刻没有工程了 ----
    assert!(
        port.try_project().is_none(),
        "关掉之后端口不得再交出一份工程（那会是一个幽灵读数）"
    );

    // ---- 4. 真正的断言：解析目标**不许 panic** ----
    let landed =
        std::panic::catch_unwind(AssertUnwindSafe(|| host::live_note_target_of(Some(&port))));
    let landed = match landed {
        Ok(target) => target,
        Err(payload) => panic!(
            "⭐ 工程关闭之后 `live_note_target_of` 必须返回 `None`，实得 panic: {} —— \
             生产里这一刻正是用户在按导出 / 采 MIDI，panic = 整个进程消失",
            panic_message(&payload)
        ),
    };
    assert_eq!(
        landed, None,
        "没有工程 ⇒ 没有落点（⛔ 不许回退到装配期那份旧工程）"
    );
    assert_eq!(
        landed, None,
        "第二遍仍然 `None`（解析必须是纯读，不缓存过期结果）"
    );

    // ---- 5. 端口缺席那一档：同样 `None`，同样不 panic ----
    assert_eq!(
        host::live_note_target_of(None),
        None,
        "没有端口 = 没有落点（与 import-midi 的 no-undo-port 同一档）"
    );

    mount.stop().expect("停机必须成功");
}

/// 把 `UndoPort::project()` 的 **panic 文案**钉住：它是记录在案的契约（`# Panics` 一节），
/// 不是随手写的日志。
///
/// 为什么值得一条判据：上一条判据证明了"GUI 的路不走它"。这一条证明"它**仍然**会 panic" ——
/// 于是"哪些调用方必须用 `try_project`"这个判断有据可依，而不是靠读者记得。
#[test]
fn the_panic_entry_still_says_what_it_documents() {
    let (mount, bearer, port) = mounted();
    let reply = call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"yeban_close_project","arguments":{"saveFirst":false}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert!(port.try_project().is_none(), "前提：工程已关");

    let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| port.project()));
    let payload = match panicked {
        Ok(project) => panic!(
            "⛔ `UndoPort::project()` 在没有活跃工程时**必须** panic（它是 `# Panics` 文档的\
             那一条），实得一份工程: tracks={}",
            project.tracks.len()
        ),
        Err(payload) => payload,
    };
    let message = panic_message(&payload);
    assert!(
        message.contains("撤销端口必须有一份工程"),
        "panic 文案是契约的一部分（调用方据此决定用 try_project）: 实得 {message:?}"
    );

    mount.stop().expect("停机必须成功");
}

/// `catch_unwind` 的载荷 ⇒ 人类可读的字符串（`&str` / `String` 两种都见过）。
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<非字符串 panic 载荷>".to_owned()
    }
}
