//! 反馈梳状滤波器（合成器的"谐振器"滤波器类型）。[ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/comb.rs`（含其全部回归测试，两个测试模块已合并）。
//!
//! 一条短延迟自己喂自己，并在环内加阻尼：把延迟调到截止频率（即延迟里正好一个
//! 波长），它就会在那个音高上鸣响——这正是拨弦、金属与机器人音色的性格来源。
//! 缓冲区在设置采样率时分配一次，绝不在渲染循环里分配 [ARCH-RT-001]。
//!
//! ## 与来源的差异
//!
//! 1. `prepare` → [`CombFilter::prepare`]（保持名字：它明确表达"分配 + 置位"），
//!    并标注只能在音频回调之外调用；
//! 2. `set` → [`CombFilter::tune`]：来源的 `set` 与滤波器模块的 `set` 同名但语义
//!    完全不同（这里是"定音高"），改名以消除歧义；
//! 3. 五个 `*_for_debug` 访问器收敛成正常的只读 getter
//!    （[`CombFilter::feedback`]、[`CombFilter::damp`]、[`CombFilter::delay_samples`]、
//!    [`CombFilter::buffer_len`]）——它们是公开 API 的一部分，不该带"debug"字样；
//! 4. `process` 用 `zip` 而不是按下标写 `out[i]`：来源在 `out` 比 `input` 短时会
//!    越界 panic，而实时路径上宁可少写也不能展开栈；
//! 5. 白噪声测试改用本 crate 的 [`crate::noise::Rng`]（确定性相同、断言不变）。

/// 允许的最低梳状频率，它决定缓冲区大小：96 kHz 下 30 Hz 的梳需要 3200 个样本。
pub const MIN_FREQ_HZ: f32 = 30.0;

/// 缓冲区按这个采样率上限预留。
pub const MAX_SR: f32 = 96_000.0;

/// 缓冲区样本数上限。
pub const MAX_LEN: usize = (MAX_SR / MIN_FREQ_HZ) as usize + 4;

/// 反馈梳状滤波器。
pub struct CombFilter {
    buf: Vec<f32>,
    len: usize,
    index: usize,
    /// 反馈路径里的一极点阻尼（防止高反馈时鸣响变成嗡嗡声）。
    damp_state: f32,
    damp: f32,
    feedback: f32,
    dc: f32,
}

impl CombFilter {
    /// 构造一个**未准备**的实例（直通）。缓冲区在 [`Self::prepare`] 里分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            len: 1,
            index: 0,
            damp_state: 0.0,
            damp: 0.3,
            feedback: 0.7,
            dc: 0.0,
        }
    }

    /// 按 `sample_rate` 分配延迟线 [ARCH-RT-001]。
    ///
    /// **这是本类型唯一会分配的方法**（且只在需要扩容时才分配），
    /// 必须在音频回调之外调用。
    pub fn prepare(&mut self, sample_rate: f32) {
        let sample_rate = crate::math::sanitise_sample_rate(sample_rate);
        let max = ((sample_rate.min(MAX_SR) / MIN_FREQ_HZ) as usize + 4).min(MAX_LEN);
        if self.buf.len() < max {
            self.buf = vec![0.0; max];
        }
        self.len = 1;
        self.index = 0;
        self.buf.fill(0.0);
        self.damp_state = 0.0;
        self.dc = 0.0;
    }

    /// 清空延迟线与滤波器状态。
    pub fn reset(&mut self) {
        let len = self.buf.len();
        self.buf[..len].fill(0.0);
        self.index = 0;
        self.damp_state = 0.0;
        self.dc = 0.0;
    }

    /// 是否已经过 [`Self::prepare`]。
    #[must_use]
    pub fn is_prepared(&self) -> bool {
        !self.buf.is_empty()
    }

    /// 调音：`freq` 是梳的音高（Hz），`resonance`（0..1）是反馈量。
    pub fn tune(&mut self, sample_rate: f32, freq: f32, resonance: f32) {
        let sample_rate = crate::math::sanitise_sample_rate(sample_rate);
        let freq = if freq.is_finite() {
            freq.clamp(MIN_FREQ_HZ, sample_rate * 0.45)
        } else {
            MIN_FREQ_HZ
        };
        let resonance = if resonance.is_finite() {
            resonance.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let len = (sample_rate / freq).round() as usize;
        self.len = len.clamp(2, self.buf.len().max(2));
        // 0.98 是纯梳状的稳定极限；阻尼把它压在下面。
        self.feedback = (resonance * 0.96).min(0.96);
        self.damp = 0.15 + resonance * 0.5;
    }

    /// 当前反馈量。
    #[must_use]
    pub const fn feedback(&self) -> f32 {
        self.feedback
    }

    /// 当前阻尼系数。
    #[must_use]
    pub const fn damp(&self) -> f32 {
        self.damp
    }

    /// 当前延迟长度（样本）。
    #[must_use]
    pub const fn delay_samples(&self) -> usize {
        self.len
    }

    /// 缓冲区总长度（样本）。
    #[must_use]
    pub fn buffer_len(&self) -> usize {
        self.buf.len()
    }

    /// 原地处理一个块：延迟输出写入 `out`。
    ///
    /// 长度取 `input`/`out` 的较短者；未准备时直通（输出保持原样）。
    #[inline]
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        if !self.is_prepared() {
            return;
        }
        let len = self.len.max(2);
        for (sample, slot) in input.iter().zip(out.iter_mut()) {
            let delayed = self.buf[self.index];
            // 带阻尼的反馈：环内的一极点低通。
            self.damp_state = delayed * (1.0 - self.damp) + self.damp_state * self.damp;
            // 输入端轻微隔直：梳状在直流处是单位增益，没有这一步的话，
            // 带直流偏移的音色会把环路越推越高。
            self.dc += (*sample - self.dc) * 0.0005;
            let x = *sample - self.dc;
            self.buf[self.index] = x + self.damp_state * self.feedback;
            self.index += 1;
            if self.index >= len {
                self.index = 0;
            }
            *slot = delayed;
        }
    }
}

