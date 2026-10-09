//! 鼓组型 (drum pattern)：把**已经存在的** [`MetricGrid`] 读成一份确定性的鼓点网格。
//!
//! ## 为什么需要这个模块
//!
//! `docs/ledger/theory-core-notes.md` 的 `pending 3` 记着：本 crate
//! "没有实现节奏型/鼓组 pattern：`GenreRule` 只有密度提示与摇摆比例，
//! 没有具体的鼓点网格"。`crate::rhythm` 已经补上**网格**那一半（每个 onset
//! 落在哪个 tick）。本模块补上**可用性**那一半：同一份 onset 网格按**度量重量**
//! 与**拍分组**分派到鼓件，产出一份可以逐件读回的鼓组型。
//!
//! ## 本模块**不新增任何登记数据**（这是它与"逐流派鼓点型"的分界）
//!
//! 全部击点都从 [`MetricGrid`] 既有的 `hits`（`tick` / `bar` / `cell` /
//! `weight`）与**调用方传入**的参数（分组、反拍位置、摇摆千分比）导出。
//! 本模块没有鼓件权重表、没有流派鼓点表、没有"每小节打几下"的常量表
//! —— 那需要逐流派登记数据，本 crate 无权发明（理由与 `onsets_per_bar`
//! 同源，见 [`crate::rhythm`] 的模块文档）。因此 `pending 3` 剩下的
//! "逐流派的鼓点型数据"**仍未**登记；本模块只把已有网格变成鼓点。
//!
//! ## 语义（全部是整数运算）
//!
//! 1. **击点集合 = 网格的 onset 集合**。本模块不新增 tick、不移动 tick。
//!    每个击点的 `tick` / `bar` / `cell` / `weight` 逐位取自对应的
//!    [`crate::rhythm::GridHit`]。因此摇摆位移（见 [`crate::rhythm::swung_metric_grid`]）
//!    自动生效，本模块**不**再算一次摇摆。
//! 2. **拍**。每小节的格点被分成 [`felt_beats_per_bar`] 个拍，每拍
//!    `cells_per_bar / 拍数` 个格点（与 [`metric_weight_grouped`] 同一除法）。
//!    一格的"拍内偏移"是 `cell % 每拍格位数`；偏移为 0 即"拍的起点"。
//! 3. **拍分组**。分组把拍切成若干组，组起点 = 组的第 0 拍。
//!
//!    - **调用方给了** [`BeatGrouping`]（5/4 = `[3, 2]`、7/8 = `[2, 2, 3]`）：
//!      组起点就是分组自己判定的那些拍；重量取 [`metric_weight_in`] 与
//!      [`metric_weight_grouped`] 的**较大者**（分组只允许加重某几拍，见
//!      [`crate::rhythm::BeatGrouping`] 的"与内置口径的关系"）。
//!    - **没给**：组起点由**拍号自身的度量层级**导出（不是发明数据）：
//!      第 `b` 拍是组起点当且仅当
//!      `metric_weight_in(meter, b × 每拍格位数) >= STRONG_BEAT_WEIGHT`
//!      —— 即"拍号自己判成强位的那些拍"。4/4 ⇒ 第 0、2 拍；3/4 ⇒ 第 0 拍；
//!      7/8 ⇒ 第 0 拍；6/8（复合二拍）⇒ 第 0、1 拍。缺省口径下重量就是
//!      [`metric_weight_in`]，与 [`crate::rhythm::metric_grid`] 的选点同源。
//! 4. **分派规则**（底鼓与军鼓都只落在**拍的起点**上：偏移为 0）：
//!
//!    | 鼓件 | 条件 |
//!    | :--- | :--- |
//!    | [`DrumVoice::Kick`] | 该 onset 是**组起点**（拍的起点，且该拍是强位拍／组的第 0 拍） |
//!    | [`DrumVoice::Snare`] | 该 onset 是拍的起点，且其**拍序号**能被 `backbeat` 整除 |
//!    | [`DrumVoice::HiHat`] | 网格里**每一个** onset |
//!    | [`DrumVoice::Ride`] | 组起点上**同时**是强位（`weight >= STRONG_BEAT_WEIGHT`）时，叠加在底鼓之上 |
//!
//!    "拍的起点"这一条**不能省**：只用"重量 ≥ 强位"会把强拍内部的每一格都算成
//!    组起点（4/4 因此得到 8 个军鼓而不是 2 个 —— 这条退化由判据
//!    `four_four_full_grid_gives_the_textbook_backbeat` 与 [`is_meter_group_start`]
//!    的文档钉住）。
//!
//!    `accent` = `weight >= STRONG_BEAT_WEIGHT`（强位加重的记号，与
//!    [`STRONG_BEAT_WEIGHT`] 同源）。
//! 5. **反拍位置 `backbeat`**。单位是"拍"，第 0 基，`1..=拍数`。
//!    军鼓打**拍的起点里拍序号能被 `backbeat` 整除**的那些（不是只打一拍）：
//!    `4/4 + backbeat = 2` ⇒ 第 0、2 拍；`6/8 + backbeat = 1` ⇒ 每一拍
//!    （复合拍的通行配器）；`3/4 + backbeat = 2` ⇒ 第 0、2 拍（中间那拍）。
//!    [`default_backbeat`] 给出缺省值：拍数为偶数取其中点，为奇数取中间的拍。
//!    `backbeat` 为 0 或大于拍数时 [`swung_drum_pattern`] 返回 `Ok(None)`
//!    —— **不**钳制、**不**静默改成 1（与 [`crate::swing::validate_swing_permille`]
//!    的口径一致）。
//! 6. **恒有**（逐条有判据）：
//!
//!    - `hits` 按 `(tick, tick 内固定鼓件序)` 严格排列，同一 tick 内鼓件序固定
//!      为 `Kick < Snare < HiHat < Ride`；
//!    - 同一 tick 上**同一鼓件**最多一个击点（"底鼓 + 踩镲"这种同 tick 组合合法，
//!      "两个底鼓"不合法）；
//!    - 每个击点的 `(tick, bar, cell, weight)` 都能在网格里找到**逐位相同**的
//!      onset（本模块不发明 tick，也不改重量）；`beat` 等于由 `cell` 与拍长
//!      算出的拍序号；
//!    - 全部击点落在 `0..total_ticks` 内；`total_ticks == 网格的 total_ticks`；
//!    - 同输入同输出（跨进程跨平台逐位一致）；`onsets_per_bar` 不同时，
//!      两份鼓组型的击点是各自网格的子集。
//!
//! ## 边界（音频红线）
//!
//! - **构造期分配，逐样本零分配**：构造函数分配一个 `Vec`（`Σ 每 onset 的鼓件数`，
//!   上界 `4 × grid.len()`），因此**不得**在音频线程上调用。读侧
//!   （[`DrumPattern::hits`]、[`DrumPattern::hits_in_bar`]）只返回切片或计数：
//!   不分配、不加锁、不做阻塞 I/O、不打日志。
//! - 逐 onset 的判定用**栈上定长数组**（[`DRUM_VOICE_COUNT`] 个元素），
//!   因此循环体内零堆分配。
//! - 本模块不含 `HashMap` / `HashSet` [MODEL-AST-003 / AGENTS.md 红线 4]：
//!   唯一的容器是结果 `Vec`，顺序由构造过程唯一确定。
//! - 本模块不读时钟、不做 I/O、不打印；输出只由入参决定。

