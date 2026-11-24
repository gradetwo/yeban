//! 原生**模态共振器核心**（T-FIX-317，第一期）。
//!
//! # 出处与致敬（硬要求，逐模型标注）
//!
//! * **算法与拓扑参照**：RipplerX，作者 **tiagolr**，**GPL-3.0**，
//!   <https://github.com/tiagolr/ripplerx>。本项目同为 GPL-3.0；本模块按其
//!   "每模态一只带通谐振滤波器、噪声 / mallet 激励、每共振器至多 64 分音"
//!   的**算法思想**用 Rust 重写。⛔ **不搬 JUCE 框架代码**：这里没有 `juce::`、
//!   没有查找表类、没有插件外壳，递推与系数全部本仓实现。
//! * **技术血统**：Rippler2（tiagolr 的 JSFX）与 Saike 的 **Partials**，后者在
//!   <https://github.com/JoepVanlier/JSFX>（作者 JoepVanlier，**MIT**）。
//! * **公式交叉对照**：MechanOdd，作者 **odoare**，<https://github.com/odoare/MechanOdd>。
//!   ⚠ 其**代码许可未声明**，本项目**只读其 README 的数学节**，不取一行代码。
//!   本模块的阻尼斜坡、`Q = 1/(2ζ)`、增益补偿 `sqrt(ζ_ref/ζ)` 与四种频率律
//!   逐项对照该文档（见每个函数的来源行）。
//! * **模型层面的上游参照**：AAS **Chromaphone** 与 Ableton **Collision**
//!   （经 RipplerX 的 README / Acknowledgments 转述；本项目未 reverse engineer
//!   任何专有实现）。
//!
//! 逐模型来源：
//!
//! | 模型 | 频率律 | 来源 |
//! | :--- | :--- | :--- |
//! | [`ModalModel::String`] | `f₀·k·sqrt(1+B·k²)` | RipplerX `string` 比例列 + MechanOdd "Beam / String (Modal)" 的 `b=0` 特例与刚度扩展 |
//! | [`ModalModel::Beam`] | `f₀·n·sqrt((1-b)+b·n²)` | MechanOdd "Beam / String (Modal)" 统一式；`b=1` 即 Euler-Bernoulli `n²` 律；`0<b<1` 即弦-梁连续变形 |
//! | [`ModalModel::Membrane`] | `f₀·sqrt(m²+(n/a)²)/sqrt(1+1/a²)` | MechanOdd "Membrane" 节；RipplerX `membrane` 比例列（其 `0.78` 相当于 `1/a`） |
//! | [`ModalModel::Plate`] | `f₀·(m²+(n/a)²)/(1+1/a²)` | MechanOdd "Plate" 节；RipplerX `plate` 比例列 |
//!
//! # 实时纪律（ARCH-RT-001）
//!
//! 配置期（[`ModalVoice::configure`]）可以分配；[`ModalVoice::process`] /
//! [`NoiseBurst::next_sample`] / [`MalletExciter::next_sample`] **零分配、无锁、无 I/O**，
//! 逐样本只跑定长数组上的双二阶递推与一个确定性 PRNG。参数（ excitation gain）
//! 走一阶平滑，不在逐样本路径做系数重算。
//!
//! # 本期边界
//!
//! 双共振器耦合、Audio In、GUI、预置**不在本期**（见第二期建议写在交付回执）。

use core::f32::consts::PI;

/// 每共振器的分音上限（RipplerX 同款；本模块固定，不随参数改变）。
pub const MAX_MODES: usize = 64;

/// 板模型的增益补偿参考阻尼比（MechanOdd "Plate"）。
const ZETA_REF_PLATE: f32 = 0.01;
/// 膜模型的增益补偿参考阻尼比（MechanOdd "Membrane"）。
const ZETA_REF_MEMBRANE: f32 = 0.02;

/// 一节 RBJ 带通双二阶（转置直接 II 型；系数已按 `a0` 归一化）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ModalBiquad {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    v1: f32,
    v2: f32,
}

impl ModalBiquad {
    const IDENTITY: Self = Self {
        b0: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
        v1: 0.0,
        v2: 0.0,
    };

