//! **SFZ 声部消费弯音曲线（`pitch_bend_curve`）的判据**（清单 `05_yeban.md` #16 修复）。
//!
//! 缺陷（修复前）：弯音曲线只被**复音合成器**声部消费（`synth.rs` 的探针路径），
//! `SfzVoicePool` 全文不读形状下标 ⇒ SFZ 轨的音符**参数被接受、渲染无声地按
//! 没弯音出声**，且全仓无任何警告。
//!
//! 修复口径：采样器按与合成器**同一语义**消费曲线 —— 同一探针
//! （调度表旁表 `NoteSchedule::cents_at`）、同一时间基准（距音符起始帧的样本
//! 偏移）、同一 cents 口径与乘子 `2^(cents/1200)`；合成点与 portamento 的块内
//! 速率 ramp 同一处（`render_slot` 第 3 步：块首/块末各求一次、块内线性 ramp）。
//! ⛔ 不另立第二套曲线语义。
//!
//! # 判据表（"怎么变红" ＝ 注入）
//!
//! | # | 判据 | 怎么变红 |
//! | :--- | :--- | :--- |
//! | P1 | 恒定 +100 cents 探针 ⇒ 440 Hz 正弦样本实测 ≈ **466.16 Hz**；探针收到的形状下标 ＝ 触发时给的、首个 `local` ＝ 0 | 乘子公式错 / 形状没进池 / 局部时间原点错 |
//! | P2 | 斜坡探针（0→+1200 cents / 1 s）：头窗 ≈ 440 Hz、**中窗 ≈ 627 Hz**、尾窗 ≈ 880 Hz | 只在起音求一次值（中窗会停在低位）/ 局部时间不推进 |
//! | P3 | 形状 ＝ 哨兵 ＋ 探针在场 ⇒ 探针**一次都不被调用**，输出与旧入口基线**逐位相同** | 哨兵槽也调探针 / 无曲线工程被改声 |
//! | P4 | 真形状 ＋ 探针恒回 0.0 ⇒ 输出与基线**逐位相同**（乘子恰 1.0） | 0 cents 也改速率（浮点偏移） |
//! | P5 | 端到端：工程音符带 `[(0, 100)]` 弯音 + 快照注入 SFZ ⇒ 主频 ≈ 466.16 Hz；无曲线对照 ≈ 440 Hz | 分发层没把形状/探针送进 SFZ 支路 |
//!
//! 测频方法与 `tests/portamento.rs` 同一份：纯正弦 ⇒ 过零时刻线性插值求平均
//! 周期（实测精度 < 1 Hz）；端到端用 `tests/pitch_bend.rs` 的加窗 DFT 扫描。

mod support;

use std::cell::Cell;

use support::{NoteSpec, note_project, render_snapshot};
use yeban_engine::sfz_instrument::{
    InMemorySample, PreparedSfz, SfzInstrument, SfzPlan, SfzVoicePool,
};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::NO_PITCH_SHAPE;
use yeban_model::{ClipContent, MidiNote, YebanProjectV1};

/// 采样率（Hz）。
const HZ: f32 = 48_000.0;
/// 内存样本长度（帧）：2 秒（弯音后速率 > 1，1 秒的渲染要吃掉 > 48000 源帧）。
const SAMPLE_FRAMES: usize = 96_000;
/// 池级渲染块长（与 `tests/portamento.rs` 同值）。
const BLOCK: usize = 4096;
/// 测频窗口（帧）：10 ms。
const WINDOW: usize = 480;
/// 夹具区域：key 60、键心 60 ⇒ 音符 60 按样本原速（440 Hz）发声。
const REGION: &str = "<region> sample=a.wav pitch_keycenter=60\n";

/// 摊平一支 sfz（48 kHz、440 Hz 正弦样本）。与 `tests/portamento.rs::prepare` 同款。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功")
}

