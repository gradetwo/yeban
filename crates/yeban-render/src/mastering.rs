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
//! ## 真峰值的口径: 母带这一侧用 **16×**（[`MASTERING_TRUE_PEAK_OVERSAMPLING`]）
//!
//! - **两个场景, 两个倍数**。上游台账 `docs/ledger/dsp-loudness-notes.md` 的 §2.5 结论原文:
//!   "**默认 8×** —— 逐通道计量（大工程里每个声部都挂表）时成本敏感 …… **母带母线 /
//!   导出天花板用 16×** —— "只付一次"的场景, 把最坏欠读压到 −0.064 dB, 这对 `-1 dBTP`
//!   这类硬天花板是有意义的余量"; 同一份台账的 N1 与 N6 把下一步的归属分别写成
//!   "后续母带/计量线" 与 "母带线"。
//!   `HD-26`（`docs/ledger/human-decisions.md`, 现位于第 57 行）的落地记录同口径: "真峰值 4× → **8×**
//!   （母带 16× 可选）"。
//! - 因此本模块的母带测量（[`measure_master`]）与导出天花板（[`ExportPreset::apply`] /
//!   [`export_master`]）走 **16×**（[`TruePeakOversampling::Sixteen`]）, 而不是
//!   `yeban_dsp::meter::TruePeakDetector::new()` 的 8× 默认值。8× 那一档由
//!   [`measure_master_at`] / [`ExportPreset::apply_at`] 显式给出（逐通道计量口径）。
//! - **本模块不声称**真峰值上限是"不会削顶"的保证: 16× 在 `6/13·fs` 附近仍有 −0.0636 dB
//!   的固有欠读（台账 §2.5 的频点表）。
//! - **dB 换算一律走 `libm`**（[ARCH-DET-001] 的"不使用平台 libm"纪律）: 本模块用
//!   `libm::log10f` / `libm::pow`, **不用** `yeban_dsp::meter::dbfs`（它走 std 的 `f32::log10`）。
//!   两者在**最后一位**上可能不同。理由: 母带读数是可复算的交付物, 不该随宿主 libm 漂移。
//!
//! ## 导出落点：把测量写进容器（[ARCH-FMT-001] 的那一条）
//!
//! [`export_master`] 把链子接起来：渲染产物 [`crate::render::RenderOutput`] →
//! [`ExportPreset::apply`]（增益）→ [`crate::dither::quantize`]（TPDF 抖动/降位深）→
//! [`crate::rf64::write_container`]（RF64 / BW64 / RIFF），并把**实测**响度写进
//! `bext` v2 的 EBU R128 块。
//!
//! 依据是原文：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:461`（`ARCH-FMT-001`）
//! 要求"完整内嵌广播级 `bext` (Broadcast Extension) 元数据块（录制起始时间码、
//! **响度元数据 EBU R128**、工程 ULID 全局唯一标识）"。
//!
//! **这条落点此前不存在**，两处独立证据：
//!
//! - `crates/yeban-render/src/rf64.rs:414`（`Bext::for_project` 的注释）：
//!   "导出时尚未测量响度: 按哨兵 `UNKNOWN` 写入 …… **母带链测出真值后应改写这些字段**";
//! - `crates/yeban-mcp/src/domain/render.rs:1280` 是仓库里唯一的**非测试**构造点,
//!   它构造完 `Bext` 后只改写 `coding_history`, **没有**改写 `loudness`。
//!
//! 因此 [`MasterLoudness::to_bext_loudness`] 在 [`export_master`] 出现之前**没有调用方**
//! （判据 `the_bext_bridge_scales_finite_readings_and_sentinels_the_rest` 只在测试内部调它）。
//!
//! ## 边界（这一层没有证明什么）
//!
//! - **单声道口径**: 立体声接口把左右两路都算进能量（`G = 1`）。本模块**不**做"同信号喂两路"的
//!   单声道近似, 也不支持 >2 声道（与 `yeban-dsp` 的 N5 同一限制）。
//! - **不实现限制器**: 真峰值上限的做法是**减小增益**（让步响度目标）, 不是"加增益后削顶"。
//!   削顶是限幅器的职责（`ROAD-M2` 的 `yeban-dsp` 侧）, 本模块不代它做决定。
//! - **不做流式**: 全部接口都是一次性离线计算（本 crate 的承重契约就是"完全离线"）。
//! - **有限但荒谬的目标不做白名单**: 目标/上限只要求**有限**（非有限的按"未提供"处理,
//!   见 [`ExportPreset::apply`]）; 一个有限却远离节目响度的目标（例如 +100 LUFS）仍会
//!   按请求施加增益, 于是样本可能被量化器钳到端点轨。本模块**不发明**一个"合法 LUFS
//!   区间"—— 那是工具面的输入校验（`yeban-mcp` 对 `targetLufs` 就登记了 `[-70, 0]`）。
//!   真正把"静默交出坏文件"堵死的是: 增益把样本推成非有限时 [`export_master`] **拒绝**
//!   （判据 `the_export_refuses_a_gain_that_overflows_the_master`）。
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

use crate::dither::{BitDepth, DitherRng, quantize};
use crate::loudness::{ABSOLUTE_GATE_LUFS, GatedLoudness, LUFS_OFFSET_DB};
use crate::render::{RenderOutput, db_to_linear};
use crate::rf64::{
    Bext, ContainerKind, ContainerPlan, Loudness, PcmFormat, Rf64Error, write_container,
};
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

/// 真峰值检测器的**过采样倍数**（`yeban_dsp::meter::TruePeakDetector` 的相位数）。
///
/// 这是一个**封闭**的两档枚举, 而不是一个裸 `usize`: `yeban-dsp` 只接受 8 与 16
/// （`TruePeakDetector::with_oversampling` 对其余值返回 `None`, **不静默回落**）,
/// 用枚举把"不可能的倍数"表达不出来, 于是本模块不需要一条运行期校验分支。
///
/// 两档各有归属（上游台账 `docs/ledger/dsp-loudness-notes.md` §2.5 的结论）:
/// 逐通道计量用 8×（每个声部都挂表时成本敏感）; 母带母线 / 导出天花板用 16×。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TruePeakOversampling {
    /// 8× —— `yeban_dsp::meter::TruePeakDetector::new()` 的默认口径（逐通道计量）。
    Eight,
    /// 16× —— 母带母线 / 导出天花板口径。
    Sixteen,
}

impl TruePeakOversampling {
    /// 相位数（`L`）。就是传给 `TruePeakDetector::with_oversampling` 的那个数。
    #[must_use]
    pub const fn phases(self) -> usize {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}

/// 本模块（母带母线 / 导出天花板）使用的真峰值过采样倍数。
///
/// 取值 [`TruePeakOversampling::Sixteen`], 依据是上游台账
/// `docs/ledger/dsp-loudness-notes.md` 的 §2.5 结论原文: "默认 8× …… **母带母线 /
/// 导出天花板用 16×**"。`HD-26`（`docs/ledger/human-decisions.md`, 现位于第 57 行）的落地记录
/// 同口径（"真峰值 4× → 8×（母带 16× 可选）"）。
///
/// **为什么不是 `TruePeakDetector::new()`**: 那个构造器给的是 8×。母带这一侧必须
/// **显式**选 16×, 否则 `bext` 的 `MaxTruePeakLevel` 与 `-1 dBTP` 这类硬天花板都按
/// 8× 的欠读口径测量。实测差（本机, 精确 ±1.0 的周期 9 方波, 见判据
/// `the_mastering_true_peak_uses_the_sixteen_times_oversampling`）:
/// 8× 读到 `0x3FB2A846`, 16× 读到 `0x3FB33680`, 收紧了 `0.026969 dB`。
pub const MASTERING_TRUE_PEAK_OVERSAMPLING: TruePeakOversampling = TruePeakOversampling::Sixteen;

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
    /// 真峰值（dBTP）。幅度为 0 ⇒ `NEG_INFINITY`。
    ///
    /// **读数自带口径**: 它是哪个过采样倍数测出来的, 由 [`Self::oversampling`] 写明。
    /// 8× 与 16× 在 `4/9·fs` 一类相称频率上**不是同一个数**（16× 从不更差, 见
    /// `yeban-dsp` 的 `sixteen_times_contains_eight_times_phases_bit_for_bit`）,
    /// 因此"这个 dBTP 是 8× 还是 16× 测的"是读数的一部分, 不是可以靠上下文推断的东西。
    pub true_peak_dbtp: f32,
    /// [`Self::true_peak_dbtp`] 的口径（过采样倍数）。
    pub oversampling: TruePeakOversampling,
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
    /// 真峰值按 [`MASTERING_TRUE_PEAK_OVERSAMPLING`]（**16×**）测量 —— 这是"导出天花板"
    /// 的口径, 见模块文档 §"真峰值的口径"。要指定倍数（例如逐通道计量的 8×）用
    /// [`Self::apply_at`]。
    ///
    /// 增益为 `0.0` 时**不碰任何样本**（逐位不变）—— 判据
    /// `a_zero_gain_preset_leaves_every_sample_bit_identical` 钉住这一点。
    /// 同一预设**重复施加是不动点**（第二次起逐位不改样本）—— 见
    /// [`Self::apply_at`] 的 §"重复施加同一个预设"。
    /// 两声道按**较短者**测量并施加（与 [`measure_master`] 同一约定）。
    ///
    /// # 非有限的目标与上限**不是约束**
    ///
    /// `Some(f32::INFINITY)` / `Some(f32::NAN)` 这样的目标或上限按"未提供"处理,
    /// 因为它们算不出一个可施加的倍数: [`crate::render::db_to_linear`] 对非有限输入
    /// 返回 `1.0`。若不这样挡, 返回的 [`NormalizeOutcome`] 会**自相矛盾** ——
    /// 实测（见判据 `tests::a_non_finite_target_or_ceiling_is_not_a_constraint`）:
    /// `Some(f32::INFINITY)` 的目标给出 `gain_db = inf` 与
    /// `bound = LoudnessTarget`, 而样本一位都没动。按"未提供"处理后,
    /// `gain_db` 恒为有限数, 因此 [`NormalizeOutcome`] 永远描述一件真事。
    #[must_use]
    pub fn apply(
        &self,
        sample_rate: u32,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Option<NormalizeOutcome> {
        self.apply_at(sample_rate, left, right, MASTERING_TRUE_PEAK_OVERSAMPLING)
    }

    /// 与 [`Self::apply`] 相同, 但**显式指定**真峰值过采样倍数。
    ///
    /// 两次测量（增益之前与之后）用**同一个**倍数, 否则 [`NormalizeOutcome::before`] 与
    /// [`NormalizeOutcome::after`] 会把两个口径的数并排放在一起（见
    /// [`MasterLoudness::oversampling`]）。倍数决定的是**天花板怎么算**:
    /// [`GainBound::TruePeakCeiling`] 的 `allowed_db = ceiling − before.true_peak_dbtp`,
    /// 因此 `before` 按 8× 测、而导出文件按 16× 读时, 天花板在 16× 口径下会差 0.027 dB
    /// （实测, 见判据 `both_normalize_readings_share_one_oversampling`）。
    ///
    /// # Errors
    ///
    /// 与 [`Self::apply`] 相同（采样率不受支持 ⇒ `None`）。
    ///
    /// # 重复施加同一个预设
    ///
    /// 本函数是一个"**测一遍 → 算增益 → 施加 → 再测一遍**"的伺服, 而两次测量都落在
    /// `f32` 上。因此第一次施加之后, 目标与实测值之间**仍然**可能差一个读数量化单位 ——
    /// 本机实测（2 s 的 0.1 幅度 997 Hz 正弦 + [`ExportPreset::streaming`]）:
    /// 第一次的 `gain_db = 5.9999523`, 母带落在 `−13.9999990463257 LUFS`,
    /// 第二次测出来的残差是 `−9.5367431640625e-7 dB` —— 恰好是那个读数处的**一个 ULP**,
    /// 且非零。
    ///
    /// 若不处理这个残差, 它会被当成一个真的要施加的增益: 实测它改掉
    /// **191,998 / 192,000** 个样本的位型（每个最多差 2 个最低有效位）。后果不是"听得出",
    /// 而是"同一份母带第二次导出得到**不同的文件字节**" —— 判据
    /// `the_same_master_exported_twice_is_byte_identical` 钉住这一条。
    ///
    /// 因此本函数把**落在测量分辨率里**（≤ 1 ULP, 见下面的
    /// `within_measurement_resolution`）的残差当成 `0.0`: 不施加, 样本逐位不动。
    /// [`NormalizeOutcome::gain_db`] 于是报 `0.0`, 而 [`NormalizeOutcome::bound`]
    /// 仍是那个**已经满足**的约束 —— 与"目标恰好等于实测值"的既有行为一致
    /// （判据 `a_zero_gain_preset_leaves_every_sample_bit_identical`）。
    /// 这一条**不削弱**任何真实增益: 判据
    /// `applying_the_same_preset_twice_is_a_bit_exact_no_op` 同时自证"那个残差真的非零,
    /// 而且真的会改掉样本"。
    #[must_use]
    pub fn apply_at(
        &self,
        sample_rate: u32,
        left: &mut [f32],
        right: &mut [f32],
        oversampling: TruePeakOversampling,
    ) -> Option<NormalizeOutcome> {
        let before = measure_master_at(sample_rate, left, right, oversampling)?;
        let mut gain_db = 0.0f32;
        let mut bound = GainBound::NothingToDo;
        // 定下 `gain_db` 的那条约束的**两个操作数**（请求值, 实测值）。它们的量级决定
        // 这次测量的分辨率, 也就是"多小的残差已经不可区分"（见上面的
        // §"重复施加同一个预设"）。`NothingToDo` 时没有操作数, 也就没有残差要处理。
        let mut operands: Option<(f32, f32)> = None;

        if let Some(target) = self.target_lufs
            && target.is_finite()
            && before.integrated_is_measurable()
        {
            gain_db = target - before.integrated_lufs;
            bound = GainBound::LoudnessTarget;
            operands = Some((target, before.integrated_lufs));
        }

        if let Some(ceiling) = self.true_peak_ceiling_dbtp
            && ceiling.is_finite()
            // ⚠ **上游契约的名字**（不是本守卫自己的防线）:
            // `yeban_dsp::meter::sanitize_sample` 把 `±inf` 钳到 `±MAX_LINEAR_MAGNITUDE`、
            // 把 `NaN` 钳到 `0.0`, 因此 `TruePeakDetector::true_peak()` 的读数**恒有限**
            // （本机实测: ±inf 输入 ⇒ 有限 25.20702 dBTP）。该契约由 `yeban-dsp` 的
            // `meter::tests::the_true_peak_reading_is_always_finite` 守住
            // （由 `mod-render` 的交接说明 `/tmp/mod-render/handoff-mod-dsp.md` 转派, 见裁决 R50）。
            // ⇒ 本行目前是**文档**而非防线: 可达的非有限取值只有 `−inf`, 那一侧
            // `allowed_db = +inf`, 而 `+inf < gain_db` 对任何 `gain_db` 都是假。
            // **若上游那条判据变红或被删除, 本行就变成真防线** —— 请同时复核
            // `within_measurement_resolution` 的文档（它依赖同一条上游契约）。
            && before.true_peak_dbtp.is_finite()
        {
            // 上限允许多少增益: 真峰值加上它就正好碰到上限。
            let allowed_db = ceiling - before.true_peak_dbtp;
            if allowed_db < gain_db {
                gain_db = allowed_db;
                bound = GainBound::TruePeakCeiling;
                operands = Some((ceiling, before.true_peak_dbtp));
            }
        }

        // 已在目标上 ⇒ **不乘**。这一句必须落在下面那次乘法**之前**: 它让
        // "同一个预设施加两次"与"施加一次"逐位相同（见上面的
        // §"重复施加同一个预设"）。
        if let Some((demand, actual)) = operands
            && within_measurement_resolution(demand, actual, gain_db)
        {
            gain_db = 0.0;
        }

        if gain_db != 0.0 {
            let linear = db_to_linear(gain_db);
            let frames = left.len().min(right.len());
            for sample in left[..frames].iter_mut().chain(right[..frames].iter_mut()) {
                *sample *= linear;
            }
        }

        let after = measure_master_at(sample_rate, left, right, oversampling)?;
        Some(NormalizeOutcome {
            gain_db,
            bound,
            before,
            after,
        })
    }
}

/// 残差是否**小到不能与测量本身的舍入区分**: `≤ 1 ULP`（按两个操作数里量级较大的那个取）。
///
/// # 为什么要有这条判据
///
/// [`ExportPreset::apply_at`] 的增益是**两个 `f32` 读数之差**（目标 − 实测）。两个读数
/// 各自带着自己的舍入误差, 量级 `M` 处的 `f32` 间距是 1 ULP。于是"差一个 ULP 的残差"
/// 这件事本身**分不出来**是"真的还差一点"还是"两次舍入的差" —— 它落在测量精度之下。
///
/// 而施加它不是无害的: `*sample *= db_to_linear(残差)` 里的因子不等于 `1.0`
/// （实测 `db_to_linear(-9.5367431640625e-7) = 0.99999994`）, 于是每个样本的位型都可能
/// 被改掉。**不施加**才是"已经在目标上"的正确表达。
///
/// # 为什么取**较大**的那个操作数
///
/// `f32` 的间距随量级增长, 读数的舍入误差也随量级增长, 因此两个操作数里较大的那个
/// 给出较大的分辨率 —— 这是保守的一侧（阈值更大, 更不容易把"真的还差一点"当成已达标）。
/// 量级恰为 2 的幂时 [`f32::next_up`] 给出的是**上方**那一档间距（比下方大一倍）,
/// 于是该点上的阈值偏宽一倍: 那是亚微分贝量级, 不影响任何可听的判断。
///
/// # 它**不**吞掉真实增益
///
/// 阈值只覆盖"读数最后一位"这一档。任何真实的归一化增益（例如把 −19.999952 LUFS 抬到
/// −14 LUFS 的 `+5.9999523 dB`）比它大 6 个数量级。判据
/// `applying_the_same_preset_twice_is_a_bit_exact_no_op` 里那条"残差真的非零、且真的会
/// 改掉样本"的自证, 就是这条边界的守门人。
///
/// 两个操作数都已被调用方确认有限（目标/上限 `is_finite`, 实测读数有限）, 因此
/// `next_up` 不会碰到 `NaN`; 它仍然可以是 `+inf`（见下一节）。
///
/// # `f32::MAX` 那一格：分辨率会**溢出成 `+inf`**（本轮修掉的缺口）
///
/// `magnitude.next_up()` 在 `magnitude == f32::MAX` 时是 `+inf`, 于是
/// `next_up() - magnitude` 也是 `+inf` —— 而 `residual <= +inf` **恒真**,
/// 任何残差（包括 3.4e38 dB）都会被判成"落在测量分辨率里"。实测:
/// 2 s −19.999952 LUFS 的母带 + 目标 `Some(f32::MAX)` LUFS ⇒ 修复前
/// `NormalizeOutcome { gain_db: 0.0, bound: LoudnessTarget }` 而**样本一位未动**
/// —— 报告说"已经在目标上", 而目标差 3.4e38 LUFS。这与
/// [`ExportPreset::apply`] 文档里"`NormalizeOutcome` 永远描述一件真事"是同一条契约,
/// 只是入口换成了一个**有限**数（`Some(f32::INFINITY)` 那一档早就有判据了）。
///
/// `+inf` 不是一个间距, 因此这一格必须**不**吸收残差。判据是
/// `tests::a_finite_target_at_the_top_of_the_range_is_not_already_met`。
fn within_measurement_resolution(demand: f32, actual: f32, residual: f32) -> bool {
    let magnitude = demand.abs().max(actual.abs());
    let resolution = magnitude.next_up() - magnitude;
    resolution.is_finite() && residual.abs() <= resolution
}

/// 母带导出被拒绝的原因。
///
/// 每一种拒绝都是**有意的**：静默降级会把一个自相矛盾或元数据缺失的文件交给用户。
///
/// 本枚举**不实现 `Eq`**：[`Self::NonFiniteSamples`] 携带 `f32`, 而 `NaN` 让比较退化成
/// 偏序。`PartialEq` 仍然可用（判据用 `assert_eq!` 比错误值）。
#[derive(Clone, Debug, PartialEq)]
pub enum MasterExportError {
    /// 采样率不在内置四档（44.1 / 48 / 88.2 / 96 kHz）里 ⇒ 响度读数会错 ⇒ 拒绝。
    UnsupportedSampleRate(u32),
    /// 声道数不是 2（响度计量与常设预设都是立体声口径）⇒ 拒绝, **不**做下混。
    NotStereo(usize),
    /// 交错缓冲的长度不是偶数 ⇒ 最后一个样本落单 ⇒ 拒绝, **不**静默丢弃它。
    RaggedInterleavedBuffer(usize),
    /// `bext` 模板的版本不是 1 或 2 ⇒ 既不写, 也不猜。
    ///
    /// # 为什么需要**上界**这一半（实测缺口）
    ///
    /// [`crate::rf64::Bext::from_bytes`] 只接受版本 1 与 2。修复前
    /// [`export_master`] 只挡住"版本 < 2", 于是 `version = 3` 的模板会被
    /// `crate::rf64::Bext::to_bytes` 照原样写进文件, 而同一份字节交给本 crate 的
    /// 读取器得到 [`Rf64Error::UnsupportedBextVersion`]`(3)` —— 调用方拿到成功,
    /// 交付物却是一个**自己读不回来**的容器。判据是
    /// `the_export_refuses_a_bext_version_the_reader_cannot_read`。
    UnsupportedBextVersion(u16),
    /// `bext` 模板的某个文本字段取值在字段编码里**无法表示**（含 NUL）⇒ 既不写, 也不截断。
    ///
    /// # 为什么需要这一条（与版本上界同族的实测缺口）
    ///
    /// BWF 的文本字段是 NUL 终止的定长/变长字节串, 因此 [`crate::rf64::Bext::from_bytes`]
    /// 在第一个 NUL 处停止读取。修复前 [`export_master`] 会把含 NUL 的字段照写进文件,
    /// 而同一份字节读回来的字符串**比调用方给的那个短**（本机实测:
    /// `Description = "a\0b"` 读回 `"a"`, `OriginationTime = "13\0:37:00"` 读回 `"13"`）
    /// —— 调用方拿到成功, 交付物的元数据却与请求的不是同一个值。
    /// 判定与容器层共用 [`crate::rf64::Bext::field_that_does_not_round_trip`]。
    ///
    /// 检查点在**任何改写之前**: 失败时调用方的 [`RenderOutput`] 与它的位级摘要一位未动
    /// （与 [`Self::NonFiniteSamples`] 同一条纪律）。判据是
    /// `the_export_refuses_a_bext_field_the_reader_cannot_read_back`。
    UnrepresentableBextField {
        /// 出问题的字段名（BWF 的字段名, 如 `Description`）。
        field: &'static str,
    },
    /// `bext` 模板的版本小于 2 ⇒ 它没有 EBU R128 响度字段 [ARCH-FMT-001]。
    ///
    /// 拒绝而不是把版本改成 2：版本是调用方给的元数据语义, 本函数不改写它;
    /// 而 [`crate::rf64::Bext::to_bytes`] 对"版本 1 却有响度"会 panic,
    /// 对"版本 2 但无响度"也会 panic —— 两条路都只能靠拒绝避开。
    BextCannotCarryLoudness(u16),
    /// 母带缓冲里有 `NaN` / `±inf` 样本 ⇒ 拒绝, **不**静默换成一个 0 样本。
    ///
    /// `index` 是**交错缓冲里第一个**非有限样本的下标, `value` 是那个样本本身。
    ///
    /// 两个来源共用本变体: 调用方交进来的缓冲（在增益之前查）, 以及按预设施加增益
    /// **之后**变成非有限的缓冲（在回写之前查）。后者由 [`ExportPreset::apply`]
    /// 施加的一个**有限但溢出**的增益引起 —— 见 [`export_master`] 的 `# Errors`。
    ///
    /// # 为什么拒绝而不是让抖动层替换
    ///
    /// [`crate::dither::quantize`] 的契约是"别把 `NaN` 写进容器", 它把非有限样本换成
    /// `0.0`（32f 路径）或钳成一个整数端点（16/24 位路径）。若导出就此放行, 出来的文件
    /// 是一段**静音或端点直流**, 而 `bext` 的 EBU R128 块仍写着"实测"的响度与真峰值 ——
    /// 元数据与音频互相矛盾, 且调用方拿不到任何信号。同一条纪律的既有落点是
    /// [`crate::wav::check_match`]: 越界样本在那里也是**拒绝**（`RejectedFormat`）,
    /// 不是替换。
    NonFiniteSamples {
        /// 交错缓冲里第一个非有限样本的下标（样本计数, 不是帧计数）。
        index: usize,
        /// 那个非有限样本本身（`NaN` / `+inf` / `-inf`）。
        value: f32,
    },
    /// 容器写入失败（透传 [`Rf64Error`]）。
    Container(Rf64Error),
}

impl core::fmt::Display for MasterExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedSampleRate(rate) => {
                write!(
                    f,
                    "响度计量不支持 {rate} Hz（内置档位: 44.1/48/88.2/96 kHz）"
                )
            }
            Self::NotStereo(channels) => {
                write!(f, "母带导出只支持立体声, 实际 {channels} 声道")
            }
            Self::RaggedInterleavedBuffer(len) => {
                write!(f, "交错缓冲长度 {len} 不是偶数（立体声的帧必须成对）")
            }
            Self::UnsupportedBextVersion(version) => write!(
                f,
                "bext 版本 {version} 的字段表未核验（读取器只接受 1 与 2）; \
                 写出去就会产出一个本 crate 读不回来的容器"
            ),
            Self::UnrepresentableBextField { field } => write!(
                f,
                "bext 的 {field} 字段取值含 NUL: 读取器在 NUL 处停止（或裁掉尾随 NUL）, \
                 写出去的元数据与请求的不是同一个值"
            ),
            Self::BextCannotCarryLoudness(version) => write!(
                f,
                "bext 版本 {version} 没有 EBU R128 响度字段, 无法承载实测响度 [ARCH-FMT-001]"
            ),
            Self::NonFiniteSamples { index, value } => write!(
                f,
                "母带缓冲的第 {index} 个样本是 {value}（非有限）; 导出会静默丢弃它, 因此拒绝"
            ),
            Self::Container(error) => write!(f, "容器写入失败: {error}"),
        }
    }
}

