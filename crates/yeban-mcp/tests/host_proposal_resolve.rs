//! 宿主（界面抽屉）的**采纳 / 放弃**判据 —— `HostAction::MergeProposal` /
//! `HostAction::RejectProposal` 两臂（任务单 #29：抽屉「采纳 / 放弃」真接线）。
//!
//! 这一格补的洞（实测, 不是转述）：抽屉的 `resolve-available` 今天恒 `false`, 因为
//! 界面侧的写入口 `HostAction` 只有 `Undo` / `Redo` / `Commit` 三臂 —— 而领域侧
//! `yeban_merge_proposal` / `yeban_reject_proposal` 早就把这两件事做完了。
//! 本文件钉的是**接上之后不许漂移**：界面上的采纳必须与工具面的采纳落在同一条提交、
//! 同一个状态迁移、同一套拒绝码上。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | 1 | 宿主采纳的**预览**（`Plan::project_after`）与它**真落**的工程逐字段相同, 且与工具面 `dryRun` 的 `projectDigestAfter` 同摘要 | [`the_host_merge_previews_exactly_what_it_lands`] |
//! | 2 | 宿主采纳落的是一条**多父合并提交**（`is_merge`, 两个父），血缘与工具路径同一实现 | [`the_host_merge_lands_a_multi_parent_commit_on_the_proposal_lineage`] |
//! | 3 | 宿主放弃**只改记录**：状态 `rejected` + 原因, 工程、提交数、施加修订号、撤销游标**一位都不动** | [`the_host_reject_flips_only_the_record`] |
//! | 4 | 再采纳一次是**幂等命中**：`steps == 0` / `commit == None` / `op_kinds` 空, 读数不变 | [`a_second_host_merge_is_an_idempotent_no_step`] |
//! | 5 | 已放弃的提案**不许**被宿主采纳, 错误码与工具面逐字相同（`CONFLICT`）, 且状态不变 | [`the_host_merge_refuses_a_rejected_proposal_like_the_tool_does`] |
//! | 6 | 非法 / 不存在的 `proposalId`：宿主面的错误码与工具面**逐字相同**, 工程一位不动 | [`the_host_path_refuses_the_ids_the_tool_path_refuses`] |
//! | 7 | 抽屉读到的那行 `ProposalRow::id` 就是宿主写入口要的那个身份（读面 ↔ 写面闭环） | [`the_row_id_the_drawer_reads_is_the_id_the_host_write_takes`] |
//! | 8 | 自述读数一致：宿主采纳 `commitModel == "direct"`、宿主放弃 `== "none"`, 与工具面回执同值 | [`the_host_arms_report_the_commit_model_the_tool_arms_report`] |
//! | 9 | **只有一份合并实现**（源码层）：`commit_merge` / `Ok(Plan::Merge {` / `Ok(Plan::Reject {` 各恰好一处 | [`there_is_exactly_one_merge_and_one_reject_implementation`] |
//!
//! ⚠ 判据 9 是**文本层**的, 它证明的是"结构上长不出第二套合并", ⛔ 不替代上面任何一条
//! 运行时判据；每条负向断言都配了正向对照（同一探针里必须**读得到**被禁的东西之外的
//! 那些名字）, 否则"文件没读到"也会通过。
//!
//! 单位：一条判据 = 一个 `Dispatcher` 会话上的一组断言（`apply_host_action` 直连
//! `dispatcher.domain_mut()` —— 与宿主经 `HttpServer::apply_host_action` 拿到的是
//! **同一个** `Mutex<Dispatcher>` 里的那一个 `Domain`）。

use std::path::PathBuf;
use std::str::FromStr as _;

use serde_json::{Value, json};

use yeban_mcp::Dispatcher;
use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::proposal::ProposalStatus;
use yeban_mcp::domain::store;
use yeban_mcp::domain::{HostAction, Plan, apply_host_action, commit_model_of};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{COMMIT_MODEL_KEY, ErrorCode};
use yeban_model::EntityId;

