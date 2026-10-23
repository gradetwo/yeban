//! 任务单 `#303`：**压缩级**在 `--export-wav`（`render_core`）里真的执行。
//!
//! ## 本文件的四个读数（每个都有单位）
//!
//! | 判据 | 量 | 断言 |
//! | :--- | :--- | :--- |
//! | ① 复现 / 修后 | 母带浮点摘要（`sha256`）＋整段能量 | 有压缩 **≠** 无压缩；能量比 < 0.5 |
//! | ② 正对照 | 母带浮点摘要 | **没有** `threshold_db` 的工程 ⇒ 逐字节等于 `#303` 之前（钉成常量） |
//! | ③ 检波位置 | 三档声相的 `E_compressed / E_dry` | 三档**相等**（相对差 < 1e-5）⇒ 检波在声相之前 |
//! | ④ 边界（MIDI ＋ 音频片段） | 逐样本 ulp | 整通道乘 pan vs 逐源乘 pan 的差 ≤ 实测上界 |
//!
//! ## 边界（⛔ 不声称）
//!
//! - 没有 pin `pan != 0` 的绝对摘要：声相曲线走宿主 `f32::cos/sin`（`yeban-engine` 的
//!   `pan_gains`），跨机不可复现（台账第 313 条）。pan = 0 的摘要才是跨平台契约。
//! - ⛔ 不证听感：本文件只证"压缩改变了导出"、"阈值以下不变"、"压缩量与声相无关"。

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::export_wav::{WavExportOptions, export_wav_with_sfz_file_and_assets};
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_model::{
    AssetHash, AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget,
    AutomationWriteMode, ClipContent, ClipPlacement, ClipPoolEntry, CurveType, DeviceDefinition,
    DeviceKind, EntityId, LoopConfig, MediaKind, MidiNote, ParameterValue, RoutingEdge,
    RoutingKind, SampleRate, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::automation::centered_pan_gains;
use yeban_render::dither::BitDepth;

/// 1 秒 @ 48 kHz。
const FRAMES: u64 = 48_000;
/// 1 秒 @ 120 BPM / 960 PPQ = 1920 tick。
const ONE_SECOND_TICKS: u64 = 1_920;
/// 源素材：常量 0.5 的立体声。
const ASSET_FRAMES: usize = 48_000;

/// `#303` 的**正对照**：一份没有压缩参数的工程（增益级 `-6 dB`，`pan = 0`）的浮点母带
/// 摘要。它在 `#303` 改动**之前**与**之后**各实测一次，两次**逐字节相同**。
const NO_COMPRESSOR_MASTER_DIGEST: &str =
    "453ca2559c68507e5307aa31f9d9880677f583a4bae88a209520e9ffd1934f43";

fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZS{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 常量立体声素材（f32 WAV）。
fn constant_asset(level: f32) -> Vec<u8> {
    stereo_asset(|_| level)
}

/// 逐帧构造 L=+v／R=−v 的 1 s f32 立体声 WAV。
///
/// 检波器读 `max(l², r²)` ⇒ 对这份素材读到的是 `20·log10|v|`（DC 无纹波，
/// 电平可手算到小数点后两位）。⭐ `#305` 的阶跃素材也走这里，⛔ 不复制 WAV 头。
fn stereo_asset(sample: impl Fn(usize) -> f32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(ASSET_FRAMES * 2 * 4);
    for index in 0..ASSET_FRAMES {
        let value = sample(index);
        payload.extend_from_slice(&value.to_le_bytes());
        payload.extend_from_slice(&(-value).to_le_bytes());
    }
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
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&2u16.to_le_bytes()); // 立体声
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&(48_000u32 * 8).to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(payload.len()).expect("小文件").to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// `at` 帧处一个电平阶跃的立体声素材。
///
/// ⚠ `after` **必须非零**：`release_s` 只在"目标增益回到 1"时被选中，而输入为 0
/// 时输出恒为 0 ⇒ 下阶跃到静音**读不出**释放时间（`#305` 的登记项之一）。
fn step_asset(before: f32, after: f32, at: usize) -> Vec<u8> {
    stereo_asset(|index| if index < at { before } else { after })
}

/// 夹具旋钮。
struct Tune<'a> {
    /// 轨声相（`-1..=1`）。
    pan: f32,
    /// 设备参数（空 = 没有设备）。
    params: &'a [(&'a str, f32)],
    /// 是否放一段 MIDI（`NoteToneSource` 的方波）。
    midi: bool,
    /// 是否放一段音频片段。
    audio: bool,
}