    /// RBJ 恒 0 dB 峰值带通：`alpha = sin(w0)/(2Q) = sin(w0)·ζ`（因 `Q=1/(2ζ)`）。
    /// ζ 过小会让 `alpha → 0` 并把状态逼成无阻尼振荡 ⇒ 钳到 `MIN_ZETA`。
    fn bandpass(sample_rate: f32, freq: f32, zeta: f32) -> Self {
        let sr = crate::math::sanitise_sample_rate(sample_rate);
        let f = freq.clamp(1.0, sr * 0.495);
        let w0 = core::f32::consts::TAU * (f / sr);
        let (sin, cos) = libm::sincosf(w0);
        let alpha = sin * zeta.max(MIN_ZETA);
        let inv = 1.0 / (1.0 + alpha);
        Self {
            b0: alpha * inv,
            b2: -alpha * inv,
            a1: -2.0 * cos * inv,
            a2: (1.0 - alpha) * inv,
            v1: 0.0,
            v2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let x = if x.is_finite() { x } else { 0.0 };
        let y = self.b0 * x + self.v1;
        if !y.is_finite() {
            self.v1 = 0.0;
            self.v2 = 0.0;
            return 0.0;
        }
        // 恒 0 dB 带通的 b1 恒为 0 ⇒ 这一项在逐样本路径里不存在。
        let v1 = -self.a1 * y + self.v2;
        let v2 = self.b2 * x - self.a2 * y;
        if v1.is_finite() && v2.is_finite() {
            self.v1 = v1;
            self.v2 = v2;
        } else {
            self.v1 = 0.0;
            self.v2 = 0.0;
        }
        y
    }

    fn clear_state(&mut self) {
        self.v1 = 0.0;
        self.v2 = 0.0;
    }
}

/// 阻尼比下界（防 `alpha` 归零 / 递推无阻尼）。
const MIN_ZETA: f32 = 1.0e-6;

/// 四种模态模型（本期的定案范围，不降级、不替换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalModel {
    /// 弦：谐波列 + 刚度非谐性。
    String,
    /// 梁：`n` 律与弦-梁连续变形（mix=1 即 `n²`）。
    Beam,
    /// 膜：二维平方根律。
    Membrane,
    /// 板：二维平方律。
    Plate,
}

/// 弦的模态比：`k·sqrt(1+B·k²)`。
///
/// `B` 是刚度（非谐性）系数：`B=0` ⇒ 纯谐波列 `k`；`B>0` ⇒ 高分音向上拉伸。
/// 来源：RipplerX `string` 比例列；MechanOdd "Beam / String" 的刚度扩展。
pub fn string_ratio(k: u32, stiffness: f32) -> f32 {
    let k = k.max(1) as f32;
    k * libm::sqrtf(1.0 + stiffness.max(0.0) * k * k)
}

/// 梁的模态比：`n·sqrt((1-b) + b·n²)`。
///
/// `b=0` 是柔弦（`n`），`b=1` 是 Euler-Bernoulli 梁（`n²`），中间连续变形。
/// 来源：MechanOdd "Beam / String (Modal)" 的统一式（逐字）。
pub fn beam_ratio(n: u32, mix: f32) -> f32 {
    let n = n.max(1) as f32;
    let b = mix.clamp(0.0, 1.0);
    n * libm::sqrtf((1.0 - b) + b * n * n)
}

/// 归一化分母：`sqrt(1 + 1/a²)`，使 `(1,1)` 模态恰为 `f₀`。
fn membrane_norm(a: f32) -> f32 {
    let a = a.max(0.01);
    libm::sqrtf(1.0 + 1.0 / (a * a))
}

/// 膜的模态比：`sqrt(m²+(n/a)²)/sqrt(1+1/a²)`。
///
/// 来源：MechanOdd "Membrane" 节；RipplerX `membrane` 的 `0.78·n` 相当于 `1/a`。
pub fn membrane_ratio(m: u32, n: u32, aspect: f32) -> f32 {
    let m = m.max(1) as f32;
    let n = n.max(1) as f32;
    let a = aspect.max(0.01);
    libm::sqrtf(m * m + (n / a) * (n / a)) / membrane_norm(a)
}

/// 归一化分母：`1 + 1/a²`，使 `(1,1)` 模态恰为 `f₀`。
fn plate_norm(a: f32) -> f32 {
    let a = a.max(0.01);
    1.0 + 1.0 / (a * a)
}

