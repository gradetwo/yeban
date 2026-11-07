//! **#156 的判据**：效果器参数契约的**可发现性** + `query_engine_state` 的验收读数。
//!
//! ## 这个文件量什么、单位是什么
//!
//! | 判据 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | [`tools_list_publishes_one_branch_per_parser_kind`] | `catalog()` 里 `yeban_edit_notes.inputSchema.properties.ops.items.oneOf` | **分支数**（每个 `OP_KINDS` 条目 1 个） |
//! | [`every_published_effect_name_is_a_quoted_literal_in_insert_rs`] | `effect_params::recognised_names()` | **名字数**（含别名），逐个对照 `crates/yeban-engine/src/insert.rs` 的带引号字面量 |
//! | [`the_published_effect_contract_carries_name_range_unit_and_default`] | `device.params.items.oneOf` | **参数分支数**（= 32 个规范名） |
//! | [`eq_and_reverb_round_trip_and_sends_are_readable`] | 一次 `insertDevice` + `connectRouting` → `query_engine_state` | `inserts[]` / `sends[]` 的**条目数** |
//!
//! 复跑读数：`cargo test -p yeban-mcp --test effect_contract -- --nocapture`

use std::str::FromStr as _;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_mcp::tools::{self, TOOLS};
use yeban_mcp::{effect_params, op_schema};
use yeban_model::EntityId;

/// `tools/list` 里 `yeban_edit_notes` 的 `ops.items`。
fn published_ops_items() -> Value {
    let catalog = tools::catalog();
    let edit = catalog["tools"]
        .as_array()
        .expect("tools 必须是数组")
        .iter()
        .find(|tool| tool["name"] == "yeban_edit_notes")
        .expect("yeban_edit_notes 必须注册");
    edit["inputSchema"]["properties"]["ops"]["items"].clone()
}

/// `device.params.items.oneOf`（效果器参数契约）。
fn published_effect_branches() -> Vec<Value> {
    let items = published_ops_items();
    let insert = items["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .find(|branch| branch["title"] == "insertDevice")
        .expect("insertDevice 分支必须存在");
    insert["properties"]["device"]["properties"]["params"]["items"]["oneOf"]
        .as_array()
        .expect("device.params.items.oneOf")
        .clone()
}

/// 判据 ①：`ops.items` 的 `oneOf` 恰好覆盖 `OP_KINDS` 全集，且每个分支都必填 `kind`。
///
/// `additionalProperties` 也逐分支对账：解析器里只有
/// `add`/`delete`/`move`/`velocity`/`setParam`/`setAutomationLane` 六个形态
/// **不**拒绝操作对象上的多余键；其余形态都有 `reject_*_fields`。
#[test]
fn tools_list_publishes_one_branch_per_parser_kind() {
    let items = published_ops_items();
    let branches = items["oneOf"].as_array().expect("ops.items.oneOf");
    let mut titles: Vec<&str> = branches
        .iter()
        .map(|branch| branch["title"].as_str().expect("每个分支必须有 title"))
        .collect();
    titles.sort_unstable();
    let mut expected: Vec<&str> = yeban_mcp::domain::notes::OP_KINDS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        titles, expected,
        "`ops.items.oneOf` 的分支集合必须与解析器的 `OP_KINDS` 逐项相等"
    );

    let open = [
        "add",
        "delete",
        "move",
        "velocity",
        "setParam",
        "setAutomationLane",
    ];
    for branch in branches {
        let kind = branch["title"].as_str().expect("title");
        assert_eq!(
            branch["properties"]["kind"]["const"], kind,
            "`{kind}` 分支的 `kind` 必须用 `const` 锁定"
        );
        assert!(
            branch["required"]
                .as_array()
                .expect("required")
                .iter()
                .any(|item| item == "kind"),
            "`{kind}` 分支必须把 `kind` 列为必填"
        );
        // 解析器真的会拒绝多余键 ⇔ schema 的 additionalProperties:false。
        assert_eq!(
            branch["additionalProperties"],
            Value::from(open.contains(&kind)),
            "`{kind}` 的 additionalProperties 与解析器的未知键口径不一致"
        );
    }
    // 效果器参数契约必须挂在 `insertDevice.device.params.items` 上。
    let branches = published_effect_branches();
    assert_eq!(
        branches.len(),
        effect_params::EFFECT_GROUPS
            .iter()
            .map(|group| group.params.len())
            .sum::<usize>(),
        "每个引擎参数一个 branch"
    );
    assert!(
        !branches.is_empty(),
        "device.params.items 不得为空（空 oneOf 等于没公告）"
    );
}

