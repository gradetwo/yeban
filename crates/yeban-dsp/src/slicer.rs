//! 音频切片：onset 检测切出片段 → 三变形 → 淡入淡出。
//!
//! ## 它补的是哪一格
//!
//! Wang 2026-11-10 的缺口分级（C 级第 2 条）指出：`chop.py` 在仓库外做
//! "onset 检测切到字 → tape（变速变调）/shift/stretch 三变形 → 4/10 ms
//! 淡入淡出"，而 yeban 里没有"切成采样器 region"这个动作 ——
//! `import_audio` 只能整条导入；`transient` 是 [`crate::shaping`] 里的
//! 瞬态塑形**效果器**（改音色，不检测）；`anatomy.rs` 是 Anatomy 底鼓
//! **合成器**（发声，不检测）；全仓没有任何音频 onset 检测。
//! 本模块就是那个动作的 DSP 核：切点、三变形、淡入淡出；落盘成采样器
//! region（SFZ 文本 ＋ 每片一段 WAV）由 [`yeban-app` 的 `audio_slicer`
//! 模块](../yeban-app/src/audio_slicer.rs)接在 CLI 出口上。
//!
//! ## 流程口径（与 chop.py 的对应关系）
//!
//! `chop.py` 本身已随 2026-11-09 的清理从工作区删除，流程以 Wang
//! 2026-11-10 报告的一行为准（"onset 检测切到字 → tape（变速变调）/
//! shift/stretch 三变形 → 4/10 ms 淡入淡出"），细节按下面逐条钉死：
//!
//! | 步骤 | 本模块的做法 | 与 chop.py 的对应 |
//! | :--- | :--- | :--- |
//! | onset 检测 | 预加重（一阶差分）＋ 帧能量通量 ＋ 自适应阈值（局部中位数 × 系数）＋ 最小间隔贪心取峰 | "onset 检测切到字"；无 FFT、无新依赖 |
//! | tape（变速变调） | [`tape_varispeed`]：Catmull-Rom 三次插值重采样，时长与音高同变 | "tape（变速变调）"；⚠ `yeban-dsp` 的 [`crate::tape`] 是**饱和器**，不是变速 —— 不要混淆 |
//! | shift（移调） | [`crate::vocal_pitch_shift::PitchShifter`]（周期重采样，时长不变） | "shift"；切片按整段浊音处理（`voiced` 全真），因切片本就是唱出的音节 |
//! | stretch（拉伸） | [`crate::vocal_timemap::render_continuous_timemap`]（signalsmith，音高不变） | "stretch"；均匀映射（单段直线） |
//! | 淡入淡出 | [`apply_fades`]：线性，淡入 4 ms、淡出 10 ms | "4/10 ms 淡入淡出"：淡入 4 ms、淡出 10 ms |
//!
//! ## 离线限定
//!
//! 与 [`crate::vocal_pitch_shift`] / [`crate::vocal_timemap`] 同款：
//! 本模块只用于离线切片，**绝不进入实时回调** ——
//! [`detect_onsets`] 返回 `Vec`、 [`tape_varispeed`] 分配输出，
//! 不是逐块定长预算。调用方必须在离线线程调用。
//!
//! ## 确定性
//!
//! 全部参数钉死在常量里（帧长／跳长／阈值系数／淡入淡出毫秒数），
//! 不读时钟、不读环境变量、不引入随机源；同一输入逐位相同
//! （判据 `detecting_the_same_audio_twice_gives_the_same_cuts` 钉住）。

use std::cmp::Ordering;

/// onset 检测的帧跳长（采样）。512 @ 48 kHz ≈ 10.7 ms：切点误差不大于一跳。
pub const DEFAULT_HOP_FRAMES: usize = 512;
/// onset 检测的帧窗长（采样）。1024 @ 48 kHz ≈ 21.3 ms，与跳长 50% 重叠。
pub const DEFAULT_WINDOW_FRAMES: usize = 1024;
/// 自适应阈值系数：阈值 = 局部通量中位数 × 此系数（再与绝对底线取大）。
pub const DEFAULT_THRESHOLD_FACTOR: f32 = 2.0;
/// 绝对底线（dBFS RMS）：帧能量与通量低于此值不触发（静音不切）。
pub const DEFAULT_FLOOR_DB: f32 = -50.0;
/// 两切点之间的最小间隔（毫秒）：小于此间隔的弱峰被强的吞掉。
pub const DEFAULT_MIN_GAP_MS: f32 = 80.0;
/// 淡入时长（毫秒）：chop.py 的"4 ms 淡入"。
pub const FADE_IN_MS: f32 = 4.0;
/// 淡出时长（毫秒）：chop.py 的"10 ms 淡出"。
pub const FADE_OUT_MS: f32 = 10.0;
/// tape 变速比合法域下界（0.25× = 慢 4 倍、低两个八度）。
pub const TAPE_RATE_MIN: f32 = 0.25;
/// tape 变速比合法域上界（4× = 快 4 倍、高两个八度）。
pub const TAPE_RATE_MAX: f32 = 4.0;
/// stretch 拉伸比合法域下界（输出秒 / 输入秒）。
pub const STRETCH_RATE_MIN: f32 = 0.25;
/// stretch 拉伸比合法域上界（输出秒 / 输入秒）。
pub const STRETCH_RATE_MAX: f32 = 4.0;

