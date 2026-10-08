//! 母带导出：**LRA（响度范围）测量**与**响度归一化导出预设**。
//!
//! 本模块补的是离线母带链上两个**此前不存在**的读数与一个**此前不存在**的动作:
//!
//! | 能力 | 本仓库此前的位置 |
//! | :--- | :--- |
//! | 门限积分响度（LUFS） | 已有: [`crate::loudness::GatedLoudness`]（`yeban-dsp`, ITU-R BS.1770-4） |
//! | 瞬时 / 短时窗口 | 已有: 同上（本模块**只读**它们的窗口, 不重算滤波器） |
//! | **LRA（响度范围）** | **不存在**。`crates/yeban-dsp/src/loudness.rs:52` 写"**LRA（响度范围）没有实现**"; `docs/ledger/dsp-loudness-notes.md:548`（N4）把它的归属写成"**后续母带线**" —— 本模块就是那条线 |
//! | **响度归一化（施加增益去够目标 LUFS）** | **不存在**。仓库里唯一的"归一化"是 `crates/yeban-mcp/src/domain/render.rs:1243` 的**峰值**归一化; 同文件 `:128` 明写"**不做响度归一化**" |
//!
//! ## 规范状态（先说清楚"有"与"没有"）
//!
//! **有的**（都只是"**显示**"要求, 不是"目标"要求）:
//!
//! - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md:245`:
//!   "主母带总线提供标准的 **LUFS (Momentary / Short-term / Integrated)** 与响度范围（LRA）数值显示"。
//!   ⚠ 该文档已于 2026-10-06 被负责人**自 Normative 降级**为 Advisory
//!   （`AGENTS.md` §1.3）⇒ 它是参考设计, 不是硬性实现规范。
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:461`（`ARCH-FMT-001`）: RF64 `bext` 要内嵌
//!   "响度元数据 EBU R128" —— 这是**容器字段**要求, 不规定这些字段怎么测出来。
//!
//! **没有的**（本模块的取舍因此是**工程选择**, 不是规范读数）:
//!
//! - `docs/ledger/mcp-tools-expansion-notes.md:621` 的登记: "架构 §5.2 只要求**显示** LUFS/LRA,
//!   **没有**任何'目标 / 达标 / 归一化到某 LUFS'的要求";
//! - 路线图里也没有响度目标条目（同处 `:622` 记录: `grep 'LUFS'` 只命中限制器那一句）。
//!
//! ⇒ **规范未定义 LRA 的算法, 也完全未定义响度归一化。** 本模块的两处口径因此逐条声明依据:
//!
//! 1. **LRA 的口径**取自本仓库自己登记的算法描述 —— `docs/ledger/dsp-loudness-notes.md:548`（N4）:
//!    "LRA 需要按 **−20 LU 相对门限**对 **3 s 短时值**做直方图（**1 s 步进**）"。
//!    本模块照这条实现, 因此 LRA = 相对门限后短时值的 **P95 − P10**。
//! 2. **百分位的估计方式**（本模块的取舍）: 对排序后的值做**线性插值**（等价于 R 语言 type 7 /
//!    numpy 默认）。理由: EBU Tech 3342 的原文 PDF 在本机不可机读（与 `Loudness::UNKNOWN`
//!    登记的同一类限制, 见 `crates/yeban-render/src/rf64.rs:281`）, 参考实现的直方图**分箱原点与
//!    箱宽**因此**无法核验**。线性插值是"不引入未核验常数"的最小选择。
//!    **我不能验证**: 本模块的 LRA 与 libebur128 等参考实现在分箱上是否逐位一致。
//! 3. **目标 LUFS 的数字**（[`STREAMING_TARGET_LUFS`] / [`EBU_R128_TARGET_LUFS`]）是**工程选择**。
//!    **我不能验证**这两个数字的权威原文（本机没有 EBU R128 / 流媒体平台规范的可机读副本）。
//!
//! ## 只读复用, 没有第二份响度实现
//!
//! 本模块**没有**一个 K 加权系数、**没有**一个滤波器状态、**没有**一条门限积分公式。
//! 它只做三件 `yeban-dsp` 不做的事:
//!
//! 1. 把 3 s 短时窗口**按 1 s 步进采样**出来（[`short_term_values`]）;
//! 2. 对采样出来的短时值做 **−20 LU 相对门限 + P95−P10**（[`loudness_range_from_short_term`]）;
//! 3. 按目标算增益并**施加**到母带缓冲上（[`ExportPreset::apply`]）。
//!
//! 判据 `short_term_values_match_a_hand_driven_meter` 钉住第 1 件事: 本模块的读数与**手工喂**
//! [`crate::loudness::GatedLoudness`] 得到的向量**逐位相同** ⇒ 不存在第二份窗口实现。
//!
//! ## 真峰值的两个口径声明
//!
//! - **过采样倍数 8×**（[`crate::loudness`] 的邻居 `yeban_dsp::meter::TruePeakDetector::new`）。
//!   `HD-26` 已裁决真峰值 4× → 8×（`docs/ledger/human-decisions.md:57`）, 且 8× 在 `0.4·fs`
//!   附近**仍有固有欠读**（同处记录 4× 欠读 0.44 dB）。**本模块不声称**真峰值上限是"不会削顶"的保证。
//! - **dB 换算一律走 `libm`**（[ARCH-DET-001] 的"不使用平台 libm"纪律）: 本模块用
//!   `libm::log10f` / `libm::pow`, **不用** `yeban_dsp::meter::dbfs`（它走 std 的 `f32::log10`）。
//!   两者在**最后一位**上可能不同。理由: 母带读数是可复算的交付物, 不该随宿主 libm 漂移。
//!
//! ## 边界（这一层没有证明什么）
//!
//! - **单声道口径**: 立体声接口把左右两路都算进能量（`G = 1`）。本模块**不**做"同信号喂两路"的
//!   单声道近似, 也不支持 >2 声道（与 `yeban-dsp` 的 N5 同一限制）。
//! - **不实现限制器**: 真峰值上限的做法是**减小增益**（让步响度目标）, 不是"加增益后削顶"。
//!   削顶是限幅器的职责（`ROAD-M2` 的 `yeban-dsp` 侧）, 本模块不代它做决定。
//! - **不做流式**: 全部接口都是一次性离线计算（本 crate 的承重契约就是"完全离线"）。
//!
//! ## 例
//!
//! ```
//! use yeban_render::mastering::{ExportPreset, GainBound};
//!
//! // 1 秒 997 Hz 的 −20 dBFS 立体声正弦: BS.1770 的标定点 ⇒ 读数 ≈ −20.0 LUFS。
//! let tone: Vec<f32> = (0..48_000)
//!     .map(|i| (0.1 * (std::f64::consts::TAU * 997.0 * i as f64 / 48_000.0).sin()) as f32)
//!     .collect();
//! let (mut left, mut right) = (tone.clone(), tone);
//!
//! let outcome = ExportPreset::new(Some(-14.0), None)
//!     .apply(48_000, &mut left, &mut right)
//!     .expect("48 kHz 是内置的四档采样率之一");
//!
//! assert_eq!(outcome.bound, GainBound::LoudnessTarget);
//! assert!((outcome.gain_db - 6.0).abs() < 0.05, "实际 {}", outcome.gain_db);
//! assert!((outcome.after.integrated_lufs + 14.0).abs() < 0.05);
//! ```

