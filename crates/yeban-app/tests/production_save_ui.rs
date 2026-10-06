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
    // ---- 1. 无头 Tier-1 平台 + 生产构造入口 + 生产保存接线 ----
    let tier1 = Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let scratch = Scratch::new("window");
    let project_path = scratch.project("mounted.yeban");
    let fixture_bytes = std::fs::read(&project_path).expect("读夹具字节");
    let before = open_project_file(&project_path).expect("读夹具");
    let before_view = ViewState::from_project(&before).expect("夹具投影");
    let lead = before_view.tracks.first().expect("样本工程必须有轨道");
    let lead_id = lead.id.clone();
    let lane_id = format!("track-{}-automation-pan-lane", lead.index);
    assert!(
        !lane_element_ids(&before).contains(&lane_id),
        "起点：夹具里不该有 `{lane_id}`"
    );

    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::WritableFile(project_path.clone()),
    )
    .expect("挂载决策")
    .expect("必须挂载");
    let authority = mount.project_authority();
    let bearer = format!("Bearer {}", mount.token().expose());

    let authority_view =
        ViewState::from_project(&authority.project().expect("权威有活跃工程")).expect("权威投影");
    let scene = DemoScene::from_view(&authority_view);
    let ui = host::build_main_window(&authority_view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");

    // 生产装配（与 `run_gui` 同一条）：目标路径来自 `--open` 的那个文件。
    host::wire_save(
        &ui,
        host::SaveStatus {
            target: Some(project_path.clone()),
            project: authority.project().expect("权威有活跃工程"),
        },
        Some(authority.clone()),
    );
    assert_eq!(
        ui.get_save_status().to_string(),
        "",
        "还没按过保存 ⇒ 状态文本必须是空的（不是'已保存'）"
    );

    // ---- 2. 会话侧改工程，且**不**重投影 ⇒ 窗口画的是旧那一版 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        &format!(
            r#"{{"jsonrpc":"2.0","id":41,"method":"tools/call","params":{{"name":"yeban_edit_automation","arguments":{{"trackId":"{lead_id}","lane":"TrackPan","point":{{"tick":0,"value":0.25,"curve":"Linear"}}}}}}}}"#
        ),
    );
    assert_eq!(
        reply["result"]["status"], "success",
        "写类工具必须成功: {reply}"
    );
    assert!(
        lane_element_ids(&authority.project().expect("权威有活跃工程")).contains(&lane_id),
        "权威工程里必须已经有 `{lane_id}`"
    );

    // ---- 3. 生产保存入口：`.slint` 的回调 ⇒ 写权威当前那一版 ----
    let revision_before_save = authority.apply_revision();
    ui.invoke_save_project();
    let status = ui.get_save_status().to_string();
    assert!(
        ui.get_save_succeeded(),
        "生产保存入口必须成功（否则它落到了被排他锁拒的本地路径上）: {status}"
    );
    assert!(
        status.contains("经控制面会话写盘"),
        "状态文本必须点名这条保存走了控制面会话（用户看到的原话）: {status}"
    );
    assert!(
        status.contains(&project_path.display().to_string()),
        "状态文本必须点名写到哪个文件: {status}"
    );
    let written = std::fs::metadata(&project_path).expect("文件在").len();
    assert!(written > 0, "写出的容器不能是 0 字节");
    assert_ne!(
        std::fs::read(&project_path).expect("读回"),
        fixture_bytes,
        "保存过之后磁盘内容必须真的换了（不是原样没动）"
    );
    let saved = open_project_file(&project_path).expect("后续读者读回");
    assert!(
        lane_element_ids(&saved).contains(&lane_id),
        "窗口的保存必须写出**权威当前那一版**（`{lane_id}` 必须能被后续读者读回）: {:?}",
        lane_element_ids(&saved)
    );

    // ---- 3b. 保存**不**改工程内容 ⇒ 施加修订号一位不动（推进点仍只有 `apply` 一处）----
    assert_eq!(
        authority.apply_revision(),
        revision_before_save,
        "保存不得推进 `apply_revision`（它不改工程内容）"
    );
    ui.invoke_save_project();
    assert!(
        ui.get_save_succeeded(),
        "第二次保存也必须成功（同一个会话可以反复写盘）"
    );
    assert_eq!(
        authority.apply_revision(),
        revision_before_save,
        "第二次保存同样不得推进 `apply_revision`"
    );

    // ---- 4. 没有控制面时同一个入口走本地路径（产品路径两条分支都可达）----
    let local_path = scratch.join("local.yeban");
    let request = SaveRequest::new(local_path.clone(), before);
    let outcome = dispatch_save(&request, None);
    let SaveOutcome::Saved {
        bytes,
        by_authority,
        ..
    } = &outcome
    else {
        panic!("没有控制面时生产保存入口必须成功，实际: {outcome:?}");
    };
    assert!(!by_authority, "没有控制面时它必须走本地原子写");
    assert_eq!(
        std::fs::metadata(&local_path).expect("本地文件在").len(),
        *bytes as u64,
        "本地那一次的字节数必须与回执一致"
    );

    mount.stop().expect("停机");

    // 一条人可读的读数（CI 日志里能直接看到这次保存写了多少字节），
    // 顺带证明这个窗口仍然是一棵能光栅化的活窗口（不是已经拆掉的对象）。
    report_line(&format!(
        "[app] 生产保存入口: 权威 {} 字节 → {}；本地 {} 字节 → {}（窗口 {:?}）",
        written,
        project_path.display(),
        bytes,
        local_path.display(),
        tier1.size()
    ));
}