/// 相对通量下限：通量须 ≥ 帧能量的此比例才算 onset。
/// 稳态音的窗长抖动（窗内非整数周期所致）≤ 帧能量的约 1.7%（440 Hz）
/// ～约 14%（55 Hz 纯正弦最坏情形）；真起音一跳内能量翻倍级跳变，
/// 通量 ≥ 帧能量的约 50%。15% 把两者分开。
pub const RELATIVE_FLUX_MIN: f32 = 0.15;
/// onset 检测的选项（全部有缺省；非法值防御性落回缺省，不 panic）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnsetOptions {
    /// 帧跳长（采样）。
    pub hop_frames: usize,
    /// 帧窗长（采样）。
    pub window_frames: usize,
    /// 自适应阈值系数。
    pub threshold_factor: f32,
    /// 绝对底线（dBFS RMS）。
    pub floor_db: f32,
    /// 最小间隔（毫秒）。
    pub min_gap_ms: f32,
}

impl Default for OnsetOptions {
    fn default() -> Self {
        Self {
            hop_frames: DEFAULT_HOP_FRAMES,
            window_frames: DEFAULT_WINDOW_FRAMES,
            threshold_factor: DEFAULT_THRESHOLD_FACTOR,
            floor_db: DEFAULT_FLOOR_DB,
            min_gap_ms: DEFAULT_MIN_GAP_MS,
        }
    }
}

impl OnsetOptions {
    /// 钳到合法域的副本：跳长／窗长非零、系数与间隔为正且有限、底线有限。
    /// 非法值落回缺省（调用方传错不 panic、不静默接受一个无意义的检测器）。
    #[must_use]
    pub fn sanitised(self) -> Self {
        let defaults = Self::default();
        Self {
            hop_frames: if self.hop_frames > 0 {
                self.hop_frames
            } else {
                defaults.hop_frames
            },
            window_frames: if self.window_frames > 0 {
                self.window_frames
            } else {
                defaults.window_frames
            },
            threshold_factor: if self.threshold_factor.is_finite() && self.threshold_factor > 0.0 {
                self.threshold_factor
            } else {
                defaults.threshold_factor
            },
            floor_db: if self.floor_db.is_finite() {
                self.floor_db
            } else {
                defaults.floor_db
            },
            min_gap_ms: if self.min_gap_ms.is_finite() && self.min_gap_ms > 0.0 {
                self.min_gap_ms
            } else {
                defaults.min_gap_ms
            },
        }
    }
}

/// 切片失败的原因（调用方如实上报，不静默降级）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlicerError {
    /// 变速／拉伸比越界或非有限。
    RateOutOfRange {
        /// 哪一种变形（`"tape"` / `"stretch"`）。
        kind: &'static str,
        /// 给定的比值。
        rate: f32,
        /// 合法域下界。
        min: f32,
        /// 合法域上界。
        max: f32,
    },
}

impl core::fmt::Display for SlicerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RateOutOfRange {
                kind,
                rate,
                min,
                max,
            } => write!(
                formatter,
                "{kind} 的比值 {rate} 越界（合法域 {min}..={max}）"
            ),
        }
    }
}

impl std::error::Error for SlicerError {}

