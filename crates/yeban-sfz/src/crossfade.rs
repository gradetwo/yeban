//! SFZ 交叉淡化（`xfin_*` / `xfout_*`）：按键盘位置 / 力度 / MIDI CC 缩放 region 的音量。
//!
//! 这一族在规范里是**三个维度 × 两个方向**的六个骨架，外加三条曲线选择：
//!
//! | 骨架 | 驱动量 | 规范出处 |
//! | :--- | :--- | :--- |
//! | `xfin_lokey` / `xfin_hikey` | 键盘位置（MIDI 音号） | <https://sfzformat.com/opcodes/xfin_lokey/> |
//! | `xfin_lovel` / `xfin_hivel` | 力度 | <https://sfzformat.com/opcodes/xfin_lovel/> |
//! | `xfin_loccN` / `xfin_hiccN` | MIDI CC N | <https://sfzformat.com/opcodes/xfin_loccN/> |
//! | `xfout_lokey` / `xfout_hikey` | 键盘位置 | <https://sfzformat.com/opcodes/xfout_lokey/> |
//! | `xfout_lovel` / `xfout_hivel` | 力度 | <https://sfzformat.com/opcodes/xfout_lovel/> |
//! | `xfout_loccN` / `xfout_hiccN` | MIDI CC N | <https://sfzformat.com/opcodes/xfout_loccN/> |
//! | `xf_keycurve` / `xf_velcurve` / `xf_cccurve` | 上表三个维度的曲线 | <https://sfzformat.com/opcodes/xf_cccurve/> |
//!
//! 规范正文对两个方向的措辞（`xfin_lovel` / `xfout_loccN` 两页）：
//!
//! > The volume of the region will be zero for velocities lower than or equal to
//! > `xfin_lovel`, and maximum (as defined by the volume opcode) for velocities greater
//! > than or equal to `xfin_hivel`.
//!
//! > The volume of the region will be maximum (as defined by the volume opcode) for
//! > values of the MIDI continuous controller N lower than or equal to `xfout_loccN`,
//! > and zero for values greater than or equal to `xfout_hiccN`.
//!
//! 即：**淡入**从下界处的 0 升到上界处的 1，**淡出**从下界处的 1 降到上界处的 0。
//! 本模块把这两个端点读成闭区间 `[low, high]` 上的归一化位置
//! `t = (value - low) / (high - low)`（区间外按上面的正文钳到 0 / 1）。
//!
//! # 缺省端点：两条骨架各自的“中性”缺省
//!
//! 规范表（`_data/sfz/syntax.yml`，与 `docs/ledger/sfz-core-notes.md` 第 2 节同源）给的
//! Default 是**每个端点各一个值**：`xfin_*` 一族的下界 0、上界 0；`xfout_*` 一族的
//! 下界 127、上界 127（`xfout_lokey` / `xfout_hikey` / `xfout_lovel` / `xfout_hivel` 四行的
//! Default 都是 127）。因此：
//!
//! - 只给上界的淡入（语料里 `xfin_hicc1` 的**主要**用法）⇒ `[0, hicc]`，是真正的淡入；
//! - 只给下界的淡出（语料里 `xfout_locc1` 的主要用法）⇒ `[locc, 127]`，是真正的淡出；
//! - 两端都缺省时区间长度为 0。
//!
//! **长度为 0 或为负的区间在本 crate 是“不生效”**：`gain = 1.0`（音量不变），
//! 两个方向都一样。这条是工程裁决，理由有三：① 规范正文只描述
//! “下界处 0、上界处 1”，长度 0 的区间没有可插值的内部；② 参考实现 sfizz
//! （BSD-2-Clause，`src/sfizz/ModifierHelpers.h` 的 `crossfadeIn` / `crossfadeOut`）
//! 对 `length <= 0` 也返回 1.0；③ 让“只给了无用那一端”的输入退化成**不改变音量**，
//! 而不是把整层静音 —— 后者会在语料上把 `xfout_hicc1` 这类写法变成哑音。
//!
//! # 已登记的规范冲突：`xfout_loccN` 的 Default
//!
//! `_data/sfz/syntax.yml` 里 `xfout_loccN` / `xfout_hiccN` 两行的 Default 写作 **0**，
//! 而同族的 `xfout_lokey` / `xfout_hikey` / `xfout_lovel` / `xfout_hivel` 四行写作 **127**。
//! 本 crate 对**整个 xfout 族**取 127（[`XfRange::FADE_OUT_DEFAULT`]），理由：
//!
//! 1. 语义上，`xfout_loccN` 单独出现时若取 0，区间就是 `[0, 0]`（不生效）或
//!    `[0, hicc]`（在 0 处最大、在 hicc 处归零）—— 前者让作者写的淡出**整个丢失**，
//!    后者与“淡出”的写法相反；
//! 2. 登记语料实测（1267 个可解析的 `.sfz`；探针口径 = `IncludeResolver` 展开后
//!    `parse_sources`，再逐**已归约**的 region 统计）：交叉淡化段共 **9259** 段，其中
//!    **4087** 段归约成「下界非 0、上界正好 127」，**4088** 段归约成
//!    「下界 0、上界非 0」，两端都不是中性值的只有 **556** 段 ⇒ 语料里的写法
//!    **绝大多数只在一个端点上偏离缺省**，那个缺省值直接决定该段是真淡化还是不生效；
//! 3. 参考实现 sfizz 的 `Defaults.cpp` 对 `xfoutLo` / `xfoutHi` 也取 127。
//!
//! 同一批读数还给出「不生效 ⇒ 1.0」这条的语料依据：**528** 段真的落在长度 0 的区间上
//! （形状全部是 `[0, 0]` 的淡入段，即只写了 `xfin_loccN=0`），它们必须退化成
//! 「不改变音量」而不是静音。
//!
//! 这条冲突**不改写规范**，只登记取舍；裁决留给集成者写进 `docs/adr/`
//! （本 crate 的改动面不含台账与 ADR）。
//!
//! # `power` 曲线的形状：规范只约束“等功率”，本 crate 取 `sqrt`
//!
//! `xf_cccurve` / `xf_velcurve` / `xf_keycurve` 三页对 `power` 的正文都是同一句
//! “Equal-power RMS crossfade ... a constant power level is kept during the crossfade”，
//! 规范**没有**给出具体式子。满足“两端增益的平方和为 1”的形状有无穷多条。本 crate 取
//!
//! ```text
//! 淡入：gain = sqrt(t)      淡出：gain = sqrt(1 - t)
//! ```
//!
//! 理由：① 与参考实现 sfizz（`crossfadeIn` / `crossfadeOut` 的 `CrossfadeCurve::power`
//! 分支）逐点一致；② `sqrt` 满足 `g_in² + g_out² = t + (1 - t) = 1`，正是“等功率”；
//! ③ **`sqrt` 是 IEEE-754 要求正确舍入的运算**，因此整条链路落在裁决 `ADR-0001` D32 的
//! **IEEE 精确类**（该条把 `sqrt` 与加减乘除并列），跨架构逐位相同；若改用
//! `sin` / `cos` 就会把它推进**超越函数类**（4096 ulp 预算 + 冻结架构才逐位相同），
//! 而规范并没有要求那条曲线。
//!
//! # 与 sfizz 的一处已知差异（登记，不追平）
//!
//! sfizz 在分母里减掉一个 `1/127`（`gapOffset`），目的是让整数值栅格上的最后一个
//! 整数步几乎到达满幅。本 crate 不做这个偏移：规范正文只说“下界处 0、上界处 1”，
//! `t = (value - low) / (high - low)` 是它最直接的读法。两者在 `[0, 127]` 上的增益差
//! 最大 **3.09e-5**（`value = 126` 处；`value = 64` 处：本 crate `0.709_885_2`，
//! sfizz 的 `126.992` 分母给 `0.709_907_2`）。本 crate 不承诺与任一具体播放器
//! bit-for-bit 兼容（见 `docs/ledger/sfz-core-notes.md` 第 9 节）。
//!
//! # 实时安全
//!
//! [`fade_in`] / [`fade_out`] / [`Crossfade::gain_at`] 只做整数减法、一次 `f32` 除法与
//! 最多一次 `sqrt`：**零堆分配、零锁、零阻塞 I/O、零日志**，可在逐样本路径调用。
//! 它们**不钳位**输入（调用方给 0..=127 的 7-bit 值），也**不 panic**：区间长度非正时
//! 提前返回 1.0，因此没有除零。

