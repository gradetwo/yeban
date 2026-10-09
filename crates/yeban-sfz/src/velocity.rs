//! SFZ 力度 → 振幅映射：`amp_veltrack` 与 `amp_velcurve_N`。
//!
//! 规范出处（全部取自 <https://sfzformat.com/opcodes/>）：
//!
//! - `amp_veltrack`：<https://sfzformat.com/opcodes/amp_veltrack/>。Type = float，
//!   Default = 100，Range = -100 to 100，Unit = %。同页正文给出**唯一一条公式**：
//!
//!   > With `amp_veltrack` at the default value of 100, volume is modified by the amount
//!   > calculated by the following expression, based on incoming velocity.
//!   > `Gain(v) = 20 * log10[(v/127)^2] dB`
//!
//! - `amp_velcurve_N`：<https://sfzformat.com/opcodes/amp_velcurve_N/>。Type = float，
//!   Range = 0 to 1，Unit = N/A。N 是 MIDI 力度（同页正文："N can be from 0 to 127"）。
//!   同页正文给出三条规则：
//!
//!   1. > The value of the opcode indicates the normalized amplitude (0 to 1) for the
//!      > specified velocity.
//!   2. > The player will interpolate lineraly between specified opcodes for
//!      > unspecified ones —— 同页给出的算例是
//!      > `amp_velcurve_1=0.2 amp_velcurve_3=0.3` ⇒ `amp_velcurve_2` 是 `0.25`。
//!   3. > If `amp_velcurve_127` is not specified, the player will assign it the value of 1.
//!
//!   同页正文另有："By default, `amp_velcurve_0` is effectively 0."
//!
//! # 缺省端点的口径
//!
//! [`VelocityCurve::from_points`] 要求调用方给出**显式**点；它自己补上缺省端点
//! `0 → 0.0` 与 `127 → 1.0`（上面第 3 条 + Practical Considerations 一句）。因此
//! [`VelocityCurve::amplitude`] 的点表恒含横坐标 0 与 127，`u8` 域内的任何力度都
//! **不需要外推**、也不会做除零。
//!
//! # `amp_veltrack` 的非缺省取值：本 crate 的工程裁决
//!
//! 格式页只给出 `amp_veltrack = 100` 这一点的公式。本 crate 需要一条对
//! `-100..=100` 全定义域**总**（total）的律，因此取 dB 域的百分比插值：
//!
//! ```text
//! gain_db(v) = (amp_veltrack / 100) * 40 * log10(v / 127)
//! ```
//!
//! 它等价于线性域的一条幂律（两者是同一个式子）：
//!
//! ```text
//! gain(v) = (v / 127) ^ (2 * amp_veltrack / 100)
//! ```
//!
//! **两个锚点都不是发明**：`amp_veltrack = 100` 时它就是上面那条规范公式
//! （`20*log10[(v/127)^2] = 40*log10(v/127)`，逐位相同）；`amp_veltrack = 0` 时它是
//! 恒等（`x^0 = 1`，力度不改变振幅），这正是格式页与教程
//! <https://sfzformat.com/tutorials/volume/> 对 0 的描述：
//!
//! > Remember that `amp_veltrack` is 100 by default, so if dynamics are to be controlled
//! > by things other than velocity ... then set `amp_veltrack` to 0
//!
//! 中间取值与负值没有规范公式，是**登记在案的待裁决项**（见
//! `docs/ledger/sfz-core-notes.md`，该文件由集成者独占）。同页正文对 `-100` 的
//! 散文描述（"which would make velocity 127 notes silent, and low-velocity notes loud"）
//! 与本公式不一致：公式在 `v = 127` 处恒为 1（不是静音）。本 crate 取**公式**
//! （唯一被量化的陈述），并把这条冲突原样登记。
//!
//! # 实时安全
//!
//! [`VelocityCurve::amplitude`] 与 [`veltrack_gain`] 只做标量算术与切片索引：
//! 无堆分配、无锁、无 I/O、无日志，可在实时路径逐样本调用。两者都**不钳位**
//! （与 [`crate::playback`] 的 `linear_gain` 同一条口径：不发明钳制策略）。
//! 唯一的例外是 [`veltrack_gain`] 在力度 0 处返回 `0.0`：MIDI 力度 0 是 note-off
//! （格式页原文："As a MIDI velocity 0 note is a note-off message"），而负
//! `amp_veltrack` 的幂律在 `v = 0` 处会溢出成 `+inf` —— 解析路径可以合法地得到
//! `amp_veltrack = -100`，因此这里必须给出一个有限值而不是 `inf` / `NaN`。