use crate::error::TheoryError;
use crate::progression::Meter;
use crate::rhythm::{
    BeatGrouping, GridHit, MetricGrid, STRONG_BEAT_WEIGHT, WeightedGrid, cells_per_bar,
    felt_beats_per_bar, metric_weight_grouped, metric_weight_in,
};

/// 本模块分派的鼓件种数。
///
/// 顺序就是"同一 tick 内的固定鼓件序"（见模块文档规则 6）：底鼓 → 军鼓 →
/// 踩镲 → 吊镲。全部判定用 `[_; DRUM_VOICE_COUNT]` 的定长数组，不用集合类型。
pub const DRUM_VOICE_COUNT: usize = 4;

/// 一个鼓件。
///
/// 只有四件：这是**可以只用既有网格数据分派**的最小集合。真正的编曲还要
/// 嗵鼓、牛铃、拍手等，但它们的分派需要逐流派的配器数据（本模块不发明，
/// 见模块文档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrumVoice {
    /// 底鼓（大鼓）：组起点。
    Kick,
    /// 军鼓：反拍拍位。
    Snare,
    /// 踩镲：每一个 onset。
    HiHat,
    /// 吊镲：强位上的组起点（叠加在底鼓之上）。
    Ride,
}

impl DrumVoice {
    /// 全部鼓件，按"同一 tick 内的固定鼓件序"排列。
    pub const ALL: [Self; DRUM_VOICE_COUNT] = [Self::Kick, Self::Snare, Self::HiHat, Self::Ride];

    /// 该鼓件在 [`DrumVoice::ALL`] 里的下标（`0..DRUM_VOICE_COUNT`）。
    ///
    /// 这是 `usize` → 鼓件的**唯一**映射：数组下标只经由本函数产生，
    /// 因此"枚举序"与"数组下标"不可能漂移。
    #[must_use]
    pub const fn ordinal(self) -> usize {
        match self {
            Self::Kick => 0,
            Self::Snare => 1,
            Self::HiHat => 2,
            Self::Ride => 3,
        }
    }

    /// 稳定的英文名（ASCII 小写）；用于日志与错误文本，不参与判定。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kick => "kick",
            Self::Snare => "snare",
            Self::HiHat => "hihat",
            Self::Ride => "ride",
        }
    }

    /// 逐帧可播的通用 MIDI 打击乐音高（GM 通道 10 的键号）。
    ///
    /// 36 = Bass Drum 1、38 = Acoustic Snare、42 = Closed Hi-Hat、
    /// 51 = Ride Cymbal 1（都是通用 MIDI 打击乐映射里的公有领域事实）。
    /// 本类型**不**依赖任何 MIDI crate：它只给一个 `u8`，由调用方决定怎么用。
    #[must_use]
    pub const fn gm_key(self) -> u8 {
        match self {
            Self::Kick => 36,
            Self::Snare => 38,
            Self::HiHat => 42,
            Self::Ride => 51,
        }
    }
}

/// 鼓组型里的一个击点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrumHit {
    /// 被击的鼓件。
    pub voice: DrumVoice,
    /// 绝对 tick（逐位取自网格 onset，含摇摆位移）。
    pub tick: u64,
    /// 所在小节（0 基）。
    pub bar: u32,
    /// 小节内的格点下标（0 基）。
    pub cell: u32,
    /// 拍序号（0 基，`felt_beats_per_bar` 口径）。
    pub beat: u8,
    /// 该格点的度量重量（逐位取自网格 onset）。
    pub weight: u8,
    /// 该格点是否在强位上（`weight >= STRONG_BEAT_WEIGHT`）。
    pub accent: bool,
}

/// 反拍（军鼓）位置：拍序号 `1..=拍数`，第 0 基。
///
/// 见模块文档规则 5。类型别名只表达单位，运行时校验在
/// [`swung_drum_pattern`]（越界返回 `Ok(None)`）。
pub type Backbeat = u8;

