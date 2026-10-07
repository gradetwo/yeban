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
use std::io::Write as _;
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
        // 「标记存在」= 「标记完整」这件事由**写入方**保证
        // （见 `write_handshake_atomically`）：子进程用同目录临时文件 + `rename`
        // 落盘，所以这条 `exists()` 判据的声明语义在源头为真。
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

/// 握手标记的**常量**权限：这是 `fs::write` 在这份夹具上的既有口径
/// （`0o666 & !umask`；本机 umask 022 ⇒ `0o644`）。它在**创建时**就定死，
/// 因此临时文件不会先以更宽（或更窄）的权限出现。
#[cfg(unix)]
const HANDSHAKE_FILE_MODE: u32 = 0o644;

/// 把子进程那次 `yeban_open_project` 的**真实响应**原子地写成握手标记：
/// 同目录临时文件 → 常量权限 → `sync_all` → `rename` 覆盖目标。
///
/// 为什么必须原子（真跑复现的竞态，不是猜测）：`fs::write` 等价于
/// `File::create` + `write_all`。`File::create` 一返回，文件**就已经存在**，
/// 但内容还是空的。父进程的轮询判据是 `exists()`（见 `ChildHolder::spawn`），
/// 于是它可能在子进程被抢占到 `write_all` 之前就读到空文件 ⇒ `serde_json` 报
/// `EOF while parsing a value at line 1 column 0`（30 次
/// `cargo test -p yeban-app --features in-process-mcp --test in_process_mcp_lock`
/// 里复现 1 次；36 并发 × 15 轮 = 540 次执行里复现 462 次）。
/// 同目录 `rename(2)` 是原子的 ⇒ 「标记存在」就等于「标记完整」，父进程那条
/// `exists()` 判据因此才成立（先例: `[ARCH-SEC-004]` 的
/// `crates/yeban-mcp/src/domain/store.rs` `write_then_replace`、
/// `crates/yeban-mcp/src/security.rs` 的令牌文件，以及
/// `crates/yeban-mcp/tests/lock_advisory.rs` 的同名修法）。
///
/// **不重试、不放宽**：写失败照样 `expect` 报错；写出的内容不合法，
/// 父进程那条 `serde_json` 断言照样立刻红。
fn write_handshake_atomically(ready: &Path, value: &Value) -> std::io::Result<()> {
    // 临时文件与目标**同目录**：跨目录 `rename` 不是原子替换，还可能 `EXDEV`。
    let mut temp = ready.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let text = serde_json::to_string(value).expect("序列化子进程响应");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(HANDSHAKE_FILE_MODE);
    }
    let outcome = options
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temp, ready));
    if outcome.is_err() {
        let _ = fs::remove_file(&temp);
    }
    outcome
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
    // 原子落盘是**父进程那条 `exists()` 轮询判据**成立的前提
    // （理由见 `write_handshake_atomically` 的注释）。
    write_handshake_atomically(&ready, &value).expect("写握手标记");

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

// ---------------------------------------------------------------------------
// ③ GUI 的**保存路径**与别的形态争同一把锁（`ROAD-M4-008` 选项 (a) 第三片）
// ---------------------------------------------------------------------------

