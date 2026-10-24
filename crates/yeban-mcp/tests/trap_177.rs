//! **`#177` 的判据**：`#174` 验收里剩下的四条低成本坑（⑤–⑧ 中的第 5–8 条，
//! 即坑④ / 坑⑩ / 坑⑪ / 坑⑥'）的机械读数。
//!
//! ## 这个文件量什么、单位是什么
//!
//! | 判据 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | [`add_track_no_longer_needs_a_clip_id`] | 一次 `addTrack`（不传 `clipId`） | 1 个 `status` + 提案数 +1 |
//! | [`the_old_call_with_a_clip_id_still_works`] | 同一次调用但**带** `clipId` | 1 个 `status`（向后兼容） |
//! | [`kinds_that_read_the_envelope_clip_report_a_loud_missing_error`] | 5 个音符形态缺 `clipId` | 5 条 `reason` + 5 份 `requiredByKinds` |
//! | [`a_clip_id_is_ignored_by_kinds_that_do_not_read_it`] | `addTrack` 给了**非法** ULID | 1 个 `status`（接受但忽略） |
//! | [`the_published_fields_enum_is_the_parser_whitelist`] | `tools/list` 的 `fields.items.enum` | **枚举成员个数**（= `SELECTORS` 的 24） |
//! | [`an_unknown_field_selector_is_refused_loudly`] | `fields:["notAField"]` | 1 个 `code` + 1 份 `availableSelectors` |
//! | [`the_undo_cursor_and_the_undo_readback_are_the_same_number`] | 同一会话两处读数 | **2 对**数字（1/1 与 0/0） |
//! | [`proposal_not_found_lists_the_known_proposals`] | 坏 `proposalId` 的 merge | `knownProposals` 里的**真 id 出现次数**（1） |
//! | [`every_ops_branch_publishes_the_envelope_clip_rule`] | `ops.items.oneOf` 的 38 个分支 | 分支数（38）+ 逐分支 `x-envelope.clipId` |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test trap_177 -- --nocapture`

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::notes::{ENVELOPE_CLIP_KINDS, OP_KINDS};
use yeban_mcp::domain::view::SELECTORS;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools;
use yeban_model::EntityId;

/// 注入规范样本工程的分发器（与 `tests/trap_175.rs` 同一形状）。
fn dispatcher() -> (Dispatcher, String, yeban_model::YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::path::PathBuf::from("/tmp/yeban-trap-177.yeban"),
            project.clone(),
            false,
        )
        .expect("注入规范工程");
    (dispatcher, authorization, project)
}

/// 走 `tools/call` 并返回带内 `ToolResponse`（成功或领域失败）。
fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "trap-177",
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

/// 一条**带摆放**的 `(trackId, clipId)`。
fn track_and_clip(project: &yeban_model::YebanProjectV1) -> (EntityId, EntityId) {
    project
        .tracks
        .values()
        .find_map(|track| {
            track
                .clips
                .values()
                .next()
                .map(|placement| (track.id, placement.clip_id))
        })
        .expect("样本里必须有一条带摆放的音轨")
}

/// 一次成功的 `addTrack`（**不**含 `clipId`，`ops` 自带 `trackId`）。
fn add_track_arguments(track_id: &str) -> Value {
    json!({
        "trackId": track_id,
        "dryRun": false,
        "ops": [{
            "kind": "addTrack",
            "trackId": "01J8ZQ0000000000000000A177",
            "name": "Trap 177 Track",
            "trackKind": "Midi",
        }],
    })
}

/// 判据 ①（§2.1）：`addTrack` 不再需要 `clipId` —— 不传也成功，且真的建了一条提案。
#[test]
fn add_track_no_longer_needs_a_clip_id() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, _clip_id) = track_and_clip(&project);
    let before = dispatcher.domain().proposal_count();
    let response = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        add_track_arguments(&track_id.to_canonical_string()),
    );
    assert_eq!(response["status"], "success", "{response}");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        before + 1,
        "不传 `clipId` 的 addTrack 必须真的建出一条提案"
    );
    assert_eq!(response["data"]["pendingMerge"], true, "{response}");
    println!(
        "### 逐字：addTrack 不传 clipId ⇒ status = {}, proposalCount = {} → {}",
        response["status"],
        before,
        before + 1
    );
}

