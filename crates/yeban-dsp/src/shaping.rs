//! bitcrusher / shaping EQ / transient shaper。[ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/fx_shaping.rs`。
//!
//! 三者都是普通 Rust：每个效果节点一个定长状态块，无分配、无 C 桥接口。
//! 每个节点持有自己的状态——这正是让路由图把一台 crusher 与一台 EQ 放在两个
//! 不同位置而不共享滤波器的原因。
//!
//! # bitcrusher
//!
//! `bits`（4..16）的量化器后面跟一个 `down`（1..64）的采样保持——采样率被
//! `down` 整除，因此频率为 `f` 的正弦会在 `k·(sr/down) ± f` 处产生镜像，
//! 而高于抽取后 Nyquist 的一切都会折回。抗混叠控制 `aa` 在分频器**之前**混入
//! 两级级联单极点低通（转折在抽取后的 Nyquist），**之后**再混入两级来给阶梯
//! 插值。`aa = 0` 时 crusher 就是原始、混叠的那一个，这正是"镜像随 aa 上升而
//! 下降"这两条断言有意义的前提。
//!
//! # shaping EQ
//!
//! 三级串联的 RBJ biquad：低架、可扫的中频峰值、高架。系数按块计算，用 `exp2`
//! 而不是 `pow`（四个 EQ 旋钮不值得链入 libm 的 `pow`），并用 `a0` 归一化。
//! 增益为 0 dB 的频段在数学上是恒等变换，因此平坦 EQ 是真正的旁通。
//!
//! # transient shaper
//!
//! 对整流信号先用快包络跟随器、再用一个更慢的跟随器去追它：上升时快的领先，
//! 下降时快的滞后，而稳态音符上两者相等、差值恰好为零。`attack_amt` 缩放差值
//! 的领先半边，`sustain_amt` 缩放滞后半边，都以分贝计。两者都为 0 时是数学恒等
//! ——增益恰好是 1、输出逐位等于输入。
//!
//! 整流器用两个远高于任何音符载频的极点平滑，因此持续音的纹波在取瞬态之前就
//! 没了；没有这一步，shaper 会以两倍于音符自身的频率对它做增益调制，门禁会把它
//! 读成谐波失真。两个跟随器随后只看到包络，而且慢的那个追的是快的那个而不是原始
//! 信号，因此持续音会在几十毫秒内回到单位增益，而不是整段音符都保持抬升。
//!
//! 增益是 `2^(dB/6)`（即恰好 `dB` 分贝），硬钳在 `[0, 8]` 内，因此任何控制量组合
//! 都不可能让信号反相或发散。
//!
//! ## 与来源的差异
//!
//! 1. `process` 去掉冗余的 `frames` 参数（改取四个缓冲区的较短长度）：
//!    来源允许 `frames` 大于缓冲区长度，那是实时路径上的越界 panic；
//! 2. `gain_to_a` 收敛到 [`crate::math::db_to_gain`] 的平方根
//!    （`10^(db/40) == sqrt(10^(db/20))`），避免第二份 dB 换算常量漂移；
//! 3. 三个效果类型与三个参数结构都补上 `Default`（工作区 `clippy::all` 要求
//!    `new()` 与 `Default` 成对），并给全部公开字段补上规范 ID 标注的文档；
//! 4. 采样率一律经过 [`crate::math::sanitise_sample_rate`] 钳制。
//!
//! ## 测试口径（重要）
//!
//! 来源的测试模块分两半：一半直接测 DSP 结构，另一半通过 `crate::engine::Engine`
//! 与 `crate::params` 渲染（`engine_dry_path_is_bit_exact_when_the_mix_is_zero`、
//! `engine_transient_*`、`engine_crush_quantises_and_mirrors`、
//! `engine_eq_bands_match_their_gain`、`abrupt_changes_stay_bounded_and_finite`）。
//! 后者是**引擎层**测试，本 crate 没有（也不该有）`Engine`，因此不移植；
//! 其中"参数被猛砸时仍然有界且有限"这条被重新落到 DSP 层实现为
//! `abrupt_parameter_slams_stay_bounded`，其它四条登记在
//! `docs/ledger/dsp-core-provenance.md` 的待办里（归属 `yeban-engine`）。
//!
//! ## 非有限输入样本（本轮补齐）
//!
//! 三个效果节点都持有**递归**状态：crusher 的两级抗混叠单极点与两级插值单极点、
//! EQ 的三个转置直接 II 型 biquad 的 `v1`/`v2`、transient shaper 的整流与两个
//! 包络跟随器。任何一处写进 `NaN`／`±∞`，`s += c · (x − s)` 的每一次迭代都把它
//! 原样留下（`NaN · c = NaN`、`∞ · 0.5 = ∞`），**输入恢复干净也回不来**
//! —— 三个类型的 `reset` 是唯一出路。实时路径上无法报错，只能在入口回落。
//!
//! ⇒ 三个 `process` 在读取输入样本处过 `math::finite_or_zero`：`NaN` 与 `±∞`
//! 归 `0.0`，**有限样本逐位不变**（既有音色、既有频谱判据与
//! `abrupt_parameter_slams_stay_bounded` 一个比特都不改）。
//!
//! ⚠ transient shaper 的症状**不是**非有限输出：`exp2`（本 crate 的版本）对非有限
//! 输入返回 `0`，因此中毒后增益恒为 `0`、湿信号恒为 `0` —— 输出**有限但永久错**。
//! 这正是判据必须写成"与'该样本换成 `0.0`'的对照运行逐位相同"、而不能只写
//! "输出有限"的原因。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入（`3e38 · 8` 仍可能溢出成 `∞`）；那条归
//! 调用方的电平口径管（同 `math::finite_or_zero` 的文档）。

use core::f32::consts::TAU;

use crate::math::{db_to_gain, exp2, finite_or_zero, sanitise_sample_rate};

/// RBJ cookbook 要的幅度 `A = 10^(db/40)`。
///
/// `10^(db/40) == sqrt(10^(db/20))`，因此直接复用 [`db_to_gain`]，
/// 全 crate 只有一处 dB 换算常量。
#[inline]
fn gain_to_a(db: f32) -> f32 {
    db_to_gain(db).sqrt()
}

/// 段首列表里 crusher 提供的最大采样保持除数。
pub const MAX_DIVISOR: f32 = 64.0;

