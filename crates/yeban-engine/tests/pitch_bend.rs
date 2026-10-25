//! ⭐ 任务单 `#311b` 的判据：**音高随时间变化**在复音合成器里**真的发声**
//! （弯音曲线 ＋ 滑音，`ADR-0012` D4/D5/D9/D10）。
//!
//! # 这一票之前的事实
//!
//! `MidiNote::pitch_bend_curve` 与 `MidiNote::slide` 只存在于持久化 AST 里：
//! `project_schedules` 只投影 `pitch` / `velocity` / 起止样本，`PolySynth` 的
//! `inc` 在触发时算一次、之后只由换采样率重算 ⇒ **常量增量、没有音高输入口**。
//!
//! # 这一票之后的数据流（逐段可核）
//!
//! ```text
//! MidiNote::{pitch_bend_curve, slide}
//!   └─(构造期, snapshot::project_pitch_shape)→ PitchShape { bend: [样本偏移, cents], slide }
//!        └─ NoteSchedule::pitch_shapes（每轨旁表）＋ ScheduledNote::pitch_shape: u16
//!             └─(实时侧, render_track) 探针 Option<&dyn Fn(u16, u64) -> f32>
//!                  └─ PolySynth::render_with_pitch → 音符局部 64 样本网格重算 inc
//! ```
//!
//! # 判据表（"怎么变红" ＝ 注入）
//!
//! | # | 判据 | 怎么变红 |
//! | :--- | :--- | :--- |
//! | B1 | A4 ＋ `[(0,0),(960,1200)]`：**首段主频 440 Hz、末段 880 Hz**（Goertzel 扫描给数） | 探针不生效 / 曲线没进旁表 |
//! | B2 | **D4 首点之前取 0**：`[(240,200)]` 的前 6000 样本与**空曲线**逐位相同；之后 493.88 Hz | 外插首值 / 把首点当成"从 0 开始弯" |
//! | B3 | **滑音**：`SlideConfig { target_pitch: 81 }` ⇒ 末段 880 Hz（末频 ＝ 目标） | 滑音没投影 / `ease` 用错 |
//! | B4 | **D9 相加**：弯音 1200 ＋ 滑音 1200 ⇒ 末段 **1760 Hz** | 两条通路取 `max`/覆盖而不是相加 |
//! | B5 | **正对照**：没有曲线的工程 ⇒ `pitch_shapes` 为空、每个音符 `NO_PITCH_SHAPE` | 给空曲线也建表项（那样默认路径就不再"一次浮点都不加"） |
//!
//! ⚠ **不证**（如实登记）：人耳听感；64 样本网格（3.2 cents/步）在极快弯音下的
//! 拉链噪声；`ratchet`/循环副本"各自从 0 重放"的**听觉**差异（`D10` 只钉了投影的
//! 局部坐标，B1–B4 用的是单音符单副本）。

mod support;

use support::{NoteSpec, note_project, render};
use yeban_engine::synth::NO_PITCH_SHAPE;
use yeban_model::{ClipContent, CurveType, MidiNote, SlideConfig, YebanProjectV1};

/// 夹具采样率（`support` 的固定栅格：120 BPM / 960 PPQ ⇒ **1 tick = 25 样本**）。
const SAMPLE_RATE: f64 = 48_000.0;

/// 改夹具里**每一个** MIDI 音符（本文件的判据都只有一个音符）。
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

/// 单 bin 的加窗（Hann）DFT 幅度。窗口抑制泄漏，扫描时才敢用 1 Hz 步长。
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

/// 在 `[lo, hi]` 里按 `step` 扫出**幅度最大**的频率（单位 Hz）。
fn dominant_frequency(samples: &[f32], lo: f64, hi: f64, step: f64) -> f64 {
    let mut best = lo;
    let mut best_magnitude = f64::NEG_INFINITY;
    let mut freq = lo;
    while freq <= hi {
        let magnitude = magnitude_at(samples, freq, SAMPLE_RATE);
        if magnitude > best_magnitude {
            best_magnitude = magnitude;
            best = freq;
        }
        freq += step;
    }
    best
}

/// 一个 1 秒（1920 tick = 48000 样本）的 A4 音符夹具 + 375 个量子（1 s）的渲染。
fn render_a4<F>(mutate: F) -> Vec<f32>
where
    F: Fn(&mut MidiNote),
{
    let fixture = note_project(&[NoteSpec::at(0, 1_920, 69, 100)]);
    let project = with_note(fixture.project, mutate);
    // 375 个量子 × 128 帧 = 48000 帧 = 1 s。
    render(&project, 375).left
}

