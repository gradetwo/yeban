//! **portamento 的端到端判据**（本单）：`egN_pitch_oncc140` ＋ 块内速率 ramp。
//!
//! 规范事实与生态写法见 `yeban-sfz/src/pitch_eg.rs` 的模块文档。这里只钉**可观察的后果**：
//!
//! * 没有 `eg1_*` ⇒ 第二个音**立刻**按目标音高发声（659 Hz）；
//! * 有 `eg1_*`（`level0=-1`、`level1=0`、`time1=0.3`、`depth=100`）且前一个音是 60
//!   ⇒ 第二个音（67）**从 60 的音高起滑**（440 Hz），0.3 s 后到 659 Hz。
//!
//! 测量方法：`InMemorySample::sine(440)` 是纯正弦 ⇒ **过零率 ≈ 2 × 频率** 是可靠读数
//! （480 帧窗口内 4～7 个周期，过零计数与频率成正比）。

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 采样率（Hz）。
const HZ: f32 = 48_000.0;
/// 内存样本长度（帧）：2 秒，足够跑完 0.3 s 的滑音。
const SAMPLE_FRAMES: usize = 96_000;
/// 测频窗口（帧）：10 ms ⇒ 440 Hz 约 4.4 个周期、659 Hz 约 6.6 个周期。
const WINDOW: usize = 480;

/// 不带 portamento 的对照区域。
const NO_EG: &str = "<region> sample=a.wav pitch_keycenter=60\n";

/// 带 portamento 的区域：`level0=-1`（从上一个音起）、`level1=0`（滑到目标音高）、
/// `time1=0.3`（0.3 秒）、`pitch_oncc140=100`（100 cents / 半音）。
const WITH_EG: &str = "<region> sample=a.wav pitch_keycenter=60\n\
                       eg1_level0=-1\n\
                       eg1_level1=0\n\
                       eg1_time1=0.3\n\
                       eg1_pitch_oncc140=100\n";

/// 摊平一支 sfz（48 kHz、440 Hz 正弦样本）。
fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功")
}

/// 过零率推出来的频率（Hz）—— 纯正弦下用**过零时刻的线性插值**求平均周期。
///
/// ⚠ 直接"数过零个数 / 窗口时长"的量化台阶是 `1 / (2 × 10 ms) = 50 Hz`
/// （实测把 659 Hz 读成 700），不足以分辨 440 / 659。这里改数**过零时刻**：
/// 首末两个过零点之间跨了 `n − 1` 个周期 ⇒ `f = (n − 1) × 采样率 / 跨度`，
/// 精度由插值给定（本夹具实测 < 1 Hz）。
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

/// 触发 60 → 丢掉它的尾巴 → 触发 67 → 返回**第二音前 `WINDOW` 帧**与**0.3 s 之后**的读数。
fn slide_readings(prepared: &PreparedSfz) -> (f32, f32) {
    let mut pool = SfzVoicePool::new(HZ);
    let mut sink = vec![0.0f32; 4096];
    // 1) 触发 60（首个音，delta = 0 ⇒ 不滑），渲染 0.2 s 让它进入稳态。
    pool.trigger_note(prepared, 60, 127, u64::MAX, 1.0, None);
    for block in 0..3 {
        pool.render_into_with_cc(prepared, (block * sink.len()) as u64, &mut sink, None);
    }
    // 2) 触发 67（delta = +7 半音）⇒ 有 eg 时从 440 Hz 起滑。
    pool.trigger_note(prepared, 67, 127, u64::MAX, 1.0, None);
    let mut head = vec![0.0f32; WINDOW];
    pool.render_into_with_cc(prepared, 0, &mut head, None);
    // 3) 再走 0.4 s（> `time1 = 0.3`）⇒ 应已到目标音高。
    let mut rest = vec![0.0f32; 19_200];
    pool.render_into_with_cc(prepared, 0, &mut rest, None);
    let mut tail = vec![0.0f32; WINDOW];
    pool.render_into_with_cc(prepared, 0, &mut tail, None);
    (frequency_of(&head), frequency_of(&tail))
}

