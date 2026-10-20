//! 任务单 `#257`：**`TrackPan` 自动化在两条导出里都生效**。
//!
//! 这一份判据与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的声相判据**同一口径**：
//! 两条导出共用 `yeban_render::automation::TrackPanAutomation`（模型唯一求值入口
//! `automation_value_at` ＋ `ParamSmoother` 5 ms ＋ 源侧静态声相关掉后乘**绝对**左右增益），
//! 曲线都是同一条等功率曲线（`audio_clip::track_pan_gains` ／ MCP 的 `track_pan_gains`，
//! 两者都经 `centered_pan_gains`）。
//!
//! 音源刻意用**同一份 f32 常量音频片段**（两个导出都逐位透传解码）：这样
//! `masterDigest` 与前后两段的 L/R 比可以直接跨导出对账，而不是"两个合成器各弹各的"。
//!
//! 边界：⛔ 不证听感；只证以下可机械判定的东西：
//! 1. 修前——声相泳道与无泳道的产物**逐字节相同**（自动化被忽略）；
//! 2. 修后——前段左重、后段右重（两段的 L/R 比**反向**）；
//! 3. 正对照——无泳道工程的 sha256 修前/修后相同；
//! 4. `readEnabled:false` ⇒ 等于没有泳道；
//! 5. 同一段曲线末端两条导出的 `masterDigest` 逐位相同（⇒ 同网格同读数）；
//! 6. 同一 `pan` 值下自动化值 = 静态轨值（本夹具轨增益 1.0 ⇒ **逐位**）；
//! 7. `pan = -1` 时**浮点层**右声道恰为 `0.0`（抖动层 ±1 LSB，另行读数）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_model::{
    AssetHash, AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget,
    AutomationWriteMode, ClipContent, ClipPlacement, ClipPoolEntry, CurveType, EntityId,
    LoopConfig, MediaKind, RoutingEdge, RoutingKind, SampleRate, TrackKind, TrackV3,
    YebanProjectV1,
};
use yeban_render::dither::BitDepth;
use yeban_render::wav::read_plain_wav;

const FRAMES: u64 = 48_000;
const ONE_SECOND_TICKS: u64 = 1_920;
const I24_SCALE: f32 = 8_388_608.0;

/// `#257` 实测：声相渐强工程（`-1 → +1`）的**浮点母带位级摘要**。
///
/// 与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的同名常量**逐字相同** ——
/// 两条导出各自钉住它 ⇒ “同一份浮点母带”是机械可判的。
const PAN_RAMP_MASTER_DIGEST: &str =
    "84e17115c2895a44a7b57d7b2638ca4a4ea8aed32f98f95b7b85d70631068ad4";

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
            "yeban-257-{tag}-{}-{}",
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

/// 常量 0.5 的立体声素材：左右都是同一个值 ⇒ L/R 比只由声相决定。
fn constant(frames: usize, level: f32) -> Vec<f32> {
    vec![level; frames * 2]
}

// ---------------------------------------------------------------------------
// 工程：Master ← Lead(Audio 片段)
// ---------------------------------------------------------------------------

/// 夹具旋钮：泳道形状、`readEnabled`、静态 `pan` 三个自由度。
struct Tune {
    points: &'static [(u64, f32)],
    read_enabled: bool,
    track_pan: f32,
    /// 是否真的插入泳道对象（空点 + 关掉 = “没有泳道”与“关掉的空泳道”的对照）。
    insert_lane: bool,
}

fn project(tune: &Tune, bytes: &[u8], seed: u32) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let edge = id(40);
    let asset = AssetHash::of_bytes(bytes);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "track pan automation fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0257,
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
        volume_db: 0.0,
        pan: tune.track_pan,
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
    if tune.insert_lane {
        let target = AutomationTarget::TrackPan { track_id: lead };
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
        track.automation_lanes.insert(
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
    project.tracks.insert(lead, track);
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

/// 只跑 `render_core` 的浮点母带（判逐位/浮点层读数用）+ digest。
fn render(project: &YebanProjectV1, bytes: &[u8]) -> RenderProjectOutput {
    let asset = AssetHash::of_bytes(bytes);
    render_project_f32_with_assets_under(project, options(), None, &[(asset, bytes.to_vec())], None)
        .expect("浮点渲染必须成功")
}

/// 真的落一次盘并回读 **L/R 交织**浮点样本 + 文件 SHA-256 + 浮点母带摘要。
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
        yeban_render::dither::PcmBuffer::Int24(values) => values
            .iter()
            .map(|value| (*value as f32) / I24_SCALE)
            .collect::<Vec<_>>(),
        _ => panic!("本判据只导出 24-bit：{format:?}"),
    };
    assert_eq!(samples.len(), usize::try_from(FRAMES).unwrap() * channels);
    (samples, sha, report.digest)
}

