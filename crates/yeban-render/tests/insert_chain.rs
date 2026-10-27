//! **离线插入链**的判据：通道条（增益 → 3 段 EQ → 低通 → **压缩**）＋ Freeverb
//! ＋ 卷积混响。
//!
//! ⭐ 任务单 `#303` 起**压缩级执行**（之前整级不执行；见 `src/inserts.rs` §1.1）。
//! 本文件因此同时钉住三件事：① 压缩真的压（阈值以上）/ 真的不动（阈值以下）；
//! ② 零延迟（帧数与冲激位置不变、PDC 一条不动）；③ 没有压缩参数的工程**逐字节不变**
//! （正对照，母带摘要钉成常量）。
//!
//! 这些判据量的是**导出母带的样本**，不是"投影出来的参数"：每条都给出两档读数
//! （有器件 / 无器件基线），因此"接线了"与"没接线"在读数上是可区分的两件事。
//!
//! 顺序差异（实时 = 单声道插入 → 声相；离线 = 会压缩的纯源轨插入 → 声相、其余
//! 轨声相 → 求和 → 立体声插入）登记在 `src/inserts.rs` 的模块头 §2。本文件的 EQ 判据
//! 用**逐声道线性等价**的那条（纯正弦的基频带能量），混响/卷积判据量的是**尾音**，
//! 不声称与实时侧逐位相同。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, CurveType,
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind,
    TrackV3, YebanProjectV1,
};
use yeban_render::automation::{
    compressing_nodes_with_lanes, device_param_automations, forced_insert_instances,
};
use yeban_render::inserts::{
    InsertChain, project_inserts, project_inserts_with_forced_instances,
    project_inserts_with_forced_strips, statically_compressing_nodes, strip_source_params,
    strip_source_slot, track_has_unhandled_dsp,
};
use yeban_render::render::{
    AudioSource, BlockContext, InsertMeterReading, RenderError, RenderOptions, RenderOutput,
    RenderPlan,
};

/// 判据统一用 48 kHz（卷积 IR = 4 800 帧 = 0.1 s，与实时引擎同值）。
const SR: u32 = 48_000;

/// "第一个设备"用的固定 ULID 序号 —— 判据不依赖顺序，这里只为可读性。
fn node(seed: u32) -> EntityId {
    let mut text = [b'0'; 26];
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    for position in 0..4 {
        text[25 - position] = ALPHABET[((seed >> (5 * position)) & 0x1F) as usize];
    }
    core::str::from_utf8(&text)
        .expect("ASCII")
        .parse()
        .expect("合法 ULID")
}

fn param(name: &str, value: f32) -> ParameterValue {
    ParameterValue {
        name: name.to_owned(),
        value,
        unit: None,
    }
}

fn effect(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "FX".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| param(name, *value))
            .collect(),
        latency_samples: 0,
    }
}

fn track(devices: Vec<DeviceDefinition>) -> TrackV3 {
    TrackV3 {
        devices,
        ..TrackV3::default()
    }
}

fn graph(nodes: &[EntityId], edges: &[(EntityId, EntityId, RoutingKind)]) -> RoutingGraph {
    let mut map = BTreeMap::new();
    for &(source_node, destination_node, kind) in edges {
        let id = EntityId::new();
        map.insert(
            id,
            RoutingEdge {
                id,
                source_node,
                destination_node,
                kind,
                gain_db: None,
            },
        );
    }
    RoutingGraph {
        nodes: nodes.to_vec(),
        edges: map,
    }
}

/// 一段确定性的单声道缓冲：正弦 × 首尾 5 ms 余弦淡入淡出；`note_frames` 之后为静音。
///
/// 淡入淡出只用于避免"突然截断"制造宽带瞬态；它不影响两档读数之间的比较
/// （有器件与无器件用的是**同一条**缓冲）。
fn burst(freq: f64, frames: usize, note_frames: usize, amplitude: f32) -> Vec<f32> {
    let fade = (SR as usize / 200).max(1); // 5 ms
    (0..frames)
        .map(|index| {
            if index >= note_frames {
                return 0.0;
            }
            let phase = 2.0 * core::f64::consts::PI * freq * index as f64 / f64::from(SR);
            let mut envelope = 1.0f64;
            if index < fade {
                envelope = 0.5 - 0.5 * (core::f64::consts::PI * index as f64 / fade as f64).cos();
            }
            let remaining = note_frames - index;
            if remaining < fade {
                envelope *=
                    0.5 - 0.5 * (core::f64::consts::PI * remaining as f64 / fade as f64).cos();
            }
            (amplitude as f64 * envelope * phase.sin()) as f32
        })
        .collect()
}

/// 把一条单声道缓冲复制到各声道的源。
struct BufferSource {
    mono: Vec<f32>,
    position: usize,
}

impl BufferSource {
    fn new(mono: Vec<f32>) -> Self {
        Self { mono, position: 0 }
    }
}

impl AudioSource for BufferSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        for frame in 0..context.frames {
            let value = self.mono.get(self.position + frame).copied().unwrap_or(0.0);
            for channel in 0..context.channels {
                out[frame * context.channels + channel] = value;
            }
        }
        self.position += context.frames;
        Ok(())
    }
}

/// 渲染一个"源节点 → 输出"的图，返回母带。
fn render(
    routing: &RoutingGraph,
    master: EntityId,
    inserts: &BTreeMap<EntityId, InsertChain>,
    frames: usize,
    mono: Vec<f32>,
    source_node: EntityId,
) -> RenderOutput {
    let options = RenderOptions::l1(frames as u64, 2, SR, 0x155).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        routing,
        master,
        options,
        &BTreeMap::new(),
        inserts,
    )
    .expect("编译");
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(source_node, Box::new(BufferSource::new(mono)));
    plan.execute(sources).expect("渲染")
}

/// 交错立体声缓冲在 `[start, end)` 帧上的 RMS（两声道一起算）。
fn rms(samples: &[f32], start: usize, end: usize) -> f64 {
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

/// 左声道在 `freq` 上的 DFT 幅度平方（窗口内整数个周期时无泄漏）。
fn tone_energy(samples: &[f32], start: usize, end: usize, freq: f64) -> f64 {
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    let length = (end - start) as f64;
    for frame in start..end {
        let sample = f64::from(samples[frame * 2]);
        let phase = 2.0 * core::f64::consts::PI * freq * (frame - start) as f64 / f64::from(SR);
        re += sample * phase.cos();
        im -= sample * phase.sin();
    }
    (re * re + im * im) / (length * length)
}

fn hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 判据 1：`eq_low_gain: +12 dB` @ `low_freq: 120 Hz` 把 80 Hz 基频带能量抬高 ≥ 6 dB。
#[test]
fn a_low_shelf_raises_the_fundamental_band_by_more_than_six_db() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000; // 0.5 s
    let mono = burst(80.0, frames, frames, 0.1);

    let mut tracks = BTreeMap::new();
    tracks.insert(
        track_node,
        track(vec![effect(&[
            ("eq_low_gain", 12.0),
            ("eq_low_freq", 120.0),
        ])]),
    );
    let inserts = project_inserts(&tracks, SR);
    assert!(!inserts.is_empty(), "EQ 设备必须投影出非空链");
    let with_eq = render(&routing, master, &inserts, frames, mono.clone(), track_node);
    let baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        mono.clone(),
        track_node,
    );

    // 窗口 = 0.125 s … 0.5 s（避开 EQ 的起始瞬态）；80 Hz 在 0.375 s 里恰好 30 个周期。
    let start = 6_000;
    let end = frames;
    let boosted = tone_energy(&with_eq.samples, start, end, 80.0);
    let plain = tone_energy(&baseline.samples, start, end, 80.0);
    let gain_db = 10.0 * (boosted / plain).log10();
    println!(
        "EQ 读数: 基频带能量 有器件 = {boosted:.9e} / 无设备 = {plain:.9e} ⇒ {gain_db:+.3} dB"
    );
    assert!(
        gain_db >= 6.0,
        "低架 +12 dB @120 Hz 对 80 Hz 的能量提升只有 {gain_db:+.3} dB"
    );
}

/// 判据 2：轨 A `SendToAux` → Aux 轨上 `reverb_wet:1` / `reverb_size:0.8`，
/// 音符结束后 200–700 ms 的尾音 RMS 显著高于 `reverb_wet:0` 基线。
#[test]
fn the_reverb_on_the_aux_return_keeps_ringing_after_the_note_ends() {
    let master = node(0xFF);
    let source = node(1);
    let aux = node(2);
    let routing = graph(
        &[master, source, aux],
        &[
            (source, aux, RoutingKind::SendToAux),
            (aux, master, RoutingKind::TrackToBus),
        ],
    );
    let frames = 72_000; // 1.5 s
    let note_frames = 4_800; // 0.1 s
    let mono = burst(440.0, frames, note_frames, 0.3);

    let mut wet_tracks = BTreeMap::new();
    wet_tracks.insert(
        aux,
        track(vec![effect(&[("reverb_size", 0.8), ("reverb_wet", 1.0)])]),
    );
    let wet_inserts = project_inserts(&wet_tracks, SR);
    let wet = render(&routing, master, &wet_inserts, frames, mono.clone(), source);

    let mut dry_tracks = BTreeMap::new();
    dry_tracks.insert(
        aux,
        track(vec![effect(&[("reverb_size", 0.8), ("reverb_wet", 0.0)])]),
    );
    let dry_inserts = project_inserts(&dry_tracks, SR);
    let dry = render(&routing, master, &dry_inserts, frames, mono, source);

    // 音符结束 (4 800) 之后 200–700 ms。
    let tail_start = note_frames + 9_600;
    let tail_end = note_frames + 33_600;
    let wet_tail = rms(&wet.samples, tail_start, tail_end);
    let dry_tail = rms(&dry.samples, tail_start, tail_end);
    let ratio = if dry_tail > 0.0 {
        format!("{:.1}×", wet_tail / dry_tail)
    } else {
        "干 = 0 ⇒ 比值未定义".to_owned()
    };
    println!("混响读数: 200–700 ms 尾音 RMS 湿 = {wet_tail:.9e} / 干 = {dry_tail:.9e}（{ratio}）");
    assert!(
        wet_tail > 1.0e-4,
        "混响尾音几乎为零 —— 插入链没有作用到 Aux 返回轨: {wet_tail:.9e}"
    );
    assert!(
        wet_tail > dry_tail * 1_000.0,
        "湿尾应当远高于干尾: {wet_tail:.9e} vs {dry_tail:.9e}"
    );
    assert_eq!(dry_tail, 0.0, "wet=0 时器件整级跳过, 尾音必须是精确的 0");
}

