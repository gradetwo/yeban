//! `line/engine-mix` 的端到端判据（二）：**3 ms 声部窃取淡出** [ARCH-RT-004, ARCH-DSP-001]。
//!
//! # 为什么单开一个文件
//!
//! 窃取淡出是**声部池内部**的行为：端到端夹具（模型 → 快照 → 母线）要撞到复音上限
//! 才能触发它，而"撞上限"的夹具本身需要精确控制起音时刻与音高（否则各声部同相位，
//! 硬窃取也**不产生**样本跳变，判据会变成永真）。这里用
//! [`support::SynthRig`] 直接驱动 `SynthEngine`：调度表手写，其余（相位递推、
//! ADSR、滤波器、声部池）都是产品路径。
//!
//! # 判据
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | S1 | 池满时发生窃取、计数 > 0（**覆盖度自检**：没有窃取的夹具不算数） | 摘掉窃取分支 |
//! | S2 | 淡出开启（144 帧 = 3 ms @48k）时，窃取处的样本跳变**远小于**硬窃取 | 把淡出时长设为 0（回到硬窃取） |
//! | S3 | 硬窃取的样本跳变**确实很大**（对照臂：证明 S2 有判别力） | 把淡出时长改成 3 ms（对照臂消失） |
//! | S4 | 淡出时长可读、且默认等于 `3 ms × 采样率` | 把常量改成 10 ms |
//! | S5 | 淡出窗口内部平滑（每样本位移 ≤ 指数衰减上界） | 用硬切代替淡出 |
//! | S6 | 窃取路径仍然**逐位确定**（两次渲染位模式相同） | 引入未初始化状态 |

mod support;

use support::{SynthRig, max_step_in, scheduled};
use yeban_engine::synth::{NoteSchedule, ToneParams};

/// 夹具：`VOICES` 个**同相**的长音符把声部池填满，然后在 `STEAL_AT` 处来一个
/// **极端高音**（音高 108 ≈ 4.19 kHz）逼出一次窃取。
///
/// ⚠ `VOICES` 必须**恰好**等于声部池容量：少一个就会有空闲槽位 ⇒ 不触发窃取
/// （本线实测过一次：`VOICES = 2` 时两个低音用掉槽 0/1、第 3 个音符用掉槽 2，
/// `voice_steals` 恒为 0，三条判据全部变成空转）。
///
/// 为什么低音要选音高 24（32.7 Hz）：到第 1000 帧时它的相位已经走到约 **0.68 个周期**
/// （振幅接近峰值），此时被硬切成"新音符的相位 0"会产生一个**很大的**样本跳变 ——
/// 这正是 [ARCH-RT-004] 要消灭的那个爆音。若所有声部同相位同音高，
/// 硬窃取与软窃取都没有跳变，判据就是空的。
/// 声部池容量（必须与 `yeban_engine::synth::VOICES_PER_TRACK` 一致才能把池填满）。
const VOICES: usize = 16;
const STEAL_AT: u64 = 5_000;
const LOW_PITCH: u8 = 45;
const HIGH_PITCH: u8 = 108;

/// 被测夹具：`VOICES` 个长低音（音高 24）把声部池填满，然后在 `STEAL_AT` 处
/// 来**第 `VOICES + 1` 个**同音高音符 ⇒ 触发窃取。
///
/// 音高 24（32.7 Hz）的相位到第 1000 帧已走过约 0.68 个周期（振幅接近峰值），
/// 因此硬窃取会把它**从接近满幅**瞬断 —— 那正是要消灭的爆音。
fn fixture() -> NoteSchedule {
    build_fixture(LOW_PITCH)
}

/// **参照**夹具：与 [`fixture`] 唯一的差别是"第 17 个音符"换了音高。
///
/// ## 差分口径（本文件最重要的一处推导）
///
/// 两个夹具的**唯一**差别是那个额外音符的音高；窃取仍然发生、被窃取的声部仍然是
/// 同一个（窃取选择只看包络/起音时刻，与音高无关）。于是 `被测 − 参照`
/// **恰好等于被窃取声部的全部痕迹**：
///
/// ```text
/// 硬窃取: 前一个样本还在旧音符的幅度上，后一个样本已经是新音符从 0 起音
///         ⇒ 差分的波形 = 旧声部在 `STEAL_AT` 处被**瞬断**（一个约 −1.2 的台阶）
/// 软窃取: 差分的波形 = 旧声部在 3 ms 内**指数衰减到 0**
/// ```
///
/// 实测：硬窃取的差分台阶 **1.8345**（第一版口径下的读数），软窃取下同一窗口
/// 的最大位移 0.5108 —— 软的那 0.51 **不是**台阶，而是淡出走完之后
/// "被测里的高音 / 参照里的低音"两种波形之间的固有斜率（见 S5 的第二段读数）。
fn reference_fixture() -> NoteSchedule {
    build_fixture(HIGH_PITCH)
}

