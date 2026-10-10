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
//!       c) 走带按同一份快照的 `sample_rate` / `bpm` 武装（位置不动 ⇒ 不跳变）；
//!       d) 节拍器按同一份快照武装开关与拍栅格（波形已在构造期算好）
//! 3) 渲染 + 电平：对快照里**每条非母线轨**
//!       SynthEngine::render_track(该轨的 NoteSchedule) → track_scratch（声相之前、单声道）
//!         →  **插入链：该轨的通道条**（`TrackV3.devices` 的内置效果器投影 ⇒ `yeban_dsp::channel_strip`；没有则整段跳过）
//!         →  **插入链：该轨的混响**（同一份设备链 ⇒ `yeban_dsp::reverb`，单声道喂两路取中值；没有则整段跳过）
//!         →  **插入链：该轨的卷积混响**（同一份设备链 ⇒ `yeban_dsp::convolution_reverb`，
//!            交错立体声喂同一样本、取中值；IR 在构造期合成并预建，没有则整段跳过）
//!         →  MeterBank::measure(...)                        ← 逐轨电平口径不变
//!         →  **PDC 补偿延迟线（`D(v) = L_max − arrival(v)` 采样点）** [ARCH-PDC-001]
//!         →  sum_into_bus(声相增益 (cos θ, sin θ)，构造期算好)
//!    然后 **节拍器咔哒声**（`transport.metronome_enabled` 的投影；默认关 ⇒ 整段跳过）
//!    然后 **主总线推子**（构造期标量，逐样本只乘）
//!    再 **BusLimiter::process_stereo(block)**              ← 母线峰值限制（前瞻 33 帧）
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
//! ──(该轨的插入通道条, 若有)──► (该轨的插入混响, 若有；单声道取中值) ──► 声相增益 ──► 母线 L/R ──(前瞻峰值限制器)──► AudioBlock ──► cpal / NullBackend
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
//! - [`crate::metronome::render_quantum`]：`[usize; 4]` + `[f32; 4]`（本量子的拍点，栈上定长）
//! - [`EngineRuntime::block`]：`AudioBlock<128>`（`[f32; 128]` × 2）
//! - [`EngineRuntime::synth`]：`[TrackSlot; 16]` × `[Voice; 16]`（声部池，定长）
//! - [`EngineRuntime::armed_strips`]：`[(EntityId, Option<ChannelStrip>); 16]`（每轨插入链，定长；`ChannelStrip` 不含 `Vec`/`Box` ⇒ 无堆）
//! - [`EngineRuntime::reverb_pool`]：`[Reverb; 16]`（每轨混响，**延迟线在构造期分配**；回调内只 `set_params` 与逐样本处理）
//! - [`EngineRuntime::reverb_scratch`]：`[[f32; 128]; 2]`（混响的单声道取中值暂存，栈/内联）
//! - [`EngineRuntime::conv_pool`]：`[ConvolutionReverb; 16]`（每轨卷积混响；**IR 频谱与预延迟线
//!   在构造期分配**，长度是引擎常量 ⇒ 回调内只做同长度的换 IR 与 `set_params`）
//! - [`EngineRuntime::conv_scratch`]：`[f32; 256]`（卷积混响的**交错立体声**暂存，栈/内联）
//! - [`EngineRuntime::synth`] 的 `drums`：`Box<[DrumMachine<16>; 16]>`（每槽一台鼓机，
//!   **构造期一次性分配**；回调内只 `set_params` / `trigger` / `render`）
//! - [`MeterBank`]：`[MeterSlot; 256]`（每节点电平状态，定长数组 + 原位 `swap` 对齐）
//!
//! ⚠ [`EngineRuntime::reverb_pool`] / [`EngineRuntime::conv_pool`] 与 [`SynthEngine`] 的
//! 鼓机池是这份清单里**仅有的**持有堆的字段（混响有自己的延迟线 `Vec`，卷积混响有 IR 频谱
//! 与预延迟线 `Vec`，鼓机池是 `Box<[_; 16]>`）。它们满足禁令的
//! 方式不是"没有堆"，而是"**堆只在构造期建立**"（[`EngineRuntime::new`]）：
//! 回调内一次也不分配、不释放。见 [`crate::insert`] 模块文档 §8.4、§8.5 与 §9.5
//! （为什么连"换采样率"也不能在回调里重建延迟线）与 [`crate::synth`] 的
//! `drums` 字段文档（为什么鼓机池要 `Box`：实测 `DrumMachine<16>` = 13 616 字节）。
//!
//! 唯一允许的"共享状态"是原子量与 rtrb 队列；唯一的系统调用级别操作是
//! FTZ/DAZ 控制寄存器写入（一次）。

use std::sync::Arc;

use yeban_dsp::smoothing::ParamSmoother;
use yeban_model::EntityId;

