//! **端到端判据**：十个 Intent 工具在真实文件系统上真的做事 [MCP-TOOL-001..010]。
//!
//! 这个文件是"从 `-32005 NOT_IMPLEMENTED` 变成真的做事"的**判决书**。
//! 每一条判据都尽量按"能变红"的方式写：用一个可以被注入破坏的量（字节、计数、
//! 错误码集合）而不是"函数返回了 `Ok`"。
//!
//! ## 与 `src/` 内单元判据的分工
//!
//! - `src/domain/**` 的单元判据测**单个规划器**（确定性身份、环路判定、发声数……）；
//! - 本文件测**整条管线 + 真实文件系统**：JSON-RPC 入口 → 鉴权 → scope →
//!   `dryRun`/幂等 → 领域执行 → 磁盘字节。
//!
//! ## 临时目录纪律
//!
//! 全部落在 `std::env::temp_dir()/<唯一子目录>`，且 [`Scratch`] 的 `Drop`
//! 负责把目录（含只读目录）恢复权限并删除 —— **绝不污染仓库**
//! （`docs/DEVELOPMENT_LEDGER.md` L16）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::jsonrpc::ErrorObject;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{ErrorCode, TOOLS};

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
            "yeban-mcp-e2e-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn text(&self, name: &str) -> String {
        self.join(name).display().to_string()
    }

    /// 写一份**确定的**工程文件，返回 `(路径, 文本, 摘要)`。
    fn write_project(&self, name: &str) -> (PathBuf, String, String) {
        let project = yeban_model::samples::filled_project();
        let mut text = serde_json::to_string_pretty(&project).expect("序列化");
        text.push('\n');
        let path = self.join(name);
        fs::write(&path, &text).expect("写工程");
        let digest = digest(&text);
        (path, text, digest)
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

/// 走 `tools/call` 并断言这是**带内** `ToolResponse`（成功或领域失败）。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let (status, outcome) = call_raw(dispatcher, auth, name, arguments);
    assert_eq!(
        status, 200,
        "{name} 必须是带内结果（领域失败也走 ToolResponse）"
    );
    outcome.unwrap_or_else(|error| panic!("{name} 不该有 JSON-RPC 层错误: {error:?}"))
}

/// 断言带内 `ToolResponse` 是某个契约错误码。
fn assert_domain_error(value: &Value, expected: &str, context: &str) {
    assert_eq!(value["status"], "error", "{context}: {value}");
    assert_eq!(value["error"]["code"], expected, "{context}: {value}");
    assert!(
        ErrorCode::SCHEMA_CONTRACT
            .iter()
            .any(|code| code.as_str() == expected),
        "{context}: {expected} 不在契约 enum 里"
    );
}

/// SHA-256 十六进制摘要（复用 `yeban-model` 的 CAS 哈希）。
fn digest(text: &str) -> String {
    yeban_model::AssetHash::of_bytes(text.as_bytes())
        .as_str()
        .to_owned()
}

/// 提案里**领域 op 本体**的列表（剥掉每提案唯一的 `origin` 信封）。
///
/// `origin.McpProposal.proposal_id` 按定义必须逐提案不同（两条提案不能抢同一条
/// 隔离分支），因此"同一请求同一载荷"要比较的是 op 本体。
fn op_bodies(proposal: &Value) -> Vec<Value> {
    proposal["ops"]
        .as_array()
        .expect("ops 必须是数组")
        .iter()
        .map(|entry| entry["op"].clone())
        .collect()
}

/// 工程的规范化文本（用于"逐字节相同"的断言）。
fn project_bytes(dispatcher: &Dispatcher) -> String {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let mut text = serde_json::to_string_pretty(project).expect("序列化");
    text.push('\n');
    text
}

/// 扮演"**另一个打开者**"：真的对 `<工程>.lock` 施加排他 OS 建议锁。
///
/// 用 `std::fs::File::try_lock`（stable 1.89.0；Unix = `flock(2)`，Windows =
/// `LockFileEx`）而不是"写一个锁文件" —— 后者在新语义下是**崩溃遗留**，
/// 必须能被接管（见 `opening_a_locked_project_is_project_locked_and_leaves_it_alone`）。
///
/// 返回的 `File` 必须活到断言结束：`Drop`（关闭 fd）就是释放建议锁。
fn hold_exclusive_advisory_lock(lock: &Path) -> fs::File {
    if let Some(parent) = lock.parent() {
        fs::create_dir_all(parent).expect("建锁文件父目录");
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)
        .expect("打开锁文件");
    // 元数据只是给人看的（持有者 PID），判定完全靠建议锁。
    let metadata = format!(
        "{{\"pid\": {}, \"lock_mode\": \"ExclusiveWrite\"}}\n",
        std::process::id()
    );
    let mut writer = &file;
    use std::io::Write as _;
    writer.write_all(metadata.as_bytes()).expect("写锁元数据");
    file.try_lock().expect("另一个打开者必须拿到排他建议锁");
    file
}

/// 打开一份最新写入的工程，返回 `(路径, 打开响应)`。
fn open(scratch: &Scratch, dispatcher: &mut Dispatcher, auth: &str) -> (PathBuf, Value) {
    let (path, _text, _digest) = scratch.write_project("demo.yeban");
    let opened = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(opened["data"]["opened"], true);
    (path, opened)
}

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

/// 创建一条提案，返回 `proposalId`。
fn propose_macro(dispatcher: &mut Dispatcher, auth: &str, track: &str, value: f64) -> String {
    let created = call(
        dispatcher,
        auth,
        "yeban_set_macro",
        json!({ "trackId": track, "macroIndex": 0, "value": value }),
    );
    assert_eq!(created["status"], "success", "{created}");
    created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned()
}

// ---------------------------------------------------------------------------
// 判据 ①：dryRun 真的不改状态
// ---------------------------------------------------------------------------

