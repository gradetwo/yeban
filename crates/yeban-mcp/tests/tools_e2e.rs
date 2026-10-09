//! **端到端判据**：Intent 工具在真实文件系统上真的做事 [MCP-TOOL-001..010 + D45/D46 扩展]。
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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::jsonrpc::ErrorObject;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{ErrorCode, TOOLS};
use yeban_model::{EntityId, YebanProjectV1};

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

    /// 写一份**真容器**工程文件（`ADR-0001 D43`：容器是唯一工程格式）。
    ///
    /// 旧夹具写裸 JSON，那条读路径已随 D43 删除 —— 本夹具现在与生产写出路径
    /// （`store::container_bytes`）同形：`project.json` + 空 `history.dag`。
    /// 返回 `(路径, 容器字节)`。
    fn write_project(&self, name: &str) -> (PathBuf, Vec<u8>) {
        let bytes = container_fixture(&yeban_model::samples::filled_project());
        let path = self.join(name);
        fs::write(&path, &bytes).expect("写容器工程");
        (path, bytes)
    }
}

/// 工程 → **真容器**字节（`project.json` + 空 `history.dag`，无资产）。
fn container_fixture(project: &YebanProjectV1) -> Vec<u8> {
    let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱 JSON");
    yeban_model::container::write_project_container(project, &history, &BTreeMap::new())
        .expect("写真容器")
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
    let (path, _bytes) = scratch.write_project("demo.yeban");
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

/// 把**任意**一份工程写成真容器并返回路径（用于构造"缺件"这类负样本）。
fn write_project_of(scratch: &Scratch, name: &str, project: &YebanProjectV1) -> PathBuf {
    let path = scratch.join(name);
    fs::write(&path, container_fixture(project)).expect("写容器工程");
    path
}

/// 一份**合法但没有可用配器材料**的工程：只剩音频条目，指向被删条目的摆放一并删掉。
///
/// 它必须能通过 `open` 的 `validate()`，否则测的就不是"缺材料"而是"工程坏了"。
fn project_without_midi_material() -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    project
        .clip_pool
        .retain(|_, entry| entry.content.notes().is_none());
    let remaining: Vec<yeban_model::EntityId> = project.clip_pool.keys().copied().collect();
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| remaining.contains(&placement.clip_id));
    }
    project.validate().expect("负样本自身必须合法");
    project
}

/// 合并之后，某章节**第一个**声部片段（`BTreeMap` 键序）的排序音高表。
fn generated_pitches(dispatcher: &Dispatcher, prefix: &str) -> Vec<u8> {
    let project = dispatcher.domain().active_project().expect("工程");
    let mut pitches: Vec<u8> = project
        .clip_pool
        .values()
        .filter(|entry| entry.name.starts_with(prefix))
        .find_map(|entry| entry.content.notes())
        .map(|notes| notes.values().map(|note| note.pitch).collect())
        .unwrap_or_default();
    pitches.sort_unstable();
    pitches
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
            json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
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
    // 这条判据比"预览的 op"与"提交的 op"两侧的**完整载荷** ⇒ 两侧都显式索取
    // （缺省形状只回结构化字段, 见 `yeban_mcp::tools::INCLUDE_OPS_PARAM`）。
    let arguments = json!({ "trackId": track, "macroIndex": 0, "value": 0.4, "includeOps": true });

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
// 判据 ②·合并：多父合并提交必须**落进 `history.dag`**
// ---------------------------------------------------------------------------

/// 判据：合并提案写出的**多父合并提交**经过一次真实的"存档 → 关 → 重开"仍然逐位不变。
///
/// 关闭 `docs/ledger/tools-domain-notes.md` 的 **needs-5** 第二半：合并关系由
/// `CommitGraph`（= 容器里的 `history.dag`）承担，而不是只写在 MCP 的 `Proposal` 记录里。
/// 因此判据必须**跨进程边界**成立 —— 只在内存的 `Domain` 上看一眼不算。
///
/// 量的是三个**身份集合**（不是"返回了 Ok"）：
/// 合并提交的父集合、提案提交的父集合、以及 `validate()` 的结论。
#[test]
fn the_merge_commit_survives_the_container_round_trip_with_both_parents() {
    let scratch = Scratch::new("merge-dag");
    let (mut dispatcher, auth) = dispatcher();
    let (path, _opened) = open(&scratch, &mut dispatcher, &auth);
    let track = macro_track(&dispatcher);

    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_set_macro",
        json!({ "trackId": track, "macroIndex": 0, "value": 0.7 }),
    );
    let proposal = &created["data"]["proposal"];
    let proposal_id = proposal["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let base_commit = proposal["baseCommit"]
        .as_str()
        .expect("baseCommit")
        .to_owned();
    let proposal_head = proposal["headCommit"]
        .as_str()
        .expect("headCommit")
        .to_owned();

    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "接入宏" }),
    );
    assert_eq!(merged["data"]["merged"], true, "{merged}");
    let merge_commit = merged["data"]["commit"]["id"]
        .as_str()
        .expect("合并提交身份")
        .to_owned();
    assert_eq!(
        merged["data"]["commit"]["parents"],
        json!([base_commit, proposal_head]),
        "第一父 = 合并前的活跃头, 第二父 = 提案头"
    );
    assert_eq!(merged["data"]["commit"]["isMerge"], true);

    // 存档 → 关 → 重开：走真实的容器字节（`project.json` + `history.dag` + assets）。
    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["status"], "success", "{saved}");
    let closed = call(&mut dispatcher, &auth, "yeban_close_project", json!({}));
    assert_eq!(closed["data"]["closed"], true, "{closed}");
    let reopened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(reopened["status"], "success", "{reopened}");

    let graph = dispatcher.domain().graph();
    let merge_id = EntityId::from_str(&merge_commit).expect("合并身份是 ULID");
    let head_id = EntityId::from_str(&proposal_head).expect("提案身份是 ULID");
    let base_id = EntityId::from_str(&base_commit).expect("基线身份是 ULID");
    let round_tripped = graph
        .commit(&merge_id)
        .expect("合并提交必须落进 history.dag");
    assert_eq!(
        round_tripped.parents,
        vec![base_id, head_id],
        "重开之后父集合与顺序都必须逐位相同"
    );
    assert!(round_tripped.is_merge(), "重开之后必须仍认得出这是一次合并");
    assert_eq!(
        graph.commit(&head_id).expect("提案提交").parents,
        vec![base_id],
        "分叉点（提案头 → 基线）也必须落盘"
    );
    assert_eq!(graph.validate(), Ok(()), "重开的图谱必须自洽");
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
    let (path, original) = scratch.write_project("demo.yeban");
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
    let after = fs::read(&path).expect("原文件必须还在");
    assert_eq!(after, original, "失败的保存破坏了原文件");
    // 也不许留下临时文件。
    let leftovers: Vec<String> = fs::read_dir(&scratch.dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件必须被清理: {leftovers:?}");

    // 权限恢复之后, 同一次保存必须成功, 且磁盘上真的换成了**容器字节**。
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "{saved}");
    assert_eq!(saved["data"]["atomic"], true);
    assert_eq!(
        saved["data"]["format"], "yeban-container",
        "ARCH-SEC-003: 落盘形态必须是容器"
    );
    let after = fs::read(&path).expect("读回");
    assert_ne!(after, original, "成功保存必须写出新内容");
    assert_eq!(
        &after[..4],
        b"PK\x03\x04",
        "ARCH-SEC-003: 容器必须以 ZIP 本地文件头开始"
    );
    assert!(!dispatcher.domain().is_dirty(), "保存后未保存标记必须清掉");
    // 完整的"保存 → 重新打开 → 工程逐字节相同 + 资产哈希对得上"在本文件的
    // 兄弟文件 `tests/container_store.rs` 里（它专测容器接线）。
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
                    "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 8
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
        // 非容器文件（`ADR-0001 D43` 之后没有裸 JSON 读路径）⇒ `IO_ERROR`。
        let broken = scratch.join("broken.yeban");
        fs::write(&broken, "{ oops").expect("写坏文件");
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": broken.display().to_string() }),
        ));
        // 结构非法（bpm 越界）的工程：**包进真容器**，这样被考的是工程的
        // 版本门 / `validate()`（`OUT_OF_RANGE`），而不是"容器边界"。
        let invalid = scratch.join("invalid.yeban");
        let mut value =
            serde_json::to_value(yeban_model::samples::filled_project()).expect("序列化");
        value["bpm"] = Value::from(1.0);
        let invalid_project: YebanProjectV1 =
            serde_json::from_value(value).expect("反序列化回工程");
        fs::write(&invalid, container_fixture(&invalid_project)).expect("写容器");
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({ "path": invalid.display().to_string() }),
        ));

        // 打开一个合法工程。
        let (path, _bytes) = scratch.write_project("ok.yeban");
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
        let (other_path, _bytes) = scratch.write_project("other.yeban");
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
        // ⚠ `polka` **是** `yeban-theory` 的流派 ID（D49 接线后风格清单 = `GenreLibrary::ids()`），
        // 因此"未知风格"必须用一个 theory 里真的不存在的名字。
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_propose_section",
            json!({ "sectionName": "X", "stylePreset": "yeban_unknown_style", "bars": 4 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_propose_section",
            json!({ "sectionName": "X", "stylePreset": "lo_fi_hip_hop", "bars": 0 }),
        ));
        record(&call(
            &mut dispatcher,
            &auth,
            "yeban_propose_section",
            json!({ "sectionName": "X", "stylePreset": "lo_fi_hip_hop", "bars": 4, "scale": "H minor" }),
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
    // 实现级码只许出现在 JSON-RPC 层, 而渲染器接线之后**本 crate 的工具路径上
    // 已经没有实现级状况**了: 好参数真的渲染并落盘, 坏参数是带内领域失败。
    let scratch = Scratch::new("not-impl");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let (status, outcome) = call_raw(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48000, "dryRun": true }),
    );
    assert_eq!(status, 200, "渲染已接线 ⇒ 不再有 501");
    let result = outcome.expect("必须是带内结果而不是 JSON-RPC 错误");
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["data"]["preview"]["wired"], true);
    assert!(
        !ErrorCode::SCHEMA_CONTRACT.contains(&ErrorCode::NotImplemented),
        "NOT_IMPLEMENTED 不许混进契约 enum"
    );

    // 坏参数仍然是**领域失败**, 走带内 ToolResponse。
    let bad = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "mp3", "sampleRate": 48000 }),
    );
    assert_domain_error(&bad, "INVALID_PARAMETER_RANGE", "坏格式");
}

// ---------------------------------------------------------------------------
// 其它判据：注册表里的工具都真的做事 + 生命周期 + 锁
// ---------------------------------------------------------------------------

#[test]
fn no_tool_answers_with_a_blanket_not_implemented() {
    // 本轮的核心判决: 注册表里的每个工具没有一个还在一律 -32005。
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
            json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
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
        // D45 的两条扩展：这一步只要求"走到领域实现"，撤不动是**带内领域失败**
        // （新会话一条 op 都没提交 ⇒ INDEX_OUT_OF_BOUNDS），不是实现级出口。
        ("yeban_undo", json!({})),
        ("yeban_redo", json!({})),
        // D46 的三类扩展（`ADR-0001` D46）：自动化泳道 / 设备与引擎 / 音频导入。
        // 这里只要求"走到领域实现 + 契约形状"；逐工具的"真做事 + 可逆 + dryRun"
        // 证据在 `tests/extension_tools.rs`。
        (
            "yeban_edit_automation",
            json!({
                "trackId": track,
                "lane": "TrackVolume",
                "ticks": [0, 1920],
            }),
        ),
        ("yeban_query_engine_state", json!({ "trackId": track })),
        // 音频导入：源文件不存在 ⇒ **带内** `FILE_NOT_FOUND`（不是实现级出口）。
        (
            "yeban_import_audio",
            json!({ "name": "Imported", "path": scratch.text("missing.wav") }),
        ),
        // [D56] 诊断导出：`outDir` 指到 scratch 内, 免得把测试产物写进仓库。
        (
            "yeban_export_diagnostics",
            json!({ "outDir": scratch.text("diag-out") }),
        ),
        // SMF 导出：只读且无参数（活跃工程有 MIDI 内容 ⇒ 带内成功）。
        ("yeban_export_midi", json!({})),
        // 关闭放最后: 前面的用例都要有活跃工程。
        ("yeban_close_project", json!({ "saveFirst": false })),
    ];
    assert_eq!(arguments.len(), TOOLS.len(), "每个注册的工具都要有用例");
    for (name, arguments) in arguments {
        let (status, outcome) = call_raw(&mut dispatcher, &auth, name, arguments);
        // 渲染器接线之后, 工具路径上不再有任何实现级出口: 全部必须是带内 ToolResponse。
        assert_eq!(status, 200, "{name} 不得返回 JSON-RPC 层错误");
        let result = outcome
            .unwrap_or_else(|error| panic!("{name} 不该有实现级错误 (渲染器已接线): {error:?}"));
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
}

