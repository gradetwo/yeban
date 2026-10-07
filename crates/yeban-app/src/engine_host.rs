//! **引擎宿主**：把 `YebanProjectV1` 变成一个真的在跑的 `EngineRuntime` + 一条新的电平队列
//! （`ui/reload_engine` 的落地；`[ARCH-RT-002]` / `[ROAD-M2-002]` / `[ARCH-UI-002]`）。
//!
//! ## 规范来源 (Normative)
//!
//! - `[ARCH-RT-002]` / `[ROAD-M2-002]`：`EngineSnapshot` 投影 + `SnapshotSlot` 原子交换 +
//!   退役回收队列（旧快照由**非实时线程**释放）。
//! - `[ARCH-UI-002]`：电平经独立 SPSC 解耦 —— 本模块建队列、把**消费端**交给 UI 线程
//!   （[`crate::meters::MeterRuntime`]），生产端留在引擎运行时里。
//! - `[ARCH-TOP-002]`：线程拓扑。**实时线程**跑 `EngineRuntime::process_quantum`；
//!   本模块的 `reload` 是**控制面/测试线程**的动作（允许分配、允许建队列）。
//! - ADR-0001 **D19**：本文件只用 `yeban-engine` 与设备无关的公开面
//!   （`snapshot` / `rt` / `ring` / `meter`），**一个 `cpal::*` 都不出现**；
//!   设备宿主（`device` feature）仍是引擎侧的事。
//!
//! ## 为什么这不是"假装重建"
//!
//! `reload` 做的是**真的**四件事，每一步都有可观测的后果：
//!
//! 1. `EngineSnapshot::from_project(project, revision)` —— 真的把工程投影成引擎快照
//!    （失败就**报错**，例如工程没有主总线 ⇒ `SnapshotError::NoMasterBus`）；
//! 2. `SnapshotSlot::new(snapshot)` + `retire_channel` + `event_channel` + `meter_channel`
//!    —— 四样东西全是**新**的，旧的一代整体丢弃（不是"改改参数继续用"）；
//! 3. `EngineRuntime::new(...)` 后**真的推 `quanta` 个量子**：
//!    `meter_bulk_publishes == quanta`、`meter_frames == quanta × (非母线轨数 + 1)`
//!    （这两个数是引擎侧的结构性契约，见 `docs/ledger/engine-meters-notes.md` §2）；
//! 4. 新队列的**消费端**交还给 UI 线程 ⇒ 电平面板被清空后重新收到引擎发布的新帧
//!    （界面上的可观测副作用：混音台电平条回到下限）。
//!
//! ## 诚实边界（不要误读）
//!
//! - **引擎仍然不发声**：轨道渲染是占位静音，因此引擎实际发布的电平恒为静音
//!   （`docs/ledger/engine-meters-notes.md` §0.1）。"重建是真的"指的是快照/队列/量子
//!   驱动是真的，而不是"有声音了"。
//! - **没有开声卡**：本切片不调用 `yeban_engine::device`（红线 6 与 D19 都要求设备 I/O
//!   单独裁决）。`process_quantum` 由控制面/测试线程显式驱动，因此在"设备回调"这条路上
//!   它是**同一个函数**（`EngineRuntime::process_quantum` 就是 cpal 回调会调的那个），
//!   但"由 cpal 回调驱动"这件事本身仍未接线（登记在 notes 的 pending）。
//!
//! ## 编辑 ⇒ 发声：增量发布 ＋ 退役回收心跳
//!
//! `reload` **整体换代**（新槽 / 新队列 / 新 `EngineRuntime`，并推 `quanta` 个量子），
//! 因此它只适合 `ui/reload_engine` 那种"重建"动作：工程**改了一个参数**之后要换的只是
//! **快照**，用 `reload` 会连带改掉 `quanta` 契约。编辑路径因此走另一条：
//!
//! | 动作 | 谁做 | 何时 | 线程 |
//! | :--- | :--- | :--- | :--- |
//! | 增量发布（[`EngineHost::publish_project`]） | 控制线程 | 工程改动的那一跳（[`EditMark`] 变了） | UI / 主线程 |
//! | 写者侧回收（`SnapshotSlot::prune`） | [`EngineHost::heartbeat`] | 每跳（60Hz） | UI / 主线程 |
//! | 退役队列排空（`RetireQueue::drain`） | [`EngineHost::heartbeat`] | 每跳（60Hz） | UI / 主线程 |
//!
//! 生产接线是 `src/main.rs` 的 `run_gui`：一个 `slint::Timer`（`TimerMode::Repeated`，
//! 16 ms ⇒ 62.5 Hz）在**创建该槽的那一个线程**上跑"按需发布 + 回收"。窗口关闭 ⇒
//! `run_gui` 返回 ⇒ 定时器被 drop（停止）；控制面会话被关掉 ⇒ 权威工程读不到 ⇒
//! 发布自然停止，回收心跳继续跑到窗口关闭为止。
//!
//! ⚠ **没有留下"只发布、不回收"的路径**：发布（[`EngineHost::publish_project`]）与回收
//! （[`EngineHost::heartbeat`]）是**同一个调用序列**里的相邻两步，并且两者都在
//! **非实时线程**上 —— 音频回调那条读路径（`SnapshotReader::begin_block`）一位没动
//! [ARCH-RT-001 / MUST-GATE-001]。

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::{DEFAULT_METER_CAPACITY, MeterCollector, meter_channel};
use yeban_engine::ring::{EngineEvent, EventSender, TransportCommand, event_channel};
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{
    EngineSnapshot, RetireQueue, SnapshotError, SnapshotSlot, retire_channel,
};
use yeban_engine::transport::{TransportMirror, TransportReading, TransportState};
use yeban_model::EntityId;
use yeban_model::project::YebanProjectV1;

/// 退役队列容量（条）。
///
/// `reload` 同一次内最多发生一次快照交换；增量发布每一跳**至多**一次（[`EditMark`]
/// 变了才发），而心跳每一跳都会排空队列 ⇒ 32 条是 32 倍余量。真的排不上时读者会走
/// `stash`（寄存）并记 `stash_events`，**不会**在音频线程释放任何东西 [ARCH-RT-002]。
const RETIRE_CAPACITY: usize = 32;