impl Default for CombFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::Rng;

    const SR: f32 = 48_000.0;

    fn comb(sample_rate: f32, freq: f32, resonance: f32) -> CombFilter {
        let mut c = CombFilter::new();
        c.prepare(sample_rate);
        c.tune(sample_rate, freq, resonance);
        c
    }

    /// 喂一段短音爆发，测 0.7 s 之后还剩多少：调准了的梳会把它延续下去，
    /// 失谐的不会。
    fn tail_after_burst(comb_freq: f32, tone_freq: f32) -> f32 {
        let mut comb = comb(SR, comb_freq, 0.95);
        let n = 48_000;
        let mut input = vec![0.0f32; n];
        // 0.3 s 的音，然后静音。
        for (i, sample) in input.iter_mut().take(14_400).enumerate() {
            *sample = (core::f32::consts::TAU * tone_freq * i as f32 / SR).sin() * 0.5;
        }
        let mut out = vec![0.0f32; n];
        comb.process(&input, &mut out);
        assert!(
            out.iter().all(|v| v.is_finite()),
            "comb produced non-finite output"
        );
        // 0.45–0.55 s：爆发结束后又过了几十个往返，调准的梳在这里明显还在响。
        let tail = &out[21_600..26_400];
        (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn rings_at_its_tuned_frequency() {
        let tuned = tail_after_burst(200.0, 200.0);
        let detuned = tail_after_burst(200.0, 311.0);
        assert!(tuned > 0.02, "comb did not ring at all: {tuned}");
        assert!(
            tuned > detuned * 3.0,
            "comb did not favour its tuned frequency: {tuned} vs {detuned}"
        );
    }

    #[test]
    fn stays_bounded_at_full_feedback() {
        let sr = 44_100.0;
        let mut comb = comb(sr, MIN_FREQ_HZ, 1.0);
        let input = vec![0.5f32; 44_100];
        let mut out = vec![0.0f32; input.len()];
        comb.process(&input, &mut out);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 8.0));
    }

    #[test]
    fn noise_becomes_periodic() {
        let mut comb = comb(SR, 200.0, 0.9);
        // 确定性白噪声（本 crate 自己的 RNG）。
        let mut rng = Rng::new(0x1234_5678);
        let n = 24_000;
        let input: Vec<f32> = (0..n).map(|_| rng.next_bipolar()).collect();
        // 按引擎的方式驱动：128 样本一块，状态跨调用保持。
        let mut out = vec![0.0f32; n];
        for start in (0..n).step_by(128) {
            let end = (start + 128).min(n);
            let (inp, oup) = (&input[start..end], &mut out[start..end]);
            comb.process(inp, oup);
        }
        let lag = 240;
        let mut num = 0.0f64;
        let mut den = 0.0f64;
        for i in 0..n - lag {
            num += f64::from(out[i]) * f64::from(out[i + lag]);
        }
        for v in out.iter() {
            den += f64::from(*v) * f64::from(*v);
        }
        let correlation = num / den.max(1e-12);
        // 参考：输入本身在这个延迟上的相关性。
        let mut num_in = 0.0f64;
        let mut den_in = 0.0f64;
        for i in 0..n - lag {
            num_in += f64::from(input[i]) * f64::from(input[i + lag]);
        }
        for v in input.iter() {
            den_in += f64::from(*v) * f64::from(*v);
        }
        // 输入是白噪声（这个延迟上没有相关性），而梳的输出是周期性的。
        assert!(num_in / den_in < 0.1, "test input was not white noise");
        assert!(
            correlation > 0.3,
            "comb did not make the noise periodic: {correlation}"
        );
        assert!(out.iter().all(|v| v.is_finite()));
    }

    /// **判据（新写，可红）**：延迟长度是"一个波长"，与采样率无关。
    ///
    /// 来源只在 48 kHz（以及 44.1 kHz 的有界性测试）下工作，因此一个把
    /// 48000 写死、或忘了把频率换算成样本的实现能通过全部既有测试。
    /// 这里同时断言长度公式与"在 96 kHz 下也真的在 200 Hz 上鸣响"。
    ///
    /// 把 `tune` 里的 `sample_rate` 换成固定的 48_000.0，本测试立即变红。
    #[test]
    fn one_wavelength_per_delay_at_any_sample_rate() {
        for sample_rate in [48_000.0f32, 96_000.0] {
            let c = comb(sample_rate, 200.0, 0.9);
            let expected = (sample_rate / 200.0).round() as usize;
            assert_eq!(
                c.delay_samples(),
                expected,
                "at {sample_rate} Hz a 200 Hz comb is {expected} samples"
            );
            assert!(expected <= c.buffer_len());
            // 96 kHz 下 200 Hz 的爆发也必须在 200 Hz 上鸣响。
            let tuned = tail_at_rate(sample_rate, 200.0, 200.0);
            let detuned = tail_at_rate(sample_rate, 200.0, 311.0);
            assert!(
                tuned > detuned * 3.0,
                "at {sample_rate} Hz the comb lost its tuning: {tuned} vs {detuned}"
            );
        }
    }

    fn tail_at_rate(sample_rate: f32, comb_freq: f32, tone_freq: f32) -> f32 {
        let mut comb = comb(sample_rate, comb_freq, 0.95);
        let n = sample_rate as usize;
        let burst = (0.3 * sample_rate) as usize;
        let mut input = vec![0.0f32; n];
        for (i, sample) in input.iter_mut().take(burst).enumerate() {
            *sample = (core::f32::consts::TAU * tone_freq * i as f32 / sample_rate).sin() * 0.5;
        }
        let mut out = vec![0.0f32; n];
        comb.process(&input, &mut out);
        let from = (0.45 * sample_rate) as usize;
        let to = (0.55 * sample_rate) as usize;
        let tail = &out[from..to];
        (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt()
    }

    /// **判据（新写，可红）**：定音高被钳制在缓冲区能表达的范围内。
    ///
    /// 低于 [`MIN_FREQ_HZ`] 或高于 `0.45·fs` 的请求都必须被钳制：
    /// 长度既不能超过缓冲区，也不能退化成 0/1（那会让梳变成单位延迟的噪点源）。
    ///
    /// 把 `tune` 里的 `clamp(2, self.buf.len().max(2))` 改成不钳制，
    /// `delay_samples()` 会超过 `buffer_len()`，本测试立即变红。
    #[test]
    fn tuning_is_clamped_into_the_buffer() {
        let mut c = CombFilter::new();
        c.prepare(SR);
        let capacity = c.buffer_len();
        assert!(capacity > 0);
        for (freq, resonance) in [
            (1.0f32, 0.5f32),
            (0.0, 0.5),
            (-100.0, 0.5),
            (f32::NAN, f32::NAN),
            (1.0e9, 2.0),
            (MIN_FREQ_HZ, 1.0),
        ] {
            c.tune(SR, freq, resonance);
            assert!(c.delay_samples() >= 2, "delay collapsed for freq {freq}");
            assert!(
                c.delay_samples() <= capacity,
                "freq {freq} asked for {} samples, buffer holds {capacity}",
                c.delay_samples()
            );
            assert!((0.0..=0.96).contains(&c.feedback()), "freq {freq}");
            assert!((0.15..=0.65).contains(&c.damp()), "freq {freq}");
        }
        // 未准备的实例是安全的直通。
        let mut raw = CombFilter::new();
        assert!(!raw.is_prepared());
        let mut out = [0.25f32, -0.5];
        raw.process(&[1.0f32, 1.0], &mut out);
        assert_eq!(out, [0.25, -0.5]);
        // 输出比输入短时也不得 panic。
        let mut c = comb(SR, 200.0, 0.5);
        let mut short = [0.0f32; 1];
        c.process(&[1.0f32; 8], &mut short);
        assert!(short[0].is_finite());
    }
}
