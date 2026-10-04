//! **容器接线的端到端判据**：`yeban_save_project` 写出 `ARCH-SEC-003` 的 `.yeban` ZIP 容器，
//! `yeban_open_project` 读回它，`ARCH-SEC-004` 的原子落盘协议不变
//! [ARCH-SEC-003, ARCH-SEC-004, ARCH-OPS-002, MUST-GATE-006, MUST-GATE-007, MODEL-AST-007]。
//!
//! ## 这个文件与 `tools_e2e.rs` 的分工
//!
//! `tools_e2e.rs` 证明"十个工具真的做事、`dryRun`/幂等/锁成立"；
//! 本文件只回答一个问题：**"保存/打开"这条路上的字节到底是什么**。
//!
//! | # | 判据 | 钉住的事实 |
//! | :--- | :--- | :--- |
//! | 1 | `save_writes_a_standard_container_with_the_zip_magic` | 落盘前 4 字节是 `PK\x03\x04`，模型层容器读取器能解出条目 |
//! | 2 | `save_then_open_round_trips_the_project_byte_for_byte` | 往返后工程**逐字节**相同 |
//! | 3 | `save_then_open_restores_the_commit_graph_from_history_dag` | `history.dag` 不是死重量：合并过的 DAG 跨打开存活 |
//! | 4 | `two_forced_saves_of_the_same_session_are_byte_identical` | 同输入两次保存字节相同（`ARCH-DET-001`） |
//! | 5 | `save_into_a_read_only_directory_keeps_the_original_container_bytes` | 失败**不破坏原文件**、不留临时文件 |
//! | 6 | `save_replaces_the_target_inode_instead_of_truncating_it` | 真的是 tmp+`rename`，不是原地截断写（unix inode 变化） |
//! | 7 | `bare_json_projects_still_open_on_the_compat_path` | 裸 JSON 兼容路径活着，且响应**披露**形态 |
//! | 8 | `container_project_json_is_accepted_by_the_project_schema` | 容器里的 `project.json` 被 `schemas/project.schema.json` 接受（Python jsonschema 独立对账） |
//! | 9 | `assets_round_trip_with_content_addressing` | `assets/{sha256}` 的条目名 = 字节的 SHA-256；往返后池内容一致 |
//! | 10 | `truncated_container_is_refused_instead_of_loading_half_a_project` | 截断 → `IO_ERROR`，不静默加载半个工程 |
//! | 11 | `crc_tampered_container_is_refused_instead_of_loading_it` | 篡改 → `IO_ERROR`，不静默加载 |
//! | 12 | `zip_slip_entry_name_is_refused_with_a_contract_error_code` | `MUST-GATE-006` → `IO_ERROR` + `category=path-traversal` |
//! | 13 | `unsupported_compression_is_refused_with_a_contract_error_code` | deflate 不被静默跳过 → `IO_ERROR` + `category=unsupported-container-feature` |
//! | 14 | `container_without_history_dag_is_refused_as_a_layout_error` | §5.3 布局缺失 → `IO_ERROR` + `category=container-layout` |
//! | 15 | `every_container_rejection_stays_inside_the_contract_enum` | 全部容器失败码都落在 `ADR-0001 D25` 的 20 值联集里 |
//! | 16 | `corrupt_history_dag_is_refused_instead_of_silently_dropping_history` | 坏 `history.dag` / 无 `main` 分支的 DAG 必须在打开时就被拒绝 |
//!
//! 临时目录纪律：全部落在 `std::env::temp_dir()/<唯一子目录>`，`Drop` 负责恢复权限并删除
//! （`docs/DEVELOPMENT_LEDGER.md` L16）—— **绝不污染仓库**。
//!
//! 会"显式 skip"的一处（**不会伪装成通过**）：判据 8 需要 `python3` + `jsonschema`。

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::jsonrpc::ErrorObject;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::ErrorCode;
use yeban_model::container::{
    ContainerEntry, ContainerLimits, HISTORY_DAG_NAME, PROJECT_JSON_NAME, read_container,
    read_project_container, write_container,
};
use yeban_model::{AssetHash, YebanProjectV1};

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 进程内唯一序号（避免同一纳秒内两个测试撞名）。
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 一个临时目录，`Drop` 时清理（含被改成只读的情况）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-container-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // 只读目录必须先恢复权限才能删掉自己。
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 测试用分发器（全量 scope、生产模式、固定时钟）。
fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 走 `tools/call`，返回 `(http 状态, JSON-RPC 层结果)`。
fn call_raw(
    dispatcher: &mut Dispatcher,
    auth: &str,
    name: &str,
    arguments: Value,
) -> (u16, Result<Value, ErrorObject>) {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    match (response.result, response.error) {
        (Some(result), None) => (outcome.http_status, Ok(result)),
        (None, Some(error)) => (outcome.http_status, Err(error)),
        other => panic!("result 与 error 必须恰好一个: {other:?}"),
    }
}

