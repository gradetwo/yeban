//! K 加权与**无门限**积分响度（LUFS）的最小子集。[ARCH-UI-002]
//!
//! 规范把响度计量的口径留给实现，但 UI 的"响度表"与母带链的增益决策需要它，
//! 因此这里给出 **ITU-R BS.1770-4 的最小可复算子集**：
//!
//! ```text
//! 滤波:  x ──▶ 高架 (Stage 1) ──▶ RLB 高通 (Stage 2) ──▶ y
//! 计量:  z = ( Σ_t y_L[t]² + Σ_t y_R[t]² ) / frames          (通道权重 G = 1)
//! 响度:  LUFS = −0.691 + 10·log10(z)
//! ```
//!
//! ## 为什么是 −0.691 与 997 Hz
//!
//! BS.1770 的 K 加权在 997 Hz 处的**功率**增益是 `10^(0.691/10) = 1.1725`，
//! 于是 −0.691 的偏置恰好把它抵消：**997 Hz、−20 dBFS 的双声道正弦读数就是
//! −20.0 LUFS**。这不是巧合而是标准的标定方式，也是本模块最强的判据
//! （[`tests::minus_twenty_dbfs_997hz_stereo_is_minus_twenty_lufs`]）。
//!
//! ## 诚实的边界（**不要**误读为完整的 BS.1770 积分响度）
//!
//! 本模块**故意只做最小子集**，以下都**没有**实现，做之前不要宣称有：
//!
//! 1. **门限（gating）**：没有 −70 LUFS 绝对门限，也没有 −10 LU 相对门限，
//!    更没有"400 ms 块 + 75% 重叠 + 门限后重算"的流程。因此
//!    [`LoudnessMeter`] 给的是**喂进去的全部样本**的能量平均：
//!    静音段会实打实地把读数拉低（判据
//!    [`tests::silence_padding_pulls_ungated_loudness_down`] 把这个语义钉住）。
//!    真正的积分响度需要门限，属于后续切片（见 notes 的 pending）。
//! 2. **3 s 短时窗口 / 400 ms 瞬时窗口**：本类型不做窗口，只做累计。
//! 3. **真峰值模式**：[ARCH-UI-002] 的"真峰值"由 [`crate::meter::TruePeakDetector`]
//!    单独提供；BS.1770-4 里"用真峰值防止 MP3 过载"的那条路径没有接。
//! 4. **多声道权重与 LFE**：只支持单声道与立体声，通道权重恒为 1；
//!    5.1 的环绕权重（1.0 / 1.41）与 LFE 排除未实现。
//! 5. **采样率**：K 加权系数是**采样率相关**的。仓库里只核验过 48 kHz 的那一组
//!    （[`KWeighting::new_48k`]），其他采样率一律由 [`KWeighting::for_sample_rate`]
//!    明确拒绝，而不是拿 48 kHz 的系数硬套（那会给出错误读数）。
//!
//! ## 数值纪律
//!
//! 滤波器状态与累加器用 `f64`：BS.1770 的参考实现就是双精度，而 RLB 高通的极点
//! 非常靠近单位圆（`a2 = 0.99007`），`f32` 状态会在长积分上引入可见漂移。
//! 样本本身仍是 `f32`（与引擎的块接口一致）。零分配、零锁、零 I/O [ARCH-RT-001]。

/// LUFS 标定偏置（dB）。见模块文档"为什么是 −0.691"。
pub const LUFS_OFFSET_DB: f64 = -0.691;

/// K 加权系数被核验过的采样率（Hz）。
pub const K_WEIGHTING_SAMPLE_RATE_HZ: f32 = 48_000.0;

/// BS.1770-4 Stage 1 高架滤波器 `(b0, b1, b2, a1, a2)` @48 kHz（`a0` 已归一化为 1）。
const SHELF_48K: [f64; 5] = [
    1.535_124_859_586_97,
    -2.691_696_189_406_38,
    1.198_392_810_852_85,
    -1.690_659_293_182_41,
    0.732_480_774_215_85,
];

/// BS.1770-4 Stage 2 RLB 高通滤波器 `(b0, b1, b2, a1, a2)` @48 kHz。
const HIGH_PASS_48K: [f64; 5] = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];

