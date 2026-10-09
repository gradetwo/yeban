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
//! 3. **选哪些格点**：按**度量重量**从强到弱取，重量由 [`metric_weight_in`]
//!    按**拍号**算出（Lerdahl–Jackendoff 式的度量层级）：小节起点最重；
//!    简单拍再取半小节处的强拍；其余拍等重；拍内再区分正拍与细分。
//!    同重量按格点下标升序打破平局。判定全部是整数，无浮点比较 ⇒
//!    同输入同输出、跨平台逐位一致 [ARCH-DET-001]。
//!
//!    ⚠ 重量**必须**读拍号，不能只看格点下标：6/8 与 3/4 的小节长度都是
//!    2880 tick、都是 12 个 16 分格位，但 6/8 是**复合二拍**（每拍 = 三连八分
//!    = 6 格，第二拍起于第 6 格），3/4 是**三拍**（每拍 = 四分 = 4 格，
//!    各拍起于第 0、4、8 格）。只看下标的旧口径把两者判成同一条重量序列，
//!    于是 6/8 的两拍被选成第 0、8 格而不是第 0、6 格。
//!    见 [`metric_weight`]（拍号无关的旧口径，保留为公开面）与
//!    [`metric_weight_in`]（本模块使用的新口径）。
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
//! ## 加性拍分组（补上"5/4 的 3+2 与 2+3 不给"这一条的**机制**侧）
//!
//! [`metric_weight_in`] 只用**拍号本身**算重量。它表达得了复合拍（6/8 的两拍），
//! 表达不了**加性/混合拍**的分组：5/4 是 3+2 还是 2+3、7/8 是 2+2+3 还是
//! 3+2+2、8/8 是不是 3+3+2、11/8 怎么切 —— 这些是**作品或地区的选择**，
//! 拍号里没有写。因此这些拍号在 [`metric_weight_in`] 下只有"小节起点"一个
//! 位置比别的位置重（其余拍等重），网格于是按格点升序取到最靠前的几拍。
//!
//! [`BeatGrouping`] 补上**机制**那一半：分组由**调用方**传入，与
//! `onsets_per_bar` 同口径（见上一节"为什么 onset 数由调用方给出"）。
//! [`metric_weight_grouped`] 给出分组口径的重量，[`grouped_metric_grid`] /
//! [`grouped_swung_metric_grid`] 给出按该口径选点的网格。
//!
//! **仍未做**（如实登记，与 `onsets_per_bar` 的理由同源）：逐流派的**分组数据**
//! 没有登记进 [`crate::genre::GENRES`]（登记表里 `balkan`、`progressive_rock`、
//! `progressive_metal`、`math_rock` 四条都是 7/8，而 7/8 的两种切法都通行）。
//! 数据没登记 ⇒ 本模块**不**替任何流派猜一个分组，
//! [`crate::genre::GenreRule::rhythm_grid`] 继续走 [`metric_weight_in`]。
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

/// 小节内第 `cell` 个格点的**度量重量**（越大越强），**只看格点下标**。
///
/// 口径：二分度量层级。`cell == 0` 取 [`MAX_METRIC_WEIGHT`]；
/// 其余取 `cell` 二进制末尾零的个数（上限 [`MAX_METRIC_WEIGHT`]）。
/// 4/4（16 格）因此得到：格点 0 → 8、格点 8 → 3、格点 4/12 → 2、
/// 格点 2/6/10/14 → 1、奇数格点 → 0。
///
/// ⚠ 本函数**不读拍号**，因此它隐含"每拍 4 格、小节是 2 的幂"这一假设。
/// 对 2/4 与 4/4 它与 [`metric_weight_in`] 逐位相同；对 3/4、5/4、7/8
/// 它只给近似；对 6/8 它给出**错的**结构 —— 6/8 与 3/4 的小节长度都是
/// 2880 tick、都是 12 个 16 分格位，本函数把两者判成同一条重量序列，
/// 而 6/8 的第二拍其实起于第 6 格（见 [`is_compound_meter`]）。
///
/// 保留本函数是为了不破坏既有公开面；新调用方请用 [`metric_weight_in`]。
/// 各地区的节拍分组（例如 5/4 的 3+2 与 2+3）本函数与 [`metric_weight_in`]
/// 都**不**给：那需要逐流派数据，属 `pending 3` 的另一半。
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

/// 非首拍的**强拍**重量：简单拍的半小节拍，以及复合拍的每一拍。
///
/// 取 3 的依据：它必须严格大于 [`BEAT_WEIGHT`]（普通拍）且严格小于
/// [`MAX_METRIC_WEIGHT`]（小节起点），这样"重量降序取格点"选出的前几名
/// 恰好是小节起点、强拍、普通拍。4/4 的第 8 格沿用本值（旧口径
/// [`metric_weight`] 在那里也取 3）。
pub const STRONG_BEAT_WEIGHT: u8 = 3;

/// 非首拍、非强拍的**普通拍**重量（例如 4/4 的第 4、12 格）。取 2。
pub const BEAT_WEIGHT: u8 = 2;

/// **拍内细分**的重量（例如 4/4 的第 2、6、10、14 格，即八分反拍）。取 1。
///
/// [`crate::melody::CHORD_TONE_WEIGHT_FLOOR`] 取 1：重量 ≥ 1 的位置优先取
/// 和弦构成音。因此在**每拍格位数为偶数**的拍号上，"重量 ≥ 1"恒等价于
/// "格点下标为偶数"，与该常量的文档一致。
pub const OFFBEAT_WEIGHT: u8 = 1;

