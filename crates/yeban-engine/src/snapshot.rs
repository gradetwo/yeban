//! 不可变引擎快照、原子交换槽与退役回收队列。
//! [ARCH-RT-002, ROAD-M2-002, ARCH-TOP-002]
//!
//! ## 规范要求
//!
//! [ARCH-RT-002] 原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2）：
//!
//! > 音频线程内使用原子指针读取最新的不可变 `Arc<EngineSnapshot>`；若检测到拓扑版本更新，
//! > 音频线程将持有的旧快照 **move** 进预先分配的无锁队列
//! > `rtrb::Producer<Arc<EngineSnapshot>>`；**主线程以 60Hz 轮询从回收队列出队并负责旧快照
//! > 的 Drop 析构**，彻底杜绝音频线程内发生任何内存释放（Zero Free）。
//!
//! 因此本模块把职责切成三块：
//!
//! | 角色 | 线程 | 做的事 |
//! | :--- | :--- | :--- |
//! | [`SnapshotSlot::publish`] | Model 线程（非实时） | 构造新快照、原子换指针、把旧 `Arc` 移入待回收清单 |
//! | [`SnapshotReader`] | cpal 回调线程 | 无锁读指针、`Arc` 克隆、把旧 `Arc` push 进退役队列（[`RetireProducer`]） |
//! | [`RetireQueue::drain`] + [`SnapshotSlot::prune`] | 主线程 60Hz | 出队并 **Drop**；释放写者侧待回收清单 |
//!
//! ## 控制面看得到的退役健康读数
//!
//! [`RetireQueue`] 的 `pending` / `drained` / `foreign_drains` / 释放线程归属现在记在一个
//! 与 [`RetireProducer`]（音频线程侧）**共享的** [`RetireAccounting`] 上（全是原子量，
//! 无锁、零分配）⇒ 渲染驱动可以把它们直接读进 [`crate::rt::EngineStats`]，
//! 控制面因此能看见"退役队列积压了多少 / 释放是不是集中在同一个线程"。
//! 语义、单调性与重置语义见 `docs/ledger/engine-stats-notes.md`。
//!
//! ## 无锁指针交换的**内存回收安全证明**（本模块唯一的 `unsafe` 依据）
//!
//! 裸指针方案的唯一危险是"读者 load 到指针、还没来得及把引用计数 +1，写者就把对象释放了"。
//! 本模块用一个**纪元握手**（epoch handshake）关掉这个窗口，只依赖 Acquire/Release：
//!
//! ```text
//! 写者 publish():                        读者 begin_block():
//!   anchor = new                            epoch = slot.epoch.load(Acquire)   (1)
//!   ptr.store(new, Release)                 ptr   = slot.ptr.load(Acquire)     (2)
//!   e = epoch.fetch_add(1, AcqRel) + 1      if ptr != held { increment_strong_count; from_raw }
//!   pending.push((e, old))                  读者 end_block():
//!                                             slot.reader_done.store(epoch, Release)  (3)
//! 写者侧 prune():
//!   done = reader_done.load(Acquire)
//!   drop 掉所有 e <= done 的待回收项
//! ```
//!
//! **论证**：
//!
//! 1. 写者先 `ptr.store` 再 `epoch.fetch_add`（程序序）。读者若在 (1) 读到纪元 `e`，
//!    则 `ptr.store(Release)` **happens-before** 这次 `fetch_add`（写者程序序），
//!    而读者以 `Acquire` 读到了 `fetch_add` 的结果，因此 `ptr.store` 也 happens-before
//!    读者的 (1)。由写读一致性，读者其后的 (2) 不可能读到比 `ptr.store` 更早的值：
//!    **读者在纪元 `e` 的块里绝不会解引用被 `e` 标记淘汰的那个快照**。
//! 2. `reader_done` 只写一次"我完成了纪元 `e` 的块"，且读者是**单线程顺序**
//!    执行块：一旦 `done >= e`，所有更早的块都已完整结束，其中对旧指针的
//!    `increment_strong_count` 也已完成。
//! 3. 读者以 `Release` 写 (3)、写者以 `Acquire` 读 `done` ⇒ 读者在 (3) 之前做的一切
//!    （包括引用计数 +1）happens-before 写者 `prune` 里的 `drop`。因此读者**自己的**
//!    `Arc` 与写者 anchor 的引用计数都已被抬升，`drop` 不会打断任何在途读者。
//! 4. 锚（`anchor`）始终持有当前快照的一个强引用 ⇒ `slot.ptr` 永不悬垂，
//!    任何时刻新建的读者都能安全地克隆它。
//!
//! 只要读者**每个块**都调用 [`SnapshotReader::end_block`]，`prune` 就总能在两个块周期内
//! 释放写者侧清单。读者线程若停止（关流），[`Drop`] 会把 `reader_done` 置为 `u64::MAX`
//! 并解除 attached 标记，`prune` 随即可以清空全部待回收项。
//!
//! ## 边界（本切片没有证明的）
//!
//! - **`unsafe` 未做形式化验证**：上面的论证是纸面推导 + 单线程顺序假设，
//!   `Miri`/`loom` 的机械化验证列入 notes 的 needs 清单。当前不引入 `loom`（新依赖）
//!   是为了不扩大依赖图。
//! - **只支持单个读者**：`SnapshotReader` 的"单线程顺序块"前提是安全证明的一部分。
//!   多读者需要换成真正的 hazard pointer 数组。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::ThreadId;

use thiserror::Error;
use yeban_dsp::math::note_to_hz;
use yeban_model::project::DEFAULT_BPM;
use yeban_model::{
    BlockSize, ClipContent, EntityId, ModelError, RoutingGraph, TrackKind, TrackV3, YebanProjectV1,
};

use crate::graph::{LatencyTable, PdcError, PdcPlan};
use crate::mixer::{PanLaw, pan_gains};
use crate::synth::{
    MAX_NOTES_PER_TRACK, NoteSchedule, ScheduledNote, ToneParams, tick_to_sample, velocity_gain,
};

/// 退役队列默认容量（条）。一帧 60Hz 内被替换的快照远不会超过这个数。
pub const DEFAULT_RETIRE_CAPACITY: usize = 32;

/// `EngineSnapshot` **释放事件**的可观测探针（[MUST-GATE-012] 的运行期判据）。
///
/// 规范要求（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` 的 `[MUST-GATE-012]`）：
/// 「高频交换压测下，音频线程**无任何堆释放**，所有旧快照均在**主线程 60Hz 循环**中安全释放」。
///
/// 这条门禁的核心是**释放发生在哪个线程**。而 [`RetireQueue::drained`] /
/// [`SnapshotSlot::prune`] 只能证明"谁**调用**了 drain/prune"，证不到"`Drop` 真的跑在哪个线程"
/// —— 一旦有人把旧 `Arc` 就地 `drop`（而不是推进队列），那两个计数都**不会**变红。
///
/// 因此引擎在 [`EngineSnapshot`] 的 `Drop` 里记两个数：
///
/// - [`total`](release_probe::total())：进程内**所有**快照析构的累计数（跨线程原子量）。
///   它让"零泄漏"成为一条**等式**（创建数 = 释放数 + 存活数），而不是"没崩就算过"；
/// - [`released_by_current_thread`](release_probe::released_by_current_thread())：
///   **本线程**跑过多少次快照析构（线程局部 `Cell<u64>`，无锁、零分配、无 TLS 析构）。
///   判据在实时线程窗口内打开 [`watch_current_thread`](release_probe::watch_current_thread())，
///   于是"音频线程释放了 0 个快照"是**直接测量**，而不是从代码形状推断出来的。
///
/// # 成本（为什么可以留在生产代码里）
///
/// 一次 [`Cell::get`] + 一次 `Relaxed` 原子加，只在**快照析构**时发生 ——
/// 而快照只在**拓扑变更**（用户改路由/参数）时才被淘汰，不是每个渲染量子。
/// 实时热路径（`process_quantum`）不受影响。
pub mod release_probe {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    thread_local! {
        /// 本线程是否在观测窗口内（`const` 初始化 ⇒ 无惰性分配、无析构器）。
        static WATCHING: Cell<bool> = const { Cell::new(false) };
        /// 本线程在观测窗口内跑过的快照析构次数。
        static RELEASED_HERE: Cell<u64> = const { Cell::new(0) };
    }

    /// 进程内累计的快照析构次数（所有线程）。
    static RELEASED_TOTAL: AtomicU64 = AtomicU64::new(0);

    /// 每次 `EngineSnapshot::drop` 调用一次（由 `impl Drop` 唯一调用点保证）。
    pub(super) fn note_release() {
        RELEASED_TOTAL.fetch_add(1, Ordering::Relaxed);
        // `try_with`（而不是 `with`）：线程 teardown 期间线程局部量可能不可用，
        // 而此时**不能**从析构里 panic 出去。观测不到就按"没在观测"处理。
        let _ = WATCHING.try_with(|watching| {
            if watching.get() {
                let _ = RELEASED_HERE.try_with(|count| count.set(count.get().saturating_add(1)));
            }
        });
    }

    /// 开始观测**本线程**上的快照析构（窗口之外调用）。
    ///
    /// 观测是**线程局部**的：多个线程可以各自开自己的窗口而不会互相污染计数。
    pub fn watch_current_thread() {
        let _ = WATCHING.try_with(|watching| watching.set(true));
    }

    /// 结束观测**本线程**上的快照析构（计数保留，供
    /// [`released_by_current_thread`](Self::released_by_current_thread()) 读回）。
    pub fn unwatch_current_thread() {
        let _ = WATCHING.try_with(|watching| watching.set(false));
    }

    /// 本线程在最近一次观测窗口里跑过的快照析构次数。
    ///
    /// 语义是"本线程释放了几个**快照分配**"（每个 `Arc<EngineSnapshot>` 的最后一个强引用
    /// 被丢掉时恰好记一次），与"`drain` 返回了几条"不同 —— 后者只是从队列里**取走**。
    #[must_use]
    pub fn released_by_current_thread() -> u64 {
        WATCHING
            .try_with(|_| RELEASED_HERE.try_with(Cell::get).unwrap_or(0))
            .unwrap_or(0)
    }

    /// 清空本线程的观测计数（**不**改变 `WATCHING` 开关）。
    pub fn reset_current_thread() {
        let _ = RELEASED_HERE.try_with(|count| count.set(0));
    }

    /// 进程内累计的快照析构次数（跨线程；用**差值**做单窗口/单轮对账）。
    #[must_use]
    pub fn total() -> u64 {
        RELEASED_TOTAL.load(Ordering::Relaxed)
    }
}

/// 快照投影错误。
#[derive(Debug, Error, PartialEq)]
pub enum SnapshotError {
    /// PDC 拓扑计算失败（成环 / master 不在图里 / 边端点缺失）。
    #[error(transparent)]
    Pdc(#[from] PdcError),

    /// 模型层校验失败。
    #[error(transparent)]
    Model(#[from] ModelError),

    /// 工程没有可用的主总线节点 —— 无法确定 PDC 的关键路径终点。
    ///
    /// 触发条件：`YebanProjectV1::master_bus_track_id` 是空 ULID，
    /// 或者它不在 `routing_graph.nodes` 里。
    #[error("engine snapshot: project has no usable master bus node")]
    NoMasterBus,
}

/// 音频线程需要的单轨参数（`TrackV3` 的**只读投影**）。
///
/// 故意不搬运 `name`/`color`/`clips` 等界面或编辑期数据：音频线程只需混音参数，
/// 少复制一个 `String` 就少一处 `Drop` 出现在快照里 [ARCH-RT-001]。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackParams {
    kind: TrackKind,
    volume_db: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    solo_safe: bool,
    device_count: u32,
    latency_samples: u32,
}

impl TrackParams {
    /// 从模型层音轨投影（**只读**，不改动模型）。
    ///
    /// `latency_samples` 由调用方提供：模型层的
    /// `DeviceDefinition::latency_samples` **已经存在**（且是**必需字段** [ADR-0001 D43]），
    /// [`crate::graph::LatencyTable`] 按设备链汇总它并把结果作为这里的入参传入
    /// ⇒ 本函数自身不需要（也不应该）再认识设备链，延迟的**唯一事实源**留在
    /// [`crate::graph`]。
    #[must_use]
    pub fn from_track(track: &TrackV3, latency_samples: u32) -> Self {
        Self {
            kind: track.kind,
            volume_db: track.volume_db,
            pan: track.pan,
            mute: track.mute,
            solo: track.solo,
            solo_safe: track.solo_safe,
            device_count: u32::try_from(track.devices.len()).unwrap_or(u32::MAX),
            latency_samples,
        }
    }

