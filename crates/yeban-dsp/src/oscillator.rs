//! 限带波表振荡器与全局 LFO。[ARCH-DSP-001] [ARCH-DET-001] [ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/wavetable.rs` + `src/dsp/lfo.rs`。
//!
//! # 一、mipmap 限带波表
//!
//! 一个波表就是一个波形周期，按音符频率回放。天真地这么做会严重混叠：周期里
//! 含有远高于高音符 Nyquist 的谐波。修法是 **mipmap**——每个倍频程一份限带副本，
//! 每份只保留在该音高下仍在 Nyquist 之下的谐波，因此一个音符无论选中哪一级，
//! 都**在构造上不可能**混叠。
//!
//! 工厂表由谐波配方生成，而不是作为样本附带：配方是几个数字，mipmap 由它派生，
//! 于是"限带"是发生器的性质，而不是某一步谨慎重采样的性质。
//!
//! [`Wavetable::from_cycle`] 走另一条路，服务于演奏者导入的波形：一个周期的样本
//! 过正向变换，然后每一级由低于其自身 Nyquist 的 bin 重建（理想砖墙，含相位），
//! 再归一化。同一种保证，同一种测法。
//!
//! ## 为什么每一级都是 [`BASE_LEN`] 长
//!
//! 来源的 P9.7 实测把残差归因于**插值器**：用线性插值读一份短表，是对带限信号的
//! 分段线性逼近，弦误差是一串高谐波，会折回 `SR − k·f0`（1/N²：16 点时 −24 dB，
//! 128 点时 −58 dB）。显而易见的修法——表更短但点更多——是用一个错误换一个更糟的：
//! 读取速率由音符固定，一份 `N` 点、`h` 个谐波的表，其每输出样本的相位步进是
//! `2πh/N`，弦误差由这个乘积决定，而不是由 `N` 单独决定。
//!
//! 反过来让每一级都保持 [`BASE_LEN`] 点：C8 只用 2048 点表里的 4 个谐波，
//! 读取几乎逐点对应，折回随之消失。实测（BH-7，4 s，精确 bin，真实 wasm）：
//! 最差的工厂库从 −25.8 dB 降到 −105 dB，全谱导入锯齿在 C2 从 −31 dB 降到 −119 dB。
//!
//! # 二、全局 LFO
//!
//! 所有声部共用一个 LFO（与参考 UI 一致）。它每个块求值一次写入暂存缓冲，
//! 于是颤音之类的逐样本目标保持无咔哒，而截止频率之类的逐块目标零额外成本。
//! one-shot 模式跑完一个周期后**停在周期末端**并保持该值，因此它表现得像一个
//! 额外的包络。
//!
//! # 与来源的差异
//!
//! 1. `Table` → [`Wavetable`]、`Recipe` → [`WaveRecipe`]、`CycleError` →
//!    [`WavetableError`]（`Table`/`Recipe` 在 DAW 语境里太泛）；
//! 2. `LfoWave` 在来源里住在 `crate::params`（与 TypeScript UI 共享的参数模型）。
//!    夜半的 `yeban-dsp` 不允许依赖上层模型，因此枚举定义在这里，并补上
//!    `#![deny(missing_docs)]` 要求的文档；
//! 3. 新增 [`WavetableOscillator`]：来源把相位累加器留在 `engine`/`voice` 里，
//!    但"选级 + 累加相位"正是这条抗混叠保证的落点，放在 DSP 层才能被单独判据锁住；
//! 4. `Lfo::render` 改为逐样本 [`Lfo::process`] 的薄封装，并新增 one-shot 回归测试
//!    （来源只在注释里声称该行为，没有测）。
//!
//! 表是构造期一次性建好的 `Vec`；逐样本路径零分配 [ARCH-RT-001]。

use crate::math::{fft, sanitise_sample_rate};

/// 每一级的表长。**所有**级都是这个长度：级与级的区别在含有多少谐波，
/// 而不在用了多少样本点。
pub const BASE_LEN: usize = 2048;

/// mip 级数：每一级承载一个倍频程的谐波量。
///
/// 最短的那一级才要紧：在键盘顶端，哪怕 16 个谐波也会越到 Nyquist 之上
///（16 × 4186 Hz），因此库必须一直降到只有几个谐波，否则最高音会混叠。
pub const LEVELS: usize = 9;

/// [`Wavetable::from_cycle`] 能接受的最短周期。分析阶段会把它重采样到
/// [`BASE_LEN`]，因此短到源分辨率不再有意义的程度就直接拒绝。
pub const MIN_CYCLE: usize = 64;

/// 谐波配方：每个谐波的幅度，`1` 表示基频。
pub type WaveRecipe = &'static [(u32, f32)];

/// 风琴味：强基频加前几个泛音。
pub const ORGAN: WaveRecipe = &[
    (1, 1.0),
    (2, 0.5),
    (3, 0.35),
    (4, 0.25),
    (6, 0.12),
    (8, 0.08),
];
/// 空腔味：只有奇次谐波，经典的类单簧管音色。
pub const HOLLOW: WaveRecipe = &[
    (1, 1.0),
    (3, 0.33),
    (5, 0.2),
    (7, 0.14),
    (9, 0.11),
    (11, 0.09),
];
/// 人声味：第 3–5 谐波附近有共振峰式强调。
pub const VOCAL: WaveRecipe = &[
    (1, 1.0),
    (2, 0.4),
    (3, 0.9),
    (4, 0.7),
    (5, 0.6),
    (6, 0.2),
    (7, 0.1),
];
/// 金属味：稀疏的高谐波，比值略有拉伸。
pub const METALLIC: WaveRecipe = &[
    (1, 1.0),
    (3, 0.5),
    (5, 0.4),
    (8, 0.3),
    (11, 0.25),
    (15, 0.2),
    (19, 0.15),
];
/// 玻璃味：明亮、像钟一样一直往上走的频谱。
pub const GLASS: WaveRecipe = &[
    (1, 1.0),
    (2, 0.2),
    (5, 0.5),
    (9, 0.35),
    (14, 0.25),
    (20, 0.18),
    (27, 0.12),
    (35, 0.08),
];