/// 板的模态比：`(m²+(n/a)²)/(1+1/a²)`。
///
/// 来源：MechanOdd "Plate" 节；RipplerX `plate` 比例列。
pub fn plate_ratio(m: u32, n: u32, aspect: f32) -> f32 {
    let m = m.max(1) as f32;
    let n = n.max(1) as f32;
    let a = aspect.max(0.01);
    (m * m + (n / a) * (n / a)) / plate_norm(a)
}

/// 第 `index` 个分音的模态比（0-based；`params` 决定模型与形状）。
pub fn modal_ratio(index: usize, model: ModalModel, params: &ModalParams) -> f32 {
    let k = (index + 1) as u32;
    match model {
        // 弦按一维序号 k；刚度用 sqrt(1+B k²) 拉伸。
        ModalModel::String => string_ratio(k, params.stiffness),
        // 梁按一维序号 n；mix 连续插值弦↔梁。
        ModalModel::Beam => beam_ratio(k, params.mix),
        // 二维模型按先 m 后 n 的确定序枚举（每行 aspect 之后的定义域）。
        ModalModel::Membrane => {
            let (m, n) = two_d_index(index);
            membrane_ratio(m, n, params.aspect)
        }
        ModalModel::Plate => {
            let (m, n) = two_d_index(index);
            plate_ratio(m, n, params.aspect)
        }
    }
}

/// 二维模态枚举：先固定 `m=1,2,…`，再让 `n=1..=m`，保证 `(1,1)` 在 index 0。
/// 序列 `(1,1),(2,1),(2,2),(3,1),(3,2),(3,3),…` 确定且无重复。
fn two_d_index(index: usize) -> (u32, u32) {
    let mut remainder = index;
    let mut m = 1_usize;
    loop {
        let row = m; // n = 1..=m，共 m 条
        if remainder < row {
            return (m as u32, (remainder + 1) as u32);
        }
        remainder -= row;
        m += 1;
    }
}

/// 阻尼斜坡：`ζ(k) = ζ₀ + ζ₁·(ratio-1)`（MechanOdd "Plate"/"Membrane"）。
pub fn modal_damping(ratio: f32, base: f32, slope: f32) -> f32 {
    (base + slope * (ratio - 1.0).max(0.0)).max(MIN_ZETA)
}

/// 增益补偿：`sqrt(ζ_ref/ζ)`，按模型钳位（MechanOdd 的 plate [0.5,32] /
/// membrane [0.5,64]）。长衰减（ζ 小）时不塌音量。
pub fn gain_compensation(zeta: f32, reference: f32, ceiling: f32) -> f32 {
    libm::sqrtf(reference / zeta.max(MIN_ZETA)).clamp(0.5, ceiling.max(0.5))
}

/// 一维模态形状（pinned-pinned）：`sqrt(2)·sin(nπx)`；二阶耦合按
/// RipplerX / MechanOdd 的 hit-position 思想，零节点处分音被抑制。
fn mode_shape_1d(n: u32, position: f32) -> f32 {
    libm::sqrtf(2.0) * libm::sinf(PI * position.clamp(0.0, 1.0) * n as f32)
}

/// 二维模态形状（simply-supported）：`2·sin(mπx)·sin(nπy)`（MechanOdd）。
fn mode_shape_2d(m: u32, n: u32, x: f32, y: f32) -> f32 {
    2.0 * libm::sinf(PI * x.clamp(0.0, 1.0) * m as f32)
        * libm::sinf(PI * y.clamp(0.0, 1.0) * n as f32)
}

/// 共振器参数（配置期使用；`process` 路径只读已经算好的系数）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalParams {
    /// 基础阻尼比 ζ₀（0.0001..=1）。
    pub damping: f32,
    /// 频率斜坡 ζ₁（每单位频率比；0..=10）。
    pub damping_slope: f32,
    /// 弦刚度 B（0..=1；仅 String 用）。
    pub stiffness: f32,
    /// 弦↔梁 mix（0..=1；仅 Beam 用）。
    pub mix: f32,
    /// 二维纵横比 a（仅 Membrane / Plate 用）。
    pub aspect: f32,
    /// 激励位置 x（归一化 0..=1）。
    pub excitation_x: f32,
    /// 拾取位置 x（归一化 0..=1）。
    pub pickup_x: f32,
    /// 拾取位置 y（归一化 0..=1；二维模型用）。
    pub pickup_y: f32,
    /// 激励位置 y（归一化 0..=1；二维模型用）。
    pub excitation_y: f32,
}