#[test]
fn open_save_close_round_trip_preserves_bytes_on_disk() {
    let scratch = Scratch::new("round-trip");
    let (mut dispatcher, auth) = dispatcher();
    let (path, original) = scratch.write_project("demo.yeban");
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

    // 真改一次工程再保存 ⇒ 磁盘字节改变（含新提交，容器字节必然不同）。
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
    let after = fs::read(&path).expect("读");
    assert_ne!(after, original, "保存必须写出新内容");
    assert_eq!(
        &after[..4],
        b"PK\x03\x04",
        "ARCH-SEC-003: `yeban_save_project` 写出的必须是 ZIP 容器（D43 之后唯一工程格式）"
    );
    assert_eq!(saved["data"]["format"], "yeban-container");

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
    let (path, original) = scratch.write_project("demo.yeban");
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
    assert_eq!(fs::read(&path).expect("读"), original);

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
    let (second, _bytes) = scratch.write_project("second.yeban");
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
        "sectionName": "Chorus", "stylePreset": "synthwave", "bars": 8, "scale": "C minor",
        // 这条判据比的是**完整 op 载荷**的逐字节确定性 ⇒ 显式索取它
        // （缺省只回结构化字段, 见 `yeban_mcp::tools::INCLUDE_OPS_PARAM`）。
        "includeOps": true,
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
    // 判据 ⑧：曾经的假阻塞声明（`ADR-0001` D27 之后已不成立）必须消失，
    // 且它是**从真实 opKinds 推导**出来的 —— op 在，响应就不许喊缺。
    assert_eq!(
        first["data"]["unwired"],
        json!([]),
        "片段池与声部连接都已由 AddClip/AddRoutingNode/ConnectRouting 接线: {first}"
    );
    let kinds: Vec<String> = first["data"]["proposal"]["opKinds"]
        .as_array()
        .expect("opKinds")
        .iter()
        .map(|kind| kind.as_str().expect("名字").to_owned())
        .collect();
    for required in [
        "AddClip",
        "AddTrack",
        "AddClipPlacement",
        "AddRoutingNode",
        "ConnectRouting",
    ] {
        assert!(
            kinds.iter().any(|kind| kind.as_str() == required),
            "缺少 {required}: {kinds:?}"
        );
    }
    // "将要做什么"的派生清单与真实 op 数量一致。
    assert_eq!(
        first["data"]["willCreate"]["opCount"], first["data"]["proposal"]["opCount"],
        "{first}"
    );
    assert_eq!(
        first["data"]["willCreate"]["clipPoolEntries"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "synthwave 的走向全是三和弦 ⇒ theory 推出 3 声部 ⇒ 3 条片段池条目: {first}"
    );
    assert_eq!(
        first["data"]["willCreate"]["routingEdges"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "3 条声部连接: {first}"
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
    assert_eq!(
        parts, 3,
        "synthwave 的典型走向全是三和弦 ⇒ 3 个声部: {parts}"
    );
    project.validate().expect("合并后必须合法");
}

/// 判据 ③（`line/theory-wiring`，`ADR-0001` D49）：未知风格仍然 ⇒ `STYLE_NOT_FOUND`，
/// 而且候选清单**就是** `yeban-theory` 的流派 ID 清单 —— 错误语义没有因为换来源而放松。
#[test]
fn propose_section_unknown_style_is_rejected_with_the_theory_catalogue() {
    let scratch = Scratch::new("section-style");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let before = project_bytes(&dispatcher);

    let outcome = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "yeban_unknown_style", "bars": 4 }),
    );
    assert_domain_error(&outcome, "STYLE_NOT_FOUND", "未知风格预设");
    let presets: Vec<&str> = outcome["error"]["data"]["availablePresets"]
        .as_array()
        .expect("availablePresets")
        .iter()
        .filter_map(|name| name.as_str())
        .collect();
    assert!(presets.contains(&"synthwave"), "{presets:?}");
    assert!(presets.contains(&"lo_fi_hip_hop"), "{presets:?}");
    assert!(
        presets.len() > 100,
        "候选清单必须来自 theory 的整库流派: {}",
        presets.len()
    );
    assert_eq!(project_bytes(&dispatcher), before, "失败不得改工程");
    assert_eq!(dispatcher.domain().proposal_count(), 0, "失败不得留下提案");

    // 反向：theory 里**存在**的 ID 必须被接受 —— `polka` 曾经被本地 4 行表挡住，
    // 现在它来自 `GenreLibrary::ids()`。
    let accepted = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Polka", "stylePreset": "polka", "bars": 2, "dryRun": true }),
    );
    assert_eq!(accepted["status"], "success", "{accepted}");
}

/// 判据 ① + ②：合并之后工程里**真的**多出骨架（段落 / 片段池条目 / 摆放）与
/// 声部连接（路由节点 / 路由边，含方向与类型）。
#[test]
fn propose_section_merge_writes_the_skeleton_and_the_voice_routing() {
    let scratch = Scratch::new("skeleton");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let before = dispatcher.domain().active_project().cloned().expect("工程");
    let bus = before.master_bus_track_id;

    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Drop", "stylePreset": "orchestral_film_score", "bars": 6 }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "骨架" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let after = dispatcher.domain().active_project().expect("工程");

    // ① 骨架：段落 / 片段池条目 / 摆放 / 音轨的具体增量。
    assert_eq!(after.sections.len(), before.sections.len() + 1);
    assert_eq!(
        after.clip_pool.len(),
        before.clip_pool.len() + 3,
        "3 个声部"
    );
    assert_eq!(after.tracks.len(), before.tracks.len() + 3);
    let mut new_tracks = 0;
    for (id, track) in &after.tracks {
        if before.tracks.contains_key(id) {
            assert!(before.tracks[id].clips == track.clips, "既有音轨不得被改");
            continue;
        }
        new_tracks += 1;
        assert_eq!(track.kind, yeban_model::TrackKind::Midi);
        assert!(track.name.starts_with("Drop · "), "{}", track.name);
        assert_eq!(track.clips.len(), 1, "每个声部恰好一个摆放");
        let placement = track.clips.values().next().expect("摆放");
        let entry = after.clip_pool.get(&placement.clip_id).expect("片段池条目");
        assert!(
            !before.clip_pool.contains_key(&placement.clip_id),
            "摆放必须指向**新**条目"
        );
        assert!(
            entry.content.notes().is_some_and(|notes| !notes.is_empty()),
            "配器骨架的片段必须带真实材料"
        );
        assert_eq!(placement.start_tick, 15_360, "接在最后一个段落之后");
        assert_eq!(placement.duration_ticks, 3_840 * 6);
    }
    assert_eq!(new_tracks, 3);

    // ② 声部连接：节点集合 + 边的方向与类型。
    let new_nodes: Vec<_> = after
        .routing_graph
        .nodes
        .iter()
        .filter(|node| !before.routing_graph.nodes.contains(node))
        .collect();
    assert_eq!(new_nodes.len(), 3, "每个声部一个路由节点");
    let new_edges: Vec<_> = after
        .routing_graph
        .edges
        .iter()
        .filter(|(id, _)| !before.routing_graph.edges.contains_key(id))
        .map(|(_, edge)| edge)
        .collect();
    assert_eq!(new_edges.len(), 3, "每个声部一条声部连接");
    for edge in &new_edges {
        assert_eq!(
            edge.kind,
            yeban_model::RoutingKind::TrackToBus,
            "声部 → 总线的类型"
        );
        assert_eq!(edge.destination_node, bus, "方向: 声部 → 主总线");
        assert!(new_nodes.contains(&&edge.source_node), "{edge:?}");
        assert_eq!(edge.gain_db, None, "单位增益 = None");
    }
    after.validate().expect("合并后必须合法");
}

/// 判据 ③：把提案批次的 op **逐条逆过来**，工程逐字节回到调用前。
#[test]
fn propose_section_ops_are_reversible_byte_for_byte() {
    let scratch = Scratch::new("section-reverse");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let before = dispatcher.domain().active_project().cloned().expect("工程");

    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Verse", "stylePreset": "lo_fi_hip_hop", "bars": 4, "scale": "D dorian" }),
    );
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("id")
        .to_owned();
    let record = {
        use std::str::FromStr as _;
        let id = yeban_model::EntityId::from_str(&proposal_id).expect("ULID");
        dispatcher.domain().proposal(&id).expect("提案记录").clone()
    };
    assert!(
        record
            .ops
            .iter()
            .any(|stamped| stamped.op.name() == "AddClip")
    );
    assert!(
        record
            .ops
            .iter()
            .any(|stamped| stamped.op.name() == "ConnectRouting")
    );

    call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "骨架" }),
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

/// 判据 ④：`dryRun` 预览给出"将要做什么"，且工程一个字节都不动。
#[test]
fn propose_section_dry_run_previews_without_touching_bytes() {
    let scratch = Scratch::new("section-preview");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let before = project_bytes(&dispatcher);
    let commits_before = dispatcher.domain().commit_count();
    let proposals_before = dispatcher.domain().proposal_count();

    let planned = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Intro", "stylePreset": "folk", "bars": 2,
            "scale": "G major", "dryRun": true,
            // "预览必须给出完整 op 载荷"这条能力由 `includeOps` 显式打开
            // （缺省形状是结构化字段, 见 `yeban_mcp::tools::INCLUDE_OPS_PARAM`）。
            "includeOps": true
        }),
    );
    assert_eq!(planned["status"], "success", "{planned}");
    assert_eq!(planned["data"]["dryRun"], true);
    assert_eq!(planned["data"]["stateUnchanged"], true);
    let preview = &planned["data"]["preview"];
    assert_eq!(preview["kind"], "section");
    assert_eq!(preview["willCreate"]["opCount"], preview["opCount"]);
    assert_eq!(
        preview["willCreate"]["tracks"].as_array().map(Vec::len),
        Some(3),
        "folk 3 个声部: {planned}"
    );
    assert_eq!(
        preview["willCreate"]["sections"][0]["name"], "Intro",
        "{planned}"
    );
    assert!(
        preview["ops"].as_array().is_some_and(|ops| !ops.is_empty()),
        "预览必须给出完整 op 载荷"
    );

    assert_eq!(project_bytes(&dispatcher), before, "dryRun 不得改工程字节");
    assert_eq!(dispatcher.domain().commit_count(), commits_before);
    assert_eq!(dispatcher.domain().proposal_count(), proposals_before);
}

/// 判据 ⑤：片段池缺件 ⇒ **明确**的 `CLIP_NOT_FOUND`（不是 panic、不是空工程、不是 unwired）。
#[test]
fn propose_section_without_usable_material_is_a_clear_clip_not_found() {
    let scratch = Scratch::new("section-no-material");
    let (mut dispatcher, auth) = dispatcher();
    let path = write_project_of(&scratch, "naked.yeban", &project_without_midi_material());
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    let before = project_bytes(&dispatcher);

    let outcome = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
    );
    assert_domain_error(&outcome, "CLIP_NOT_FOUND", "没有可用材料");
    assert_eq!(
        outcome["error"]["data"]["missing"], "usableClipPoolEntries",
        "{outcome}"
    );
    assert!(
        outcome["error"]["data"]["why"].is_string(),
        "必须说明缺什么: {outcome}"
    );
    assert_eq!(outcome["error"]["data"]["requiredParts"], 3);
    assert!(
        outcome["data"].get("unwired").is_none(),
        "不许用 unwired 含糊带过: {outcome}"
    );
    assert_eq!(project_bytes(&dispatcher), before, "失败不得改工程");
    assert_eq!(dispatcher.domain().proposal_count(), 0, "失败不得留下提案");
}

