//! 自动相位对齐：互相关测轨间延时（手动相位对齐第二层）。
//!
//! 手动层（[`crate::sample_delay::SampleDelay`] + `TrackV3::delay_samples`）
//! 只给“把一条轨推后 N 采样”的执行机构，不回答“该推多少”。本模块回答后者：
//! 给定两条单声道信号（基准轨与目标轨的 solo 渲染混音），用归一化互相关
//! 量出目标轨相对基准轨的到达时差，并判断极性是否反转。
//!
//! ## 设计来源
//!
//! Logic Pro **没有**自动互相关对齐功能（只有轨延时与 Sample Delay 插件，
//! 手动层已照抄），因此本模块的设计不抄 Logic，抄常见自动对齐插件的做法
//! （Sound Radix Auto-Align 式）：归一化互相关 + 峰值搜索 + 抛物线插值。
//! 补偿侧（只延时先到的轨、正延时、±4800 采样窗）抄 Logic 的轨延时语义。
//!
//! ## 符号约定
//!
//! `lag_samples > 0` = 目标轨比基准轨**晚到** `lag_samples` 采样
//! （`target[n] ≈ reference[n - lag]`）。补偿时延时**先到的那条轨**：
//! `lag > 0` 延基准轨 `+lag`，`lag < 0` 延目标轨 `|lag|` —— 只用正延时，
//! 不需要前瞻（lookahead）。
//!
//! ## 算法
//!
//! 1. 取两信号公共长度的前段；过短（< [`MIN_SIGNAL_FRAMES`]）拒绝。
//! 2. 在最响的 [`ANALYSIS_WINDOW_FRAMES`] 帧窗口里量（整曲太长时只量最响
//!    的 5 秒：成本有界，且静音段不贡献相关性）；两轨都静音（RMS <
//!    [`SILENCE_RMS`]）时拒绝，不返回“lag = 0”的假答案。
//! 3. 粗筛：8 倍抽取（块均值）后算原始互相关，峰值定粗 lag；精修：在粗 lag
//!    附近 ±[`REFINE_RADIUS`] 采样内按**逐 lag 归一化**相关系数重算，取
//!    绝对值最大的极值（正峰或负峰）；抛物线插值给亚采样 lag。
//! 4. 负峰胜出 ⇒ [`DelayMeasurement::polarity_inverted`] 为真（两轨极性相反）。
//!
//! ## 确定性 [ARCH-DET-001]
//!
//! 纯函数：同一对输入、同一 `max_lag_samples` ⇒ 逐位相同的输出。无随机源、
//! 无时间、无线程。并列峰（例如周期信号）按“`|lag|` 最小、其次取正”
//! 的固定顺序裁决，不依赖迭代器或哈希顺序。

/// 测量所需的最短公共长度（帧）。短于此的信号连一个相关峰都撑不起来。
pub const MIN_SIGNAL_FRAMES: usize = 1_024;

/// 分析窗口长度（帧）：5 秒 @ 48 kHz。整曲只量最响的 5 秒，成本有界。
pub const ANALYSIS_WINDOW_FRAMES: usize = 240_000;

/// 粗筛抽取倍数（块均值下采样）。
pub const DECIMATION: usize = 8;

/// 精修半径（采样）：粗 lag 附近 ±16 采样重算，覆盖抽取误差（±8）并留余量。
pub const REFINE_RADIUS: usize = 16;

/// 静音门限（RMS）：低于此视为无信号，拒绝测量（不返回假 lag）。
pub const SILENCE_RMS: f32 = 1e-6;

/// `High` 置信度的相关系数下界。
pub const CONFIDENCE_HIGH: f32 = 0.7;

/// `Medium` 置信度的相关系数下界（低于此为 `Low`）。
pub const CONFIDENCE_MEDIUM: f32 = 0.4;

/// 施加补偿的相关系数下界（调用方口径：低于此不值得动轨延时）。
pub const APPLY_MIN_CORRELATION: f32 = 0.3;

