//! 节奏网格 (rhythm grid)：把**流派登记的拍号与摇摆比例**落成具体的小节内 onset 网格。
//!
//! ## 为什么需要这个模块
//!
//! `docs/ledger/theory-core-notes.md` 的 `pending 3` 记着：本 crate
//! "没有实现节奏型/鼓组 pattern：`GenreRule` 只有密度提示与摇摆比例，
//! 没有具体的鼓点网格"。本模块补上**网格**这一半：给定拍号、小节数与
//! 每小节的 onset 数，产出每个 onset **具体落在哪个 tick** 的确定性网格。
//!
//! 本模块**没有**关闭 `pending 3` 的另一半（逐流派的鼓点型数据）。原因见下一节。
//!
//! ## 为什么 onset 数由调用方给出，而不是读 `GenreRule::note_density_hint`
//!
//! `GenreRule::note_density_hint` 的字段文档写的是"**每小节的典型音符数区间**"，
//! 而登记数据里存在 `(48, 160)`、`(32, 160)`、`(32, 128)` 这样的区间
//! （`crates/yeban-theory/src/genre.rs` 的 `GENRES`）。4/4 一小节在
//! [`GRID_CELL_TICKS`] = 240 tick 的网格上只有 **16** 个格位 ⇒
//! "每小节 160 个音符"**不可能**读成"每小节 160 个 onset"。
//! 它计的是**音符数**（含和弦内的复音），不是 onset 数。
//!
//! 把音符数当 onset 数用会同时踩两条纪律：要么静默钳制（本 crate 明确反对，
//! 见 [`crate::swing::validate_swing_permille`] 的口径），要么按未登记的比例折算
//! （凭空发明数据）。因此 onset 数是**调用方参数**；密度提示留给"织体/配器"一层。
//!
//! ## 语义
//!
//! 1. 网格单元 = [`GRID_CELL_TICKS`] = [`crate::progression::MIN_DURATION_TICKS`]
//!    = 240 tick（一个 16 分音符），与 crate 既有的"最小时间分辨率"口径同源。
//! 2. 每小节格点数 = `ticks_per_bar / GRID_CELL_TICKS`（整数）。拍号由
//!    [`Meter::new`] 校验为 2 的幂分母，因此这个除法恒为整除。
//! 3. **选哪些格点**：按**度量重量**从强到弱取。重量用二分度量层级
//!    （Lerdahl–Jackendoff 式）：小节起点最重，其次是小节对半处，再是四分位……
//!    同重量按格点下标升序打破平局。判定全部是整数，无浮点比较 ⇒
//!    同输入同输出、跨平台逐位一致 [ARCH-DET-001]。
//! 4. **摇摆**：给出千分比时，把每个摇摆对（= 一个八分音符 = 480 tick）**切开**：
//!    前半格点不动，后半格点落在 [`crate::swing::SwingPair::first`] 处
//!    （上限为对长 - 1 tick）。因此每对的起点始终落在平直网格上，
//!    且两个格点**永不重合**。摇摆对以**小节起点**为基准（格点 0 与 1 是一对）。
//!
//!    这里用 [`crate::swing::swung_pair_span`]（**切分**）而不是
//!    [`crate::swing::quantize_onset`]（**量化**）：量化是多对一的，
//!    `permille == 1000` 时"正中间归前半"的平局规则会把一个格点吸回前一个
//!    （`quantize_onset(240, 480, 1000) == 0`），网格里两个格点就会重合。
//!    网格必须保持"一格一点"，所以取切分语义。
//! 5. 恒有：`tick` 严格升序、每小节恰好 `onsets_per_bar` 个 onset、
//!    全部落在本小节内、小节按顺序铺满 `bars × ticks_per_bar`。`permille == None`
//!    与 `permille == Some(500)` 产出的 `hits` 逐位相同（只有 `swing_permille()`
//!    回显的入参不同）。
//!
//! ## 边界（音频红线）
//!
//! - **构造期分配，逐样本零分配**：构造网格会分配一个 `Vec`（`bars × onsets_per_bar`
//!   个元素），因此构造器**不得**在音频线程上调用。读侧（[`MetricGrid::hits`]、
//!   [`MetricGrid::hits_in_bar`]）只返回切片：不分配、不加锁、不做阻塞 I/O、不打日志。
//! - 本模块不含 `HashMap` / `HashSet` [MODEL-AST-003 / AGENTS.md 红线 4]。
//! - 本模块不读时钟、不做 I/O、不打印；输出只由入参决定。