/// **needs-8 的关闭判据**（`docs/ledger/tools-domain-notes.md:283`）：空材料工程
/// 做不了配器 ⇒ 现在有一条**材料创建**的路，且它真的把死角打开。
///
/// 这条判据故意把**同一份工程**走两遍：先证明死角存在（`CLIP_NOT_FOUND`），
/// 再用 `yeban_edit_notes {create: true}` 造出 MIDI 材料、合并、然后证明
/// `yeban_propose_section` **成功**。把 `create` 那一半摘掉，第 ③ 段立刻变红。
#[test]
fn needs_8_create_true_supplies_the_midi_material_arranging_requires() {
    let scratch = Scratch::new("needs-8");
    let (mut dispatcher, auth) = dispatcher();
    let path = write_project_of(&scratch, "naked.yeban", &project_without_midi_material());
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");

    // ① 负样本本身必须真的"没有可用材料"（否则下面测的不是 needs-8）。
    let project = dispatcher.domain().active_project().expect("工程").clone();
    assert!(
        project
            .clip_pool
            .values()
            .all(|entry| entry.content.notes().is_none()),
        "负样本里不得有 MIDI 片段"
    );
    let source_track = project
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::Midi)
        .expect("样本里必须有 MIDI 音轨")
        .id
        .to_canonical_string();

    // ② 死角：配器要材料，工程没有，任何既有工具也给不出来。
    let blocked = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
    );
    assert_domain_error(&blocked, "CLIP_NOT_FOUND", "空材料工程配器必须响亮失败");
    assert_eq!(
        blocked["error"]["data"]["missing"], "usableClipPoolEntries",
        "{blocked}"
    );

    // ③ 材料创建：`create: true` ⇒ 恰好一条 `Op::AddClip`，内容 = 两条 `add`。
    let clip_id = yeban_mcp::domain::ids::deterministic_id("clip:needs-8:e2e");
    let clip_id = clip_id.to_canonical_string();
    let bytes_before = project_bytes(&dispatcher);
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": source_track,
            "clipId": clip_id,
            "clipName": "Motif",
            "create": true,
            "includeOps": true,
            "ops": [
                {"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}},
                {"kind": "add", "note": {"startTick": 480, "pitch": 67, "durationTicks": 480}},
            ],
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    assert_eq!(created["data"]["proposal"]["kind"], "notes", "{created}");
    assert_eq!(
        created["data"]["willCreate"]["opKinds"],
        json!(["AddClip"]),
        "整批 `add` 必须折成**一条** AddClip: {created}"
    );
    assert_eq!(
        created["data"]["willCreate"]["clipPoolEntries"],
        json!([{ "id": clip_id, "name": "Motif", "notes": 2 }]),
        "{created}"
    );
    assert_eq!(
        created["data"]["projectUnchanged"], true,
        "提案不得直接改工程: {created}"
    );
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before,
        "提案不得改工程字节"
    );

    // ④ 合并 ⇒ 池子里真的多了一条 MIDI 材料。
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "材料" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let after = dispatcher.domain().active_project().expect("工程").clone();
    assert_eq!(
        after.clip_pool.len(),
        project.clip_pool.len() + 1,
        "池子里必须恰好多一条"
    );
    let entry = after
        .clip_pool
        .values()
        .find(|entry| entry.id.to_canonical_string() == clip_id)
        .expect("池里必须有新片段");
    assert_eq!(entry.name, "Motif");
    assert_eq!(
        entry.content.notes().map(std::collections::BTreeMap::len),
        Some(2)
    );
    // 材料只在池子里 —— 本工具不摆放（如实口径，不是缺陷）。
    assert!(
        after.tracks.values().all(|track| track
            .clips
            .values()
            .all(|p| p.clip_id.to_canonical_string() != clip_id)),
        "`create: true` 不摆放 (摆放是 AddClipPlacement 的事)"
    );

    // ⑤ 死角**打开**：同一个请求现在成功，且骨架的每个声部都带真实材料。
    let arranged = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
    );
    assert_eq!(
        arranged["status"], "success",
        "有了 MIDI 材料之后配器必须成功: {arranged}"
    );
    assert_eq!(
        arranged["data"]["willCreate"]["opKinds"]
            .as_array()
            .map(|kinds| kinds.iter().filter(|kind| *kind == "AddClip").count()),
        Some(3),
        "3 个声部各一条新片段: {arranged}"
    );
}

/// `create: true` 的三条**响亮失败**口径（端到端；每条都在真实文件系统上）。
#[test]
fn needs_8_create_true_refuses_what_it_cannot_honour() {
    let scratch = Scratch::new("needs-8-refusals");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let project = dispatcher.domain().active_project().expect("工程").clone();
    let source_track = project
        .tracks
        .values()
        .find(|track| track.kind == yeban_model::TrackKind::Midi)
        .expect("MIDI 音轨")
        .id
        .to_canonical_string();
    let existing_clip = project
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some())
        .expect("既有 MIDI 片段")
        .id
        .to_canonical_string();
    let fresh_clip =
        yeban_mcp::domain::ids::deterministic_id("clip:needs-8:refusals").to_canonical_string();
    let add = json!({"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}});

    let bytes_before = project_bytes(&dispatcher);
    let proposals_before = dispatcher.domain().proposal_count();

    // (a) 池里已有该身份 ⇒ CONFLICT（绝不覆盖既有片段）。
    let clash = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": source_track, "clipId": existing_clip, "create": true, "ops": [add],
        }),
    );
    assert_domain_error(&clash, "CONFLICT", "create 不得覆盖既有片段");
    assert_eq!(clash["error"]["data"]["reason"], "clipAlreadyExists");

    // (b) `create: true` 时 `delete`/`move`/`velocity` 没有可指向的音符 ⇒ 拒绝。
    for op in [
        json!({"kind": "delete", "noteId": "01ARZ3NDEKTSV4RRFFQ69G5FAV"}),
        json!({"kind": "move", "noteId": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "deltaTick": 1, "deltaPitch": 0}),
        json!({"kind": "velocity", "noteId": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "velocity": 1}),
    ] {
        let outcome = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": source_track, "clipId": fresh_clip, "create": true,
                "ops": [add.clone(), op.clone()],
            }),
        );
        assert_domain_error(&outcome, "INVALID_PARAMETER_RANGE", "create 只允许 add");
        assert_eq!(outcome["error"]["data"]["reason"], "createRequiresAddOps");
    }

    // (c) 不给 `create` ⇒ 旧行为逐字不变：片段不存在仍是 CLIP_NOT_FOUND。
    let legacy = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({ "trackId": source_track, "clipId": fresh_clip, "ops": [add] }),
    );
    assert_domain_error(&legacy, "CLIP_NOT_FOUND", "缺省路径必须与旧行为一致");

    // (d) 发声数上限在**创建路径上**也生效。这是**接线**判据：模块级的
    //     `compile_create` + `check_polyphony` 判据盖不住 `plan_edit_notes` 里那一行调用
    //     （注入证明：把那行删掉，模块级判据仍然全绿）。
    //     33 个同时发声的音符 > 上限 32 ⇒ OUT_OF_RANGE，且带 peak/limit。
    let flood: Vec<Value> = (0..=32_u8)
        .map(|index| {
            json!({
                "kind": "add",
                "note": {
                    "startTick": 0,
                    "pitch": 60 + index,
                    "durationTicks": 960,
                },
            })
        })
        .collect();
    let over = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": source_track, "clipId": fresh_clip, "create": true,
            "ops": flood,
        }),
    );
    assert_domain_error(&over, "OUT_OF_RANGE", "创建路径必须继承发声数上限");
    assert_eq!(over["error"]["data"]["peak"], 33, "{over}");
    assert_eq!(over["error"]["data"]["limit"], 32, "{over}");

    assert_eq!(project_bytes(&dispatcher), bytes_before, "失败不得改工程");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_before,
        "失败不得留下提案"
    );
}

/// 判据 ⑥：同一个 `idempotencyKey` 重复调用 ⇒ 不重复生成（复用既有幂等层）。
#[test]
fn propose_section_same_idempotency_key_does_not_duplicate_the_skeleton() {
    let scratch = Scratch::new("section-idem");
    let (mut dispatcher, auth) = dispatcher();
    open(&scratch, &mut dispatcher, &auth);
    let tracks_before = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .tracks
        .len();
    let arguments = json!({
        "sectionName": "Chorus", "stylePreset": "synthwave", "bars": 4,
        "idempotencyKey": "section-idem-1"
    });

    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        arguments.clone(),
    );
    assert_eq!(first["status"], "success", "{first}");
    assert!(first.get("replayed").is_none(), "第一次不该是重放: {first}");
    let second = call(&mut dispatcher, &auth, "yeban_propose_section", arguments);
    assert_eq!(second["replayed"], true, "第二次必须命中幂等缓存: {second}");
    assert_eq!(
        second["response"]["result"]["data"]["proposal"]["proposalId"],
        first["data"]["proposal"]["proposalId"],
        "重放必须返回**同一条**提案: {second}"
    );
    assert_eq!(
        dispatcher.domain().proposal_count(),
        1,
        "不得产生第二条提案"
    );

    let proposal_id = first["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "骨架" }),
    );
    assert_eq!(
        merged["data"]["appliedOps"], first["data"]["proposal"]["opCount"],
        "{merged}"
    );
    let project = dispatcher.domain().active_project().expect("工程");
    let parts = project
        .tracks
        .values()
        .filter(|track| track.name.starts_with("Chorus · "))
        .count();
    assert_eq!(
        parts, 3,
        "重复调用 + 合并之后仍然只有 3 个声部（不是 6）: {parts}"
    );
    assert_eq!(project.tracks.len(), tracks_before + 3);
}