/// 判据 B1：`[(0,0),(960,1200)]` ⇒ 首段 440 Hz、末段 880 Hz。
#[test]
fn a_bent_note_reaches_880_hz_in_its_last_segment() {
    let samples = render_a4(|note| {
        note.pitch_bend_curve = vec![(0, 0), (960, 1_200)];
    });
    assert_eq!(samples.len(), 48_000, "1 s @48 kHz");
    // 首段：跳过 5 ms attack（256 帧）之后到弯音中点之前。
    // 末段：弯音在 tick 960 = 样本 24000 处结束 ⇒ 30000..47000 是**保持 880**。
    let first = dominant_frequency(&samples[1_000..11_000], 380.0, 520.0, 1.0);
    let last = dominant_frequency(&samples[30_000..47_000], 800.0, 960.0, 1.0);
    let peak_440 = magnitude_at(&samples[30_000..47_000], 440.0, SAMPLE_RATE);
    let peak_880 = magnitude_at(&samples[30_000..47_000], 880.0, SAMPLE_RATE);
    println!(
        "### [B1] 首段主频 = {first:.3} Hz（期望 ≈440）；末段主频 = {last:.3} Hz（期望 ≈880）；\
         末段 440 幅度 = {peak_440:.9}、880 幅度 = {peak_880:.9}"
    );
    // ⚠ 首段是**弯音的斜坡**（曲线从 tick 0 就开始爬），因此这里只断言它
    // **还没到 880**（一个 1 Hz 扫描在斜坡上给出的是窗口内的"平均峰"）。
    assert!(
        first < 700.0,
        "首段必须还在 440 附近爬升（⛔ 不是已经 880），实测 {first}"
    );
    assert!(
        (last - 880.0).abs() <= 2.0,
        "末段主频必须 ≈880 Hz，实测 {last}"
    );
    assert!(
        peak_880 > 10.0 * peak_440,
        "末段 880 的幅度必须远大于 440（880={peak_880}, 440={peak_440}）"
    );
}

/// 判据 B2（`ADR-0012` D4 的 2026-10-25 裁决）：首点之前**取 0**，且与空曲线
/// **逐位相同**；首点之后 200 cents ⇒ 493.88 Hz。
#[test]
fn a_late_first_point_is_zero_before_it_and_two_hundred_cents_after() {
    let with_curve = render_a4(|note| {
        // ⭐ 刻意**不写**首点 `(0, 0)`：这正是 D4 裁决要支持的形状。
        note.pitch_bend_curve = vec![(240, 200)];
    });
    let without_curve = render_a4(|_| {});
    // tick 240 = 240 × 25 = 6000 样本。
    assert_eq!(
        with_curve[..6_000]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        without_curve[..6_000]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        "首点之前必须**逐位**等于空曲线（cents = 0）"
    );
    let before = dominant_frequency(&with_curve[1_000..5_500], 380.0, 520.0, 1.0);
    // 后面一段：tick 960 之后曲线保持 200 cents ⇒ 440 × 2^(200/1200) = 493.883 Hz。
    let after = dominant_frequency(&with_curve[30_000..47_000], 450.0, 540.0, 1.0);
    println!(
        "### [B2] 首点之前主频 = {before:.3} Hz（期望 ≈440，且前 6000 样本逐位等于空曲线）；首点之后 = {after:.3} Hz（期望 ≈493.88）"
    );
    assert!(
        (before - 440.0).abs() <= 2.0,
        "首点之前必须仍是 440 Hz，实测 {before}"
    );
    assert!(
        (after - 493.883).abs() <= 2.0,
        "首点之后必须 ≈493.88 Hz（200 cents），实测 {after}"
    );
}

/// 判据 B3：滑音到 `target_pitch = 81`（A5）⇒ **末频 ＝ 目标**（880 Hz）。
#[test]
fn a_slide_reaches_the_target_pitch() {
    let samples = render_a4(|note| {
        note.slide = Some(SlideConfig {
            duration_ticks: 960,
            target_pitch: 81,
            curve: CurveType::Linear,
        });
    });
    let first = dominant_frequency(&samples[1_000..11_000], 380.0, 520.0, 1.0);
    let last = dominant_frequency(&samples[30_000..47_000], 800.0, 960.0, 1.0);
    println!(
        "### [B3] 滑音首段 = {first:.3} Hz（期望 ≈440）；末段 = {last:.3} Hz（目标 81 ⇒ 880）"
    );
    assert!(
        first < 700.0,
        "滑音起点段必须还在 440 附近（⛔ 不是已经 880），实测 {first}"
    );
    assert!(
        (last - 880.0).abs() <= 2.0,
        "滑音末频必须精确落在目标音高（880 Hz），实测 {last}"
    );
}

