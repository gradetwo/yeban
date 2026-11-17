//! `yeban_generate_drum_pattern` 的承重判据（鼓机一期，MCP 入口）。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | 缺省建新鼓轨 + 电子默认鼓组（五键 GM 映射齐全）+ 片段落轨 | `generates_into_new_track_with_synth_kit` |
//! | ② | 原声风格绑 SFZ 采样器（Virtuosity 基础组路径） | `acoustic_style_binds_sfz_sampler` |
//! | ③ | 同种子同参数两次生成 ⇒ 片段逐字节一致 | `same_seed_byte_identical_clip` |
//! | ④ | 生成音符与 theory 生成层逐项一致（tick/键/力度/时值） | `notes_match_theory_generation` |
//! | ⑤ | 未知风格 ⇒ `STYLE_NOT_FOUND` + `availableStyles` | `unknown_style_is_style_not_found` |
//! | ⑥ | 既有轨已有音源 ⇒ 鼓组不动、只落片段 | `existing_track_with_instrument_keeps_kit` |
//! | ⑦ | 既有空轨 ⇒ 装默认鼓组再落片段 | `existing_empty_track_gets_default_kit` |
//! | ⑧ | 旋钮越界 ⇒ `INVALID_PARAMETER_RANGE` | `knob_out_of_range_is_rejected` |

use std::path::PathBuf;
use std::str::FromStr as _;

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{ClipContent, EntityId, YebanProjectV1};

/// `filled_project` 的 lead 轨（带 PolySynth 设备）。
const LEAD: &str = "01J8ZQ00000000000000000002";
/// `filled_project` 的 bass 轨（无设备、无采样器）。
const BASS: &str = "01J8ZQ00000000000000000003";

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
            "yeban-mcp-drum-gen-{}-{tag}-{}",
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
        "`{name}` 不得产出 JSON-RPC 层错误（领域失败走 ToolResponse）: {outcome:?}"
    );
    let response = outcome.response.expect("响应");
    response.result.expect("result")
}

fn generate(dispatcher: &mut Dispatcher, arguments: Value) -> Value {
    call_tool(dispatcher, "yeban_generate_drum_pattern", arguments)
}

fn project(dispatcher: &Dispatcher) -> &YebanProjectV1 {
    dispatcher.domain().active_project().expect("活跃工程")
}

/// 找出名为 `<Style> Drums` 的新轨（生成工具建的鼓轨）。
fn drum_track<'p>(project: &'p YebanProjectV1, name: &str) -> &'p yeban_model::TrackV3 {
    project
        .tracks
        .values()
        .find(|track| track.name == name)
        .expect("鼓轨存在")
}

#[test]
fn generates_into_new_track_with_synth_kit() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "trap", "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["data"]["autoMerged"], true, "{result}");

    let project = project(&dispatcher);
    let track = drum_track(project, "Trap Drums");
    // 电子族：无采样器，一台内置乐器设备，五个键位名参数齐全。
    assert!(track.sampler.is_none());
    assert_eq!(track.devices.len(), 1);
    let device = &track.devices[0];
    assert_eq!(device.kind, yeban_model::DeviceKind::InternalInstrument);
    let names: Vec<&str> = device.params.iter().map(|p| p.name.as_str()).collect();
    for key in [
        "kick_note",
        "snare_note",
        "closed_hat_note",
        "open_hat_note",
        "clap_note",
    ] {
        assert!(names.contains(&key), "缺键位参数 {key}: {names:?}");
    }
    // 片段落轨：一条摆放，时长 = 4 小节。
    assert_eq!(track.clips.len(), 1);
    let placement = track.clips.values().next().expect("摆放");
    assert_eq!(placement.start_tick, 0);
    assert_eq!(placement.duration_ticks, 4 * 3840);
    let clip = project
        .clip_pool
        .get(&placement.clip_id)
        .expect("片段在池里");
    let ClipContent::Midi { notes } = &clip.content else {
        panic!("片段必须是 MIDI");
    };
    assert!(!notes.is_empty());
    // GM 键只来自 trap 预置的道（36/38/42/46/82）。
    for note in notes.values() {
        assert!(
            [36_u8, 38, 42, 46, 82].contains(&note.pitch),
            "非法 GM 键 {}",
            note.pitch
        );
    }
}

