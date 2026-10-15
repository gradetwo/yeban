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
use yeban_engine::sfz_instrument::SfzInstrument;
use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz};
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
        engine.render_track(track, None, None, &mut out[..step]);
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

/// ⭐ 判据 a：**摊平后的回调路径与离线路径逐样本一致** —— 场景是**两条区域都匹配**。
/// 判定量（已命名）：**逐样本最大绝对差** = `max_i |offline[i] − prepared[i]|`。
///
/// ⭐ 本判据的 SFZ 刻意让**两条 `key=60` 的区域同时匹配**（第二条另有 `volume=-6`）：
/// ① 若两侧的**多匹配语义**不一致（离线 `find` 只取第一条、摊平叠加所有条）⇒ 差值非 0；
/// ② 若摊平的**增益**仍冻结在 `(60, 100)` 或漏掉某一条 ⇒ 差值非 0。
/// 另有一条 `key=72` 的区域做**不命中**的反向对照。
#[test]
fn prepared_matches_offline_sample_by_sample() {
    const TWO_MATCH_ONE_MISS: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n\
                                      <region> sample=b.wav key=60 pitch_keycenter=60 volume=-6\n\
                                      <region> sample=c.wav key=72 pitch_keycenter=60\n";
    let sfz = SfzInstrument::load(TWO_MATCH_ONE_MISS.to_owned(), &Default::default())
        .expect("SFZ 必须成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, 480);

    // ① 两条都匹配 ⇒ 两侧都必须叠加两条
    let mut offline = vec![0.0f32; 512];
    sfz.render_note_into(&sample, 60, 100, &mut offline, 48_000.0)
        .expect("离线渲染不得报错");

    let prepared = PreparedSfz::prepare(&sfz, &sample, 48_000.0).expect("摊平不得报错");
    let mut flat = vec![0.0f32; 512];
    prepared.render_block_into(60, 100, &mut flat);

    let max_abs_diff = offline
        .iter()
        .zip(flat.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "[perf] 逐样本最大绝对差 = {max_abs_diff:e}（摊平区域数 {}，内存 {} 字节）",
        prepared.region_count(),
        prepared.memory_bytes()
    );
    assert_eq!(
        max_abs_diff, 0.0,
        "⭐ 摊平路径必须与离线路径**逐样本相同**（两条都匹配的多匹配场景）"
    );
    // ② 反向对照 1：两侧都必须**真的出声**（⛔ 不能"两边都全零"也算一致）
    assert!(offline.iter().any(|s| *s != 0.0), "离线路径必须真的出声");
    assert!(flat.iter().any(|s| *s != 0.0), "摊平路径必须真的出声");

    // ③ 反向对照 2：**"两条都匹配"真的叠加了两条** —— 只留一条的 SFZ 必须给出不同波形。
    //    （若两侧都退化成"只取第一条"，offline 会与 one 逐位相同 ⇒ 本断言红。）
    let single = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
        &Default::default(),
    )
    .expect("单条 SFZ 必须成功");
    let mut one = vec![0.0f32; 512];
    single
        .render_note_into(&sample, 60, 100, &mut one, 48_000.0)
        .expect("离线渲染不得报错");
    let layered_max_abs_diff = offline
        .iter()
        .zip(one.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        layered_max_abs_diff > 0.0,
        "两条都匹配 ⇒ 叠加结果必须与单条不同（实得 {layered_max_abs_diff:e}）"
    );

    // ④ 反向对照 3：不命中的音符（key=64，三条区域都不覆盖）两侧都必须静音
    let mut offline_miss = vec![0.0f32; 512];
    sfz.render_note_into(&sample, 64, 100, &mut offline_miss, 48_000.0)
        .expect("离线渲染不得报错");
    let mut flat_miss = vec![0.0f32; 512];
    prepared.render_block_into(64, 100, &mut flat_miss);
    assert!(
        offline_miss.iter().all(|s| *s == 0.0),
        "不命中的音符离线侧必须静音"
    );
    assert!(
        flat_miss.iter().all(|s| *s == 0.0),
        "不命中的音符摊平侧必须静音"
    );
}

