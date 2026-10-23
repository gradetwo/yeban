//! 任务单 `#249` / `[ADR-0010 D4]` 的**实时**判据：音频片段按遍展开并循环。
//!
//! # 判据与读数的对应
//!
//! | 判据 | 内容 |
//! | :--- | :--- |
//! | [`an_audio_loop_is_expanded_into_two_spans_and_both_segments_peak_equally`] | `0..960 / 摆放 1920` ⇒ 一个摆放展开成 2 个 span；两段峰值相等（修前第 2 段静音） |
//! | [`the_two_counts_are_reported_separately`] | "摆放数"仍 1、"span 数"变 2（两个口径分开报） |
//! | [`source_offset_points_each_pass_at_the_loop_region`] | 递增振幅素材 ⇒ 每遍读的素材段由 `source_offset` 指明（pickup 与三遍循环各自的精确样本值） |
//! | [`disabled_covering_and_beyond_are_bit_identical_to_no_loop`] | 正对照：关掉 / 恰好覆盖 / 越过末端 ⇒ 排程与**全量子逐位**等于不循环 |
//!
//! # 口径
//!
//! 引擎的逐轨暂存是**单声道、声相之前**，声相由后续一级施加。本文件的夹具把轨推到
//! **硬左**（`pan = -1` ⇒ `pan_gains(-1) == (1, 0)`）⇒ 左声道峰值就是"素材值 × 增益"，
//! 与 MCP / app 的 `pan == 0 → (gain, gain)` 落在同一条可直读的网格上（三端直接比数值）。
//! 这⛔ 不是在回避引擎既有声相：`pan == 0` 的居中 −3.01 dB 差异另有既有判据登记
//! （见 `crates/yeban-engine/src/audio_clip.rs` 的模块文档）。

mod support;

use std::sync::Arc;

use yeban_engine::audio_clip::{AudioPcm, project_audio_clips};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{LoopConfig, YebanProjectV1};

use support::{Fixture, Render, audio_clip_asset, audio_clip_project_at, render_snapshot};

fn fixture_rate(project: &YebanProjectV1) -> u32 {
    project.audio_config.sample_rate.hz()
}

/// 一段**交织单声道**常量 PCM（`frames` 帧、每帧 `value`）。
fn mono_pcm(value: f32, frames: usize, sample_rate: u32) -> AudioPcm {
    let samples: Arc<[f32]> = Arc::from(vec![value; frames].into_boxed_slice());
    AudioPcm::new(samples, 1, sample_rate).expect("单声道常量 PCM 必须合法")
}

/// 递增振幅素材：第 `n` 帧 = `(n + 1) × 1e-6`（＋1 让第一帧非零 ⇒ 位置即读数）。
fn ramp_pcm(frames: usize, sample_rate: u32) -> AudioPcm {
    let samples: Vec<f32> = (0..frames)
        .map(|frame| ((frame as f32) + 1.0) * 1e-6)
        .collect();
    AudioPcm::new(Arc::from(samples.into_boxed_slice()), 1, sample_rate)
        .expect("单声道斜坡 PCM 必须合法")
}

fn snapshot_with_pcm(project: &YebanProjectV1, pcm: AudioPcm) -> EngineSnapshot {
    EngineSnapshot::from_project(project, 1)
        .expect("夹具工程必须能编译成快照")
        .with_audio_asset(&audio_clip_asset(), pcm)
        .expect("注入的 PCM 必须被接受")
}

/// 给夹具**唯一**的音频摆放设置循环配置。
fn set_loop(fixture: &mut Fixture, loop_config: LoopConfig) {
    let track = fixture.track;
    let placement = fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条音频轨")
        .clips
        .values_mut()
        .next()
        .expect("夹具里必须有一个摆放");
    placement.loop_config = loop_config;
}

/// 把夹具唯一的音频轨推到**硬左**（`pan = -1`）。
///
/// 这样引擎与 MCP / app 的"左声道段峰值"落在同一条可直读的网格上：
/// `pan_gains(-1) == (1, 0)` ⇒ 左声道峰值 = 素材值 × 增益，⛔ 不再带居中 −3.01 dB。
/// 三端一致的判据因此可以**直接比数值**（见报告 §2）。
fn set_hard_left(fixture: &mut Fixture) {
    let track = fixture.track;
    fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条音频轨")
        .pan = -1.0;
}

