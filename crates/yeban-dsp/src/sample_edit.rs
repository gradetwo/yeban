//! 采样库编辑的纯 DSP 件：基频测量、音准校正、力度层合成、RMS 归一。
//!
//! ## 它补的是哪一格
//!
//! Wang 2026-11-10 缺口报告 C-1：`build_erhu2.py` 逐采样 FFT 取基频 → 音准校正到
//! ±0.5 cent，用 `sus` 的 >2.2 kHz 分量伪造 mf/f 力度层，再 RMS 归一；284 个采样，
//! 手工不可能。yeban 有 SFZ 解析与播放，没有采样编辑器。
//!
//! ## 脚本已不可考（先写清楚，再写实现）
//!
//! `build_erhu2.py` 本人已不在工作区：全工作区 `find` 无此文件，memory 无记录，
//! 它随 2026-11-09 实验目录清理被删除。因此**无法**逐行复现脚本，只能按缺口报告的
//! 文字口径（FFT 基频、±0.5 cent 校正、>2.2 kHz 伪造力度层、RMS 归一）实现。
//! 其中两处口径按仓内已有部件落地，与字面方法不同、与要求相同：
//!
//! * **基频测量不用 FFT 峰值法**：仓内已有
//!   [`crate::vocal_pitch_shift::estimate_period`]（归一化自相关＋抛物线插值，
//!   F0 50–1000 Hz），它是被测试钉住的测量辅助；FFT 峰值法的 bin 量化（44.1 kHz
//!   下 16384 点 FFT 仍有 2.7 Hz bin）反而更难稳住 ±0.5 cent（440 Hz 处约
//!   ±0.13 Hz）。要求是"校正后落在 ±0.5 cent 内"，不是"必须用 FFT" ——
//!   方法服务于要求，不服务于字面。粗估计另有约 +2.4 音分的系统性偏高（抛物线
//!   插值＋有限窗包络），由本模块的分数滞后自相关精修（`refine_period`）除掉。
//! * **音准校正不用变速重采样**：仓内已有
//!   [`crate::vocal_pitch_shift::PitchShifter`]（逐样本周期重采样，**保时长**）。
//!   变速会改写样本长度与 SFZ `loop_start` / `loop_end` 的意义；保时长则循环点、
//!   时值、region 长度全部不变 —— 对采样库这是严格更优。移调量恒 ≤1200 音分
//!   （超出的不是"跑调"，是 SFZ 映射错了，见 [`SampleEditError::ShiftTooLarge`]）。
//!
//! 其余两处是本模块新增（仓内确实没有）：
//!
//! * **力度层合成**：2200 Hz 四阶 Butterworth 高通（两节 RBJ 二阶串联），取调好音的
//!   sus（或任一源采样）的高频分量；阶数是本人的选择（脚本已佚）：二阶在 1 kHz 处
//!   仍剩约 20%，四阶剩约 4% —— "只取 >2.2 kHz 分量"要求陡峭的截止。
//! * **RMS 归一**：缩放到目标 RMS；静音输入是明确错误（除以零没有意义）。
//!
//! ## 确定性
//!
//! 全模块纯函数、无随机源、无时钟、无线程：同一输入逐位同一输出（判据钉住）。
//! 超越函数走 `libm`（与 [`crate::math`] 同口径，[ARCH-DET-001]）。

use libm::{cos, log2, sin, sqrt};

use crate::MIN_SAMPLE_RATE;
use crate::vocal_pitch_shift::{
    MAX_SHIFT_CENTS, PitchShiftConfig, PitchShiftReceipt, PitchShifter, estimate_period_tracked,
};

/// 力度层合成的高通截止（Hz）：报告口径 ">2.2 kHz"。
pub const LAYER_CUTOFF_HZ: f32 = 2200.0;

/// 力度层合成后开头淡入（ms）：压住高通启动瞬态（见 [`synthesize_layer`]）。
pub const LAYER_FADE_IN_MS: f32 = 5.0;