/// 判据 3：同形状的卷积混响 —— IR 只有 0.1 s，因此尾音窗口取音符结束后 20–90 ms；
/// 同时给出 200–700 ms 的读数（按 IR 长度它**必然**是 0，如实报告）。
#[test]
fn the_convolution_on_the_aux_return_rings_for_the_length_of_its_ir() {
    let master = node(0xFF);
    let source = node(1);
    let aux = node(2);
    let routing = graph(
        &[master, source, aux],
        &[
            (source, aux, RoutingKind::SendToAux),
            (aux, master, RoutingKind::TrackToBus),
        ],
    );
    let frames = 72_000;
    let note_frames = 4_800;
    let mono = burst(440.0, frames, note_frames, 0.3);

    let mut wet_tracks = BTreeMap::new();
    wet_tracks.insert(
        aux,
        track(vec![effect(&[
            ("conv_wet", 1.0),
            ("conv_dry", 0.0),
            ("conv_ir_decay_s", 0.35),
            ("conv_ir_seed", 1.0),
        ])]),
    );
    let wet_inserts = project_inserts(&wet_tracks, SR);
    let wet = render(&routing, master, &wet_inserts, frames, mono.clone(), source);

    let mut dry_tracks = BTreeMap::new();
    dry_tracks.insert(
        aux,
        track(vec![effect(&[("conv_wet", 0.0), ("conv_dry", 1.0)])]),
    );
    let dry_inserts = project_inserts(&dry_tracks, SR);
    let dry = render(&routing, master, &dry_inserts, frames, mono, source);

    // IR = 4 800 帧 ⇒ 尾音只能活到音符结束后 100 ms（4 800 帧）。
    let tail_start = note_frames + 960; // 20 ms
    let tail_end = note_frames + 4_320; // 90 ms
    let wet_tail = rms(&wet.samples, tail_start, tail_end);
    let dry_tail = rms(&dry.samples, tail_start, tail_end);
    let late_start = note_frames + 9_600; // 200 ms
    let late_end = note_frames + 33_600; // 700 ms
    let wet_late = rms(&wet.samples, late_start, late_end);
    let dry_late = rms(&dry.samples, late_start, late_end);
    println!(
        "卷积读数: 20–90 ms 尾音 RMS 湿 = {wet_tail:.9e} / 干 = {dry_tail:.9e}；\
         200–700 ms RMS 湿 = {wet_late:.9e} / 干 = {dry_late:.9e}"
    );
    assert!(
        wet_tail > 1.0e-4,
        "卷积尾音几乎为零 —— 插入链没有作用到 Aux 返回轨: {wet_tail:.9e}"
    );
    assert!(
        wet_tail > dry_tail * 1_000.0,
        "湿尾应当远高于干尾: {wet_tail:.9e} vs {dry_tail:.9e}"
    );
    assert_eq!(dry_tail, 0.0, "wet=0 时器件整级跳过, 尾音必须是精确的 0");
    assert_eq!(wet_late, 0.0, "IR 只有 0.1 s ⇒ 200–700 ms 必须是 0");
    assert_eq!(dry_late, 0.0);
}

/// 判据 5：同一工程连渲两次 ⇒ SHA-256 相同（同一计划重复执行 + 全新编译各一次）。
#[test]
fn two_renders_of_the_same_project_have_the_same_sha256() {
    let master = node(0xFF);
    let source = node(1);
    let aux = node(2);
    let routing = graph(
        &[master, source, aux],
        &[
            (source, aux, RoutingKind::SendToAux),
            (aux, master, RoutingKind::TrackToBus),
        ],
    );
    let frames = 24_000;
    let mono = burst(220.0, frames, 12_000, 0.2);
    let options = RenderOptions::l1(frames as u64, 2, SR, 0x155).with_threads(1);
    let mut tracks = BTreeMap::new();
    tracks.insert(
        aux,
        track(vec![
            effect(&[("eq_low_gain", 6.0), ("eq_low_freq", 200.0)]),
            effect(&[("reverb_size", 0.7), ("reverb_wet", 0.6)]),
            effect(&[("conv_wet", 0.5), ("conv_dry", 0.5)]),
        ]),
    );
    let inserts = project_inserts(&tracks, SR);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &routing,
        master,
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("编译");
    let first = plan
        .execute(BTreeMap::from([(
            source,
            Box::new(BufferSource::new(mono.clone())) as Box<dyn AudioSource>,
        )]))
        .expect("第一次");
    let second = plan
        .execute(BTreeMap::from([(
            source,
            Box::new(BufferSource::new(mono.clone())) as Box<dyn AudioSource>,
        )]))
        .expect("第二次");
    let mut fresh = RenderPlan::compile_with_latencies_and_inserts(
        &routing,
        master,
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("重新编译");
    let third = fresh
        .execute(BTreeMap::from([(
            source,
            Box::new(BufferSource::new(mono)) as Box<dyn AudioSource>,
        )]))
        .expect("第三次");
    println!(
        "确定性读数: 同一计划两次 = {} / {}；全新编译 = {}",
        hex(&first.digest),
        hex(&second.digest),
        hex(&third.digest)
    );
    assert_eq!(first.digest, second.digest, "同一计划第二次执行的摘要不同");
    assert_eq!(first.digest, third.digest, "全新编译的摘要不同");
}

/// 判据 6a（⭐ **任务单 `#173` 修订**）：主总线上的设备链**被应用**。
///
/// ## 为什么改（原文见文件末尾的修订登记）
///
/// `#155` 的原文断言是"主总线插入被应用了 —— 实时引擎跳过 master, 这会制造
/// 导出/监听不一致"（`assert_eq!(touched.samples, baseline.samples)`）。那条断言
/// 在 `#173` 之后**正好把一个缺陷钉成了不变量**：`rt.rs` 的 3a⁗⁗ 现在处理主总线，
/// 离线侧也把 `render.rs` 的 `None` 删掉了 ⇒ 若还要求"母带逐位不变"，主总线设备
/// 就又变成听不到。故按**三件套**改：① 实时侧 `rt.rs`（3a⁗⁗ ＋ 武装池 ＋ 计数）；
/// ② 离线侧 `render.rs`（master 分支不再写 `None`）＋ 本判据；③ 模块头
/// （`src/inserts.rs` §1）登记"为什么改 / 仍存的形状差"。
///
/// 三个读数（都从母带样本现算）：
/// 1. **EQ 真的生效**：master 低架 +12 dB @ 120 Hz，80 Hz 基频带能量提升（dB）；
/// 2. **摘要有牙**：有设备 / 无设备的 SHA-256 不同（⛔ 不是"有输出就算过"）；
/// 3. **反向腿**：同一台 master 设备 `bypassed = true` ⇒ 投影为空链 ⇒ 母带与
///    无设备基线**逐位相同**（证明读数来自链，不是"多挂一个节点"的副作用）。
#[test]
fn master_bus_inserts_are_applied() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000; // 0.5 s（与判据 1 同一个窗口口径）
    let mono = burst(80.0, frames, frames, 0.1);

    let mut tracks = BTreeMap::new();
    tracks.insert(
        master,
        track(vec![effect(&[
            ("eq_low_gain", 12.0),
            ("eq_low_freq", 120.0),
        ])]),
    );
    let with_master_inserts = project_inserts(&tracks, SR);
    assert!(
        with_master_inserts.contains_key(&master),
        "投影必须包含 master（投影规则本身不剔除任何节点）"
    );
    let touched = render(
        &routing,
        master,
        &with_master_inserts,
        frames,
        mono.clone(),
        track_node,
    );
    let baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        mono.clone(),
        track_node,
    );
    assert_ne!(
        touched.samples, baseline.samples,
        "主总线插入**没有被应用** —— 离线导出里 master 上的设备又听不到了"
    );

    // 窗口 = 0.125 s … 0.5 s（避开 EQ 的起始瞬态）；80 Hz 在 0.375 s 里恰好 30 个周期。
    let boosted = tone_energy(&touched.samples, 6_000, frames, 80.0);
    let plain = tone_energy(&baseline.samples, 6_000, frames, 80.0);
    let gain_db = 10.0 * (boosted / plain).log10();
    println!(
        "主总线 EQ 读数: 80 Hz 基频带能量 有 master 设备 = {boosted:.9e} / 无 = {plain:.9e} \
         ⇒ {gain_db:+.3} dB；摘要 有设备 = {} / 无 = {}",
        hex(&touched.digest),
        hex(&baseline.digest)
    );
    assert!(
        gain_db >= 6.0,
        "master 低架 +12 dB @120 Hz 对 80 Hz 的能量提升只有 {gain_db:+.3} dB"
    );

    // ③ 反向腿：旁通 ⇒ 投影为空链 ⇒ 与基线逐位相同。
    let mut bypassed = effect(&[("eq_low_gain", 12.0), ("eq_low_freq", 120.0)]);
    bypassed.bypassed = true;
    let mut bypassed_tracks = BTreeMap::new();
    bypassed_tracks.insert(master, track(vec![bypassed]));
    let bypassed_inserts = project_inserts(&bypassed_tracks, SR);
    assert!(
        !bypassed_inserts.contains_key(&master),
        "旁通的 master 设备不得投影成链"
    );
    let untouched = render(
        &routing,
        master,
        &bypassed_inserts,
        frames,
        mono,
        track_node,
    );
    assert_eq!(
        untouched.samples, baseline.samples,
        "旁通的 master 设备必须让母带与无设备基线逐位相同"
    );
}