/// 交叉淡化曲线（`xf_keycurve` / `xf_velcurve` / `xf_cccurve`）。
///
/// 规范表格：Type = string，Default = `power`，Options = `gain, power`
/// （<https://sfzformat.com/opcodes/xf_cccurve/>）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XfCurve {
    /// `gain`：线性增益交叉淡化（规范正文：“Linear gain crossfade”）。
    Gain,
    /// `power`：等功率 RMS 交叉淡化（规范缺省；形状见模块文档）。
    Power,
}

impl XfCurve {
    /// 规范缺省值 `power`。
    pub const DEFAULT: Self = Self::Power;

    /// 白名单（`opcode=value` 的大小写不敏感匹配集合）。
    pub const OPTIONS: &'static [(&'static str, XfCurve)] =
        &[("gain", XfCurve::Gain), ("power", XfCurve::Power)];

    /// 用于错误信息的允许值列表。
    pub const ALLOWED: &'static str = "gain, power";

    /// 淡入方向在归一化位置 `t` 处的增益。
    fn fade_in_at(self, t: f32) -> f32 {
        match self {
            XfCurve::Gain => t,
            XfCurve::Power => t.sqrt(),
        }
    }

    /// 淡出方向在归一化位置 `t` 处的增益（`t` 是**同一**位置，不是它的补）。
    fn fade_out_at(self, t: f32) -> f32 {
        match self {
            XfCurve::Gain => 1.0 - t,
            XfCurve::Power => (1.0 - t).sqrt(),
        }
    }
}