/// 走 `tools/call` 并断言这是**带内** `ToolResponse`。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let (status, outcome) = call_raw(dispatcher, auth, name, arguments);
    assert_eq!(
        status, 200,
        "{name} 必须是带内结果（领域失败也走 ToolResponse）"
    );
    outcome.unwrap_or_else(|error| panic!("{name} 不该有 JSON-RPC 层错误: {error:?}"))
}

/// 写一份**裸 JSON** 工程夹具（容器接线之前的格式 —— 兼容路径的输入）并打开它。
///
/// 返回 `(路径, 打开响应)`。
fn open_bare_json(scratch: &Scratch, dispatcher: &mut Dispatcher, auth: &str) -> (PathBuf, Value) {
    let project = yeban_model::samples::filled_project();
    let mut text = serde_json::to_string_pretty(&project).expect("序列化");
    text.push('\n');
    let path = scratch.join("demo.yeban");
    fs::write(&path, &text).expect("写裸 JSON 工程");
    let opened = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(
        opened["data"]["format"], "bare-json",
        "裸 JSON 夹具必须走兼容路径: {opened}"
    );
    (path, opened)
}

/// 工程的规范化文本（`projectDigest` 的口径，用于"逐字节相同"的断言）。
fn canonical_text(project: &YebanProjectV1) -> String {
    let mut text = serde_json::to_string_pretty(project).expect("序列化");
    text.push('\n');
    text
}

/// 一份最小合法容器（`project.json` + `history.dag`，外加调用方给的条目）。
fn minimal_container(project: &YebanProjectV1, extra: &[ContainerEntry]) -> Vec<u8> {
    let json = serde_json::to_vec(project).expect("工程 JSON");
    let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱 JSON");
    let mut entries = vec![
        ContainerEntry::new(PROJECT_JSON_NAME, json),
        ContainerEntry::new(HISTORY_DAG_NAME, history),
    ];
    entries.extend(extra.iter().cloned());
    write_container(&entries).expect("写出合法容器")
}

/// 断言带内 `ToolResponse` 是容器拒绝，且分类/规范 ID/契约成员资格都对。
fn assert_container_error(value: &Value, category: &str, spec_id: &str, context: &str) {
    assert_eq!(value["status"], "error", "{context}: {value}");
    assert_eq!(value["error"]["code"], "IO_ERROR", "{context}: {value}");
    assert_eq!(
        value["error"]["data"]["category"], category,
        "{context}: {value}"
    );
    assert_eq!(
        value["error"]["data"]["specId"], spec_id,
        "{context}: {value}"
    );
    assert!(
        ErrorCode::SCHEMA_CONTRACT
            .iter()
            .any(|code| code.as_str() == "IO_ERROR"),
        "IO_ERROR 必须在契约 enum 里（ADR-0001 D25 的 20 值联集）"
    );
}

/// 把容器写到磁盘并尝试打开，返回带内结果。
fn open_bytes(
    scratch: &Scratch,
    dispatcher: &mut Dispatcher,
    auth: &str,
    name: &str,
    bytes: &[u8],
) -> Value {
    let path = scratch.join(name);
    fs::write(&path, bytes).expect("写容器字节");
    call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    )
}

/// 仓库根（`scripts/gates/**` 的相对基准）。
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("仓库根")
}

// ---------------------------------------------------------------------------
// 判据 1：保存写出容器，且前 4 字节是 ZIP 本地文件头
// ---------------------------------------------------------------------------

