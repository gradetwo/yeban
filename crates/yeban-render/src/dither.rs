//! TPDF 抖动与位深转换 [ARCH-FMT-001] [ARCH-DET-001]。
//!
//! ## 规范依据
//!
//! > **[ARCH-FMT-001]** 内置高质量 TPDF (Triangular Probability Density Function)
//! > 高精抖动算法, 在将 32-bit Float 降采样为 24-bit / 16-bit 整数导出时,
//! > 消除量化非线性截断失真。
//!
//! ## 抖动定义 (核验过的上游事实, 出处见 notes §1)
//!
//! TPDF 的构造是**两个独立均匀 (RPDF) 随机量之和**:
//!
//! - Wikipedia《Dither》§Noise distributions: "Triangular distribution can be
//!   achieved by adding two independent RPDF sources."; §"Which noise distribution
//!   to use" 进一步要求 "a triangular-type dither that has an amplitude of **two
//!   quantization steps** so that the dither values computed range from, for example,
//!   −1 to +1, or 0 to 2"。
//! - SoX `src/dither.c` 的 `flow_no_shape` 把**两个** `RANQD1 >> prec` (各覆盖 1 LSB)
//!   相加后再四舍五入, 并在 `i <= (-1 << (prec-1))` / `i > SOX_INT_MAX(prec)` 处钳位。
//!
//! 因此本实现取
//!
//! ```text
//! d = u1 - u2,   u1, u2 ~ U[0, 1)   =>   d ∈ (-1, +1) LSB, 三角分布, E[d] = 0
//! ```
//!
//! 用**差**而不是和: 两个 `U[0,1)` 之和的均值是 +1 LSB, 会给输出注入 1 LSB 的直流
//! 偏移; 差则严格零均值。这不是风格选择, 是 `tests::no_dc_offset` 要钉住的量 ——
//! 它同时配了一条变异敏感度自证 (`tests::one_sided_rpdf_dither_would_be_rejected`),
//! 证明"求和"那一版会被同一条容差拒绝。
//!
//! ## 与"平台 libm"的关系
//!
//! [ARCH-DET-001] 禁止把结果押在宿主 libm 上。本模块只用到:
//! 加法、减法、乘法、`f32::round()`。[IEEE 754] 把 `round` 指定为
//! `roundToIntegralTiesAway` —— 它**不是**超越函数, 在所有符合 IEEE 754 的目标上
//! 逐位相同, 因此不需要 (也不能) 绕道 `libm`。真正的超越函数 (dB→线性增益)
//! 走 `libm::powf`, 见 `render.rs`。
//!
//! [IEEE 754]: https://en.wikipedia.org/wiki/IEEE_754
//!
//! 本模块**不依赖任何第三方 crate**, 可用
//! `rustc --edition 2024 --test src/dither.rs` 单独执行。

/// 抖动所需的均匀随机源, `[0, 1)`。
///
/// 由调用方**注入**而不是在模块内创建: [ARCH-DET-001] 要求"使用固定种子 PRNG",
/// 种子的所有权必须在会话/工程层 (见 `render::RenderOptions::seed`), 而不是埋在
/// 一个算法模块里。生产实现见 `crate::rng` (包装 `yeban_dsp::noise::Rng`)。
pub trait DitherRng {
    /// 返回 `[0, 1)` 上的均匀分布样本。
    fn next_unit(&mut self) -> f32;
}

/// 一次 TPDF 抽样的最大幅值 (1 LSB)。TPDF 的峰峰值是 **2 LSB**。
pub const TPDF_PEAK_LSB: f32 = 1.0;

/// 导出的位深。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitDepth {
    /// 16-bit 有符号整数 PCM。
    Int16,
    /// 24-bit 有符号整数 PCM (`i32` 承载, 高 8 位恒为符号扩展)。
    Int24,
    /// 32-bit 浮点 PCM —— **不做抖动**, 原样透传 (见 [`quantize`])。
    Float32,
}

