//! ⭐ 任务单 `#331`：`off_mode=time` 真的读 **`off_time`**（此前按 `normal` 处理）。
//!
//! # 缺陷（`ADR-0008` 裁决 **C** 的书面偏离，本轮撤销）
//!
//! ```text
//! off_mode=time 被当成 normal：用 ampeg_release 决定关断长度，off_time 一个字节都不读
//! ```
//!
//! 规范逐字（两页一起读才是完整语义）：
//!
//! * <https://sfzformat.com/opcodes/off_mode/> —— ARIA 的 `time` "can be used to specify a
//!   time **independent of the sample release time** using `off_time`"；
//! * <https://sfzformat.com/opcodes/off_time/> —— "When `off_mode` is set to time, this
//!   specifies the fadeout time for regions being muted by voice-stealing."（缺省 `0.006`）
//!
//! ⇒ `time` 的淡出长度由 `off_time` **唯一**决定，⛔ 与 `ampeg_release` 无关。
//!
//! # 量什么（对象 ＋ 单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 非零帧数 | [`SfzVoicePool`] 的渲染输出里 `!= 0.0` 的帧 | 帧（@48 kHz） |
//! | `end_sample` | 被关断声部的引擎终点 | `u64`（`u64::MAX` ＝ 未改） |
//! | 包络段位 | 被关断声部的段位读数 | `AmpegStage` |
//!
//! # 判据
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`off_time_decides_the_fadeout_not_the_release`] | `off_time=0.01` ＋ `ampeg_release=1.0` ⇒ 淡出 **480** 帧（⛔ 48000） |
//! | [`the_fadeout_is_independent_of_the_release`] | 同一 `off_time`，`ampeg_release` 0.01 / 2.0 ⇒ 淡出长度**相同** |
//! | [`the_default_off_time_is_six_milliseconds`] | 不写 `off_time` ⇒ **288** 帧（6 ms @ 48 kHz） |
//! | [`a_zero_off_time_hard_stops`] | `off_time=0` ⇒ 关断后**一个非零帧都没有**（⛔ 不是"永远响"） |
//! | [`normal_still_walks_the_ampeg_release`] | 负对照：`off_mode=normal` ⇒ 走 `ampeg_release`（480 帧线性），与 `time` 的读数**不同** |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证曲线形状与 ARIA 一致：本仓用的是**既有**窃取淡出（指数到 `STEAL_FADE_FLOOR`），
//!   `off_shape` / `off_curve` 仍未做（台账登记）；
//! - ⛔ 不证 `note-off` 会看 `off_time`：规范把 `off_mode` 的作用面限定在 `off_by`
//!   （"how a region is turned off by an `off_by` opcode"）⇒ note-off 走 `ampeg_release`。

use std::sync::Arc;

use yeban_engine::sfz_instrument::{
    InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool, seconds_to_frames,
};

/// 关断发生在第几帧（与 `S39` 同一条夹具节奏）。
const KILL_AT: usize = 200;
/// 样本 PCM 长度（`flat_sample` 下最多写 `PCM − 1` 帧）。
const PCM: usize = 65_536;

fn flat_sample(frames: usize) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![1.0f32; frames]),
        sample_hz: 48_000.0,
        loop_points: None,
    }
}

fn prepare_with(text: &str, sample: &InMemorySample) -> PreparedSfz {
    let instrument =
        SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare(&instrument, sample, 48_000.0).expect("摊平必须成功")
}

/// 夹具：A 出声（`group=1`）＋ B 是规范登记的**静音关断区**（`end=-1`，`off_by=1`）。
///
/// 与 `S39` 的夹具只差 `off_mode` 与两个时间参数 —— 这样"关断长度"的差异**只能**来自
/// 本单改的那三样东西。
fn off_by_sfz(off_mode: &str, off_time: Option<&str>, release: f32) -> String {
    let off_time = off_time.map_or_else(String::new, |value| format!("off_time={value} "));
    format!(
        "<region> sample=a.wav key=60 pitch_keycenter=60 group=1 off_mode={off_mode} {off_time}\
         ampeg_release={release}\n\
         <region> sample=s.wav key=62 pitch_keycenter=62 end=-1 off_by=1\n"
    )
}

