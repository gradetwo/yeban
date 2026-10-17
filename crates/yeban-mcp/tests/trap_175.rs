//! **#175 的判据**：#174 外部 agent 试跑抓到的两个"以为写成了"的坑（坑⑤ 拼错参数、
//! 坑① 提案语义）＋ `render_master` 削波读数（坑⑧，判据住在 `tests/render_master.rs`）。
//!
//! ## 这个文件量什么、单位是什么
//!
//! | 判据 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | [`a_misspelled_effect_param_is_refused_with_the_nearest_name`] | 一次 `insertDevice`（`eq_low_gian`）的**带内错误** | 1 个错误码 + 1 条建议（最近邻） |
//! | [`the_correct_spelling_is_accepted_and_reads_back_clean`] | `eq_low_gain` 合并后 `inserts[].unrecognisedParams` | **参数名个数**（0） |
//! | [`a_deliberately_unrecognised_name_is_accepted_but_warned`] | `reverb_mix`（#156 刻意不认） | 1 条 `warnings[]` + 读回 1 个未识别名 |
//! | [`the_eight_fifty_eight_published_spellings_are_still_all_there`] | `insertDevice.device.params.items` 的 `name.enum` | **拼写个数**（32 规范 + 26 别名 = 58） |
//! | [`edit_notes_description_says_the_proposal_must_be_merged`] | `tools/list` 的 `yeban_edit_notes.description` | 1 句逐字 + 响应里 `pendingMerge` |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test trap_175 -- --nocapture`

use std::str::FromStr as _;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::{effect_params, op_schema, tools};
use yeban_model::EntityId;

/// 注入规范样本工程的分发器（与 `tests/effect_contract.rs` 同一形状）。
fn dispatcher() -> (Dispatcher, String, yeban_model::YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::path::PathBuf::from("/tmp/yeban-trap-175.yeban"),
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
        "id": "trap-175",
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

/// 工具签名要求的顶层 `(trackId, clipId)`（一条带摆放的音轨）。
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

fn ulid(text: &str) -> EntityId {
    EntityId::from_str(text).expect("夹具 ULID 必须合法")
}

/// 一台 `InternalEffect` 设备（`params` 由调用方给）。
fn effect_device(device_id: EntityId, params: Value) -> Value {
    json!({
        "deviceId": device_id.to_canonical_string(),
        "name": "EQ",
        "kind": "InternalEffect",
        "bypassed": false,
        "latencySamples": 0,
        "params": params,
    })
}

/// 判据 ①：`eq_low_gian` ⇒ **响亮失败**，报文与 `data` 逐字给出最近邻 `eq_low_gain`。
///
/// #174 实测的旧行为是 `status:"success"` + `inserts[0].unrecognisedParams:["eq_low_gian"]`
/// —— 外部 agent **以为写成了**。现在它必须在**写入时**红，且不能留下一个提案。
#[test]
fn a_misspelled_effect_param_is_refused_with_the_nearest_name() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let proposals_before = dispatcher.domain().proposal_count();
    let commits_before = dispatcher.domain().commit_count();

    let response = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{
                "kind": "insertDevice",
                "trackId": track_id.to_canonical_string(),
                "device": effect_device(
                    ulid("01ARZ3NDEKTSV4RRFFQ69G5FA1"),
                    json!([{"name": "eq_low_gian", "value": 9.0, "unit": "dB"}]),
                ),
            }],
        }),
    );
    println!(
        "### 拼错参数名的响应（逐字）\n{}",
        serde_json::to_string_pretty(&response).expect("pretty")
    );

    assert_eq!(
        response["status"], "error",
        "拼错的参数名必须响亮失败（⛔ 不许 status:\"success\"）: {response}"
    );
    assert_eq!(
        response["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{response}"
    );
    let message = response["error"]["message"]
        .as_str()
        .expect("错误报文必须是字符串");
    assert!(
        message.contains("eq_low_gian"),
        "报文必须点名调用方写的那个词: {message}"
    );
    assert!(
        message.contains("eq_low_gain"),
        "报文必须给出**可用的名字**（最近邻）: {message}"
    );
    let data = &response["error"]["data"];
    assert_eq!(data["reason"], "unknownEffectParam", "{data}");
    assert_eq!(
        data["unrecognisedParams"],
        json!(["eq_low_gian"]),
        "被拒的拼写必须逐字回传: {data}"
    );
    let suggestions = data["suggestions"]
        .as_array()
        .expect("suggestions 必须是数组");
    assert!(!suggestions.is_empty(), "必须给最近邻: {data}");
    assert!(
        suggestions.iter().any(|suggestion| {
            suggestion["requested"] == "eq_low_gian"
                && suggestion["canonicalName"] == "eq_low_gain"
                && suggestion["effect"] == "eq"
                && suggestion["distance"].as_u64() == Some(2)
        }),
        "建议必须是 `eq_low_gian` ⇒ `eq_low_gain`（编辑距离 2）: {suggestions:?}"
    );

    // ⛔ "不静默"的另一半：没有提案被建、工程没变、提交数不变。
    assert_eq!(
        dispatcher.domain().proposal_count(),
        proposals_before,
        "拼错的调用不得留下一个提案"
    );
    assert_eq!(dispatcher.domain().commit_count(), commits_before);
}