/// 判据 ②：每个公告名都**真的**以带引号字面量出现在引擎白名单里。
///
/// 这是对"字面量抄写"的机械对账（本 crate ⛔ 不依赖 `yeban-engine`，因此读源码
/// 而不是链接）：少一个名字 / 多一个引擎不认的名字都会红。
///
/// ⚠ 本判据是**单向**的：它拦不住"引擎认、公告不认"的漂移（通道条 `#341/#342/#343/
/// #344/#346` 十条就是这么漏的）。反向由判据 ②′（`every_engine_effect_name_is_published`）钉住。
///
/// 白名单住在两处，缺一不可：`insert.rs` 自己的匹配臂（通道条 / 混响 / 卷积）＋
/// `yeban-dsp` 的 `device_param.rs` 共享拼写表（延迟 15 拼写；引擎的 `delay_of`
/// 直接调 `DelayField::from_name`，⛔ 不在引擎里抄第二份表 —— 抄了就会漂）。
/// 因此本判据同时扫这两份源码：名字只在其中一份里出现即算认下。
#[test]
fn every_published_effect_name_is_a_quoted_literal_in_insert_rs() {
    let engine_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("yeban-engine")
        .join("src");
    let insert_rs = engine_dir.join("insert.rs");
    let insert_source = std::fs::read_to_string(&insert_rs)
        .unwrap_or_else(|error| panic!("读引擎白名单 {} 失败: {error}", insert_rs.display()));
    let dsp_rs = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("yeban-dsp")
        .join("src")
        .join("device_param.rs");
    let dsp_source = std::fs::read_to_string(&dsp_rs)
        .unwrap_or_else(|error| panic!("读共享拼写表 {} 失败: {error}", dsp_rs.display()));

    let names = effect_params::recognised_names();
    assert!(names.len() >= 32, "公告名数量异常: {}", names.len());
    let mut missing: Vec<&str> = Vec::new();
    for name in &names {
        let literal = format!("\"{name}\"");
        if !insert_source.contains(&literal) && !dsp_source.contains(&literal) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "这些公告名在 `insert.rs` 与共享拼写表里都找不到带引号字面量（引擎不认 ⇒ 公告是假的）: {missing:?}"
    );
    // 反向：引擎刻意不认的两个名字不得出现在公告里。
    for forbidden in ["reverb_mix", "reverb_width"] {
        assert!(
            !names.contains(&forbidden),
            "`{forbidden}` 是引擎刻意不认的名字，不得公告"
        );
        assert!(
            effect_params::recognise(forbidden).is_none(),
            "`{forbidden}` 不得被识别"
        );
    }
    // 大小写不敏感（引擎把名字转小写后匹配）。
    assert!(effect_params::recognise("EQ_LOW_GAIN").is_some());
    assert!(effect_params::recognise("Reverb_Wet").is_some());
}

/// 判据 ②′：反向对账 —— 引擎真认的每个名字，公告表都必须认。
///
/// 判据 ② 是单向的（表 ⊆ 引擎），拦不住“引擎认、公告不认”的漂移：通道条
/// `#341/#342/#343/#344/#346` 十条（`hpf_hz` / `strip_width` / `mono_below_hz` /
/// `ms_mid_*` / `deq_*`，各 1 别名）就是这么漏的 —— 夜半原生预制线实测：
/// `Preset: Channel Strip` 的 `strip_width` 等被报 `unrecognisedEffectParam`
/// （“不会生效”），而渲染侧 `insert.rs` 的 `StripProjection::apply` 真生效。
/// 本判据同时扫两份引擎真相：`insert.rs` 的 match 臂与 `device_param.rs` 的
/// `SPELLINGS` 表；任一名字不在公告里即红。
#[test]
fn every_engine_effect_name_is_published() {
    use yeban_mcp::effect_params;
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let insert_source = std::fs::read_to_string(
        crate_dir
            .join("..")
            .join("yeban-engine")
            .join("src")
            .join("insert.rs"),
    )
    .expect("读 insert.rs");
    let dsp_source = std::fs::read_to_string(
        crate_dir
            .join("..")
            .join("yeban-dsp")
            .join("src")
            .join("device_param.rs"),
    )
    .expect("读 device_param.rs");
    let mut engine_names: Vec<String> = Vec::new();
    // `insert.rs` 的 match 臂：`"名" | "别名" =>`（小写蛇形名才收；大写打头的
    // 设备 kind 字面量不在此列）。
    for line in insert_source.lines() {
        let Some(arrow) = line.find("=>") else {
            continue;
        };
        engine_names.extend(quoted_snake_names(&line[..arrow]));
    }
    // `device_param.rs` 的拼写表：`("名", Self::字段),`。
    for line in dsp_source.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("(\"") || !trimmed.contains(", Self::") {
            continue;
        }
        engine_names.extend(quoted_snake_names(trimmed));
    }
    assert!(
        !engine_names.is_empty(),
        "引擎侧一个名字都没扫到：扫描器坏了，不是表全了"
    );
    let mut missing: Vec<String> = Vec::new();
    for name in &engine_names {
        if effect_params::recognise(name).is_none() {
            missing.push(name.clone());
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "这些引擎真认的名字在公告表里找不到（调用方会被误报 `unrecognisedEffectParam`）: {missing:?}"
    );
}

