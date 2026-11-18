//! lane 级侧链的**音频级**判据（逐件＋侧链期 4）：同轨 lane→lane 的键
//! 真的改变了母带样本，而不只是"计划里接上了"。
//!
//! 夹具 = 一条合成鼓机轨（120 BPM / 960 PPQ / 48 kHz ⇒ 1 tick = 25 帧）：
//!
//! ```text
//!   808 (键 35, 长音) ── bass 组: 压缩器（阈值 −30 dB / 8:1）
//!   kick (键 36)      ── kick 组: 无链 —— 侧链边的 key 源（链前抽头）
//!   hat (键 42)       ── hats 组: 高通（隔离判据用）
//!   边: Sidechain(source_lane=36, dest_lane=35)，同轨
//! ```
//!
//! 组序按首键升序 = bass(35) → kick(36) → hats(42)：目的组排在源组
//! **之前**，正是两遍渲染要解决的情形（单遍顺序渲染拿不到本块的键）。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | kick 作 key 闪避 808：kick 落点后窗口内母带比无边对照低 ≥ 6 dB，且边被计为已求值 | `kick_ducks_the_808_lane` |
//! | ② | hat 组高通不影响 kick：首个 hat 之前 kick+808 段与无高通渲染逐位相同；hat 段本身必须有别（高通真生效） | `hat_highpass_does_not_touch_kick` |
//! | ③ | 目的键落空（无组认领）：边未求值、渲染不报错、与无边逐位相同 | `dangling_dest_lane_is_unevaluated_and_bit_identical` |
//! | ④ | 跨轨 lane 边：本期不求值、渲染不报错 | `cross_track_lane_edge_is_unevaluated` |
//! | ⑤ | 无侧链边时挂接调用不改一字节；同工程两次渲染逐位相同（确定性） | `no_edge_attach_changes_nothing_and_render_is_deterministic` |
//! | ⑥ | 独立 808 音色槽：键 35 出声且与键 36 的渲染不同（独立声部） | `key_35_is_an_independent_audible_voice` |
//! | ⑦ | 模型：lane 端点只许挂侧链边（其它类型响亮拒绝）；旧 JSON 无新字段照常加载 | `lane_endpoints_only_on_sidechain_edges` |

use std::collections::BTreeMap;
use std::str::FromStr as _;

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

/// 合成鼓机设备：五键识别 + 独立 808 键位与长衰减（2.5 s 的持续低音，
/// 被闪避的节目本体）。
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
        // 镲的声部内滤波放宽：能量铺到 1.5 kHz 起 ⇒ 链上 6 kHz 高通
        // 切得动它（出厂缺省 6k/14k 时镲能量几乎全在高通之上，判据量不到）。
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

/// bass 组的压缩通道条（与节点级音频判据同定值：阈值 −30 dB、8:1、
/// 检波 10 ms、起振 1 ms、释放 100 ms）。
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

/// hats 组的高通通道条（6 kHz 低切；与压缩同条，隔离靠对照组同链旁通）。
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
        title: "lane sidechain fixture".to_owned(),
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
        .unwrap_or_else(|error| panic!("lane 侧链夹具必须合法: {error}"));
    (project, track)
}