/// 一段 `[from, to)` 帧的左右声道**绝对值和**。素材是常量正值 ⇒ 和比 = 能量比。
fn half_sums(samples: &[f32], from: usize, to: usize) -> (f64, f64) {
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    for frame in from..to {
        left += f64::from(samples[frame * 2].abs());
        right += f64::from(samples[frame * 2 + 1].abs());
    }
    (left, right)
}

fn ratio(left: f64, right: f64) -> f64 {
    if right == 0.0 {
        f64::INFINITY
    } else {
        left / right
    }
}

/// ⭐ 判据 2/5/6：声相渐强在 `--export-wav` 里生效，前后两段 L/R 比**反向**。
#[test]
fn a_track_pan_ramp_moves_left_to_right_through_export_wav() {
    let scratch = Scratch::new("ramp");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let ramp = project(
        &Tune {
            points: &[(0, -1.0), (ONE_SECOND_TICKS, 1.0)],
            read_enabled: true,
            track_pan: 0.0,
            insert_lane: true,
        },
        &bytes,
        300,
    );
    let (samples, sha, digest) = export(&scratch, "ramp", &ramp, &bytes);
    let (no_lane, no_lane_sha, no_lane_digest) = export(
        &scratch,
        "no-lane",
        &project(
            &Tune {
                points: &[],
                read_enabled: false,
                track_pan: 0.0,
                insert_lane: false,
            },
            &bytes,
            350,
        ),
        &bytes,
    );
    let mid = usize::try_from(FRAMES / 2).unwrap();
    let frames = usize::try_from(FRAMES).unwrap();
    let (early_l, early_r) = half_sums(&samples, 0, mid);
    let (late_l, late_r) = half_sums(&samples, mid, frames);
    let early = ratio(early_l, early_r);
    let late = ratio(late_l, late_r);
    let (base_l, base_r) = half_sums(&no_lane, 0, frames);
    println!(
        "### app pan ramp: sha={sha} digest={digest}\n\
         ###   early L/R={early:.6} (L={early_l:.3} R={early_r:.3}) \
         late L/R={late:.6} (L={late_l:.3} R={late_r:.3})\n\
         ###   no-lane sha={no_lane_sha} digest={no_lane_digest} L/R={:.6}",
        ratio(base_l, base_r)
    );
    // 判据 1：修前（本判据在 main 上跑）泳道被忽略 ⇒ 与无泳道相同；下面这三行在修后
    // 必须**不**成立 —— 方向性就是“自动化真的改了音频”的机械证据。
    assert!(early > 2.0, "前段必须左重（全左附近）: L/R={early:.6}");
    assert!(late < 0.5, "后段必须右重（全右附近）: L/R={late:.6}");
    assert!(early > late, "两段 L/R 比必须反向: {early} vs {late}");
    // 正对照：无泳道工程左右相等。
    assert!((ratio(base_l, base_r) - 1.0).abs() < 1e-6);
    // 判据 5/6：两条导出同一份浮点母带（MCP 侧同名常量）。
    assert_eq!(
        digest, PAN_RAMP_MASTER_DIGEST,
        "声相渐强的浮点母带必须与 MCP 侧逐位相同"
    );
}

