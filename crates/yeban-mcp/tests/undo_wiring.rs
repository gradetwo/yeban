//! **撤销接线判据（MCP 侧）** —— ADR-0001 **D45** 的"AI 工具能撤销"这一半 [ARCH-OPS-002, MODEL-ISO-001]。
//!
//! 判据清单（编号与台账 `docs/ledger/undo-wiring-notes.md` 一致）：
//!
//! | # | 判据 | 落点 |
//! | :-- | :--- | :--- |
//! | ① | 单步撤销**逐字节**回到上一版（真工具：`yeban_edit_notes` → `yeban_merge_proposal` → `yeban_undo`） | [`one_undo_returns_to_the_previous_bytes`] |
//! | ② | 连续 N 步逐步回退 | [`consecutive_undos_step_back`] |
//! | ③ | 重做逐字节回到撤销前 | [`redo_returns_to_the_bytes_before_the_undo`] |
//! | ④ | `Op::Batch`（`yeban_propose_section` 那批）算**一步** | [`a_proposal_batch_counts_as_one_step`] |
//! | ⑤ | 无历史可撤 ⇒ 明确错误码 + 工程不变（含 `steps: 0`） | [`undo_without_history_is_an_explicit_domain_error`] |
//! | ⑥ | `dryRun=true` ⇒ 状态一位不变 + 预览与真做一致 | [`dry_run_does_not_touch_state_and_matches_the_real_undo`] |
//! | ⑦ | 游标**不落盘**（容器字节级） | [`the_cursor_never_reaches_the_project_container`] |
//! | ⑧ | UI 路径与 MCP 路径结果**逐字节相同** | [`the_ui_path_and_the_mcp_path_agree_byte_for_byte`] |
//! | ⑪ | 撤销后保存、再打开 ⇒ 与撤销后的内存态一致（容器往返 + 历史跨打开存活） | [`saved_after_an_undo_reopens_to_the_same_bytes`] |
//! | ⑫ | 撤销不越过"工程打开"边界 | [`undo_does_not_cross_a_project_open_boundary`] |
//! | ⑬ | 幂等：同 `idempotencyKey` 不得撤两次 | [`the_same_idempotency_key_never_undoes_twice`] |
//! | ⑩ | 撤销不破坏既有不变量（`validate()`） | [`the_project_still_validates_after_undo`] |
//!
//! **"共用同一实现"**在这里是文件级事实：本文件与 `yeban-app` 用的是**同一个**
//! `crate::undo_session`（`#[path]` 引入同一份源码），判据
//! `undo_session::tests::no_second_undo_implementation_exists_in_the_workspace`
//! 扫描生产源码证明工作区里不存在第二份反向应用实现。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use yeban_mcp::Dispatcher;
use yeban_mcp::jsonrpc::ErrorObject;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::undo_session::{self, CommitRequest, UndoRefusal, UndoSession, wiring_fixture};
use yeban_model::container::{self, ContainerLimits};
use yeban_model::samples::filled_project;
use yeban_model::{OpOrigin, YebanProjectV1};

const NOW: u64 = 1_760_000_000_000;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 一个确定性的临时工程路径（**不建父目录**：这些判据都不落盘）。
fn temp_project_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-undo-wiring-{}-{tag}-{}",
            std::process::id(),
            yeban_model::EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

/// 一个只含**确定性命名字段**的临时目录（供真落盘 / 真读盘的判据用）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-undo-wiring-disk-{}-{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn new_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(NOW);
    (dispatcher, auth)
}

/// 注入一份规范样本工程（不碰文件系统）。
fn seed(dispatcher: &mut Dispatcher, tag: &str) {
    dispatcher
        .domain_mut()
        .open_in_memory(temp_project_path(tag), filled_project(), false)
        .expect("注入规范工程");
}

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

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let (status, outcome) = call_raw(dispatcher, auth, name, arguments);
    assert_eq!(
        status, 200,
        "{name} 必须是带内结果（领域失败也走 ToolResponse）"
    );
    outcome.unwrap_or_else(|error| panic!("{name} 不该有 JSON-RPC 层错误: {error:?}"))
}

