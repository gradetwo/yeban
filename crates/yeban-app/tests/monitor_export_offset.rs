//! 任务单 `#302` 的复现探针（**先量**）：监听输出相对导出的固定帧偏移。
//!
//! 口径与 `#301` 一致：中性 fixture（单轨、`pan = 0`、无设备/自动化）、
//! 监听 = `EngineRuntime::process_quantum` 推 375 个量子（48,000 帧），
//! 导出 = `render_project_f32_with_assets_under` 的 f32 母带。
//!
//! 本文件先只**报读数**（`best_offset`、各候选偏移的不匹配帧数、引擎延迟读数）；
//! 修后的判据（`best_offset = 0` 且逐帧 0 差异）在实现阶段把它改成断言。

use std::collections::BTreeMap;

use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::render_project_f32_with_assets_under;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

const FRAMES: usize = 48_000;
const QUANTA: usize = FRAMES / 128;

/// 单轨中性 fixture：与 `neutral_pan_leg.rs` 逐字同源。
fn neutral_project() -> YebanProjectV1 {
    let master = EntityId::new();
    let lead = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note = EntityId::new();

    let mut pool = BTreeMap::new();
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    note,
                    MidiNote {
                        velocity: 100,
                        ..MidiNote::new(note, 0, 69, 480)
                    },
                )]),
            },
        },
    );
    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    lead_track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 480,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let master_track = TrackV3 {
        id: master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };
    let mut project = YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks: BTreeMap::from([(master, master_track), (lead, lead_track)]),
        clip_pool: pool,
        ..YebanProjectV1::default()
    };
    project.routing_graph = RoutingGraph {
        nodes: vec![lead, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: lead,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("中性夹具必须合法");
    project
}

/// 监听腿：返回 `(左, 右, 运行时读数)`。
fn render_monitoring(
    project: &YebanProjectV1,
    limiter_enabled: bool,
) -> (Vec<f32>, Vec<f32>, EngineStats) {
    let snapshot =
        EngineSnapshot::from_project_with_limiter(project, 1, limiter_enabled).expect("快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut left = Vec::with_capacity(FRAMES);
    let mut right = Vec::with_capacity(FRAMES);
    let mut out = vec![0.0f32; 256];
    for _ in 0..QUANTA {
        out.fill(0.0);
        runtime.process_quantum(&mut out, 2);
        for frame in 0..128 {
            left.push(out[frame * 2]);
            right.push(out[frame * 2 + 1]);
        }
    }
    let stats = runtime.stats();
    (left, right, stats)
}

/// 导出腿（`limiter` = 任务单 `#302` 的 per-call 开关）。
fn render_export(project: &YebanProjectV1, limiter: bool) -> (Vec<f32>, Vec<f32>, u64) {
    let options = WavExportOptions {
        frames: FRAMES as u64,
        limiter,
        ..WavExportOptions::default()
    };
    let output = render_project_f32_with_assets_under(project, options, None, &[], None)
        .expect("中性夹具必须能导出");
    let mut left = Vec::with_capacity(FRAMES);
    let mut right = Vec::with_capacity(FRAMES);
    for pair in output.samples.as_chunks::<2>().0 {
        left.push(pair[0]);
        right.push(pair[1]);
    }
    (left, right, output.limiter_reductions)
}

/// 在偏移 `offset` 下逐位比较的重叠帧数 + 不匹配帧数（左右分开）。
fn compare(mon: &[f32], exp: &[f32], offset: usize) -> (usize, usize) {
    let frames = mon.len().saturating_sub(offset).min(exp.len());
    let mut mismatches = 0usize;
    for frame in 0..frames {
        if mon[offset + frame].to_bits() != exp[frame].to_bits() {
            mismatches += 1;
        }
    }
    (frames, mismatches)
}

#[test]
fn default_monitoring_is_sample_aligned_with_export() {
    let project = neutral_project();
    let (mon_l, mon_r, stats) = render_monitoring(&project, false);
    let (exp_l, exp_r, _) = render_export(&project, false);

    let mut best = (0usize, 0usize, 0usize); // (offset, mismatch_l, mismatch_r)
    let mut best_sum = usize::MAX;
    for offset in 0..=64usize {
        let (frames, mismatch_l) = compare(&mon_l, &exp_l, offset);
        let (_, mismatch_r) = compare(&mon_r, &exp_r, offset);
        if frames > 0 && mismatch_l + mismatch_r < best_sum {
            best = (offset, mismatch_l, mismatch_r);
            best_sum = mismatch_l + mismatch_r;
        }
    }
    let (best_offset, best_l, best_r) = best;
    let (frames_at_best, _) = compare(&mon_l, &exp_l, best_offset);
    let (frames_at_33, mismatch_l_33) = compare(&mon_l, &exp_l, 33);
    let (_, mismatch_r_33) = compare(&mon_r, &exp_r, 33);
    let (_, mismatch_l_16) = compare(&mon_l, &exp_l, 16);
    let (_, mismatch_r_16) = compare(&mon_r, &exp_r, 16);
    let (_, mismatch_l_0) = compare(&mon_l, &exp_l, 0);
    let (_, mismatch_r_0) = compare(&mon_r, &exp_r, 0);

    println!(
        "[#302/probe] best_offset={best_offset} (L mismatch={best_l}, R mismatch={best_r}, \
         compared frames={frames_at_best})"
    );
    println!(
        "[#302/probe] offset=0  L/R mismatch={mismatch_l_0}/{mismatch_r_0}; \
         offset=16 L/R mismatch={mismatch_l_16}/{mismatch_r_16}; \
         offset=33 L/R mismatch={mismatch_l_33}/{mismatch_r_33} \
         (compared frames={frames_at_33})"
    );
    println!(
        "[#302/probe] stats: pdc_alignment_frames={} engine_output_latency_frames={} \
         quanta={} limiter_gain_reductions={} limiter_max_reduction={:.9} \
         limiter_current_reduction={:.9}",
        stats.pdc_alignment_frames,
        stats.engine_output_latency_frames,
        stats.quanta,
        stats.limiter_gain_reductions,
        stats.limiter_max_reduction,
        stats.limiter_current_reduction,
    );
    println!(
        "[#302/probe] monitor peaks L/R = {:.9}/{:.9}; export peaks L/R = {:.9}/{:.9}",
        mon_l.iter().fold(0.0f32, |p, s| p.max(s.abs())),
        mon_r.iter().fold(0.0f32, |p, s| p.max(s.abs())),
        exp_l.iter().fold(0.0f32, |p, s| p.max(s.abs())),
        exp_r.iter().fold(0.0f32, |p, s| p.max(s.abs())),
    );

    assert_eq!(
        best_offset, 0,
        "默认关：监听必须与导出在**样本层**对齐（best_offset = 0）"
    );
    assert_eq!(
        (best_l, best_r),
        (0, 0),
        "默认关：整段 48,000 帧必须逐位相同"
    );
    assert_eq!(
        stats.engine_output_latency_frames, 0,
        "默认关：引擎输出延迟读数必须是 0"
    );
    assert_eq!(stats.pdc_alignment_frames, 0, "中性夹具的 PDC 对齐基准是 0");
}

/// ⭐ 开关的双态判据（任务单 `#302`，父 agent 要求）：
/// **同一 fixture**，off ⇒ `best_offset = 0`；on ⇒ `best_offset = 33`（限制器环）。
///
/// 这条同时钉住"开关真的有效"与"默认档偏移真的消失"；它不依赖
/// `engine_latency_reading` 的读数，而是直接从**样本**里量出来。
#[test]
fn limiter_switch_changes_the_offset_from_zero_to_33() {
    let project = neutral_project();
    let (exp_l, exp_r, _) = render_export(&project, false);

    // off：0
    let (off_l, off_r, off_stats) = render_monitoring(&project, false);
    let (_, off_l_mismatch) = compare(&off_l, &exp_l, 0);
    let (_, off_r_mismatch) = compare(&off_r, &exp_r, 0);
    assert_eq!(
        (off_l_mismatch, off_r_mismatch),
        (0, 0),
        "off：offset 0 下不许有差异"
    );
    assert_eq!(off_stats.engine_output_latency_frames, 0);

    // on：33（中性夹具限制器透明 ⇒ 纯延迟）
    let (on_l, on_r, on_stats) = render_monitoring(&project, true);
    let (frames, on_l_mismatch) = compare(&on_l, &exp_l, 33);
    let (_, on_r_mismatch) = compare(&on_r, &exp_r, 33);
    assert_eq!(
        (on_l_mismatch, on_r_mismatch),
        (0, 0),
        "on：offset 33 下不许有差异（限制器必须透明）"
    );
    assert_eq!(frames, FRAMES - 33, "on 态可比较的重叠帧数");
    assert_eq!(
        on_stats.engine_output_latency_frames, 33,
        "on：引擎输出延迟读数必须是 33"
    );
    assert_eq!(
        on_stats.pdc_alignment_frames, 0,
        "on：中性夹具的 PDC 对齐基准仍是 0"
    );
    assert_eq!(
        on_stats.limiter_gain_reductions, 0,
        "on：中性夹具必须真的透明（否则 off/on 的差里混了压限）"
    );

    println!(
        "[#302/dual] off best_offset=0 mismatches=0/0；on best_offset=33 \
         mismatches=0/0（overlap {frames}）；reading off/on = {}/{}",
        off_stats.engine_output_latency_frames, on_stats.engine_output_latency_frames,
    );
}

/// ⭐ 导出侧的 per-call 开关（任务单 `#302`）：
/// 中性母带 ⇒ on 与 off **逐位相同**（限制器透明 + 33 帧补偿）；热母带 ⇒ on 压到
/// `LIMITER_CEILING` 之下、长度不变、且与 off 不同。
#[test]
fn export_limiter_is_transparent_on_neutral_and_caps_a_hot_master() {
    let neutral = neutral_project();
    let (off_l, off_r, _) = render_export(&neutral, false);
    let (on_l, on_r, _) = render_export(&neutral, true);
    assert_eq!(on_l.len(), FRAMES, "开启限制器不得改变输出帧数");
    let bits = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        bits(&on_l),
        bits(&off_l),
        "中性母带：限制器开/关必须逐位相同（左）"
    );
    assert_eq!(
        bits(&on_r),
        bits(&off_r),
        "中性母带：限制器开/关必须逐位相同（右）"
    );

    // 热母带：单轨 +12 dB ⇒ 导出峰值远高于阈值。
    let mut hot = neutral_project();
    for track in hot.tracks.values_mut() {
        if track.kind != yeban_model::TrackKind::Master {
            track.volume_db = 12.0;
        }
    }
    let (hot_off_l, _, hot_off_reductions) = render_export(&hot, false);
    let (hot_on_l, _, hot_on_reductions) = render_export(&hot, true);
    let peak = |samples: &[f32]| samples.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    let off_peak = peak(&hot_off_l);
    let on_peak = peak(&hot_on_l);
    assert!(off_peak > 0.9, "热母带必须越过阈值（实测 {off_peak}）");
    assert!(
        on_peak <= yeban_engine::mixer::LIMITER_CEILING + f32::EPSILON,
        "开启限制器后峰值必须 ≤ 天花板（实测 {on_peak}）"
    );
    assert_eq!(hot_on_l.len(), FRAMES, "热母带开启限制器也不得改变帧数");
    assert_ne!(hot_on_l, hot_off_l, "热母带：限制器必须真的改变样本");
    assert!(
        hot_on_reductions > 0,
        "热母带：on 态必须真的压过帧（否则是假开关）"
    );
    assert_eq!(hot_off_reductions, 0, "off 态不得压任何帧");
    let reduction_db = 20.0 * (on_peak / off_peak).log10();
    println!(
        "[#302/export] 中性 on==off 逐位相同；热母带 off peak={off_peak:.9}（压过帧={hot_off_reductions}）\
         / on peak={on_peak:.9}（压过帧={hot_on_reductions}，峰值变化={reduction_db:.9} dB，天花板 {:.3}）",
        yeban_engine::mixer::LIMITER_CEILING
    );
}
