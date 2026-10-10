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
//! | 真峰值 `true_peak` | **8×** 多相过采样后的 `max abs`（可选 16×，见 [`TruePeakDetector`]） | 线性幅度 |
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
// 真峰值（true peak）：8× 多相过采样（按需 16×）
// ---------------------------------------------------------------------------
//
// HD-26：4× 在 `f = 0.4·fs` 处**固有**欠读 0.436 dB（连续峰值落在
// `t = 5/8` 样本上，而 4× 的网格步长是 1/4），母带/导出场景不可接受。
// 依 D43（1.0.0 之前直接推翻，不留兼容层）本线把核换成默认 **8×**，并另留 16×。
//
// 判据 [`tests::eight_times_oversampling_fixes_the_four_tenths_nyquist_underread`]
// 在**同一夹具、同一段测量代码**上同时给出改建前（冻结的 4× 核表）与改建后的读数。

/// 默认过采样倍数（= 多相核的相位数）：**8×**。
pub const TRUE_PEAK_PHASES: usize = 8;

/// 按需的高倍数：**16×**（用 [`TruePeakDetector::with_oversampling`] 选择）。
pub const TRUE_PEAK_PHASES_HIGH: usize = 16;

/// 每个相位的抽头数（原型总系数 = 相位数 × 本值）。
pub const TRUE_PEAK_TAPS: usize = 32;

/// 多相核的 Kaiser 窗形状参数 β。
pub const TRUE_PEAK_KAISER_BETA: f64 = 8.0;

/// 真峰值检测器引入的延迟（**基础采样率**样本）= `TRUE_PEAK_TAPS / 2`。
///
/// 因果 FIR 不可能零延迟；真峰值是**幅度**量，因此这个延迟不影响读数，
/// 只影响"读到的是哪一段时间"（供需要对齐的调用方参考）。
pub const TRUE_PEAK_LATENCY_SAMPLES: usize = TRUE_PEAK_TAPS / 2;

