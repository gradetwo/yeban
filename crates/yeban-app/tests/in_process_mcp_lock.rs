//! `[ROAD-M0-007]` / `[MUST-GATE-008]`：app 进程内控制面与**别的形态**之间的
//! `.yeban.lock` **跨进程**互斥判据。
//!
//! ## 这条判据要证的句子
//!
//! > 当 app 以内嵌环回 HTTP 控制面（形态 A）打开一个**磁盘上的真工程**时，
//! > 那个会话**参与同一把 `.yeban.lock`**：另一个进程要**排他写**同一工程会被
//! > `PROJECT_LOCKED` 挡住；另一个进程只**读**则可以共存；控制面停机之后，
//! > 那把锁真的被释放。
//!
//! 反方向同样要证：**别的形态先持排他锁**时，app 侧控制面**拒绝挂载**
//! （而不是挂一个没有保护的口）。
//!
//! ## 为什么必须真的跨进程（而不是同进程两个 `Dispatcher`）
//!
//! `crates/yeban-mcp/tests/lock_advisory.rs` 已经把 OS 建议锁的跨进程语义钉死。
//! 本文件**不重复**那些断言 —— 它证的是**新接线的那一半**：`InProcessMcp`
//! 的 `SessionSource::File` 真的去取了那把锁，而且取的是**共享读**（只读会话）。
//! 因此这里用的手法与那一份**逐条对齐**：
//!
//! | 手法 | 为什么 |
//! | :--- | :--- |
//! | `std::process::Command` 重新执行**本测试二进制** | 子进程是"另一个进程的 yeban-mcp"，不是同进程的第二个对象 |
//! | **文件握手**（子进程把真实响应写成 JSON） | 同步靠"事情已经发生"，不靠 `sleep` 猜时间 |
//! | 有界等待（30s） | 坏回归是**响亮的失败**，不是挂到 CI 上限 |
//! | `Child::kill()` + `wait()` 收尸 | 不留僵尸；`SIGKILL` 不走 `Drop`，释放只可能来自内核 |
//!
//! ## 临时目录纪律
//!
//! 全部落在 `std::env::temp_dir()/<唯一子目录>`，`Scratch::drop` 负责删除 ——
//! **绝不污染仓库**（`docs/DEVELOPMENT_LEDGER.md` L16）。
//!
//! 运行：
//!
//! ```text
//! bash scripts/dev/cargo-local.sh test -p yeban-app --features in-process-mcp --tests
//! ```
#![cfg(feature = "in-process-mcp")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_app::mcp_mount::{InProcessMcp, MountError, SessionSource};
use yeban_mcp::Dispatcher;
use yeban_mcp::domain::store::{LockMode, lock_path};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 进程内唯一序号（同一测试进程里两个用例并行也各拿各的目录）。
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 子进程角色开关：本测试二进制被自己重新执行时用它区分"父"与"子"。
const CHILD_ROLE_ENV: &str = "YEBAN_APP_LOCK_CHILD";
/// 子进程要打开的工程路径。
const CHILD_PROJECT_ENV: &str = "YEBAN_APP_LOCK_PROJECT";
/// 子进程写"我开完了"握手标记的路径。
const CHILD_READY_ENV: &str = "YEBAN_APP_LOCK_READY";
/// 子进程的打开模式（`exclusive` / `shared`）。
const CHILD_MODE_ENV: &str = "YEBAN_APP_LOCK_MODE";