use crate::curve::CurvePoint;

/// `amp_veltrack` 的规范缺省值（%）。
///
/// 出处：<https://sfzformat.com/opcodes/amp_veltrack/> 的表格行
/// （Type = float，Default = 100，Range = -100 to 100，Unit = %）。
pub const AMP_VELTRACK_DEFAULT: f32 = 100.0;

/// `amp_veltrack` 的规范下界（%）。
pub const AMP_VELTRACK_MIN: f32 = -100.0;

/// `amp_veltrack` 的规范上界（%）。
pub const AMP_VELTRACK_MAX: f32 = 100.0;

/// `amp_velcurve_N` 的 `N` 下界（含）：规范正文写 "N can be from 0 to 127"。
pub const MIN_VELCURVE_INDEX: u8 = 0;

/// `amp_velcurve_N` 的 `N` 上界（含）。
pub const MAX_VELCURVE_INDEX: u8 = 127;

/// `amp_velcurve_N` 取值的下界（含）：规范表格 Range = `0 to 1`。
pub const MIN_VELCURVE_AMPLITUDE: f32 = 0.0;

/// `amp_velcurve_N` 取值的上界（含）：规范表格 Range = `0 to 1`。
pub const MAX_VELCURVE_AMPLITUDE: f32 = 1.0;

/// 一条 `amp_velcurve_N` 力度曲线。
///
/// 点表的横坐标是 MIDI 力度（`0..=127`），纵坐标是归一化振幅（`0..=1`）。
/// 复用 [`CurvePoint`]（同一个 `0..=127` 的整数横坐标 + `f32` 纵坐标形状）；
/// 与 `<curve>` 的区别是横坐标的语义：那里是 CC 值，这里是力度。
///
/// 构造方（[`VelocityCurve::from_points`]）保证：按 `at` 升序、无重复 `at`、
/// 首点 `at == 0`、末点 `at == 127`。因此 [`VelocityCurve::amplitude`] 对任何
/// `u8` 力度都落在点表闭区间内，不做外推。
#[derive(Debug, Clone, PartialEq)]
pub struct VelocityCurve {
    points: Vec<CurvePoint>,
}

impl VelocityCurve {
    /// 用文件里读到的**显式**点构造一条力度曲线（解析器内部使用）。
    ///
    /// 本函数补上规范缺省端点并排序去重：
    /// `amp_velcurve_0` 缺省 `0.0`、`amp_velcurve_127` 缺省 `1.0`（同一横坐标
    /// 重复给出时**后者胜**，与作用域归约同一条口径）。因此结果恒含
    /// `at == 0` 与 `at == 127` 两个端点。
    pub(crate) fn from_points(points: impl IntoIterator<Item = CurvePoint>) -> Self {
        let mut sorted: Vec<CurvePoint> = points.into_iter().collect();
        // 稳定排序：同一 `at` 的多个点里，**最后**出现的那一个胜（作用域优先级由
        // 调用方按 global → master → group → region 的顺序喂进来）。
        sorted.sort_by_key(|point| point.at);
        let mut points: Vec<CurvePoint> = Vec::with_capacity(sorted.len());
        for point in sorted {
            match points.last_mut() {
                Some(last) if last.at == point.at => *last = point,
                _ => points.push(point),
            }
        }
        if points.first().map(|point| point.at) != Some(0) {
            points.insert(0, CurvePoint { at: 0, value: 0.0 });
        }
        if points.last().map(|point| point.at) != Some(127) {
            points.push(CurvePoint {
                at: 127,
                value: 1.0,
            });
        }
        Self { points }
    }

