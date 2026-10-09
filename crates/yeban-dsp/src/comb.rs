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
//!
//! ## 非有限输入样本（本轮补齐）
//!
//! 本滤波器是**递归**的：`buf[i] = x + damped(buf) · feedback`，`feedback ≤ 0.96`。
//! 一个 `NaN`／`±∞` 样本会被原样留在环里，此后读出的 `delayed` 永远非有限，
//! 且隔直状态 `dc` 与阻尼状态也被污染，**输入恢复干净也回不来** ——
//! [`CombFilter::reset`] 是唯一出路。实时路径上无法报错，只能在入口回落。
//!
//! ⇒ [`CombFilter::process`] 在更新 `dc` 与写环之前过 `math::finite_or_zero`：
//! `NaN` 与 `±∞` 归 `0.0`，**有限样本逐位不变**（既有读数一个比特都不改）。
//! 未准备时的直通**不**经过守卫（没有样本进入递归）。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入（`3e38 · 0.96` 仍可能溢出成 `∞`）；
//! 那条归调用方的电平口径管（同 `math::finite_or_zero` 的文档）。
//!
//! ## 延迟长度变更（本轮补齐）
//!
//! [`CombFilter::tune`] 会把长度 `len` 改成"采样率 ÷ 音高"。**改前的写法**把写头
//! 按 `len` 回绕、并在同一个槽位读/写，于是环只用到 `buf[0..len)`：长度**变短**时
//! `buf[len..]` 那一整段就不再被触碰，里面留着上一次（更长）延迟写进去、此后从未被
//! 读出的旧音频；长度再**变长**时那些槽位重新进入环，那份旧音频就被当成延迟输出
//! 重放出来（一段"幽灵回声"）。它**取决于上一次的处理历史** ⇒ 同一段输入、同一组
//! 参数，因为中间调过一次音高而得到不同的输出，不是 [ARCH-DET-001] 允许的可复现输出。
//!
//! ⇒ 本版把写头改成按**缓冲容量**回绕（`buf` 的长度，[`CombFilter::prepare`] 里
//! 一次分配、此后只增长不缩小），读抽头固定在写头**之前 `len` 帧**处。抽头读到的
//! 因此永远是"`len` 帧之前写进去的那个样本"，与上一次用的是什么长度无关：长度怎么改
//! 都**不需要**清线，也不会重放线外的东西，而线里已有的尾巴照旧保留（换长度不擦掉
//! 谐振器的状态）。**定长运行下两种写法的输出逐位相同** —— 判据
//! `comb::tests::a_fixed_length_matches_a_textbook_ring_buffer_bit_for_bit` 用一个
//! "写头按 `len` 回绕"的教科书环当参照实现逐位对账，因此既有音色与既有读数一个比特
//! 都不改；变的只是"换长度那一刻之后"的输出。
//!
//! ⚠ 代价（诚实边界）：本器件**不**对长度变更做交叉淡化，也**不做**分数延迟插值 ——
//! 抽头是**整帧**跳过去的，换长度那一下本来就是一次跳变（改前也一样，只是改前跳到的
//! 位置可能是线外的旧音频）。要平滑扫频的调用方应在**器件之外**做参数平滑，或把长度
//! 量化到可接受的步长。这条取舍与 [`crate::reverb`]／[`crate::convolution_reverb`] 的
//! 预延迟**不同**：那两条线是纯延迟（尾巴在梳状组里，清线碰不到尾巴），而本器件的线
//! **就是**谐振器本体，所以这里选"保留历史、换抽头距离"而不是"清线"。

use crate::math::finite_or_zero;

/// 允许的最低梳状频率，它决定缓冲区大小：96 kHz 下 30 Hz 的梳需要 3200 个样本。
pub const MIN_FREQ_HZ: f32 = 30.0;

/// 缓冲区按这个采样率上限预留。
pub const MAX_SR: f32 = 96_000.0;

/// 缓冲区样本数上限。
pub const MAX_LEN: usize = (MAX_SR / MIN_FREQ_HZ) as usize + 4;

