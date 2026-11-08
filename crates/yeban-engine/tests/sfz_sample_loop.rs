//! ⭐ 任务单 `#331`：**采样文件自带的循环点**（WAV 的 `smpl` 块）成为缺省窗口。
//!
//! # 缺陷（`#330` 报告逐字）
//!
//! ```text
//! 采样库里大量音色只写 loop_mode=loop_continuous、loop 画在音频编辑器里（WAV smpl 块），
//! 本仓整段不循环 ⇒ 一个音只响一声
//! ```
//!
//! 规范逐字（<https://sfzformat.com/opcodes/loop_start/>）：
//! > If `loop_start` is not specified and the sample has a loop defined, the SFZ player
//! > will use the start point of the first loop defined in the file.
//!
//! ⇒ "SFZ 没写点" ≠ "不循环"，而是"用样本的点"。
//!
//! # 量什么（对象 ＋ 单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 循环窗口 | [`PreparedRegion::loop_window`] 的 `(start, end)` | 样本帧（半开区间） |
//! | 渲染输出 | 斜坡样本（每帧值 ＝ 该帧下标）逐帧回绕的位置 | 样本帧 |
//!
//! # 判据
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | [`the_sample_loop_supplies_the_window_when_sfz_writes_nothing`] | 样本 loop `(100,300)` ＋ SFZ 只写 `loop_mode` ⇒ 窗口 `(100,300)` |
//! | [`the_render_wraps_inside_the_sample_loop`] | 斜坡样本：输出序列在 100..300 之间回绕（⛔ 不是播到样本尾就停） |
//! | [`sfz_points_override_the_sample_loop`] | SFZ 显式写 `(10,50)` ＋ 样本 `(100,300)` ⇒ 窗口 `(10,50)`（SFZ 优先） |
//! | [`no_sample_loop_keeps_the_whole_sample_window`] | 样本没有 loop ⇒ 窗口仍是 `(0, pcm_len)`（＝ 接线前的行为） |
//! | [`a_degenerate_sample_loop_is_ignored`] | 样本 loop `end <= start` ⇒ `None`（⛔ 不造零长窗口） |
//!
//! # 边界（⛔ 不声称）
//!
//! - ⛔ 不证"逐点回退"（只写 `loop_end` 时 `loop_start` 取样本的）：本仓的
//!   `Region::loop_start` 是 `u32`、缺省 `0`，"写了 0"与"没写"不可区分 ⇒ 裁决是
//!   **整体回退**（`loop_end == 0` 时才算"没写点"）。偏离与理由写在
//!   [`reduce_loop_window`] 的文档里；
//! - ⛔ 不证 `smpl` 的**多 loop**：只读第一个（规范逐字"the first loop defined in the file"）。

use std::sync::Arc;

use yeban_engine::sfz_instrument::{
    InMemorySample, PreparedSfz, SampleLoop, SfzInstrument, SfzVoicePool,
};

const PCM: usize = 4_096;

/// 斜坡样本：第 `i` 帧的值 ＝ `i as f32` ⇒ 输出序列能**逐帧反解**位置。
fn ramp_sample(loop_points: Option<SampleLoop>) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new((0..PCM).map(|index| index as f32).collect()),
        sample_hz: 48_000.0,
        loop_points,
    }
}

fn prepare(text: &str, sample: &InMemorySample) -> PreparedSfz {
    let instrument =
        SfzInstrument::load(text.to_owned(), &Default::default()).expect("夹具 SFZ 必须解析成功");
    PreparedSfz::prepare(&instrument, sample, 48_000.0).expect("摊平必须成功")
}

/// SFZ 只写 `loop_mode`，**一个点都不写**（采样库的常态）。
const MODE_ONLY: &str =
    "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous\n";

/// 判据 ①：样本自带的 loop 成为窗口。
#[test]
fn the_sample_loop_supplies_the_window_when_sfz_writes_nothing() {
    let sample = ramp_sample(Some(SampleLoop {
        start: 100,
        end: 300,
    }));
    let plan = prepare(MODE_ONLY, &sample);
    let window = plan.regions()[0].loop_window(PCM, sample.loop_points);
    println!("[engine-sfz/#331-smpl] 窗口 = {window:?}");
    assert_eq!(
        window.map(|found| (found.start, found.end)),
        Some((100, 300)),
        "⭐ SFZ 没写点 ⇒ 必须用样本的 `smpl` loop（100,300）"
    );
}

