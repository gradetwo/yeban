//! 侧链键输入消费面（v1）的实时侧判据：
//!
//! * [`yeban_engine::routing::RoutingPlan::compile`] 的键路由规则（构造期、纯数据）：
//!   源确定在循环里跑链 ＋ 实体序 ＋ 首条合格边胜出 ＋ master 目的不受序限；
//! * 端到端：`YebanProjectV1` → `EngineSnapshot::from_project` → `process_quantum`，
//!   响 kick 压轻 pad（与无键对照的差 = pad × (1 − 增益)，kick 逐位抵消）；
//! * 惰性边对照：没有压缩器的目的上，侧链边一个字节都不改输出。
//!
//! 全部断言在**真实渲染路径**产出的样本上（`support` 模块刻意不给测试专用捷径）。

mod support;

use std::collections::BTreeMap;
use std::str::FromStr;

use support::{NoteSpec, render, two_track_project};
use yeban_engine::routing::RoutingPlan;
use yeban_engine::snapshot::TrackParams;
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind,
    TrackKind, TrackV3, YebanProjectV1,
};

/// 造一个 26 字符 ULID 文本：前 24 位为 `0`，末 2 位为序号 ⇒ 序号升序 = 实体序升序。
fn ulid(index: u32) -> EntityId {
    let text = format!("000000000000000000000000{index:02}");
    assert_eq!(text.len(), 26, "ULID 文本恒为 26 字符");
    EntityId::from_str(&text).expect("合法 ULID")
}

/// 一张效果器设备（`InternalEffect`）。
fn effect(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Strip".to_owned(),
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

/// 把设备链接到工程的某条轨上（**只改夹具**，不改模型层）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project.tracks.get_mut(&track).expect("夹具里必须有那条轨");
    entry.devices = devices;
}

/// 键目的上的压缩器：阈值 −18 dB、4:1、硬膝、检波 1 ms、起振 1 ms、释放 50 ms、无 makeup。
///
/// 阈值取 −18 dB 的理由：kick（velocity 127，峰值约 0 dBFS）在阈值之上 ⇒ 有键时压下去；
/// pad（velocity 10，峰值约 −22 dBFS）在阈值之下 ⇒ 无键对照里增益恒为 1.0（差 = pad × (1 − 增益)
/// 的恒等式才成立）。
const COMP: [(&str, f32); 8] = [
    ("compressor_enabled", 1.0),
    ("threshold_db", -18.0),
    ("ratio", 4.0),
    ("knee_db", 0.0),
    ("detector_s", 0.001),
    ("attack_s", 0.001),
    ("release_s", 0.05),
    ("makeup_db", 0.0),
];

