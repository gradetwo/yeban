//! 渲染量子驱动：**不依赖 cpal** 的回调内逻辑。[ARCH-TOP-002, ARCH-RT-001, ARCH-UI-002]
//!
//! 本模块把"一个音频回调该做什么"完整实现成 [`EngineRuntime::process_quantum`]，
//! 并且**完全不引用 cpal**。这样做的理由是很具体的工程约束：
//!
//! - CI 上没有声卡，真实设备路径无法端到端运行；
//! - 但"回调逻辑"（事件批量出队 → 快照无锁切换 → 渲染 → 逐轨/母线电平上报 → 退役入队）
//!   才是红线 7 的所在，必须可测。
//!
//! 于是分成两层：
//!
//! ```text
//!  EngineRuntime::process_quantum(&mut [f32], channels)   ← 本模块, 纯逻辑, 可单测
//!         ▲                                    ▲
//!         │ cpal 回调闭包                       │ device::NullBackend（无设备驱动）
//!  device::open_output(...)              device::NullBackend::render(frames)
//! ```
//!
//! ## 每量子的处理顺序（这条顺序就是契约）
//!
//! ```text
//! 1) 事件：每块**一次**批量出队 [ROAD-M2-007]
//!      走带命令（`EngineEvent::Transport`）在这一步按 FIFO 应用 [crate::transport]：
//!        Play/Stop/Pause ⇒ 状态切换（位置保留）；SeekTicks(t) ⇒ 位置 = t 且
//!        合成器播放头对齐到该 tick 的帧位置
//! 2) 快照：begin_block() 无锁切换 [ARCH-RT-002]；revision 变化时
//!       a) 重设电平弹道系数；b) 声部池对齐到新轨道集合 + 游标校正（只增不减）；
//!       c) 走带按同一份快照的 `sample_rate` / `bpm` 武装（位置不动 ⇒ 不跳变）
//! 3) 渲染 + 电平：对快照里**每条非母线轨**
//!       SynthEngine::render_track(该轨的 NoteSchedule) → track_scratch（声相之前、单声道）
//!         →  MeterBank::measure(...)                        ← 逐轨电平口径不变
//!         →  **PDC 补偿延迟线（`D(v) = L_max − arrival(v)` 采样点）** [ARCH-PDC-001]
//!         →  sum_into_bus(声相增益 (cos θ, sin θ)，构造期算好)
//!    然后 **BusLimiter::apply(block)**                    ← 母线峰值限制（前瞻 33 帧）
//!    再对母线（stereo-linked）MeterBank::measure_bus_stereo(block)   ← **限制之后**的读数
//!    最后播放头前进 frames（**每量子一次**，与轨道数无关）；
//!      **走带停住时**这一步被跳过、逐轨渲染也被跳过（输出静音、不触发音符）——
//!      电平照常计量，所以"每量子一次批量发布"与走带状态无关
//! 4) 发布：**每量子恰好一次** meters.publish(本量子的全部帧) [ARCH-UI-002, ROAD-M2-008]
//! 5) 走带读数：**每量子恰好一次** transport.publish(镜面)（原子 seqlock，无锁无分配）
//! 6) end_block() 公布读者进度
//! ```
//!
//! **每量子发布帧数 = 非母线轨数 + 1（母线）**。母线同时出现在 `tracks()` 里时
//! （`EngineSnapshot::from_project` 的常态，主总线本身也是一条 `TrackV3`）
//! **不会**被重复计量 —— 这正是本线修正的一处口径。
//!
//! ## 真的出声了（本切片的核心）
//!
//! 步骤 3 的第一句在本次切片之前是 `render_track_into` 的**占位静音**
//! （`out.fill(0.0)`）。现在它由 [`crate::synth::SynthEngine`] 从
//! **快照里的音符调度表**驱动逐样本合成：
//!
//! ```text
//! YebanProjectV1 ──(控制线程投影, snapshot::project_schedules)──► NoteSchedule
//!   clips → ClipPlacement → clip_pool(Midi) → MidiNote ──(tick → sample)──►
//!     ScheduledNote { start_sample, end_sample, phase_inc, freq_hz, gain }
//! ──(RT: 游标触发 → 定长声部池 → 整数相位波表读数 → 声部低通)──► track_scratch
//! ──(声相增益)──► 母线 L/R ──(前瞻峰值限制器)──► AudioBlock ──► cpal / NullBackend
//! ```
//!
//! 实时侧仍然是零分配/零锁/零 I/O：声部池是 `[TrackSlot; 16]`（每槽 16 个声部），
//! 波表是构造期建好的 `Vec`，逐样本路径只有整数递推与 IEEE 精确类浮点运算
//! （D32 分类见 [`crate::synth`] 模块文档 §2）。
//!
//! 播放头（[`EngineRuntime::position_samples`]）属于引擎自己，**不在快照里**：
//! [MODEL-ISO-001] 明确禁止把挥发性走带状态塞进模型投影。
//!
//! ## 电平口径
//!
//! 峰值 / 峰值保持（20 dB/s 指数释放）/ 块 RMS / 平滑 RMS（τ=300 ms）/
//! `NaN`·`±∞` 钳位全部在 [`crate::level`]，每节点状态由 [`MeterBank`] 持有。
//! 每轨测的是**单声道、声相之前**的轨道渲染结果；母线测的是**立体声联动**的汇总块。
//!
//! ## 回调内禁令自检（[AGENTS.md §2 红线 7]）
//!
//! `process_quantum` 的调用树里**没有**：`Vec::push` / `Box::new` / `format!` / `println!`
//! / `Mutex::lock` / 文件或网络调用。所有临时缓冲都是结构体字段里的定长数组：
//!
//! - [`EngineRuntime::scratch_events`]：`[EngineEvent; 128]`（栈/内联）
//! - [`EngineRuntime::scratch_meters`]：`[MeterFrame; 256]`（本量子的发布批次）
//! - [`EngineRuntime::track_scratch`]：`[f32; 128]`（单轨渲染结果，复用一个缓冲）
//! - [`EngineRuntime::block`]：`AudioBlock<128>`（`[f32; 128]` × 2）
//! - [`EngineRuntime::synth`]：`[TrackSlot; 16]` × `[Voice; 16]`（声部池，定长）
//! - [`MeterBank`]：`[MeterSlot; 256]`（每节点电平状态，定长数组 + 原位 `swap` 对齐）
//!
//! 唯一允许的"共享状态"是原子量与 rtrb 队列；唯一的系统调用级别操作是
//! FTZ/DAZ 控制寄存器写入（一次）。

use std::sync::Arc;

use yeban_model::EntityId;

use crate::block::{AudioBlock, DEFAULT_BLOCK_FRAMES};
use crate::fpu::{self, FtzDazOutcome};
use crate::graph::CompensationBank;
use crate::meter::{MeterBank, MeterFrame, MeterPublisher, SCRATCH_METERS};
use crate::mixer::{BusLimiter, PanLaw};
use crate::ring::{EngineEvent, EventReceiver, SCRATCH_EVENTS};
use crate::rt_probe::{self, RtDiagEvent};
use crate::snapshot::{RetireProducer, SnapshotReader, SnapshotSlot};
use crate::synth::{MAX_TRACK_SLOTS, SynthEngine};
use crate::transport::{
    Transport, TransportEffect, TransportMirror, TransportReading, TransportState,
};

// 编译期钉住块长是规范允许的取值 [ARCH-DET-001, MODEL-AST-002]。
const _: () = crate::block::assert_supported_frames::<DEFAULT_BLOCK_FRAMES>();
// 栈上临时事件缓冲至少能装下一个量子的参数洪峰。
const _: () = assert!(SCRATCH_EVENTS >= DEFAULT_BLOCK_FRAMES);

/// 实时 PDC 延迟线池的**槽位数**：与声部池的轨道上限同源（[`MAX_TRACK_SLOTS`]）。
///
/// 理由不是"刚好够用"而是"同一个事实源"：`SynthEngine` 只为前 [`MAX_TRACK_SLOTS`]
/// 条轨分配声部槽，第 17 条轨本身就没有信号可补偿。槽位用尽的计划由
/// [`EngineStats::pdc_unarmed_nodes`] 如实计数（**不静默**）。
pub const PDC_SLOTS: usize = MAX_TRACK_SLOTS;

/// 每条 PDC 延迟线的**最大补偿延迟**（采样点）。
///
/// ⚠ 这是**实现边界，不是规范常数**：[ARCH-PDC-001] / [ARCH-PDC-002] 只说
/// "插入 `D_i = L_max - L_i` 个采样点的环形延迟缓冲"，**没有给上限**。而实时回调
/// 不许分配 [MUST-GATE-001] ⇒ 缓冲必须在构造期定长预分配 ⇒ 必然存在一条容量线。
///
/// 取值 8192 帧 @48 kHz ≈ **170.7 ms**，比本仓库任何已登记的量级都大两个数量级
/// （模型样本设备 32 帧；母线前瞻限制器 33 帧；`process_quantum` 的处理量子 128 帧）。
/// 超出这条线的计划由 [`EngineStats::pdc_clamped_frames`] 计数；`yeban-render` 的
/// 离线路径按计划精确分配容量、**没有**这条上限（它允许分配）。
pub const MAX_PDC_DELAY_FRAMES: usize = 8192;