/// 渲染一份工程，返回（母带交错样本, lane 已求值边数）。
fn render(project: &YebanProjectV1, track: EntityId, attach_lane: bool) -> (Vec<f32>, usize) {
    let timbre = EngineTimbre::from_project(project).expect("夹具必须成快照");
    let options = RenderOptions::l1(FRAMES, 2, SR, 0x5EED).with_threads(1);
    let mut plan =
        RenderPlan::compile(&project.routing_graph, project.master_bus_track_id, options)
            .expect("计划必须能编译");
    let evaluated = if attach_lane {
        plan.attach_lane_key_routes(&lane_key_layouts(project, SR));
        plan.lane_key_evaluated_edge_ids().len()
    } else {
        0
    };
    let chains = &project.tracks[&track].lane_chains;
    let source = timbre
        .lane_track_source(track, chains, project.bpm)
        .expect("分组源必须能武装");
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(track, Box::new(source));
    let output = plan.execute(sources).expect("渲染必须成功");
    (output.samples, evaluated)
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

/// ① kick 作 key 闪避 808（验收例，设计 §5.2.2 的可测化：凹口 ≥ 6 dB）。
#[test]
fn kick_ducks_the_808_lane() {
    let (keyed_project, track) = project(NOTES, standard_chains(Vec::new()), Some((36, 35)));
    let (keyed, evaluated) = render(&keyed_project, track, true);
    assert_eq!(evaluated, 1, "同轨 lane 边必须被求值");
    let (plain_project, track) = project(NOTES, standard_chains(Vec::new()), None);
    let (plain, _) = render(&plain_project, track, true);
    // 量法（双差分）：无边对照里 808 被自己的信号自键压缩（恒定 GR），
    // 键控渲染里 GR 被 kick 锁定调制 —— 两种渲染各自的「近窗（落点后
    // 5..25 ms，凹口最深）比远窗（落点后 350..400 ms，释放已恢复）低
    // 多少」之差，就是键独有的闪避调制。kick 自身尾巴在两种渲染里
    // 形状相同，被差分抵消。
    for onset in [24_000usize, 48_000] {
        let near = onset + 240..onset + 1_200;
        let far = onset + 16_800..onset + 19_200;
        let mod_keyed = rms_db(&keyed, far.clone()) - rms_db(&keyed, near.clone());
        let mod_plain = rms_db(&plain, far.clone()) - rms_db(&plain, near.clone());
        let duck = mod_keyed - mod_plain;
        println!(
            "kick@{onset}: 键控调制 {mod_keyed:.2} dB / 对照调制 {mod_plain:.2} dB / 闪避量 {duck:.2} dB"
        );
        assert!(
            duck >= 6.0,
            "kick 落点 {onset} 处键必须把 808 压出 ≥ 6 dB 的锁定凹口，实测 {duck:.2} dB"
        );
    }
}

/// ② hat 组高通不影响 kick（逐位）；hat 段本身必须被高通改动（判据不空转）。
#[test]
fn hat_highpass_does_not_touch_kick() {
    // 专用音符：kick@0（独占 0..12000 帧）、开镲 46@480（长衰减，高通
    // 前后在起音段的低频含量差异最大）、808 不参与（不淹没测量窗）。
    let notes: &[(u64, u8, u64)] = &[(0, 36, 240), (480, 46, 960), (1_920, 36, 240)];
    let chains = |hat_devices: Vec<DeviceDefinition>| {
        vec![
            chain(&[36], "kick", Vec::new()),
            chain(&[42, 46], "hats", hat_devices),
        ]
    };
    let (with_hpf, track) = project(notes, chains(vec![hpf_strip()]), None);
    let (keyed_a, _) = render(&with_hpf, track, true);
    // 对照 = 同一条链、只把低切置 0（旁通位）：两份渲染的唯一差别是高通。
    let bypassed = DeviceDefinition {
        params: vec![param("hpf_hz", 0.0)],
        ..comp_strip()
    };
    let (without_hpf, track) = project(notes, chains(vec![bypassed]), None);
    let (keyed_b, _) = render(&without_hpf, track, true);
    // 首个 hat 在帧 12000；其前只有 kick ⇒ 逐位相同。
    for (index, (a, b)) in keyed_a[..12_000 * 2]
        .iter()
        .zip(&keyed_b[..12_000 * 2])
        .enumerate()
    {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "hat 高通不得影响 kick 段（第 {index} 个样本）"
        );
    }
    // 开镲起音段（12000..15600）必须有别：高通真的在 hat 组上生效。
    let hat_a = rms_db(&keyed_a, 12_000..15_600);
    let hat_b = rms_db(&keyed_b, 12_000..15_600);
    println!("开镲起音段 RMS：高通 {hat_a:.2} dB / 无高通 {hat_b:.2} dB");
    assert!(
        (hat_a - hat_b).abs() > 0.5,
        "高通必须真的改动 hat 段（差 {:.2} dB）",
        (hat_a - hat_b).abs()
    );
}

/// ③ 目的键落空：边未求值、渲染不报错、与无边逐位相同（设计 §5.2.4）。
#[test]
fn dangling_dest_lane_is_unevaluated_and_bit_identical() {
    // chains 只有 hats 组：dest 键 35 没有组认领。
    let chains = vec![chain(&[42], "hats", Vec::new())];
    let (dangling_project, track) = project(NOTES, chains, Some((36, 35)));
    let (out, evaluated) = render(&dangling_project, track, true);
    assert_eq!(evaluated, 0, "目的键落空的边不得求值");
    let chains = vec![chain(&[42], "hats", Vec::new())];
    let (plain_project, track) = project(NOTES, chains, None);
    let (plain, _) = render(&plain_project, track, true);
    assert_eq!(out.len(), plain.len());
    for (index, (a, b)) in out.iter().zip(&plain).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "落空边不得改动任何样本（第 {index} 个样本）"
        );
    }
}