/// UI → 引擎事件通道容量（条）。通道必须**在打开设备之前**建立
/// （`EngineRuntime::new` 的文档要求），所以它在这里就位。
///
/// 走带命令经这条通道进引擎（[`EngineHost::send_transport`]）：它是**既有的**
/// 无锁 SPSC 批量通道（`[ARCH-RT-001]` / `[ROAD-M2-007]`），不是为走带新造的第二条。
/// 容量 256 远超实际（一次走带动作只发 1–2 条），所以通道**永不成为瓶颈**。
const EVENT_CAPACITY: usize = 256;

/// 引擎重建失败的原因。
#[derive(Debug)]
pub enum EngineHostError {
    /// 工程无法投影成引擎快照（没有主总线 / 路由图成环 / 模型校验失败）。
    Snapshot(SnapshotError),
    /// 还没有一代引擎（`reload` 从未成功过）⇒ 没有槽可以发布。
    NoEngine,
}

impl core::fmt::Display for EngineHostError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "引擎快照投影失败: {error}"),
            Self::NoEngine => write!(formatter, "还没有一代引擎 (reload 从未成功过)"),
        }
    }
}

impl core::error::Error for EngineHostError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Snapshot(error) => Some(error),
            Self::NoEngine => None,
        }
    }
}

impl From<SnapshotError> for EngineHostError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

/// 工程改动的**轻量标记**：这一跳要不要重新发布快照。
///
/// 它取自撤销端口的**显示态**（[`crate::undo::UndoPort::display`] 的模型读数），只保留两个
/// 会随编辑变化的字段：
///
/// | 编辑 | 变化 |
/// | :--- | :--- |
/// | 一次提交（推子松手 / 静音 / 加音符 / 删音符 / AI 的工具调用） | `head` 换成新提交身份 |
/// | 一次撤销 / 重做 | `undone` 变，而 `head` **不变** |
///
/// 为什么不逐跳算 `UndoPort::fingerprint`：那是整份容器字节的 SHA-256，逐跳算它是浪费。
/// 标记只用来回答"要不要重新投影"，**不是**工程指纹 —— 指纹的唯一用途仍是判据的逐字节证据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditMark {
    /// 活跃分支头（一次提交 ⇒ 新身份）。
    pub head: Option<EntityId>,
    /// 已撤销步数（一次撤销 / 重做 ⇒ 变化）。
    pub undone: usize,
}

impl EditMark {
    /// 由两个字段构造。
    #[must_use]
    pub const fn new(head: Option<EntityId>, undone: usize) -> Self {
        Self { head, undone }
    }

    /// 从撤销端口的显示态取标记（**唯一**来源；不要另算一份）。
    #[must_use]
    pub fn from_display(display: &crate::undo::UndoDisplay) -> Self {
        Self::new(display.head, display.undone)
    }
}

/// 一次心跳（[`EngineHost::heartbeat`]）的**字面读数**。
///
/// 单位：`published` / `pruned` 是**累计条数**（快照数，单调不减），`pending_len` /
/// `released` / `retired` / `retire_pending` 是**条数**（`released` / `retired` 是**本次**
/// 释放 / 排空的条数，其余是**此刻**的长度）。它们全部来自
/// `yeban-engine` 的快照槽与退役队列，不是从工程推算的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeartbeatReadings {
    /// 累计发布次数（`SnapshotSlot::published()`）。
    pub published: u64,
    /// 心跳**之后**写者侧待回收清单的长度（`SnapshotSlot::pending_len()`）。
    pub pending_len: usize,
    /// 累计由 `SnapshotSlot::prune()` 释放的条数。
    pub pruned: u64,
    /// **本次** `prune` 释放的条数。
    pub released: usize,
    /// **本次** `drain` 真的 `Drop` 掉的旧快照条数。
    pub retired: usize,
    /// 心跳**之后**退役队列里还排着几条。
    pub retire_pending: usize,
}

/// 快照槽的计数读数（[`EngineHost::snapshot_counts`]，没有引擎时是 `None`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotCounts {
    /// 累计发布次数。
    pub published: u64,
    /// 写者侧待回收清单长度。
    pub pending_len: usize,
    /// 累计由 `prune` 释放的条数。
    pub pruned: u64,
    /// 纪元（每次发布 +1）。
    pub epoch: u64,
    /// 当前快照的版本号（`EngineSnapshot::revision`）。
    pub revision: u64,
    /// 退役队列里排着几条。
    pub retire_pending: usize,
    /// 累计真的 `Drop` 掉的旧快照条数。
    pub retire_dropped: u64,
}

/// 一次引擎重建的**读数**（判据与 `ui/reload_engine` 的回执都用它）。
///
/// 每个数字都来自 `EngineRuntime::stats()` 或快照本身，**没有一个是从工程里推算出来的**
/// ——"报告的数字"与"引擎实际做的事"必须是同一份事实。
///
/// 刻意**不** derive `Clone` / `PartialEq`：它是"一次动作的读数"，不是可以随便复制比较的值
/// 对象，而且它携带的 [`MeterCollector`] 是**所有权**（消费端只能被采纳一次）。
#[derive(Debug)]
pub struct EngineRebuild {
    /// 第几代引擎（1 起，单调递增）。
    pub generation: u64,
    /// 快照的模型层版本号（`EngineSnapshot::from_project` 的 `revision`）。
    pub revision: u64,
    /// 快照里的节点数（含主总线）。
    pub tracks: usize,
    /// 本次真的推了多少个量子。
    pub quanta: u64,
    /// 引擎累计的批量发布次数（**结构性契约**：应等于 `quanta`）。
    pub meter_bulk_publishes: u64,
    /// 引擎累计发布的电平帧数（应等于 `quanta × (非母线轨数 + 1)`）。
    pub meter_frames: u64,
    /// 容量耗尽导致未计量/被淘汰的次数（应为 0）。
    pub meter_capacity_drops: u64,
    /// 重建之后引擎上报的**走带读数**。
    ///
    /// `reload` **不改变**走带状态（见它的第 5 步说明）：新引擎沿用"自由跑"默认值
    /// ⇒ 这里通常是 `Playing` / tick 0。"加载即停住"由控制面显式调 [`EngineHost::stop`]。
    pub transport: TransportReading,
    /// 新引擎的电平队列**消费端** —— UI 线程必须采纳它（[`crate::meters::MeterRuntime::adopt`]）。
    pub collector: MeterCollector,
}

impl EngineRebuild {
    /// 非母线轨数（发布帧数的分母；快照里含主总线）。
    #[must_use]
    pub const fn non_bus_tracks(&self) -> usize {
        self.tracks.saturating_sub(1)
    }
}