use crate::loudness::{ABSOLUTE_GATE_LUFS, GatedLoudness, LUFS_OFFSET_DB};
use crate::render::db_to_linear;
use crate::rf64::Loudness;
use yeban_dsp::meter::TruePeakDetector;

/// LRA 的**相对门限**（LU）: 相对"过了绝对门限的短时值的平均响度"再降 20 LU。
///
/// 与积分响度的相对门限 `−10 LU`（`crate::loudness::RELATIVE_GATE_LU`, BS.1770-4）**不是同一个数**:
/// LRA 用 `−20 LU`。依据 `docs/ledger/dsp-loudness-notes.md:548`（N4）"按 −20 LU 相对门限"。
pub const LRA_RELATIVE_GATE_LU: f64 = -20.0;

/// 流媒体常见的目标节目响度（LUFS）。**工程选择**: 规范未定义响度目标（模块文档 §"规范状态"）。
///
/// **未核实**: 本机没有该数字的权威原文副本。
pub const STREAMING_TARGET_LUFS: f32 = -14.0;

/// EBU R128 的目标节目响度（LUFS）。**工程选择**: 规范未定义响度目标（模块文档 §"规范状态"）。
///
/// **未核实**: 本机没有 EBU R128 原文的可机读副本。
pub const EBU_R128_TARGET_LUFS: f32 = -23.0;

/// 两个内置预设共用的真峰值上限（dBTP）。**工程选择**: 规范未定义真峰值上限。
pub const DEFAULT_TRUE_PEAK_CEILING_DBTP: f32 = -1.0;

