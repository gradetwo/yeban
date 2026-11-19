//! D3/D4 接线判据（鼓机缺口线）：瞬态塑形与 Bitcrusher 进插入链后的
//! **音频级**证据——器件挂上轨链/逐件链真的改变母带样本，而不是只在
//! 计划里出现。定值与出厂预置同源：funk 的 `fx.transient`
//! （attack 0.45 / sustain 0.15）与 lofi 的 `fx.crusher`
//! （bits 10 / down 2 / aa 0.35）。
//!
//! 夹具 = 一条合成鼓机轨（120 BPM / 960 PPQ / 48 kHz ⇒ 1 tick = 25 帧，
//! 3840 tick = 96 000 帧）：kick ×4 正拍、snare ×4 反拍、hat ×8 八分。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | funk 瞬态定值挂轨链：与无器件对照逐样本不同且 RMS 有别 | `funk_transient_values_change_the_track_render` |
//! | ② | lofi 碾碎定值挂轨链：与无器件对照大面积不同 | `lofi_crusher_values_change_the_track_render` |
//! | ③ | 中性瞬态（0,0）逐位恒等（恒等门）；碾碎缺省（16,1,0）非恒等（16 位量化真改低位） | `neutral_transient_is_bit_identical_and_default_crusher_is_not` |
//! | ④ | 碾碎挂 hat 逐件链：首个 hat 之前的 kick 段逐位不变、hat 段有别 | `crusher_on_a_lane_chain_changes_only_that_lanes_region` |

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId,
    LaneChainDef, LoopConfig, MidiNote, ParameterValue, RoutingEdge, RoutingKind, SampleRate,
    TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::engine_source::EngineTimbre;
use yeban_render::inserts::project_inserts;
use yeban_render::render::{AudioSource, RenderOptions, RenderPlan};

const SR: u32 = 48_000;
const FRAMES: u64 = 96_000;

fn id(seed: usize) -> EntityId {
    EntityId::from_str(&format!("01J8ZN{seed:020}")).expect("夹具 ULID 必须合法")
}

fn param(name: &str, value: f32) -> ParameterValue {
    ParameterValue {
        name: name.to_owned(),
        value,
        unit: None,
    }
}

fn effect_device(name: &str, params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params.iter().map(|(n, v)| param(n, *v)).collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// 合成鼓机设备（键位 + kick 调音；snare/hat 走器件缺省，与期 4 夹具同口径）。
fn drum_device() -> DeviceDefinition {
    let params = [
        ("kick_note", 36.0),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
        ("kick_tune_hz", 55.0),
        ("kick_glide_hz", 120.0),
        ("kick_glide_s", 0.02),
        ("kick_attack_s", 0.001),
        ("kick_decay_s", 0.08),
        ("kick_level", 1.0),
        ("kick_pan", 0.0),
        ("hihat_highpass_hz", 1_500.0),
        ("hihat_lowpass_hz", 9_000.0),
    ];
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drum Kit (Synth)".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: params.iter().map(|(n, v)| param(n, *v)).collect(),
        latency_samples: 0,
        plugin: None,
    }
}

const NOTES: &[(u64, u8, u64)] = &[
    (0, 36, 240),
    (960, 36, 240),
    (1_920, 36, 240),
    (2_880, 36, 240),
    (480, 38, 240),
    (1_440, 38, 240),
    (2_400, 38, 240),
    (3_360, 38, 240),
    (240, 42, 120),
    (720, 42, 120),
    (1_200, 42, 120),
    (1_680, 42, 120),
    (2_160, 42, 120),
    (2_640, 42, 120),
    (3_120, 42, 120),
    (3_600, 42, 120),
];

fn project(fx: Vec<DeviceDefinition>, chains: Vec<LaneChainDef>) -> (YebanProjectV1, EntityId) {
    let master = id(1);
    let track = id(100);
    let clip = id(1_000);
    let placement = id(2_000);
    let mut project = YebanProjectV1 {
        title: "shaping chain fixture".to_owned(),
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
    project.routing_graph.nodes.push(master);
    let mut drum_track = TrackV3 {
        id: track,
        name: "Drums".to_owned(),
        kind: TrackKind::Midi,
        lane_chains: chains,
        ..TrackV3::default()
    };
    drum_track.devices.push(drum_device());
    drum_track.devices.extend(fx);
    drum_track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 3_840,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    project.tracks.insert(track, drum_track);
    let mut note_map = BTreeMap::new();
    for (index, &(start, pitch, duration)) in NOTES.iter().enumerate() {
        let note = id(3_000 + index);
        note_map.insert(note, MidiNote::new(note, start, pitch, duration));
    }
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes: note_map },
        },
    );
    project.routing_graph.nodes.push(track);
    let audio_edge = id(4_000);
    project.routing_graph.edges.insert(
        audio_edge,
        RoutingEdge {
            id: audio_edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("shaping 夹具必须合法: {error}"));
    (project, track)
}

fn render(project: &YebanProjectV1, track: EntityId) -> Vec<f32> {
    let timbre = EngineTimbre::from_project(project).expect("夹具必须成快照");
    let options = RenderOptions::l1(FRAMES, 2, SR, 0x5EED).with_threads(1);
    // 轨级插入链走计划的 inserts 入口（`project_inserts` 投影轨上器件）；
    // 逐件链由分组源内部施加，两条路与导出路径同一份代码。
    let inserts = project_inserts(&project.tracks, SR, project.bpm);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &project.routing_graph,
        project.master_bus_track_id,
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("计划必须能编译");
    let chains = &project.tracks[&track].lane_chains;
    let source: Box<dyn AudioSource> = if chains.is_empty() {
        Box::new(timbre.track_source(track).expect("轨源必须能武装"))
    } else {
        Box::new(
            timbre
                .lane_track_source(track, chains, project.bpm)
                .expect("分组源必须能武装"),
        )
    };
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(track, source);
    plan.execute(sources).expect("渲染必须成功").samples
}