/// 一个只含确定性命名字段的临时目录（真开工程要落盘路径）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-host-resolve-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线，返回**整个** `ToolResponse` 值（成功与带内失败都要读）。
fn call_full(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "host-resolve",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "{name} 不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

/// 成功调用的 `data`。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let full = call_full(dispatcher, auth, name, arguments);
    assert_eq!(full["status"], "success", "{name} 工具层失败: {full}");
    full["data"].clone()
}

/// 带内领域失败的错误码原文（与契约 enum 同一字符串）。
fn tool_error_code(
    dispatcher: &mut Dispatcher,
    auth: &str,
    name: &str,
    arguments: Value,
) -> String {
    let full = call_full(dispatcher, auth, name, arguments);
    assert_eq!(full["status"], "error", "{name} 应当带内失败: {full}");
    full["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} 缺错误码: {full}"))
        .to_owned()
}

/// [`Fault`] 的契约码原文；`Fault::Impl`（实现级状况）如实报 `None`。
fn fault_code(fault: &Fault) -> Option<&'static str> {
    match fault {
        Fault::Domain { code, .. } => Some(code.as_str()),
        Fault::Impl { .. } => None,
    }
}

/// 开一个新建工程（乐理生成路径空池可用, 见 `auto_merge.rs` 同句注释）。
fn open_fresh(dispatcher: &mut Dispatcher, auth: &str, scratch: &Scratch) {
    let path = scratch.dir.join("host.yebanproject");
    call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "宿主采纳判据工程",
        }),
    );
}

/// 让 AI 提一条案（走真实工具），返回其身份原文。
fn propose(dispatcher: &mut Dispatcher, auth: &str) -> String {
    let data = call(
        dispatcher,
        auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Verse",
            "stylePreset": "pop",
            "materialSource": "theory",
            "bars": 2,
        }),
    );
    data["proposal"]["proposalId"]
        .as_str()
        .expect("提案必须带 proposalId")
        .to_owned()
}

/// 会话的**可变读数**（判据用它证明"一位都没动"）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Readings {
    digest: Option<String>,
    commits: usize,
    head: Option<String>,
    revision: u64,
    undone: usize,
    open: Vec<String>,
}

fn read(dispatcher: &Dispatcher) -> Readings {
    let domain = dispatcher.domain();
    Readings {
        digest: domain.project_digest(),
        commits: domain.commit_count(),
        head: domain.active_head().map(|id| id.to_canonical_string()),
        revision: domain.apply_revision(),
        undone: domain.undo_display().undone,
        open: domain
            .open_proposal_rows()
            .into_iter()
            .map(|row| row.id)
            .collect(),
    }
}

fn id_of(text: &str) -> EntityId {
    EntityId::from_str(text).expect("判据用的身份必须是合法 ULID")
}

fn merge_action(proposal_id: &str) -> HostAction {
    HostAction::MergeProposal {
        proposal_id: proposal_id.to_owned(),
        commit_message: None,
    }
}

fn reject_action(proposal_id: &str, reason: &str) -> HostAction {
    HostAction::RejectProposal {
        proposal_id: proposal_id.to_owned(),
        reason: Some(reason.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// ① 预览 == 真做
// ---------------------------------------------------------------------------

/// 宿主采纳**预览**的就是它落下的那份工程 —— 抽屉将来要显示的 diff 因此不可能与
/// 图谱里的不一致（`simulate_merge` 只有一处实现, `Plan::Merge` 与 `Plan::Host` 共用）。
#[test]
fn the_host_merge_previews_exactly_what_it_lands() {
    let scratch = Scratch::new("preview");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);

    let plan = Plan::Host(merge_action(&proposal_id));
    // 采纳真的动工作工程 ⇒ 施加修订号必须推进（界面重投影的触发口径就在这一个读数上）。
    assert!(
        plan.mutates_project(),
        "宿主采纳必须与 `Plan::Merge` 同口径: 它真的改工作工程"
    );
    assert_eq!(plan.commit_delta(), 1, "采纳恰好加一条提交");

    let previewed = plan
        .project_after(dispatcher.domain())
        .expect("宿主预览不得失败")
        .expect("采纳必然给出工程预览");
    let preview_digest = store::digest_of(
        store::serialize_project(&previewed)
            .expect("预览工程必须可序列化")
            .as_bytes(),
    );

    // 工具面 `dryRun` 的预览走的是**同一个** `project_after`（经 `Plan::Merge`）
    // ⇒ 同一份字节、同一个摘要。两个入口两套预览就是这个洞的原始形态。
    let dry = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "dryRun": true}),
    );
    assert_eq!(
        dry["preview"]["projectDigestAfter"],
        json!(preview_digest),
        "工具面 dryRun 与宿主面的预览摘要必须相同: {dry}"
    );

    let outcome = apply_host_action(dispatcher.domain_mut(), merge_action(&proposal_id))
        .expect("宿主采纳必须成功");
    assert_eq!(outcome.steps, 1, "真落了一步合并: {outcome:?}");

    let landed = dispatcher
        .domain()
        .active_project()
        .cloned()
        .expect("已开工程");
    assert_eq!(
        landed, previewed,
        "预览 ≠ 真做：界面上给用户看的那份差异不是将要发生的那份"
    );
    assert_eq!(
        dispatcher.domain().project_digest().as_deref(),
        Some(preview_digest.as_str()),
        "落盘口径的工程摘要必须等于预览算出的那一个"
    );
}