/// 判据 ⑦：章节名 / 风格 / 小节数 / 调式**真的**影响了输出。
#[test]
fn propose_section_outputs_track_the_inputs() {
    // 两个独立的会话（同一份样本工程），避免两次合并互相冲突。
    let scratch_a = Scratch::new("section-inputs-c");
    let (mut dispatcher_a, auth_a) = dispatcher();
    open(&scratch_a, &mut dispatcher_a, &auth_a);
    let scratch_b = Scratch::new("section-inputs-d");
    let (mut dispatcher_b, auth_b) = dispatcher();
    open(&scratch_b, &mut dispatcher_b, &auth_b);

    let c_minor = call(
        &mut dispatcher_a,
        &auth_a,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "synthwave", "bars": 8,
                "scale": "C minor", "includeOps": true }),
    );
    let d_minor = call(
        &mut dispatcher_b,
        &auth_b,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "synthwave", "bars": 8,
                "scale": "D minor", "includeOps": true }),
    );
    assert_ne!(
        op_bodies(&c_minor["data"]["proposal"]),
        op_bodies(&d_minor["data"]["proposal"]),
        "调式必须真的进输出（移调 + 确定性身份）"
    );
    assert_eq!(
        c_minor["data"]["proposal"]["ops"][0]["op"]["SetSection"]["new_section"]["name"],
        "Chorus"
    );
    assert_eq!(
        c_minor["data"]["proposal"]["ops"][0]["op"]["SetSection"]["new_section"]["end_tick"],
        d_minor["data"]["proposal"]["ops"][0]["op"]["SetSection"]["new_section"]["end_tick"],
        "同样的 bars ⇒ 同样的跨度"
    );

    // 声部名与数量：名字是本层的**声部角色**词表（theory 没有乐器概念），
    // 数量由该流派在 theory 里的和弦构成音数推出。
    let lofi = call(
        &mut dispatcher_b,
        &auth_b,
        "yeban_propose_section",
        json!({ "sectionName": "Verse", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
    );
    assert_eq!(
        lofi["data"]["willCreate"]["tracks"]
            .as_array()
            .expect("tracks")
            .iter()
            .filter_map(|track| track["name"].as_str())
            .collect::<Vec<_>>(),
        vec!["Verse · Bass", "Verse · Alto", "Verse · Soprano"],
        "{lofi}"
    );
    // 风格 → 声部数真的来自 theory：`funk` 的走向含七和弦 `I7-IV7` ⇒ 4 声部。
    let funk = call(
        &mut dispatcher_b,
        &auth_b,
        "yeban_propose_section",
        json!({ "sectionName": "Funk", "stylePreset": "funk", "bars": 4 }),
    );
    assert_eq!(
        funk["data"]["willCreate"]["tracks"]
            .as_array()
            .expect("tracks")
            .iter()
            .filter_map(|track| track["name"].as_str())
            .collect::<Vec<_>>(),
        vec![
            "Funk · Bass",
            "Funk · Tenor",
            "Funk · Alto",
            "Funk · Soprano"
        ],
        "{funk}"
    );

    // 小节数 → 段落跨度与摆放时值（同一份工程上的第二条提案）。
    let long = call(
        &mut dispatcher_b,
        &auth_b,
        "yeban_propose_section",
        json!({ "sectionName": "Long", "stylePreset": "lo_fi_hip_hop", "bars": 16 }),
    );
    let short_span = lofi["data"]["willCreate"]["sections"][0]["endTick"]
        .as_u64()
        .expect("endTick")
        - lofi["data"]["willCreate"]["sections"][0]["startTick"]
            .as_u64()
            .expect("startTick");
    let long_span = long["data"]["willCreate"]["sections"][0]["endTick"]
        .as_u64()
        .expect("endTick")
        - long["data"]["willCreate"]["sections"][0]["startTick"]
            .as_u64()
            .expect("startTick");
    assert_eq!(long_span, short_span * 4, "16 小节 = 4 × 4 小节");
    assert_eq!(
        long["data"]["willCreate"]["placements"]
            .as_array()
            .expect("placements")
            .iter()
            .map(|placement| placement["durationTicks"].as_u64().expect("时值"))
            .collect::<Vec<_>>(),
        vec![long_span; 3],
        "每个声部的摆放覆盖整个段落"
    );

    // 合并两侧的 C / D minor 提案，用**工程里的音高**证明移调真的发生了。
    for (dispatcher, auth, response) in [
        (&mut dispatcher_a, &auth_a, &c_minor),
        (&mut dispatcher_b, &auth_b, &d_minor),
    ] {
        let proposal_id = response["data"]["proposal"]["proposalId"]
            .as_str()
            .expect("id")
            .to_owned();
        let merged = call(
            dispatcher,
            auth,
            "yeban_merge_proposal",
            json!({ "proposalId": proposal_id, "commitMessage": "骨架" }),
        );
        assert_eq!(merged["status"], "success", "{merged}");
    }
    let pitches_c = generated_pitches(&dispatcher_a, "Chorus · ");
    let pitches_d = generated_pitches(&dispatcher_b, "Chorus · ");
    assert!(!pitches_c.is_empty(), "生成的片段必须带音符");
    assert_eq!(pitches_c.len(), pitches_d.len());
    assert_ne!(pitches_c, pitches_d, "D minor 必须整体移调");
    // 口径：**等音类移调**（音级 +2）—— 逐音高类相差 2，且相对音程保持不变。
    // 不是"音级相同"（那是没移调），也不是"绝对音高 +2"（八度折叠会让它不成立）。
    for (index, pitch) in pitches_c.iter().enumerate() {
        let delta = (u16::from(pitches_d[index]) + 12 - u16::from(*pitch)) % 12;
        assert_eq!(
            delta, 2,
            "D minor = C minor 整体 +2 半音（第 {index} 音: {pitch} → {}）",
            pitches_d[index]
        );
    }
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
    let cyclic: YebanProjectV1 = serde_json::from_value(value).expect("反序列化回工程");
    let path = scratch.join("cyclic.yeban");
    fs::write(&path, container_fixture(&cyclic)).expect("写容器工程");
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
        json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
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
        json!({ "trackId": track, "macroIndex": 0, "value": 0.25, "includeOps": true }),
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
    let (path, original) = scratch.write_project("demo.yeban");
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
    assert_eq!(fs::read(&path).expect("读"), original);

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

// ---------------------------------------------------------------------------
// `yeban_open_project` 的 `create: true`：主链路的第一步
// ---------------------------------------------------------------------------

/// 磁盘上这份文件的 SHA-256（**从字节读**，不是回抄响应）。
fn file_sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("读产物");
    yeban_model::AssetHash::of_bytes(&bytes).as_str().to_owned()
}

/// **从零建工程 ⇒ 建轨/主总线 ⇒ propose_section ⇒ merge ⇒ save ⇒ export_midi
/// ⇒ render_master ⇒ close** 的端到端判据。
///
/// 为什么必须存在：在这条判据之前，主链路的第一步是断的 —— 对不存在的路径
/// `yeban_open_project` 只会 `FILE_NOT_FOUND`，而**没有任何工具**能从零建工程；
/// 模型 `Op` 全集又写不了 `master_bus_track_id`（见
/// `crates/yeban-mcp/src/domain/project_create.rs` 模块头的逐条证据）。
///
/// ## 这条判据的牙长在哪
///
/// 三个量会**在"主总线缺失"或"建工程退回旧行为"时变红**：
///
/// 1. `create` 的响应里 `seed.masterBusTrackId` 必须是 26 字符且**不是**全零；
/// 2. `masterBusInRoutingGraph` 必须是 `true`，且文档里
///    `routing_graph.nodes` 真的含那条身份；
/// 3. `yeban_render_master` 必须 `status=success` 且磁盘上的 WAV 的 SHA-256
///    等于响应里的 `sha256`（旧行为这里一定是 `RENDER_FAILED MasterNotInGraph`
///    / `TRACK_NOT_FOUND`）。
///
/// 判据里的每一步都用**工具自己的响应**推进（`proposalId` 从提案响应里取），
/// 因此它不是"照着内部 API 拼出来的"，而是一条真能复跑的调用序列。
#[test]
fn create_project_then_whole_chain_reaches_a_rendered_master_and_midi() {
    let scratch = Scratch::new("create-chain");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("fresh.yeban");
    let wav = scratch.join("fresh.master.wav");

    // ---- ① 从**不存在的路径**建工程 ----
    assert!(!path.exists(), "前提: 目标路径必须不存在");
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "Fresh",
            "bpm": 120.0,
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    assert_eq!(created["data"]["created"], true, "{created}");
    assert!(
        path.exists(),
        "建工程必须真的在磁盘上留下容器: {}",
        path.display()
    );

    // ①a 主总线：身份非全零 + **真的在路由图节点表里**。
    let bus = created["data"]["seed"]["masterBusTrackId"]
        .as_str()
        .expect("seed.masterBusTrackId")
        .to_owned();
    assert_ne!(bus, "00000000000000000000000000", "主总线不得是全零身份");
    assert_eq!(bus.len(), 26, "主总线身份必须是 26 字符 ULID: {bus}");
    assert_eq!(
        created["data"]["seed"]["masterBusInRoutingGraph"], true,
        "主总线必须在路由图里（否则 render_master 报 MasterNotInGraph）: {created}"
    );
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let bus_id =
        <yeban_model::EntityId as std::str::FromStr>::from_str(&bus).expect("解析主总线身份");
    assert!(
        project.routing_graph.nodes.contains(&bus_id),
        "文档里的 routing_graph.nodes 必须含主总线"
    );
    assert_eq!(project.master_bus_track_id, bus_id, "文档与响应必须同源");
    assert!(
        project.tracks.contains_key(&bus_id),
        "主总线必须真的是一条音轨"
    );

    // ①b 默认必须给出**能渲染的最小内容**：工程里真的有 MIDI 音符。
    let default_notes: usize = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum();
    assert!(
        default_notes > 0,
        "默认工程必须有可渲染的 MIDI 内容 (否则 render_master 报 0 帧)"
    );

    // ---- ② propose_section（配器骨架）----
    let proposed = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Intro", "stylePreset": "pop", "bars": 2 }),
    );
    assert_eq!(
        proposed["status"], "success",
        "主总线建好之后 propose_section 必须能跑: {proposed}"
    );
    assert_eq!(
        proposed["data"]["unwired"],
        json!([]),
        "骨架的全部 op 都已接线: {proposed}"
    );
    let proposal_id = proposed["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();

    // ---- ③ merge ----
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "merge Intro" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    assert_eq!(merged["data"]["merged"], true, "{merged}");

    // ---- ④ save ----
    let saved = call(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["status"], "success", "{saved}");
    assert_eq!(saved["data"]["saved"], true, "{saved}");

    // ---- ⑤ export_midi（只读，字节以 base64 回传）----
    let midi = call(&mut dispatcher, &auth, "yeban_export_midi", json!({}));
    assert_eq!(midi["status"], "success", "{midi}");
    assert!(
        midi["data"]["notes"].as_u64().unwrap_or(0) >= default_notes as u64,
        "导出的音符数不得少于默认种子: {midi}"
    );

    // ---- ⑥ render_master（旧行为在这里必然 RENDER_FAILED）----
    let rendered = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48_000 }),
    );
    assert_eq!(
        rendered["status"], "success",
        "主总线在路由图里 ⇒ 必须真的渲染出母带: {rendered}"
    );
    assert_eq!(rendered["data"]["rendered"], true, "{rendered}");
    assert_eq!(rendered["data"]["format"], "wav", "{rendered}");
    assert!(
        rendered["data"]["frames"].as_u64().unwrap_or(0) > 0,
        "母带不得是 0 帧: {rendered}"
    );
    assert_eq!(
        rendered["data"]["path"],
        json!(wav.display().to_string()),
        "缺省输出路径 = <工程 stem>.master.<format>: {rendered}"
    );
    // 产物必须真的在磁盘上，且它的 SHA-256 等于响应里的读数。
    assert!(wav.exists(), "母带必须落在磁盘上: {}", wav.display());
    assert_eq!(
        file_sha256(&wav),
        rendered["data"]["sha256"].as_str().expect("sha256"),
        "磁盘字节的摘要必须等于响应里的读数"
    );

    // ---- ⑦ close ----
    let closed = call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": true }),
    );
    assert_eq!(closed["status"], "success", "{closed}");
    assert_eq!(closed["data"]["closed"], true, "{closed}");
    assert_eq!(closed["data"]["releasedLock"], true, "{closed}");
    assert!(
        dispatcher.domain().active_path().is_none(),
        "关闭后不得留下活跃工程"
    );
}

/// 建工程必须**明确拒绝**已存在的文件（绝不静默覆盖），并逐字节保住原文件。
#[test]
fn create_refuses_to_overwrite_an_existing_project() {
    let scratch = Scratch::new("create-conflict");
    let (mut dispatcher, auth) = dispatcher();
    let (path, original) = scratch.write_project("taken.yeban");

    let refused = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string(), "create": true }),
    );
    assert_domain_error(&refused, "CONFLICT", "已存在的文件不得被新建覆盖");
    assert_eq!(
        refused["error"]["data"]["reason"], "projectAlreadyExists",
        "{refused}"
    );
    assert_eq!(
        fs::read(&path).expect("读原文件"),
        original,
        "原字节必须不变"
    );
}

/// `create: true` 与 `readOnly: true` 同给 ⇒ 既有的 `INVALID_PARAMETER_RANGE`
/// （**没有**发明新错误码，`ADR-0001` D25）。
#[test]
fn create_with_read_only_is_an_invalid_parameter_range() {
    let scratch = Scratch::new("create-readonly");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("never.yeban");
    let refused = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string(), "create": true, "readOnly": true }),
    );
    assert_domain_error(&refused, "INVALID_PARAMETER_RANGE", "create + readOnly");
    assert!(!path.exists(), "被拒的请求不得留下任何文件");
}

