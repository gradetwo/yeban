//! `yeban_launch_scene`（`#290`）的**领域面**判据（形态 B / dryRun / 有界队列）。
//!
//! ## 这条判据要证的句子
//!
//! - **形态 B（独立 stdio 进程）没有引擎** ⇒ 工具**响亮**报
//!   `CONFLICT` + `reason:"engineUnavailable"`，⛔ 不返回 `success` + `engine:null`；
//! - **接上宿主口**（形态 A 的 `yeban-app` 会做）之后，一次调用**恰好**入队一条请求，
//!   响应里 `queued:true` 但 `applied:false`（量化启动在**未来**生效）；
//! - `dryRun` **一条都不入队**；
//! - 未知 `sceneId` 是 `ENTITY_NOT_FOUND`（`reason:"sceneNotFound"` + `sceneCount`），
//!   ⛔ **不**回落第一个场景。
//!
//! 真正“到点生效”的判据在 `crates/yeban-app/tests/mcp_scene_launch.rs`
//! （形态 A：真 `EngineHost` + 真 `EngineRuntime::queue_scene_launch` + pump）。
//!
//! 运行：`cargo test -p yeban-mcp --test scene_launch_tool`

use std::path::PathBuf;

use serde_json::{Value, json};

use yeban_mcp::Dispatcher;
use yeban_mcp::domain::scene_launch::MAX_PENDING_SCENE_LAUNCHES;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{EntityId, YebanProjectV1};

/// 注入规范样本工程的分发器。
fn dispatcher_with_project(project: YebanProjectV1) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let authorization = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(
            PathBuf::from("/tmp/yeban-scene-launch-tool.yeban"),
            project,
            false,
        )
        .expect("注入规范工程");
    (dispatcher, authorization)
}

