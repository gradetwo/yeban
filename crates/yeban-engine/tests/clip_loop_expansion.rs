//! `#246` / [ADR-0010]：片段循环在**引擎排程**里真的展开。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | S2 | 48 kHz / 120 BPM（1 tick = 25 样本）下的硬编码帧表 | `loop_repeat` 恒 `None` ⇒ 只剩 1 条 |
//! | S3 | 母带四等分的**段峰值相等**；关循环臂只有第 1 段非零 | 不裁尾巴 / 只撑长度 |
//! | S4 | `covering` / `beyond` / `enabled:false` 三臂**逐位**等于基线 | 把 `end >= D` 的 `==` 丢掉 |
//! | S6 | 与 `yeban-mcp` 的模型端断言**同一张** tick 表与帧表 | 任一端另立一份实现 |
//! | S7 | 大 N 时 `note_schedule_drops` **非 0**（8192/轨上限） | 假装它恒 0 |
//!
//! 夹具刻意与 `crates/yeban-mcp/src/domain/render.rs` 的
//! `loop_expansion_is_delegated_to_the_model` **逐字同参数**。

mod support;

use std::collections::BTreeMap;

use support::{Render, render};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 摆放跨度：`1920` tick（48 kHz / 120 BPM ⇒ 48 000 样本）。
const DURATION_TICKS: u64 = 1_920;

/// 判据用的音符：`0..240` tick、中音 C、力度 100。
const NOTE: (u64, u64, u8, u8) = (0, 240, 60, 100);

fn looping(start_tick: u64, end_tick: u64) -> LoopConfig {
    LoopConfig {
        enabled: true,
        start_tick,
        end_tick,
    }
}

/// 主总线 + 一条 MIDI 轨 + 一个 `DURATION_TICKS` 长的摆放（循环配置由调用方给）。
fn fixture(loop_config: LoopConfig, notes: &[(u64, u64, u8, u8)]) -> (YebanProjectV1, EntityId) {
    let master = EntityId::new();
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();

    let mut note_map = BTreeMap::new();
    for (start_tick, duration_ticks, pitch, velocity) in notes {
        let id = EntityId::new();
        let mut note = MidiNote::new(id, *start_tick, *pitch, *duration_ticks);
        note.velocity = *velocity;
        note_map.insert(id, note);
    }
    let mut pool = BTreeMap::new();
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes: note_map },
        },
    );

    let mut clips = BTreeMap::new();
    clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: DURATION_TICKS,
            loop_config,
            muted: false,
        },
    );

    let mut tracks = BTreeMap::new();
    tracks.insert(
        track,
        TrackV3 {
            id: track,
            name: "Track".to_owned(),
            kind: TrackKind::Midi,
            clips,
            ..TrackV3::default()
        },
    );
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
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

    (
        YebanProjectV1 {
            bpm: 120.0,
            master_bus_track_id: master,
            tracks,
            routing_graph: routing,
            clip_pool: pool,
            ..YebanProjectV1::default()
        },
        track,
    )
}

/// S2 + S6（引擎端）：`(start_sample, end_sample, pitch)` 的硬编码帧表，
/// 以及由它反推的 `(start_tick, end_tick, pitch)` tick 表。
#[test]
fn s2_engine_schedule_is_the_hardcoded_frame_and_tick_table() {
    let (project, track) = fixture(looping(0, 480), &[NOTE]);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
    let schedule = snapshot.schedule(&track).expect("MIDI 轨必有排程");

    let frames: Vec<(u64, u64, u8)> = schedule
        .notes()
        .iter()
        .map(|note| (note.start_sample(), note.end_sample(), note.pitch()))
        .collect();
    assert_eq!(
        frames,
        vec![
            (0, 6_000, 60),
            (12_000, 18_000, 60),
            (24_000, 30_000, 60),
            (36_000, 42_000, 60)
        ],
        "48 kHz / 120 BPM ⇒ 1 tick = 25 样本；N = ceil(1920 / 480) = 4"
    );

    // 同一张表换算回 tick（夹具的 25 样本/tick 是精确整数）。
    let ticks: Vec<(u64, u64, u8)> = frames
        .iter()
        .map(|(start_sample, end_sample, pitch)| (start_sample / 25, end_sample / 25, *pitch))
        .collect();
    assert_eq!(
        ticks,
        vec![
            (0, 240, 60),
            (480, 720, 60),
            (960, 1_200, 60),
            (1_440, 1_680, 60)
        ],
        "引擎端与 MCP 端（模型端）同一张 tick 表"
    );
    assert_eq!(snapshot.note_schedule_drops(), 0, "4 遍远低于 8192 上限");
}

/// 一个 `Render` 的逐位指纹（左右两声道全部样本，按 `start_sample` 原序）。
fn bits(rendered: &Render) -> Vec<u32> {
    rendered
        .left
        .iter()
        .chain(rendered.right.iter())
        .map(|sample| sample.to_bits())
        .collect()
}