/// transient shaper 的增益硬上限（任何控制量组合都不会超过它）。
pub const MAX_TRANSIENT_GAIN: f32 = 8.0;

/// bitcrusher 的控制量（一个块的有效值）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrushParams {
    /// 量化位数，钳制到 4..16。
    pub bits: f32,
    /// 采样保持除数，钳制到 1..[`MAX_DIVISOR`]。
    pub down: f32,
    /// 抗混叠量，0 = 原始（混叠），1 = 全开。
    pub aa: f32,
}

impl Default for CrushParams {
    fn default() -> Self {
        Self {
            bits: 16.0,
            down: 1.0,
            aa: 0.0,
        }
    }
}

/// shaping EQ 的控制量（一个块的有效值），全部以 dB / Hz 计。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqParams {
    /// 低架增益（dB），钳制到 ±18。
    pub low_gain: f32,
    /// 低架转折频率（Hz），钳制到 20..1000。
    pub low_freq: f32,
    /// 中频峰值增益（dB），钳制到 ±18。
    pub mid_gain: f32,
    /// 中频中心频率（Hz），钳制到 200..8000。
    pub mid_freq: f32,
    /// 中频品质因数，钳制到 0.3..6。
    pub mid_q: f32,
    /// 高架增益（dB），钳制到 ±18。
    pub high_gain: f32,
    /// 高架转折频率（Hz），钳制到 1000..16000。
    pub high_freq: f32,
}

impl Default for EqParams {
    /// 平坦 EQ（所有增益 0 dB）：数学恒等变换。
    fn default() -> Self {
        Self {
            low_gain: 0.0,
            low_freq: 200.0,
            mid_gain: 0.0,
            mid_freq: 1000.0,
            mid_q: 0.9,
            high_gain: 0.0,
            high_freq: 4000.0,
        }
    }
}

/// 中置量化（mid-tread）：全量程分 `2^bits` 级，因此满量程本身落在网格上，
/// 钳制永远不会停在两个网格点之间。16 bit 的步长是 3.05e-5，4 bit 是 0.125。
#[inline]
fn quantise(x: f32, step: f32, limit: f32) -> f32 {
    (x / step).round().clamp(-limit, limit) * step
}

/// bitcrusher 状态，每个效果节点一份。
#[derive(Clone, Copy, Debug)]
pub struct BitCrusher {
    /// 两级级联抗混叠单极点状态，每声道一组。
    pre: [[f32; 2]; 2],
    /// 采样保持值，每声道一个。
    hold: [f32; 2],
    /// 两级级联插值单极点状态，每声道一组。
    post: [[f32; 2]; 2],
    /// 当前保持样本内的位置，单位是输入样本。
    phase: f32,
}

impl BitCrusher {
    /// 构造：状态归零；`phase` 从一个大于任何除数的值开始，
    /// 因此第一个样本就会被捕获，而不是先输出 `down` 个样本的静音。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pre: [[0.0; 2]; 2],
            hold: [0.0; 2],
            post: [[0.0; 2]; 2],
            phase: MAX_DIVISOR,
        }
    }

    /// 复位到初始状态。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// 渲染湿（被碾过）信号。干/湿交叉淡化是调用方的事，
    /// 这正是让 `mix = 0` 成为逐位旁通的原因。
    ///
    /// 各缓冲区取较短长度，不做任何分配 [ARCH-RT-001]。
    #[inline(never)]
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        params: CrushParams,
        sample_rate: f32,
    ) {
        let frames = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());
        if frames == 0 {
            return;
        }
        let sample_rate = sanitise_sample_rate(sample_rate);
        let bits = if params.bits.is_finite() {
            params.bits.clamp(4.0, 16.0)
        } else {
            16.0
        };
        let down = if params.down.is_finite() {
            params.down.clamp(1.0, MAX_DIVISOR)
        } else {
            1.0
        };
        let aa = if params.aa.is_finite() {
            params.aa.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // `exp2` 是一条指令；对一个只有 4..16 的控制范围来说，
        // `powf` 会白白拉进一个 libm 例程。
        let levels = exp2(bits);
        let step = 2.0 / levels;
        let limit = levels * 0.5;
        // 抗混叠转折在抽取后的 Nyquist；插值转折略低一点，
        // 那是单极点给不出的额外阻带衰减。
        let decimated = sample_rate / down;
        let pre_coeff = (1.0 - (-TAU * 0.45 * decimated / sample_rate).exp()).clamp(0.0, 1.0);
        let post_coeff = (1.0 - (-TAU * 0.30 * decimated / sample_rate).exp()).clamp(0.0, 1.0);
        for i in 0..frames {
            // 入口守卫见模块文档"非有限输入样本"一节。有限样本逐位不变。
            let raw = [finite_or_zero(in_l[i]), finite_or_zero(in_r[i])];
            self.phase += 1.0;
            let capture = self.phase >= down;
            if capture {
                self.phase -= down;
            }
            let mut wet = [0.0f32; 2];
            for (channel, slot) in wet.iter_mut().enumerate() {
                let x = raw[channel];
                self.pre[channel][0] += pre_coeff * (x - self.pre[channel][0]);
                self.pre[channel][1] += pre_coeff * (self.pre[channel][0] - self.pre[channel][1]);
                // `aa = 0` 保留原始输入，因此只有量化器与分频器在塑造信号。
                let source = x + aa * (self.pre[channel][1] - x);
                if capture {
                    self.hold[channel] = quantise(source, step, limit);
                }
                self.post[channel][0] += post_coeff * (self.hold[channel] - self.post[channel][0]);
                self.post[channel][1] +=
                    post_coeff * (self.post[channel][0] - self.post[channel][1]);
                *slot = self.hold[channel] + aa * (self.post[channel][1] - self.hold[channel]);
            }
            out_l[i] = wet[0];
            out_r[i] = wet[1];
        }
    }
}

impl Default for BitCrusher {
    fn default() -> Self {
        Self::new()
    }
}

/// 一节 RBJ biquad，转置直接 II 型，按 `a0` 归一化。
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    /// 每声道的 `[v1, v2]`。
    v: [[f32; 2]; 2],
}

