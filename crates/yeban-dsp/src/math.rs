//! 基础数学原语。[ARCH-RT-001] [ARCH-DET-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/util.rs`（全量移植）+ `src/dsp/fmath.rs`（仅取 Padé `tanh` 思路，
//! 其余按 provenance §4 明确不移植）。
//!
//! 本模块只放**无状态**的纯函数：音高换算、软限幅、线性插值、共享 FFT。
//! 有状态的东西（RNG、噪声颜色、平滑器）在各自的模块里。
//!
//! ## 与来源的差异
//!
//! - 来源 `dsp/util.rs` 把 RNG 也放在这里；夜半把 [`crate::noise::Rng`] 移到
//!   `noise` 模块，避免"数学库"和"噪声源"互相拖带。
//! - 来源的 `exp2` 只是 `x.exp2()` 的别名（为 wasm 无 libm 构建服务）。
//!   夜半不是 `no_std`，因此保留它作为**语义命名**（音高换算全部以 2 的幂表达），
//!   但让它做参数钳制，避免 `NaN` 静默传播。
//! - 新增 [`db_to_gain`] / [`gain_to_db`]：来源把 dB 换算散落在 `fx_shaping.rs`
//!   与 `engine` 里，夜半把它收敛成一处，并钉死"0 dB = 1.0、−∞ dB = 0.0"。

use crate::MIN_SAMPLE_RATE;

/// 音高换算用的 `2^x`。
///
/// `NaN` 输入返回 0，避免一次坏参数把整条信号链污染成 `NaN`
/// （实时路径上无法"报错", 只能把故障限制在局部）。
#[inline]
#[must_use]
pub fn exp2(x: f32) -> f32 {
    let value = x.exp2();
    if value.is_finite() { value } else { 0.0 }
}

/// 十二平均律 MIDI 音高（允许小数）转 Hz，A4 = 440 Hz。
#[inline]
#[must_use]
pub fn note_to_hz(note: f32) -> f32 {
    440.0 * exp2((note - 69.0) / 12.0)
}

/// 半音偏移对应的频率倍率。
#[inline]
#[must_use]
pub fn semitone_ratio(semitones: f32) -> f32 {
    exp2(semitones / 12.0)
}

/// dB 转线性增益（0 dB = 1.0）。
#[inline]
#[must_use]
pub fn db_to_gain(db: f32) -> f32 {
    /// `20 * log10(2)`，即一个八度/一倍增益对应的 dB 数。
    const DB_PER_OCTAVE: f32 = 6.020_6;
    if db <= -120.0 {
        0.0
    } else {
        exp2(db / DB_PER_OCTAVE)
    }
}

/// 线性增益转 dB；0 返回 [`f32::NEG_INFINITY`]。
#[inline]
#[must_use]
pub fn gain_to_db(gain: f32) -> f32 {
    /// `20 * log10(2)`，与 [`db_to_gain`] 共用同一常数。
    const DB_PER_OCTAVE: f32 = 6.020_6;
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        DB_PER_OCTAVE * gain.log2()
    }
}

/// 线性插值：`t = 0` 取 `a`，`t = 1` 取 `b`。
#[inline]
#[must_use]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 带线性区的软限幅：`|x| <= KNEE` 时**逐位透明**，之上平滑压缩，绝不越过 ±1。
///
/// `soft_clip`（膝点 ~0.3）在响的复音总线上是可听的染色；这个版本只在最后几 dB
/// 弯曲，因此常规演奏保持干净，而瞬态仍被接住且不产生爆音 [ARCH-DSP-001]。
#[inline]
#[must_use]
pub fn soft_limit(x: f32) -> f32 {
    /// 线性区上界。低于它**不做任何**算术改写（`assert_eq!` 级别的透明）。
    const KNEE: f32 = 0.82;
    if !x.is_finite() {
        // ±inf 落在硬天花板上；NaN 归零而不是继续传播。
        return if x.is_nan() { 0.0 } else { x.signum() };
    }
    let magnitude = x.abs();
    if magnitude <= KNEE {
        return x;
    }
    let over = (magnitude - KNEE) / (1.0 - KNEE);
    let shaped = KNEE + (1.0 - KNEE) * (1.0 - (-over).exp());
    if shaped >= 1.0 {
        x.signum()
    } else {
        shaped.copysign(x)
    }
}

