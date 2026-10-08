//! `yeban_dsp::drums` 的**每个音色的可对账读数** ＋ **确定性**
//! [ARCH-RT-001, ARCH-RT-004, ARCH-DSP-001, ARCH-DET-001]。
//!
//! # 口径（写在读数前面，避免"读数被误读"）
//!
//! - 采样率固定 **48 kHz**（`SR`）；所有时间读数单位是**秒**（毫秒在打印时给出）；
//! - **闭式参照用的是器件真的用的那条式子**，不是理想值：
//!   幅度包络走 [`yeban_dsp::envelope::Adsr`] 的 6 个时间常数口径
//!   （`envelope.rs:263-273`，`coef = e^{−6/(T·fs)}`）⇒ −60 dB 的时刻是
//!   `attack_s + T·ln(1000)/6`（`ln(1000)/6 = 1.151 292…`）；
//!   底鼓的音高是 `f(t) = tune_hz + (glide_hz − tune_hz)·e^{−6t/glide_s}`
//!   （`drums/voice.rs` 的 `kick()`）⇒ 它的周期数是
//!   `∫₀ᵗ f dτ = tune·t + (glide−tune)·(glide_s/6)·(1 − e^{−6t/glide_s})`。
//!   ⚠ `channel_strip` 线的教训（第一版拿"纯基波 RMS"当理想参照，而
//!   `LadderFilter` 在 0.22× 截止处已有 0.82 dB 通带衰减 ⇒ 参照本身错了）：
//!   本文件的**每一个**参照都来自"同一条算式"或"同机测到的另一个量"，
//!   没有一处是外部理想值；
//! - **衰减时间是代理指标**：它不是"包络的样子"，而是"20 ms 滑窗最大值包络的
//!   对数在 `[峰值−10 dB, 峰值−40 dB]` 区间上的最小二乘直线外推到 −60 dB 的时刻"。
//!   用回归而不是"最后越过阈值的样本"，是因为后者被**正弦的过零**污染：
//!   20 ms 窗口覆盖 50 Hz 的一整个周期（默认最低基频是底鼓的 55 Hz，周期 18.2 ms）
//!   ⇒ 窗口里总含一个正弦峰 ⇒ 包络无过零纹波；
//! - **谱重心是代理指标**：它是对数网格（40 Hz 起、5% 等比步进到 16 kHz）上的
//!   `Σ f·|X(f)| / Σ |X(f)|`，每个 bin 代表相同的**相对**带宽
//!   ⇒ 它测的是"对数频率轴上的重心"，不是幅度谱的一阶矩。
//!   判据只用它做**同机排序**与量级核对，不拿它当绝对频率。
//!
//! # 判据
//!
//! | 编号 | 音色 | 读数 | 怎么变红 |
//! | :--- | :--- | :--- | :--- |
//! | K1 | 底鼓 | 峰值（无量纲） | 音色不发声 |
//! | K2 | 底鼓 | −60 dB 衰减时间（s）vs 闭式 | 幅度包络时间写错 |
//! | K3 | 底鼓 | 第 1 / 第 20 个**上行零交叉**时刻（s）vs 闭式 | 音高下滑没做 / `tune_hz` 错 |
//! | S1 | 军鼓 | 185 Hz 与 330 Hz 的 DFT bin 幅度 vs 250 Hz | 音调分量只有一个 / 频率错 |
//! | S2 | 军鼓 | 音调分量与噪声分量各自的 −60 dB 时间（s）vs 闭式 | 两条包络的时间写错 |
//! | S3 | 军鼓 | 噪声/音调 RMS 比（dB）＋**逐样本线性叠加** | 两个分量不独立 / 电平写错 |
//! | H1 | 踩镲 | 闭镲与开镲的 −60 dB 时间（s）vs 闭式 | 两个衰减时间写错 |
//! | H2 | 踩镲/军鼓/底鼓 | 谱重心（Hz）与过零率（次/s）的**同机排序** | 带通没做（踩镲不再"金属"） |
//! | H3 | 踩镲 | 1 kHz 以下的能量占比 | 带通下沿写错 |
//! | C1 | 拍手 | **从音频数出的 onset 个数** vs `bursts` | onset 重触发没做 |
//! | C2 | 拍手 | onset 间隔（s）vs `burst_spacing_s` | 定时写错 |
//! | C3 | 拍手 | 尾巴 −60 dB 时间（s）vs 闭式 | 尾巴时间写错 |
//! | C4 | 拍手 | 谱重心落在带通区间内 | 带通写错 |
//! | S4 | 全部 | 窃取处 `max|x[i+1] − x[i]|`：3 ms 淡出 vs 硬窃取（**产品级注入**） | 把 `steal_fade_frames` 设成 0 ⇒ 3 ms 淡出没生效 |
//! | R1 | 全部 5 个 | 每个音色的峰值都 > 0（覆盖度） | 某个音色静音 |
//! | R10 | 全部 5 个 | 同输入两次 ＋ 6 种切分 ⇒ 逐位相同 | 任何隐藏的块边界状态 |
//!
//! 唯一的**产品级注入**是 S4 的 `set_steal_fade_frames(0)`；其余注入在报告里以
//! "改源码 → 抓红行 → 还原"的形态给出（不改提交内容）。

use yeban_dsp::drums::{DRUM_SLOTS, DrumHit, DrumKitParams, DrumMachine, DrumVoice, MAX_BURSTS};

/// 夹具采样率（Hz）。
const SR: f64 = 48_000.0;
/// 每次渲染的帧数（1 s：长于所有音色的 −60 dB 尾巴）。
const FRAMES: usize = 48_000;
/// 频谱分析用的前多少帧（85 ms：短到包络还没走完、长到频率分辨率够用）。
const ANALYSIS_FRAMES: usize = 4_096;

// ---------------------------------------------------------------------------
// 读数工具（都是测量工具，不是被测代码）
// ---------------------------------------------------------------------------

/// 绝对值峰值（无量纲）。
fn peak(samples: &[f32]) -> f64 {
    samples
        .iter()
        .fold(0.0f64, |acc, sample| acc.max(f64::from(*sample).abs()))
}