/// 一次母带测量的全部读数（离线、一次性）。
///
/// 所有 dB 量都可能取到 `f32::NEG_INFINITY`, 它表示"**测不出**"而不是一个很大的负分贝:
/// 全零信号的 `true_peak_dbtp`、短于 400 ms 或整体低于绝对门限的 `integrated_lufs` 都是它。
/// [`Self::integrated_is_measurable`] 是给调用方的判断入口。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasterLoudness {
    /// 门限积分响度（LUFS）。测不出 ⇒ `NEG_INFINITY`。
    pub integrated_lufs: f32,
    /// 响度范围 LRA（LU）。样本不足 3 s、或没有任何短时值过门限 ⇒ `None`。
    pub loudness_range_lu: Option<f32>,
    /// 迄今最大的瞬时（400 ms）读数（LUFS）。从未填满窗口 ⇒ `NEG_INFINITY`。
    pub max_momentary_lufs: f32,
    /// 迄今最大的短时（3 s）读数（LUFS）。从未填满窗口 ⇒ `NEG_INFINITY`。
    pub max_short_term_lufs: f32,
    /// 真峰值（dBTP, 8× 过采样）。幅度为 0 ⇒ `NEG_INFINITY`。
    pub true_peak_dbtp: f32,
}

impl MasterLoudness {
    /// 门限积分响度是否**测得出**（有限数）。
    ///
    /// 归一化必须看这一条: 静音的响度是 `NEG_INFINITY`, 任何增益都够不到一个有限目标,
    /// 因此[`ExportPreset::apply`]在此时**不动样本**。
    #[must_use]
    pub fn integrated_is_measurable(&self) -> bool {
        self.integrated_lufs.is_finite()
    }

    /// 转成 `bext` v2 的 EBU R128 响度块（[ARCH-FMT-001], `crates/yeban-render/src/rf64.rs:258`）。
    ///
    /// **测不出的量写哨兵** [`Loudness::UNKNOWN`], 绝不写一个假的 0 或假的读数
    /// （与 `crates/yeban-mcp/src/domain/render.rs:1027` 的同一条纪律一致）。
    /// `loudness_range` 与其余字段共用 `×100` 的缩放（0.01 LU 与 0.01 LUFS 刻度相同）。
    #[must_use]
    pub fn to_bext_loudness(&self) -> Loudness {
        fn lufs(value: f32) -> i16 {
            if value.is_finite() {
                Loudness::from_lufs(value)
            } else {
                Loudness::UNKNOWN
            }
        }
        Loudness {
            loudness_value: lufs(self.integrated_lufs),
            loudness_range: match self.loudness_range_lu {
                Some(range) if range.is_finite() => Loudness::from_lufs(range),
                _ => Loudness::UNKNOWN,
            },
            max_true_peak_level: if self.true_peak_dbtp.is_finite() {
                Loudness::from_dbtp(self.true_peak_dbtp)
            } else {
                Loudness::UNKNOWN
            },
            max_momentary_loudness: lufs(self.max_momentary_lufs),
            max_short_term_loudness: lufs(self.max_short_term_lufs),
        }
    }
}

/// 增益是**被哪个约束定下来的**。
///
/// 归一化有两个约束（目标响度、真峰值上限）, 它们可能互相冲突。
/// 报告"谁赢了"比只报一个增益数字更有用: 调用方能据此告诉用户"目标没达到, 是上限挡的"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainBound {
    /// 增益由**目标响度**定下, 且没有突破真峰值上限。
    LoudnessTarget,
    /// 增益由**真峰值上限**定下 ⇒ 目标响度**没有**达到（上限赢了）。
    TruePeakCeiling,
    /// 没有任何约束需要改变样本（增益 0 dB）: 没给目标、上限本就满足, 或响度测不出。
    NothingToDo,
}

/// 一次归一化的结果: 实际增益 + 谁定下了它 + 前后两次测量。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizeOutcome {
    /// 实际施加的增益（dB）。`0.0` 表示样本**逐位未改动**。
    pub gain_db: f32,
    /// 定下这个增益的约束。
    pub bound: GainBound,
    /// 施加**之前**的测量。
    pub before: MasterLoudness,
    /// 施加**之后**的测量（重新测一遍, 不是推算的）。
    pub after: MasterLoudness,
}

/// 导出预设: 目标响度 + 真峰值上限。
///
/// 两个字段都可选, 因为它们的组合都有意义:
///
/// | `target_lufs` | `true_peak_ceiling_dbtp` | 行为 |
/// | :--- | :--- | :--- |
/// | `Some` | `None` | 纯响度归一化（可能把真峰值推过 0 dBTP） |
/// | `Some` | `Some` | 响度归一化 + 真峰值让步（上限赢时**减小**增益） |
/// | `None` | `Some` | 只做真峰值上限（超了就衰减, 不抬响度） |
/// | `None` | `None` | 不做任何事（[`GainBound::NothingToDo`]） |
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportPreset {
    /// 目标门限积分响度（LUFS）。
    target_lufs: Option<f32>,
    /// 真峰值上限（dBTP）。
    true_peak_ceiling_dbtp: Option<f32>,
}

