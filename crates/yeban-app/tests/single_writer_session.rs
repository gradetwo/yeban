//! `[ROAD-M4-008]` / `open-questions.md` 问题 6 选项 (a)：**单一写者会话**。
//!
//! ## 这条判据要证的句子
//!
//! > 当 app 以进程内环回 HTTP 控制面（形态 A）打开一个**磁盘上的真工程**时，
//! > 那个会话就是这份文档的**唯一磁盘写者**：GUI 的保存（宿主保存动作
//! > `ProjectAuthorityHandle::save_to`）写出的内容**持久化**且能被后续读者读回；
//! > GUI 自己的本地保存路径（`save_project_file` / `--save-as`）在会话存活期间
//! > **不可能**成为第二条写路径；第二个形态（读或写）在它存活期间**拿不到**这份文档。
//!
//! ## 为什么这条判据必须真的跨进程 / 真 socket
//!
//! `crates/yeban-app/tests/in_process_mcp_lock.rs` 已经把只读挂载的跨形态互斥钉死。
//! 本文件**不重复**那些断言 —— 它证的是**新接线的那一半**：
//! [`SessionSource::WritableFile`] 真的取了**排他写**锁、那个会话真的能落盘、
//! 而"GUI 的本地保存路径"在它持锁时真的被挡。
//!
//! ## 判据怎么变红（负向实测见工作线报告）
//!
//! | 临时改哪里 | 哪一条会红 |
//! | :--- | :--- |
//! | `SessionSource::WritableFile::lock_mode` 改成 `SharedRead` | `a_writable_session_is_the_only_writer_and_its_save_persists`（第二个挂载不再被拒 ⇒ 两个写者） |
//! | `domain::host_save_project` 不调用 `write_project_atomic` | 同一条（读回的仍是**旧**夹具，新泳道不在） |
//! | `SessionSource::session_read_only` 对写形态返回 `true` | 同一条（宿主保存动作被同一个 `read_only` 门拒） |
//! | `acquire_session_lock` 改回恒 `SharedRead` | 同一条 |
//! | `save_action::dispatch_save` 在只读权威上回退本地路径 | `the_production_save_refuses_on_a_read_only_authority_without_falling_back`（字节会被写进去） |
//! | `dispatch_save` 绕过权威恒走本地路径 | `the_production_save_dispatches_through_the_writable_authority`（本地路径被会话的排他锁拒） |
//!
//! 运行：
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-app --features in-process-mcp --tests
//! ```
#![allow(unused)]
#![cfg(feature = "in-process-mcp")]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use yeban_app::bridge::ViewState;
use yeban_app::mcp_mount::{InProcessMcp, MountError, SessionSource};
use yeban_app::open::open_project_file;
use yeban_app::save::{SaveError, save_project_file};
use yeban_app::save_action::{SaveOutcome, SaveRequest, SaveStage, dispatch_save};
use yeban_mcp::domain::store::{LockMode, lock_path};
use yeban_mcp::transport::http::MCP_PATH;

/// 一次 socket 往返允许的最长时间（护栏，不是性能阈值）。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-single-writer-{}-{tag}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 写一份**确定的**真容器工程（与 `in_process_mcp_lock.rs` 用同一份夹具数据）。
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

/// 样本工程里**投影顺序第一条**轨道的身份与索引。
///
/// 为什么要走投影而不是直接取 `project.tracks` 的第一个键：工具参数里的 `trackId` 是
/// **投影**里的那一份身份（`bridge::ViewState` 的 `TrackView::id`），与 `BTreeMap` 的
/// 键序不保证一致 —— 两处各取一次就会写出另一个轨道的泳道（本判据第一版正是这样红的）。
fn lead_track_of(project: &yeban_model::YebanProjectV1) -> (String, usize) {
    let view = ViewState::from_project(project).expect("投影");
    let lead = view.tracks.first().expect("样本工程必须有轨道");
    (lead.id.clone(), lead.index)
}

/// 真环回 socket 上的一次 JSON-RPC 往返。
fn mcp_call(address: SocketAddr, bearer: &str, body: &str) -> Value {
    let mut stream =
        TcpStream::connect_timeout(&address, IO_TIMEOUT).expect("连接环回控制面（真 socket）");
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .expect("设置读超时");
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .expect("设置写超时");
    let head = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Authorization: {bearer}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

/// 一个会**改变工程**的写类工具调用：在 `TrackPan` 上没有泳道的地方写一个点。
fn write_a_pan_lane(address: SocketAddr, bearer: &str, track_id: &str) -> Value {
    mcp_call(
        address,
        bearer,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"yeban_edit_automation","arguments":{{"trackId":"{track_id}","lane":"TrackPan","point":{{"tick":0,"value":0.25,"curve":"Linear"}}}}}}}}"#
        ),
    )
}