/// 一次走带动作的**记录**（走带接线判据的注入点）。
///
/// 它存在的唯一理由是"控件树里有这个元素"**不能**当作"回调接线了"的证据
/// （`docs/ledger/feature-alignment.md` 错位 7 登记过这个误判）。记录里每一个字段
/// 都来自**引擎**：命令本身、命令应用之后的状态与位置、为了让它生效推了几个量子。
///
/// 它是控制线程的私有缓冲（允许分配），不进实时路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportActionRecord {
    /// 发出的命令（`Stop`/`Pause` 会展开成一条记录；`stop_and_rewind` 是两条）。
    pub command: TransportCommand,
    /// 命令**应用之后**引擎上报的状态。
    pub state_after: TransportState,
    /// 命令应用之后引擎上报的位置（960 PPQ tick）。
    pub position_ticks_after: u64,
    /// 为了让命令在量子边界生效而推进的量子数（0 = 没有推进）。
    pub quanta_pumped: u64,
}

/// 引擎宿主：持有"当前这一代"的全部引擎侧对象。
///
/// 字段全部私有 + 只有 [`EngineHost::reload`] 能换掉它们 —— "换引擎"这条路径只有一条，
/// 不存在"改了快照但没换队列"这类半更新的形态。
#[derive(Default)]
pub struct EngineHost {
    generation: u64,
    /// **快照版本号**（`EngineSnapshot::revision` 的唯一来源）：每次
    /// [`EngineHost::reload`] 或 [`EngineHost::publish_project`] `+1`。
    ///
    /// 它与 `generation`（换代计数）**不是**同一个量：增量发布不换代，因此版本号会比
    /// 代数跑得快。既有判据只约束"连续两次 `reload` 的 `revision` 是 1、2"，在没有发布的
    /// 情况下两者逐步相等。
    snapshot_revision: u64,
    slot: Option<std::sync::Arc<SnapshotSlot>>,
    runtime: Option<EngineRuntime>,
    retire: Option<RetireQueue>,
    /// 最近一次发布所依据的工程标记（[`EngineHost::should_publish`] 的比较基准）。
    ///
    /// `None` = 还没有发布过（或刚 `reload` 过 ⇒ 由调用方用
    /// [`EngineHost::mark_applied`] 记下"当前快照已经反映哪个标记"）。
    last_mark: Option<EditMark>,
    /// UI → 引擎的**唯一**命令生产端（既有无锁 SPSC）。
    ///
    /// 它是 `Option` 只因为 `EngineHost::default()` 必须先存在（还没有引擎时没有通道）；
    /// 一旦 [`EngineHost::reload`] 成功，它就一定在（"有引擎必有通道"）。
    events: Option<EventSender>,
    /// RT → UI 的走带读数镜面（与 `EngineRuntime` 里那一份是**同一个** `Arc`）。
    transport_mirror: Option<std::sync::Arc<TransportMirror>>,
    /// 走带动作日志（控制线程私有；判据的注入点，见 [`TransportActionRecord`]）。
    journal: Vec<TransportActionRecord>,
}

/// 手写 `Debug`：`EngineRuntime` / `SnapshotSlot` 刻意没有 `Debug`（它们持有裸指针与
/// 互斥量，打印它们只会误导），因此这里只报告"第几代、活没活着"。
impl core::fmt::Debug for EngineHost {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EngineHost")
            .field("generation", &self.generation)
            .field("has_engine", &self.has_engine())
            .finish_non_exhaustive()
    }
}

impl EngineHost {
    /// 还没有引擎的宿主。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前是第几代（`0` = 从未成功重建过）。
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// 是否有一代活的引擎。
    #[must_use]
    pub const fn has_engine(&self) -> bool {
        self.runtime.is_some()
    }