/// 判据 ②：**真的回绕**（⛔ 不只是窗口读数对）。
///
/// 斜坡样本下输出值就是"当前源位置"⇒ 序列必须是
/// `0,1,…,99,100,…,299,100,101,…`。这一条把"窗口算对了但播放没回绕"的实现钉红。
#[test]
fn the_render_wraps_inside_the_sample_loop() {
    let sample = ramp_sample(Some(SampleLoop {
        start: 100,
        end: 300,
    }));
    let plan = prepare(MODE_ONLY, &sample);
    let mut pool = SfzVoicePool::new(48_000.0);
    assert_eq!(pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, None), 1);
    let mut out = vec![0.0f32; 700];
    pool.render_into(&plan, 0, &mut out);
    // 前 100 帧是样本头（0..100），之后进入窗口并回绕。
    let expected: Vec<f32> = (0..700)
        .map(|frame| {
            if frame < 100 {
                frame as f32
            } else {
                (100 + (frame - 100) % 200) as f32
            }
        })
        .collect();
    let mismatches = out
        .iter()
        .zip(&expected)
        .filter(|(got, want)| (*got - *want).abs() > 1.0e-3)
        .count();
    println!(
        "[engine-sfz/#331-smpl] 回绕：不符 {mismatches} 帧；第 100/300/500 帧 = {} / {} / {}",
        out[100], out[300], out[500]
    );
    assert_eq!(
        mismatches, 0,
        "⭐ 输出必须在 [100,300) 内回绕（斜坡可逐帧反解）"
    );
    assert_eq!(
        (out[100], out[300], out[500]),
        (100.0, 100.0, 100.0),
        "⭐ 回绕点：第 100/300/500 帧都回到窗口起点"
    );
}

/// 判据 ③：SFZ 显式写的点**覆盖**样本的点。
#[test]
fn sfz_points_override_the_sample_loop() {
    let sample = ramp_sample(Some(SampleLoop {
        start: 100,
        end: 300,
    }));
    let plan = prepare(
        "<region> sample=a.wav key=60 pitch_keycenter=60 loop_mode=loop_continuous \
         loop_start=10 loop_end=50\n",
        &sample,
    );
    let window = plan.regions()[0].loop_window(PCM, sample.loop_points);
    println!("[engine-sfz/#331-smpl] 覆盖后窗口 = {window:?}");
    assert_eq!(
        window.map(|found| (found.start, found.end)),
        Some((10, 50)),
        "⭐ SFZ 的 `loop_start`/`loop_end` **永远优先**（规范：\"it will be used instead\"）"
    );
}

/// 判据 ④：样本没有 loop ⇒ 窗口仍是"整段"（＝ 接线前的行为，⛔ 不是不循环）。
#[test]
fn no_sample_loop_keeps_the_whole_sample_window() {
    let sample = ramp_sample(None);
    let plan = prepare(MODE_ONLY, &sample);
    let window = plan.regions()[0].loop_window(PCM, sample.loop_points);
    println!("[engine-sfz/#331-smpl] 无 smpl 窗口 = {window:?}");
    assert_eq!(
        window.map(|found| (found.start, found.end)),
        Some((0, PCM as u32)),
        "⭐ 没有 `smpl` ⇒ 沿用 `#311` 的缺省（`loop_end == 0` ⇒ 样本尾），逐位不变"
    );
}

/// 判据 ⑤：退化的样本 loop（`end <= start`）⇒ **不循环**（⛔ 不造零长窗口）。
#[test]
fn a_degenerate_sample_loop_is_ignored() {
    let sample = ramp_sample(Some(SampleLoop {
        start: 300,
        end: 300,
    }));
    let plan = prepare(MODE_ONLY, &sample);
    let window = plan.regions()[0].loop_window(PCM, sample.loop_points);
    println!("[engine-sfz/#331-smpl] 退化 smpl 窗口 = {window:?}");
    assert!(
        window.is_none(),
        "⭐ 退化窗口必须判 `None`（与 `loop_end <= loop_start` 同一条出口），\
         否则回绕会把位置卡在同一个样本上（听感是「直流/爆音」）"
    );
}
