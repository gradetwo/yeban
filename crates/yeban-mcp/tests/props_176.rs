//! **#176 的判据**：`#174` 验收里剩下的两个高价值坑 ——
//! **② 提案不可发现/不可恢复**（新增 `yeban_list_proposals`）与
//! **⑥ 21 个工具全无 `outputSchema`**（五个主力工具各声明一份，并被真实响应反校）。
//!
//! ## 这个文件量什么、单位是什么
//!
//! | 判据 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | [`list_proposals_is_discoverable_and_the_counts_move_with_it`] | `tools/list` 里的工具数 / `TOOL_COUNT` / 样本 `registry.count` / 契约 enum | **工具个数**（23） |
//! | [`a_pending_proposal_is_listed_and_disappears_after_merge`] | `yeban_list_proposals` 的 `openCount` | **提案条数**（1 → 0） |
//! | [`the_empty_state_is_an_empty_array_plus_one_sentence`] | 空态响应 | 1 个空数组 + 1 句逐字 `note` |
//! | [`five_main_tools_publish_an_output_schema_their_real_responses_satisfy`] | 5 个主力工具的真实 `data` | **5 份 schema**，每份都被自己的响应反校 |
//! | [`the_validator_can_go_red_and_the_schema_is_not_idealised`] | 校验器 | 3 种注入（少 required / 错类型 / 声明字段缺席）各 1 条 |
//! | [`the_58_published_spellings_are_still_all_there`] | `insertDevice.device.params.items` | **拼写个数**（58） |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test props_176 -- --nocapture`

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::{output_schema, samples, tools};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, MidiNote, RoutingEdge, RoutingKind,
    SampleRate, TrackKind, TrackV3, YebanProjectV1,
};

/// 注入规范样本工程的分发器（与 `tests/trap_175.rs` 同一形状）。
fn dispatcher() -> (Dispatcher, String, YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-props-176.yeban"),
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
        "id": "props-176",
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

/// 注入**渲染夹具**工程的分发器（1 秒 / 一个音符）。
fn render_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-props-176-render.yeban"),
            render_project(),
            false,
        )
        .expect("注入渲染夹具工程");
    (dispatcher, authorization)
}

/// 工具签名要求的顶层 `(trackId, clipId)`（一条带摆放的音轨）。
fn track_and_clip(project: &YebanProjectV1) -> (EntityId, EntityId) {
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

/// 造一个**最小**的提案（`setTempo`：不需要额外材料，只产生一条 `Op`）。
fn make_proposal(dispatcher: &mut Dispatcher, auth: &str, project: &YebanProjectV1) -> Value {
    let (track_id, clip_id) = track_and_clip(project);
    let response = call(
        dispatcher,
        auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{"kind": "setTempo", "bpm": 121.0}],
        }),
    );
    assert_eq!(response["status"], "success", "{response}");
    assert_eq!(response["data"]["pendingMerge"], true, "{response}");
    response
}