    /// **重建引擎**：投影新快照 → 建新队列 → 推 `quanta` 个量子 → 交还消费端。
    ///
    /// `quanta = 0` 是合法的（只重建、不推进）；但**失败必须被上报**：快照投影失败时
    /// 旧的一代**原样保留**（不做半更新），调用方拿到 `Err` 后界面上的引擎仍是上一代 ——
    /// 这比"清空成空引擎"诚实得多。
    ///
    /// # Errors
    ///
    /// [`EngineHostError::Snapshot`]：工程没有主总线 / 路由图成环 / 模型校验失败。
    pub fn reload(
        &mut self,
        project: &YebanProjectV1,
        quanta: u64,
    ) -> Result<EngineRebuild, EngineHostError> {
        let generation = self.generation.saturating_add(1);
        let revision = self.snapshot_revision.saturating_add(1);
        // 第 1 步：真的投影快照（失败在这里就返回，旧的一代不动）。
        let snapshot = EngineSnapshot::from_project(project, revision)?;
        let tracks = snapshot.tracks().len();
        let channels = snapshot.channels().max(1);

        // 第 2 步：四样东西全是新的。事件**生产端**保留下来（走带命令从这里进引擎）。
        let slot = SnapshotSlot::new(snapshot);
        let (retire_producer, retire) = retire_channel(RETIRE_CAPACITY);
        let (events, event_receiver) = event_channel(EVENT_CAPACITY);
        let (publisher, collector) = meter_channel(DEFAULT_METER_CAPACITY);
        let mut runtime = EngineRuntime::new(&slot, retire_producer, event_receiver, publisher);
        let mirror = std::sync::Arc::clone(runtime.transport_mirror());

        // 第 3 步：真的推量子。
        //
        // ⚠ **输出缓冲必须恰好一个"运行时量子"**（`[ARCH-DET-001]` 的 `DEFAULT_BLOCK_FRAMES`
        // = 128 帧 × 声道数）。`EngineRuntime::process_quantum` 会把**任意长度**的缓冲按
        // `DEFAULT_BLOCK_FRAMES` 切片，每片一个量子（每片一次批量发布）——
        // 而快照里 `ProjectAudioConfig::block_size` 是**模型层的声明值**（演示工程 = 256），
        // 两者是两个概念。第一版按"快照声明的块长"分配缓冲 ⇒ 一次调用推进了 **2** 个量子
        // （实测 `meter_bulk_publishes = 2 × quanta`），本机探针当场抓到。
        // 详见 `docs/ledger/app-mixer-notes.md` §6 的实测记录与 needs。
        let mut output = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)];
        for _ in 0..quanta {
            runtime.process_quantum(&mut output, channels);
        }
        let stats = runtime.stats();

        // 第 4 步：装机（旧的一代在这里整体被替换；它持有的快照由 `SnapshotReader`
        // 的退役队列回收 —— 本线程内不会有积压，因此 `drain` 一次即可）。
        self.slot = Some(slot);
        self.runtime = Some(runtime);
        self.retire = Some(retire);
        self.events = Some(events);
        self.transport_mirror = Some(mirror);
        self.generation = generation;
        self.snapshot_revision = revision;
        // 新快照**就是** `project` 的投影 ⇒ "已经反映的标记"未知，由调用方用
        // [`EngineHost::mark_applied`] 记下（生产路径与判据都这么做）；在此之前
        // [`EngineHost::should_publish`] 恒为 `true`（宁多发一次，不漏发）。
        self.last_mark = None;
        self.drain_retired();

        // ⚠ 第 5 步**刻意不做**：`reload` 不碰走带状态。
        //
        // 为什么不在这里顺手发一条 `Stop`（本线第一版就是这么做的，被既有判据打红）：
        // `ui/reload_engine` 的契约是"推 `engine_quanta` 个量子"，而让命令在量子边界
        // 生效必须**再推一个量子**（命令是在 `render_block` 第 1 步出队的）——
        // 那会让报告里的 `quanta` 比调用方要的多 1（实测红在
        // `crates/yeban-app/tests/live_ui_mcp.rs` 的 `admin_reload_engine_...`）。
        // 因此"加载即停住"是**控制面的显式动作**：GUI 路径在 `reload` 之后调
        // [`EngineHost::stop`]，读回引擎读数再注入界面（见 `src/main.rs`）。
        let transport = self.transport();

        let rebuild = EngineRebuild {
            generation: self.generation,
            revision,
            tracks,
            quanta: stats.quanta,
            meter_bulk_publishes: stats.meter_bulk_publishes,
            meter_frames: stats.meter_frames,
            meter_capacity_drops: stats.meter_capacity_drops,
            transport,
            collector,
        };
        debug_assert_eq!(
            rebuild.meter_bulk_publishes, rebuild.quanta,
            "引擎的结构性契约: 每量子恰好一次电平批量发布 [ROAD-M2-007]"
        );
        Ok(rebuild)
    }

    // -----------------------------------------------------------------------
    // 编辑 ⇒ 发声（增量发布 + 退役回收心跳）
    // -----------------------------------------------------------------------

    /// **增量发布**：把 `project` 投影成一份新快照，经 `SnapshotSlot::publish_arc`
    /// 原子换掉当前快照。
    ///
    /// 这是"编辑 ⇒ 发声"链路的发布点。它与 [`EngineHost::reload`] 的分工是刻意的：
    ///
    /// | | `reload` | `publish_project` |
    /// | :--- | :--- | :--- |
    /// | 槽 / 队列 / `EngineRuntime` | **全部重建** | 一位不动 |
    /// | 推进量子 | 推 `quanta` 个 | 不推 |
    /// | `quanta` / `meter_*` 契约 | 由它定义（`ui/reload_engine` 的判据钉住） | **不影响** |
    /// | 走带状态 | 不动（见 `reload` 第 5 步） | 不动 |
    ///
    /// 音频线程在**下一个块边界**（`SnapshotReader::begin_block`）就会看到新快照；
    /// 本函数自己跑在**控制线程**上（写者路径允许 `Mutex`，见 `snapshot.rs` 的
    /// `publish_arc` 文档），因此不触碰实时红线 [ARCH-RT-001 / MUST-GATE-001]。
    ///
    /// 旧快照进入写者侧待回收清单 ⇒ 调用方**必须**紧接着跑 [`EngineHost::heartbeat`]
    /// （60Hz），否则清单只涨不回收。
    ///
    /// # Errors
    ///
    /// [`EngineHostError::NoEngine`]（还没 `reload` 过）或
    /// [`EngineHostError::Snapshot`]（投影失败 ⇒ **当前快照一位不动**）。
    pub fn publish_project(
        &mut self,
        project: &YebanProjectV1,
        mark: EditMark,
    ) -> Result<u64, EngineHostError> {
        let Some(slot) = self.slot.as_ref() else {
            return Err(EngineHostError::NoEngine);
        };
        let revision = self.snapshot_revision.saturating_add(1);
        let snapshot = EngineSnapshot::from_project(project, revision)?;
        // 复用槽自己的写者路径（`publish` 就是本函数的包装）。
        slot.publish_arc(std::sync::Arc::new(snapshot));
        self.snapshot_revision = revision;
        self.last_mark = Some(mark);
        Ok(revision)
    }

    /// 这一跳要不要发布：`true` = 标记与"当前快照已经反映的"不同。
    ///
    /// `reload` 之后到第一次 `publish_project` / [`EngineHost::mark_applied`] 之前恒为 `true`。
    #[must_use]
    pub fn should_publish(&self, mark: EditMark) -> bool {
        self.last_mark != Some(mark)
    }

    /// 记下"当前快照已经反映这个标记"（`reload` 之后调用一次，避免开局多发一份）。
    pub fn mark_applied(&mut self, mark: EditMark) {
        self.last_mark = Some(mark);
    }

    /// **心跳**（控制线程 / 主线程，60Hz）：释放写者侧待回收清单，并排空退役队列。
    ///
    /// 两条回收路径都必须在这里有着落，否则"队列空了"可能只是"写者侧清单在涨"
    /// （`snapshot.rs` 的 `pruned` 文档原话）：
    ///
    /// 1. `SnapshotSlot::prune()` —— 释放"被 `ptr` 淘汰、且读者已确认不再引用"的写者侧强引用；
    /// 2. `RetireQueue::drain(RETIRE_CAPACITY)` —— 出队并 **Drop** 读者持有的旧快照。
    ///
    /// 两者都在**非实时线程**上做真正的 `dealloc`：音频线程只 `push`，从不 `drop`
    /// [ARCH-RT-002 / MUST-GATE-012]。没有引擎时返回全 0（不 panic）。
    pub fn heartbeat(&mut self) -> HeartbeatReadings {
        let Some(slot) = self.slot.as_ref() else {
            return HeartbeatReadings {
                published: 0,
                pending_len: 0,
                pruned: 0,
                released: 0,
                retired: 0,
                retire_pending: 0,
            };
        };
        let released = slot.prune();
        let retired = self
            .retire
            .as_mut()
            .map_or(0, |queue| queue.drain(RETIRE_CAPACITY));
        HeartbeatReadings {
            published: slot.published(),
            pending_len: slot.pending_len(),
            pruned: slot.pruned(),
            released,
            retired,
            retire_pending: self.retire.as_ref().map_or(0, RetireQueue::pending),
        }
    }

    /// 当前快照槽的计数（[`HeartbeatReadings`] 之外的静止读数；没有引擎时 `None`）。
    #[must_use]
    pub fn snapshot_counts(&self) -> Option<SnapshotCounts> {
        self.slot.as_ref().map(|slot| SnapshotCounts {
            published: slot.published(),
            pending_len: slot.pending_len(),
            pruned: slot.pruned(),
            epoch: slot.epoch(),
            revision: self.snapshot_revision,
            retire_pending: self.retire.as_ref().map_or(0, RetireQueue::pending),
            retire_dropped: self.retire.as_ref().map_or(0, RetireQueue::dropped),
        })
    }

    /// 当前快照的一份 `Arc`（**控制线程**读：`SnapshotSlot::current()` 会在写者锁上短暂等待）。
    ///
    /// 判据与诊断的读口。音频线程走的仍然是
    /// [`yeban_engine::snapshot::SnapshotReader::begin_block`] —— 本函数不参与那条路径。
    #[must_use]
    pub fn current_snapshot(&self) -> Option<std::sync::Arc<EngineSnapshot>> {
        self.slot.as_ref().map(|slot| slot.current())
    }

    /// 引擎累计统计（没有引擎时 `None`）。
    ///
    /// `EngineStats::snapshot_switches` 是"音频读路径真的换了快照"的见证
    /// （它是 `SnapshotReader::begin_block` 里计的数）。
    #[must_use]
    pub fn engine_stats(&self) -> Option<EngineStats> {
        self.runtime.as_ref().map(EngineRuntime::stats)
    }

    // -----------------------------------------------------------------------
    // 走带（`line/transport-engine`）
    // -----------------------------------------------------------------------

    /// 当前走带读数（**来自引擎**；还没有引擎时是中性冷值）。
    ///
    /// 它读的是 [`TransportMirror`]（原子 seqlock），不是界面自己的状态 ——
    /// "播放中"这个显示值只有一个事实源。
    #[must_use]
    pub fn transport(&self) -> TransportReading {
        self.transport_mirror
            .as_ref()
            .map_or_else(TransportReading::cold, |mirror| mirror.read())
    }

    /// 走带动作日志（判据的注入点；见 [`TransportActionRecord`]）。
    #[must_use]
    pub fn transport_journal(&self) -> &[TransportActionRecord] {
        &self.journal
    }

    /// 是否有一代活着的引擎（走带命令只有在这种情况下才发得出去）。
    #[must_use]
    pub fn transport_ready(&self) -> bool {
        self.runtime.is_some() && self.events.is_some()
    }

    /// 发一批走带命令并**推一个量子**让它们生效，返回命令应用之后的读数。
    ///
    /// 为什么"发完还要推量子"：命令是在**量子边界**由实时侧出队应用的
    /// （`EngineRuntime::render_block` 第 1 步）。没有设备回调时，唯一让边界到来的
    /// 方式就是控制面显式推量子 —— 这正是本 crate 既有的边界
    /// （`engine_host.rs` 模块文档："实时线程今天由控制面/测试线程显式驱动"）。
    /// 真实设备接管之后，这里的推量子会被设备时钟取代，而**命令通道不变**。
    ///
    /// 推的量子数是 `1`：走带位置会因此前进一个运行时量子（128 帧）。这是当前
    /// "没有声卡"形态的**已知代价**，不是走带的语义（写在 notes 的未实现项里）。
    pub fn send_transport(&mut self, commands: &[TransportCommand]) -> TransportReading {
        if !self.transport_ready() {
            return TransportReading::cold();
        }
        let events: Vec<EngineEvent> = commands
            .iter()
            .map(|command| EngineEvent::Transport { command: *command })
            .collect();
        if let Some(sender) = self.events.as_mut() {
            sender.publish(&events);
        }
        let pumped = self.pump(1);
        let reading = self.transport();
        for command in commands {
            self.journal.push(TransportActionRecord {
                command: *command,
                state_after: reading.state,
                position_ticks_after: reading.position_ticks,
                quanta_pumped: pumped,
            });
        }
        reading
    }

    /// 推 `quanta` 个运行时量子（每个量子 [`DEFAULT_BLOCK_FRAMES`] 帧），返回实际推进数。
    ///
    /// 它是"走带路径的控制线程推量子"：推完顺带 `drain` 一次退役队列（历史形态，见
    /// [`Self::drive_audio`]）。没有引擎时返回 0（不 panic、不假装推进过）。
    pub fn pump(&mut self, quanta: u64) -> u64 {
        let driven = self.drive_audio(quanta);
        self.drain_retired();
        driven
    }

    /// 只推量子、**不**排空退役队列（返回实际推进数；没有引擎时 0）。
    ///
    /// 它模拟的是**设备回调**：回调在每个块边界只做"读者无锁换快照 + 把旧快照
    /// `push` 进退役队列"（[`crate::engine_host`] 模块文档的接线图），**释放是心跳的事**
    /// （[`Self::heartbeat`]）。生产 GUI 里音频侧由设备时钟驱动、控制线程不再调
    /// `pump`，因此"读者换了快照、但还没人回收"这个窗口是真实存在的 —— 判据用本函数
    /// 把它造出来，用来证明心跳的 `drain` 腿有牙。
    pub fn drive_audio(&mut self, quanta: u64) -> u64 {
        let Some(runtime) = self.runtime.as_mut() else {
            return 0;
        };
        let channels = self
            .slot
            .as_ref()
            .map_or(2, |slot| slot.current().channels().max(1));
        let mut output = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * usize::from(channels)];
        for _ in 0..quanta {
            runtime.process_quantum(&mut output, channels);
        }
        quanta
    }

    /// 播放（**位置保留** ⇒ 停住之后从这里继续）。
    pub fn play(&mut self) -> TransportReading {
        self.send_transport(&[TransportCommand::Play])
    }

    /// 停住（**位置保留**）。"回到起始点"用 [`Self::stop_and_rewind`]。
    pub fn stop(&mut self) -> TransportReading {
        self.send_transport(&[TransportCommand::Stop])
    }

    /// 停止并回到 tick 0：**一条批量里的两条命令**（`Stop` + `SeekTicks(0)`），
    /// 因此它们在**同一个量子边界**按 FIFO 一起生效 —— 不存在"停住了但还在半路"的中间态。
    pub fn stop_and_rewind(&mut self) -> TransportReading {
        self.send_transport(&[TransportCommand::Stop, TransportCommand::SeekTicks(0)])
    }

    /// 播放 / 停住的切换（界面 `toggle-play` 的落点）。
    ///
    /// 判断依据是**引擎读数**（不是界面属性）：
    /// 读数说 Playing ⇒ 发 `Stop`；否则发 `Play`。
    pub fn toggle_play(&mut self) -> TransportReading {
        if self.transport().state.is_running() {
            self.stop()
        } else {
            self.play()
        }
    }

    /// 定位到 `tick`（960 PPQ），播放状态不变。
    pub fn seek(&mut self, tick: u64) -> TransportReading {
        self.send_transport(&[TransportCommand::SeekTicks(tick)])
    }

    /// 主线程腿：把退役队列里的旧快照真正 **Drop** 掉（[`RetireQueue::drain`]）。
    ///
    /// 返回本次释放的条数。生产路径的正规调用点是 60Hz 心跳（[`Self::heartbeat`]）；
    /// 保留本函数是因为 [`Self::pump`]（走带路径）与 [`Self::reload`] 末尾也要它。
    pub fn drain_retired(&mut self) -> usize {
        self.retire
            .as_mut()
            .map_or(0, |queue| queue.drain(RETIRE_CAPACITY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::project::YebanProjectV1;
    use yeban_model::{AutomationTarget, EntityId, Op};

    use crate::bridge::{ViewState, demo_project};
    use crate::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与 app / MCP 侧既有夹具同一个常量）。
    const NOW: u64 = 1_760_000_000_000;

    /// 判据 1：重建真的推量子，且**每量子恰好一次**批量发布、帧数 = 非母线轨 + 1。
    #[test]
    fn reload_runs_quanta_and_respects_the_publishing_contract() {
        let project = demo_project();
        let view = ViewState::from_project(&project).expect("投影");
        let mut host = EngineHost::new();
        assert_eq!(host.generation(), 0);
        assert!(!host.has_engine());

        let rebuild = host.reload(&project, 16).expect("重建");
        assert_eq!(rebuild.generation, 1);
        assert_eq!(rebuild.revision, 1);
        assert_eq!(rebuild.quanta, 16);
        assert_eq!(rebuild.meter_bulk_publishes, 16, "每量子恰好一次批量发布");
        // 非母线轨（投影里的 tracks）+ 母线 1 条。
        assert_eq!(rebuild.non_bus_tracks(), view.tracks.len());
        assert_eq!(
            rebuild.meter_frames,
            16 * (view.tracks.len() as u64 + 1),
            "帧数 = 量子数 × (非母线轨数 + 1)"
        );
        assert_eq!(rebuild.meter_capacity_drops, 0);
        assert!(host.has_engine());
        assert_eq!(host.generation(), 1);
    }

    /// 判据 2：连着重建两次 ⇒ 代数是 1、2，且每次都给一条**新**队列。
    #[test]
    fn a_second_reload_advances_the_generation_and_hands_out_a_new_queue() {
        let project = demo_project();
        let mut host = EngineHost::new();
        let first = host.reload(&project, 4).expect("第一代");
        assert_eq!(first.generation, 1);
        assert_eq!(first.revision, 1);
        assert_eq!(first.quanta, 4);

        let second = host.reload(&project, 2).expect("第二代");
        assert_eq!(second.generation, 2);
        assert_eq!(second.revision, 2);
        assert_eq!(second.quanta, 2, "统计是**这一代**的, 不是累加的");
        assert_eq!(second.meter_bulk_publishes, 2);
        assert_eq!(
            second.meter_frames,
            2 * (second.non_bus_tracks() as u64 + 1)
        );
    }

    /// 判据 3：**失败必须上报**，而且旧的一代原样保留（不做半更新）。
    #[test]
    fn a_project_without_a_master_bus_is_an_explicit_error_and_keeps_the_old_engine() {
        let good = demo_project();
        let mut host = EngineHost::new();
        host.reload(&good, 3).expect("先装一代好的");
        assert_eq!(host.generation(), 1);

        // `YebanProjectV1::default()` 的 `master_bus_track_id` 是 nil ⇒ NoMasterBus。
        let error = host
            .reload(&YebanProjectV1::default(), 3)
            .expect_err("没有主总线必须报错");
        assert!(
            matches!(error, EngineHostError::Snapshot(SnapshotError::NoMasterBus)),
            "实际错误: {error:?}"
        );
        assert_eq!(host.generation(), 1, "失败不得推进代数");
        assert!(host.has_engine(), "失败不得把旧引擎清空");
    }

    /// 判据 4：`quanta = 0` 是合法的（只重建、不推进）—— 报告里就是 0，不是 panic。
    #[test]
    fn zero_quanta_rebuilds_without_advancing() {
        let mut host = EngineHost::new();
        let rebuild = host.reload(&demo_project(), 0).expect("重建");
        assert_eq!(rebuild.quanta, 0);
        assert_eq!(rebuild.meter_bulk_publishes, 0);
        assert_eq!(rebuild.meter_frames, 0);
        assert_eq!(rebuild.generation, 1);
        assert!(host.has_engine());
    }

    /// 判据 5：**运行时量子长度 = `[ARCH-DET-001]` 的 `DEFAULT_BLOCK_FRAMES`**，
    /// 而快照声明的 `block_size` 是模型层的另一个概念（演示工程声明 256）。
    ///
    /// 这条判据把第一版的真错误钉住：按"快照声明的块长"分配输出缓冲 ⇒ 一次
    /// `process_quantum` 推进 2 个量子 ⇒ `meter_bulk_publishes` 翻倍。
    #[test]
    fn the_runtime_quantum_is_the_det_block_not_the_declared_block_size() {
        let project = demo_project();
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");
        assert!(snapshot.block_size_matches_enum(), "声明值必须是合法枚举");
        assert_eq!(snapshot.channels(), 2, "引擎快照恒以立体声投影");
        assert_eq!(
            yeban_engine::block::DEFAULT_BLOCK_FRAMES,
            128,
            "运行时量子长度是 [ARCH-DET-001] 的固定 128"
        );
        assert_eq!(
            snapshot.block_frames(),
            256,
            "演示工程声明的 block_size 是 256（与运行时量子不是同一个数）"
        );
        // 一次 `reload(.., 1)` 必须恰好推进 **1** 个量子（而不是 2）。
        let mut host = EngineHost::new();
        let rebuild = host.reload(&project, 1).expect("重建");
        assert_eq!(rebuild.quanta, 1);
        assert_eq!(rebuild.meter_bulk_publishes, 1);
        assert_eq!(rebuild.meter_frames, rebuild.non_bus_tracks() as u64 + 1);
    }

    /// 判据 6：退役队列被 drain（返回 0 而不是 panic；本线程内不产生退役项）。
    #[test]
    fn retire_queue_is_drained_without_panicking() {
        let mut host = EngineHost::new();
        assert_eq!(host.drain_retired(), 0, "没有引擎时是 0");
        host.reload(&demo_project(), 2).expect("重建");
        assert_eq!(host.drain_retired(), 0, "同线程内换快照不产生退役项");
    }

    // -----------------------------------------------------------------------
    // 编辑 ⇒ 发声（本票新增的判据 7–10）
    // -----------------------------------------------------------------------

    /// 判据 7–9 的夹具：一条真的撤销会话 + 一代真的引擎 + 一条真的轨道。
    ///
    /// 返回 `(端口, 引擎, 轨道身份, 起点音量 dB)`。端口是 `UndoPort`（**混音台写入面的
    /// 同一条路径**，`host::wire_mixer_edit` 用的就是它）；引擎是 `reload(.., 0)`
    /// 出来的一代（0 个量子 ⇒ 不空转），并且已经用 [`EngineHost::mark_applied`] 记下
    /// "初始快照已经反映开局标记"。
    fn mixer_fixture() -> (UndoPort, EngineHost, EntityId, f32) {
        let project = demo_project();
        let track_id = *project.tracks.keys().next().expect("演示工程至少一条轨道");
        let volume_db = project.tracks[&track_id].volume_db;
        let port = UndoPort::new(
            UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开撤销会话"),
        );
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("重建引擎");
        host.mark_applied(EditMark::from_display(&port.display()));
        (port, host, track_id, volume_db)
    }

    /// 经 `UndoPort::commit_ops` 改一条轨道的音量（**混音台推子松手的同一条路径**）。
    fn commit_volume(port: &UndoPort, track_id: EntityId, old_val: f32, new_val: f32, step: u64) {
        port.commit_ops(
            NOW + step + 1,
            "mixer: set track volume",
            vec![Op::SetParam {
                target: AutomationTarget::TrackVolume { track_id },
                old_val,
                new_val,
            }],
        )
        .expect("音量提交必须成功");
    }

    /// 判据 7（**编辑 ⇒ 发声**，本票的端到端）：经 `UndoPort::commit_ops` 改一个混音参数
    /// ⇒ 增量发布 ⇒ 快照里那**一个字段**真的换了，而且音频读路径在下一个块边界换了快照。
    ///
    /// 起点 / 终点都是**字面读数**：`EngineSnapshot::track(id).volume_db()`（dB，f32）。
    #[test]
    fn a_mixer_edit_reaches_the_engine_snapshot_and_the_reader_switches() {
        let (port, mut host, track_id, before_db) = mixer_fixture();
        let before = host
            .current_snapshot()
            .expect("有引擎必有快照")
            .track(&track_id)
            .expect("快照里有这条轨道")
            .volume_db();
        assert_eq!(before, before_db, "起点：快照里的音量 = 工程里的音量");

        let new_db = before_db - 6.0;
        commit_volume(&port, track_id, before_db, new_db, 0);
        assert_eq!(
            port.project().tracks[&track_id].volume_db,
            new_db,
            "工程真的改了（模型侧读数）"
        );

        // —— 控制线程的一跳：按需发布 + 回收（生产 60Hz 定时器调的就是这三步）——
        let mark = EditMark::from_display(&port.display());
        assert!(host.should_publish(mark), "工程改过 ⇒ 这一跳必须发布");
        let revision = host
            .publish_project(&port.project(), mark)
            .expect("增量发布");
        assert_eq!(revision, 2, "增量发布的版本号 = 快照版本号 + 1");
        assert_eq!(
            host.generation(),
            1,
            "增量发布**不**推进代数（不是 reload）"
        );
        let tick = host.heartbeat();
        assert_eq!(tick.published, 2, "槽里累计发布 2 次（reload 1 + 增量 1）");
        assert_eq!(
            tick.pending_len, 1,
            "读者还没走过这个纪元 ⇒ 那一份必须保持存活（`prune` 不得提前释放）"
        );
        assert_eq!(tick.released, 0, "读者没确认之前一条都不许释放");
        assert!(!host.should_publish(mark), "同一个标记不会再发第二次");

        let after = host
            .current_snapshot()
            .expect("有引擎必有快照")
            .track(&track_id)
            .expect("快照里有这条轨道")
            .volume_db();
        assert_ne!(
            after, before,
            "改音量之后快照里那一个字段必须不同 —— 这就是「编辑 ⇒ 发声」的断点"
        );
        assert_eq!(after, new_db, "快照读到的就是新的那一版");

        // 音频读路径（`begin_block`）真的换了快照：`snapshot_switches` 由它计数。
        let switches_before = host
            .engine_stats()
            .expect("有引擎必有统计")
            .snapshot_switches;
        assert_eq!(host.drive_audio(1), 1, "推一个量子");
        assert_eq!(
            host.engine_stats().expect("统计").snapshot_switches,
            switches_before + 1,
            "下一个块边界必须换到新快照"
        );
        let tick = host.heartbeat();
        assert_eq!(tick.retired, 1, "读者交出的旧快照由心跳排空并 Drop");
        assert_eq!(
            tick.released, 1,
            "读者走过之后，写者侧那一份也在同一跳被回收"
        );
        assert_eq!(tick.pending_len, 0, "回收之后清单回到基线 0");
        // 交付报告要的是**字面读数**：`--nocapture` 时这一行会打出来。
        println!(
            "[engine-snapshot] 音量 {before:.1} → {after:.1} dB; published={} pending_len={} \
             pruned={} switches={}",
            tick.published,
            tick.pending_len,
            tick.pruned,
            host.engine_stats().expect("统计").snapshot_switches,
        );
    }

    /// 判据 8（**发布了必须回收**，本票的回归基线）：连续 100 次编辑。
    ///
    /// 逐跳探针（不是只读末值）：每一跳都断言"这一跳发布了 1 份、释放了 1 份、
    /// 清单回到基线"。因此"某一跳不发布"或"某一跳不回收"都会在**那一跳**当场变红。
    ///
    /// 判据阈值与理由：`pending_len == 0`（**严格等于**基线 0，无容差）。本夹具在每次
    /// 发布之后、心跳之前恰好推**一个**量子 ⇒ 读者的 `reader_done` 覆盖该条目的
    /// `retired_at`（这正是 `prune` 的释放条件）⇒ 任何非零都意味着"发了没回收"。
    /// 生产形态的界是"设备时钟让读者落后多少"，不是本判据的容差。
    #[test]
    fn a_hundred_edits_leave_nothing_pending_on_the_writer_side() {
        const EDITS: u32 = 100;
        let (port, mut host, track_id, start_db) = mixer_fixture();
        let baseline = host.snapshot_counts().expect("有引擎必有计数");
        assert_eq!(baseline.published, 1, "reload 之后槽里只有那一份初始快照");
        assert_eq!(baseline.pending_len, 0, "基线：写者侧清单为空");
        assert_eq!(baseline.pruned, 0, "基线：还没回收过");
        assert_eq!(baseline.retire_dropped, 0, "基线：还没 Drop 过旧快照");

        let mut current_db = start_db;
        let mut max_pending = 0_usize;
        for step in 0..EDITS {
            let next_db = current_db - 0.5;
            commit_volume(&port, track_id, current_db, next_db, u64::from(step));
            current_db = next_db;

            let mark = EditMark::from_display(&port.display());
            assert!(
                host.should_publish(mark),
                "第 {step} 跳：提交之后标记必须变（否则这一跳不会发布）"
            );
            host.publish_project(&port.project(), mark)
                .expect("增量发布");
            // 设备回调：读者在块边界换快照（旧快照进退役队列），**不**回收。
            assert_eq!(host.drive_audio(1), 1, "第 {step} 跳：推一个量子");

            let tick = host.heartbeat();
            assert_eq!(
                tick.released, 1,
                "第 {step} 跳：心跳必须释放写者侧那一条（发布了不回收 = 这里红）"
            );
            assert_eq!(
                tick.pending_len, 0,
                "第 {step} 跳：写者侧清单必须回到基线 0"
            );
            assert_eq!(
                tick.retired, 1,
                "第 {step} 跳：心跳必须排空退役队列并 Drop（不排空 = 这里红）"
            );
            assert_eq!(tick.retire_pending, 0, "第 {step} 跳：退役队列必须为空");
            max_pending = max_pending.max(tick.pending_len);
        }

        let end = host.snapshot_counts().expect("有引擎必有计数");
        assert_eq!(
            end.published,
            1 + u64::from(EDITS),
            "发布次数 = 基线 1 + 编辑 {EDITS} 次"
        );
        assert_eq!(
            end.pruned,
            u64::from(EDITS),
            "每一次发布都在同一跳被回收（累计释放条数）"
        );
        assert_eq!(end.pending_len, 0, "编辑结束：写者侧清单回到基线");
        assert_eq!(
            end.retire_dropped,
            u64::from(EDITS),
            "读者交出的每一份旧快照都真的 Drop 了"
        );
        assert_eq!(end.retire_pending, 0, "编辑结束：退役队列为空");
        assert_eq!(max_pending, 0, "全程逐跳没有一次积压（不是只看末值）");
        // 音频读路径真的换了 100 次快照（不是"发了但读者看不见"）。
        assert_eq!(
            host.engine_stats().expect("统计").snapshot_switches,
            u64::from(EDITS),
            "每一次编辑之后的下一个块边界都换到了新快照"
        );
        // 末值探针：最后那一版仍在快照里（逐跳改的是不同的值）。
        assert_eq!(
            host.current_snapshot()
                .expect("快照")
                .track(&track_id)
                .expect("轨道")
                .volume_db(),
            current_db,
            "第 {EDITS} 次编辑的字段值必须留在当前快照里"
        );
        assert_eq!(current_db, start_db - 0.5 * EDITS as f32);
        // 交付报告要的是**字面读数**：`--nocapture` 时这一行会打出来。
        println!(
            "[snapshot-heartbeat] edits={EDITS} published={} pruned={} pending_len={} \
             retire_dropped={} retire_pending={} switches={} max_pending={max_pending}",
            end.published,
            end.pruned,
            end.pending_len,
            end.retire_dropped,
            end.retire_pending,
            host.engine_stats().expect("统计").snapshot_switches,
        );
    }

    /// 判据 9：投影失败时**当前快照一位不动**（版本号 / 发布计数都不推进）。
    #[test]
    fn a_failed_incremental_publish_keeps_the_current_snapshot() {
        let project = demo_project();
        let track_id = *project.tracks.keys().next().expect("轨道");
        let before_db = project.tracks[&track_id].volume_db;
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("重建");

        // `YebanProjectV1::default()` 的 `master_bus_track_id` 是 nil ⇒ NoMasterBus。
        let error = host
            .publish_project(&YebanProjectV1::default(), EditMark::new(None, 7))
            .expect_err("没有主总线的工程不可能发布");
        assert!(
            matches!(error, EngineHostError::Snapshot(SnapshotError::NoMasterBus)),
            "实际错误: {error:?}"
        );
        let counts = host.snapshot_counts().expect("有引擎必有计数");
        assert_eq!(counts.published, 1, "失败不得推进发布计数");
        assert_eq!(counts.revision, 1, "失败不得推进快照版本号");
        assert_eq!(counts.pending_len, 0, "失败不得留下待回收条目");
        assert_eq!(
            host.current_snapshot()
                .expect("快照")
                .track(&track_id)
                .expect("轨道")
                .volume_db(),
            before_db,
            "失败之后当前快照仍是老那一份"
        );
    }

    /// 判据 10：没有引擎时心跳是**中性**的（全 0，不 panic），发布如实报 `NoEngine`。
    #[test]
    fn heartbeat_without_an_engine_is_neutral_and_publish_is_refused() {
        let mut host = EngineHost::new();
        assert_eq!(
            host.heartbeat(),
            HeartbeatReadings {
                published: 0,
                pending_len: 0,
                pruned: 0,
                released: 0,
                retired: 0,
                retire_pending: 0,
            }
        );
        assert!(host.snapshot_counts().is_none());
        assert!(host.current_snapshot().is_none());
        assert!(host.engine_stats().is_none());
        let error = host
            .publish_project(&demo_project(), EditMark::new(None, 0))
            .expect_err("没有引擎时发布必须报错");
        assert!(matches!(error, EngineHostError::NoEngine), "{error:?}");
    }
}