/// 缺省反拍位置：拍数为偶数取中点，为奇数取中间的拍。
///
/// 单位是"拍"（第 0 基）。`4/4` ⇒ 2、`6/8`（2 拍）⇒ 1、`3/4`（3 拍）⇒ 2
/// —— 它是"最靠近中点的**非零**拍"，因此恒落在 `1..=拍数` 内（拍数恒 ≥ 1，
/// 见 [`crate::rhythm::metric_weight_in`] 的"拍数恒不为 0"）。
#[must_use]
pub const fn default_backbeat(meter: Meter) -> Backbeat {
    let beats = felt_beats_per_bar(meter) as u32;
    if beats <= 1 {
        return 1;
    }
    if beats.is_multiple_of(2) {
        (beats / 2) as u8
    } else {
        (beats / 2 + 1) as u8
    }
}

/// 拍号**自身**的度量层级判定的"这一格是不是组的起点"。
///
/// 单位是**格点**（第 0 基，即 onset 的 `cell`），不是拍序号：`cells_per_beat`
/// 是每拍的格点数（`cells_per_bar / felt_beats_per_bar`）。判据有两条：
///
/// 1. 这一格是它所在**拍的起点**（`cell % cells_per_beat == 0`）；
/// 2. 该拍的 [`metric_weight_in`] ≥ [`STRONG_BEAT_WEIGHT`]。
///
/// 两个条件缺一不可：只用条件 2 会把强拍**内部**的每一格都算成组起点
/// （3/4 的格点 1、2、3 都属于第 0 拍，于是底鼓会在每格响一次 —— 这不是
/// "组起点"，是"强拍"）。重量读数就是 [`crate::rhythm::metric_grid`] 选点用的
/// 同一份重量表，因此缺省分组与网格口径**同源**，不是第二份"哪些拍重要"的知识。
///
/// `cells_per_beat == 0`（每小节不足一拍）、拍号非法或 `cell` 越出小节时
/// 返回 `false`（不 panic）。
#[must_use]
pub const fn is_meter_group_start(meter: Meter, cell: u32, cells_per_beat: u64) -> bool {
    if cells_per_beat == 0 {
        return false;
    }
    let Some(cells) = cells_per_bar(meter) else {
        return false;
    };
    if cell as u64 >= cells {
        return false;
    }
    if !(cell as u64).is_multiple_of(cells_per_beat) {
        return false;
    }
    metric_weight_in(meter, cell) >= STRONG_BEAT_WEIGHT
}

/// 鼓组型：对某一份 [`MetricGrid`] 的鼓件分派结果。
///
/// `hits` 恒按 `(tick, DrumVoice::ordinal)` 严格升序（见模块文档规则 6）。
/// 本类型不复制网格的 `meter` / `bars` / `onsets_per_bar`，因此**不存在**
/// "鼓组型与它来自的网格漂移"这条失败模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumPattern {
    grid: MetricGrid,
    hits: Vec<DrumHit>,
}

impl DrumPattern {
    /// 全部击点，按 `(tick, 鼓件序)` 严格升序。
    #[must_use]
    pub fn hits(&self) -> &[DrumHit] {
        &self.hits
    }

    /// 击点总数（`0` 表示网格一个 onset 都没有）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// 击点总数为 0。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// 该鼓件的击点个数（不分配、不加锁、不做 I/O）。
    #[must_use]
    pub fn hit_count(&self, voice: DrumVoice) -> usize {
        self.hits.iter().filter(|hit| hit.voice == voice).count()
    }

    /// 它读的那份网格（只读借用）。
    ///
    /// 判据用它与网格对账；生产路径通常不需要它。
    #[must_use]
    pub const fn grid(&self) -> &MetricGrid {
        &self.grid
    }

    /// 拍号（转发网格）。
    #[must_use]
    pub const fn meter(&self) -> Meter {
        self.grid.meter()
    }

    /// 小节数（转发网格）。
    #[must_use]
    pub const fn bars(&self) -> u32 {
        self.grid.bars()
    }

    /// 每小节的 onset 数（转发网格）。
    #[must_use]
    pub const fn onsets_per_bar(&self) -> u32 {
        self.grid.onsets_per_bar()
    }

    /// 每小节 tick 数（转发网格）。
    #[must_use]
    pub const fn ticks_per_bar(&self) -> u64 {
        self.grid.ticks_per_bar()
    }

    /// 覆盖的总 tick 数（恒等于网格的 `total_ticks`）。
    #[must_use]
    pub const fn total_ticks(&self) -> u64 {
        self.grid.total_ticks()
    }

    /// 第 `bar` 小节的击点切片；`bar >= bars` 时返回空切片。
    ///
    /// 击点按 `(tick, 鼓件序)` 升序且**只含该小节**的 tick，因此每个小节的
    /// 切片可以直接按顺序拼成整条轨道。不分配、不加锁、不做 I/O。
    #[must_use]
    pub fn hits_in_bar(&self, bar: u32) -> &[DrumHit] {
        if bar >= self.grid.bars() {
            return &[];
        }
        let start = self.hits.partition_point(|hit| hit.bar < bar);
        let end = self.hits.partition_point(|hit| hit.bar <= bar);
        &self.hits[start..end]
    }
}

