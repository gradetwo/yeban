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
use yeban_engine::sfz_instrument::InMemorySample;
use yeban_engine::sfz_instrument::SfzInstrument;
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

/// ⭐ 有**音符排程**的 8 秒渲染读数（"⛔ 不是下界"的那一版）：
/// 每 12,000 帧（0.25 s）触发一次 SFZ 区域，样本是内存里的极短正弦；
/// 与 `arm_from_snapshot_and_render_eight_seconds` 的静音通路读数**同口径**（8 秒、512 帧块）。
#[test]
fn sfz_note_render_eight_seconds() {
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
        &Default::default(),
    )
    .expect("最小 SFZ 必须成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, 480);

    const BLOCK: usize = 512;
    const FIRE_EVERY: usize = 12_000; // 0.25 s @48k ⇒ 32 次触发
    let frames = 48_000usize * 8;
    let mut out = vec![0.0f32; BLOCK];
    let mut total_energy = 0.0f64;
    let mut voices = 0usize;
    let start = Instant::now();
    let mut rendered = 0usize;
    while rendered < frames {
        let step = BLOCK.min(frames - rendered);
        out[..step].fill(0.0);
        // ⚠ 触发条件必须与**块大小**对齐：`rendered % FIRE_EVERY == 0` 在 512 帧块下
        // 只会在 12,000 的整数倍恰好等于某个块边界时命中（本批实测：8 秒只触发 **2** 次，
        // 而预期是 32 次）⇒ 改成"落在下一段触发窗口内"。
        if rendered % FIRE_EVERY < BLOCK {
            let written = sfz
                .render_note_into(&sample, 60, 100, &mut out[..step], 48_000.0)
                .expect("渲染不得报错");
            if written > 0 {
                voices += 1;
            }
        }
        total_energy += out[..step]
            .iter()
            .map(|s| (*s as f64) * (*s as f64))
            .sum::<f64>();
        rendered += step;
    }
    let wall_ms = start.elapsed().as_secs_f64() * 1e3;
    println!(
        "[perf] SFZ 有排程 8 秒（{frames} 帧、块 {BLOCK}、触发 {voices} 次）: 墙钟 = {wall_ms:.3} ms，总能量 = {total_energy:.3}"
    );
    assert_eq!(rendered, frames, "必须真正渲染 {frames} 帧");
    assert!(voices > 0, "必须有区域被触发（否则这一版仍是'下界'）");
    assert!(
        total_energy > 0.0,
        "⭐ 有排程的渲染必须**非静音**（能量 {total_energy}）"
    );
}
