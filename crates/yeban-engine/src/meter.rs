//! VU / 峰值电平计量的独立 SPSC 解耦与**实时侧电平状态机**。[ARCH-UI-002, ROAD-M2-008]
//!
//! 规范原文（[ARCH-UI-002]）：
//!
//! > UI 线程绝不直接读取音频实时上下文。实时线程以 60Hz 速度向无锁 Meter SPSC
//! > 压入真峰值与 RMS 电平，UI 线程定时器批量出队更新 Slint Properties。
//!
//! 工程含义：
//!
//! 1. **独立队列**：电平数据量远大于参数事件（每轨每量子一条），必须与参数队列分开，
//!    否则电平洪峰会挤掉参数/音符事件 [ROAD-M2-008]；
//! 2. **高容量**：UI 以 60Hz 抽干，音频线程以 `sample_rate / block_frames` Hz 生产
//!    （48kHz / 128 = 375Hz）⇒ 一个 UI 帧积压 ~6 条/轨。默认容量 8192 条足够 64 轨；
//! 3. **有损**：队列满时 `rtrb::Producer::push_partial_slice` 只写入装得下的**前缀**，
//!    被丢的是**最新**的那几条帧（不是最旧的）。丢一帧只是 UI 少跳一次，
//!    而阻塞音频线程会直接爆音 —— 所以宁可丢。**丢了多少必须可观测**：
//!    [`MeterPublisher::dropped`] > 0 就是"UI 已经落后到会丢最新帧"的健康告警；
//!    每帧都带 `quantum`，UI 因此能判断自己看到的是不是最新的
//!    （见 [`MeterBoard::latest_quantum`]）。
//!
//! 生产侧使用批量 API（一次 `push_partial_slice` 推送本量子的全部轨），
//! 消费侧有两条路：
//!
//! | 消费 API | 语义 | 契约 |
//! | :--- | :--- | :--- |
//! | [`MeterCollector::tick`] | FIFO：把当前可读的**前缀**搬到 scratch | 每 tick **恰好一次**批量读 |
//! | [`MeterCollector::drain_latest`] | 丢弃旧帧、只留**每节点最新**一帧 | 循环批量读直到抽干 |
//!
//! ## 真峰值 / 真 RMS 从哪来
//!
//! 电平口径全部在 [`crate::level`]（零依赖纯函数/纯状态机）：峰值、峰值保持（20 dB/s
//! 指数释放）、块 RMS、平滑 RMS（τ=300 ms 一阶低通）、dBFS 换算、`NaN`/`±∞` 钳位。
//! 本模块负责**把口径接到 SPSC 上**：
//!
//! - [`MeterBank`] 是音频线程私有的**每节点状态机**（定长数组 + 线性对齐，零分配）；
//! - [`MeterFrame`] 是过队列的 `Copy` 载荷（无 `Drop` ⇒ 音频线程出队不会释放内存 [红线 7]）。
//!
//! ## 结构性契约（判据钉住的）
//!
//! - 音频线程**每个量子恰好一次** `publish`，一次推 `轨道数 + 母线 1 条`；
//! - `publish` 绝不阻塞、绝不扩容（满则丢并计数）；
//! - `NaN`/`Inf` 输入不得让任何一帧变成 `NaN`（[`MeterFrame::is_sane`]）；
//! - **弹道系数属于池**：[`MeterBank::set_quanta_per_second`] 记下本池当前的每秒
//!   量子数，**惰性建立**的槽位与 [`MeterBank::reset`] 都按它建检测器 ⇒ 一条轨的
//!   弹道不取决于"它的槽位是在哪一次修订被建立的"。

use rtrb::{Consumer, Producer, RingBuffer};
use yeban_model::EntityId;

use crate::level::{
    self, DEFAULT_PEAK_DECAY_DB_PER_SEC, DEFAULT_QUANTA_PER_SECOND, DEFAULT_RMS_TIME_CONSTANT_SEC,
    LevelDetector, LevelReading, dbfs, dbfs_clamped,
};

/// 把每秒量子数钳到合法域：与 `LevelDetector::set_ballistics` 的回落口径**逐字相同**
/// （非有限或 ≤ 0 ⇒ [`DEFAULT_QUANTA_PER_SECOND`]）。
///
/// 池必须与器件用**同一个**口径判合法，否则"池记下的数"与"器件实际用的系数"会分叉。
#[must_use]
fn sanitise_quanta_per_second(quanta_per_second: f32) -> f32 {
    if quanta_per_second.is_finite() && quanta_per_second > 0.0 {
        quanta_per_second
    } else {
        DEFAULT_QUANTA_PER_SECOND
    }
}

/// 按**给定的**每秒量子数建一台电平检测器（沿用默认的 20 dB/s 与 τ = 300 ms）。
///
/// 它与 `LevelDetector::set_quanta_per_second`（本池刷新在册槽位时用的那一个）
/// **是同一条算术**：两者都调用 `set_ballistics(qps, 默认 dB/s, 默认 τ)` ⇒
/// "刷新一个已有槽位"与"新建一个槽位"得到的系数**逐位相同**。
#[must_use]
fn detector_at(quanta_per_second: f32) -> LevelDetector {
    LevelDetector::with_ballistics(
        quanta_per_second,
        DEFAULT_PEAK_DECAY_DB_PER_SEC,
        DEFAULT_RMS_TIME_CONSTANT_SEC,
    )
}

/// 电平队列默认容量（条）。
pub const DEFAULT_METER_CAPACITY: usize = 8192;

/// UI 侧建议的栈上临时缓冲长度。
pub const SCRATCH_METERS: usize = 256;

/// [`MeterCollector::drain_latest`] 的栈上搬运块长度（条）。
///
/// 一次 `pop_partial_slice` 最多搬这么多条；抽干就是循环若干次。
/// 选 128：栈上 128 × `sizeof(MeterFrame)`，对 UI 线程完全无压力。
pub const DRAIN_CHUNK: usize = 128;

/// 一条电平计量帧（`Copy`，无 `Drop` ⇒ 音频线程出队不会释放内存 [红线 7]）。
///
/// 四个电平字段都是**线性幅度**（1.0 = 0 dBFS），并且保证有限
/// （[`is_sane`](Self::is_sane)）；dBFS 换算在 UI 侧按需做，见
/// [`peak_dbfs`](Self::peak_dbfs) / [`peak_hold_dbfs`](Self::peak_hold_dbfs) /
/// [`rms_dbfs`](Self::rms_dbfs)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterFrame {
    /// 计量对象（音轨 / 总线身份）。
    pub node: EntityId,
    /// 量子序号（`sample_rate / block_frames` 递增），UI 用它判断数据新鲜度。
    pub quantum: u64,
    /// 本量子绝对值峰值（线性幅度）。
    pub peak: f32,
    /// 峰值保持：`max(peak, 上一量子保持 × 释放乘子)`，默认 20 dB/s。
    pub peak_hold: f32,
    /// 本量子 RMS（线性幅度）。
    pub rms: f32,
    /// 平滑 RMS：均方经一阶低通（τ=300 ms）后开方。
    pub rms_smoothed: f32,
}

impl MeterFrame {
    /// 用"本量子块读数"直接构造一帧（`peak_hold = peak`、`rms_smoothed = rms`）。
    ///
    /// 这是给**无状态**场景（离线分析、测试夹具）用的；实时路径走 [`MeterBank`]，
    /// 它带跨量子的弹道状态。
    #[must_use]
    pub const fn new(node: EntityId, quantum: u64, peak: f32, rms: f32) -> Self {
        Self {
            node,
            quantum,
            peak,
            peak_hold: peak,
            rms,
            rms_smoothed: rms,
        }
    }