/// 判据 ①：新工具**可发现**，且四处机械读数同步（`tools/list` / `TOOL_COUNT` /
/// 扩展清单 / 样本 registry）。`service-info` 这条通道**不存在** ⇒ 用真实存在的
/// 两个读面（`tools/list` 长度与 `samples::registry_sample().count`）。
#[test]
fn list_proposals_is_discoverable_and_the_counts_move_with_it() {
    let catalog = tools::catalog();
    let descriptors = catalog["tools"].as_array().expect("tools 必须是数组");
    let names: Vec<&str> = descriptors
        .iter()
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect();
    assert!(
        names.contains(&"yeban_list_proposals"),
        "`tools/list` 里必须能找到 yeban_list_proposals: {names:?}"
    );
    let descriptor = descriptors
        .iter()
        .find(|tool| tool["name"] == "yeban_list_proposals")
        .expect("新工具的描述符");
    assert_eq!(
        descriptor["annotations"]["specId"], "MCP-TOOL-EXT-LIST-PROPOSALS",
        "扩展 ID 不得伪装成规范编号（MCP-TOOL-011）"
    );
    assert_eq!(
        descriptor["annotations"]["sideEffecting"], false,
        "只读工具"
    );
    assert_eq!(
        descriptor["annotations"]["requiredScope"], "app:admin",
        "{descriptor}"
    );
    for name in [tools::DRY_RUN_PARAM, tools::IDEMPOTENCY_KEY_PARAM, "kind"] {
        assert!(
            descriptor["inputSchema"]["properties"][name].is_object(),
            "inputSchema 必须声明 `{name}`: {}",
            descriptor["inputSchema"]
        );
    }
    // 不在 #176 的五个主力工具里 ⇒ 不广播 outputSchema（⛔ 不拿空对象冒充）。
    assert!(
        descriptor.get("outputSchema").is_none(),
        "list_proposals 不在 outputSchema 的五个工具里: {descriptor}"
    );
    // 描述里必须如实说"不跨进程"。
    let description = descriptor["description"].as_str().expect("description");
    assert!(
        description.contains("进程重启后这里为空"),
        "工具描述必须写明提案记录不跨进程: {description}"
    );

    let registry = samples::registry_sample();
    println!(
        "### 计数读数\nTOOL_COUNT={} | tools/list={} | DOCUMENTED={} | EXTENSION={} | ExtensionToolArguments.$defs={} | registry.count={} | registry.tools={}",
        tools::TOOL_COUNT,
        descriptors.len(),
        tools::DOCUMENTED_TOOL_COUNT,
        tools::EXTENSION_TOOL_COUNT,
        tools::EXTENSION_NAMES.len(),
        registry["count"],
        registry["tools"].as_array().map_or(0, Vec::len),
    );
    assert_eq!(descriptors.len(), tools::TOOL_COUNT, "tools/list 长度");
    assert_eq!(
        tools::TOOL_COUNT,
        tools::DOCUMENTED_TOOL_COUNT + tools::EXTENSION_TOOL_COUNT
    );
    assert_eq!(
        tools::TOOL_COUNT,
        24,
        "10 个规范工具 + 14 个扩展 = 24（#176 新增 1 条、#290 新增场景启动 1 条、#345 新增 MIDI 导入 1 条）"
    );
    assert_eq!(
        registry["count"],
        tools::TOOL_COUNT,
        "样本 registry 的 count"
    );
    assert_eq!(
        registry["tools"].as_array().expect("registry.tools").len(),
        tools::TOOL_COUNT
    );
    assert_eq!(
        tools::EXTENSION_NAMES.last().copied(),
        Some("yeban_launch_scene"),
        "扩展清单顺序 = 注册顺序（新工具追加在末尾）"
    );

    // 契约 enum 也是 23 个名字，且与注册表逐字相同（顺序一致）。
    let contract: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("schemas")
                .join("mcp-tools.schema.json"),
        )
        .expect("读契约"),
    )
    .expect("契约是合法 JSON");
    let enum_names: Vec<&str> = contract["definitions"]["ToolCall"]["properties"]["name"]["enum"]
        .as_array()
        .expect("name.enum")
        .iter()
        .map(|name| name.as_str().expect("字符串"))
        .collect();
    println!(
        "### 契约 enum 尾部\n{:?}",
        &enum_names[enum_names.len().saturating_sub(3)..]
    );
    assert_eq!(enum_names.len(), 24, "契约 enum 的工具个数");
    assert_eq!(enum_names, tools::tool_names(), "enum 顺序 = 注册表顺序");
}