/// 互相关测量的失败原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureDelayError {
    /// 公共长度短于 [`MIN_SIGNAL_FRAMES`]。
    TooShort {
        /// 实际公共长度（帧）。
        frames: usize,
    },
    /// 基准轨在分析窗内静音（RMS < [`SILENCE_RMS`]）。
    SilentReference,
    /// 目标轨在分析窗内静音（RMS < [`SILENCE_RMS`]）。
    SilentTarget,
    /// `max_lag_samples` 为 0（搜索窗不存在）。
    InvalidMaxLag,
}

impl core::fmt::Display for MeasureDelayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { frames } => {
                write!(f, "信号太短，无法测量延时（公共长度 {frames} 帧）")
            }
            Self::SilentReference => write!(f, "基准轨在分析窗内静音，无法测量延时"),
            Self::SilentTarget => write!(f, "目标轨在分析窗内静音，无法测量延时"),
            Self::InvalidMaxLag => write!(f, "搜索窗 max_lag_samples 不能为 0"),
        }
    }
}

impl std::error::Error for MeasureDelayError {}

/// 测量结果的置信度分档（按 |ρ| 划档，见模块文档）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// |ρ| ≥ 0.7：峰值清晰，可放心施加。
    High,
    /// 0.4 ≤ |ρ| < 0.7：峰值可辨，施加前宜人耳复核。
    Medium,
    /// |ρ| < 0.4：峰值不可信，调用方不应施加。
    Low,
}