/// 带内领域失败的错误码。
fn domain_error(value: &Value, context: &str) -> String {
    assert_eq!(value["status"], "error", "{context}: {value}");
    value["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("{context} 缺错误码: {value}"))
        .to_owned()
}

/// 权威工程的**逐字节**文本（判据的标尺）。
fn project_bytes(dispatcher: &Dispatcher) -> String {
    let project = dispatcher.domain().active_project().expect("活跃工程");
    let mut text = serde_json::to_string_pretty(project).expect("序列化");
    text.push('\n');
    text
}

/// 工程的一份**跨侧可比**指纹（模型容器字节的 SHA-256；两侧共用同一函数）。
fn fingerprint(project: &YebanProjectV1) -> String {
    undo_session::project_fingerprint(project).expect("指纹")
}

/// 真工具改一个音符力度（走 `yeban_edit_notes` 的 `velocity` 编译路径）。
fn edit_velocity(dispatcher: &mut Dispatcher, auth: &str, target: u8) -> String {
    let project = dispatcher
        .domain()
        .active_project()
        .expect("活跃工程")
        .clone();
    let fixture = wiring_fixture(&project).expect("夹具");
    let created = call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({
            "trackId": fixture.track_id.to_canonical_string(),
            "clipId": fixture.clip_id.to_canonical_string(),
            "ops": [{
                "kind": "velocity",
                "noteId": fixture.note_id.to_canonical_string(),
                "velocity": target,
            }],
        }),
    );
    assert_eq!(created["status"], "success", "{created}");
    created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned()
}

/// 合并一个提案（走真工具；`Op::Batch` 因此落在提交图谱上）。
fn merge(dispatcher: &mut Dispatcher, auth: &str, proposal_id: &str) -> Value {
    call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({ "proposalId": proposal_id, "commitMessage": "接线判据" }),
    )
}

// ---------------------------------------------------------------------------
// ① ② ③ ④ ⑤ ⑥ ⑩ ⑪ ⑫ ⑬
// ---------------------------------------------------------------------------

#[test]
fn one_undo_returns_to_the_previous_bytes() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "one-undo");
    let pristine = project_bytes(&dispatcher);
    let baseline_fingerprint = fingerprint(dispatcher.domain().active_project().expect("工程"));

    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);
    let edited = project_bytes(&dispatcher);
    assert_ne!(pristine, edited, "合并必须真的改了工程");

    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["status"], "success", "{undone}");
    assert_eq!(undone["data"]["steps"], 1);
    assert_eq!(undone["data"]["undoneTotal"], 1);
    assert_eq!(undone["data"]["opKinds"][0], "Batch", "一次提交 = 一条 op");
    assert_eq!(undone["data"]["cursorPersisted"], false);
    assert_eq!(project_bytes(&dispatcher), pristine, "必须逐字节回到上一版");
    assert_eq!(
        fingerprint(dispatcher.domain().active_project().expect("工程")),
        baseline_fingerprint
    );
    // 显示态来自模型读数。
    let after = &undone["data"]["after"];
    assert_eq!(after["canUndo"], false);
    assert_eq!(after["canRedo"], true);
    assert_eq!(after["undone"], 1);
    assert_eq!(after["undoable"], 0);
}

#[test]
fn consecutive_undos_step_back() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "consecutive");
    let mut states = vec![project_bytes(&dispatcher)];
    for target in [30_u8, 31, 32] {
        let proposal = edit_velocity(&mut dispatcher, &auth, target);
        merge(&mut dispatcher, &auth, &proposal);
        states.push(project_bytes(&dispatcher));
    }
    assert_eq!(
        dispatcher.domain().undo_state().undone().to_string(),
        "0",
        "还没撤"
    );
    assert_eq!(dispatcher.domain().undo_display().undoable, 3);
    for step in (0..3).rev() {
        let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
        assert_eq!(undone["data"]["steps"], 1);
        assert_eq!(project_bytes(&dispatcher), states[step]);
    }
    let refused = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(domain_error(&refused, "撤到底"), "INDEX_OUT_OF_BOUNDS");
}