/// 判据 6b：没有可识别参数 / 全部旁通 ⇒ 空链 ⇒ 输出与无设备时**逐位相同**。
#[test]
fn unknown_or_bypassed_devices_leave_the_audio_bit_identical() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 12_000;
    let mono = burst(440.0, frames, 8_000, 0.2);
    let mut unknown_tracks = BTreeMap::new();
    unknown_tracks.insert(
        track_node,
        track(vec![effect(&[
            ("filter_cutoff_hz", 800.0),
            ("reverb_mix", 0.3),
        ])]),
    );
    let unknown_inserts = project_inserts(&unknown_tracks, SR);
    assert!(unknown_inserts.is_empty(), "未知名字不得被猜成器件");
    assert!(
        track_has_unhandled_dsp(&unknown_tracks[&track_node]),
        "未知参数必须仍被登记"
    );
    let unknown = render(
        &routing,
        master,
        &unknown_inserts,
        frames,
        mono.clone(),
        track_node,
    );
    let baseline = render(&routing, master, &BTreeMap::new(), frames, mono, track_node);
    assert_eq!(unknown.samples, baseline.samples);
}

/// 投影的白名单边界：EQ / 混响 / 卷积认什么、刻意不认什么。
#[test]
fn the_projection_recognises_only_the_documented_names() {
    let eq = InsertChain::from_devices(
        &[effect(&[
            ("eq_low_gain", 12.0),
            ("low_freq", 120.0),
            ("eq_mid_q", 2.0),
        ])],
        SR,
    );
    let params = eq.eq().expect("EQ 应被识别");
    assert_eq!(params.low_gain, 12.0);
    assert_eq!(params.low_freq, 120.0);
    assert_eq!(params.mid_q, 2.0);

    let reverb =
        InsertChain::from_devices(&[effect(&[("reverb_size", 0.8), ("reverb_wet", 1.0)])], SR);
    let params = reverb.reverb().expect("混响应被识别");
    assert_eq!(params.size, 0.8);
    assert_eq!(params.mix, 1.0);

    let convolution =
        InsertChain::from_devices(&[effect(&[("conv_wet", 1.0), ("conv_ir_seed", 7.0)])], SR);
    let plan = convolution.convolution().expect("卷积应被识别");
    assert_eq!(plan.ir().len(), 4_800, "48 kHz ⇒ IR = 采样率 ÷ 10");
    assert_eq!(plan.params().wet, 1.0);

    // `reverb_mix` / `reverb_width` 刻意不认（与引擎 §8.1 同口径）。
    let not_recognised =
        InsertChain::from_devices(&[effect(&[("reverb_mix", 0.3), ("reverb_width", 1.0)])], SR);
    assert!(not_recognised.is_empty());
}

/// 通道条的投影边界：增益 / 滤波认什么、刻意不认什么、压缩为什么是"有投影但不执行"。
#[test]
fn the_strip_projection_mirrors_the_engine_whitelist() {
    let chain = InsertChain::from_devices(
        &[effect(&[
            ("trim_db", 3.0),
            ("output_gain", -2.0),
            ("cutoff_hz", 800.0),
            ("resonance", 0.4),
            ("filter_drive", 0.25),
        ])],
        SR,
    );
    let params = chain.strip().expect("通道条应被识别");
    assert_eq!(
        params.input_gain_db, 3.0,
        "`trim_db` 是 `input_gain_db` 的别名"
    );
    assert_eq!(params.output_gain_db, -2.0);
    assert!(
        !params.eq_enabled,
        "没写 EQ 旋钮 ⇒ EQ 级不执行（引擎 §4.1）"
    );
    assert!(!params.compressor_enabled, "没写动态旋钮 ⇒ 压缩级不执行");
    assert!(params.filter_enabled);
    assert_eq!(params.filter.cutoff_hz, 800.0);
    assert_eq!(params.filter.resonance, 0.4);
    assert_eq!(params.filter.drive, 0.25);
    assert!(!chain.is_empty());

    // ⚠ `filter_cutoff_hz` 刻意不认（引擎模块文档 §4.3）：只有 `cutoff_hz`。
    let not_recognised = InsertChain::from_devices(&[effect(&[("filter_cutoff_hz", 800.0)])], SR);
    assert!(not_recognised.is_empty());

    // ⭐ `#303`：只有压缩参数 ⇒ **非空链**（压缩级自本单起真的执行）。
    let compressor_only = InsertChain::from_devices(&[effect(&[("threshold_db", -30.0)])], SR);
    assert!(compressor_only.strip().is_some(), "压缩仍是通道条的一部分");
    assert!(
        !compressor_only.is_empty(),
        "压缩级已接通（#303）⇒ 只有压缩参数的链必须挂运行态"
    );
    assert!(
        compressor_only.compresses_statically(),
        "`threshold_db` ⇒ 压缩级执行"
    );
    // 显式写 0 的开关仍然关：全恒等 ⇒ 空链（`#303` ⛔ 不改变这条默认口径）。
    let disabled = InsertChain::from_devices(&[effect(&[("compressor_enabled", 0.0)])], SR);
    assert!(
        disabled.is_empty(),
        "`compressor_enabled: 0` 且三级全关 ⇒ 空链"
    );
    assert!(!disabled.compresses_statically());
}

/// 机制判据：通道条白名单逐名对账（引擎 `insert.rs` 的**冻结副本**）。
///
/// `yeban-render` 不能依赖 `yeban-engine`（依赖方向门），因此这里保存一份逐字抄写的
/// 名单。每个通道条名字都必须被投影认下、都**已被消费**（⛔ 不许再登记
/// `deviceChainDsp`）。⚠ `#303` 之前压缩的 14 个拼写是例外（整级不执行 ⇒ 仍登记）；
/// 三个刻意不认的名字（`filter_cutoff_hz` / `reverb_mix` / `reverb_width`）**现在仍是**
/// "仍登记"的那一档（引擎同样不认它们）。
#[test]
fn strip_names_are_the_engine_whitelist() {
    const STRIP_NAMES: &[&str] = &[
        "input_gain_db",
        "input_gain",
        "trim_db",
        "output_gain_db",
        "output_gain",
        "fader_db",
        "eq_enabled",
        "filter_enabled",
        "compressor_enabled",
        "cutoff_hz",
        "resonance",
        "filter_resonance",
        "drive",
        "filter_drive",
        "eq_low_gain",
        "low_gain",
        "eq_low_freq",
        "low_freq",
        "eq_mid_gain",
        "mid_gain",
        "eq_mid_freq",
        "mid_freq",
        "eq_mid_q",
        "mid_q",
        "eq_high_gain",
        "high_gain",
        "eq_high_freq",
        "high_freq",
    ];
    // ⭐ `#303`：28 个通道条拼写**全部**被消费（压缩从"整级不执行"变成"执行"）。
    for &name in STRIP_NAMES {
        let chain = InsertChain::from_devices(&[effect(&[(name, 0.5)])], SR);
        assert!(chain.strip().is_some(), "`{name}` 必须被通道条投影认下");
        assert!(
            !track_has_unhandled_dsp(&track(vec![effect(&[(name, 0.5)])])),
            "`{name}` 已被消费，不该再登记 `deviceChainDsp`"
        );
        assert!(
            !chain.is_empty(),
            "`{name}` 是开关/参数之一 ⇒ 链里至少有一个可执行级"
        );
    }

    // 压缩的 14 个拼写：单写任何一个都会让压缩级执行（投影的 `compressor_seen` 规则）。
    const COMPRESSOR_NAMES: &[&str] = &[
        "compressor_enabled",
        "threshold_db",
        "threshold",
        "ratio",
        "knee_db",
        "knee",
        "detector_s",
        "detector",
        "attack_s",
        "attack",
        "release_s",
        "release",
        "makeup_db",
        "makeup",
    ];
    for &name in COMPRESSOR_NAMES {
        let devices = [effect(&[(name, -20.0)])];
        let chain = InsertChain::from_devices(&devices, SR);
        assert!(chain.strip().is_some(), "`{name}` 仍是引擎认的通道条名字");
        assert!(
            chain.compresses_statically(),
            "`{name}` 单独出现 ⇒ 压缩级必须执行（#303）"
        );
        assert!(
            !track_has_unhandled_dsp(&track(vec![effect(&[(name, -20.0)])])),
            "`{name}` 已被离线求值 ⇒ 不许再登记 `deviceChainDsp`"
        );
    }

    for &name in &["filter_cutoff_hz", "reverb_mix", "reverb_width"] {
        let devices = [effect(&[(name, 0.5)])];
        let chain = InsertChain::from_devices(&devices, SR);
        assert!(chain.is_empty(), "`{name}` 刻意不认 ⇒ 不得投影出器件");
        assert!(
            track_has_unhandled_dsp(&track(vec![effect(&[(name, 0.5)])])),
            "`{name}` 没有被求值 ⇒ 必须登记"
        );
    }
}

