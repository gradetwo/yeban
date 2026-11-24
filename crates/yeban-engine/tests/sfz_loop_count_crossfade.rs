//! ⭐ **SFZ `loop_count` 与 `loop_crossfade`**（T-FIX-282）。
//!
//! # 这条判据在修什么
//!
//! SFZ v2 的 `loop_count` 与 `loop_crossfade` 没有实现：`loop_count=3` 仍按
//! `loop_continuous` 无限回绕、直到 note-off；`loop_crossfade=0.01` 不在接缝做
//! 交叉淡化。规范逐字（<https://sfzformat.com/opcodes/loop_count/>）：
//!
//! > "The number of times a loop will repeat. … loop_count=3 will result in the
//! > looped part of the sample being played **four times**, not three times.
//! > If the sample contains data after the end of the loop, this portion of the
//! > sample will be played after all repetitions of the loop have been completed
//! > (assuming no note-off message had been received by then)."
//!
//! # 夹具与算数
//!
//! 斜坡样本 `pcm[i] = i / 1024`（`rate = 1` ⇒ 输出值可**反解**成源下标），
//! 样本长 9600 帧，窗口 `[1000, 5000)`（长 4000）、头 1000 帧、尾巴
//! `5000..9598`（4599 帧 —— 与 L3 同一条"最后一帧差一格"的既有语义）。
//! 无 note-off（`held_frames = None`）。
//!
//! | 判据 | 场景 | 期望写入帧数 |
//! | :--- | :--- | :--- |
//! | C1 | `loop_count=3` | `1000 + 4×4000 + 4599 = 21599`，且各边界反解的源下标逐点对得上 |
//! | C2 | 不写 `loop_count`（无限） | 写满输出（30000）—— C1 的"停"是次数给的，⛔ 不是别的东西 |
//! | C3 | `loop_crossfade=0` | 与不写它**逐位相同**（硬切回归守卫） |
//! | C4 | `loop_crossfade=0.01`（480 帧） | 接缝处最大帧间跳变 < 0.5（无淡化是 ≈3.91）；淡化区之前与尾巴**逐位不变**（⛔ 不整体平移） |
//! | C5 | `one_shot` ＋两者 | 与不写两者**逐位相同**、写 `9599` 帧（`one_shot` 没有循环窗口） |
//! | C6 | `loop_sustain` ＋ `loop_count=1` | `1000 + 2×4000 + 4599 = 13599`（sustain 也受次数管） |
//! | C7 | `direction=reverse` ＋ `loop_crossfade` | 与不写淡化**逐位相同**（反向不做淡化，登记的局限） |
/// | C8 | 实时池（单声道）＋ `loop_count=3` | 最后一帧非零下标恰为 21598，边界反解与 C1 相同 |
/// | C9 | 实时池（立体声）＋ `loop_count=3` | 左右声道**逐位相同**（双跑快照带回绕计数）＋ 最后一帧非零下标 21598 |
use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 引擎采样率。
const HZ: f32 = 48_000.0;
/// 样本长度（帧）：0.2 s。
const PCM: usize = 9_600;
/// 输出长度（帧）：0.625 s（> C1 的 21599，< 无限循环的写满量）。
const OUT: usize = 30_000;
/// 循环窗口。
const LOOP_START: usize = 1_000;
const LOOP_END: usize = 5_000;
const LOOP_LEN: usize = LOOP_END - LOOP_START;
/// 头（`0..LOOP_START`）与尾巴（`LOOP_END..PCM-1`）的帧数。
const HEAD: usize = LOOP_START;
const TAIL: usize = PCM - 1 - LOOP_END;
/// 块长（池侧判据用）。
const BLOCK: usize = 512;

