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
//! | ⑨ | edm 自动化段落成混响湿电平泳道（点列逐项 + 求值对账） | `edm_generation_lands_reverb_wet_automation_lane` |
//! | ⑩ | 自动化点随 `startTick` 平移且身份确定性派生 | `edm_automation_points_shift_with_start_tick_and_stay_deterministic` |
//! | ⑪ | 无自动化段的风格不写泳道 | `style_without_automation_lands_no_lane` |

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
    // 电子族：无采样器，一台内置乐器设备，五个键位名参数齐全；
    // 其后是效果层。改钉依据（鼓机 v2）：trap 预置 fx 段带 delay
    // （trap.json: eq/comp + reverb + delay），drum_fx_device_specs
    // 按 strip → tape → reverb → delay 序产出，trap 无 tape，故效果层
    // 为通道条 + 混响 + 延迟，共 4 台器件（v1 期望 3 台系 v2 前旧值）。
    // 鼓总线器件挂在独立的总线轨上，不计入本轨。
    assert!(track.sampler.is_none());
    assert_eq!(track.devices.len(), 4);
    assert_eq!(track.devices[1].name, "Channel Strip");
    assert_eq!(track.devices[2].name, "Reverb");
    assert_eq!(track.devices[3].name, "Delay");
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
    // GM 键只来自 trap 预置的道。改钉依据（鼓机 v2）：trap.json 的道
    // 为 kick 36 / snare 38 / hihat 42 / crash 49 / shaker 82——v2 以
    // crash 道取代 v1 的开镲 46 道，故白名单 46 → 49（此断言原被同测
    // 的器件数断言挡在前面，改钉器件数后才暴露）。期 5 追加 35：
    // trap 新增 808 道（gm_key 35，steps 镜像 kick）承载出厂侧链。
    for note in notes.values() {
        assert!(
            [35_u8, 36, 38, 42, 49, 82].contains(&note.pitch),
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
    // 改钉依据（鼓机 v2 命名口径）：原声族预置名带 Acoustic 前缀
    // （rock.json name = "Acoustic Rock"，theory 判据
    // acoustic_factory_styles_carry_acoustic_names_and_chosen_programs
    // 钉住），轨道名 = format!("{} Drums", preset.name)。
    let track = drum_track(project, "Acoustic Rock Drums");
    let sampler = track.sampler.as_ref().expect("原声族必须绑采样器");
    assert_eq!(sampler.path, yeban_theory::ACOUSTIC_KIT_SFZ_PATH);
    // rock 的效果层：通道条 + 磁带 + 混响。
    let names: Vec<&str> = track.devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Channel Strip", "Tape Saturation", "Reverb"]);
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
        // 改钉依据同 acoustic_style_binds_sfz_sampler：boombap 预置名
        // 为 "Acoustic Boom Bap"（v2 Acoustic 前缀口径）。
        let track = drum_track(project, "Acoustic Boom Bap Drums");
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
    // 改钉依据（鼓机 v2）：出厂风格由 10 扩到 14（新增 jazz/funk/
    // rnb/hiphop），FACTORY_SOURCES 在 drum_preset.rs 钉为 14 条。
    assert_eq!(available.len(), 14);
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
    // 原有 PolySynth 设备不动、不绑采样器；其后补 rock 的效果层
    // （通道条 + 磁带 + 混响），再多一条摆放。
    assert_eq!(lead.devices.len(), 4);
    assert_eq!(lead.devices[0].name, "Yeban PolySynth");
    assert_eq!(lead.devices[1].name, "Channel Strip");
    assert_eq!(lead.devices[2].name, "Tape Saturation");
    assert_eq!(lead.devices[3].name, "Reverb");
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
    // 改钉依据（鼓机 v2）：lofi 已转电子族（lofi.json
    // kit_family = "electronic"），空轨装合成鼓机而非 SFZ 采样器：
    // sampler 恒空，首台器件为内置乐器（合成鼓组）。
    assert!(bass.sampler.is_none());
    let kit = bass.devices.first().expect("空轨须装默认鼓组器件");
    assert_eq!(kit.kind, yeban_model::DeviceKind::InternalInstrument);
    assert_eq!(kit.name, "Yeban Drum Kit (Synth)");
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

// ---------------------------------------------------------------------------
// 逐件链（lane_chains）判据（逐件处理线期 2）
// ---------------------------------------------------------------------------

#[test]
fn lane_chains_installed_on_new_track() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "house", "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    // 计划文本必须点名逐件链（声明落位要读得出来，不许静默装）。
    assert!(
        result.to_string().contains("逐件链"),
        "提案文本须点名逐件链: {result}"
    );
    let project = project(&dispatcher);
    let track = drum_track(project, "House Drums");
    assert_eq!(track.lane_chains.len(), 1, "house 预置带一条帽组逐件链");
    let hats = &track.lane_chains[0];
    assert_eq!(hats.name, "hats");
    assert_eq!(hats.keys, vec![42, 46]);
    assert_eq!(hats.devices.len(), 1, "帽组链 = 一台通道条");
    assert_eq!(hats.devices[0].name, "Channel Strip");
    assert_eq!(hats.devices[0].latency_samples, 0, "逐件链设备必须零延迟");
    let params: Vec<(&str, f32)> = hats.devices[0]
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.value))
        .collect();
    assert!(
        params.contains(&("hpf_hz", 400.0)),
        "帽组高通 400 Hz 必须落位: {params:?}"
    );
    assert!(
        params.contains(&("eq_high_gain", 2.0)),
        "帽组高频搁架 +2 dB 必须落位: {params:?}"
    );
}

