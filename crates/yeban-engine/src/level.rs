//! 电平计量原语：峰值保持、RMS 一阶平滑、dBFS 换算、输入钳位与"取最新"判据。
//! [ARCH-UI-002, ROAD-M2-008]
//!
//! ## 为什么单独一个模块
//!
//! [ARCH-UI-002] 要求实时线程压入的是**真峰值与 RMS 电平**。真值 = 一套可复算的
//! DSP 口径，而不是"把样本绝对值取个 max"。本模块把口径做成**零依赖纯函数/纯状态机**：
//!
//! - 不引用 `rtrb` / `yeban_model` / cpal，也不引用本 crate 的其它模块；
//! - 因此可以**脱离重依赖**单独编译与运行：
//!   `rustc --edition 2024 --test -D warnings crates/yeban-engine/src/level.rs`；
//! - 实时路径（[`crate::rt`]）与 UI 侧（[`crate::meter`]）都只调用这里。
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
//!
//! **时间常数（默认值，见常量）**：峰值释放 **20 dB/s**（每秒恰好降 20 dB）；
//! 平滑 RMS 一阶低通时间常数 **τ = 300 ms**。两者都按**每量子**折算，
//! 折算基准为 `quanta_per_second = sample_rate / block_frames`
//! （规范默认 48 000 / 128 = 375 Hz）。快照切换时重算一次，见 [`crate::rt`]。
//!
//! **为什么是 20 dB/s / 300 ms**：20 dB/s 是业界峰值表的常见回落速率
//! （1 秒回落一个数量级，既不会"钉死"也不会闪得看不清）；300 ms 接近 VU 的
//! 积分观感，用于 RMS 平滑。它们是**选择**而不是规范硬性数字，因此以常量 +
//! 文档的形式公开，允许后续线按 UI 观感调整（调整必须同步改本表的判据）。
//!
//! ## 输入钳位（去爆音/防污染）
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
//! ——后者属于声部合成切片，本模块不做任何 DSP 处理。

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
        let qps = DEFAULT_QUANTA_PER_SECOND;
        let quanta = qps as usize;
        let mut detector = LevelDetector::new();
        let first = detector.analyze(&[1.0f32; 64]);
        assert!((first.peak_hold - 1.0).abs() < 1e-6);

        let mut last = first;
        for _ in 0..quanta {
            last = detector.analyze(&[0.0f32; 64]);
        }
        // 起始幅度是 1.0(0 dBFS) ⇒ 回落量就是保持值的负 dBFS
        let decay_db = -last.peak_hold_dbfs();
        assert!(
            (decay_db - DEFAULT_PEAK_DECAY_DB_PER_SEC).abs() < 0.05,
            "1 秒应恰好回落 {} dB, 实际 {} dB",
            DEFAULT_PEAK_DECAY_DB_PER_SEC,
            decay_db
        );
    }

    #[test]
    fn smoothed_rms_follows_the_documented_time_constant() {
        // 常数 0.5 输入下, 均方一阶低通在 τ 秒后达到目标的 1-1/e。
        let qps = DEFAULT_QUANTA_PER_SECOND;
        let tau_quanta = (DEFAULT_RMS_TIME_CONSTANT_SEC * qps).round() as usize;
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
}
