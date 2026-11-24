//! `[ADR-0010 D5]` 片段循环展开的**唯一**实现：纯整数、零 I/O。
//!
//! ## 为什么住在 `yeban-model`
//!
//! 循环区间是**摆放相对**坐标（`[ADR-0010 D1]`）：`loop_config.start_tick` /
//! `end_tick` 从 `ClipPlacement::start_tick` 起算，与 `MidiNote::start_tick`
//! 同一参照系。什么时候重复、每一遍多长，是**模型域**的纯整数判定
//! （`[ADR-0010 D2/D3]`），而 `yeban-engine` 与 `yeban-mcp` 是它的两个消费者。
//!
//! ⚠ 把这份语义放进 `yeban-render` 会让 `yeban-engine` 新增一条今天不存在的
//! 层间依赖边，并把 WAV/SMF/线程池拖进实时 crate 的依赖闭包 —— 见
//! `[ADR-0010 D5]` 的理由段。`yeban-engine` 与 `yeban-mcp` **都**依赖本 crate，
//! 因此放在这里是**零新依赖边**。
//!
//! ## 口径（逐字对应 ADR，⛔ 不在这里另立一套）
//!
//! 设 `L = end_tick − start_tick`（区间长度）、`D = duration_ticks`：
//!
//! - `enabled == false` ⇒ 单遍；
//! - `end_tick ≥ D`（含"恰好覆盖"）⇒ 单遍，**逐位等于**关闭；
//! - `end_tick ≤ start_tick` ⇒ 单遍（非法区间不猜测作者意图）；
//! - 否则 `[start, end)` 按 `L` 平铺，`N = ceil((D − start) / L)`；
//! - 每一遍以 `end_tick` 为硬切边界；`start > 0` 时区间**之前**的内容在开头
//!   播一遍（pickup）；起音 `≥ end` 不参与循环演奏。
//!
//! ⚠ `[ADR-0010 D6]`：tick→帧换算**不在**本模块，也**不在**本 ADR 范围。

use crate::project::ClipPlacement;

/// 一个摆放的循环展开计划（`[ADR-0010 D2]`）。
///
/// 由 [`ClipPlacement::loop_repeat`] 算出；`None` 表示单遍播放，与"不循环"
/// **逐位相同**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRepeat {
    /// 循环区间起点（摆放相对 tick）。
    pub start_tick: u64,
    /// 循环区间终点（摆放相对 tick，**不含**）。
    pub end_tick: u64,
    /// 循环周期（tick）= `end_tick - start_tick`。
    pub period_ticks: u64,
    /// 铺满摆放所需的遍数 `ceil((duration_ticks - start_tick) / period_ticks)`。
    pub iterations: u64,
}

impl ClipPlacement {
    /// 由摆放算出循环展开计划；`None` = 单遍播放（逐位等于不循环）。
    ///
    /// 判定顺序逐字对应 `[ADR-0010 D2]`：
    ///
    /// 1. `enabled == false` ⇒ `None`；
    /// 2. `end_tick <= start_tick` ⇒ `None`（防御：模型层
    ///    [`crate::LoopConfig::validate`] 已拒绝启用状态下的这一形态，但手改容器
    ///    可能绕过它；这里不下溢、不 panic）；
    /// 3. `end_tick >= duration_ticks` ⇒ `None`（含恰好覆盖 —— 播放期间不会回卷）；
    /// 4. 其余 ⇒ `Some`，`iterations = ceil((duration - start) / period)`。
    #[must_use]
    pub fn loop_repeat(&self) -> Option<LoopRepeat> {
        let loop_config = self.loop_config;
        if !loop_config.enabled {
            return None;
        }
        let start_tick = loop_config.start_tick;
        let end_tick = loop_config.end_tick;
        // `[ADR-0010 D2]`：非法区间按"无循环"处理，不猜测作者意图；同时避免
        // `end - start` 下溢。
        if end_tick <= start_tick {
            return None;
        }
        if end_tick >= self.duration_ticks {
            // `[ADR-0010 D2]`：回卷点在摆放末端之外（或恰好重合）⇒ 播放期间不会回卷。
            return None;
        }
        let period_ticks = end_tick - start_tick;
        let remaining = self.duration_ticks.saturating_sub(start_tick);
        // `remaining > 0`：因为 `end_tick < duration_ticks` 且 `start_tick < end_tick`
        // ⇒ `start_tick < duration_ticks`。`div_ceil` 让最后一遍不完整时也发出。
        let iterations = remaining.div_ceil(period_ticks).max(1);
        Some(LoopRepeat {
            start_tick,
            end_tick,
            period_ticks,
            iterations,
        })
    }

