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
//! - `NaN`/`Inf` 输入不得让任何一帧变成 `NaN`（[`MeterFrame::is_sane`]）。

use rtrb::{Consumer, Producer, RingBuffer};
use yeban_model::EntityId;

use crate::level::{self, LevelDetector, LevelReading, dbfs, dbfs_clamped};

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

    /// 为一个节点新建激活槽（默认弹道）。
    fn fresh(node: EntityId, quantum: u64) -> Self {
        Self {
            node: Some(node),
            detector: LevelDetector::new(),
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
    pub fn set_quanta_per_second(&mut self, quanta_per_second: f32) {
        self.bus.set_quanta_per_second(quanta_per_second);
        for slot in &mut self.slots {
            if slot.node.is_some() {
                slot.detector.set_quanta_per_second(quanta_per_second);
            }
        }
    }

    /// 全部清零（换流/关流时用）。
    pub fn reset(&mut self) {
        self.slots = [MeterSlot::EMPTY; N];
        self.bus = LevelDetector::new();
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
                    self.slots[free] = MeterSlot::fresh(node, quantum);
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
                    self.slots[victim] = MeterSlot::fresh(node, quantum);
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
}