// ---------------------------------------------------------------------------
// ② 血缘
// ---------------------------------------------------------------------------

/// 宿主采纳落下的是**多父合并提交**, 且提案记录里登记的 `mergeCommit` 就是它。
///
/// 这条判据的存在理由：`OpOrigin::McpProposal` 的血缘不能因为"按钮在界面上"而改写 ——
/// "谁按下的"与"这条提交并了谁"是两条轴。
#[test]
fn the_host_merge_lands_a_multi_parent_commit_on_the_proposal_lineage() {
    let scratch = Scratch::new("lineage");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);
    let before = read(&dispatcher);
    assert_eq!(before.open.len(), 1, "提案未合并时必须看得见: {before:?}");

    let outcome = apply_host_action(dispatcher.domain_mut(), merge_action(&proposal_id))
        .expect("宿主采纳必须成功");
    let head = outcome.commit.expect("落了提交就必须报身份");
    let record = dispatcher
        .domain()
        .graph()
        .commit(&head)
        .expect("新头必须在图谱里");

    assert!(record.is_merge(), "采纳必须并出提案分支, 不是线性追加");
    assert_eq!(record.parents.len(), 2, "两个父: 活跃头 + 提案分支头");
    let expected_message = format!("merge proposal {id}", id = id_of(&proposal_id));
    assert_eq!(
        record.message, expected_message,
        "缺省提交信息口径必须与工具面 `merge_plan` 完全相同"
    );
    assert!(
        !outcome.op_kinds.is_empty(),
        "采纳真的施加了提案那一批 op ⇒ op_kinds 必须报出来: {outcome:?}"
    );

    let after = read(&dispatcher);
    assert_eq!(after.commits, before.commits + 1, "{before:?} → {after:?}");
    assert_eq!(after.revision, before.revision + 1, "采纳推进施加修订号");
    assert!(after.open.is_empty(), "采纳后清单里不得再出现这条提案");

    let proposal = dispatcher
        .domain()
        .proposal(&id_of(&proposal_id))
        .expect("提案记录必须还在（幂等靠它）");
    assert_eq!(proposal.status, ProposalStatus::Merged);
    assert_eq!(
        proposal.merge_commit,
        Some(head),
        "提案记录登记的合并提交必须就是图上那条"
    );
}

// ---------------------------------------------------------------------------
// ③ 放弃只改记录
// ---------------------------------------------------------------------------