    /// 全部点（含补齐的缺省端点），按 `at` 升序；首点 `at == 0`、末点 `at == 127`。
    #[must_use]
    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// 求 `velocity` 处的归一化振幅。
    ///
    /// 规则（规范原文见模块文档）：
    /// 1. 已定义力度上**逐位命中**该点的值；
    /// 2. 已定义点之间**线性插值**；
    /// 3. 点表恒含 `at == 0` 与 `at == 127`，所以不外推。
    ///
    /// 实时安全：无分配、无锁、无 I/O。逐位可复现：只用 `+ - * /` 与比较，
    /// **不含超越函数**（裁决 ADR-0001 的 IEEE 精确类）。
    #[must_use]
    pub fn amplitude(&self, velocity: u8) -> f32 {
        let (Some(first), Some(last)) = (self.points.first(), self.points.last()) else {
            // 构造方保证至少两个端点；空表只可能来自未来改动，这里明确退化而不是 panic。
            return 0.0;
        };
        if velocity <= first.at {
            return first.value;
        }
        if velocity >= last.at {
            return last.value;
        }
        let upper = self.points.partition_point(|point| point.at <= velocity);
        if upper == 0 || upper >= self.points.len() {
            return last.value;
        }
        let low = self.points[upper - 1];
        let high = self.points[upper];
        // 构造方保证 `low.at < high.at`；`saturating_sub` 只是「任意输入不 panic」的
        // 兜底（真出现 0 时结果是 IEEE 的 `NaN` / `inf`，不是崩溃）。
        let span = high.at.saturating_sub(low.at);
        let t = f32::from(velocity.saturating_sub(low.at)) / f32::from(span);
        low.value + t * (high.value - low.value)
    }
}