/// 建工程的最小内容实参**真的**改变产物（否则那几个参数就是装饰）。
#[test]
fn create_seed_arguments_change_the_document() {
    let scratch = Scratch::new("create-seed");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("seeded.yeban");
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "Seeded",
            "bpm": 96.0,
            "seed": {
                "trackCount": 2,
                "clipName": "Motif X",
                "notes": [{ "pitch": 72, "durationTicks": 480 }],
            },
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let project = dispatcher.domain().active_project().expect("活跃工程");
    assert_eq!(project.title, "Seeded");
    assert!((project.bpm - 96.0).abs() < f64::EPSILON);
    // 2 条内容轨 + 主总线。
    assert_eq!(project.tracks.len(), 3);
    assert_eq!(
        created["data"]["seed"]["trackCount"],
        json!(3),
        "读数必须从文档里数出来"
    );
    let pitches: Vec<u8> = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    assert_eq!(pitches, vec![72], "种子音符必须真的进文档");
    assert!(
        project
            .clip_pool
            .values()
            .any(|entry| entry.name == "Motif X"),
        "片段名必须真的进文档"
    );
}

/// `create: true` 的 `timeSignature` 真的改变产物，且**下游真的按它算**。
///
/// ## 缺口（实测，不是推测）
///
/// `YebanProjectV1::time_signature` 是模型层一等字段，四个地方在消费它
/// （`section_build::ticks_per_bar` 的段落长度、SMF 的拍号元事件、会话读数、
/// 宿主的时间码），但模型 `Op` 全集里**没有**写它的变体 ⇒ 模板层（`create`）
/// 是唯一落点。在加这条实参之前，工具面建出来的工程**恒为 `4/4`**。
///
/// ## 这条判据的牙长在哪（四个独立读数）
///
/// 1. `create` 的响应 `seed.timeSignature` / `seed.ticksPerBar`（从**文档**算出的读数）；
/// 2. 文档里那条种子摆放的时值 = 一小节 = `2880`（`3/4` 的 3 拍 × 960 PPQ；
///    `4/4` 是 `3840`）—— 它由 `Op::AddClipPlacement` 的载荷**逐字节**决定，
///    因此"写死 `PPQ * 4`"会在这里变红；
/// 3. `yeban_propose_section` 的段落长度 = `bars × 2880`（配器与拍号共用同一个函数）；
/// 4. SMF：共享映射编出的字节被 `yeban_midi::midi::parse_smf` 读回，
///    conductor 轨上 tick 0 的拍号必须是 `(3, 4)`（`denominator_pow2 = 2`）。
#[test]
fn create_with_a_three_four_signature_reaches_the_seed_the_arrangement_and_the_smf() {
    let scratch = Scratch::new("create-3-4");
    // 三段各要一份**干净的会话**（`create` 对"已有另一个活跃工程"是 `CONFLICT`，
    // 那是另一条判据的地盘，不该在这里被顺带触发）。名字 `dispatcher` 的绑定会遮蔽
    // 同名工厂函数，因此它必须是**最后**一条。
    let (mut plain_dispatcher, plain_auth) = dispatcher();
    let (mut refusing_dispatcher, refusing_auth) = dispatcher();
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("waltz.yeban");

    // ---- ① 建一个 3/4 工程 ----
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "Waltz",
            "timeSignature": "3/4",
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    assert_eq!(
        created["data"]["seed"]["timeSignature"],
        json!({"numerator": 3, "denominator": 4}),
        "响应必须如实回读文档里的拍号: {created}"
    );
    assert_eq!(
        created["data"]["seed"]["ticksPerBar"],
        json!(2_880),
        "{created}"
    );

    // ---- ② 文档：拍号 + 种子摆放正好一小节 ----
    let project = dispatcher.domain().active_project().expect("活跃工程");
    assert_eq!(project.time_signature.numerator, 3);
    assert_eq!(project.time_signature.denominator, 4);
    let durations: Vec<u64> = project
        .tracks
        .values()
        .flat_map(|track| {
            track
                .clips
                .values()
                .map(|placement| placement.duration_ticks)
        })
        .collect();
    assert_eq!(durations, vec![2_880], "种子摆放必须正好一个 3/4 小节");

    // ---- ③ 稀疏视图把拍号读回来（与实参无关的第二条读路径）----
    let viewed = call(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({ "fields": ["time_signature"] }),
    );
    assert_eq!(viewed["status"], "success", "{viewed}");
    assert_eq!(
        viewed["data"]["project"]["time_signature"],
        json!({"numerator": 3, "denominator": 4}),
        "{viewed}"
    );

    // ---- ④ 配器：段落长度按 3/4 算（2 小节 = 5760 tick）----
    let proposed = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Waltz", "stylePreset": "pop", "bars": 2 }),
    );
    assert_eq!(proposed["status"], "success", "{proposed}");
    let section = &proposed["data"]["willCreate"]["sections"][0];
    assert_eq!(section["startTick"], json!(0), "{proposed}");
    assert_eq!(
        section["endTick"],
        json!(5_760),
        "2 个 3/4 小节 = 2 × 2880: {proposed}"
    );
    let proposal_id = proposed["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "merge Waltz" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let merged_section = project
        .sections
        .values()
        .find(|candidate| candidate.name == "Waltz")
        .expect("段落必须真的进文档");
    assert_eq!(
        merged_section.end_tick - merged_section.start_tick,
        5_760,
        "文档里的段落长度必须等于响应里的派生读数"
    );

    // ---- ⑤ SMF：拍号进 conductor 轨，且能读回 ----
    let mapping = yeban_midi::export::export_from_project(project).expect("共享映射");
    let bytes = mapping.to_smf_bytes().expect("SMF 编码");
    let parsed = yeban_midi::midi::parse_smf(&bytes).expect("读回");
    let conductor = parsed.tempos.first().expect("tick 0 的 tempo + 拍号");
    assert_eq!(conductor.tick, 0);
    assert_eq!(conductor.numerator, Some(3), "拍号分子必须是 3");
    assert_eq!(
        conductor.denominator_pow2,
        Some(2),
        "分母 4 = 2^2（SMF 的 dd 字段）"
    );
    // 工具面自己也必须能导出（同一份映射，`extension_tools` 已钉住逐字节相等）。
    let exported = call(&mut dispatcher, &auth, "yeban_export_midi", json!({}));
    assert_eq!(exported["status"], "success", "{exported}");

    // ---- ⑥ 缺省路径仍是 4/4（逐字节等于接线之前）----
    let plain = scratch.join("plain.yeban");
    let default_created = call(
        &mut plain_dispatcher,
        &plain_auth,
        "yeban_open_project",
        json!({ "path": plain.display().to_string(), "create": true, "title": "Plain" }),
    );
    assert_eq!(default_created["status"], "success", "{default_created}");
    assert_eq!(
        default_created["data"]["seed"]["timeSignature"],
        json!({"numerator": 4, "denominator": 4})
    );
    assert_eq!(default_created["data"]["seed"]["ticksPerBar"], json!(3_840));

    // ---- ⑦ 越界拍号：响亮拒绝，且**一个字节都不写** ----
    let rejected = scratch.join("refused.yeban");
    let (status, outcome) = call_raw(
        &mut refusing_dispatcher,
        &refusing_auth,
        "yeban_open_project",
        json!({
            "path": rejected.display().to_string(),
            "create": true,
            "timeSignature": "4/5",
        }),
    );
    assert_eq!(status, 200, "领域失败必须走带内 ToolResponse");
    let refused = outcome.expect("带内 ToolResponse");
    assert_domain_error(&refused, "OUT_OF_RANGE", "分母 5 不在模型集合里");
    assert!(
        !rejected.exists(),
        "被拒的请求不得留下任何文件: {}",
        rejected.display()
    );
}

/// **只对 `create: true` 有意义**的实参，在没有 `create` 时**响亮拒绝**，绝不静默丢弃。
///
/// 为什么这条必须存在：四个键（`title` / `bpm` / `timeSignature` / `seed`）都是
/// "模板层"的东西，打开已有工程时它们一个字节也改不了。若静默忽略，调用方会得到
/// `success`，却以为 `timeSignature: "3/4"` 生效了 —— 那正是本 crate 反复立过的
/// 纪律要拦住的事（`yeban_import_audio` 的 `placementWithoutTrack` 是同一款处置）。
#[test]
fn create_only_arguments_are_refused_when_create_is_absent() {
    let scratch = Scratch::new("create-only");
    let (path, _bytes) = scratch.write_project("existing.yeban");

    for (name, value) in [
        ("title", json!("Nope")),
        ("bpm", json!(96.0)),
        ("timeSignature", json!("3/4")),
        ("seed", json!({"trackCount": 2})),
    ] {
        let (mut dispatcher, auth) = dispatcher();
        let mut arguments = json!({ "path": path.display().to_string() });
        arguments[name] = value;
        let refused = call(&mut dispatcher, &auth, "yeban_open_project", arguments);
        assert_domain_error(&refused, "INVALID_PARAMETER_RANGE", name);
        assert_eq!(
            refused["error"]["data"]["reason"],
            json!("createOnlyParameter"),
            "`{name}` 必须报出机器可读的理由: {refused}"
        );
        assert_eq!(
            refused["error"]["data"]["parameters"],
            json!([name]),
            "被拒的键必须逐个列出: {refused}"
        );
        assert!(
            dispatcher.domain().active_project().is_none(),
            "`{name}`: 被拒的请求不得打开任何工程"
        );
    }

    // 对照组：同样的工程、不带那四个键 ⇒ 正常打开（证明上面红的是"多给了键"）。
    let (mut dispatcher, auth) = dispatcher();
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    assert_eq!(opened["data"]["opened"], true, "{opened}");
}

/// 建工程与既有幂等层的行为一致：同一个非空 `idempotencyKey` 只建**一次**
/// （第二次拿到 `replayed: true` 信封 + 首次的载荷），文件字节一位不改。
///
/// 为什么这条必须存在：`create: true` 是**有副作用**的（写一个新文件）。
/// 幂等层的价值正是在这种调用上 —— "同键不重复施加"是结构性的（命中缓存就
/// 走不到 `domain::execute`），但这条判据把"它真的适用于新建路径"钉成事实。
#[test]
fn create_project_respects_the_idempotency_key() {
    let scratch = Scratch::new("create-idem");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("idem-create.yeban");
    let arguments = json!({
        "path": path.display().to_string(),
        "create": true,
        "title": "Idem",
        "idempotencyKey": "create-once",
    });
    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        arguments.clone(),
    );
    assert_eq!(first["status"], "success", "{first}");
    assert!(path.exists(), "首次必须真的落盘");
    let bytes_after_first = fs::read(&path).expect("读新建的工程");

    let second = call(&mut dispatcher, &auth, "yeban_open_project", arguments);
    assert_eq!(second["replayed"], true, "第二次必须命中缓存: {second}");
    // 重放的主体与首次逐字节相同（只换了信封与 id）。
    assert_eq!(first, second["response"]["result"], "重放必须返回首次结果");
    assert_eq!(
        fs::read(&path).expect("重读"),
        bytes_after_first,
        "命中缓存不得重写文件"
    );
    assert_eq!(dispatcher.replayed(), 1);
}

/// 没有 `idempotencyKey` 时，第二次 `create` 走的是**存在性拒绝**（不是缓存）：
/// 幂等层只在给了键时介入，因此"重复建同一个路径"是一个明确的 `CONFLICT`。
#[test]
fn create_without_a_key_rejects_the_second_attempt() {
    let scratch = Scratch::new("create-idem-none");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("twice.yeban");
    let arguments = json!({ "path": path.display().to_string(), "create": true });
    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        arguments.clone(),
    );
    assert_eq!(first["status"], "success", "{first}");
    // ⚠ 第二次调用会撞上"已有另一个活跃工程"这条**同样**正确的前置检查
    // （与普通打开同口径）；把会话清干净才能测到**存在性**那条。
    let closed = call(&mut dispatcher, &auth, "yeban_close_project", json!({}));
    assert_eq!(closed["data"]["closed"], true, "{closed}");
    let second = call(&mut dispatcher, &auth, "yeban_open_project", arguments);
    assert_domain_error(&second, "CONFLICT", "第二次建同一个路径");
    assert_eq!(
        second["error"]["data"]["reason"], "projectAlreadyExists",
        "{second}"
    );
}