/// 平直鼓组型：`permille == None` 的 [`swung_drum_pattern`]。
///
/// ```
/// use yeban_theory::drum::{DrumVoice, drum_pattern};
/// use yeban_theory::progression::Meter;
///
/// // 4/4、一个小节、16 个 onset、缺省分组、反拍在第 2 拍。
/// let pattern = drum_pattern(Meter::COMMON, 1, 16, None, 2).unwrap().unwrap();
/// assert_eq!(pattern.hit_count(DrumVoice::HiHat), 16); // 每格一击
/// assert_eq!(pattern.hit_count(DrumVoice::Kick), 2);   // 两个强位拍
/// assert_eq!(pattern.hit_count(DrumVoice::Snare), 2);  // 反拍在第 0、2 拍
/// // 反拍位置越界（4/4 只有 4 拍）⇒ `Ok(None)`，不钳制。
/// assert!(drum_pattern(Meter::COMMON, 1, 16, None, 5).unwrap().is_none());
/// ```
///
/// # Errors
///
/// 与 [`crate::rhythm::grouped_swung_metric_grid`] 逐条相同（`bars == 0` ⇒
/// [`TheoryError::ZeroBars`]、拍号非法 ⇒ [`TheoryError::ZeroBars`]、
/// `onsets_per_bar` 超过格位数 ⇒ [`TheoryError::ProgressionTooDense`]）。
pub fn drum_pattern(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    grouping: Option<BeatGrouping<'_>>,
    backbeat: Backbeat,
) -> Result<Option<DrumPattern>, TheoryError> {
    swung_drum_pattern(meter, bars, onsets_per_bar, None, grouping, backbeat)
}

/// 摇摆鼓组型：先按 `grouping` 造网格（缺省走拍号口径），再把 onset 分派到鼓件。
///
/// 返回 `Ok(None)` 表示**参数自身**不合法（`backbeat` 为 0 或大于拍数，
/// 或每小节不足一拍）：这不是 [`TheoryError`] 的变体，因为它是调用方可以
/// 自查的输入约束，与 [`BeatGrouping::new`] / [`cells_per_bar`] 同口径
/// （本 crate **不新增**错误变体，见 [`crate::rhythm::BeatGrouping`] 的文档）。
///
/// # Errors
///
/// 与 [`crate::rhythm::grouped_swung_metric_grid`] 逐条相同。
pub fn swung_drum_pattern(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    grouping: Option<BeatGrouping<'_>>,
    backbeat: Backbeat,
) -> Result<Option<DrumPattern>, TheoryError> {
    match grouping {
        None => place_voices(
            meter,
            bars,
            onsets_per_bar,
            permille,
            backbeat,
            |cell| metric_weight_in(meter, cell),
            // 缺省分组 = 拍号自身的度量层级（见 [`is_meter_group_start`]）。
            |cell| is_meter_group_start(meter, cell, cells_per_beat_of(&meter)),
        ),
        Some(grouping) => {
            place_grouped_voices(meter, bars, onsets_per_bar, permille, backbeat, grouping)
        }
    }
}

/// [`swung_drum_pattern`] 的**显式分组**分支。
///
/// 与缺省分支的唯一差别是"重量/组起点"这一对读数：
/// [`crate::rhythm::metric_weight_grouped`] 把**组起点**判成
/// [`STRONG_BEAT_WEIGHT`]，而组起点不一定是拍的起点（5/4 的 `[3, 2]` 把第 3 拍
/// 判成组起点，它的格点偏移是 0；7/8 的 `[2, 2, 3]` 同理），因此这里用
/// [`crate::rhythm::metric_weight_in`] 补上"拍号口径里这一格自己的重量"
/// —— 两者取较大者，再交给同一份 [`place_voices`] 分派。
///
/// 这条"取较大者"不是发明数据：分组只允许**加重**某几拍
/// （见 [`crate::rhythm::BeatGrouping`] 的"与内置口径的关系"），因此结果是
/// "拍号口径 ∨ 分组口径"，两个读数都来自本 crate 已有的公开函数。
fn place_grouped_voices(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    backbeat: Backbeat,
    grouping: BeatGrouping<'_>,
) -> Result<Option<DrumPattern>, TheoryError> {
    let cells_per_beat = cells_per_beat_of(&meter);
    let grouped_weight = |cell: u32| metric_weight_grouped(meter, cell, grouping);
    let rule = |cell: u32| metric_weight_in(meter, cell).max(grouped_weight(cell));
    // 组起点判定**只信调用方的分组**（缺省口径在这里不适用：分组的组起点
    // 不一定是拍号口径里的强位拍 —— 5/4 的 [3, 2] 把第 3 拍判成组起点，
    // 而拍号口径给它 BEAT_WEIGHT）。
    place_voices(
        meter,
        bars,
        onsets_per_bar,
        permille,
        backbeat,
        rule,
        |cell| {
            if cells_per_beat == 0 {
                return false;
            }
            let offset = u64::from(cell) % cells_per_beat;
            let beat = u64::from(cell) / cells_per_beat;
            offset == 0
                && beat < u64::from(felt_beats_per_bar(meter))
                && grouping.is_group_start(beat as u8)
        },
    )
}

/// 每拍的格点数：`cells_per_bar / felt_beats_per_bar`（单位是格）。
///
/// 拍号非法、每小节不足一拍或拍数为 0 时返回 0（调用方据此走 `false`/`None`
/// 分支，不 panic）。
fn cells_per_beat_of(meter: &Meter) -> u64 {
    let beats = felt_beats_per_bar(*meter);
    if beats == 0 {
        return 0;
    }
    match cells_per_bar(*meter) {
        Some(cells) => cells / u64::from(beats),
        None => 0,
    }
}