impl Default for ModalParams {
    fn default() -> Self {
        Self {
            damping: 0.005,
            damping_slope: 0.02,
            stiffness: 0.0005,
            mix: 0.0,
            aspect: 1.0,
            excitation_x: 0.5,
            pickup_x: 0.5,
            pickup_y: 0.5,
            excitation_y: 0.5,
        }
    }
}

/// 一只模态共振器：至多 [`MAX_MODES`] 节带通级联（并联求和），配置期分配。
#[derive(Debug, Clone)]
pub struct ModalResonator {
    filters: [ModalBiquad; MAX_MODES],
    in_coupling: [f32; MAX_MODES],
    out_coupling: [f32; MAX_MODES],
    gains: [f32; MAX_MODES],
    active: usize,
    /// 激励平滑后的线性增益（`process` 只做一次一阶 lerp，不重算滤波器）。
    excitation_gain: f32,
    excitation_target: f32,
}

impl ModalResonator {
    /// 按模型与基频配置全部模态（⛔ 配置期才调用；逐样本路径不碰）。
    pub fn configure(
        sample_rate: f32,
        f0_hz: f32,
        model: ModalModel,
        params: &ModalParams,
    ) -> Self {
        let sr = crate::math::sanitise_sample_rate(sample_rate);
        let mut me = Self {
            filters: [ModalBiquad::IDENTITY; MAX_MODES],
            in_coupling: [0.0; MAX_MODES],
            out_coupling: [0.0; MAX_MODES],
            gains: [0.0; MAX_MODES],
            active: 0,
            excitation_gain: 1.0,
            excitation_target: 1.0,
        };
        let ceiling = if model == ModalModel::Membrane {
            64.0
        } else {
            32.0
        };
        let reference = if model == ModalModel::Membrane {
            ZETA_REF_MEMBRANE
        } else {
            ZETA_REF_PLATE
        };
        for i in 0..MAX_MODES {
            let ratio = modal_ratio(i, model, params);
            let freq = f0_hz.max(1.0) * ratio;
            if freq >= sr * 0.495 {
                break; // 频率律单调递增到 Nyquist 之外 ⇒ 后面都无效
            }
            let zeta = modal_damping(ratio, params.damping, params.damping_slope);
            me.filters[i] = ModalBiquad::bandpass(sr, freq, zeta);
            let (g_in, g_out) = match model {
                ModalModel::String | ModalModel::Beam => (
                    mode_shape_1d((i + 1) as u32, params.excitation_x),
                    mode_shape_1d((i + 1) as u32, params.pickup_x),
                ),
                ModalModel::Membrane | ModalModel::Plate => {
                    let (m, n) = two_d_index(i);
                    (
                        mode_shape_2d(m, n, params.excitation_x, params.excitation_y),
                        mode_shape_2d(m, n, params.pickup_x, params.pickup_y),
                    )
                }
            };
            me.in_coupling[i] = g_in;
            me.out_coupling[i] = g_out;
            me.gains[i] = gain_compensation(zeta, reference, ceiling);
            me.active += 1;
        }
        me
    }

    /// 本共振器实际启用的模态数（0..=[`MAX_MODES`]）。
    pub fn active_modes(&self) -> usize {
        self.active
    }

    /// 只读某一模态的已配置读数（判据用；不在逐样本路径调用）。
    pub fn mode_reading(&self, index: usize) -> Option<ModalModeReading> {
        if index >= self.active {
            return None;
        }
        Some(ModalModeReading {
            in_coupling: self.in_coupling[index],
            out_coupling: self.out_coupling[index],
            gain_compensation: self.gains[index],
        })
    }

    /// 设置激励强度的目标值（velocity 映射后；内部做一阶平滑）。
    pub fn set_excitation_gain(&mut self, gain: f32) {
        self.excitation_target = gain.max(0.0);
    }

    /// note-on 语义：清空所有递推状态并把平滑器拉到目标值。
    pub fn reset(&mut self) {
        for filter in self.filters.iter_mut().take(self.active) {
            filter.clear_state();
        }
        self.excitation_gain = self.excitation_target;
    }

