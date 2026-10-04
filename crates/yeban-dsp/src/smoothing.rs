//! 参数自动化平滑：一阶低通 `y[n] = (1 − α)·x[n] + α·y[n−1]`。[ARCH-DSP-001] [ARCH-RT-003]
//!
//! **本模块是新写的**（来源库没有对应实现；规范 §4.1 给了公式，但没有给代码）。
//!
//! 规范要求："所有瞬变自动化事件经过单极点低通滤波（`y[n] = (1 - α) x[n] + α y[n-1]`，
//! 时间常数 τ ≈ 5ms），根除阶跃断崖引起的咔嗒杂音"。
//!
//! ## 为什么 α 在构造期算好
//!
//! `α = exp(−1 / (τ · fs))`。`exp` 每个样本算一次是实时路径上白送的开销，
//! 因此 α 只在采样率或时间常数变化时重算（[`ParamSmoother::set_time_constant`]），
//! 逐样本路径只有一次乘加。
//!
//! ## 为什么有"吸附"（snap）
//!
//! 一阶低通在浮点上**会停摆**：当 `(1 − α)·|target − value|` 小于 `|value|` 的
//! 半个 ulp 时，这个增量加到 `value` 上会被舍入掉，平滑器就永远停在离目标
//! 一点点的地方（实测：τ = 5 ms @48 kHz、target = 1 时卡在 0.99999285，
//! 差值 7.15e-6，而它再也走不动了）。这既是精度问题，也是次正规数问题：
//! 差值继续缩小时会进入次正规区间，触发 CPU 的次正规数慢路径
//!（x86 上可达 100× 周期），这正是 [ARCH-RT-003] 要根除的问题之一。
//!
//! 因此平滑器**主动吸附**：差值落到 `1e-5 · max(|value|, |target|, 1)` 之内时
//! 直接取目标值。这个门限比浮点停摆点（约 `7e-6 · |value|`）高一个量级，
//! 又远低于任何可听的参数误差；它也把"平滑完成后输出与目标逐位相等"变成
//! 一条可断言的判据。
//!
//! 本模块无堆分配、无全局状态、无锁 [ARCH-RT-001]。

use crate::math::sanitise_sample_rate;

/// 规范规定的时间常数：τ ≈ 5 ms [ARCH-DSP-001]。
pub const DEFAULT_TIME_CONSTANT_S: f32 = 0.005;

/// 吸附门限（相对量）。见模块注释里的浮点停摆实测。
const SNAP_RELATIVE: f32 = 1e-5;

/// 单参数一阶低通平滑器（一个参数一个实例，无共享状态）。
///
/// ```
/// use yeban_dsp::smoothing::ParamSmoother;
///
/// let mut cutoff = ParamSmoother::new(48_000.0, 0.005);
/// cutoff.snap_to(200.0);
/// cutoff.set_target(2_000.0);
/// let first = cutoff.process();
/// // 第一个样本只走了一小步，而不是跳到 2000。
/// assert!(first > 200.0 && first < 210.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSmoother {
    /// 平滑系数，构造期算好：`exp(-1 / (τ · fs))`，恒在 `[0, 1)`。
    alpha: f32,
    value: f32,
    target: f32,
    time_constant_s: f32,
    sample_rate: f32,
}

impl ParamSmoother {
    /// 以规范默认时间常数（5 ms）构造。
    #[must_use]
    pub fn with_default_time(sample_rate: f32) -> Self {
        Self::new(sample_rate, DEFAULT_TIME_CONSTANT_S)
    }

    /// 构造：`time_constant_s <= 0` 时退化为"无平滑"（α = 0，`next` 立即返回目标值）。
    ///
    /// "无平滑"是显式的、可测的模式，而不是把 τ 设成极小的数——后者在浮点上
    /// 会得到 `α` 舍入到 1 的意外结果。
    #[must_use]
    pub fn new(sample_rate: f32, time_constant_s: f32) -> Self {
        let mut smoother = Self {
            alpha: 0.0,
            value: 0.0,
            target: 0.0,
            time_constant_s,
            sample_rate: sanitise_sample_rate(sample_rate),
        };
        smoother.recompute();
        smoother
    }

