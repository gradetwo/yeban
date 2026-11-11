//! 结构化人性化引擎 v2：分布替换（第一层）+ 自回归记忆（第二层）。
//!
//! 本模块是 `humanizeClip` 的结构化抖动路径（`model = "structured"`）。
//! 旧均匀路径（`model = "uniform"`，缺省）仍在 `notes.rs` 里、逐位不变，
//! 只换种子派生（见 [`derive_track_seed`]）。
//!
//! ## 第一层：分布替换
//!
//! 均匀抖动在统计上最不真人：真人时值/力度误差有偏、有记忆、有结构。
//! 本层把抖动换成高斯分布（Box-Muller，由种子 splitmix64 均匀抽样生成；
//! 对数正态 [`lognormal`] 已实现、供未来时值/比率类误差用，
//! `humanizeClip` 只抖有符号的时值与力度、暂不接线）。
//!
//! 时值 σ 缺省 = `timeJitterTicks / 3`（最小 1）：±幅度 ≈ ±3σ，
//! 参考实测（弦乐四重奏微时值偏差均值 6.51 ms、标准差 2.65 ms）——
//! 15 tick（120 BPM、960 PPQ）≈ 2.6 ms，σ = 5 tick 即复现该标准差。
//! 偏差 |d| ≤ min(幅度, 240)：模型 `micro_timing_ticks` 的 ±240 上界
//! （120 BPM 下 ≈ ±125 ms）比参考的 ~200 ms 更紧，以模型上界为准。
//!
//! ## 第二层：自回归记忆
//!
//! 偏差序列按音符 tick 顺序生成 AR(2)：
//! `d[n] = φ1·d[n−1] + φ2·d[n−2] + σ_ε·z[n]`，
//! `φ1 = 3α/2`、`φ2 = −α²/2`（实根形：主根 α、次根 α/2；
//! α = 0 时退化为白噪声，α → 1 时趋向平滑/趋势）。
//! 创新方差按平稳方差归一化（`σ_ε² = σ²·(1 − φ1ρ1 − φ2ρ2)`），
//! 使平稳标准差恰为目标 σ；前 16 步燃尽（burn-in），首个可听偏差
//! 已是平稳分布。α 以千分比 `arAlphaPermille`（0..=950）传参，
//! 缺省 800（α = 0.8，滞后 1 自相关 ρ1 ≈ 0.91，偏平滑侧）。
//!
//! ## 阻尼（细粒度平滑，Wang 2026-11-11 定）
//!
//! 除自回归外另有显式阻尼：相邻音偏差跳变受限，
//! `|d[n] − d[n−1]| ≤ max(1, 生效幅度 / 4)`（整数精确，见 [`structured_deviations`]；
//! 生效幅度 = 截断后的幅度：时值 `min(幅度, 240)`，力度 `幅度`）。
//! 首音以前一值为燃尽段末值（平稳抽样），不从 0 起跳。
//! 阻尼只管结构化路径；旧均匀路径保持均匀性状，不加阻尼。
//!
//! ## 种子与确定性（Wang 2026-11-11 定）
//!
//! 全部随机只走 splitmix64 纯函数（⛔ 无 `rand`、无 `thread_rng`）：
//! 同（base seed，轨，参数）⇒ 同输出。
//! 有效种子按轨派生：`effective_seed = splitmix64(base_seed ⊕ 轨盐)`，
//! 轨盐是轨 ULID 高低 64 位的嵌套 splitmix64 折叠，
//! 与 `yeban-model` 的 `trigger_hash` 同型；
//! 同轨跑两次序列相同，不同轨即使音符相同、base seed 相同，
//! 抖动序列也不同（消除跨轨碰撞；确定性契约现为每（seed，轨））。
//!
//! ## 扩展点（第三层/第四层是后续线，本模块只留接口）
//!
//! [`HumanizeModel`] 是扩展点：后续线加 `model` 变体
//! （HMM/GMM 隐状态、分数高斯噪声长程相关）时，
//! 在此加变体 + 生成函数，`humanizeClip` 的字段契约与展开通道不变。

