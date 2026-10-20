//! 任务单 `#256`：**`app` 的 `--export-wav` 也渲染 `SendGain` 自动化**。
//!
//! 这一份判据与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的发送增益判据**同一口径**：
//! 两条导出共用 `yeban_render::automation::SendGainAutomation`（模型唯一求值入口
//! `automation_value_at` ＋ `ParamSmoother` 5 ms ＋ R55 比率），落点都是
//! `yeban_render` 的**入边归约**。
//!
//! 音源刻意用**同一份 f32 常量音频片段**（两个导出都逐位透传解码）：这样
//! `masterDigest` 与 4 段峰值可以直接跨导出对账，而不是“两个合成器各弹各的”。
//!
//! 边界：⛔ 不证听感；只证 4 等段峰值单调、正对照相等、关掉/静态恒等逐字节不变。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_model::{
    AssetHash, AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget,
    AutomationWriteMode, ClipContent, ClipPlacement, ClipPoolEntry, CurveType, EntityId,
    LoopConfig, MediaKind, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::wav::read_plain_wav;

const FRAMES: u64 = 48_000;
const ONE_SECOND_TICKS: u64 = 1_920;
const I24_SCALE: f32 = 8_388_608.0;

/// `#256` 实测：发送渐强工程的**浮点母带位级摘要**。
///
/// 与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的同名常量**逐字相同** ——
/// 两条导出各自钉住它 ⇒ “同一份浮点母带”是机械可判的，不靠肉眼比分段峰值。
const SEND_RAMP_MASTER_DIGEST: &str =
    "b0897bc0e84190746037c73c9311241aeb1f97374d75e0e558379c1d9652b72f";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-256-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }

    fn wav(&self, tag: &str) -> PathBuf {
        self.dir.join(format!("{tag}.wav"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// 素材：与 MCP 判据**逐字节相同**的 f32 WAV（常量 0.5，立体声，48 kHz）
// ---------------------------------------------------------------------------

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

fn constant(frames: usize, level: f32) -> Vec<f32> {
    vec![level; frames * 2]
}

// ---------------------------------------------------------------------------
// 工程：Master ＋ Lead(Audio) →(send) Aux → Master
// ---------------------------------------------------------------------------

/// 夹具旋钮：只有“发送泳道形状”与“源轨静音”两个自由度。
struct Tune {
    points: &'static [(u64, f32)],
    read_enabled: bool,
    mute: bool,
}

fn project(tune: &Tune, bytes: &[u8], seed: u32) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let aux = id(3);
    let clip = id(10);
    let placement = id(20);
    let send_edge = id(42);
    let aux_edge = id(43);
    let asset = AssetHash::of_bytes(bytes);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "send gain automation fixture".to_owned(),
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
    let mut track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Audio,
        mute: tune.mute,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: ONE_SECOND_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Kick".to_owned(),
            content: ClipContent::Audio {
                asset: asset.clone(),
                gain_db: 0.0,
            },
        },
    );
    project.assets.insert(
        asset.clone(),
        AssetMetadata {
            hash: asset,
            original_path: "samples/kick.wav".to_owned(),
            byte_len: u64::try_from(bytes.len()).expect("小文件"),
            media_kind: MediaKind::Audio,
            license: "CC0-1.0".to_owned(),
        },
    );
    project.tracks.insert(lead, track);
    project.tracks.insert(
        aux,
        TrackV3 {
            id: aux,
            name: "Aux".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![lead, aux, master];
    project.routing_graph.edges.insert(
        send_edge,
        RoutingEdge {
            id: send_edge,
            source_node: lead,
            destination_node: aux,
            kind: RoutingKind::SendToAux,
            gain_db: None,
        },
    );
    project.routing_graph.edges.insert(
        aux_edge,
        RoutingEdge {
            id: aux_edge,
            source_node: aux,
            destination_node: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    );
    if !tune.points.is_empty() || tune.read_enabled {
        let target = AutomationTarget::SendGain {
            track_id: lead,
            edge_id: send_edge,
        };
        let mut points = BTreeMap::new();
        for (index, (tick, value)) in tune.points.iter().enumerate() {
            let point = AutomationPoint {
                id: id(seed + index as u32),
                tick: *tick,
                value: *value,
                curve: CurveType::Linear,
            };
            points.insert(point.id, point);
        }
        project
            .tracks
            .get_mut(&lead)
            .expect("Lead")
            .automation_lanes
            .insert(
                target,
                AutomationLane {
                    target,
                    points,
                    read_enabled: tune.read_enabled,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
    }
    project.validate().expect("夹具工程必须合法");
    project
}

fn options() -> WavExportOptions {
    WavExportOptions {
        frames: FRAMES,
        sample_rate: 48_000,
        channels: 2,
        bit_depth: BitDepth::Int24,
        // ⛔ 不用引擎快照：音频片段是唯一音源，两端（MCP / app）走同一个解码器。
        use_engine_source: false,
        ..WavExportOptions::default()
    }
}

/// 导出一次并回读左声道浮点样本 + 文件 SHA-256 + 浮点母带摘要。
fn export(
    scratch: &Scratch,
    tag: &str,
    project: &YebanProjectV1,
    bytes: &[u8],
) -> (Vec<f32>, String, String) {
    let path = scratch.wav(tag);
    let asset = AssetHash::of_bytes(bytes);
    let report = export_wav_with_sfz_file_and_assets(
        project,
        &path,
        options(),
        None,
        &[(asset, bytes.to_vec())],
    )
    .expect("--export-wav 必须成功");
    assert_eq!(report.frames, FRAMES);
    let file_bytes = std::fs::read(&path).expect("读产物");
    let sha = AssetHash::of_bytes(&file_bytes).as_str().to_owned();
    let (format, buffer) = read_plain_wav(&path).expect("产物必须可回读");
    let channels = usize::from(format.channels.max(1));
    let samples = match buffer {
        PcmBuffer::Float32(values) => values.iter().step_by(channels).copied().collect(),
        PcmBuffer::Int24(values) => values
            .iter()
            .step_by(channels)
            .map(|value| (*value as f32) / I24_SCALE)
            .collect(),
        other => panic!("本判据只导出 24-bit / 32f：{other:?}"),
    };
    (samples, sha, report.digest)
}

fn four_window_peaks(samples: &[f32]) -> [f32; 4] {
    let window = usize::try_from(FRAMES / 4).expect("窗口");
    let mut peaks = [0.0f32; 4];
    for (index, peak) in peaks.iter_mut().enumerate() {
        let start = index * window;
        let end = (start + window).min(samples.len());
        *peak = samples[start..end]
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));
    }
    peaks
}

fn assert_strictly_increasing(peaks: &[f32; 4], context: &str) {
    println!("### {context}: 4 段峰值 = {peaks:?}");
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "{context}: 4 段必须严格单调递增，实测 {peaks:?}"
    );
}

/// ⭐ 判据 2/5：发送量渐强在 `--export-wav` 里也生效，且 4 段峰值与 MCP **同网格同读数**。
#[test]
fn a_send_gain_ramp_is_monotonic_through_export_wav() {
    let scratch = Scratch::new("ramp");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let project = project(
        &Tune {
            points: &[(0, -60.0), (ONE_SECOND_TICKS, 0.0)],
            read_enabled: true,
            mute: false,
        },
        &bytes,
        300,
    );
    let (samples, sha, digest) = export(&scratch, "ramp", &project, &bytes);
    let peaks = four_window_peaks(&samples);
    assert_strictly_increasing(&peaks, "app 发送渐强");
    let ratio = peaks[3] / peaks[0];
    println!(
        "### app 发送渐强 4 段峰值 = {peaks:?} / ratio = {ratio:.6} / sha256 = {sha} / masterDigest = {digest}"
    );
    assert!(ratio >= 3.0, "发送渐强比值 {ratio} 必须 >= 3.0");
    assert_eq!(
        digest, SEND_RAMP_MASTER_DIGEST,
        "发送渐强的浮点母带必须与 MCP 侧逐位相同"
    );
}

/// ⭐ 判据 3/4：无泳道、`readEnabled:false`、曲线恒等于静态边增益 ⇒ 逐字节相同。
#[test]
fn send_gain_identity_paths_are_byte_identical_to_no_lane() {
    let scratch = Scratch::new("identity");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let no_lane = project(
        &Tune {
            points: &[],
            read_enabled: false,
            mute: false,
        },
        &bytes,
        310,
    );
    let (_, baseline_sha, baseline_digest) = export(&scratch, "base", &no_lane, &bytes);

    let disabled = project(
        &Tune {
            points: &[(0, -60.0), (ONE_SECOND_TICKS, 0.0)],
            read_enabled: false,
            mute: false,
        },
        &bytes,
        320,
    );
    let (_, disabled_sha, disabled_digest) = export(&scratch, "disabled", &disabled, &bytes);
    assert_eq!(
        disabled_sha, baseline_sha,
        "readEnabled:false 必须等于没有泳道"
    );
    assert_eq!(disabled_digest, baseline_digest);

    let flat = project(
        &Tune {
            points: &[(0, 0.0), (960, 0.0), (ONE_SECOND_TICKS, 0.0)],
            read_enabled: true,
            mute: false,
        },
        &bytes,
        330,
    );
    let (_, flat_sha, flat_digest) = export(&scratch, "flat", &flat, &bytes);
    println!(
        "### app 恒等: baseline={baseline_sha} disabled={disabled_sha} flat={flat_sha} / digest={baseline_digest}"
    );
    assert_eq!(flat_sha, baseline_sha, "静态恒等必须逐字节相同");
    assert_eq!(flat_digest, baseline_digest);
}

/// ⭐ 判据 7：源轨静音 + 0 dB 发送泳道 ⇒ 仍全静音、且与无泳道的静音版逐字节相同。
#[test]
fn a_send_gain_lane_does_not_resurrect_a_muted_source_through_export_wav() {
    let scratch = Scratch::new("mute");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let muted_base = project(
        &Tune {
            points: &[],
            read_enabled: false,
            mute: true,
        },
        &bytes,
        340,
    );
    let (_, baseline_sha, _) = export(&scratch, "mute-base", &muted_base, &bytes);

    let muted = project(
        &Tune {
            points: &[(0, 0.0)],
            read_enabled: true,
            mute: true,
        },
        &bytes,
        350,
    );
    let (samples, sha, _) = export(&scratch, "mute", &muted, &bytes);
    let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    println!("### app 静音轨 + 0 dB 发送泳道: peak={peak} sha={sha}");
    assert!(
        peak <= 2.0 / I24_SCALE,
        "静音轨的发送必须仍全静音（peak={peak}）"
    );
    assert_eq!(sha, baseline_sha, "自动化不许改变静音源的字节");
}
