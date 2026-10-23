//! 任务单 `#266`：**`DeviceParam` 自动化（本单只接 `output_gain_db`）在两条导出里都生效**。
//!
//! 这一份判据与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的设备参数判据**同一口径**：
//! 两条导出共用 `yeban_render::automation::DeviceParamAutomation`（模型唯一求值入口
//! `automation_value_at` ＋ **块边界** `InsertRuntime::apply_device_params`，
//! ⛔ 不逐样本平滑），曲线是**原生绝对值**（dB，⛔ 不是 0..1 归一化）。
//!
//! 音源刻意用**同一份 f32 常量音频片段**（两个导出都逐位透传解码）：这样
//! `masterDigest` 与四段峰值可以直接跨导出对账，而不是"两个合成器各弹各的"。
//!
//! 边界：⛔ 不证听感；只证以下可机械判定的东西：
//! 1. 修前——`output_gain_db` 泳道与无泳道的产物**逐字节相同**（自动化被忽略）；
//! 2. 修后——四段峰值严格单调递减（`0 → −12 dB`）；
//! 3. 正对照——无泳道工程的 sha256 / 浮点摘要修前/修后相同（钉成常量）；
//! 4. `readEnabled:false` ⇒ 等于没有泳道（逐字节）；
//! 5. 泳道处处等于静态值 ⇒ 与无泳道逐字节相同（⛔ 不是"乘 1.0 近似"）；
//! 6. 同一段曲线末端两条导出的 `masterDigest` 逐位相同（⇒ 同网格同读数）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_model::{
    AssetHash, AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget,
    AutomationWriteMode, ClipContent, ClipPlacement, ClipPoolEntry, CurveType, DeviceDefinition,
    DeviceKind, EntityId, LoopConfig, MediaKind, ParameterValue, RoutingEdge, RoutingKind,
    SampleRate, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::BitDepth;
use yeban_render::wav::read_plain_wav;

const FRAMES: u64 = 48_000;
const ONE_SECOND_TICKS: u64 = 1_920;
const I24_SCALE: f32 = 8_388_608.0;

/// `#266` 实测：`output_gain_db` 从 `0` 渐降到 `−12 dB` 的**浮点母带位级摘要**。
///
/// 与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的同名常量**逐字相同** ——
/// 两条导出各自钉住它 ⇒ “同一份浮点母带”是机械可判的。
const DEVICE_RAMP_MASTER_DIGEST: &str =
    "a47b0538ec3a4355c5349ef855fae5a1bfae92533aa84dc2591119abf4494157";

/// 正对照：**没有泳道**的同一份工程（设备参数静态 `0 dB`）的浮点母带摘要。
///
/// 这个值在修前（`main`）实测、修后必须**一字不变** ⇒ 判据 3 的机械锚。
const NO_LANE_MASTER_DIGEST: &str =
    "cad5bf7a9e4a53621af13476ae8bcc2c653acacb66cc91af57fd728aaee46e7c";

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
            "yeban-266-{tag}-{}-{}",
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

/// 常量 0.5 的立体声素材：左右都是同一个值 ⇒ 输出电平只由设备增益决定。
fn constant(frames: usize, level: f32) -> Vec<f32> {
    vec![level; frames * 2]
}

// ---------------------------------------------------------------------------
// 工程：Master ← Lead(Audio 片段) ＋ 一台只写 `output_gain_db` 的内置效果器
// ---------------------------------------------------------------------------

/// 夹具旋钮：泳道形状、`readEnabled`、设备参数表三个自由度。
struct Tune<'a> {
    points: &'a [(u64, f32)],
    read_enabled: bool,
    /// 设备的参数表。**第 0 位就是泳道指向的那个参数**（`param_index = 0`）——
    /// `#267` 的矩阵用这一点把"被测名字"放在 `params[0]`。
    params: &'a [(&'a str, f32)],
    /// 是否真的插入泳道对象（空点 + 关掉 = “没有泳道”与“关掉的空泳道”的对照）。
    insert_lane: bool,
}