/// 交叉淡化的驱动量。
///
/// 键盘位置与力度在 [`crate::RegionQuery`] 里恒有取值；CC 需要调用方提供
/// `RegionQuery::cc` 探针（见 [`crate::Region::crossfade_gain`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum XfAxis {
    /// 键盘位置（MIDI 音号 0..=127）。
    Key,
    /// 触发力度（0..=127）。
    Velocity,
    /// MIDI CC N（N ≤ 127；越界的 `N` 在解析期按未知 opcode 忽略）。
    Cc(u8),
}

/// 交叉淡化方向（`xfin_` = 淡入，`xfout_` = 淡出）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum XfDirection {
    /// `xfin_*`：从下界处的 0 升到上界处的 1。
    In,
    /// `xfout_*`：从下界处的 1 降到上界处的 0。
    Out,
}

impl XfDirection {
    /// 该方向的规范缺省区间（见模块文档“缺省端点”一节）。
    #[must_use]
    pub fn default_range(self) -> XfRange {
        match self {
            XfDirection::In => XfRange::FADE_IN_DEFAULT,
            XfDirection::Out => XfRange::FADE_OUT_DEFAULT,
        }
    }
}

/// 一段交叉淡化区间（闭区间，端点是 7-bit 的 MIDI 值 0..=127）。
///
/// `low` / `high` 是两个**独立的** opcode（例如 `xfin_lovel` 与 `xfin_hivel`），
/// 各自有自己的规范 Default；未给出的那一个由 [`XfDirection::default_range`] 补齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XfRange {
    /// 下界（含）。淡入在这里增益 0，淡出在这里增益 1。
    pub low: u8,
    /// 上界（含）。淡入在这里增益 1，淡出在这里增益 0。
    pub high: u8,
}

impl XfRange {
    /// `xfin_*` 一族的缺省区间：下界 0、上界 0（两个端点的规范 Default 都是 0）。
    pub const FADE_IN_DEFAULT: Self = Self { low: 0, high: 0 };

    /// `xfout_*` 一族的缺省区间：下界 127、上界 127
    /// （`xfout_lokey` / `xfout_hikey` / `xfout_lovel` / `xfout_hivel` 的 Default 都是 127；
    /// `xfout_loccN` / `xfout_hiccN` 两行的 0 是本 crate 明确不采的异常值，见模块文档）。
    pub const FADE_OUT_DEFAULT: Self = Self {
        low: 127,
        high: 127,
    };

