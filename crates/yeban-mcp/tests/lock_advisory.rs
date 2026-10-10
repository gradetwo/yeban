//! **`MUST-GATE-008` 判据**：OS 级建议锁（`.yeban.lock`）[ARCH-SEC-001]。
//!
//! 这个文件是"**打开工程时必须成功施加 OS 建议锁并原子创建锁文件，
//! 并发读写立即抛出 `PROJECT_LOCKED`**"的判决书。它与 `tools_e2e.rs` 的分工：
//!
//! - `tools_e2e.rs` 判**契约形状**（错误码、响应字段、幂等、字节不变）；
//! - 本文件判**内核语义**：锁到底是不是 OS 建议锁、"文件存在但无人持有"
//!   能不能接管、跨进程是不是真的被拦住、进程崩溃后锁是不是真的自动释放。
//!
//! ## 三类场景的真实制造方式（不是"看起来对"）
//!
//! | 场景 | 怎么制造 | 判据 |
//! | :--- | :--- | :--- |
//! | 同进程两个打开者 | 两个 `Domain`（两个独立 `File` 句柄）打开同一路径 | ②③ |
//! | **跨进程** | `std::process::Command` 重新执行**本测试二进制**，子进程用真实的 `Dispatcher` 打开工程；用**文件握手**（不是 `sleep` 猜时间）同步 | ⑨⑩⑪ |
//! | 崩溃遗留陈旧锁 | 子进程持锁后被 `SIGKILL`（真崩溃，不走 `Drop`）；锁文件仍在磁盘上 | ⑫⑬⑭ |
//!
//! ## 判据清单（14 条）
//!
//! | # | 判据 | 断言的内核/契约性质 |
//! | :--- | :--- | :--- |
//! | ① | `lock_path` 命名 | `<工程文件名>.lock` |
//! | ② | 同进程第二个 `Domain` | `PROJECT_LOCKED` |
//! | ③ | 独占期间**只读**打开 | 也被 `PROJECT_LOCKED`（读者不得绕开写者） |
//! | ④ | 排他 `Drop` 后 | 锁文件被删除，下一个打开者成功且**不是接管** |
//! | ⑤ | **仅凭文件存在不算被占用** | 无持有者的锁文件 ⇒ 打开成功 + `tookOverStaleLock` |
//! | ⑥ | 接管会**重写**锁内容 | PID/时间戳/工程路径被换成新持有者 |
//! | ⑦ | 锁内容损坏（半截 JSON） | 仍可接管（元数据不参与判定） |
//! | ⑧ | 共享读 + 共享读 | 两个读者共存，**都**拿到 `SharedRead` |
//! | ⑨ | 读 + 写 | 写者被 `PROJECT_LOCKED` |
//! | ⑩ | 跨进程独占 | 子进程 `PROJECT_LOCKED`（真的另一个 PID） |
//! | ⑪ | 跨进程共享读共存 | 父读者 + 子读者都成功 |
//! | ⑫ | 崩溃自愈 | 子进程 `SIGKILL` 后父进程立刻能拿锁 |
//! | ⑬ | 崩溃后的表现 | 锁文件仍在，但 `tookOverStaleLock = true` |
//! | ⑭ | 契约码 | 锁占用**只**报 `PROJECT_LOCKED`（不发明新码） |
//!
//! 未覆盖（**pending，不假装**）：Windows 分支从未在本机编译过（P1）、
//! 非 Unix/非 Windows 的 `UnsupportedPlatform` **只有 cfg 注入判据没有真机**（P3）。
//!
//! ## 临时目录纪律
//!
//! 全部落在 `std::env::temp_dir()/<唯一子目录>`，`Scratch::drop` 负责删除 ——
//! **绝不污染仓库**（`docs/DEVELOPMENT_LEDGER.md` L16）。
//! 子进程继承同一目录；父进程 `Child::kill()` 后一定 `wait()` 收尸，不留僵尸。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::lock::{lock_path, read_metadata};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::ErrorCode;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 进程内唯一序号。
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 子进程角色开关：本测试二进制被自己重新执行时用它区分"父"与"子"。
const CHILD_ROLE_ENV: &str = "YEBAN_LOCK_TEST_CHILD";
/// 子进程要打开的工程路径。
const CHILD_PROJECT_ENV: &str = "YEBAN_LOCK_TEST_PROJECT";
/// 子进程写"我已持锁"握手标记的路径。
const CHILD_READY_ENV: &str = "YEBAN_LOCK_TEST_READY";
/// 子进程的打开模式（`exclusive` / `shared`）。
const CHILD_MODE_ENV: &str = "YEBAN_LOCK_TEST_MODE";

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
            "yeban-lock-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 写一份**确定的**工程文件（`tools_e2e.rs` 用同一份夹具数据）。
    ///
    /// `ADR-0001 D43`：容器是唯一工程格式，因此夹具写的是**真容器**字节
    /// （旧夹具写裸 JSON，那条读路径已删除）。
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

/// 测试用分发器（全量 scope、生产模式）。
fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 走 `tools/call` 并断言这是**带内** `ToolResponse`。
fn call_tool(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "l",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    assert_eq!(outcome.http_status, 200, "领域失败必须走带内 ToolResponse");
    let response = outcome.response.expect("必须有响应");
    response
        .result
        .unwrap_or_else(|| panic!("不该有 JSON-RPC 层错误: {:?}", response.error))
}