/// 非有限样本归零（与 [`crate::shaping`] 同一条理由：`NaN` 会污染
/// 能量统计与插值，归零是最保守的处置）。
#[inline]
#[must_use]
fn sanitise(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// 检测 onset，返回切点（采样下标，升序）。
///
/// 算法（无 FFT、无新依赖，全部参数见 [`OnsetOptions`]）：
/// 1. 预加重（一阶差分 `x[n] − x[n−1]`）：起音的瞬态比稳态响得多，
///    差分后起音凸出、稳态被压住；
/// 2. 帧能量：窗 `window_frames`、跳 `hop_frames`，帧内预加重样本的均方；
/// 3. 能量通量：`max(0, E[k] − E[k−1])`（只看"变响"，不看"变轻"）；
/// 4. 自适应阈值：`max(局部 ±10 帧通量中位数 × 系数, 绝对底线)`，
///    且帧能量本身也要过绝对底线（静音段不触发）；再加一道相对门：
///    通量须 ≥ 帧能量的 15%（稳态音的窗长抖动再大也不过约 14%，
///    真起音一跳内通量 ≥ 帧能量的约 50%）；
/// 5. 局部峰：通量不小于左右邻帧；
/// 6. 最小间隔贪心：按通量从大到小收，离已收切点不足 `min_gap_ms` 的不要。
///
/// 切点是帧起点（`k × hop`），误差不大于一跳（512 @ 48 kHz ≈ 10.7 ms）。
/// 空输入或采样率为 0 ⇒ 空（不是错误：没有音频就没有切点）。
#[must_use]
pub fn detect_onsets(samples: &[f32], sample_rate: u32, options: &OnsetOptions) -> Vec<usize> {
    let options = options.sanitised();
    if samples.is_empty() || sample_rate == 0 {
        return Vec::new();
    }
    let hop = options.hop_frames;
    let window = options.window_frames.max(hop);
    let n_frames = samples.len().div_ceil(hop);

    // 帧能量（预加重后均方；末帧零填充到窗长）。
    // ⚠ 预加重是流式差分：每帧首的"前一样本"是本帧起点的前一个样本
    // （不是上一帧的末样本 —— 帧 50% 重叠，上一帧末尾比本帧起点晚半帧；
    // 拿错会把相隔 511 个样本的两值相减，每帧首注入一个假尖峰）。
    let mut energy = Vec::with_capacity(n_frames);
    for frame in 0..n_frames {
        let start = frame * hop;
        let mut prev = if start == 0 {
            0.0
        } else {
            sanitise(samples[start - 1])
        };
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for index in 0..window {
            let at = start + index;
            if at >= samples.len() {
                break;
            }
            let current = sanitise(samples[at]);
            let diff = current - prev;
            prev = current;
            sum += diff * diff;
            count += 1;
        }
        energy.push(if count > 0 { sum / count as f32 } else { 0.0 });
    }

    // 能量通量（只取正向）。
    let mut flux = Vec::with_capacity(n_frames);
    for frame in 0..n_frames {
        let prev_energy = if frame == 0 { 0.0 } else { energy[frame - 1] };
        flux.push((energy[frame] - prev_energy).max(0.0));
    }

    // 绝对底线：dBFS RMS → 能量（均方）。
    let energy_floor = 10f32.powf(options.floor_db / 10.0);
    let min_gap = ((options.min_gap_ms / 1000.0) * sample_rate as f32)
        .round()
        .max(1.0) as usize;

    // 候选：过阈值 ＋ 局部峰。两道门缺一不可：
    // ① 自适应阈值（局部通量中位数 × 系数，与绝对底线取大）—— 适应局部
    //    响度与抖动水平；② 相对通量下限（通量 ≥ 帧能量的 15%）—— 稳态
    //    音的窗长抖动（窗内非整数周期所致）再大也不过帧能量的约 14%，
    //    真起音一跳内通量 ≥ 帧能量的约 50%，15% 把两者分开。
    //    只留 ① 时，440 Hz 稳态正弦的抖动峰会贴着阈值线过关（实测）。
    let mut candidates: Vec<(usize, f32)> = Vec::new();
    for frame in 0..n_frames {
        if energy[frame] <= energy_floor || flux[frame] <= energy_floor {
            continue;
        }
        if flux[frame] < RELATIVE_FLUX_MIN * energy[frame] {
            continue;
        }
        let low = frame.saturating_sub(10);
        let high = (frame + 10).min(n_frames - 1);
        let mut window_flux: Vec<f32> = flux[low..=high].to_vec();
        window_flux.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let median = window_flux[window_flux.len() / 2];
        let threshold = (median * options.threshold_factor).max(energy_floor);
        if flux[frame] <= threshold {
            continue;
        }
        let left = if frame == 0 {
            f32::NEG_INFINITY
        } else {
            flux[frame - 1]
        };
        let right = if frame + 1 >= n_frames {
            f32::NEG_INFINITY
        } else {
            flux[frame + 1]
        };
        if flux[frame] < left || flux[frame] < right {
            continue;
        }
        candidates.push((frame, flux[frame]));
    }

    // 最小间隔贪心：通量大的先收，近的不要；最后按时间排回升序。
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    let mut accepted: Vec<usize> = Vec::new();
    for (frame, _) in candidates {
        let onset = frame * hop;
        if accepted
            .iter()
            .all(|&taken| onset.abs_diff(taken) >= min_gap)
        {
            accepted.push(onset);
        }
    }
    accepted.sort_unstable();
    accepted
}

/// tape 变速变调（`rate` = 输出速率 / 输入速率：`> 1` 变快变高，`< 1` 变慢变低）。
///
/// 输出长度 `⌊输入帧数 / rate⌋`、音高 × `rate`：`rate = 2` 时一首 440 Hz 的
/// 正弦出来是 880 Hz、时长一半。插值是 Catmull-Rom 三次（边缘下标夹住，
/// 不越界）；非有限输入样本按 0 处理。
///
/// # Errors
///
/// `rate` 非有限或不在 [`TAPE_RATE_MIN`]..=[`TAPE_RATE_MAX`] 内时
/// [`SlicerError::RateOutOfRange`]（调用方如实报，不静默钳位）。
pub fn tape_varispeed(samples: &[f32], rate: f32) -> Result<Vec<f32>, SlicerError> {
    if !rate.is_finite() || rate < TAPE_RATE_MIN || rate > TAPE_RATE_MAX {
        return Err(SlicerError::RateOutOfRange {
            kind: "tape",
            rate,
            min: TAPE_RATE_MIN,
            max: TAPE_RATE_MAX,
        });
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let out_len = ((samples.len() as f64) / f64::from(rate)).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        out.push(catmull_rom(samples, index as f32 * rate));
    }
    Ok(out)
}

/// Catmull-Rom 三次插值（`pos` 是输入坐标，`t` 是小数部分）。
fn catmull_rom(samples: &[f32], pos: f32) -> f32 {
    let len = samples.len();
    let base = pos.floor() as isize;
    let t = pos - base as f32;
    let at = |offset: isize| -> f32 {
        let clamped = offset.clamp(0, (len - 1) as isize) as usize;
        sanitise(samples[clamped])
    };
    let p0 = at(base - 1);
    let p1 = at(base);
    let p2 = at(base + 1);
    let p3 = at(base + 2);
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// 淡入淡出（就地）：线性淡入 [`FADE_IN_MS`]、线性淡出 [`FADE_OUT_MS`]。
///
/// 首样本 ×0、淡入结束处 ×1、末样本 ×0；片段短于两淡入淡出之和时，
/// 淡入取 `len/2`、淡出取剩余（不重叠、不反向）。空输入与采样率 0 ⇒ 不动。
pub fn apply_fades(samples: &mut [f32], sample_rate: u32) {
    let len = samples.len();
    if len == 0 || sample_rate == 0 {
        return;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fade_in = ((FADE_IN_MS / 1000.0) * sample_rate as f32).round() as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fade_out = ((FADE_OUT_MS / 1000.0) * sample_rate as f32).round() as usize;
    let fade_in = fade_in.min(len / 2);
    let fade_out = fade_out.min(len - fade_in);
    for (index, sample) in samples.iter_mut().enumerate().take(fade_in) {
        *sample *= (index as f32) / (fade_in as f32);
    }
    for index in 0..fade_out {
        let at = len - fade_out + index;
        samples[at] *= ((fade_out - 1 - index) as f32) / (fade_out as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空输入 / 零采样率 / 非法选项不 panic、不产出切点。
    #[test]
    fn empty_or_degenerate_input_detects_nothing() {
        assert!(detect_onsets(&[], 48_000, &OnsetOptions::default()).is_empty());
        assert!(detect_onsets(&[0.5; 100], 0, &OnsetOptions::default()).is_empty());
        let hostile = OnsetOptions {
            hop_frames: 0,
            window_frames: 0,
            threshold_factor: f32::NAN,
            floor_db: f32::INFINITY,
            min_gap_ms: -1.0,
        };
        assert!(detect_onsets(&[0.5; 100], 48_000, &hostile).is_empty());
    }

    /// tape 比值越界是明确错误（不静默钳位）；空输入是空输出。
    #[test]
    fn tape_rejects_out_of_range_rates_and_accepts_empty_input() {
        assert!(matches!(
            tape_varispeed(&[0.0; 10], 0.1),
            Err(SlicerError::RateOutOfRange { .. })
        ));
        assert!(matches!(
            tape_varispeed(&[0.0; 10], f32::NAN),
            Err(SlicerError::RateOutOfRange { .. })
        ));
        assert_eq!(tape_varispeed(&[], 2.0).expect("空输入"), Vec::<f32>::new());
    }

    /// 1 样本的淡入淡出：唯一一个样本归零（不越界、不除零）。
    #[test]
    fn fades_on_a_single_sample_do_not_panic() {
        let mut one = [1.0f32];
        apply_fades(&mut one, 48_000);
        assert_eq!(one[0], 0.0);
        let mut none: [f32; 0] = [];
        apply_fades(&mut none, 48_000);
    }
}