#[test]
fn dry_run_leaves_the_project_bytes_and_commit_count_untouched() {
    let scratch = Scratch::new("dryrun");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    let clip = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some())
        .expect("MIDI 片段")
        .id
        .to_canonical_string();

    let cases: Vec<(&str, Value)> = vec![
        (
            "yeban_save_project",
            json!({ "force": true, "dryRun": true }),
        ),
        ("yeban_close_project", json!({ "dryRun": true })),
        ("yeban_query_project", json!({ "limit": 4, "dryRun": true })),
        (
            "yeban_propose_section",
            json!({ "sectionName": "Chorus", "stylePreset": "synthwave", "bars": 8, "dryRun": true }),
        ),
        (
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": clip,
                "ops": [{"kind": "add", "note": {"startTick": 7680, "pitch": 72, "durationTicks": 480}}],
                "dryRun": true
            }),
        ),
        (
            "yeban_set_macro",
            json!({ "trackId": track, "macroIndex": 0, "value": 0.9, "dryRun": true }),
        ),
        (
            "yeban_render_master",
            json!({ "format": "wav", "sampleRate": 48000, "dryRun": true }),
        ),
    ];

    for (name, arguments) in cases {
        let bytes_before = project_bytes(&dispatcher);
        let digest_before = digest(&bytes_before);
        let commits_before = dispatcher.domain().commit_count();
        let proposals_before = dispatcher.domain().proposal_count();

        let (status, outcome) = call_raw(&mut dispatcher, &auth, name, arguments);
        assert_eq!(status, 200, "{name} 的 dryRun 必须是带内结果");
        let result = outcome
            .unwrap_or_else(|error| panic!("{name} 的 dryRun 不该有 JSON-RPC 层错误: {error:?}"));
        // 唯一允许的"失败"是渲染器未接线, 而那是**参数校验通过之后**的事。
        match result["status"].as_str() {
            Some("success") => {
                assert_eq!(result["data"]["stateUnchanged"], true, "{name}");
                assert!(result["data"]["preview"].is_object(), "{name}");
            }
            Some("error") => {
                let code = result["error"]["code"].as_str().unwrap_or_default();
                assert!(
                    ErrorCode::SCHEMA_CONTRACT
                        .iter()
                        .any(|known| known.as_str() == code),
                    "{name} 的 dryRun 领域失败码必须在契约里: {result}"
                );
            }
            other => panic!("{name} 的 status 非法: {other:?}"),
        }

        // ① 的核心断言：工程**逐字节**相同 + 提交数/提案数不变。
        let bytes_after = project_bytes(&dispatcher);
        assert_eq!(
            digest(&bytes_after),
            digest_before,
            "{name} 的 dryRun 改了工程内容"
        );
        assert_eq!(bytes_after, bytes_before, "{name} 的 dryRun 改了工程字节");
        assert_eq!(
            dispatcher.domain().commit_count(),
            commits_before,
            "{name} 的 dryRun 改了提交数"
        );
        assert_eq!(
            dispatcher.domain().proposal_count(),
            proposals_before,
            "{name} 的 dryRun 建了提案"
        );
        assert_eq!(
            dispatcher.idempotency_len(),
            0,
            "{name} 的 dryRun 写了幂等缓存"
        );
    }
}

#[test]
fn dry_run_reports_why_it_would_fail_instead_of_pretending_success() {
    // 没有活跃工程时, 会改状态的工具的 dryRun 必须如实报 NO_ACTIVE_PROJECT。
    let (mut dispatcher, auth) = dispatcher();
    let cases: Vec<(&str, Value)> = vec![
        ("yeban_save_project", json!({})),
        ("yeban_close_project", json!({})),
        ("yeban_query_project", json!({})),
        (
            "yeban_propose_section",
            json!({ "sectionName": "Chorus", "stylePreset": "lofi-beats", "bars": 4 }),
        ),
        (
            "yeban_edit_notes",
            json!({
                "trackId": "01J8ZQ00000000000000000001",
                "clipId": "01J8ZQ00000000000000000001",
                "ops": [{"kind": "delete", "noteId": "01J8ZQ00000000000000000001"}]
            }),
        ),
        (
            "yeban_set_macro",
            json!({ "trackId": "01J8ZQ00000000000000000001", "macroIndex": 0, "value": 0.5 }),
        ),
        (
            "yeban_render_master",
            json!({ "format": "wav", "sampleRate": 48000 }),
        ),
    ];
    for (name, arguments) in cases {
        let mut arguments = arguments;
        arguments["dryRun"] = Value::from(true);
        let result = call(&mut dispatcher, &auth, name, arguments);
        assert_domain_error(
            &result,
            "NO_ACTIVE_PROJECT",
            &format!("{name} 的 dryRun 必须如实报领域失败"),
        );
    }
}

#[test]
fn dry_run_preview_matches_what_the_real_call_commits() {
    // dryRun 的"将要发生什么"必须**真的**是接下来会发生的事（确定性身份）。
    let scratch = Scratch::new("preview");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    let arguments = json!({ "trackId": track, "macroIndex": 0, "value": 0.4 });

    let preview = call(&mut dispatcher, &auth, "yeban_set_macro", {
        let mut with_dry = arguments.clone();
        with_dry["dryRun"] = Value::from(true);
        with_dry
    });
    let planned = preview["data"]["preview"]["ops"].clone();
    assert!(planned.as_array().is_some_and(|ops| !ops.is_empty()));

    let created = call(&mut dispatcher, &auth, "yeban_set_macro", arguments);
    let committed = created["data"]["proposal"]["ops"].clone();
    let committed_ops: Vec<Value> = committed
        .as_array()
        .expect("ops")
        .iter()
        .map(|entry| entry["op"].clone())
        .collect();
    assert_eq!(
        planned,
        Value::Array(committed_ops),
        "预览的 op 必须与真调用提交的 op 逐字节相同"
    );
}

// ---------------------------------------------------------------------------
// 判据 ②：idempotencyKey 真的幂等
// ---------------------------------------------------------------------------