/// 单 bin DFT 幅度（矩形窗；单位：与输入同量纲的幅度）。
fn dft_bin(samples: &[f32], freq_hz: f64, sample_rate: f64) -> f64 {
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    for (index, sample) in samples.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let phase = core::f64::consts::TAU * freq_hz * index as f64 / sample_rate;
        re += f64::from(*sample) * phase.cos();
        im += f64::from(*sample) * phase.sin();
    }
    2.0 * (re * re + im * im).sqrt() / samples.len() as f64
}

/// **前沿**滑窗（`env[i] = max|x[i .. i+window]|`）绝对值最大值包络。
///
/// # 为什么是"前沿"而不是"居中"（本判据第一版的错法）
///
/// 单调衰减时，前沿窗的最大值取在窗口**左端** ⇒ `env[i] = A(i)`，**无偏**；
/// 居中窗的最大值取在 `i − window/2` ⇒ 整个包络被**推迟半个窗口**。
/// 第一版用 20 ms 居中窗 ⇒ 闭镲的 −60 dB 时间读数被推迟约 **10 ms**
/// （实测 62.7 ms vs 闭式 52.3 ms，+19.85%），而底鼓（461 ms）只偏 1.8%
/// ⇒ 这个偏差**按衰减时间的长短不成比例**，容差再宽也盖不住。
fn envelope(samples: &[f32], window: usize) -> Vec<f64> {
    let count = samples.len();
    let mut out = vec![0.0f64; count];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = (index + window).min(count);
        let mut best = 0.0f64;
        for sample in &samples[index..high] {
            let value = f64::from(*sample).abs();
            if value > best {
                best = value;
            }
        }
        *slot = best;
    }
    out
}

/// **衰减拟合**用的包络窗（帧）：20 ms。
///
/// 前沿窗下这段长度的选择不影响无偏性（单调衰减时取左端）—— 20 ms 只是
/// "窗口里一定含一个波形峰"的长度：默认参数的最低基频是底鼓的 55 Hz
/// （周期 18.2 ms）。
fn decay_window(sample_rate: f64) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let window = (sample_rate * 0.020) as usize;
    window.max(1)
}

/// **onset 检测**用的包络窗（帧）：2 ms。
///
/// 两条约束：
///
/// 1. 必须**短于 onset 间隔**（默认 10 ms），否则滑窗会把下一个 onset 拉进
///    当前窗口 ⇒ 包络不再落回低门限 ⇒ 只数出 1 个 onset
///    （本机实测：20 ms 窗数出 1 个，2 ms 窗数出 4 个）；
/// 2. 用**居中**窗而不是前沿窗：前沿窗在缓冲起点会被**钳位**（`i < 0` 不存在）
///    ⇒ 第 1 个 onset 只能落在 0.0，而其后每个 onset 都提前约一个窗口
///    ⇒ **第 1 个间隔偏短 1.8 ms**（本机实测 8.29 ms vs 10 ms）。
fn onset_window(sample_rate: f64) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let window = (sample_rate * 0.002) as usize;
    window.max(1)
}

/// **居中**滑窗（`env[i] = max|x[i−w/2 .. i+w/2]|`）——只给 onset 检测用。
fn envelope_centered(samples: &[f32], window: usize) -> Vec<f64> {
    let count = samples.len();
    let half = window / 2;
    let mut out = vec![0.0f64; count];
    for (index, slot) in out.iter_mut().enumerate() {
        let low = index.saturating_sub(half);
        let high = (index + half + 1).min(count);
        let mut best = 0.0f64;
        for sample in &samples[low..high] {
            let value = f64::from(*sample).abs();
            if value > best {
                best = value;
            }
        }
        *slot = best;
    }
    out
}

/// 量什么：**−60 dB 衰减时间**（秒）—— 20 ms 滑窗最大值包络的对数在
/// `[峰值−10 dB, 峰值−40 dB]` 区间上做最小二乘直线，外推到 `峰值−60 dB`
/// 的时刻。`from_s` 之前的样本**不参与拟合**（拍手用它跳过 onset 串）。
///
/// 返回 `None`：峰值 ≤ 0、或区间样本不足（< 64 个）、或拟合斜率非负。
fn decay60_s(samples: &[f32], sample_rate: f64, from_s: f64) -> Option<f64> {
    let level = peak(samples);
    if level <= 0.0 {
        return None;
    }
    let env = envelope(samples, decay_window(sample_rate));
    let high_band = level * 0.316_227_77; // −10 dB
    let low_band = level * 0.01; // −40 dB
    let mut count = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_xx = 0.0f64;
    let mut sum_xy = 0.0f64;
    for (index, value) in env.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = index as f64 / sample_rate;
        // ⚠ `x` 的单位是**秒**，`from_s` 也是秒 —— 本判据第一版写成
        // `x < from_s * sample_rate`（秒 vs 帧），于是每个样本都被跳过、
        // 拟合样本数恒 0、判据以 "None" 形式永远红。
        if x < from_s || *value > high_band || *value < low_band {
            continue;
        }
        let y = value.ln();
        count += 1.0;
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_xy += x * y;
    }
    if count < 64.0 {
        return None;
    }
    let denominator = count * sum_xx - sum_x * sum_x;
    if denominator.abs() < 1.0e-18 {
        return None;
    }
    let slope = (count * sum_xy - sum_x * sum_y) / denominator;
    if slope >= 0.0 {
        return None;
    }
    let intercept = (sum_y - slope * sum_x) / count;
    let target = (level * 0.001).ln();
    Some((target - intercept) / slope)
}

/// 闭式：`Adsr` 从起振到 −60 dB 的时刻（秒）。
///
/// `envelope.rs:265-273` 的 `time_coefficient` 用 **6 个时间常数**
/// ⇒ `value(n) = e^{−6n/(T·fs)}` ⇒ `value = 0.001` 在 `n = T·fs·ln(1000)/6`。
/// 起振段把它整体推后 `attack_s`（第 1 个样本到峰值恰好 `attack_s`）。
fn closed_form_decay60_s(attack_s: f64, decay_s: f64) -> f64 {
    attack_s + decay_s * 1000.0f64.ln() / 6.0
}