/// 音准校正的死区（音分）：|偏差| ≤ 此值不处理 —— 既是 ±0.5 cent 的验收口径，
///
/// 也是"已经够准就别动样本"的 deadband（PSOLA 重写一个已准的样本只会引入 artifacts）。
pub const TUNE_DEADBAND_CENTS: f32 = 0.5;

/// 基频测量的窗长（采样数，与 [`crate::vocal_pitch_shift::DEFAULT_ESTIMATION_WINDOW_SAMPLES`] 同值）。
pub const MEASURE_WINDOW_SAMPLES: usize = 2048;

/// 基频测量的窗 hop（采样数）。
pub const MEASURE_HOP_SAMPLES: usize = 1024;

/// 采样库编辑的 DSP 错误（纯值，无 I/O）。
#[derive(Debug, Clone, PartialEq)]
pub enum SampleEditError {
    /// 输入全静音（RMS 为 0）：归一化没有可缩放的东西。
    Silence,
    /// 输入太短，连一个估计窗都放不下。
    TooShort,
    /// 没有一个窗估计出周期（能量为零 / 自相关峰值过低 / 不在 50–1000 Hz 内）。
    NoFundamental,
    /// 要求移调的量超过移调器上限（±1200 音分 = ±一个八度）：这不是"跑调"，
    /// 是 SFZ 映射错了（例如文件名八度与 `pitch_keycenter` 差两个八度），拒绝。
    ShiftTooLarge {
        /// 要求的移调量（音分）。
        cents: f32,
    },
    /// 目标 RMS 非法（非有限或 ≤0）。
    InvalidTargetRms(f32),
    /// 采样率太低，高通截止已越过 Nyquist（`sample_rate <= 2 × 2200 Hz`）。
    SampleRateTooLow(f32),
}

impl core::fmt::Display for SampleEditError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Silence => write!(formatter, "输入全静音，无法归一化"),
            Self::TooShort => write!(formatter, "输入太短，放不下一个估计窗"),
            Self::NoFundamental => write!(formatter, "没有估计出基频"),
            Self::ShiftTooLarge { cents } => write!(
                formatter,
                "要求移调 {cents:.1} 音分，超过移调器上限 ±{MAX_SHIFT_CENTS:.0} 音分：SFZ 映射可能错了"
            ),
            Self::InvalidTargetRms(target) => {
                write!(formatter, "目标 RMS {target} 非法（必须有限且 >0）")
            }
            Self::SampleRateTooLow(rate) => write!(
                formatter,
                "采样率 {rate} Hz 太低，高通截止 {LAYER_CUTOFF_HZ} Hz 已越过 Nyquist"
            ),
        }
    }
}

/// 一次基频测量的读数（回执的**唯一**来源：不从入参抄）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FundamentalMeasurement {
    /// 中位数基频（Hz）。
    pub hz: f64,
    /// 各窗估计相对中位数的最大偏离（音分）：测量稳不稳看它。
    pub spread_cents: f32,
    /// 成功估计的窗数。
    pub windows: usize,
}

/// 两个频率之间的音分差（`1200 × log2(m / t)`，正值 = 测得的比目标高）。
#[must_use]
pub fn cents_between(measured_hz: f64, target_hz: f64) -> f32 {
    (1200.0 * log2(measured_hz / target_hz)) as f32
}