/// 判据 ①（§2.1 后半）：**旧调用（带 `clipId`）仍成功** —— 向后兼容。
#[test]
fn the_old_call_with_a_clip_id_still_works() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let before = dispatcher.domain().proposal_count();
    let mut arguments = add_track_arguments(&track_id.to_canonical_string());
    arguments["clipId"] = Value::from(clip_id.to_canonical_string());
    let response = call(&mut dispatcher, &auth, "yeban_edit_notes", arguments);
    assert_eq!(response["status"], "success", "{response}");
    assert_eq!(
        dispatcher.domain().proposal_count(),
        before + 1,
        "带 `clipId` 的旧调用必须逐字节等价（仍建一条提案）"
    );
    println!(
        "### 逐字：addTrack 带 clipId ⇒ status = {}",
        response["status"]
    );
}

/// 判据 ②（§2.2）：**需要 `clipId` 的 kind 缺它 ⇒ 响亮**（五个音符形态各一条）。
#[test]
fn kinds_that_read_the_envelope_clip_report_a_loud_missing_error() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, _clip_id) = track_and_clip(&project);
    let note_id = "01J8ZQ0000000000000000B177";
    let cases: [(&str, Value); 5] = [
        (
            "add",
            json!({"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}),
        ),
        ("delete", json!({"kind": "delete", "noteId": note_id})),
        (
            "move",
            json!({"kind": "move", "noteId": note_id, "deltaTick": 0, "deltaPitch": 0}),
        ),
        (
            "velocity",
            json!({"kind": "velocity", "noteId": note_id, "velocity": 64}),
        ),
        (
            "setNoteTiming",
            json!({
                "kind": "setNoteTiming", "noteId": note_id,
                "startTick": 0, "durationTicks": 480,
            }),
        ),
    ];
    for (kind, op) in cases {
        let proposals_before = dispatcher.domain().proposal_count();
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({"trackId": track_id.to_canonical_string(), "ops": [op]}),
        );
        assert_eq!(
            response["status"], "error",
            "`{kind}` 缺 clipId 必须响亮: {response}"
        );
        assert_eq!(
            response["error"]["code"], "INVALID_PARAMETER_RANGE",
            "`{kind}` 的码: {response}"
        );
        assert_eq!(
            response["error"]["data"]["reason"], "clipIdRequired",
            "`{kind}` 的 reason: {response}"
        );
        assert_eq!(
            response["error"]["data"]["requiredByKinds"],
            json!([kind]),
            "`{kind}` 必须逐字点名自己: {response}"
        );
        assert_eq!(
            dispatcher.domain().proposal_count(),
            proposals_before,
            "被拒的 `{kind}` 不得留下提案"
        );
        println!(
            "### 逐字：{kind} 缺 clipId ⇒ {} / {} / requiredByKinds={}",
            response["error"]["code"],
            response["error"]["data"]["reason"],
            response["error"]["data"]["requiredByKinds"]
        );
    }
}

/// 判据 ② 的反面：**不需要 `clipId` 的 kind 给了就接受但忽略** —— 连非法 ULID 都不解析。
#[test]
fn a_clip_id_is_ignored_by_kinds_that_do_not_read_it() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, _clip_id) = track_and_clip(&project);
    let mut arguments = add_track_arguments(&track_id.to_canonical_string());
    arguments["clipId"] = Value::from("not-a-ulid-and-nobody-reads-it");
    let response = call(&mut dispatcher, &auth, "yeban_edit_notes", arguments);
    assert_eq!(
        response["status"], "success",
        "不读 `clipId` 的形态必须**接受但忽略**（不解析 ULID）: {response}"
    );
    println!(
        "### 逐字：addTrack 给非法 clipId ⇒ status = {}（接受但忽略）",
        response["status"]
    );
}

/// `tools/list` 里 `yeban_query_project.inputSchema.properties.fields`。
fn fields_schema() -> Value {
    let catalog = tools::catalog();
    catalog["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|tool| tool["name"] == "yeban_query_project")
        .expect("yeban_query_project 必须在 tools/list 里")["inputSchema"]["properties"]["fields"]
        .clone()
}

/// 判据 ③（§2.3）：`fields` 的枚举**可读**，且**逐字等于**解析器的白名单。
#[test]
fn the_published_fields_enum_is_the_parser_whitelist() {
    let fields = fields_schema();
    let published = fields["items"]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("`fields.items.enum` 必须广播出来: {fields}"));
    assert_eq!(
        published.len(),
        SELECTORS.len(),
        "广播的枚举成员个数必须等于解析器白名单的条目数"
    );
    assert_eq!(
        json!(
            published
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
        ),
        json!(SELECTORS),
        "枚举必须与 `SELECTORS` 逐项同序（⛔ 不是手抄的第二份清单）"
    );
    assert_eq!(fields["items"]["type"], "string");
    println!(
        "### 逐字：fields.items.type = {}, enum 成员数 = {}",
        fields["items"]["type"],
        published.len()
    );
    println!("### 逐字：fields.items.enum = {}", json!(SELECTORS));
}

