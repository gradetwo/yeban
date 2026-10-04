//! 电平计量原语：峰值/真峰值保持、RMS 一阶平滑、dBFS 换算、输入钳位与"取最新"判据。
//! [ARCH-UI-002, ROAD-M2-008]
//!
//! ## 为什么电平口径属于 `yeban-dsp`
//!
//! [ARCH-UI-002] 要求实时线程压入的是**真峰值与 RMS 电平**。真值 = 一套可复算的
//! DSP 口径，而不是"把样本绝对值取个 max"。它是**纯数学**：没有队列、没有线程、
//! 没有设备、没有模型类型。因此它的归属是 `yeban-dsp`（本 crate），
//! 由引擎（`yeban-engine`）以 re-export 消费 —— 混音台、母带、导出都能直接复用
//! 同一份口径，而不是各自再写一遍。
//!
//! **迁移留痕（main `b014e8f` → 本线）**：本模块的整体原样来自
//! `crates/yeban-engine/src/level.rs`（同一仓库、同一作者、同一 GPL-3.0-only 许可，
//! 不是第三方代码）。搬迁纪律是**只搬家、不改行为**：数值语义逐位不变，
//! 证据是 [`tests::frozen_pre_hoist_table`] 里冻结的 285 个 `f32` 位模式
//! （搬迁前实测，搬迁后必须仍然通过）。engine 侧只剩 `pub use`，没有第二份实现。
//!
//! ## 口径（这就是判据要钉住的东西）
//!
//! | 量 | 定义 | 单位 |
//! | :--- | :--- | :--- |
//! | 峰值 `peak` | 本量子内 `max abs(x)`，样本先经 [`sanitize_sample`] | 线性幅度（1.0 = 0 dBFS） |
//! | 峰值保持 `peak_hold` | `max(peak, peak_hold × release)`，`release` = 每量子乘子 | 线性幅度 |
//! | RMS `rms` | `sqrt(mean(x²))`，本量子独立计算（不跨量子） | 线性幅度 |
//! | 平滑 RMS `rms_smoothed` | 对**均方**做一阶低通后开方：`ms ← c·ms + (1-c)·mean(x²)` | 线性幅度 |
//! | dBFS | `20·log10(幅度)`；`幅度 ≤ 0` ⇒ 负无穷 | dBFS |
//! | 真峰值 `true_peak` | 4× 过采样后的 `max abs`（见 [`TruePeakDetector`]） | 线性幅度 |
//!
//! **时间常数（默认值，见常量）**：峰值释放 **20 dB/s**（每秒恰好降 20 dB）；
//! 平滑 RMS 一阶低通时间常数 **τ = 300 ms**。两者都按**每量子**折算，
//! 折算基准为 `quanta_per_second = sample_rate / block_frames`
//! （规范默认 48 000 / 128 = 375 Hz）。快照切换时重算一次，见 `yeban-engine` 的 `rt`。
//!
//! **为什么是 20 dB/s / 300 ms**：20 dB/s 是业界峰值表的常见回落速率
//! （1 秒回落一个数量级，既不会"钉死"也不会闪得看不清）；300 ms 接近 VU 的
//! 积分观感，用于 RMS 平滑。它们是**选择**而不是规范硬性数字，因此以常量 +
//! 文档的形式公开，允许后续线按 UI 观感调整（调整必须同步改本表的判据）。
//!
//! ## 输入钳位（数值卫生，不是去爆音）
//!
//! 实时路径可能拿到 `NaN` / `±∞`（数值爆炸、未初始化内存读入、上游 bug）。
//! 若原样进入电平，UI 曲线会被 `NaN` 永久污染（`NaN` 参与比较恒为假，
//! 峰值保持会卡死在 `NaN`）。因此：
//!
//! - `NaN` → `0.0`（当作静音）；
//! - `±∞` 与超过 [`MAX_LINEAR_MAGNITUDE`] 的有限值 → 钳到 `±MAX_LINEAR_MAGNITUDE`
//!   （4× 满量程 = +24.08 dBFS）；
//! - 结果保证是**有限**数，[`LevelReading::is_sane`] 可判。
//!
//! 这是**输入侧的数值卫生**，不是 [ARCH-DSP-001] 的语音偷取淡出/参数平滑
//! ——后者属于声部合成切片（见本 crate 的 [`crate::smoothing`] / [`crate::loop_window`]）。
//!
//! ## 与 [`crate::math`] 的 dB 换算的关系
//!
//! [`dbfs`] 是"幅度 → dBFS"的**计量**方向（`≤ 0` ⇒ 负无穷，便于 UI 画柱）；
//! [`crate::math::gain_to_db`] / [`crate::math::db_to_gain`] 是**增益**方向的互转
//! （−120 dB 以下 ⇒ 0，避免次正规数）。两者边界语义不同，因此并存而不是互相包装。

/// 静音下限（dBFS）。UI 需要有限值做柱高映射时用它替代负无穷。
pub const SILENCE_FLOOR_DBFS: f32 = -120.0;

/// 线性幅度上限（4× 满量程 = +24.08 dBFS）。`NaN`/`±∞` 与越界有限值都被钳到这里。
pub const MAX_LINEAR_MAGNITUDE: f32 = 16.0;

/// 默认峰值释放速率（dB/s）：每秒回落 20 dB。
pub const DEFAULT_PEAK_DECAY_DB_PER_SEC: f32 = 20.0;

/// 默认平滑 RMS 一阶低通时间常数（秒）。
pub const DEFAULT_RMS_TIME_CONSTANT_SEC: f32 = 0.3;

/// 默认折算基准（量子/秒）：48 000 Hz ÷ 128 帧 = 375 Hz [ARCH-DET-001]。
pub const DEFAULT_QUANTA_PER_SECOND: f32 = 375.0;

/// 把单个样本钳到有限、有界的线性幅度（**不做任何分配**）。
///
/// `NaN` ⇒ `0.0`；`±∞` 与越界值 ⇒ `±` [`MAX_LINEAR_MAGNITUDE`]。
#[must_use]
pub fn sanitize_sample(sample: f32) -> f32 {
    if sample.is_nan() {
        0.0
    } else if sample.is_infinite() {
        if sample > 0.0 {
            MAX_LINEAR_MAGNITUDE
        } else {
            -MAX_LINEAR_MAGNITUDE
        }
    } else {
        sample.clamp(-MAX_LINEAR_MAGNITUDE, MAX_LINEAR_MAGNITUDE)
    }
}

/// 线性幅度 → dBFS。`幅度 ≤ 0`（含 `NaN`）返回负无穷（**不是** `NaN`）。
#[must_use]
pub fn dbfs(amplitude: f32) -> f32 {
    if amplitude > 0.0 {
        20.0 * amplitude.log10()
    } else {
        f32::NEG_INFINITY
    }
}

/// 线性幅度 → dBFS，并按下限钳位（UI 柱高用）。
#[must_use]
pub fn dbfs_clamped(amplitude: f32, floor_dbfs: f32) -> f32 {
    let value = dbfs(amplitude);
    if value < floor_dbfs {
        floor_dbfs
    } else {
        value
    }
}

/// "取最新"判据：候选帧的量子序号是否**不早于**已持有的值。
///
/// `>=` 而不是 `>`：同一量子的重复投递是幂等的，直接覆盖不会引入回退，
/// 而 `>` 会让"同一量子内先到的帧"永远无法被修正。
#[must_use]
pub const fn supersedes(candidate_quantum: u64, held_quantum: u64) -> bool {
    candidate_quantum >= held_quantum
}

/// 一个量子内的电平原语读数（线性幅度，全部有限）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelReading {
    /// 本量子瞬时峰值。
    pub peak: f32,
    /// 峰值保持（带指数释放）。
    pub peak_hold: f32,
    /// 本量子 RMS。
    pub rms: f32,
    /// 平滑 RMS（均方做一阶低通后开方）。
    pub rms_smoothed: f32,
}

impl LevelReading {
    /// 全静音读数（四项都是 `0.0`，**不是** `NaN`）。
    #[must_use]
    pub const fn silence() -> Self {
        Self {
            peak: 0.0,
            peak_hold: 0.0,
            rms: 0.0,
            rms_smoothed: 0.0,
        }
    }

    /// 四项是否都是有限数（`NaN`/`±∞` 一律为假）。
    #[must_use]
    pub fn is_sane(&self) -> bool {
        self.peak.is_finite()
            && self.peak_hold.is_finite()
            && self.rms.is_finite()
            && self.rms_smoothed.is_finite()
    }

    /// 峰值 dBFS（`peak ≤ 0` ⇒ 负无穷）。
    #[must_use]
    pub fn peak_dbfs(&self) -> f32 {
        dbfs(self.peak)
    }

    /// 峰值保持 dBFS。
    #[must_use]
    pub fn peak_hold_dbfs(&self) -> f32 {
        dbfs(self.peak_hold)
    }

    /// 平滑 RMS 的 dBFS。
    #[must_use]
    pub fn rms_dbfs(&self) -> f32 {
        dbfs(self.rms_smoothed)
    }
}

/// 单节点电平检测器：**有状态**的峰值保持 + 平滑 RMS（零分配、零锁、零 I/O）。
///
/// 状态只有一个量子深度的推进：每个量子调一次
/// [`analyze`](Self::analyze) / [`analyze_stereo`](Self::analyze_stereo)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelDetector {
    /// 每量子峰值释放乘子（0..=1）。
    peak_release: f32,
    /// 每量子均方一阶低通系数（0..=1）。
    rms_coeff: f32,
    /// 峰值保持状态（线性）。
    peak_hold: f32,
    /// 平滑均方状态（线性平方域）。
    mean_square: f32,
}