/// `amp_veltrack` 的力度 → 线性振幅律（工程裁决，见模块文档）。
///
/// ```text
/// gain(v) = (v / 127) ^ (2 * amp_veltrack / 100)
/// ```
///
/// `amp_veltrack = 100`（规范缺省）时等价于规范公式
/// `20 * log10[(v/127)^2] dB`；`0` 时等价于恒等（力度不改变振幅）。
///
/// 力度 0 返回 `0.0`（MIDI 力度 0 是 note-off），因此返回值恒为**有限**值，
/// 即使 `amp_veltrack` 是负数（负幂律在 0 处会溢出成 `+inf`）。
/// `amp_veltrack` 取 `NaN` / `±inf` 时结果由 IEEE 语义决定（本 crate 不钳位）。
///
/// 实时安全：无分配、无锁、无 I/O。含一个超越函数（`powf`），因此
/// 跨架构逐位一致性（ARCH-DET-002）与本 crate 的 `StealFade::gain_at` 登记在
/// 同一条 pending 上；`4096 ulp` 预算见裁决 ADR-0001 的超越函数类。
#[must_use]
pub fn veltrack_gain(velocity: u8, amp_veltrack: f32) -> f32 {
    if velocity == 0 {
        return 0.0;
    }
    let ratio = f64::from(velocity) / 127.0;
    let exponent = 2.0 * f64::from(amp_veltrack) / 100.0;
    ratio.powf(exponent) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(at: u8, value: f32) -> CurvePoint {
        CurvePoint { at, value }
    }

    fn close(left: f32, right: f32, tolerance: f32) -> bool {
        (left - right).abs() <= tolerance
    }

    #[test]
    fn default_endpoints_are_added_when_not_specified() {
        // 规范：`amp_velcurve_0` 实际缺省 0；`amp_velcurve_127` 未给出时被赋 1。
        let curve = VelocityCurve::from_points([point(64, 0.5)]);
        assert_eq!(curve.points().first(), Some(&point(0, 0.0)));
        assert_eq!(curve.points().last(), Some(&point(127, 1.0)));
        assert_eq!(curve.amplitude(0), 0.0);
        assert_eq!(curve.amplitude(127), 1.0);
        assert_eq!(curve.amplitude(64), 0.5);
    }

    #[test]
    fn explicit_endpoints_win_over_the_defaults() {
        let curve = VelocityCurve::from_points([point(0, 0.25), point(127, 0.75)]);
        assert_eq!(curve.amplitude(0), 0.25);
        assert_eq!(curve.amplitude(127), 0.75);
    }

    #[test]
    fn interpolation_matches_the_specification_worked_example() {
        // 规范原文算例：`amp_velcurve_1=0.2 amp_velcurve_3=0.3` ⇒ `amp_velcurve_2` 是 0.25。
        let curve = VelocityCurve::from_points([point(1, 0.2), point(3, 0.3)]);
        assert!(close(curve.amplitude(2), 0.25, 1.0e-6));
        assert_eq!(curve.amplitude(1), 0.2);
        assert_eq!(curve.amplitude(3), 0.3);
    }

    #[test]
    fn amplitude_is_monotone_between_two_adjacent_points() {
        let curve = VelocityCurve::from_points([point(10, 0.0), point(20, 1.0)]);
        let mut previous = f32::NEG_INFINITY;
        for velocity in 10..=20 {
            let value = curve.amplitude(velocity);
            assert!(value >= previous, "velocity {velocity} went backwards");
            previous = value;
        }
    }

    #[test]
    fn duplicate_indices_keep_the_last_point_and_are_sorted() {
        // 同一横坐标重复：后者胜（作用域优先级由调用方按外层 → 内层喂入）。
        let curve = VelocityCurve::from_points([point(64, 0.1), point(32, 0.9), point(64, 0.7)]);
        assert_eq!(curve.amplitude(32), 0.9);
        assert_eq!(curve.amplitude(64), 0.7);
        let ats: Vec<u8> = curve.points().iter().map(|p| p.at).collect();
        assert_eq!(ats, vec![0, 32, 64, 127]);
    }

    #[test]
    fn veltrack_at_the_specification_default_is_the_literal_formula() {
        // 规范公式：Gain(v) = 20*log10[(v/127)^2] dB ⇒ 线性 (v/127)^2。
        for velocity in [1u8, 32, 64, 100, 127] {
            let expected = (f64::from(velocity) / 127.0).powi(2) as f32;
            assert!(
                close(
                    veltrack_gain(velocity, AMP_VELTRACK_DEFAULT),
                    expected,
                    1.0e-6
                ),
                "velocity {velocity}"
            );
        }
        assert!(close(
            veltrack_gain(64, AMP_VELTRACK_DEFAULT),
            4096.0 / 16129.0,
            1.0e-6
        ));
    }

    #[test]
    fn veltrack_zero_is_the_identity_above_velocity_zero() {
        // 教程原文：dynamics 由别的东西控制时 `amp_veltrack` 设 0 ⇒ 力度不改变振幅。
        // 力度 0 是 note-off，见 `veltrack_gain` 的文档（恒返回 0.0，不是 1.0）。
        assert_eq!(veltrack_gain(0, 0.0), 0.0);
        for velocity in 1u8..=127 {
            assert_eq!(veltrack_gain(velocity, 0.0), 1.0, "velocity {velocity}");
        }
    }

    #[test]
    fn veltrack_gain_is_finite_for_every_amplitude_and_velocity() {
        // 力度 0 是 note-off：即使 `amp_veltrack` 为负（幂律在 0 处会溢出），也必须有限。
        for amp_veltrack in [AMP_VELTRACK_MIN, -50.0, 0.0, 50.0, AMP_VELTRACK_DEFAULT] {
            for velocity in 0u8..=127 {
                let gain = veltrack_gain(velocity, amp_veltrack);
                assert!(
                    gain.is_finite(),
                    "amp_veltrack {amp_veltrack} velocity {velocity} gave {gain}"
                );
                assert!(gain >= 0.0, "gain must not be negative");
            }
        }
    }

    #[test]
    fn veltrack_gain_matches_the_ledger_curve_exponent() {
        // 同一条幂律的另一种写法：2 * t/100 的指数。
        for amp_veltrack in [25.0f32, 40.0, 75.0] {
            let exponent = 2.0 * f64::from(amp_veltrack) / 100.0;
            for velocity in [1u8, 7, 63, 126] {
                let expected = (f64::from(velocity) / 127.0).powf(exponent) as f32;
                assert!(close(
                    veltrack_gain(velocity, amp_veltrack),
                    expected,
                    1.0e-6
                ));
            }
        }
    }
}