impl std::error::Error for MasterExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Container(error) => Some(error),
            _ => None,
        }
    }
}

/// 一次母带导出的产物。
///
/// 全部字段都是**构造期**的产物（本 crate 的契约是"完全离线", 见 crate 模块头）:
/// 没有一项会在实时音频线程上被求值。
#[derive(Clone, Debug, PartialEq)]
pub struct MasterExport {
    /// 整个容器文件的字节（头部 + `data` 负载 + 补位字节）。
    pub file: Vec<u8>,
    /// `data` chunk 的负载（抖动/量化之后的 PCM 字节）。
    pub payload: Vec<u8>,
    /// 写进容器的 `bext` 块。它的 `loudness` 是**实测**值, 不是模板里的哨兵。
    pub bext: Bext,
    /// 预设的结果（实际增益、谁定下它、增益前后两次测量）。
    pub outcome: NormalizeOutcome,
    /// 导出的位深。
    pub depth: BitDepth,
    /// 导出的帧数（= 交错缓冲长度 / 2）。
    pub frames: u64,
    /// **文件字节**的 SHA-256（判"同种子 ⇒ 逐字节相同"的载体; 不是样本的摘要）。
    pub digest: [u8; 32],
}

/// 扫出交错缓冲里**第一个** `NaN` / `±inf` 样本; 全部有限 ⇒ `Ok(())`。
///
/// 只读一遍缓冲, 零分配。它存在的理由是 [`crate::dither::quantize`] 的既有契约:
/// 非有限样本在量化时被**替换**成 `0.0`（32f）或一个整数端点（16/24 位）。替换本身
/// 是对的（容器里不许出现 `NaN` 位型）, 但导出不能因此**静默**产出一段静音:
/// `bext` 会同时写着"实测"的响度与真峰值, 元数据与音频于是互相矛盾。
///
/// 返回**第一个**命中的下标而不是"有/没有"两态: 调用方要能定位是哪一帧坏的。
fn require_finite_samples(samples: &[f32]) -> Result<(), MasterExportError> {
    match samples
        .iter()
        .copied()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        Some((index, value)) => Err(MasterExportError::NonFiniteSamples { index, value }),
        None => Ok(()),
    }
}

