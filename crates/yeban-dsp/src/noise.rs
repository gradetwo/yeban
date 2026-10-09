//! 确定性噪声源与 xorshift RNG。[ARCH-DET-001] [ARCH-RT-001]
//!
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)
//! `src/dsp/noise.rs`（噪声颜色）+ `src/dsp/util.rs` 的 `Rng`（确定性随机源）。
//!
//! 白噪声按赫兹是平坦的；粉噪声每倍频程降 3 dB、棕噪声降 6 dB，这正是它们听感
//! 更"柔"的原因，也是每台模拟合成器都有它们的原因。两者都是"白噪声过一阶滤波"，
//! 因此每个振荡器只需要很少的状态。
//!
//! 噪声滤波器的转折频率**按采样率缩放**：固定 Hz 的转折点会让音色随设备采样率
//! 改变，那是不可接受的（同一工程在不同声卡上必须同色）[ARCH-DET-001]。
//!
//! ## 与来源的差异
//!
//! - `Rng` 从 `util.rs` 移到这里，并且**只保留显式播种**这一种构造方式：
//!   夜半不允许隐式全局熵源，跨运行/跨机器必须同序 [ARCH-DET-001]。
//! - [`NoiseGen::process`] 仍由调用方提供白噪声样本，RNG 的所有权留在调用方，
//!   因此"噪声"与"随机数发生器"可以各自独立测试。
//!
//! ## 非有限输入样本（本轮补齐）
//!
//! [`NoiseGen::process`] 的 `white` 实参是**调用方给的样本**，而粉/棕两条颜色的
//! 状态是**递归**的：`pink[k] = a·pink[k] + b·white`、`brown = (brown + 0.05·white)·leak`。
//! 一个 `NaN`／`±∞` 会被原样留在状态里（`NaN · 0.99765 = NaN`），此后每一次输出
//! 都非有限，**输入恢复干净也回不来** —— [`NoiseGen::reset`] 是唯一出路。
//! 实时路径上无法报错，只能在入口回落。
//!
//! ⇒ [`NoiseGen::process`] 在滤波之前过 `math::finite_or_zero`：`NaN` 与 `±∞`
//! 归 `0.0`，**有限样本逐位不变**（既有判据一个比特都不改）。白噪声档是纯直通，
//! 归零后输出 `+0.0`，与"把那个样本换成 `0.0`"的对照运行逐位相同。
//!
//! ⚠ 守卫**不**覆盖"有限但极大"的输入（`3e38` 仍可能溢出成 `∞`）；那条归调用方
//! 的噪声幅度口径管（同 `math::finite_or_zero` 的文档）。

use crate::math::finite_or_zero;

/// xorshift32 —— 无分配、跨运行确定。
///
/// 周期 2³²−1，全零状态是唯一的不动点，因此构造时把 0 替换成非零常数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    state: u32,
}

impl Rng {
    /// 全零状态的替代种子（黄金比例常数，非 0）。
    const FALLBACK_SEED: u32 = 0x9e37_79b9;

    /// 用给定种子构造；`seed == 0` 会被替换成非零常数。
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { Self::FALLBACK_SEED } else { seed },
        }
    }

    /// 当前内部状态（用于把 RNG 状态存档/回放以复现一段渲染）。
    #[must_use]
    pub const fn state(&self) -> u32 {
        self.state
    }

    /// 下一个 32 位值。
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// `[-1, 1)` 上的均匀分布。
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// `[0, 1)` 上的均匀分布。
    #[inline]
    pub fn next_unit(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

/// 噪声颜色。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseColour {
    /// 白噪声：按赫兹平坦。
    White,
    /// 粉噪声：−3 dB/倍频程。
    Pink,
    /// 棕噪声：−6 dB/倍频程。
    Brown,
}

/// 有色噪声发生器（每个声部一个实例，无共享状态）。
#[derive(Clone, Copy, Debug)]
pub struct NoiseGen {
    colour: NoiseColour,
    /// 粉噪声：Paul Kellet 三极点近似，10 Hz–20 kHz 内跟踪 −3 dB/倍频程
    /// 误差约 0.05 dB。
    pink: [f32; 3],
    /// 棕噪声：白噪声的漏积分器。
    brown: f32,
    /// 粉/棕滤波器转折频率（Hz）。
    corner_hz: f32,
}