/// ④ 跨轨 lane 边：本期不求值（渲染照常、计数为 0）。
#[test]
fn cross_track_lane_edge_is_unevaluated() {
    let (mut project, track) = project(NOTES, standard_chains(Vec::new()), None);
    // 第二条轨带自己的逐件链；边从鼓轨指向它（跨轨）。
    let other = id(200);
    let mut other_track = TrackV3 {
        id: other,
        name: "Other".to_owned(),
        kind: TrackKind::Midi,
        lane_chains: vec![chain(&[35], "bass", vec![comp_strip()])],
        ..TrackV3::default()
    };
    other_track.devices.push(drum_device());
    project.tracks.insert(other, other_track);
    project.routing_graph.nodes.push(other);
    let audio_edge = id(4_003);
    project.routing_graph.edges.insert(
        audio_edge,
        RoutingEdge {
            id: audio_edge,
            source_node: other,
            destination_node: project.master_bus_track_id,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    let edge = id(4_002);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: other,
            kind: RoutingKind::Sidechain,
            gain_db: None,
            source_lane: Some(36),
            dest_lane: Some(35),
        },
    );
    let timbre = EngineTimbre::from_project(&project).expect("夹具必须成快照");
    let options = RenderOptions::l1(FRAMES, 2, SR, 0x5EED).with_threads(1);
    let mut plan =
        RenderPlan::compile(&project.routing_graph, project.master_bus_track_id, options)
            .expect("计划必须能编译");
    plan.attach_lane_key_routes(&lane_key_layouts(&project, SR));
    assert_eq!(
        plan.lane_key_evaluated_edge_ids().len(),
        0,
        "跨轨 lane 边本期不得求值"
    );
    // 两条轨都要有源（other 无片段 ⇒ 用空源语义的分组源照常推进）。
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let chains = &project.tracks[&track].lane_chains;
    sources.insert(
        track,
        Box::new(
            timbre
                .lane_track_source(track, chains, project.bpm)
                .expect("分组源必须能武装"),
        ),
    );
    let other_chains = &project.tracks[&other].lane_chains;
    sources.insert(
        other,
        Box::new(
            timbre
                .lane_track_source(other, other_chains, project.bpm)
                .expect("分组源必须能武装"),
        ),
    );
    plan.execute(sources).expect("跨轨边在场时渲染必须成功");
}

/// ⑤ 无边挂接不改一字节 + 确定性。
#[test]
fn no_edge_attach_changes_nothing_and_render_is_deterministic() {
    let (project, track) = project(NOTES, standard_chains(Vec::new()), None);
    let (attached, _) = render(&project, track, true);
    let (unattached, _) = render(&project, track, false);
    assert_eq!(attached.len(), unattached.len());
    for (index, (a, b)) in attached.iter().zip(&unattached).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "无 lane 边时挂接不得改动样本（第 {index} 个样本）"
        );
    }
    let (again, _) = render(&project, track, true);
    assert_eq!(attached, again, "同工程两次渲染必须逐位相同");
}

/// ⑥ 独立 808 音色槽：键 35 出声、且与键 36 不是同一个声部。
#[test]
fn key_35_is_an_independent_audible_voice() {
    let render_notes = |notes: &[(u64, u8, u64)]| {
        let (project, track) = project(notes, Vec::new(), None);
        let timbre = EngineTimbre::from_project(&project).expect("夹具必须成快照");
        let options = RenderOptions::l1(FRAMES, 2, SR, 0x5EED).with_threads(1);
        let mut plan =
            RenderPlan::compile(&project.routing_graph, project.master_bus_track_id, options)
                .expect("计划必须能编译");
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(
            track,
            Box::new(timbre.track_source(track).expect("单源必须能武装")),
        );
        plan.execute(sources).expect("渲染必须成功").samples
    };
    let bass = render_notes(&[(0, 35, 3_840)]);
    let kick = render_notes(&[(0, 36, 240)]);
    let bass_level = rms_db(&bass, 2_000..20_000);
    println!("808 长音段 RMS：{bass_level:.2} dBFS");
    assert!(
        bass_level > -40.0,
        "键 35 必须出声（实测 {bass_level:.2} dBFS）"
    );
    assert_ne!(bass, kick, "键 35 与键 36 必须是两个独立声部");
}

/// ⑦ 模型校验：lane 端点只许挂侧链边；旧 JSON（无 lane 字段）照常加载。
#[test]
fn lane_endpoints_only_on_sidechain_edges() {
    let base = RoutingEdge {
        id: id(4_010),
        source_node: id(100),
        destination_node: id(100),
        kind: RoutingKind::TrackToBus,
        gain_db: None,
        source_lane: Some(36),
        dest_lane: Some(35),
    };
    assert!(base.validate().is_err(), "非侧链边带 lane 端点必须响亮拒绝");
    let sidechain = RoutingEdge {
        kind: RoutingKind::Sidechain,
        ..base.clone()
    };
    assert!(sidechain.validate().is_ok(), "侧链边带 lane 端点合法");
    // 旧工程 JSON 没有 source_lane/dest_lane 字段 ⇒ 加载为 None。
    let legacy = r#"{"id":"01J8ZN00000000000000004010","source_node":"01J8ZN00000000000000000100","destination_node":"01J8ZN00000000000000000100","kind":"Sidechain","gain_db":null}"#;
    let loaded: RoutingEdge = serde_json::from_str(legacy).expect("旧 JSON 必须能加载");
    assert_eq!(loaded.source_lane, None);
    assert_eq!(loaded.dest_lane, None);
    // 带端点的边序列化出字段且可往返。
    let text = serde_json::to_string(&sidechain).expect("序列化");
    assert!(text.contains("\"source_lane\":36"), "{text}");
    let back: RoutingEdge = serde_json::from_str(&text).expect("往返加载");
    assert_eq!(back, sidechain);
}