/// 把一次渲染产物落成带**实测**响度元数据的广播级容器文件 [ARCH-FMT-001]。
///
/// 这一步把三条既有能力接成一条链, 顺序如下（每一步的顺序都有理由）:
///
/// 1. **校验**: 立体声、交错长度成对、`bext` 版本 ∈ `{1, 2}` 且 ≥ 2、全部样本有限
///    （见 [`MasterExportError`]。非有限样本在**施加增益之前**就被拒绝。
///    版本的上界与下界都查: 只有下界时 `version = 3` 会被写成一个
///    [`crate::rf64::Bext::from_bytes`] 读不回来的容器）;
/// 2. **[`ExportPreset::apply`]**: 按目标响度与真峰值上限**就地**施加增益
///    （上限赢时减小增益, 不削顶）; 它只施加由**有限**读数算出的增益, 因此有限输入
///    不会在这里变成非有限值（论证见函数体内那段注释）;
/// 3. **测量**: 用第 2 步返回的 `after`（在**抖动之前**的浮点母带上测的），
///    经 [`MasterLoudness::to_bext_loudness`] 写进 `bext` v2 的 EBU R128 块;
///    `after.true_peak_dbtp` 及其写进 `bext` 的 `MaxTruePeakLevel` 都是
///    [`MASTERING_TRUE_PEAK_OVERSAMPLING`]（**16×**）口径 —— 这是"导出天花板"的口径,
///    见模块文档 §"真峰值的口径";
/// 4. **抖动/量化**: [`crate::dither::quantize`]（16/24 位走 TPDF, 32f 透传）;
/// 5. **容器**: [`ContainerPlan::for_payload`] + [`write_container`]。
///
/// # 就地改写
///
/// 增益施加在 `master.samples` 上, 并且**重算** `master.digest`
/// （[`RenderOutput::digest_of`]）—— 否则缓冲与它的位级摘要会不一致, 而那份摘要是
/// L1 判据的载体。调用方因此既拿到文件, 也拿到归一化后的母带。
///
/// # 重复导出是幂等的（同一份母带 + 同一个预设 + 同一种子 ⇒ 同一份字节）
///
/// 在**同一个** `master` 上连续调用本函数两次（预设、位深、容器、种子都不变）⇒
/// 两次的 `file` / `payload` / `bext` / `digest` **逐字节相同**
/// （判据 `the_same_master_exported_twice_is_byte_identical`）。第二次的
/// `outcome.gain_db` 是 `0.0`: 第一次施加之后的残差落在**读数量化单位**以内
/// （本机实测恰好 1 ULP）, [`ExportPreset::apply`] 因此把它当成已达标
/// （见它的 §"重复施加同一个预设"）。
///
/// # 抖动源必须**从种子新建**, 不能跨两次导出复用同一个实例
///
/// 这一条是**允许的**非确定性, 但调用方必须照做: `rng` 是 `&mut`, 16/24 位路径上
/// 每个样本从中取两次抽样 —— 复用同一个实例意味着第二次导出的抖动序列与第一次不同,
/// 于是同一份母带、同一份参数会得到不同的字节。生产路径因此一律
/// [`crate::rng::dither_rng_for`]`(seed, node)` 现造一个（`lib.rs` 的端到端契约判据与
/// `mastering` 的判据都这么做）。32f 路径不取抽样, 因此不受影响
/// （`crate::rng` 的 `float_path_does_not_consume_the_rng` 钉住这一点）。
///
/// # 构造期与逐样本
///
/// 本函数**全部在构造期**：反交错、交错、`bext`、头部、文件缓冲各分配一次
/// （离线路径允许分配）。逐样本的循环只有两处 —— [`crate::dither::quantize`] 内部
/// 的量化, 以及本函数里 32f 用的交错拷贝。本函数**不在**音频线程上被调用。
///
/// # Errors
///
/// 见 [`MasterExportError`]。`sample_rate` 不受支持时**不静默回落**到 48 kHz;
/// 母带里有 `NaN` / `±inf` 样本时**不静默替换成 0** —— 这条纪律对**两处**都成立:
/// 调用方交进来的缓冲（增益之前）, 以及按预设施加增益**之后**的缓冲。
/// 后一种由目标本身是有限数、但算出的增益把样本推出 `f32` 有限域引起（判据
/// `the_export_refuses_a_gain_that_overflows_the_master`）。两种情况都**在回写
/// `master` 之前**拒绝, 因此失败时调用方的缓冲与摘要一位未动。
pub fn export_master(
    sample_rate: u32,
    master: &mut RenderOutput,
    preset: ExportPreset,
    depth: BitDepth,
    preferred: ContainerKind,
    metadata: &Bext,
    rng: &mut impl DitherRng,
) -> Result<MasterExport, MasterExportError> {
    if master.channels != 2 {
        return Err(MasterExportError::NotStereo(master.channels));
    }
    if !master.samples.len().is_multiple_of(2) {
        return Err(MasterExportError::RaggedInterleavedBuffer(
            master.samples.len(),
        ));
    }
    if !matches!(metadata.version, 1 | 2) {
        return Err(MasterExportError::UnsupportedBextVersion(metadata.version));
    }
    if metadata.version < 2 {
        return Err(MasterExportError::BextCannotCarryLoudness(metadata.version));
    }
    // 文本字段的取值必须能被读取器**读回同一个字符串**。与上面两条版本检查同一个位置:
    // 都在**任何改写之前**, 因此拒绝时调用方的缓冲与摘要一位未动。
    if let Some(field) = metadata.field_that_does_not_round_trip() {
        return Err(MasterExportError::UnrepresentableBextField { field });
    }
    // 非有限样本在**任何**改写之前就拒绝: 这样 `master` 保持调用方交进来的原样
    // （不触发"先施加增益、再报错"的半成品状态）, 且 `NaN` 不会先污染响度读数。
    require_finite_samples(&master.samples)?;

    // 帧数取自**缓冲的实际长度**（不是 `master.frames`）: 写进 `ds64` 的必须是
    // 文件里真有的帧数, 否则容器会声明一段不存在的音频。
    let frames = master.samples.len() / 2;

    // 反交错: `ExportPreset::apply` 的接口是左右两路切片（与本模块其余接口一致）。
    let mut left: Vec<f32> = Vec::with_capacity(frames);
    let mut right: Vec<f32> = Vec::with_capacity(frames);
    for pair in master.samples.as_chunks::<2>().0 {
        left.push(pair[0]);
        right.push(pair[1]);
    }

    let outcome = preset
        .apply(sample_rate, &mut left, &mut right)
        .ok_or(MasterExportError::UnsupportedSampleRate(sample_rate))?;

    // 增益**之后**再查一遍有限性, 而且查在**回写 `master` 之前**。
    //
    // 旧实现只有上面那一遍（施加增益之前的调用方缓冲）, 理由是"有限的 f32 样本乘上
    // 有限的线性增益不会产生 NaN"。那条论证对**调用方给的目标**不成立: 目标本身有限,
    // 不代表由它算出的增益不会把样本推出 f32 的有限域。实测（见判据
    // `the_export_refuses_a_gain_that_overflows_the_master`）: 2 秒 −20.00 LUFS 的母带
    // + 目标 `1e38` LUFS ⇒ 增益 `1e38` dB ⇒ 96,000 个样本全部变成 `+inf`,
    // 而旧实现照旧返回 `Ok` —— 容器写成功, 里面的样本却是量化器钳出来的轨,
    // `bext` 里更是一份由钳位值算出的读数。
    //
    // 放在回写之前: 失败时调用方的 `RenderOutput` 与它的位级摘要**一位都没动**
    // （与上面那遍"在任何改写之前拒绝"同一条纪律）。
    require_finite_samples(&left)?;
    require_finite_samples(&right)?;

    // 把归一化后的母带回写, 并让位级摘要跟上传缓冲 —— 摘要不许过期。
    for (slot, (left_sample, right_sample)) in master
        .samples
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip(left.iter().zip(right.iter()))
    {
        slot[0] = *left_sample;
        slot[1] = *right_sample;
    }
    // 到这里 `left`/`right` 已被上面那一遍有限性检查确认过, 因此回写是安全的,
    // 位级摘要也跟着母带一起更新（摘要不许过期）。
    master.digest = RenderOutput::digest_of(&master.samples);

    // 实测响度进 `bext` v2 的 EBU R128 块。测不出的量由 `to_bext_loudness` 写哨兵。
    let mut bext = metadata.clone();
    bext.loudness = Some(outcome.after.to_bext_loudness());

    let mut interleaved: Vec<f32> = Vec::with_capacity(frames * 2);
    for (left_sample, right_sample) in left.iter().zip(right.iter()) {
        interleaved.push(*left_sample);
        interleaved.push(*right_sample);
    }
    let payload = quantize(&interleaved, depth, rng).to_le_bytes();

    let format = if depth.is_integer() {
        PcmFormat::integer(2, sample_rate, depth.bits())
    } else {
        PcmFormat::float(2, sample_rate, depth.bits())
    };
    let plan = ContainerPlan::for_payload(
        preferred,
        format,
        payload.len() as u64,
        frames as u64,
        Some(bext.clone()),
    );
    let mut file: Vec<u8> = Vec::with_capacity(plan.header_bytes().len() + payload.len() + 1);
    write_container(&mut file, &plan, &payload).map_err(MasterExportError::Container)?;

    Ok(MasterExport {
        digest: sha256_of(&file),
        file,
        payload,
        bext,
        outcome,
        depth,
        frames: frames as u64,
    })
}

/// 对任意字节串取 SHA-256。
///
/// 与 [`RenderOutput::digest_of`] 的区别是**输入**: 那个对 `f32` 位型取摘要,
/// 这个对文件字节取摘要。同种子导出的两个文件靠它比较。
fn sha256_of(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let output = hasher.finalize();
    let mut digest = [0u8; 32];
    for (slot, byte) in digest.iter_mut().zip(output.iter()) {
        *slot = *byte;
    }
    digest
}

/// 测一段母带的全部读数; 采样率不受支持 ⇒ `None`。
///
/// 信号被遍历三遍（短时窗口扫描、门限积分两遍）。这是**离线**接口, 代价可接受;
/// 实时路径不许调用它（本 crate 的契约本来就是"完全离线", 见 crate 的模块头文档）。
///
/// 真峰值按 [`MASTERING_TRUE_PEAK_OVERSAMPLING`]（**16×**）测量 —— 母带这一侧的口径,
/// 依据见模块文档 §"真峰值的口径"。要指定倍数（例如逐通道计量的 8×）用
/// [`measure_master_at`]。
///
/// # 声道长度不等时按**较短者**测量
///
/// 三处读数全部只覆盖 `min(left.len(), right.len())` 帧:
///
/// - 短时窗口（[`scan_short_term`] 的 `frames`）;
/// - 门限积分（`yeban_dsp::loudness` 的 `GatedLoudness` 自己就按 `min` 取）;
/// - **真峰值**（本函数里那两个检测器）。
///
/// 第三条是本函数此前的缺口（实测见判据
/// `tests::the_true_peak_of_a_ragged_pair_comes_from_the_shorter_channel`）:
/// 旧实现把两路**整条**切片喂给各自的检测器, 于是长边多出来的尾巴也进了真峰值。
/// 这不只是"读数偏大" —— [`ExportPreset::apply`] 用这个读数算增益, 而增益只施加在
/// `min` 帧上, 因此 [`GainBound::TruePeakCeiling`] 会声称"上限赢了", 而尾巴一位都没改
/// ⇒ 输出仍然超过上限（判据 `tests::a_true_peak_ceiling_is_really_achieved_on_ragged_channels`）。
#[must_use]
pub fn measure_master(sample_rate: u32, left: &[f32], right: &[f32]) -> Option<MasterLoudness> {
    measure_master_at(sample_rate, left, right, MASTERING_TRUE_PEAK_OVERSAMPLING)
}

