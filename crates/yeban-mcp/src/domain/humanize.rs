//! 结构化人性化引擎 v3：分布替换（第一层）+ 自回归记忆（第二层）
//! + 隐状态（第三层）+ 长程相关（第四层）。
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
//! ## 第三层：隐状态（HMM）
//!
//! 真人演奏不是一直在同一种"手感"里：一段里弹得准、一段里整体偏早、
//! 一段里整体偏晚、一段里放得开。四种隐状态（[`HiddenState`]）按
//! 马尔可夫链沿音符走：自留概率 `p` 缺省 0.8（`hmmStayPermille`），
//! 平均每态持续 `1/(1−p)` ≈ 5 个音；转移时在其余三态间均匀选。
//! 转移矩阵对称 ⇒ 平稳分布均匀（每态 1/4），与初始抽样一致。
//!
//! 状态只调制**时值**（偏早/偏晚是时值概念；力度不走 HMM）：
//! `d[n] = (bias_s + scale_s · m[n]) · norm · σ`，其中 `m[n]` 是
//! 第二层 + 第四层混合后的单位方差序列（见下），
//! `bias = [0, −0.5, +0.5, 0] σ`、`scale = [0.5, 0.75, 0.75, 1.25]`，
//! `norm = 1/√(E[scale²] + E[bias²]) = 1/√0.859375` ——
//! 均匀平稳分布下 `E[scale²] = 0.734375`、`E[bias²] = 0.125`，
//! 乘 `norm` 后时值平稳标准差仍是目标 σ（不漂移）。
//! 状态切换时的偏置跳变仍受阻尼管束（相邻步 ≤ 幅度/4），
//! 不会因为换态就突变。
//!
//! ## 第四层：长程相关（分数高斯噪声 fGn）
//!
//! AR(2) 只有短程记忆（相关长度几个音）；真人演奏的快慢起伏
//! 有长程相关：DFA 指数 α ≈ 0.81（Hurst 指数 H ≈ 0.81，
//! α = H）。本层用 Hosking 精确模拟生成标准分数高斯噪声
//! `g[n]`（自协方差 `γ(k) = ½(|k+1|^{2H} − 2|k|^{2H} + |k−1|^{2H})`，
//! Durbin-Levinson 递推，O(n²)，音符数 n 通常 < 1000），
//! 再与 AR 序列混合：`m[n] = (a[n] + w·g[n]) / √(1 + w²)`，
//! `w` 缺省 0.5（`fgnWeightPermille`，0 = 关掉本层）。
//! 混合仍是单位方差、零均值、平稳的；大尺度上 fGn 占优，
//! 故 DFA α 显著 > 0.5（纯 fGn 时 α ≈ H）。
//!
//! ## 扩展点
//!
//! 四层已齐。后续线若加第五层（如力度-时值耦合），
//! 在此加生成函数，`humanizeClip` 的字段契约与展开通道不变。

use std::f64::consts::TAU;

use yeban_model::EntityId;
use yeban_model::music::MICRO_TIMING_MAX_ABS;

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
///
/// ⚠ **唯一实现住在 `yeban_model::clip_ops::derive_track_seed`**：宿主（GUI）
/// 与判据也调那一份 —— 这里只转调，⛔ 不另写一份公式（两份公式必然分叉）。
#[must_use]
pub(crate) fn derive_track_seed(base_seed: u64, track_id: &EntityId) -> u64 {
    yeban_model::clip_ops::derive_track_seed(base_seed, track_id)
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

/// HMM 自留概率 p 的缺省值（千分比 800 = p 0.8，平均每态持续 5 个音）。
pub const DEFAULT_HMM_STAY_PERMILLE: u16 = 800;

/// HMM 自留概率 p 的上界（千分比 990；再高一态可占整段，失去切换意义）。
pub const MAX_HMM_STAY_PERMILLE: u16 = 990;

/// fGn Hurst 指数 H 的缺省值（千分比 810 = H 0.81，DFA α ≈ 0.81）。
pub const DEFAULT_HURST_PERMILLE: u16 = 810;

/// fGn Hurst 指数 H 的下界（千分比 500 = H 0.5，白噪声，无长程相关）。
pub const MIN_HURST_PERMILLE: u16 = 500;

/// fGn Hurst 指数 H 的上界（千分比 950 = H 0.95；再高 Hosking 递推
/// 的条件方差过小、数值不稳）。
pub const MAX_HURST_PERMILLE: u16 = 950;

/// fGn 混合权重 w 的缺省值（千分比 500 = w 0.5；fGn 占混合方差的 20%）。
pub const DEFAULT_FGN_WEIGHT_PERMILLE: u16 = 500;

/// fGn 混合权重 w 的上界（千分比 1000 = w 1.0；fGn 与 AR 各占一半方差）。
pub const MAX_FGN_WEIGHT_PERMILLE: u16 = 1000;

/// 第三/四层旋钮（`humanizeClip` 的三个新可选字段；缺省见各常量）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerParams {
    /// HMM 自留概率 p 的千分比（0..=990）。
    pub hmm_stay_permille: u16,
    /// fGn Hurst 指数 H 的千分比（500..=950）。
    pub hurst_permille: u16,
    /// fGn 混合权重 w 的千分比（0..=1000；0 = 关掉第四层）。
    pub fgn_weight_permille: u16,
}

