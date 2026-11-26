//! 音频摆放的淡入淡出窗口（T-FIX-374）。
//!
//! ## 为什么这里只有"窗口"，没有曲线代码的三份副本
//!
//! 离线母带（`yeban-app` / `yeban-mcp`）与实时引擎（`yeban-engine`）各自拥有
//! 一份音频采样循环，但**何时开始淡、何时结束淡**必须只有一个定义。本模块
//! 负责把「摆放的帧区间 + 显式 fade tick」折叠成四个**绝对帧**：
//!
//! ```text
//! fade_in:  [in_start, in_end)   线性/等功率升到 1
//! fade_out: [out_start, out_end) 等功率降到 0
//! ```
//!
//! 曲线本身是**等功率**（`sqrt(t)` / `sqrt(1-t)`）：两条独立素材在重叠区
//! 交叉淡化时，功率之和恒为 1（线性淡化在同相位素材上会掉 −6 dB，在反相
//! 素材上根本消不掉阶跃）。`sqrt` 是代数运算，不是超越函数，实时回调可以用
//! [MUST-GATE-001]。
//!
//! ## 旧工程为什么逐字节不变
//!
//! 一个摆放两个 fade 字段都缺省 ⇒ [`ClipFadeWindow`] 不生成、各渲染器的 span
//! 里 `fade_window` 是 `None` ⇒ 渲染循环里一次乘法都不增加，输出位型与接线前
//! 相同。**重叠自动交叉淡化**也只在轨道上至少有一个摆放显式配置了 fade 时
//! 才启用（见 [`plan_fade_windows`] 的 `enabled` 判定）：没有任何 fade 字段的
//! 旧工程即使有重叠摆放，也仍然走旧的硬叠加，⛔ 不被新语义偷偷改写。

/// 一条摆放的**绝对帧**淡入淡出窗口。
///
/// 窗口用绝对帧而不是"相对本 span 的帧数"：循环展开后同一摆放会拆成多个
/// span，绝对帧让每个 span 都能算出自己落在窗口的哪一段，不会每个循环片
/// 都从 0 重新淡入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipFadeWindow {
    /// 淡入起点（含）。
    pub fade_in_start: u64,
    /// 淡入终点（不含）；与起点相同 ⇒ 没有淡入。
    pub fade_in_end: u64,
    /// 淡出起点（含）。
    pub fade_out_start: u64,
    /// 淡出终点（不含）；与起点相同 ⇒ 没有淡出。
    pub fade_out_end: u64,
    /// 这条淡出是否**收尾到静音**（T-FIX-441）：显式 fade-out 落在摆放自己的
    /// 尾巴上时为 `true`，渲染窗口的最后一帧必须落到 0；自动交叉淡化生成的
    /// 淡出（后面还有另一条素材顶着）为 `false`，末帧仍留 `sqrt(1/len)` 以
    /// 保持两侧功率之和为 1。
    pub fade_out_closes_to_silence: bool,
}

impl ClipFadeWindow {
    /// 是否一个 fade 都没有（渲染器据此整段跳过乘法）。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fade_in_start == self.fade_in_end && self.fade_out_start == self.fade_out_end
    }

    /// 某一**绝对帧**上的等功率增益（0..=1）。
    ///
    /// 两个窗口在同一点重叠时相乘（显式 fade 与自动交叉淡化可以叠加）。
    #[must_use]
    pub fn gain_at(&self, frame: u64) -> f32 {
        let mut gain = 1.0_f32;
        let in_len = self.fade_in_end.saturating_sub(self.fade_in_start);
        if in_len > 0 && frame >= self.fade_in_start && frame < self.fade_in_end {
            let elapsed = frame.saturating_sub(self.fade_in_start);
            let t = (elapsed as f32 / in_len as f32).clamp(0.0, 1.0);
            gain *= t.sqrt();
        }
        let out_len = self.fade_out_end.saturating_sub(self.fade_out_start);
        if out_len > 0 && frame >= self.fade_out_start && frame < self.fade_out_end {
            // T-FIX-441：收尾到静音的淡出用**闭区间**（末帧 = `end - 1` ⇒ t = 0），
            // 否则 25 帧（1 tick）尾巴会停在 sqrt(1/25) = 0.2，下一帧静音形成台阶。
            // 交叉淡化的淡出**不进这条**：它的末帧还要与后一条的淡入凑成功率 1。
            let t = if self.fade_out_closes_to_silence {
                let last = self.fade_out_end.saturating_sub(1);
                let span = out_len.saturating_sub(1);
                if span == 0 {
                    0.0
                } else {
                    (last.saturating_sub(frame) as f32 / span as f32).clamp(0.0, 1.0)
                }
            } else {
                let remaining = self.fade_out_end.saturating_sub(frame);
                (remaining as f32 / out_len as f32).clamp(0.0, 1.0)
            };
            gain *= t.sqrt();
        }
        gain
    }
}