/// 一行里 `=>` 之前（或拼写表行里）的全部带引号小写蛇形名。
fn quoted_snake_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        let name = &after[..end];
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            out.push(name.to_owned());
        }
        rest = &after[end + 1..];
    }
    out
}

/// 判据 ③：每个参数分支都能读到 **名字 + 类型 + 范围 + 单位 + 缺省值**。
#[test]
fn the_published_effect_contract_carries_name_range_unit_and_default() {
    let branches = published_effect_branches();
    for branch in &branches {
        let names = branch["properties"]["name"]["enum"]
            .as_array()
            .expect("name.enum");
        assert!(!names.is_empty(), "每个参数至少要有一个拼写");
        let value = &branch["properties"]["value"];
        assert_eq!(value["type"], "number", "参数值必须是 number（f32）");
        assert!(
            value["minimum"].as_f64().is_some() && value["maximum"].as_f64().is_some(),
            "参数 `{}` 必须广播数值范围: {branch}",
            names[0]
        );
        assert!(
            value["default"].as_f64().is_some(),
            "参数 `{}` 必须广播缺省值: {branch}",
            names[0]
        );
        assert!(
            branch["description"]
                .as_str()
                .is_some_and(|text| text.contains("值域")),
            "分支描述必须写明值域与单位"
        );
        assert!(
            branch["properties"]["unit"]["type"]
                .as_array()
                .is_some_and(|types| types.contains(&Value::from("string"))),
            "unit 必须是 string|null"
        );
    }

    let find = |name: &str| {
        branches
            .iter()
            .find(|branch| {
                branch["properties"]["name"]["enum"]
                    .as_array()
                    .expect("enum")
                    .iter()
                    .any(|item| item == name)
            })
            .unwrap_or_else(|| panic!("公告里没有 `{name}`"))
    };
    let eq_low_gain = find("eq_low_gain");
    assert_eq!(eq_low_gain["properties"]["value"]["minimum"], -18.0);
    assert_eq!(eq_low_gain["properties"]["value"]["maximum"], 18.0);
    assert_eq!(
        eq_low_gain["properties"]["value"]["default"], 0.0,
        "缺省 0 dB"
    );
    // 别名 `low_gain` 与规范名在同一个分支里。
    assert!(
        eq_low_gain["properties"]["name"]["enum"]
            .as_array()
            .expect("enum")
            .iter()
            .any(|item| item == "low_gain")
    );

    let reverb_wet = find("reverb_wet");
    assert_eq!(reverb_wet["properties"]["value"]["minimum"], 0.0);
    assert_eq!(reverb_wet["properties"]["value"]["maximum"], 1.0);
    let reverb_predelay = find("reverb_predelay");
    assert_eq!(reverb_predelay["properties"]["value"]["maximum"], 0.1);
    let conv_decay = find("conv_ir_decay_s");
    assert_eq!(conv_decay["properties"]["value"]["minimum"], 0.005);
    assert_eq!(conv_decay["properties"]["value"]["maximum"], 5.0);
}

// ---------------------------------------------------------------------------
// 端到端：写 EQ / 混响 / 发送，然后从 `query_engine_state` 读回来
// ---------------------------------------------------------------------------

/// 测试用分发器 + 规范样本工程。
fn dispatcher() -> (Dispatcher, String, yeban_model::YebanProjectV1) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let project = yeban_model::samples::filled_project();
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            std::path::PathBuf::from("/tmp/yeban-effect-contract.yeban"),
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
        "id": "effect-contract",
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