impl BitDepth {
    /// 每个样本占用的字节数 (16→2, 24→3, 32f→4)。
    #[must_use]
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::Int16 => 2,
            Self::Int24 => 3,
            Self::Float32 => 4,
        }
    }

    /// 有效位深 (32f 报告 32)。
    #[must_use]
    pub const fn bits(self) -> u16 {
        (self.bytes_per_sample() as u16) * 8
    }

    /// `true` 表示整数 PCM (需要抖动), `false` 表示 IEEE 浮点容器。
    #[must_use]
    pub const fn is_integer(self) -> bool {
        !matches!(self, Self::Float32)
    }

    /// 满量程负端 (整数位深), 即 `-2^(bits-1)`。
    ///
    /// # Panics
    ///
    /// 对 [`BitDepth::Float32`] 调用是逻辑错误 (浮点没有整数满量程)。
    #[must_use]
    pub const fn full_scale_min(self) -> i32 {
        match self {
            Self::Int16 => i16::MIN as i32,
            Self::Int24 => -(1 << 23),
            Self::Float32 => panic!("32f 没有整数满量程"),
        }
    }

    /// 满量程正端 (整数位深), 即 `2^(bits-1) - 1`。
    ///
    /// 注意是 `-1`: 24-bit 的合法上界是 `8388607`, 不是 `8388608`。
    ///
    /// # Panics
    ///
    /// 对 [`BitDepth::Float32`] 调用是逻辑错误。
    #[must_use]
    pub const fn full_scale_max(self) -> i32 {
        match self {
            Self::Int16 => i16::MAX as i32,
            Self::Int24 => (1 << 23) - 1,
            Self::Float32 => panic!("32f 没有整数满量程"),
        }
    }

    /// 抖动前把 `[-1, 1]` 归一化浮点映射到 LSB 整数域的缩放系数 `2^(bits-1)`。
    ///
    /// # Panics
    ///
    /// 对 [`BitDepth::Float32`] 调用是逻辑错误。
    #[must_use]
    pub fn scale(self) -> f32 {
        match self {
            Self::Int16 => 32768.0,
            Self::Int24 => 8_388_608.0,
            Self::Float32 => panic!("32f 不做 LSB 缩放"),
        }
    }
}

/// 一次 TPDF 抽样, 单位为 LSB, 落在 `(-1, +1)`, 均值为 0。
///
/// 消耗随机源**两次**。调用方必须按样本数 1:1 地调用它, 否则渲染不可复现。
#[inline]
pub fn tpdf_lsb(rng: &mut impl DitherRng) -> f32 {
    rng.next_unit() - rng.next_unit()
}

/// 把单个 `[-1, 1]` 浮点样本量化到 16-bit, 带 TPDF 抖动。
///
/// 步骤严格按 [crate] 文档的顺序: 缩放 → 加抖动 → 四舍五入 → 钳位。
/// 抖动必须在**四舍五入之前**加入, 否则只是给量化误差加了噪声, 起不到去相关作用。
#[must_use]
pub fn quantize_i16(sample: f32, rng: &mut impl DitherRng) -> i16 {
    let scaled = sample * BitDepth::Int16.scale() + tpdf_lsb(rng);
    clamp_i32(round_away_from_zero(scaled), BitDepth::Int16) as i16
}

/// 把单个 `[-1, 1]` 浮点样本量化到 24-bit (符号扩展存于 `i32`), 带 TPDF 抖动。
#[must_use]
pub fn quantize_i24(sample: f32, rng: &mut impl DitherRng) -> i32 {
    let scaled = sample * BitDepth::Int24.scale() + tpdf_lsb(rng);
    clamp_i32(round_away_from_zero(scaled), BitDepth::Int24)
}

