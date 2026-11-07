//! 静音感知分块：渲染 chunk 边界规划与分块渲染（P1-2，vocaledit Phase 2 第 5 项）。
//!
//! 本模块实现跟进清单 P1-2 与开发计划 §3 第 5 项的分块原则：长音频渲染
//! 分块时，块边界只落在检测到的静音段内，绝不在浊音核中间切；无合规静音
//! 时宁可不断尾（尾段单 chunk 扛到底），也不任意切浊音。
//!
//! * **静音分类**：与 Phase 1 `vocal_timemap` 同算法——RMS 20 ms 窗／
//!   10 ms 跳，阈值复用 Phase 1 公开常量 [`SILENCE_DB`]，持续 ≥
//!   [`SILENCE_MIN_S`] 的低能量段为静音，其余为换气。`rms_curve` 与
//!   `gap_segments` 在 Phase 1 里是私有的，本模块按同一算法重写，
//!   不改 Phase 1 文件（Phase 1 diff 为 0 行）。
//! * **规划器**：[`plan_chunks`] 游走切分：剩余超过目标块长时，找第一
//!   个中点落在 `[cursor + min_chunk_s, end − min_chunk_s]` 内的可切
//!   静音段（段长 ≥ `min_cut_silence_s`），在段中点（hop 对齐）切一刀；
//!   找不到合规静音即停。
//! * **渲染器**：[`render_chunked`] 每 chunk 调一次调用方闭包（闭包内
//!   每 chunk 新建渲染器实例，保证状态新鲜），拼接输出；每 chunk 输出
//!   长度须等于输入长度，否则报 [`ChunkError`]。
//!   [`render_chunked_with_lead_in`] 是交叠变体：每 chunk 从切点减
//!   lead_in 起渲染，拼接时丢弃 lead_in 段输出（lead_in 取自同一静音
//!   段内，见 [`ChunkCut::lead_in_samples`]）。
//! * **收据**：[`ChunkPlan`] 含每刀的采样下标、每刀所落的静音段起止、
//!   每 chunk 起止与长度、配置快照。
//!
//! # 离线限定
//!
//! 本模块只用于离线渲染，绝不进入实时回调（与 `vocal_timemap` 同款
//! 离线限定）：规划与渲染均分配输出与收据。
//!
//! # Clean-room
//!
//! 本模块只按跟进清单 P1-2 的算法描述重写（分块目标 5 秒、最小 3 秒、
//! 只在检测到的静音内切块、hop 对齐、无合规静音时不断尾），未打开、
//! 未对照、未粘贴任何 OpenTune 代码。OpenTune 本体 AGPL-3.0，禁入
//! MIT 仓库。
//!
//! # 等价性说明
//!
//! 切块渲染与整行渲染是否逐位相同，取决于渲染器状态：分块后每 chunk
//! 用新实例渲染，实例状态在静音段内是否收敛到整行实例的状态，须实测
//! 判定，不得先验断言。等价性实测（硬切与 lead-in 交叠两种方案、合成
//! 信号与 pilot 行）见 `~/workspace/vocaledit-phase2/REPORT-p1-2.md`，
//! 结论只写实测。

use crate::vocal_timemap::{SILENCE_DB, SILENCE_MIN_S};
use std::error::Error;
use std::fmt;

/// RMS hop（秒）：10 ms，与 Phase 1 RMS 跳一致；hop 采样数由采样率推导。
pub const HOP_S: f64 = 0.010;
/// 默认分块目标块长（秒）：OpenTune 分块目标。
pub const DEFAULT_TARGET_CHUNK_S: f64 = 5.0;
/// 默认最小块长（秒）：OpenTune 最小块长。
pub const DEFAULT_MIN_CHUNK_S: f64 = 3.0;
/// 默认可切静音段的最小长度（秒）：保证切点能落在段内且离两端 ≥ 1 hop。
pub const DEFAULT_MIN_CUT_SILENCE_S: f64 = 0.100;
/// 默认 lead-in 交叠长度（秒）：取自切点所在的同一静音段内。
pub const DEFAULT_LEAD_IN_S: f64 = 0.050;

/// 静音感知分块配置：全部阈值与开关均可配置，默认值钉死（见各字段）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkPlanConfig {
    /// 分块目标块长（秒）。默认 `5.0`。
    pub target_chunk_s: f64,
    /// 最小块长（秒）：切后每段不得短于此。默认 `3.0`。
    pub min_chunk_s: f64,
    /// 静音判定分贝（RMS 低于此即低能量）。默认 [`SILENCE_DB`]。
    pub silence_db: f64,
    /// 静音最短持续（秒）：短于此的低能量段按换气处理，不按静音吸收。
    /// 默认 [`SILENCE_MIN_S`]。
    pub min_silence_s: f64,
    /// 可切静音段的最小长度（秒）：只有段长 ≥ 此值的静音段才能落刀。
    /// 默认 `0.100`。
    pub min_cut_silence_s: f64,
    /// lead-in 交叠长度（秒）：交叠渲染时每 chunk 从切点减此值起渲染。
    /// 默认 `0.050`。
    pub lead_in_s: f64,
}