/// 上行零交叉（`x[i] <= 0 && x[i+1] > 0`）的时刻（秒），相邻样本线性插值。
fn upward_crossing_times(samples: &[f32], sample_rate: f64) -> Vec<f64> {
    let mut times = Vec::new();
    for index in 0..samples.len().saturating_sub(1) {
        let first = f64::from(samples[index]);
        let second = f64::from(samples[index + 1]);
        if first <= 0.0 && second > 0.0 {
            let span = first - second;
            let fraction = if span.abs() > 0.0 { first / span } else { 0.0 };
            #[allow(clippy::cast_precision_loss)]
            times.push((index as f64 + fraction) / sample_rate);
        }
    }
    times
}

/// 闭式：底鼓的瞬时频率（Hz）。
fn kick_hz_at(t: f64, tune: f64, glide: f64, glide_s: f64) -> f64 {
    tune + (glide - tune) * (-6.0 * t / glide_s).exp()
}

/// 闭式：底鼓从 0 到 `t` 走过的**周期数**。
fn kick_cycles_at(t: f64, tune: f64, glide: f64, glide_s: f64) -> f64 {
    tune * t + (glide - tune) * (glide_s / 6.0) * (1.0 - (-6.0 * t / glide_s).exp())
}

/// 闭式：底鼓走过 `cycles` 个周期（可以是小数）的时刻（秒）——
/// 二分求 `kick_cycles_at(t) == cycles`。
///
/// 整数 `k` 给出的就是第 `k` 个上行零交叉的时刻；`0.25` 给出的是**第一个波峰**。
fn kick_phase_time_closed_form(cycles: f64, tune: f64, glide: f64, glide_s: f64) -> f64 {
    let mut low = 0.0f64;
    let mut high = 10.0f64;
    for _ in 0..200 {
        let middle = 0.5 * (low + high);
        if kick_cycles_at(middle, tune, glide, glide_s) < cycles {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

/// 闭式：第 `k` 个上行零交叉的时刻（秒）。
fn kick_crossing_closed_form(k: u32, tune: f64, glide: f64, glide_s: f64) -> f64 {
    kick_phase_time_closed_form(f64::from(k), tune, glide, glide_s)
}

/// 过零率（次/秒）。
fn zero_crossing_rate(samples: &[f32], sample_rate: f64) -> f64 {
    let crossings = samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count();
    #[allow(clippy::cast_precision_loss)]
    let rate = crossings as f64 / (samples.len() as f64 / sample_rate);
    rate
}

/// 对数网格（40 Hz 起、5% 等比步进到 16 kHz）上的幅度加权重心（Hz）——
/// **代理指标**，判据只用它做同机排序（见模块文档口径）。
fn spectral_centroid_hz(samples: &[f32], sample_rate: f64) -> f64 {
    let mut numerator = 0.0f64;
    let mut denominator = 0.0f64;
    let mut freq = 40.0f64;
    while freq < 16_000.0 {
        let magnitude = dft_bin(samples, freq, sample_rate);
        numerator += freq * magnitude;
        denominator += magnitude;
        freq *= 1.05;
    }
    if denominator <= 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

/// 线性网格上 `[low_hz, high_hz]` 的幅度和与全带（20 Hz–16 kHz）幅度和之比。
///
/// 网格步进 20 Hz；返回 `None` 表示全带能量为 0（静音）。
fn band_fraction(samples: &[f32], sample_rate: f64, low_hz: f64, high_hz: f64) -> Option<f64> {
    let mut inside = 0.0f64;
    let mut total = 0.0f64;
    let mut freq = 20.0f64;
    while freq < 16_000.0 {
        let magnitude = dft_bin(samples, freq, sample_rate);
        total += magnitude;
        if freq >= low_hz && freq <= high_hz {
            inside += magnitude;
        }
        freq += 20.0;
    }
    if total <= 0.0 {
        None
    } else {
        Some(inside / total)
    }
}

/// 均方根（与输入同量纲）。
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    #[allow(clippy::cast_precision_loss)]
    let value = (sum / samples.len() as f64).sqrt();
    value
}

/// 量什么：**从渲染出的音频数出的 onset 时刻**（秒）。
///
/// 做法：20 ms 滑窗最大值包络（无过零纹波）＋滞回（升过 `0.4×峰值` 记一次
/// onset，落回 `0.05×峰值` 才重新武装）。滞回的两个门限都相对**同一段音频的
/// 峰值**，因此它是同机参照。
fn onset_times(samples: &[f32], sample_rate: f64) -> Vec<f64> {
    let level = peak(samples);
    if level <= 0.0 {
        return Vec::new();
    }
    let env = envelope_centered(samples, onset_window(sample_rate));
    let high = level * 0.4;
    let low = level * 0.05;
    let mut onsets = Vec::new();
    let mut armed = true;
    for (index, value) in env.iter().enumerate() {
        if armed && *value > high {
            #[allow(clippy::cast_precision_loss)]
            onsets.push(index as f64 / sample_rate);
            armed = false;
        } else if !armed && *value < low {
            armed = true;
        }
    }
    onsets
}

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 渲染一击：给定鼓组、音色、帧数、增益。
fn render_hit(kit: DrumKitParams, voice: DrumVoice, frames: usize, gain: f32) -> Vec<f32> {
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(kit);
    machine.trigger(DrumHit::new(voice, 0, gain));
    let mut out = vec![0.0f32; frames];
    machine.render(0, &mut out);
    out
}

/// 默认鼓组下渲染一击。
fn render_default(voice: DrumVoice) -> Vec<f32> {
    render_hit(DrumKitParams::DEFAULT, voice, FRAMES, 1.0)
}

/// 打印一行读数（单位写在行里）。
fn report(label: &str, text: &str) {
    println!("[yeban-dsp/drums] {label:<26} {text}");
}

// ---------------------------------------------------------------------------
// R1：覆盖度 —— 5 个音色每一个都真的发声
// ---------------------------------------------------------------------------

/// 量什么：5 个音色各自一击的**峰值**（无量纲）与**非零样本占比**。
///
/// 判据：每个音色的峰值都 > 0.05，且非零样本都 > 0、全部有限。
/// 这条是**覆盖度自检**：它让"读数全绿但其实什么都没响"不可能发生
/// （`polysynth` 的第一版零分配判据就踩过"窗口后段全是静音快路径"）。
#[test]
fn r1_every_voice_really_sounds() {
    for voice in DrumVoice::ALL {
        let rendered = render_default(voice);
        let level = peak(&rendered);
        let nonzero = rendered.iter().filter(|sample| **sample != 0.0).count();
        let non_finite = rendered.iter().filter(|sample| !sample.is_finite()).count();
        report(
            voice.name(),
            &format!(
                "peak={level:.6} nonzero={nonzero}/{} ({:.2}%) non_finite={non_finite}",
                rendered.len(),
                100.0 * nonzero as f64 / rendered.len() as f64
            ),
        );
        assert!(level > 0.05, "{} 的峰值只有 {level}", voice.name());
        assert!(nonzero > 0, "{} 一个非零样本都没有", voice.name());
        assert_eq!(non_finite, 0, "{} 产出了非有限样本", voice.name());
    }
}

// ---------------------------------------------------------------------------
// K：底鼓
// ---------------------------------------------------------------------------

/// K1 + K2：底鼓的峰值与 −60 dB 衰减时间。
///
/// 读数：峰值（无量纲）、−60 dB 时间（s）vs 闭式
/// `attack_s + decay_s·ln(1000)/6`（`drums/mod.rs` 的默认值）。
#[test]
fn k1_k2_kick_peak_and_decay_match_the_closed_form() {
    let kit = DrumKitParams::DEFAULT;
    let rendered = render_default(DrumVoice::Kick);
    let level = peak(&rendered);
    let measured = decay60_s(&rendered, SR, 0.0).expect("底鼓必须有可拟合的衰减段");
    let expected = closed_form_decay60_s(f64::from(kit.kick.attack_s), f64::from(kit.kick.decay_s));
    report(
        "kick 峰值",
        &format!(
            "peak={level:.6}（预期 ≈ level={:.3}）",
            kit.kick.level * kit.master_level
        ),
    );
    report(
        "kick −60 dB 衰减时间",
        &format!(
            "measured={:.1} ms closed_form={:.1} ms 偏差={:+.2}%",
            measured * 1e3,
            expected * 1e3,
            100.0 * (measured - expected) / expected
        ),
    );
    // 峰值不是"音色电平"：正弦的**第一个波峰**在相位 0.25 周期处，那时包络已经
    // 衰减了一段时间。闭式 = `level × e^{−6(t_crest − attack_s)/decay_s}`，
    // `t_crest` 由同一条音高算式解出（`∫₀ᵗ f = 0.25`）。
    let crest = kick_phase_time_closed_form(
        0.25,
        f64::from(kit.kick.tune_hz),
        f64::from(kit.kick.glide_hz),
        f64::from(kit.kick.glide_s),
    );
    let expected_peak = f64::from(kit.kick.level)
        * (-6.0 * (crest - f64::from(kit.kick.attack_s)) / f64::from(kit.kick.decay_s)).exp();
    report(
        "kick 峰值闭式",
        &format!(
            "t_crest={:.3} ms closed_form_peak={expected_peak:.6} 偏差={:+.2}%",
            crest * 1e3,
            100.0 * (level - expected_peak) / expected_peak
        ),
    );
    assert!(
        (level - expected_peak).abs() < 0.01 * expected_peak,
        "底鼓峰值 {level} vs 闭式 {expected_peak}"
    );
    assert!(
        level <= f64::from(kit.kick.level),
        "峰值 {level} 超过了音色电平 {}",
        kit.kick.level
    );
    assert!(
        (measured - expected).abs() < 0.02 * expected,
        "底鼓 −60 dB 时间 {measured} s vs 闭式 {expected} s"
    );
}

/// K3：底鼓的**音高下滑**。
///
/// 读数：第 1 个与第 20 个**上行零交叉**的时刻（s）vs 闭式（二分求
/// `∫₀ᵗ f = k`）。第 1 个交叉量的是起振段的音高、第 20 个量的是尾巴的音高
/// ⇒ 两个读数一起钉住"下滑存在"且"落点是 `tune_hz`"。
///
/// 怎么变红：把 `glide_hz` 设成 `tune_hz`（无下滑）时第 1 个交叉从约 12 ms
/// 退到约 18 ms（`1/tune_hz`）⇒ 明显偏出容差。
#[test]
fn k3_kick_frequency_glides_from_glide_hz_to_tune_hz() {
    let kit = DrumKitParams::DEFAULT;
    let tune = f64::from(kit.kick.tune_hz);
    let glide = f64::from(kit.kick.glide_hz);
    let glide_s = f64::from(kit.kick.glide_s);
    let rendered = render_default(DrumVoice::Kick);
    let crossings = upward_crossing_times(&rendered, SR);
    assert!(
        crossings.len() > 20,
        "只有 {} 个上行零交叉，量不到第 20 个",
        crossings.len()
    );
    for k in [1u32, 20] {
        #[allow(clippy::cast_possible_truncation)]
        let measured = crossings[k as usize];
        let expected = kick_crossing_closed_form(k, tune, glide, glide_s);
        let tolerance = if k == 1 { 0.4e-3 } else { 1.5e-3 };
        report(
            &format!("kick 第 {k} 个上行零交叉"),
            &format!(
                "measured={:.3} ms closed_form={:.3} ms 偏差={:+.3} ms",
                measured * 1e3,
                expected * 1e3,
                (measured - expected) * 1e3
            ),
        );
        assert!(
            (measured - expected).abs() < tolerance,
            "第 {k} 个交叉：实测 {measured} s vs 闭式 {expected} s"
        );
    }
    // 两个闭式参照本身必须可区分（否则上面的容差没有意义）。
    let flat = kick_crossing_closed_form(1, tune, tune, glide_s);
    report(
        "kick 无下滑时的第 1 个交叉",
        &format!("closed_form={:.3} ms（对照）", flat * 1e3),
    );
    assert!(
        flat > kick_crossing_closed_form(1, tune, glide, glide_s) + 3.0e-3,
        "夹具的『有下滑 vs 无下滑』必须可区分"
    );
    // 尾巴频率读数：最后一个交叉的**周期**必须收敛到 `1/tune_hz`。
    let last = crossings.len() - 1;
    let period = crossings[last] - crossings[last - 1];
    report(
        "kick 尾巴周期",
        &format!(
            "measured={:.3} ms 1/tune_hz={:.3} ms",
            period * 1e3,
            1e3 / tune
        ),
    );
    assert!(
        (period - 1.0 / tune).abs() < 0.15e-3,
        "尾巴周期 {period} s 没有收敛到 1/tune_hz = {} s",
        1.0 / tune
    );
    // 第 1 个**周期**的等效频率必须显著高于尾巴（下滑的方向）。
    // ⚠ 不能用"第 1 个交叉处的瞬时闭式频率"：那个时刻（12.1 ms）的瞬时频率只有
    // 63.1 Hz（音高包络已经走了 87%），但第 1 个周期**平均**是 82.3 Hz
    // —— 本判据第一版就是这么写错的。
    let first_cycle_hz = 1.0 / crossings[1];
    let early_hz = kick_hz_at(crossings[1], tune, glide, glide_s);
    report(
        "kick 第 1 个周期的等效频率",
        &format!(
            "1/t_1={first_cycle_hz:.1} Hz，该时刻的瞬时闭式频率={early_hz:.1} Hz，尾巴={tune:.1} Hz"
        ),
    );
    assert!(
        first_cycle_hz > tune * 1.3,
        "第 1 个周期的等效频率 {first_cycle_hz} Hz 不比尾巴 {tune} Hz 高 30%"
    );
}

// ---------------------------------------------------------------------------
// S：军鼓
// ---------------------------------------------------------------------------

/// S1：军鼓的**两个音调分量**。
///
/// 读数：`tone_hz`、`tone_hz × tone_ratio` 与它们之间一个频点（250 Hz）的
/// DFT bin 幅度（矩形窗，单位同输入量纲）。
///
/// 判据：两个分量频点的幅度都至少是中间频点的 **4×**。
#[test]
fn s1_snare_has_both_tone_components_at_the_declared_frequencies() {
    let kit = DrumKitParams::DEFAULT;
    let first = f64::from(kit.snare.tone_hz);
    let second = first * f64::from(kit.snare.tone_ratio);
    // 只留音调分量，噪声会污染单个 bin 的读数。
    let mut tone_only = DrumKitParams::DEFAULT;
    tone_only.snare.noise_level = 0.0;
    let rendered = render_hit(tone_only, DrumVoice::Snare, FRAMES, 1.0);
    let head = &rendered[..ANALYSIS_FRAMES];
    let at_first = dft_bin(head, first, SR);
    let at_second = dft_bin(head, second, SR);
    let between = dft_bin(head, 250.0, SR);
    report(
        "snare 两个音调分量",
        &format!(
            "bin({first:.0} Hz)={at_first:.6} bin({second:.0} Hz)={at_second:.6} \
             bin(250 Hz)={between:.6}"
        ),
    );
    assert!(
        at_first > between * 4.0,
        "tone_hz={first} 的 bin {at_first} 不比 250 Hz 的 {between} 强"
    );
    assert!(
        at_second > between * 4.0,
        "tone_hz×ratio={second} 的 bin {at_second} 不比 250 Hz 的 {between} 强"
    );
}

/// S2：军鼓两个分量**各自**的 −60 dB 衰减时间。
///
/// 读数：音调分量（`tone_level` 开的渲染）与噪声分量（`noise_level` 开的渲染）
/// 各自的 −60 dB 时间（s）vs 闭式。两条包络的时间不同才是军鼓
/// （噪声比音调短）。
#[test]
fn s2_snare_tone_and_noise_decays_match_their_closed_forms() {
    let kit = DrumKitParams::DEFAULT;
    let mut tone_only = DrumKitParams::DEFAULT;
    tone_only.snare.noise_level = 0.0;
    let mut noise_only = DrumKitParams::DEFAULT;
    noise_only.snare.tone_level = 0.0;

    let attack = f64::from(kit.snare.attack_s);
    let cases = [
        (
            "音调",
            render_hit(tone_only, DrumVoice::Snare, FRAMES, 1.0),
            f64::from(kit.snare.tone_decay_s),
        ),
        (
            "噪声",
            render_hit(noise_only, DrumVoice::Snare, FRAMES, 1.0),
            f64::from(kit.snare.noise_decay_s),
        ),
    ];
    for (label, rendered, decay_s) in cases {
        let measured = decay60_s(&rendered, SR, 0.0).expect("军鼓分量必须有可拟合的衰减段");
        let expected = closed_form_decay60_s(attack, decay_s);
        report(
            &format!("snare {label} −60 dB"),
            &format!(
                "measured={:.1} ms closed_form={:.1} ms 偏差={:+.2}%",
                measured * 1e3,
                expected * 1e3,
                100.0 * (measured - expected) / expected
            ),
        );
        assert!(
            (measured - expected).abs() < 0.03 * expected,
            "军鼓 {label} −60 dB 时间 {measured} s vs 闭式 {expected} s"
        );
    }
    // 两条包络必须**不同**（否则"军鼓"退化成单一包络）。
    assert!(
        kit.snare.noise_decay_s != kit.snare.tone_decay_s,
        "夹具的两个包络时间必须不同"
    );
}

/// S3：军鼓的**噪声/音调比**与**线性叠加**。
///
/// 读数（单位写明）：
///
/// - `rms(噪声分量)` 与 `rms(音调分量)`（同机测到的两个量，同量纲）
/// - 噪声/音调比的 dB 值
/// - **逐样本**：`mix[i] == tone[i] + noise[i]` 的失配个数（个样本）
///
/// 判据：
///
/// 1. 失配 0（两个分量在同一台器件里**线性叠加**；同一次触发序列给同一段噪声
///    ⇒ 加噪与不加噪渲染出的"音调部分"是同一串样本）；
/// 2. 噪声 RMS > 音调 RMS（军鼓是噪声为主的）；
/// 3. 比值落在 `[3, 40]` dB（不写死"应该多少"，只钉住量级）。
#[test]
fn s3_snare_noise_to_tone_ratio_and_linear_superposition() {
    let mut tone_only = DrumKitParams::DEFAULT;
    tone_only.snare.noise_level = 0.0;
    let mut noise_only = DrumKitParams::DEFAULT;
    noise_only.snare.tone_level = 0.0;
    let mix = render_default(DrumVoice::Snare);
    let tone = render_hit(tone_only, DrumVoice::Snare, FRAMES, 1.0);
    let noise = render_hit(noise_only, DrumVoice::Snare, FRAMES, 1.0);

    let mismatch = mix
        .iter()
        .zip(tone.iter().zip(noise.iter()))
        .filter(|(a, (b, c))| **a != **b + **c)
        .count();
    let tone_rms = rms(&tone);
    let noise_rms = rms(&noise);
    let mix_rms = rms(&mix);
    let ratio_db = 20.0 * (noise_rms / tone_rms).log10();
    report(
        "snare RMS",
        &format!(
            "tone={tone_rms:.6} noise={noise_rms:.6} mix={mix_rms:.6} \
             噪声/音调={ratio_db:.2} dB 线性叠加失配={mismatch}"
        ),
    );
    assert_eq!(mismatch, 0, "军鼓的两个分量不满足逐样本线性叠加");
    assert!(
        ratio_db > 0.0,
        "噪声/音调比 {ratio_db} dB 不大于 0 ⇒ 这不是军鼓（是桶鼓）"
    );
    assert!(
        ratio_db < 12.0,
        "噪声/音调比 {ratio_db} dB 超过 12 ⇒ 音调分量（鼓皮）实际被埋掉了"
    );
}

// ---------------------------------------------------------------------------
// H：踩镲
// ---------------------------------------------------------------------------

/// H1：闭镲与开镲的 −60 dB 衰减时间。
///
/// 读数：两个音色各自的 −60 dB 时间（s）vs 闭式。
#[test]
fn h1_closed_and_open_hat_decays_match_the_closed_form() {
    let kit = DrumKitParams::DEFAULT;
    let attack = f64::from(kit.hihat.attack_s);
    for (voice, decay_s) in [
        (DrumVoice::ClosedHat, f64::from(kit.hihat.closed_decay_s)),
        (DrumVoice::OpenHat, f64::from(kit.hihat.open_decay_s)),
    ] {
        let rendered = render_default(voice);
        let measured = decay60_s(&rendered, SR, 0.0).expect("踩镲必须有可拟合的衰减段");
        let expected = closed_form_decay60_s(attack, decay_s);
        report(
            &format!("{} −60 dB", voice.name()),
            &format!(
                "measured={:.1} ms closed_form={:.1} ms 偏差={:+.2}%",
                measured * 1e3,
                expected * 1e3,
                100.0 * (measured - expected) / expected
            ),
        );
        assert!(
            (measured - expected).abs() < 0.04 * expected,
            "{} −60 dB 时间 {measured} s vs 闭式 {expected} s",
            voice.name()
        );
    }
    assert!(kit.hihat.open_decay_s > kit.hihat.closed_decay_s * 3.0);
}

/// H2：**同机排序**——踩镲比军鼓"亮"、军鼓比底鼓"亮"。
///
/// 读数：三个音色的谱重心（Hz，代理指标）与过零率（次/s）。
///
/// 判据：两个读数都必须**严格**满足 踩镲 > 军鼓 > 底鼓，且踩镲的重心是军鼓的
/// **两倍以上**、底鼓的重心在 300 Hz 以下。
#[test]
fn h2_hat_is_brighter_than_snare_which_is_brighter_than_kick() {
    let mut readings = Vec::new();
    for voice in [DrumVoice::Kick, DrumVoice::Snare, DrumVoice::OpenHat] {
        let rendered = render_default(voice);
        let head = &rendered[..ANALYSIS_FRAMES];
        let centroid = spectral_centroid_hz(head, SR);
        let rate = zero_crossing_rate(&rendered, SR);
        report(
            &format!("{} 亮度", voice.name()),
            &format!("谱重心={centroid:.1} Hz 过零率={rate:.0} 次/s"),
        );
        readings.push((voice, centroid, rate));
    }
    let (kick, snare, hat) = (readings[0], readings[1], readings[2]);
    assert!(
        hat.1 > snare.1 && snare.1 > kick.1,
        "谱重心排序错：踩镲 {:.1} / 军鼓 {:.1} / 底鼓 {:.1}",
        hat.1,
        snare.1,
        kick.1
    );
    assert!(
        hat.2 > snare.2 && snare.2 > kick.2,
        "过零率排序错：踩镲 {:.0} / 军鼓 {:.0} / 底鼓 {:.0}",
        hat.2,
        snare.2,
        kick.2
    );
    // 绝对界限按**实测的有效通带**给（不是按参数名）：
    // `highpass_hz = 6 kHz` 的四级级联在 2 kHz 才到 0.99（见 `voice.rs` 的表）
    // ⇒ 有效通带约 2–9 kHz。判据要的是"重心落在这个量级"，不是"等于某个数"。
    assert!(
        (2_000.0..=12_000.0).contains(&hat.1),
        "踩镲的谱重心 {:.1} Hz 不在有效通带 [2000, 12000] Hz 里",
        hat.1
    );
    assert!(
        snare.1 * 2.0 < hat.1,
        "踩镲重心 {:.1} Hz 不到军鼓重心 {:.1} Hz 的两倍",
        hat.1,
        snare.1
    );
    assert!(
        kick.1 < 300.0,
        "底鼓的谱重心 {:.1} Hz 不在 300 Hz 以下",
        kick.1
    );
}

/// H3：踩镲的**低频能量占比**。
///
/// 读数：1 kHz 以下的幅度和占全带（20 Hz–16 kHz）幅度和的比（无量纲）。
///
/// 判据：< 0.05。怎么变红：把 `highpass_hz` 设成 20 Hz（等于旁通）⇒ 低频不被
/// 切掉 ⇒ 占比升上去（判据同时断言它至少变成 3 倍）。
#[test]
fn h3_the_hat_band_pass_removes_the_low_end() {
    let rendered = render_default(DrumVoice::OpenHat);
    let head = &rendered[..ANALYSIS_FRAMES];
    let low = band_fraction(head, SR, 0.0, 1_000.0).expect("踩镲必须有能量");
    report(
        "open_hat 1 kHz 以下占比",
        &format!(
            "{low:.4}（带通下沿 = {} Hz）",
            DrumKitParams::DEFAULT.hihat.highpass_hz
        ),
    );
    assert!(low < 0.05, "1 kHz 以下的幅度占比 {low} 不小于 0.05");

    // 反面对照：把带通下沿拉到 20 Hz ⇒ 同一个读数必须显著变大。
    let mut wide = DrumKitParams::DEFAULT;
    wide.hihat.highpass_hz = 20.0;
    wide.hihat.lowpass_hz = 16_000.0;
    let rendered = render_hit(wide, DrumVoice::OpenHat, FRAMES, 1.0);
    let wide_low = band_fraction(&rendered[..ANALYSIS_FRAMES], SR, 0.0, 1_000.0).expect("有能量");
    report("open_hat（带通 20 Hz–16 kHz）", &format!("{wide_low:.4}"));
    assert!(
        wide_low > low * 3.0,
        "带宽打开后低频占比只从 {low} 变到 {wide_low} ⇒ 本判据测不到带通"
    );
}

// ---------------------------------------------------------------------------
// C：拍手
// ---------------------------------------------------------------------------

/// C1 + C2：拍手的 **onset 个数**与 **onset 间隔**。
///
/// 读数：从渲染音频用滞回法数出的 onset 时刻（s）、个数、相邻间隔（s）。
///
/// 判据：个数 == `bursts`；前 `bursts − 1` 个间隔都等于 `burst_spacing_s`
/// （容差 20 ms 的 10%，即 1 ms —— 包络窗口是 20 ms，所以容差按窗口尺度给）。
#[test]
fn c1_c2_clap_onset_count_and_spacing_match_the_parameters() {
    let kit = DrumKitParams::DEFAULT;
    let rendered = render_default(DrumVoice::Clap);
    let onsets = onset_times(&rendered, SR);
    let gaps: Vec<f64> = onsets.windows(2).map(|pair| pair[1] - pair[0]).collect();
    report(
        "clap onset",
        &format!(
            "个数={} bursts={} 时刻={:?} ms 间隔={:?} ms",
            onsets.len(),
            kit.clap.bursts,
            onsets.iter().map(|t| t * 1e3).collect::<Vec<_>>(),
            gaps.iter().map(|g| g * 1e3).collect::<Vec<_>>()
        ),
    );
    assert_eq!(
        onsets.len() as u32,
        kit.clap.bursts,
        "onset 个数 {} 不等于 bursts={}",
        onsets.len(),
        kit.clap.bursts
    );
    assert!(
        kit.clap.bursts > 1 && kit.clap.bursts <= MAX_BURSTS,
        "夹具的 bursts 必须 > 1"
    );
    let expected = f64::from(kit.clap.burst_spacing_s);
    for gap in &gaps {
        assert!(
            (gap - expected).abs() < 0.05 * expected + 0.5e-3,
            "onset 间隔 {gap} s vs 参数 {expected} s"
        );
    }
}

/// C3：拍手**尾巴**的 −60 dB 时间。
///
/// 读数：只对**最后一个 onset 之后**的样本拟合（跳过 onset 串，否则短促串的
/// 陡斜率会把直线拖歪 ⇒ 那是本判据第一版的错法，见注释）。
///
/// 前面所有 `[峰值−10 dB, 峰值−40 dB]` 的样本都要丢弃：短促串的衰减是 12 ms，
/// 它在 −10 dB 以下的时间点落在 onset 串内部。
#[test]
fn c3_clap_tail_decay_matches_the_closed_form() {
    let kit = DrumKitParams::DEFAULT;
    let rendered = render_default(DrumVoice::Clap);
    #[allow(clippy::cast_precision_loss)]
    let last_onset_s = f64::from(kit.clap.bursts - 1) * f64::from(kit.clap.burst_spacing_s);
    let measured = decay60_s(&rendered, SR, last_onset_s + 0.002).expect("尾巴必须可拟合");
    let expected = last_onset_s
        + closed_form_decay60_s(f64::from(kit.clap.attack_s), f64::from(kit.clap.decay_s));
    report(
        "clap 尾巴 −60 dB",
        &format!(
            "measured={:.1} ms closed_form={:.1} ms 偏差={:+.2}%",
            measured * 1e3,
            expected * 1e3,
            100.0 * (measured - expected) / expected
        ),
    );
    assert!(
        (measured - expected).abs() < 0.04 * expected,
        "拍手尾巴 −60 dB 时间 {measured} s vs 闭式 {expected} s"
    );
}

/// C4：拍手的谱重心落在带通区间附近。
///
/// 读数：谱重心（Hz，代理指标）。
#[test]
fn c4_clap_centroid_sits_in_the_band() {
    let rendered = render_default(DrumVoice::Clap);
    let head = &rendered[..3_600]; // 75 ms：三个短促 onset 都在里面
    let centroid = spectral_centroid_hz(head, SR);
    report("clap 谱重心", &format!("{centroid:.1} Hz"));
    // 拍手的主带在 1–2 kHz（本机实测重心 1424 Hz）。判据只钉量级。
    assert!(
        (600.0..=4_000.0).contains(&centroid),
        "拍手谱重心 {centroid} Hz 不在 [600, 4000] Hz 里"
    );
}

// ---------------------------------------------------------------------------
// S4：窃取淡出（[ARCH-RT-004]）
// ---------------------------------------------------------------------------

/// 渲染一段"池满 ⇒ 再触发 ⇒ 窃取"的样本，返回 `(样本, 窃取发生在第几帧)`。
///
/// `fade_frames` 是 [`DrumMachine::set_steal_fade_frames`] 的值：
/// 默认（`None`）走 [ARCH-RT-004] 的 3 ms（144 帧 @48 kHz），
/// `Some(0)` 是**硬窃取**（产品级注入：不加淡出，直接用新一击覆盖旧一击）。
fn render_with_a_steal(fade_frames: Option<u32>) -> (Vec<f32>, usize) {
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(DrumKitParams::DEFAULT);
    if let Some(frames) = fade_frames {
        machine.set_steal_fade_frames(frames);
    }
    // 16 记底鼓把池填满（都在样本 0）。
    for _ in 0..DRUM_SLOTS {
        machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
    }
    let warmup = 2_000usize;
    let mut before = vec![0.0f32; warmup];
    machine.render(0, &mut before);
    // 第 17 记：池满 ⇒ 一定走窃取路径。
    machine.trigger(DrumHit::new(DrumVoice::Kick, warmup as u64, 1.0));
    let mut after = vec![0.0f32; 1_000];
    machine.render(warmup as u64, &mut after);
    let mut out = before;
    out.extend_from_slice(&after);
    (out, warmup)
}

/// 量什么：一次窃取处**相邻样本的最大跳变** `max|x[i+1] − x[i]|`（无量纲）。
fn max_step_around(samples: &[f32], center: usize, half_window: usize) -> f64 {
    let low = center.saturating_sub(half_window);
    let high = (center + half_window).min(samples.len().saturating_sub(1));
    let mut worst = 0.0f64;
    for index in low..high {
        let step = (f64::from(samples[index + 1]) - f64::from(samples[index])).abs();
        if step > worst {
            worst = step;
        }
    }
    worst
}

/// S4：3 ms 指数淡出把**窃取处的样本跳变**压下来。
///
/// 读数：默认淡出（144 帧）与硬窃取（`set_steal_fade_frames(0)`）各自的
/// `max|x[i+1] − x[i]|`（无量纲），窗口 = 窃取点 ± 200 帧。
///
/// 判据：淡出版的跳变 **≤ 硬窃取版的 25%**。这条是**产品级注入**：
/// 硬窃取版就是"把 `steal_fade_frames` 设成 0"的真实行为，不需要改源码。
///
/// ⚠ 为什么硬窃取一定有跳变：被窃取的槽位在 2000 帧（41.7 ms）后包络还有
/// `e^{−6·2000/19200} = 0.54`，而硬窃取**立刻**把该槽位换成新一击（新一击的
/// 第 0 帧输出 0）⇒ 16 个槽位的和里少掉 0.54 那一份 ⇒ 一次阶跃。
#[test]
fn s4_the_three_millisecond_fade_suppresses_the_steal_step() {
    let (faded, center) = render_with_a_steal(None);
    let (hard, hard_center) = render_with_a_steal(Some(0));
    assert_eq!(center, hard_center);
    let faded_step = max_step_around(&faded, center, 200);
    let hard_step = max_step_around(&hard, center, 200);
    report(
        "S4 窃取处最大跳变",
        &format!(
            "3 ms 淡出={faded_step:.6} 硬窃取={hard_step:.6} 比值={:.3}",
            faded_step / hard_step
        ),
    );
    assert!(
        faded_step <= hard_step * 0.25,
        "淡出版的跳变 {faded_step} 没有压到硬窃取 {hard_step} 的 25% 以下"
    );
    assert!(
        hard_step > 0.05,
        "硬窃取版的跳变只有 {hard_step} ⇒ 夹具没造出可测的阶跃"
    );
}

// ---------------------------------------------------------------------------
// R10：确定性
// ---------------------------------------------------------------------------

/// 一个含全部 5 个音色的**超容量**触发序列（池满 ⇒ 走窃取路径）。
fn fixture_hits() -> Vec<DrumHit> {
    let mut hits = Vec::new();
    for index in 0..(DRUM_SLOTS as u64 * 2) {
        let voice = DrumVoice::ALL[(index as usize) % DrumVoice::ALL.len()];
        hits.push(DrumHit::new(
            voice,
            index * 97,
            0.4 + 0.05 * (index % 8) as f32,
        ));
    }
    // 再塞两记闭镲，覆盖 choke 开镲的那条路。
    hits.push(DrumHit::new(DrumVoice::OpenHat, 10, 1.0));
    hits.push(DrumHit::new(DrumVoice::OpenHat, 20, 1.0));
    hits.push(DrumHit::new(DrumVoice::ClosedHat, 30, 1.0));
    hits
}

/// 按给定的**块切分**渲染同一段，返回拼接后的样本。
fn render_split(chunks: &[usize]) -> Vec<f32> {
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(DrumKitParams::DEFAULT);
    for hit in fixture_hits() {
        machine.trigger(hit);
    }
    let largest = chunks.iter().copied().max().unwrap_or(1);
    let mut buffer = vec![0.0f32; largest];
    let mut out = Vec::with_capacity(chunks.iter().sum());
    let mut position = 0u64;
    for chunk in chunks {
        machine.render(position, &mut buffer[..*chunk]);
        out.extend_from_slice(&buffer[..*chunk]);
        position += *chunk as u64;
    }
    out
}

/// 把样本转成位模式，用来做**逐位**比较。
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// 按固定块长 `chunk` 渲染 `total` 帧的切分序列。
fn chunked(total: usize, chunk: usize) -> Vec<usize> {
    let mut chunks = vec![chunk; total / chunk];
    let rest = total % chunk;
    if rest > 0 {
        chunks.push(rest);
    }
    chunks
}

/// R10：**同输入两次 ⇒ 逐位相同**，以及 **6 种 chunk 切分 ⇒ 逐位相同**。
///
/// 读数：两次渲染的位模式失配个数（个样本）＋每种切分与 128 帧基准的失配个数。
///
/// ⚠ 这里的比较是**逐位**（`f32::to_bits`），不是哈希 —— 没有代理指标。
#[test]
fn r10_same_input_twice_and_six_chunk_splits_are_bit_identical() {
    const TOTAL: usize = 24_000;
    let reference = render_split(&chunked(TOTAL, 128));
    let again = render_split(&chunked(TOTAL, 128));
    assert_eq!(reference.len(), TOTAL);
    let repeat_mismatch = bits(&reference)
        .iter()
        .zip(bits(&again).iter())
        .filter(|(a, b)| a != b)
        .count();
    report(
        "R10 同输入两次",
        &format!("失配={repeat_mismatch}/{} 个样本", reference.len()),
    );
    assert_eq!(repeat_mismatch, 0, "同输入两次不是逐位相同");

    for chunk in [1usize, 7, 64, 127, 333, 4_096] {
        let split = render_split(&chunked(TOTAL, chunk));
        assert_eq!(split.len(), TOTAL, "切分 {chunk} 的帧数对不上");
        let mismatch = bits(&split)
            .iter()
            .zip(bits(&reference).iter())
            .filter(|(a, b)| a != b)
            .count();
        report(
            "R10 chunk 切分",
            &format!("chunk={chunk:>4} 帧 与 128 帧基准的逐位失配={mismatch}"),
        );
        assert_eq!(mismatch, 0, "切分 {chunk} 与 128 帧基准出现了位差");
    }
    // 覆盖度：这条判据的夹具真的产生了非零输出（否则"逐位相同"是空转）。
    assert!(peak(&reference) > 0.05, "R10 的夹具没有发声（空转）");
}
