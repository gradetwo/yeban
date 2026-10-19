//! **离线插入链**的判据：EQ（3 段）＋ Freeverb ＋ 卷积混响。
//!
//! 这些判据量的是**导出母带的样本**，不是"投影出来的参数"：每条都给出两档读数
//! （有器件 / 无器件基线），因此"接线了"与"没接线"在读数上是可区分的两件事。
//!
//! 顺序差异（实时 = 单声道插入 → 声相；离线 = 声相 → 求和 → 立体声插入）登记在
//! `src/inserts.rs` 的模块头 §2。本文件的 EQ 判据用**逐声道线性等价**的那条
//! （纯正弦的基频带能量），混响/卷积判据量的是**尾音**，不声称与实时侧逐位相同。

use std::collections::BTreeMap;

use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind,
    TrackV3,
};
use yeban_render::inserts::{InsertChain, project_inserts, track_has_unhandled_dsp};
use yeban_render::render::{
    AudioSource, BlockContext, RenderError, RenderOptions, RenderOutput, RenderPlan,
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

/// 判据：`track_has_unhandled_dsp` 的口径（MCP 的 `deviceChainDsp` 登记依据）。
#[test]
fn unhandled_device_classification_matches_the_slice_boundary() {
    let eq_track = track(vec![effect(&[("eq_low_gain", 6.0)])]);
    assert!(!track_has_unhandled_dsp(&eq_track), "EQ 已被消费");

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
        "内置乐器是音源，不是插入链来源"
    );

    let compressor = track(vec![effect(&[("threshold_db", -12.0), ("ratio", 4.0)])]);
    assert!(
        track_has_unhandled_dsp(&compressor),
        "压缩留 S2 ⇒ 必须仍登记"
    );
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
