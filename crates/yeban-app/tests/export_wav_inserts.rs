//! 任务单 `#244`：`app` 的离线导出（`--export-wav` / UI `export-wav` / `record_wav` 共用
//! 的渲染核心）**真的跑插入链**（EQ / Freeverb / 卷积）。
//!
//! # 缺口（修前）
//!
//! `crates/yeban-app/src/render_core.rs` 走的是 `RenderPlan::compile(...)`
//! ⇒ 内部两条空表：`latencies = {}` 与 `inserts = {}`（[`RenderPlan::compile`] 的文档：
//! "等价于传一张空表"）。于是 `TrackV3.devices` 里的 EQ / 混响 / 卷积**一个都不进导出**，
//! 而同一仓里的 MCP（`yeban-mcp/src/domain/render.rs:2020`）已经在用
//! `compile_with_latencies_and_inserts` ⇒ "插入链能不能在离线渲染里跑"的答案是**能**。
//!
//! # 判据表（本文件）
//!
//! | 判据 | 断言 | 证什么 / ⛔ 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 有 EQ vs 无设备 | 整文件 `sha256` **不同** ＋ 母带峰值比 > 1.5 | 证 EQ 真的改了导出字节；⛔ 不证"听起来对" |
//! | ② 正对照：无设备 | 同工程连导两次 `sha256` 相同（同机自比） | 证渲染确定性；修复前的读数由报告逐字给出 |
//! | ③ bypass | `setDeviceBypassed(true)` ≡ 没有那台设备（`sha256` 相同） | 证旁通真的等于"没有"（不是"静默地仍处理"） |
//! | ④ Freeverb | `sha256` 不同 ＋ 音符结束后的尾巴 RMS > 无设备档 × 10 | 证混响进了导出 |
//! | ⑤ 卷积 | 同上（合成 IR 的 `conv_*` 设备） | 证卷积进了导出 |
//! | ⑥ 延迟 | 单链：起点偏移 **0**；两条并行支路（一条声明 96 帧）：短支路被补 **96** 帧 | 证 PDC 只做分支对齐、不做"整体前滚" ⇒ 导出**不需要**再补偿 |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 没有人耳听过：本文件只证"EQ / 混响 / 卷积改了导出音频"（字节 ＋ 峰值 ＋ 尾巴能量），
//!   不证"听起来对"；
//! - ⛔ 不钉 WAV 的绝对 `sha256` 作为跨机常量（机器相关，台账第 313 条）：
//!   同一台机器上"有设备 vs 无设备"的**相对**读数才是证据；
//! - ⛔ 不证实时监听的插入链：那由 `yeban-engine` 的判据负责；本文件只证**导出侧**。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_app::bridge::{self, BuiltinEffect};
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets_under};
use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 导出 1 秒：够量峰值与"音符结束后"的尾巴（音符 0.25 秒）。
const FRAMES: u64 = 48_000;
/// 音符／摆放长度（tick）：240 tick = 0.5 拍 = 0.25 秒 @ 120 BPM / 480 PPQ。
const NOTE_TICKS: u64 = 240;

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P1{tail}")).expect("夹具 ULID 必须合法")
}

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-244-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 一条带一个 MIDI 音符的轨 ＋ 它引用的 `ClipPoolEntry`（音符在 `clip` 里，摆放在 0）。
fn track_with_note(
    track_id: EntityId,
    name: &str,
    pitch: u8,
    clip_id: EntityId,
    placement_id: EntityId,
    note_id: EntityId,
) -> (TrackV3, ClipPoolEntry) {
    let mut track = TrackV3 {
        id: track_id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        volume_db: -6.0,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(
        note_id,
        MidiNote {
            velocity: 100,
            ..MidiNote::new(note_id, 0, pitch, NOTE_TICKS)
        },
    );
    let entry = ClipPoolEntry {
        id: clip_id,
        name: name.to_owned(),
        content: ClipContent::Midi { notes },
    };
    (track, entry)
}

/// 把一条轨 ＋ 一条 `TrackToBus` 边挂到 `default_project` 的主总线上。
fn attach(project: &mut YebanProjectV1, track: TrackV3, entry: ClipPoolEntry, edge_id: EntityId) {
    let master = project.master_bus_track_id;
    let track_id = track.id;
    project.clip_pool.insert(entry.id, entry);
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge_id,
        RoutingEdge {
            id: edge_id,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
}

/// 夹具工程：`default_project` 的主总线 ＋ 一条 MIDI 轨（pitch 60）＋ `TrackToBus` 边。
///
/// `devices` 原样写进那条轨的设备链（＝ 产品里"用户在设备架装的东西"）。
fn project_with_devices(devices: Vec<DeviceDefinition>) -> YebanProjectV1 {
    let mut project = yeban_model::samples::default_project();
    let track_id = fixture_id("T1");
    let (mut track, entry) = track_with_note(
        track_id,
        "Insert",
        60,
        fixture_id("K1"),
        fixture_id("P1"),
        fixture_id("N1"),
    );
    track.devices = devices;
    attach(&mut project, track, entry, fixture_id("E1"));
    project.routing_graph.nodes.sort_unstable();
    project.validate().expect("夹具工程必须合法");
    project
}

/// 契约里的一台 EQ：低架 +18 dB @ 1 kHz（把 261 Hz 基频与低次谐波一起抬起来）。
fn eq_device(gain_db: f32) -> DeviceDefinition {
    let mut device = bridge::builtin_effect_device(BuiltinEffect::Eq).expect("契约里必须有 EQ");
    for param in &mut device.params {
        match param.name.as_str() {
            "eq_low_gain" | "eq_mid_gain" | "eq_high_gain" => param.value = gain_db,
            "eq_low_freq" => param.value = 1_000.0,
            _ => {}
        }
    }
    device
}

/// 契约里的一台 Freeverb：湿路拉满 ＋ 无预延迟（尾巴在音符结束后立刻可量）。
fn reverb_device() -> DeviceDefinition {
    let mut device =
        bridge::builtin_effect_device(BuiltinEffect::Reverb).expect("契约里必须有 Reverb");
    for param in &mut device.params {
        match param.name.as_str() {
            "reverb_wet" => param.value = 1.0,
            "reverb_predelay" => param.value = 0.0,
            "reverb_size" => param.value = 0.9,
            _ => {}
        }
    }
    device
}

/// 一台卷积混响（契约分组 `convolution`；IR 由 `conv_ir_decay_s` / `conv_ir_seed` 合成）。
fn convolution_device() -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Convolution".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![
            ParameterValue {
                name: "conv_wet".to_owned(),
                value: 1.0,
                unit: None,
            },
            ParameterValue {
                name: "conv_dry".to_owned(),
                value: 0.0,
                unit: None,
            },
            ParameterValue {
                name: "conv_ir_decay_s".to_owned(),
                value: 0.35,
                unit: Some("s".to_owned()),
            },
            ParameterValue {
                name: "conv_ir_seed".to_owned(),
                value: 1.0,
                unit: None,
            },
        ],
        latency_samples: 0,
    }
}