    /// 一个 MIDI 音符在一个摆放里要排的**每一遍** `(起点 tick, 时值 tick)`。
    ///
    /// 参数是**摆放相对**坐标（与 `MidiNote::start_tick` 同一参照系）：
    ///
    /// - 没有循环计划 ⇒ 原样一遍（`vec![(start, duration)]`，与展开之前逐位相同）；
    /// - 循环计划在场且音符起音**在区间之前** ⇒ 原样一遍（pickup，不复制）；
    /// - 起音在 `[start, end)` ⇒ `N` 遍，每遍时值裁到区间末端 `end`
    ///   （`[ADR-0010 D3]` 的硬切边界；最后一遍再裁到摆放末端）；
    /// - 起音 `>= end` ⇒ 空（循环区间就是被演奏的内容）。
    ///
    /// 时值裁到区间末端是**有意的**：不裁的话前一遍的尾音会叠进下一遍，
    /// 分段峰值就不再相等 —— 那正是"只把长度撑长"与"真的重复"的分界。
    ///
    /// ⚠ **不受信任的工程时长**（导入/手改文件、`duration_ticks` 无上限）不要直接调本方法：
    /// 它按完整遍数物化，`D=10^7 / loop 0..1` 会分配 10^7 份。需要容量上限的投影/排程
    /// 一律用 [`Self::note_copies_limited`]（T-FIX-247）。
    #[must_use]
    pub fn note_copies(&self, note_start: u64, note_duration: u64) -> Vec<(u64, u64)> {
        self.note_copies_limited(note_start, note_duration, usize::MAX)
            .copies
    }

    /// 与 [`Self::note_copies`] 同一张 tick 表，但**最多物化 `max_copies` 份**。
    ///
    /// 返回 `(可物化副本, 被省略的副本数)`。省略数按**未物化的有效副本**计，
    /// 不按 tick 跨度计 ⇒ 调用方可以直接把它累加进容量丢弃计数。
    ///
    /// 这条存在的理由：`Vec::with_capacity(repeat.iterations)` 直接信任工程字段，
    /// 一个 `D = 10^7 / loop 0..1` 的摆放会先在投影期分配 10^7 份副本再按
    /// `MAX_NOTES_PER_TRACK` 丢弃（OOM/卡死）。本方法把"分配"和"计数"分开：
    /// 分配只到 `max_copies`，省略只做整数减法，⛔ 不遍历被省略的那一段。
    #[must_use]
    pub fn note_copies_limited(
        &self,
        note_start: u64,
        note_duration: u64,
        max_copies: usize,
    ) -> NoteCopyBatch {
        let single = |omitted: u64| NoteCopyBatch {
            copies: if max_copies == 0 {
                Vec::new()
            } else {
                vec![(note_start, note_duration)]
            },
            omitted,
        };
        let Some(repeat) = self.loop_repeat() else {
            return single(u64::from(max_copies == 0));
        };
        if note_start < repeat.start_tick {
            return single(u64::from(max_copies == 0));
        }
        if note_start >= repeat.end_tick {
            return NoteCopyBatch {
                copies: Vec::new(),
                omitted: 0,
            };
        }
        let clipped = note_duration.min(repeat.end_tick - note_start);
        if clipped == 0 || note_start >= self.duration_ticks {
            return NoteCopyBatch {
                copies: Vec::new(),
                omitted: 0,
            };
        }
        // 有效副本数：`start < D` 才算。`note_start` 在 `[loop_start, loop_end)`，
        // 但最后一遍可能超过 `D`，所以不能直接拿 `repeat.iterations`。
        let total = (self.duration_ticks - note_start).div_ceil(repeat.period_ticks);
        let materialized = usize::try_from(total).unwrap_or(usize::MAX).min(max_copies);
        let omitted = total.saturating_sub(u64::try_from(materialized).unwrap_or(u64::MAX));
        let mut copies = Vec::with_capacity(materialized);
        for iteration in 0..materialized {
            #[allow(clippy::cast_possible_truncation)]
            let iteration = iteration as u64;
            let start = note_start.saturating_add(iteration.saturating_mul(repeat.period_ticks));
            let duration = clipped.min(self.duration_ticks - start);
            if duration > 0 {
                copies.push((start, duration));
            }
        }
        NoteCopyBatch { copies, omitted }
    }
}