/// 宿主放弃**只**把记录翻成 `rejected`：工程字节、提交数、施加修订号、撤销游标全不变。
#[test]
fn the_host_reject_flips_only_the_record() {
    let scratch = Scratch::new("reject");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);
    let before = read(&dispatcher);

    let plan = Plan::Host(reject_action(&proposal_id, "太满"));
    assert!(
        !plan.mutates_project(),
        "放弃不动工作工程 —— 与 `Plan::Reject` 同一读数"
    );
    assert_eq!(plan.commit_delta(), 0);
    assert!(
        plan.project_after(dispatcher.domain())
            .expect("放弃不得失败")
            .is_none(),
        "放弃的工程预览必须是 `None`（= 内容不变）, ⛔ 不是「顺手删掉提案」"
    );

    let outcome = apply_host_action(dispatcher.domain_mut(), reject_action(&proposal_id, "太满"))
        .expect("宿主放弃必须成功");
    assert_eq!(outcome.steps, 0, "放弃不产生提交 ⇒ 步数恒 0");
    assert_eq!(outcome.commit, None);
    assert!(
        outcome.op_kinds.is_empty(),
        "放弃一条 op 都不施加: {outcome:?}"
    );

    let after = read(&dispatcher);
    assert_eq!(
        after,
        Readings {
            open: Vec::new(),
            ..before.clone()
        },
        "放弃之后除了清单, 一个读数都不许变"
    );

    let proposal = dispatcher
        .domain()
        .proposal(&id_of(&proposal_id))
        .expect("放弃**绝不**删除记录");
    assert_eq!(proposal.status, ProposalStatus::Rejected);
    assert_eq!(
        proposal.resolution.as_deref(),
        Some("太满"),
        "原因原文进记录（界面不许再拼一份文案）"
    );
    assert_eq!(proposal.merge_commit, None);

    // 与工具面同一条记录、同一个终态：再调一次工具面放弃是幂等命中。
    let tool_data = call(
        &mut dispatcher,
        &auth,
        "yeban_reject_proposal",
        json!({"proposalId": proposal_id, "reason": "再来一次"}),
    );
    assert_eq!(tool_data["alreadyRejected"], json!(true), "{tool_data}");
}

// ---------------------------------------------------------------------------
// ④ 幂等
// ---------------------------------------------------------------------------

/// 已采纳的提案再点一次采纳 ⇒ **一步都不落**, 三个读数（步数 / 提交身份 / op 名单）
/// 全部如实为空。
///
/// 这一条是 `HostOutcome` 的诚实性钉子：`steps` 从图谱的**实际变化**里读, 不从
/// "我请求了什么"里推。把请求当成结果, 界面上就会出现"提示成功并了新提交, 而时光机里
/// 什么都没有"。
#[test]
fn a_second_host_merge_is_an_idempotent_no_step() {
    let scratch = Scratch::new("idempotent");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);

    apply_host_action(dispatcher.domain_mut(), merge_action(&proposal_id))
        .expect("第一次宿主采纳必须成功");
    let after_first = read(&dispatcher);

    let second = apply_host_action(dispatcher.domain_mut(), merge_action(&proposal_id))
        .expect("幂等命中不是失败");
    assert_eq!(second.steps, 0, "第二次采纳一位都没动: {second:?}");
    assert_eq!(second.commit, None, "不许把旧提交冒充成新落的");
    assert!(
        second.op_kinds.is_empty(),
        "没施加任何 op 时 op_kinds 必须是空, ⛔ 不能报提案里那一批: {second:?}"
    );

    let after_second = read(&dispatcher);
    assert_eq!(after_second.digest, after_first.digest);
    assert_eq!(after_second.commits, after_first.commits);
    assert_eq!(after_second.head, after_first.head);
    assert_eq!(after_second.undone, after_first.undone);
    assert_eq!(after_second.open, after_first.open);
    // ⚠ `apply_revision` 的口径是"施加过**可能**改工程的计划"（见字段文档）⇒ 幂等命中
    // 同样经过唯一施加口, 它如实 +1。界面上"这次动了什么"读的是 `steps` / `commit`,
    // ⛔ 不是这个数（把它当步数显示就会出现"提示动了 1 步而时光机里什么都没有"）。
    assert_eq!(
        after_second.revision,
        after_first.revision + 1,
        "幂等命中不改工程内容, 但它确实经过了一次施加口"
    );
}

// ---------------------------------------------------------------------------
// ⑤ / ⑥ 拒绝与错误码
// ---------------------------------------------------------------------------