/// 一份完整读数：母带样本、峰值、母带 digest、WAV 文件 `sha256`。
struct Reading {
    samples: Vec<f32>,
    peak: f32,
    digest: String,
    sha256: String,
}

fn render(project: &YebanProjectV1, fixture: &Fixture, label: &str) -> Reading {
    let options = WavExportOptions {
        frames: FRAMES,
        ..WavExportOptions::default()
    };
    let output: RenderProjectOutput = render_project_f32_with_assets_under(
        project,
        options,
        None,
        &[],
        Some(fixture.dir.as_path()),
    )
    .expect("夹具工程必须能渲染");
    let peak = output
        .samples
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));
    let path = fixture.dir.join(format!("{label}.wav"));
    export_wav(project, &path, options).expect("导出 WAV");
    let sha256 = sha256_of(&path);
    println!(
        "[244/{label}] frames={} peak={peak:.9} digest={} sha256={sha256}",
        output.frames, output.digest
    );
    Reading {
        samples: output.samples,
        peak,
        digest: output.digest,
        sha256,
    }
}

fn sha256_of(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("读产物");
    AssetHash::of_bytes(&bytes).to_string()
}

/// 交错立体声在 `[start, end)` 帧上的 RMS（两声道一起算）。
fn rms(samples: &[f32], start: usize, end: usize) -> f64 {
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for frame in start..end {
        for channel in 0..2 {
            let value = f64::from(samples[frame * 2 + channel]);
            sum += value * value;
            count += 1;
        }
    }
    (sum / count as f64).sqrt()
}