/// 与 [`measure_master`] 相同, 但**显式指定**真峰值过采样倍数。
///
/// 两次调用（同一个信号、两个倍数）除 [`MasterLoudness::true_peak_dbtp`] 与
/// [`MasterLoudness::oversampling`] 之外**每一项都相同**: 响度三件套（积分、瞬时、短时）
/// 与真峰值检测器无关, 本函数只把倍数喂给两个检测器。
///
/// # 为什么倍数必须显式给出
///
/// `yeban_dsp::meter::TruePeakDetector::new()` 给的是 8×, 而母带母线 / 导出天花板按
/// 上游台账 §2.5 的结论要用 16×。若把倍数留给调用方"顺手用默认值", 母带这一侧就会
/// 悄悄退回 8×（实测差 0.026969 dB, 见 [`MASTERING_TRUE_PEAK_OVERSAMPLING`]）——
/// 这正是本函数存在的理由。
///
/// # 读数的口径自带
///
/// [`MasterLoudness::oversampling`] 记录本次用的是哪一档, 因此不必从调用链上下文
/// 推断一个 dBTP 读数的口径。
#[must_use]
pub fn measure_master_at(
    sample_rate: u32,
    left: &[f32],
    right: &[f32],
    oversampling: TruePeakOversampling,
) -> Option<MasterLoudness> {
    let rate = sample_rate as f32;
    let scan = scan_short_term(rate, left, right)?;
    let integrated_lufs = GatedLoudness::integrated_stereo_at(rate, left, right)?;

    // `TruePeakOversampling` 是**封闭**的枚举, 两档都在 `yeban-dsp` 的支持集里
    // （判据 `both_oversampling_choices_are_supported_by_the_detector` 钉住这条）。
    // 这里 `expect` 而不是回落: 上游若有一天不再支持某一档, 母带读数**必须是红**,
    // 不能安静地换一个倍数继续算（那正是"口径漂移"）。
    let mut detector_left = TruePeakDetector::with_oversampling(oversampling.phases())
        .expect("TruePeakOversampling 的两档都必须被 yeban-dsp 支持");
    let mut detector_right = TruePeakDetector::with_oversampling(oversampling.phases())
        .expect("TruePeakOversampling 的两档都必须被 yeban-dsp 支持");

    let frames = left.len().min(right.len());
    detector_left.process(&left[..frames]);
    detector_right.process(&right[..frames]);
    let true_peak = detector_left.true_peak().max(detector_right.true_peak());

    Some(MasterLoudness {
        integrated_lufs,
        loudness_range_lu: loudness_range_from_short_term(&scan.short_terms),
        max_momentary_lufs: scan.max_momentary_lufs,
        max_short_term_lufs: scan.max_short_term_lufs,
        true_peak_dbtp: amplitude_to_dbtp(true_peak),
        oversampling,
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
///
/// **尾巴必须也喂进 meter**: 不足一个整秒的剩余帧**不产生** short-term 读数
/// （1 s 步进的契约, 见 [`short_term_values`]）, 但它们仍是信号的一部分 ——
/// `max_momentary_lufs`（400 ms 窗口）与 `max_short_term_lufs`（3 s 窗口）是
/// **整段信号的最大值**, 不喂尾巴就会漏掉尾巴里的窗口。
///
/// 旧实现的缺口（实测, 见判据
/// `tests::the_tail_still_reaches_the_momentary_and_short_term_maxima`）:
/// 循环条件 `start + step <= frames` 之后没有任何尾巴喂入, 于是 48 kHz、
/// 前 4 s 数字静音 + 尾 0.5 s −20 dBFS 的 997 Hz 正弦给出
/// `integrated_lufs = -21.548813`（有限）而两个最大值**都是** `NEG_INFINITY`
/// ⇒ `bext` 的 `LoudnessValue` 写实测值、`MaxMomentaryLoudness` /
/// `MaxShortTermLoudness` 写 `UNKNOWN` 哨兵, 同一份元数据自相矛盾。
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
    // 尾巴: **不推** short-term 读数（1 s 步进的契约不许变）, 但必须进 meter,
    // 否则尾巴里的瞬时/短时窗口到不了两个最大值。
    if start < frames {
        meter.add_stereo(&left[start..frames], &right[start..frames]);
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

    /// 判据: **尾巴里的 400 ms 瞬时值必须进 `max_momentary_lufs`**。
    ///
    /// 缺口原文: [`scan_short_term`] 的函数文档写"用一个 [`GatedLoudness`] 喂**完整段信号**",
    /// 而循环是 `while start + step <= frames` ⇒ 尾巴 `frames % step` 帧**一位都没进 meter**。
    /// 后果不是"少一条 LRA 读数"（"1 s 步进、尾巴不产生读数"那条契约是**对的**,
    /// 本判据同时把它钉住）, 而是两个**最大值**读数被截断:
    ///
    /// 实测夹具 = 48 kHz、前 4 s 数字静音 + 尾 0.5 s −20 dBFS 的 997 Hz 正弦。
    /// 尾巴里有 5 个完整 100 ms 跳 ⇒ 最后一个 400 ms 瞬时窗口**整体**是有声的。
    #[test]
    fn the_tail_still_reaches_the_momentary_and_short_term_maxima() {
        const WHOLE: usize = 48_000 * 4;
        const TAIL: usize = 24_000;
        let mut left = vec![0.0f32; WHOLE + TAIL];
        let mut right = left.clone();
        let tone = sine_997(0.1, TAIL);
        left[WHOLE..].copy_from_slice(&tone);
        right[WHOLE..].copy_from_slice(&tone);

        // 尾巴**不产生** short-term 读数: 4 s 整秒 ⇒ 4 条（这条契约不许被这次修复改变）。
        assert_eq!(
            short_term_values(48_000, &left, &right)
                .expect("48 kHz")
                .len(),
            4,
            "尾巴不得新增 short-term 读数"
        );

        let measured = measure_master(48_000, &left, &right).expect("48 kHz");
        assert!(
            measured.integrated_is_measurable(),
            "尾巴里有 0.5 s 有声信号 ⇒ 积分响度必须测得出, 实际 {}",
            measured.integrated_lufs
        );
        assert!(
            measured.max_momentary_lufs.is_finite(),
            "尾巴最后一个 400 ms 窗口整体有声 ⇒ 瞬时最大值必须有限, 实际 {}",
            measured.max_momentary_lufs
        );
        // **手算**: 短时窗口 = 30 个 100 ms 跳, 其中末尾 **5** 跳有声（0.5 s 尾巴）
        // ⇒ 窗口均方 = 满值的 5/30 ⇒ `−20 + 10·log10(5/30)` = **−27.7815** LUFS。
        // 少喂一份、多喂一份（或整个尾巴不喂）都会让这个数变。实测 **−27.781736**。
        assert!(
            (measured.max_short_term_lufs + 27.7815).abs() < 0.01,
            "手算 −20 + 10·log10(5/30) = −27.7815 LUFS, 实际 {}",
            measured.max_short_term_lufs
        );

        // `bext` 侧因此既不许写"未知"哨兵, 也不许写别的数。旧实现把尾巴丢掉之后
        // 同一份元数据是**自相矛盾**的: `LoudnessValue` 写实测的 −2155,
        // 而 `MaxMomentaryLoudness` / `MaxShortTermLoudness` 都写 `UNKNOWN`
        // （= `i16::MIN` = −32768, 见 `crate::rf64::Loudness`）。
        // 单位是 0.01 LUFS（[`crate::rf64::Loudness`] 的刻度）。
        let bext = measured.to_bext_loudness();
        assert_eq!(
            (
                bext.loudness_value,
                bext.max_momentary_loudness,
                bext.max_short_term_loudness
            ),
            (-2155, -2000, -2778),
            "0.01 刻度的实测字面量: −21.548813 / −19.999035 / −27.781736 LUFS"
        );
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
        assert!((lra - 20.0).abs() < 0.05, "手算 20.0 LU, 实际 {lra}"); // 容差 0.05 LU：powf／log10 跨平台（裁决 R25）
    }

    /// **相对门限判据**: 85 条 −20 LUFS + 15 条 −60 LUFS。
    ///
    /// 手算: `mean_z = (85·0.011725 + 15·1.1725e−4)/100 = 0.00998384`;
    /// `Γr = −0.691 + 10·log10(0.00998384) − 20 = −40.698` ⇒ 15 条 −60 全部被相对门限丢掉,
    /// 只剩 −20 ⇒ LRA = **0.0 LU**。
    ///
    /// **这条判据有鉴别力**: 去掉相对门限后排序是 `15×(−60) + 85×(−20)`, `P10 = −60`、
    /// `P95 = −20` ⇒ LRA = **40.0 LU** ⇒ 变红。`15` 条是**刻意**的: 离群值只有 1 条时
    /// 它到不了 P10（`rank = 0.10·99 = 9.9` 落在 99 条 −20 之内）, 判据对"去掉相对门限"
    /// **不敏感** —— 第一版就是 1 条, 注入证明抓出了这个缺陷。
    #[test]
    fn the_relative_gate_drops_the_quiet_tail() {
        let mut values = vec![-20.0f32; 85];
        values.extend(core::iter::repeat_n(-60.0f32, 15));
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

    /// 判据 (**类别 7: 范围极值 / 边界值**): 绝对门限 `Γa = −70 LUFS` 是**严格**下界
    /// —— 恰好等于 `Γa` 的短时值是"未过门限", 不是"过门限"。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 `loudness_range_from_short_term` 的
    /// `.filter(|value| value.is_finite() && f64::from(*value) > ABSOLUTE_GATE_LUFS)`
    /// 注入成 `>=`, 全量 `cargo test -p yeban-render` **全绿**
    /// （`test result: ok. 170 passed; 0 failed`）—— 既有的绝对门限判据
    /// （`everything_below_the_absolute_gate_has_no_lra`）用的是 `−80` / `−75` / `−71`,
    /// 三个都**严格低于** `−70`, 因此两条写法在那些输入上同判; 而
    /// `−71` 这一格离门限还有 1 LU, 不是边界值。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一组**恰好** `−70.0 LUFS` 的短时值（单位: LUFS）, 与它的**上方相邻** f32
    /// （`−70.0f32.next_up()`, 即严格大于 `Γa` 的最小可表示读数）。读数:
    /// [`loudness_range_from_short_term`] 的 `Option<f32>`（单位: LU）。
    ///
    /// # 非空证明
    ///
    /// 两个输入都非空, 且它们的差是**一个 ULP**（`next_up` 的定义）—— 因此这一对不是
    /// "都用同一个数"的退化情形。上方那一格必须过门限并给出 `0.0`（所有值相等 ⇒
    /// `P95 = P10`）, 恰好等于门限的一格必须被丢掉。
    #[test]
    fn the_absolute_gate_is_a_strict_lower_bound() {
        let at_the_gate = vec![-70.0f32; 8];
        let above_the_gate = vec![(-70.0f32).next_up(); 8];
        assert!(
            f64::from(above_the_gate[0]) > ABSOLUTE_GATE_LUFS,
            "上方相邻的 f32 必须严格大于门限"
        );
        assert_eq!(
            loudness_range_from_short_term(&at_the_gate),
            None,
            "恰好等于 −70 LUFS 的短时值是**未过**绝对门限"
        );
        // ⚠️ 跨平台（裁决 R25）：响度链路里有 powf／log10，**绝对值不必跨平台相同**，
        // 所以这里断言**性质**而不是精确值：刚过门限的一组同值短时值必须给出一个
        // **有限、非负、极小**的 LRA（同值 ⇒ 上下百分位相同 ⇒ 差值趋零）。
        match loudness_range_from_short_term(&above_the_gate) {
            Some(range) => {
                assert!(
                    range.is_finite(),
                    "同值短时值的 LRA 必须是有限值，实际 {range}"
                );
                assert!(range >= 0.0, "LRA 不能为负，实际 {range}");
                assert!(range < 0.5, "同值短时值的 LRA 必须趋零，实际 {range}");
            }
            None => panic!("刚过门限的值必须过门限"),
        }
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

    /// 判据 (**类别 5: 幂等性**): 同一个预设**重复施加是不动点** —— 第二次施加不改任何
    /// 样本的位型, 且 `gain_db` 报 `0.0`。
    ///
    /// 这条判据钉住的是一个真实的缺口（本机实测, 修复前）: 第一次施加之后母带落在
    /// `−13.9999990463257 LUFS`, 第二次算出的残差是 `−9.5367431640625e-7 dB`
    /// （非零, 恰好是一个读数量化单位）, 于是 `*sample *= 0.99999994` 改掉了
    /// **191,998 / 192,000** 个样本的位型（每个最多差 2 个最低有效位）。
    #[test]
    fn applying_the_same_preset_twice_is_a_bit_exact_no_op() {
        let (mut left, mut right) = (sine_997(0.1, 48_000 * 2), sine_997(0.1, 48_000 * 2));

        let first = ExportPreset::streaming()
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert!(
            first.gain_db > 5.0,
            "第一次必须真的抬了电平, 实际 {} dB",
            first.gain_db
        );
        let after_first: Vec<u32> = left.iter().map(|sample| sample.to_bits()).collect();

        let second = ExportPreset::streaming()
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(second.gain_db, 0.0, "残差必须被当成 0");
        assert_eq!(
            first.bound,
            GainBound::LoudnessTarget,
            "目标响度是那条已经满足的约束"
        );
        assert_eq!(
            left.iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            after_first,
            "第二次施加必须逐位不动样本"
        );

        // ---- 敏感度自证: 那个残差真的非零, 而且真的会改掉样本 ----
        //
        // 没有这一段, 上面那条 `assert_eq!` 可能只是"残差恰好是 0"的空判据。
        let residual = STREAMING_TARGET_LUFS - second.before.integrated_lufs;
        assert_ne!(residual, 0.0, "残差为 0 ⇒ 本判据测不到任何东西, 换一段信号");
        assert!(
            within_measurement_resolution(
                STREAMING_TARGET_LUFS,
                second.before.integrated_lufs,
                residual
            ),
            "残差 {residual} 不落在测量分辨率里 ⇒ 它是一条真实增益, 不该被吃掉"
        );
        let factor = db_to_linear(residual);
        assert_ne!(factor, 1.0, "因子恰好是 1.0 ⇒ 乘不乘都一样, 判据为空");
        let mut if_applied: Vec<f32> = left.clone();
        for sample in &mut if_applied {
            *sample *= factor;
        }
        let changed = if_applied
            .iter()
            .zip(left.iter())
            .filter(|(scaled, kept)| scaled.to_bits() != kept.to_bits())
            .count();
        assert!(
            changed > 0,
            "修复前的代码路径必须真的会改掉样本, 否则本判据没有守门人"
        );
    }

    /// 判据 (守门人, **类别 5: 幂等性**): 测量分辨率的阈值取**两个操作数里量级较大的
    /// 那个**的**上方**间距 —— 两条都必须是文档写的那一条。
    ///
    /// 取**较小**的那个操作数（`min` 替 `max`）时阈值减半; 取**下方**间距
    /// （`magnitude - magnitude.next_down()` 替 `magnitude.next_up() - magnitude`）时,
    /// 量级恰为 2 的幂的那一格阈值同样减半（文档 §"为什么取较大的那个操作数"末段写明
    /// `next_up` 给的是上方那一档）。两者都会让一个**确实落在测量分辨率里**的残差被判成
    /// 真实增益去施加, 于是 [`ExportPreset::apply_at`] 的"重复施加是不动点"承诺失守。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 `max` 改成 `min`、把 `next_up` 改成 `next_down` 各注入一次, **全量 168 条
    /// 判据全绿**（两次 `test result: ok. 168 passed; 0 failed`）——
    /// `applying_the_same_preset_twice_is_a_bit_exact_no_op` 用的是 `−14.0` 与
    /// `−13.999999` 这一对**同一个二进制档**里的操作数, 两个操作数的间距相等, 因此分不出
    /// 上面两种写法。本条改用一对**跨档**的操作数把差异放大成 2 倍。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: [`within_measurement_resolution`] 这一条纯函数, 输入是一对 `f32` 读数
    /// （单位: LUFS）与它们的差（单位: dB）。读数: 布尔判决, 加上三个 `f32` 相等断言。
    /// 本判据**不**依赖任何信号或预设, 因此与测量链的夹具无关。
    ///
    /// # 运算类别（ADR-0001 的 D32）
    ///
    /// 本判据只用到 `abs` / `max` / 减法 / 比较与 `next_up`（IEEE-754 精确类,
    /// 全部是位运算与精确算术）, **不含**超越函数, 因此按 D32 第 1 类**可以在任何架构上
    /// 逐位断言**, 不需要 `cfg!(target_arch)` 的分支。
    #[test]
    fn the_measurement_resolution_uses_the_larger_operand_and_its_upper_spacing() {
        // 8.0 恰是 2 的幂: 它的上方间距是 2^-20, 而它下方第二个可表示值的上方间距只有
        // 2^-21 —— 两者相差一倍。两个间距都写成 `f32::EPSILON` 的整数倍（2^-23 的
        // 倍数）, 因此是**精确**的 2 的幂, 不是十进制近似。
        let upper_spacing = 8.0f32 * f32::EPSILON; // 2^-20
        let lower_spacing = 4.0f32 * f32::EPSILON; // 2^-21
        let demand = 8.0f32;
        // 7.99999904632568359375 = 8.0 - 2^-20（跨档的下方操作数）。
        let actual = f32::from_bits(0x40FF_FFFE);
        // Sterbenz 引理: 两数在 2 倍以内, 减法**精确**, 因此残差恰好是上方间距。
        let residual = demand - actual;

        assert_eq!(residual, upper_spacing, "残差必须恰好是 2^-20");
        assert_eq!(demand.next_up() - demand, upper_spacing);
        assert_eq!(actual.next_up() - actual, lower_spacing);
        assert!(
            residual > lower_spacing,
            "残差必须严格大于下方那一档的间距, 否则本条分不出 max/min 与 next_up/next_down"
        );
        assert!(
            within_measurement_resolution(demand, actual, residual),
            "量级较大的那个操作数的**上方**间距必须吸收这个残差"
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

    /// 判据 (**类别 1/4: 零值与前哨兵的重合**): `to_bext_loudness` 的
    /// `loudness_range` 必须按 0.01 LU 定标自**实测**的 LRA, 不能是一个常数。
    ///
    /// # 为什么既有判据测不到（本机实测的注入读数）
    ///
    /// 本机把 `to_bext_loudness` 里那一支
    /// `Some(range) if range.is_finite() => Loudness::from_lufs(range)` 整支换成常数
    /// **`0`** —— 全量 `cargo test -p yeban-render --no-default-features --lib --tests`
    /// **全绿**（`test result: ok. 179 passed; 0 failed` + 10 + 13）。
    ///
    /// 两条既有判据都挡不住它, 而且**各有各的原因**（因此不能靠加强它们中的任何一条）:
    ///
    /// 1. `the_bext_bridge_scales_finite_readings_and_sentinels_the_rest` 对
    ///    `loudness_range` 只断言 `assert_ne!(.., Loudness::UNKNOWN)` —— `0` 也满足它;
    /// 2. 同一夹具（0.1 幅度、997 Hz、8 s 稳态正弦）的实测 LRA 只有 **1.9073486e-5 LU**
    ///    （本机实测）, 经 `Loudness::from_lufs` 定标后**恰好是 0** —— 所以"拿它当夹具
    ///    去区分 `0` 与 `from_lufs(range)`"这条捷径**在数值上不存在**: 正确实现在那个
    ///    夹具上也给出 0。
    ///
    /// 因此本条判据**不改那个夹具**, 而是直接构造一个合成读数: LRA 取 **2.75 LU**
    /// （定标 275）—— 一个 `0` 与 `from_lufs(range)` 必然分开的值。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个显式构造的 [`MasterLoudness`]（单位: LUFS / LU / dBTP）。读数:
    /// [`MasterLoudness::to_bext_loudness`] 的五个 `i16` 字段（单位: 0.01 刻度）。
    ///
    /// # 非空证明
    ///
    /// `loudness_range_lu = Some(2.75)` ⇒ 期望 `275`; 若实现写成常数 `0`, 该断言必红
    /// （`275 != 0`）。后半段把同一个读数换成 `None` ⇒ 期望 `UNKNOWN`, 而
    /// `275 != UNKNOWN` —— 因此这一对不是"两格都指向同一个值"的退化判据。三个有限读数
    /// 取三个**互不相同**的值, 因此"把三个字段写成同一个来源"也会红。
    ///
    /// # 运算类别（ADR-0001 的 D32）
    ///
    /// 定标是 `scale_hundredths`（×100 后取整）—— 纯乘法与取整, IEEE 精确类, 不含超越
    /// 函数, 因此这里逐位断言, 不给容差。
    #[test]
    fn a_measurable_lra_reaches_the_bext_block_scaled_not_as_a_constant() {
        let unit = MasterLoudness {
            integrated_lufs: -14.0,
            loudness_range_lu: Some(2.75),
            max_momentary_lufs: -12.25,
            max_short_term_lufs: -13.5,
            true_peak_dbtp: -1.0,
            oversampling: TruePeakOversampling::Sixteen,
        };
        let bext = unit.to_bext_loudness();
        assert_eq!(bext.loudness_value, -1400);
        assert_eq!(
            bext.loudness_range, 275,
            "2.75 LU ⇒ 275 (0.01 LU); 常数 0 与 275 必须是两个不同的读数"
        );
        assert_ne!(bext.loudness_range, 0, "本判据的夹具必须让 275 与 0 分开");
        assert_eq!(bext.max_momentary_loudness, -1225);
        assert_eq!(bext.max_short_term_loudness, -1350);
        assert_eq!(bext.max_true_peak_level, -100);
        assert_ne!(
            bext.max_short_term_loudness, bext.max_momentary_loudness,
            "两个最大值的夹具必须互不相同, 否则字段互换测不到"
        );

        let unmeasurable = MasterLoudness {
            loudness_range_lu: None,
            ..unit
        };
        assert_eq!(
            unmeasurable.to_bext_loudness().loudness_range,
            Loudness::UNKNOWN,
            "测不出 LRA ⇒ 必须写哨兵, 不是 0"
        );
        assert_ne!(Loudness::UNKNOWN, 275, "哨兵与那个有限读数必须不同");
    }

    /// `percentile` 的插值规则（R type 7）: 对 `[0, 10]` 取 P10 ⇒ `1.0`（不是 `0.0`）。
    #[test]
    fn percentile_interpolates_between_order_statistics() {
        let values = [0.0f32, 10.0];
        assert!((percentile(&values, 0.10) - 1.0).abs() < 1e-6);
        assert!((percentile(&values, 0.95) - 9.5).abs() < 1e-6);
        assert_eq!(percentile(&[3.0f32], 0.10), 3.0);
    }

    /// 测试用的抖动节点身份（与 `lib.rs` 契约测试同一枚 ULID）。
    fn dither_node() -> yeban_model::EntityId {
        use core::str::FromStr;
        yeban_model::EntityId::from_str("01J8ZK9WQ7F5N2V4B6C8D0E1F2").expect("合法 ULID")
    }

    /// 生产路径上的抖动随机源（种子由调用方给, [ARCH-DET-001]）。
    fn seed_rng(seed: u64) -> crate::rng::DeterministicDitherRng {
        crate::rng::dither_rng_for(seed, dither_node())
    }

    /// 手工构造一个立体声交错的渲染产物（这些判据不需要真的跑 Rayon 调度）。
    fn master_output(left: &[f32], right: &[f32]) -> RenderOutput {
        let mut samples: Vec<f32> = Vec::with_capacity(left.len().min(right.len()) * 2);
        for (left_sample, right_sample) in left.iter().zip(right.iter()) {
            samples.push(*left_sample);
            samples.push(*right_sample);
        }
        let digest = RenderOutput::digest_of(&samples);
        let frames = (samples.len() / 2) as u64;
        RenderOutput {
            samples,
            frames,
            channels: 2,
            blocks: 0,
            longest_path_frames: 0,
            digest,
        }
    }

    /// 一个版本 2 的 `bext` 模板: 响度字段是哨兵, 与 `Bext::for_project` 一致。
    fn metadata() -> Bext {
        Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00")
    }

    /// **导出落点判据**: 归一化之后母带的**实测**响度必须出现在容器的 `bext` 里。
    ///
    /// 手算: −20 LUFS 的 997 Hz 标定音 + 流媒体预设（目标 −14 LUFS、上限 −1 dBTP）
    /// ⇒ 增益 **+6 dB** ⇒ 文件里应写 **−1400**（0.01 LUFS 刻度）。
    ///
    /// **有鉴别力**: 若写模板里的哨兵 `UNKNOWN`, 或写**增益之前**的读数（−2000）,
    /// 这条判据变红 —— 那正是本切片之前仓库里的行为。
    #[test]
    fn the_export_embeds_the_measured_loudness_of_the_normalized_master() {
        let tone = sine_997(0.1, 48_000 * 8);
        let mut master = master_output(&tone, &tone);
        let template = metadata();
        let mut rng = seed_rng(0x0BAD_C0DE_DEAD_BEEF);

        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int24,
            ContainerKind::Rf64,
            &template,
            &mut rng,
        )
        .expect("48 kHz 立体声 + v2 模板");

        assert_eq!(export.outcome.bound, GainBound::LoudnessTarget);
        assert!(
            (export.outcome.gain_db - 6.0).abs() < 0.05,
            "实际 {} dB",
            export.outcome.gain_db
        );
        assert_eq!(export.frames, 48_000 * 8);
        assert_eq!(
            export.payload.len(),
            48_000 * 8 * 2 * 3,
            "24-bit 立体声 ⇒ 每帧 6 字节"
        );

        let parsed = crate::rf64::parse_container(&export.file).expect("读回自研容器");
        assert_eq!(parsed.kind, ContainerKind::Rf64);
        assert_eq!(parsed.sizes.sample_count, export.frames);
        assert_eq!(&export.file[parsed.data.clone()], export.payload.as_slice());

        let bext = parsed.bext.expect("导出必须带 bext");
        let loudness = bext.loudness.expect("版本 2 必须带 EBU R128 响度块");
        // 数字的来源被钉死: 它是增益**之后**那一次测量的桥接结果。
        assert_eq!(
            loudness.loudness_value,
            Loudness::from_lufs(export.outcome.after.integrated_lufs)
        );
        assert_eq!(
            loudness.max_true_peak_level,
            Loudness::from_dbtp(export.outcome.after.true_peak_dbtp)
        );
        assert!(
            (f32::from(loudness.loudness_value) / 100.0 + 14.0).abs() < 0.05,
            "手算 −1400, 实际 {}",
            loudness.loudness_value
        );
        assert!(
            (f32::from(loudness.max_true_peak_level) / 100.0 + 14.0).abs() < 0.15,
            "手算 ≈ −1400, 实际 {}",
            loudness.max_true_peak_level
        );
        assert_ne!(loudness.loudness_range, Loudness::UNKNOWN, "8 s ⇒ LRA 可写");

        // 模板的其余字段原样保留 —— 只有响度被改写。
        assert_eq!(bext.originator_reference, template.originator_reference);
        assert_eq!(bext.description, template.description);
        assert_eq!(bext.coding_history, template.coding_history);
        assert_eq!(bext.version, 2);
        // 摘要描述的是**文件字节**, 不是样本。
        assert_eq!(export.digest, sha256_of(&export.file));
    }

    /// **导出落点判据（格式参数）**: 用 [`Bext::for_project_with_format`] 当模板时,
    /// 导出的**文件字节**里必须出现真实的 `F=`/`W=`, 且**不得**出现任何
    /// `<sample_rate>` / `<bits>` 占位符字面量。
    ///
    /// # 这条判据钉住的是什么
    ///
    /// `export_master` 是**唯一**同时知道 `sample_rate` 与 `depth` 的地方,
    /// 因此"编码历史写真实格式"这件事只能落在调用方给它的模板上。修复前
    /// `Bext::for_project` 的字面量是 `A=PCM,F=<sample_rate>,W=<bits>,M=stereo,T=Yeban`,
    /// 于是导出文件会**声称自己的采样率是一个尖括号标记**（本机实测, 见提交说明）。
    ///
    /// 两档一起钉: 24-bit ⇒ `W=24`, 32f ⇒ `W=32`（`BitDepth::bits()` 的机械读数）。
    #[test]
    fn the_export_lands_the_real_format_in_the_coding_history() {
        fn contains(haystack: &[u8], needle: &[u8]) -> bool {
            haystack
                .windows(needle.len())
                .any(|window| window == needle)
        }

        let tone = sine_997(0.1, 48_000 * 4);
        let cases: [(BitDepth, &str); 2] = [
            (BitDepth::Int24, "A=PCM,F=48000,W=24,M=stereo,T=Yeban"),
            (BitDepth::Float32, "A=PCM,F=48000,W=32,M=stereo,T=Yeban"),
        ];
        for (depth, expected) in cases {
            let mut master = master_output(&tone, &tone);
            let template = Bext::for_project_with_format(
                "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
                "2026-10-08",
                "13:37:00",
                48_000,
                depth.bits(),
            );
            let mut rng = seed_rng(0x0BAD_C0DE_DEAD_BEEF);
            let export = export_master(
                48_000,
                &mut master,
                ExportPreset::streaming(),
                depth,
                ContainerKind::Rf64,
                &template,
                &mut rng,
            )
            .expect("48 kHz 立体声 + v2 模板");

            let parsed = crate::rf64::parse_container(&export.file).expect("读回自研容器");
            let bext = parsed.bext.expect("导出必须带 bext");
            assert_eq!(
                bext.coding_history, expected,
                "{depth:?} 的编码历史必须带真实 F=/W="
            );
            // 判据落在**文件字节**上, 不是落在一个字符串字段上:
            // 真实参数必须真的被写进容器, 而占位符必须真的不在文件里。
            assert!(
                contains(&export.file, expected.as_bytes()),
                "{depth:?}: 编码历史必须逐字节出现在文件里"
            );
            assert!(
                !contains(&export.file, b"<sample_rate>") && !contains(&export.file, b"<bits>"),
                "{depth:?}: 交付文件里不得出现模板占位符"
            );
            // 响度落点不受本判据影响（同一次导出里两条落点各自独立）。
            assert_eq!(
                bext.loudness.expect("v2 必须带响度块").loudness_value,
                Loudness::from_lufs(export.outcome.after.integrated_lufs)
            );
        }
    }

    /// 静音母带: 五个响度字段全部写哨兵 `UNKNOWN`（**不是** 0.0 LUFS 的假读数）,
    /// 增益为 0, 负载逐字节全零。
    #[test]
    fn a_silent_master_writes_unknown_in_every_loudness_field() {
        let silence = vec![0.0f32; 48_000 * 4];
        let mut master = master_output(&silence, &silence);
        let mut rng = seed_rng(1);
        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int24,
            ContainerKind::Rf64,
            &metadata(),
            &mut rng,
        )
        .expect("导出");

        assert_eq!(export.outcome.bound, GainBound::NothingToDo);
        assert_eq!(export.outcome.gain_db, 0.0);
        let loudness = crate::rf64::parse_container(&export.file)
            .expect("读回")
            .bext
            .expect("有 bext")
            .loudness
            .expect("版本 2 有响度块");
        assert_eq!(loudness.loudness_value, Loudness::UNKNOWN);
        assert_eq!(loudness.loudness_range, Loudness::UNKNOWN);
        assert_eq!(loudness.max_true_peak_level, Loudness::UNKNOWN);
        assert_eq!(loudness.max_momentary_loudness, Loudness::UNKNOWN);
        assert_eq!(loudness.max_short_term_loudness, Loudness::UNKNOWN);
        // 增益 0 ⇒ 浮点母带逐位不变。
        assert!(master.samples.iter().all(|sample| *sample == 0.0));
        // 但**负载不是全零字节**: TPDF 抖动在四舍五入**之前**加入
        // （`crate::dither::quantize_i24`）, 因此数字静音得到 ±1 LSB 的噪声。
        // 这是抖动模块的既有契约, 本判据把它写成可执行的读数。
        let quantized: Vec<i32> = export
            .payload
            .as_chunks::<3>()
            .0
            .iter()
            .map(|bytes| i32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]) << 8 >> 8)
            .collect();
        assert_eq!(quantized.len(), 48_000 * 4 * 2);
        assert!(
            quantized.iter().all(|value| value.abs() <= 1),
            "抖动幅度必须 ≤ 1 LSB"
        );
        assert!(
            quantized.iter().any(|value| *value != 0),
            "数字静音也必须带抖动噪声, 而不是被优化成全零"
        );
    }

    /// 三种容器 × 三种位深的九种组合都往返, 且每一个产物都带实测响度块。
    ///
    /// 这条判据让"多格式"变成可执行的读数: 位深标签写错（例如 32f 写成整数标签）
    /// 或容器种类没有按请求写出, 都会在这里变红。
    #[test]
    fn every_container_and_depth_combination_round_trips_with_the_loudness_block() {
        for kind in [
            ContainerKind::Riff,
            ContainerKind::Rf64,
            ContainerKind::Bw64,
        ] {
            for depth in [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32] {
                let tone = sine_997(0.1, 48_000 * 2);
                let mut master = master_output(&tone, &tone);
                let mut rng = seed_rng(0x5EED);
                let export = export_master(
                    48_000,
                    &mut master,
                    ExportPreset::new(Some(-23.0), None),
                    depth,
                    kind,
                    &metadata(),
                    &mut rng,
                )
                .expect("容器 × 位深组合必须能导出");

                let frames = 96_000usize;
                assert_eq!(export.frames, frames as u64);
                assert_eq!(
                    export.payload.len(),
                    frames * 2 * depth.bytes_per_sample(),
                    "{kind:?} × {depth:?}"
                );
                let parsed = crate::rf64::parse_container(&export.file).expect("读回");
                assert_eq!(parsed.kind, kind, "{kind:?} × {depth:?}");
                assert_eq!(
                    parsed.format.bits_per_sample,
                    depth.bits(),
                    "{kind:?} × {depth:?}"
                );
                assert_eq!(
                    parsed.format.is_float,
                    !depth.is_integer(),
                    "{kind:?} × {depth:?}"
                );
                assert_eq!(parsed.format.channels, 2);
                assert_eq!(parsed.format.sample_rate, 48_000);
                assert_eq!(parsed.sizes.sample_count, frames as u64);
                assert_eq!(&export.file[parsed.data.clone()], export.payload.as_slice());
                let loudness = parsed.bext.expect("有 bext").loudness.expect("版本 2");
                assert_ne!(
                    loudness.loudness_value,
                    Loudness::UNKNOWN,
                    "{kind:?} × {depth:?} 必须写实测响度"
                );
                assert_eq!(
                    loudness.loudness_value,
                    Loudness::from_lufs(export.outcome.after.integrated_lufs)
                );
            }
        }
    }

    /// 同种子导出两次 ⇒ **文件逐字节相同**; 换种子 ⇒ 字节必须不同。
    ///
    /// 后半条是 [ARCH-DET-001] 的落地证据: 抖动的随机源只能来自调用方给的 `rng`,
    /// 不能有隐式熵源。
    #[test]
    fn the_same_seed_is_byte_identical_and_a_different_seed_is_not() {
        let tone = sine_997(0.1, 48_000 * 2);
        let run = |seed: u64| {
            let mut master = master_output(&tone, &tone);
            let mut rng = seed_rng(seed);
            export_master(
                48_000,
                &mut master,
                ExportPreset::streaming(),
                BitDepth::Int16,
                ContainerKind::Rf64,
                &metadata(),
                &mut rng,
            )
            .expect("导出")
        };

        let first = run(0xABCD);
        let second = run(0xABCD);
        assert_eq!(first.file, second.file, "同种子必须逐字节相同");
        assert_eq!(first.digest, second.digest);
        assert_eq!(
            first.digest,
            sha256_of(&first.file),
            "digest 必须是文件字节的 SHA-256"
        );

        let other = run(0xABCE);
        assert_ne!(first.file, other.file, "不同抖动种子必须产出不同字节");
        assert_ne!(first.digest, other.digest);
    }

    /// 判据 (**类别 5: 幂等性, 端到端**): 在**同一个** `RenderOutput` 上连续导出两次
    /// （同一个预设、同一种子、同一个容器与位深）⇒ 两次的**文件字节逐字节相同**。
    ///
    /// 与上面那条 [`Self::the_same_seed_is_byte_identical_and_a_different_seed_is_not`] 的
    /// 区别是**输入是不是同一个对象**: 那条每次新建一份母带（"同一输入的两份拷贝"),
    /// 这条是"同一份母带被导出两次"。修复前**这条是红的** —— 第二次导出会把
    /// [`ExportPreset::apply`] 的读数量化残差当成真实增益, 于是母带里
    /// 191,998 / 192,000 个样本的位型被改掉, `file` / `digest` / 负载字节全都不同
    /// （本机实测）。
    ///
    /// 第二次的 `gain_db` 必须是 `0.0`, 而 `bext` 必须与第一次**逐字段相同**
    /// （两次测量的是同一段样本）—— 后者顺带钉住"`bext` 写的是实测值, 不是推算值"。
    #[test]
    fn the_same_master_exported_twice_is_byte_identical() {
        let tone = sine_997(0.1, 48_000 * 2);
        let mut master = master_output(&tone, &tone);

        let first = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int16,
            ContainerKind::Rf64,
            &metadata(),
            &mut seed_rng(0xABCD),
        )
        .expect("第一次导出");
        let audio_after_first = master.samples.clone();

        let second = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int16,
            ContainerKind::Rf64,
            &metadata(),
            &mut seed_rng(0xABCD),
        )
        .expect("第二次导出");

        assert!(
            first.outcome.gain_db > 5.0,
            "第一次必须真的抬了电平, 实际 {} dB",
            first.outcome.gain_db
        );
        assert_eq!(
            second.outcome.gain_db, 0.0,
            "第二次的残差必须被当成 0（否则下面几条会红）"
        );
        assert_eq!(
            master.samples, audio_after_first,
            "第二次导出不得再改母带样本"
        );
        assert_eq!(
            second.file, first.file,
            "同一个母带两次导出的文件必须逐字节相同"
        );
        assert_eq!(second.digest, first.digest);
        assert_eq!(
            second.digest,
            sha256_of(&second.file),
            "digest 必须是文件字节的 SHA-256"
        );
        assert_eq!(second.payload, first.payload, "负载字节必须相同");
        assert_eq!(second.bext, first.bext, "bext 的实测响度必须逐字段相同");
    }

    /// 四种输入缺陷都被**拒绝**, 不静默降级。
    #[test]
    fn malformed_exports_are_refused_instead_of_silently_degraded() {
        // 非立体声: 拒绝, 不做下混。
        for channels in [1usize, 4] {
            let mut master = RenderOutput {
                samples: vec![0.0f32; channels * 10],
                frames: 10,
                channels,
                blocks: 0,
                longest_path_frames: 0,
                digest: [0u8; 32],
            };
            let mut rng = seed_rng(1);
            assert_eq!(
                export_master(
                    48_000,
                    &mut master,
                    ExportPreset::streaming(),
                    BitDepth::Int16,
                    ContainerKind::Riff,
                    &metadata(),
                    &mut rng
                ),
                Err(MasterExportError::NotStereo(channels))
            );
        }

        // 交错长度落单: 拒绝, 不丢弃最后一个样本。
        let mut odd = RenderOutput {
            samples: vec![0.0f32; 101],
            frames: 50,
            channels: 2,
            blocks: 0,
            longest_path_frames: 0,
            digest: [0u8; 32],
        };
        let mut rng = seed_rng(1);
        assert_eq!(
            export_master(
                48_000,
                &mut odd,
                ExportPreset::streaming(),
                BitDepth::Int16,
                ContainerKind::Riff,
                &metadata(),
                &mut rng
            ),
            Err(MasterExportError::RaggedInterleavedBuffer(101))
        );

        // `bext` v1 模板放不下 EBU R128 响度块 [ARCH-FMT-001]: 拒绝, 不改调用方的版本。
        let mut version_one = metadata();
        version_one.version = 1;
        version_one.loudness = None;
        let tone = sine_997(0.1, 48_000);
        let mut master = master_output(&tone, &tone);
        let mut rng = seed_rng(1);
        assert_eq!(
            export_master(
                48_000,
                &mut master,
                ExportPreset::streaming(),
                BitDepth::Int16,
                ContainerKind::Riff,
                &version_one,
                &mut rng
            ),
            Err(MasterExportError::BextCannotCarryLoudness(1))
        );

        // 采样率不在内置四档: 拒绝, 不静默回落到 48 kHz 的系数。
        let mut master = master_output(&tone, &tone);
        let mut rng = seed_rng(1);
        assert_eq!(
            export_master(
                22_050,
                &mut master,
                ExportPreset::streaming(),
                BitDepth::Int16,
                ContainerKind::Riff,
                &metadata(),
                &mut rng
            ),
            Err(MasterExportError::UnsupportedSampleRate(22_050))
        );
    }

    /// **导出落点判据（`bext` 版本的上界）**: 导出器**不得**产出一个本 crate 的
    /// 读取器读不回来的容器。
    ///
    /// # 这条判据钉住的是什么
    ///
    /// [`Bext::from_bytes`] 只接受版本 1 与 2。修复前 [`export_master`] 只挡住
    /// "版本 < 2", 于是一个 `version = 3` 的模板会被 [`Bext::to_bytes`] 照原样写进
    /// 文件（版本 ≥ 2 的 `loudness.is_some()` 断言满足, 因此整块照写）, 而同一份字节
    /// 交给 `crate::rf64::parse_container` 得到
    /// [`Rf64Error::UnsupportedBextVersion`]`(3)` —— 调用方拿到 `Ok`, 交付物却是
    /// 本 crate 自己读不回来的容器。本机实测（修复前）:
    ///
    /// ```text
    /// version=3 -> to_bytes 写出 version 字段=3, from_bytes=Err(UnsupportedBextVersion(3))
    /// ```
    ///
    /// 判据有两半, 缺一半就会留一条后路:
    ///
    /// 1. **拒绝半**: 版本 3 与版本 0 都报
    ///    [`MasterExportError::UnsupportedBextVersion`]（版本 1 的路径仍然报既有的
    ///    [`MasterExportError::BextCannotCarryLoudness`]）;
    /// 2. **正对照**: 版本 2 的模板仍然导出成功, 且**产物真的能被读回来** ——
    ///    只查第 1 半, 把导出整体关掉也会"绿"。
    #[test]
    fn the_export_refuses_a_bext_version_the_reader_cannot_read() {
        let tone = sine_997(0.1, 48_000);

        for version in [0u16, 3, 4] {
            let mut template = metadata();
            template.version = version;
            let mut master = master_output(&tone, &tone);
            let mut rng = seed_rng(1);
            assert_eq!(
                export_master(
                    48_000,
                    &mut master,
                    ExportPreset::streaming(),
                    BitDepth::Int16,
                    ContainerKind::Riff,
                    &template,
                    &mut rng
                ),
                Err(MasterExportError::UnsupportedBextVersion(version)),
                "bext 版本 {version} 的字段表未核验, 导出必须拒绝"
            );
        }

        // 正对照: 版本 2 照旧导出, 且产物能被本 crate 的读取器读回来。
        let mut master = master_output(&tone, &tone);
        let mut rng = seed_rng(1);
        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int16,
            ContainerKind::Riff,
            &metadata(),
            &mut rng,
        )
        .expect("版本 2 是受支持的模板");
        let parsed = crate::rf64::parse_container(&export.file).expect("产物必须能被读回来");
        assert_eq!(parsed.bext.expect("导出必须带 bext").version, 2);
    }

    /// 判据: 一个**读不回同一个字符串**的 `bext` 文本字段必须让导出明确拒绝, 而且拒绝
    /// 发生在**任何改写之前**。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: **2 种**形态（`Description` 含内部 NUL、`CodingHistory` 尾随 NUL）。
    /// 单位: "字段形态"。每一种都查三件事:
    /// ① 错误值精确等于
    /// [`MasterExportError::UnrepresentableBextField`] 且点名该字段;
    /// ② 调用方交进来的 `RenderOutput` 的样本与位级摘要**一位未动**（与调用前的快照比）;
    /// ③ 正对照: 同一个字段的**合法**取值照旧导出, 且容器里读回的字符串与模板逐字节相同。
    ///
    /// # 修复前的字面读数（本机实测: 把 base 的 `mastering.rs` 与 `rf64.rs` 放回工作树,
    /// 加一个临时探针跑出来的一行）
    ///
    /// ```text
    /// BASE export_master = Ok; file = 192676 bytes; read-back Description = "a"
    /// ```
    ///
    /// 也就是说: 调用方拿到 `Ok` 与一份 192676 字节的交付物, 而里面的
    /// `Description` 已经不是它给的那个字符串（`"a\0b"` ⇒ `"a"`）。
    ///
    /// # 注入（本机实测的**字面**红形态）
    ///
    /// 删掉 [`export_master`] 里那次 `field_that_does_not_round_trip` 检查 ⇒ 导出在算缓冲
    /// 容量时会直接调 [`crate::rf64::ContainerPlan::header_bytes`], 而后者对这段畸形字段会经
    /// [`crate::rf64::Bext::to_bytes`] panic（实测: 该函数里的 `panic!`）⇒ 本判据以一个
    /// 非预期的 panic 结束, 因此红。少了"样本与摘要一位未动"那两条断言,
    /// "先施加增益、再在容器层报错"的形态就会溜过去。
    #[test]
    fn the_export_refuses_a_bext_field_the_reader_cannot_read_back() {
        let tone = sine_997(0.1, 48_000);
        let cases: [(&'static str, Bext); 2] = [
            (
                "Description",
                Bext {
                    description: "a\u{0}b".to_owned(),
                    ..metadata()
                },
            ),
            (
                "CodingHistory",
                Bext {
                    coding_history: "A=PCM\u{0}".to_owned(),
                    ..metadata()
                },
            ),
        ];
        for (field, template) in cases {
            let mut master = master_output(&tone, &tone);
            let samples_before = master.samples.clone();
            let digest_before = master.digest;
            let mut rng = seed_rng(1);
            assert_eq!(
                export_master(
                    48_000,
                    &mut master,
                    ExportPreset::streaming(),
                    BitDepth::Int16,
                    ContainerKind::Riff,
                    &template,
                    &mut rng
                ),
                Err(MasterExportError::UnrepresentableBextField { field }),
                "{field}: 含 NUL 的字段必须让导出拒绝, 而不是截断"
            );
            assert_eq!(
                master.samples, samples_before,
                "{field}: 拒绝必须发生在任何改写之前"
            );
            assert_eq!(
                master.digest, digest_before,
                "{field}: 拒绝时位级摘要也必须一位未动"
            );
        }

        // 正对照: 同一个字段的合法取值照旧导出, 且读回的值与模板逐字节相同。
        let mut template = metadata();
        template.description = "a b".to_owned();
        let mut master = master_output(&tone, &tone);
        let mut rng = seed_rng(1);
        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int16,
            ContainerKind::Riff,
            &template,
            &mut rng,
        )
        .expect("合法字段必须照旧导出");
        let parsed = crate::rf64::parse_container(&export.file).expect("读回");
        assert_eq!(
            parsed.bext.expect("有 bext").description,
            "a b",
            "读回的 Description 必须与调用方给的值逐字节相同"
        );
    }

    /// 就地施加增益后, `master.digest` 必须跟上缓冲 —— 摘要不许过期。
    #[test]
    fn the_normalized_master_digest_never_goes_stale() {
        let tone = sine_997(0.1, 48_000 * 4);
        let mut master = master_output(&tone, &tone);
        let before = master.digest;
        let mut rng = seed_rng(1);
        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::streaming(),
            BitDepth::Int24,
            ContainerKind::Rf64,
            &metadata(),
            &mut rng,
        )
        .expect("导出");

        assert_ne!(master.digest, before, "增益改了样本 ⇒ 位级摘要必须变");
        assert_eq!(master.digest, RenderOutput::digest_of(&master.samples));

        // 第 500 帧: 正弦在该点非零, 因此"乘过同一个线性增益"是可观测的。
        let gain = db_to_linear(export.outcome.gain_db);
        assert!((master.samples[500 * 2] - tone[500] * gain).abs() < 1e-6);
        assert!((master.samples[500 * 2 + 1] - tone[500] * gain).abs() < 1e-6);
    }

    /// **非有限样本必须被拒绝, 不能被静默替换成 0。**
    ///
    /// # 这条判据针对的实测行为
    ///
    /// 修复前, [`crate::dither::quantize`] 会把非有限样本**替换**成 `0.0`（32f 路径）
    /// 或一个整数端点（16/24 位路径）, 而 `bext` 仍然写着**实测**的响度与真峰值。
    /// 本机实测（本次改动之前, 用 480 帧的夹具）: 第 10 个样本 `NaN`、第 11 个 `+inf`,
    /// `export_master` 返回 **`Ok`**, 2596 字节的 RIFF 文件, `Float32` 载荷里那两个位置
    /// 是 `0.0`, 而 `bext.max_true_peak_level` = **2408**（+24.08 dBTP）。元数据说"峰值
    /// 很高", 音频说"这里是静音" —— 两者互相矛盾, 调用方拿不到任何信号。
    ///
    /// 同类纪律的既有落点是 [`crate::wav::check_match`]: 越界样本在那里也是**拒绝**。
    #[test]
    fn a_non_finite_master_is_refused_before_the_quantizer_can_zero_it() {
        let frames = 480usize;
        let tone = sine_997(0.1, frames);
        // 交点: 第 10 帧的**右**声道 = 交错下标 21。
        let mut samples: Vec<f32> = Vec::with_capacity(frames * 2);
        for (index, value) in tone.iter().enumerate() {
            samples.push(*value);
            samples.push(if index == 10 { f32::NAN } else { *value });
        }
        let pristine = samples
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        let mut master = master_output(&tone, &tone);
        master.samples = samples;

        // ⚠ **R93**: 这个 `for` 遍历的是本地字面量数组（不会被环境过滤掉）, 但按纪律
        // 仍显式断言下界 —— 将来若有人把它换成"按条件挑深度", 这条会立刻红。
        let depths = [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32];
        assert_eq!(depths.len(), 3, "三种位深都必须被扫到");
        for depth in depths {
            let mut attempt = master.clone();
            let mut rng = seed_rng(1);
            let error = export_master(
                48_000,
                &mut attempt,
                ExportPreset::new(None, None),
                depth,
                ContainerKind::Riff,
                &metadata(),
                &mut rng,
            )
            .expect_err("NaN 必须在量化**之前**被拒绝, 而不是被替换成 0.0");
            // 不能直接 `assert_eq!(error, NonFiniteSamples { value: f32::NAN })`:
            // `NaN != NaN`, 所以这里先比**变体**, 再单独比下标的字面读数。
            match error {
                MasterExportError::NonFiniteSamples { index, value } => {
                    assert_eq!(index, 21, "{depth:?}: 第 10 帧的右声道 = 交错下标 21");
                    assert!(value.is_nan(), "{depth:?}: 报出的取值必须是那个 NaN");
                }
                other => panic!("{depth:?}: 期望 NonFiniteSamples, 得到 {other:?}"),
            }
            // 拒绝是**纯**的: 缓冲逐位保持调用方交进来的样子（没有"先改写再报错"）。
            assert_eq!(
                attempt
                    .samples
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                pristine,
                "{depth:?}: 被拒的导出不许改动母带"
            );
        }

        // `±inf` 走同一条路, 且报出的下标/取值就是那个样本本身。
        for (offset, value) in [(3usize, f32::INFINITY), (7usize, f32::NEG_INFINITY)] {
            let mut samples = vec![0.0f32; 32];
            samples[offset] = value;
            let mut master = master_output(&[0.0f32; 16], &[0.0f32; 16]);
            master.samples = samples;
            let mut rng = seed_rng(1);
            assert_eq!(
                export_master(
                    48_000,
                    &mut master,
                    ExportPreset::streaming(),
                    BitDepth::Int24,
                    ContainerKind::Rf64,
                    &metadata(),
                    &mut rng
                ),
                Err(MasterExportError::NonFiniteSamples {
                    index: offset,
                    value
                })
            );
        }
    }

    /// 造一对**参差**的声道: 右声道是 `common`, 左声道是 `common` 再加 33 帧尾巴,
    /// 尾巴中段一个满幅脉冲（尾巴完全落在右声道不存在的那一段里）。
    ///
    /// 尾巴长度固定 33（16 + 1 + 16）: 尾部脉冲的**两侧各留 16 帧**静音,
    /// 与真峰值检测器的窗长同阶, 保证脉冲不靠相邻样本把读数抬起来。
    fn ragged_pair_with_a_full_scale_tail_spike(common: Vec<f32>) -> (Vec<f32>, Vec<f32>, usize) {
        let right = common.clone();
        let mut left = common;
        let tail_start = left.len();
        left.extend(core::iter::repeat_n(0.0f32, 16));
        left.push(1.0);
        left.extend(core::iter::repeat_n(0.0f32, 16));
        (left, right, tail_start)
    }

    /// 精确 `±1.0` 的**周期 9 方波**: 谐波落在 `k/9`, 其中 `4/9·fs` 正是
    /// 上游台账 `docs/ledger/dsp-loudness-notes.md` §2.5 记录的两个口径的**最坏点**。
    ///
    /// 只用精确可表示的 `±1.0`, **不用** `sin`/`sinf`: 输入位型因此与平台 libm 无关,
    /// 而 `TruePeakDetector::process` 只做 f32 乘加、`abs` 与比较（IEEE 精确类）
    /// ⇒ 本夹具的线性读数**逐位**跨平台相同（ADR-0001 的 D32「按运算类别分策」,
    /// `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`, 现位于第 373 行起）。
    /// 这不是"碰巧稳定", 是刻意避开超越函数。dB 值那一层走 `libm::log10f`
    /// （超越函数类）, 因此判据里的 dB **比较**只在同一次运行内互相比较,
    /// 不写平台相关的十进制字面量。
    fn p9_square(frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|index| if index % 9 < 4 { 1.0f32 } else { -1.0 })
            .collect()
    }

    /// **母带这一侧的真峰值走 16×**, 不是 `TruePeakDetector::new()` 的 8×。
    ///
    /// 依据（原文）: `docs/ledger/dsp-loudness-notes.md` §2.5 的结论
    /// "**默认 8×** …… **母带母线 / 导出天花板用 16×** —— "只付一次"的场景,
    /// 把最坏欠读压到 −0.064 dB, 这对 `-1 dBTP` 这类硬天花板是有意义的余量";
    /// `HD-26`（`docs/ledger/human-decisions.md`, 现位于第 57 行）同口径（"母带 16× 可选"）。
    ///
    /// 判别力（本机实测, 硬红字面量）: 同一个周期 9 方波, 8× 的线性读数是
    /// `0x3FB2A846`、16× 是 `0x3FB33680`。把 [`MASTERING_TRUE_PEAK_OVERSAMPLING`]
    /// 改成 [`TruePeakOversampling::Eight`] ⇒ 红在"母带读数不是 16× 口径"。
    #[test]
    fn the_mastering_true_peak_uses_the_sixteen_times_oversampling() {
        let signal = p9_square(48_000);

        let mut eight = TruePeakDetector::with_oversampling(8).expect("8× 必须可选");
        let mut sixteen = TruePeakDetector::with_oversampling(16).expect("16× 必须可选");
        eight.process(&signal);
        sixteen.process(&signal);
        assert_eq!(eight.true_peak().to_bits(), 0x3FB2_A846, "8× 的线性读数");
        assert_eq!(sixteen.true_peak().to_bits(), 0x3FB3_3680, "16× 的线性读数");
        assert!(
            sixteen.true_peak() > eight.true_peak(),
            "这个夹具必须让两个口径分开, 否则本判据没有区分力"
        );
        // 收紧量是**超越函数类**（两个 `libm::log10f` 之差）: 按 ADR-0001:379-383 的
        // 4096 ulp 预算给容差。本机实测 0.026969 dB, 余量约 27×。
        let db_tightening =
            amplitude_to_dbtp(sixteen.true_peak()) - amplitude_to_dbtp(eight.true_peak());
        assert!(
            (0.02..0.04).contains(&db_tightening),
            "16× 相对 8× 的收紧量本机实测 0.026969 dB, 实际 {db_tightening}"
        );

        let measured = measure_master(48_000, &signal, &signal).expect("48 kHz");
        assert_eq!(
            measured.oversampling,
            TruePeakOversampling::Sixteen,
            "母带读数的口径必须自带, 且必须是 16×"
        );
        assert_eq!(
            measured.true_peak_dbtp.to_bits(),
            amplitude_to_dbtp(sixteen.true_peak()).to_bits(),
            "母带读数不是 16× 口径: 实际 {} dBTP",
            measured.true_peak_dbtp
        );
        assert_ne!(
            measured.true_peak_dbtp.to_bits(),
            amplitude_to_dbtp(eight.true_peak()).to_bits(),
            "8× 与 16× 在这个夹具上必须给出不同的读数"
        );
    }

    /// 显式 **8×** 的入口与 `TruePeakDetector::new()`（8× 默认）**逐位**一致
    /// ⇒ 本切片没有动"逐通道计量"那一档。
    #[test]
    fn the_explicit_eight_times_entry_matches_the_detector_default() {
        let signal = p9_square(48_000);
        let measured = measure_master_at(48_000, &signal, &signal, TruePeakOversampling::Eight)
            .expect("48 kHz");
        assert_eq!(measured.oversampling, TruePeakOversampling::Eight);

        let mut default_detector = TruePeakDetector::new();
        default_detector.process(&signal);
        assert_eq!(default_detector.oversampling(), 8, "new() 必须是 8×");
        assert_eq!(
            measured.true_peak_dbtp.to_bits(),
            amplitude_to_dbtp(default_detector.true_peak()).to_bits(),
            "显式 8× 必须与检测器默认口径同值"
        );
    }

    /// 两个口径都被 `yeban-dsp` 接受（否则 [`measure_master_at`] 会 panic,
    /// 那是"口径漂移"而不是静默回落）。
    #[test]
    fn both_oversampling_choices_are_supported_by_the_detector() {
        for choice in [TruePeakOversampling::Eight, TruePeakOversampling::Sixteen] {
            let detector = TruePeakDetector::with_oversampling(choice.phases());
            assert!(
                detector.is_some(),
                "yeban-dsp 不再支持 {}× —— measure_master_at 会 panic, 必须显式处理",
                choice.phases()
            );
            assert_eq!(
                detector.expect("上面刚断言过").oversampling(),
                choice.phases()
            );
        }
        assert_eq!(TruePeakOversampling::Eight.phases(), 8);
        assert_eq!(TruePeakOversampling::Sixteen.phases(), 16);
        assert_eq!(
            MASTERING_TRUE_PEAK_OVERSAMPLING,
            TruePeakOversampling::Sixteen,
            "母带口径就是台账 §2.5 的 16×"
        );
    }

    /// [`NormalizeOutcome`] 的**两次测量共用同一个口径**。
    ///
    /// 混用会让 `before` 与 `after` 不可比: 增益是按 `before` 算的, 而"上限是否真的被
    /// 达到"是按 `after` 读的。8× 与 16× 在同一条信号上差 0.027 dB（见上一条判据）
    /// ⇒ 混用会让 `GainBound::TruePeakCeiling` 的结论失真。判据同时钉住"两个字段都等于
    /// 请求的那一档", 因此 `before` 用参数、`after` 写死常数这类半途而废的写法会红。
    #[test]
    fn both_normalize_readings_share_one_oversampling() {
        for choice in [TruePeakOversampling::Eight, TruePeakOversampling::Sixteen] {
            let signal = p9_square(48_000);
            let (mut left, mut right) = (signal.clone(), signal);
            let outcome = ExportPreset::new(None, Some(0.0))
                .apply_at(48_000, &mut left, &mut right, choice)
                .expect("48 kHz");
            assert_eq!(
                outcome.before.oversampling, choice,
                "增益之前的测量口径不对"
            );
            assert_eq!(
                outcome.after.oversampling, choice,
                "增益之后的测量口径不对 —— 两次测量必须同口径"
            );
            assert_eq!(outcome.bound, GainBound::TruePeakCeiling);
        }
    }

    /// **导出天花板的口径落到容器**: [`export_master`] 写进 `bext` v2 的
    /// `MaxTruePeakLevel` 是 **16×** 的读数, 不是 8× 的。
    ///
    /// 归一化用**已经满足的上限** `Some(3.5)` dBTP（夹具的真峰值 ≈ 2.92 dBTP）
    /// ⇒ 增益恒 `0.0`、样本逐位不改、`after == before`, 于是 `bext` 里就是输入信号
    /// 自身那一档的读数 —— 口径因此成为唯一变量。
    ///
    /// 本机实测（0.01 dBTP 刻度）: 16× ⇒ **292**, 8× ⇒ **290**。判据按**同一次运行内**
    /// 算出的两个值比较, 不写平台相关的十进制字面量。
    #[test]
    fn the_export_lands_the_sixteen_times_true_peak_in_the_bext() {
        let signal = p9_square(48_000);
        let mut master = master_output(&signal, &signal);
        let before: Vec<u32> = master.samples.iter().map(|s| s.to_bits()).collect();
        let mut rng = seed_rng(0x0BAD_C0DE_DEAD_BEEF);

        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::new(None, Some(3.5)),
            BitDepth::Int24,
            ContainerKind::Rf64,
            &metadata(),
            &mut rng,
        )
        .expect("导出必须成功");

        assert_eq!(export.outcome.bound, GainBound::NothingToDo);
        assert_eq!(export.outcome.gain_db, 0.0);
        assert_eq!(
            before,
            master
                .samples
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            "上限本就满足 ⇒ 样本必须逐位不改"
        );

        let mut sixteen = TruePeakDetector::with_oversampling(16).expect("16×");
        sixteen.process(&signal);
        let mut eight = TruePeakDetector::with_oversampling(8).expect("8×");
        eight.process(&signal);
        let landed = Loudness::from_dbtp(amplitude_to_dbtp(sixteen.true_peak()));
        let eight_only = Loudness::from_dbtp(amplitude_to_dbtp(eight.true_peak()));
        assert_ne!(landed, eight_only, "两个口径必须给出不同的 bext 值");
        assert_eq!(
            export
                .bext
                .loudness
                .expect("v2 模板必带响度块")
                .max_true_peak_level,
            landed,
            "bext 的 MaxTruePeakLevel 必须是 16× 口径（本机实测 292, 8× 是 290）"
        );
    }

    /// **参差声道的真峰值口径**: 长边多出来的尾巴**不得**进真峰值读数。
    ///
    /// 共同区是 0.25 幅度的 997 Hz 正弦 ⇒ 真峰值**本机实测 −12.041201 dBTP**;
    /// 尾巴里的脉冲是 **0 dBTP**。旧实现（两路各自整条切片）读到尾巴那一格。
    ///
    /// 判别力: 把 `measure_master` 里那两个检测器的入参从最短切片还原成整条切片 ⇒ 红。
    #[test]
    fn the_true_peak_of_a_ragged_pair_comes_from_the_shorter_channel() {
        let (left, right, tail_start) =
            ragged_pair_with_a_full_scale_tail_spike(sine_997(0.25, 4_096));
        assert_eq!(left.len() - right.len(), 33, "尾巴必须真的比右声道长");

        let ragged = measure_master(48_000, &left, &right).expect("48 kHz");
        let truncated = measure_master(48_000, &left[..tail_start], &right).expect("48 kHz");

        assert_eq!(
            ragged.true_peak_dbtp.to_bits(),
            truncated.true_peak_dbtp.to_bits(),
            "参差的尾巴进了真峰值: 完整 {} dBTP vs 截断 {} dBTP",
            ragged.true_peak_dbtp,
            truncated.true_peak_dbtp
        );
        assert!(
            (ragged.true_peak_dbtp + 12.041_201).abs() < 0.05,
            "共同区的真峰值本机实测 −12.041201 dBTP, 实际 {}",
            ragged.true_peak_dbtp
        );
    }

    /// **类别⑥ 的实测量**: 单声道信号喂立体声母带器件时, 只填一路 / 两路都填 / 只填另一路
    /// —— 三个读数的关系是本机实测的字面值。
    ///
    /// # 三条读数（对象 = 同一份 2 s、0.1 幅度、997 Hz 单声道信号; 单位 = LUFS / dBTP）
    ///
    /// | 填法 | 积分响度 (LUFS) | 真峰值 (dBTP) |
    /// | :--- | :--- | :--- |
    /// | 只填**左**路, 右路数字静音 | **−23.010252** | **−20.000000** |
    /// | 两路都填同一个信号 | **−19.999952** | **−20.000000** |
    /// | 只填**右**路, 左路数字静音 | **−23.010252** | **−20.000000** |
    ///
    /// # 这两格说明什么（不是"修 bug", 是把既有口径钉住）
    ///
    /// 1. **真峰值与"填在哪个声道"无关**: 两路各自过检测器后取 `max`
    ///    （`measure_master_at` 那两行）, 而 `max` 可交换 ⇒ 只填左与只填右**逐位相同**。
    /// 2. **积分响度不是**, 且差**恰好** `10·log₁₀(2)` = **3.010300 dB**: K 加权后的
    ///    声道功率是**相加**的（`GatedLoudness::integrated_stereo_at`）, 因此同一份信号
    ///    放两路比放一路响 3.01 LU。实测差 = `−19.999952 − (−23.010252)` = `3.010300`,
    ///    与理论值在全部打印位小数上相同。
    ///
    /// # 因此"单声道输入进立体声导出"这条路**在导出层被拒绝**, 不是在这里被下混
    ///
    /// 上面第 2 格意味着: 若让"一路填满、一路静音"的母带走完导出, 响度会按**单声道口径**
    /// 报低 3.01 LU 并被抬 3.01 dB 的增益 —— 于是同一份单声道内容**复制到两路**与**只放一路**
    /// 会得到两个不同的交付物。`export_master` 的 `NotStereo` 守卫（判据
    /// `malformed_exports_are_refused_instead_of_silently_degraded`）正是在这里生效:
    /// **不做下混、不复制、不猜**。本判据把该守卫要挡的那 3.01 dB 量成字面值。
    ///
    /// 判别力: 把真峰值那两行改成"按左路为准"（去掉 `max`）⇒ 第三条断言红;
    /// 让响度只算单声道功率（去掉声道求和）⇒ 第二条断言红。
    #[test]
    fn a_mono_signal_in_a_stereo_device_is_three_lufs_louder_when_both_channels_carry_it() {
        let mono = sine_997(0.1, 96_000);
        let silent = vec![0.0f32; mono.len()];

        let left_only = measure_master(48_000, &mono, &silent).expect("48 kHz");
        let both = measure_master(48_000, &mono, &mono).expect("48 kHz");
        let right_only = measure_master(48_000, &silent, &mono).expect("48 kHz");

        // 1. 真峰值对"填在哪个声道"可交换, 且三个读数的真峰值是同一个数。
        assert_eq!(
            left_only.true_peak_dbtp.to_bits(),
            right_only.true_peak_dbtp.to_bits(),
            "真峰值取两路 max, 只填左与只填右必须逐位相同: {} vs {}",
            left_only.true_peak_dbtp,
            right_only.true_peak_dbtp
        );
        assert_eq!(
            left_only.true_peak_dbtp.to_bits(),
            both.true_peak_dbtp.to_bits(),
            "两声道的真峰值必须相同: {} vs {}",
            left_only.true_peak_dbtp,
            both.true_peak_dbtp
        );
        assert_eq!(
            both.true_peak_dbtp.to_bits(),
            (-20.0f32).to_bits(),
            "本机实测的真峰值字面值 −20.000000 dBTP, 实际 {}",
            both.true_peak_dbtp
        );
        // 2. 积分响度按声道功率求和 ⇒ 两路差恰好 10·log10(2)。
        assert_eq!(left_only.integrated_lufs, -23.010_252, "只填一路的本机实测");
        assert_eq!(both.integrated_lufs, -19.999_952, "两路都填的本机实测");
        assert_eq!(
            left_only.integrated_lufs.to_bits(),
            right_only.integrated_lufs.to_bits(),
            "响度对左右互换必须对称: {} vs {}",
            left_only.integrated_lufs,
            right_only.integrated_lufs
        );
        let delta = both.integrated_lufs - left_only.integrated_lufs;
        assert!(
            (delta - 3.010_3).abs() < 1e-6,
            "两路与一路的差必须是 10·log10(2) = 3.010300 LU, 实际 {delta}"
        );

        // 3. 短时/瞬时窗口的口径: 2 s 信号给不出 LRA（3 s 窗口从未填满）, 而**瞬时**
        //    （400 ms）这一个最大值也照同一条 3.01 dB 走 —— 两条独立读数互为交叉验证。
        assert_eq!(
            left_only.max_momentary_lufs, -23.008_871,
            "本机实测的瞬时最大值"
        );
        assert_eq!(both.max_momentary_lufs, -19.998_571, "本机实测的瞬时最大值");
        assert_eq!(
            left_only.loudness_range_lu, None,
            "2 s 信号不得给出 LRA（3 s 窗口没填满）—— 若这里出了数, 说明读数覆盖了超过 2 s"
        );
        assert_eq!(both.loudness_range_lu, None);
        assert_eq!(right_only.loudness_range_lu, None);
    }

    /// **参差声道下的真峰值上限必须真的被达到**: [`GainBound::TruePeakCeiling`] 不是一句空话。
    ///
    /// 尾巴里的满幅脉冲不在母带区域内（[`ExportPreset::apply`] 只改最短的 `min` 帧）,
    /// 因此它不该抬 `before`、也不该让 `after` 停在 0 dBTP。
    ///
    /// 旧实现: 真峰值看整条切片 ⇒ `before` = 0 dBTP ⇒ 增益被上限压到 −20 dB,
    /// 而尾巴一位未改 ⇒ `after.true_peak_dbtp` 仍是 **0 dBTP** ⇒ 本判据红。
    #[test]
    fn a_true_peak_ceiling_is_really_achieved_on_ragged_channels() {
        let (mut left, mut right, tail_start) =
            ragged_pair_with_a_full_scale_tail_spike(sine_997(0.25, 4_096));
        let tail_before: Vec<u32> = left[tail_start..]
            .iter()
            .map(|sample| sample.to_bits())
            .collect();

        let outcome = ExportPreset::new(None, Some(-20.0))
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");

        assert_eq!(outcome.bound, GainBound::TruePeakCeiling);
        assert!(
            outcome.after.true_peak_dbtp <= -20.0 + 1e-3,
            "声称上限赢, 输出却是 {} dBTP",
            outcome.after.true_peak_dbtp
        );
        assert_eq!(
            left[tail_start..]
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            tail_before,
            "尾巴不在母带区域内, 增益不该碰它"
        );
    }

    /// **非有限的目标/上限不是约束**: 归一化的报告不许出现"算出 inf 增益却没动样本"。
    ///
    /// 旧实现: `Some(f32::INFINITY)` 的目标 ⇒ `gain_db = inf`、`bound = LoudnessTarget`,
    /// 而 `db_to_linear(inf)` 返回 `1.0` ⇒ 样本一位未动 —— 报告与实际互相矛盾。
    /// `Some(f32::NEG_INFINITY)` 的上限同理 ⇒ `gain_db = -inf`、`bound = TruePeakCeiling`。
    ///
    /// 判别力: 去掉 `apply` 里的 `target.is_finite()` 或 `ceiling.is_finite()` ⇒ 红。
    #[test]
    fn a_non_finite_target_or_ceiling_is_not_a_constraint() {
        let tone = sine_997(0.1, 48_000 * 2);

        for target in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let (mut left, mut right) = (tone.clone(), tone.clone());
            let pristine: Vec<u32> = left.iter().map(|sample| sample.to_bits()).collect();
            let outcome = ExportPreset::new(Some(target), None)
                .apply(48_000, &mut left, &mut right)
                .expect("48 kHz");
            assert!(
                outcome.gain_db.is_finite(),
                "目标 {target} 算出了 {} dB 的增益",
                outcome.gain_db
            );
            assert_eq!(outcome.gain_db, 0.0, "目标 {target}");
            assert_eq!(outcome.bound, GainBound::NothingToDo, "目标 {target}");
            assert_eq!(
                left.iter()
                    .map(|sample| sample.to_bits())
                    .collect::<Vec<_>>(),
                pristine,
                "目标 {target}: 0 dB 增益必须逐位不动样本"
            );
        }

        for ceiling in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let (mut left, mut right) = (tone.clone(), tone.clone());
            let pristine: Vec<u32> = left.iter().map(|sample| sample.to_bits()).collect();
            let outcome = ExportPreset::new(None, Some(ceiling))
                .apply(48_000, &mut left, &mut right)
                .expect("48 kHz");
            assert_eq!(outcome.gain_db, 0.0, "上限 {ceiling}");
            assert_eq!(outcome.bound, GainBound::NothingToDo, "上限 {ceiling}");
            assert_eq!(
                left.iter()
                    .map(|sample| sample.to_bits())
                    .collect::<Vec<_>>(),
                pristine,
                "上限 {ceiling}"
            );
        }
    }

    /// **导出拒绝一个把样本推成非有限的增益**（目标本身有限, 但算出的增益溢出）。
    ///
    /// 实测: 2 秒 0.1 幅度的 997 Hz 正弦（本机读数 −19.999952 LUFS）+ 目标 `1e38` LUFS
    /// ⇒ 增益 `1e38` dB ⇒ `db_to_linear` 溢出成 `+inf` ⇒ 全部样本变成 `+inf`。
    /// 旧实现照旧返回 `Ok`: 容器写成功, 里面的样本是量化器钳出来的轨, `bext`
    /// 里是一份由钳位值算出的读数 —— 正是 [`MasterExportError::NonFiniteSamples`]
    /// 要挡的那件事, 只是入口从"调用方的缓冲"换成了"预设算出的增益"。
    ///
    /// 判别力: 去掉 `export_master` 里增益之后那两遍有限性检查 ⇒ 红。
    #[test]
    fn the_export_refuses_a_gain_that_overflows_the_master() {
        let tone = sine_997(0.1, 48_000 * 2);
        let mut master = master_output(&tone, &tone);
        let pristine: Vec<u32> = master
            .samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect();
        let digest_before = master.digest;
        let mut rng = seed_rng(0x0BAD_C0DE_DEAD_BEEF);

        let error = export_master(
            48_000,
            &mut master,
            ExportPreset::new(Some(1.0e38), None),
            BitDepth::Int24,
            ContainerKind::Riff,
            &metadata(),
            &mut rng,
        )
        .expect_err("溢出成 ±inf 的母带必须在回写之前被拒绝");

        assert!(
            matches!(error, MasterExportError::NonFiniteSamples { .. }),
            "期望 NonFiniteSamples, 得到 {error:?}"
        );
        // 拒绝是**纯**的: 缓冲与位级摘要一位未动。
        assert_eq!(
            master
                .samples
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            pristine,
            "被拒的导出不许改动母带"
        );
        assert_eq!(master.digest, digest_before, "被拒的导出不许改动摘要");
    }

    /// 判据 (守门人, **类别 4: 极值与不可用读数**): 响度范围取到**非有限**值时,
    /// [`MasterLoudness::to_bext_loudness`] 必须写哨兵 [`Loudness::UNKNOWN`], 不得把它
    /// 当成一个合法读数缩放进 `bext`。
    ///
    /// # 为什么这一格可达
    ///
    /// 五个响度字段都是 `pub`, 而 [`MasterLoudness`] 是 `#[derive(Clone, Copy)]` 的公开
    /// 数据载体: 调用方（或将来另一条测量链）可以把 `loudness_range_lu` 写成
    /// `Some(f32::INFINITY)`。其余四个字段的"非有限 ⇒ 哨兵"由真实静音夹具覆盖
    /// （`the_bext_bridge_scales_finite_readings_and_sentinels_the_rest` 走的是
    /// `measure_master` 的静音读数）, 只有 `loudness_range` 这一支**走不到**:
    /// 它的 `None`（低于绝对门限 / 样本不足 3 s）与"`Some(非有限)`"是两回事,
    /// 而静音夹具只产出 `None`。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个手工构造的 [`MasterLoudness`], 只把 `loudness_range_lu` 依次置为
    /// `Some(inf)` / `Some(NaN)` / `Some(7.5)`。单位: LRA 是 **0.01 LU**（`bext` 的
    /// `LoudnessRange` 刻度）。读数: 三个 `i16`。
    ///
    /// # 运算类别（ADR-0001 的 D32）
    ///
    /// 本判据只用 `is_finite` / `×100` / `round` 与整数比较, 不含超越函数, 因此按 D32
    /// 第 1 类**可以在任何架构上逐位断言**。
    ///
    /// 注入证明（本机实测）: 删掉 `to_bext_loudness` 里 `Some(range)` 的
    /// `if range.is_finite()` 守卫之后本轮注入里的这一次**全绿**（192 条判据无一变红）;
    /// 本判据在那条注入下变红
    /// （`inf` 会被 `scale_hundredths` 饱和成 `i16::MAX`）。
    #[test]
    fn a_non_finite_loudness_range_becomes_the_unknown_sentinel() {
        let mut loudness = MasterLoudness {
            integrated_lufs: -20.0,
            loudness_range_lu: Some(f32::INFINITY),
            max_momentary_lufs: -20.0,
            max_short_term_lufs: -20.0,
            true_peak_dbtp: -6.0,
            oversampling: MASTERING_TRUE_PEAK_OVERSAMPLING,
        };
        assert_eq!(
            loudness.to_bext_loudness().loudness_range,
            Loudness::UNKNOWN,
            "+inf 的 LRA 必须写成哨兵"
        );
        loudness.loudness_range_lu = Some(f32::NAN);
        assert_eq!(
            loudness.to_bext_loudness().loudness_range,
            Loudness::UNKNOWN,
            "NaN 的 LRA 必须写成哨兵"
        );
        loudness.loudness_range_lu = Some(f32::NEG_INFINITY);
        assert_eq!(
            loudness.to_bext_loudness().loudness_range,
            Loudness::UNKNOWN,
            "-inf 的 LRA 必须写成哨兵"
        );
        // 防空判据: 有限值必须照常按 0.01 LU 缩放 —— 否则上面三条只是把功能关掉。
        loudness.loudness_range_lu = Some(7.5);
        assert_eq!(loudness.to_bext_loudness().loudness_range, 750);
        loudness.loudness_range_lu = Some(0.0);
        assert_eq!(loudness.to_bext_loudness().loudness_range, 0);
    }

    /// 判据: 写进容器的帧数取自**缓冲的实际长度**, 不是 [`RenderOutput::frames`] 字段 ——
    /// 后者是调用方给的数据, 可能与缓冲不一致。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `let frames = master.samples.len() / 2;` 换成 `master.frames as usize` 之后,
    /// 全量判据**全绿** —— 既有的夹具（[`master_output`]）让那个字段恒等于
    /// `samples.len() / 2`, 两者在每一格上都相等。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 一个**故意说谎**的 [`RenderOutput`]（缓冲 2 000 帧, 字段写 7 帧）。
    /// 读数: [`MasterExport::frames`]（帧）、`data` 负载长度（字节）与
    /// `ds64.sampleCount`（帧）—— 三个读数各自独立地指向缓冲长度。
    ///
    /// # 非空证明
    ///
    /// `7 ≠ 2 000`（下面直接断言）, 而 24 位立体声的负载长度是 `2 000 × 2 × 3`
    /// = 12 000 字节 —— 字段那一版会给 7。
    #[test]
    fn the_container_frame_count_comes_from_the_buffer_not_the_field() {
        let tone = sine_997(0.1, 2_000);
        let mut master = master_output(&tone, &tone);
        assert_eq!(master.frames, 2_000, "夹具的实际帧数");
        master.frames = 7; // 调用方给的字段与缓冲不一致
        let mut rng = seed_rng(0x0BAD_C0DE_DEAD_BEEF);

        let export = export_master(
            48_000,
            &mut master,
            ExportPreset::new(None, None),
            BitDepth::Int24,
            ContainerKind::Rf64,
            &metadata(),
            &mut rng,
        )
        .expect("导出必须成功");

        assert_ne!(export.frames, master.frames, "字段与缓冲必须真的不一致");
        assert_eq!(export.frames, 2_000, "帧数必须来自缓冲的实际长度");
        assert_eq!(
            export.payload.len(),
            2_000 * 2 * 3,
            "24 位立体声的负载字节数"
        );
        let parsed = crate::rf64::parse_container(&export.file).expect("解析");
        assert_eq!(parsed.sizes.sample_count, 2_000, "ds64 的 sampleCount");
    }

    /// 判据 (**类别 4: 极值**): **有限**但顶到 `f32::MAX` 的目标**不是**"已经在目标上"。
    ///
    /// # 缺口（本机实测, 修复前）
    ///
    /// 测量分辨率是 `magnitude.next_up() - magnitude`, 而
    /// `f32::MAX.next_up() == f32::INFINITY` ⇒ 量级恰为 `f32::MAX` 的那一格分辨率是
    /// **`+inf`**, 于是 `residual <= inf` 恒真、`gain_db` 被改写成 `0.0`。实测:
    /// 2 s −19.999952 LUFS 的母带 + 目标 `Some(f32::MAX)` LUFS ⇒ 修复前返回
    /// `NormalizeOutcome { gain_db: 0.0, bound: LoudnessTarget }` 而样本**一位未动** ——
    /// 报告说"已经在目标上", 而目标差 3.4e38 LUFS。这与既有的
    /// `a_non_finite_target_or_ceiling_is_not_a_constraint` 要消灭的自相矛盾是同一件事,
    /// 只是入口换成了一个**有限**数。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 2 s、0.1 幅度、997 Hz 立体声（单位: LUFS / dBTP）与目标 `Some(f32::MAX)`
    /// （单位: LUFS）。读数: [`NormalizeOutcome::gain_db`]（dB）、
    /// [`NormalizeOutcome::bound`]、样本位型, 以及 [`export_master`] 的判决。
    ///
    /// # 非空证明
    ///
    /// 后半段把目标换成一个普通的有限值（`-14.0`）⇒ 同一个结构必须给出
    /// `bound = LoudnessTarget`、`gain_db > 5` 且样本**真的被改**。少了这一半,
    /// "把所有增益都算成 0"也会让前半段全绿。
    #[test]
    fn a_finite_target_at_the_top_of_the_range_is_not_already_met() {
        let tone = sine_997(0.1, 48_000 * 2);
        let pristine: Vec<u32> = tone.iter().map(|sample| sample.to_bits()).collect();

        let (mut left, mut right) = (tone.clone(), tone.clone());
        let outcome = ExportPreset::new(Some(f32::MAX), None)
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(
            outcome.bound,
            GainBound::LoudnessTarget,
            "有限的目标仍然是那条起作用的约束"
        );
        assert!(
            outcome.gain_db > 1.0e30,
            "目标比实测响度高约 3.4e38 LUFS ⇒ 增益必须是一个巨大的正数（dB）, 实际 {}",
            outcome.gain_db
        );

        // 与目标一致: 那个增益把样本推出 f32 有限域 ⇒ 导出必须拒绝
        // （与 `the_export_refuses_a_gain_that_overflows_the_master` 的 `1e38` 同一条纪律）。
        let mut master = master_output(&tone, &tone);
        let mut rng = seed_rng(0x0BAD_C0DE_DEAD_BEEF);
        let error = export_master(
            48_000,
            &mut master,
            ExportPreset::new(Some(f32::MAX), None),
            BitDepth::Int24,
            ContainerKind::Riff,
            &metadata(),
            &mut rng,
        )
        .expect_err("溢出成 ±inf 的母带必须在回写之前被拒绝");
        assert!(
            matches!(error, MasterExportError::NonFiniteSamples { .. }),
            "期望 NonFiniteSamples, 得到 {error:?}"
        );

        // 防空判据: 普通有限目标照常施加增益（不是"一律不施加"）。
        let (mut left, mut right) = (tone.clone(), tone.clone());
        let ordinary = ExportPreset::new(Some(-14.0), None)
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(ordinary.bound, GainBound::LoudnessTarget);
        assert!(ordinary.gain_db > 5.0, "实际 {}", ordinary.gain_db);
        assert_ne!(
            left.iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            pristine,
            "普通目标必须真的改掉样本"
        );
    }

    /// 判据 (**类别 4: 极值**): 次正规（subnormal）幅度**不是**"测不出" ——
    /// [`amplitude_to_dbtp`] 的契约是"幅度 ≤ 0 ⇒ `NEG_INFINITY`", 而次正规数是 **> 0**。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 把 `if amplitude > 0.0` 换成 `if amplitude >= f32::MIN_POSITIVE`（即把次正规数
    /// 当成 0）之后, 全量判据**全绿** —— 既有的每一处真峰值夹具的幅度都在 `1e-1` 量级,
    /// 没有一格落在次正规区。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 3 个正的线性幅度（单位: 满量程的线性倍数）: `f32::MIN_POSITIVE`（最小的
    /// **正规**数）、`f32::MIN_POSITIVE / 2.0` 与 `f32::from_bits(1)`（两者都是次正规数）。
    /// 读数: [`amplitude_to_dbtp`] 的返回（单位: dBTP）。
    ///
    /// # 运算类别（ADR-0001 的 D32）
    ///
    /// `libm::log10f` 是超越函数 ⇒ 按 4096 ulp 预算**只断言性质**（有限、单调、
    /// 落在一个宽区间内）, **不钉精确值**。
    ///
    /// # 非空证明
    ///
    /// 三个正幅度的读数**互不相同**且随幅度单调下降（下面直接断言两条严格不等）;
    /// 而 `0.0` / `-0.0` / 负数必须给 `NEG_INFINITY` —— 两半一起, 才区分得开
    /// "把正数当 0"与"把 0 当正数"。
    #[test]
    fn a_subnormal_amplitude_is_a_measurement_not_silence() {
        let smallest_normal = f32::MIN_POSITIVE;
        let half_normal = smallest_normal / 2.0;
        let smallest_subnormal = f32::from_bits(1);
        assert!(
            half_normal > 0.0 && half_normal < smallest_normal,
            "夹具必须是次正规数"
        );
        assert!(smallest_subnormal > 0.0 && smallest_subnormal < half_normal);

        for amplitude in [smallest_normal, half_normal, smallest_subnormal] {
            let reading = amplitude_to_dbtp(amplitude);
            assert!(
                reading.is_finite(),
                "幅度 {amplitude:e} 是一个真实读数, 不能是 {reading}"
            );
            assert!(
                (-920.0..-700.0).contains(&reading),
                "幅度 {amplitude:e} 的 dBTP 应落在 (−920, −700) 内, 实际 {reading}"
            );
        }
        // 单调: 幅度越小, 读数越低。
        assert!(amplitude_to_dbtp(smallest_normal) > amplitude_to_dbtp(half_normal));
        assert!(amplitude_to_dbtp(half_normal) > amplitude_to_dbtp(smallest_subnormal));
        // 恰好为零（含 `-0.0`）与负数才是"测不出"。
        assert_eq!(amplitude_to_dbtp(0.0), f32::NEG_INFINITY);
        assert_eq!(amplitude_to_dbtp(-0.0), f32::NEG_INFINITY);
        assert_eq!(amplitude_to_dbtp(-1.0e-30), f32::NEG_INFINITY);
    }

    /// 判据 (**静音 + 只给上限**): 一个测不出真峰值的母带**不得**因为"上限"而被算出
    /// 一个无穷增益。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// 既有的 `silence_is_never_normalized_and_never_becomes_nan` 用的是
    /// **流媒体预设**（目标 + 上限）: 目标那一支因为"响度测不出"被跳过, 上限那一支
    /// 也只有一个 `Some(-1.0)`。把上限那一支的 `if allowed_db < gain_db` 改成
    /// `if true`（无条件采用上限给出的增益）之后, 静音的 `allowed_db =
    /// ceiling − (−inf) = +inf` ⇒ `gain_db = +inf`、`bound = TruePeakCeiling`,
    /// 而**既有那条判据仍然全绿**（它不带上限单独出现的这一格）。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 1 s 的数字静音立体声（单位: 线性样本）与预设 `(None, Some(-1.0))`
    /// （单位: dBTP）。读数: [`NormalizeOutcome::bound`]、`gain_db`（dB）、
    /// `before.true_peak_dbtp`（dBTP）与两侧样本的位型（`u32`）。
    ///
    /// # 非空证明
    ///
    /// 后半段用**非静音**的同一结构作对照: 它必须真的有动作（`bound` 不是
    /// `NothingToDo`, 或样本真的被改）—— 少了这一半, "把整个上限分支关掉"也会让前半段全绿。
    #[test]
    fn a_silent_master_with_only_a_ceiling_stays_untouched() {
        let mut left = vec![0.0f32; 48_000];
        let mut right = vec![0.0f32; 48_000];
        let outcome = ExportPreset::new(None, Some(-1.0))
            .apply(48_000, &mut left, &mut right)
            .expect("48 kHz");
        assert_eq!(outcome.bound, GainBound::NothingToDo);
        assert_eq!(outcome.gain_db, 0.0, "静音上不许算出一个无穷增益");
        assert!(
            !outcome.before.true_peak_dbtp.is_finite(),
            "静音的真峰值就是「测不出」"
        );
        assert!(left.iter().all(|sample| sample.to_bits() == 0));
        assert!(right.iter().all(|sample| sample.to_bits() == 0));

        // 对照: 非静音 + 同一个上限 ⇒ 这一支必须真的有动作。
        let tone = sine_997(0.5, 48_000);
        let (mut loud_left, mut loud_right) = (tone.clone(), tone);
        let loud = ExportPreset::new(None, Some(-20.0))
            .apply(48_000, &mut loud_left, &mut loud_right)
            .expect("48 kHz");
        assert_eq!(loud.bound, GainBound::TruePeakCeiling);
        assert!(loud.gain_db < 0.0, "实际 {} dB", loud.gain_db);
    }

    /// 判据 (**R58**): [`GainBound`] 与 [`TruePeakOversampling`] 的相等必须读**每个变体**。
    ///
    /// # 为什么既有判据测不到（本机注入实测的读数, `--no-fail-fast`）
    ///
    /// `assert_eq!(outcome.bound, GainBound::…, ..)` 在本文件里出现 **16 次**,
    /// `assert_eq!(measured.oversampling, TruePeakOversampling::…)` 出现 **5 次** ——
    /// 两条链**全部是正向**, 一个恒真的 `==` 会让这 **21 条断言一起变成空判据**
    /// （"预设到底由谁定下"与"读数是什么口径"这两件对外承诺会同时失守）。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: `GainBound` 的 3 格与 `TruePeakOversampling` 的 2 格（单位: 一个取值）。
    /// 读数: `==` / `!=` 的判决（3 + 1 对）。
    ///
    /// # 非空证明
    ///
    /// 两族各自的格子**两两不等**且自反 ⇒ 恒真/恒假两种退化都会红。
    #[test]
    fn the_gain_bound_and_oversampling_equality_are_pinned() {
        let bounds = [
            GainBound::NothingToDo,
            GainBound::LoudnessTarget,
            GainBound::TruePeakCeiling,
        ];
        for (index, left) in bounds.iter().enumerate() {
            // ⚠ R75 自审: 自反探针（`x == x`）强度很弱, 只挡"`==` 恒假"的实现;
            // 真正的牙是下面"三格两两不等"与 8×/16× 那一对。
            assert_eq!(*left, *left, "自反");
            for right in &bounds[index + 1..] {
                assert_ne!(*left, *right, "{left:?} 与 {right:?} 必须不同");
            }
        }
        assert_eq!(bounds.len(), 3, "三种归属");
        let oversampling = [TruePeakOversampling::Eight, TruePeakOversampling::Sixteen];
        assert_eq!(oversampling[0], TruePeakOversampling::Eight, "自反");
        assert_ne!(
            oversampling[0], oversampling[1],
            "8× 与 16× 必须不同（母带一侧的口径就是靠它区分的）"
        );
        assert_eq!(oversampling.len(), 2, "两档过采样");
    }
}
