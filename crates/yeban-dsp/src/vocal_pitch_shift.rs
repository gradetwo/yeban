//! 周期重采样移调器：逐样本类 PSOLA 修音（P1-1，vocaledit Phase 2 第一项）。
//!
//! 本模块实现 Phase 2 渲染链的 DSP 移调腿（跟进清单 P1-1、开发计划 §5）：
//!
//! * **逐样本周期重采样**：输出指针每采样前进 `ratio` 个输入采样；当输入
//!   指针越过一个整周期边界时，插入（重复）或删除（跳过）一个整周期，
//!   使输出时长与输入逐位相同、音高按 `ratio` 移动。跳变量恒为一个整
//!   周期，故跳变前后相位推进不变（输出仍是同频率的周期信号）。
//! * **约 5 采样前视**：周期边界的重复跳变不在越过瞬间执行，而是经
//!   [`PitchShiftConfig::lookahead_samples`] 个输出采样确认后再执行；
//!   删除跳变在边界处立即执行（删的是已判定无用的材料，延迟只会多发
//!   已判死刑的采样）。
//! * **无硬复位**：读指针 [`PitchShifter::in_pos`] 是跨 `process` 调用、
//!   跨浊音／清音边界连续的状态，永不 snap 回写指针。清音段内读指针
//!   以 1.0 推进（地址连续），浊音段恢复时从当前地址续读。
//! * **snap 与检测器 valid 是独立开关**：
//!   [`PitchShiftConfig::snap_to_semitone`] 把移调量吸附到最近半音；
//!   检测器 valid 以每 chunk 的 `voiced` 标志传入，决定该 chunk 走
//!   移调还是直通。两者互不干涉。
//! * **收据**：每次 [`PitchShifter::process`] 返回
//!   [`PitchShiftReceipt`]，每 chunk 标记路由、施加的音分、浊音标志、
//!   所用周期、插／删周期数与过渡淡入淡出。
//!
//! # 离线限定
//!
//! 本模块只用于离线渲染，绝不进入实时回调（与 `vocal_timemap` 同款
//! 离线限定）：`process` 分配输出与收据；周期估计按 chunk 做自相关，
//! 不是逐采样定长预算。
//!
//! # 命名（ADR-0067 §1 教训）
//!
//! 移调量以**音分**（`shift_cents`）为唯一输入口径，内部比率
//! `ratio = 2^(cents/1200)` 由 [`pitch_ratio_from_cents`] 换算；
//! 共振峰保持只用 factor 语义（[`PitchShiftConfig::formant_factor`]），
//! 且本模块不做共振峰修正——只接受 `1.0`，其余值在构造时拒绝。
//! `formant_base` 式调用永不恢复。
//!
//! # Clean-room
//!
//! 本模块只按跟进清单 P1-1 的算法描述重写（逐样本周期重采样、5 采样
//! 前视、无硬复位、snap／valid 开关、收据），未打开、未对照、未粘贴
//! 任何 OpenTune 代码。OpenTune 本体 AGPL-3.0，禁入 MIT 仓库。
//!
//! # 周期估计
//!
//! 每 chunk 用归一化自相关估计周期（F0 范围 50–1000 Hz，与
//! `vocal_refine` 同口径），峰值取抛物线插值亚采样精度。已有上一周期
//! 时先在 −25%/+33% 窄窗内跟踪，跟踪失败再全窗重搜。估计值只在与当前网格
//! 周期相对偏差超过 [`PitchShiftConfig::reanchor_relative_change`]
//! 时才重锚网格（累计偏差口径，非单 chunk 增量），避免抖动。
//!
//! # 局限
//!
//! * 周期估计假设 chunk 内近似稳态周期信号；滑音／颤音每 chunk 重锚，
//!   平均音高仍对，但瞬时相位有约一个周期的量化。
//! * 浊音／清音边界用 [`PitchShiftConfig::transition_fade_s`] 线性
//!   淡入淡出衔接（默认 5 ms）； abrupt 起音＋无淡入淡出时，边界处
//!   仍可能出现约一个周期的相位台阶（见测试
//!   `boundary_no_click_gradual_without_fade` 的对照说明）。
//! * 共振峰不修正（factor 恒 1.0）：大移调量下音色会随音高漂移，
//!   与 P0-1 路由的 ≥75 音分走神经腿的设定一致（大偏差不走本腿）。

use libm::pow;

/// 最大移调量（音分）：±1200 音分＝±一个八度。超出此值构造时拒绝。
pub const MAX_SHIFT_CENTS: f32 = 1200.0;
/// 默认前视（输出采样数）：周期边界重复跳变的确认延迟。
pub const DEFAULT_LOOKAHEAD_SAMPLES: usize = 5;
/// 前视上限（输出采样数）。
pub const MAX_LOOKAHEAD_SAMPLES: usize = 64;
/// 默认 chunk 长度（采样数）：10 ms @ 48 kHz，与 `vocal_refine` 的 hop 同口径。
pub const DEFAULT_CHUNK_SAMPLES: usize = 480;
/// 默认周期估计窗长（采样数）。
pub const DEFAULT_ESTIMATION_WINDOW_SAMPLES: usize = 2048;
/// 周期估计的最短窗长（采样数）：短于此不估计，沿用上一周期。
pub const MIN_ESTIMATION_WINDOW_SAMPLES: usize = 512;
/// 默认自相关峰值下限：低于此视为无周期（按清音处理）。
pub const DEFAULT_MIN_AUTOCORR_PEAK: f32 = 0.5;
/// 默认重锚阈值：新估计周期与网格周期相对偏差超过此值才重锚网格。
pub const DEFAULT_REANCHOR_RELATIVE_CHANGE: f32 = 0.02;
/// 默认浊音／清音过渡淡入淡出时长（秒）：5 ms。
pub const DEFAULT_TRANSITION_FADE_S: f32 = 0.005;
/// 周期估计的 F0 下限（Hz），与 `vocal_refine` 同口径。
pub const MIN_F0_HZ: f32 = 50.0;
/// 周期估计的 F0 上限（Hz），与 `vocal_refine` 同口径。
pub const MAX_F0_HZ: f32 = 1000.0;