/// 判据 A：**没有 `eg1_*` ⇒ 第二个音立刻按 67 的音高发声**（对照臂，⛔ 不许"也滑了"）。
///
/// 怎么变红：把 `PitchTrack` 无条件构造（不判 `pitch_eg`）⇒ 第二音起始读数掉到 440。
#[test]
fn without_a_pitch_envelope_the_second_note_starts_at_its_own_pitch() {
    let (head, tail) = slide_readings(&prepare(NO_EG));
    println!("[portamento/对照] 第二音起始 {head:.1} Hz；0.3 s 后 {tail:.1} Hz");
    // 67 半音在 `pitch_keycenter=60` 上是 2^(7/12) = 1.4983 ⇒ 659.3 Hz。
    assert!(
        (head - 659.3).abs() < 15.0,
        "对照臂必须立刻是目标音高（659 Hz），实读 {head} Hz"
    );
    assert!(
        (tail - 659.3).abs() < 15.0,
        "对照臂尾部仍是 659 Hz，实读 {tail} Hz"
    );
}

/// 判据 B：**有 `eg1_*` ⇒ 第二个音从上一个音的音高起滑，0.3 s 后到位**。
///
/// 这一条同时钉住了三件事：
/// 1. `CC140` 的 key delta 真的由**上一个触发音**算出（不是 0 —— 否则起始就是 659）；
/// 2. `eg1_time1` 真的是**秒**（0.4 s 后必须到位，而不是一直在滑）；
/// 3. 块内速率真的在**连续**变化（`mix_span_into` 的 ramp），而不是每块跳一档
///    （跳档会在 480 帧窗口里给出**混合**频率，读数落在 440 与 659 之间且方差大）。
///
/// 怎么变红：把 `eg1_time1=0.3` 改成 `0` ⇒ 起始读数变成 659（零长段）。
#[test]
fn with_a_pitch_envelope_the_second_note_glides_from_the_previous_pitch() {
    let (head, tail) = slide_readings(&prepare(WITH_EG));
    println!("[portamento/滑音] 第二音起始 {head:.1} Hz；0.3 s 后 {tail:.1} Hz");
    assert!(
        (head - 440.0).abs() < 15.0,
        "滑音起点必须是上一个音的音高（440 Hz），实读 {head} Hz"
    );
    assert!(
        (tail - 659.3).abs() < 15.0,
        "0.3 s 后必须到目标音高（659 Hz），实读 {tail} Hz"
    );
    // 反向腿：起点与终点必须**真的分开**（⛔ 允许"两端都读成中间值"的假绿）。
    assert!(
        tail - head > 150.0,
        "滑音跨度必须 > 150 Hz（440 → 659），实读 {head} → {tail} Hz"
    );
}

/// 判据 C：**下行也滑**（`CC140` 是有符号的；`CC141` 才是绝对值）。
///
/// 做法：从 67 滑到 60。若实现把 delta 取了绝对值，这一臂会**往上滑**（读数从 659 起）。
#[test]
fn the_key_delta_is_signed_so_descending_glides_work() {
    let prepared = prepare(WITH_EG);
    let mut pool = SfzVoicePool::new(HZ);
    let mut sink = vec![0.0f32; 4096];
    pool.trigger_note(&prepared, 67, 127, u64::MAX, 1.0, None);
    for block in 0..3 {
        pool.render_into_with_cc(&prepared, (block * sink.len()) as u64, &mut sink, None);
    }
    pool.trigger_note(&prepared, 60, 127, u64::MAX, 1.0, None);
    let mut head = vec![0.0f32; WINDOW];
    pool.render_into_with_cc(&prepared, 0, &mut head, None);
    let start = frequency_of(&head);
    println!("[portamento/下行] 60 的起始 {start:.1} Hz");
    assert!(
        (start - 659.3).abs() < 15.0,
        "下行滑音必须从 67 的音高（659 Hz）起，实读 {start} Hz"
    );
}
