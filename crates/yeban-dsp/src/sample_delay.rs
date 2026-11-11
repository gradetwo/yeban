//! 采样延迟（Sample Delay）：纯延时 + 极性反转。
//!
//! 手动相位对齐第一层（轨延时 / Sample Delay device 共用的 DSP 本体）。
//!
//! 与 ping-pong 回声（[`crate::delay::Delay`]）不同：本器件是**前馈、整数采样、
//! 无反馈、无滑行** —— 延时量是精确整数，脉冲延时 `N` 采样后峰值位置精确移动 `N`
//! 采样；极性反转在延迟之后对输出取反（`out = -x(t - delay)`）。
//!
//! ## 语义
//!
//! - `delay_samples == 0` 且 `invert == false` ⇒ **逐位直通**（与输入逐字节相同）。
//! - `delay_samples == N` ⇒ 前 `N` 帧输出静音，之后是输入延迟 `N` 采样
//!   （与离线渲染的 PDC 延迟线同一口径；前 `N` 帧静音是延迟线应有的行为，
//!   不是 bug）。
//! - `invert == true` ⇒ 输出取反；与延时组合时先延时、后取反。
//! - 延时量是**整数采样**：毫秒由调用方按工程采样率换算（[`millis_to_samples`]），
//!   换算取最接近的整数（四舍五入），`0.1 ms` 在 44.1 kHz 下是 4 采样、48 kHz 下是
//!   5 采样 —— 小于 1 采样的毫秒量无法表达，调用方须如实告知用户。
//!
//! ## 非有限输入样本
//!
//! 与 [`crate::delay::Delay`] 同一条纪律：写进延迟线之前过
//! [`crate::math::finite_or_zero`]，`NaN` 与 `±∞` 归 `0.0`，**有限样本逐位不变**。
//! 前馈线没有递归状态，非有限值不会永久污染本器件；守卫只是让输出保持确定。
//!
//! ## 确定性 [ARCH-DET-001]
//!
//! 同一台刚 `configure` 过的实例、同一组参数、同一段输入 ⇒ 逐位相同的输出；
//! [`SampleDelay::reset`] 之后与刚 `configure` 过的实例逐位相同（清历史、不动配置）。

use crate::math::finite_or_zero;

/// 采样延迟能装下的最长时间（采样点）。100 ms @ 192 kHz 是 19_200 采样；
/// 本上限覆盖 100 ms @ 192 kHz 并留有余量（1 s @ 48 kHz）。
pub const MAX_SAMPLE_DELAY_SAMPLES: usize = 48_000;

/// 毫秒换算成采样点（取最接近的整数，四舍五入）。
///
/// `millis` 必须有限且非负；越界（`millis < 0.0` 或非有限）返回 `0`。
/// 换算不钳制到 [`MAX_SAMPLE_DELAY_SAMPLES`] —— 钳制是 [`SampleDelay::process`]
/// 的事（调用方读得到实际落点）。
#[must_use]
pub fn millis_to_samples(millis: f32, sample_rate: u32) -> usize {
    if !millis.is_finite() || millis < 0.0 || sample_rate == 0 {
        return 0;
    }
    let samples = f64::from(millis) * f64::from(sample_rate) / 1000.0;
    if samples >= usize::MAX as f64 {
        return usize::MAX;
    }
    samples.round() as usize
}

/// 采样点换算成毫秒（调用方做显示用；不取整）。
#[must_use]
pub fn samples_to_millis(samples: usize, sample_rate: u32) -> f32 {
    if sample_rate == 0 {
        return 0.0;
    }
    samples as f32 * 1000.0 / sample_rate as f32
}

/// 采样延迟参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleDelayParams {
    /// 延迟量（采样点），`0..=MAX_SAMPLE_DELAY_SAMPLES`；超出由 `process` 钳制。
    pub delay_samples: usize,
    /// 极性反转：输出取反。
    pub invert: bool,
}

impl SampleDelayParams {
    /// 默认：零延迟、不反转（直通）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            delay_samples: 0,
            invert: false,
        }
    }

    /// 本参数是否是**恒等**（零延迟且不反转 ⇒ `process` 逐位直通）。
    ///
    /// 运行态的建级门：恒等链不建器件（省下 `2 × 48_001` 帧缓冲），
    /// 与乒乓延迟的 `mix = 0` 门同款。
    #[must_use]
    pub const fn is_identity(self) -> bool {
        self.delay_samples == 0 && !self.invert
    }
}

impl Default for SampleDelayParams {
    fn default() -> Self {
        Self::new()
    }
}