/// 音分换算为移调比率：`ratio = 2^(cents/1200)`。
/// `cents > 0` 表示升高（比率大于 1）。
pub fn pitch_ratio_from_cents(cents: f32) -> f64 {
    pow(2.0, f64::from(cents) / 1200.0)
}

/// 移调器配置：全部阈值与开关均可配置，默认值钉死（见各字段）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchShiftConfig {
    /// 移调量（音分）。正值升高，负值降低。默认 `0.0`。
    pub shift_cents: f32,
    /// snap 模式：为真时移调量吸附到最近的半音（100 音分整数倍）。
    /// 默认 `false`。
    pub snap_to_semitone: bool,
    /// 共振峰保持因子。**只接受 `1.0`（不做共振峰修正）**，
    /// 其余值在构造时拒绝。默认 `1.0`。
    pub formant_factor: f32,
    /// 前视（输出采样数）：周期边界重复跳变的确认延迟。默认 `5`。
    pub lookahead_samples: usize,
    /// chunk 长度（采样数）：收据与浊音标志的单位。默认 `480`。
    pub chunk_samples: usize,
    /// 周期估计窗长（采样数）。默认 `2048`。
    pub estimation_window_samples: usize,
    /// 自相关峰值下限：低于此视为无周期。默认 `0.5`。
    pub min_autocorr_peak: f32,
    /// 重锚阈值（相对偏差）。默认 `0.02`。
    pub reanchor_relative_change: f32,
    /// 浊音／清音过渡淡入淡出时长（秒）。`0.0` 表示无淡入淡出。
    /// 默认 `0.005`。
    pub transition_fade_s: f32,
}

impl Default for PitchShiftConfig {
    fn default() -> Self {
        Self {
            shift_cents: 0.0,
            snap_to_semitone: false,
            formant_factor: 1.0,
            lookahead_samples: DEFAULT_LOOKAHEAD_SAMPLES,
            chunk_samples: DEFAULT_CHUNK_SAMPLES,
            estimation_window_samples: DEFAULT_ESTIMATION_WINDOW_SAMPLES,
            min_autocorr_peak: DEFAULT_MIN_AUTOCORR_PEAK,
            reanchor_relative_change: DEFAULT_REANCHOR_RELATIVE_CHANGE,
            transition_fade_s: DEFAULT_TRANSITION_FADE_S,
        }
    }
}

impl PitchShiftConfig {
    /// 实际施加的移调量（音分）：snap 开启时吸附到最近半音。
    pub fn applied_cents(&self) -> f32 {
        if self.snap_to_semitone {
            (self.shift_cents / 100.0).round() * 100.0
        } else {
            self.shift_cents
        }
    }

    /// 校验全部字段。不合法时 panic（构造期错误，不应进入渲染）。
    ///
    /// # Panics
    ///
    /// `|shift_cents| > 1200`、`formant_factor != 1.0`、`lookahead_samples
    /// > 64`、`chunk_samples == 0`、`estimation_window_samples < 512`、
    /// `min_autocorr_peak` 不在 `(0, 1)` 内、`reanchor_relative_change`
    /// 不在 `(0, 0.5]` 内、`transition_fade_s < 0` 时 panic。
    pub fn validate(&self) {
        assert!(
            self.shift_cents.abs() <= MAX_SHIFT_CENTS,
            "shift_cents |{}| exceeds +/-{} cents",
            self.shift_cents,
            MAX_SHIFT_CENTS
        );
        assert!(
            self.formant_factor == 1.0,
            "formant correction is not implemented: formant_factor must be 1.0, got {}",
            self.formant_factor
        );
        assert!(
            self.lookahead_samples <= MAX_LOOKAHEAD_SAMPLES,
            "lookahead_samples {} exceeds {}",
            self.lookahead_samples,
            MAX_LOOKAHEAD_SAMPLES
        );
        assert!(self.chunk_samples > 0, "chunk_samples must be > 0");
        assert!(
            self.estimation_window_samples >= MIN_ESTIMATION_WINDOW_SAMPLES,
            "estimation_window_samples {} is below {}",
            self.estimation_window_samples,
            MIN_ESTIMATION_WINDOW_SAMPLES
        );
        assert!(
            self.min_autocorr_peak > 0.0 && self.min_autocorr_peak < 1.0,
            "min_autocorr_peak {} must be in (0, 1)",
            self.min_autocorr_peak
        );
        assert!(
            self.reanchor_relative_change > 0.0 && self.reanchor_relative_change <= 0.5,
            "reanchor_relative_change {} must be in (0, 0.5]",
            self.reanchor_relative_change
        );
        assert!(
            self.transition_fade_s >= 0.0,
            "transition_fade_s {} must be >= 0",
            self.transition_fade_s
        );
    }
}

/// 一个 chunk 的渲染路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkRoute {
    /// 本 chunk 走了 DSP 移调（浊音且周期已估计）。
    DspShift,
    /// 本 chunk 直通（移调量为 0、清音、或周期估计失败）。
    Passthrough,
}

/// 一个 chunk 的收据：路由与偏差信息逐 chunk 标记。
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkReceipt {
    /// chunk 序号（从 0 起）。
    pub chunk_index: usize,
    /// 本 chunk 的路由。
    pub route: ChunkRoute,
    /// 本 chunk 实际施加的移调量（音分，snap 后）。
    pub cents_applied: f32,
    /// 检测器 valid 标志（调用方传入的浊音判定）。
    pub voiced: bool,
    /// 本 chunk 移调所用的周期（采样数）；直通 chunk 为 `None`。
    pub period_samples: Option<f32>,
    /// 本 chunk 内插入（重复）的周期数。
    pub periods_inserted: u32,
    /// 本 chunk 内删除（跳过）的周期数。
    pub periods_deleted: u32,
    /// 本 chunk 内是否启动了浊音／清音过渡淡入淡出。
    pub transition_fade: bool,
}