/// 一个临时目录，`Drop` 时清理。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-app-lock-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 写一份**确定的**真容器工程（与 `lock_advisory.rs` 用同一份夹具数据）。
    fn project(&self, name: &str) -> PathBuf {
        let project = yeban_model::samples::filled_project();
        let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱 JSON");
        let bytes =
            yeban_model::container::write_project_container(&project, &history, &BTreeMap::new())
                .expect("写真容器");
        let path = self.join(name);
        fs::write(&path, &bytes).expect("写容器工程");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// 跨进程：把本测试二进制当成"另一个进程的 yeban-mcp"
// ---------------------------------------------------------------------------

/// 父进程侧的跨进程持有者句柄。
struct ChildHolder {
    child: Child,
    /// 子进程那一次 `yeban_open_project` 的完整带内响应（握手文件内容）。
    response: Value,
}

impl ChildHolder {
    /// 启动一个**真的子进程**：它用真实的 `Dispatcher` 打开工程、拿到（或被拒绝）锁，
    /// 然后把自己的响应写成握手标记并停下等被杀。
    fn spawn(project: &Path, ready: &Path, exclusive: bool) -> Self {
        let exe = std::env::current_exe().expect("本测试二进制的路径");
        let mut child = Command::new(exe)
            // libtest 过滤器：只跑子进程角色那一个用例。
            .args([
                "lock_child_process_opens_the_project_and_holds_it",
                "--nocapture",
            ])
            .env(CHILD_ROLE_ENV, "1")
            .env(CHILD_PROJECT_ENV, project)
            .env(CHILD_READY_ENV, ready)
            .env(
                CHILD_MODE_ENV,
                if exclusive { "exclusive" } else { "shared" },
            )
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("启动子进程");
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "子进程 {pid} 30 秒内没有写握手标记 {}",
                ready.display()
            );
            assert!(
                child.try_wait().expect("try_wait").is_none(),
                "子进程 {pid} 在写握手标记之前就退出了"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let text = fs::read_to_string(ready).expect("读握手标记");
        let response: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("握手标记必须是 JSON: {error}: {text}"));
        Self { child, response }
    }

    /// 启动子进程并断言它**成功拿到锁**。
    fn spawn_holding(project: &Path, ready: &Path, exclusive: bool) -> Self {
        let holder = Self::spawn(project, ready, exclusive);
        assert_eq!(
            holder.response["status"], "success",
            "子进程必须成功打开（否则『另一个进程持有者』这个前提不成立）: {}",
            holder.response
        );
        holder
    }

    /// 启动子进程并断言它**被 `PROJECT_LOCKED` 拒绝**（跨形态互斥）。
    fn spawn_blocked(project: &Path, ready: &Path, exclusive: bool) -> Self {
        let holder = Self::spawn(project, ready, exclusive);
        assert_eq!(
            holder.response["status"], "error",
            "另一个进程必须拿到带内领域失败: {}",
            holder.response
        );
        assert_eq!(
            holder.response["error"]["code"], "PROJECT_LOCKED",
            "另一个进程必须拿到 PROJECT_LOCKED: {}",
            holder.response
        );
        assert_eq!(
            holder.response["error"]["data"]["advisoryLockHeld"], true,
            "载荷必须说明占用来自内核建议锁: {}",
            holder.response
        );
        holder
    }
}

impl Drop for ChildHolder {
    fn drop(&mut self) {
        // 收尸：不留僵尸。`kill()` 是 `SIGKILL` ⇒ 不走子进程的 `Drop`，
        // 因此"锁被释放"只可能来自**内核**在进程死亡时的清理。
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 子进程角色：拿锁 → 写握手标记 → 停下等被杀。
///
/// 父进程运行时它既是"另一个进程的 yeban-mcp"，也是一个 `#[test]`：
/// 直接被 `cargo test` 跑到时（没有环境变量）它**立刻返回**，不做任何事。
#[test]
fn lock_child_process_opens_the_project_and_holds_it() {
    let Ok(role) = std::env::var(CHILD_ROLE_ENV) else {
        // 不是子进程角色：正常测试跑里这是一个空判据。
        return;
    };
    assert_eq!(role, "1");
    let project = PathBuf::from(std::env::var(CHILD_PROJECT_ENV).expect("子进程需要工程路径"));
    let ready = PathBuf::from(std::env::var(CHILD_READY_ENV).expect("子进程需要握手标记路径"));
    let exclusive = std::env::var(CHILD_MODE_ENV).expect("子进程需要模式") == "exclusive";

    // 与 `lock_advisory.rs` 的 `dispatcher()` 同一构造：全量 scope、生产模式。
    // 这里**不是**在测鉴权，因此直接喂一个真令牌。
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);

    let line = json!({
        "jsonrpc": "2.0",
        "id": "child",
        "method": "tools/call",
        "params": {
            "name": "yeban_open_project",
            "arguments": {
                "path": project.display().to_string(),
                "readOnly": !exclusive,
            }
        }
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth.as_str()), &line);
    assert_eq!(
        outcome.http_status, 200,
        "领域失败必须走带内 ToolResponse（不是协议层错误）"
    );
    let response = outcome.response.expect("必须有响应");
    let value = response
        .result
        .unwrap_or_else(|| panic!("不该有 JSON-RPC 层错误: {:?}", response.error));
    // 无论成功还是 `PROJECT_LOCKED`，都把**真实响应**写进握手文件：
    // 父进程据此断言"另一个进程到底看到了什么"，而不是靠猜。
    fs::write(
        &ready,
        serde_json::to_string(&value).expect("序列化子进程响应"),
    )
    .expect("写握手标记");

    // 停下来等父进程杀死自己（真崩溃 ⇒ 只可能由内核释放建议锁）。
    std::thread::sleep(Duration::from_secs(60));
}

// ---------------------------------------------------------------------------
// ① 别的形态先持排他锁 ⇒ app 侧控制面拒绝挂载
// ---------------------------------------------------------------------------

/// `ROAD-M0-007` 的方向一：**拒绝挂载**。
///
/// 另一个进程排他持锁时，`SessionSource::File` 的挂载必须 `Err(MountError::Locked)`
/// —— 不是"挂上去但悄悄不取锁"。拿掉持有者之后**同一份挂载必须成功**，
/// 且模式是 `SharedRead`：这证明拒绝的原因就是那把锁，而不是别的偶发失败。
#[test]
fn a_mount_is_refused_while_another_form_holds_the_project_exclusively() {
    let scratch = Scratch::new("refuse");
    let project = scratch.project("demo.yeban");
    let ready = scratch.join("holder.json");

    // ---- 另一个进程（真 Command）以排他写打开并持锁 ----
    let holder = ChildHolder::spawn_holding(&project, &ready, true);
    assert!(
        lock_path(&project).exists(),
        "排他持有者必须真的建出了 {}",
        lock_path(&project).display()
    );

    // ---- 被占用时必须拒绝挂载 ----
    let error = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::File(project.clone()),
    )
    .expect_err("被排他持有时挂载必须失败");
    match &error {
        MountError::Locked { path, fault } => {
            assert_eq!(path, &project, "报错必须点名目标工程");
            assert_eq!(
                fault.domain_code().map(yeban_mcp::tools::ErrorCode::as_str),
                Some("PROJECT_LOCKED"),
                "锁竞争的**唯一**契约出口就是 PROJECT_LOCKED: {fault:?}"
            );
        }
        other => panic!("必须是 MountError::Locked（拒绝挂载），实际是 {other:?}"),
    }
    // 失败必须**响亮**：错误文本要能被人读懂是"别的形态占着"。
    let text = error.to_string();
    assert!(
        text.contains("别的形态") && text.contains("PROJECT_LOCKED"),
        "拒绝的原因必须写在错误文本里: {text}"
    );

