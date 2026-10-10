//! K 加权、**门限积分**响度（LUFS）与瞬时/短时窗口。[ARCH-UI-002]
//!
//! 规范把响度计量的口径留给实现，但 UI 的"响度表"与母带链的增益决策需要它，
//! 因此这里给出 **ITU-R BS.1770-4** 的可复算实现：
//!
//! ```text
//! 滤波:  x ──▶ 高架 (Stage 1) ──▶ RLB 高通 (Stage 2) ──▶ y
//! 计量:  z = ( Σ_t y_L[t]² + Σ_t y_R[t]² ) / frames          (通道权重 G = 1)
//! 响度:  LUFS = −0.691 + 10·log10(z)
//!
//! 瞬时 momentary : 400 ms 滑窗, 每 100 ms 更新（[`GatedLoudness`]）
//! 短时 short-term: 3 s   滑窗, 每 100 ms 更新
//! 积分 integrated: 400 ms 块 + 75% 重叠（100 ms 步进）,
//!                  Γa = −70 LUFS 绝对门限 + Γr = −10 LU 相对门限（[`GatedLoudness`]）
//! ```
//!
//! ## 两种计量器，两种职责（不要混用）
//!
//! | 类型 | 语义 | 保留理由 |
//! | :--- | :--- | :--- |
//! | [`LoudnessMeter`] | **无门限**的能量累加器：`−0.691 + 10·log10(整段均方)` | 上一线的语义，**逐位不变**（见 §"不改既有语义"） |
//! | [`GatedLoudness`] | BS.1770-4 的**门限积分** + 瞬时/短时窗口 | 本线按 HD-27 补齐；静音段不再拉低读数 |
//!
//! 两者共用同一套 K 加权系数与同一份 `f64` 双二阶实现。
//!
//! ## 为什么 −0.691 与 997 Hz
//!
//! BS.1770 的 K 加权在 997 Hz 处的**功率**增益是 `10^(0.691/10) = 1.1725`，
//! 于是 −0.691 的偏置恰好把它抵消：**997 Hz、−20 dBFS 的双声道正弦读数就是
//! −20.0 LUFS**。这不是巧合而是标准的标定方式，也是本模块最强的判据
//! （[`tests::minus_twenty_dbfs_997hz_stereo_is_minus_twenty_lufs`]）。
//!
//! 注意 −0.691 是**48 kHz 的**标定常数。本实现把它用于所有采样率（标准正文也只给
//! 了这一个常数），因此其它采样率上 997 Hz 的标定点有 **±0.02 dB 以内**的残余偏差
//! （**实测**：44.1 kHz −19.997343、48 kHz −20.000105、88.2 kHz −20.015779、
//! 96 kHz −20.017427 ⇒ 偏差 +0.0027 / 0.0001 / −0.0158 / −0.0174 dB；
//! 判据 [`tests::minus_twenty_dbfs_997hz_is_minus_twenty_lufs_at_every_supported_rate`]
//! 把这个数字钉死到 ±0.025 dB）。这是**标准的口径**而不是本实现的偷懒：换一个
//! "每率归一化"的偏置会更贴合 997 Hz 标定点，但那样就不再是 BS.1770 的公式了。
//!
//! ## 诚实的边界（**不要**误读为完整 BS.1770）
//!
//! 1. **积分响度是两遍（two-pass）离线测量**：相对门限 `Γr` 依赖于"过绝对门限的
//!    那些块的平均响度"，因此真正精确的积分值必须看完整段信号（BS.1770-4 的参考
//!    实现也是这么算的）。[`GatedLoudness::integrated_stereo`] 因此**走两遍滤波**：
//!    第一遍求 `Γr`，第二遍按 `max(Γa, Γr)` 求和。好处是**逐位精确、零分配、零状态**，
//!    代价是 2× 滤波成本（离线导出可以接受）。要做**流式** integrated 就得要么把
//!    所有块的 `z` 存下来（无界），要么用量化直方图（有损）—— 两者都没有必要，
//!    见 notes 的 pending。
//!    **流式**部分（瞬时/短时窗口）是**有界**的：[`GatedLoudness`] 只保留 30 个
//!    100 ms 跳的能量（固定数组），实时路径零分配 [ARCH-RT-001]。
//! 2. **3 s 短时窗口**已实现（每 100 ms 更新一次）；**LRA（响度范围）**没有实现
//!    （它需要按 −20 LU 相对门限对短时值做直方图，属于后续切片）。
//! 3. **真峰值模式**：[ARCH-UI-002] 的"真峰值"由 [`crate::meter::TruePeakDetector`]
//!    单独提供；BS.1770-4 里"用真峰值防止 MP3 过载"的那条路径没有接。
//! 4. **多声道权重与 LFE**：只支持单声道与立体声，通道权重恒为 1；
//!    5.1 的环绕权重（1.0 / 1.41）与 LFE 排除未实现。
//! 5. **采样率**：K 加权系数是**采样率相关**的。本实现内置
//!    [`K_WEIGHTING_SAMPLE_RATES_HZ`]（44.1 / 48 / 88.2 / 96 kHz）四档，
//!    由**同一条解析原型**（BS.1770-4 的高架 + RLB 高通参数）经双线性变换推导，
//!    推导口径、验证方式与逐率对照见 `docs/ledger/dsp-loudness-notes.md` §3。
//!    其它采样率一律由 [`KWeighting::for_sample_rate`] 明确**拒绝**，
//!    而不是拿 48 kHz 的系数硬套（那会给出错误读数：96 kHz 的宽带信号实测偏 0.80 dB）。
//!
//! ## 不改既有语义（重要）
//!
//! [`LoudnessMeter`] 的全部方法（`new_48k`/`add_*`/`mean_square`/`loudness_lufs`/
//! `integrated_*`）在本线**一位都没有改**，48 kHz 的滤波器系数
//! （[`SHELF_48K`]/[`HIGH_PASS_48K`]）也是原样字面量 —— 既有判据因此仍然逐位通过。
//! 被**显式改写**的只有两条与"无门限"这一语义直接冲突的判据
//! （见 [`tests::silence_padding_pulls_ungated_loudness_down`] 与
//! [`tests::sample_rate_support_covers_the_four_derived_tables`] 的注释）。
//!
//! ## 数值纪律
//!
//! 滤波器状态与累加器用 `f64`：BS.1770 的参考实现就是双精度，而 RLB 高通的极点
//! 非常靠近单位圆（`a2 = 0.99007`），`f32` 状态会在长积分上引入可见漂移。
//! 样本本身仍是 `f32`（与引擎的块接口一致）。零分配、零锁、零 I/O [ARCH-RT-001]。

/// LUFS 标定偏置（dB）。见模块文档"为什么是 −0.691"。
pub const LUFS_OFFSET_DB: f64 = -0.691;

/// K 加权系数的**参考采样率**（Hz）：BS.1770-4 正文给出系数表的那一档。
pub const K_WEIGHTING_SAMPLE_RATE_HZ: f32 = 48_000.0;

/// 内置并核验过的采样率（Hz），升序。
///
/// 四档都由**同一条解析原型**经双线性变换推导（推导与验证：
/// `docs/ledger/dsp-loudness-notes.md` §3），其中 48 kHz 那一档还能**复现
/// BS.1770-4 正文的系数表**（判据 [`tests::derived_coefficients_reproduce_the_itu_48k_table`]）。
pub const K_WEIGHTING_SAMPLE_RATES_HZ: [f32; 4] = [44_100.0, 48_000.0, 88_200.0, 96_000.0];

/// 门限积分的**绝对门限** `Γa`（LUFS）。低于它的 400 ms 块一律不参与积分。
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;

/// 门限积分的**相对门限** `Γr`（LU）：`Γr = (过绝对门限块的平均响度) − 10`。
pub const RELATIVE_GATE_LU: f64 = -10.0;

