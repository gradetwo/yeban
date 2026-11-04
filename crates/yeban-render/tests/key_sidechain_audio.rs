//! 侧链键输入的**音频级**判据：键真的改变了母带的样本，而不只是"计划里接上了"。
//!
//! `render.rs` 的单元判据只钉住**接线**（`key_input` / `key_tap` / `key_routes`），
//! 说不出"接上之后有没有声音上的后果"。本文件补的就是这一段：用一条**只看键**的
//! 通路（kick 的音频边压到 −40 dB，听不见；但它照样被抽头成键）去驱动 bus 上的
//! 压缩器，然后量母带。
//!
//! 图（层号 0 / 1 / 2）：
//!
//! ```text
//!   kick ──(-40 dB)──► master          ← 只为把 kick 留在保留子图里并出键
//!   pad  ────────────► bus  ─────────► master
//!   kick ──Sidechain─► bus             ← 键：kick 的链后抽头
//! ```
//!
//! bus 的节目是**稳态 1 kHz 正弦**（幅度 0.1），kick 是 60 Hz 爆发（幅度 0.9）。
//! 于是"有键 / 无键"两次渲染的差值**只**能来自 bus 压缩器的检波器换了输入。

use std::collections::{BTreeMap, BTreeSet};

use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind,
};
use yeban_render::inserts::InsertChain;
use yeban_render::render::{AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan};

const SR: u32 = 48_000;

fn ulid(index: u32) -> EntityId {
    let mut text = [b'0'; 26];
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    for position in 0..4 {
        text[25 - position] = ALPHABET[((index >> (5 * position)) & 0x1F) as usize];
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

/// bus 上的通道条：阈值 −30 dB、8:1、硬膝、检波 10 ms、起振 1 ms、释放 100 ms。
fn strip_device(compressor_enabled: f32) -> DeviceDefinition {
    let params = [
        ("compressor_enabled", compressor_enabled),
        ("threshold_db", -30.0),
        ("ratio", 8.0),
        ("knee_db", 0.0),
        ("detector_s", 0.01),
        ("attack_s", 0.001),
        ("release_s", 0.1),
        ("makeup_db", 0.0),
    ];
    DeviceDefinition {
        id: EntityId::new(),
        name: "CS · bus".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| param(name, *value))
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

fn graph(sidechain_gain_db: Option<f32>) -> RoutingGraph {
    let master = ulid(0xFFFF);
    let kick = ulid(1);
    let bus = ulid(2);
    let pad = ulid(3);
    let mut edges = BTreeMap::new();
    let mut add = |source_node: EntityId,
                   destination_node: EntityId,
                   kind: RoutingKind,
                   gain_db: Option<f32>| {
        let id = EntityId::new();
        edges.insert(
            id,
            RoutingEdge {
                id,
                source_node,
                destination_node,
                kind,
                gain_db,
            },
        );
    };
    // kick 只用来出键：到 master 的那条音频边压到 −40 dB，否则它自己会盖住 bus 的读数。
    add(kick, master, RoutingKind::TrackToBus, Some(-40.0));
    add(pad, bus, RoutingKind::TrackToBus, None);
    add(bus, master, RoutingKind::TrackToBus, None);
    if let Some(gain_db) = sidechain_gain_db {
        add(kick, bus, RoutingKind::Sidechain, Some(gain_db));
    }
    RoutingGraph {
        nodes: vec![master, kick, pad, bus],
        edges,
    }
}

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

/// kick：每 12000 帧一次 41 ms 的 60 Hz 爆发（幅度 0.9），其余静音。
fn kick_buffer(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            let phase = index % 12_000;
            if phase >= 2_000 {
                return 0.0;
            }
            let t = 2.0 * core::f64::consts::PI * 60.0 * phase as f64 / f64::from(SR);
            (0.9 * t.sin()) as f32
        })
        .collect()
}

/// bus 自己的节目：稳态 1 kHz 正弦，幅度 0.1。
fn program_buffer(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            let t = 2.0 * core::f64::consts::PI * 1000.0 * index as f64 / f64::from(SR);
            (0.1 * t.sin()) as f32
        })
        .collect()
}