    // ---- 拿掉持有者（SIGKILL + 收尸）⇒ 同一份挂载必须成功 ----
    drop(holder);
    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::File(project.clone()),
    )
    .expect("持有者消失后挂载决策")
    .expect("持有者消失后必须能挂载");
    assert_eq!(
        mount.lock_mode(),
        Some(LockMode::SharedRead),
        "只读会话取的必须是共享读锁（不是排他，也不是不取）"
    );
    assert_eq!(
        mount.lock_path(),
        Some(lock_path(&project).as_path()),
        "会话持有的必须是这个工程的锁文件"
    );
    mount.stop().expect("停机");

    // ---- 会话来源是纯粹的内存样本 ⇒ 没有锁文件 ----
    let sample = scratch.join("never-a-file");
    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::InMemory(PathBuf::from("sample:filled")),
    )
    .expect("内存会话挂载决策")
    .expect("内存会话必须能挂载");
    assert_eq!(mount.lock_mode(), None, "内存会话没有锁可取");
    assert_eq!(mount.lock_path(), None, "内存会话不该有锁路径");
    mount.stop().expect("停机");
    assert!(
        !lock_path(&sample).exists(),
        "内存会话绝不可以在磁盘上造锁文件: {}",
        lock_path(&sample).display()
    );
}

// ---------------------------------------------------------------------------
// ② app 侧控制面持共享读锁 ⇒ 别的形态的写者被挡、读者共存、停机释放
// ---------------------------------------------------------------------------

/// `ROAD-M0-007` 的方向二：**跨形态互斥真的生效**。
///
/// 四步各自是一条可失败的断言：
/// 1. 挂载出来的会话持有 `SharedRead`（观测得到，不是声明）；
/// 2. 另一个进程要**写** ⇒ `PROJECT_LOCKED`（这就是"跨形态互斥"）；
/// 3. 另一个进程只**读** ⇒ 成功（共享读锁的语义，证明取的不是排他锁）；
/// 4. 读者退出 + 控制面停机 ⇒ 写者不再被挡（RAII 释放，锁挂在挂载生命周期上）。
#[test]
fn the_mounted_control_plane_excludes_writers_and_shares_with_readers() {
    let scratch = Scratch::new("exclude");
    let project = scratch.project("demo.yeban");

    // ---- 1. 挂载（取共享读锁）----
    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::File(project.clone()),
    )
    .expect("挂载决策")
    .expect("开关打开时必须真的挂载");
    assert_eq!(
        mount.lock_mode(),
        Some(LockMode::SharedRead),
        "只读控制面必须是共享读者"
    );

    // ---- 2. 另一个进程要写 ⇒ 被挡 ----
    let writer_ready = scratch.join("writer.json");
    let writer = ChildHolder::spawn_blocked(&project, &writer_ready, true);

    // ---- 3. 另一个进程只读 ⇒ 共存 ----
    let reader_ready = scratch.join("reader.json");
    let reader = ChildHolder::spawn_holding(&project, &reader_ready, false);

    // ---- 4. 两个共享读者都退出（先读者、后控制面）⇒ 写者不再被挡 ----
    drop(reader);
    mount.stop().expect("停机");
    let after_ready = scratch.join("after.json");
    let after = ChildHolder::spawn_holding(&project, &after_ready, true);
    drop(after);
    drop(writer);
}