/// 该拍号是否是**复合拍 (compound meter)**。
///
/// 口径：分母为 8、分子是 3 的倍数且大于 3 ⇒ 6/8、9/8、12/8 是复合拍，
/// 它们的"拍"是三连八分（附点四分），不是八分。
/// 3/8（分子不大于 3）与 8/8（分子不是 3 的倍数）**不**算复合拍：
/// 两者的拍都取八分，因此 3/8 是 3 拍、8/8 是 8 拍。
/// 8/8 的 3+3+2 分组属作品选择，本条不发明它（与 5/4 的 3+2 同理）。
#[must_use]
pub const fn is_compound_meter(meter: Meter) -> bool {
    meter.denominator == 8 && meter.numerator > 3 && meter.numerator.is_multiple_of(3)
}

/// 每小节的**拍数**（felt beats）：简单拍取分子，复合拍取分子 / 3。
///
/// 单位是"拍"。`6/8` ⇒ 2、`3/4` ⇒ 3、`4/4` ⇒ 4、`7/8` ⇒ 7。
/// 分子为 0（只能由直接赋值公有字段得到的非法 [`Meter`]）时返回 0。
#[must_use]
pub const fn felt_beats_per_bar(meter: Meter) -> u8 {
    if is_compound_meter(meter) {
        meter.numerator / 3
    } else {
        meter.numerator
    }
}

/// 按**拍号**算出小节内第 `cell` 个格点的度量重量（越大越强）。
///
/// ## 层级
///
/// 每小节的格点被分成 [`felt_beats_per_bar`] 个等长的**拍**，每拍含
/// `cells_per_bar / 拍数` 个格点。重量按下面的固定规则给（全部是整数）：
///
/// 1. 小节起点（拍 0 的格点 0）⇒ [`MAX_METRIC_WEIGHT`]；
/// 2. 复合拍的非首拍 ⇒ [`STRONG_BEAT_WEIGHT`]（6/8 的第 6 格是 1440 tick
///    处的第二个强拍）；
/// 3. 简单拍且拍数为偶数且拍数 ≥ 4 时，半小节处的拍 ⇒ [`STRONG_BEAT_WEIGHT`]
///    （4/4 的第 8 格），其余拍 ⇒ [`BEAT_WEIGHT`]；
/// 4. 拍内格点下标为偶数 ⇒ [`OFFBEAT_WEIGHT`]，为奇数 ⇒ 0。
///
/// 该规则在 2/4 与 4/4 上与 [`metric_weight`] **逐位相同**（回归护栏见测试
/// `metric_weight_in_keeps_binary_meters_bit_identical_to_metric_weight`）。
///
/// ## 回退
///
/// 下列三种输入退到拍号无关的 [`metric_weight`]，**不** panic：公有的 [`Meter`]
/// 字段可以绕过 [`crate::progression::Meter::new`] 直接赋值。
///
/// 1. 拍号非法 ⇒ [`cells_per_bar`] 为 `None`；
/// 2. `cell` 越界（≥ 每小节格点数）。这也覆盖"每小节不足 1 格"的病态拍号
///    （例如分母 128：`cells_per_bar` 为 0，此时任何 `cell` 都越界）；
/// 3. 每拍格位数为 0（例如 2/32：每小节 1 格却有 2 拍）。
///
/// 拍数恒不为 0：`cells_per_bar` 返回 `Some` 蕴含分子不为 0，因此简单拍的
/// 拍数 ≥ 1、复合拍的拍数 ≥ 2。这里不再写一条不可达的 `beats == 0` 分支
/// —— 不可达的分支无法被注入判据覆盖（实测：删掉它没有判据变红）。
///
/// ## 与 4/4 的对照（16 格）
///
/// 格点 0 → 8、8 → 3、4/12 → 2、2/6/10/14 → 1、奇数 → 0，与旧口径一致。
/// 6/8（12 格）得到 0 → 8、6 → 3、2/4/8/10 → 1、其余 → 0。
#[must_use]
pub const fn metric_weight_in(meter: Meter, cell: u32) -> u8 {
    let Some(cells) = cells_per_bar(meter) else {
        return metric_weight(cell);
    };
    if cell as u64 >= cells {
        return metric_weight(cell);
    }
    let beats = felt_beats_per_bar(meter) as u64;
    let cells_per_beat = cells / beats;
    if cells_per_beat == 0 {
        return metric_weight(cell);
    }
    let beat_index = cell as u64 / cells_per_beat;
    let offset = cell as u64 % cells_per_beat;
    if offset == 0 {
        if beat_index == 0 {
            return MAX_METRIC_WEIGHT;
        }
        if is_compound_meter(meter) {
            return STRONG_BEAT_WEIGHT;
        }
        if beats >= 4 && beats.is_multiple_of(2) && beat_index == beats / 2 {
            return STRONG_BEAT_WEIGHT;
        }
        BEAT_WEIGHT
    } else if offset.is_multiple_of(2) {
        OFFBEAT_WEIGHT
    } else {
        0
    }
}