#[test]
fn acoustic_style_binds_sfz_sampler() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "rock", "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let track = drum_track(project, "Rock Drums");
    let sampler = track.sampler.as_ref().expect("原声族必须绑采样器");
    assert_eq!(sampler.path, yeban_theory::ACOUSTIC_KIT_SFZ_PATH);
    assert!(track.devices.is_empty());
}

#[test]
fn same_seed_byte_identical_clip() {
    let args = json!({
        "style": "boombap", "seed": 7, "complexity": 90, "intensity": 65,
        "bars": 4, "autoMerge": true,
    });
    let mut a = dispatcher_with_project(filled_project());
    let mut b = dispatcher_with_project(filled_project());
    assert_eq!(generate(&mut a, args.clone())["status"], "success");
    assert_eq!(generate(&mut b, args)["status"], "success");
    let clip_bytes = |dispatcher: &Dispatcher| -> String {
        let project = project(dispatcher);
        let track = drum_track(project, "Boom Bap Drums");
        let placement = track.clips.values().next().expect("摆放");
        let clip = project.clip_pool.get(&placement.clip_id).expect("片段");
        serde_json::to_string(clip).expect("片段可序列化")
    };
    assert_eq!(clip_bytes(&a), clip_bytes(&b));
}

#[test]
fn notes_match_theory_generation() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({
            "style": "house", "seed": 3, "complexity": 50, "intensity": 80,
            "bars": 2, "autoMerge": true,
        }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let track = drum_track(project, "House Drums");
    let placement = track.clips.values().next().expect("摆放");
    let clip = project.clip_pool.get(&placement.clip_id).expect("片段");
    let ClipContent::Midi { notes } = &clip.content else {
        panic!("片段必须是 MIDI");
    };
    let preset = yeban_theory::factory_drum_style("house")
        .expect("预置可解析")
        .expect("house 存在");
    let expected = yeban_theory::generate_drum_pattern(
        &preset,
        &yeban_theory::DrumGenParams {
            complexity: 50,
            intensity: 80,
            swing_permille: None,
            seed: 3,
            bars: 2,
            fill: true,
        },
    )
    .expect("theory 生成");
    assert_eq!(notes.len(), expected.len());
    let mut got: Vec<(u64, u8, u8, u64)> = notes
        .values()
        .map(|n| (n.start_tick, n.pitch, n.velocity, n.duration_ticks))
        .collect();
    got.sort();
    let mut want: Vec<(u64, u8, u8, u64)> = expected
        .iter()
        .map(|n| (n.tick, n.gm_key, n.velocity, n.duration_ticks))
        .collect();
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn unknown_style_is_style_not_found() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(&mut dispatcher, json!({ "style": "klingon" }));
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(result["error"]["code"], "STYLE_NOT_FOUND", "{result}");
    let available = result["error"]["data"]["availableStyles"]
        .as_array()
        .expect("availableStyles 数组");
    assert_eq!(available.len(), 10);
}

#[test]
fn existing_track_with_instrument_keeps_kit() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "rock", "trackId": LEAD, "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let lead_id = EntityId::from_str(LEAD).expect("ULID");
    let lead = project.tracks.get(&lead_id).expect("lead 轨");
    // 原有 PolySynth 设备不动、不绑采样器；只多一条摆放。
    assert_eq!(lead.devices.len(), 1);
    assert_eq!(lead.devices[0].name, "Yeban PolySynth");
    assert!(lead.sampler.is_none());
    assert!(
        lead.clips
            .values()
            .any(|p| project.clip_pool.contains_key(&p.clip_id))
    );
}

#[test]
fn existing_empty_track_gets_default_kit() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "lofi", "trackId": BASS, "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let bass_id = EntityId::from_str(BASS).expect("ULID");
    let bass = project.tracks.get(&bass_id).expect("bass 轨");
    // lofi 是原声族：空轨装 SFZ 默认鼓组。
    assert_eq!(
        bass.sampler.as_ref().map(|s| s.path.as_str()),
        Some(yeban_theory::ACOUSTIC_KIT_SFZ_PATH)
    );
}

#[test]
fn knob_out_of_range_is_rejected() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "trap", "complexity": 101 }),
    );
    assert_eq!(result["status"], "error", "{result}");
    assert_eq!(
        result["error"]["code"], "INVALID_PARAMETER_RANGE",
        "{result}"
    );
}