#[test]
fn save_writes_a_standard_container_with_the_zip_magic() {
    let scratch = Scratch::new("magic");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);

    // 改一处内容（让"未保存标记"为真，走真落盘路径）。
    let track = macro_track(&dispatcher);
    propose_and_merge(&mut dispatcher, &auth, &track);

    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_eq!(saved["data"]["format"], "yeban-container");
    assert_eq!(saved["data"]["atomic"], true);

    let bytes = fs::read(&path).expect("读容器字节");
    assert_eq!(
        &bytes[..4],
        b"PK\x03\x04",
        "ARCH-SEC-003: 保存出的前 4 字节必须是 ZIP 本地文件头"
    );
    assert_eq!(
        saved["data"]["bytes"].as_u64().unwrap() as usize,
        bytes.len(),
        "响应里的字节数必须等于磁盘字节数"
    );

    // 模型层的容器读取器能解开它（不是"长得像 ZIP"）。
    let archive = read_container(&bytes, &ContainerLimits::default()).expect("容器必须可读");
    let names: Vec<&str> = archive.names().collect();
    assert!(names.contains(&PROJECT_JSON_NAME), "{names:?}");
    assert!(names.contains(&HISTORY_DAG_NAME), "{names:?}");
    assert_eq!(archive.len(), 2, "没有资产时恰好 2 个条目: {names:?}");
    assert_eq!(
        saved["data"]["assets"], 0,
        "夹具没有 CAS 资产 ⇒ 容器里只有 project.json + history.dag"
    );
}

// ---------------------------------------------------------------------------
// 判据 2：往返后工程逐字节相同
// ---------------------------------------------------------------------------

#[test]
fn save_then_open_round_trips_the_project_byte_for_byte() {
    let scratch = Scratch::new("roundtrip");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);

    // 改一处，保存，记下内存工程的规范化文本。
    let track = macro_track(&dispatcher);
    propose_and_merge(&mut dispatcher, &auth, &track);
    let in_memory = dispatcher.domain().active_project().expect("工程").clone();
    let expected = canonical_text(&in_memory);
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "{saved}");

    // 关掉会话（释放锁），再从磁盘**真的**打开一次。
    let closed = call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    assert_eq!(closed["data"]["closed"], true);
    let reopened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(reopened["data"]["format"], "yeban-container");

    let after = dispatcher.domain().active_project().expect("工程");
    assert_eq!(
        canonical_text(after),
        expected,
        "保存 → 重新打开后工程必须逐字节相同"
    );
    assert_eq!(after, &in_memory);
    assert_eq!(
        reopened["data"]["projectDigest"], saved["data"]["projectDigest"],
        "往返后工程摘要必须相同"
    );
}

// ---------------------------------------------------------------------------
// 判据 3：history.dag 真的被恢复
// ---------------------------------------------------------------------------

#[test]
fn save_then_open_restores_the_commit_graph_from_history_dag() {
    let scratch = Scratch::new("history");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    propose_and_merge(&mut dispatcher, &auth, &track);
    // 根提交 + 提案提交 + 合并提交。
    assert_eq!(dispatcher.domain().commit_count(), 3);
    let head_before = dispatcher.domain().main_head().expect("主分支头");

    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["historyCommits"], 3, "{saved}");

    call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    let reopened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(
        reopened["data"]["historyRestored"], true,
        "`history.dag` 不是死重量: {reopened}"
    );
    assert_eq!(
        dispatcher.domain().commit_count(),
        3,
        "恢复的 DAG 必须仍是 3 条提交（不是重新开一条根提交）"
    );
    assert_eq!(
        dispatcher.domain().main_head(),
        Some(head_before),
        "主分支头必须被原样恢复"
    );
}

// ---------------------------------------------------------------------------
// 判据 4：同输入两次保存字节相同（ARCH-DET-001）
// ---------------------------------------------------------------------------

#[test]
fn two_forced_saves_of_the_same_session_are_byte_identical() {
    let scratch = Scratch::new("determinism");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    propose_and_merge(&mut dispatcher, &auth, &track);

    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(first["data"]["saved"], true, "{first}");
    let bytes_a = fs::read(&path).expect("第一次保存的字节");

    let second = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(second["data"]["saved"], true, "force 必须绕过'无改动跳过'");
    let bytes_b = fs::read(&path).expect("第二次保存的字节");

    assert!(!bytes_a.is_empty());
    assert_eq!(
        bytes_a, bytes_b,
        "ARCH-DET-001: 同一会话同一状态两次落盘必须逐字节相同"
    );
}