/// 反馈梳状滤波器。
pub struct CombFilter {
    /// 延迟线，长度 = 缓冲区**容量**（[`CombFilter::prepare`] 按采样率定，此后只增长）。
    buf: Vec<f32>,
    /// 当前延迟长度（帧），由 [`CombFilter::tune`] 设定；读抽头就在写头**之前**这么多帧。
    len: usize,
    /// 写头在 `buf` 里的位置。它按**缓冲容量**回绕（不是按 `len`）—— 见模块文档
    /// 「延迟长度变更」：容量固定的环在换长度时既不必清线，也不会重放线外的旧音频。
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
    /// 必须在音频回调之外调用。分配出来的长度是**容量**：写头按它回绕，读抽头则在
    /// 写头之前 `len` 帧处（见模块文档「延迟长度变更」）。容量只增长、不缩小。
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
    ///
    /// **长度可以随时改**：读抽头与写头之间的距离就是 `len`，而写头按缓冲**容量**
    /// 回绕 ⇒ 抽头读到的永远是"`len` 帧之前写进去的那个样本"，与上一次用的是哪个长度
    /// 无关。因此这里**不需要**清线，也**不清**（线里已有的尾巴是谐振器的状态，换长度
    /// 不擦掉它）。见模块文档「延迟长度变更」。
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

    /// 缓冲**容量**（样本）：写头按它回绕，读抽头距离 `len` 不得超过它。
    #[must_use]
    pub fn buffer_len(&self) -> usize {
        self.buf.len()
    }