/// 判据：输入 / 输出增益级在离线导出里真的执行 —— 各写 -6 dB，基频带能量降 6.02 dB。
#[test]
fn the_gain_stages_scale_the_master_by_the_written_db() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000;
    let mono = burst(440.0, frames, frames, 0.1);
    let baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        mono.clone(),
        track_node,
    );
    let plain = tone_energy(&baseline.samples, 6_000, frames, 440.0);

    for (name, value) in [("input_gain_db", -6.0f32), ("output_gain_db", -6.0f32)] {
        let mut tracks = BTreeMap::new();
        tracks.insert(track_node, track(vec![effect(&[(name, value)])]));
        let inserts = project_inserts(&tracks, SR);
        assert!(!inserts.is_empty(), "`{name}` 必须投影出非空链");
        let rendered = render(&routing, master, &inserts, frames, mono.clone(), track_node);
        let energy = tone_energy(&rendered.samples, 6_000, frames, 440.0);
        let gain_db = 10.0 * (energy / plain).log10();
        println!("增益读数: `{name} = {value}` ⇒ 基频带能量 {gain_db:+.3} dB");
        assert!(
            (gain_db - (-6.0206)).abs() < 0.05,
            "`{name} = {value}` 的实测增益是 {gain_db:+.3} dB，不是 -6.02 dB"
        );
    }
}

/// 判据：低通滤波级在离线导出里真的执行 —— 500 Hz 截止把 2 kHz 基频带压低。
#[test]
fn the_low_pass_filter_shapes_the_master() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000;
    let mono = burst(2_000.0, frames, frames, 0.1);
    let baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        mono.clone(),
        track_node,
    );
    let plain = tone_energy(&baseline.samples, 6_000, frames, 2_000.0);

    let mut tracks = BTreeMap::new();
    tracks.insert(
        track_node,
        track(vec![effect(&[("cutoff_hz", 500.0), ("resonance", 0.0)])]),
    );
    let inserts = project_inserts(&tracks, SR);
    assert!(!inserts.is_empty(), "`cutoff_hz` 必须投影出非空链");
    let filtered = render(&routing, master, &inserts, frames, mono, track_node);
    let energy = tone_energy(&filtered.samples, 6_000, frames, 2_000.0);
    let attenuation_db = 10.0 * (energy / plain).log10();
    println!("低通读数: 2 kHz 在 500 Hz 截止下 {attenuation_db:+.3} dB");
    assert!(
        attenuation_db <= -20.0,
        "500 Hz 低通对 2 kHz 的衰减只有 {attenuation_db:+.3} dB —— 滤波级没有执行"
    );
}

/// 判据：`track_has_unhandled_dsp` 的口径（MCP 的 `deviceChainDsp` 登记依据）。
#[test]
fn unhandled_device_classification_matches_the_slice_boundary() {
    let eq_track = track(vec![effect(&[("eq_low_gain", 6.0)])]);
    assert!(!track_has_unhandled_dsp(&eq_track), "EQ 已被消费");

    let gain_track = track(vec![effect(&[("input_gain_db", -6.0)])]);
    assert!(
        !track_has_unhandled_dsp(&gain_track),
        "增益级已被通道条消费"
    );

    let filter_track = track(vec![effect(&[("cutoff_hz", 500.0)])]);
    assert!(
        !track_has_unhandled_dsp(&filter_track),
        "滤波级已被通道条消费"
    );

    let empty_effect = track(vec![effect(&[])]);
    assert!(
        track_has_unhandled_dsp(&empty_effect),
        "没有参数可求值 ≠ 已经求值（既有 MCP 判据的口径）"
    );

    let mut bypassed = effect(&[("eq_low_gain", 6.0)]);
    bypassed.bypassed = true;
    assert!(
        !track_has_unhandled_dsp(&track(vec![bypassed])),
        "旁通设备不参与求值，也不算未消费"
    );

    let mut instrument = effect(&[("eq_low_gain", 6.0)]);
    instrument.kind = DeviceKind::InternalInstrument;
    assert!(
        track_has_unhandled_dsp(&track(vec![instrument])),
        "音源**不认**的名字（`eq_low_gain` 是通道条旋钮）仍必须登记"
    );

    // ⭐ G5（审计 2026-10-27）：引擎音源**真的求值**的参数不再登记。
    // 判据口径 = `yeban_engine::synth::is_consumed_instrument_param`。
    let mut wave = effect(&[("osc1_wave", 1.0), ("osc2_wave", 4.0)]);
    wave.kind = DeviceKind::InternalInstrument;
    assert!(
        !track_has_unhandled_dsp(&track(vec![wave])),
        "音色 `osc*_wave` 由引擎音源投影求值（G5）⇒ 报 deviceChainDsp 就是假话"
    );

    let mut drum_kit = effect(&[("kick_note", 36.0), ("kick_tune_hz", 50.0)]);
    drum_kit.kind = DeviceKind::InternalInstrument;
    assert!(
        !track_has_unhandled_dsp(&track(vec![drum_kit])),
        "鼓机键位与旋钮由引擎鼓机投影求值 ⇒ 不登记"
    );

    let mut mixed = effect(&[("osc1_wave", 1.0), ("mystery", 2.0)]);
    mixed.kind = DeviceKind::InternalInstrument;
    assert!(
        track_has_unhandled_dsp(&track(vec![mixed])),
        "乐器设备里混着**没人消费**的名字 ⇒ 仍如实登记"
    );

    let mut empty_instrument = effect(&[]);
    empty_instrument.kind = DeviceKind::InternalInstrument;
    assert!(
        !track_has_unhandled_dsp(&track(vec![empty_instrument])),
        "空参数的乐器 = 出厂默认音色, 没有'没被求值'的东西（InternalEffect 的\
         空参数规则**不**外推到音源）"
    );

    // ⭐ `#303`：压缩已被离线插入链消费 ⇒ 不再登记。
    let compressor = track(vec![effect(&[("threshold_db", -12.0), ("ratio", 4.0)])]);
    assert!(
        !track_has_unhandled_dsp(&compressor),
        "压缩已接通（#303）⇒ 不许再登记 deviceChainDsp"
    );

    let gain_and_compressor = track(vec![effect(&[
        ("input_gain_db", -6.0),
        ("threshold_db", -12.0),
    ])]);
    assert!(
        !track_has_unhandled_dsp(&gain_and_compressor),
        "增益与压缩都被消费 ⇒ 不应登记"
    );

    // 正对照：刻意不认的名字仍必须登记（否则这条判据变成"什么都不登记"）。
    let unknown = track(vec![effect(&[("mystery", 1.0)])]);
    assert!(track_has_unhandled_dsp(&unknown), "未知旋钮必须仍登记");
}

/// 判据：非立体声 + 非空插入链 ⇒ 如实失败（不静默跳过混响/卷积）。
#[test]
fn a_non_stereo_target_refuses_a_non_empty_insert_chain() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let options = RenderOptions::l1(4_800, 1, SR, 1);
    let mut tracks = BTreeMap::new();
    tracks.insert(track_node, track(vec![effect(&[("reverb_wet", 0.5)])]));
    let inserts = project_inserts(&tracks, SR);
    let result = RenderPlan::compile_with_latencies_and_inserts(
        &routing,
        master,
        options,
        &BTreeMap::new(),
        &inserts,
    );
    assert_eq!(
        result.err(),
        Some(RenderError::UnsupportedInsertChannels(1))
    );
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#240`：源与入边在**插入链之前**的同一个输入端相加（Logic 通道条）
// ---------------------------------------------------------------------------

/// 判据：总线自己的源与入边**在插入链之前**已经合并。
///
/// 形状（与 Logic 的通道条同形）：`source_node ─SendToAux→ bus_node → master`，
/// `bus_node` 自己也有一个源。BUS 上挂一段 `eq_low_gain = -12 dB @ 120 Hz`。
///
/// 区分的两种实现：
///
/// - **输入端合并（本单）**：`EQ(source + send)`。无 EQ 时两路和 = `0.75`；
///   挂 EQ 后稳态 ≈ `0.75 × g`（g ≈ -12 dB）。
/// - **输出端另加源（错误形状）**：`EQ(send) + source` ⇒ 稳态 ≈ `0.25 × g + 0.5`，
///   会比 `0.5` **大**。
///
/// 判据因此同时钉住两件事：① 两路都进了同一条链；② 不是"源在链后另加"。
#[test]
fn the_source_and_the_incoming_edge_are_summed_before_the_insert_chain() {
    let master = node(0xFF);
    let send_source = node(1);
    let bus = node(2);
    let routing = graph(
        &[master, send_source, bus],
        &[
            (send_source, bus, RoutingKind::SendToAux),
            (bus, master, RoutingKind::TrackToBus),
        ],
    );
    let frames = 48_000;

    let mut tracks = BTreeMap::new();
    tracks.insert(
        bus,
        track(vec![effect(&[
            ("eq_low_gain", -12.0),
            ("eq_low_freq", 120.0),
        ])]),
    );
    let inserts = project_inserts(&tracks, SR);

    // (bus 自己的源, 入边源) ⇒ 母带。`with_eq = false` 是输入端求和的直接读数。
    let render_pair = |bus_source: f32, incoming: f32, with_eq: bool| -> RenderOutput {
        let options = RenderOptions::l1(frames as u64, 2, SR, 0x240).with_threads(1);
        let plan_inserts = if with_eq {
            inserts.clone()
        } else {
            BTreeMap::new()
        };
        let mut plan = RenderPlan::compile_with_latencies_and_inserts(
            &routing,
            master,
            options,
            &BTreeMap::new(),
            &plan_inserts,
        )
        .expect("编译");
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        sources.insert(
            send_source,
            Box::new(BufferSource::new(vec![incoming; frames])),
        );
        sources.insert(bus, Box::new(BufferSource::new(vec![bus_source; frames])));
        plan.execute(sources).expect("渲染")
    };

    // 左声道在末尾窗口的均值（避开 EQ 的起始瞬态）。
    let tail = |output: &RenderOutput| -> f64 {
        let start = frames - 480;
        let mut sum = 0.0f64;
        for frame in start..frames {
            sum += f64::from(output.samples[frame * 2]);
        }
        sum / 480.0
    };

    // bus → master 是单位增益边、master 没有插入链 ⇒ 母带读数就是 **bus 插入链入口**的读数
    // （无 EQ 时 0.75 即两路在输入端相加的结果）。
    let both_dry = render_pair(0.5, 0.25, false);
    let dry = tail(&both_dry);
    println!("[240/input-sum] 无 EQ 的两路和（左声道末尾均值）= {dry:.9}");
    assert!(
        (dry - 0.75).abs() < 1e-6,
        "输入端求和必须 = 0.5（自己的源）+ 0.25（入边）= 0.75，实际 {dry:.9}"
    );

    let both_wet = render_pair(0.5, 0.25, true);
    let source_wet = render_pair(0.5, 0.0, true);
    let send_wet = render_pair(0.0, 0.25, true);
    let (combined, only_source, only_send) = (tail(&both_wet), tail(&source_wet), tail(&send_wet));
    println!(
        "[240/before-inserts] 合 = {combined:.9} / 只源 = {only_source:.9} / \
         只发送 = {only_send:.9} / 源+发送 = {:.9} / 差 = {:.3e}",
        only_source + only_send,
        combined - (only_source + only_send)
    );

    assert!(
        // f32 的 IIR 在 48 000 帧上有累积舍入；6.1e-5 是实测读数，1e-3 是判据余量。
        (combined - (only_source + only_send)).abs() < 1e-3,
        "插入链必须作用在**合并后**的输入端信号上: 合 {combined:.9} vs 源+发送 {:.9}",
        only_source + only_send
    );
    assert!(
        combined < 0.5,
        "源若被放到插入链之后另加, 合会 ≥ 0.5（当前 {combined:.9}）—— 那不是 Logic 的形状"
    );
}