use crate::error::TheoryError;
use crate::progression::{MIN_DURATION_TICKS, Meter, PPQ};

/// 网格单元的长度：一个 16 分音符 = 240 tick。
///
/// 与 [`MIN_DURATION_TICKS`] 同值。这个常量是 [`crate::progression::PPQ`]
/// 网格上本 crate 承诺的**最小时间分辨率**。
pub const GRID_CELL_TICKS: u64 = MIN_DURATION_TICKS;

/// 一个摇摆对的长度：一个八分音符 = 480 tick。
///
/// 与 [`crate::swing`] 的模块文档同口径（`PPQ` = 960 下的八分音符）。
pub const SWING_PAIR_TICKS: u64 = PPQ / 2;

/// 度量重量的上界。小节起点（格点 0）取这个值。
///
/// 取值 8 的依据：只要格点下标能被 `2^8 = 256` 整除，它就已经落在小节的前
/// `1/256` 边界上；在 4/4（16 个格位）与 7/8（14 个格位）里都超过任何实际格点，
/// 因此不会与格点 0 之外的真实位置混淆。
pub const MAX_METRIC_WEIGHT: u8 = 8;

/// 一小节里的格点数。
///
/// 单位是"格"（1 格 = [`GRID_CELL_TICKS`] = 240 tick）。
/// `meter` 非法（分子为 0、分母不是 2 的幂）时返回 `None`：那些拍号由
/// [`Meter::new`] 拒绝，且 `ticks_per_bar` 会做有损整除，本模块不接受。
#[must_use]
pub const fn cells_per_bar(meter: Meter) -> Option<u64> {
    if meter.numerator == 0 || meter.denominator == 0 || !meter.denominator.is_power_of_two() {
        return None;
    }
    Some(meter.ticks_per_bar() / GRID_CELL_TICKS)
}

/// 小节内第 `cell` 个格点的**度量重量**（越大越强）。
///
/// 口径：二分度量层级。`cell == 0` 取 [`MAX_METRIC_WEIGHT`]；
/// 其余取 `cell` 二进制末尾零的个数（上限 [`MAX_METRIC_WEIGHT`]）。
/// 4/4（16 格）因此得到：格点 0 → 8、格点 8 → 3、格点 4/12 → 2、
/// 格点 2/6/10/14 → 1、奇数格点 → 0。
///
/// 这是二拍子体系的经典口径；对 3/4、5/4、7/8 这类非二倍分子只做近似，
/// 本函数不假装它给出各地区的节拍分组（那需要逐流派数据，属 `pending 3` 的另一半）。
#[must_use]
pub const fn metric_weight(cell: u32) -> u8 {
    if cell == 0 {
        return MAX_METRIC_WEIGHT;
    }
    let trailing = cell.trailing_zeros();
    if trailing >= MAX_METRIC_WEIGHT as u32 {
        MAX_METRIC_WEIGHT
    } else {
        trailing as u8
    }
}

/// 网格里的一个 onset。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GridHit {
    /// 绝对 tick（已应用摇摆位移；平直时等于格点 tick）。
    pub tick: u64,
    /// 所在小节（0 基）。
    pub bar: u32,
    /// 小节内的格点下标（0 基）。
    pub cell: u32,
    /// 该格点的度量重量，见 [`metric_weight`]。
    pub weight: u8,
}

/// 一个拍号在若干小节上的 onset 网格。
///
/// 由 [`metric_grid`] / [`swung_metric_grid`] 构造，或由
/// [`crate::genre::GenreRule::rhythm_grid`] 按流派登记数据构造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricGrid {
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    hits: Vec<GridHit>,
}