    /// 该区间是否**生效**（长度为正）。
    ///
    /// `false` 时两个方向的增益都恒为 1.0（不改变音量）；这是本 crate 的工程裁决，
    /// 理由与 sfizz 的对照见模块文档。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.high > self.low
    }

    /// 归一化位置 `t`：`value <= low` ⇒ 0.0，`value >= high` ⇒ 1.0，其间线性。
    ///
    /// **只在 [`XfRange::is_active`] 为真时有定义**（否则分母为 0）；调用方
    /// [`fade_in`] / [`fade_out`] 已先做该判断，因此这里不会除零。
    fn position(&self, value: u8) -> f32 {
        if value <= self.low {
            return 0.0;
        }
        if value >= self.high {
            return 1.0;
        }
        f32::from(value - self.low) / f32::from(self.high - self.low)
    }
}

/// 淡入增益（`xfin_*`）：`value <= low` ⇒ 0.0，`value >= high` ⇒ 1.0，其间按 `curve`。
///
/// 区间不生效（`high <= low`）时返回 1.0。零分配、无锁、无 I/O；
/// 只用整数减法、一次浮点除法与（`power` 时）一次 `sqrt` ⇒ 裁决 ADR-0001 的
/// IEEE 精确类，跨架构逐位相同。
#[must_use]
pub fn fade_in(range: XfRange, value: u8, curve: XfCurve) -> f32 {
    if !range.is_active() {
        return 1.0;
    }
    curve.fade_in_at(range.position(value))
}

/// 淡出增益（`xfout_*`）：`value <= low` ⇒ 1.0，`value >= high` ⇒ 0.0，其间按 `curve`。
///
/// 区间不生效（`high <= low`）时返回 1.0。实时安全与确定性同 [`fade_in`]。
#[must_use]
pub fn fade_out(range: XfRange, value: u8, curve: XfCurve) -> f32 {
    if !range.is_active() {
        return 1.0;
    }
    curve.fade_out_at(range.position(value))
}

/// 一段落在某个 region 上的交叉淡化：驱动量 + 方向 + 区间 + 曲线。
///
/// 一个 region 可以有**多段**（多个 CC、以及 in/out 同时存在）；它们相乘，
/// 见 [`crate::Region::crossfade_gain`]。规范正文对同时使用 `xfin_*` 与 `xfout_*`
/// 的说明（`xfin_loccN` 页）：
///
/// > When there are multiple regions under the same note with `xfin_loccN`,
/// > `xfin_hiccN`, `xfout_loccN`, `xfout_hiccN` ... used to determine which regions are
/// > currently heard (and at what volume), all regions will be triggered - but some of
/// > them may play at zero volume, and therefore be inaudible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossfade {
    /// 驱动量。
    pub axis: XfAxis,
    /// 方向。
    pub direction: XfDirection,
    /// 区间（端点已按方向的规范 Default 补齐）。
    pub range: XfRange,
    /// 曲线（`xf_keycurve` / `xf_velcurve` / `xf_cccurve`，缺省
    /// [`XfCurve::DEFAULT`]）。
    pub curve: XfCurve,
}

impl Crossfade {
    /// 构造一段交叉淡化。
    #[must_use]
    pub fn new(axis: XfAxis, direction: XfDirection, range: XfRange, curve: XfCurve) -> Self {
        Self {
            axis,
            direction,
            range,
            curve,
        }
    }