#[test]
fn same_idempotency_key_applies_exactly_once() {
    let scratch = Scratch::new("idem");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);

    // 用"创建提案"当副作用: 它会新增一条隔离分支（可数）。
    let arguments = json!({
        "trackId": track, "macroIndex": 0, "value": 0.33, "idempotencyKey": "same-key"
    });
    let candidates_before = dispatcher.domain().proposal_count();
    let commits_before = dispatcher.domain().commit_count();

    let first = call(&mut dispatcher, &auth, "yeban_set_macro", arguments.clone());
    let proposals_after_first = dispatcher.domain().proposal_count();
    let commits_after_first = dispatcher.domain().commit_count();
    assert_eq!(
        proposals_after_first,
        candidates_before + 1,
        "首次必须真的施加"
    );
    assert_eq!(commits_after_first, commits_before + 1, "首次必须建提交");

    let second = call(&mut dispatcher, &auth, "yeban_set_macro", arguments.clone());
    assert_eq!(second["replayed"], true, "第二次必须命中缓存");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_after_first,
        "同键不得重复建提案"
    );
    assert_eq!(
        dispatcher.domain().commit_count(),
        commits_after_first,
        "同键不得重复建提交"
    );
    // 重放的主体与首次逐字节相同（只换了信封与 id）。
    assert_eq!(first, second["response"]["result"], "重放必须返回首次结果");

    // 不同 key 视为新请求。
    let mut other = arguments.clone();
    other["idempotencyKey"] = Value::from("other-key");
    let third = call(&mut dispatcher, &auth, "yeban_set_macro", other);
    assert!(third.get("replayed").is_none(), "不同 key 不得命中缓存");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_after_first + 1,
        "不同 key 必须真的施加"
    );
    assert_eq!(dispatcher.replayed(), 1);
}

#[test]
fn idempotent_merge_does_not_apply_the_batch_twice() {
    let scratch = Scratch::new("idem-merge");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    let proposal_id = propose_macro(&mut dispatcher, &auth, &track, 0.7);
    let commits_before_merge = dispatcher.domain().commit_count();

    let arguments = json!({
        "proposalId": proposal_id, "commitMessage": "宏一次", "idempotencyKey": "merge-1"
    });
    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        arguments.clone(),
    );
    assert_eq!(first["data"]["merged"], true);
    let after_first = project_bytes(&dispatcher);
    let commits_after_first = dispatcher.domain().commit_count();
    assert_eq!(commits_after_first, commits_before_merge + 1);

    let second = call(&mut dispatcher, &auth, "yeban_merge_proposal", arguments);
    assert_eq!(second["replayed"], true);
    assert_eq!(
        dispatcher.domain().commit_count(),
        commits_after_first,
        "幂等重放不得再建提交"
    );
    assert_eq!(
        project_bytes(&dispatcher),
        after_first,
        "幂等重放不得再施加 Op::Batch"
    );
    // 领域层的幂等: 换一个 key 再合并同一条提案, 也是"已合并"。
    let third = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "再来一次" }),
    );
    assert_eq!(third["data"]["alreadyMerged"], true);
    assert_eq!(third["data"]["appliedOps"], 0);
    assert_eq!(dispatcher.domain().commit_count(), commits_after_first);
}

// ---------------------------------------------------------------------------
// 判据 ③：非法字段选择器
// ---------------------------------------------------------------------------

#[test]
fn invalid_field_selector_is_a_contract_error_not_a_panic() {
    let scratch = Scratch::new("selector");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);

    for bad in [
        json!({ "fields": ["tracks[0].id"] }),
        json!({ "fields": ["../../etc/passwd"] }),
        json!({ "fields": [""] }),
        json!({ "fields": ["Tracks"] }),
        json!({ "fields": [7] }),
    ] {
        let result = call(&mut dispatcher, &auth, "yeban_query_project", bad.clone());
        assert_domain_error(
            &result,
            "INVALID_FIELD_SELECTOR",
            &format!("选择器 {bad} 必须被拒"),
        );
        assert!(
            result["error"]["data"]["availableSelectors"].is_array(),
            "必须回报可选选择器: {result}"
        );
    }

    // 合法选择器必须真的产出稀疏视图（不是"空成功"）。
    let ok = call(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({ "fields": ["title", "tracks.name"], "limit": 2 }),
    );
    let project = &ok["data"]["project"];
    assert!(project["title"].is_string());
    assert!(project.get("bpm").is_none(), "未请求的字段不得出现");
    assert!(project["tracks"].is_object());
    assert_eq!(ok["data"]["page"]["limit"], 2);
    assert_eq!(ok["data"]["page"]["returned"], 2);
}

#[test]
fn pagination_is_bounded_and_validated() {
    let scratch = Scratch::new("paging");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);

    let total =
        call(&mut dispatcher, &auth, "yeban_query_project", json!({}))["data"]["page"]["total"]
            .as_u64()
            .expect("total");
    assert!(total > 2, "样本工程必须有多个实体: {total}");

    for bad in [
        json!({ "limit": 0 }),
        json!({ "limit": -1 }),
        json!({ "offset": -5 }),
    ] {
        let result = call(&mut dispatcher, &auth, "yeban_query_project", bad.clone());
        assert_domain_error(&result, "INVALID_PARAMETER_RANGE", &format!("{bad}"));
    }

    let clamped = call(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({ "limit": 100_000 }),
    );
    assert_eq!(clamped["data"]["page"]["limit"], 1000);
    assert_eq!(clamped["data"]["page"]["limitClamped"], true);

    // 越界 offset 是空页, 不是错误。
    let beyond = call(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({ "offset": total + 100 }),
    );
    assert_eq!(beyond["status"], "success");
    assert_eq!(beyond["data"]["page"]["returned"], 0);
    assert_eq!(beyond["data"]["page"]["hasMore"], false);
}

// ---------------------------------------------------------------------------
// 判据 ④：保存失败不得破坏原文件
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn save_into_a_read_only_directory_fails_with_io_error_and_keeps_the_original() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new("readonly-dir");
    let (mut dispatcher, auth) = dispatcher();
    let (path, original, _digest) = scratch.write_project("demo.yeban");
    call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );

    // 改一处工程内容（真合并一条提案），让"未保存标记"为真。
    let track = macro_track(&dispatcher);
    let proposal = propose_macro(&mut dispatcher, &auth, &track, 0.8);
    call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "改一下" }),
    );
    assert!(dispatcher.domain().is_dirty(), "合并之后必须有未保存改动");

    // 把父目录改成只读 —— 临时文件创建必须失败。
    fs::set_permissions(&scratch.dir, fs::Permissions::from_mode(0o555)).expect("只读化目录");
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    fs::set_permissions(&scratch.dir, fs::Permissions::from_mode(0o755)).expect("恢复权限");

    assert_domain_error(&saved, "IO_ERROR", "只读目录下的保存");
    // 核心: 原文件**逐字节不变**（注入"直接截断写"会让这条变红）。
    let after = fs::read_to_string(&path).expect("原文件必须还在");
    assert_eq!(after, original, "失败的保存破坏了原文件");
    // 也不许留下临时文件。
    let leftovers: Vec<String> = fs::read_dir(&scratch.dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件必须被清理: {leftovers:?}");

    // 权限恢复之后, 同一次保存必须成功, 且字节真的变了。
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_eq!(saved["data"]["atomic"], true);
    let after = fs::read_to_string(&path).expect("读回");
    assert_ne!(after, original, "成功保存必须写出新内容");
    assert_eq!(
        after,
        project_bytes(&dispatcher),
        "磁盘字节必须等于内存工程"
    );
    assert!(!dispatcher.domain().is_dirty(), "保存后未保存标记必须清掉");
}