impl NoiseGen {
    /// 构造一个白噪声发生器（默认转折频率 5 Hz）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            colour: NoiseColour::White,
            pink: [0.0; 3],
            brown: 0.0,
            // 5 Hz：泄漏只是为了防止随机游走漂到直流。一阶转折放在 20 Hz 时
            // −6 dB 斜率到 250 Hz 还没成型（实测 −4.2 dB/倍频程）；把它压低
            // 才能让渐近斜率覆盖整个可听频段。
            corner_hz: 5.0,
        }
    }

    /// 切换噪声颜色（不清状态；需要干净起点请再调用 [`Self::reset`]）。
    pub fn set_colour(&mut self, colour: NoiseColour) {
        self.colour = colour;
    }

    /// 当前颜色。
    #[must_use]
    pub const fn colour(&self) -> NoiseColour {
        self.colour
    }

    /// 设置漏积分器转折频率，钳制到 1–200 Hz。
    pub fn set_corner_hz(&mut self, hz: f32) {
        if hz.is_finite() {
            self.corner_hz = hz.clamp(1.0, 200.0);
        }
    }

    /// 清空滤波器状态（换音符/换色时使用）。
    pub fn reset(&mut self) {
        self.pink = [0.0; 3];
        self.brown = 0.0;
    }

    /// 输出当前颜色的一个样本。
    ///
    /// `white` 是调用方提供的 −1..1 白噪声源，因此 RNG 留在它自己的位置上；
    /// `sample_rate` 显式传入，用来把转折频率换算成系数。
    #[inline]
    pub fn process(&mut self, white: f32, sample_rate: f32) -> f32 {
        // 入口守卫：非有限样本会被粉/棕的递归状态永久留下（见模块文档）。
        // 有限样本逐位不变。
        let white = finite_or_zero(white);
        match self.colour {
            NoiseColour::White => white,
            NoiseColour::Pink => {
                // Paul Kellet 的改进法：三个转折频率铺开在频段上的单极点。
                self.pink[0] = 0.997_65 * self.pink[0] + white * 0.099_046;
                self.pink[1] = 0.963 * self.pink[1] + white * 0.296_516;
                self.pink[2] = 0.57 * self.pink[2] + white * 1.052_691;
                (self.pink[0] + self.pink[1] + self.pink[2] + white * 0.184_8) * 0.28
            }
            NoiseColour::Brown => {
                // 漏积分：泄漏阻止随机游走漂到直流，并按采样率缩放，
                // 因此颜色不随设备采样率改变。
                let sr = crate::math::sanitise_sample_rate(sample_rate);
                let leak = 1.0 - (core::f32::consts::TAU * self.corner_hz / sr);
                self.brown = (self.brown + white * 0.05) * leak.clamp(0.9, 0.999_9);
                self.brown * 3.5
            }
        }
    }
}