/// 全部工厂配方（名字 + 配方），供 UI 枚举。
pub const FACTORY_RECIPES: [(&str, WaveRecipe); 5] = [
    ("organ", ORGAN),
    ("hollow", HOLLOW),
    ("vocal", VOCAL),
    ("metallic", METALLIC),
    ("glass", GLASS),
];

/// 导入周期无法变成波表的原因。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WavetableError {
    /// 样本数少于 [`MIN_CYCLE`]。
    TooShort,
    /// 含 `NaN` 或无穷。
    NotFinite,
    /// 去掉直流之后什么都不剩。
    Silent,
}

/// 某个**回放频率**在给定采样率下应选的 mip 级号 —— 与 [`Wavetable::level_for`]
/// 是**同一条规则**，只是不需要那张表。
///
/// 存在的理由只有一个：[`crate::polysynth::PolySynth::set_sample_rate`] 必须在
/// **没有波表库**的情况下把在响声部的 mip 级重选到新采样率上（那个方法的签名里
/// 没有库，而库由调用方持有）。选级规则本身只依赖 [`LEVELS`] 与 [`level_top`]
/// （都是编译期常量），因此可以独立成函数；[`Wavetable::level_for`] 现在**委派**
/// 到本函数，两处因此不可能漂移（判据
/// [`tests::level_for_freq_agrees_with_the_wavetable`] 逐频点对账）。
///
/// 判据规则（与 `Wavetable::level_for` 逐字相同）：谐波上限 `h` 满足 `h · f ≤ Nyquist`
/// 的**最高**（最长）一级；没有一级满足时退到最短一级。
#[inline]
#[must_use]
pub fn level_for_freq(freq_hz: f32, sample_rate: f32) -> usize {
    let nyquist = sanitise_sample_rate(sample_rate) * 0.5;
    let mut index = LEVELS - 1;
    for level in 0..LEVELS {
        // 该级含到它自己那个倍频程 Nyquist 为止的谐波；音符的第 `h` 次谐波
        // 位于 `h * freq`，因此在 `h * freq <= nyq` 时放得下。
        let max_harmonic = level_top(level) as f32;
        if freq_hz * max_harmonic <= nyquist {
            index = level;
            break;
        }
    }
    index
}

/// 一个波形的全部 mip 级。级按"最长在前"的顺序存储（所有级等长）。
///
/// `Vec` 只在构造期分配，`process*` 路径上不再触碰堆 [ARCH-RT-001]。
#[derive(Clone, Debug)]
pub struct Wavetable {
    levels: Vec<Vec<f32>>,
}

/// mip `level` 的谐波上限：`BASE_LEN >> level` 个谐波，也就是在该级被选中播放
/// 的音高下仍能落在 Nyquist 之下的内容。级的**长度**不再参与这件事：每一级都是
/// [`BASE_LEN`] 点。
///
/// 注意那个 `/ 2`：含 `h` 个谐波的级在 `h <= SR / (2f)` 时才是安全读取的，
/// 而这正是 [`Wavetable::level_for`] 检查的内容（引擎把 `f` 钳到
/// `SR / (BASE_LEN >> level)`）。因此这里的谐波**数**是 `(BASE_LEN >> level) / 2`
/// 而不是 `BASE_LEN >> level`：相对该钳制留一个倍频程的余量。没有它，第 8 级会含
/// 8 个谐波，而 C8（4186 Hz）的第 6–8 谐波会折回到 22.9/18.7/14.5 kHz。
#[inline]
fn level_top(level: usize) -> u32 {
    ((BASE_LEN >> level) / 2).max(1) as u32
}

impl Wavetable {
    /// 由配方构建 mipmap：每一级都是 [`BASE_LEN`] 点，只含到它自己那个倍频程
    /// Nyquist 为止的谐波。
    #[must_use]
    pub fn from_recipe(recipe: WaveRecipe) -> Self {
        let mut levels = Vec::with_capacity(LEVELS);
        for level in 0..LEVELS {
            levels.push(render_level(recipe, BASE_LEN, level_top(level)));
        }
        Self { levels }
    }