    /// 从 [`LevelReading`] 构造（实时路径用的那个）。
    #[must_use]
    pub const fn from_reading(node: EntityId, quantum: u64, reading: LevelReading) -> Self {
        Self {
            node,
            quantum,
            peak: reading.peak,
            peak_hold: reading.peak_hold,
            rms: reading.rms,
            rms_smoothed: reading.rms_smoothed,
        }
    }

    /// 从一块样本算出一条**无状态**计量帧（**不做任何分配**）。
    ///
    /// 空切片返回全零帧（RMS 定义为 0，而不是 `0/0 = NaN` —— NaN 会污染 UI 曲线）。
    /// `NaN`/`±∞` 输入经 [`crate::level::sanitize_sample`] 钳位。
    #[must_use]
    pub fn measure(node: EntityId, quantum: u64, samples: &[f32]) -> Self {
        let mut detector = LevelDetector::new();
        Self::from_reading(node, quantum, detector.analyze(samples))
    }

    /// 帧内四个电平字段是否都是有限数（`NaN`/`±∞` 一律为假）。
    #[must_use]
    pub fn is_sane(&self) -> bool {
        self.peak.is_finite()
            && self.peak_hold.is_finite()
            && self.rms.is_finite()
            && self.rms_smoothed.is_finite()
    }

    /// 峰值的 dBFS（`peak <= 0` 时返回负无穷）。
    #[must_use]
    pub fn peak_dbfs(&self) -> f32 {
        dbfs(self.peak)
    }

    /// 峰值保持的 dBFS。
    #[must_use]
    pub fn peak_hold_dbfs(&self) -> f32 {
        dbfs(self.peak_hold)
    }

    /// 平滑 RMS 的 dBFS。
    #[must_use]
    pub fn rms_dbfs(&self) -> f32 {
        dbfs(self.rms_smoothed)
    }

    /// 峰值 dBFS，按下限钳位（UI 柱高：静音时给有限值而不是 -∞）。
    #[must_use]
    pub fn peak_dbfs_clamped(&self, floor_dbfs: f32) -> f32 {
        dbfs_clamped(self.peak, floor_dbfs)
    }
}

/// 电平队列的生产端（**音频线程**持有）。
#[derive(Debug)]
pub struct MeterPublisher {
    producer: Producer<MeterFrame>,
    bulk_push_calls: u64,
    events_pushed: u64,
    dropped: u64,
}

impl MeterPublisher {
    /// 队列容量（条）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.producer.buffer().capacity()
    }

    /// 批量推送本量子的全部电平帧，返回实际写入条数。
    ///
    /// 恰好一次 `push_partial_slice`（见 [`crate::ring`] 的批量契约）[ROAD-M2-007]。
    /// 队列满时多余的帧被丢弃（`rtrb` 丢的是**前缀之后**的部分，即最新帧）
    /// 并计入 [`dropped`](Self::dropped) —— **绝不阻塞** [红线 7]。
    pub fn publish(&mut self, frames: &[MeterFrame]) -> usize {
        self.bulk_push_calls = self.bulk_push_calls.saturating_add(1);
        let (written, _remainder) = self.producer.push_partial_slice(frames);
        let pushed = written.len();
        self.events_pushed = self.events_pushed.saturating_add(pushed as u64);
        self.dropped = self.dropped.saturating_add((frames.len() - pushed) as u64);
        pushed
    }

    /// 累计批量写次数（**结构性判据**：应等于"有快照的量子数"）。
    #[must_use]
    pub const fn bulk_push_calls(&self) -> u64 {
        self.bulk_push_calls
    }

    /// 累计写入帧数。
    #[must_use]
    pub const fn frames_pushed(&self) -> u64 {
        self.events_pushed
    }

    /// 累计因队列满而丢弃的帧数（UI 若持续不抽，这个数字会上升 —— 可观测的健康指标）。
    ///
    /// ⚠ 本句柄归**音频线程**所有（`EngineRuntime` 的私有字段）⇒ 设备腿下控制面读不到它。
    /// 引擎把**同一个**计数器转发进 [`crate::rt::EngineStats::meter_dropped_frames`]
    /// （以及跨线程只读镜像），控制面因此不必拿到本句柄。
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// 电平队列的消费端（**UI 线程**持有，60Hz 批量抽干）。
#[derive(Debug)]
pub struct MeterCollector {
    consumer: Consumer<MeterFrame>,
    bulk_pop_calls: u64,
    frames_drained: u64,
    frames_superseded: u64,
    ticks: u64,
}

impl MeterCollector {
    /// 当前积压帧数。
    #[must_use]
    pub fn pending(&self) -> usize {
        self.consumer.slots()
    }

    /// 一次 60Hz 抽取：把当前可读的帧**按 FIFO 前缀**批量搬到 `scratch` 前台，返回条数。
    ///
    /// 恰好一次 `pop_partial_slice`。一次抽不完就留到下一帧（下一帧再抽），
    /// 因此 `scratch` 给大一点可以显著降低"UI 永远追不上"的风险。
    /// 只要"丢弃旧帧"比"取回全部"更重要，就应当改用 [`drain_latest`](Self::drain_latest)。
    pub fn tick(&mut self, scratch: &mut [MeterFrame]) -> usize {
        self.ticks = self.ticks.saturating_add(1);
        if scratch.is_empty() {
            return 0;
        }
        self.bulk_pop_calls = self.bulk_pop_calls.saturating_add(1);
        let (filled, _remainder) = self.consumer.pop_partial_slice(scratch);
        let drained = filled.len();
        self.frames_drained = self.frames_drained.saturating_add(drained as u64);
        drained
    }

    /// 一轮 UI 定时器里**抽干整个积压**，并只保留每个节点的最新一帧。
    ///
    /// 返回 `scratch` 中填好的条数（= 本轮见到的不同节点数，≤ `scratch.len()`）。
    /// 这就是"丢弃旧帧、只取最新"的直接实现：
    ///
    /// - 队列里的每一帧都会被**消费掉**（因此积压不会无限增长）；
    /// - 每节点只保留 `quantum` 最大的一帧（同量子重复投递取后者）；
    /// - 全过程中零分配（搬运块在栈上，去重用的就是调用方的 `scratch`）。
    ///
    /// `scratch` 装不下所有节点时：队列**仍然被抽干**（防止积压），装不下的节点帧被丢弃并
    /// 计入 [`frames_superseded`](Self::frames_superseded) —— 下一轮会重新拿到它们的最新帧。
    ///
    /// `frames_superseded` 统计的是**真正被丢弃的帧**：同一节点被更新的一帧覆盖掉的旧帧，
    /// 加上 `scratch` 装不下的帧（`quantum` 更旧的迟到帧也算在内）。
    ///
    /// 结构性契约：`bulk_pop_calls` 每轮增加 `⌈本轮积压 / DRAIN_CHUNK⌉` 次，
    /// 积压恰好是 `DRAIN_CHUNK` 的整数倍时多一次空读（与 [`tick`](Self::tick) 的
    /// "每 tick 恰好一次"不同，见模块文档的表）。
    pub fn drain_latest(&mut self, scratch: &mut [MeterFrame]) -> usize {
        self.ticks = self.ticks.saturating_add(1);
        if scratch.is_empty() {
            return 0;
        }
        let mut chunk = [MeterFrame::default(); DRAIN_CHUNK];
        let mut unique = 0usize;
        let mut consumed = 0usize;
        let mut discarded = 0usize;
        loop {
            self.bulk_pop_calls = self.bulk_pop_calls.saturating_add(1);
            let (filled, _remainder) = self.consumer.pop_partial_slice(&mut chunk);
            let taken = filled.len();
            if taken == 0 {
                break;
            }
            consumed += taken;
            for frame in filled.iter() {
                match scratch[..unique]
                    .iter()
                    .position(|slot| slot.node == frame.node)
                {
                    Some(index) => {
                        if level::supersedes(frame.quantum, scratch[index].quantum) {
                            // 覆盖掉的那一帧被丢弃。
                            scratch[index] = *frame;
                            discarded += 1;
                        } else {
                            // 迟到的旧帧: 直接丢弃。
                            discarded += 1;
                        }
                    }
                    None => {
                        if unique < scratch.len() {
                            scratch[unique] = *frame;
                            unique += 1;
                        } else {
                            discarded += 1;
                        }
                    }
                }
            }
            if taken < DRAIN_CHUNK {
                break;
            }
        }
        self.frames_drained = self.frames_drained.saturating_add(consumed as u64);
        self.frames_superseded = self.frames_superseded.saturating_add(discarded as u64);
        unique
    }