fn looping(start_tick: u64, end_tick: u64) -> LoopConfig {
    LoopConfig {
        enabled: true,
        start_tick,
        end_tick,
    }
}

/// 一段 `[from, to)` 的峰值（左声道）。
fn peak_in(render: &Render, from: usize, to: usize) -> f32 {
    render.left[from..to]
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

/// `#249` 的主判据：一个**循环**音频摆放 ⇒ 展开成 2 个 span，两段峰值相等。
///
/// 夹具：素材 0.5 常量 / 24 000 帧；摆放 `0..1920` tick（48 000 帧）；
/// 循环区间 `0..960` tick（24 000 帧）⇒ `N = 2`，两遍都搬素材的 `[0, 24 000)`。
///
/// ⚠ **修前**：引擎不读 `loop_config`（`AudioClipSpan` 没有 `source_offset`），
/// 这个夹具的实际行为**逐位等于关掉循环** ⇒ 第 2 段静音。那一条由下面的
/// "关掉循环"控制腿现场复现（段峰值 `[0.5, 0]`，跳过 33 帧限制器延迟）。
#[test]
fn an_audio_loop_is_expanded_into_two_spans_and_both_segments_peak_equally() {
    let mut fixture = audio_clip_project_at(0, 1_920, 0.0);
    set_loop(&mut fixture, looping(0, 960));
    set_hard_left(&mut fixture);
    let rate = fixture_rate(&fixture.project);
    let segment = 24_000usize;
    let snapshot = snapshot_with_pcm(&fixture.project, mono_pcm(0.5, segment, rate));

    // 两个口径的**事实**（不是从输出反推）。
    assert_eq!(snapshot.audio_placements(), 1, "工程里只有一个音频摆放");
    assert_eq!(
        snapshot.audio_spans(),
        2,
        "循环 0..960 / 摆放 1920 ⇒ 2 个 span"
    );
    assert_eq!(snapshot.audio_pcm_spans(), 2, "两个 span 都必须注入 PCM");
    let schedule = snapshot
        .audio_clip_schedule(&fixture.track)
        .expect("音频轨必须在表里");
    assert_eq!(schedule.placements(), 1, "摆放数不随展开增长");
    assert_eq!(schedule.len(), 2, "span 数 = 2");
    let spans: Vec<(u64, u64, u64)> = schedule
        .spans()
        .iter()
        .map(|span| (span.start(), span.end(), span.source_offset()))
        .collect();
    assert_eq!(
        spans,
        vec![(0, 24_000, 0), (24_000, 48_000, 0)],
        "两遍都从素材帧 0 起读（循环区间从 0 开始）"
    );

    let rendered = render_snapshot(snapshot, 400);
    // 硬左 ⇒ 左声道增益 1.0（`pan_gains(-1) == (1, 0)`）。
    let expected = 0.5;
    let first = peak_in(&rendered, 0, segment);
    let second = peak_in(&rendered, segment, 2 * segment);
    let relative = (first - second).abs() / expected;
    println!(
        "[#249/loop] 两段峰值（逐字）：第1段={first:.9} 第2段={second:.9} \
         期望={expected:.9} 相对差={relative:e}；span 表={spans:?}"
    );
    assert!(
        (first - expected).abs() < 1e-6,
        "第 1 段峰值必须 = 素材 0.5 × 硬左增益 1.0（实测 {first}）"
    );
    assert!(
        (second - expected).abs() < 1e-6,
        "第 2 段峰值必须与第 1 段相同（实测 {second}，期望 {expected}）"
    );
    assert!(
        relative <= 1e-6,
        "两段峰值的相对差必须 ≤ 1e-6（实测 {relative:e}）"
    );

    // 控制腿（＝修前行为的现场复现）：同一夹具**关掉**循环 ⇒ 一个 span、第 2 段静音。
    let mut plain = audio_clip_project_at(0, 1_920, 0.0);
    set_loop(&mut plain, LoopConfig::default());
    set_hard_left(&mut plain);
    let plain_snapshot = snapshot_with_pcm(&plain.project, mono_pcm(0.5, segment, rate));
    assert_eq!(plain_snapshot.audio_placements(), 1);
    assert_eq!(plain_snapshot.audio_spans(), 1, "关掉循环 ⇒ 单 span");
    let plain_render = render_snapshot(plain_snapshot, 400);
    let plain_first = peak_in(&plain_render, 0, segment);
    // ⭐ `#302`：默认档限制器关 ⇒ 没有 33 帧前瞻延迟；第 2 段从 `segment` 起
    // 就必须逐位静音（时序前移 33 帧是**修对**）。
    let plain_second = peak_in(&plain_render, segment, 2 * segment);
    println!(
        "[#249/loop-control] 关掉循环（＝修前口径）两段峰值：{plain_first:.9} / \
         [{}, {}) 峰值 {plain_second:.9}（无限制器延迟）",
        segment,
        2 * segment
    );
    assert!(
        (plain_first - expected).abs() < 1e-6,
        "控制腿第 1 段仍应是 0.5（硬左增益 1.0）"
    );
    assert_eq!(
        plain_second, 0.0,
        "控制腿第 2 段必须逐位静音（素材只有 0.5 秒）"
    );
}

/// 两个口径**分开报**：`placements()` / `audio_placements()` 与
/// `len()` / `audio_spans()` 在同一个循环工程上给出 `1` 与 `2`。
#[test]
fn the_two_counts_are_reported_separately() {
    let mut fixture = audio_clip_project_at(0, 1_920, 0.0);
    set_loop(&mut fixture, looping(0, 960));
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    let schedule = snapshot
        .audio_clip_schedule(&fixture.track)
        .expect("音频轨必须在表里");
    println!(
        "[#249/counts] 摆放数：snapshot={} schedule={}；span 数：snapshot={} schedule={}",
        snapshot.audio_placements(),
        schedule.placements(),
        snapshot.audio_spans(),
        schedule.len()
    );
    assert_eq!(snapshot.audio_placements(), 1, "摆放数仍是 1");
    assert_eq!(schedule.placements(), 1, "摆放数仍是 1");
    assert_eq!(snapshot.audio_spans(), 2, "span 数变成 2");
    assert_eq!(schedule.len(), 2, "span 数变成 2");
}

/// `source_offset` 真的对：递增振幅素材 ⇒ 每一遍读的素材段可逐样本读出。
///
/// 夹具：摆放 `0..1920`、循环区间 `480..960` ⇒ pickup `[0, 480)` ＋ 三遍
/// `[480, 960)`。素材第 `n` 帧 = `(n+1)×1e-6`。
///
/// 读数（绝对帧 − 33 帧限制器前瞻 = 排列位置）：
///
/// * pickup 读素材 `0..12000`；
/// * 三遍都读素材 `12000..24000`（`source_offset = 12000`）；
/// * 因此三遍在各自段内**同一相对位置**的样本值逐位相同，且**不同于** pickup
///   同相对位置的值（`18001e-6` vs `5001e-6`）。
#[test]
fn source_offset_points_each_pass_at_the_loop_region() {
    let mut fixture = audio_clip_project_at(0, 1_920, 0.0);
    set_loop(&mut fixture, looping(480, 960));
    set_hard_left(&mut fixture);
    let rate = fixture_rate(&fixture.project);
    let snapshot = snapshot_with_pcm(&fixture.project, ramp_pcm(48_000, rate));

    let schedule = snapshot
        .audio_clip_schedule(&fixture.track)
        .expect("音频轨必须在表里");
    let spans: Vec<(u64, u64, u64)> = schedule
        .spans()
        .iter()
        .map(|span| (span.start(), span.end(), span.source_offset()))
        .collect();
    println!("[#249/offset] span 表（start, end, source_offset）={spans:?}");
    assert_eq!(
        spans,
        vec![
            (0, 12_000, 0),
            (12_000, 24_000, 12_000),
            (24_000, 36_000, 12_000),
            (36_000, 48_000, 12_000),
        ],
        "pickup + 三遍循环区间；三遍的 source_offset 都指向素材帧 12000"
    );

    let rendered = render_snapshot(snapshot, 400);
    // ⭐ `#302`：默认档去掉母线限制器的 33 帧环 ⇒ 监听输出**不再后移 33 帧**，
    // 输出绝对帧直接等于排程帧。时序前移 33 帧是**修对**（监听与导出在样本层对齐）。
    // 硬左 ⇒ 左声道增益 1.0。
    let pan = 1.0_f32;
    let read = |asset_frame: usize| rendered.left[asset_frame];
    let cases = [
        ("pickup @5000", 5_000usize, 5_001.0 * 1e-6 * pan),
        ("pass1 @18000", 18_000, 18_001.0 * 1e-6 * pan),
        ("pass2 @18000", 30_000, 18_001.0 * 1e-6 * pan),
        ("pass3 @18000", 42_000, 18_001.0 * 1e-6 * pan),
    ];
    let mut lines = Vec::new();
    for (label, position, expected) in cases {
        let actual = read(position);
        let error = (actual - expected).abs();
        lines.push(format!(
            "{label}: 输出帧 {position} ⇒ {actual:.9}（期望 {expected:.9}，误差 {error:e}）"
        ));
        assert!(
            error < 1e-7,
            "{label} 的样本值必须由 source_offset 决定（实测 {actual}，期望 {expected}）"
        );
    }
    println!("[#249/offset] 逐样本读数：{}", lines.join("；"));

    // 更强的反证：若忽略 source_offset（＝修前），pass2 会读素材 5000 那一带
    // 而不是 18000 那一带 ⇒ 值与 pickup 同量级。这里钉住"不是那个值"。
    let without_offset = 5_001.0 * 1e-6 * pan;
    assert!(
        (read(30_000) - without_offset).abs() > 1e-4,
        "pass2 必须读素材 12000..（不是 0..）；实测 {} 与修前值 {without_offset} 太近",
        read(30_000)
    );
}

/// 正对照：关掉 / 恰好覆盖 / 越过末端 ⇒ 排程与**全量子逐位**等于不循环。
///
/// 这三类配置在 `ClipPlacement::loop_repeat()` 里都是 `None` ⇒ 单 span、
/// `source_offset == 0` ⇒ 与 `#249` 之前**一个字节都不动**。
#[test]
fn disabled_covering_and_beyond_are_bit_identical_to_no_loop() {
    let rate = fixture_rate(&audio_clip_project_at(0, 1_920, 0.0).project);
    let mut baseline = audio_clip_project_at(0, 1_920, 0.0);
    set_loop(&mut baseline, LoopConfig::default());
    set_hard_left(&mut baseline);
    let baseline_render = render_snapshot(
        snapshot_with_pcm(&baseline.project, mono_pcm(0.5, 24_000, rate)),
        400,
    );
    let baseline_schedule = project_audio_clips(&baseline.project)
        .remove(&baseline.track)
        .expect("基线必须有一条音频轨");
    let baseline_fingerprint = baseline_render.fingerprint();
    println!(
        "[#249/positive] 基线（不循环）：span 数={} 摆放数={} 指纹=0x{baseline_fingerprint:016x}",
        baseline_schedule.len(),
        baseline_schedule.placements()
    );
    assert_eq!(baseline_schedule.len(), 1);
    assert_eq!(baseline_schedule.placements(), 1);

    for (label, loop_config) in [
        ("关掉", LoopConfig::default()),
        ("恰好覆盖 0..1920", looping(0, 1_920)),
        ("越过末端 0..3840", looping(0, 3_840)),
        ("非法区间 960..960", looping(960, 960)),
    ] {
        let mut fixture = audio_clip_project_at(0, 1_920, 0.0);
        set_loop(&mut fixture, loop_config);
        set_hard_left(&mut fixture);
        let schedule = project_audio_clips(&fixture.project)
            .remove(&fixture.track)
            .expect("音频轨必须在表里");
        let render = render_snapshot(
            snapshot_with_pcm(&fixture.project, mono_pcm(0.5, 24_000, rate)),
            400,
        );
        let fingerprint = render.fingerprint();
        println!(
            "[#249/positive] {label}：span 数={} 摆放数={} 指纹=0x{fingerprint:016x} \
             与基线逐位相同={}",
            schedule.len(),
            schedule.placements(),
            render.left_bits() == baseline_render.left_bits()
        );
        assert_eq!(
            schedule, baseline_schedule,
            "{label} 的排程必须逐位等于不循环"
        );
        assert_eq!(
            render.left_bits(),
            baseline_render.left_bits(),
            "{label} 的全量子左声道必须逐位等于不循环"
        );
        assert_eq!(
            fingerprint, baseline_fingerprint,
            "{label} 的指纹必须等于基线"
        );
        assert!(
            schedule
                .spans()
                .iter()
                .all(|span| span.source_offset() == 0),
            "{label} 的每个 span 的 source_offset 都必须是 0"
        );
    }
}