/// ⭐ 读数：摊平一次的耗时与内存 ＋ 回调侧每块（512 帧）耗时（8 秒、32 次触发，与 12.373 ms 同口径）。
#[test]
fn prepare_and_callback_readings() {
    let sfz = SfzInstrument::load(
        "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
        &Default::default(),
    )
    .expect("最小 SFZ 必须成功");
    let sample = InMemorySample::sine(440.0, 48_000.0, 480);

    let start = Instant::now();
    let prepared = PreparedSfz::prepare(&sfz, &sample, 48_000.0).expect("摊平不得报错");
    let prepare_us = start.elapsed().as_micros();
    println!(
        "[perf] 摊平一次：{prepare_us} µs；内存 = {} 字节（区域 {} × 结构 {} B ＋ PCM {} 帧 × 4 B）",
        prepared.memory_bytes(),
        prepared.region_count(),
        std::mem::size_of::<yeban_engine::sfz_instrument::PreparedRegion>(),
        sample.pcm.len()
    );

    // 回调侧：8 秒、512 帧块、每 12,000 帧触发一次（与离线 12.373 ms 的口径可比）
    const BLOCK: usize = 512;
    const FIRE_EVERY: usize = 12_000;
    let frames = 48_000usize * 8;
    let mut out = vec![0.0f32; BLOCK];
    let mut triggers = 0usize;
    let start = Instant::now();
    let mut rendered = 0usize;
    while rendered < frames {
        let step = BLOCK.min(frames - rendered);
        out[..step].fill(0.0);
        if rendered % FIRE_EVERY < BLOCK
            && prepared.render_block_into(60, 100, &mut out[..step]) > 0
        {
            triggers += 1;
        }
        rendered += step;
    }
    let wall_ms = start.elapsed().as_secs_f64() * 1e3;
    let per_block_us = wall_ms * 1e3 / (frames as f64 / BLOCK as f64);
    println!(
        "[perf] 摊平回调侧 8 秒（{frames} 帧、块 {BLOCK}、触发 {triggers} 次）：墙钟 = {wall_ms:.3} ms ⇒ 每块 = {per_block_us:.3} µs"
    );
    assert_eq!(rendered, frames, "必须真正渲染 {frames} 帧");
    assert!(triggers > 0, "必须有触发");
}

/// ⭐ A④ 剩余件 2：**输入回调体每块耗时**。
///
/// 口径（写明）：块 = **512 帧 × 2 声道**（1,024 个 `f32`）；迭代 **N = 1000**；
/// 计时用 `std::time::Instant`；报**最小值与中位数**（µs/块），⛔ 不写"A 比 B 快"
/// （与摊平回调侧的口径各自独立）。
#[test]
fn input_callback_body_per_block_cost() {
    use yeban_engine::record::{push_input_block, record_channel};
    const BLOCK_FRAMES: usize = 512;
    const CHANNELS: u16 = 2;
    const ITERATIONS: usize = 1000;
    let (mut sender, mut receiver) = record_channel(4096, CHANNELS);
    let block: Vec<f32> = (0..BLOCK_FRAMES * usize::from(CHANNELS))
        .map(|i| (i % 61) as f32 / 61.0)
        .collect();

    let mut samples_us: Vec<f64> = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        // 每轮先排空，保证测的是"整帧入环"而不是"环满即丢"
        let _ = receiver.drain_frames(BLOCK_FRAMES, |_| {});
        let start = Instant::now();
        let accepted = push_input_block(&mut sender, &block, CHANNELS);
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        assert_eq!(accepted, BLOCK_FRAMES, "空环必须整块接受");
        samples_us.push(elapsed);
    }
    samples_us.sort_by(|a, b| a.partial_cmp(b).expect("有限值"));
    let min_us = samples_us[0];
    let median_us = samples_us[samples_us.len() / 2];
    println!(
        "[perf] 输入回调体：块 = {BLOCK_FRAMES} 帧 × {CHANNELS} 声道，N = {ITERATIONS}，Instant，\
         最小 = {min_us:.3} µs/块，中位数 = {median_us:.3} µs/块（丢弃 = {} 帧）",
        sender.dropped_frames()
    );
    assert!(min_us > 0.0, "计时必须为正（否则说明块没被处理）");
    assert_eq!(sender.dropped_frames(), 0, "每轮排空后不得丢帧");
}