use std::f64::consts::TAU;

use yeban_model::music::MICRO_TIMING_MAX_ABS;
use yeban_model::EntityId;

/// splitmix64 的黄金常数（与 `yeban-model` / `yeban-theory` 同值）。
pub(crate) const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// splitmix64 的 5 步终结器（与 `yeban-model` 的同名实现逐位一致）。
#[must_use]
pub(crate) fn splitmix64_finalizer(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

/// 单参 splitmix64（与 `yeban-model` 的 `splitmix64(seed)` 同型）。
#[must_use]
pub(crate) fn splitmix64_single(seed: u64) -> u64 {
    splitmix64_finalizer(seed.wrapping_add(SPLITMIX_GAMMA))
}

/// 双参 splitmix64 混合：`finalizer(seed + salt·GAMMA + GAMMA)`。
#[must_use]
pub(crate) fn splitmix64_mix(seed: u64, salt: u64) -> u64 {
    splitmix64_finalizer(
        seed.wrapping_add(salt.wrapping_mul(SPLITMIX_GAMMA))
            .wrapping_add(SPLITMIX_GAMMA),
    )
}

/// 按轨派生有效种子（Wang 2026-11-11 定：每轨种子必须不一致）。
///
/// `effective_seed = splitmix64(base_seed ⊕ 轨盐)`，
/// 轨盐 = 轨 ULID 高低 64 位的嵌套 splitmix64 折叠，
/// 与 `yeban-model` 的 `trigger_hash` 同型（轨用身份、不用下标：
/// 轨重排不改变抖动）。
#[must_use]
pub(crate) fn derive_track_seed(base_seed: u64, track_id: &EntityId) -> u64 {
    let raw = track_id.as_ulid().0;
    let low = raw as u64;
    let high = (raw >> 64) as u64;
    splitmix64_single(base_seed ^ splitmix64_single(low ^ splitmix64_single(high)))
}

/// 抖动模型（`humanizeClip` 的 `model` 字段；缺省 `Uniform`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HumanizeModel {
    /// 旧均匀抖动（缺省；新字段缺省时走此路）。
    #[default]
    Uniform,
    /// 结构化抖动 v2：高斯 + AR(2) + 阻尼（本模块）。
    Structured,
}

impl HumanizeModel {
    /// 解析 `model` 字段值（小写；未知值返回 `None`，由调用方报
    /// `INVALID_PARAMETER_RANGE`）。
    #[must_use]
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "uniform" => Some(Self::Uniform),
            "structured" => Some(Self::Structured),
            _ => None,
        }
    }
}

/// 自回归系数 α 的缺省值（千分比 800 = α 0.8，偏平滑侧）。
pub const DEFAULT_AR_ALPHA_PERMILLE: u16 = 800;

/// 自回归系数 α 的上界（千分比 950 = α 0.95；再高创新方差过小、
/// 序列近乎常数，失去人性化意义）。
pub const MAX_AR_ALPHA_PERMILLE: u16 = 950;

/// 结构化路径的参数（解析时缺省已填好，见 [`resolve_structured_params`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructuredParams {
    /// 时值高斯 σ（tick）。
    pub timing_std_ticks: u64,
    /// 力度高斯 σ。
    pub velocity_std: u64,
    /// 自回归系数 α 的千分比（0..=950）。
    pub ar_alpha_permille: u16,
}