/// 判据 ③ 的反面：**我写了个不在枚举里的字段名 ⇒ 响亮拒绝**。
#[test]
fn an_unknown_field_selector_is_refused_loudly() {
    let (mut dispatcher, auth, _project) = dispatcher();
    let response = call(
        &mut dispatcher,
        &auth,
        "yeban_query_project",
        json!({"fields": ["notAField"]}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(
        response["error"]["code"], "INVALID_FIELD_SELECTOR",
        "{response}"
    );
    assert_eq!(response["error"]["data"]["unknown"], "notAField");
    let available = response["error"]["data"]["availableSelectors"]
        .as_array()
        .expect("必须回传白名单");
    assert_eq!(
        available.len(),
        SELECTORS.len(),
        "拒绝报文里的白名单必须与枚举同一份真相"
    );
    println!(
        "### 逐字：fields=[notAField] ⇒ {} / availableSelectors = {} 项",
        response["error"]["code"],
        available.len()
    );
}

/// 判据 ④（§2.4）：同一会话里 `session.undoCursor` 与 `yeban_undo` 的
/// `before.undoable` **同一个数**（前后两对读数：1/1 与 0/0）。
#[test]
fn the_undo_cursor_and_the_undo_readback_are_the_same_number() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);

    // 先造一条**已合并**的提交（一次 `add` 音符）。
    let proposal = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 60, "durationTicks": 480}}],
        }),
    );
    assert_eq!(proposal["status"], "success", "{proposal}");
    let proposal_id = proposal["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "trap-177 merge"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    // 读数 ①：合并之后（1 条可撤销）。
    let state = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({}),
    );
    let cursor_one = state["data"]["session"]["undoCursor"]
        .as_u64()
        .expect("undoCursor");
    let undo_read = call(&mut dispatcher, &auth, "yeban_undo", json!({"steps": 1}));
    let undoable_one = undo_read["data"]["before"]["undoable"]
        .as_u64()
        .expect("before.undoable");
    assert_eq!(
        cursor_one, undoable_one,
        "两处读数必须相等（undoCursor={cursor_one}, before.undoable={undoable_one}）"
    );
    assert_eq!(cursor_one, 1, "一条提交之后两处都必须是 1");
    println!("### 逐字：session.undoCursor = {cursor_one} / before.undoable = {undoable_one}");

    // 上面那次是**真的**撤销（不是 dryRun），断言它确实生效。
    assert_eq!(undo_read["status"], "success", "{undo_read}");
    // 撤完之后再读：两处都必须是 0。
    let state = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({}),
    );
    let cursor_zero = state["data"]["session"]["undoCursor"]
        .as_u64()
        .expect("undoCursor");
    let undo_read = call(
        &mut dispatcher,
        &auth,
        "yeban_undo",
        json!({"steps": 1, "dryRun": true}),
    );
    // 撤到底之后 `yeban_undo` 报 `INDEX_OUT_OF_BOUNDS`（没有可撤的历史），
    // 它的 `data.undoable` 是**同一个口径**的 0 —— 两处读数仍然一致。
    let undoable_zero = undo_read["error"]["data"]["undoable"]
        .as_u64()
        .expect("撤到底时 data.undoable");
    assert_eq!(
        cursor_zero, undoable_zero,
        "撤到底之后两处读数也必须相等（undoCursor={cursor_zero}, undo.data.undoable={undoable_zero}）"
    );
    assert_eq!(cursor_zero, 0, "撤到底之后两处都必须是 0");
    println!(
        "### 逐字：撤一步后 session.undoCursor = {cursor_zero} / undo.data.undoable = {undoable_zero}"
    );
}

/// 判据 ⑤（§2.5）：造一个提案 → 用坏 `proposalId` 调 merge ⇒ `data.knownProposals`
/// 里出现**真 id**（注释承诺过的那件事）。
#[test]
fn proposal_not_found_lists_the_known_proposals() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let proposal = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{"kind": "add", "note": {"startTick": 0, "pitch": 62, "durationTicks": 480}}],
        }),
    );
    let real_id = proposal["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let bogus = "01J8ZQ0000000000000000C177";
    let response = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": bogus, "commitMessage": "trap-177 bad merge"}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(
        response["error"]["code"], "PROPOSAL_NOT_FOUND",
        "{response}"
    );
    assert_eq!(response["error"]["data"]["reason"], "proposalNotFound");
    assert_eq!(response["error"]["data"]["proposalId"], bogus);
    let known = response["error"]["data"]["knownProposals"]
        .as_array()
        .expect("`knownProposals` 必须在 data 里");
    let hits = known
        .iter()
        .filter(|record| record["proposalId"] == real_id.as_str())
        .count();
    assert_eq!(
        hits, 1,
        "真提案 id 必须在 knownProposals 里恰好出现一次: {response}"
    );
    assert_eq!(response["error"]["data"]["knownCount"], json!(known.len()));
    assert_eq!(
        response["error"]["data"]["listTool"],
        "yeban_list_proposals"
    );
    println!(
        "### 逐字：坏 proposalId ⇒ {} / knownProposals = {} 条 / 真 id 命中 = {}",
        response["error"]["code"],
        known.len(),
        hits
    );
    println!(
        "### 逐字：knownProposals[0].proposalId = {}",
        known[0]["proposalId"]
    );
}