// ---------------------------------------------------------------------------
// 判据 5：保存失败不破坏原文件（容器版本）
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn save_into_a_read_only_directory_keeps_the_original_container_bytes() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new("readonly-container");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);
    // 先成功保存一次，得到一份**真实存在**的容器。
    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    let original = fs::read(&path).expect("原容器字节");
    assert_eq!(&original[..4], b"PK\x03\x04");

    // 再改一处（制造未保存改动），然后把父目录改成只读。
    let track = macro_track(&dispatcher);
    propose_and_merge(&mut dispatcher, &auth, &track);
    assert!(dispatcher.domain().is_dirty(), "合并之后必须有未保存改动");
    fs::set_permissions(&scratch.dir, fs::Permissions::from_mode(0o555)).expect("只读化目录");
    let failed = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    fs::set_permissions(&scratch.dir, fs::Permissions::from_mode(0o755)).expect("恢复权限");

    // 这一条失败来自文件系统（临时文件创建不了），走 io::Error 映射 ⇒ `IO_ERROR`。
    assert_eq!(failed["status"], "error", "{failed}");
    assert_eq!(failed["error"]["code"], "IO_ERROR", "{failed}");
    // 核心：原容器**逐字节不变**。注入"原地截断写"会让这条变红。
    let after = fs::read(&path).expect("原文件必须还在");
    assert_eq!(after, original, "失败的保存破坏了原容器");
    let leftovers: Vec<String> = fs::read_dir(&scratch.dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件必须被清理: {leftovers:?}");

    // 权限恢复后同一次保存必须成功，且磁盘上仍是一个可读的容器。
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    let after = fs::read(&path).expect("读回");
    assert_ne!(after, original, "成功保存必须写出新内容");
    assert_eq!(&after[..4], b"PK\x03\x04");
    read_project_container(&after, &ContainerLimits::default()).expect("新字节仍是合法容器");
}

// ---------------------------------------------------------------------------
// 判据 6：确实是 tmp + rename（不是原地截断写）
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn save_replaces_the_target_inode_instead_of_truncating_it() {
    use std::os::unix::fs::MetadataExt as _;

    let scratch = Scratch::new("inode");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);
    let before = fs::metadata(&path).expect("原文件元数据").ino();

    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    let after = fs::metadata(&path).expect("新文件元数据").ino();
    assert_ne!(
        before, after,
        "ARCH-SEC-004: 保存必须是'新文件 + rename'（inode 变化），不能原地截断写"
    );
}

// ---------------------------------------------------------------------------
// 判据 7：裸 JSON 兼容路径
// ---------------------------------------------------------------------------

#[test]
fn bare_json_projects_still_open_on_the_compat_path() {
    let scratch = Scratch::new("compat");
    let (mut dispatcher, auth) = dispatcher();
    // `open_bare_json` 已经断言了打开响应里的 `format == "bare-json"`。
    let (path, opened) = open_bare_json(&scratch, &mut dispatcher, &auth);
    assert_eq!(
        opened["data"]["historyRestored"], false,
        "裸 JSON 里没有 history.dag: {opened}"
    );
    assert_eq!(opened["data"]["assets"], 0, "{opened}");

    // 真落盘一次 ⇒ 同一个路径变成容器。
    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_eq!(&fs::read(&path).expect("字节")[..4], b"PK\x03\x04");
    call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );

    // 再打开必须报容器形态 —— 兼容路径只影响**读**，写一律产出容器。
    let reopened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(
        reopened["data"]["format"], "yeban-container",
        "写一律产出容器, 读才需要兼容: {reopened}"
    );
    assert_eq!(
        reopened["data"]["projectDigest"], saved["data"]["projectDigest"],
        "形态变了, 工程内容不该变"
    );
}

// ---------------------------------------------------------------------------
// 判据 8：容器里的 project.json 被 project.schema.json 接受
// ---------------------------------------------------------------------------

