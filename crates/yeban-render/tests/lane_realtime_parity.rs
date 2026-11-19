//! lane 逐件链 + 侧链键控的**实时 vs 离线一致性**判据（逐件＋侧链期 5）。
//!
//! 夹具与期 4 的 `lane_sidechain_audio` **同一份工程**（合成鼓机轨，
//! 120 BPM / 960 PPQ / 48 kHz ⇒ 1 tick = 25 帧）：
//!
//! ```text
//!   808 (键 35, 长音) ── bass 组: 压缩器（阈值 −40 dB / 8:1）
//!   kick (键 36)      ── kick 组: 无链 —— 侧链边的 key 源（链前抽头）
//!   hat (键 42)       ── hats 组: 高通（隔离判据用）
//!   边: Sidechain(source_lane=36, dest_lane=35)，同轨
//! ```
//!
//! 同一工程走两条产品路径：离线 = `RenderPlan` + `lane_track_source`；
//! 实时 = `EngineSnapshot::from_project` → `EngineRuntime::process_quantum`。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | 验收例在两条路径都成立（双差分闪避 ≥ 6 dB），且两路的闪避量之差有界（实测钉值） | `kick_ducks_808_in_realtime_and_offline` |
//! | ② | hat 组高通不影响 kick：实时侧首个 hat 之前逐位相同、hat 段本身有别 | `realtime_hat_highpass_does_not_touch_kick` |
//! | ③ | 实时响亮读数：lane 边已求值计数、组链处理帧数；无边工程两项皆为 0 | `realtime_lane_stats_are_loud` |

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId,
    LaneChainDef, LoopConfig, MidiNote, ParameterValue, RoutingEdge, RoutingKind, SampleRate,
    TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::automation::lane_key_layouts;
use yeban_render::engine_source::EngineTimbre;
use yeban_render::render::{AudioSource, RenderOptions, RenderPlan};

const SR: u32 = 48_000;
const FRAMES: u64 = 96_000;
const QUANTA: usize = 750; // 96_000 帧 / 128 帧每量子

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