    /// 音轨类型。
    #[must_use]
    pub const fn kind(&self) -> TrackKind {
        self.kind
    }

    /// 音量 (dB)。
    #[must_use]
    pub const fn volume_db(&self) -> f32 {
        self.volume_db
    }

    /// 声相 -1.0..=1.0。
    #[must_use]
    pub const fn pan(&self) -> f32 {
        self.pan
    }

    /// 静音。
    #[must_use]
    pub const fn is_muted(&self) -> bool {
        self.mute
    }

    /// 独奏。
    #[must_use]
    pub const fn is_soloed(&self) -> bool {
        self.solo
    }

    /// 独奏安全。
    #[must_use]
    pub const fn is_solo_safe(&self) -> bool {
        self.solo_safe
    }

    /// 设备链长度。
    #[must_use]
    pub const fn device_count(&self) -> u32 {
        self.device_count
    }

    /// 本轨自身处理延迟（采样点）。
    #[must_use]
    pub const fn latency_samples(&self) -> u32 {
        self.latency_samples
    }

    /// 本轨的声相增益 `(左, 右)`：由 `pan` 与**声相定律**在**构造期**算出。
    ///
    /// 为什么在这里算而不是在实时侧算：`cos`/`sin` 属 [ADR-0001 D32] 的
    /// **超越函数类**（4096 ulp 预算），实时侧只允许 IEEE 精确类乘法。
    /// 实时侧因此只读这两个已算好的 `f32`（见 [`crate::mixer::pan_gains`]）。
    ///
    /// 口径（默认律 `ConstantPowerMinus3dB`）：`cos θ` / `sin θ`、`θ = (pan+1)·π/4`；
    /// 居中 ⇒ `(√2/2, √2/2)`（每声道 −3.01 dB）。详见 [`crate::mixer`] 模块文档 §1。
    #[must_use]
    pub fn pan_gains(&self, law: PanLaw) -> (f32, f32) {
        pan_gains(self.pan, law)
    }
}

/// 不可变引擎快照：音频线程在每个渲染量子边界读取的唯一权威运行态
/// [ARCH-RT-002, ROAD-M2-002]。
///
/// - **全部字段私有 + 只有 getter** ⇒ 构造后无法修改，可以安全地 `Arc` 共享给音频线程
///   而不需要任何锁；
/// - 由模型层数据**投影**而来（[`EngineSnapshot::from_project`]），不持有 `&mut` 模型引用；
/// - `Send + Sync`（字段都是 `Send + Sync`），因此 `Arc<EngineSnapshot>` 可以跨线程传递。
#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    revision: u64,
    sample_rate: u32,
    /// 工程速度（BPM）——`YebanProjectV1::bpm` 的投影。
    ///
    /// 走带每帧的 tick 增量是 `bpm × 16 / sample_rate`（[`crate::transport`]），
    /// 而 `sample_rate` 也在这里 —— 两个量必须来自**同一份**快照，否则"换快照时
    /// 按新速度推进"会出现半更新（用了新的采样率配旧的速度）。
    /// 速度是**模型字段**（不是运行态），所以放进不可变快照不违反 [MODEL-ISO-001]
    /// （那条禁的是播放头位置）。
    bpm: f64,
    block_frames: usize,
    channels: u16,
    master: EntityId,
    tracks: BTreeMap<EntityId, TrackParams>,
    /// 每轨的音符调度表（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    ///
    /// 实时侧只读它、不构造它：tick → 样本的换算、`probability` 触发判定、
    /// 力度/音量增益全部在**控制线程**算完（见 [`crate::synth`] 模块文档 §1）。
    schedules: BTreeMap<EntityId, NoteSchedule>,
    /// 每轨的**音色参数**（四极低通的三个旋钮）[ARCH-DSP-001, ROAD-M2-006]。
    ///
    /// ⚠ **引擎侧临时形状**：`yeban-model` 还没有"乐器参数 → 音频线程"的投影，
    /// 因此这里由 [`ToneParams::from_devices`] 从 `TrackV3.devices` 的
    /// `InternalInstrument` 设备的 `params` 里抽取。
    /// 它**不是**模型的第二份定义，等模型线补齐后应整体删除
    /// （见 `docs/ledger/engine-mix-notes.md` 的 needs 与 [`crate::synth::ToneParams`]）。
    tones: BTreeMap<EntityId, ToneParams>,
    /// 声相衰减律（`audio_config.pan_law` 的投影）[MODEL-AST-002]。
    ///
    /// 模型层已有这个枚举，但**快照此前没有投影它**（`line/engine-sound` 的 needs N4）。
    /// 投影进来之后，"声相定律影响输出"才是端到端可判据的；曲线本身仍在构造期
    /// 展开成两个 `f32`（[`TrackParams::pan_gains`]），实时侧只做乘法。
    pan_law: PanLaw,
    /// 当前快照里已调度的音符总条数（诊断/判据用）。
    scheduled_notes: usize,
    /// 因 [`MAX_NOTES_PER_TRACK`] 容量上限而被丢弃的音符条数（构造期计数）。
    note_schedule_drops: u64,
    pdc: PdcPlan,
}

impl EngineSnapshot {
    /// 从工程投影出一个快照 [ROAD-M2-002]。
    ///
    /// `revision` 是模型层的提交版本号（单调递增），用于让 UI/日志判断"音频线程是否
    /// 已经追上"。
    ///
    /// 节点延迟**自动**从模型读取（[`LatencyTable::from_project`]，即
    /// `DeviceDefinition::latency_samples` 之和）[ARCH-PDC-001]。
    /// 需要注入测量值/构造合成场景时用 [`from_project_with_latencies`](Self::from_project_with_latencies)。
    ///
    /// # Errors
    ///
    /// - [`SnapshotError::NoMasterBus`]：工程没有可用的主总线节点；
    /// - [`SnapshotError::Pdc`]：路由图成环 / 边端点缺失；
    /// - [`SnapshotError::Model`]：`RoutingGraph::validate()` 失败。
    pub fn from_project(project: &YebanProjectV1, revision: u64) -> Result<Self, SnapshotError> {
        let latencies = LatencyTable::from_project(project);
        Self::from_project_with_latencies(project, revision, &latencies)
    }

    /// 同 [`from_project`](Self::from_project)，但延迟表由调用方显式提供。
    ///
    /// 用途：离线对账时注入实测延迟、测试时构造"只有一条支路有延迟"的合成场景。
    /// 生产路径应当用 [`from_project`](Self::from_project)，避免出现第二个延迟事实源。
    ///
    /// # Errors
    ///
    /// 同 [`from_project`](Self::from_project)。
    pub fn from_project_with_latencies(
        project: &YebanProjectV1,
        revision: u64,
        latencies: &LatencyTable,
    ) -> Result<Self, SnapshotError> {
        let master = project.master_bus_track_id;
        if master.is_nil() || !project.routing_graph.nodes.contains(&master) {
            return Err(SnapshotError::NoMasterBus);
        }
        project.routing_graph.validate()?;

        let mut tracks: BTreeMap<EntityId, TrackParams> = BTreeMap::new();
        let mut tones: BTreeMap<EntityId, ToneParams> = BTreeMap::new();
        for (id, track) in &project.tracks {
            tracks.insert(*id, TrackParams::from_track(track, latencies.get(id)));
            tones.insert(*id, ToneParams::from_devices(&track.devices));
        }
        let block = project.audio_config.block_size.frames() as usize;
        let (schedules, dropped) = project_schedules(project);
        Self::from_parts(
            revision,
            project.audio_config.sample_rate.hz(),
            block,
            2,
            master,
            tracks,
            &project.routing_graph,
            latencies,
        )
        .map(|snapshot| {
            snapshot
                .with_schedules(schedules, dropped)
                .with_tones(tones)
                .with_pan_law(PanLaw::from_model(project.audio_config.pan_law))
                .with_bpm(project.bpm)
        })
    }

    /// 低层构造：显式给出全部字段（离线渲染器与测试用）。
    ///
    /// **不含**音符调度表（构造出的是静音快照）；需要发声时用
    /// [`with_schedules`](Self::with_schedules) 附上，或直接用
    /// [`from_project`](Self::from_project)（它会把工程的 MIDI 摆放投影成调度表）。
    ///
    /// # Errors
    ///
    /// 同 [`from_project`](Self::from_project)（不含 `NoMasterBus` 的 nil 判断，
    /// 但仍要求 `master` 在 `routing_graph.nodes` 里）。
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        revision: u64,
        sample_rate: u32,
        block_frames: usize,
        channels: u16,
        master: EntityId,
        tracks: BTreeMap<EntityId, TrackParams>,
        routing: &RoutingGraph,
        latencies: &LatencyTable,
    ) -> Result<Self, SnapshotError> {
        let pdc = PdcPlan::compute(routing, master, latencies)?;
        Ok(Self {
            revision,
            sample_rate,
            bpm: DEFAULT_BPM,
            block_frames,
            channels,
            master,
            tracks,
            schedules: BTreeMap::new(),
            tones: BTreeMap::new(),
            pan_law: PanLaw::default(),
            scheduled_notes: 0,
            note_schedule_drops: 0,
            pdc,
        })
    }

    /// 附上音符调度表（构造期**允许分配**：这一步在控制线程上）。
    ///
    /// `dropped` 是构造调度表时因容量上限丢弃的音符条数
    /// （见 [`crate::synth::MAX_NOTES_PER_TRACK`]）。
    #[must_use]
    pub fn with_schedules(
        mut self,
        schedules: BTreeMap<EntityId, NoteSchedule>,
        dropped: u64,
    ) -> Self {
        self.scheduled_notes = schedules.values().map(NoteSchedule::len).sum();
        self.note_schedule_drops = dropped;
        self.schedules = schedules;
        self
    }

    /// 附上每轨的音色参数（**引擎侧临时形状**，见 [`crate::synth::ToneParams`]）。
    ///
    /// 不在表里的轨道在实时侧退回**旁通** ⇒ 逐位不变（这是默认口径）。
    #[must_use]
    pub fn with_tones(mut self, tones: BTreeMap<EntityId, ToneParams>) -> Self {
        self.tones = tones;
        self
    }

    /// 覆盖声相衰减律（**投影自 `audio_config.pan_law`**；低层构造默认默认律）。
    #[must_use]
    pub const fn with_pan_law(mut self, law: PanLaw) -> Self {
        self.pan_law = law;
        self
    }

    /// 覆盖工程速度（**投影自 `YebanProjectV1::bpm`**；低层构造默认 120 BPM）。
    ///
    /// 非有限值 / 越界值在这里**不被**钳位：钳位是走带侧 [`crate::transport::Transport::arm`]
    /// 的兜底（它必须对任何输入都不 panic）。快照保留模型给的原值，这样"模型里写了什么"
    /// 与"引擎看到了什么"是同一份事实。
    #[must_use]
    pub const fn with_bpm(mut self, bpm: f64) -> Self {
        self.bpm = bpm;
        self
    }

    /// 模型层提交版本号。
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// 采样率 (Hz)——来自 `audio_config.sample_rate`，本 crate 不再另立事实源。
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 工程速度（BPM）——`YebanProjectV1::bpm` 的投影 [MODEL-AST-002]。
    #[must_use]
    pub const fn bpm(&self) -> f64 {
        self.bpm
    }

    /// 渲染量子帧数（[ARCH-DET-001] 的 L1 固定块长，默认 128）。
    #[must_use]
    pub const fn block_frames(&self) -> usize {
        self.block_frames
    }

    /// 输出通道数。
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// 主总线节点身份。
    #[must_use]
    pub const fn master(&self) -> EntityId {
        self.master
    }

    /// 音轨参数表（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    #[must_use]
    pub const fn tracks(&self) -> &BTreeMap<EntityId, TrackParams> {
        &self.tracks
    }

    /// 单轨参数。
    #[must_use]
    pub fn track(&self, id: &EntityId) -> Option<&TrackParams> {
        self.tracks.get(id)
    }

    /// 全部音轨的音符调度表（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    #[must_use]
    pub const fn schedules(&self) -> &BTreeMap<EntityId, NoteSchedule> {
        &self.schedules
    }

    /// 单轨的音符调度表（没有该轨时为 `None`）。
    #[must_use]
    pub fn schedule(&self, id: &EntityId) -> Option<&NoteSchedule> {
        self.schedules.get(id)
    }

    /// 全部音轨的**音色参数**（`BTreeMap` ⇒ 迭代顺序确定 [MODEL-AST-003]）。
    ///
    /// ⚠ **引擎侧临时形状**：见字段文档与 [`crate::synth::ToneParams`]。
    #[must_use]
    pub const fn tones(&self) -> &BTreeMap<EntityId, ToneParams> {
        &self.tones
    }

    /// 单轨的音色参数（没有该轨时为 `None` ⇒ 实时侧按旁通处理）。
    #[must_use]
    pub fn tone(&self, id: &EntityId) -> Option<&ToneParams> {
        self.tones.get(id)
    }

    /// 声相衰减律（`audio_config.pan_law` 的投影）。
    #[must_use]
    pub const fn pan_law(&self) -> PanLaw {
        self.pan_law
    }

    /// 本快照已调度的音符总条数。
    #[must_use]
    pub const fn scheduled_notes(&self) -> usize {
        self.scheduled_notes
    }

    /// 构造调度表时因容量上限丢弃的音符条数。
    #[must_use]
    pub const fn note_schedule_drops(&self) -> u64 {
        self.note_schedule_drops
    }

    /// PDC 计划（拓扑序 + 每节点补偿延迟）。
    #[must_use]
    pub const fn pdc(&self) -> &PdcPlan {
        &self.pdc
    }

    /// 模型层 `BlockSize` 枚举是否与本快照的块长一致（L1 契约自检）。
    #[must_use]
    pub fn block_size_matches_enum(&self) -> bool {
        BlockSize::from_frames(self.block_frames as u32)
            .map(|size| size.frames() as usize == self.block_frames)
            .unwrap_or(false)
    }
}