impl Biquad {
    const fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            v: [[0.0; 2]; 2],
        }
    }

    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        let inv = 1.0 / a0;
        self.b0 = b0 * inv;
        self.b1 = b1 * inv;
        self.b2 = b2 * inv;
        self.a1 = a1 * inv;
        self.a2 = a2 * inv;
    }

    /// RBJ 低架，斜率 `S = 1`（12 dB/octave 渐近线）。
    fn low_shelf(&mut self, sample_rate: f32, freq: f32, gain_db: f32) {
        let a = gain_to_a(gain_db);
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin * 0.5 * core::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha),
            (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha,
        );
    }

    /// RBJ 高架，斜率 `S = 1`。
    fn high_shelf(&mut self, sample_rate: f32, freq: f32, gain_db: f32) {
        let a = gain_to_a(gain_db);
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin * 0.5 * core::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha),
            (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha,
        );
    }

    /// RBJ 峰值均衡，中心 `freq`、带宽 `q`。
    fn peaking(&mut self, sample_rate: f32, freq: f32, q: f32, gain_db: f32) {
        let a = gain_to_a(gain_db);
        let w0 = TAU * (freq / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(0.05));
        self.set(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        );
    }

    #[inline(never)]
    fn process_in_place(&mut self, left: &mut [f32], right: &mut [f32], frames: usize) {
        for i in 0..frames {
            for (channel, x) in [left[i], right[i]].into_iter().enumerate() {
                let v1 = self.v[channel][0];
                let v2 = self.v[channel][1];
                let v = x - self.a1 * v1 - self.a2 * v2;
                let y = self.b0 * v + self.b1 * v1 + self.b2 * v2;
                self.v[channel][0] = v;
                self.v[channel][1] = v1;
                if channel == 0 {
                    left[i] = y;
                } else {
                    right[i] = y;
                }
            }
        }
    }
}

/// shaping EQ 状态，每个效果节点一份。
#[derive(Clone, Copy, Debug)]
pub struct ShapingEq {
    low: Biquad,
    mid: Biquad,
    high: Biquad,
}

impl ShapingEq {
    /// 构造（三级都是恒等变换）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            low: Biquad::identity(),
            mid: Biquad::identity(),
            high: Biquad::identity(),
        }
    }

    /// 复位到恒等状态。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// 渲染湿（被均衡）信号到 `out_*`；干/湿交叉淡化是调用方的事。
    #[inline(never)]
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        params: EqParams,
        sample_rate: f32,
    ) {
        let frames = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());
        if frames == 0 {
            return;
        }
        let sample_rate = sanitise_sample_rate(sample_rate);
        // 逐块重算系数，是让四个扫频旋钮"无需系数平滑方案也能听出来"的原因：
        // 引擎已经在平滑参数本身，因此系数也就跟着平滑移动。
        self.low.low_shelf(
            sample_rate,
            clamp_or(params.low_freq, 20.0, 1000.0, 200.0),
            clamp_or(params.low_gain, -18.0, 18.0, 0.0),
        );
        self.mid.peaking(
            sample_rate,
            clamp_or(params.mid_freq, 200.0, 8000.0, 1000.0),
            clamp_or(params.mid_q, 0.3, 6.0, 0.9),
            clamp_or(params.mid_gain, -18.0, 18.0, 0.0),
        );
        self.high.high_shelf(
            sample_rate,
            clamp_or(params.high_freq, 1000.0, 16000.0, 4000.0),
            clamp_or(params.high_gain, -18.0, 18.0, 0.0),
        );
        // 入口守卫见模块文档"非有限输入样本"一节：三级 biquad 的 `v1`/`v2` 是递归
        // 状态，非有限样本必须在写进去之前归零。有限样本逐位不变（等价于既有的
        // 整段拷贝 ⇒ 平坦 EQ 仍然是逐位旁通）。
        for (i, (l, r)) in out_l[..frames]
            .iter_mut()
            .zip(out_r[..frames].iter_mut())
            .enumerate()
        {
            *l = finite_or_zero(in_l[i]);
            *r = finite_or_zero(in_r[i]);
        }
        self.low.process_in_place(out_l, out_r, frames);
        self.mid.process_in_place(out_l, out_r, frames);
        self.high.process_in_place(out_l, out_r, frames);
    }
}

impl Default for ShapingEq {
    fn default() -> Self {
        Self::new()
    }
}

/// 非有限值回落到 `fallback`，否则钳制到 `[min, max]`。
#[inline]
fn clamp_or(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// transient shaper 的控制量（一个块的有效值）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransientParams {
    /// 上升瞬态被抬升（`+`）或压下去（`-`）的强度，−1..1。
    pub attack_amt: f32,
    /// 下降包络的同一件事，−1..1。
    pub sustain_amt: f32,
}

impl Default for TransientParams {
    fn default() -> Self {
        Self {
            attack_amt: 0.0,
            sustain_amt: 0.0,
        }
    }
}

/// 整流平滑的转折频率（Hz）。高于任何音符的载频（最低 65 Hz，其整流纹波在
/// 130 Hz），因此持续音的纹波在取瞬态之前就没了，shaper 不会变成持续音的失真器。
const RECT_HZ: f32 = 200.0;

/// 快跟随器的转折：它在几毫秒内跟上包络。
const FAST_HZ: f32 = 60.0;

/// 瞬态是"快包络与**它自己**的慢平均之差"。持续音会在几十毫秒内落到零——
/// 一个要一秒才追上来的慢参考会让音符的第一秒被永久抬升，那不是"attack"的意思。
const SLOW_HZ: f32 = 4.0;

/// `attack = 1` 时满量程起音的分贝增益：`attack = 0.5` 因此把起音移动约 3 dB，
/// `attack = -1` 是它的镜像。
const ATTACK_DB: f32 = 8.5;

/// 下降包络的同一件事。探测器在一次 release 上的偏移只有它在一个 onset 上的
/// 约四分之一（快跟随器要沿着慢的那个的尾巴往下走），因此 release 需要大得多的
/// 每单位分贝数，才能让 `sustain = ±0.5` 落进与 `attack = ±0.5` 相同的 ±3 dB 窗口。
/// 两个分支都被 [`MAX_TRANSIENT_GAIN`] 约束，因此最坏情况仍是有限增益。
const SUSTAIN_DB: f32 = 22.0;

/// `log2(10) / 20`：把分贝换算成以 2 为底的幂指数。
const DB_TO_LOG2: f32 = 0.166_096_2;