impl StructuredParams {
    /// 由 `humanizeClip` 的幅度与可选字段填缺省：
    /// σ 缺省 = 幅度 / 3（最小 1；幅度 0 时 σ = 0，即不动）；
    /// α 缺省 [`DEFAULT_AR_ALPHA_PERMILLE`]。
    #[must_use]
    pub(crate) fn resolve(
        time_jitter_ticks: u64,
        velocity_jitter: u8,
        timing_std_ticks: Option<u64>,
        velocity_std: Option<u64>,
        ar_alpha_permille: Option<u16>,
    ) -> Self {
        let default_std = |amplitude: u64| -> u64 {
            if amplitude == 0 {
                0
            } else {
                (amplitude / 3).max(1)
            }
        };
        Self {
            timing_std_ticks: timing_std_ticks.unwrap_or_else(|| default_std(time_jitter_ticks)),
            velocity_std: velocity_std
                .unwrap_or_else(|| default_std(u64::from(velocity_jitter))),
            ar_alpha_permille: ar_alpha_permille.unwrap_or(DEFAULT_AR_ALPHA_PERMILLE),
        }
    }
}

/// splitmix64 驱动的均匀流（Box-Muller 与 AR 的创新源）。
pub(crate) struct SplitMixStream {
    state: u64,
}

impl SplitMixStream {
    /// 以种子开流。
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// 下一个 64 位均匀输出。
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(SPLITMIX_GAMMA);
        splitmix64_finalizer(self.state)
    }

    /// (0, 1] 上的均匀浮点：`1 − (x >> 11) / 2^53`，永不为 0
    /// （Box-Muller 取 `ln` 安全；取 1 时半径为 0，亦合法）。
    pub(crate) fn next_unit_open(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / 9_007_199_254_740_992.0; // 2^-53
        1.0 - ((self.next_u64() >> 11) as f64 * SCALE)
    }
}

/// Box-Muller：一对独立标准正态抽样（均值 0，标准差 1）。
///
/// `f64` 的 `ln` / `cos` / `sin` / `sqrt` 是 IEEE 确定性运算：
/// 同种子同平台 ⇒ 同输出（本仓库金向量判据已依赖同类浮点确定性）。
#[must_use]
pub(crate) fn gaussian_pair(stream: &mut SplitMixStream) -> (f64, f64) {
    let u1 = stream.next_unit_open();
    let u2 = stream.next_unit_open();
    let radius = (-2.0 * u1.ln()).sqrt();
    let angle = TAU * u2;
    (radius * angle.cos(), radius * angle.sin())
}

/// 对数正态抽样：`exp(σ·z)`，中位数 1、右偏、恒正。
///
/// 供未来时值/比率类误差用（`humanizeClip` 只抖有符号的时值与力度，
/// 暂不接线）。现只在测试里用（`#[cfg(test)]`）；
/// 时值/比率抖动槽落地时去 `cfg(test)`、接线到生成路径。
#[cfg(test)]
#[must_use]
pub(crate) fn lognormal(stream: &mut SplitMixStream, sigma: f64) -> f64 {
    (sigma * gaussian_pair(stream).0).exp()
}

/// AR(2) 系数：`φ1 = 3α/2`、`φ2 = −α²/2`（实根形：主根 α、次根 α/2）。
///
/// α = 0 时 `(0, 0)`（白噪声）；α 越近 1，滞后 1 自相关越近 1。
#[must_use]
pub(crate) fn ar_coefficients(alpha_permille: u16) -> (f64, f64) {
    let alpha = f64::from(alpha_permille) / 1000.0;
    (3.0 * alpha / 2.0, -alpha * alpha / 2.0)
}

/// 平稳方差因子 `1 − φ1ρ1 − φ2ρ2`（Yule-Walker：`ρ1 = φ1/(1−φ2)`，
/// `ρ2 = φ1ρ1 + φ2`；创新方差 = 因子 × 目标方差）。
///
/// α ∈ [0, 0.95] 时因子 ∈ [0.028…, 1]，恒正。
#[must_use]
fn ar_variance_factor(phi1: f64, phi2: f64) -> f64 {
    let rho1 = phi1 / (1.0 - phi2);
    let rho2 = phi1 * rho1 + phi2;
    1.0 - phi1 * rho1 - phi2 * rho2
}

