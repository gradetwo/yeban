//! T-FIX-123：逐轨渲染的 `track_budget` 只许约束**电平打点**，不许约束音频渲染。
//!
//! 前提（M2-D3 / P0）：`render_branch` 的逐轨主循环用
//! `if *id == master || produced >= track_budget { continue; }` 跳过整轨；
//! `produced` 只在写 `scratch_meters` 时递增，`track_budget = SCRATCH_METERS - 1 = 255`。
//! 于是第 256 条及以后的轨被整段跳过：不合成、不放片段、不走插入链、不进路由求和 ⇒ 无声。
//!
//! 本文件先用产品路径证明前提（改前：标靶轨落在第 256 位 ⇒ 输出全静音），
//! 修复后再证明渲染恢复（输出非静音）。

mod support;

use std::collections::BTreeMap;

use support::render;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 非 master 轨数量，刻意 > track_budget(255) 让"第 256 条"越界。
const TRACKS: usize = 260;
/// 标靶轨在排序后序号（0-based）：255 ⇒ 第 256 条非 master 轨，正是被 continue 的第一条。
const TARGET_INDEX: usize = 255;

/// 给一条轨加一个中音 C 音符（只夹一个片段）。
fn put_note(track: &mut TrackV3, pool: &mut BTreeMap<EntityId, ClipPoolEntry>, pitch: u8) {
    let clip = EntityId::new();
    let placement = EntityId::new();
    let mut notes = BTreeMap::new();
    let nid = EntityId::new();
    let mut note = MidiNote::new(nid, 0, pitch, 960);
    note.velocity = 110;
    notes.insert(nid, note);
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Target".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
}

/// 组装工程：给 tracks 里每条轨一条 TrackToBus 边连到 master。
fn wire_to_master(tracks: &BTreeMap<EntityId, TrackV3>, master: EntityId) -> RoutingGraph {
    let mut routing = RoutingGraph {
        nodes: Vec::new(),
        ..RoutingGraph::default()
    };
    for id in tracks.keys() {
        routing.nodes.push(*id);
        if *id == master {
            continue;
        }
        let eid = EntityId::new();
        routing.edges.insert(
            eid,
            RoutingEdge {
                id: eid,
                source_node: *id,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
                source_lane: None,
                dest_lane: None,
            },
        );
    }
    routing
}

/// 构造 TRACKS 条非 master 轨 + 一条 master。
/// 按 EntityId 排序后的第 TARGET_INDEX 条轨带一个音符，其余轨全静音。
/// 因此输出若非静音，只可能来自标靶轨。
fn build_project() -> YebanProjectV1 {
    build_project_with_target_at(TARGET_INDEX)
}

/// 快照必须能编成 >255 轨（MAX_TRACK_SLOTS=2048，容量足够）——前提前提。
#[test]
fn snapshot_accepts_more_than_track_budget_tracks() {
    let project = build_project();
    EngineSnapshot::from_project(&project, 1).expect("260 轨必须能编译成快照");
}

/// 核心判据：标靶轨（第 256 条非 master）必须发声。
///
/// 改前：`produced >= 255` 时整轨 continue ⇒ 只剩静音 ⇒ 本测试红。
/// 改后：渲染对所有轨执行，只有 meter 写入受预算限制 ⇒ 标靶轨发声 ⇒ 本测试绿。
#[test]
fn track_beyond_meter_budget_still_renders_audio() {
    let project = build_project();
    let out = render(&project, 64);
    let peak = out.peak();
    assert!(
        peak > 1e-4,
        "第 256 条轨的音符必须发声；改前因 `produced >= track_budget` 整轨 continue 而静音，\
         实测峰值={peak:.8}（非零样本={}）",
        out.nonzero()
    );
}

/// 最强判据：track_budget 只影响打点、不影响音频 —— 同一条发声轨无论排在
/// 预算内（第 1 条）还是预算外（第 256 条），输出必须逐位一致（其余轨全静音）。
#[test]
fn audio_is_bit_identical_regardless_of_meter_budget_position() {
    let a = build_project_with_target_at(0);
    let b = build_project_with_target_at(TARGET_INDEX);
    let out_a = render(&a, 64);
    let out_b = render(&b, 64);
    assert_eq!(
        out_a.left_bits(),
        out_b.left_bits(),
        "标靶轨在预算内(index 0)与预算外(index 255)的音频必须逐位一致"
    );
}

/// 构造 TRACKS 条轨，标靶轨放在排序后第 target_index 位。
fn build_project_with_target_at(target_index: usize) -> YebanProjectV1 {
    let mut ids: Vec<EntityId> = (0..TRACKS).map(|_| EntityId::new()).collect();
    ids.sort();
    let mut master = EntityId::new();
    while ids.last().is_some_and(|last| *last >= master) {
        master = EntityId::new();
    }
    let mut tracks = BTreeMap::new();
    let mut pool = BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        let mut track = TrackV3 {
            id: *id,
            name: format!("T{i}"),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        };
        if i == target_index {
            put_note(&mut track, &mut pool, 60);
        }
        tracks.insert(*id, track);
    }
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let routing = wire_to_master(&tracks, master);
    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        routing_graph: routing,
        tracks,
        clip_pool: pool,
        ..YebanProjectV1::default()
    }
}

/// 对照组：同一音符落在预算内（工程只有 1 条非 master 轨）必出声，
/// 证明上面那条的差异只来自 track_budget，不是音符本身无效。
#[test]
fn same_note_under_budget_is_audible() {
    let master = EntityId::new();
    let id = EntityId::new();
    // 保证 master 排序在普通轨之后（更稳的口径；非必须，但避免 master 被当成第 1 条）。
    let (master, id) = if master >= id {
        (master, id)
    } else {
        (id, master)
    };
    let mut tracks = BTreeMap::new();
    let mut pool = BTreeMap::new();
    let mut track = TrackV3 {
        id,
        name: "Only".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    put_note(&mut track, &mut pool, 60);
    tracks.insert(id, track);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let routing = wire_to_master(&tracks, master);
    let project = YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        routing_graph: routing,
        tracks,
        clip_pool: pool,
        ..YebanProjectV1::default()
    };
    let out = render(&project, 64);
    assert!(out.peak() > 1e-4, "基线：预算内的同一音符必须发声");
}