#[test]
fn lane_chains_installed_on_existing_empty_track() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "trap", "trackId": BASS, "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let bass_id = EntityId::from_str(BASS).expect("ULID");
    let bass = project.tracks.get(&bass_id).expect("bass 轨");
    assert_eq!(bass.lane_chains.len(), 3, "trap 预置带三条逐件链（期 5）");
    // 期 5 出厂三组按首键升序：808（侧链压缩）/ kick（透明 EQ）/ hats。
    assert_eq!(bass.lane_chains[0].name, "bass");
    assert_eq!(bass.lane_chains[0].keys, vec![35]);
    assert_eq!(bass.lane_chains[1].name, "kick");
    assert_eq!(bass.lane_chains[1].keys, vec![36]);
    assert_eq!(bass.lane_chains[2].name, "hats");
    assert_eq!(bass.lane_chains[2].keys, vec![42]);
    // 出厂侧链边同批落位：同轨 Sidechain 边 source_lane=36 → dest_lane=35
    //（夹具工程自带其它侧链边，按端点 + lane 精确找本批这条）。
    let edge = project
        .routing_graph
        .edges
        .values()
        .find(|edge| {
            edge.kind == yeban_model::RoutingKind::Sidechain
                && edge.source_node == bass_id
                && edge.destination_node == bass_id
                && edge.dest_lane == Some(35)
        })
        .expect("出厂侧链边必须落位");
    assert_eq!(edge.source_lane, Some(36));
    assert_eq!(edge.dest_lane, Some(35));
}

