//! 本地声学边界精修器（夜半导入侧，离线限定）。
//!
//! 从回执粗起点出发，在 ±80 ms 窗内精修每音节的声学边界（ADR-0067）：
//!
//! * `consonant_start_s`：能量上升沿回溯（RMS 曲线上最后一个低于
//!   1/4 峰值的帧之后一帧）。
//! * `vowel_start_s`（VOP）：300 Hz–3 kHz 谐波带能量上升沿 + 浊音帧
//!   （自相关峰值 > 0.5）的首个交汇帧；不是能量通量峰。
//! * `voiced_end_s`：周期性跌破 + 带能量跌落 14 dB 持续 30 ms 后的
//!   最后一个浊音帧。
//! * `f0_median_hz`：浊音核内自相关 F0 的中位数。
//! * `hnr_db`：自相关峰值估计器 `10·log10(r/(1−r))`（估计值，非临床 HNR）。
//! * `align_confidence`：起音跳变强度 × F0 一致性；< 0.65 只标记人工复核。
//!
//! 全部时间为 take 内秒（与 [`crate::vocal_timemap`] 同一基准）。
//! 帧网格：窗 2048（42.7 ms）、跳 480（10 ms）、帧时间取窗中心。
//! F0 自相关用 FFT 法（功率谱 → 逆变换），非逐滞后直算。

use super::math::fft;
use super::vocal_timemap::GapKind;

/// VOP 搜索半窗（专家 Q28 精修窗：±80 ms）。
pub const REFINE_HALF_WINDOW_S: f64 = 0.080;
/// VOP 判定：带能量需超过本底倍数。
pub const VOP_RISE_RATIO: f32 = 4.0;
/// 浊音判定：归一化自相关峰值阈值。
pub const VOICED_PEAK: f32 = 0.35;

/// 浊音带能量下限（FFT 功率和单位）。归一化自相关峰对近零信号同样
/// 敏感：数字静音帧（带能量 ≤5）常以峰值 0.8–0.98 被判浊音，导致 off
/// 越过静音桥接到下一段有调内容（行 13 隙字尾实测：53.82 后 90 ms 静音
/// 被判浊音，off 从 53.82 拖到 54.28）。50 = 静音上限 10 倍，远低于实测
/// 最弱元音起音帧（≥1000）；比 folk-v3 安静 20 dB 以上的 take 可能出现
/// 软元音漏判（报告里以未锚定可见），届时再按 take 校准。
pub const VOICED_BAND_FLOOR: f64 = 50.0;
/// 收音判定：带能量跌落分贝。
pub const OFF_DROP_DB: f32 = 14.0;
/// 收音判定：跌落持续帧数（3 帧 = 30 ms）。
pub const OFF_SUSTAIN_FRAMES: usize = 3;
/// 人工复核线：置信度低于此只标记，不阻塞。
pub const REVIEW_CONFIDENCE: f64 = 0.65;
/// 谐波带：下限 Hz。
pub const BAND_LO_HZ: f64 = 300.0;
/// 谐波带：上限 Hz。
pub const BAND_HI_HZ: f64 = 3000.0;
/// F0 搜索范围：下限 Hz。
pub const F0_MIN_HZ: f64 = 50.0;
/// F0 搜索范围：上限 Hz。
pub const F0_MAX_HZ: f64 = 1000.0;
/// 静音判定分贝（与 timemap 一致）。
pub const SILENCE_DB: f64 = -55.0;
/// 静音最短持续秒。
pub const SILENCE_MIN_S: f64 = 0.030;
/// 起音窗锁定 35 ms（与 timemap 的 LOCK_VOP_S 一致）。
const LOCK_VOP_S: f64 = 0.035;

/// 一帧的声学量。
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// 帧中心时间（take 内秒）。
    time_s: f64,
    /// 300 Hz–3 kHz 带能量（线性）。
    band_energy: f32,
    /// 自相关 F0（Hz，0 表示无可靠峰值）。
    f0_hz: f32,
    /// 归一化自相关峰值。
    peak: f32,
    /// 是否浊音（`peak > VOICED_PEAK` 且 F0 落在范围内，且带能量 ≥
    /// `VOICED_BAND_FLOOR`）。
    voiced: bool,
}