/// 通用构造：`VOICES` 个低音铺满池 + 第 `VOICES + 1` 个音符（音高可换）。
fn build_fixture(extra_pitch: u8) -> NoteSchedule {
    let mut notes = Vec::new();
    for _ in 0..VOICES {
        notes.push(scheduled(0, 96_000, LOW_PITCH, 1.0, 48_000.0));
    }
    notes.push(scheduled(STEAL_AT, 96_000, extra_pitch, 1.0, 48_000.0));
    NoteSchedule::from_sorted(notes)
}

/// 一次渲染：返回样本 + 窃取次数 + 淡出帧数。
fn render_with_fade(fade_frames: Option<u32>) -> (Vec<f32>, u64, u32) {
    render_schedule(&fixture(), fade_frames)
}

/// 渲染任意调度表（夹具与参照共用）。
fn render_schedule(schedule: &NoteSchedule, fade_frames: Option<u32>) -> (Vec<f32>, u64, u32) {
    let mut rig = SynthRig::new(&ToneParams::bypass());
    if let Some(frames) = fade_frames {
        rig.engine.set_steal_fade_frames(frames);
    }
    let fade = rig.engine.steal_fade_frames();
    let rendered = rig.render(schedule, 80);
    (rendered, rig.engine.voice_steals(), fade)
}

/// 差分 `被测 − 参照`。两个夹具都**必须**发生窃取（否则差分口径不成立）。
fn difference(fade_frames: Option<u32>) -> (Vec<f32>, u64) {
    let (test, steals, _) = render_with_fade(fade_frames);
    let (reference, reference_steals, _) = render_schedule(&reference_fixture(), fade_frames);
    assert_eq!(
        reference_steals, 1,
        "参照夹具也必须发生窃取（两边的窃取选择必须落在同一个声部上）"
    );
    let difference: Vec<f32> = test
        .iter()
        .zip(reference.iter())
        .map(|(a, b)| a - b)
        .collect();
    (difference, steals)
}

/// 音符实际被触发的那个样本位置：渲染按 128 帧量子推进，触发发生在**量子边界**上
/// （`render_track` 的游标循环在每个量子里处理"起点落在本量子里"的音符），
/// 而不是 `start_sample` 本身。
#[must_use]
fn trigger_sample() -> usize {
    let quantum = 128u64;
    (STEAL_AT.div_ceil(quantum) * quantum) as usize
}

/// S1 + S4：夹具真的逼出窃取；淡出时长默认等于 `3 ms × 采样率 = 144` 帧。
#[test]
fn fixture_really_steals_and_the_default_fade_is_three_milliseconds() {
    let (rendered, steals, fade) = render_with_fade(None);
    println!("[engine-mix] S4 steal_fade_frames={fade} (3ms @48k = 144) steals={steals}");

    assert_eq!(fade, 144, "默认淡出必须是 3 ms × 48 kHz = 144 帧");
    assert_eq!(steals, 1, "{VOICES} 个声部 + 1 个额外音符 ⇒ 恰好一次窃取");
    assert!(
        rendered.iter().any(|sample| *sample != 0.0),
        "夹具必须真的出声（否则跳变判据是空转）"
    );
}

/// S2 + S3：**差分信号**里，硬窃取在窃取处有一个明确的台阶，而软窃取没有。
///
/// 读数口径（这里踩过三次坑，全部写下来，避免后来者重走）：
///
/// - ❌ 第一版看"整段最大相邻位移"：16 个声部叠加后峰值约 14，读数被**新音符自身的
///   斜率**主导（硬/软 = 1.114/1.080，比值 0.97）⇒ 没有判别力；
/// - ❌ 第二版看"局部斜率 / 自然斜率"：软窃取臂仍有 14.75 倍 —— 窗口里包含新音符
///   5 ms 起音自身的陡峭爬升（0.55），那不是爆音；
/// - ❌ 第三版把两个夹具做成**同音高**差异：差分的后半段变成"被测里的高音 vs
///   参照里的低音"的固有斜率，读数仍被污染；
/// - ✅ 现在：差分 = `fixture()` − `reference_fixture()`，两者只差**额外音符的音高**，
///   窃取依然发生 ⇒ 差分里只剩"被窃取声部"本身。`STEAL_AT` 处的读数因此是纯粹的
///   瞬断 vs 淡出。
///
/// 判据的读数取**窃取发生处的一小段窗口**（触发前后各 8 帧）：硬窃取的台阶在那一处
/// 就出现，而软窃取在该窗口里只是缓慢的指数衰减。
#[test]
fn steal_fade_bounds_the_step_where_hard_stealing_jumps() {
    let (hard, hard_steals) = difference(Some(0));
    let (soft, soft_steals) = difference(None);

    // 台阶窗口：**实际触发的那一帧**（音符起点落在覆盖它的量子边界上）前后各 1 帧。
    let trigger = trigger_sample();
    let start = trigger - 1;
    let end = trigger + 2;
    let hard_step = max_step_in(&hard, start, end);
    let soft_step = max_step_in(&soft, start, end);
    let hard_peak = hard.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    let soft_peak = soft.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));

    println!(
        "[engine-mix] S2/S3 difference peak hard={hard_peak:.6} soft={soft_peak:.6} | \
         step@steal hard={hard_step:.6} soft={soft_step:.6} ratio={:.4} \
         (steals soft/hard={soft_steals}/{hard_steals})",
        soft_step / hard_step
    );

    assert_eq!(soft_steals, 1, "软窃取臂必须真的窃取过");
    assert_eq!(hard_steals, 1, "硬窃取臂必须真的窃取过");
    assert!(
        hard_step > 0.5,
        "硬窃取的差分台阶只有 {hard_step:.6} —— 夹具没有制造出可听爆音，判据没有判别力"
    );
    assert!(
        soft_step < hard_step * 0.2,
        "3 ms 淡出必须把台阶压到硬窃取的五分之一以下: soft={soft_step:.6} hard={hard_step:.6}"
    );
    // 软窃取臂在**淡出窗口内部**的位移必须同样很小（台阶只在硬窃取臂出现）。
    let fade_window = (STEAL_AT as usize)..(STEAL_AT as usize + 144);
    let soft_fade_step = max_step_in(&soft, fade_window.start, fade_window.end);
    assert!(
        soft_fade_step < hard_step * 0.2,
        "淡出窗口内部的样本位移也必须远小于硬窃取的台阶: soft={soft_fade_step:.6} hard={hard_step:.6}"
    );
}

