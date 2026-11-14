//! **SFZ 声部消费音符滑音（`MidiNote::slide` / `SlideConfig`）的判据**
//! （issue_report `P2-1` 修复）。
//!
//! 缺陷（报告口径）：SFZ 规范**没有** portamento/glide opcode（检索结论登记在
//! `yeban-sfz/src/pitch_eg.rs` 头部）；引擎里 SFZ 文件级的滑音只有
//! `egN_pitch_oncc140|141` 最小子集（滑音时间写死取 region EG）。而制作入口
//! （MCP 写音符）能给音符挂的滑音是**音符级** `slide` —— 它在合成器路已消费
//! （`tests/pitch_bend.rs` 的 B3/B4），采样器路必须按**同一语义**消费。
//!
//! 修复口径（⛔ 不发明第二套语义、⛔ 不做 SFZ 文件级扩展 opcode）：
//!
//! - 语义源头唯一：`SlideConfig { duration_ticks, target_pitch, curve }`，
//!   投影期由 `snapshot::project_pitch_shape` 换算成 [`SlideSpan`]
//!   （`target_cents = (target_pitch − pitch) × 100`、时长换音符局部样本、
//!   `CurveType::ease` 是唯一 ease 实现），与弯音点同住 [`PitchShape`] 旁表；
//! - 消费通路唯一：`PitchShape::cents_at = bend + slide`（`ADR-0012` D9 相加），
//!   分发层把调度表 `cents_at` 作为**同一份探针**交给复音与 SFZ 两路，
//!   SFZ 池在 `render_slot` 第 3 步与 portamento **同一个合成点**把
//!   `2^(cents/1200)` 乘进块首/块末速率、块内线性 ramp；
//! - ⇒ 采样器**不需要新的逐声部状态**：滑音状态就是既有池级旁表
//!   （`pitch_shape` / `note_start`）里的形状下标与起始帧，与弯音同一条纪律，
//!   `SfzVoice` 仍 40 B。
//!
//! 本文件落地前，上述通路**没有任何采样器侧判据**覆盖滑音（`sfz_pitch_bend.rs`
//! 的 P1–P5 只测弯音部分）⇒ 探针若被改成只回弯音，采样器滑音会无声失效而
//! 全套判据保持绿。本文件补的就是这一组判据。
//!
//! # 判据表（"怎么变红" ＝ 注入）
//!
//! | # | 判据 | 怎么变红 |
//! | :--- | :--- | :--- |
//! | G1 | 池级：只含滑音的真形状探针（线性、+1200 cents / 24000 样本）⇒ 头窗 ≈453 Hz、中窗 ≈622 Hz、尾窗 ≈880 Hz | 滑音没进形状 / 局部时间原点错 / 只在起音求一次值 |
//! | G2 | 池级：目标音分 ＝ 0 的滑音形状 ⇒ 输出与旧入口基线**逐位相同** | 0 音分也改速率（浮点偏移） |
//! | G3 | 端到端：工程音符 `slide{960 tick → 72, Linear}` ⇒ 尾段主频 ≈ **880 Hz**、中途 ≈622 Hz、头部 < 中部 | 分发层没把滑音形状/探针送进 SFZ 支路 |
//! | G4 | 端到端：弯音 +600（恒定）＋ 滑音 +1200 ⇒ 尾段 ≈ **1244.5 Hz**（两者**相加**，D9） | 两条通路取 max/覆盖而不是相加 |
//! | G5 | 端到端：`slide{target_pitch ＝ 本音高}`（目标音分 0）⇒ 与无滑音工程**逐位相同** | 挂滑音本身改声（形状在场就动速率） |
//! | G6 | 端到端：`curve = Exponential` ⇒ 中途 ≈ **523 Hz**（t=0.5 时 ease=0.25 ⇒ +300 cents）、尾段仍精确 880 Hz | ease 被线性替代 / 端点不精确 |
//!
//! 测频方法与 `tests/sfz_pitch_bend.rs` 同一份：池级用过零插值
//! （`tests/portamento.rs` 同款），端到端用加窗 DFT 扫描
//! （`tests/pitch_bend.rs` 同款）。
//!
//! ⚠ **不证**：人耳听感；`PreparedSfz::render_block_into` /
//! `SfzInstrument::render_note_into` 两条单音预览路（无调度上下文，与弯音线
//! 同一登记，见 `sfz_pitch_bend.rs` 的范围外注明）。