    /// 由一段"一个周期的样本"构建 mipmap。
    ///
    /// 周期被重采样到 [`BASE_LEN`]、去掉直流，一次变换给出全部谐波内容。
    /// 随后每一级由低于其自身 Nyquist 的 bin 重建——直流与其上的一切被丢弃，
    /// 其余分毫不动——因此这些级既是砖墙限带的，**又保留了原始相位**，这正是
    /// 让导入的锯齿仍然看起来像锯齿、而不是一堆余弦的原因。每一级单独做峰值
    /// 归一化，与配方表完全一致，因此音高上升时切换级不会跳响度。
    ///
    /// # 错误
    ///
    /// 见 [`WavetableError`]：过短、含非有限值、去直流后静音。
    pub fn from_cycle(cycle: &[f32]) -> Result<Self, WavetableError> {
        if cycle.len() < MIN_CYCLE {
            return Err(WavetableError::TooShort);
        }
        if cycle.iter().any(|value| !value.is_finite()) {
            return Err(WavetableError::NotFinite);
        }

        let n = BASE_LEN;
        let mut re = vec![0.0f64; n];
        let mut im = vec![0.0f64; n];
        let step = cycle.len() as f64 / n as f64;
        for (index, slot) in re.iter_mut().enumerate() {
            let position = index as f64 * step;
            let first = (position.floor() as usize) % cycle.len();
            let second = (first + 1) % cycle.len();
            let fraction = position - position.floor();
            *slot =
                f64::from(cycle[first]) * (1.0 - fraction) + f64::from(cycle[second]) * fraction;
        }

        let mean = re.iter().sum::<f64>() / n as f64;
        for value in re.iter_mut() {
            *value -= mean;
        }
        let rms = (re.iter().map(|value| value * value).sum::<f64>() / n as f64).sqrt();
        if rms <= 1e-4 {
            return Err(WavetableError::Silent);
        }

        fft(&mut re, &mut im, false);

        let mut levels = Vec::with_capacity(LEVELS);
        for level in 0..LEVELS {
            let mut level_re = re.clone();
            let mut level_im = im.clone();
            // 谐波 1..=top 存活；`bin` 把上半谱折叠到它的镜像上，于是一次判断
            // 覆盖两侧。
            let top = level_top(level);
            for (k, (r, i)) in level_re.iter_mut().zip(level_im.iter_mut()).enumerate() {
                let bin = if k <= n / 2 { k } else { n - k };
                if bin == 0 || bin as u32 > top {
                    *r = 0.0;
                    *i = 0.0;
                }
            }
            fft(&mut level_re, &mut level_im, true);

            // 每一级都保持全表长：逆变换**就是**这一级，没有抽取。这就是 P9.7 的
            // 修法——每输出样本的读取步进是 `freq / sample_rate`，而"长表装少量
            // 谐波"才是弦误差小的原因（见模块注释里关于 `BASE_LEN` 的说明）。
            let mut out: Vec<f32> = level_re.iter().map(|value| *value as f32).collect();
            let peak = out.iter().fold(0.0f32, |peak, value| peak.max(value.abs()));
            if peak > 0.0 {
                let gain = 1.0 / peak;
                for value in out.iter_mut() {
                    *value *= gain;
                }
            }
            levels.push(out);
        }
        Ok(Self { levels })
    }

    /// 某个回放频率应选的级号：谐波仍能落在 Nyquist 之下的**最高**（最长）一级。
    ///
    /// 规则本体在 [`level_for_freq`]（那里说明了为什么它必须能脱离表存在），本方法
    /// 只是把它钳进这张表自己的级数 —— 由 [`Self::from_recipe`] /
    /// [`Self::from_cycle`] 建出来的表恒有 [`LEVELS`] 级，因此这个钳制在既有构造
    /// 路径上是恒等映射（判据 [`tests::level_for_freq_agrees_with_the_wavetable`]）。
    #[must_use]
    pub fn level_for(&self, freq_hz: f32, sample_rate: f32) -> usize {
        level_for_freq(freq_hz, sample_rate).min(self.levels.len().saturating_sub(1))
    }

    /// 第 `level` 级的表长（越界钳制）。
    #[must_use]
    pub fn level_len(&self, level: usize) -> usize {
        self.levels[level.min(self.levels.len().saturating_sub(1))].len()
    }

    /// 级数（恒为 [`LEVELS`]）。
    #[must_use]
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// 只读访问某一级的原始表（测试与离线分析用）。
    #[must_use]
    pub fn level_samples(&self, level: usize) -> &[f32] {
        &self.levels[level.min(self.levels.len().saturating_sub(1))]
    }

    /// 以 0..1 相位线性插值读取该级。
    #[inline]
    #[must_use]
    pub fn sample(&self, level: usize, phase: f32) -> f32 {
        let table = &self.levels[level.min(self.levels.len().saturating_sub(1))];
        let len = table.len();
        let position = phase.rem_euclid(1.0) * len as f32;
        let index = position as usize;
        let fraction = position - index as f32;
        let a = table[index % len];
        let b = table[(index + 1) % len];
        a + (b - a) * fraction
    }
}

/// 生成一级：配方的谐波直到 `top`，作为 `len` 点表上的正弦和。
fn render_level(recipe: WaveRecipe, len: usize, top: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; len];
    let mut peak = 0.0f32;
    for (harmonic, amplitude) in recipe.iter() {
        if *harmonic > top {
            // 更高的级干脆丢掉放不下的部分：这**就是**限带，也正是高音符
            // 不可能混叠的原因。
            continue;
        }
        for (index, value) in out.iter_mut().enumerate() {
            let phase = core::f32::consts::TAU * (*harmonic as f32) * index as f32 / len as f32;
            *value += amplitude * phase.sin();
        }
    }
    for value in out.iter() {
        peak = peak.max(value.abs());
    }
    if peak > 0.0 {
        let gain = 1.0 / peak;
        for value in out.iter_mut() {
            *value *= gain;
        }
    }
    out
}

/// LFO 波形。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LfoWave {
    /// 正弦。
    Sine,
    /// 三角。
    Triangle,
    /// 方波。
    Square,
    /// 锯齿。
    Saw,
}

/// 全局低频振荡器（每个实例一份状态，无全局变量）。
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    /// 相位，`0..1`；one-shot 结束后停在 1.0。
    phase: f32,
    /// 最后一个输出值（逐块消费者读它）。
    pub value: f32,
    /// one-shot 模式跑完一个周期就停下，而不是循环。
    pub one_shot: bool,
}