/// 判据 ②：正确的 `eq_low_gain` 不受影响 —— 成功、`pendingMerge` 可见、合并后读回干净。
#[test]
fn the_correct_spelling_is_accepted_and_reads_back_clean() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let device_id = ulid("01ARZ3NDEKTSV4RRFFQ69G5FA2");

    let written = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{
                "kind": "insertDevice",
                "trackId": track_id.to_canonical_string(),
                "device": effect_device(
                    device_id,
                    json!([
                        {"name": "eq_low_gain", "value": 6.0, "unit": "dB"},
                        {"name": "eq_low_freq", "value": 120.0, "unit": "Hz"}
                    ]),
                ),
            }],
        }),
    );
    println!(
        "### 正确拼写的响应（逐字）\n{}",
        serde_json::to_string_pretty(&written).expect("pretty")
    );
    assert_eq!(written["status"], "success", "{written}");
    // ⭐ #175（#174 坑①）：提案语义是**显式字段**，⛔ 不许靠 `projectUnchanged` 反推。
    assert_eq!(written["data"]["pendingMerge"], true, "{written}");
    assert_eq!(
        written["data"]["mergeTool"], "yeban_merge_proposal",
        "{written}"
    );
    assert_eq!(written["data"]["projectUnchanged"], true, "{written}");
    assert_eq!(written["data"]["warnings"], json!([]), "正确名字不该有警告");

    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "EQ（#175 判据）"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");

    let queried = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({"trackId": track_id.to_canonical_string()}),
    );
    let inserts = queried["data"]["track"]["inserts"]
        .as_array()
        .expect("inserts");
    let inserted = inserts
        .iter()
        .find(|insert| insert["deviceId"] == device_id.to_canonical_string())
        .expect("新设备必须出现在 inserts[] 里");
    assert_eq!(
        inserted["recognisedParams"],
        json!(["eq_low_gain", "eq_low_freq"]),
        "{inserted}"
    );
    assert_eq!(
        inserted["unrecognisedParams"],
        json!([]),
        "正确的拼写不得留下任何未识别参数: {inserted}"
    );
    assert_eq!(inserted["effects"], json!(["eq"]), "{inserted}");
}

/// 判据 ③：`#156` 刻意不认的名字**照旧接受**（距离 3 > 阈值 2），但写入侧**不静默**：
/// 响应 `warnings[]` 逐条报出，合并后读回 `unrecognisedParams` 也逐字可见。
#[test]
fn a_deliberately_unrecognised_name_is_accepted_but_warned() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let device_id = ulid("01ARZ3NDEKTSV4RRFFQ69G5FA3");

    let written = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{
                "kind": "insertDevice",
                "trackId": track_id.to_canonical_string(),
                "device": effect_device(
                    device_id,
                    json!([
                        {"name": "reverb_size", "value": 0.7},
                        {"name": "reverb_mix", "value": 0.5}
                    ]),
                ),
            }],
        }),
    );
    assert_eq!(
        written["status"], "success",
        "`reverb_mix` 与最近的已识别拼写差 3 ⇒ 必须仍被接受（#156 的口径）: {written}"
    );
    let warnings = written["data"]["warnings"]
        .as_array()
        .expect("warnings 必须是数组");
    assert_eq!(warnings.len(), 1, "正好一条未识别参数警告: {warnings:?}");
    assert_eq!(
        warnings[0]["code"], "unrecognisedEffectParam",
        "{warnings:?}"
    );
    assert_eq!(warnings[0]["paramName"], "reverb_mix", "{warnings:?}");
    assert_eq!(warnings[0]["deviceName"], "EQ", "{warnings:?}");

    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "reverb_mix（#175 判据）"}),
    );
    assert_eq!(merged["status"], "success", "{merged}");
    let queried = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({"trackId": track_id.to_canonical_string()}),
    );
    let inserted = queried["data"]["track"]["inserts"]
        .as_array()
        .expect("inserts")
        .iter()
        .find(|insert| insert["deviceId"] == device_id.to_canonical_string())
        .expect("新设备必须出现在 inserts[] 里");
    assert_eq!(
        inserted["unrecognisedParams"],
        json!(["reverb_mix"]),
        "引擎不认的名字必须在读回里逐字可见（#156）: {inserted}"
    );
}