/// 测量一个采样的基频：稳态窗＋逐窗跟踪＋中位数。
///
/// 窗取样本中段（跳过前 20% 起音、后 20% 尾部），每窗
/// [`MEASURE_WINDOW_SAMPLES`] 点、hop [`MEASURE_HOP_SAMPLES`] 点；每窗先乘 Hann
/// 窗（压住有限窗的边界项），上一窗的周期作跟踪提示
/// （`estimate_period_tracked` 的窄窗跟踪）；中位数抗住个别坏窗。
/// 窗数不足或全窗失败 ⇒ `None`（调用方记为 skip，不静默编一个数）。
///
/// ## 为什么每窗还要精修一次
///
/// `estimate_period_tracked` 的抛物线插值在纯正弦上有约 +2.4 音分的系统性偏高
/// （实测：440 Hz 正弦测得 440.60 Hz）：余弦峰上的抛物线顶点恒向最近整数滞后
/// 偏，且有限窗的三角包络把峰顶向小滞后推。±0.5 cent 的验收口径容不下它，因此
/// 每窗的粗估计再经 [`refine_period`] 精修（窗归一化的分数滞后自相关最大化）。
#[must_use]
pub fn measure_fundamental(samples: &[f32], sample_rate: f32) -> Option<FundamentalMeasurement> {
    if sample_rate < MIN_SAMPLE_RATE || samples.len() < MEASURE_WINDOW_SAMPLES {
        return None;
    }
    let len = samples.len();
    let start = len / 5;
    let end = len * 4 / 5;
    let mut frequencies: Vec<f64> = Vec::new();
    let mut prev: Option<f32> = None;
    let mut pos = start;
    while pos + MEASURE_WINDOW_SAMPLES <= end {
        let window = &samples[pos..pos + MEASURE_WINDOW_SAMPLES];
        if let Some(period) = estimate_in_window(window, sample_rate, prev) {
            frequencies.push(f64::from(sample_rate) / f64::from(period));
            prev = Some(period);
        }
        pos += MEASURE_HOP_SAMPLES;
    }
    if frequencies.is_empty() {
        // 短样本：中段放不下一个完整窗时，退回"整段一个窗"（仍须 ≥512 点，
        // 否则 `estimate_period_tracked` 自己会拒绝）。
        let window = &samples[start.min(len)..];
        if let Some(period) = estimate_in_window(window, sample_rate, None) {
            frequencies.push(f64::from(sample_rate) / f64::from(period));
        }
    }
    if frequencies.is_empty() {
        return None;
    }
    frequencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let median = frequencies[frequencies.len() / 2];
    let spread = frequencies
        .iter()
        .map(|hz| cents_between(*hz, median).abs())
        .fold(0.0_f32, f32::max);
    Some(FundamentalMeasurement {
        hz: median,
        spread_cents: spread,
        windows: frequencies.len(),
    })
}

/// 一个窗内的基频估计：Hann 窗 → 粗估计 → 精修，返回周期（采样数）。
///
/// 窗口函数压住有限窗的边界项（矩形窗的边界项在 2048 点窗上仍有约 1 音分的
/// 残差）；精修再除掉窗本身的自相关倾斜（见 [`refine_period`]）。
fn estimate_in_window(window: &[f32], sample_rate: f32, prev: Option<f32>) -> Option<f32> {
    if window.len() < MEASURE_WINDOW_SAMPLES.min(512) {
        return None;
    }
    let n = window.len();
    let hann: Vec<f32> = (0..n)
        .map(|i| {
            0.5 * (1.0 - (2.0 * core::f64::consts::PI * i as f64 / (n - 1) as f64).cos() as f32)
        })
        .collect();
    let windowed: Vec<f32> = window.iter().zip(hann.iter()).map(|(x, w)| x * w).collect();
    let coarse = estimate_period_tracked(&windowed, sample_rate, 0.5, prev)?;
    Some(refine_period(&windowed, &hann, coarse))
}

/// 分数位置采样（Catmull-Rom 三次插值）：`pos` 越界时钳到端点。
fn fractional_sample(samples: &[f32], pos: f64) -> f64 {
    let len = samples.len();
    if len == 0 {
        return 0.0;
    }
    let clamped = pos.clamp(0.0, (len - 1) as f64);
    let i = clamped.floor() as usize;
    let t = clamped - i as f64;
    let get = |index: isize| -> f64 {
        let clamped_index = index.clamp(0, (len - 1) as isize) as usize;
        f64::from(samples[clamped_index])
    };
    let (p0, p1, p2, p3) = (
        get(i as isize - 1),
        get(i as isize),
        get(i as isize + 1),
        get(i as isize + 2),
    );
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)
}