/// 单件鼓件的判定：`(底鼓, 军鼓, 踩镲, 吊镲)`，下标同 [`DrumVoice::ordinal`]。
///
/// `offset_in_beat` 是这一格在本拍内的偏移（格）；底鼓与军鼓都要求
/// `offset_in_beat == 0`（即**只落在拍的起点**上）。少了这条会退化，见
/// `is_meter_group_start` 的文档与测试
/// `four_four_full_grid_gives_the_textbook_backbeat`：
/// 3/4 的格点 1、2、3 都属于第 0 拍，只看"拍序号能被反拍整除"会让军鼓
/// 在强拍内部每一格都响（4/4 因此得到 8 个军鼓而不是 2 个）。
fn voices_at(
    backbeat: Backbeat,
    beat: u8,
    offset_in_beat: u64,
    is_group_start: bool,
    accent: bool,
) -> [bool; 4] {
    let on_beat_start = offset_in_beat == 0;
    let backbeat_hit = on_beat_start && backbeat != 0 && beat.is_multiple_of(backbeat);
    [
        on_beat_start && is_group_start,
        backbeat_hit,
        true,
        on_beat_start && is_group_start && accent,
    ]
}

/// 把 `rule` 读出的重量灌进网格，再把 onset 分派到鼓件。
///
/// 这是公开入口的**唯一**实现：网格由
/// [`crate::rhythm::build_weighted_grid`] 造 —— 分派与网格共用**同一份**
/// 选点读数，因此不可能对"重心在哪"有两种读法。
fn place_voices(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    backbeat: Backbeat,
    rule: impl Fn(u32) -> u8,
    group_start: impl Fn(u32) -> bool,
) -> Result<Option<DrumPattern>, TheoryError> {
    let beats = felt_beats_per_bar(meter);
    if backbeat == 0 || backbeat > beats {
        // 参数自身不合法：先于造网格就返回，免得白分配一个 `Vec`。
        return Ok(None);
    }
    let Some(cells) = cells_per_bar(meter) else {
        // 拍号非法：网格构造器会报同一个错误（口径只有一处）。
        return Ok(None);
    };
    let cells_per_beat = cells / u64::from(beats);
    if cells_per_beat == 0 {
        return Ok(None);
    }
    let weighted: WeightedGrid =
        crate::rhythm::build_weighted_grid(meter, bars, onsets_per_bar, permille, rule)?;
    let hits = dispatch(
        weighted.grid(),
        weighted.selected(),
        backbeat,
        cells_per_beat,
        group_start,
    );
    debug_assert!(
        hits.iter().all(|hit| hit.beat < beats),
        "every hit must land on a felt beat"
    );
    Ok(Some(DrumPattern {
        grid: weighted.grid().clone(),
        hits,
    }))
}