    /// 重设采样率（α 随之重算；当前值与目标不变）。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.recompute();
        }
    }

    /// 重设时间常数（秒）。`<= 0` 表示关闭平滑。
    pub fn set_time_constant(&mut self, time_constant_s: f32) {
        if time_constant_s != self.time_constant_s {
            self.time_constant_s = time_constant_s;
            self.recompute();
        }
    }

    /// 当前采样率。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 当前时间常数（秒）。
    #[must_use]
    pub const fn time_constant(&self) -> f32 {
        self.time_constant_s
    }

    /// 平滑系数 α（测试与调试可读）。
    #[must_use]
    pub const fn alpha(&self) -> f32 {
        self.alpha
    }

    /// 设置自动化目标；输出按 τ 逼近它。
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// 当前目标值。
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.target
    }

    /// 立即跳到某个值（初始化、复位、采样率切换时使用；不产生渐变）。
    pub fn snap_to(&mut self, value: f32) {
        self.value = value;
        self.target = value;
    }

    /// 当前输出值。
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// 是否已经吸附到目标（逐位相等）。
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.value == self.target
    }

    /// 推进一个样本并返回新的输出值（等价于逐样本"处理一次"）。
    #[inline]
    pub fn process(&mut self) -> f32 {
        let difference = self.target - self.value;
        // 相对门限：`value` 的量级越大，半个 ulp 也越大，停摆点随之抬高。
        let scale = self.value.abs().max(self.target.abs()).max(1.0);
        if difference.abs() <= SNAP_RELATIVE * scale {
            self.value = self.target;
        } else {
            self.value += (1.0 - self.alpha) * difference;
        }
        self.value
    }

    /// 用平滑值填满一个块（每样本一个目标的消费者，例如颤音）。
    pub fn render_block(&mut self, out: &mut [f32]) {
        for slot in out.iter_mut() {
            *slot = self.process();
        }
    }

    /// 重算 α。`α = exp(−1 / (τ·fs))`；关闭平滑时 α = 0。
    ///
    /// `τ <= 0` 与 `τ` 非有限（`NaN`/`±inf`）都表示"关闭平滑"。
    /// 注意必须显式判 `NaN`：`NaN <= 0.0` 是 `false`，只写 `<= 0.0` 会让
    /// `exp(NaN)` 把 α 变成 `NaN`，从而把整个参数链路静默毒化。
    fn recompute(&mut self) {
        if !(self.time_constant_s.is_finite() && self.time_constant_s > 0.0)
            || !self.sample_rate.is_finite()
        {
            self.alpha = 0.0;
            return;
        }
        let samples = self.time_constant_s * self.sample_rate;
        let alpha = (-1.0 / samples).exp();
        // 极端参数（τ 极短或 fs 极低）下 α 可能舍入到 1，那会让平滑器永久卡住。
        self.alpha = alpha.clamp(0.0, 1.0 - f32::EPSILON);
    }
}

impl Default for ParamSmoother {
    fn default() -> Self {
        Self::with_default_time(48_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// **判据（新写，可红）**：阶跃输入下输出**不得**出现瞬时跳变。
    ///
    /// 单样本位移的上界是 `(1 − α)·Δ`，而这里的 α 由**规范**算出来
    ///（τ = 5 ms @48 kHz），**不是**读被测对象的 `alpha()`：从被测对象推导上界
    /// 会让这条判据在"平滑被关掉"时退化成同义反复（α = 0 时上界恰好等于 1.0，
    /// 于是"瞬时跳到满值"的实现也能通过）。
    ///
    /// 把 [`ParamSmoother::recompute`] 里的 α 改成 0（即"关掉平滑"），
    /// 第一个样本就会直接跳到 1.0 并越界，本测试立即变红。
    #[test]
    fn a_step_never_jumps_instantly() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.snap_to(0.0);
        smoother.set_target(1.0);
        // 规范推出的期望系数（与 `the_time_constant_is_five_milliseconds` 同一公式）。
        let spec_alpha = (-1.0f32 / (DEFAULT_TIME_CONSTANT_S * SR)).exp();
        let bound = (1.0 - spec_alpha) + 1e-6;
        // 30τ 之后必须已经吸附（逐位相等），因此循环里同时验证"收敛"。
        let steps = (30.0 * DEFAULT_TIME_CONSTANT_S * SR) as usize;
        let mut previous = 0.0f32;
        let mut seen_progress = false;
        for sample in 0..steps {
            let value = smoother.process();
            let step = value - previous;
            assert!(
                step >= 0.0 && step <= bound,
                "sample {sample}: step {step} exceeds the smoothed bound {bound}"
            );
            assert!(value <= 1.0, "sample {sample}: overshoot {value}");
            seen_progress |= value > 0.0;
            previous = value;
        }
        assert!(seen_progress, "output never moved toward the target");
        assert!(smoother.is_settled(), "never settled: {}", smoother.value());
    }

    /// **判据（新写，可红）**：时间常数与规范值一致。
    ///
    /// 一阶低通在 1τ 处到达阶跃的 `1 − 1/e ≈ 63.2%`。若 α 的公式写成
    /// `exp(-1/τ)`（漏掉采样率）或 `exp(-τ/fs)`，这条立即变红。
    #[test]
    fn the_time_constant_is_five_milliseconds() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.snap_to(0.0);
        smoother.set_target(1.0);
        let target_level = 1.0 - 1.0 / core::f32::consts::E;
        let mut crossing = None;
        for sample in 0..(SR as usize) {
            if smoother.process() >= target_level {
                crossing = Some(sample + 1);
                break;
            }
        }
        let crossing = crossing.expect("never reached 63.2%");
        let expected = (DEFAULT_TIME_CONSTANT_S * SR) as usize; // 240 样本
        assert!(
            crossing.abs_diff(expected) <= 2,
            "63.2% at sample {crossing}, expected ~{expected}"
        );
        // 系数本身也必须对：α = exp(-1/(τ·fs))。
        let expected_alpha = (-1.0f32 / (DEFAULT_TIME_CONSTANT_S * SR)).exp();
        assert!((smoother.alpha() - expected_alpha).abs() < 1e-6);
    }

