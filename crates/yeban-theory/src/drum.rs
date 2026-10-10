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
//! `weight`）与**调用方传入**的参数（分组、反拍位置、摇摆千分比、[`DrumStyle`]）导出。
//! 本模块没有鼓件权重表、没有流派鼓点表、没有"每小节打几下"的常量表
//! —— 那需要逐流派登记数据，本 crate 无权发明（理由与 `onsets_per_bar`
//! 同源，见 [`crate::rhythm`] 的模块文档）。
//!
//! ## 逐流派的底鼓落点 [`DrumStyle`]（`pending 3` 的**一部分**登记了）
//!
//! [`DrumStyle`] 是**调用方声明的选择**，不是本模块推导出来的知识：调用方说
//! "这条流派用四踩底鼓"，本模块才把底鼓铺到每一拍。因此本模块仍然**没有**
//! 流派表，登记发生在 [`crate::genre::GENRES`] 的 `drum_style` 字段
//! （[`crate::genre::GenreRule::drum_pattern`] 把它传进来）：
//!
//! - 缺省 [`DrumStyle::Metric`] 逐位等于本模块的旧口径，因此**未登记**的流派
//!   行为不变（判据 `metric_style_is_bit_identical_to_the_legacy_entry_points`）；
//! - [`DrumStyle::FourOnTheFloor`] 只把底鼓从"组起点"扩到"每一拍的起点"。
//!
//! **仍未登记**（如实登记，不假装完成）：`pending 3` 要的"每条流派打什么样的
//! **鼓点序列**"只登记了**底鼓落点**这一维；军鼓位置仍走 [`Backbeat`] 参数、
//! 踩镲仍铺满网格、onset 数仍由调用方给出。因此 `pending 3` 的数据那一半是
//! **部分**关闭，不是全部关闭。
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
//!    | [`DrumVoice::Kick`] | 该 onset 是拍的起点，且 [`DrumStyle`] **把它判成底鼓位**（缺省 = 组起点，见下一节） |
//!    | [`DrumVoice::Snare`] | 该 onset 是拍的起点，且其**拍序号**能被 `backbeat` 整除 |
//!    | [`DrumVoice::HiHat`] | 网格里**每一个** onset |
//!    | [`DrumVoice::Ride`] | 组起点上**同时**是强位（`weight >= STRONG_BEAT_WEIGHT`）时，叠加在底鼓之上 |
//!
//!    [`DrumStyle`] **只改底鼓**：军鼓、踩镲、吊镲的判定与重量读数都不因它改变
//!    （判据 `a_drum_style_only_moves_the_kick`）。
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

/// 底鼓落点口径：**调用方声明的选择**，由 [`crate::genre::GenreRule`] 的
/// `drum_style` 字段登记进来。
///
/// 本枚举**只改底鼓**（军鼓、踩镲、吊镲的判定与重量读数都不变，判据
/// `a_drum_style_only_moves_the_kick`）。两个变体的差别只有一处：
/// "[`strikes_kick`](DrumStyle::strikes_kick) 拿到的组起点读数是真是假时算不算底鼓"。
///
/// 选择哪一个不是本模块能推导的知识：它是**流派的通行配器口径**，
/// 因此登记数据在 [`crate::genre::GENRES`]，缺省值 [`DrumStyle::Metric`]
/// 让未登记的流派行为与旧口径**逐位相同**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrumStyle {
    /// 度量口径（缺省，也是全部未登记流派的取值）：底鼓落在**组起点**上。
    ///
    /// "组起点"由调用方的 [`BeatGrouping`] 或拍号自身的度量层级判定，
    /// 与本模块的旧口径逐位相同。
    Metric,
    /// 四踩底鼓：底鼓落在**每一个拍的起点**上（不要求该拍是组起点）。
    ///
    /// 这是 disco / house / techno / trance 一类舞曲的通行配器口径
    /// （每拍一击的底鼓是这些流派不加修饰的默认）。它只**增加**底鼓击点：
    /// 因为组起点恒是拍的起点，四踩的底鼓集合是度量口径底鼓集合的超集
    /// （判据 `four_on_the_floor_is_a_superset_of_the_metric_kick`）。
    FourOnTheFloor,
}