#[test]
fn save_with_a_missing_parent_directory_is_an_io_error() {
    let scratch = Scratch::new("missing-parent");
    // 直接注入一份"已经在内存里打开、但目标父目录不存在"的会话
    // （走 `yeban_open_project` 会先拿到 FILE_NOT_FOUND，那是另一条判据）。
    let missing = scratch.join("nope").join("demo.yeban");
    let (mut dispatcher, auth) = dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(
            missing.clone(),
            yeban_model::samples::filled_project(),
            false,
        )
        .expect("注入");

    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_domain_error(&saved, "IO_ERROR", "父目录不存在");
    assert!(!missing.exists(), "不得凭空造出目录树");
}

// ---------------------------------------------------------------------------
// 判据 ⑤：提案的 Op 可逆（复用 yeban-model 的 invert）
// ---------------------------------------------------------------------------

#[test]
fn proposal_ops_are_reversible_through_the_model_inverse() {
    let scratch = Scratch::new("reversible");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let before = dispatcher.domain().active_project().cloned().expect("工程");
    let track = macro_track(&dispatcher);

    let proposal_id = propose_macro(&mut dispatcher, &auth, &track, 0.1);
    let record = {
        use std::str::FromStr as _;
        let id = yeban_model::EntityId::from_str(&proposal_id).expect("ULID");
        dispatcher.domain().proposal(&id).expect("记录").clone()
    };
    assert!(!record.ops.is_empty());

    // 合并 → 工程真的变了。
    call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "合并" }),
    );
    let after = dispatcher.domain().active_project().cloned().expect("工程");
    assert_ne!(after, before, "合并必须真的改工程");

    // 用 **model 的 invert** 逐步回滚（本 crate 不写第二套逆操作）。
    let mut undone = after;
    for stamped in record.ops.iter().rev() {
        stamped
            .apply_inverse(&mut undone)
            .expect("逆操作必须可构造可施加");
    }
    assert_eq!(
        serde_json::to_string(&undone).expect("序列化"),
        serde_json::to_string(&before).expect("序列化"),
        "invert 之后必须回到合并前的逐字节状态"
    );
}

// ---------------------------------------------------------------------------
// 判据 ⑥：错误码集合必须**等于**契约 enum
// ---------------------------------------------------------------------------

/// 读 `schemas/mcp-tools.schema.json` 里的 `ToolResponse.error.code.enum`。
fn contract_error_codes() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schemas")
        .join("mcp-tools.schema.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("读契约 {} 失败: {error}", path.display());
    });
    let schema: Value = serde_json::from_str(&text).expect("契约必须是 JSON");
    schema["definitions"]["ToolResponse"]["properties"]["error"]["properties"]["code"]["enum"]
        .as_array()
        .expect("enum 必须是数组")
        .iter()
        .map(|value| value.as_str().expect("enum 元素是字符串").to_owned())
        .collect()
}

