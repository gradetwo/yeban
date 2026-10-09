//! SFZ `<curve>` 头：MIDI CC 调制曲线表。
//!
//! 规范出处 <https://sfzformat.com/headers/curve/>，原文：
//!
//! > A header for defining curves for MIDI CC controls.
//! > One curve header is used to define each curve. The values for various points along
//! > the curve can then be set, from `v000` to `v127`. The default is `v000=0` and `v127=1`.
//! > Any points along the curve not defined explicitly will be interpolated linearly
//! > between points which are defined.
//!
//! 同一页给出的 ARIA 内建曲线表：
//!
//! | Number | Description | Range | Notes |
//! | ---: | :--- | :--- | :--- |
//! | 0 | Default | 0 to 1 | Linear |
//! | 1 | Bipolar | -1 to 1 | Linear, used by CC10 panning by defeault |
//! | 2 | Inverted | 1 to 0 | Linear |
//! | 3 | Bipolar inverted | 1 to -1 | Linear |
//! | 4 | Concave | 0 to 1 | Nonlinear, used for CC7 volume tracking and amp_veltrack |
//! | 5 | Xfin power curve | 0 to 1 | Based on Dimension Pro behavior |
//! | 6 | Xfout power curve | 1 to 0 | Based on Dimension Pro behavior |
//!
//! > These cannot be overwritten. Use `curve_index` numbers of 7 and above for custom
//! > curves. Curve_index in ARIA can be any integer from 0 to 254.
//!
//! 本模块只实现规范**给出公式**的那一部分：内建曲线 0..=3 的规范描述是「从 A 到 B 的
//! 线性」，因此由两个端点逐位确定（[`Curve::built_in`]）。4..=6 被规范明文称为
//! `Nonlinear` 却**没有给出任何公式**，本 crate 不发明公式：它们返回 `None`。
//! 该缺口与 `<sample>` 段头一起登记在
//! `docs/ledger/sfz-core-notes.md`（该文件由集成者独占）。
//!
//! ## 契约
//!
//! - **求值**：[`Curve::value_at`] 在**已定义点之间线性插值**（规范原文），区间外取端点值，
//!   `NaN` 输入原样传出（`NaN` 绝不「修成 0」）。已定义点上逐位命中该点的值。
//! - **取值不钳位**：规范允许负值（内建 `Bipolar` 是 `-1..=1`），本 crate 原样保留。
//! - **实时安全**：求值是纯算术 + 切片索引，无分配、无锁、无 I/O、无日志，可在实时路径调用。
//! - **逐位可复现**：只用 IEEE 754 精确的 `+ - * /` 与比较，**不含超越函数**，
//!   因此没有 `4096 ulp` 预算问题（裁决 ADR-0001 的 IEEE 精确类）。

use std::borrow::Cow;

/// 曲线上的一个显式点：`vNNN` 里的 `NNN` 是 [`CurvePoint::at`]，取值是 [`CurvePoint::value`]。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    /// 横坐标（`0..=127`）：`<curve>` 的 `vNNN` 里的 `NNN`，
    /// 或 `amp_velcurve_N` 里的 `N`（见 [`crate::velocity::VelocityCurve`]）。
    pub at: u8,
    /// 该点的取值。规范允许负值；本 crate 不钳位。
    pub value: f32,
}

/// 内建曲线 0：`Default`，`0 to 1`，`Linear`。
static BUILT_IN_0: [CurvePoint; 2] = [
    CurvePoint { at: 0, value: 0.0 },
    CurvePoint {
        at: 127,
        value: 1.0,
    },
];

/// 内建曲线 1：`Bipolar`，`-1 to 1`，`Linear`（ARIA 下 CC10 声像的缺省曲线）。
static BUILT_IN_1: [CurvePoint; 2] = [
    CurvePoint { at: 0, value: -1.0 },
    CurvePoint {
        at: 127,
        value: 1.0,
    },
];

/// 内建曲线 2：`Inverted`，`1 to 0`，`Linear`。
static BUILT_IN_2: [CurvePoint; 2] = [
    CurvePoint { at: 0, value: 1.0 },
    CurvePoint {
        at: 127,
        value: 0.0,
    },
];

