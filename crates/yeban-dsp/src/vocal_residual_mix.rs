//! 残差混合（P0-2B，改编应用：瞬态锚定高频增强）。
//!
//! # 背景
//!
//! 跟进清单 §P0-2（OpenTune 只读吸收）描述了两档应用：应用 A（神经声＋
//! 原声高频非周期残差）随 P0-1 神经腿走；应用 B 是改编假设——Phase 1
//! 现有输出是拉伸后的原声（不是神经合成声），其高频已存在、只是被展宽／
//! 延迟，此时“补残差”不是补缺失成分，而是**瞬态锚定的高频增强**：在
//! VOP 锁窗处用原声瞬态强化拉伸输出的起音沿。本模块只实现应用 B。
//!
//! # 做法（clean-room，只按清单算法描述重写）
//!
//! 1. 从原声提非周期高频残差：STFT（Hann 窗）后，沿频率做中值滤波得到
//!    谐波本底；高于本底 `harmonic_peak_ratio` 倍的谱峰判定为谐波成分
//!    （置零到本底水平），其余保留为非周期成分；再按 `hf_cutoff_hz`
//!    高通；逆变换重叠相加还原。瞬态是宽带能量，在各频带上与本底相当，
//!    故保留；元音稳态的谐波峰被去除。
//! 2. 把残差按同一 [`PiecewiseTimeMap`] 弯曲到输出时间（输出采样 → 输入
//!    采样，线性插值），与拉伸输出对齐。
//! 3. 只在锚定 VOP 目标附近的窗口内混合（`anchor_pre_s`／
//!    `anchor_post_s`），窗口边缘用 `crossfade_samples` 升余弦淡入淡出；
//!    窗口重叠时取包络最大值，不重复叠加。
//! 4. 瞬态检测（谱 flux，`flux_threshold`）只用于收据：核对弯曲后残差的
//!    瞬态峰是否落在每个 VOP 目标附近（`boundary_radius_samples` 内），
//!    偏离记入收据，不据此移动窗口（映射本身精确，见 ADR-0067）。
//!
//! # 证伪口径
//!
//! 本模块的验收是证伪实验：同一批 pilot、同一行同一配置，只开关残差
//! 混合，对比 VOP 残差中位与 p90、行起点残差中位、行 0 光字锁窗波形
//! NCC 与无双峰、无 drone、门检结论。任一指标劣化或引入可闻伪影即
//! **不采用**，不强行保留。结论见 `~/workspace/vocaledit-phase2/
//! REPORT-p0-2b.md`。
//!
//! # 红线
//!
//! * **离线限定**：只用于离线渲染，绝不进入实时回调（与
//!   [`vocal_timemap`]、[`vocal_pitch_shift`] 同款限定）。
//! * **默认关闭**：`ResidualMixConfig::enabled` 默认 `false`；关闭时输出
//!   与输入逐位相同。
//! * **不改 Phase 1**：`Custom2400_480` 与验收数冻结，门检语义不动；
//!   本模块不调用、不改动 `render_continuous_timemap`，只做其输出的
//!   后处理；不调用新入口的工程渲染路径与旧版逐位相同。
//! * **无新依赖**：STFT 复用 [`crate::math::fft`]，不引入 FFT crate；
//!   不粘贴、不对照任何 OpenTune 代码（AGPL-3.0 禁入）。
//! * **高频截止的取舍**：`hf_cutoff_hz` 默认 1000 Hz——高于人声基频与
//!   低次谐波，低于精修器带顶（3000 Hz），使起音瞬态在 1–3 kHz 的能量
//!   能到达起音检测器；截止取 3000 Hz 会使检测带内完全看不到残差，
//!   实验对 VOP 指标失去意义，故不取该值（见报告）。

use crate::math::fft;
use crate::vocal_timemap::PiecewiseTimeMap;

/// 默认 STFT 帧长（采样，清单常量）。
pub const DEFAULT_FFT_SIZE: usize = 2048;
/// 默认 STFT 跳长（采样，清单常量）。
pub const DEFAULT_HOP_SIZE: usize = 512;
/// 默认窗口边缘交叉淡化长度（采样，清单常量）。
pub const DEFAULT_CROSSFADE_SAMPLES: usize = 220;
/// 默认瞬态检测 flux 阈值（归一化 flux，清单常量）。
pub const DEFAULT_FLUX_THRESHOLD: f32 = 0.30;
/// 默认瞬态检测 FFT 长度（采样，清单常量）。
pub const DEFAULT_TRANSIENT_FFT_SIZE: usize = 512;
/// 默认瞬态检测跳长（采样，清单常量）。
pub const DEFAULT_TRANSIENT_HOP: usize = 128;
/// 默认边界搜索半径（采样，清单常量）。
pub const DEFAULT_BOUNDARY_RADIUS_SAMPLES: usize = 512;
/// 默认边界搜索步进（采样，清单常量）。
pub const DEFAULT_BOUNDARY_STEP_SAMPLES: usize = 128;
/// 默认高频截止（Hz）：高于人声基频与低次谐波，低于精修器带顶 3000 Hz。
pub const DEFAULT_HF_CUTOFF_HZ: f32 = 1000.0;
/// 默认混合增益（1.0＝全额补回提取到的残差）。
pub const DEFAULT_MIX_GAIN: f32 = 1.0;
/// 默认锚定窗口：VOP 目标之前（秒），与光字锁窗 NCC 窗口的起点一致。
pub const DEFAULT_ANCHOR_PRE_S: f64 = 0.050;
/// 默认锚定窗口：VOP 目标之后（秒），与光字锁窗 NCC 窗口的终点一致。
pub const DEFAULT_ANCHOR_POST_S: f64 = 0.200;
/// 默认静音门限：帧 RMS 低于此值时该帧残差置零（不混入静音区噪声）。
pub const DEFAULT_SILENCE_FLOOR: f32 = 1e-4;
/// 默认谐波峰判定倍数：谱幅度高于局部中值本底此倍即判为谐波成分。
pub const DEFAULT_HARMONIC_PEAK_RATIO: f32 = 2.0;
/// 中值滤波半宽（频带数）：局部本底取 ±7 个频带的中值。
pub const MEDIAN_HALF_WIDTH: usize = 7;