/// 分数滞后的自相关（按重叠点数归一的均值积）：三角包络被除掉，峰顶即真周期。
///
/// 分母用重叠点数而不用总能量 —— 总能量归一的版本带 `(1 - τ/N)` 包络，峰顶向
/// 小滞后偏约 0.12 个采样（440 Hz 处约 2 音分，正是粗估计偏差的另一半来源）。
fn mean_product_at_lag(samples: &[f32], lag: f64) -> f64 {
    let usable = (samples.len() as f64 - lag - 2.0).floor() as usize;
    if usable == 0 {
        return f64::NEG_INFINITY;
    }
    let mut sum = 0.0_f64;
    for n in 0..usable {
        sum += f64::from(samples[n]) * fractional_sample(samples, n as f64 + lag);
    }
    sum / usable as f64
}

/// 精修周期：以粗估计为中心 ±2% 做黄金分割搜索，最大化**窗归一化**的分数滞后自相关。
///
/// 搜索区间内自相关只有一个主瓣（余弦主瓣宽 ±T/2，区间只有 ±0.02T），黄金分割
/// 安全；40 轮后区间缩到约 1e-8 个采样，远超所需精度。
///
/// 窗归一化是关键：Hann 窗本身的自相关随滞后递减，会把峰顶向小滞后推（与矩形窗
/// 的边界项同量级）；目标函数除以窗在同一滞后的自相关后，倾斜被除掉，峰顶回到
/// 真周期。只除均值积（不除总能量）时，矩形窗的 `(1 - τ/N)` 包络也已被除掉。
fn refine_period(windowed: &[f32], window: &[f32], coarse: f32) -> f32 {
    let score = |lag: f64| -> f64 {
        let numerator = mean_product_at_lag(windowed, lag);
        let denominator = mean_product_at_lag(window, lag);
        if denominator <= 0.0 || !numerator.is_finite() {
            return f64::NEG_INFINITY;
        }
        numerator / denominator
    };
    let coarse = f64::from(coarse);
    let (mut lo, mut hi) = (coarse * 0.98, coarse * 1.02);
    let inv_phi = (sqrt(5.0) - 1.0) / 2.0;
    let mut c = hi - inv_phi * (hi - lo);
    let mut d = lo + inv_phi * (hi - lo);
    let mut fc = score(c);
    let mut fd = score(d);
    for _ in 0..40 {
        if fc > fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - inv_phi * (hi - lo);
            fc = score(c);
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + inv_phi * (hi - lo);
            fd = score(d);
        }
    }
    ((lo + hi) / 2.0) as f32
}

/// 音准校正：把样本的音高移动 `shift_cents` 音分（正值升高），**时长不变**。
///
/// * `|shift_cents| <= 0.5`（死区）⇒ 原样返回（逐位相同），不重写已准的样本；
/// * `|shift_cents| > 1200` ⇒ [`SampleEditError::ShiftTooLarge`]（映射错了，拒绝）；
/// * 其余 ⇒ [`PitchShifter`] 全浊音直推（攻击段也走移调：一次校正里没有"清音段"，
///   全浊音避免浊音／清音边界的过渡淡入淡出改写起音）。
///
/// # Errors
///
/// [`SampleEditError::ShiftTooLarge`]（移调量超限）或非有限移调量。
pub fn correct_pitch(
    samples: &[f32],
    sample_rate: f32,
    shift_cents: f32,
) -> Result<(Vec<f32>, PitchShiftReceipt), SampleEditError> {
    if !shift_cents.is_finite() {
        return Err(SampleEditError::ShiftTooLarge { cents: shift_cents });
    }
    if shift_cents.abs() <= TUNE_DEADBAND_CENTS {
        return Ok((samples.to_vec(), PitchShiftReceipt::default()));
    }
    if shift_cents.abs() > MAX_SHIFT_CENTS {
        return Err(SampleEditError::ShiftTooLarge { cents: shift_cents });
    }
    let config = PitchShiftConfig {
        shift_cents,
        ..PitchShiftConfig::default()
    };
    let mut shifter = PitchShifter::new(sample_rate, config);
    let chunk = PitchShiftConfig::default().chunk_samples;
    let chunks = samples.len().div_ceil(chunk).max(1);
    let voiced = vec![true; chunks];
    Ok(shifter.process(samples, &voiced))
}