/// [`ClipPlacement::note_copies_limited`] 的返回值。
///
/// `copies` 是**已物化**的 tick 副本（最多 `max_copies` 份）；`omitted` 是因
/// 调用方容量上限而没有生成的**有效副本数**（每份本应各占一个音符/脉冲起点）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteCopyBatch {
    /// 已物化的 `(起点 tick, 时值 tick)` 副本。
    pub copies: Vec<(u64, u64)>,
    /// 被省略的有效副本数（⛔ 不包含 `start >= duration_ticks` 的无效遍）。
    pub omitted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LoopConfig;

    /// 夹具：`D = 1920`，循环配置由调用方给（`#246` S1）。
    fn placed(loop_config: LoopConfig) -> ClipPlacement {
        ClipPlacement {
            duration_ticks: 1_920,
            loop_config,
            ..ClipPlacement::default()
        }
    }

    fn looping(start_tick: u64, end_tick: u64) -> LoopConfig {
        LoopConfig {
            enabled: true,
            start_tick,
            end_tick,
        }
    }

    /// S1：tick 级 copies 表 —— `D=1920 / loop 0..480 / 音符 0..240`。
    #[test]
    fn tick_table_tiles_the_loop_region_four_times() {
        let placement = placed(looping(0, 480));
        assert_eq!(
            placement.note_copies(0, 240),
            vec![(0, 240), (480, 240), (960, 240), (1_440, 240)],
            "N = ceil(1920 / 480) = 4，每遍原样 240 tick"
        );
    }

    /// S1：pickup 只播一遍，区间内的音符各播一遍（`end == D` ⇒ 不回卷）。
    #[test]
    fn tick_table_keeps_pickup_and_does_not_wrap_when_end_is_the_placement_end() {
        let placement = placed(looping(480, 1_920));
        let mut table = placement.note_copies(0, 240);
        table.extend(placement.note_copies(480, 240));
        assert_eq!(
            table,
            vec![(0, 240), (480, 240)],
            "区间之前 (0..240) 是 pickup 一遍；end == D ⇒ 区间内 (480..720) 也只播一遍"
        );
    }

    /// S1：`end >= D`（含越过末端）⇒ 单遍，与关闭逐位相同。
    #[test]
    fn tick_table_is_single_pass_when_the_loop_end_is_past_the_placement_end() {
        assert_eq!(
            placed(looping(0, 1_920)).note_copies(0, 240),
            vec![(0, 240)],
            "恰好覆盖 ⇒ 单遍"
        );
        assert_eq!(
            placed(looping(0, 3_840)).note_copies(0, 240),
            vec![(0, 240)],
            "越过末端 ⇒ 单遍"
        );
    }

    /// `loop_repeat` 的**穷尽边界**（纯函数，不需要渲染）。
    ///
    /// 摆放跨度固定为 `0..1920`。
    #[test]
    fn loop_repeat_covers_every_boundary() {
        fn with_bounds(enabled: bool, start_tick: u64, end_tick: u64) -> ClipPlacement {
            placed(LoopConfig {
                enabled,
                start_tick,
                end_tick,
            })
        }
        // 关掉的循环 ⇒ 单遍（哪怕区间形态怪异）。
        assert_eq!(with_bounds(false, 0, 0).loop_repeat(), None);
        assert_eq!(with_bounds(false, 480, 960).loop_repeat(), None);
        // 恰好覆盖 `0..duration_ticks` ⇒ 回卷点与末端重合 ⇒ 单遍。
        assert_eq!(with_bounds(true, 0, 1_920).loop_repeat(), None);
        // 区间短于摆放 ⇒ 真的重复：N = ceil(1920 / 960) = 2。
        assert_eq!(
            with_bounds(true, 0, 960).loop_repeat(),
            Some(LoopRepeat {
                start_tick: 0,
                end_tick: 960,
                period_ticks: 960,
                iterations: 2,
            })
        );
        // 起点不为 0：先播 480 tick 的 pickup，然后 `480..960` 重复 3 遍。
        assert_eq!(
            with_bounds(true, 480, 960).loop_repeat(),
            Some(LoopRepeat {
                start_tick: 480,
                end_tick: 960,
                period_ticks: 480,
                iterations: 3,
            })
        );
        // 终点正好是摆放末端 ⇒ 不会回卷 ⇒ 单遍。
        assert_eq!(with_bounds(true, 480, 1_920).loop_repeat(), None);
        // 终点越过摆放末端 ⇒ 同样不会回卷 ⇒ 单遍。
        assert_eq!(with_bounds(true, 0, 3_840).loop_repeat(), None);
        // 模型层拒绝 `end <= start`；这里防御性返回 None（不下溢、不 panic）。
        assert_eq!(with_bounds(true, 960, 960).loop_repeat(), None);
        assert_eq!(with_bounds(true, 1_920, 960).loop_repeat(), None);
    }

    /// ⭐ T-FIX-247：`note_copies_limited` 只物化上限内的副本，
    /// 省略数按有效副本计；小 N 时与 `note_copies` 前缀**逐项相同**。
    #[test]
    fn note_copies_limited_caps_materialization_without_losing_the_count() {
        // 小 N 对照：4 遍、上限 3 ⇒ 前 3 遍原样，省略 1。
        let placement = placed(looping(0, 480));
        let batch = placement.note_copies_limited(0, 240, 3);
        assert_eq!(batch.copies, vec![(0, 240), (480, 240), (960, 240)]);
        assert_eq!(batch.omitted, 1);
        // 上限 0 ⇒ 一份都不物化，但省略数仍知道总量（⛔ 不是 0）。
        let none = placement.note_copies_limited(0, 240, 0);
        assert!(none.copies.is_empty());
        assert_eq!(none.omitted, 4);
        // 超大跨度：D=10^7 / loop 0..1、上限 8192 ⇒ 物化固定 8192，
        // 省略 10^7 − 8192；另一个 D=10^5 也得到同样的**物化上限**。
        for duration in [100_000u64, 10_000_000] {
            let huge = ClipPlacement {
                duration_ticks: duration,
                loop_config: looping(0, 1),
                ..ClipPlacement::default()
            };
            let batch = huge.note_copies_limited(0, 1, 8192);
            assert_eq!(batch.copies.len(), 8192, "物化份数固定在上限");
            assert_eq!(
                batch.omitted,
                duration - 8192,
                "省略数 = 有效总份数 − 物化份数"
            );
        }
    }

    /// `note_copies`：区间内的音符复制 N 遍且尾巴裁到区间末端；
    /// pickup 只播一遍；区间之后的音符不参与循环演奏。
    #[test]
    fn note_copies_repeat_only_the_loop_region() {
        let placement = placed(looping(0, 480));
        assert_eq!(
            placement.note_copies(0, 240),
            vec![(0, 240), (480, 240), (960, 240), (1_440, 240)],
            "区间内的音符复制 N=4 遍"
        );
        assert_eq!(
            placement.note_copies(0, 600),
            vec![(0, 480), (480, 480), (960, 480), (1_440, 480)],
            "尾巴裁到区间末端 (480), 否则前一遍会叠进后一遍"
        );
        assert_eq!(
            placement.note_copies(600, 240),
            Vec::<(u64, u64)>::new(),
            "区间之后的音符不参与循环演奏"
        );
        // pickup：区间起点之前的内容只播一遍。
        let shifted = placed(looping(480, 960));
        assert_eq!(
            shifted.note_copies(0, 240),
            vec![(0, 240)],
            "pickup 只播一遍"
        );
        assert_eq!(
            shifted.note_copies(480, 240),
            vec![(480, 240), (960, 240), (1_440, 240)],
            "区间内的音符从起点开始重复"
        );
    }
}