/// 残差混合配置。全部阈值与开关均为可配置参数并钉死默认值，不得写死
/// 后无法复核（与 P0-1 路由阈值同款要求）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResidualMixConfig {
    /// 采样率（Hz），须与输入音频一致。
    pub sample_rate: u32,
    /// STFT 帧长（采样，须为 2 的幂）。
    pub fft_size: usize,
    /// STFT 跳长（采样，须小于帧长）。
    pub hop_size: usize,
    /// 窗口边缘交叉淡化长度（采样，须大于 0）。
    pub crossfade_samples: usize,
    /// 瞬态检测 flux 阈值（归一化 flux，须在 (0, 1] 内）。
    pub flux_threshold: f32,
    /// 瞬态检测 FFT 长度（采样，须为 2 的幂）。
    pub transient_fft_size: usize,
    /// 瞬态检测跳长（采样，须小于瞬态 FFT 长度）。
    pub transient_hop: usize,
    /// 边界搜索半径（采样）。
    pub boundary_radius_samples: usize,
    /// 边界搜索步进（采样，须大于 0）。
    pub boundary_step_samples: usize,
    /// 高频截止（Hz，须在 (0, 采样率/2) 内）。
    pub hf_cutoff_hz: f32,
    /// 混合增益（须在 [0, 4] 内）。
    pub mix_gain: f32,
    /// 锚定窗口：VOP 目标之前（秒，须 ≥ 0）。
    pub anchor_pre_s: f64,
    /// 锚定窗口：VOP 目标之后（秒，须 > 0）。
    pub anchor_post_s: f64,
    /// 静音门限（帧 RMS，须 ≥ 0）。
    pub silence_floor: f32,
    /// 谐波峰判定倍数（须 > 1）。
    pub harmonic_peak_ratio: f32,
    /// 总开关：关闭时混合函数原样返回输入（逐位相同）。
    pub enabled: bool,
}

impl Default for ResidualMixConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            fft_size: DEFAULT_FFT_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            crossfade_samples: DEFAULT_CROSSFADE_SAMPLES,
            flux_threshold: DEFAULT_FLUX_THRESHOLD,
            transient_fft_size: DEFAULT_TRANSIENT_FFT_SIZE,
            transient_hop: DEFAULT_TRANSIENT_HOP,
            boundary_radius_samples: DEFAULT_BOUNDARY_RADIUS_SAMPLES,
            boundary_step_samples: DEFAULT_BOUNDARY_STEP_SAMPLES,
            hf_cutoff_hz: DEFAULT_HF_CUTOFF_HZ,
            mix_gain: DEFAULT_MIX_GAIN,
            anchor_pre_s: DEFAULT_ANCHOR_PRE_S,
            anchor_post_s: DEFAULT_ANCHOR_POST_S,
            silence_floor: DEFAULT_SILENCE_FLOOR,
            harmonic_peak_ratio: DEFAULT_HARMONIC_PEAK_RATIO,
            enabled: false,
        }
    }
}

/// 残差混合错误：全部为调用方输入问题，大声失败，不静默回落。
#[derive(Debug, Clone, PartialEq)]
pub enum ResidualMixError {
    /// 输入为空。
    EmptyInput,
    /// 长度不一致（拉伸输出与弯曲后残差须等长）。
    LengthMismatch,
    /// 配置非法（附原因）。
    InvalidConfig(&'static str),
}

impl core::fmt::Display for ResidualMixError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ResidualMixError::EmptyInput => write!(f, "empty input"),
            ResidualMixError::LengthMismatch => {
                write!(f, "stretched and residual lengths differ")
            }
            ResidualMixError::InvalidConfig(message) => {
                write!(f, "invalid config: {message}")
            }
        }
    }
}

impl std::error::Error for ResidualMixError {}