/// 判据 ⑥（§2.1 的广播面）：`ops.items` 的**每个**分支都带 `x-envelope.clipId`，
/// 且"必需"集合**逐字等于** `ENVELOPE_CLIP_KINDS`（不是手抄）。
#[test]
fn every_ops_branch_publishes_the_envelope_clip_rule() {
    let schema = yeban_mcp::op_schema::ops_items();
    let branches = schema["oneOf"].as_array().expect("oneOf");
    assert_eq!(
        branches.len(),
        OP_KINDS.len(),
        "分支数必须等于 `OP_KINDS` 的条目数"
    );
    let mut published_required: Vec<String> = Vec::new();
    for branch in branches {
        let kind = branch["properties"]["kind"]["const"]
            .as_str()
            .expect("每个分支都要有 `kind.const`")
            .to_owned();
        let marker = branch["x-envelope"]["clipId"]
            .as_str()
            .unwrap_or_else(|| panic!("`{kind}` 分支缺 `x-envelope.clipId`: {branch}"));
        let required = marker == "required";
        assert_eq!(
            required,
            ENVELOPE_CLIP_KINDS.contains(&kind.as_str()),
            "`{kind}` 的 `x-envelope.clipId` 必须与 ENVELOPE_CLIP_KINDS 一致"
        );
        let description = branch["description"].as_str().expect("分支描述");
        assert!(
            description.contains("顶层 `clipId`"),
            "`{kind}` 的分支描述必须逐 kind 写明 `clipId` 口径"
        );
        if required {
            published_required.push(kind);
        }
    }
    assert_eq!(
        json!(published_required),
        json!(ENVELOPE_CLIP_KINDS),
        "广播出去的必需集合必须逐项等于 ENVELOPE_CLIP_KINDS"
    );
    assert_eq!(
        schema["x-envelope"]["clipIdRequiredKinds"],
        json!(ENVELOPE_CLIP_KINDS)
    );
    let ignored = schema["x-envelope"]["clipIdIgnoredKinds"]
        .as_array()
        .expect("clipIdIgnoredKinds");
    assert_eq!(
        ignored.len(),
        OP_KINDS.len() - ENVELOPE_CLIP_KINDS.len(),
        "必需 + 忽略 必须覆盖 `OP_KINDS` 全集"
    );
    println!(
        "### 逐字：分支数 = {} / 必需 = {} / 忽略 = {} / ENVELOPE_CLIP_KINDS = {}",
        branches.len(),
        published_required.len(),
        ignored.len(),
        json!(ENVELOPE_CLIP_KINDS)
    );
}

/// 判据 ⑥ 的 `tools/list` 面：`clipId` 已从 `required` 里移除，且仍在 `properties` 里
/// （向后兼容：旧调用传它不会撞 `additionalProperties: false`）。
#[test]
fn the_published_signature_makes_clip_id_optional_but_still_accepted() {
    let catalog = tools::catalog();
    let schema = catalog["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|tool| tool["name"] == "yeban_edit_notes")
        .expect("yeban_edit_notes")["inputSchema"]
        .clone();
    let required = schema["required"].as_array().expect("required");
    assert!(
        !required.iter().any(|name| name == "clipId"),
        "`clipId` 必须从顶层 required 里移除: {schema}"
    );
    assert!(
        required.iter().any(|name| name == "trackId"),
        "`trackId` 仍是顶层必填（本票只改 `clipId`）: {schema}"
    );
    assert!(
        schema["properties"]["clipId"]["type"] == "string",
        "`clipId` 仍必须在 properties 里（旧调用可继续传）: {schema}"
    );
    assert_eq!(
        schema["properties"]["ops"]["items"]["x-envelope"]["clipIdRequiredKinds"],
        json!(ENVELOPE_CLIP_KINDS),
        "`ops.items` 根上的 `x-envelope` 必须广播必需集合: {schema}"
    );
    println!(
        "### 逐字：required = {} / properties.clipId.type = {}",
        json!(required),
        schema["properties"]["clipId"]["type"]
    );
}