#[test]
fn existing_track_with_user_lane_chains_keeps_them() {
    // 既有轨已有用户逐件链 ⇒ 生成不覆盖（R5），计划文本如实说明。
    let mut fixture = filled_project();
    let lead_id = EntityId::from_str(LEAD).expect("ULID");
    let user_chain = yeban_model::LaneChainDef {
        keys: vec![36],
        name: "mine".to_owned(),
        devices: Vec::new(),
        pan: Some(0.5),
        gain_db: None,
    };
    fixture
        .tracks
        .get_mut(&lead_id)
        .expect("lead 轨")
        .lane_chains = vec![user_chain.clone()];
    let mut dispatcher = dispatcher_with_project(fixture);
    let result = generate(
        &mut dispatcher,
        json!({ "style": "house", "trackId": LEAD, "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert!(
        result.to_string().contains("未覆盖"),
        "计划文本须说明未覆盖用户逐件链: {result}"
    );
    let project = project(&dispatcher);
    let lead = project.tracks.get(&lead_id).expect("lead 轨");
    assert_eq!(lead.lane_chains, vec![user_chain], "用户逐件链必须逐值保留");
}

#[test]
fn edm_generation_lands_reverb_wet_automation_lane() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "edm", "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    assert!(
        result.to_string().contains("reverb.reverb_wet"),
        "计划文本须提及落成的自动化段: {result}"
    );

    let project = project(&dispatcher);
    let track = drum_track(project, "EDM Big Room Drums");
    // 目标按轨上最终布局对账：混响器件槽位 + 其 reverb_wet 参数下标。
    let (slot_index, device) = track
        .devices
        .iter()
        .enumerate()
        .find(|(_, device)| device.name == "Reverb")
        .expect("混响器件在轨上");
    let param_index = device
        .params
        .iter()
        .position(|param| param.name == "reverb_wet")
        .expect("reverb_wet 参数在混响器件上");
    let target = yeban_model::AutomationTarget::DeviceParam {
        track_id: track.id,
        slot_index,
        param_index,
    };
    let lane = track
        .automation_lanes
        .get(&target)
        .expect("edm 的自动化泳道必须落成");
    assert!(lane.read_enabled, "生成泳道默认读开");
    assert_eq!(lane.write_mode, yeban_model::AutomationWriteMode::Off);
    let mut points: Vec<(u64, f32)> = lane.points.values().map(|p| (p.tick, p.value)).collect();
    points.sort_by_key(|(tick, _)| *tick);
    assert_eq!(points, vec![(0, 0.10), (3840, 0.15), (7680, 0.22)]);
    assert!(
        lane.points
            .values()
            .all(|p| p.curve == yeban_model::CurveType::SCurve),
        "段级 s_curve 逐点展开"
    );
    // 求值对账（模型唯一求值入口）：点上逐位精确、末点后保持、
    // 段内插值落在两端之间。
    let value_at = |tick: u64| {
        project
            .automation_value_at(&target, tick)
            .expect("目标对账通过")
    };
    assert_eq!(value_at(0), Some(0.10));
    assert_eq!(value_at(3840), Some(0.15));
    assert_eq!(value_at(7680), Some(0.22));
    assert_eq!(value_at(20_000), Some(0.22));
    let mid = value_at(1920).expect("段内应有自动化值");
    assert!(mid > 0.10 && mid < 0.15, "s_curve 插值应在两端之间: {mid}");
}

#[test]
fn edm_automation_points_shift_with_start_tick_and_stay_deterministic() {
    let run = |dispatcher: &mut Dispatcher| {
        let result = generate(
            dispatcher,
            json!({ "style": "edm", "startTick": 3840, "autoMerge": true }),
        );
        assert_eq!(result["status"], "success", "{result}");
        let project = project(dispatcher);
        let track = drum_track(project, "EDM Big Room Drums");
        let lane = track
            .automation_lanes
            .values()
            .next()
            .expect("自动化泳道必须落成");
        let mut points: Vec<(u64, String)> = lane
            .points
            .values()
            .map(|p| (p.tick, p.id.to_canonical_string()))
            .collect();
        points.sort_by_key(|(tick, _)| *tick);
        points
    };
    let mut first = dispatcher_with_project(filled_project());
    let mut second = dispatcher_with_project(filled_project());
    let points_first = run(&mut first);
    let points_second = run(&mut second);
    // 点 tick 按摆放起点平移（3840 + pattern 内坐标）。
    let ticks: Vec<u64> = points_first.iter().map(|(tick, _)| *tick).collect();
    assert_eq!(ticks, vec![3840, 7680, 11520]);
    // 同参数两次生成 ⇒ 点身份逐一相同（确定性派生）。
    assert_eq!(points_first, points_second);
}

