//! ⭐ **stage 22** 判据：**总线级 PDC 真的接上了**（不只是"算对了计划"）。
//!
//! ## 判什么
//!
//! 工程：`slow → bus → master`，并行 `fast → master`（`fast` 上挂一台
//! **上报延迟** 64 采样、但本身不产生物理延迟的设备 —— 这正是
//! `pdc_mix_path.rs` 文件头第 2 条登记的边界：`latency_samples` 是**上报值**）。
//!
//! 计划（下游求和式）：`arrival(fast) = 64`、`arrival(bus) = 0`、`L(master) = 64`
//! ⇒ `D(bus) = 64`、`D(slow) = D(fast) = 0`。于是**唯一**该被延迟的是总线输出。
//!
//! 因此本判据量的是**总线上那条 MIDI 轨的音频到底有没有整体后移 64 帧**：
//! * 上报延迟 0 ⇒ 起点 `o`；
//! * 上报延迟 64 ⇒ 起点 `o + 64`，且前 64 帧**逐位静音**、之后**逐位相同**。
//!
//! ⛔ 这不是"读数断言"：把 stage 22 加在总线转发段的那行 `apply_pair` 删掉，
//! 起点**不会**后移 ⇒ 本判据必红（注入实测见提交说明）。
//!
//! ⚠ **为什么不能只用轨来验**：轨的 `pdc.apply` 从 `ROAD-M2-004` 起就接好了，
//! 它证明不了"总线输出也走延迟线"。本判据的慢支路**必经总线**，因此红/绿完全
//! 由总线那一段决定。

mod support;

use std::collections::BTreeMap;

use support::render;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 夹具栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本。
const SAMPLES_PER_TICK: u64 = 25;

/// `fast` 支路上那台设备的**上报**延迟（采样点）。正好两个量子的一半，肉眼可辨。
const REPORTED_LATENCY: u32 = 64;

/// 音符时值（tick）：足够长，整个渲染窗口里都在发声。
const NOTE_TICKS: u64 = 960;

/// 力度取小：单支路远低于限制器阈值（默认档限制器也是关的，这里只是保守）。
const VELOCITY: u8 = 40;

/// 渲染窗口：8 量子 × 128 帧 = 1024 帧（> 64 + 一个量子的余量）。
const QUANTA: usize = 8;

#[derive(Clone, Copy)]
struct Ids {
    master: EntityId,
    fast: EntityId,
    slow: EntityId,
    bus: EntityId,
}

/// 一条 MIDI 轨 + 一个音符片段（起点 tick 0）。
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

fn ids() -> Ids {
    Ids {
        master: EntityId::new(),
        fast: EntityId::new(),
        slow: EntityId::new(),
        bus: EntityId::new(),
    }
}

/// `slow → bus → master` ＋ `fast → master`。
///
/// `reported_latency` 只挂在 `fast` 的设备上（`fast` **没有音符** ⇒ 它的音频不参与，
/// 但它的**设备链延迟**参与 `PdcPlan` —— 这正是"别的支路更慢"的合成方式）。
fn project(ids: Ids, reported_latency: u32) -> YebanProjectV1 {
    let (mut fast, _) = note_track(ids.fast, "Fast");
    // `fast` 静音：只借它的设备延迟。
    fast.clips.clear();
    fast.devices = vec![DeviceDefinition {
        id: EntityId::new(),
        name: "Reported".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: reported_latency,
        plugin: None,
    }];
    let (slow, slow_clip) = note_track(ids.slow, "Slow");

    let mut tracks = BTreeMap::new();
    tracks.insert(ids.fast, fast);
    tracks.insert(ids.slow, slow);
    tracks.insert(
        ids.bus,
        TrackV3 {
            id: ids.bus,
            name: "Bus".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        },
    );
    tracks.insert(
        ids.master,
        TrackV3 {
            id: ids.master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut clip_pool = BTreeMap::new();
    clip_pool.insert(slow_clip.id, slow_clip);

    let mut routing = RoutingGraph {
        nodes: vec![ids.fast, ids.slow, ids.bus, ids.master],
        ..RoutingGraph::default()
    };
    for (source, destination) in [
        (ids.fast, ids.master),
        (ids.slow, ids.bus),
        (ids.bus, ids.master),
    ] {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: source,
                destination_node: destination,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
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

/// 左声道第一个**非零**帧的下标（静音工程返回 `None`）。
fn first_nonzero(render: &support::Render) -> Option<usize> {
    render
        .left
        .iter()
        .position(|sample| sample.abs() > f32::EPSILON)
}

/// ⭐ **stage 22 判据**：另一条支路报 64 帧延迟时，**经过总线的那条轨整体后移 64 帧**。
#[test]
fn a_bus_that_is_the_short_branch_is_delayed_by_the_plan() {
    let ids = ids();
    let reference = render(&project(ids, 0), QUANTA);
    let shifted = render(&project(ids, REPORTED_LATENCY), QUANTA);

    let base = first_nonzero(&reference).expect("没挂延迟时慢支路必须发声（否则判据是空转）");
    let late = first_nonzero(&shifted).expect("挂了 64 帧上报延迟后慢支路仍必须发声");

    let delay = REPORTED_LATENCY as usize;
    assert_eq!(
        late,
        base + delay,
        "总线的短支路必须被补偿后移 {delay} 帧：reference={base}、shifted={late}"
    );

    // 前 `delay` 帧逐位静音：证明是**延迟线**（不是"整体音量变了"）。
    for (index, sample) in shifted.left[..delay].iter().enumerate() {
        assert_eq!(
            sample.to_bits(),
            0.0f32.to_bits(),
            "补偿延迟的前 {delay} 帧必须逐位静音（第 {index} 帧 = {sample}）"
        );
    }
    // 之后逐位相同：证明延迟的是**同一条波形**（不是换了渲染路径）。
    for index in 0..(reference.left.len() - delay) {
        assert_eq!(
            reference.left[index].to_bits(),
            shifted.left[index + delay].to_bits(),
            "第 {index} 帧：补偿后的波形必须与未补偿逐位相同（延迟线只移相位）"
        );
    }
    // 反向腿：两者**真的不是同一条缓冲**（否则"等号成立"可能来自同一个常量）。
    assert_ne!(
        reference.left, shifted.left,
        "两份渲染必须真的不同（证明延迟确实施加了）"
    );
    // ⚠ 读数边界：上报延迟只让 `output` 与 `total` 变化，而**支路内的延迟**
    // 由总线那一条线承担 —— 断言它确实被计入"真的补过"的见证。
    assert_eq!(
        shifted.stats.pdc_alignment_frames,
        reference.stats.pdc_alignment_frames + REPORTED_LATENCY,
        "总延迟读数必须增加上报的 64 帧（证明计划真的换了）"
    );
}