/// 已放弃的提案**不许**被宿主采纳, 且错误码与工具面**逐字相同**（`CONFLICT`）, 状态不变。
#[test]
fn the_host_merge_refuses_a_rejected_proposal_like_the_tool_does() {
    let scratch = Scratch::new("rejected");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);
    apply_host_action(
        dispatcher.domain_mut(),
        reject_action(&proposal_id, "先不用"),
    )
    .expect("宿主放弃必须成功");
    let before = read(&dispatcher);

    let tool_code = tool_error_code(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id}),
    );
    let fault = apply_host_action(dispatcher.domain_mut(), merge_action(&proposal_id))
        .expect_err("已放弃的提案不得被采纳");
    assert_eq!(
        fault_code(&fault),
        Some(ErrorCode::Conflict.as_str()),
        "宿主面的拒绝码必须与工具面同一份: 工具={tool_code} 宿主={fault:?}"
    );
    assert_eq!(fault_code(&fault), Some(tool_code.as_str()));

    let after = read(&dispatcher);
    assert_eq!(after, before, "被拒的采纳必须一位都不动");
}

/// 非法 / 不存在的 `proposalId`：宿主面与工具面给出**同一份**错误码, 工程一位不动。
///
/// 这一条钉的是"界面上按了按钮却什么也没说清"那一类缺陷：抽屉传的是字符串身份, 解析
/// 失败与查无此人必须分得开（`parse_id` / `proposal_not_found` 都是工具面在用的函数）。
#[test]
fn the_host_path_refuses_the_ids_the_tool_path_refuses() {
    let scratch = Scratch::new("bad-id");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);
    let before = read(&dispatcher);

    let unknown = EntityId::new().to_canonical_string();
    let cases = [
        ("不是 ULID", ErrorCode::InvalidParameterRange),
        (unknown.as_str(), ErrorCode::ProposalNotFound),
    ];
    for (text, expected) in cases {
        let host_fault = apply_host_action(dispatcher.domain_mut(), merge_action(text))
            .expect_err("非法或查无此人的身份必须响亮拒绝");
        assert_eq!(
            fault_code(&host_fault),
            Some(expected.as_str()),
            "宿主面 `{text}`: {host_fault:?}"
        );
        let tool_code = tool_error_code(
            &mut dispatcher,
            &auth,
            "yeban_merge_proposal",
            json!({"proposalId": text}),
        );
        assert_eq!(
            tool_code,
            expected.as_str(),
            "工具面对同一个 `{text}` 必须给同一个码"
        );

        let reject_fault = apply_host_action(dispatcher.domain_mut(), reject_action(text, "无"))
            .expect_err("放弃面同样必须拒绝");
        assert_eq!(
            fault_code(&reject_fault),
            Some(expected.as_str()),
            "宿主放弃 `{text}`: {reject_fault:?}"
        );

        let after = read(&dispatcher);
        assert_eq!(after, before, "被拒的动作一位都不许动（含 `{text}`）");
    }
    assert_eq!(
        before.open,
        vec![proposal_id],
        "正向对照：这一会话确实有一条可采纳的提案, 上面的拒绝不是因为清单本来就空"
    );
}

// ---------------------------------------------------------------------------
// ⑦ 读面 ↔ 写面闭环
// ---------------------------------------------------------------------------

/// 抽屉读到的那一行 `ProposalRow::id`, 就是宿主写入口要的 `proposalId` —— 中间不需要
/// 界面自己造任何身份。
#[test]
fn the_row_id_the_drawer_reads_is_the_id_the_host_write_takes() {
    let scratch = Scratch::new("row-id");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);
    let proposal_id = propose(&mut dispatcher, &auth);

    // 宿主的读面（`yeban-app` 的 `ProposalSource::rows()` 经 `open_proposal_rows()`
    // 取的就是这一份, 顺序也是同一份）。
    let rows = dispatcher.domain().open_proposal_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].id, proposal_id, "行身份必须就是领域的提案身份");
    assert_eq!(rows[0].status, "open");
    assert!(rows[0].op_count > 0, "提案是一个 op 批次, 条数是真实读数");

    apply_host_action(dispatcher.domain_mut(), merge_action(&rows[0].id))
        .expect("用界面读到的那个身份必须能直接采纳");
    assert!(
        dispatcher.domain().open_proposal_rows().is_empty(),
        "采纳后行必须从清单里消失（否则抽屉会一直画一条已经并掉的提案）"
    );
    let tool_list = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    assert_eq!(
        tool_list["proposals"].as_array().map(Vec::len),
        Some(0),
        "工具面与界面在同一条历史里读到同一件事: {tool_list}"
    );
}