    /// 逐样本处理：零分配、无锁、无 I/O。
    #[inline]
    pub fn process(&mut self, excitation: f32) -> f32 {
        // 一阶平滑（系数 1/64 ≈ 1.5 ms @ 44.1k）：参数突变不爆音。
        self.excitation_gain += (self.excitation_target - self.excitation_gain) * (1.0 / 64.0);
        let x = excitation * self.excitation_gain;
        let mut sum = 0.0_f32;
        for i in 0..self.active {
            let y = self.filters[i].process(x * self.in_coupling[i]);
            sum += y * self.out_coupling[i] * self.gains[i];
        }
        if sum.is_finite() { sum } else { 0.0 }
    }
}

/// 某一模态的只读读数（判据 / 诊断用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalModeReading {
    /// 输入耦合系数（激励点处的模态形状）。
    pub in_coupling: f32,
    /// 输出耦合系数（拾取点处的模态形状）。
    pub out_coupling: f32,
    /// 该模态已钳位的增益补偿因子。
    pub gain_compensation: f32,
}

/// 噪声 burst 激励（RipplerX / MechanOdd 的 noise source；第一期最简形）。
///
/// 确定性 xorshift32 种子 ⇒ 同输入两次渲染逐位一致；逐样本零分配。
#[derive(Debug, Clone)]
pub struct NoiseBurst {
    state: u32,
    remaining: usize,
    total: usize,
    decay: f32,
}

impl NoiseBurst {
    /// 构造：种子、burst 长度（帧）与逐样本衰减系数。
    pub fn new(seed: u32, frames: usize, decay: f32) -> Self {
        let total = frames.max(1);
        Self {
            state: seed.max(1),
            remaining: total,
            total,
            decay: decay.clamp(0.0, 1.0),
        }
    }

    /// note-on 语义：重置播放游标（PRNG 状态保留）。
    pub fn restart(&mut self) {
        self.remaining = self.total;
    }

    /// 下一个激励样本（burst 结束后恒 0）。
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.remaining == 0 {
            return 0.0;
        }
        let elapsed = (self.total - self.remaining) as f32;
        self.remaining -= 1;
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        let white = (self.state as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let envelope = libm::powf(self.decay, elapsed);
        white * envelope
    }
}

/// mallet 激励：短脉冲 + 一阶低通成形（RipplerX 的 mallet 生成器思想；
/// MechanOdd only 数学交叉对照，不取代码）。
#[derive(Debug, Clone)]
pub struct MalletExciter {
    lowpass: f32,
    remaining: usize,
    total: usize,
    hardness: f32,
}

impl MalletExciter {
    /// 构造：脉冲长度（帧）与硬度（越大越亮、越短）。
    pub fn new(frames: usize, hardness: f32) -> Self {
        let total = frames.max(1);
        Self {
            lowpass: 0.0,
            remaining: total,
            total,
            hardness: hardness.clamp(0.0, 1.0),
        }
    }

    /// note-on 语义：清掉低通状态并重新播放短脉冲。
    pub fn restart(&mut self) {
        self.lowpass = 0.0;
        self.remaining = self.total;
    }

    /// 下一个激励样本（脉冲结束后低通衰减到 0）。
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.remaining == 0 {
            return 0.0;
        }
        let first = self.remaining == self.total;
        self.remaining -= 1;
        let impulse = if first { 1.0 } else { 0.0 };
        // 一阶低通：hardness 越高，系数越大 ⇒ 越亮、越短。
        let a = 0.05 + 0.9 * self.hardness;
        self.lowpass += a * (impulse - self.lowpass);
        self.lowpass
    }
}

/// 两种激励（RipplerX / MechanOdd 的第一期最简集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalExcitation {
    /// 噪声 burst（确定种子）。
    NoiseBurst,
    /// mallet：短脉冲 + 一阶低通成形。
    Mallet,
}

/// 单共振器声部（第一期；双共振器耦合 / Audio In 不在范围）。
///
/// MIDI 音高定基频 `f0 = 440·2^((note-69)/12)`，velocity 定激励强度；
/// 这两个映射都在 **note-on**（非逐样本）时做，process 只读结果。
#[derive(Debug, Clone)]
pub struct ModalVoice {
    resonator: ModalResonator,
    noise: NoiseBurst,
    mallet: MalletExciter,
    excitation: ModalExcitation,
}