impl Default for LevelDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelDetector {
    /// **无弹道**检测器：峰值保持只升不降，平滑 RMS 等于本量子块 RMS。
    ///
    /// 用途：离线分析/测试里想要"本块读数"而不想要历史；以及定长数组的 `const` 初值
    /// （未激活槽）。它是**合法**配置，不是占位魔法：`peak_release = 1`（不释放）、
    /// `rms_coeff = 0`（不平滑）。
    #[must_use]
    pub const fn silent() -> Self {
        Self {
            peak_release: 1.0,
            rms_coeff: 0.0,
            peak_hold: 0.0,
            mean_square: 0.0,
        }
    }

    /// 按默认口径（375 量子/s、20 dB/s、τ=300 ms）新建。
    #[must_use]
    pub fn new() -> Self {
        Self::with_ballistics(
            DEFAULT_QUANTA_PER_SECOND,
            DEFAULT_PEAK_DECAY_DB_PER_SEC,
            DEFAULT_RMS_TIME_CONSTANT_SEC,
        )
    }

    /// 按显式弹道参数新建。
    ///
    /// - `quanta_per_second` 非法（非有限或 ≤ 0）⇒ 退回 [`DEFAULT_QUANTA_PER_SECOND`]；
    /// - `peak_decay_db_per_second < 0` ⇒ 当作 0（不释放）；
    /// - `rms_time_constant_seconds ≤ 0` ⇒ 当作一量子（系数 0，即不平滑）。
    #[must_use]
    pub fn with_ballistics(
        quanta_per_second: f32,
        peak_decay_db_per_second: f32,
        rms_time_constant_seconds: f32,
    ) -> Self {
        let mut detector = Self {
            peak_release: 1.0,
            rms_coeff: 0.0,
            peak_hold: 0.0,
            mean_square: 0.0,
        };
        detector.set_ballistics(
            quanta_per_second,
            peak_decay_db_per_second,
            rms_time_constant_seconds,
        );
        detector
    }

    /// 重设弹道系数（**保留**当前电平状态）。快照切换时调用一次。
    ///
    /// 内部只做 `powf`/`exp`，不分配、不加锁、不阻塞 —— 可在实时路径调用。
    pub fn set_ballistics(
        &mut self,
        quanta_per_second: f32,
        peak_decay_db_per_second: f32,
        rms_time_constant_seconds: f32,
    ) {
        let qps = if quanta_per_second.is_finite() && quanta_per_second > 0.0 {
            quanta_per_second
        } else {
            DEFAULT_QUANTA_PER_SECOND
        };
        let decay = if peak_decay_db_per_second.is_finite() {
            peak_decay_db_per_second.max(0.0)
        } else {
            DEFAULT_PEAK_DECAY_DB_PER_SEC
        };
        let tau = if rms_time_constant_seconds.is_finite() && rms_time_constant_seconds > 0.0 {
            rms_time_constant_seconds
        } else {
            1.0 / qps
        };
        // 1 秒后的幅度乘子 = 10^(-decay/20) ⇒ 每量子乘子取 qps 次根。
        self.peak_release = 10f32.powf(-decay / 20.0 / qps).clamp(0.0, 1.0);
        // 一阶低通：y ← c·y + (1-c)·x，c = exp(-1/(τ·qps))。
        self.rms_coeff = (-1.0 / (tau * qps)).exp().clamp(0.0, 1.0);
    }

    /// 按 `sample_rate / block_frames` 重设弹道（沿用默认 dB/s 与 τ）。
    pub fn set_quanta_per_second(&mut self, quanta_per_second: f32) {
        self.set_ballistics(
            quanta_per_second,
            DEFAULT_PEAK_DECAY_DB_PER_SEC,
            DEFAULT_RMS_TIME_CONSTANT_SEC,
        );
    }

    /// 清空电平状态（系数保留）。
    pub fn reset(&mut self) {
        self.peak_hold = 0.0;
        self.mean_square = 0.0;
    }

    /// 当前峰值保持（线性）。
    #[must_use]
    pub const fn peak_hold(&self) -> f32 {
        self.peak_hold
    }

    /// 当前平滑均方（线性平方域）。
    #[must_use]
    pub const fn mean_square(&self) -> f32 {
        self.mean_square
    }

    /// 分析一个**单声道**量子。
    ///
    /// 空切片等价于静音块：峰值 0、RMS 0，但峰值保持与平滑 RMS 仍在衰减
    /// （这正是"没有信号时表针要落下来"的语义）。
    pub fn analyze(&mut self, samples: &[f32]) -> LevelReading {
        let mut peak = 0.0f32;
        let mut sum_squares = 0.0f64;
        for &raw in samples {
            let sample = sanitize_sample(raw);
            let magnitude = sample.abs();
            if magnitude > peak {
                peak = magnitude;
            }
            sum_squares += f64::from(sample) * f64::from(sample);
        }
        let mean_square = mean_of_squares(sum_squares, samples.len());
        self.commit(peak, mean_square)
    }

    /// 分析一个**立体声联动**量子：峰值取两声道最大绝对值，
    /// 均方按两声道平均（每声道各计一次分母）。
    ///
    /// 长度不等时按较短者工作（实时尾块可能出现），**不 panic**。
    pub fn analyze_stereo(&mut self, left: &[f32], right: &[f32]) -> LevelReading {
        let frames = left.len().min(right.len());
        let mut peak = 0.0f32;
        let mut sum_squares = 0.0f64;
        for index in 0..frames {
            let l = sanitize_sample(left[index]);
            let r = sanitize_sample(right[index]);
            let magnitude = l.abs().max(r.abs());
            if magnitude > peak {
                peak = magnitude;
            }
            sum_squares += f64::from(l) * f64::from(l) + f64::from(r) * f64::from(r);
        }
        let mean_square = mean_of_squares(sum_squares, frames.saturating_mul(2));
        self.commit(peak, mean_square)
    }

    /// 把本量子的 `peak` 与 `mean_square` 推进状态机并产出读数。
    fn commit(&mut self, peak: f32, mean_square: f32) -> LevelReading {
        self.peak_hold = (self.peak_hold * self.peak_release).max(peak);
        self.mean_square = self.rms_coeff * self.mean_square + (1.0 - self.rms_coeff) * mean_square;
        if !self.peak_hold.is_finite() {
            self.peak_hold = 0.0;
        }
        if !self.mean_square.is_finite() {
            self.mean_square = 0.0;
        }
        LevelReading {
            peak,
            peak_hold: self.peak_hold,
            rms: mean_square.max(0.0).sqrt(),
            rms_smoothed: self.mean_square.max(0.0).sqrt(),
        }
    }
}

/// 均方：`sum_squares / count`；`count == 0` 时返回 `0.0`（**不是** `0/0 = NaN`）。
fn mean_of_squares(sum_squares: f64, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        (sum_squares / count as f64) as f32
    }
}

// ---------------------------------------------------------------------------
// 真峰值（true peak）：4× 过采样
// ---------------------------------------------------------------------------

/// 真峰值检测器的过采样相位数（= 过采样倍数）。
pub const TRUE_PEAK_PHASES: usize = 4;

/// 真峰值检测器每个相位的抽头数（总原型长度 = `TRUE_PEAK_PHASES * TRUE_PEAK_TAPS`）。
pub const TRUE_PEAK_TAPS: usize = 16;

/// 真峰值检测器引入的延迟（**基础采样率**样本）。
///
/// 因果 FIR 不可能零延迟；真峰值是**幅度**量，因此这个延迟不影响读数，
/// 只影响"读到的是哪一段时间"（供需要对齐的调用方参考）。
pub const TRUE_PEAK_LATENCY_SAMPLES: usize = 8;

/// 4× 多相插值核：`TRUE_PEAK_KERNEL[phase][m]` 乘 `x[i − m]`。
///
/// 生成口径：理想插值核 `sinc(m − 8 + phase/4)` 乘 Kaiser 窗（β = 8，长度 16），
/// 再**逐相位归一化到直流增益 1**（于是直流/低频近位透明）。系数按**最短往返表示**
/// 写成 `f32` 字面量（与 `crate::oversample` 同一纪律：数值不变，只是不再触发
/// `clippy::excessive_precision`）。
///
/// `phase == 0` 恰好退化为 `δ(m − 8)`（整数偏移处的 sinc 是 delta），
/// 因此过采样输出**天然包含原始样本** ⇒ 真峰值 ≥ 采样峰值恒成立。
///
/// 判据 [`tests::true_peak_kernel_is_coherent`] 钉住：每个相位直流增益为 1、
/// 相位 0 是 delta、以及在基础 Nyquist 以下每个相位的频响幅度 ≈ 1（±5e-3）。
#[rustfmt::skip]
const TRUE_PEAK_KERNEL: [[f32; TRUE_PEAK_TAPS]; TRUE_PEAK_PHASES] = [
    // phase 0
    [
        0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0,
    ],
    // phase 1
    [
        0.0, 0.00087815203, -0.0037137852, 0.010788576,
        -0.025480973, 0.053615782, -0.11102466, 0.29632586,
        0.8889776, -0.15543453, 0.065530404, -0.029401124,
        0.01205782, -0.0040674787, 0.0009484042, 0.0,
    ],
    // phase 2
    [
        0.0, 0.0012842105, -0.005467616, 0.016036933,
        -0.038446367, 0.08305417, -0.18240735, 0.62594604,
        0.62594604, -0.18240735, 0.08305417, -0.038446367,
        0.016036933, -0.005467616, 0.0012842105, 0.0,
    ],
    // phase 3
    [
        0.0, 0.0009484042, -0.0040674787, 0.01205782,
        -0.029401124, 0.065530404, -0.15543453, 0.8889776,
        0.29632586, -0.11102466, 0.053615782, -0.025480973,
        0.010788576, -0.0037137852, 0.00087815203, 0.0,
    ],
];

