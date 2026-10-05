//! 端到端口径判据：**只走 `yeban-dsp` 的公开面**（下游 `yeban-render` / `yeban-mcp` /
//! `yeban-app` 消费这套口径的方式）。
//!
//! 单元判据住在各自的模块里（能碰到私有核表）；这里额外钉住三件事：
//!
//! 1. 公开 API 的**读数本身**（真峰值 + 无门限 LUFS + 门限 LUFS）与 notes 记录的
//!    数字一致 —— 防止"模块内部对了、公开面接错了"；
//! 2. 门限语义在**公开面**上确实生效（静音段不拉低整体响度）；
//! 3. 口径常量（采样率档位、门限、窗口时长、过采样倍数）是公开且稳定的。
//!
//! 规范锚点：`ARCH-UI-002`（真峰值与 RMS 电平）、`ROAD-M2-008`；
//! 裁决：`HD-26`（真峰值 8×）、`HD-27`（LUFS 门限/窗口 + 其它采样率），
//! 政策 `ADR-0001` D43（1.0.0 之前直接推翻，不留兼容层）。

use yeban_dsp::loudness::{
    ABSOLUTE_GATE_LUFS, GATING_BLOCK_SECONDS, GATING_HOP_SECONDS, GatedLoudness,
    K_WEIGHTING_SAMPLE_RATES_HZ, LoudnessMeter, RELATIVE_GATE_LU, SHORT_TERM_SECONDS,
};
use yeban_dsp::meter::{TRUE_PEAK_PHASES, TRUE_PEAK_PHASES_HIGH, TRUE_PEAK_TAPS, TruePeakDetector};

/// 997 Hz 的正弦夹具（相位用 `f64` 算）。
fn sine(amplitude: f32, frequency: f64, sample_rate: f32, len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let phase = std::f64::consts::TAU * frequency * i as f64 / f64::from(sample_rate);
            (f64::from(amplitude) * phase.sin()) as f32
        })
        .collect()
}

#[test]
fn public_pipeline_matches_the_documented_loudness_numbers() {
    // 1 秒 −20 LUFS 的 997 Hz 立体声正弦 + 1 秒静音。
    let mut samples = sine(0.1, 997.0, 48_000.0, 48_000);
    let tone = samples.clone();
    samples.extend(std::iter::repeat_n(0.0f32, 48_000));

    // 无门限(上一线的语义, 逐位不变): 静音段把读数拉到 −23.0103。
    let ungated = LoudnessMeter::integrated_stereo(&samples, &samples);
    assert!(
        (ungated + 23.010_3).abs() < 0.05,
        "无门限读数应为 −23.0103, 实际 {ungated}"
    );
    // 门限积分(HD-27): 只有"跨在切换点上"的三块损失能量 ⇒ −20.7059。
    let gated = GatedLoudness::integrated_stereo(&samples, &samples);
    assert!(
        (gated + 20.705_9).abs() < 0.05,
        "门限积分读数应为 −20.7059, 实际 {gated}"
    );
    assert!(gated - ungated > 2.0, "门限必须显著抬高读数");

    // 真峰值: 纯正弦的连续峰值就是幅度(0.1), 8× 应收紧到 1e-3 以内。
    let mut detector = TruePeakDetector::new();
    let peak = detector.process(&tone);
    assert!(
        (peak - 0.1).abs() < 1e-3,
        "−20 dBFS 正弦的真峰值应约 0.1, 实际 {peak}"
    );
    assert!(peak >= 0.1, "真峰值不得低于采样峰值");

    // 流式窗口: 1 秒 ⇒ 瞬时读数已更新, 短时(3 s)还没满。
    let mut meter = GatedLoudness::new_48k();
    meter.add_stereo(&tone, &tone);
    assert!(
        (meter.momentary_lufs() + 20.0).abs() < 0.05,
        "瞬时读数应约 −20.0, 实际 {}",
        meter.momentary_lufs()
    );
    assert_eq!(meter.short_term_lufs(), f32::NEG_INFINITY);
}

#[test]
fn public_gates_exclude_quiet_material_and_reject_unsupported_rates() {
    // 远低于绝对门限: 门限积分完全没有可用的块 ⇒ 负无穷(而不是被安静段拉低)。
    let amplitude = (0.1 * 10f64.powf((-75.0 + 20.0) / 20.0)) as f32;
    let quiet = sine(amplitude, 997.0, 48_000.0, 96_000);
    assert_eq!(
        GatedLoudness::integrated_stereo(&quiet, &quiet),
        f32::NEG_INFINITY
    );
    assert!(LoudnessMeter::integrated_stereo(&quiet, &quiet).is_finite());

    // 采样率支持是显式枚举的四档; 其余响亮失败(不静默回落)。
    assert_eq!(
        K_WEIGHTING_SAMPLE_RATES_HZ,
        [44_100.0, 48_000.0, 88_200.0, 96_000.0]
    );
    for rate in K_WEIGHTING_SAMPLE_RATES_HZ {
        assert!(GatedLoudness::for_sample_rate(rate).is_some());
        assert!(LoudnessMeter::for_sample_rate(rate).is_some());
    }
    assert!(GatedLoudness::for_sample_rate(192_000.0).is_none());
    assert!(LoudnessMeter::for_sample_rate(22_050.0).is_none());
}

#[test]
fn public_constants_pin_the_true_peak_and_gating_contract() {
    assert_eq!(TRUE_PEAK_PHASES, 8);
    assert_eq!(TRUE_PEAK_PHASES_HIGH, 16);
    assert_eq!(TRUE_PEAK_TAPS, 32);
    assert_eq!(TruePeakDetector::new().oversampling(), 8);
    assert!(TruePeakDetector::with_oversampling(4).is_none());
    assert!(TruePeakDetector::with_oversampling(16).is_some());

    assert_eq!(ABSOLUTE_GATE_LUFS, -70.0);
    assert_eq!(RELATIVE_GATE_LU, -10.0);
    assert_eq!(GATING_BLOCK_SECONDS, 0.4);
    assert_eq!(GATING_HOP_SECONDS, 0.1);
    assert_eq!(SHORT_TERM_SECONDS, 3.0);
    // 400 ms 块 = 4 × 100 ms 跳(75% 重叠); 3 s 短时 = 30 个跳。
    assert!((GATING_BLOCK_SECONDS / GATING_HOP_SECONDS - 4.0).abs() < 1e-6);
    assert!((SHORT_TERM_SECONDS / GATING_HOP_SECONDS - 30.0).abs() < 1e-6);
}