/// 收集一次端到端演练里**出现过的每一个**契约错误码。
fn exercised_error_codes() -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut record = |value: &Value| {
        if let Some(code) = value["error"]["code"].as_str()
            && !seen.iter().any(|known| known == code)
        {
            seen.push(code.to_owned());
        }
    };

    let scratch = Scratch::new("codes");
    {
        let (mut dispatcher, auth) = dispatcher();

        // 没有活跃工程。
        for name in [
            "yeban_save_project",
            "yeban_close_project",
            "yeban_query_project",
            "yeban_propose_section",
            "yeban_edit_notes",
            "yeban_set_macro",
            "yeban_render_master",
        ] {
            let args = match name {
                "yeban_edit_notes" => json!({
                    "trackId": "01J8ZQ00000000000000000001",
                    "clipId": "01J8ZQ00000000000000000001",
                    "ops": [{"kind": "delete", "noteId": "01J8ZQ00000000000000000001"}]
                }),
                "yeban_propose_section" => json!({
                    "sectionName": "Chorus", "stylePreset": "lofi-beats", "bars": 8
                }),
                "yeban_set_macro" => json!({
                    "trackId": "01J8ZQ00000000000000000001", "macroIndex": 0, "value": 0.5
                }),
                "yeban_render_master" => json!({"format": "wav", "sampleRate": 48000}),
                _ => json!({}),
            };
            record(&call(&mut dispatcher, &auth, name, args));
        }
        // 文件不存在 / 目录 / 版本门。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": scratch.join("missing.yeban").display().to_string() }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": scratch.dir.display().to_string() }),
        ));
        // 坏 JSON。
        let broken = scratch.join("broken.yeban");
        fs::write(&broken, "{ oops").expect("写坏文件");
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": broken.display().to_string() }),
        ));
        // 结构非法（bpm 越界）的工程。
        let invalid = scratch.join("invalid.yeban");
        let mut value =
            serde_json::to_value(yeban_model::samples::filled_project()).expect("序列化");
        value["bpm"] = Value::from(1.0);
        fs::write(&invalid, serde_json::to_string(&value).expect("序列化")).expect("写");
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": invalid.display().to_string() }),
        ));

        // 打开一个合法工程。
        let (path, _text, _digest) = scratch.write_project("ok.yeban");
        let opened = call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": path.display().to_string() }),
        );
        assert_eq!(opened["status"], "success", "{opened}");

        // 锁被占用（同一进程再开一次，锁文件已存在）。
        let locked = call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": path.display().to_string() }),
        );
        // 同一个路径 ⇒ 幂等分支（不加锁）；换一个**真正被占用**的路径才拿 PROJECT_LOCKED。
        assert_eq!(locked["status"], "success");
        let (other_path, _text, _digest) = scratch.write_project("other.yeban");
        // 手工造一个**真的被建议锁持有**的锁文件（模拟另一个进程持有排他锁）。
        // 注意不能只写一个 JSON 文件: 那在新语义下是"崩溃遗留 ⇒ 可接管"。
        let _holder =
            hold_exclusive_advisory_lock(&PathBuf::from(format!("{}.lock", other_path.display())));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": other_path.display().to_string() }),
        ));

        let track = macro_track(&dispatcher);
        let clip = dispatcher
            .domain()
            .active_project()
            .expect("工程")
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_some())
            .expect("MIDI 片段")
            .id
            .to_canonical_string();
        let note = dispatcher
            .domain()
            .active_project()
            .expect("工程")
            .clip_pool
            .values()
            .find_map(|entry| entry.content.notes())
            .and_then(|notes| notes.keys().next().copied())
            .expect("音符")
            .to_canonical_string();

        // 选择器 / 分页。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_query_project",
            json!({ "fields": ["nope"] }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_query_project",
            json!({ "limit": 0 }),
        ));
        // 章节骨架。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_propose_section",
            json!({ "sectionName": "X", "stylePreset": "polka", "bars": 4 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_propose_section",
            json!({ "sectionName": "X", "stylePreset": "lofi-beats", "bars": 0 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_propose_section",
            json!({ "sectionName": "X", "stylePreset": "lofi-beats", "bars": 4, "scale": "H minor" }),
        ));
        // 音符编辑。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": clip,
                "ops": [{"kind": "delete", "noteId": "01J8ZQ00000000000000000009"}]
            }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": "01J8ZQ00000000000000000009", "clipId": clip,
                "ops": [{"kind": "velocity", "noteId": note, "velocity": 1}]
            }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": "01J8ZQ00000000000000000009",
                "ops": [{"kind": "velocity", "noteId": note, "velocity": 1}]
            }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": clip,
                "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 200, "durationTicks": 480}}]
            }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": clip,
                "ops": [{"kind": "nonsense"}]
            }),
        ));
        // 所有**现存**音符同时启动 ⇒ 发声数越界。
        let flood: Vec<Value> = (0..40)
            .map(|index| {
                json!({
                    "kind": "add",
                    "note": {"startTick": 0, "pitch": 60 + (index % 12), "durationTicks": 960}
                })
            })
            .collect();
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({ "trackId": track, "clipId": clip, "ops": Value::Array(flood) }),
        ));
        // 宏。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_set_macro",
            json!({ "trackId": "01J8ZQ00000000000000000009", "macroIndex": 0, "value": 0.5 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_set_macro",
            json!({ "trackId": track, "macroIndex": 99, "value": 0.5 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_set_macro",
            json!({ "trackId": track, "macroIndex": 0, "value": 7.5 }),
        ));
        // 渲染参数。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_render_master",
            json!({ "format": "mp3", "sampleRate": 48000 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_render_master",
            json!({ "format": "wav", "sampleRate": 12345 }),
        ));
        // 提案。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_merge_proposal",
            json!({ "proposalId": "01J8ZQ00000000000000000009", "commitMessage": "m" }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_reject_proposal",
            json!({ "proposalId": "01J8ZQ00000000000000000009", "reason": "r" }),
        ));
        // 合并一条被拒绝的提案 ⇒ CONFLICT。
        let proposal = propose_macro(&mut dispatcher, &auth, &track, 0.2);
        call(
            &mut dispatcher,
            &auth,
            "yeban_reject_proposal",
            json!({ "proposalId": proposal, "reason": "不要" }),
        );
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_merge_proposal",
            json!({ "proposalId": proposal, "commitMessage": "m" }),
        ));
        // 保存到只读目录 ⇒ IO_ERROR。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": scratch.join("sub").join("x.yeban").display().to_string() }),
        ));
    }
    drop(record);
    seen
}

#[test]
fn every_emitted_error_code_is_inside_the_contract_enum() {
    let contract = contract_error_codes();
    let emitted = exercised_error_codes();
    assert!(
        emitted.len() >= 10,
        "演练必须覆盖到实质数量的错误码, 实际 {emitted:?}"
    );
    let mut outside: Vec<&String> = emitted
        .iter()
        .filter(|code| !contract.contains(code))
        .collect();
    outside.sort();
    assert!(
        outside.is_empty(),
        "这些错误码不在契约 enum 里（会产出非法 ToolResponse）: {outside:?}"
    );
}

#[test]
fn implementation_error_code_catalog_equals_the_contract_enum_exactly() {
    // 判据 ⑥ 的"集合相等"这一半: 实现的契约码目录必须与 schema 的 enum
    // **集合完全相等**（双向包含 + 计数）—— 少一个/多一个都变红。
    let contract = contract_error_codes();
    let mut implemented: Vec<String> = ErrorCode::SCHEMA_CONTRACT
        .iter()
        .map(|code| code.as_str().to_owned())
        .collect();
    let mut expected = contract.clone();
    implemented.sort();
    expected.sort();
    assert_eq!(
        implemented, expected,
        "ErrorCode::SCHEMA_CONTRACT 与 schemas/mcp-tools.schema.json 的 enum 必须集合相等"
    );
    assert_eq!(implemented.len(), 20, "ADR-0001 D25 的联集 20 值");
}

#[test]
fn not_implemented_never_appears_in_a_tool_response() {
    // 实现级码只许出现在 JSON-RPC 层。唯一的实现级状况是渲染器未接线:
    // 它必须走 `Err(ErrorObject)`, 且 `data.code = NOT_IMPLEMENTED`。
    let scratch = Scratch::new("not-impl");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let (status, outcome) = call_raw(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48000 }),
    );
    assert_eq!(status, 501);
    let error = outcome.expect_err("渲染器未接线必须是实现级状况");
    assert_eq!(error.code, yeban_mcp::jsonrpc::NOT_IMPLEMENTED);
    let data = error.data.expect("data");
    assert_eq!(data["code"], "NOT_IMPLEMENTED");
    assert_eq!(data["validated"], true, "参数校验必须先于未接线");
    assert!(
        !ErrorCode::SCHEMA_CONTRACT.contains(&ErrorCode::NotImplemented),
        "NOT_IMPLEMENTED 不许混进契约 enum"
    );

    // 反过来: 坏参数是**领域失败**, 走带内 ToolResponse。
    let bad = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "mp3", "sampleRate": 48000 }),
    );
    assert_domain_error(&bad, "INVALID_PARAMETER_RANGE", "坏格式");
}

