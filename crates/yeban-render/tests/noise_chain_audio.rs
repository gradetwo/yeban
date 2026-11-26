//! D5 噪声源判据（鼓机缺口线）：噪声发生器（白/粉红底噪 + 爆裂声）进插入链
//! 后的**音频级**证据。定值与出厂预置同源：lofi 的 `fx.noise`
//! （level 0.035 / color 0.85 / crackle_density 5 / crackle_level 0.4 /
//! seed 2026）。
//!
//! 夹具与 `shaping_chain_audio.rs` 同形：合成鼓机轨（120 BPM / 960 PPQ /
//! 48 kHz ⇒ 1 tick = 25 帧，3840 tick = 96 000 帧）＋一条无音符的静音夹具。
//!
//! | # | 判据 | 用例 |
//! | :--- | :--- | :--- |
//! | ① | lofi 噪声定值挂轨链：与无器件对照大面积不同；同工程双渲逐位一致（种子可复现） | `lofi_noise_values_change_the_track_render_and_reproduce_bit_exactly` |
//! | ② | 中性噪声（level 0 且 crackle_level 0）逐位恒等（恒等门：不写段/关段零行为变化） | `neutral_noise_is_bit_identical` |
//! | ③ | 静音夹具只挂噪声：底噪 RMS 落在可听带内；爆裂声事件可数且稀疏；换种子输出必变 | `noise_only_render_has_a_floor_and_sparse_countable_crackle` |

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

/// lofi 出厂噪声定值（与 `crates/yeban-theory/drum_styles/lofi.json` 的
/// `fx.noise` 逐值同源；theory 侧判据另钉 JSON 与本表一致）。
const LOFI_NOISE: &[(&str, f32)] = &[
    ("noise_level", 0.035),
    ("noise_color", 0.85),
    ("noise_crackle_density", 5.0),
    ("noise_crackle_level", 0.4),
    ("noise_seed", 2026.0),
];

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

/// 合成鼓机设备（与 `shaping_chain_audio.rs` 夹具同口径）。
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