    /// 原地处理一个块：延迟输出写入 `out`。
    ///
    /// 长度取 `input`/`out` 的较短者；未准备时直通（输出保持原样）。
    ///
    /// 读抽头在写头**之前** `len` 帧处，写头按缓冲**容量**回绕 ⇒ 换长度既不清线，
    /// 也不重放线外的旧音频（见模块文档「延迟长度变更」）。
    #[inline]
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        if !self.is_prepared() {
            return;
        }
        let len = self.len.max(2);
        let capacity = self.buf.len();
        debug_assert!(len <= capacity, "读抽头距离不得超过缓冲容量");
        for (sample, slot) in input.iter().zip(out.iter_mut()) {
            // 入口守卫：非有限样本一旦写进环就被 `feedback ≤ 0.96` 永久留下，
            // 隔直与阻尼状态也被污染 ⇒ 就地回落成 `0.0`。有限样本逐位不变。
            let sample = finite_or_zero(*sample);
            // 抽头 = 写头往前走 `len` 帧的位置（往回退 `len`，越过零点就绕到容量尾部）。
            let read = if self.index >= len {
                self.index - len
            } else {
                self.index + capacity - len
            };
            let delayed = self.buf[read];
            // 带阻尼的反馈：环内的一极点低通。
            self.damp_state = delayed * (1.0 - self.damp) + self.damp_state * self.damp;
            // 输入端轻微隔直：梳状在直流处是单位增益，没有这一步的话，
            // 带直流偏移的音色会把环路越推越高。
            self.dc += (sample - self.dc) * 0.0005;
            let x = sample - self.dc;
            self.buf[self.index] = x + self.damp_state * self.feedback;
            self.index += 1;
            if self.index >= capacity {
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

    /// **判据（新写，可红）**：定长运行下，本器件必须与"写头按 `len` 回绕"的
    /// 教科书环形缓冲**逐位相同**。
    ///
    /// 量什么：同一段确定性输入下，本器件与参照实现的逐帧输出**位模式**。单位是
    /// **帧**（有多少帧的位模式不同）。
    ///
    /// 本版把写头改成按**缓冲容量**回绕、读抽头固定在写头之前 `len` 帧处（为了让换
    /// 长度不再重放线外的旧音频，见模块文档「延迟长度变更」）。长度**不变**时两种写法
    /// 描述的是同一个递推 —— 抽头永远读到"`len` 帧之前写进去的那个样本" —— 因此输出
    /// 必须逐位相同。这条就是"既有音色一个比特都不改"的机械证据。
    ///
    /// 把抽头距离写成 `len + 1`，或把写头改回按 `len` 回绕，本测试立即变红。
    #[test]
    fn a_fixed_length_matches_a_textbook_ring_buffer_bit_for_bit() {
        /// 参照实现：**改造前**的写法逐字保留（写头按 `len` 回绕、同槽位读/写）。
        fn reference(len: usize, input: &[f32], damp: f32, feedback: f32) -> Vec<f32> {
            let mut buf = vec![0.0f32; len];
            let mut index = 0usize;
            let mut damp_state = 0.0f32;
            let mut dc = 0.0f32;
            let mut out = vec![0.0f32; input.len()];
            for (sample, slot) in input.iter().zip(out.iter_mut()) {
                let delayed = buf[index];
                damp_state = delayed * (1.0 - damp) + damp_state * damp;
                dc += (sample - dc) * 0.0005;
                let x = sample - dc;
                buf[index] = x + damp_state * feedback;
                index += 1;
                if index >= len {
                    index = 0;
                }
                *slot = delayed;
            }
            out
        }

        let mut rng = Rng::new(0x0BAD_F00D);
        let input: Vec<f32> = (0..9_000).map(|_| rng.next_bipolar()).collect();
        for freq in [MIN_FREQ_HZ, 60.0, 200.0, 1_000.0, 4_000.0, 20_000.0] {
            for resonance in [0.0f32, 0.5, 0.95] {
                let mut device = comb(SR, freq, resonance);
                let mut got = vec![0.0f32; input.len()];
                device.process(&input, &mut got);
                let want = reference(
                    device.delay_samples(),
                    &input,
                    device.damp(),
                    device.feedback(),
                );
                let differing = got
                    .iter()
                    .zip(want.iter())
                    .filter(|(a, b)| a.to_bits() != b.to_bits())
                    .count();
                assert_eq!(
                    differing, 0,
                    "freq {freq} / resonance {resonance}: 与教科书环有 {differing} 帧不同"
                );
            }
        }
        // 覆盖度自检：对账真的取到了多个长度（否则可能全钉在一个长度上）。
        let mut lengths: Vec<usize> = [MIN_FREQ_HZ, 60.0f32, 200.0, 1_000.0, 4_000.0, 20_000.0]
            .iter()
            .map(|freq| comb(SR, *freq, 0.5).delay_samples())
            .collect();
        lengths.sort_unstable();
        lengths.dedup();
        assert!(
            lengths.len() >= 4,
            "夹具只覆盖了 {} 个长度 ⇒ 对账强度不足",
            lengths.len()
        );
        eprintln!("[yeban-dsp/comb] 定长对账: 覆盖长度 {lengths:?} 帧，逐位差异 0 帧");
    }

    /// **判据（新写，可红）**：改变延迟长度不得让**线外**的旧音频重新可闻。
    ///
    /// 夹具（[`SR`] = `48 kHz`，容量 `1604` 帧，干路无关，混响量为 `0` ⇒ 纯延迟）：
    ///
    /// ① 长长度 `480` 帧（`100 Hz`）下把幅度 `1.0` 的标记样本写在线内第 `400` 个槽位
    ///    —— 这一段的输出**峰值必须恰好 `0.0`**：`process` 是读在写之前，而这 `401`
    ///    帧里抽头指到的槽位都还没被写过。这是夹具的前置条件（标记确实进了线，
    ///    且还没出来）；
    /// ② 把长度改成 `48` 帧（`1000 Hz`）再喂 `4800` 帧静音：标记现在落在抽头距离
    ///    之内，必须在**换长度之后第 `48` 帧**以 `0.9995` 左右的幅度出来（峰值
    ///    `> 0.9`）。这条是**反向对照**：它证明换长度**没有**把线擦掉 —— 谐振器的
    ///    状态照旧保留（"清线"式修法会让这条变红）；
    /// ③ 再把长度改回 `480` 帧、喂 `1440` 帧静音：输出峰值必须 `> 0.9` 的读数
    ///    **消失**（`< 1e-3`，剩下的只是隔直状态的衰减尾巴）。`400` 号槽位在 `48`
    ///    帧的长度下不可达，那份旧音频不属于"最近历史"，不得被当成延迟输出重放。
    ///
    /// **本机实测（aarch64，本票；三个读数都是线性幅度）**：改后 ① `0e0`、
    /// ② `9.995e-1`、③ `5.760263e-5`、正对照 `9.995e-1`。把
    /// [`CombFilter::process`] 的读抽头改回"写头按 `len` 回绕、同槽位读/写"（即改造
    /// 前的写法）⇒ ② 变成 `4.9950014e-4`（旧写法下 `400` 号槽位在 `48` 帧的环外，
    /// 标记出不来）、③ 变成 `9.995e-1`（幽灵回声）⇒ 两条断言都变红。
    #[test]
    fn retuning_never_replays_audio_from_outside_the_line() {
        /// 长长度（Hz）：`48000 / 100 = 480` 帧。
        const LONG_HZ: f32 = 100.0;
        /// 长长度的帧数（与 [`LONG_HZ`] 一致，由参考长度公式算出）。
        const LONG_FRAMES: usize = 480;
        /// 短长度（Hz）：`48000 / 1000 = 48` 帧（刻意与 `480` 不成整数倍）。
        const SHORT_HZ: f32 = 1_000.0;
        /// 标记样本在线里的槽位：`< 480` 且 `>= 48`。
        const MARKER_AT: usize = 400;
        /// 标记幅度（线性）。
        const MARKER: f32 = 1.0;
        /// 短长度下先跑掉的帧数：`100 × 48`，让短环跑满 `100` 圈。
        const SHORT_RUN: usize = 4_800;
        /// 观察"回到长长度之后"的帧数：`3 × 480`。
        const BACK_RUN: usize = 3 * LONG_FRAMES;

        /// 联合峰值：`abs()` 把 `-0.0` 映成 `+0.0`，于是"是不是恰好 `0.0`"不受零的符号影响。
        fn peak(values: &[f32]) -> f32 {
            values
                .iter()
                .fold(0.0f32, |m, v| if v.abs() > m { v.abs() } else { m })
        }

        // ---- ① 写标记：这一段必须恰好全 0（抽头还没指到写过的槽位） ----
        let mut device = comb(SR, LONG_HZ, 0.0);
        let mut carrier = vec![0.0f32; MARKER_AT + 1];
        carrier[MARKER_AT] = MARKER;
        let mut device_warmup = vec![0.0f32; carrier.len()];
        device.process(&carrier, &mut device_warmup);
        let device_warmup_peak = peak(&device_warmup);
        assert_eq!(
            device_warmup_peak, 0.0,
            "前置条件失败：标记还没进线就已经出来了 ⇒ 夹具不成立"
        );

        // ---- ② 换成长长度的 1/10：标记必须在新的抽头距离上原样出来 ----
        device.tune(SR, SHORT_HZ, 0.0);
        let silence = vec![0.0f32; SHORT_RUN];
        let mut short_run = vec![0.0f32; SHORT_RUN];
        device.process(&silence, &mut short_run);
        let short_peak = peak(&short_run);
        assert!(
            short_peak > 0.9 * MARKER,
            "换长度把线擦掉了（或抽头距离算错）：短长度段峰值 {short_peak:e}"
        );

        // ---- ③ 换回长长度：线外的那份旧音频不得被重放 ----
        device.tune(SR, LONG_HZ, 0.0);
        let silence = vec![0.0f32; BACK_RUN];
        let mut back_run = vec![0.0f32; BACK_RUN];
        device.process(&silence, &mut back_run);
        let back_peak = peak(&back_run);
        assert!(
            back_peak < 1e-3,
            "换回长长度后线外的旧音频被重放：峰值 {back_peak:e}（应 < 1e-3）"
        );

        // ---- 正对照：长度**不变**时，同一个标记必须原样从线里出来 ----
        let mut control = comb(SR, LONG_HZ, 0.0);
        let mut carrier = vec![0.0f32; MARKER_AT + 1];
        carrier[MARKER_AT] = MARKER;
        let mut control_warmup = vec![0.0f32; carrier.len()];
        control.process(&carrier, &mut control_warmup);
        let silence = vec![0.0f32; LONG_FRAMES];
        let mut control_tail = vec![0.0f32; LONG_FRAMES];
        control.process(&silence, &mut control_tail);
        let control_peak = peak(&control_tail);
        assert!(
            control_peak > 0.9 * MARKER,
            "正对照失败：长度不变时标记都没出来（{control_peak:e}）⇒ ②③ 是空断言"
        );
        // 标记落点就是"写进去之后第 `LONG_FRAMES` 帧"（`401 + 479 = 880`）。
        assert!(
            control_tail[LONG_FRAMES - 1].abs() > 0.9 * MARKER,
            "标记落点不在第 {} 帧：{}",
            LONG_FRAMES - 1,
            control_tail[LONG_FRAMES - 1]
        );

        eprintln!(
            "[yeban-dsp/comb] 换长度: 前置峰值 {device_warmup_peak:e} | 短长度段峰值 \
             {short_peak:e} | 回到长长度后峰值 {back_peak:e} | 正对照峰值 {control_peak:e}"
        );
    }
}