    /// 该段在 `value` 处的增益（0.0 = 静音，1.0 = 满幅）。
    #[must_use]
    pub fn gain_at(&self, value: u8) -> f32 {
        match self.direction {
            XfDirection::In => fade_in(self.range, value, self.curve),
            XfDirection::Out => fade_out(self.range, value, self.curve),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 逐位比较（判确定性用）。
    fn bits(value: f32) -> u32 {
        value.to_bits()
    }

    #[test]
    fn the_two_default_endpoints_are_the_specification_table_values() {
        // `xfin_*` 两行 Default 都是 0；`xfout_*` 的 key / vel 四行 Default 都是 127。
        assert_eq!(XfRange::FADE_IN_DEFAULT, XfRange { low: 0, high: 0 });
        assert_eq!(
            XfRange::FADE_OUT_DEFAULT,
            XfRange {
                low: 127,
                high: 127
            }
        );
        assert_eq!(XfDirection::In.default_range(), XfRange::FADE_IN_DEFAULT);
        assert_eq!(XfDirection::Out.default_range(), XfRange::FADE_OUT_DEFAULT);
        // 两个缺省区间都不生效 ⇒ 只给无用那一端的写法不会改变音量。
        assert!(!XfRange::FADE_IN_DEFAULT.is_active());
        assert!(!XfRange::FADE_OUT_DEFAULT.is_active());
        assert_eq!(XfCurve::DEFAULT, XfCurve::Power);
        assert_eq!(XfCurve::ALLOWED, "gain, power");
    }

    #[test]
    fn fade_in_is_zero_at_the_low_end_and_one_at_the_high_end() {
        // 规范正文："zero for values lower than or equal to xfin_lovel, and maximum
        // ... for values greater than or equal to xfin_hivel"。
        let range = XfRange { low: 45, high: 65 };
        for curve in [XfCurve::Gain, XfCurve::Power] {
            assert_eq!(fade_in(range, 0, curve), 0.0);
            assert_eq!(fade_in(range, 45, curve), 0.0);
            assert_eq!(fade_in(range, 65, curve), 1.0);
            assert_eq!(fade_in(range, 127, curve), 1.0);
        }
    }

    #[test]
    fn fade_out_is_one_at_the_low_end_and_zero_at_the_high_end() {
        // 规范正文（xfout）："maximum ... for values ... lower than or equal to
        // xfout_loccN, and zero for values greater than or equal to xfout_hiccN"。
        let range = XfRange { low: 45, high: 65 };
        for curve in [XfCurve::Gain, XfCurve::Power] {
            assert_eq!(fade_out(range, 0, curve), 1.0);
            assert_eq!(fade_out(range, 45, curve), 1.0);
            assert_eq!(fade_out(range, 65, curve), 0.0);
            assert_eq!(fade_out(range, 127, curve), 0.0);
        }
    }

    #[test]
    fn gain_curve_is_linear_and_power_curve_is_the_square_root() {
        // `[0, 4]` 在 value = 1 处位置是 1/4（二进制精确）。
        let range = XfRange { low: 0, high: 4 };
        assert_eq!(fade_in(range, 1, XfCurve::Gain), 0.25);
        assert_eq!(fade_out(range, 1, XfCurve::Gain), 0.75);
        // sqrt(0.25) = 0.5 与 sqrt(0.75) 都是正确舍入的结果（IEEE 精确类）。
        assert_eq!(fade_in(range, 1, XfCurve::Power), 0.5);
        assert_eq!(
            bits(fade_out(range, 1, XfCurve::Power)),
            bits(0.75f32.sqrt())
        );
        // 规范正文对两档曲线的描述：“Linear gain crossfade” 与
        // “Equal-power RMS crossfade ... constant power level”。
        assert_eq!(fade_in(range, 2, XfCurve::Gain), 0.5);
        assert_eq!(fade_in(range, 2, XfCurve::Power), 0.5f32.sqrt());
    }

    #[test]
    fn the_power_curve_keeps_a_constant_power_level() {
        // 等功率的定义：淡入与淡出在**同一**位置上的增益平方和为 1
        // （g_in² + g_out² = t + (1 - t) = 1）。
        let range = XfRange { low: 0, high: 8 };
        for value in 0u8..=8 {
            let sum = fade_in(range, value, XfCurve::Power).powi(2)
                + fade_out(range, value, XfCurve::Power).powi(2);
            assert!(
                (sum - 1.0).abs() <= 1.0e-6,
                "value {value} gave {sum}, expected 1.0"
            );
        }
        // 线性档**不**保持等功率：中点的平方和是 0.5。
        let sum =
            fade_in(range, 4, XfCurve::Gain).powi(2) + fade_out(range, 4, XfCurve::Gain).powi(2);
        assert_eq!(sum, 0.5);
    }

    #[test]
    fn an_inactive_range_never_changes_the_volume() {
        // 长度 0 与“倒置”的区间都恒返回 1.0（工程裁决 + sfizz 的 length <= 0 分支）。
        // R93：把夹具绑成具名数组并**先钉住长度** —— 内联字面量虽然非空，
        // 但显式下界让「夹具被清空」这件事变成红，而不是静默真空通过。
        let ranges = [
            XfRange { low: 0, high: 0 },
            XfRange {
                low: 127,
                high: 127,
            },
            XfRange { low: 100, high: 20 },
        ];
        assert_eq!(ranges.len(), 3, "the fixture list must not shrink (R93)");
        for range in ranges {
            assert!(!range.is_active(), "{range:?} must be inactive");
            for value in [0u8, 1, 63, 100, 126, 127] {
                for curve in [XfCurve::Gain, XfCurve::Power] {
                    assert_eq!(fade_in(range, value, curve), 1.0);
                    assert_eq!(fade_out(range, value, curve), 1.0);
                }
            }
        }
    }

    #[test]
    fn every_gain_is_finite_and_within_the_unit_interval() {
        // 叶子 crate 红线：任意已解析输入都不得 panic，也不得产生 NaN / inf。
        for low in [0u8, 1, 45, 126, 127] {
            for high in [0u8, 1, 45, 126, 127] {
                let range = XfRange { low, high };
                for value in 0u8..=127 {
                    for curve in [XfCurve::Gain, XfCurve::Power] {
                        for gain in [fade_in(range, value, curve), fade_out(range, value, curve)] {
                            assert!(
                                gain.is_finite() && (0.0..=1.0).contains(&gain),
                                "range {range:?} value {value} gave {gain}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_gain_is_monotone_in_the_driving_value() {
        let range = XfRange { low: 30, high: 100 };
        for curve in [XfCurve::Gain, XfCurve::Power] {
            let mut previous = -1.0f32;
            for value in 0u8..=127 {
                let gain = fade_in(range, value, curve);
                assert!(gain >= previous, "fade_in went backwards at {value}");
                previous = gain;
            }
            let mut previous = 2.0f32;
            for value in 0u8..=127 {
                let gain = fade_out(range, value, curve);
                assert!(gain <= previous, "fade_out went forwards at {value}");
                previous = gain;
            }
        }
    }

    #[test]
    fn the_same_inputs_always_give_the_same_bits() {
        // ARCH-DET-001：同一输入 ⇒ 同一结果。整条链路只用 IEEE 正确舍入的运算，
        // 因此这里的逐位相等同时在同架构上成立（跨架构见模块文档的 ADR-0001 引用）。
        for low in [0u8, 7, 64] {
            for high in [1u8, 64, 127] {
                let range = XfRange { low, high };
                for value in 0u8..=127 {
                    for curve in [XfCurve::Gain, XfCurve::Power] {
                        // ⚠️ **自比（两侧同一个表达式）**：本条只抓**非确定性**，
                        // ⛔ 不是契约（R70②／R75）。字面契约在同文件的
                        // `crossfade_gain_curve_is_pinned`（`0.5f32.sqrt()` 等）。
                        assert_eq!(
                            bits(fade_in(range, value, curve)),
                            bits(fade_in(range, value, curve))
                        );
                        // ⚠️ 同上的**自比**（只证明确定性）。
                        assert_eq!(
                            bits(fade_out(range, value, curve)),
                            bits(fade_out(range, value, curve))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_crossfade_dispatches_on_its_direction() {
        let range = XfRange { low: 0, high: 4 };
        let fade_up = Crossfade::new(XfAxis::Velocity, XfDirection::In, range, XfCurve::Gain);
        let fade_down = Crossfade::new(XfAxis::Cc(1), XfDirection::Out, range, XfCurve::Gain);
        assert_eq!(fade_up.gain_at(1), 0.25);
        assert_eq!(fade_down.gain_at(1), 0.75);
        assert_eq!(fade_up.axis, XfAxis::Velocity);
        assert_eq!(fade_down.axis, XfAxis::Cc(1));
        // ⚠️ 自反性（恒真，⛔ 不是证据）；真正的牙是下一行的 `assert_ne!`（R58／R69）。
        assert_eq!(fade_up, fade_up);
        assert_ne!(fade_up, fade_down);
    }

    #[test]
    fn the_single_endpoint_writings_of_the_corpus_are_real_fades() {
        // 语料里的两种主导写法（见模块文档）：只给 `xfin_hiccN` 与只给 `xfout_loccN`。
        let fade_in_range = XfRange {
            low: XfRange::FADE_IN_DEFAULT.low,
            high: 100,
        };
        assert!(fade_in_range.is_active());
        assert_eq!(fade_in(fade_in_range, 0, XfCurve::Power), 0.0);
        assert_eq!(fade_in(fade_in_range, 100, XfCurve::Power), 1.0);
        let fade_out_range = XfRange {
            low: 64,
            high: XfRange::FADE_OUT_DEFAULT.high,
        };
        assert!(fade_out_range.is_active());
        assert_eq!(fade_out(fade_out_range, 64, XfCurve::Power), 1.0);
        assert_eq!(fade_out(fade_out_range, 127, XfCurve::Power), 0.0);
    }

    #[test]
    fn position_is_pinned_on_a_non_zero_low_range() {
        // 上面几条判据只读 `value <= low` 与 `value >= high` 这两个早退分支；
        // `gain_curve_is_linear_and_power_curve_is_the_square_root` 用的区间是 `[0, 4]`，
        // 那里 `low == 0`，于是分母 `(high - low)` 与 `high` 数值相同 —— 抓不到
        // 「分母漏掉 low 偏移」的改写。这里用一个 low 非零的区间把插值本身钉在字面上。
        // 链路只有整数→f32 转换、一次减法、一次除法与（`power` 档）一次 `sqrt`，
        // 全是 IEEE 正确舍入的运算 ⇒ 裁决 ADR-0001 的 IEEE 精确类，跨架构逐位相同。
        let range = XfRange { low: 64, high: 127 };
        // (65 - 64) / (127 - 64) = 1/63：与 `curve::tests` 里的 1/63 同一位型。
        assert_eq!(bits(fade_in(range, 65, XfCurve::Gain)), 0x3c82_0821);
        // (100 - 64) / 63 = 36/63。
        assert_eq!(bits(fade_in(range, 100, XfCurve::Gain)), 0x3f12_4925);
        assert_eq!(bits(fade_out(range, 100, XfCurve::Gain)), 0x3edb_6db6);
        assert_eq!(bits(fade_in(range, 100, XfCurve::Power)), 0x3f41_848f);
        // 非空证明：漏掉 low 偏移的分母在这一点上给出的是 36/127，位型不同。
        assert_ne!(
            bits(fade_in(range, 100, XfCurve::Gain)),
            bits(36.0f32 / 127.0)
        );
        assert_eq!(fade_in(range, 100, XfCurve::Gain), 36.0f32 / 63.0);
    }

    #[test]
    fn the_ledger_divergence_from_sfizz_stays_within_a_ten_thousandth() {
        // 与 sfizz 的 gapOffset 差异（见模块文档）：`[0, 127]` 上的最大差 3.09e-5
        // （在 value = 126 处），这里对全区间钉一个 1e-4 的上界。
        let range = XfRange { low: 0, high: 127 };
        let mut worst = 0.0f32;
        for value in 0u8..=127 {
            let ours = fade_in(range, value, XfCurve::Power);
            let sfizz_position = f32::from(value) / (127.0 - 1.0 / 127.0);
            let sfizz = sfizz_position.sqrt().min(1.0);
            worst = worst.max((ours - sfizz).abs());
        }
        assert!(worst <= 1.0e-4, "worst divergence was {worst}");
        assert!(worst >= 1.0e-5, "the divergence vanished: {worst}");
        // 差异是登记的，不是“通过”：本判据只钉住量级，不钉住相等。
        let at_64 = fade_in(range, 64, XfCurve::Power);
        let sfizz_at_64 = (64.0f32 / (127.0 - 1.0 / 127.0)).sqrt();
        assert_ne!(bits(at_64), bits(sfizz_at_64));
        // 本 crate 的读数（与模块文档的两个字面值一致）。
        assert!((at_64 - 0.709_885_2).abs() <= 1.0e-7, "{at_64}");
    }
}