/// 渲染驱动的累计统计（音频线程写入，非实时线程读取 —— 只用于诊断/UI）。
///
/// 刻意**不派生 `Eq`**：`quanta_per_second` 是 `f32`（浮点没有全序），
/// 与 `yeban-model::ModelError` 当初去掉 `Eq` 是同一个理由。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EngineStats {
    /// 累计渲染量子数。
    pub quanta: u64,
    /// 累计应用的事件条数（不含 `Idle` 填充）。
    pub events_applied: u64,
    /// 累计快照切换次数。
    pub snapshot_switches: u64,
    /// 累计批量出队调用次数（结构性判据：应等于 `quanta`）。
    pub event_bulk_pops: u64,
    /// 累计上报的电平帧数。
    pub meter_frames: u64,
    /// 累计电平批量发布次数（结构性判据：应等于"有快照的量子数"）。
    pub meter_bulk_publishes: u64,
    /// 累计因电平容量耗尽而未计量/被淘汰的节点次数
    /// （发布批次放不下的轨道 + [`MeterBank`] 淘汰的槽）。
    pub meter_capacity_drops: u64,
    /// 本线程的 FTZ/DAZ 开关结果。
    pub ftz: Option<FtzDazOutcome>,
    /// 播放头已渲染的样本数（= 音频线程当前的绝对位置）。
    ///
    /// 它是 **`EngineRuntime` 自己的状态**，不在快照里：`EngineSnapshot` 是不可变的
    /// 模型投影，按 [MODEL-ISO-001] 不允许携带挥发性走带状态。
    pub rendered_samples: u64,
    /// 当前快照里已调度的音符条数（模型 → 快照 → 合成的**覆盖度**判据）。
    pub scheduled_notes: u64,
    /// 当前快照构造时因容量上限丢弃的音符条数。
    pub note_schedule_drops: u64,
    /// 因声部池轨道槽耗尽而未参与合成的轨道次数。
    pub track_drops: u64,
    /// 累计触发过的音符数。
    pub notes_triggered: u64,
    /// 声部池累计**软窃取**次数（每次伴随 3 ms 淡出 [ARCH-RT-004]）。
    pub voice_steals: u64,
    /// 母线限制器**累计压过的样本数**（[ARCH-DSP-001]；0 = 从未越过阈值）。
    ///
    /// 它是"限制器真的接在母线上"的结构性证据：只断言"峰值 ≤ 阈值"在
    /// **从未越过阈值**的夹具上会永真（假绿），因此把"压了多少个样本"暴露出来。
    pub limiter_gain_reductions: u64,
    /// 母线限制器累计的**最大**瞬时压限量（1.0 − 最小增益；0 = 从未压过）。
    pub limiter_max_reduction: f32,
    /// 因实时侧 PDC 延迟线池**槽位用尽**而未能武装的节点数（累计；正常恒为 0）[ROAD-M2-004]。
    ///
    /// 非 0 = 那一份计划里有节点**没有**得到补偿。它是"容量不足不静默"的机械形式：
    /// 只把补偿"尽力而为"地做掉，会让相位对齐悄悄失效而没有任何读数。
    pub pdc_unarmed_nodes: u64,
    /// 因延迟线**容量上限**（[`MAX_PDC_DELAY_FRAMES`]）而被钳掉的补偿帧数
    /// （累计；正常恒为 0）。
    pub pdc_clamped_frames: u64,
    /// PDC 延迟线**真的施加过非零延迟**的"节点·量子"次数（累计）[ROAD-M2-004]。
    ///
    /// 它是"接线"这件事的**见证**：[`EngineRuntime::armed_pdc_delay`] 只能证明
    /// "计划被武装进了池"，而一个被武装却**从不被调用**的池与一个正常工作的池
    /// 在读数上完全一样（相位错位不会 panic、也不会让峰值判据变红）。
    /// `delay == 0` 的直通节点不计入（它们没有工作可做）。
    pub pdc_processed_blocks: u64,
    /// 当前快照下武装的**每秒量子数**（= `sample_rate / DEFAULT_BLOCK_FRAMES`）。
    ///
    /// 为什么把它暴露出来: 它曾经被错算成 `sample_rate / 设备缓冲长度`
    /// （项目声明的 `audio_config.block_size`, 例如 256）⇒ 峰值保持按 10 dB/s 而不是 20 dB/s 衰减。
    /// 那种错**不会 panic、也不会让既有判据变红**, 只会让表头慢慢不准 ——
    /// 所以把"武装进去的那个数"变成可读的统计量, 让判据能直接钉住它。
    ///
    /// `None` = 还没有任何快照被处理过。
    pub quanta_per_second: Option<f32>,
    /// 走带状态（[`crate::transport`]；**不是**设备流的播放状态）。
    pub transport_state: TransportState,
    /// 走带位置（960 PPQ 整数 tick）。
    pub position_ticks: u64,
    /// 走带位置（帧；与 [`Self::rendered_samples`] 同步）。
    pub position_frames: u64,
    /// 累计应用的走带命令条数（含幂等的重复 `Play`/`Stop`）。
    pub transport_commands: u64,
    /// 累计在**推进状态**下处理过的量子数（停住时不自增）。
    pub transport_quanta: u64,
    /// 因**退役队列满**而被迫把旧快照寄存到读者手里的累计次数
    /// （[`EngineRuntime::snapshot_stash_events`] 进统计面的那一份；`> 0` = 曾追不上写者）。
    ///
    /// 语义见 [`Self::is_snapshot_lagging`] / [`Self::stash_events_since_last_read`]。
    pub snapshot_stash_events: u64,
    /// 退役队列当前待回收条数的**跨线程镜像**（**量规**：可升可降）。
    ///
    /// 精确读在控制线程上是 `RetireQueue::pending()`；这一份来自
    /// [`crate::snapshot::RetireAccounting`]，因此渲染驱动与音频线程也看得到。
    ///
    /// 它 = **成功入队数 − 已出队数**（两个单调量之差，见 `RetireAccounting` 的文档）：
    /// 静止点上与精确读**恒等**；并发窗口里可能瞬时差 1（环里已放进去、`+1` 还没落地）。
    /// ⚠ 第一版把它做成"`drain` 后覆写为真实剩余"的量规，那个覆写与音频线程的 `+1`
    /// 不原子 ⇒ 会**永久**少记一条（CI 实测 `镜像=512 权威=513`）。
    pub retire_pending: u64,
    /// 累计真正出队并 `Drop` 的旧快照条数（`= RetireQueue::dropped()`；单调不减，饱和）。
    pub retire_drained: u64,
    /// 累计**非空** `drain` 调用次数（`= RetireQueue::drain_calls()`；结构性判据：
    /// 控制面的 60Hz 排空循环是否真的在排空，而不是"空转但不为空"）。
    pub retire_drain_calls: u64,
    /// 累计由写者侧清单 `SnapshotSlot::prune()` 释放的强引用条数（单调不减，饱和）。
    ///
    /// 它与 [`Self::retire_drained`] 是退役回收的**两条**路径：读者交回的那一份 vs
    /// 写者 anchor 淘汰的那一份。只看一条会把"另一条在涨"误读成健康。
    pub retire_pruned: u64,
    /// **释放线程归属**：首个执行退役释放的线程是否就是创建退役队列的那个线程
    /// （= 控制线程 / 释放归属线程；尚无释放时为 `true`）。
    ///
    /// 与 [`EngineRuntime::snapshot_stash_events`] 同族的健康读数：`false` = 旧快照的
    /// `Drop` 没有集中在控制线程上（[MUST-GATE-012] 的硬要求）。
    pub release_thread_is_main: bool,
    /// 在释放归属线程**之外**发生的非空 `drain` 次数（`> 0` = 释放不集中；要求恒为 0）。
    ///
    /// 与 [`Self::release_thread_is_main`] 是两个不同的判据：首个 `drain` 就跑在外线程时
    /// 这个数是 0、而归属布尔量已经是 `false`。
    pub foreign_drains: u64,
}

impl EngineStats {
    /// **"追不上"的累计判定**：自进程开始以来，是否至少发生过一次
    /// "退役队列满 ⇒ 读者停止切换快照、把旧快照寄存"（[`Self::snapshot_stash_events`] `> 0`）。
    ///
    /// `true` 的含义**不是**"当前很慢"，而是"本进程的拓扑更新至少被推迟过一次"——
    /// 因为该状态在设计上是**粘滞**的：读者会一直用旧快照，直到控制面排空退役队列。
    ///
    /// **控制面应当据此降速**（`docs/ledger/engine-stats-notes.md` 的 needs）：
    /// ① 停止/降低 `SnapshotSlot::publish` 的频率；② 立刻 `RetireQueue::drain` + `prune`；
    /// ③ 提示"引擎拓扑更新被推迟"，而不是继续按原速率发布。
    /// 需要"这一段采样窗口里是否又追不上"的**增量**判定时用
    /// [`Self::stash_events_since_last_read`] / [`Self::is_snapshot_lagging_since`]。
    #[must_use]
    pub const fn is_snapshot_lagging(&self) -> bool {
        self.snapshot_stash_events > 0
    }

    /// **自上次读取以来**新增的 stash 次数（增量；`previous` 是上一次的 `EngineStats`）。
    ///
    /// 语义：`self.snapshot_stash_events - previous.snapshot_stash_events`（饱和减法，
    /// 因此读到更旧的快照时给出 `0` 而不是回绕）。
    /// 控制面的 60Hz 循环每次读 `stats()` 时把上一帧留着，比较这个增量即可得到
    /// "**这 16.7 ms 内**引擎有没有被退役积压逼停"——这正是降速决策需要的信号；
    /// 只看累计量无法区分"很久以前抖过一次"和"现在正在抖"。
    #[must_use]
    pub const fn stash_events_since_last_read(&self, previous: &Self) -> u64 {
        self.snapshot_stash_events
            .saturating_sub(previous.snapshot_stash_events)
    }

    /// 采样窗口内的"追不上"判定：`self` 相对 `previous` 新增了至少一次 stash。
    #[must_use]
    pub const fn is_snapshot_lagging_since(&self, previous: &Self) -> bool {
        self.stash_events_since_last_read(previous) > 0
    }

    /// 退役队列是否仍有待回收（背压还在；**量规**，与 [`Self::is_snapshot_lagging`] 不同：
    /// 它可以自行回落，而 lagging 是粘滞的）。
    #[must_use]
    pub const fn has_retire_backlog(&self) -> bool {
        self.retire_pending > 0
    }
}