#[test]
fn container_project_json_is_accepted_by_the_project_schema() {
    let scratch = Scratch::new("schema");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open_bare_json(&scratch, &mut dispatcher, &auth);
    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["data"]["saved"], true, "{saved}");

    // 从磁盘容器里**原样**取出 `project.json`，不改一个字节。
    let bytes = fs::read(&path).expect("容器字节");
    let archive = read_container(&bytes, &ContainerLimits::default()).expect("容器可读");
    let project_json = archive
        .get(PROJECT_JSON_NAME)
        .expect("容器必须有 project.json")
        .data
        .clone();

    // 交给与门禁**同一个**脚本：Python jsonschema 是独立实现，
    // "serde 能读回自己写的"证明不了"契约接受它"。
    let samples = scratch.join("samples");
    fs::create_dir_all(&samples).expect("建样本目录");
    fs::write(samples.join("project.from-container.json"), &project_json).expect("写样本");

    let script = repo_root()
        .join("scripts")
        .join("gates")
        .join("validate_schemas.py");
    let output = match Command::new("python3")
        .arg(&script)
        .arg("--samples-dir")
        .arg(&samples)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            println!("[skip] 无法启动 python3 ({error}): 本判据在本机显式跳过, 不伪装成通过");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "容器里的 project.json 必须被 schemas/project.schema.json 接受:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("project.from-container.json"),
        "脚本必须**真的**校验了这一份样本（否则'通过'可能是空转）:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// 判据 9：assets/{sha256} 的内容寻址端到端
// ---------------------------------------------------------------------------

#[test]
fn assets_round_trip_with_content_addressing() {
    let scratch = Scratch::new("assets");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("assets.yeban");
    // 一份"已打开"的会话（不碰文件系统），然后把一份资产放进 CAS 池。
    dispatcher
        .domain_mut()
        .open_in_memory(path.clone(), yeban_model::samples::filled_project(), false)
        .expect("注入会话");
    let payload: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    let hash = dispatcher
        .domain_mut()
        .put_asset(payload.clone())
        .expect("放资产");
    assert_eq!(
        hash,
        AssetHash::of_bytes(&payload),
        "CAS 键必须等于字节摘要"
    );
    assert_eq!(dispatcher.domain().asset_count(), 1);
    assert_eq!(
        dispatcher.domain().asset(&hash),
        Some(payload.as_slice()),
        "池里必须能按哈希取回字节"
    );

    // 保存 ⇒ 容器里必须出现 `assets/<sha256>`，条目名就是字节的 SHA-256。
    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_eq!(saved["data"]["assets"], 1, "{saved}");

    let bytes = fs::read(&path).expect("容器字节");
    let archive = read_container(&bytes, &ContainerLimits::default()).expect("容器可读");
    let expected_name = format!("assets/{hash}");
    let entry = archive.get(&expected_name).unwrap_or_else(|| {
        panic!(
            "容器里必须有 `{expected_name}`: {:?}",
            archive.names().collect::<Vec<_>>()
        )
    });
    assert_eq!(entry.data, payload, "容器里的资产字节必须逐个相同");
    assert_eq!(
        AssetHash::of_bytes(&entry.data),
        hash,
        "条目名携带的 SHA-256 必须等于解出字节的 SHA-256"
    );
    // 走模型层的工程容器读取器再验一遍 CAS 完整性（它会**重算**哈希）。
    let project_archive =
        read_project_container(&bytes, &ContainerLimits::default()).expect("工程容器可读");
    assert_eq!(project_archive.assets.len(), 1);
    assert_eq!(project_archive.assets[0].0, hash);
    assert_eq!(project_archive.assets[0].1, payload);

    // 关闭 → 重新打开 ⇒ 资产池原样回来（"放进资产 → 保存 → 重新打开"的端到端）。
    call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    let reopened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(reopened["data"]["assets"], 1, "{reopened}");
    assert_eq!(
        dispatcher.domain().asset_hashes(),
        vec![hash.clone()],
        "池的键序 = 哈希升序（BTreeMap）"
    );
    assert_eq!(
        dispatcher.domain().asset(&hash),
        Some(payload.as_slice()),
        "重新打开后资产字节必须对得上"
    );

    // 再保存一次 ⇒ 资产仍在（不是"读进来但写不回去"）。
    let saved_again = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved_again["data"]["assets"], 1, "{saved_again}");
    let bytes = fs::read(&path).expect("容器字节");
    let archive = read_container(&bytes, &ContainerLimits::default()).expect("容器可读");
    assert!(archive.get(&expected_name).is_some(), "资产必须被写回");
}