/// 跑一次"发起 → 关断 → 收尾"，返回（全部输出, 被关声部的 `end_sample` 关断后读数）。
fn run_and_kill(sfz: &str, tail: usize) -> (Vec<f32>, u64) {
    let sample = flat_sample(PCM);
    let plan = prepare_with(sfz, &sample);
    let mut pool = SfzVoicePool::new(48_000.0);
    assert_eq!(
        pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None),
        1,
        "A 必须建 1 个声部"
    );
    let mut out = vec![0.0f32; KILL_AT + tail];
    pool.render_into(&plan, 0, &mut out[..KILL_AT]);
    assert!(
        out[..KILL_AT].iter().all(|value| *value == 1.0),
        "⭐ 前置见证：关断之前 A 恒 1.0（attack 缺省 0、sustain 缺省 100 %）"
    );
    assert_eq!(
        pool.trigger_note(&plan, 62, 127, u64::MAX, 1.0, None),
        0,
        "⭐ 静音关断区（`end=-1`）⛔ 不建声部"
    );
    let end_sample = pool.voices()[0].end_sample();
    pool.render_into(&plan, KILL_AT as u64, &mut out[KILL_AT..]);
    (out, end_sample)
}

fn nonzero_tail(out: &[f32]) -> usize {
    out[KILL_AT..].iter().filter(|value| **value != 0.0).count()
}

/// 判据 ①：`off_time` 决定淡出长度，**`ampeg_release` 不参与**。
#[test]
fn off_time_decides_the_fadeout_not_the_release() {
    // 0.01 s @ 48 kHz = 480 帧；`ampeg_release=1.0` = 48000 帧 —— 两者差 100 倍，
    // 读数落在哪一边**一眼可判**。
    let (out, end_sample) = run_and_kill(&off_by_sfz("time", Some("0.01"), 1.0), 1_000);
    let faded = nonzero_tail(&out);
    println!(
        "[engine-sfz/#331] time: off_time=0.01 release=1.0 ⇒ 淡出 {faded} 帧, end_sample={end_sample}"
    );
    assert_eq!(
        faded,
        seconds_to_frames(0.01, 48_000.0) as usize,
        "⭐ 淡出长度必须 ＝ off_time（480 帧），⛔ 不是 ampeg_release 的 48000"
    );
    assert_eq!(
        end_sample,
        u64::MAX,
        "⭐ `time` ⛔ 不挪 `end_sample`（挪了就是走 ampeg 释放，＝ `normal` 的行为）"
    );
    assert!(
        out[KILL_AT + faded..].iter().all(|value| *value == 0.0),
        "⭐ 淡出走完 ⇒ 逐位 0（声部已退休）"
    );
}

/// 判据 ②：**独立性** —— 同一 `off_time`，把 `ampeg_release` 从 0.01 拉到 2.0，
/// 淡出长度**一帧都不变**。
#[test]
fn the_fadeout_is_independent_of_the_release() {
    let (short, _) = run_and_kill(&off_by_sfz("time", Some("0.02"), 0.01), 1_200);
    let (long, _) = run_and_kill(&off_by_sfz("time", Some("0.02"), 2.0), 1_200);
    let (short_tail, long_tail) = (nonzero_tail(&short), nonzero_tail(&long));
    println!(
        "[engine-sfz/#331] 独立性: release=0.01 ⇒ {short_tail} 帧；release=2.0 ⇒ {long_tail} 帧"
    );
    assert_eq!(
        short_tail,
        seconds_to_frames(0.02, 48_000.0) as usize,
        "0.02 s ⇒ 960 帧"
    );
    assert_eq!(
        short_tail, long_tail,
        "⭐ `independent of the sample release time`：release 改 200 倍，淡出长度不变"
    );
}