impl Default for Tune<'_> {
    fn default() -> Self {
        Self {
            pan: 0.0,
            params: &[],
            midi: true,
            audio: true,
        }
    }
}

fn project(tune: &Tune<'_>, bytes: &[u8]) -> YebanProjectV1 {
    let master = id(1);
    let lead = id(2);
    let midi_clip = id(10);
    let midi_placement = id(20);
    let audio_clip = id(11);
    let audio_placement = id(21);
    let edge = id(40);
    let device = id(101);
    let asset = AssetHash::of_bytes(bytes);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "compressor fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0303,
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
        kind: TrackKind::Midi,
        volume_db: 0.0,
        pan: tune.pan,
        devices: if tune.params.is_empty() {
            Vec::new()
        } else {
            vec![DeviceDefinition {
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
            }]
        },
        ..TrackV3::default()
    };
    if tune.midi {
        track.clips.insert(
            midi_placement,
            ClipPlacement {
                id: midi_placement,
                clip_id: midi_clip,
                start_tick: 0,
                duration_ticks: ONE_SECOND_TICKS,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
        project.clip_pool.insert(
            midi_clip,
            ClipPoolEntry {
                id: midi_clip,
                name: "Lead".to_owned(),
                content: ClipContent::Midi {
                    notes: BTreeMap::from([(
                        id(30),
                        MidiNote {
                            velocity: 100,
                            ..MidiNote::new(id(30), 0, 69, ONE_SECOND_TICKS)
                        },
                    )]),
                },
            },
        );
    }
    if tune.audio {
        track.clips.insert(
            audio_placement,
            ClipPlacement {
                id: audio_placement,
                clip_id: audio_clip,
                start_tick: 0,
                duration_ticks: ONE_SECOND_TICKS,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
        project.clip_pool.insert(
            audio_clip,
            ClipPoolEntry {
                id: audio_clip,
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
        // 最小音源（方波）：没有引擎快照，跨机可复现（⛔ 不依赖 `cpal`/引擎音色）。
        use_engine_source: false,
        ..WavExportOptions::default()
    }
}

fn render(project: &YebanProjectV1, bytes: &[u8]) -> RenderProjectOutput {
    let asset = AssetHash::of_bytes(bytes);
    render_project_f32_with_assets_under(project, options(), None, &[(asset, bytes.to_vec())], None)
        .expect("浮点渲染必须成功")
}

/// 整段母带的能量（两声道样本平方和）。
fn master_energy(samples: &[f32]) -> f64 {
    samples
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum()
}

/// 轨声相的 `(左, 右)` —— 与 `audio_clip::track_pan_gains` **同一条**曲线与中心规则。
fn pan_gains_of(project: &YebanProjectV1) -> (f32, f32) {
    let pan = project
        .tracks
        .get(&id(2))
        .map(|track| track.pan)
        .expect("Lead");
    centered_pan_gains(pan, |pan| {
        pan_gains(pan, PanLaw::from_model(project.audio_config.pan_law))
    })
}

/// 压缩夹具的设备参数（硬膝 + 短弹道 ⇒ 稳态 GR 可手算）。
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

/// 判据 ①（复现 / 修后）＋ 判据 ②（正对照）：一次给两组读数。
///
/// * 无 `threshold_db`（增益 `-6 dB`，`pan = 0`）⇒ 摘要必须等于 `#303` 之前的常量；
/// * 有 `threshold_db` ⇒ 摘要必须**不同**、能量比 < 0.5。
#[test]
fn the_export_really_compresses_and_the_positive_control_is_untouched() {
    let bytes = constant_asset(0.5);

    // ---- 正对照：没有压缩参数 ----
    let control_project = project(
        &Tune {
            params: &[("input_gain_db", -6.0)],
            midi: false,
            audio: true,
            ..Tune::default()
        },
        &bytes,
    );
    let control = render(&control_project, &bytes);
    println!(
        "### #303 正对照（无 threshold_db，pan=0）: digest={} energy={:.9e}",
        control.digest.clone(),
        master_energy(&control.samples)
    );
    assert_eq!(
        control.digest.clone(),
        NO_COMPRESSOR_MASTER_DIGEST,
        "没有压缩参数的工程必须逐字节等于 #303 之前的母带"
    );

    // ---- 修后：同一份工程加上压缩 ----
    let dry_project = project(
        &Tune {
            midi: false,
            audio: true,
            ..Tune::default()
        },
        &bytes,
    );
    let wet_project = project(
        &Tune {
            params: compressor_params(),
            midi: false,
            audio: true,
            ..Tune::default()
        },
        &bytes,
    );
    let dry = render(&dry_project, &bytes);
    let wet = render(&wet_project, &bytes);
    let ratio = master_energy(&wet.samples) / master_energy(&dry.samples);
    println!(
        "### #303 压缩（pan=0）: dry_digest={} wet_digest={} E_dry={:.9e} E_wet={:.9e} ratio={ratio:.9}",
        dry.digest.clone(),
        wet.digest.clone(),
        master_energy(&dry.samples),
        master_energy(&wet.samples)
    );
    assert_ne!(
        wet.digest, dry.digest,
        "写了 threshold_db ⇒ 导出必须真的被压（修前两者逐字节相同）"
    );
    assert!(
        ratio < 0.5,
        "0.5 的常量素材 + 阈值 −30 dB ⇒ 必须显著被压，实得能量比 {ratio}"
    );
}

/// 判据 ③（裁决一）：**压缩量不受声相影响** ⇒ 检波器在声相之前。
#[test]
fn the_compression_amount_does_not_depend_on_pan() {
    let bytes = constant_asset(0.5);
    let mut ratios = Vec::new();
    for pan in [0.0f32, 0.8, -0.8] {
        let dry = render(
            &project(
                &Tune {
                    pan,
                    midi: false,
                    audio: true,
                    ..Tune::default()
                },
                &bytes,
            ),
            &bytes,
        );
        let wet = render(
            &project(
                &Tune {
                    pan,
                    params: compressor_params(),
                    midi: false,
                    audio: true,
                },
                &bytes,
            ),
            &bytes,
        );
        let ratio = master_energy(&wet.samples) / master_energy(&dry.samples);
        println!(
            "### #303 app pan={pan:+.3}: E_dry={:.9e} E_wet={:.9e} ratio={ratio:.9}",
            master_energy(&dry.samples),
            master_energy(&wet.samples)
        );
        assert!(ratio < 0.5, "pan={pan}: 压缩必须真的压，实得 {ratio}");
        ratios.push((pan, ratio));
    }
    let base = ratios[0].1;
    for (pan, ratio) in &ratios[1..] {
        let relative = (ratio - base).abs() / base;
        assert!(
            relative < 1.0e-5,
            "pan={pan} 的压缩量 {ratio} 与 pan=0 的 {base} 不同（相对差 {relative:.3e}）—— \
             检波器不在声相之前"
        );
    }
    println!("### #303 app 三档能量比: {ratios:?}（必须相等 ⇒ 检波在声相前）");
}

/// f32 的**全序**整数键（`−0.0 < +0.0` 不区分；只用于数 ulp 距离）。
fn order_key(value: f32) -> u64 {
    let bits = value.to_bits();
    if bits & 0x8000_0000 != 0 {
        0x8000_0000u64 - u64::from(bits & 0x7fff_ffff)
    } else {
        0x8000_0000u64 + u64::from(bits)
    }
}

/// 判据 ④（边界 ②）：MIDI ＋ 音频片段的轨，把声相从"逐源乘"改成"整条通道乘"的
/// 差**只有浮点结合顺序**，给出实测 **ulp 上界**。
///
/// 量法（同一份夹具、`pan = +0.8`、**无设备**）：
/// * `O` = 现在的逐源路径（源侧声相）：`Σ(x_k · p)`；
/// * `S` = 同一工程 `pan = 0` 的母带：`Σ x_k`；
/// * 整通道路径 = `S · p`（`#303` 对**会压缩**的轨采用的结合顺序）。
///
/// `|O − S·p|` 的最大 ulp 距离就是"搬家"的算术代价。实测上界钉在断言里：
/// ⛔ 不是"有差"，而是一个**数**。
#[test]
fn the_pan_association_difference_on_a_mixed_track_is_bounded_in_ulps() {
    let bytes = constant_asset(0.5);
    let panned = render(
        &project(
            &Tune {
                pan: 0.8,
                midi: true,
                audio: true,
                ..Tune::default()
            },
            &bytes,
        ),
        &bytes,
    );
    let centered = render(
        &project(
            &Tune {
                pan: 0.0,
                midi: true,
                audio: true,
                ..Tune::default()
            },
            &bytes,
        ),
        &bytes,
    );
    let panned_project = project(
        &Tune {
            pan: 0.8,
            midi: true,
            audio: true,
            ..Tune::default()
        },
        &bytes,
    );
    let (pan_left, pan_right) = pan_gains_of(&panned_project);
    println!("### #303 声相增益: pan=+0.8 ⇒ L={pan_left:.9} R={pan_right:.9}");

    let mut max_ulp = 0u64;
    let mut differing = 0u64;
    let mut total = 0u64;
    for (index, pair) in panned.samples.as_chunks::<2>().0.iter().enumerate() {
        let (sum_left, sum_right) = (centered.samples[index * 2], centered.samples[index * 2 + 1]);
        for (reference, sum, gain) in [
            (pair[0], sum_left, pan_left),
            (pair[1], sum_right, pan_right),
        ] {
            let whole_channel = sum * gain;
            let ulp = order_key(reference).abs_diff(order_key(whole_channel));
            max_ulp = max_ulp.max(ulp);
            if ulp != 0 {
                differing += 1;
            }
            total += 1;
        }
    }
    println!("### #303 结合顺序差: max_ulp={max_ulp} 有差样本={differing}/{total}");
    // 实测上界：单次乘法与两次乘法的结合顺序差最多 1 ulp（逐源乘是两次乘 + 一次加，
    // 整通道乘是一次加 + 一次乘）。把上界钉在 4 ulp 给跨平台的 FMA 差异留余量。
    assert!(
        max_ulp <= 4,
        "整通道声相与逐源声相的差必须 ≤ 4 ulp，实测最大 {max_ulp} ulp"
    );
    assert!(
        max_ulp > 0,
        "两端都是有限随机样本，不可能逐位相同（这会让判据失去意义）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#305`：压缩 8 字段里除 `threshold_db` 外的 **7 个**，各自一条端到端音频判据
// （app 导出：`render_core` 浮点母带 digest ＋ 24-bit 文件 sha256 ＋ 分段读数）。
//
// `#304` 只给 `threshold_db` 做了端到端；其余 7 个字段在 `yeban-render` 单测里覆盖了
// `from_name` / 静态值 / 渐变 / 恒等 / 强制实例化，但**没有各自一条音频判据**。
// 本片按同一条形状补齐：每个字段一条“静态基线 vs 另一个静态值”的导出判据，
// 读数按其物理方向严格单调，并给 `base_sha` / `variant_sha` / `base_digest` /
// `variant_digest` 与窗口 GR（`20·log10(rms_variant / rms_dry)`）。
//
// ⚠ **四条“受素材约束”的显式登记**（⛔ 不是省略，⛔ 也不是 `#[ignore]` —— 每条都有
// 素材能让它可观测）：
// 1. `detector_s` 只随**阶跃**变：对称平均的稳态 `E[ms]` 与 τ 无关，常量素材上
//    读到的是同一个电平（这正是 `#304` 说“1 s 常量素材不可靠”的代码原因）。
// 2. `release_s` 在**常量**素材上不可观测（target 与 gain 相等，不走 release 分支）；
//    且**下阶跃到 0** 时输出恒为 0 ⇒ 必须下阶跃到“小但非零”的电平。
// 3. `knee_db` 在**高于 `threshold + knee/2`** 时不可观测：膝上分支
//    `gain = over/R − over` **不含 W** ⇒ 必须用近阈值电平。
// 4. `compressor_enabled` 是开关：“关”本身按定义听不出 ⇒ 只做**开/关之差**
//    （关＝与干声逐字节相同；开＝GR < 0），⛔ 不做三点单调。
//
// 素材一律 1 s / 48 kHz / L=+v、R=−v。DC 的检波电平是精确的 `20·log10|v|`
// （无正弦纹波、跨平台不依赖 `sin`），所以上表的 GR 可以手算对账。
// ---------------------------------------------------------------------------

/// 真的落盘并回读文件字节的 SHA-256；临时文件用完即删（并行测试用 pid + 序号隔离）。
static EXPORT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn export_sha(project: &YebanProjectV1, bytes: &[u8], tag: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "yeban-305-{}-{}-{tag}.wav",
        std::process::id(),
        EXPORT_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
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
    let _ = std::fs::remove_file(&path);
    AssetHash::of_bytes(&file_bytes).as_str().to_owned()
}

/// 交错立体声缓冲在 `[start, end)` 帧上的 RMS（两声道合并）。
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

/// 在**已建好**的工程上插一条 `DeviceParam` 泳道，指向设备 `params[0]`。
///
/// ⚠ 本判据的 7 个 case 都把被测字段放在 `params[0]`；这样恒等/关闭两条正对照
/// 与方向判据共用同一台设备，⛔ 不需要复制工程构造函数。
fn add_device_lane(project: &mut YebanProjectV1, points: &[(u64, f32)], read_enabled: bool) {
    let track_id = id(2);
    let target = AutomationTarget::DeviceParam {
        track_id,
        slot_index: 0,
        param_index: 0,
    };
    let mut lane_points = BTreeMap::new();
    for (index, (tick, value)) in points.iter().enumerate() {
        let point = AutomationPoint {
            id: id(400 + index as u32),
            tick: *tick,
            value: *value,
            curve: CurveType::Linear,
        };
        lane_points.insert(point.id, point);
    }
    project
        .tracks
        .get_mut(&track_id)
        .expect("Lead")
        .automation_lanes
        .insert(
            target,
            AutomationLane {
                target,
                points: lane_points,
                read_enabled,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
    project.validate().expect("带泳道的夹具必须合法");
}

/// 阶跃发生的位置（0.2 s @ 48 kHz）。
const STEP_AT: usize = 9_600;

#[derive(Clone, Copy)]
enum Material {
    /// 1 s 常量。
    Steady { level: f32 },
    /// 0 → `after`（`at = STEP_AT`）。
    StepUp { after: f32 },
    /// `before` → `after`（`after` 必须非零）。
    StepDown { before: f32, after: f32 },
}

impl Material {
    fn bytes(self) -> Vec<u8> {
        match self {
            Self::Steady { level } => constant_asset(level),
            Self::StepUp { after } => step_asset(0.0, after, STEP_AT),
            Self::StepDown { before, after } => step_asset(before, after, STEP_AT),
        }
    }
}

/// 一个字段一条端到端用例。`params[0]` **就是**被测字段（泳道也指向 `param_index = 0`）。
struct CompCase {
    name: &'static str,
    params: &'static [(&'static str, f32)],
    from: f32,
    mid: f32,
    to: f32,
    material: Material,
    /// 读数窗口（帧，48 kHz）。
    window: (usize, usize),
    /// 字段值从 `from` 增到 `to` 时，窗口 RMS 是否上升。
    rising: bool,
}

/// ⭐ 7 个字段 / 7 条判据。数值全部是**手算**（DC 电平精确）：
/// - `ratio`：阈值 −24、电平 −6.02 ⇒ over 17.98 ⇒ GR 8.99/13.48/15.73 dB；
/// - `knee_db`：阈值 −6.0 ⇒ over +0.02 ⇒ GR 0.015/1.13/2.26 dB（膝内）；
/// - `detector_s`：上阶跃后 2 ms 窗，早期 GR 约 13.4/9.9/2.9 dB；
/// - `attack_s`：上阶跃后 20 ms 窗，窗末增益约 0.21/0.32/0.86；
/// - `release_s`：下阶跃到 0.05 后 20 ms 窗，窗末增益约 0.47/0.29/0.23；
/// - `makeup_db`：0.05 在阈值 −6 以下 ⇒ 压缩恒 0，+6/+12 dB 直接乘到输出。
const COMP_CASES: &[CompCase] = &[
    CompCase {
        name: "compressor_enabled",
        params: &[("compressor_enabled", 0.0), ("threshold_db", -12.0)],
        from: 0.0,
        mid: 1.0,
        to: 1.0, // 开关：第三个点与第二个点同值（⛔ 不做三点单调）
        material: Material::Steady { level: 0.5 },
        window: (4_800, 43_200),
        rising: false,
    },
    CompCase {
        name: "ratio",
        params: &[
            ("ratio", 2.0),
            ("threshold_db", -24.0),
            ("knee_db", 0.0),
            ("detector_s", 0.000_5),
            ("attack_s", 0.000_1),
            ("release_s", 0.05),
            ("makeup_db", 0.0),
        ],
        from: 2.0,
        mid: 4.0,
        to: 8.0,
        material: Material::Steady { level: 0.5 },
        window: (4_800, 43_200),
        rising: false,
    },
    CompCase {
        name: "knee_db",
        params: &[
            ("knee_db", 0.0),
            ("threshold_db", -6.0),
            ("ratio", 4.0),
            ("detector_s", 0.000_5),
            ("attack_s", 0.000_1),
            ("release_s", 0.05),
            ("makeup_db", 0.0),
        ],
        from: 0.0,
        mid: 12.0,
        to: 24.0,
        material: Material::Steady { level: 0.5 },
        window: (4_800, 43_200),
        rising: false,
    },
    CompCase {
        name: "detector_s",
        params: &[
            ("detector_s", 0.000_5),
            ("threshold_db", -24.0),
            ("ratio", 4.0),
            ("knee_db", 0.0),
            ("attack_s", 0.000_1),
            ("release_s", 0.05),
            ("makeup_db", 0.0),
        ],
        from: 0.000_5,
        mid: 0.005,
        to: 0.05,
        material: Material::StepUp { after: 0.5 },
        window: (STEP_AT, STEP_AT + 96), // 2 ms
        rising: true,
    },
    CompCase {
        name: "attack_s",
        params: &[
            ("attack_s", 0.000_1),
            ("threshold_db", -24.0),
            ("ratio", 4.0),
            ("knee_db", 0.0),
            ("detector_s", 0.000_5),
            ("release_s", 0.05),
            ("makeup_db", 0.0),
        ],
        from: 0.000_1,
        mid: 0.01,
        to: 0.1,
        material: Material::StepUp { after: 0.5 },
        window: (STEP_AT, STEP_AT + 960), // 20 ms
        rising: true,
    },
    CompCase {
        name: "release_s",
        params: &[
            ("release_s", 0.05),
            ("threshold_db", -24.0),
            ("ratio", 4.0),
            ("knee_db", 0.0),
            ("detector_s", 0.000_5),
            ("attack_s", 0.000_1),
            ("makeup_db", 0.0),
        ],
        from: 0.05,
        mid: 0.2,
        to: 1.0,
        material: Material::StepDown {
            before: 0.5,
            after: 0.05, // ⛔ 不是 0：输出 = 0 × gain 恒为 0，release 读不出
        },
        window: (STEP_AT, STEP_AT + 960), // 20 ms
        rising: false,
    },
    CompCase {
        name: "makeup_db",
        params: &[
            ("makeup_db", 0.0),
            ("threshold_db", -6.0),
            ("ratio", 4.0),
            ("knee_db", 0.0),
            ("detector_s", 0.000_5),
            ("attack_s", 0.000_1),
            ("release_s", 0.05),
        ],
        from: 0.0,
        mid: 6.0,
        to: 12.0,
        // 电平 −26.02 dBFS 在阈值 −6 以下 ⇒ 压缩增益恒 0，读数只随 makeup 走。
        material: Material::Steady { level: 0.05 },
        window: (4_800, 43_200),
        rising: true,
    },
];

/// 把 case 的 `params[0]` 换成 `value`（其余固定参数一字不改）。
fn params_with(case: &CompCase, value: f32) -> Vec<(&'static str, f32)> {
    let mut params = case.params.to_vec();
    params[0].1 = value;
    params
}

fn project_with(case: &CompCase, value: f32, bytes: &[u8]) -> YebanProjectV1 {
    let params = params_with(case, value);
    project(
        &Tune {
            params: &params,
            midi: false,
            audio: true,
            ..Tune::default()
        },
        bytes,
    )
}

/// ⭐ `#305` 判据 1：7 个字段各一条端到端音频判据（静态基线 vs 另一个静态值）。
///
/// 每条断言：
/// 1. 24-bit 文件 `sha256` **与**浮点母带 `digest` 都变；
/// 2. 窗口 RMS（以及相对干声的窗口 GR）按该字段的**物理方向**严格单调
///    （开关只有“开/关”两档，第二个点与第一个点同值，⛔ 三点单调）。
#[test]
fn each_compressor_field_moves_its_export_reading_in_the_documented_direction() {
    println!(
        "### #305 cases ({}) = {:?}",
        COMP_CASES.len(),
        COMP_CASES.iter().map(|c| c.name).collect::<Vec<_>>()
    );
    for case in COMP_CASES {
        let bytes = case.material.bytes();
        let (start, end) = case.window;

        // 干声：同一段素材、根本没有设备。窗口 GR 的分母。
        let dry = render(
            &project(
                &Tune {
                    params: &[],
                    midi: false,
                    audio: true,
                    ..Tune::default()
                },
                &bytes,
            ),
            &bytes,
        );
        let dry_read = window_rms(&dry.samples, start, end);
        assert!(dry_read > 0.0, "{}: 干声窗口必须非零", case.name);

        let base = project_with(case, case.from, &bytes);
        let base_out = render(&base, &bytes);
        let base_sha = export_sha(&base, &bytes, "base");
        let base_read = window_rms(&base_out.samples, start, end);
        let base_gain = 20.0 * (base_read / dry_read).log10();
        println!(
            "### #305 {} base: value={} sha={} digest={} rms={:.9e} gain={base_gain:+.3} dB",
            case.name, case.from, base_sha, base_out.digest, base_read
        );

        // 开关只取一个非从值（mid == to）；其余字段取 mid、to 两个点。
        let variants: Vec<f32> = if (case.mid - case.to).abs() < f32::EPSILON {
            vec![case.mid]
        } else {
            vec![case.mid, case.to]
        };
        let mut previous = base_read;
        for value in variants {
            let variant = project_with(case, value, &bytes);
            let out = render(&variant, &bytes);
            let sha = export_sha(&variant, &bytes, "variant");
            let read = window_rms(&out.samples, start, end);
            let gain = 20.0 * (read / dry_read).log10();
            println!(
                "### #305 {} variant: value={} sha={} digest={} rms={:.9e} gain={gain:+.3} dB",
                case.name, value, sha, out.digest, read
            );
            assert_ne!(
                out.digest, base_out.digest,
                "{}={value}: 浮点母带必须真的不同（否则字段没有落地）",
                case.name
            );
            assert_ne!(
                sha, base_sha,
                "{}={value}: 24-bit 文件字节必须真的不同（⛔ 只改了一个读数）",
                case.name
            );
            if case.rising {
                assert!(
                    read > previous,
                    "{}={value}: 窗口 RMS 必须严格上升（上一个 {previous:.9e}，本个 {read:.9e}）",
                    case.name
                );
            } else {
                assert!(
                    read < previous,
                    "{}={value}: 窗口 RMS 必须严格下降（上一个 {previous:.9e}，本个 {read:.9e}）",
                    case.name
                );
            }
            previous = read;
        }
    }
}

/// ⭐ `#305` 判据 2：**恒等正对照** —— 7 个字段各一条“泳道处处等于静态值”
/// ⇒ `sha256` 与浮点 `digest` **逐字节等于**无泳道；再加一条 `readEnabled:false`
/// 的**非恒等**泳道 ⇒ 同样逐字节等于无泳道（⛔ 不是“乘 1.0 近似”）。
#[test]
fn a_flat_lane_for_every_compressor_field_is_byte_identical_to_no_lane() {
    for case in COMP_CASES {
        let bytes = case.material.bytes();

        // 开关的恒等对照取“开”（静态 1）；其余字段取静态 `from`。
        let flat_value = if case.name == "compressor_enabled" {
            1.0
        } else {
            case.from
        };
        let static_flat = project_with(case, flat_value, &bytes);
        let static_out = render(&static_flat, &bytes);
        let static_sha = export_sha(&static_flat, &bytes, "static-flat");
        let mut flat = project_with(case, flat_value, &bytes);
        add_device_lane(
            &mut flat,
            &[
                (0, flat_value),
                (960, flat_value),
                (ONE_SECOND_TICKS, flat_value),
            ],
            true,
        );
        let flat_out = render(&flat, &bytes);
        let flat_sha = export_sha(&flat, &bytes, "flat-lane");
        println!(
            "### #305 {} flat: value={flat_value} static_sha={static_sha} lane_sha={flat_sha} \
             static_digest={} lane_digest={}",
            case.name, static_out.digest, flat_out.digest
        );
        assert_eq!(
            flat_out.digest, static_out.digest,
            "{}: 恒等泳道必须逐字节等于无泳道（浮点母带）",
            case.name
        );
        assert_eq!(
            flat_sha, static_sha,
            "{}: 恒等泳道必须逐字节等于无泳道（文件 sha256）",
            case.name
        );

        // `readEnabled:false`：非恒等泳道（from → to）也必须被整段忽略。
        let static_from = project_with(case, case.from, &bytes);
        let static_from_out = render(&static_from, &bytes);
        let static_from_sha = export_sha(&static_from, &bytes, "static-from");
        let mut disabled = project_with(case, case.from, &bytes);
        add_device_lane(
            &mut disabled,
            &[(0, case.from), (960, case.to), (ONE_SECOND_TICKS, case.to)],
            false,
        );
        let disabled_out = render(&disabled, &bytes);
        let disabled_sha = export_sha(&disabled, &bytes, "disabled-lane");
        println!(
            "### #305 {} readDisabled: static_sha={static_from_sha} lane_sha={disabled_sha} \
             static_digest={} lane_digest={}",
            case.name, static_from_out.digest, disabled_out.digest
        );
        assert_eq!(
            disabled_out.digest, static_from_out.digest,
            "{}: readEnabled:false 必须逐字节等于无泳道（浮点母带）",
            case.name
        );
        assert_eq!(
            disabled_sha, static_from_sha,
            "{}: readEnabled:false 必须逐字节等于无泳道（文件 sha256）",
            case.name
        );
    }
}

/// ⭐ `#305` 判据 3：这 7 个 case 就是“压缩 8 字段减去 `threshold_db`”。
///
/// ⛔ 不手抄第二张拼写表：名字从 [`StripField`] 的 8 个**变体**现读
/// `canonical_name()`，再与 `COMP_CASES` 互钉（任一边漏/多一个字段都会红）。
#[test]
fn the_seven_cases_are_the_compressor_fields_minus_threshold() {
    use yeban_render::inserts::StripField;

    const EIGHT: [StripField; 8] = [
        StripField::CompressorEnabled,
        StripField::ThresholdDb,
        StripField::Ratio,
        StripField::KneeDb,
        StripField::DetectorS,
        StripField::AttackS,
        StripField::ReleaseS,
        StripField::MakeupDb,
    ];
    let covered: BTreeSet<&str> = COMP_CASES.iter().map(|case| case.name).collect();
    let mut expected = BTreeSet::new();
    for field in EIGHT {
        if field != StripField::ThresholdDb {
            expected.insert(field.canonical_name());
        }
    }
    println!("### #305 covered={covered:?}");
    println!("### #305 expected={expected:?}");
    assert_eq!(EIGHT.len(), 8, "压缩字段是 8 个");
    assert_eq!(StripField::ALL.len(), 22, "通道条共 22 个字段");
    assert_eq!(COMP_CASES.len(), 7, "除 threshold_db 外是 7 个");
    assert_eq!(covered, expected, "COMP_CASES 必须逐字覆盖这 7 个字段");
    assert!(
        !covered.contains("threshold_db"),
        "threshold_db 已由 #304 覆盖"
    );
    for name in &covered {
        assert!(
            StripField::from_name(name).is_some(),
            "{name} 必须是名字表认下的规范名"
        );
    }
}