impl Default for LayerParams {
    /// 缺省：自留 800、Hurst 810、fGn 权重 500。
    fn default() -> Self {
        Self {
            hmm_stay_permille: DEFAULT_HMM_STAY_PERMILLE,
            hurst_permille: DEFAULT_HURST_PERMILLE,
            fgn_weight_permille: DEFAULT_FGN_WEIGHT_PERMILLE,
        }
    }
}

impl LayerParams {
    /// 由 `humanizeClip` 的可选字段填缺省（`None` = 各自缺省）。
    #[must_use]
    pub(crate) fn resolve(
        hmm_stay_permille: Option<u16>,
        hurst_permille: Option<u16>,
        fgn_weight_permille: Option<u16>,
    ) -> Self {
        Self {
            hmm_stay_permille: hmm_stay_permille.unwrap_or(DEFAULT_HMM_STAY_PERMILLE),
            hurst_permille: hurst_permille.unwrap_or(DEFAULT_HURST_PERMILLE),
            fgn_weight_permille: fgn_weight_permille.unwrap_or(DEFAULT_FGN_WEIGHT_PERMILLE),
        }
    }
}

/// 隐状态数（precise / slightly-early / slightly-late / free）。
pub const HIDDEN_STATE_COUNT: usize = 4;

/// 隐状态调制的方差因子 `E[scale²] + E[bias²]`（均匀平稳分布下）：
/// `E[scale²] = (0.5² + 0.75² + 0.75² + 1.25²) / 4 = 0.734375`，
/// `E[bias²] = (0 + 0.5² + 0.5² + 0) / 4 = 0.125`，合计 0.859375。
/// 时值偏差乘 `1/√0.859375` 后平稳标准差仍是目标 σ（见单测）。
pub const HMM_VARIANCE_FACTOR: f64 = 0.859_375;

/// 结构化路径的参数（解析时缺省已填好，见 [`resolve_structured_params`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructuredParams {
    /// 时值高斯 σ（tick）。
    pub timing_std_ticks: u64,
    /// 力度高斯 σ。
    pub velocity_std: u64,
    /// 自回归系数 α 的千分比（0..=950）。
    pub ar_alpha_permille: u16,
    /// HMM 自留概率 p 的千分比（0..=990）。
    pub hmm_stay_permille: u16,
    /// fGn Hurst 指数 H 的千分比（500..=950）。
    pub hurst_permille: u16,
    /// fGn 混合权重 w 的千分比（0..=1000；0 = 关掉第四层）。
    pub fgn_weight_permille: u16,
}