/// **概率触发在工具面上可达**：`yeban_edit_notes` 的 `add.note.probability` 真的
/// 进了工程（合并后逐字段可读），并且**真的可回退**（撤销逐字节复原）。
///
/// 改动之前 `parse_note` 根本不读这个字段 ⇒ 模型的 `probability` 能力在 MCP 工具面
/// 上不可达（渲染器也忽略它）。这条判据从**工具调用**开始，因此注入"删掉这个字段的
/// 解析"会立刻变红。
#[test]
fn edit_notes_add_carries_probability_into_the_project_and_undo_restores_it() {
    let scratch = Scratch::new("probability-e2e");
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

    // 越界必须先响亮失败（不是静默夹紧到 1.0）。
    let bad = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip,
            "ops": [{"kind": "add", "note": {
                "startTick": 0, "pitch": 60, "durationTicks": 480, "probability": 1.5
            }}]
        }),
    );
    assert_domain_error(&bad, "OUT_OF_RANGE", "probability 1.5");
    assert_eq!(bad["error"]["data"]["field"], "probability", "{bad}");

    let bytes_before = project_bytes(&dispatcher);
    let note_id = yeban_mcp::domain::ids::deterministic_id("note:probability:e2e");
    let note_id = note_id.to_canonical_string();
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip, "includeOps": true,
            "ops": [{"kind": "add", "note": {
                "id": note_id, "startTick": 0, "pitch": 60,
                "durationTicks": 480, "probability": 0.25
            }}]
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    assert_eq!(
        created["data"]["proposal"]["ops"][0]["op"]["AddNote"]["note"]["probability"], 0.25,
        "提案的 op 载荷必须带着概率: {created}"
    );
    assert_eq!(project_bytes(&dispatcher), bytes_before, "提案不得改工程");

    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "概率音符" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let note = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values())
        .find(|note| note.id.to_canonical_string() == note_id)
        .expect("新音符必须在池里的那个片段里");
    assert_eq!(note.probability, Some(0.25), "合并后工程里必须带着概率");

    // 可回退：撤销一次 ⇒ 逐字节回到提案之前的工程。
    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before,
        "撤销必须逐字节复原 (概率字段随 AddNote 一起可逆)"
    );
}

/// **连击与微时序在工具面上可达，且被母带渲染器真的消费**：
/// `yeban_edit_notes` 的 `add.note.ratchet` / `add.note.microTimingTicks` 进工程
/// （合并后逐字段可读），渲染响应的 `data.ratchet` 与逐源 `notesRatcheted` 反映它，
/// 撤销逐字节复原。
///
/// 改动之前 `parse_note` 根本不读这两个字段（**静默丢弃**），而渲染器**已经**按模型
/// 语义展开连击 ⇒ 已实现的能力在 17 个工具的面上不可达。这条判据从工具调用一路走到
/// 磁盘上的 WAV，因此注入"删掉这两个字段的解析"会当场变红。
///
/// 同一判据里还有两条**响亮失败**（与顶层实参同一条纪律：拼错的参数不许静默忽略）：
/// `ratchet: 4.5` 是形状错 ⇒ `INVALID_PARAMETER_RANGE`；`note.slyde` 是未知键 ⇒
/// `INVALID_PARAMETER_RANGE`，`data` 带 `supportedNoteFields`。
#[test]
fn edit_notes_ratchet_and_micro_timing_reach_the_rendered_master() {
    let scratch = Scratch::new("expressive-e2e");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("expressive.yeban");
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "Expressive",
            "bpm": 120.0,
        }),
    );
    assert_eq!(created["status"], "success", "{created}");

    // 必须挑**真的被摆放**的 MIDI 片段: 只进池子不摆放的片段渲染器不会遍历。
    let (track, clip) = {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        project
            .tracks
            .values()
            .find_map(|track| {
                track.clips.values().find_map(|placement| {
                    let entry = project.clip_pool.get(&placement.clip_id)?;
                    entry.content.notes().is_some().then(|| {
                        (
                            track.id.to_canonical_string(),
                            entry.id.to_canonical_string(),
                        )
                    })
                })
            })
            .expect("默认种子必须有一条被摆放的 MIDI 片段")
    };

    // ---- ① 形状错与未知键都必须**响亮**失败 ----
    let half = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip,
            "ops": [{"kind": "add", "note": {
                "startTick": 0, "pitch": 60, "durationTicks": 240, "ratchet": 4.5
            }}]
        }),
    );
    assert_domain_error(&half, "INVALID_PARAMETER_RANGE", "ratchet 必须是整数");

    let typo = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip,
            "ops": [{"kind": "add", "note": {
                "startTick": 0, "pitch": 60, "durationTicks": 240, "slyde": 1
            }}]
        }),
    );
    assert_domain_error(&typo, "INVALID_PARAMETER_RANGE", "未知键不得静默丢弃");
    assert_eq!(typo["error"]["data"]["field"], "slyde", "{typo}");
    let supported = typo["error"]["data"]["supportedNoteFields"]
        .as_array()
        .expect("supportedNoteFields 必须是数组");
    for expected in [
        "id",
        "startTick",
        "pitch",
        "durationTicks",
        "velocity",
        "probability",
        "ratchet",
        "microTimingTicks",
    ] {
        assert!(
            supported.iter().any(|value| value == expected),
            "支持集合必须含 {expected}: {typo}"
        );
    }
    // 越界（1..=16 / -240..=240）也是 `OUT_OF_RANGE`，不是静默夹紧。
    for (field, bad) in [
        ("ratchet", json!(0)),
        ("ratchet", json!(17)),
        ("microTimingTicks", json!(241)),
    ] {
        let out = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({
                "trackId": track, "clipId": clip,
                "ops": [{"kind": "add", "note": {
                    "startTick": 0, "pitch": 60, "durationTicks": 240, field: bad
                }}]
            }),
        );
        assert_domain_error(&out, "OUT_OF_RANGE", field);
        assert_eq!(out["error"]["data"]["field"], field, "{out}");
    }

    // ---- ② 合法值 ⇒ 提案真的带着它们，且不动工程字节 ----
    let bytes_before = project_bytes(&dispatcher);
    let note_id = yeban_mcp::domain::ids::deterministic_id("note:ratchet:e2e");
    let note_id = note_id.to_canonical_string();
    let edited = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": clip, "includeOps": true,
            "ops": [{"kind": "add", "note": {
                "id": note_id, "startTick": 0, "pitch": 96,
                "durationTicks": 240, "ratchet": 4, "microTimingTicks": -12
            }}]
        }),
    );
    assert_eq!(edited["status"], "success", "{edited}");
    let payload = &edited["data"]["proposal"]["ops"][0]["op"]["AddNote"]["note"];
    assert_eq!(payload["ratchet"], 4, "提案 op 载荷必须带着连击: {edited}");
    // 工具面的实参名是 camelCase（`microTimingTicks`），落进工程的是模型自己的
    // 持久化键（`micro_timing_ticks`）—— 两处名字不同是**同一份字段**，不是两份。
    assert_eq!(payload["micro_timing_ticks"], -12, "{edited}");
    assert_eq!(project_bytes(&dispatcher), bytes_before, "提案不得改工程");

    // ---- ③ 合并 ⇒ 工程里逐字段可读 ----
    let proposal_id = edited["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "连击 + 微时序" }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let persisted = dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values())
        .find(|note| note.id.to_canonical_string() == note_id)
        .expect("新音符必须在池里的那个片段里");
    assert_eq!(persisted.ratchet, Some(4), "合并后工程里必须带着连击");
    assert_eq!(
        persisted.micro_timing_ticks,
        Some(-12),
        "合并后工程里必须带着微时序"
    );

    // ---- ④ 母带渲染器**真的**消费它们 ----
    let rendered = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48_000 }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    assert!(
        rendered["data"]["ratchet"]["notesExpanded"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "至少一个音符必须被登记为连击: {rendered}"
    );
    assert!(
        rendered["data"]["ratchet"]["pulses"].as_u64().unwrap_or(0) >= 4,
        "连击脉冲数必须真的展开: {rendered}"
    );
    let ratcheted: u64 = rendered["data"]["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|source| source["notesRatcheted"].as_u64().unwrap_or(0))
        .sum();
    assert!(ratcheted >= 1, "逐源读数必须反映连击: {rendered}");
    let wav = scratch.join("expressive.master.wav");
    assert!(wav.exists(), "母带必须落在磁盘上: {}", wav.display());
    assert_eq!(
        file_sha256(&wav),
        rendered["data"]["sha256"].as_str().expect("sha256"),
        "磁盘字节的摘要必须等于响应里的读数"
    );

    // ---- ⑤ 可回退：撤销一次 ⇒ 逐字节回到提案之前的工程 ----
    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before,
        "撤销必须逐字节复原 (连击与微时序随 AddNote 一起可逆)"
    );
}

/// 合并上一步拿到的提案（提案类工具的下一步永远是这一步；判据里出现多次，抽成一行）。
///
/// # Panics
///
/// 提案响应不含 `proposalId`，或合并失败。
fn merge(dispatcher: &mut Dispatcher, auth: &str, proposal: &Value, message: &str) {
    let proposal_id = proposal["data"]["proposal"]["proposalId"]
        .as_str()
        .unwrap_or_else(|| panic!("提案必须有 proposalId: {proposal}"))
        .to_owned();
    let merged = call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": message }),
    );
    assert_eq!(merged["status"], "success", "{merged}");
}

