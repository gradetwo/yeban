//! T-FIX-498：`yeban_save_project` 回执与 `host_save_project` 读数带持久化读数。
//!
//! 两条判据（都是**成功路径**的读数形状 —— 警告路径由 `store.rs` 的注入缝判据
//! `atomic_write_reports_warning_when_dir_sync_fails_but_file_is_committed` 覆盖，
//! 断电注入在测试里不构造）：
//! 1. 写盘分支：工具回执带 `"durabilityWarning": null`（刷目录成功时无警告），
//!    且 `host_save_project` 的 `durability_warning` 是 `None`。
//! 2. 跳过分支（内存与磁盘一致）：同样带 `"durabilityWarning": null`。
//!
//! 夹具：`filled_project()` 注入内存会话，工程路径 `<scratch>/proj/song.yeban`。

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::EntityId;
use yeban_model::samples::{filled_project, filled_project_assets};

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-save-durability-{tag}-{}-{}",
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

/// ① 写盘分支：回执与宿主读数都带持久化读数（成功时为 null / None）。
#[test]
fn a_written_save_reports_a_null_durability_warning() {
    let scratch = Scratch::new("written");
    let project_path = scratch.join("proj/song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");

    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory_with_assets(
            project_path.clone(),
            filled_project(),
            false,
            filled_project_assets(),
        )
        .expect("注入夹具工程");

    let saved = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    let data = &saved["data"];
    assert_eq!(data["saved"], true, "force 保存必须写盘: {saved}");
    assert!(
        data["durabilityWarning"].is_null(),
        "刷目录成功时回执的 durabilityWarning 必须是 null: {saved}"
    );

    // 宿主读数（GUI 侧走这条，不解析工具 JSON）。
    let outcome =
        yeban_mcp::domain::host_save_project(dispatcher.domain_mut(), &project_path, true)
            .expect("宿主保存必须成功");
    assert_eq!(
        outcome.durability_warning, None,
        "刷目录成功时宿主读数的 durability_warning 必须是 None: {outcome:?}"
    );
}

/// ② 跳过分支：没写盘 ⇒ 同样带 `"durabilityWarning": null`（不是缺字段）。
#[test]
fn a_skipped_save_reports_a_null_durability_warning() {
    let scratch = Scratch::new("skipped");
    let project_path = scratch.join("proj/song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");

    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory_with_assets(
            project_path,
            filled_project(),
            false,
            filled_project_assets(),
        )
        .expect("注入夹具工程");

    let first = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(first["data"]["saved"], true, "force 保存必须写盘: {first}");

    let second = call_tool(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    let data = &second["data"];
    assert_eq!(data["skipped"], true, "第二次保存必须跳过: {second}");
    assert!(
        data["durabilityWarning"].is_null(),
        "跳过分支的 durabilityWarning 也必须是 null（没写盘，没有警告）: {second}"
    );
}