/// 母带四等分中第 `index` 段的峰值（左右两声道取最大绝对值）。
fn segment_peak(rendered: &Render, index: usize) -> f32 {
    const FRAMES: usize = 48_000;
    const SEGMENT: usize = FRAMES / 4;
    let range = index * SEGMENT..(index + 1) * SEGMENT;
    rendered.left[range.clone()]
        .iter()
        .chain(rendered.right[range].iter())
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

/// S3：四段（各 12 000 帧 = 480 tick）**段峰值相等**；关循环臂只有第 1 段非零。
///
/// 段峰值就是"真的重复了"的目标值：每一遍都以 `end_tick` 硬切，所以前一遍的尾音
/// 不会叠进后一遍 —— 四段的起音/衰减/释放形状逐段相同。
#[test]
fn s3_master_quarters_have_equal_peaks_and_the_disabled_arm_only_sounds_once() {
    // 375 个量子 × 128 帧 = 48 000 帧 = 1920 tick，恰好覆盖整个摆放。
    let (looped, _) = fixture(looping(0, 480), &[NOTE]);
    let rendered = render(&looped, 375);
    assert_eq!(rendered.frames(), 48_000);
    let peaks: Vec<f32> = (0..4).map(|index| segment_peak(&rendered, index)).collect();
    println!("[engine-loop] S3 looped quarter peaks = {peaks:?}");
    let max = peaks.iter().copied().fold(0.0f32, f32::max);
    let min = peaks.iter().copied().fold(f32::MAX, f32::min);
    assert!(min > 0.01, "四段都必须真的出声: {peaks:?}");
    assert!(
        (max - min) <= 1e-6 * max,
        "四段峰值必须相等（相对差 ≤ 1e-6；只撑长度/不裁尾巴会让它们不等）: {peaks:?}"
    );

    // 关循环臂：同一份夹具，唯一差别是 `loop_config`。
    let (plain, _) = fixture(LoopConfig::default(), &[NOTE]);
    let plain = render(&plain, 375);
    let plain_peaks: Vec<f32> = (0..4).map(|index| segment_peak(&plain, index)).collect();
    println!("[engine-loop] S3 disabled quarter peaks = {plain_peaks:?}");
    assert!(plain_peaks[0] > 0.01, "第 1 段必须出声: {plain_peaks:?}");
    for (index, peak) in plain_peaks.iter().enumerate().skip(1) {
        assert_eq!(
            peak.to_bits(),
            0.0f32.to_bits(),
            "关循环只播一遍 ⇒ 第 {} 段必须逐位静音: {plain_peaks:?}",
            index + 1
        );
    }
}

/// S4：不循环 / 恰好覆盖 / 越过末端三臂**逐位**等于基线，且 `note_schedule_drops == 0`。
///
/// `masterDigest == DEFAULT_MASTER_DIGEST` 那一半在 `yeban-mcp` 的
/// `a_loop_that_does_not_wrap_before_the_placement_ends_is_bit_identical` 上，
/// 本判据钉住的是**引擎侧全量子**。两条成对读。
#[test]
fn s4_non_wrapping_loops_do_not_change_one_bit_and_drop_nothing() {
    let (baseline, _) = fixture(LoopConfig::default(), &[NOTE]);
    let baseline_snapshot = EngineSnapshot::from_project(&baseline, 1).expect("投影成功");
    assert_eq!(baseline_snapshot.note_schedule_drops(), 0);
    let baseline_render = render(&baseline, 64);
    assert!(baseline_render.peak() > 0.01, "基线必须真的出声");

    for (tag, loop_config) in [
        ("enabled-false", LoopConfig::default()),
        ("covering", looping(0, DURATION_TICKS)),
        ("beyond", looping(0, 2 * DURATION_TICKS)),
    ] {
        let (project, _) = fixture(loop_config, &[NOTE]);
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
        assert_eq!(snapshot.note_schedule_drops(), 0, "{tag}: 单遍不触上限");
        let rendered = render(&project, 64);
        assert_eq!(
            bits(&rendered),
            bits(&baseline_render),
            "{tag}: 全量子必须逐位等于基线（一个字节都不许动）"
        );
        assert_eq!(
            rendered.fingerprint(),
            baseline_render.fingerprint(),
            "{tag}: 指纹也必须相同"
        );
    }
}

/// S7：大 N 时 `note_schedule_drops` **会**非 0（每轨上限 8192）。
///
/// `D = 20 000`、循环 `0..1`、音符 `0..1` ⇒ `N = 20 000` 遍；前 8192 遍进表，
/// 其余 **11 808** 遍被丢弃并计数。这里显式声明该读数，⛔ 不假装它恒 0。
#[test]
fn s7_large_n_exceeds_the_per_track_cap_and_is_counted() {
    let (project, track) = fixture(looping(0, 1), &[(0, 1, 60, 100)]);
    let mut project = project;
    // 放大摆放跨度：`fixture` 的 `DURATION_TICKS` 是常量，这里改 20 000。
    let placement = project
        .tracks
        .get_mut(&track)
        .expect("夹具轨")
        .clips
        .values_mut()
        .next()
        .expect("夹具摆放");
    placement.duration_ticks = 20_000;

    let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
    let scheduled = snapshot.schedule(&track).expect("MIDI 轨必有排程").len();
    println!(
        "[engine-loop] S7 scheduled = {scheduled}, drops = {}",
        snapshot.note_schedule_drops()
    );
    assert_eq!(scheduled, yeban_engine::synth::MAX_NOTES_PER_TRACK);
    assert_eq!(
        snapshot.note_schedule_drops(),
        20_000 - yeban_engine::synth::MAX_NOTES_PER_TRACK as u64,
        "副本真的计入丢弃计数（⛔ 不是恒 0）"
    );
}