// ---------------------------------------------------------------------------
// ⭐ 修订登记（任务单 `#173`）：判据 6a 从"master **不**处理"改为"master 处理"
// ---------------------------------------------------------------------------
//
// **被修订的原文**（`#155`，逐字）：
//
// ```text
// /// 判据 6a：主总线上的设备链**不**被应用（实时引擎跳过 master）。
// fn master_bus_inserts_are_not_applied() { … }
// assert_eq!(touched.samples, baseline.samples,
//     "主总线插入被应用了 —— 实时引擎跳过 master, 这会制造导出/监听不一致");
// ```
//
// **为什么原文在那时是对的**：实时引擎的逐轨循环跳过 master、离线计划也把 master 的
// 链写成 `None` ⇒ 两条路都听不到；那时的 `assert_eq!` 把"两条路一致（都不处理）"
// 钉住。**`#173` 之后它变成缺陷的机械形式**：实时侧补了 3a⁗⁗、离线侧删掉了 `None`
// ⇒ 若还要求母带逐位不变，主总线设备就又变成"挂得上、听不到"。
//
// **改法（三件套）**：
// 1. `crates/yeban-engine/src/rt.rs`：`INSERT_SLOTS = MAX_TRACK_SLOTS + 1` 的主总线
//    槽位 ＋ 三个武装循环"主总线收尾" ＋ 3a⁗⁗（下混 → 链 → 复制两路）＋
//    `master_insert_frames` 覆盖度计数；零分配判据 `tests/rt_zero_alloc.rs` ㊺。
// 2. `crates/yeban-render/src/render.rs`：`node == master ⇒ None` 删除，master 与其它
//    总线走同一条"求和之后 → 插入"；非立体声守卫改为**包含** master。
// 3. `crates/yeban-render/src/inserts.rs` §1：登记"为什么改"与"仍存的形状差"
//    （实时单声道插入 vs 离线立体声插入 ⇒ 宽度/尾巴形状差异，与 `#155`/`#158` 同族）。

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#266`：设备参数自动化的**编译期强制实例化**
// ---------------------------------------------------------------------------

/// 只写 `output_gain_db: 0.0` 的设备：`from_devices` 的"已识别"兜底
/// （`params != DEFAULT`）不认它 ⇒ 静态投影是 `None`；但设备参数泳道要改它。
#[test]
fn a_forced_strip_exists_even_when_the_projection_is_the_identity() {
    let track_id = node(2);
    let mut tracks = BTreeMap::new();
    tracks.insert(track_id, track(vec![effect(&[("output_gain_db", 0.0)])]));
    assert!(
        !project_inserts(&tracks, SR).contains_key(&track_id),
        "静态投影是恒等 ⇒ 默认不挂运行态"
    );
    let forced = project_inserts_with_forced_strips(&tracks, SR, &BTreeSet::from([track_id]));
    let chain = forced.get(&track_id).expect("强制实例化必须留下一条链");
    assert!(chain.is_forced_strip());
    assert!(!chain.is_empty(), "有器件可改 ⇒ 不是空链");
    // 投影原样仍是 `None`（⛔ 本单不改引擎镜像）；强制位让**运行态**用 DEFAULT
    // 参数（三级全关、增益 0 dB）实例化一台恒等通道条 —— 泳道随后在块边界改它。
    assert!(chain.strip().is_none());
    assert_eq!(
        strip_source_slot(&tracks[&track_id]),
        None,
        "引擎的投影规则仍认为本轨没有通道条来源（⛔ 本单不改引擎镜像）"
    );
}

/// "来源设备"只有一处判定：`strip_source_slot` / `strip_source_params` 必须与
/// `from_devices` 选出**同一台**设备（首个含已识别名字的非旁通内置效果器）。
#[test]
fn strip_source_lookup_matches_the_projection_source() {
    // 槽位 0 只有一个不认识的旋钮；槽位 1 才是通道条来源。
    let track_id = node(3);
    let mut tracks = BTreeMap::new();
    tracks.insert(
        track_id,
        track(vec![
            effect(&[("mix", 0.5)]),
            effect(&[("output_gain_db", -6.0), ("eq_low_gain", 3.0)]),
        ]),
    );
    assert_eq!(strip_source_slot(&tracks[&track_id]), Some(1));
    let projected = strip_source_params(&tracks[&track_id]).expect("槽位 1 是来源");
    assert_eq!(projected.output_gain_db.to_bits(), (-6.0f32).to_bits());
    let chain = project_inserts(&tracks, SR);
    let raw = chain[&track_id].strip().expect("投影出通道条");
    assert_eq!(
        raw.output_gain_db.to_bits(),
        projected.output_gain_db.to_bits()
    );
    assert_eq!(
        raw.input_gain_db.to_bits(),
        projected.input_gain_db.to_bits()
    );
    assert_eq!(raw.eq_enabled, projected.eq_enabled);
}

/// 强制实例化**不改变**恒等判定以外的东西：没有强制的轨仍按原规则（空链被剔除）。
#[test]
fn forcing_one_track_does_not_materialise_other_tracks() {
    let quiet = node(4);
    let loud = node(5);
    let mut tracks = BTreeMap::new();
    tracks.insert(quiet, track(vec![effect(&[("output_gain_db", 0.0)])]));
    tracks.insert(loud, track(vec![effect(&[("output_gain_db", -12.0)])]));
    let forced = project_inserts_with_forced_strips(&tracks, SR, &BTreeSet::from([quiet]));
    assert!(forced.contains_key(&quiet));
    assert!(forced.contains_key(&loud), "-12 dB 静态投影本来就非空");
    let untouched = project_inserts(&tracks, SR);
    assert!(!untouched.contains_key(&quiet));
    assert!(untouched.contains_key(&loud));
    // 空集 ⇒ 与 `project_inserts` 逐项相同。
    let empty = project_inserts_with_forced_strips(&tracks, SR, &BTreeSet::new());
    assert_eq!(empty.len(), untouched.len());
    for (id, chain) in &untouched {
        assert_eq!(empty[id].is_empty(), chain.is_empty());
        assert_eq!(empty[id].is_forced_strip(), chain.is_forced_strip());
    }
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#303`：压缩级接通（渲染侧判据）
// ---------------------------------------------------------------------------

/// 与 [`render`] 相同，但把计划留下以读**动态级读数**（插入链器件自己的计数器）。
fn render_with_readings(
    routing: &RoutingGraph,
    master: EntityId,
    inserts: &BTreeMap<EntityId, InsertChain>,
    frames: usize,
    mono: Vec<f32>,
    source_node: EntityId,
) -> (RenderOutput, Vec<InsertMeterReading>, u32) {
    let options = RenderOptions::l1(frames as u64, 2, SR, 0x155).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        routing,
        master,
        options,
        &BTreeMap::new(),
        inserts,
    )
    .expect("编译");
    let longest_path = plan.longest_path_frames();
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(source_node, Box::new(BufferSource::new(mono)));
    let output = plan.execute(sources).expect("渲染");
    (output, plan.insert_meter_readings(), longest_path)
}

