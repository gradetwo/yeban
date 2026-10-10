//! 四极谐振低通（TPT / 零延迟反馈梯形滤波器）。[ARCH-DSP-001] [ARCH-RT-003]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/ladder.rs`（含其两条关键回归测试）。
//!
//! 反馈环里有四级单极点与一个有界饱和级，这就是经典的 24 dB/oct 合成器低通。
//! 取自研实现而不是 vendored DaisySP，原因见来源文件头：那份 C++ 在 wasm 构建下
//! 每个渲染块边界都会丢一个样本（每 128 采样一次咔哒声），而同一份 C++ 在宿主上
//! 是干净的。自己拥有这个滤波器就消除了该依赖，并且在每个目标上行为一致。
//!
//! ## 与来源的差异
//!
//! 1. [`LadderFilter::set`] 改名 [`LadderFilter::configure`]（`set` 在夜半的读法里
//!    太含糊），并显式钳制采样率；
//! 2. `fast_tanh` 收敛到 [`crate::math::pade_tanh`]，避免两份 Padé 系数漂移；
//! 3. 私有 `soft_clip` 改名 `bounded_saturate`：来源里有两个不同的 `soft_clip`
//!    （本文件的膝点 0.7 与 `util.rs` 的膝点 0.82），同名会让后来者接错线；
//! 4. **新增次正规数软件兜底**（来源依赖硬件 FTZ/DAZ）：见 [`DENORMAL_FLOOR`]。
//!
//! ## 来源回归测试全部保留
//!
//! - `stays_continuous_on_a_sine`：纯正弦过滤波器不得出现超过输入自身斜率的
//!   样本间跳变——这正是当年"每个渲染块咔哒一次"的化石；
//! - `is_transparent_below_the_knee`：**手写加窗 DFT**，断言 2–12 次谐波总能量
//!   比基频低 70 dB 以上。一个滤波器在未被驱动时必须透明。
//!
//! ## 非有限输入样本（本轮补齐）
//!
//! 四级积分器状态是**递归**的（`state = 2y − state`，且解出的 `u` 里含
//! `feedback · state_sum`）。一个 `NaN`／`±∞` 样本会在同一样本就写进四级状态，
//! 此后每一级都把它原样留下（`NaN · 系数 = NaN`），`flush_denormals` 也清不掉，
//! **输入恢复干净也回不来** —— [`LadderFilter::reset`] 是唯一出路。
//! 实时路径上无法报错，只能在入口回落。
//!
//! ⇒ [`LadderFilter::process`] 在计算之前过 `math::finite_or_zero`：`NaN` 与 `±∞`
//! 归 `0.0`，**有限样本逐位不变**（既有音色与两条来源回归判据一个比特都不改）。
//! [`LadderFilter::process_block`] 逐样本转调 `process`，因此自动继承这条守卫。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入：`3e38 · drive` 仍可能溢出成 `∞`。那条归
//! 调用方的电平口径管（同 `math::finite_or_zero` 的文档）。

use crate::math::{finite_or_zero, pade_tanh, sanitise_sample_rate};

/// 输出级的软饱和拐点：以下**完全线性**，以上平滑饱和，最多到 1.0。
///
/// 用朴素的 `tanh(v)` 会给一切染色：在再普通不过的 0.5 电平上它已经压缩 3%，
/// 频谱门禁会把它量成 −46 dB 的谐波。滤波器在"真的被驱动"之前必须是透明的，
/// 因此饱和只从拐点之上开始——而那仍然足以约束自激振荡，因为自激远在拐点之上。
const SOFT_KNEE: f32 = 0.7;

/// 输出微调。被替换掉的 vendored 梯形滤波器通带增益是 0.5，所有既有音色都是
/// 对着"比单位增益低 6.02 dB"的滤波器配平的；保持一致才能让既有音色、余量与
/// 音频门禁停在原处。夜半版本取 1.0（不额外衰减）。
const PASSBAND_TRIM: f32 = 1.0;