/// [MUST-GATE-012] 的**释放线程**观测点：本快照的最后一个强引用被丢掉时记一次账。
///
/// 为什么值得有一个手写 `Drop`（而不是只靠 `RetireQueue::dropped()`）：
/// 队列的 `dropped()` 记的是"主线程**出队**了几条"，而**释放**（`Arc` 强引用归零 ⇒
/// 真正的 `dealloc`）可能发生在任何人身上。把 `Drop` 变成可观测事件之后，
/// "音频线程释放 0 个"与"创建数 = 释放数 + 存活数"才是**测出来的**。
/// 详见 [`release_probe`] 模块文档。
impl Drop for EngineSnapshot {
    fn drop(&mut self) {
        release_probe::note_release();
    }
}

/// 把工程的 MIDI 摆放投影成"每轨一份已调度音符表"[ROAD-M2-005, ROAD-M2-006]。
///
/// 这是**控制线程**上的纯投影（允许分配、允许超越函数），实时侧只读结果：
///
/// ```text
/// TrackV3.clips ─► ClipPlacement ─► clip_pool[clip_id] 为 Midi ─► BTreeMap<EntityId, MidiNote>
///   │  起点 tick = placement.start_tick + note.start_tick + micro_timing_ticks
///   │  ratchet 把时值等分成 N 个脉冲（整数除法，余数不补）
///   │  probability 用 MidiNote::triggers(project.rng_seed) 做**确定性**判定
///   ▼  tick → sample（一次 f64 换算，IEEE 精确类）
/// ScheduledNote { start_sample, end_sample, phase_inc, freq_hz, gain }
/// ```
///
/// 语义裁决（本切片明确采取的口径，未在规范里定义的都登记在
/// `docs/ledger/engine-sound-notes.md` 的 needs）：
///
/// 1. **音符时值被裁剪到摆放区间** `[start_tick, start_tick + duration_ticks)`；
/// 2. **`loop_config` 不展开**（一个摆放只播一遍），坐标语义待裁决；
/// 3. **力度 0 仍然进调度表**，由 `velocity_gain(0) == 0.0` 让它静音 ——
///    这样"力度 0 不发声"是**增益路径**的判据，而不是"被调度器丢掉"的巧合；
/// 4. **静音/独奏**在构造期折算成增益门（`track_is_audible`）：
///    不可闻的轨道照常触发声部，但增益恒为 0 ⇒ 输出逐位为 0；
/// 5. **`ClipContent::Audio` 不产生声音**（采样播放/SFZ 尚未接入，见 notes）；
/// 6. 每轨超过 [`MAX_NOTES_PER_TRACK`] 的音符被丢弃并计数。
fn project_schedules(project: &YebanProjectV1) -> (BTreeMap<EntityId, NoteSchedule>, u64) {
    let sample_rate = project.audio_config.sample_rate.hz();
    #[allow(clippy::cast_precision_loss)]
    let sample_rate_f32 = sample_rate as f32;
    let samples_per_tick = crate::synth::samples_per_tick(project.bpm, sample_rate);
    let any_solo = project.tracks.values().any(|track| track.solo);
    let mut schedules: BTreeMap<EntityId, NoteSchedule> = BTreeMap::new();
    let mut dropped = 0u64;

    for (id, track) in &project.tracks {
        let audible =
            crate::synth::track_is_audible(track.mute, track.solo, track.solo_safe, any_solo);
        let gain = if audible {
            crate::synth::track_gain(track.volume_db)
        } else {
            0.0
        };
        let mut notes: Vec<ScheduledNote> = Vec::new();

        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let ClipContent::Midi { notes: pool } = &entry.content else {
                continue;
            };
            let placement_start = i128::from(placement.start_tick);
            let placement_end = placement_start + i128::from(placement.duration_ticks);

            for note in pool.values() {
                // 确定性概率触发：种子来自工程（`rng_seed`），与调用次数/顺序无关
                // [MODEL-AST-005, ARCH-DET-001]。
                if !note.triggers(project.rng_seed) {
                    continue;
                }
                let ratchet = u64::from(note.ratchet.unwrap_or(1).clamp(1, 16));
                let step = (note.duration_ticks / ratchet).max(1);
                let offset =
                    i128::from(note.start_tick) + i128::from(note.micro_timing_ticks.unwrap_or(0));

                for pulse in 0..ratchet {
                    #[allow(clippy::cast_possible_wrap)]
                    let raw_start = placement_start + offset + (pulse * step) as i128;
                    let start_tick = raw_start.clamp(0, placement_end).min(i128::from(u64::MAX));
                    let end_tick = (start_tick + i128::from(step))
                        .min(placement_end)
                        .min(i128::from(u64::MAX));
                    if end_tick <= start_tick {
                        continue;
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let (start_tick, end_tick) = (start_tick as u64, end_tick as u64);
                    let (Some(start_sample), Some(end_sample)) = (
                        tick_to_sample(start_tick, samples_per_tick),
                        tick_to_sample(end_tick, samples_per_tick),
                    ) else {
                        continue;
                    };
                    if end_sample <= start_sample {
                        continue;
                    }
                    if notes.len() >= MAX_NOTES_PER_TRACK {
                        dropped = dropped.saturating_add(1);
                        continue;
                    }
                    notes.push(ScheduledNote::new(
                        start_sample,
                        end_sample,
                        note.pitch,
                        note.velocity,
                        note_to_hz(f32::from(note.pitch)),
                        velocity_gain(note.velocity) * gain,
                        sample_rate_f32,
                    ));
                }
            }
        }

        // 排序 ⇒ 实时侧的单调游标成立（`BTreeMap` 顺序不等于时间顺序）。
        // 稳定排序 + 全序 key ⇒ 迭代顺序与平台无关 [MODEL-AST-003]。
        notes.sort_by_key(|note| (note.start_sample(), note.pitch(), note.end_sample()));
        schedules.insert(*id, NoteSchedule::from_sorted(notes));
    }

    (schedules, dropped)
}

