//! 逐件链渲染回执判据（逐件处理线期 3）：`laneChainsDeclared` /
//! `laneChainsUnapplied` 必须说真话。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | 鼓机生成装上的逐件链在渲染后 `declared = 1`、`unapplied = 0`（真的建了分组源） | `generated_lane_chains_render_applied` |
//! | ② | 一条没有音符、没进渲染源集的轨上声明的逐件链 ⇒ `declared = 1`、`unapplied = 1`（不许冒充已应用） | `lane_chains_on_an_unrendered_track_stay_unapplied` |
//! | ③ | 出厂 trap/phonk 预置（bass 组以 kick 为键的同轨 lane 侧链）经 generate → render_master 端到端渲出非静音母带，lane 边被求值（`sidechainLaneEdgesEvaluated = 1`） | `factory_sidechain_styles_render_end_to_end` |

use std::fs;
use std::path::{Path, PathBuf};

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
        gain_db: None,
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
            source_lane: None,
            dest_lane: None,
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

/// 读一份 24-bit PCM 母带的峰值（全部声道 `max |sample|`；静音 = 恰 0，
/// 抖动噪声在 1e-7 量级 ⇒ 判据门 0.01 与两者都有数量级距离）。
fn wav_peak(path: &Path) -> f32 {
    let bytes = fs::read(path).expect("读母带");
    assert_eq!(&bytes[0..4], b"RIFF", "RIFF 头");
    assert_eq!(&bytes[8..12], b"WAVE", "WAVE 标记");
    let mut position = 12usize;
    let mut channels = 1usize;
    let mut data: &[u8] = &[];
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let length = u32::from_le_bytes(
            bytes[position + 4..position + 8]
                .try_into()
                .expect("chunk 长度 4 B"),
        ) as usize;
        let body = &bytes[position + 8..position + 8 + length];
        if id == b"fmt " {
            channels = usize::from(u16::from_le_bytes([body[2], body[3]])).max(1);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            assert_eq!(bits, 24, "MCP 母带是 24-bit PCM");
        }
        if id == b"data" {
            data = body;
        }
        position += 8 + length + (length & 1);
    }
    assert!(!data.is_empty(), "必须有 data 块");
    let mut peak = 0.0_f32;
    for frame in data.chunks_exact(3 * channels) {
        for channel in 0..channels {
            let at = channel * 3;
            let raw =
                i32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 2]]) >> 8;
            peak = peak.max((raw as f32 / 8_388_608.0).abs());
        }
    }
    peak
}

/// ③ 出厂侧链风格（trap/phonk）端到端：生成落的同轨 lane 侧链边
/// （bass 组键 35 的压缩以 kick 键 36 为键）在 `render_master` 里必须
/// 真的落到分组源上并渲出非静音母带。
///
/// 回归缺陷（2026-11-19 鼓机 loop 重渲线首次暴露）：`RenderPlan::execute`
/// 把已求值的 lane 键路由交给节点**最外层**源，而生产构造的包装层
/// （`PeakTap` / `TrackSource` / `MidiSource` 等）只转发 `seek_to`、
/// 不转发 `attach_lane_keys` ⇒ 特征缺省实现返回 `false` ⇒ 整次渲染
/// `RENDER_FAILED`（「该节点的源不是逐件分组源」），尽管最内层建的
/// 正是 `LaneSplitSource`。期 4/5 判据把分组源直接交给计划、绕过了
/// 包装层，所以没照到这条缝。
#[test]
fn factory_sidechain_styles_render_end_to_end() {
    for style in ["trap", "phonk"] {
        let mut dispatcher = dispatcher_with_project(filled_project());
        let generated = call_tool(
            &mut dispatcher,
            "yeban_generate_drum_pattern",
            json!({ "style": style, "autoMerge": true }),
        );
        assert_eq!(
            generated["status"], "success",
            "{style} 生成必须成功: {generated}"
        );
        let out = std::env::temp_dir().join(format!(
            "yeban-mcp-lane-render-{}-factory-{style}.wav",
            std::process::id()
        ));
        let result = call_tool(
            &mut dispatcher,
            "yeban_render_master",
            json!({
                "format": "wav",
                "sampleRate": 48000,
                "path": out.display().to_string(),
            }),
        );
        assert_eq!(
            result["status"], "success",
            "{style} 出厂侧链预置必须端到端渲染成功: {result}"
        );
        let data = &result["data"];
        assert_eq!(
            data["laneChainsDeclared"], 3,
            "{style} 出厂预置声明三条逐件链: {data}"
        );
        assert_eq!(
            data["laneChainsUnapplied"], 0,
            "{style} 鼓轨进了本次渲染且建了分组源 ⇒ 未应用必须归零: {data}"
        );
        assert_eq!(
            data["sidechainLaneEdgesEvaluated"], 1,
            "{style} 的同轨 lane 侧链边（kick 键 → bass 组）必须被求值: {data}"
        );
        let peak = wav_peak(&out);
        assert!(
            peak > 0.01,
            "{style} 母带不得静音（电子鼓组为合成音源、无样本依赖），实测峰值 {peak}"
        );
        let _ = fs::remove_file(&out);
    }
}