#[test]
fn redo_returns_to_the_bytes_before_the_undo() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "redo");
    let proposal = edit_velocity(&mut dispatcher, &auth, 55);
    merge(&mut dispatcher, &auth, &proposal);
    let edited = project_bytes(&dispatcher);

    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    let redone = call(&mut dispatcher, &auth, "yeban_redo", json!({}));
    assert_eq!(redone["status"], "success", "{redone}");
    assert_eq!(redone["data"]["steps"], 1);
    assert_eq!(redone["data"]["undoneTotal"], 0);
    assert_eq!(
        project_bytes(&dispatcher),
        edited,
        "重做必须逐字节回到撤销前"
    );
    let refused = call(&mut dispatcher, &auth, "yeban_redo", json!({}));
    assert_eq!(domain_error(&refused, "没有可重做"), "INDEX_OUT_OF_BOUNDS");
}

/// 判据 ④：`yeban_propose_section` 产出的那批（章节 + 摆放 + 声部连接 + 片段）算**一步**。
#[test]
fn a_proposal_batch_counts_as_one_step() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "batch");
    let pristine = project_bytes(&dispatcher);

    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 8 }),
    );
    assert_eq!(created["status"], "success", "{created}");
    let batch_ops = created["data"]["willCreate"]
        .as_object()
        .map_or(0, serde_json::Map::len);
    assert!(batch_ops > 0, "章节骨架必须真的生成东西: {created}");
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = merge(&mut dispatcher, &auth, &proposal_id);
    // 提案的 `Op::Batch` 是一条 `StampedOp` ⇒ 可撤销步数 = 1。
    assert_eq!(
        dispatcher.domain().undo_display().undoable,
        1,
        "整批算一步: {merged}"
    );
    assert!(merged["data"]["appliedOps"].as_u64().expect("appliedOps") >= 1);

    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["data"]["steps"], 1);
    assert_eq!(undone["data"]["opKinds"][0], "Batch");
    assert_eq!(project_bytes(&dispatcher), pristine, "整批一步回退");
}

#[test]
fn undo_without_history_is_an_explicit_domain_error() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "no-history");
    let before = project_bytes(&dispatcher);

    let refused = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(domain_error(&refused, "无历史"), "INDEX_OUT_OF_BOUNDS");
    assert_eq!(
        refused["error"]["data"]["reason"], "no-history",
        "错误载荷要说清是哪一类越界"
    );
    assert_eq!(project_bytes(&dispatcher), before, "拒绝时工程一位不变");
    assert_eq!(dispatcher.domain().undo_state().undone(), 0);

    // 没有活跃工程 ⇒ NO_ACTIVE_PROJECT（另一条领域失败，不是实现级出口）。
    let (mut empty, auth_empty) = new_dispatcher();
    let no_project = call(&mut empty, &auth_empty, "yeban_undo", json!({}));
    assert_eq!(domain_error(&no_project, "无工程"), "NO_ACTIVE_PROJECT");

    // `steps: 0` 是**显式**非法取值（不替调用方猜"等价于 1"）。
    let zero = call(&mut dispatcher, &auth, "yeban_undo", json!({ "steps": 0 }));
    assert_eq!(domain_error(&zero, "steps=0"), "INVALID_PARAMETER_RANGE");
    assert_eq!(project_bytes(&dispatcher), before);
}

#[test]
fn dry_run_does_not_touch_state_and_matches_the_real_undo() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "dry-run");
    let proposal = edit_velocity(&mut dispatcher, &auth, 61);
    merge(&mut dispatcher, &auth, &proposal);
    let bytes_before = project_bytes(&dispatcher);
    let commits_before = dispatcher.domain().commit_count();
    let undone_before = dispatcher.domain().undo_state().undone();

    let preview = call(
        &mut dispatcher,
        &auth,
        "yeban_undo",
        json!({ "dryRun": true, "idempotencyKey": "k-dry" }),
    );
    assert_eq!(preview["status"], "success", "{preview}");
    assert_eq!(preview["data"]["dryRun"], true);
    assert_eq!(preview["data"]["stateUnchanged"], true);
    assert_eq!(preview["data"]["tool"], "yeban_undo");
    assert_eq!(preview["data"]["specId"], "MCP-TOOL-EXT-UNDO");
    assert_eq!(preview["data"]["sideEffect"], "project-state");
    // 状态一位不变。
    assert_eq!(project_bytes(&dispatcher), bytes_before);
    assert_eq!(dispatcher.domain().commit_count(), commits_before);
    assert_eq!(dispatcher.domain().undo_state().undone(), undone_before);
    assert_eq!(dispatcher.idempotency_len(), 0, "dryRun 不写幂等缓存");
    // 预览是**实测**：摘要与真做一致。
    let preview_digest = preview["data"]["preview"]["projectDigestAfter"]
        .as_str()
        .expect("projectDigestAfter")
        .to_owned();
    assert_eq!(preview["data"]["preview"]["willUndoSteps"], 1);
    assert_eq!(preview["data"]["preview"]["undoableSteps"], 1);
    assert_eq!(preview["data"]["preview"]["opKinds"][0], "Batch");

    let real = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(real["data"]["steps"], 1);
    assert_eq!(
        real["data"]["projectDigest"], preview_digest,
        "预览里的摘要必须与真做的读数逐字节相同"
    );
}