mod support;

use support::{NoteSpec, note_project, render_snapshot};
use yeban_engine::sfz_instrument::{
    InMemorySample, PreparedSfz, SfzInstrument, SfzPlan, SfzVoicePool,
};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::{NoteSchedule, PitchShape, SlideSpan};
use yeban_model::{ClipContent, CurveType, MidiNote, SlideConfig, YebanProjectV1};

/// 采样率（Hz）。
const HZ: f32 = 48_000.0;
/// 内存样本长度（帧）：4 秒（滑音＋弯音叠加后速率最高 2.83×，1 秒渲染
/// 最多吃掉约 114000 源帧，2 秒样本不够、4 秒留足余量）。
const SAMPLE_FRAMES: usize = 192_000;
/// 池级渲染块长（与 `tests/portamento.rs` 同值）。
const BLOCK: usize = 4096;
/// 测频窗口（帧）：10 ms。
const WINDOW: usize = 480;
/// 夹具区域：key 60、键心 60 ⇒ 音符 60 按样本原速（440 Hz）发声。
const REGION: &str = "<region> sample=a.wav pitch_keycenter=60\n";

/// 摊平一支 sfz（48 kHz、440 Hz 正弦样本）。与 `tests/sfz_pitch_bend.rs::prepare` 同款。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功")
}

/// 过零率推出来的频率（Hz）。与 `tests/portamento.rs::frequency_of` **逐字同款**。
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