/// 4× 过采样真峰值检测器（[ARCH-UI-002] 的"真峰值"字面要求）。
///
/// **为什么需要它**：样本峰值会漏掉采样点之间的过冲（intersample peak）。
/// 一个在 `fs/4` 上相位偏移 45° 的满幅正弦，样本全是 `±0.7071`
/// （采样峰值 −3.01 dBFS），而它真实的连续峰值是 1.0（0 dBFS）——
/// 只按样本峰值做限制器就会在真峰值上失真/超限。
///
/// **口径**：4× 多相插值（[`TRUE_PEAK_KERNEL`]，BS.1770 Annex 2 同族的做法），
/// 取插值后样本与原始样本的 `max abs`。
///
/// **诚实的边界**（不要误读为"精确真峰值"）：
///
/// 1. 4× 是**估计**：真峰值的真正峰值可能落在 4× 网格之间。例如 `f = 0.4·fs`
///    的满幅正弦，连续峰值处 `t = 5/8` 不在 `1/4` 网格上，4× 会读成
///    `cos(π/10) = 0.9511`（−0.44 dB）—— 这是 4× 方案的**固有**欠读，
///    提高倍数（8×/16×）才能收紧，本最小实现不做；
/// 2. 输入经 [`sanitize_sample`]（`NaN → 0`、`±∞`/越界 → `±16`）；
/// 3. 通道口径沿用调用方：本类型只处理**给定的一路样本流**，
///    多声道的联动/求和由调用方决定（与 [`LevelDetector`] 的分工一致）。
///
/// **零分配**：状态是定长数组，`process` 内只有乘加与移位 [ARCH-RT-001]。
///
/// ```
/// use yeban_dsp::meter::TruePeakDetector;
///
/// // fs/4 上相位偏移 45° 的满幅正弦: 样本峰值只有 0.7071, 真峰值接近 1.0。
/// let mut detector = TruePeakDetector::new();
/// let samples: Vec<f32> = (0..512)
///     .map(|i| (std::f32::consts::FRAC_PI_4 + std::f32::consts::FRAC_PI_2 * (i % 4) as f32).sin())
///     .collect();
/// let sample_peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
/// let true_peak = detector.process(&samples);
/// assert!(sample_peak < 0.72);
/// assert!(true_peak > 0.999, "真峰值应接近 1.0, 实际 {true_peak}");
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TruePeakDetector {
    /// 最近 [`TRUE_PEAK_TAPS`] 个样本，`window[0]` 是最新。
    window: [f32; TRUE_PEAK_TAPS],
    /// 自上次 [`reset`](Self::reset) 以来的真峰值。
    peak: f32,
}