/// 参与自动交叉淡化规划的一条摆放（帧区间 + 显式 fade 帧数）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipFadeInterval {
    /// 摆放起始帧（含）。
    pub start: u64,
    /// 摆放结束帧（不含）。
    pub end: u64,
    /// 显式淡入帧数（0 ⇒ 无）。
    pub fade_in_frames: u64,
    /// 显式淡出帧数（0 ⇒ 无）。
    pub fade_out_frames: u64,
    /// 这条摆放是否**显式**配置过 fade（`Some`，含 0 不算）。
    pub explicit: bool,
}

/// 由摆放的帧区间生成逐摆放的 fade 窗口。
///
/// 规则（唯一一份）：
///
/// 1. 显式 `fade_in_frames` / `fade_out_frames` 分别贴在摆放自己的两端；
/// 2. **只有**轨道上至少一条摆放显式配置了 fade 时（`explicit`），才为
///    相邻**部分重叠**的摆放生成**自动交叉淡化**：重叠区
///    `[max(start_i, start_j), min(end_i, end_j))` 既作为前一条的淡出尾，
///    也作为后一条的淡入头；内层在外层尾部之前结束的**包含式重叠**不做
///    这种并集，两条摆放各自保留显式窗口（见下）；
/// 3. 没有显式 fade 的旧工程 ⇒ 全部窗口为空 ⇒ 渲染逐字节不变（含重叠工程）；
/// 4. 窗口按参数顺序返回，调用方用同一个下标贴回自己的 span。
#[must_use]
pub fn plan_fade_windows(intervals: &[ClipFadeInterval]) -> Vec<ClipFadeWindow> {
    let mut windows: Vec<ClipFadeWindow> = intervals
        .iter()
        .map(|interval| {
            let fade_in_end = if interval.fade_in_frames == 0 {
                interval.start
            } else {
                interval.start.saturating_add(interval.fade_in_frames)
            };
            let fade_out_start = if interval.fade_out_frames == 0 {
                interval.end
            } else {
                interval.end.saturating_sub(interval.fade_out_frames)
            };
            ClipFadeWindow {
                fade_in_start: interval.start,
                fade_in_end,
                fade_out_start,
                fade_out_end: interval.end,
                fade_out_closes_to_silence: interval.fade_out_frames > 0,
            }
        })
        .collect();
    if !intervals.iter().any(|interval| interval.explicit) {
        return windows;
    }
    // 调用方保证 intervals 已按 start 升序（同 start 的顺序不影响结果）。
    for i in 0..intervals.len() {
        for j in (i + 1)..intervals.len() {
            let left = intervals[i];
            let right = intervals[j];
            if right.start >= left.end {
                break;
            }
            let overlap_start = left.start.max(right.start);
            let overlap_end = left.end.min(right.end);
            if overlap_start >= overlap_end {
                continue;
            }
            // 包含式重叠（一条摆放整段落在另一条里面、且内层在外层尾部之前
            // 结束）不是交叉淡化：外层与内层各自保留自己的显式窗口。把整个
            // 重叠区并进外层淡出会把内层的显式 fade 拉成整段，并让外层从内层
            // 起点一路淡到自己的尾部（ISS-MWVZY8WH-444EA9）。若内层与外层
            // 同尾结束，两条素材都要收尾，仍按下面的并集做交叉淡化；否则内层
            // 消失时外层会硬切。需要压外层的场景应另走 duck 包络，不能拿等
            // 功率交叉淡化窗口冒充。
            let left_contains_right = left.start <= right.start && right.end < left.end;
            let right_contains_left = right.start <= left.start && left.end < right.end;
            if left_contains_right || right_contains_left {
                continue;
            }
            // 部分重叠：前一条淡出窗口并入重叠区（并集，不缩短用户显式给的
            // 尾巴）。
            windows[i].fade_out_start = windows[i].fade_out_start.min(overlap_start);
            windows[i].fade_out_end = windows[i].fade_out_end.max(overlap_end);
            // 后一条：淡入窗口并入重叠区。
            windows[j].fade_in_start = windows[j].fade_in_start.min(overlap_start);
            windows[j].fade_in_end = windows[j].fade_in_end.max(overlap_end);
        }
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(start: u64, end: u64, fade_in: u64, fade_out: u64) -> ClipFadeInterval {
        ClipFadeInterval {
            start,
            end,
            fade_in_frames: fade_in,
            fade_out_frames: fade_out,
            explicit: fade_in > 0 || fade_out > 0,
        }
    }

    #[test]
    fn equal_power_window_hits_the_expected_anchors() {
        let window = ClipFadeWindow {
            fade_in_start: 0,
            fade_in_end: 100,
            fade_out_start: 100,
            fade_out_end: 200,
            // 本夹具覆盖的是 100 帧等功率**交叉淡化**口径（不是收尾到静音）：
            // 中点仍应为 1/√2，末帧仍应与后一条凑成功率 1。
            fade_out_closes_to_silence: false,
        };
        assert!((window.gain_at(0) - 0.0).abs() < 1e-6);
        assert!((window.gain_at(50) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((window.gain_at(100) - 1.0).abs() < 1e-6);
        assert!((window.gain_at(150) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(window.gain_at(199) <= 0.1 + 1e-6);
    }

    /// T-FIX-441：25 帧 = 1 tick @120BPM/48kHz/PPQ960。收尾到静音的淡出，
    /// 最后一个被渲染帧必须正好 0；相邻帧阶跃只允许由等功率 sqrt 网格本身
    /// 决定（`sqrt(1/(len-1))`），不允许再叠加"末帧没淡完"的台阶。
    #[test]
    fn a_25_frame_terminal_fade_out_ends_at_zero_without_an_extra_step() {
        let window = ClipFadeWindow {
            fade_in_start: 0,
            fade_in_end: 0,
            fade_out_start: 0,
            fade_out_end: 25,
            fade_out_closes_to_silence: true,
        };
        assert_eq!(window.gain_at(0), 1.0, "首帧仍是满增益");
        assert_eq!(window.gain_at(24), 0.0, "末帧必须正好落到 0");
        assert_eq!(window.gain_at(25), 1.0, "窗口外不再乘淡出");

        let mut max_step = 0.0_f32;
        for frame in 1..25u64 {
            max_step = max_step.max((window.gain_at(frame) - window.gain_at(frame - 1)).abs());
        }
        // sqrt 网格的最后一格：sqrt(1/24) ≈ 0.204124；留 1e-4 的浮点余量。
        assert!(
            (max_step - 0.204_124).abs() < 1e-4,
            "相邻帧最大阶跃只能是 sqrt 网格值，实测 {max_step}"
        );
        assert!(max_step <= 0.204_2, "短淡出不得出现额外台阶: {max_step}");
    }

    /// 1 帧淡出的边界：窗口只剩一帧，必须直接是 0（旧公式这里是 1.0）。
    #[test]
    fn a_one_frame_terminal_fade_out_is_silent_on_its_only_frame() {
        let window = ClipFadeWindow {
            fade_in_start: 0,
            fade_in_end: 0,
            fade_out_start: 0,
            fade_out_end: 1,
            fade_out_closes_to_silence: true,
        };
        assert_eq!(window.gain_at(0), 0.0);
    }

    /// T-FIX-441 的交叉淡化口径：自动交叉淡化的淡出**不**归零，两侧功率和恒为 1。
    /// 这条防止"为了短淡出归零"把 T-FIX-374 的等功率补偿弄坏。
    #[test]
    fn an_automatic_crossfade_keeps_equal_power_after_the_terminal_fix() {
        let mut right = interval(480, 1440, 0, 0);
        right.explicit = true;
        let windows = plan_fade_windows(&[interval(0, 960, 0, 0), right]);
        assert!(!windows[0].fade_out_closes_to_silence, "交叉淡化不是收尾");
        for frame in 480..960u64 {
            let power = windows[0].gain_at(frame).powi(2) + windows[1].gain_at(frame).powi(2);
            assert!(
                (power - 1.0).abs() < 1e-5,
                "交叉淡化第 {frame} 帧功率和 {power} 偏离 1"
            );
        }
    }

    /// 100 帧既有锚点不放宽：普通（非收尾）100 帧窗口第 199 帧仍是 1/10。
    #[test]
    fn the_hundred_frame_anchor_is_not_relaxed() {
        let window = ClipFadeWindow {
            fade_in_start: 0,
            fade_in_end: 0,
            fade_out_start: 100,
            fade_out_end: 200,
            fade_out_closes_to_silence: false,
        };
        assert!(
            (window.gain_at(199) - 0.1).abs() < 1e-6,
            "旧锚点必须保持 0.1"
        );
        let terminal = ClipFadeWindow {
            fade_out_closes_to_silence: true,
            ..window
        };
        assert_eq!(terminal.gain_at(199), 0.0, "收尾窗口末帧必须 0");
    }

    #[test]
    fn no_explicit_fade_keeps_every_window_empty_even_when_overlapping() {
        let windows = plan_fade_windows(&[interval(0, 960, 0, 0), interval(480, 1440, 0, 0)]);
        assert!(windows.iter().all(ClipFadeWindow::is_empty));
    }

    #[test]
    fn explicit_fade_turns_the_overlap_into_a_crossfade() {
        // 没有 explicit ⇒ 不交叉；把其中一条标成 explicit（哪怕是 0 帧）再规划。
        let mut right = interval(480, 1440, 0, 0);
        right.explicit = true;
        let windows = plan_fade_windows(&[interval(0, 960, 0, 0), right]);
        assert_eq!(
            windows[0],
            ClipFadeWindow {
                fade_in_start: 0,
                fade_in_end: 0,
                fade_out_start: 480,
                fade_out_end: 960,
                // 自动交叉淡化：后一条顶上，末帧不能归零（否则功率和掉档）。
                fade_out_closes_to_silence: false,
            }
        );
        assert_eq!(
            windows[1],
            ClipFadeWindow {
                fade_in_start: 480,
                fade_in_end: 960,
                fade_out_start: 1440,
                fade_out_end: 1440,
                fade_out_closes_to_silence: false,
            }
        );
    }

    #[test]
    fn anti_phase_dc_overlap_has_no_hard_step() {
        // T-FIX-374 的复现夹具：+0.3 与 -0.3 两段 DC 在 500 帧处硬拼，
        // 硬叠加会在一帧内出现 −0.6 的阶跃。交叉淡化后相邻帧差必须很小。
        let mut right = interval(500, 1000, 0, 0);
        right.explicit = true;
        let windows = plan_fade_windows(&[interval(0, 1000, 0, 0), right]);
        let mut previous: Option<f32> = None;
        let mut max_step = 0.0_f32;
        for frame in 0..1000u64 {
            let left = if frame < 1000 {
                0.3 * windows[0].gain_at(frame)
            } else {
                0.0
            };
            let right = if frame >= 500 {
                -0.3 * windows[1].gain_at(frame)
            } else {
                0.0
            };
            let mixed = left + right;
            if let Some(previous) = previous {
                max_step = max_step.max((mixed - previous).abs());
            }
            previous = Some(mixed);
        }
        assert!(
            max_step < 0.02,
            "交叉淡化后最大相邻帧阶跃必须远小于旧的 0.6（实测 {max_step}）"
        );
    }

    #[test]
    fn containment_preserves_inner_explicit_fades_and_outer_tail() {
        // ISS-MWVZY8WH-444EA9：B 整段落在 A 里。包含式重叠不能把 B 的
        // 10 帧 fade_in 拉成整段，也不能让 A 从 B 的起点一路淡到 A 的尾。
        let outer = interval(0, 1000, 0, 0);
        let inner = interval(100, 200, 10, 10);
        let windows = plan_fade_windows(&[outer, inner]);
        assert_eq!(
            windows[0],
            ClipFadeWindow {
                fade_in_start: 0,
                fade_in_end: 0,
                fade_out_start: 1000,
                fade_out_end: 1000,
                // 外层没有显式 fade-out（空窗口）；这里的标记不表达任何收尾语义。
                fade_out_closes_to_silence: false,
            }
        );
        assert_eq!(windows[1].fade_in_start, 100);
        assert_eq!(windows[1].fade_in_end, 110);
        assert_eq!(windows[1].fade_out_start, 190);
        assert_eq!(windows[1].fade_out_end, 200);
        assert!(
            windows[1].gain_at(199) < 0.5,
            "内层结束前一帧不能残留接近满增益（实测 {}）",
            windows[1].gain_at(199)
        );
        assert_eq!(windows[0].gain_at(200), 1.0);
    }

    #[test]
    fn containment_does_not_move_outer_explicit_fade_out() {
        let outer = interval(0, 1000, 0, 100);
        let inner = interval(100, 200, 10, 10);
        let windows = plan_fade_windows(&[outer, inner]);
        assert_eq!(windows[0].fade_out_start, 900);
        assert_eq!(windows[0].fade_out_end, 1000);
        assert_eq!(windows[0].fade_in_start, 0);
        assert_eq!(windows[0].fade_in_end, 0);
    }

    #[test]
    fn same_start_containment_is_independent_of_interval_order() {
        let outer = interval(0, 1000, 0, 0);
        let inner = interval(0, 200, 10, 10);
        let forward = plan_fade_windows(&[outer, inner]);
        let reversed = plan_fade_windows(&[inner, outer]);
        assert_eq!(forward[0], reversed[1]);
        assert_eq!(forward[1], reversed[0]);
        assert_eq!(forward[1].fade_in_end, 10);
        assert_eq!(forward[1].fade_out_start, 190);
    }

    #[test]
    fn explicit_edges_are_unioned_with_the_overlap() {
        let mut left = interval(0, 960, 0, 0);
        left.explicit = true;
        let windows = plan_fade_windows(&[left, interval(480, 1440, 0, 0)]);
        assert_eq!(windows[0].fade_out_start, 480);
        assert_eq!(windows[0].fade_out_end, 960);
        assert_eq!(windows[1].fade_in_start, 480);
        assert_eq!(windows[1].fade_in_end, 960);
    }
}