/// 判据 B4（`D9`）：`total = bend + slide` —— 两条都 +1200 ⇒ 末段 **1760 Hz**。
#[test]
fn slide_and_bend_add_up() {
    let samples = render_a4(|note| {
        // 弯音在 tick 480（= 样本 12000）就到 +1200 并保持。
        note.pitch_bend_curve = vec![(0, 0), (480, 1_200)];
        // 滑音在 tick 960（= 样本 24000）到 +1200（A5）并保持。
        note.slide = Some(SlideConfig {
            duration_ticks: 960,
            target_pitch: 81,
            curve: CurveType::Linear,
        });
    });
    let last = dominant_frequency(&samples[30_000..47_000], 1_600.0, 1_900.0, 1.0);
    let bend_only = magnitude_at(&samples[30_000..47_000], 880.0, SAMPLE_RATE);
    let sum = magnitude_at(&samples[30_000..47_000], 1_760.0, SAMPLE_RATE);
    println!(
        "### [B4] 相加末段主频 = {last:.3} Hz（期望 440×2^(2400/1200) = 1760）；880 幅度 = {bend_only:.9}、1760 幅度 = {sum:.9}"
    );
    assert!(
        (last - 1_760.0).abs() <= 3.0,
        "相加之后必须 ≈1760 Hz（+2400 cents），实测 {last}"
    );
    assert!(
        sum > 10.0 * bend_only,
        "1760 的幅度必须远大于 880（sum={sum}, 880={bend_only}）"
    );
}

/// 判据 B5（正对照）：没有曲线的工程 ⇒ 旁表为空、每个音符都是 `NO_PITCH_SHAPE`。
///
/// 这条是"空曲线 ⇒ **一次浮点都不加**"在投影层的机械读数：旁表为空 ⇒ 引擎传
/// `None` 探针 ⇒ 器件走今天这条路径 ⇒ 既有音频摘要逐位不动。
#[test]
fn an_empty_curve_projects_to_no_pitch_shape_at_all() {
    use yeban_engine::snapshot::EngineSnapshot;

    let fixture = note_project(&[NoteSpec::at(0, 1_920, 69, 100)]);
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1).expect("快照必须能建");
    let schedule = snapshot.schedule(&fixture.track).expect("本轨必须有调度表");
    assert!(
        !schedule.has_pitch_shapes(),
        "空曲线 ⇒ 旁表必须为空（⛔ 不许建一个恒 0 的表项）"
    );
    assert!(schedule.pitch_shapes().is_empty());
    assert!(
        schedule
            .notes()
            .iter()
            .all(|note| note.pitch_shape() == NO_PITCH_SHAPE),
        "空曲线 ⇒ 每个音符都必须是 NO_PITCH_SHAPE"
    );
    // 反向对照：真的写一条曲线 ⇒ 旁表里有且只有一项，音符下标指向它。
    let fixture = note_project(&[NoteSpec::at(0, 1_920, 69, 100)]);
    let project = with_note(fixture.project, |note| {
        note.pitch_bend_curve = vec![(240, 200)];
    });
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照必须能建");
    let schedule = snapshot.schedule(&fixture.track).expect("本轨必须有调度表");
    assert_eq!(schedule.pitch_shapes().len(), 1);
    assert_eq!(schedule.notes()[0].pitch_shape(), 0);
    // D4 的读数：tick 240 = 6000 样本之前 0，之后 200。
    assert_eq!(schedule.cents_at(0, 5_999), 0.0);
    assert_eq!(schedule.cents_at(0, 6_000), 200.0);
    assert_eq!(schedule.cents_at(0, 100_000), 200.0, "末点之后保持末值");
    assert_eq!(schedule.cents_at(NO_PITCH_SHAPE, 6_000), 0.0, "哨兵 ⇒ 0");
    assert_eq!(
        schedule.cents_at(9_999, 6_000),
        0.0,
        "越界下标 ⇒ 0（⛔ 不 panic）"
    );
}