    /// 平滑器与"每样本解析解"在数值上一致，而不仅是单调。
    ///
    /// 解析解用 `f64` 独立累加（不调用被测实现），因此这条判据同时覆盖了
    /// "递推写错方向"和"α 前后不一致"两类错误。
    #[test]
    fn the_response_matches_the_closed_form() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.snap_to(0.0);
        smoother.set_target(1.0);
        let alpha = (-1.0f64 / (f64::from(DEFAULT_TIME_CONSTANT_S) * f64::from(SR))).exp();
        let mut expected = 0.0f64;
        for sample in 0..512 {
            expected = 1.0 - (1.0 - expected) * alpha;
            let value = f64::from(smoother.process());
            assert!(
                (value - expected).abs() < 1e-4,
                "sample {sample}: {value} vs closed form {expected}"
            );
        }
    }

    #[test]
    fn a_disabled_smoother_is_explicitly_instant() {
        // τ = 0 是"无平滑"的显式表达：第一个样本就是目标值。
        let mut smoother = ParamSmoother::new(SR, 0.0);
        smoother.snap_to(0.0);
        smoother.set_target(0.75);
        assert_eq!(smoother.alpha(), 0.0);
        assert_eq!(smoother.process(), 0.75);
        assert!(smoother.is_settled());
    }

    #[test]
    fn snap_to_bypasses_the_ramp() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.set_target(1.0);
        smoother.snap_to(0.5);
        assert_eq!(smoother.value(), 0.5);
        assert_eq!(smoother.target(), 0.5);
        assert!(smoother.is_settled());
    }

    #[test]
    fn block_rendering_stays_on_the_ramp() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.snap_to(0.0);
        smoother.set_target(1.0);
        let mut block = [0.0f32; 64];
        smoother.render_block(&mut block);
        // 块内严格递增，且块尾仍远未到达目标（64 样本 ≈ 0.27τ）。
        for pair in block.windows(2) {
            assert!(pair[1] > pair[0]);
        }
        assert!(block[63] < 0.4, "ramp ran too fast: {}", block[63]);
    }

    #[test]
    fn degenerate_parameters_never_produce_nan_or_a_stuck_smoother() {
        for sample_rate in [0.0f32, -1.0, f32::NAN, 1.0, 48_000.0] {
            for tau in [0.0f32, -0.001, f32::NAN, 1e-9, 10.0] {
                let mut smoother = ParamSmoother::new(sample_rate, tau);
                assert!(
                    (0.0..1.0).contains(&smoother.alpha()),
                    "alpha {}",
                    smoother.alpha()
                );
                smoother.set_target(1.0);
                for _ in 0..1000 {
                    let value = smoother.process();
                    assert!(value.is_finite(), "sr {sample_rate} tau {tau}: {value}");
                }
            }
        }
    }

    #[test]
    fn sample_rate_changes_recompute_the_coefficient() {
        let mut smoother = ParamSmoother::with_default_time(48_000.0);
        let at_48k = smoother.alpha();
        smoother.set_sample_rate(96_000.0);
        assert!(
            smoother.alpha() > at_48k,
            "a shorter τ in samples must smooth less"
        );
        assert_eq!(smoother.sample_rate(), 96_000.0);
        // 同一个值重复设置不得改变系数（防御性调用的成本是零）。
        let stable = smoother.alpha();
        smoother.set_sample_rate(96_000.0);
        assert_eq!(smoother.alpha(), stable);
    }
}