/// 饱和自增（**不回绕**）：到 `u64::MAX` 就停住，返回新值。
///
/// 为什么不用 `fetch_add`：`AtomicU64` 没有饱和加法，而"计数溢出回绕"会把一条
/// "健康读数"变成一条**看似回到 0 的谎报**（`docs/ledger/engine-stats-notes.md` §判据⑥）。
/// CAS 循环无锁、无分配 —— 它只在**控制线程**的 `drain`/`prune` 路径上跑（不在渲染热路径）。
fn saturating_bump(counter: &AtomicU64, delta: u64) -> u64 {
    if delta == 0 {
        return counter.load(Ordering::Acquire);
    }
    // 手写 CAS 循环（而不是 `fetch_update`）：它是 stable 上恒久可用的形式，
    // 不受某一个工具链版本对 `fetch_update` 改名/弃用的影响（本仓库钉 1.99.0）。
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let next = current.saturating_add(delta);
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

/// 退役队列的**跨线程记账**：控制线程写（`drain`），音频线程与任意读者只读。
///
/// 存在的理由（[MUST-GATE-012] 的观测面，`needs` N2/N5 的落地）：
/// [`RetireQueue`] 自己的计数只有**队列持有者**（控制线程）读得到，而"控制面必须能看见的
/// 引擎健康读数"要能从**渲染驱动**（[`crate::rt::EngineRuntime::stats`]）读到 ——
/// 而驱动只拿得到生产端。于是把计数放进一个与 [`RetireProducer`] 共享的 `Arc` 结构：
///
/// | 字段 | 谁写 | 语义 | 单调性 |
/// | :--- | :--- | :--- | :--- |
/// | `pushed` | 音频线程 `push` 成功（+1，一次 `fetch_add`） | 累计**成功入队**的条数 | 单调不减（占用以队列容量为界 ⇒ 不会回绕） |
/// | `drained` | 控制线程 `drain` | 累计真正出队并 `Drop` 的条数 | 单调不减（饱和） |
/// | `drain_calls` | 控制线程 `drain` | 累计**非空** `drain` 调用次数 | 单调不减（饱和） |
/// | `foreign_drains` | 控制线程 `drain` | 在释放归属线程之外发生的非空 `drain` 次数 | 单调不减（饱和） |
/// | `release_thread_is_owner` | 控制线程**首个**非空 `drain` | 首个执行释放的线程是否即归属线程 | 只可能 `true → false` |
///
/// # 为什么 `pending` 是**两个单调量之差**，而不是一个被 `drain` 覆写的量规
///
/// 第一版（`line/engine-stats`）把 `pending` 做成了一个量规：`push` 成功 `+1`，
/// `drain` 之后**覆写**为"出队后的真实剩余"。那个覆写与音频线程的 `+1` **不是原子的**：
///
/// ```text
///   控制线程                                    音频线程
///   ────────────────────────────────────────    ─────────────────────────
///   ① 求值实参 self.consumer.slots() = R
///      （此后还要跑两条饱和 CAS 才 store）
///                                              ② inner.push(X)   ← 环里有 X 了
///                                              ③ note_push()     ← 量规 +1
///   ④ pending.store(R)                        ← ② 的 +1 被**永久盖掉**
/// ```
///
/// ① 与 ④ 之间隔着 `drain_calls` / `drained` 两条 CAS 循环（[`saturating_bump`]），
/// 在两条线程抢同一条缓存行时这是一个**几百纳秒**的窗口 —— 于是镜像会**永久**少记一条，
/// 而"静止点上镜像 == 权威"的判据当场变红（CI 实测 `retire_pending 镜像=512 权威=513`）。
///
/// 修法：两个方向都做成**只增不减**的计数（`pushed` / `drained`），
/// [`pending`](Self::pending) 取 `pushed.saturating_sub(drained)` —— 没有任何
/// "读旧值再覆写"的步骤，于是**不存在**能丢掉一次 `+1` 的窗口；静止点上它与
/// `Consumer::slots()` 恒等。代价是失去"覆写自愈"：那个自愈只在**下一次** drain 生效，
/// 而"最后一轮 drain 之后到静止点之间"根本没有下一次 —— 所以它不是保险，是缺陷来源。
/// 现在改成"按构造不可能漂"，并由 `snapshot_retire_churn` 的静止点判据 + 本模块的
/// 并发见证单测（`pending_mirror_stays_exact_while_a_drain_races_a_push`）钉住。
///
/// **零锁、零分配**：全是原子 `load`/`store`/CAS。`pushed` 的 `+1` 用一次 `fetch_add`
/// （占用以队列容量为界 ⇒ 永不可能回绕），累计量用饱和 CAS（[`saturating_bump`]）。
#[derive(Debug)]
pub struct RetireAccounting {
    /// **释放归属线程**：创建退役队列的那个线程（控制线程 = 60Hz 排空循环所在线程）。
    ///
    /// 之后**不可变**（因此共享它不需要原子量）。它是
    /// [`release_thread_is_owner`](Self::release_thread_is_owner) 的比较基准，
    /// 与 `gate-snapshot-churn` 判据里的 `queue.release_thread() == Some(main_thread)` 同口径
    /// （那条工作线的 harness 正是在主线程上建队列 ⇒ 两者逐项一致）。
    owner_thread: ThreadId,
    /// 累计**成功入队**的条数（音频线程一次 `fetch_add`；只增不减）。
    pushed: AtomicU64,
    /// 累计**真正出队并 `Drop`** 的条数（控制线程饱和加法；只增不减）。
    ///
    /// `pending()` = `pushed - drained`：两者都是单调量 ⇒ 差值不会因为一次
    /// 并发 `push` + `drain` 的交错而**永久**偏掉（见结构体文档的窗口图）。
    drained: AtomicU64,
    drain_calls: AtomicU64,
    foreign_drains: AtomicU64,
    release_thread_is_owner: AtomicBool,
}

impl RetireAccounting {
    fn new(owner_thread: ThreadId) -> Self {
        Self {
            owner_thread,
            pushed: AtomicU64::new(0),
            drained: AtomicU64::new(0),
            drain_calls: AtomicU64::new(0),
            foreign_drains: AtomicU64::new(0),
            // **vacuous 真**：还没有任何非空 `drain` ⇒ 没有观测到"外来释放"。
            // 这与"首个 drain 的线程不是归属线程 ⇒ 立刻置假"合起来给出完整语义。
            release_thread_is_owner: AtomicBool::new(true),
        }
    }

    /// 队列当前占用的跨线程镜像 = **成功入队数 − 已出队数**（控制线程上请用精确读
    /// [`RetireQueue::pending`]）。
    ///
    /// 两个操作数都只增不减 ⇒ 静止点上它与 `Consumer::slots()` **恒等**；
    /// 并发窗口里它可能瞬时差 1（环里已经放进去、`+1` 还没落地），那是**量规**的正常语义，
    /// 不是漂移（漂移的定义是"静止点上仍然不等"）。
    #[must_use]
    pub fn pending(&self) -> u64 {
        self.pushed
            .load(Ordering::Acquire)
            .saturating_sub(self.drained.load(Ordering::Acquire))
    }

    /// 累计**成功入队**的旧快照条数（只增不减；[`pending`](Self::pending) 的被减数）。
    #[must_use]
    pub fn pushed(&self) -> u64 {
        self.pushed.load(Ordering::Acquire)
    }

    /// 累计真正出队并 `Drop` 的旧快照条数。
    #[must_use]
    pub fn drained(&self) -> u64 {
        self.drained.load(Ordering::Acquire)
    }

    /// 累计**非空** `drain` 调用次数（结构性判据：60Hz 排空循环真的在跑）。
    #[must_use]
    pub fn drain_calls(&self) -> u64 {
        self.drain_calls.load(Ordering::Acquire)
    }

    /// 在**释放归属线程之外**发生的非空 `drain` 次数（`> 0` = 释放没有集中在一个线程）。
    #[must_use]
    pub fn foreign_drains(&self) -> u64 {
        self.foreign_drains.load(Ordering::Acquire)
    }

    /// 首个执行释放的线程是否就是**释放归属线程**（[`Self::owner_thread`]）。
    ///
    /// 尚无任何非空 `drain` 时为 `true`（vacuously：没有观测到外来释放）。
    #[must_use]
    pub fn release_thread_is_owner(&self) -> bool {
        self.release_thread_is_owner.load(Ordering::Acquire)
    }

    /// 释放归属线程（退役队列的创建线程）。
    #[must_use]
    pub fn owner_thread(&self) -> ThreadId {
        self.owner_thread
    }

    /// 音频线程侧：一次成功的 `push` ⇒ **成功入队数 +1**（单次原子 RMW，无锁无分配）。
    ///
    /// ⚠ 严格发生在 `rtrb::Producer::push` **之后**（见 [`RetireProducer::push`]）：
    /// 于是"环里有这一条"永远**不晚于**"记账里有这一条"。反过来的顺序会让
    /// `drain` 在 `+1` 落地前就消费掉它，从而制造**永久**的多记（下面 `pending()`
    /// 的 `saturating_sub` 只能把瞬时负数夹成 0，夹不回那一笔）。
    fn note_push(&self) {
        // 占用以队列容量为界 ⇒ 这里**不可能**回绕；刻意不用 CAS 循环，
        // 让渲染路径上的这条记账保持"一条指令"。
        self.pushed.fetch_add(1, Ordering::Release);
    }

    /// 控制线程侧：一次非空 `drain` 的完整记账（**唯一写者**）。
    ///
    /// `taken` 是本次真正出队并 `Drop` 的条数。**没有**"覆写剩余量"这一步：
    /// 剩余量由 `pending()` 从两个单调量算出来（见结构体文档的窗口图）。
    fn note_drain(&self, taken: usize) {
        saturating_bump(&self.drain_calls, 1);
        saturating_bump(&self.drained, taken as u64);
    }

    /// 控制线程侧：钉住**首个**释放线程的归属（只可能 `true → false`）。
    fn note_release_thread(&self, is_owner: bool) {
        if !is_owner {
            self.release_thread_is_owner.store(false, Ordering::Release);
        }
    }

    /// 控制线程侧：一次来自"非首个释放线程"的非空 `drain`。
    fn note_foreign_drain(&self) {
        saturating_bump(&self.foreign_drains, 1);
    }
}

/// 退役队列的**生产端**（音频线程持有）。
///
/// 它是 `rtrb::Producer<Arc<EngineSnapshot>>` 的薄包装，唯一多出来的是与
/// [`RetireQueue`] 共享的 [`RetireAccounting`] —— 于是
/// [`crate::rt::EngineRuntime`] 只要拿到生产端，就能把退役队列的
/// `pending` / `drained` / 释放线程归属读进 [`crate::rt::EngineStats`]，
/// **控制面不需要额外接线**（[`retire_channel`] 的每个调用点一个字都不用改）。
pub struct RetireProducer {
    inner: rtrb::Producer<Arc<EngineSnapshot>>,
    accounting: Arc<RetireAccounting>,
}

impl RetireProducer {
    /// 把旧快照推进退役队列；队列满时把值**原样退回**（调用方据此寄存到 `stash`）。
    ///
    /// 实时路径：一次 `rtrb` 的 `push` + 成功时一次 `Release` 原子加
    /// （**入队数** +1；顺序刻意是"先入环、后记账"，见 `RetireAccounting::note_push`）。
    ///
    /// # Errors
    ///
    /// 队列满时返回 [`rtrb::PushError::Full`]，其中带着**未被消费的值**。
    pub fn push(
        &mut self,
        snapshot: Arc<EngineSnapshot>,
    ) -> Result<(), rtrb::PushError<Arc<EngineSnapshot>>> {
        let result = self.inner.push(snapshot);
        if result.is_ok() {
            self.accounting.note_push();
        }
        result
    }

    /// 与 [`RetireQueue`] 共享的跨线程记账（只读）。
    #[must_use]
    pub fn accounting(&self) -> &Arc<RetireAccounting> {
        &self.accounting
    }
}

impl core::fmt::Debug for RetireProducer {
    /// 手写 `Debug`：`rtrb::Producer` 的可调试性不构成本模块的契约，这里只暴露记账句柄。
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RetireProducer")
            .field("accounting", &self.accounting)
            .finish_non_exhaustive()
    }
}

/// 退役回收队列的消费端（**主线程**持有）。
///
/// 音频线程把旧快照 push 进来；主线程以 60Hz `drain` 并在这里真正 **Drop**
/// [ARCH-RT-002]。因此"释放快照"这件事永远不会发生在音频线程上。
///
/// 计数（`dropped` / `drain_calls` / `foreign_drains`）住在共享的
/// [`RetireAccounting`] 上（音频线程侧的 [`crate::rt::EngineStats`] 读得到）；
/// 只有"首个释放线程的**身份**"（[`Self::release_thread`]）留在这里 ——
/// `ThreadId` 在 stable 上无法放进原子量（`as_u64` 不稳定）。
#[derive(Debug)]
pub struct RetireQueue {
    consumer: rtrb::Consumer<Arc<EngineSnapshot>>,
    accounting: Arc<RetireAccounting>,
    /// **释放线程**：第一次真正出队（`take > 0`）的线程。`None` = 还没回收过。
    ///
    /// [MUST-GATE-012] 要求旧快照"在主线程 60Hz 循环中释放"；本字段把
    /// "哪个线程执行了 `Drop`"变成可读事实（配合 [`release_probe`] 的逐线程计数）。
    release_thread: Option<ThreadId>,
}

impl RetireQueue {
    /// 当前排队待回收的条数（**控制线程上的精确读**；跨线程镜像见
    /// [`RetireAccounting::pending`]）。
    #[must_use]
    pub fn pending(&self) -> usize {
        self.consumer.slots()
    }