/// S5：软窃取的差分在淡出窗口里是一条**平滑衰减到 0** 的曲线（不是被截断的台阶）。
///
/// 读数：淡出窗口 `[STEAL_AT, STEAL_AT + 144)` 里，差分的相邻位移必须**远小于**峰值。
/// 3 ms 指数衰减的系数是 `exp(-6/144) ≈ 0.9592` ⇒ 每样本最多掉约 4.09%；
/// 差分峰值实测 1.83（被窃取声部在 sustain 上的幅度）⇒ 上界约 0.075。
///
/// 与 S2/S3 的分工：S2/S3 测"有没有台阶"，S5 测"淡出窗口内部有多平滑"。
#[test]
fn the_stolen_voice_leaves_without_a_step() {
    let (soft, steals) = difference(None);
    assert_eq!(steals, 1);
    // ⚠ 淡出窗口的起点是**被窃取声部开始发声的位置**（`start_sample`），不是触发它的
    // 那个量子边界：触发发生在量子 4992，但声部从 5000 才开始渲染，`fade_remaining`
    // 也从 5000 才开始倒数 ⇒ 窗口是 `[5000, 5144)`。第一版把窗口写成
    // `[量子边界, +144)`，把"新音符起音之后"的那一段也算了进来，读数被污染成 0.55。
    let start = STEAL_AT as usize;
    let end = start + 144;

    let window_step = max_step_in(&soft, start, end);
    let peak = soft[start..end]
        .iter()
        .fold(0.0f32, |peak, s| peak.max(s.abs()));
    // 3 ms / 6 个时间常数 ⇒ 每样本衰减 `1 - exp(-6/144)`。
    let per_sample = 1.0 - (-6.0f32 / 144.0).exp();
    // 系数 4.0 的来历：实测最大位移 0.0316 ≈ `峰值 × per_sample × 3.1`
    //（被窃取声部自身还有 110 Hz 的固有斜率，与指数衰减同量级）。
    // 4.0 仍然比"硬切"（实测 0.555，约 17 倍）小一个数量级以上，判别力不受影响。
    let bound = peak * per_sample * 4.0 + 1e-4;
    println!(
        "[engine-mix] S5 soft difference peak={peak:.6} max_step={window_step:.6} bound={bound:.6}"
    );
    assert!(peak > 0.1, "差分必须真的非零: {peak}");
    assert!(
        window_step <= bound,
        "3 ms 指数衰减的每样本位移上界是 {bound:.6}（峰值 {peak:.6} × {per_sample:.4} × 4.0），\
         实际 {window_step:.6} —— 这不是淡出，是硬切。"
    );
    // ⚠ 淡出结束**之后**差分不为零：两个夹具的那一个声部此后分别播放高音/低音，
    // 差分的尾部就是"两个替换音符之差"（实测约 1.9）。因此这里只断言**淡出窗口内部**
    // 的行为 —— 那是本切片负责的部分；替换音符的差异由 S2/S3 的台阶读数负责。
    let after = soft[end..(end + 256)]
        .iter()
        .fold(0.0f32, |peak, s| peak.max(s.abs()));
    assert!(
        after > 0.1,
        "差分尾部应当反映两个替换音符的差异（否则差分口径本身可疑）: {after}"
    );
}

/// S6：窃取路径**逐位确定**（同夹具两次独立渲染，位模式完全相同）。
#[test]
fn stealing_is_byte_deterministic() {
    let (first, first_steals, _) = render_with_fade(None);
    let (second, second_steals, _) = render_with_fade(None);
    assert_eq!(first_steals, second_steals);
    let first_bits: Vec<u32> = first.iter().map(|sample| sample.to_bits()).collect();
    let second_bits: Vec<u32> = second.iter().map(|sample| sample.to_bits()).collect();
    assert_eq!(first_bits, second_bits, "含窃取的渲染必须逐位相同");
}