impl MetricGrid {
    /// 全部 onset，按 `tick` 严格升序。
    #[must_use]
    pub fn hits(&self) -> &[GridHit] {
        &self.hits
    }

    /// onset 总数（恒等于 `bars × onsets_per_bar`）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// onset 总数为 0（即调用方请求了 `onsets_per_bar == 0`）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// 拍号。
    #[must_use]
    pub const fn meter(&self) -> Meter {
        self.meter
    }

    /// 小节数。
    #[must_use]
    pub const fn bars(&self) -> u32 {
        self.bars
    }

    /// 每小节的 onset 数。
    #[must_use]
    pub const fn onsets_per_bar(&self) -> u32 {
        self.onsets_per_bar
    }

    /// 应用的摇摆千分比；`None` = 平直。
    #[must_use]
    pub const fn swing_permille(&self) -> Option<u16> {
        self.permille
    }

    /// 每小节 tick 数。
    #[must_use]
    pub const fn ticks_per_bar(&self) -> u64 {
        self.meter.ticks_per_bar()
    }

    /// 覆盖的总 tick 数（恒等于 `bars × ticks_per_bar`）。
    #[must_use]
    pub const fn total_ticks(&self) -> u64 {
        self.ticks_per_bar() * self.bars as u64
    }

    /// 第 `bar` 小节的 onset 切片；`bar >= bars` 时返回空切片。
    ///
    /// 不分配、不加锁、不做 I/O。
    #[must_use]
    pub fn hits_in_bar(&self, bar: u32) -> &[GridHit] {
        if bar >= self.bars {
            return &[];
        }
        let width = self.onsets_per_bar as usize;
        let start = bar as usize * width;
        &self.hits[start..start + width]
    }
}

/// 平直网格：`onsets_per_bar` 个 onset/小节，不应用摇摆。
///
/// # Errors
///
/// - `bars == 0` ⇒ [`TheoryError::ZeroBars`]；
/// - 拍号不是合法拍号（见 [`Meter::new`]）⇒ [`TheoryError::ZeroBars`]；
/// - `onsets_per_bar` 超过小节内的格位数 ⇒ [`TheoryError::ProgressionTooDense`]
///   （`degrees` 承载请求的 onset 数，`slots` 承载可用格位数）。
///   这里**不**静默钳制：钳制会让"每小节 20 个 onset"悄悄变成 16 个。
pub fn metric_grid(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
) -> Result<MetricGrid, TheoryError> {
    swung_metric_grid(meter, bars, onsets_per_bar, None)
}