/// 判据 ⑦：游标是**会话运行态**，一个字节都不进工程容器 [MODEL-ISO-001]。
#[test]
fn the_cursor_never_reaches_the_project_container() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "cursor");
    let proposal = edit_velocity(&mut dispatcher, &auth, 47);
    merge(&mut dispatcher, &auth, &proposal);

    let project_before = dispatcher.domain().active_project().expect("工程").clone();
    let keys_before: Vec<String> = serde_json::to_value(&project_before)
        .expect("工程 JSON")
        .as_object()
        .expect("对象")
        .keys()
        .cloned()
        .collect();
    let dag_before = serde_json::to_vec(dispatcher.domain().graph()).expect("图谱字节");

    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(dispatcher.domain().undo_state().undone(), 1);

    // (1) 撤销**不动**提交图谱：`history.dag` 的字节完全相同。
    let dag_after = serde_json::to_vec(dispatcher.domain().graph()).expect("图谱字节");
    assert_eq!(dag_after, dag_before, "撤销不得改动 history.dag 一个字节");

    // (2) 工程文档的**键集合**与撤销前逐一相同（没有多出游标字段）。
    let project_after = dispatcher.domain().active_project().expect("工程");
    let value_after = serde_json::to_value(project_after).expect("工程 JSON");
    let keys_after: Vec<String> = value_after
        .as_object()
        .expect("对象")
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys_after, keys_before, "工程文档的键集合不得改变");
    let text = serde_json::to_string(&value_after).expect("文本");
    for needle in ["cursor", "Cursor", "undone", "redo"] {
        assert!(
            !text.contains(needle),
            "工程 JSON 里不该出现 `{needle}`: {text}"
        );
    }
    // (3) 容器字节级：`project.json` 变了（被撤销的那次变更），`history.dag` 与资产没变。
    let bytes_before =
        container::write_project_container(&project_before, &dag_before, &BTreeMap::new())
            .expect("容器字节");
    let bytes_after =
        container::write_project_container(project_after, &dag_after, &BTreeMap::new())
            .expect("容器字节");
    assert_ne!(
        bytes_before, bytes_after,
        "被撤销的那次变更必须反映在容器字节里"
    );
    let read_after =
        container::read_project_container(&bytes_after, &ContainerLimits::default()).expect("读回");
    assert_eq!(read_after.history_dag, dag_before, "history.dag 逐字节不变");
    assert!(read_after.assets.is_empty());
}

