//! B1 端到端判据（引擎段）：**440 Hz 正弦 PCM → `SfzInstrument::render_note_into`
//! → 测音高**。
//!
//! ## 来历（B1 裁决 (c) 不复现，2026-11-14 实测钉住）
//!
//! 报告称「单声道 FLAC 播放系统性低约 100 音分」。盒上 ffmpeg 夹具三段量测全清
//! （解码两路 +0.00 音分、MCP 工具渲染 −0.05~+0.16 音分），本段是引擎离线渲染
//! 的同轮读数：
//!
//! | 样本率 | 引擎率 48,000 | 引擎率 44,100 |
//! | --- | --- | --- |
//! | 44,100 | −1.23 音分 | +0.00 音分 |
//! | 48,000 | +0.00 音分 | +0.61 音分 |
//!
//! 非零读数只出现在样本率 ≠ 引擎率的重采样对上且符号随方向翻转，量级约 1 音分，
//! 与宣称的约 100 音分差两个数量级 ⇒ 播放侧速率口径无系统性偏差。本判据防未来
//! 回归，门限取 **10 音分**（宣称量级的 1/10；同率对实测 0.00）。
//!
//! ## 夹具自包含
//!
//! PCM 在测试内按 `i16 / 32768` 标度生成（值 = `round(0.75 * 32767 * sin)`），
//! 与 `yeban-mcp` 的 `flac_mono_pitch_e2e` 判据里 FLAC 夹具的解码输出逐值同构
//! ——FLAC 特异性由那份判据的解码段钉住，本文件只隔离引擎消费
//! `(pcm, sample_hz)` 的口径：单区域 `key=69 pitch_keycenter=69`、演奏音高 69
//! （音高比 = 1 的不动点）。
//!
//! ## 判定量的口径
//!
//! 基频由 0.25 s..1.5 s 稳态段内**上升零交叉**（线性插值定分数时刻）的平均周期
//! 换算；报告量为相对 440.00 Hz 的音分偏差 `1200 * log2(f / 440)`。

use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument};

fn sine_pcm(rate: u32, total_samples: usize) -> Vec<f32> {
    (0..total_samples)
        .map(|n| {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * n as f64 / f64::from(rate);
            let v = (0.75 * 32_767.0 * phase.sin()).round() as i16;
            f32::from(v) / 32_768.0
        })
        .collect()
}

fn measure_freq_cents(pcm: &[f32], rate: u32, from_s: f64, to_s: f64) -> (f64, f64) {
    let start = (from_s * f64::from(rate)) as usize;
    let end = ((to_s * f64::from(rate)) as usize).min(pcm.len());
    assert!(end > start + 2, "量测窗口为空");
    let mut crossings: Vec<f64> = Vec::new();
    for i in start..end - 1 {
        let (a, b) = (f64::from(pcm[i]), f64::from(pcm[i + 1]));
        if a < 0.0 && b >= 0.0 {
            crossings.push(i as f64 + (-a / (b - a)));
        }
    }
    assert!(
        crossings.len() >= 100,
        "交叉点太少（{}），信号可能不是 440 Hz 正弦",
        crossings.len()
    );
    let span = crossings[crossings.len() - 1] - crossings[0];
    let freq = (crossings.len() - 1) as f64 / span * f64::from(rate);
    (freq, 1200.0 * (freq / 440.0).log2())
}

#[test]
fn engine_render_pitch_is_exact() {
    let sfz = SfzInstrument::load(
        "<region> sample=tone.wav key=69 pitch_keycenter=69\n".to_owned(),
        &Default::default(),
    )
    .expect("SFZ 解析");
    for (rate, total) in [(44_100u32, 86_016usize), (48_000, 94_208)] {
        let pcm = sine_pcm(rate, total);
        // 输入自检：PCM 本身的音高（与解码段相互印证）。
        let (_, cents_in) = measure_freq_cents(&pcm, rate, 0.5, 1.5);
        assert!(
            cents_in.abs() <= 5.0,
            "{rate} Hz 输入 PCM 偏差 {cents_in:+.2} 音分"
        );
        let sample = InMemorySample {
            pcm: Arc::new(pcm),
            sample_hz: rate as f32,
            loop_points: None,
        };
        for engine_hz in [48_000u32, 44_100] {
            let mut out = vec![0.0f32; (engine_hz * 2) as usize];
            let written = sfz
                .render_note_into(&sample, 69, 100, &mut out, engine_hz as f32)
                .expect("渲染");
            let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(
                written > 0 && peak > 0.01,
                "{rate}@{engine_hz} 渲染不得静音"
            );
            let (_, cents) = measure_freq_cents(&out, engine_hz, 0.25, 1.5);
            assert!(
                cents.abs() <= 10.0,
                "{rate}@{engine_hz} 渲染偏差 {cents:+.2} 音分（门限 10；B1 宣称约 −100）"
            );
        }
    }
}