/// 渲染驱动：音频回调持有的全部可变状态。
pub struct EngineRuntime {
    snapshot: SnapshotReader,
    events: EventReceiver,
    meters: MeterPublisher,
    block: AudioBlock<DEFAULT_BLOCK_FRAMES>,
    scratch_events: [EngineEvent; SCRATCH_EVENTS],
    scratch_meters: [MeterFrame; SCRATCH_METERS],
    /// 单轨渲染结果（声相之前、单声道）。复用同一个缓冲，避免每轨一份。
    track_scratch: [f32; DEFAULT_BLOCK_FRAMES],
    /// 每节点电平状态机（峰值保持 / 平滑 RMS）。
    bank: MeterBank<SCRATCH_METERS>,
    /// 声部池 + 播放头（**真的合成**：见 [`crate::synth`]）。
    synth: SynthEngine,
    /// **走带状态机**（960 PPQ 整数 tick；[`crate::transport`]）[MODEL-ISO-001]。
    ///
    /// 它**不在快照里**：快照是不可变的模型投影，而播放头是挥发性会话运行态。
    /// 位置每量子按整数有理数推进；`Stop` 时冻结且输出静音。
    transport: Transport,
    /// RT → 控制侧的走带读数镜面（原子量 + seqlock；无锁、零分配）。
    ///
    /// 控制面（`yeban-app::engine_host`）持同一个 `Arc` 的一份克隆读数。
    /// 实时侧只写不读，因此永远不会被读者阻塞。
    transport_mirror: Arc<TransportMirror>,
    /// 母线峰值限制器（前瞻式，立体声联动）[ARCH-DSP-001]。
    ///
    /// 位置：**逐轨汇流之后、母线电平之前** ⇒ 母线电平读数（[`EngineStats::meter_frames`]）
    /// 就是**限制后**的读数，而逐轨电平仍是**声相之前**的单声道读数。
    limiter: BusLimiter,
    /// **内部 PDC 延迟线池** [ARCH-PDC-001, ROAD-M2-004]。
    ///
    /// 位置：**每轨输出之后、汇入母线之前**（规范 §3.4 第 3 条的字面位置：
    /// "在进入总线求和节点前自动插入 `D_i = L_max − L_i` 采样点的环形延迟缓冲"）。
    /// 延迟量来自快照里的 [`crate::graph::PdcPlan::compensation`]，在**快照边界**
    /// （修订变化时）用 [`CompensationBank::rearm`] 重新武装 —— 只写 `set_delay`，
    /// 因此回调内**零分配** [MUST-GATE-001]。
    pdc: CompensationBank,
    /// 见 [`EngineStats::pdc_unarmed_nodes`]。
    pdc_unarmed_nodes: u64,
    /// 见 [`EngineStats::pdc_clamped_frames`]。
    pdc_clamped_frames: u64,
    /// 本快照武装的声相衰减律（`audio_config.pan_law` 的投影）。
    armed_pan_law: PanLaw,
    /// 本快照武装的每轨声相增益 `(左, 右)`（构造期算好，实时侧只做乘法）。
    armed_pan_gains: [(EntityId, f32, f32); MAX_TRACK_SLOTS],
    /// 本快照武装的声相增益条数（前 `n` 项有效）。
    armed_pan_slots: usize,
    /// 累计被限制器压过的样本数（与 [`EngineStats::limiter_gain_reductions`] 同源）。
    limiter_gain_reductions: u64,
    /// 累计最大压限量（与 [`EngineStats::limiter_max_reduction`] 同源）。
    limiter_max_reduction: f32,
    /// 已按哪一份快照的采样率/块长设置过弹道系数。
    armed_revision: Option<u64>,
    quanta: u64,
    events_applied: u64,
    event_bulk_pops: u64,
    meter_frames: u64,
    meter_bulk_publishes: u64,
    meter_capacity_drops: u64,
    ftz: Option<FtzDazOutcome>,
    ftz_ready: bool,
    /// 与 [`EngineStats::quanta_per_second`] 同源（实时侧只写一次, 控制面只读）。
    armed_quanta_per_second: Option<f32>,
    /// 与 [`EngineStats::scheduled_notes`] 同源（武装快照时抓取）。
    armed_scheduled_notes: u64,
    /// 与 [`EngineStats::note_schedule_drops`] 同源（武装快照时抓取）。
    armed_note_schedule_drops: u64,
}

impl EngineRuntime {
    /// 组装渲染驱动。
    ///
    /// `slot` / `retire` 来自 [`SnapshotSlot`] 与 [`crate::snapshot::retire_channel`]；
    /// `events` / `meters` 来自 [`crate::ring::event_channel`] 与 [`crate::meter::meter_channel`]。
    /// 全部通道都必须在**打开设备之前**建立（回调内不允许分配）。
    ///
    /// `retire` 是 [`RetireProducer`]（`rtrb::Producer` 的薄包装）：它带着退役队列的
    /// 跨线程记账 ⇒ [`Self::stats`] 能直接报出退役队列的 `pending` / `drained` /
    /// 释放线程归属，**控制面不需要额外接线**（`retire_channel` 的调用点一字未改）。
    #[must_use]
    pub fn new(
        slot: &Arc<SnapshotSlot>,
        retire: RetireProducer,
        events: EventReceiver,
        meters: MeterPublisher,
    ) -> Self {
        let runtime = Self {
            snapshot: SnapshotReader::attach(slot, retire),
            events,
            meters,
            block: AudioBlock::new(),
            scratch_events: [EngineEvent::IDLE; SCRATCH_EVENTS],
            scratch_meters: [MeterFrame::default(); SCRATCH_METERS],
            track_scratch: [0.0; DEFAULT_BLOCK_FRAMES],
            bank: MeterBank::new(),
            // 采样率先按 48 kHz 武装；第一次武装快照时按快照校准（`begin_snapshot`）。
            synth: SynthEngine::new(48_000),
            // 走带默认**自由跑**：接入走带之前 `process_quantum` 的语义就是
            // "快照一发布就从 tick 0 起滚"，因此未收到任何命令时行为逐位不变。
            // 要"加载即停住"的控制面显式发一条 `Stop`（`yeban-app` 的 GUI 路径
            // 在 `EngineHost::reload` 之后就这么做；`reload` 自己**不**碰走带状态）。
            transport: Transport::free_running(48_000, yeban_model::project::DEFAULT_BPM),
            transport_mirror: Arc::new(TransportMirror::new()),
            limiter: BusLimiter::new(),
            // PDC 延迟线池：**构造期**按上限预分配（回调内绝不再分配）。
            pdc: CompensationBank::preallocated(PDC_SLOTS, MAX_PDC_DELAY_FRAMES),
            pdc_unarmed_nodes: 0,
            pdc_clamped_frames: 0,
            armed_pan_law: PanLaw::default(),
            armed_pan_gains: [(
                EntityId::default(),
                core::f32::consts::FRAC_1_SQRT_2,
                core::f32::consts::FRAC_1_SQRT_2,
            ); MAX_TRACK_SLOTS],
            armed_pan_slots: 0,
            limiter_gain_reductions: 0,
            limiter_max_reduction: 0.0,
            armed_revision: None,
            quanta: 0,
            events_applied: 0,
            event_bulk_pops: 0,
            meter_frames: 0,
            meter_bulk_publishes: 0,
            meter_capacity_drops: 0,
            ftz: None,
            ftz_ready: false,
            armed_quanta_per_second: None,
            armed_scheduled_notes: 0,
            armed_note_schedule_drops: 0,
        };
        // 先发布一次初值：控制面在**第一次量子之前**就能读到"Playing / tick 0"，
        // 而不是一个与引擎实际状态不符的冷值（`TransportReading::cold`）。
        runtime.transport.publish(&runtime.transport_mirror);
        runtime
    }

    /// 处理一个（可能是任意长度的）输出缓冲：按 [`DEFAULT_BLOCK_FRAMES`] 切成整量子。
    ///
    /// `output` 是 cpal 给出的**交错**缓冲；`channels` 是通道数。长度不是
    /// `channels * k` 时，尾部不足一帧的样本保持原值（cpal 保证输出缓冲预填静音）。
    ///
    /// 本函数是实时路径：零分配、零锁、零阻塞 I/O [红线 7]。
    pub fn process_quantum(&mut self, output: &mut [f32], channels: u16) {
        self.arm_fpu_once();
        let channels = usize::from(channels.max(1));
        let total_frames = output.len() / channels;
        let mut offset = 0usize;
        while offset < total_frames {
            let frames = (total_frames - offset).min(DEFAULT_BLOCK_FRAMES);
            self.render_block(frames);
            for channel in 0..channels {
                for frame in 0..frames {
                    let sample = self.block.get(channel, frame).unwrap_or(0.0);
                    output[(offset + frame) * channels + channel] = sample;
                }
            }
            offset += frames;
        }
        // 帧边界对齐的交错缓冲不会留下尾巴；非对齐的残余保持 cpal 预填的静音。
    }

    /// 当前累计统计（**只读快照**：无锁、零分配；读它不会影响渲染路径）。
    ///
    /// 它把控制面必须能看见的**引擎健康读数**一并交出（`needs` N2/N5）：
    /// [`EngineStats::snapshot_stash_events`]（"追不上"）、
    /// [`EngineStats::retire_pending`] / [`EngineStats::retire_drained`] /
    /// [`EngineStats::retire_pruned`]（退役回收的两条路径）、
    /// [`EngineStats::release_thread_is_main`] / [`EngineStats::foreign_drains`]
    /// （"释放发生在哪个线程"）。判定见 [`EngineStats::is_snapshot_lagging`] 与
    /// [`EngineStats::stash_events_since_last_read`]。
    #[must_use]
    pub fn stats(&self) -> EngineStats {
        let retire = self.snapshot.retire_accounting();
        EngineStats {
            quanta: self.quanta,
            events_applied: self.events_applied,
            snapshot_switches: self.snapshot.switches(),
            event_bulk_pops: self.event_bulk_pops,
            meter_frames: self.meter_frames,
            meter_bulk_publishes: self.meter_bulk_publishes,
            meter_capacity_drops: self
                .meter_capacity_drops
                .saturating_add(self.bank.capacity_drops()),
            ftz: self.ftz,
            rendered_samples: self.synth.position(),
            scheduled_notes: self.armed_scheduled_notes,
            note_schedule_drops: self.armed_note_schedule_drops,
            voice_steals: self.synth.voice_steals(),
            track_drops: self.synth.track_drops(),
            notes_triggered: self.synth.notes_triggered(),
            limiter_gain_reductions: self.limiter_gain_reductions,
            limiter_max_reduction: self.limiter_max_reduction,
            pdc_unarmed_nodes: self.pdc_unarmed_nodes,
            pdc_clamped_frames: self.pdc_clamped_frames,
            pdc_processed_blocks: self.pdc.processed_blocks(),
            quanta_per_second: self.armed_quanta_per_second,
            transport_state: self.transport.state(),
            position_ticks: self.transport.position_ticks(),
            position_frames: self.transport.position_frames(),
            transport_commands: self.transport.commands_applied(),
            transport_quanta: self.transport.quanta_played(),
            snapshot_stash_events: self.snapshot.stash_events(),
            retire_pending: retire.pending(),
            retire_drained: retire.drained(),
            retire_drain_calls: retire.drain_calls(),
            retire_pruned: self.snapshot.pruned(),
            release_thread_is_main: retire.release_thread_is_owner(),
            foreign_drains: retire.foreign_drains(),
        }
    }