/// **加性拍分组**：把一小节的**拍**切成若干**组**，每组若干拍。
///
/// 例：5/4 = `[3, 2]`（3+2）或 `[2, 3]`（2+3）、7/8 = `[2, 2, 3]` 或 `[3, 2, 2]`、
/// 8/8 = `[3, 3, 2]`、11/8 = `[2, 2, 3, 2, 2]`、6/8 = `[1, 1]`（两个复合拍）。
///
/// ## 为什么分组是调用方参数
///
/// 分组是**作品或地区的选择**，不是拍号写出来的：7/8 的两种切法都通行。
/// 本 crate 的流派登记表里**没有**这个字段，因此本模块不替任何流派猜一个
/// （理由与 `onsets_per_bar` 同源，见模块文档）。
///
/// ## 合法性（[`BeatGrouping::new`]）
///
/// 只在下列条件**全部**成立时给出 `Some`：
///
/// 1. 分组非空；
/// 2. 每一组至少 1 拍（没有 0）；
/// 3. 各组之和**恰好**等于 [`felt_beats_per_bar`]（分组必须覆盖全部拍，
///    不多不少）。
///
/// ⛔ 违例返回 `None`：**不**截断、**不**补齐、**不** panic。
/// 这里用 `Option` 而不是 `Result`，因为本 crate **不新增**
/// [`crate::TheoryError`] 的变体（那是 `yeban-mcp` 也在消费的跨 crate 契约），
/// 而"分组非法"是调用方可自查的输入约束 —— 与 [`cells_per_bar`] 返回
/// `Option` 同口径。
///
/// ## 与内置口径的关系
///
/// 每个拍号都有一条能把 [`metric_weight_in`] **逐位**复现的分组
/// （2/4 = `[2]`、3/4 = `[3]`、4/4 = `[2, 2]`、5/4 = `[5]`、7/8 = `[7]`、
/// 6/8 = `[1, 1]`、9/8 = `[1, 1, 1]`、12/8 = `[1, 1, 1, 1]`，
/// 判据见测试 `metric_weight_grouped_reproduces_the_builtin_hierarchy`）。
/// 传那条分组等价于不传；换一条（5/4 = `[3, 2]`、7/8 = `[2, 2, 3]`）才改读数。
///
/// ⚠ 8/8 没有能把内置口径逐位复现的分组：内置口径只在**半小节**处
/// （第 4 拍）给强拍，而任何分组都会把**每一组**的起点判成强拍。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeatGrouping<'a> {
    groups: &'a [u8],
    beats: u8,
}

impl<'a> BeatGrouping<'a> {
    /// 为 `meter` 构造一条覆盖它全部拍的分组；不合法时返回 `None`（见类型文档）。
    #[must_use]
    pub fn new(meter: Meter, groups: &'a [u8]) -> Option<Self> {
        let beats = felt_beats_per_bar(meter);
        if beats == 0 || groups.is_empty() || groups.contains(&0) {
            return None;
        }
        let total: u32 = groups.iter().map(|&count| u32::from(count)).sum();
        if total != u32::from(beats) {
            return None;
        }
        Some(Self { groups, beats })
    }

    /// 各组的拍数，与构造时的入参逐位相同。
    #[must_use]
    pub const fn groups(&self) -> &'a [u8] {
        self.groups
    }

    /// 分组覆盖的拍数（构造时恒等于 `felt_beats_per_bar(meter)`）。
    #[must_use]
    pub const fn beats(&self) -> u8 {
        self.beats
    }

    /// 组数。
    #[must_use]
    pub const fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// 第 `beat` 拍（0 基）是否落在某一组的**起点**上。
    ///
    /// 第 0 拍恒是起点（第一条分组的起点就是小节起点）。`beat >= beats` 时
    /// 返回 `false`（越界不是任何组的起点，也不 panic）。
    #[must_use]
    pub const fn is_group_start(&self, beat: u8) -> bool {
        let mut boundary: u16 = 0;
        let mut index = 0usize;
        while index < self.groups.len() {
            if beat as u16 == boundary {
                return true;
            }
            boundary += self.groups[index] as u16;
            index += 1;
        }
        false
    }
}

/// 按**拍分组**算小节内第 `cell` 个格点的度量重量（越大越强）。
///
/// 与 [`metric_weight_in`] 的唯一差别是"哪些拍算强拍"：
///
/// 1. 小节起点（拍 0 的格点 0）⇒ [`MAX_METRIC_WEIGHT`]；
/// 2. 某一组的起点（不是小节起点）⇒ [`STRONG_BEAT_WEIGHT`]；
/// 3. 其余拍的起点 ⇒ [`BEAT_WEIGHT`]；
/// 4. 拍内格点下标为偶数 ⇒ [`OFFBEAT_WEIGHT`]，为奇数 ⇒ 0。
///
/// 用 `[3, 2]` 读 5/4，第 3 拍（第 12 格 = 2880 tick）因此与第 0 格同属强位；
/// 用 `[2, 2, 3]` 读 7/8，第 2、4 拍（第 4、8 格 = 960、1920 tick）是强位。
///
/// ## 回退
///
/// 下列输入退到拍号口径的 [`metric_weight_in`]，**不** panic：
///
/// 1. `grouping` 的拍数不等于 `meter` 的 [`felt_beats_per_bar`]（调用方拿了
///    **另一个拍号**的分组）；
/// 2. [`metric_weight_in`] 自己列出的全部回退条件（拍号非法、`cell` 越界、
///    每拍格位数为 0）。
///
/// 分组内部的合法性在 [`BeatGrouping::new`] 已经查过，因此这里不会出现
/// "分组自相矛盾"的输入。
///
/// ⚠ 分组**不能**表达"哪一组更长"以外的层级（例如 3+2+2 里第 2 组内部还想
/// 再分强弱）：同一条分组里，所有非起点的拍等重（[`BEAT_WEIGHT`]）。
#[must_use]
pub const fn metric_weight_grouped(meter: Meter, cell: u32, grouping: BeatGrouping<'_>) -> u8 {
    if grouping.beats() != felt_beats_per_bar(meter) {
        return metric_weight_in(meter, cell);
    }
    let Some(cells) = cells_per_bar(meter) else {
        return metric_weight_in(meter, cell);
    };
    if cell as u64 >= cells {
        return metric_weight_in(meter, cell);
    }
    let beats = felt_beats_per_bar(meter) as u64;
    let cells_per_beat = cells / beats;
    if cells_per_beat == 0 {
        return metric_weight_in(meter, cell);
    }
    let beat_index = cell as u64 / cells_per_beat;
    let offset = cell as u64 % cells_per_beat;
    if offset == 0 {
        if beat_index == 0 {
            return MAX_METRIC_WEIGHT;
        }
        if grouping.is_group_start(beat_index as u8) {
            return STRONG_BEAT_WEIGHT;
        }
        return BEAT_WEIGHT;
    }
    if offset.is_multiple_of(2) {
        OFFBEAT_WEIGHT
    } else {
        0
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
    /// 该格点的度量重量：拍号口径见 [`metric_weight_in`]，加性分组口径见
    /// [`metric_weight_grouped`]。两种口径都不只看格点下标。
    pub weight: u8,
}

/// 一个拍号在若干小节上的 onset 网格。
///
/// 由 [`metric_grid`] / [`swung_metric_grid`] 构造，也可由
/// [`grouped_metric_grid`] / [`grouped_swung_metric_grid`] 按调用方给的
/// [`BeatGrouping`] 构造，或由 [`crate::genre::GenreRule::rhythm_grid`]
/// 按流派登记数据构造。
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
    build_metric_grid(meter, bars, onsets_per_bar, permille, |cell| {
        metric_weight_in(meter, cell)
    })
}