/// transient shaper 状态，每个效果节点一份。
#[derive(Clone, Copy, Debug)]
pub struct TransientShaper {
    /// 两级整流平滑，每声道一组。
    rect: [[f32; 2]; 2],
    /// 快包络跟随器，每声道一个。
    fast: [f32; 2],
    /// 慢包络跟随器，每声道一个。
    slow: [f32; 2],
}

impl TransientShaper {
    /// 构造（状态归零）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rect: [[0.0; 2]; 2],
            fast: [0.0; 2],
            slow: [0.0; 2],
        }
    }

    /// 复位到初始状态。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// 渲染湿（被塑形）信号到 `out_*`；干/湿交叉淡化是调用方的事。
    #[inline(never)]
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        params: TransientParams,
        sample_rate: f32,
    ) {
        let frames = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());
        if frames == 0 {
            return;
        }
        let sample_rate = sanitise_sample_rate(sample_rate);
        let rect_coeff = one_pole(RECT_HZ, sample_rate);
        let fast_coeff = one_pole(FAST_HZ, sample_rate);
        let slow_coeff = one_pole(SLOW_HZ, sample_rate);
        let attack = clamp_or(params.attack_amt, -1.0, 1.0, 0.0);
        let sustain = clamp_or(params.sustain_amt, -1.0, 1.0, 0.0);
        // 两个量都为 0 是真正的恒等：下面的增益恰好是 `exp2(0) = 1`，
        // 输出逐样本等于输入。
        let neutral = attack == 0.0 && sustain == 0.0;
        for i in 0..frames {
            // 入口守卫见模块文档"非有限输入样本"一节：整流与两个包络跟随器都是
            // 递归状态。有限样本逐位不变。
            let raw = [finite_or_zero(in_l[i]), finite_or_zero(in_r[i])];
            let mut wet = [0.0f32; 2];
            for (channel, slot) in wet.iter_mut().enumerate() {
                // 先整流再平滑：持续音的载频纹波必须在取瞬态之前消失，
                // 否则持续音会被以两倍于自身的频率做增益调制。
                self.rect[channel][0] += rect_coeff * (raw[channel].abs() - self.rect[channel][0]);
                self.rect[channel][1] +=
                    rect_coeff * (self.rect[channel][0] - self.rect[channel][1]);
                let source = self.rect[channel][1];
                self.fast[channel] += fast_coeff * (source - self.fast[channel]);
                // 慢跟随器追的是快的那个，而不是整流信号：包络稳定后两者相等，
                // 因此持续音没有任何东西可移。
                self.slow[channel] += slow_coeff * (self.fast[channel] - self.slow[channel]);
                let fast = self.fast[channel];
                let slow = self.slow[channel];
                // 差值用更响的那个跟随器归一化（因此 shaper 与电平无关），
                // 并由构造保证落在 ±1 内。下限防止静音时除以零：
                // 没有东西可塑形时增益恰好是 1。
                let scale = 1.0 / fast.max(slow).max(1e-6);
                let db = if neutral {
                    0.0
                } else if fast > slow {
                    attack * (fast - slow) * scale * ATTACK_DB
                } else {
                    sustain * (fast - slow) * scale * SUSTAIN_DB
                };
                let gain = exp2(db * DB_TO_LOG2).clamp(0.0, MAX_TRANSIENT_GAIN);
                *slot = raw[channel] * gain;
            }
            out_l[i] = wet[0];
            out_r[i] = wet[1];
        }
    }
}

impl Default for TransientShaper {
    fn default() -> Self {
        Self::new()
    }
}