    /// 累计已回收（Drop）的快照数。
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.accounting.drained()
    }

    /// 累计非空 `drain` 调用次数。
    #[must_use]
    pub fn drain_calls(&self) -> u64 {
        self.accounting.drain_calls()
    }

    /// 实际执行过释放的**线程**（首次非空 `drain` 的线程）。`None` = 从未真正回收。
    #[must_use]
    pub fn release_thread(&self) -> Option<ThreadId> {
        self.release_thread
    }

    /// 首个执行释放的线程是否就是**创建本队列的那个线程**（释放归属线程）。
    ///
    /// 与 `gate-snapshot-churn` 判据的 `queue.release_thread() == Some(main_thread)`
    /// **同一口径**（那条 harness 在主线程上建队列）⇒ 两个读数逐项一致。
    #[must_use]
    pub fn release_thread_is_main(&self) -> bool {
        self.accounting.release_thread_is_owner()
    }

    /// 在 [`release_thread`](Self::release_thread) 之外的线程上发生的非空 `drain` 次数。
    ///
    /// [MUST-GATE-012] 判据要求它恒为 `0`（所有旧快照都在**同一个主线程**上释放）。
    #[must_use]
    pub fn foreign_drains(&self) -> u64 {
        self.accounting.foreign_drains()
    }

    /// 与 [`RetireProducer`] 共享的跨线程记账（只读）。
    #[must_use]
    pub fn accounting(&self) -> &Arc<RetireAccounting> {
        &self.accounting
    }

    /// 一次性出队至多 `max` 条并 **Drop**，返回实际回收条数。
    ///
    /// 使用 `rtrb` 的**批量** API（`Consumer::read_chunk` + `IntoIterator`），
    /// 一次调用搬空整批 [ROAD-M2-007]。本函数在主线程调用，允许发生真正的 `dealloc`。
    pub fn drain(&mut self, max: usize) -> usize {
        let available = self.consumer.slots();
        let take = available.min(max);
        if take == 0 {
            return 0;
        }
        // [MUST-GATE-012] 记账**释放线程**：第一次真正出队时钉住，之后再从别的线程出队
        // 就记一次 `foreign_drains`（判据据此变红）。
        let here = std::thread::current().id();
        match self.release_thread {
            None => {
                self.release_thread = Some(here);
                // **首个**释放线程的归属：与 `foreign_drains` 是两个不同的判据 ——
                // 首个 drain 就跑在外线程时 `foreign_drains` 仍是 0，只有这条布尔量抓得住。
                self.accounting
                    .note_release_thread(here == self.accounting.owner_thread());
            }
            Some(owner) if owner != here => self.accounting.note_foreign_drain(),
            Some(_) => {}
        }
        let chunk = match self.consumer.read_chunk(take) {
            Ok(chunk) => chunk,
            // slots() 与 read_chunk 之间没有别的消费者，这里理论上不可达；
            // 真发生了也只是"这一轮少回收一点"，下一轮再来 —— 不 panic。
            //
            // 记账：**什么都不记**（一条都没出队）。这与 `take == 0` 的早退口径一致：
            // `drain_calls` 数的是"**非空** drain 调用"。旧实现在这里记一次 `drain_calls`，
            // 与它自己的字段语义矛盾（见 `RetireAccounting` 的字段表）。
            Err(_) => return 0,
        };
        let mut count = 0usize;
        for snapshot in chunk {
            // 主线程负责 Drop [ARCH-RT-002]：这是整个设计的目的所在。
            drop(snapshot);
            count += 1;
        }
        // 只记"取走了几条"；占用镜像由 `RetireAccounting::pending()` 从
        // 两个单调量算出来（**没有**覆写，见结构体文档里的窗口图）。
        self.accounting.note_drain(count);
        count
    }
}

/// 建立退役队列：[`RetireProducer`] 给音频线程，[`RetireQueue`] 给主线程。
///
/// 两者共享同一个 [`RetireAccounting`]（在**本调用所在的线程**上创建 ⇒ 该线程
/// 就是"释放归属线程"，见 [`RetireAccounting::owner_thread`]）。
#[must_use]
pub fn retire_channel(capacity: usize) -> (RetireProducer, RetireQueue) {
    let (producer, consumer) = rtrb::RingBuffer::<Arc<EngineSnapshot>>::new(capacity.max(1));
    let accounting = Arc::new(RetireAccounting::new(std::thread::current().id()));
    (
        RetireProducer {
            inner: producer,
            accounting: Arc::clone(&accounting),
        },
        RetireQueue {
            consumer,
            accounting,
            release_thread: None,
        },
    )
}

/// 快照原子交换槽的写者侧（Model 线程持有）。
///
/// 读者通过 [`SnapshotReader::attach`] 在册，并在回调线程内读 `ptr`。
pub struct SnapshotSlot {
    /// 当前快照指针（Release 写 / Acquire 读）。
    ptr: AtomicPtr<EngineSnapshot>,
    /// 当前快照的**锚**：只要槽活着，`ptr` 就永不悬垂。
    anchor: Mutex<Arc<EngineSnapshot>>,
    /// 纪元：每次 publish 后 +1。读者用它证明"我不会再碰被淘汰的快照"。
    epoch: AtomicU64,
    /// 读者已完整结束的最大纪元。
    reader_done: AtomicU64,
    /// 是否存在在册读者。没有读者时待回收项可以立即释放。
    reader_attached: AtomicBool,
    /// 已被 `ptr` 淘汰、但在读者确认之前必须保持存活的强引用。
    pending: Mutex<Vec<(u64, Arc<EngineSnapshot>)>>,
    published: AtomicU64,
    /// 累计由 [`prune`](Self::prune) 释放的写者侧强引用条数（[MUST-GATE-012] 的观测面）。
    ///
    /// 它补上退役回收的另一半：退役队列回收的是"读者曾经持有的那一份"，`prune` 回收的是
    /// "写者 anchor 淘汰下来的那一份"；两者都是同一个快照的强引用。控制面需要看到
    /// **两条**回收路径都在动，否则"队列空了"可能只是"写者侧清单在涨"。
    pruned: AtomicU64,
}

// 线程安全说明：`SnapshotSlot` 的每个字段都是 `Send + Sync` —— `AtomicPtr`/`AtomicU64`/
// `AtomicBool` 是无条件 `Send + Sync` 的原子类型，`Mutex<T>` 在 `T: Send` 时是 `Send + Sync`。
// 因此**不需要**手写 `unsafe impl`；`mod tests` 里的 `assert_send_sync::<SnapshotSlot>()`
// 把这一点钉在编译期。
impl SnapshotSlot {
    /// 用初始快照建立交换槽。
    #[must_use]
    pub fn new(initial: EngineSnapshot) -> Arc<Self> {
        let anchor = Arc::new(initial);
        let raw = Arc::as_ptr(&anchor).cast_mut();
        Arc::new(Self {
            ptr: AtomicPtr::new(raw),
            anchor: Mutex::new(anchor),
            epoch: AtomicU64::new(0),
            reader_done: AtomicU64::new(0),
            reader_attached: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
            published: AtomicU64::new(1),
            pruned: AtomicU64::new(0),
        })
    }

    /// 发布一个新快照（**Model 线程**调用）。
    ///
    /// 旧快照的强引用被移入待回收清单，由 [`prune`](Self::prune) 在读者确认之后释放。
    /// 写者路径允许 `Mutex`（它不是实时线程）[ARCH-TOP-002]。
    pub fn publish(&self, snapshot: EngineSnapshot) {
        self.publish_arc(Arc::new(snapshot));
    }

    /// 以 `Arc` 形式发布（避免调用方多余的一次 `Arc::new`）。
    pub fn publish_arc(&self, snapshot: Arc<EngineSnapshot>) {
        let new_raw = Arc::as_ptr(&snapshot).cast_mut();
        let old = {
            let mut anchor = lock(&self.anchor);
            let old = std::mem::replace(&mut *anchor, snapshot);
            // Release: 与读者的 Acquire load 配对；且必须早于 `epoch.fetch_add`。
            self.ptr.store(new_raw, Ordering::Release);
            old
        };
        let retired_at = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        lock(&self.pending).push((retired_at, old));
        self.published.fetch_add(1, Ordering::Release);
    }

    /// 当前快照的指针（仅用于诊断/测试；正常读者走 [`SnapshotReader`]）。
    #[must_use]
    pub fn current_ptr(&self) -> *const EngineSnapshot {
        self.ptr.load(Ordering::Acquire)
    }

    /// 当前快照的 `Arc` 克隆（**非实时路径**：它会在写者锁上短暂等待）。
    #[must_use]
    pub fn current(&self) -> Arc<EngineSnapshot> {
        Arc::clone(&lock(&self.anchor))
    }

    /// 纪元计数（每次 publish +1）。
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// 累计 publish 次数。
    #[must_use]
    pub fn published(&self) -> u64 {
        self.published.load(Ordering::Acquire)
    }

    /// 写者侧待回收清单长度（诊断用；不是实时路径）。
    #[must_use]
    pub fn pending_len(&self) -> usize {
        lock(&self.pending).len()
    }

    /// 累计由 [`prune`](Self::prune) 释放的写者侧强引用条数（单调不减，饱和）。
    #[must_use]
    pub fn pruned(&self) -> u64 {
        self.pruned.load(Ordering::Acquire)
    }

    /// 释放所有已被读者确认淘汰的强引用，返回释放条数（**主线程 60Hz** 调用）。
    ///
    /// 这是"内存回收"发生的地方；音频线程只 `push`，从不 `drop` [ARCH-RT-002]。
    pub fn prune(&self) -> usize {
        let mut pending = lock(&self.pending);
        let before = pending.len();
        if !self.reader_attached.load(Ordering::Acquire) {
            // 没有在册读者 ⇒ 没有任何在途指针解引用 ⇒ 全部可以立即释放。
            pending.clear();
            // 释放条数在 `pending` 被清空**之前**取（`before`），且必须记进**饱和**累计量。
            saturating_bump(&self.pruned, before as u64);
            return before;
        }
        let done = self.reader_done.load(Ordering::Acquire);
        pending.retain(|(retired_at, _)| *retired_at > done);
        let released = before - pending.len();
        saturating_bump(&self.pruned, released as u64);
        released
    }
}

/// 快照原子交换槽的读者侧（**cpal 回调线程**持有）。
///
/// 使用方式（每个渲染量子一次，见 [`crate::rt`]）：
///
/// ```ignore
/// if let Some(snapshot) = reader.begin_block() {
///     let frames = snapshot.block_frames();
///     // … 只读访问, 零分配、零锁 …
/// }
/// reader.end_block();
/// ```
pub struct SnapshotReader {
    slot: Arc<SnapshotSlot>,
    retire: RetireProducer,
    held: Option<Arc<EngineSnapshot>>,
    /// 当前持有快照的**地址**（不保存裸指针：裸指针 `*const T` 是 `!Send`，
    /// 而整个读者必须能 move 进 cpal 的回调线程）。地址比较与指针比较语义相同，
    /// 且 `*const T as usize` 是安全转换。
    held_addr: usize,
    /// 退役队列满时的临时寄存位（仍有强引用 ⇒ 不会在音频线程释放）。
    stash: Option<Arc<EngineSnapshot>>,
    local_epoch: u64,
    switches: u64,
    stash_events: u64,
}

impl SnapshotReader {
    /// 在册化一个读者。**必须在音频线程启动前调用一次**。
    #[must_use]
    pub fn attach(slot: &Arc<SnapshotSlot>, retire: RetireProducer) -> Self {
        // 先复位进度再登记: 顺序无关紧要（见模块文档的证明），
        // 但"先保守后激进"更不容易出错。
        slot.reader_done.store(0, Ordering::Release);
        slot.reader_attached.store(true, Ordering::Release);
        let held = Arc::clone(&lock(&slot.anchor));
        let held_addr = Arc::as_ptr(&held) as usize;
        Self {
            slot: Arc::clone(slot),
            retire,
            held: Some(held),
            held_addr,
            stash: None,
            local_epoch: 0,
            switches: 0,
            stash_events: 0,
        }
    }

    /// 本块开始时调用：必要时切换到最新快照；返回当前快照的只读引用。
    ///
    /// 实时安全：一次 `Acquire` 载入 +（仅在换快照时）一次引用计数 +1 与一次 `push`。
    /// **没有分配、没有锁、没有阻塞** [红线 7]。
    ///
    /// 若退役队列满（`stash` 也被占用），本块**放弃切换**、继续用旧快照：
    /// 宁可晚一个块切拓扑，也绝不在音频线程释放内存 [ARCH-RT-002]。
    pub fn begin_block(&mut self) -> Option<&Arc<EngineSnapshot>> {
        self.local_epoch = self.slot.epoch.load(Ordering::Acquire);
        self.flush_stash();

        let current = self.slot.ptr.load(Ordering::Acquire);
        let unchanged = current as usize == self.held_addr;
        if !current.is_null() && !unchanged && self.stash.is_none() {
            // SAFETY: `current` 来自 `Arc::as_ptr`（写者 anchor 持有的那个 `Arc`），
            // 且模块文档证明了以下不变式：
            //   1. anchor 始终持有当前快照的强引用 ⇒ 在本次 `begin_block` 期间
            //      `current` 指向的分配一定活着（引用计数 >= 1）；
            //   2. 本块读到的纪元 `e` 之后，写者绝不会把 `current` 判定为"可淘汰"，
            //      因为 `prune` 要求 `reader_done >= retired_at`，而读者要到 `end_block`
            //      才会把 `reader_done` 推到 `e`；
            //   3. 因此 `increment_strong_count` + `from_raw` 是"把已存在的强引用
            //      再复制一份"，与 `Arc::increment_strong_count` 官方文档给出的
            //      `as_ptr` → `increment_strong_count` → `from_raw` 用法完全一致，
            //      不会产生悬垂或二次释放。
            unsafe {
                Arc::increment_strong_count(current);
                let new = Arc::from_raw(current);
                let old = self.held.replace(new);
                self.held_addr = current as usize;
                self.switches = self.switches.saturating_add(1);
                if let Some(old) = old {
                    self.retire_or_stash(old);
                }
            }
        }
        self.held.as_ref()
    }

