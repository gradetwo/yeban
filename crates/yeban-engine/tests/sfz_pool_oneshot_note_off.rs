//! ⭐ **实时池路的 `one_shot` 语义**（判据 P1／P2；Wang 2026-11-18 裁决「甲」）。
//!
//! # 裁决与背景
//!
//! `loop_mode=one_shot` 的声部**完全忽略 note-off、不进 `ampeg` release 段**
//!（与 Logic Pro Quick Sampler／Sampler 及 sfzformat 一致；掐尾由 `off_mode`
//! 独占组承担，⛔ 不由 release 段兼任）。
//!
//! 核查线已证实三条路径（调度／离线／实时池）现行语义均已符合，但具名判据此前
//! 只有离线路有（O5，见 `sfz_offline_note_off.rs`）；实时池路
//!（[`SfzVoicePool`]）靠 `slot_readings` 屏蔽与模块头文档钉住，**没有具名判据**。
//! 本文件补这一条，防日后静默回归。
//!
//! # 判据表
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | P1 | 池路 `one_shot` 忽略 note-off | 0.35 s 处 `release_note` 的渲染与从不松键的渲染**逐 `f32` 位相同**；两者实测 T40 均等于源 T40（1.600 s，容差 ±0.01 s） |
//! | P2 | 对照：去掉 `one_shot` 后 release 必须生效 | 同一夹具无 `one_shot`：不松键 T40 仍 ≈ 1.600 s；0.35 s 松键后 T40 < 1.05 s 且比不松键短 ≥ 0.30 s（证明 P1 不是假绿） |
//!
//! 夹具自包含：`<global>` 写 `ampeg_release=0.7`（P1 另加 `loop_mode=one_shot`），
//! 样本为指数衰减合成 PCM（源 T40 由构造钉死为 1.600 s），不依赖任何真实 kit。

use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

/// 引擎采样率（判据里的"帧"一律指它）。
const HZ: f32 = 48_000.0;
/// 触发音符（与区域 `key=60` 对齐 ⇒ 速率恰 1.0、无重采样漂移）。
const NOTE: u8 = 60;
/// 源样本的 T40（秒）：构造时钉死。
const SOURCE_T40_S: f64 = 1.600;
/// 样本长度：3.0 s（渲染窗 2.5 s 内声部不会撞到样本尾）。
const SAMPLE_FRAMES: usize = 144_000;
/// 渲染总长：2.5 s（越过 T40 的 1.6 s、未到样本尾 3.0 s）。
const RENDER_FRAMES: usize = 120_000;
/// 渲染块长：480 帧（10 ms，与实时回调同量级；松键帧须是它的整数倍）。
const BLOCK: usize = 480;
/// 松键帧：16 800 ＝ 0.35 s（衰减中途；此时源电平约 −8.75 dB，release 若生效必可检出）。
const RELEASE_FRAME: u64 = 16_800;

/// 指数衰减合成样本：`pcm(t) = 0.8 × 10^(−40·t / (20·T40))`，即电平每 T40 秒掉 40 dB。
fn decay_sample() -> InMemorySample {
    let pcm: Vec<f32> = (0..SAMPLE_FRAMES)
        .map(|n| {
            let t = f64::from(n as u32) / f64::from(HZ);
            (0.8 * 10.0_f64.powf(-2.0 * t / SOURCE_T40_S)) as f32
        })
        .collect();
    InMemorySample {
        channels: 1,
        pcm: Arc::new(pcm),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 摊平一份单区域 SFZ：`<global>` 带 `global_ops`，区域只有 `sample=` 与 `key=`。
fn prepare(global_ops: &str) -> PreparedSfz {
    let text = format!("<global> {global_ops}\n<region> sample=a.wav key=60");
    let instrument = SfzInstrument::load(text, &Default::default()).expect("SFZ 必须解析成功");
    PreparedSfz::prepare_with_samples(&instrument, &[("a.wav".to_owned(), decay_sample())], HZ)
        .expect("摊平必须成功")
}

/// 经池路渲染整段：触发后逐块 `render_into`；`release_at` 给出时在该帧处调
/// `release_note`（并断言它命中了声部 —— 没命中则逐位相同是假绿）。
fn render(plan: &PreparedSfz, release_at: Option<u64>) -> Vec<f32> {
    let mut pool = SfzVoicePool::new(HZ);
    let created = pool.trigger_note(plan, NOTE, 127, u64::MAX, 1.0, None);
    assert_eq!(created, 1, "单区域夹具必须恰好建 1 个声部");
    let mut out = vec![0.0_f32; RENDER_FRAMES];
    let mut released = false;
    let mut position = 0_usize;
    while position < RENDER_FRAMES {
        if let Some(at) = release_at
            && !released
            && position as u64 >= at
        {
            assert!(
                pool.release_note(plan, NOTE, at),
                "release_note 必须命中在发声的声部（未命中 ⇒ 对照无意义）"
            );
            released = true;
        }
        let end = (position + BLOCK).min(RENDER_FRAMES);
        pool.render_into(plan, position as u64, &mut out[position..end]);
        position = end;
    }
    if release_at.is_some() {
        assert!(released, "渲染窗内必须真的调过 release_note");
    }
    out
}

/// 实测 T40（秒）：峰值 −40 dB 阈值之上最后一帧的时刻。
fn t40_seconds(buf: &[f32]) -> f64 {
    let peak = buf.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "夹具必须出声");
    let threshold = peak * 0.01;
    let last = buf
        .iter()
        .rposition(|v| v.abs() >= threshold)
        .expect("衰减必越过 −40 dB 阈值");
    last as f64 / f64::from(HZ)
}

/// 逐 `f32` 位比较两段渲染，第一处不同即失败并报帧号。
fn assert_bit_identical(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}：长度不同");
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x.to_bits(),
            y.to_bits(),
            "{what}：第 {index} 帧逐位不同：{x} vs {y}"
        );
    }
}