/// 一条音轨的投影（`device_count` = 设备数；其余字段取模型默认值）。
fn track_params(id: EntityId, devices: usize) -> (EntityId, TrackParams) {
    let mut track = TrackV3 {
        id,
        name: "Track".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.devices = (0..devices).map(|_| effect(&[])).collect();
    (id, TrackParams::from_track(&track, 0))
}

/// 一张图的投影：节点 + 边（`id` 即键 ⇒ `BTreeMap` 迭代 = 边身份升序）。
fn plan_graph(
    nodes: Vec<EntityId>,
    edges: Vec<RoutingEdge>,
    master: EntityId,
    tracks: &BTreeMap<EntityId, TrackParams>,
) -> RoutingPlan {
    let graph = RoutingGraph {
        nodes,
        edges: edges.into_iter().map(|edge| (edge.id, edge)).collect(),
    };
    RoutingPlan::compile(&graph, master, tracks, &[])
}

fn audio_edge(id: EntityId, source: EntityId, destination: EntityId) -> RoutingEdge {
    RoutingEdge {
        id,
        source_node: source,
        destination_node: destination,
        kind: RoutingKind::TrackToBus,
        gain_db: None,
    }
}

fn sidechain_edge(id: EntityId, source: EntityId, destination: EntityId) -> RoutingEdge {
    RoutingEdge {
        id,
        source_node: source,
        destination_node: destination,
        kind: RoutingKind::Sidechain,
        gain_db: None,
    }
}

/// 判据：无入边、实体序在前的源 ⇒ 键路由成立；目的与源的槽位同一把尺子。
#[test]
fn key_route_compiled_for_cycle_phase_source() {
    let master = ulid(0);
    let kick = ulid(1);
    let pad = ulid(2);
    let tracks: BTreeMap<EntityId, TrackParams> = [track_params(kick, 0), track_params(pad, 1)]
        .into_iter()
        .collect();
    let plan = plan_graph(
        vec![kick, pad, master],
        vec![
            audio_edge(ulid(10), kick, master),
            audio_edge(ulid(11), pad, master),
            sidechain_edge(ulid(12), kick, pad),
        ],
        master,
        &tracks,
    );
    assert_eq!(plan.key_route(&pad), plan.slot_of(&kick));
    assert!(plan.key_route(&pad).is_some(), "kick→pad 必须求值");
    assert_eq!(plan.key_tap_slot(&kick), plan.slot_of(&kick));
    assert_eq!(plan.key_route(&kick), None);
    assert_eq!(plan.key_tap_slot(&pad), None);
}

/// 判据：源的实体序在目的之后 ⇒ 不求值（逐轨循环里目的先跑，抽头还没写）。
#[test]
fn key_route_rejected_when_source_after_destination() {
    let master = ulid(0);
    let kick = ulid(1);
    let pad = ulid(2);
    let tracks: BTreeMap<EntityId, TrackParams> = [track_params(kick, 0), track_params(pad, 0)]
        .into_iter()
        .collect();
    let plan = plan_graph(
        vec![kick, pad, master],
        vec![
            audio_edge(ulid(10), kick, master),
            audio_edge(ulid(11), pad, master),
            sidechain_edge(ulid(12), pad, kick),
        ],
        master,
        &tracks,
    );
    assert_eq!(plan.key_route(&kick), None);
    assert_eq!(plan.key_tap_slot(&pad), None);
}

/// 判据：有入边且有器件的总线源 ⇒ 不求值（它的链可能在转发阶段跑）；
/// 且不合格的边不占位 —— 同一目的的后一条合格边仍可生效。
#[test]
fn key_route_rejected_for_bus_source_with_incoming_and_devices() {
    let master = ulid(0);
    let kick = ulid(1);
    let bus = ulid(2);
    let pad = ulid(3);
    let tracks: BTreeMap<EntityId, TrackParams> = [
        track_params(kick, 0),
        track_params(bus, 1),
        track_params(pad, 1),
    ]
    .into_iter()
    .collect();
    let plan = plan_graph(
        vec![kick, bus, pad, master],
        vec![
            audio_edge(ulid(10), kick, bus),
            audio_edge(ulid(11), bus, master),
            audio_edge(ulid(12), pad, master),
            // 故意让不合格的 bus 边身份更小：它先被看到、被否决，不占位。
            sidechain_edge(ulid(13), bus, pad),
            sidechain_edge(ulid(14), kick, pad),
        ],
        master,
        &tracks,
    );
    assert_eq!(plan.key_route(&pad), plan.slot_of(&kick));
    assert_eq!(plan.key_tap_slot(&bus), None);
}

/// 判据：v1 单键 —— 同一目的的多条合格边中边身份升序第一条生效；
/// 生效顺序与边的构造顺序无关（只与边身份有关）。
#[test]
fn first_qualified_sidechain_edge_wins() {
    let master = ulid(0);
    let kick_a = ulid(1);
    let kick_b = ulid(2);
    let pad = ulid(3);
    let tracks: BTreeMap<EntityId, TrackParams> = [
        track_params(kick_a, 0),
        track_params(kick_b, 0),
        track_params(pad, 1),
    ]
    .into_iter()
    .collect();
    let plan = plan_graph(
        vec![kick_a, kick_b, pad, master],
        vec![
            audio_edge(ulid(10), kick_a, master),
            audio_edge(ulid(11), kick_b, master),
            audio_edge(ulid(12), pad, master),
            // 故意把大身份的边放在前面：BTreeMap 迭代仍按身份升序。
            sidechain_edge(ulid(14), kick_a, pad),
            sidechain_edge(ulid(13), kick_b, pad),
        ],
        master,
        &tracks,
    );
    assert_eq!(plan.key_route(&pad), plan.slot_of(&kick_b));
    assert_eq!(plan.key_tap_slot(&kick_a), None);
}

/// 判据：master 目的不受实体序限制（主总线链在全部循环与转发之后）；
/// master 源与未知目的则一律排除。
#[test]
fn key_route_to_master_ignores_order_but_master_source_and_unknown_dest_rejected() {
    let master = ulid(0);
    let pad = ulid(3);
    let ghost = ulid(9);
    let tracks: BTreeMap<EntityId, TrackParams> = [track_params(pad, 1)].into_iter().collect();
    let plan = plan_graph(
        vec![pad, master],
        vec![
            audio_edge(ulid(10), pad, master),
            // pad 的实体序在 master 之后，仍可作 master 的键源。
            sidechain_edge(ulid(11), pad, master),
            // master 作源：母带是求和终点，不进循环 ⇒ 排除。
            sidechain_edge(ulid(12), master, pad),
            // 目的不在轨表里 ⇒ 排除。
            sidechain_edge(ulid(13), pad, ghost),
        ],
        master,
        &tracks,
    );
    assert_eq!(plan.key_route(&master), plan.slot_of(&pad));
    assert_eq!(plan.key_route(&pad), None);
    assert_eq!(plan.key_tap_slot(&master), None);
}

/// 键夹具：响 kick（velocity 127）＋ 轻 pad（velocity 10）＋ 可选的 pad 压缩器 ＋
/// kick→pad 侧链边。角色按实体序定：小 id = kick（响），大 id = pad（轻）——
/// 若两轨的随机 id 与音符分配相反，就交换两轨的片段表（路由边与设备挂接不受影响）。
fn keyed_fixture(with_compressor: bool) -> (YebanProjectV1, EntityId, EntityId, EntityId) {
    let (mut project, first, second) = two_track_project(
        &[NoteSpec::at(0, 3840, 36, 127)],
        &[NoteSpec::at(0, 3840, 60, 10)],
    );
    if first > second {
        let mut first_track = project.tracks.remove(&first).expect("first");
        let mut second_track = project.tracks.remove(&second).expect("second");
        std::mem::swap(&mut first_track.clips, &mut second_track.clips);
        project.tracks.insert(first, first_track);
        project.tracks.insert(second, second_track);
    }
    let (kick, pad) = if first < second {
        (first, second)
    } else {
        (second, first)
    };
    if with_compressor {
        mount(&mut project, pad, vec![effect(&COMP)]);
    }
    let edge = EntityId::new();
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: kick,
            destination_node: pad,
            kind: RoutingKind::Sidechain,
            gain_db: None,
        },
    );
    (project, kick, pad, edge)
}