impl Default for ChunkPlanConfig {
    fn default() -> Self {
        Self {
            target_chunk_s: DEFAULT_TARGET_CHUNK_S,
            min_chunk_s: DEFAULT_MIN_CHUNK_S,
            silence_db: SILENCE_DB,
            min_silence_s: SILENCE_MIN_S,
            min_cut_silence_s: DEFAULT_MIN_CUT_SILENCE_S,
            lead_in_s: DEFAULT_LEAD_IN_S,
        }
    }
}

impl ChunkPlanConfig {
    /// 校验全部字段。不合法时 panic（构造期错误，不应进入渲染）。
    ///
    /// # Panics
    ///
    /// `target_chunk_s <= 0` 或非有限、`min_chunk_s <= 0` 或非有限、
    /// `min_chunk_s > target_chunk_s`、`silence_db` 非有限或 `>= 0`、
    /// `min_silence_s <= 0` 或非有限、`min_cut_silence_s < hop`
    /// （0.010 s）或非有限、`lead_in_s < 0` 或非有限或
    /// `> min_cut_silence_s` 时 panic。
    pub fn validate(&self) {
        assert!(
            self.target_chunk_s.is_finite() && self.target_chunk_s > 0.0,
            "target_chunk_s {} must be finite and > 0",
            self.target_chunk_s
        );
        assert!(
            self.min_chunk_s.is_finite() && self.min_chunk_s > 0.0,
            "min_chunk_s {} must be finite and > 0",
            self.min_chunk_s
        );
        assert!(
            self.min_chunk_s <= self.target_chunk_s,
            "min_chunk_s {} exceeds target_chunk_s {}",
            self.min_chunk_s,
            self.target_chunk_s
        );
        assert!(
            self.silence_db.is_finite() && self.silence_db < 0.0,
            "silence_db {} must be finite and < 0",
            self.silence_db
        );
        assert!(
            self.min_silence_s.is_finite() && self.min_silence_s > 0.0,
            "min_silence_s {} must be finite and > 0",
            self.min_silence_s
        );
        assert!(
            self.min_cut_silence_s.is_finite() && self.min_cut_silence_s >= HOP_S,
            "min_cut_silence_s {} must be finite and >= hop {} s",
            self.min_cut_silence_s,
            HOP_S
        );
        assert!(
            self.lead_in_s.is_finite()
                && self.lead_in_s >= 0.0
                && self.lead_in_s <= self.min_cut_silence_s,
            "lead_in_s {} must be finite and in [0, min_cut_silence_s {}]",
            self.lead_in_s,
            self.min_cut_silence_s
        );
    }
}

/// 一段检测到的静音（采样下标，左闭右开）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilenceSegment {
    /// 静音段起点（采样下标）。
    pub start: usize,
    /// 静音段终点（采样下标，左闭右开）。
    pub end: usize,
}

impl SilenceSegment {
    /// 段长（采样数）。
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// 段是否为空。
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// 段长（秒）。
    pub fn len_s(&self, sample_rate: u32) -> f64 {
        self.len() as f64 / sample_rate as f64
    }

    /// 采样下标是否落在段内。
    pub fn covers(&self, sample: usize) -> bool {
        sample >= self.start && sample < self.end
    }
}

/// 一刀切块点：切点采样下标＋所落的静音段＋交叠用的 lead-in。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkCut {
    /// 切点采样下标（hop 对齐，落在静音段内）。
    pub at: usize,
    /// 切点所落静音段的起点（采样下标）。
    pub silence_start: usize,
    /// 切点所落静音段的终点（采样下标，左闭右开）。
    pub silence_end: usize,
    /// 交叠渲染时下一 chunk 从切点减此采样数起渲染（已钳制到
    /// `at - silence_start` 以内，落在同一静音段内）。
    pub lead_in_samples: usize,
}

/// 一个 chunk 的起止（采样下标，左闭右开）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSpan {
    /// chunk 起点（采样下标）。
    pub start: usize,
    /// chunk 终点（采样下标，左闭右开）。
    pub end: usize,
}

impl ChunkSpan {
    /// chunk 长度（采样数）。
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// chunk 是否为空。
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// chunk 长度（秒）。
    pub fn len_s(&self, sample_rate: u32) -> f64 {
        self.len() as f64 / sample_rate as f64
    }
}

