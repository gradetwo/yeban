//! 逐件链渲染回执判据（逐件处理线期 3）：`laneChainsDeclared` /
//! `laneChainsUnapplied` 必须说真话。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | 鼓机生成装上的逐件链在渲染后 `declared = 1`、`unapplied = 0`（真的建了分组源） | `generated_lane_chains_render_applied` |
//! | ② | 一条没有音符、没进渲染源集的轨上声明的逐件链 ⇒ `declared = 1`、`unapplied = 1`（不许冒充已应用） | `lane_chains_on_an_unrendered_track_stay_unapplied` |

use std::path::PathBuf;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, LaneChainDef, ParameterValue, RoutingEdge, RoutingKind,
    TrackKind, TrackV3, YebanProjectV1,
};

fn dispatcher_with_project(project: YebanProjectV1) -> Dispatcher {
    let token = BearerToken::generate().token;
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(unique_path("fixture"), project, false)
        .expect("注入规范工程");
    dispatcher
}

fn unique_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "yeban-mcp-lane-render-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ))
        .join("demo.yeban")
}

fn call_tool(dispatcher: &mut Dispatcher, name: &str, arguments: Value) -> Value {
    let auth = format!("Bearer {}", dispatcher.expected_token().expose());
    let line = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    assert_eq!(
        outcome.error_code(),
        None,
        "`{name}` 不得产出 JSON-RPC 层错误: {outcome:?}"
    );
    let response = outcome.response.expect("响应");
    response.result.expect("result")
}

fn render_master(dispatcher: &mut Dispatcher, tag: &str) -> Value {
    let out = std::env::temp_dir().join(format!(
        "yeban-mcp-lane-render-{}-{tag}.wav",
        std::process::id()
    ));
    let result = call_tool(
        dispatcher,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
        }),
    );
    assert_eq!(result["status"], "success", "{result}");
    result["data"].clone()
}

/// ① house 生成装上一条 hats 逐件链 ⇒ 渲染回执 declared 1 / unapplied 0。
#[test]
fn generated_lane_chains_render_applied() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let generated = call_tool(
        &mut dispatcher,
        "yeban_generate_drum_pattern",
        json!({ "style": "house", "autoMerge": true }),
    );
    assert_eq!(generated["status"], "success", "{generated}");
    let data = render_master(&mut dispatcher, "applied");
    assert_eq!(
        data["laneChainsDeclared"], 1,
        "house 预置声明一条逐件链: {data}"
    );
    assert_eq!(
        data["laneChainsUnapplied"], 0,
        "鼓轨进了本次渲染且建了分组源 ⇒ 未应用必须归零: {data}"
    );
}

/// ② 空轨（无片段、不进源集）上的逐件链：声明在、应用不在。
#[test]
fn lane_chains_on_an_unrendered_track_stay_unapplied() {
    let mut project = filled_project();
    let master = project.master_bus_track_id;
    let track_id = EntityId::new();
    let edge_id = EntityId::new();
    let mut track = TrackV3 {
        id: track_id,
        name: "Empty Lane Track".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.lane_chains = vec![LaneChainDef {
        keys: vec![42],
        name: "hats".to_owned(),
        devices: vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Channel Strip".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![ParameterValue {
                name: "hpf_hz".to_owned(),
                value: 400.0,
                unit: None,
            }],
            latency_samples: 0,
            plugin: None,
        }],
        pan: None,
    }];
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge_id,
        RoutingEdge {
            id: edge_id,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("空轨逐件链夹具必须合法");

    let mut dispatcher = dispatcher_with_project(project);
    let data = render_master(&mut dispatcher, "unapplied");
    assert_eq!(
        data["laneChainsDeclared"], 1,
        "工程里声明了一条逐件链: {data}"
    );
    assert_eq!(
        data["laneChainsUnapplied"], 1,
        "该轨没进本次渲染 ⇒ 不许报已应用: {data}"
    );
}
