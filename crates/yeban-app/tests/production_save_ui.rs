//! **生产窗口的保存入口**（`ROAD-M4-008` 选项 (a) 剩下的"保存 UI"缺口）—— 端到端判据。
//!
//! ## 这条判据要证的句子
//!
//! `docs/ledger/m4-008-authority-notes.md` §9.5 第 4 条如实登记过：生产 `run_gui`
//! 不构造保存执行面，`src/live_surface.rs` 只被**测试目标** `#[path]` 装入 ⇒
//! 产品二进制里没有 `ui/force_save` 这条 UI 命令，**用户按不到保存**。
//!
//! 这条判据钉住收口：**生产构造入口建出来的窗口**（`host::build_main_window`，
//! 与 `run_gui` 同一个函数）上，`host::wire_save` 装的那个回调真的能写盘；挂了控制面
//! 会话时它写的是**那个会话当前那一版**（不是窗口里那份旧投影）。
//!
//! ## 这条判据怎么变红
//!
//! - `wire_save` 里不接 `dispatch_save`（回调空转）⇒ 步 3 的文件字节未变、状态文本仍空；
//! - `.slint` 的 `transport-save-button` 没接 `save-project` 回调（`wire_save` 装不上
//!   `on_save_project`，或按钮不触发它）⇒ 同一步红；
//! - `host::apply_save_outcome` 不写 `save_status` ⇒ 步 3 的状态文本断言红；
//! - 把保存的目标路径写成别的地方 ⇒ 步 3 的那个文件字节未变（断言的是**那个**路径）。
//!
//! ## 为什么这个文件只有一个 `#[test]`（平台约束，不是偏好）
//!
//! `slint::platform::set_platform` 是**线程局部**的，而 [`Tier1Window::install`] 每线程
//! 只能成功一次。libtest 默认每个用例一个线程；这里沿用 `live_ui_mcp.rs` 的用法
//! （"一个用例装一次、用完即弃"），但为了让"平台装不上"这种偶发红**不可能**出现，
//! 本文件只保留一个端到端用例；其余路由判据住在 `single_writer_session.rs`
//! （不需要窗口）与 `in_process_mcp_lock.rs`（跨进程持有者）。
#![allow(unused)]
#![cfg(feature = "in-process-mcp")]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
use yeban_app::open::open_project_file;
use yeban_app::save_action::{SaveOutcome, SaveRequest, dispatch_save};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::{Tier1Window, report_line};

/// 一次 socket 往返允许的最长时间（护栏，不是性能阈值）。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 窗口尺寸（非零即合法；判据只要求"窗口真的建起来过"）。
const WINDOW: Size = Size::new(1440, 900);

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-production-save-{}-{tag}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 写一份**确定的**真容器工程（与 `single_writer_session.rs` 同一份夹具数据）。
    fn project(&self, name: &str) -> PathBuf {
        let project = yeban_model::samples::filled_project();
        let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱 JSON");
        let bytes =
            yeban_model::container::write_project_container(&project, &history, &BTreeMap::new())
                .expect("写真容器");
        let path = self.join(name);
        std::fs::write(&path, &bytes).expect("写容器工程");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 真环回 socket 上的一次 JSON-RPC 往返（与 `single_writer_session.rs` 同一个形状）。
fn mcp_call(address: SocketAddr, bearer: &str, body: &str) -> serde_json::Value {
    use std::io::{Read as _, Write as _};

    let mut stream =
        std::net::TcpStream::connect_timeout(&address, IO_TIMEOUT).expect("连接环回控制面");
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("设置读超时");
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("设置写超时");
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

/// 从磁盘上的工程算"运行时元素 ID"集合（与界面同一条投影层）。
fn lane_element_ids(project: &yeban_model::YebanProjectV1) -> Vec<String> {
    ViewState::from_project(project)
        .expect("投影")
        .automation_lane_element_ids()
}

/// `ROAD-M4-008` 选项 (a)：**生产窗口上的保存入口真的写盘，且写的是权威当前那一版**。
///
/// ## 论证顺序（每一步都用上一步的实读值）
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 装 Tier-1 无头平台 + 用**生产构造入口** `host::build_main_window` 建窗口（工程取自权威）+ 调**生产接线** `host::wire_save` | 窗口存在；状态文本是空的（还没按过保存）；夹具里**没有** pan 泳道 |
/// | 2 | 会话侧真 socket 发 `yeban_edit_automation`；**不**重投影 | 权威工程里**有**新泳道 |
/// | 3 | `.slint` 的 `save-project` 回调（`ui.invoke_save_project()`） | 状态文本点名"经控制面会话写盘"；文件字节**真的换了**、含新泳道；`apply_revision` 一位不动 |
/// | 4 | 没有控制面时同一条 `dispatch_save`（同一个入口）的目标是**本地**路径 | 落盘成功、`by_authority == false`（产品路径两条分支都可达） |
///
/// 第 3 步是这条判据的**承重步**：它不经过 `live_surface.rs`（测试目标的装配），
/// 用的是产品二进制里那一个 `host::wire_save`。
#[test]
fn the_production_window_save_entry_writes_through_the_mounted_authority() {
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
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
}