/// 分块规划收据：刀、静音段、chunk 与配置快照。
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkPlan {
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 输入总采样数。
    pub total_samples: usize,
    /// hop 采样数（10 ms）。
    pub hop_samples: usize,
    /// 全部检测到的静音段（按时间递增）。
    pub silences: Vec<SilenceSegment>,
    /// 全部切块点（按时间递增）。
    pub cuts: Vec<ChunkCut>,
    /// 全部 chunk 起止（按时间递增，覆盖 `[0, total_samples)`）。
    pub chunks: Vec<ChunkSpan>,
    /// 规划所用的配置快照。
    pub config: ChunkPlanConfig,
}

impl ChunkPlan {
    /// 刀数。
    pub fn cut_count(&self) -> usize {
        self.cuts.len()
    }

    /// 输入总长（秒）。
    pub fn total_s(&self) -> f64 {
        self.total_samples as f64 / self.sample_rate as f64
    }
}

/// hop 采样数（10 ms），与 Phase 1 RMS 跳同口径。
fn hop_samples(sample_rate: u32) -> usize {
    (sample_rate as usize * 10 / 1000).max(1)
}

/// 输入 PCM 的 RMS 曲线（20 ms 窗、10 ms 跳），与 Phase 1
/// `vocal_timemap::rms_curve` 同算法（该函数是私有的，此处重写）。
fn rms_curve(source: &[f32], sample_rate: u32) -> Vec<f32> {
    let win = (sample_rate as usize * 20 / 1000).max(1);
    let hop = hop_samples(sample_rate);
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
        curve.push(mean.sqrt() as f32);
        start += hop;
    }
    curve
}

/// 由 RMS 曲线推导每 shifter chunk 的浊音标志：chunk 内 RMS 帧均值
/// ≥ 阈值（`silence_db` 换算为线性）即浊音。
///
/// * `chunk_samples`：shifter 的 chunk 长度（采样数，与
///   [`crate::vocal_pitch_shift::DEFAULT_CHUNK_SAMPLES`] 同口径时，
///   48 kHz 下每 chunk 恰含一帧 RMS，标志与整行逐 chunk 标志逐位对齐）。
/// * 帧起落在 chunk 内的 RMS 帧参与均值；chunk 内无帧时退回 chunk 采样
///   直算 RMS（只在 `chunk_samples < hop` 时发生）。
pub fn rms_voiced_flags(
    samples: &[f32],
    sample_rate: u32,
    chunk_samples: usize,
    silence_db: f64,
) -> Vec<bool> {
    assert!(chunk_samples > 0, "chunk_samples must be > 0");
    assert!(
        sample_rate >= crate::MIN_SAMPLE_RATE as u32,
        "sample_rate {sample_rate} is below {}",
        crate::MIN_SAMPLE_RATE
    );
    let rms = rms_curve(samples, sample_rate);
    let hop = hop_samples(sample_rate);
    let threshold = 10_f32.powf((silence_db / 20.0) as f32);
    let n_chunks = samples.len().div_ceil(chunk_samples);
    let mut flags = Vec::with_capacity(n_chunks);
    for c in 0..n_chunks {
        let start = c * chunk_samples;
        let end = ((c + 1) * chunk_samples).min(samples.len());
        let first_frame = start.div_ceil(hop);
        let last_frame = end.div_ceil(hop).min(rms.len());
        let mut sum = 0.0_f64;
        let mut count = 0usize;
        for value in &rms[first_frame..last_frame] {
            sum += f64::from(*value);
            count += 1;
        }
        let voiced = if count > 0 {
            (sum / count as f64) as f32 >= threshold
        } else {
            // chunk 内无 RMS 帧：直算 chunk 采样 RMS。
            let mut acc = 0.0_f64;
            for sample in &samples[start..end] {
                let value = f64::from(*sample);
                acc += value * value;
            }
            let mean = acc / (end - start).max(1) as f64;
            (mean.sqrt() as f32) >= threshold
        };
        flags.push(voiced);
    }
    flags
}

/// 检测全部静音段（与 Phase 1 `gap_segments` 同算法：持续超过
/// `min_silence_s` 的低能量帧串为静音，其余为换气）。
fn find_silences(
    samples: &[f32],
    sample_rate: u32,
    config: &ChunkPlanConfig,
) -> Vec<SilenceSegment> {
    let rms = rms_curve(samples, sample_rate);
    let hop = hop_samples(sample_rate);
    let hop_s = hop as f64 / sample_rate as f64;
    let threshold = 10_f32.powf((config.silence_db / 20.0) as f32);
    let min_run = (config.min_silence_s / hop_s).ceil() as usize;
    let mut silences = Vec::new();
    let mut idx = 0;
    while idx < rms.len() {
        if rms[idx] < threshold {
            let run_start = idx;
            while idx < rms.len() && rms[idx] < threshold {
                idx += 1;
            }
            if idx - run_start >= min_run {
                let start = run_start * hop;
                let end = (idx * hop).min(samples.len());
                if end > start {
                    silences.push(SilenceSegment { start, end });
                }
            }
        } else {
            idx += 1;
        }
    }
    silences
}