// ---------------------------------------------------------------------------
// 其它判据：十个工具都真的做事 + 生命周期 + 锁
// ---------------------------------------------------------------------------

#[test]
fn no_tool_answers_with_a_blanket_not_implemented() {
    // 本轮的核心判决: 十个工具没有一个还在一律 -32005。
    let scratch = Scratch::new("no-blanket");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    let clip = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some())
        .expect("MIDI")
        .id
        .to_canonical_string();
    let arguments: Vec<(&str, Value)> = vec![
        (
            "yeban_open_project",
            json!({ "path": scratch.text("demo.yeban") }),
        ),
        ("yeban_save_project", json!({})),
        ("yeban_query_project", json!({})),
        (
            "yeban_propose_section",
            json!({ "sectionName": "Chorus", "stylePreset": "lofi-beats", "bars": 4 }),
        ),
        (
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": clip,
                "ops": [{"kind": "add", "note": {"startTick": 7680, "pitch": 60, "durationTicks": 480}}]
            }),
        ),
        (
            "yeban_set_macro",
            json!({ "trackId": track, "macroIndex": 0, "value": 0.5 }),
        ),
        (
            "yeban_render_master",
            json!({ "format": "wav", "sampleRate": 48000 }),
        ),
        (
            "yeban_merge_proposal",
            json!({ "proposalId": "01J8ZQ00000000000000000009", "commitMessage": "m" }),
        ),
        (
            "yeban_reject_proposal",
            json!({ "proposalId": "01J8ZQ00000000000000000009", "reason": "r" }),
        ),
        // 关闭放最后: 前面的用例都要有活跃工程。
        ("yeban_close_project", json!({ "saveFirst": false })),
    ];
    assert_eq!(arguments.len(), TOOLS.len(), "十个工具都要有用例");
    for (name, arguments) in arguments {
        let (_, outcome) = call_raw(&mut dispatcher, &auth, name, arguments);
        match outcome {
            Ok(result) => {
                assert!(
                    matches!(result["status"].as_str(), Some("success" | "error")),
                    "{name} 必须是契约形状的 ToolResponse: {result}"
                );
                if let Some(code) = result["error"]["code"].as_str() {
                    assert_ne!(code, "NOT_IMPLEMENTED", "{name} 不得伪造实现级码");
                    assert!(
                        ErrorCode::SCHEMA_CONTRACT
                            .iter()
                            .any(|known| known.as_str() == code),
                        "{name} 的错误码必须在契约里: {code}"
                    );
                }
            }
            Err(error) => {
                // 唯一的合法实现级出口: 渲染器未接线。
                assert_eq!(name, "yeban_render_master", "{name} 不该有实现级错误");
                assert_eq!(error.code, yeban_mcp::jsonrpc::NOT_IMPLEMENTED);
                assert_eq!(error.data.expect("data")["validated"], true);
            }
        }
    }
}

#[test]
fn open_save_close_round_trip_preserves_bytes_on_disk() {
    let scratch = Scratch::new("round-trip");
    let (mut dispatcher, auth) = dispatcher();
    let (path, original, _digest) = scratch.write_project("demo.yeban");
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["data"]["locked"], true);
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    assert!(lock.is_file(), "排他打开必须创建锁文件");

    // 打开后立刻保存 ⇒ 内容未变 ⇒ 默认跳过（force 才有意义）。
    let skipped = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(skipped["data"]["skipped"], true);
    assert_eq!(skipped["data"]["saved"], false);

    // 真改一次工程再保存 ⇒ 磁盘字节改变。
    let track = macro_track(&dispatcher);
    let proposal = propose_macro(&mut dispatcher, &auth, &track, 0.85);
    call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "改" }),
    );
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_ne!(fs::read_to_string(&path).expect("读"), original);

    // 关闭 ⇒ 释放锁 + 清空会话。
    let closed = call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    assert_eq!(closed["data"]["closed"], true);
    assert_eq!(closed["data"]["releasedLock"], true);
    assert!(!lock.exists(), "关闭必须释放 .yeban.lock");
    assert!(dispatcher.domain().active_project().is_none());
    let again = call(&mut dispatcher, &auth, "yeban_close_project", json!({}));
    assert_domain_error(&again, "NO_ACTIVE_PROJECT", "重复关闭");
}

#[test]
fn opening_a_locked_project_is_project_locked_and_leaves_it_alone() {
    // `MUST-GATE-008` 的端到端判决: 占用 = **OS 建议锁被内核持有**,
    // 不是"锁文件存在"。因此这条判据必须由**真的持有建议锁**的第二个句柄来制造,
    // 而不是手写一个 JSON 文件（手写文件在新语义下恰好是"崩溃遗留 ⇒ 可接管"）。
    let scratch = Scratch::new("locked");
    let (mut dispatcher, auth) = dispatcher();
    let (path, original, _digest) = scratch.write_project("demo.yeban");
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    let other_opener = hold_exclusive_advisory_lock(&lock);

    let outcome = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_domain_error(&outcome, "PROJECT_LOCKED", "锁被占用");
    assert_eq!(
        outcome["error"]["data"]["lockFile"],
        lock.display().to_string()
    );
    assert!(outcome["error"]["data"]["holder"].is_string());
    assert_eq!(
        outcome["error"]["data"]["advisoryLockHeld"], true,
        "PROJECT_LOCKED 的载荷必须说明占用来自内核建议锁: {outcome}"
    );
    assert!(dispatcher.domain().active_project().is_none());
    assert_eq!(fs::read_to_string(&path).expect("读"), original);

    // 只读打开也要观察到排他锁（读者与写者互斥）。
    let read_only = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string(), "readOnly": true }),
    );
    assert_domain_error(&read_only, "PROJECT_LOCKED", "只读也要拒绝");

    // 释放建议锁之后可以打开 —— 注意此时锁文件**仍然存在**,
    // 证明占用判定来自建议锁而不是文件存在性。
    drop(other_opener);
    assert!(lock.exists(), "释放建议锁不删文件（本判据的前提）");
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(
        opened["data"]["tookOverStaleLock"], true,
        "无人持有的锁文件必须被判为陈旧并接管: {opened}"
    );
}

