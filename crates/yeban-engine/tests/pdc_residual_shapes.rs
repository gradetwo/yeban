//! ⭐ **逐边 PDC 同族残余两形状**判据（`line/pdc-residual-shapes`）。
//!
//! ## 背景：同一个不变量的另外两种破法
//!
//! stage 30 待查项的根因（见 `pdc_edge_compensation.rs` 文件头与
//! `DIAGNOSIS-pdc.md`）：**一条节点的补偿线在一个量子内被两个站点喂入**。
//! 第一形状（无入边的纯轨被总线转发段 3a⁗ 以零块二度喂线）已修：3a⁗ 跳过
//! 无入边节点。但同一不变量在两个形状下仍不成立：
//!
//! 1. **总线/混合节点且补偿延迟 `D > 128` 帧（一量子）**：其线在逐轨段先被喂
//!    自有静音/内容、3a⁗ 再喂收入之和。`D ≤ 128` 时两遍输出恰分区无重叠
//!    （纯总线变体实测成立）；`D > 128` 分区破裂 —— 本文件 S1 量它。
//! 2. **混合轨（自有片段 + 入边、无已武装插入器件）且任一出边 `D > 0`**：
//!    自有内容与收入之和是**两份真实流**，先后喂同一条线，输出是两流的交错
//!    延迟而非「和」的延迟 —— 本文件 S2（逐边档）与 S3（节点级档）量它。
//!
//! 修法（两形状同一条）：节点输出**先汇合、再单点喂线** —— 有入边且不是
//! 处理型总线的节点，其自有内容在逐轨段不再单独喂线/路由，而是并入自己的
//! 总线暂存槽；3a⁗ 转发时槽内已是「收入 + 自有」的完整输出，补偿线每量子
//! 只喂这一次。
//!
//! ## 守恒口径
//!
//! * `D = 0` 的线被喂两次时第二遍输出恒等于输入 ⇒ 旧行为与新行为逐位相同；
//! * 纯总线 `D ≤ 128` 的分区行为：自有内容恒为静音，汇合后的流与收入逐位
//!   相同（`x + 0.0 == x`）；
//! * E5 与 pdc 全套判据保持绿（全套验证见本线回执）。

mod support;

use std::collections::BTreeMap;

use support::render;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

const NOTE_TICKS: u64 = 960;
const VELOCITY: u8 = 40;
/// S2/S3 的力度：四份音符分量叠加后仍须留在母带限制器的透明区
/// （增益恒 1.0），「两份参考之和」这个 oracle 才成立。
const QUIET: u8 = 16;
const QUANTA: usize = 24;
/// 夹具栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 帧。
const SAMPLES_PER_TICK: u64 = 25;

fn note_track_v(
    id: EntityId,
    name: &str,
    start_tick: u64,
    pitch: u8,
    with_notes: bool,
    velocity: u8,
) -> (TrackV3, Option<ClipPoolEntry>) {
    let mut track = TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    if !with_notes {
        return (track, None);
    }
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut note = MidiNote::new(note_id, start_tick, pitch, NOTE_TICKS);
    note.velocity = velocity;
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
            duration_ticks: start_tick + NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    (track, Some(entry))
}

fn note_track(
    id: EntityId,
    name: &str,
    start_tick: u64,
    pitch: u8,
    with_notes: bool,
) -> (TrackV3, Option<ClipPoolEntry>) {
    note_track_v(id, name, start_tick, pitch, with_notes, VELOCITY)
}

fn aux_bus(id: EntityId, name: &str, latency: u32) -> TrackV3 {
    let devices = if latency == 0 && name != "Bus3" && name != "Bus" {
        Vec::new()
    } else {
        vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Reported".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: Vec::new(),
            latency_samples: latency,
            plugin: None,
        }]
    };
    TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::AuxReturn,
        devices,
        ..TrackV3::default()
    }
}

fn master_track(id: EntityId) -> TrackV3 {
    TrackV3 {
        id,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    }
}

fn routing(nodes: Vec<EntityId>, edges: &[(EntityId, EntityId)]) -> RoutingGraph {
    let mut graph = RoutingGraph {
        nodes,
        ..RoutingGraph::default()
    };
    for (source, destination) in edges {
        let id = EntityId::new();
        graph.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: *source,
                destination_node: *destination,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
                source_lane: None,
                dest_lane: None,
            },
        );
    }
    graph
}