/// 一次 `process` 调用的完整收据。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PitchShiftReceipt {
    /// 逐 chunk 收据（顺序与输入 chunk 一致）。
    pub chunks: Vec<ChunkReceipt>,
    /// 本次调用插入（重复）的周期总数。
    pub periods_inserted: u32,
    /// 本次调用删除（跳过）的周期总数。
    pub periods_deleted: u32,
}

/// 滞后 `lag` 处的归一化自相关值（能量归一化，`energy` 为窗内总能量）。
fn autocorr_at(samples: &[f32], energy: f32, lag: usize) -> f32 {
    if energy <= 0.0 || lag >= samples.len() {
        return 0.0;
    }
    let n = samples.len() - lag;
    let num: f64 = samples
        .iter()
        .take(n)
        .zip(samples.iter().skip(lag))
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    (num / f64::from(energy)) as f32
}

/// 抛物线插值：`y1` 为峰值点，`y0`/`y2` 为左右邻点，返回峰值偏移
/// （采样数，钳制到 `[-1, 1]`）。
fn parabolic_peak(y0: f32, y1: f32, y2: f32) -> f32 {
    let denom = y0 - 2.0 * y1 + y2;
    if denom.abs() <= f32::EPSILON {
        return 0.0;
    }
    (0.5 * (y0 - y2) / denom).clamp(-1.0, 1.0)
}

/// 在已算好的自相关值上选周期：`values[i]` 对应滞后 `min_lag + i`。
/// 取全局最大值；若其 90% 以上另有更小滞后的局部峰，取最小的那个
/// （避免取到周期的整数倍，即倍频程错误）；抛物线插值给亚采样精度。
/// 峰值低于 `min_peak` 时返回 `None`。
fn pick_period(values: &[f32], min_lag: usize, min_peak: f32) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut best_idx = 0;
    let mut best_val = values[0];
    for (i, &v) in values.iter().enumerate().skip(1) {
        if v > best_val {
            best_val = v;
            best_idx = i;
        }
    }
    if best_val < min_peak {
        return None;
    }
    let threshold = 0.9 * best_val;
    let mut chosen = best_idx;
    for i in 0..=best_idx {
        let v = values[i];
        if v < threshold {
            continue;
        }
        let left = if i == 0 {
            f32::NEG_INFINITY
        } else {
            values[i - 1]
        };
        let right = if i + 1 >= values.len() {
            f32::NEG_INFINITY
        } else {
            values[i + 1]
        };
        if v >= left && v >= right {
            chosen = i;
            break;
        }
    }
    let lag = if chosen > 0 && chosen + 1 < values.len() {
        let delta = parabolic_peak(values[chosen - 1], values[chosen], values[chosen + 1]);
        (min_lag + chosen) as f32 + delta
    } else {
        (min_lag + chosen) as f32
    };
    Some(lag)
}

/// 估计窗内信号的周期（采样数，亚采样精度），失败返回 `None`。
/// 窗短于 [`MIN_ESTIMATION_WINDOW_SAMPLES`]、能量为零、或峰值低于
/// `min_peak` 时失败。
/// `prev` 给出上一周期时，先在 `±20%` 窄窗内跟踪，跟踪失败再全窗重搜。
/// 本函数是公开的测量辅助（供测试与后续 P0-1 路由复用），不是生产
/// F0 检测器；生产测量用 FCPE（见跟进清单 §3）。
pub fn estimate_period(samples: &[f32], sample_rate: f32) -> Option<f32> {
    estimate_period_tracked(samples, sample_rate, DEFAULT_MIN_AUTOCORR_PEAK, None)
}

/// 带跟踪提示的 [`estimate_period`]：`prev` 为上一周期估计值。
pub fn estimate_period_tracked(
    samples: &[f32],
    sample_rate: f32,
    min_peak: f32,
    prev: Option<f32>,
) -> Option<f32> {
    let n = samples.len();
    if n < MIN_ESTIMATION_WINDOW_SAMPLES || sample_rate < crate::MIN_SAMPLE_RATE {
        return None;
    }
    let min_lag = ((sample_rate / MAX_F0_HZ).round() as usize).max(1);
    let max_lag = (sample_rate / MIN_F0_HZ) as usize;
    if max_lag <= min_lag || max_lag >= n {
        return None;
    }
    let energy: f32 = samples.iter().map(|x| x * x).sum();
    if energy <= 0.0 {
        return None;
    }
    if let Some(p) = prev {
        let lo = ((p * 0.75).round() as usize).max(min_lag);
        let hi = ((p * 1.33).round() as usize).min(max_lag);
        if hi > lo {
            let values: Vec<f32> = (lo..=hi)
                .map(|lag| autocorr_at(samples, energy, lag))
                .collect();
            if let Some(period) = pick_period(&values, lo, min_peak) {
                return Some(period);
            }
        }
    }
    let values: Vec<f32> = (min_lag..=max_lag)
        .map(|lag| autocorr_at(samples, energy, lag))
        .collect();
    pick_period(&values, min_lag, min_peak)
}

/// 浊音／清音过渡淡入淡出状态。
#[derive(Debug, Clone, Copy)]
struct FadeState {
    /// 剩余淡入淡出采样数。
    remaining: usize,
    /// 总淡入淡出采样数。
    total: usize,
    /// 淡出侧是否为浊音（为真：浊音→清音；为假：清音→浊音）。
    from_voiced: bool,
}

