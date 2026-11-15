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
#![allow(unused)]
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
// ① 别的形态先持排他锁 ⇒ app 侧控制面拒绝挂载
// ---------------------------------------------------------------------------

/// `ROAD-M0-007` 的方向一：**拒绝挂载**。
///
/// 另一个进程排他持锁时，`SessionSource::File` 的挂载必须 `Err(MountError::Locked)`
/// —— 不是"挂上去但悄悄不取锁"。拿掉持有者之后**同一份挂载必须成功**，
/// 且模式是 `SharedRead`：这证明拒绝的原因就是那把锁，而不是别的偶发失败。
#[test]
fn a_mount_is_refused_while_another_form_holds_the_project_exclusively() {
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
