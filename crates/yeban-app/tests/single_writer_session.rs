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
//!
//! 运行：
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-app --features in-process-mcp --tests
//! ```
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
    let scratch = Scratch::new("writable");
    let project_path = scratch.project("demo.yeban");
    let fixture_bytes = std::fs::read(&project_path).expect("读夹具字节");

    // 起点：磁盘上的夹具**没有** pan 泳道（否则第 ③ 步的对照不成立）。
    let before = open_project_file(&project_path).expect("读夹具");
    let (lead, lead_index) = lead_track_of(&before);
    let lane_id = format!("track-{lead_index}-automation-pan-lane");
    assert!(
        !lane_element_ids(&before).contains(&lane_id),
        "起点：夹具里不该有 `{lane_id}`"
    );

    // ---- ① 写形态：排他写锁 + 会话可写 ----
    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::WritableFile(project_path.clone()),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    assert_eq!(
        mount.lock_mode(),
        Some(LockMode::ExclusiveWrite),
        "单一写者会话必须取排他写锁（否则第二个形态能共存 ⇒ 两个写者）"
    );
    assert_eq!(
        mount.lock_path(),
        Some(lock_path(&project_path).as_path()),
        "会话持有的必须是这个工程的锁文件"
    );
    let authority = mount.project_authority();
    assert!(
        authority.is_writable(),
        "单一写者会话的 Domain 不能是只读打开的"
    );
    let bearer = format!("Bearer {}", mount.token().expose());
    let address = mount.address();

    // ---- ② 第二个形态挂不上：读形态与写形态都必须被 PROJECT_LOCKED 挡住 ----
    for (tag, source) in [
        ("只读", SessionSource::File(project_path.clone())),
        ("写", SessionSource::WritableFile(project_path.clone())),
    ] {
        let error =
            InProcessMcp::start_for_project(true, yeban_model::samples::filled_project(), source)
                .expect_err(&format!("{tag}形态在写者存活时不得挂载"));
        match &error {
            MountError::Locked { path, fault } => {
                assert_eq!(path, &project_path, "报错必须点名目标工程");
                assert_eq!(
                    fault.domain_code().map(yeban_mcp::tools::ErrorCode::as_str),
                    Some("PROJECT_LOCKED"),
                    "锁竞争的**唯一**契约出口就是 PROJECT_LOCKED: {fault:?}"
                );
            }
            other => panic!("{tag}形态必须是 MountError::Locked（拒绝挂载），实际是 {other:?}"),
        }
    }

    // ---- ③ 会话侧改工程 ⇒ 宿主保存动作写盘 ⇒ 后续读者读回那一版 ----
    let reply = write_a_pan_lane(address, &bearer, &lead);
    assert_eq!(
        reply["result"]["status"], "success",
        "写类工具必须成功: {reply}"
    );
    assert_eq!(reply["result"]["data"]["applied"], true, "{reply}");

    let report = authority
        .save_to(&project_path)
        .expect("宿主保存动作必须成功（会话可写）");
    assert!(!report.skipped, "force = true ⇒ 必须真的落盘");
    assert!(report.bytes > 0, "写出的容器不能是 0 字节");
    assert_eq!(
        std::fs::metadata(&project_path).expect("文件在").len(),
        report.bytes as u64,
        "落盘字节数必须等于回执里的数字"
    );

    let saved = open_project_file(&project_path).expect("后续读者读回");
    assert!(
        lane_element_ids(&saved).contains(&lane_id),
        "保存必须**持久化**会话当前那一版（`{lane_id}` 必须能被后续读者读回）: {:?}",
        lane_element_ids(&saved)
    );

    // ---- 同一个会话的**两条入口**写出同一份字节（"一个写者"不是口号） ----
    let twin = scratch.join("twin.yeban");
    let twin_report = authority.save_to(&twin).expect("同一会话再存到另一个路径");
    assert_eq!(
        std::fs::read(&twin).expect("读 twin"),
        std::fs::read(&project_path).expect("读主路径"),
        "宿主保存动作两次写出的必须是**逐字节相同**的容器（同一个 save_material 口径）"
    );
    assert_eq!(twin_report.bytes, report.bytes);

    // ---- ④ 工具面 `yeban_save_project` 与宿主保存动作是同一个写会话 ----
    let reply = mcp_call(
        address,
        &bearer,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"yeban_save_project","arguments":{"force":true}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert_eq!(
        reply["result"]["data"]["saved"], true,
        "写会话的工具保存必须真的落盘（这就是 `read_only` 的翻转口径）: {reply}"
    );
    let after_tool = open_project_file(&project_path).expect("工具保存后读回");
    assert!(
        lane_element_ids(&after_tool).contains(&lane_id),
        "工具保存写出的仍是同一份权威工程"
    );
    assert_ne!(
        std::fs::read(&project_path).expect("读回"),
        fixture_bytes,
        "保存过之后磁盘内容必须真的换了（不是原样没动）"
    );

    // ---- 停机之后锁释放 ----
    mount.stop().expect("停机");
    assert!(
        !lock_path(&project_path).exists(),
        "排他持有者停机后必须收走锁文件: {}",
        lock_path(&project_path).display()
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
    let scratch = Scratch::new("local-refused");
    let project_path = scratch.project("demo.yeban");
    let before = std::fs::read(&project_path).expect("读原文");

    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::WritableFile(project_path.clone()),
    )
    .expect("挂载决策")
    .expect("必须挂载");

    // ---- 2a. `ui/force_save` 的落点：本地原子写必须被拒 ----
    let error = save_project_file(&yeban_model::samples::filled_project(), &project_path)
        .expect_err("写会话持排他锁时本地保存必须被拒");
    assert!(
        matches!(error, SaveError::Locked(_)),
        "必须是 SaveError::Locked, 实际: {error:?}"
    );
    assert_eq!(
        std::fs::read(&project_path).expect("旧文件仍在"),
        before,
        "被拒绝的本地保存绝不能碰工程文件"
    );

    // ---- 2b. 真二进制 `--save-as` 也必须被拒（退出码 4） ----
    let bin = env!("CARGO_BIN_EXE_yeban-app");
    let blocked = Command::new(bin)
        .args(["--save-as".to_owned(), project_path.display().to_string()])
        .output()
        .expect("跑 yeban-app --save-as");
    assert_eq!(
        blocked.status.code(),
        Some(4),
        "写会话持锁时 --save-as 必须退出 4; stderr={}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert_eq!(
        std::fs::read(&project_path).expect("旧文件仍在"),
        before,
        "被拒绝的 `--save-as` 绝不能碰工程文件"
    );

    // ---- 2c. 停机 ⇒ 本地路径必须成功（拒绝的原因只能是那把锁） ----
    mount.stop().expect("停机");
    let report = save_project_file(&yeban_model::samples::filled_project(), &project_path)
        .expect("会话停机后本地保存必须成功");
    assert!(report.bytes > 0);
    assert_ne!(
        std::fs::read(&project_path).expect("读回"),
        before,
        "成功的那一次必须真的换了内容"
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
    let scratch = Scratch::new("read-only-refused");
    let project_path = scratch.project("demo.yeban");
    let before = std::fs::read(&project_path).expect("读原文");

    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::File(project_path.clone()),
    )
    .expect("挂载决策")
    .expect("必须挂载");
    assert_eq!(mount.lock_mode(), Some(LockMode::SharedRead));
    let authority = mount.project_authority();
    assert!(!authority.is_writable(), "只读挂载的会话不能是可写的");

    let error = authority
        .save_to(&project_path)
        .expect_err("只读会话的宿主保存动作必须被拒");
    assert_eq!(
        error.domain_code().map(yeban_mcp::tools::ErrorCode::as_str),
        Some("IO_ERROR"),
        "只读会话落盘必须走与 `yeban_save_project` 同一个门: {error:?}"
    );
    assert_eq!(
        std::fs::read(&project_path).expect("旧文件仍在"),
        before,
        "被拒绝的宿主保存绝不能碰工程文件"
    );
    mount.stop().expect("停机");
}