/// 单个锚定窗口的混合收据。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResidualWindowReceipt {
    /// VOP 目标（输出秒）。
    pub target_vop_s: f64,
    /// 窗口内弯曲后残差的峰值幅度。
    pub peak_residual: f32,
    /// 窗口内最近瞬态峰相对 VOP 目标的偏移（采样；无瞬态峰时为 None）。
    pub flux_peak_offset_samples: Option<i64>,
    /// 最近瞬态峰是否落在边界搜索半径外（映射与残差不一致的信号）。
    pub misaligned: bool,
}

/// 残差混合收据：每窗口一条，全部可观测（与夜半收据文化一致）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResidualMixReceipt {
    /// 是否实际执行了混合（关闭或增益为 0 时为 false，输出逐位相同）。
    pub applied: bool,
    /// 锚定窗口数。
    pub windows: usize,
    /// 各窗口收据（与输入 VOP 目标一一对应）。
    pub per_window: Vec<ResidualWindowReceipt>,
    /// 混合后输出峰值幅度（> 1.0 表示写出时会削波，属伪影信号）。
    pub peak_out: f32,
    /// 混合前拉伸输出峰值幅度。
    pub peak_in: f32,
}

impl ResidualMixConfig {
    /// 校验全部参数；任一非法即返回原因，不静默修正。
    pub fn validate(&self) -> Result<(), ResidualMixError> {
        if self.sample_rate == 0 {
            return Err(ResidualMixError::InvalidConfig("sample_rate is 0"));
        }
        if !self.fft_size.is_power_of_two() || self.fft_size < 2 {
            return Err(ResidualMixError::InvalidConfig(
                "fft_size must be a power of two >= 2",
            ));
        }
        if self.hop_size == 0 || self.hop_size >= self.fft_size {
            return Err(ResidualMixError::InvalidConfig(
                "hop_size must be in (0, fft_size)",
            ));
        }
        if self.crossfade_samples == 0 {
            return Err(ResidualMixError::InvalidConfig(
                "crossfade_samples must be > 0",
            ));
        }
        if !(0.0 < self.flux_threshold && self.flux_threshold <= 1.0) {
            return Err(ResidualMixError::InvalidConfig(
                "flux_threshold must be in (0, 1]",
            ));
        }
        if !self.transient_fft_size.is_power_of_two() || self.transient_fft_size < 2 {
            return Err(ResidualMixError::InvalidConfig(
                "transient_fft_size must be a power of two >= 2",
            ));
        }
        if self.transient_hop == 0 || self.transient_hop >= self.transient_fft_size {
            return Err(ResidualMixError::InvalidConfig(
                "transient_hop must be in (0, transient_fft_size)",
            ));
        }
        if self.boundary_step_samples == 0 {
            return Err(ResidualMixError::InvalidConfig(
                "boundary_step_samples must be > 0",
            ));
        }
        let nyquist = self.sample_rate as f32 / 2.0;
        if !(0.0 < self.hf_cutoff_hz && self.hf_cutoff_hz < nyquist) {
            return Err(ResidualMixError::InvalidConfig(
                "hf_cutoff_hz must be in (0, sample_rate/2)",
            ));
        }
        if !(0.0..=4.0).contains(&self.mix_gain) {
            return Err(ResidualMixError::InvalidConfig(
                "mix_gain must be in [0, 4]",
            ));
        }
        if self.anchor_pre_s < 0.0 || self.anchor_post_s <= 0.0 {
            return Err(ResidualMixError::InvalidConfig(
                "anchor window must satisfy pre >= 0 and post > 0",
            ));
        }
        if self.silence_floor < 0.0 {
            return Err(ResidualMixError::InvalidConfig(
                "silence_floor must be >= 0",
            ));
        }
        if self.harmonic_peak_ratio <= 1.0 {
            return Err(ResidualMixError::InvalidConfig(
                "harmonic_peak_ratio must be > 1",
            ));
        }
        Ok(())
    }
}

/// 汉宁窗系数（`libm`，与 [`crate::math`] 的确定性口径一致）。
fn hann_window(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| 0.5 * (1.0 - libm::cos(2.0 * core::f64::consts::PI * i as f64 / (n - 1) as f64)))
        .collect()
}

/// 一维中值（复制后排序；空切片返回 0.0）。
fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