    /// 累计批量读次数（结构性判据用）。
    #[must_use]
    pub const fn bulk_pop_calls(&self) -> u64 {
        self.bulk_pop_calls
    }

    /// 累计抽干帧数（从队列真正取走的条数）。
    #[must_use]
    pub const fn frames_drained(&self) -> u64 {
        self.frames_drained
    }

    /// 累计被丢弃的帧数：同一节点被更新的一帧覆盖掉的旧帧，加上 `scratch` 容量装不下的帧。
    /// `drain_latest` 的去重成果（结构性判据用）。
    #[must_use]
    pub const fn frames_superseded(&self) -> u64 {
        self.frames_superseded
    }

    /// 累计 60Hz tick / drain 次数。
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }
}

/// UI 侧"每节点最新电平"面板 [ARCH-UI-002]。
///
/// 电平是**观测量**：UI 只关心每节点最后一个值，中间的过渡帧可以直接覆盖。
/// 因此面板只需要"喂入本次抽到的帧 → 覆盖对应节点"，不需要保留历史。
///
/// 覆盖规则是 [`level::supersedes`]：候选帧量子**不早于**已持有值才被接受
/// ⇒ 迟到的旧帧（乱序、重放、跨 tick 交错）永远不会把新值顶回去。
///
/// 这个类型只活在 UI 线程上，所以允许 `BTreeMap` 插入（可能分配）。仍然用 `BTreeMap`
/// 而不是 `HashMap`，以保持与项目红线 4 一致的确定性迭代顺序（UI 元素顺序稳定）。
#[derive(Clone, Debug, Default)]
pub struct MeterBoard {
    latest: std::collections::BTreeMap<EntityId, MeterFrame>,
}

impl MeterBoard {
    /// 空面板。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一批帧；同节点的旧值被覆盖，返回被更新的节点数。
    pub fn ingest(&mut self, frames: &[MeterFrame]) -> usize {
        let mut updated = 0usize;
        for frame in frames {
            match self.latest.get(&frame.node) {
                Some(previous) if !level::supersedes(frame.quantum, previous.quantum) => {}
                _ => {
                    self.latest.insert(frame.node, *frame);
                    updated += 1;
                }
            }
        }
        updated
    }

    /// 某个节点的最新电平。
    #[must_use]
    pub fn latest(&self, node: &EntityId) -> Option<&MeterFrame> {
        self.latest.get(node)
    }

    /// 面板里最大的量子序号（UI 判断"我看到的是不是最新的"）。
    ///
    /// 与音频线程当前的量子序号比较：落后很多 ⇒ 要么 UI 没抽干，要么队列满丢了最新帧
    /// （此时 [`MeterPublisher::dropped`] > 0）。
    #[must_use]
    pub fn latest_quantum(&self) -> Option<u64> {
        self.latest.values().map(|frame| frame.quantum).max()
    }

    /// 已知节点数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.latest.len()
    }

    /// 是否还没有任何电平。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.latest.is_empty()
    }

    /// 按节点身份升序迭代（确定性）。
    pub fn iter(&self) -> impl Iterator<Item = (&EntityId, &MeterFrame)> {
        self.latest.iter()
    }
}

/// 音频线程私有的**每节点电平状态**槽。
///
/// `Copy` 且无 `Drop`：可以在定长数组里原位创建/淘汰（零分配、零锁）。
#[derive(Clone, Copy, Debug)]
struct MeterSlot {
    node: Option<EntityId>,
    detector: LevelDetector,
    /// 最近一次被计量的量子序号（LRU 淘汰用；`0` = 从未）。
    touched: u64,
}

impl MeterSlot {
    /// 未激活槽。只作为定长数组的初值；被激活时一律经 [`MeterSlot::fresh`] 重建为
    /// 默认弹道。占位检测器是**无弹道**配置（合法的 `const` 值），不参与测量。
    const EMPTY: Self = Self {
        node: None,
        detector: LevelDetector::silent(),
        touched: 0,
    };

    /// 为一个节点新建激活槽（**按池当前的弹道**，不是按器件默认口径）。
    ///
    /// ⚠ 这里曾经写 `LevelDetector::new()`：那是硬编码的 375 量子/s（48 kHz）⇒
    /// 一个在**换采样率之后**才第一次被计量的节点会拿到旧采样率的系数，而它每个量子
    /// 仍被推进一次 ⇒ 峰值保持按 `20 dB/s × (本率 / 48 kHz)` 回落（96 kHz 下是
    /// **40 dB/s**），RMS 时间常数同比例失真。判据见
    /// `tests::a_slot_activated_after_the_rate_change_uses_the_armed_ballistics`。
    fn fresh(node: EntityId, quantum: u64, quanta_per_second: f32) -> Self {
        Self {
            node: Some(node),
            detector: detector_at(quanta_per_second),
            touched: quantum,
        }
    }
}

/// **实时侧**每节点电平状态机（定长数组，零分配）。
///
/// 用法（每个量子）：
///
/// ```ignore
/// bank.begin_quantum();
/// for id in snapshot.tracks().keys() {
///     // 把该轨本量子的渲染结果写进 track_scratch, 然后:
///     scratch_meters[n] = bank.measure(*id, quantum, &track_scratch[..frames])?;
///     n += 1;
/// }
/// scratch_meters[n] = bank.measure_bus_stereo(snapshot.master(), quantum, block.left(), block.right());
/// ```
///
/// ## 节点对齐规则（状态在快照切换后仍然连续）
///
/// 槽位**绑定节点身份**（不是下标）：`measure` 先在定长数组里线性查找该节点的旧槽，
/// 找不到才用空槽。于是：
///
/// - 快照里轨道增删/重排（`BTreeMap` 迭代顺序变化）**不会**丢掉峰值保持与平滑 RMS；
/// - 不同节点的状态**永远不混用**（一个槽只服务一个 `EntityId`）；
/// - 每量子成本是 O(节点数 × 槽数) 次整数比较（典型 64 × 256），无分配、无锁、上界确定。
///
/// ## 容量
///
/// `N` 个槽全被占用时，新节点**淘汰最久未被计量的那个槽**（它的状态本轮本来就没用上），
/// 计数进 [`capacity_drops`](Self::capacity_drops) —— **绝不 panic、绝不扩容**。
/// 调用方（[`crate::rt`]）另外给母线保留一个发布槽位，因此正常路径不会触发淘汰。
#[derive(Clone, Debug)]
pub struct MeterBank<const N: usize> {
    slots: [MeterSlot; N],
    bus: LevelDetector,
    /// 本池**当前**的每秒量子数（弹道系数的唯一事实源）。
    ///
    /// 它必须住在池里而不是只存在于"最近一次刷新"的调用里：槽位是**惰性**建立的
    /// （[`MeterSlot::fresh`]，在音频线程的 [`Self::measure`] 里），而
    /// [`Self::set_quanta_per_second`] 只刷新**已经在册**的槽位 ⇒ 没有这个字段，
    /// "换率之后才出现的节点"就会拿到器件默认口径（375 量子/s = 48 kHz）。
    ///
    /// 初值 [`DEFAULT_QUANTA_PER_SECOND`]：池在收到任何快照之前的行为与旧实现
    /// **逐位相同**（48 kHz 下那正是 `sample_rate / 128`）。
    quanta_per_second: f32,
    measured: usize,
    capacity_drops: u64,
}