/// ⭐ `#304`：一条轨 + 一台设备 +（可选）一条 `DeviceParam` 泳道的**模型工程**。
///
/// `points` 为空 ⇒ 不插泳道（正对照）。`param_index` 是设备 `params` 里的下标。
fn project_with_device_automation(
    devices: Vec<DeviceDefinition>,
    param_index: usize,
    points: &[(u64, f32)],
) -> YebanProjectV1 {
    let track_id = node(1);
    let mut track = track(devices);
    track.id = track_id;
    if !points.is_empty() {
        let target = AutomationTarget::DeviceParam {
            track_id,
            slot_index: 0,
            param_index,
        };
        let mut lane_points = BTreeMap::new();
        for (tick, value) in points {
            let point = AutomationPoint {
                id: EntityId::new(),
                tick: *tick,
                value: *value,
                curve: CurveType::Linear,
            };
            lane_points.insert(point.id, point);
        }
        track.automation_lanes.insert(
            target,
            AutomationLane {
                target,
                points: lane_points,
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
    }
    let mut project = YebanProjectV1 {
        master_bus_track_id: node(0xFF),
        ..YebanProjectV1::default()
    };
    project.tracks.insert(track_id, track);
    project
}

/// ⭐ `#304`：编译"带设备参数自动化"的计划并执行，返回母带与通道条读数。
///
/// 与生产两条导出同一条装配：`device_param_automations` → `forced_insert_instances`
/// → `project_inserts_with_forced_instances` → `attach_device_param_automation`
/// （⛔ 不在判据里复制"哪些名字"的判定）。
fn render_with_device_automations(
    routing: &RoutingGraph,
    master: EntityId,
    project: &YebanProjectV1,
    frames: usize,
    mono: Vec<f32>,
    source_node: EntityId,
) -> (RenderOutput, Vec<InsertMeterReading>) {
    let automations = device_param_automations(project, SR);
    let forced = forced_insert_instances(&automations);
    let inserts = project_inserts_with_forced_instances(&project.tracks, SR, &forced);
    let options = RenderOptions::l1(frames as u64, 2, SR, 0x155).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        routing,
        master,
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("编译");
    for automation in automations.into_values().flatten() {
        assert!(
            plan.attach_device_param_automation(automation),
            "非恒等设备参数泳道必须有运行态可改"
        );
    }
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(source_node, Box::new(BufferSource::new(mono)));
    let output = plan.execute(sources).expect("渲染");
    (output, plan.insert_meter_readings())
}

/// 一条轨的通道条读数（没有运行态 ⇒ 判据自己的前置条件失败）。
fn strip_reading(readings: &[InsertMeterReading], node: EntityId) -> &InsertMeterReading {
    readings
        .iter()
        .find(|reading| reading.node == node)
        .expect("本轨必须有通道条读数")
}

/// 一条压缩器的参数：**硬膝**（`knee_db: 0`）+ 短起振/释放，读数可手算对账。
///
/// 静态曲线（`knee = 0`）：`GR = (1 − 1/R)·(x − T)`，`x` 是检波器看到的 RMS dBFS。
fn hard_knee_compressor(threshold_db: f32) -> DeviceDefinition {
    effect(&[
        ("threshold_db", threshold_db),
        ("ratio", 4.0),
        ("knee_db", 0.0),
        ("detector_s", 0.005),
        ("attack_s", 0.002),
        ("release_s", 0.050),
        ("makeup_db", 0.0),
    ])
}

/// 判据 1 / 2（`#303` 的核心）：**阈值以上真的被压、阈值以下一个字节都不动**。
///
/// 读数（全部可手算）：
/// - 高幅正弦 `A = 0.5` ⇒ 检波器读到真 RMS `20·log10(0.5/√2) = −9.03 dBFS`；
///   `T = −24 dBFS`、`R = 4` ⇒ 稳态 `GR = 0.75·(−9.03 + 24) = 11.23 dB`；
/// - 低幅正弦 `A = 0.02` ⇒ RMS `−36.99 dBFS < T` ⇒ `GR = 0` ⇒ 与无设备基线**逐字节相同**。
///
/// 这一条同时是"复现 / 修后"的机械判据：修前 `threshold_db` 被 `effective_strip` 无条件
/// 关掉 ⇒ 第一档读数会等于基线（0 dB）。
#[test]
fn the_compressor_reduces_above_threshold_and_is_transparent_below_it() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000; // 0.5 s：起振/释放都走完，稳态窗口从 0.25 s 起
    let steady = (frames / 2)..frames;

    // ⚠ 用**只有淡入**的稳态正弦（`burst` 的 5 ms 淡出会让渲染末尾的检波器读数
    // 掉下去，那是"读数取样点"的问题，不是器件问题）。`detector_level_db` 是
    // `execute` 之后读的快照 ⇒ 末尾必须仍在稳态。
    let fade = (SR as usize / 200).max(1);
    let loud: Vec<f32> = (0..frames)
        .map(|index| {
            let envelope = if index < fade {
                index as f32 / fade as f32
            } else {
                1.0
            };
            let phase = core::f64::consts::TAU * 440.0 * index as f64 / f64::from(SR);
            (0.5f64 * f64::from(envelope) * phase.sin()) as f32
        })
        .collect();
    let baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        loud.clone(),
        track_node,
    );
    let plain_rms = rms(&baseline.samples, steady.start, steady.end);

    let mut tracks = BTreeMap::new();
    tracks.insert(track_node, track(vec![hard_knee_compressor(-24.0)]));
    let inserts = project_inserts(&tracks, SR);
    let chain = inserts.get(&track_node).expect("压缩参数必须投影出非空链");
    assert!(chain.compresses_statically(), "`threshold_db` ⇒ 压缩级执行");
    let (compressed, readings, longest_path) =
        render_with_readings(&routing, master, &inserts, frames, loud, track_node);

    let compressed_rms = rms(&compressed.samples, steady.start, steady.end);
    let gain_db = 20.0 * (compressed_rms / plain_rms).log10();
    let read = readings
        .iter()
        .find(|reading| reading.node == track_node)
        .expect("本轨必须有通道条读数");
    println!(
        "压缩读数: 基线 RMS = {plain_rms:.6} / 压缩后 = {compressed_rms:.6} ⇒ {gain_db:+.3} dB；\
         器件读数 maxReduction = {:.3} dB / currentReduction = {:.3} dB / reductions = {} / \
         stripFrames = {} / detector = {:.3} dBFS / longestPath = {longest_path}",
        read.max_reduction_db,
        read.current_reduction_db,
        read.gain_reductions,
        read.strip_frames,
        read.detector_level_db
    );
    // 手算 11.23 dB；检波器纹波与弹道残差都在 ±1.0 dB 内（窗口 0.25 s 之后）。
    assert!(
        (gain_db - (-11.23)).abs() < 1.0,
        "稳态增益衰减必须是 −11.23±1.0 dB（手算 (1−1/4)·(−9.03+24)），实得 {gain_db:+.3} dB"
    );
    assert!(
        read.max_reduction_db > 10.0,
        "器件自己的最大衰减读数必须 > 10 dB，实得 {}",
        read.max_reduction_db
    );
    assert!(
        read.gain_reductions > 0 && read.gain_reductions <= read.strip_frames,
        "被压帧数必须在 (0, stripFrames] 内，实得 {}/{}",
        read.gain_reductions,
        read.strip_frames
    );
    assert!(
        read.gain_reductions >= read.strip_frames - 1_000,
        "稳态正弦（淡入 240 帧）⇒ 至少 95.8% 的帧被压，实得 {}/{}",
        read.gain_reductions,
        read.strip_frames
    );
    assert!(
        (read.detector_level_db - (-9.03)).abs() < 1.0,
        "检波器电平必须是 −9.03±1.0 dBFS（声相之前的真 RMS），实得 {}",
        read.detector_level_db
    );

    // ---- 阈值以下：一个字节都不动（压缩增益恒 1.0，乘 1.0 是 IEEE 恒等）----
    let quiet = burst(440.0, frames, frames, 0.02);
    let quiet_baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        quiet.clone(),
        track_node,
    );
    let (quiet_compressed, quiet_readings, _) =
        render_with_readings(&routing, master, &inserts, frames, quiet, track_node);
    let quiet_read = quiet_readings
        .iter()
        .find(|reading| reading.node == track_node)
        .expect("本轨必须有通道条读数");
    println!(
        "阈值以下读数: digest 基线 = {} / 压缩后 = {}；maxReduction = {:.6} dB / reductions = {}",
        hex(&quiet_baseline.digest),
        hex(&quiet_compressed.digest),
        quiet_read.max_reduction_db,
        quiet_read.gain_reductions
    );
    assert_eq!(
        quiet_compressed.digest, quiet_baseline.digest,
        "阈值以下必须与无压缩逐字节相同（母带）"
    );
    assert_eq!(
        quiet_read.max_reduction_db, 0.0,
        "阈值以下不得出现任何增益衰减"
    );
    assert_eq!(quiet_read.gain_reductions, 0, "阈值以下不得计一次压缩");
}