/// 力度层合成：取源采样的 >2.2 kHz 分量（四阶 Butterworth 高通，两节 RBJ 串联）。
///
/// 输出与输入等长、未归一（调用方按层目标 RMS 归一）。共振峰不修正：层是**音色**
/// 素材，不是独立乐器；移调器同样不修正共振峰（factor 恒 1.0），口径一致。
///
/// 开头 5 ms 升余弦淡入：高通从零状态启动时，前几毫秒会冒出一个比稳态体
/// 大 40 倍的瞬态（440 Hz 正弦实测：层峰值 0.0225、体 RMS 0.00055）；不压住它，
/// 调用方按整层 RMS 归一时增益被瞬态抬高的读数压小、体落到目标下 0.9–2.2 dB，
/// 归一后峰值 4.89 又在写盘量化时硬削波 —— 两处同根。淡入只动开头 5 ms，
/// 层的主体与长度不变。
///
/// # Errors
///
/// [`SampleEditError::SampleRateTooLow`]（采样率 ≤ 2 × 截止频率）。
pub fn synthesize_layer(samples: &[f32], sample_rate: f32) -> Result<Vec<f32>, SampleEditError> {
    if sample_rate <= 2.0 * LAYER_CUTOFF_HZ {
        return Err(SampleEditError::SampleRateTooLow(sample_rate));
    }
    let mut out: Vec<f32> = samples.to_vec();
    // 两节二阶 Butterworth 高通串联 = 四阶（-24 dB/oct）：一节在 1 kHz 处仍剩约
    // 20%，两节剩约 4% —— "只取 >2.2 kHz 分量"要求陡峭的截止。
    for _ in 0..2 {
        highpass_biquad(&mut out, sample_rate);
    }
    // 压住启动瞬态（见上）：5 ms 升余弦淡入，短样本时不超总长一半。
    let fade = ((LAYER_FADE_IN_MS / 1000.0 * sample_rate) as usize).min(out.len() / 2);
    for (index, sample) in out.iter_mut().take(fade).enumerate() {
        let phase = core::f32::consts::PI * index as f32 / fade.max(1) as f32;
        *sample *= 0.5 - 0.5 * phase.cos();
    }
    Ok(out)
}

/// 一节 RBJ 二阶 Butterworth 高通（Q = 1/√2），就地 Direct Form I。
///
/// 系数用 `f64` 算、状态用 `f64` 存：`f32` 状态在长样本上会累出可测的直流漂移。
fn highpass_biquad(samples: &mut [f32], sample_rate: f32) {
    let w0 = 2.0 * core::f64::consts::PI * f64::from(LAYER_CUTOFF_HZ) / f64::from(sample_rate);
    let q = core::f64::consts::FRAC_1_SQRT_2;
    let alpha = sin(w0) / (2.0 * q);
    let cos_w0 = cos(w0);
    let b0 = (1.0 + cos_w0) / 2.0;
    let b1 = -(1.0 + cos_w0);
    let b2 = (1.0 + cos_w0) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;
    let (b0, b1, b2, a1, a2) = (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    for sample in samples.iter_mut() {
        let x0 = f64::from(*sample);
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *sample = y0 as f32;
    }
}

/// 均方根（`f64` 累加：`f32` 累加在长样本上会丢精度）。
#[must_use]
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|x| f64::from(*x) * f64::from(*x)).sum();
    sqrt(sum / samples.len() as f64) as f32
}