impl Default for TruePeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl TruePeakDetector {
    /// 新建（历史归零）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            window: [0.0; TRUE_PEAK_TAPS],
            peak: 0.0,
        }
    }

    /// 丢弃历史与峰值（换流/关流时用）。
    pub fn reset(&mut self) {
        self.window = [0.0; TRUE_PEAK_TAPS];
        self.peak = 0.0;
    }

    /// 本实例的延迟（基础采样率样本）。
    #[must_use]
    pub const fn latency_samples(&self) -> usize {
        TRUE_PEAK_LATENCY_SAMPLES
    }

    /// 自上次 [`reset`](Self::reset) 以来的真峰值。
    #[must_use]
    pub const fn true_peak(&self) -> f32 {
        self.peak
    }

    /// 喂入一段样本，返回**本段**的真峰值（并把全局峰值并进 [`true_peak`](Self::true_peak)）。
    ///
    /// 空切片返回 `0.0` 且不改状态。`NaN`/`±∞` 经 [`sanitize_sample`] 钳位，
    /// 因此返回的永远是有限数。
    pub fn process(&mut self, samples: &[f32]) -> f32 {
        let mut block_peak = 0.0f32;
        for &raw in samples {
            let sample = sanitize_sample(raw);
            // 移位窗口: window[0] 是最新样本。
            let mut index = TRUE_PEAK_TAPS - 1;
            while index > 0 {
                self.window[index] = self.window[index - 1];
                index -= 1;
            }
            self.window[0] = sample;
            for kernel in &TRUE_PEAK_KERNEL {
                let mut acc = 0.0f32;
                for (tap, coefficient) in kernel.iter().enumerate() {
                    acc += coefficient * self.window[tap];
                }
                let magnitude = acc.abs();
                if magnitude > block_peak {
                    block_peak = magnitude;
                }
            }
        }
        if block_peak > self.peak {
            self.peak = block_peak;
        }
        block_peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无分配生成一段正弦（测试用；实时路径不生成信号，只测量）。
    fn sine(amplitude: f32, cycles: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let phase = cycles * std::f32::consts::TAU * index as f32 / len as f32;
                amplitude * phase.sin()
            })
            .collect()
    }

    #[test]
    fn full_scale_sine_peaks_at_zero_dbfs() {
        // 一个完整周期的采样数必须足够多, 峰值才能贴近 1.0: 4800 点覆盖 10 周期,
        // 理论峰值落在样本间的误差 < 1 %(≈0.09 dB), 因此容差取 0.05 dB 过严,
        // 取 0.1 dB 并附理由: sin 的离散采样峰值 = cos(π/N) ≈ 1 - 5e-6 ⇒ 实际误差远小于 0.1 dB。
        let samples = sine(1.0, 10.0, 4800);
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&samples);
        assert!(reading.is_sane());
        assert!((reading.peak - 1.0).abs() < 1e-3, "峰值应达满量程");
        assert!(
            reading.peak_dbfs().abs() < 0.1,
            "满幅正弦峰值应接近 0 dBFS, 实际 {}",
            reading.peak_dbfs()
        );
        // 正弦 RMS = 1/√2 ⇒ -3.01 dBFS(容差给 0.05 dB: 整数周期离散 RMS 误差 < 1e-3 dB)
        assert!(
            (reading.rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "正弦 RMS 应为 1/√2, 实际 {}",
            reading.rms
        );
        assert!(
            (dbfs(reading.rms) + 3.0103).abs() < 0.05,
            "正弦 RMS 应约 -3.01 dBFS, 实际 {}",
            dbfs(reading.rms)
        );
        // 平滑 RMS 在**第一个**量子必然远低于块 RMS(一阶低通从 0 起步), 这正是"平滑"的证据
        assert!(
            reading.rms_smoothed < reading.rms,
            "首块的平滑 RMS 必须低于块 RMS"
        );
    }

    #[test]
    fn half_scale_sine_is_minus_six_dbfs() {
        let samples = sine(0.5, 10.0, 4800);
        let reading = LevelDetector::new().analyze(&samples);
        assert!(
            (reading.peak_dbfs() + 6.0206).abs() < 0.1,
            "半幅峰值应约 -6.02 dBFS, 实际 {}",
            reading.peak_dbfs()
        );
    }

    #[test]
    fn zero_input_is_negative_infinity_or_floor_never_nan() {
        let silence = [0.0f32; 128];
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&silence);
        assert!(reading.is_sane(), "静音不得产生 NaN/Inf");
        assert_eq!(reading.peak, 0.0);
        assert_eq!(reading.rms, 0.0);
        assert_eq!(reading.peak_dbfs(), f32::NEG_INFINITY);
        assert_eq!(reading.rms_dbfs(), f32::NEG_INFINITY);
        assert_eq!(
            dbfs_clamped(reading.peak, SILENCE_FLOOR_DBFS),
            SILENCE_FLOOR_DBFS,
            "UI 侧应得到静音下限而不是 NaN"
        );
        // 空切片同理(0/0 必须是 0 而不是 NaN)
        let empty = detector.analyze(&[]);
        assert!(empty.is_sane());
        assert_eq!(empty.rms, 0.0);
        let empty_stereo = detector.analyze_stereo(&[], &[]);
        assert!(empty_stereo.is_sane());
        assert_eq!(empty_stereo.rms, 0.0);
    }

    #[test]
    fn levels_are_monotonic_in_amplitude() {
        let amplitudes = [0.001f32, 0.01, 0.1, 0.5, 1.0];
        let mut previous_peak = -1.0f32;
        let mut previous_rms = -1.0f32;
        let mut previous_db = f32::NEG_INFINITY;
        for amplitude in amplitudes {
            let samples = sine(amplitude, 10.0, 4800);
            // 每个幅度用**全新**检测器: 排除峰值保持/平滑的历史影响。
            let reading = LevelDetector::new().analyze(&samples);
            assert!(reading.peak > previous_peak, "峰值必须随幅度单调增");
            assert!(reading.rms > previous_rms, "RMS 必须随幅度单调增");
            assert!(reading.peak_dbfs() > previous_db, "dBFS 必须随幅度单调增");
            previous_peak = reading.peak;
            previous_rms = reading.rms;
            previous_db = reading.peak_dbfs();
        }
    }

    #[test]
    fn nan_and_infinity_inputs_never_produce_nan_levels() {
        let hostile = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0e30,
            -1.0e30,
            0.5,
            f32::NAN,
        ];
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&hostile);
        assert!(
            reading.is_sane(),
            "NaN/Inf 输入必须被钳位, 实际 {reading:?}"
        );
        assert!(
            reading.peak <= MAX_LINEAR_MAGNITUDE,
            "钳位后的峰值不得超过上限, 实际 {}",
            reading.peak
        );
        assert!(reading.peak_hold <= MAX_LINEAR_MAGNITUDE);
        assert!(reading.rms <= MAX_LINEAR_MAGNITUDE);
        assert!(reading.peak_dbfs().is_finite(), "有限幅度 ⇒ 有限 dBFS");

        // 全部是 NaN 的块 ⇒ 静音, 且状态不被污染(后续静音仍然 sane)
        let all_nan = [f32::NAN; 64];
        let nan_reading = detector.analyze(&all_nan);
        assert!(nan_reading.is_sane());
        assert_eq!(nan_reading.peak, 0.0);
        let after = detector.analyze(&[0.0f32; 64]);
        assert!(after.is_sane(), "NaN 之后仍必须 sane");
    }

    #[test]
    fn sanitize_clamps_and_never_returns_nan() {
        assert_eq!(sanitize_sample(f32::NAN), 0.0);
        assert_eq!(sanitize_sample(f32::INFINITY), MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(f32::NEG_INFINITY), -MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(1.0e30), MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(-1.0e30), -MAX_LINEAR_MAGNITUDE);
        assert_eq!(sanitize_sample(0.25), 0.25);
        assert!(sanitize_sample(f32::NAN).is_finite());
    }

    #[test]
    fn peak_hold_decays_by_the_documented_rate() {
        // 默认口径: 375 量子/s、20 dB/s ⇒ 375 个量子之后恰好降 20 dB。
        //
        // ⚠ 这里的 375 与 20 **写死**，不从常量读：判据若与常量同源，
        // 把常量从 20 改成 10 时判据会跟着"通过"（实测过一次，已修正）。
        const QPS: usize = 375;
        const DECAY_DB_PER_SEC: f32 = 20.0;
        let mut detector = LevelDetector::new();
        let first = detector.analyze(&[1.0f32; 64]);
        assert!((first.peak_hold - 1.0).abs() < 1e-6);

        let mut last = first;
        for _ in 0..QPS {
            last = detector.analyze(&[0.0f32; 64]);
        }
        // 起始幅度是 1.0(0 dBFS) ⇒ 回落量就是保持值的负 dBFS
        let decay_db = -last.peak_hold_dbfs();
        assert!(
            (decay_db - DECAY_DB_PER_SEC).abs() < 0.05,
            "1 秒应恰好回落 {DECAY_DB_PER_SEC} dB, 实际 {decay_db} dB"
        );
        // 半秒应恰好回落一半(线性 dB 释放 ⇒ 速率恒定)。
        let mut detector = LevelDetector::new();
        let _ = detector.analyze(&[1.0f32; 64]);
        let mut half = first;
        for _ in 0..(QPS / 2) {
            half = detector.analyze(&[0.0f32; 64]);
        }
        let half_decay = -half.peak_hold_dbfs();
        assert!(
            (half_decay - DECAY_DB_PER_SEC / 2.0).abs() < 0.05,
            "0.5 秒应回落 {} dB, 实际 {half_decay} dB",
            DECAY_DB_PER_SEC / 2.0
        );
        // 常量本身也必须还是 20(防止"判据写死、常量被改"的另一半)。
        assert_eq!(DEFAULT_PEAK_DECAY_DB_PER_SEC, DECAY_DB_PER_SEC);
        assert_eq!(DEFAULT_QUANTA_PER_SECOND, QPS as f32);
    }

    #[test]
    fn smoothed_rms_follows_the_documented_time_constant() {
        // 常数 0.5 输入下, 均方一阶低通在 τ 秒后达到目标的 1-1/e。
        // ⚠ 375 与 0.3 写死(不从常量读), 否则改常量会让判据跟着漂移。
        const QPS: f32 = 375.0;
        const TAU_SECONDS: f32 = 0.3;
        assert_eq!(DEFAULT_QUANTA_PER_SECOND, QPS);
        assert_eq!(DEFAULT_RMS_TIME_CONSTANT_SEC, TAU_SECONDS);
        let qps = QPS;
        let tau_quanta = (TAU_SECONDS * qps).round() as usize;
        let mut detector = LevelDetector::new();
        let mut last = LevelReading::silence();
        for _ in 0..tau_quanta {
            last = detector.analyze(&[0.5f32; 64]);
        }
        let expected = 0.5 * (1.0 - (-1.0f32).exp()).sqrt();
        assert!(
            (last.rms_smoothed - expected).abs() < 0.01,
            "τ 之后平滑 RMS 应约 {expected}, 实际 {}",
            last.rms_smoothed
        );

        // 充分长时间后收敛到块 RMS(0.5)
        for _ in 0..(tau_quanta * 20) {
            last = detector.analyze(&[0.5f32; 64]);
        }
        assert!(
            (last.rms_smoothed - 0.5).abs() < 0.005,
            "稳态应收敛到 0.5, 实际 {}",
            last.rms_smoothed
        );
    }

    #[test]
    fn stereo_reading_is_channel_linked() {
        let left = sine(1.0, 10.0, 4800);
        let right = vec![0.0f32; 4800];
        let mut detector = LevelDetector::new();
        let reading = detector.analyze_stereo(&left, &right);
        assert!(
            reading.peak_dbfs().abs() < 0.1,
            "单声道满幅时联动峰值应达 0 dBFS, 实际 {}",
            reading.peak_dbfs()
        );
        // 均方按两声道平均: 只有一个满幅声道 ⇒ RMS = √(1/2)·(1/√2) = 0.5
        assert!(
            (reading.rms - 0.5).abs() < 1e-3,
            "联动 RMS 应为 0.5, 实际 {}",
            reading.rms
        );

        // 长度不等(实时尾块)不 panic, 按较短者工作
        let short = detector.analyze_stereo(&[1.0, 1.0], &[1.0]);
        assert!(short.is_sane());
        assert_eq!(short.peak, 1.0);
    }

    #[test]
    fn empty_block_still_decays_hold_and_keeps_finite_rms() {
        let mut detector = LevelDetector::new();
        let loud = detector.analyze(&[1.0f32; 128]);
        assert!((loud.peak_hold - 1.0).abs() < 1e-6);
        let mut quiet = loud;
        for _ in 0..375 {
            quiet = detector.analyze(&[]);
        }
        assert!(quiet.is_sane());
        assert_eq!(quiet.peak, 0.0, "空块峰值必须是 0");
        assert!(
            quiet.peak_hold < 0.11,
            "空块也必须让保持值回落, 实际 {}",
            quiet.peak_hold
        );
        assert!(quiet.rms.is_finite() && quiet.rms_smoothed.is_finite());
    }

    #[test]
    fn supersedes_accepts_equal_and_newer_but_not_older() {
        assert!(supersedes(7, 7));
        assert!(supersedes(8, 7));
        assert!(!supersedes(6, 7));
        // 单调推进: 新值被接受之后, 旧值再也不能翻回来
        assert!(!supersedes(7, 8));
    }

    #[test]
    fn ballistics_reject_invalid_parameters_without_nan() {
        let mut detector = LevelDetector::with_ballistics(f32::NAN, -5.0, 0.0);
        let reading = detector.analyze(&[1.0f32; 32]);
        assert!(reading.is_sane(), "非法弹道参数必须退回默认而不是 NaN");
        assert!(reading.peak_hold <= 1.0);
        // 不释放(decay=0) ⇒ 保持值不下降
        let mut hold = LevelDetector::with_ballistics(375.0, 0.0, 0.3);
        let loud = hold.analyze(&[1.0f32; 32]);
        let later = hold.analyze(&[]);
        assert!((later.peak_hold - loud.peak_hold).abs() < 1e-6);
    }

    #[test]
    fn silent_detector_has_no_ballistics() {
        let mut detector = LevelDetector::silent();
        let first = detector.analyze(&[0.5f32; 32]);
        assert!((first.peak_hold - 0.5).abs() < 1e-6);
        assert!(
            (first.rms_smoothed - 0.5).abs() < 1e-6,
            "不平滑 ⇒ 等于块 RMS"
        );
        // 不释放: 静音块之后保持值不下降
        let later = detector.analyze(&[0.0f32; 32]);
        assert!((later.peak_hold - 0.5).abs() < 1e-6);
        assert_eq!(later.rms, 0.0);
        assert_eq!(later.rms_smoothed, 0.0, "均方系数 0 ⇒ 跟随当块");
    }

    #[test]
    fn reset_clears_state_but_keeps_ballistics() {
        let mut detector = LevelDetector::new();
        let _ = detector.analyze(&[1.0f32; 32]);
        assert!(detector.peak_hold() > 0.0);
        detector.reset();
        assert_eq!(detector.peak_hold(), 0.0);
        assert_eq!(detector.mean_square(), 0.0);
        let reading = detector.analyze(&[]);
        assert!(reading.is_sane());
        assert_eq!(reading.peak_hold, 0.0);
    }

    #[test]
    fn full_scale_square_wave_rms_is_exactly_one() {
        // 满幅方波是"RMS = 1"的解析基准(每样本 ±1 ⇒ mean(x²) = 1)。
        let block: Vec<f32> = (0..512)
            .map(|index| if index % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let reading = LevelDetector::new().analyze(&block);
        assert_eq!(reading.peak, 1.0);
        assert_eq!(reading.rms, 1.0, "满幅方波 RMS 必须恰好是 1.0");
        assert_eq!(dbfs(reading.rms), 0.0, "RMS = 1 ⇒ 0 dBFS");
        // 首块的**平滑** RMS 从 0 起步, 因此必然低于块 RMS(这是"平滑"的直接证据)。
        assert!(reading.rms_smoothed < reading.rms);
    }

    #[test]
    fn dbfs_is_the_exact_inverse_of_the_documented_definition() {
        // 定义就是 20·log10(幅度); 钉住几个解析点与边界。
        assert_eq!(dbfs(1.0), 0.0);
        assert_eq!(dbfs(0.0), f32::NEG_INFINITY);
        assert_eq!(dbfs(-0.0), f32::NEG_INFINITY);
        assert_eq!(dbfs(-1.0), f32::NEG_INFINITY, "负幅度不是有效幅度");
        assert_eq!(dbfs(f32::NAN), f32::NEG_INFINITY, "NaN 不得产生 NaN dBFS");
        assert!((dbfs(0.5) + 6.020_6).abs() < 1e-3);
        assert!((dbfs(2.0) - 6.020_6).abs() < 1e-3);
        // 钳位边界: 1e-9 ⇒ -180 dBFS, 低于下限 ⇒ 被抬到 -120 dBFS。
        assert!((dbfs(1.0e-9) + 180.0).abs() < 1e-3);
        assert_eq!(dbfs_clamped(1.0e-9, SILENCE_FLOOR_DBFS), SILENCE_FLOOR_DBFS);
        // 恰好等于下限的读数不得被抬高(边界不抖动)。
        let at_floor = crate::math::db_to_gain(SILENCE_FLOOR_DBFS);
        assert_eq!(
            dbfs_clamped(at_floor, SILENCE_FLOOR_DBFS),
            SILENCE_FLOOR_DBFS
        );
        // 上限方向不做钳制: 16.0 ⇒ +24.08 dBFS 原样返回。
        assert!((dbfs_clamped(MAX_LINEAR_MAGNITUDE, SILENCE_FLOOR_DBFS) - 24.082_4).abs() < 1e-3);
    }

    // -----------------------------------------------------------------------
    // 真峰值
    // -----------------------------------------------------------------------

    /// 判据：**采样点之间的过冲必须被看见**。
    ///
    /// `fs/4` 上相位偏移 45° 的满幅正弦，样本恒为 `±√2/2 = 0.7071`
    /// （采样峰值 −3.01 dBFS），而连续峰值是 1.0。样本峰值表会漏掉 3 dB，
    /// 真峰值表必须抓到它。
    ///
    /// 注入：让 `process` 只返回样本峰值（去掉多相插值循环）⇒ 本判据变红。
    #[test]
    fn true_peak_sees_the_intersample_overshoot() {
        // 相位按 i % 4 归约, 避免大自变量的 f32 sin 精度漂移。
        let samples: Vec<f32> = (0..1024)
            .map(|i| {
                let phase =
                    std::f32::consts::FRAC_PI_4 + std::f32::consts::FRAC_PI_2 * (i % 4) as f32;
                phase.sin()
            })
            .collect();
        let sample_peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            (sample_peak - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
            "构造前提: 采样峰值应恰为 √2/2, 实际 {sample_peak}"
        );
        let mut detector = TruePeakDetector::new();
        let block_peak = detector.process(&samples);
        assert!(
            block_peak > 0.999,
            "4× 真峰值应抓到 1.0 的过冲, 实际 {block_peak} ({:.3} dBFS)",
            dbfs(block_peak)
        );
        assert!(
            block_peak >= sample_peak,
            "真峰值必须 ≥ 采样峰值 (真峰值包含原始样本)"
        );
        assert_eq!(detector.true_peak(), block_peak);
        assert_eq!(detector.process(&[]), 0.0, "空切片返回 0");
        assert_eq!(detector.true_peak(), block_peak, "空切片不改全局峰值");
    }

    /// 判据：直流与低频**近位**透明（每相位直流增益归一化为 1）。
    #[test]
    fn true_peak_is_transparent_at_dc_and_low_frequency() {
        // ⚠ FIR 从零状态起步时, 直流**阶跃**会被窗化 sinc 的 Gibbs 过冲读到
        // (实测 0.5 → 0.563)。这是本方案的固有瞬态, 不是稳态失准: 先喂满窗口
        // 再测量, 稳态必须逐位透明。
        let mut detector = TruePeakDetector::new();
        let _warm_up = detector.process(&[0.5f32; 64]);
        let dc = detector.process(&[0.5f32; 256]);
        assert!(
            (dc - 0.5).abs() < 1e-6,
            "稳态直流 0.5 的真峰值必须是 0.5, 实际 {dc}"
        );
        // 低频正弦: 稳态峰值就是幅度(容差含 4× 网格与窗函数的残余)。
        let low = sine(0.25, 3.0, 4096);
        let mut detector = TruePeakDetector::new();
        let _warm_up = detector.process(&low[..256]);
        let peak = detector.process(&low[256..]);
        assert!(
            (peak - 0.25).abs() < 2e-3,
            "低频正弦真峰值应约 0.25, 实际 {peak}"
        );
    }

    /// 判据：4× 方案在 `f = 0.4·fs` 处的**固有欠读**被如实钉住
    /// （这是方案边界，不是 bug；见 [`TruePeakDetector`] 的文档第 1 条）。
    #[test]
    fn four_times_oversampling_underreads_at_four_tenths_nyquist_as_documented() {
        let samples: Vec<f32> = (0..8192)
            .map(|i| (std::f32::consts::TAU * 0.4 * i as f32).sin())
            .collect();
        let mut detector = TruePeakDetector::new();
        let peak = detector.process(&samples);
        // 连续峰值在 t = 5/8, 不在 1/4 网格上 ⇒ 4× 读到 cos(π/10)。
        let expected = (std::f32::consts::PI / 10.0).cos();
        assert!(
            (peak - expected).abs() < 2e-3,
            "4× 在 f=0.4·fs 应读到 {expected}, 实际 {peak}"
        );
        assert!(peak > 0.9, "欠读不等于失效: 仍应接近 1.0");
    }

    /// 判据：真峰值 ≥ 采样峰值（含确定性的伪随机样本），且对敌对输入有限。
    #[test]
    fn true_peak_dominates_sample_peak_and_sanitizes_hostile_input() {
        let mut state = 0x1234_5678u32;
        let mut samples = Vec::new();
        for _ in 0..4096 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            samples.push((state as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        let sample_peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let mut detector = TruePeakDetector::new();
        let true_peak = detector.process(&samples);
        assert!(
            true_peak >= sample_peak && true_peak > sample_peak,
            "宽带信号上真峰值应严格大于采样峰值: {true_peak} vs {sample_peak}"
        );
        assert!(true_peak < 2.0, "窗化插值不应产生离谱过冲: {true_peak}");

        let hostile = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0e30, 0.5];
        let mut detector = TruePeakDetector::new();
        let peak = detector.process(&hostile);
        assert!(peak.is_finite(), "敌对输入不得产生 NaN/Inf");
        assert!(peak <= MAX_LINEAR_MAGNITUDE);
    }

    /// 判据：核表自洽——每相位直流增益 1、相位 0 是 delta、Nyquist 以下幅度 ≈ 1。
    ///
    /// 注入：改表里任意一个系数（哪怕一位小数）⇒ 直流增益或频响断言变红。
    #[test]
    fn true_peak_kernel_is_coherent() {
        // 相位 0 必须是 δ(m − TRUE_PEAK_LATENCY_SAMPLES)。
        for (m, value) in TRUE_PEAK_KERNEL[0].iter().enumerate() {
            let expected = if m == TRUE_PEAK_LATENCY_SAMPLES {
                1.0
            } else {
                0.0
            };
            assert_eq!(*value, expected, "相位 0 在 m={m} 处不是 delta");
        }
        for (phase, kernel) in TRUE_PEAK_KERNEL.iter().enumerate() {
            let dc: f64 = kernel.iter().map(|v| f64::from(*v)).sum();
            assert!(
                (dc - 1.0).abs() < 1e-6,
                "相位 {phase} 的直流增益是 {dc}, 不是 1"
            );
            // 基础 Nyquist 以下若干频点的幅度响应必须 ≈ 1。
            for frequency in [0.01f64, 0.05, 0.1, 0.2, 0.25, 0.3] {
                let omega = -std::f64::consts::TAU * frequency;
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (m, value) in kernel.iter().enumerate() {
                    let angle = omega * m as f64;
                    re += f64::from(*value) * angle.cos();
                    im += f64::from(*value) * angle.sin();
                }
                let magnitude = (re * re + im * im).sqrt();
                assert!(
                    (magnitude - 1.0).abs() < 5e-3,
                    "相位 {phase} 在 f={frequency} 处幅度是 {magnitude}"
                );
            }
        }
    }

    /// 判据：真峰值检测器`process` 是确定性的（同输入同输出位模式）。
    #[test]
    fn true_peak_is_bit_deterministic() {
        let block: Vec<f32> = (0..777)
            .map(|i| (0.3 * i as f32).sin() * 0.9 + 0.05)
            .collect();
        let mut a = TruePeakDetector::new();
        let mut b = TruePeakDetector::new();
        assert_eq!(a.process(&block).to_bits(), b.process(&block).to_bits());
        assert_eq!(a.process(&block).to_bits(), b.process(&block).to_bits());
        assert_eq!(a.true_peak().to_bits(), b.true_peak().to_bits());
        // reset 之后必须逐位回到初态。
        a.reset();
        assert_eq!(a.true_peak(), 0.0);
        assert_eq!(a, TruePeakDetector::new());
    }

    // -----------------------------------------------------------------------
    // 搬迁的逐位不变证据
    // -----------------------------------------------------------------------

    /// 确定性 xorshift：给检测器喂"不像正弦"的样本来覆盖 RMS/峰值/释放的合流路径。
    struct XorShift(u32);

    impl XorShift {
        fn next_sample(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
    }

    /// 搬迁前的**同一组输入**脚本（与 `crates/yeban-engine/tests/level_bit_table.rs` 逐行一致）。
    ///
    /// 它只通过本模块的公共面取值，因此搬迁前后跑的是同一段测量代码。
    fn probe() -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = Vec::new();
        macro_rules! rec {
            ($name:expr, $v:expr) => {
                out.push((String::from($name), ($v).to_bits()))
            };
        }
        macro_rules! req {
            ($name:expr, $v:expr) => {
                out.push(($name.to_string(), ($v).to_bits()))
            };
        }
        macro_rules! reading {
            ($prefix:expr, $r:expr) => {{
                let r: LevelReading = $r;
                let p: String = $prefix.to_string();
                req!(format!("{p}.peak"), r.peak);
                req!(format!("{p}.peak_hold"), r.peak_hold);
                req!(format!("{p}.rms"), r.rms);
                req!(format!("{p}.rms_smoothed"), r.rms_smoothed);
                req!(format!("{p}.peak_dbfs"), r.peak_dbfs());
                req!(format!("{p}.peak_hold_dbfs"), r.peak_hold_dbfs());
                req!(format!("{p}.rms_dbfs"), r.rms_dbfs());
                req!(
                    format!("{p}.is_sane"),
                    if r.is_sane() { 1.0f32 } else { 0.0f32 }
                );
            }};
        }

        for (name, x) in [
            ("san.nan", f32::NAN),
            ("san.pinf", f32::INFINITY),
            ("san.ninf", f32::NEG_INFINITY),
            ("san.1e30", 1.0e30),
            ("san.-1e30", -1.0e30),
            ("san.0.25", 0.25),
            ("san.16.0", 16.0),
            ("san.-16.0", -16.0),
            ("san.16.000001", 16.000_001),
            ("san.-16.000001", -16.000_001),
            ("san.neg_zero", -0.0),
            ("san.zero", 0.0),
            ("san.one", 1.0),
            ("san.-one", -1.0),
            ("san.20", 20.0),
            ("san.-20", -20.0),
            ("san.min_positive", f32::MIN_POSITIVE),
            ("san.max", f32::MAX),
            ("san.min", f32::MIN),
            ("san.subnormal", 1.0e-45),
        ] {
            rec!(name, sanitize_sample(x));
        }

        for (name, x) in [
            ("db.1.0", 1.0f32),
            ("db.0.5", 0.5),
            ("db.0.25", 0.25),
            ("db.0.1", 0.1),
            ("db.2.0", 2.0),
            ("db.16.0", 16.0),
            ("db.0", 0.0),
            ("db.neg_zero", -0.0),
            ("db.-1", -1.0),
            ("db.nan", f32::NAN),
            ("db.inf", f32::INFINITY),
            ("db.ninf", f32::NEG_INFINITY),
            ("db.1e-30", 1.0e-30),
            ("db.min_positive", f32::MIN_POSITIVE),
            ("db.fract_1_sqrt2", std::f32::consts::FRAC_1_SQRT_2),
        ] {
            rec!(name, dbfs(x));
        }

        for (name, x) in [
            ("dbc.0", 0.0f32),
            ("dbc.1e-9", 1.0e-9),
            ("dbc.1.0", 1.0),
            ("dbc.nan", f32::NAN),
            ("dbc.16.0", 16.0),
        ] {
            rec!(name, dbfs_clamped(x, -120.0));
        }

        for (name, candidate, held) in [
            ("sup.7_7", 7u64, 7u64),
            ("sup.8_7", 8, 7),
            ("sup.6_7", 6, 7),
            ("sup.0_0", 0, 0),
            ("sup.max_max", u64::MAX, u64::MAX),
            ("sup.0_max", 0, u64::MAX),
        ] {
            rec!(
                name,
                if supersedes(candidate, held) {
                    1.0f32
                } else {
                    0.0f32
                }
            );
        }

        rec!("const.qps", DEFAULT_QUANTA_PER_SECOND);

        {
            let mut d = LevelDetector::new();
            reading!("sine1", d.analyze(&sine(1.0, 10.0, 4800)));
        }
        {
            let mut d = LevelDetector::new();
            reading!("sine05", d.analyze(&sine(0.5, 10.0, 4800)));
        }
        {
            let mut d = LevelDetector::new();
            let block: Vec<f32> = (0..512)
                .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                .collect();
            reading!("square1", d.analyze(&block));
        }
        {
            let mut d = LevelDetector::new();
            reading!("silence", d.analyze(&[0.0f32; 128]));
            reading!("empty", d.analyze(&[]));
            reading!("empty_stereo", d.analyze_stereo(&[], &[]));
        }
        {
            let mut d = LevelDetector::new();
            reading!(
                "hostile",
                d.analyze(&[
                    f32::NAN,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    1.0e30,
                    -1.0e30,
                    0.5,
                    f32::NAN
                ])
            );
            reading!("all_nan", d.analyze(&[f32::NAN; 64]));
            reading!("after_nan", d.analyze(&[0.0f32; 64]));
        }
        {
            let mut d = LevelDetector::new();
            let _ = d.analyze(&[1.0f32; 64]);
            rec!("release.one_quantum", d.analyze(&[]).peak_hold);
        }
        {
            let mut d = LevelDetector::new();
            let r = d.analyze(&[1.0f32; 64]);
            rec!("smooth.one_quantum_rms", r.rms_smoothed);
            rec!("smooth.one_quantum_ms", d.mean_square());
        }
        {
            let mut d = LevelDetector::new();
            let first = d.analyze(&[1.0f32; 64]);
            rec!("decay.first_hold", first.peak_hold);
            let mut last = first;
            for _ in 0..DEFAULT_QUANTA_PER_SECOND as usize {
                last = d.analyze(&[0.0f32; 64]);
            }
            reading!("decay.after_1s", last);
        }
        {
            let tau_quanta = (0.3 * DEFAULT_QUANTA_PER_SECOND).round() as usize;
            let mut d = LevelDetector::new();
            let mut last = LevelReading::silence();
            for _ in 0..tau_quanta {
                last = d.analyze(&[0.5f32; 64]);
            }
            reading!("tau.at_tau", last);
            for _ in 0..(tau_quanta * 20) {
                last = d.analyze(&[0.5f32; 64]);
            }
            reading!("tau.steady", last);
        }
        {
            let left = sine(1.0, 10.0, 4800);
            let right = vec![0.0f32; 4800];
            let mut d = LevelDetector::new();
            reading!("stereo.linked", d.analyze_stereo(&left, &right));
            reading!("stereo.short", d.analyze_stereo(&[1.0, 1.0], &[1.0]));
        }
        {
            let mut d = LevelDetector::new();
            let mut rng = XorShift(0x1234_5678);
            for quantum in 0..8 {
                let block: Vec<f32> = (0..128).map(|_| rng.next_sample()).collect();
                let name: String = match quantum {
                    0 => format!("rng.q{}", 0),
                    1 => format!("rng.q{}", 1),
                    2 => format!("rng.q{}", 2),
                    3 => format!("rng.q{}", 3),
                    4 => format!("rng.q{}", 4),
                    5 => format!("rng.q{}", 5),
                    6 => format!("rng.q{}", 6),
                    _ => String::from("rng.q7"),
                };
                reading!(name, d.analyze(&block));
            }
        }
        {
            let mut d = LevelDetector::silent();
            reading!("silent.first", d.analyze(&[0.5f32; 32]));
            reading!("silent.later", d.analyze(&[0.0f32; 32]));
        }
        {
            let mut d = LevelDetector::with_ballistics(f32::NAN, -5.0, 0.0);
            reading!("badballistics", d.analyze(&[1.0f32; 32]));
        }
        {
            let mut d = LevelDetector::with_ballistics(375.0, 0.0, 0.3);
            reading!("norelease.loud", d.analyze(&[1.0f32; 32]));
            reading!("norelease.later", d.analyze(&[]));
        }
        {
            let mut d = LevelDetector::new();
            let _ = d.analyze(&[1.0f32; 64]);
            d.set_quanta_per_second(750.0);
            let mut last = LevelReading::silence();
            for _ in 0..750 {
                last = d.analyze(&[]);
            }
            reading!("qps750.after_1s", last);
        }
        {
            let mut d = LevelDetector::new();
            let _ = d.analyze(&[1.0f32; 32]);
            d.reset();
            reading!("reset.after", d.analyze(&[]));
            rec!("reset.hold", d.peak_hold());
            rec!("reset.ms", d.mean_square());
        }

        out
    }

    /// 冻结值在**非冻结架构**上允许的 ulp 预算（只对经过超越函数的条目生效）。
    ///
    /// 依据：`log10`/`exp`/`powf` 不要求正确舍入，标准库实现跨架构可差 1 ulp；
    /// 弹道递归最多迭代 750 次，相对误差上界 ≈ `750 · 2^-24 ≈ 4.5e-5`
    /// ≈ 750 ulp。给 4096 ulp（≈ 2.4e-4 相对）留 5× 余量，
    /// 而最小的注入效应（20→10 dB/s 改动 `peak_release` 0.3%）≈ 50000 ulp，
    /// 仍然远超预算 ⇒ 行为漂移照样变红（§6 的注入 1 实测）。
    const TRANSCENDENTAL_ULP_BUDGET: i64 = 4096;

    /// IEEE-754 单调映射：把 `f32` 位模式映射成可比较大小的整数，差值即 ulp 距离。
    fn ordered_bits(bits: u32) -> i64 {
        if bits & 0x8000_0000 != 0 {
            -i64::from(bits & 0x7fff_ffff)
        } else {
            i64::from(bits)
        }
    }

    /// 该冻结条目是否只经过 **IEEE 正确舍入** 的运算（⇒ 跨架构必须逐位相同）。
    ///
    /// 精确类：`sanitize_sample`（比较/钳位）、`supersedes`（整数比较）、常量、
    /// 以及检测器读数里的 `.peak`（`abs`/`max`）、`.rms`（乘加/除/`sqrt`）、
    /// `.is_sane`（有限性判断）——它们不碰 `log10`/`exp`/`powf`。
    fn frozen_entry_is_ieee_exact(name: &str) -> bool {
        name.starts_with("san.")
            || name.starts_with("sup.")
            || name.starts_with("const.")
            || name.ends_with(".peak")
            || name.ends_with(".rms")
            || name.ends_with(".is_sane")
    }

    /// **搬迁的机械证据**：285 个 `f32` 位模式在 `yeban-engine/src/level.rs`（main `b014e8f`）
    /// 上实测冻结；搬到本模块后必须**仍然相同**。
    ///
    /// 比较分两类（见 [`frozen_entry_is_ieee_exact`]）：
    ///
    /// - **IEEE 精确类**：跨架构**逐位**相同，没有容差；
    /// - **超越函数类**：给 4096 ulp 的跨架构预算（[`TRANSCENDENTAL_ULP_BUDGET`]），
    ///   在冻结架构（aarch64）上仍然要求逐位相同。
    ///
    /// 为什么用位模式而不是容差：容差会掩盖"搬家时顺手改了行为"。
    /// 任何一处行为漂移（释放率、时间常数、钳位、`0/0`、NaN 处理）都会在这里变红。
    /// 实测：本判据在 CI（x86_64 Linux）第一版把 `dbfs(√½)` 的 **1 ulp** 差异抓了出来
    /// （aarch64 `0xc040a8c2` vs x86_64 `0xc040a8c3`）——那正是需要区分类别的证据。
    #[test]
    fn frozen_pre_hoist_table_is_reproduced_bit_for_bit() {
        const FROZEN: &[(&str, u32)] = &[
            ("san.nan", 0x00000000),
            ("san.pinf", 0x41800000),
            ("san.ninf", 0xc1800000),
            ("san.1e30", 0x41800000),
            ("san.-1e30", 0xc1800000),
            ("san.0.25", 0x3e800000),
            ("san.16.0", 0x41800000),
            ("san.-16.0", 0xc1800000),
            ("san.16.000001", 0x41800000),
            ("san.-16.000001", 0xc1800000),
            ("san.neg_zero", 0x80000000),
            ("san.zero", 0x00000000),
            ("san.one", 0x3f800000),
            ("san.-one", 0xbf800000),
            ("san.20", 0x41800000),
            ("san.-20", 0xc1800000),
            ("san.min_positive", 0x00800000),
            ("san.max", 0x41800000),
            ("san.min", 0xc1800000),
            ("san.subnormal", 0x00000001),
            ("db.1.0", 0x00000000),
            ("db.0.5", 0xc0c0a8c2),
            ("db.0.25", 0xc140a8c2),
            ("db.0.1", 0xc1a00000),
            ("db.2.0", 0x40c0a8c2),
            ("db.16.0", 0x41c0a8c2),
            ("db.0", 0xff800000),
            ("db.neg_zero", 0xff800000),
            ("db.-1", 0xff800000),
            ("db.nan", 0xff800000),
            ("db.inf", 0x7f800000),
            ("db.ninf", 0xff800000),
            ("db.1e-30", 0xc4160000),
            ("db.min_positive", 0xc43da61e),
            ("db.fract_1_sqrt2", 0xc040a8c2),
            ("dbc.0", 0xc2f00000),
            ("dbc.1e-9", 0xc2f00000),
            ("dbc.1.0", 0x00000000),
            ("dbc.nan", 0xc2f00000),
            ("dbc.16.0", 0x41c0a8c2),
            ("sup.7_7", 0x3f800000),
            ("sup.8_7", 0x3f800000),
            ("sup.6_7", 0x00000000),
            ("sup.0_0", 0x3f800000),
            ("sup.max_max", 0x3f800000),
            ("sup.0_max", 0x00000000),
            ("const.qps", 0x43bb8000),
            ("sine1.peak", 0x3f800000),
            ("sine1.peak_hold", 0x3f800000),
            ("sine1.rms", 0x3f3504f3),
            ("sine1.rms_smoothed", 0x3d883b02),
            ("sine1.peak_dbfs", 0x00000000),
            ("sine1.peak_hold_dbfs", 0x00000000),
            ("sine1.rms_dbfs", 0xc1bc5432),
            ("sine1.is_sane", 0x3f800000),
            ("sine05.peak", 0x3f000000),
            ("sine05.peak_hold", 0x3f000000),
            ("sine05.rms", 0x3eb504f3),
            ("sine05.rms_smoothed", 0x3d083b02),
            ("sine05.peak_dbfs", 0xc0c0a8c2),
            ("sine05.peak_hold_dbfs", 0xc0c0a8c2),
            ("sine05.rms_dbfs", 0xc1ec7e63),
            ("sine05.is_sane", 0x3f800000),
            ("square1.peak", 0x3f800000),
            ("square1.peak_hold", 0x3f800000),
            ("square1.rms", 0x3f800000),
            ("square1.rms_smoothed", 0x3dc0a8b6),
            ("square1.peak_dbfs", 0x00000000),
            ("square1.peak_hold_dbfs", 0x00000000),
            ("square1.rms_dbfs", 0xc1a43f1b),
            ("square1.is_sane", 0x3f800000),
            ("silence.peak", 0x00000000),
            ("silence.peak_hold", 0x00000000),
            ("silence.rms", 0x00000000),
            ("silence.rms_smoothed", 0x00000000),
            ("silence.peak_dbfs", 0xff800000),
            ("silence.peak_hold_dbfs", 0xff800000),
            ("silence.rms_dbfs", 0xff800000),
            ("silence.is_sane", 0x3f800000),
            ("empty.peak", 0x00000000),
            ("empty.peak_hold", 0x00000000),
            ("empty.rms", 0x00000000),
            ("empty.rms_smoothed", 0x00000000),
            ("empty.peak_dbfs", 0xff800000),
            ("empty.peak_hold_dbfs", 0xff800000),
            ("empty.rms_dbfs", 0xff800000),
            ("empty.is_sane", 0x3f800000),
            ("empty_stereo.peak", 0x00000000),
            ("empty_stereo.peak_hold", 0x00000000),
            ("empty_stereo.rms", 0x00000000),
            ("empty_stereo.rms_smoothed", 0x00000000),
            ("empty_stereo.peak_dbfs", 0xff800000),
            ("empty_stereo.peak_hold_dbfs", 0xff800000),
            ("empty_stereo.rms_dbfs", 0xff800000),
            ("empty_stereo.is_sane", 0x3f800000),
            ("hostile.peak", 0x41800000),
            ("hostile.peak_hold", 0x41800000),
            ("hostile.rms", 0x41418a9b),
            ("hostile.rms_smoothed", 0x3f91a781),
            ("hostile.peak_dbfs", 0x41c0a8c2),
            ("hostile.peak_hold_dbfs", 0x41c0a8c2),
            ("hostile.rms_dbfs", 0x3f8fa676),
            ("hostile.is_sane", 0x3f800000),
            ("all_nan.peak", 0x00000000),
            ("all_nan.peak_hold", 0x417e6ed4),
            ("all_nan.rms", 0x00000000),
            ("all_nan.rms_smoothed", 0x3f910226),
            ("all_nan.peak_dbfs", 0xff800000),
            ("all_nan.peak_hold_dbfs", 0x41c03b87),
            ("all_nan.rms_dbfs", 0x3f8ab57b),
            ("all_nan.is_sane", 0x3f800000),
            ("after_nan.peak", 0x00000000),
            ("after_nan.peak_hold", 0x417ce01d),
            ("after_nan.rms", 0x00000000),
            ("after_nan.rms_smoothed", 0x3f905d87),
            ("after_nan.peak_dbfs", 0xff800000),
            ("after_nan.peak_hold_dbfs", 0x41bfce4d),
            ("after_nan.rms_dbfs", 0x3f85c482),
            ("after_nan.is_sane", 0x3f800000),
            ("release.one_quantum", 0x3f7e6ed4),
            ("smooth.one_quantum_rms", 0x3dc0a8b6),
            ("smooth.one_quantum_ms", 0x3c10fd80),
            ("decay.first_hold", 0x3f800000),
            ("decay.after_1s.peak", 0x00000000),
            ("decay.after_1s.peak_hold", 0x3dcccd27),
            ("decay.after_1s.rms", 0x00000000),
            ("decay.after_1s.rms_smoothed", 0x3c918de6),
            ("decay.after_1s.peak_dbfs", 0xff800000),
            ("decay.after_1s.peak_hold_dbfs", 0xc19fffe1),
            ("decay.after_1s.rms_dbfs", 0xc20c0779),
            ("decay.after_1s.is_sane", 0x3f800000),
            ("tau.at_tau.peak", 0x3f000000),
            ("tau.at_tau.peak_hold", 0x3f000000),
            ("tau.at_tau.rms", 0x3f000000),
            ("tau.at_tau.rms_smoothed", 0x3ecbcc41),
            ("tau.at_tau.peak_dbfs", 0xc0c0a8c2),
            ("tau.at_tau.peak_hold_dbfs", 0xc0c0a8c2),
            ("tau.at_tau.rms_dbfs", 0xc10005c3),
            ("tau.at_tau.is_sane", 0x3f800000),
            ("tau.steady.peak", 0x3f000000),
            ("tau.steady.peak_hold", 0x3f000000),
            ("tau.steady.rms", 0x3f000000),
            ("tau.steady.rms_smoothed", 0x3effffe4),
            ("tau.steady.peak_dbfs", 0xc0c0a8c2),
            ("tau.steady.peak_hold_dbfs", 0xc0c0a8c2),
            ("tau.steady.rms_dbfs", 0xc0c0a8e0),
            ("tau.steady.is_sane", 0x3f800000),
            ("stereo.linked.peak", 0x3f800000),
            ("stereo.linked.peak_hold", 0x3f800000),
            ("stereo.linked.rms", 0x3effffff),
            ("stereo.linked.rms_smoothed", 0x3d40a8b5),
            ("stereo.linked.peak_dbfs", 0x00000000),
            ("stereo.linked.peak_hold_dbfs", 0x00000000),
            ("stereo.linked.rms_dbfs", 0xc1d4694c),
            ("stereo.linked.is_sane", 0x3f800000),
            ("stereo.short.peak", 0x3f800000),
            ("stereo.short.peak_hold", 0x3f800000),
            ("stereo.short.rms", 0x3f800000),
            ("stereo.short.rms_smoothed", 0x3dd73569),
            ("stereo.short.peak_dbfs", 0x00000000),
            ("stereo.short.peak_hold_dbfs", 0x00000000),
            ("stereo.short.rms_dbfs", 0xc19c8e24),
            ("stereo.short.is_sane", 0x3f800000),
            ("rng.q0.peak", 0x3f7c7176),
            ("rng.q0.peak_hold", 0x3f7c7176),
            ("rng.q0.rms", 0x3f0fe2cf),
            ("rng.q0.rms_smoothed", 0x3d5891dd),
            ("rng.q0.peak_dbfs", 0xbdf8e261),
            ("rng.q0.peak_hold_dbfs", 0xbdf8e261),
            ("rng.q0.rms_dbfs", 0xc1cc482d),
            ("rng.q0.is_sane", 0x3f800000),
            ("rng.q1.peak", 0x3f7f58dc),
            ("rng.q1.peak_hold", 0x3f7f58dc),
            ("rng.q1.rms", 0x3f136d2f),
            ("rng.q1.rms_smoothed", 0x3d9ab2d0),
            ("rng.q1.peak_dbfs", 0xbcb5b3d2),
            ("rng.q1.peak_hold_dbfs", 0xbcb5b3d2),
            ("rng.q1.rms_dbfs", 0xc1b37eba),
            ("rng.q1.is_sane", 0x3f800000),
            ("rng.q2.peak", 0x3f7f7602),
            ("rng.q2.peak_hold", 0x3f7f7602),
            ("rng.q2.rms", 0x3f14a441),
            ("rng.q2.rms_smoothed", 0x3dbe59c2),
            ("rng.q2.peak_dbfs", 0xbc95fb27),
            ("rng.q2.peak_hold_dbfs", 0xbc95fb27),
            ("rng.q2.rms_dbfs", 0xc1a51588),
            ("rng.q2.is_sane", 0x3f800000),
            ("rng.q3.peak", 0x3f7eee54),
            ("rng.q3.peak_hold", 0x3f7eee54),
            ("rng.q3.rms", 0x3f1d1d4b),
            ("rng.q3.rms_smoothed", 0x3ddf5e55),
            ("rng.q3.peak_dbfs", 0xbd14e0f9),
            ("rng.q3.peak_hold_dbfs", 0xbd14e0f9),
            ("rng.q3.rms_dbfs", 0xc199f823),
            ("rng.q3.is_sane", 0x3f800000),
            ("rng.q4.peak", 0x3f79284a),
            ("rng.q4.peak_hold", 0x3f7d5ed5),
            ("rng.q4.rms", 0x3f108922),
            ("rng.q4.rms_smoothed", 0x3df78e31),
            ("rng.q4.peak_dbfs", 0xbe70f8d5),
            ("rng.q4.peak_hold_dbfs", 0xbdb7aa66),
            ("rng.q4.rms_dbfs", 0xc192d340),
            ("rng.q4.is_sane", 0x3f800000),
            ("rng.q5.peak", 0x3f7ad6aa),
            ("rng.q5.peak_hold", 0x3f7bd1c8),
            ("rng.q5.rms", 0x3f126313),
            ("rng.q5.rms_smoothed", 0x3e06fad2),
            ("rng.q5.peak_dbfs", 0xbe352907),
            ("rng.q5.peak_hold_dbfs", 0xbe127229),
            ("rng.q5.rms_dbfs", 0xc18cce15),
            ("rng.q5.is_sane", 0x3f800000),
            ("rng.q6.peak", 0x3f7e58ee),
            ("rng.q6.peak_hold", 0x3f7e58ee),
            ("rng.q6.rms", 0x3f1037c9),
            ("rng.q6.rms_smoothed", 0x3e10eccb),
            ("rng.q6.peak_dbfs", 0xbd666a7a),
            ("rng.q6.peak_hold_dbfs", 0xbd666a7a),
            ("rng.q6.rms_dbfs", 0xc187dd76),
            ("rng.q6.is_sane", 0x3f800000),
            ("rng.q7.peak", 0x3f7feff3),
            ("rng.q7.peak_hold", 0x3f7feff3),
            ("rng.q7.rms", 0x3f15f23f),
            ("rng.q7.rms_smoothed", 0x3e1aec1c),
            ("rng.q7.peak_dbfs", 0xbb0b6eb1),
            ("rng.q7.peak_hold_dbfs", 0xbb0b6eb1),
            ("rng.q7.rms_dbfs", 0xc1833ad2),
            ("rng.q7.is_sane", 0x3f800000),
            ("silent.first.peak", 0x3f000000),
            ("silent.first.peak_hold", 0x3f000000),
            ("silent.first.rms", 0x3f000000),
            ("silent.first.rms_smoothed", 0x3f000000),
            ("silent.first.peak_dbfs", 0xc0c0a8c2),
            ("silent.first.peak_hold_dbfs", 0xc0c0a8c2),
            ("silent.first.rms_dbfs", 0xc0c0a8c2),
            ("silent.first.is_sane", 0x3f800000),
            ("silent.later.peak", 0x00000000),
            ("silent.later.peak_hold", 0x3f000000),
            ("silent.later.rms", 0x00000000),
            ("silent.later.rms_smoothed", 0x00000000),
            ("silent.later.peak_dbfs", 0xff800000),
            ("silent.later.peak_hold_dbfs", 0xc0c0a8c2),
            ("silent.later.rms_dbfs", 0xff800000),
            ("silent.later.is_sane", 0x3f800000),
            ("badballistics.peak", 0x3f800000),
            ("badballistics.peak_hold", 0x3f800000),
            ("badballistics.rms", 0x3f800000),
            ("badballistics.rms_smoothed", 0x3f4b890f),
            ("badballistics.peak_dbfs", 0x00000000),
            ("badballistics.peak_hold_dbfs", 0x00000000),
            ("badballistics.rms_dbfs", 0xbffef9e2),
            ("badballistics.is_sane", 0x3f800000),
            ("norelease.loud.peak", 0x3f800000),
            ("norelease.loud.peak_hold", 0x3f800000),
            ("norelease.loud.rms", 0x3f800000),
            ("norelease.loud.rms_smoothed", 0x3dc0a8b6),
            ("norelease.loud.peak_dbfs", 0x00000000),
            ("norelease.loud.peak_hold_dbfs", 0x00000000),
            ("norelease.loud.rms_dbfs", 0xc1a43f1b),
            ("norelease.loud.is_sane", 0x3f800000),
            ("norelease.later.peak", 0x00000000),
            ("norelease.later.peak_hold", 0x3f800000),
            ("norelease.later.rms", 0x00000000),
            ("norelease.later.rms_smoothed", 0x3dbfcdfe),
            ("norelease.later.peak_dbfs", 0xff800000),
            ("norelease.later.peak_hold_dbfs", 0x00000000),
            ("norelease.later.rms_dbfs", 0xc1a48e2a),
            ("norelease.later.is_sane", 0x3f800000),
            ("qps750.after_1s.peak", 0x00000000),
            ("qps750.after_1s.peak_hold", 0x3dccccc7),
            ("qps750.after_1s.rms", 0x00000000),
            ("qps750.after_1s.rms_smoothed", 0x3c918de4),
            ("qps750.after_1s.peak_dbfs", 0xff800000),
            ("qps750.after_1s.peak_hold_dbfs", 0xc1a00002),
            ("qps750.after_1s.rms_dbfs", 0xc20c077a),
            ("qps750.after_1s.is_sane", 0x3f800000),
            ("reset.after.peak", 0x00000000),
            ("reset.after.peak_hold", 0x00000000),
            ("reset.after.rms", 0x00000000),
            ("reset.after.rms_smoothed", 0x00000000),
            ("reset.after.peak_dbfs", 0xff800000),
            ("reset.after.peak_hold_dbfs", 0xff800000),
            ("reset.after.rms_dbfs", 0xff800000),
            ("reset.after.is_sane", 0x3f800000),
            ("reset.hold", 0x00000000),
            ("reset.ms", 0x00000000),
        ];
        let actual = probe();
        assert_eq!(
            actual.len(),
            FROZEN.len(),
            "探针产生的读数条数变了: {} vs {}",
            actual.len(),
            FROZEN.len()
        );
        for ((name, bits), (frozen_name, frozen_bits)) in actual.iter().zip(FROZEN.iter()) {
            assert_eq!(name, frozen_name, "读数顺序/命名漂移");
            if frozen_entry_is_ieee_exact(name) {
                // 只经过比较/钳位/abs/max/min/加减乘除/开方 ⇒ IEEE-754 要求正确舍入
                // ⇒ **跨架构也必须逐位相同**。
                assert_eq!(
                    bits,
                    frozen_bits,
                    "{name} 漂移: 实测 {bits:#010x} ({}), 搬迁前 {frozen_bits:#010x} ({})",
                    f32::from_bits(*bits),
                    f32::from_bits(*frozen_bits)
                );
            } else {
                // 经过 `log10`/`exp`/`powf`：标准库实现**不要求**正确舍入，
                // aarch64 与 x86_64 可以差 1 ulp，误差还会沿弹道递归累积。
                // 因此给一个 ulp 预算（见常量）；在冻结架构上仍然要求逐位相同。
                let distance = (ordered_bits(*bits) - ordered_bits(*frozen_bits)).abs();
                assert!(
                    distance <= TRANSCENDENTAL_ULP_BUDGET,
                    "{name} 超出 ulp 预算: 实测 {bits:#010x} ({}), 搬迁前 {frozen_bits:#010x} ({}), 相距 {distance} ulp",
                    f32::from_bits(*bits),
                    f32::from_bits(*frozen_bits)
                );
                if cfg!(target_arch = "aarch64") {
                    assert_eq!(
                        bits, frozen_bits,
                        "{name} 在冻结架构 (aarch64) 上必须逐位相同"
                    );
                }
            }
        }
    }
}