impl StructuredParams {
    /// 由 `humanizeClip` 的幅度与可选字段填缺省：
    /// σ 缺省 = 幅度 / 3（最小 1；幅度 0 时 σ = 0，即不动）；
    /// α 缺省 [`DEFAULT_AR_ALPHA_PERMILLE`]；第三/四层旋钮见 `layers`。
    #[must_use]
    pub(crate) fn resolve(
        time_jitter_ticks: u64,
        velocity_jitter: u8,
        timing_std_ticks: Option<u64>,
        velocity_std: Option<u64>,
        ar_alpha_permille: Option<u16>,
        layers: LayerParams,
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
            velocity_std: velocity_std.unwrap_or_else(|| default_std(u64::from(velocity_jitter))),
            ar_alpha_permille: ar_alpha_permille.unwrap_or(DEFAULT_AR_ALPHA_PERMILLE),
            hmm_stay_permille: layers.hmm_stay_permille,
            hurst_permille: layers.hurst_permille,
            fgn_weight_permille: layers.fgn_weight_permille,
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
/// ⚠ `libm::log` / `libm::cos` / `libm::sin` 而不是 `f64` 的 `.ln()` / `.cos()` /
/// `.sin()`：`ARCH-DET-001` 要求"统一启用纯 Rust libm"，禁止落到宿主 libm ——
/// 否则同一份工程导出的母带会随平台差 LSB。本 crate 的判据
/// `probe_authenticity::the_crates_own_transcendental_call_sites_are_the_registered_list`
/// 会**当场**把 std 的 `.ln()` / `.cos()` / `.sin()` 抓出来（`sfz.rs` 的 `sine` 同例）。
/// `sqrt` 属 IEEE 精确类（正确舍入），可留。
#[must_use]
pub(crate) fn gaussian_pair(stream: &mut SplitMixStream) -> (f64, f64) {
    let u1 = stream.next_unit_open();
    let u2 = stream.next_unit_open();
    let radius = (-2.0 * libm::log(u1)).sqrt();
    let angle = TAU * u2;
    (radius * libm::cos(angle), radius * libm::sin(angle))
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

/// 隐状态（第三层）：一段演奏里的"手感"模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HiddenState {
    /// 弹得准：偏置 0，尺度 0.5（紧）。
    Precise,
    /// 整体偏早：偏置 −0.5σ，尺度 0.75。
    SlightlyEarly,
    /// 整体偏晚：偏置 +0.5σ，尺度 0.75。
    SlightlyLate,
    /// 放得开：偏置 0，尺度 1.25（宽）。
    Free,
}

impl HiddenState {
    /// 按序号取态（0..=3；均匀抽样用，序号取模）。
    #[must_use]
    fn from_index(index: usize) -> Self {
        match index % HIDDEN_STATE_COUNT {
            0 => Self::Precise,
            1 => Self::SlightlyEarly,
            2 => Self::SlightlyLate,
            _ => Self::Free,
        }
    }

    /// 序号（0..=3；转移时跳过当前态用）。
    #[must_use]
    fn index(self) -> usize {
        match self {
            Self::Precise => 0,
            Self::SlightlyEarly => 1,
            Self::SlightlyLate => 2,
            Self::Free => 3,
        }
    }

    /// 偏置（单位 σ）。
    #[must_use]
    fn bias_sigma(self) -> f64 {
        match self {
            Self::Precise | Self::Free => 0.0,
            Self::SlightlyEarly => -0.5,
            Self::SlightlyLate => 0.5,
        }
    }

    /// 尺度（乘在单位方差序列上）。
    #[must_use]
    fn scale(self) -> f64 {
        match self {
            Self::Precise => 0.5,
            Self::SlightlyEarly | Self::SlightlyLate => 0.75,
            Self::Free => 1.25,
        }
    }
}

/// 生成 `count` 个隐状态（马尔可夫链；初态均匀，转移时以概率 `p`
/// 自留，否则在其余三态间均匀选；`p` 越近 1，同一手感持续越久）。
///
/// 转移矩阵对称 ⇒ 平稳分布均匀（每态 1/4），与初态抽样一致；
/// 平均每态持续 `1/(1−p)` 个音（p = 0.8 时 ≈ 5 个音）。
#[must_use]
pub(crate) fn hmm_states(
    stream: &mut SplitMixStream,
    count: usize,
    stay_permille: u16,
) -> Vec<HiddenState> {
    let stay = f64::from(stay_permille) / 1000.0;
    let mut out = Vec::with_capacity(count);
    if count == 0 {
        return out;
    }
    let mut current = HiddenState::from_index(
        ((stream.next_unit_open() * HIDDEN_STATE_COUNT as f64) as usize)
            .min(HIDDEN_STATE_COUNT - 1),
    );
    for _ in 0..count {
        out.push(current);
        if stream.next_unit_open() >= stay {
            // 换态：在其余三态间均匀选（抽 0..=2，跳过当前态的序号）。
            let pick = ((stream.next_unit_open() * 3.0) as usize).min(2);
            let next_index = if pick >= current.index() {
                pick + 1
            } else {
                pick
            };
            current = HiddenState::from_index(next_index);
        }
    }
    out
}

/// 分数高斯噪声自协方差 `γ(k)`（Hurst 指数 H；k ≥ 0；γ(0) = 1）。
///
/// `γ(k) = ½(|k+1|^{2H} − 2|k|^{2H} + |k−1|^{2H})`。
///
/// ⚠ `libm::pow` 而不是 `f64::powf`：见 [`gaussian_pair`] 的 ARCH-DET-001 说明。
#[must_use]
fn fgn_autocovariance(k: usize, two_h: f64) -> f64 {
    if k == 0 {
        return 1.0;
    }
    let k = k as f64;
    0.5 * (libm::pow(k + 1.0, two_h) - 2.0 * libm::pow(k, two_h) + libm::pow(k - 1.0, two_h))
}

/// 生成 `count` 个平稳标准正态分数高斯噪声（Hosking 精确模拟）。
///
/// Durbin-Levinson 递推：O(n²) 时间、O(n) 内存；n 是音符数（通常 < 1000）。
/// H = 0.5 时退化为白噪声（γ(k>0) = 0，递推系数全 0）。
/// 返回（序列，序列前值）——前值是首音阻尼的参考点（与 AR 的燃尽段末值
/// 同位；Hosking 是联合模拟，前值与首值同分布且相关，比凭空取 0 更忠实）。
#[must_use]
pub(crate) fn fgn_normals(
    stream: &mut SplitMixStream,
    count: usize,
    hurst_permille: u16,
) -> (Vec<f64>, f64) {
    // 多生成一个作前值。
    let total = count + 1;
    let two_h = 2.0 * f64::from(hurst_permille) / 1000.0;
    let mut gamma = Vec::with_capacity(total);
    for k in 0..total {
        gamma.push(fgn_autocovariance(k, two_h));
    }
    // phi[j] 是当前步的第 j 个系数（1-based 记号 φ[n][j]）；每步先由上一行
    // 算新对角元，再由上一行的副本更新其余系数（就地更新会串行污染）。
    let mut phi = vec![0.0f64; total];
    let mut values = Vec::with_capacity(total);
    let mut variance = gamma[0]; // v_0 = γ(0) = 1
    for n in 0..total {
        let (z, _) = gaussian_pair(stream);
        if n == 0 {
            values.push(variance.max(0.0).sqrt() * z);
            continue;
        }
        // φ[n][n] = (γ(n) − Σ_{j=1..n-1} φ[n−1][j]·γ(n−j)) / v_{n−1}。
        let mut numerator = gamma[n];
        for j in 1..n {
            numerator -= phi[j] * gamma[n - j];
        }
        let phi_nn = if variance > 0.0 {
            numerator / variance
        } else {
            0.0
        };
        let prev = phi.clone();
        for j in 1..n {
            phi[j] = prev[j] - phi_nn * prev[n - j];
        }
        phi[n] = phi_nn;
        variance *= 1.0 - phi_nn * phi_nn;
        let mut mean = 0.0;
        for j in 1..=n {
            mean += phi[j] * values[n - j];
        }
        values.push(mean + variance.max(0.0).sqrt() * z);
    }
    let pre = values[0];
    values.remove(0);
    (values, pre)
}

/// 一条通道的结构化偏差（浮点序列，单位 σ）：L2 AR 与 L4 fGn 混合，
/// 时值再叠加 L3 HMM 状态调制（力度不走 HMM：偏早/偏晚是时值概念）。
///
/// 返回（每音单位 σ 的浮点偏差，首音前的平稳前值）。
/// 浮点序列供 [`structured_channel`] 取整/截断/阻尼，也供单测断言
/// DFA α 与滞后 1 自相关（整数量化会压低这些量）。
#[must_use]
fn structured_floats(
    effective_seed: u64,
    salt_ar: u64,
    salt_fgn: u64,
    salt_hmm: u64,
    count: usize,
    params: &StructuredParams,
    apply_hmm: bool,
) -> (Vec<f64>, f64) {
    let mut ar_stream = SplitMixStream::new(splitmix64_mix(effective_seed, salt_ar));
    let (ar, ar_pre) = ar_normals(&mut ar_stream, count, params.ar_alpha_permille);
    let weight = f64::from(params.fgn_weight_permille) / 1000.0;
    let (mixed, mixed_pre) = if params.fgn_weight_permille == 0 {
        (ar, ar_pre)
    } else {
        let mut fgn_stream = SplitMixStream::new(splitmix64_mix(effective_seed, salt_fgn));
        let (fgn, fgn_pre) = fgn_normals(&mut fgn_stream, count, params.hurst_permille);
        let mix_norm = (1.0 + weight * weight).sqrt();
        let mixed = ar
            .iter()
            .zip(fgn.iter())
            .map(|(a, g)| (a + weight * g) / mix_norm)
            .collect();
        let pre = (ar_pre + weight * fgn_pre) / mix_norm;
        (mixed, pre)
    };
    if !apply_hmm {
        return (mixed, mixed_pre);
    }
    let mut hmm_stream = SplitMixStream::new(splitmix64_mix(effective_seed, salt_hmm));
    let states = hmm_states(&mut hmm_stream, count, params.hmm_stay_permille);
    let norm = (1.0 / HMM_VARIANCE_FACTOR).sqrt();
    let modulated = mixed
        .iter()
        .zip(states.iter())
        .map(|(value, state)| (state.bias_sigma() + state.scale() * value) * norm)
        .collect();
    let first_state = states.first().copied().unwrap_or(HiddenState::Precise);
    let pre = (first_state.bias_sigma() + first_state.scale() * mixed_pre) * norm;
    (modulated, pre)
}

/// 一条通道的生成规约（`structured_channel` 的入参束；参数过多会触发
/// `clippy::too_many_arguments`）。
struct ChannelSpec {
    /// 按轨派生的有效种子。
    effective_seed: u64,
    /// AR 流的盐。
    salt_ar: u64,
    /// fGn 流的盐。
    salt_fgn: u64,
    /// HMM 流的盐（力度不用，预留）。
    salt_hmm: u64,
    /// 音符数。
    count: usize,
    /// 抖动幅度（0 = 不动）。
    amplitude: u64,
    /// 高斯 σ。
    std_dev: u64,
    /// 截断上限（时值 `min(幅度, 240)`，力度 `幅度`）。
    cap: i64,
    /// 是否叠加 HMM 状态调制（只用于时值）。
    apply_hmm: bool,
}

/// 一条通道的结构化偏差：浮点序列 × σ → 取整 → 幅度截断 → 阻尼限摆。
///
/// - 幅度 0 或数量 0 时全 0（不耗流，输出仍确定）；
/// - 偏差先截到 `±cap`（时值 `cap = min(幅度, 240)`，力度 `cap = 幅度`），
///   再做整数阻尼 `|d[n] − d[n−1]| ≤ max(1, cap / 4)`（`cap` 是生效幅度：
///   幅度本身是 `u64`、可任意大，阻尼必须以截断后的生效幅度算，
///   否则 `as i64` 溢出会反转截断区间）；
/// - 首音以前一值为平稳前值（同 σ 缩放、同样截到 `±cap`）。
/// - 时值与力度各走独立流（盐：AR 0/1、fGn 2/3、HMM 4/5；HMM 只用于时值，
///   力度的 HMM 盐 5 预留）。
fn structured_channel(spec: ChannelSpec, params: &StructuredParams) -> Vec<i64> {
    if spec.count == 0 || spec.amplitude == 0 {
        return vec![0; spec.count];
    }
    let (floats, pre) = structured_floats(
        spec.effective_seed,
        spec.salt_ar,
        spec.salt_fgn,
        spec.salt_hmm,
        spec.count,
        params,
        spec.apply_hmm,
    );
    let std_dev = spec.std_dev as f64;
    let slew = (spec.cap / 4).max(1);
    let mut prev = ((pre * std_dev).round() as i64).clamp(-spec.cap, spec.cap);
    let mut out = Vec::with_capacity(spec.count);
    for value in floats {
        let raw = ((value * std_dev).round() as i64).clamp(-spec.cap, spec.cap);
        let damped = raw.clamp(prev - slew, prev + slew);
        prev = damped;
        out.push(damped);
    }
    out
}

/// 结构化偏差对：时值偏差（tick）与力度偏差（整数单位）。
///
/// 时值与力度各走独立流（盐：AR 0/1、fGn 2/3、HMM 4/5），同种子同参数 ⇒ 同输出；
/// 时值 `cap = min(time_jitter_ticks, 240)`（模型上界），
/// 力度 `cap = velocity_jitter`（最终力度 1..=127 的截断在调用方）。
/// 时值叠加 HMM 状态调制（L3），力度只走 AR + fGn（L2 + L4）。
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
            ChannelSpec {
                effective_seed,
                salt_ar: 0,
                salt_fgn: 2,
                salt_hmm: 4,
                count,
                amplitude: time_jitter_ticks,
                std_dev: params.timing_std_ticks,
                cap: timing_cap,
                apply_hmm: true,
            },
            params,
        ),
        velocity: structured_channel(
            ChannelSpec {
                effective_seed,
                salt_ar: 1,
                salt_fgn: 3,
                salt_hmm: 5,
                count,
                amplitude: u64::from(velocity_jitter),
                std_dev: params.velocity_std,
                cap: i64::from(velocity_jitter),
                apply_hmm: false,
            },
            params,
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
    use super::*;
    use crate::domain::ids::deterministic_id;

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
        // R75: 纯函数自比 —— 同一种子两次调用必须输出同一高斯流（确定性探针）。
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
        // 有界：`.all(` 站点须有长度界（assertion_bounds_audit），非空即真。
        assert_eq!(values.len(), 4096);
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
        assert!(
            mean_abs_step > 0.9,
            "α=0 时平均 |Δ| = {mean_abs_step}，应近 1.13"
        );
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
        assert!(
            mean_abs_step < 0.6,
            "缺省 α 时平均 |Δ| = {mean_abs_step}，应 < 0.6"
        );
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
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / values.len() as f64;
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
        // R75: 纯函数自比 —— 同一种子同轨两次派生必须同一有效种子（确定性探针）。
        assert_eq!(
            derive_track_seed(42, &track_a),
            derive_track_seed(42, &track_a)
        );
        assert_ne!(
            derive_track_seed(42, &track_a),
            derive_track_seed(42, &track_b)
        );
        assert_ne!(
            derive_track_seed(42, &track_a),
            derive_track_seed(43, &track_a)
        );
    }