/// 判据 ②③：造一个**不 merge** 的提案 ⇒ 列表含它（`proposalId` 逐字）；
/// merge 之后 ⇒ 列表变空。两个读数都打印出来。
#[test]
fn a_pending_proposal_is_listed_and_disappears_after_merge() {
    let (mut dispatcher, auth, project) = dispatcher();

    let before = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    assert_eq!(before["status"], "success", "{before}");
    let before_open = before["data"]["openCount"].as_u64().expect("openCount");
    assert_eq!(before_open, 0, "初始没有提案: {before}");

    let created = make_proposal(&mut dispatcher, &auth, &project);
    let proposal_id = created["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();

    let listed = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    println!(
        "### merge 前\n{}",
        serde_json::to_string_pretty(&listed["data"]).expect("pretty")
    );
    assert_eq!(listed["status"], "success", "{listed}");
    assert_eq!(listed["data"]["openCount"], 1, "{listed}");
    assert_eq!(listed["data"]["totalCount"], 1, "{listed}");
    let proposals = listed["data"]["proposals"]
        .as_array()
        .expect("proposals 必须是数组");
    assert_eq!(proposals.len(), 1, "{listed}");
    let entry = &proposals[0];
    assert_eq!(
        entry["proposalId"], proposal_id,
        "列表里的 proposalId 必须与创建响应**逐字**相同"
    );
    assert_eq!(entry["status"], "open", "{entry}");
    assert_eq!(entry["kind"], "notes", "{entry}");
    assert_eq!(entry["opCount"], 1, "{entry}");
    assert_eq!(entry["opKinds"], json!(["SetTempo"]), "{entry}");
    assert_eq!(entry["index"], 1, "1 起始序号: {entry}");
    assert!(
        !entry["title"].as_str().expect("title").is_empty(),
        "{entry}"
    );
    assert_eq!(
        entry["branch"],
        format!("ai/proposal-{proposal_id}"),
        "分支名由 proposalId 派生: {entry}"
    );
    for field in ["baseCommit", "headCommit", "createdAt"] {
        assert!(!entry[field].is_null(), "`{field}` 必须有值: {entry}");
    }
    assert_eq!(entry["createdAt"], 1_760_000_000_000u64, "{entry}");

    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "#176 判据"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    assert_eq!(merged["data"]["merged"], true, "{merged}");

    let after = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    println!(
        "### merge 后\n{}",
        serde_json::to_string_pretty(&after["data"]).expect("pretty")
    );
    assert_eq!(after["status"], "success", "{after}");
    assert_eq!(
        after["data"]["openCount"], 0,
        "合并之后列表必须变空: {after}"
    );
    assert_eq!(after["data"]["proposals"], json!([]), "{after}");
    assert_eq!(
        after["data"]["totalCount"], 1,
        "记录**不删除**（可追溯），只是不再待合并: {after}"
    );
    println!(
        "### openCount 读数\nmerge 前 = {before_open} → 建提案后 = {} → merge 后 = {}",
        listed["data"]["openCount"], after["data"]["openCount"]
    );
}

/// 判据 ④：空态是**空数组 + 一句逐字说明**（⛔ 不报错）。
#[test]
fn the_empty_state_is_an_empty_array_plus_one_sentence() {
    let (mut dispatcher, auth, _project) = dispatcher();
    let response = call(&mut dispatcher, &auth, "yeban_list_proposals", json!({}));
    println!(
        "### 空态（逐字）\n{}",
        serde_json::to_string_pretty(&response).expect("pretty")
    );
    assert_eq!(response["status"], "success", "空列表不是错误: {response}");
    assert_eq!(response["data"]["proposals"], json!([]), "{response}");
    assert_eq!(response["data"]["openCount"], 0, "{response}");
    assert_eq!(response["data"]["totalCount"], 0, "{response}");
    assert_eq!(
        response["data"]["note"], "当前没有未合并的提案。",
        "空态说明必须逐字: {response}"
    );
    assert_eq!(
        response["data"]["mergeTool"], "yeban_merge_proposal",
        "{response}"
    );

    // `kind` 过滤：非法取值响亮失败（⛔ 不静默返回全部）。
    let refused = call(
        &mut dispatcher,
        &auth,
        "yeban_list_proposals",
        json!({"kind": "notez"}),
    );
    assert_eq!(refused["status"], "error", "{refused}");
    assert_eq!(
        refused["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{refused}"
    );
    // 合法过滤的空态也有自己的那句话（逐字不同）。
    let filtered = call(
        &mut dispatcher,
        &auth,
        "yeban_list_proposals",
        json!({"kind": "section"}),
    );
    assert_eq!(filtered["status"], "success", "{filtered}");
    assert_eq!(
        filtered["data"]["note"], "当前没有未合并的 `section` 类提案。",
        "{filtered}"
    );
}

/// 判据 ⑤⑥：五个主力工具的 `outputSchema` **与真实响应一致** ——
/// 校验器核 `required` ＋ 顶层 `type` ＋ 声明的字段名/类型。
///
/// 真实响应全部来自 `tools/call`（不是手写夹具）；每个工具打印一条读数。
#[test]
fn five_main_tools_publish_an_output_schema_their_real_responses_satisfy() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, _clip_id) = track_and_clip(&project);

    // ① `yeban_edit_notes`（提案响应）
    let edit_notes = make_proposal(&mut dispatcher, &auth, &project);
    // ② `yeban_merge_proposal`（正常合并 → 全字段形状）
    let proposal_id = edit_notes["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merge = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "#176 outputSchema"}),
    );
    assert_eq!(merge["status"], "success", "{merge}");
    // ③ `yeban_query_engine_state`（点名一条音轨 ⇒ `sends` 也在）
    let engine_state = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({"trackId": track_id.to_canonical_string()}),
    );
    assert_eq!(engine_state["status"], "success", "{engine_state}");
    // ④ `yeban_render_master`（真的渲染一份 1 秒母带；独立夹具工程）
    let scratch = Scratch::new("output-schema");
    let (mut render_dispatcher, render_auth) = render_dispatcher();
    let rendered = call(
        &mut render_dispatcher,
        &render_auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48_000,
            "path": scratch.join("master.wav").display().to_string(),
        }),
    );
    assert_eq!(rendered["status"], "success", "{rendered}");
    // ⑤ `yeban_undo`（撤销刚才那次合并）
    let undo = call(&mut dispatcher, &auth, "yeban_undo", json!({"steps": 1}));
    assert_eq!(undo["status"], "success", "{undo}");

    let responses: [(&str, &Value, &[&str]); 5] = [
        (
            "yeban_edit_notes",
            &edit_notes,
            &["pendingMerge", "proposal", "warnings"],
        ),
        (
            "yeban_merge_proposal",
            &merge,
            &["appliedOps", "commit", "proposal"],
        ),
        (
            "yeban_query_engine_state",
            &engine_state,
            &["engine", "session", "stateSources", "track"],
        ),
        (
            "yeban_render_master",
            &rendered,
            &["clippedSamples", "truePeakDbfs", "peak", "sha256"],
        ),
        (
            "yeban_undo",
            &undo,
            &["before", "after", "undone", "cursorPersisted"],
        ),
    ];
    for (name, response, spot_fields) in responses {
        assert_eq!(response["status"], "success", "{name}: {response}");
        let schema = tools::catalog()["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("`{name}` 必须在 tools/list 里"))["outputSchema"]
            .clone();
        assert!(
            !schema.is_null(),
            "`{name}` 必须在 `tools/list` 里广播 outputSchema"
        );
        let data = &response["data"];
        output_schema::validate_complete(&schema, data).unwrap_or_else(|violation| {
            panic!("`{name}` 的真实响应违反自己的 schema: {violation}\n{data}")
        });
        // 读数：声明的字段个数 / required 个数 / 抽查字段的逐字值。
        let required = schema["required"].as_array().map_or(0, Vec::len);
        println!(
            "### `{name}`\n  declared={} required={} spot={}",
            output_schema::declared_fields(&schema).len(),
            required,
            serde_json::to_string(
                &spot_fields
                    .iter()
                    .map(|field| (
                        (*field).to_owned(),
                        data.get(*field).cloned().unwrap_or(Value::Null)
                    ))
                    .collect::<BTreeMap<_, _>>()
            )
            .expect("spot"),
        );
        for field in spot_fields {
            assert!(
                data.get(*field).is_some(),
                "`{name}` 的响应必须含 `{field}`"
            );
        }
    }

    // 两个逐字读数（判据 ⑦ 的例子）：提案语义与削波读数。
    println!(
        "### 逐字：edit_notes.pendingMerge = {}\n### 逐字：render_master.clippedSamples = {} / truePeakDbfs = {}",
        edit_notes["data"]["pendingMerge"],
        rendered["data"]["clippedSamples"],
        rendered["data"]["truePeakDbfs"]
    );
    assert_eq!(edit_notes["data"]["pendingMerge"], true);
    assert!(
        rendered["data"]["clippedSamples"].is_number(),
        "削波样本数必须是读数: {rendered}"
    );

    // `alreadyMerged` 的**短形状**也必须满足同一份 schema 的 `required`
    // （这是"两种成功形状"那条口径的可执行证据）。
    let again = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "重复合并"}),
    );
    println!(
        "### alreadyMerged 的短形状（逐字）\n{}",
        serde_json::to_string_pretty(&again).expect("pretty")
    );
    assert_eq!(again["status"], "success", "{again}");
    assert_eq!(again["data"]["alreadyMerged"], true, "{again}");
    assert_eq!(again["data"]["appliedOps"], 0, "{again}");
    assert!(
        again["data"].get("commit").is_none(),
        "短形状**没有** commit: {again}"
    );
    let merge_schema = output_schema::for_tool("yeban_merge_proposal").expect("schema");
    output_schema::validate(&merge_schema, &again["data"])
        .expect("短形状必须满足同一份 schema 的 required");
}