/// 平直的**加性分组**网格：`permille == None` 的 [`grouped_swung_metric_grid`]。
///
/// # Errors
///
/// 与 [`metric_grid`] 逐条相同（`grouping` 的合法性在 [`BeatGrouping::new`]
/// 已经查过，本函数没有额外的错误路径）。
pub fn grouped_metric_grid(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    grouping: BeatGrouping<'_>,
) -> Result<MetricGrid, TheoryError> {
    grouped_swung_metric_grid(meter, bars, onsets_per_bar, None, grouping)
}

/// 加性分组网格：与 [`swung_metric_grid`] 同语义，但选点的重量读
/// [`metric_weight_grouped`]（分组口径）而不是 [`metric_weight_in`]（拍号口径）。
///
/// 选点规则不变：重量降序、同重量按格点下标升序，再按 tick 排好。
/// 因此分组唯一改变的是"哪些格点更重"，进而改变被选中的格点集合。
///
/// 例（`onsets_per_bar` = 组数时，选出的恰好是每一组的起点）：
///
/// - 7/8、`[2, 2, 3]`、3 个 onset ⇒ 第 0、4、8 格（0、960、1920 tick）；
///   不分组时同请求给出第 0、2、4 格（0、480、960 tick）。
/// - 5/4、`[3, 2]`、2 个 onset ⇒ 第 0、12 格（0、2880 tick）；
///   不分组时给出第 0、4 格（0、960 tick）。
///
/// # Errors
///
/// 与 [`metric_grid`] 逐条相同（`grouping` 的合法性在 [`BeatGrouping::new`]
/// 已经查过，本函数没有额外的错误路径）。
pub fn grouped_swung_metric_grid(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    grouping: BeatGrouping<'_>,
) -> Result<MetricGrid, TheoryError> {
    build_metric_grid(meter, bars, onsets_per_bar, permille, |cell| {
        metric_weight_grouped(meter, cell, grouping)
    })
}

