//! `autoMerge`（ADR-0029）的承重判据：三个提案类工具一次调用完成「提案 + 合并」。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | 1 | `autoMerge: true` ⇒ 一次调用后提案状态就是 **merged**、工程真的变了 | `auto_merge_lands_in_one_call` |
//! | 2 | **不写该键**的调用方对 `autoMerge` 的**两条键形状**零感知（`pendingMerge: true`、无 `autoMerged`/`merge` 键） | `default_shape_is_unchanged_without_auto_merge` |
//! | 3 | `dryRun` + `autoMerge` ⇒ 一位都不改：提案**不会**被创建，预览如实说 `wouldAutoMerge` | `dry_run_writes_nothing_even_with_auto_merge` |
//! | 4 | `mergeCommitMessage` 原样进合并提交的 `commit.message`（缺省 `"merge proposal {id}"`） | `merge_commit_message_passes_through` |
//! | 5 | **不给 `mergeCommitMessage`** ⇒ 信息落回既有缺省 `"merge proposal {proposalId}"`（缺省轴，见 `documented_arg_coverage.rs` 第三条轴） | `absent_merge_commit_message_falls_back_to_the_proposal_id_default` |
//!
//! ⚠ **D4（合并失败 ⇒ 提案保持 Open）不在此列**：`apply_propose` 与 `apply_merge`
//! 在同一把锁内背靠背执行，基线在两步之间被移动需要宿主竞态，
//! 测试面无法从 `tools/call` 内构造；该分支由代码走查 + ADR-0029 D4 的
//! "结构化错误带 `proposalCreated`/`proposalId`" 契约钉住，⛔ 不造假夹具。

use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::COMMIT_MODEL_KEY;
use yeban_model::EntityId;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-automerge-{}-{tag}-{}",
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

/// 走真实 `tools/call` 管线。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "auto-merge",
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
    let result = response.result.expect("必须有 result");
    assert_eq!(result["status"], "success", "{name} 工具层失败: {result}");
    result["data"].clone()
}

/// 开一个新建工程（不播种材料：本文件的四组调用一律走 `materialSource: "theory"`，
/// 乐理生成路径**空池也可用**，见 `domain/section_build.rs` 的 `ADR-0016` 一节）。
fn open_fresh(dispatcher: &mut Dispatcher, auth: &str, scratch: &Scratch) -> Value {
    let path = scratch.dir.join("auto.yebanproject");
    call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "autoMerge 判据工程",
        }),
    )
}

#[test]
fn auto_merge_lands_in_one_call() {
    let scratch = Scratch::new("land");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let data = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Verse",
            "stylePreset": "pop",
            "materialSource": "theory",
            "bars": 2,
            "autoMerge": true,
        }),
    );
    assert_eq!(data["created"], json!(true), "{data}");
    assert_eq!(
        data["pendingMerge"],
        json!(false),
        "autoMerge 之后不得再教调用方去 merge: {data}"
    );
    assert_eq!(data["autoMerged"], json!(true), "{data}");
    assert_eq!(
        data["mergeTool"],
        json!("applied-in-process:autoMerge"),
        "mergeTool 必须如实说明合并已在进程内发生: {data}"
    );
    assert_eq!(
        data["projectUnchanged"],
        json!(false),
        "autoMerge 已施加 ⇒ projectUnchanged 必须是假话的反面: {data}"
    );
    assert_eq!(data["merge"]["merged"], json!(true), "{data}");
    data["proposal"]["proposalId"]
        .as_str()
        .expect("提案必须带 proposalId");
    assert_eq!(
        data["proposal"]["status"],
        json!("merged"),
        "merged 的权威读数是回执里的 `proposal` 投影（`ADR-0029` D3）：\
         `list_proposals` 只列 open，不会在这条历史里给出 status"
    );

    let list = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    // `list_proposals` 的既有口径是**只列未合并（open）的提案**
    // （`note: "当前没有未合并的提案。"`），合并后的那条只在计数里留痕。
    // ⇒ 自动合并成功的机械读数 = openCount 归零 + totalCount 仍算这条历史。
    assert_eq!(
        list["openCount"],
        json!(0),
        "autoMerge 后不该再有 open 提案: {list}"
    );
    assert_eq!(
        list["totalCount"],
        json!(1),
        "历史计数必须包含这条已合并提案: {list}"
    );
    assert_eq!(
        list["proposals"]
            .as_array()
            .expect("`proposals` 必须是数组")
            .len(),
        0,
        "清单只列 open ⇒ 已合并的那条不得出现在这里: {list}"
    );
    // 工程**真的**变了：合并回执里的 digest 与提案创建回执（改前）不同。
    assert_ne!(
        data["projectDigest"], data["merge"]["projectDigest"],
        "autoMerge 后工程摘要必须移动: {data}"
    );
}