/// 8× 多相插值核：`TRUE_PEAK_KERNEL_8X[phase][m]` 乘 `x[i − m]`。
///
/// **生成口径**（16× 表用同一条公式，只把相位数换成 16）：
///
/// ```text
/// C = T/2 = 16                      // sinc 中心 = 延迟（基础采样率样本）
/// h_p[m] = sinc(m − C + p/L) · Kaiser(m; β = 8, 中心 (T−1)/2, 支撑 [0, T−1])
/// L = 8（相位数）, T = 32（每相抽头数）, β = 8
/// 逐相位归一化: h_p[m] ← h_p[m] / Σ_k h_p[k]     // 每相直流增益 = 1
/// 相位 0 强制为 δ(m − C)                         // 过采样输出含原始样本
/// ```
///
/// **这不是另起一套口径**：同一条公式（β = 8、窗中心 (T−1)/2、逐相位归一化）
/// 能复现本仓库**旧的 4× 核表**到 1.3e-8 以内（残余是 f32 最短往返表示的舍入），
/// 本线只是把 L 从 4 提到 8。复现脚本与逐项对照见
/// `docs/ledger/dsp-loudness-notes.md` §2。
///
/// 系数按**最短往返表示**写成 `f32` 字面量（与 `crate::oversample` 同一纪律：
/// 数值不变，只是不再触发 `clippy::excessive_precision`）。
///
/// 判据 [`tests::true_peak_kernel_is_coherent`] 钉住：每相直流增益为 1、相位 0 是 δ、
/// **每个相位**的频响幅度在 `f ≤ 0.42·fs` 处都落在 `1 ± 2e-3`（实测最坏 −7.6e-4），
/// 以及带边（`f = 0.49·fs`）**如实**下垂（实测最小相位幅度 0.2655）——
/// 后一条是"T = 32 的带边极限"这个事实的机械记录，不是要修的 bug。
///
/// 16× 表的偶数相位与 8× 表逐位相同（16× 的相位 2k 就是 8× 的相位 k）——
/// 判据 [`tests::sixteen_times_contains_eight_times_phases_bit_for_bit`] 钉住这一点。
#[rustfmt::skip]
const TRUE_PEAK_KERNEL_8X: [[f32; TRUE_PEAK_TAPS]; 8] = [
    // phase 0
        [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    // phase 1
        [
        0.0, 8.152421e-05, -0.0002190799, 0.0004751827, -0.00090775196, 0.0015891907, -0.0026080068, 0.0040724045,
        -0.0061187544, 0.008931006, -0.012784809, 0.018151874, -0.025972238, 0.03850026, -0.06286008, 0.13897151,
        0.9728006, -0.1047668, 0.05208859, -0.032205574, 0.021452215, -0.014655756, 0.010024599, -0.006762834,
        0.0044483184, -0.0028223635, 0.0017069085, -0.0009689487, 0.00050457544, -0.00023159875, 8.585293e-05, 0.0,
        ],
    // phase 2
        [
        0.0, 0.00015172423, -0.0004079775, 0.0008855254, -0.0016930365, 0.0029668813, -0.004874663, 0.007622801,
        -0.0114740115, 0.016786985, -0.024106693, 0.034379996, -0.04952827, 0.07428005, -0.12429153, 0.2992099,
        0.8976297, -0.17400815, 0.09078673, -0.057148006, 0.038424704, -0.026402568, 0.018129943, -0.012265322,
        0.008084789, -0.005138158, 0.0031116072, -0.0017682825, 0.00092166936, -0.00042337287, 0.00015704789, 0.0,
        ],
    // phase 3
        [
        0.0, 0.00019978156, -0.00053753494, 0.0011675733, -0.0022341665, 0.003919079, -0.006446927, 0.01009643,
        -0.015225845, 0.022330316, -0.03217268, 0.046099056, -0.06689301, 0.101596765, -0.17475536, 0.46877182,
        0.78128636, -0.20652907, 0.112291165, -0.07184805, 0.048733287, -0.033669084, 0.023206016, -0.015741976,
        0.010397815, -0.006618845, 0.0040135146, -0.002283269, 0.0011911606, -0.0005475823, 0.00020325602, 0.0,
        ],
    // phase 4
        [
        0.0, 0.0002180515, -0.0005870637, 0.00127609, -0.002443917, 0.0042914045, -0.0070681493, 0.011086228,
        -0.01675081, 0.0246288, -0.03560598, 0.051270444, -0.07497156, 0.11543699, -0.20486532, 0.6340848,
        0.6340848, -0.20486532, 0.11543699, -0.07497156, 0.051270444, -0.03560598, 0.0246288, -0.01675081,
        0.011086228, -0.0070681493, 0.0042914045, -0.002443917, 0.00127609, -0.0005870637, 0.0002180515, 0.0,
        ],
    // phase 5
        [
        0.0, 0.00020325602, -0.0005475823, 0.0011911606, -0.002283269, 0.0040135146, -0.006618845, 0.010397815,
        -0.015741976, 0.023206016, -0.033669084, 0.048733287, -0.07184805, 0.112291165, -0.20652907, 0.78128636,
        0.46877182, -0.17475536, 0.101596765, -0.06689301, 0.046099056, -0.03217268, 0.022330316, -0.015225845,
        0.01009643, -0.006446927, 0.003919079, -0.0022341665, 0.0011675733, -0.00053753494, 0.00019978156, 0.0,
        ],
    // phase 6
        [
        0.0, 0.00015704789, -0.00042337287, 0.00092166936, -0.0017682825, 0.0031116072, -0.005138158, 0.008084789,
        -0.012265322, 0.018129943, -0.026402568, 0.038424704, -0.057148006, 0.09078673, -0.17400815, 0.8976297,
        0.2992099, -0.12429153, 0.07428005, -0.04952827, 0.034379996, -0.024106693, 0.016786985, -0.0114740115,
        0.007622801, -0.004874663, 0.0029668813, -0.0016930365, 0.0008855254, -0.0004079775, 0.00015172423, 0.0,
        ],
    // phase 7
        [
        0.0, 8.585293e-05, -0.00023159875, 0.00050457544, -0.0009689487, 0.0017069085, -0.0028223635, 0.0044483184,
        -0.006762834, 0.010024599, -0.014655756, 0.021452215, -0.032205574, 0.05208859, -0.1047668, 0.9728006,
        0.13897151, -0.06286008, 0.03850026, -0.025972238, 0.018151874, -0.012784809, 0.008931006, -0.0061187544,
        0.0040724045, -0.0026080068, 0.0015891907, -0.00090775196, 0.0004751827, -0.0002190799, 8.152421e-05, 0.0,
        ],
];

/// 16× 多相插值核（同一条公式，`L = 16`；偶数相位与 8× 表逐位相同）。
#[rustfmt::skip]
const TRUE_PEAK_KERNEL_16X: [[f32; TRUE_PEAK_TAPS]; 16] = [
    // phase 0
        [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    // phase 1
        [
        0.0, 4.1420284e-05, -0.00011127512, 0.00024127089, -0.00046071765, 0.00080618483, -0.0013222577, 0.002063244,
        -0.0030972573, 0.0045156213, -0.0064542517, 0.0091440035, -0.013040911, 0.019225208, -0.031037157, 0.06617744,
        0.99266165, -0.05659717, 0.027381359, -0.016766885, 0.011113482, -0.0075698015, 0.0051673604, -0.0034809883,
        0.002287162, -0.001449924, 0.0008762879, -0.0004971586, 0.0002587724, -0.00011872895, 4.399755e-05, 0.0,
        ],
    // phase 2
        [
        0.0, 8.152421e-05, -0.0002190799, 0.0004751827, -0.00090775196, 0.0015891907, -0.0026080068, 0.0040724045,
        -0.0061187544, 0.008931006, -0.012784809, 0.018151874, -0.025972238, 0.03850026, -0.06286008, 0.13897151,
        0.9728006, -0.1047668, 0.05208859, -0.032205574, 0.021452215, -0.014655756, 0.010024599, -0.006762834,
        0.0044483184, -0.0028223635, 0.0017069085, -0.0009689487, 0.00050457544, -0.00023159875, 8.585293e-05, 0.0,
        ],
    // phase 3
        [
        0.0, 0.00011877271, -0.0003192748, 0.00069274823, -0.001323916, 0.0023188924, -0.0038077426, 0.005950063,
        -0.00894798, 0.013075792, -0.01874738, 0.026676374, -0.038297698, 0.057096623, -0.094340816, 0.21712604,
        0.9408795, -0.14399388, 0.073409945, -0.04580705, 0.030657921, -0.0210061, 0.014396579, -0.009726065,
        0.006404266, -0.0040667728, 0.0024611556, -0.0013978778, 0.00072827376, -0.00033440633, 0.00012400499, 0.0,
        ],
    // phase 4
        [
        0.0, 0.00015172423, -0.0004079775, 0.0008855254, -0.0016930365, 0.0029668813, -0.004874663, 0.007622801,
        -0.0114740115, 0.016786985, -0.024106693, 0.034379996, -0.04952827, 0.07428005, -0.12429153, 0.2992099,
        0.8976297, -0.17400815, 0.09078673, -0.057148006, 0.038424704, -0.026402568, 0.018129943, -0.012265322,
        0.008084789, -0.005138158, 0.0031116072, -0.0017682825, 0.00092166936, -0.00042337287, 0.00015704789, 0.0,
        ],
    // phase 5
        [
        0.0, 0.00017908959, -0.00048171042, 0.0010459392, -0.0020005703, 0.0035075492, -0.0057664528, 0.009024,
        -0.013595748, 0.019915165, -0.02864543, 0.040947694, -0.059200272, 0.089336246, -0.15149806, 0.3836505,
        0.84403116, -0.19478323, 0.1038232, -0.06590219, 0.044508364, -0.03066746, 0.021098245, -0.014292966,
        0.009431098, -0.005998659, 0.0036350966, -0.0020668877, 0.0010777952, -0.00049527973, 0.0001837819, 0.0,
        ],
    // phase 6
        [
        0.0, 0.00019978156, -0.00053753494, 0.0011675733, -0.0022341665, 0.003919079, -0.006446927, 0.01009643,
        -0.015225845, 0.022330316, -0.03217268, 0.046099056, -0.06689301, 0.101596765, -0.17475536, 0.46877182,
        0.78128636, -0.20652907, 0.112291165, -0.07184805, 0.048733287, -0.033669084, 0.023206016, -0.015741976,
        0.010397815, -0.006618845, 0.0040135146, -0.002283269, 0.0011911606, -0.0005475823, 0.00020325602, 0.0,
        ],
    // phase 7
        [
        0.0, 0.00021295718, -0.000573166, 0.0012454217, -0.0023841509, 0.004184304, -0.0068874587, 0.010794519,
        -0.016294193, 0.023927009, -0.034531604, 0.04959956, -0.07224551, 0.11046439, -0.19290383, 0.5528356,
        0.71078855, -0.20967807, 0.11612923, -0.07487262, 0.050996736, -0.035325434, 0.024391612, -0.016568046,
        0.010954439, -0.0069786836, 0.004234415, -0.0024102072, 0.0012579384, -0.00057849777, 0.00021480097, 0.0,
        ],
    // phase 8
        [
        0.0, 0.0002180515, -0.0005870637, 0.00127609, -0.002443917, 0.0042914045, -0.0070681493, 0.011086228,
        -0.01675081, 0.0246288, -0.03560598, 0.051270444, -0.07497156, 0.11543699, -0.20486532, 0.6340848,
        0.6340848, -0.20486532, 0.11543699, -0.07497156, 0.051270444, -0.03560598, 0.0246288, -0.01675081,
        0.011086228, -0.0070681493, 0.0042914045, -0.002443917, 0.00127609, -0.0005870637, 0.0002180515, 0.0,
        ],
    // phase 9
        [
        0.0, 0.00021480097, -0.00057849777, 0.0012579384, -0.0024102072, 0.004234415, -0.0069786836, 0.010954439,
        -0.016568046, 0.024391612, -0.035325434, 0.050996736, -0.07487262, 0.11612923, -0.20967807, 0.71078855,
        0.5528356, -0.19290383, 0.11046439, -0.07224551, 0.04959956, -0.034531604, 0.023927009, -0.016294193,
        0.010794519, -0.0068874587, 0.004184304, -0.0023841509, 0.0012454217, -0.000573166, 0.00021295718, 0.0,
        ],
    // phase 10
        [
        0.0, 0.00020325602, -0.0005475823, 0.0011911606, -0.002283269, 0.0040135146, -0.006618845, 0.010397815,
        -0.015741976, 0.023206016, -0.033669084, 0.048733287, -0.07184805, 0.112291165, -0.20652907, 0.78128636,
        0.46877182, -0.17475536, 0.101596765, -0.06689301, 0.046099056, -0.03217268, 0.022330316, -0.015225845,
        0.01009643, -0.006446927, 0.003919079, -0.0022341665, 0.0011675733, -0.00053753494, 0.00019978156, 0.0,
        ],
    // phase 11
        [
        0.0, 0.0001837819, -0.00049527973, 0.0010777952, -0.0020668877, 0.0036350966, -0.005998659, 0.009431098,
        -0.014292966, 0.021098245, -0.03066746, 0.044508364, -0.06590219, 0.1038232, -0.19478323, 0.84403116,
        0.3836505, -0.15149806, 0.089336246, -0.059200272, 0.040947694, -0.02864543, 0.019915165, -0.013595748,
        0.009024, -0.0057664528, 0.0035075492, -0.0020005703, 0.0010459392, -0.00048171042, 0.00017908959, 0.0,
        ],
    // phase 12
        [
        0.0, 0.00015704789, -0.00042337287, 0.00092166936, -0.0017682825, 0.0031116072, -0.005138158, 0.008084789,
        -0.012265322, 0.018129943, -0.026402568, 0.038424704, -0.057148006, 0.09078673, -0.17400815, 0.8976297,
        0.2992099, -0.12429153, 0.07428005, -0.04952827, 0.034379996, -0.024106693, 0.016786985, -0.0114740115,
        0.007622801, -0.004874663, 0.0029668813, -0.0016930365, 0.0008855254, -0.0004079775, 0.00015172423, 0.0,
        ],
    // phase 13
        [
        0.0, 0.00012400499, -0.00033440633, 0.00072827376, -0.0013978778, 0.0024611556, -0.0040667728, 0.006404266,
        -0.009726065, 0.014396579, -0.0210061, 0.030657921, -0.04580705, 0.073409945, -0.14399388, 0.9408795,
        0.21712604, -0.094340816, 0.057096623, -0.038297698, 0.026676374, -0.01874738, 0.013075792, -0.00894798,
        0.005950063, -0.0038077426, 0.0023188924, -0.001323916, 0.00069274823, -0.0003192748, 0.00011877271, 0.0,
        ],
    // phase 14
        [
        0.0, 8.585293e-05, -0.00023159875, 0.00050457544, -0.0009689487, 0.0017069085, -0.0028223635, 0.0044483184,
        -0.006762834, 0.010024599, -0.014655756, 0.021452215, -0.032205574, 0.05208859, -0.1047668, 0.9728006,
        0.13897151, -0.06286008, 0.03850026, -0.025972238, 0.018151874, -0.012784809, 0.008931006, -0.0061187544,
        0.0040724045, -0.0026080068, 0.0015891907, -0.00090775196, 0.0004751827, -0.0002190799, 8.152421e-05, 0.0,
        ],
    // phase 15
        [
        0.0, 4.399755e-05, -0.00011872895, 0.0002587724, -0.0004971586, 0.0008762879, -0.001449924, 0.002287162,
        -0.0034809883, 0.0051673604, -0.0075698015, 0.011113482, -0.016766885, 0.027381359, -0.05659717, 0.99266165,
        0.06617744, -0.031037157, 0.019225208, -0.013040911, 0.0091440035, -0.0064542517, 0.0045156213, -0.0030972573,
        0.002063244, -0.0013222577, 0.00080618483, -0.00046071765, 0.00024127089, -0.00011127512, 4.1420284e-05, 0.0,
        ],
];

/// 取某个过采样倍数对应的核表；只认 8× 与 16×。
const fn kernel_for(oversampling: usize) -> Option<&'static [[f32; TRUE_PEAK_TAPS]]> {
    match oversampling {
        TRUE_PEAK_PHASES => Some(&TRUE_PEAK_KERNEL_8X),
        TRUE_PEAK_PHASES_HIGH => Some(&TRUE_PEAK_KERNEL_16X),
        _ => None,
    }
}

/// 多相过采样真峰值检测器（[ARCH-UI-002] 的"真峰值"字面要求）。
///
/// **为什么需要它**：样本峰值会漏掉采样点之间的过冲（intersample peak）。
/// 一个在 `fs/4` 上相位偏移 45° 的满幅正弦，样本全是 `±0.7071`
/// （采样峰值 −3.01 dBFS），而它真实的连续峰值是 1.0（0 dBFS）——
/// 只按样本峰值做限制器就会在真峰值上失真/超限。
///
/// **口径**：`L×` 多相插值（默认 `L = 8`，[`TRUE_PEAK_KERNEL_8X`]），
/// 取**插值后样本与原始样本**的 `max abs`。相位 0 是精确的 `δ`，
/// 因此过采样输出天然包含原始样本 ⇒ [`真峰值 ≥ 采样峰值`](Self::process) 恒成立。
///
/// **诚实的边界**（不要误读为"精确真峰值"）：
///
/// 1. `L×` 是**估计**：真正峰值可能落在 `1/L` 网格之间。本线实测（同一夹具、同一段测量代码）：
///    4× 在 `f = 0.4·fs` 读 `cos(π/10) = 0.9511`（**−0.436 dB**）；换成 8× 后同一频点
///    读 **−0.0005 dB**（8× 的网格步长 `1/8` 恰好覆盖该频点的真峰 `t = 5/8`）。
///    **频率轴上真正的最坏点在相称频率**（真峰与网格的相对位置固定）：
///    `f = 4/9·fs` 处 8× 读到 **0.984808（−0.133 dB）** —— 这是 8× 的**固有**残余，
///    如实钉在
///    [`tests::true_peak_steady_state_underread_is_within_the_documented_budget`] 里。
///    同一批频点上 4× 的最坏读数是 `f = 0.4·fs` 的 **−0.436 dB** ⇒ 8× 把**最坏值**
///    收紧 **0.303 dB**；16× 的最坏读数是 `6/13·fs` 的 **−0.064 dB**
///    （`4/9·fs` 处比 8× 收紧 **0.106 dB**）—— "要不要 16×"的完整数字见
///    [`tests::sixteen_times_tightens_the_worst_case_grid_comb_by_the_documented_margin`]
///    与 notes §2.5。一般频率（分母大、相对位置遍历整周）上 8× 的读数 ≥ 0.9998。
/// 2. **冷启动瞬态过读**：FIR 从零状态起步时，窗化 sinc 的暂态可以读到
///    `f = 0.4·fs` 的 **1.0568**（+0.48 dB，实测）。这是"头一个窗还没填满"的暂态，
///    不是稳态失准；要稳态读数请先喂满至少 [`TRUE_PEAK_TAPS`] 个样本（判据
///    [`tests::true_peak_is_transparent_at_dc_and_low_frequency`] 就是这么做的）。
///    过读方向在**安全侧**（限制器不会因此漏拦），但把它当稳态值会误判。
/// 3. 输入经 [`sanitize_sample`]（`NaN → 0`、`±∞`/越界 → `±16`）。
/// 4. 通道口径沿用调用方：本类型只处理**给定的一路样本流**，
///    多声道的联动/求和由调用方决定（与 [`LevelDetector`] 的分工一致）。
///
/// **零分配**：状态是定长数组（`[f32; 32] + f32 + 一个 `'static` 表引用`），
/// `process` 内只有乘加与移位 [ARCH-RT-001]。
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
    /// 多相核（`'static` 表，按过采样倍数选）。
    kernel: &'static [[f32; TRUE_PEAK_TAPS]],
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
    /// 新建默认（**8×**，历史归零）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kernel: &TRUE_PEAK_KERNEL_8X,
            window: [0.0; TRUE_PEAK_TAPS],
            peak: 0.0,
        }
    }

    /// 按过采样倍数新建；只接受 [`TRUE_PEAK_PHASES`]（8）与 [`TRUE_PEAK_PHASES_HIGH`]（16），
    /// 其余返回 `None`（**不静默回落**：不支持的倍数会让"真峰值"口径立刻失真）。
    #[must_use]
    pub const fn with_oversampling(oversampling: usize) -> Option<Self> {
        match kernel_for(oversampling) {
            Some(kernel) => Some(Self {
                kernel,
                window: [0.0; TRUE_PEAK_TAPS],
                peak: 0.0,
            }),
            None => None,
        }
    }

    /// 本实例的过采样倍数（相位数）。
    #[must_use]
    pub const fn oversampling(&self) -> usize {
        self.kernel.len()
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
    ///
    /// 相位 0 的核是精确 `δ(m − 16)` ⇒ 每段输出里都**原样包含**样本峰值
    /// ⇒ 返回值恒 `≥` 该段的样本峰值（判据
    /// [`tests::true_peak_preserves_the_sample_grid`]）。
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
            for kernel in self.kernel {
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
    // 真峰值（默认 8×，按需 16×）——HD-26
    // -----------------------------------------------------------------------

    /// 稳态真峰值的**逐频点预算**：`(f/fs, 8× 与 16× 都必须达到的最低读数)`。
    ///
    /// 频点分两类（这是本方案**实测**出来的结构，不是随手挑的）：
    ///
    /// - **相称频率**（有理数、分母小 ⇒ 真峰与 `1/L` 网格的相对位置**固定**）：
    ///   欠读表现为**网格梳**。`L = 8` 的最坏点在 `f = 4/9·fs`：
    ///   8× 读 `0.984808`（**−0.133 dB**）；同一频点 16× 读 `0.996884`。
    /// - **一般频率**（分母大 ⇒ 相对位置遍历整周）：网格几乎总能落在真峰附近，
    ///   读数 ≈ 1（实测最坏 `−0.0017 dB @ 0.1·fs`）。
    ///
    /// 预算 = 实测值往下留 ~0.001 的余量（既有判别力，又不会因 1 ulp 级抖动变红）。
    /// 4× 在同一批频点上会**大面积变红**：`0.4·fs` 处 0.9511、
    /// `2/7·fs` 处 0.9750、`1/4·fs` 与 `4/9·fs` 处 0.9848 —— 最坏 −0.436 dB。
    const TRUE_PEAK_STEADY_FLOOR: &[(f64, f32)] = &[
        // --- 一般频率 ---
        (0.02, 0.9985),
        (0.05, 0.9990),
        (0.1, 0.9985),
        (0.15, 0.9990),
        (0.2, 0.9990),
        (0.25, 0.9990),
        (0.3, 0.9985),
        (1.0 / 3.0, 0.9990),
        (0.35, 0.9990),
        (0.375, 0.9990),
        (0.4375, 0.9990),
        (0.4, 0.9990),
        (0.42, 0.9985),
        (0.45, 0.9990),
        (0.47, 0.9990),
        (0.44, 0.9970),
        (0.48, 0.9970),
        // --- 相称频率（网格梳；越靠后越坏）---
        (13.0 / 30.0, 0.9960),
        (7.0 / 15.0, 0.9935),
        (6.0 / 13.0, 0.9915),
        (4.0 / 11.0, 0.9885),
        (5.0 / 11.0, 0.9885),
        (4.0 / 9.0, 0.9835),
    ];

    /// 冻结的 **4× 核表**：改建前本模块里那张表的原样字面量。
    ///
    /// 它**不是**兼容层（公开 API 里已经没有任何 4× 路径，见 D43 / HD-26），
    /// 而是"改建前"的**对照仪器**：让
    /// [`eight_times_oversampling_fixes_the_four_tenths_nyquist_underread`]
    /// 能在**同一夹具、同一段测量代码**上同时给出 4× 与 8× 的读数，
    /// 而不是只在文档里留一个孤立的数字。
    #[rustfmt::skip]
    const FROZEN_FOUR_TIMES_KERNEL: [[f32; 16]; 4] = [
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

    /// 冻结 4× 核表的**最小复刻**（自带窗口状态；只用于对照测量，不属于公开面）。
    struct FrozenFourTimes {
        window: [f32; 16],
    }

    impl FrozenFourTimes {
        fn new() -> Self {
            Self { window: [0.0; 16] }
        }

        /// 与改建前 `TruePeakDetector::process` 逐行同构（只是核表是 4×/16 抽头）。
        fn process(&mut self, samples: &[f32]) -> f32 {
            let mut block_peak = 0.0f32;
            for &raw in samples {
                let sample = sanitize_sample(raw);
                let mut index = 15;
                while index > 0 {
                    self.window[index] = self.window[index - 1];
                    index -= 1;
                }
                self.window[0] = sample;
                for kernel in &FROZEN_FOUR_TIMES_KERNEL {
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
            block_peak
        }
    }

    /// 以 `fs` 为单位的正弦夹具（相位用 `f64` 算，避免大自变量的 `f32` 正弦漂移）。
    fn sine_at(cycles_per_sample: f64, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| (std::f64::consts::TAU * cycles_per_sample * i as f64).sin() as f32)
            .collect()
    }

    /// **稳态**真峰值：先喂 `warm` 个样本把窗填满（冷启动瞬态见
    /// [`TruePeakDetector`] 文档第 2 条），再在余下样本上取峰值。
    fn steady_true_peak(detector: &mut TruePeakDetector, samples: &[f32], warm: usize) -> f32 {
        let (head, tail) = samples.split_at(warm);
        let _ = detector.process(head);
        detector.process(tail)
    }

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
            "8× 真峰值应抓到 1.0 的过冲, 实际 {block_peak} ({:.3} dBFS)",
            dbfs(block_peak)
        );
        assert!(
            block_peak >= sample_peak,
            "真峰值必须 ≥ 采样峰值 (真峰值包含原始样本)"
        );
        assert_eq!(detector.true_peak(), block_peak);
        assert_eq!(detector.oversampling(), TRUE_PEAK_PHASES);
        assert_eq!(detector.process(&[]), 0.0, "空切片返回 0");
        assert_eq!(detector.true_peak(), block_peak, "空切片不改全局峰值");
    }

    /// 判据（**改建前后的实测对照**，HD-26 的主判据）：
    /// `f = 0.4·fs` 处的固有欠读被 8× 显著收紧。
    ///
    /// 4× 的网格步长是 `1/4` 样本，而连续峰值落在 `t = 5/8` 样本上 ⇒ 读
    /// `cos(π/10) = 0.9511`（−0.436 dB）。8× 的网格步长是 `1/8`，`5/8` **正好在网格上**
    /// ⇒ 读数回到 1.0 附近（残余只是核的带边误差）。
    ///
    /// 注入：把 [`TRUE_PEAK_PHASES`] 改回 4（或把核表换成 4×）⇒ 8× 的断言变红。
    #[test]
    fn eight_times_oversampling_fixes_the_four_tenths_nyquist_underread() {
        const WARM: usize = 512;
        let samples = sine_at(0.4, 8192);

        let mut reference = FrozenFourTimes::new();
        let _ = reference.process(&samples[..WARM]);
        let four_times = reference.process(&samples[WARM..]);

        let mut detector = TruePeakDetector::new();
        let eight_times = steady_true_peak(&mut detector, &samples, WARM);

        // 改建前: 4× 读到 cos(π/10)。
        let expected_four = (std::f64::consts::PI / 10.0).cos() as f32;
        assert!(
            (four_times - expected_four).abs() < 2e-3,
            "构造前提: 4× 应读到 {expected_four}, 实际 {four_times} ({:.4} dBFS)",
            dbfs(four_times)
        );
        // 改建后: 8× 必须把欠读从 0.436 dB 收到 0.01 dB 以内。
        assert!(
            (eight_times - 1.0).abs() < 1e-2,
            "8× 在 f=0.4·fs 应读到 1.0 附近, 实际 {eight_times} ({:.4} dBFS)",
            dbfs(eight_times)
        );
        let improvement_db = dbfs(eight_times) - dbfs(four_times);
        assert!(
            improvement_db > 0.4,
            "8× 相对 4× 的改善应 > 0.4 dB, 实际 {improvement_db:.4} dB \
             (4× {four_times} → 8× {eight_times})"
        );
        // 同一夹具上如实报告两个读数（判据失败时上面两条会带上实际值）。
        assert!(four_times < 0.96, "4× 的欠读必须被如实记录: {four_times}");
    }

    /// 判据：**稳态欠读的逐频点预算**（相称频率上的网格梳也不能超）。
    ///
    /// 实测（[`TRUE_PEAK_STEADY_FLOOR`] 的 23 个频点，4096 样本、前 512 样本填窗）：
    ///
    /// | 频点 | 8× 读数 | dB | 4× 会读到 |
    /// | :--- | ---: | ---: | ---: |
    /// | `0.4·fs` | 0.999942541 | **−0.0005** | 0.951056540（**−0.4359**） |
    /// | `0.42·fs` | 0.999703526 | −0.0026 | 0.998026729 |
    /// | `0.44 / 0.48·fs` | 0.998026729 | −0.0172 | 0.998026729 |
    /// | `5/11·fs` | 0.989821434 | −0.0889 | 0.989821434 |
    /// | **`4/9·fs`（最坏）** | **0.984807730** | **−0.1330** | 0.984807730 |
    ///
    /// **最坏欠读 −0.1330 dB 落在 `4/9·fs`（相称频率的网格梳）**，不是 `0.4·fs`；
    /// `0.4·fs` 那一列的改善（−0.4359 → −0.0005 dB）才是 `HD-26` 的靶心。
    /// 两件事都由这条判据钉住：预算逐点写死，最坏值也在注释里给出。
    ///
    /// 注入：回到 4× 网格 ⇒ `0.4·fs` 一项立刻跌到 0.9511（< 0.9990）⇒ 变红。
    #[test]
    fn true_peak_steady_state_underread_is_within_the_documented_budget() {
        const WARM: usize = 512;
        let mut worst = (f32::MAX, 0.0f64);
        for &(frequency, floor) in TRUE_PEAK_STEADY_FLOOR {
            let samples = sine_at(frequency, 4096);
            let mut detector = TruePeakDetector::new();
            let reading = steady_true_peak(&mut detector, &samples, WARM);
            assert!(
                reading >= floor,
                "8× 在 f={frequency}·fs 的稳态读数是 {reading} ({:.4} dBFS) —— \
                 低于该频点的预算 {floor}",
                dbfs(reading)
            );
            if reading < worst.0 {
                worst = (reading, frequency);
            }
        }
        // 最坏点与最坏值都必须与文档一致（换核/换倍数会立刻挪动它们）。
        assert!(
            (worst.1 - 4.0 / 9.0).abs() < 1e-9,
            "8× 的最坏频点应是 4/9·fs, 实际 {}·fs",
            worst.1
        );
        assert!(
            dbfs(worst.0) > -0.14 && dbfs(worst.0) < -0.12,
            "8× 的最坏稳态欠读应是 −0.133 dB 量级, 实际 {:.4} dB @ {}·fs",
            dbfs(worst.0),
            worst.1
        );
        // 母带最关心的两个点必须 ≥ 0.999。
        for frequency in [0.25f64, 0.4] {
            let samples = sine_at(frequency, 4096);
            let mut detector = TruePeakDetector::new();
            let reading = steady_true_peak(&mut detector, &samples, WARM);
            assert!(
                reading >= 0.999,
                "母带关心的 f={frequency}·fs 必须 ≥ 0.999, 实际 {reading}"
            );
        }
    }

    /// 判据：**真峰值 ≥ 采样峰值恒成立**（插值核的硬性质），**不给容差**。
    ///
    /// 依据：相位 0 是精确的 `δ(m − 16)` ⇒ 每个输入样本在延迟 16 个样本之后
    /// 被**原样**读出。于是有两层保证，本判据分别钉住：
    ///
    /// 1. **块内**：一次 `process` 的输出包含 `x[0 .. len−TAPS]` 的原样副本
    ///    （最后 `TAPS` 个样本要到下一次调用才会到达 `δ` 的抽头位置）；
    /// 2. **全局**：再喂 `TAPS` 个静音把窗冲干净之后，`true_peak()` 必须 ≥
    ///    整段的采样峰值 —— 这才是"真峰值 ≥ 采样峰值"的完整语义。
    ///
    /// 后者顺带把**延迟语义**钉住了：若把 `TRUE_PEAK_LATENCY_SAMPLES` 与核里 δ 的位置
    /// 弄错，第 2 层保证会立刻变红（样本永远到不了 δ 的抽头）。
    #[test]
    fn true_peak_preserves_the_sample_grid() {
        let mut fixtures: Vec<Vec<f32>> = vec![
            sine_at(0.4, 2048),
            sine_at(0.123_456_7, 2048),
            vec![0.5f32; 512],
            vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0e30, 0.25],
        ];
        let mut state = 0x1234_5678u32;
        let mut noise = Vec::new();
        for _ in 0..2048 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            noise.push((state as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
        fixtures.push(noise);

        for samples in fixtures {
            let sample_peak = samples
                .iter()
                .fold(0.0f32, |m, x| m.max(sanitize_sample(*x).abs()));
            let reproduced = samples.len().saturating_sub(TRUE_PEAK_TAPS);
            let guaranteed_peak = samples[..reproduced]
                .iter()
                .fold(0.0f32, |m, x| m.max(sanitize_sample(*x).abs()));
            for oversampling in [TRUE_PEAK_PHASES, TRUE_PEAK_PHASES_HIGH] {
                let mut detector =
                    TruePeakDetector::with_oversampling(oversampling).expect("8×/16× 都必须可选");
                let block_peak = detector.process(&samples);
                assert!(
                    block_peak >= guaranteed_peak,
                    "{oversampling}× 的块内真峰值 {block_peak} 低于前 {reproduced} 个样本的峰值 \
                     {guaranteed_peak}"
                );
                // 冲干净窗口: 现在全局峰值必须覆盖**整段**。
                let _ = detector.process(&[0.0f32; TRUE_PEAK_TAPS]);
                assert!(
                    detector.true_peak() >= sample_peak,
                    "{oversampling}× 的全局真峰值 {} 低于整段采样峰值 {sample_peak}",
                    detector.true_peak()
                );
            }
        }
    }

    /// 判据：只接受 8× 与 16×；其它倍数**响亮失败**而不是静默回落。
    #[test]
    fn true_peak_oversampling_is_either_eight_or_sixteen() {
        assert_eq!(TruePeakDetector::new().oversampling(), 8);
        assert_eq!(TRUE_PEAK_PHASES, 8);
        assert_eq!(TRUE_PEAK_PHASES_HIGH, 16);
        assert!(TruePeakDetector::with_oversampling(8).is_some());
        assert!(TruePeakDetector::with_oversampling(16).is_some());
        for rejected in [0usize, 1, 2, 4, 12, 32, 64, usize::MAX] {
            assert!(
                TruePeakDetector::with_oversampling(rejected).is_none(),
                "{rejected}× 没有被核验过的核表, 必须拒绝而不是硬套 8×"
            );
        }
        // 延迟与抽头数同步(改 TAPS 而忘记延迟会让调用方对不齐)。
        assert_eq!(TRUE_PEAK_TAPS, 32);
        assert_eq!(TRUE_PEAK_LATENCY_SAMPLES, TRUE_PEAK_TAPS / 2);
        assert_eq!(TRUE_PEAK_KAISER_BETA, 8.0);
    }

    /// 判据：**16× 的偶数相位与 8× 的相位逐位相同**（16× 的网格是 8× 的超集）。
    ///
    /// 这条是"16× 不会比 8× 差"的**结构性**依据：同一输入下两者的窗口状态相同，
    /// 共享网格点上的乘加序列也相同 ⇒ `16× 的读数 ≥ 8× 的读数`（逐位成立）。
    #[test]
    fn sixteen_times_contains_eight_times_phases_bit_for_bit() {
        for (phase, kernel) in TRUE_PEAK_KERNEL_8X.iter().enumerate() {
            for (tap, value) in kernel.iter().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    TRUE_PEAK_KERNEL_16X[phase * 2][tap].to_bits(),
                    "16× 的相位 {} 与 8× 的相位 {phase} 在抽头 {tap} 处不同",
                    phase * 2
                );
            }
        }
    }

    /// 判据：**要不要 16×** 的实测依据。
    ///
    /// 1. **结构性**：16× 表的偶数相位与 8× 表逐位相同，而 16× 的网格是 8× 的**超集**
    ///    ⇒ 同一输入下 `16× 的读数 ≥ 8× 的读数`（逐位成立，零容差）；
    /// 2. **实测收益**（[`TRUE_PEAK_STEADY_FLOOR`] 的 23 个频点）：
    ///
    /// | 量 | 8× | 16× |
    /// | :--- | ---: | ---: |
    /// | 最坏读数所在的频点 | **`4/9·fs`** | **`6/13·fs`** |
    /// | 最坏读数 | 0.984807730（**−0.1330 dB**） | 0.992708862（**−0.0636 dB**） |
    /// | `4/9·fs` 处的读数 | 0.984807730 | 0.996883929（**+0.1059 dB**） |
    ///
    /// 最坏值收紧 **+0.0694 dB**、最坏点处收紧 **+0.1059 dB** —— 这**不是**"零收益"：
    /// 上一版判据只覆盖了 `0.02…0.48` 的 16 个频点，恰好漏掉了最坏的相称频率
    /// （`4/9` 与 `5/11`），于是得出"收益 < 0.01 dB"的**错误**结论；本判据把
    /// 相称频率补进表里之后结论被修正（notes §2.5 记录了这个修正）。
    ///
    /// 成本：2× 乘加（512 vs 256 MAC/样本）与 2× 表体积。
    /// **结论**：默认 8×（逐通道计量，成本敏感，最坏 −0.133 dB，比 4× 的 −0.436 dB
    /// 收紧 0.303 dB）；**母带母线/导出天花板用 16×**（"只付一次"的场景，
    /// 把最坏欠读压到 −0.064 dB）。
    ///
    /// 注入：改错 16× 表的偶数相位 ⇒ `16× ≥ 8×` 或逐位比较变红；
    /// 把默认倍数退回 4× ⇒ 本判据的收益断言也会红。
    #[test]
    fn sixteen_times_tightens_the_worst_case_grid_comb_by_the_documented_margin() {
        const WARM: usize = 512;
        let mut worst = ((f32::MAX, 0.0f64), (f32::MAX, 0.0f64));
        let mut gain_at_four_ninths = 0.0f64;
        for &(frequency, _) in TRUE_PEAK_STEADY_FLOOR {
            let samples = sine_at(frequency, 4096);
            let mut eight = TruePeakDetector::new();
            let mut sixteen =
                TruePeakDetector::with_oversampling(TRUE_PEAK_PHASES_HIGH).expect("16×");
            let a = steady_true_peak(&mut eight, &samples, WARM);
            let b = steady_true_peak(&mut sixteen, &samples, WARM);
            assert!(
                b >= a,
                "16× 的网格是 8× 的超集, 读数不得更小: f={frequency}·fs {b} vs {a}"
            );
            if a < worst.0.0 {
                worst.0 = (a, frequency);
            }
            if b < worst.1.0 {
                worst.1 = (b, frequency);
            }
            if (frequency - 4.0 / 9.0).abs() < 1e-9 {
                gain_at_four_ninths = f64::from(dbfs(b)) - f64::from(dbfs(a));
            }
        }
        assert!(
            (worst.0.1 - 4.0 / 9.0).abs() < 1e-9,
            "8× 的最坏频点应是 4/9·fs, 实际 {}",
            worst.0.1
        );
        assert!(
            (worst.1.1 - 6.0 / 13.0).abs() < 1e-9,
            "16× 的最坏频点应是 6/13·fs, 实际 {}",
            worst.1.1
        );
        assert!(worst.0.0 >= 0.9835, "8× 最坏读数 {}", worst.0.0);
        assert!(worst.1.0 >= 0.9915, "16× 最坏读数 {}", worst.1.0);
        let worst_gain = f64::from(dbfs(worst.1.0)) - f64::from(dbfs(worst.0.0));
        assert!(
            (0.05..0.10).contains(&worst_gain),
            "最坏值收紧量应在 0.05–0.10 dB 之间(实测 0.0694), 实际 {worst_gain:.4} dB"
        );
        assert!(
            (0.08..0.13).contains(&gain_at_four_ninths),
            "4/9·fs 处的收益应在 0.08–0.13 dB 之间(实测 0.1059), 实际 {gain_at_four_ninths:.4} dB"
        );
    }

    /// 判据：直流与低频**近位**透明（每相位直流增益归一化为 1）。
    ///
    /// 实测：直流 0.5 的稳态读数是 `0.50000012`（+1.2e-7，来自 f32 累加），
    /// 低频正弦（3 个周期 / 4096 点 ⇒ `f ≈ 7.3e-4·fs`）的读数是 0.25 的
    /// 1−1.3e-4 ⇒ 容差 2e-3 覆盖"网格梳 + 窗残余"，比任何真实伪影都紧。
    #[test]
    fn true_peak_is_transparent_at_dc_and_low_frequency() {
        // ⚠ FIR 从零状态起步时, 直流**阶跃**会被窗化 sinc 的瞬态读到
        // (实测 0.5 → 0.6 量级)。这是本方案的固有瞬态, 不是稳态失准:
        // 先喂满一个窗(TAPS=32)再加余量, 稳态必须透明。
        let mut detector = TruePeakDetector::new();
        let _warm_up = detector.process(&[0.5f32; 128]);
        let dc = detector.process(&[0.5f32; 256]);
        assert!(
            (dc - 0.5).abs() < 1e-6,
            "稳态直流 0.5 的真峰值必须是 0.5, 实际 {dc}"
        );
        // 低频正弦: 稳态峰值就是幅度(容差含网格梳与窗函数的残余)。
        let low = sine(0.25, 3.0, 4096);
        let mut detector = TruePeakDetector::new();
        let _warm_up = detector.process(&low[..512]);
        let peak = detector.process(&low[512..]);
        assert!(
            (peak - 0.25).abs() < 2e-3,
            "低频正弦真峰值应约 0.25, 实际 {peak}"
        );
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

    /// 判据：核表自洽——每相位直流增益 1、相位 0 是 delta、带内频响 ≈ 1。
    ///
    /// 覆盖 8× 与 16× 两张表。频响断言分两段（这是本方案**实测**出来的形状）：
    ///
    /// - `f ≤ 0.42·fs`：**每个相位**的 `|H_p(f)|` 都落在 `1 ± 2e-3`
    ///   （实测最坏 −7.6e-4 @ 0.42）；
    /// - `f ≥ 0.44·fs`：单个相位开始明显下垂（0.45 处 −7.4e-2，0.49 处 −0.73），
    ///   但**检测器取的是"相位 × 网格"的最大值**，所以读数仍然紧
    ///   （扫频判据 `true_peak_steady_state_underread_is_within_the_documented_budget`
    ///   把它钉在 −0.0172 dB 以内）。单相位下垂本身不是 bug，是 T=32 的带边极限。
    ///
    /// 注入：改表里任意一个系数（哪怕一位小数）⇒ 直流增益或频响断言变红。
    #[test]
    fn true_peak_kernel_is_coherent() {
        let tables: [(&str, &[[f32; TRUE_PEAK_TAPS]]); 2] =
            [("8x", &TRUE_PEAK_KERNEL_8X), ("16x", &TRUE_PEAK_KERNEL_16X)];
        for (name, table) in tables {
            assert_eq!(table.len(), if name == "8x" { 8 } else { 16 });
            // 相位 0 必须是 δ(m − TRUE_PEAK_LATENCY_SAMPLES)。
            for (m, value) in table[0].iter().enumerate() {
                let expected = if m == TRUE_PEAK_LATENCY_SAMPLES {
                    1.0
                } else {
                    0.0
                };
                assert_eq!(*value, expected, "{name} 的相位 0 在 m={m} 处不是 delta");
            }
            // 带边必须**如实**下垂：相位 0 是精确 δ(|H| ≡ 1), 但总有相位在 0.49
            // 处掉到 0.6 以下(实测 min = 0.2655)—— 若哪天被人为"修平", 这条会红,
            // 提醒我们核表被换过, 那时 notes 与预算要一起改。
            let edge_min = table
                .iter()
                .map(|kernel| magnitude_at(kernel, 0.49))
                .fold(f64::MAX, f64::min);
            assert!(
                edge_min < 0.6,
                "{name} 在 f=0.49 处的最小相位幅度是 {edge_min}"
            );
            for (phase, kernel) in table.iter().enumerate() {
                let dc: f64 = kernel.iter().map(|v| f64::from(*v)).sum();
                assert!(
                    (dc - 1.0).abs() < 1e-6,
                    "{name} 相位 {phase} 的直流增益是 {dc}, 不是 1"
                );
                for frequency in [0.01f64, 0.05, 0.1, 0.2, 0.25, 0.3, 0.35, 0.4, 0.42] {
                    let magnitude = magnitude_at(kernel, frequency);
                    assert!(
                        (magnitude - 1.0).abs() < 2e-3,
                        "{name} 相位 {phase} 在 f={frequency} 处幅度是 {magnitude}"
                    );
                }
                // 相位 0 是精确 δ ⇒ 它的 |H| 恒为 1; 其余相位在带边**如实**下垂。
                if phase == 0 {
                    assert_eq!(magnitude_at(kernel, 0.49), 1.0);
                }
            }
        }
    }

    /// 核在频率 `f`（单位 `fs`）处的幅度响应 `|H_p(f)|`。
    fn magnitude_at(kernel: &[f32; TRUE_PEAK_TAPS], frequency: f64) -> f64 {
        let omega = -std::f64::consts::TAU * frequency;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (m, value) in kernel.iter().enumerate() {
            let angle = omega * m as f64;
            re += f64::from(*value) * angle.cos();
            im += f64::from(*value) * angle.sin();
        }
        (re * re + im * im).sqrt()
    }
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

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
        // 分块喂入必须与一次喂入逐位相同(窗口状态是连续推进的)。
        let mut chunked = TruePeakDetector::new();
        let mut whole = TruePeakDetector::new();
        for chunk in block.chunks(64) {
            let _ = chunked.process(chunk);
        }
        let _ = whole.process(&block);
        assert_eq!(chunked.true_peak().to_bits(), whole.true_peak().to_bits());
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

    /// **判据（新写，可红）**：真峰值**上报**的延迟必须等于多相核的群延迟。
    ///
    /// 量什么：`TruePeakDetector::latency_samples()` 的读数（单位：样本），
    /// 以及 `TRUE_PEAK_TAPS` / `TRUE_PEAK_LATENCY_SAMPLES` 的**字面值**。
    ///
    /// 为什么需要它（机械读数）：既有判据只钉住 `TRUE_PEAK_LATENCY_SAMPLES ==
    /// TRUE_PEAK_TAPS / 2`（**常量对常量**）与核的相干性，**没有一条**把
    /// **上报口径**与那个常量绑起来。把访问器的返回从 `TRUE_PEAK_LATENCY_SAMPLES`
    /// 改成 `TRUE_PEAK_LATENCY_SAMPLES + 1` 时，全库 437 条判据**全绿**
    /// （实测：本票 48 次注入里的 L11）。上报值差一个样本，调用方的 PDC 补偿
    /// 就整体差一个样本。
    ///
    /// 字面值断言是刻意的：若 `TRUE_PEAK_LATENCY_SAMPLES` 与 `TRUE_PEAK_TAPS`
    /// **一起**被改（例如 `34` 与 `17`），"常量对常量"的判据仍然绿。
    /// 注入实测：改访问器的返回 ⇒ 本判据变红。
    #[test]
    fn the_reported_true_peak_latency_is_the_kernel_centre() {
        // 字面值（不是常量对常量）。
        assert_eq!(TRUE_PEAK_TAPS, 32);
        assert_eq!(TRUE_PEAK_LATENCY_SAMPLES, 16);
        // 上报口径 == 核的 sinc 中心 == 常量。
        assert_eq!(
            TruePeakDetector::new().latency_samples(),
            TRUE_PEAK_LATENCY_SAMPLES,
            "默认 8× 表的上报延迟必须等于核中心"
        );
        assert_eq!(
            TruePeakDetector::with_oversampling(TRUE_PEAK_PHASES_HIGH)
                .expect("16× 表存在")
                .latency_samples(),
            TRUE_PEAK_LATENCY_SAMPLES,
            "上报口径与过采样倍数无关（两张表共用同一个抽头数）"
        );
        // 处理与复位都不改这个读数（它是编译期常量返回）。
        let mut detector = TruePeakDetector::new();
        let _ = detector.process(&[1.0f32; 64]);
        detector.reset();
        assert_eq!(detector.latency_samples(), TRUE_PEAK_LATENCY_SAMPLES);
    }
    // 平台感知（R135／裁决 R24-R25）：本位型比较只在本架构（冻结架构）上有意义，
    // 异平台不上比对绝对值（CI 的 windows 腿只跑非位型部分）。

    /// **判据（新写，可红）**：真峰值读数**恒有限**（`+inf` 不可达）。
    ///
    /// 量什么：`TruePeakDetector`（默认 8×）对四种常量输入各喂 `4 096` 帧之后
    /// `true_peak()` 的读数（线性幅度），以及 `±inf` / `NaN` / 静音的**位型相等性**。
    ///
    /// 为什么需要它：⭐ **本 crate 唯一的"契约在别处被依赖"项**。
    /// `yeban-render` 的 `ExportPreset::apply_at` 里有一行
    /// `before.true_peak_dbtp.is_finite()`；那一行**是真守卫还是文档**，
    /// 完全取决于本判据钉住的性质：
    ///
    /// - 若 `true_peak()` 能取到 `+inf` ⇒ `allowed_db = ceiling − (+inf) = −inf` ⇒
    ///   `−inf < gain_db` 为真 ⇒ 真的进入上限分支 ⇒ 那行是**真守卫**；
    /// - 若恒有限（本判据钉住）⇒ `allowed_db` 只可能是有限值或 `+inf` ⇒
    ///   `+inf < gain_db` 恒假 ⇒ 那行**冗余**（文档价值 > 防线价值）。
    ///
    /// 机理：`sanitize_sample` 把 `NaN → 0`、`±inf → ±MAX_LINEAR_MAGNITUDE`、
    /// 有限值钳进 `±16`；核系数是定长有限表（最大 `0.41999155`、`32` 个抽头）
    /// ⇒ `acc` 的绝对值有界 ⇒ `peak` 恒有限。
    ///
    /// 注入实测（第四批／本批）：把 `process` 里的
    /// `let sample = sanitize_sample(raw);` 换成 `let sample = raw;`
    /// ⇒ 本判据变红（`±inf` 输入给出 `+inf` 读数）。
    #[test]
    fn the_true_peak_reading_is_always_finite() {
        /// 观测帧数。
        const FRAMES: usize = 4_096;
        /// 每次 `process` 的块长。
        const CHUNK: usize = 128;
        let measure = |sample: f32| -> f32 {
            let mut detector = TruePeakDetector::new();
            let block = vec![sample; FRAMES];
            for chunk in block.chunks(CHUNK) {
                let _ = detector.process(chunk);
            }
            detector.true_peak()
        };
        let positive_infinity = measure(f32::INFINITY);
        let negative_infinity = measure(f32::NEG_INFINITY);
        let not_a_number = measure(f32::NAN);
        let silence = measure(0.0);
        for (label, value) in [
            ("+inf", positive_infinity),
            ("-inf", negative_infinity),
            ("NaN", not_a_number),
            ("0.0", silence),
        ] {
            assert!(
                value.is_finite(),
                "{label} 输入给出了非有限的真峰值 {value} ⇒ `+inf` 可达"
            );
            assert!(value >= 0.0, "{label} 输入给出了负的真峰值 {value}");
        }
        assert_eq!(
            positive_infinity.to_bits(),
            negative_infinity.to_bits(),
            "`±inf` 必须给出同一个读数（`sanitize_sample` 把它们钳到 `±MAX_LINEAR_MAGNITUDE`）"
        );
        assert_eq!(
            not_a_number.to_bits(),
            0.0f32.to_bits(),
            "`NaN` 必须钳到 `0.0`"
        );
        assert_eq!(silence.to_bits(), 0.0f32.to_bits(), "静音必须是 `0.0`");
        // 非空证明：±inf 的读数必须**超过**钳位幅度本身（插值核有增益），
        // 否则"恒有限"可能只是"钳到 16 拉倒"。
        assert!(
            positive_infinity >= MAX_LINEAR_MAGNITUDE,
            "±inf 的真峰值 {} 低于钳位幅度 {MAX_LINEAR_MAGNITUDE}",
            positive_infinity
        );
        // ⭐ **字面量锚点**（`yeban-render` 的具体数值也有锚了）：
        // `[±inf; N]` 的线性真峰值 = `18.195227`（`0x4191_8fd3`），即 `25.207 dBTP`。
        // 该数只经过核表的**乘加**（无超越函数）⇒ 属 IEEE 精确类 ⇒ 跨架构逐位相同，
        // 可以钉字面量（裁决 R24 不需要 ulp 预算）。
        assert_eq!(
            positive_infinity.to_bits(),
            0x4191_8fd3,
            "±inf 的线性真峰值位型漂移了（实得 {}）",
            positive_infinity
        );
        // ⚠ dB 侧**只断言区间**：`dbfs` 走 `f32::log10`（宿主 libm 的超越函数）
        // ⇒ 按裁决 R24/R25 不得跨平台钉精确值。容差取 0.01 dB（≈ 0.1% 线性）。
        let dbtp = crate::meter::dbfs(positive_infinity);
        assert!(
            (dbtp - 25.207).abs() < 0.01,
            "±inf 的 dBTP 读数 {dbtp} 偏离 25.207 超过 0.01 dB"
        );
    }
}