/// 门限块时长（秒）：400 ms。
pub const GATING_BLOCK_SECONDS: f32 = 0.4;

/// 门限块之间的步进（秒）：100 ms ⇒ **75% 重叠**。
pub const GATING_HOP_SECONDS: f32 = 0.1;

/// 瞬时（momentary）窗口时长（秒）。
pub const MOMENTARY_SECONDS: f32 = 0.4;

/// 短时（short-term）窗口时长（秒）。
pub const SHORT_TERM_SECONDS: f32 = 3.0;

/// 一个门限块 = 几个 100 ms 跳（`400 ms / 100 ms`）。
const GATING_HOPS_PER_BLOCK: usize = 4;

/// 3 s 短时窗口 = 几个 100 ms 跳（`3 s / 100 ms`）。
const SHORT_TERM_HOPS: usize = 30;

/// BS.1770-4 Stage 1 高架滤波器 `(b0, b1, b2, a1, a2)` @48 kHz（`a0` 已归一化为 1）。
///
/// **原样保留**上一线的字面量：48 kHz 路径的逐位不变契约靠它（改一位就会让既有的
/// 285 条冻结位模式里与 LUFS 相关的读数漂移）。
const SHELF_48K: [f64; 5] = [
    1.535_124_859_586_97,
    -2.691_696_189_406_38,
    1.198_392_810_852_85,
    -1.690_659_293_182_41,
    0.732_480_774_215_85,
];

/// BS.1770-4 Stage 2 RLB 高通滤波器 `(b0, b1, b2, a1, a2)` @48 kHz。
const HIGH_PASS_48K: [f64; 5] = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];

/// Stage 1 高架 @44.1 kHz（同一条解析原型，`K = tan(π·f0/fs)`）。
const SHELF_44K1: [f64; 5] = [
    1.5308412300503478,
    -2.6509799951547297,
    1.169079079921587,
    -1.6636551132560204,
    0.7125954280732254,
];

/// Stage 2 RLB 高通 @44.1 kHz。
const HIGH_PASS_44K1: [f64; 5] = [1.0, -2.0, 1.0, -1.989169673629796, 0.9891990357870393];

/// Stage 1 高架 @88.2 kHz。
const SHELF_88K2: [f64; 5] = [
    1.557515375579654,
    -2.905627079926345,
    1.3613339774722122,
    -1.8309199879623321,
    0.8441422610878527,
];

/// Stage 2 RLB 高通 @88.2 kHz。
const HIGH_PASS_88K2: [f64; 5] = [1.0, -2.0, 1.0, -1.9945775154503445, 0.9945848758780549];

/// Stage 1 高架 @96 kHz。
const SHELF_96K: [f64; 5] = [
    1.5597142289757966,
    -2.9267415782510824,
    1.3782612023158187,
    -1.8446094698901085,
    0.8558433229306412,
];

/// Stage 2 RLB 高通 @96 kHz。
const HIGH_PASS_96K: [f64; 5] = [1.0, -2.0, 1.0, -1.9950175447247156, 0.9950237590409233];