// ---------------------------------------------------------------------------
// 判据 10：截断 → 报错，不静默加载半个工程
// ---------------------------------------------------------------------------

#[test]
fn truncated_container_is_refused_instead_of_loading_half_a_project() {
    let scratch = Scratch::new("truncated");
    let (mut dispatcher, auth) = dispatcher();
    let full = minimal_container(&yeban_model::samples::filled_project(), &[]);
    let cut = &full[..full.len() / 2];
    let outcome = open_bytes(&scratch, &mut dispatcher, &auth, "cut.yeban", cut);
    assert_eq!(
        outcome["status"], "error",
        "截断的容器绝不能被静默加载: {outcome}"
    );
    assert_eq!(outcome["error"]["code"], "IO_ERROR", "{outcome}");
    assert!(
        dispatcher.domain().active_project().is_none(),
        "失败的打开不得留下半个活跃工程"
    );
}

// ---------------------------------------------------------------------------
// 判据 11：CRC 篡改 → 报错，不静默加载
// ---------------------------------------------------------------------------

#[test]
fn crc_tampered_container_is_refused_instead_of_loading_it() {
    let scratch = Scratch::new("crc");
    let (mut dispatcher, auth) = dispatcher();
    let mut bytes = minimal_container(&yeban_model::samples::filled_project(), &[]);
    // 找到第一个 local file header 的数据起点，翻转其中一个字节（JSON 文本）。
    let offset = first_local_data_offset(&bytes).expect("local header");
    bytes[offset] ^= 0x01;
    let outcome = open_bytes(&scratch, &mut dispatcher, &auth, "tampered.yeban", &bytes);
    assert_eq!(
        outcome["status"], "error",
        "被篡改的容器绝不能被静默加载: {outcome}"
    );
    assert_eq!(outcome["error"]["code"], "IO_ERROR", "{outcome}");
    assert_eq!(
        outcome["error"]["data"]["category"], "malformed-container",
        "{outcome}"
    );
}

// ---------------------------------------------------------------------------
// 判据 12：Zip-Slip 条目名 → IO_ERROR + path-traversal
// ---------------------------------------------------------------------------

#[test]
fn zip_slip_entry_name_is_refused_with_a_contract_error_code() {
    let scratch = Scratch::new("zipslip");
    let (mut dispatcher, auth) = dispatcher();
    // 写出一个**合法**容器，其中一条的名字等长可换：`aaaaaaaa` (8 字节)。
    let mut bytes = minimal_container(
        &yeban_model::samples::filled_project(),
        &[ContainerEntry::new("aaaaaaaa", b"payload".to_vec())],
    );
    let needle = b"aaaaaaaa";
    let count = bytes
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count();
    assert!(
        count >= 2,
        "local header 与 central directory 两处名字都要被改到, 实际命中 {count}"
    );
    replace_all(&mut bytes, needle, b"../../xy");

    let outcome = open_bytes(&scratch, &mut dispatcher, &auth, "slip.yeban", &bytes);
    assert_container_error(
        &outcome,
        "path-traversal",
        "MUST-GATE-006",
        "容器里的 `..` 段必须被拒绝",
    );
}

// ---------------------------------------------------------------------------
// 判据 13：不支持的压缩法 → IO_ERROR + unsupported-container-feature
// ---------------------------------------------------------------------------

#[test]
fn unsupported_compression_is_refused_with_a_contract_error_code() {
    let scratch = Scratch::new("deflate");
    let (mut dispatcher, auth) = dispatcher();
    let mut bytes = minimal_container(&yeban_model::samples::filled_project(), &[]);
    // 把每个条目的压缩法从 0 (stored) 改成 8 (deflate)：deflate 未实现 ⇒ 必须**明确报错**，
    // 绝不静默跳过、也绝不猜内容。
    patch_u16_after_signature(&mut bytes, b"PK\x03\x04", 8, 8);
    patch_u16_after_signature(&mut bytes, b"PK\x01\x02", 10, 8);

    let outcome = open_bytes(&scratch, &mut dispatcher, &auth, "deflate.yeban", &bytes);
    assert_container_error(
        &outcome,
        "unsupported-container-feature",
        "ARCH-SEC-003",
        "deflate 必须被明确拒绝",
    );
    assert!(
        outcome["error"]["data"]["containerError"]
            .as_str()
            .is_some_and(|detail| detail.contains("UnsupportedCompression")),
        "诊断必须指名是压缩法问题: {outcome}"
    );
}