/// 斜坡样本：`pcm[i] = i / 1024` ⇒ 输出值可反解下标。
fn ramp() -> InMemorySample {
    let pcm: Vec<f32> = (0..PCM).map(|i| i as f32 / 1024.0).collect();
    InMemorySample {
        channels: 1,
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 立体声斜坡样本：左右声道同一份斜坡（交错）。
fn stereo_ramp() -> InMemorySample {
    let mut pcm = Vec::with_capacity(PCM * 2);
    for i in 0..PCM {
        let v = i as f32 / 1024.0;
        pcm.push(v);
        pcm.push(v);
    }
    InMemorySample {
        channels: 2,
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 离线渲染一份 SFZ（单区域，无 note-off），返回（写入帧数, 输出缓冲）。
///
/// ⚠ 力度用 127：`velocity_gain = (vel/127)²`（`amp_veltrack` 缺省 100）⇒
/// 只有 127 的增益恰为 `1.0`，输出值才能**绝对反解**成源下标（L1～L4 只做
/// 相对比较所以用了 100；本文件要逐点反解，必须用 127）。
fn render(ops: &str) -> (usize, Vec<f32>) {
    let text = format!("<region> sample=a.wav key=60 pitch_keycenter=60 {ops}");
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), ramp());
    let mut out = vec![0.0_f32; OUT];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 127, None, None, &mut out, HZ)
        .expect("渲染");
    (written, out)
}

/// 把输出帧 `t` 的值反解成源下标（斜坡夹具：`out[t] == src / 1024`）。
fn src_at(out: &[f32], t: usize) -> i32 {
    (out[t] * 1024.0).round() as i32
}

/// 断言 `t` 处输出的源下标（失败信息里同时报输出值）。
fn assert_src(out: &[f32], t: usize, expected: usize, what: &str) {
    assert_eq!(
        src_at(out, t),
        expected as i32,
        "⭐ {what}：输出帧 {t} 必须是源 {expected}，实得源 {}（值 {}）",
        src_at(out, t),
        out[t],
    );
}

/// C1 的全部边界反解（窗口 `[1000, 5000)`、次数 3 ⇒ 播 4 次 ＋ 尾巴）。
fn assert_c1_boundaries(out: &[f32]) {
    assert_src(out, 0, 0, "头第一帧");
    assert_src(out, HEAD - 1, LOOP_START - 1, "头最后一帧");
    assert_src(out, HEAD, LOOP_START, "窗口第一播起点");
    assert_src(out, HEAD + LOOP_LEN - 1, LOOP_END - 1, "第一播末帧");
    assert_src(out, HEAD + LOOP_LEN, LOOP_START, "第二播起点（第一次回绕）");
    assert_src(out, HEAD + 2 * LOOP_LEN, LOOP_START, "第三播起点");
    assert_src(out, HEAD + 3 * LOOP_LEN, LOOP_START, "第四播起点");
    assert_src(out, HEAD + 4 * LOOP_LEN - 1, LOOP_END - 1, "第四播末帧");
    assert_src(
        out,
        HEAD + 4 * LOOP_LEN,
        LOOP_END,
        "尾巴起点（次数耗尽、不再回绕）",
    );
    assert_src(out, HEAD + 4 * LOOP_LEN + TAIL - 1, PCM - 2, "最后一帧");
}

/// ⭐ C1：`loop_count=3` ⇒ 循环部分播 4 次，然后播尾巴、无 note-off 自然结束。
#[test]
fn c1_loop_count_3_plays_the_window_four_times_then_the_tail() {
    let (written, out) =
        render("loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3");
    let expected = HEAD + 4 * LOOP_LEN + TAIL;
    assert_eq!(
        written, expected,
        "⭐ `loop_count=3` ⇒ 头 {HEAD} ＋ 4×{LOOP_LEN} ＋ 尾巴 {TAIL} ＝ {expected} 帧；实得 {written} 帧"
    );
    assert_c1_boundaries(&out);
}

/// ⭐ C2：反向腿 —— 不写 `loop_count` ⇒ 无限循环、写满输出。
#[test]
fn c2_without_loop_count_the_loop_never_ends() {
    let (written, _) = render("loop_mode=loop_continuous loop_start=1000 loop_end=5000");
    assert_eq!(
        written, OUT,
        "⭐ 不写 `loop_count`（缺省 0 ＝ 无限）⇒ 必须写满输出；实得 {written} 帧（＝ C1 的停是次数给的）"
    );
}

/// ⭐ C3：`loop_crossfade=0` ⇒ 与不写它逐位相同。
#[test]
fn c3_zero_crossfade_is_bit_identical() {
    let (written, out) = render(
        "loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3 loop_crossfade=0",
    );
    let (reference_written, reference) =
        render("loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3");
    assert_eq!(written, reference_written, "写入帧数必须相同");
    assert_eq!(
        out, reference,
        "⭐ `loop_crossfade=0` ⇒ 与不写它**逐位相同**（硬切回归守卫）"
    );
}

/// ⭐ C4：`loop_crossfade=0.01` ⇒ 接缝跳变显著低于无淡化基线，且不整体平移。
#[test]
fn c4_crossfade_softens_the_seam_without_shifting_the_loop() {
    let xf_frames = 480usize; // 0.01 s @ 48 kHz。
    let zone_start = HEAD + LOOP_LEN - xf_frames; // 第一个淡化区段的输出帧。
    let (written, out) = render(
        "loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3 loop_crossfade=0.01",
    );
    let (reference_written, reference) =
        render("loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3");
    assert_eq!(
        written, reference_written,
        "淡化不改变总帧数（次数与尾巴都不动）；实得 {written} vs {reference_written}"
    );
    // ① 淡化区段之前逐位不变（⛔ 不整体平移）；区段首帧 `t == 0` 时淡化恰为恒等。
    assert_eq!(
        out[..=zone_start],
        reference[..=zone_start],
        "⭐ 淡化区段之前（含首帧）必须**逐位不变**"
    );
    // ② 次数耗尽后的尾巴逐位不变（最后一播结束时不再回绕 ⇒ 也不再淡化）。
    let tail_start = HEAD + 4 * LOOP_LEN;
    assert_eq!(
        out[tail_start..written],
        reference[tail_start..written],
        "⭐ 尾巴必须**逐位不变**"
    );
    // ③ 接缝处最大帧间跳变：无淡化 ≈ 3.91（`pcm[4999] − pcm[1000]`），
    // 有淡化必须显著低于它（实测约 0.22；判据取 0.5 留足余量）。
    let seam = |buf: &[f32]| {
        (zone_start..HEAD + 2 * LOOP_LEN)
            .map(|t| (buf[t + 1] - buf[t]).abs())
            .fold(0.0f32, f32::max)
    };
    let with_xf = seam(&out);
    let without_xf = seam(&reference);
    println!(
        "[T-FIX-282/C4] seam max |Δ|: without crossfade = {without_xf:.4}, with 0.01 s crossfade = {with_xf:.4}"
    );
    assert!(
        without_xf > 3.0,
        "⭐ 无淡化基线必须真的有硬切（接缝跳变 {without_xf}，预期 > 3.0）"
    );
    assert!(
        with_xf < 0.5,
        "⭐ 有淡化时接缝跳变必须显著低于基线：实得 {with_xf}（基线 {without_xf}）"
    );
}

/// ⭐ C5：`one_shot` ＋两者 ⇒ 与不写两者逐位相同（`one_shot` 没有循环窗口）。
#[test]
fn c5_one_shot_ignores_loop_count_and_crossfade() {
    let (written, out) =
        render("loop_mode=one_shot loop_start=1000 loop_end=5000 loop_count=3 loop_crossfade=0.01");
    let (reference_written, reference) = render("loop_mode=one_shot loop_start=1000 loop_end=5000");
    assert_eq!(
        written,
        PCM - 1,
        "⭐ `one_shot` 播到样本尾就停；实得 {written} 帧"
    );
    assert_eq!(written, reference_written, "写入帧数必须相同");
    assert_eq!(
        out, reference,
        "⭐ `one_shot` 上两个 opcode 必须**毫无影响**"
    );
}

/// ⭐ C6：`loop_sustain` 也受次数管（`loop_count=1` ⇒ 播 2 次 ＋ 尾巴）。
#[test]
fn c6_loop_sustain_honours_the_count() {
    let (written, out) =
        render("loop_mode=loop_sustain loop_start=1000 loop_end=5000 loop_count=1");
    let expected = HEAD + 2 * LOOP_LEN + TAIL;
    assert_eq!(
        written, expected,
        "⭐ `loop_sustain` ＋ `loop_count=1` ⇒ 头 {HEAD} ＋ 2×{LOOP_LEN} ＋ 尾巴 {TAIL} ＝ {expected} 帧；实得 {written} 帧"
    );
    assert_src(&out, HEAD + LOOP_LEN, LOOP_START, "第二播起点");
    assert_src(&out, HEAD + 2 * LOOP_LEN, LOOP_END, "尾巴起点");
}

/// ⭐ C7：反向 ＋ 淡化 ⇒ 与不写淡化逐位相同（反向不做交叉淡化，登记的局限）。
#[test]
fn c7_reverse_ignores_crossfade_but_honours_the_count() {
    let (written, out) = render(
        "direction=reverse loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3 loop_crossfade=0.01",
    );
    let (reference_written, reference) = render(
        "direction=reverse loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3",
    );
    assert_eq!(written, reference_written, "写入帧数必须相同");
    assert_eq!(
        out, reference,
        "⭐ 反向写了 `loop_crossfade` 也按硬切渲染（登记的局限）"
    );
    // 次数在反向同样生效：有限次数 ⇒ 写入帧数有限（⛔ 不是写满输出）。
    assert!(
        written < OUT,
        "⭐ 反向 `loop_count=3` 也必须停绕、自然结束；实得 {written} 帧"
    );
}

/// 摊平一条 SFZ 文本（控制侧；允许分配）。
fn prepare(text: &str, sample: &InMemorySample) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare(&sfz, sample, HZ).expect("摊平必须成功")
}