/// 判据 ⑧：**同一条操作序列**，UI 路径与 MCP 路径撤销后的工程**逐字节相同**。
#[test]
fn the_ui_path_and_the_mcp_path_agree_byte_for_byte() {
    // ---- MCP 路径：真工具 ----
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "two-paths");
    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);
    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    let mcp_project = dispatcher.domain().active_project().expect("工程").clone();

    // ---- UI 路径：`yeban-app` 用的**同一个** `UndoSession` ----
    let fixture = wiring_fixture(&filled_project()).expect("夹具");
    let mut ui = UndoSession::open("<判据>", "yeban-app", filled_project(), NOW).expect("打开");
    ui.commit(CommitRequest {
        now_ms: NOW + 1,
        origin: OpOrigin::UserUi,
        message: "接线判据".to_owned(),
        ops: vec![fixture.op()],
    })
    .expect("提交");
    ui.undo_steps(1).expect("撤销");
    let ui_project = ui.project().clone();

    // 两侧都必须等于**原始工程**（逐字节），因此彼此也逐字节相同。
    let pristine = filled_project();
    assert_eq!(
        fingerprint(&mcp_project),
        fingerprint(&pristine),
        "MCP 路径必须逐字节回到上一版"
    );
    assert_eq!(
        fingerprint(&ui_project),
        fingerprint(&pristine),
        "UI 路径必须逐字节回到上一版"
    );
    assert_eq!(
        fingerprint(&ui_project),
        fingerprint(&mcp_project),
        "UI 路径与 MCP 路径的结果必须逐字节相同"
    );
    assert_eq!(
        serde_json::to_string(&ui_project).expect("序列化"),
        serde_json::to_string(&mcp_project).expect("序列化")
    );
    // 会话态是**两侧各自的**运行态，因此游标不在工程里。
    assert_eq!(ui.state().undone(), 1);
    assert_eq!(dispatcher.domain().undo_state().undone(), 1);
}