fn project(tune: &Tune<'_>, bytes: &[u8], seed: u32) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let clip = id(10);
    let placement = id(20);
    let edge = id(40);
    let device = id(101);
    let asset = AssetHash::of_bytes(bytes);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "device param automation fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0266,
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
        pan: 0.0,
        devices: vec![DeviceDefinition {
            id: device,
            name: "strip".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: tune
                .params
                .iter()
                .map(|(name, value)| ParameterValue {
                    name: (*name).to_owned(),
                    value: *value,
                    unit: None,
                })
                .collect(),
            latency_samples: 0,
        }],
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
        let target = AutomationTarget::DeviceParam {
            track_id: lead,
            slot_index: 0,
            param_index: 0,
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

/// **没有泳道**的对照工程（设备参数静态 `0 dB`）。
fn no_lane(bytes: &[u8], seed: u32) -> YebanProjectV1 {
    project(
        &Tune {
            points: &[],
            read_enabled: false,
            params: &[("output_gain_db", 0.0)],
            insert_lane: false,
        },
        bytes,
        seed,
    )
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

/// 4 等段的**峰值**（左右声道取最大绝对样本；与 MCP 侧 `four_window_peaks` 同一网格）。
fn four_window_peaks(samples: &[f32]) -> [f32; 4] {
    let frames = samples.len() / 2;
    let window = frames / 4;
    let mut peaks = [0.0f32; 4];
    for (index, peak) in peaks.iter_mut().enumerate() {
        let start = index * window;
        let end = (start + window).min(frames);
        *peak = (start..end).fold(0.0f32, |acc, frame| {
            acc.max(samples[frame * 2].abs())
                .max(samples[frame * 2 + 1].abs())
        });
    }
    peaks
}

/// ⭐ 判据 1/2/6：`output_gain_db` 从 `0` 渐降到 `−12 dB` ⇒ 四段峰值严格单调递减，
/// 且浮点母带与 MCP 侧同名常量逐位相同。
#[test]
fn a_device_param_ramp_lowers_the_export_and_matches_the_mcp_digest() {
    let scratch = Scratch::new("ramp");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let ramp = project(
        &Tune {
            points: &[(0, 0.0), (ONE_SECOND_TICKS, -12.0)],
            read_enabled: true,
            params: &[("output_gain_db", 0.0)],
            insert_lane: true,
        },
        &bytes,
        300,
    );
    let (samples, sha, digest) = export(&scratch, "ramp", &ramp, &bytes);
    let (base, base_sha, base_digest) = export(&scratch, "no-lane", &no_lane(&bytes, 350), &bytes);
    let peaks = four_window_peaks(&samples);
    let base_peaks = four_window_peaks(&base);
    println!(
        "### app device-param ramp: sha={sha} digest={digest} peaks={peaks:?}\n\
         ###   no-lane: sha={base_sha} digest={base_digest} peaks={base_peaks:?}"
    );
    // 判据 1 的反面（修前的读数在 `#266` 报告里逐字给出）：自动化必须真的改字节。
    assert_ne!(sha, base_sha, "有泳道的产物必须与无泳道不同");
    // 判据 2：四段严格单调递减。
    assert!(
        peaks[0] > peaks[1] && peaks[1] > peaks[2] && peaks[2] > peaks[3],
        "四段峰值必须严格单调递减: {peaks:?}"
    );
    // 判据 3 的对照一半：无泳道工程的浮点母带与修前逐位相同。
    assert_eq!(base_digest, NO_LANE_MASTER_DIGEST, "无泳道工程不许变化");
    // 判据 6：与 MCP 侧同一份浮点母带。
    assert_eq!(
        digest, DEVICE_RAMP_MASTER_DIGEST,
        "设备参数渐降的浮点母带必须与 MCP 侧逐位相同"
    );
}

/// 压缩夹具的设备参数（硬膝 + 短弹道 ⇒ 稳态 GR 可手算；`params[0]` 是
/// `threshold_db`，泳道指向 `param_index = 0`）。
fn compressor_params() -> &'static [(&'static str, f32)] {
    &[
        ("threshold_db", -30.0),
        ("ratio", 4.0),
        ("knee_db", 0.0),
        ("detector_s", 0.005),
        ("attack_s", 0.002),
        ("release_s", 0.050),
        ("makeup_db", 0.0),
    ]
}

/// ⭐ `#304` 判据 2/3：`threshold_db` 泳道在 **app 导出**里真的改音频
/// （`sha` / `digest` 都变、四段峰值随阈值上升而上升），且"处处等于静态值"的泳道
/// **逐字节等于**无泳道。
///
/// 常量 0.5 素材的检波电平 −6.02 dBFS：静态 `T = −30 dBFS` ⇒ 稳态 GR ≈ 17.98 dB；
/// 泳道末端 `T = −12 dBFS` ⇒ GR ≈ 4.49 dB ⇒ 峰值上升。
/// GR 器件读数在 `crates/yeban-render/tests/insert_chain.rs` 的同名判据里。
#[test]
fn an_automated_compressor_threshold_changes_the_export_and_flat_is_byte_identical() {
    let scratch = Scratch::new("compressor-threshold");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let params = compressor_params();

    let base = project(
        &Tune {
            points: &[],
            read_enabled: false,
            params,
            insert_lane: false,
        },
        &bytes,
        500,
    );
    let (base_samples, base_sha, base_digest) = export(&scratch, "compressor-base", &base, &bytes);

    let ramp = project(
        &Tune {
            points: &[(0, -30.0), (ONE_SECOND_TICKS, -12.0)],
            read_enabled: true,
            params,
            insert_lane: true,
        },
        &bytes,
        510,
    );
    let (samples, sha, digest) = export(&scratch, "compressor-ramp", &ramp, &bytes);
    let peaks = four_window_peaks(&samples);
    let base_peaks = four_window_peaks(&base_samples);
    println!(
        "### #304 app compressor: sha={sha} digest={digest} peaks={peaks:?}\n\
         ###   no-lane: sha={base_sha} digest={base_digest} peaks={base_peaks:?}"
    );
    assert_ne!(
        sha, base_sha,
        "压缩阈值泳道必须改文件字节（静默无效会得到相同 sha）"
    );
    assert_ne!(
        digest, base_digest,
        "压缩阈值泳道必须改浮点母带（静默无效会得到相同 digest）"
    );
    // 窗口 0 含压缩器 attack 的**初始瞬态**（前几毫秒还没压下去）⇒ 不参与单调方向；
    // 后三段必须随阈值上升（压缩变少）严格上升，且末端显著高于静态 −30 的峰值。
    assert!(
        peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "阈值上升 ⇒ 后三段峰值必须上升（窗口 0 是 attack 瞬态）: {peaks:?}"
    );
    assert!(
        peaks[3] > base_peaks[3] * 2.0,
        "泳道末端少压 ⇒ 峰值必须显著高于静态 −30: {peaks:?} vs base {base_peaks:?}"
    );

    // ---- 正对照：处处等于静态值 ⇒ 逐字节等于无泳道 ----
    let flat = project(
        &Tune {
            points: &[(0, -30.0), (960, -30.0), (ONE_SECOND_TICKS, -30.0)],
            read_enabled: true,
            params,
            insert_lane: true,
        },
        &bytes,
        520,
    );
    let (_, flat_sha, flat_digest) = export(&scratch, "compressor-flat", &flat, &bytes);
    println!("### #304 app compressor flat: sha={flat_sha} digest={flat_digest}");
    assert_eq!(flat_sha, base_sha, "处处等于静态值 ⇒ 必须逐字节等于无泳道");
    assert_eq!(flat_digest, base_digest, "浮点母带也必须相同");
}

/// ⭐ 判据 3/4/5：无泳道 / `readEnabled:false` / 曲线恒等于静态值 ⇒ 逐字节相同。
#[test]
fn device_param_identity_paths_are_byte_identical_to_no_lane() {
    let scratch = Scratch::new("identity");
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let baseline = no_lane(&bytes, 400);
    let (_, baseline_sha, baseline_digest) = export(&scratch, "base", &baseline, &bytes);

    let disabled = project(
        &Tune {
            points: &[(0, 0.0), (ONE_SECOND_TICKS, -12.0)],
            read_enabled: false,
            params: &[("output_gain_db", 0.0)],
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
            params: &[("output_gain_db", 0.0)],
            insert_lane: true,
        },
        &bytes,
        420,
    );
    let (_, flat_sha, flat_digest) = export(&scratch, "flat", &flat, &bytes);
    println!(
        "### app device-param 恒等: baseline={baseline_sha} disabled={disabled_sha} flat={flat_sha} / digest={baseline_digest}"
    );
    assert_eq!(flat_sha, baseline_sha, "静态恒等必须逐字节相同");
    assert_eq!(flat_digest, baseline_digest);
    // 判据 3：修前实测的无泳道摘要，修后一字不变。
    assert_eq!(
        baseline_digest, NO_LANE_MASTER_DIGEST,
        "无泳道工程的浮点母带修前/修后必须相同"
    );
}

/// ⭐ 额外：泳道恒定 `−12 dB` ＝ 设备参数静态 `−12 dB`（**原生绝对值**，⛔ 不是比率/归一化）。
#[test]
fn a_constant_device_param_lane_equals_a_static_device_param() {
    let bytes = wav_f32(2, 48_000, &constant(48_000, 0.5));
    let automated = render(
        &project(
            &Tune {
                points: &[(0, -12.0)],
                read_enabled: true,
                params: &[("output_gain_db", 0.0)],
                insert_lane: true,
            },
            &bytes,
            500,
        ),
        &bytes,
    );
    let static_device = render(
        &project(
            &Tune {
                points: &[],
                read_enabled: false,
                params: &[("output_gain_db", -12.0)],
                insert_lane: false,
            },
            &bytes,
            510,
        ),
        &bytes,
    );
    let mut max_ulp = 0u32;
    for (a, b) in automated.samples.iter().zip(static_device.samples.iter()) {
        let a = a.to_bits();
        let b = b.to_bits();
        max_ulp = max_ulp.max(a.abs_diff(b));
    }
    println!(
        "### app device-param 自动化(-12dB) vs 静态(-12dB): max_ulp={max_ulp} \
         automated_digest={} static_digest={}",
        automated.digest, static_device.digest
    );
    assert_eq!(
        automated.digest, static_device.digest,
        "同一个原生 dB 值必须给出同一份母带（max_ulp={max_ulp}）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ `#267`：**通道条整组字段**（14 个字段 / 27 个拼写）在两条导出里都生效；
// `#293` 的 Freeverb 标量另有一条端到端判据（见文件末尾）。
// ---------------------------------------------------------------------------

/// 一段正弦（左右相同）：`freq` 让被测字段"听得见"（增益看电平、EQ 看频段、
/// 截止/谐振看它自己的中心频率附近的幅度）。
fn sine(frames: usize, freq: f32, level: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    let step = core::f32::consts::TAU * freq / 48_000.0;
    for frame in 0..frames {
        let value = level * (step * frame as f32).sin();
        out.push(value);
        out.push(value);
    }
    out
}

/// 四段峰值的**方向**（判据 1 的可机械判定形状）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trend {
    /// 逐段**严格**递增。
    Rising,
    /// 逐段**严格**递减。
    Falling,
    /// 开关字段：阶跃 ⇒ 逐段**非增**，且首段 > 末段（严格）。
    ///
    /// ⚠ 开关不是"渐变"：`0.0 → 1.0` 在第一个块边界就翻真（`value != 0.0`），
    /// 于是四段峰值里必然有两段相等 —— "严格单调"对布尔量不成立，⛔ 不是放宽。
    StepDown,
}

/// 一个**字段**一条用例。`params[0]` 就是被测名字（泳道指向 `param_index = 0`），
/// `from` 必须等于 `params[0].1`（否则"恒等"那条就不是恒等）。
struct FieldCase {
    name: &'static str,
    params: &'static [(&'static str, f32)],
    freq: f32,
    from: f32,
    to: f32,
    trend: Trend,
}