impl DrumStyle {
    /// 全部口径，按声明顺序排列。
    pub const ALL: [Self; 2] = [Self::Metric, Self::FourOnTheFloor];

    /// 稳定的英文名（ASCII 小写）；用于审计直方图与错误文本，不参与判定。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::FourOnTheFloor => "four-on-the-floor",
        }
    }

    /// 这一格（已知落在**拍的起点**上）是否落底鼓。
    ///
    /// `is_group_start` 是这一格在调用方口径下"是不是组起点"的读数；
    /// [`DrumStyle::Metric`] 直接采信它，[`DrumStyle::FourOnTheFloor`] 忽略它。
    #[must_use]
    pub const fn strikes_kick(self, is_group_start: bool) -> bool {
        match self {
            Self::Metric => is_group_start,
            Self::FourOnTheFloor => true,
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
    styled_drum_pattern(
        meter,
        bars,
        onsets_per_bar,
        grouping,
        backbeat,
        DrumStyle::Metric,
    )
}

/// 平直鼓组型，底鼓落点由调用方声明的 [`DrumStyle`] 给出。
///
/// 这是 [`swung_styled_drum_pattern`] 的 `permille == None` 写法；
/// `style == DrumStyle::Metric` 时与 [`drum_pattern`] **逐位相同**。
///
/// ```
/// use yeban_theory::drum::{DrumStyle, DrumVoice, styled_drum_pattern};
/// use yeban_theory::progression::Meter;
///
/// // 4/4、一个小节、16 个 onset、反拍在第 2 拍、四踩底鼓。
/// let pattern = styled_drum_pattern(Meter::COMMON, 1, 16, None, 2, DrumStyle::FourOnTheFloor)
///     .unwrap()
///     .unwrap();
/// assert_eq!(pattern.hit_count(DrumVoice::Kick), 4);   // 每一拍一击
/// assert_eq!(pattern.hit_count(DrumVoice::Snare), 2);  // 军鼓不变
/// assert_eq!(pattern.hit_count(DrumVoice::HiHat), 16); // 踩镲不变
/// ```
///
/// # Errors
///
/// 与 [`drum_pattern`] 逐条相同。
pub fn styled_drum_pattern(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    grouping: Option<BeatGrouping<'_>>,
    backbeat: Backbeat,
    style: DrumStyle,
) -> Result<Option<DrumPattern>, TheoryError> {
    swung_styled_drum_pattern(meter, bars, onsets_per_bar, None, grouping, backbeat, style)
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
    swung_styled_drum_pattern(
        meter,
        bars,
        onsets_per_bar,
        permille,
        grouping,
        backbeat,
        DrumStyle::Metric,
    )
}

/// 摇摆鼓组型，底鼓落点由调用方声明的 [`DrumStyle`] 给出。
///
/// 与 [`swung_drum_pattern`] 的唯一差别是 `style`：它只改**底鼓**的落点判定
/// （见 [`DrumStyle::strikes_kick`]），网格、重量、军鼓、踩镲、吊镲全部不变。
/// `style == DrumStyle::Metric` 时与 [`swung_drum_pattern`] **逐位相同**。
///
/// 这是本模块的**唯一**公开实现：另外三个入口都转发到这里，因此
/// "网格怎么造 / 怎么分派"在本模块只有一份（口径不可能漂移）。
///
/// # Errors
///
/// 与 [`swung_drum_pattern`] 逐条相同。
pub fn swung_styled_drum_pattern(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    grouping: Option<BeatGrouping<'_>>,
    backbeat: Backbeat,
    style: DrumStyle,
) -> Result<Option<DrumPattern>, TheoryError> {
    match grouping {
        None => {
            let cells_per_beat = cells_per_beat_of(&meter);
            place_voices(
                meter,
                bars,
                onsets_per_bar,
                permille,
                backbeat,
                style,
                |cell| GridReading {
                    weight: metric_weight_in(meter, cell),
                    // 缺省分组 = 拍号自身的度量层级（见 [`is_meter_group_start`]）。
                    group_start: is_meter_group_start(meter, cell, cells_per_beat),
                },
            )
        }
        Some(grouping) => place_grouped_voices(
            meter,
            bars,
            onsets_per_bar,
            permille,
            backbeat,
            style,
            grouping,
        ),
    }
}

/// [`swung_styled_drum_pattern`] 的**显式分组**分支。
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
    style: DrumStyle,
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
        style,
        |cell| GridReading {
            weight: rule(cell),
            group_start: {
                if cells_per_beat == 0 {
                    false
                } else {
                    let offset = u64::from(cell) % cells_per_beat;
                    let beat = u64::from(cell) / cells_per_beat;
                    offset == 0
                        && beat < u64::from(felt_beats_per_bar(meter))
                        && grouping.is_group_start(beat as u8)
                }
            },
        },
    )
}