/// 内建曲线 3：`Bipolar inverted`，`1 to -1`，`Linear`。
static BUILT_IN_3: [CurvePoint; 2] = [
    CurvePoint { at: 0, value: 1.0 },
    CurvePoint {
        at: 127,
        value: -1.0,
    },
];

/// 内建曲线（规范保证不可覆写）的**最大**编号：`0..=6` 都是内建。
pub const MAX_BUILT_IN_CURVE_INDEX: u8 = 6;

/// ARIA 允许的 `curve_index` 上界（规范原文：`from 0 to 254`）。
pub const MAX_CURVE_INDEX: u8 = 254;

/// 一条 `<curve>` 头定义的曲线（或一条内建曲线）。
///
/// 点表由构造方保证：按 [`CurvePoint::at`] 升序、首点 `at == 0`、末点 `at == 127`、
/// 无重复 `at`（重复点在同一段里按「后者胜」合并，见解析器）。因此
/// [`Curve::value_at`] 永远不需要外推，也永远不做除零。
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    index: u8,
    points: Cow<'static, [CurvePoint]>,
}

impl Curve {
    /// 用文件里读到的点表构造一条曲线（解析器内部使用）。
    ///
    /// 调用方必须先补上规范缺省 `v000=0` / `v127=1` 并按 `at` 排序。
    pub(crate) fn from_points(index: u8, points: Vec<CurvePoint>) -> Self {
        Self {
            index,
            points: Cow::Owned(points),
        }
    }