/// 过零率推出来的频率（Hz）。与 `tests/portamento.rs::frequency_of` **逐字同款**
/// （纯正弦下用过零时刻的线性插值求平均周期，实测精度 < 1 Hz）。
fn frequency_of(window: &[f32]) -> f32 {
    let mut crossings: Vec<f32> = Vec::new();
    for index in 1..window.len() {
        let (before, after) = (window[index - 1], window[index]);
        // 只取**正向**过零（负 → 正）：一个周期恰好一次 ⇒ `f = (n − 1) × 采样率 / 跨度`。
        if before < 0.0 && after >= 0.0 {
            let magnitude = before.abs() + after.abs();
            if magnitude > 0.0 {
                let offset = before.abs() / magnitude;
                crossings.push((index - 1) as f32 + offset);
            }
        }
    }
    if crossings.len() < 2 {
        return 0.0;
    }
    let span = crossings[crossings.len() - 1] - crossings[0];
    if span <= 0.0 {
        return 0.0;
    }
    (crossings.len() - 1) as f32 * HZ / span
}

/// 用带探针的入口连渲 `blocks` 块，位置单调递增（弯音局部时间的基准）。
fn render_probed(
    pool: &mut SfzVoicePool,
    plan: &PreparedSfz,
    blocks: usize,
    probe: Option<&dyn Fn(u16, u64) -> f32>,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(blocks * BLOCK);
    let mut block = vec![0.0f32; BLOCK];
    for index in 0..blocks {
        // ⚠ 池的契约是**混入**调用方给的缓冲（`+=`）⇒ 每块先清零，
        // ⛔ 不许把上一块的残响叠进来（叠加会让测频读到假频率）。
        block.fill(0.0);
        pool.render_into_with_probes(plan, (index * BLOCK) as u64, &mut block, None, probe);
        out.extend_from_slice(&block);
    }
    out
}

/// 用**旧入口**连渲 `blocks` 块（逐位对照的基线：接线前的公开渲染面）。
fn render_baseline(pool: &mut SfzVoicePool, plan: &PreparedSfz, blocks: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(blocks * BLOCK);
    let mut block = vec![0.0f32; BLOCK];
    for index in 0..blocks {
        block.fill(0.0);
        pool.render_into_with_cc(plan, (index * BLOCK) as u64, &mut block, None);
        out.extend_from_slice(&block);
    }
    out
}

/// 逐位比较（`f32::to_bits`：⛔ 不用 `==`，`-0.0 == 0.0` 会放过符号位差异）。
fn assert_bit_identical(label: &str, a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len(), "{label}：长度必须一致");
    let different = a
        .iter()
        .zip(b.iter())
        .filter(|(x, y)| x.to_bits() != y.to_bits())
        .count();
    assert_eq!(
        different, 0,
        "{label}：必须逐位相同（不同样本数 = {different}）"
    );
}

/// P1：恒定 +100 cents ⇒ 实测音高上移约 100 音分（440 → 466.16 Hz）。
#[test]
fn a_constant_100_cent_bend_raises_the_sampler_pitch_by_100_cents() {
    let plan = prepare(REGION);
    let seen_shape = Cell::new(u16::MAX);
    let first_local = Cell::new(u64::MAX);
    let probe = |shape: u16, local: u64| -> f32 {
        if first_local.get() == u64::MAX {
            first_local.set(local);
            seen_shape.set(shape);
        }
        100.0
    };
    let mut pool = SfzVoicePool::new(HZ);
    // 形状下标故意用 5（不是 0）：钉住"触发时给的下标原样到达探针"。
    pool.trigger_note_with_pitch(&plan, 60, 127, 0, u64::MAX, 1.0, None, 5, 0.0);
    let out = render_probed(&mut pool, &plan, 4, Some(&probe));
    let measured = frequency_of(&out[2 * BLOCK..2 * BLOCK + WINDOW]);
    println!("[sfz-bend/P1] +100 cents 实测 {measured:.2} Hz（期望 466.16）");
    assert_eq!(seen_shape.get(), 5, "探针必须收到触发时给的形状下标");
    assert_eq!(first_local.get(), 0, "局部时间的原点必须是音符起始帧");
    assert!(
        (measured - 466.16).abs() < 8.0,
        "+100 cents 必须把 440 Hz 抬到 ≈466.16 Hz，实读 {measured} Hz"
    );
}

