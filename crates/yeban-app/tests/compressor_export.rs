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

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_model::{
    AssetHash, AssetMetadata, ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition,
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
    let mut payload = Vec::with_capacity(ASSET_FRAMES * 2 * 4);
    for _ in 0..ASSET_FRAMES {
        payload.extend_from_slice(&level.to_le_bytes());
        payload.extend_from_slice(&(-level).to_le_bytes());
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