    #[test]
    fn structured_deviations_are_deterministic_and_damped() {
        let seed = derive_track_seed(42, &test_track_id());
        let params = StructuredParams::resolve(20, 10, None, None, None, LayerParams::default());
        let first = structured_deviations(seed, 64, 20, 10, &params);
        let second = structured_deviations(seed, 64, 20, 10, &params);
        // 解构到局部变量：`.all(` 站点的界须写在根标识符上
        //（assertion_bounds_audit：`first.timing.len()` 不算 `first` 的界）。
        let StructuredDeviations {
            timing: timing_a,
            velocity: velocity_a,
        } = first;
        let StructuredDeviations {
            timing: timing_b,
            velocity: velocity_b,
        } = second;
        assert_eq!(timing_a, timing_b);
        assert_eq!(velocity_a, velocity_b);
        // 有界：`.all(` 站点须有长度界，非空即真。
        assert_eq!(timing_a.len(), 64);
        assert_eq!(velocity_a.len(), 64);
        // 阻尼：相邻步 ≤ 幅度/4（时值 20/4 = 5，力度 10/4 = 2）。
        let max_step = |values: &[i64]| -> i64 {
            values
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .max()
                .unwrap_or(0)
        };
        assert!(max_step(&timing_a) <= 5, "时值步进超限");
        assert!(max_step(&velocity_a) <= 2, "力度步进超限");
        assert!(timing_a.iter().all(|value| value.abs() <= 20));
        assert!(velocity_a.iter().all(|value| value.abs() <= 10));
    }