/// P2：斜坡弯音中途音高跟随（0 → +1200 cents / 1 s，三窗读数单调爬升）。
#[test]
fn a_ramp_bend_is_followed_through_the_middle_of_the_note() {
    let plan = prepare(REGION);
    // 探针即"曲线本身"：局部样本 → 线性 0..1200 cents（1 秒走完，之后钳住）。
    let probe = |_shape: u16, local: u64| -> f32 { ((local as f32 / HZ) * 1200.0).min(1200.0) };
    let mut pool = SfzVoicePool::new(HZ);
    pool.trigger_note_with_pitch(&plan, 60, 127, 0, u64::MAX, 1.0, None, 2, 0.0);
    let out = render_probed(&mut pool, &plan, 13, Some(&probe));
    let head = frequency_of(&out[0..WINDOW]);
    let middle = frequency_of(&out[6 * BLOCK..6 * BLOCK + WINDOW]);
    let tail = frequency_of(&out[12 * BLOCK..12 * BLOCK + WINDOW]);
    println!("[sfz-bend/P2] 斜坡三窗：头 {head:.1} / 中 {middle:.1} / 尾 {tail:.1} Hz");
    assert!(
        (head - 440.0).abs() < 15.0,
        "斜坡起点必须 ≈440 Hz，实读 {head} Hz"
    );
    // 中窗：local ≈ 24576 ⇒ ≈614 cents ⇒ 440 × 2^(614/1200) ≈ 627 Hz。
    assert!(
        (middle - 627.0).abs() < 30.0,
        "斜坡中途必须跟到 ≈627 Hz，实读 {middle} Hz"
    );
    assert!(
        middle - head > 120.0,
        "中窗必须显著高于头窗（跟随，不是起音求一次值）：{head} → {middle}"
    );
    assert!(
        (tail - 880.0).abs() < 20.0,
        "斜坡终点必须 ≈880 Hz，实读 {tail} Hz"
    );
}

/// P3：无弯音（形状哨兵）⇒ 探针一次都不调、输出与旧入口基线逐位相同。
#[test]
fn the_sentinel_shape_never_calls_the_probe_and_matches_the_baseline_bit_for_bit() {
    let plan = prepare(REGION);
    let mut baseline_pool = SfzVoicePool::new(HZ);
    baseline_pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None);
    let baseline = render_baseline(&mut baseline_pool, &plan, 4);
    assert!(
        baseline.iter().filter(|s| **s != 0.0).count() > 1000,
        "基线必须真的出声（否则逐位相同是空转）"
    );

    let calls = Cell::new(0usize);
    let probe = |_shape: u16, _local: u64| -> f32 {
        calls.set(calls.get() + 1);
        100.0
    };
    let mut pool = SfzVoicePool::new(HZ);
    // 起始帧故意给非零（12345）：哨兵形状下它**从不被读**，输出仍须逐位相同。
    pool.trigger_note_with_pitch(
        &plan,
        60,
        127,
        12_345,
        u64::MAX,
        1.0,
        None,
        NO_PITCH_SHAPE,
        0.0,
    );
    let out = render_probed(&mut pool, &plan, 4, Some(&probe));
    assert_eq!(calls.get(), 0, "哨兵形状 ⇒ 探针一次都不许被调用");
    assert_bit_identical("P3 哨兵+探针", &out, &baseline);
}

/// P4：真形状 ＋ 探针恒回 0.0 ⇒ 乘子恰 1.0，输出与基线逐位相同。
#[test]
fn a_zero_cent_probe_matches_the_baseline_bit_for_bit() {
    let plan = prepare(REGION);
    let mut baseline_pool = SfzVoicePool::new(HZ);
    baseline_pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None);
    let baseline = render_baseline(&mut baseline_pool, &plan, 4);

    let probe = |_shape: u16, _local: u64| -> f32 { 0.0 };
    let mut pool = SfzVoicePool::new(HZ);
    pool.trigger_note_with_pitch(&plan, 60, 127, 0, u64::MAX, 1.0, None, 9, 0.0);
    let out = render_probed(&mut pool, &plan, 4, Some(&probe));
    assert_bit_identical("P4 零音分探针", &out, &baseline);
}