/// ⭐ `#304` 判据 2/3：`threshold_db` 泳道**真的改压缩量**（GR 读数随泳道变化），
/// 且"处处等于静态值"的泳道**逐字节等于**无泳道。
///
/// 手算（硬膝、`R = 4`、0.5 正弦 ⇒ 检波 RMS `−9.03 dBFS`）：
/// - 静态 `T = −30 dBFS` ⇒ 稳态 `GR = 0.75·(−9.03 + 30) = 15.73 dB`；
/// - 泳道末端 `T = −12 dBFS` ⇒ 稳态 `GR = 0.75·(−9.03 + 12) = 2.23 dB`。
#[test]
fn an_automated_compressor_threshold_changes_the_compression_amount_and_the_master() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000;
    let fade = (SR as usize / 200).max(1);
    let loud: Vec<f32> = (0..frames)
        .map(|index| {
            let envelope = if index < fade {
                index as f32 / fade as f32
            } else {
                1.0
            };
            let phase = core::f64::consts::TAU * 440.0 * index as f64 / f64::from(SR);
            (0.5f64 * f64::from(envelope) * phase.sin()) as f32
        })
        .collect();

    let base_project = project_with_device_automation(vec![hard_knee_compressor(-30.0)], 0, &[]);
    let ramp_project = project_with_device_automation(
        vec![hard_knee_compressor(-30.0)],
        0,
        &[(0, -30.0), (960, -12.0)],
    );
    let flat_project = project_with_device_automation(
        vec![hard_knee_compressor(-30.0)],
        0,
        &[(0, -30.0), (480, -30.0), (960, -30.0)],
    );

    let (base, base_readings) = render_with_device_automations(
        &routing,
        master,
        &base_project,
        frames,
        loud.clone(),
        track_node,
    );
    let (ramp, ramp_readings) = render_with_device_automations(
        &routing,
        master,
        &ramp_project,
        frames,
        loud.clone(),
        track_node,
    );
    let (flat, flat_readings) =
        render_with_device_automations(&routing, master, &flat_project, frames, loud, track_node);
    let base_read = strip_reading(&base_readings, track_node);
    let ramp_read = strip_reading(&ramp_readings, track_node);
    let flat_read = strip_reading(&flat_readings, track_node);
    println!(
        "### #304 threshold: base digest={} maxGR={:.3} curGR={:.3} / \
         ramp digest={} maxGR={:.3} curGR={:.3} / flat digest={} maxGR={:.3} curGR={:.3}",
        hex(&base.digest),
        base_read.max_reduction_db,
        base_read.current_reduction_db,
        hex(&ramp.digest),
        ramp_read.max_reduction_db,
        ramp_read.current_reduction_db,
        hex(&flat.digest),
        flat_read.max_reduction_db,
        flat_read.current_reduction_db
    );

    assert!(
        base_read.max_reduction_db > 15.0,
        "静态 −30 dB 阈值必须压 > 15 dB，实得 {}",
        base_read.max_reduction_db
    );
    assert!(
        ramp_read.current_reduction_db < 8.0,
        "泳道末端 −12 dB 阈值的稳态 GR 必须 < 8 dB，实得 {}",
        ramp_read.current_reduction_db
    );
    assert!(
        base_read.current_reduction_db - ramp_read.current_reduction_db > 5.0,
        "GR 必须随泳道变化（静态 {} / 泳道末端 {}）",
        base_read.current_reduction_db,
        ramp_read.current_reduction_db
    );
    assert_ne!(
        ramp.digest, base.digest,
        "阈值泳道必须真的改母带（静默无效会得到相同摘要）"
    );
    assert_eq!(
        flat.digest, base.digest,
        "处处等于静态值 ⇒ 必须逐字节等于无泳道"
    );
    assert_eq!(
        flat_read.max_reduction_db.to_bits(),
        base_read.max_reduction_db.to_bits(),
        "恒等泳道的 GR 读数也必须等于无泳道"
    );
    assert_eq!(
        flat_read.current_reduction_db.to_bits(),
        base_read.current_reduction_db.to_bits()
    );
}

/// ⭐ `#304` 边界判据（用户裁决：⛔ 只登记"报 applied 而听不出"）：
/// 显式 `compressor_enabled: 0` ＋ **只**自动化 `threshold_db` ⇒
/// 求值器**有落点**（`is_applicable()` 为真、曲线非恒等），但压缩级被静态旁通 ⇒
/// **GR 必须逐位为 0**，且母带逐字节等于无泳道。
///
/// 这与 `eq_enabled: 0` ＋ `eq_low_gain` 泳道同一条既有口径：参数确实送到了器件，
/// 只是该级不执行。判据把"听不出"变成读数，⛔ 不允许只报 applied。
#[test]
fn an_explicitly_disabled_compressor_is_applied_but_has_zero_gr() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000;
    let fade = (SR as usize / 200).max(1);
    let loud: Vec<f32> = (0..frames)
        .map(|index| {
            let envelope = if index < fade {
                index as f32 / fade as f32
            } else {
                1.0
            };
            let phase = core::f64::consts::TAU * 440.0 * index as f64 / f64::from(SR);
            (0.5f64 * f64::from(envelope) * phase.sin()) as f32
        })
        .collect();
    let disabled = || effect(&[("compressor_enabled", 0.0), ("threshold_db", -20.0)]);
    let base_project = project_with_device_automation(vec![disabled()], 1, &[]);
    let ramp_project =
        project_with_device_automation(vec![disabled()], 1, &[(0, -20.0), (960, -40.0)]);

    let automations = device_param_automations(&ramp_project, SR);
    let automation = automations
        .get(&track_node)
        .and_then(|lanes| lanes.first())
        .expect("门槛泳道必须被扫描到");
    assert!(automation.is_applicable(), "参数本身有落点（applied 语义）");
    assert!(!automation.is_identity(), "曲线必须是非恒等的");

    let (base, _) = render_with_device_automations(
        &routing,
        master,
        &base_project,
        frames,
        loud.clone(),
        track_node,
    );
    let (ramp, readings) =
        render_with_device_automations(&routing, master, &ramp_project, frames, loud, track_node);
    let read = strip_reading(&readings, track_node);
    println!(
        "### #304 disabled compressor + threshold lane: base digest={} / lane digest={} / \
         maxGR={:.6} curGR={:.6} reductions={}",
        hex(&base.digest),
        hex(&ramp.digest),
        read.max_reduction_db,
        read.current_reduction_db,
        read.gain_reductions
    );
    assert_eq!(
        ramp.digest, base.digest,
        "压缩级静态旁通 ⇒ 阈值泳道听不出（母带逐字节相同）"
    );
    assert_eq!(read.gain_reductions, 0, "旁通的压缩级不得计一次压缩");
    assert_eq!(
        read.max_reduction_db.to_bits(),
        0.0f32.to_bits(),
        "旁通的压缩级最大 GR 必须逐位为 0"
    );
    assert_eq!(
        read.current_reduction_db.abs().to_bits(),
        0.0f32.to_bits(),
        "旁通的压缩级当前 GR 必须为 0（器件读数是 -0.0，数值 0）"
    );
}

/// 判据 3（`#303` 裁决三）：**无 lookahead、零延迟** ⇒ 输出帧数不变、冲激不移位、
/// PDC 关键路径仍是 `0`。
///
/// 阈值取 `0 dBFS`（冲激 `0.5` 的 RMS 远低于它）⇒ 压缩器逐位透明，于是"逐字节相同"
/// 这一条同时是"没有延迟线"的判据：任何前视/延迟都会让冲激位置或总帧数变化。
#[test]
fn a_compressor_adds_no_latency_and_does_not_touch_pdc() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000;
    let impulse_at = 1_000usize;
    let mut mono = vec![0.0f32; frames];
    mono[impulse_at] = 0.5;

    let baseline = render(
        &routing,
        master,
        &BTreeMap::new(),
        frames,
        mono.clone(),
        track_node,
    );
    let mut tracks = BTreeMap::new();
    tracks.insert(
        track_node,
        track(vec![effect(&[
            ("threshold_db", 0.0),
            ("ratio", 4.0),
            ("knee_db", 0.0),
        ])]),
    );
    let inserts = project_inserts(&tracks, SR);
    let (compressed, readings, longest_path) =
        render_with_readings(&routing, master, &inserts, frames, mono, track_node);

    // 器件自己的契约（`yeban_dsp` 是**同一个**器件；这里读的是公开 API）。
    let strip = yeban_dsp::channel_strip::ChannelStrip::new(
        yeban_dsp::channel_strip::ChannelStripParams::DEFAULT,
        SR as f32,
    );
    println!(
        "零延迟读数: frames 基线 = {} / 压缩后 = {}（请求 {frames}）；digest 基线 = {} / 压缩后 = {}；\
         ChannelStrip::latency_samples = {}；longestPath = {longest_path}；reductions = {}",
        baseline.frames,
        compressed.frames,
        hex(&baseline.digest),
        hex(&compressed.digest),
        strip.latency_samples(),
        readings
            .iter()
            .find(|reading| reading.node == track_node)
            .map_or(0, |reading| reading.gain_reductions)
    );
    assert_eq!(strip.latency_samples(), 0, "通道条/压缩器的延迟契约是 0");
    assert_eq!(
        compressed.frames, frames as u64,
        "零延迟 ⇒ 输出帧数必须等于请求帧数"
    );
    assert_eq!(
        compressed.frames, baseline.frames,
        "零延迟 ⇒ 有/无压缩的输出帧数必须相同"
    );
    assert_eq!(
        compressed.digest, baseline.digest,
        "阈上不触发时压缩器必须逐位透明（任何延迟都会破坏这一条）"
    );
    assert_eq!(longest_path, 0, "压缩器不进 PDC 表 ⇒ L_max 仍是 0");
}

/// 正对照（`#303` 判据 4）：**没有 `threshold_db` 的工程**逐字节等于改前。
///
/// `NO_COMPRESSOR_MASTER_DIGEST` 在 `#303` 改动**之前**（`main` @ `164af592`）与**之后**
/// 各用同一份夹具实测：两次**逐字节相同**（`11f47f5b…`）。`#303` 只改"会压缩的链"，
/// 这条必须一字不变。
/// 夹具刻意用增益级（唯一的超越函数是 `db_to_linear` → `libm`，跨平台可复现）。
const NO_COMPRESSOR_MASTER_DIGEST: &str =
    "11f47f5b2abf50437868a7064403f799f7b9489a40ccf199caf75dde66263625";

#[test]
fn a_project_without_compressor_params_keeps_its_master_bytes() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let frames = 24_000;
    let mono = burst(440.0, frames, frames, 0.3);

    let mut tracks = BTreeMap::new();
    tracks.insert(track_node, track(vec![effect(&[("input_gain_db", -6.0)])]));
    let inserts = project_inserts(&tracks, SR);
    assert!(!inserts.is_empty(), "增益级必须投影出非空链");
    assert!(
        inserts.values().all(|chain| !chain.compresses_statically()),
        "没有压缩参数 ⇒ 任何链都不得执行压缩级"
    );
    assert!(
        statically_compressing_nodes(&tracks).is_empty(),
        "预扫描也必须判成'没有会压缩的轨'"
    );
    let rendered = render(&routing, master, &inserts, frames, mono, track_node);
    println!(
        "正对照读数（无 threshold_db）: digest = {}",
        hex(&rendered.digest)
    );
    assert_eq!(
        hex(&rendered.digest),
        NO_COMPRESSOR_MASTER_DIGEST,
        "没有压缩参数的工程必须逐字节等于 #303 之前的母带"
    );
}