impl ExportPreset {
    /// 由目标与上限构造。
    #[must_use]
    pub const fn new(target_lufs: Option<f32>, true_peak_ceiling_dbtp: Option<f32>) -> Self {
        Self {
            target_lufs,
            true_peak_ceiling_dbtp,
        }
    }

    /// 流媒体预设: [`STREAMING_TARGET_LUFS`] + [`DEFAULT_TRUE_PEAK_CEILING_DBTP`]。
    #[must_use]
    pub const fn streaming() -> Self {
        Self::new(
            Some(STREAMING_TARGET_LUFS),
            Some(DEFAULT_TRUE_PEAK_CEILING_DBTP),
        )
    }

    /// EBU R128 预设: [`EBU_R128_TARGET_LUFS`] + [`DEFAULT_TRUE_PEAK_CEILING_DBTP`]。
    #[must_use]
    pub const fn ebu_r128() -> Self {
        Self::new(
            Some(EBU_R128_TARGET_LUFS),
            Some(DEFAULT_TRUE_PEAK_CEILING_DBTP),
        )
    }

    /// 目标响度（LUFS）。
    #[must_use]
    pub const fn target_lufs(&self) -> Option<f32> {
        self.target_lufs
    }

    /// 真峰值上限（dBTP）。
    #[must_use]
    pub const fn true_peak_ceiling_dbtp(&self) -> Option<f32> {
        self.true_peak_ceiling_dbtp
    }

    /// 按预设**就地**给母带施加增益, 并返回前后两次测量。
    ///
    /// 采样率不在 `crate::loudness::K_WEIGHTING_SAMPLE_RATES_HZ`（44.1 / 48 / 88.2 / 96 kHz）
    /// 里 ⇒ `None`（**不静默回落**到 48 kHz 的系数, 那会给出错误读数）。
    ///
    /// 增益为 `0.0` 时**不碰任何样本**（逐位不变）—— 判据
    /// `a_zero_gain_preset_leaves_every_sample_bit_identical` 钉住这一点。
    /// 两声道按**较短者**测量并施加（与 [`measure_master`] 同一约定）。
    #[must_use]
    pub fn apply(
        &self,
        sample_rate: u32,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Option<NormalizeOutcome> {
        let before = measure_master(sample_rate, left, right)?;
        let mut gain_db = 0.0f32;
        let mut bound = GainBound::NothingToDo;

        if let Some(target) = self.target_lufs
            && before.integrated_is_measurable()
        {
            gain_db = target - before.integrated_lufs;
            bound = GainBound::LoudnessTarget;
        }

        if let Some(ceiling) = self.true_peak_ceiling_dbtp
            && before.true_peak_dbtp.is_finite()
        {
            // 上限允许多少增益: 真峰值加上它就正好碰到上限。
            let allowed_db = ceiling - before.true_peak_dbtp;
            if allowed_db < gain_db {
                gain_db = allowed_db;
                bound = GainBound::TruePeakCeiling;
            }
        }

        if gain_db != 0.0 {
            let linear = db_to_linear(gain_db);
            let frames = left.len().min(right.len());
            for sample in left[..frames].iter_mut().chain(right[..frames].iter_mut()) {
                *sample *= linear;
            }
        }

        let after = measure_master(sample_rate, left, right)?;
        Some(NormalizeOutcome {
            gain_db,
            bound,
            before,
            after,
        })
    }
}

/// 测一段母带的全部读数; 采样率不受支持 ⇒ `None`。
///
/// 信号被遍历三遍（短时窗口扫描、门限积分两遍）。这是**离线**接口, 代价可接受;
/// 实时路径不许调用它（本 crate 的契约本来就是"完全离线", 见 crate 的模块头文档）。
#[must_use]
pub fn measure_master(sample_rate: u32, left: &[f32], right: &[f32]) -> Option<MasterLoudness> {
    let rate = sample_rate as f32;
    let scan = scan_short_term(rate, left, right)?;
    let integrated_lufs = GatedLoudness::integrated_stereo_at(rate, left, right)?;

    let mut detector_left = TruePeakDetector::new();
    let mut detector_right = TruePeakDetector::new();
    detector_left.process(left);
    detector_right.process(right);
    let true_peak = detector_left.true_peak().max(detector_right.true_peak());

    Some(MasterLoudness {
        integrated_lufs,
        loudness_range_lu: loudness_range_from_short_term(&scan.short_terms),
        max_momentary_lufs: scan.max_momentary_lufs,
        max_short_term_lufs: scan.max_short_term_lufs,
        true_peak_dbtp: amplitude_to_dbtp(true_peak),
    })
}

/// 按 **1 s 步进**采样 3 s 短时窗口的读数（LUFS）; 采样率不受支持 ⇒ `None`。
///
/// 口径来自 `docs/ledger/dsp-loudness-notes.md:548`（N4）:"按 −20 LU 相对门限对 3 s 短时值
/// 做直方图（**1 s 步进**）"。未满一个整秒的尾巴**不产生**读数。
///
/// **前两条必是 `NEG_INFINITY`**: 3 s 窗口在 t = 3 s 才第一次填满
/// （`crate::loudness::GatedLoudness::short_term_lufs` 的文档: "窗口还没满 ⇒ 负无穷"）。
/// 它们是"窗口未满", 不是"很安静"; [`loudness_range_from_short_term`] 会把它们门限掉。
#[must_use]
pub fn short_term_values(sample_rate: u32, left: &[f32], right: &[f32]) -> Option<Vec<f32>> {
    scan_short_term(sample_rate as f32, left, right).map(|scan| scan.short_terms)
}

/// 短时扫描的内部结果: 1 s 步进的短时值 + 窗口最大值。
struct Scan {
    short_terms: Vec<f32>,
    max_momentary_lufs: f32,
    max_short_term_lufs: f32,
}

/// 用一个 [`GatedLoudness`] 喂完整段信号, 每 **1 s** 读一次短时值。
fn scan_short_term(sample_rate: f32, left: &[f32], right: &[f32]) -> Option<Scan> {
    let mut meter = GatedLoudness::for_sample_rate(sample_rate)?;
    let step = sample_rate.round() as usize;
    if step == 0 {
        return None;
    }
    let frames = left.len().min(right.len());
    let mut short_terms = Vec::with_capacity(frames / step + 1);
    let mut start = 0usize;
    while start + step <= frames {
        let end = start + step;
        meter.add_stereo(&left[start..end], &right[start..end]);
        short_terms.push(meter.short_term_lufs());
        start = end;
    }
    Some(Scan {
        short_terms,
        max_momentary_lufs: meter.max_momentary_lufs(),
        max_short_term_lufs: meter.max_short_term_lufs(),
    })
}

/// 由 1 s 步进的短时值算 **LRA**（LU）; 没有任何值过门限 ⇒ `None`。
///
/// 三步（模块文档 §"规范状态" 有依据与未核实项）:
///
/// 1. **绝对门限** `Γa = −70 LUFS`（`crate::loudness::ABSOLUTE_GATE_LUFS`, BS.1770-4）:
///    留下有限且**严格大于** `Γa` 的短时值。`NEG_INFINITY`（窗口未满）在这里被丢掉。
/// 2. **相对门限** `Γr = (过 Γa 者的平均响度) + LRA_RELATIVE_GATE_LU`（即 `−20 LU`）;
///    留下**严格大于** `Γr` 的值。
/// 3. **LRA = P95 − P10**, 百分位按**线性插值**取（见私有函数 `percentile` 的注释）。
///
/// 结果被钳到 `≥ 0`（浮点误差可能让 P95 比 P10 低一个最低有效位）。
#[must_use]
pub fn loudness_range_from_short_term(values: &[f32]) -> Option<f32> {
    let mut gated: Vec<f32> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite() && f64::from(*value) > ABSOLUTE_GATE_LUFS)
        .collect();
    if gated.is_empty() {
        return None;
    }