/// 判据 ⑪：撤销 → **真落盘** → **真打开** ⇒ 与撤销后的内存态一致；历史跨打开存活。
#[test]
fn saved_after_an_undo_reopens_to_the_same_bytes() {
    let dir = scratch("round-trip");
    let path = dir.join("demo.yeban");
    let (mut dispatcher, auth) = new_dispatcher();
    // 先写一份真容器（唯一工程格式 ADR-0001 D43）。
    let pristine = filled_project();
    std::fs::write(
        &path,
        container::write_project_container(&pristine, &[], &BTreeMap::new()).expect("容器"),
    )
    .expect("写盘");
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    let pristine_bytes = project_bytes(&dispatcher);

    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);
    let edited_bytes = project_bytes(&dispatcher);
    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(
        project_bytes(&dispatcher),
        pristine_bytes,
        "撤销后逐字节回到原样"
    );
    let saved = call(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({ "force": true }),
    );
    assert_eq!(saved["status"], "success", "{saved}");
    // 释放 `.yeban.lock`（RAII）之后再重开：同一个路径只能有一个写者 [ARCH-SEC-001]。
    drop(dispatcher);

    // 再打开（**新分发器**，因此会话态必然是全新的）。
    let (mut reopened, auth2) = new_dispatcher();
    let again = call(
        &mut reopened,
        &auth2,
        "yeban_open_project",
        json!({ "path": path.display().to_string() }),
    );
    assert_eq!(again["status"], "success", "{again}");
    assert_eq!(
        again["data"]["historyRestored"], true,
        "撤销后的提交图谱必须跨打开存活: {again}"
    );
    assert_eq!(
        project_bytes(&reopened),
        pristine_bytes,
        "再打开之后的工程 = 撤销后的内存态"
    );
    // 游标**没有落盘**，但它可以从（文档, 图谱）**推导**出来：
    // 保存下来的文档停在被撤销后的状态 ⇒ 推导出"已撤销 1 步" ⇒ 重做栈仍然是对的。
    let display = reopened.domain().undo_display();
    assert_eq!(display.undone, 1, "游标是推导出来的，不是持久化的");
    assert_eq!(display.undoable, 0, "文档已经是被撤销后的状态");
    assert!(display.can_redo);
    let redone = call(&mut reopened, &auth2, "yeban_redo", json!({}));
    assert_eq!(redone["status"], "success", "{redone}");
    assert_eq!(redone["data"]["steps"], 1);
    assert_eq!(
        project_bytes(&reopened),
        edited_bytes,
        "重开之后重做 ⇒ 回到保存前编辑过的状态"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ⑫：撤销不越过"工程打开"边界。
///
/// 强形态（也是这条判据真正的靶子）：**打开一个带历史的工程**。
/// 如果打开时不重置会话态，上一个工程的游标（"已撤销 1 步"）会跟到新工程上 ——
/// 于是新工程凭空多出一步可重做，重做还会把**另一个工程的 op** 打到这份文档上。
#[test]
fn undo_does_not_cross_a_project_open_boundary() {
    let dir = scratch("boundary");
    let path_b = dir.join("b.yeban");

    // 先造一个**带历史**的工程 B（用真工具改一次再落盘）。
    {
        let (mut writer, writer_auth) = new_dispatcher();
        let pristine = filled_project();
        std::fs::write(
            &path_b,
            container::write_project_container(&pristine, &[], &BTreeMap::new()).expect("容器"),
        )
        .expect("写盘");
        let opened = call(
            &mut writer,
            &writer_auth,
            "yeban_open_project",
            json!({ "path": path_b.display().to_string() }),
        );
        assert_eq!(opened["status"], "success", "{opened}");
        let proposal = edit_velocity(&mut writer, &writer_auth, 42);
        merge(&mut writer, &writer_auth, &proposal);
        let saved = call(
            &mut writer,
            &writer_auth,
            "yeban_save_project",
            json!({ "force": true }),
        );
        assert_eq!(saved["status"], "success", "{saved}");
    } // Drop ⇒ 释放 `.yeban.lock`

    // B 的一份**副本**，给第二段用。必须在这里拷：Windows 的 `LockFileEx` 是**强制**
    // 字节区间锁，B 一旦被下面的 `open_project` 打开，另一个句柄连读都读不到
    // （见 `domain/mod.rs` 的 `lock_holder` 文档与 `docs/ledger/lock-advisory-notes.md`）。
    let path_c = dir.join("c.yeban");
    std::fs::copy(&path_b, &path_c).expect("拷贝容器");

    // 域 A：在自己的工程上撤销一步（游标真的前移了）。
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "first-project");
    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);
    assert_eq!(dispatcher.domain().undo_display().undoable, 1);
    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(dispatcher.domain().undo_display().undone, 1);

    // 关掉 A（同一个 `Domain` 里只能有一个活跃工程）—— 关闭**不**重置会话态，
    // 因此接下来的"打开 B"就是重置会话态的唯一防线。
    let closed = call(
        &mut dispatcher,
        &auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    assert_eq!(closed["data"]["closed"], true, "{closed}");
    assert_eq!(
        dispatcher.domain().undo_display().undone,
        1,
        "关闭工程本身不动会话态（防线在打开那一侧）"
    );

    // 打开 B（同一个 `Domain`，**带历史**）⇒ 会话态必须整体重置。
    let opened = call(
        &mut dispatcher,
        &auth,
        "yeban_open_project",
        json!({ "path": path_b.display().to_string() }),
    );
    assert_eq!(opened["status"], "success", "{opened}");
    let display = dispatcher.domain().undo_display();
    assert_eq!(display.undone, 0, "打开新工程之后不许带着上一个工程的游标");
    assert!(!display.can_redo, "不许重做上一个工程的动作");
    assert_eq!(display.undoable, 1, "B 自己那条提交才是可撤销的");
    let refused = call(&mut dispatcher, &auth, "yeban_redo", json!({}));
    assert_eq!(
        domain_error(&refused, "跨打开边界的重做"),
        "INDEX_OUT_OF_BOUNDS"
    );
    // 而 B 自己那一步仍然撤得动（证明重置不是"把功能关掉"）。
    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["data"]["steps"], 1, "{undone}");

    // 强形态 2：在 A 上"撤销后继续编辑"会派生**匿名分支**；打开 B 时活跃分支必须回到
    // `main` —— 否则 B 的图谱里根本没有那条匿名分支，打开会变成一条硬错误
    // （会话态泄漏成"打不开工程"，比"多撤一步"更严重）。
    let (mut forked, forked_auth) = new_dispatcher();
    seed(&mut forked, "anon-project");
    let first = edit_velocity(&mut forked, &forked_auth, 42);
    merge(&mut forked, &forked_auth, &first);
    call(&mut forked, &forked_auth, "yeban_undo", json!({}));
    let second = edit_velocity(&mut forked, &forked_auth, 77);
    merge(&mut forked, &forked_auth, &second);
    assert!(
        forked
            .domain()
            .active_branch()
            .starts_with(yeban_model::ANONYMOUS_BRANCH_PREFIX)
    );
    call(
        &mut forked,
        &forked_auth,
        "yeban_close_project",
        json!({ "saveFirst": false }),
    );
    let reopened_b = call(
        &mut forked,
        &forked_auth,
        "yeban_open_project",
        json!({ "path": path_c.display().to_string() }),
    );
    assert_eq!(
        reopened_b["status"], "success",
        "打开新工程必须把活跃分支拉回 main: {reopened_b}"
    );
    assert_eq!(
        forked.domain().active_branch(),
        yeban_mcp::domain::MAIN_BRANCH
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_idempotency_key_never_undoes_twice() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "idempotent");
    let pristine = project_bytes(&dispatcher);
    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);

    let first = call(
        &mut dispatcher,
        &auth,
        "yeban_undo",
        json!({ "idempotencyKey": "k1" }),
    );
    assert_eq!(first["data"]["steps"], 1);
    let after_first = project_bytes(&dispatcher);
    assert_eq!(after_first, pristine);

    let second = call(
        &mut dispatcher,
        &auth,
        "yeban_undo",
        json!({ "idempotencyKey": "k1" }),
    );
    assert_eq!(second["replayed"], true, "同键必须命中幂等缓存: {second}");
    assert_eq!(
        dispatcher.domain().undo_state().undone(),
        1,
        "同键重放**不得**再撤一步"
    );
    assert_eq!(project_bytes(&dispatcher), after_first);
}