impl<const N: usize> Default for MeterBank<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> MeterBank<N> {
    /// 新建（全部槽未激活，总线检测器按默认口径）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: [MeterSlot::EMPTY; N],
            bus: LevelDetector::new(),
            quanta_per_second: DEFAULT_QUANTA_PER_SECOND,
            measured: 0,
            capacity_drops: 0,
        }
    }

    /// 开始一个量子：只重置本量子的计量计数，节点状态**跨量子保留**。
    pub fn begin_quantum(&mut self) {
        self.measured = 0;
    }

    /// 按 `sample_rate / block_frames` 重设全部节点与母线的弹道系数（保留电平状态）。
    ///
    /// 快照切换时调用一次。只做 `powf`/`exp` 与赋值：零分配、零锁。
    ///
    /// ⚠ 本方法只覆盖**已经在册**的槽位；**之后**才第一次被计量的节点由
    /// `MeterSlot::fresh` 按本方法刚刚记下的同一个数建检测器 ⇒
    /// "节点什么时候出现"不影响它的弹道。
    pub fn set_quanta_per_second(&mut self, quanta_per_second: f32) {
        let quanta_per_second = sanitise_quanta_per_second(quanta_per_second);
        self.quanta_per_second = quanta_per_second;
        self.bus.set_quanta_per_second(quanta_per_second);
        for slot in &mut self.slots {
            if slot.node.is_some() {
                slot.detector.set_quanta_per_second(quanta_per_second);
            }
        }
    }

    /// 全部清零（换流/关流时用）。
    ///
    /// **弹道系数不清**：它是配置（由 [`Self::set_quanta_per_second`] 武装），
    /// 与"电平状态"不是一回事 —— 与 `LevelDetector::reset` 的"系数保留"同口径。
    /// 母线的检测器因此也按**本池当前**的每秒量子数重建，而不是回到器件默认口径。
    pub fn reset(&mut self) {
        self.slots = [MeterSlot::EMPTY; N];
        self.bus = detector_at(self.quanta_per_second);
        self.measured = 0;
        self.capacity_drops = 0;
    }

    /// 本量子已计量的节点次数。
    #[must_use]
    pub const fn active_nodes(&self) -> usize {
        self.measured
    }

    /// 累计因容量耗尽而**淘汰**某个节点状态的次数。
    #[must_use]
    pub const fn capacity_drops(&self) -> u64 {
        self.capacity_drops
    }

    /// 测量一个节点的**单声道**量子电平。
    ///
    /// 每个量子对每个节点**恰好调用一次**（同一量子内重复调用会让弹道多走一步）。
    /// 只有 `N == 0` 时返回 `None`。
    pub fn measure(&mut self, node: EntityId, quantum: u64, samples: &[f32]) -> Option<MeterFrame> {
        if N == 0 {
            self.capacity_drops = self.capacity_drops.saturating_add(1);
            return None;
        }
        let index = match self.slots.iter().position(|slot| slot.node == Some(node)) {
            Some(found) => found,
            None => match self.slots.iter().position(|slot| slot.node.is_none()) {
                Some(free) => {
                    self.slots[free] = MeterSlot::fresh(node, quantum, self.quanta_per_second);
                    free
                }
                None => {
                    // 槽位全满: 淘汰最久未被计量的节点(它的状态本轮用不上)。
                    let mut victim = 0usize;
                    let mut oldest = u64::MAX;
                    for (candidate, slot) in self.slots.iter().enumerate() {
                        if slot.touched < oldest {
                            oldest = slot.touched;
                            victim = candidate;
                        }
                    }
                    self.capacity_drops = self.capacity_drops.saturating_add(1);
                    self.slots[victim] = MeterSlot::fresh(node, quantum, self.quanta_per_second);
                    victim
                }
            },
        };
        self.slots[index].touched = quantum;
        let reading = self.slots[index].detector.analyze(samples);
        self.measured = self.measured.saturating_add(1);
        Some(MeterFrame::from_reading(node, quantum, reading))
    }

    /// 测量**母线**（单声道视图）。
    pub fn measure_bus(&mut self, node: EntityId, quantum: u64, samples: &[f32]) -> MeterFrame {
        MeterFrame::from_reading(node, quantum, self.bus.analyze(samples))
    }

    /// 测量**母线**（立体声联动：峰值取两声道最大，均方按两声道平均）。
    pub fn measure_bus_stereo(
        &mut self,
        node: EntityId,
        quantum: u64,
        left: &[f32],
        right: &[f32],
    ) -> MeterFrame {
        MeterFrame::from_reading(node, quantum, self.bus.analyze_stereo(left, right))
    }
}