/// 精修后的音节边界。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefinedNote {
    /// 辅音起点（take 内秒）。
    pub pre_s: f64,
    /// 浊音韵母起点 VOP（take 内秒）。
    pub vop_s: f64,
    /// 浊音结束（take 内秒）。
    pub off_s: f64,
    /// 是否锚定成功（未锚定者调用方必须排除出 timemap 输入，做区间合并）。
    pub anchored: bool,
    /// 对齐置信度（0–1）；< 0.65 只标记人工复核。
    pub confidence: f64,
    /// 浊音核内 F0 中位数（Hz；无浊音核时为 0）。
    pub f0_median_hz: f64,
    /// 自相关峰值估计的 HNR（dB；估计值，非临床 HNR）。
    pub hnr_db: f64,
}

/// 换气/静音段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreathSpan {
    /// 起止（take 内秒）。
    pub start_s: f64,
    /// 起止（take 内秒）。
    pub end_s: f64,
    /// 成分。
    pub kind: GapKind,
    /// 段内平均 RMS（dBFS）。
    pub rms_db: f64,
}

/// 有调派生段（行 14 ad-lib 检出用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdlibSpan {
    /// 起止（take 内秒）。
    pub start_s: f64,
    /// 起止（take 内秒）。
    pub end_s: f64,
    /// 段内 F0 中位数（Hz）。
    pub f0_median_hz: f64,
    /// 段内自相关峰值中位数。
    pub autocorr_peak: f64,
}

/// RMS 曲线（20 ms 窗、10 ms 跳），返回 `(帧起始秒, RMS)`。
fn rms_curve(source: &[f32], sample_rate: u32) -> Vec<(f64, f32)> {
    let win = (sample_rate as usize * 20 / 1000).max(1);
    let hop = (sample_rate as usize * 10 / 1000).max(1);
    let mut curve = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let end = (start + win).min(source.len());
        let mut sum = 0.0_f64;
        for sample in &source[start..end] {
            let value = f64::from(*sample);
            sum += value * value;
        }
        let mean = sum / (end - start).max(1) as f64;
        curve.push((start as f64 / sample_rate as f64, mean.sqrt() as f32));
        start += hop;
    }
    curve
}

/// 汉宁窗系数。
fn hann(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos()))
        .collect()
}