    /// 本快照武装的**每轨 PDC 补偿延迟**（诊断/判据用；采样点）。
    ///
    /// 返回 `None` = 该节点不在本快照的 PDC 计划里（不可达 master，或池的槽位用尽）。
    /// 存在的理由与 [`Self::armed_pan_gain`] 同族：把"武装进去的那个数"变成**可读**的，
    /// 判据就不必从音频输出反推它 —— 而 PDC 恰恰是"看不出错"的那一类（相位错位
    /// 不会 panic，也不会让峰值判据变红）。
    #[must_use]
    pub fn armed_pdc_delay(&self, node: &EntityId) -> Option<usize> {
        self.pdc.line(node).map(crate::graph::DelayLine::delay)
    }

    /// 走带状态机的只读视图（**实时侧状态**；同线程判据/诊断用）。
    ///
    /// 跨线程读数请用 [`Self::transport_mirror`] —— 直接读 `&Transport` 只有在
    /// "调用者就是处理量子的那个线程"时才安全（`EngineHost` 的控制面驱动就是这种形态）。
    #[must_use]
    pub const fn transport(&self) -> &Transport {
        &self.transport
    }

    /// 因**退役队列满**而被迫把旧快照寄存在读者手里的次数（[MUST-GATE-012] 的观测面）。
    ///
    /// `> 0` 说明主线程的 60Hz 轮询没跟上快照交换速率：读者宁可**晚一个块**再切换拓扑，
    /// 也绝不在音频线程上释放内存（[ARCH-RT-002]）。
    ///
    /// 它**不是**释放违规（强引用仍然在读者手里），但会让"高频交换压测"被队列容量打折
    /// ⇒ [MUST-GATE-012] 的判据要求压测期间恒为 `0`。
    ///
    /// 同一个数已经进了统计面（[`EngineStats::snapshot_stash_events`]）；控制面应当用
    /// [`EngineStats::is_snapshot_lagging`] / [`EngineStats::stash_events_since_last_read`]
    /// 判定并**降速**（见 `docs/ledger/engine-stats-notes.md` 的 needs）。
    #[must_use]
    pub const fn snapshot_stash_events(&self) -> u64 {
        self.snapshot.stash_events()
    }

    /// RT → 控制侧的走带读数镜面（原子量；跨线程安全）。
    #[must_use]
    pub const fn transport_mirror(&self) -> &Arc<TransportMirror> {
        &self.transport_mirror
    }

    /// 控制侧读到的走带读数（**一致的一帧**，无锁）。
    #[must_use]
    pub fn transport_reading(&self) -> TransportReading {
        self.transport_mirror.read()
    }

    /// 本快照武装的**声相增益表**（诊断/判据用；`(轨道, 左, 右)`，前
    /// [`Self::armed_pan_slot_count`] 项有效）。
    ///
    /// 存在的理由与 `quanta_per_second` 同族：把"武装进去的那个数"变成**可读**的，
    /// 判据就不必从音频输出反推。实测价值：本线第一次接入声相时，"右声道拿到的
    /// 增益也是 1.0"这个 bug 用输出反推查了很久（`cos/sin` 与快照投影各自都对，
    /// 错在武装表），可读的武装表一句话就定位了。
    #[must_use]
    pub fn armed_pan_gain(&self, track: &EntityId) -> Option<(f32, f32)> {
        self.armed_pan_gains[..self.armed_pan_slots]
            .iter()
            .find(|(id, _, _)| id == track)
            .map(|(_, left, right)| (*left, *right))
    }

    /// 武装表里的轨道条数。
    #[must_use]
    pub const fn armed_pan_slot_count(&self) -> usize {
        self.armed_pan_slots
    }

    /// 播放头当前所在的绝对样本位置（0 = 工程 tick 0）。
    ///
    /// 走带是时钟的**唯一**事实源（[`crate::transport`]）：Running 时两者按同一
    /// `frames` 前进，因此 `position_samples() == transport().position_frames()`；
    /// 停住时两者都冻结。
    #[must_use]
    pub const fn position_samples(&self) -> u64 {
        self.synth.position()
    }

    /// 走带位置（960 PPQ 整数 tick）—— 判据/UI 的权威读数。
    #[must_use]
    pub const fn position_ticks(&self) -> u64 {
        self.transport.position_ticks()
    }

    /// 当前快照的模型层版本号（音频线程是否已追上模型线程）。
    #[must_use]
    pub fn revision(&self) -> Option<u64> {
        self.snapshot.held().map(|snapshot| snapshot.revision())
    }

    /// 最近一个量子的输出块（测试与诊断用；只读）。
    #[must_use]
    pub fn last_block(&self) -> &AudioBlock<DEFAULT_BLOCK_FRAMES> {
        &self.block
    }

    /// FTZ/DAZ 是否已在本线程生效 [ARCH-RT-003]。
    #[must_use]
    pub fn ftz_armed(&self) -> bool {
        self.ftz_ready
    }

    /// 每个回调线程**一次**：开启 FTZ/DAZ。
    ///
    /// 用普通 `bool` 标志而不是 `Once`：`Once` 在竞争路径上会**阻塞等待**，
    /// 那正是红线 7 禁止的锁等待。本标志只被音频线程读写，因此不需要原子量、
    /// 也不需要同步。控制寄存器是线程局部的，所以必须在这里（回调线程内）设置，
    /// 而不是在打开设备的主线程上 [ARCH-RT-003]。
    fn arm_fpu_once(&mut self) {
        if !self.ftz_ready {
            self.ftz = Some(fpu::enable_ftz_daz());
            self.ftz_ready = true;
        }
    }