/// 建立电平队列：`Publisher` 给音频线程，`Collector` 给 UI 线程。
#[must_use]
pub fn meter_channel(capacity: usize) -> (MeterPublisher, MeterCollector) {
    let (producer, consumer) = RingBuffer::<MeterFrame>::new(capacity.max(1));
    (
        MeterPublisher {
            producer,
            bulk_push_calls: 0,
            events_pushed: 0,
            dropped: 0,
        },
        MeterCollector {
            consumer,
            bulk_pop_calls: 0,
            frames_drained: 0,
            frames_superseded: 0,
            ticks: 0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_computes_peak_and_rms_without_nan_on_empty_input() {
        let node = EntityId::new();
        let frame = MeterFrame::measure(node, 3, &[1.0, -1.0, 0.0, 0.0]);
        assert_eq!(frame.peak, 1.0);
        // sqrt((1+1+0+0)/4) = sqrt(0.5)
        assert!((frame.rms - 0.5f32.sqrt()).abs() < 1e-6);
        assert_eq!(frame.quantum, 3);
        assert_eq!(frame.node, node);
        assert!(frame.is_sane());

        let empty = MeterFrame::measure(node, 0, &[]);
        assert_eq!(empty.rms, 0.0);
        assert!(!empty.rms.is_nan(), "空块不得产生 NaN");
        assert_eq!(empty.peak_dbfs(), f32::NEG_INFINITY);
        assert_eq!(empty.peak_dbfs_clamped(level::SILENCE_FLOOR_DBFS), -120.0);

        let full = MeterFrame::measure(node, 0, &[1.0, 1.0]);
        assert!(full.peak_dbfs().abs() < 1e-6, "满量程 = 0 dBFS");
    }

    /// 判据 (d)（电平侧）：音频线程**每量子一次**批量写、UI **每 tick 一次**批量读。
    #[test]
    fn meter_bulk_contract_is_one_call_per_quantum_and_per_ui_tick() {
        let (mut publisher, mut collector) = meter_channel(DEFAULT_METER_CAPACITY);
        let nodes: Vec<EntityId> = (0..4).map(|_| EntityId::new()).collect();
        let mut frames: Vec<MeterFrame> = Vec::new();

        // 模拟 10 个量子 × 4 轨
        for quantum in 0..10u64 {
            frames.clear();
            for node in &nodes {
                frames.push(MeterFrame::new(*node, quantum, 0.5, 0.25));
            }
            assert_eq!(publisher.publish(&frames), 4);
        }
        assert_eq!(publisher.bulk_push_calls(), 10, "每个量子恰好一次批量写");
        assert_eq!(publisher.frames_pushed(), 40);
        assert_eq!(publisher.dropped(), 0);

        // UI 以 60Hz 抽干（这里两 tick 抽完）
        let mut scratch = [MeterFrame::default(); SCRATCH_METERS];
        let mut drained = 0;
        for _ in 0..2 {
            drained += collector.tick(&mut scratch);
        }
        assert_eq!(drained, 40);
        assert_eq!(collector.bulk_pop_calls(), 2, "每 tick 恰好一次批量读");
        assert_eq!(collector.ticks(), 2);
        assert_eq!(collector.pending(), 0);
    }

    #[test]
    fn meter_queue_is_lossy_but_never_blocks() {
        let (mut publisher, mut collector) = meter_channel(4);
        let node = EntityId::new();
        let frames: Vec<MeterFrame> = (0..10)
            .map(|quantum| MeterFrame::new(node, quantum, 0.1, 0.05))
            .collect();
        assert_eq!(publisher.publish(&frames), 4, "容量 4 只能收 4 条");
        assert_eq!(publisher.dropped(), 6, "丢弃必须被计数, 便于观测健康度");

        let mut scratch = [MeterFrame::default(); 8];
        assert_eq!(collector.tick(&mut scratch), 4);
        assert_eq!(collector.tick(&mut scratch), 0, "空了就返回 0, 不阻塞");
    }

    #[test]
    fn board_keeps_only_the_latest_frame_per_node_and_ignores_stale_updates() {
        let a = EntityId::new();
        let b = EntityId::new();
        let mut board = MeterBoard::new();
        assert!(board.is_empty());

        let batch = [
            MeterFrame::new(a, 10, 0.9, 0.5),
            MeterFrame::new(b, 10, 0.2, 0.1),
        ];
        assert_eq!(board.ingest(&batch), 2);
        assert_eq!(board.len(), 2);
        assert_eq!(board.latest_quantum(), Some(10));

        // 同一个节点的新帧覆盖旧帧
        let newer = [MeterFrame::new(a, 11, 0.1, 0.02)];
        assert_eq!(board.ingest(&newer), 1);
        assert_eq!(board.latest(&a).map(|f| f.quantum), Some(11));
        assert_eq!(board.latest(&a).map(|f| f.peak), Some(0.1));

        // 迟到的旧帧不得回退 UI（乱序/重放保护）
        let stale = [MeterFrame::new(a, 9, 1.0, 1.0)];
        assert_eq!(board.ingest(&stale), 0, "旧量子不得覆盖新值");
        assert_eq!(board.latest(&a).map(|f| f.quantum), Some(11));

        // 迭代顺序按 EntityId（确定性）
        let order: Vec<EntityId> = board.iter().map(|(node, _)| *node).collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(order, sorted);
    }

    #[test]
    fn zero_scratch_tick_does_not_consume_the_queue() {
        let (mut publisher, mut collector) = meter_channel(8);
        let node = EntityId::new();
        assert_eq!(publisher.publish(&[MeterFrame::new(node, 0, 1.0, 1.0)]), 1);
        let mut empty: [MeterFrame; 0] = [];
        assert_eq!(collector.tick(&mut empty), 0);
        assert_eq!(collector.bulk_pop_calls(), 0);
        assert_eq!(collector.pending(), 1, "零长 scratch 不该吞掉数据");
    }

    /// `drain_latest`：批量灌入 N 个量子后，一次抽取拿到的**每个节点**都是最后一帧。
    #[test]
    fn drain_latest_returns_the_newest_frame_per_node_and_empties_the_queue() {
        let (mut publisher, mut collector) = meter_channel(1024);
        let a = EntityId::new();
        let b = EntityId::new();
        let quanta = 50u64;
        for quantum in 0..quanta {
            let peak = (quantum as f32 + 1.0) / quanta as f32;
            assert_eq!(
                publisher.publish(&[
                    MeterFrame::new(a, quantum, peak, peak / 2.0),
                    MeterFrame::new(b, quantum, peak / 4.0, peak / 8.0)
                ]),
                2
            );
        }
        assert_eq!(collector.pending(), 100);

        let mut scratch = [MeterFrame::default(); 8];
        let unique = collector.drain_latest(&mut scratch);
        assert_eq!(unique, 2, "两个节点各留一帧");
        assert_eq!(collector.pending(), 0, "整批积压必须被抽干");
        assert_eq!(collector.frames_drained(), 100);
        assert_eq!(collector.frames_superseded(), 98, "98 条旧帧被覆盖");
        for frame in &scratch[..unique] {
            assert_eq!(frame.quantum, quanta - 1, "取到的必须是最后一个量子");
        }
        assert!((scratch[0].peak - 1.0).abs() < 1e-6);
    }

    /// 消费者落后时**不得把旧帧当新帧**：先灌入较新的帧，再灌入迟到的旧帧，
    /// 面板与 `drain_latest` 都必须保持新值。
    #[test]
    fn lagging_consumer_never_treats_a_stale_frame_as_fresh() {
        let (mut publisher, mut collector) = meter_channel(64);
        let node = EntityId::new();
        // 先进的新帧
        assert_eq!(
            publisher.publish(&[MeterFrame::new(node, 100, 0.25, 0.1)]),
            1
        );
        // 后到的旧帧（乱序/重放）
        assert_eq!(publisher.publish(&[MeterFrame::new(node, 7, 1.0, 1.0)]), 1);

        let mut scratch = [MeterFrame::default(); 4];
        let unique = collector.drain_latest(&mut scratch);
        assert_eq!(unique, 1);
        assert_eq!(scratch[0].quantum, 100, "保留的必须是量子更大的一帧");
        assert_eq!(scratch[0].peak, 0.25);

        let mut board = MeterBoard::new();
        board.ingest(&scratch[..unique]);
        // 再喂一次真正迟到的帧：不得回退
        assert_eq!(board.ingest(&[MeterFrame::new(node, 7, 1.0, 1.0)]), 0);
        assert_eq!(board.latest(&node).map(|f| f.quantum), Some(100));
        assert_eq!(board.latest_quantum(), Some(100));
    }

    #[test]
    fn drain_latest_respects_scratch_capacity_without_losing_the_rest() {
        let (mut publisher, mut collector) = meter_channel(64);
        let nodes: Vec<EntityId> = (0..10).map(|_| EntityId::new()).collect();
        let frames: Vec<MeterFrame> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| MeterFrame::new(*node, 0, index as f32 / 10.0, 0.0))
            .collect();
        assert_eq!(publisher.publish(&frames), 10);

        let mut small = [MeterFrame::default(); 4];
        // 只有 4 个槽: 仍然把队列抽干(不阻塞、不丢数据), 但只填 4 个节点
        let unique = collector.drain_latest(&mut small);
        assert_eq!(unique, 4);
        assert_eq!(collector.pending(), 0, "容量不足也必须抽干队列, 防止积压");
        assert_eq!(collector.frames_drained(), 10);

        let mut empty: [MeterFrame; 0] = [];
        assert_eq!(collector.drain_latest(&mut empty), 0);
    }

    /// `MeterBank`：每节点状态跨量子连续，且快照重排后不丢状态。
    #[test]
    fn meter_bank_keeps_per_node_state_across_reordering() {
        let a = EntityId::new();
        let b = EntityId::new();
        let mut bank = MeterBank::<4>::new();

        bank.begin_quantum();
        let first = bank.measure(a, 0, &[1.0f32; 8]).expect("容量足够");
        assert_eq!(bank.active_nodes(), 1);
        assert!((first.peak_hold - 1.0).abs() < 1e-6);

        // 下一量子: 顺序反过来(b 先, a 后) —— a 的保持值必须被继承而不是从 0 开始
        bank.begin_quantum();
        let b_frame = bank.measure(b, 1, &[0.5f32; 8]).expect("容量足够");
        let a_frame = bank.measure(a, 1, &[0.0f32; 8]).expect("容量足够");
        assert_eq!(bank.active_nodes(), 2);
        assert!(a_frame.peak_hold > 0.9, "a 的保持值必须跨量子/跨重排保留");
        assert!((b_frame.peak_hold - 0.5).abs() < 1e-6);

        // 第三个量子: 节点集合缩小到只剩 a
        bank.begin_quantum();
        let only_a = bank.measure(a, 2, &[0.0f32; 8]).expect("容量足够");
        assert!(only_a.peak_hold > 0.8);

        // 超容量: 淘汰最久未被计量的槽(仍然出帧), 并计数, 不 panic
        let mut tiny = MeterBank::<1>::new();
        tiny.begin_quantum();
        assert!(tiny.measure(a, 0, &[1.0f32; 4]).is_some());
        assert!(tiny.measure(b, 0, &[1.0f32; 4]).is_some());
        assert_eq!(tiny.capacity_drops(), 1, "第二个节点必须淘汰掉第一个的槽");
        // 被淘汰的节点下一量子重新建槽(状态从 0 起)，第三节点再淘汰
        tiny.begin_quantum();
        assert!(tiny.measure(a, 1, &[0.0f32; 4]).is_some());
        assert_eq!(tiny.capacity_drops(), 2);
    }

    /// `MeterBank`：母线立体声联动；单声道满幅即 0 dBFS。
    #[test]
    fn meter_bank_bus_is_stereo_linked_and_silence_stays_finite() {
        let master = EntityId::new();
        let mut bank = MeterBank::<4>::new();
        let loud = bank.measure_bus_stereo(master, 0, &[1.0f32; 16], &[0.0f32; 16]);
        assert!(loud.is_sane());
        assert!(loud.peak_dbfs().abs() < 1e-6, "单声道满幅 = 0 dBFS");

        let silence = bank.measure_bus_stereo(master, 1, &[0.0f32; 16], &[0.0f32; 16]);
        assert!(silence.is_sane());
        assert_eq!(silence.peak, 0.0);
        assert_eq!(silence.peak_dbfs(), f32::NEG_INFINITY);
        assert!(silence.peak_hold > 0.0, "保持值仍在衰减, 不会瞬间归零");

        let mono = bank.measure_bus(master, 2, &[0.25f32; 16]);
        assert!(mono.is_sane());
    }

    /// 判据：`NaN`/`Inf` 输入经 `MeterBank` 也不得产出 `NaN` 帧。
    #[test]
    fn meter_bank_sanitizes_hostile_input_to_finite_levels() {
        let node = EntityId::new();
        let mut bank = MeterBank::<2>::new();
        bank.begin_quantum();
        let frame = bank
            .measure(node, 0, &[f32::NAN, f32::INFINITY, -1.0e30, 0.5])
            .expect("容量足够");
        assert!(frame.is_sane(), "NaN/Inf 不得污染电平帧: {frame:?}");
        assert!(frame.peak <= level::MAX_LINEAR_MAGNITUDE);
        assert!(frame.peak_dbfs().is_finite());
    }

    /// `[ARCH-UI-002]` 弹道系数属于**池**，不属于"槽位建立的那一刻"。
    ///
    /// 量什么：同一份输入、同一个每秒量子数（750 = 96 kHz ÷ 128）下，两条臂的
    /// `peak_hold` / `rms_smoothed`（单位：线性幅度）。
    ///
    /// 两条臂的**唯一**差别是"槽位建立的时刻"与"换率时刻"的先后：
    /// 臂 A 的节点槽在换率**之前**建立（弹道由 `set_quanta_per_second` 转发），
    /// 臂 B 的节点槽在换率**之后**才第一次被计量（走 `MeterSlot::fresh`）。
    /// 旧实现里 `fresh` 用 `LevelDetector::new()`（硬编码 375 量子/s = 48 kHz）
    /// ⇒ 臂 B 拿到的是**旧采样率**的系数，而它每个量子仍被推进一次
    /// ⇒ 峰值保持按 **40 dB/s** 回落（而不是契约的 20 dB/s）、RMS 时间常数
    /// 减半（300 ms → 150 ms）。
    ///
    /// 判据两段：① 两条臂**逐位相等**（弹道与槽位寿命无关）；
    /// ② 形态 —— 750 个静音量子（= 1 秒）之后峰值保持恰好回落 20 dB（不是 40 dB）。
    fn ballistics_arms_at(quanta_per_second: f32) -> (MeterBank<2>, MeterBank<2>, EntityId) {
        let node = EntityId::new();
        // 臂 A：槽位在换率**之前**建立。
        let mut early = MeterBank::<2>::new();
        early.begin_quantum();
        assert!(early.measure(node, 0, &[]).is_some(), "容量足够");
        early.set_quanta_per_second(quanta_per_second);
        // 臂 B：槽位在换率**之后**才建立。
        let mut late = MeterBank::<2>::new();
        late.set_quanta_per_second(quanta_per_second);
        late.begin_quantum();
        assert!(late.measure(node, 0, &[]).is_some(), "容量足够");
        (early, late, node)
    }

    #[test]
    fn a_slot_activated_after_the_rate_change_uses_the_armed_ballistics() {
        const RATE_96K_QPS: f32 = 750.0;
        let loud = [1.0f32; 128];
        let silence: [f32; 0] = [];
        let (mut early, mut late, node) = ballistics_arms_at(RATE_96K_QPS);

        let mut last_early = 0.0f32;
        let mut last_late = 0.0f32;
        for quantum in 1..=751u64 {
            let samples: &[f32] = if quantum == 1 { &loud } else { &silence };
            early.begin_quantum();
            let a = early.measure(node, quantum, samples).expect("容量足够");
            late.begin_quantum();
            let b = late.measure(node, quantum, samples).expect("容量足够");
            assert_eq!(
                a.peak.to_bits(),
                b.peak.to_bits(),
                "量子 {quantum}: 本量子的块峰值不该受弹道影响"
            );
            assert_eq!(
                a.peak_hold.to_bits(),
                b.peak_hold.to_bits(),
                "量子 {quantum}: 峰值保持 {:.4}（{:.3} dB）vs {:.4}（{:.3} dB）\
                 —— 槽位建立的时刻不得改变弹道",
                a.peak_hold,
                a.peak_hold_dbfs(),
                b.peak_hold,
                b.peak_hold_dbfs()
            );
            assert_eq!(
                a.rms_smoothed.to_bits(),
                b.rms_smoothed.to_bits(),
                "量子 {quantum}: 平滑 RMS {:.6} vs {:.6} —— 同上",
                a.rms_smoothed,
                b.rms_smoothed
            );
            last_early = a.peak_hold;
            last_late = b.peak_hold;
        }

        // 形态判据：750 个静音量子（= 1 秒）之后必须回落 20 dB（旧实现：40 dB）。
        let decay_db = -20.0 * last_early.log10();
        assert!(
            (decay_db - 20.0).abs() < 0.05,
            "96 kHz（750 量子/s）下 1 秒的静音应让峰值保持回落 20 dB，实测 {decay_db:.3} dB \
             （臂 A={last_early}、臂 B={last_late}）"
        );
        assert!(
            (last_late - last_early).abs() < 1e-6,
            "臂 B（换率之后才建立的槽位）的峰值保持={last_late} 必须与臂 A 的 {last_early} 同口径"
        );
    }

    #[test]
    fn meter_bank_reset_keeps_the_armed_ballistics() {
        const RATE_96K_QPS: f32 = 750.0;
        let master = EntityId::new();
        let loud = [1.0f32; 128];
        let silence: [f32; 0] = [];
        let mut bank = MeterBank::<1>::new();
        bank.set_quanta_per_second(RATE_96K_QPS);
        let _ = bank.measure_bus(master, 0, &loud);
        bank.reset();

        // `reset` 清的是**电平状态**（峰值保持 / 均方），不是**配置**（弹道系数）
        // —— 与 `LevelDetector::reset` 的"系数保留"同口径。
        let mut last = 0.0f32;
        for quantum in 1..=751u64 {
            let samples: &[f32] = if quantum == 1 { &loud } else { &silence };
            last = bank.measure_bus(master, quantum, samples).peak_hold;
        }
        let decay_db = -20.0 * last.log10();
        assert!(
            (decay_db - 20.0).abs() < 0.05,
            "reset 之后母线仍须按已武装的 750 量子/s（20 dB/s）回落，实测 {decay_db:.3} dB \
             （peak_hold={last}）—— 回到器件默认口径会给出 40 dB"
        );
    }

    #[test]
    fn meter_bank_reset_clears_state_and_capacity_counters() {
        let node = EntityId::new();
        let mut bank = MeterBank::<1>::new();
        bank.begin_quantum();
        assert!(bank.measure(node, 0, &[1.0f32; 4]).is_some());
        assert!(bank.measure(EntityId::new(), 0, &[1.0f32; 4]).is_some());
        assert_eq!(bank.capacity_drops(), 1);
        bank.reset();
        assert_eq!(bank.active_nodes(), 0);
        assert_eq!(bank.capacity_drops(), 0);
        bank.begin_quantum();
        let frame = bank.measure(node, 1, &[0.0f32; 4]).expect("容量足够");
        assert_eq!(frame.peak_hold, 0.0, "reset 之后保持值必须从 0 开始");
    }

    /// 判据：槽位全满时淘汰的是**最久未被计量**的那个节点，不是最新那个。
    ///
    /// 量什么：`MeterBank::<3>` 里四个节点依次被计量时的 `capacity_drops`（次）与
    /// "最久未被计量的那个节点是否被淘汰"（峰值保持，线性幅度）。
    ///
    /// 为什么单独立一条：既有判据只用了 `MeterBank::<1>`（槽位唯一 ⇒ 淘汰谁都是它）
    /// ⇒"淘汰最久未被计量的"这条口径此前**没有任何判据**走过。本票注入实测：
    /// 把选取写成 `slot.touched > oldest`（配合初值 `u64::MAX` ⇒ 永远淘汰槽 0），
    /// 全量 24 个目标全绿 ⇒ 池满时"谁的尾巴被丢掉"取决于槽位下标而不是 LRU 契约。
    ///
    /// ⚠ 夹具必须让**槽 0 的 touched 最大**（否则"永远淘汰槽 0"与"淘汰最久未被计量的"
    /// 恰好同解，本判据就没有判别力）—— 下面先把 a 重新计量一次正是为此。
    #[test]
    fn a_full_bank_evicts_the_least_recently_measured_slot() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let d = EntityId::new();
        let mut bank = MeterBank::<3>::new();

        // 三个槽依次被 a / b / c 占满；再把 a 重新计量一次 ⇒
        // 槽 0 的 touched 最大（3），槽 1 的 b 是**最久未被计量**的那个（1）。
        bank.begin_quantum();
        assert!(bank.measure(a, 0, &[1.0f32; 32]).is_some());
        bank.begin_quantum();
        assert!(bank.measure(b, 1, &[1.0f32; 32]).is_some());
        bank.begin_quantum();
        assert!(bank.measure(c, 2, &[1.0f32; 32]).is_some());
        bank.begin_quantum();
        assert!(bank.measure(a, 3, &[1.0f32; 32]).is_some());
        assert_eq!(
            bank.active_nodes(),
            1,
            "覆盖度：本量子只有 a 被计量（其他两个槽的状态跨量子保留、但本量子不计）"
        );

        // 第四个节点 ⇒ 槽位全满，必须淘汰 **b**（touched = 1，最久未被计量），
        // 而不是 touched 最大的 a（槽 0）。
        bank.begin_quantum();
        assert!(bank.measure(d, 4, &[1.0f32; 32]).is_some());
        assert_eq!(bank.capacity_drops(), 1, "第四个节点必须淘汰一个槽");

        // 判别点：b 若**被淘汰**，下一量子重新建槽 ⇒ 峰值保持从 0 起；
        // 若淘汰的是 a（touched 最大），b 仍在册 ⇒ 它带着 ≈ 1.0 的历史。
        bank.begin_quantum();
        let b_after = bank.measure(b, 5, &[]).expect("容量足够");
        assert_eq!(
            b_after.peak_hold, 0.0,
            "最久未被计量的节点必须被淘汰 ⇒ 它重新出现时峰值保持从 0 起；\
             实测 {}（淘汰'最新'会让它保住 ≈ 1.0 的历史）",
            b_after.peak_hold
        );
    }

    /// 判据：`reset` 必须把**槽位状态**清掉，不是只把计数器归零。
    ///
    /// 量什么：`MeterBank::<2>` 计量一个节点之后 `reset`，再计量**同一个节点**
    /// 一个静音量子 ⇒ 峰值保持必须是 `0.0`（单位：线性幅度）。
    ///
    /// 为什么单独立一条：既有的 `meter_bank_reset_clears_state_and_capacity_counters`
    /// 用的是 `MeterBank::<1>`，且它在 reset 前把槽位**填满**（第二个节点淘汰了第一个）
    /// ⇒ reset 之后重新计量第一个节点会走"淘汰"路径建新槽，状态**恰好**从 0 起
    /// —— "没清槽位"因此看不出来。本票注入实测：删掉
    /// `self.slots = [MeterSlot::EMPTY; N];`，全量 24 个目标全绿
    /// ⇒ 换流/关流之后旧节点的峰值保持会跨过一次 `reset` 活下来。
    #[test]
    fn reset_clears_the_slot_state_even_with_a_free_slot() {
        let node = EntityId::new();
        let mut bank = MeterBank::<2>::new();
        bank.begin_quantum();
        let loud = bank.measure(node, 0, &[1.0f32; 32]).expect("容量足够");
        assert!((loud.peak_hold - 1.0).abs() < 1e-6);

        bank.reset();
        assert_eq!(bank.active_nodes(), 0);
        // 另一个槽是**空**的 ⇒ 重新计量同一个节点走的是"复用旧槽"路径（不是淘汰路径）。
        bank.begin_quantum();
        let after = bank.measure(node, 1, &[0.0f32; 32]).expect("容量足够");
        assert_eq!(
            after.peak_hold, 0.0,
            "reset 之后同一个节点的峰值保持必须从 0 起（槽位必须被清掉，而不是只清计数器）"
        );
    }

    /// 判据：零容量池（`MeterBank::<0>`）拒绝计量时**必须计数**，不得静默。
    ///
    /// 量什么：`MeterBank::<0>::measure` 的返回值与 `capacity_drops`（次）。
    ///
    /// 为什么单独立一条：全仓没有 `MeterBank::<0>` 的判据
    /// （量法：`grep -rn 'MeterBank::<0>' crates/yeban-engine` 命中 0 行）
    /// ⇒ 删掉那一行计数不会有任何判据变红（本票注入实测：全量 24 个目标全绿）。
    /// 它与 `MeterBank::<N>` 的淘汰计数是同一条"装不下就报数"的口径。
    #[test]
    fn a_zero_capacity_bank_counts_every_rejected_measurement() {
        let node = EntityId::new();
        let mut bank = MeterBank::<0>::new();
        bank.begin_quantum();
        assert!(bank.measure(node, 0, &[1.0f32; 8]).is_none());
        bank.begin_quantum();
        assert!(bank.measure(node, 1, &[1.0f32; 8]).is_none());
        assert_eq!(
            bank.capacity_drops(),
            2,
            "零容量池每一次被拒的计量都必须计入 capacity_drops（不静默）"
        );
        assert_eq!(bank.active_nodes(), 0, "零容量池不得计量任何节点");
    }

    /// 判据：槽位全满时**只**淘汰最久未被计量的**那一个**槽 —— 被淘汰者的状态清零，
    /// **其余在册槽的状态原样保留**（不是整池重建，也不是淘汰多个）。
    ///
    /// 量什么：`MeterBank::<3>` 里第四个节点触发淘汰之后，两个**幸存**节点的
    /// `peak_hold`（线性幅度）与被淘汰节点重新出现时的 `peak_hold`。
    ///
    /// 为什么单独立一条：既有的 `a_full_bank_evicts_the_least_recently_measured_slot`
    /// 只断言**被淘汰者**从 0 重建。一个"溢出时把整个 `slots` 数组清空、再放进新节点"
    /// 的实现同样能让那条判据全绿（被淘汰者当然从 0 起），而它丢掉了**所有**幸存节点的
    /// 峰值保持与平滑 RMS。本判据补上"幸存者必须还在"这一半。
    ///
    /// 夹具前提：槽 0 的 `touched` 必须是**最大**的那个（下面在 q3 重测 `a` 正是为此），
    /// 否则"永远淘汰槽 0"与"淘汰最久未被计量的"恰好同解、判据没有判别力。
    #[test]
    fn a_full_bank_replaces_exactly_one_slot_and_keeps_every_survivor_state() {
        let a = EntityId::new();
        let b = EntityId::new();
        let c = EntityId::new();
        let d = EntityId::new();
        let mut bank = MeterBank::<3>::new();

        // q0..q2：三个槽依次被 a / b / c 占满，三者都满幅（peak_hold = 1.0）。
        bank.begin_quantum();
        let a0 = bank.measure(a, 0, &[1.0f32; 32]).expect("容量足够");
        assert!((a0.peak_hold - 1.0).abs() < 1e-6, "夹具前提：a 满幅");
        bank.begin_quantum();
        assert!(bank.measure(b, 1, &[1.0f32; 32]).is_some());
        bank.begin_quantum();
        assert!(bank.measure(c, 2, &[1.0f32; 32]).is_some());
        // q3：重测 a ⇒ touched 为 a = 3、b = 1、c = 2 ⇒ b 是最久未被计量的那个。
        bank.begin_quantum();
        assert!(bank.measure(a, 3, &[]).is_some());

        // q4：d 进不来 ⇒ 恰好淘汰 b 一个槽。
        bank.begin_quantum();
        assert!(bank.measure(d, 4, &[1.0f32; 32]).is_some());
        assert_eq!(bank.capacity_drops(), 1, "第四个节点只许淘汰一个槽");

        // 幸存者 a 与 c 必须带着历史继续（不是被一起清掉）。
        bank.begin_quantum();
        let a_after = bank.measure(a, 5, &[]).expect("容量足够");
        let c_after = bank.measure(c, 6, &[]).expect("容量足够");
        assert!(
            a_after.peak_hold > 0.9,
            "a（touched = 3）是幸存者 ⇒ 峰值保持必须还在回落中，实测 {}",
            a_after.peak_hold
        );
        assert!(
            c_after.peak_hold > 0.9,
            "c（touched = 2）同样是幸存者 ⇒ 峰值保持必须还在回落中，实测 {}",
            c_after.peak_hold
        );

        // 被淘汰的 b 重新出现 ⇒ 从 0 起（它在 q4 已经出池）。
        bank.begin_quantum();
        let b_after = bank.measure(b, 7, &[]).expect("容量足够");
        assert_eq!(
            b_after.peak_hold, 0.0,
            "b（touched = 1，最久未被计量）必须已被淘汰 ⇒ 重新出现时从 0 起；实测 {}",
            b_after.peak_hold
        );
    }

    /// 判据：零容量池（`MeterBank::<0>`）**逐次**拒绝计量并计数，节点槽位的零容量
    /// 不得让母线路径一起失效。
    ///
    /// 量什么：`MeterBank::<0>` 对**两个不同节点**的 `measure` 返回值与
    /// `capacity_drops`（次），加上 `measure_bus_stereo` 的 `peak`（线性幅度）。
    ///
    /// 为什么单独立一条：既有的 `a_zero_capacity_bank_counts_every_rejected_measurement`
    /// 只重复测**同一个节点**、且不跨 `begin_quantum` ⇒ 一个"同一节点第二次被拒就不再
    /// 计数"或"`begin_quantum` 顺手把累计拒绝数清零"的实现同样全绿；那条判据也完全没碰
    /// 母线 —— 母线不走槽位池（`measure_bus*` 的签名不返回 `Option`）⇒ 零节点容量下
    /// 它**必须**照常出帧，而不是被一起关掉。
    #[test]
    fn a_zero_capacity_bank_counts_every_rejected_node_and_keeps_the_bus_alive() {
        let a = EntityId::new();
        let b = EntityId::new();
        let master = EntityId::new();
        let mut bank = MeterBank::<0>::new();

        bank.begin_quantum();
        assert!(bank.measure(a, 0, &[1.0f32; 8]).is_none());
        assert!(bank.measure(b, 0, &[1.0f32; 8]).is_none());
        assert_eq!(
            bank.capacity_drops(),
            2,
            "两个**不同**节点被拒的计量必须各计一次（不静默）"
        );
        assert_eq!(bank.active_nodes(), 0, "零容量池不得计量任何节点");

        // 跨量子累计：`begin_quantum` 只重置本量子的计量计数，不得吞掉拒绝数。
        bank.begin_quantum();
        assert!(bank.measure(a, 1, &[1.0f32; 8]).is_none());
        assert_eq!(bank.capacity_drops(), 3, "拒绝数必须跨量子累计");

        // 母线不走槽位池 ⇒ 零节点容量下仍必须真的出帧。
        let bus = bank.measure_bus_stereo(master, 1, &[1.0f32; 8], &[1.0f32; 8]);
        assert!(bus.is_sane(), "母线帧必须有限: {bus:?}");
        assert!(
            (bus.peak - 1.0).abs() < 1e-6,
            "零节点容量不得让母线失效（实测 peak = {}）",
            bus.peak
        );
    }
}