/// `yeban_open_project`（默认独占写）。
fn call(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> Value {
    call_tool(dispatcher, auth, "yeban_open_project", arguments)
}

/// 打开一个工程（默认独占写）。
fn open(dispatcher: &mut Dispatcher, auth: &str, path: &Path, read_only: bool) -> Value {
    call(
        dispatcher,
        auth,
        json!({ "path": path.display().to_string(), "readOnly": read_only }),
    )
}

/// 断言响应是 `PROJECT_LOCKED` 领域失败。
fn assert_locked(dispatcher: &mut Dispatcher, auth: &str, path: &Path, context: &str) {
    let value = open(dispatcher, auth, path, false);
    assert_eq!(value["status"], "error", "{context}: {value}");
    assert_eq!(
        value["error"]["code"], "PROJECT_LOCKED",
        "{context}: {value}"
    );
    assert_eq!(
        value["error"]["data"]["advisoryLockHeld"], true,
        "{context}: 载荷必须说明占用来自内核建议锁: {value}"
    );
}

// ---------------------------------------------------------------------------
// 跨进程：把本测试二进制当成"另一个进程的 yeban-mcp"
// ---------------------------------------------------------------------------

/// 父进程侧的跨进程持有者句柄。
struct ChildHolder {
    child: Child,
    pid: u32,
    /// 子进程自己那次 `yeban_open_project` 的完整响应（握手文件内容）。
    response: Value,
}

impl ChildHolder {
    /// 启动一个**真的子进程**：它用真实的 `Dispatcher` 打开工程、拿到锁，
    /// 然后把自己的响应写成"握手标记"文件并停下等被杀。
    ///
    /// 同步方式刻意**不是 `sleep` 猜时间**：父进程轮询标记文件，
    /// 标记出现 ⇒ 子进程那一次打开已经**执行完成**（拿到锁或拿到 `PROJECT_LOCKED`）。
    fn spawn(project: &Path, ready: &Path, exclusive: bool) -> Self {
        let exe = std::env::current_exe().expect("本测试二进制的路径");
        let mut child = Command::new(exe)
            // libtest 过滤器：只跑子进程角色那一个用例。
            .args(["cross_process_child_holder", "--nocapture"])
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
        // 这里的判据是"标记**完整地**存在"：子进程用同目录 `rename` 原子落盘
        // （见 `write_handshake_atomically`），所以 `exists()` 为真就等于内容已完整。
        // 若写入方改回非原子写，这里就会读到空文件/半截文件 —— 那是写入方的缺陷，
        // 本判据**不**用重试去掩盖它（`serde_json` 那条断言会立刻红）。
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
        Self {
            child,
            pid,
            response,
        }
    }

    /// 启动子进程并断言它**成功拿到锁**（跨进程占用由子进程制造）。
    fn spawn_holding(project: &Path, ready: &Path, exclusive: bool) -> Self {
        let holder = Self::spawn(project, ready, exclusive);
        assert_eq!(
            holder.response["status"], "success",
            "子进程必须成功打开（否则『另一个进程持有者』这个前提不成立）: {}",
            holder.response
        );
        holder
    }

    /// 启动子进程并断言它**被 `PROJECT_LOCKED` 拒绝**（跨进程互斥）。
    fn spawn_blocked(project: &Path, ready: &Path, exclusive: bool) -> Self {
        let holder = Self::spawn(project, ready, exclusive);
        assert_eq!(
            holder.response["error"]["code"], "PROJECT_LOCKED",
            "另一个进程必须拿到 PROJECT_LOCKED: {}",
            holder.response
        );
        assert_eq!(
            holder.response["error"]["data"]["advisoryLockHeld"], true,
            "{}",
            holder.response
        );
        holder
    }
}

impl Drop for ChildHolder {
    fn drop(&mut self) {
        // 收尸: 不留僵尸 (CI 上尤其重要)。
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 把子进程那次 `yeban_open_project` 的**真实响应**原子地写成握手标记：
/// 同目录临时文件 → `rename`。
///
/// 为什么必须原子（真跑复现的竞态，不是猜测）：`fs::write` 等价于
/// `File::create` + `write_all`。`File::create` 一返回，文件**就已经存在**，
/// 但内容还是空的。父进程的轮询判据是 `exists()`，于是它可能在子进程被抢占到
/// `write_all` 之前就读到空文件 ⇒ `serde_json` 报
/// `EOF while parsing a value at line 1 column 0`。
/// 同目录 `rename(2)` 是原子的 ⇒ "标记存在"就等于"标记完整"，
/// 父进程那条 `exists()` 判据因此才成立（先例: `[ARCH-SEC-004]` 的
/// `crates/yeban-mcp/src/domain/store.rs` `write_then_replace` 与
/// `crates/yeban-app/src/save.rs`）。
///
/// **不重试、不放宽**：写失败照样 `expect` 报错；写出的内容不合法，
/// 父进程那条 `serde_json` 断言照样立刻红。
fn write_handshake_atomically(ready: &Path, value: &Value) -> std::io::Result<()> {
    // 临时文件与目标**同目录**：跨目录 `rename` 不是原子替换，还可能 `EXDEV`。
    let mut temp = ready.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let text = serde_json::to_string(value).expect("序列化子进程响应");
    let outcome = fs::write(&temp, text).and_then(|()| fs::rename(&temp, ready));
    if outcome.is_err() {
        let _ = fs::remove_file(&temp);
    }
    outcome
}

/// 子进程角色：拿锁 → 写握手标记 → 停下等被杀。
///
/// 父进程运行时它既是一个"另一进程的 yeban-mcp"，也是一个 `#[test]`：
/// 直接被 `cargo test` 跑到时（没有环境变量）它**立刻返回**，不做任何事，
/// 因此不会在正常测试跑里卡住。
#[test]
fn cross_process_child_holder() {
    let Ok(role) = std::env::var(CHILD_ROLE_ENV) else {
        // 不是子进程角色: 正常测试跑里这是一个空判据。
        return;
    };
    assert_eq!(role, "1");
    let project = PathBuf::from(std::env::var(CHILD_PROJECT_ENV).expect("子进程需要工程路径"));
    let ready = PathBuf::from(std::env::var(CHILD_READY_ENV).expect("子进程需要握手标记路径"));
    let exclusive = std::env::var(CHILD_MODE_ENV).expect("子进程需要模式") == "exclusive";

    let (mut dispatcher, auth) = dispatcher();
    let value = open(&mut dispatcher, &auth, &project, !exclusive);
    // 无论成功还是 `PROJECT_LOCKED`，都把**真实响应**写进握手文件：
    // 父进程据此断言"另一个进程到底看到了什么"，而不是靠猜。
    // 原子落盘是**父进程那条 `exists()` 轮询判据**成立的前提
    // （理由见 `write_handshake_atomically` 的注释）。
    write_handshake_atomically(&ready, &value).expect("写握手标记");

    // 停下来等父进程杀死自己。真崩溃（SIGKILL）不会走 `Drop`，
    // 因此"锁被释放"只可能来自**内核**在进程死亡时的清理。
    std::thread::sleep(Duration::from_secs(60));
}

// ---------------------------------------------------------------------------
// ① 命名
// ---------------------------------------------------------------------------

#[test]
fn lock_file_is_sibling_named_after_the_project_file() {
    assert_eq!(
        lock_path(Path::new("/tmp/demo.yeban")),
        PathBuf::from("/tmp/demo.yeban.lock")
    );
    assert_eq!(
        lock_path(Path::new("/tmp/no-extension")),
        PathBuf::from("/tmp/no-extension.lock")
    );
}

// ---------------------------------------------------------------------------
// ②③④ 同进程两个打开者 + 释放
// ---------------------------------------------------------------------------

#[test]
fn a_second_open_in_the_same_process_is_project_locked() {
    let scratch = Scratch::new("same-process");
    let project = scratch.project("demo.yeban");
    let (mut first, auth_first) = dispatcher();

    let opened = open(&mut first, &auth_first, &project, false);
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(opened["data"]["lockMode"], "ExclusiveWrite");
    assert_eq!(opened["data"]["advisoryLock"], true);
    assert_eq!(opened["data"]["tookOverStaleLock"], false);

    // 第二个 `Domain` = 第二个独立 `File` 句柄。旧实现（只看文件是否存在）
    // 也会拒绝，但拒绝的理由是"文件在"；这里的理由必须是"内核建议锁被占"。
    let (mut second, auth_second) = dispatcher();
    assert_locked(&mut second, &auth_second, &project, "第二个 Domain");

    // 独占期间**只读**打开同样被拒（读者不得绕开写者）。
    assert_locked(&mut second, &auth_second, &project, "独占期间只读打开");
    let read_only = open(&mut second, &auth_second, &project, true);
    assert_eq!(read_only["status"], "error", "{read_only}");
    assert_eq!(read_only["error"]["code"], "PROJECT_LOCKED", "{read_only}");
}

#[test]
fn an_exclusive_guard_releases_the_file_and_is_not_a_takeover() {
    let scratch = Scratch::new("release");
    let project = scratch.project("demo.yeban");
    let (mut first, auth_first) = dispatcher();
    assert_eq!(
        open(&mut first, &auth_first, &project, false)["status"],
        "success"
    );
    let lock = lock_path(&project);
    assert!(lock.is_file(), "持锁期间锁文件必须在");

    // 关闭工程 = 释放 `Active` 里的守卫 = 释放建议锁 + 删除锁文件。
    let closed = call_tool(
        &mut first,
        &auth_first,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    assert_eq!(closed["status"], "success", "{closed}");
    assert_eq!(closed["data"]["releasedLock"], true, "{closed}");
    assert_eq!(
        closed["data"]["releasedLockFile"],
        lock.display().to_string(),
        "{closed}"
    );
    assert!(!lock.exists(), "关闭工程后锁文件必须被删除");

    let (mut second, auth_second) = dispatcher();
    let reopened = open(&mut second, &auth_second, &project, false);
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(
        reopened["data"]["tookOverStaleLock"], false,
        "正常释放之后的重新打开不是接管: {reopened}"
    );
}

// ---------------------------------------------------------------------------
// ⑤⑥⑦ 陈旧锁：**仅凭文件存在不算被占用**
// ---------------------------------------------------------------------------

#[test]
fn a_lock_file_without_a_live_holder_is_not_occupied() {
    // 这条判据就是 `MUST-GATE-008` 要修的那个病：
    // 旧实现"存在即拒" ⇒ 崩溃后永久锁死；新实现必须能接管。
    let scratch = Scratch::new("stale");
    let project = scratch.project("demo.yeban");
    let lock = lock_path(&project);
    // 造一份**没有任何持有者**的锁文件（模拟崩溃遗留）。
    fs::write(
        &lock,
        "{\"pid\": 4242, \"lock_mode\": \"ExclusiveWrite\", \"last_heartbeat\": 1}\n",
    )
    .expect("造陈旧锁");

    let (mut dispatcher, auth) = dispatcher();
    let opened = open(&mut dispatcher, &auth, &project, false);
    assert_eq!(
        opened["status"], "success",
        "无持有者的锁文件必须可接管（这就是崩溃自愈）: {opened}"
    );
    assert_eq!(
        opened["data"]["tookOverStaleLock"], true,
        "必须明确报告「接管了陈旧锁」: {opened}"
    );
}

#[test]
fn taking_over_a_stale_lock_rewrites_the_metadata() {
    let scratch = Scratch::new("takeover-rewrite");
    let project = scratch.project("demo.yeban");
    let lock = lock_path(&project);
    fs::write(&lock, "{\"pid\": 4242, \"started_at\": 1}\n").expect("造陈旧锁");
    let before = read_metadata(&project).1.expect("旧元数据可解析");
    assert_eq!(before.pid, 4242);

    let (mut dispatcher, auth) = dispatcher();
    assert_eq!(
        open(&mut dispatcher, &auth, &project, false)["status"],
        "success"
    );

    // 接管之后的持有者元数据**从会话持有的守卫里读**（内存里那一份）——
    // Windows 的 `LockFileEx` 是强制锁, 持锁期间从另一个句柄读锁文件会被 OS 拒绝。
    let after = dispatcher
        .domain()
        .lock_holder()
        .expect("接管后守卫必须携带新持有者的元数据")
        .clone();
    assert_eq!(after.pid, std::process::id(), "PID 必须被换成新持有者");
    assert_eq!(after.lock_mode, "ExclusiveWrite");
    assert_eq!(
        after.project_path,
        project.display().to_string(),
        "锁内容必须记录被锁的工程路径"
    );
    assert!(after.started_at > 1, "接管必须刷新时间戳（旧值是 1）");
    // Unix 附加: `flock` 是建议锁 ⇒ 磁盘上的内容**也**必须被重写过
    // （证明守卫携带的那一份不是只活在内存里）。Windows 上这条读不到 —— 见
    // `holder_metadata_while_locked_is_platform_specific`。
    #[cfg(unix)]
    {
        let (text, on_disk) = read_metadata(&project);
        let on_disk = on_disk.unwrap_or_else(|| panic!("接管后锁内容必须可解析: {text}"));
        assert_eq!(on_disk.pid, std::process::id());
        assert_eq!(on_disk.lock_mode, "ExclusiveWrite");
        assert_eq!(
            on_disk.project_path,
            project.display().to_string(),
            "锁文件必须记录被锁的工程路径: {text}"
        );
        assert_eq!(on_disk, after, "磁盘内容必须等于守卫携带的元数据");
    }
    // 心跳阈值常量按规范 §0.2 钉住（它们只用于诊断，见 lock.rs 模块头）。
    assert_eq!(yeban_mcp::domain::lock::HEARTBEAT_INTERVAL_SECS, 3);
    assert_eq!(yeban_mcp::domain::lock::STALE_HEARTBEAT_SECS, 15);
    assert!(after.heartbeat_age_secs() < yeban_mcp::domain::lock::STALE_HEARTBEAT_SECS);
}

/// **持有者元数据在"持锁期间"的可读性是平台事实**（不是实现选择）。
///
/// 背景：CI 的 `windows` 手动门禁第一次执行就红了 —— 原因是
/// `fs::read_to_string(guard.path())` 在**持锁时**读锁文件：
///
/// - Unix：`std::fs::File::try_lock` = `flock(2)`，**建议锁** ⇒ 其它句柄照样能读；
/// - Windows：`std::fs::File::try_lock` = `LockFileEx`，锁的是**字节区间**且**强制** ⇒
///   其它句柄（**含同一进程的另一个句柄**）对该区间的读写被 OS 拒绝（`os error 33`）。
///
/// 因此本判据**按平台断言两种不同的行为**，并同时钉住修复后的设计：
/// "持有者是谁"一律从 [`yeban_mcp::domain::lock::LockGuard::holder`] 读（内存里那一份），
/// 两个平台行为一致。**不删断言、不两边都跳过** —— 那会把真实差异藏起来。
#[test]
fn holder_metadata_while_locked_is_platform_specific() {
    use yeban_mcp::domain::lock::{LockMode, acquire, read_snapshot};

    let scratch = Scratch::new("holder-view");
    let project = scratch.project("demo.yeban");
    let guard = acquire(&project, LockMode::ExclusiveWrite).expect("首次加锁");

    // ① 平台无关：守卫**自己携带**它写入的元数据 ⇒ 任何平台都能报告持有者。
    let holder = guard.holder().expect("排他守卫必须携带自己写入的元数据");
    assert_eq!(holder.pid, std::process::id());
    assert_eq!(holder.lock_mode, "ExclusiveWrite");
    assert_eq!(guard.mode(), LockMode::ExclusiveWrite);

    // ② 平台相关：**另一个句柄**能不能读到锁文件内容。
    let other_handle = fs::read_to_string(guard.path());
    #[cfg(unix)]
    {
        let text = other_handle.expect("flock 是建议锁 ⇒ 持锁期间其它句柄必须仍可读");
        assert_eq!(
            text,
            holder.to_json(),
            "磁盘内容必须等于守卫携带的元数据（证明它确实被写进了文件）"
        );
        assert!(read_snapshot(&project).readable(), "Unix 上快照必须可读");
        assert_eq!(read_snapshot(&project).availability(), "available");
    }
    #[cfg(windows)]
    {
        assert!(
            other_handle.is_err(),
            "LockFileEx 是强制锁 ⇒ 持锁期间其它句柄读**必须**被拒; \
             若这里变绿, 说明诊断路径的假设（持有者可读）需要重写, 而不是这条判据该删"
        );
        let snapshot = read_snapshot(&project);
        assert!(!snapshot.readable(), "Windows 上快照读不到是**预期**");
        assert_eq!(
            snapshot.availability(),
            "unavailable-on-this-platform",
            "读不到必须给出**平台口径**, 而不是含糊的 None"
        );
        assert_eq!(snapshot.metadata(), None);
        assert!(
            snapshot
                .holder_text()
                .contains("unavailable-on-this-platform"),
            "给人看的文本必须说明原因: {}",
            snapshot.holder_text()
        );
    }
    // 其它平台（wasm 等）：`acquire` 会显式 `UnsupportedPlatform`，这里只让变量不空悬。
    #[cfg(not(any(unix, windows)))]
    {
        let _ = other_handle;
    }

    // ②′ 平台无关：**自己**加锁之前取的那份快照在两个平台都可读 ——
    //     这正是"把读取放到加锁之前"的收益（对**别人的**持有者, Windows 上才会读不到）。
    //     若有人把 `LockFileSnapshot::read` 挪到 `try_lock` 之后, 这条会在 Windows 门禁上变红。
    assert!(
        guard.holder_snapshot().readable(),
        "加锁前读到的快照必须可读（证明读取确实发生在加锁之前）"
    );

    // ③ 平台无关：**占用者路径**必须产出 `PROJECT_LOCKED` 且**可读性口径**明确，
    //    而且**不 panic、不把打开操作判失败**（这正是 Windows 上退化后必须保住的性质）。
    let (mut blocked, blocked_auth) = dispatcher();
    let locked = open(&mut blocked, &blocked_auth, &project, false);
    assert_eq!(locked["error"]["code"], "PROJECT_LOCKED", "{locked}");
    assert_eq!(
        locked["error"]["data"]["advisoryLockHeld"], true,
        "{locked}"
    );
    let availability = locked["error"]["data"]["holderMetadata"]
        .as_str()
        .unwrap_or_else(|| panic!("PROJECT_LOCKED 必须披露持有者元数据的可读性: {locked}"));
    assert!(
        availability == "available" || availability == "unavailable-on-this-platform",
        "可读性口径只能是这两个值之一: {availability}"
    );
    assert!(
        locked["error"]["data"]["holder"].is_string(),
        "holder 必须始终是一个字符串（读不到时说明原因）: {locked}"
    );
    #[cfg(unix)]
    assert_eq!(availability, "available", "{locked}");
    #[cfg(windows)]
    assert_eq!(availability, "unavailable-on-this-platform", "{locked}");

    drop(guard);
}

#[test]
fn a_corrupted_lock_file_is_still_takeable() {
    // 崩溃可能发生在写完元数据之前 ⇒ 半截 JSON / 空文件。
    // 元数据**不参与判定**，所以这些残骸必须同样可接管。
    let scratch = Scratch::new("corrupt");
    let project = scratch.project("demo.yeban");
    let lock = lock_path(&project);
    for (tag, content) in [
        ("empty", ""),
        ("half-json", "{\"pid\": 12"),
        ("not-json", "not json at all\n"),
    ] {
        fs::write(&lock, content).expect("造坏锁");
        let (mut dispatcher, auth) = dispatcher();
        let opened = open(&mut dispatcher, &auth, &project, false);
        assert_eq!(
            opened["status"], "success",
            "坏锁内容({tag})必须可接管: {opened}"
        );
        assert_eq!(opened["data"]["tookOverStaleLock"], true, "{tag}: {opened}");
        drop(dispatcher);
        let _ = fs::remove_file(&lock);
    }
}

// ---------------------------------------------------------------------------
// ⑧⑨ 双模式（SHARED_READ / EXCLUSIVE_WRITE）
// ---------------------------------------------------------------------------

#[test]
fn shared_readers_coexist_and_block_a_writer() {
    let scratch = Scratch::new("shared");
    let project = scratch.project("demo.yeban");
    let (mut reader_a, auth_a) = dispatcher();
    let (mut reader_b, auth_b) = dispatcher();
    let (mut writer, auth_w) = dispatcher();

    let a = open(&mut reader_a, &auth_a, &project, true);
    assert_eq!(a["status"], "success", "{a}");
    assert_eq!(a["data"]["lockMode"], "SharedRead");
    let b = open(&mut reader_b, &auth_b, &project, true);
    assert_eq!(b["status"], "success", "多个只读打开必须共存: {b}");
    assert_eq!(b["data"]["lockMode"], "SharedRead");

    // 读者在场 ⇒ 写者被内核挡住（这是"只读"真正的含义）。
    assert_locked(&mut writer, &auth_w, &project, "读者在场时的写者");
    let writer_read_only = open(&mut writer, &auth_w, &project, true);
    assert_eq!(
        writer_read_only["status"], "success",
        "第三个读者仍应共存: {writer_read_only}"
    );
}

// ---------------------------------------------------------------------------
// ⑩⑪ 跨进程（真的另一个 PID）
// ---------------------------------------------------------------------------

#[test]
fn another_process_cannot_open_a_project_we_hold_exclusively() {
    let scratch = Scratch::new("cross-exclusive");
    let project = scratch.project("demo.yeban");
    // 父进程先独占持有。
    let (mut holder_dispatcher, holder_auth) = dispatcher();
    assert_eq!(
        open(&mut holder_dispatcher, &holder_auth, &project, false)["status"],
        "success"
    );

    // **另一个进程**（真的另一个 PID，跑真实的 MCP 打开链路）被拒。
    let ready = scratch.join("child.ready");
    let blocked = ChildHolder::spawn_blocked(&project, &ready, true);
    assert_ne!(blocked.pid, std::process::id(), "必须是另一个进程");

    // 反向：子进程持有排他锁时，本进程也必须被拒。
    drop(holder_dispatcher);
    let ready2 = scratch.join("child2.ready");
    let holder = ChildHolder::spawn_holding(&project, &ready2, true);
    let (mut other, other_auth) = dispatcher();
    assert_locked(&mut other, &other_auth, &project, "跨进程占用");
    drop(holder);
}

#[test]
fn another_process_can_share_the_read_lock_or_be_blocked_by_a_writer() {
    let scratch = Scratch::new("cross-shared");
    let project = scratch.project("demo.yeban");
    // 父进程只读（共享锁），子进程也只要共享锁 ⇒ 两个进程共存。
    let (mut reader, auth) = dispatcher();
    let opened = open(&mut reader, &auth, &project, true);
    assert_eq!(opened["status"], "success", "{opened}");

    let ready = scratch.join("shared-child.ready");
    let sharer = ChildHolder::spawn_holding(&project, &ready, false);
    assert_ne!(sharer.pid, std::process::id());
    assert_eq!(
        sharer.response["data"]["lockMode"], "SharedRead",
        "子进程必须是以共享读锁加入的: {}",
        sharer.response
    );

    // 但**写者**必须被这两个共享持有者挡住。
    let (mut writer, writer_auth) = dispatcher();
    assert_locked(
        &mut writer,
        &writer_auth,
        &project,
        "读者在场时的写者(跨进程)",
    );

    // 反向：父进程独占时，另一个进程拿**共享读**也不行（读者不得绕开写者）。
    drop(sharer);
    drop(reader);
    let (mut exclusive, exclusive_auth) = dispatcher();
    assert_eq!(
        open(&mut exclusive, &exclusive_auth, &project, false)["status"],
        "success"
    );
    // 独占期间锁文件存在且被持有 ⇒ 子进程的共享读必须被拒。
    let ready2 = scratch.join("shared-blocked.ready");
    let blocked = ChildHolder::spawn_blocked(&project, &ready2, false);
    assert_ne!(blocked.pid, std::process::id());
}

// ---------------------------------------------------------------------------
// ⑫⑬ 崩溃自愈（SIGKILL）
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_sigkilled_holder_releases_the_advisory_lock_and_can_be_taken_over() {
    let scratch = Scratch::new("crash");
    let project = scratch.project("demo.yeban");
    let ready = scratch.join("crash-child.ready");
    let mut holder = ChildHolder::spawn_holding(&project, &ready, true);
    let lock = lock_path(&project);

    // 在子进程活着的时候，父进程必须被拒（证明它真的持有建议锁）。
    let (mut blocked, blocked_auth) = dispatcher();
    assert_locked(&mut blocked, &blocked_auth, &project, "崩溃前的占用");

    // 真崩溃: `SIGKILL` 不给子进程任何清理机会（不走 `Drop`、不走 `atexit`）。
    holder.child.kill().expect("SIGKILL 子进程");
    let status = holder.child.wait().expect("收尸");
    assert!(
        !status.success(),
        "被 SIGKILL 的子进程不该是正常退出: {status:?}"
    );

    // 锁文件**还在磁盘上**（没人删它）——这正是旧实现永久锁死的现场。
    assert!(
        lock.exists(),
        "崩溃遗留: 锁文件必须仍在（否则这条判据什么都没证明）"
    );

    // 而内核已经释放了建议锁 ⇒ 新打开者必须能接管。
    let (mut recovered, recovered_auth) = dispatcher();
    let reopened = open(&mut recovered, &recovered_auth, &project, false);
    assert_eq!(
        reopened["status"], "success",
        "崩溃后必须能接管（内核在进程死亡时释放 flock）: {reopened}"
    );
    assert_eq!(reopened["data"]["tookOverStaleLock"], true, "{reopened}");

    // 接管之后持有者必须是**新**持有者（不是那个已经被 SIGKILL 的 PID）。
    // 从守卫里读（内存），而不是持锁后去读锁文件 —— 后者在 Windows 上必然被拒。
    let meta = recovered
        .domain()
        .lock_holder()
        .expect("接管后守卫必须携带元数据")
        .clone();
    assert_eq!(meta.pid, std::process::id());
    assert_ne!(meta.pid, holder.pid, "陈旧 PID 必须被覆盖");
    // Unix 附加：磁盘上的陈旧内容也必须被重写（Windows 读不到，见平台矩阵）。
    #[cfg(unix)]
    {
        let (text, on_disk) = read_metadata(&project);
        let on_disk = on_disk.unwrap_or_else(|| panic!("接管后锁内容必须可解析: {text}"));
        assert_eq!(on_disk.pid, std::process::id());
        assert_ne!(on_disk.pid, holder.pid, "陈旧 PID 必须被覆盖: {text}");
    }
    let _ = holder.child.wait();
}

// ---------------------------------------------------------------------------
// ⑭ 契约码：锁占用只报 PROJECT_LOCKED
// ---------------------------------------------------------------------------

#[test]
fn the_only_contract_code_for_lock_contention_is_project_locked() {
    // 不发明新错误码: `PROJECT_LOCKED` 必须在契约 enum 里,
    // 而且本文件所有"被占用"断言用的都是它。
    assert!(
        ErrorCode::SCHEMA_CONTRACT.contains(&ErrorCode::ProjectLocked),
        "PROJECT_LOCKED 必须在契约 enum 里"
    );
    assert_eq!(ErrorCode::ProjectLocked.as_str(), "PROJECT_LOCKED");
    assert_eq!(
        ErrorCode::SCHEMA_CONTRACT.len(),
        20,
        "ADR-0001 D25 的联集是 20 值; 本线不得增删"
    );
}

// ---------------------------------------------------------------------------
// ⑮ 平台不支持：显式拒绝（映射判据；真机判据 pending P3）
// ---------------------------------------------------------------------------

#[test]
fn an_unsupported_platform_is_refused_through_the_implementation_exit() {
    // 本机是 macOS, 走不到 `UnsupportedPlatform` 分支 —— 所以这里对
    // **映射函数**做判据（而不是假装跑过那条平台分支）:
    // 它必须走 `Fault::Impl`（JSON-RPC `-32005`），**绝不**伪造一个契约错误码。
    // 理由: 契约 enum 是闭合的 20 值, 里面没有也不该有"平台不支持";
    // 硬塞进去会让 `implementation_error_code_catalog_equals_the_contract_enum_exactly`
    // 与 `every_emitted_error_code_is_inside_the_contract_enum` 同时变红。
    let path = Path::new("/tmp/whatever.yeban");
    let fault = yeban_mcp::domain::store::lock_fault(
        path,
        yeban_mcp::domain::lock::LockError::UnsupportedPlatform {
            os: "wasm32",
            spec_id: "ARCH-SEC-001/MUST-GATE-008",
        },
    );
    assert!(
        fault.domain_code().is_none(),
        "平台不支持不是领域失败（否则会产出契约外的 ToolResponse）: {fault:?}"
    );
    assert!(
        fault.to_tool_response().is_none(),
        "必须走 JSON-RPC 出口, 不得伪造 ToolResponse"
    );
    let error = fault.into_result().expect_err("必须是实现级错误出口");
    assert_eq!(error.code, yeban_mcp::jsonrpc::NOT_IMPLEMENTED);
    let data = error.data.expect("data");
    assert_eq!(data["code"], "NOT_IMPLEMENTED");
    assert_eq!(data["os"], "wasm32");
    assert_eq!(data["specId"], "ARCH-SEC-001/MUST-GATE-008");
    assert_eq!(data["lockFile"], lock_path(path).display().to_string());
}

/// **两个接管者争同一个陈旧锁文件：恰好一个赢，输的那个不删文件。**
///
/// 为什么需要这一条：既有判据覆盖了"独占持有者与第二个打开者"（同进程/跨进程）与
/// "**单个**接管陈旧锁"，但没有一条把**两个接管者**放在同一个陈旧锁文件上。
/// 这一格是危险的方向：如果输的那个也去 `remove_file`，赢的那个的锁文件会被删掉，
/// 第三个打开者就能创建**新 inode** 并同时"持锁" ⇒ 两个独占持有者。
///
/// 单位 = 一次 `yeban_open_project` 的响应（`status` 与 `data.tookOverStaleLock`）。
/// 判据**不依赖墙钟**：只比 `tookOverStaleLock` 布尔与文件在不在。
///
/// 注入（实测红）：把 `open_lock_file` 失败分支里的"不删锁文件"改成删 ⇒ 第 3 条红；
/// 把 `took_over_stale_lock` 的 `!created` 去掉 ⇒ 第 1 条红。
#[test]
fn only_one_of_two_contenders_takes_over_a_stale_lock() {
    let scratch = Scratch::new("takeover-contended");
    let project = scratch.project("demo.yeban");
    let lock = lock_path(&project);
    // 一个**没有持有者**的陈旧锁文件（无建议锁 ⇒ 可接管）。
    fs::write(&lock, "{\"pid\": 4242, \"started_at\": 1}\n").expect("造陈旧锁");

    let (mut first, auth_first) = dispatcher();
    let won = open(&mut first, &auth_first, &project, false);
    assert_eq!(won["status"], "success", "{won}");
    assert_eq!(
        won["data"]["tookOverStaleLock"], true,
        "第一个接管者必须如实上报 `tookOverStaleLock = true`: {won}"
    );
    assert!(lock.is_file(), "赢家持锁期间锁文件必须在");

    // 第二个接管者：同一个陈旧文件、另一个 `Domain`（另一个 `File` 句柄）。
    let (mut second, auth_second) = dispatcher();
    let lost = open(&mut second, &auth_second, &project, false);
    assert_eq!(lost["status"], "error", "{lost}");
    assert_eq!(lost["error"]["code"], "PROJECT_LOCKED", "{lost}");
    assert!(
        lock.is_file(),
        "**输的那个不得删锁文件** —— 否则第三个打开者会拿到新 inode 并同时持锁"
    );

    // 赢家仍是唯一持有者：它的元数据还在，且第三个打开者仍然被拦。
    let (mut third, auth_third) = dispatcher();
    let still_locked = open(&mut third, &auth_third, &project, false);
    assert_eq!(
        still_locked["error"]["code"], "PROJECT_LOCKED",
        "{still_locked}"
    );

    // 赢家关闭（释放建议锁 + 删文件）之后，下一个打开者是**新建**而不是接管。
    let closed = call_tool(
        &mut first,
        &auth_first,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    assert_eq!(closed["status"], "success", "{closed}");
    assert!(!lock.exists(), "释放后锁文件必须被删除");
    let (mut fourth, auth_fourth) = dispatcher();
    let fresh = open(&mut fourth, &auth_fourth, &project, false);
    assert_eq!(fresh["status"], "success", "{fresh}");
    assert_eq!(
        fresh["data"]["tookOverStaleLock"], false,
        "文件已不存在 ⇒ 这是新建，不是接管: {fresh}"
    );
}

/// **活着的持有者即使心跳陈旧、PID 也不存在，也不得被接管**（抢锁只看 OS 建议锁）。
///
/// 为什么需要这一条：规范原文写的是"超过 15 秒**或** `kill(pid, 0)` 失败则允许接管"
/// —— 那是一条**能把活锁抢走**的规则（心跳落后 15 秒是正常抖动，PID 复用也发生过）。
/// 本实现把它换成"只看 `flock`/`LockFileEx`"，本判据把那件事钉住：
/// 判据把**活着的**持有者的锁文件内容改成一个古老的心跳 + 一个不存在的 PID，
/// 第二个打开者**仍然**必须拿到 `PROJECT_LOCKED`。
///
/// ⚠ `#[cfg(unix)]`：这一步要在别人持锁时**写**锁文件，而 Windows 的 `LockFileEx`
/// 是强制锁 ⇒ 写入会被 OS 拒绝（不是实现的问题）。Unix 的 `flock` 是建议锁 ⇒ 写得进去。
///
/// 单位 = 一次 `yeban_open_project` 的响应。判据**不依赖墙钟**（不比较秒数，
/// 只看成功/失败；写进去的心跳值是一个**固定**的古老常量）。
#[cfg(unix)]
#[test]
fn a_stale_heartbeat_on_a_live_holder_does_not_allow_takeover() {
    use yeban_mcp::domain::lock::{LockMode, acquire};

    let scratch = Scratch::new("heartbeat-ignored");
    let project = scratch.project("demo.yeban");
    let lock = lock_path(&project);
    let guard = acquire(&project, LockMode::ExclusiveWrite).expect("直接拿锁");
    assert!(lock.is_file());
    // 把**活着的**持有者的内容改旧：心跳 = 1（Unix 纪元），PID = 一个几乎不可能存在的值。
    // 规范那条"15 秒或 kill 失败即接管"的规则在这里**必须不生效**。
    fs::write(
        &lock,
        "{\"pid\": 1, \"lock_mode\": \"ExclusiveWrite\", \"last_heartbeat\": 1, \"started_at\": 1}\n",
    )
    .expect("写活持有者的锁文件（Unix 建议锁允许）");

    let (mut other, auth) = dispatcher();
    let refused = open(&mut other, &auth, &project, false);
    assert_eq!(
        refused["status"], "error",
        "活着的持有者即使心跳陈旧也不得被接管: {refused}"
    );
    assert_eq!(refused["error"]["code"], "PROJECT_LOCKED", "{refused}");
    assert!(lock.is_file(), "被拒的调用不得删锁文件");

    // 阴性对照：守卫释放之后同一个调用成功（证明上面红的是"锁还在"，不是别的）。
    drop(guard);
    assert!(!lock.exists(), "排他守卫 Drop 后锁文件被删除");
    let (mut third, auth_third) = dispatcher();
    assert_eq!(
        open(&mut third, &auth_third, &project, false)["status"],
        "success"
    );
}