/// 第一个 `|sample| > 1e-4` 的帧号（起点读数）。
fn onset(samples: &[f32]) -> usize {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .position(|frame| frame[0].abs() > 1e-4 || frame[1].abs() > 1e-4)
        .unwrap_or(samples.len() / 2)
}

// ===========================================================================
// ① 有 EQ vs 无设备：字节真的不同 ＋ 峰值可测抬升
// ===========================================================================

/// ⭐ 判据 ①：同一工程，只把一台 EQ 装进设备链 ⇒ 导出 `sha256` **不同**，峰值显著抬高。
///
/// 修前（`RenderPlan::compile`）这条**红**：设备链不进计划 ⇒ 两档逐字节相同。
#[test]
fn an_eq_in_the_device_chain_changes_the_exported_audio() {
    let fixture = Fixture::new("eq");
    let none = render(&project_with_devices(Vec::new()), &fixture, "none");
    let eq = render(&project_with_devices(vec![eq_device(18.0)]), &fixture, "eq");

    println!(
        "[244/eq-diff] none=({}, peak={:.9}) eq=({}, peak={:.9}) peak_ratio={:.6}",
        none.sha256,
        none.peak,
        eq.sha256,
        eq.peak,
        eq.peak / none.peak
    );
    assert!(none.peak > 0.0, "无设备档必须非静音（否则峰值比无意义）");
    assert_ne!(none.sha256, eq.sha256, "⭐ EQ 必须改变导出的每一字节");
    assert_ne!(none.digest, eq.digest, "母带 digest 也必须不同");
    let ratio = eq.peak / none.peak;
    assert!(
        ratio > 1.5,
        "⭐ +18 dB 低架必须把峰值显著抬高；实际 {ratio:.6}（修前 = 1.000000）"
    );
}

// ===========================================================================
// ② 正对照：无设备的工程连导两次逐字节相同
// ===========================================================================

/// ⭐ 判据 ②：**无设备**工程连导两次 ⇒ `sha256` 相同（同机自比）。
///
/// 本判据同时是"修前 / 修后字节不变"的读数来源：报告里给出修前与修后的同一个 `sha256`。
#[test]
fn a_no_device_project_exports_the_same_bytes_twice() {
    let fixture = Fixture::new("control");
    let first = render(&project_with_devices(Vec::new()), &fixture, "control-a");
    let second = render(&project_with_devices(Vec::new()), &fixture, "control-b");
    println!(
        "[244/control] first={} second={} equal={}",
        first.sha256,
        second.sha256,
        first.sha256 == second.sha256
    );
    assert_eq!(first.sha256, second.sha256, "无设备工程必须逐字节可复现");
    assert_eq!(first.digest, second.digest);
}

// ===========================================================================
// ③ bypass：等于没有那台设备
// ===========================================================================

/// ⭐ 判据 ③：`setDeviceBypassed(true)` ⇒ 与"没有那台设备"**逐字节相同**。
///
/// 量什么：整文件 `sha256`（无量纲摘要）；两档必须相等。
#[test]
fn a_bypassed_eq_is_byte_identical_to_having_no_device() {
    let fixture = Fixture::new("bypass");
    let none = render(&project_with_devices(Vec::new()), &fixture, "bypass-none");
    let mut bypassed = eq_device(18.0);
    bypassed.bypassed = true;
    let bypass = render(&project_with_devices(vec![bypassed]), &fixture, "bypass-eq");
    println!(
        "[244/bypass] none={} bypassed={} equal={}",
        none.sha256,
        bypass.sha256,
        none.sha256 == bypass.sha256
    );
    assert_eq!(
        none.sha256, bypass.sha256,
        "旁通的设备必须等于没有那台设备（⛔ 不是静默地仍然处理）"
    );
}