/// 采样延迟线（两声道各自独立，状态互不共享）。
pub struct SampleDelay {
    lines: [Vec<f32>; 2],
    index: usize,
    delay_samples: usize,
    ready: bool,
    /// 块边界 / 武装期写入的运行参数（实时单声道路径用这一份；离线 `process` 仍由
    /// 调用方逐块给参数，见 [`Self::process`]）。
    params: SampleDelayParams,
}

impl SampleDelay {
    /// 构造一个**未配置**的实例（直通）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lines: [Vec::new(), Vec::new()],
            index: 0,
            delay_samples: 0,
            ready: false,
            params: SampleDelayParams::new(),
        }
    }

    /// 配置（分配缓冲）。**只能在音频回调之外调用** [ARCH-RT-001]。
    ///
    /// 容量恒为 [`MAX_SAMPLE_DELAY_SAMPLES`] + 1（全量）；实时池按配置延迟配容量时
    /// 用 [`Self::configure_with_capacity`]（65 槽 × 全量 = 约 25 MiB，见设计）。
    pub fn configure(&mut self, sample_rate: f32) {
        self.configure_with_capacity(sample_rate, MAX_SAMPLE_DELAY_SAMPLES);
    }

    /// 按给定延迟量配置（分配缓冲）。**只能在音频回调之外调用** [ARCH-RT-001]。
    ///
    /// 每声道分配 `capacity + 1` 帧（`capacity` 钳到 `0..=MAX_SAMPLE_DELAY_SAMPLES`）：
    /// `capacity + 1` 是 `delay == capacity` 仍能读到正确历史的最小长度
    /// （`process` 的读下标是 `(index + capacity - delay) % capacity`，容量恰好
    /// `delay + 1` 时读到上一圈刚写入的那一帧）。容量不足时 `process` 把延迟钳到
    /// `容量 − 1`（⛔ 不下溢、不静默丢声，只缩短延迟 —— 武装期超容拒装是另一道门）。
    pub fn configure_with_capacity(&mut self, sample_rate: f32, capacity: usize) {
        let _ = sample_rate;
        let capacity = capacity.min(MAX_SAMPLE_DELAY_SAMPLES) + 1;
        self.lines = [vec![0.0; capacity], vec![0.0; capacity]];
        self.index = 0;
        self.delay_samples = 0;
        self.ready = true;
    }

    /// 已配缓冲的容量（帧；未配置 = 0）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.lines[0].len()
    }

    /// 写入运行参数（标量赋值，零分配；只供实时武装期用，离线 `process` 不读它）。
    pub fn set_params(&mut self, params: SampleDelayParams) {
        self.params = params;
    }

    /// 当前运行参数（武装期写入的那一份；`process` 的显式参数不经过这里）。
    #[must_use]
    pub const fn params(&self) -> SampleDelayParams {
        self.params
    }

    /// 是否已配置。
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.ready
    }

    /// 当前延迟量（采样点）。
    #[must_use]
    pub const fn delay_samples(&self) -> usize {
        self.delay_samples
    }

    /// 复位：清历史、不动配置。复位后的实例与刚 `configure` 过的实例逐位相同。
    pub fn reset(&mut self) {
        for line in self.lines.iter_mut() {
            line.fill(0.0);
        }
        self.index = 0;
        self.delay_samples = 0;
    }

    /// 处理一对立体声切片（原地）。
    ///
    /// 未配置时直通（不经过非有限守卫，没有样本进入延迟线）。
    /// 已配置时：延迟量钳制到 `MAX_SAMPLE_DELAY_SAMPLES` **且** `容量 − 1`（容量是
    /// [`Self::configure_with_capacity`] 配的那一份；钳制只缩短延迟，⛔ 不下溢）；
    /// 前 `delay` 帧输出静音。
    pub fn process(&mut self, params: SampleDelayParams, left: &mut [f32], right: &mut [f32]) {
        if !self.ready {
            return;
        }
        let capacity = self.lines[0].len();
        let delay = params
            .delay_samples
            .min(MAX_SAMPLE_DELAY_SAMPLES)
            .min(capacity.saturating_sub(1));
        self.delay_samples = delay;
        let len = left.len().min(right.len());
        if delay == 0 {
            if params.invert {
                for sample in left.iter_mut().take(len) {
                    *sample = -finite_or_zero(*sample);
                }
                for sample in right.iter_mut().take(len) {
                    *sample = -finite_or_zero(*sample);
                }
            }
            return;
        }
        for i in 0..len {
            let read = (self.index + capacity - delay) % capacity;
            let l_in = finite_or_zero(left[i]);
            let r_in = finite_or_zero(right[i]);
            let l_out = self.lines[0][read];
            let r_out = self.lines[1][read];
            self.lines[0][self.index] = l_in;
            self.lines[1][self.index] = r_in;
            self.index = (self.index + 1) % capacity;
            if params.invert {
                left[i] = -l_out;
                right[i] = -r_out;
            } else {
                left[i] = l_out;
                right[i] = r_out;
            }
        }
    }

    /// 处理一段单声道切片（原地），参数取武装期 [`Self::set_params`] 写入的那一份。
    ///
    /// 实时插入链是单声道（`track_scratch`）：只用左声道那条线；右声道那条线不动。
    /// 未配置时直通。延迟钳制与 [`Self::process`] 同一条（`容量 − 1`）。
    pub fn process_mono(&mut self, mono: &mut [f32]) {
        if !self.ready {
            return;
        }
        let params = self.params;
        let capacity = self.lines[0].len();
        let delay = params
            .delay_samples
            .min(MAX_SAMPLE_DELAY_SAMPLES)
            .min(capacity.saturating_sub(1));
        self.delay_samples = delay;
        if delay == 0 {
            if params.invert {
                for sample in mono.iter_mut() {
                    *sample = -finite_or_zero(*sample);
                }
            }
            return;
        }
        for sample in mono.iter_mut() {
            let read = (self.index + capacity - delay) % capacity;
            let input = finite_or_zero(*sample);
            let out = self.lines[0][read];
            self.lines[0][self.index] = input;
            self.index = (self.index + 1) % capacity;
            *sample = if params.invert { -out } else { out };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> SampleDelay {
        let mut d = SampleDelay::new();
        d.configure(48_000.0);
        d
    }

    fn pulse(len: usize, at: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0_f32; len];
        let mut right = vec![0.0_f32; len];
        left[at] = 1.0;
        right[at] = -0.5;
        (left, right)
    }

    /// 脉冲延时 N 采样后峰值位置精确移动 N（两声道）。
    #[test]
    fn pulse_moves_exactly_n_samples() {
        for delay in [1_usize, 2, 7, 100, 4800] {
            let mut d = configured();
            let (mut left, mut right) = pulse(6000, 10);
            d.process(
                SampleDelayParams {
                    delay_samples: delay,
                    invert: false,
                },
                &mut left,
                &mut right,
            );
            assert_eq!(left[10 + delay], 1.0, "左声道峰值未精确移动 {delay}");
            assert_eq!(right[10 + delay], -0.5, "右声道峰值未精确移动 {delay}");
            assert!(
                left[..10 + delay].iter().all(|v| *v == 0.0),
                "延迟 {delay} 下峰值之前必须全静音"
            );
        }
    }

    /// 极性反转后信号取反（零延时与带延时两种情形）。
    #[test]
    fn invert_negates_signal() {
        let mut d = configured();
        let (mut left, mut right) = pulse(64, 5);
        let mut right = right;
        d.process(
            SampleDelayParams {
                delay_samples: 0,
                invert: true,
            },
            &mut left,
            &mut right,
        );
        assert_eq!(left[5], -1.0);
        assert_eq!(right[5], 0.5);

        let mut d = configured();
        let (mut left, mut right) = pulse(64, 5);
        d.process(
            SampleDelayParams {
                delay_samples: 3,
                invert: true,
            },
            &mut left,
            &mut right,
        );
        assert_eq!(left[8], -1.0);
        assert_eq!(right[8], 0.5);
    }

    /// 零延迟不反转 = 逐位直通。
    #[test]
    fn zero_delay_no_invert_is_bitwise_passthrough() {
        let mut d = configured();
        let (mut left, mut right) = pulse(64, 5);
        // 恒等门：只有零延迟且不反转才是恒等。
        assert!(SampleDelayParams::new().is_identity());
        assert!(
            !SampleDelayParams {
                delay_samples: 1,
                invert: false,
            }
            .is_identity()
        );
        assert!(
            !SampleDelayParams {
                delay_samples: 0,
                invert: true,
            }
            .is_identity()
        );
        let (want_l, want_r) = (left.clone(), right.clone());
        d.process(SampleDelayParams::default(), &mut left, &mut right);
        assert_eq!(left, want_l);
        assert_eq!(right, want_r);
    }

    /// 毫秒→采样换算在 44.1k 与 48k 下都对。
    #[test]
    fn millis_to_samples_at_both_rates() {
        assert_eq!(millis_to_samples(1.0, 44_100), 44);
        assert_eq!(millis_to_samples(1.0, 48_000), 48);
        assert_eq!(millis_to_samples(0.1, 44_100), 4);
        assert_eq!(millis_to_samples(0.1, 48_000), 5);
        assert_eq!(millis_to_samples(100.0, 44_100), 4410);
        assert_eq!(millis_to_samples(100.0, 48_000), 4800);
        assert_eq!(millis_to_samples(-1.0, 48_000), 0);
        assert_eq!(millis_to_samples(f32::NAN, 48_000), 0);
        assert_eq!(millis_to_samples(1.0, 0), 0);
    }

    /// 同一参数两次输出一致（确定性）。
    #[test]
    fn same_params_same_output_twice() {
        let params = SampleDelayParams {
            delay_samples: 37,
            invert: true,
        };
        let (mut l1, mut r1) = pulse(512, 3);
        let (mut l2, mut r2) = pulse(512, 3);
        configured().process(params, &mut l1, &mut r1);
        configured().process(params, &mut l2, &mut r2);
        assert_eq!(l1, l2);
        assert_eq!(r1, r2);
    }

    /// 复位后与刚配置过的实例逐位相同。
    #[test]
    fn reset_reproduces_a_freshly_built_device_bit_for_bit() {
        let params = SampleDelayParams {
            delay_samples: 19,
            invert: false,
        };
        let mut used = configured();
        let (mut l0, mut r0) = pulse(256, 2);
        used.process(params, &mut l0, &mut r0);
        used.reset();
        let (mut l1, mut r1) = pulse(256, 2);
        used.process(params, &mut l1, &mut r1);
        let (mut l2, mut r2) = pulse(256, 2);
        configured().process(params, &mut l2, &mut r2);
        assert_eq!(l1, l2);
        assert_eq!(r1, r2);
    }

    /// 超限延迟钳制、不 panic：脉冲落在钳制后的延迟位置。
    #[test]
    fn delay_beyond_max_is_clamped() {
        let mut d = configured();
        let (mut left, mut right) = pulse(MAX_SAMPLE_DELAY_SAMPLES + 100, 0);
        d.process(
            SampleDelayParams {
                delay_samples: usize::MAX,
                invert: false,
            },
            &mut left,
            &mut right,
        );
        assert_eq!(d.delay_samples(), MAX_SAMPLE_DELAY_SAMPLES);
        assert_eq!(left[MAX_SAMPLE_DELAY_SAMPLES], 1.0);
        assert_eq!(right[MAX_SAMPLE_DELAY_SAMPLES], -0.5);
        assert!(left[..MAX_SAMPLE_DELAY_SAMPLES].iter().all(|v| *v == 0.0));
    }

    /// 未配置时直通（不改输入）。
    #[test]
    fn unconfigured_is_passthrough() {
        let mut d = SampleDelay::new();
        let (mut left, mut right) = pulse(32, 4);
        let (want_l, want_r) = (left.clone(), right.clone());
        d.process(
            SampleDelayParams {
                delay_samples: 5,
                invert: true,
            },
            &mut left,
            &mut right,
        );
        assert_eq!(left, want_l);
        assert_eq!(right, want_r);
    }

    /// `process_mono` 用武装期参数：脉冲精确移动 N 采样，且只动左声道那条线。
    #[test]
    fn process_mono_moves_pulse_by_armed_params() {
        let mut d = SampleDelay::new();
        d.configure_with_capacity(48_000.0, 16);
        d.set_params(SampleDelayParams {
            delay_samples: 5,
            invert: false,
        });
        let mut mono = vec![0.0_f32; 32];
        mono[4] = 1.0;
        d.process_mono(&mut mono);
        assert_eq!(mono[9], 1.0);
        assert!(mono[..9].iter().all(|v| *v == 0.0));
        assert_eq!(d.delay_samples(), 5);
    }

    /// `process_mono` 的反相：零延迟也取反（只读武装参数，不读 `process` 的显式参数）。
    #[test]
    fn process_mono_inverts_with_armed_params() {
        let mut d = SampleDelay::new();
        d.configure_with_capacity(48_000.0, 0);
        d.set_params(SampleDelayParams {
            delay_samples: 0,
            invert: true,
        });
        let mut mono = vec![0.5_f32, -0.25, 0.0];
        d.process_mono(&mut mono);
        assert_eq!(mono, vec![-0.5, 0.25, 0.0]);
    }

    /// 容量不足时延迟钳到 `容量 − 1`，不下溢、不 panic：配 8 帧容量、要 240 采样延迟
    /// ⇒ 实际延迟 8 采样（`capacity()` 读数是 9 帧 = 8 + 1）。
    #[test]
    fn delay_beyond_configured_capacity_is_clamped_not_panicking() {
        let mut d = SampleDelay::new();
        d.configure_with_capacity(48_000.0, 8);
        assert_eq!(d.capacity(), 9);
        let (mut left, mut right) = pulse(32, 4);
        d.process(
            SampleDelayParams {
                delay_samples: 240,
                invert: false,
            },
            &mut left,
            &mut right,
        );
        assert_eq!(d.delay_samples(), 8);
        assert_eq!(left[12], 1.0);
        assert_eq!(right[12], -0.5);
    }
}