/// 合成鼓机设备（与期 4 夹具同定值：808 衰减 2.5 s 的持续低音）。
fn drum_device() -> DeviceDefinition {
    let params = [
        ("kick_note", 36.0),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
        ("bass808_note", 35.0),
        ("bass808_tune_hz", 49.0),
        ("bass808_glide_hz", 110.0),
        ("bass808_glide_s", 0.05),
        ("bass808_attack_s", 0.002),
        ("bass808_decay_s", 2.5),
        ("bass808_level", 0.95),
        ("bass808_pan", 0.0),
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
        params: params
            .iter()
            .map(|(name, value)| param(name, *value))
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// bass 组的压缩通道条（与期 4 夹具同定值）。
fn comp_strip() -> DeviceDefinition {
    let params = [
        ("compressor_enabled", 1.0),
        ("threshold_db", -40.0),
        ("ratio", 8.0),
        ("knee_db", 0.0),
        ("detector_s", 0.01),
        ("attack_s", 0.001),
        ("release_s", 0.25),
        ("makeup_db", 0.0),
    ];
    DeviceDefinition {
        id: EntityId::new(),
        name: "Lane Strip".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| param(name, *value))
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// hats 组的高通通道条（6 kHz 低切；与期 4 夹具同定值）。
fn hpf_strip() -> DeviceDefinition {
    DeviceDefinition {
        params: vec![param("hpf_hz", 6_000.0)],
        ..comp_strip()
    }
}

fn chain(keys: &[u8], name: &str, devices: Vec<DeviceDefinition>) -> LaneChainDef {
    LaneChainDef {
        keys: keys.to_vec(),
        name: name.to_owned(),
        devices,
        pan: None,
        gain_db: None,
    }
}

/// 标准三组：bass[35]+压缩、kick[36] 无链、hats[42]（设备由参数给）。
fn standard_chains(hat_devices: Vec<DeviceDefinition>) -> Vec<LaneChainDef> {
    vec![
        chain(&[35], "bass", vec![comp_strip()]),
        chain(&[36], "kick", Vec::new()),
        chain(&[42], "hats", hat_devices),
    ]
}

const NOTES: &[(u64, u8, u64)] = &[
    (0, 35, 3_840), // 808 长音（整段持续）
    (0, 36, 240),   // kick ×4（每 960 tick 一拍）
    (960, 36, 240),
    (1_920, 36, 240),
    (2_880, 36, 240),
    (480, 42, 120), // hat ×4（反拍）
    (1_440, 42, 120),
    (2_400, 42, 120),
    (3_360, 42, 120),
];

fn project(
    notes: &[(u64, u8, u64)],
    chains: Vec<LaneChainDef>,
    sidechain: Option<(u8, u8)>,
) -> (YebanProjectV1, EntityId) {
    let master = id(1);
    let track = id(100);
    let clip = id(1_000);
    let placement = id(2_000);
    let mut project = YebanProjectV1 {
        title: "lane realtime parity fixture".to_owned(),
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
    for (index, &(start, pitch, duration)) in notes.iter().enumerate() {
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
    if let Some((source_lane, dest_lane)) = sidechain {
        let edge = id(4_001);
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: track,
                destination_node: track,
                kind: RoutingKind::Sidechain,
                gain_db: None,
                source_lane: Some(source_lane),
                dest_lane: Some(dest_lane),
            },
        );
    }
    project
        .validate()
        .unwrap_or_else(|error| panic!("lane 一致性夹具必须合法: {error}"));
    (project, track)
}

/// 离线渲染（期 4 同路）：返回（母带交错样本, lane 已求值边数）。
fn render_offline(project: &YebanProjectV1, track: EntityId) -> (Vec<f32>, usize) {
    let timbre = EngineTimbre::from_project(project).expect("夹具必须成快照");
    let options = RenderOptions::l1(FRAMES, 2, SR, 0x5EED).with_threads(1);
    let mut plan =
        RenderPlan::compile(&project.routing_graph, project.master_bus_track_id, options)
            .expect("计划必须能编译");
    plan.attach_lane_key_routes(&lane_key_layouts(project, SR));
    let evaluated = plan.lane_key_evaluated_edge_ids().len();
    let chains = &project.tracks[&track].lane_chains;
    let source = timbre
        .lane_track_source(track, chains, project.bpm)
        .expect("分组源必须能武装");
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(track, Box::new(source));
    let output = plan.execute(sources).expect("渲染必须成功");
    (output.samples, evaluated)
}

/// 实时渲染（产品路径）：快照 → `EngineRuntime::process_quantum` 逐量子。
/// 返回（母带交错样本, 结束时引擎统计）。
fn render_realtime(project: &YebanProjectV1) -> (Vec<f32>, EngineStats) {
    let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具必须成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut samples = Vec::with_capacity(QUANTA * 128 * 2);
    let mut out = vec![0.0f32; 128 * 2];
    for _ in 0..QUANTA {
        out.fill(0.0);
        runtime.process_quantum(&mut out, 2);
        samples.extend_from_slice(&out);
    }
    (samples, runtime.stats())
}

fn rms_db(samples: &[f32], frames: std::ops::Range<usize>) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for frame in frames {
        for channel in 0..2 {
            let value = f64::from(samples[frame * 2 + channel]);
            sum += value * value;
            count += 1;
        }
    }
    10.0 * (sum / count as f64).log10()
}

/// 双差分闪避量（期 4 同量法）：键控渲染与无边对照各自的「近窗比远窗低
/// 多少」之差。kick 落点取第 2、3 拍（24_000 / 48_000 帧）。
fn ducking(samples_keyed: &[f32], samples_plain: &[f32]) -> (f64, f64) {
    let mut ducks = Vec::new();
    for onset in [24_000usize, 48_000] {
        let near = onset + 240..onset + 1_200;
        let far = onset + 16_800..onset + 19_200;
        let mod_keyed = rms_db(samples_keyed, far.clone()) - rms_db(samples_keyed, near.clone());
        let mod_plain = rms_db(samples_plain, far) - rms_db(samples_plain, near);
        ducks.push(mod_keyed - mod_plain);
    }
    (ducks[0], ducks[1])
}

/// ① 验收例（kick 闪避 808）在实时与离线两条路径都成立，且两路闪避量之差有界。
#[test]
fn kick_ducks_808_in_realtime_and_offline() {
    let (keyed_project, track) = project(NOTES, standard_chains(Vec::new()), Some((36, 35)));
    let (plain_project, _) = project(NOTES, standard_chains(Vec::new()), None);

    let (offline_keyed, evaluated) = render_offline(&keyed_project, track);
    assert_eq!(evaluated, 1, "离线：同轨 lane 边必须被求值");
    let (offline_plain, _) = render_offline(&plain_project, track);
    let (rt_keyed, stats) = render_realtime(&keyed_project);
    let (rt_plain, _) = render_realtime(&plain_project);

    assert!(
        rt_keyed.iter().any(|sample| *sample != 0.0),
        "实时渲染必须有声（夹具/驱动不得空转）"
    );
    assert_eq!(
        stats.lane_key_edges_evaluated, 1,
        "实时：同轨 lane 边必须被求值（响亮读数）"
    );

    let (off_1, off_2) = ducking(&offline_keyed, &offline_plain);
    let (rt_1, rt_2) = ducking(&rt_keyed, &rt_plain);
    println!("闪避双差分：离线 {off_1:.2} / {off_2:.2} dB；实时 {rt_1:.2} / {rt_2:.2} dB");
    for (label, value) in [
        ("离线@24000", off_1),
        ("离线@48000", off_2),
        ("实时@24000", rt_1),
        ("实时@48000", rt_2),
    ] {
        assert!(
            value >= 6.0,
            "{label}：kick 必须把 808 压出 ≥ 6 dB 的锁定凹口，实测 {value:.2} dB"
        );
    }
    // 两路之差的有界钉：两条路径的器件与求值语义同一套，差只来自求和/
    // 量化路径的既有差异。首测实测（盒 yanshi-2，2026-11-19）：两路打印值
    // 在 0.01 dB 精度内完全一致（15.15 / 6.12 dB）；界值取 0.5 dB ——
    // 对实测差（<0.01 dB）留约 50 倍余量，覆盖跨机浮点尾差，又远小于
    // 「闪避成立」的 6 dB 门槛，不会把真分叉放过去。
    for (label, diff) in [
        ("@24000", (rt_1 - off_1).abs()),
        ("@48000", (rt_2 - off_2).abs()),
    ] {
        assert!(
            diff <= 0.5,
            "实时与离线的闪避量{label}相差 {diff:.2} dB，超过一致性界 0.5 dB"
        );
    }
}

/// ② hat 组高通不影响 kick（实时）：首个 hat 之前与无高通渲染逐位相同；
/// hat 段本身必须有别（高通真生效，判据不空转）。
#[test]
fn realtime_hat_highpass_does_not_touch_kick() {
    let notes: &[(u64, u8, u64)] = &[(0, 36, 240), (480, 46, 960), (1_920, 36, 240)];
    let chains = |hat_devices: Vec<DeviceDefinition>| {
        vec![
            chain(&[36], "kick", Vec::new()),
            chain(&[42, 46], "hats", hat_devices),
        ]
    };
    let (with_hpf, _) = project(notes, chains(vec![hpf_strip()]), None);
    let (without_hpf, _) = project(notes, chains(Vec::new()), None);
    let (keyed, _) = render_realtime(&with_hpf);
    let (plain, _) = render_realtime(&without_hpf);
    // 首个 hat 在 tick 480 ⇒ 帧 12_000；其前只有 kick（与 808 无关段）。
    let split = 12_000 * 2;
    assert_eq!(
        keyed[..split],
        plain[..split],
        "实时：首个 hat 之前，高通组不得改动 kick 一个样本"
    );
    assert_ne!(
        keyed[split..],
        plain[split..],
        "实时：hat 段必须被高通改动（否则判据空转）"
    );
}

/// ④ 组增益双路对拍（P4）：hats 组 `gain_db = -6` —— 实时与离线两路
/// 都必须满足：首个 hat 之前（纯 kick 段）与无增益渲染逐位相同；
/// 纯 hat 段 RMS 恰降 6 dB（容差 0.75 dB，同 `db_to_gain` 近似口径）。
#[test]
fn lane_group_gain_applies_in_realtime_and_offline() {
    let gained_chains = |gain: Option<f32>| {
        let mut hats = chain(&[42, 46], "hats", Vec::new());
        hats.gain_db = gain;
        vec![chain(&[36], "kick", Vec::new()), hats]
    };
    // A) kick + hats：首个 hat（tick 480 ⇒ 帧 12_000）之前不得动 kick。
    let notes: &[(u64, u8, u64)] = &[(0, 36, 240), (480, 46, 960), (1_920, 36, 240)];
    let (gained, track) = project(notes, gained_chains(Some(-6.0)), None);
    let (plain, _) = project(notes, gained_chains(None), None);
    let split = 12_000 * 2;
    let (rt_gained, _) = render_realtime(&gained);
    let (rt_plain, _) = render_realtime(&plain);
    assert_eq!(
        rt_gained[..split],
        rt_plain[..split],
        "实时：hat 组增益不得改动首个 hat 之前的 kick 样本"
    );
    assert_ne!(
        rt_gained[split..],
        rt_plain[split..],
        "实时：hat 段必须被增益改动（否则判据空转）"
    );
    let (off_gained, _) = render_offline(&gained, track);
    let (off_plain, _) = render_offline(&plain, track);
    assert_eq!(
        off_gained[..split],
        off_plain[..split],
        "离线：hat 组增益不得改动首个 hat 之前的 kick 样本"
    );
    // B) 纯 hats：整段 RMS 的增益量双路都必须 ≈ −6 dB。
    let hats_only: &[(u64, u8, u64)] = &[
        (480, 42, 120),
        (1_440, 42, 120),
        (2_400, 42, 120),
        (3_360, 42, 120),
    ];
    let (gained, track) = project(hats_only, gained_chains(Some(-6.0)), None);
    let (plain, _) = project(hats_only, gained_chains(None), None);
    let win = 12_000..96_000;
    let (off_gained, _) = render_offline(&gained, track);
    let (off_plain, _) = render_offline(&plain, track);
    let (rt_gained, _) = render_realtime(&gained);
    let (rt_plain, _) = render_realtime(&plain);
    let off_delta = rms_db(&off_gained, win.clone()) - rms_db(&off_plain, win.clone());
    let rt_delta = rms_db(&rt_gained, win.clone()) - rms_db(&rt_plain, win);
    println!("组增益双路实测：离线 {off_delta:.3} dB；实时 {rt_delta:.3} dB（目标 -6）");
    for (label, delta) in [("离线", off_delta), ("实时", rt_delta)] {
        assert!(
            (delta + 6.0).abs() <= 0.75,
            "{label}：-6 dB 组增益实测 {delta:.3} dB"
        );
    }
}

/// ③ 实时响亮读数：有边工程的已求值计数与组链帧数必须为正；无边工程
/// 已求值必须为 0（组链仍在跑 ⇒ 帧数仍为正，两件事分开记账）。
#[test]
fn realtime_lane_stats_are_loud() {
    let (keyed_project, _) = project(NOTES, standard_chains(Vec::new()), Some((36, 35)));
    let (_, stats) = render_realtime(&keyed_project);
    assert_eq!(stats.lane_key_edges_declared, 1);
    assert_eq!(stats.lane_key_edges_evaluated, 1);
    assert_eq!(stats.lane_key_edges_unevaluated, 0);
    assert!(
        stats.lane_insert_frames > 0,
        "实时组链必须真的处理过帧（覆盖度见证）"
    );
    assert_eq!(stats.lane_choke_fallbacks, 0);
    assert_eq!(stats.lane_bus_fallbacks, 0);
    assert_eq!(stats.lane_group_capacity_drops, 0);

    let (plain_project, _) = project(NOTES, standard_chains(Vec::new()), None);
    let (_, plain_stats) = render_realtime(&plain_project);
    assert_eq!(plain_stats.lane_key_edges_declared, 0);
    assert_eq!(plain_stats.lane_key_edges_evaluated, 0);
    assert!(
        plain_stats.lane_insert_frames > 0,
        "无侧链边时组链本身仍须在实时侧运行"
    );
}