/// 从原声提非周期高频残差（STFT 谐波本底去除＋高通＋重叠相加还原）。
///
/// * `input`：单声道 PCM（f32，`config.sample_rate` 采样率）。
/// * 返回与输入等长的残差 PCM：谐波稳态成分已去除，保留宽带瞬态与
///   噪声成分在 `hf_cutoff_hz` 以上的部分；静音帧置零。
///
/// 空输入返回 [`ResidualMixError::EmptyInput`]；配置非法返回
/// [`ResidualMixError::InvalidConfig`]。
pub fn extract_aperiodic_high(
    input: &[f32],
    config: &ResidualMixConfig,
) -> Result<Vec<f32>, ResidualMixError> {
    config.validate()?;
    if input.is_empty() {
        return Err(ResidualMixError::EmptyInput);
    }
    let n_fft = config.fft_size;
    let hop = config.hop_size;
    let sr = config.sample_rate as f64;
    let half = n_fft / 2;
    let window = hann_window(n_fft);
    // 两端各补一帧零，避免首尾帧覆盖不足。
    let padded_len = input.len() + 2 * n_fft;
    let mut padded = vec![0.0_f32; padded_len];
    padded[n_fft..n_fft + input.len()].copy_from_slice(input);
    let frame_count = (padded_len - n_fft) / hop + 1;
    let mut out = vec![0.0_f64; padded_len];
    let mut envelope = vec![0.0_f64; padded_len];
    let cutoff_bin = ((config.hf_cutoff_hz as f64 * n_fft as f64 / sr).ceil() as usize).min(half);
    let mut re = vec![0.0_f64; n_fft];
    let mut im = vec![0.0_f64; n_fft];
    let mut mag = vec![0.0_f64; half + 1];
    let mut scratch = [0.0_f64; 2 * MEDIAN_HALF_WIDTH + 1];
    for frame in 0..frame_count {
        let start = frame * hop;
        // 帧 RMS 门限：静音帧不产残差。
        let mut sum_sq = 0.0_f64;
        for (i, slot) in re.iter_mut().enumerate() {
            let sample = f64::from(padded[start + i]) * window[i];
            *slot = sample;
            sum_sq += f64::from(padded[start + i]) * f64::from(padded[start + i]);
        }
        for slot in im.iter_mut() {
            *slot = 0.0;
        }
        let rms = (sum_sq / n_fft as f64).sqrt() as f32;
        if rms < config.silence_floor {
            continue;
        }
        fft(&mut re, &mut im, false);
        for k in 0..=half {
            mag[k] = (re[k] * re[k] + im[k] * im[k]).sqrt();
        }
        // 每频带局部中值本底；高于本底 harmonic_peak_ratio 倍的谱峰
        // 判为谐波，幅度压到本底水平；其余保留。
        for k in 0..=half {
            let lo = k.saturating_sub(MEDIAN_HALF_WIDTH);
            let hi = (k + MEDIAN_HALF_WIDTH).min(half);
            let count = hi - lo + 1;
            for (j, source) in (lo..=hi).enumerate() {
                scratch[j] = mag[source];
            }
            let floor = median(&mut scratch[..count]);
            let aperiodic = if mag[k] > config.harmonic_peak_ratio as f64 * floor {
                floor
            } else {
                mag[k]
            };
            let kept = if k < cutoff_bin { 0.0 } else { aperiodic };
            let scale = if mag[k] > 1e-12 { kept / mag[k] } else { 0.0 };
            re[k] *= scale;
            im[k] *= scale;
            if k > 0 && k < half {
                re[n_fft - k] *= scale;
                im[n_fft - k] *= scale;
            }
        }
        fft(&mut re, &mut im, true);
        for i in 0..n_fft {
            out[start + i] += re[i] * window[i];
            envelope[start + i] += window[i] * window[i];
        }
    }
    let mut residual = vec![0.0_f32; input.len()];
    for (i, slot) in residual.iter_mut().enumerate() {
        let idx = i + n_fft;
        let env = envelope[idx];
        let value = if env > 1e-9 { out[idx] / env } else { 0.0 };
        *slot = value as f32;
    }
    Ok(residual)
}

/// 瞬态检测：谱 flux（FFT `transient_fft_size`／hop `transient_hop`）。
///
/// 返回归一化 flux 超过 `flux_threshold` 的帧中心采样位置（升序）。
/// 空输入返回 [`ResidualMixError::EmptyInput`]。
pub fn detect_transients(
    input: &[f32],
    config: &ResidualMixConfig,
) -> Result<Vec<usize>, ResidualMixError> {
    config.validate()?;
    if input.is_empty() {
        return Err(ResidualMixError::EmptyInput);
    }
    let n_fft = config.transient_fft_size;
    let hop = config.transient_hop;
    let window = hann_window(n_fft);
    let frame_count = input.len().div_ceil(hop);
    let mut prev_mag: Vec<f64> = Vec::new();
    let mut re = vec![0.0_f64; n_fft];
    let mut im = vec![0.0_f64; n_fft];
    let mut hits = Vec::new();
    for frame in 0..frame_count {
        let start = frame * hop;
        for (i, slot) in re.iter_mut().enumerate() {
            let idx = start + i;
            let sample = if idx < input.len() {
                f64::from(input[idx])
            } else {
                0.0
            };
            *slot = sample * window[i];
        }
        for slot in im.iter_mut() {
            *slot = 0.0;
        }
        fft(&mut re, &mut im, false);
        let half = n_fft / 2;
        let mut mag = vec![0.0_f64; half + 1];
        let mut energy = 0.0_f64;
        for k in 0..=half {
            mag[k] = (re[k] * re[k] + im[k] * im[k]).sqrt();
            energy += mag[k];
        }
        if !prev_mag.is_empty() && energy > 1e-12 {
            let mut flux = 0.0_f64;
            for k in 0..=half {
                flux += (mag[k] - prev_mag[k]).max(0.0);
            }
            if flux / energy > config.flux_threshold as f64 {
                hits.push(start + n_fft / 2);
            }
        }
        prev_mag = mag;
    }
    Ok(hits)
}