/// 判据 ⑥ 的反面：校验器**能变红**，且 schema 不是"理想形状"。
#[test]
fn the_validator_can_go_red_and_the_schema_is_not_idealised() {
    let schema = output_schema::for_tool("yeban_undo").expect("schema");
    let good = json!({
        "undone": true,
        "requestedSteps": 1,
        "steps": 1,
        "undoneTotal": 1,
        "opKinds": [],
        "branch": "main",
        "projectDigest": "digest",
        "cursorPersisted": false,
        "before": {
            "branch": "main", "head": null, "commitCount": 0, "branchCount": 1,
            "undone": 0, "undoable": 0, "canUndo": false, "canRedo": false
        },
        "after": {
            "branch": "main", "head": null, "commitCount": 0, "branchCount": 1,
            "undone": 1, "undoable": 1, "canUndo": true, "canRedo": false
        }
    });
    output_schema::validate_complete(&schema, &good).expect("合法响应必须通过");

    let mut missing = good.clone();
    missing
        .as_object_mut()
        .expect("对象")
        .remove("cursorPersisted");
    let violation = output_schema::validate(&schema, &missing).expect_err("少 required 必须红");
    println!("### 注入①：少 required\n{violation}");
    assert_eq!(violation.path, "$.cursorPersisted");

    let mut wrong = good.clone();
    wrong["steps"] = Value::from("one");
    let violation = output_schema::validate(&schema, &wrong).expect_err("类型不对必须红");
    println!("### 注入②：类型不对\n{violation}");
    assert_eq!(violation.path, "$.steps");

    // ③ 声明的字段缺席：用**另一种成功形状**的那份 schema（`sends` 只在点名音轨时出现，
    //    因此它**不在** required 里）—— `validate` 放行，`validate_complete` 必须红。
    let engine_schema = output_schema::for_tool("yeban_query_engine_state").expect("schema");
    let without_sends = json!({
        "session": {},
        "engine": {},
        "track": null,
        "stateSources": {}
    });
    output_schema::validate(&engine_schema, &without_sends).expect("sends 不是 required");
    let violation = output_schema::validate_complete(&engine_schema, &without_sends)
        .expect_err("声明了就必须出现");
    println!("### 注入③：声明字段缺席\n{violation}");
    assert_eq!(violation.path, "$.sends");
}