/// 改夹具里**每一个** MIDI 音符（与 `tests/pitch_bend.rs::with_note` 同款）。
fn with_note<F>(project: YebanProjectV1, mutate: F) -> YebanProjectV1
where
    F: Fn(&mut MidiNote),
{
    let mut project = project;
    for clip in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut clip.content {
            for note in notes.values_mut() {
                mutate(note);
            }
        }
    }
    project.validate().expect("夹具必须合法");
    project
}

/// 单 bin 的加窗（Hann）DFT 幅度。与 `tests/pitch_bend.rs::magnitude_at` 同款。
fn magnitude_at(samples: &[f32], freq_hz: f64, sample_rate: f64) -> f64 {
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    let n = samples.len();
    for (index, sample) in samples.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let position = index as f64;
        #[allow(clippy::cast_precision_loss)]
        let hann = 0.5 - 0.5 * (core::f64::consts::TAU * position / n as f64).cos();
        let phase = core::f64::consts::TAU * freq_hz * position / sample_rate;
        re += hann * f64::from(*sample) * phase.cos();
        im += hann * f64::from(*sample) * phase.sin();
    }
    (re * re + im * im).sqrt() / n as f64 * 2.0
}

/// 在 `[lo, hi]` 里按 `step` 扫出**幅度最大**的频率（Hz）。
fn dominant_frequency(samples: &[f32], lo: f64, hi: f64, step: f64) -> f64 {
    let mut best = lo;
    let mut best_magnitude = f64::NEG_INFINITY;
    let mut freq = lo;
    while freq <= hi {
        let magnitude = magnitude_at(samples, freq, f64::from(HZ));
        if magnitude > best_magnitude {
            best_magnitude = magnitude;
            best = freq;
        }
        freq += step;
    }
    best
}

/// P5：端到端 —— 工程弯音曲线经快照分发真的弯了 SFZ 轨（产品路径全链）。
#[test]
fn end_to_end_a_bent_note_on_a_sampler_track_renders_bent() {
    // 1 秒音符（1920 tick = 48000 样本），音高 60。
    let fixture = note_project(&[NoteSpec::at(0, 1_920, 60, 100)]);
    let track = fixture.track;
    let plan = SfzPlan::prepare(
        &SfzInstrument::load(
            "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
            &Default::default(),
        )
        .expect("SFZ 必须解析成功"),
        &InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES),
        HZ,
    )
    .expect("摊平必须成功");

    // 对照臂：无弯音曲线 ⇒ ≈440 Hz。
    let plain_project = fixture.project.clone();
    let plain = EngineSnapshot::from_project(&plain_project, 1)
        .expect("快照")
        .with_sfz(track, plan.clone());
    let plain_out = render_snapshot(plain, 375);
    let plain_freq = dominant_frequency(&plain_out.left[24_000..], 400.0, 520.0, 1.0);

    // 弯音臂：恒定 +100 cents（首点在 tick 0 ⇒ 全程 100）。
    let bent_project = with_note(fixture.project, |note| {
        note.pitch_bend_curve = vec![(0, 100)];
    });
    let bent = EngineSnapshot::from_project(&bent_project, 1)
        .expect("快照")
        .with_sfz(track, plan);
    let bent_out = render_snapshot(bent, 375);
    let bent_freq = dominant_frequency(&bent_out.left[24_000..], 400.0, 520.0, 1.0);

    println!("[sfz-bend/P5] 端到端主频：无弯音 {plain_freq:.1} Hz / +100 cents {bent_freq:.1} Hz");
    assert!(
        plain_out.left.iter().any(|s| *s != 0.0),
        "对照臂必须真的出声（否则主频读数无意义）"
    );
    assert!(
        (plain_freq - 440.0).abs() < 6.0,
        "无弯音对照必须 ≈440 Hz，实读 {plain_freq} Hz"
    );
    assert!(
        (bent_freq - 466.16).abs() < 6.0,
        "SFZ 轨的 +100 cents 弯音必须端到端生效（≈466.16 Hz），实读 {bent_freq} Hz"
    );
}