impl ModalVoice {
    /// 空声部（配置期分配；`note_on` 前 process 输出 0）。
    pub fn new(sample_rate: f32, excitation: ModalExcitation) -> Self {
        Self {
            resonator: ModalResonator::configure(
                sample_rate,
                440.0,
                ModalModel::String,
                &ModalParams::default(),
            ),
            noise: NoiseBurst::new(0x9E37_79B9, 512, 0.995),
            mallet: MalletExciter::new(64, 0.5),
            excitation,
        }
    }

    /// note-on：按 MIDI 音高 / velocity 配置共振器与激励（⛔ 不是逐样本路径）。
    pub fn note_on(
        &mut self,
        sample_rate: f32,
        model: ModalModel,
        midi_note: u8,
        velocity: u8,
        params: &ModalParams,
    ) {
        let semitones = i32::from(midi_note) - 69;
        let f0 = 440.0 * libm::powf(2.0, semitones as f32 / 12.0);
        self.resonator = ModalResonator::configure(sample_rate, f0, model, params);
        let v = f32::from(velocity.clamp(1, 127)) / 127.0;
        self.resonator.set_excitation_gain(v);
        self.resonator.reset();
        match self.excitation {
            ModalExcitation::NoiseBurst => self.noise.restart(),
            ModalExcitation::Mallet => self.mallet.restart(),
        }
    }