/// 判据 ⑦：`#156` 的 **58 个拼写**（32 规范 + 26 别名）一个不少（两侧）。
#[test]
fn the_58_published_spellings_are_still_all_there() {
    use yeban_mcp::{effect_params, op_schema};
    let names = effect_params::recognised_names();
    assert_eq!(names.len(), 58, "32 规范 + 26 别名");
    assert_eq!(
        effect_params::EFFECT_GROUPS
            .iter()
            .map(|group| group.params.len())
            .sum::<usize>(),
        32
    );
    let items = op_schema::ops_items();
    let insert = items["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .find(|branch| branch["title"] == "insertDevice")
        .expect("insertDevice 分支");
    let branches = insert["properties"]["device"]["properties"]["params"]["items"]["oneOf"]
        .as_array()
        .expect("device.params.items.oneOf");
    assert_eq!(branches.len(), 32);
    let published: usize = branches
        .iter()
        .map(|branch| {
            branch["properties"]["name"]["enum"]
                .as_array()
                .expect("name.enum")
                .len()
        })
        .sum();
    assert_eq!(published, 58, "公告面的拼写个数");
    assert!(effect_params::recognise("eq_low_gain").is_some());
    assert!(effect_params::recognise("eq_low_gian").is_none());
    assert_eq!(
        effect_params::similar_spellings("eq_low_gian")
            .first()
            .map(|item| item.canonical),
        Some("eq_low_gain")
    );
    assert_eq!(effect_params::TYPO_MAX_DISTANCE, 2);
    println!(
        "### 拼写读数\nrecognised={} published={published}",
        names.len()
    );
}

// ---------------------------------------------------------------------------
// 渲染夹具（与 `tests/render_master.rs` 同构的最小工程：1 秒 / 一个音符）
// ---------------------------------------------------------------------------

fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 一个临时目录，`Drop` 时删除（绝不污染仓库）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("yeban-mcp-props-176-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// "Master + 一条 MIDI 轨 → Master"的工程（判断渲染响应形状只需要它）。
fn render_project() -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let note = id(30);
    let edge = id(40);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "props-176 render fixture".to_owned(),
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 1_920,
            loop_config: yeban_model::LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(note, MidiNote::new(note, 0, 69, 1_920));
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(lead, track);
    project.routing_graph.nodes = vec![lead, master];
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: lead,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    project
}