/// 一格的两个读数：度量重量（[`crate::rhythm::build_weighted_grid`] 选点用）
/// 与"这一格是不是组起点"（鼓件分派用）。
///
/// 两个读数合成**一个**返回值，是为了让 [`place_voices`] 的参数个数留在
/// clippy 的 7 个以内 —— 不新增 `#[allow]`。
#[derive(Debug, Clone, Copy)]
struct GridReading {
    /// 这一格的度量重量。
    weight: u8,
    /// 这一格是不是组起点（口径由调用方给出）。
    group_start: bool,
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
///
/// `kick` 与 `ride` 由调用方**算好**再传进来（调用方同时知道 [`DrumStyle`]
/// 与"这一格是不是组起点"）：本函数不读 [`DrumStyle`]，因此"鼓件落在哪"
/// 的判定仍然只有这一处。
fn voices_at(
    backbeat: Backbeat,
    beat: u8,
    offset_in_beat: u64,
    kick: bool,
    ride: bool,
) -> [bool; 4] {
    let on_beat_start = offset_in_beat == 0;
    let backbeat_hit = on_beat_start && backbeat != 0 && beat.is_multiple_of(backbeat);
    [kick, backbeat_hit, true, ride]
}

/// 把 `reading` 读出的重量灌进网格，再把 onset 分派到鼓件。
///
/// 这是公开入口的**唯一**实现：网格由
/// [`crate::rhythm::build_weighted_grid`] 造 —— 分派与网格共用**同一份**
/// 选点读数，因此不可能对"重心在哪"有两种读法。
///
/// `style` **只**参与底鼓判定（`style.strikes_kick(group_start)`）；
/// 吊镲仍然只读 `group_start`，因此 [`DrumStyle`] 改变不了它。
fn place_voices(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    backbeat: Backbeat,
    style: DrumStyle,
    reading: impl Fn(u32) -> GridReading,
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
        crate::rhythm::build_weighted_grid(meter, bars, onsets_per_bar, permille, |cell| {
            reading(cell).weight
        })?;
    let hits = dispatch(
        weighted.grid(),
        weighted.selected(),
        backbeat,
        cells_per_beat,
        style,
        &reading,
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

/// 分派本身：`reading(cell).group_start` 给出"这一格是不是组的起点"。
///
/// `selected` 与 `grid.hits()` 逐位相同（由
/// [`crate::rhythm::build_weighted_grid`] 同时产出），因此这里不倒推 tick。
///
/// [`DrumStyle`] 在这里落地：底鼓 =
/// `offset_in_beat == 0 && style.strikes_kick(组起点)`；
/// 吊镲 = `offset_in_beat == 0 && 组起点 && accent`（**不**读 `style`）。
fn dispatch(
    grid: &MetricGrid,
    selected: &[GridHit],
    backbeat: Backbeat,
    cells_per_beat: u64,
    style: DrumStyle,
    reading: impl Fn(u32) -> GridReading,
) -> Vec<DrumHit> {
    debug_assert_eq!(grid.hits(), selected);
    let mut hits = Vec::with_capacity(grid.len());
    for hit in grid.hits() {
        let offset_in_beat = u64::from(hit.cell) % cells_per_beat;
        let beat = (u64::from(hit.cell) / cells_per_beat) as u8;
        let accent = hit.weight >= STRONG_BEAT_WEIGHT;
        let on_beat_start = offset_in_beat == 0;
        let group_start = reading(hit.cell).group_start;
        let flags = voices_at(
            backbeat,
            beat,
            offset_in_beat,
            on_beat_start && style.strikes_kick(group_start),
            on_beat_start && group_start && accent,
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

    /// `is_meter_group_start` 对**小节之外**的格点恒返回 `false`。
    ///
    /// 注入实测（theory-16 形态 D）：把该函数里的 `cell as u64 >= cells` 改成
    /// `> cells`，全部既有判据保持全绿 —— 既有判据只喂小节**之内**的格点，
    /// 而越界那一条是公开契约（文档写着"`cell` 越出小节时返回 `false`"）。
    /// 危害：`cell == cells` 是**下一小节的第 0 格**，它恰好是强位，于是
    /// 被误判成"本小节的组起点"，底鼓会多打一击。
    #[test]
    fn a_cell_past_the_bar_is_never_a_group_start() {
        for (meter, cells_per_beat) in [
            (COMMON, 4u64),
            (Meter::WALTZ, 4),
            (Meter::COMPOUND_DUPLE, 6),
            (Meter::SEVEN_EIGHT, 2),
        ] {
            let cells = cells_per_bar(meter).unwrap();
            for cell in [cells as u32, cells as u32 + 1, 2 * cells as u32, u32::MAX] {
                assert!(
                    !is_meter_group_start(meter, cell, cells_per_beat),
                    "{meter:?} cell {cell} (bar has {cells} cells) must not be a group start"
                );
            }
            // 小节内的格点读数不受影响（回归护栏）。
            assert!(is_meter_group_start(meter, 0, cells_per_beat));
        }
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

    /// 形态 D 注入实测（本票）：把 `place_voices` 的"每拍格位数为 0"守卫
    /// 放宽成 `<= 1` 时**没有任何既有判据变红**。逐输入核对（本机实测）后
    /// 判定这是**观测等价**：合法拍号上 `cells_per_beat == 1` 要求
    /// `cells == beats`，而 `cells = 16 × numerator / denominator`、
    /// `felt_beats_per_bar` 对简单拍就是 `numerator`，两者相等只在
    /// `denominator == 16` 时成立；那时 `cells == 16 × numerator / 16 = numerator`，
    /// 但 `cells_per_bar` 对分母 16 的量级会先向下取整到 0（`numerator < 16` 时
    /// `16 × numerator / 16` 仍是 `numerator`，而守卫的 `cells / beats` 恰好是 1）
    /// —— 本机实测 `1/16` 与 `2/32` 两条输入在两版下读数**完全相同**（前者
    /// `Some`、后者 `None`），因此这条判据不宣称能判死那个注入。
    ///
    /// 它钉住的是**合法合约**：每拍 1 格与"每拍分不到格"两种边界拍号上，
    /// 鼓组型的行为必须稳定（前者产出、后者 `Ok(None)`），且军鼓按反拍口径落位。
    #[test]
    fn one_cell_per_beat_is_legal_instead_of_being_rejected() {
        use crate::rhythm::cells_per_bar;
        // 1/16：每小节 1 格、1 拍 ⇒ 每拍 1 格。
        let one_sixteenth = Meter::new(1, 16).unwrap();
        assert_eq!(cells_per_bar(one_sixteenth), Some(1));
        assert_eq!(felt_beats_per_bar(one_sixteenth), 1);
        let pattern = swung_drum_pattern(one_sixteenth, 2, 1, None, None, 1)
            .unwrap()
            .expect("1/16 must not be rejected by the cells-per-beat guard");
        assert_eq!(pattern.bars(), 2);
        // 每小节 1 格 ⇒ 每拍 1 格：每小节恰好一个 onset，底鼓与踩镲都在格点 0。
        assert!(has(&pattern, DrumVoice::Kick, 0, 0));
        assert!(has(&pattern, DrumVoice::Kick, 1, 0));
        assert!(has(&pattern, DrumVoice::HiHat, 0, 0));
        assert!(has(&pattern, DrumVoice::Snare, 0, 0));
        // 2/32：2 拍而每小节只有 1 格 ⇒ `cells / beats` 向下取整为 0 ⇒
        // 按现行口径拒绝（这条读数在 `== 0` 与 `<= 1` 两版下相同）。
        let two_thirty_seconds = Meter::new(2, 32).unwrap();
        assert_eq!(cells_per_bar(two_thirty_seconds), Some(1));
        assert_eq!(felt_beats_per_bar(two_thirty_seconds), 2);
        assert_eq!(
            swung_drum_pattern(two_thirty_seconds, 1, 1, None, None, 1).unwrap(),
            None
        );
        // 8/32：8 拍、每小节 4 格 ⇒ 每拍 0.5 格 ⇒ `Ok(None)`。
        let eight_thirty_seconds = Meter::new(8, 32).unwrap();
        assert_eq!(cells_per_bar(eight_thirty_seconds), Some(4));
        assert_eq!(felt_beats_per_bar(eight_thirty_seconds), 8);
        assert_eq!(
            swung_drum_pattern(eight_thirty_seconds, 1, 4, None, None, 4).unwrap(),
            None
        );
        // 对照侧面：4/4 每小节 2 个 onset ⇒ 第 0、8 格（第 0、2 拍），
        // 军鼓按 `backbeat == 3` 只落在第 0 拍（第 2 拍不被 3 整除）。
        let pattern = swung_drum_pattern(Meter::COMMON, 1, 2, None, None, 3)
            .unwrap()
            .expect("4/4 with two onsets per bar must still produce a pattern");
        assert_eq!(pattern.onsets_per_bar(), 2);
        assert_eq!(pattern.hit_count(DrumVoice::Snare), 1);
        assert!(pattern.hit_count(DrumVoice::Kick) >= 1);
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

    #[test]
    fn metric_style_is_bit_identical_to_the_legacy_entry_points() {
        // 数什么：对每个 (拍号, onset 数, 摇摆比例, 反拍位置) 组合，逐位比较
        // `DrumStyle::Metric` 的新入口与旧入口（`drum_pattern` /
        // `swung_drum_pattern`）产出的击点序列与网格 onset 序列。
        // 单位 = "比较过的组合个数"；任一组合不同即失败。
        let mut compared = 0usize;
        for meter in [Meter::MARCH, Meter::WALTZ, COMMON, Meter::COMPOUND_DUPLE] {
            let beats = felt_beats_per_bar(meter);
            let cells = cells_per_bar(meter).unwrap() as u32;
            for onsets in 1..=cells {
                for permille in [None, Some(500u16), Some(667)] {
                    for backbeat in 1..=beats {
                        let legacy = swung_drum_pattern(meter, 2, onsets, permille, None, backbeat)
                            .unwrap()
                            .unwrap();
                        let styled = swung_styled_drum_pattern(
                            meter,
                            2,
                            onsets,
                            permille,
                            None,
                            backbeat,
                            DrumStyle::Metric,
                        )
                        .unwrap()
                        .unwrap();
                        assert!(
                            legacy.hits() == styled.hits(),
                            "{meter:?} onsets {onsets} permille {permille:?} backbeat {backbeat}"
                        );
                        assert!(legacy.grid().hits() == styled.grid().hits(), "{meter:?}");
                        assert_eq!(
                            styled.hits().len(),
                            legacy.hits().len(),
                            "{meter:?} onsets {onsets}"
                        );
                        compared += 1;
                    }
                }
            }
        }
        // 实测读数：4 个拍号 × 各自的格位数(8 + 12 + 16 + 12) × 3 个摇摆比例
        // × 各自的拍数(2 + 3 + 4 + 2) = 3 × (8×2 + 12×3 + 16×4 + 12×2) = **420**
        // 个组合全部逐位相同。
        assert_eq!(compared, 420);
        // 平直入口也必须与旧入口逐位相同。
        let legacy = drum_pattern(COMMON, 3, 8, None, 2).unwrap().unwrap();
        let styled = styled_drum_pattern(COMMON, 3, 8, None, 2, DrumStyle::Metric)
            .unwrap()
            .unwrap();
        assert!(legacy.hits() == styled.hits());
        assert!(legacy.grid().hits() == styled.grid().hits());
    }

    #[test]
    fn four_on_the_floor_puts_the_kick_on_every_beat() {
        // 4/4、16 格全网格、反拍在第 2 拍：四踩底鼓 = 4 个底鼓（每拍一击），
        // 度量口径 = 2 个（组起点）。军鼓 / 踩镲 / 吊镲一个都不变。
        let cells = cells_per_bar(COMMON).unwrap() as u32;
        let metric = styled_drum_pattern(COMMON, 1, cells, None, 2, DrumStyle::Metric)
            .unwrap()
            .unwrap();
        let four = styled_drum_pattern(COMMON, 1, cells, None, 2, DrumStyle::FourOnTheFloor)
            .unwrap()
            .unwrap();
        assert_eq!(metric.hit_count(DrumVoice::Kick), 2);
        assert_eq!(four.hit_count(DrumVoice::Kick), 4);
        for beat in 0..felt_beats_per_bar(COMMON) as u32 {
            assert!(has(&four, DrumVoice::Kick, 0, beat * 4), "kick beat {beat}");
        }
        for voice in [DrumVoice::Snare, DrumVoice::HiHat, DrumVoice::Ride] {
            assert_eq!(
                metric.hit_count(voice),
                four.hit_count(voice),
                "{voice:?} must not move"
            );
        }
        // 底鼓的 tick 集合只能变大：{0, 8×240} ⊂ {0, 4×240, 8×240, 12×240}。
        let kick_ticks = |p: &DrumPattern| -> Vec<u64> {
            p.hits()
                .iter()
                .filter(|hit| hit.voice == DrumVoice::Kick)
                .map(|hit| hit.tick)
                .collect()
        };
        assert_eq!(
            kick_ticks(&metric),
            vec![0, 8 * GRID_CELL_TICKS],
            "metric kick"
        );
        assert_eq!(
            kick_ticks(&four),
            vec![
                0,
                4 * GRID_CELL_TICKS,
                8 * GRID_CELL_TICKS,
                12 * GRID_CELL_TICKS
            ],
            "four-on-the-floor kick"
        );
    }

    #[test]
    fn a_drum_style_only_moves_the_kick() {
        // 数什么：对每个 (拍号, onset 数, 反拍位置) 组合，比较两种口径下
        // "非底鼓击点"的 `(voice, tick)` 序列。单位 = 比较过的组合个数。
        let mut compared = 0usize;
        for meter in [Meter::MARCH, Meter::WALTZ, COMMON, Meter::COMPOUND_DUPLE] {
            let beats = felt_beats_per_bar(meter);
            let cells = cells_per_bar(meter).unwrap() as u32;
            for onsets in [1u32, 2, 4, 8, cells] {
                for backbeat in 1..=beats {
                    let metric =
                        styled_drum_pattern(meter, 1, onsets, None, backbeat, DrumStyle::Metric)
                            .unwrap()
                            .unwrap();
                    let four = styled_drum_pattern(
                        meter,
                        1,
                        onsets,
                        None,
                        backbeat,
                        DrumStyle::FourOnTheFloor,
                    )
                    .unwrap()
                    .unwrap();
                    let others = |p: &DrumPattern| -> Vec<(DrumVoice, u64)> {
                        p.hits()
                            .iter()
                            .filter(|hit| hit.voice != DrumVoice::Kick)
                            .map(|hit| (hit.voice, hit.tick))
                            .collect()
                    };
                    assert!(
                        others(&metric) == others(&four),
                        "{meter:?} onsets {onsets}"
                    );
                    // 底鼓只增不减（组起点恒是拍的起点）。
                    for hit in metric
                        .hits()
                        .iter()
                        .filter(|hit| hit.voice == DrumVoice::Kick)
                    {
                        assert!(
                            four.hits()
                                .iter()
                                .any(|other| other.voice == DrumVoice::Kick
                                    && other.tick == hit.tick),
                            "{meter:?} onsets {onsets}: kick at {} vanished",
                            hit.tick
                        );
                    }
                    compared += 1;
                }
            }
        }
        // 4 个拍号 × 5 个 onset 数 × 各自的拍数(2 + 3 + 4 + 2) = 5 × 11 = 55。
        assert_eq!(compared, 55);
    }

    #[test]
    fn four_on_the_floor_is_a_superset_of_the_metric_kick() {
        // 显式分组也要成立：5/4 = [3, 2] 的组起点不是 4/4 式的强位拍。
        let five_four = Meter::new(5, 4).unwrap();
        let cases: [(Meter, &[u8]); 4] = [
            (Meter::MARCH, &[1, 1]),
            (Meter::WALTZ, &[1, 1, 1]),
            (COMMON, &[2, 2]),
            (five_four, &[3, 2]),
        ];
        let mut compared = 0usize;
        for (meter, groups) in cases {
            let grouping = BeatGrouping::new(meter, groups).unwrap();
            let cells = cells_per_bar(meter).unwrap() as u32;
            for onsets in [1u32, 2, 4, 8, cells] {
                for backbeat in 1..=felt_beats_per_bar(meter) {
                    let metric = swung_styled_drum_pattern(
                        meter,
                        2,
                        onsets,
                        None,
                        Some(grouping),
                        backbeat,
                        DrumStyle::Metric,
                    )
                    .unwrap()
                    .unwrap();
                    let four = swung_styled_drum_pattern(
                        meter,
                        2,
                        onsets,
                        None,
                        Some(grouping),
                        backbeat,
                        DrumStyle::FourOnTheFloor,
                    )
                    .unwrap()
                    .unwrap();
                    for hit in metric
                        .hits()
                        .iter()
                        .filter(|hit| hit.voice == DrumVoice::Kick)
                    {
                        assert!(
                            four.hits()
                                .iter()
                                .any(|other| other.voice == DrumVoice::Kick
                                    && other.tick == hit.tick),
                            "{meter:?} onsets {onsets} backbeat {backbeat}: \
                             the metric kick at {} vanished under four-on-the-floor",
                            hit.tick
                        );
                    }
                    compared += 1;
                }
            }
        }
        // 4 个 (拍号, 分组) 组合 × 5 个 onset 数 × 各自的拍数(2 + 3 + 4 + 5) = 5 × 14。
        assert_eq!(compared, 70);
    }

    #[test]
    fn a_drum_style_never_changes_which_inputs_are_rejected() {
        // 口径只改底鼓落点，不改参数校验：两种口径下四条拒绝路径逐条相同。
        for style in DrumStyle::ALL {
            assert!(
                styled_drum_pattern(COMMON, 1, 16, None, 0, style)
                    .unwrap()
                    .is_none(),
                "{style:?}: backbeat 0 must be rejected, not clamped"
            );
            assert!(
                styled_drum_pattern(COMMON, 1, 16, None, 5, style)
                    .unwrap()
                    .is_none(),
                "{style:?}: backbeat > beats must be rejected"
            );
            assert_eq!(
                styled_drum_pattern(COMMON, 0, 16, None, 2, style).unwrap_err(),
                TheoryError::ZeroBars,
                "{style:?}"
            );
            assert_eq!(
                styled_drum_pattern(Meter::MARCH, 1, 9, None, 2, style).unwrap_err(),
                TheoryError::ProgressionTooDense {
                    degrees: 9,
                    slots: 8
                },
                "{style:?}"
            );
        }
    }

    /// 形态 D 注入实测（第四批）：`DrumVoice::gm_key` 的普查读数是
    /// **0 判据 / 0 调用点**（整个工作区没有任何地方读它）⇒ 把任一个鼓件的
    /// 通用 MIDI 编号改掉时四道闸门全绿。
    ///
    /// ⚠ 既有判据 `voices_are_a_total_mapping_with_distinct_ordinals_names_and_keys`
    /// 只钉住"四个编号**两两不同**"：把 Kick 从 36 改成 38 会变红（撞上 Snare），
    /// 但把 Snare 从 38 改成 37、HiHat 从 42 改成 41、Ride 从 51 改成 50
    /// **都不会**变红。这条判据因此钉的是**具体编号**，不是"互不相同"。
    #[test]
    fn gm_key_pins_the_general_midi_percussion_numbers() {
        assert_eq!(DrumVoice::Kick.gm_key(), 36);
        assert_eq!(DrumVoice::Snare.gm_key(), 38);
        assert_eq!(DrumVoice::HiHat.gm_key(), 42);
        assert_eq!(DrumVoice::Ride.gm_key(), 51);
        // 全体映射：顺序就是 `DrumVoice::ALL`，读数与逐个断言一致。
        let keys: Vec<u8> = DrumVoice::ALL.iter().map(|voice| voice.gm_key()).collect();
        assert_eq!(keys, vec![36, 38, 42, 51]);
        assert_eq!(keys.len(), DRUM_VOICE_COUNT);
        // 编号落在一段合法的 MIDI 打击乐键位里。
        for key in keys {
            assert!(
                (35..=81).contains(&key),
                "{key} is outside the GM percussion range"
            );
        }
    }

    /// 形态 D 注入实测（第五批）：`DrumVoice::name` 的**取值**没有判据 ——
    /// 既有判据只钉"四个名字**两两不同**"。把四个名字做一次**置换**
    /// （kick→snare、snare→kick、hihat→ride、ride→hihat）仍然两两不同，
    /// 于是四道闸门**全绿**（单改一个名字会撞名，因而是 RED）。
    ///
    /// 同一条判据钉住两个 `ALL` 表的**顺序**：把 `DrumStyle::ALL` 的两个元素
    /// 互换时四道闸门同样全绿（既有直方图判据只数个数，与顺序无关）。
    #[test]
    fn enum_readouts_pin_names_ordinals_and_all_order() {
        assert_eq!(DrumVoice::Kick.name(), "kick");
        assert_eq!(DrumVoice::Snare.name(), "snare");
        assert_eq!(DrumVoice::HiHat.name(), "hihat");
        assert_eq!(DrumVoice::Ride.name(), "ride");
        assert_eq!(
            DrumVoice::ALL.map(DrumVoice::name),
            ["kick", "snare", "hihat", "ride"]
        );
        assert_eq!(
            DrumVoice::ALL.map(DrumVoice::ordinal),
            [0, 1, 2, 3],
            "`ALL` must be listed in ordinal order"
        );
        for (index, voice) in DrumVoice::ALL.iter().enumerate() {
            assert_eq!(voice.ordinal(), index, "ordinal of {voice:?}");
        }
        assert_eq!(DrumStyle::Metric.name(), "metric");
        assert_eq!(DrumStyle::FourOnTheFloor.name(), "four-on-the-floor");
        assert_eq!(
            DrumStyle::ALL.map(DrumStyle::name),
            ["metric", "four-on-the-floor"]
        );
        assert_eq!(DrumStyle::ALL.len(), 2);
        // 两个 `ALL` 表的长度常量与数组长度一致。
        assert_eq!(DrumVoice::ALL.len(), DRUM_VOICE_COUNT);
    }
}