/// 判据 ⑩：撤销**不破坏**既有不变量（路由 / 自动化 / 片段池的 `validate()`）。
#[test]
fn the_project_still_validates_after_undo() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "invariants");
    let created = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({ "sectionName": "Chorus", "stylePreset": "lo_fi_hip_hop", "bars": 4 }),
    );
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    merge(&mut dispatcher, &auth, &proposal_id);
    dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .validate()
        .expect("合并后合法");
    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .validate()
        .expect("撤销后必须仍然合法（路由/自动化/片段池）");
    call(&mut dispatcher, &auth, "yeban_redo", json!({}));
    dispatcher
        .domain()
        .active_project()
        .expect("工程")
        .validate()
        .expect("重做后必须仍然合法");
}

/// 判据：撤销后再编辑**派生匿名分支**（模型的 `fork_anonymous`），
/// 且被撤销的那条 op 不可能被再撤一次（`ARCH-OPS-002` 的"撤销后继续编辑"）。
#[test]
fn editing_after_an_undo_forks_an_anonymous_branch() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "fork");
    let pristine = project_bytes(&dispatcher);
    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);
    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(project_bytes(&dispatcher), pristine);

    // 撤销之后继续编辑：合并另一条提案。
    let second = edit_velocity(&mut dispatcher, &auth, 77);
    let merged = merge(&mut dispatcher, &auth, &second);
    assert!(
        merged["data"]["commit"]["branch"]
            .as_str()
            .expect("branch")
            .starts_with(yeban_model::ANONYMOUS_BRANCH_PREFIX),
        "撤销后继续编辑必须落在匿名分支上: {merged}"
    );
    let display = dispatcher.domain().undo_display();
    assert_eq!(display.undone, 0, "新头之上没有已撤销的步骤");
    assert_eq!(display.undoable, 1, "只有新提交那一步可撤");
    assert_eq!(
        display.branch_count,
        dispatcher.domain().graph().branches.len()
    );
    // 原分支头作为**只读孤岛**永久保全：`main` 仍然指着第一次合并的那条提交。
    let island = dispatcher
        .domain()
        .graph()
        .branches
        .get(yeban_mcp::domain::MAIN_BRANCH)
        .expect("孤岛分支");
    assert!(
        island.head != dispatcher.domain().active_head().expect("活跃头"),
        "派生匿名分支之后活跃头不再是孤岛头"
    );

    // 再撤一步 ⇒ 回到 pristine（被丢弃的那条 op 不可再撤）。
    call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(project_bytes(&dispatcher), pristine);
    let refused = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(domain_error(&refused, "被丢弃的 op"), "INDEX_OUT_OF_BOUNDS");
}