/// 转折频率 `hz` 处的单极点系数。
#[inline]
fn one_pole(hz: f32, sample_rate: f32) -> f32 {
    (1.0 - (-TAU * hz / sample_rate).exp()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    /// 每个测量块的长度（与来源一致：前一半是瞬态，只测后一半）。
    const BLOCK: usize = 8192;

    /// 单 bin 幅度，加 Hann 窗（本 crate 其余 DSP 测试用的同一把尺子）。
    fn bin_mag(samples: &[f32], freq: f32, sr: f32) -> f32 {
        let n = samples.len();
        let w = f64::from(TAU) * f64::from(freq) / f64::from(sr);
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, x) in samples.iter().enumerate() {
            let win = 0.5 - 0.5 * (f64::from(TAU) * i as f64 / n as f64).cos();
            let v = f64::from(*x) * win;
            re += v * (w * i as f64).cos();
            im -= v * (w * i as f64).sin();
        }
        ((re * re + im * im).sqrt() / n as f64) as f32 * 2.0
    }

    fn db(ratio: f32) -> f32 {
        20.0 * ratio.max(1e-12).log10()
    }

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / SR).sin()).collect()
    }

    /// 纯 DSP 增益测量：把 `n` 个样本的正弦送进一个全新的状态块，
    /// 比较稳定后的输出 bin 与输入 bin。
    fn dsp_gain(freq: f32, mut run: impl FnMut(&[f32], &mut [f32])) -> f32 {
        let n = BLOCK;
        let input = sine(freq, n);
        let mut out = vec![0.0f32; n];
        run(&input, &mut out);
        // 前一半是瞬态；测后一半。
        let settled = &out[n / 2..];
        let reference = bin_mag(&input[n / 2..], freq, SR);
        bin_mag(settled, freq, SR) / reference
    }

    // ------------------------------------------------------------ bitcrusher

    #[test]
    fn bit_depth_sets_the_quantisation_step() {
        // 全量程斜坡，无抗混叠、无分频。两件事钉住量化深度：每个输出都落在
        // `2 / 2^bits` 的网格上，且相对输入的误差从不超过半个步长。斜坡足够长
        // （2^17），因此 16 bit 时它每样本移动的距离仍小于一个量化级——
        // 否则"网格检查"测的就是斜坡自己的分辨率了。
        for bits in [4.0f32, 6.0, 8.0, 12.0, 16.0] {
            let n = 1 << 17;
            let input: Vec<f32> = (0..n)
                .map(|i| -1.0 + 2.0 * i as f32 / (n - 1) as f32)
                .collect();
            let mut out = vec![0.0f32; n];
            let mut out_r = vec![0.0f32; n];
            BitCrusher::new().process(
                &input,
                &input,
                &mut out,
                &mut out_r,
                CrushParams {
                    bits,
                    down: 1.0,
                    aa: 0.0,
                },
                SR,
            );
            let step = 2.0 / 2.0f32.powf(bits);
            let mut worst_grid = 0.0f32;
            let mut worst_error = 0.0f32;
            for (&x, &y) in input.iter().zip(&out).skip(64) {
                let grid = y / step;
                worst_grid = worst_grid.max((grid - grid.round()).abs());
                worst_error = worst_error.max((y - x).abs());
            }
            assert!(
                worst_grid < 1e-2,
                "{bits} bits: an output is {worst_grid} of a step off the {step} grid"
            );
            assert!(
                worst_error <= step * 0.5 + step * 1e-3,
                "{bits} bits: error {worst_error} exceeds half a step ({})",
                step * 0.5
            );
        }
    }

    /// **判据（新写，可红）**：`bits` 的**下界**（`4.0`）把一切更小的合法请求
    /// **折叠到同一条输出**上，而不是让它们各自生效。
    ///
    /// 量什么：同一段输入在五个 `bits` 请求下的输出位型（单位：`f32` 位型序列）
    /// 是否与 `bits = 4.0` 的那一次**逐位相同**。
    ///
    /// 为什么需要它：[`bit_depth_sets_the_quantisation_step`] 的夹具最小只到
    /// `4.0`（正是下界本身），而且它的网格期望是用**请求值**算出来的
    ///（`2.0 / 2f32.powf(bits)`）⇒ 把下界 `4.0` 放宽成 `1.0`（本票注入 S01）之后
    /// 请求值与量化步长一起变，判据自洽 ⇒ 全量 417 条判据全绿。
    ///
    /// 链路只有比较、`powf` 与乘除：`powf` 属 ADR-0001 的超越函数类
    /// ⇒ 只在**同一架构内**比对五次读数的位型，⛔ 不与字面常量比。
    #[test]
    fn a_bit_depth_below_the_floor_is_folded_onto_the_floor() {
        let n = 4_096;
        let input: Vec<f32> = (0..n)
            .map(|index| ((index * 37 % 101) as f32 / 101.0) - 0.5)
            .collect();
        let render = |bits: f32| {
            let mut out = vec![0.0f32; n];
            let mut out_r = vec![0.0f32; n];
            BitCrusher::new().process(
                &input,
                &input,
                &mut out,
                &mut out_r,
                CrushParams {
                    bits,
                    down: 1.0,
                    aa: 0.0,
                },
                SR,
            );
            out.iter()
                .map(|value| value.to_bits())
                .collect::<Vec<u32>>()
        };
        let floor = render(4.0);
        for bits in [-5.0f32, 0.0, 1.0, 2.0, 3.999] {
            assert_eq!(
                render(bits),
                floor,
                "bits = {bits} 必须被折到下界 4.0 的同一输出上"
            );
        }
        // 正对照：下界**之上**必须真的不同，否则上面几条可能是"整条路都不敏感"。
        assert_ne!(render(6.0), floor, "6 bit 必须与 4 bit 不同");
        // 非空证明：夹具本身必须真的产生非零输出。
        assert!(
            floor.iter().any(|bits| *bits != 0),
            "夹具输出全零 ⇒ 判据测的是空壳"
        );
    }

    #[test]
    fn divisor_and_anti_alias_move_the_mirror() {
        // 分频器带来两种不同的产物：
        //   * 采样保持对带内音的**镜像**，位于 sr/down − f
        //     （down = 8 时 1 kHz 变成 5 kHz），由插值滤波器清除；
        //   * 高于抽取后 Nyquist 的音的**真混叠**：8 kHz 折到 6 − 8 = −2 kHz，
        //     即 2 kHz，由前置滤波器清除。
        // 参考是同一音高、完全不分频的那一次，因此下面每个数都是相对输入的
        // 电平，而不是相对 crusher 恰好在输入频率上留下的东西
        // （8 kHz 被 8 分频后在 8 kHz 上**什么都没了**，用它做参考就是除以零）。
        let reference = |freq: f32| {
            let n = BLOCK;
            let input = sine(freq, n);
            let mut out = vec![0.0f32; n];
            let mut out_r = vec![0.0f32; n];
            BitCrusher::new().process(
                &input,
                &input,
                &mut out,
                &mut out_r,
                CrushParams {
                    bits: 16.0,
                    down: 1.0,
                    aa: 0.0,
                },
                SR,
            );
            bin_mag(&out[n / 2..], freq, SR)
        };
        let level = |freq: f32, probe: f32, down: f32, aa: f32| {
            let n = BLOCK;
            let input = sine(freq, n);
            let mut out = vec![0.0f32; n];
            let mut out_r = vec![0.0f32; n];
            BitCrusher::new().process(
                &input,
                &input,
                &mut out,
                &mut out_r,
                CrushParams {
                    bits: 8.0,
                    down,
                    aa,
                },
                SR,
            );
            db(bin_mag(&out[n / 2..], probe, SR) / reference(freq))
        };
        // 不分频：两种产物都不在。
        let no_divisor = level(1000.0, 5000.0, 1.0, 0.0);
        assert!(
            no_divisor < -40.0,
            "down = 1 still mirrors at {no_divisor:.1} dB"
        );
        // 分频器的镜像，开与不开抗混叠。
        let raw_image = level(1000.0, 5000.0, 8.0, 0.0);
        let smooth_image = level(1000.0, 5000.0, 8.0, 1.0);
        assert!(
            raw_image > -30.0,
            "the divider's image is only {raw_image:.1} dB down"
        );
        assert!(
            raw_image - smooth_image > 12.0,
            "anti-aliasing removed only {:.1} dB of image",
            raw_image - smooth_image
        );
        // 真混叠：8 kHz 高于 3 kHz 的抽取后 Nyquist。
        let raw_alias = level(8000.0, 2000.0, 8.0, 0.0);
        let smooth_alias = level(8000.0, 2000.0, 8.0, 1.0);
        assert!(
            raw_alias > -30.0,
            "the folded 8 kHz tone is only {raw_alias:.1} dB down"
        );
        assert!(
            raw_alias - smooth_alias > 12.0,
            "anti-aliasing removed only {:.1} dB of aliasing",
            raw_alias - smooth_alias
        );
    }

    #[test]
    fn deeper_divisors_push_the_mirror_lower_in_level_and_frequency() {
        let freq = 700.0f32;
        let measure = |down: f32| {
            let n = BLOCK;
            let input = sine(freq, n);
            let mut out = vec![0.0f32; n];
            let mut out_r = vec![0.0f32; n];
            BitCrusher::new().process(
                &input,
                &input,
                &mut out,
                &mut out_r,
                CrushParams {
                    bits: 16.0,
                    down,
                    aa: 0.0,
                },
                SR,
            );
            let fold = 48_000.0 / down - freq;
            let tone = bin_mag(&out[n / 2..], freq, SR);
            (fold, db(bin_mag(&out[n / 2..], fold, SR) / tone))
        };
        // 镜像位于 sr/down − f，且对每个大于 1 的除数都存在。
        for down in [2.0f32, 4.0, 8.0, 16.0] {
            let (fold, level) = measure(down);
            assert!(
                (fold - (48_000.0 / down - freq)).abs() < 1.0,
                "down {down}: fold at {fold}"
            );
            assert!(
                level > -40.0,
                "down {down}: no mirror found ({level:.1} dB)"
            );
        }
    }

    // ---------------------------------------------------------- shaping EQ

    fn eq_gain(freq: f32, params: EqParams) -> f32 {
        let zeros = vec![0.0f32; BLOCK];
        let mut rights = vec![0.0f32; BLOCK];
        let mut eq = ShapingEq::new();
        dsp_gain(freq, |input, out| {
            // 右声道喂零：测的是左声道。
            eq.process(input, &zeros, out, &mut rights, params, SR);
        })
    }

    #[test]
    fn eq_flat_is_unity() {
        // 每个频段都是 0 dB 就是恒等，因此整段完全不得染色。
        for freq in [65.0f32, 200.0, 1000.0, 4000.0, 12000.0] {
            let gain = db(eq_gain(freq, EqParams::default()));
            assert!(gain.abs() < 0.05, "flat EQ {freq} Hz: {gain:.3} dB");
        }
    }

    #[test]
    fn eq_band_gains_match_the_cookbook() {
        // 架子在远端到达它的**满**增益、在转折点到达其一半（按 dB），
        // 因此增益要在远离转折的地方测。
        let low = db(eq_gain(
            65.0,
            EqParams {
                low_gain: 12.0,
                low_freq: 400.0,
                ..EqParams::default()
            },
        ));
        assert!((low - 12.0).abs() < 1.0, "low shelf end: {low:.2} dB");
        let corner = db(eq_gain(
            400.0,
            EqParams {
                low_gain: 12.0,
                low_freq: 400.0,
                ..EqParams::default()
            },
        ));
        assert!(
            (corner - 6.0).abs() < 1.0,
            "low shelf corner: {corner:.2} dB"
        );
        // 峰值频段在中心处精确。
        let mid = db(eq_gain(
            1000.0,
            EqParams {
                mid_gain: -9.0,
                mid_freq: 1000.0,
                ..EqParams::default()
            },
        ));
        assert!((mid + 9.0).abs() < 0.5, "mid peak centre: {mid:.2} dB");
        let high = db(eq_gain(
            16000.0,
            EqParams {
                high_gain: 9.0,
                high_freq: 3000.0,
                ..EqParams::default()
            },
        ));
        assert!((high - 9.0).abs() < 1.0, "high shelf end: {high:.2} dB");
        // 偏离中心时，峰值频段已经回落向单位增益。
        let away = db(eq_gain(
            4000.0,
            EqParams {
                mid_gain: 9.0,
                mid_freq: 1000.0,
                mid_q: 1.0,
                ..EqParams::default()
            },
        ));
        assert!(away < 1.0, "mid peak two octaves away: {away:.2} dB");
    }

    #[test]
    fn eq_shelves_reach_their_gain_at_the_ends_and_slope_back() {
        let params = EqParams {
            low_gain: 12.0,
            low_freq: 400.0,
            ..EqParams::default()
        };
        let bottom = db(eq_gain(65.0, params));
        let corner = db(eq_gain(400.0, params));
        let above = db(eq_gain(3200.0, params));
        let top = db(eq_gain(16000.0, params));
        assert!(
            (bottom - 12.0).abs() < 1.0,
            "low shelf at 65 Hz: {bottom:.2} dB"
        );
        assert!(top.abs() < 1.0, "low shelf at 16 kHz: {top:.2} dB");
        // 是架子而不是峰值：回落过程中单调，转折点落在半增益附近。
        assert!(
            bottom > corner && corner > above && above > top,
            "low shelf is not monotone: {bottom:.2} {corner:.2} {above:.2} {top:.2}"
        );
        assert!(
            (corner - 6.0).abs() < 1.5,
            "the low shelf's corner should sit near half gain: {corner:.2} dB"
        );
        assert!(
            bottom - above > 6.0,
            "the low shelf only fell {:.2} dB over three octaves",
            bottom - above
        );
        // 高架是它的镜像。
        let params = EqParams {
            high_gain: -12.0,
            high_freq: 3000.0,
            ..EqParams::default()
        };
        let low_end = db(eq_gain(65.0, params));
        let high_end = db(eq_gain(16000.0, params));
        assert!(low_end.abs() < 1.0, "high shelf at 65 Hz: {low_end:.2} dB");
        assert!(
            (high_end + 12.0).abs() < 1.0,
            "high shelf at 16 kHz: {high_end:.2} dB"
        );
    }

    // ------------------------------------------------------ transient shaper

    /// **判据（新写，可红）**：两个量都是 0 时是**逐位**恒等。
    ///
    /// 来源只在注释里声称这一点（"`0` for both is a mathematical identity —
    /// the gain is exactly 1 and the output is the input"），并且在引擎层间接测过；
    /// 这里直接在 DSP 层断言逐位相等。
    ///
    /// **实测的判据强度（诚实记录）**：一条更早的说法——"删掉 `neutral` 短路就会
    /// 变红"——已被变异测试推翻：它**不会**变红。原因是 `0.0 * x` 恰好是 `0.0`、
    /// `exp2(0.0)` 恰好是 `1.0`、`x * 1.0` 恰好是 `x`，因此那个短路在算术上是
    /// 冗余的（它只是把意图说明白）。这条判据真正钉住的是**整条增益路径必须是
    /// 恒等**：把增益指数扰动 0.01 dB（`exp2((db + 0.01) * DB_TO_LOG2)`）
    /// 本测试立即变红。
    #[test]
    fn the_transient_shaper_is_bit_exact_when_neutral() {
        let n = 4096;
        let input = sine(1000.0, n);
        let mut out_l = vec![0.0f32; n];
        let mut out_r = vec![0.0f32; n];
        TransientShaper::new().process(
            &input,
            &input,
            &mut out_l,
            &mut out_r,
            TransientParams::default(),
            SR,
        );
        for (index, (a, b)) in input.iter().zip(&out_l).enumerate() {
            assert_eq!(a, b, "neutral shaper changed sample {index}");
        }
        assert_eq!(out_l, out_r);
    }

    /// **判据（新写，可红）**：起音被抬升、释放被压低，且增益与电平无关。
    ///
    /// 用同一段"起音 → 保持 → 释放"包络在两个电平上跑：`attack = 1` 应在起音
    /// 窗口内抬升，`sustain = 1` 应在释放窗口内压低；两个电平上的**增益**
    /// （输出/输入包络比）必须一致，这是"用更响的跟随器归一化"的直接推论。
    ///
    /// 把 `scale = 1.0 / fast.max(slow).max(1e-6)` 换成不做归一化的 `1.0`，
    /// 增益会随电平变化，本测试立即变红。
    #[test]
    fn the_transient_shaper_is_level_independent() {
        /// 起音 10 ms、保持 200 ms、释放 50 ms 的梯形包络。
        ///
        /// 保持段必须足够长：慢跟随器的 τ ≈ 40 ms（4 Hz），要让"持续音回到单位
        /// 增益"可测，得给它 4–5 个 τ。
        fn burst(amplitude: f32) -> Vec<f32> {
            let attack = 480;
            let hold = 9_600;
            let release = 2_400;
            let n = attack + hold + release;
            (0..n)
                .map(|i| {
                    let env = if i < attack {
                        i as f32 / attack as f32
                    } else if i < attack + hold {
                        1.0
                    } else if i < attack + hold + release {
                        1.0 - (i - attack - hold) as f32 / release as f32
                    } else {
                        0.0
                    };
                    amplitude * env * (TAU * 500.0 * i as f32 / SR).sin()
                })
                .collect()
        }
        /// 在 `[from, to)` 窗口内输出与输入的包络比。
        fn window_gain(amplitude: f32, params: TransientParams, from: usize, to: usize) -> f32 {
            let input = burst(amplitude);
            let n = input.len();
            let mut out = vec![0.0f32; n];
            let mut out_r = vec![0.0f32; n];
            TransientShaper::new().process(&input, &input, &mut out, &mut out_r, params, SR);
            let rms =
                |data: &[f32]| (data.iter().map(|v| v * v).sum::<f32>() / data.len() as f32).sqrt();
            rms(&out[from..to]) / rms(&input[from..to]).max(1e-9)
        }

        let attack_params = TransientParams {
            attack_amt: 1.0,
            sustain_amt: 0.0,
        };
        let sustain_params = TransientParams {
            attack_amt: 0.0,
            sustain_amt: 1.0,
        };
        // 起音窗口（前 5 ms，包络还在爬升）必须被抬起来。
        let loud_attack = window_gain(1.0, attack_params, 0, 240);
        let quiet_attack = window_gain(0.1, attack_params, 0, 240);
        assert!(
            loud_attack > 1.01,
            "attack did not lift the onset: {loud_attack}"
        );
        assert!(
            (db(loud_attack) - db(quiet_attack)).abs() < 0.5,
            "the shaper is level dependent: {} dB vs {} dB",
            db(loud_attack),
            db(quiet_attack)
        );
        // 释放窗口（包络下落中）必须被压下去。
        let release_from = 480 + 9_600 + 100;
        let loud_release = window_gain(1.0, sustain_params, release_from, release_from + 500);
        let quiet_release = window_gain(0.1, sustain_params, release_from, release_from + 500);
        assert!(
            loud_release < 0.99,
            "sustain did not duck the release: {loud_release}"
        );
        assert!(
            (db(loud_release) - db(quiet_release)).abs() < 0.5,
            "the shaper is level dependent on release: {} dB vs {} dB",
            db(loud_release),
            db(quiet_release)
        );
        // 稳态（保持段末尾，已经过了 4 个慢 τ）必须回到 1 附近，
        // 无论 attack 拧到多大。
        let steady = window_gain(1.0, attack_params, 480 + 7_200, 480 + 9_600);
        assert!(
            db(steady).abs() < 1.0,
            "a held note should settle to unity, got {:.2} dB",
            db(steady)
        );
        // 增益硬上限：任何控制量组合都不得越过它。
        let extreme = window_gain(
            1.0,
            TransientParams {
                attack_amt: 1.0,
                sustain_amt: -1.0,
            },
            0,
            240,
        );
        assert!(
            extreme <= MAX_TRANSIENT_GAIN,
            "gain escaped the clamp: {extreme}"
        );
    }

    /// **判据（新写，可红）**：参数被猛砸时仍然有界且有限。
    ///
    /// 这是来源 `abrupt_changes_stay_bounded_and_finite`（引擎层）的 DSP 层版本：
    /// 那一条需要 `Engine`，本 crate 没有，因此把它的**意图**落到三个效果节点上，
    /// 逐块猛砸全部连续控制量（不碰 on/off，那是唯一的分档控制）。
    ///
    /// 若任何一处参数钳制被删掉（例如 `bits.clamp(4.0, 16.0)`），
    /// `2^bits` 会溢出成 `inf`、`step` 变成 0，输出立刻出现非有限值，本测试变红。
    #[test]
    fn abrupt_parameter_slams_stay_bounded() {
        let blocks = 200;
        let block = 128;
        let mut crusher = BitCrusher::new();
        let mut eq = ShapingEq::new();
        let mut shaper = TransientShaper::new();
        let mut phase = 0.0f32;
        let mut peak = 0.0f32;
        for index in 0..blocks {
            let input: Vec<f32> = (0..block)
                .map(|_| {
                    phase += TAU * 1000.0 / SR;
                    (phase).sin() * 0.25
                })
                .collect();
            let mut crushed = vec![0.0f32; block];
            let mut crushed_r = vec![0.0f32; block];
            crusher.process(
                &input,
                &input,
                &mut crushed,
                &mut crushed_r,
                CrushParams {
                    bits: 4.0 + (index % 13) as f32,
                    down: 1.0 + (index % 64) as f32,
                    aa: (index % 11) as f32 / 10.0,
                },
                SR,
            );
            let mut shaped = vec![0.0f32; block];
            let mut shaped_r = vec![0.0f32; block];
            eq.process(
                &crushed,
                &crushed_r,
                &mut shaped,
                &mut shaped_r,
                EqParams {
                    low_gain: (index % 37) as f32 - 18.0,
                    mid_gain: -((index % 37) as f32 - 18.0),
                    high_gain: (index % 37) as f32 - 18.0,
                    mid_freq: 200.0 + (index % 40) as f32 * 195.0,
                    low_freq: 20.0 + (index % 50) as f32 * 20.0,
                    high_freq: 1000.0 + (index % 60) as f32 * 250.0,
                    mid_q: 0.3 + (index % 20) as f32 * 0.3,
                },
                SR,
            );
            let mut final_l = vec![0.0f32; block];
            let mut final_r = vec![0.0f32; block];
            shaper.process(
                &shaped,
                &shaped_r,
                &mut final_l,
                &mut final_r,
                TransientParams {
                    attack_amt: (index % 41) as f32 / 20.0 - 1.0,
                    sustain_amt: 1.0 - (index % 41) as f32 / 20.0,
                },
                SR,
            );
            for (position, value) in final_l.iter().enumerate() {
                assert!(
                    value.is_finite(),
                    "block {index} sample {position}: non-finite output"
                );
                peak = peak.max(value.abs());
            }
            assert!(final_r.iter().all(|v| v.is_finite()));
            // 输入幅度 0.25：EQ 最多 +18 dB（8×）+ transient 最多 8× 也远低于此，
            // 而 crusher 被量化器限制在 ±1。
            assert!(peak < 16.0, "block {index}: peak {peak} ran away");
        }
    }

    #[test]
    fn short_and_mismatched_buffers_are_safe() {
        let mut crusher = BitCrusher::new();
        let mut out = [0.0f32; 4];
        let mut out_r = [0.0f32; 4];
        // 输出比输入短：只处理能写下的部分，不得 panic。
        crusher.process(
            &[1.0f32; 8],
            &[1.0f32; 8],
            &mut out,
            &mut out_r,
            CrushParams::default(),
            SR,
        );
        assert!(out.iter().all(|v| v.is_finite()));
        // 空块是合法的。
        let mut empty: [f32; 0] = [];
        let mut empty_r: [f32; 0] = [];
        crusher.process(
            &[],
            &[],
            &mut empty,
            &mut empty_r,
            CrushParams::default(),
            SR,
        );
        let mut eq = ShapingEq::new();
        eq.process(&[], &[], &mut empty, &mut empty_r, EqParams::default(), SR);
        let mut shaper = TransientShaper::new();
        shaper.process(
            &[],
            &[],
            &mut empty,
            &mut empty_r,
            TransientParams::default(),
            SR,
        );
        // 退化参数与退化采样率不得产生非有限值。
        let mut out = [0.0f32; 512];
        let mut out_r = [0.0f32; 512];
        let mut shaper = TransientShaper::new();
        shaper.process(
            &[0.5f32; 512],
            &[0.5f32; 512],
            &mut out,
            &mut out_r,
            TransientParams {
                attack_amt: f32::NAN,
                sustain_amt: f32::INFINITY,
            },
            0.0,
        );
        assert!(out.iter().all(|v| v.is_finite()));
        assert!(
            out.iter().all(|v| v.abs() <= 0.5),
            "clamped amounts must not boost"
        );
        // 中性参数是逐位恒等（与状态无关）。
        let mut again = [0.0f32; 512];
        let mut again_r = [0.0f32; 512];
        shaper.process(
            &[0.5f32; 512],
            &[0.5f32; 512],
            &mut again,
            &mut again_r,
            TransientParams::default(),
            SR,
        );
        assert_eq!(again, [0.5f32; 512]);
    }

    /// **判据（新写，可红）**：`reset` 真的把状态清干净。
    ///
    /// 一个"用非中性参数跑过一段、再 reset"的实例，与一个全新的实例在同一段
    /// 输入上必须给出**逐位相同**的输出。若把 `reset` 改成空实现，本测试变红。
    #[test]
    fn reset_restores_a_fresh_state() {
        let n = 4_800;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                let env = if i < 240 { i as f32 / 240.0 } else { 1.0 };
                env * (TAU * 400.0 * i as f32 / SR).sin() * 0.5
            })
            .collect();
        let loud = TransientParams {
            attack_amt: 1.0,
            sustain_amt: -1.0,
        };

        let mut dirty = TransientShaper::new();
        let mut scratch = vec![0.0f32; n];
        let mut scratch_r = vec![0.0f32; n];
        dirty.process(&input, &input, &mut scratch, &mut scratch_r, loud, SR);
        dirty.reset();
        let mut after_reset = vec![0.0f32; n];
        let mut after_reset_r = vec![0.0f32; n];
        dirty.process(
            &input,
            &input,
            &mut after_reset,
            &mut after_reset_r,
            loud,
            SR,
        );

        let mut fresh = TransientShaper::new();
        let mut fresh_out = vec![0.0f32; n];
        let mut fresh_out_r = vec![0.0f32; n];
        fresh.process(&input, &input, &mut fresh_out, &mut fresh_out_r, loud, SR);
        assert_eq!(
            after_reset, fresh_out,
            "reset did not restore a fresh state"
        );
        assert_eq!(after_reset_r, fresh_out_r);
        // 而且这条判据不是恒真：非中性参数确实改变了输出。
        let mut neutral_out = vec![0.0f32; n];
        let mut neutral_out_r = vec![0.0f32; n];
        TransientShaper::new().process(
            &input,
            &input,
            &mut neutral_out,
            &mut neutral_out_r,
            TransientParams::default(),
            SR,
        );
        assert_ne!(
            fresh_out, neutral_out,
            "the non-neutral shaper did nothing at all"
        );
    }
}
