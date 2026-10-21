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
// ⭐ `#267`：**通道条整组字段**（14 个字段 / 27 个拼写）在两条导出里都生效
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

/// 14 个字段 —— 每个都用**规范名**（27 个拼写里的别名由
/// `yeban-render` 的 `every_applied_name_is_applied_and_ramps_monotonically`
/// 与 `every_applied_name_projects_to_its_own_strip_field` 逐条覆盖；别名与规范名
/// 在 `StripField::from_name` 里解析到**同一个**字段 ⇒ 同一条代码路径）。
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
fn every_channel_strip_field_changes_the_export_and_is_byte_identical_when_flat() {
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
fn every_channel_strip_field_gives_the_same_master_in_both_exports() {
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
            27,
            "MCP 名单必须逐条来自 yeban-render（27 个拼写）: {data}"
        );
    }
}