/// 半值远离零的四舍五入 (`roundToIntegralTiesAway`, IEEE 754 精确定义)。
///
/// 用 `f32::round` 而不是 `+0.5` 后截断: 后者对 `-0.5` 会舍到 `0` 而不是 `-1`,
/// 在零附近引入不对称。`NaN` 先归零, 避免 `NaN as i32` 的饱和语义悄悄变成
/// `i32::MIN` 而污染输出。非有限输入按满量程钳位。
#[inline]
fn round_away_from_zero(value: f32) -> i32 {
    if value.is_nan() {
        return 0;
    }
    if value >= i32::MAX as f32 {
        return i32::MAX;
    }
    if value <= i32::MIN as f32 {
        return i32::MIN;
    }
    value.round() as i32
}

/// 按位深把整数域样本钳到合法闭区间 `[min, max]`。
#[inline]
fn clamp_i32(value: i32, depth: BitDepth) -> i32 {
    value.clamp(depth.full_scale_min(), depth.full_scale_max())
}

/// 量化后的 PCM 缓冲。
#[derive(Clone, Debug, PartialEq)]
pub enum PcmBuffer {
    /// 16-bit 整数样本。
    Int16(Vec<i16>),
    /// 24-bit 整数样本 (符号扩展存于 `i32`)。
    Int24(Vec<i32>),
    /// 32-bit 浮点样本。
    Float32(Vec<f32>),
}

