//! **生成器件离线 PDC 接线**的渲染级判据（ADR-0078 / T-DRUM-9）。
//!
//! 背景：鼓机生成器给每台生成器件写 `latency_samples = 0`，但总线软削波
//! （过采样级联）与鼓轨磁带饱和（过采样往返）在离线链里真执行、真延迟。
//! 审计（T-DRUM-9 第一阶段）实证这笔延迟此前不进 PDC 的账：生成工程的鼓声
//! 在离线渲染里比计划晚到。本文件的判据量的是**母带样本里的瞬态位置**：
//! 同一冲激分别走「带生成器件的鼓路径」与「零延迟参考轨」，两条路径的
//! 峰值位置必须对齐（≤ 1 帧；削波 4 倍档的真实群延迟 46.5 帧含半帧残余，
//! 在帧网格之下）。推导断掉时鼓路径会晚 31 / 47 帧，判据必红。
//!
//! 推导读数本身的判据在 `src/inserts.rs` 与 `src/render.rs` 的单元层；
//! 生成端到端读数在 `yeban-mcp/tests/drum_generate_tool.rs`。

use std::collections::BTreeMap;

use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind,
    TrackV3,
};
use yeban_render::inserts::project_inserts;
use yeban_render::render::{
    AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan, track_latencies,
};

const SR: u32 = 48_000;
const BPM: f64 = 120.0;
const FRAMES: usize = 4096;
const IMPULSE_AT: usize = 1000;

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

fn effect(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "FX".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
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
                source_lane: None,
                dest_lane: None,
            },
        );
    }
    RoutingGraph {
        nodes: nodes.to_vec(),
        edges: map,
    }
}

/// 把一条单声道缓冲复制到各声道的源（与 `insert_chain.rs` 同形）。
struct BufferSource {
    mono: Vec<f32>,
    position: usize,
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

fn impulse(amplitude: f32) -> Vec<f32> {
    let mut buffer = vec![0.0f32; FRAMES];
    buffer[IMPULSE_AT] = amplitude;
    buffer
}

fn silence() -> Vec<f32> {
    vec![0.0f32; FRAMES]
}

/// 用生产口径（`track_latencies` + `project_inserts`）渲染并返回母带左声道
/// 的峰值帧位置。`active` 是唯一发声的源节点，另一源喂静音。
fn peak_frame(
    routing: &RoutingGraph,
    master: EntityId,
    tracks: &BTreeMap<EntityId, TrackV3>,
    sources: &[EntityId],
    active: EntityId,
    amplitude: f32,
) -> usize {
    let latencies = track_latencies(tracks, SR);
    let inserts = project_inserts(tracks, SR, BPM);
    let options = RenderOptions::l1(FRAMES as u64, 2, SR, 0xD12).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        routing, master, options, &latencies, &inserts,
    )
    .expect("编译");
    let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    for &source in sources {
        let buffer = if source == active {
            impulse(amplitude)
        } else {
            silence()
        };
        registry.insert(
            source,
            Box::new(BufferSource {
                mono: buffer,
                position: 0,
            }),
        );
    }
    let output = plan.execute(registry).expect("渲染");
    let (mut best_frame, mut best_value) = (0usize, 0.0f32);
    for frame in 0..FRAMES {
        let value = output.samples[frame * 2].abs();
        if value > best_value {
            best_value = value;
            best_frame = frame;
        }
    }
    assert!(best_value > 0.0, "母带必须有声（峰值 {best_value}）");
    best_frame
}

/// 判据 ①（总线削波）：鼓路径 = 源轨 → 带 4 倍过采样软削波的鼓总线 → master。
/// 削波参数取恒等曲线（drive 0 / ceiling 0 / knee 0，冲激在膝下），母带里
/// 只剩过采样往返的延迟 ⇒ 峰值位置差就是未补偿量。
#[test]
fn generated_bus_clipper_transient_aligns_with_the_reference_track() {
    let drum = node(1);
    let bus = node(2);
    let reference = node(3);
    let master = node(4);
    let routing = graph(
        &[drum, bus, reference, master],
        &[
            (drum, bus, RoutingKind::TrackToBus),
            (bus, master, RoutingKind::BusToMaster),
            (reference, master, RoutingKind::TrackToBus),
        ],
    );
    let mut tracks = BTreeMap::new();
    tracks.insert(drum, track(vec![]));
    tracks.insert(
        bus,
        track(vec![effect(&[
            ("clipper_drive_db", 0.0),
            ("clipper_ceiling_db", 0.0),
            ("clipper_knee_db", 0.0),
            ("clipper_mix", 1.0),
            ("clipper_oversample", 4.0),
        ])]),
    );
    tracks.insert(reference, track(vec![]));
    tracks.insert(master, track(vec![]));

    let reference_peak = peak_frame(
        &routing,
        master,
        &tracks,
        &[drum, reference],
        reference,
        0.5,
    );
    let drum_peak = peak_frame(&routing, master, &tracks, &[drum, reference], drum, 0.5);
    assert!(
        drum_peak.abs_diff(reference_peak) <= 1,
        "鼓路径峰值 {drum_peak} 必须与参考轨峰值 {reference_peak} 对齐（≤ 1 帧）；\
         差 47 帧上下即总线削波的过采样延迟没有入账"
    );
}

/// 判据 ①（鼓轨磁带）：鼓轨自身带武装的磁带饱和（小冲激近线性段），
/// 与零延迟参考轨在求和点对齐。
#[test]
fn generated_tape_transient_aligns_with_the_reference_track() {
    let drum = node(11);
    let reference = node(12);
    let master = node(13);
    let routing = graph(
        &[drum, reference, master],
        &[
            (drum, master, RoutingKind::TrackToBus),
            (reference, master, RoutingKind::TrackToBus),
        ],
    );
    let mut tracks = BTreeMap::new();
    tracks.insert(
        drum,
        track(vec![effect(&[("tape_drive", 0.5), ("tape_mix", 1.0)])]),
    );
    tracks.insert(reference, track(vec![]));
    tracks.insert(master, track(vec![]));

    let reference_peak = peak_frame(
        &routing,
        master,
        &tracks,
        &[drum, reference],
        reference,
        0.01,
    );
    let drum_peak = peak_frame(&routing, master, &tracks, &[drum, reference], drum, 0.01);
    assert!(
        drum_peak.abs_diff(reference_peak) <= 1,
        "鼓轨峰值 {drum_peak} 必须与参考轨峰值 {reference_peak} 对齐（≤ 1 帧）；\
         差 31 帧上下即磁带的过采样往返没有入账"
    );
}