impl Lfo {
    /// 构造：相位 0、输出 0、循环模式。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: 0.0,
            value: 0.0,
            one_shot: false,
        }
    }

    /// 复位到周期起点。
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.value = 0.0;
    }

    /// 重启周期（逐音符重触发模式使用）。
    pub fn retrigger(&mut self) {
        self.phase = 0.0;
    }

    /// 当前相位。
    #[must_use]
    pub const fn phase(&self) -> f32 {
        self.phase
    }

    /// 按波形求 `phase` 处的值。
    #[inline]
    #[must_use]
    pub fn shape(wave: LfoWave, phase: f32) -> f32 {
        match wave {
            LfoWave::Sine => (phase * core::f32::consts::TAU).sin(),
            LfoWave::Triangle => {
                if phase < 0.25 {
                    phase * 4.0
                } else if phase < 0.75 {
                    2.0 - phase * 4.0
                } else {
                    phase * 4.0 - 4.0
                }
            }
            LfoWave::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoWave::Saw => phase * 2.0 - 1.0,
        }
    }

    /// 推进一个样本并返回新值。
    ///
    /// one-shot 模式在跑完一个周期后**停在** `phase = 1.0` 并一直输出该处的值，
    /// 因此它表现得像一个额外的包络。
    #[inline]
    pub fn process(&mut self, wave: LfoWave, rate_hz: f32, sample_rate: f32) -> f32 {
        let sample_rate = sanitise_sample_rate(sample_rate);
        let increment = if rate_hz.is_finite() {
            (rate_hz / sample_rate).clamp(0.0, 0.5)
        } else {
            0.0
        };
        let value = Self::shape(wave, self.phase);
        self.phase += increment;
        if self.phase >= 1.0 {
            self.phase = if self.one_shot { 1.0 } else { self.phase - 1.0 };
        }
        self.value = value;
        value
    }

    /// 用 LFO 形状填满 `out`，并记住最后一个值，供逐块消费者读 [`Self::value`]。
    pub fn render(&mut self, wave: LfoWave, rate_hz: f32, sample_rate: f32, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = self.process(wave, rate_hz, sample_rate);
        }
    }
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new()
    }
}

/// 单表限带振荡器：相位累加 + 按音高选 mip 级。
///
/// 表由调用方以只读引用传入（因此多个声部共用一份表，**不做** `Clone`——
/// `Wavetable::clone` 会复制 `Vec`，那是逐声部分配）。
#[derive(Clone, Copy, Debug)]
pub struct WavetableOscillator {
    phase: f32,
    frequency: f32,
    level: usize,
    sample_rate: f32,
}