/// **needs-6 的关闭判据**（`docs/ledger/mcp-tools-expansion-notes.md` §6）：
/// "没有『放置/引用片段』的工具" —— `Op::AddClipPlacement` 在 MCP 侧只有三个写者
/// （`yeban_open_project` 的 `seed`、`yeban_propose_section` **自建**的声部、
/// `yeban_import_audio` 的**音频**片段），而**已经在 `clip_pool` 里**的片段
/// （尤其 `create: true` 刚建出来的 MIDI 材料）**没有任何工具**能摆到音轨上。
/// 渲染器只遍历 `track.clips` ⇒ 那些材料一帧都不出声。
///
/// 这条判据从工具调用一路走到**磁盘上的 WAV**，因此"摆放真的改变了母带"是实测的，
/// 不是声明的：
///
/// 1. **阴性对照**（这条判据的承重结构）：先把材料建进池子、**不摆放**，
///    此时母带字节必须与建材料**之前**逐字节相同（`sha256` 相等、帧数相等）——
///    这一条钉住"池子里的片段不出声"这个前提本身；
/// 2. 再用 `placement` 把它摆到时间轴上 ⇒ 母带 `sha256` 必须**变**且帧数**变长**；
/// 3. 摆放逐字段可读（`startTick`/`durationTicks`/`muted`/身份）；
/// 4. 撤销逐字节复原。
///
/// 另有三条**响亮失败**（同一判据内，全部要求工程字节不变）：`placement` 与
/// `create: true` 同给 / `placement` 里的未知键 / 同一摆放身份重复提交。
#[test]
fn needs_6_placement_puts_pool_material_on_the_timeline_and_the_master_hears_it() {
    let scratch = Scratch::new("needs-6-placement");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("placement.yeban");
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "Placement",
            "bpm": 120.0,
        }),
    );
    assert_eq!(created["status"], "success", "{created}");

    // 种子里**已经摆好**的 MIDI 片段所在的那条音轨（渲染器只看得到被摆放的东西）。
    let track = {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        project
            .tracks
            .values()
            .find_map(|track| {
                track
                    .clips
                    .values()
                    .any(|placement| {
                        project
                            .clip_pool
                            .get(&placement.clip_id)
                            .is_some_and(|entry| entry.content.notes().is_some())
                    })
                    .then(|| track.id.to_canonical_string())
            })
            .expect("默认种子必须有一条被摆放的 MIDI 片段")
    };

    // ---- ① 阴性对照: 材料在池子里但没摆放 ⇒ 母带一位都不变 ----
    let rendered_before = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48_000 }),
    );
    assert_eq!(rendered_before["status"], "success", "{rendered_before}");
    let sha_before = rendered_before["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let frames_before = rendered_before["data"]["frames"].as_u64().expect("frames");
    // 逐源**排程到的音符数**（判据用它证明新片的音符真的进了排程，而不只是"文件变长了"）。
    let scheduled_notes = |rendered: &Value| -> u64 {
        rendered["data"]["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .map(|source| source["notes"].as_u64().unwrap_or(0))
            .sum()
    };
    let notes_before = scheduled_notes(&rendered_before);

    let pool_clip =
        yeban_mcp::domain::ids::deterministic_id("clip:needs-6:pool-only").to_canonical_string();
    let material = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "clipName": "PoolOnly", "create": true,
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 100, "durationTicks": 480}}],
        }),
    );
    assert_eq!(material["status"], "success", "{material}");
    merge(&mut dispatcher, &auth, &material, "池子材料");

    {
        let project = dispatcher.domain().active_project().expect("工程");
        assert!(
            project
                .clip_pool
                .keys()
                .any(|id| id.to_canonical_string() == pool_clip),
            "材料必须真的进了池子"
        );
        assert!(
            project.tracks.values().all(|track| track
                .clips
                .values()
                .all(|placement| placement.clip_id.to_canonical_string() != pool_clip)),
            "`create: true` **不摆放** —— 这正是 needs-6 描述的状态"
        );
    }
    let rendered_pool_only = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48_000 }),
    );
    assert_eq!(
        rendered_pool_only["data"]["sha256"].as_str(),
        Some(sha_before.as_str()),
        "阴性对照: 池子里的片段没被摆放 ⇒ 母带字节必须逐字节不变: {rendered_pool_only}"
    );
    assert_eq!(
        rendered_pool_only["data"]["frames"].as_u64(),
        Some(frames_before),
        "阴性对照: 没摆放的片段不得改变母带长度"
    );
    let bytes_before_placement = project_bytes(&dispatcher);

    // ---- ② `ops: []` 单独给（没有 placement）仍然响亮失败: 空数组守卫没被放松 ----
    let empty_ops = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({ "trackId": track, "clipId": pool_clip, "ops": [] }),
    );
    assert_domain_error(
        &empty_ops,
        "INVALID_PARAMETER_RANGE",
        "空 ops 不是一次编辑请求",
    );
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before_placement,
        "失败不得改工程"
    );

    // ---- ③ 摆放: 空 `ops` + `placement` 合法, 且编译成**一条** AddClipPlacement ----
    let placed = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"startTick": 7680, "muted": false},
            "includeOps": true,
        }),
    );
    assert_eq!(placed["status"], "success", "{placed}");
    assert_eq!(
        placed["data"]["willCreate"]["opKinds"],
        json!(["AddClipPlacement"]),
        "摆放必须编译成一条 AddClipPlacement: {placed}"
    );
    let placement_payload = &placed["data"]["proposal"]["ops"][0]["op"]["AddClipPlacement"];
    assert_eq!(
        placement_payload["placement"]["clip_id"], pool_clip,
        "摆放必须指向池子里那条材料: {placed}"
    );
    assert_eq!(
        placement_payload["placement"]["start_tick"], 7680,
        "{placed}"
    );
    assert_eq!(
        placement_payload["placement"]["duration_ticks"], 480,
        "时值缺省 = 片段内容长度 (最后一个音符的结束 tick): {placed}"
    );
    assert_eq!(placement_payload["placement"]["muted"], false, "{placed}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before_placement,
        "提案不得改工程字节"
    );
    let placement_id = placed["data"]["willCreate"]["placements"][0]["id"]
        .as_str()
        .expect("placement id")
        .to_owned();
    merge(&mut dispatcher, &auth, &placed, "摆放池子材料");

    // ---- ④ 摆放逐字段可读 ----
    {
        let project = dispatcher.domain().active_project().expect("工程");
        let track_entry = project
            .tracks
            .values()
            .find(|entry| entry.id.to_canonical_string() == track)
            .expect("目标音轨");
        let placement = track_entry
            .clips
            .values()
            .find(|placement| placement.id.to_canonical_string() == placement_id)
            .expect("时间轴上必须有这条摆放");
        assert_eq!(placement.start_tick, 7680);
        assert_eq!(placement.duration_ticks, 480);
        assert!(!placement.muted);
        assert_eq!(placement.clip_id.to_canonical_string(), pool_clip);
    }

    // ---- ⑤ 母带**真的**听得见它: 字节变、帧数变长 ----
    let rendered_placed = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48_000 }),
    );
    assert_eq!(rendered_placed["status"], "success", "{rendered_placed}");
    assert_ne!(
        rendered_placed["data"]["sha256"].as_str(),
        Some(sha_before.as_str()),
        "摆放之后母带必须变 (阴性对照已经证明不摆放时它不变): {rendered_placed}"
    );
    let frames_placed = rendered_placed["data"]["frames"].as_u64().expect("frames");
    assert!(
        frames_placed > frames_before,
        "摆在 7680 tick 之后母带必须更长: {frames_placed} vs {frames_before}"
    );
    // 长度变长**还不够**：一条"只把时间轴拉长、但不排程新片音符"的注入也会让字节变、
    // 让帧数变长。因此再钉一条**排程读数**：逐源音符数恰好 +1（新片只有 1 个音符），
    // 且最长源的 `endTick` 恰好是摆放的末端 7680 + 480。
    assert_eq!(
        scheduled_notes(&rendered_placed),
        notes_before + 1,
        "新片的音符必须真的进排程: {rendered_placed}"
    );
    let max_end_tick = rendered_placed["data"]["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|source| source["endTick"].as_u64().unwrap_or(0))
        .max()
        .expect("至少一个源");
    assert_eq!(
        max_end_tick, 8160,
        "时间轴末端必须落在摆放末端 (7680 + 480): {rendered_placed}"
    );
    let wav = scratch.join("placement.master.wav");
    assert_eq!(
        file_sha256(&wav),
        rendered_placed["data"]["sha256"].as_str().expect("sha256"),
        "磁盘字节的摘要必须等于响应里的读数"
    );

    // ---- ⑥ 三条响亮失败 (全部要求工程字节不变) ----
    let bytes_after_placement = project_bytes(&dispatcher);
    // (a) 同一摆放身份重复提交 ⇒ CONFLICT (幂等重放请走 idempotencyKey)。
    let dup = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"startTick": 7680},
        }),
    );
    assert_domain_error(&dup, "CONFLICT", "同一条摆放不重复提交");
    assert_eq!(
        dup["error"]["data"]["reason"], "placementAlreadyExists",
        "{dup}"
    );
    // (b) `placement` 与 `create: true` 同给 ⇒ INVALID_PARAMETER_RANGE。
    //     `ops` 刻意给**一条合法 `add`** 且 `clipId` 是一个**池子里还没有**的身份：
    //     于是"没有这道守卫"的世界里这次调用会**成功**（并静默丢掉 `placement`），
    //     判据因此卡在 `status` 上，而不是卡在别的错误码上。
    let fresh = yeban_mcp::domain::ids::deterministic_id("clip:needs-6:create-and-place")
        .to_canonical_string();
    let both = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": fresh, "create": true,
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}],
            "placement": {"startTick": 0},
        }),
    );
    assert_domain_error(&both, "INVALID_PARAMETER_RANGE", "建与摆不能同给");
    assert_eq!(
        both["error"]["data"]["reason"], "placementIsNotCreation",
        "{both}"
    );
    // (c) `placement` 里的未知键 ⇒ INVALID_PARAMETER_RANGE + 支持集合 (不静默丢弃)。
    let typo = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"startTick": 0, "loopEnabled": true},
        }),
    );
    assert_domain_error(&typo, "INVALID_PARAMETER_RANGE", "未知摆放键不得静默丢弃");
    assert_eq!(
        typo["error"]["data"]["reason"], "unknownPlacementField",
        "{typo}"
    );
    assert_eq!(typo["error"]["data"]["field"], "loopEnabled", "{typo}");
    let supported = typo["error"]["data"]["supportedPlacementFields"]
        .as_array()
        .expect("supportedPlacementFields 必须是数组");
    for expected in ["startTick", "durationTicks", "placementId", "muted"] {
        assert!(
            supported.iter().any(|value| value == expected),
            "支持集合必须含 {expected}: {typo}"
        );
    }
    // (d) 零时值 ⇒ 响亮拒绝 (不是静默夹紧到 1)。
    let zero = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"startTick": 0, "durationTicks": 0},
        }),
    );
    assert_domain_error(&zero, "INVALID_PARAMETER_RANGE", "零时值的摆放必须被拒");
    assert_eq!(
        zero["error"]["data"]["field"], "placement.durationTicks",
        "{zero}"
    );
    // (e) 两个端点必须真的存在。
    let ghost = yeban_mcp::domain::ids::deterministic_id("ghost").to_canonical_string();
    let ghost_clip = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({ "trackId": track, "clipId": ghost, "ops": [], "placement": {"startTick": 0} }),
    );
    assert_domain_error(&ghost_clip, "CLIP_NOT_FOUND", "幽灵片段必须响亮失败");
    let ghost_track = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({ "trackId": ghost, "clipId": pool_clip, "ops": [], "placement": {"startTick": 0} }),
    );
    assert_domain_error(&ghost_track, "TRACK_NOT_FOUND", "幽灵音轨必须响亮失败");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_after_placement,
        "五条失败都不得改动工程字节"
    );

    // ---- ⑦ 可回退: 撤销一次 ⇒ 逐字节回到摆放之前的工程 ----
    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before_placement,
        "撤销必须逐字节复原 (摆放随 AddClipPlacement 一起可逆)"
    );
    let rendered_after_undo = call(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({ "format": "wav", "sampleRate": 48_000 }),
    );
    assert_eq!(
        rendered_after_undo["data"]["sha256"].as_str(),
        Some(sha_before.as_str()),
        "撤销之后母带必须回到摆放之前的字节: {rendered_after_undo}"
    );
}