/// 用带探针的入口连渲 `blocks` 块，位置单调递增（滑音局部时间的基准）。
fn render_probed(
    pool: &mut SfzVoicePool,
    plan: &PreparedSfz,
    blocks: usize,
    probe: Option<&dyn Fn(u16, u64) -> f32>,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(blocks * BLOCK);
    let mut block = vec![0.0f32; BLOCK];
    for index in 0..blocks {
        // ⚠ 池的契约是**混入**调用方给的缓冲（`+=`）⇒ 每块先清零。
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

/// G1：池级滑音跟随 —— 只含滑音的真形状（`PitchShape::cents_at` 即产品探针
/// 的求值本体）驱动池：头/中/尾三窗单调爬升、尾窗到达目标音高。
#[test]
fn the_pool_follows_a_slide_only_shape_to_its_target() {
    let plan = prepare(REGION);
    // 滑音：24000 样本（0.5 s）内线性滑 +1200 cents（一个八度）。
    let schedule = NoteSchedule::from_sorted(Vec::new()).with_pitch_shapes(vec![PitchShape::new(
        Vec::new(),
        Some(SlideSpan::new(24_000, 1_200.0, CurveType::Linear)),
    )]);
    let probe = |shape: u16, local: u64| -> f32 { schedule.cents_at(shape, local) };
    let mut pool = SfzVoicePool::new(HZ);
    pool.trigger_note_with_pitch(&plan, 60, 127, 0, u64::MAX, 1.0, None, 0, 0.0);
    let out = render_probed(&mut pool, &plan, 13, Some(&probe));
    let head = frequency_of(&out[1_000..1_000 + WINDOW]);
    let middle = frequency_of(&out[12_000..12_000 + WINDOW]);
    let tail = frequency_of(&out[40_000..40_000 + WINDOW]);
    println!("[sfz-glide/G1] 滑音三窗：头 {head:.1} / 中 {middle:.1} / 尾 {tail:.1} Hz");
    // 头窗：local ≈ 1000 ⇒ +50 cents ⇒ ≈452.8 Hz。
    assert!(
        (head - 453.0).abs() < 30.0,
        "滑音头部必须 ≈453 Hz（刚起步），实读 {head} Hz"
    );
    // 中窗：local ≈ 12000 ⇒ +600 cents ⇒ ≈622.25 Hz（在两端之间）。
    assert!(
        (middle - 622.0).abs() < 30.0,
        "滑音中途必须 ≈622 Hz，实读 {middle} Hz"
    );
    assert!(
        middle - head > 100.0,
        "中窗必须显著高于头窗（跟随，不是起音求一次值）：{head} → {middle}"
    );
    // 尾窗：local ≈ 40000 > 24000 ⇒ 已到目标 +1200 cents ⇒ 880 Hz。
    assert!(
        (tail - 880.0).abs() < 15.0,
        "滑音尾段必须到达目标 880 Hz，实读 {tail} Hz"
    );
}

/// G2：池级零滑音 —— 目标音分 ＝ 0 的滑音形状在场，输出与基线逐位相同。
#[test]
fn a_zero_cent_slide_shape_matches_the_baseline_bit_for_bit() {
    let plan = prepare(REGION);
    let mut baseline_pool = SfzVoicePool::new(HZ);
    baseline_pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None);
    let baseline = render_baseline(&mut baseline_pool, &plan, 4);
    assert!(
        baseline.iter().filter(|s| **s != 0.0).count() > 1000,
        "基线必须真的出声（否则逐位相同是空转）"
    );

    let schedule = NoteSchedule::from_sorted(Vec::new()).with_pitch_shapes(vec![PitchShape::new(
        Vec::new(),
        Some(SlideSpan::new(24_000, 0.0, CurveType::Linear)),
    )]);
    let probe = |shape: u16, local: u64| -> f32 { schedule.cents_at(shape, local) };
    let mut pool = SfzVoicePool::new(HZ);
    pool.trigger_note_with_pitch(&plan, 60, 127, 0, u64::MAX, 1.0, None, 0, 0.0);
    let out = render_probed(&mut pool, &plan, 4, Some(&probe));
    assert_bit_identical("G2 零音分滑音", &out, &baseline);
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

/// 端到端夹具的摊平表（与 `tests/sfz_pitch_bend.rs` 的 P5 同款区域）。
fn end_to_end_plan() -> SfzPlan {
    SfzPlan::prepare(
        &SfzInstrument::load(
            "<region> sample=a.wav key=60 pitch_keycenter=60\n".to_owned(),
            &Default::default(),
        )
        .expect("SFZ 必须解析成功"),
        &InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES),
        HZ,
    )
    .expect("摊平必须成功")
}

/// 端到端渲染一支工程（1 秒音符、音高 60、`mutate` 改音符后注入 SFZ 轨）。
fn render_end_to_end(mutate: impl Fn(&mut MidiNote)) -> support::Render {
    let fixture = note_project(&[NoteSpec::at(0, 1_920, 60, 100)]);
    let track = fixture.track;
    let project = with_note(fixture.project, mutate);
    let snapshot = EngineSnapshot::from_project(&project, 1)
        .expect("快照")
        .with_sfz(track, end_to_end_plan());
    render_snapshot(snapshot, 375)
}

/// G3：端到端 —— 工程音符的 `slide` 经快照分发真的滑了 SFZ 轨（产品路径全链）。
#[test]
fn end_to_end_a_sliding_note_on_a_sampler_track_reaches_its_target() {
    let out = render_end_to_end(|note| {
        note.slide = Some(SlideConfig {
            duration_ticks: 960,
            target_pitch: 72,
            curve: CurveType::Linear,
        });
    });
    assert!(
        out.left.iter().any(|s| *s != 0.0),
        "渲染必须真的出声（否则主频读数无意义）"
    );
    // 头部（local ≈ 1000 ⇒ +50 cents ⇒ ≈453 Hz）、中途（local ≈ 12000 ⇒
    // +600 cents ⇒ ≈622 Hz）、尾段（滑音在 local 24000 结束 ⇒ 保持 +1200）。
    let head = frequency_of(&out.left[1_000..1_000 + WINDOW]);
    let middle = frequency_of(&out.left[12_000..12_000 + WINDOW]);
    let tail = dominant_frequency(&out.left[30_000..47_000], 800.0, 960.0, 1.0);
    println!("[sfz-glide/G3] 端到端滑音：头 {head:.1} / 中 {middle:.1} / 尾 {tail:.1} Hz");
    assert!(
        (head - 453.0).abs() < 30.0,
        "滑音头部必须 ≈453 Hz，实读 {head} Hz"
    );
    assert!(
        (middle - 622.0).abs() < 30.0,
        "滑音中途必须 ≈622 Hz（在 440 与 880 之间），实读 {middle} Hz"
    );
    assert!(
        f64::from(head) < f64::from(middle) && f64::from(middle) < tail,
        "三窗必须单调爬升：{head} → {middle} → {tail}"
    );
    assert!(
        (tail - 880.0).abs() < 6.0,
        "SFZ 轨的滑音尾段必须到达目标音高 880 Hz，实读 {tail} Hz"
    );
}

/// G4：端到端 —— 弯音与滑音**相加**（D9）：恒定弯音 +600 ＋ 滑音 +1200
/// ⇒ 尾段 +1800 cents ⇒ 440 × 2^1.5 ≈ 1244.5 Hz。
#[test]
fn end_to_end_bend_and_slide_add_up_on_a_sampler_track() {
    let out = render_end_to_end(|note| {
        note.pitch_bend_curve = vec![(0, 600)];
        note.slide = Some(SlideConfig {
            duration_ticks: 960,
            target_pitch: 72,
            curve: CurveType::Linear,
        });
    });
    let tail = dominant_frequency(&out.left[30_000..47_000], 1_100.0, 1_400.0, 1.0);
    println!("[sfz-glide/G4] 弯音＋滑音尾段主频 {tail:.1} Hz（期望 ≈1244.5）");
    assert!(
        (tail - 1_244.5).abs() < 12.0,
        "弯音与滑音必须相加（+600 + +1200 = +1800 cents ⇒ ≈1244.5 Hz），实读 {tail} Hz"
    );
}

/// G5：端到端 —— 目标音高 ＝ 本音高的滑音（目标音分 0）与无滑音逐位相同。
#[test]
fn end_to_end_a_zero_slide_is_bit_identical_to_no_slide() {
    let plain = render_end_to_end(|_note| {});
    let zero = render_end_to_end(|note| {
        note.slide = Some(SlideConfig {
            duration_ticks: 960,
            target_pitch: 60,
            curve: CurveType::Linear,
        });
    });
    assert!(
        plain.left.iter().any(|s| *s != 0.0),
        "对照臂必须真的出声（否则逐位相同是空转）"
    );
    assert_bit_identical("G5 左声道", &zero.left, &plain.left);
    assert_bit_identical("G5 右声道", &zero.right, &plain.right);
}

/// G6：端到端 —— ease 曲线被如实消费：`Exponential` 在中点只走 1/4
/// （+300 cents ⇒ ≈523.25 Hz），端点仍精确到达目标。
#[test]
fn end_to_end_an_exponential_slide_eases_and_still_lands_on_target() {
    let out = render_end_to_end(|note| {
        note.slide = Some(SlideConfig {
            duration_ticks: 960,
            target_pitch: 72,
            curve: CurveType::Exponential,
        });
    });
    let middle = frequency_of(&out.left[12_000..12_000 + WINDOW]);
    let tail = dominant_frequency(&out.left[30_000..47_000], 800.0, 960.0, 1.0);
    println!("[sfz-glide/G6] 指数滑音：中 {middle:.1} Hz（期望 ≈523.25）/ 尾 {tail:.1} Hz");
    assert!(
        (middle - 523.25).abs() < 30.0,
        "Exponential 中点必须 ≈523.25 Hz（ease(0.5)=0.25 ⇒ +300 cents），实读 {middle} Hz"
    );
    assert!(
        (tail - 880.0).abs() < 6.0,
        "滑音端点必须精确到达 880 Hz（与 ease 形状无关），实读 {tail} Hz"
    );
}