/// 周期重采样移调器（有状态，离线限定）。
///
/// 状态（读指针、周期网格、误差扩散进位、待执行的重复跳变、上一
/// chunk 的浊音模式）跨 `process` 调用保持，流式处理时不要在中途
/// 另建实例；重头开始用 [`PitchShifter::reset`]。
pub struct PitchShifter {
    sample_rate: f32,
    config: PitchShiftConfig,
    /// 实际施加的移调量（音分，snap 后）。
    cents_applied: f32,
    /// 移调比率 `2^(cents/1200)`。
    ratio: f64,
    /// 过渡淡入淡出长度（采样数，由 `transition_fade_s` 换算）。
    fade_samples: usize,
    /// 读指针（输入采样坐标，连续状态，永不 snap 回写指针）。
    in_pos: f64,
    /// 当前网格周期（采样数）；`None` 表示尚未估计出周期。
    period: Option<f32>,
    /// 当前周期网格的起点（输入采样坐标）。
    period_start: f64,
    /// 下一个周期边界（输入采样坐标）。
    next_boundary: f64,
    /// 当前周期已完成的发射次数。
    emissions_done: u32,
    /// 当前周期的计划发射次数（误差扩散给出）。
    emissions_planned: u32,
    /// 误差扩散进位：发射次数的小数部分累计。
    carry: f64,
    /// 待执行的重复跳变：剩余确认采样数（`lookahead_samples` 倒数）。
    pending_repeat: Option<usize>,
    /// 上一 chunk 的实际处理模式（浊音移调／直通），用于检出模式变化。
    prev_effective_voiced: Option<bool>,
    /// 进行中的过渡淡入淡出。
    fade: Option<FadeState>,
    /// 累计插入（重复）的周期数。
    inserted_total: u32,
    /// 累计删除（跳过）的周期数。
    deleted_total: u32,
}

impl PitchShifter {
    /// 构造移调器。`sample_rate` 须 ≥ [`crate::MIN_SAMPLE_RATE`]。
    ///
    /// # Panics
    ///
    /// 配置不合法（见 [`PitchShiftConfig::validate`]）或采样率低于
    /// 下限时 panic。
    pub fn new(sample_rate: f32, config: PitchShiftConfig) -> Self {
        config.validate();
        assert!(
            sample_rate >= crate::MIN_SAMPLE_RATE,
            "sample_rate {sample_rate} is below {}",
            crate::MIN_SAMPLE_RATE
        );
        let cents_applied = config.applied_cents();
        let ratio = pitch_ratio_from_cents(cents_applied);
        let fade_samples = (config.transition_fade_s * sample_rate).round().max(0.0) as usize;
        Self {
            sample_rate,
            config,
            cents_applied,
            ratio,
            fade_samples,
            in_pos: 0.0,
            period: None,
            period_start: 0.0,
            next_boundary: f64::INFINITY,
            emissions_done: 0,
            emissions_planned: 0,
            carry: 0.0,
            pending_repeat: None,
            prev_effective_voiced: None,
            fade: None,
            inserted_total: 0,
            deleted_total: 0,
        }
    }

    /// 实际施加的移调量（音分，snap 后）。
    pub fn applied_cents(&self) -> f32 {
        self.cents_applied
    }

    /// 重置全部流式状态（读指针、周期网格、进位、淡入淡出），
    /// 配置与采样率不变。
    pub fn reset(&mut self) {
        self.in_pos = 0.0;
        self.period = None;
        self.period_start = 0.0;
        self.next_boundary = f64::INFINITY;
        self.emissions_done = 0;
        self.emissions_planned = 0;
        self.carry = 0.0;
        self.pending_repeat = None;
        self.prev_effective_voiced = None;
        self.fade = None;
        self.inserted_total = 0;
        self.deleted_total = 0;
    }

    /// 移调一整段单声道音频。
    ///
    /// * `input`：输入采样（`f32`，单声道）。
    /// * `voiced`：每 chunk 的检测器 valid 标志（浊音为真）；长度
    ///   应为 `ceil(input.len() / chunk_samples)`，不足按清音计，
    ///   多余忽略。
    ///
    /// 返回 `(输出采样, 收据)`。输出长度恒等于输入长度（时长逐位
    /// 不变）；移调量为 0 时输出与输入逐位相同（快速路径）。
    /// 本函数只用于离线渲染（见模块文档）。
    pub fn process(&mut self, input: &[f32], voiced: &[bool]) -> (Vec<f32>, PitchShiftReceipt) {
        let n = input.len();
        let mut output = Vec::with_capacity(n);
        let mut receipt = PitchShiftReceipt::default();
        if n == 0 {
            return (output, receipt);
        }
        let chunk = self.config.chunk_samples.max(1);
        let n_chunks = n.div_ceil(chunk);
        if self.cents_applied.abs() < f32::EPSILON {
            output.extend_from_slice(input);
            self.in_pos += n as f64;
            for c in 0..n_chunks {
                receipt.chunks.push(ChunkReceipt {
                    chunk_index: c,
                    route: ChunkRoute::Passthrough,
                    cents_applied: 0.0,
                    voiced: voiced.get(c).copied().unwrap_or(false),
                    period_samples: None,
                    periods_inserted: 0,
                    periods_deleted: 0,
                    transition_fade: false,
                });
            }
            return (output, receipt);
        }
        for c in 0..n_chunks {
            let start = c * chunk;
            let end = (start + chunk).min(n);
            let detector_voiced = voiced.get(c).copied().unwrap_or(false);
            if self.pending_repeat.take().is_some() {
                self.execute_repeat();
            }
            let ins_before = self.inserted_total;
            let del_before = self.deleted_total;
            let win_start = (self.in_pos.floor() as usize).min(n.saturating_sub(1));
            let win_end = (win_start + self.config.estimation_window_samples).min(n);
            if detector_voiced
                && win_end - win_start >= MIN_ESTIMATION_WINDOW_SAMPLES
                && let Some(p) = estimate_period_tracked(
                    &input[win_start..win_end],
                    self.sample_rate,
                    self.config.min_autocorr_peak,
                    self.period,
                )
            {
                let need_anchor = match self.period {
                    None => true,
                    Some(grid) => (p - grid).abs() / grid > self.config.reanchor_relative_change,
                } || self.prev_effective_voiced != Some(true);
                if need_anchor {
                    self.reanchor(p);
                }
            }
            let effective_voiced = detector_voiced && self.period.is_some();
            let period_used = if effective_voiced { self.period } else { None };
            let mut transition = false;
            if self.fade.is_none()
                && let Some(prev) = self.prev_effective_voiced
                && prev != effective_voiced
                && self.fade_samples > 0
            {
                self.fade = Some(FadeState {
                    remaining: self.fade_samples,
                    total: self.fade_samples,
                    from_voiced: prev,
                });
                transition = true;
            }
            for idx in start..end {
                let pass = input[idx];
                let shifted = if effective_voiced || matches!(self.fade, Some(f) if f.from_voiced) {
                    self.shifted_sample(input)
                } else {
                    self.in_pos += 1.0;
                    pass
                };
                let out = match self.fade {
                    Some(mut fade) => {
                        let w = 1.0 - (fade.remaining as f32 / fade.total as f32);
                        let (a, b) = if fade.from_voiced {
                            (shifted, pass)
                        } else {
                            (pass, shifted)
                        };
                        let mixed = (1.0 - w) * a + w * b;
                        fade.remaining -= 1;
                        if fade.remaining == 0 {
                            self.fade = None;
                        } else {
                            self.fade = Some(fade);
                        }
                        mixed
                    }
                    None => {
                        if effective_voiced {
                            shifted
                        } else {
                            pass
                        }
                    }
                };
                output.push(out);
            }
            self.prev_effective_voiced = Some(effective_voiced);
            receipt.chunks.push(ChunkReceipt {
                chunk_index: c,
                route: if effective_voiced {
                    ChunkRoute::DspShift
                } else {
                    ChunkRoute::Passthrough
                },
                cents_applied: self.cents_applied,
                voiced: detector_voiced,
                period_samples: period_used,
                periods_inserted: self.inserted_total - ins_before,
                periods_deleted: self.deleted_total - del_before,
                transition_fade: transition,
            });
        }
        receipt.periods_inserted = self.inserted_total;
        receipt.periods_deleted = self.deleted_total;
        (output, receipt)
    }