/// `ROAD-M4-008` 选项 (a) 第三片的方向一（**跨进程**）：别的形态排他持锁时，
/// app 的**保存路径**必须拒绝写入 —— 两条都测：
///
/// | # | 保存路径 | 与 UI 的关系 |
/// | :--- | :--- | :--- |
/// | 1 | [`yeban_app::save::save_project_file`] | `ui/force_save` 调的就是它（`src/live_surface.rs`） |
/// | 1b | [`yeban_app::save_action::dispatch_save`]（无权威） | **生产**窗口的保存按钮调的就是它（`crate::host::wire_save`，见判据 ④） |
/// | 2 | 真二进制 `yeban-app --save-as <path>` | 命令行保存路径（默认构建里就存在） |
///
/// 四步各自可失败：
/// 1. 子进程（真 `Command` + 文件握手）以**排他写**打开同一工程 ⇒ 它真的持有那把锁；
/// 2. 两条保存路径都被拒（`SaveError::Locked` / 退出码 4），工程字节**逐字节未变**；
/// 3. `SIGKILL` 掉持有者（内核释放建议锁）⇒ 同一条保存**必须成功**；
/// 4. 成功的那一次换掉了内容，且不留残留 `.yeban.lock`（排他持有者 `Drop` 收走）。
#[test]
fn the_gui_save_paths_are_refused_while_another_process_holds_the_project_exclusively() {
    use yeban_app::save::{SaveError, save_project_file};

    let scratch = Scratch::new("gui-save");
    let project = scratch.project("demo.yeban");
    let ready = scratch.join("holder.json");

    // ---- 1. 另一个进程（真 Command）排他持有 ----
    let holder = ChildHolder::spawn_holding(&project, &ready, true);
    let before = fs::read(&project).expect("读持有者打开前后的字节");

    // ---- 2a. `ui/force_save` 的落点：`save_project_file` 必须被拒 ----
    let error = save_project_file(&yeban_model::samples::filled_project(), &project)
        .expect_err("持锁时 app 侧保存必须被拒");
    match &error {
        SaveError::Locked(hold) => {
            assert_eq!(hold.path, project, "拒绝必须点名目标工程");
            assert_eq!(hold.lock_file, lock_path(&project), "拒绝必须点名锁文件");
            assert_eq!(
                hold.holder_metadata, "available",
                "Unix 上建议锁是**建议**的 ⇒ 加锁前的快照必须读到持有者元数据: {error}"
            );
        }
        other => panic!("必须是 SaveError::Locked, 实际: {other:?}"),
    }
    assert_eq!(
        fs::read(&project).expect("旧文件仍在"),
        before,
        "被拒绝的保存绝不能碰工程文件"
    );

    // ---- 2a-bis. **生产保存入口**（无权威）也必须被同一把锁拒，且原因可读 ----
    //
    // 这一条钉住 `ROAD-M4-008` 选项 (a) 的"保存 UI"缺口：产品的保存按钮走
    // `save_action::dispatch_save`，而**没有控制面**时它的落点就是上面那条本地路径。
    // 因此"另一个形态正开着这份工程"必须在**用户看到的那句话**里说出来，而不是静默失败。
    let request = yeban_app::save_action::SaveRequest::new(
        project.clone(),
        yeban_model::samples::filled_project(),
    );
    let outcome = yeban_app::save_action::dispatch_save(&request, None);
    let yeban_app::save_action::SaveOutcome::Failed { stage, message } = &outcome else {
        panic!("持锁时生产保存入口必须被拒，实际: {outcome:?}");
    };
    assert_eq!(
        *stage,
        yeban_app::save_action::SaveStage::Local,
        "没有权威时它走的必须是本地路径: {message}"
    );
    assert!(
        message.contains("被 `.yeban.lock` 建议锁占用") && message.contains("拒绝写入"),
        "拒绝原因必须点名那把锁（用户要能读懂为什么没存上）: {message}"
    );
    assert!(
        message.contains("持有者") && message.contains("模式"),
        "拒绝原因必须带上持有者与锁模式（否则'谁占着'查不出来）: {message}"
    );
    assert_eq!(
        fs::read(&project).expect("旧文件仍在"),
        before,
        "被拒绝的生产保存绝不能碰工程文件"
    );

    // ---- 2b. 真二进制 `--save-as` 也必须被拒（退出码 4）----
    let bin = env!("CARGO_BIN_EXE_yeban-app");
    let blocked = Command::new(bin)
        .args(["--save-as".to_owned(), project.display().to_string()])
        .output()
        .expect("跑 yeban-app --save-as");
    assert_eq!(
        blocked.status.code(),
        Some(4),
        "被持锁时 --save-as 必须退出 4; stderr={}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("拒绝写入"),
        "拒绝的原因必须写在 stderr 里: {}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    assert_eq!(
        fs::read(&project).expect("旧文件仍在"),
        before,
        "被拒绝的 `--save-as` 绝不能碰工程文件"
    );

    // ---- 3. 杀掉持有者（SIGKILL ⇒ 内核释放 flock）⇒ 保存必须成功 ----
    drop(holder);
    let report = save_project_file(&yeban_model::samples::filled_project(), &project)
        .expect("持有者消失后保存必须成功");
    assert!(report.bytes > 0, "保存出的容器不能是 0 字节");

    // ---- 4. 内容真的换了，且不留残留锁 ----
    assert_ne!(
        fs::read(&project).expect("读回"),
        before,
        "成功的那一次必须真的换了内容（`history.dag` 空字节 vs 子进程写的图谱）"
    );
    assert!(
        !lock_path(&project).exists(),
        "成功的排他保存必须收走自己的锁文件: {}",
        lock_path(&project).display()
    );
}