/// 生成 `count` 个平稳标准正态 AR(2) 偏差（均值 0，平稳 σ = 1）。
///
/// 前 16 步燃尽（起始态 (0, 0)，主根 0.8 时残余 ≈ 0.8^16 ≈ 0.03），
/// 首个可听偏差已是平稳分布；每步只取 Box-Muller 对的第一分量
/// （第二分量丢弃；确定性不受影响）。
/// 返回（序列，燃尽段末值）——末值是首音阻尼的参考点。
#[must_use]
pub(crate) fn ar_normals(
    stream: &mut SplitMixStream,
    count: usize,
    alpha_permille: u16,
) -> (Vec<f64>, f64) {
    const BURN_IN: usize = 16;
    let (phi1, phi2) = ar_coefficients(alpha_permille);
    let innovation_std = ar_variance_factor(phi1, phi2).max(0.0).sqrt();
    let mut prev2 = 0.0f64; // d[n−2]
    let mut prev1 = 0.0f64; // d[n−1]
    let mut burn_in_tail = 0.0f64;
    let mut out = Vec::with_capacity(count);
    for step in 0..BURN_IN + count {
        let (z, _) = gaussian_pair(stream);
        let current = phi1 * prev1 + phi2 * prev2 + innovation_std * z;
        prev2 = prev1;
        prev1 = current;
        if step < BURN_IN {
            if step + 1 == BURN_IN {
                burn_in_tail = current;
            }
        } else {
            out.push(current);
        }
    }
    (out, burn_in_tail)
}

/// 一条通道的结构化偏差：AR 序列 × σ → 取整 → 幅度截断 → 阻尼限摆。
///
/// - 幅度 0 或数量 0 时全 0（不耗流，输出仍确定）；
/// - 偏差先截到 `±cap`（时值 `cap = min(幅度, 240)`，力度 `cap = 幅度`），
///   再做整数阻尼 `|d[n] − d[n−1]| ≤ max(1, cap / 4)`（`cap` 是生效幅度：
///   幅度本身是 `u64`、可任意大，阻尼必须以截断后的生效幅度算，
///   否则 `as i64` 溢出会反转截断区间）；
/// - 首音以前一值为燃尽段末值（同 σ 缩放、同样截到 `±cap`）。
fn structured_channel(
    effective_seed: u64,
    salt: u64,
    count: usize,
    amplitude: u64,
    std_dev: u64,
    alpha_permille: u16,
    cap: i64,
) -> Vec<i64> {
    if count == 0 || amplitude == 0 {
        return vec![0; count];
    }
    let mut stream = SplitMixStream::new(splitmix64_mix(effective_seed, salt));
    let (normals, burn_in_tail) = ar_normals(&mut stream, count, alpha_permille);
    let std_dev = std_dev as f64;
    let slew = (cap / 4).max(1);
    let mut prev = ((burn_in_tail * std_dev).round() as i64).clamp(-cap, cap);
    let mut out = Vec::with_capacity(count);
    for z in normals {
        let raw = ((z * std_dev).round() as i64).clamp(-cap, cap);
        let damped = raw.clamp(prev - slew, prev + slew);
        prev = damped;
        out.push(damped);
    }
    out
}

/// 结构化偏差对：时值偏差（tick）与力度偏差（整数单位）。
///
/// 时值与力度各走独立流（盐 0 / 1），同种子同参数 ⇒ 同输出；
/// 时值 `cap = min(time_jitter_ticks, 240)`（模型上界），
/// 力度 `cap = velocity_jitter`（最终力度 1..=127 的截断在调用方）。
#[must_use]
pub(crate) fn structured_deviations(
    effective_seed: u64,
    count: usize,
    time_jitter_ticks: u64,
    velocity_jitter: u8,
    params: &StructuredParams,
) -> StructuredDeviations {
    // `cap` 在 `u64` 域里取 min（幅度是 `u64`、可任意大；
    // 先 `as i64` 再 min 会在幅度 > i64::MAX 时 wrap 成负数、反转截断区间）。
    let timing_cap =
        time_jitter_ticks.min(u64::try_from(MICRO_TIMING_MAX_ABS).unwrap_or(240)) as i64;
    StructuredDeviations {
        timing: structured_channel(
            effective_seed,
            0,
            count,
            time_jitter_ticks,
            params.timing_std_ticks,
            params.ar_alpha_permille,
            timing_cap,
        ),
        velocity: structured_channel(
            effective_seed,
            1,
            count,
            u64::from(velocity_jitter),
            params.velocity_std,
            params.ar_alpha_permille,
            i64::from(velocity_jitter),
        ),
    }
}