/// 归一化双二阶（`a0 = 1`）的转置直接 II 型实现（`f64` 状态）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    /// `(b0, b1, b2, a1, a2)`。
    coeffs: [f64; 5],
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// 用归一化系数构造（`a0 = 1`）。
    #[must_use]
    pub const fn new(coeffs: [f64; 5]) -> Self {
        Self {
            coeffs,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// 清空状态（系数保留）。
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// 处理一个样本（`f64` 进出，便于两级级联不丢精度）。
    pub fn process(&mut self, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.coeffs;
        let y = b0 * x + self.z1;
        self.z1 = b1 * x - a1 * y + self.z2;
        self.z2 = b2 * x - a2 * y;
        y
    }
}

/// K 加权滤波器（两级双二阶级联），左右声道各一套独立状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KWeighting {
    left: [Biquad; 2],
    right: [Biquad; 2],
}

impl Default for KWeighting {
    fn default() -> Self {
        Self::new_48k()
    }
}

impl KWeighting {
    /// 48 kHz 的 K 加权（唯一被核验过的一组系数）。
    #[must_use]
    pub const fn new_48k() -> Self {
        Self {
            left: [Biquad::new(SHELF_48K), Biquad::new(HIGH_PASS_48K)],
            right: [Biquad::new(SHELF_48K), Biquad::new(HIGH_PASS_48K)],
        }
    }

    /// 按采样率构造；**只支持 48 kHz**，其余返回 `None`。
    ///
    /// 明确拒绝而不是拿 48 kHz 系数硬套：BS.1770 的系数是采样率相关的，
    /// 硬套会得到错误的 LUFS（属于"说谎"而不是"近似"）。
    #[must_use]
    pub fn for_sample_rate(sample_rate: f32) -> Option<Self> {
        if sample_rate == K_WEIGHTING_SAMPLE_RATE_HZ {
            Some(Self::new_48k())
        } else {
            None
        }
    }

    /// 清空四个双二阶的状态。
    pub fn reset(&mut self) {
        for stage in &mut self.left {
            stage.reset();
        }
        for stage in &mut self.right {
            stage.reset();
        }
    }

    /// 处理一个立体声帧，返回 `(left, right)` 的 K 加权输出。
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f64, f64) {
        let l = self.left[0].process(f64::from(left));
        let l = self.left[1].process(l);
        let r = self.right[0].process(f64::from(right));
        let r = self.right[1].process(r);
        (l, r)
    }

    /// 处理一个单声道样本（走左声道那套状态）。
    pub fn process_mono(&mut self, sample: f32) -> f64 {
        let y = self.left[0].process(f64::from(sample));
        self.left[1].process(y)
    }
}

/// **无门限**积分响度计（LUFS 最小子集）。见模块文档的边界清单。
///
/// ```
/// use yeban_dsp::loudness::LoudnessMeter;
///
/// // 997 Hz、−20 dBFS 的双声道正弦 ⇒ −20.0 LUFS（BS.1770 的标定点）。
/// let samples: Vec<f32> = (0..48_000)
///     .map(|i| 0.1 * (std::f64::consts::TAU * 997.0 * i as f64 / 48_000.0).sin() as f32)
///     .collect();
/// let loudness = LoudnessMeter::integrated_stereo(&samples, &samples);
/// assert!((loudness + 20.0).abs() < 0.1, "实际 {loudness}");
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessMeter {
    k: KWeighting,
    sum_squares: f64,
    frames: u64,
}

impl Default for LoudnessMeter {
    fn default() -> Self {
        Self::new_48k()
    }
}

impl LoudnessMeter {
    /// 48 kHz 的新计量器（能量归零）。
    #[must_use]
    pub const fn new_48k() -> Self {
        Self {
            k: KWeighting::new_48k(),
            sum_squares: 0.0,
            frames: 0,
        }
    }

    /// 清空滤波器状态与累计能量。
    pub fn reset(&mut self) {
        self.k.reset();
        self.sum_squares = 0.0;
        self.frames = 0;
    }