/// 判据 ③：**缺省 6 ms** —— 源文件不写 `off_time` 时，规范缺省是 `0.006 s`（288 帧）。
#[test]
fn the_default_off_time_is_six_milliseconds() {
    let (out, _) = run_and_kill(&off_by_sfz("time", None, 1.0), 1_000);
    let faded = nonzero_tail(&out);
    println!("[engine-sfz/#331] 缺省: 不写 off_time ⇒ 淡出 {faded} 帧");
    assert_eq!(
        faded,
        seconds_to_frames(0.006, 48_000.0) as usize,
        "⭐ 缺省 `off_time` 必须是 6 ms（288 帧 @ 48 kHz），⛔ 不是 0"
    );
    assert_eq!(faded, 288, "28.8 帧/ms × 10？⛔：6 ms @ 48 kHz 恰 288 帧");
}

/// 判据 ④：`off_time=0` ⇒ **立刻停**（⛔ 不是"淡出 0 帧 ⇒ 永远响"）。
#[test]
fn a_zero_off_time_hard_stops() {
    let (out, end_sample) = run_and_kill(&off_by_sfz("time", Some("0"), 1.0), 500);
    let faded = nonzero_tail(&out);
    println!("[engine-sfz/#331] off_time=0 ⇒ 关断后非零 {faded} 帧, end_sample={end_sample}");
    assert_eq!(
        faded, 0,
        "⭐ `off_time=0` ＝ 立刻停（`FadeRamp::new(0)` 是 `NONE` ⇒ 必须显式硬停，\
         ⛔ 否则声部会一直响下去）"
    );
    assert!(
        out[..KILL_AT].iter().all(|value| *value == 1.0),
        "关断之前照常发声（证明不是夹具本身静音）"
    );
}

/// 判据 ⑤：**负对照** —— `off_mode=normal` 仍走 `ampeg_release`（480 帧线性）。
///
/// 两条路的**非零长度可能偶然相同**（这里 `off_time=0.01` 与 `release=0.01` 都是 480），
/// 所以额外比**第 100 帧的电平**：`normal` 是线性 `(R−k)/R`（0.7917），`time` 是指数
/// （`1e-3^(100/480) ≈ 0.2371`）。两者差一个数量级 ⇒ 这条判据不可能被"碰巧"满足。
#[test]
fn normal_still_walks_the_ampeg_release() {
    let (normal, normal_end) = run_and_kill(&off_by_sfz("normal", None, 0.01), 1_000);
    let (timed, timed_end) = run_and_kill(&off_by_sfz("time", Some("0.01"), 0.01), 1_000);
    let release_frames = seconds_to_frames(0.01, 48_000.0) as usize;
    println!(
        "[engine-sfz/#331] 对照: normal 非零 {} 帧 (end_sample={normal_end}, 第 100 帧 {:.4})；\
         time 非零 {} 帧 (end_sample={timed_end}, 第 100 帧 {:.4})",
        nonzero_tail(&normal),
        normal[KILL_AT + 100],
        nonzero_tail(&timed),
        timed[KILL_AT + 100]
    );
    assert_eq!(
        normal_end, 0,
        "⭐ `normal` 把 victim 的 note-off 挪到「现在」（`end_sample = 0`，既有 note-off 机接手）"
    );
    assert_eq!(
        nonzero_tail(&normal),
        release_frames,
        "⭐ `normal` 的长度由 `ampeg_release` 决定（0.01 s ⇒ 480 帧）"
    );
    let linear = (release_frames - 100) as f32 / release_frames as f32;
    assert!(
        (normal[KILL_AT + 100] - linear).abs() < 1.0e-5,
        "⭐ `normal` 的释放段是线性 (R−k)/R（期望 {linear}，实得 {}）",
        normal[KILL_AT + 100]
    );
    assert_eq!(
        timed_end,
        u64::MAX,
        "⭐ `time` ⛔ 不挪 `end_sample`（这就是两条路的**结构性**差异）"
    );
    assert!(
        timed[KILL_AT + 100] < normal[KILL_AT + 100] * 0.5,
        "⭐ `time` 的曲线与 `normal` 不同（指数 vs 线性）：{} vs {}",
        timed[KILL_AT + 100],
        normal[KILL_AT + 100]
    );
}
