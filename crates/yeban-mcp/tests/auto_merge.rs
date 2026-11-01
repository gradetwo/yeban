//! `autoMerge`（ADR-0029）的承重判据：三个提案类工具一次调用完成「提案 + 合并」。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | 1 | `autoMerge: true` ⇒ 一次调用后提案状态就是 **merged**、工程真的变了 | `auto_merge_lands_in_one_call` |
//! | 2 | **不写该键**的调用方对 `autoMerge` 的**两条键形状**零感知（`pendingMerge: true`、无 `autoMerged`/`merge` 键） | `default_shape_is_unchanged_without_auto_merge` |
//! | 3 | `dryRun` + `autoMerge` ⇒ 一位都不改：提案**不会**被创建，预览如实说 `wouldAutoMerge` | `dry_run_writes_nothing_even_with_auto_merge` |
//! | 4 | `mergeCommitMessage` 原样进合并提交的 `commit.message`（缺省 `"merge proposal {id}"`） | `merge_commit_message_passes_through` |
//! | 5 | **不给 `mergeCommitMessage`** ⇒ 信息落回既有缺省 `"merge proposal {proposalId}"`（缺省轴，见 `documented_arg_coverage.rs` 第三条轴） | `absent_merge_commit_message_falls_back_to_the_proposal_id_default` |
//! | 6 | 三个提案类工具广播的 `data.merge` 槽**同源**，且核得过**真实**回执（长形状过 `validate_complete`；真实短形状在外层过、在槽里红） | `the_broadcast_merge_slot_validates_the_real_auto_merge_receipt` |
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

/// ⭐ s33 判据 6（**真实回执**反校新广播的 `data.merge` 槽）：三个提案类工具广播出去的
/// `merge` 槽，必须核得过一次真 `tools/call` 出来的 `data.merge` —— 用
/// [`validate_complete`]（"声明的每个键都得在"），因为这一槽⚠ **只有一种形状**：
/// 拼装点合并的是本次调用刚创建的那条提案，`apply_merge` 的幂等短形状在这条路径上不可达。
///
/// 同一处还钉外层/内层的**不对称**：`commitModel` 只由派发器注入**外层** `data`，
/// `data.merge` 是 `apply_merge` 未经注入的原始 `data` ⇒ 这一槽里⛔ 没有该键。
/// （早先设计稿曾把 `merge_proposal()` 原样复用给这三槽，那会在真实回执上要求一个
/// 生产端不会发的键 —— 这条判据就是那一步的防线。）
#[test]
fn the_broadcast_merge_slot_validates_the_real_auto_merge_receipt() {
    use yeban_mcp::output_schema;

    let scratch = Scratch::new("merge-slot");
    let (mut dispatcher, auth) = fresh_dispatcher();
    open_fresh(&mut dispatcher, &auth, &scratch);

    let data = call(
        &mut dispatcher,
        &auth,
        "yeban_propose_section",
        json!({
            "sectionName": "Bridge",
            "stylePreset": "pop",
            "materialSource": "theory",
            "bars": 2,
            "autoMerge": true,
        }),
    );

    // ① 三个工具的广播槽必须**读回同一个值**（单一来源在广播侧的读数）。
    let slots: Vec<Value> = [
        "yeban_propose_section",
        "yeban_edit_notes",
        "yeban_set_macro",
    ]
    .iter()
    .map(|name| {
        let schema = output_schema::for_tool(name).expect("三个提案类工具都广播了 outputSchema");
        assert!(
            !schema["required"]
                .as_array()
                .expect("required 必须是数组")
                .iter()
                .any(|key| key == "merge"),
            "`{name}` 的缺省形状没有 merge ⇒ 它不得进顶层 required: {schema}"
        );
        let slot = schema["properties"]["merge"].clone();
        assert_eq!(
            slot["required"]
                .as_array()
                .expect("`data.merge` 的 required 必须是数组")
                .len(),
            7,
            "`{name}` 广播的 merge 槽必须逐键声明（此前那是裸对象声明）"
        );
        slot
    })
    .collect();
    assert_eq!(slots.len(), 3, "三个提案类工具都必须有这一槽");
    assert_eq!(slots[0], slots[1], "三个工具的 merge 槽必须同源");
    assert_eq!(slots[1], slots[2], "三个工具的 merge 槽必须同源");

    // ② 真实回执必须过自己广播的那一份（⚠ 用 `validate_complete`：七键都得在）。
    output_schema::validate_complete(&slots[0], &data["merge"]).unwrap_or_else(|violation| {
        panic!(
            "真实 `data.merge` 违反广播的槽: {violation}\n{}",
            data["merge"]
        )
    });

    // ③ 外层/内层的不对称是**回执里读回来的**，不是 schema 里推断的。
    assert_eq!(
        data[COMMIT_MODEL_KEY],
        json!("proposalAutoMerged"),
        "外层必须带 commitModel（派发器注入外层 data）: {data}"
    );
    assert!(
        data["merge"].get(COMMIT_MODEL_KEY).is_none(),
        "`data.merge` 是 apply_merge 的原始 data ⇒ 里面不该有 {COMMIT_MODEL_KEY}: {}",
        data["merge"]
    );

    // ④ 真实**短形状**回执（把已合并的提案再合并一次）：外层必须收、这一槽必须红 ——
    // "这一槽只有一种形状"那句话的证据来自**回执本身**，⛔ 来自手写夹具。
    let proposal_id = data["proposal"]["proposalId"]
        .as_str()
        .expect("提案回执必须回显 proposalId")
        .to_owned();
    let again = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "重复合并"}),
    );
    assert_eq!(again["alreadyMerged"], json!(true), "{again}");
    assert!(
        again.get("commit").is_none(),
        "短形状里没有 commit ⇒ 它只能是外层的形状: {again}"
    );
    let outer = output_schema::for_tool("yeban_merge_proposal").expect("schema");
    output_schema::validate(&outer, &again)
        .unwrap_or_else(|violation| panic!("外层必须收短形状: {violation}\n{again}"));
    assert_eq!(
        output_schema::validate(&slots[0], &again)
            .expect_err("同一份短形状喂进 `data.merge` 槽必须红（这条路径上不可达）")
            .path,
        "$.baseCommitMoved",
        "红必须点名短形状缺席的那个键: {again}"
    );
}