/// 摇摆网格：与 [`metric_grid`] 同语义，另外把每个摇摆对的后半格点推移。
///
/// `permille` 为 `None` 时与 [`metric_grid`] 逐位相同；为 `Some(p)` 时
/// `p` 必须落在 [`crate::swing::SWING_PERMILLE_STRAIGHT`]`..=`[`crate::swing::SWING_PERMILLE_MAX`]。
///
/// # Errors
///
/// 除 [`metric_grid`] 的错误外，`p` 越界时返回 [`TheoryError::SwingOutOfRange`]。
pub fn swung_metric_grid(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
) -> Result<MetricGrid, TheoryError> {
    if bars == 0 {
        return Err(TheoryError::ZeroBars);
    }
    // 借用 Meter::new 的校验：公有的 `Meter` 字段可以被绕过构造器直接赋值，
    // 那时 `ticks_per_bar` 会做有损整除（甚至分母为 0 时 panic）。
    Meter::new(meter.numerator, meter.denominator)?;
    if let Some(p) = permille {
        crate::swing::validate_swing_permille(p)?;
    }
    let cells = cells_per_bar(meter).ok_or(TheoryError::ZeroBars)?;
    let slots = usize::try_from(cells).unwrap_or(usize::MAX);
    let wanted = onsets_per_bar as usize;
    if wanted > slots {
        return Err(TheoryError::ProgressionTooDense {
            degrees: wanted,
            slots,
        });
    }

    // 选格点：重量降序、同重量按格点升序。排序键是整数，因此结果确定。
    let mut order: Vec<u32> = (0..cells as u32).collect();
    order.sort_by_key(|&cell| (core::cmp::Reverse(metric_weight(cell)), cell));
    order.truncate(wanted);
    order.sort_unstable();

    let bar_ticks = meter.ticks_per_bar();

    // 后半格点在一对之内的落点（tick）。用**切分**（[`crate::swing::swung_pair_span`]）
    // 而不是**量化**（[`crate::swing::quantize_onset`]）：量化是多对一的，
    // 在 `permille == 1000` 时"正中间归前半"的平局规则会把后半格点吸回前半
    // （`quantize_onset(240, 480, 1000) == 0`）—— 网格里两个不同格点因此会重合。
    // 切分给出对内的第二个槽位，并把它钳在对尾之前一 tick，与
    // `swing::tests::maximum_swing_keeps_an_onset_inside_its_pair` 同口径。
    let second_slot = match permille {
        None => None,
        Some(p) => Some(
            crate::swing::swung_pair_span(SWING_PAIR_TICKS, p)?
                .first
                .min(SWING_PAIR_TICKS - 1),
        ),
    };

    let mut hits = Vec::with_capacity(bars as usize * wanted);
    for bar in 0..bars {
        let bar_start = bar as u64 * bar_ticks;
        for &cell in &order {
            let local = u64::from(cell) * GRID_CELL_TICKS;
            let local_tick = match second_slot {
                None => local,
                Some(slot) => {
                    // 摇摆对以小节起点为基准（格点 0 与 1 是一对）。
                    let pair_start = local - local % SWING_PAIR_TICKS;
                    if local == pair_start {
                        local
                    } else {
                        pair_start + slot
                    }
                }
            };
            hits.push(GridHit {
                tick: bar_start + local_tick,
                bar,
                cell,
                weight: metric_weight(cell),
            });
        }
    }
    debug_assert!(
        hits.windows(2).all(|pair| pair[0].tick < pair[1].tick),
        "grid onsets must be strictly ascending: meter={meter:?} bars={bars} onsets={onsets_per_bar} permille={permille:?} offenders={:?}",
        hits.windows(2)
            .filter(|pair| pair[0].tick >= pair[1].tick)
            .collect::<Vec<_>>()
    );
    Ok(MetricGrid {
        meter,
        bars,
        onsets_per_bar,
        permille,
        hits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swing::{SWING_PERMILLE_MAX, SWING_PERMILLE_STRAIGHT};

    const COMMON: Meter = Meter::COMMON;

    #[test]
    fn cells_per_bar_counts_sixteenth_notes() {
        assert_eq!(cells_per_bar(COMMON), Some(16)); // 4/4: 3840 / 240
        assert_eq!(cells_per_bar(Meter::WALTZ), Some(12)); // 3/4: 2880 / 240
        assert_eq!(cells_per_bar(Meter::MARCH), Some(8)); // 2/4: 1920 / 240
        assert_eq!(cells_per_bar(Meter::COMPOUND_DUPLE), Some(12)); // 6/8: 2880 / 240
        assert_eq!(cells_per_bar(Meter::SEVEN_EIGHT), Some(14)); // 7/8: 3360 / 240
        assert_eq!(cells_per_bar(Meter::QUINTUPLE), Some(20)); // 5/4: 4800 / 240
        // 非法拍号（公有的 Meter 字段可以绕过 Meter::new）→ None，不 panic。
        assert_eq!(
            cells_per_bar(Meter {
                numerator: 4,
                denominator: 0
            }),
            None
        );
        assert_eq!(
            cells_per_bar(Meter {
                numerator: 0,
                denominator: 4
            }),
            None
        );
        assert_eq!(
            cells_per_bar(Meter {
                numerator: 4,
                denominator: 3
            }),
            None
        );
    }

    #[test]
    fn metric_weight_ranks_the_binary_hierarchy() {
        assert_eq!(metric_weight(0), MAX_METRIC_WEIGHT);
        assert_eq!(metric_weight(1), 0);
        assert_eq!(metric_weight(2), 1);
        assert_eq!(metric_weight(4), 2);
        assert_eq!(metric_weight(8), 3);
        assert_eq!(metric_weight(16), 4);
        assert_eq!(metric_weight(256), MAX_METRIC_WEIGHT);
        // 重量单调：把格点减半只会更强（除 0 之外）。
        for cell in [2u32, 6, 10, 12, 14, 28, 32] {
            assert!(
                metric_weight(cell) > metric_weight(cell / 2),
                "cell {cell} should be weaker than its half"
            );
        }
    }

    #[test]
    fn four_four_takes_the_metric_hierarchy_in_order() {
        let grid = metric_grid(COMMON, 1, 4).unwrap();
        // 16 格里的重量序：0 → 8、8 → 3、4/12 → 2、2/6/10/14 → 1、奇数 → 0。
        // 前 4 名 = 格点 0、8、4、12（后两者同重量，按格点升序）⇒ 排好后 0/4/8/12。
        let ticks: Vec<u64> = grid.hits().iter().map(|hit| hit.tick).collect();
        assert_eq!(ticks, vec![0, 960, 1920, 2880]);
        let cells: Vec<u32> = grid.hits().iter().map(|hit| hit.cell).collect();
        assert_eq!(cells, vec![0, 4, 8, 12]);
        let weights: Vec<u8> = grid.hits().iter().map(|hit| hit.weight).collect();
        assert_eq!(weights, vec![MAX_METRIC_WEIGHT, 2, 3, 2]);
        assert_eq!(grid.len(), 4);
        assert_eq!(grid.total_ticks(), 3840);
        assert_eq!(grid.ticks_per_bar(), 3840);
        assert_eq!(grid.swing_permille(), None);
    }

    #[test]
    fn one_onset_per_bar_is_always_the_downbeat() {
        for meter in [
            COMMON,
            Meter::WALTZ,
            Meter::COMPOUND_DUPLE,
            Meter::SEVEN_EIGHT,
        ] {
            let grid = metric_grid(meter, 3, 1).unwrap();
            assert_eq!(grid.len(), 3);
            for (index, hit) in grid.hits().iter().enumerate() {
                assert_eq!(hit.bar, index as u32);
                assert_eq!(hit.cell, 0);
                assert_eq!(hit.weight, MAX_METRIC_WEIGHT);
                assert_eq!(hit.tick, index as u64 * meter.ticks_per_bar());
            }
        }
    }

    #[test]
    fn every_bar_gets_exactly_the_requested_number_of_onsets() {
        let grid = metric_grid(Meter::SEVEN_EIGHT, 4, 5).unwrap();
        assert_eq!(grid.len(), 20);
        for bar in 0..4u32 {
            let slice = grid.hits_in_bar(bar);
            assert_eq!(slice.len(), 5, "bar {bar}");
            assert!(slice.iter().all(|hit| hit.bar == bar));
        }
        assert!(grid.hits_in_bar(4).is_empty());
        assert!(grid.hits_in_bar(u32::MAX).is_empty());
    }

    #[test]
    fn onsets_never_leave_their_own_bar() {
        for meter in [
            COMMON,
            Meter::WALTZ,
            Meter::MARCH,
            Meter::COMPOUND_DUPLE,
            Meter::SEVEN_EIGHT,
            Meter::QUINTUPLE,
        ] {
            let cells = cells_per_bar(meter).unwrap();
            let onsets = u32::try_from(cells).unwrap();
            let grid = metric_grid(meter, 3, onsets).unwrap();
            let bar_ticks = meter.ticks_per_bar();
            for hit in grid.hits() {
                let bar_start = u64::from(hit.bar) * bar_ticks;
                assert!(
                    hit.tick >= bar_start && hit.tick < bar_start + bar_ticks,
                    "meter {meter:?} hit {hit:?}"
                );
            }
        }
    }

    #[test]
    fn straight_swing_is_bit_identical_to_no_swing() {
        let plain = metric_grid(COMMON, 2, 9).unwrap();
        let straight = swung_metric_grid(COMMON, 2, 9, Some(SWING_PERMILLE_STRAIGHT)).unwrap();
        assert_eq!(plain.hits(), straight.hits());
        assert_eq!(
            straight
                .hits()
                .iter()
                .map(|hit| hit.tick)
                .collect::<Vec<_>>(),
            plain.hits().iter().map(|hit| hit.tick).collect::<Vec<_>>()
        );
        // 只有回显的入参不同：`None` vs `Some(500)`。
        assert_eq!(plain.swing_permille(), None);
        assert_eq!(straight.swing_permille(), Some(SWING_PERMILLE_STRAIGHT));
    }

    #[test]
    fn swing_postpones_only_the_second_half_of_a_pair() {
        // 4/4、每小节 9 个 onset ⇒ 前 8 名是偶数格点，第 9 名是格点 1（重量 0）。
        let grid = swung_metric_grid(COMMON, 1, 9, Some(667)).unwrap();
        let by_cell = |cell: u32| {
            grid.hits()
                .iter()
                .find(|hit| hit.cell == cell)
                .unwrap()
                .tick
        };
        // 前半格点不动。
        assert_eq!(by_cell(0), 0);
        assert_eq!(by_cell(8), 1920);
        // 后半格点 1 落在对 [0, 480) 里：480 * 667 / 1000 = 320（整除）。
        assert_eq!(by_cell(1), 320);
        assert_eq!(
            by_cell(1),
            crate::swing::swung_pair_span(SWING_PAIR_TICKS, 667)
                .unwrap()
                .first
        );
    }

    #[test]
    fn maximum_swing_never_collapses_two_cells_onto_one_tick() {
        // 这条判据在改用"切分"之前是红的：`quantize_onset(240, 480, 1000)` 的
        // 平局规则返回 0，于是格点 1 与格点 0 重合（网格丢了一个 onset）。
        let grid = swung_metric_grid(COMMON, 1, 16, Some(SWING_PERMILLE_MAX)).unwrap();
        let ticks: Vec<u64> = grid.hits().iter().map(|hit| hit.tick).collect();
        assert_eq!(grid.len(), 16);
        // 后半格点落在对尾之前一 tick：0+479、480+479、960+479 ……
        assert_eq!(ticks[0], 0);
        assert_eq!(ticks[1], SWING_PAIR_TICKS - 1);
        assert_eq!(ticks[2], SWING_PAIR_TICKS);
        assert_eq!(ticks[3], SWING_PAIR_TICKS + SWING_PAIR_TICKS - 1);
        // 全部 16 个 tick 两两不同。
        let mut sorted = ticks.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 16, "ticks: {ticks:?}");
    }

    #[test]
    fn swing_keeps_the_grid_strictly_ascending_and_inside_the_bars() {
        for meter in [
            COMMON,
            Meter::WALTZ,
            Meter::MARCH,
            Meter::COMPOUND_DUPLE,
            Meter::SEVEN_EIGHT,
            Meter::QUINTUPLE,
            Meter::new(3, 16).unwrap(),
            Meter::new(1, 16).unwrap(),
        ] {
            let cells = cells_per_bar(meter).unwrap();
            let onsets = u32::try_from(cells).unwrap();
            let bar_ticks = meter.ticks_per_bar();
            for permille in [
                SWING_PERMILLE_STRAIGHT,
                540,
                558,
                600,
                660,
                667,
                750,
                SWING_PERMILLE_MAX,
            ] {
                let grid = swung_metric_grid(meter, 4, onsets, Some(permille)).unwrap();
                assert_eq!(grid.len(), 4 * onsets as usize);
                for pair in grid.hits().windows(2) {
                    assert!(
                        pair[0].tick < pair[1].tick,
                        "meter {meter:?} permille {permille}: {pair:?}"
                    );
                }
                for hit in grid.hits() {
                    let bar_start = u64::from(hit.bar) * bar_ticks;
                    assert!(
                        hit.tick >= bar_start && hit.tick < bar_start + bar_ticks,
                        "meter {meter:?} permille {permille} hit {hit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_grid_is_a_pure_function_of_its_inputs() {
        let first = swung_metric_grid(Meter::SEVEN_EIGHT, 5, 6, Some(660)).unwrap();
        let second = swung_metric_grid(Meter::SEVEN_EIGHT, 5, 6, Some(660)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn zero_onsets_per_bar_is_an_empty_grid_not_an_error() {
        let grid = metric_grid(COMMON, 2, 0).unwrap();
        assert!(grid.is_empty());
        assert_eq!(grid.len(), 0);
        assert!(grid.hits_in_bar(0).is_empty());
        assert_eq!(grid.total_ticks(), 2 * 3840);
    }

    #[test]
    fn zero_bars_is_an_error() {
        assert_eq!(
            metric_grid(COMMON, 0, 4).unwrap_err(),
            TheoryError::ZeroBars
        );
        assert_eq!(
            swung_metric_grid(COMMON, 0, 4, Some(660)).unwrap_err(),
            TheoryError::ZeroBars
        );
    }

    #[test]
    fn an_over_dense_request_is_an_error_not_a_silent_clamp() {
        // 4/4 只有 16 个格位；17 个 onset/小节放不下。
        assert_eq!(
            metric_grid(COMMON, 1, 17).unwrap_err(),
            TheoryError::ProgressionTooDense {
                degrees: 17,
                slots: 16
            }
        );
        // 刚好放满 = 合法。
        assert_eq!(metric_grid(COMMON, 1, 16).unwrap().len(), 16);
    }

    #[test]
    fn invalid_meters_and_swing_ratios_are_rejected_without_panicking() {
        assert_eq!(
            metric_grid(
                Meter {
                    numerator: 4,
                    denominator: 0
                },
                1,
                1
            )
            .unwrap_err(),
            TheoryError::ZeroBars
        );
        assert_eq!(
            metric_grid(
                Meter {
                    numerator: 4,
                    denominator: 3
                },
                1,
                1
            )
            .unwrap_err(),
            TheoryError::ZeroBars
        );
        for permille in [0u16, 1, 499, 1001] {
            assert_eq!(
                swung_metric_grid(COMMON, 1, 2, Some(permille)).unwrap_err(),
                TheoryError::SwingOutOfRange { value: permille }
            );
        }
    }

    #[test]
    fn genre_rhythm_grid_consumes_the_registered_meter_and_swing() {
        use crate::genre::GenreLibrary;

        // `jazz_swing`：4/4、swing = 66.0 ⇒ 660 千分比。
        let swing = GenreLibrary::get("jazz_swing").unwrap();
        let grid = swing.rhythm_grid(1, 9).unwrap();
        assert_eq!(grid.meter(), COMMON);
        assert_eq!(grid.swing_permille(), Some(660));
        let offbeat = grid
            .hits()
            .iter()
            .find(|hit| hit.cell == 1)
            .expect("9 onsets select the first weak sixteenth");
        // 480 * 660 / 1000 = 316（整除）。
        assert_eq!(offbeat.tick, 316);

        // `gregorian_chant`：swing = None ⇒ 平直，后半格点留在 240。
        let straight = GenreLibrary::get("gregorian_chant").unwrap();
        let grid = straight.rhythm_grid(1, 9).unwrap();
        assert_eq!(grid.swing_permille(), None);
        let offbeat = grid.hits().iter().find(|hit| hit.cell == 1).unwrap();
        assert_eq!(offbeat.tick, 240);
    }

    #[test]
    fn genre_rhythm_grid_uses_the_genres_own_meter() {
        use crate::genre::GenreLibrary;

        // `waltz` 是 3/4 ⇒ 每小节 12 个格位、每小节 2880 tick。
        let waltz = GenreLibrary::get("waltz").unwrap();
        assert_eq!(waltz.meter, (3, 4));
        let grid = waltz.rhythm_grid(2, 4).unwrap();
        assert_eq!(grid.ticks_per_bar(), 2880);
        assert_eq!(grid.total_ticks(), 5760);
        assert_eq!(grid.hits_in_bar(1)[0].tick, 2880);
    }
}