    let mean_power = gated
        .iter()
        .map(|value| loudness_to_power(f64::from(*value)))
        .sum::<f64>()
        / gated.len() as f64;
    let relative_gate = LUFS_OFFSET_DB + 10.0 * libm::log10(mean_power) + LRA_RELATIVE_GATE_LU;
    gated.retain(|value| f64::from(*value) > relative_gate);
    if gated.is_empty() {
        return None;
    }

    gated.sort_by(f32::total_cmp);
    let low = percentile(&gated, 0.10);
    let high = percentile(&gated, 0.95);
    Some((high - low).max(0.0))
}

/// 排序后的切片上取**线性插值**百分位（R type 7 / numpy 默认）。
///
/// `fraction = 0.10` ⇒ P10, `0.95` ⇒ P95。`sorted` 必须非空且升序。
fn percentile(sorted: &[f32], fraction: f64) -> f32 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = fraction * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let weight = (rank - lower as f64) as f32;
    sorted[lower] + weight * (sorted[upper] - sorted[lower])
}

/// LUFS ⇒ 均方能量（BS.1770-4 的 `LUFS = −0.691 + 10·log10(z)` 反解）。
fn loudness_to_power(lufs: f64) -> f64 {
    libm::pow(10.0, (lufs - LUFS_OFFSET_DB) / 10.0)
}