/// 规划分块：算出静音段，游走切分。
///
/// * 空输入 → 零刀空计划（无 chunk）。
/// * 短于目标的输入 → 单 chunk 零刀。
/// * 剩余超过 `target_chunk_s` 时，找第一个中点 ≥ `cursor +
///   min_chunk_s` 且 ≤ `end − min_chunk_s` 的可切静音段（段长 ≥
///   `min_cut_silence_s`），在该段中点（hop 对齐）切一刀；找不到
///   合规静音则停（尾段单 chunk 扛到底，绝不在浊音核中间切）。
///
/// # Panics
///
/// 配置不合法（见 [`ChunkPlanConfig::validate`]）或采样率低于
/// [`crate::MIN_SAMPLE_RATE`] 时 panic。
pub fn plan_chunks(samples: &[f32], sample_rate: u32, config: ChunkPlanConfig) -> ChunkPlan {
    config.validate();
    assert!(
        sample_rate >= crate::MIN_SAMPLE_RATE as u32,
        "sample_rate {sample_rate} is below {}",
        crate::MIN_SAMPLE_RATE
    );
    let hop = hop_samples(sample_rate);
    let n = samples.len();
    let silences = find_silences(samples, sample_rate, &config);
    let target = (config.target_chunk_s * sample_rate as f64).round() as usize;
    let min_chunk = (config.min_chunk_s * sample_rate as f64).ceil() as usize;
    let min_cut = (config.min_cut_silence_s * sample_rate as f64).ceil() as usize;
    let lead_in = (config.lead_in_s * sample_rate as f64).round() as usize;
    let mut cuts: Vec<ChunkCut> = Vec::new();
    let mut cursor = 0usize;
    while n.saturating_sub(cursor) > target {
        let mut found: Option<ChunkCut> = None;
        for seg in &silences {
            if seg.end <= cursor || seg.len() < min_cut {
                continue;
            }
            let mid = (seg.start + seg.end) / 2;
            let mut at = mid / hop * hop;
            if at < seg.start {
                at = seg.start.div_ceil(hop) * hop;
            }
            if at >= seg.end {
                continue;
            }
            if at >= cursor + min_chunk && at <= n.saturating_sub(min_chunk) {
                let lead_in_samples = lead_in.min(at.saturating_sub(seg.start));
                found = Some(ChunkCut {
                    at,
                    silence_start: seg.start,
                    silence_end: seg.end,
                    lead_in_samples,
                });
                break;
            }
        }
        match found {
            Some(cut) => {
                cursor = cut.at;
                cuts.push(cut);
            }
            None => break,
        }
    }
    let mut chunks: Vec<ChunkSpan> = Vec::new();
    if n > 0 {
        let mut start = 0usize;
        for cut in &cuts {
            chunks.push(ChunkSpan { start, end: cut.at });
            start = cut.at;
        }
        chunks.push(ChunkSpan { start, end: n });
    }
    ChunkPlan {
        sample_rate,
        total_samples: n,
        hop_samples: hop,
        silences,
        cuts,
        chunks,
        config,
    }
}

/// 分块渲染错误（按 `vocal_pitch_route::RouteError` 同款风格）。
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkError {
    /// 输入与规划不匹配（长度或采样率对不上规划）。
    InvalidInput(String),
    /// 某 chunk 的渲染输出长度不等于输入长度。
    LengthMismatch {
        /// 出错的 chunk 序号（从 0 起）。
        chunk_index: usize,
        /// 期望的输出长度（采样数，等于输入 chunk 长度）。
        expected: usize,
        /// 实际的输出长度（采样数）。
        got: usize,
    },
}

impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkError::InvalidInput(msg) => write!(f, "invalid chunked input: {msg}"),
            ChunkError::LengthMismatch {
                chunk_index,
                expected,
                got,
            } => write!(
                f,
                "chunk {chunk_index} rendered {got} samples, expected {expected}"
            ),
        }
    }
}

impl Error for ChunkError {}