    /// 累计的**通道和**均方（`(Σ y_L² + Σ y_R²) / frames`），尚未加偏置/取对数。
    #[must_use]
    pub fn mean_square(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.sum_squares / self.frames as f64
        }
    }

    /// 已累计的帧数（立体声的一帧 = L + R 各一个样本）。
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// 当前读数（LUFS）。没有样本或能量为 0 ⇒ [`f32::NEG_INFINITY`]（**不是** `NaN`）。
    #[must_use]
    pub fn loudness_lufs(&self) -> f32 {
        let mean_square = self.mean_square();
        if mean_square > 0.0 {
            (LUFS_OFFSET_DB + 10.0 * mean_square.log10()) as f32
        } else {
            f32::NEG_INFINITY
        }
    }

    /// 喂入一段**单声道**样本（通道权重 1）。
    pub fn add_mono(&mut self, samples: &[f32]) {
        for &sample in samples {
            let y = self.k.process_mono(clean(sample));
            self.sum_squares += y * y;
            self.frames += 1;
        }
    }

    /// 喂入一段**立体声**样本；长度不等时按较短者工作（**不 panic**）。
    pub fn add_stereo(&mut self, left: &[f32], right: &[f32]) {
        let frames = left.len().min(right.len());
        for index in 0..frames {
            let (l, r) = self
                .k
                .process_stereo(clean(left[index]), clean(right[index]));
            self.sum_squares += l * l + r * r;
            self.frames += 1;
        }
    }

    /// 一次性测一段单声道的无门限积分响度。
    #[must_use]
    pub fn integrated_mono(samples: &[f32]) -> f32 {
        let mut meter = Self::new_48k();
        meter.add_mono(samples);
        meter.loudness_lufs()
    }

    /// 一次性测一段立体声的无门限积分响度。
    #[must_use]
    pub fn integrated_stereo(left: &[f32], right: &[f32]) -> f32 {
        let mut meter = Self::new_48k();
        meter.add_stereo(left, right);
        meter.loudness_lufs()
    }
}