    /// 取一条 ARIA 内建曲线：`0..=3` 有规范公式（线性 A→B），`4..=6` 是 `Nonlinear`
    /// 但规范未给公式 ⇒ `None`（不发明），`7..=254` 不是内建 ⇒ `None`。
    ///
    /// 零分配：点表是 `'static` 的。
    #[must_use]
    pub fn built_in(index: u8) -> Option<Self> {
        let points: &'static [CurvePoint] = match index {
            0 => &BUILT_IN_0,
            1 => &BUILT_IN_1,
            2 => &BUILT_IN_2,
            3 => &BUILT_IN_3,
            _ => return None,
        };
        Some(Self {
            index,
            points: Cow::Borrowed(points),
        })
    }

    /// 曲线编号（规范 `curve_index`，`0..=254`）。
    #[must_use]
    pub fn index(&self) -> u8 {
        self.index
    }

    /// 全部显式点，按 `at` 升序；首点 `at == 0`、末点 `at == 127`（规范缺省已补齐）。
    #[must_use]
    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// 求 `x` 处的曲线值。
    ///
    /// 规则（规范原文见模块文档）：
    /// 1. 已定义点上**逐位命中**该点的值；
    /// 2. 已定义点之间**线性插值**；
    /// 3. `x` 在 `0..=127` 之外取端点值（点表已含 `at == 0` 与 `at == 127`，所以不外推）；
    /// 4. `x` 是 `NaN` ⇒ 返回 `NaN`。
    ///
    /// 实时安全：无分配、无锁、无 I/O。逐位可复现：只有 `+ - * /` 与比较。
    #[must_use]
    pub fn value_at(&self, x: f32) -> f32 {
        if x.is_nan() {
            return f32::NAN;
        }
        let (Some(first), Some(last)) = (self.points.first(), self.points.last()) else {
            // 构造方保证至少两个点；空表只可能来自未来改动，这里明确退化而不是 panic。
            return 0.0;
        };
        if x <= f32::from(first.at) {
            return first.value;
        }
        if x >= f32::from(last.at) {
            return last.value;
        }
        let upper = self
            .points
            .partition_point(|point| f32::from(point.at) <= x);
        if upper == 0 || upper >= self.points.len() {
            return last.value;
        }
        let low = self.points[upper - 1];
        let high = self.points[upper];
        let t = (x - f32::from(low.at)) / f32::from(high.at - low.at);
        low.value + t * (high.value - low.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(index: u8, points: &[(u8, f32)]) -> Curve {
        Curve::from_points(
            index,
            points
                .iter()
                .map(|&(at, value)| CurvePoint { at, value })
                .collect(),
        )
    }

    #[test]
    fn built_in_linear_curves_match_the_specification_table() {
        // 出处 <https://sfzformat.com/headers/curve/> 的内建曲线表：
        // 0 Default 0→1 / 1 Bipolar -1→1 / 2 Inverted 1→0 / 3 Bipolar inverted 1→-1。
        let expected: [(u8, f32, f32); 4] =
            [(0, 0.0, 1.0), (1, -1.0, 1.0), (2, 1.0, 0.0), (3, 1.0, -1.0)];
        for (index, start, end) in expected {
            let curve = Curve::built_in(index).expect("0..=3 are built in");
            assert_eq!(curve.index(), index);
            assert_eq!(curve.points().len(), 2, "curve {index}");
            assert_eq!(curve.value_at(0.0), start, "curve {index} at v000");
            assert_eq!(curve.value_at(127.0), end, "curve {index} at v127");
            assert_eq!(
                curve.value_at(63.5),
                (start + end) / 2.0,
                "curve {index} midpoint must be the linear midpoint"
            );
        }
        // 4..=6 是 `Nonlinear` 且规范未给公式；7 以上不是内建 ⇒ 一律 None（不发明公式）。
        for index in [4u8, 5, 6, 7, 40, 254] {
            assert!(
                Curve::built_in(index).is_none(),
                "{index} has no normative formula"
            );
        }
    }

    #[test]
    fn built_in_bipolar_crosses_zero_exactly_at_the_midpoint() {
        let bipolar = Curve::built_in(1).expect("built in");
        assert_eq!(bipolar.value_at(63.5).to_bits(), 0.0f32.to_bits());
        assert!(bipolar.value_at(63.0) < 0.0);
        assert!(bipolar.value_at(64.0) > 0.0);
    }

    #[test]
    fn defined_points_are_hit_bit_exactly() {
        // v000=0 v063=1 v127=0 —— 规范缺省已补齐，因此 0..=127 全程有点可插值。
        let curve = curve(7, &[(0, 0.0), (63, 1.0), (127, 0.0)]);
        for (at, value) in [(0u8, 0.0f32), (63, 1.0), (127, 0.0)] {
            assert_eq!(
                curve.value_at(f32::from(at)).to_bits(),
                value.to_bits(),
                "v{at:03} must be hit bit exactly"
            );
        }
        assert_eq!(curve.value_at(31.5), 0.5, "linear midpoint of 0 -> 63");
        assert_eq!(curve.value_at(95.0), 0.5, "linear midpoint of 63 -> 127");
    }

    #[test]
    fn interpolation_is_ieee_exact_class() {
        // 逐位字面量：`1/63` 的 binary32 位型（只有 `+ - * /`，无超越函数 ⇒ 跨架构逐位相同）。
        let curve = curve(7, &[(0, 0.0), (63, 1.0), (127, 0.0)]);
        assert_eq!(curve.value_at(1.0).to_bits(), 0x3c82_0821);
        assert_eq!(1.0f32 / 63.0, curve.value_at(1.0));
    }

    #[test]
    fn values_outside_the_domain_take_the_endpoint() {
        let curve = curve(8, &[(0, -1.0), (127, 1.0)]);
        assert_eq!(curve.value_at(-100.0), -1.0);
        assert_eq!(curve.value_at(-0.0), -1.0);
        assert_eq!(curve.value_at(1000.0), 1.0);
        assert_eq!(curve.value_at(f32::NEG_INFINITY), -1.0);
        assert_eq!(curve.value_at(f32::INFINITY), 1.0);
    }

    #[test]
    fn nan_propagates_and_negative_values_are_not_clamped() {
        let curve = curve(9, &[(0, -2.5), (127, 3.5)]);
        assert!(curve.value_at(f32::NAN).is_nan());
        assert_eq!(curve.value_at(0.0), -2.5);
        assert_eq!(curve.value_at(127.0), 3.5);
    }

    #[test]
    fn a_single_defined_point_still_interpolates_against_the_defaults() {
        // 只有 v064=0.5：规范缺省把 v000=0 / v127=1 补上，于是得到 0 → 0.5 → 1。
        let curve = curve(10, &[(0, 0.0), (64, 0.5), (127, 1.0)]);
        assert_eq!(curve.value_at(64.0), 0.5);
        assert_eq!(curve.value_at(32.0), 0.25);
        assert_eq!(curve.value_at(95.5), 0.75);
    }
}