/// 按 timemap 把输入时间残差弯曲到输出时间。
///
/// * `residual`：输入时间残差（与 timemap 的输入时间同基：take 内秒）。
/// * `map`：输出秒 → 输入秒映射（与 [`render_continuous_timemap`] 同款）。
/// * `target_len`：输出长度（采样）。
/// * 超出残差范围的输出采样填 0.0（线性插值，无外推）。
pub fn warp_residual(
    residual: &[f32],
    map: &PiecewiseTimeMap,
    target_len: usize,
    config: &ResidualMixConfig,
) -> Result<Vec<f32>, ResidualMixError> {
    config.validate()?;
    if residual.is_empty() {
        return Err(ResidualMixError::EmptyInput);
    }
    let sr = config.sample_rate as f64;
    let mut warped = vec![0.0_f32; target_len];
    for (n, slot) in warped.iter_mut().enumerate() {
        let in_s = map.map(n as f64 / sr);
        let pos = in_s * sr;
        if pos < 0.0 || pos > (residual.len() - 1) as f64 {
            *slot = 0.0;
            continue;
        }
        let lo = pos.floor() as usize;
        let frac = (pos - lo as f64) as f32;
        let hi = (lo + 1).min(residual.len() - 1);
        *slot = residual[lo] * (1.0 - frac) + residual[hi] * frac;
    }
    Ok(warped)
}

/// 升余弦淡入淡出系数（0→1 上升沿或 1→0 下降沿，`libm`）。
fn raised_cosine(t: f64) -> f64 {
    0.5 - 0.5 * libm::cos(core::f64::consts::PI * t.clamp(0.0, 1.0))
}