/// 结构化偏差对（时值 tick + 力度单位）。
pub struct StructuredDeviations {
    /// 每音时值偏差（tick；与 tick 排序后的音符一一对应）。
    pub timing: Vec<i64>,
    /// 每音力度偏差（与 tick 排序后的音符一一对应）。
    pub velocity: Vec<i64>,
}

/// 滞后 1 自相关（均值中心化；长度 < 2 或方差 0 时返回 0）。
///
/// 平滑性判据的测试辅助（`notes.rs` 的结构化测试用它断言 ρ1 下界）；
/// 现只在测试里用（`#[cfg(test)]`）。
#[cfg(test)]
#[must_use]
pub(crate) fn lag1_autocorrelation(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (index, value) in values.iter().enumerate() {
        let centered = value - mean;
        denominator += centered * centered;
        if index > 0 {
            numerator += centered * (values[index - 1] - mean);
        }
    }
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::ids::deterministic_id;
    use super::*;

    /// 固定测试轨身份（确定性派生，不手写 ULID 文本）。
    fn test_track_id() -> EntityId {
        deterministic_id("humanize-v2-test-track-a")
    }

    #[test]
    fn box_muller_pair_is_standard_normal() {
        let mut stream = SplitMixStream::new(42);
        let count = 4096;
        let mut sum = 0.0;
        let mut sum_sq = 0.0;
        for _ in 0..count {
            let (z1, z2) = gaussian_pair(&mut stream);
            sum += z1 + z2;
            sum_sq += z1 * z1 + z2 * z2;
        }
        let total = f64::from(count * 2);
        let mean = sum / total;
        let variance = sum_sq / total - mean * mean;
        assert!(mean.abs() < 0.05, "均值 {mean} 应近 0");
        assert!(
            (0.95..=1.05).contains(&variance.sqrt()),
            "标准差 {} 应近 1",
            variance.sqrt()
        );
    }

    #[test]
    fn gaussian_stream_is_deterministic() {
        let take = |seed: u64| -> Vec<f64> {
            let mut stream = SplitMixStream::new(seed);
            (0..64).map(|_| gaussian_pair(&mut stream).0).collect()
        };
        assert_eq!(take(42), take(42));
        assert_ne!(take(42), take(43));
    }

    #[test]
    fn lognormal_median_is_one_and_deterministic() {
        let take = |seed: u64| -> Vec<f64> {
            let mut stream = SplitMixStream::new(seed);
            (0..4096).map(|_| lognormal(&mut stream, 0.5)).collect()
        };
        let mut values = take(7);
        let first = take(7);
        assert_eq!(values, first);
        values.sort_by(|left, right| left.partial_cmp(right).unwrap());
        let median = values[values.len() / 2];
        assert!((0.9..=1.1).contains(&median), "中位数 {median} 应近 1");
        assert!(values.iter().all(|value| *value > 0.0));
    }

    #[test]
    fn ar_alpha_zero_is_white_noise() {
        let mut stream = SplitMixStream::new(42);
        let (values, _) = ar_normals(&mut stream, 2000, 0);
        let rho1 = lag1_autocorrelation(&values);
        assert!(rho1.abs() < 0.1, "α=0 时 ρ1 = {rho1}，应近 0");
        let mean_abs_step = values
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .sum::<f64>()
            / (values.len() - 1) as f64;
        assert!(mean_abs_step > 0.9, "α=0 时平均 |Δ| = {mean_abs_step}，应近 1.13");
    }

    #[test]
    fn ar_default_alpha_is_smooth() {
        let mut stream = SplitMixStream::new(42);
        let (values, _) = ar_normals(&mut stream, 2000, DEFAULT_AR_ALPHA_PERMILLE);
        let rho1 = lag1_autocorrelation(&values);
        assert!(rho1 >= 0.8, "缺省 α=0.8 时 ρ1 = {rho1}，应 ≥ 0.8");
        let mean_abs_step = values
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .sum::<f64>()
            / (values.len() - 1) as f64;
        assert!(mean_abs_step < 0.6, "缺省 α 时平均 |Δ| = {mean_abs_step}，应 < 0.6");
    }

    #[test]
    fn ar_alpha_controls_smoothness() {
        let mean_abs_step = |alpha: u16| -> f64 {
            let mut stream = SplitMixStream::new(42);
            let (values, _) = ar_normals(&mut stream, 2000, alpha);
            values
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .sum::<f64>()
                / (values.len() - 1) as f64
        };
        let white = mean_abs_step(0);
        let smooth = mean_abs_step(DEFAULT_AR_ALPHA_PERMILLE);
        assert!(
            white > 2.0 * smooth,
            "α=0 平均 |Δ| {white} 应大于缺省 α 的 2 倍 {smooth}",
        );
    }

    #[test]
    fn ar_variance_is_normalized() {
        let mut stream = SplitMixStream::new(42);
        let (values, _) = ar_normals(&mut stream, 4000, DEFAULT_AR_ALPHA_PERMILLE);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance =
            values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len() as f64;
        assert!(
            (0.9..=1.1).contains(&variance.sqrt()),
            "平稳标准差 {} 应近 1",
            variance.sqrt()
        );
    }

    #[test]
    fn derive_track_seed_differs_per_track() {
        let track_a = test_track_id();
        let track_b = deterministic_id("humanize-v2-test-track-b");
        assert_eq!(derive_track_seed(42, &track_a), derive_track_seed(42, &track_a));
        assert_ne!(derive_track_seed(42, &track_a), derive_track_seed(42, &track_b));
        assert_ne!(derive_track_seed(42, &track_a), derive_track_seed(43, &track_a));
    }

    #[test]
    fn structured_deviations_are_deterministic_and_damped() {
        let seed = derive_track_seed(42, &test_track_id());
        let params = StructuredParams::resolve(20, 10, None, None, None);
        let first = structured_deviations(seed, 64, 20, 10, &params);
        let second = structured_deviations(seed, 64, 20, 10, &params);
        assert_eq!(first.timing, second.timing);
        assert_eq!(first.velocity, second.velocity);
        // 阻尼：相邻步 ≤ 幅度/4（时值 20/4 = 5，力度 10/4 = 2）。
        let max_step = |values: &[i64]| -> i64 {
            values
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .max()
                .unwrap_or(0)
        };
        assert!(max_step(&first.timing) <= 5, "时值步进超限");
        assert!(max_step(&first.velocity) <= 2, "力度步进超限");
        assert!(first.timing.iter().all(|value| value.abs() <= 20));
        assert!(first.velocity.iter().all(|value| value.abs() <= 10));
    }

    #[test]
    fn structured_deviations_golden_vector() {
        let seed = derive_track_seed(42, &test_track_id());
        let params = StructuredParams::resolve(20, 10, None, None, None);
        let deviations = structured_deviations(seed, 8, 20, 10, &params);
        // 金向量（固定种子/轨/参数的首 8 个偏差；重钉机制：改断言即可）。
        assert_eq!(deviations.timing, [-10, -9, -8, -3, 0, 0, 1, 4]);
        assert_eq!(deviations.velocity, [2, 3, 2, 2, 2, 3, 2, 0]);
    }
}