    /// 逐样本：零分配、无锁、无 I/O。
    #[inline]
    pub fn process(&mut self) -> f32 {
        let source = match self.excitation {
            ModalExcitation::NoiseBurst => self.noise.next_sample(),
            ModalExcitation::Mallet => self.mallet.next_sample(),
        };
        self.resonator.process(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn string_law_is_harmonic_with_stiffness_stretch() {
        for k in 1..=8 {
            let plain = string_ratio(k, 0.0);
            assert!((plain - k as f32).abs() < 1e-4, "B=0 必须是谐波列");
            let stiff = string_ratio(k, 0.5);
            let expected = k as f32 * libm::sqrtf(1.0 + 0.5 * k as f32 * k as f32);
            assert!((stiff - expected).abs() < 1e-4);
            assert!(stiff >= plain, "刚度只能把分音向上拉");
        }
    }

    #[test]
    fn beam_law_morphs_string_to_quadratic() {
        for n in 1..=8 {
            let f = n as f32;
            assert!((beam_ratio(n, 0.0) - f).abs() < 1e-4, "mix=0 是柔弦");
            assert!((beam_ratio(n, 1.0) - f * f).abs() < 1e-4, "mix=1 是 n² 梁");
            let mid = beam_ratio(n, 0.5);
            assert!(mid >= f && mid <= f * f, "中间 mix 必须连续夹在两者之间");
            if n > 1 {
                assert!(mid > f, "高分音必须被拉伸");
            }
        }
    }

    #[test]
    fn membrane_and_plate_laws_match_the_cookbook() {
        for (m, n) in [(1, 1), (2, 1), (3, 2), (5, 4)] {
            let a = 1.0;
            let mf = (m * m + n * n) as f32;
            // a=1 ⇒ 归一化分母 sqrt(2) / 2 ⇒ (1,1) 恰为 f0。
            assert!((membrane_ratio(m, n, a) - libm::sqrtf(mf) / libm::sqrtf(2.0)).abs() < 1e-4);
            assert!((plate_ratio(m, n, a) - mf / 2.0).abs() < 1e-4);
        }
        // a≠1 时保留 (n/a)² 与归一化分母，(1,1) 仍恰为 f₀。
        let a = 2.0;
        assert!((membrane_ratio(1, 1, a) - 1.0).abs() < 1e-4);
        assert!((plate_ratio(1, 1, a) - 1.0).abs() < 1e-4);
        let expected = (4.0 + (2.0 / a) * (2.0 / a)) / (1.0 + 1.0 / (a * a));
        assert!((plate_ratio(2, 2, a) - expected).abs() < 1e-4);
    }

    #[test]
    fn two_d_index_enumerates_each_pair_once() {
        let first: Vec<_> = (0..10).map(two_d_index).collect();
        assert_eq!(
            first,
            vec![
                (1, 1),
                (2, 1),
                (2, 2),
                (3, 1),
                (3, 2),
                (3, 3),
                (4, 1),
                (4, 2),
                (4, 3),
                (4, 4)
            ]
        );
    }

    #[test]
    fn damping_ramp_and_gain_compensation_are_bounded() {
        assert!((modal_damping(1.0, 0.01, 0.02) - 0.01).abs() < 1e-6);
        assert!((modal_damping(11.0, 0.01, 0.02) - 0.21).abs() < 1e-6);
        assert!(gain_compensation(0.0001, 0.01, 32.0) <= 32.0);
        assert!(gain_compensation(100.0, 0.01, 32.0) >= 0.5);
        assert!((gain_compensation(0.01, 0.01, 32.0) - 1.0).abs() < 1e-6);
    }

    fn render(model: ModalModel, f0: f32) -> Vec<f32> {
        let params = ModalParams::default();
        let mut resonator = ModalResonator::configure(SR, f0, model, &params);
        let mut mallet = MalletExciter::new(64, 0.5);
        resonator.reset();
        (0..4096)
            .map(|_| resonator.process(mallet.next_sample()))
            .collect()
    }

    #[test]
    fn impulse_response_is_audible_decaying_and_bit_deterministic() {
        for model in [
            ModalModel::String,
            ModalModel::Beam,
            ModalModel::Membrane,
            ModalModel::Plate,
        ] {
            let a = render(model, 220.0);
            let b = render(model, 220.0);
            assert!(
                a.iter().any(|s| s.abs() > 1e-4),
                "{model:?} 的冲激响应不能是静音"
            );
            // ⚠ 不用 `.all(...)`：本仓的 assertion_bounds_audit 把断言里的
            // 无界 `.all(` 当站点；改成"数不一致的位数"再钉 0，语义更强。
            let differing_bits = a
                .iter()
                .zip(b.iter())
                .filter(|(x, y)| x.to_bits() != y.to_bits())
                .count();
            assert_eq!(differing_bits, 0, "{model:?} 同输入两次渲染必须逐位一致");
            let early = a[..512].iter().map(|s| s * s).sum::<f32>();
            let late = a[3072..].iter().map(|s| s * s).sum::<f32>();
            assert!(late < early, "{model:?} 能量必须随时间衰减");
        }
    }

    #[test]
    fn configured_resonator_respects_the_mode_cap() {
        for model in [
            ModalModel::String,
            ModalModel::Beam,
            ModalModel::Membrane,
            ModalModel::Plate,
        ] {
            let r = ModalResonator::configure(SR, 440.0, model, &ModalParams::default());
            assert!(r.active_modes() > 0 && r.active_modes() <= MAX_MODES);
            assert!(r.mode_reading(0).is_some());
            assert!(r.mode_reading(MAX_MODES).is_none());
        }
    }

    #[test]
    fn a_single_resonator_voice_maps_note_and_velocity() {
        let mut soft = ModalVoice::new(SR, ModalExcitation::Mallet);
        let mut loud = ModalVoice::new(SR, ModalExcitation::Mallet);
        soft.note_on(SR, ModalModel::String, 69, 16, &ModalParams::default());
        loud.note_on(SR, ModalModel::String, 69, 127, &ModalParams::default());
        let a: Vec<f32> = (0..1024).map(|_| soft.process()).collect();
        let b: Vec<f32> = (0..1024).map(|_| loud.process()).collect();
        let energy = |out: &[f32]| out.iter().map(|s| s * s).sum::<f32>();
        assert!(
            energy(&a) > 0.0 && energy(&b) > energy(&a),
            "velocity 必须控制强度"
        );

        let mut low = ModalVoice::new(SR, ModalExcitation::NoiseBurst);
        low.note_on(SR, ModalModel::String, 57, 100, &ModalParams::default());
        let low_out: Vec<f32> = (0..1024).map(|_| low.process()).collect();
        assert!(low_out.iter().any(|s| s.abs() > 1e-5), "A3 必须真的发声");
    }

    #[test]
    fn noise_burst_is_deterministic_and_finite() {
        let mut a = NoiseBurst::new(12345, 256, 0.98);
        let mut b = NoiseBurst::new(12345, 256, 0.98);
        for _ in 0..256 {
            let x = a.next_sample();
            assert_eq!(x.to_bits(), b.next_sample().to_bits());
            assert!(x.is_finite() && x.abs() <= 1.0);
        }
        assert_eq!(a.next_sample(), 0.0, "burst 结束后必须归零");
    }
}