// ---------------------------------------------------------------------------
// 判据 14：§5.3 布局缺失 → IO_ERROR + container-layout
// ---------------------------------------------------------------------------

#[test]
fn container_without_history_dag_is_refused_as_a_layout_error() {
    let scratch = Scratch::new("no-history");
    let (mut dispatcher, auth) = dispatcher();
    let json = serde_json::to_vec(&yeban_model::samples::filled_project()).expect("工程 JSON");
    let bytes = write_container(&[ContainerEntry::new(PROJECT_JSON_NAME, json)]).expect("写容器");
    let outcome = open_bytes(&scratch, &mut dispatcher, &auth, "partial.yeban", &bytes);
    assert_container_error(
        &outcome,
        "container-layout",
        "ARCH-SEC-003",
        "缺 history.dag 的容器必须被拒绝",
    );
}

// ---------------------------------------------------------------------------
// 判据 15：所有容器拒绝都落在契约 enum 内
// ---------------------------------------------------------------------------

#[test]
fn every_container_rejection_stays_inside_the_contract_enum() {
    let scratch = Scratch::new("codes");
    let (mut dispatcher, auth) = dispatcher();
    let good = minimal_container(&yeban_model::samples::filled_project(), &[]);

    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    // 截断
    cases.push((
        "truncated.yeban".to_owned(),
        good[..good.len() / 2].to_vec(),
    ));
    // 缺 history.dag
    let json = serde_json::to_vec(&yeban_model::samples::filled_project()).expect("JSON");
    cases.push((
        "nohist.yeban".to_owned(),
        write_container(&[ContainerEntry::new(PROJECT_JSON_NAME, json)]).expect("写容器"),
    ));
    // Zip-Slip
    let mut slip = minimal_container(
        &yeban_model::samples::filled_project(),
        &[ContainerEntry::new("aaaaaaaa", b"x".to_vec())],
    );
    replace_all(&mut slip, b"aaaaaaaa", b"../../xy");
    cases.push(("slip.yeban".to_owned(), slip));
    // deflate
    let mut deflate = minimal_container(&yeban_model::samples::filled_project(), &[]);
    patch_u16_after_signature(&mut deflate, b"PK\x03\x04", 8, 8);
    patch_u16_after_signature(&mut deflate, b"PK\x01\x02", 10, 8);
    cases.push(("deflate.yeban".to_owned(), deflate));
    // `project.json` 不是合法工程文档（CRC 由写入器算对 ⇒ 这是**布局**类失败）
    let bad_project = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, b"{ not a project }".to_vec()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"{}".to_vec()),
    ])
    .expect("写容器");
    cases.push(("bad-project.yeban".to_owned(), bad_project));

    for (name, bytes) in cases {
        let outcome = open_bytes(&scratch, &mut dispatcher, &auth, &name, &bytes);
        assert_eq!(outcome["status"], "error", "{name}: {outcome}");
        let code = outcome["error"]["code"].as_str().unwrap_or_default();
        assert_eq!(code, "IO_ERROR", "{name}: 容器拒绝一律 IO_ERROR");
        assert!(
            ErrorCode::SCHEMA_CONTRACT
                .iter()
                .any(|known| known.as_str() == code),
            "{name}: 容器拒绝产出了契约外的错误码 `{code}`"
        );
    }
}

// ---------------------------------------------------------------------------
// 判据 16：坏 history.dag 必须在打开时被拒绝（不许静默丢历史）
// ---------------------------------------------------------------------------