/// 机制判据：pre-pan tap 的**静态预扫描**（[`statically_compressing_nodes`]）与编译出来的链同解。
///
/// 两个导出（app / MCP）在构造音源之前只拿得到静态预扫描 —— 它与
/// [`InsertChain::compresses_statically`] 漂移的话，"声相搬家"和"真的压缩"就会落在不同的轨上。
/// ⚠ 它只钉**静态那一半**；完整生产判据是 [`compressing_nodes_with_lanes`]（`#309` 起）。
#[test]
fn statically_compressing_nodes_agree_with_the_compiled_chains() {
    let compressor = node(2);
    let gain_only = node(3);
    let eq_only = node(4);
    let explicit_off = node(5);
    let threshold_alias = node(6);
    let mut tracks = BTreeMap::new();
    tracks.insert(compressor, track(vec![hard_knee_compressor(-18.0)]));
    tracks.insert(gain_only, track(vec![effect(&[("input_gain_db", -3.0)])]));
    tracks.insert(eq_only, track(vec![effect(&[("eq_low_gain", 6.0)])]));
    tracks.insert(
        explicit_off,
        track(vec![effect(&[("compressor_enabled", 0.0)])]),
    );
    tracks.insert(
        threshold_alias,
        track(vec![effect(&[("threshold", -12.0)])]),
    );

    let inserts = project_inserts(&tracks, SR);
    let from_chains: BTreeSet<EntityId> = inserts
        .iter()
        .filter(|(_, chain)| chain.compresses_statically())
        .map(|(&id, _)| id)
        .collect();
    let pre_scan = statically_compressing_nodes(&tracks);
    println!("预扫描读数: statically_compressing_nodes = {pre_scan:?}；来自链 = {from_chains:?}");
    assert_eq!(pre_scan, from_chains, "预扫描与编译出来的链必须同解");
    assert_eq!(
        pre_scan,
        BTreeSet::from([compressor, threshold_alias]),
        "只有真的写了压缩参数的轨会压缩（显式 0 的开关不算）"
    );
}

/// ⭐ `#309` 机制判据：pre-pan tap 的**并集**判据必须**精度匹配**。
///
/// 生产判据 = [`compressing_nodes_with_lanes`]（静态投影 ∪ 非恒等
/// `compressor_enabled` 泳道）。本判据钉的是**粒度的结果**，⛔ 不是"有泳道就搬"：
///
/// 1. 静态关 ＋ 非恒等 `compressor_enabled` 泳道（`0 → 1`）⇒ 静态预扫描**仍空**、
///    并集**含该轨**（`#308` 的缺陷面 / `#309` 的修法）；
/// 2. **平泳道**（`0 → 0`）⇒ identity ⇒ 并集仍空；
/// 3. 只自动化 `threshold_db`（静态关）⇒ 压缩级不执行（`#304` 边界 1）⇒ 并集仍空
///    —— ⚠ 这一条正是 [`forced_insert_instances`] 的**反例**：它按"任一非恒等 strip
///    泳道"置位，会把这条 EQ/增益粒度的轨也搬进输出级；
/// 4. `read_enabled: false` 的非恒等泳道 ⇒ 并集仍空；
/// 5. 静态开（无泳道）⇒ 并集仍含该轨（静态那一半没被并集弄丢）。
#[test]
fn compressing_nodes_with_lanes_counts_only_a_nonidentity_compressor_enabled_lane() {
    let devices = || {
        vec![effect(&[
            ("compressor_enabled", 0.0),
            ("threshold_db", -12.0),
        ])]
    };
    let track_id = node(1);
    let union = |project: &YebanProjectV1| {
        let automations = device_param_automations(project, SR);
        let nodes = compressing_nodes_with_lanes(&project.tracks, &automations);
        let forced = forced_insert_instances(&automations);
        (nodes, forced)
    };

    // 0. 静态关、无泳道：静态与并集都空。
    let plain = project_with_device_automation(devices(), 0, &[]);
    let (nodes, forced) = union(&plain);
    assert!(forced.strip.is_empty());
    assert!(nodes.is_empty(), "没有泳道 ⇒ 不压缩：{nodes:?}");

    // 1. 静态关 ＋ `compressor_enabled` 的 0→1 泳道：静态空、并集含该轨。
    let ramp = project_with_device_automation(devices(), 0, &[(0, 0.0), (1920, 1.0)]);
    let automations = device_param_automations(&ramp, SR);
    assert!(
        statically_compressing_nodes(&ramp.tracks).is_empty(),
        "静态预扫描必须仍空（这是 #308 的缺陷面）"
    );
    assert_eq!(
        compressing_nodes_with_lanes(&ramp.tracks, &automations),
        BTreeSet::from([track_id]),
        "非恒等 `compressor_enabled` 泳道 ⇒ 该轨会压缩 ⇒ 并集必须含它"
    );

    // 2. 平泳道（0→0）＝ identity ⇒ 不置位。
    let flat = project_with_device_automation(devices(), 0, &[(0, 0.0), (1920, 0.0)]);
    let (nodes, _) = union(&flat);
    assert!(nodes.is_empty(), "平泳道是恒等 ⇒ 不得搬声相：{nodes:?}");

    // 3. 只自动化 `threshold_db`（静态关）⇒ 压缩级不执行 ⇒ 不置位；
    //    ⚠ 同一条轨在 `forced_insert_instances` 里**会**被置位（粗粒度的反例）。
    let threshold_only = project_with_device_automation(devices(), 1, &[(0, -24.0), (1920, -6.0)]);
    let (nodes, forced) = union(&threshold_only);
    assert!(
        forced.strip.contains(&track_id),
        "对照：粗粒度扫描器会把这条轨置位（所以 ⛔ 不能拿它当 pre-pan 判据）"
    );
    assert!(
        nodes.is_empty(),
        "静态关时 `threshold_db` 泳道听不出 ⇒ 不得搬声相：{nodes:?}"
    );

    // 4. `read_enabled: false` 的非恒等泳道 ⇒ 整段忽略 ⇒ 不置位。
    let target = AutomationTarget::DeviceParam {
        track_id,
        slot_index: 0,
        param_index: 0,
    };
    let mut disabled = project_with_device_automation(devices(), 0, &[(0, 0.0), (1920, 1.0)]);
    disabled
        .tracks
        .get_mut(&track_id)
        .expect("轨必须存在")
        .automation_lanes
        .get_mut(&target)
        .expect("泳道必须存在")
        .read_enabled = false;
    let (nodes, _) = union(&disabled);
    assert!(
        nodes.is_empty(),
        "read_enabled:false ⇒ 不得搬声相：{nodes:?}"
    );

    // 5. 静态开（无泳道）⇒ 静态那一半仍然在并集里。
    let static_on =
        project_with_device_automation(vec![effect(&[("compressor_enabled", 1.0)])], 0, &[]);
    let (nodes, _) = union(&static_on);
    assert_eq!(
        nodes,
        BTreeSet::from([track_id]),
        "静态开 ⇒ 并集必须含它（静态那一半没丢）"
    );
}

/// 判据（`#310`）：`pub fn` 名字以 `compress` 开头时，**必须**说清它看的是哪一半 ——
/// 静态投影含 `statically`，并集/泳道含 `_with_lanes`。
///
/// 为什么需要它：`compresses()` / `compressing_nodes()` 的名字读起来像"会压缩"，而
/// 实现只看**静态** `compressor_enabled` ⇒ 静态关 ＋ `0 → 1` 泳道时它们判 `false`，
/// 未来调用者拿它做 pre-pan 判据就会**退回 `#308`**。`#309` 的机制判据只钉并集、
/// **不钉"未来调用者"**；名字是编译器唯一看得见的防线，所以把这条命名规则变成
/// 源码扫描判据（先例：`quantifier_lower_bounds.rs:760` 读 `src/`）。
///
/// ⚠ 本判据扫的是**真实源文件**，⛔ 不是一份抄写的名单；新增静态语义的压缩函数
/// 若不带 `statically` 会当场红。
#[test]
fn pub_functions_about_compression_say_whether_they_mean_statically() {
    /// 规则本体：正/反对照都走这一个函数（⛔ 两边各写一套）。
    fn name_says_which_half(name: &str) -> bool {
        !name.starts_with("compress") || name.contains("statically") || name.contains("_with_lanes")
    }

    // 正对照：本单之后真实存在的两个 `compress` 前缀名字都过。
    assert!(name_says_which_half("compresses_statically"));
    assert!(name_says_which_half("compressing_nodes_with_lanes"));
    // 反对照：改名前的旧名字必须被判红（否则本判据是惰性的）。
    assert!(!name_says_which_half("compresses"));
    assert!(!name_says_which_half("compressing_nodes"));

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0usize;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in std::fs::read_dir(&root).expect("读 src/") {
        let path = entry.expect("目录项").path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("读源文件");
        scanned += 1;
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            let rest = trimmed
                .strip_prefix("pub fn ")
                .or_else(|| trimmed.strip_prefix("pub const fn "))
                .or_else(|| trimmed.strip_prefix("pub(crate) fn "));
            let Some(rest) = rest else { continue };
            let name = rest
                .split(|c: char| c == '(' || c == '<' || c.is_whitespace())
                .next()
                .expect("函数名");
            if !name.starts_with("compress") {
                continue;
            }
            seen.insert(name.to_owned());
            assert!(
                name_says_which_half(name),
                "{}:{}: `{name}` 的名字没说清是静态还是并集 ⇒ 静态语义必须含 `statically`、\
                 并集必须含 `_with_lanes`（#310；否则未来调用者会退回 #308）",
                path.display(),
                index + 1
            );
        }
    }
    assert!(
        scanned >= 7,
        "至少扫到 7 个源文件（与 `quantifier_lower_bounds.rs` 同一条下界），实际 {scanned}"
    );
    assert!(
        seen.contains("compresses_statically") && seen.contains("compressing_nodes_with_lanes"),
        "本判据必须真的扫到 `compresses_statically` 与 `compressing_nodes_with_lanes`：{seen:?}"
    );
}
