//! 任务 A：**最小工程夹具** ＋ `arm_from_snapshot` 的三项性能读数。
//!
//! 量什么（对象 + 单位）：
//! * `arm_from_snapshot` **一次调用的耗时**（微秒，N=100 取均值与最小值；该调用幂等，可重复）；
//! * **渲染 8 秒音频的墙钟耗时**（毫秒；48 kHz × 8 s = 384,000 帧，块 512 帧）；
//! * 峰值内存（RSS）由 `scripts/gates/measure_rss.py` 在**本机**量，读数写在提交信息与报告里。
//!
//! ⚠ 夹具形态照抄 `snapshot.rs` 的私有 `simple_project()`（一条 `TrackToBus` 支路 + 一条轨道）。
//! ⚠ 本判据**不**断言"快"或"慢"——它只**打印数值**：性能门的阈值属于账本，⛔ 不由本判据发明。

use std::collections::BTreeMap;
use std::time::Instant;
use yeban_engine::graph::LatencyTable;
use yeban_engine::snapshot::{EngineSnapshot, TrackParams};
use yeban_engine::synth::SynthEngine;
use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind, TrackV3};

/// 最小工程夹具：一条轨道 → master，48 kHz，双声道，块 512 帧。
fn minimal_snapshot() -> (EngineSnapshot, EntityId) {
    let master = EntityId::new();
    let track = EntityId::new();
    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let id = EntityId::new();
    routing.edges.insert(
        id,
        RoutingEdge {
            id,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    let mut tracks = BTreeMap::new();
    // `TrackParams` 由模型层音轨**投影**而来（无 `Default`）⇒ 照 `simple_project()` 的形态造一条。
    let track_v3 = TrackV3 {
        id: track,
        volume_db: -6.0,
        ..TrackV3::default()
    };
    tracks.insert(track, TrackParams::from_track(&track_v3, 0));
    let snapshot = EngineSnapshot::from_parts(
        1,
        48_000,
        512,
        2,
        master,
        tracks,
        &routing,
        &LatencyTable::new(),
    )
    .expect("一条 TrackToBus 支路的图必须合法");
    (snapshot, track)
}

#[test]
fn arm_from_snapshot_and_render_eight_seconds() {
    let (snapshot, track) = minimal_snapshot();
    let mut engine = SynthEngine::new(48_000);

    // ① arm 一次：N=100（幂等 ⇒ 重复调用不改变结果，只量耗时）
    let iterations = 100u32;
    let mut total = 0u128;
    let mut best = u128::MAX;
    for _ in 0..iterations {
        let start = Instant::now();
        engine
            .arm_from_snapshot(&snapshot)
            .expect("最小夹具必须武装成功");
        let elapsed = start.elapsed().as_micros();
        total += elapsed;
        best = best.min(elapsed);
    }
    println!(
        "[perf] arm_from_snapshot: 均值 = {} µs／次，最小 = {} µs／次（N={iterations}，48 kHz，1 轨）",
        total / u128::from(iterations),
        best
    );

    // ② 渲染 8 秒：384,000 帧，块 512 帧 ⇒ 750 次 `render_track`
    const BLOCK: usize = 512;
    let frames = 48_000usize * 8;
    let mut out = vec![0.0f32; BLOCK];
    let start = Instant::now();
    let mut rendered = 0usize;
    while rendered < frames {
        let step = BLOCK.min(frames - rendered);
        engine.render_track(track, None, &mut out[..step]);
        rendered += step;
    }
    let wall = start.elapsed();
    let wall_ms = wall.as_secs_f64() * 1e3;
    println!(
        "[perf] 渲染 8 秒音频（{frames} 帧、块 {BLOCK}）: 墙钟 = {wall_ms:.3} ms ⇒ 实时倍率 = {:.1}×",
        (frames as f64 / 48_000.0) / wall.as_secs_f64()
    );
    // ⭐ 非真空：8 秒必须真的推进了这么多帧（⛔ 防止"循环没跑"也能打印"
    assert_eq!(rendered, frames, "必须真正渲染 {frames} 帧");

    // ③ 夹具本身的自检（⛔ 不用来断言性能）：
    //    * `arm_from_snapshot` 对**同一**快照重复调用不 panic（幂等性）；无 master ⇒ 报错。
    let bare = {
        let master = EntityId::new();
        let mut routing = RoutingGraph::default();
        routing.nodes.push(master);
        EngineSnapshot::from_parts(
            2,
            48_000,
            512,
            2,
            master,
            BTreeMap::new(),
            &routing,
            &LatencyTable::new(),
        )
        .expect("单节点图合法")
    };
    let mut fresh = SynthEngine::new(48_000);
    let error = fresh
        .arm_from_snapshot(&bare)
        .expect_err("无母线的快照必须报错");
    println!("[perf] 无母线快照的报错: {error:?}");
}