#[test]
fn corrupt_history_dag_is_refused_instead_of_silently_dropping_history() {
    let scratch = Scratch::new("bad-history");
    let (mut dispatcher, auth) = dispatcher();
    let json = serde_json::to_vec(&yeban_model::samples::filled_project()).expect("工程 JSON");

    // (a) 不是合法 CommitGraph JSON。
    let not_json = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, json.clone()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"{ not a graph }".to_vec()),
    ])
    .expect("写容器");
    let outcome = open_bytes(
        &scratch,
        &mut dispatcher,
        &auth,
        "bad-history.yeban",
        &not_json,
    );
    assert_eq!(outcome["status"], "error", "{outcome}");
    assert_eq!(outcome["error"]["code"], "IO_ERROR", "{outcome}");
    assert!(
        outcome["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("history.dag")),
        "诊断必须指名 history.dag: {outcome}"
    );
    assert!(
        dispatcher.domain().active_project().is_none(),
        "失败的打开不得留下活跃工程"
    );

    // (b) 合法 CommitGraph，但**没有 main 分支** —— 这种"半个历史"会让后续每个
    //     `yeban_propose_*` 都撞 CONFLICT，因此必须在这里就拒绝。
    let mut graph = yeban_model::CommitGraph::new();
    graph
        .genesis(
            yeban_model::CommitDraft::new(
                yeban_model::EntityId::new(),
                "not-main",
                "fixture",
                "孤立分支",
            )
            .with_created_at(1_760_000_000_000),
        )
        .expect("建孤立分支");
    let orphan = serde_json::to_vec(&graph).expect("图谱 JSON");
    let no_main = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, json),
        ContainerEntry::new(HISTORY_DAG_NAME, orphan),
    ])
    .expect("写容器");
    let outcome = open_bytes(&scratch, &mut dispatcher, &auth, "no-main.yeban", &no_main);
    assert_eq!(outcome["status"], "error", "{outcome}");
    assert_eq!(outcome["error"]["code"], "IO_ERROR", "{outcome}");
    assert!(
        outcome["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("main")),
        "诊断必须指名缺 main 分支: {outcome}"
    );
}

// ---------------------------------------------------------------------------
// 字节级助手
// ---------------------------------------------------------------------------

/// 第一个 local file header 的**数据起点**（`30 + name_len + extra_len`）。
fn first_local_data_offset(bytes: &[u8]) -> Option<usize> {
    let signature = b"PK\x03\x04";
    let start = bytes
        .windows(signature.len())
        .position(|window| window == signature)?;
    let name_len = u16::from_le_bytes([bytes[start + 26], bytes[start + 27]]) as usize;
    let extra_len = u16::from_le_bytes([bytes[start + 28], bytes[start + 29]]) as usize;
    Some(start + 30 + name_len + extra_len)
}

/// 把字节流里**每一处** `needle` 换成等长的 `replacement`（长度必须相等）。
fn replace_all(bytes: &mut [u8], needle: &[u8], replacement: &[u8]) {
    assert_eq!(needle.len(), replacement.len(), "替换必须等长");
    let mut hits = 0_u32;
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if bytes[index..index + needle.len()] == *needle {
            bytes[index..index + needle.len()].copy_from_slice(replacement);
            hits += 1;
            index += needle.len();
        } else {
            index += 1;
        }
    }
    assert!(hits > 0, "必须命中至少一处");
}

/// 在**每一处** `signature` 之后的固定偏移处写一个 u16（小端）。
///
/// ZIP 的 local file header 与 central directory 的字段布局是 APPNOTE 固定的，
/// 因此"签名 + 偏移"是改单个字段而不破坏其它结构的可靠方式。
fn patch_u16_after_signature(bytes: &mut [u8], signature: &[u8; 4], offset: usize, value: u16) {
    let mut index = 0;
    let mut hits = 0_u32;
    while index + 4 <= bytes.len() {
        if bytes[index..index + 4] == *signature {
            let at = index + offset;
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
            hits += 1;
            index += 4;
        } else {
            index += 1;
        }
    }
    assert!(hits > 0, "签名 {signature:?} 一处都没命中");
}

// ---------------------------------------------------------------------------
// 领域助手（只保留本文件用到的两个）
// ---------------------------------------------------------------------------

/// 取一个带宏的音轨身份。
fn macro_track(dispatcher: &Dispatcher) -> String {
    dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .tracks
        .values()
        .find(|track| !track.macros.is_empty())
        .expect("带宏的音轨")
        .id
        .to_canonical_string()
}

/// 创建一条改宏的提案并合并（让工程**真的**变一次，同时推进 `history.dag`）。
fn propose_and_merge(dispatcher: &mut Dispatcher, auth: &str, track: &str) {
    let created = call(
        dispatcher,
        auth,
        "yeban_set_macro",
        json!({ "trackId": track, "macroIndex": 0, "value": 0.42 }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let proposal = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "容器判据夹具" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
}