/// 取某个采样率的两级系数；不在 [`K_WEIGHTING_SAMPLE_RATES_HZ`] 里就返回 `None`。
///
/// 只做**精确相等**比较：系数是按离散的采样率档位推导并逐档核验的，
/// "就近取一档"会让读数在档位之间悄悄错掉（BS.1770 的系数不是连续插值的）。
#[must_use]
fn coefficients_for(sample_rate: f32) -> Option<([f64; 5], [f64; 5])> {
    if sample_rate == 44_100.0 {
        Some((SHELF_44K1, HIGH_PASS_44K1))
    } else if sample_rate == K_WEIGHTING_SAMPLE_RATE_HZ {
        Some((SHELF_48K, HIGH_PASS_48K))
    } else if sample_rate == 88_200.0 {
        Some((SHELF_88K2, HIGH_PASS_88K2))
    } else if sample_rate == 96_000.0 {
        Some((SHELF_96K, HIGH_PASS_96K))
    } else {
        None
    }
}

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

    /// 按采样率构造；只接受 [`K_WEIGHTING_SAMPLE_RATES_HZ`] 里推导并核验过的四档，
    /// 其余返回 `None`。
    ///
    /// 明确拒绝而不是拿 48 kHz 系数硬套：BS.1770 的系数是采样率相关的，
    /// 硬套会得到错误的 LUFS（96 kHz 的宽带夹具上**实测偏 0.82 dB** ——
    /// 属于"说谎"而不是"近似"）。
    #[must_use]
    pub fn for_sample_rate(sample_rate: f32) -> Option<Self> {
        let (shelf, high_pass) = coefficients_for(sample_rate)?;
        Some(Self {
            left: [Biquad::new(shelf), Biquad::new(high_pass)],
            right: [Biquad::new(shelf), Biquad::new(high_pass)],
        })
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

    /// 按采样率的新计量器；不在 [`K_WEIGHTING_SAMPLE_RATES_HZ`] 里就返回 `None`。
    ///
    /// 与 [`KWeighting::for_sample_rate`] 同一份（逐档核验过的）系数。
    #[must_use]
    pub fn for_sample_rate(sample_rate: f32) -> Option<Self> {
        Some(Self {
            k: KWeighting::for_sample_rate(sample_rate)?,
            sum_squares: 0.0,
            frames: 0,
        })
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
        lufs_of(self.mean_square())
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

// ---------------------------------------------------------------------------
// 门限积分响度 + 瞬时/短时窗口（BS.1770-4 的"计量"部分）——HD-27
// ---------------------------------------------------------------------------

/// LUFS 的 `f64` 计算（供门限比较用；与 [`LoudnessMeter::loudness_lufs`] 同一表达式）。
#[must_use]
fn lufs_f64(mean_square: f64) -> f64 {
    if mean_square > 0.0 {
        LUFS_OFFSET_DB + 10.0 * mean_square.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// LUFS 读数（`f32`，与上一线的口径一致；能量为 0 ⇒ 负无穷，**不是** `NaN`）。
#[must_use]
fn lufs_of(mean_square: f64) -> f32 {
    lufs_f64(mean_square) as f32
}

/// 100 ms 跳的样本数（四档采样率下都是整数：44.1k→4410、48k→4800、88.2k→8820、96k→9600）。
#[must_use]
fn hop_samples(sample_rate: f32) -> usize {
    (f64::from(sample_rate) * f64::from(GATING_HOP_SECONDS)).round() as usize
}

/// 走一遍信号，对每个**完整**的 400 ms 块（75% 重叠）回调它的均方 `z`。
///
/// `right == None` 表示**单声道**：只走左声道那套状态，能量是 `Σy²`（通道权重 1，与
/// [`LoudnessMeter::add_mono`] 同一约定 —— 单声道**不是**"同一个信号喂两个通道"，
/// 后者会平白多 3.01 dB）。
///
/// 零分配：块 = 4 个跳 ⇒ 只需要 4 个 `f64` 的环。不足一个块的尾巴**丢弃**
/// （与 BS.1770-4 的参考实现一致：门限块必须完整）。
fn visit_gating_blocks(
    sample_rate: f32,
    left: &[f32],
    right: Option<&[f32]>,
    mut visit: impl FnMut(f64),
) {
    let hop = hop_samples(sample_rate);
    if hop == 0 {
        return;
    }
    let block = hop * GATING_HOPS_PER_BLOCK;
    let frames = match right {
        Some(right) => left.len().min(right.len()),
        None => left.len(),
    };
    if frames < block {
        return;
    }
    let Some(mut weighting) = KWeighting::for_sample_rate(sample_rate) else {
        return;
    };
    let mut ring = [0.0f64; GATING_HOPS_PER_BLOCK];
    let mut ring_pos = 0usize;
    let mut filled = 0usize;
    let mut hop_sum = 0.0f64;
    let mut hop_frames = 0usize;
    for index in 0..frames {
        let energy = match right {
            Some(right) => {
                let (l, r) = weighting.process_stereo(clean(left[index]), clean(right[index]));
                l * l + r * r
            }
            None => {
                let y = weighting.process_mono(clean(left[index]));
                y * y
            }
        };
        hop_sum += energy;
        hop_frames += 1;
        if hop_frames == hop {
            ring[ring_pos] = hop_sum;
            ring_pos += 1;
            if ring_pos == GATING_HOPS_PER_BLOCK {
                ring_pos = 0;
            }
            if filled < GATING_HOPS_PER_BLOCK {
                filled += 1;
            }
            hop_sum = 0.0;
            hop_frames = 0;
            if filled == GATING_HOPS_PER_BLOCK {
                let sum: f64 = ring.iter().sum();
                visit(sum / block as f64);
            }
        }
    }
}

/// BS.1770-4 的**门限积分**响度 + 瞬时（400 ms）/短时（3 s）窗口。
///
/// ## 口径（全部由公开常量给出）
///
/// - **步进**：每 100 ms（[`GATING_HOP_SECONDS`]）收一个能量跳；
/// - **瞬时 momentary**：最近 [`GATING_HOPS_PER_BLOCK`] 个跳（= 400 ms）的均方 → LUFS；
/// - **短时 short-term**：最近 [`SHORT_TERM_HOPS`] 个跳（= 3 s）的均方 → LUFS；
/// - **积分 integrated**：400 ms 块 + 75% 重叠（块 = 4 个跳），
///   先按 `Γa = −70 LUFS`（[`ABSOLUTE_GATE_LUFS`]）过绝对门限，
///   再按 `Γr = (过绝对门限块的平均响度) − 10 LU`（[`RELATIVE_GATE_LU`]）过相对门限，
///   最后对留下来的块求能量平均。
///
/// ## 用法（流式窗口 vs 离线积分）
///
/// ```text
/// 流式（有界, 30 个 f64 的环, 零分配）: add_mono/add_stereo → momentary_lufs/short_term_lufs
/// 离线（两遍滤波, 零分配零状态）:        GatedLoudness::integrated_stereo(left, right)
/// ```
///
/// **为什么积分是两遍**：相对门限 `Γr` 依赖整段信号里"过绝对门限的块"的平均响度，
/// 所以精确的积分值必须回看全部块。两遍滤波换来的是**逐位精确 + 零分配 + 零状态**；
/// 要做流式 integrated 就得存下所有块（无界）或做有损直方图（见模块文档第 1 条）。
///
/// ```
/// use yeban_dsp::loudness::GatedLoudness;
///
/// // 1 秒 −20 LUFS 的 997 Hz 正弦 + 1 秒静音:
/// // 无门限会被拉到 −23.01, 门限积分只损失边界块 ⇒ ≈ −20.71 LUFS。
/// let mut samples: Vec<f32> = (0..48_000)
///     .map(|i| (0.1 * (std::f64::consts::TAU * 997.0 * i as f64 / 48_000.0).sin()) as f32)
///     .collect();
/// samples.extend(std::iter::repeat_n(0.0f32, 48_000));
/// let gated = GatedLoudness::integrated_stereo(&samples, &samples);
/// assert!((gated + 20.71).abs() < 0.05, "实际 {gated}");
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatedLoudness {
    /// 采样率（Hz）。
    sample_rate: f32,
    /// 两级 K 加权（左右各一套状态）。
    k: KWeighting,
    /// 一个跳的样本数。
    hop_samples: usize,
    /// 当前跳已累计的帧数。
    hop_frames: usize,
    /// 当前跳的能量和 `Σ(y_L² + y_R²)`。
    hop_sum: f64,
    /// 最近 [`SHORT_TERM_HOPS`] 个跳的能量和（环形）。
    ring: [f64; SHORT_TERM_HOPS],
    /// 环形写指针。
    ring_pos: usize,
    /// 环里已有的跳数（≤ [`SHORT_TERM_HOPS`]）。
    ring_filled: usize,
    /// 最近一个**完整**的 400 ms 窗口读数。
    momentary_lufs: f32,
    /// 最近一个**完整**的 3 s 窗口读数。
    short_term_lufs: f32,
    /// 迄今最大的瞬时读数。
    max_momentary_lufs: f32,
    /// 迄今最大的短时读数。
    max_short_term_lufs: f32,
}

impl Default for GatedLoudness {
    fn default() -> Self {
        Self::new_48k()
    }
}

impl GatedLoudness {
    /// 48 kHz 的新计量器（状态归零）。
    #[must_use]
    pub fn new_48k() -> Self {
        Self::for_sample_rate(K_WEIGHTING_SAMPLE_RATE_HZ).expect("48 kHz 的 K 加权系数是内置的")
    }

    /// 按采样率构造；不在 [`K_WEIGHTING_SAMPLE_RATES_HZ`] 里就返回 `None`。
    #[must_use]
    pub fn for_sample_rate(sample_rate: f32) -> Option<Self> {
        let k = KWeighting::for_sample_rate(sample_rate)?;
        Some(Self {
            sample_rate,
            k,
            hop_samples: hop_samples(sample_rate),
            hop_frames: 0,
            hop_sum: 0.0,
            ring: [0.0; SHORT_TERM_HOPS],
            ring_pos: 0,
            ring_filled: 0,
            momentary_lufs: f32::NEG_INFINITY,
            short_term_lufs: f32::NEG_INFINITY,
            max_momentary_lufs: f32::NEG_INFINITY,
            max_short_term_lufs: f32::NEG_INFINITY,
        })
    }

    /// 清空滤波器状态与所有窗口（配置不变）。
    pub fn reset(&mut self) {
        self.k.reset();
        self.hop_frames = 0;
        self.hop_sum = 0.0;
        self.ring = [0.0; SHORT_TERM_HOPS];
        self.ring_pos = 0;
        self.ring_filled = 0;
        self.momentary_lufs = f32::NEG_INFINITY;
        self.short_term_lufs = f32::NEG_INFINITY;
        self.max_momentary_lufs = f32::NEG_INFINITY;
        self.max_short_term_lufs = f32::NEG_INFINITY;
    }

    /// 采样率（Hz）。
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// 一个 100 ms 跳的样本数。
    #[must_use]
    pub const fn hop_samples(&self) -> usize {
        self.hop_samples
    }

    /// 最近一个完整 400 ms 窗口的瞬时响度；窗口还没满 ⇒ 负无穷。
    #[must_use]
    pub const fn momentary_lufs(&self) -> f32 {
        self.momentary_lufs
    }

    /// 最近一个完整 3 s 窗口的短时响度；窗口还没满 ⇒ 负无穷。
    #[must_use]
    pub const fn short_term_lufs(&self) -> f32 {
        self.short_term_lufs
    }

    /// 迄今最大的瞬时读数（`GatedLoudness` 的 `MaxMomentary`，RF64/BW64 的响度块会用到）。
    #[must_use]
    pub const fn max_momentary_lufs(&self) -> f32 {
        self.max_momentary_lufs
    }

    /// 迄今最大的短时读数（`MaxShortTerm`）。
    #[must_use]
    pub const fn max_short_term_lufs(&self) -> f32 {
        self.max_short_term_lufs
    }

    /// 喂入一段**单声道**样本（通道权重 1）。
    ///
    /// 单声道**不是**"同一个信号喂两个通道"：后者会平白多 3.01 dB。
    /// 判据 [`tests::gated_loudness_mono_matches_one_silent_channel`] 钉住这个约定。
    pub fn add_mono(&mut self, samples: &[f32]) {
        for &sample in samples {
            let y = self.k.process_mono(clean(sample));
            self.push_frame(y, None);
        }
    }

    /// 喂入一段**立体声**样本；长度不等时按较短者工作（**不 panic**）。
    pub fn add_stereo(&mut self, left: &[f32], right: &[f32]) {
        let frames = left.len().min(right.len());
        for index in 0..frames {
            let (l, r) = self
                .k
                .process_stereo(clean(left[index]), clean(right[index]));
            self.push_frame(l, Some(r));
        }
    }

    /// 推进一帧（K 加权后的样本），必要时结算一个 100 ms 跳与窗口。
    ///
    /// `right == None` ⇒ 单声道：只累计左声道那一路的能量。
    fn push_frame(&mut self, left: f64, right: Option<f64>) {
        self.hop_sum += match right {
            Some(right) => left * left + right * right,
            None => left * left,
        };
        self.hop_frames += 1;
        if self.hop_frames == self.hop_samples {
            self.finish_hop();
        }
    }

    /// 结算一个跳：入环、必要时更新瞬时/短时窗口与各自的最大值。
    fn finish_hop(&mut self) {
        self.ring[self.ring_pos] = self.hop_sum;
        self.ring_pos += 1;
        if self.ring_pos == SHORT_TERM_HOPS {
            self.ring_pos = 0;
        }
        if self.ring_filled < SHORT_TERM_HOPS {
            self.ring_filled += 1;
        }
        self.hop_sum = 0.0;
        self.hop_frames = 0;
        if self.ring_filled >= GATING_HOPS_PER_BLOCK {
            self.momentary_lufs = lufs_of(self.window_mean_square(GATING_HOPS_PER_BLOCK));
            if self.momentary_lufs > self.max_momentary_lufs {
                self.max_momentary_lufs = self.momentary_lufs;
            }
        }
        if self.ring_filled == SHORT_TERM_HOPS {
            self.short_term_lufs = lufs_of(self.window_mean_square(SHORT_TERM_HOPS));
            if self.short_term_lufs > self.max_short_term_lufs {
                self.max_short_term_lufs = self.short_term_lufs;
            }
        }
    }

    /// 最近 `hops` 个 100 ms 跳的均方（`Σ能量 / (hops · hop_samples)`）。
    fn window_mean_square(&self, hops: usize) -> f64 {
        let mut sum = 0.0f64;
        for offset in 0..hops.min(self.ring_filled) {
            let index = (self.ring_pos + SHORT_TERM_HOPS - 1 - offset) % SHORT_TERM_HOPS;
            sum += self.ring[index];
        }
        let frames = (hops * self.hop_samples) as f64;
        if frames > 0.0 { sum / frames } else { 0.0 }
    }

    /// 一次性测一段**单声道**的**门限积分**响度（48 kHz）。
    #[must_use]
    pub fn integrated_mono(samples: &[f32]) -> f32 {
        Self::integrated_mono_at(K_WEIGHTING_SAMPLE_RATE_HZ, samples)
            .expect("48 kHz 的 K 加权系数是内置的")
    }

    /// 一次性测一段**立体声**的**门限积分**响度（48 kHz）。
    #[must_use]
    pub fn integrated_stereo(left: &[f32], right: &[f32]) -> f32 {
        Self::integrated_stereo_at(K_WEIGHTING_SAMPLE_RATE_HZ, left, right)
            .expect("48 kHz 的 K 加权系数是内置的")
    }

    /// 按采样率测一段单声道的门限积分响度；采样率不支持 ⇒ `None`。
    ///
    /// 单声道口径 = 单通道能量（与 [`LoudnessMeter::add_mono`] 一致），
    /// **不是**"同一个信号喂两个通道"。
    #[must_use]
    pub fn integrated_mono_at(sample_rate: f32, samples: &[f32]) -> Option<f32> {
        Self::integrated_at(sample_rate, samples, None)
    }

    /// 按采样率测一段立体声的门限积分响度；采样率不支持 ⇒ `None`。
    ///
    /// 两遍滤波（见类型文档"为什么积分是两遍"）。没有任何块能过绝对门限
    /// （例如整段安静于 −70 LUFS，或长度不足 400 ms）⇒ 负无穷。
    #[must_use]
    pub fn integrated_stereo_at(sample_rate: f32, left: &[f32], right: &[f32]) -> Option<f32> {
        Self::integrated_at(sample_rate, left, Some(right))
    }

    /// 门限积分的公共实现：`right == None` ⇒ 单声道（单通道能量）。
    #[must_use]
    fn integrated_at(sample_rate: f32, left: &[f32], right: Option<&[f32]>) -> Option<f32> {
        coefficients_for(sample_rate)?;
        // 第一遍: 绝对门限筛出 Jg, 并用它的平均响度定出相对门限 Γr。
        let mut gated_sum = 0.0f64;
        let mut gated_count = 0usize;
        visit_gating_blocks(sample_rate, left, right, |z| {
            if lufs_f64(z) > ABSOLUTE_GATE_LUFS {
                gated_sum += z;
                gated_count += 1;
            }
        });
        if gated_count == 0 {
            return Some(f32::NEG_INFINITY);
        }
        let relative_gate = lufs_f64(gated_sum / gated_count as f64) + RELATIVE_GATE_LU;
        // 第二遍: 取 max(Γa, Γr) 之上的块求平均 —— 这才是 BS.1770-4 的积分响度。
        let mut sum = 0.0f64;
        let mut count = 0usize;
        visit_gating_blocks(sample_rate, left, right, |z| {
            let loudness = lufs_f64(z);
            if loudness > ABSOLUTE_GATE_LUFS && loudness > relative_gate {
                sum += z;
                count += 1;
            }
        });
        if count == 0 {
            return Some(f32::NEG_INFINITY);
        }
        Some(lufs_of(sum / count as f64))
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

    /// **判据（新写，可红）**：`GatedLoudness::reset` 之后的实例与**全新实例**在
    /// 同样的流式输入下给出逐位相同的窗口读数。
    ///
    /// 量什么：瞬时读数与最大瞬时读数（LUFS，`f32` 位型）。
    ///
    /// 夹具刻意在**复位之后只喂 3 个跳（300 ms）**：结算一个跳时用"环里已有的跳数"
    /// 决定瞬时窗口是否成立，而瞬时窗口需要 4 个跳 ⇒ 全新实例此时读出 `−∞`，
    /// 而带陈旧跳数的实例会提前给出有限读数。注入实测：去掉
    /// `self.ring_filled = 0;` ⇒ 既有全量判据**全绿**。
    #[test]
    fn reset_reproduces_a_freshly_built_gated_meter_bit_for_bit() {
        let samples: Vec<f32> = (0..24_000)
            .map(|index| {
                let phase = std::f64::consts::TAU * 997.0 * index as f64 / 48_000.0;
                (phase.sin() * 0.25) as f32
            })
            .collect();
        // 先喂 0.5 s（5 个跳）把"环里已有的跳数"推到瞬时窗口之上。
        let mut used = GatedLoudness::new_48k();
        used.add_stereo(&samples, &samples);
        assert!(used.momentary_lufs().is_finite(), "预热必须让瞬时窗口成立");
        used.reset();
        // 复位之后只喂 0.3 s（3 个跳）：全新实例的瞬时窗口还不成立。
        let short = &samples[..14_400];
        let drive = |meter: &mut GatedLoudness| -> (f32, f32) {
            meter.add_stereo(short, short);
            (meter.momentary_lufs(), meter.max_momentary_lufs())
        };
        let after = drive(&mut used);
        let fresh = drive(&mut GatedLoudness::new_48k());
        assert_eq!(
            fresh.0,
            f32::NEG_INFINITY,
            "全新实例在 3 个跳时不应有瞬时读数"
        );
        assert_eq!(
            after.0.to_bits(),
            fresh.0.to_bits(),
            "复位后的瞬时读数不一致"
        );
        assert_eq!(
            after.1.to_bits(),
            fresh.1.to_bits(),
            "复位后的最大瞬时读数不一致"
        );
    }

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
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

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

    /// 判据：双二阶的直流/奈奎斯特增益与解析值一致（系数本身没被改坏）。
    #[test]
    fn biquad_gains_match_the_itu_analytic_values() {
        // Stage 1 直流增益 ≈ 1; Nyquist 增益 ≈ +4 dB。
        assert!((shelf_dc_gain() - 1.0).abs() < 1e-3);
        assert!((shelf_nyquist_gain_db() - 3.999).abs() < 0.01);
        // Stage 2 直流增益 ≈ 0（高通）；997 Hz 处幅度 ≈ 1。
        assert!(high_pass_dc_gain().abs() < 1e-3);
        assert!((magnitude_at(HIGH_PASS_48K, 997.0, 48_000.0) - 1.0035).abs() < 1e-3);
        // 级联在 997 Hz 的幅度增益 ≈ +0.691 dB（标定常数 −0.691 的来源）。
        let combined_db = 20.0
            * (magnitude_at(SHELF_48K, 997.0, 48_000.0)
                * magnitude_at(HIGH_PASS_48K, 997.0, 48_000.0))
            .log10();
        assert!(
            (combined_db - 0.691).abs() < 0.01,
            "997 Hz 级联增益应为 +0.691 dB, 实际 {combined_db}"
        );
    }

    // -----------------------------------------------------------------------
    // 采样率与系数推导（HD-27 的第 3 条）
    // -----------------------------------------------------------------------

    /// 以 `fs` 为单位的正弦夹具（相位用 `f64` 算，避免大自变量的 `f32` 正弦漂移）。
    fn sine_at_rate(amplitude: f32, frequency: f64, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let phase = std::f64::consts::TAU * frequency * i as f64 / f64::from(sample_rate);
                (f64::from(amplitude) * phase.sin()) as f32
            })
            .collect()
    }

    /// 宽带夹具：100 / 1000 / 5000 Hz 三条正弦之和（峰值 ≈ 0.2）。
    fn wideband(sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f64 / f64::from(sample_rate);
                let value = (std::f64::consts::TAU * 100.0 * t).sin()
                    + (std::f64::consts::TAU * 1000.0 * t).sin()
                    + (std::f64::consts::TAU * 5000.0 * t).sin();
                (0.2 * value / 3.0) as f32
            })
            .collect()
    }

    /// BS.1770-4 两级 K 加权的**解析原型**（本实现四档系数表的唯一来源）。
    ///
    /// 参数取自标准的滤波器设计描述（高架：`f0 ≈ 1681.97 Hz`、`G ≈ +4 dB`、
    /// `Q ≈ 0.7072`、`Vb = Vh^0.4997`；RLB 高通：`f0 ≈ 38.135 Hz`、`Q ≈ 0.50033`），
    /// 变换是**预扭曲双线性变换**：`K = tan(π·f0/fs)`，两级的分母都按 `a0` 归一化，
    /// 高通的分子固定为 `(1 − z⁻¹)²`（这正是 BS.1770 表 2 的形状）。
    ///
    /// 判据 [`derived_coefficients_reproduce_the_itu_48k_table`] 用**标准正文的 48 kHz
    /// 系数表**做锚点验证这条推导（复现到 ≤3.3e-16 相对误差，即 1–2 ulp of f64）。
    fn derived_coefficients(sample_rate: f64) -> ([f64; 5], [f64; 5]) {
        const F0_SHELF: f64 = 1681.974450955533;
        const G_SHELF: f64 = 3.999843853973347;
        const Q_SHELF: f64 = 0.7071752369554196;
        const VB_EXP: f64 = 0.4996667741545416;
        const F0_HIGH_PASS: f64 = 38.13547087602444;
        const Q_HIGH_PASS: f64 = 0.5003270373238773;

        let k = (std::f64::consts::PI * F0_SHELF / sample_rate).tan();
        let vh = 10f64.powf(G_SHELF / 20.0);
        let vb = vh.powf(VB_EXP);
        let a0 = 1.0 + k / Q_SHELF + k * k;
        let shelf = [
            (vh + vb * k / Q_SHELF + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / Q_SHELF + k * k) / a0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / Q_SHELF + k * k) / a0,
        ];
        let k = (std::f64::consts::PI * F0_HIGH_PASS / sample_rate).tan();
        let a0 = 1.0 + k / Q_HIGH_PASS + k * k;
        let high_pass = [
            1.0,
            -2.0,
            1.0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / Q_HIGH_PASS + k * k) / a0,
        ];
        (shelf, high_pass)
    }

    /// 判据：**系数推导的锚点是标准正文**（不是"我们自己的另一份常数"）。
    ///
    /// 1. 解析原型 ⇒ **复现 BS.1770-4 表 1 / 表 2 的 48 kHz 系数**（容差 1e-12：
    ///    实测 ≤3.3e-16，即 1–2 ulp of f64；而任何真实的推导错误 —— 例如把
    ///    `tan` 换成 `sin`、或漏掉预扭曲 —— 都会差 ≫1e-3）；
    /// 2. 同一推导 ⇒ **内置的四档系数表**逐项一致（48 kHz 那一档是正文的原样字面量，
    ///    其余三档是本线按同一条推导生成的）。
    ///
    /// 注入：改内置表里任意一个系数（哪怕 1e-9）⇒ 本判据变红。
    #[test]
    fn derived_coefficients_reproduce_the_itu_48k_table() {
        /// BS.1770-4 表 1：Stage 1 高架 @48 kHz。
        const ITU_SHELF_48K: [f64; 5] = [
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
            -1.690_659_293_182_41,
            0.732_480_774_215_85,
        ];
        /// BS.1770-4 表 2：Stage 2 RLB 高通 @48 kHz。
        const ITU_HIGH_PASS_48K: [f64; 5] =
            [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];

        let (shelf, high_pass) = derived_coefficients(48_000.0);
        for (index, (got, want)) in shelf.iter().zip(ITU_SHELF_48K.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-12,
                "推导的 shelf[{index}] = {got} 与标准正文的 {want} 不符"
            );
        }
        for (index, (got, want)) in high_pass.iter().zip(ITU_HIGH_PASS_48K.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-12,
                "推导的 high_pass[{index}] = {got} 与标准正文的 {want} 不符"
            );
        }

        for rate in K_WEIGHTING_SAMPLE_RATES_HZ {
            let (shelf, high_pass) = derived_coefficients(f64::from(rate));
            let (embedded_shelf, embedded_high_pass) = coefficients_for(rate).expect("内置档位");
            for (index, (got, want)) in shelf.iter().zip(embedded_shelf.iter()).enumerate() {
                assert!(
                    (got - want).abs() < 1e-12,
                    "{rate} Hz 的 shelf[{index}]: 内置 {want} ≠ 推导 {got}"
                );
            }
            for (index, (got, want)) in high_pass.iter().zip(embedded_high_pass.iter()).enumerate()
            {
                assert!(
                    (got - want).abs() < 1e-12,
                    "{rate} Hz 的 high_pass[{index}]: 内置 {want} ≠ 推导 {got}"
                );
            }
        }
    }

    /// 判据：**每档采样率上的 997 Hz 标定点**都在 ±0.025 dB 内。
    ///
    /// 实测（无门限与门限积分两条路径都测）：
    ///
    /// | 采样率 | 读数 | 相对 −20.0 的偏差 |
    /// | ---: | ---: | ---: |
    /// | 44.1 kHz | −19.997 | +0.003 |
    /// | 48 kHz | −20.000 | 0.000 |
    /// | 88.2 kHz | −20.016 | −0.016 |
    /// | 96 kHz | −20.017 | −0.017 |
    ///
    /// 残余偏差的来源是**标准只给了一个 −0.691 常数**（它是 48 kHz 的标定值），
    /// 而数字滤波器在 997 Hz 的增益随采样率略有不同（双线性变换的频率扭曲）。
    /// 0.02 dB 远小于任何可听的/可测的响度差，也远小于换错系数表的后果。
    ///
    /// 注入：把某一档的两级系数整体换成 48 kHz 的 ⇒ 该档的 997 Hz 标定点实测变成
    /// **−19.791**（44.1 kHz，偏 0.21 dB）、**−20.622**（88.2 kHz）、
    /// **−20.650**（96 kHz，偏 0.65 dB）⇒ 全部变红。
    #[test]
    fn minus_twenty_dbfs_997hz_is_minus_twenty_lufs_at_every_supported_rate() {
        for rate in K_WEIGHTING_SAMPLE_RATES_HZ {
            let samples = sine_at_rate(0.1, 997.0, rate, rate as usize);
            let mut ungated_meter = LoudnessMeter::for_sample_rate(rate).expect("内置档位");
            ungated_meter.add_stereo(&samples, &samples);
            let ungated = ungated_meter.loudness_lufs();
            let gated =
                GatedLoudness::integrated_stereo_at(rate, &samples, &samples).expect("内置档位");
            assert!(
                (ungated + 20.0).abs() < 0.025,
                "{rate} Hz 的无门限读数 {ungated} 偏离 −20.0"
            );
            assert!(
                (gated + 20.0).abs() < 0.025,
                "{rate} Hz 的门限读数 {gated} 偏离 −20.0"
            );
        }
    }

    /// 判据：**同一段模拟信号在不同采样率下的读数一致**（系数推导正确性的行为证据）。
    ///
    /// 宽带夹具（100 + 1000 + 5000 Hz）在四档采样率上的读数**实测**：
    /// 44.1 kHz −17.711996、48 kHz −17.715944、88.2 kHz −17.736189、96 kHz −17.738140
    /// —— 最大跨度 **0.0261 dB**（容差取 0.05 dB）。
    ///
    /// 反例（同一判据的"注入"侧）：拿 48 kHz 的系数去算 **96 kHz** 的信号，
    /// 实测读到 **−18.538059**，与正确的 −17.738140 差 **0.822 dB** ⇒ 远超容差。
    /// 这条比较在判据里**显式跑一遍**（`KWeighting::new_48k` 对 96 kHz 的信号），
    /// 让"不能硬套 48 kHz 系数"这句话有数字支撑，而不是只写在注释里。
    #[test]
    fn k_weighting_is_rate_consistent_on_a_wideband_fixture() {
        let mut readings = Vec::new();
        for rate in K_WEIGHTING_SAMPLE_RATES_HZ {
            let samples = wideband(rate, (5.0 * f64::from(rate)) as usize);
            let reading =
                GatedLoudness::integrated_stereo_at(rate, &samples, &samples).expect("内置档位");
            readings.push((rate, reading));
        }
        let reference = readings[1].1;
        for (rate, reading) in &readings {
            assert!(
                (reading - reference).abs() < 0.05,
                "{rate} Hz 的宽带读数是 {reading}, 与 48 kHz 的 {reference} 相差过大"
            );
        }

        // 反例: 96 kHz 的信号用 48 kHz 的系数硬套。
        let rate = 96_000.0f32;
        let samples = wideband(rate, (5.0 * f64::from(rate)) as usize);
        let mut wrong_meter = LoudnessMeter::new_48k();
        wrong_meter.add_stereo(&samples, &samples);
        let wrong = wrong_meter.loudness_lufs();
        let right = readings[3].1;
        assert!(
            (wrong - right).abs() > 0.5,
            "硬套 48 kHz 系数必须显著走样: wrong={wrong} right={right}"
        );
    }

    /// 判据：采样率支持是**显式枚举**的 —— 四档有系数，其余明确拒绝。
    ///
    /// ⚠ 与上一线相比这是**显式改写的语义**（HD-27 要求补齐其它采样率）：
    /// 旧判据（`only_the_verified_sample_rate_is_accepted`）要求 44.1/88.2/96 kHz
    /// 返回 `None`；本线按裁决把它们变成了**有推导、有验证的档位**。
    /// 仍然拒绝的采样率（22.05/32/192 kHz、0、负数、NaN）一律是显式 `None`，
    /// 不允许"就近取一档"。
    #[test]
    fn sample_rate_support_covers_the_four_derived_tables() {
        assert_eq!(
            K_WEIGHTING_SAMPLE_RATES_HZ,
            [44_100.0, 48_000.0, 88_200.0, 96_000.0]
        );
        for rate in K_WEIGHTING_SAMPLE_RATES_HZ {
            assert!(
                KWeighting::for_sample_rate(rate).is_some(),
                "{rate} 应有系数"
            );
            assert!(LoudnessMeter::for_sample_rate(rate).is_some());
            assert!(GatedLoudness::for_sample_rate(rate).is_some());
            assert!(GatedLoudness::integrated_stereo_at(rate, &[], &[]).is_some());
        }
        for rate in [
            0.0f32,
            -48_000.0,
            22_050.0,
            32_000.0,
            88_201.0,
            96_001.0,
            192_000.0,
            f32::NAN,
        ] {
            assert!(
                KWeighting::for_sample_rate(rate).is_none(),
                "{rate} Hz 没有推导过的系数, 必须拒绝而不是硬套 48 kHz"
            );
            assert!(LoudnessMeter::for_sample_rate(rate).is_none());
            assert!(GatedLoudness::for_sample_rate(rate).is_none());
            assert!(GatedLoudness::integrated_stereo_at(rate, &[], &[]).is_none());
        }
    }

    // -----------------------------------------------------------------------
    // 门限与窗口（HD-27 的第 1、2 条）
    // -----------------------------------------------------------------------

    /// 判据：门限/窗口的**口径常量**被写死钉住（防止"判据与常量同源"的假绿）。
    #[test]
    fn gate_thresholds_and_window_lengths_are_the_documented_values() {
        assert_eq!(ABSOLUTE_GATE_LUFS, -70.0);
        assert_eq!(RELATIVE_GATE_LU, -10.0);
        assert_eq!(GATING_BLOCK_SECONDS, 0.4);
        assert_eq!(GATING_HOP_SECONDS, 0.1);
        assert_eq!(MOMENTARY_SECONDS, 0.4);
        assert_eq!(SHORT_TERM_SECONDS, 3.0);
        assert_eq!(LUFS_OFFSET_DB, -0.691);
        assert_eq!(GATING_HOPS_PER_BLOCK, 4);
        assert_eq!(SHORT_TERM_HOPS, 30);
        // 400 ms 块 = 4 × 100 ms 跳（75% 重叠）；3 s 短时 = 30 × 100 ms 跳。
        assert!((GATING_HOPS_PER_BLOCK as f32 * GATING_HOP_SECONDS - 0.4).abs() < 1e-6);
        assert!((SHORT_TERM_HOPS as f32 * GATING_HOP_SECONDS - 3.0).abs() < 1e-6);
        assert_eq!(hop_samples(48_000.0), 4_800);
        assert_eq!(hop_samples(44_100.0), 4_410);
        assert_eq!(hop_samples(96_000.0), 9_600);
    }

    /// 判据：**绝对门限两侧的行为**（−75 LUFS 被排除、−65 LUFS 被保留）。
    ///
    /// 单条 −75 LUFS 的 997 Hz 正弦：无门限读数 −75.0，**门限积分是负无穷**
    /// （所有块都在 Γa 之下 ⇒ 没有可积分的块）。
    /// 单条 −65 LUFS：两侧都在 Γa 之上 ⇒ 门限读数 ≈ −65.0。
    /// 这一对把 Γa = −70 从**两个方向**钉住（只测一侧的话，把门限改成 0 也照样过）。
    ///
    /// 注入：把 [`ABSOLUTE_GATE_LUFS`] 改成 0 ⇒ −65 那条变红（读数变成负无穷）。
    #[test]
    fn the_absolute_gate_sits_at_minus_seventy_lufs() {
        for (target, expected) in [(-75.0f64, f32::NEG_INFINITY), (-65.0, -65.0)] {
            // 997 Hz 立体声的读数 ≈ 20·log10(幅度) ⇒ 反解出目标读数所需的幅度。
            let amplitude = (0.1 * 10f64.powf((target + 20.0) / 20.0)) as f32;
            let samples = sine_at_rate(amplitude, 997.0, 48_000.0, 96_000);
            let ungated = LoudnessMeter::integrated_stereo(&samples, &samples);
            let gated = GatedLoudness::integrated_stereo(&samples, &samples);
            assert!(
                (ungated - target as f32).abs() < 0.05,
                "构造前提: 无门限读数应约 {target}, 实际 {ungated}"
            );
            if expected.is_infinite() {
                assert_eq!(gated, f32::NEG_INFINITY, "低于 Γa 的信号必须被整体排除");
            } else {
                assert!(
                    (gated - expected).abs() < 0.05,
                    "高于 Γa 的信号应读到 {expected}, 实际 {gated}"
                );
            }
        }
    }

    /// 判据：**相对门限 Γr 真的在起作用**（−20 LUFS 段 + −40 LUFS 段）。
    ///
    /// 夹具：10 秒 −20 LUFS（997 Hz 双声道）+ 10 秒 −40 LUFS。
    ///
    /// | 口径 | 读数 | 说明 |
    /// | :--- | ---: | :--- |
    /// | 无门限 | −22.967075 | 两段的能量直接平均 |
    /// | 只过绝对门限 | −22.967075 | 两段都在 −70 之上 ⇒ 绝对门限一个人都拦不住 |
    /// | **门限积分**（Γa + Γr） | **−20.064968** | Γr ≈ −32.97 ⇒ 安静段整体被排除 |
    ///
    /// 所以"−20.065"这个读数**只有相对门限存在时才可能出现**：少了 Γr 会读到 −22.97。
    ///
    /// 注入：把 [`RELATIVE_GATE_LU`] 改成 0（Γr 抬高到 −20，两段都可能被排除）
    /// 或把 [`ABSOLUTE_GATE_LUFS`] 改成 0 ⇒ 本判据变红。
    #[test]
    fn the_relative_gate_removes_the_quiet_section() {
        let loud = sine_at_rate(0.1, 997.0, 48_000.0, 480_000);
        let quiet = sine_at_rate(0.01, 997.0, 48_000.0, 480_000);
        let mut both = loud.clone();
        both.extend_from_slice(&quiet);

        let ungated = LoudnessMeter::integrated_stereo(&both, &both);
        let gated = GatedLoudness::integrated_stereo(&both, &both);
        assert!(
            (ungated + 22.967).abs() < 0.05,
            "无门限应读到 −22.967, 实际 {ungated}"
        );
        assert!(
            (gated + 20.065).abs() < 0.1,
            "门限积分应读到 −20.065(安静段被 Γr 排除), 实际 {gated}"
        );
        assert!(
            gated - ungated > 2.5,
            "相对门限的效应必须显著: gated={gated} ungated={ungated}"
        );
        // 单独测安静段: 它自己过绝对门限 ⇒ 读数就是 −40 量级(而不是负无穷)。
        let quiet_only = GatedLoudness::integrated_stereo(&quiet, &quiet);
        assert!(
            (quiet_only + 40.0).abs() < 0.1,
            "安静段单独应读到 −40 量级, 实际 {quiet_only}"
        );
    }

    /// 判据：**瞬时窗口 = 400 ms、75% 重叠**（4 个 100 ms 跳的滑动窗口）。
    ///
    /// 夹具：4 个跳的 −20 LUFS 正弦 + 4 个跳的静音。窗口是"最近 4 个跳"，因此：
    ///
    /// | 跳 | 窗口内容 | 期望 | 实测 |
    /// | :-: | :--- | ---: | ---: |
    /// | 1–3 | 未满 | −∞ | −∞ |
    /// | 4 | 响×4 | −20.00 | −19.997 |
    /// | 5 | 响×3 + 静 | −21.25 | −21.245 |
    /// | 6 | 响×2 + 静×2 | −23.01 | −23.006 |
    /// | 7 | 响 + 静×3 | −26.02 | −26.016 |
    ///
    /// 若窗口长度不是 4（例如 3 跳 ⇒ −21.76、5 跳 ⇒ −20.97、1 跳 ⇒ −40），
    /// 第 5 个跳的读数会落在别处 ⇒ 本判据变红。
    ///
    /// 注入：把 [`GATING_HOPS_PER_BLOCK`] 改成 3（窗口 300 ms）⇒ 变红。
    #[test]
    fn momentary_window_covers_four_hundred_milliseconds_with_seventy_five_percent_overlap() {
        let loud = sine_at_rate(0.1, 997.0, 48_000.0, 4_800);
        let silent = vec![0.0f32; 4_800];
        let mut meter = GatedLoudness::new_48k();
        assert_eq!(meter.hop_samples(), 4_800);
        assert_eq!(meter.sample_rate(), 48_000.0);

        for hop in 1..=3 {
            meter.add_stereo(&loud, &loud);
            assert_eq!(
                meter.momentary_lufs(),
                f32::NEG_INFINITY,
                "第 {hop} 个跳时 400 ms 窗口还没满"
            );
        }
        meter.add_stereo(&loud, &loud);
        assert!(
            (meter.momentary_lufs() + 20.0).abs() < 0.05,
            "第 4 个跳应读到 −20.0, 实际 {}",
            meter.momentary_lufs()
        );

        // 第 5 个跳: 窗口 = [响, 响, 响, 静] ⇒ 能量剩 3/4。
        meter.add_stereo(&silent, &silent);
        let after_one_silent_hop = meter.momentary_lufs();
        assert!(
            (after_one_silent_hop + 21.245).abs() < 0.06,
            "第 5 个跳应读到 −21.245(3/4 能量), 实际 {after_one_silent_hop}"
        );
        // 排除"窗口是 3 跳或 5 跳"的读法。
        assert!(
            after_one_silent_hop < -21.15 && after_one_silent_hop > -21.35,
            "读数 {after_one_silent_hop} 不对应 4 个跳的窗口"
        );

        meter.add_stereo(&silent, &silent);
        assert!(
            (meter.momentary_lufs() + 23.006).abs() < 0.06,
            "第 6 个跳应读到 −23.006, 实际 {}",
            meter.momentary_lufs()
        );
        meter.add_stereo(&silent, &silent);
        assert!(
            (meter.momentary_lufs() + 26.016).abs() < 0.06,
            "第 7 个跳应读到 −26.016, 实际 {}",
            meter.momentary_lufs()
        );
        assert!(
            (meter.max_momentary_lufs() + 19.997).abs() < 0.05,
            "最大瞬时读数应保持第 4 个跳的值, 实际 {}",
            meter.max_momentary_lufs()
        );
    }

    /// 判据：**短时窗口 = 3 s（30 个 100 ms 跳）**，并且不会提前给出读数。
    #[test]
    fn short_term_window_is_three_seconds_wide() {
        let loud = sine_at_rate(0.1, 997.0, 48_000.0, 4_800);
        let mut meter = GatedLoudness::new_48k();
        for _ in 1..30 {
            meter.add_stereo(&loud, &loud);
            assert_eq!(
                meter.short_term_lufs(),
                f32::NEG_INFINITY,
                "3 s 窗口没满之前不得给短时读数"
            );
        }
        meter.add_stereo(&loud, &loud);
        assert!(
            (meter.short_term_lufs() + 20.0).abs() < 0.05,
            "第 30 个跳应给出 ≈−20.0 的短时读数, 实际 {}",
            meter.short_term_lufs()
        );
        assert!(
            (meter.max_short_term_lufs() + 20.0).abs() < 0.05,
            "最大短时读数应被记录, 实际 {}",
            meter.max_short_term_lufs()
        );
        // 空输入 / 静音: 读数不得是 NaN。
        let silence = vec![0.0f32; 48_000];
        let mut quiet = GatedLoudness::new_48k();
        quiet.add_stereo(&silence, &silence);
        assert!(!quiet.momentary_lufs().is_nan());
        assert!(!quiet.short_term_lufs().is_nan());
    }
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

    /// 判据：**单声道口径 = 单通道能量**（**不是**"同一个信号喂两个通道"）。
    ///
    /// 单声道信号喂两遍会平白多 `10·log10(2) = 3.01 dB`。这条判据把**流式**与
    /// **离线**两条路径都钉住：`add_mono(x)` 的读数必须与 `add_stereo(x, 0)` 逐位相同，
    /// 且要与上一线的无门限计量器 [`LoudnessMeter::integrated_mono`] 同一约定
    /// （单声道 −20 dBFS 的 997 Hz 正弦 ⇒ ≈ −23.01 LUFS）。
    ///
    /// 注入：把 `push_frame(y, None)` 改回 `push_frame(y, Some(y))`（或把
    /// `integrated_mono_at` 改成 `integrated_stereo_at(rate, x, x)`）⇒ 读数 +3.01 dB ⇒ 变红。
    #[test]
    fn gated_loudness_mono_matches_one_silent_channel() {
        let tone = sine_997(0.1, 96_000);
        let silent = vec![0.0f32; tone.len()];

        let mut mono = GatedLoudness::new_48k();
        mono.add_mono(&tone);
        let mut one_channel = GatedLoudness::new_48k();
        one_channel.add_stereo(&tone, &silent);
        assert_eq!(
            mono.momentary_lufs().to_bits(),
            one_channel.momentary_lufs().to_bits(),
            "单声道流式读数必须与'另一路静音'的立体声读数逐位相同"
        );
        assert_eq!(
            mono.max_short_term_lufs().to_bits(),
            one_channel.max_short_term_lufs().to_bits()
        );
        assert!(
            (mono.momentary_lufs() + 23.010_3).abs() < 0.05,
            "单声道 −20 dBFS 正弦的瞬时读数应约 −23.01, 实际 {}",
            mono.momentary_lufs()
        );

        let mono_integrated = GatedLoudness::integrated_mono(&tone);
        let stereo_integrated = GatedLoudness::integrated_stereo(&tone, &silent);
        assert_eq!(
            mono_integrated.to_bits(),
            stereo_integrated.to_bits(),
            "单声道离线读数必须与'另一路静音'的立体声读数逐位相同"
        );
        assert!(
            (mono_integrated + 23.010_3).abs() < 0.05,
            "单声道 −20 dBFS 正弦的门限积分应约 −23.01, 实际 {mono_integrated}"
        );
        // 与上一线的无门限计量器**同一约定**（它在 one_channel_only_is_three_db_quieter 里钉的也是这个）。
        let ungated_mono = LoudnessMeter::integrated_mono(&tone);
        assert!(
            (mono_integrated - ungated_mono).abs() < 0.05,
            "单声道口径必须与 LoudnessMeter::integrated_mono 一致: {mono_integrated} vs {ungated_mono}"
        );
        // 96 kHz 也要走同一条路（`integrated_mono_at` 不能退化成"喂两遍"）。
        let tone_96k = sine_at_rate(0.1, 997.0, 96_000.0, 96_000);
        let silent_96k = vec![0.0f32; tone_96k.len()];
        let mono_96k = GatedLoudness::integrated_mono_at(96_000.0, &tone_96k).expect("96 kHz");
        let stereo_96k =
            GatedLoudness::integrated_stereo_at(96_000.0, &tone_96k, &silent_96k).expect("96 kHz");
        assert_eq!(mono_96k.to_bits(), stereo_96k.to_bits());
    }
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

    /// 判据：门限计量的**确定性 + 分块不变性**（实时路径会按任意块长喂入）。
    #[test]
    fn gated_loudness_is_bit_deterministic_and_blocking_invariant() {
        let block = sine_at_rate(0.13, 997.0, 48_000.0, 200_000);
        let mut whole = GatedLoudness::new_48k();
        whole.add_stereo(&block, &block);
        let mut chunked = GatedLoudness::new_48k();
        for chunk in block.chunks(97) {
            chunked.add_stereo(chunk, chunk);
        }
        assert_eq!(
            whole.momentary_lufs().to_bits(),
            chunked.momentary_lufs().to_bits()
        );
        assert_eq!(
            whole.max_short_term_lufs().to_bits(),
            chunked.max_short_term_lufs().to_bits()
        );
        assert_eq!(whole, chunked);

        let a = GatedLoudness::integrated_stereo(&block, &block);
        let b = GatedLoudness::integrated_stereo(&block, &block);
        assert_eq!(a.to_bits(), b.to_bits());
        assert!(a.is_finite());
        // 长度不等时按较短者工作, 不 panic（24000 帧 = 1.25 个门限块 ⇒ 有完整块）。
        let left = vec![0.5f32; 48_000];
        let right = vec![0.5f32; 24_000];
        assert!(GatedLoudness::integrated_stereo(&left, &right).is_finite());
        // 空输入 / 太短: 一个完整块都没有 ⇒ 负无穷, 绝不是 NaN。
        for short in [0usize, 1, 4_799, 19_199] {
            let samples = vec![1.0f32; short];
            let reading = GatedLoudness::integrated_stereo(&samples, &samples);
            assert_eq!(reading, f32::NEG_INFINITY, "{short} 个样本应没有完整块");
            assert!(!reading.is_nan());
        }
    }

    /// 判据：**"无门限"与"门限积分"的语义差异被显式钉住**。
    ///
    /// ⚠ 这条判据是 HD-27 落地时对上一线
    /// `silence_padding_pulls_ungated_loudness_down` 的**显式改写**
    /// （`dsp-level-notes.md` §8 的 N5 要求"落地时必须显式改写"而不是让它悄悄变红）。
    ///
    /// 夹具：1 秒 −20 LUFS 的 997 Hz 立体声正弦 + 1 秒静音。
    ///
    /// | 计量器 | 读数 | 为什么 |
    /// | :--- | ---: | :--- |
    /// | [`LoudnessMeter`]（无门限） | **−23.010311** | 静音段的零能量实打实进了分母（能量减半） |
    /// | [`GatedLoudness`]（门限积分） | **−20.705866** | 静音块被 Γa 排除；只剩三块"跨在切换点上"的 400 ms 块 |
    ///
    /// 残余的 −0.71 dB **全部来自块边界**（75% 重叠的块里有三块跨在静音上，
    /// 分别含 3/4、1/2、1/4 的信号能量）：
    /// `10·log10((7 + 0.75 + 0.5 + 0.25)/10) = −0.706 dB`。这不是门限没生效。
    ///
    /// 注入：把 [`ABSOLUTE_GATE_LUFS`] 改成 0（等于"什么都过不了"）⇒ 门限读数
    /// 变负无穷 ⇒ 变红。
    #[test]
    fn silence_padding_pulls_ungated_loudness_down_but_not_the_gated_one() {
        let mut samples = sine_997(0.1, 48_000);
        samples.extend(std::iter::repeat_n(0.0f32, 48_000));
        let ungated = LoudnessMeter::integrated_stereo(&samples, &samples);
        assert!(
            (ungated + 23.010_3).abs() < 0.05,
            "无门限时静音段应把读数拉到 −23.01 LUFS, 实际 {ungated}"
        );
        let gated = GatedLoudness::integrated_stereo(&samples, &samples);
        assert!(
            (gated + 20.706).abs() < 0.05,
            "门限积分应读到 −20.706(只有边界块损失能量), 实际 {gated}"
        );
        assert!(
            gated - ungated > 2.0,
            "门限必须把静音段排除掉: gated={gated} ungated={ungated}"
        );
    }

    fn magnitude_at(coeffs: [f64; 5], frequency: f64, sample_rate: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = coeffs;
        let omega = std::f64::consts::TAU * frequency / sample_rate;
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
