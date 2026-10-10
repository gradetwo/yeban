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
//!
//! ## 非有限参数（入口回落）
//!
//! `value` 是**递归**状态（`value ← value + (1 − α)·(target − value)`），而它的两个
//! 自变量都来自控制面。非有限输入一旦写进它，每一次迭代都把它原样留下：`NaN` 是
//! 不动点（`NaN − NaN = NaN`），`±∞` 更糟 —— 下一步 `∞ − ∞` 就化成 `NaN`。
//! `reset` 之外没有出路，而本类型没有 `reset`。因此两个**写目标 / 写值**的入口
//! （[`ParamSmoother::set_target`]、[`ParamSmoother::snap_to`]）都**忽略**非有限
//! 输入，保持当前值与当前目标不动。构造期与 [`ParamSmoother::set_time_constant`]
//! 那一侧的守卫在 [`ParamSmoother::recompute`] 里（非有限 τ 是"关闭平滑"）。

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
    ///
    /// ⚠ **非有限目标被忽略**（当前目标与当前值都不动）。`alpha`／`value`／`target`
    /// 里只有 `value` 是**递归**状态，而一个非有限目标会在**同一个样本**里把它写成
    /// 非有限值：`difference = target − value` 是 `NaN`／`±∞`，吸附判断为假，
    /// `value += (1 − α)·difference` 于是把 `NaN`／`±∞` 留在 `value` 上；
    /// 此后每一步都是同一个结果（`NaN − NaN = NaN`），输入恢复干净也回不来。
    /// 实时路径上无法报错，只能在入口回落。
    ///
    /// "忽略"而不是"退回某个默认值"是**保守**的一侧：本类型是**通用**平滑器
    /// （增益、截止频率、声像都用它），没有一个能对所有用途都安全的默认值 ——
    /// 对增益而言 `0.0` 是静音，那是本仓库明文登记过的"巨大且听得见的改动"
    /// （见 [`crate::channel_strip::ChannelStripParams::sanitised`] 的说明）。
    /// 忽略也正与唯一的生产调用方 `yeban-engine` 的 `ParamTable` 对非法参数的口径
    /// 一致（那里是"拒绝且**不施加**"）；同一条口径的另一个既有站点是
    /// [`crate::noise::NoiseGen::set_corner_hz`]。
    ///
    /// **有限目标逐位不变**。
    pub fn set_target(&mut self, target: f32) {
        if target.is_finite() {
            self.target = target;
        }
    }

    /// 当前目标值。
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.target
    }

    /// 立即跳到某个值（初始化、复位、采样率切换时使用；不产生渐变）。
    ///
    /// ⚠ **非有限的 `value` 被忽略**（当前值与当前目标都不动），理由与
    /// [`Self::set_target`] 相同。这里还多一条：本方法是**唯一**能把一个已被毒化的
    /// 平滑器救回来的入口（它直接写 `value`），因此它自己更不能写进非有限值 ——
    /// 否则"回落"这条路就被堵死了。
    ///
    /// # 调用纪律（⚠ 这条说明**不可判据化**，按裁决 R51 的形态）
    ///
    /// 本方法**同时写 `value` 与 `target`** —— 这是"立即跳到某个值"的定义本身
    /// （⛔ 去掉写 `target` 会让"立即"不成立），也是它作为**唯一救援入口**的前提。
    /// 因此：**同一个量子内不要既 `set_target` 又 `snap_to`** —— 后者会**静默取消**
    /// 前者写入的目标（`grep -n 'self.target ='` 只有 `set_target` 与 `snap_to` 两处）。
    ///
    /// ⚠ **本段不是判据**：它约束的是**调用方**的时序，而本 crate 看不见调用方
    /// （裁决 R76 的引擎侧判据 `pan_automation_resets()` 计数才是那条契约的落点）。
    /// 在本 crate 内没有任何单点注入能让"违反了这条纪律"变红 ⇒ 按 R51 如实标注：
    /// **这是一条调用纪律，不是一条已验证的判据**。
    pub fn snap_to(&mut self, value: f32) {
        if value.is_finite() {
            self.value = value;
            self.target = value;
        }
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
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

    /// **判据（新写，可红）**：吸附门限有 `1.0` 的**绝对下限**，因此远小于门限的
    /// 目标在**一个样本**内被取到。
    ///
    /// 量什么：`process()` 的返回值（线性参数值，`f32` 位型）与 `is_settled()`。
    ///
    /// 文档化的门限是 `1e-5 · max(|value|, |target|, 1)` —— `1.0` 那一项让门限在
    /// **小参数上不随参数缩小**。它正是模块注释里那条理由：差值继续缩小会进入
    /// 次正规区间（x86 上可达 100× 周期），[ARCH-RT-003] 要根除那条路径。
    /// 注入实测：把 `scale` 的 `.max(1.0)` 去掉 ⇒ 门限缩成 `1e-5 · 1e-6 = 1e-11`，
    /// 第一步只走 `(1 − α)·1e-6 ≈ 4.2e-9` ⇒ 本判据变红。
    #[test]
    fn the_snap_threshold_has_an_absolute_floor_for_tiny_parameters() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.snap_to(0.0);
        smoother.set_target(1.0e-6);
        let first = smoother.process();
        assert_eq!(
            first.to_bits(),
            1.0e-6f32.to_bits(),
            "远小于绝对门限的目标必须在一个样本内被取到（实得 {first:e}）"
        );
        assert!(smoother.is_settled());
    }
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

    /// **判据（新写，可红）**：文档化的吸附门限量级被两端夹住。
    ///
    /// 量什么：阶跃响应里 `is_settled()` 第一次为真的样本下标（单位：样本）。
    ///
    /// `τ = 5 ms` @48 kHz 的误差按 `exp(-n/240)` 缩小，吸附发生在
    /// `|目标 − 值| ≤ 1e-5 · scale` 时 ⇒ 下标约 `240 · ln(1e5) ≈ 2 763`。
    /// 因此 `2 000` 之前**不得**吸附（否则门限被放大到 `> 2.4e-4`，参数在可听误差
    /// 处就被硬拽到目标），`3 000` 之时**必须**已经吸附且逐位相等（否则门限被压到
    /// 浮点停摆点 `≈ 7e-6` 之下，平滑器停摆）。
    ///
    /// 注入实测：`SNAP_RELATIVE` 从 `1e-5` 改成 `1e-3` ⇒ 吸附提前到约 `1 658` 样本
    /// ⇒ 第一条断言变红；改成 `1e-7` ⇒ 停摆、`expect` 变红。
    ///
    /// 链路含 `exp` ⇒ 按 [ADR-0001 D32] 属超越函数类，因此只断言**区间**，
    /// ⛔ 不钉精确下标。
    #[test]
    fn the_snap_threshold_sits_between_the_stall_point_and_the_audible_error() {
        let mut smoother = ParamSmoother::with_default_time(SR);
        smoother.snap_to(0.0);
        smoother.set_target(1.0);
        let mut settled_at = None;
        for sample in 0..3_000 {
            smoother.process();
            if smoother.is_settled() {
                settled_at = Some(sample + 1);
                break;
            }
        }
        let settled_at = settled_at.expect("3 000 样本之内必须吸附（门限不得低于浮点停摆点）");
        assert!(
            settled_at > 2_000,
            "吸附发生在样本 {settled_at} ⇒ 门限被放大到可听误差量级"
        );
        assert!(settled_at <= 3_000, "吸附不得晚于 3 000 样本: {settled_at}");
        assert_eq!(
            smoother.value().to_bits(),
            1.0f32.to_bits(),
            "吸附必须逐位取目标值"
        );
    }

    /// **判据（新写，可红）**：极大的时间常数不得把 α 舍入成 `1.0`。
    ///
    /// 量什么：`alpha()`（无量纲）与推进 64 个样本后的输出（线性参数值）。
    ///
    /// [`ParamSmoother::recompute`] 的上界 `1.0 - f32::EPSILON` 存在的理由是
    /// "α 舍入到 1 会让平滑器永久卡住"，但既有夹具的时间常数最大只到 `10 s`
    ///（`τ·fs = 480 000`），离 `exp(-1/(τ·fs))` 在 `f32` 上舍入到 `1.0` 的门槛
    ///（`τ·fs > 2^24 ≈ 1.7e7`）还差两个数量级 ⇒ 那个上界从未被走到。
    /// 注入实测：上界改成 `1.0` ⇒ α 恰为 `1.0`，本判据第一条断言变红。
    ///
    /// 链路含 `exp` ⇒ 只断言性质（α 严格小于 1、递归仍然移动），⛔ 不钉精确值。
    #[test]
    fn a_time_constant_beyond_the_float_grid_cannot_round_alpha_to_one() {
        let mut smoother = ParamSmoother::new(SR, 1.0e30);
        assert!(
            smoother.alpha() < 1.0,
            "α 舍入到 1.0 ⇒ 平滑器永久卡死：{}",
            smoother.alpha()
        );
        assert!(smoother.alpha() >= 0.0, "α 不得为负: {}", smoother.alpha());
        smoother.snap_to(0.0);
        smoother.set_target(1.0);
        let mut moved = false;
        for _ in 0..64 {
            moved |= smoother.process() > 0.0;
        }
        assert!(moved, "输出一步都不动 ⇒ 递归被 α 停摆");
    }

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
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

    /// **判据（新写，可红）**：非有限**目标 / 吸附值**不得写进递归状态。
    ///
    /// 观测：吸附到 `0.4` 之后，把目标设成 `NaN`／`±∞`（再用同样的值调 `snap_to`），
    /// 然后推进 `1 000` 个样本。要求：目标与值都**逐位不动**、`is_settled` 仍为真、
    /// 每个输出逐位等于 `0.4`。最后验证**恢复路径**仍然活着（`snap_to` 一个新值仍生效）。
    ///
    /// 注入（实测红行见报告）：去掉 [`ParamSmoother::set_target`] 里的 `is_finite`
    /// 判断 ⇒ 第 0 个样本就是 `NaN`，本判据立即变红。同一处对
    /// [`ParamSmoother::snap_to`] 同理。
    #[test]
    fn a_non_finite_target_or_snap_never_enters_the_recursive_state() {
        for hostile in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut smoother = ParamSmoother::with_default_time(SR);
            smoother.snap_to(0.4);
            let value_before = smoother.value();
            let target_before = smoother.target();

            smoother.set_target(hostile);
            assert_eq!(
                smoother.target().to_bits(),
                target_before.to_bits(),
                "set_target({hostile}) 移动了目标"
            );
            assert_eq!(
                smoother.value().to_bits(),
                value_before.to_bits(),
                "set_target({hostile}) 移动了值"
            );

            smoother.snap_to(hostile);
            assert_eq!(
                smoother.target().to_bits(),
                target_before.to_bits(),
                "snap_to({hostile}) 移动了目标"
            );
            assert_eq!(
                smoother.value().to_bits(),
                value_before.to_bits(),
                "snap_to({hostile}) 移动了值"
            );

            assert!(smoother.is_settled(), "非有限值把'已吸附'拆散了");
            for sample in 0..1_000 {
                let out = smoother.process();
                assert_eq!(
                    out.to_bits(),
                    0.4f32.to_bits(),
                    "sample {sample}: 非有限值之后输出 {out}"
                );
            }

            // 恢复路径必须仍然活着：吸附到一个新值要真的生效。
            smoother.snap_to(0.9);
            assert_eq!(smoother.value(), 0.9);
            assert_eq!(smoother.target(), 0.9);
        }
    }
}