/// 判据 P1：池路 `one_shot` 声部完全忽略 note-off —— 提前松键与从不松键逐位
/// 相同，且 T40 等于源 T40（release 段 0.7 s 一帧都没有插进来）。
#[test]
fn p1_pool_one_shot_ignores_note_off() {
    let plan = prepare("loop_mode=one_shot ampeg_release=0.7");
    let short = render(&plan, Some(RELEASE_FRAME));
    let long = render(&plan, None);

    assert_bit_identical(&short, &long, "one_shot 短/长音符");

    let peak = long.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
    assert!(
        (peak - 0.8).abs() <= 0.008,
        "峰值应为样本首值 0.8（力度 127 ⇒ 增益 1.0）：实测 {peak}"
    );
    // 1.2 s 处的电平锚点：源衰减 0.8 × 10^(−1.5) ≈ 0.0253（包络 sustain 段恒 1.0）。
    let at_1_2s = long[57_600];
    assert!(
        (at_1_2s - 0.025_3).abs() <= 0.000_6,
        "1.2 s 电平应贴源衰减 ≈ 0.0253：实测 {at_1_2s}"
    );

    for (name, buf) in [("短音符", &short), ("长音符", &long)] {
        let t40 = t40_seconds(buf);
        assert!(
            (t40 - SOURCE_T40_S).abs() <= 0.01,
            "{name} T40 应等于源 T40 {SOURCE_T40_S} s（容差 ±0.01 s）：实测 {t40}"
        );
    }
}

/// 判据 P2（对照）：同一夹具去掉 `one_shot`，`release_note` 必须真的把声部
/// 推入 release 段 —— 松键后 T40 显著短于不松键（证明 P1 的判据能检出 release）。
#[test]
fn p2_pool_without_one_shot_release_shortens_t40() {
    let plan = prepare("ampeg_release=0.7");
    let short = render(&plan, Some(RELEASE_FRAME));
    let long = render(&plan, None);

    let long_t40 = t40_seconds(&long);
    assert!(
        (long_t40 - SOURCE_T40_S).abs() <= 0.01,
        "对照长音符 T40 应仍等于源 T40 {SOURCE_T40_S} s：实测 {long_t40}"
    );
    let short_t40 = t40_seconds(&short);
    assert!(
        short_t40 > 0.35 && short_t40 < 1.05,
        "对照短音符 T40 必须落在 release 窗内（0.35 s 松键＋0.7 s release 段走完前\
         越过 −40 dB）：实测 {short_t40}"
    );
    assert!(
        long_t40 - short_t40 >= 0.30,
        "对照短/长 T40 必须显著拉开（≥ 0.30 s）：长 {long_t40} vs 短 {short_t40}"
    );
    // release 段走完（0.35 + 0.7 = 1.05 s）之后，1.2 s 处短音符必须已静音、
    // 长音符仍按源衰减在响。
    assert!(
        short[57_600].abs() <= 1.0e-6,
        "对照短音符 1.2 s 处应已静音：实测 {}",
        short[57_600]
    );
    assert!(
        long[57_600] > 0.02,
        "对照长音符 1.2 s 处仍应出声：实测 {}",
        long[57_600]
    );
}