/// 池侧渲染：按 `BLOCK` 逐块写满 `OUT` 帧，返回整段输出。
fn render_pool_mono(plan: &PreparedSfz) -> Vec<f32> {
    let mut pool = SfzVoicePool::empty();
    // ⚠ 力度 127（增益恰 1.0，见 `render` 的注释）。
    let created = pool.trigger_note(plan, 60, 127, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "必须建起一个声部");
    let mut out = vec![0.0f32; OUT];
    let mut position = 0u64;
    for chunk in out.chunks_mut(BLOCK) {
        chunk.fill(0.0);
        pool.render_into(plan, position, chunk);
        position += BLOCK as u64;
    }
    assert_eq!(
        pool.active_count(),
        0,
        "⭐ 播完尾巴后声部必须退休（次数耗尽 ＋ 样本播完）"
    );
    out
}

/// ⭐ C8：实时池（单声道）与离线路同一份次数语义。
#[test]
fn c8_realtime_pool_honours_loop_count() {
    let plan = prepare(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3",
        &ramp(),
    );
    let out = render_pool_mono(&plan);
    let last_nonzero = out.iter().rposition(|v| *v != 0.0).expect("必须出声");
    assert_eq!(
        last_nonzero,
        HEAD + 4 * LOOP_LEN + TAIL - 1,
        "⭐ 最后一帧非零下标必须是尾巴末帧；实得 {last_nonzero}"
    );
    assert_c1_boundaries(&out);
}