#[test]
fn only_one_project_can_be_active_at_a_time() {
    let scratch = Scratch::new("single-active");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let (second, _text, _digest) = scratch.write_project("second.yeban");
    let outcome = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": second.display().to_string() }),
    );
    assert_domain_error(&outcome, "CONFLICT", "同时打开两个工程");
    assert_eq!(
        outcome["error"]["data"]["activePath"],
        scratch.text("demo.yeban")
    );
}

#[test]
fn propose_section_creates_a_real_section_and_is_deterministic() {
    let scratch = Scratch::new("section");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let before = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .sections
        .len();

    let arguments = json!({
        "sectionName": "Chorus", "stylePreset": "synthwave", "bars": 8, "scale": "C minor"
    });
    // 同一基态下的两条提案：确定性身份 ⇒ 载荷必须**逐字节**相同。
    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        arguments.clone(),
    );
    assert_eq!(first["status"], "success", "{first}");
    assert_eq!(first["data"]["projectUnchanged"], true);
    assert_eq!(
        first["data"]["unwired"],
        json!(["clipPoolEntries", "routingEdges"]),
        "声部连接与片段池在 Op 全集里不可表达 —— 必须如实上报"
    );
    let second = call(&mut dispatcher, &auth, "yeban_propose_section", arguments);
    assert_eq!(
        op_bodies(&first["data"]["proposal"]),
        op_bodies(&second["data"]["proposal"]),
        "确定性身份: 同一请求必须产出同一份领域 op 载荷（含确定性身份）"
    );
    assert_ne!(
        first["data"]["proposal"]["proposalId"], second["data"]["proposal"]["proposalId"],
        "提案身份本身必须是新的（不能两条提案抢同一个分支）"
    );

    let proposal_id = first["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("id")
        .to_owned();
    // 工程内容确实没变（提案在隔离分支上）。
    assert_eq!(
        dispatcher
            .domain()
            .active_project()
            .expect("工程")
            .sections
            .len(),
        before
    );

    call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "章节骨架" }),
    );
    let project = dispatcher.domain().active_project().expect("工程");
    assert_eq!(project.sections.len(), before + 1, "合并后必须有新段落");
    assert!(project.sections.values().any(|s| s.name == "Chorus"));
    let parts = project
        .tracks
        .values()
        .filter(|track| track.name.starts_with("Chorus · "))
        .count();
    assert_eq!(parts, 4, "synthwave 预设有 4 个声部: {parts}");
    project.validate().expect("合并后必须合法");
}

#[test]
fn routing_cycles_are_refused_before_arranging() {
    let scratch = Scratch::new("cycle");
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(
        BearerToken::generate().token,
        ScopeSet::all(),
        RunMode::Production,
    );
    let mut value = serde_json::to_value(&project).expect("序列化");
    // 造一个二元环: 复用样本里已有的两个路由节点。
    let nodes = value["routing_graph"]["nodes"].clone();
    let a = nodes[0].clone();
    let b = nodes[1].clone();
    let edges = value["routing_graph"]["edges"]
        .as_array_mut()
        .expect("edges");
    for (index, (source, destination)) in [(a.clone(), b.clone()), (b, a)].into_iter().enumerate() {
        edges.push(json!({
            "id": format!("01J8ZQ{:020}", 900 + index),
            "source_node": source,
            "destination_node": destination,
            "kind": "TrackToBus",
        }));
    }
    let text = serde_json::to_string_pretty(&value).expect("序列化");
    let path = scratch.join("cyclic.yeban");
    fs::write(&path, &text).expect("写工程");
    // 模型层不判环 ⇒ 这份工程是"合法"的, 因此能打开。
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    let outcome = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "lofi-beats", "bars": 4 }),
    );
    assert_domain_error(&outcome, "CYCLE_DETECTED", "已成环的工程");
    assert!(outcome["error"]["data"]["cycle"].is_array());
}

#[test]
fn edit_notes_enforces_range_and_polyphony() {
    let scratch = Scratch::new("notes");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    let clip = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some())
        .expect("MIDI")
        .id
        .to_canonical_string();

    // 音域。
    let out_of_range = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip,
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 128, "durationTicks": 480}}]
        }),
    );
    assert_domain_error(&out_of_range, "OUT_OF_RANGE", "音高 128");

    // 平移出音域。
    let note = {
        let project = dispatcher.domain().active_project().expect("工程");
        project
            .clip_pool
            .values()
            .find_map(|entry| entry.content.notes())
            .and_then(|notes| notes.values().max_by_key(|note| note.pitch))
            .expect("音符")
            .id
            .to_canonical_string()
    };
    let moved = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip,
            "ops": [{"kind": "move", "noteId": note, "deltaTick": 0, "deltaPitch": 100}]
        }),
    );
    assert_domain_error(&moved, "OUT_OF_RANGE", "平移出音域");

    // 发声数。
    let flood: Vec<Value> = (0..40)
        .map(|index| {
            json!({
                "kind": "add",
                "note": {"startTick": 0, "pitch": 40 + (index % 40), "durationTicks": 960}
            })
        })
        .collect();
    let polyphony = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({ "trackId": track, "clipId": clip, "ops": Value::Array(flood) }),
    );
    assert_domain_error(&polyphony, "OUT_OF_RANGE", "发声数超限");
    assert_eq!(polyphony["error"]["data"]["limit"], 32);

    // 合法编辑 ⇒ 提案 + 可合并。
    let ok = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip,
            "ops": [
                {"kind": "add", "note": {"startTick": 7680, "pitch": 64, "durationTicks": 480, "velocity": 90}},
                {"kind": "velocity", "noteId": note, "velocity": 33}
            ]
        }),
    );
    assert_eq!(ok["status"], "success", "{ok}");
    let proposal = ok["data"]["proposal"]["proposalId"].as_str().expect("id");
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "两个音符动作" }),
    );
    assert_eq!(merged["data"]["appliedOps"], 2);
    let project = dispatcher.domain().active_project().expect("工程");
    let notes = project
        .clip_pool
        .get(
            &clip
                .parse::<yeban_model::EntityId>()
                .unwrap_or_else(|_| panic!("clip id 必须是 ULID")),
        )
        .and_then(|entry| entry.content.notes())
        .expect("音符集合");
    assert_eq!(notes.len(), 5, "4 个原有 + 1 个新增");
    project.validate().expect("合法");
}