    /// DFA 指数 α（单测辅助）：序列去均值后累积求和，按尺度分箱、
    /// 每箱去线性趋势、均方根涨落 F(s)；log F(s) 对 log s 的最小二乘斜率。
    /// 白噪声 α ≈ 0.5；fGn(H) 的 α ≈ H。
    /// ⚠ `libm::log` 而不是 `.ln()`：与生产代码同一条 ARCH-DET-001 口径。
    #[must_use]
    fn dfa_alpha(values: &[f64], scales: &[usize]) -> f64 {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let mut profile = Vec::with_capacity(values.len());
        let mut acc = 0.0;
        for value in values {
            acc += value - mean;
            profile.push(acc);
        }
        let mut log_s = Vec::new();
        let mut log_f = Vec::new();
        for &scale in scales {
            if scale < 4 || scale * 2 > values.len() {
                continue;
            }
            let boxes = values.len() / scale;
            let mut sum_sq = 0.0;
            let mut count = 0usize;
            for box_index in 0..boxes {
                let segment = &profile[box_index * scale..(box_index + 1) * scale];
                let n = scale as f64;
                let mean_x = (n - 1.0) / 2.0;
                let mean_y = segment.iter().sum::<f64>() / n;
                let mut sxy = 0.0;
                let mut sxx = 0.0;
                for (index, y) in segment.iter().enumerate() {
                    let x = index as f64 - mean_x;
                    sxy += x * (y - mean_y);
                    sxx += x * x;
                }
                let slope = if sxx == 0.0 { 0.0 } else { sxy / sxx };
                let intercept = mean_y - slope * mean_x;
                for (index, y) in segment.iter().enumerate() {
                    let fit = slope * index as f64 + intercept;
                    sum_sq += (y - fit) * (y - fit);
                    count += 1;
                }
            }
            log_s.push(libm::log(scale as f64));
            log_f.push(libm::log((sum_sq / count as f64).sqrt()));
        }
        let n = log_s.len() as f64;
        let mean_s = log_s.iter().sum::<f64>() / n;
        let mean_f = log_f.iter().sum::<f64>() / n;
        let mut sxy = 0.0;
        let mut sxx = 0.0;
        for (index, value) in log_s.iter().enumerate() {
            sxy += (value - mean_s) * (log_f[index] - mean_f);
            sxx += (value - mean_s) * (value - mean_s);
        }
        if sxx == 0.0 { 0.0 } else { sxy / sxx }
    }