impl Default for NoiseGen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 以倍频程带能量算出的 dB/倍频程斜率（250 Hz–4 kHz，四个倍频程）。
    ///
    /// 在频域测量是**要点**：粉/棕是关于频谱的陈述，只看电平无法把它和白噪声区分开。
    fn slope(colour: NoiseColour) -> f32 {
        const SR: f32 = 48_000.0;
        const N: usize = 1 << 14;
        let mut noise = NoiseGen::new();
        noise.set_colour(colour);
        // 确定性来源，测试不依赖引擎 RNG。这里刻意保留来源的 LCG（而不是
        // 本 crate 的 `Rng`）：被测对象是噪声滤波器，白噪声序列必须与
        // 来源实测的那一条完全一致，否则容忍度就要重新标定。
        let mut state = 0x1234_5678u32;
        let mut re = vec![0.0f64; N];
        let mut im = vec![0.0f64; N];
        for (i, slot) in re.iter_mut().enumerate() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (state >> 8) as f32 / 8_388_608.0 - 1.0;
            let v = noise.process(white, SR) as f64;
            let win = 0.5 - 0.5 * (core::f64::consts::TAU * i as f64 / N as f64).cos();
            *slot = v * win;
        }
        // 手写迭代基 2 FFT（与 `math::fft` 相同算法的测试内副本，
        // 刻意不复用被测代码以避免"自己证明自己"）。
        let mut j = 0usize;
        for i in 1..N {
            let mut bit = N >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= N {
            let ang = -core::f64::consts::TAU / len as f64;
            let mut i = 0;
            while i < N {
                for k in 0..len / 2 {
                    let wr = (ang * k as f64).cos();
                    let wi = (ang * k as f64).sin();
                    let ur = re[i + k];
                    let ui = im[i + k];
                    let vr = re[i + k + len / 2] * wr - im[i + k + len / 2] * wi;
                    let vi = re[i + k + len / 2] * wi + im[i + k + len / 2] * wr;
                    re[i + k] = ur + vr;
                    im[i + k] = ui + vi;
                    re[i + k + len / 2] = ur - vr;
                    im[i + k + len / 2] = ui - vi;
                }
                i += len;
            }
            len <<= 1;
        }
        // 250 Hz–4 kHz 的每倍频程能量。
        let mut bands = [0.0f64; 4];
        let mut counts = [0usize; 4];
        for bin in 1..N / 2 {
            let hz = bin as f64 * SR as f64 / N as f64;
            if !(250.0..4000.0).contains(&hz) {
                continue;
            }
            let index = ((hz / 250.0).log2().floor() as usize).min(3);
            bands[index] += re[bin] * re[bin] + im[bin] * im[bin];
            counts[index] += 1;
        }
        for (band, count) in bands.iter_mut().zip(counts) {
            *band /= count.max(1) as f64;
        }
        // dB 对倍频程序号的最小二乘斜率。
        let db: Vec<f64> = bands.iter().map(|e| 10.0 * e.max(1e-30).log10()).collect();
        let mean_x = 1.5;
        let mean_y = db.iter().sum::<f64>() / 4.0;
        let mut num = 0.0;
        let mut den = 0.0;
        for (index, y) in db.iter().enumerate() {
            let x = index as f64 - mean_x;
            num += x * (y - mean_y);
            den += x * x;
        }
        (num / den) as f32
    }

    #[test]
    fn pink_falls_at_three_db_per_octave() {
        let measured = slope(NoiseColour::Pink);
        assert!(
            (measured + 3.0).abs() < 0.6,
            "pink slope should be about -3 dB/octave, measured {measured:.2}"
        );
    }

    #[test]
    fn brown_falls_at_six_db_per_octave() {
        let measured = slope(NoiseColour::Brown);
        assert!(
            (measured + 6.0).abs() < 0.8,
            "brown slope should be about -6 dB/octave, measured {measured:.2}"
        );
    }

    #[test]
    fn white_is_flat() {
        let measured = slope(NoiseColour::White);
        assert!(
            measured.abs() < 0.5,
            "white should be flat, measured {measured:.2}"
        );
    }

    #[test]
    fn stays_bounded() {
        let mut noise = NoiseGen::new();
        for colour in [NoiseColour::White, NoiseColour::Pink, NoiseColour::Brown] {
            noise.set_colour(colour);
            noise.reset();
            for i in 0..100_000 {
                let v = noise.process(if i % 2 == 0 { 1.0 } else { -1.0 }, 48_000.0);
                assert!(v.is_finite() && v.abs() < 20.0, "{colour:?} ran away: {v}");
            }
        }
    }

    #[test]
    fn rng_is_deterministic_and_bipolar() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..64 {
            let x = a.next_bipolar();
            assert_eq!(x, b.next_bipolar());
            assert!((-1.0..1.0).contains(&x));
        }
    }

    #[test]
    fn rng_never_gets_stuck_and_never_reaches_one() {
        // 种子 0 必须被替换，否则 xorshift 会永远停在 0。
        let mut rng = Rng::new(0);
        assert_ne!(rng.state(), 0);
        let mut seen_low = false;
        let mut seen_high = false;
        for _ in 0..100_000 {
            let unit = rng.next_unit();
            assert!((0.0..1.0).contains(&unit), "unit out of range: {unit}");
            assert_ne!(rng.state(), 0, "xorshift reached its fixed point");
            seen_low |= unit < 0.01;
            seen_high |= unit > 0.99;
        }
        assert!(
            seen_low && seen_high,
            "distribution is not covering the range"
        );
    }

    #[test]
    fn rng_state_can_be_restored_to_replay_a_render() {
        let mut original = Rng::new(0xdead_beef);
        for _ in 0..7 {
            original.next_u32();
        }
        let mut replay = Rng::new(original.state());
        for _ in 0..16 {
            assert_eq!(original.next_u32(), replay.next_u32());
        }
    }

    #[test]
    fn colour_switch_is_observable_and_corner_is_clamped() {
        let mut noise = NoiseGen::new();
        assert_eq!(noise.colour(), NoiseColour::White);
        noise.set_colour(NoiseColour::Pink);
        assert_eq!(noise.colour(), NoiseColour::Pink);
        // 退化输入不得污染状态：输出仍必须有限。
        noise.set_corner_hz(f32::NAN);
        noise.set_colour(NoiseColour::Brown);
        for sample in 0..4096 {
            let white = if sample % 2 == 0 { 1.0 } else { -1.0 };
            assert!(noise.process(white, 0.0).is_finite());
        }
    }
}