/// 判据 ④：`#156` 的 **58 个拼写**（32 规范 + 26 别名）一个不少 —— 公告面与查表面两侧。
#[test]
fn the_eight_fifty_eight_published_spellings_are_still_all_there() {
    let names = effect_params::recognised_names();
    assert_eq!(
        names.len(),
        58,
        "拼写总数必须是 32 规范 + 26 别名 = 58，实测 {names:?}"
    );
    assert_eq!(
        effect_params::EFFECT_GROUPS
            .iter()
            .map(|group| group.params.len())
            .sum::<usize>(),
        32,
        "规范名个数"
    );

    // 公告面：`tools/list` 的 `insertDevice.device.params.items.oneOf[].name.enum`。
    let items = op_schema::ops_items();
    let insert = items["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .find(|branch| branch["title"] == "insertDevice")
        .expect("insertDevice 分支必须存在");
    let branches = insert["properties"]["device"]["properties"]["params"]["items"]["oneOf"]
        .as_array()
        .expect("device.params.items.oneOf");
    assert_eq!(branches.len(), 32, "每个引擎参数一个分支");
    let published: Vec<String> = branches
        .iter()
        .flat_map(|branch| {
            branch["properties"]["name"]["enum"]
                .as_array()
                .expect("name.enum")
                .iter()
                .map(|value| value.as_str().expect("enum 项是字符串").to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(published.len(), 58, "公告的拼写总数");
    for name in &names {
        assert!(
            published.iter().any(|item| item == name),
            "`{name}` 必须在公告面里"
        );
    }

    // 查表面：拼错给建议、刻意不认的名字**不**给建议（阈值 2 的两侧）。
    assert!(
        effect_params::recognise("eq_low_gian").is_none(),
        "拼错的词不得被识别"
    );
    let suggestions = effect_params::similar_spellings("eq_low_gian");
    assert_eq!(
        suggestions.first().map(|item| item.canonical),
        Some("eq_low_gain"),
        "{suggestions:?}"
    );
    assert!(
        effect_params::similar_spellings("reverb_mix").is_empty(),
        "`reverb_mix` 与最近的已识别拼写差 3 ⇒ 阈值 2 下不得给建议（#156 的口径）"
    );
    assert_eq!(effect_params::TYPO_MAX_DISTANCE, 2);
}

/// 判据 ⑤：`tools/list` 的 `yeban_edit_notes.description` **逐字**有那一句，
/// 且真实响应的 `pendingMerge` 逐字为 `true`。
#[test]
fn edit_notes_description_says_the_proposal_must_be_merged() {
    const REQUIRED: &str = "本调用永远写入隔离提案分支，必须 `yeban_merge_proposal` 才落到 main";
    let catalog = tools::catalog();
    let edit = catalog["tools"]
        .as_array()
        .expect("tools 必须是数组")
        .iter()
        .find(|tool| tool["name"] == "yeban_edit_notes")
        .expect("yeban_edit_notes 必须注册");
    let description = edit["description"].as_str().expect("description");
    assert!(
        description.contains(REQUIRED),
        "描述里必须有这一句（逐字）: {REQUIRED}\n实际: {description}"
    );
    println!("### 命中的那一句（逐字）\n{REQUIRED}");
    // 也把 `data.pendingMerge` 的逐字响应印出来（判据 ④ 的读数）。
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let response = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [{"kind": "setTempo", "bpm": 121.0}],
        }),
    );
    println!(
        "### 提案响应的 pendingMerge（逐字）\n{}",
        serde_json::to_string_pretty(&json!({
            "status": response["status"],
            "pendingMerge": response["data"]["pendingMerge"],
            "mergeTool": response["data"]["mergeTool"],
            "projectUnchanged": response["data"]["projectUnchanged"],
            "proposalStatus": response["data"]["proposal"]["status"],
        }))
        .expect("pretty")
    );
    assert_eq!(response["data"]["pendingMerge"], true, "{response}");
    assert_eq!(response["data"]["proposal"]["status"], "open", "{response}");
}