#[test]
fn reject_keeps_a_traceable_record_and_blocks_merging() {
    let scratch = Scratch::new("reject");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);
    let bytes_before = project_bytes(&dispatcher);
    let proposal = propose_macro(&mut dispatcher, &auth, &track, 0.5);

    let rejected = call(
        &mut dispatcher,
        &auth,
        "yeban_reject_proposal",
        json!({ "proposalId": proposal, "reason": "织体过密" }),
    );
    assert_eq!(rejected["data"]["rejected"], true);
    assert_eq!(rejected["data"]["archived"], true);
    assert_eq!(rejected["data"]["proposal"]["status"], "rejected");
    assert_eq!(rejected["data"]["proposal"]["resolution"], "织体过密");
    assert_eq!(
        rejected["data"]["proposal"]["resolvedAt"], 1_760_000_000_000_u64,
        "注入的时钟必须进记录"
    );
    // 拒绝不改工程。
    assert_eq!(project_bytes(&dispatcher), bytes_before);
    // 记录仍在（可追溯），再拒一次是幂等的。
    assert_eq!(dispatcher.domain().proposal_count(), 1);
    let again = call(
        &mut dispatcher,
        &auth,
        "yeban_reject_proposal",
        json!({ "proposalId": proposal, "reason": "织体过密" }),
    );
    assert_eq!(again["data"]["alreadyRejected"], true);

    // 已拒绝的提案不能再合并。
    let conflict = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal, "commitMessage": "m" }),
    );
    assert_domain_error(&conflict, "CONFLICT", "拒绝后合并");

    // 未知提案。
    let missing = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": "01J8ZQ00000000000000000009", "commitMessage": "m" }),
    );
    assert_domain_error(&missing, "PROPOSAL_NOT_FOUND", "未知提案");
}

#[test]
fn set_macro_checks_track_index_and_value_range() {
    let scratch = Scratch::new("macro");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);

    let missing = call(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({ "trackId": "01J8ZQ00000000000000000009", "macroIndex": 0, "value": 0.5 }),
    );
    assert_domain_error(&missing, "TRACK_NOT_FOUND", "未知音轨");

    let index = call(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({ "trackId": track, "macroIndex": 42, "value": 0.5 }),
    );
    assert_domain_error(&index, "INDEX_OUT_OF_BOUNDS", "宏下标越界");
    assert_eq!(index["error"]["data"]["macroCount"], 1);

    let value = call(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({ "trackId": track, "macroIndex": 0, "value": 4.0 }),
    );
    assert_domain_error(&value, "OUT_OF_RANGE", "宏值越界");

    // 合法: 级联点真的进了提案（归一化）。
    let ok = call(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({ "trackId": track, "macroIndex": 0, "value": 0.25 }),
    );
    let ops = ok["data"]["proposal"]["ops"].as_array().expect("ops");
    assert_eq!(ops.len(), 3, "SetMacro + 每个映射 2 个级联点");
    assert_eq!(ops[0]["op"]["SetMacro"]["new_val"], 0.25);
    assert_eq!(ops[1]["origin"]["McpProposal"]["agent_name"], "yeban-mcp");
}

#[test]
fn spec_ids_in_responses_match_the_registry() {
    // 规范 ID 闭环: 每个工具的成功/失败响应都能追到 `MCP-TOOL-00x`。
    let scratch = Scratch::new("spec-ids");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let dry = call(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({ "dryRun": true }),
    );
    assert_eq!(dry["data"]["specId"], "MCP-TOOL-004");
    assert_eq!(dry["data"]["tool"], "yeban_query_project");
    assert_eq!(dry["data"]["requiredScope"], "app:admin");
    assert_eq!(dry["data"]["sideEffect"], "read-only");
    // `tools/list` 的注解仍然是唯一事实源。
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let tools = outcome.response.expect("响应").result.expect("result")["tools"]
        .as_array()
        .expect("tools")
        .clone();
    assert_eq!(tools.len(), TOOLS.len());
    for (descriptor, spec) in tools.iter().zip(TOOLS.iter()) {
        assert_eq!(descriptor["name"], spec.name);
        assert_eq!(descriptor["annotations"]["specId"], spec.spec_id);
    }
}

#[test]
fn save_refuses_a_read_only_session() {
    let scratch = Scratch::new("read-only-session");
    let (mut dispatcher, auth) = dispatcher();
    let (path, original, _digest) = scratch.write_project("demo.yeban");
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string(), "readOnly": true }),
    );
    assert_eq!(opened["data"]["readOnly"], true);
    assert_eq!(opened["data"]["lockMode"], "SharedRead");
    // 只读打开现在**真的持有一把共享建议锁**（锁在同一个 inode 上，所以能与
    // 排他写者互斥）。旧实现什么都不锁 ⇒ 写者照样能改，只读是口号。
    assert_eq!(opened["data"]["locked"], true);
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    assert!(lock.exists(), "只读打开必须留下共享锁的锚点（锁文件）");

    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_domain_error(&saved, "IO_ERROR", "只读会话落盘");
    assert_eq!(fs::read_to_string(&path).expect("读"), original);

    // 读者在场 ⇒ 排他写者必须被内核挡住（这正是"只读"应有的语义）。
    let blocked = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_domain_error(&blocked, "PROJECT_LOCKED", "读者在场时写者必须被拒");

    let closed = call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": true }),
    );
    assert_eq!(closed["data"]["closed"], true);
    assert_eq!(closed["data"]["saved"], false, "只读会话不该尝试保存");
    assert_eq!(closed["data"]["releasedLock"], false);

    // 会话关闭后共享锁由 `Drop` 释放；残留的锁文件**没有持有者** ⇒ 可接管。
    let reopened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");
    assert_eq!(
        reopened["data"]["tookOverStaleLock"], true,
        "读者退出后留下的锁文件不是永久锁: {reopened}"
    );
}

#[test]
fn the_workspace_is_never_used_as_scratch_space() {
    // 反证: 上面所有测试的工程路径都在 std::env::temp_dir() 之下。
    let scratch = Scratch::new("hygiene");
    let path = scratch.join("demo.yeban");
    assert!(
        path.starts_with(std::env::temp_dir()),
        "临时工程必须落在系统临时目录: {}",
        path.display()
    );
    assert!(
        !path.starts_with(Path::new(env!("CARGO_MANIFEST_DIR"))),
        "不得污染仓库"
    );
}