/// Padé 逼近的 `tanh`：廉价、平滑、在有效区间内单调。
///
/// 来源：`synth-core/src/dsp/ladder.rs` 的 `fast_tanh`。用在饱和级里，
/// 精度要求由"听感"和"有界"决定，而不是由数值库决定。
#[inline]
#[must_use]
pub fn pade_tanh(x: f32) -> f32 {
    let x2 = x * x;
    (x * (27.0 + x2)) / (27.0 + 9.0 * x2)
}

/// 把采样率钳制到 [`MIN_SAMPLE_RATE`] 之上。
///
/// 每个需要采样率的入口都先过这里：`0`/负数/`NaN` 会让系数变成 `inf` 或 `NaN`。
#[inline]
#[must_use]
pub(crate) fn sanitise_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() {
        sample_rate.max(MIN_SAMPLE_RATE)
    } else {
        MIN_SAMPLE_RATE
    }
}

/// 把**非有限**样本归零；**有限样本逐位不变**（含 `±0.0` 与次正规数）。
///
/// 这是喂给**递归状态**（反馈环、延迟线、单极点/双二阶状态、包络跟随器）的输入
/// 样本的入口守卫。本 crate 已有三处同类守卫（`compressor` 与 `channel_strip` 各自的
/// `finite_or_zero`、`limiter` 的 `nan_to_zero`），三者口径**互有差别**（见下）；本函数
/// 补的是**递归**这一条：有限状态里一旦写进 `NaN`／`±∞`，`state = a·state + b·x` 的
/// 每一次迭代都把它原样留下（`NaN · 0 = NaN`、`∞ · 0.9 = ∞`），因此 `reset()` 之外
/// **没有任何出路**，湿路从此永久坏掉。实时路径上无法报错，只能在入口就地回落。
///
/// 与相邻口径的差别（**不是**同义词，别混用）：
///
/// - [`crate::meter::sanitize_sample`]（`channel_strip` 的入口用它）把幅度也钳到
///   `±16`，那是**电平**口径；
/// - `limiter` 的 `nan_to_zero` 只归零 `NaN`、**保留** `±∞`（它的窗口峰值证明需要
///   无穷大可见）；
/// - 本函数归零 `NaN` 与 `±∞`，且**不**钳制有限幅度 ⇒ 对任何有限输入逐位恒等，
///   既有输出一个比特都不改。
///
/// 它**不**承诺"有限但极大的输入不会溢出"：`3e38 · 8` 仍是 `∞`。那条由调用方的
/// 电平口径负责（同 [`crate::meter::sanitize_sample`]）。
#[inline]
#[must_use]
pub(crate) fn finite_or_zero(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

/// 原地迭代基 2 复数变换（正向或逆向）。
///
/// 由两处非音频速率的消费者共用：波表导入分析（[`crate::oscillator`]）。
/// `f64` 与直白实现在这里是合适的——它不在渲染循环里——且 `n` 必须是 2 的幂。
/// 长度不足 2 或长度不匹配时直接返回（不 panic、不分配）。
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two(), "FFT 长度必须是 2 的幂");
    if im.len() != n || n < 2 {
        return;
    }

    // 位反转置换。
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // 蝶形：每级跨度翻倍。
    let mut half = 1usize;
    while half < n {
        let span = half * 2;
        let angle = core::f64::consts::TAU / span as f64 * if inverse { 1.0 } else { -1.0 };
        let (twiddle_re, twiddle_im) = (angle.cos(), angle.sin());
        let mut base = 0usize;
        while base < n {
            let (mut wr, mut wi) = (1.0f64, 0.0f64);
            for k in 0..half {
                let (ar, ai) = (re[base + k], im[base + k]);
                let (br, bi) = (re[base + k + half], im[base + k + half]);
                let (vr, vi) = (br * wr - bi * wi, br * wi + bi * wr);
                re[base + k] = ar + vr;
                im[base + k] = ai + vi;
                re[base + k + half] = ar - vr;
                im[base + k + half] = ai - vi;
                let next_wr = wr * twiddle_re - wi * twiddle_im;
                wi = wr * twiddle_im + wi * twiddle_re;
                wr = next_wr;
            }
            base += span;
        }
        half = span;
    }

    if inverse {
        let scale = 1.0 / n as f64;
        for value in re.iter_mut() {
            *value *= scale;
        }
        for value in im.iter_mut() {
            *value *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据**：`finite_or_zero` 对**任何有限**输入逐位恒等（含 `±0.0`、次正规数
    /// 与 `f32::MAX`），对 `NaN` 与 `±∞` 恰好返回 `+0.0`。
    ///
    /// 量什么：返回值的 `to_bits()` 与期望位型的比较（单位：`f32` 位型）。
    /// 为什么用位型而不是 `==`：`0.0 == -0.0` 为真，而本 crate 的混响文档
    /// 有一条 `-0.0` 的实测例外，因此符号位必须可判。
    #[test]
    fn finite_or_zero_is_the_bitwise_identity_on_finite_values() {
        // 正对照：表里必须真的含非有限值，否则"归零"这一半是空断言。
        let table = [
            (0.0f32, false),
            (-0.0f32, false),
            (1.0, false),
            (-1.0, false),
            (f32::MIN_POSITIVE, false),
            (-f32::MIN_POSITIVE, false),
            (f32::MIN_POSITIVE / 2.0, false), // 次正规数
            (f32::MAX, false),
            (f32::MIN, false),
            (f32::NAN, true),
            (f32::INFINITY, true),
            (f32::NEG_INFINITY, true),
        ];
        assert!(
            table.iter().any(|(v, hostile)| *hostile && !v.is_finite()),
            "夹具必须包含非有限值"
        );
        for (value, hostile) in table {
            let got = finite_or_zero(value);
            if hostile {
                assert_eq!(
                    got.to_bits(),
                    0.0f32.to_bits(),
                    "非有限输入 {:?} 必须归 +0.0，实得位型 {:#010x}",
                    value,
                    got.to_bits()
                );
            } else {
                assert_eq!(
                    got.to_bits(),
                    value.to_bits(),
                    "有限输入 {:?} 必须逐位不变，实得 {:?}",
                    value,
                    got
                );
            }
        }
    }

    #[test]
    fn note_to_hz_matches_concert_pitch() {
        assert!((note_to_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((note_to_hz(60.0) - 261.6256).abs() < 1e-2);
        assert!((note_to_hz(81.0) - 880.0).abs() < 1e-2);
        // 八度必须精确是 2 倍（平均律的定义）。
        assert!((note_to_hz(72.0) / note_to_hz(60.0) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn semitone_ratio_is_an_octave_at_twelve_semitones() {
        assert!((semitone_ratio(0.0) - 1.0).abs() < 1e-6);
        assert!((semitone_ratio(12.0) - 2.0).abs() < 1e-5);
        assert!((semitone_ratio(-12.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn db_conversion_round_trips() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(-6.020_6) - 0.5).abs() < 1e-5);
        assert_eq!(db_to_gain(-200.0), 0.0);
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
        for db in [-60.0f32, -12.0, -3.0, 3.0, 12.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-3, "db {db}");
        }
    }

    #[test]
    fn soft_limit_is_transparent_then_bounded() {
        // 线性区内逐位透明…
        for x in [0.0f32, 0.1, 0.5, 0.82, -0.7] {
            assert_eq!(soft_limit(x), x);
        }
        // …之上平滑、单调压缩，且永不越过 ±1。
        assert!(soft_limit(0.9) > 0.82 && soft_limit(0.9) < 0.9);
        assert!(soft_limit(0.9) < soft_limit(0.95));
        assert!(soft_limit(0.95) < soft_limit(1.0));
        assert!(soft_limit(1.0) < soft_limit(2.0));
        assert!(soft_limit(10.0) <= 1.0 && soft_limit(-10.0) >= -1.0);
        assert!(soft_limit(f32::INFINITY) <= 1.0);
        for x in [0.9f32, 1.2, 3.0, 1e6] {
            assert!((soft_limit(x) + soft_limit(-x)).abs() < 1e-6);
        }
    }

    #[test]
    fn soft_limit_never_produces_nan() {
        assert_eq!(soft_limit(f32::NAN), 0.0);
        assert_eq!(soft_limit(f32::INFINITY), 1.0);
        assert_eq!(soft_limit(f32::NEG_INFINITY), -1.0);
        assert_eq!(exp2(f32::NAN), 0.0);
    }

    #[test]
    fn pade_tanh_is_monotonic_and_accurate_in_range() {
        // 非递减（在 |x| ≈ 3 处导数为 0，因此不能断言严格递增）且处处有限。
        let mut previous = f32::NEG_INFINITY;
        for step in -200..=200 {
            let x = step as f32 * 0.05;
            let y = pade_tanh(x);
            assert!(y.is_finite());
            assert!(y >= previous, "not monotonic at {x}");
            previous = y;
        }
        // 逼近真 tanh，且在小信号处几乎线性。
        assert!(pade_tanh(0.0).abs() < 1e-9);
        assert!((pade_tanh(0.1) - 0.1f32.tanh()).abs() < 1e-3);
        assert!((pade_tanh(0.5) - 0.5f32.tanh()).abs() < 5e-3);
        assert!((pade_tanh(1.0) - 1.0f32.tanh()).abs() < 0.02);
        assert!(pade_tanh(1.0) > pade_tanh(0.5));
    }

    #[test]
    fn the_shared_fft_round_trips() {
        let mut re: Vec<f64> = (0..256).map(|i| ((i * 37) % 11) as f64 - 5.0).collect();
        let mut im = vec![0.0f64; 256];
        let original = re.clone();
        fft(&mut re, &mut im, false);
        // 重 DC 的实信号必须落在 bin 0 与其镜像。
        assert!(re[0].abs() > 1.0);
        fft(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(original.iter()) {
            assert!((a - b).abs() < 1e-9, "round trip changed the signal");
        }
    }

    #[test]
    fn the_shared_fft_puts_a_sine_in_one_bin() {
        // 256 点里 32 个周期：能量只应在 bin 32 与 224。
        let mut re: Vec<f64> = (0..256)
            .map(|i| (core::f64::consts::TAU * 32.0 * i as f64 / 256.0).sin())
            .collect();
        let mut im = vec![0.0f64; 256];
        fft(&mut re, &mut im, false);
        for bin in 1..128 {
            let power = re[bin].hypot(im[bin]);
            if bin == 32 {
                assert!(power > 100.0, "the sine's own bin is empty");
            } else {
                assert!(power < 1e-6, "energy leaked into bin {bin}");
            }
        }
    }

    #[test]
    fn fft_tolerates_mismatched_and_degenerate_lengths() {
        // 不 panic、不越界：长度不匹配与长度 < 2 都直接返回。
        let mut re = [1.0f64; 4];
        let mut im = [0.0f64; 3];
        fft(&mut re, &mut im, false);
        assert_eq!(re, [1.0; 4]);
        let mut one_re = [2.0f64];
        let mut one_im = [0.0f64];
        fft(&mut one_re, &mut one_im, false);
        assert_eq!(one_re, [2.0]);
    }

    #[test]
    fn sanitise_sample_rate_clamps_degenerate_inputs() {
        assert_eq!(sanitise_sample_rate(0.0), MIN_SAMPLE_RATE);
        assert_eq!(sanitise_sample_rate(-5.0), MIN_SAMPLE_RATE);
        assert_eq!(sanitise_sample_rate(f32::NAN), MIN_SAMPLE_RATE);
        assert_eq!(sanitise_sample_rate(48_000.0), 48_000.0);
    }
}