// ===========================================================================
// ④ Freeverb：尾巴能量
// ===========================================================================

/// ⭐ 判据 ④：Freeverb 进导出 ⇒ `sha256` 不同，且**音符结束后**（0.5 秒之后）的尾巴 RMS
/// 比无设备档大 10 倍以上。
#[test]
fn a_freeverb_in_the_device_chain_adds_a_measurable_tail() {
    let fixture = Fixture::new("reverb");
    let none = render(&project_with_devices(Vec::new()), &fixture, "reverb-none");
    let reverb = render(
        &project_with_devices(vec![reverb_device()]),
        &fixture,
        "reverb-wet",
    );
    let tail_start = 24_000; // 0.5 秒
    let tail_end = FRAMES as usize;
    let none_tail = rms(&none.samples, tail_start, tail_end);
    let reverb_tail = rms(&reverb.samples, tail_start, tail_end);
    println!(
        "[244/reverb] none={} reverb={} diff={} none_tail_rms={none_tail:.12} \
         reverb_tail_rms={reverb_tail:.12} ratio={:.3}",
        none.sha256,
        reverb.sha256,
        none.sha256 != reverb.sha256,
        if none_tail > 0.0 {
            reverb_tail / none_tail
        } else {
            f64::INFINITY
        }
    );
    assert_ne!(none.sha256, reverb.sha256, "⭐ 混响必须改变导出字节");
    assert!(
        reverb_tail > 1e-4,
        "混响档在音符结束后必须还有能量；实际 {reverb_tail:.12}"
    );
    assert!(
        reverb_tail > none_tail * 10.0,
        "混响尾巴必须比无设备档大 10 倍以上；实际 {reverb_tail:.12} vs {none_tail:.12}"
    );
}

// ===========================================================================
// ⑤ 卷积：尾巴能量
// ===========================================================================

/// ⭐ 判据 ⑤：卷积混响（合成 IR）进导出 ⇒ `sha256` 不同 ＋ 尾巴 RMS 可测。
#[test]
fn a_convolution_in_the_device_chain_adds_a_measurable_tail() {
    let fixture = Fixture::new("conv");
    let none = render(&project_with_devices(Vec::new()), &fixture, "conv-none");
    let conv = render(
        &project_with_devices(vec![convolution_device()]),
        &fixture,
        "conv-wet",
    );
    // 合成 IR 的长度是 `采样率 ÷ 10` = **0.1 秒**（`inserts.rs` 的
    // `CONV_IR_SAMPLE_RATE_DIVISOR`，与引擎同值）⇒ 干声结束后最多再响 0.1 秒。
    // 本机读数：干声最后一声在第 9540 帧、卷积档第 14338 帧 ⇒ 尾巴窗口取
    // **10000…14000 帧**（0.208…0.292 秒），整段落在 IR 的真实支撑区内。
    let tail_start = 10_000;
    let tail_end = 14_000;
    let none_tail = rms(&none.samples, tail_start, tail_end);
    let conv_tail = rms(&conv.samples, tail_start, tail_end);
    // 读数：最后一声的帧号（尾音的真实支撑区）—— 用于核对"窗口是否落在 IR 支撑区内"。
    let last_sound = |samples: &[f32]| -> usize {
        samples
            .as_chunks::<2>()
            .0
            .iter()
            .rposition(|frame| frame[0].abs() > 1e-6 || frame[1].abs() > 1e-6)
            .unwrap_or(0)
    };
    println!(
        "[244/conv] none={} conv={} diff={} none_tail_rms={none_tail:.12} \
         conv_tail_rms={conv_tail:.12} ratio={:.3} none_last_sound={} conv_last_sound={}",
        none.sha256,
        conv.sha256,
        none.sha256 != conv.sha256,
        if none_tail > 0.0 {
            conv_tail / none_tail
        } else {
            f64::INFINITY
        },
        last_sound(&none.samples),
        last_sound(&conv.samples)
    );
    assert_ne!(none.sha256, conv.sha256, "⭐ 卷积必须改变导出字节");
    assert!(
        conv_tail > 1e-5,
        "卷积档在干声结束后必须还有能量；实际 {conv_tail:.12}"
    );
    assert!(
        conv_tail > none_tail * 10.0,
        "卷积尾巴必须比无设备档大 10 倍以上；实际 {conv_tail:.12} vs {none_tail:.12}"
    );
}