/// **摆放编辑的关闭判据**：`placement.kind` 让一条**已经在时间轴上**的摆放可以被
/// **平移**与**取走**（`Op::MoveClipPlacement` / `Op::RemoveClipPlacement`）。
///
/// 在 `ff23302`（`add` 形态）之后，工具面能**加**一条摆放，却**没有**任何工具能挪它
/// 或取走它 —— 那两个 `Op` 在模型层早已实现（各自带自包含撤销载荷），渲染器也
/// **真的**按 `track.clips` 出片，因此"挪不动 / 取不走"是一条可听的能力缺口：
/// 摆错位置只剩"整次调用撤销"，而撤到那一步之前的编辑会一起丢。
///
/// 判据从工具调用一路走到**磁盘上的 WAV**，因此"取走真的让它不出声"是实测的：
///
/// 1. `add` 摆到 tick 7680 ⇒ 母带 `sha256` 变、帧数变长（阴性对照由 needs-6 那条
///    判据承担；这一条自己再证一次"变了"）；
/// 2. `move` 到 tick 15360 ⇒ 编译成**一条** `MoveClipPlacement`，其 `old_start_tick`
///    必须是**文档**里的 7680（不是调用方声明的），`new_start_tick` 是 15360；
///    合并后母带 `sha256` 再变、**帧数严格变长**；
/// 3. `remove` ⇒ 编译成**一条** `RemoveClipPlacement`，其 `previous_placement`
///    逐字段等于文档里那一条；合并后母带 `sha256` 与帧数**逐字节回到 `add` 之前**
///    （阴性对照的镜像：池子里的材料还在，但时间轴上没有了 ⇒ 一帧都不出声）；
/// 4. 两次编辑各自**逐字节可回退**（撤销一次 ⇒ 回到施加前）；
/// 5. 五条响亮失败（未知形态 / 幽灵摆放 / 片段不一致 / 本形态不适用的键 /
///    零位移），全部要求工程字节不变。
#[test]
fn placement_edits_move_and_remove_a_placement_and_the_master_follows() {
    let scratch = Scratch::new("placement-edit");
    let (mut dispatcher, auth) = dispatcher();
    let path = scratch.join("placement-edit.yeban");
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "PlacementEdit",
            "bpm": 120.0,
        }),
    );
    assert_eq!(created["status"], "success", "{created}");

    // 种子里**已经摆好** MIDI 片段的那条音轨（渲染器只看得到被摆放的东西）。
    let track = {
        let project = dispatcher.domain().active_project().expect("活跃工程");
        project
            .tracks
            .values()
            .find_map(|track| {
                track
                    .clips
                    .values()
                    .any(|placement| {
                        project
                            .clip_pool
                            .get(&placement.clip_id)
                            .is_some_and(|entry| entry.content.notes().is_some())
                    })
                    .then(|| track.id.to_canonical_string())
            })
            .expect("默认种子必须有一条被摆放的 MIDI 片段")
    };

    let render = |dispatcher: &mut Dispatcher| -> Value {
        let rendered = call(
            dispatcher,
            &auth,
            "yeban_render_master",
            json!({ "format": "wav", "sampleRate": 48_000 }),
        );
        assert_eq!(rendered["status"], "success", "{rendered}");
        rendered
    };

    // ---- ⓪ 基线：还没有这条材料时的母带（下面 `remove` 之后必须逐字节回到它）----
    let baseline = render(&mut dispatcher);
    let sha_baseline = baseline["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let frames_baseline = baseline["data"]["frames"].as_u64().expect("frames");

    // ---- ① 建材料（进池子）再 `add` 摆到 7680 ----
    let pool_clip = yeban_mcp::domain::ids::deterministic_id("clip:placement-edit:pool-only")
        .to_canonical_string();
    let material = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "clipName": "EditMe", "create": true,
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 100, "durationTicks": 480}}],
        }),
    );
    assert_eq!(material["status"], "success", "{material}");
    merge(&mut dispatcher, &auth, &material, "池子材料");

    let placed = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"startTick": 7680},
            "includeOps": true,
        }),
    );
    assert_eq!(placed["status"], "success", "{placed}");
    assert_eq!(
        placed["data"]["willCreate"]["opKinds"],
        json!(["AddClipPlacement"]),
        "{placed}"
    );
    let placement_id = placed["data"]["willCreate"]["placements"][0]["id"]
        .as_str()
        .expect("placement id")
        .to_owned();
    merge(&mut dispatcher, &auth, &placed, "摆放材料");

    let after_add = render(&mut dispatcher);
    let sha_after_add = after_add["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let frames_after_add = after_add["data"]["frames"].as_u64().expect("frames");
    assert_ne!(
        sha_after_add, sha_baseline,
        "摆了东西母带必须变: {after_add}"
    );
    assert!(
        frames_after_add > frames_baseline,
        "摆在 7680 tick 之后母带必须更长: {frames_after_add} vs {frames_baseline}"
    );
    let bytes_after_add = project_bytes(&dispatcher);

    // ---- ② `move`: 7680 → 15360, 旧起点必须取自**文档** ----
    let moved = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"kind": "move", "placementId": placement_id, "startTick": 15360},
            "includeOps": true,
        }),
    );
    assert_eq!(moved["status"], "success", "{moved}");
    assert_eq!(
        moved["data"]["willCreate"]["opKinds"],
        json!(["MoveClipPlacement"]),
        "`move` 必须编译成一条 MoveClipPlacement: {moved}"
    );
    let move_payload = &moved["data"]["proposal"]["ops"][0]["op"]["MoveClipPlacement"];
    assert_eq!(
        move_payload["old_start_tick"], 7680,
        "旧起点必须取自文档, 不是调用方声明的: {moved}"
    );
    assert_eq!(move_payload["new_start_tick"], 15360, "{moved}");
    assert_eq!(move_payload["placement_id"], placement_id, "{moved}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_after_add,
        "提案不得改工程字节"
    );
    merge(&mut dispatcher, &auth, &moved, "平移摆放");

    // ③ 逐字段可读：真的挪了, 且**没有**多出一条摆放。
    {
        let project = dispatcher.domain().active_project().expect("工程");
        let track_entry = project
            .tracks
            .values()
            .find(|entry| entry.id.to_canonical_string() == track)
            .expect("目标音轨");
        let mine: Vec<_> = track_entry
            .clips
            .values()
            .filter(|placement| placement.id.to_canonical_string() == placement_id)
            .collect();
        assert_eq!(mine.len(), 1, "平移不得新增第二条摆放: {mine:?}");
        assert_eq!(mine[0].start_tick, 15360);
        assert_eq!(mine[0].clip_id.to_canonical_string(), pool_clip);
        assert_eq!(
            track_entry
                .clips
                .values()
                .filter(|placement| placement.clip_id.to_canonical_string() == pool_clip)
                .count(),
            1,
            "同一条材料在这条音轨上只该有一条摆放"
        );
    }
    let after_move = render(&mut dispatcher);
    let sha_after_move = after_move["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let frames_after_move = after_move["data"]["frames"].as_u64().expect("frames");
    assert_ne!(
        sha_after_move, sha_after_add,
        "平移之后母带必须再变 (起点的变化真的进了排程): {after_move}"
    );
    assert!(
        frames_after_move > frames_after_add,
        "挪到 15360 tick 之后母带必须更长: {frames_after_move} vs {frames_after_add}"
    );
    let bytes_after_move = project_bytes(&dispatcher);

    // ---- ④ `remove`: 取走之后母带必须逐字节回到 `add` 之前 ----
    let removed = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"kind": "remove", "placementId": placement_id},
            "includeOps": true,
        }),
    );
    assert_eq!(removed["status"], "success", "{removed}");
    assert_eq!(
        removed["data"]["willCreate"]["opKinds"],
        json!(["RemoveClipPlacement"]),
        "`remove` 必须编译成一条 RemoveClipPlacement: {removed}"
    );
    let remove_payload = &removed["data"]["proposal"]["ops"][0]["op"]["RemoveClipPlacement"];
    assert_eq!(
        remove_payload["previous_placement"]["start_tick"], 15360,
        "撤销载荷必须是文档里那一条摆放 (平移之后的起点): {removed}"
    );
    assert_eq!(
        remove_payload["previous_placement"]["clip_id"], pool_clip,
        "{removed}"
    );
    merge(&mut dispatcher, &auth, &removed, "取走摆放");
    {
        let project = dispatcher.domain().active_project().expect("工程");
        assert!(
            project.tracks.values().all(|track| track
                .clips
                .values()
                .all(|placement| placement.clip_id.to_canonical_string() != pool_clip)),
            "取走之后这条材料不得再出现在任何音轨的时间轴上"
        );
        assert!(
            project
                .clip_pool
                .keys()
                .any(|id| id.to_canonical_string() == pool_clip),
            "`remove` **只**取走摆放, 池子里的材料不动"
        );
    }
    let after_remove = render(&mut dispatcher);
    assert_eq!(
        after_remove["data"]["sha256"].as_str(),
        Some(sha_baseline.as_str()),
        "取走摆放之后母带必须逐字节回到 `add` 之前: {after_remove}"
    );
    assert_eq!(
        after_remove["data"]["frames"].as_u64(),
        Some(frames_baseline),
        "取走摆放之后母带长度必须回到 `add` 之前"
    );

    // ---- ⑤ 两条编辑各自逐字节可回退 ----
    let undo_remove = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undo_remove["status"], "success", "{undo_remove}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_after_move,
        "撤销 `remove` 必须逐字节回到平移之后"
    );
    let undo_move = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undo_move["status"], "success", "{undo_move}");
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_after_add,
        "撤销 `move` 必须逐字节回到 `add` 之后"
    );
    // 再回到"什么都还没摆"的状态：撤销 `add`。
    let undo_add = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undo_add["status"], "success", "{undo_add}");
    let back_to_onset = render(&mut dispatcher);
    assert_eq!(
        back_to_onset["data"]["sha256"].as_str(),
        Some(sha_baseline.as_str()),
        "三次撤销之后母带必须回到基线字节: {back_to_onset}"
    );

    // ---- ⑥ 五条响亮失败 (全部要求工程字节不变) ----
    // 先重新摆一条，好让 (c)/(d)/(e) 有一个真实存在的摆放可以指名。
    let re_placed = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"startTick": 0},
            "includeOps": true,
        }),
    );
    assert_eq!(re_placed["status"], "success", "{re_placed}");
    let re_placement_id = re_placed["data"]["willCreate"]["placements"][0]["id"]
        .as_str()
        .expect("placement id")
        .to_owned();
    merge(&mut dispatcher, &auth, &re_placed, "重新摆放");
    let bytes_before_failures = project_bytes(&dispatcher);
    // (a) 未知形态 ⇒ INVALID_PARAMETER_RANGE + 支持集合 (不静默按 add 处理)。
    let bad_kind = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"kind": "shift", "placementId": re_placement_id, "startTick": 0},
        }),
    );
    assert_domain_error(
        &bad_kind,
        "INVALID_PARAMETER_RANGE",
        "未知摆放形态不得静默按 add 处理",
    );
    assert_eq!(
        bad_kind["error"]["data"]["reason"], "unknownPlacementKind",
        "{bad_kind}"
    );
    // (b) 已经 `remove` 过的那条摆放 ⇒ ENTITY_NOT_FOUND (不是静默新建一条)。
    let ghost = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"kind": "remove", "placementId": placement_id},
        }),
    );
    assert_domain_error(&ghost, "ENTITY_NOT_FOUND", "幽灵摆放必须响亮失败");
    assert_eq!(
        ghost["error"]["data"]["reason"], "placementNotFound",
        "{ghost}"
    );
    // (c) `clipId` 与文档里那条摆放引用的片段不一致 ⇒ 不静默改用文档那一条。
    let other_clip = {
        let project = dispatcher.domain().active_project().expect("工程");
        project
            .clip_pool
            .keys()
            .map(EntityId::to_canonical_string)
            .find(|id| *id != pool_clip)
            .expect("样本里必须还有第二条片段池条目")
    };
    let mismatch = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": other_clip, "ops": [],
            "placement": {"kind": "remove", "placementId": re_placement_id},
        }),
    );
    assert_domain_error(
        &mismatch,
        "INVALID_PARAMETER_RANGE",
        "片段不一致必须响亮失败",
    );
    assert_eq!(
        mismatch["error"]["data"]["reason"], "placementClipMismatch",
        "{mismatch}"
    );
    // (d) 本形态不适用但别处合法的键 ⇒ `placementFieldNotApplicable` (不静默丢弃)。
    let inapplicable = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"kind": "remove", "placementId": re_placement_id, "durationTicks": 480},
        }),
    );
    assert_domain_error(
        &inapplicable,
        "INVALID_PARAMETER_RANGE",
        "本形态不适用的键必须被拒",
    );
    assert_eq!(
        inapplicable["error"]["data"]["reason"], "placementFieldNotApplicable",
        "{inapplicable}"
    );
    // (e) 零位移 ⇒ CONFLICT (没有可提交的改动, 不制造一条空提案)。
    let zero_move = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track, "clipId": pool_clip, "ops": [],
            "placement": {"kind": "move", "placementId": re_placement_id, "startTick": 0},
        }),
    );
    assert_domain_error(&zero_move, "CONFLICT", "零位移必须响亮失败");
    assert_eq!(
        zero_move["error"]["data"]["reason"], "placementAlreadyAtStartTick",
        "{zero_move}"
    );
    assert_eq!(
        project_bytes(&dispatcher),
        bytes_before_failures,
        "五条失败都不得改动工程字节"
    );
}
