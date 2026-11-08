//! ⭐ **`loop_mode` 的规范缺省：不写窗口 ≠ 不循环**（任务 `#311`）。
//!
//! # 这条判据在修什么
//!
//! `sfzformat.com/opcodes/loop_end/` 逐字：**"Default: `-1` (end of sample)"**，
//! `loop_start` 缺省 `0`。也就是说
//!
//! ```text
//! <region> sample=a.wav loop_mode=loop_continuous
//! ```
//!
//! 的语义是**循环整个样本**。而本仓把未写的 `loop_end` 读成 `0`（`read_u32(.., 0)`），
//! `reduce_loop_window` 又按 `loop_end <= loop_start ⇒ None` 判死 ⇒ **一个本该延音的
//! 长音在样本尾被硬切**。
//!
//! 这不是边角情形：`vsco2ce` 全套弦乐、以及绝大多数"整段循环"的音色都只写
//! `loop_mode`、不写窗口。用户听到的是"弦乐断气"、"长音突然没声"，而 `.sfz` 与采样都是好的。
//!
//! # 判据
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | L1 | **只写 `loop_mode`** ⇒ 循环整个样本 | 输出 0.5 s、样本 0.2 s ⇒ `written == 0.5 s`，且 `out[t]` 与 `out[t + pcm_len]` **逐位相同**（斜坡夹具可逐帧反解下标） |
//! | L2 | `loop_end` **超出** `source_end` ⇒ **钳位后仍循环**（⛔ 不是判死） | 与 L1 **逐位相同** |
//! | L3 | **反向腿**：`no_loop` ⇒ 不循环 | `written == pcm_len`（⛔ 不是"写满输出"），证明 L1 的"循环"不是空绿 |
//! | L4 | **退化窗口不变**：`loop_end <= loop_start` ⇒ `None` | 与判据 `S31` 同口径（本修订⛔ 不碰它） |
//!
//! ⚠ L1／L2 的"逐位相同"用斜坡夹具：`pcm[i] = i / 1024`，`rate = 1`（`key` 与
//! `pitch_keycenter` 同为 60）⇒ 输出值可以**反解**成源下标，回绕点一处对不上就红。

use std::collections::BTreeMap;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument};

/// 引擎采样率。
const HZ: f32 = 48_000.0;
/// 样本长度（帧）：0.2 s。
const PCM: usize = 9_600;
/// 输出长度（帧）：0.5 s（＝ 2.5 个样本长，循环 2 次以上）。
const OUT: usize = 24_000;

/// 斜坡样本：`pcm[i] = i / 1024` ⇒ 输出值可反解下标。
fn ramp() -> InMemorySample {
    let pcm: Vec<f32> = (0..PCM).map(|i| i as f32 / 1024.0).collect();
    InMemorySample {
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 渲染一份 SFZ（单区域），返回（写入帧数, 输出缓冲）。
fn render(ops: &str) -> (usize, Vec<f32>) {
    let text = format!("<region> sample=a.wav key=60 pitch_keycenter=60 {ops}");
    let sfz = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    let mut table = BTreeMap::new();
    table.insert("a.wav".to_owned(), ramp());
    let mut out = vec![0.0_f32; OUT];
    let written = sfz
        .render_note_into_from_samples(&table, 60, 100, Some(OUT as u64), None, &mut out, HZ)
        .expect("渲染");
    (written, out)
}

/// ⭐ L1：只写 `loop_mode` ⇒ 循环整个样本。
#[test]
fn l1_loop_mode_without_a_window_loops_the_whole_sample() {
    let (written, out) = render("loop_mode=loop_continuous");
    assert_eq!(
        written, OUT,
        "⭐ 只写 `loop_mode=loop_continuous`（规范缺省 `loop_end = -1` ＝ 样本尾）⇒ \
         必须能循环、写满输出；实得 {written} 帧（＝ 样本长 {PCM} ⇒ 说明仍在'播完就停'）"
    );
    // 周期 = PCM（整段循环）。`t` 从 PCM 起逐帧比对 —— 回绕点错一格就红。
    let mut worst = 0.0_f32;
    let mut worst_at = 0usize;
    for t in PCM..OUT - PCM {
        let d = (out[t] - out[t - PCM]).abs();
        if d > worst {
            worst = d;
            worst_at = t;
        }
    }
    assert!(
        worst < 1e-6,
        "⭐ 循环周期必须恰好 = 样本长 {PCM} 帧：`t = {worst_at}` 处与 `t - {PCM}` 相差 {worst}"
    );
}

/// ⭐ L2：`loop_end` 超出 `source_end` ⇒ 钳位后仍循环。
#[test]
fn l2_loop_end_beyond_the_sample_is_clamped_not_rejected() {
    let (written, out) = render("loop_mode=loop_continuous loop_end=999999");
    assert_eq!(
        written, OUT,
        "⭐ `loop_end` 越界必须**钳到** `source_end`（⛔ 不是整条判死）；实得 {written} 帧"
    );
    // 与 L1 同解：钳位之后就是"循环整个样本"。
    let (_, reference) = render("loop_mode=loop_continuous");
    let mut worst = 0.0_f32;
    for (a, b) in out.iter().zip(&reference) {
        worst = worst.max((a - b).abs());
    }
    assert!(
        worst < 1e-6,
        "⭐ 钳位后必须与'只写 loop_mode'**逐位相同**（差 {worst}）"
    );
}

/// ⭐ L3：反向腿 —— `no_loop` 不循环（证明 L1 的"循环"不是空绿）。
///
/// ⚠ 期望 `PCM - 1` 而不是 `PCM`：`mix_span_into` 的停止条件是"**插值要读的下一格**
/// 仍在 PCM 内"，最后一帧的右邻不存在 ⇒ 差一帧是**既有语义**（判据 `O3` 的注释里
/// 记过同一条）。写 `PCM` 会把这条既有语义当成 bug。
#[test]
fn l3_no_loop_still_stops_at_the_sample_end() {
    let (written, _) = render("loop_mode=no_loop");
    assert_eq!(
        written,
        PCM - 1,
        "⭐ `no_loop` 必须播到样本尾就停（⛔ 不是写满输出）；实得 {written} 帧"
    );
}

/// ⭐ L4：退化窗口不变（与判据 `S31` 同口径）。
#[test]
fn l4_degenerate_window_is_still_not_a_loop() {
    let (written, _) = render("loop_mode=loop_continuous loop_start=100 loop_end=100");
    assert_eq!(
        written,
        PCM - 1,
        "⭐ `loop_end <= loop_start` ⇒ 既有语义是**按不循环渲染**（⛔ 死循环窗口）；实得 {written} 帧"
    );
}