impl WavetableOscillator {
    /// 以给定采样率构造（默认 A4 = 440 Hz）。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sanitise_sample_rate(sample_rate);
        Self {
            phase: 0.0,
            frequency: 440.0,
            level: 0,
            sample_rate,
        }
    }

    /// 设置采样率（级号随之重算）。
    pub fn set_sample_rate(&mut self, sample_rate: f32, table: &Wavetable) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.reselect_level(table);
        }
    }

    /// 设置频率（Hz），并按该音高重选 mip 级。
    ///
    /// 频率被钳到 `[0, 0.45·fs]`：Nyquist 之上没有可回放的内容，
    /// 而 0.45 与滤波器模块的天花板一致。
    pub fn set_frequency(&mut self, table: &Wavetable, frequency_hz: f32) {
        let frequency = if frequency_hz.is_finite() {
            frequency_hz.clamp(0.0, self.sample_rate * 0.45)
        } else {
            0.0
        };
        if frequency != self.frequency {
            self.frequency = frequency;
            self.reselect_level(table);
        }
    }

    /// 当前频率。
    #[must_use]
    pub const fn frequency(&self) -> f32 {
        self.frequency
    }

    /// 当前选中的 mip 级。
    #[must_use]
    pub const fn level(&self) -> usize {
        self.level
    }

    /// 当前相位。
    #[must_use]
    pub const fn phase(&self) -> f32 {
        self.phase
    }

    /// 直接设置相位（复位到周期起点、或做相位对齐时使用）。
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// 复位相位。
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// 推进一个样本。
    #[inline]
    pub fn process(&mut self, table: &Wavetable) -> f32 {
        let value = table.sample(self.level, self.phase);
        self.phase += self.frequency / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
        }
        value
    }

    /// 渲染一个块（原地）。
    pub fn process_block(&mut self, table: &Wavetable, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = self.process(table);
        }
    }

    fn reselect_level(&mut self, table: &Wavetable) {
        self.level = table.level_for(self.frequency, self.sample_rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------- 波表

    /// 高于某频率的能量，用对该级自身的直接 DFT 测量。
    fn energy_above(table: &[f32], hz_above: f32, sample_rate: f32) -> f32 {
        let n = table.len();
        let mut energy = 0.0f32;
        let mut probe = hz_above;
        while probe < sample_rate * 0.5 {
            let w = core::f32::consts::TAU * probe / sample_rate;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (index, value) in table.iter().enumerate() {
                re += value * (w * index as f32).cos();
                im -= value * (w * index as f32).sin();
            }
            energy += (re * re + im * im) / (n * n) as f32;
            probe += sample_rate / n as f32;
        }
        energy
    }

    #[test]
    fn every_level_is_band_limited_to_its_own_nyquist() {
        // 如果某一级含高于它自己 Nyquist 的谐波，用它被选中时的音高播放就会混叠。
        // 量出来：该级的频谱在 `len / 2` 个谐波之上必须是空的。
        for recipe in FACTORY_RECIPES {
            let table = Wavetable::from_recipe(recipe.1);
            for level in 0..LEVELS {
                let len = table.level_len(level);
                let content = table.level_samples(level);
                // 存在的谐波都在 `len / 2` 个表内周期之下；对该表自身周期做 DFT
                // 会在整数 bin 上看到它们。
                let energy = energy_above(content, len as f32 * 0.5, len as f32);
                assert!(
                    energy < 1e-6,
                    "{} level {level} has energy above its Nyquist: {energy:e}",
                    recipe.0
                );
            }
        }
    }

    #[test]
    fn every_level_is_full_length_and_still_normalised() {
        let table = Wavetable::from_recipe(GLASS);
        for level in 0..LEVELS {
            // P9.7：级不再变短——长度才是让读取步进变小的东西。
            // 只有谐波上限随级下降。
            assert_eq!(table.level_len(level), BASE_LEN);
        }
        for level in 0..LEVELS {
            let peak = (0..1024)
                .map(|i| table.sample(level, i as f32 / 1024.0).abs())
                .fold(0.0f32, f32::max);
            assert!((peak - 1.0).abs() < 0.15, "level {level} peak {peak}");
        }
        assert_eq!(table.level_count(), LEVELS);
    }

    #[test]
    fn the_level_chosen_for_a_note_cannot_alias() {
        let table = Wavetable::from_recipe(GLASS);
        for freq in [55.0f32, 110.0, 440.0, 880.0, 2093.0, 4186.0] {
            let level = table.level_for(freq, 48_000.0);
            // 该级存活下来的最高谐波是 `level_top(level)`，而这个分音在该音高下
            // 必须仍在 Nyquist 之下——这正是 mipmap 的全部意义。
            let top = level_top(level) as f32 * freq;
            assert!(
                top <= 48_000.0 * 0.5,
                "{freq} Hz picked level {level} whose top harmonic is {top} Hz"
            );
            // 而且它必须是满足该条件的**最长**一级，否则音符会无谓地发闷。
            if level > 0 {
                let longer = level_top(level - 1) as f32 * freq;
                assert!(
                    longer > 48_000.0 * 0.5,
                    "{freq} Hz could have used level {} ({longer} Hz top)",
                    level - 1
                );
            }
        }
        // 键盘最顶端必须退到最短一级。
        assert_eq!(table.level_for(4186.0, 48_000.0), LEVELS - 1);
    }

    /// **判据（新写，可红）**：[`level_for_freq`] 满足选级规则本身，且与每张表自己的
    /// [`Wavetable::level_for`] 在频点网格上**逐点同解**。
    ///
    /// 量什么：① 与一条**独立参照**的差（单位：级号）—— 参照在测试里用
    /// `Iterator::position` 从一张**独立构造**的谐波上限表上取"升序第一个放得下的级"，
    /// 没有一级放得下时取 [`LEVELS`] − 1；② 选中级的**形态**两条：放得下
    ///（或是退路的那一级）、且任何更长的级都放不下；③ 与表自己的读数之差。
    ///
    /// 为什么三条都要：单看 ③ 会被"两侧同时改错"骗过 —— 表自己的
    /// [`Wavetable::level_for`] 现在**委派**到本函数，两边同错时 ③ 仍相等
    ///（本机实测：把 `LEVELS - 1` 改成 `LEVELS - 2` 时 ③ 全绿）。① 是第二份算式，
    /// ② 是**不依赖任何算式**的性质，两者一起才把读数钉死。
    ///
    /// 注入（本机实测）：把 [`level_for_freq`] 的 `LEVELS - 1` 起点改成 `LEVELS - 2`
    /// ⇒ ① 在"没有一级放得下"的频点（20 kHz @ 8 kHz）变红；把它内部的
    /// `level_top(level) as f32` 乘 2 ⇒ ② 变红。
    ///
    /// 覆盖：全部 5 条工厂配方 + 一条导入周期（`from_cycle`）的表，
    /// 频率 20 Hz…20 kHz 的对数网格与 4 档采样率。
    #[test]
    fn level_for_freq_agrees_with_the_wavetable() {
        let imported: Vec<f32> = (0..BASE_LEN)
            .map(|index| (core::f64::consts::TAU * index as f64 / BASE_LEN as f64).sin() as f32)
            .collect();
        let mut tables: Vec<Wavetable> = FACTORY_RECIPES
            .iter()
            .map(|(_, recipe)| Wavetable::from_recipe(recipe))
            .collect();
        tables.push(Wavetable::from_cycle(&imported).expect("纯正弦周期"));

        // 独立参照表：各 mip 级的谐波上限（与 `level_for_freq` 的循环分离地构造一次）。
        let tops: Vec<f32> = (0..LEVELS).map(|level| level_top(level) as f32).collect();

        let mut checked = 0usize;
        let mut fallbacks = 0usize;
        for table in &tables {
            assert_eq!(
                table.level_count(),
                LEVELS,
                "本判据的前提是每张表恒有 {LEVELS} 级"
            );
            for sample_rate in [8_000.0f32, 44_100.0, 48_000.0, 96_000.0] {
                let nyquist = sample_rate * 0.5;
                for step in 0..64 {
                    // 20 Hz…20 kHz 的对数网格。
                    let freq = 20.0 * 10.0f32.powf(3.0 * step as f32 / 63.0);
                    let level = level_for_freq(freq, sample_rate);

                    // ① 与独立参照逐点同解。
                    let expected = tops
                        .iter()
                        .position(|top| freq * top <= nyquist)
                        .unwrap_or(LEVELS - 1);
                    assert_eq!(
                        level, expected,
                        "freq {freq} Hz @ {sample_rate} Hz：实得级号 {level}，独立参照 {expected}"
                    );

                    // ② 形态：放得下（或无可选时的退路），且没有更长的级放得下。
                    if freq * tops[LEVELS - 1] > nyquist {
                        fallbacks += 1;
                        assert_eq!(level, LEVELS - 1, "没有一级放得下时必须退到最短一级");
                    } else {
                        assert!(
                            freq * tops[level] <= nyquist,
                            "freq {freq} Hz @ {sample_rate} Hz：选中级号 {level} 放不下"
                        );
                        for (shorter, top) in tops.iter().enumerate().take(level) {
                            assert!(
                                freq * top > nyquist,
                                "freq {freq} Hz @ {sample_rate} Hz：更长的级 {shorter} 其实放得下"
                            );
                        }
                    }

                    // ③ 与表自己的读数同解（委派不应被改回第二份循环）。
                    assert_eq!(
                        level,
                        table.level_for(freq, sample_rate),
                        "freq {freq} Hz @ {sample_rate} Hz：自由函数与表的读数不一致"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, tables.len() * 4 * 64, "覆盖度不足");
        assert!(
            fallbacks > 0,
            "网格从没走到'没有一级放得下'的退路分支 ⇒ 那一支没有被覆盖"
        );
        // 退化频率：自由函数与表都不 panic，且读数落在级数之内。
        for freq in [0.0f32, -1.0, f32::NAN, f32::INFINITY, 1.0e9] {
            let level = level_for_freq(freq, 48_000.0);
            assert!(level < LEVELS, "freq {freq} ⇒ 级号 {level} 越界");
        }
    }

    #[test]
    fn sampling_wraps_and_interpolates() {
        let table = Wavetable::from_recipe(ORGAN);
        let at_zero = table.sample(0, 0.0);
        let at_one = table.sample(0, 1.0);
        assert!(
            (at_zero - at_one).abs() < 1e-5,
            "phase 1.0 should wrap to 0.0"
        );
        // 负相位与超过 1 的相位都要回绕而不是 panic。
        assert!(table.sample(0, -0.25).is_finite());
        assert!(table.sample(0, 3.75).is_finite());
        // 线性插值：中点落在两个邻居之间。
        let a = table.sample(0, 0.1000);
        let b = table.sample(0, 0.1005);
        let mid = table.sample(0, 0.100_25);
        assert!(mid <= a.max(b) + 1e-6 && mid >= a.min(b) - 1e-6);
    }

    // ------------------------------------------------------- 导入的单周期

    /// 表某一谐波的幅度，用对该表自身周期的直接 DFT 测量。
    fn harmonic_amplitude(table: &[f32], harmonic: usize) -> f32 {
        let n = table.len();
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (index, value) in table.iter().enumerate() {
            let phase = core::f64::consts::TAU * harmonic as f64 * index as f64 / n as f64;
            re += f64::from(*value) * phase.cos();
            im -= f64::from(*value) * phase.sin();
        }
        ((re * re + im * im).sqrt() / n as f64) as f32 * 2.0
    }

    fn sine_cycle(harmonics: &[(f32, f32)]) -> Vec<f32> {
        (0..BASE_LEN)
            .map(|index| {
                let phase = core::f32::consts::TAU * index as f32 / BASE_LEN as f32;
                harmonics
                    .iter()
                    .map(|(ratio, amplitude)| amplitude * (ratio * phase).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn an_imported_cycle_keeps_its_harmonic_balance() {
        let cycle = sine_cycle(&[(1.0, 1.0), (3.0, 0.5), (7.0, 0.25)]);
        let table = Wavetable::from_cycle(&cycle).expect("clean cycle");
        let level0 = table.level_samples(0);
        let first = harmonic_amplitude(level0, 1);
        assert!(first > 0.1, "fundamental vanished: {first}");
        assert!(
            (harmonic_amplitude(level0, 3) / first - 0.5).abs() < 0.02,
            "third harmonic changed level"
        );
        assert!(
            (harmonic_amplitude(level0, 7) / first - 0.25).abs() < 0.02,
            "seventh harmonic changed level"
        );
        // 谐波之间没有凭空造出内容。
        assert!(harmonic_amplitude(level0, 2) < first * 1e-3);
        assert!(harmonic_amplitude(level0, 5) < first * 1e-3);
    }

    #[test]
    fn an_imported_cycle_keeps_its_phase() {
        // 余弦与正弦的幅度谱相同，因此只保幅度的导入会通过上面的平衡测试，
        // 却播放错误的波形。对两者分别做相关。
        let cycle: Vec<f32> = (0..BASE_LEN)
            .map(|index| (core::f32::consts::TAU * index as f32 / BASE_LEN as f32).cos())
            .collect();
        let table = Wavetable::from_cycle(&cycle).expect("clean cycle");
        let (mut with_cos, mut with_sin) = (0.0f64, 0.0f64);
        for (index, value) in table.level_samples(0).iter().enumerate() {
            let phase = core::f64::consts::TAU * index as f64 / BASE_LEN as f64;
            with_cos += f64::from(*value) * phase.cos();
            with_sin += f64::from(*value) * phase.sin();
        }
        let n = BASE_LEN as f64;
        // 基频能量中落在余弦上的份额：与缩放无关，因此峰值归一化无法美化它。
        let share = with_cos / with_cos.hypot(with_sin);
        assert!(
            share > 0.99,
            "phase was not preserved, cosine share {share}"
        );
        assert!(
            with_cos.hypot(with_sin) * 2.0 / n > 0.98,
            "fundamental came back weak"
        );
    }

    #[test]
    fn an_imported_cycle_is_band_limited_at_every_level() {
        // 锯齿一直到顶都有谐波，因此每一级都有东西可丢。如果砖墙漏了，
        // 用它被选中时的音高播放就会混叠。
        let cycle: Vec<f32> = (0..BASE_LEN)
            .map(|index| {
                let phase = core::f32::consts::TAU * index as f32 / BASE_LEN as f32;
                (1..=BASE_LEN / 2)
                    .map(|k| (k as f32 * phase).sin() / k as f32)
                    .sum()
            })
            .collect();
        let table = Wavetable::from_cycle(&cycle).expect("clean cycle");
        for level in 0..LEVELS {
            let len = table.level_len(level);
            let content = table.level_samples(level);
            let energy = energy_above(content, len as f32 * 0.5, len as f32);
            assert!(
                energy < 1e-6,
                "level {level} leaked above its Nyquist: {energy:e}"
            );
        }
    }

    #[test]
    fn an_imported_cycle_is_normalised_and_dc_free() {
        let cycle = sine_cycle(&[(1.0, 0.4)]);
        let offset: Vec<f32> = cycle.iter().map(|value| value + 0.7).collect();
        let table = Wavetable::from_cycle(&offset).expect("clean cycle");
        for level in 0..LEVELS {
            let peak = table
                .level_samples(level)
                .iter()
                .fold(0.0f32, |peak, value| peak.max(value.abs()));
            assert!((peak - 1.0).abs() < 1e-3, "level {level} peak {peak}");
        }
        let mean = table.level_samples(0).iter().sum::<f32>() / BASE_LEN as f32;
        assert!(mean.abs() < 1e-3, "DC offset survived: {mean}");
    }

    #[test]
    fn a_short_cycle_is_resampled_rather_than_refused() {
        // 文件里的周期不要求正好 2048 点。
        let cycle: Vec<f32> = (0..64)
            .map(|index| (core::f32::consts::TAU * index as f32 / 64.0).sin())
            .collect();
        let table = Wavetable::from_cycle(&cycle).expect("64-sample cycle");
        let first = harmonic_amplitude(table.level_samples(0), 1);
        assert!(first > 0.9, "fundamental lost in resampling: {first}");
        for harmonic in 2..12 {
            assert!(
                harmonic_amplitude(table.level_samples(0), harmonic) < first * 0.05,
                "resampling invented harmonic {harmonic}"
            );
        }
    }

    #[test]
    fn unusable_cycles_are_rejected_with_a_reason() {
        assert_eq!(
            Wavetable::from_cycle(&[0.0; MIN_CYCLE - 1]).err(),
            Some(WavetableError::TooShort)
        );
        assert_eq!(
            Wavetable::from_cycle(&[0.0; BASE_LEN]).err(),
            Some(WavetableError::Silent)
        );
        assert_eq!(
            Wavetable::from_cycle(&[0.25; BASE_LEN]).err(),
            Some(WavetableError::Silent),
            "a constant has no waveform once DC is removed"
        );
        let mut spike = vec![0.0f32; BASE_LEN];
        spike[3] = f32::NAN;
        assert_eq!(
            Wavetable::from_cycle(&spike).err(),
            Some(WavetableError::NotFinite)
        );
        let mut huge = vec![0.0f32; BASE_LEN];
        huge[7] = f32::INFINITY;
        assert_eq!(
            Wavetable::from_cycle(&huge).err(),
            Some(WavetableError::NotFinite)
        );
    }

    // ------------------------------------------------------------ 振荡器

    /// 手写加窗 DFT，返回 `freqs` 上每个频点的幅度。
    fn spectrum(data: &[f32], freqs: &[f32], sample_rate: f32) -> Vec<f64> {
        let n = data.len();
        freqs
            .iter()
            .map(|f| {
                let w = core::f64::consts::TAU * f64::from(*f) / f64::from(sample_rate);
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, v) in data.iter().enumerate() {
                    // Hann 窗，减少泄漏。
                    let win = 0.5 - 0.5 * (core::f64::consts::TAU * i as f64 / n as f64).cos();
                    re += f64::from(*v) * win * (w * i as f64).cos();
                    im -= f64::from(*v) * win * (w * i as f64).sin();
                }
                (re * re + im * im).sqrt() / n as f64
            })
            .collect()
    }

    /// 把非谐波频点的能量与基频比较（dB）。混叠会在这些频点上留下能量。
    fn non_harmonic_energy_db(table: &Wavetable, level: usize, freq: f32, sr: f32) -> f64 {
        let n = 8192;
        let mut data = vec![0.0f32; n];
        let mut phase = 0.0f32;
        for sample in data.iter_mut() {
            *sample = table.sample(level, phase);
            phase += freq / sr;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
        // 候选频点：每一个不是 `freq` 整数倍的位置（避开真谐波 ±40 Hz）。
        let mut probes = Vec::new();
        let mut hz = 60.0f32;
        while hz < sr * 0.5 - 60.0 {
            let ratio = hz / freq;
            let distance = (ratio - ratio.round()).abs() * freq;
            if distance > 40.0 {
                probes.push(hz);
            }
            hz += 60.0;
        }
        let magnitudes = spectrum(&data, &probes, sr);
        let junk: f64 = magnitudes.iter().map(|m| m * m).sum::<f64>().sqrt();
        let fundamental = spectrum(&data, &[freq], sr)[0];
        20.0 * (junk / fundamental).log10()
    }

    /// **判据（新写，可红）**：端到端的抗混叠保证。
    ///
    /// 键盘顶端附近的音（C8 = 4186 Hz）在 48 kHz 下播放时，输出里**只能**有它的
    /// 谐波序列：任何非谐波频点的能量必须比基频低 40 dB 以上。
    ///
    /// 对照同时给出：同一张表强行用第 0 级（256 个谐波）播放时，非谐波能量会
    /// 高出 20 dB 以上——这证明这条测量真的能看见混叠。
    /// 若把 `WavetableOscillator::reselect_level` 改成固定 `level = 0`，
    /// 本测试立即变红。
    #[test]
    fn a_high_note_has_no_energy_outside_its_harmonics() {
        let sr = 48_000.0f32;
        let frequency = 4186.0f32;
        let table = Wavetable::from_recipe(GLASS);

        let mut oscillator = WavetableOscillator::new(sr);
        oscillator.set_frequency(&table, frequency);
        // 选级必须真的是"该音高下的最高可用级"。
        assert_eq!(oscillator.level(), table.level_for(frequency, sr));

        let clean = non_harmonic_energy_db(&table, oscillator.level(), frequency, sr);
        assert!(
            clean < -40.0,
            "band-limited playback leaked {clean:.1} dB outside the harmonics"
        );

        // 对照：同样的音高、同样的表，但拒绝降级到 mip 级。
        let aliased = non_harmonic_energy_db(&table, 0, frequency, sr);
        assert!(
            aliased > clean + 20.0,
            "the control did not alias ({aliased:.1} dB vs {clean:.1} dB); \
             the measurement cannot see aliasing"
        );
    }

    #[test]
    fn the_oscillator_wraps_phase_and_stays_bounded() {
        let table = Wavetable::from_recipe(ORGAN);
        let mut oscillator = WavetableOscillator::new(48_000.0);
        oscillator.set_frequency(&table, 440.0);
        assert_eq!(oscillator.frequency(), 440.0);
        let mut block = [0.0f32; 4096];
        oscillator.process_block(&table, &mut block);
        for (index, value) in block.iter().enumerate() {
            assert!(
                value.is_finite() && value.abs() <= 1.0001,
                "at {index}: {value}"
            );
        }
        assert!((0.0..1.0).contains(&oscillator.phase()));
        // 频率被钳到 0.45·fs：Nyquist 之上没有可回放的内容。
        oscillator.set_frequency(&table, 1.0e9);
        assert!(oscillator.frequency() <= 48_000.0 * 0.45);
        oscillator.set_frequency(&table, f32::NAN);
        assert_eq!(oscillator.frequency(), 0.0);
        // 换采样率必须重选级。
        let before = oscillator.level();
        oscillator.set_frequency(&table, 2093.0);
        oscillator.set_sample_rate(96_000.0, &table);
        assert!(oscillator.level() <= before.max(table.level_for(2093.0, 96_000.0)));
        assert_eq!(oscillator.level(), table.level_for(2093.0, 96_000.0));
    }

    // ---------------------------------------------------------------- LFO

    #[test]
    fn sine_stays_in_range_and_advances_phase() {
        let mut lfo = Lfo::new();
        let mut buf = [0.0f32; 512];
        lfo.render(LfoWave::Sine, 1.0, 48000.0, &mut buf);
        for v in buf {
            assert!((-1.0..=1.0).contains(&v));
        }
        // 1 Hz 跑一秒应该大致完成一个周期。
        assert!(lfo.phase() > 0.0 && lfo.phase() < 0.05);
    }

    #[test]
    fn square_is_bipolar() {
        // 采样率被钳到 [`crate::MIN_SAMPLE_RATE`]（1 kHz），因此这里用
        // "每样本 ¼ 周期"（250 Hz @1 kHz）复现来源的 2 Hz @8 Hz 步进。
        let mut lfo = Lfo::new();
        let mut buf = [0.0f32; 4];
        lfo.render(LfoWave::Square, 250.0, 1_000.0, &mut buf);
        assert_eq!(buf, [1.0, 1.0, -1.0, -1.0]);
    }

    /// **判据（新写，可红）**：one-shot 跑完一个周期后停住并保持。
    ///
    /// 来源只在注释里声称这个行为，没有测试。断言三件事：相位停在 1.0、
    /// 之后的块不再前进、输出恒等于该点的形状值。
    ///
    /// 采样率被钳到 1 kHz、相位步进被钳到 0.5，因此"两个样本走完一个周期"
    /// 就是这条路径能表达的最快 one-shot。
    ///
    /// 若把 `process` 里的 `phase = if one_shot { 1.0 } else { ... }` 改成
    /// 永远回绕，相位会继续循环，本测试立即变红。
    #[test]
    fn one_shot_parks_at_the_end_of_its_cycle() {
        let mut lfo = Lfo::new();
        lfo.one_shot = true;
        // 步进 0.5：两个样本恰好走完一个周期。
        let mut first = [0.0f32; 2];
        lfo.render(LfoWave::Saw, 500.0, 1_000.0, &mut first);
        assert_eq!(lfo.phase(), 1.0, "one-shot must park at the cycle end");
        let parked = Lfo::shape(LfoWave::Saw, 1.0);
        let mut second = [0.0f32; 8];
        lfo.render(LfoWave::Saw, 500.0, 1_000.0, &mut second);
        assert_eq!(lfo.phase(), 1.0, "one-shot advanced again");
        for v in second {
            assert_eq!(v, parked, "one-shot output drifted from its parked value");
        }
        // 循环模式下，同样两段渲染必须回绕而不是停在 1.0：第二个块的首样本
        // 是周期起点的形状值，而不是 one-shot 停住的那个值。
        let mut looping = Lfo::new();
        let mut buf = [0.0f32; 2];
        looping.render(LfoWave::Saw, 500.0, 1_000.0, &mut buf);
        assert_eq!(looping.phase(), 0.0, "looping LFO should wrap, not park");
        let mut more = [0.0f32; 2];
        looping.render(LfoWave::Saw, 500.0, 1_000.0, &mut more);
        assert_eq!(more[0], Lfo::shape(LfoWave::Saw, 0.0));
        assert_ne!(more[0], parked, "looping LFO behaved like a one-shot");
    }

    #[test]
    fn lfo_shapes_are_bounded_and_retriggerable() {
        for wave in [
            LfoWave::Sine,
            LfoWave::Triangle,
            LfoWave::Square,
            LfoWave::Saw,
        ] {
            let mut lfo = Lfo::new();
            for step in 0..1000 {
                let value = lfo.process(wave, 3.0, 48_000.0);
                assert!(
                    value.is_finite() && (-1.0..=1.0).contains(&value),
                    "{wave:?} at {step}"
                );
            }
            lfo.retrigger();
            assert_eq!(lfo.phase(), 0.0);
            lfo.reset();
            assert_eq!(lfo.value, 0.0);
        }
        // 退化采样率与速率不得产生 NaN。
        let mut lfo = Lfo::new();
        for rate in [0.0f32, -1.0, f32::NAN, 1.0e9] {
            for sr in [0.0f32, f32::NAN, 48_000.0] {
                assert!(lfo.process(LfoWave::Sine, rate, sr).is_finite());
            }
        }
    }
}