    #[test]
    fn layer_params_defaults() {
        let resolved = LayerParams::resolve(None, None, None);
        assert_eq!(resolved, LayerParams::default());
        assert_eq!(resolved.hmm_stay_permille, DEFAULT_HMM_STAY_PERMILLE);
        assert_eq!(resolved.hurst_permille, DEFAULT_HURST_PERMILLE);
        assert_eq!(resolved.fgn_weight_permille, DEFAULT_FGN_WEIGHT_PERMILLE);
        let partial = LayerParams::resolve(Some(900), None, Some(0));
        assert_eq!(partial.hmm_stay_permille, 900);
        assert_eq!(partial.hurst_permille, DEFAULT_HURST_PERMILLE);
        assert_eq!(partial.fgn_weight_permille, 0);
    }

    #[test]
    fn hmm_states_are_deterministic_and_persistent() {
        let take = |seed: u64| -> Vec<HiddenState> {
            let mut stream = SplitMixStream::new(seed);
            hmm_states(&mut stream, 2000, DEFAULT_HMM_STAY_PERMILLE)
        };
        // R75: 纯函数自比 —— 同一种子两次调用必须输出同一状态序列（确定性探针）。
        assert_eq!(take(42), take(42));
        assert_ne!(take(42), take(43));
        let states = take(42);
        // 持续性：平均每态持续 ≈ 1/(1−0.8) = 5 个音。
        let mut runs = 0usize;
        for pair in states.windows(2) {
            if pair[0] != pair[1] {
                runs += 1;
            }
        }
        let mean_run = states.len() as f64 / (runs + 1) as f64;
        assert!(
            (3.0..=8.0).contains(&mean_run),
            "平均每态持续 {mean_run} 个音，应近 5"
        );
        // 平稳分布均匀：每态占比近 1/4。界放宽到 [0.18, 0.32]：
        // 态是相关的（平均持续 5 个音），有效样本是段数（≈400）不是音数，
        // 占比的标准差 ≈ 0.022，±3σ ≈ ±0.065。
        for state in [
            HiddenState::Precise,
            HiddenState::SlightlyEarly,
            HiddenState::SlightlyLate,
            HiddenState::Free,
        ] {
            let share =
                states.iter().filter(|value| **value == state).count() as f64 / states.len() as f64;
            assert!(
                (0.18..=0.32).contains(&share),
                "态 {state:?} 占比 {share}，应近 0.25"
            );
        }
    }

