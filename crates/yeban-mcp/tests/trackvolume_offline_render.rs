//! TrackVolume 自动化离线渲染回归：泳道必须真的改变母带幅度。
//!
//! 复现（修前应失败）：一条音轨挂时变 TrackVolume 泳道（-50 dB → 0 dB），
//! 离线渲染母带的首尾窗口 RMS 必须显著不同；修前泳道被忽略，两窗口均为素材电平。
//! 对照：无泳道时渲染输出必须与素材电平一致（修后不得改变已有工程的声音）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use yeban_mcp::domain::error::Fault;
use yeban_mcp::domain::render::{self, RenderArtifact};
use yeban_model::{
    AssetHash, AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget,
    AutomationWriteMode, ClipContent, ClipPlacement, ClipPoolEntry, CurveType, EntityId,
    LoopConfig, MediaKind, MidiNote, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
};
use yeban_render::rf64::parse_container;

const NOW_MS: u64 = 1_760_000_000_000;
/// 120 BPM / 960 PPQ 下 1 秒 = 1920 tick。
const TICKS_PER_SECOND: u64 = 1_920;

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-trackvol-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

fn wav_bytes(channels: u16, sample_rate: u32, tag: u16, bits: u16, payload: &[u8]) -> Vec<u8> {
    let block_align = channels * (bits / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let mut out = Vec::with_capacity(44 + payload.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(
        &u32::try_from(36 + payload.len())
            .expect("小文件")
            .to_le_bytes(),
    );
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(payload.len()).expect("小文件").to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn wav_f32(channels: u16, sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        payload.extend_from_slice(&sample.to_le_bytes());
    }
    wav_bytes(channels, sample_rate, 3, 32, &payload)
}

/// 泳道挂在哪条轨上。
#[derive(Clone, Copy, PartialEq)]
enum LaneOn {
    None,
    Lead,
    Master,
    Bus,
}

/// -50 dB → 0 dB 两点线性泳道。
fn volume_lane(target: EntityId) -> AutomationLane {
    let mut points = BTreeMap::new();
    points.insert(
        id(101),
        AutomationPoint {
            id: id(101),
            tick: 0,
            value: -50.0,
            curve: CurveType::Linear,
        },
    );
    points.insert(
        id(102),
        AutomationPoint {
            id: id(102),
            tick: 2 * TICKS_PER_SECOND,
            value: 0.0,
            curve: CurveType::Linear,
        },
    );
    AutomationLane {
        target: AutomationTarget::TrackVolume { track_id: target },
        points,
        read_enabled: true,
        write_mode: AutomationWriteMode::Off,
        domain: None,
    }
}

/// Master + 一条音频轨 → Master；恒定 0.5 立体声素材 2 秒；可选 TrackVolume 泳道。
/// `Bus` 时多一条 AuxReturn 总线轨（Lead → Bus → Master），泳道挂在总线轨上。
fn fixture_with_lane(on: LaneOn) -> (YebanProjectV1, BTreeMap<AssetHash, Vec<u8>>) {
    let bytes = wav_f32(2, 48_000, &vec![0.5; 96_000 * 2]);
    let asset = AssetHash::of_bytes(&bytes);
    let master = id(1);
    let lead = id(2);
    let bus = id(3);
    let clip = id(10);
    let placement = id(20);
    let edge = id(40);
    let edge2 = id(41);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "trackvolume offline fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0001,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    let mut master_track = TrackV3 {
        id: master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };
    if on == LaneOn::Master {
        master_track.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: master },
            volume_lane(master),
        );
    }
    project.tracks.insert(master, master_track);
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Audio,
        volume_db: 0.0,
        pan: 0.0,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 2 * TICKS_PER_SECOND,
            muted: false,
            loop_config: LoopConfig::default(),
        },
    );
    if on == LaneOn::Lead {
        track.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: lead },
            volume_lane(lead),
        );
    }
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Const".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: 0.0,
            },
        },
    );
    project.assets.insert(
        asset.clone(),
        AssetMetadata {
            hash: asset.clone(),
            original_path: "samples/const.wav".to_owned(),
            byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            media_kind: MediaKind::Audio,
            license: "CC0-1.0".to_owned(),
        },
    );
    project.tracks.insert(lead, track);
    if on == LaneOn::Bus {
        let mut bus_track = TrackV3 {
            id: bus,
            name: "Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            volume_db: 0.0,
            pan: 0.0,
            ..TrackV3::default()
        };
        bus_track.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: bus },
            volume_lane(bus),
        );
        project.tracks.insert(bus, bus_track);
        project.routing_graph.nodes = vec![lead, bus, master];
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: lead,
                destination_node: bus,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        project.routing_graph.edges.insert(
            edge2,
            RoutingEdge {
                id: edge2,
                source_node: bus,
                destination_node: master,
                kind: RoutingKind::BusToMaster,
                gain_db: None,
            },
        );
    } else {
        project.routing_graph.nodes = vec![lead, master];
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: lead,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    let mut pool = BTreeMap::new();
    pool.insert(asset, bytes);
    (project, pool)
}