use crate::block::{AudioBlock, DEFAULT_BLOCK_FRAMES};
use crate::fpu::{self, FtzDazOutcome};
use crate::graph::CompensationBank;
use crate::insert::{
    ChannelStrip, ChannelStripParams, CompressorParams, ConvolutionReverb, ConvolutionReverbParams,
    Reverb, ReverbParams,
};
use crate::level::MAX_LINEAR_MAGNITUDE;
use crate::meter::{MeterBank, MeterFrame, MeterPublisher, SCRATCH_METERS};
use crate::metronome::{MetronomeVoice, render_quantum as render_metronome_quantum};
use crate::mixer::{BusLimiter, PanLaw};
use crate::param::{ParamTable, TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT};
use crate::ring::{EngineEvent, EventReceiver, SCRATCH_EVENTS};
use crate::rt_probe::{self, RtDiagEvent};
use crate::snapshot::{RetireProducer, SnapshotReader, SnapshotSlot};
use crate::stats_mirror::EngineStatsMirror;
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
    /// 累计因电平 SPSC 队列满而**丢弃的帧数**（`= MeterPublisher::dropped()`；
    /// 单调不减（饱和）；无重置）。
    ///
    /// 单位是**帧（条）**，不是"节点次"。它与 [`Self::meter_capacity_drops`] 是
    /// **两个不同的失败面**：那个数的是"本量子的某一帧在**发布批次**里放不下"
    /// （`SCRATCH_METERS` 装不下轨数，或 `MeterBank` 淘汰了槽），这个数的是
    /// "批次放得下，但**SPSC 环**已经满了"（UI 侧没有按 60Hz 抽干）。
    /// 合并成一个数会让"轨数超出临时缓冲"与"UI 落后到丢帧"在读数上无法区分。
    ///
    /// ⚠ [`Self::meter_frames`] 统计的是**真的写进队列**的帧数 ⇒ 本应发布的帧数
    /// = `meter_frames + meter_dropped_frames`。两者之差就是"UI 没跟上"的精确条数
    /// （等号判据见 `tests/meter_rt_contract.rs` 的 S8）。
    ///
    /// 为什么需要它：`MeterPublisher::dropped()` 一直被称为"可观测的健康指标"
    /// （`crates/yeban-engine/src/meter.rs` 现位于第 199 行），但那个计数器住在
    /// [`EngineRuntime`] 的**私有字段**（`meters`）里，而运行时归音频回调线程所有
    /// （设备腿 = `yeban-app` 的默认运行形态）⇒ 控制面拿不到它。
    /// `line/engine-meters` 的台账把这条登记为缺口（"`dropped` 没有接到 UI 告警"，
    /// 该条现位于 `docs/ledger/engine-meters-notes.md` 第 309–310 行）。
    /// 本字段把那个**已有的**计数器搬进 `EngineStats` 与跨线程只读镜像
    /// （不新增第二份计数：读数是发布端自己的那一个）。
    ///
    /// **冷值 0**（与 `EngineStats::default()` 同值）：从来没有发生过丢帧
    /// ⇒ 判据可以用 `> 0` 当"UI 落后过"的见证，而不必从音频输出反推。
    pub meter_dropped_frames: u64,
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
    /// 累计触发过的音符数（**两条音源合计**：鼓机派发 + 复音派发）。
    ///
    /// ⚠ 口径：本计数器在鼓机分支与复音分支上都 `+1` ⇒ 它**不是**"复音合成器触发过
    /// 几个音"。要单独要复音那一条，读 [`Self::poly_notes_triggered`]（器件侧），
    /// 或做这条减法：`notes_triggered − drum_hits`。
    pub notes_triggered: u64,
    /// **复音合成器器件自己**累计接收的触发数（各槽之和；单位：次；0 = 从没触发）。
    ///
    /// 它与 [`Self::notes_triggered`] − [`Self::drum_hits`]（引擎侧的复音派发数）是
    /// **同一个事件的两侧**，与鼓机那一对（[`Self::drum_hits`] /
    /// [`Self::drum_triggers`]）**同款**：那个在引擎的派发处 `+1`，这个在器件的
    /// `note_on` 入口 `+1`。两者在静止点上**恒等**：
    /// `notes_triggered − drum_hits == poly_notes_triggered`
    /// （判据见 `tests/drums_instrument.rs` 的 D10 与 `src/synth.rs` 的库单测）。
    ///
    /// 口径：读数取各槽 `PolySynth::notes_triggered` 之和
    /// （`SynthEngine::poly_notes_triggered`）。器件计数器**只增不减**，且器件自己的
    /// `reset` 只清在响的声部、**不**清它；槽位一旦占用就不会被重建
    /// ⇒ 本字段与 [`Self::notes_triggered`] 同为引擎生命周期的累计量。
    ///
    /// 为什么需要它：[`Self::notes_triggered`] 混了两条音源 ⇒ 只读它无法区分
    /// "复音轨真的触发了"与"全是鼓机在打"；而 [`Self::voice_steals`] 只在**声部池溢出**
    /// 时才推进 ⇒ 一个从不溢出的复音工程在读数面上没有"触发过"的见证。
    /// 该器件 getter 在本次改动前在 `yeban-engine` 里**一个读者都没有**
    /// （量法：`grep -rn 'slot\.synth\.' crates/yeban-engine/src` 命中 12 个器件调用点
    /// = 11 个方法名，其中 `notes_triggered` 零命中；改动后同一条命令命中 13 行）。
    ///
    /// **冷值 0**：全部轨都不是复音合成器时（默认）恒为 0。读它只汇总 16 个 `u64`
    /// 字段：不碰样本、不分配、不加锁 ⇒ 渲染输出逐位不变。
    pub poly_notes_triggered: u64,
    /// **复音合成器此刻占用着的声部数**（各槽之和；单位：声部；0 = 一个都没有）。
    ///
    /// 这是一个**量规**（可升可降），与它上面那一族的每一个读数都不同：
    /// [`Self::notes_triggered`] / [`Self::poly_notes_triggered`] /
    /// [`Self::voice_steals`] 都只增不减，它们回答"曾经发生过什么"；本读数回答
    /// "**现在**占了多少"。`voice_steals > 0` 只证明池**曾经**满过 ⇒ 没有本读数时，
    /// "正在溢出"与"刚好差一个声部"在读数面上不可区分。
    ///
    /// 口径（三条，都要与判据一起读）：
    ///
    /// 1. 读数取各槽 `PolySynth::active_voices` 之和
    ///    （`SynthEngine::poly_active_voices`）。该器件 getter 在本次改动前
    ///    **只有一条读者**，且那条读者住在 `#[cfg(debug_assertions)]` 的
    ///    `SynthEngine::debug_active_voices` 里 —— 那个访问器在 `src` 与 `tests` 里
    ///    **零调用者**（量法：`grep -rnw 'debug_active_voices' crates/yeban-engine`
    ///    在本次改动前命中 `src` 1 行 = 它自己的定义、`tests` 0 行）⇒
    ///    **产物路径上**这件事根本读不出来；
    /// 2. 它只数 16 个槽位的定长声部数组（每槽 16 个 `active` 布尔）⇒ 不碰样本、
    ///    不分配、不加锁、不做 I/O ⇒ 渲染输出逐位不变；
    /// 3. 口径是**全池合计**（与 [`Self::voice_steals`] 的跨槽口径相同），不是逐轨。
    ///
    /// **冷值 0**：全部轨都不是复音合成器时（默认）恒为 0。
    pub poly_active_voices: u64,
    /// 声部池累计**软窃取**次数（每次伴随 3 ms 淡出 [ARCH-RT-004]）。
    pub voice_steals: u64,
    /// 母线限制器**累计压过的样本数**（[ARCH-DSP-001]；0 = 从未越过阈值）。
    ///
    /// 它是"限制器真的接在母线上"的结构性证据：只断言"峰值 ≤ 阈值"在
    /// **从未越过阈值**的夹具上会永真（假绿），因此把"压了多少个样本"暴露出来。
    pub limiter_gain_reductions: u64,
    /// 母线限制器累计的**最大**瞬时压限量（1.0 − 最小增益；0 = 从未压过）。
    pub limiter_max_reduction: f32,
    /// 母线限制器**上一个有快照的渲染量子结束时**的瞬时压限量
    /// （`1.0 − gain()`；`0.0` = 那一刻完全透明）。
    ///
    /// 与 [`Self::limiter_max_reduction`] 的**区别是语义，不是单位**：那个是**全程最大**
    /// （只增不减的累计量），这个是**量规**（可升可降，随限制器释放回落）。两者单位相同
    /// （线性 `1.0 − 增益`），因此可以互相比较 —— 逐量子采样本字段再取最大值，结果
    /// **恰好**等于同一段渲染里的 [`Self::limiter_max_reduction`]（等号判据见
    /// `tests/mix_render.rs` 的 M10）。
    ///
    /// 为什么需要它：`line/engine-mix` 的台账把"限制器的增益衰减表（GR）上报"登记为
    /// 一条缺口 —— 当时只有"累计压过多少样本"与"全程最大压限"两个数，**没有**
    /// "当前压了多少"这个按量子可读的量。该票据的边界清单原文逐字为
    /// "只暴露了 `limiter_gain_reductions` 与 `limiter_max_reduction` 两个累计量，
    /// 没有按量子发布'当前 GR'给 UI"。本字段就是那一条读数（该行现位于
    /// `docs/ledger/engine-mix-notes.md` §8.1）。
    ///
    /// ⚠ 语义边界（必须与判据一起读）：它取的是**量子边界**上的值，不是量子内峰值；
    /// 没有快照的量子（输出静音、限制器不参与）**不更新**它 —— 它保持上一次的值，
    /// 因此它描述的是"限制器最后一次工作时压了多少"，而不是"这个量子压了多少"。
    pub limiter_current_reduction: f32,
    /// **每轨插入器件**的**动态级**累计压过的帧数（[`crate::insert`]；0 = 从未压过）。
    ///
    /// 与 [`Self::limiter_gain_reductions`] 同族、但**不是**同一个读数：它把"插入器件
    /// 真的接在轨上、并且动态级真的在工作"变成可读的数，而不是从音频输出反推。
    /// 口径：读数取 [`ChannelStrip::gain_reduction_count`]，即**动态级**压过的帧数
    /// （器件把 `process_gain` 的调用次数换算成"被压过的帧"）。
    /// 全部轨都没有插入器件时（默认）**恒为 0**；插入仪器的动态级被旁通时也恒为 0。
    pub insert_gain_reductions: u64,
    /// 每轨插入器件累计的**最大**增益衰减（dB，`≥ 0`；0 = 从未压过）。
    ///
    /// ⚠ 单位是 **dB**（与 [`Self::limiter_max_reduction`] 的"1.0 − 增益"线性口径**不同**）：
    /// [`ChannelStrip::max_gain_reduction_db`]（转自压缩器的 `max_reduction_db`）
    /// 本来就以 dB 报数，这里**不换算**，免得引入第二套口径。
    pub insert_max_reduction_db: f32,
    /// **每轨插入器件**动态级在**本量子结束时**的瞬时增益衰减
    /// （dB，`≥ 0`；`0.0` = 那一刻完全透明）。
    ///
    /// 与 [`Self::insert_max_reduction_db`] 的区别是**语义，不是单位**：那个是**全程最大**
    /// （只增不减的累计量），这个是**量规**（可升可降，随动态级释放回落）。两者单位相同
    /// （dB）⇒ 可以互相比较：任意时刻本读数 **≤** [`Self::insert_max_reduction_db`]
    /// （等号与回落判据见 `tests/channel_strip_insert.rs` 的 C15）。
    ///
    /// 口径（三条，都要与判据一起读）：
    ///
    /// 1. 读数取 [`ChannelStrip::current_gain_reduction_db`] —— 器件里**早就存在**的
    ///    getter（住在 `crates/yeban-dsp/src/channel_strip.rs`，现位于第 655 行），
    ///    而 `yeban-engine` 此前**一个读者都没有**（量法：
    ///    `grep -rn 'current_gain_reduction_db' crates/yeban-engine/src` 在本次改动前
    ///    命中 **0** 行）。它与上面的 `max_reduction_db`、以及
    ///    [`Self::insert_gain_reductions`] 用的是**同一个器件、同一个瞬间**的状态
    ///    ⇒ 本字段**不增加**任何 DSP 调用、不碰任何样本、不分配、不加锁
    ///    ⇒ 渲染输出逐位不变；
    /// 2. 一个量子内可能有多条轨的通道条在工作 ⇒ 取**最大值**（与
    ///    [`Self::insert_max_reduction_db`] 的跨轨口径**相同**：两者都是"这条引擎里
    ///    压得最狠的那一台"）；
    /// 3. 本量子**一台武装通道条都没有处理过帧** ⇒ **不更新**（保持上一次的值），
    ///    与 [`Self::limiter_current_reduction`] 的边界口径相同 —— 它描述的是
    ///    "插入链最后一次工作时压了多少"，不是"这个量子压了多少"。**有**武装通道条
    ///    但它的动态级没开（只有 EQ／滤波的通道条）时读数是 `0.0`：
    ///    那是"插入链现在是透明的"这一条**事实**，不是缺失。
    ///
    /// 为什么需要它：[`Self::insert_max_reduction_db`] 回答不了"此刻压了多少" ——
    /// 一个 UI 的插入链 GR 表要的是后者。母线限制器**早有**这一对读数
    /// （[`Self::limiter_max_reduction`] ＋ [`Self::limiter_current_reduction`]），
    /// 而插入链只有累计量 ⇒ 本字段补的是同一条读数在**插入链**上的缺项。
    pub insert_current_reduction_db: f32,
    /// **每轨插入器件**（通道条）动态级**检波器**看到的电平
    /// （dBFS；`None` = 从来没有一台武装通道条处理过帧）。
    ///
    /// 单位与 [`Self::insert_current_reduction_db`] 的**不同**：那个是 dB 衰减，本读数是
    /// **dBFS 电平**。两者是同一台器件、同一个瞬间的**两侧**：本读数是压缩器检波器的
    /// **输入**侧，那个是它算出来的**输出**侧（压了多少）。
    ///
    /// 口径（四条，都要与判据一起读）：
    ///
    /// 1. 读数取 [`ChannelStrip::detector_level_db`]，它是压缩器的 `level_db()`
    ///    （`crates/yeban-dsp/src/channel_strip.rs` 现位于第 663 行，转发
    ///    `crates/yeban-dsp/src/compressor.rs` 现位于第 663 行的同名 getter）——
    ///    器件里**早就存在**的 getter，而 `yeban-engine` 此前**一个读者都没有**
    ///    （量法：`grep -rnw 'detector_level_db' crates/yeban-engine` 在本次改动前
    ///    ⇒ `src` 0 行、`tests` 0 行；同一条命令射程放到全 `crates` 也只剩 **1** 行
    ///    = 它自己的定义 ⇒ 它在整个工作区里没有任何读者）。它只读一个 `f32` 字段
    ///    （`const fn`）⇒ 本字段**不增加**任何 DSP 调用、不碰任何样本、不分配、不加锁
    ///    ⇒ 渲染输出逐位不变；
    /// 2. 采样点是**滤波级之后、动态级之前**（通道条的固定顺序是
    ///    输入增益 → EQ → 滤波 → 动态 → 输出增益，见 `crates/yeban-dsp/src/channel_strip.rs`
    ///    §2）⇒ 它与 [`crate::level`] 的逐轨电平读数**不是**同一个点：那个在整条插入链
    ///    **之后**。本读数因此是这条链上的**内部探针** —— 只有它能看到"EQ 与滤波之后、
    ///    压限之前"的那一档电平（EQ 的平坦档位不是逐位恒等，实测最大绝对差 4.566e-5，
    ///    出处同上 §3）；
    /// 3. 一个量子内可能有多条轨的通道条在工作 ⇒ 取**最大值**（与
    ///    [`Self::insert_max_reduction_db`] / [`Self::insert_current_reduction_db`]
    ///    的跨轨口径相同：都是"这条引擎里电平最高的那一台"）；
    /// 4. 本量子**一台武装通道条都没有处理过帧** ⇒ **不更新**（保持上一次的值），
    ///    与 [`Self::insert_current_reduction_db`] 的边界口径相同、由**同一个**判据
    ///    条件（"本量子真的有一台武装通道条处理过帧"）驱动。
    ///
    /// **冷值是 `None`，不是 `0.0`**：`0.0` 是一个**合法且有意义**的检波器电平
    /// （0 dBFS = 满刻度）⇒ 用 `0.0` 表示"从来没有过读数"会让控制面在第一个量子之前
    /// 读到一条**假读数**。这与 [`Self::quanta_per_second`] 选 `Option<f32>` 是同一个
    /// 理由，也是本字段与同族其它读数（它们的冷值 `0` 恰好等于语义上的"没处理过 /
    /// 透明"）**唯一**的形状差别。
    ///
    /// ⚠ 语义边界：动态级被**旁通**（`compressor_enabled = false`，即设备一个动态旋钮
    /// 都没写）的通道条照样在"处理帧"（它的其它级在跑），但它报出的是压缩器里**冻结**
    /// 的上一次读数；从未运行过的压缩器报它的构造初值 `MIN_LEVEL_DB`
    /// （`−120 dBFS`，定义在 `crates/yeban-dsp/src/compressor.rs` 现位于第 240 行）。
    /// 那是**器件侧的事实**（那个 getter 的返回值），不是缺失 —— 判据
    /// `tests/channel_strip_insert.rs` 的 C16 把这一档逐位钉住。
    ///
    /// 为什么需要它：[`Self::insert_gain_reductions`] /
    /// [`Self::insert_max_reduction_db`] / [`Self::insert_current_reduction_db`] 三条
    /// 都是压限的**输出**侧（压过多少帧、最多压了多少、此刻压了多少），而 GR 是
    /// **增益弹道之后**的量 —— 三条一起读仍然回答不了"动态级的**工作点**在哪、
    /// 检波器离阈值有多远"。检波器电平是唯一回答那个问题的读数（形态判据见
    /// `tests/channel_strip_insert.rs` 的 C16）。
    pub insert_detector_level_db: Option<f32>,
    /// **每轨插入器件**（通道条）累计处理过的帧数（[`crate::insert`]；0 = 从未处理）。
    ///
    /// 口径：读数取 [`ChannelStrip::processed_frames`]，即通道条**整级链**处理过的帧数
    /// （与 [`Self::insert_gain_reductions`] 的"动态级压过的帧数"**不同**：
    /// 前者在 EQ／滤波级工作时照样推进，后者只在动态级真的压到时推进）。
    ///
    /// 它存在的理由：`EngineStats` 的插入读数原来只有"压缩量"这一类，
    /// 而一个**只有 EQ／滤波**的通道条不产生任何压缩量 ⇒ 没有这条读数，
    /// "EQ 级真的接在轨上"就只能从音频输出反推。全部轨都没有插入器件时**恒为 0**。
    pub insert_strip_frames: u64,
    /// **每轨插入器件的混响级**累计处理过的帧数（[`crate::insert`]；0 = 从未处理）。
    ///
    /// 口径：读数由引擎在**真的调用了** [`Reverb::process`] 的那一条分支上累加
    /// （条件 = 本轨武装了混响 **且** `Reverb::is_active()`，即 `mix > 1e-4`）。
    /// 与 [`Self::insert_strip_frames`] 同族：没有它，"混响真的接在轨上"就只能从
    /// 音频输出反推。全部轨都没有混响时（默认）**恒为 0**。
    pub insert_reverb_frames: u64,
    /// 因**混响延迟线的采样率与武装时不同**而整段未武装的快照修订次数
    /// （[`crate::insert`] 模块文档 §8.5；正常恒为 0）。
    ///
    /// 非 0 = 这一份快照里有混响设备，但引擎**拒绝**在音频线程重建延迟线
    /// （那会分配 + 释放 [MUST-GATE-001]）⇒ 那一份快照里混响不工作。它是"容量/
    /// 配置不足不静默"的机械形式：宁可少一个器件，也不做出听不出来的错。
    pub insert_reverb_rate_rejects: u64,
    /// **每轨插入器件的卷积混响级**累计处理过的帧数（[`crate::insert`]；0 = 从未处理）。
    ///
    /// 口径：读数由引擎在**真的调用了** [`ConvolutionReverb::process`] 的那一条分支上累加
    /// （条件 = 本轨武装了卷积混响 **且** `ConvolutionReverb::is_active()`，
    /// 即 `wet > 1e-4`）。与 [`Self::insert_reverb_frames`] 同族：没有它，"卷积混响真的
    /// 接在轨上"就只能从音频输出反推。全部轨都没有卷积混响时（默认）**恒为 0**。
    pub insert_convolution_frames: u64,
    /// 因**配置不可用**而整段未武装的卷积混响设备次数（[`crate::insert`] 模块文档 §9.5；
    /// 正常恒为 0）。
    ///
    /// 两条来源，都记在这一个读数里（都是"这一份快照里有卷积混响设备，但引擎没有武装它"）：
    ///
    /// 1. 快照的采样率与**武装时**不同（IR 的帧数是 `采样率 ÷ 10` ⇒ 缓冲要重建 = 分配，
    ///    音频线程不允许 [MUST-GATE-001]）；
    /// 2. 器件**拒绝**了这条 IR（非有限样本 / 频谱溢出）—— 由构造期合成的 IR 在数学上
    ///    不可能触发（峰值归一化的有限噪声），这条是器件的唯一拦截点留下的**安全网**。
    ///
    /// 它是"容量/配置不足不静默"的机械形式：宁可少一个器件，也不做出听不出来的错。
    pub insert_convolution_rejects: u64,
    /// **实时侧参数目标表**累计把增益乘过的帧数（[`crate::param`]；0 = 从未乘过）。
    ///
    /// 它是"`SetParam` 真的改变了声音"的**见证**：没有它，`events_applied` 只能证明
    /// "事件出队了"，而一个出队后什么都不做的 `SetParam` 与一个正常工作的参数表
    /// 在读数上完全一样。全部轨都没有收到被接受的参数事件时（默认）**恒为 0**。
    pub param_gain_frames: u64,
    /// **主总线**增益乘子累计乘过的帧数（[`crate::param`] 模块文档 §2.3；0 = 从未乘过）。
    ///
    /// 与 [`Self::param_gain_frames`] **分开**记账：两者回答两个不同的问题
    /// （"有音轨的乘子生效了吗" vs "母线的乘子生效了吗"）。合并成一个数会让
    /// "只自动化了主总线"与"只自动化了一条轨"在读数上无法区分。
    /// 主总线槽位没有收到过被接受的参数事件时（默认）**恒为 0**。
    pub param_master_gain_frames: u64,
    /// 因**取值非法**（非有限 / 负数）而被忽略的 `SetParam` 事件数（[`crate::param`] §2）。
    ///
    /// 非 0 = 控制面发过一个本槽位语义里不存在的值；引擎**不**把它钳成静音
    /// （那是"听得出、读不出"的行为反转），而是忽略并计数。
    pub param_gain_rejects: u64,
    /// 因**地址未映射**而被忽略的 `SetParam` 事件数（[`crate::param`] §2）。
    ///
    /// 未映射 = 槽位号与实体**不匹配**：实体不是主总线时槽位号须是
    /// [`crate::param::TRACK_GAIN_SLOT`]、实体是主总线时须是
    /// [`crate::param::MASTER_GAIN_SLOT`]（两者不通用，见 [`crate::param`] §2）。
    /// 花在"引擎根本不消费的地址"上的事件因此是**可见**的，而不是静默丢弃。
    pub param_unmapped_events: u64,
    /// 因**参数槽位表已满**而未被接受的 `SetParam` 事件数（累计；正常恒为 0）。
    ///
    /// 槽位数与声部池的轨道上限同源（[`crate::param::PARAM_SLOTS`]）；
    /// 容量不足在这里同样**不静默**。
    pub param_capacity_drops: u64,
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
    /// **支路对齐基准 `L_max`**（采样点；`PdcPlan::total_latency()` 的读数）。
    ///
    /// 它是"进入总线求和节点之前的最长支路延迟"，也就是那些延迟线要补齐的目标
    /// （逐节点的补偿量 `D(v)` 由 [`EngineRuntime::armed_pdc_delay`] 可读）。
    /// **不含**接在总线求和**之后**的 `master` 自身延迟 —— 母线前瞻限制器的
    /// [`BUS_LIMITER_LATENCY_FRAMES`](crate::mixer::BUS_LIMITER_LATENCY_FRAMES) = 33 帧
    /// [ADR-0001 D44(b)]。问"喂进引擎第 0 帧的信号第几帧出现在输出"要读
    /// [`Self::engine_output_latency_frames`]。
    ///
    /// 取**已武装快照**的计划值（快照边界覆写；还没有任何快照被处理时是 `0`）。
    /// 它与 [`Self::engine_output_latency_frames`] 是**两个不同的数**：把后者当成
    /// 前者会给出错误的端到端延迟预算（差 `master` 自身的延迟）。
    pub pdc_alignment_frames: u32,
    /// **引擎输出延迟**（采样点；`PdcPlan::output_latency()` 的读数）。
    ///
    /// `= pdc_alignment_frames + master 自身延迟`，即"喂进引擎第 0 帧的信号在第几帧
    /// 出现在输出"。它是 `[ARCH-PDC-002]` 的监听延迟预算表里"内部 DSP 拓扑调度"
    /// 那一格**在引擎侧**的可读形式：这个数此前**只算不读** ——
    /// `PdcPlan::output_latency` 由母线限制器延迟回填那一条改动（`b6842b0`）引入，
    /// 而 `yeban-engine` 侧没有任何**运行时**读者（量法：
    /// `grep -rn 'output_latency' crates/yeban-engine/src` 在本次改动前的读数是
    /// `graph.rs` 13 行（定义与文档）、`mixer.rs` 2 行与 `snapshot.rs` 5 行
    /// （都是文档或判据里的引用）、**`rt.rs` 零行**）。控制面要算端到端预算，
    /// 必须把设备侧读数（[`crate::latency`]）与这个数相加。
    ///
    /// 取**已武装快照**的计划值（快照边界覆写；还没有任何快照被处理时是 `0`）。
    /// 生产路径（[`crate::snapshot::EngineSnapshot::from_project`]）在 `master` 上回填
    /// 母线限制器的 33 帧 [ADR-0001 D44(b)]，因此它通常**大于**
    /// [`Self::pdc_alignment_frames`]；显式注入延迟表的那条路径
    /// （`from_project_with_latencies`）**不**追加那 33 帧 ⇒ 两者相等。
    pub engine_output_latency_frames: u32,
    /// **节拍器累计触发过的咔哒声次数**（[`crate::metronome`]）。
    ///
    /// 与 [`Self::pdc_processed_blocks`] 同族：它把"节拍器真的在打拍子"变成可读的
    /// 读数，而不是从音频输出反推。`metronome_enabled = false`（默认）时**恒为 0**。
    pub metronome_clicks: u64,
    /// **鼓机累计触发过的鼓击次数**（[`crate::drums`]；0 = 从没打过鼓）。
    ///
    /// 与 [`Self::metronome_clicks`] 同族：`EngineRuntime::armed_drum_slot_count()` 只说明
    /// "本快照里有几轨武装了鼓机"，这个数才说明"鼓击真的落到了器件上"。
    /// 全部轨都不是鼓机时（默认）**恒为 0**。
    pub drum_hits: u64,
    /// 鼓机器件**自己**累计接收的触发数（各槽之和；0 = 器件从没收到过触发）。
    ///
    /// 它与 [`Self::drum_hits`] 是**同一个事件的两侧**：那个在引擎的派发处 `+1`，
    /// 这个在器件的 `trigger` 入口 `+1`（器件自带的计数器，本字段只是把它搬过来）。
    /// 两者在静止点上**恒等** —— 因此本字段的用途是把那条等号变成可读的
    /// **内部一致性判据**（判据见 `tests/drums_instrument.rs` 的 D9）：任何"派发了却
    /// 没到器件"（或反之）的接线错误都会让它当场不等，而从音频输出反推不出来。
    ///
    /// 全部轨都不是鼓机时（默认）**恒为 0**；器件计数器只增不减且 `reset` 不清它
    /// ⇒ 本字段与 [`Self::drum_hits`] 同为引擎生命周期的累计量。
    pub drum_triggers: u64,
    /// 鼓机**声部池**累计的窃取次数（各槽之和；单位：次；0 = 从未窃取）。
    ///
    /// ⚠ 它与 [`Self::voice_steals`] 是**两个不同的池**：那个汇总复音合成器的槽位，
    /// 本字段汇总鼓机器件的槽位。分开记账的理由是排除一个真实的读数盲区 ——
    /// 鼓击池满时器件会替换一个在响槽位并推进自己的计数器，而 `voice_steals`
    /// 只走复音那条 ⇒ **只挂鼓机的工程在池满时 `voice_steals` 恒为 0**，
    /// "声部窃取"在读数面上完全不可见。合并成一个数会让"哪个池满了"无法区分。
    ///
    /// 单位与 [`Self::voice_steals`] 相同（线性次数），两者各自随自己的池饱和增长。
    /// 全部轨都不是鼓机时（默认）**恒为 0**。
    pub drum_voice_steals: u64,
    /// 累计的**闭镲 choke 开镲**次数（各槽之和；单位：次；0 = 从未 choke 过）。
    ///
    /// 器件口径：触发一个闭镲时，所有正在响的开镲槽位立刻进入窃取淡出，每一件被
    /// choke 的开镲记 1；反向（开镲 choke 闭镲）不做。
    ///
    /// 为什么需要它：这是一条**跨声部**行为，任何单轨电平或触发计数都看不到它
    /// （被 choke 的开镲本来就该衰减，音频输出上看不出"是谁让它衰减的"）。
    /// 全部轨都不是鼓机时（默认）**恒为 0**。
    pub drum_hat_chokes: u64,
    /// 累计的**发声槽位帧**（各槽之和；单位：槽位×帧；0 = 器件从没算出过非零样本）。
    ///
    /// 器件口径：一次 `render` 的某一帧里某个在响槽位产出非零样本就记 1；
    /// 除以渲染帧数即"平均同时发声的槽位数"。器件文档明说它是**覆盖度仪器**、
    /// **不改变音频输出**：非零只说明那个槽位在算东西，不说明它多响
    /// （一个 −120 dB 的尾音也记）。
    ///
    /// 为什么需要它：[`Self::drum_hits`] 只证明"触发派发到了器件"，证明不了
    /// "器件真的算出了声音" —— 一个把在响槽位全部丢掉的重置缺陷会让
    /// [`Self::drum_hits`] 照常增长而输出逐位静音。本字段是那条覆盖度见证。
    /// 全部轨都不是鼓机时（默认）**恒为 0**。
    pub drum_sounding_slot_frames: u64,
    /// **鼓机此刻占用着的槽位数**（各槽之和；单位：槽位；0 = 一件鼓都不在响）。
    ///
    /// 它与 [`Self::drum_sounding_slot_frames`] 是**两个不同的质量维度**：那个是
    /// **累计**的"槽位×帧"覆盖度读数（只增不减），本读数是**瞬时**占用数（可升可降）。
    /// 器件里两个读数早就有（`DrumMachine::sounding_slot_frames` 与
    /// `DrumMachine::active_slots`），而后者在本次改动前在 `yeban-engine` 的
    /// `src` 与 `tests` 里**一个读者都没有**（量法：
    /// `grep -rnw 'active_slots' crates/yeban-engine` ⇒ `src` 0 行、`tests` 0 行）。
    ///
    /// 口径（三条，都要与判据一起读）：
    ///
    /// 1. 读数取各槽 `DrumMachine::active_slots` 之和
    ///    （`SynthEngine::drum_active_slots`）：只数 16 个槽位的定长数组，每个槽位一个
    ///    `active` 布尔 ⇒ 不碰样本、不分配、不加锁、不做 I/O ⇒ 渲染输出逐位不变；
    /// 2. 与 [`Self::poly_active_voices`] **分开记账**：鼓机与复音是**两个独立的池**
    ///    （该口径见 [`Self::drum_voice_steals`] 的文档），合并成一个数会让"哪个池
    ///    满了/多满"无法区分；
    /// 3. 与 [`Self::drum_triggers`] 一族的**累计量**口径刻意不同：本读数是**量规**，
    ///    器件 `reset` 清空槽位时它会当场回落。
    ///
    /// **冷值 0**：全部轨都不是鼓机时（默认）恒为 0。
    pub drum_active_slots: u64,
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

    /// **"UI 落后到丢电平帧"的累计判定**：自进程开始以来，是否至少丢弃过一帧
    /// （[`Self::meter_dropped_frames`] `> 0`）。
    ///
    /// 与 [`Self::is_snapshot_lagging`] **同一个形状**（累计、**粘滞**）：
    /// `true` 的含义不是"此刻正在丢"，而是"本进程的 UI 至少落后过一次"。
    /// 控制面要对"这一段采样窗口"提问时用 [`Self::is_meter_lagging_since`]。
    #[must_use]
    pub const fn is_meter_lagging(&self) -> bool {
        self.meter_dropped_frames > 0
    }

    /// **自上次读取以来**丢弃的电平帧数（增量；饱和减法）。
    ///
    /// 语义：`self.meter_dropped_frames - previous.meter_dropped_frames`（读到更旧的
    /// 基线时给出 `0` 而不是回绕）。控制面的 60Hz 循环每次读 `stats()` 时把上一帧留着，
    /// 比较这个增量就能区分"很久以前丢过一帧"与"这 16.7 ms 正在丢"—— 降速/告警决策
    /// 需要的是后者。
    #[must_use]
    pub const fn meter_dropped_since_last_read(&self, previous: &Self) -> u64 {
        self.meter_dropped_frames
            .saturating_sub(previous.meter_dropped_frames)
    }

    /// 采样窗口内的"UI 落后"判定：`self` 相对 `previous` 至少新丢了一帧。
    #[must_use]
    pub const fn is_meter_lagging_since(&self, previous: &Self) -> bool {
        self.meter_dropped_since_last_read(previous) > 0
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
    /// **累计统计的跨线程只读镜像**（[`crate::stats_mirror`]）。
    ///
    /// 为什么需要：运行时一旦挂到设备回调上就归音频线程所有 ⇒ 控制线程拿不到
    /// [`Self::stats`]（`yeban-app` 的 `EngineHost::engine_stats` 因此返回 `None`，
    /// 登记见该模块文档与 `docs/ledger/feature-alignment.md` 的 "cpal 设备宿主" 行）。
    /// 构造期把这份 `Arc` 的克隆交给控制面（[`Self::stats_mirror`]），此后**每量子**
    /// 发布一次 ⇒ 设备腿活跃时健康读数仍然可读。
    ///
    /// 与 `transport_mirror` 同款：实时侧只写、控制侧只读，双向都不阻塞；
    /// 发布只做原子存（零分配、零锁、零 I/O、零日志）[MUST-GATE-001]。
    stats_mirror: Arc<EngineStatsMirror>,
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
    /// 见 [`EngineStats::pdc_alignment_frames`]（快照边界覆写；初值 0）。
    pdc_alignment_frames: u32,
    /// 见 [`EngineStats::engine_output_latency_frames`]（快照边界覆写；初值 0）。
    engine_output_latency_frames: u32,
    /// 本快照武装的声相衰减律（`audio_config.pan_law` 的投影）。
    armed_pan_law: PanLaw,
    /// 本快照武装的每轨声相增益 `(左, 右)`（构造期算好，实时侧只做乘法）。
    armed_pan_gains: [(EntityId, f32, f32); MAX_TRACK_SLOTS],
    /// 本快照武装的声相增益条数（前 `n` 项有效）。
    armed_pan_slots: usize,
    /// **声相自动化的逐样本平滑器**（左 / 右），与 `armed_pan_gains` 同下标。
    ///
    /// 裁决 P4=(b)：声相**不**在逐样本路径上算 `cos`/`sin`（那是超越函数类 ⇒ 只能落在
    /// 构造期或事件边界）。控制侧把声相位置折成**绝对** `(左, 右)` 增益发进来，
    /// 这里只做"从当前值逐样本收敛到目标值"（`ParamSmoother`，与增益槽位同形）。
    ///
    /// `armed_pan_armed[i] == false` ⇒ 逐样本走**原来那条**常量增益路径
    /// （[`sum_into_bus`]）⇒ 没有声相自动化的工程**逐位不变**。
    armed_pan_l: [ParamSmoother; MAX_TRACK_SLOTS],
    /// 见 [`Self::armed_pan_l`]（右声道）。
    armed_pan_r: [ParamSmoother; MAX_TRACK_SLOTS],
    /// 本槽位是否收到过**被接受的**声相自动化目标（真 ⇒ 走平滑路径）。
    armed_pan_armed: [bool; MAX_TRACK_SLOTS],
    /// **覆盖度见证**：被**平滑**声相乘过的帧数（累计）。
    ///
    /// 零分配判据必须能回答"那条平滑路径真的被走到了吗"——它至少在两个可观测上与
    /// 常量路径相同（都不分配、都不加锁）⇒ 没有这个数，"四元组全 0"可能是空转。
    /// 读口见 [`EngineRuntime::pan_automation_frames`]。
    pan_automation_frames: u64,
    /// 被拒绝的声相**值**数（非有限 / 负数）。
    pan_automation_value_rejects: u64,
    /// 本快照武装的**主总线线性增益**（构造期由
    /// [`crate::snapshot::EngineSnapshot::master_gain`] 算好）。
    ///
    /// 与声相增益表同一个理由：`dB → 线性` 含 `exp2`（超越函数类），只能在快照
    /// 边界读一次；逐样本路径只做乘法（见 [`Self::armed_master_gain`]）。
    armed_master_gain: f32,
    /// 本快照是否开启节拍器（`transport.metronome_enabled` 的投影）。
    ///
    /// `false`（默认）时**整段咔哒声代码不被执行** ⇒ 输出与接线前逐字节相同。
    armed_metronome_enabled: bool,
    /// 本快照武装的**拍栅格**：一拍多少 tick（`PPQ × 4 / 拍号分母`，构造期算好）。
    armed_metronome_ticks_per_beat: u64,
    /// 本快照武装的一小节拍数（拍号分子）。
    armed_metronome_beats_per_bar: u64,
    /// **节拍器运行态**（游标 / 下一拍 / 累计触发数）。
    ///
    /// 像走带位置一样属于挥发性会话运行态（[MODEL-ISO-001]），**不进快照**；
    /// 波形与栅格来自快照，因此"换快照 ⇒ 一起换"。
    metronome: MetronomeVoice,
    /// 累计被限制器压过的样本数（与 [`EngineStats::limiter_gain_reductions`] 同源）。
    limiter_gain_reductions: u64,
    /// 累计最大压限量（与 [`EngineStats::limiter_max_reduction`] 同源）。
    limiter_max_reduction: f32,
    /// **上一个有快照的量子**结束时的瞬时压限量
    /// （与 [`EngineStats::limiter_current_reduction`] 同源；**量规**，可升可降）。
    limiter_current_reduction: f32,
    /// **每轨插入链**：`(轨道, 通道条)`，前 [`Self::armed_insert_slots`] 项有效。
    ///
    /// 与 `armed_pan_gains` 同一个形状与同一个理由：**构造期**（快照边界）把器件建好，
    /// 逐样本路径只做"查表 + 调用"。槽位是**预分配**的定长数组 ⇒ 武装步骤
    /// （[`ChannelStrip::new`] / `set_params`，含 `exp`）**零分配** [MUST-GATE-001]。
    ///
    /// `None` 的槽位不参与查找（[`Self::armed_insert_slots`] 之外的内容一律无效）。
    ///
    /// ⚠ 本字段是 `c792fdc` 的 `armed_compressors` 的**同一张表**改名而来：`c792fdc`
    /// 里槽位装的是裸 `Compressor`，本票装的是**含动态级**的 `ChannelStrip`
    /// （见 [`crate::insert`] 模块文档 §2）。表形状、槽位数、查找方式都没变。
    armed_strips: [(EntityId, Option<ChannelStrip>); MAX_TRACK_SLOTS],
    /// 本快照武装的插入器件条数（前 `n` 项有效）。
    armed_insert_slots: usize,
    /// **混响延迟线池**：每槽一台，**构造期**按初始快照的采样率预分配
    /// [ARCH-RT-001, MUST-GATE-001]。
    ///
    /// 与 [`Self::armed_strips`] 同一个槽位表形状，但有一个**结构差别**：
    /// [`Reverb::set_sample_rate`] 会分配（并因此释放）延迟线，是本器件**唯一**的
    /// 分配点 ⇒ 它只在 [`Self::new`]（音频回调之外）调用一次。快照边界只允许
    /// `set_params`（标量赋值，零分配）。理由见 [`crate::insert`] 模块文档 §8.4 / §8.5。
    reverb_pool: [Reverb; MAX_TRACK_SLOTS],
    /// 本快照武装的混响 `(轨道, 是否武装)`；前 [`Self::armed_reverb_slots`] 项有效。
    ///
    /// ⚠ 槽位里装的**不是** `Option<Reverb>`：延迟线必须是**构造期**建好的那一批，
    /// 因此 `Option` 只表达"这一槽本快照是否武装、武装给哪条轨"，实例本身恒存在。
    armed_reverbs: [(EntityId, bool); MAX_TRACK_SLOTS],
    /// 本快照武装的混响条数（前 `n` 项有效）。
    armed_reverb_slots: usize,
    /// 混响延迟线池武装时用的采样率（**构造期**定，运行期不变；§8.5）。
    armed_reverb_sample_rate: u32,
    /// 混响的**单声道取中值**暂存：`[左, 右]` 各一个量子长度（[`crate::insert`] §8.3）。
    ///
    /// 复用一个定长缓冲（与 [`Self::track_scratch`] 同一个形状）⇒ 零分配。
    reverb_scratch: [[f32; DEFAULT_BLOCK_FRAMES]; 2],
    /// **卷积混响池**：每槽一台，**构造期**按初始快照的采样率把 IR 频谱与预延迟线
    /// 全部建好 [ARCH-RT-001, MUST-GATE-001]。
    ///
    /// 与 [`Self::reverb_pool`] 同一个槽位表形状，但"分配发生在哪"这件事**不同**：
    /// 卷积混响要在构造期交给器件一条**IR**（占位用全零、长度 = [`convolution_ir_frames`]），
    /// 因为器件的 IR 长度一旦改变就会重建缓冲。占位 IR 把每槽的缓冲**长度**钉死，
    /// 于是快照边界上的换 IR 是**长度不变**的那条路径 ⇒ 零分配
    /// （`crates/yeban-dsp/src/convolution.rs` 的 `set_impulse_response` 文档，
    /// 现位于第 271 行）。理由见 [`crate::insert`] 模块文档 §9.2／§9.5。
    ///
    /// ⚠ 代价是每个槽位都持有一份 IR 频谱（与"按上限预分配预延迟线"同款）。量级：
    /// `48 kHz` ⇒ IR 4 800 帧 ⇒ 38 个分区 ⇒ 每槽 4 条核 × 4 个 `4 902` 长的 `f32` 数组
    /// ≈ 313 KiB，16 槽 ≈ 4.8 MiB（算术口径与出处见 [`crate::insert`] 模块文档 §9.2）。
    ///
    /// ⚠ 类型是 **`Box<[_; 16]>`** 而不是 `[_; 16]`，理由与 [`SynthEngine`] 的鼓机池
    /// **同款**（那里的字段文档记了尺寸读数）：`[ConvolutionReverb; 16]` 是一个
    /// **约 81 KiB** 的值，把它放进 `EngineRuntime`（一个按值返回、按值传递的结构体）
    /// 会把它加到**每一个** `EngineRuntime` 局部变量的栈帧上。实测（本票）：加上它之后
    /// `tests/param_automation.rs` 的一条判据**栈溢出**（`fatal runtime error:
    /// stack overflow`）⇒ 这是必须 `Box` 的机械证据，不是风格问题。
    /// 分配的**次数**是 1（`Vec::with_capacity` → `into_boxed_slice`），发生在
    /// [`Self::new`] 里，回调内一次也不分配。
    conv_pool: Box<[ConvolutionReverb]>,
    /// 本快照武装的卷积混响 `(轨道, 是否武装)`；前 [`Self::armed_conv_slots`] 项有效。
    ///
    /// 与 [`Self::armed_reverbs`] 同款：槽位里装的**不是** `Option<ConvolutionReverb>`
    /// —— 缓冲必须是**构造期**建好的那一批，`Option` 只表达"这一槽本快照是否武装"。
    armed_convs: [(EntityId, bool); MAX_TRACK_SLOTS],
    /// 本快照武装的卷积混响条数（前 `n` 项有效）。
    armed_conv_slots: usize,
    /// 每个槽位里**当前装着的那一份 IR** 的内容标识（[`ConvolutionPlan::ir_hash`]）。
    ///
    /// 用途：快照边界上判断"这一条 IR 与器件里已经装着的是不是同一个" ⇒ 是同一个就
    /// **只**调 `set_params`（保留频域延迟线与重叠相加尾），不是同一个才重设 IR
    /// （那会把状态复位）。没有它，**每一次工程编辑**都会切断全部卷积尾巴，并为每条轨
    /// 重算 `分区数` 次 256 点变换。理由与碰撞口径见 [`ConvolutionPlan`] 的字段文档。
    ///
    /// 初值 `0` 与任何真实 IR 的哈希都不同（FNV-1a 的偏移基值不是 0）⇒ 未武装过的槽位
    /// 第一次一定会走"重设 IR"那一支。
    armed_conv_ir_hashes: [u64; MAX_TRACK_SLOTS],
    /// 卷积混响池武装时用的采样率（**构造期**定，运行期不变）。
    ///
    /// ⚠ 它同时钉住 IR 的**帧数**（`采样率 ÷ 10`）⇒ 换采样率必须整段不武装，
    /// 否则那条边界上会出现一次 `Vec` 重建（分配）。与 §8.5 同款，见 §9.5。
    armed_conv_sample_rate: u32,
    /// 卷积混响的**交错立体声**暂存：`2 × 量子长度`（`[L, R, L, R, …]`）。
    ///
    /// 器件的接口是交错立体声（[`crate::insert`] §9.4），引擎的插入点是单声道
    /// ⇒ 先把单声道样本**同时**写进左右两路，处理完再取两路的中值。复用一个定长缓冲
    /// ⇒ 零分配。
    conv_scratch: [f32; 2 * DEFAULT_BLOCK_FRAMES],
    /// 累计被**插入器件的动态级**压过的帧数（与 [`EngineStats::insert_gain_reductions`] 同源）。
    insert_gain_reductions: u64,
    /// 插入器件累计最大衰减（dB；与 [`EngineStats::insert_max_reduction_db`] 同源）。
    insert_max_reduction_db: f32,
    /// 插入链动态级**当前**衰减（dB；量规，可升可降。与
    /// [`EngineStats::insert_current_reduction_db`] 同源）。
    insert_current_reduction_db: f32,
    /// 插入链动态级**检波器**看到的电平（dBFS；量规，可升可降。`None` = 从来没有一台
    /// 武装通道条处理过帧。与 [`EngineStats::insert_detector_level_db`] 同源）。
    insert_detector_level_db: Option<f32>,
    /// 累计被**插入器件**处理过的帧数（与 [`EngineStats::insert_strip_frames`] 同源）。
    insert_strip_frames: u64,
    /// 累计被**插入器件的混响级**处理过的帧数（与 [`EngineStats::insert_reverb_frames`] 同源）。
    insert_reverb_frames: u64,
    /// 因混响延迟线的采样率不匹配而未武装的修订次数
    /// （与 [`EngineStats::insert_reverb_rate_rejects`] 同源）。
    insert_reverb_rate_rejects: u64,
    /// 累计被**插入器件的卷积混响级**处理过的帧数
    /// （与 [`EngineStats::insert_convolution_frames`] 同源）。
    insert_convolution_frames: u64,
    /// 因配置不可用而未武装的卷积混响设备次数
    /// （与 [`EngineStats::insert_convolution_rejects`] 同源）。
    insert_convolution_rejects: u64,
    /// **实时侧参数目标表**（[`crate::param`]）：`SetParam` → 逐样本平滑的逐轨增益乘子。
    ///
    /// 与 [`Self::armed_strips`] 的差别：这张表由**事件**（不是快照）驱动
    /// ⇒ 它的槽位在音频线程里惰性分配（定长数组内部，**零分配**），
    /// 采样率在快照边界同步给已分配的平滑器。
    params: ParamTable,
    /// 当前快照的主总线身份（事件边界用来判定"这个地址本表不消费"）。
    ///
    /// 取值在快照边界刷新；构造期先读初始快照的一份（[`Self::new`] 已经要求槽里
    /// 有一份快照 —— 混响延迟线的预分配用的就是同一个来源）。
    armed_master: EntityId,
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
    /// ## 这里是混响延迟线的**唯一**分配点
    ///
    /// 本函数是**非实时**路径（调用方必须在打开设备之前建好运行时），因此它是
    /// [`Reverb::set_sample_rate`] 唯一合法的地方：那个方法会 `Vec` 重分配 + 释放
    /// 延迟线（[`crate::insert`] 模块文档 §8.4 / §8.5）。采样率取自 `slot` 里的
    /// **初始快照** —— 两个调用方（`EngineHost::reload` 与设备腿）都是先用工程建好
    /// 快照、再建运行时，因此这里拿到的是本工程将要用的那一个采样率。
    /// `slot.current()` 会在写者锁上短暂等待，因此**只允许**在非实时路径调用。
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
        // 混响延迟线的**唯一**分配点（[`crate::insert`] 模块文档 §8.4 / §8.5）：
        // 采样率取自**初始快照**——调用方（`EngineHost::reload` / `device` 腿）都是
        // 先用工程建好快照、再建运行时，因此这里拿到的就是本工程将要用的采样率。
        // `SnapshotSlot::current` 会在写者锁上短暂等待，因此只允许在**非实时**路径调用
        // （本函数就是那条路径：全部通道必须在打开设备之前建立）。
        let armed_reverb_sample_rate = slot.current().sample_rate();
        let mut reverb_pool: [Reverb; MAX_TRACK_SLOTS] = core::array::from_fn(|_| Reverb::new());
        for reverb in &mut reverb_pool {
            // 构造期允许分配（`Vec` 延迟线在这里建好，回调内一次也不重建）。
            reverb.set_sample_rate(armed_reverb_sample_rate as f32);
        }
        // 卷积混响池的**唯一**分配点（[`crate::insert`] 模块文档 §9.2／§9.5）：预延迟线
        // ＋ 四条 IR 频谱。两者都按**引擎常量**的 IR 长度建满，于是快照边界上的换 IR
        // 永远是"长度不变"的那条路径 ⇒ 零分配。
        //
        // ⚠ 占位 IR 是**全零**而不是空：空 IR 在器件里是"拒绝"（回到未配置直通，
        // 缓冲容量保留但内容被清），而我们要的是**缓冲长度**被建出来。全零非空 ⇒ 校验通过
        // ⇒ `span` 从此固定。未武装的槽位永远不会被逐样本路径查表命中（查的是
        // `armed_convs`），因此这份占位数据不会出声。
        let armed_conv_sample_rate = armed_reverb_sample_rate;
        let conv_placeholder_frames = crate::insert::convolution_ir_frames(armed_conv_sample_rate);
        let placeholder = vec![0.0f32; conv_placeholder_frames];
        // ⚠ 逐台建、再 `into_boxed_slice`，**不**写 `Box::new([...; 16])`：后者会先造一个
        // 约 81 KiB 的栈上临时量（那正是本节要避免的东西，见字段文档的栈溢出读数）。
        let mut conv_pool: Vec<ConvolutionReverb> = Vec::with_capacity(MAX_TRACK_SLOTS);
        for _ in 0..MAX_TRACK_SLOTS {
            let mut conv = ConvolutionReverb::new();
            conv.set_sample_rate(armed_conv_sample_rate as f32);
            conv.set_impulse_response(&placeholder, &placeholder, &placeholder, &placeholder);
            conv_pool.push(conv);
        }
        let conv_pool = conv_pool.into_boxed_slice();
        // 参数目标表：同样在**构造期**按初始快照的采样率建满（每个平滑器的 `α`
        // 含一次 `exp`；回调内一次也不重算 —— 采样率不变时 `set_sample_rate` 直接返回）。
        let params = ParamTable::new(armed_reverb_sample_rate as f32);
        let armed_master = slot.current().master();
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
            // 统计镜像：**构造期**分配一次（回调内只做原子存）[MUST-GATE-001]。
            stats_mirror: Arc::new(EngineStatsMirror::new()),
            limiter: BusLimiter::new(),
            // PDC 延迟线池：**构造期**按上限预分配（回调内绝不再分配）。
            pdc: CompensationBank::preallocated(PDC_SLOTS, MAX_PDC_DELAY_FRAMES),
            pdc_unarmed_nodes: 0,
            pdc_clamped_frames: 0,
            pdc_alignment_frames: 0,
            engine_output_latency_frames: 0,
            armed_pan_law: PanLaw::default(),
            armed_pan_gains: [(
                EntityId::default(),
                core::f32::consts::FRAC_1_SQRT_2,
                core::f32::consts::FRAC_1_SQRT_2,
            ); MAX_TRACK_SLOTS],
            armed_pan_slots: 0,
            // 声相平滑器：**构造期**预分配（回调内绝不再分配）。`with_default_time`
            // 需要一个采样率来算 α；真正的采样率在快照边界用 `set_sample_rate` 校准
            // （同值提前返回 ⇒ 不重复算 `exp`）。
            // 初值采样率用 48 kHz（与 `ParamTable::new` 的判据默认值同一个数）；
            // 真正的采样率在**快照边界**用 `set_sample_rate` 校准（同值提前返回）。
            armed_pan_l: core::array::from_fn(|_| ParamSmoother::with_default_time(48_000.0)),
            armed_pan_r: core::array::from_fn(|_| ParamSmoother::with_default_time(48_000.0)),
            armed_pan_armed: [false; MAX_TRACK_SLOTS],
            pan_automation_frames: 0,
            pan_automation_value_rejects: 0,
            armed_master_gain: 1.0,
            armed_metronome_enabled: false,
            armed_metronome_ticks_per_beat: 0,
            armed_metronome_beats_per_bar: 0,
            metronome: MetronomeVoice::new(),
            limiter_gain_reductions: 0,
            limiter_max_reduction: 0.0,
            limiter_current_reduction: 0.0,
            // 插入链：**构造期**预分配全部槽位（回调内绝不再分配）。
            // `from_fn` 而不是 `[expr; N]`：`ChannelStrip` 没有 `const` 构造器，
            // 而 `Option<ChannelStrip>` 的"全 `None`"初值用函数形式表达最直接
            // （`from_fn` 在**构造期**调用，不在回调里）。
            armed_strips: core::array::from_fn(|_| (EntityId::default(), None)),
            armed_insert_slots: 0,
            insert_gain_reductions: 0,
            insert_max_reduction_db: 0.0,
            insert_current_reduction_db: 0.0,
            // 冷值就是 `None`（= `EngineStats::default()` 的派生值）：`0.0` 是一个合法
            // 的检波器电平（0 dBFS）⇒ 不能用它表示"从没有过读数"。
            insert_detector_level_db: None,
            insert_strip_frames: 0,
            insert_reverb_frames: 0,
            insert_reverb_rate_rejects: 0,
            insert_convolution_frames: 0,
            insert_convolution_rejects: 0,
            // 参数目标表：**构造期**建满（回调内惰性分配的槽位都落在这张定长表里）。
            params,
            armed_master,
            // 混响延迟线池：**构造期**按初始快照的采样率预分配（回调内绝不再分配）。
            reverb_pool,
            armed_reverbs: [(EntityId::default(), false); MAX_TRACK_SLOTS],
            armed_reverb_slots: 0,
            armed_reverb_sample_rate,
            reverb_scratch: [[0.0; DEFAULT_BLOCK_FRAMES]; 2],
            // 卷积混响池：**构造期**按初始快照的采样率把占位 IR 与预延迟线全部建好
            // （回调内只做同长度的换 IR 与 `set_params`）。
            conv_pool,
            armed_convs: [(EntityId::default(), false); MAX_TRACK_SLOTS],
            armed_conv_slots: 0,
            armed_conv_ir_hashes: [0; MAX_TRACK_SLOTS],
            armed_conv_sample_rate,
            conv_scratch: [0.0; 2 * DEFAULT_BLOCK_FRAMES],
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
        // 统计镜像也先发布一次：控制面在**第一个量子之前**就能读到与引擎实际状态
        // 相符的数（`ftz` 与走带状态就是这一批里最有用的两个），而不是全零冷值。
        // 与上面那条同一个理由、同一个位置（非实时路径）。
        let cold = runtime.stats();
        runtime.stats_mirror.publish(&cold);
        runtime
    }

    /// 累计统计的**跨线程只读镜像**（构造期建好；`Arc` 的一份克隆）。
    ///
    /// 用途：设备腿把 [`Self`] 移进 cpal 回调之后，控制线程再也拿不到 `&Self`
    /// ⇒ 想要引擎健康读数就只能读这份镜像（[`EngineStatsMirror::read`]）。
    /// 取句柄**必须在移走运行时之前**（克隆 `Arc`，不分配）。
    ///
    /// ⚠ 这是 [`Self::stats`] 的**搬运**，不是第二份事实源：写入点只有
    /// [`Self::process_quantum`] 的每量子发布（值取自同一个 [`Self::stats`]）
    /// 与 [`Self::new`] 的初值发布。判据：静止点上 `stats_mirror().read()` 与
    /// `stats()` **逐字段相等**（`tests/synth_rt_zero_alloc.rs` 场景 ⑮）。
    #[must_use]
    pub fn stats_mirror(&self) -> Arc<EngineStatsMirror> {
        Arc::clone(&self.stats_mirror)
    }

    /// 处理一个（可能是任意长度的）输出缓冲：按 [`DEFAULT_BLOCK_FRAMES`] 切成整量子。
    ///
    /// `output` 是 cpal 给出的**交错**缓冲；`channels` 是通道数。长度不是
    /// `channels * k` 时，尾部不足一帧的样本保持原值（cpal 保证输出缓冲预填静音）。
    ///
    /// **通道映射（已由判据钉住）**：`ch0 = 左`、`ch1..chN-1 = 右`（判据
    /// `the_interleaved_output_maps_channel_zero_to_left_and_the_rest_to_right`，
    /// N = 1/2/3/4/6/8 逐帧逐路逐位）。⚠ `channels == 1` 时因此**只写左声道**：
    /// 这是当前的**映射**行为，不是"单声道设备应当怎么混"的裁决 —— 把它改成
    /// `(L + R) · 0.5` 之类的下混会改变渲染输出（实测注入：判据 ⑥-10 的 N = 1
    /// 分支当场变红）⇒ 修法属**裁决**，措辞写在
    /// `tests/idempotency_and_channel_consistency.rs` 的模块文档 §4 发现 1。
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
            // ⚠ `channels == 1` ⇒ 只有第 0 路（= 左）被写：右声道的内容在单声道设备上
            // **无处可去**。这是**待裁决**的已知缺口，不是可以就地改掉的小事：
            // 见 `tests/idempotency_and_channel_consistency.rs` 的模块文档 §4 发现 1。
            for channel in 0..channels {
                for frame in 0..frames {
                    let sample = self.block.get(channel, frame).unwrap_or(0.0);
                    output[(offset + frame) * channels + channel] = sample;
                }
            }
            offset += frames;
        }
        // 帧边界对齐的交错缓冲不会留下尾巴；非对齐的残余保持 cpal 预填的静音。
        //
        // 收尾：把**此刻**的累计统计发布到跨线程镜像（[`Self::stats_mirror`]）。
        // 位置取"本回调全部量子都渲染完之后"而不是每个量子中间：镜像的读者是控制面，
        // 它要的是"现在引擎处在什么状态"，中间的过渡值对它没有意义；而一次回调发布
        // 一次也把原子存的总量压到最低。发布本身只做原子存（零分配/零锁/零 I/O）。
        let stats = self.stats();
        self.stats_mirror.publish(&stats);
    }

    /// 当前累计统计（**只读快照**：无锁、零分配；读它不会影响渲染路径）。
    ///
    /// 它把控制面必须能看见的**引擎健康读数**一并交出（`needs` N2/N5）：
    /// [`EngineStats::snapshot_stash_events`]（"追不上"）、
    /// [`EngineStats::retire_pending`] / [`EngineStats::retire_drained`] /
    /// [`EngineStats::retire_pruned`]（退役回收的两条路径）、
    /// [`EngineStats::release_thread_is_main`] / [`EngineStats::foreign_drains`]
    /// （"释放发生在哪个线程"）、以及电平 SPSC 的丢帧读数
    /// [`EngineStats::meter_dropped_frames`]（"UI 有没有落后到丢帧"）。判定见
    /// [`EngineStats::is_snapshot_lagging`] / [`EngineStats::stash_events_since_last_read`]
    /// 与 [`EngineStats::is_meter_lagging`] / [`EngineStats::meter_dropped_since_last_read`]。
    ///
    /// ⚠ 本函数要 `&self` ⇒ 只有在**调用者就是音频线程**（或运行时尚未移走）时才能用。
    /// 运行时归 cpal 回调线程所有之后，控制面请读 [`Self::stats_mirror`]
    /// （同一份读数的跨线程镜像；每量子发布一次）。
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
            // 不在这里重数一遍：读数就是发布端自己那一个计数器（`publish` 里
            // `frames.len() - pushed` 的累计）。**零分配、零锁、零 I/O**：一次 `u64` 字段读。
            meter_dropped_frames: self.meters.dropped(),
            ftz: self.ftz,
            rendered_samples: self.synth.position(),
            scheduled_notes: self.armed_scheduled_notes,
            note_schedule_drops: self.armed_note_schedule_drops,
            voice_steals: self.synth.voice_steals(),
            track_drops: self.synth.track_drops(),
            notes_triggered: self.synth.notes_triggered(),
            // 复音触发面的**另一侧**（器件侧）：与上面那一条只差一次字段汇总
            // （`SynthEngine::poly_notes_triggered` 的 `fold` 16 个槽位）。
            // 它不碰任何样本、不分配、不加锁 ⇒ 实时路径的分配/锁/IO 读数不变
            // （判据见 `tests/synth_rt_zero_alloc.rs` 的场景 11）。
            poly_notes_triggered: self.synth.poly_notes_triggered(),
            // 复音声部池的**瞬时占用**（量规）：与上面那条同一个 `fold` 形状，
            // 只是把 `u64` 字段求和换成把每槽的 `active` 布尔计数。不碰样本、
            // 不分配、不加锁 ⇒ 渲染输出逐位不变。
            poly_active_voices: self.synth.poly_active_voices(),
            limiter_gain_reductions: self.limiter_gain_reductions,
            limiter_max_reduction: self.limiter_max_reduction,
            limiter_current_reduction: self.limiter_current_reduction,
            insert_gain_reductions: self.insert_gain_reductions,
            insert_max_reduction_db: self.insert_max_reduction_db,
            insert_current_reduction_db: self.insert_current_reduction_db,
            insert_detector_level_db: self.insert_detector_level_db,
            insert_strip_frames: self.insert_strip_frames,
            insert_reverb_frames: self.insert_reverb_frames,
            insert_reverb_rate_rejects: self.insert_reverb_rate_rejects,
            insert_convolution_frames: self.insert_convolution_frames,
            insert_convolution_rejects: self.insert_convolution_rejects,
            param_gain_frames: self.params.gain_frames(),
            param_master_gain_frames: self.params.master_gain_frames(),
            param_gain_rejects: self.params.rejections(),
            param_unmapped_events: self.params.unmapped(),
            param_capacity_drops: self.params.capacity_drops(),
            pdc_unarmed_nodes: self.pdc_unarmed_nodes,
            pdc_clamped_frames: self.pdc_clamped_frames,
            pdc_processed_blocks: self.pdc.processed_blocks(),
            pdc_alignment_frames: self.pdc_alignment_frames,
            engine_output_latency_frames: self.engine_output_latency_frames,
            metronome_clicks: self.metronome.clicks(),
            drum_hits: self.synth.drum_hits(),
            // 鼓机器件**自己**的四个计数器：与上面那条只差一次字段汇总
            // （`SynthEngine` 的四个 `drum_*` getter，各自 fold 16 个 `u64`）。
            // 它们不碰任何样本、不分配、不加锁 ⇒ 实时路径的分配/锁/IO 读数不变
            // （判据见 `tests/synth_rt_zero_alloc.rs` 的场景 11）。
            drum_triggers: self.synth.drum_triggers(),
            drum_voice_steals: self.synth.drum_voice_steals(),
            drum_hat_chokes: self.synth.drum_hat_chokes(),
            drum_sounding_slot_frames: self.synth.drum_sounding_slot_frames(),
            // 鼓机槽位池的**瞬时占用**（量规）：与复音那一侧分开记账（两个独立的池），
            // 与上面一族的累计量口径刻意不同。同样不碰样本、不分配、不加锁。
            drum_active_slots: self.synth.drum_active_slots(),
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

    /// **见证**：PDC 补偿延迟线的槽位**换过主人**的累计次数
    /// （[`CompensationBank::rebindings`](crate::graph::CompensationBank::rebindings)）。
    ///
    /// 槽位是**按计划键序的下标**绑定的（`compensation` 是按键升序的 `BTreeMap`）
    /// ⇒ 工程里删掉一个节点就会让它后面的节点各下移一格，新节点继承上一任的环。
    /// 修法是"换主人就清线"（见 `CompensationBank::rearm` 的文档）。
    ///
    /// 存在理由与 [`Self::snapshot_stash_events`] 同族：清线是**唯一**会读/写旧历史
    /// 的配置变更路径，而"走过它"与"没走过"在四元组读数上完全一样（两支都不分配、
    /// 不加锁、不做 I/O）⇒ 零分配判据必须用这个数证明自己没有空转。
    /// 判据：`tests/synth_rt_zero_alloc.rs` 的场景 20 与 `tests/rt_zero_alloc.rs` 的 ⑰d。
    #[must_use]
    pub const fn pdc_rebindings(&self) -> u64 {
        self.pdc.rebindings()
    }

    /// **声相自动化的覆盖度见证**：被**平滑**声相乘过的帧数（累计）。
    ///
    /// 为什么需要它：那条平滑路径至少在两个可观测上与常量路径相同（都不分配、
    /// 都不加锁）⇒ 零分配判据必须用这个数证明"自己不是空转"。
    /// 判据见 `tests/rt_zero_alloc.rs` 的声相自动化窗口。
    #[must_use]
    pub const fn pan_automation_frames(&self) -> u64 {
        self.pan_automation_frames
    }

    /// **不静默**：被拒绝的声相自动化**值**数（非有限 / 负数）。
    #[must_use]
    pub const fn pan_automation_value_rejects(&self) -> u64 {
        self.pan_automation_value_rejects
    }

    /// **本量子**已计量的节点次数
    /// （[`MeterBank::active_nodes`](crate::meter::MeterBank::active_nodes) 的搬运）。
    ///
    /// ## 它为什么存在（`line/engine-26` 的注入 R13 实测）
    ///
    /// 每个量子开始时 [`EngineRuntime::render_block`] 调用一次
    /// `bank.begin_quantum()`，而那个方法的**唯一**效果是把计量池里的 `measured` 归零。
    /// 在补上本访问器之前，`measured` 在 `EngineRuntime` 之外**没有任何读者**
    /// （量法：`grep -rn active_nodes crates/yeban-engine/src crates/yeban-engine/tests`
    /// 只命中 `meter.rs` 自己的定义与它自己的三条单测；`EngineStats` 不搬它）
    /// ⇒ 把那个调用整句删掉，产物与**全部**既有判据都看不出差别
    /// （本票注入 R13 实测：`cargo test -p yeban-engine --no-default-features --all-targets`
    /// 之下**没有任何判据**因它变红。⚠ `snapshot_retire_churn` 的 60Hz 节拍判据在本机
    /// 高负载时本来就红 —— 它与本注入无关，见本票交付报告里的负载读数）。
    ///
    /// 因此本访问器是那条接线的**可读见证**，与 [`Self::pdc_rebindings`] /
    /// [`Self::armed_metronome_ticks_per_beat`] 同族：把"内部计数真的每量子重置"
    /// 变成可判定的差分（判据见 `rt::tests::meter_active_nodes_is_per_quantum_not_cumulative`）。
    ///
    /// 它只读一个 `usize` 字段：零分配、零锁、零 I/O。
    ///
    /// 口径：数的是本量子走 `MeterBank::measure`（**逐轨槽位**）的次数 ——
    /// 母线走 `measure_bus_stereo`，它有自己的检测器、**不计入**此数
    /// （`meter.rs` 的 `MeterBank::measure` 是唯一推进它的入口）。
    #[must_use]
    pub const fn meter_active_nodes(&self) -> usize {
        self.bank.active_nodes()
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

    /// 本快照武装的**每轨插入器件（通道条）参数**（诊断/判据用）。
    ///
    /// 返回 `None` = 这条轨**没有**插入器件（实时侧整段跳过）。
    /// 存在的理由与 [`Self::armed_pan_gain`] 同族：把"武装进去的那个数"变成**可读**的，
    /// 判据就不必从音频输出反推 —— 引擎线第一次接入声相时的那个 bug
    /// （武装表与投影各自都对、错在武装表）正是这样定位的。
    /// [`crate::insert`] 模块文档 §6 说明了本读数的**构造期/逐样本**归属
    /// （`c792fdc` 的同一处注释写的是 §5；本票新增 §4.1–§4.3 之后实时分类后移为 §6）。
    #[must_use]
    pub fn armed_strip(&self, track: &EntityId) -> Option<ChannelStripParams> {
        self.armed_strips[..self.armed_insert_slots]
            .iter()
            .find(|(id, _)| id == track)
            .and_then(|(_, strip)| strip.as_ref())
            .map(ChannelStrip::params)
    }

    /// 本快照武装的**每轨插入器件的动态级参数**（通道条 `compressor` 字段的直读）。
    ///
    /// `c792fdc` 的判据按这个读数对账（那时槽位里装的是裸压缩器）。
    /// 本票把槽位换成通道条之后，这个读数的**含义不变**：它仍是"本轨动态级的参数"。
    #[must_use]
    pub fn armed_compressor(&self, track: &EntityId) -> Option<CompressorParams> {
        self.armed_strip(track).map(|strip| strip.compressor)
    }

    /// 武装表里的**插入器件**条数。
    #[must_use]
    pub const fn armed_insert_slot_count(&self) -> usize {
        self.armed_insert_slots
    }

    /// 参数目标表里**已分配**的槽位数（[`crate::param`]；诊断/判据用）。
    ///
    /// 与 [`Self::armed_insert_slot_count`] 的区别：那张表由**快照**武装，
    /// 这张表由**事件**武装 ⇒ 它证明"事件真的建了槽位"，而不是"快照里有参数"。
    #[must_use]
    pub const fn armed_param_slot_count(&self) -> usize {
        self.params.slot_count()
    }

    /// 某条轨当前的**参数增益乘子目标值**（`None` = 本表里没有它的槽位）。
    ///
    /// 与 [`Self::armed_reverb`] 同族：把"武装进去的那个数"变成**可读**的，
    /// 判据不必只从音频输出反推。
    #[must_use]
    pub fn armed_param_target(&self, track: &EntityId) -> Option<f32> {
        self.params.target(*track)
    }

    /// 某条轨当前的**参数增益乘子输出值**（平滑中的瞬时值；`None` = 没有槽位）。
    ///
    /// `is_settled` 的那一档可以逐位断言：平滑走完之后它与
    /// [`Self::armed_param_target`] 逐位相等（器件自己的吸附语义，
    /// 见 `yeban_dsp::smoothing`）。
    #[must_use]
    pub fn armed_param_gain(&self, track: &EntityId) -> Option<f32> {
        self.params.gain(*track)
    }

    /// 主总线参数槽位当前的**目标值**（`None` = 从未收到过被接受的目标）。
    ///
    /// 与 [`Self::armed_param_target`] 同族，但主总线只有**一格**、且它不属于
    /// 逐轨槽位表（[`crate::param`] 模块文档 §2.3）⇒ 用"是否武装过"表达
    /// "有没有槽位"这一维。
    #[must_use]
    pub fn armed_master_param_target(&self) -> Option<f32> {
        self.params
            .master_armed()
            .then(|| self.params.master_target())
    }

    /// 主总线参数槽位当前的**输出值**（平滑中的瞬时值；同上口径）。
    #[must_use]
    pub fn armed_master_param_gain(&self) -> Option<f32> {
        self.params
            .master_armed()
            .then(|| self.params.master_gain())
    }

    /// 本快照武装的**每轨插入器件（混响）参数**（诊断/判据用）。
    ///
    /// 返回 `None` = 这条轨**没有**混响（实时侧整段跳过）。
    /// 与 [`Self::armed_strip`] 同族，但读数取自器件自己的 `params()`（混响没有
    /// "级开关"这一类中间读数，模块文档 §8.2）。
    #[must_use]
    pub fn armed_reverb(&self, track: &EntityId) -> Option<ReverbParams> {
        // 槽位下标就是池的下标（同一张表）⇒ 先找下标，再读池里那一台的当前参数。
        self.armed_reverbs[..self.armed_reverb_slots]
            .iter()
            .position(|(id, armed)| *armed && id == track)
            .map(|index| self.reverb_pool[index].params())
    }

    /// 武装表里的**混响**条数。
    #[must_use]
    pub const fn armed_reverb_slot_count(&self) -> usize {
        self.armed_reverb_slots
    }

    /// 混响延迟线池武装时用的采样率（**构造期**定；诊断/判据用）。
    #[must_use]
    pub const fn armed_reverb_sample_rate(&self) -> u32 {
        self.armed_reverb_sample_rate
    }

    /// 本快照武装的**每轨插入器件（卷积混响）参数**（诊断/判据用）。
    ///
    /// 返回 `None` = 这条轨**没有**卷积混响（实时侧整段跳过）。
    /// 与 [`Self::armed_reverb`] 同族，读数取自器件自己的 `params()`（本器件同样没有
    /// "级开关"这一类中间读数）。
    #[must_use]
    pub fn armed_convolution(&self, track: &EntityId) -> Option<ConvolutionReverbParams> {
        // 槽位下标就是池的下标（同一张表）⇒ 先找下标，再读池里那一台的当前参数。
        self.armed_convs[..self.armed_conv_slots]
            .iter()
            .position(|(id, armed)| *armed && id == track)
            .map(|index| self.conv_pool[index].params())
    }

    /// 武装表里的**卷积混响**条数。
    #[must_use]
    pub const fn armed_convolution_slot_count(&self) -> usize {
        self.armed_conv_slots
    }

    /// 卷积混响池武装时用的采样率（**构造期**定；诊断/判据用）。
    ///
    /// ⚠ 它同时是"IR 帧数"的权威来源（帧数 = 采样率 ÷ 10，
    /// 见 [`crate::insert::convolution_ir_frames`]）。
    #[must_use]
    pub const fn armed_convolution_sample_rate(&self) -> u32 {
        self.armed_conv_sample_rate
    }

    /// 某条轨**武装进去的 IR 帧数**（诊断/判据用；`None` = 没有武装卷积混响）。
    ///
    /// 存在的理由：判据要能证明"武装进去的 IR 就是投影合成的那一条"，
    /// 而器件自己不公开它的 IR ⇒ 这里读器件真正接受的帧数。
    #[must_use]
    pub fn armed_convolution_ir_frames(&self, track: &EntityId) -> Option<usize> {
        self.armed_convs[..self.armed_conv_slots]
            .iter()
            .position(|(id, armed)| *armed && id == track)
            .map(|index| self.conv_pool[index].ir_frames())
    }

    /// 本快照武装的**每轨鼓机音源**（[`crate::drums`]；诊断/判据用）。
    ///
    /// 与 [`Self::armed_reverb`] 同族：把"武装进去的那一份投影"变成**可读**的，
    /// 判据不必从音频输出反推。`None` = 该轨的音源是复音合成器（默认口径）。
    /// 鼓机与插入链**不是**同一张表：它住在 [`SynthEngine`] 里（音源在上游，
    /// 见 [`crate::drums`] 模块文档 §2）。
    #[must_use]
    pub fn armed_drums(&self, track: &EntityId) -> Option<crate::drums::DrumsParams> {
        self.synth.drum_params(*track)
    }

    /// 武装为**鼓机音源**的槽位数（诊断/判据用）。
    #[must_use]
    pub fn armed_drum_slot_count(&self) -> usize {
        self.synth.armed_drum_slots()
    }

    /// 本快照武装的**每轨音色参数**（[`crate::synth::ToneParams`]；诊断/判据用）。
    ///
    /// 与 [`Self::armed_drums`] 同族（同一个上游对象的只读投影）：把"武装进去的那个数"
    /// 变成**可读**的，判据不必只从音频输出反推。`None` = 该轨没有占槽或不在本快照里。
    /// 波形下标与第二条支路的电平/失谐都在返回值里（[`crate::synth::ToneParams`] 的四个
    /// `osc*` 读数）⇒ "换了一份快照之后振荡器参数真的到了实时侧"可以直接断言。
    #[must_use]
    pub fn armed_tone(&self, track: &EntityId) -> Option<crate::synth::ToneParams> {
        self.synth.tone_params(*track)
    }

    /// 本快照武装的**主总线线性增益**（诊断/判据用）。
    ///
    /// 与 [`Self::armed_pan_gain`] 同族：把"武装进去的那个数"变成**可读**的，
    /// 判据不必从音频输出反推。单位增益（`1.0`，默认 0 dB）时逐样本路径**整段跳过**。
    #[must_use]
    pub const fn armed_master_gain(&self) -> f32 {
        self.armed_master_gain
    }

    /// 本快照武装的**节拍器开关**（`transport.metronome_enabled` 的投影；
    /// 诊断/判据用）。
    ///
    /// 与 [`Self::armed_master_gain`] 同族：它证明"模型字段真的走到了实时侧"，
    /// 而不是"快照里有、实时侧没读"。`false` 时逐样本路径**整段跳过**。
    #[must_use]
    pub const fn armed_metronome_enabled(&self) -> bool {
        self.armed_metronome_enabled
    }

    /// 本快照武装的**每拍 tick 数**（`PPQ × 4 / 拍号分母`；诊断/判据用）。
    #[must_use]
    pub const fn armed_metronome_ticks_per_beat(&self) -> u64 {
        self.armed_metronome_ticks_per_beat
    }

    /// 本快照武装的**一小节拍数**（拍号分子；诊断/判据用）。
    #[must_use]
    pub const fn armed_metronome_beats_per_bar(&self) -> u64 {
        self.armed_metronome_beats_per_bar
    }

    /// 累计触发过的咔哒声次数（= [`EngineStats::metronome_clicks`]）。
    #[must_use]
    pub const fn metronome_clicks(&self) -> u64 {
        self.metronome.clicks()
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
            limiter_current_reduction,
            armed_strips,
            armed_insert_slots,
            insert_gain_reductions,
            insert_max_reduction_db,
            insert_current_reduction_db,
            insert_detector_level_db,
            insert_strip_frames,
            reverb_pool,
            armed_reverbs,
            armed_reverb_slots,
            armed_reverb_sample_rate,
            reverb_scratch,
            insert_reverb_frames,
            insert_reverb_rate_rejects,
            conv_pool,
            armed_convs,
            armed_conv_slots,
            armed_conv_ir_hashes,
            armed_conv_sample_rate,
            conv_scratch,
            insert_convolution_frames,
            insert_convolution_rejects,
            params,
            armed_master,
            // 声相一族（P4=(b)）：事件循环要写平滑器的目标，逐样本路径要推进它们。
            armed_pan_gains,
            armed_pan_slots,
            armed_pan_l,
            armed_pan_r,
            armed_pan_armed,
            pan_automation_frames,
            pan_automation_value_rejects,
            pdc_unarmed_nodes,
            pdc_clamped_frames,
            pdc_alignment_frames,
            engine_output_latency_frames,
            armed_revision,
            armed_master_gain,
            armed_metronome_enabled,
            armed_metronome_ticks_per_beat,
            armed_metronome_beats_per_bar,
            metronome,
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
        pan_gains[..*armed_pan_slots].copy_from_slice(&armed_pan_gains[..*armed_pan_slots]);

        *quanta = quanta.wrapping_add(1);
        let quantum = *quanta;

        // --- 1) 参数/音符/**走带**事件：每块一次批量出队 [ROAD-M2-007] ---
        //
        // 走带命令在**量子边界**按 FIFO 顺序应用 ⇒ "同一输入序列 ⇒ 同一 tick 轨迹"
        // （确定性来自"命令在哪一个量子生效"只由出队顺序决定，与墙钟无关）。
        // `SeekTicks` 必须同时把合成器播放头挪到目标位置，否则"定位"只改数字、不出声。
        //
        // `SetParam` 交给**参数目标表**（[`crate::param`]）：本块只更新目标值，
        // 逐样本的平滑与应用在下面的逐轨循环里（位置见 `params.apply` 的调用点）。
        // 三类"没有生效"的裁决各自计数 ⇒ "事件到了但什么都没发生"永远可见
        // [ARCH-DSP-001, MUST-GATE-001]。
        let mut applied = 0usize;
        events.drain_with(scratch_events, |event| {
            if !event.is_idle() {
                applied += 1;
            }
            if let EngineEvent::SetParam { target, value } = event {
                // --- 声相自动化：**先分流**（裁决 P4=(b)）---
                //
                // `ParamTable` 只认 `TRACK_GAIN_SLOT`（0）／`MASTER_GAIN_SLOT`（1）：
                // 声相的两个槽位（2/3）**必须在这里被截走**，否则它们会落进
                // `params.accept` 被判成 `Unmapped` 并计数 —— 声相就永远不动，而
                // 那正是"公开面能写、效果被静默丢弃"这一类缺陷。
                // 截走之后写的是那条**独立的**逐轨声相平滑对（逐样本只做乘加）。
                // ⚠ 只在**找得到槽位**时截走：找不到（母线、还没武装的快照、超出
                // `MAX_TRACK_SLOTS` 的轨）就原样交给 `params.accept` ⇒ 由它判
                // `Unmapped` 并计数（**单一归属**：未映射的裁决只有一处）。
                // 这样 slot 2/3 的两条读数都有机械见证：
                //   * 打得到槽位 ⇒ 生效，`param_unmapped_events` **不动**；
                //   * 打不到槽位 ⇒ `param_unmapped_events` +1（既有判据因此逐字不变）。
                let pan_index =
                    if target.slot == TRACK_PAN_LEFT_SLOT || target.slot == TRACK_PAN_RIGHT_SLOT {
                        armed_pan_gains[..*armed_pan_slots]
                            .iter()
                            .position(|(id, _, _)| *id == target.entity)
                    } else {
                        None
                    };
                if let Some(index) = pan_index {
                    if !value.is_finite() || value < 0.0 {
                        // 与 `ParamTable::accept` 同一条口径：非有限 / 负值 ⇒ **计数**拒绝。
                        *pan_automation_value_rejects =
                            pan_automation_value_rejects.wrapping_add(1);
                    } else {
                        if target.slot == TRACK_PAN_LEFT_SLOT {
                            armed_pan_l[index].set_target(value);
                        } else {
                            armed_pan_r[index].set_target(value);
                        }
                        armed_pan_armed[index] = true;
                    }
                } else {
                    // 三类"没有生效"的裁决（非法值 / 未映射地址 / 容量不足）由**表自己**
                    // 计数，随后经 `EngineStats::param_*` 读出 ⇒ 这里不重复记账。
                    let _ = params.accept(target, value, *armed_master);
                }
            }
            if let EngineEvent::Transport { command } = event
                && let TransportEffect::Seeked { frames, .. } = transport.apply(command)
            {
                synth.seek(frames);
                // 节拍器也要重新对齐：从第 8 小节跳回第 1 小节之后，下一拍必须落在
                // 新位置之后的拍栅格上，而不是继续等第 9 小节的强拍（`ticks_per_beat`
                // 为 0 = 还没有任何快照被武装 ⇒ `resync` 不发明栅格）。
                metronome.resync(transport.position_ticks(), *armed_metronome_ticks_per_beat);
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

                // --- 2a') 参数目标表的采样率与主总线身份（每修订一次）---
                // [crate::param]。采样率只用来重算平滑器的 `α`（含 `exp`）⇒ 属 [ADR-0001 D32]
                // 的**超越函数类**，只能在快照边界做；采样率没变时器件自己直接返回。
                // 主总线身份供**事件**边界判定"这个地址本表不消费"（母线增益的槽位未开，
                // 见 `crate::param` 模块文档 §2 第 2 条）。
                params.set_sample_rate(current.sample_rate() as f32);
                *armed_master = master;

                // --- 2b) 声部池对齐到新快照的轨道集合（每修订一次, 非逐样本）---
                // 新轨道占槽、消失的轨道标记 absent（状态保留）、游标**只增不减**地校正
                // ⇒ 已触发过的音符绝不重复触发，在鸣的音符不被快照切换切断。
                synth.begin_snapshot(
                    current.sample_rate(),
                    current.tracks().keys().filter(|id| **id != master),
                    current.tones().iter().filter(|(id, _)| **id != master),
                    current.drums().iter().filter(|(id, _)| **id != master),
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

                // --- 2b''') PDC 计划的**两个延迟读数**：纯读、不碰任何样本 ---
                // [ARCH-PDC-001, ARCH-PDC-002]。此前这两个数**只算不读**：
                // `PdcPlan` 在控制线程算好，实时侧只消费逐节点的 `D(v)`，
                // 于是"引擎输出延迟（`L_max` ＋ 母线求和之后的 `master` 自身延迟）"
                // 在整条引擎链路上无法读到（量法见
                // [`EngineStats::engine_output_latency_frames`] 的字段文档）。
                //
                // 位置刻意与 `rearm` 同一个快照边界：两者读的是**同一份**计划
                // ⇒ "武装了什么"与"读到的延迟是多少"不可能指向两个修订。
                // 逐量子不重读（计划在快照生命周期内不变）⇒ 实时路径只多两次
                // `u32` 拷贝；**零分配、零锁、零 I/O、零日志** [MUST-GATE-001]。
                *pdc_alignment_frames = current.pdc().total_latency();
                *engine_output_latency_frames = current.pdc().output_latency();

                // --- 2c) 声相增益：在**构造期语义**下算一次（`cos`/`sin` 属超越函数类,
                // 不进逐样本路径）。`pan_law` 与 `pan` 在整份快照的生命周期内不变。
                // 表先写进 `self`（权威副本，诊断可读），再刷新栈上那份 —— 顺序无所谓，
                // 但**必须在本量子的逐轨循环之前**（第一版的顺序错误见函数文档）。
                self.armed_pan_law = current.pan_law();
                *armed_pan_slots = 0;
                let pan_sample_rate = current.sample_rate() as f32;
                for (id, params) in current.tracks() {
                    if *id == master || *armed_pan_slots >= MAX_TRACK_SLOTS {
                        continue;
                    }
                    let (gain_l, gain_r) = params.pan_gains(self.armed_pan_law);
                    let slot = *armed_pan_slots;
                    // ⚠ **换主人**（类别③）：槽位是按下标复用的，新主人**不得**继承
                    // 上一任的声相自动化目标 —— 那会把另一条轨的声相播给这一条。
                    // 判据见 `a_pan_slot_changing_owner_does_not_inherit_the_automation`。
                    if armed_pan_gains[slot].0 != *id {
                        armed_pan_armed[slot] = false;
                    }
                    armed_pan_gains[slot] = (*id, gain_l, gain_r);
                    // 平滑器的 α 跟着快照的采样率（同值提前返回 ⇒ 不重复算 `exp`）；
                    // **基值**也跟着新快照走 —— 但**在飞的自动化不被覆盖**（裁决：
                    // "手动改工程 ≠ 覆盖在飞的自动化"；改工程之后**新的**事件继续生效）。
                    armed_pan_l[slot].set_sample_rate(pan_sample_rate);
                    armed_pan_r[slot].set_sample_rate(pan_sample_rate);
                    if !armed_pan_armed[slot] {
                        armed_pan_l[slot].snap_to(gain_l);
                        armed_pan_r[slot].snap_to(gain_r);
                    }
                    *armed_pan_slots += 1;
                }
                pan_gains[..*armed_pan_slots].copy_from_slice(&armed_pan_gains[..*armed_pan_slots]);

                // --- 2c') 主总线推子：与声相表同一个形状（构造期标量，逐样本只乘）---
                // `dB → 线性` 已经在快照构造期算完（`EngineSnapshot::master_gain`）；
                // 这里只把这个 `f32` **读**进实时侧。主总线**不**进声相表（见上面
                // 的 `*id == master` 分支）：母带的声相由总线求和决定，与 `yeban-mcp`
                // 的 `masterPan` 登记同一口径。
                *armed_master_gain = current.master_gain();

                // --- 2c'') 每轨**插入器件**（通道条）：与声相表同一个形状 ---
                // [crate::insert]。计划（哪条轨挂通道条、参数是多少）在**控制线程**的
                // `EngineSnapshot::from_project` 里算好；这里只做**定长槽位内**的
                // `ChannelStrip::new` / `set_params`（标量 + 定长数组，**零分配**）。
                //
                // ⚠ `set_params` / `set_sample_rate` 会重算 EQ 系数、两个滤波器系数与
                // 压缩器的三个一阶低通系数（含 `exp`）⇒ 属 [ADR-0001 D32] 的
                // **超越函数类**，因此**只能**在这里（快照边界、每个修订一次）调用，
                // 绝不进逐样本路径 —— 与同一个分支里的 `params.pan_gains(...)`
                // （`cos`/`sin`）是同一条纪律。
                //
                // 默认口径：`current.inserts()` 在工程没有已识别的效果器设备时是**空表**
                // ⇒ 本循环不执行、`armed_insert_slots` 为 0 ⇒ 逐样本路径整段跳过。
                //
                // 槽位里装的是**通道条**而不是裸压缩器：`c792fdc` 的压缩器是通道条的
                // 动态级（`crate::insert` 模块文档 §2）。只写压缩器参数的设备投影成
                // "只开动态级"的通道条 ⇒ 那条路径的输出与 `c792fdc` **逐位相同**。
                *armed_insert_slots = 0;
                for (id, insert) in current.inserts() {
                    if *id == master || *armed_insert_slots >= MAX_TRACK_SLOTS {
                        continue;
                    }
                    let Some(params) = insert.strip() else {
                        continue;
                    };
                    let slot = *armed_insert_slots;
                    let sample_rate = current.sample_rate() as f32;
                    match &mut armed_strips[slot] {
                        // 同一条轨仍占同一个槽位 ⇒ 只换参数 + 校准采样率：
                        // **各级的状态保留**（重建通道条会把滤波器的积分器与压缩器的
                        // 增益弹道跳回初值，那是一次听得见的阶跃）。轨→槽位的分配由
                        // `BTreeMap` 的确定性迭代顺序决定 [MODEL-AST-003]。
                        (existing, Some(strip)) if *existing == *id => {
                            strip.set_params(params);
                            strip.set_sample_rate(sample_rate);
                        }
                        // 新占用该槽位 ⇒ 建一个新的器件（各级状态从初值起）。
                        entry => {
                            *entry = (*id, Some(ChannelStrip::new(params, sample_rate)));
                        }
                    }
                    *armed_insert_slots += 1;
                }

                // --- 2c''') 每轨插入器件的**混响级**：与通道条同一个槽位表形状 ---
                // [crate::insert]。延迟线在 `Self::new` 里就按**初始快照的采样率**
                // 分配好了（`Reverb::set_sample_rate` 是本器件唯一的分配点）⇒ 这里
                // **只允许** `set_params`（标量赋值 + 非有限值回落，零分配）。
                //
                // ⚠ **采样率与武装时不同 ⇒ 整段不武装**（模块文档 §8.5）：
                // `set_sample_rate` 会重建延迟线（`Vec` 重分配 + 释放），音频线程不允许
                // [MUST-GATE-001]。宁可少一个器件，也不在回调里分配、也不拿旧采样率的
                // 延迟线去处理新采样率的信号。拒绝次数进 `insert_reverb_rate_rejects`。
                //
                // 默认口径：`insert.reverb()` 在设备没有已识别混响参数时是 `None`
                // ⇒ 本循环那一项 `continue`、`armed_reverb_slots` 保持 0 ⇒ 逐样本路径
                // 整段跳过（不是"参数取成透明"）⇒ 那类轨的输出与接线前**逐位相同**。
                *armed_reverb_slots = 0;
                if current.sample_rate() == *armed_reverb_sample_rate {
                    for (id, insert) in current.inserts() {
                        if *id == master || *armed_reverb_slots >= MAX_TRACK_SLOTS {
                            continue;
                        }
                        let Some(params) = insert.reverb() else {
                            continue;
                        };
                        let slot = *armed_reverb_slots;
                        let sample_rate = current.sample_rate() as f32;
                        let (existing, was_armed) = armed_reverbs[slot];
                        if existing == *id && was_armed {
                            // 同一条轨仍占同一个槽位 ⇒ 只换参数：**器件状态保留**
                            // （重建会把延迟线清空 ⇒ 一次听得见的尾巴切断）。
                            reverb_pool[slot].set_params(params);
                        } else {
                            // 新占用该槽位：**不重建**（重建会 `Drop` 旧延迟线 = 回调内
                            // 释放），改为按**同一采样率**重新初始化 = 状态复位。
                            // 采样率与武装时逐位相同 ⇒ `setup` 走 `fill(0.0)` 分支，
                            // **零分配**（这正是守卫存在的理由）。
                            reverb_pool[slot].set_sample_rate(sample_rate);
                            reverb_pool[slot].set_params(params);
                            armed_reverbs[slot] = (*id, true);
                        }
                        *armed_reverb_slots += 1;
                    }
                } else {
                    *insert_reverb_rate_rejects = insert_reverb_rate_rejects.wrapping_add(1);
                }

                // --- 2c'''') 每轨插入器件的**卷积混响级**：与混响同一个槽位表形状 ---
                // [crate::insert] 模块文档 §9。器件的缓冲（IR 频谱 ＋ 预延迟线）在
                // `Self::new` 里就按**初始快照的采样率**与**引擎常量的 IR 长度**建好了
                // （`set_impulse_response` 是本器件唯一的分配入口）⇒ 这里**只允许**
                // **长度不变**的换 IR（原地复用缓冲，实测零分配）与 `set_params`
                // （标量 ＋ 一次 `libm::expf` 折算 IR 增益）。
                //
                // ⚠ **采样率与武装时不同 ⇒ 整段不武装**：IR 的帧数是 `采样率 ÷ 10`
                // ⇒ 换采样率就要改缓冲长度 = 分配，音频线程不允许 [MUST-GATE-001]。
                // 与混响的 §8.5 同款：宁可少一个器件，也不在回调里分配、也不拿旧长度的
                // IR 去处理新采样率的信号。次数进 `insert_convolution_rejects`。
                //
                // 默认口径：`insert.convolution()` 在设备没有已识别 `conv_` 参数时是
                // `None` ⇒ 本循环那一项 `continue`、`armed_conv_slots` 保持 0 ⇒ 逐样本
                // 路径整段跳过（不是"参数取成透明"）⇒ 那类轨的输出与接线前**逐位相同**。
                *armed_conv_slots = 0;
                if current.sample_rate() == *armed_conv_sample_rate {
                    for (id, insert) in current.inserts() {
                        if *id == master || *armed_conv_slots >= MAX_TRACK_SLOTS {
                            continue;
                        }
                        let Some(plan) = insert.convolution() else {
                            continue;
                        };
                        // IR 的长度在**构造期**由同一个 `convolution_ir_frames` 决定，
                        // 而采样率已在这一支的门口比对过 ⇒ 长度**必然**相等。把它写成
                        // 显式守卫仍然值得：长度相等正是"零分配"的全部前提，一旦将来
                        // 有人让长度变成旋钮，这条 `continue` 就是唯一的拦截点。
                        let slot = *armed_conv_slots;
                        if plan.ir_frames()
                            != crate::insert::convolution_ir_frames(*armed_conv_sample_rate)
                        {
                            *insert_convolution_rejects =
                                insert_convolution_rejects.wrapping_add(1);
                            continue;
                        }
                        let (existing, was_armed) = armed_convs[slot];
                        if existing == *id
                            && was_armed
                            && armed_conv_ir_hashes[slot] == plan.ir_hash()
                        {
                            // 同一条轨、同一条 IR ⇒ **只**换参数：频域延迟线与重叠相加尾
                            // （时间状态）保留 ⇒ 输出逐位不变（判据 V4 的第二半）。
                            conv_pool[slot].set_params(plan.params());
                        } else {
                            let accepted = conv_pool[slot].set_impulse_response(
                                plan.ir(),
                                plan.silence(),
                                plan.silence(),
                                plan.ir(),
                            );
                            if accepted != plan.ir_frames() {
                                // 器件拒绝（非有限样本 / 频谱溢出）⇒ 整台已回到未配置直通。
                                // 构造期合成的 IR 不可能走到这里（有限、峰值归一化），
                                // 这条是器件的唯一拦截点留下的安全网，并**计数**而不是静默。
                                *insert_convolution_rejects =
                                    insert_convolution_rejects.wrapping_add(1);
                                continue;
                            }
                            conv_pool[slot].set_params(plan.params());
                            armed_convs[slot] = (*id, true);
                            armed_conv_ir_hashes[slot] = plan.ir_hash();
                        }
                        *armed_conv_slots += 1;
                    }
                } else {
                    // 这一份快照里**可能**有卷积混响设备（也可能没有）⇒ 只有真的有
                    // 才计数：读数要回答"有几个设备因此没工作"，不是"换过几次采样率"。
                    let unarmed = current
                        .inserts()
                        .iter()
                        .filter(|(id, insert)| **id != master && insert.convolution().is_some())
                        .count() as u64;
                    *insert_convolution_rejects = insert_convolution_rejects.wrapping_add(unarmed);
                }

                // --- 2d) 节拍器：波形与拍栅格都在**构造期**算好（`sin` 属超越函数类），
                // 这里只把两个整数（每拍 tick / 每小节拍数）与一个开关读进实时侧
                // （[`crate::metronome`]）。**关掉时**把武装标志置假并丢掉可能正在响的
                // 尾巴 ⇒ 输出与"从未开过节拍器"逐字节相同（见 `tests/metronome_render.rs`）。
                // 换快照**不重置** `next_beat_tick`：拍栅格以 tick 为单位、与 BPM 无关，
                // 因此改速度不会让拍点跳一格；陈旧对齐由 `render_quantum` 就地拉回。
                // ⚠ **拍格本身变了**（拍号分母换档，例如 4/4 的 960 tick ⟶ 3/2 的 1920 tick）
                // 时旧的下拍 tick 不再落在新格上，而且每拍加 `ticks_per_beat` 会**一直**
                // 偏半格 —— 因此这里显式重对齐一次（整数比较 + 一次 `div_ceil`，零分配）。
                match current.metronome() {
                    Some(plan) => {
                        let grid_changed = *armed_metronome_ticks_per_beat != plan.ticks_per_beat();
                        *armed_metronome_enabled = true;
                        *armed_metronome_ticks_per_beat = plan.ticks_per_beat();
                        *armed_metronome_beats_per_bar = plan.beats_per_bar();
                        if grid_changed {
                            metronome.resync(transport.position_ticks(), plan.ticks_per_beat());
                        }
                    }
                    None => {
                        *armed_metronome_enabled = false;
                        *armed_metronome_ticks_per_beat = 0;
                        *armed_metronome_beats_per_bar = 0;
                        metronome.silence();
                    }
                }
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
            // 本量子插入链动态级的**当前**衰减与**检波器**电平（两者都跨轨取最大；量规）。
            //
            // 三个局部量只在**本量子**内有意义：`insert_current_db` / `insert_detector_db`
            // 分别收集"本量子每条轨的通道条报出的瞬时衰减 / 检波器电平"的最大值，
            // `insert_current_seen` 记下"本量子真的有一台武装通道条处理过帧"。循环之后
            // 只在 `seen` 为真时**覆写**两个读数 —— 与 `limiter_current_reduction` 的
            // "没有快照的量子不更新"同口径（见 [`EngineStats::insert_current_reduction_db`]
            // 与 [`EngineStats::insert_detector_level_db`] 的口径第 3／4 条）。
            //
            // ⚠ 两条读数**共用**同一个 `seen` 标志：它们来自同一台器件、同一个瞬间
            // （见下面对 `strip` 的那一段读），因此"本量子有没有器件工作过"是**同一个**
            // 事实，分成两个标志只会给它们两个不同的更新口径。
            //
            // ⚠ 这三个局部量是**读数的搬运**，不参与任何样本计算 ⇒ 逐样本路径**一个字
            // 都没动** ⇒ 渲染输出逐位不变（判据见 `tests/channel_strip_insert.rs` 的 C15/C16）。
            let mut insert_current_db = 0.0f32;
            // 初值取 `-∞`：任何一台真实器件的检波器电平都 `≥ MIN_LEVEL_DB`（−120 dBFS）
            // ⇒ 它一定被第一次赋值覆盖（`seen` 为真时至少有一台器件报过数）。
            let mut insert_detector_db = f32::NEG_INFINITY;
            let mut insert_current_seen = false;
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
                // --- 3a⁰) 参数目标表的**逐轨增益乘子**：逐样本 ---
                // [crate::param]。位置：**声源之后、插入链之前** —— 与"在构造期把该轨
                // 音量设成另一个值"同解（静态音量也是在插入链之前烧进声部增益的，
                // 见 `crate::snapshot::project_schedules` 的 `track_gain`）。
                //
                // ⚠ 刻意**不**放在插入链之后：那样压缩器看到的是**未自动化**的电平，
                // 是另一种（也不好听的）行为。理由写在 `crate::param` 模块文档 §2.2。
                //
                // **默认口径**：本表里没有这条轨的槽位、或它已吸附到 `1.0`（恒等）
                // ⇒ `apply` **一个样本都不碰**并返回 0 ⇒ 没有 `SetParam` 的工程与
                // 接线前**逐位相同**（模块文档 §5）。
                //
                // 逐样本只有一次乘加（平滑器）与一次乘；**零分配、零锁、零 I/O、
                // 零日志** [MUST-GATE-001, ARCH-DSP-001]。
                params.apply(track, &mut track_scratch[..frames]);
                // --- 3a') 插入链：本轨的通道条（若有）**逐样本**处理 ---
                // [crate::insert]。位置：**轨道自己的渲染之后、逐轨电平与 PDC 之前**
                // ⇒ 电平读数（`EngineStats::meter_frames`）与汇入母线的信号都是
                // "插入之后"的结果。
                //
                // **默认口径**：本轨没有插入器件（`armed_strips` 里查不到）⇒
                // 这个分支**整段跳过**（不是"参数取成透明"）⇒ 这类轨的输出与接线前
                // **逐位相同**（证据见 `tests/compressor_insert.rs` 的 C0）。
                //
                // 停住时喂的是静音：各级状态是**时间状态**（滤波器的积分器、压缩器的
                // 增益弹道），喂静音让它们按各自的时间常数回到初值 —— 与 PDC 延迟线
                // "停住也喂静音"是同一条纪律。
                //
                // 逐样本路径上只有器件的乘加与一阶递归（器件把 `exp` 全部留在
                // `set_params` 里，EQ 的中间缓冲是**栈数组**）；这里**零分配、零锁、
                // 零 I/O** [MUST-GATE-001]。
                if let Some(index) = armed_strips[..*armed_insert_slots]
                    .iter()
                    .position(|(id, _)| id == &track)
                    && let (_, Some(strip)) = &mut armed_strips[index]
                {
                    let before_reductions = strip.gain_reduction_count();
                    let before_frames = strip.processed_frames();
                    strip.process_mono(&mut track_scratch[..frames]);
                    let reduced = strip
                        .gain_reduction_count()
                        .saturating_sub(before_reductions);
                    *insert_gain_reductions = insert_gain_reductions.wrapping_add(reduced);
                    let processed = strip.processed_frames().saturating_sub(before_frames);
                    *insert_strip_frames = insert_strip_frames.wrapping_add(processed);
                    let reduction_db = strip.max_gain_reduction_db();
                    if reduction_db > *insert_max_reduction_db {
                        *insert_max_reduction_db = reduction_db;
                    }
                    // **当前**衰减（量规）：与上面两条计数**同一台器件、同一个瞬间**读出。
                    // 它只读字段（`−compressor.gain_db()`），不碰样本、不分配、不加锁。
                    let current_db = strip.current_gain_reduction_db();
                    if current_db > insert_current_db {
                        insert_current_db = current_db;
                    }
                    // **检波器**电平（量规，单位 dBFS）：同一个器件、同一个瞬间的**输入**侧。
                    // 它同样只读一个字段（`compressor.level_db`，一个 `const fn`）
                    // ⇒ 不碰样本、不分配、不加锁 ⇒ 渲染输出逐位不变。
                    let detector_db = strip.detector_level_db();
                    if detector_db > insert_detector_db {
                        insert_detector_db = detector_db;
                    }
                    insert_current_seen = true;
                }
                // --- 3a''') 插入链的**混响级**：逐样本（器件的接口是立体声块）---
                // [crate::insert] 模块文档 §8.3。位置与通道条**同一条链上的后一级**：
                // 轨道自己的渲染之后、逐轨电平与 PDC 之前。
                //
                // 单声道口径：把 `track_scratch` 同时喂给左右两路，取两路输出的中值
                // `(out_l + out_r) · 0.5`。那是恒等式而不是近似（§8.3 给了代数证明）：
                // 湿路径的输入与"单声道输入"同解，而宽度项在中值里恰好抵消。
                //
                // **默认口径**：本轨没有混响（`armed_reverbs` 里查不到武装项）⇒
                // 整段跳过；`Reverb::is_active()`（`mix ≤ 1e-4`）为假时同样跳过 ⇒
                // 这两种情形下器件的状态**不推进**（与器件自己的早返回守卫同口径）。
                //
                // 逐样本只有环形缓冲读写与乘加；**零分配、零锁、零 I/O、零日志**
                // [MUST-GATE-001]。
                if let Some(index) = armed_reverbs[..*armed_reverb_slots]
                    .iter()
                    .position(|(id, armed)| *armed && id == &track)
                {
                    let reverb = &mut reverb_pool[index];
                    if reverb.is_active() {
                        let frames = frames.min(DEFAULT_BLOCK_FRAMES);
                        reverb_scratch[0][..frames].copy_from_slice(&track_scratch[..frames]);
                        reverb_scratch[1][..frames].copy_from_slice(&track_scratch[..frames]);
                        let (left, right) = reverb_scratch.split_at_mut(1);
                        reverb.process(&mut left[0][..frames], &mut right[0][..frames]);
                        for index in 0..frames {
                            track_scratch[index] =
                                (reverb_scratch[0][index] + reverb_scratch[1][index]) * 0.5;
                        }
                        *insert_reverb_frames = insert_reverb_frames.wrapping_add(frames as u64);
                    }
                }
                // --- 3a'''') 插入链的**卷积混响级**：逐样本（器件的接口是交错立体声块）---
                // [crate::insert] 模块文档 §9.4。位置与混响**同一条链上的后一级**：
                // 轨道自己的渲染之后、逐轨电平与 PDC 之前。
                //
                // 单声道口径（与混响同款）：把 `track_scratch` **同时**写进左右两路，
                // 取两路输出的中值 `(out_l + out_r) · 0.5`。那是恒等式而不是近似：
                // 四条通路线性，中值 = `conv((h_LL + h_LR + h_RL + h_RR) / 2, m)`，
                // 而投影交进去的是 `h_LL = h_RR = ir`、`h_LR = h_RL = 0` ⇒ 恰好是
                // `conv(ir, m)`（§9.4 给了这段代数）。
                //
                // ⚠ IR 本身**不在**这条路径上：它只在快照边界换（且长度不变 ⇒ 零分配）。
                //
                // **默认口径**：本轨没有卷积混响（`armed_convs` 里查不到武装项）⇒
                // 整段跳过；`ConvolutionReverb::is_active()`（`wet ≤ 1e-4` 或未配置）为假时
                // 同样跳过 ⇒ 这两种情形下器件的状态**不推进**。
                //
                // 逐样本只有频域分块乘加与 256 点变换；**零分配、零锁、零 I/O、零日志**
                // [MUST-GATE-001]。⚠ 它是 [ADR-0001 D32] 的**超越函数类**（旋转因子表由
                // 宿主 libm 的 `f32::cos`/`f32::sin` 在构造期算出）⇒ 本票不声称跨架构逐位相同。
                if let Some(index) = armed_convs[..*armed_conv_slots]
                    .iter()
                    .position(|(id, armed)| *armed && id == &track)
                {
                    let conv = &mut conv_pool[index];
                    if conv.is_active() {
                        let frames = frames.min(DEFAULT_BLOCK_FRAMES);
                        for frame in 0..frames {
                            let mono = track_scratch[frame];
                            conv_scratch[2 * frame] = mono;
                            conv_scratch[2 * frame + 1] = mono;
                        }
                        conv.process(&mut conv_scratch[..2 * frames]);
                        for frame in 0..frames {
                            track_scratch[frame] =
                                (conv_scratch[2 * frame] + conv_scratch[2 * frame + 1]) * 0.5;
                        }
                        *insert_convolution_frames =
                            insert_convolution_frames.wrapping_add(frames as u64);
                    }
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
                // 这里只做环形读写：**零分配、逐样本无分支**（`delay == 0` 时输出
                // 逐位等于输入，但延迟线**仍然记录** —— 见 `graph::DelayLine` 的
                // "环的不变量"）。
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
                //
                // **声相自动化**（裁决 P4=(b)）只多一个分支：该槽位收到过被接受的
                // 声相目标 ⇒ 走**平滑路径**（逐样本把两个增益收敛到目标，零分配）；
                // 否则走**原来那条**常量路径 ⇒ 没有声相自动化的工程**逐位不变**。
                let pan_index = pan_gains[..*armed_pan_slots]
                    .iter()
                    .position(|(id, _, _)| *id == track);
                if let Some(index) = pan_index.filter(|index| armed_pan_armed[*index]) {
                    sum_into_bus_smoothed(
                        block,
                        &track_scratch[..frames],
                        &mut armed_pan_l[index],
                        &mut armed_pan_r[index],
                    );
                    *pan_automation_frames = pan_automation_frames.wrapping_add(frames as u64);
                } else {
                    let (gain_l, gain_r) = pan_index.map_or(
                        (
                            core::f32::consts::FRAC_1_SQRT_2,
                            core::f32::consts::FRAC_1_SQRT_2,
                        ),
                        |index| (pan_gains[index].1, pan_gains[index].2),
                    );
                    sum_into_bus(block, &track_scratch[..frames], gain_l, gain_r);
                }
            }

            // --- 插入链动态级的**当前**衰减与**检波器**电平读数（3a' 的收尾）：
            //     逐轨循环之后覆写**一次** ---
            //
            // 读数的搬运（不是计算）：本量子至少有一台武装通道条处理过帧时，把
            // `insert_current_db` 与 `insert_detector_db`（两者都是跨轨最大）覆写进统计。
            // 放在循环**之后**而不是循环**之内**有两个理由：
            //
            // 1. 一个量子只有**一个**读数（与 `insert_max_reduction_db` 的跨轨口径相同）。
            //    循环内每轨各写一次会让读数的终值取决于"最后一条轨是谁"（`BTreeMap`
            //    的键序），那是把实现细节当成语义；
            // 2. 它们是**量规**：一台都不工作时**不覆写**（保持上一次的值），
            //    见 [`EngineStats::insert_current_reduction_db`] 的口径第 3 条与
            //    [`EngineStats::insert_detector_level_db`] 的口径第 4 条。
            //
            // 零分配、零锁、零 I/O、零日志：一次 `f32` 字段写与一次 `Option<f32>` 字段写。
            if insert_current_seen {
                *insert_current_reduction_db = insert_current_db;
                // `insert_detector_db` 此时必定已被至少一台器件赋值过（`seen` 的定义）
                // ⇒ `Some(…)` 里不是 `NEG_INFINITY`。
                *insert_detector_level_db = Some(insert_detector_db);
            }

            // --- 3a') 节拍器咔哒声：**逐轨汇流之后、主总线推子与母线限制器之前** ---
            //
            // ## 位置（为什么在这里）
            //
            // 1. **在限制器之前**：限制器是母线输出的最后一道约束（[`crate::mixer`] 模块文档
            //    §4 的上界证明）。咔哒声作为**母线信号**混进来 ⇒ "限制后峰值 ≤ 天花板"这条
            //    约束对它同样成立。放到限制器**之后**会让咔哒声直接推高输出（可能越过天花板），
            //    那等于给总线开了一个不受约束的入口。
            // 2. **在主总线推子之前**：推子是"整条母线输出"的音量（[`scale_bus`] 的位置说明）。
            //    咔哒声也是母线输出的一部分 ⇒ 推子对它同样有效：拉低推子时整条总线（含咔哒声）
            //    一起变小。取舍是**明说的**：主总线静音时听不到节拍器。另一半方案（放在推子
            //    之后）能让节拍器不受推子影响，但它会让"母线音量"对两个信号有两种含义 ——
            //    本仓库宁可口径单一。
            // 3. 混进来的是**本快照的波形**（`current.metronome()`）：`None` ⇒ 开关为假
            //    ⇒ **整段跳过**（零分支代价、输出逐位不变）。
            //
            // ## 实时约束
            //
            // 逐样本只有"整数比对 + 一次乘 + 两次加"（[`crate::metronome::render_quantum`]）；
            // 唯一的整数除法在每拍的帧位置反算里（[`Transport::frames_until_tick`]）。
            // **没有**分配/释放/锁/I-O/日志/超越函数 [MUST-GATE-001, ADR-0001 D32]。
            if *armed_metronome_enabled && let Some(plan) = current.metronome() {
                render_metronome_quantum(metronome, plan, transport, block, frames);
            }

            // --- 3a'') 主总线推子：**逐轨汇流之后、母线限制器之前** ---
            // 增益在**构造期**算好（[`EngineSnapshot::master_gain`]；`exp2` 属超越函数类），
            // 实时侧只有一次乘（[`scale_bus`]）—— 零分配、零锁、零 I/O、零除法、零超越函数
            // [MUST-GATE-001]。
            // `== 1.0`（默认 0 dB）时**整段跳过** ⇒ 单位增益下的输出与接线前**逐位**相同。
            // 这个数在整份快照的生命周期内不变（快照不可变，见 2c'）。
            let master_gain = *armed_master_gain;
            if master_gain != 1.0 {
                scale_bus(block, master_gain);
            }

            // --- 3a''') 主总线增益的**自动化乘子**：与推子**同一个位置** ---
            // [crate::param] 的第二个槽位（[`crate::param::MASTER_GAIN_SLOT`]，模块文档 §2.3）。
            // 位置**刻意**与 3a'' 相同：本乘子作用在"整条母线输出"上（含节拍器），
            // 与推子同口径。顺序也是契约的一部分（浮点乘法不满足结合律）：
            // **先**静态推子 `scale_bus`、**后**本乘子 —— 与逐轨槽位"先构造期静态音量、
            // 后运行期乘子"一致。
            //
            // **默认口径**：主总线槽位从未收到过被接受的 `SetParam`、或它已吸附到恒等
            // `1.0` ⇒ `apply_master` **一个样本都不碰**并返回 0 ⇒ 没有该事件的工程与
            // 接线前**逐位相同**（[crate::param] 模块文档 §5）。
            //
            // 逐样本（每帧）：一次单极点低通（`process`）＋ 两条声道各一次乘。
            // 立体声联动是**强制**的：左右用同一个增益值（模块文档 §2.3）。
            // **零分配、零锁、零 I/O、零日志** [MUST-GATE-001, ARCH-DSP-001]。
            let (master_left, master_right) = block.stereo_mut();
            params.apply_master(master_left, master_right);

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
            // 前瞻式峰值限制、立体声联动、逐样本确定（实现与上界证明住在
            // `yeban_dsp::limiter`；本 crate 只 re-export，见 `crate::mixer` 模块文档 §2）。
            //
            // ⚠ 主总线推子（步骤 3a''）在**它之前**：见 [`scale_bus`] 的位置说明。
            let before = limiter.reduction_count();
            {
                // 上移之后器件入口是**切片**（`Limiter::process_stereo`，与
                // `yeban_dsp::compressor`/`channel_strip` 同风格）：本量子的帧数在这里
                // 切出来。钳制口径与上移前的 `apply(&mut AudioBlock, frames)` **相同**
                // （`frames.min(块容量)`）⇒ 行为逐位不变。
                let limiter_frames = frames.min(block.capacity());
                let (left, right) = block.stereo_mut();
                // --- 3b⁰) 母线**有限值守卫**（本票）：见 [`saturate_bus_to_finite`] ---
                //
                // ⚠ 位置是**限制器的输入侧**，不能换：限制器是**递归状态**器件，
                // 它自己的口径对 `NaN` 有定义（`nan_to_zero`）但对 `±∞` **刻意保留**
                // （`crates/yeban-dsp/src/limiter.rs` 的 `nan_to_zero` 文档逐字写着
                // "`±∞` 原样保留"）⇒ 一个 `∞` 会让窗口峰值 `W = ∞`、目标增益
                // `T / W = 0`，随后 `∞ · 0 = NaN` **静默**污染整条母线。
                // 因此"交给限制器的必须是有限值"是**引擎侧**的契约。
                //
                // 有限样本**逐位不变** ⇒ 既有（有限）渲染输出逐位不变；只有
                // 非有限样本被收进有限域。零分配、零锁、零 I/O、零日志
                // [MUST-GATE-001]：只是一次 `is_finite` 判定的分支与一次赋值。
                saturate_bus_to_finite(&mut left[..limiter_frames], &mut right[..limiter_frames]);
                limiter.process_stereo(&mut left[..limiter_frames], &mut right[..limiter_frames]);
            }
            let reduced = limiter.reduction_count().saturating_sub(before);
            if reduced > 0 {
                *limiter_gain_reductions = limiter_gain_reductions.wrapping_add(reduced);
            }
            let reduction = 1.0 - limiter.gain();
            if reduction > *limiter_max_reduction {
                *limiter_max_reduction = reduction;
            }
            // --- 3b') **当前**压限量：按量子覆写的**量规** ---
            // 与上面的 `limiter_max_reduction` 用的是**同一个已读出的值**
            // （`Limiter::gain()` 在上一行已经为了算最大值被读过一次）⇒ 本字段
            // **不增加**任何 DSP 调用、不碰任何样本、不分配 ⇒ 渲染输出逐位不变。
            // 它是 `line/engine-mix` 台账登记的那条"按量子发布当前 GR"读数
            // （见 [`EngineStats::limiter_current_reduction`] 的文档）。
            *limiter_current_reduction = reduction;

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

/// 与 [`sum_into_bus`] **同形**，但两个增益**逐帧**从各自的平滑器里取
/// （声相自动化，裁决 P4=(b)）。
///
/// 为什么需要它：声相从"当前值"平滑走到自动化目标必须是**逐样本**的，否则
/// Σ 就是一次阶跃（可闻的 zipper noise）。这里逐帧只多两次一阶递归（一次乘加
/// ⇒ IEEE 精确类），**零分配、零锁、零 I/O、零日志** [MUST-GATE-001]。
///
/// ⚠ 没有声相自动化的工程**不走这条**（调用点按 `armed_pan_armed[slot]` 分流）
/// ⇒ 它们的渲染输出与接线前**逐位相同**。
fn sum_into_bus_smoothed(
    block: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    mono: &[f32],
    gain_l: &mut ParamSmoother,
    gain_r: &mut ParamSmoother,
) {
    // 两个切片都由 `stereo_mut()` 按有效帧数给出, `zip` 天然按较短者截断。
    let (left, right) = block.stereo_mut();
    for ((l, r), m) in left.iter_mut().zip(right.iter_mut()).zip(mono) {
        *l += *m * gain_l.process();
        *r += *m * gain_r.process();
    }
}

/// 主总线推子：把**已汇流**的立体声块按构造期算好的标量缩放（左右各一次乘）。
///
/// ## 位置（为什么在**限制器之前**）
///
/// 调用点是 [`EngineRuntime::render_block`] 的步骤 3a''：**逐轨汇流之后、
/// [`crate::mixer::BusLimiter::process_stereo`] 之前**。这个顺序有两条理由：
///
/// 1. 限制器是母线输出的**最后一道**约束 —— 它自己的契约是"限制后峰值 ≤ 天花板"
///    （[`crate::mixer`] 模块文档 §2 指向的上界证明）。推子放在它的**输入侧**，
///    这条约束对**任何**主总线音量都成立：推子推高时限制器压得更狠，而**不是**
///    让输出越过天花板；
/// 2. `yeban-mcp` 的离线母带把 Master 轨增益放在**整条渲染之后**
///    （`crates/yeban-mcp/src/domain/render.rs` 第 9 步），那条链路里**没有**限制器
///    ⇒ 两者不冲突。本引擎里限制器是**总线器件**，推子接在它的输入侧。
///
/// ⚠ 顺序**没有**规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 与
/// `docs/adr/**` 里查不到"主总线推子 vs 母线限制器"的先后），本实现按上面两条
/// 理由选择，并登记为待裁决（见报告与 needs）。
///
/// ## 实时约束
///
/// 逐样本只有一次乘（[ADR-0001 D32] 的 IEEE 精确类）：**没有**除法、**没有**
/// 超越函数、**没有**分配/锁/I-O。增益非 `1.0` 时才被调用（见 3a'' 的跳过分支）。
fn scale_bus(block: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>, gain: f32) {
    // 与 `sum_into_bus` 同源：`stereo_mut()` 只给出**有效帧**。
    let (left, right) = block.stereo_mut();
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        *l *= gain;
        *r *= gain;
    }
}

/// 母线**有限值守卫**：把非有限样本收进有限域，**有限样本逐位不变**。
///
/// 调用点是 [`EngineRuntime::render_block`] 的步骤 3b⁰：**母线限制器的输入侧**。
///
/// ## 1. 它修的是一个实测出来的缺陷（类别 4「参数极值」／类别 1「非有限输出」）
///
/// **量什么／怎么量／单位**：把 `TrackV3::volume_db` 设成各种值，用
/// `EngineSnapshot::from_project` + `EngineRuntime::process_quantum`（1 条轨、24 个交叠
/// 音符、400 个量子 × 128 帧 × 2 声道 = **102 400** 个输出样本）渲染，数输出里
/// **非有限样本的个数**（单位：个）。实测（`volume_db` → NaN 个数）：
///
/// | `volume_db` | `synth::track_gain` | 输出 NaN 个数 |
/// | :--- | :--- | ---: |
/// | `0.0` | `1.0` | `0` |
/// | `700.0` | `1.0000022e35` | `0` |
/// | `760.0` | `1.0000008e38` | `23554` |
/// | `770.0` | `3.1622822e38`（**有限**） | `99448` |
/// | `771.0` | `0.0`（`exp2` 对溢出结果归零） | `0` |
///
/// 同一张表里换**主总线**那一轨（`master.volume_db`，经 [`scale_bus`]）在 `770.0` 上
/// 给出 `72742` 个 NaN。两条路径的输入都是**模型接受**的工程：`TrackV3::validate`
/// 只要求 `volume_db` **有限**（`crates/yeban-model/src/project.rs`，现位于第 1204 行起），
/// 没有上界 ⇒ `770.0` 是**合法**工程。`SetParam` 事件侧同款：`ParamTable::accept`
/// 接受任何**有限且非负**的值，而 `value = f32::MAX` 在 4 / 24 个交叠音符下分别给出
/// `84888` / `100112` 个 NaN（窗口 600 量子 ⇒ 153 600 个样本）。
///
/// **机理**（三段，每一段都实测过）：
///
/// 1. 引擎把**有限**增益乘到信号上，乘积**上溢出**成 `±∞`
///    （`3.1622822e38 × 24 个声部的合成结果 > f32::MAX`）；
/// 2. `∞` 流进母线限制器：它是**递归状态**器件，窗口峰值 `W = ∞` ⇒ 目标增益
///    `T / W = 0`（非有限输入本身由器件的 `nan_to_zero` 挡住，但那个函数**刻意**
///    保留 `±∞`，见 `crates/yeban-dsp/src/limiter.rs` 的 `nan_to_zero` 文档）；
/// 3. 输出 `sample(∞) × gain(0) = NaN`，`soft_knee(NaN) = NaN` ⇒ **NaN 写进声卡缓冲**。
///
/// ⚠ 本 crate 改不了第 2 段（限制器住在 `yeban-dsp`，且那条口径是有意为之），
/// 因此把契约补在**引擎这一侧**：交给限制器的输入必须是有限值。
///
/// ## 2. 饱和到什么值，为什么
///
/// `NaN → 0.0`（与限制器自己的 `nan_to_zero` **同口径** ⇒ 限制器环形缓冲里存进去
/// 的仍是同一个 `0.0`，一位不差）；`±∞ → ±`[`MAX_LINEAR_MAGNITUDE`]
/// （`= 16.0` = 4 × 满量程 = **+24.08 dBFS**）。
///
/// 为什么是这个常数而不是 `f32::MAX`：`f32::MAX` 会让限制器的目标增益
/// `0.9 / 3.4e38 = 2.6e-39` 落进**次正规数**，而引擎的实时路径开着 FTZ/DAZ
/// （[ARCH-RT-003]）⇒ 那个目标会被冲刷成 `0.0`，输出变成**整段静音**（比 NaN 好，
/// 但比"响亮的一声被限制"更不像"饱和"）。`MAX_LINEAR_MAGNITUDE` 是**引擎已有的**
/// 线性幅度上限（[`crate::level`] 的 `sanitize_sample` 用的就是它，口径"超出即视为
/// 已削顶"），取它不需要发明任何新常数，也不需要第二份域表。
///
/// ## 3. 为什么它**不**改变既有渲染输出
///
/// 判定只有一条：`!sample.is_finite()`。因此
///
/// - **有限**样本（含 `±0.0`、次正规数、`f32::MAX`）走的是"原样写回"⇒ 逐位相同；
/// - 现状下会产生 `NaN` 的那些样本，本来就已经是坏的输出。
///
/// 也就是说：**每一个有限输入下的输出都与加本守卫之前逐位相同**；改变只发生在
/// 今天输出 `NaN` 的那些情形。母线输出的上界（限制器契约"限制后峰值 ≤ 天花板"）
/// 因此第一次对**任意**输入成立 —— 今天它在输 `∞` 时是失效的。
///
/// ## 4. 实时分类（[ADR-0001 D32]）
///
/// 逐样本只有一次 `is_finite` 判定、一次比较与（罕见路径上的）一次赋值：**分配 0、
/// 释放 0、锁 0、阻塞 I/O 0、日志 0** [MUST-GATE-001]。判据：
/// `tests/synth_rt_zero_alloc.rs` 的场景 J21（四元组）与 `rt::tests` 的三条单元判据。
///
/// ## 5. 登记（本守卫**没有**做的事）
///
/// 1. **不计数**：饱和了多少个样本不可观测。这与 [`crate::level`] 的 `sanitize_sample`
///    钳位是同一个已登记的缺口（`docs/ledger/engine-meters-notes.md` 现位于第 308 行），
///    本票按同一口径登记而不新增 `EngineStats` 字段（那会改动跨线程镜像的契约）；
/// 2. **不修上游**：`SetParam` 的极大有限值与 `volume_db` 的极大有限值仍然被**接受**
///    —— 本守卫作用于**信号**，不发明参数值域（那需要一条裁决：见交付报告的 needs）；
/// 3. **不动限制器本身**：`yeban-dsp` 的 `nan_to_zero` 保留 `±∞` 那条口径由它的
///    属主决定，本 crate 只保证"不把 `∞` 交给它"。
fn saturate_bus_to_finite(left: &mut [f32], right: &mut [f32]) {
    for sample in left.iter_mut().chain(right.iter_mut()) {
        if !sample.is_finite() {
            *sample = if sample.is_nan() {
                0.0
            } else {
                MAX_LINEAR_MAGNITUDE.copysign(*sample)
            };
        }
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

    /// 与 [`snapshot_with_master_track`] 同形，但**每条轨挂一台被识别的通道条**
    /// （`("eq_low_gain", 6.0)` ⇒ `insert.strip()` 是 `Some`）—— 只用于
    /// **插入槽容量边界**的判据（`armed_strips` 是 `MAX_TRACK_SLOTS` 长的定长数组）。
    fn snapshot_with_master_track_and_strips(revision: u64, track_count: usize) -> EngineSnapshot {
        let master = EntityId::new();
        let mut nodes = vec![master];
        let mut routing = RoutingGraph {
            nodes: Vec::new(),
            ..RoutingGraph::default()
        };
        let mut tracks: BTreeMap<EntityId, TrackParams> = BTreeMap::new();
        let mut inserts: BTreeMap<EntityId, crate::insert::InsertParams> = BTreeMap::new();
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
                devices: vec![yeban_model::DeviceDefinition {
                    id: EntityId::new(),
                    name: "Strip".to_owned(),
                    kind: yeban_model::DeviceKind::InternalEffect,
                    bypassed: false,
                    params: vec![yeban_model::ParameterValue {
                        name: "eq_low_gain".to_owned(),
                        value: 6.0,
                        unit: None,
                    }],
                    latency_samples: 0,
                }],
                ..TrackV3::default()
            };
            tracks.insert(track, TrackParams::from_track(&model, 0));
            let insert = crate::insert::InsertParams::from_devices(&model.devices, 48_000);
            if !insert.is_empty() {
                inserts.insert(track, insert);
            }
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
        .with_inserts(inserts)
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

    /// 指定**电平 SPSC 容量**的装配：容量 1 是"制造 UI 落后"的标准夹具
    /// （一个量子要发"非母线轨 + 1"帧 ⇒ 控制面不抽干就必然丢帧）。
    fn rig_with_meter_capacity(meter_capacity: usize) -> Rig {
        let slot = SnapshotSlot::new(simple_snapshot(1));
        let (retire, queue) = retire_channel(16);
        let (sender, receiver) = event_channel(64);
        let (publisher, collector) = meter_channel(meter_capacity);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Rig {
            slot,
            queue,
            sender,
            collector,
            runtime,
        }
    }

    /// `[ARCH-UI-002]` **电平丢帧必须可读**，而且读数必须是"本应发布的帧数"的**精确分解**。
    ///
    /// 量什么：`meter_frames`（真的写进队列的帧数）与 `meter_dropped_frames`
    /// （因 SPSC 环满而丢掉的帧数），单位都是**帧**。
    ///
    /// 为什么要两条臂：一条**大容量**的臂给出"本应发布的帧数"（`= 量子数 × 每量子帧数`），
    /// 它是独立测出来的基线；另一条**容量 1** 的臂必须满足
    /// `meter_frames + meter_dropped_frames == 基线` 这个**等号**。容量 1 的臂同时是
    /// 覆盖度见证（`meter_dropped_frames > 0`）—— 否则"读数恒为 0"也能让等号成立（假绿）。
    ///
    /// 注入：把 `stats()` 的 `meter_dropped_frames` 写死成 `0`、或换成
    /// `meter_capacity_drops`，本判据立刻变红（本票实测记录见报告）。
    #[test]
    fn meter_drops_are_readable_and_decompose_the_offered_frames_exactly() {
        const QUANTA: usize = 32;
        let mut output = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];

        // 臂 A：容量足够 + 不抽干 ⇒ **零丢帧**（"读数不会假警报"的对照臂）。
        let mut generous = rig_with_meter_capacity(4096);
        for _ in 0..QUANTA {
            generous.runtime.process_quantum(&mut output, 2);
        }
        let baseline = generous.runtime.stats();
        assert_eq!(
            baseline.meter_dropped_frames, 0,
            "容量足够时不许丢帧（假警报会让这条读数失去意义）"
        );
        assert!(!baseline.is_meter_lagging());
        assert!(
            baseline.meter_frames > 0,
            "覆盖度：窗口里必须真的发布了电平帧"
        );
        let offered = baseline.meter_frames;

        // 臂 B：容量 1 + 不抽干 ⇒ 环持续满着，每量子都丢帧。
        let mut starved = rig_with_meter_capacity(1);
        for _ in 0..QUANTA {
            starved.runtime.process_quantum(&mut output, 2);
        }
        let stats = starved.runtime.stats();
        assert!(
            stats.meter_dropped_frames > 0,
            "容量 1 + 不抽干必须丢帧，实际 {}",
            stats.meter_dropped_frames
        );
        assert_eq!(
            stats.meter_frames + stats.meter_dropped_frames,
            offered,
            "本应发布的帧数 = 写进队列的帧数 + 丢掉的帧数（两者单位都是帧）"
        );
        assert!(stats.is_meter_lagging(), "丢过帧 ⇒ 粘滞判定为真");
        // **两个失败面不是同一件事**：容量 1 的丢帧来自 SPSC 环满（发布批次放得下），
        // 而 `meter_capacity_drops` 数的是"帧在发布批次里放不下 / MeterBank 淘汰槽"。
        // 注入：把本字段接到 `meter_capacity_drops` ⇒ 这一条与上面那条等号同时红。
        assert_eq!(
            stats.meter_capacity_drops, 0,
            "SPSC 环满**不是**批次容量不足（两个读数单位不同、来源不同）"
        );
        assert_eq!(
            stats.meter_bulk_publishes, baseline.meter_bulk_publishes,
            "两条臂的量子数相同 ⇒ 批量发布次数相同（丢帧不改变结构计数）"
        );

        // 增量判定：与更早的基线比。
        let previous = EngineStats {
            meter_dropped_frames: stats.meter_dropped_frames - 1,
            ..EngineStats::default()
        };
        assert_eq!(stats.meter_dropped_since_last_read(&previous), 1);
        assert!(stats.is_meter_lagging_since(&previous));
        assert_eq!(stats.meter_dropped_since_last_read(&stats), 0);
        assert!(!stats.is_meter_lagging_since(&stats));
        // 读到更旧的基线 ⇒ 0（饱和减法，不回绕）。
        let newer = EngineStats {
            meter_dropped_frames: stats.meter_dropped_frames + 7,
            ..EngineStats::default()
        };
        assert_eq!(stats.meter_dropped_since_last_read(&newer), 0);

        // 跨线程只读镜像：静止点上与权威读数**逐字段相等**（新字段在镜像里也有位置）。
        let mirror = starved.runtime.stats_mirror();
        assert_eq!(
            mirror.read(),
            stats,
            "镜像必须带上 meter_dropped_frames（一个字段一个原子量）"
        );
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

    /// 判据（`line/engine-26` 注入 R13 的处置）：`EngineRuntime` 每个量子开始都必须
    /// 调用一次 `MeterBank::begin_quantum`，因此
    /// [`EngineRuntime::meter_active_nodes`] 是**本量子**的计量节点数，不是累计量。
    ///
    /// 为什么需要它：`begin_quantum` 的唯一效果是把池里的 `measured` 归零，而那个计数
    /// 在 `EngineRuntime` 之外**没有任何读者**（`EngineStats` 不搬它）⇒ 删掉那个调用，
    /// 全部既有判据都看不出差别（本票注入 R13 实测：那个注入之下没有任何判据变红）。
    /// 本判据把"本量子 vs 累计"做成可判定的差分：
    /// 夹具（`simple_snapshot(1)`）每个量子走 `MeterBank::measure` **1** 次
    /// （1 条轨；母线走 `measure_bus_stereo`，不计入此数），三个量子之后必须是 **1**；
    /// 累计会给出 **3**。
    #[test]
    fn meter_active_nodes_is_per_quantum_not_cumulative() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for quantum in 1..=3u64 {
            rig.runtime.process_quantum(&mut out, 2);
            assert_eq!(
                rig.runtime.meter_active_nodes(),
                1,
                "第 {quantum} 个量子：夹具 `simple_snapshot(1)` 逐轨计量 1 次\
                 （累计会给出 {quantum}）"
            );
        }
        assert_eq!(rig.runtime.stats().quanta, 3, "覆盖度：真的跑了三个量子");
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

    /// 判据：主总线推子只在**有效帧**上乘，且每个有效样本恰好乘一次增益。
    ///
    /// 有效帧范围由 [`AudioBlock::stereo_mut`] 按 `set_frames` 截断给出（与
    /// [`sum_into_bus`] 共用同一个契约）；`frames` 之外的槽位**不由公共 API 暴露**，
    /// 因此本判据钉住的是"缩放的长度 == 有效帧数"。
    #[test]
    fn scale_bus_multiplies_valid_frames_only() {
        let mut block = AudioBlock::<DEFAULT_BLOCK_FRAMES>::new();
        block.set_frames(3);
        {
            let (left, right) = block.stereo_mut();
            left.fill(2.0);
            right.fill(-4.0);
        }
        scale_bus(&mut block, 0.5);
        assert_eq!(block.left(), &[1.0f32, 1.0, 1.0][..]);
        assert_eq!(block.right(), &[-2.0f32, -2.0, -2.0][..]);
        assert_eq!(block.left().len(), 3, "只缩放有效帧");
    }

    /// 判据（本票）：有限样本经 [`saturate_bus_to_finite`] **逐位不变**
    /// —— 含 `±0.0`、次正规数、`f32::MAX`、`f32::MIN` 与普通值。
    ///
    /// 这是"本守卫不改既有（有限）渲染输出"的**实现本体**：断言用 `to_bits()`，
    /// 因此 `-0.0` 与 `+0.0` 的差别也在量程内。
    #[test]
    fn the_bus_guard_leaves_every_finite_sample_bit_for_bit() {
        let finite = [
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            MAX_LINEAR_MAGNITUDE,
            -MAX_LINEAR_MAGNITUDE,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            f32::from_bits(1), // 最小次正规数
            f32::MAX,
            f32::MIN,
            1.0e-45,
        ];
        let mut left = finite;
        let mut right = finite;
        saturate_bus_to_finite(&mut left, &mut right);
        for (index, (got, want)) in left.iter().zip(finite.iter()).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "左声道第 {index} 个有限样本必须逐位不变（实得 {got:e}，期望 {want:e}）"
            );
        }
        assert_eq!(right.map(f32::to_bits), finite.map(f32::to_bits));
    }

    /// 判据（本票）：`NaN` 归 `0.0`（与限制器的 `nan_to_zero` 同口径），
    /// `±∞` 收进 `±`[`MAX_LINEAR_MAGNITUDE`]（而不是留给限制器 —— 那会变成 `NaN`）。
    ///
    /// 反向见证：本判据同时断言**饱和真的发生**（`16.0` 而不是原值），
    /// 因此它不会因为"守卫什么都没做"而假绿。
    #[test]
    fn the_bus_guard_saturates_non_finite_samples_into_the_linear_range() {
        let mut left = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -f32::NAN,
            0.5,
            f32::INFINITY,
        ];
        let mut right = [f32::NEG_INFINITY, 0.25, f32::NAN, f32::INFINITY, 2.0, 0.0];
        saturate_bus_to_finite(&mut left, &mut right);
        assert_eq!(
            left,
            [
                0.0,
                MAX_LINEAR_MAGNITUDE,
                -MAX_LINEAR_MAGNITUDE,
                0.0,
                0.5,
                MAX_LINEAR_MAGNITUDE
            ],
            "NaN ⇒ 0、±∞ ⇒ ±{MAX_LINEAR_MAGNITUDE}，有限值不动"
        );
        assert_eq!(
            right,
            [
                -MAX_LINEAR_MAGNITUDE,
                0.25,
                0.0,
                MAX_LINEAR_MAGNITUDE,
                2.0,
                0.0
            ]
        );
        assert!(
            left.iter().chain(right.iter()).all(|s| s.is_finite()),
            "守卫之后不允许还有非有限样本"
        );
    }

    /// 判据（本票）：饱和值必须在**限制器**里得到定义好的结果 ——
    /// `+MAX_LINEAR_MAGNITUDE` 进限制器 ⇒ 输出有限且不超过天花板。
    ///
    /// 它钉住"为什么取 16.0 而不是 `f32::MAX`"：`f32::MAX` 会让目标增益
    /// `0.9 / 3.4e38` 落进**次正规数**，被 FTZ/DAZ 冲刷成 `0.0` ⇒ 输出静音。
    #[test]
    fn the_saturation_value_survives_the_bus_limiter() {
        let mut limiter = BusLimiter::new();
        let mut left = [MAX_LINEAR_MAGNITUDE; DEFAULT_BLOCK_FRAMES];
        let mut right = [-MAX_LINEAR_MAGNITUDE; DEFAULT_BLOCK_FRAMES];
        saturate_bus_to_finite(&mut left, &mut right);
        // 守卫对有限值逐位不变 ⇒ 送进限制器的就是 ±16.0。
        assert_eq!(left[0], MAX_LINEAR_MAGNITUDE);
        limiter.process_stereo(&mut left, &mut right);
        assert!(
            left.iter().chain(right.iter()).all(|s| s.is_finite()),
            "±16.0 经母线限制器之后必须全是有限值"
        );
        let peak = left[crate::mixer::LIMITER_LATENCY_FRAMES..]
            .iter()
            .chain(right[crate::mixer::LIMITER_LATENCY_FRAMES..].iter())
            .fold(0.0f32, |acc, s| acc.max(s.abs()));
        assert!(
            peak > 0.0,
            "饱和值必须产出**有声**的有限输出，而不是被冲刷成静音"
        );
        assert!(
            peak <= crate::mixer::LIMITER_CEILING + f32::EPSILON,
            "限制器契约：限制后峰值 ≤ 天花板（实得 {peak}）"
        );
    }

    /// 判据：轨道数超过电平状态容量时**不 panic、不扩容**，而是计数并保留母线。
    ///
    /// `line/engine-24` 追加了 `track_drops` 那一格：它与 `meter_capacity_drops` 是
    /// **两个不同的容量面**（前者是声部池的 `MAX_TRACK_SLOTS`，后者是电平的
    /// `SCRATCH_METERS`）⇒ 必须各自被钉住。量法（单位：条）：注入实测把
    /// `EngineStats::track_drops` 那一格硬写成 `0` 之后，整个
    /// `cargo test -p yeban-engine --no-default-features --all-targets`（**20** 个目标）
    /// **全绿** ⇒ 在此之前那条搬运**没有任何判据**。
    /// 期望值 `300 − MAX_TRACK_SLOTS = 284`：夹具是 300 条**非母线**轨，声部池只有
    /// [`MAX_TRACK_SLOTS`] 个槽，`begin_snapshot` 按 `BTreeMap` 键序把前 16 条放进池、
    /// 其余逐条计入丢弃（`synth.rs` 的 `None => self.track_drops.saturating_add(1)`）。
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
        // 声部池那一侧：300 条非母线轨只有 16 个声部槽 ⇒ 284 条被丢弃。
        assert_eq!(
            stats.track_drops,
            (SCRATCH_METERS + 44 - crate::synth::MAX_TRACK_SLOTS) as u64,
            "超出声部池的 {MAX_TRACK_SLOTS} 个槽的轨道必须逐条计入 `track_drops`；\
             这条搬运此前没有任何判据（把那一格硬写成 0 时 20 个目标全绿）",
            MAX_TRACK_SLOTS = crate::synth::MAX_TRACK_SLOTS
        );
    }

    /// 判据：**插入槽**的上限边界是 `>= MAX_TRACK_SLOTS`（不是 `>`）。
    ///
    /// `process_quantum` 的插入武装循环把通道条装进
    /// `armed_strips: [(EntityId, Option<ChannelStrip>); MAX_TRACK_SLOTS]`（定长栈数组）。
    /// 守卫若写成 `*armed_insert_slots > MAX_TRACK_SLOTS`，第 `MAX_TRACK_SLOTS + 1`
    /// 条带插入的轨就会写 `armed_strips[MAX_TRACK_SLOTS]` ⇒ **数组越界 panic**。
    /// 声相槽那一侧有既有判据守着（注入 `self.armed_pan_slots >= MAX_TRACK_SLOTS` → `>`
    /// 实测变红），插入槽这一侧此前没有 —— 三张姊妹表（声相 / 通道条 / 混响 / 卷积）
    /// 的同一道边界必须各自有判据。
    ///
    /// **量什么**：`MAX_TRACK_SLOTS + 1` 条各带一台通道条的轨，渲染 1 个量子是否
    /// 返回（不 panic）、`track_drops`（条）。
    ///
    /// 注入实测：`if *id == master || *armed_insert_slots >= MAX_TRACK_SLOTS {` → `>`
    /// （`rt.rs`）⇒ 本判据实测变红（`index out of bounds`）。
    #[test]
    fn insert_slot_exhaustion_is_counted_and_never_panics() {
        let slot = SnapshotSlot::new(snapshot_with_master_track_and_strips(
            1,
            crate::synth::MAX_TRACK_SLOTS + 1,
        ));
        let (retire, _queue) = retire_channel(16);
        let (_sender, receiver) = event_channel(64);
        let (publisher, _collector) = meter_channel(1024);
        let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        runtime.process_quantum(&mut out, 2);
        assert_eq!(
            runtime.stats().track_drops,
            1,
            "第 {} 条带插入的轨必须被计数（不 panic、不扩容）",
            crate::synth::MAX_TRACK_SLOTS + 1
        );
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

    /// 判据（R50②）：`--no-default-features` 下**因 feature 门不参与编译**的清单必须显式。
    ///
    /// 本 crate 只有一条 feature 链：`device = ["dep:cpal"]`，且 `default = ["device"]`。
    /// **关掉之后**：`src/device.rs` **整个模块**不编译（⇒ 它的 `mod tests` 也不存在，
    /// 本模块 3 条判据所在的文件整体缺席）、`src/rt.rs` 有 1 段 device 专属代码不编译、
    /// `tests/rt_zero_alloc.rs` 有 4 条 device 专属判据不编译、
    /// `examples/measure_latency.rs` 有 2 段不编译。
    /// 这张清单任何一处被**静默**改动（例如 `#[cfg]` 拼错成恒假），都会让"四道闸门
    /// 全绿"这句话的覆盖面缩水而**不报红** —— 本判据把它变成机械读数。
    ///
    /// ⚠ **针由运行时拼出**（不是字面量）：否则 `include_str!("rt.rs")` 会把本判据
    /// **自己源码里的那一段字面量**也算进去（"黄金表让被钉文本出现两次"，硬规则 4）。
    ///
    /// **量什么**：四个文件里 feature 门出现的**次数**（单位：处）。
    /// 注入实测（第四批）：把 `src/rt.rs` 的门改成 `device_gate_probe` ⇒
    /// *默认档*下全绿，*device 档*下本判据实测变红。
    #[test]
    fn the_default_off_build_has_an_explicit_feature_gate_inventory() {
        let needle = ["#[cfg(feature = ", "\"device\")]"].concat();
        let lib = include_str!("lib.rs");
        assert!(
            lib.contains(&format!("{needle}\npub mod device;")),
            "`device` 模块必须仍然被门控"
        );
        assert_eq!(
            lib.matches(needle.as_str()).count(),
            1,
            "lib.rs 的 device 门数"
        );
        assert_eq!(
            include_str!("rt.rs").matches(needle.as_str()).count(),
            1,
            "rt.rs 的 device 门数"
        );
        assert_eq!(
            include_str!("../tests/rt_zero_alloc.rs")
                .matches(needle.as_str())
                .count(),
            4,
            "rt_zero_alloc.rs 的 device 门数"
        );
        assert_eq!(
            include_str!("../examples/measure_latency.rs")
                .matches(needle.as_str())
                .count(),
            2,
            "measure_latency.rs 的 device 门数"
        );
        // R58：`==` 的判据必须另有一条 `assert_ne!` 落在**同一个**表达式上 ——
        // 这里证明这个计数器**真的能区分不同的文件**（1 vs 4），不是恒等于同一个值。
        assert_ne!(
            lib.matches(needle.as_str()).count(),
            include_str!("../tests/rt_zero_alloc.rs")
                .matches(needle.as_str())
                .count(),
            "lib.rs(1) 与 rt_zero_alloc.rs(4) 的门数必须不同（否则计数器没有区分力）"
        );
    }
    /// 一份**带一个长音符**、可指定静态声相的快照：用来观察声相自动化真的作用到样本上。
    fn note_snapshot_with_pan(revision: u64, pan: f32) -> (EngineSnapshot, EntityId) {
        let track = EntityId::new();
        (note_snapshot_with_pan_for(revision, pan, track), track)
    }

    /// 同 [`note_snapshot_with_pan`]，但**轨身份由调用方给**：用于"同一条轨换快照"
    /// 与"槽位换主人"两类判据（前者要保持 id，后者要换 id）。
    fn note_snapshot_with_pan_for(revision: u64, pan: f32, track: EntityId) -> EngineSnapshot {
        let master = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![track, master],
            ..RoutingGraph::default()
        };
        let edge = EntityId::new();
        routing.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        let mut tracks: BTreeMap<EntityId, TrackParams> = BTreeMap::new();
        let model = TrackV3 {
            id: track,
            pan,
            ..TrackV3::default()
        };
        tracks.insert(track, TrackParams::from_track(&model, 0));
        let mut schedules: BTreeMap<EntityId, crate::synth::NoteSchedule> = BTreeMap::new();
        let note = crate::synth::ScheduledNote::new(
            0,
            480_000,
            60,
            127,
            yeban_dsp::math::note_to_hz(60.0),
            1.0,
            48_000.0,
        );
        schedules.insert(track, crate::synth::NoteSchedule::from_sorted(vec![note]));
        let snapshot = EngineSnapshot::from_parts(
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
        .with_schedules(schedules, 0);
        snapshot
    }

    /// 从一个给定快照装一台运行时（事件通道容量 64）。
    fn rig_with_snapshot(snapshot: EngineSnapshot) -> Rig {
        let slot = SnapshotSlot::new(snapshot);
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

    /// 判据（裁决 P4=(b)）：声相自动化**被事件循环截走**、**逐样本平滑**、并**收敛到目标**。
    ///
    /// 四条读数**一次跑出来**：
    /// ① `param_unmapped_events == 0` ⇒ 事件被 `rt.rs` 截走（**不是**落进 `ParamTable`
    ///    被判 `Unmapped`）—— 这一条钉住"分流点在哪"；
    /// ② `pan_automation_frames() > 0` ⇒ 平滑路径真的被走到（覆盖度见证）；
    /// ③ 第一量子的右声道**还**不是 0 ⇒ 是**平滑**而不是阶跃（阶跃会有 zipper）；
    /// ④ 跑够量子之后右声道**恰好** 0.0 ⇒ 平滑器吸附到硬左目标。
    #[test]
    fn pan_automation_is_intercepted_and_smoothed_per_sample() {
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        // 基线（居中）：左右逐位相同 —— 证明下面观察的差异来自声相，不是别的。
        assert!(
            out.iter()
                .step_by(2)
                .zip(out.iter().skip(1).step_by(2))
                .all(|(l, r)| l.to_bits() == r.to_bits()),
            "居中声相 ⇒ 左右必须逐位相同"
        );

        // 自动化：硬左（绝对增益 L = 1.0、R = 0.0）。
        let events = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: 1.0,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
                value: 0.0,
            },
        ];
        assert_eq!(rig.sender.publish(&events), 2, "两个声相事件必须都进通道");
        let mut first = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut first, 2);
        assert_eq!(
            rig.runtime.stats().param_unmapped_events,
            0,
            "声相槽位必须被 rt.rs 截走，不得被判成未映射"
        );
        assert!(
            rig.runtime.pan_automation_frames() > 0,
            "平滑路径必须真的被走到（见证）"
        );
        assert!(
            first.iter().skip(1).step_by(2).any(|r| *r != 0.0),
            "第一量子的右声道不得已经归零（那说明是阶跃，不是平滑）"
        );

        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut settled = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut settled, 2);
        assert!(
            settled.iter().skip(1).step_by(2).all(|r| *r == 0.0),
            "硬左之后右声道必须**恰好**为 0（平滑器吸附到目标）"
        );
        // R58：等号判据的 `assert_ne!` 落在同一个表达式上 —— 左声道必须仍然有声。
        assert!(
            settled.iter().step_by(2).any(|l| *l != 0.0),
            "硬左的左声道不得也是 0（否则上面那条是静音对静音）"
        );
    }

    /// 判据：**不静默** —— 找不到槽位的声相事件、以及**负值**都被计数拒绝。
    ///
    /// 负值这一条不是假想：硬右时 `pan_gains(1.0).0` 实测是 `-4.371139e-8`
    /// （见 `crate::mixer` 的端点判据）⇒ 控制侧**必须 clamp 到 `>= 0`**，
    /// 否则硬右的左增益会被这里拒掉（静默丢失）。
    #[test]
    fn pan_automation_rejects_unknown_tracks_and_negative_values() {
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        // 先武装（否则"找不到槽位"会把已知轨的事件也算成拒绝，判据就不精确了）。
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);
        let unknown = EntityId::new();
        let events = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(unknown, TRACK_PAN_LEFT_SLOT),
                value: 0.5,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: -4.371_139e-8,
            },
        ];
        assert_eq!(rig.sender.publish(&events), 2);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        // ⚠ 未知轨 ⇒ **找得到槽位吗？找不到** ⇒ 交给 `accept` ⇒ 判 `Unmapped` 并计数
        // （单一归属：未映射只有一处裁决）。它**不是**静默丢弃。
        assert_eq!(
            rig.runtime.stats().param_unmapped_events,
            1,
            "未知轨的声相事件必须被判未映射并计数（⛔ 不静默）"
        );
        assert_eq!(
            rig.runtime.pan_automation_value_rejects(),
            1,
            "负的声相增益必须被计数拒绝（硬右的左增益就是负的！）"
        );
        assert_eq!(
            rig.runtime.pan_automation_frames(),
            0,
            "两次都没生效 ⇒ 不得走平滑路径"
        );
    }

    /// 判据（**边界，必须不静默**）：声相表是**快照派生**的 ⇒ 在第一次武装之前
    /// 到达的声相事件**找不到槽位**，因此被计进 `pan_automation_rejects`。
    ///
    /// 为什么值得钉：生产上 UI 的第一跳有可能早于音频线程处理第一个量子
    /// （`EngineHost::publish_automation` 在 UI 线程、表在音频线程武装）。
    /// 那一刻的事件**会丢一拍**；本条判据保证它**可见**（计数），
    /// 而 app 侧每一跳都重新采样 ⇒ 下一跳自愈。
    /// ⛔ 不许把它改成静默（那会让"自动化不动"变成不可归因）。
    #[test]
    fn a_pan_event_before_the_first_arming_is_counted_not_silent() {
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        // **不**预热：表还没武装（`armed_pan_slots == 0`）。
        let events = [EngineEvent::SetParam {
            target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
            value: 1.0,
        }];
        assert_eq!(rig.sender.publish(&events), 1);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        assert_eq!(
            rig.runtime.stats().param_unmapped_events,
            1,
            "武装之前的事件必须被判未映射并**计数**（⛔ 不静默）"
        );
        assert_eq!(
            rig.runtime.pan_automation_frames(),
            0,
            "它没有生效 ⇒ 不得走平滑路径"
        );
        // 自愈的一半：**同一个**事件在表武装之后再发一次 ⇒ 生效。
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);
        assert_eq!(rig.sender.publish(&events), 1);
        let mut after = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut after, 2);
        assert!(
            rig.runtime.pan_automation_frames() > 0,
            "武装之后重发必须生效（app 每跳重采样 ⇒ 自愈）"
        );
    }

    /// 判据（裁决：与增益槽位同型）：**换快照不覆盖在飞的声相自动化**，
    /// 而换工程之后的**新事件继续生效**。
    #[test]
    fn a_snapshot_swap_does_not_override_an_in_flight_pan_automation() {
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        // 先跑一个量子：快照边界**武装**声相表（表是快照派生的 ⇒ 武装之前没有槽位）。
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);
        let events = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: 1.0,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
                value: 0.0,
            },
        ];
        assert_eq!(rig.sender.publish(&events), 2);
        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }

        // 换一份**静态声相 = 硬右**的快照，**同一条轨 id**（⇒ 同一个槽位；
        // 换主人是另一回事，见 `a_pan_slot_changing_owner_...`）。
        let hard_right = note_snapshot_with_pan_for(2, 1.0, track);
        rig.slot.publish(hard_right);
        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut after_swap = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut after_swap, 2);
        assert!(
            after_swap.iter().skip(1).step_by(2).all(|r| *r == 0.0),
            "换快照不得覆盖在飞的自动化（右声道必须仍然是 0）"
        );

        // 换工程**之后**的新事件继续生效：改成硬右（L = 0、R = 1）。
        let new_events = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: 0.0,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
                value: 1.0,
            },
        ];
        assert_eq!(rig.sender.publish(&new_events), 2);
        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut after_events = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut after_events, 2);
        assert!(
            after_events.iter().step_by(2).all(|l| *l == 0.0),
            "换工程之后的新事件必须继续生效（左声道收敛到 0）"
        );
        assert!(
            after_events.iter().skip(1).step_by(2).any(|r| *r != 0.0),
            "右声道必须重新有声（不是静音对静音）"
        );
    }

    /// 判据（类别③ **换主人**）：声相槽位是**按下标复用**的 ⇒ 新主人**不得**继承
    /// 上一任的声相自动化目标 —— 否则会把另一条轨的声相播给这一条。
    #[test]
    fn a_pan_slot_changing_owner_does_not_inherit_the_automation() {
        let (snapshot_a, track_a) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot_a);
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);
        // 轨 A 武装**硬左**（右增益目标 = 0）。
        let events = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track_a, TRACK_PAN_LEFT_SLOT),
                value: 1.0,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track_a, TRACK_PAN_RIGHT_SLOT),
                value: 0.0,
            },
        ];
        assert_eq!(rig.sender.publish(&events), 2);
        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }

        // 换一份**另一条轨**的快照（静态**居中** ⇒ 左右逐位相同）。
        let (snapshot_b, track_b) = note_snapshot_with_pan(2, 0.0);
        assert_ne!(track_a, track_b, "夹具前提：两条轨身份必须不同");
        rig.slot.publish(snapshot_b);
        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        assert!(
            out.iter()
                .step_by(2)
                .zip(out.iter().skip(1).step_by(2))
                .all(|(l, r)| l.to_bits() == r.to_bits()),
            "新主人必须从它**自己的**静态声相（居中）起步，⛔ 不得继承上一任的硬左目标"
        );
    }
}
