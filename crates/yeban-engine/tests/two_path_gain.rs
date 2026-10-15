//! 判据：**同一个 `(track, pitch, velocity)` 在两条触发路径上落地的增益逐位一致**。
//! `[ARCH-RT-004]`
//!
//! # 两条路是什么
//!
//! | 路 | 入口 | 增益的来源 |
//! | :--- | :--- | :--- |
//! | **已调度** | `EngineSnapshot::from_project` 投影 → `NoteSchedule` 游标 → [`SynthEngine::render_track`] → `PolySynth::note_on` | `snapshot.rs` 的投影：`velocity_gain(velocity) × track_gain_scale(...)` |
//! | **环音符**（活音符） | `EngineEvent::NoteOn` → [`SynthEngine::note_on`] | `velocity_gain(velocity) × slot.gain_scale`（`slot.gain_scale` 由 `set_track_gain_scales` 用**同一个** `track_gain_scale` 投影） |
//!
//! 两条路的**公式**由构造保证同源（`track_gain_scale` 是那段组合的唯一实现，
//! 见 `synth.rs` 的文档）。但"同源"是**源码形状**；本判据读的是**器件里真正落地的那一份**
//! （`PolyVoice.gain`，经只读取值器 `SynthEngine::debug_gain_for`），并逐位比较。
//!
//! # 三个档（每个档都要求两路**逐位**相等）
//!
//! 1. **力度 0** ⇒ 两路都必须 `== 0.0`（`velocity_gain(0) = 0`）；
//! 2. **静音轨** ⇒ 两路都必须 `== 0.0`（`track_gain_scale` 的可闻门为假）；
//! 3. **正常** ⇒ 两路都**非 0** 且逐位相等（否则"相等"可能只是"两边都 0"的假绿）。
//!
//! # 怎么变红
//!
//! 让任何一侧的公式漂开即可，例如：把 `SynthEngine::note_on` 的
//! `velocity_gain(velocity) * slot.gain_scale` 改成直接 `velocity_gain(velocity)`
//! （静音轨档与正常档两路的比值立刻不同）；或把快照投影的
//! `velocity_gain(note.velocity) * gain` 改成只取 `gain`。
//!
//! ⚠ 本判据**不需要** `device` feature：夹具走的是
//! `YebanProjectV1 → EngineSnapshot::from_project → SynthEngine::arm_from_snapshot`，
//! 没有声卡、没有测试专用捷径。

use std::collections::BTreeMap;

use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::SynthEngine;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 判据用的音高与（正常档的）力度。
const PITCH: u8 = 64;
const VELOCITY: u8 = 100;
/// 音符时值（tick）：960 PPQ 下一拍，足够长到"触发之后仍在响"。
const DURATION_TICKS: u64 = 960;

/// 一条 MIDI 轨（最多一个音符）＋ 母线的最小工程。
///
/// `mute` 直接进 [`TrackV3::mute`] ⇒ 两条路读的是**同一个**可闻门；
/// `velocity` 同时进已调度音符与（下面）环音符事件 ⇒ 两条路的力度自变量逐位相同。
fn project(mute: bool, velocity: u8) -> (YebanProjectV1, EntityId) {
    let master = EntityId::new();
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();

    let note_id = EntityId::new();
    let mut note = MidiNote::new(note_id, 0, PITCH, DURATION_TICKS);
    note.velocity = velocity;
    let mut notes = BTreeMap::new();
    notes.insert(note_id, note);

    let mut track_def = TrackV3 {
        id: track,
        name: "Track".to_owned(),
        kind: TrackKind::Midi,
        mute,
        ..TrackV3::default()
    };
    track_def.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: DURATION_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );

    let mut tracks = BTreeMap::new();
    tracks.insert(track, track_def);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut clip_pool = BTreeMap::new();
    clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );

    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

    let project = YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    };
    (project, track)
}

/// 读同一份工程在两条路上的**落地增益**：`(已调度, 环音符)`。
fn applied_gains(mute: bool, velocity: u8) -> (f32, f32) {
    let (project, track) = project(mute, velocity);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须能编译成快照");

    // --- 已调度路径：投影 → 游标 → 器件 ---
    let mut scheduled = SynthEngine::new(48_000);
    scheduled
        .arm_from_snapshot(&snapshot)
        .expect("武装最小工程不得报错");
    let schedule = snapshot.schedules().get(&track);
    assert!(
        schedule.is_some_and(|schedule| !schedule.notes().is_empty()),
        "夹具必须真的投影出一条已调度音符（否则本档的'已调度'一侧是空壳）"
    );
    let mut out = vec![0.0f32; 512];
    scheduled.render_track(track, schedule, None, &mut out);
    let scheduled_gain = scheduled
        .debug_gain_for(track, PITCH)
        .expect("已调度路径必须已经占住一个声明该频率的声部");

    // --- 环音符路径：同一份快照（同一个可闻门/推子）---
    let mut live = SynthEngine::new(48_000);
    live.arm_from_snapshot(&snapshot)
        .expect("武装最小工程不得报错");
    let _ = live.note_on(track, PITCH, velocity);
    let live_gain = live
        .debug_gain_for(track, PITCH)
        .expect("环音符路径必须已经占住一个声明该频率的声部");

    (scheduled_gain, live_gain)
}

/// 三个档：力度 0 / 静音轨 / 正常 —— 每个档都要求两路**逐位**相等。
#[test]
fn the_two_trigger_paths_apply_the_same_gain_value_for_value() {
    // ③ 正常档：先证"两路都真的非 0"，再比逐位相等（否则相等可能只是"两边都 0"）。
    let (normal_scheduled, normal_live) = applied_gains(false, VELOCITY);
    assert_ne!(
        normal_scheduled, 0.0,
        "正常档的已调度增益必须非 0（否则本档证明的是'0 == 0'）"
    );
    assert_ne!(normal_live, 0.0, "正常档的环音符增益必须非 0");
    assert_eq!(
        normal_scheduled.to_bits(),
        normal_live.to_bits(),
        "正常档：已调度 {normal_scheduled} / 环音符 {normal_live} 必须逐位相等"
    );

    // ① 力度 0 ⇒ 两路都必须 0。
    let (zero_scheduled, zero_live) = applied_gains(false, 0);
    assert_eq!(zero_scheduled, 0.0, "力度 0 的已调度增益必须是 0");
    assert_eq!(zero_live, 0.0, "力度 0 的环音符增益必须是 0");
    assert_eq!(
        zero_scheduled.to_bits(),
        zero_live.to_bits(),
        "力度 0 档：两路都必须逐位 0"
    );

    // ② 静音轨 ⇒ 两路都必须 0。
    let (mute_scheduled, mute_live) = applied_gains(true, VELOCITY);
    assert_eq!(mute_scheduled, 0.0, "静音轨的已调度增益必须是 0");
    assert_eq!(mute_live, 0.0, "静音轨的环音符增益必须是 0");
    assert_eq!(
        mute_scheduled.to_bits(),
        mute_live.to_bits(),
        "静音轨档：两路都必须逐位 0"
    );

    println!(
        "[two-path-gain] 正常档 已调度={normal_scheduled:?} 环音符={normal_live:?}（逐位相等）；\
         力度 0 档 两路={zero_scheduled:?}/{zero_live:?}；静音轨档 两路={mute_scheduled:?}/{mute_live:?}"
    );
}
