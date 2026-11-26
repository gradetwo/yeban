//! T-FIX-497：`host_save_project` 另存到别处时必须尊重目标的 `.yeban.lock`。
//!
//! 复现场景（issue 原文）：
//! 1. 会话 A 打开工程 X（持 X 的排他锁）；
//! 2. 会话 B（另一工程 Y）经宿主动作另存到 X 的路径；
//! 3. 修法前 B 的保存成功，A 的内容被 B 的 `rename` 静默覆盖 —— 无任何一方收到
//!    `PROJECT_LOCKED`。
//!
//! 判据：
//! 1. 另存到另一活会话持有的工程 ⇒ `PROJECT_LOCKED`，目标**一个字节都不变**，
//!    且被锁会话事后仍能正常保存到自己的路径（拒绝没有扰动持有者的锁）；
//! 2. 保存到自己的路径仍成功（不重取自己已持有的锁 —— `flock` 是 fd 粒度，
//!    同进程第二个 fd 再取同一把锁必然 `WouldBlock`）；
//! 3. 另存到无锁的新路径成功，且排他守卫 `Drop` 后不留下 `.yeban.lock`。
//!
//! 夹具：两个 `Dispatcher`（两个独立 `Domain`，即两个会话）+ 临时目录，
//! 工程用 `yeban_open_project { create: true }` 真打开（`apply_open` 持锁），
//! 不用 `open_in_memory`（它不取锁，`lock: None`）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::lock::lock_path;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::ErrorCode;
use yeban_model::EntityId;

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-host-save-lock-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 调工具；工具层失败就直接 panic（打开必须成功，失败是夹具错不是被测错）。
fn call_tool(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    response.result.unwrap_or_else(|| {
        panic!(
            "工具 {name} 必须成功: {}",
            serde_json::to_string(&response.error).unwrap_or_default()
        )
    })
}

/// 真打开一份新工程（`create: true` 落盘 + `apply_open` 持排他锁）。
fn open_new(dispatcher: &mut Dispatcher, auth: &str, path: &Path) {
    fs::create_dir_all(path.parent().expect("工程必须有父目录")).expect("建工程目录");
    let opened = call_tool(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string(), "create": true }),
    );
    assert_eq!(opened["status"], "success", "打开必须成功: {opened}");
    assert_eq!(opened["data"]["opened"], true, "{opened}");
    assert_eq!(opened["data"]["locked"], true, "打开必须持锁: {opened}");
}

/// ① 另存到另一活会话持有的工程 ⇒ `PROJECT_LOCKED`，目标一个字节都不变。
#[test]
fn save_as_to_project_held_by_another_live_session_is_refused() {
    let scratch = Scratch::new("locked-target");
    let x_path = scratch.join("x/song.yeban");
    let y_path = scratch.join("y/song.yeban");

    // 会话 A 打开 X（持 X 的排他锁）；会话 B 打开 Y。
    let (mut dispatcher_a, auth_a) = fresh_dispatcher();
    open_new(&mut dispatcher_a, &auth_a, &x_path);
    let (mut dispatcher_b, auth_b) = fresh_dispatcher();
    open_new(&mut dispatcher_b, &auth_b, &y_path);

    let bytes_before = fs::read(&x_path).expect("X 必须已落盘");

    // B 另存到 X：必须被拒，且 X 一个字节都不变。
    let error = yeban_mcp::domain::host_save_project(dispatcher_b.domain_mut(), &x_path, true)
        .expect_err("另存到活会话持有的工程必须被拒");
    assert_eq!(
        error.domain_code(),
        Some(ErrorCode::ProjectLocked),
        "只报 PROJECT_LOCKED，不发明新码: {error:?}"
    );
    assert_eq!(
        fs::read(&x_path).expect("X 必须还在"),
        bytes_before,
        "被拒的保存一个字节都不许写"
    );

    // 拒绝没有扰动持有者：A 事后仍能正常保存到自己的路径。
    let outcome = yeban_mcp::domain::host_save_project(dispatcher_a.domain_mut(), &x_path, true)
        .expect("持有者事后保存到自己路径必须成功");
    assert!(!outcome.skipped, "force=true 必须真写: {outcome:?}");
    assert_eq!(outcome.path, x_path, "{outcome:?}");
}

/// ② 保存到自己的路径仍成功（不重取自己已持有的锁）。
#[test]
fn save_to_own_path_still_succeeds() {
    let scratch = Scratch::new("own-path");
    let y_path = scratch.join("y/song.yeban");

    let (mut dispatcher, auth) = fresh_dispatcher();
    open_new(&mut dispatcher, &auth, &y_path);

    let outcome = yeban_mcp::domain::host_save_project(dispatcher.domain_mut(), &y_path, true)
        .expect("保存到自己路径必须成功（会话已持有这把锁，不重取）");
    assert!(!outcome.skipped, "force=true 必须真写: {outcome:?}");
    assert_eq!(outcome.path, y_path, "{outcome:?}");
}

/// ③ 另存到无锁的新路径成功，且排他守卫 `Drop` 后不留下 `.yeban.lock`。
#[test]
fn save_as_to_unlocked_new_path_succeeds_and_leaves_no_lock_file() {
    let scratch = Scratch::new("fresh-target");
    let y_path = scratch.join("y/song.yeban");
    let z_path = scratch.join("z/song.yeban");

    let (mut dispatcher, auth) = fresh_dispatcher();
    open_new(&mut dispatcher, &auth, &y_path);

    // 目标父目录须存在（`write_project_atomic` 不建目录 —— 与修法无关的既有口径）。
    fs::create_dir_all(z_path.parent().expect("目标必须有父目录")).expect("建目标目录");

    let outcome = yeban_mcp::domain::host_save_project(dispatcher.domain_mut(), &z_path, true)
        .expect("另存到无锁的新路径必须成功");
    assert!(!outcome.skipped, "force=true 必须真写: {outcome:?}");
    assert_eq!(outcome.path, z_path, "{outcome:?}");
    assert!(z_path.exists(), "目标必须已落盘");
    assert!(
        !lock_path(&z_path).exists(),
        "排他守卫 Drop 后不得留下 .yeban.lock"
    );
}