    /// 渲染一个量子（**实时路径**：零分配、零锁、零 I/O）。
    ///
    /// ## 声相增益表为什么要在这里"先拷到栈上"
    ///
    /// 武装发生在**本函数内部**（快照修订变化时），而逐轨循环既要可变借用 `synth`、
    /// 又要读 `self` 的字段 —— 因此先把表拷成栈上的定长数组（`MAX_TRACK_SLOTS` = 16、
    /// 每项 24 字节 ⇒ 384 字节，`Copy`、无分配）。
    ///
    /// ⚠ 第一版是把拷贝放在**调用本函数之前**（另一个包装函数里）。那是一个**真 bug**：
    /// 武装发生在拷贝之后 ⇒ 第 1 个量子永远用**构造期的初值**（全居中），
    /// 于是"全左时右声道静音"这条判据实测拿到 `peak_r == peak_l`。
    /// 判据抓住了它（见 `docs/ledger/engine-mix-notes.md` 的事故记录）。
    fn render_block(&mut self, frames: usize) {
        // [MUST-GATE-001] **实时路径探针**：本量子确实经过了探针边界。
        //
        // 未武装时只做一次线程局部读取；武装时在此处对 `rt_probe::rt_path_lock()`
        // 做一次 **非阻塞** 试探（`try_lock`，不等待）并记进判据窗口 ——
        // 它是"探针真的有牙"的证据（见 `crate::rt_probe` 的模块文档）。
        // 零分配、零等待、零系统调用。
        rt_probe::quantum_enter();
        let Self {
            snapshot,
            events,
            meters,
            block,
            scratch_events,
            scratch_meters,
            track_scratch,
            bank,
            synth,
            transport,
            transport_mirror,
            limiter,
            limiter_gain_reductions,
            limiter_max_reduction,
            pdc_unarmed_nodes,
            pdc_clamped_frames,
            armed_revision,
            armed_scheduled_notes,
            armed_note_schedule_drops,
            quanta,
            events_applied,
            event_bulk_pops,
            meter_frames,
            meter_bulk_publishes,
            meter_capacity_drops,
            ..
        } = self;

        // 本量子的声相增益表（栈上定长；在下面的分支里可能被本次武装刷新）。
        // 只拷**已武装的**那几项 ⇒ 未武装的槽位保持"查不到 ⇒ 居中"的语义，
        // 同时避免把上一份快照的残留增益带进来。
        let mut pan_gains = [(EntityId::default(), 1.0f32, 1.0f32); MAX_TRACK_SLOTS];
        pan_gains[..self.armed_pan_slots]
            .copy_from_slice(&self.armed_pan_gains[..self.armed_pan_slots]);

        *quanta = quanta.wrapping_add(1);
        let quantum = *quanta;

        // --- 1) 参数/音符/**走带**事件：每块一次批量出队 [ROAD-M2-007] ---
        //
        // 走带命令在**量子边界**按 FIFO 顺序应用 ⇒ "同一输入序列 ⇒ 同一 tick 轨迹"
        // （确定性来自"命令在哪一个量子生效"只由出队顺序决定，与墙钟无关）。
        // `SeekTicks` 必须同时把合成器播放头挪到目标位置，否则"定位"只改数字、不出声。
        let mut applied = 0usize;
        events.drain_with(scratch_events, |event| {
            if !event.is_idle() {
                applied += 1;
            }
            if let EngineEvent::Transport { command } = event
                && let TransportEffect::Seeked { frames, .. } = transport.apply(command)
            {
                synth.seek(frames);
            }
        });
        *event_bulk_pops = event_bulk_pops.wrapping_add(1);
        *events_applied = events_applied.wrapping_add(applied as u64);

        // --- 2) 快照边界处的无锁切换 [ARCH-RT-002] ---
        let mut produced = 0usize;
        if let Some(current) = snapshot.begin_block() {
            // 母线（master）虽然通常也在 tracks() 里，但它是**总线**：单独出一帧
            // stereo-linked 的母线电平，绝不按"轨道"重复计量；它也不参与声部合成
            // （母线是汇流点，没有自己的声源）。
            let master = current.master();

            // 采样率/块长变了 ⇒ 电平弹道系数按新的"量子/秒"折算(保留电平状态)。
            let revision = current.revision();
            if *armed_revision != Some(revision) {
                // ⚠ 弹道系数必须跟随**实际的处理量子**（[`DEFAULT_BLOCK_FRAMES`] = 128, L1 契约钉死），
                // 而**不是**设备缓冲长度（`current.block_frames()` = 项目声明的 `audio_config.block_size`，
                // 例如 256）。两者的区别是真实的：`process_quantum` 会把任意长度的设备缓冲
                // **按 128 帧切成整量子**，所以一秒内的量子数是 `sample_rate / 128`。
                // 用 256 会把它算成一半 ⇒ 峰值保持按 10 dB/s 衰减而不是 20 dB/s
                // （由 `line/app-mixer` 交叉核对时发现；判据见 `meter_ballistics_follow_the_processing_quantum`）。
                let quanta_per_second = current.sample_rate() as f32 / DEFAULT_BLOCK_FRAMES as f32;
                bank.set_quanta_per_second(quanta_per_second);
                self.armed_quanta_per_second = Some(quanta_per_second);
                *armed_revision = Some(revision);

                // --- 2b) 声部池对齐到新快照的轨道集合（每修订一次, 非逐样本）---
                // 新轨道占槽、消失的轨道标记 absent（状态保留）、游标**只增不减**地校正
                // ⇒ 已触发过的音符绝不重复触发，在鸣的音符不被快照切换切断。
                synth.begin_snapshot(
                    current.sample_rate(),
                    current.tracks().keys().filter(|id| **id != master),
                    current.tones().iter().filter(|(id, _)| **id != master),
                );
                synth.align_cursors(current.schedules().iter().filter(|(id, _)| **id != master));
                // --- 2b') 走带武装：采样率与 BPM 必须来自**同一份快照**（见快照的 `bpm` 字段）。
                // 位置与状态都不动 ⇒ 换快照 / 改速度不跳变 [ARCH-DET-001]。
                transport.arm(current.sample_rate(), current.bpm());
                *armed_scheduled_notes = current.scheduled_notes() as u64;
                *armed_note_schedule_drops = current.note_schedule_drops();

                // --- 2b'') PDC：把这一份快照的补偿计划武装进**预分配**延迟线池 ---
                // [ARCH-PDC-001, ROAD-M2-004] 规范 §3.4 第 3 条：
                // "对于累积延迟为 L_i 的并行分支, 在进入总线求和节点前自动插入
                //   D_i = L_max − L_i 采样点的环形延迟缓冲 (PDC Delay Line)"。
                // 计划在**控制线程**（`EngineSnapshot::from_project` → `PdcPlan::compute`）
                // 算好，这里只做 `set_delay`：**零分配、零锁、零 I/O** [MUST-GATE-001]。
                // 池装不下的部分不静默：记进两个累计读数（见 `RearmShortfall`）。
                let shortfall = self.pdc.rearm(current.pdc());
                *pdc_unarmed_nodes = pdc_unarmed_nodes.wrapping_add(shortfall.unarmed_nodes as u64);
                *pdc_clamped_frames = pdc_clamped_frames.wrapping_add(shortfall.clamped_frames);

                // --- 2c) 声相增益：在**构造期语义**下算一次（`cos`/`sin` 属超越函数类,
                // 不进逐样本路径）。`pan_law` 与 `pan` 在整份快照的生命周期内不变。
                // 表先写进 `self`（权威副本，诊断可读），再刷新栈上那份 —— 顺序无所谓，
                // 但**必须在本量子的逐轨循环之前**（第一版的顺序错误见函数文档）。
                self.armed_pan_law = current.pan_law();
                self.armed_pan_slots = 0;
                for (id, params) in current.tracks() {
                    if *id == master || self.armed_pan_slots >= MAX_TRACK_SLOTS {
                        continue;
                    }
                    let (gain_l, gain_r) = params.pan_gains(self.armed_pan_law);
                    let slot = self.armed_pan_slots;
                    self.armed_pan_gains[slot] = (*id, gain_l, gain_r);
                    self.armed_pan_slots += 1;
                }
                pan_gains[..self.armed_pan_slots]
                    .copy_from_slice(&self.armed_pan_gains[..self.armed_pan_slots]);
            }

            block.silence();
            block.set_frames(frames);
            bank.begin_quantum();

            // --- 3a) 逐轨：渲染 → 电平 → 汇入母线 ---
            //
            // **走带冻结 ⇒ 输出静音、且不触发任何音符**（`notes` 契约见
            // `docs/ledger/engine-sound-notes.md` 的 needs N2）。电平仍然照常计量
            // （静音的读数），因此"每量子恰好一次批量发布"这条结构性契约与走带状态无关。
            let running = transport.is_playing();
            let metered_tracks = current.tracks().keys().filter(|id| **id != master).count();
            // 给母线留一个槽位, 保证母线永远有电平可发。
            let track_budget = scratch_meters.len().saturating_sub(1);
            if metered_tracks > track_budget {
                *meter_capacity_drops =
                    meter_capacity_drops.wrapping_add((metered_tracks - track_budget) as u64);
                // [MUST-GATE-001, N6 选项 A] 诊断走 `rt_probe::note_suppressed` ——
                // 它是**纯计数**出口（不读 sink、不分配、不加锁、不阻塞），
                // 因为本函数就是音频回调：实时路径上不许有任何到 I/O 边界的调用。
                rt_probe::note_suppressed(RtDiagEvent::MeterCapacityDrop);
            }
            for id in current.tracks().keys() {
                if *id == master || produced >= track_budget {
                    continue;
                }
                let track = *id;
                if running {
                    // 真的合成：快照里的音符调度表 → 声部池 → 单声道、声相之前的样本。
                    // 参数/音符的投影全部在控制线程完成；这里只有整数相位递推、
                    // 线性插值与 ADSR（全是 IEEE 精确类运算，见 `synth` 模块文档 §2）。
                    synth.render_track(
                        track,
                        current.schedule(&track),
                        &mut track_scratch[..frames],
                    );
                } else {
                    // 停住：不碰声部池（不触发、不推进、不窃取），只把静音喂给电平表。
                    track_scratch[..frames].fill(0.0);
                }
                if let Some(frame) = bank.measure(track, quantum, &track_scratch[..frames]) {
                    scratch_meters[produced] = frame;
                    produced += 1;
                } else {
                    *meter_capacity_drops = meter_capacity_drops.wrapping_add(1);
                    rt_probe::note_suppressed(RtDiagEvent::MeterCapacityDrop);
                }
                // --- PDC：本轨输出 → 补偿延迟线 → 声相/母线求和 ---
                // [ARCH-PDC-001, ROAD-M2-004] 位置就是规范 §3.4 第 3 条说的
                // "在进入总线求和节点前"。延迟量在快照边界武装好（见 2b''），
                // 这里只做环形读写：**零分配、逐样本无分支**（`delay == 0` 时
                // `process_in_place` 直接返回，是显式直通快路径）。
                //
                // ⚠ 刻意放在 `bank.measure` **之后**：逐轨电平的取样点因此**一位没动**
                // （仍是"该轨自己渲染出来的、声相之前的单声道结果"），PDC 是**求和节点
                // 输入侧**的事 —— 这正是规范那句话的位置。放到电平之前也能对齐相位，
                // 但那样会顺带改掉电平的读数口径。
                //
                // 走带停住时**也**喂延迟线（喂的是静音）：延迟线是时间状态，
                // 不推进它会让恢复播放时吐出上一次停住前的陈音频。
                self.pdc.apply(&track, &mut track_scratch[..frames]);
                // 汇入立体声母线：按本轨的**声相增益**分别写 L/R
                // （构造期算好的 `cos/sin`，见 `mixer` 模块文档 §1）。
                // 找不到该轨的增益（超出 `MAX_TRACK_SLOTS`）时按**居中**处理，
                // 而不是静音 —— 宁可声相不准，也不要一条轨无声。
                let (gain_l, gain_r) = pan_gains.iter().find(|(id, _, _)| *id == track).map_or(
                    (
                        core::f32::consts::FRAC_1_SQRT_2,
                        core::f32::consts::FRAC_1_SQRT_2,
                    ),
                    |(_, l, r)| (*l, *r),
                );
                sum_into_bus(block, &track_scratch[..frames], gain_l, gain_r);
            }
            // 播放头前进：**每个量子一次**（与轨道数无关）。
            //
            // 走带是**唯一**的时钟事实源：`Running` 时合成器与走带位置同步前进
            // （两者的绝对原点都是 tick 0 ⇒ 帧 / tick 两条读数描述同一瞬间）；
            // `Stopped` 时**两者都不动**（[`Transport::advance_frames`] 恒返回 0）。
            if running {
                transport.advance_frames(frames as u64);
                synth.advance(frames);
            }

            // --- 3b) 母线限制器（[ARCH-DSP-001]）：逐轨汇流之后、母线电平之前 ---
            // 前瞻式峰值限制、立体声联动、逐样本确定（`mixer` 模块文档 §2–§4）。
            let before = limiter.reduction_count();
            limiter.apply(block, frames);
            let reduced = limiter.reduction_count().saturating_sub(before);
            if reduced > 0 {
                *limiter_gain_reductions = limiter_gain_reductions.wrapping_add(reduced);
            }
            let reduction = 1.0 - limiter.gain();
            if reduction > *limiter_max_reduction {
                *limiter_max_reduction = reduction;
            }

            // --- 3c) 母线：立体声联动电平（**限制之后**） ---
            if produced < scratch_meters.len() {
                scratch_meters[produced] =
                    bank.measure_bus_stereo(master, quantum, block.left(), block.right());
                produced += 1;
            } else {
                *meter_capacity_drops = meter_capacity_drops.wrapping_add(1);
                rt_probe::note_suppressed(RtDiagEvent::MeterCapacityDrop);
            }
        } else {
            // 极端情况（写者尚未发布任何快照）：输出静音但绝不 panic。
            block.silence();
            block.set_frames(frames);
            rt_probe::note_suppressed(RtDiagEvent::NoSnapshot);
        }
        snapshot.end_block();

        // --- 4) 电平：**每量子恰好一次**批量推送（本量子的全部帧）[ROAD-M2-008] ---
        if produced > 0 {
            let published = meters.publish(&scratch_meters[..produced]);
            *meter_frames = meter_frames.wrapping_add(published as u64);
            *meter_bulk_publishes = meter_bulk_publishes.wrapping_add(1);
        }

        // --- 5) 走带读数发布：**每量子恰好一次**（原子写，无锁无分配） ---
        //
        // 无快照时也发布：控制面要能读到"命令已被应用、状态确实变了"，
        // 而不是靠猜。写者在任何情况下都不会等待读者。
        transport.publish(transport_mirror.as_ref());
    }
}