#[test]
fn style_without_automation_lands_no_lane() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "trap", "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let track = drum_track(project, "Trap Drums");
    assert!(
        track.automation_lanes.is_empty(),
        "无 automation 段的风格不得写自动化泳道"
    );
}

/// ⭐ ADR-0078 / T-DRUM-9：生成工程端到端 —— lofi 生成后，离线延迟表必须把
/// 鼓轨磁带（往返 31 + 调制名义值 69 = 100）与鼓总线削波 4 倍（47）入账；
/// 生成器件的声明值仍全是 0（推导只在离线累加处发生，⛔ 不回写模型，
/// 实时计划不受影响 —— 引擎侧守卫在 `yeban-engine` 的 snapshot 判据）。
#[test]
fn generated_lofi_latencies_are_derived_in_the_offline_table() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let result = generate(
        &mut dispatcher,
        json!({ "style": "lofi", "autoMerge": true }),
    );
    assert_eq!(result["status"], "success", "{result}");
    let project = project(&dispatcher);
    let latencies = yeban_render::render::track_latencies(&project.tracks, 48_000);

    // 鼓轨按内容定位：带 `tape_drive` 参数的生成器件所在轨。
    let (drum_id, drum) = project
        .tracks
        .iter()
        .find(|(_, track)| {
            track
                .devices
                .iter()
                .any(|device| device.params.iter().any(|p| p.name == "tape_drive"))
        })
        .expect("lofi 生成工程必须有带磁带的鼓轨");
    assert_eq!(
        latencies[drum_id], 100,
        "鼓轨离线延迟 = 磁带往返 31 + wow/flutter 名义值 69（48 kHz）"
    );
    // 鼓总线按名定位（生成器以 `<Style> Drum Bus` 命名）。
    let (bus_id, bus) = project
        .tracks
        .iter()
        .find(|(_, track)| track.name.ends_with("Drum Bus"))
        .expect("lofi 生成工程必须有鼓总线轨");
    assert_eq!(
        latencies[bus_id], 47,
        "鼓总线离线延迟 = 软削波 4 倍过采样级联 47（质心 46.5 取整）"
    );
    // 守卫：两条轨上每台生成器件的声明值仍是 0。
    for track in [drum, bus] {
        for device in &track.devices {
            assert_eq!(
                device.latency_samples, 0,
                "生成器件声明不得被离线推导回写: {}",
                device.name
            );
        }
    }
}

/// T-FIX-37：鼓机逐件链为 lane 级侧链建 `source == destination == 鼓轨` 的
/// Sidechain 自环（语义合法：不同 lane 的键输入）。`detect_cycle` 不能把侧链边
/// 当音频边，否则生成鼓之后一切 `propose_section` 都被误判成环路而拒绝。
#[test]
fn hiphop_drum_sidechain_self_loop_does_not_block_propose_section() {
    let mut dispatcher = dispatcher_with_project(filled_project());
    let drum_result = generate(
        &mut dispatcher,
        json!({ "style": "hiphop", "autoMerge": true }),
    );
    assert_eq!(
        drum_result["status"], "success",
        "鼓生成必须成功: {drum_result}"
    );

    // 前提：确实建了 Sidechain 自环（source == destination == 同一轨）。
    let p = project(&dispatcher);
    let self_loop = p.routing_graph.edges.values().any(|e| {
        e.kind == yeban_model::RoutingKind::Sidechain && e.source_node == e.destination_node
    });
    assert!(
        self_loop,
        "hiphop 必须建 lane 侧链自环（否则本判据失去判别力）"
    );

    let section = call_tool(
        &mut dispatcher,
        "yeban_propose_section",
        json!({
            "sectionName": "Verse", "stylePreset": "synthwave", "bars": 4, "scale": "C minor"
        }),
    );
    assert_eq!(
        section["status"], "success",
        "生成鼓后的 propose_section 不得被侧链自环误判为环路: {section}"
    );
}