/// 分派本身：`is_group_start(cell)` 给出"这一格是不是组的起点"。
///
/// `selected` 与 `grid.hits()` 逐位相同（由
/// [`crate::rhythm::build_weighted_grid`] 同时产出），因此这里不倒推 tick。
fn dispatch(
    grid: &MetricGrid,
    selected: &[GridHit],
    backbeat: Backbeat,
    cells_per_beat: u64,
    is_group_start: impl Fn(u32) -> bool,
) -> Vec<DrumHit> {
    debug_assert_eq!(grid.hits(), selected);
    let mut hits = Vec::with_capacity(grid.len());
    for hit in grid.hits() {
        let offset_in_beat = u64::from(hit.cell) % cells_per_beat;
        let beat = (u64::from(hit.cell) / cells_per_beat) as u8;
        let accent = hit.weight >= STRONG_BEAT_WEIGHT;
        let flags = voices_at(
            backbeat,
            beat,
            offset_in_beat,
            is_group_start(hit.cell),
            accent,
        );
        for voice in DrumVoice::ALL {
            if flags[voice.ordinal()] {
                hits.push(DrumHit {
                    voice,
                    tick: hit.tick,
                    bar: hit.bar,
                    cell: hit.cell,
                    beat,
                    weight: hit.weight,
                    accent,
                });
            }
        }
    }
    debug_assert!(
        hits.windows(2)
            .all(|pair| (pair[0].tick, pair[0].voice) < (pair[1].tick, pair[1].voice)),
        "drum hits must be strictly ascending in (tick, voice)"
    );
    debug_assert!(
        hits.windows(2)
            .all(|pair| pair[0].tick != pair[1].tick || pair[0].voice != pair[1].voice),
        "one voice must not hit the same tick twice"
    );
    debug_assert!(
        hits.windows(2).all(|pair| pair[0].bar <= pair[1].bar),
        "hits must stay grouped by bar"
    );
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progression::PPQ;
    use crate::rhythm::{GRID_CELL_TICKS, MetricGrid, metric_weight_grouped};

    const COMMON: Meter = Meter::COMMON;

    /// 一份"全格点"网格：`onsets_per_bar` = 每小节格点数（16 分音符网格）。
    fn full_grid(meter: Meter, bars: u32) -> MetricGrid {
        let cells = cells_per_bar(meter).unwrap();
        crate::rhythm::swung_metric_grid(meter, bars, cells as u32, None).unwrap()
    }

    /// 某鼓件在 `(bar, cell)` 上的击点是否存在。
    fn has(pattern: &DrumPattern, voice: DrumVoice, bar: u32, cell: u32) -> bool {
        pattern
            .hits()
            .iter()
            .any(|hit| hit.voice == voice && hit.bar == bar && hit.cell == cell)
    }

    fn pattern(
        meter: Meter,
        bars: u32,
        onsets_per_bar: u32,
        grouping: Option<BeatGrouping<'_>>,
        backbeat: Backbeat,
    ) -> DrumPattern {
        swung_drum_pattern(meter, bars, onsets_per_bar, None, grouping, backbeat)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn voices_are_a_total_mapping_with_distinct_ordinals_names_and_keys() {
        for (index, voice) in DrumVoice::ALL.iter().enumerate() {
            assert_eq!(voice.ordinal(), index, "{}", voice.name());
        }
        let mut names = DrumVoice::ALL.map(DrumVoice::name);
        names.sort_unstable();
        for pair in names.windows(2) {
            assert_ne!(pair[0], pair[1], "duplicate voice name");
        }
        let mut keys = DrumVoice::ALL.map(DrumVoice::gm_key);
        keys.sort_unstable();
        for pair in keys.windows(2) {
            assert_ne!(pair[0], pair[1], "duplicate GM key");
        }
    }

    #[test]
    fn default_backbeat_is_the_middle_beat_and_never_zero() {
        // 拍数：2/4 → 2、3/4 → 3、4/4 → 4、5/4 → 5、6/8 → 2、7/8 → 7。
        assert_eq!(default_backbeat(Meter::MARCH), 1);
        assert_eq!(default_backbeat(Meter::WALTZ), 2);
        assert_eq!(default_backbeat(COMMON), 2);
        assert_eq!(default_backbeat(Meter::QUINTUPLE), 3);
        assert_eq!(default_backbeat(Meter::COMPOUND_DUPLE), 1);
        assert_eq!(default_backbeat(Meter::SEVEN_EIGHT), 4);
        for numerator in 1u8..=16 {
            let meter = Meter {
                numerator,
                denominator: 4,
            };
            let backbeat = default_backbeat(meter);
            assert!(
                backbeat >= 1 && backbeat <= felt_beats_per_bar(meter),
                "{meter:?} → {backbeat}"
            );
        }
    }

    #[test]
    fn meter_group_start_reads_the_meter_hierarchy_not_a_second_table() {
        // 传入的是**格点**下标，`cells_per_beat` 是每拍格点数。
        // 4/4：每拍 4 格，强位拍的**起点**在格点 0（拍 0）与 8（拍 2）。
        assert!(is_meter_group_start(COMMON, 0, 4));
        assert!(!is_meter_group_start(COMMON, 4, 4));
        assert!(is_meter_group_start(COMMON, 8, 4));
        assert!(!is_meter_group_start(COMMON, 12, 4));
        // 强拍**内部**的格点不是组起点（只有拍起点才是）。
        for cell in [1u32, 2, 3, 9, 10, 11] {
            assert!(
                !is_meter_group_start(COMMON, cell, 4),
                "cell {cell} is inside a strong beat, not a beat start"
            );
        }
        // 3/4：只有小节起点是强位。
        assert!(is_meter_group_start(Meter::WALTZ, 0, 4));
        assert!(!is_meter_group_start(Meter::WALTZ, 4, 4));
        assert!(!is_meter_group_start(Meter::WALTZ, 8, 4));
        // 6/8 是复合二拍：每拍 6 格，两拍的起点都是强位。
        assert!(is_meter_group_start(Meter::COMPOUND_DUPLE, 0, 6));
        assert!(is_meter_group_start(Meter::COMPOUND_DUPLE, 6, 6));
        assert!(!is_meter_group_start(Meter::COMPOUND_DUPLE, 3, 6));
        // 7/8：每拍 2 格，只有小节起点。
        assert!(is_meter_group_start(Meter::SEVEN_EIGHT, 0, 2));
        assert!(!is_meter_group_start(Meter::SEVEN_EIGHT, 2, 2));
        // 病态/越界输入不 panic。
        assert!(!is_meter_group_start(COMMON, 0, 0));
        assert!(!is_meter_group_start(COMMON, 1000, 4));
        assert!(!is_meter_group_start(
            Meter {
                numerator: 4,
                denominator: 0
            },
            0,
            4
        ));
    }

    #[test]
    fn every_onset_carries_a_hihat_and_hits_are_ascending_in_tick_then_voice() {
        for meter in [Meter::MARCH, Meter::WALTZ, COMMON, Meter::SEVEN_EIGHT] {
            let grid = full_grid(meter, 2);
            let pattern = pattern(meter, 2, grid.onsets_per_bar(), None, 2);
            for hit in grid.hits() {
                assert!(has(&pattern, DrumVoice::HiHat, hit.bar, hit.cell));
            }
            assert_eq!(pattern.hit_count(DrumVoice::HiHat), grid.len());
            for pair in pattern.hits().windows(2) {
                assert!(
                    (pair[0].tick, pair[0].voice.ordinal())
                        < (pair[1].tick, pair[1].voice.ordinal()),
                    "{meter:?}: {:?} then {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn four_four_full_grid_gives_the_textbook_backbeat() {
        // 4/4、16 格、backbeat = 2 ⇒ 底鼓在强位拍 0 / 2（格点 0 / 8）；
        // 军鼓在拍 0 / 2（格点 0 / 8，与底鼓同 tick）；踩镲在全部 16 格；
        // 吊镲叠加在两个强位组起点（格点 0 与 8）。
        let pattern = pattern(COMMON, 1, 16, None, 2);
        assert_eq!(pattern.hit_count(DrumVoice::Kick), 2);
        assert_eq!(pattern.hit_count(DrumVoice::Snare), 2);
        assert_eq!(pattern.hit_count(DrumVoice::HiHat), 16);
        assert_eq!(pattern.hit_count(DrumVoice::Ride), 2);
        for cell in [0u32, 8] {
            assert!(has(&pattern, DrumVoice::Kick, 0, cell), "kick {cell}");
            assert!(has(&pattern, DrumVoice::Snare, 0, cell), "snare {cell}");
        }
        for cell in 0u32..16 {
            assert!(has(&pattern, DrumVoice::HiHat, 0, cell), "hihat {cell}");
        }
        // 军鼓与底鼓都只落在**拍的起点**上：强拍内部的格点（1、2、3、9、10、11）
        // 一个都不能响 —— 这是"重量 ≥ 强位"与"是拍起点"两条条件的分离判据。
        for cell in [1u32, 2, 3, 9, 10, 11] {
            assert!(!has(&pattern, DrumVoice::Kick, 0, cell), "kick {cell}");
            assert!(!has(&pattern, DrumVoice::Snare, 0, cell), "snare {cell}");
        }
        assert!(has(&pattern, DrumVoice::Ride, 0, 0));
        assert!(has(&pattern, DrumVoice::Ride, 0, 8));
        // 底鼓的 tick 就是格点 tick：0 与 8 × 240。
        let kicks: Vec<u64> = pattern
            .hits()
            .iter()
            .filter(|hit| hit.voice == DrumVoice::Kick)
            .map(|hit| hit.tick)
            .collect();
        assert_eq!(kicks, vec![0, 8 * GRID_CELL_TICKS]);
    }

    #[test]
    fn three_four_puts_the_snare_on_the_middle_beat() {
        // 3/4、12 格、backbeat = 2 ⇒ 底鼓只在格点 0（唯一强位拍）；
        // 军鼓在拍 0 与 2（格点 0、8）。
        let pattern = pattern(Meter::WALTZ, 1, 12, None, 2);
        assert_eq!(pattern.hit_count(DrumVoice::Kick), 1);
        assert_eq!(pattern.hit_count(DrumVoice::Snare), 2);
        assert!(has(&pattern, DrumVoice::Kick, 0, 0));
        assert!(has(&pattern, DrumVoice::Snare, 0, 0));
        assert!(has(&pattern, DrumVoice::Snare, 0, 8));
        assert!(!has(&pattern, DrumVoice::Snare, 0, 4));
    }

    #[test]
    fn compound_meter_double_two_puts_the_snare_on_the_second_felt_beat() {
        // 6/8、12 格、拍数 2（复合二拍）、backbeat = 1 ⇒ 每一拍都是强位组起点，
        // 因此底鼓与军鼓都落在格点 0 与 6。
        let pattern = pattern(Meter::COMPOUND_DUPLE, 1, 12, None, 1);
        assert_eq!(felt_beats_per_bar(Meter::COMPOUND_DUPLE), 2);
        assert_eq!(pattern.hit_count(DrumVoice::Kick), 2);
        assert_eq!(pattern.hit_count(DrumVoice::Snare), 2);
        assert_eq!(pattern.hit_count(DrumVoice::Ride), 2);
        for cell in [0u32, 6] {
            assert!(has(&pattern, DrumVoice::Kick, 0, cell), "kick {cell}");
            assert!(has(&pattern, DrumVoice::Snare, 0, cell), "snare {cell}");
        }
        assert_eq!(pattern.hits()[0].beat, 0);
    }

    #[test]
    fn additive_grouping_moves_the_kick_to_the_group_starts() {
        // 5/4、20 格、只取 4 个 onset：拍号口径选 0/4/8/12（重量 8/2/2/2），
        // 其中只有格点 0 是强位 ⇒ 底鼓 1 个。分组 [3, 2] 让第 3 拍（格点 12）
        // 也变成强位 ⇒ 底鼓落在 0 与 12。
        let grouping = BeatGrouping::new(Meter::QUINTUPLE, &[3, 2]).unwrap();
        let plain = pattern(Meter::QUINTUPLE, 1, 4, None, 3);
        assert!(has(&plain, DrumVoice::Kick, 0, 0));
        assert!(!has(&plain, DrumVoice::Kick, 0, 4));
        assert!(!has(&plain, DrumVoice::Kick, 0, 12));

        let grouped = pattern(Meter::QUINTUPLE, 1, 4, Some(grouping), 3);
        assert!(has(&grouped, DrumVoice::Kick, 0, 0));
        assert!(has(&grouped, DrumVoice::Kick, 0, 12));
        assert_eq!(grouped.hit_count(DrumVoice::Kick), 2);
        // 分组口径的重量与网格的口径一致（底鼓落在组起点上）。
        assert_eq!(metric_weight_grouped(Meter::QUINTUPLE, 12, grouping), 3);
        assert_eq!(grouped.hit_count(DrumVoice::HiHat), grouped.grid().len());
        assert_eq!(grouped.hit_count(DrumVoice::HiHat), 4);
        // 击点总数 = 每件鼓件的击点数之和（不是 onset 数）。
        assert_eq!(
            grouped.len(),
            DrumVoice::ALL
                .iter()
                .map(|&voice| grouped.hit_count(voice))
                .sum::<usize>()
        );
    }

    #[test]
    fn seven_eight_grouping_2_2_3_puts_the_kick_on_the_selected_group_starts() {
        // 7/8、14 格、每拍 2 格、3 个 onset。`[2, 2, 3]` 的组起点是第 0、2、4 拍
        // （格点 0、4、8），重量都是 8 / 3 / 3 ⇒ 选点恰好是这三格。
        let grouping = BeatGrouping::new(Meter::SEVEN_EIGHT, &[2, 2, 3]).unwrap();
        let pattern = pattern(Meter::SEVEN_EIGHT, 1, 3, Some(grouping), 4);
        for cell in [0u32, 4, 8] {
            assert!(has(&pattern, DrumVoice::Kick, 0, cell), "kick {cell}");
        }
        assert_eq!(pattern.hit_count(DrumVoice::Kick), 3);
        assert_eq!(pattern.grid().hits().len(), 3);
        // 三个组起点（格点 0、4、8）的重量分别是 8、3、3 ⇒ 三个都拿吊镲。
        assert_eq!(pattern.hit_count(DrumVoice::Ride), 3);
    }

    #[test]
    fn swing_moves_the_hits_and_nothing_else() {
        let permille = Some(667u16);
        // 用**全格点**网格：8 个 onset 的 4/4 网格只取偶数格点，而每个格点都是
        // 一个摇摆对的起点（480 tick 的整数倍），摇摆于是不动任何 tick
        // —— 那不是"摇摆不生效"，是"这份网格里没有被推后的槽位"。
        let straight = pattern(COMMON, 1, 16, None, 2);
        let swung = swung_drum_pattern(COMMON, 1, 16, permille, None, 2)
            .unwrap()
            .unwrap();
        // 鼓件分派（同一 cell 上的鼓件集合）不因摇摆改变。
        for hit in straight.hits() {
            assert!(
                swung
                    .hits()
                    .iter()
                    .any(|other| other.cell == hit.cell && other.voice == hit.voice),
                "{hit:?}"
            );
        }
        assert_eq!(straight.len(), swung.len());
        // 一对的起点不被移动。
        let on_pair_start = swung
            .hits()
            .iter()
            .find(|hit| hit.cell == 2)
            .expect("cell 2 is in the grid");
        assert_eq!(on_pair_start.tick, 2 * GRID_CELL_TICKS);
        // 对内的第二个槽位被推到 `swung_pair_span(480, 667).first`。
        let slot = crate::swing::swung_pair_span(PPQ / 2, 667).unwrap().first;
        assert!(slot < crate::rhythm::SWING_PAIR_TICKS);
        let second = swung
            .hits()
            .iter()
            .find(|hit| hit.cell == 1)
            .expect("cell 1 is in the grid");
        assert_eq!(second.tick, slot);
        assert!(
            swung
                .hits()
                .windows(2)
                .all(|pair| pair[0].tick <= pair[1].tick),
            "swing must not reorder ticks"
        );
    }

    #[test]
    fn an_out_of_range_backbeat_is_rejected_not_clamped() {
        // 4/4 有 4 拍 ⇒ backbeat 合法区间 1..=4。
        for bad in [0u8, 5, 200] {
            assert_eq!(
                swung_drum_pattern(COMMON, 1, 8, None, None, bad).unwrap(),
                None,
                "backbeat {bad}"
            );
        }
        for good in 1u8..=4 {
            assert!(
                swung_drum_pattern(COMMON, 1, 8, None, None, good)
                    .unwrap()
                    .is_some(),
                "backbeat {good}"
            );
        }
    }

    #[test]
    fn two_groupings_of_the_same_meter_give_two_kick_readings() {
        // [4] 与 [2, 2] 都把第 0 拍判成组起点，也都把这一格选进网格 ⇒ 两者
        // 都在小节起点打底鼓；[4] 没有别的组起点（第 2 拍的格点 8 不在 4 个
        // onset 里），[2, 2] 则在格点 8 多打一个 ⇒ 这次比较的是**组起点判定的
        // 分离**，不是"零个底鼓"。
        let flat = BeatGrouping::new(COMMON, &[4]).unwrap();
        let halves = BeatGrouping::new(COMMON, &[2, 2]).unwrap();
        let with_flat = pattern(COMMON, 1, 4, Some(flat), 2);
        let with_halves = pattern(COMMON, 1, 4, Some(halves), 2);
        assert_eq!(with_flat.hit_count(DrumVoice::Kick), 1);
        assert_eq!(with_halves.hit_count(DrumVoice::Kick), 2);
        for with in [&with_flat, &with_halves] {
            assert!(has(with, DrumVoice::Kick, 0, 0));
        }
        assert!(!has(&with_flat, DrumVoice::Kick, 0, 8));
        assert!(has(&with_halves, DrumVoice::Kick, 0, 8));
    }

    #[test]
    fn zero_bars_and_zero_onsets_are_still_well_defined() {
        assert_eq!(
            swung_drum_pattern(COMMON, 0, 8, None, None, 2).unwrap_err(),
            TheoryError::ZeroBars
        );
        let empty = pattern(COMMON, 1, 0, None, 2);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert!(empty.hits_in_bar(0).is_empty());
        assert!(empty.hits_in_bar(9).is_empty());
        assert_eq!(empty.total_ticks(), 4 * PPQ);
    }

    #[test]
    fn hits_in_bar_partitions_the_hits_by_bar() {
        let pattern = pattern(COMMON, 3, 4, None, 2);
        let mut rebuilt = Vec::new();
        for bar in 0..3 {
            let slice = pattern.hits_in_bar(bar);
            assert!(slice.iter().all(|hit| hit.bar == bar));
            rebuilt.extend_from_slice(slice);
        }
        assert!(rebuilt == pattern.hits().to_vec());
        assert!(pattern.hits_in_bar(3).is_empty());
    }

    #[test]
    fn every_hit_matches_its_grid_onset_bit_for_bit() {
        for meter in [Meter::MARCH, Meter::WALTZ, COMMON, Meter::COMPOUND_DUPLE] {
            let grid = full_grid(meter, 2);
            let pattern = pattern(meter, 2, grid.onsets_per_bar(), None, 1);
            for hit in pattern.hits() {
                let onset = grid.hits().iter().find(|onset| {
                    onset.tick == hit.tick
                        && onset.bar == hit.bar
                        && onset.cell == hit.cell
                        && onset.weight == hit.weight
                });
                assert!(onset.is_some(), "{meter:?}: {hit:?} is not a grid onset");
                assert!(hit.tick < pattern.total_ticks());
                assert_eq!(hit.accent, hit.weight >= STRONG_BEAT_WEIGHT);
            }
            assert_eq!(pattern.meter(), meter);
            assert_eq!(pattern.bars(), 2);
            assert_eq!(pattern.ticks_per_bar(), meter.ticks_per_bar());
        }
    }
}