/// ⭐ C9：实时池（立体声双跑）左右声道逐位相同，且次数语义与 C8 一致。
#[test]
fn c9_stereo_pool_left_and_right_agree_on_loop_count() {
    let plan = prepare(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous loop_start=1000 loop_end=5000 loop_count=3",
        &stereo_ramp(),
    );
    let mut pool = SfzVoicePool::empty();
    // ⚠ 力度 127（增益恰 1.0，见 `render` 的注释）。
    let created = pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "必须建起一个声部");
    let mut left = vec![0.0f32; OUT];
    let mut right = vec![0.0f32; OUT];
    let mut position = 0u64;
    for (lchunk, rchunk) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
        pool.render_into_stereo_with_probes(&plan, position, lchunk, rchunk, None, None);
        position += BLOCK as u64;
    }
    // ⭐ 双跑快照必须带回绕计数：左遍推进的次数若漏进右遍的起点，两声道会在
    // 回绕点分家（右声道提前停绕）。`pan=0` ＋ 左右同源 ⇒ 两声道逐位相同。
    assert_eq!(
        left, right,
        "⭐ 立体声双跑的左右声道必须**逐位相同**（快照带回绕计数）"
    );
    let last_nonzero = left.iter().rposition(|v| *v != 0.0).expect("必须出声");
    assert_eq!(
        last_nonzero,
        HEAD + 4 * LOOP_LEN + TAIL - 1,
        "⭐ 最后一帧非零下标必须是尾巴末帧；实得 {last_nonzero}"
    );
    assert_c1_boundaries(&left);
}
