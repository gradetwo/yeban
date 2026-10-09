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

    /// **判据（新写，可红）**：转折频率的**下界 1 Hz** 被钉住 —— 低于下界的请求与
    /// 请求下界本身给出**逐位相同**的输出流。
    ///
    /// 量什么：128 个输出样本（`f32` 位型）。
    ///
    /// 文档化的域是 `1–200 Hz`（见 [`NoiseGen::set_corner_hz`]）。注入实测：把
    /// `hz.clamp(1.0, 200.0)` 的下界改成 `0.0` ⇒ 既有全量判据**全绿**
    /// ⇒ 这条下界此前没有被守住（0 Hz 的漏积分器把泄漏项顶到自己的上界，
    /// 棕噪声的音色随之改变）。
    #[test]
    fn a_corner_below_the_documented_floor_is_folded_onto_the_floor() {
        let render = |corner_hz: f32| -> Vec<f32> {
            let mut noise = NoiseGen::new();
            noise.set_colour(NoiseColour::Brown);
            noise.set_corner_hz(corner_hz);
            (0..128)
                .map(|index| {
                    let white = if index % 2 == 0 { 0.5 } else { -0.5 };
                    noise.process(white, 48_000.0)
                })
                .collect()
        };
        let at_floor = render(1.0);
        for below in [0.0f32, -1.0, 0.5] {
            assert_eq!(render(below), at_floor, "转折频率 {below} Hz 必须折到 1 Hz");
        }
        // 正对照：下界之上不得被折，否则上面三条是空断言。
        assert_ne!(render(50.0), at_floor, "50 Hz 必须与 1 Hz 不同");
    }

    /// **判据（新写，可红）**：`reset` 之后的噪声源与**全新构造的同色实例**在同样
    /// 白噪声序列下逐位一致。
    ///
    /// 量什么：128 个输出样本（`f32` 位型）。
    ///
    /// `reset` 清两处颜色状态（粉噪声三元组与棕噪声漏积分器）。既有判据
    /// `stays_bounded` 只是**在循环里**调 `reset` 而不比对读数 ⇒ 没有一条判据守住
    /// 复位后的等价性。注入实测：去掉 `self.brown = 0.0;` ⇒ 既有全量判据**全绿**。
    #[test]
    fn reset_reproduces_a_freshly_built_noise_gen_bit_for_bit() {
        let build = || {
            let mut noise = NoiseGen::new();
            noise.set_colour(NoiseColour::Brown);
            noise
        };
        let drive = |noise: &mut NoiseGen| -> Vec<f32> {
            (0..128)
                .map(|index| {
                    let white = if index % 2 == 0 { 0.5 } else { -0.5 };
                    noise.process(white, 48_000.0)
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

    /// **判据（新写，可红）**：给定种子的序列被**逐位冻结**。
    ///
    /// 量什么：`Rng::new(0xdead_beef)` 的前 8 个 `next_u32()`（整数）、前 8 个
    /// `next_unit()` 与 `next_bipolar()` 的 `f32` 位型、以及 `Rng::new(0)` 的内部状态。
    ///
    /// [ARCH-DET-001] 要求"跨运行/跨机器同序"，而既有判据要么是**同一个算法自己
    /// 对自己**（`rng_is_deterministic_and_bipolar` 比的是两个 `Rng::new(42)`），
    /// 要么是统计性质 ⇒ 换掉 xorshift 的移位常量、换掉零种子的替代常量都**全绿**
    /// （注入实测：左移 13→12、替代种子 `0x9e37_79b9`→`0x1234_5678`）。
    /// 本判据把序列本身钉成字面量 ⇒ 两次注入都变红。
    ///
    /// 这里的运算只有整数异或/移位与 `f32` 的除、乘、减 ⇒ 属 IEEE 精确类，
    /// 跨架构逐位相同（裁决 R24 不需要 ulp 预算）。
    #[test]
    fn the_seeded_sequence_is_frozen_bit_for_bit() {
        let mut rng = Rng::new(0xdead_beef);
        let mut words = [0u32; 8];
        for slot in &mut words {
            *slot = rng.next_u32();
        }
        assert_eq!(
            words,
            [
                0x477d_20b7,
                0x8e1d_9142,
                0xba8c_2458,
                0xfee0_503b,
                0x680e_0348,
                0xa48d_b81b,
                0x6254_ea5c,
                0x1cfd_afb3,
            ],
            "xorshift 的序列漂移了（跨运行同序是 [ARCH-DET-001] 的契约）"
        );

        let mut rng = Rng::new(0xdead_beef);
        let mut unit_bits = [0u32; 8];
        for slot in &mut unit_bits {
            *slot = rng.next_unit().to_bits();
        }
        assert_eq!(
            unit_bits,
            [
                0x3e8e_fa41,
                0x3f0e_1d91,
                0x3f3a_8c24,
                0x3f7e_e050,
                0x3ed0_1c07,
                0x3f24_8db8,
                0x3ec4_a9d5,
                0x3de7_ed7e,
            ],
            "next_unit 的映射漂移了"
        );

        let mut rng = Rng::new(0xdead_beef);
        let mut bipolar_bits = [0u32; 8];
        for slot in &mut bipolar_bits {
            *slot = rng.next_bipolar().to_bits();
        }
        assert_eq!(
            bipolar_bits,
            [
                0xbee2_0b7e,
                0x3de1_d910,
                0x3eea_3090,
                0x3f7d_c0a0,
                0xbe3f_8fe4,
                0x3e92_36e0,
                0xbe6d_58ac,
                0xbf46_04a0,
            ],
            "next_bipolar 的映射漂移了"
        );

        // 零种子的替代值也是对外可观测的（`Rng::new(0)` 的序列由它决定）。
        assert_eq!(Rng::new(0).state(), 0x9e37_79b9);
    }

    /// 固定夹具：给定颜色、转折频率与采样率，喂 8 个固定的白噪声样本。
    fn render_bits(colour: NoiseColour, corner_hz: f32, sample_rate: f32) -> [u32; 8] {
        let mut noise = NoiseGen::new();
        noise.set_colour(colour);
        noise.set_corner_hz(corner_hz);
        let mut out = [0u32; 8];
        for (index, slot) in out.iter_mut().enumerate() {
            let white = if index % 2 == 0 { 0.5 } else { -0.25 };
            *slot = noise.process(white, sample_rate).to_bits();
        }
        out
    }

    /// **判据（新写，可红）**：三种颜色的渲染被**逐位冻结**。
    ///
    /// 量什么：8 个输出样本的 `f32` 位型（白／粉／棕各一组）。
    ///
    /// 成文契约只有"白平坦、粉 −3 dB/倍频程、棕 −6 dB/倍频程"与"有界"这两条，
    /// 而既有斜率判据的容差是 ±0.6／±0.8 dB ⇒ 极点系数与输出缩放都能在容差内被改掉
    /// 而**全绿**（注入实测三处：粉噪声第 0 极点 `0.99765→0.99`、棕噪声积分步长
    /// `0.05→0.5`、粉噪声输出缩放 `0.28→0.5`）⇒ 本判据三处都变红。
    ///
    /// 噪声的滤波链只有乘加、`TAU` 常量与一次除法（无超越函数）⇒ 属 IEEE 精确类，
    /// 跨架构逐位相同（裁决 R24）。
    #[test]
    fn the_noise_render_is_frozen_bit_for_bit() {
        assert_eq!(
            render_bits(NoiseColour::White, 5.0, 48_000.0),
            [
                0x3f00_0000,
                0xbe80_0000,
                0x3f00_0000,
                0xbe80_0000,
                0x3f00_0000,
                0xbe80_0000,
                0x3f00_0000,
                0xbe80_0000,
            ],
            "白噪声渲染漂移"
        );
        assert_eq!(
            render_bits(NoiseColour::Pink, 5.0, 48_000.0),
            [
                0x3e6a_1d4e,
                0x3cc0_8589,
                0x3e85_11c5,
                0x3d53_2142,
                0x3e92_581c,
                0x3d9b_a5df,
                0x3e9e_4dc7,
                0x3dc9_b36a,
            ],
            "粉噪声渲染漂移（极点系数或输出缩放被改了）"
        );
        assert_eq!(
            render_bits(NoiseColour::Brown, 50.0, 48_000.0),
            [
                0x3db2_06f2,
                0x3d2f_b25e,
                0x3e04_a678,
                0x3dae_8cf4,
                0x3e2f_b7b7,
                0x3e02_0f90,
                0x3e5a_391d,
                0x3e2c_49bd,
            ],
            "棕噪声渲染漂移（漏积分步长或泄漏被改了）"
        );
    }

    /// **判据（新写，可红）**：棕噪声泄漏的钳制区间**两端都是承重的**。
    ///
    /// 量什么：两组"原始泄漏落在钳制区间之外"的 `(corner, sample_rate)` 各自的 32 个
    /// 输出样本位型；同一组内两个请求必须逐位相同。
    ///
    /// `leak = 1 − TAU·corner/sr`，文档域是 `corner ∈ [1, 200] Hz` 与
    /// `sr ≥ MIN_SAMPLE_RATE = 1 000 Hz`（采样率没有上界）。既有判据全部用
    /// `48 kHz / 5 Hz`（原始泄漏 `0.99935`，落在区间**之内**）⇒ 把
    /// `leak.clamp(0.9, 0.999_9)` 放宽成 `(0.0, 1.0)` **全绿**（注入实测）。
    /// 本判据在两端各取一对"原始泄漏都越界"的输入：下界侧
    /// `(200 Hz, 1 kHz)` 的原始泄漏是 `−0.2566`、`(200 Hz, 2 kHz)` 是 `0.3717`；
    /// 上界侧 `(1 Hz, 1e8)` 是 `0.99999994`、`(1 Hz, 1e9)` 在 `f32` 里正好是 `1.0`。
    /// 钳制生效时组内逐位相同，钳制被拿掉时组内立刻分叉。
    #[test]
    fn the_brown_leak_clamp_is_load_bearing_at_both_ends() {
        let render = |corner_hz: f32, sample_rate: f32| -> [u32; 32] {
            let mut noise = NoiseGen::new();
            noise.set_colour(NoiseColour::Brown);
            noise.set_corner_hz(corner_hz);
            let mut out = [0u32; 32];
            for (index, slot) in out.iter_mut().enumerate() {
                let white = if index % 2 == 0 { 0.5 } else { -0.25 };
                *slot = noise.process(white, sample_rate).to_bits();
            }
            out
        };
        assert_eq!(
            render(200.0, 1_000.0),
            render(200.0, 2_000.0),
            "下界侧：两个原始泄漏都低于 0.9 的请求必须折到同一个泄漏上"
        );
        assert_eq!(
            render(1.0, 1.0e8),
            render(1.0, 1.0e9),
            "上界侧：两个原始泄漏都高于 0.9999 的请求必须折到同一个泄漏上"
        );
        // 正对照：不越界的两组请求必须给出不同的流（这条判据不是在空转）。
        assert_ne!(
            render(1.0, 48_000.0),
            render(200.0, 48_000.0),
            "不同的原始泄漏必须给出不同的流"
        );
    }
}