/// 母线汇流：把一条轨的**单声道、声相之前**渲染结果按声相增益写进左右两声道。
///
/// `(gain_l, gain_r)` 由 [`crate::mixer::pan_gains`] 在**构造期**算出
/// （`cos`/`sin` 属超越函数类，不进实时路径），实时侧这里只做两次乘加。
/// 居中（默认律）时 `(√2/2, √2/2)` ⇒ 每声道 −3.01 dB，与 `yeban-mcp` 的离线渲染同口径。
///
/// 仍然是**占位**的部分（本切片没做，见 notes 的 needs）：发送/辅助汇流、
/// 路由边的 `gain_db`（`RoutingEdge::gain_db` 目前被忽略）。
/// PDC 延迟线对齐**已接线**（调用点见 [`EngineRuntime::render_block`] 步骤 3a'）。
fn sum_into_bus(
    block: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    mono: &[f32],
    gain_l: f32,
    gain_r: f32,
) {
    // 两个切片都由 `stereo_mut()` 按有效帧数给出, `zip` 天然按较短者截断。
    let (left, right) = block.stereo_mut();
    for ((l, r), m) in left.iter_mut().zip(right.iter_mut()).zip(mono) {
        *l += *m * gain_l;
        *r += *m * gain_r;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::LatencyTable;
    use crate::meter::meter_channel;
    use crate::ring::event_channel;
    use crate::snapshot::{EngineSnapshot, TrackParams, retire_channel};
    use std::collections::BTreeMap;
    use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind, TrackV3};

    /// 轨道 id 升序的前 n 个（用于断言确定性顺序）。
    fn simple_snapshot(revision: u64) -> EngineSnapshot {
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
        let track_model = TrackV3 {
            id: track,
            ..TrackV3::default()
        };
        tracks.insert(
            track,
            crate::snapshot::TrackParams::from_track(&track_model, 0),
        );
        EngineSnapshot::from_parts(
            revision,
            48_000,
            DEFAULT_BLOCK_FRAMES,
            2,
            master,
            tracks,
            &routing,
            &LatencyTable::new(),
        )
        .expect("合法图")
    }

    /// 母线**也在** `tracks()` 里的快照（`from_project` 的常态）：
    /// `master` + `track_count` 条普通轨。
    fn snapshot_with_master_track(revision: u64, track_count: usize) -> EngineSnapshot {
        let master = EntityId::new();
        let mut nodes = vec![master];
        let mut routing = RoutingGraph {
            nodes: Vec::new(),
            ..RoutingGraph::default()
        };
        let mut tracks: BTreeMap<EntityId, TrackParams> = BTreeMap::new();
        let master_model = TrackV3 {
            id: master,
            ..TrackV3::default()
        };
        tracks.insert(
            master,
            crate::snapshot::TrackParams::from_track(&master_model, 0),
        );
        for _ in 0..track_count {
            let track = EntityId::new();
            nodes.push(track);
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
            let model = TrackV3 {
                id: track,
                ..TrackV3::default()
            };
            tracks.insert(track, TrackParams::from_track(&model, 0));
        }
        routing.nodes = nodes;
        EngineSnapshot::from_parts(
            revision,
            48_000,
            DEFAULT_BLOCK_FRAMES,
            2,
            master,
            tracks,
            &routing,
            &LatencyTable::new(),
        )
        .expect("合法图")
    }

    struct Rig {
        slot: Arc<SnapshotSlot>,
        queue: crate::snapshot::RetireQueue,
        sender: crate::ring::EventSender,
        collector: crate::meter::MeterCollector,
        runtime: EngineRuntime,
    }

    fn rig() -> Rig {
        rig_with_retire_capacity(16)
    }

    /// 指定**退役队列容量**的装配：容量 1 是"制造追不上"的标准夹具
    /// （读者一旦要交回第二份旧快照就必须寄存 ⇒ 停止切换）。
    fn rig_with_retire_capacity(retire_capacity: usize) -> Rig {
        let slot = SnapshotSlot::new(simple_snapshot(1));
        let (retire, queue) = retire_channel(retire_capacity);
        let (sender, receiver) = event_channel(64);
        let (publisher, collector) = meter_channel(256);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Rig {
            slot,
            queue,
            sender,
            collector,
            runtime,
        }
    }

    /// `[ARCH-UI-002]` 弹道系数必须跟随**处理量子（128）**而不是**设备缓冲（项目声明的 block_size）**。
    ///
    /// 实测背景（`line/app-mixer` 交叉核对时发现）：`process_quantum` 把任意长度的设备缓冲按
    /// [`DEFAULT_BLOCK_FRAMES`] 切成整量子，而旧实现用 `snapshot.block_frames()`（项目声明的 256）
    /// 折算"每秒量子数" ⇒ 峰值保持按 **10 dB/s** 衰减而不是契约要求的 **20 dB/s**（整整差 2 倍）。
    /// 这种错不会 panic、也不会让既有判据变红 —— 它只会让表头**慢慢变得不准**。
    ///
    /// 判据直接钉住"武装进去的那个数"（`EngineStats::quanta_per_second`）：
    /// 无论工程声明的设备缓冲是 256 还是 128，**处理量子恒为 128** ⇒ 每秒量子数恒为 `48000/128 = 375`。
    #[test]
    fn meter_ballistics_follow_the_processing_quantum() {
        // 与 `rig()` 相同, 但显式声明**设备缓冲** = 256（≠ 处理量子 128）。
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
            TrackParams::from_track(
                &TrackV3 {
                    id: track,
                    ..TrackV3::default()
                },
                0,
            ),
        );

        for declared_block in [256_usize, 128] {
            let snapshot = EngineSnapshot::from_parts(
                1,
                48_000,
                declared_block,
                2,
                master,
                tracks.clone(),
                &routing,
                &LatencyTable::new(),
            )
            .expect("合法图");
            assert_eq!(snapshot.block_frames(), declared_block);

            let slot = SnapshotSlot::new(snapshot);
            let (retire, _queue) = retire_channel(16);
            let (_sender, receiver) = event_channel(64);
            let (publisher, _collector) = meter_channel(256);
            let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
            assert_eq!(
                runtime.stats().quanta_per_second,
                None,
                "还没处理过快照时应当是 None"
            );

            let mut output = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            runtime.process_quantum(&mut output, 2);

            assert_eq!(
                runtime.stats().quanta_per_second,
                Some(375.0),
                "声明 {declared_block} 的设备缓冲时, 每秒量子数仍须是 48000/{DEFAULT_BLOCK_FRAMES} = 375 \
                 —— 旧实现会给出 {} （弹道按 10 dB/s 衰减而不是 20 dB/s）",
                48_000.0 / declared_block as f32,
            );
        }
    }

    /// 编译期判据：渲染驱动必须能 move 进 cpal 的回调闭包（`D: Send + 'static`）。
    ///
    /// `NullBackend` 属于 `device` feature（它的配置类型含 cpal 类型），
    /// 因此那条断言按 feature 门控 [ADR-0001 D19]。
    #[test]
    fn runtime_is_send_so_it_can_move_into_the_audio_callback() {
        fn assert_send<T: Send>() {}
        assert_send::<EngineRuntime>();
        #[cfg(feature = "device")]
        assert_send::<crate::device::NullBackend>();
    }

    #[test]
    fn null_path_renders_silence_and_counts_quanta() {
        let mut rig = rig();
        let mut out = [1.5f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0), "占位渲染必须输出静音");
        let stats = rig.runtime.stats();
        assert_eq!(stats.quanta, 1);
        assert_eq!(stats.event_bulk_pops, 1, "每量子一次批量出队");
        assert_eq!(stats.meter_frames, 2, "一条音轨 + 一条 master");
        assert_eq!(stats.meter_bulk_publishes, 1, "每量子恰好一次批量发布");
        assert_eq!(stats.meter_capacity_drops, 0);
        assert!(rig.runtime.ftz_armed());
        assert_eq!(rig.runtime.revision(), Some(1));
    }

    #[test]
    fn partial_and_multi_quantum_buffers_are_split_at_the_boundary() {
        let mut rig = rig();
        // 非整块: 200 帧 = 128 + 72
        let mut out = [0.5f32; 400];
        rig.runtime.process_quantum(&mut out, 2);
        assert_eq!(rig.runtime.stats().quanta, 2, "200 帧必须切成两个量子");
        assert_eq!(rig.runtime.stats().event_bulk_pops, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(rig.runtime.last_block().frames(), 72);
    }

    #[test]
    fn events_are_drained_once_per_block_and_applied() {
        let mut rig = rig();
        let target = crate::ring::ParamAddress::new(EntityId::new(), 3);
        let batch: Vec<EngineEvent> = (0..5)
            .map(|i| EngineEvent::SetParam {
                target,
                value: i as f32,
            })
            .collect();
        assert_eq!(rig.sender.publish(&batch), 5);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let stats = rig.runtime.stats();
        assert_eq!(stats.events_applied, 5);
        assert_eq!(stats.event_bulk_pops, 1, "5 个事件仍然只调用一次批量 API");
    }

    #[test]
    fn snapshot_swap_moves_old_arc_into_retire_queue_and_never_drops_in_rt() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);

        rig.slot.publish(simple_snapshot(2));
        rig.slot.publish(simple_snapshot(3));
        rig.runtime.process_quantum(&mut out, 2);

        assert_eq!(rig.runtime.revision(), Some(3), "音频线程追上了最新快照");
        assert_eq!(
            rig.runtime.stats().snapshot_switches,
            1,
            "两次 publish 只切一次"
        );
        assert_eq!(rig.queue.pending(), 1, "旧快照被 move 进退役队列");
        assert_eq!(rig.queue.drain(4), 1, "主线程负责 Drop");
        assert!(rig.slot.prune() >= 1, "读者推进后写者侧清单也可回收");
    }

    #[test]
    fn meter_frames_are_published_per_track_with_bulk_api() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..5 {
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut scratch = [MeterFrame::default(); 64];
        let drained = rig.collector.tick(&mut scratch);
        assert_eq!(drained, 10, "5 个量子 × 2 条计量");
        assert_eq!(rig.collector.bulk_pop_calls(), 1, "UI 一次 tick 一次批量读");
        assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));
        assert!(scratch[..drained].iter().all(MeterFrame::is_sane));
        // 量子序号必须递增（UI 用它做新鲜度判断）
        assert!(scratch[0].quantum < scratch[2].quantum);
        let stats = rig.runtime.stats();
        assert_eq!(stats.meter_frames, 10);
        assert_eq!(stats.meter_bulk_publishes, 5, "每个量子恰好一次批量发布");
    }

    /// 判据：**每量子发布帧数 == 非母线轨数 + 1 条母线**，
    /// 且母线即使在 `tracks()` 里也**不重复计量**。
    #[test]
    fn bus_is_metered_once_even_when_master_is_in_the_track_map() {
        let slot = SnapshotSlot::new(snapshot_with_master_track(1, 3));
        let (retire, _queue) = retire_channel(16);
        let (_sender, receiver) = event_channel(64);
        let (publisher, mut collector) = meter_channel(256);
        let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        runtime.process_quantum(&mut out, 2);

        let mut scratch = [MeterFrame::default(); 16];
        let drained = collector.tick(&mut scratch);
        assert_eq!(drained, 4, "3 条普通轨 + 1 条母线");
        let stats = runtime.stats();
        assert_eq!(stats.meter_frames, 4);
        assert_eq!(stats.meter_bulk_publishes, 1);

        // 节点顺序 = BTreeMap 升序的非母线轨, 最后是母线; 母线只出现一次
        let master = slot.current().master();
        let expected: Vec<EntityId> = {
            let snapshot = slot.current();
            let mut ids: Vec<EntityId> = snapshot
                .tracks()
                .keys()
                .copied()
                .filter(|id| *id != master)
                .collect();
            ids.push(master);
            ids
        };
        let seen: Vec<EntityId> = scratch[..drained].iter().map(|f| f.node).collect();
        assert_eq!(seen, expected, "节点顺序必须确定(轨道升序 + 母线)");
        assert_eq!(
            seen.iter().filter(|id| **id == master).count(),
            1,
            "母线不得被当成普通轨再计一次"
        );
    }

    /// 判据：静音输入的每一帧都必须是**有限**且峰值 0（dBFS = 负无穷），没有 NaN。
    #[test]
    fn silent_input_yields_finite_silent_frames() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let mut scratch = [MeterFrame::default(); 8];
        let drained = rig.collector.tick(&mut scratch);
        assert_eq!(drained, 2);
        for frame in &scratch[..drained] {
            assert!(frame.is_sane(), "静音不得产生 NaN: {frame:?}");
            assert_eq!(frame.peak, 0.0);
            assert_eq!(frame.peak_hold, 0.0);
            assert_eq!(frame.peak_dbfs(), f32::NEG_INFINITY);
            assert_eq!(
                frame.peak_dbfs_clamped(crate::level::SILENCE_FLOOR_DBFS),
                crate::level::SILENCE_FLOOR_DBFS
            );
            assert_eq!(frame.quantum, 1);
        }
    }

    /// 判据：母线电平是**单声道视图不可见**的那一路也参与联动
    /// （这里用 `sum_into_bus` 直接验证汇流路径本身）。
    #[test]
    fn sum_into_bus_is_additive_and_bounded_by_valid_frames() {
        let mut block = AudioBlock::<DEFAULT_BLOCK_FRAMES>::new();
        block.set_frames(4);
        sum_into_bus(&mut block, &[0.25, -0.5, 1.0, 0.0], 1.0, 1.0);
        assert_eq!(block.left(), &[0.25f32, -0.5, 1.0, 0.0][..]);
        assert_eq!(block.right(), &[0.25f32, -0.5, 1.0, 0.0][..]);
        // 超长输入只按有效帧数累加, 不越界
        sum_into_bus(&mut block, &[1.0; DEFAULT_BLOCK_FRAMES], 1.0, 1.0);
        assert_eq!(block.left()[0], 1.25);
        assert_eq!(block.left().len(), 4);
    }

    /// 判据：轨道数超过电平状态容量时**不 panic、不扩容**，而是计数并保留母线。
    #[test]
    fn oversized_track_set_is_counted_and_never_panics() {
        let slot = SnapshotSlot::new(snapshot_with_master_track(1, SCRATCH_METERS + 44));
        let (retire, _queue) = retire_channel(16);
        let (_sender, receiver) = event_channel(64);
        let (publisher, _collector) = meter_channel(1024);
        let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        runtime.process_quantum(&mut out, 2);
        let stats = runtime.stats();
        // 300 条轨: 255 条进槽(给母线留 1), 余 45 条计入容量丢弃
        assert_eq!(stats.meter_capacity_drops, 45);
        assert_eq!(stats.meter_frames, SCRATCH_METERS as u64);
        assert_eq!(stats.meter_bulk_publishes, 1);
    }

    #[test]
    fn odd_channel_counts_and_empty_buffers_do_not_panic() {
        let mut rig = rig();
        let mut out = [0.7f32; 5]; // 5 个样本 / 2 声道 = 2 帧 + 1 个残余样本
        rig.runtime.process_quantum(&mut out, 2);
        assert_eq!(rig.runtime.stats().quanta, 1);
        // 残余样本保持原值（cpal 会预填静音, 这里只断言没有越界写）
        assert_eq!(out[4], 0.7);
        let mut empty: [f32; 0] = [];
        rig.runtime.process_quantum(&mut empty, 2);
        assert_eq!(rig.runtime.stats().quanta, 1, "空缓冲不产生量子");
    }

    // -----------------------------------------------------------------------
    // 控制面健康读数（`needs` N2/N5 的落地）：判据 ①~⑥
    // -----------------------------------------------------------------------
    //
    // 交付清单与读数见 `docs/ledger/engine-stats-notes.md`。这里的判据刻意都用
    // **既有夹具**（`rig()` / `rig_with_retire_capacity` / 既有 60Hz 排空语义），
    // 注入记录（3 组）也在那份 notes 里。

    /// ⑥ 计数的"大数行为"必须**写明并断言**（不是靠"跑不到那么大"）。
    ///
    /// `quanta` / `events_applied` / `meter_frames` 等既有计数是结构性计数，
    /// 用 `wrapping_add`（u64 全宽）；本 crate 钉死 `DEFAULT_BLOCK_FRAMES = 128`
    /// ⇒ 48 kHz 下**每秒恰好 375 个量子**。这个编译期断言说明：即使 24 小时不停
    /// 渲染，回绕也需要 **> 10 亿年** ⇒ `wrapping_add` 与饱和加法在物理上等价。
    const _: () = {
        let quanta_per_second: u64 = 375;
        let seconds_per_year: u64 = 365 * 24 * 60 * 60;
        let years_to_wrap: u64 = u64::MAX / quanta_per_second / seconds_per_year;
        assert!(years_to_wrap > 1_000_000_000);
    };

    /// 判据 ①：正常播放（队列容量充足、控制面照常排空）⇒ `stash == 0`、判定为不 lagging，
    /// 且两条回收路径（读者交回的 `drain` / 写者 anchor 淘汰的 `prune`）**都真的在动**。
    #[test]
    fn stats_report_a_healthy_retire_pipeline_during_normal_playback() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let before = rig.runtime.stats();

        for revision in 2..=5u64 {
            rig.slot.publish(simple_snapshot(revision));
            rig.runtime.process_quantum(&mut out, 2);
        }
        // 主线程 60Hz 排空语义：drain（读者交回的那一份）+ prune（写者 anchor 淘汰的那一份）。
        let drained = rig.queue.drain(16);
        let pruned = rig.slot.prune();
        let stats = rig.runtime.stats();

        assert_eq!(stats.snapshot_switches, 4, "4 次发布 = 4 次切换");
        assert_eq!(drained, 4, "每次切换都把上一份交回退役队列");
        assert_eq!(pruned, 4, "写者侧清单同步回收");
        assert_eq!(stats.retire_drained, 4);
        assert_eq!(stats.retire_drain_calls, 1, "非空 drain 恰好一次");
        assert_eq!(stats.retire_pruned, 4);
        assert_eq!(stats.retire_pending, 0, "排空后队列空");
        assert!(!stats.has_retire_backlog());

        assert_eq!(stats.snapshot_stash_events, 0, "正常路径不得出现寄存");
        assert!(!stats.is_snapshot_lagging());
        assert_eq!(stats.stash_events_since_last_read(&before), 0);
        assert!(!stats.is_snapshot_lagging_since(&before));
        assert!(stats.release_thread_is_main, "本线程建队列、本线程 drain");
        assert_eq!(stats.foreign_drains, 0);
    }

    /// 判据 ②：**制造追不上**（既有夹具：容量 1 的退役队列 + 控制面不排空）。
    ///
    /// 实测语义（这就是 `line/gate-snapshot-churn` 注入 I2 的那个行为）：
    /// 队列满 ⇒ 读者把旧快照寄存进 `stash` 并**停止切换快照**，
    /// 控制侧"追上新 revision"**静默失败** —— 现在这件事在 [`EngineStats`] 里可判定。
    #[test]
    fn stats_flag_snapshot_lagging_when_the_reader_cannot_keep_up() {
        let mut rig = rig_with_retire_capacity(1);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let healthy = rig.runtime.stats();
        assert_eq!(healthy.snapshot_stash_events, 0);

        for revision in 2..=6u64 {
            rig.slot.publish(simple_snapshot(revision));
            rig.runtime.process_quantum(&mut out, 2);
        }
        let lagging = rig.runtime.stats();

        assert_eq!(
            lagging.snapshot_stash_events, 1,
            "容量 1 + 不排空 ⇒ 恰好发生一次寄存（第二次开始连寄存都做不了，只能停切换）"
        );
        assert!(lagging.is_snapshot_lagging(), "累计判定必须为真");
        assert_eq!(lagging.stash_events_since_last_read(&healthy), 1);
        assert!(lagging.is_snapshot_lagging_since(&healthy));
        assert_eq!(lagging.retire_pending, 1, "退役队列被那一份寄存占满");
        assert!(lagging.has_retire_backlog());
        // **控制面看不见的那件事**：读者停在 revision 3，而槽里已经是 6 ——
        // 而且音频线程**不会**报错：`stash_events` 是唯一的可见信号。
        assert_eq!(rig.runtime.revision(), Some(3), "寄存之后读者停止切换");
        assert_ne!(rig.runtime.revision(), Some(6));

        // 控制面**降速**：先排空一次 ⇒ 读者恢复推进并最终追上最新 revision。
        assert_eq!(rig.queue.drain(1), 1);
        rig.runtime.process_quantum(&mut out, 2);
        let recovered = rig.runtime.stats();
        assert_eq!(
            rig.runtime.revision(),
            Some(6),
            "排空之后追上了最新 revision"
        );
        assert!(
            recovered.snapshot_stash_events >= lagging.snapshot_stash_events,
            "累计量绝不回退"
        );
        assert_eq!(
            recovered.snapshot_stash_events, 2,
            "容量 1 时'追上'本身还要再寄存一次（读者必须交回滞留的那一份）—— \
             所以说降速 = 排空 + 延后发布，不是'排一次就够'"
        );
        assert_eq!(recovered.retire_drained, 1);
    }

    /// 判据 ② 的边界：读到一个**更旧**的基线时，增量读数不得回绕（饱和减法语义）。
    #[test]
    fn stash_delta_readings_never_underflow_on_a_stale_baseline() {
        let mut rig = rig_with_retire_capacity(1);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        for revision in 2..=4u64 {
            rig.slot.publish(simple_snapshot(revision));
            rig.runtime.process_quantum(&mut out, 2);
        }
        let lagging = rig.runtime.stats();
        assert!(lagging.is_snapshot_lagging());

        let cold = EngineStats::default();
        assert_eq!(
            lagging.stash_events_since_last_read(&cold),
            lagging.snapshot_stash_events
        );
        assert_eq!(
            cold.stash_events_since_last_read(&lagging),
            0,
            "更旧的基线 ⇒ 0（不是 u64 回绕）"
        );
        assert!(!cold.is_snapshot_lagging_since(&lagging));
        assert!(!cold.is_snapshot_lagging());
    }

    /// 判据 ③：`EngineStats` 的退役读数与**队列自己的**读数**逐项一致**
    /// （与 `gate-snapshot-churn` 的 `release_thread_is_main` / `foreign_drains` 同口径）。
    #[test]
    fn stats_retire_readings_match_the_queue_item_by_item() {
        let mut rig = rig_with_retire_capacity(8);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        for revision in 2..=6u64 {
            rig.slot.publish(simple_snapshot(revision));
            rig.runtime.process_quantum(&mut out, 2);
        }
        let drained = rig.queue.drain(2);
        let pruned = rig.slot.prune();
        let stats = rig.runtime.stats();
        let main = std::thread::current().id();

        assert_eq!(drained, 2);
        assert!(pruned >= 1, "写者侧清单也必须真的回收（实际 {pruned}）");
        assert_eq!(stats.retire_drained, rig.queue.dropped(), "累计回收条数");
        assert_eq!(stats.retire_drain_calls, rig.queue.drain_calls());
        assert_eq!(stats.retire_pending, rig.queue.pending() as u64);
        assert_eq!(stats.retire_pruned, rig.slot.pruned());
        assert_eq!(
            stats.release_thread_is_main,
            rig.queue.release_thread_is_main(),
            "两个读数必须同一个口径"
        );
        assert_eq!(
            stats.release_thread_is_main,
            rig.queue.release_thread() == Some(main),
            "与 gate-snapshot-churn 的判据逐字相同"
        );
        assert_eq!(stats.foreign_drains, rig.queue.foreign_drains());
        assert_eq!(stats.snapshot_stash_events, 0);
        assert_eq!(
            stats.snapshot_stash_events,
            rig.runtime.snapshot_stash_events()
        );
        assert!(stats.release_thread_is_main && stats.foreign_drains == 0);
    }

    /// 判据 ③ 的**反例**（同时是注入判据的靶子）：队列在 A 线程建、**首个**释放发生在
    /// B 线程 ⇒ `release_thread_is_main == false`，而 `foreign_drains` 仍然是 `0`
    /// （它数的是"非首个释放线程"，不是"非归属线程"）。
    ///
    /// 这条反例证明两件事：① 归属布尔量**不是** `foreign_drains == 0` 的别名；
    /// ② 把 `release_thread_is_main` 写死为 `true` 会被本判据抓住（notes 的注入 E2；
    /// 正常路径的判据**抓不住**它 —— 两边会一起撒谎）。
    #[test]
    fn stats_report_a_foreign_release_thread_while_foreign_drains_stays_zero() {
        let Rig {
            slot,
            queue,
            runtime,
            ..
        } = rig();
        let mut runtime = runtime;
        let mut queue = queue;
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        runtime.process_quantum(&mut out, 2);
        slot.publish(simple_snapshot(2));
        runtime.process_quantum(&mut out, 2);
        assert_eq!(queue.pending(), 1, "队列里有一条待回收");

        // 首个非空 drain 交给**另一个线程**：`release_thread != owner_thread`。
        let (drained, mut queue) = std::thread::spawn(move || {
            let count = queue.drain(8);
            (count, queue)
        })
        .join()
        .expect("排空线程不 panic");
        assert_eq!(drained, 1);

        let main = std::thread::current().id();
        assert_ne!(queue.release_thread(), Some(main), "首个释放线程是外线程");
        assert!(!queue.release_thread_is_main(), "归属不成立");
        assert_eq!(
            queue.foreign_drains(),
            0,
            "`foreign_drains` 只数'非首个释放线程'上的 drain ⇒ 首个外线程 drain 不算"
        );
        let stats = runtime.stats();
        assert!(!stats.release_thread_is_main, "统计面必须看见归属被破坏");
        assert_eq!(stats.foreign_drains, 0);
        assert_eq!(stats.retire_drained, 1);
        assert_eq!(stats.retire_pending, 0);

        // 再在主线程排空一次 ⇒ 这一次相对"首个释放线程"属于外来 ⇒ `foreign_drains == 1`。
        slot.publish(simple_snapshot(3));
        runtime.process_quantum(&mut out, 2);
        assert_eq!(queue.drain(8), 1);
        let after = runtime.stats();
        assert_eq!(
            after.foreign_drains, 1,
            "主线程这次 drain 相对首个释放线程是外来"
        );
        assert_eq!(after.foreign_drains, queue.foreign_drains());
        assert!(!after.release_thread_is_main, "归属一旦被破坏不会自愈");
        assert_eq!(after.retire_drained, 2);
    }

    /// 判据 ⑤：`stats()` 是**只读快照**——同一瞬间两次读取完全相等；
    /// 引擎继续跑之后，累计量只增不减；`retire_pending` 是**量规**（明确可回落）。
    #[test]
    fn stats_are_snapshots_and_never_go_backwards_between_reads() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let idle = rig.runtime.stats();
        assert_eq!(idle, rig.runtime.stats(), "读 stats() 不改变引擎状态");

        for revision in 2..=4u64 {
            rig.slot.publish(simple_snapshot(revision));
            rig.runtime.process_quantum(&mut out, 2);
        }
        let backed_up = rig.runtime.stats();
        assert!(
            backed_up.retire_pending >= 3,
            "3 份旧快照还在队列里（实际 {}）",
            backed_up.retire_pending
        );
        assert!(backed_up.has_retire_backlog());

        let drained = rig.queue.drain(16);
        assert_eq!(drained, 3);
        rig.slot.prune();
        // 引擎继续跑一个量子（**没有**新发布 ⇒ 不再切换、不再入队）。
        rig.runtime.process_quantum(&mut out, 2);
        let after_drain = rig.runtime.stats();

        // ---- 累计量：单调不减（这是"不回退"的主体）----
        assert!(after_drain.quanta > backed_up.quanta);
        assert!(after_drain.snapshot_switches >= backed_up.snapshot_switches);
        assert!(after_drain.events_applied >= backed_up.events_applied);
        assert!(after_drain.meter_frames >= backed_up.meter_frames);
        assert!(after_drain.retire_drained >= backed_up.retire_drained);
        assert!(after_drain.retire_drain_calls >= backed_up.retire_drain_calls);
        assert!(after_drain.retire_pruned >= backed_up.retire_pruned);
        assert!(after_drain.snapshot_stash_events >= backed_up.snapshot_stash_events);
        // ---- 量规：明确可回落（这就是那条"明确的重置语义"）----
        assert_eq!(idle.retire_pending, 0);
        assert_eq!(after_drain.retire_pending, 0, "排空让量规回落");
        assert!(!after_drain.has_retire_backlog());
        // ---- 增量读数与累计量自洽 ----
        assert_eq!(
            after_drain.stash_events_since_last_read(&idle),
            after_drain.snapshot_stash_events - idle.snapshot_stash_events
        );
    }
}