    #[test]
    fn hmm_variance_factor_matches_state_table() {
        // 方差因子 = E[scale²] + E[bias²]（均匀平稳分布下逐项算，不手填总数）。
        let states = [
            HiddenState::Precise,
            HiddenState::SlightlyEarly,
            HiddenState::SlightlyLate,
            HiddenState::Free,
        ];
        let mean_scale_sq = states
            .iter()
            .map(|state| state.scale() * state.scale())
            .sum::<f64>()
            / states.len() as f64;
        let mean_bias_sq = states
            .iter()
            .map(|state| state.bias_sigma() * state.bias_sigma())
            .sum::<f64>()
            / states.len() as f64;
        assert_eq!(mean_scale_sq + mean_bias_sq, HMM_VARIANCE_FACTOR);
    }

    #[test]
    fn fgn_hurst_controls_dfa_alpha() {
        // 尺度取 [16, 32, 64, 128]：256/512 只有 16/8 个箱、单实现波动大
        // （多种子实测：全尺度白噪声 0.48–0.56、fGn 0.75–0.90；
        // 中尺度白噪声 0.48–0.53、fGn 0.81–0.87，更稳）。
        let scales = [16usize, 32, 64, 128];
        let take = |seed: u64, hurst: u16| -> Vec<f64> {
            let mut stream = SplitMixStream::new(seed);
            fgn_normals(&mut stream, 4096, hurst).0
        };
        // R75: 纯函数自比 —— 同一种子两次调用必须输出同一 fGn 序列（确定性探针）。
        assert_eq!(
            take(42, DEFAULT_HURST_PERMILLE),
            take(42, DEFAULT_HURST_PERMILLE)
        );
        assert_ne!(
            take(42, DEFAULT_HURST_PERMILLE),
            take(43, DEFAULT_HURST_PERMILLE)
        );
        // H = 0.81 时 DFA α ≈ 0.81（Wang 2026-11-11 锚点）。
        let alpha = dfa_alpha(&take(42, DEFAULT_HURST_PERMILLE), &scales);
        assert!(
            (0.74..=0.88).contains(&alpha),
            "H=0.81 时 DFA α = {alpha}，应近 0.81"
        );
        // H = 0.5 时退化为白噪声，DFA α ≈ 0.5。
        let alpha_white = dfa_alpha(&take(42, MIN_HURST_PERMILLE), &scales);
        assert!(
            (0.42..=0.58).contains(&alpha_white),
            "H=0.5 时 DFA α = {alpha_white}，应近 0.5"
        );
    }