    /// 本块结束时调用：向写者公布"我已完整结束纪元 `e` 的块"。
    ///
    /// `fetch_max` 保证进度单调，即使块与块之间发生乱序重排也不会回退。
    pub fn end_block(&mut self) {
        self.slot
            .reader_done
            .fetch_max(self.local_epoch, Ordering::Release);
    }

    /// 当前持有的快照。
    #[must_use]
    pub fn held(&self) -> Option<&Arc<EngineSnapshot>> {
        self.held.as_ref()
    }

    /// 累计完成的快照切换次数。
    #[must_use]
    pub const fn switches(&self) -> u64 {
        self.switches
    }

    /// 因退役队列满而暂时寄存的次数（> 0 说明主线程 drain 不及时）。
    #[must_use]
    pub const fn stash_events(&self) -> u64 {
        self.stash_events
    }

    /// 与 [`RetireQueue`] 共享的退役队列记账（跨线程只读；[`crate::rt::EngineStats`] 读它）。
    #[must_use]
    pub fn retire_accounting(&self) -> &Arc<RetireAccounting> {
        self.retire.accounting()
    }

    /// 写者侧待回收清单**累计**释放条数（= [`SnapshotSlot::pruned`]）。
    ///
    /// 渲染驱动把这条读数并进 [`crate::rt::EngineStats::retire_pruned`]。
    #[must_use]
    pub fn pruned(&self) -> u64 {
        self.slot.pruned()
    }

    /// 把旧快照推进退役队列；队列满则寄存到 `stash`（下次块边界重试）。
    ///
    /// 队列满是一条真正的告警（主线程 60Hz 轮询没跟上）⇒ 走
    /// [`crate::rt_probe::note_suppressed`] 这个**纯计数**的实时诊断出口
    /// （`N6` 裁决 = 选项 A）：本函数在音频回调的调用树里
    /// （`begin_block` ← `render_block`），因此**不许**触到 [`crate::rt_probe::diag`]
    /// 那个 I/O 边界 —— 一旦有人给发布构建装了写文件的 sink，后者就是实时线程上的
    /// 真实阻塞 I/O（历史实测：容量不足时 `io_requests == io_ops == 400`）。
    ///
    /// 事实没有丢：`stash_events` 是本路径自己的计数器
    /// （[`SnapshotReader::stash_events`] → [`crate::rt::EngineStats::snapshot_stash_events`]，
    /// 控制面 60Hz 可读），`rt_probe` 一侧另有按种类的累计读数。
    fn retire_or_stash(&mut self, old: Arc<EngineSnapshot>) {
        match self.retire.push(old) {
            Ok(()) => {}
            Err(rtrb::PushError::Full(arc)) => {
                self.stash = Some(arc);
                self.stash_events = self.stash_events.saturating_add(1);
                crate::rt_probe::note_suppressed(crate::rt_probe::RtDiagEvent::SnapshotRetireStash);
            }
        }
    }

    /// 冲刷寄存位（块边界调用，属于实时路径）。
    fn flush_stash(&mut self) {
        if let Some(pending) = self.stash.take() {
            match self.retire.push(pending) {
                Ok(()) => {}
                Err(rtrb::PushError::Full(arc)) => {
                    self.stash = Some(arc);
                }
            }
        }
    }
}

impl Drop for SnapshotReader {
    fn drop(&mut self) {
        // 读者退场: 之后不会再有指针解引用 ⇒ 允许写者释放全部待回收项。
        self.slot.reader_done.store(u64::MAX, Ordering::Release);
        self.slot.reader_attached.store(false, Ordering::Release);
        // 寄存位里的旧快照在这里释放 —— 此时已经不在音频线程上（读者已 Drop），
        // 因此不违反"音频线程零释放"。
        self.stash = None;
    }
}