/// 旧 fixture 入口：`with_lane` = 泳道挂在源轨上。
fn fixture(with_lane: bool) -> (YebanProjectV1, BTreeMap<AssetHash, Vec<u8>>) {
    fixture_with_lane(if with_lane {
        LaneOn::Lead
    } else {
        LaneOn::None
    })
}

fn build_with(
    project: &YebanProjectV1,
    assets: &BTreeMap<AssetHash, Vec<u8>>,
    path: &Path,
    arguments: Value,
) -> Result<RenderArtifact, Fault> {
    let request =
        render::validate(arguments.as_object().expect("参数必须是对象")).expect("参数合法");
    let project_path = path.with_extension("project.yeban");
    render::build(project, &project_path, &request, NOW_MS, assets)
}

fn decode_i24(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|chunk| {
            let raw =
                i32::from(chunk[0]) | (i32::from(chunk[1]) << 8) | (i32::from(chunk[2]) << 16);
            let signed = (raw << 8) >> 8;
            signed as f32 / 8_388_608.0
        })
        .collect()
}

fn payload_samples(artifact: &RenderArtifact) -> Vec<f32> {
    let parsed = parse_container(&artifact.bytes).expect("产物必须是合法 WAVE 容器");
    decode_i24(&artifact.bytes[parsed.data.clone()])
}

/// 交织立体声样本在 [start_frame, end_frame) 内的 RMS（双声道合并）。
fn window_rms(samples: &[f32], start_frame: usize, end_frame: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for frame in start_frame..end_frame {
        for channel in 0..2 {
            let value = samples[frame * 2 + channel] as f64;
            sum += value * value;
            count += 1;
        }
    }
    (sum / count as f64).sqrt() as f32
}

#[test]
fn trackvolume_lane_changes_offline_render_amplitude() {
    let scratch = Scratch::new("lane");
    let (project, pool) = fixture(true);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        &project,
        &pool,
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    let samples = payload_samples(&artifact);
    let frames = samples.len() / 2;
    assert!(frames >= 96_000, "母带至少 2 秒，实得 {frames} 帧");
    // 窗口避开 5 ms 平滑与抖动边界：起段 0.02–0.08 秒（泳道约 -49 dB），
    // 末段 1.92–1.98 秒（泳道约 -1 dB）。插值域（dB 或线性）不同，结论都应成立。
    let early = window_rms(&samples, 960, 3_840);
    let late = window_rms(&samples, 92_160, 95_040);
    assert!(
        early < 0.05,
        "起段应被泳道压低，实得 RMS {early}（素材电平 0.5）"
    );
    assert!(late > 0.2, "末段应接近素材电平，实得 RMS {late}");
    assert!(
        late / early > 10.0,
        "首尾 RMS 比应 > 10，实得 {}（起 {early}，末 {late}）",
        late / early
    );
}

#[test]
fn no_lane_renders_at_source_level() {
    let scratch = Scratch::new("nolane");
    let (project, pool) = fixture(false);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        &project,
        &pool,
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    let samples = payload_samples(&artifact);
    let early = window_rms(&samples, 960, 3_840);
    let late = window_rms(&samples, 92_160, 95_040);
    assert!(
        (early - 0.5).abs() < 0.01,
        "无泳道时起段应为素材电平 0.5，实得 {early}"
    );
    assert!(
        (late - 0.5).abs() < 0.01,
        "无泳道时末段应为素材电平 0.5，实得 {late}"
    );
}