impl Confidence {
    /// 契约字符串（进 MCP 响应 `confidence`）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    /// 按 |ρ| 划档（阈值见 [`CONFIDENCE_HIGH`] / [`CONFIDENCE_MEDIUM`]）。
    #[must_use]
    pub fn from_correlation(correlation: f32) -> Self {
        let magnitude = correlation.abs();
        if magnitude >= CONFIDENCE_HIGH {
            Self::High
        } else if magnitude >= CONFIDENCE_MEDIUM {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

/// 一次互相关测量的结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelayMeasurement {
    /// 目标轨相对基准轨的到达时差（采样，亚采样插值；> 0 = 目标轨晚到）。
    pub lag_samples: f64,
    /// [`Self::lag_samples`] 四舍五入到整数采样（补偿真正写入的值）。
    pub lag_samples_rounded: i32,
    /// 峰值归一化相关系数（有符号：负值 = 极性反转）。
    pub correlation: f32,
    /// 两轨极性是否相反（负峰胜出）。
    pub polarity_inverted: bool,
    /// 置信度分档（按 |ρ|）。
    pub confidence: Confidence,
    /// 分析窗起点（帧，相对输入起点）。
    pub window_start_frame: usize,
    /// 分析窗长度（帧）。
    pub window_frames: usize,
}

/// 找最响的窗口：按 4096 帧一块扫能量，窗口以最响块为中心。
fn loudest_window(reference: &[f32], target: &[f32]) -> (usize, usize) {
    const BLOCK: usize = 4_096;
    let len = reference.len().min(target.len());
    if len <= ANALYSIS_WINDOW_FRAMES {
        return (0, len);
    }
    let mut best_block = 0;
    let mut best_energy = f64::NEG_INFINITY;
    let mut start = 0;
    while start < len {
        let end = (start + BLOCK).min(len);
        let mut energy = 0.0_f64;
        for index in start..end {
            energy += f64::from(reference[index]) * f64::from(reference[index])
                + f64::from(target[index]) * f64::from(target[index]);
        }
        if energy > best_energy {
            best_energy = energy;
            best_block = start;
        }
        start = end;
    }
    let half = (ANALYSIS_WINDOW_FRAMES - BLOCK) / 2;
    let window_start = best_block
        .saturating_sub(half)
        .min(len - ANALYSIS_WINDOW_FRAMES);
    (window_start, ANALYSIS_WINDOW_FRAMES)
}

/// 均方根（f64 累加）。
fn rms(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0_f64;
    for &sample in signal {
        sum += f64::from(sample) * f64::from(sample);
    }
    (sum / signal.len() as f64).sqrt() as f32
}

/// 去均值（f64 均值；返回 f32 信号，逐位确定）。
fn remove_mean(signal: &[f32]) -> Vec<f32> {
    if signal.is_empty() {
        return Vec::new();
    }
    let mut sum = 0.0_f64;
    for &sample in signal {
        sum += f64::from(sample);
    }
    let mean = (sum / signal.len() as f64) as f32;
    signal.iter().map(|&sample| sample - mean).collect()
}

/// 块均值抽取（`DECIMATION` 倍下采样；尾部不足一块的丢弃）。
fn decimate(signal: &[f32]) -> Vec<f32> {
    let blocks = signal.len() / DECIMATION;
    let mut out = Vec::with_capacity(blocks);
    for block in 0..blocks {
        let mut sum = 0.0_f64;
        for index in 0..DECIMATION {
            sum += f64::from(signal[block * DECIMATION + index]);
        }
        out.push((sum / DECIMATION as f64) as f32);
    }
    out
}

/// 按“|lag| 最小、其次取正”的顺序枚举 lag（并列峰的裁决顺序，全函数共用）。
fn lags_by_abs_asc(max_lag: i32) -> Vec<i32> {
    let mut lags = Vec::with_capacity((2 * max_lag + 1).max(0) as usize);
    lags.push(0);
    for magnitude in 1..=max_lag {
        lags.push(magnitude);
        lags.push(-magnitude);
    }
    lags
}

/// 原始互相关（f32 累加，粗筛用）：`Σ x[i]·y[i+lag]`，`lag` 可正可负。
fn raw_correlation(reference: &[f32], target: &[f32], lag: i32) -> f32 {
    let mut acc = 0.0_f32;
    if lag >= 0 {
        let lag = lag as usize;
        let overlap = reference.len().saturating_sub(lag).min(target.len());
        for index in 0..overlap {
            acc += reference[index] * target[index + lag];
        }
    } else {
        let lag = (-lag) as usize;
        let overlap = target.len().saturating_sub(lag).min(reference.len());
        for index in 0..overlap {
            acc += reference[index + lag] * target[index];
        }
    }
    acc
}

/// 逐 lag 归一化相关系数（f64 累加，精修用）：分子是重叠区的互相关，
/// 分母是重叠区各自能量的几何均值（重叠区随 lag 变化，分母必须逐 lag 算）。
fn normalized_correlation(reference: &[f32], target: &[f32], lag: i32) -> f32 {
    let (ref_offset, target_offset, overlap) = if lag >= 0 {
        let lag = lag as usize;
        (
            0_usize,
            lag,
            reference.len().saturating_sub(lag).min(target.len()),
        )
    } else {
        let lag = (-lag) as usize;
        (
            lag,
            0_usize,
            target.len().saturating_sub(lag).min(reference.len()),
        )
    };
    if overlap == 0 {
        return 0.0;
    }
    let mut numerator = 0.0_f64;
    let mut ref_energy = 0.0_f64;
    let mut target_energy = 0.0_f64;
    for index in 0..overlap {
        let x = f64::from(reference[ref_offset + index]);
        let y = f64::from(target[target_offset + index]);
        numerator += x * y;
        ref_energy += x * x;
        target_energy += y * y;
    }
    if ref_energy == 0.0 || target_energy == 0.0 {
        return 0.0;
    }
    (numerator / (ref_energy * target_energy).sqrt()) as f32
}

/// 按“|lag| 最小、其次取正”的顺序枚举闭区间 `[low, high]` 内的 lag。
///
/// 精修步必须用它：`lags_by_abs_asc(high)` 再过滤在 `high < 0` 时是空的
/// （`1..=负数` 为空区间），负 lag 区的精修会直接丢峰（实测：-120 测成 0）。
fn lags_in_range_asc(low: i32, high: i32) -> Vec<i32> {
    let mut lags = Vec::with_capacity((high - low + 1).max(0) as usize);
    if low <= 0 && 0 <= high {
        lags.push(0);
    }
    let max_magnitude = low.unsigned_abs().max(high.unsigned_abs());
    for magnitude in 1..=max_magnitude {
        let magnitude = magnitude as i32;
        if low <= magnitude && magnitude <= high {
            lags.push(magnitude);
        }
        if low <= -magnitude && -magnitude <= high {
            lags.push(-magnitude);
        }
    }
    lags
}

/// 抛物线插值：三点相关系数（`center` 处取极值）顶点相对 `center` 的偏移，
/// 钳到 [-1, 1]（边界上无三点时调用方不插值）。
fn parabolic_offset(before: f32, center: f32, after: f32) -> f64 {
    let denominator = f64::from(before) - 2.0 * f64::from(center) + f64::from(after);
    if denominator == 0.0 {
        return 0.0;
    }
    let offset = 0.5 * (f64::from(before) - f64::from(after)) / denominator;
    offset.clamp(-1.0, 1.0)
}

/// 测两条单声道信号的到达时差（归一化互相关，见模块文档）。
///
/// `reference` / `target` 是同一采样率的单声道信号（调用方负责混音与对齐
/// 长度：只用公共长度）；`max_lag_samples` 是搜索窗（±，采样）。
///
/// # Errors
///
/// - 信号太短 / 任一轨静音 / `max_lag_samples == 0` → [`MeasureDelayError`]。
#[must_use = "测量失败必须由调用方处理，静默丢弃会产出假 lag"]
pub fn measure_delay(
    reference: &[f32],
    target: &[f32],
    max_lag_samples: usize,
) -> Result<DelayMeasurement, MeasureDelayError> {
    if max_lag_samples == 0 {
        return Err(MeasureDelayError::InvalidMaxLag);
    }
    let len = reference.len().min(target.len());
    if len < MIN_SIGNAL_FRAMES {
        return Err(MeasureDelayError::TooShort { frames: len });
    }
    let (window_start, window_frames) = loudest_window(reference, target);
    let ref_window = &reference[window_start..window_start + window_frames];
    let target_window = &target[window_start..window_start + window_frames];
    if rms(ref_window) < SILENCE_RMS {
        return Err(MeasureDelayError::SilentReference);
    }
    if rms(target_window) < SILENCE_RMS {
        return Err(MeasureDelayError::SilentTarget);
    }
    let reference = remove_mean(ref_window);
    let target = remove_mean(target_window);

    // 粗筛：8 倍抽取后算原始互相关，峰值（绝对值最大）定粗 lag。
    let max_lag = max_lag_samples as i32;
    let coarse_max = max_lag / DECIMATION as i32;
    let ref_coarse = decimate(&reference);
    let target_coarse = decimate(&target);
    let mut best_coarse_lag = 0_i32;
    let mut best_coarse_abs = f32::NEG_INFINITY;
    for lag in lags_by_abs_asc(coarse_max) {
        let value = raw_correlation(&ref_coarse, &target_coarse, lag).abs();
        // 相对容差内的“并列”按最小 |lag| 裁决：周期信号的正峰与反相 trough
        // 在实数算术里严格相等，只差 f32 累加噪声（实测：480 周期正弦的
        // 15 与 45 两处 |raw| 只差累加舍入，严格 > 会误取 45 → 终值 360）。
        // 容差 1e-4 远小于真实相邻峰差（≥ 2e-4 量级），只吞舍入噪声。
        if value > best_coarse_abs * (1.0 + 1e-4) {
            best_coarse_abs = value;
            best_coarse_lag = lag;
        }
    }

    // 精修：粗 lag 附近 ±16 采样内逐 lag 归一化，取绝对值最大的极值。
    let refine_low = (best_coarse_lag * DECIMATION as i32 - REFINE_RADIUS as i32).max(-max_lag);
    let refine_high = (best_coarse_lag * DECIMATION as i32 + REFINE_RADIUS as i32).min(max_lag);
    let mut best_lag = 0_i32;
    let mut best_rho = 0.0_f32;
    let mut best_abs = f32::NEG_INFINITY;
    let mut rho_by_lag = std::collections::BTreeMap::new();
    for lag in lags_in_range_asc(refine_low, refine_high) {
        let rho = normalized_correlation(&reference, &target, lag);
        rho_by_lag.insert(lag, rho);
        // 与粗筛同一条并列裁决（容差按 f64 累加的舍入取 1e-6；抛物线插值
        // 会把 1 采样内的取舍误差抹平，终值仍落在真峰附近）。
        if rho.abs() > best_abs * (1.0 + 1e-6) {
            best_abs = rho.abs();
            best_rho = rho;
            best_lag = lag;
        }
    }

    // 抛物线插值（边界上无三点时不插值，lag 即整数峰）。
    let mut lag = f64::from(best_lag);
    if best_lag > refine_low
        && best_lag < refine_high
        && let (Some(&before), Some(&after)) = (
            rho_by_lag.get(&(best_lag - 1)),
            rho_by_lag.get(&(best_lag + 1)),
        )
    {
        lag += parabolic_offset(before, best_rho, after);
    }
    Ok(DelayMeasurement {
        lag_samples: lag,
        lag_samples_rounded: lag.round() as i32,
        correlation: best_rho,
        polarity_inverted: best_rho < 0.0,
        confidence: Confidence::from_correlation(best_rho),
        window_start_frame: window_start,
        window_frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性伪随机噪声（xorshift64；测试夹具专用，不进生产路径）。
    fn noise(seed: u64, len: usize) -> Vec<f32> {
        let mut state = seed | 1;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            // [-1, 1) 均匀噪声。
            out.push((state >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0);
        }
        out
    }

    /// 目标轨 = 基准轨延迟 `delay` 采样（前 `delay` 采样补 0）。
    fn delayed(signal: &[f32], delay: usize) -> Vec<f32> {
        let mut out = vec![0.0_f32; signal.len()];
        for (index, &sample) in signal.iter().enumerate() {
            if index + delay < out.len() {
                out[index + delay] = sample;
            }
        }
        out
    }

    /// 目标轨 = 基准轨延迟 `delay` 采样（线性插值，亚采样延迟）。
    fn delayed_fractional(signal: &[f32], delay: f64) -> Vec<f32> {
        let mut out = vec![0.0_f32; signal.len()];
        for index in 0..signal.len() {
            let source = index as f64 - delay;
            if source < 0.0 || source + 1.0 >= signal.len() as f64 {
                continue;
            }
            let base = source.floor() as usize;
            let alpha = (source - base as f64) as f32;
            out[index] = signal[base] * (1.0 - alpha) + signal[base + 1] * alpha;
        }
        out
    }

    #[test]
    fn integer_lag_is_recovered_exactly() {
        let reference = noise(0x1234_5678_9abc_def0, 48_000);
        let target = delayed(&reference, 240);
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert_eq!(measurement.lag_samples_rounded, 240);
        assert!((measurement.lag_samples - 240.0).abs() < 0.05);
        assert!(measurement.correlation > 0.99);
        assert!(!measurement.polarity_inverted);
        assert_eq!(measurement.confidence, Confidence::High);
        assert_eq!(measurement.window_start_frame, 0);
    }

    #[test]
    fn negative_lag_is_recovered() {
        // 目标轨比基准轨早到 120 采样：基准 = 目标延迟 120 后的版本。
        let target = noise(0xdead_beef_cafe_f00d, 48_000);
        let reference = delayed(&target, 120);
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert_eq!(measurement.lag_samples_rounded, -120);
        assert!(measurement.correlation > 0.99);
    }

    #[test]
    fn fractional_lag_interpolates() {
        let reference = noise(0x1111_2222_3333_4444, 96_000);
        let target = delayed_fractional(&reference, 240.4);
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        // 抛物线拟合三角峰顶有系统性偏差（真值 240.4 时拟合值约 240.25）：
        // 线性插值的相关峰是三角形，抛物线低估偏移量，最大偏差约 0.15。
        // 施加只用整数值（240），浮点值只作显示，容差取 0.3。
        assert!((measurement.lag_samples - 240.4).abs() < 0.3);
        assert_eq!(measurement.lag_samples_rounded, 240);
        // 线性插值把能量分到相邻两采样（0.6/0.4）：理论峰值 = 0.6 / √(0.6²+0.4²)
        // ≈ 0.83，不是 1.0；0.9 的门是错的，0.8 才是对的门。
        assert!(measurement.correlation > 0.8);
    }

    #[test]
    fn inverted_polarity_is_detected() {
        let reference = noise(0x5555_6666_7777_8888, 48_000);
        let mut target = delayed(&reference, 100);
        for sample in &mut target {
            *sample = -*sample;
        }
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert_eq!(measurement.lag_samples_rounded, 100);
        assert!(measurement.polarity_inverted);
        assert!(measurement.correlation < -0.99);
        assert_eq!(measurement.confidence, Confidence::High);
    }

    #[test]
    fn silence_is_an_error() {
        let reference = noise(0x9999_aaaa_bbbb_cccc, 48_000);
        let silent = vec![0.0_f32; 48_000];
        assert_eq!(
            measure_delay(&silent, &reference, 2_400),
            Err(MeasureDelayError::SilentReference)
        );
        assert_eq!(
            measure_delay(&reference, &silent, 2_400),
            Err(MeasureDelayError::SilentTarget)
        );
    }

    #[test]
    fn too_short_is_an_error() {
        let reference = noise(1, 100);
        let target = noise(2, 100);
        assert_eq!(
            measure_delay(&reference, &target, 100),
            Err(MeasureDelayError::TooShort { frames: 100 })
        );
        assert_eq!(
            measure_delay(&reference, &target, 0),
            Err(MeasureDelayError::InvalidMaxLag)
        );
    }

    #[test]
    fn uncorrelated_signals_have_low_correlation() {
        let reference = noise(0xaaaa_0000_1111_2222, 48_000);
        let target = noise(0xbbbb_3333_4444_5555, 48_000);
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert!(measurement.correlation.abs() < 0.1);
        assert_eq!(measurement.confidence, Confidence::Low);
    }

    #[test]
    fn periodic_signal_prefers_smallest_lag() {
        // 周期 480 采样的正弦：lag 120 与 lag 600 的相关峰相同，裁决取 |lag| 最小。
        let mut reference = Vec::with_capacity(48_000);
        for index in 0..48_000 {
            reference.push((2.0 * core::f32::consts::PI * index as f32 / 480.0).sin());
        }
        let target = delayed(&reference, 600);
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert_eq!(measurement.lag_samples_rounded, 120);
        assert!(measurement.correlation > 0.99);
    }

    #[test]
    fn window_picks_loudest_region() {
        // 前 40 秒静音、后 8 秒有声：窗口必须落在有声段。
        let mut reference = vec![0.0_f32; 48_000 * 48];
        let burst = noise(0x1234_9876_5432_10fe, 48_000 * 8);
        reference[48_000 * 40..].copy_from_slice(&burst);
        let target = delayed(&reference, 240);
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert_eq!(measurement.lag_samples_rounded, 240);
        assert!(measurement.window_start_frame >= 48_000 * 35);
        assert_eq!(measurement.window_frames, ANALYSIS_WINDOW_FRAMES);
    }

    #[test]
    fn identical_signals_have_zero_lag() {
        let reference = noise(0x7777_8888_9999_0000, 48_000);
        let target = reference.clone();
        let measurement = measure_delay(&reference, &target, 2_400).expect("可测");
        assert_eq!(measurement.lag_samples_rounded, 0);
        assert!(measurement.correlation > 0.999);
        assert!(!measurement.polarity_inverted);
    }
}