/// RMS 归一：缩放到目标 RMS，返回（归一后样本，归一后峰值）。
///
/// 峰值一并返回：目标 RMS 把峰值顶过 1.0 时调用方必须在报告里写出来（响亮），
/// ⛔ 不静默削波。
///
/// # Errors
///
/// [`SampleEditError::InvalidTargetRms`]（目标非有限或 ≤0）、
/// [`SampleEditError::Silence`]（输入全静音）。
pub fn normalize_rms(samples: &[f32], target_rms: f32) -> Result<(Vec<f32>, f32), SampleEditError> {
    if !target_rms.is_finite() || target_rms <= 0.0 {
        return Err(SampleEditError::InvalidTargetRms(target_rms));
    }
    let current = rms(samples);
    if current == 0.0 {
        return Err(SampleEditError::Silence);
    }
    let gain = target_rms / current;
    let mut peak = 0.0_f32;
    let out: Vec<f32> = samples
        .iter()
        .map(|x| {
            let y = *x * gain;
            peak = peak.max(y.abs());
            y
        })
        .collect();
    Ok((out, peak))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::note_to_hz;

    /// 2 秒纯正弦（44.1 kHz），指定频率与幅度。
    fn sine(hz: f64, seconds: f32, amplitude: f32) -> Vec<f32> {
        let rate = 44_100.0_f32;
        let len = (seconds * rate) as usize;
        (0..len)
            .map(|i| {
                (2.0 * core::f64::consts::PI * hz * (i as f64) / f64::from(rate)).sin() as f32
                    * amplitude
            })
            .collect()
    }

    /// 判据：纯正弦的基频测量在 1 音分内（测量本身的精度底线）。
    #[test]
    fn measure_fundamental_finds_a_pure_sine_within_one_cent() {
        let samples = sine(440.0, 2.0, 0.5);
        let measured = measure_fundamental(&samples, 44_100.0).expect("测得出");
        let cents = cents_between(measured.hz, 440.0).abs();
        assert!(
            cents < 1.0,
            "440 Hz 正弦测得 {} Hz，偏离 {cents:.3} 音分（限 1 音分）",
            measured.hz
        );
        assert!(
            measured.windows >= 3,
            "窗数 {}（至少 3 窗）",
            measured.windows
        );
    }

    /// 判据：445 Hz 正弦校正到 440 Hz 后，再测落在 ±0.5 cent 内（本模块的 headline 口径）。
    #[test]
    fn correct_pitch_lands_within_half_a_cent() {
        let samples = sine(445.0, 2.0, 0.5);
        let shift = -cents_between(445.0, 440.0);
        let (corrected, _) = correct_pitch(&samples, 44_100.0, shift).expect("能校正");
        assert_eq!(corrected.len(), samples.len(), "校正保时长");
        let remeasured = measure_fundamental(&corrected, 44_100.0).expect("校正后测得出");
        let cents = cents_between(remeasured.hz, 440.0).abs();
        assert!(
            cents <= TUNE_DEADBAND_CENTS,
            "校正后 {} Hz，偏离 {cents:.3} 音分（限 ±0.5 音分）",
            remeasured.hz
        );
    }

    /// 判据：死区内（0.2 音分）原样返回，逐位相同。
    #[test]
    fn correct_pitch_deadband_returns_the_input_bit_for_bit() {
        let samples = sine(440.0, 1.0, 0.5);
        let (out, _) = correct_pitch(&samples, 44_100.0, 0.2).expect("死区内不报错");
        assert_eq!(out, samples, "死区内必须逐位相同");
    }

    /// 判据：超过一个八度的移调量拒绝（映射错了，不是跑调）。
    #[test]
    fn correct_pitch_refuses_more_than_an_octave() {
        let samples = sine(440.0, 1.0, 0.5);
        let error = correct_pitch(&samples, 44_100.0, 2400.0).expect_err("2400 音分必须拒绝");
        assert_eq!(error, SampleEditError::ShiftTooLarge { cents: 2400.0 });
    }

    /// 判据：力度层只留高频带 —— 1 kHz 正弦剩 <6%，5 kHz 正弦剩 >95%。
    #[test]
    fn synthesize_layer_keeps_only_the_high_band() {
        let low = sine(1000.0, 1.0, 0.5);
        let high = sine(5000.0, 1.0, 0.5);
        let low_layer = synthesize_layer(&low, 44_100.0).expect("能合成");
        let high_layer = synthesize_layer(&high, 44_100.0).expect("能合成");
        // 起振瞬态：高通前 1000 点不计（滤波器建立段）。
        let low_ratio = rms(&low_layer[1000..]) / rms(&low);
        let high_ratio = rms(&high_layer[1000..]) / rms(&high);
        assert!(low_ratio < 0.06, "1 kHz 层剩 {low_ratio:.3}（限 <6%）");
        assert!(high_ratio > 0.95, "5 kHz 层剩 {high_ratio:.3}（限 >95%）");
    }

    /// 判据：层开头无启动瞬态 —— 440 Hz 正弦的层，峰值须 < 体 RMS 的 8 倍
    /// （淡入前实测 41 倍：峰值 0.0225、体 RMS 0.00055；瞬态不压住时归一
    /// 增益被抬高的整层读数压小、体落到目标下 0.9–2.2 dB，归一后峰值 4.89
    /// 又在写盘量化时硬削波）。
    #[test]
    fn synthesize_layer_has_no_start_transient() {
        let samples = sine(440.0, 1.0, 0.5);
        let layer = synthesize_layer(&samples, 44_100.0).expect("能合成");
        let body_rms = rms(&layer[1000..]);
        let peak = layer.iter().fold(0.0f32, |max, x| max.max(x.abs()));
        assert!(
            peak < 8.0 * body_rms,
            "层峰值 {peak:.5}（体 RMS {body_rms:.5}，限 <8 倍）"
        );
    }

    /// 判据：RMS 归一到达目标值（±0.1%），峰值如实报出。
    #[test]
    fn normalize_rms_reaches_the_target() {
        let samples = sine(440.0, 1.0, 0.3);
        let (out, peak) = normalize_rms(&samples, 0.1).expect("能归一");
        let got = rms(&out);
        assert!(
            (got - 0.1).abs() / 0.1 < 0.001,
            "RMS {got}（目标 0.1，限 ±0.1%）"
        );
        assert!(
            (peak - 0.1 * core::f32::consts::SQRT_2).abs() < 1e-5,
            "峰值 {peak}"
        );
    }

    /// 判据：全静音归一化是明确错误，不是静默返回零。
    #[test]
    fn normalize_rms_refuses_silence() {
        let silence = vec![0.0_f32; 4096];
        let error = normalize_rms(&silence, 0.1).expect_err("静音必须拒绝");
        assert_eq!(error, SampleEditError::Silence);
    }

    /// 判据：同一输入跑两次，三件工具的输出逐位相同（确定性）。
    #[test]
    fn the_pipeline_is_deterministic() {
        let samples = sine(445.0, 2.0, 0.5);
        let shift = -cents_between(445.0, 440.0);
        let (a_tuned, _) = correct_pitch(&samples, 44_100.0, shift).expect("能校正");
        let (b_tuned, _) = correct_pitch(&samples, 44_100.0, shift).expect("能校正");
        assert_eq!(a_tuned, b_tuned, "校正逐位相同");
        let a_layer = synthesize_layer(&a_tuned, 44_100.0).expect("能合成");
        let b_layer = synthesize_layer(&b_tuned, 44_100.0).expect("能合成");
        assert_eq!(a_layer, b_layer, "层合成逐位相同");
        let (a_norm, _) = normalize_rms(&a_layer, 0.1).expect("能归一");
        let (b_norm, _) = normalize_rms(&b_layer, 0.1).expect("能归一");
        assert_eq!(a_norm, b_norm, "归一逐位相同");
    }

    /// 判据：MIDI 69 = 440 Hz（`note_to_hz` 的口径，SFZ `pitch_keycenter` 的目标频率来源）。
    #[test]
    fn midi_69_is_440_hz() {
        assert!((note_to_hz(69.0) - 440.0).abs() < 1e-4, "69 号音 = 440 Hz");
    }
}