fn rms(samples: &[f32]) -> f64 {
    (samples
        .iter()
        .map(|sample| (*sample as f64).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

/// 判据（端到端）：pad 上的压缩器被 kick 的键信号压下去。
///
/// 隔离设计：有键与无键的唯一差别就是侧链边 ⇒ kick 样本逐位抵消，
/// `无键 − 有键` = pad × (1 − 增益)；pad 单独渲染给出分母（pad 原样）。
/// 窗口取 quanta 50..150（帧 6400..19200）：音符正响、压缩器已收敛，
/// 且避开了起振瞬态与可能的尾部衰减。
#[test]
fn keyed_pad_compressor_ducks_program_when_key_is_loud() {
    const QUANTA: usize = 400;
    let (keyed_project, kick, _pad, _edge) = keyed_fixture(true);
    let mut control_project = keyed_project.clone();
    control_project
        .routing_graph
        .edges
        .retain(|_, edge| edge.kind != RoutingKind::Sidechain);
    let mut pad_alone_project = keyed_project.clone();
    pad_alone_project
        .tracks
        .get_mut(&kick)
        .expect("kick")
        .clips
        .clear();
    let keyed = render(&keyed_project, QUANTA);
    let control = render(&control_project, QUANTA);
    let pad_alone = render(&pad_alone_project, QUANTA);
    let from = 50 * 128;
    let to = 150 * 128;
    let keyed_window = &keyed.left[from..to];
    let control_window = &control.left[from..to];
    let pad_window = &pad_alone.left[from..to];
    let pad_rms = rms(pad_window);
    assert!(pad_rms > 0.002, "pad 必须真的响：RMS = {pad_rms}");
    assert_ne!(
        keyed_window, control_window,
        "有键与无键在窗口里必须不同（否则键根本没接上）"
    );
    let diff: Vec<f32> = control_window
        .iter()
        .zip(keyed_window.iter())
        .map(|(control_sample, keyed_sample)| control_sample - keyed_sample)
        .collect();
    let diff_rms = rms(&diff);
    assert!(
        diff_rms > 0.35 * pad_rms,
        "键必须压下去：差 RMS = {diff_rms}，pad RMS = {pad_rms}（比值须 > 0.35）"
    );
}

/// 判据（惰性边对照）：没有压缩器的目的上，侧链边一个字节都不改输出。
#[test]
fn sidechain_edge_alone_changes_no_output_byte() {
    let (keyed_project, _kick, _pad, _edge) = keyed_fixture(false);
    let mut control_project = keyed_project.clone();
    control_project
        .routing_graph
        .edges
        .retain(|_, edge| edge.kind != RoutingKind::Sidechain);
    let keyed = render(&keyed_project, 400);
    let control = render(&control_project, 400);
    assert_eq!(
        keyed.fingerprint(),
        control.fingerprint(),
        "侧链边单独存在时输出必须逐位相同"
    );
    assert_eq!(keyed.left_bits(), control.left_bits());
}