/// 一条带摆放的音轨 + 片段（工具签名要求顶层 `trackId`/`clipId`）。
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

/// 判据 ④：按 `tools/list` 公告的形状写 EQ + 混响 + 发送，再读回验收读数。
#[test]
fn eq_and_reverb_round_trip_and_sends_are_readable() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let eq_id = ulid("01ARZ3NDEKTSV4RRFFQ69G5FA1");
    let reverb_id = ulid("01ARZ3NDEKTSV4RRFFQ69G5FA2");
    let send_edge_id = ulid("01ARZ3NDEKTSV4RRFFQ69G5FA3");

    // 这台音轨上已有一条 sample 里的发送边（lead → Aux Reverb，-12 dB）。
    let aux_id = project
        .routing_graph
        .edges
        .values()
        .find(|edge| edge.kind == yeban_model::RoutingKind::SendToAux)
        .map(|edge| edge.destination_node)
        .expect("样本里必须有一条 SendToAux 边");

    let eq_device = json!({
        "deviceId": eq_id.to_canonical_string(),
        "name": "EQ",
        "kind": "InternalEffect",
        "bypassed": false,
        "latencySamples": 0,
        "params": [
            {"name": "eq_low_gain", "value": 6.0, "unit": "dB"},
            {"name": "eq_low_freq", "value": 120.0, "unit": "Hz"},
            {"name": "eq_mid_gain", "value": -3.0, "unit": "dB"},
            {"name": "eq_mid_freq", "value": 900.0, "unit": "Hz"},
            {"name": "eq_mid_q", "value": 1.4, "unit": "Q"},
            {"name": "eq_high_gain", "value": 2.0, "unit": "dB"},
            {"name": "eq_high_freq", "value": 6000.0, "unit": "Hz"},
        ],
    });
    let reverb_device = json!({
        "deviceId": reverb_id.to_canonical_string(),
        "name": "Plate",
        "kind": "InternalEffect",
        "bypassed": false,
        "latencySamples": 0,
        "params": [
            {"name": "reverb_size", "value": 0.7},
            {"name": "reverb_damp", "value": 0.3},
            {"name": "reverb_wet", "value": 0.35},
            {"name": "reverb_predelay", "value": 0.02, "unit": "s"},
            // ⚠ 引擎**刻意不认**这个名字 ⇒ 读回必须报在 `unrecognisedParams` 里。
            {"name": "reverb_mix", "value": 0.5},
        ],
    });

    let written = call(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": track_id.to_canonical_string(),
            "clipId": clip_id.to_canonical_string(),
            "ops": [
                {"kind": "insertDevice", "trackId": track_id.to_canonical_string(), "device": eq_device},
                {"kind": "insertDevice", "trackId": track_id.to_canonical_string(), "device": reverb_device},
                // 发送的写法就是公告的 `connectRouting` + `edgeKind:"SendToAux"`。
                {
                    "kind": "connectRouting",
                    "edgeId": send_edge_id.to_canonical_string(),
                    "sourceNode": track_id.to_canonical_string(),
                    "destinationNode": aux_id.to_canonical_string(),
                    "edgeKind": "SendToAux",
                    "gainDb": -6.0
                }
            ],
        }),
    );
    assert_eq!(written["status"], "success", "写入失败: {written}");
    // ⚠ `yeban_edit_notes` 产出的是一份**提案**（可撤销），必须显式合并才落到工程。
    let proposal_id = written["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("提案 id")
        .to_owned();
    let merged = call(
        &mut dispatcher,
        &auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal_id, "commitMessage": "EQ + reverb + send (#156 判据)"}),
    );
    assert_eq!(merged["status"], "success", "合并失败: {merged}");

    let queried = call(
        &mut dispatcher,
        &auth,
        "yeban_query_engine_state",
        json!({"trackId": track_id.to_canonical_string()}),
    );
    let data = &queried["data"];
    println!(
        "### yeban_query_engine_state data（逐字）\n{}",
        serde_json::to_string_pretty(data).expect("pretty")
    );

    // ① 每轨 insert 摘要：设备名 + 槽位 + 已识别 / 未识别参数名。
    let inserts = data["track"]["inserts"].as_array().expect("inserts");
    let eq = inserts
        .iter()
        .find(|insert| insert["deviceName"] == "EQ")
        .expect("EQ 摘要必须存在");
    assert_eq!(eq["active"], true);
    assert_eq!(eq["effects"], json!(["eq"]));
    assert_eq!(
        eq["recognisedParams"],
        json!([
            "eq_low_gain",
            "eq_low_freq",
            "eq_mid_gain",
            "eq_mid_freq",
            "eq_mid_q",
            "eq_high_gain",
            "eq_high_freq"
        ])
    );
    assert_eq!(eq["unrecognisedParams"], json!([]));
    assert!(eq["slotIndex"].as_u64().is_some(), "槽位必须是数字");

    let reverb = inserts
        .iter()
        .find(|insert| insert["deviceName"] == "Plate")
        .expect("混响摘要必须存在");
    assert_eq!(reverb["effects"], json!(["reverb"]));
    assert_eq!(
        reverb["recognisedParams"],
        json!([
            "reverb_size",
            "reverb_damp",
            "reverb_wet",
            "reverb_predelay"
        ])
    );
    assert_eq!(
        reverb["unrecognisedParams"],
        json!(["reverb_mix"]),
        "引擎不认的名字必须**响亮**列出来（⛔ 不许静默通过）"
    );

    // 参数级读数也带 recognised/canonicalName/effect。
    let params = data["track"]["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .find(|device| device["name"] == "Plate")
        .expect("Plate 设备")
        .get("params")
        .and_then(Value::as_array)
        .expect("params");
    let mix = params
        .iter()
        .find(|param| param["name"] == "reverb_mix")
        .expect("reverb_mix 参数");
    assert_eq!(mix["recognised"], false);
    assert!(mix["canonicalName"].is_null());
    assert!(mix["effect"].is_null());
    let wet = params
        .iter()
        .find(|param| param["name"] == "reverb_wet")
        .expect("reverb_wet 参数");
    assert_eq!(wet["recognised"], true);
    assert_eq!(wet["canonicalName"], "reverb_wet");
    assert_eq!(wet["effect"], "reverb");

    // ② 发送：edgeId + gainDb + 目的地（身份与显示名）。
    let sends = data["sends"].as_array().expect("顶层 sends");
    let created = sends
        .iter()
        .find(|send| send["edgeId"] == send_edge_id.to_canonical_string())
        .expect("新建的发送边必须出现在 sends[] 里");
    assert_eq!(created["gainDb"], -6.0);
    assert_eq!(created["destinationNode"], aux_id.to_canonical_string());
    assert_eq!(created["destinationName"], "Aux Reverb");
    assert_eq!(created["kind"], "SendToAux");
    let sample_send = sends
        .iter()
        .find(|send| send["gainDb"] == -12.0)
        .expect("样本里那条 -12 dB 的发送边");
    assert_eq!(sample_send["destinationName"], "Aux Reverb");

    // 这条音轨自己的发送也挂在 `track.sends` 下。
    let track_sends = data["track"]["sends"].as_array().expect("track.sends");
    assert!(
        track_sends
            .iter()
            .any(|send| send["edgeId"] == send_edge_id.to_canonical_string()),
        "新建的发送必须出现在 track.sends 下"
    );
}