fn differing(a: &[f32], b: &[f32]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

fn rms_db(samples: &[f32]) -> f64 {
    let sum: f64 = samples.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    10.0 * (sum / samples.len() as f64).log10()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, v| acc.max(v.abs()))
}

/// ① funk 的瞬态定值（attack 0.45 / sustain 0.15）挂轨链必须真改声。
#[test]
fn funk_transient_values_change_the_track_render() {
    let (plain_project, track) = project(Vec::new(), Vec::new());
    let plain = render(&plain_project, track);
    let (funk_project, track) = project(
        vec![effect_device(
            "Transient Shaper",
            &[("transient_attack", 0.45), ("transient_sustain", 0.15)],
        )],
        Vec::new(),
    );
    let shaped = render(&funk_project, track);
    let diff = differing(&plain, &shaped);
    eprintln!(
        "funk transient: differing {diff}/{} samples, rms {:+.3} -> {:+.3} dB, peak {:.4} -> {:.4}",
        plain.len(),
        rms_db(&plain),
        rms_db(&shaped),
        peak(&plain),
        peak(&shaped)
    );
    assert!(
        diff > plain.len() / 20,
        "瞬态定值应改动超过 5% 的样本，实测 {diff}/{}",
        plain.len()
    );
    assert!(
        (rms_db(&shaped) - rms_db(&plain)).abs() > 0.05,
        "RMS 应有可测变化：{:.3} -> {:.3} dB",
        rms_db(&plain),
        rms_db(&shaped)
    );
}

/// ② lofi 的碾碎定值（bits 10 / down 2 / aa 0.35）挂轨链必须大面积改声。
#[test]
fn lofi_crusher_values_change_the_track_render() {
    let (plain_project, track) = project(Vec::new(), Vec::new());
    let plain = render(&plain_project, track);
    let (lofi_project, track) = project(
        vec![effect_device(
            "Bitcrusher",
            &[
                ("crush_bits", 10.0),
                ("crush_down", 2.0),
                ("crush_aa", 0.35),
            ],
        )],
        Vec::new(),
    );
    let crushed = render(&lofi_project, track);
    let diff = differing(&plain, &crushed);
    eprintln!(
        "lofi crusher: differing {diff}/{} samples, rms {:+.3} -> {:+.3} dB, peak {:.4} -> {:.4}",
        plain.len(),
        rms_db(&plain),
        rms_db(&crushed),
        peak(&plain),
        peak(&crushed)
    );
    assert!(
        diff > plain.len() / 2,
        "碾碎定值应改动过半样本，实测 {diff}/{}",
        plain.len()
    );
}

/// ③ 中性瞬态逐位恒等（恒等门在渲染级成立）；碾碎缺省非恒等（量化真改低位，
/// 故碾碎不设恒等门——本判据把这条不对称钉死）。
#[test]
fn neutral_transient_is_bit_identical_and_default_crusher_is_not() {
    let (plain_project, track) = project(Vec::new(), Vec::new());
    let plain = render(&plain_project, track);
    let (neutral_project, track) = project(
        vec![effect_device(
            "Transient Shaper",
            &[("transient_attack", 0.0), ("transient_sustain", 0.0)],
        )],
        Vec::new(),
    );
    let neutral = render(&neutral_project, track);
    assert_eq!(
        differing(&plain, &neutral),
        0,
        "中性瞬态 (0,0) 必须逐位恒等（恒等门）"
    );
    let (default_project, track) = project(
        vec![effect_device("Bitcrusher", &[("crush_bits", 16.0)])],
        Vec::new(),
    );
    let defaulted = render(&default_project, track);
    let diff = differing(&plain, &defaulted);
    eprintln!("crusher default (16,1,0): differing {diff}/{}", plain.len());
    assert!(
        diff > 0,
        "碾碎缺省 (16,1,0) 的 16 位量化必须真改样本（无恒等门的设计依据）"
    );
}

/// ④ 碾碎挂在 hat 逐件链上：只改 hat 组——首个 hat（tick 240 ⇒ 帧 6000）
/// 之前只有 kick 在响，该段必须逐位不变；其后必须有别。
#[test]
fn crusher_on_a_lane_chain_changes_only_that_lanes_region() {
    let hat_chain = |devices: Vec<DeviceDefinition>| LaneChainDef {
        keys: vec![42],
        name: "hats".to_owned(),
        devices,
        pan: None,
        gain_db: None,
    };
    let (plain_project, track) = project(Vec::new(), vec![hat_chain(Vec::new())]);
    let plain = render(&plain_project, track);
    let (crushed_project, track) = project(
        Vec::new(),
        vec![hat_chain(vec![effect_device(
            "Bitcrusher",
            &[("crush_bits", 8.0), ("crush_down", 4.0), ("crush_aa", 0.0)],
        )])],
    );
    let crushed = render(&crushed_project, track);
    let first_hat_frame = 240 * 25;
    assert_eq!(
        differing(
            &plain[..first_hat_frame * 2],
            &crushed[..first_hat_frame * 2]
        ),
        0,
        "首个 hat 之前只有 kick 在响，逐件链不得碰它"
    );
    let tail_diff = differing(
        &plain[first_hat_frame * 2..],
        &crushed[first_hat_frame * 2..],
    );
    eprintln!(
        "lane crusher: tail differing {tail_diff}/{} samples",
        plain.len() - first_hat_frame * 2
    );
    assert!(tail_diff > 0, "hat 段必须被逐件链上的碾碎改动");
}