#[test]
fn default_shape_is_unchanged_without_auto_merge() {
    let scratch = Scratch::new("default");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let data = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({"sectionName": "Chorus", "stylePreset": "pop", "materialSource": "theory", "bars": 2}),
    );
    assert_eq!(data["pendingMerge"], json!(true), "{data}");
    assert_eq!(data["projectUnchanged"], json!(true), "{data}");
    assert_eq!(
        data["mergeTool"],
        json!("yeban_merge_proposal"),
        "缺省路径的 mergeTool 必须逐字节是今天的样子: {data}"
    );
    assert!(
        data.get("autoMerged").is_none(),
        "不写 autoMerge 的调用方对**这条键**零感知 ⇒ 不得出现 autoMerged 键: {data}"
    );
    assert!(
        data.get("merge").is_none(),
        "不写 autoMerge 的调用方对**这条键**零感知 ⇒ 不得出现 merge 键: {data}"
    );
    // ⭐ `ADR-0032` **D4**：原话"逐字节零感知"的范围**就地收窄**到 `autoMerge` 的
    // 两条键形状（上面两条断言的强度一位都不减）。本票给**每条**成功回执新增一个
    // 自述键 `commitModel`（`ADR-0029` 之后落地），缺省路径因此不再是逐字节相同 ——
    // 把它钉成显式读数，⛔ 不让它成为没人登记的漂移。
    assert_eq!(
        data[COMMIT_MODEL_KEY],
        json!("proposal"),
        "缺省路径仍然是提案模型（回执自述与 `pendingMerge: true` 同向）: {data}"
    );

    // 手动 merge 仍然是唯一落盘路径，且语义与既有判据一致。
    let proposal_id = data["proposal"]["proposalId"].as_str().unwrap();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id}),
    );
    assert_eq!(merged["merged"], json!(true), "{merged}");
    assert_eq!(
        merged["commit"]["message"],
        json!(format!("merge proposal {proposal_id}")),
        "commitMessage 缺省口径必须与 plan_merge 的既有缺省一致"
    );
}

#[test]
fn dry_run_writes_nothing_even_with_auto_merge() {
    let scratch = Scratch::new("dry");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let dry = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Bridge",
            "stylePreset": "pop",
            "materialSource": "theory",
            "bars": 2,
            "autoMerge": true,
            "dryRun": true,
        }),
    );
    let preview = &dry["preview"];
    assert_eq!(
        preview["wouldAutoMerge"],
        json!(true),
        "预览必须如实说这一步会顺带合并: {dry}"
    );

    let list = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    assert_eq!(
        list["proposals"]
            .as_array()
            .expect("`proposals` 必须是数组")
            .len(),
        0,
        "dryRun 一位都不改 ⇒ 提案必须不存在: {list}"
    );
}

#[test]
fn merge_commit_message_passes_through() {
    let scratch = Scratch::new("msg");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let data = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Outro",
            "stylePreset": "pop",
            "materialSource": "theory",
            "bars": 2,
            "autoMerge": true,
            "mergeCommitMessage": "outro 一次到位",
        }),
    );
    assert_eq!(
        data["merge"]["commit"]["message"],
        json!("outro 一次到位"),
        "mergeCommitMessage 必须原样进合并提交: {data}"
    );
    assert_eq!(data["merge"]["commit"]["isMerge"], json!(true), "{data}");
}

/// ⭐ 判据 5（`mergeCommitMessage` 的**缺省那面**，效果面登记表第三条轴）：
/// **不给该键** ⇒ 合并提交的信息必须落回代码里的既有缺省 `"merge proposal {proposalId}"`
/// （`src/domain/mod.rs` 的 `merge_commit_message.unwrap_or_else(...)`）。
///
/// 本文件头表第 4 行一直写着"（缺省 `"merge proposal {id}"`）"，而判据 4 只跑了
/// **给了**那一支 —— 缺省支此前是**说明性文字**，⛔ 是断言。这一条把它变成断言：
/// 期望值从**同一份回执里的 `proposalId`** 现拼，⛔ 抄一个固定串（提案 id 每次都在换）。
#[test]
fn absent_merge_commit_message_falls_back_to_the_proposal_id_default() {
    let scratch = Scratch::new("msg-default");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let data = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Outro",
            "stylePreset": "pop",
            "materialSource": "theory",
            "bars": 2,
            "autoMerge": true,
        }),
    );
    let proposal_id = data["proposal"]["proposalId"]
        .as_str()
        .expect("自动合并的回执必须回显 proposalId");
    assert_eq!(
        data["merge"]["commit"]["message"],
        json!(format!("merge proposal {proposal_id}")),
        "不给 mergeCommitMessage ⇒ 信息必须是既有缺省文案（⛔ 空串、⛔ 段落名）: {data}"
    );
}