/// 主总线泳道：执行之后乘静态主推子 × R55 比率。
#[test]
fn master_trackvolume_lane_changes_offline_render_amplitude() {
    let scratch = Scratch::new("master");
    let (project, pool) = fixture_with_lane(LaneOn::Master);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        &project,
        &pool,
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    let samples = payload_samples(&artifact);
    let early = window_rms(&samples, 960, 3_840);
    let late = window_rms(&samples, 92_160, 95_040);
    assert!(early < 0.05, "主总线泳道起段应被压低，实得 RMS {early}");
    assert!(late > 0.2, "主总线泳道末段应接近素材电平，实得 RMS {late}");
    assert!(
        late / early > 10.0,
        "主总线泳道首尾 RMS 比应 > 10，实得 {}",
        late / early
    );
}

/// 总线轨泳道：输出级推子 × R55 比率（整条总线，含入边求和）。
#[test]
fn bus_trackvolume_lane_changes_offline_render_amplitude() {
    let scratch = Scratch::new("bus");
    let (project, pool) = fixture_with_lane(LaneOn::Bus);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        &project,
        &pool,
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    let samples = payload_samples(&artifact);
    let early = window_rms(&samples, 960, 3_840);
    let late = window_rms(&samples, 92_160, 95_040);
    assert!(early < 0.05, "总线泳道起段应被压低，实得 RMS {early}");
    assert!(late > 0.2, "总线泳道末段应接近素材电平，实得 RMS {late}");
    assert!(
        late / early > 10.0,
        "总线泳道首尾 RMS 比应 > 10，实得 {}",
        late / early
    );
}