/// 分块渲染（硬切）：每 chunk 调一次 `render` 闭包，拼接输出。
///
/// * `render(start_sample, chunk_samples)`：调用方闭包，须对收到的
///   chunk 返回等长输出；闭包内由调用方每 chunk 新建渲染器实例，
///   保证状态新鲜。`start_sample` 是 chunk 的绝对起始下标，便于调
///   用方按段切 voiced 标志等伴生数组。
/// * 每 chunk 输出长度须等于输入长度，否则返回
///   [`ChunkError::LengthMismatch`]；输入长度与规划不符时返回
///   [`ChunkError::InvalidInput`]。
///
/// 本函数只用于离线渲染（见模块文档）。
pub fn render_chunked<F>(
    samples: &[f32],
    plan: &ChunkPlan,
    mut render: F,
) -> Result<Vec<f32>, ChunkError>
where
    F: FnMut(usize, &[f32]) -> Vec<f32>,
{
    render_chunked_inner(samples, plan, &mut render, false)
}

/// 分块渲染（lead-in 交叠）：每 chunk 从切点减 lead_in 起渲染，拼接
/// 时丢弃 lead_in 段输出（lead_in 取自同一静音段内，见
/// [`ChunkCut::lead_in_samples`]；首 chunk 无 lead-in）。
///
/// 其余契约与 [`render_chunked`] 相同：闭包须返回等长输出（等长按
/// 实际渲染的 slice 计，含 lead-in 段），否则返回 [`ChunkError`]。
///
/// 本函数只用于离线渲染（见模块文档）。
pub fn render_chunked_with_lead_in<F>(
    samples: &[f32],
    plan: &ChunkPlan,
    mut render: F,
) -> Result<Vec<f32>, ChunkError>
where
    F: FnMut(usize, &[f32]) -> Vec<f32>,
{
    render_chunked_inner(samples, plan, &mut render, true)
}

