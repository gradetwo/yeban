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
//! 2) 快照：begin_block() 无锁切换 [ARCH-RT-002]；revision 变化时
//!       a) 重设电平弹道系数；b) 声部池对齐到新轨道集合 + 游标校正（只增不减）
//! 3) 渲染 + 电平：对快照里**每条非母线轨**
//!       SynthEngine::render_track(该轨的 NoteSchedule) → track_scratch（声相之前、单声道）
//!         →  MeterBank::measure(...)                        ← 逐轨电平口径不变
//!         →  sum_into_bus(声相增益 (cos θ, sin θ)，构造期算好)
//!    然后 **BusLimiter::apply(block)**                    ← 母线峰值限制（前瞻 33 帧）
//!    再对母线（stereo-linked）MeterBank::measure_bus_stereo(block)   ← **限制之后**的读数
//!    最后播放头前进 frames（**每量子一次**，与轨道数无关）
//! 4) 发布：**每量子恰好一次** meters.publish(本量子的全部帧) [ARCH-UI-002, ROAD-M2-008]
//! 5) end_block() 公布读者进度
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

use rtrb::Producer;
use yeban_model::EntityId;

use crate::block::{AudioBlock, DEFAULT_BLOCK_FRAMES};
use crate::fpu::{self, FtzDazOutcome};
use crate::meter::{MeterBank, MeterFrame, MeterPublisher, SCRATCH_METERS};
use crate::mixer::{BusLimiter, PanLaw};
use crate::ring::{EngineEvent, EventReceiver, SCRATCH_EVENTS};
use crate::snapshot::{EngineSnapshot, SnapshotReader, SnapshotSlot};
use crate::synth::{MAX_TRACK_SLOTS, SynthEngine};

// 编译期钉住块长是规范允许的取值 [ARCH-DET-001, MODEL-AST-002]。
const _: () = crate::block::assert_supported_frames::<DEFAULT_BLOCK_FRAMES>();
// 栈上临时事件缓冲至少能装下一个量子的参数洪峰。
const _: () = assert!(SCRATCH_EVENTS >= DEFAULT_BLOCK_FRAMES);

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
    /// 当前快照下武装的**每秒量子数**（= `sample_rate / DEFAULT_BLOCK_FRAMES`）。
    ///
    /// 为什么把它暴露出来: 它曾经被错算成 `sample_rate / 设备缓冲长度`
    /// （项目声明的 `audio_config.block_size`, 例如 256）⇒ 峰值保持按 10 dB/s 而不是 20 dB/s 衰减。
    /// 那种错**不会 panic、也不会让既有判据变红**, 只会让表头慢慢不准 ——
    /// 所以把"武装进去的那个数"变成可读的统计量, 让判据能直接钉住它。
    ///
    /// `None` = 还没有任何快照被处理过。
    pub quanta_per_second: Option<f32>,
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
    /// 母线峰值限制器（前瞻式，立体声联动）[ARCH-DSP-001]。
    ///
    /// 位置：**逐轨汇流之后、母线电平之前** ⇒ 母线电平读数（[`EngineStats::meter_frames`]）
    /// 就是**限制后**的读数，而逐轨电平仍是**声相之前**的单声道读数。
    limiter: BusLimiter,
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
    #[must_use]
    pub fn new(
        slot: &Arc<SnapshotSlot>,
        retire: Producer<Arc<EngineSnapshot>>,
        events: EventReceiver,
        meters: MeterPublisher,
    ) -> Self {
        Self {
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
            limiter: BusLimiter::new(),
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
        }
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

    /// 当前累计统计。
    #[must_use]
    pub fn stats(&self) -> EngineStats {
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
            quanta_per_second: self.armed_quanta_per_second,
        }
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
    #[must_use]
    pub const fn position_samples(&self) -> u64 {
        self.synth.position()
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
            limiter,
            limiter_gain_reductions,
            limiter_max_reduction,
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

        // --- 1) 参数/音符/走带事件：**每块一次**批量出队 [ROAD-M2-007] ---
        let mut applied = 0usize;
        events.drain_with(scratch_events, |event| {
            if !event.is_idle() {
                applied += 1;
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
                *armed_scheduled_notes = current.scheduled_notes() as u64;
                *armed_note_schedule_drops = current.note_schedule_drops();

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
            let metered_tracks = current.tracks().keys().filter(|id| **id != master).count();
            // 给母线留一个槽位, 保证母线永远有电平可发。
            let track_budget = scratch_meters.len().saturating_sub(1);
            if metered_tracks > track_budget {
                *meter_capacity_drops =
                    meter_capacity_drops.wrapping_add((metered_tracks - track_budget) as u64);
            }
            for id in current.tracks().keys() {
                if *id == master || produced >= track_budget {
                    continue;
                }
                let track = *id;
                // 真的合成：快照里的音符调度表 → 声部池 → 单声道、声相之前的样本。
                // 参数/音符的投影全部在控制线程完成；这里只有整数相位递推、
                // 线性插值与 ADSR（全是 IEEE 精确类运算，见 `synth` 模块文档 §2）。
                synth.render_track(
                    track,
                    current.schedule(&track),
                    &mut track_scratch[..frames],
                );
                if let Some(frame) = bank.measure(track, quantum, &track_scratch[..frames]) {
                    scratch_meters[produced] = frame;
                    produced += 1;
                } else {
                    *meter_capacity_drops = meter_capacity_drops.wrapping_add(1);
                }
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
            synth.advance(frames);

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
            }
        } else {
            // 极端情况（写者尚未发布任何快照）：输出静音但绝不 panic。
            block.silence();
            block.set_frames(frames);
        }
        snapshot.end_block();

        // --- 4) 电平：**每量子恰好一次**批量推送（本量子的全部帧）[ROAD-M2-008] ---
        if produced > 0 {
            let published = meters.publish(&scratch_meters[..produced]);
            *meter_frames = meter_frames.wrapping_add(published as u64);
            *meter_bulk_publishes = meter_bulk_publishes.wrapping_add(1);
        }
    }
}

/// 母线汇流：把一条轨的**单声道、声相之前**渲染结果按声相增益写进左右两声道。
///
/// `(gain_l, gain_r)` 由 [`crate::mixer::pan_gains`] 在**构造期**算出
/// （`cos`/`sin` 属超越函数类，不进实时路径），实时侧这里只做两次乘加。
/// 居中（默认律）时 `(√2/2, √2/2)` ⇒ 每声道 −3.01 dB，与 `yeban-mcp` 的离线渲染同口径。
///
/// 仍然是**占位**的部分（本切片没做，见 notes 的 needs）：发送/辅助汇流、
/// 路由边的 `gain_db`（`RoutingEdge::gain_db` 目前被忽略）、PDC 延迟线对齐。
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
    use crate::snapshot::{TrackParams, retire_channel};
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
        let slot = SnapshotSlot::new(simple_snapshot(1));
        let (retire, queue) = retire_channel(16);
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
}
