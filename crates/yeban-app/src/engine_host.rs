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
//! - **退役队列只被 `drain` 一次**：本切片没有 60Hz 主线程心跳，`reload` 末尾主动
//!   `drain` 一次即可（快照交换发生在本线程内，队列不可能积压）。

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::{DEFAULT_METER_CAPACITY, MeterCollector, meter_channel};
use yeban_engine::ring::{EngineEvent, EventSender, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{
    EngineSnapshot, RetireQueue, SnapshotError, SnapshotSlot, retire_channel,
};
use yeban_engine::transport::{TransportMirror, TransportReading, TransportState};
use yeban_model::project::YebanProjectV1;

/// 退役队列容量（条）。同一次 `reload` 内最多发生一次快照交换，32 条足够。
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
}

impl core::fmt::Display for EngineHostError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Snapshot(error) => write!(formatter, "引擎快照投影失败: {error}"),
        }
    }
}

impl core::error::Error for EngineHostError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Snapshot(error) => Some(error),
        }
    }
}

impl From<SnapshotError> for EngineHostError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
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
    slot: Option<std::sync::Arc<SnapshotSlot>>,
    runtime: Option<EngineRuntime>,
    retire: Option<RetireQueue>,
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
        let revision = self.generation.saturating_add(1);
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
        self.generation = revision;
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
    /// 没有引擎时返回 0（不 panic、不假装推进过）。
    pub fn pump(&mut self, quanta: u64) -> u64 {
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
        self.drain_retired();
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
    /// 返回本次释放的条数。生产路径应当由 60Hz 心跳调用；本切片在 `reload` 末尾调用一次
    /// （唯一会产生退役的时刻就是换快照）。
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

    use crate::bridge::{ViewState, demo_project};

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
}