/// 状态冲刷门限 [ARCH-RT-003]。
///
/// 规范要求音频线程显式开启硬件 FTZ/DAZ 来根除次正规数导致的 100× 周期惩罚，
/// 但那依赖目标 CPU 与平台。这里加一道**纯软件、跨平台确定**的兜底：
/// 四级状态全部落到 `1e-30` 以下时直接归零。`1e-30` 远高于 f32 的次正规数
/// 下界（≈1.18e-38），因此输出永远不会进入次正规区间；而 `-600 dB` 的尾部
/// 在听感上早就是静音。
pub const DENORMAL_FLOOR: f32 = 1e-30;

/// 有界输出级：`|x| <= SOFT_KNEE` 时完全线性，之上平滑饱和并收敛到 ±1。
#[inline]
fn bounded_saturate(x: f32) -> f32 {
    let magnitude = x.abs();
    if magnitude <= SOFT_KNEE {
        return x;
    }
    let over = (magnitude - SOFT_KNEE) / (1.0 - SOFT_KNEE);
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    sign * (SOFT_KNEE + (1.0 - SOFT_KNEE) * pade_tanh(over))
}

/// TPT/零延迟反馈四极梯形低通。
#[derive(Clone, Copy, Debug)]
pub struct LadderFilter {
    /// 单极点系数，已折算成 `g / (1 + g)`。
    coeff: f32,
    /// 反馈量：0（无谐振）到接近 4（自激）。
    feedback: f32,
    /// 进入饱和级的输入增益。
    drive: f32,
    /// 四级积分器状态。
    state: [f32; 4],
}

impl LadderFilter {
    /// 构造：系数 0.1、无谐振、单位驱动、状态归零。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            coeff: 0.1,
            feedback: 0.0,
            drive: 1.0,
            state: [0.0; 4],
        }
    }

    /// 清空状态（换音符/复位）。
    pub fn reset(&mut self) {
        self.state = [0.0; 4];
    }

    /// 配置：`cutoff_hz` 单位 Hz，`resonance` / `drive` 都是 0..1 的旋钮值。
    pub fn configure(&mut self, sample_rate: f32, cutoff_hz: f32, resonance: f32, drive: f32) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        // 把截止频率留在 Nyquist 之下：`tan()` 在 sr/2 处爆炸，
        // 而 0.45 这个天花板是引擎其余部分一直在用的值。
        let cutoff_hz = if cutoff_hz.is_finite() {
            cutoff_hz.clamp(20.0, sample_rate * 0.45)
        } else {
            20.0
        };
        let w = (core::f32::consts::PI * cutoff_hz / sample_rate).tan();
        self.coeff = (w / (1.0 + w)).clamp(0.0, 0.999_99);
        self.feedback = if resonance.is_finite() {
            resonance.clamp(0.0, 1.0) * 3.9
        } else {
            0.0
        };
        // 旋钮底部是单位增益，最大到 2×：足够听得出"脏"，又不会把正弦变成
        // 失真器（音频门禁要求满驱动下 THD 仍低于 6%）。
        self.drive = 1.0
            + if drive.is_finite() {
                drive.clamp(0.0, 1.0)
            } else {
                0.0
            } * 0.8;
    }

    /// 当前单极点系数（测试/调试可读）。
    #[must_use]
    pub const fn coefficient(&self) -> f32 {
        self.coeff
    }

    /// 当前反馈量（测试/调试可读）。
    #[must_use]
    pub const fn feedback(&self) -> f32 {
        self.feedback
    }

    /// 当前的输入驱动增益（进入饱和级之前）。**旋钮底部是 `1.0`（单位增益）**，
    /// 上限见 [`Self::configure`] 的文档。与 [`Self::coefficient`] /
    /// [`Self::feedback`] 同一条纪律：只读、无副作用、供判据与调试使用。
    #[must_use]
    pub const fn drive(&self) -> f32 {
        self.drive
    }

    /// 处理一个样本。
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let g = self.coeff;
        // 入口守卫：非有限样本会在同一样本写进四级递归状态并被永久留下
        // （`NaN · 系数 = NaN`），`flush_denormals` 清不掉 ⇒ 就地回落成 `0.0`。
        // 有限样本逐位不变。
        let input = finite_or_zero(input);
        // 单位驱动是透明的：输入饱和级只在旋钮拧起来之后才塑形，
        // 因此通带电平保持不变。
        let x = if self.drive > 1.0001 {
            pade_tanh(input * self.drive) / self.drive
        } else {
            input
        };
        // 零延迟反馈：解反馈环，而不是用上一个样本反馈。没有这一步，
        // 四极环在到达满谐振之前就会失稳；有了它，`feedback` 接近 4 时
        // 能干净地自激。每级是 `y = g*x + (1 - g)*z`。
        let g2 = g * g;
        let g3 = g2 * g;
        let g4 = g3 * g;
        let bases = [
            (1.0 - g) * self.state[0],
            (1.0 - g) * self.state[1],
            (1.0 - g) * self.state[2],
            (1.0 - g) * self.state[3],
        ];
        let state_sum = g3 * bases[0] + g2 * bases[1] + g * bases[2] + bases[3];
        let u = (x - self.feedback * state_sum) / (1.0 + self.feedback * g4);
        let mut v = u;
        for (state, base) in self.state.iter_mut().zip(bases.iter()) {
            let y = g * v + base;
            // z_new = y + (y - z_old)
            *state = 2.0 * y - *state;
            v = y;
        }
        self.flush_denormals();
        // 反馈路径里的饱和是自激保持有界的原因：没有它，环路会一直增长，
        // 直到被主限制器削平。
        bounded_saturate(v) * PASSBAND_TRIM
    }

    /// 渲染一个块（引擎使用的块 ABI，原地）。
    pub fn process_block(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample);
        }
    }

    /// 四级状态全部进入次正规数区间之前主动归零 [ARCH-RT-003]。
    #[inline]
    fn flush_denormals(&mut self) {
        if self.state.iter().all(|state| state.abs() < DENORMAL_FLOOR) {
            self.state = [0.0; 4];
        }
    }
}

