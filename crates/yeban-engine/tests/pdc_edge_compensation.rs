//! ⭐ **stage 26** 判据：**逐边补偿**（一个节点两条边、各边下游求和点不同）。
//!
//! ## 判什么
//!
//! 工程：一条会发声的轨 `split` **同时**直连 `master`、以及送到一条自带 64 帧上报
//! 延迟的总线 `bus`（`bus → master`）。
//!
//! 计划：
//! * `arrival(split) = 0`、`arrival(bus) = 64`（总线自己的设备链延迟）；
//! * `L(bus) = 0`、`L(master) = max(0, 64) = 64`；
//! * 逐边：`D(split→bus) = 0`、`D(split→master) = 64` —— **两条边不同**；
//! * 节点式（旧口径）：`D(split) = max(0, 64) = 64` —— **两条边都被补 64**。
//!
//! 于是正确实现下，`split` 的直连分量与"经总线回来"的分量在 `master` 上**同相
//! 叠加**（`64` 的补偿正好抵掉总线自己的 `64`）；旧口径会把直连分量也多推 `64`
//! ⇒ 两个分量错开一整段，**只有一份**先到。
//!
//! ## 怎么量（不靠读数，靠波形）
//!
//! 同一份工程渲染两次：`bus` 的上报延迟 `64` / `0`。
//! * `64` 的那次：直连边必须被补 64 ⇒ 两条分量**同相**；
//! * `0` 的那次：两条边都是 0 ⇒ 同样**同相**。
//!
//! 两次的**逐帧波形必须逐位相同**（补偿把"多出来的 64"整段抵掉了）。旧口径
//! （节点 `max`）在 `64` 那次会把直连分量也推 64 ⇒ 两次波形在起点处相差一整段
//! ⇒ 判据红。反向腿：把逐边那一段关掉 ⇒ 红（注入实测见提交说明）。

mod support;

use std::collections::BTreeMap;

use support::render;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

const REPORTED_LATENCY: u32 = 64;
const NOTE_TICKS: u64 = 960;
const VELOCITY: u8 = 40;
const QUANTA: usize = 8;

#[derive(Clone, Copy)]
struct Ids {
    master: EntityId,
    split: EntityId,
    bus: EntityId,
}

fn ids() -> Ids {
    Ids {
        master: EntityId::new(),
        split: EntityId::new(),
        bus: EntityId::new(),
    }
}

fn note_track(id: EntityId, name: &str) -> (TrackV3, ClipPoolEntry) {
    let mut track = TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut note = MidiNote::new(note_id, 0, 60, NOTE_TICKS);
    note.velocity = VELOCITY;
    let mut note_map = BTreeMap::new();
    note_map.insert(note_id, note);
    let entry = ClipPoolEntry {
        id: clip,
        name: format!("{name} clip"),
        content: ClipContent::Midi { notes: note_map },
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    (track, entry)
}

/// `split → master` ＋ `split → bus(报告延迟) → master`。总线上的上报延迟由 `bus`
/// 自己的设备链给（⛔ 不是让 `split` 变慢：那样两条边就一样了）。
fn project(ids: Ids, bus_latency: u32) -> YebanProjectV1 {
    let (split, split_clip) = note_track(ids.split, "Split");
    let bus = TrackV3 {
        id: ids.bus,
        name: "Bus".to_owned(),
        kind: TrackKind::AuxReturn,
        devices: vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Reported".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: Vec::new(),
            latency_samples: bus_latency,
            plugin: None,
        }],
        ..TrackV3::default()
    };
    let master = TrackV3 {
        id: ids.master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };

    let mut tracks = BTreeMap::new();
    tracks.insert(ids.split, split);
    tracks.insert(ids.bus, bus);
    tracks.insert(ids.master, master);

    let mut clip_pool = BTreeMap::new();
    clip_pool.insert(split_clip.id, split_clip);

    let mut routing = RoutingGraph {
        nodes: vec![ids.split, ids.bus, ids.master],
        ..RoutingGraph::default()
    };
    for (source, destination, kind) in [
        (ids.split, ids.master, RoutingKind::TrackToBus),
        (ids.split, ids.bus, RoutingKind::TrackToBus),
        (ids.bus, ids.master, RoutingKind::TrackToBus),
    ] {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: source,
                destination_node: destination,
                kind,
                gain_db: None,
                source_lane: None,
                dest_lane: None,
            },
        );
    }

    YebanProjectV1 {
        master_bus_track_id: ids.master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

/// ⭐ **stage 26 判据**：两次渲染**只差一个整段 64 帧**（逐边补偿把"总线自己那 64"
/// 只算在它自己那条边上），而旧口径（节点 `max`）会让**直连分量与总线分量错开
/// 64 帧** ⇒ 波形不只是位移，而是"两份分量不再同相"。
#[test]
fn two_edges_with_different_downstream_points_are_aligned_per_edge() {
    let ids = ids();
    let with_latency = render(&project(ids, REPORTED_LATENCY), QUANTA);
    let without = render(&project(ids, 0), QUANTA);

    let onset = without
        .left
        .iter()
        .position(|sample| sample.abs() > f32::EPSILON)
        .expect("工程必须发声（否则判据空转）");
    let shifted_onset = with_latency
        .left
        .iter()
        .position(|sample| sample.abs() > f32::EPSILON)
        .expect("带 64 帧总线延迟的那次必须出声");

    // 1) 整体位移恰是 `REPORTED_LATENCY`（总线自己那 64 帧 ⇒ 整个工程后移 64）。
    assert_eq!(
        shifted_onset,
        onset + REPORTED_LATENCY as usize,
        "总线自己报 64 帧 ⇒ 整段输出必须恰好后移 64 帧（reference={onset}、shifted={shifted_onset}）"
    );

    // 2) 位移之后**逐位相同** —— 这就是"两条分量仍然同相"的机械证据。
    //    旧口径（节点 `max`）下直连分量也吃 64、总线分量再吃自己的 64 ⇒ 两条分量
    //    错开 64 帧 ⇒ 波形不是简单位移 ⇒ 这里必红。
    let delay = REPORTED_LATENCY as usize;
    for index in 0..(without.left.len() - delay) {
        assert_eq!(
            without.left[index].to_bits(),
            with_latency.left[index + delay].to_bits(),
            "第 {index} 帧：逐边补偿后只该整体后移 {delay} 帧（实得不同 ⇒ 两条分量错开了）"
        );
    }

    // 3) 反向腿：两份必须**真的不同**（否则"相等"可能来自同一个常量缓冲）。
    assert_ne!(
        without.left, with_latency.left,
        "两次渲染必须不同（证明 64 帧延迟确实施加了）"
    );
}