fn render_chunked_inner<F>(
    samples: &[f32],
    plan: &ChunkPlan,
    render: &mut F,
    lead_in: bool,
) -> Result<Vec<f32>, ChunkError>
where
    F: FnMut(usize, &[f32]) -> Vec<f32>,
{
    if samples.len() != plan.total_samples {
        return Err(ChunkError::InvalidInput(format!(
            "samples len {} does not match plan total {}",
            samples.len(),
            plan.total_samples
        )));
    }
    let mut out: Vec<f32> = Vec::with_capacity(samples.len());
    for (index, chunk) in plan.chunks.iter().enumerate() {
        let skip = if lead_in && index > 0 {
            plan.cuts[index - 1].lead_in_samples
        } else {
            0
        };
        let render_start = chunk.start - skip;
        let slice = &samples[render_start..chunk.end];
        let rendered = render(render_start, slice);
        if rendered.len() != slice.len() {
            return Err(ChunkError::LengthMismatch {
                chunk_index: index,
                expected: slice.len(),
                got: rendered.len(),
            });
        }
        out.extend_from_slice(&rendered[skip..]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocal_pitch_shift::{DEFAULT_CHUNK_SAMPLES, PitchShiftConfig, PitchShifter};
    use std::f32::consts::PI;

    const SR: u32 = 48_000;
    const SR_F: f32 = 48_000.0;

    /// 合成信号：按 `(时长秒, 是否浊音)` 段表拼接；浊音段为相位连续的
    /// 440 Hz 纯音（幅值 0.5），静音段为精确零。
    fn synth(segments: &[(f64, bool)]) -> Vec<f32> {
        let mut out = Vec::new();
        let mut phase = 0.0f32;
        for (dur_s, voiced) in segments {
            let len = (*dur_s * f64::from(SR)).round() as usize;
            for _ in 0..len {
                if *voiced {
                    out.push(0.5 * (phase).sin());
                    phase += 2.0 * PI * 440.0 / SR_F;
                    if phase >= 2.0 * PI {
                        phase -= 2.0 * PI;
                    }
                } else {
                    out.push(0.0);
                }
            }
        }
        out
    }

    /// 区间内最大绝对值（独立于规划器的原始采样能量复核）。
    fn max_abs(samples: &[f32], start: usize, end: usize) -> f32 {
        let mut peak = 0.0f32;
        for sample in &samples[start..end] {
            let value = sample.abs();
            if value > peak {
                peak = value;
            }
        }
        peak
    }

    /// 浊音–静音–浊音交替的长信号：4 s 音／0.5 s 静音 ×3＋2.5 s 音，共 16 s。
    fn alternating() -> Vec<f32> {
        synth(&[
            (4.0, true),
            (0.5, false),
            (4.0, true),
            (0.5, false),
            (4.0, true),
            (0.5, false),
            (2.5, true),
        ])
    }

    #[test]
    fn cuts_land_in_silence() {
        let samples = alternating();
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        assert!(plan.cut_count() >= 1, "expected cuts, got none");
        let min_chunk = (3.0 * f64::from(SR)).ceil() as usize;
        for cut in &plan.cuts {
            // 切点须落在某段检测到的静音内。
            let mut inside = false;
            for seg in &plan.silences {
                if seg.covers(cut.at) {
                    inside = true;
                    assert_eq!(cut.silence_start, seg.start);
                    assert_eq!(cut.silence_end, seg.end);
                }
            }
            assert!(inside, "cut at {} is not inside any silence", cut.at);
            // 原始采样能量独立复核：切点前后 ±5 ms 须为精确静音。
            let lo = cut.at.saturating_sub(240);
            let hi = (cut.at + 240).min(samples.len());
            assert_eq!(
                max_abs(&samples, lo, hi),
                0.0,
                "cut at {} is not in raw silence",
                cut.at
            );
        }
        for chunk in &plan.chunks {
            assert!(
                chunk.len() >= min_chunk,
                "chunk {}..{} shorter than min_chunk_s",
                chunk.start,
                chunk.end
            );
        }
        // chunk 首尾衔接、无间隙、无重叠。
        assert_eq!(plan.chunks[0].start, 0);
        assert_eq!(plan.chunks[plan.chunks.len() - 1].end, samples.len());
        for pair in plan.chunks.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn no_cut_without_silence() {
        let samples = synth(&[(12.0, true)]);
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        assert_eq!(plan.cut_count(), 0);
        assert_eq!(plan.chunks.len(), 1);
        assert_eq!(plan.chunks[0].start, 0);
        assert_eq!(plan.chunks[0].end, samples.len());
    }

    #[test]
    fn breath_not_a_cut_point() {
        // 20 ms 低能量 dip（短于 SILENCE_MIN_S，属换气）＋0.5 s 真静音。
        let samples = synth(&[
            (4.0, true),
            (0.020, false),
            (4.0, true),
            (0.5, false),
            (4.0, true),
        ]);
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        assert_eq!(plan.cut_count(), 1, "expected exactly one cut");
        let cut = plan.cuts[0];
        // 刀只落在真静音段（约 8.02–8.52 s），不在 4.0 s 处的换气 dip 内。
        let dip_start = (4.0 * f64::from(SR)).round() as usize;
        let dip_end = ((4.02) * f64::from(SR)).round() as usize;
        assert!(
            cut.at < dip_start || cut.at >= dip_end,
            "cut at {} landed in the breath dip",
            cut.at
        );
        assert!(
            cut.at > dip_end,
            "cut at {} should be in the true silence after the dip",
            cut.at
        );
        // 换气 dip 不得被登记为静音段。
        for seg in &plan.silences {
            assert!(
                seg.end <= dip_start || seg.start >= dip_end,
                "breath dip registered as silence {seg:?}"
            );
        }
    }

    /// 两路输出的差异分区：切点静音段内 vs 段外；返回（段外数，段内数）。
    fn partition_diffs(whole: &[f32], chunked: &[f32], plan: &ChunkPlan) -> (usize, usize) {
        assert_eq!(whole.len(), chunked.len());
        let mut outside = 0usize;
        let mut inside = 0usize;
        for i in 0..whole.len() {
            if whole[i] != chunked[i] {
                let mut in_cut_silence = false;
                for cut in &plan.cuts {
                    if i >= cut.silence_start && i < cut.silence_end {
                        in_cut_silence = true;
                    }
                }
                if in_cut_silence {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        (outside, inside)
    }

    /// 真实 PitchShifter（+50 音分）整行渲染：单实例＋RMS 浊音标志。
    fn render_whole(samples: &[f32]) -> Vec<f32> {
        let mut shifter = PitchShifter::new(
            SR_F,
            PitchShiftConfig {
                shift_cents: 50.0,
                ..Default::default()
            },
        );
        let flags = rms_voiced_flags(samples, SR, DEFAULT_CHUNK_SAMPLES, SILENCE_DB);
        let (out, _) = shifter.process(samples, &flags);
        out
    }

    /// 真实 PitchShifter（+50 音分）切块渲染：每 chunk 新实例。
    fn render_hard_cut(samples: &[f32], plan: &ChunkPlan) -> Vec<f32> {
        render_chunked(samples, plan, |_, slice| {
            let mut shifter = PitchShifter::new(
                SR_F,
                PitchShiftConfig {
                    shift_cents: 50.0,
                    ..Default::default()
                },
            );
            let flags = rms_voiced_flags(slice, SR, DEFAULT_CHUNK_SAMPLES, SILENCE_DB);
            let (out, _) = shifter.process(slice, &flags);
            out
        })
        .expect("hard-cut render must succeed")
    }

    /// 真实 PitchShifter（+50 音分）切块渲染（lead-in 交叠）：每 chunk
    /// 新实例，从切点减 lead_in 起渲染。
    fn render_lead_in_cut(samples: &[f32], plan: &ChunkPlan) -> Vec<f32> {
        render_chunked_with_lead_in(samples, plan, |_, slice| {
            let mut shifter = PitchShifter::new(
                SR_F,
                PitchShiftConfig {
                    shift_cents: 50.0,
                    ..Default::default()
                },
            );
            let flags = rms_voiced_flags(slice, SR, DEFAULT_CHUNK_SAMPLES, SILENCE_DB);
            let (out, _) = shifter.process(slice, &flags);
            out
        })
        .expect("lead-in render must succeed")
    }

    /// 最佳滞后（采样数）与均方误差：窗口内 whole[i] 与 chunked[i+lag]
    /// 的 SSE，lag ∈ [-1000, 1000]。返回（最佳 lag，最佳 SSE／采样，
    /// lag 0 的 SSE／采样）。
    fn best_lag_sse(whole: &[f32], chunked: &[f32], start: usize, len: usize) -> (i64, f64, f64) {
        let mut best_lag = 0i64;
        let mut best_sse = f64::INFINITY;
        let mut sse0 = 0.0f64;
        for lag in -1000i64..=1000i64 {
            let mut sse = 0.0f64;
            for k in 0..len {
                let j = start as i64 + k as i64 + lag;
                let b = if j >= 0 && (j as usize) < chunked.len() {
                    f64::from(chunked[j as usize])
                } else {
                    0.0
                };
                let d = f64::from(whole[start + k]) - b;
                sse += d * d;
            }
            if lag == 0 {
                sse0 = sse;
            }
            if sse < best_sse {
                best_sse = sse;
                best_lag = lag;
            }
        }
        (best_lag, best_sse / len as f64, sse0 / len as f64)
    }

    /// 切块与整行对照（真实 PitchShifter，+50 音分）。
    ///
    /// 注意：本测试**不**断言 brief 原定的“除含切点的 Silence 段外
    /// 逐位相同”——实测否定了该前提（见 REPORT-p1-2.md）：合成信号
    /// 上段外差异 504,160 采样、pilot take 上 2,941,895 采样，硬切与
    /// lead-in 交叠两方案数逐位相同。根因是移调器的读指针漂移（每
    /// 周期误差扩散余量，界为一个周期）在静音段内冻结：整行实例带
    /// 着漂移进浊音恢复，新实例从零起步，两路输出差一个恒定时移
    /// （合成信号上最佳滞后 SSE 比 lag 0 小四个量级以上）。lead-in
    /// 不能修它：lead-in 只让新实例在同一静音段内早 50 ms 起步，
    /// 直通段读指针仍按 1.0 推进，漂移仍为零。
    ///
    /// 本测试钉死实测的等价形态：首刀之前逐位相同；切点静音段内
    /// 逐位相同（合成信号）；首刀之后差异是恒定时移（最佳滞后
    /// 对齐后误差极小）；lead-in 与硬切只差浮点舍入（最大绝对差
    /// 6.0e-8，该移调器上 lead-in 无信号级作用，见报告）。逐位相同
    /// 口径是否须改为此时移口径，由协调员裁决——改移调器（静音处
    /// 重同步读指针）或每 chunk 预热状态均超出 P1-2 范围。
    #[test]
    fn chunked_matches_whole_up_to_time_shift() {
        let samples = alternating();
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        assert!(plan.cut_count() >= 1);
        let whole = render_whole(&samples);
        let chunked = render_hard_cut(&samples, &plan);
        let lead_in = render_lead_in_cut(&samples, &plan);
        assert_eq!(whole.len(), chunked.len());
        assert_eq!(whole.len(), lead_in.len());
        let (outside, inside) = partition_diffs(&whole, &chunked, &plan);
        // 首刀之前状态完全相同：chunk 0 逐位相同。
        let first_cut = plan.cuts[0].at;
        assert_eq!(whole[..first_cut], chunked[..first_cut]);
        // 合成信号上切点静音段内逐位相同（段内差异只作记录，本信号为 0）。
        assert_eq!(inside, 0, "chunked output differs inside cut silences");
        // 首刀之后差异是恒定时移：第三段音内窗口（9.5–10.0 s）最佳
        // 滞后对齐后误差比 lag 0 小三个量级以上（实测约 1.9e-5 对 0.50）。
        let start = (9.5 * f64::from(SR)).round() as usize;
        let len = (0.5 * f64::from(SR)).round() as usize;
        let (lag, best_sse, sse0) = best_lag_sse(&whole, &chunked, start, len);
        assert!(lag.abs() <= 1000, "best lag {lag} outside search range");
        assert!(
            best_sse < 1e-4 || best_sse < 0.001 * sse0,
            "chunked output is not a time shift of whole: best_sse={best_sse:.6} sse0={sse0:.6}"
        );
        // lead-in 与硬切只差浮点舍入（实测最大绝对差 6.0e-8，见报告）：
        // lead-in 的新实例在同一静音段内早 50 ms 起步，读指针绝对
        // 位置相同、f64 累加顺序不同，插值权重末位不同。本移调器上
        // lead-in 无信号级作用。
        let mut lead_max_abs = 0.0f32;
        for i in 0..chunked.len() {
            let d = (lead_in[i] - chunked[i]).abs();
            if d > lead_max_abs {
                lead_max_abs = d;
            }
        }
        assert!(
            lead_max_abs < 1e-6,
            "lead-in render differs from hard cut by {lead_max_abs:.9}, expected float noise only"
        );
        // 段外差异数只作记录（实测 504,160，见 REPORT-p1-2.md）。
        let _ = outside;
    }

    #[test]
    fn tail_not_cut() {
        // 8 s 行、静音只在 7.9 s 处：留尾不足 min_chunk_s，不切。
        let samples = synth(&[(7.9, true), (0.1, false)]);
        assert_eq!(samples.len(), (8.0 * f64::from(SR)).round() as usize);
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        assert_eq!(plan.cut_count(), 0);
        assert_eq!(plan.chunks.len(), 1);
    }

    #[test]
    fn cut_is_hop_aligned() {
        let samples = alternating();
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        assert!(plan.cut_count() >= 1);
        for cut in &plan.cuts {
            assert_eq!(
                cut.at % plan.hop_samples,
                0,
                "cut at {} is not hop-aligned",
                cut.at
            );
        }
    }

    #[test]
    fn short_and_empty_input() {
        // 短输入：单 chunk 零刀。
        let short = synth(&[(2.0, true)]);
        let plan = plan_chunks(&short, SR, ChunkPlanConfig::default());
        assert_eq!(plan.cut_count(), 0);
        assert_eq!(plan.chunks.len(), 1);
        assert_eq!(plan.chunks[0].end, short.len());
        let rendered = render_chunked(&short, &plan, |_, slice| slice.to_vec())
            .expect("short render must succeed");
        assert_eq!(rendered, short);
        // 空输入：零刀空计划空输出。
        let empty: Vec<f32> = Vec::new();
        let plan = plan_chunks(&empty, SR, ChunkPlanConfig::default());
        assert_eq!(plan.cut_count(), 0);
        assert_eq!(plan.chunks.len(), 0);
        let rendered = render_chunked(&empty, &plan, |_, slice| slice.to_vec())
            .expect("empty render must succeed");
        assert_eq!(rendered.len(), 0);
    }

    #[test]
    fn render_length_mismatch_errors() {
        let samples = alternating();
        let plan = plan_chunks(&samples, SR, ChunkPlanConfig::default());
        let err = render_chunked(&samples, &plan, |_, slice| {
            slice[..slice.len() - 1].to_vec()
        })
        .expect_err("short chunk output must error");
        match err {
            ChunkError::LengthMismatch {
                chunk_index,
                expected,
                got,
            } => {
                assert_eq!(chunk_index, 0);
                assert_eq!(expected, plan.chunks[0].len());
                assert_eq!(got, plan.chunks[0].len() - 1);
            }
            other => panic!("unexpected error {other:?}"),
        }
        // 输入长度与规划不符。
        let longer = synth(&[(17.0, true)]);
        let err = render_chunked(&longer, &plan, |_, slice| slice.to_vec())
            .expect_err("mismatched input must error");
        match err {
            ChunkError::InvalidInput(_) => {}
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "target_chunk_s")]
    fn config_validation_target_nonpositive() {
        let config = ChunkPlanConfig {
            target_chunk_s: 0.0,
            ..Default::default()
        };
        config.validate();
    }

    #[test]
    #[should_panic(expected = "min_chunk_s")]
    fn config_validation_min_exceeds_target() {
        let config = ChunkPlanConfig {
            target_chunk_s: 3.0,
            min_chunk_s: 5.0,
            ..Default::default()
        };
        config.validate();
    }

    #[test]
    #[should_panic(expected = "min_cut_silence_s")]
    fn config_validation_min_cut_below_hop() {
        let config = ChunkPlanConfig {
            min_cut_silence_s: 0.005,
            ..Default::default()
        };
        config.validate();
    }

    #[test]
    #[should_panic(expected = "lead_in_s")]
    fn config_validation_lead_in_exceeds_min_cut() {
        let config = ChunkPlanConfig {
            lead_in_s: 0.200,
            ..Default::default()
        };
        config.validate();
    }
}