/// `ROAD-M4-008` 选项 (a) 第三片的方向二（**同进程**，也是"为什么还不能翻 `read_only`"
/// 的**实测**）：只读控制面会话正持 [`LockMode::SharedRead`] 时，app 的保存路径
/// （`ui/force_save` 的落点）**拿不到**排他写建议锁 ⇒ 保存被**拒绝**（fail-closed），
/// 而不是产生第二个写者。
///
/// 这条是**登记事实**的判据，不是"想要的结局"：它把"同一个进程里，长命的共享读者
/// 与短命的排他写者互斥"变成可失败的断言。要让它变成"能存"，前提是让**一个**持有者
/// （会话）成为唯一写者 —— 那需要新的宿主保存动作，见
/// `docs/ledger/m4-008-authority-notes.md` §7.4。
///
/// 三步：
/// 1. 挂载（`SessionSource::File` ⇒ `SharedRead`）⇒ 保存被拒且字节未变；
/// 2. 停机（RAII 释放）⇒ 同一条保存必须成功；
/// 3. 成功之后内容真的换了，且锁文件被收走。
#[test]
fn a_mounted_read_only_session_refuses_the_gui_save_on_the_same_file() {
    use yeban_app::save::{SaveError, save_project_file};

    let scratch = Scratch::new("self-conflict");
    let project = scratch.project("demo.yeban");
    let before = fs::read(&project).expect("读原文");

    // ---- 1. 挂载：本进程的只读会话持共享读锁 ----
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

    let error = save_project_file(&yeban_model::samples::filled_project(), &project)
        .expect_err("本进程的共享读者持锁时, 排他写保存必须被拒（fail-closed）");
    assert!(
        matches!(error, SaveError::Locked(_)),
        "必须是 SaveError::Locked, 实际: {error:?}"
    );
    assert_eq!(
        fs::read(&project).expect("旧文件仍在"),
        before,
        "被拒绝的保存绝不能碰工程文件"
    );

    // ---- 2. 停机 ⇒ 锁释放 ⇒ 同一条保存必须成功 ----
    mount.stop().expect("停机");
    let report = save_project_file(&yeban_model::samples::filled_project(), &project)
        .expect("会话停机后保存必须成功");
    assert!(report.bytes > 0, "保存出的容器不能是 0 字节");

    // ---- 3. 内容真的换了，且不留残留锁 ----
    assert_ne!(
        fs::read(&project).expect("读回"),
        before,
        "成功的那一次必须真的换了内容（`history.dag` 空字节 vs 夹具写的图谱）"
    );
    assert!(
        !lock_path(&project).exists(),
        "成功的排他保存必须收走自己的锁文件: {}",
        lock_path(&project).display()
    );
}
