//! 立体声延迟：ping-pong 交叉馈送 + 反馈路径里的高频阻尼。[ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/delay.rs`（含其全部回归测试）。
//!
//! 它取代了 vendored Soundpipe 的延迟：那份是两条各自独立的单声道线加一个焊死的
//! dry+wet 求和，因此无法交叉馈送，回声也永远不会变暗——而这两点正是"拍击回声"
//! 与"能坐进混音里的延迟"之间的差别。
//!
//! 延迟时间的变化是**滑行**而不是跳变：移动的读取抽头会咔哒，而能读小数位置正是
//! 让抽头动起来的前提。
//!
//! ## 与来源的差异
//!
//! 1. `setup(sample_rate)` 改名 [`Delay::configure`]：它做的是分配与初始化，
//!    而不是"设置"，并明确标注**只能在音频回调之外调用** [ARCH-RT-001]；
//! 2. `process` 去掉冗余的 `frames` 参数，改用 `left`/`right` 的较短长度——
//!    来源的 `frames` 允许与切片长度不一致，那会在实时路径上越界 panic；
//! 3. 新增 [`Delay::is_configured`] 与"未配置即直通"的行为约束；
//! 4. 新增判据 `the_delay_time_is_seconds_not_samples`：来源的所有测试都钉在
//!    48 kHz，一个把 48 kHz 写死的实现能全过；这条按 96 kHz 断言回声落点。
//!
//! ## 非有限输入样本（本轮补齐）
//!
//! 延迟线是**递归**的：`line[i] = input + damped(line) · feedback`，`feedback ≤ 0.95`。
//! 一个 `NaN`／`±∞` 样本会被反馈量原样留在环里（`NaN · 0.95 = NaN`、
//! `∞ · 0.95 = ∞`），此后每一条输出都是非有限值，**输入恢复干净也回不来** ——
//! [`Delay::reset`] 是唯一出路。实时路径上无法报错，只能在入口回落。
//!
//! ⇒ [`Delay::process`] 在写进延迟线之前过 `math::finite_or_zero`：`NaN` 与 `±∞`
//! 归 `0.0`，**有限样本逐位不变**（既有音色与既有读数一个比特都不改）。
//! 未配置时的直通**不**经过这个守卫（没有样本进入递归），与 [`Delay::reset`] 的
//! "清历史、不动配置"同一条纪律。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入：`3e38 · 0.95` 仍可能溢出成 `∞`。那条归
//! 调用方的电平口径管（同 `math::finite_or_zero` 的文档）。
//!
//! ⚠ 与 [`crate::convolution`] 的"逐样本净化是调用方的职责"**不矛盾**：卷积核没有
//! 递归状态，`NaN` 写进它的频域延迟线后会在 `partitions` 个块之内被冲掉；
//! 本模块的环是永久的。

use crate::math::finite_or_zero;

/// 延迟线能装下的最长时间（秒）。20 BPM 的四分音符要 3 s，
/// 这条线到此为止，更长的时间会被钳制。
pub const MAX_DELAY_SECONDS: f32 = 2.0;

/// 延迟线长度的硬上限，免得 192 kHz 宿主每声道占 1.5 MB。
/// 在合成器实际运行的采样率下不会触及。
const MAX_DELAY_SAMPLES: usize = 192_000;

/// 延迟时间变化的一极点系数（≈20 ms @48 kHz）。
const TIME_SLEW: f32 = 0.0008;

/// 反馈低通的范围下限：1.0 = 全开，[`DAMP_MIN`] = 重阻尼。
const DAMP_MIN: f32 = 0.05;

/// 延迟参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelayParams {
    /// 延迟时间（秒）。
    pub time_s: f32,
    /// 回声重复次数对应的反馈量，0..0.95；再高这条线就不衰减了。
    pub feedback: f32,
    /// 叠加到干信号上的湿电平（与被替换掉的延迟同一约定，既有音色因此保持平衡）。
    pub mix: f32,
    /// 0 = 明亮的重复，1 = 每一次重复都失去高频。
    pub damp: f32,
    /// 交叉馈送两个声道，让回声左右交替。
    pub ping_pong: bool,
}