    /// 在当前读指针处读一个采样（线性插值；整数位置精确返回原值；
    /// 越过尾端时保持最后一个采样）。
    fn read_at(input: &[f32], pos: f64) -> f32 {
        if input.is_empty() {
            return 0.0;
        }
        if pos <= 0.0 {
            return input[0];
        }
        let i = pos.floor() as usize;
        if i >= input.len() - 1 {
            return input[input.len() - 1];
        }
        let t = (pos - i as f64) as f32;
        if t <= 0.0 {
            return input[i];
        }
        input[i] * (1.0 - t) + input[i + 1] * t
    }

    /// 按当前模式产生一个移调采样：读指针前进 `ratio`，越过周期边界
    /// 时按计划重复或跳过一个整周期。重复跳变经前视确认后执行。
    fn shifted_sample(&mut self, input: &[f32]) -> f32 {
        if self.period.is_none() {
            let out = Self::read_at(input, self.in_pos);
            self.in_pos += 1.0;
            return out;
        }
        if let Some(countdown) = self.pending_repeat.take() {
            if countdown == 0 {
                self.execute_repeat();
            } else {
                self.pending_repeat = Some(countdown);
            }
        }
        let out = Self::read_at(input, self.in_pos);
        self.in_pos += self.ratio;
        if let Some(countdown) = self.pending_repeat.take() {
            self.pending_repeat = Some(countdown.saturating_sub(1));
        } else if self.in_pos >= self.next_boundary {
            self.emissions_done += 1;
            if self.emissions_done < self.emissions_planned {
                self.pending_repeat = Some(self.config.lookahead_samples);
            } else {
                self.advance_period();
            }
        }
        out
    }

    /// 执行一个已确认的重复跳变：读指针后退一个整周期（重读该周期）。
    fn execute_repeat(&mut self) {
        if let Some(p) = self.period {
            self.in_pos -= f64::from(p);
            self.inserted_total += 1;
        }
    }

    /// 误差扩散：为下一个周期计划发射次数（`ratio` 的整数部分＋
    /// 进位），小数部分累计进 `carry`，长期平均恰为 `ratio`。
    fn plan_next_period(&mut self) {
        let target = self.ratio + self.carry;
        let planned = target.floor().max(0.0) as u32;
        self.carry = target - f64::from(planned);
        self.emissions_planned = planned;
    }

    /// 网格前进一个周期：计划下一周期；计划为 0 的周期立即跳过
    /// （删除），直至下一周期计划 ≥ 1。
    fn advance_period(&mut self) {
        let Some(p) = self.period else {
            return;
        };
        let period = f64::from(p);
        self.period_start = self.next_boundary;
        self.next_boundary += period;
        self.emissions_done = 0;
        self.plan_next_period();
        while self.emissions_planned == 0 {
            self.skip_one_period();
        }
    }

    /// 跳过一个整周期（删除）：读指针前进一个周期，网格同步前进。
    fn skip_one_period(&mut self) {
        let Some(p) = self.period else {
            return;
        };
        let period = f64::from(p);
        self.in_pos += period;
        self.deleted_total += 1;
        self.period_start = self.next_boundary;
        self.next_boundary += period;
        self.emissions_done = 0;
        self.plan_next_period();
    }