/// 判据：契约形状。工具响应必须是 `ToolResponse`，错误码在契约 enum 内；
/// 未鉴权的调用仍然拿不到工具存在性信息（既有安全不变式不受影响）。
#[test]
fn the_two_new_tools_respect_the_existing_pipeline() {
    let (mut dispatcher, auth) = new_dispatcher();
    let listed = {
        let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string();
        let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
        outcome.response.expect("响应").result.expect("result")
    };
    let names: Vec<String> = listed["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("name").to_owned())
        .collect();
    assert_eq!(names.len(), yeban_mcp::tools::TOOL_COUNT);
    assert!(names.contains(&"yeban_undo".to_owned()));
    assert!(names.contains(&"yeban_redo".to_owned()));
    let undo_descriptor = listed["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|tool| tool["name"] == "yeban_undo")
        .expect("yeban_undo 描述符");
    assert_eq!(
        undo_descriptor["annotations"]["specId"],
        "MCP-TOOL-EXT-UNDO"
    );
    assert_eq!(
        undo_descriptor["annotations"]["sideEffect"],
        "project-state"
    );
    assert_eq!(undo_descriptor["annotations"]["dryRunSupported"], true);
    assert_eq!(
        undo_descriptor["inputSchema"]["properties"]["steps"]["type"],
        "integer"
    );
    assert!(
        undo_descriptor["inputSchema"].get("required").is_none(),
        "steps 是可选参数（缺省 1）"
    );

    // 未鉴权 + 未知工具 ⇒ 401（鉴权在工具解析之前）。
    let outcome = dispatcher.handle_line(
        Channel::Http,
        None,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"yeban_undo","arguments":{}}})
        .to_string(),
    );
    assert_eq!(outcome.http_status, 401);
    assert_eq!(outcome.error_code(), Some(yeban_mcp::jsonrpc::UNAUTHORIZED));
}

/// 判据：模型的入口确实**没有被**绕开（`undo_with` 是唯一执行者）。
///
/// 扫描口径（生产区截断、注释豁免）住在共享实现里，本文件不重复实现一份 ——
/// 否则"守卫本身"就成了第二份会漂移的口径。
#[test]
fn the_tool_path_uses_the_model_inverse_entry_point_only() {
    let sources = undo_session::read_rust_sources(&undo_session::production_source_roots());
    let violations = undo_session::scan_second_undo_implementations(&sources);
    assert!(
        violations.is_empty(),
        "生产代码里出现了第二份撤销实现: {violations:?}"
    );
    let shared = sources
        .iter()
        .find(|(path, _)| path.ends_with("src/undo_session.rs"))
        .expect("共享实现必须在扫描集合里");
    assert!(
        shared.1.contains(".undo_with("),
        "共享实现必须真的调用模型入口 `CommitGraph::undo_with`"
    );
}

/// 判据：拒绝路径**不留下半成品**（拒绝之后工具的下一步仍然正常）。
#[test]
fn a_refusal_leaves_the_session_usable() {
    let (mut dispatcher, auth) = new_dispatcher();
    seed(&mut dispatcher, "refusal-usable");
    for _ in 0..2 {
        let refused = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
        assert_eq!(domain_error(&refused, "无历史"), "INDEX_OUT_OF_BOUNDS");
    }
    // 拒绝之后仍然能正常做事：提案 → 合并 → 撤销。
    let pristine = project_bytes(&dispatcher);
    let proposal = edit_velocity(&mut dispatcher, &auth, 42);
    merge(&mut dispatcher, &auth, &proposal);
    assert_ne!(project_bytes(&dispatcher), pristine);
    let undone = call(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(undone["data"]["steps"], 1);
    assert_eq!(project_bytes(&dispatcher), pristine);
}

/// 判据：`yeban_undo` 的拒绝**不是**实现级出口（不伪造 `-32005`）。
#[test]
fn undo_refusals_are_in_band_not_implementation_errors() {
    let (mut dispatcher, auth) = new_dispatcher();
    // 没有活跃工程也一样：带内 NO_ACTIVE_PROJECT。
    let (status, outcome) = call_raw(&mut dispatcher, &auth, "yeban_undo", json!({}));
    assert_eq!(status, 200);
    let result = outcome.expect("必须带内");
    assert_eq!(result["status"], "error");
    assert!(
        yeban_mcp::tools::ErrorCode::SCHEMA_CONTRACT
            .iter()
            .any(|code| code.as_str() == result["error"]["code"].as_str().unwrap_or_default()),
        "错误码必须在契约联集内: {result}"
    );
    let _ = UndoRefusal::NoRedo; // 词表本身可枚举（类型存在的证据）。
}