// ---------------------------------------------------------------------------
// ⑧ 自述读数
// ---------------------------------------------------------------------------

/// `commitModel` 这个自述在两个入口上必须同值：宿主采纳 = 工具采纳 = `direct`,
/// 宿主放弃 = 工具放弃 = `none`。
#[test]
fn the_host_arms_report_the_commit_model_the_tool_arms_report() {
    let scratch = Scratch::new("commit-model");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let merge_id = propose(&mut dispatcher, &auth);
    let reject_id = propose(&mut dispatcher, &auth);
    assert_eq!(read(&dispatcher).open.len(), 2, "两条提案都要在清单里");

    assert_eq!(
        commit_model_of(&Plan::Host(merge_action(&merge_id))),
        "direct",
        "采纳当场生效, 没有「下一步去合并」"
    );
    assert_eq!(
        commit_model_of(&Plan::Host(reject_action(&reject_id, "不要"))),
        "none",
        "放弃不产生任何提交 —— 与 `Plan::Reject` 同一读数"
    );

    let merged = call_full(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": merge_id}),
    );
    assert_eq!(
        merged["data"][COMMIT_MODEL_KEY],
        json!(commit_model_of(&Plan::Host(merge_action(&merge_id)))),
        "工具回执的自述必须等于宿主面的分类: {merged}"
    );
    let rejected = call_full(
        &mut dispatcher,
        &auth,
        "yeban_reject_proposal",
        json!({"proposalId": reject_id, "reason": "不要"}),
    );
    assert_eq!(
        rejected["data"][COMMIT_MODEL_KEY],
        json!(commit_model_of(&Plan::Host(reject_action(
            &reject_id, "不要"
        )))),
        "同上, 放弃轴: {rejected}"
    );
}

// ---------------------------------------------------------------------------
// ⑨ 结构层：只有一份合并实现
// ---------------------------------------------------------------------------

/// 去掉注释后的源码文本（**文本层探针**的正则口径与 `yeban-app` 的同族判据一致）。
fn code_only(source: &str) -> String {
    let mut without_blocks = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        match rest[start..].find("*/") {
            Some(offset) => {
                without_blocks.push_str(&rest[..start]);
                rest = &rest[start + offset + 2..];
            }
            // 没有闭合的 `*/`：剩下的都是原文（本仓库不会出现, 但⛔ 不许在此 panic）。
            None => break,
        }
    }
    without_blocks.push_str(rest);
    without_blocks
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **只有一份**合并 / 拒绝实现：宿主两臂因此不可能长出第二套语义。
#[test]
fn there_is_exactly_one_merge_and_one_reject_implementation() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/domain/mod.rs");
    let code = code_only(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读不到 {}: {error}", path.display())),
    );
    assert!(
        !code.is_empty(),
        "探针必须真的读到源码（空文本会让下面每条都假通过）"
    );

    // 正向对照：这些名字**必须**在（它们是被复用的一侧）。
    for expected in [
        "fn apply_merge",
        "fn apply_reject",
        "fn merge_plan",
        "fn reject_plan",
        "fn simulate_merge",
        "HostAction::MergeProposal",
        "HostAction::RejectProposal",
    ] {
        assert!(
            code.contains(expected),
            "正向对照缺失：源码里没有 `{expected}`, 这个探针测不到任何东西"
        );
    }

    // 负向断言：合并提交的生产点 / 两个计划的构造点各**恰好一处**。
    for (site, expected) in [
        ("commit_merge(", 1),
        ("Ok(Plan::Merge {", 1),
        ("Ok(Plan::Reject {", 1),
        ("fn apply_merge", 1),
        ("fn apply_reject", 1),
    ] {
        let hits = code.matches(site).count();
        assert_eq!(
            hits,
            expected,
            "`{site}` 在 {} 的代码层出现 {hits} 次, 期望 {expected} 次 —— \
             两处就意味着界面上的采纳与工具面的采纳可以是两件不同的事",
            path.display()
        );
    }
}