fn project(fx: Vec<DeviceDefinition>, with_notes: bool) -> (YebanProjectV1, EntityId) {
    let master = id(1);
    let track = id(100);
    let clip = id(1_000);
    let placement = id(2_000);
    let mut project = YebanProjectV1 {
        title: "noise chain fixture".to_owned(),
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
        lane_chains: Vec::<LaneChainDef>::new(),
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
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
    project.tracks.insert(track, drum_track);
    let mut note_map = BTreeMap::new();
    if with_notes {
        for (index, &(start, pitch, duration)) in NOTES.iter().enumerate() {
            let note = id(3_000 + index);
            note_map.insert(note, MidiNote::new(note, start, pitch, duration));
        }
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
        .unwrap_or_else(|error| panic!("noise 夹具必须合法: {error}"));
    (project, track)
}

fn render(project: &YebanProjectV1, track: EntityId) -> Vec<f32> {
    let timbre = EngineTimbre::from_project(project).expect("夹具必须成快照");
    let options = RenderOptions::l1(FRAMES, 2, SR, 0x5EED).with_threads(1);
    let inserts = project_inserts(&project.tracks, SR, project.bpm);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &project.routing_graph,
        project.master_bus_track_id,
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("计划必须能编译");
    let source: Box<dyn AudioSource> =
        Box::new(timbre.track_source(track).expect("轨源必须能武装"));
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

/// 数一帧流里爆裂事件的个数：样本绝对值越过 `threshold` 且距上一个事件
/// 超过 `refractory` 帧，记一次事件起点。
fn count_events(channel: &[f32], threshold: f32, refractory: usize) -> usize {
    let mut count = 0usize;
    let mut last = usize::MAX;
    for (index, sample) in channel.iter().enumerate() {
        if sample.abs() > threshold && (last == usize::MAX || index - last > refractory) {
            count += 1;
            last = index;
        }
    }
    count
}

fn channel(samples: &[f32], ch: usize) -> Vec<f32> {
    samples.iter().skip(ch).step_by(2).copied().collect()
}

/// ① lofi 噪声定值挂轨链必须大面积改声（底噪连续存在、爆裂声叠加），且
/// 同工程同种子双渲逐位一致（种子可复现是硬规则）。
#[test]
fn lofi_noise_values_change_the_track_render_and_reproduce_bit_exactly() {
    let (plain_project, track) = project(Vec::new(), true);
    let plain = render(&plain_project, track);
    let (noise_project, track) = project(vec![effect_device("Noise Source", LOFI_NOISE)], true);
    let noised = render(&noise_project, track);
    let diff = differing(&plain, &noised);
    eprintln!(
        "lofi noise: differing {diff}/{} samples, rms {:+.3} -> {:+.3} dB",
        plain.len(),
        rms_db(&plain),
        rms_db(&noised),
    );
    assert!(
        diff > plain.len() / 2,
        "连续底噪应改动过半样本，实测 {diff}/{}",
        plain.len()
    );
    let repeated = render(&noise_project, track);
    assert_eq!(
        differing(&noised, &repeated),
        0,
        "同种子双渲必须逐位一致（种子可复现）"
    );
}

/// ② 中性噪声（level 0 且 crackle_level 0）逐位恒等：关掉的噪声源不得
/// 改任何一个样本（不写段 = 零行为变化的渲染级对应物）。
#[test]
fn neutral_noise_is_bit_identical() {
    let (plain_project, track) = project(Vec::new(), true);
    let plain = render(&plain_project, track);
    let (neutral_project, track) = project(
        vec![effect_device(
            "Noise Source",
            &[
                ("noise_level", 0.0),
                ("noise_color", 0.85),
                ("noise_crackle_density", 5.0),
                ("noise_crackle_level", 0.0),
                ("noise_seed", 2026.0),
            ],
        )],
        true,
    );
    let neutral = render(&neutral_project, track);
    assert_eq!(
        differing(&plain, &neutral),
        0,
        "中性噪声 (level 0, crackle_level 0) 必须逐位恒等（恒等门）"
    );
}

/// ③ 静音夹具：只有噪声源在响。底噪 RMS 须落在可听带（不是零、也不许
/// 大到淹没节目）；爆裂声事件可数且稀疏；换种子输出必变。
#[test]
fn noise_only_render_has_a_floor_and_sparse_countable_crackle() {
    // 底噪（无爆裂声）：整段就是"静音间隙"，RMS 即间隙底噪。
    let (floor_project, track) = project(
        vec![effect_device(
            "Noise Source",
            &[
                ("noise_level", 0.035),
                ("noise_color", 0.85),
                ("noise_crackle_density", 0.0),
                ("noise_crackle_level", 0.0),
                ("noise_seed", 2026.0),
            ],
        )],
        false,
    );
    let floor = render(&floor_project, track);
    let floor_db = rms_db(&floor);
    eprintln!("noise floor only: rms {floor_db:+.3} dBFS");
    assert!(
        (-48.0..=-24.0).contains(&floor_db),
        "底噪 RMS 须落在 -48..-24 dBFS 可听带，实测 {floor_db:+.3}"
    );

    // 爆裂声（无连续底噪）：事件可数且稀疏（2 秒、每声道期望 16 次）。
    let crackle_params = |seed: f32| {
        vec![effect_device(
            "Noise Source",
            &[
                ("noise_level", 0.0),
                ("noise_color", 0.85),
                ("noise_crackle_density", 8.0),
                ("noise_crackle_level", 0.6),
                ("noise_seed", seed),
            ],
        )]
    };
    let (crackle_project, track) = project(crackle_params(2026.0), false);
    let crackle = render(&crackle_project, track);
    for ch in 0..2 {
        let events = count_events(&channel(&crackle, ch), 0.15, 240);
        eprintln!("crackle ch{ch}: {events} events in 2 s at density 8/s");
        assert!(
            (4..=40).contains(&events),
            "爆裂事件须可数且稀疏（每声道 2 秒期望 16 次），实测 {events}"
        );
    }
    let (other_project, track) = project(crackle_params(7.0), false);
    let other = render(&other_project, track);
    assert!(
        differing(&crackle, &other) > 0,
        "换种子后爆裂排布必须改变（种子真的在决定输出）"
    );
}