/// 把弯曲后残差混入拉伸输出，只在锚定 VOP 窗口内。
///
/// * `stretched`：Phase 1 拉伸输出（与 `warped_residual` 等长）。
/// * `warped_residual`：已按同一 timemap 弯曲到输出时间的残差。
/// * `vop_targets_s`：锚定 VOP 目标（输出秒）；窗口为
///   `[vop − anchor_pre_s, vop + anchor_post_s]`。
/// * 关闭或增益为 0 时原样返回输入（逐位相同），收据 `applied` 为 false。
/// * 窗口重叠时包络取最大值，不重复叠加。
/// * 返回混合后音频与收据；`peak_out > 1.0` 表示写出时会削波。
pub fn mix_residual(
    stretched: &[f32],
    warped_residual: &[f32],
    vop_targets_s: &[f64],
    config: &ResidualMixConfig,
) -> Result<(Vec<f32>, ResidualMixReceipt), ResidualMixError> {
    config.validate()?;
    if stretched.is_empty() || warped_residual.is_empty() {
        return Err(ResidualMixError::EmptyInput);
    }
    if stretched.len() != warped_residual.len() {
        return Err(ResidualMixError::LengthMismatch);
    }
    let peak_in = stretched
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    let mut receipt = ResidualMixReceipt {
        applied: false,
        windows: vop_targets_s.len(),
        per_window: Vec::with_capacity(vop_targets_s.len()),
        peak_out: peak_in,
        peak_in,
    };
    if !config.enabled || config.mix_gain == 0.0 {
        for target in vop_targets_s {
            receipt.per_window.push(ResidualWindowReceipt {
                target_vop_s: *target,
                peak_residual: 0.0,
                flux_peak_offset_samples: None,
                misaligned: false,
            });
        }
        return Ok((stretched.to_vec(), receipt));
    }
    let sr = config.sample_rate as f64;
    let crossfade = config.crossfade_samples;
    // 瞬态峰（弯曲后残差上）：只用于收据核对，不移动窗口。
    let transient_centers = detect_transients(warped_residual, config)?;
    let mut envelope = vec![0.0_f32; stretched.len()];
    for target in vop_targets_s {
        let center = *target * sr;
        let start_f = (center - config.anchor_pre_s * sr).max(0.0);
        let end_f = (center + config.anchor_post_s * sr).min(stretched.len() as f64);
        let start = start_f as usize;
        let end = end_f as usize;
        if end <= start {
            receipt.per_window.push(ResidualWindowReceipt {
                target_vop_s: *target,
                peak_residual: 0.0,
                flux_peak_offset_samples: None,
                misaligned: true,
            });
            continue;
        }
        let mut peak_residual = 0.0_f32;
        for n in start..end {
            peak_residual = peak_residual.max(warped_residual[n].abs());
            let ramp_up = if n - start < crossfade {
                raised_cosine((n - start) as f64 / crossfade as f64) as f32
            } else {
                1.0
            };
            let ramp_down = if end - n <= crossfade {
                raised_cosine((end - n) as f64 / crossfade as f64) as f32
            } else {
                1.0
            };
            let gain_here = ramp_up.min(ramp_down);
            if gain_here > envelope[n] {
                envelope[n] = gain_here;
            }
        }
        // 最近瞬态峰相对 VOP 目标的偏移：从中心向外步进搜索，取绝对值
        // 最小的候选偏移（半径内无峰即 None，记 misaligned）。
        let center_sample = center as i64;
        let mut best: Option<i64> = None;
        for offset in
            (0..=config.boundary_radius_samples as i64).step_by(config.boundary_step_samples.max(1))
        {
            for candidate in [offset, -offset] {
                let probe = center_sample + candidate;
                if transient_centers
                    .iter()
                    .any(|c| (*c as i64 - probe).abs() < config.boundary_step_samples as i64)
                {
                    let better = match best {
                        None => true,
                        Some(b) => candidate.abs() < b.abs(),
                    };
                    if better {
                        best = Some(candidate);
                    }
                }
            }
            if best.is_some() && offset > 0 {
                // 已在较小半径内命中，更大半径不可能更优（绝对值只增）。
                break;
            }
        }
        let misaligned = best.is_none();
        receipt.per_window.push(ResidualWindowReceipt {
            target_vop_s: *target,
            peak_residual,
            flux_peak_offset_samples: best,
            misaligned,
        });
    }
    let gain = config.mix_gain;
    let mut mixed = Vec::with_capacity(stretched.len());
    let mut peak_out = 0.0_f32;
    for n in 0..stretched.len() {
        let value = stretched[n] + gain * envelope[n] * warped_residual[n];
        peak_out = peak_out.max(value.abs());
        mixed.push(value);
    }
    receipt.applied = true;
    receipt.peak_out = peak_out;
    Ok((mixed, receipt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocal_timemap::PiecewiseTimeMap;

    const SR: u32 = 48000;

    fn enabled_config() -> ResidualMixConfig {
        ResidualMixConfig {
            enabled: true,
            ..ResidualMixConfig::default()
        }
    }

    /// 确定性伪随机数（LCG，无 rand 依赖；测试专用）。
    struct Lcg(u64);

    impl Lcg {
        fn next_f32(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn disabled_returns_input_bit_identical() {
        let config = ResidualMixConfig::default();
        assert!(!config.enabled);
        let mut rng = Lcg(1);
        let stretched: Vec<f32> = (0..SR as usize).map(|_| rng.next_f32() * 0.5).collect();
        let residual: Vec<f32> = (0..SR as usize).map(|_| rng.next_f32() * 0.5).collect();
        let (out, receipt) = mix_residual(&stretched, &residual, &[0.5], &config).expect("mix");
        assert_eq!(out, stretched);
        assert!(!receipt.applied);
        assert_eq!(receipt.peak_out, receipt.peak_in);
    }

    #[test]
    fn zero_gain_returns_input_bit_identical() {
        let config = ResidualMixConfig {
            enabled: true,
            mix_gain: 0.0,
            ..ResidualMixConfig::default()
        };
        let mut rng = Lcg(2);
        let stretched: Vec<f32> = (0..SR as usize).map(|_| rng.next_f32() * 0.5).collect();
        let residual: Vec<f32> = (0..SR as usize).map(|_| rng.next_f32() * 0.5).collect();
        let (out, receipt) = mix_residual(&stretched, &residual, &[0.5], &config).expect("mix");
        assert_eq!(out, stretched);
        assert!(!receipt.applied);
    }

    #[test]
    fn silence_in_silence_out() {
        let config = enabled_config();
        let silence = vec![0.0_f32; SR as usize];
        let residual = extract_aperiodic_high(&silence, &config).expect("extract");
        assert_eq!(residual.len(), SR as usize);
        assert!(residual.iter().all(|s| *s == 0.0));
        let (out, receipt) = mix_residual(&silence, &residual, &[0.5], &config).expect("mix");
        assert_eq!(out.len(), SR as usize);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(receipt.applied);
    }

    #[test]
    fn steady_harmonic_tone_residual_near_zero() {
        let config = enabled_config();
        // 稳态谐波音：220 Hz 基频＋10 次谐波（振幅 0.5/k），2 秒。
        let mut tone = vec![0.0_f32; 2 * SR as usize];
        for (n, slot) in tone.iter_mut().enumerate() {
            let t = n as f64 / SR as f64;
            let mut value = 0.0_f64;
            for k in 1..=10 {
                value +=
                    0.5 / k as f64 * libm::sin(2.0 * core::f64::consts::PI * 220.0 * k as f64 * t);
            }
            *slot = value as f32;
        }
        let residual = extract_aperiodic_high(&tone, &config).expect("extract");
        // 谐波峰被压到本底水平：残差 RMS 不到输入的 5%。
        assert!(
            rms(&residual) < 0.05 * rms(&tone),
            "residual rms {} vs tone rms {}",
            rms(&residual),
            rms(&tone)
        );
    }

    #[test]
    fn noise_burst_residual_retained() {
        let config = enabled_config();
        // 100 ms 白噪声突发（其余静音）：宽带能量无窄谱峰，应保留。
        let mut rng = Lcg(3);
        let mut input = vec![0.0_f32; SR as usize];
        for slot in input.iter_mut().take(SR as usize / 10) {
            *slot = rng.next_f32() * 0.5;
        }
        let residual = extract_aperiodic_high(&input, &config).expect("extract");
        assert_eq!(residual.len(), SR as usize);
        let burst = &residual[..SR as usize / 10];
        assert!(
            rms(burst) > 0.5 * rms(&input[..SR as usize / 10]),
            "burst residual rms {} vs input rms {}",
            rms(burst),
            rms(&input[..SR as usize / 10])
        );
        // 静音区不产残差。
        assert!(residual[SR as usize / 2..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn warp_identity_map_is_exact() {
        let config = enabled_config();
        let map = PiecewiseTimeMap::new(vec![(0.0, 0.0), (2.0, 2.0)]);
        let mut rng = Lcg(4);
        let residual: Vec<f32> = (0..2 * SR as usize).map(|_| rng.next_f32()).collect();
        let warped = warp_residual(&residual, &map, 2 * SR as usize, &config).expect("warp");
        let max_diff = warped
            .iter()
            .zip(residual.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_diff < 1e-6, "max diff {max_diff}");
    }

    #[test]
    fn mix_only_inside_windows() {
        let config = enabled_config();
        let mut rng = Lcg(5);
        let stretched: Vec<f32> = (0..2 * SR as usize).map(|_| rng.next_f32() * 0.3).collect();
        let residual: Vec<f32> = (0..2 * SR as usize).map(|_| rng.next_f32() * 0.3).collect();
        let (out, receipt) = mix_residual(&stretched, &residual, &[1.0], &config).expect("mix");
        assert!(receipt.applied);
        assert_eq!(receipt.windows, 1);
        // 窗口 [0.95, 1.20] 之外逐位相同。
        assert_eq!(
            out[..(0.95 * SR as f64) as usize],
            stretched[..(0.95 * SR as f64) as usize]
        );
        assert_eq!(
            out[(1.20 * SR as f64) as usize..],
            stretched[(1.20 * SR as f64) as usize..]
        );
        // 窗口内至少一处不同（残差非零）。
        let inside = &out[(SR as usize)..(SR as usize + SR as usize / 10)];
        let inside_in = &stretched[(SR as usize)..(SR as usize + SR as usize / 10)];
        assert!(inside.iter().zip(inside_in.iter()).any(|(a, b)| a != b));
    }

    #[test]
    fn mix_output_bounded_no_nan() {
        let config = enabled_config();
        let mut rng = Lcg(6);
        let stretched: Vec<f32> = (0..SR as usize).map(|_| rng.next_f32() * 0.5).collect();
        let residual: Vec<f32> = (0..SR as usize).map(|_| rng.next_f32() * 0.5).collect();
        let (out, receipt) = mix_residual(&stretched, &residual, &[0.5], &config).expect("mix");
        assert_eq!(out.len(), SR as usize);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(receipt.peak_out <= receipt.peak_in + config.mix_gain * 0.5 + 1e-6);
    }

    #[test]
    fn transient_detected_at_burst() {
        let config = enabled_config();
        // 背景噪声 0.001＋1.0 秒处 20 ms 噪声突发。
        let mut rng = Lcg(7);
        let mut input: Vec<f32> = (0..2 * SR as usize)
            .map(|_| rng.next_f32() * 0.001)
            .collect();
        for slot in input.iter_mut().skip(SR as usize).take(SR as usize / 50) {
            *slot = rng.next_f32() * 0.5;
        }
        let hits = detect_transients(&input, &config).expect("detect");
        assert!(
            hits.iter().any(|c| (*c as i64 - SR as i64).abs() <= 1024),
            "no hit near 1.0s, hits: {hits:?}"
        );
    }

    #[test]
    fn config_validation_rejects_bad_values() {
        let base = ResidualMixConfig::default();
        assert!(base.validate().is_ok());
        for bad in [
            ResidualMixConfig {
                sample_rate: 0,
                ..base.clone()
            },
            ResidualMixConfig {
                fft_size: 2047,
                ..base.clone()
            },
            ResidualMixConfig {
                hop_size: 2048,
                ..base.clone()
            },
            ResidualMixConfig {
                crossfade_samples: 0,
                ..base.clone()
            },
            ResidualMixConfig {
                flux_threshold: 0.0,
                ..base.clone()
            },
            ResidualMixConfig {
                flux_threshold: 1.5,
                ..base.clone()
            },
            ResidualMixConfig {
                transient_fft_size: 100,
                ..base.clone()
            },
            ResidualMixConfig {
                hf_cutoff_hz: 24_000.0,
                ..base.clone()
            },
            ResidualMixConfig {
                mix_gain: -1.0,
                ..base.clone()
            },
            ResidualMixConfig {
                mix_gain: 5.0,
                ..base.clone()
            },
            ResidualMixConfig {
                anchor_post_s: 0.0,
                ..base.clone()
            },
            ResidualMixConfig {
                harmonic_peak_ratio: 1.0,
                ..base.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "expected invalid config: {bad:?}");
        }
        assert_eq!(
            extract_aperiodic_high(&[], &base),
            Err(ResidualMixError::EmptyInput)
        );
        assert_eq!(
            mix_residual(&[0.0; 8], &[0.0; 4], &[], &base),
            Err(ResidualMixError::LengthMismatch)
        );
    }

    /// 精修器起音门的简化版：帧 2048/480、带 300–3000 Hz；返回首个带能量
    /// 超过起音前本底 4 倍的帧中心时间（无则为 None）。只测上升沿机制，
    /// 不复现完整精修器（浊音门、持续门见 vocal_refine）。
    fn first_rise_time(samples: &[f32], coarse_s: f64) -> Option<f64> {
        let win = 2048usize;
        let hop = 480usize;
        let lo_bin = (300.0 * win as f64 / SR as f64).floor() as usize;
        let hi_bin = (3000.0 * win as f64 / SR as f64).ceil() as usize;
        let window = hann_window(win);
        let mut bands: Vec<(f64, f64)> = Vec::new();
        let mut start = ((coarse_s - 0.300).max(0.0) * SR as f64) as usize;
        let end = ((coarse_s + 0.300) * SR as f64) as usize;
        let mut re = vec![0.0_f64; win];
        let mut im = vec![0.0_f64; win];
        while start < end && start < samples.len() {
            for (i, slot) in re.iter_mut().enumerate() {
                let idx = start + i;
                let sample = if idx < samples.len() {
                    f64::from(samples[idx])
                } else {
                    0.0
                };
                *slot = sample * window[i];
            }
            for slot in im.iter_mut() {
                *slot = 0.0;
            }
            fft(&mut re, &mut im, false);
            let mut band = 0.0_f64;
            for k in lo_bin..=hi_bin {
                band += re[k] * re[k] + im[k] * im[k];
            }
            bands.push(((start + win / 2) as f64 / SR as f64, band));
            start += hop;
        }
        let mut floor_values: Vec<f64> = bands
            .iter()
            .filter(|(t, _)| *t < coarse_s - 0.080)
            .map(|(_, b)| *b)
            .collect();
        let floor = median(&mut floor_values).max(1e-12);
        bands
            .iter()
            .find(|(t, b)| *t >= coarse_s - 0.080 && *b > 4.0 * floor)
            .map(|(t, _)| *t)
    }

    #[test]
    fn mechanism_mixed_onset_rises_earlier() {
        let config = enabled_config();
        // 原声：220 Hz 谐波音（10 次谐波），1.0 秒起音 10 ms 上升沿＋
        // 5 ms 噪声突发；背景噪声 0.001 定本底。
        let mut rng = Lcg(8);
        let mut original = vec![0.0_f32; 2 * SR as usize];
        for (n, slot) in original.iter_mut().enumerate() {
            let t = n as f64 / SR as f64;
            let mut value = rng.next_f32() * 0.001;
            if t >= 1.0 {
                let rise = ((t - 1.0) / 0.010).min(1.0);
                let mut tone = 0.0_f64;
                for k in 1..=10 {
                    tone += 0.5 / k as f64
                        * libm::sin(2.0 * core::f64::consts::PI * 220.0 * k as f64 * t);
                }
                value += (tone * rise) as f32;
                if t < 1.005 {
                    value += rng.next_f32() * 0.3;
                }
            }
            *slot = value;
        }
        // 拉伸版：同一谐波音，起音延迟 20 ms、上升沿展宽到 30 ms，无突发。
        let mut stretched = vec![0.0_f32; 2 * SR as usize];
        for (n, slot) in stretched.iter_mut().enumerate() {
            let t = n as f64 / SR as f64;
            let mut value = rng.next_f32() * 0.001;
            if t >= 1.02 {
                let rise = ((t - 1.02) / 0.030).min(1.0);
                let mut tone = 0.0_f64;
                for k in 1..=10 {
                    tone += 0.5 / k as f64
                        * libm::sin(2.0 * core::f64::consts::PI * 220.0 * k as f64 * t);
                }
                value += (tone * rise) as f32;
            }
            *slot = value;
        }
        let residual = extract_aperiodic_high(&original, &config).expect("extract");
        let map = PiecewiseTimeMap::new(vec![(0.0, 0.0), (2.0, 2.0)]);
        let warped = warp_residual(&residual, &map, 2 * SR as usize, &config).expect("warp");
        let (mixed, receipt) = mix_residual(&stretched, &warped, &[1.0], &config).expect("mix");
        assert!(receipt.applied);
        let rise_stretched = first_rise_time(&stretched, 1.0).expect("stretched rise");
        let rise_mixed = first_rise_time(&mixed, 1.0).expect("mixed rise");
        assert!(
            rise_mixed < rise_stretched,
            "mixed rise {rise_mixed:.4}s not earlier than stretched rise {rise_stretched:.4}s"
        );
        // 混合后起音须落在原声起音附近：残差的时间分辨率是一个 STFT
        // 帧（2048 采样 ≈ 42.7 ms），突发能量会提前一个帧窗出现，故
        // 允许比原声起音早至一帧；晚于原声起音超过一 hop（10 ms）则
        // 说明混合没有把起音锚回来。机制成立的判据仍是上一条断言
        // （比拉伸版早）；本条只防“早过头／没锚上”。
        let frame_s = 2048.0 / SR as f64;
        assert!(
            rise_mixed >= 1.0 - frame_s - 1e-9 && rise_mixed <= 1.0 + 0.010 + 1e-9,
            "mixed rise {rise_mixed:.4}s outside [1.0-frame, 1.0+hop]"
        );
    }
}