/// 取写者侧锁；中毒时取回内部数据（写者路径上宁可继续也不 panic [红线 7 的精神]）。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use yeban_model::{RoutingEdge, RoutingKind};

    fn simple_project() -> YebanProjectV1 {
        let master = EntityId::new();
        let track = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![track, master],
            ..RoutingGraph::default()
        };
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        let mut tracks = BTreeMap::new();
        tracks.insert(
            track,
            TrackV3 {
                id: track,
                volume_db: -6.0,
                ..TrackV3::default()
            },
        );
        YebanProjectV1 {
            master_bus_track_id: master,
            routing_graph: routing,
            tracks,
            ..YebanProjectV1::default()
        }
    }

    fn snapshot_with(frames: usize, revision: u64) -> EngineSnapshot {
        let master = EntityId::new();
        let mut routing = RoutingGraph::default();
        routing.nodes.push(master);
        EngineSnapshot::from_parts(
            revision,
            48_000,
            frames,
            2,
            master,
            BTreeMap::new(),
            &routing,
            &LatencyTable::new(),
        )
        .expect("单节点图合法")
    }

    #[test]
    fn project_projection_carries_track_params_and_pdc() {
        let project = simple_project();
        let snapshot = EngineSnapshot::from_project(&project, 7).expect("投影成功");
        assert_eq!(snapshot.revision(), 7);
        assert_eq!(snapshot.sample_rate(), 48_000);
        assert_eq!(snapshot.block_frames(), 256, "BlockSize 默认是 256");
        assert!(snapshot.block_size_matches_enum());
        assert_eq!(snapshot.master(), project.master_bus_track_id);
        assert_eq!(snapshot.tracks().len(), 1);
        let params = snapshot
            .track(&project.tracks.keys().next().copied().unwrap())
            .expect("轨道参数已投影");
        assert_eq!(params.volume_db(), -6.0);
        assert_eq!(params.pan(), 0.0);
        assert!(!params.is_muted());
        assert_eq!(params.device_count(), 0);
        assert_eq!(snapshot.pdc().total_latency(), 0);
        assert_eq!(snapshot.pdc().compensation(&snapshot.master()), Some(0));
    }

    #[test]
    fn project_without_master_node_is_rejected_explicitly() {
        // 默认空工程的 master_bus_track_id 是 nil ULID
        let project = YebanProjectV1::default();
        assert_eq!(
            EngineSnapshot::from_project(&project, 0),
            Err(SnapshotError::NoMasterBus)
        );

        // master id 存在但不在路由节点里 → 同样是 NoMasterBus
        let mut project = simple_project();
        let master = project.master_bus_track_id;
        project.routing_graph.nodes.retain(|node| *node != master);
        assert_eq!(
            EngineSnapshot::from_project(&project, 0),
            Err(SnapshotError::NoMasterBus)
        );
    }

    #[test]
    fn cyclic_routing_propagates_pdc_error() {
        let a = EntityId::new();
        let b = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![a, b],
            ..RoutingGraph::default()
        };
        for (source, destination) in [(a, b), (b, a)] {
            let id = EntityId::new();
            routing.edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: source,
                    destination_node: destination,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            );
        }
        let project = YebanProjectV1 {
            master_bus_track_id: a,
            routing_graph: routing,
            ..YebanProjectV1::default()
        };
        match EngineSnapshot::from_project(&project, 0) {
            Err(SnapshotError::Pdc(PdcError::Cycle { nodes })) => {
                let expected: BTreeSet<EntityId> = [a, b].into_iter().collect();
                assert_eq!(nodes.into_iter().collect::<BTreeSet<_>>(), expected);
            }
            other => panic!("预期 Cycle 错误, 实际 {other:?}"),
        }
    }

    /// 判据 (a)：**退役队列不泄漏** —— 旧快照在 drain 之后引用计数归零。
    ///
    /// 用 `Weak::upgrade()` 而不是 `Arc::strong_count`，因为前者是"对象是否还活着"的
    /// 直接证据：`strong_count == 0` 也可能只是暂时读到了别的计数。
    #[test]
    fn retired_snapshots_are_dropped_after_drain_and_reference_count_hits_zero() {
        let (mut producer, mut queue) = retire_channel(8);
        let snapshot = Arc::new(snapshot_with(128, 1));
        let weak = Arc::downgrade(&snapshot);

        // 音频线程侧的动作：把旧快照 move 进队列（不清引用）
        producer.push(Arc::clone(&snapshot)).expect("队列有空间");
        assert_eq!(Arc::strong_count(&snapshot), 2);
        drop(snapshot); // 音频线程不再持有它
        assert!(weak.upgrade().is_some(), "队列还持有强引用");

        assert_eq!(queue.pending(), 1);
        assert_eq!(queue.dropped(), 0);
        assert_eq!(queue.drain(8), 1, "主线程抽取 1 条");
        assert!(
            weak.upgrade().is_none(),
            "drain 之后强引用必须归零 —— 否则就是泄漏"
        );
        assert_eq!(queue.dropped(), 1);
        assert_eq!(queue.pending(), 0);
        assert_eq!(queue.drain(8), 0, "空队列 drain 返回 0");
    }

    /// 判据 (a) 的边界：队列满时音频线程**不释放**内存，而是寄存到 `stash`，
    /// 等主线程腾出空间后再推入 —— 并且最终仍会归零。
    #[test]
    fn full_retire_queue_never_frees_on_the_audio_thread() {
        let (producer, mut queue) = retire_channel(1);
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        // 初代快照的唯一强引用由 anchor 持有；这里只留一个 Weak 用于事后判断死活。
        let initial_weak = Arc::downgrade(&slot.current());
        let mut reader = SnapshotReader::attach(&slot, producer);

        let mut weaks = Vec::new();
        for revision in 1..=4u64 {
            slot.publish(snapshot_with(128, revision));
            let held = reader.begin_block().expect("块内总有快照");
            weaks.push(Arc::downgrade(held));
            reader.end_block();
        }
        assert!(reader.switches() >= 1, "至少完成一次快照切换");
        assert!(
            reader.stash_events() > 0,
            "容量 1 的退役队列必然触发寄存 —— 这正是「音频线程不 free」的直接证据"
        );
        // **关键断言**: 主线程 drain 之前, 所有音频线程曾经持有的快照都还活着
        // ⇒ 音频线程上没有发生任何 Drop [ARCH-RT-002]。
        assert!(
            initial_weak.upgrade().is_some(),
            "主线程抽取之前, 被替换的初代快照必须仍然存活"
        );
        for weak in &weaks {
            assert!(weak.upgrade().is_some(), "音频线程不得释放任何快照");
        }

        // 主线程以 60Hz 节奏抽干
        let mut drained = 0;
        for _ in 0..16 {
            drained += queue.drain(4);
            slot.prune();
        }
        assert!(drained >= 1, "主线程必须真的回收到了东西, 实际 {drained}");
        assert!(queue.dropped() >= 1);

        // 回收之后: 初代快照（既不在 anchor、也不再被读者持有）必须归零
        assert!(
            initial_weak.upgrade().is_none(),
            "drain + prune 之后初代快照的强引用必须归零 —— 否则就是泄漏"
        );
        // 读者当前持有的那个仍必须活着（它自己拿着强引用）
        assert!(reader.held().is_some());
    }

    /// 判据 (a) 的第二面：**写者侧**待回收清单也必须归零（否则 `publish` 会无限泄漏）。
    #[test]
    fn writer_pending_list_is_pruned_after_reader_progress() {
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        let (producer, mut queue) = retire_channel(64);
        let mut reader = SnapshotReader::attach(&slot, producer);

        for revision in 1..=5u64 {
            slot.publish(snapshot_with(128, revision));
        }
        assert_eq!(slot.pending_len(), 5, "5 次 publish 淘汰了 5 个快照");

        // 在读者推进之前, 一项都不许释放
        assert_eq!(slot.prune(), 0, "读者还没确认, 写者不能释放任何一项");

        // 读者跑两个块边界
        let _ = reader.begin_block();
        reader.end_block();
        let _ = reader.begin_block();
        reader.end_block();

        let freed = slot.prune();
        assert!(
            freed >= 4,
            "读者推进后写者应释放绝大多数待回收项, 实际 {freed}"
        );
        assert!(slot.pending_len() <= 1, "残余至多一项(最新的那次淘汰)");

        // 读者 push 进队列的那个旧快照也必须被主线程回收掉
        let mut reclaimed = 0;
        for _ in 0..8 {
            reclaimed += queue.drain(8);
            slot.prune();
        }
        assert!(reclaimed > 0, "退役队列里的条目必须被主线程取走并 Drop");
        assert_eq!(slot.pending_len(), 0, "写者侧清单最终必须清空");
    }

    /// 读者退场后，写者可以立即释放全部待回收项（否则关流时会泄漏）。
    #[test]
    fn dropping_the_reader_releases_writer_side_backlog() {
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        let (producer, _queue) = retire_channel(64);
        {
            let mut reader = SnapshotReader::attach(&slot, producer);
            for revision in 1..=3u64 {
                slot.publish(snapshot_with(128, revision));
            }
            // 读者还**没有**跑完任何块 ⇒ reader_done == 0 ⇒ 写者一项都不许释放。
            assert_eq!(slot.pending_len(), 3);
            assert_eq!(slot.prune(), 0, "读者在册且未完成任何块时不许释放");

            // 跑一个块：读者现在持有最新快照, 但这个测试关心的是"退场后清空"
            let _ = reader.begin_block();
            reader.end_block();
            assert!(slot.pending_len() >= 1);
        }
        // 读者 Drop ⇒ reader_attached = false ⇒ 所有待回收项立即可以释放
        let before = slot.pending_len();
        assert!(before >= 1, "退场时应当还有待回收项可释放, 实际 {before}");
        assert_eq!(slot.prune(), before);
        assert_eq!(slot.pending_len(), 0, "读者退场 ⇒ 全部待回收项可释放");
    }

    /// 快照是真正的"不可变 + 可共享"：多个 `Arc` 克隆看到同一份数据，
    /// 且跨线程发送是合法的（`Send + Sync` 编译期断言）。
    #[test]
    fn snapshot_is_immutable_shareable_and_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        fn assert_send<T: Send>() {}
        assert_send_sync::<EngineSnapshot>();
        assert_send_sync::<SnapshotSlot>();
        assert_send_sync::<Arc<EngineSnapshot>>();
        // 读者必须能 move 进 cpal 的回调闭包（`D: Send + 'static`）——
        // 这正是"裸指针字段会让它变成 !Send"那个坑的编译期判据。
        assert_send::<SnapshotReader>();

        let snapshot = Arc::new(snapshot_with(128, 3));
        let clone = Arc::clone(&snapshot);
        let handle = std::thread::spawn(move || clone.revision());
        assert_eq!(handle.join().expect("线程不 panic"), 3);
        assert_eq!(snapshot.revision(), 3);
        // 只有 getter、没有 setter：字段全私有，编译期即不可变。
        assert_eq!(snapshot.channels(), 2);
    }

    /// 判据 (i)：**延迟的唯一来源是模型的 `DeviceDefinition::latency_samples`**
    /// [ARCH-PDC-001]，`yeban-engine` 不再自造第二个事实源。
    ///
    /// 构造两条并联支路：`heavy` 的设备链有 32 采样延迟，`light` 没有；
    /// PDC 必须把 `light` 补 32 采样，`heavy` 补 0。
    #[test]
    fn pdc_reads_device_latency_samples_from_the_model() {
        use yeban_model::{DeviceDefinition, DeviceKind};

        let heavy = EntityId::new();
        let light = EntityId::new();
        let master = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![heavy, light, master],
            ..RoutingGraph::default()
        };
        for (source, destination) in [(heavy, master), (light, master)] {
            let id = EntityId::new();
            routing.edges.insert(
                id,
                RoutingEdge {
                    id,
                    source_node: source,
                    destination_node: destination,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            );
        }
        let mut tracks = BTreeMap::new();
        tracks.insert(
            heavy,
            TrackV3 {
                id: heavy,
                devices: vec![DeviceDefinition {
                    kind: DeviceKind::ExternalEffect,
                    latency_samples: 32,
                    ..DeviceDefinition::default()
                }],
                ..TrackV3::default()
            },
        );
        tracks.insert(
            light,
            TrackV3 {
                id: light,
                ..TrackV3::default()
            },
        );
        let project = YebanProjectV1 {
            master_bus_track_id: master,
            routing_graph: routing,
            tracks,
            ..YebanProjectV1::default()
        };

        let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
        assert_eq!(
            snapshot.pdc().total_latency(),
            32,
            "L_max 由 32 采样的设备决定"
        );
        assert_eq!(snapshot.pdc().compensation(&heavy), Some(0));
        assert_eq!(snapshot.pdc().compensation(&light), Some(32));
        // 投影出来的轨道参数也带上该延迟（供实时侧做诊断/UI）
        assert_eq!(
            snapshot.track(&heavy).map(|p| p.latency_samples()),
            Some(32)
        );
        assert_eq!(snapshot.track(&light).map(|p| p.latency_samples()), Some(0));
    }

    /// 旁通设备的延迟**不计入**（它不在信号路径上）。
    #[test]
    fn bypassed_devices_do_not_contribute_latency() {
        use yeban_model::{DeviceDefinition, DeviceKind};

        let track = EntityId::new();
        let master = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![track, master],
            ..RoutingGraph::default()
        };
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        let mut tracks = BTreeMap::new();
        tracks.insert(
            track,
            TrackV3 {
                id: track,
                devices: vec![
                    DeviceDefinition {
                        kind: DeviceKind::ExternalEffect,
                        latency_samples: 64,
                        bypassed: true,
                        ..DeviceDefinition::default()
                    },
                    DeviceDefinition {
                        kind: DeviceKind::InternalEffect,
                        latency_samples: 8,
                        ..DeviceDefinition::default()
                    },
                ],
                ..TrackV3::default()
            },
        );
        let project = YebanProjectV1 {
            master_bus_track_id: master,
            routing_graph: routing,
            tracks,
            ..YebanProjectV1::default()
        };
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影成功");
        assert_eq!(
            snapshot.pdc().total_latency(),
            8,
            "只有未旁通的那个设备贡献延迟"
        );
    }

    #[test]
    fn block_frames_that_are_not_a_spec_enum_value_are_reported() {
        let snapshot = snapshot_with(200, 0);
        assert!(!snapshot.block_size_matches_enum());
        let ok = snapshot_with(64, 0);
        assert!(ok.block_size_matches_enum());
    }

    /// [MUST-GATE-012] 的**释放线程**记账：非空 `drain` 钉住释放线程，
    /// 换一个线程出队必须被记成 `foreign_drains`（判据据此变红）。
    #[test]
    fn drain_records_the_release_thread_and_flags_foreign_drains() {
        let (empty_producer, mut queue) = retire_channel(8);
        assert_eq!(queue.release_thread(), None, "还没回收过就没有释放线程");
        assert_eq!(queue.drain(8), 0, "空队列不算释放");
        assert_eq!(queue.release_thread(), None, "空 drain 不许钉住释放线程");
        drop(empty_producer);
        drop(queue);

        let (mut producer, mut queue) = retire_channel(8);
        producer
            .push(Arc::new(snapshot_with(128, 7)))
            .expect("有空间");
        drop(producer);

        let main_thread = std::thread::current().id();
        assert_eq!(queue.drain(8), 1);
        assert_eq!(queue.release_thread(), Some(main_thread));
        assert_eq!(queue.foreign_drains(), 0, "同一个线程出队不算 foreign");

        // 换一个线程出队: 它必须被记一次 foreign（而不是 panic / 静默）。
        let foreign = std::thread::spawn(move || {
            let (mut producer, mut queue) = retire_channel(8);
            producer
                .push(Arc::new(snapshot_with(128, 8)))
                .expect("有空间");
            drop(producer);
            assert_eq!(queue.release_thread(), None);
            let released = queue.drain(8);
            (released, queue.release_thread())
        })
        .join()
        .expect("线程不得 panic");
        assert_eq!(foreign.0, 1);
        assert_ne!(
            foreign.1,
            Some(main_thread),
            "释放线程必须是那个**真的**执行了 Drop 的线程"
        );

        // 同一队列上先主线程、后另一个线程 ⇒ foreign_drains 必须 +1。
        let (producer, mut queue) = retire_channel(8);
        let mut producer = producer;
        producer
            .push(Arc::new(snapshot_with(128, 9)))
            .expect("有空间");
        assert_eq!(queue.drain(8), 1);
        assert_eq!(queue.release_thread(), Some(main_thread));
        producer
            .push(Arc::new(snapshot_with(128, 10)))
            .expect("有空间");
        std::thread::scope(|scope| {
            let queue = &mut queue;
            scope.spawn(move || {
                assert_eq!(queue.drain(8), 1, "第二个线程真的回收了一条");
            });
        });
        assert_eq!(queue.foreign_drains(), 1, "换线程出队必须被记账");
        assert_eq!(queue.release_thread(), Some(main_thread), "释放线程不回退");
    }

    /// [MUST-GATE-012] 的**零泄漏对账**仪器自检：`release_probe::total()` 必须
    /// 与"快照分配被真正释放"一一对应，且**逐线程**计数只归属到跑 `drop` 的那个线程。
    #[test]
    fn release_probe_counts_every_snapshot_drop_on_the_dropping_thread() {
        release_probe::reset_current_thread();
        release_probe::watch_current_thread();
        let before_total = release_probe::total();
        let before_here = release_probe::released_by_current_thread();

        let snapshot = Arc::new(snapshot_with(128, 11));
        // 克隆不增加释放数（同一个分配），最后一个强引用归零才记账。
        drop(Arc::clone(&snapshot));
        assert_eq!(
            release_probe::released_by_current_thread(),
            before_here,
            "还有强引用时不得记账"
        );
        drop(snapshot);
        assert_eq!(
            release_probe::released_by_current_thread(),
            before_here + 1,
            "最后一个强引用归零 ⇒ 记账一次"
        );
        // `total()` 是**全局**量：libtest 会并行跑别的测试，它们也在丢快照
        // ⇒ 这里只能断言"至少涨了 1"。**精确对账**在 `harness = false` 的
        // `tests/snapshot_retire_churn.rs` 里做（那个进程只有本判据的线程）。
        assert!(
            release_probe::total() > before_total,
            "全局释放数必须跟着涨"
        );
        release_probe::unwatch_current_thread();

        // 窗口关掉之后不得再计错线程（本线程的计数不动）。
        drop(snapshot_with(128, 12));
        assert_eq!(
            release_probe::released_by_current_thread(),
            before_here + 1,
            "关掉窗口后本线程的计数不许继续涨"
        );

        // 别的线程上的释放必须记在**那个**线程的账上，而不是本线程。
        let (total_delta, there) = std::thread::spawn(|| {
            release_probe::watch_current_thread();
            let before = release_probe::total();
            drop(snapshot_with(128, 13));
            (
                release_probe::total() - before,
                release_probe::released_by_current_thread(),
            )
        })
        .join()
        .expect("线程不得 panic");
        assert!(total_delta >= 1, "全局释放数必须跟着涨");
        assert_eq!(there, 1, "释放记在了跑 drop 的那个线程上");
        assert_eq!(
            release_probe::released_by_current_thread(),
            before_here + 1,
            "别的线程的释放不许污染本线程的计数（否则判据会把音频线程的 0 读错）"
        );
    }

    // -----------------------------------------------------------------------
    // 跨线程退役记账（`needs` N2/N5）：判据⑥ 的"大数行为"在这里断言
    // -----------------------------------------------------------------------

    /// 判据⑥（计数不饱和/不溢出）：累计量在 `u64::MAX` 附近必须**饱和**，不得回绕。
    ///
    /// "回绕成 0"会把一条健康读数变成**谎报**（"从没释放过"）—— 这正是
    /// `saturating_bump` 存在的理由。两条断言：① 饱和加法本身；② 队列真的 drain 一次
    /// 之后累计量停在 `u64::MAX`（用私有字段预置大数，这是**测试专用**的注入点）。
    #[test]
    fn retire_counters_saturate_instead_of_wrapping() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(saturating_bump(&counter, 5), u64::MAX);
        assert_eq!(
            saturating_bump(&counter, 5),
            u64::MAX,
            "已饱和 ⇒ 仍然是 MAX"
        );
        assert_eq!(counter.load(Ordering::Acquire), u64::MAX);
        assert_eq!(saturating_bump(&counter, 0), u64::MAX, "0 增量是纯读");

        // 队列级：预置成 `u64::MAX - 1`，再真的回收 2 条 ⇒ 停在 `u64::MAX`（不是 1）。
        let (mut producer, mut queue) = retire_channel(4);
        producer
            .push(Arc::new(snapshot_with(128, 1)))
            .expect("容量 4 有空间");
        producer
            .push(Arc::new(snapshot_with(128, 2)))
            .expect("容量 4 有空间");
        queue
            .accounting
            .drained
            .store(u64::MAX - 1, Ordering::Release);
        queue
            .accounting
            .drain_calls
            .store(u64::MAX - 1, Ordering::Release);
        assert_eq!(queue.drain(4), 2);
        assert_eq!(queue.dropped(), u64::MAX, "饱和，而不是回绕成 1");
        assert_eq!(queue.drain_calls(), u64::MAX);
        assert_eq!(queue.foreign_drains(), 0);
        // 再排一次空队列 ⇒ 非空 drain 计数不动（结构性判据不受空转影响）。
        assert_eq!(queue.drain(4), 0);
        assert_eq!(queue.drain_calls(), u64::MAX);
    }

    /// 判据⑥ 的另一半：写者侧 `prune` 的累计量同样饱和（它走的是同一条饱和加法）。
    #[test]
    fn slot_pruned_counter_saturates_instead_of_wrapping() {
        let slot = SnapshotSlot::new(snapshot_with(128, 0));
        slot.publish(snapshot_with(128, 1));
        slot.publish(snapshot_with(128, 2));
        assert_eq!(slot.pending_len(), 2, "没有在册读者 ⇒ 两项都可回收");
        slot.pruned.store(u64::MAX - 1, Ordering::Release);
        assert_eq!(slot.prune(), 2);
        assert_eq!(slot.pruned(), u64::MAX, "饱和，而不是回绕成 1");
        assert_eq!(slot.prune(), 0);
        assert_eq!(slot.pruned(), u64::MAX);
    }

    /// 判据⑥ 的"量规不回绕"：`pending` 是跨线程镜像，但它是
    /// **两个单调量之差**（`pushed − drained`）而不是一个被覆写的量规
    /// ⇒ 它既不会回绕，也**没有**能丢掉一次 `+1` 的窗口（见 `RetireAccounting` 文档）。
    ///
    /// ⚠ 这条测试是**改口径**过的：第一版叫
    /// `retire_pending_mirror_is_overwritten_by_drain_and_self_heals`，断言
    /// "人工把镜像写成 999 ⇒ 下一次 drain 用真实剩余盖掉"。那个"自愈"正是 CI 变红的机制
    /// （覆写与音频线程的 `+1` 不原子 ⇒ 永久少记一条）；现在改成断言**按构造成立的不变量**。
    #[test]
    fn retire_pending_mirror_is_the_difference_of_two_monotonic_counters() {
        let (mut producer, mut queue) = retire_channel(8);
        producer
            .push(Arc::new(snapshot_with(128, 1)))
            .expect("容量 8 有空间");
        assert_eq!(queue.accounting().pending(), 1, "push 成功 ⇒ 入队数 +1");
        assert_eq!(queue.accounting().pushed(), 1);
        assert_eq!(queue.accounting().drained(), 0);
        assert_eq!(queue.drain(8), 1);
        assert_eq!(queue.accounting().pending(), 0, "出队 1 ⇒ 差值为 0");
        assert_eq!(queue.accounting().pushed(), 1);
        assert_eq!(queue.accounting().drained(), 1);
        assert_eq!(queue.pending(), 0);
        assert_eq!(queue.accounting().drain_calls(), 1);

        // 一个"入队数不动、出队数被人工抬高"的越界状态只会把量规夹到 0（饱和减法），
        // 绝不会**回绕成 u64::MAX** —— 控制面不会读到一个天文数字的积压。
        queue.accounting.drained.store(u64::MAX, Ordering::Release);
        assert_eq!(queue.accounting().pending(), 0, "饱和减法，不回绕");
    }

    /// **见证（判据 ③ 的库内版本）**：`drain` 与 `push` **真并发**之后，
    /// 每个**静止点**上 `RetireAccounting::pending()` 必须与精确读数
    /// （`Consumer::slots()`）**逐项相等**。
    ///
    /// # 为什么这条是"注入 ⇒ 变红"的宿主
    ///
    /// 它每个 epoch 取一个静止点样本（本机实跑 `EPOCHS` 个），所以"镜像少记/多记一次"
    /// 这类注入**必定**在某个样本上暴露。判据本身**不含任何容差**：静止点上必须是等号。
    ///
    /// # 静止点协议（每个 epoch）—— 用**代际回执**，不用"睡一会儿"
    ///
    /// ```text
    ///   主线程（drain 侧）                            生产者线程（push 侧）
    ///   ─────────────────────────────────────────     ────────────────────────────────
    ///   ① 清掉上一轮尾巴（此刻生产者已回执 ⇒ 静止）
    ///   ② base_pushed = pushed；base_ack = ack
    ///   ③ cmd = PUSH
    ///                                                 ④ 连续 push，直到 cmd != PUSH
    ///   ⑤ 等 pushed > base_pushed（本轮真推了）
    ///   ⑥ queue.drain(全部)        ← 与 ④ 真并发
    ///   ⑦ cmd = PARK
    ///                                                 ⑧ 退出 push 循环，ack += 1（Release）
    ///   ⑨ 等 ack > base_ack（Acquire）⇒ ⑧ 之后不可能再有 push
    ///   ⑩ 静止点：pending() == consumer.slots()
    /// ```
    ///
    /// 为什么必须是**代际**回执（`ack` 只增不减）而不是一个布尔"停/走"状态：
    /// 第一版把"停下"与"收工"写成**同一个**状态值 ⇒ 生产者在每个 epoch 结束时都可能
    /// 直接**退出线程**，下一轮的"等环里有货"就会空转到断言红（本机实测：
    /// 带负载连跑 14 次复现，`snapshot.rs:2139:17: 生产者没有开始 push`）。
    /// 那是**判据自己的状态机**缺陷，不是被测对象的漂移 —— 修法是**加回执世代**，
    /// 不是把断言放宽。
    ///
    /// ⑧→⑨ 的 Release/Acquire 把"生产者已经停下"变成**可判定**的事实：Acquire 载入
    /// `ack` 与生产者的 Release 存储同步 ⇒ 生产者在 ⑧ 之前的**全部** `+1` 都对本线程可见。
    #[test]
    fn pending_mirror_stays_exact_at_every_quiescent_point_while_drain_races_push() {
        use std::sync::atomic::{AtomicU8, AtomicU64};

        const EPOCHS: usize = 400;
        const PUSH: u8 = 1;
        const PARK: u8 = 2;
        const QUIT: u8 = 3;

        let payload = Arc::new(snapshot_with(128, 7));
        let (mut producer, mut queue) = retire_channel(1024);
        let accounting = Arc::clone(queue.accounting());
        let cmd = Arc::new(AtomicU8::new(PARK));
        let ack = Arc::new(AtomicU64::new(0));

        let producer_thread = {
            let cmd = Arc::clone(&cmd);
            let ack = Arc::clone(&ack);
            let payload = Arc::clone(&payload);
            std::thread::spawn(move || -> u64 {
                let mut pushed = 0u64;
                loop {
                    // 等命令：**只有 QUIT 才退出**；PARK 只是"回到等待位"（不是收工）。
                    loop {
                        match cmd.load(Ordering::Acquire) {
                            PUSH => break,
                            QUIT => {
                                ack.fetch_add(1, Ordering::Release);
                                return pushed;
                            }
                            _ => std::hint::spin_loop(),
                        }
                    }
                    // 连续 push，直到主线程把命令改成 PARK（或 QUIT）。
                    while cmd.load(Ordering::Acquire) == PUSH {
                        if producer.push(Arc::clone(&payload)).is_ok() {
                            pushed += 1;
                        }
                    }
                    // 回执：主线程据此判定"此刻没有在飞的 push"。
                    ack.fetch_add(1, Ordering::Release);
                }
            })
        };

        let mut samples = 0usize;
        let mut non_zero_samples = 0usize;
        let mut max_exact = 0usize;
        let mut base_ack = 0u64;
        for _ in 0..EPOCHS {
            // ① 上一轮已回执 ⇒ 生产者停在等待位；清掉上一轮的尾巴，
            //    让本轮的 push 一定推得进环（否则"等 pushed 增长"会假红）。
            queue.drain(usize::MAX);
            let base_pushed = accounting.pushed();
            // ②③ 发令：生产者开始连续 push。
            cmd.store(PUSH, Ordering::Release);
            // ⑤ 等**本代**真的发生了一次入队（否则这次 drain 没有并发可言，见证是空话）。
            let mut spins = 0u64;
            while accounting.pushed() == base_pushed {
                std::hint::spin_loop();
                spins += 1;
                assert!(spins < 1_000_000_000, "生产者没有开始 push");
            }
            // ⑥ 与生产者**真并发**的一次排空（判据的被测对象就是这个交错）。
            queue.drain(usize::MAX);
            // ⑦⑨ 令其停下，并等到**本代**的回执（`ack > base_ack`）。
            cmd.store(PARK, Ordering::Release);
            let mut spins = 0u64;
            while ack.load(Ordering::Acquire) <= base_ack {
                std::hint::spin_loop();
                spins += 1;
                assert!(spins < 1_000_000_000, "生产者没有回执静止");
            }
            base_ack = ack.load(Ordering::Acquire);
            // ⑩ 静止点。
            let exact = queue.pending();
            let mirrored = accounting.pending();
            samples += 1;
            if exact > 0 {
                non_zero_samples += 1;
            }
            max_exact = max_exact.max(exact);
            assert_eq!(
                mirrored, exact as u64,
                "静止点上镜像必须与精确读数相等（第 {samples} 个样本）"
            );
            // 结构等式：入队 − 出队 == 精确占用。
            assert_eq!(
                accounting.pushed() - accounting.drained(),
                exact as u64,
                "静止点上 pushed − drained 必须等于精确占用（第 {samples} 个样本）"
            );
        }
        // 收工：QUIT 是**唯一**能让生产者退出的命令；join 之后再验一次静止点。
        cmd.store(QUIT, Ordering::Release);
        let pushed_total = producer_thread.join().expect("生产者线程不该 panic");
        assert!(pushed_total > 0, "生产者一条都没推进去 ⇒ 见证是空的");
        assert_eq!(samples, EPOCHS, "样本数必须真的是 {EPOCHS} 个");
        assert!(
            non_zero_samples > 0,
            "所有样本的精确读数都是 0 ⇒ 比较的是两个 0（假绿）"
        );
        assert!(max_exact > 0, "精确读数从来没到过 0 以上");
        assert_eq!(accounting.pending(), queue.pending() as u64, "收尾静止点");
        assert!(accounting.pushed() >= pushed_total);
        assert!(accounting.drained() > 0, "一次都没出队 ⇒ 并发交错没被覆盖");
    }
}