/// 线性幅度 ⇒ dBTP。
///
/// 走 `libm::log10f`（[ARCH-DET-001]: 不使用平台 libm）, 因此**不**复用
/// `yeban_dsp::meter::dbfs`（后者走 std 的 `f32::log10`）。`幅度 ≤ 0` ⇒ `NEG_INFINITY`。
fn amplitude_to_dbtp(amplitude: f32) -> f32 {
    if amplitude > 0.0 {
        20.0 * libm::log10f(amplitude)
    } else {
        f32::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 48 kHz 的 997 Hz 正弦（`amplitude` 是线性幅度）。
    fn sine_997(amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|index| {
                (f64::from(amplitude)
                    * (core::f64::consts::TAU * 997.0 * index as f64 / 48_000.0).sin())
                    as f32
            })
            .collect()
    }

    /// `short_term_values` 的读数与**手工喂**同一个 `GatedLoudness` **逐位相同**
    /// ⇒ 本模块没有第二份窗口实现。
    #[test]
    fn short_term_values_match_a_hand_driven_meter() {
        let left = sine_997(0.1, 48_000 * 8);
        let right = {
            let mut channel = sine_997(0.1, 48_000 * 8);
            channel.reverse();
            channel
        };

        let ours = short_term_values(48_000, &left, &right).expect("48 kHz");

        let mut meter = GatedLoudness::new_48k();
        let mut theirs = Vec::new();
        for chunk in 0..8 {
            let start = chunk * 48_000;
            meter.add_stereo(&left[start..start + 48_000], &right[start..start + 48_000]);
            theirs.push(meter.short_term_lufs());
        }

        assert_eq!(ours.len(), theirs.len(), "1 s 步进 ⇒ 8 条读数");
        assert_eq!(
            ours.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            theirs.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "本模块必须复用 yeban-dsp 的窗口, 而不是自己再算一遍"
        );
    }

    /// 3 s 窗口在 t = 3 s 才填满 ⇒ 前两条读数是 `NEG_INFINITY`。
    #[test]
    fn the_first_two_short_term_readings_are_the_unfilled_window() {
        let values = short_term_values(
            48_000,
            &sine_997(0.1, 48_000 * 5),
            &sine_997(0.1, 48_000 * 5),
        )
        .expect("48 kHz");
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], f32::NEG_INFINITY, "t = 1 s: 窗口只有 1 s");
        assert_eq!(values[1], f32::NEG_INFINITY, "t = 2 s: 窗口只有 2 s");
        assert!(values[2].is_finite(), "t = 3 s: 窗口第一次填满");
        assert!(values[3].is_finite());
        assert!(values[4].is_finite());
    }

    /// **手算判据**: 50 条 −20 LUFS + 50 条 −40 LUFS。
    ///
    /// 手算: `z(−20) = 0.011725`、`z(−40) = 1.1725e−4`;
    /// 均值 `z = (50·0.011725 + 50·1.1725e−4)/100 = 0.005921125`;
    /// `Γr = −0.691 + 10·log10(0.005921125) − 20 = −42.966` ⇒ 两组都过门限;
    /// `P10 = −40`、`P95 = −20` ⇒ LRA = **20.0 LU**。
    #[test]
    fn lra_of_two_levels_is_hand_computable() {
        let mut values = vec![-20.0f32; 50];
        values.extend(core::iter::repeat_n(-40.0f32, 50));
        let lra = loudness_range_from_short_term(&values).expect("有过门限的值");
        assert!((lra - 20.0).abs() < 1e-4, "手算 20.0 LU, 实际 {lra}");
    }

    /// **相对门限判据**: 99 条 −20 LUFS + 1 条 −60 LUFS。
    ///
    /// `Γr ≈ −40.04 LUFS`（手算见测试体注释）⇒ −60 那一条被相对门限丢掉,
    /// 只剩同一个值 ⇒ LRA = **0.0 LU**。若相对门限被去掉, 这条会变成 ≈ 9.5 LU ⇒ 变红。
    #[test]
    fn the_relative_gate_drops_the_quiet_tail() {
        let mut values = vec![-20.0f32; 99];
        values.push(-60.0);
        let lra = loudness_range_from_short_term(&values).expect("有过门限的值");
        assert!(lra < 1e-6, "相对门限后只剩 −20 ⇒ LRA 0.0, 实际 {lra}");
    }

    /// **绝对门限判据**: 全部低于 −70 LUFS ⇒ 没有值可算 ⇒ `None`。
    #[test]
    fn everything_below_the_absolute_gate_has_no_lra() {
        assert_eq!(loudness_range_from_short_term(&[-80.0, -75.0, -71.0]), None);
        assert_eq!(
            loudness_range_from_short_term(&[f32::NEG_INFINITY; 4]),
            None
        );
        assert_eq!(loudness_range_from_short_term(&[]), None);
    }

    /// 未满 3 s 的信号没有短时值 ⇒ LRA 是 `None`（不是 0, 也不是 NaN）。
    #[test]
    fn a_signal_shorter_than_the_window_has_no_lra() {
        let tone = sine_997(0.1, 48_000 * 2);
        let measured = measure_master(48_000, &tone, &tone).expect("48 kHz");
        assert_eq!(measured.loudness_range_lu, None);
        assert!(measured.integrated_is_measurable(), "2 s 够算积分响度");
    }

    /// 标定点走完整测量路径: 997 Hz、−20 dBFS 立体声 ⇒ `≈ −20.0 LUFS`。
    ///
    /// 同一信号在 8 s 上的 LRA 应当**很小**（稳态）, 但不必是 0: 3 s 窗口的头几条含
    /// K 加权滤波器的冷启动暂态。判据取 `< 1.0 LU` 而不是 `== 0.0`, 正是因为这个原因。
    #[test]
    fn the_calibration_tone_reads_minus_twenty_lufs() {
        let tone = sine_997(0.1, 48_000 * 8);
        let measured = measure_master(48_000, &tone, &tone).expect("48 kHz");
        assert!(
            (measured.integrated_lufs + 20.0).abs() < 0.05,
            "实际 {}",
            measured.integrated_lufs
        );
        let lra = measured.loudness_range_lu.expect("8 s > 3 s");
        assert!(lra < 1.0, "稳态正弦的 LRA 应当很小, 实际 {lra}");
        assert!(measured.max_short_term_lufs.is_finite());
        assert!(measured.max_momentary_lufs.is_finite());
        // 0.1 幅度的正弦, 真峰值 ≈ −20 dBTP（8× 过采样在 997 Hz 上几乎无过冲）。
        assert!(
            (measured.true_peak_dbtp + 20.0).abs() < 0.1,
            "实际 {}",
            measured.true_peak_dbtp
        );
    }

    /// 目标 −14 LUFS 的流媒体预设: −20 LUFS 的母带应当抬 **≈ +6 dB**。
    #[test]
    fn a_streaming_target_lifts_minus_twenty_to_minus_fourteen() {
        let (mut left, mut right) = (sine_997(0.1, 48_000 * 8), sine_997(0.1, 48_000 * 8));
        let outcome = ExportPreset::new(Some(-14.0), None)
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.bound, GainBound::LoudnessTarget);
        assert!(
            (outcome.gain_db - 6.0).abs() < 0.05,
            "实际 {} dB",
            outcome.gain_db
        );
        assert!(
            (outcome.after.integrated_lufs + 14.0).abs() < 0.05,
            "实际 {} LUFS",
            outcome.after.integrated_lufs
        );
    }

    /// 真峰值上限**赢**的时候: 目标会突破上限 ⇒ 增益被压到上限, 目标**达不到**。
    #[test]
    fn the_true_peak_ceiling_wins_over_the_loudness_target() {
        let (mut left, mut right) = (sine_997(0.1, 48_000 * 8), sine_997(0.1, 48_000 * 8));
        // 上限 −26 dBTP, 真峰值 ≈ −20 dBTP ⇒ 只允许 −6 dB。
        let outcome = ExportPreset::new(Some(-14.0), Some(-26.0))
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.bound, GainBound::TruePeakCeiling);
        assert!(
            (outcome.gain_db + 6.0).abs() < 0.05,
            "实际 {} dB",
            outcome.gain_db
        );
        assert!(
            outcome.after.true_peak_dbtp <= -26.0 + 1e-3,
            "真峰值必须在上限之内, 实际 {}",
            outcome.after.true_peak_dbtp
        );
        // 目标**没有**达到 —— 这正是"上限赢了"的可观测后果。
        assert!(
            outcome.after.integrated_lufs < -14.0 - 0.5,
            "上限赢时不应达到目标, 实际 {}",
            outcome.after.integrated_lufs
        );
    }

    /// 上限已经满足且没有响度目标 ⇒ 什么都不做, 样本**逐位**不变。
    #[test]
    fn a_satisfied_ceiling_changes_nothing() {
        let (mut left, mut right) = (sine_997(0.1, 48_000 * 4), sine_997(0.1, 48_000 * 4));
        let before = left.clone();
        let outcome = ExportPreset::new(None, Some(-1.0))
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.bound, GainBound::NothingToDo);
        assert_eq!(outcome.gain_db, 0.0);
        assert_eq!(
            left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            before.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }

    /// 增益 0 dB 的预设**逐位**不改样本（`db_to_linear(0.0) == 1.0` 且我们根本不乘）。
    #[test]
    fn a_zero_gain_preset_leaves_every_sample_bit_identical() {
        let (mut left, mut right) = (sine_997(0.1, 48_000 * 4), sine_997(0.1, 48_000 * 4));
        let (left_before, right_before) = (left.clone(), right.clone());
        // 目标 = 实测积分响度 ⇒ 增益 0。
        let measured = measure_master(48_000, &left, &right).expect("48 kHz");
        let outcome = ExportPreset::new(Some(measured.integrated_lufs), Some(-1.0))
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.gain_db, 0.0);
        assert_eq!(
            left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            left_before.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        assert_eq!(
            right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            right_before.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }

    /// 静音: 响度测不出 ⇒ **不动样本**, 不产生 NaN。
    #[test]
    fn silence_is_never_normalized_and_never_becomes_nan() {
        let (mut left, mut right) = (vec![0.0f32; 48_000 * 4], vec![0.0f32; 48_000 * 4]);
        let outcome = ExportPreset::streaming()
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.bound, GainBound::NothingToDo);
        assert_eq!(outcome.gain_db, 0.0);
        assert!(!outcome.before.integrated_is_measurable());
        assert_eq!(outcome.before.true_peak_dbtp, f32::NEG_INFINITY);
        assert!(left.iter().all(|value| *value == 0.0));
    }

    /// 采样率不在内置四档 ⇒ `None`, **不静默回落**。
    #[test]
    fn unsupported_sample_rates_are_rejected() {
        let tone = sine_997(0.1, 4_800);
        assert_eq!(measure_master(22_050, &tone, &tone), None);
        assert_eq!(short_term_values(22_050, &tone, &tone), None);
    }

    /// 两个具名预设的读数与构造一致（数字本身是工程选择, 见模块文档）。
    #[test]
    fn the_named_presets_expose_their_engineering_choices() {
        assert_eq!(ExportPreset::streaming().target_lufs(), Some(-14.0));
        assert_eq!(
            ExportPreset::streaming().true_peak_ceiling_dbtp(),
            Some(-1.0)
        );
        assert_eq!(ExportPreset::ebu_r128().target_lufs(), Some(-23.0));
        assert_eq!(
            ExportPreset::ebu_r128().true_peak_ceiling_dbtp(),
            Some(-1.0)
        );
    }

    /// EBU R128 预设对 −20 LUFS 的母带是**衰减**（−3 dB）。
    #[test]
    fn the_ebu_r128_preset_attenuates_a_hot_master() {
        let (mut left, mut right) = (sine_997(0.1, 48_000 * 8), sine_997(0.1, 48_000 * 8));
        let outcome = ExportPreset::ebu_r128()
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.bound, GainBound::LoudnessTarget);
        assert!(
            (outcome.gain_db + 3.0).abs() < 0.05,
            "实际 {} dB",
            outcome.gain_db
        );
        assert!((outcome.after.integrated_lufs + 23.0).abs() < 0.05);
    }

    /// `bext` v2 的响度块: 有限读数被 ×100 定标, 测不出的量写 `UNKNOWN`。
    #[test]
    fn the_bext_bridge_scales_finite_readings_and_sentinels_the_rest() {
        let tone = sine_997(0.1, 48_000 * 8);
        let measured = measure_master(48_000, &tone, &tone).expect("48 kHz");
        let bext = measured.to_bext_loudness();
        assert_eq!(bext.loudness_value, -2000, "−20.0 LUFS ⇒ −2000 (0.01 LUFS)");
        assert_ne!(bext.loudness_range, Loudness::UNKNOWN, "8 s 有 LRA 可写");
        assert_eq!(
            bext.max_true_peak_level, -2000,
            "−20.0 dBTP ⇒ −2000 (0.01 dBTP)"
        );

        let silence = measure_master(48_000, &[0.0f32; 48_000], &[0.0f32; 48_000]).expect("48 kHz");
        let bext = silence.to_bext_loudness();
        assert_eq!(bext.loudness_value, Loudness::UNKNOWN);
        assert_eq!(bext.loudness_range, Loudness::UNKNOWN);
        assert_eq!(bext.max_true_peak_level, Loudness::UNKNOWN);
        assert_eq!(bext.max_momentary_loudness, Loudness::UNKNOWN);
        assert_eq!(bext.max_short_term_loudness, Loudness::UNKNOWN);
    }

    /// `percentile` 的插值规则（R type 7）: 对 `[0, 10]` 取 P10 ⇒ `1.0`（不是 `0.0`）。
    #[test]
    fn percentile_interpolates_between_order_statistics() {
        let values = [0.0f32, 10.0];
        assert!((percentile(&values, 0.10) - 1.0).abs() < 1e-6);
        assert!((percentile(&values, 0.95) - 9.5).abs() < 1e-6);
        assert_eq!(percentile(&[3.0f32], 0.10), 3.0);
    }
}