impl PcmBuffer {
    /// 样本个数 (不是字节数, 也不是帧数)。
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Int16(samples) => samples.len(),
            Self::Int24(samples) => samples.len(),
            Self::Float32(samples) => samples.len(),
        }
    }

    /// `true` 表示没有任何样本。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 对应的位深。
    #[must_use]
    pub const fn depth(&self) -> BitDepth {
        match self {
            Self::Int16(_) => BitDepth::Int16,
            Self::Int24(_) => BitDepth::Int24,
            Self::Float32(_) => BitDepth::Float32,
        }
    }

    /// 容器字节数 (每样本按 [`BitDepth::bytes_per_sample`] 计, 不含任何文件头)。
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.len() * self.depth().bytes_per_sample()
    }

    /// 编码为**小端**交织字节流 —— RIFF/RF64 的 `data` chunk 载荷就是这个。
    ///
    /// 24-bit 取低 3 字节小端: 规范的 24-bit PCM 就是"3 字节小端有符号",
    /// 第 4 字节是 `i32` 承载带来的符号扩展, **不得**写进文件。
    #[must_use]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.byte_len());
        match self {
            Self::Int16(samples) => {
                for &sample in samples {
                    out.extend_from_slice(&sample.to_le_bytes());
                }
            }
            Self::Int24(samples) => {
                for &sample in samples {
                    let bytes = sample.to_le_bytes();
                    out.extend_from_slice(&bytes[..3]);
                }
            }
            Self::Float32(samples) => {
                for &sample in samples {
                    out.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
        out
    }
}

/// 全缓冲量化。
///
/// - [`BitDepth::Int16`] / [`BitDepth::Int24`]: 每个样本消耗随机源两次 (TPDF), 加抖动后量化;
/// - [`BitDepth::Float32`]: **透传**。32-bit 浮点是本工程的内部精度, 对它做抖动
///   等于在还没降位深时就污染母带 —— 抖动的定义域是"降位深"这一步 [ARCH-FMT-001]。
///   非有限值在此路径上被替换为 `0.0`, 以免写出含 `NaN` 的容器。
#[must_use]
pub fn quantize(input: &[f32], depth: BitDepth, rng: &mut impl DitherRng) -> PcmBuffer {
    match depth {
        BitDepth::Int16 => PcmBuffer::Int16(
            input
                .iter()
                .map(|&sample| quantize_i16(sample, rng))
                .collect(),
        ),
        BitDepth::Int24 => PcmBuffer::Int24(
            input
                .iter()
                .map(|&sample| quantize_i24(sample, rng))
                .collect(),
        ),
        BitDepth::Float32 => PcmBuffer::Float32(
            input
                .iter()
                .map(|&sample| if sample.is_finite() { sample } else { 0.0 })
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **测试脚手架**, 不是生产随机源。
    ///
    /// 生产随机源是 `yeban_dsp::noise::Rng` (由 `crate::rng` 接到 [`DitherRng`] 上)。
    /// 这里自带一个 xorshift32 是为了让 `dither.rs` 保持**零第三方依赖**,
    /// 从而能用 `rustc --test` 单独编译执行。算法与 `yeban-dsp` 的 `Rng` 相同。
    struct Xorshift32(u32);

    impl DitherRng for Xorshift32 {
        fn next_unit(&mut self) -> f32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x as f32 / u32::MAX as f32
        }
    }

    fn rng() -> Xorshift32 {
        Xorshift32(0x1234_5678)
    }

    #[test]
    fn tpdf_is_bounded_by_one_lsb() {
        let mut rng = rng();
        for _ in 0..100_000 {
            let sample = tpdf_lsb(&mut rng);
            assert!(sample > -1.0 && sample < 1.0, "越界: {sample}");
        }
    }

    /// 判据 1: TPDF 严格零均值 —— 这是"不引入 DC 偏移"的直接证明。
    #[test]
    fn no_dc_offset() {
        let mut rng = rng();
        let draws = 200_000;
        let mut sum = 0.0f64;
        for _ in 0..draws {
            sum += f64::from(tpdf_lsb(&mut rng));
        }
        let mean = sum / f64::from(draws);
        // 三角分布方差 = 1/6 LSB², 因此均值的标准差 ≈ 0.408/sqrt(2e5) ≈ 0.0009 LSB。
        // 0.01 LSB 约为 11σ, 且 RNG 已固定种子 -> 本判据无抖动。
        assert!(mean.abs() < 0.01, "TPDF 均值 {mean} 超出 0.01 LSB");
    }

    /// 判据 2 (变异敏感度自证): 把差分换成**求和** (即两个 `U[0,1)` 相加, 均值 +1 LSB),
    /// 同一个统计量必须被同一条容差拒绝。没有这条, 判据 1 可能是空判据。
    #[test]
    fn one_sided_rpdf_dither_would_be_rejected() {
        let mut rng = rng();
        let draws = 200_000;
        let mut sum = 0.0f64;
        for _ in 0..draws {
            let u1 = rng.next_unit();
            let u2 = rng.next_unit();
            sum += f64::from(u1 + u2); // 故意写错: 和 => +1 LSB 直流
        }
        let mean = sum / f64::from(draws);
        assert!(
            (mean - 1.0).abs() < 0.01,
            "求和的均值应约为 +1 LSB, 实测 {mean}"
        );
        assert!(mean.abs() >= 0.01, "判据 1 的容差必须能拒绝这一版实现");
    }

    /// 判据 3: 量化输出恒在合法闭区间内, 包括 ±1.0 之外的过载输入。
    #[test]
    fn output_stays_inside_legal_range() {
        let mut rng = rng();
        let inputs = [
            0.0f32,
            1.0,
            -1.0,
            1.5,
            -1.5,
            12.0,
            -12.0,
            0.999_999_94,
            -0.999_999_94,
            f32::MAX,
            f32::MIN,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for &input in &inputs {
            let i16_sample = quantize_i16(input, &mut rng);
            assert!(
                (i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&i32::from(i16_sample)),
                "i16 越界: input={input} -> {i16_sample}"
            );
            let i24_sample = quantize_i24(input, &mut rng);
            assert!(
                (BitDepth::Int24.full_scale_min()..=BitDepth::Int24.full_scale_max())
                    .contains(&i24_sample),
                "i24 越界: input={input} -> {i24_sample}"
            );
        }
    }

    /// 判据 4: `+1.0` 映射到满量程正端 `2^(bits-1) - 1`, **不得**绕回负端。
    /// 这是 24-bit 最容易写错的一格 (`8388608` vs `8388607`)。
    #[test]
    fn positive_full_scale_does_not_wrap_around() {
        let mut rng = rng();
        assert_eq!(quantize_i24(1.0, &mut rng), 8_388_607);
        assert_eq!(quantize_i16(1.0, &mut rng), 32_767);
        assert_eq!(quantize_i24(-1.0, &mut rng), -8_388_608);
        assert_eq!(quantize_i16(-1.0, &mut rng), -32_768);
    }

    /// 判据 5: 恒定输入的**期望**量化输出等于输入 (TPDF 的无偏性)。
    ///
    /// 取一个无法用 16-bit 精确表示的常量: 12345.6789 LSB。
    #[test]
    fn constant_input_is_unbiased() {
        let mut rng = rng();
        let ideal = 12345.6789f32;
        let input = ideal / 32768.0;
        let draws = 200_000;
        let mut sum = 0.0f64;
        for _ in 0..draws {
            sum += f64::from(quantize_i16(input, &mut rng));
        }
        let mean = sum / f64::from(draws);
        let error_lsb = mean - f64::from(ideal);
        assert!(
            error_lsb.abs() < 0.01,
            "恒定输入出现 {error_lsb} LSB 的直流偏移"
        );
    }

    /// 判据 6: 32f 是透传 —— 不得抖动、不得改位型, 且非有限值被清零。
    #[test]
    fn float32_path_is_bit_transparent() {
        let mut rng = rng();
        let input = [0.1f32, -0.25, 1.0, f32::NAN, f32::INFINITY];
        let out = quantize(&input, BitDepth::Float32, &mut rng);
        match out {
            PcmBuffer::Float32(samples) => {
                assert_eq!(samples[0].to_bits(), 0.1f32.to_bits());
                assert_eq!(samples[1].to_bits(), (-0.25f32).to_bits());
                assert_eq!(samples[2], 1.0);
                assert_eq!(samples[3], 0.0);
                assert_eq!(samples[4], 0.0);
            }
            other => panic!("期望 Float32, 得到 {other:?}"),
        }
    }

    #[test]
    fn twenty_four_bit_payload_is_three_bytes_little_endian() {
        let buffer = PcmBuffer::Int24(vec![1, -1, 0x7F_FFFF, -0x80_0000]);
        let bytes = buffer.to_le_bytes();
        assert_eq!(bytes.len(), 12);
        assert_eq!(&bytes[0..3], &[0x01, 0x00, 0x00]);
        assert_eq!(&bytes[3..6], &[0xFF, 0xFF, 0xFF]);
        assert_eq!(&bytes[6..9], &[0xFF, 0xFF, 0x7F]);
        assert_eq!(&bytes[9..12], &[0x00, 0x00, 0x80]);
    }

    /// 判据 7: 同一输入 + 同一种子 ⇒ 逐位相同 (ARCH-DET-001 的可复现性前提)。
    #[test]
    fn quantization_is_reproducible_for_a_fixed_seed() {
        let input: Vec<f32> = (0..1024).map(|i| (i as f32 / 1024.0) - 0.5).collect();
        let first = quantize(&input, BitDepth::Int24, &mut rng());
        let second = quantize(&input, BitDepth::Int24, &mut rng());
        assert_eq!(first, second);
        let third = quantize(&input, BitDepth::Int24, &mut Xorshift32(0xDEAD_BEEF));
        assert_ne!(first, third, "不同种子必须给出不同抖动序列");
    }

    #[test]
    fn depth_reporting_is_consistent() {
        assert_eq!(BitDepth::Int16.bits(), 16);
        assert_eq!(BitDepth::Int24.bits(), 24);
        assert_eq!(BitDepth::Float32.bits(), 32);
        assert_eq!(BitDepth::Int24.bytes_per_sample(), 3);
        assert!(BitDepth::Int16.is_integer());
        assert!(!BitDepth::Float32.is_integer());
        assert_eq!(BitDepth::Int16.full_scale_min(), -32_768);
        assert_eq!(BitDepth::Int24.full_scale_max(), 8_388_607);
    }
}