/// 输入侧的数值卫生：非有限样本按 0 处理（与 [`crate::meter::sanitize_sample`] 同口径）。
///
/// 不做这一步的话，一个 `NaN` 样本会把双二阶状态与累计能量**永久**污染成 `NaN`，
/// 读数只能退化成"负无穷"——计量器就死了。这与电平侧的理由完全一致。
#[must_use]
fn clean(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 997 Hz 的正弦（振幅 = 幅度），长度 = 1 秒。
    fn sine_997(amplitude: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let phase = std::f64::consts::TAU * 997.0 * i as f64 / 48_000.0;
                (f64::from(amplitude) * phase.sin()) as f32
            })
            .collect()
    }

    /// **最强判据**：BS.1770 的标定点。
    ///
    /// 997 Hz、−20 dBFS 的振荡加到 L 与 R 两个通道 ⇒ **−20.0 LUFS**。
    /// 这一条同时钉住了：两级滤波器的系数、级联顺序、通道求和、−0.691 偏置、
    /// 以及"均方而不是均方根"四件事 —— 任何一处错都会移动好几个 dB。
    ///
    /// 注入：把偏置改成 0 ⇒ 读数变 −19.31；漏掉高通级 ⇒ 读数偏 +0.69；
    /// 只用单通道能量 ⇒ 读数变 −23.01。
    #[test]
    fn minus_twenty_dbfs_997hz_stereo_is_minus_twenty_lufs() {
        let samples = sine_997(0.1, 48_000);
        let loudness = LoudnessMeter::integrated_stereo(&samples, &samples);
        assert!(
            (loudness + 20.0).abs() < 0.05,
            "997 Hz −20 dBFS 双声道应为 −20.0 LUFS, 实际 {loudness}"
        );
    }

    /// 判据：只在**左**通道加同一个正弦 ⇒ 能量减半 ⇒ −23.01 LUFS
    /// （`10·log10(2) = 3.0103`）。这钉住"两个通道的能量是**相加**的"。
    #[test]
    fn one_channel_only_is_three_db_quieter() {
        let samples = sine_997(0.1, 48_000);
        let silent = vec![0.0f32; 48_000];
        let loudness = LoudnessMeter::integrated_stereo(&samples, &silent);
        assert!(
            (loudness + 23.010_3).abs() < 0.05,
            "单通道 −20 dBFS 应为 −23.01 LUFS, 实际 {loudness}"
        );
        // 单声道接口应当给出同一个读数（权重都是 1）。
        let mono = LoudnessMeter::integrated_mono(&samples);
        assert!(
            (mono - loudness).abs() < 1e-4,
            "单声道接口与'右通道静音'必须一致: {mono} vs {loudness}"
        );
    }

    /// 判据：幅度每降 6.0206 dB，LUFS 也降 6.0206 dB（计量是线性能量的对数）。
    #[test]
    fn loudness_tracks_amplitude_by_six_db_per_halving() {
        let full = LoudnessMeter::integrated_stereo(&sine_997(0.2, 48_000), &sine_997(0.2, 48_000));
        let half = LoudnessMeter::integrated_stereo(&sine_997(0.1, 48_000), &sine_997(0.1, 48_000));
        assert!(
            ((full - half) - 6.020_6).abs() < 0.02,
            "减半应降 6.0206 dB, 实际 {}",
            full - half
        );
    }

    /// 判据：K 加权的高通级**拒绝直流**（这是它存在的理由）。
    ///
    /// 直流 1.0 的输入能量是 1.0；高通级在**稳态**把它压到解析零点
    /// （`(b0+b1+b2)/(1+a1+a2) = 0`）。这里先跑掉阶跃瞬态（4800 样本 ≈ 24 个时间常数），
    /// 再在 1 秒窗上累计，读数必须低到 −100 LUFS 以下。
    ///
    /// 注入：丢掉高通级（只留高架）⇒ 多出一个 +4 dB 的直通台阶，读数跳到 0 LUFS 附近。
    #[test]
    fn dc_is_rejected_by_the_k_weighting_high_pass() {
        let mut k = KWeighting::new_48k();
        let mut sum = 0.0f64;
        let mut frames = 0u64;
        for index in 0..48_000 {
            let (l, r) = k.process_stereo(1.0, 1.0);
            if index >= 4_800 {
                sum += l * l + r * r;
                frames += 1;
            }
        }
        let loudness = LUFS_OFFSET_DB + 10.0 * (sum / frames as f64).log10();
        assert!(
            loudness < -100.0,
            "稳态直流必须被高通级拒绝, 实际 {loudness} LUFS"
        );
        assert!(loudness.is_finite());
    }

    /// 判据：**没有门限**是本实现的诚实语义（不是 bug）。
    ///
    /// 1 秒 −20 dBFS 正弦 + 1 秒静音 ⇒ 能量减半 ⇒ −23.01 LUFS。
    /// 真正的 BS.1770 积分响度会用 −70 LUFS 绝对门限把静音段排除掉、
    /// 读数仍是 −20；本最小子集不做门限，因此这里明确钉住"会被拉低"。
    /// 等门限切片落地时，**这条判据必须被显式改写**（而不是悄悄变红）。
    #[test]
    fn silence_padding_pulls_ungated_loudness_down() {
        let mut samples = sine_997(0.1, 48_000);
        samples.extend(std::iter::repeat_n(0.0f32, 48_000));
        let loudness = LoudnessMeter::integrated_stereo(&samples, &samples);
        assert!(
            (loudness + 23.010_3).abs() < 0.05,
            "无门限时静音段应把读数拉到 −23.01 LUFS, 实际 {loudness}"
        );
    }

    /// 判据：静音/空输入 ⇒ 负无穷，**绝不是** `NaN`。
    #[test]
    fn silence_and_empty_input_are_negative_infinity_never_nan() {
        let meter = LoudnessMeter::new_48k();
        assert_eq!(meter.frames(), 0);
        assert_eq!(meter.mean_square(), 0.0);
        assert_eq!(meter.loudness_lufs(), f32::NEG_INFINITY);

        let silence = vec![0.0f32; 4096];
        let loudness = LoudnessMeter::integrated_stereo(&silence, &silence);
        assert_eq!(loudness, f32::NEG_INFINITY);
        assert!(!loudness.is_nan());
        assert_eq!(
            LoudnessMeter::integrated_stereo(&[], &[]),
            f32::NEG_INFINITY
        );

        // 长度不等时按较短者工作, 不 panic。
        let asymmetric = LoudnessMeter::integrated_stereo(&[0.5f32; 32], &[0.5f32; 8]);
        assert!(asymmetric.is_finite());
    }

    /// 判据：非法样本（NaN/±∞）不得污染读数 —— 与电平侧同样的数值卫生。
    ///
    /// 注入：删掉 `clean`（把样本直接喂进双二阶）⇒ 状态变 `NaN`、累计能量变 `NaN`、
    /// 读数退化成 −∞，本判据变红。
    #[test]
    fn hostile_samples_do_not_poison_the_reading() {
        let clean_left = sine_997(0.1, 24_000);
        let mut hostile_left = clean_left.clone();
        hostile_left[1234] = f32::NAN;
        hostile_left[2345] = f32::INFINITY;
        hostile_left[2346] = f32::NEG_INFINITY;
        let right = sine_997(0.1, 24_000);
        let loudness = LoudnessMeter::integrated_stereo(&hostile_left, &right);
        let reference = LoudnessMeter::integrated_stereo(&clean_left, &right);
        assert!(loudness.is_finite(), "敌对样本不得产生 NaN/−∞: {loudness}");
        assert!(
            (loudness - reference).abs() < 1e-3,
            "3 个坏样本不得污染整段读数: {loudness} vs {reference}"
        );
    }

    /// 判据：同输入同输出**逐位**一致（计量是确定性的）。
    #[test]
    fn loudness_is_bit_deterministic() {
        let samples = sine_997(0.13, 9_999);
        let a = LoudnessMeter::integrated_stereo(&samples, &samples);
        let b = LoudnessMeter::integrated_stereo(&samples, &samples);
        assert_eq!(a.to_bits(), b.to_bits());
        // 分块喂入必须与一次喂入逐位相同（滤波器状态是连续推进的）。
        let mut chunked = LoudnessMeter::new_48k();
        for chunk in samples.chunks(97) {
            chunked.add_stereo(chunk, chunk);
        }
        assert_eq!(chunked.loudness_lufs().to_bits(), a.to_bits());
    }

    /// 判据：`reset` 之后逐位回到初态。
    #[test]
    fn reset_restores_the_initial_state() {
        let mut meter = LoudnessMeter::new_48k();
        meter.add_stereo(&sine_997(0.5, 4096), &sine_997(0.5, 4096));
        assert!(meter.loudness_lufs().is_finite());
        meter.reset();
        assert_eq!(meter, LoudnessMeter::new_48k());
        assert_eq!(meter.frames(), 0);
        assert_eq!(meter.loudness_lufs(), f32::NEG_INFINITY);
    }

    /// 判据：采样率支持是**显式**的 —— 48 kHz 有系数，其余明确拒绝。
    #[test]
    fn only_the_verified_sample_rate_is_accepted() {
        assert!(KWeighting::for_sample_rate(48_000.0).is_some());
        assert_eq!(LoudnessMeter::new_48k().frames(), 0);
        for rate in [44_100.0f32, 88_200.0, 96_000.0, 0.0, -48_000.0] {
            assert!(
                KWeighting::for_sample_rate(rate).is_none(),
                "{rate} Hz 没有被核验过的系数, 必须拒绝而不是硬套 48 kHz"
            );
        }
    }

    /// 判据：双二阶的直流/奈奎斯特增益与解析值一致（系数本身没被改坏）。
    #[test]
    fn biquad_gains_match_the_itu_analytic_values() {
        // Stage 1 直流增益 ≈ 1; Nyquist 增益 ≈ +4 dB。
        assert!((shelf_dc_gain() - 1.0).abs() < 1e-3);
        assert!((shelf_nyquist_gain_db() - 3.999).abs() < 0.01);
        // Stage 2 直流增益 ≈ 0（高通）；997 Hz 处幅度 ≈ 1。
        assert!(high_pass_dc_gain().abs() < 1e-3);
        assert!((magnitude_at(HIGH_PASS_48K, 997.0) - 1.0035).abs() < 1e-3);
        // 级联在 997 Hz 的幅度增益 ≈ +0.691 dB（标定常数 −0.691 的来源）。
        let combined_db =
            20.0 * (magnitude_at(SHELF_48K, 997.0) * magnitude_at(HIGH_PASS_48K, 997.0)).log10();
        assert!(
            (combined_db - 0.691).abs() < 0.01,
            "997 Hz 级联增益应为 +0.691 dB, 实际 {combined_db}"
        );
    }

    fn magnitude_at(coeffs: [f64; 5], frequency: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = coeffs;
        let omega = std::f64::consts::TAU * frequency / 48_000.0;
        let (cos1, sin1) = ((-omega).cos(), (-omega).sin());
        let (cos2, sin2) = ((-2.0 * omega).cos(), (-2.0 * omega).sin());
        let num_re = b0 + b1 * cos1 + b2 * cos2;
        let num_im = b1 * sin1 + b2 * sin2;
        let den_re = 1.0 + a1 * cos1 + a2 * cos2;
        let den_im = a1 * sin1 + a2 * sin2;
        (num_re * num_re + num_im * num_im).sqrt() / (den_re * den_re + den_im * den_im).sqrt()
    }

    fn shelf_dc_gain() -> f64 {
        let [b0, b1, b2, a1, a2] = SHELF_48K;
        (b0 + b1 + b2) / (1.0 + a1 + a2)
    }

    fn shelf_nyquist_gain_db() -> f64 {
        let [b0, b1, b2, a1, a2] = SHELF_48K;
        20.0 * ((b0 - b1 + b2) / (1.0 - a1 + a2)).log10()
    }

    fn high_pass_dc_gain() -> f64 {
        let [b0, b1, b2, a1, a2] = HIGH_PASS_48K;
        (b0 + b1 + b2) / (1.0 + a1 + a2)
    }
}