/// AuxReturn 带源 + 入边：泳道必须作用于整条总线求和（含入边），不只自己的源。
/// 夹具：Lead（0.5 恒定）→ Aux（自有 0.5 恒定片段＋泳道 -50→0 dB）→ Master。
/// 若 Aux 自己的源根本没进渲染，末段 ≈ 0.5（只剩入边一路）；两路都进则末段 ≈ 1.0。
fn fixture_aux_with_source(
    with_lane: bool,
    aux_db: f32,
) -> (YebanProjectV1, BTreeMap<AssetHash, Vec<u8>>) {
    let bytes = wav_f32(2, 48_000, &vec![0.5; 96_000 * 2]);
    let asset = AssetHash::of_bytes(&bytes);
    let master = id(1);
    let lead = id(2);
    let aux = id(3);
    let clip = id(10);
    let placement_lead = id(20);
    let placement_aux = id(21);
    let edge1 = id(40);
    let edge2 = id(41);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "trackvolume aux fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0001,
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
    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Audio,
        volume_db: 0.0,
        pan: 0.0,
        ..TrackV3::default()
    };
    lead_track.clips.insert(
        placement_lead,
        ClipPlacement {
            id: placement_lead,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 2 * TICKS_PER_SECOND,
            muted: false,
            loop_config: LoopConfig::default(),
        },
    );
    project.tracks.insert(lead, lead_track);
    let mut aux_track = TrackV3 {
        id: aux,
        name: "Aux".to_owned(),
        kind: TrackKind::AuxReturn,
        volume_db: aux_db,
        pan: 0.0,
        ..TrackV3::default()
    };
    aux_track.clips.insert(
        placement_aux,
        ClipPlacement {
            id: placement_aux,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 2 * TICKS_PER_SECOND,
            muted: false,
            loop_config: LoopConfig::default(),
        },
    );
    if with_lane {
        aux_track.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: aux },
            volume_lane(aux),
        );
    }
    project.tracks.insert(aux, aux_track);
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Const".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: 0.0,
            },
        },
    );
    project.assets.insert(
        asset.clone(),
        AssetMetadata {
            hash: asset.clone(),
            original_path: "samples/const.wav".to_owned(),
            byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            media_kind: MediaKind::Audio,
            license: "CC0-1.0".to_owned(),
        },
    );
    project.routing_graph.nodes = vec![lead, aux, master];
    project.routing_graph.edges.insert(
        edge1,
        RoutingEdge {
            id: edge1,
            source_node: lead,
            destination_node: aux,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.routing_graph.edges.insert(
        edge2,
        RoutingEdge {
            id: edge2,
            source_node: aux,
            destination_node: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    let mut pool = BTreeMap::new();
    pool.insert(asset, bytes);
    (project, pool)
}

#[test]
fn aux_with_source_trackvolume_lane_applies_to_whole_bus_sum() {
    let scratch = Scratch::new("auxlane");
    let (project, pool) = fixture_aux_with_source(true, 0.0);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        &project,
        &pool,
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    let samples = payload_samples(&artifact);
    let early = window_rms(&samples, 960, 3_840);
    let late = window_rms(&samples, 92_160, 95_040);
    assert!(
        early < 0.05,
        "Aux 泳道起段应压低整条总线求和（两路 0.5），实得 RMS {early}"
    );
    assert!(
        late > 0.8,
        "Aux 泳道末段应为两路求和 1.0（自有源＋入边），实得 RMS {late}"
    );
    assert!(
        late / early > 10.0,
        "Aux 泳道首尾 RMS 比应 > 10，实得 {}",
        late / early
    );
}

/// 静态探针：Aux 无泳道、静态推子 -6 dB（线性 0.501）时整条和的电平。
/// 两路都进且推子作用于整条和 ⇒ ≈ 0.501；只乘自有源 ⇒ ≈ 0.75；自有源被丢且推子作用于入边 ⇒ ≈ 0.25。
#[test]
fn aux_with_source_static_fader_applies_to_whole_bus_sum() {
    let scratch = Scratch::new("auxstatic");
    let (project, pool) = fixture_aux_with_source(false, -6.0);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        &project,
        &pool,
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    let samples = payload_samples(&artifact);
    let level = window_rms(&samples, 92_160, 95_040);
    assert!(
        (level - 0.501).abs() < 0.05,
        "Aux 静态推子应作用于整条总线求和（两路 0.5 × 0.501 ≈ 0.501），实得 RMS {level}"
    );
}

/// MIDI 轨泳道：引擎音源（无 SFZ，默认合成器）＋源侧 TrackVolume 比率。
/// 合成器包络本身可能起伏，故以无泳道对照的同窗口 RMS 作基线：泳道起段应远低于对照起段。
fn fixture_midi_with_lane(with_lane: bool) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let edge = id(40);
    let note = id(30);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "trackvolume midi fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0002,
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
    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        volume_db: 0.0,
        pan: 0.0,
        ..TrackV3::default()
    };
    lead_track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 2 * TICKS_PER_SECOND,
            muted: false,
            loop_config: LoopConfig::default(),
        },
    );
    if with_lane {
        lead_track.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: lead },
            volume_lane(lead),
        );
    }
    project.tracks.insert(lead, lead_track);
    let mut notes = BTreeMap::new();
    notes.insert(
        note,
        MidiNote {
            velocity: 100,
            ..MidiNote::new(note, 0, 69, 2 * TICKS_PER_SECOND)
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Note".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.routing_graph.nodes = vec![lead, master];
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: lead,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    project
}

fn render_midi(project: &YebanProjectV1, tag: &str) -> Vec<f32> {
    let scratch = Scratch::new(tag);
    let wav_path = scratch.join("out.wav");
    let artifact = build_with(
        project,
        &BTreeMap::new(),
        &wav_path,
        json!({"format": "wav", "sampleRate": 48000, "path": wav_path.display().to_string()}),
    )
    .expect("渲染必须成功");
    payload_samples(&artifact)
}

#[test]
fn midi_trackvolume_lane_changes_offline_render_amplitude() {
    let lane = render_midi(&fixture_midi_with_lane(true), "midilane");
    let plain = render_midi(&fixture_midi_with_lane(false), "midiplain");
    let lane_early = window_rms(&lane, 960, 3_840);
    let lane_late = window_rms(&lane, 92_160, 95_040);
    let plain_early = window_rms(&plain, 960, 3_840);
    let plain_late = window_rms(&plain, 92_160, 95_040);
    assert!(
        plain_early > 0.01,
        "无泳道对照起段必须有声（否则夹具无声），实得 RMS {plain_early}"
    );
    assert!(
        lane_early < plain_early * 0.1,
        "MIDI 泳道起段应远低于无泳道对照起段，泳道 {lane_early} 对照 {plain_early}"
    );
    assert!(
        lane_late > plain_late * 0.5,
        "MIDI 泳道末段应接近无泳道对照末段，泳道 {lane_late} 对照 {plain_late}"
    );
}

/// MIDI 总线源轨：Lead 音符 → Aux（自有音符＋泳道 -50→0 dB）→ Master。
/// 引擎臂的推子中性化走快照投影副本；以无泳道对照校准合成器包络。
fn fixture_midi_aux_with_source(with_lane: bool) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let aux = id(3);
    let clip = id(10);
    let placement_lead = id(20);
    let placement_aux = id(21);
    let edge1 = id(40);
    let edge2 = id(41);
    let note_lead = id(30);
    let note_aux = id(31);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "trackvolume midi aux fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0003,
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
    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        volume_db: 0.0,
        pan: 0.0,
        ..TrackV3::default()
    };
    lead_track.clips.insert(
        placement_lead,
        ClipPlacement {
            id: placement_lead,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 2 * TICKS_PER_SECOND,
            muted: false,
            loop_config: LoopConfig::default(),
        },
    );
    project.tracks.insert(lead, lead_track);
    let mut aux_track = TrackV3 {
        id: aux,
        name: "Aux".to_owned(),
        kind: TrackKind::AuxReturn,
        volume_db: 0.0,
        pan: 0.0,
        ..TrackV3::default()
    };
    aux_track.clips.insert(
        placement_aux,
        ClipPlacement {
            id: placement_aux,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 2 * TICKS_PER_SECOND,
            muted: false,
            loop_config: LoopConfig::default(),
        },
    );
    if with_lane {
        aux_track.automation_lanes.insert(
            AutomationTarget::TrackVolume { track_id: aux },
            volume_lane(aux),
        );
    }
    project.tracks.insert(aux, aux_track);
    let mut notes = BTreeMap::new();
    notes.insert(
        note_lead,
        MidiNote {
            velocity: 100,
            ..MidiNote::new(note_lead, 0, 69, 2 * TICKS_PER_SECOND)
        },
    );
    notes.insert(
        note_aux,
        MidiNote {
            velocity: 100,
            ..MidiNote::new(note_aux, 0, 69, 2 * TICKS_PER_SECOND)
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Notes".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.routing_graph.nodes = vec![lead, aux, master];
    project.routing_graph.edges.insert(
        edge1,
        RoutingEdge {
            id: edge1,
            source_node: lead,
            destination_node: aux,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.routing_graph.edges.insert(
        edge2,
        RoutingEdge {
            id: edge2,
            source_node: aux,
            destination_node: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    );
    project
        .validate()
        .unwrap_or_else(|error| panic!("夹具工程必须合法: {error}"));
    project
}

#[test]
fn midi_aux_with_source_trackvolume_lane_applies_to_whole_bus_sum() {
    let lane = render_midi(&fixture_midi_aux_with_source(true), "midiauxlane");
    let plain = render_midi(&fixture_midi_aux_with_source(false), "midiauxplain");
    let lane_early = window_rms(&lane, 960, 3_840);
    let lane_late = window_rms(&lane, 92_160, 95_040);
    let plain_early = window_rms(&plain, 960, 3_840);
    let plain_late = window_rms(&plain, 92_160, 95_040);
    assert!(
        plain_early > 0.01,
        "无泳道对照起段必须有声（否则夹具无声），实得 RMS {plain_early}"
    );
    assert!(
        lane_early < plain_early * 0.1,
        "MIDI Aux 泳道起段应远低于无泳道对照起段，泳道 {lane_early} 对照 {plain_early}"
    );
    assert!(
        lane_late > plain_late * 0.7,
        "MIDI Aux 泳道末段应接近无泳道对照末段（两路音符求和；只剩一路时恰为 0.5 倍），泳道 {lane_late} 对照 {plain_late}"
    );
}