fn render(sidechain_gain_db: Option<f32>, compressor_enabled: f32, frames: usize) -> Vec<f32> {
    let master = ulid(0xFFFF);
    let kick = ulid(1);
    let bus = ulid(2);
    let pad = ulid(3);
    let routing = graph(sidechain_gain_db);
    let mut inserts = BTreeMap::new();
    inserts.insert(
        bus,
        InsertChain::from_devices(&[strip_device(compressor_enabled)], SR, 120.0),
    );
    let options = RenderOptions::l1(frames as u64, 2, SR, 0x155).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &routing,
        master,
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("编译");
    let armed: BTreeSet<EntityId> = [bus].into_iter().collect();
    plan.attach_key_armed_nodes(&armed);
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(kick, Box::new(BufferSource::new(kick_buffer(frames))));
    sources.insert(pad, Box::new(BufferSource::new(program_buffer(frames))));
    plan.execute(sources).expect("渲染").samples
}

fn rms_db(samples: &[f32], start: usize, end: usize) -> f64 {
    let mut sum = 0.0f64;
    for frame in start..end {
        for channel in 0..2 {
            let value = f64::from(samples[frame * 2 + channel]);
            sum += value * value;
        }
    }
    10.0 * (sum / ((end - start) as f64 * 2.0)).log10()
}

/// 每次 kick 落点上"有键 − 无键"的母带差值（dB）。
///
/// 窗口取爆发后半段（1000..2000 帧）：检波 10 ms 的低通与 1 ms 起振都已稳定。
fn duck_per_hit(keyed: &[f32], plain: &[f32]) -> Vec<f64> {
    [0usize, 12_000, 24_000, 36_000]
        .iter()
        .map(|onset| {
            let start = onset + 1_000;
            let end = onset + 2_000;
            rms_db(keyed, start, end) - rms_db(plain, start, end)
        })
        .collect()
}

#[test]
fn the_key_reaches_the_compressor_through_the_whole_render_chain() {
    let frames = 48_000;
    let keyed = render(Some(0.0), 1.0, frames);
    let plain = render(None, 1.0, frames);
    let deltas = duck_per_hit(&keyed, &plain);
    println!("每次 kick 落点的差值（有键 − 无键）：{deltas:?} dB");
    let worst = deltas.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        worst < -6.0,
        "键必须真的把 bus 压下去：最浅的一次也要低 6 dB 以上，实测 {worst:.2} dB（{deltas:?}）"
    );
}

/// 边的 `gain_db` 必须真的缩放键：把键压到 −12 dB 之后，同一次 kick 的凹口
/// 必须**明显变浅**（键电平低了 12 dB ⇒ 8:1 下少压 10.5 dB，实测约 6–7 dB）。
///
/// 反方向（+12 dB 比 0 dB 更深）在 8:1 + 本夹具的节目电平下会撞到增益下限，
/// 只差 1.9 dB，区分度不足 ⇒ 只用"压键"这一侧钉。
#[test]
fn the_sidechain_edge_gain_scales_the_key() {
    let frames = 48_000;
    let unity = render(Some(0.0), 1.0, frames);
    let quiet = render(Some(-12.0), 1.0, frames);
    let plain = render(None, 1.0, frames);
    let unity_deltas = duck_per_hit(&unity, &plain);
    let quiet_deltas = duck_per_hit(&quiet, &plain);
    println!("0 dB 键：{unity_deltas:?} dB；−12 dB 键：{quiet_deltas:?} dB");
    for (index, (unity, quiet)) in unity_deltas.iter().zip(quiet_deltas.iter()).enumerate() {
        assert!(
            *quiet > *unity + 4.0,
            "第 {index} 次 kick：键压到 −12 dB 之后凹口必须浅 4 dB 以上（{quiet:.2} vs {unity:.2}）"
        );
    }
}

#[test]
fn without_the_compressor_the_key_changes_nothing_bit_for_bit() {
    let frames = 24_000;
    let keyed = render(Some(12.0), 0.0, frames);
    let plain = render(None, 0.0, frames);
    assert_eq!(keyed.len(), plain.len());
    for (index, (a, b)) in keyed.iter().zip(plain.iter()).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "压缩器关着时键必须一位都不改（第 {index} 个样本）"
        );
    }
}