    /// 在当前读指针处重锚周期网格（新周期、模式恢复、或偏差超阈）。
    /// 网格起点即当前读指针（边界与当前相位对齐）；计划为 0 的首个
    /// 周期立即跳过。
    fn reanchor(&mut self, period: f32) {
        self.period = Some(period);
        self.period_start = self.in_pos;
        self.next_boundary = self.in_pos + f64::from(period);
        self.emissions_done = 0;
        self.plan_next_period();
        while self.emissions_planned == 0 {
            self.skip_one_period();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libm::{cosf, log2, sinf};
    use std::f32::consts::PI;

    const SR: f32 = 48_000.0;
    const NOTE_LEN: usize = 24_000; // 0.5 s @ 48 kHz
    const NOTES: [f32; 4] = [220.0, 277.18, 329.63, 440.0];
    const SHIFTS: [f32; 6] = [-74.0, -50.0, -25.0, 25.0, 50.0, 74.0];

    /// 相位连续的纯音（`phase` 为进出相位，弧度）。
    fn tone(freq: f32, len: usize, phase: &mut f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(0.8 * sinf(*phase));
            *phase += 2.0 * PI * freq / SR;
            if *phase >= 2.0 * PI {
                *phase -= 2.0 * PI;
            }
        }
        out
    }

    /// 带 10 ms 升余弦起音／收音斜坡的纯音（首尾各 `ramp` 采样）。
    fn ramped_tone(freq: f32, len: usize, phase: &mut f32, ramp: usize) -> Vec<f32> {
        let mut out = tone(freq, len, phase);
        let ramp = ramp.min(len);
        for i in 0..ramp {
            let w = 0.5 * (1.0 - cosf(PI * i as f32 / ramp as f32));
            out[i] *= w;
            out[len - 1 - i] *= w;
        }
        out
    }

    /// 纯音阶梯：220 / 277.18 / 329.63 / 440 Hz，各 0.5 s，相位连续。
    fn staircase() -> Vec<f32> {
        let mut out = Vec::with_capacity(NOTES.len() * NOTE_LEN);
        let mut phase = 0.0f32;
        for &f in &NOTES {
            out.extend_from_slice(&tone(f, NOTE_LEN, &mut phase));
        }
        out
    }

    /// 确定性白噪声（xorshift32，种子固定）。
    fn noise(len: usize, mut seed: u32) -> Vec<f32> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            out.push((seed as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        out
    }

    /// 音分误差：实测 F0 相对目标 F0。
    fn cents_error(measured: f32, target: f32) -> f32 {
        1200.0 * log2(f64::from(measured) / f64::from(target)) as f32
    }

    /// 区间内最大逐采样差分绝对值（咔哒检测）。
    fn max_abs_diff(samples: &[f32], range: core::ops::Range<usize>) -> f32 {
        let mut max = 0.0f32;
        for i in range.start.max(1)..range.end.min(samples.len()) {
            let d = (samples[i] - samples[i - 1]).abs();
            if d > max {
                max = d;
            }
        }
        max
    }

    /// Hann 窗 DFT 在频率 `freq` 处的幅度（按窗和归一化）。
    fn dft_mag(samples: &[f32], freq: f32) -> f32 {
        let n = samples.len() as f32;
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        let mut wsum = 0.0f32;
        for (i, &x) in samples.iter().enumerate() {
            let w = 0.5 * (1.0 - cosf(2.0 * PI * i as f32 / n));
            let ang = 2.0 * PI * freq * i as f32 / SR;
            re += x * w * cosf(ang);
            im -= x * w * sinf(ang);
            wsum += w;
        }
        (re * re + im * im).sqrt() / wsum
    }

    /// 440 Hz（0.5 s）— 静音（0.2 s）— 554.37 Hz（0.5 s），起音 abrupt。
    /// 边界在采样 24000 与 33600 处。
    fn tone_silence_tone() -> Vec<f32> {
        let mut out = Vec::with_capacity(57_600);
        let mut phase = 0.0f32;
        out.extend_from_slice(&tone(440.0, NOTE_LEN, &mut phase));
        out.extend_from_slice(&[0.0; 9_600]);
        phase = 0.0;
        out.extend_from_slice(&tone(554.37, NOTE_LEN, &mut phase));
        out
    }

    #[test]
    fn defaults_are_pinned() {
        let cfg = PitchShiftConfig::default();
        assert_eq!(cfg.shift_cents, 0.0);
        assert!(!cfg.snap_to_semitone);
        assert_eq!(cfg.formant_factor, 1.0);
        assert_eq!(cfg.lookahead_samples, 5);
        assert_eq!(cfg.chunk_samples, 480);
        assert_eq!(cfg.estimation_window_samples, 2048);
        assert_eq!(cfg.min_autocorr_peak, 0.5);
        assert_eq!(cfg.reanchor_relative_change, 0.02);
        assert_eq!(cfg.transition_fade_s, 0.005);
    }

    #[test]
    fn estimator_finds_pure_tones() {
        // Bar is the acceptance bar (±5 cents). Worst case observed is
        // 110 Hz at +2.25 cents: energy normalisation makes the
        // autocorrelation peak asymmetric, which biases the parabolic
        // interpolation toward a shorter lag at low F0.
        let mut worst = 0.0f32;
        for &freq in &[110.0f32, 220.0, 440.0, 880.0] {
            let mut phase = 0.0f32;
            let signal = tone(freq, 4096, &mut phase);
            let period = estimate_period(&signal, SR).expect("period must be found");
            let measured = SR / period;
            let err = cents_error(measured, freq);
            worst = worst.max(err.abs());
            assert!(
                err.abs() <= 5.0,
                "estimator: {freq} Hz measured {measured:.2} Hz ({err:.2} cents)"
            );
        }
        eprintln!("estimator worst cents error: {worst:.2}");
    }

    #[test]
    fn zero_shift_is_bit_identical() {
        let input = staircase();
        let mut shifter = PitchShifter::new(SR, PitchShiftConfig::default());
        let n_chunks = input.len().div_ceil(480);
        let voiced = vec![true; n_chunks];
        let (output, receipt) = shifter.process(&input, &voiced);
        assert_eq!(output, input, "zero shift must be bit-identical");
        assert_eq!(receipt.chunks.len(), n_chunks);
        assert!(
            receipt
                .chunks
                .iter()
                .all(|c| c.route == ChunkRoute::Passthrough),
            "zero shift routes everything through passthrough"
        );
        assert_eq!(receipt.periods_inserted, 0);
        assert_eq!(receipt.periods_deleted, 0);
    }

    #[test]
    fn unvoiced_only_is_bit_identical() {
        let input = noise(48_000, 0x1234_5678);
        let cfg = PitchShiftConfig {
            shift_cents: 50.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg);
        let voiced = vec![false; 100];
        let (output, receipt) = shifter.process(&input, &voiced);
        assert_eq!(
            output, input,
            "unvoiced-only input must pass through bit-identical"
        );
        assert_eq!(receipt.chunks.len(), 100);
        assert!(
            receipt
                .chunks
                .iter()
                .all(|c| c.route == ChunkRoute::Passthrough),
            "unvoiced chunks must be passthrough"
        );
    }

    #[test]
    fn snap_mode_quantizes_to_semitone() {
        let cfg = PitchShiftConfig {
            shift_cents: 25.0,
            snap_to_semitone: true,
            ..PitchShiftConfig::default()
        };
        assert_eq!(cfg.applied_cents(), 0.0);
        let input = staircase();
        let mut shifter = PitchShifter::new(SR, cfg);
        let voiced = vec![true; input.len().div_ceil(480)];
        let (output, _) = shifter.process(&input, &voiced);
        assert_eq!(output, input, "snapped +25 cents must be bit-identical");

        let cfg = PitchShiftConfig {
            shift_cents: 74.0,
            snap_to_semitone: true,
            ..PitchShiftConfig::default()
        };
        assert_eq!(cfg.applied_cents(), 100.0);
        let mut shifter = PitchShifter::new(SR, cfg);
        let (output, _) = shifter.process(&input, &voiced);
        let window = &output[NOTE_LEN + 7_200..NOTE_LEN + 19_200];
        let measured = estimate_period(window, SR).map(|p| SR / p).expect("period");
        let target = 277.18 * pitch_ratio_from_cents(100.0) as f32;
        let err = cents_error(measured, target);
        assert!(
            err.abs() <= 5.0,
            "snapped +100 cents: measured {measured:.2} Hz vs target {target:.2} Hz ({err:.2} cents)"
        );
    }

    #[test]
    fn length_preserved_for_all_test_shifts() {
        let input = staircase();
        let voiced = vec![true; input.len().div_ceil(480)];
        for &shift in &SHIFTS {
            let cfg = PitchShiftConfig {
                shift_cents: shift,
                ..PitchShiftConfig::default()
            };
            let mut shifter = PitchShifter::new(SR, cfg);
            let (output, _) = shifter.process(&input, &voiced);
            assert_eq!(
                output.len(),
                input.len(),
                "shift {shift} cents changed length: {} vs {}",
                output.len(),
                input.len()
            );
        }
    }

    #[test]
    fn staircase_f0_within_5_cents() {
        let input = staircase();
        let voiced = vec![true; input.len().div_ceil(480)];
        for &shift in &SHIFTS {
            let cfg = PitchShiftConfig {
                shift_cents: shift,
                ..PitchShiftConfig::default()
            };
            let mut shifter = PitchShifter::new(SR, cfg);
            let (output, _) = shifter.process(&input, &voiced);
            let ratio = pitch_ratio_from_cents(shift) as f32;
            let mut worst = 0.0f32;
            for (k, &freq) in NOTES.iter().enumerate() {
                let start = k * NOTE_LEN + 7_200;
                let end = k * NOTE_LEN + 19_200;
                let window = &output[start..end];
                let measured = estimate_period(window, SR)
                    .map(|p| SR / p)
                    .expect("output period must be measurable");
                let target = freq * ratio;
                let err = cents_error(measured, target);
                worst = worst.max(err.abs());
                assert!(
                    err.abs() <= 5.0,
                    "shift {shift:+} cents, note {freq} Hz: measured {measured:.2} Hz vs target {target:.2} Hz ({err:.2} cents)"
                );
            }
            eprintln!("shift {shift:+} cents: worst F0 error {worst:.2} cents");
        }
    }

    #[test]
    fn boundary_no_click_abrupt_with_fade() {
        let input = tone_silence_tone();
        let cfg = PitchShiftConfig {
            shift_cents: 50.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg);
        let mut voiced = vec![true; 50];
        voiced.extend_from_slice(&[false; 20]);
        voiced.extend_from_slice(&[true; 50]);
        let (output, _) = shifter.process(&input, &voiced);
        assert_eq!(output.len(), input.len());
        assert!(output.iter().all(|x| x.is_finite()));
        let interior =
            max_abs_diff(&output, 4_800..14_400).max(max_abs_diff(&output, 38_400..48_000));
        let boundary =
            max_abs_diff(&output, 23_760..24_240).max(max_abs_diff(&output, 33_360..33_840));
        eprintln!(
            "abrupt boundary with fade: boundary max|d| {boundary:.4}, interior {interior:.4}"
        );
        assert!(
            boundary <= 3.0 * interior,
            "abrupt boundary click: boundary max|d| {boundary:.4} vs interior {interior:.4}"
        );
    }

    #[test]
    fn boundary_no_click_gradual_without_fade() {
        let mut signal = Vec::with_capacity(57_600);
        let mut phase = 0.0f32;
        signal.extend_from_slice(&ramped_tone(440.0, NOTE_LEN, &mut phase, 480));
        signal.extend_from_slice(&[0.0; 9_600]);
        phase = 0.0;
        signal.extend_from_slice(&ramped_tone(554.37, NOTE_LEN, &mut phase, 480));
        let cfg = PitchShiftConfig {
            shift_cents: 50.0,
            transition_fade_s: 0.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg);
        let mut voiced = vec![true; 50];
        voiced.extend_from_slice(&[false; 20]);
        voiced.extend_from_slice(&[true; 50]);
        let (output, _) = shifter.process(&signal, &voiced);
        assert_eq!(output.len(), signal.len());
        assert!(output.iter().all(|x| x.is_finite()));
        let interior =
            max_abs_diff(&output, 4_800..14_400).max(max_abs_diff(&output, 38_400..48_000));
        let boundary =
            max_abs_diff(&output, 23_760..24_240).max(max_abs_diff(&output, 33_360..33_840));
        eprintln!(
            "gradual boundary without fade: boundary max|d| {boundary:.4}, interior {interior:.4}"
        );
        assert!(
            boundary <= 3.0 * interior,
            "gradual boundary click without fade: boundary max|d| {boundary:.4} vs interior {interior:.4}"
        );
    }

    #[test]
    fn spectrum_no_abrupt_change_at_boundary() {
        // A window straddling a tone edge already leaks in the INPUT
        // (rectangular truncation of the tone inside the window), so the
        // honest bar is output-vs-input at identical windows: the shifter
        // must not add HF content beyond what the input already has.
        let input = tone_silence_tone();
        let cfg = PitchShiftConfig {
            shift_cents: 50.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg);
        let mut voiced = vec![true; 50];
        voiced.extend_from_slice(&[false; 20]);
        voiced.extend_from_slice(&[true; 50]);
        let (output, _) = shifter.process(&input, &voiced);
        // 1024-sample Hann windows: two boundaries, two interiors.
        let windows = [
            (&output[23_488..24_512], &input[23_488..24_512], 440.0f32),
            (&output[33_088..34_112], &input[33_088..34_112], 554.37f32),
            (&output[9_472..10_496], &input[9_472..10_496], 440.0f32),
            (&output[42_752..43_776], &input[42_752..43_776], 554.37f32),
        ];
        let hf_ratio = |win: &[f32], fundamental: f32| {
            let fund = dft_mag(win, fundamental);
            let mut hf = 0.0f32;
            for &f in &[8_000.0f32, 12_000.0, 16_000.0] {
                hf = hf.max(dft_mag(win, f));
            }
            hf / fund
        };
        for (i, (out_win, in_win, fundamental)) in windows.iter().enumerate().take(2) {
            let out_ratio = hf_ratio(out_win, *fundamental);
            let in_ratio = hf_ratio(in_win, *fundamental);
            eprintln!(
                "boundary window {i}: output HF/fundamental {out_ratio:.2e}, input {in_ratio:.2e}"
            );
            assert!(
                out_ratio <= 3.0 * in_ratio,
                "boundary window {i} adds HF: output {out_ratio:.2e} vs input {in_ratio:.2e}"
            );
            assert!(
                out_ratio <= 1e-3,
                "boundary window {i} HF/fundamental ratio {out_ratio:.2e} exceeds 1e-3"
            );
        }
    }

    #[test]
    fn receipt_marks_every_chunk() {
        let input = tone_silence_tone();
        let cfg = PitchShiftConfig {
            shift_cents: 50.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg);
        let mut voiced = vec![true; 50];
        voiced.extend_from_slice(&[false; 20]);
        voiced.extend_from_slice(&[true; 50]);
        let (output, receipt) = shifter.process(&input, &voiced);
        assert_eq!(output.len(), input.len());
        assert_eq!(receipt.chunks.len(), 120);
        for (i, chunk) in receipt.chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
            assert_eq!(chunk.cents_applied, 50.0);
            if matches!(i, 50..70) {
                assert_eq!(chunk.route, ChunkRoute::Passthrough);
                assert!(!chunk.voiced);
                assert_eq!(chunk.period_samples, None);
            } else {
                assert_eq!(chunk.route, ChunkRoute::DspShift);
                assert!(chunk.voiced);
                assert!(chunk.period_samples.is_some());
            }
        }
        assert!(
            receipt.chunks[50].transition_fade,
            "chunk 50 must start a fade"
        );
        assert!(
            receipt.chunks[70].transition_fade,
            "chunk 70 must start a fade"
        );
        assert!(receipt.periods_inserted > 0, "shift up must insert periods");
        eprintln!(
            "receipt: {} chunks, inserted {}, deleted {}",
            receipt.chunks.len(),
            receipt.periods_inserted,
            receipt.periods_deleted
        );
    }

    #[test]
    fn shift_up_inserts_shift_down_deletes() {
        let input = staircase();
        let voiced = vec![true; input.len().div_ceil(480)];
        let cfg_up = PitchShiftConfig {
            shift_cents: 74.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg_up);
        let (_, receipt) = shifter.process(&input, &voiced);
        assert!(receipt.periods_inserted > 0, "shift up must insert periods");
        assert_eq!(receipt.periods_deleted, 0, "shift up must delete none");
        let cfg_down = PitchShiftConfig {
            shift_cents: -74.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg_down);
        let (_, receipt) = shifter.process(&input, &voiced);
        assert!(
            receipt.periods_deleted > 0,
            "shift down must delete periods"
        );
        assert_eq!(receipt.periods_inserted, 0, "shift down must insert none");
    }

    #[test]
    fn reset_restarts_the_stream() {
        let input = staircase();
        let voiced = vec![true; input.len().div_ceil(480)];
        let cfg = PitchShiftConfig {
            shift_cents: 50.0,
            ..PitchShiftConfig::default()
        };
        let mut shifter = PitchShifter::new(SR, cfg);
        let (first, receipt_first) = shifter.process(&input, &voiced);
        shifter.reset();
        let (second, receipt_second) = shifter.process(&input, &voiced);
        assert_eq!(
            second, first,
            "reset must reproduce the first run bit-for-bit"
        );
        assert_eq!(receipt_second, receipt_first);
    }

    #[test]
    #[should_panic(expected = "formant_factor must be 1.0")]
    fn formant_factor_must_be_one() {
        let cfg = PitchShiftConfig {
            formant_factor: 1.5,
            ..PitchShiftConfig::default()
        };
        let _ = PitchShifter::new(SR, cfg);
    }
}