/// ⭐ 判据 3/4：无泳道 / `readEnabled:false` / 曲线恒等于静态 `pan` ⇒ 逐字节相同。
#[test]
fn pan_identity_paths_are_byte_identical_to_no_lane() {
    let scratch = Scratch::new("identity");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let no_lane = project(
        &Tune {
            points: &[],
            read_enabled: false,
            track_pan: 0.0,
            insert_lane: false,
        },
        &bytes,
        400,
    );
    let (_, baseline_sha, baseline_digest) = export(&scratch, "base", &no_lane, &bytes);

    let disabled = project(
        &Tune {
            points: &[(0, -1.0), (ONE_SECOND_TICKS, 1.0)],
            read_enabled: false,
            track_pan: 0.0,
            insert_lane: true,
        },
        &bytes,
        410,
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
            track_pan: 0.0,
            insert_lane: true,
        },
        &bytes,
        420,
    );
    let (_, flat_sha, flat_digest) = export(&scratch, "flat", &flat, &bytes);
    println!(
        "### app pan 恒等: baseline={baseline_sha} disabled={disabled_sha} flat={flat_sha} / digest={baseline_digest}"
    );
    assert_eq!(flat_sha, baseline_sha, "静态恒等必须逐字节相同");
    assert_eq!(flat_digest, baseline_digest);
}

/// ⭐ 判据 6：同一 `pan` 值下，**自动化给的值 = 静态轨给的值**（本夹具轨增益 1.0
/// ⇒ 逐位相同；容差口径 ≤ 1 LSB，这里更强）。
#[test]
fn a_constant_pan_lane_equals_a_static_pan_track() {
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    // 自动化：静态 pan = 0，泳道恒定 -0.5。
    let automated = render(
        &project(
            &Tune {
                points: &[(0, -0.5)],
                read_enabled: true,
                track_pan: 0.0,
                insert_lane: true,
            },
            &bytes,
            500,
        ),
        &bytes,
    );
    // 静态：pan = -0.5，没有泳道。
    let static_track = render(
        &project(
            &Tune {
                points: &[],
                read_enabled: false,
                track_pan: -0.5,
                insert_lane: false,
            },
            &bytes,
            510,
        ),
        &bytes,
    );
    let mut max_ulp = 0u32;
    for (a, b) in automated.samples.iter().zip(static_track.samples.iter()) {
        let a = a.to_bits();
        let b = b.to_bits();
        max_ulp = max_ulp.max(a.abs_diff(b));
    }
    println!(
        "### app pan 自动化 vs 静态(-0.5): max_ulp={max_ulp} automated_digest={} static_digest={}",
        automated.digest, static_track.digest
    );
    assert!(max_ulp <= 1, "同一 pan 值必须 ≤ 1 LSB，实测 {max_ulp}");
    assert_eq!(
        automated.digest, static_track.digest,
        "本夹具（轨增益 1.0）下必须逐位相同"
    );
}

/// ⭐ 判据 `#337`：`pan = -1` 时**浮点层**右声道恰为 `0.0`（不是 `-0.0`、不是次正规），
/// 而 24-bit 抖动层允许 ±1 LSB —— 两个层次分开读数。
#[test]
fn a_hard_left_lane_zeroes_the_right_channel_in_the_float_master() {
    let scratch = Scratch::new("hard-left");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let project = project(
        &Tune {
            points: &[(0, -1.0)],
            read_enabled: true,
            track_pan: 0.0,
            insert_lane: true,
        },
        &bytes,
        600,
    );
    let float = render(&project, &bytes);
    let right_bits: Vec<u32> = float
        .samples
        .chunks(2)
        .map(|frame| frame[1].to_bits())
        .collect();
    let nonzero = right_bits.iter().filter(|bits| **bits != 0).count();
    let left_peak = float
        .samples
        .chunks(2)
        .fold(0.0f32, |acc, frame| acc.max(frame[0].abs()));
    let (file_samples, sha, _) = export(&scratch, "hard-left", &project, &bytes);
    let file_right_peak = file_samples
        .chunks(2)
        .fold(0.0f32, |acc, frame| acc.max(frame[1].abs()));
    println!(
        "### hard-left: float right nonzero={nonzero}/{} left_peak={left_peak} \
         file right peak={file_right_peak} ({:.3} LSB) sha={sha}",
        right_bits.len(),
        file_right_peak * I24_SCALE
    );
    assert_eq!(nonzero, 0, "浮点层右声道必须恰为 0.0");
    assert!(left_peak > 0.4, "左声道必须仍在: {left_peak}");
    assert!(
        file_right_peak * I24_SCALE <= 1.5,
        "抖动层只允许 ±1 LSB: {file_right_peak} ({:.3} LSB)",
        file_right_peak * I24_SCALE
    );
}