// ===========================================================================
// ⑥ 延迟：只做分支对齐，不做整体前滚
// ===========================================================================

/// ⭐ 判据 ⑥：延迟读数 ＋ 判"要不要补偿"。
///
/// 形状：master 上挂两条并行支路 —— `A`（`T1`）的链里有一台**声明 96 帧延迟**的 EQ，
/// `B`（`T2`）没有设备。`A` 被静音（`mute = true`，源增益 0 ⇒ 导出里只剩 `B`），
/// 因此母带里第一个非零样本的位置**只**反映 PDC 插在 `B → master` 边上的补偿。
///
/// 两条读数（帧）：
///
/// - `single`：只有一条支路（单链）且它声明 96 帧 ⇒ 起点 **0**（没有整体前滚）；
/// - `parallel`：两条支路、一条声明 96 帧 ⇒ 短支路被补 **96** 帧。
///
/// 结论：`compile_with_latencies_and_inserts` 的 PDC 是**逐边对齐**（[ARCH-PDC-001]），
/// 不是把整段母带往后推 `L_max` ⇒ 导出侧**不需要**再补偿；若另外裁掉 96 帧，
/// 反而会把关键路径上的真实音频剪掉、并让导出比监听**早** 96 帧。
#[test]
fn a_declared_latency_shifts_only_the_shorter_parallel_branch() {
    let fixture = Fixture::new("latency");

    // 单链：一台声明 96 帧延迟的 EQ。起点必须仍是 0（没有整体前滚）。
    let mut latched = eq_device(0.0);
    latched.latency_samples = 96;
    let single = render(
        &project_with_devices(vec![latched.clone()]),
        &fixture,
        "latency-single",
    );
    let single_onset = onset(&single.samples);

    // 两条并行支路：A（`latency` 帧、静音）、B（0 帧、有声）。
    // 只改 A 的**声明延迟**，别的字节一律不动 ⇒ 两次渲染的差就是 PDC 的读数。
    let parallel_project = |latency_frames: u32| -> YebanProjectV1 {
        let mut slow = latched.clone();
        slow.latency_samples = latency_frames;
        let mut project = project_with_devices(vec![slow]);
        let fast = fixture_id("T2");
        let (fast_track, fast_entry) = track_with_note(
            fast,
            "Fast",
            48,
            fixture_id("K2"),
            fixture_id("P2"),
            fixture_id("N2"),
        );
        attach(&mut project, fast_track, fast_entry, fixture_id("E2"));
        project.routing_graph.nodes.sort_unstable();
        // A（慢支路）静音：导出里只剩 B，起点读数就只反映 B 的补偿。
        project
            .tracks
            .get_mut(&fixture_id("T1"))
            .expect("A 必须在")
            .mute = true;
        project.validate().expect("两条支路的夹具必须合法");
        project
    };

    let fast_only = render(&parallel_project(0), &fixture, "latency-parallel-0");
    let compensated = render(&parallel_project(96), &fixture, "latency-parallel-96");
    let fast_onset = onset(&fast_only.samples);
    let slow_onset = onset(&compensated.samples);

    println!(
        "[244/latency] single_latency={} single_onset={single_onset} \
         parallel_onset_declared_0={fast_onset} parallel_onset_declared_96={slow_onset} \
         shift={}",
        latched.latency_samples,
        slow_onset as i64 - fast_onset as i64
    );
    assert!(
        single_onset <= 4,
        "单链声明的延迟不得变成整体前滚（起点必须仍是 0±4 帧）；实际 {single_onset}"
    );
    assert_eq!(
        slow_onset as i64 - fast_onset as i64,
        96,
        "并行支路的短支路必须被补 96 帧（PDC 逐边对齐）"
    );
}