/// 走 `tools/call` 并返回带内 `ToolResponse`。
fn call(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "scene-launch",
        "method": "tools/call",
        "params": {"name": "yeban_launch_scene", "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "不得有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

/// 样本工程里的第一个场景身份（`BTreeMap` 迭代顺序）。
fn first_scene(project: &YebanProjectV1) -> EntityId {
    *project.scenes.keys().next().expect("样本必须有场景")
}

#[test]
fn the_tool_is_discoverable_on_every_catalog_face() {
    use yeban_mcp::tools::{self, TOOL_COUNT};

    assert!(
        tools::EXTENSION_NAMES.contains(&"yeban_launch_scene"),
        "扩展清单必须点名它"
    );
    let spec = tools::tool("yeban_launch_scene").expect("注册表");
    assert_eq!(spec.spec_id, "MCP-TOOL-EXT-SCENE-LAUNCH");
    assert_eq!(spec.scope, yeban_mcp::security::Scope::AppAdmin);
    assert!(spec.param("sceneId").expect("sceneId 参数").required);

    // ① `tools/list`（= `tools::catalog()`）
    let catalog = tools::catalog();
    let descriptors = catalog["tools"].as_array().expect("tools 数组");
    let names: Vec<&str> = descriptors
        .iter()
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect();
    assert!(names.contains(&"yeban_launch_scene"), "{names:?}");
    assert_eq!(names.len(), TOOL_COUNT, "tools/list 必须列全");

    // ② 规范样本的 registry（同一份注册表的机器可读清单）
    let registry = yeban_mcp::samples::registry_sample();
    assert_eq!(registry["count"], TOOL_COUNT);
    assert!(
        registry["tools"]
            .as_array()
            .expect("registry.tools")
            .iter()
            .any(|tool| tool["name"] == "yeban_launch_scene"),
        "registry 样本必须点名它"
    );

    // ③ schema 的 enum / if-then / $defs 三处
    let schema: Value = serde_json::from_str(
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
    let enum_names: Vec<&str> = schema["definitions"]["ToolCall"]["properties"]["name"]["enum"]
        .as_array()
        .expect("name.enum")
        .iter()
        .map(|name| name.as_str().expect("字符串"))
        .collect();
    assert!(enum_names.contains(&"yeban_launch_scene"), "{enum_names:?}");
    assert_eq!(enum_names.len(), TOOL_COUNT);
    assert!(
        schema["definitions"]["ExtensionToolArguments"]["$defs"]
            .get("yeban_launch_scene")
            .is_some(),
        "ExtensionToolArguments.$defs 必须有点名"
    );
    assert!(
        schema["definitions"]["ToolCall"]["allOf"]
            .as_array()
            .expect("allOf")
            .iter()
            .any(|branch| branch["if"]["properties"]["name"]["const"] == "yeban_launch_scene"),
        "if/then 必须接线"
    );

    // ④ 没有声明 `outputSchema` 就**不广播**（⛔ 不拿空对象冒充）。
    let descriptor = descriptors
        .iter()
        .find(|tool| tool["name"] == "yeban_launch_scene")
        .expect("描述符");
    assert!(descriptor.get("outputSchema").is_none());
}

#[test]
fn without_a_host_the_tool_is_a_loud_engine_unavailable_error() {
    let project = yeban_model::samples::filled_project();
    let scene = first_scene(&project);
    let (mut dispatcher, auth) = dispatcher_with_project(project);

    let response = call(
        &mut dispatcher,
        &auth,
        json!({"sceneId": scene.to_canonical_string()}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], "CONFLICT", "{response}");
    let data = &response["error"]["data"];
    assert_eq!(data["reason"], "engineUnavailable", "{response}");
    assert_eq!(data["form"], "stdio", "{response}");
    assert_eq!(data["hostAttached"], false, "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .expect("message")
            .contains("本形态没有引擎"),
        "报文必须逐字说明没有引擎: {response}"
    );
    assert_eq!(
        dispatcher.domain().pending_scene_launch_count(),
        0,
        "失败不得留下排队请求"
    );
}

#[test]
fn attaching_the_host_queues_exactly_one_request_and_dry_run_queues_none() {
    let project = yeban_model::samples::filled_project();
    let scene = first_scene(&project);
    let expected_tempo = project.scenes[&scene].tempo;
    let (mut dispatcher, auth) = dispatcher_with_project(project);
    dispatcher.domain_mut().attach_scene_launch_host();
    assert!(dispatcher.domain().scene_launch_host_attached());

    // ---- dryRun：成功预览，但**一条都不入队** ----
    let dry = call(
        &mut dispatcher,
        &auth,
        json!({"sceneId": scene.to_canonical_string(), "dryRun": true}),
    );
    assert_eq!(dry["status"], "success", "{dry}");
    assert_eq!(dry["data"]["dryRun"], true, "{dry}");
    let preview = &dry["data"]["preview"];
    assert_eq!(preview["queued"], false, "{dry}");
    assert_eq!(preview["applied"], false, "{dry}");
    assert_eq!(preview["sceneId"], scene.to_canonical_string(), "{dry}");
    assert_eq!(preview["wouldQueue"], true, "{dry}");
    assert_eq!(
        dispatcher.domain().pending_scene_launch_count(),
        0,
        "dryRun 不得入队"
    );

    // ---- 真调用：恰好一条，queued=true / applied=false ----
    let real = call(
        &mut dispatcher,
        &auth,
        json!({"sceneId": scene.to_canonical_string()}),
    );
    assert_eq!(real["status"], "success", "{real}");
    let data = &real["data"];
    assert_eq!(data["queued"], true, "{real}");
    assert_eq!(data["applied"], false, "{real}");
    assert_eq!(data["sceneId"], scene.to_canonical_string(), "{real}");
    assert_eq!(data["sceneIndex"], 0, "{real}");
    assert_eq!(data["host"], "in-process", "{real}");
    assert_eq!(data["quantization"], "Bar", "{real}");
    if let Some(tempo) = expected_tempo {
        assert_eq!(data["tempoBpm"], tempo, "{real}");
        assert_eq!(data["tempoSource"], "sceneOverride", "{real}");
    }
    assert_eq!(dispatcher.domain().pending_scene_launch_count(), 1);

    let requests = dispatcher.domain_mut().take_scene_launch_requests();
    assert_eq!(requests.len(), 1, "恰好一条");
    assert_eq!(requests[0].scene_id, scene);
    assert_eq!(
        requests[0].quantization,
        yeban_model::LaunchQuantization::Bar
    );
    assert_eq!(dispatcher.domain().pending_scene_launch_count(), 0);
}

#[test]
fn an_unknown_scene_id_is_entity_not_found_and_never_falls_back() {
    let project = yeban_model::samples::filled_project();
    let scene_count = project.scenes.len();
    let (mut dispatcher, auth) = dispatcher_with_project(project);
    dispatcher.domain_mut().attach_scene_launch_host();
    let ghost = "01J8ZQ00000000000000000000";

    let response = call(&mut dispatcher, &auth, json!({"sceneId": ghost}));
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], "ENTITY_NOT_FOUND", "{response}");
    assert_eq!(
        response["error"]["data"]["reason"], "sceneNotFound",
        "{response}"
    );
    assert_eq!(response["error"]["data"]["sceneId"], ghost, "{response}");
    assert_eq!(
        response["error"]["data"]["sceneCount"], scene_count,
        "{response}"
    );
    assert_eq!(
        dispatcher.domain().pending_scene_launch_count(),
        0,
        "⛔ 不得回落到第一个场景"
    );
}

#[test]
fn the_queue_is_bounded_and_full_is_a_loud_busy() {
    let project = yeban_model::samples::filled_project();
    let scene = first_scene(&project);
    let (mut dispatcher, auth) = dispatcher_with_project(project);
    dispatcher.domain_mut().attach_scene_launch_host();
    for _ in 0..MAX_PENDING_SCENE_LAUNCHES {
        let response = call(
            &mut dispatcher,
            &auth,
            json!({"sceneId": scene.to_canonical_string()}),
        );
        assert_eq!(response["status"], "success", "{response}");
    }
    let over = call(
        &mut dispatcher,
        &auth,
        json!({"sceneId": scene.to_canonical_string()}),
    );
    assert_eq!(over["status"], "error", "{over}");
    assert_eq!(over["error"]["code"], "BUSY", "{over}");
    assert_eq!(
        over["error"]["data"]["reason"], "sceneLaunchQueueFull",
        "{over}"
    );
    assert_eq!(
        dispatcher.domain().pending_scene_launch_count(),
        MAX_PENDING_SCENE_LAUNCHES
    );
}