/// 网格构造的公共骨架：`weight_of` 给出每个格点的度量重量。
///
/// 两条口径（[`metric_weight_in`] 与 [`metric_weight_grouped`]）只在**重量**
/// 上不同，因此选点、摇摆位移与全部校验都走这一份实现 —— 免得两份实现漂移。
fn build_metric_grid(
    meter: Meter,
    bars: u32,
    onsets_per_bar: u32,
    permille: Option<u16>,
    weight_of: impl Fn(u32) -> u8,
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
    // 重量读**拍号**（`metric_weight_in`）或**分组**（`metric_weight_grouped`）：
    // 6/8 的第二拍在第 6 格，只看下标的 `metric_weight` 会把第 8 格排到它前面。
    let mut order: Vec<u32> = (0..cells as u32).collect();
    order.sort_by_key(|&cell| (core::cmp::Reverse(weight_of(cell)), cell));
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
                weight: weight_of(cell),
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
    fn metric_weight_in_keeps_binary_meters_bit_identical_to_metric_weight() {
        // 2/4 与 4/4 是纯二分层级 ⇒ 新旧两条口径必须**逐位**相同。
        // 这是回归护栏：新口径若在常用拍号上悄悄改了读数，这条先红。
        for meter in [COMMON, Meter::MARCH] {
            let cells = cells_per_bar(meter).unwrap();
            for cell in 0..cells as u32 {
                assert_eq!(
                    metric_weight_in(meter, cell),
                    metric_weight(cell),
                    "{meter:?} cell {cell}"
                );
            }
        }
    }

    #[test]
    fn metric_weight_in_separates_compound_duple_from_waltz() {
        // 6/8 与 3/4 的小节长度都是 2880 tick、都是 12 个格位，但拍结构不同：
        // 6/8 = 复合二拍（每拍 = 三连八分 = 6 格），3/4 = 三拍（每拍 = 四分 = 4 格）。
        let waltz: Vec<u8> = (0..12)
            .map(|cell| metric_weight_in(Meter::WALTZ, cell))
            .collect();
        assert_eq!(waltz, vec![8, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0]);
        let compound: Vec<u8> = (0..12)
            .map(|cell| metric_weight_in(Meter::COMPOUND_DUPLE, cell))
            .collect();
        assert_eq!(compound, vec![8, 0, 1, 0, 1, 0, 3, 0, 1, 0, 1, 0]);
        assert_ne!(waltz, compound, "6/8 must not be ranked like 3/4");

        // 6/8 的两个强拍在第 0、6 格（0 与 1440 tick = 附点四分），不在第 8 格。
        let grid = metric_grid(Meter::COMPOUND_DUPLE, 1, 2).unwrap();
        assert_eq!(
            grid.hits().iter().map(|hit| hit.cell).collect::<Vec<_>>(),
            vec![0, 6]
        );
        assert_eq!(
            grid.hits().iter().map(|hit| hit.tick).collect::<Vec<_>>(),
            vec![0, 1440]
        );
        // 3/4 的三拍在小节的三等分处。
        let grid = metric_grid(Meter::WALTZ, 1, 3).unwrap();
        assert_eq!(
            grid.hits().iter().map(|hit| hit.cell).collect::<Vec<_>>(),
            vec![0, 4, 8]
        );
        // 整条网格（全部 12 个格位）也不再相同。
        assert_ne!(
            metric_grid(Meter::COMPOUND_DUPLE, 1, 12).unwrap().hits(),
            metric_grid(Meter::WALTZ, 1, 12).unwrap().hits()
        );
    }

    #[test]
    fn metric_weight_in_ranks_seven_eight_and_quintuple_beats_equally() {
        // 7/8：七拍都是八分（每拍 2 格）⇒ 偶数格点是拍（2），奇数格点是十六分（0）。
        let seven: Vec<u8> = (0..14)
            .map(|cell| metric_weight_in(Meter::SEVEN_EIGHT, cell))
            .collect();
        assert_eq!(seven, vec![8, 0, 2, 0, 2, 0, 2, 0, 2, 0, 2, 0, 2, 0]);
        // 5/4：五拍都是四分（每拍 4 格）⇒ 第 16 格（第 5 拍）不再比第 8 格强。
        // 3+2 / 2+3 的分组是作品选择，本口径不发明它（五拍等重）。
        let five: Vec<u8> = (0..20)
            .map(|cell| metric_weight_in(Meter::QUINTUPLE, cell))
            .collect();
        assert_eq!(
            five,
            vec![8, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0]
        );
        assert_eq!(metric_weight(16), 4, "旧口径在第 5 拍给出 4");
        assert_eq!(metric_weight_in(Meter::QUINTUPLE, 16), 2);
    }

    #[test]
    fn compound_meter_boundary_is_pinned() {
        let meter = |numerator: u8, denominator: u8| {
            Meter::new(numerator, denominator).expect("test meters are legal")
        };
        // 复合拍：分母 8、分子是 3 的倍数且大于 3。
        assert!(is_compound_meter(meter(6, 8)));
        assert!(is_compound_meter(meter(9, 8)));
        assert!(is_compound_meter(meter(12, 8)));
        // 边界之外：3/8 与 8/8 的拍都是八分；6/4 的分母不是 8。
        assert!(!is_compound_meter(meter(3, 8)));
        assert!(!is_compound_meter(meter(8, 8)));
        assert!(!is_compound_meter(meter(6, 4)));
        assert!(!is_compound_meter(meter(4, 4)));
        // 拍数：简单拍取分子，复合拍取分子 / 3。
        assert_eq!(felt_beats_per_bar(meter(6, 8)), 2);
        assert_eq!(felt_beats_per_bar(meter(9, 8)), 3);
        assert_eq!(felt_beats_per_bar(meter(12, 8)), 4);
        assert_eq!(felt_beats_per_bar(meter(3, 8)), 3);
        assert_eq!(felt_beats_per_bar(meter(8, 8)), 8);
        assert_eq!(felt_beats_per_bar(meter(6, 4)), 6);
        assert_eq!(felt_beats_per_bar(meter(7, 8)), 7);
        assert_eq!(
            felt_beats_per_bar(Meter {
                numerator: 0,
                denominator: 4
            }),
            0
        );
        // 9/8：三个复合拍起于第 0、6、12 格（0、1440、2880 tick）。
        let nine: Vec<u8> = (0..18)
            .map(|cell| metric_weight_in(meter(9, 8), cell))
            .collect();
        assert_eq!(
            nine,
            vec![8, 0, 1, 0, 1, 0, 3, 0, 1, 0, 1, 0, 3, 0, 1, 0, 1, 0]
        );
        let grid = metric_grid(meter(9, 8), 1, 3).unwrap();
        assert_eq!(
            grid.hits().iter().map(|hit| hit.cell).collect::<Vec<_>>(),
            vec![0, 6, 12]
        );
        assert_eq!(
            grid.hits().iter().map(|hit| hit.tick).collect::<Vec<_>>(),
            vec![0, 1440, 2880]
        );
    }

    #[test]
    fn metric_weight_in_falls_back_instead_of_panicking() {
        // 公有的 Meter 字段可以绕过构造器 ⇒ 非法拍号必须回退，不 panic、不除零。
        let invalid = Meter {
            numerator: 4,
            denominator: 0,
        };
        assert_eq!(metric_weight_in(invalid, 3), metric_weight(3));
        let zero_numerator = Meter {
            numerator: 0,
            denominator: 4,
        };
        assert_eq!(metric_weight_in(zero_numerator, 3), metric_weight(3));
        let not_power_of_two = Meter {
            numerator: 4,
            denominator: 3,
        };
        assert_eq!(metric_weight_in(not_power_of_two, 3), metric_weight(3));
        // 越界格点回退（网格永远只产出 cell < 每小节格点数的 hit）。
        // 边界两侧都要钉：cell == 每小节格点数 已经越界（4/4 的第 16 格）。
        assert_eq!(cells_per_bar(COMMON), Some(16));
        assert_eq!(metric_weight_in(COMMON, 15), 0);
        assert_eq!(metric_weight_in(COMMON, 16), metric_weight(16));
        assert_eq!(metric_weight_in(COMMON, 999), metric_weight(999));
        // 分母为 128 时每小节不足 1 格 ⇒ cells_per_bar 为 0 ⇒ 回退，不除零。
        let tiny = Meter {
            numerator: 2,
            denominator: 128,
        };
        assert_eq!(cells_per_bar(tiny), Some(0));
        assert_eq!(metric_weight_in(tiny, 1), metric_weight(1));
        // 分母为 32 时每小节只有 1 格、却有 2 拍 ⇒ 每拍格位数为 0 ⇒ 回退，不除零。
        let thirty_second = Meter {
            numerator: 2,
            denominator: 32,
        };
        assert_eq!(cells_per_bar(thirty_second), Some(1));
        assert_eq!(felt_beats_per_bar(thirty_second), 2);
        assert_eq!(metric_weight_in(thirty_second, 0), metric_weight(0));
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

    // -----------------------------------------------------------------------
    // 加性拍分组：`BeatGrouping`
    // -----------------------------------------------------------------------

    /// 分组合法性：覆盖全部拍才给 `Some`，其余一律 `None`（不截断、不补齐）。
    #[test]
    fn beat_grouping_accepts_only_coverings_of_every_beat() {
        let meter = |numerator: u8, denominator: u8| Meter::new(numerator, denominator).unwrap();

        // 合法：各组之和恰好等于拍数。
        assert_eq!(
            BeatGrouping::new(COMMON, &[2, 2]).unwrap().groups(),
            &[2, 2]
        );
        assert_eq!(BeatGrouping::new(COMMON, &[4]).unwrap().groups(), &[4]);
        assert_eq!(
            BeatGrouping::new(Meter::SEVEN_EIGHT, &[2, 2, 3])
                .unwrap()
                .groups(),
            &[2, 2, 3]
        );
        assert_eq!(
            BeatGrouping::new(Meter::SEVEN_EIGHT, &[3, 2, 2])
                .unwrap()
                .groups(),
            &[3, 2, 2]
        );
        assert_eq!(
            BeatGrouping::new(Meter::QUINTUPLE, &[3, 2])
                .unwrap()
                .groups(),
            &[3, 2]
        );
        // 6/8 的拍是**复合拍**（两个附点四分），不是六个八分 ⇒ 分组按 2 拍记。
        assert_eq!(
            BeatGrouping::new(Meter::COMPOUND_DUPLE, &[1, 1])
                .unwrap()
                .beats(),
            2
        );
        // 11/8 不是复合拍（11 不是 3 的倍数）⇒ 十一拍，分组和必须等于 11。
        let eleven = meter(11, 8);
        assert_eq!(felt_beats_per_bar(eleven), 11);
        assert_eq!(
            BeatGrouping::new(eleven, &[2, 2, 3, 2, 2])
                .unwrap()
                .groups(),
            &[2, 2, 3, 2, 2]
        );

        // 不合法：空、含 0、和不等于拍数、拍数为 0 的非法拍号。
        assert!(BeatGrouping::new(COMMON, &[]).is_none());
        assert!(BeatGrouping::new(COMMON, &[2, 2, 0]).is_none());
        assert!(BeatGrouping::new(COMMON, &[2, 3]).is_none());
        assert!(BeatGrouping::new(COMMON, &[1, 1, 1]).is_none());
        assert!(BeatGrouping::new(Meter::SEVEN_EIGHT, &[2, 2, 2]).is_none());
        assert!(BeatGrouping::new(Meter::SEVEN_EIGHT, &[7]).is_some());
        assert!(
            BeatGrouping::new(
                Meter {
                    numerator: 0,
                    denominator: 4
                },
                &[1]
            )
            .is_none()
        );
        // 分子非 0、分母非法：拍数还算得出来（4）⇒ 构造成功；非法**拍号**
        // 由网格构造器拒绝（见 `grouped_grids_reject_illegal_meters_like_the_builtin_ones`）。
        assert!(
            BeatGrouping::new(
                Meter {
                    numerator: 4,
                    denominator: 0
                },
                &[2, 2]
            )
            .is_some()
        );
    }

    /// 组起点的判定：第 0 拍恒是起点，越界的拍不是。
    #[test]
    fn beat_grouping_marks_exactly_the_group_boundaries() {
        let seven = BeatGrouping::new(Meter::SEVEN_EIGHT, &[2, 2, 3]).unwrap();
        assert_eq!(seven.group_count(), 3);
        let starts: Vec<u8> = (0..7).filter(|&beat| seven.is_group_start(beat)).collect();
        assert_eq!(starts, vec![0, 2, 4]);
        assert!(!seven.is_group_start(7));
        assert!(!seven.is_group_start(255));

        let five = BeatGrouping::new(Meter::QUINTUPLE, &[3, 2]).unwrap();
        let starts: Vec<u8> = (0..5).filter(|&beat| five.is_group_start(beat)).collect();
        assert_eq!(starts, vec![0, 3]);
    }

    /// 每条拍号都有一条分组能把**内置口径逐位复现**；换一条才改读数。
    #[test]
    fn metric_weight_grouped_reproduces_the_builtin_hierarchy_on_regular_meters() {
        let meter = |numerator: u8, denominator: u8| Meter::new(numerator, denominator).unwrap();
        // (拍号, 与 `metric_weight_in` 逐位相同的分组)
        let cases: [(Meter, &[u8]); 8] = [
            (Meter::MARCH, &[2]),
            (Meter::WALTZ, &[3]),
            (COMMON, &[2, 2]),
            (Meter::QUINTUPLE, &[5]),
            (Meter::SEVEN_EIGHT, &[7]),
            (Meter::COMPOUND_DUPLE, &[1, 1]),
            (meter(9, 8), &[1, 1, 1]),
            (meter(12, 8), &[1, 1, 1, 1]),
        ];
        for (index, (meter, groups)) in cases.iter().enumerate() {
            let grouping =
                BeatGrouping::new(*meter, groups).expect("case grouping covers every beat");
            let cells = cells_per_bar(*meter).unwrap();
            for cell in 0..cells as u32 {
                assert_eq!(
                    metric_weight_grouped(*meter, cell, grouping),
                    metric_weight_in(*meter, cell),
                    "case {index}: {meter:?} cell {cell}"
                );
            }
        }
        // 8/8 是**例外**：内置口径只在半小节（第 4 拍）给强拍，任何分组都会把
        // 每一组的起点判成强拍 ⇒ 没有分组能逐位复现它。
        let eight_eight = meter(8, 8);
        let grouping = BeatGrouping::new(eight_eight, &[3, 3, 2]).unwrap();
        assert_eq!(metric_weight_in(eight_eight, 6), 2);
        assert_eq!(metric_weight_grouped(eight_eight, 6, grouping), 3);
    }

    /// 加性分组真的把 onset 移到组起点上（字面读数）。
    #[test]
    fn grouping_moves_the_onsets_of_an_irregular_meter_to_the_group_starts() {
        let meter = |numerator: u8, denominator: u8| Meter::new(numerator, denominator).unwrap();

        // 7/8 = 2+2+3、3 个 onset（= 组数）⇒ 恰好取三个组起点：第 0、4、8 格。
        let seven = meter(7, 8);
        let grouped =
            grouped_metric_grid(seven, 1, 3, BeatGrouping::new(seven, &[2, 2, 3]).unwrap())
                .unwrap();
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 4, 8]
        );
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.tick)
                .collect::<Vec<_>>(),
            vec![0, 960, 1920]
        );
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.weight)
                .collect::<Vec<_>>(),
            vec![MAX_METRIC_WEIGHT, STRONG_BEAT_WEIGHT, STRONG_BEAT_WEIGHT]
        );
        // 不分组时同一个请求取到最靠前的三拍（第 0、2、4 格）。
        let plain = metric_grid(seven, 1, 3).unwrap();
        assert_eq!(
            plain.hits().iter().map(|hit| hit.cell).collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(
            plain.hits().iter().map(|hit| hit.tick).collect::<Vec<_>>(),
            vec![0, 480, 960]
        );

        // 5/4 = 3+2、2 个 onset ⇒ 第 0、12 格（0、2880 tick）；3+2 与 2+3 不同。
        let five = Meter::QUINTUPLE;
        let three_two =
            grouped_metric_grid(five, 1, 2, BeatGrouping::new(five, &[3, 2]).unwrap()).unwrap();
        assert_eq!(
            three_two
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 12]
        );
        assert_eq!(
            three_two
                .hits()
                .iter()
                .map(|hit| hit.tick)
                .collect::<Vec<_>>(),
            vec![0, 2880]
        );
        let two_three =
            grouped_metric_grid(five, 1, 2, BeatGrouping::new(five, &[2, 3]).unwrap()).unwrap();
        assert_eq!(
            two_three
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 8]
        );
        assert_eq!(
            two_three
                .hits()
                .iter()
                .map(|hit| hit.tick)
                .collect::<Vec<_>>(),
            vec![0, 1920]
        );
        assert_ne!(three_two.hits(), two_three.hits());
        let plain = metric_grid(five, 1, 2).unwrap();
        assert_eq!(
            plain.hits().iter().map(|hit| hit.cell).collect::<Vec<_>>(),
            vec![0, 4]
        );
        assert_eq!(
            plain.hits().iter().map(|hit| hit.tick).collect::<Vec<_>>(),
            vec![0, 960]
        );

        // 8/8 = 3+3+2、3 个 onset ⇒ 第 0、6、12 格（0、1440、2880 tick）。
        let eight = meter(8, 8);
        let grouped =
            grouped_metric_grid(eight, 1, 3, BeatGrouping::new(eight, &[3, 3, 2]).unwrap())
                .unwrap();
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 6, 12]
        );
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.tick)
                .collect::<Vec<_>>(),
            vec![0, 1440, 2880]
        );
        // 不分组时内置口径只在半小节处给强拍 ⇒ 第 0、2、8 格。
        assert_eq!(
            metric_grid(eight, 1, 3)
                .unwrap()
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 2, 8]
        );

        // 11/8 = 2+2+3+2+2（`docs/ledger/theory-core-notes.md` §5 记的"尚未登记"的
        // 复杂拍号）：5 个 onset ⇒ 第 0、4、8、14、18 格。
        let eleven = meter(11, 8);
        let grouped = grouped_metric_grid(
            eleven,
            1,
            5,
            BeatGrouping::new(eleven, &[2, 2, 3, 2, 2]).unwrap(),
        )
        .unwrap();
        assert_eq!(grouped.ticks_per_bar(), 5280);
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 4, 8, 14, 18]
        );
        assert_eq!(
            grouped
                .hits()
                .iter()
                .map(|hit| hit.tick)
                .collect::<Vec<_>>(),
            vec![0, 960, 1920, 3360, 4320]
        );
    }

    /// 分组与内置口径在常规拍号上给出**逐位相同**的网格（回归护栏：
    /// 分组不该动 4/4、3/4、6/8 这些本来就读对的拍号）。
    #[test]
    fn grouping_leaves_the_regular_meters_bit_identical() {
        let cases: [(Meter, &[u8]); 5] = [
            (Meter::MARCH, &[2]),
            (Meter::WALTZ, &[3]),
            (COMMON, &[2, 2]),
            (Meter::COMPOUND_DUPLE, &[1, 1]),
            (Meter::SEVEN_EIGHT, &[7]),
        ];
        for (meter, groups) in cases {
            let grouping = BeatGrouping::new(meter, groups).unwrap();
            for onsets in [1u32, 2, 3, 5] {
                assert_eq!(
                    grouped_metric_grid(meter, 3, onsets, grouping)
                        .unwrap()
                        .hits(),
                    metric_grid(meter, 3, onsets).unwrap().hits(),
                    "{meter:?} onsets {onsets}"
                );
            }
        }
        // 4/4 = `[4]`（一个大组）不是内置口径：第 2 拍（第 8 格）不再是强拍。
        let one_group =
            grouped_metric_grid(COMMON, 1, 2, BeatGrouping::new(COMMON, &[4]).unwrap()).unwrap();
        assert_eq!(
            one_group
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 4]
        );
        assert_eq!(
            metric_grid(COMMON, 1, 2)
                .unwrap()
                .hits()
                .iter()
                .map(|hit| hit.cell)
                .collect::<Vec<_>>(),
            vec![0, 8]
        );
    }

    /// 拿**另一个拍号**的分组来算重量 ⇒ 回退到内置口径，不 panic、不误算。
    #[test]
    fn a_grouping_from_another_meter_falls_back_to_the_builtin_hierarchy() {
        let common_grouping = BeatGrouping::new(COMMON, &[2, 2]).unwrap();
        for cell in 0..14u32 {
            assert_eq!(
                metric_weight_grouped(Meter::SEVEN_EIGHT, cell, common_grouping),
                metric_weight_in(Meter::SEVEN_EIGHT, cell),
                "cell {cell}"
            );
        }
        // 网格同样逐位回退。
        assert_eq!(
            grouped_metric_grid(Meter::SEVEN_EIGHT, 2, 3, common_grouping)
                .unwrap()
                .hits(),
            metric_grid(Meter::SEVEN_EIGHT, 2, 3).unwrap().hits()
        );
        // 非法拍号（公有的 Meter 字段可以绕过构造器）也回退，不 panic、不除零。
        let invalid = Meter {
            numerator: 4,
            denominator: 0,
        };
        let grouping = BeatGrouping::new(
            Meter {
                numerator: 4,
                denominator: 4,
            },
            &[2, 2],
        )
        .unwrap();
        assert_eq!(
            metric_weight_grouped(invalid, 3, grouping),
            metric_weight(3)
        );
        // 越界格点回退。
        assert_eq!(
            metric_weight_grouped(COMMON, 16, common_grouping),
            metric_weight(16)
        );
        // 每拍格位数为 0 的病态拍号（2/32：每小节只有 1 格，却有 2 拍）也回退，
        // 不除零。这条判据是**注入法**逼出来的：把该分支的 `return` 改成
        // `return 0` 时没有任何判据变红（实测 132 单元 + 36 集成 + 1 doctest
        // 全绿）⇒ 该分支当时没有被覆盖，补上这条读数为证。
        let thirty_second = Meter {
            numerator: 2,
            denominator: 32,
        };
        assert_eq!(cells_per_bar(thirty_second), Some(1));
        assert_eq!(felt_beats_per_bar(thirty_second), 2);
        let two_beats = BeatGrouping::new(thirty_second, &[2]).unwrap();
        assert_eq!(
            metric_weight_grouped(thirty_second, 0, two_beats),
            metric_weight_in(thirty_second, 0)
        );
        assert_eq!(metric_weight_grouped(thirty_second, 0, two_beats), 8);
    }

    /// 分组网格的错误路径与内置口径**逐条相同**（分组不新增错误）。
    #[test]
    fn grouped_grids_reject_illegal_meters_like_the_builtin_ones() {
        let grouping = BeatGrouping::new(COMMON, &[2, 2]).unwrap();
        assert_eq!(
            grouped_metric_grid(COMMON, 0, 4, grouping).unwrap_err(),
            TheoryError::ZeroBars
        );
        assert_eq!(
            grouped_metric_grid(
                Meter {
                    numerator: 4,
                    denominator: 0
                },
                1,
                1,
                grouping
            )
            .unwrap_err(),
            TheoryError::ZeroBars
        );
        assert_eq!(
            grouped_metric_grid(COMMON, 1, 17, grouping).unwrap_err(),
            TheoryError::ProgressionTooDense {
                degrees: 17,
                slots: 16
            }
        );
        assert_eq!(
            grouped_swung_metric_grid(COMMON, 1, 2, Some(499), grouping).unwrap_err(),
            TheoryError::SwingOutOfRange { value: 499 }
        );
        // 平直入口与"摇摆 = 500 千分比"逐位相同，只有回显不同。
        let plain = grouped_metric_grid(COMMON, 2, 9, grouping).unwrap();
        let straight =
            grouped_swung_metric_grid(COMMON, 2, 9, Some(SWING_PERMILLE_STRAIGHT), grouping)
                .unwrap();
        assert_eq!(plain.hits(), straight.hits());
        assert_eq!(plain.swing_permille(), None);
        assert_eq!(straight.swing_permille(), Some(SWING_PERMILLE_STRAIGHT));
    }

    /// 分组网格保持全部结构不变量：数量、严格升序、落在本小节内，
    /// 且每个 hit 的重量与公开的重量函数逐位一致。
    #[test]
    fn grouped_grids_keep_every_structural_invariant() {
        let meter = |numerator: u8, denominator: u8| Meter::new(numerator, denominator).unwrap();
        let cases: [(Meter, &[u8]); 6] = [
            (COMMON, &[2, 2]),
            (COMMON, &[4]),
            (Meter::WALTZ, &[3]),
            (Meter::SEVEN_EIGHT, &[2, 2, 3]),
            (Meter::QUINTUPLE, &[3, 2]),
            (meter(11, 8), &[2, 2, 3, 2, 2]),
        ];
        for (meter, groups) in cases {
            let grouping = BeatGrouping::new(meter, groups).unwrap();
            let cells = u32::try_from(cells_per_bar(meter).unwrap()).unwrap();
            for onsets in [0u32, 1, 3, 5, cells] {
                for permille in [None, Some(500u16), Some(667), Some(1000)] {
                    let grid =
                        grouped_swung_metric_grid(meter, 3, onsets, permille, grouping).unwrap();
                    assert_eq!(grid.len(), 3 * onsets as usize, "{meter:?} onsets {onsets}");
                    let bar_ticks = meter.ticks_per_bar();
                    for pair in grid.hits().windows(2) {
                        assert!(pair[0].tick < pair[1].tick, "{pair:?}");
                    }
                    for hit in grid.hits() {
                        let bar_start = u64::from(hit.bar) * bar_ticks;
                        assert!(
                            hit.tick >= bar_start && hit.tick < bar_start + bar_ticks,
                            "{meter:?} hit {hit:?}"
                        );
                        assert_eq!(
                            hit.weight,
                            metric_weight_grouped(meter, hit.cell, grouping),
                            "{meter:?} hit {hit:?}"
                        );
                    }
                    // 每小节的 onset 数恒定，且按小节连续铺满。
                    for bar in 0..3u32 {
                        assert_eq!(grid.hits_in_bar(bar).len(), onsets as usize);
                    }
                    assert!(grid.hits_in_bar(3).is_empty());
                }
            }
        }
    }
}