/// 14 个**增益/EQ/滤波**通道条字段 —— 每个都用**规范名**（27 个拼写里的别名由
/// `yeban-render` 的 `every_applied_name_is_applied_and_ramps_monotonically`
/// 与 `every_applied_name_projects_to_its_own_field` 逐条覆盖；别名与规范名
/// 在 `StripField::from_name` 里解析到**同一个**字段 ⇒ 同一条代码路径）。
///
/// ⭐ `#304`：压缩 8 字段（14 拼写）**不在这张矩阵**里 —— 它们的趋势依赖电平 / 比率
/// 与弹道，单独由 `an_automated_compressor_threshold_changes_the_export_and_flat_is_byte_identical`
/// 与 render 级 GR 判据覆盖；机械覆盖（55 拼写逐个）在 `yeban-render` 的单元判据里。
const FIELD_CASES: &[FieldCase] = &[
    FieldCase {
        name: "input_gain_db",
        params: &[("input_gain_db", 0.0)],
        freq: 1_000.0,
        from: 0.0,
        to: -18.0,
        trend: Trend::Falling,
    },
    FieldCase {
        name: "output_gain_db",
        params: &[("output_gain_db", 0.0)],
        freq: 1_000.0,
        from: 0.0,
        to: -18.0,
        trend: Trend::Falling,
    },
    FieldCase {
        name: "eq_enabled",
        params: &[("eq_enabled", 0.0), ("eq_low_gain", -24.0)],
        freq: 120.0,
        from: 0.0,
        to: 1.0,
        trend: Trend::StepDown,
    },
    FieldCase {
        name: "filter_enabled",
        params: &[("filter_enabled", 0.0), ("cutoff_hz", 300.0)],
        freq: 2_000.0,
        from: 0.0,
        to: 1.0,
        trend: Trend::StepDown,
    },
    FieldCase {
        name: "eq_low_gain",
        params: &[("eq_low_gain", 0.0)],
        freq: 120.0,
        from: 0.0,
        to: 12.0,
        trend: Trend::Rising,
    },
    FieldCase {
        name: "eq_low_freq",
        params: &[("eq_low_freq", 100.0), ("eq_low_gain", 12.0)],
        freq: 1_000.0,
        from: 100.0,
        to: 1_000.0,
        trend: Trend::Rising,
    },
    FieldCase {
        name: "eq_mid_gain",
        params: &[("eq_mid_gain", 0.0)],
        freq: 1_000.0,
        from: 0.0,
        to: 12.0,
        trend: Trend::Rising,
    },
    FieldCase {
        name: "eq_mid_freq",
        params: &[("eq_mid_freq", 1_000.0), ("eq_mid_gain", 12.0)],
        freq: 1_000.0,
        from: 1_000.0,
        to: 8_000.0,
        trend: Trend::Falling,
    },
    FieldCase {
        name: "eq_mid_q",
        params: &[
            ("eq_mid_q", 0.4),
            ("eq_mid_gain", 12.0),
            ("eq_mid_freq", 1_000.0),
        ],
        freq: 1_500.0,
        from: 0.4,
        to: 4.0,
        trend: Trend::Falling,
    },
    FieldCase {
        name: "eq_high_gain",
        params: &[("eq_high_gain", 0.0)],
        freq: 6_000.0,
        from: 0.0,
        to: 12.0,
        trend: Trend::Rising,
    },
    FieldCase {
        name: "eq_high_freq",
        params: &[("eq_high_freq", 2_000.0), ("eq_high_gain", 12.0)],
        freq: 8_000.0,
        from: 2_000.0,
        to: 12_000.0,
        trend: Trend::Falling,
    },
    FieldCase {
        name: "cutoff_hz",
        params: &[("cutoff_hz", 20_000.0)],
        freq: 2_000.0,
        from: 20_000.0,
        to: 500.0,
        trend: Trend::Falling,
    },
    FieldCase {
        name: "resonance",
        params: &[("resonance", 0.0), ("cutoff_hz", 1_000.0)],
        freq: 1_000.0,
        from: 0.0,
        to: 0.9,
        trend: Trend::Rising,
    },
    FieldCase {
        name: "drive",
        params: &[("drive", 0.0), ("cutoff_hz", 20_000.0)],
        freq: 1_000.0,
        from: 0.0,
        to: 1.0,
        // ⚠ 实测：drive 是**饱和器的输入量**，推满会把 1 kHz 正弦压得更低
        // （0.1995 → 0.1962 → 0.1950 → 0.1937）⇒ 方向是**下降**。
        // ⛔ 不按"驱动=更响"的直觉写成 Rising。
        trend: Trend::Falling,
    },
];