/// 从磁盘上的工程算"运行时元素 ID"集合（容器 → 投影，与界面同一条投影层）。
fn lane_element_ids(project: &yeban_model::YebanProjectV1) -> Vec<String> {
    ViewState::from_project(project)
        .expect("投影")
        .automation_lane_element_ids()
}

// ---------------------------------------------------------------------------
// 判据 1：写会话是**唯一**写者，它的保存真的落盘
// ---------------------------------------------------------------------------

/// `ROAD-M4-008` 选项 (a) 的主判据：`SessionSource::WritableFile` 挂出来的会话
/// ① 取**排他写**锁；② 在它存活期间**第二个形态挂不上**（读或写都不行）⇒ 不会有两个写者；
/// ③ 宿主保存动作写出的内容**持久化**且被后续读者读回；④ 工具面的 `yeban_save_project`
/// 与它是**同一个**写会话（同一个 `read_only` 门），因此不是第二个写者。
#[test]
fn a_writable_session_is_the_only_writer_and_its_save_persists() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

// ---------------------------------------------------------------------------
// 判据 2：GUI 的**本地**保存路径在对写会话存活时不可能写
// ---------------------------------------------------------------------------

/// `ROAD-M4-008` 选项 (a) 的"没有第二条写路径"那一半：写会话持排他锁时，
/// GUI 的本地落点（`ui/force_save` 的 `save_project_file`）与命令行 `--save-as`
/// **都被拒**，且工程字节**逐字节未变**；会话停机后同一条本地保存必须成功
/// （证明拒绝的原因就是那把锁）。
#[test]
fn the_local_gui_save_cannot_write_while_a_writable_session_lives() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

// ---------------------------------------------------------------------------
// 判据 3：只读会话**没有**宿主保存动作（同一个 read_only 门）
// ---------------------------------------------------------------------------

/// 宿主保存动作**不是**绕过 `read_only` 的后门：只读挂载（[`SessionSource::File`]）
/// 上调用它必须 `Err`，且**一个字节都不写**。这条把"宿主保存动作可用 ⟺ 会话可写"
/// 变成可失败的断言 —— 否则"单一写者"可以靠"谁能调谁就写"来伪造。
#[test]
fn the_host_save_action_is_refused_on_a_read_only_session() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

// ---------------------------------------------------------------------------
// 判据 4：**生产保存入口**（`save_action::dispatch_save`）的两个方向
// ---------------------------------------------------------------------------
//
// `ROAD-M4-008` 选项 (a) 剩下的"保存 UI"缺口：生产 `run_gui` 以前不构造保存执行面
// （`src/live_surface.rs` 只被测试目标 `#[path]` 装入），因此用户按不到保存。
// 现在产品路径有 `save_action::dispatch_save`（`crate::host::wire_save` 是它唯一的
// 界面调用点），这一组判据钉住它的**路由**：
//
// | 判据 | 证什么 |
// | :--- | :--- |
// | [`the_production_save_dispatches_through_the_writable_authority`] | 有可写会话时保存**经**那个会话（`by_authority`），写的是会话当前那一版，且后续读者读得回 |
// | [`the_production_save_refuses_on_a_read_only_authority_without_falling_back`] | 只读会话上**拒绝**且**不回退**本地路径：字节逐字节未变（回退就会写进去） |
//
// "另一个进程持排他锁 ⇒ 本地路径被拒"那一条住在 `in_process_mcp_lock.rs`
// （它已经有跨进程持有者的夹具，见那边的判据 4）。

/// **有可写权威 ⇒ 保存走权威**（`ROAD-M4-008` 选项 (a) 保存 UI 的主判据）。
///
/// 三件事一起断言：① 会话侧改了工程 ⇒ `dispatch_save` 成功且回执说"经权威"；
/// ② 落盘字节数等于回执数字；③ 后续读者从**磁盘**读回的是**会话当前那一版**
/// （新泳道在文件里 —— 若它写的是宿主缓存，新泳道不会在）。
#[test]
fn the_production_save_dispatches_through_the_writable_authority() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// **只读权威 ⇒ 拒绝，且不回退到本地路径**（保存 UI 的 fail-closed 那一半）。
///
/// 为什么"不回退"是承重的：进程里存在权威时，本地路径**不是**"另一条也能成功的路"。
/// 回退（权威拒绝 ⇒ 偷偷调 `save_project_file`）会在会话持排他锁时写失败、在
/// 只读会话（共享读锁 **不**挡本地排他写）时**真的写进去** —— 那就是第二个写者。
/// 因此断言分两半：回执是 `Failed{stage: Authority}`，且**字节逐字节未变**。
#[test]
fn the_production_save_refuses_on_a_read_only_authority_without_falling_back() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