impl DelayParams {
    /// 默认：300 ms、反馈 0.3、湿 0.2、无阻尼、不交叉。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            time_s: 0.3,
            feedback: 0.3,
            mix: 0.2,
            damp: 0.0,
            ping_pong: false,
        }
    }
}

impl Default for DelayParams {
    fn default() -> Self {
        Self::new()
    }
}

/// 立体声延迟线（两条线各自独立，状态互不共享）。
pub struct Delay {
    lines: [Vec<f32>; 2],
    /// 阻尼滤波器的一极点状态，每声道一个。
    damp_state: [f32; 2],
    index: usize,
    /// 当前（已滑行的）与目标的延迟量，单位样本。
    samples: f32,
    target: f32,
    sample_rate: f32,
    ready: bool,
}

impl Delay {
    /// 构造一个**未配置**的实例（直通）。缓冲区在 [`Self::configure`] 里分配。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lines: [Vec::new(), Vec::new()],
            damp_state: [0.0; 2],
            index: 0,
            samples: 0.0,
            target: 0.0,
            sample_rate: 48_000.0,
            ready: false,
        }
    }

    /// 按 `sample_rate` 分配两条延迟线 [ARCH-RT-001]。
    ///
    /// **这是本类型唯一会分配的方法**，因此必须在音频回调之外调用一次
    /// （引擎初始化时）。它同时清零时间抽头与阻尼状态。
    pub fn configure(&mut self, sample_rate: f32) {
        let sample_rate = crate::math::sanitise_sample_rate(sample_rate);
        let max = ((MAX_DELAY_SECONDS * sample_rate) as usize).clamp(64, MAX_DELAY_SAMPLES);
        for line in self.lines.iter_mut() {
            line.clear();
            line.resize(max + 2, 0.0);
        }
        self.index = 0;
        self.samples = 0.0;
        self.target = 0.0;
        self.sample_rate = sample_rate;
        self.damp_state = [0.0; 2];
        self.ready = true;
    }

    /// 是否已经过 [`Self::configure`]。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.ready
    }

    /// 清空延迟线与阻尼状态（不重新分配）。
    pub fn reset(&mut self) {
        for line in self.lines.iter_mut() {
            line.fill(0.0);
        }
        self.damp_state = [0.0; 2];
        self.index = 0;
    }

    /// 本实例能产生的最长延迟（秒）。
    #[must_use]
    pub fn max_seconds(&self) -> f32 {
        self.lines[0].len().saturating_sub(2) as f32 / self.sample_rate
    }

    /// 线性插值读取延迟线（越界用回绕处理，绝不 panic）。
    #[inline]
    fn read(line: &[f32], index: usize, delay: f32) -> f32 {
        let len = line.len();
        let mut position = index as f32 - delay;
        while position < 0.0 {
            position += len as f32;
        }
        let first = position as usize % len;
        let second = (first + 1) % len;
        let fraction = position - position.floor();
        line[first] * (1.0 - fraction) + line[second] * fraction
    }

    /// 原地处理一个块：湿信号以 `mix` **叠加**到干信号上。
    ///
    /// 长度取 `left`/`right` 的较短者；未配置时直通。全程零分配。
    pub fn process(&mut self, params: DelayParams, left: &mut [f32], right: &mut [f32]) {
        if !self.ready || self.lines[0].is_empty() {
            return;
        }
        let len = self.lines[0].len();
        let max = (len - 2) as f32;
        let time = if params.time_s.is_finite() {
            params.time_s.clamp(0.001, self.max_seconds())
        } else {
            0.001
        };
        let target = (time * self.sample_rate).min(max);
        if self.samples <= 0.0 {
            // 首次使用或刚复位：直接从目标时间开始，而不是从零滑上去。
            self.samples = target;
        }
        self.target = target;
        let feedback = if params.feedback.is_finite() {
            params.feedback.clamp(0.0, 0.95)
        } else {
            0.0
        };
        let mix = if params.mix.is_finite() {
            params.mix.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let damp = if params.damp.is_finite() {
            params.damp.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let damp_coeff = 1.0 - damp * (1.0 - DAMP_MIN);

        for (dry_l, dry_r) in left.iter_mut().zip(right.iter_mut()) {
            // 入口守卫：一个非有限样本写进延迟线就会被反馈量原样留在环里
            // （`NaN · feedback = NaN`、`∞ · 0.95 = ∞`），输入恢复干净也回不来，
            // `reset()` 之外没有出路 ⇒ 就地回落成 `0.0`。有限样本逐位不变。
            let input_l = finite_or_zero(*dry_l);
            let input_r = finite_or_zero(*dry_r);
            let delayed_l = Self::read(&self.lines[0], self.index, self.samples);
            let delayed_r = Self::read(&self.lines[1], self.index, self.samples);

            // 阻尼在环内，因此每一次重复都比喂给它的那一次更暗，
            // 而不是整条尾巴只被滤一次。
            self.damp_state[0] += (delayed_l - self.damp_state[0]) * damp_coeff;
            self.damp_state[1] += (delayed_r - self.damp_state[1]) * damp_coeff;

            if params.ping_pong {
                // 输入只进左线，两条线互相喂，因此第一条回声从左声道出来，
                // 之后左右交替。先求和成单声道才能让立体声输入也这样工作：
                // 否则硬右声道的信号永远到不了左线。
                let mono = (input_l + input_r) * 0.5;
                self.lines[0][self.index] = mono + self.damp_state[1] * feedback;
                self.lines[1][self.index] = self.damp_state[0] * feedback;
            } else {
                self.lines[0][self.index] = input_l + self.damp_state[0] * feedback;
                self.lines[1][self.index] = input_r + self.damp_state[1] * feedback;
            }

            *dry_l = input_l + delayed_l * mix;
            *dry_r = input_r + delayed_r * mix;

            self.index = (self.index + 1) % len;
            self.samples += (self.target - self.samples) * TIME_SLEW;
        }
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn delay(time_s: f32, feedback: f32, mix: f32, damp: f32, ping_pong: bool) -> Delay {
        let mut d = Delay::new();
        d.configure(SR);
        let params = DelayParams {
            time_s,
            feedback,
            mix,
            damp,
            ping_pong,
        };
        // 先把滑行时间预热到目标，第一条回声才会落在它被要求的位置。
        let mut l = [0.0f32; 1];
        let mut r = [0.0f32; 1];
        d.process(params, &mut l, &mut r);
        debug_assert_eq!(l[0], 0.0);
        d
    }

    /// 单声道的冲激响应：回声落在哪里、有多响。
    fn impulse_response(mut d: Delay, params: DelayParams, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        left[0] = 1.0;
        d.process(params, &mut left, &mut right);
        (left, right)
    }

    #[test]
    fn ping_pong_alternates_the_channels() {
        let time = 0.1;
        let params = DelayParams {
            time_s: time,
            feedback: 0.7,
            mix: 0.8,
            damp: 0.0,
            ping_pong: true,
        };
        let (left, right) = impulse_response(delay(time, 0.7, 0.8, 0.0, true), params, 48_000);
        let period = (time * SR) as usize;
        // 输入被求和成单声道，因此左声道里 1.0 的冲激以 0.5 进线，
        // 加上湿混合后以 0.4 出来。第一条回声在左，第二条在右，第三条又回左，
        // 每一条都比上一条小一个反馈系数。
        assert!(
            left[period].abs() > 0.35,
            "first echo should be on the left: {}",
            left[period]
        );
        assert!(
            right[period].abs() < 0.05,
            "the right channel must wait its turn"
        );
        assert!(
            right[period * 2].abs() > 0.2,
            "second echo should be on the right"
        );
        assert!(left[period * 2].abs() < 0.05);
        assert!(
            left[period * 3].abs() > 0.1,
            "third echo should be back on the left"
        );
        // 每条回声都比它前面两条小一个往返的反馈量，因此幅度按**两次**抽头
        // 下降一个 feedback。
        let ratio = left[period * 3].abs() / left[period].abs();
        assert!((ratio - 0.49).abs() < 0.05, "ping-pong decay {ratio}");
    }

    #[test]
    fn a_plain_delay_keeps_the_channels_apart() {
        let params = DelayParams {
            time_s: 0.1,
            feedback: 0.6,
            mix: 0.8,
            damp: 0.0,
            ping_pong: false,
        };
        let (left, right) = impulse_response(delay(0.1, 0.6, 0.8, 0.0, false), params, 48_000);
        let period = 4800;
        assert!(left[period].abs() > 0.5);
        assert!(
            right.iter().all(|v| v.abs() < 1e-6),
            "nothing should cross over"
        );
    }

    #[test]
    fn echoes_decay_at_the_feedback_rate() {
        let params = DelayParams {
            time_s: 0.05,
            feedback: 0.5,
            mix: 1.0,
            damp: 0.0,
            ping_pong: false,
        };
        let (left, _) = impulse_response(delay(0.05, 0.5, 1.0, 0.0, false), params, 24_000);
        let period = 2400;
        let first = left[period].abs();
        let second = left[period * 2].abs();
        let third = left[period * 3].abs();
        assert!(
            (second / first - 0.5).abs() < 0.02,
            "second echo {}",
            second / first
        );
        assert!(
            (third / second - 0.5).abs() < 0.02,
            "third echo {}",
            third / second
        );
    }

    /// 阻尼是亮度控制而不是音量控制：明亮的爆发必须一次次失去高频，
    /// 而低频几乎不变。
    #[test]
    fn damping_darkens_later_repeats() {
        let period = 2400usize; // 50 ms
        let measure = |freq: f32, damp: f32| {
            let params = DelayParams {
                time_s: 0.05,
                feedback: 0.6,
                mix: 1.0,
                damp,
                ping_pong: false,
            };
            let mut d = delay(0.05, 0.6, 1.0, damp, false);
            let frames = period * 4;
            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            // 一个延迟周期的稳定音，然后静音：此后的重复就是该音的干净副本。
            for (i, sample) in left.iter_mut().take(period).enumerate() {
                *sample = (core::f32::consts::TAU * freq * i as f32 / SR).sin();
            }
            d.process(params, &mut left, &mut right);
            // 第一次与第三次重复的幅度，用 DFT 测量。
            let magnitude = |from: usize| {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for i in 0..period {
                    let phase = core::f64::consts::TAU * f64::from(freq) * i as f64 / f64::from(SR);
                    let v = f64::from(left[from + i]);
                    re += v * phase.cos();
                    im -= v * phase.sin();
                }
                (re * re + im * im).sqrt() / period as f64
            };
            (magnitude(period), magnitude(period * 3))
        };

        let (bright_first, bright_third) = measure(6000.0, 0.0);
        let (damped_first, damped_third) = measure(6000.0, 0.85);
        let bright_ratio = bright_third / bright_first;
        let damped_ratio = damped_third / damped_first;
        assert!(
            damped_ratio < bright_ratio * 0.4,
            "damped {damped_ratio} vs bright {bright_ratio}"
        );

        // 低频几乎不受影响：这是一条高频损失。
        let (low_bright_first, low_bright_third) = measure(120.0, 0.0);
        let (low_damped_first, low_damped_third) = measure(120.0, 0.85);
        let low_bright = low_bright_third / low_bright_first;
        let low_damped = low_damped_third / low_damped_first;
        assert!(
            low_damped > low_bright * 0.6,
            "damping should not gut the low end"
        );
    }

    #[test]
    fn a_time_change_slides_instead_of_jumping() {
        // 读取指针硬跳会咔哒；滑行正是阻止它的东西。喂一个音、中途改时间，
        // 然后找比该音自身样本间变化大得多的台阶。
        let mut d = delay(0.2, 0.0, 1.0, 0.0, false);
        let frames = 12_000;
        // 低频音：抽头滑动时会重采样它（这是刻意的多普勒），因此输出频率由滑行
        // 速率而不是输入决定。绝不能出现的是**不连续**。
        let tone = |offset: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| (core::f32::consts::TAU * 40.0 * (i + offset) as f32 / SR).sin())
                .collect()
        };
        let mut left = tone(0);
        let mut right = tone(0);
        d.process(
            DelayParams {
                time_s: 0.2,
                feedback: 0.0,
                mix: 1.0,
                damp: 0.0,
                ping_pong: false,
            },
            &mut left,
            &mut right,
        );
        let mut left2 = tone(frames);
        let mut right2 = tone(frames);
        d.process(
            DelayParams {
                time_s: 0.05,
                feedback: 0.0,
                mix: 1.0,
                damp: 0.0,
                ping_pong: false,
            },
            &mut left2,
            &mut right2,
        );

        // 抽头每样本最多滑 TIME_SLEW × 全距离，因此该音最多被移调约 7×，
        // 它的台阶仍然很小。
        let most_shifted_step = core::f32::consts::TAU * 40.0 * 7.0 / SR;
        let mut worst = 0.0f32;
        for pair in left2.windows(2) {
            worst = worst.max((pair[1] - pair[0]).abs());
        }
        assert!(
            worst < most_shifted_step * 1.5,
            "delay time change stepped by {worst} (a click would be far larger)"
        );

        // 而且抽头本身是平滑移动而不是跳过去的。
        let distance = (0.2 - 0.05) * SR;
        assert!(
            (d.target - d.samples).abs() < distance,
            "the tap should still be on its way, not already there"
        );
    }

    #[test]
    fn an_unallocated_delay_is_a_passthrough() {
        let mut d = Delay::new();
        assert!(!d.is_configured());
        let mut left = [0.5f32, -0.25];
        let mut right = [0.1f32, 0.2];
        d.process(DelayParams::new(), &mut left, &mut right);
        assert_eq!(left, [0.5, -0.25]);
        assert_eq!(right, [0.1, 0.2]);
    }

    /// 两个实例是两条线：时间、抽头与反馈各自独立，
    /// 跑其中一个不可能移动或染色另一个。
    #[test]
    fn two_instances_do_not_share_time_or_feedback() {
        let frames = 24_000;
        let short = DelayParams {
            time_s: 0.05,
            feedback: 0.0,
            mix: 1.0,
            damp: 0.0,
            ping_pong: false,
        };
        let long = DelayParams {
            time_s: 0.2,
            feedback: 0.7,
            mix: 1.0,
            damp: 0.0,
            ping_pong: false,
        };

        let render = |params: DelayParams| {
            let mut d = delay(
                params.time_s,
                params.feedback,
                params.mix,
                params.damp,
                params.ping_pong,
            );
            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            left[0] = 1.0;
            right[0] = 1.0;
            d.process(params, &mut left, &mut right);
            (left, right)
        };

        // 各自单独跑，然后同一趟里跑两个。
        let (short_alone, _) = render(short);
        let (long_alone, _) = render(long);
        let mut a = delay(
            short.time_s,
            short.feedback,
            short.mix,
            short.damp,
            short.ping_pong,
        );
        let mut b = delay(
            long.time_s,
            long.feedback,
            long.mix,
            long.damp,
            long.ping_pong,
        );
        let mut together_short = vec![0.0f32; frames];
        let mut together_long = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        together_short[0] = 1.0;
        together_long[0] = 1.0;
        a.process(short, &mut together_short, &mut r);
        b.process(long, &mut together_long, &mut r);

        assert_eq!(
            together_short, short_alone,
            "the second instance changed the first one's output"
        );
        assert_eq!(
            together_long, long_alone,
            "the first instance changed the second one's output"
        );
        // …而且每条回声落在自己的时间上、有自己的重复次数。
        let short_tap = (short.time_s * SR) as usize;
        let long_tap = (long.time_s * SR) as usize;
        assert!(
            short_alone[short_tap].abs() > 0.9,
            "the 50 ms echo should be there"
        );
        assert!(
            short_alone[short_tap * 2].abs() < 1e-4,
            "feedback 0 makes exactly one echo"
        );
        assert!(
            long_alone[long_tap].abs() > 0.05,
            "the 200 ms echo should be there"
        );
        assert!(
            long_alone[long_tap * 2].abs() > 0.05,
            "feedback 0.7 keeps repeating"
        );
    }

    /// **判据（新写，可红）**：延迟时间以**秒**计，与采样率无关。
    ///
    /// 来源的全部测试都钉在 48 kHz：一个把 48 kHz 写死的实现能全部通过。
    /// 这里在 96 kHz 下断言同一条 50 ms 延迟的回声落在 4800（而不是 2400）样本处。
    ///
    /// 把 `configure` 里 `sample_rate` 的用法换成固定的 48_000.0，
    /// 本测试立即变红。
    #[test]
    fn the_delay_time_is_seconds_not_samples() {
        for (sample_rate, expected_tap) in [(48_000.0f32, 2400usize), (96_000.0, 4800)] {
            let mut d = Delay::new();
            d.configure(sample_rate);
            let params = DelayParams {
                time_s: 0.05,
                feedback: 0.0,
                mix: 1.0,
                damp: 0.0,
                ping_pong: false,
            };
            // 预热滑行时间。
            let mut warm_l = [0.0f32; 1];
            let mut warm_r = [0.0f32; 1];
            d.process(params, &mut warm_l, &mut warm_r);
            let frames = expected_tap * 3;
            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            left[0] = 1.0;
            d.process(params, &mut left, &mut right);
            let peak = left
                .iter()
                .enumerate()
                .skip(1)
                .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
                .map(|(i, _)| i)
                .unwrap();
            assert_eq!(
                peak, expected_tap,
                "at {sample_rate} Hz a 50 ms delay must land at {expected_tap} samples"
            );
            assert!(
                (d.max_seconds() - MAX_DELAY_SECONDS).abs() < 1e-3,
                "the line must hold {MAX_DELAY_SECONDS} s at any rate"
            );
        }
    }

    /// **判据（新写，可红）**：退化参数必须被钳制，而不是产生 `NaN` 或失控。
    #[test]
    fn degenerate_parameters_never_escape_the_clamps() {
        let mut d = Delay::new();
        d.configure(f32::NAN);
        assert!(d.is_configured());
        let absurd = DelayParams {
            time_s: f32::NAN,
            feedback: f32::NAN,
            mix: f32::NAN,
            damp: f32::NAN,
            ping_pong: false,
        };
        let mut left = vec![0.3f32; 4096];
        let mut right = vec![-0.3f32; 4096];
        d.process(absurd, &mut left, &mut right);
        assert!(left.iter().all(|v| v.is_finite()));
        assert!(right.iter().all(|v| v.is_finite()));
        // 反馈被钳在 0.95：即使一直喂满幅直流，也必须收敛到有界。
        let hot = DelayParams {
            time_s: 0.01,
            feedback: 10.0,
            mix: 1.0,
            damp: 0.0,
            ping_pong: false,
        };
        let mut left = vec![1.0f32; 96_000];
        let mut right = vec![1.0f32; 96_000];
        d.process(hot, &mut left, &mut right);
        let peak = left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            peak.is_finite() && peak < 100.0,
            "clamped feedback ran away: {peak}"
        );
        // 超长的时间请求被钳到线的容量之内。
        let long = DelayParams {
            time_s: 1.0e9,
            feedback: 0.0,
            mix: 1.0,
            damp: 0.0,
            ping_pong: false,
        };
        let mut left = vec![0.0f32; 64];
        let mut right = vec![0.0f32; 64];
        d.process(long, &mut left, &mut right);
        assert!(left.iter().all(|v| v.is_finite()));
        assert!(d.max_seconds() <= MAX_DELAY_SECONDS + 1e-3);
    }
}