fn assert_trend(case: &FieldCase, peaks: [f32; 4]) {
    let ok = match case.trend {
        Trend::Rising => peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        Trend::Falling => peaks[0] > peaks[1] && peaks[1] > peaks[2] && peaks[2] > peaks[3],
        Trend::StepDown => {
            // 阶跃 ⇒ 第一段（还带着关闭时的电平）明显更高，其后三段**持平**
            // （1% 容差：器件状态不会在阶跃后逐位恒定）。
            peaks[0] > peaks[1] * 1.1
                && peaks[1] > 0.0
                && peaks[3] <= peaks[1] * 1.01
                && peaks[3] < peaks[0]
        }
    };
    assert!(
        ok,
        "{}: 四段峰值方向不对（期望 {:?}）: {peaks:?}",
        case.name, case.trend
    );
    assert_eq!(
        case.from.to_bits(),
        case.params[0].1.to_bits(),
        "{}: from 必须等于静态值（否则恒等那条不是恒等）",
        case.name
    );
}

/// ⭐ `#267` 判据 1/3/4（每个字段一条，两条导出各跑一遍）：渐变 ⇒ 四段峰值按方向
/// 单调 **＋ `sha256` 变**；泳道处处等于静态值 ⇒ 逐字节等于无泳道；`readEnabled:false`
/// ⇒ 逐字节等于无泳道。
#[test]
fn every_gain_eq_filter_field_changes_the_export_and_is_byte_identical_when_flat() {
    let scratch = Scratch::new("fields");
    for (index, case) in FIELD_CASES.iter().enumerate() {
        let seed = 600 + (index as u32) * 10;
        // 源电平 0.2：最坏情况（+12 dB）也只到 0.8 ⇒ ⛔ 不撞 24-bit 满刻度
        // （撞顶会把"单调"读成两段 1.0）。
        let bytes = wav_f32(2, 48_000, &sine(FRAMES as usize, case.freq, 0.2));
        let base = project(
            &Tune {
                points: &[],
                read_enabled: false,
                params: case.params,
                insert_lane: false,
            },
            &bytes,
            seed,
        );
        let (base_samples, base_sha, base_digest) = export(&scratch, "base", &base, &bytes);
        let base_peaks = four_window_peaks(&base_samples);

        let ramp = project(
            &Tune {
                points: &[(0, case.from), (ONE_SECOND_TICKS, case.to)],
                read_enabled: true,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed + 1,
        );
        let (samples, ramp_sha, ramp_digest) = export(&scratch, "ramp", &ramp, &bytes);
        let peaks = four_window_peaks(&samples);
        assert_ne!(
            ramp_sha, base_sha,
            "{}: 渐变必须改字节（自动化没落地？）",
            case.name
        );
        assert_ne!(ramp_digest, base_digest, "{}: 浮点母带也要变", case.name);
        assert_trend(case, peaks);
        println!(
            "### #267 {}: sha={ramp_sha} digest={ramp_digest} peaks={peaks:?} \
             (base sha={base_sha} peaks={base_peaks:?})",
            case.name
        );

        let flat = project(
            &Tune {
                points: &[
                    (0, case.from),
                    (960, case.from),
                    (ONE_SECOND_TICKS, case.from),
                ],
                read_enabled: true,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed + 2,
        );
        let (_, flat_sha, flat_digest) = export(&scratch, "flat", &flat, &bytes);
        assert_eq!(
            flat_sha, base_sha,
            "{}: 处处等于静态值 ⇒ 必须逐字节等于无泳道",
            case.name
        );
        assert_eq!(flat_digest, base_digest, "{}", case.name);

        let disabled = project(
            &Tune {
                points: &[(0, case.from), (ONE_SECOND_TICKS, case.to)],
                read_enabled: false,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed + 3,
        );
        let (_, disabled_sha, disabled_digest) = export(&scratch, "disabled", &disabled, &bytes);
        assert_eq!(
            disabled_sha, base_sha,
            "{}: readEnabled:false ⇒ 必须逐字节等于无泳道",
            case.name
        );
        assert_eq!(disabled_digest, base_digest, "{}", case.name);
    }
}

/// ⭐ `#267` 判据 5：**每一个通道条字段**的 `masterDigest` 在两条导出（CLI 的
/// `render_project_f32` 与 MCP 的 `render::build`）里**逐位相同**。
///
/// 为什么这条判据挂在 `yeban-app` 的测试目标里而不是 `yeban-mcp`：MCP 不能反向依赖
/// `yeban-app`（`scripts/gates/check_mcp_dependency_direction.py`），而**同一份夹具**
/// 必须被两条导出各渲染一次才能比 —— 夹具与 CLI 侧都在这里，MCP 侧通过
/// `in-process-mcp` 这个 feature 直接调它的领域渲染本体（⛔ 不走 socket，⛔ 不复制
/// 夹具）。没有这个 feature 时本判据不编译（判据 1/3/4 仍在默认档覆盖每个字段）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn every_gain_eq_filter_field_gives_the_same_master_in_both_exports() {
    /// MCP 侧固定注入时钟（与 `yeban-mcp/tests/render_audio_clips.rs` 同值）。
    const NOW_MS: u64 = 1_760_000_000_000;
    let scratch = Scratch::new("two-exports");
    for (index, case) in FIELD_CASES.iter().enumerate() {
        let seed = 900 + (index as u32) * 10;
        let bytes = wav_f32(2, 48_000, &sine(FRAMES as usize, case.freq, 0.5));
        let fixture = project(
            &Tune {
                points: &[(0, case.from), (ONE_SECOND_TICKS, case.to)],
                read_enabled: true,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed,
        );
        // CLI 侧：浮点母带摘要（`render_project_f32`）。
        let cli = render(&fixture, &bytes);
        // MCP 侧：领域渲染本体（`yeban_render_master` 的同一条装配）。
        let mcp_path = scratch.wav("mcp");
        let arguments = serde_json::json!({
            "format": "wav",
            "sampleRate": 48_000,
            "frames": FRAMES,
            "path": mcp_path.display().to_string(),
        });
        let request = yeban_mcp::domain::render::validate(arguments.as_object().expect("参数对象"))
            .expect("MCP 渲染参数合法");
        let asset = AssetHash::of_bytes(&bytes);
        let pool: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::from([(asset, bytes.clone())]);
        let artifact = yeban_mcp::domain::render::build(
            &fixture,
            &mcp_path.with_extension("project.yeban"),
            &request,
            NOW_MS,
            &pool,
        )
        .expect("MCP 渲染必须成功");
        let data = artifact.response_data();
        let mcp_digest = data["masterDigest"].as_str().expect("masterDigest");
        println!(
            "### #267 two-exports {}: cli={} mcp={mcp_digest} sha={}",
            case.name, cli.digest, data["sha256"]
        );
        assert_eq!(
            cli.digest, mcp_digest,
            "{}: 两条导出的浮点母带必须逐位相同",
            case.name
        );
        // 泳道在这一侧也必须被报成"真的应用了"（⛔ 不是一条静默的）。
        let lane = &data["automation"]["tracks"][0]["lanes"][0];
        assert_eq!(lane["target"], "DeviceParam", "{}", case.name);
        assert_eq!(lane["supported"], true, "{}", case.name);
        assert_eq!(lane["applied"], true, "{}", case.name);
        assert_eq!(
            lane["resolvedParamName"].as_str().expect("名字"),
            case.name,
            "{}: 解析到的参数名",
            case.name
        );
        let listed = data["automation"]["appliedDeviceParamNames"]
            .as_array()
            .expect("appliedDeviceParamNames");
        assert_eq!(
            listed.len(),
            55,
            "MCP 名单必须逐条来自 yeban-render（55 个拼写：41 通道条（含压缩）+ 6 混响 + 8 卷积）: {data}"
        );
        for name in [
            "output_gain_db",
            "cutoff_hz",
            "reverb_wet",
            "reverb_size",
            "conv_wet",
            "conv_ir_gain_db",
        ] {
            assert!(
                listed.iter().any(|value| value == name),
                "{name} 必须在名单里: {data}"
            );
        }
        for name in ["conv_ir_decay_s", "conv_ir_seed"] {
            assert!(
                !listed.iter().any(|value| value == name),
                "{name} 不许在名单里: {data}"
            );
        }
        // ⭐ `#304`：压缩 14 拼写现在**必须**在名单里。
        for name in ["compressor_enabled", "threshold_db", "makeup_db"] {
            assert!(
                listed.iter().any(|value| value == name),
                "{name} 自 #304 起必须在名单里: {data}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// ⭐ `#293`：Freeverb 标量自动化（判据 1 的“复现/修后”改用混响；压缩器留作负对照）
// ---------------------------------------------------------------------------

/// 0.1 s 的 440 Hz 正弦，之后是精确静音（用于量“音符结束后”的混响尾音）。
fn tone_burst(frames: usize, note_frames: usize, level: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let step = core::f32::consts::TAU * 440.0 / 48_000.0;
    let fade = 240usize; // 5 ms
    for frame in 0..note_frames.min(frames) {
        let mut envelope = 1.0f32;
        if frame < fade {
            envelope = frame as f32 / fade as f32;
        }
        let remaining = note_frames.min(frames) - frame;
        if remaining < fade {
            envelope *= remaining as f32 / fade as f32;
        }
        let value = level * envelope * (step * frame as f32).sin();
        out[2 * frame] = value;
        out[2 * frame + 1] = value;
    }
    out
}

fn window_rms(samples: &[f32], start: usize, end: usize) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for frame in start..end {
        for channel in 0..2 {
            let value = f64::from(samples[frame * 2 + channel]);
            sum += value * value;
            count += 1;
        }
    }
    (sum / count as f64).sqrt()
}

/// ⭐ `#293` 判据 1/2：**只写 `reverb_wet: 0`** 的源设备 + 一条非恒等 `reverb_wet`
/// 泳道（0 → 1）⇒ 修前（旧装配）没有运行态器件，导出与无泳道逐字节相同；
/// 修后必须真的建出 Freeverb、音频真的变（尾音 > 0），并且不是“报 applied 却静默无效”。
///
/// 正对照：同一工程**无泳道**时，静态 `reverb_wet: 0` 的链不建运行态器件 ⇒ 摘要与
/// “同一段素材、根本没有那台设备”的工程**逐位相同**。
#[test]
fn a_reverb_wet_zero_source_really_changes_the_export_when_automated() {
    let scratch = Scratch::new("reverb-automation");
    let bytes = wav_f32(2, 48_000, &tone_burst(48_000, 4_800, 0.5));

    let bare = project(
        &Tune {
            points: &[],
            read_enabled: false,
            params: &[],
            insert_lane: false,
        },
        &bytes,
        690,
    );
    let (_, bare_sha, bare_digest) = export(&scratch, "reverb-bare", &bare, &bytes);

    let baseline = project(
        &Tune {
            points: &[],
            read_enabled: false,
            params: &[("reverb_wet", 0.0)],
            insert_lane: false,
        },
        &bytes,
        700,
    );
    let (base_samples, base_sha, base_digest) = export(&scratch, "reverb-base", &baseline, &bytes);
    assert_eq!(
        base_digest, bare_digest,
        "无泳道 + 静态 wet=0 ⇒ 必须与无设备工程逐位相同（不建器件）"
    );
    assert_eq!(base_sha, bare_sha);
    let base_tail = window_rms(&base_samples, 7_200, 24_000);
    assert!(
        base_tail < 1.0e-5,
        "静态 wet=0 ⇒ 音符结束后必须接近静音（24-bit 抖动底噪）: {base_tail:.9e}"
    );

    let automated = project(
        &Tune {
            points: &[(0, 0.0), (960, 1.0), (1_920, 1.0)],
            read_enabled: true,
            params: &[("reverb_wet", 0.0)],
            insert_lane: true,
        },
        &bytes,
        710,
    );
    let (samples, sha, digest) = export(&scratch, "reverb-auto", &automated, &bytes);
    let tail = window_rms(&samples, 7_200, 24_000);
    println!(
        "### #293 reverb_wet 0→1: base_sha={base_sha} auto_sha={sha} \
         base_digest={base_digest} auto_digest={digest} \
         tail(base)={base_tail:.9e} tail(auto)={tail:.9e}"
    );
    assert_ne!(sha, base_sha, "修后：混响自动化必须真的改字节");
    assert_ne!(digest, base_digest, "修后：浮点母带必须真的不同");
    assert!(
        tail > 1.0e-5,
        "修后：音符结束后的混响尾音必须大于零（静默无效会得到 {tail:.9e}）"
    );
    assert!(
        tail > base_tail * 1_000.0,
        "湿尾必须远高于干尾: {tail:.9e} vs {base_tail:.9e}"
    );
}

// ---------------------------------------------------------------------------
// ⭐ `#294`：`#293` 剩下的 **7 个字段**逐个端到端音频
// （混响 4 ＋ 卷积 4 = 8 个字段，减去 `#293` 已做过的 `reverb_wet`）。
//
// `#293` 已给 `reverb_wet` 一条端到端判据；其它 7 个字段只有落点/镜像/单调判据
// （`inserts.rs` 的 `*_field_clamp_is_the_device_clamp` 与 `automation.rs` 的
// `every_applied_name_*`），**没有**“静态基线 vs 有泳道”的音频读数。本切片按
// 同一条形状补齐：每个字段都给 `base sha` / `auto sha` / `digest` / 分段读数，
// 且把“泳道恒等于静态值 ⇒ 逐字节等于无泳道”与“`readEnabled:false` ⇒ 逐字节
// 等于无泳道”两条正对照逐字段跑一遍。
//
// 单调读数按字段的**物理方向**选，不统一硬套峰值：
// - `size`：尾音电平 —— 反馈变长 ⇒ 同一尾窗的能量变大；
// - `damp`：尾音电平 —— 高频阻尼变强 ⇒ 同一尾窗的能量变小；
// - `predelay`（混响/卷积）：尾音**能量重心**（秒）—— 湿路被整体推迟 ⇒ 重心变大
//   （两者的湿路都是 LTI，推迟不改变包络形状）；
// - `conv_dry`：整段 RMS —— 干路电平变大；
// - `conv_wet` / `conv_ir_gain_db`：卷积尾窗 RMS —— 湿路电平/IR 增益变大。
// ---------------------------------------------------------------------------

/// 单调读数。窗口单位：帧（48 kHz）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModRead {
    /// `[start, end)` 的 RMS（左右声道合并，与 `window_rms` 同口径）。
    WindowRms { start: usize, end: usize },
    /// `[start, end)` 内 `|x|²` 的能量重心，单位：秒。
    Centroid { start: usize, end: usize },
}

/// 字段对读数的作用方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModDirection {
    Rising,
    Falling,
}

/// 一个字段一条端到端用例。
struct ModCase {
    name: &'static str,
    /// 源设备的静态参数表。`params[0]` 就是被测名字（泳道指向 `param_index = 0`）。
    params: &'static [(&'static str, f32)],
    from: f32,
    mid: f32,
    to: f32,
    read: ModRead,
    direction: ModDirection,
}

const MOD_CASES: &[ModCase] = &[
    ModCase {
        name: "reverb_size",
        params: &[("reverb_size", 0.2)],
        from: 0.2,
        mid: 0.6,
        to: 1.0,
        // 0.30 s … 0.60 s：0.1 s 音符结束后、`to` 已保持的尾窗。
        read: ModRead::WindowRms {
            start: 14_400,
            end: 28_800,
        },
        direction: ModDirection::Rising,
    },
    ModCase {
        name: "reverb_damp",
        params: &[("reverb_damp", 0.0)],
        from: 0.0,
        mid: 0.5,
        to: 1.0,
        read: ModRead::WindowRms {
            start: 14_400,
            end: 28_800,
        },
        direction: ModDirection::Falling,
    },
    ModCase {
        name: "reverb_predelay_s",
        params: &[("reverb_predelay", 0.0)],
        from: 0.0,
        mid: 0.05,
        to: 0.1,
        read: ModRead::Centroid {
            start: 4_800,
            end: 43_200,
        },
        direction: ModDirection::Rising,
    },
    ModCase {
        name: "conv_predelay_s",
        params: &[("conv_predelay_s", 0.0)],
        from: 0.0,
        mid: 0.05,
        to: 0.1,
        read: ModRead::Centroid {
            start: 4_800,
            end: 24_000,
        },
        direction: ModDirection::Rising,
    },
    ModCase {
        name: "conv_dry",
        // 干路电平要**单独**读：把 IR 增益压到 −60 dB，湿路近似静音，整段 RMS
        // 就只随 `dry` 走。⚠ 不这么做会读出**反方向**：同一夹具、源电平 0.5、
        // IR 增益 0 dB 时，dry 0→0.5→1 的总 RMS 是 0.3516→0.3487→0.3477
        // （湿路比干路响，干湿相加后总 RMS 反而下降）—— 那是混音读数的形状，
        // 不是 `dry` 这个字段的方向。
        params: &[("conv_dry", 0.0), ("conv_ir_gain_db", -60.0)],
        from: 0.0,
        mid: 0.5,
        to: 1.0,
        read: ModRead::WindowRms {
            start: 0,
            end: 48_000,
        },
        direction: ModDirection::Rising,
    },
    ModCase {
        name: "conv_wet",
        params: &[("conv_wet", 0.0)],
        from: 0.0,
        mid: 0.5,
        to: 1.0,
        // 0.11 s … 0.30 s：干路已结束（0.1 s），只剩卷积尾（IR 0.1 s）。
        read: ModRead::WindowRms {
            start: 5_280,
            end: 14_400,
        },
        direction: ModDirection::Rising,
    },
    ModCase {
        name: "conv_ir_gain_db",
        params: &[("conv_ir_gain_db", -60.0)],
        from: -60.0,
        mid: -18.0,
        to: 24.0,
        read: ModRead::WindowRms {
            start: 5_280,
            end: 14_400,
        },
        direction: ModDirection::Rising,
    },
];

fn mod_read(read: ModRead, samples: &[f32]) -> f64 {
    match read {
        ModRead::WindowRms { start, end } => window_rms(samples, start, end),
        ModRead::Centroid { start, end } => {
            let mut moment = 0.0f64;
            let mut energy = 0.0f64;
            for frame in start..end {
                let mut power = 0.0f64;
                for channel in 0..2 {
                    let value = f64::from(samples[frame * 2 + channel]);
                    power += value * value;
                }
                moment += frame as f64 * power;
                energy += power;
            }
            if energy == 0.0 {
                0.0
            } else {
                moment / energy / 48_000.0
            }
        }
    }
}

/// 3 点常量阶梯：`base`（无泳道，静态 `from`）→ `mid` → `to`。
///
/// 返回 `mid` / `to` 两个读数（`base` 由调用方传入）。⛔ 不用一条渐变曲线读
/// “单调”：渐变把参数与时间混在一起，而常量泳道让读数的**唯一自变量**就是字段值。
fn mod_ladder(scratch: &Scratch, bytes: &[u8], case: &ModCase, base: f64, seed: u32) -> (f64, f64) {
    let mut reads = [0.0f64; 2];
    for (slot, value) in [case.mid, case.to].into_iter().enumerate() {
        let fixture = project(
            &Tune {
                points: &[(0, value), (960, value), (ONE_SECOND_TICKS, value)],
                read_enabled: true,
                params: case.params,
                insert_lane: true,
            },
            bytes,
            seed + slot as u32,
        );
        let (samples, _, _) = export(scratch, &format!("ladder-{slot}"), &fixture, bytes);
        reads[slot] = mod_read(case.read, &samples);
    }
    assert!(
        match case.direction {
            ModDirection::Rising => base < reads[0] && reads[0] < reads[1],
            ModDirection::Falling => base > reads[0] && reads[0] > reads[1],
        },
        "{}: 常量阶梯必须按 {:?} 单调（base={base:.9e}, mid={:.9e}, to={:.9e}）",
        case.name,
        case.direction,
        reads[0],
        reads[1]
    );
    (reads[0], reads[1])
}

/// ⭐ `#294` 判据 1/2/3：**每个剩余标量字段**一条端到端判据（静态基线 vs 有泳道）。
///
/// 每条都断言：
/// 1. 非恒等泳道 ⇒ `sha256` 与浮点母带 `digest` **都变**（自动化真的落了地）；
/// 2. 泳道恒等于静态值 ⇒ **逐字节**等于无泳道（⛔ 不是“乘 1.0 近似”）；
/// 3. `readEnabled:false` ⇒ **逐字节**等于无泳道；
/// 4. 常量阶梯（`from` / `mid` / `to`）⇒ 读数按该字段的物理方向**严格单调**。
///
/// `conv_wet` 另加 `#293` 那条正对照：静态 `conv_wet: 0` 的链不建运行态器件 ⇒
/// 基线与“同一段素材、根本没有那台设备”的工程**逐位相同**。
#[test]
fn every_remaining_scalar_field_really_changes_the_export_and_moves_monotonically() {
    let names: Vec<&str> = MOD_CASES.iter().map(|case| case.name).collect();
    println!("### #294 fields ({}) = {names:?}", names.len());
    let scratch = Scratch::new("mod-fields");
    // 0.1 s 的 440 Hz 音符 + 0.9 s 静音：湿路的尾巴与干路（0..0.1 s）分开可读。
    // 源电平 0.05（⛔ 不是 0.5）：IR 是峰值归一化的，默认 `wet = 0.25`/`dry = 1`
    // 时干湿相加会撞 24-bit 满刻度，撞顶会把“分段读数”读成 1.0（实测 0.25 时
    // `conv_predelay_s` 的四段峰值首段仍是 1.0）。0.05 下所有读数都不撞顶。
    let bytes = wav_f32(2, 48_000, &tone_burst(48_000, 4_800, 0.05));

    for (index, case) in MOD_CASES.iter().enumerate() {
        let seed = 1_000 + (index as u32) * 10;

        // 1) 静态基线：设备参数静态 `from`，没有泳道。
        let baseline = project(
            &Tune {
                points: &[],
                read_enabled: false,
                params: case.params,
                insert_lane: false,
            },
            &bytes,
            seed,
        );
        let (base_samples, base_sha, base_digest) = export(&scratch, "base", &baseline, &bytes);
        let base_read = mod_read(case.read, &base_samples);
        let base_peaks = four_window_peaks(&base_samples);

        // 2) 有泳道：0 → `to`（0.25 s 内到顶，之后保持 `to`）。
        let automated = project(
            &Tune {
                points: &[(0, case.from), (480, case.to), (ONE_SECOND_TICKS, case.to)],
                read_enabled: true,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed + 1,
        );
        let (samples, sha, digest) = export(&scratch, "auto", &automated, &bytes);
        let read = mod_read(case.read, &samples);
        let peaks = four_window_peaks(&samples);
        println!(
            "### #294 {}: base_sha={base_sha} auto_sha={sha} \
             base_digest={base_digest} auto_digest={digest} \
             read(base)={base_read:.9e} read(auto)={read:.9e} \
             base_peaks={base_peaks:?} auto_peaks={peaks:?}",
            case.name
        );
        assert_ne!(sha, base_sha, "{}: 泳道必须真的改字节", case.name);
        assert_ne!(digest, base_digest, "{}: 浮点母带必须真的不同", case.name);

        // 3) 正对照：泳道处处等于静态值 ⇒ 逐字节等于无泳道。
        let flat = project(
            &Tune {
                points: &[
                    (0, case.from),
                    (960, case.from),
                    (ONE_SECOND_TICKS, case.from),
                ],
                read_enabled: true,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed + 2,
        );
        let (flat_samples, flat_sha, flat_digest) = export(&scratch, "flat", &flat, &bytes);
        assert_eq!(
            flat_sha, base_sha,
            "{}: 恒等泳道必须逐字节等于无泳道",
            case.name
        );
        assert_eq!(flat_digest, base_digest, "{}", case.name);
        assert_eq!(
            four_window_peaks(&flat_samples),
            base_peaks,
            "{}: 恒等泳道的分段读数也必须相同",
            case.name
        );

        // 4) 正对照：`readEnabled:false` ⇒ 逐字节等于无泳道。
        let disabled = project(
            &Tune {
                points: &[(0, case.from), (480, case.to), (ONE_SECOND_TICKS, case.to)],
                read_enabled: false,
                params: case.params,
                insert_lane: true,
            },
            &bytes,
            seed + 3,
        );
        let (_, disabled_sha, disabled_digest) = export(&scratch, "disabled", &disabled, &bytes);
        assert_eq!(
            disabled_sha, base_sha,
            "{}: readEnabled:false 必须逐字节等于无泳道",
            case.name
        );
        assert_eq!(disabled_digest, base_digest, "{}", case.name);

        // 5) `conv_wet` 的 `#293` 正对照：静态 wet=0 ⇒ 与“根本没有那台设备”逐位相同。
        if case.name == "conv_wet" {
            let bare = project(
                &Tune {
                    points: &[],
                    read_enabled: false,
                    params: &[],
                    insert_lane: false,
                },
                &bytes,
                seed + 4,
            );
            let (_, bare_sha, bare_digest) = export(&scratch, "bare", &bare, &bytes);
            assert_eq!(
                base_digest, bare_digest,
                "静态 conv_wet=0 ⇒ 必须与无设备工程逐位相同（不建器件）"
            );
            assert_eq!(base_sha, bare_sha);
        }

        // 6) 单调：常量阶梯。
        let (mid_read, to_read) = mod_ladder(&scratch, &bytes, case, base_read, seed + 5);
        println!(
            "### #294 {} ladder: from={} -> {base_read:.9e}, mid={} -> {mid_read:.9e}, \
             to={} -> {to_read:.9e}",
            case.name, case.from, case.mid, case.to
        );
    }
}

/// ⭐ `#294`：这 7 个名字就是“混响/卷积的 8 个可自动化字段减去 `reverb_wet`”
/// （`#293` 唯一做过端到端音频的那一个）。⚠ 卷积的 `conv_wet` **仍在**这 7 个里：
/// `#293` 只做了 `reverb_wet`。
///
/// ⛔ 不手抄第二份清单：名字从 `ReverbField::ALL` / `ConvolutionField::ALL` 现读，
/// 因此本判据与 `MOD_CASES` 互为对方的机械锚（任一边漏一个字段都会红）。
#[test]
fn the_seven_remaining_fields_are_the_scalar_fields_minus_reverb_wet() {
    use yeban_render::inserts::{ConvolutionField, ReverbField};

    let mut expected: Vec<&str> = Vec::new();
    for field in ReverbField::ALL {
        if *field != ReverbField::Wet {
            expected.push(field.canonical_name());
        }
    }
    for field in ConvolutionField::ALL {
        expected.push(field.canonical_name());
    }
    let covered: Vec<&str> = MOD_CASES.iter().map(|case| case.name).collect();
    println!("### #294 expected={expected:?}");
    println!("### #294 covered={covered:?}");
    assert_eq!(expected.len(), 7, "混响 3 ＋ 卷积 4 = 7");
    assert_eq!(covered, expected, "MOD_CASES 必须逐字覆盖这 7 个字段");
}