fn assemble(
    tracks: Vec<(TrackV3, Option<ClipPoolEntry>)>,
    master: EntityId,
    graph: RoutingGraph,
) -> YebanProjectV1 {
    let mut track_map = BTreeMap::new();
    let mut clip_pool = BTreeMap::new();
    for (track, clip) in tracks {
        track_map.insert(track.id, track);
        if let Some(clip) = clip {
            clip_pool.insert(clip.id, clip);
        }
    }
    track_map.insert(master, master_track(master));
    YebanProjectV1 {
        master_bus_track_id: master,
        tracks: track_map,
        routing_graph: graph,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

// ===========================================================================
// S1：总线补偿延迟 D = 256 > 一量子（128）
// ===========================================================================

#[derive(Clone, Copy)]
struct S1Ids {
    master: EntityId,
    src: EntityId,
    bus1: EntityId,
    bus2: EntityId,
    bus3: EntityId,
}

fn s1_project(ids: S1Ids, bus3_latency: u32) -> YebanProjectV1 {
    let src = note_track(ids.src, "Src", 0, 60, true);
    let bus1 = (aux_bus(ids.bus1, "Bus1", 0), None);
    let bus2 = (aux_bus(ids.bus2, "Bus2", 0), None);
    let bus3 = (aux_bus(ids.bus3, "Bus3", bus3_latency), None);
    let graph = routing(
        vec![ids.src, ids.bus1, ids.bus2, ids.bus3, ids.master],
        &[
            (ids.src, ids.bus1),
            (ids.src, ids.bus2),
            (ids.bus1, ids.master),
            (ids.bus2, ids.master),
            (ids.bus3, ids.master),
        ],
    );
    assemble(vec![src, bus1, bus2, bus3], ids.master, graph)
}

/// S1：两条总线的出边补偿 `D = 256`（由第三条总线的上报延迟撑起
/// `L(master) = 256`）。总线在逐轨段被喂自有静音、3a⁗ 再喂收入之和：
/// `D ≤ 128` 时两遍输出分区无重叠，`D = 256` 时延迟线的时间轴以两倍速
/// 推进 ⇒ 分量实际只晚 128 帧。断言：分量**恰好晚 256 帧**（逐位）。
#[test]
fn bus_delay_beyond_one_quantum_lands_on_plan() {
    let ids = S1Ids {
        master: EntityId::new(),
        src: EntityId::new(),
        bus1: EntityId::new(),
        bus2: EntityId::new(),
        bus3: EntityId::new(),
    };
    let delay = 256usize;

    // ---- 计划面 ----
    {
        let snapshot =
            EngineSnapshot::from_project(&s1_project(ids, delay as u32), 1).expect("工程必须合法");
        let plan = snapshot.pdc();
        assert_eq!(
            plan.edge_compensation(&ids.bus1, &ids.master),
            Some(delay as u32),
            "计划：bus1→master 必须吃 256"
        );
        assert_eq!(
            plan.edge_compensation(&ids.bus2, &ids.master),
            Some(delay as u32),
            "计划：bus2→master 必须吃 256"
        );
        assert_eq!(
            plan.edge_compensation(&ids.bus3, &ids.master),
            Some(0),
            "计划：bus3→master 吃 0（它自己的 256 在它之后）"
        );
    }

    let base = render(&s1_project(ids, 0), QUANTA);
    let dual = render(&s1_project(ids, delay as u32), QUANTA);

    // ---- 反空转：两次渲染必须真的不同，且基线非静音 ----
    assert_ne!(base.left, dual.left, "上报 0 与上报 256 的渲染必须不同");
    let loud = base.left.iter().filter(|s| s.abs() > 1e-3).count();
    assert!(loud > 500, "基线必须有足够多非静音帧（实测 {loud}）");
    assert_eq!(dual.stats.pdc_unarmed_nodes, 0, "总线节点线必须进池");

    // ---- 落点：dual[t] == base[t − 256] 逐位 ----
    for index in 0..base.frames() {
        let expected = if index >= delay {
            base.left[index - delay]
        } else {
            0.0
        };
        assert_eq!(
            dual.left[index].to_bits(),
            expected.to_bits(),
            "S1 第 {index} 帧：总线分量必须恰好晚 {delay} 帧"
        );
    }
}