/// 判据 ⑤：未知**顶层实参**仍被响亮拒绝（JSON-RPC `-32602`，报文点名参数）。
#[test]
fn an_unknown_top_level_argument_is_still_loudly_rejected() {
    let (mut dispatcher, auth, project) = dispatcher();
    let (track_id, clip_id) = track_and_clip(&project);
    let line = json!({
        "jsonrpc": "2.0",
        "id": "unknown-param",
        "method": "tools/call",
        "params": {
            "name": "yeban_edit_notes",
            "arguments": {
                "trackId": track_id.to_canonical_string(),
                "clipId": clip_id.to_canonical_string(),
                "ops": [{"kind": "removeClip"}],
                "UNKNOWN": 1
            }
        }
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    let response = outcome.response.expect("必须有响应");
    let error = response.error.expect("未知实参必须是 JSON-RPC 层错误");
    assert_eq!(error.code, -32602, "未知实参必须是 INVALID_PARAMS");
    println!(
        "### unknown argument error: code={} message={}",
        error.code, error.message
    );
    let detail = serde_json::to_string(&error.data).expect("错误 data 必须可序列化");
    println!("### unknown argument error data: {detail}");
    assert!(
        detail.contains("UNKNOWN"),
        "结构化错误必须点名那个未知参数: {detail}"
    );
    assert!(response.result.is_none(), "未知实参不得产生带内成功结果");
}

/// `op_schema` 是公开 API（判据也用它做覆盖度自检）。
#[test]
fn op_schema_is_the_same_object_the_catalog_publishes() {
    let direct = op_schema::ops_items();
    let published = published_ops_items();
    assert_eq!(direct, published, "`tools/list` 必须发布同一份 schema");
    assert_eq!(
        direct["oneOf"].as_array().expect("oneOf").len(),
        TOOLS
            .iter()
            .find(|spec| spec.name == "yeban_edit_notes")
            .expect("注册项")
            .param("ops")
            .expect("ops 参数")
            .items
            .map(|build| build()["oneOf"].as_array().expect("oneOf").len())
            .expect("ops 必须带 items 构造器"),
    );
}

/// `#311c`：`add.note` 的**公布** schema 必须真的广告 `pitchBendCurve` / `slide`，
/// 且 `slide` 的必填集与曲线枚举与解析器同源。
///
/// 为什么单独一条：只测解析器（`notes.rs` 的单测）会漏掉"能力可达但**不可发现**"——
/// 外部 Agent 按 `tools/list` 写载荷，schema 里没有的键它不会写。本条钉住
/// `note_object()` 的公告面与 `add.note` 的必填集（两个新字段都必须是**可选**的）。
#[test]
fn the_published_add_note_schema_advertises_bend_and_slide() {
    let items = published_ops_items();
    let add = items["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .find(|branch| branch["title"] == "add")
        .expect("add 分支");
    let note = &add["properties"]["note"];
    let properties = &note["properties"];

    let required: Vec<&str> = note["required"]
        .as_array()
        .expect("note.required")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        required,
        vec!["startTick", "pitch", "durationTicks"],
        "两个新字段必须是**可选**的（与 parse_note 的缺省 = 模型默认一致）"
    );

    assert_eq!(
        properties["pitchBendCurve"]["type"], "array",
        "`pitchBendCurve` 必须公布: {properties}"
    );
    assert_eq!(properties["pitchBendCurve"]["items"]["minItems"], 2);
    assert_eq!(properties["pitchBendCurve"]["items"]["maxItems"], 2);
    assert_eq!(
        properties["pitchBendCurve"]["items"]["items"]["type"], "integer",
        "`[tick, cents]` 的两项都必须是整数"
    );

    let slide = &properties["slide"];
    assert_eq!(slide["type"], "object", "`slide` 必须公布: {properties}");
    assert_eq!(
        slide["additionalProperties"], false,
        "`slide` 的未知键口径必须是封闭的（与 read_slide 的响亮拒绝一致）"
    );
    let slide_required: Vec<&str> = slide["required"]
        .as_array()
        .expect("slide.required")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        slide_required,
        vec!["durationTicks", "targetPitch"],
        "`slide` 的两个必填键必须与 `read_slide` 同源"
    );
    assert_eq!(
        slide["properties"]["curve"]["enum"],
        serde_json::json!(yeban_mcp::domain::extension_pure::CURVE_NAMES),
        "`slide.curve` 的枚举必须引用规范名表（⛔ 手抄第二份）"
    );

    assert_eq!(
        properties["channel"]["type"], "integer",
        "`channel` 必须公布: {properties}"
    );
    assert_eq!(properties["channel"]["minimum"], 0);
    assert_eq!(properties["channel"]["maximum"], 15);

    assert_eq!(
        properties["syllable"]["type"], "string",
        "`syllable` 必须公布: {properties}"
    );
}

/// `setKeySignature` 分支在 `tools/list` 里的发布面验证。
#[test]
fn the_published_set_key_signature_schema_advertises_correct_fields() {
    let items = published_ops_items();
    let set_key = items["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .find(|branch| branch["title"] == "setKeySignature")
        .expect("setKeySignature 分支");
    assert_eq!(set_key["additionalProperties"], false);
    let props = &set_key["properties"];
    assert_eq!(props["kind"]["const"], "setKeySignature");
    let key_sig = &props["keySignature"];
    assert_eq!(key_sig["type"], json!(["object", "null"]));
    let ks_props = &key_sig["properties"];
    assert_eq!(ks_props["sharpsFlats"]["type"], "integer");
    assert_eq!(ks_props["sharpsFlats"]["minimum"], -7);
    assert_eq!(ks_props["sharpsFlats"]["maximum"], 7);
    assert_eq!(ks_props["isMinor"]["type"], "boolean");
    assert_eq!(key_sig["required"], json!(["sharpsFlats", "isMinor"]));
}