impl Default for LadderFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判据（新写，可红）**：`reset` 之后的梯形滤波器与**全新构造的同参数实例**
    /// 在同样输入下逐位一致（四级状态整组归零）。
    ///
    /// 量什么：256 个输出样本（`f32` 位型）。
    ///
    /// `reset` 的唯一动作是把四级状态整组清零。注入实测：把它改成只清第 0 级
    /// ⇒ 其余三级带旧状态 ⇒ 输出与全新实例不同，而**既有全量判据全绿**
    /// ⇒ 这条复位契约此前没有被守住。
    #[test]
    fn reset_reproduces_a_freshly_built_filter_bit_for_bit() {
        let build = || {
            let mut filter = LadderFilter::new();
            filter.configure(48_000.0, 1_200.0, 0.7, 0.5);
            filter
        };
        let drive = |filter: &mut LadderFilter| -> Vec<f32> {
            (0..256)
                .map(|index| {
                    let phase = core::f32::consts::TAU * 300.0 * index as f32 / 48_000.0;
                    filter.process(phase.sin())
                })
                .collect()
        };
        let mut used = build();
        let _ = drive(&mut used);
        used.reset();
        let after = drive(&mut used);
        let fresh = drive(&mut build());
        assert_eq!(after, fresh, "reset 之后与全新实例逐位不一致");
    }

    fn render(freq: f32, res: f32, drive: f32, seconds: f32) -> Vec<f32> {
        let sr = 48_000.0;
        let mut filter = LadderFilter::new();
        filter.configure(sr, freq, res, drive);
        let n = (seconds * sr) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let x = (core::f32::consts::TAU * 440.0 * i as f32 / sr).sin() * 0.5;
            out.push(filter.process(x));
        }
        out
    }

    /// 纯正弦必须平滑输出：样本间跳变不得超过输入自身斜率允许的量。
    /// 这就是当年那个 bug 的回归——旧滤波器每个渲染块咔哒一次。
    #[test]
    fn stays_continuous_on_a_sine() {
        let sr = 48_000.0;
        for (freq, res, drive) in [(500.0, 0.1, 0.0), (4_000.0, 0.5, 0.3), (18_000.0, 0.0, 0.0)] {
            let out = render(freq, res, drive, 1.0);
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let mut worst = 0.0f32;
            for pair in out.windows(2) {
                worst = worst.max((pair[1] - pair[0]).abs());
            }
            // 这个峰值下的 440 Hz 正弦每样本最大步进是 2π·440/sr·peak；
            // 一个正常的低通不可能在此基础上多加多少。
            let ideal = core::f32::consts::TAU * 440.0 / sr * peak;
            assert!(
                worst < ideal * 2.5,
                "cutoff {freq} res {res} drive {drive}: step {worst} vs ideal {ideal}"
            );
            assert!(out.iter().all(|v| v.is_finite()));
        }
    }

    /// 瞬态衰减后的稳态正弦幅度（这里峰值就够了：下面的测试看响应，
    /// 引擎级门禁才看失真）。
    fn amplitude(freq: f32, res: f32, tone: f32) -> f32 {
        let sr = 48_000.0;
        let mut filter = LadderFilter::new();
        filter.configure(sr, freq, res, 0.0);
        let mut peak = 0.0f32;
        let total = 48_000;
        for i in 0..total {
            let x = (core::f32::consts::TAU * tone * i as f32 / sr).sin();
            let y = filter.process(x);
            if i > total / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn low_pass_attenuates_above_the_cutoff() {
        let pass = amplitude(2_000.0, 0.0, 200.0);
        let stop = amplitude(2_000.0, 0.0, 8_000.0);
        assert!(pass > 0.8, "passband too quiet: {pass}");
        assert!(stop < pass * 0.1, "not a low-pass: {stop} vs {pass}");
    }

    #[test]
    fn resonance_lifts_the_cutoff() {
        let flat = amplitude(1_000.0, 0.0, 1_000.0);
        let resonant = amplitude(1_000.0, 0.9, 1_000.0);
        assert!(resonant > flat * 1.5, "no resonance: {resonant} vs {flat}");
    }

    #[test]
    fn full_resonance_stays_bounded() {
        let out = render(800.0, 1.0, 1.0, 2.0);
        assert!(out.iter().all(|v| v.is_finite()));
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 12.0, "ladder ran away: {peak}");
    }

    /// 滤波器必须在常规电平上透明：0.5 幅度的正弦过全开低通，输出里
    /// 不应有任何新增内容。这是 `stays_continuous_on_a_sine` 的频域孪生。
    ///
    /// **手写加窗 DFT**（刻意不复用 `math::fft`）：门禁要独立于被测代码。
    #[test]
    fn is_transparent_below_the_knee() {
        let sr = 48_000.0;
        let out = render(18_000.0, 0.05, 0.0, 1.0);
        let n = out.len();
        let mag = |f: f32| {
            let w = core::f32::consts::TAU * f / sr;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, v) in out.iter().enumerate().skip(n / 2) {
                let win = 0.5
                    - 0.5 * (core::f32::consts::TAU * (i - n / 2) as f32 / (n / 2) as f32).cos();
                re += (*v * win) as f64 * (w * i as f32).cos() as f64;
                im -= (*v * win) as f64 * (w * i as f32).sin() as f64;
            }
            (re * re + im * im).sqrt() / (n / 2) as f64
        };
        let fundamental = mag(440.0);
        let mut junk = 0.0f64;
        for k in 2..=12 {
            junk += mag(440.0 * k as f32).powi(2);
        }
        // 基频之上的一切至少低 70 dB。
        let ratio = 20.0 * (junk.sqrt() / fundamental).log10();
        assert!(ratio < -70.0, "filter colours a clean sine: {ratio:.1} dB");
    }

    /// **判据（新写，可红）**：尾部永不输出次正规数 [ARCH-RT-003]。
    ///
    /// 硬件 FTZ/DAZ 在 CI 上不可断言（取决于 CPU 与编译标志），因此这里断言
    /// 的是**软件兜底**：脉冲之后 20 万个静音样本里，输出要么恰好是 0，
    /// 要么是正常数（`>= f32::MIN_POSITIVE`）；绝不允许出现 `0 < |v| < MIN_POSITIVE`
    /// 的次正规数。
    ///
    /// 把 `flush_denormals` 的阈值改成 0.0（等于删掉这道兜底），
    /// 衰减尾部会陆续落进次正规数区间，本测试立即变红。
    #[test]
    fn never_emits_subnormal_tail_values() {
        let mut filter = LadderFilter::new();
        filter.configure(48_000.0, 18_000.0, 0.5, 0.0);
        // 先把状态激励满。
        for _ in 0..64 {
            filter.process(1.0);
        }
        let mut reached_exact_zero = false;
        for sample in 0..200_000 {
            let value = filter.process(0.0);
            assert!(value.is_finite(), "sample {sample}: not finite");
            assert!(
                value == 0.0 || value.abs() >= f32::MIN_POSITIVE,
                "sample {sample}: subnormal output {value:e}"
            );
            reached_exact_zero |= value == 0.0;
            assert!(filter.state.iter().all(|s| s.is_finite()));
        }
        assert!(reached_exact_zero, "tail never reached exact silence");
        assert_eq!(filter.state, [0.0; 4], "states were left unflushed");
    }

    #[test]
    fn degenerate_configuration_is_clamped() {
        let mut filter = LadderFilter::new();
        filter.configure(f32::NAN, f32::NAN, f32::NAN, f32::NAN);
        assert!(filter.coefficient().is_finite());
        assert!(filter.feedback().is_finite());
        // 截止频率必须留在 Nyquist 之下：极端参数下输出仍必须有界。
        filter.configure(48_000.0, 1.0e9, 1.0, 1.0);
        for i in 0..4096 {
            let x = (core::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin();
            assert!(filter.process(x).is_finite());
        }
        assert!(filter.coefficient() < 1.0);
    }

    /// **判据（新写，可红）**：截止频率的**天花板**（`0.45 · fs`）被逐位钉住。
    ///
    /// 量什么：`LadderFilter::coefficient()` 的 `to_bits()`（单位：`f32` 位型），
    /// 五个截止频率之间的相等关系。
    ///
    /// 为什么需要它：[`degenerate_configuration_is_clamped`] 只断言
    /// `coefficient() < 1.0` 且有限。实测（本票注入 A31）：把 `sample_rate * 0.45`
    /// 放宽成 `sample_rate * 1.45` 之后，全量 417 条判据仍全绿 —— 越界的截止频率
    /// 让 `π · fc / fs` 越过 `π/2`，`tan` 翻成负数，`w / (1 + w)` 落进负区间再被
    /// `clamp(0.0, 0.999_99)` 抬成 `0.0`，于是读数变成"滤波器完全打开"，而
    /// `0.0 < 1.0` 与"有限"两条断言都仍然成立。
    ///
    /// 判据：`0.45·fs` 与它之上的四个截止频率必须给出**同一个**系数位型；
    /// 并带非空证明（天花板读数必须**不同于**下界读数）。链路只有 `tan`、除法、
    /// 钳位 ⇒ 超越函数类 ⇒ 只在同一架构内比对位型，⛔ 不与字面常量比。
    #[test]
    fn the_cutoff_ceiling_is_pinned_at_forty_five_percent_of_the_sample_rate() {
        const SR: f32 = 48_000.0;
        let coefficient = |cutoff_hz: f32| {
            let mut filter = LadderFilter::new();
            filter.configure(SR, cutoff_hz, 0.0, 0.0);
            filter.coefficient().to_bits()
        };
        let ceiling = coefficient(SR * 0.45);
        for cutoff_hz in [SR * 0.5, SR * 0.75, SR, SR * 1.45, 1.0e9] {
            assert_eq!(
                coefficient(cutoff_hz),
                ceiling,
                "截止频率 {cutoff_hz} Hz 必须与 0.45·fs 的天花板给出同一个系数"
            );
        }
        // 非空证明：天花板读数**不是**下界读数（否则"同一个"这句话没有内容）。
        assert_ne!(
            ceiling,
            coefficient(1.0),
            "天花板读数与下界读数相同 ⇒ 判据测不出这个边界"
        );
        // 正对照：天花板确实在 (0, 1) 内，不是被钳到 0 或 1。
        // （正浮点数的位型序与数值序同向 ⇒ 可以直接比位型。）
        assert!(
            ceiling > 0.0f32.to_bits() && ceiling < 1.0f32.to_bits(),
            "天花板读数 {ceiling:#010x} 越界"
        );
    }

    /// **判据（新写，可红）**：驱动增益的范围（`1.0` 到 `1.8`）被钉住。
    ///
    /// 量什么：`LadderFilter::drive()` 的读数（单位：线性增益）。
    ///
    /// 为什么需要它：[`degenerate_configuration_is_clamped`] 用 `drive = 1.0`
    /// 只断言输出有限。实测（本票注入 S08）：把 `* 0.8` 改成 `* 0.4` 之后，
    /// 全量 417 条判据仍全绿 —— 驱动量被砍掉一半，而没有任何判据读它。
    /// `configure` 的注释写明"旋钮底部是单位增益，最大到 2×"（实现取
    /// `1.0 + 旋钮 · 0.8`），这条范围就是本判据钉的内容。
    ///
    /// 链路只有比较、钳位与乘加 ⇒ IEEE 精确类 ⇒ 处处硬断言位型。
    #[test]
    fn the_drive_range_is_pinned_from_unity_to_the_documented_maximum() {
        let mut filter = LadderFilter::new();
        // 构造期的默认值。
        assert_eq!(filter.drive().to_bits(), 1.0f32.to_bits());
        // 旋钮底部：单位增益。
        filter.configure(48_000.0, 1_000.0, 0.0, 0.0);
        assert_eq!(
            filter.drive().to_bits(),
            1.0f32.to_bits(),
            "底部必须是单位增益"
        );
        // 旋钮顶部：1.8×。
        filter.configure(48_000.0, 1_000.0, 0.0, 1.0);
        assert_eq!(filter.drive().to_bits(), 1.8f32.to_bits(), "顶部比例漂移了");
        // 越界与非有限：钳到两端。
        filter.configure(48_000.0, 1_000.0, 0.0, 1.0e9);
        assert_eq!(filter.drive().to_bits(), 1.8f32.to_bits());
        filter.configure(48_000.0, 1_000.0, 0.0, -3.0);
        assert_eq!(filter.drive().to_bits(), 1.0f32.to_bits());
        filter.configure(48_000.0, 1_000.0, 0.0, f32::NAN);
        assert_eq!(filter.drive().to_bits(), 1.0f32.to_bits());
        // 非空证明：两端必须真的不同 —— 比的是**两次观测到的读数**，
        // ⛔ 不是两个字面量（`assert_ne!(1.0f32.to_bits(), 1.8f32.to_bits())` 是
        // 编译期恒真、不涉及被测对象 ⇒ 按裁决 R69 属**假探针**，已替换）。
        filter.configure(48_000.0, 1_000.0, 0.0, 1.0);
        let top = filter.drive();
        filter.configure(48_000.0, 1_000.0, 0.0, 0.0);
        let bottom = filter.drive();
        assert_ne!(
            top.to_bits(),
            bottom.to_bits(),
            "驱动旋钮的两端必须给出不同的读数（{top} vs {bottom}）"
        );
    }

    #[test]
    fn process_block_matches_sample_by_sample() {
        let mut blockwise = LadderFilter::new();
        blockwise.configure(48_000.0, 1_200.0, 0.6, 0.4);
        let mut single = LadderFilter::new();
        single.configure(48_000.0, 1_200.0, 0.6, 0.4);
        let mut buffer: Vec<f32> = (0..128)
            .map(|i| (core::f32::consts::TAU * 300.0 * i as f32 / 48_000.0).sin())
            .collect();
        let expected: Vec<f32> = buffer.iter().map(|x| single.process(*x)).collect();
        blockwise.process_block(&mut buffer);
        assert_eq!(buffer, expected);
    }

    /// **判据（新写，可红）**：截止频率天花板是 `0.45 · fs` 这个**比例**本身。
    ///
    /// 量什么：`coefficient()` 的 `f32` 位型（三处）。
    ///
    /// 既有判据只断言"`0.45 · fs` 与更大的请求给出**同一个**位型" —— 把比例改成
    /// `0.40` 时所有越界请求仍被钳到同一个点 ⇒ 那条判据照样绿（注入实测 0.45→0.40
    /// **全绿**）。本判据补上另一半：天花板**之下**（`0.44 · fs`）必须仍然改变系数
    /// ⇒ 比例本身被钉住。三处比较都在同一台机器的同一次运行内 ⇒ 与 `tan` 的跨架构
    /// 差异无关（裁决 R24）。
    #[test]
    fn the_cutoff_ceiling_ratio_is_pinned_by_a_literal() {
        let mut filter = LadderFilter::new();
        filter.configure(48_000.0, 48_000.0 * 0.45, 0.0, 0.0);
        let at_ceiling = filter.coefficient();
        filter.configure(48_000.0, 1.0e9, 0.0, 0.0);
        assert_eq!(
            filter.coefficient().to_bits(),
            at_ceiling.to_bits(),
            "天花板之上的请求必须与 0.45·fs 给出同一位型"
        );
        filter.configure(48_000.0, 48_000.0 * 0.44, 0.0, 0.0);
        assert_ne!(
            filter.coefficient().to_bits(),
            at_ceiling.to_bits(),
            "0.44·fs 必须给出与 0.45·fs 不同的系数（否则天花板比例不是 0.45）"
        );
        assert!(
            at_ceiling > 0.0 && at_ceiling < 1.0,
            "天花板处的单极点系数必须落在 (0, 1) 内：{at_ceiling}"
        );
    }

    /// **判据（新写，可红）**：谐振满量程是字面量 `3.9`（旋钮 `1.0` 给出它）。
    ///
    /// 量什么：`feedback()` 的 `f32` 位型（满量程、半量程、越界三处）。
    ///
    /// 既有判据只测"谐振会抬起截止点附近的增益"这类**相对**性质，对满量程的绝对
    /// 刻度不敏感。注入实测：`resonance.clamp(0.0, 1.0) * 3.9` 改成 `* 3.0`
    /// ⇒ 既有全量判据**全绿** ⇒ 本判据变红。§`full_resonance_stays_bounded`
    /// 仍然绿，因为 3.0 也有界 —— 那正是这条判据要补的位置。
    #[test]
    fn the_full_resonance_feedback_is_pinned_by_a_literal() {
        let mut filter = LadderFilter::new();
        filter.configure(48_000.0, 1_000.0, 1.0, 0.0);
        assert_eq!(filter.feedback().to_bits(), 3.9f32.to_bits());
        filter.configure(48_000.0, 1_000.0, 0.5, 0.0);
        assert_eq!(filter.feedback().to_bits(), 1.95f32.to_bits());
        for over in [2.0f32, 1.0e9, f32::NAN, f32::INFINITY] {
            filter.configure(48_000.0, 1_000.0, over, 0.0);
            assert!(
                filter.feedback() <= 3.9,
                "谐振旋钮 {over} 必须钳到满量程（实测 {}）",
                filter.feedback()
            );
        }
    }

    /// **判据（新写，可红）**：饱和拐点由字面量 `0.7` 从**两侧**钉住 —— 拐点以下
    /// （含拐点本身）逐位恒等，拐点以上立刻开始塑形。
    ///
    /// 量什么：`bounded_saturate` 在 5 个拐点以下取值上的 `f32` 位型，以及 5 个拐点
    /// 以上取值的幅度是否**严格变小**。
    ///
    /// 拐点常量只在"拐点之上"生效，而既有判据 `is_transparent_below_the_knee` 用的
    /// 幅度全在拐点之下，且只测谐波比例（相对量）⇒ 拐点改成 `0.5` 或 `0.8` 都
    /// **全绿**（注入实测两次都绿）。本判据两侧同时钉：改小 ⇒ 拐点以下的逐位恒等
    /// 失败；改大 ⇒ 拐点以上的"严格变小"失败。`pade_tanh` 是代数式（无超越函数），
    /// 且全部断言在同一台机器上比较同一函数的输出 ⇒ 跨架构无关。
    #[test]
    fn the_saturation_knee_is_pinned_from_both_sides() {
        for sample in [0.0f32, 0.1, 0.5, 0.7, -0.7] {
            assert_eq!(
                bounded_saturate(sample).to_bits(),
                sample.to_bits(),
                "拐点以下（含拐点本身）必须逐位恒等：{sample}"
            );
        }
        for sample in [0.71f32, 0.8, 1.0, -0.71, -1.0] {
            let out = bounded_saturate(sample);
            assert!(
                out.abs() < sample.abs(),
                "拐点以上 {sample} 必须被塑形，实际 {out}"
            );
            assert!(out.is_finite() && out.abs() < 1.0, "饱和输出越界：{out}");
        }
    }
}