/// 计算 `[start_s, end_s]` 内全部帧的声学量。帧时间取窗中心。
fn analyze_frames(source: &[f32], sample_rate: u32, start_s: f64, end_s: f64) -> Vec<Frame> {
    let win = 2048usize;
    let hop = 480usize;
    let window = hann(win);
    let sr = sample_rate as f64;
    let lo_bin = ((BAND_LO_HZ * win as f64 / sr).floor() as usize).max(1);
    let hi_bin = ((BAND_HI_HZ * win as f64 / sr).ceil() as usize).min(win / 2);
    let min_lag = (sr / F0_MAX_HZ).floor() as usize;
    let max_lag = (sr / F0_MIN_HZ).ceil() as usize;
    let mut frames = Vec::new();
    let mut start = (start_s.max(0.0) * sr) as usize;
    let end = (end_s.max(0.0) * sr) as usize;
    let mut re = vec![0.0_f64; win];
    let mut im = vec![0.0_f64; win];
    while start < end && start < source.len() {
        // 带能量：加窗 FFT 后 300–3000 Hz 功率和。
        for (i, slot) in re.iter_mut().enumerate() {
            let idx = start + i;
            let sample = if idx < source.len() {
                f64::from(source[idx])
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
        // 自相关：功率谱逆变换，归一化后在 F0 滞后范围找峰。
        for i in 0..win {
            let power = re[i] * re[i] + im[i] * im[i];
            re[i] = power;
            im[i] = 0.0;
        }
        fft(&mut re, &mut im, true);
        let scale = re[0];
        let mut best_lag = 0usize;
        let mut best_peak = 0.0_f64;
        if scale > 0.0 {
            let lag_count = max_lag.min(win - 1).saturating_sub(min_lag) + 1;
            for (lag, value) in re.iter().enumerate().skip(min_lag).take(lag_count) {
                let value = *value / scale;
                if value > best_peak {
                    best_peak = value;
                    best_lag = lag;
                }
            }
        }
        // 抛物线插值精修峰位置。
        let mut peak_lag = best_lag as f64;
        if best_lag > min_lag && best_lag + 1 < win && scale > 0.0 {
            let left = re[best_lag - 1] / scale;
            let center = re[best_lag] / scale;
            let right = re[best_lag + 1] / scale;
            let denom = left - 2.0 * center + right;
            if denom.abs() > 1e-12 {
                let shift = 0.5 * (left - right) / denom;
                if shift.abs() <= 1.0 {
                    peak_lag = best_lag as f64 + shift;
                }
            }
        }
        let f0 = if best_peak > 0.0 && peak_lag > 0.0 {
            sr / peak_lag
        } else {
            0.0
        };
        // 浊音判定：峰值 + F0 范围 + 峰滞后不在搜索边界（边界峰多为噪声
        // 或超限 F0 的边缘伪影，如 take 起始零信号帧在最小滞后处的假峰）
        // + 带能量下限（近零信号的归一化自相关峰同样高，无能量门则静音
        // 帧成批判浊音，off 越过静音桥接、VOP 在静音里误触发）。
        let lag_hi = max_lag.min(win - 1);
        let voiced = best_peak as f32 > VOICED_PEAK
            && (F0_MIN_HZ..=F0_MAX_HZ).contains(&f0)
            && best_lag > min_lag
            && best_lag < lag_hi
            && band >= VOICED_BAND_FLOOR;
        frames.push(Frame {
            time_s: (start as f64 + win as f64 / 2.0) / sr,
            band_energy: band as f32,
            f0_hz: if voiced { f0 as f32 } else { 0.0 },
            peak: best_peak as f32,
            voiced,
        });
        start += hop;
    }
    frames
}

/// 中位数（f64 切片，空返回 0）。
fn median(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

/// VOP 候选判定：浊音帧、显著性（带能量 ≥1% 窗内最大，排除数值噪声）、
/// 上升证据三选一（从本底下方越过 4 倍本底线，或单帧比 ≥1.5，或 3 帧
/// 比 ≥2.0）、持续性（后 3 帧带能量中位 ≥ 本帧一半）。本底规则必须是
/// “越过”而非“高于”：窗在上一元音内打开时，窗内所有帧都高于本底线，
/// 无越过则不判（行 13 隙字实测：粗起点早 164 ms，窗开在缝字元音尾内，
/// 无越过规则会把缝尾帧误作隙起音）。持续性区分噪声毛刺与真实起音：
/// 毛刺下一帧即回落，真实起音后带能量持续走高。窗内 10% 分位门已删除：
/// 扩展窗含响亮元音时分位被抬高，会把真实起音推迟约 35 ms（流字实测）。
fn vop_qualifies(frames: &[Frame], idx: usize, floor: f32, window_max: f32) -> bool {
    let frame = &frames[idx];
    if !frame.voiced {
        return false;
    }
    if frame.band_energy < 0.01 * window_max {
        return false;
    }
    let prev = &frames[idx - 1];
    let rise = (frame.band_energy > VOP_RISE_RATIO * floor
        && prev.band_energy <= VOP_RISE_RATIO * floor)
        || (prev.band_energy > 0.0 && frame.band_energy / prev.band_energy >= 1.5)
        || (idx >= 3
            && frames[idx - 3].band_energy > 0.0
            && frame.band_energy / frames[idx - 3].band_energy >= 2.0);
    if !rise {
        return false;
    }
    let mut next_bands: Vec<f32> = frames
        .iter()
        .skip(idx + 1)
        .take(3)
        .map(|frame| frame.band_energy)
        .collect();
    if next_bands.is_empty() {
        return false;
    }
    next_bands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    next_bands[next_bands.len() / 2] >= 0.5 * frame.band_energy
}

/// 在窗内找 VOP，返回帧下标。`closest_to` 为空时取首个合格帧（±80 ms
/// 窗）；非空时取离该时间最近的合格帧（±300 ms 扩展窗：插值粗起点误差
/// 可达 100 ms 以上，首帧可能是上一音节起音）。旧“窗内首个浊音帧”
/// 规则在连唱中误取窗起点，“窗内最低点后首帧”规则误取噪声毛刺，
/// 均已删除。
fn find_vop(
    frames: &[Frame],
    start: f64,
    end: f64,
    floor: f32,
    next_bound: f64,
    closest_to: Option<f64>,
) -> Option<usize> {
    let window: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| {
            frame.time_s >= start && frame.time_s <= end && frame.time_s < next_bound
        })
        .map(|(idx, _)| idx)
        .collect();
    if window.is_empty() {
        return None;
    }
    let mut bands: Vec<f32> = window.iter().map(|idx| frames[*idx].band_energy).collect();
    bands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let window_max = bands.last().copied().unwrap_or(0.0);
    let mut best: Option<usize> = None;
    let mut best_dist = f64::MAX;
    for idx in window {
        if idx == 0 {
            continue;
        }
        if !vop_qualifies(frames, idx, floor, window_max) {
            continue;
        }
        match closest_to {
            None => return Some(idx),
            Some(target) => {
                let dist = (frames[idx].time_s - target).abs();
                if dist < best_dist {
                    best_dist = dist;
                    best = Some(idx);
                }
            }
        }
    }
    best
}

/// 精修一个音节。
///
/// * `coarse_s`：回执粗起点（take 内秒）。
/// * `prev_bound_s`：前一音节 off（pre 的下限钳制）。
/// * `next_bound_s`：下一音节 pre（off 的上限钳制）。
pub fn refine_note(
    source: &[f32],
    sample_rate: u32,
    coarse_s: f64,
    prev_bound_s: f64,
    next_bound_s: f64,
) -> RefinedNote {
    let search_start = coarse_s - REFINE_HALF_WINDOW_S;
    let search_end = coarse_s + REFINE_HALF_WINDOW_S;
    // 行首音节的粗起点常是段起点（辅音/静音起点），真 VOP 可晚 100 ms 以上；
    // 插值粗起点误差也可超 100 ms：±80 ms 内找不到时，向两侧扩展到 ±300 ms
    // 再找一次，取离粗起点最近的合格帧（首帧可能是上一音节起音）。
    // 扩展候选仍须 < next_bound_s（否则是下一音）且 > prev_bound_s。
    let search_end_expanded = coarse_s + 0.300;
    let search_start_expanded = coarse_s - 0.300;
    let floor_start = coarse_s - 0.300;
    let frames = analyze_frames(
        source,
        sample_rate,
        floor_start,
        next_bound_s.max(search_end_expanded),
    );
    let floor_frames: Vec<f32> = frames
        .iter()
        .filter(|frame| frame.time_s >= floor_start && frame.time_s < search_start)
        .map(|frame| frame.band_energy)
        .collect();
    let mut floor_sorted = floor_frames.clone();
    floor_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = if floor_sorted.is_empty() {
        frames
            .iter()
            .map(|frame| frame.band_energy)
            .fold(f32::MAX, f32::min)
            .max(1e-12)
    } else {
        floor_sorted[floor_sorted.len() / 2].max(1e-12)
    };
    let vop_idx = find_vop(
        &frames,
        search_start.max(prev_bound_s),
        search_end,
        floor,
        next_bound_s,
        None,
    )
    .or_else(|| {
        find_vop(
            &frames,
            search_start_expanded.max(prev_bound_s),
            search_end_expanded,
            floor,
            next_bound_s,
            Some(coarse_s),
        )
    });
    let Some(vop_idx) = vop_idx else {
        return RefinedNote {
            pre_s: coarse_s,
            vop_s: coarse_s,
            off_s: coarse_s,
            anchored: false,
            confidence: 0.0,
            f0_median_hz: 0.0,
            hnr_db: -20.0,
        };
    };
    let vop = frames[vop_idx];
    let vop_s = vop.time_s;
    // pre：谐波带能量后向 walk。从 VOP 帧起，先跳过元音帧（带 ≥0.5× 元音
    // 水平），再跳过辅音帧（带 <0.5× 元音水平），停在上一元音帧；pre =
    // 该帧时间 + 10 ms。偏早是安全的（锁窗含一段前元音尾/静音仍按 1.0），
    // 偏晚会丢辅音，故取早界；走到窗起点仍在辅音里时 pre = 窗起点。
    let vowel_band = {
        let mut bands: Vec<f32> = frames
            .iter()
            .filter(|frame| frame.time_s >= vop_s && frame.time_s <= vop_s + 0.100)
            .map(|frame| frame.band_energy)
            .collect();
        bands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if bands.is_empty() {
            vop.band_energy
        } else {
            bands[bands.len() / 2].max(1e-12)
        }
    };
    // pre：谐波带回溯。先跳过元音帧（≥ 0.5× 元音带能量），再跳过辅音
    // 帧（< 阈值但 ≥ 能量下限）；遇上一元音（≥ 阈值）或静音（< 下限）即
    // 停，pre = 该帧 + 10 ms（辅音起点）。静音必须停：否则回溯穿过
    // 静音直达窗起点，把 150 ms 静音谎报成辅音，锁窗白白吃掉输出预算
    // （行 13 缝字实测：pre 落窗起点，核被压到 0.67×）。静音里停不丢辅
    // 音：辅音若在静音前，自在间隙里以换气 1.0 保留；塞音闭塞段本身是
    // 静音，本就不需锁窗保护。
    let pre_window_start = vop_s - 0.150;
    let threshold = 0.5 * vowel_band;
    let mut j = vop_idx;
    while j > 0 && frames[j].time_s > pre_window_start && frames[j].band_energy >= threshold {
        j -= 1;
    }
    while j > 0
        && frames[j].time_s > pre_window_start
        && frames[j].band_energy < threshold
        && frames[j].band_energy >= VOICED_BAND_FLOOR as f32
    {
        j -= 1;
    }
    let mut pre_s =
        if frames[j].band_energy >= threshold || frames[j].band_energy < VOICED_BAND_FLOOR as f32 {
            frames[j].time_s + 0.010
        } else {
            pre_window_start
        };
    pre_s = pre_s.clamp(prev_bound_s, vop_s);
    // off：从 VOP 起，带能量跌落 14 dB 且失浊持续 3 帧后的最后浊音帧。
    let peak_band = frames
        .iter()
        .filter(|frame| frame.time_s >= vop_s && frame.time_s <= (vop_s + 0.500).min(next_bound_s))
        .map(|frame| frame.band_energy)
        .fold(0.0_f32, f32::max)
        .max(1e-12);
    let drop_line = peak_band * 10_f32.powf(-OFF_DROP_DB / 10.0);
    let mut off_s = next_bound_s;
    let mut run = 0usize;
    let mut last_voiced = vop_s;
    for frame in frames
        .iter()
        .filter(|frame| frame.time_s >= vop_s && frame.time_s <= next_bound_s)
    {
        if frame.voiced {
            last_voiced = frame.time_s;
            run = 0;
        } else if frame.band_energy < drop_line {
            run += 1;
            if run >= OFF_SUSTAIN_FRAMES {
                off_s = last_voiced;
                break;
            }
        } else {
            run = 0;
        }
    }
    if off_s == next_bound_s {
        off_s = last_voiced;
    }
    off_s = off_s.clamp(vop_s, next_bound_s);
    // 零长/极短核无法容纳 35 ms 锁窗 + 30 ms 收音尾：记为未锚定，
    // 调用方做区间合并（其音频以 1.0  ride 在间隙里），不伪造边界。
    let anchored = off_s - vop_s >= 0.065;
    // 核内 F0 中位数与 HNR 估计。
    let core_frames: Vec<&Frame> = frames
        .iter()
        .filter(|frame| {
            frame.time_s >= vop_s + LOCK_VOP_S && frame.time_s <= off_s - 0.030 && frame.voiced
        })
        .collect();
    let core_frames = if core_frames.is_empty() {
        frames
            .iter()
            .filter(|frame| frame.time_s >= vop_s && frame.time_s <= off_s && frame.voiced)
            .collect()
    } else {
        core_frames
    };
    let f0_values: Vec<f64> = core_frames
        .iter()
        .map(|frame| f64::from(frame.f0_hz))
        .collect();
    let f0_median = median(f0_values);
    let peak_values: Vec<f64> = core_frames
        .iter()
        .map(|frame| f64::from(frame.peak))
        .collect();
    let peak_median = median(peak_values).clamp(0.01, 0.99);
    let hnr_db = 10.0 * (peak_median / (1.0 - peak_median)).log10();
    // 置信度：起音跳变强度 × F0 一致性。跳变本底取 [coarse−300 ms,
    // coarse+80 ms] 内最低 3 帧带能量的中位数（连唱时即辅音水平，不是
    // 上一元音水平）；一致性容差 ±100 音分（颤音容忍，仍拒八度错）。
    let mut context_bands: Vec<f32> = frames
        .iter()
        .filter(|frame| frame.time_s >= floor_start && frame.time_s <= search_end)
        .map(|frame| frame.band_energy)
        .collect();
    context_bands.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let local_floor = if context_bands.len() >= 3 {
        context_bands[1].max(1e-12)
    } else {
        floor
    };
    let jump_db = 10.0 * (f64::from(vop.band_energy) / f64::from(local_floor)).log10();
    let onset_strength = ((jump_db - 3.0) / 17.0).clamp(0.0, 1.0);
    let consistent = if f0_median > 0.0 {
        let total = core_frames.len();
        let good = core_frames
            .iter()
            .filter(|frame| {
                let cents = 1200.0 * (f64::from(frame.f0_hz) / f0_median).log2();
                cents.abs() <= 100.0
            })
            .count();
        if total == 0 {
            0.0
        } else {
            good as f64 / total as f64
        }
    } else {
        0.0
    };
    RefinedNote {
        pre_s,
        vop_s,
        off_s,
        anchored,
        confidence: onset_strength * consistent,
        f0_median_hz: f0_median,
        hnr_db,
    }
}

/// 对间隙列表做换气/静音分段（RMS 曲线 + 30 ms 持续门限）。
pub fn breath_spans(source: &[f32], sample_rate: u32, gaps: &[(f64, f64)]) -> Vec<BreathSpan> {
    let rms = rms_curve(source, sample_rate);
    let hop_s = 10.0 / 1000.0;
    let threshold = 10_f32.powf((SILENCE_DB / 20.0) as f32);
    let mut spans = Vec::new();
    for (gap_start, gap_end) in gaps {
        if gap_end <= gap_start {
            continue;
        }
        let hop = (sample_rate as usize * 10 / 1000).max(1);
        let start_idx = ((*gap_start * sample_rate as f64) as usize / hop).min(rms.len());
        let end_idx = ((*gap_end * sample_rate as f64) as usize / hop).min(rms.len());
        let min_run = (SILENCE_MIN_S / hop_s).ceil() as usize;
        let mut cursor = *gap_start;
        let mut idx = start_idx;
        while idx < end_idx {
            if rms[idx].1 < threshold {
                let run_start = idx;
                while idx < end_idx && rms[idx].1 < threshold {
                    idx += 1;
                }
                if idx - run_start >= min_run {
                    let seg_start = (run_start as f64 * hop_s).max(cursor);
                    let seg_end = (idx as f64 * hop_s).min(*gap_end);
                    if seg_start > cursor {
                        spans.push(span_with_rms(&rms, cursor, seg_start, GapKind::Breath));
                    }
                    spans.push(span_with_rms(&rms, seg_start, seg_end, GapKind::Silence));
                    cursor = seg_end;
                }
            } else {
                idx += 1;
            }
        }
        if cursor < *gap_end {
            spans.push(span_with_rms(&rms, cursor, *gap_end, GapKind::Breath));
        }
    }
    spans
}

fn span_with_rms(rms: &[(f64, f32)], start_s: f64, end_s: f64, kind: GapKind) -> BreathSpan {
    let values: Vec<f32> = rms
        .iter()
        .filter(|(time, _)| *time >= start_s && *time < end_s)
        .map(|(_, value)| *value)
        .collect();
    let mean = if values.is_empty() {
        0.0
    } else {
        values.iter().map(|value| f64::from(*value)).sum::<f64>() / values.len() as f64
    };
    let rms_db = if mean > 0.0 {
        20.0 * mean.log10()
    } else {
        -120.0
    };
    BreathSpan {
        start_s,
        end_s,
        kind,
        rms_db,
    }
}

/// 在休止窗内检出有调派生段：浊音帧 + 自相关峰值 > 0.75 持续 > 200 ms。
/// 返回段按时间递增；相邻段间隔 < 50 ms 时合并。
pub fn detect_adlib(
    source: &[f32],
    sample_rate: u32,
    rest_windows: &[(f64, f64)],
) -> Vec<AdlibSpan> {
    let mut spans = Vec::new();
    for (win_start, win_end) in rest_windows {
        let frames = analyze_frames(source, sample_rate, *win_start, *win_end);
        let mut run: Vec<&Frame> = Vec::new();
        let flush = |run: &mut Vec<&Frame>, spans: &mut Vec<AdlibSpan>| {
            if run.len() as f64 * 0.010 > 0.200 {
                let f0_values: Vec<f64> = run.iter().map(|frame| f64::from(frame.f0_hz)).collect();
                let peak_values: Vec<f64> = run.iter().map(|frame| f64::from(frame.peak)).collect();
                spans.push(AdlibSpan {
                    start_s: run.first().map(|frame| frame.time_s).unwrap_or(0.0),
                    end_s: run.last().map(|frame| frame.time_s).unwrap_or(0.0),
                    f0_median_hz: median(f0_values),
                    autocorr_peak: median(peak_values),
                });
            }
            run.clear();
        };
        for frame in &frames {
            if frame.voiced && frame.peak > 0.75 {
                run.push(frame);
            } else {
                flush(&mut run, &mut spans);
            }
        }
        flush(&mut run, &mut spans);
    }
    // 合并间隔 < 50 ms 的相邻段。
    let mut merged: Vec<AdlibSpan> = Vec::new();
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.start_s - last.end_s < 0.050
        {
            last.end_s = span.end_s;
            continue;
        }
        merged.push(span);
    }
    merged
}

/// 行浊音占比：锚定音节浊音跨度之和 / 首末锚定音节跨度。
pub fn line_voiced_ratio(notes: &[RefinedNote]) -> f64 {
    let anchored: Vec<&RefinedNote> = notes.iter().filter(|note| note.anchored).collect();
    if anchored.len() < 2 {
        return 0.0;
    }
    let voiced: f64 = anchored
        .iter()
        .map(|note| (note.off_s - note.vop_s).max(0.0))
        .sum();
    let span = anchored.last().map(|note| note.off_s).unwrap_or(0.0)
        - anchored.first().map(|note| note.vop_s).unwrap_or(0.0);
    if span <= 0.0 { 0.0 } else { voiced / span }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成信号：静音 0.5 s + 噪声辅音 0.1 s + 谐波元音 0.8 s + 静音。
    fn synthetic_syllable(sr: u32) -> Vec<f32> {
        let sr_usize = sr as usize;
        let mut signal = vec![0.0_f32; sr_usize * 2];
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next_noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as f32 / u64::MAX as f32) * 2.0 - 1.0
        };
        // 辅音：0.5–0.6 s 白噪声 0.3。
        for sample in signal
            .iter_mut()
            .skip(sr_usize / 2)
            .take(sr_usize * 6 / 10 - sr_usize / 2)
        {
            *sample = next_noise() * 0.3;
        }
        // 元音：0.6–1.4 s，基频 150 Hz 谐波（20 ms 起音）。
        for (i, sample) in signal
            .iter_mut()
            .enumerate()
            .skip(sr_usize * 6 / 10)
            .take(sr_usize * 14 / 10 - sr_usize * 6 / 10)
        {
            let t = (i - sr_usize * 6 / 10) as f64 / sr as f64;
            let attack = (t / 0.020).min(1.0);
            let phase = 2.0 * std::f64::consts::PI * 150.0 * t;
            let mut value = 0.0;
            for harmonic in 1..=5 {
                value += (phase * harmonic as f64).sin() / harmonic as f64;
            }
            *sample = (value * 0.4 * attack) as f32;
        }
        signal
    }

    #[test]
    fn refine_finds_vop_pre_off_and_f0() {
        let sr = 48_000;
        let signal = synthetic_syllable(sr as u32);
        let note = refine_note(&signal, sr as u32, 0.63, 0.0, 2.0);
        assert!(note.anchored, "synthetic vowel must anchor");
        assert!(
            (note.vop_s - 0.60).abs() <= 0.030,
            "VOP {} must be near 0.60",
            note.vop_s
        );
        assert!(
            note.pre_s <= 0.53 && note.pre_s >= 0.44,
            "pre {} must not miss the consonant (true 0.50; early bias is safe)",
            note.pre_s
        );
        assert!(
            (note.off_s - 1.40).abs() <= 0.050,
            "off {} must be near 1.40",
            note.off_s
        );
        assert!(
            (note.f0_median_hz - 150.0).abs() <= 5.0,
            "F0 {} must be near 150",
            note.f0_median_hz
        );
        assert!(
            note.confidence >= 0.65,
            "confidence {} must clear review line",
            note.confidence
        );
    }

    #[test]
    fn refine_legato_fallback_finds_second_vowel() {
        // 连唱：元音 1（150 Hz，0.2–0.6 s）→ 0.2 倍弱谷（0.6–0.65 s）→
        // 元音 2（200 Hz，0.65–1.2 s）。窗前本底是元音 1（主规则失效），
        // 备用上升规则须在 0.65 s 附近找到 VOP，而不是窗起点。
        let sr = 48_000;
        let sr_usize = sr as usize;
        let mut signal = vec![0.0_f32; sr_usize * 2];
        for (i, sample) in signal.iter_mut().enumerate() {
            let t = i as f64 / sr as f64;
            let (freq, amp) = if t < 0.2 {
                (0.0, 0.0)
            } else if t < 0.6 {
                (150.0, 0.4)
            } else if t < 0.65 {
                (200.0, 0.08)
            } else if t < 1.2 {
                (200.0, 0.4)
            } else {
                (0.0, 0.0)
            };
            if freq > 0.0 {
                let phase = 2.0 * std::f64::consts::PI * freq * t;
                let mut value = 0.0;
                for harmonic in 1..=5 {
                    value += (phase * harmonic as f64).sin() / harmonic as f64;
                }
                *sample = (value * amp) as f32;
            }
        }
        let note = refine_note(&signal, sr as u32, 0.68, 0.0, 2.0);
        assert!(note.anchored, "legato second vowel must anchor");
        assert!(
            (note.vop_s - 0.65).abs() <= 0.030,
            "VOP {} must be near 0.65",
            note.vop_s
        );
        assert!(
            (note.f0_median_hz - 200.0).abs() <= 5.0,
            "F0 {} must be near 200",
            note.f0_median_hz
        );
    }

    #[test]
    fn refine_unanchored_on_silence() {
        let sr = 48_000;
        let signal = vec![0.0_f32; sr * 2];
        let note = refine_note(&signal, sr as u32, 1.0, 0.0, 2.0);
        assert!(!note.anchored);
        assert_eq!(note.confidence, 0.0);
    }

    #[test]
    fn refine_tiny_periodic_signal_not_voiced() {
        // 极小幅值纯音：归一化自相关峰 ≈1.0（无能量门会被判浊音），但带
        // 能量远低于 VOICED_BAND_FLOOR，必须未锚定。钉住行 13 隙字尾与
        // 行 5 呼吸段的静音误判修复（带能量 ≤5 的近零帧成批判浊音）。
        let sr = 48_000;
        let signal: Vec<f32> = (0..sr * 2)
            .map(|i| {
                (2.0 * std::f64::consts::PI * 300.0 * i as f64 / sr as f64).sin() as f32 * 0.001
            })
            .collect();
        let note = refine_note(&signal, sr as u32, 1.0, 0.0, 2.0);
        assert!(!note.anchored);
    }

    #[test]
    fn breath_spans_split_silence_and_breath() {
        let sr = 48_000;
        let mut signal = vec![0.0_f32; sr * 2];
        for sample in signal.iter_mut().take(sr).skip(sr / 2) {
            *sample = 0.05;
        }
        let spans = breath_spans(&signal, sr as u32, &[(0.4, 1.2)]);
        let silence: Vec<&BreathSpan> = spans
            .iter()
            .filter(|span| span.kind == GapKind::Silence)
            .collect();
        assert!(!silence.is_empty(), "silence part must be detected");
    }
}