    #[test]
    fn fgn_stationary_std_is_one() {
        let mut stream = SplitMixStream::new(42);
        let (values, _) = fgn_normals(&mut stream, 4000, DEFAULT_HURST_PERMILLE);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / values.len() as f64;
        assert!(
            (0.9..=1.1).contains(&variance.sqrt()),
            "fGn 平稳标准差 {} 应近 1",
            variance.sqrt()
        );
    }

    #[test]
    fn structured_floats_keep_unit_variance_and_gain_long_range_correlation() {
        let seed = derive_track_seed(42, &test_track_id());
        let params = StructuredParams::resolve(20, 10, None, None, None, LayerParams::default());
        let (timing, _) = structured_floats(seed, 0, 2, 4, 2000, &params, true);
        let mean = timing.iter().sum::<f64>() / timing.len() as f64;
        let variance = timing
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / timing.len() as f64;
        assert!(
            (0.85..=1.15).contains(&variance.sqrt()),
            "时值浮点平稳标准差 {} 应近 1",
            variance.sqrt()
        );
        // 混合输出的 DFA α 显著 > 0.5（L4 长程相关落地；纯 L2 时 α → 0.5）。
        // 尺度取中段 [16, 32, 64, 128]（同 `fgn_hurst_controls_dfa_alpha` 的取舍）。
        let scales = [16usize, 32, 64, 128];
        let alpha = dfa_alpha(&timing, &scales);
        assert!(alpha > 0.6, "混合输出 DFA α = {alpha}，应 > 0.6");
        // 短程记忆仍在：滞后 1 自相关 ≥ 0.7（L2 AR + HMM 持续性）。
        let rho1 = lag1_autocorrelation(&timing);
        assert!(rho1 >= 0.7, "混合输出 ρ1 = {rho1}，应 ≥ 0.7");
    }

    #[test]
    fn structured_deviations_golden_vector() {
        let seed = derive_track_seed(42, &test_track_id());
        let params = StructuredParams::resolve(20, 10, None, None, None, LayerParams::default());
        let deviations = structured_deviations(seed, 8, 20, 10, &params);
        // 金向量（固定种子/轨/参数的首 8 个偏差；重钉机制：改断言即可）。
        // ⚠ v3 重钉：L3 HMM + L4 fGn 叠进 structured，旧 v2 金向量已失效
        // （时值 [-10, -9, -8, -3, 0, 0, 1, 4]、力度 [2, 3, 2, 2, 2, 3, 2, 0]）；
        // 均匀路径与字段契约不变，只有 structured 的派生值变。
        assert_eq!(deviations.timing, [-11, -11, -9, -5, -4, -7, -2, 3]);
        assert_eq!(deviations.velocity, [1, 2, 4, 5, 6, 5, 5, 3]);
    }
}
