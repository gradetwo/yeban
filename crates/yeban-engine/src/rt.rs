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
//! ## ⭐ 总线（aux 返回）上的插入链（任务单 `#158`）
//!
//! `#154` 让实时引擎消费路由图，但总线暂存池里的**累积信号**只乘边增益、不经过
//! 那条总线自己的插入链 ⇒ `SendToAux` 送到的是**干**总线。本片把插入链补在
//! **3a⁗ 总线转发之前**：一条总线若**有已武装的插入器件**（通道条／混响／卷积混响）
//! 且**真的收到**别的节点搬来的音频，就先把累积信号送过链，再按出边转发。
//!
//! ### 裁决 **(甲)**：**下混成单声道 → 插入 → 复制到两路**（⛔ 不是左右各跑一套）
//!
//! 理由与代价（逐条登记，⛔ 不隐藏）：
//!
//! 1. **零新状态**：总线本身是一条 `TrackV3`，它的器件已经占着逐轨武装池
//!    （[`BranchShare::armed_strips`] / `armed_reverbs` / `armed_convs`）里的一个槽位
//!    ⇒ 转发阶段**复用同一台器件**即可，`EngineRuntime` / `EngineSnapshot` **＋0 B**。
//!    左右各一套则需要每总线两份器件状态：`Reverb` 的 24 条延迟线（2 × 8 comb ＋
//!    2 × 4 allpass，约 2.3 万样本 @48 kHz）与 `ConvolutionReverb` 的 4 条 IR 频谱
//!    ＋ 2 条预延迟线都要翻倍，两个池 `[Reverb; 16]` / `[ConvolutionReverb; 16]`
//!    要各扩到 32 槽，快照侧三张 `armed_*` 表也要 32 槽 ⇒ 尺寸与构造期堆都翻倍。
//! 2. **CPU**：单声道只跑一条链；左右各一套是 2× 滤波器/压缩器/混响运算。
//! 3. **与既有实时口径一致**：逐轨插入本来就是**单声道**（声相之前、混响"单声道喂
//!    两路取中值"）。总线插入用同一条链、同一条投影，不引入第二套"立体声插入"语义。
//! 4. **代价（如实登记）**：总线累积信号被**下混**（`(L + R) · 0.5`）后处理、再复制回
//!    两路 ⇒ **只要总线上有已武装的器件**（哪怕 `reverb_wet: 0`／EQ 全 0），该总线的
//!    发送就不再保留左右差异。居中声相的发送逐位不变（`L == R` 时下混恰等于原值）；
//!    硬声相的发送会变。这与**离线渲染**（`yeban-render` 的 `crate::inserts` 是
//!    真立体声插入）的差异见报告 §7：对齐的是"有/没有"插入，⛔ 不是逐位相同。
//! 5. **与导出的顺序差**：离线是"声源 → 声相 → 求和 → 插入（立体声）"；实时是
//!    "逐轨单声道插入 → 声相 → 求和 → 总线插入（下混）"。EQ 是逐声道线性可交换的，
//!    湿路的宽度/尾巴形状则会有差异（`#155` 已登记同一族差异）。
//!
//! ### 每量子**每节点恰好一次**
//!
//! 一条**处理型总线**（有器件 ＋ 有入边）的插入链在逐轨循环里**整段跳过**，只在
//! 3a⁗ 跑一次；它自己的声源（若有）先以声相之后的立体声并入自己的暂存槽，与发送
//! **一起**过链。这样器件的**时间状态**（混响延迟线、压缩器弹道）每量子只推进
//! `frames` 个样本 —— 若两处都跑，尾巴会以两倍速衰减（那是听得见的错，不是口径差）。
//! 判据 `tests/bus_insert.rs` 的 B1／B2／B6 分别用
//! `insert_reverb_frames`／`insert_strip_frames`／`insert_convolution_frames`
//! **等于渲染帧数**钉住这一点。
//!
//! ### 快路（默认工程一分钱都不多花）
//!
//! * `RoutingPlan::uses_buses()` 为假（所有节点直连 master）⇒ 整个 3a⁗ 段与
//!   "处理型总线"判定都**不进**（`&&` 短路）；
//! * 总线没有已武装器件（无设备／**全旁通**／参数名一个都不认识）⇒ 逐轨路径与
//!   `#154` 逐位相同；
//! * 总线有器件但**没有入边** ⇒ 不搬（保持接线前的"链在逐轨跑"）。
//!
//! ### 电平读数
//!
//! 处理型总线的电平帧改在 3a⁗ **插入之后**计量（读的是送进来的信号 ＋ 自己的声源），
//! 仍满足"每量子每节点恰好一次"；其它轨道的取样点**一位没动**。
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
// ⭐ 任务单 `#107`：MIDI CC 的**会话运行态**表（`EngineEvent::ControlChange` 的落点，
// 也是实时 SFZ 通路的 CC 探针来源）。定长、`Copy`、零分配（见该模块文档）。
use crate::cc_state::ControlChangeState;
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
use crate::routing::{RouteTarget, RoutingPlan};
use crate::rt_probe::{self, RtDiagEvent};
use crate::snapshot::{EngineSnapshot, RetireProducer, SnapshotReader, SnapshotSlot};
use crate::stats_mirror::EngineStatsMirror;
use crate::synth::{MAX_TRACK_SLOTS, SynthEngine};
use crate::transport::{
    Transport, TransportEffect, TransportMirror, TransportReading, TransportState, next_beat_tick,
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
    /// ⭐ 环音符事件触发时的**窃取/重触发**次数（⭐ 与 `events_applied` **分开**）。
    ///
    /// ⚠ 口径：⭐ 池满（`active_voices == VOICES_PER_TRACK`）或"⭐ 同 pitch 未松键又按"
    /// ⇒ ⭐ +1。⚠ ⭐ **不区分**被窃取者是否已 `Idle`（⭐ 器件内部状态，⭐ 这里看不到）。
    pub note_steals: u64,
    /// ⭐ `NoteOff` **找不到匹配活跃声部**的次数（⭐ 与 `events_applied` **分开**）。
    ///
    /// ⚠ **合并口径（⭐ 三条，⭐ 与器件 `PolySynth::note_off` 的双支路一起读）**：
    /// ① ⭐ 本计数**不区分**"⭐ 从未存在过"与"⭐ 已被窃取/⭐ 已换音"
    /// （⭐ 因为不记槽位 ⇒ ⭐ 两者无法区分）。⭐ ⛔ 别读成"⭐ 从未存在"；
    /// ② ⭐ **被窃取音符的释放必然变成孤儿**：⭐ 窃取那一刻槽位改宣称**新**频率
    /// （⭐ 这正是被窃取者"⭐ 不再存在"的形态）⇒ ⭐ 指向**旧**频率的 `NoteOff`
    /// 配不上任何声部；
    /// ③ ⭐ **淡出窗口内**（⭐ 提升之前）槽位**仍宣称被窃取者的频率** ⇒ ⭐ 指向**旧**
    /// 频率的释放**仍然作用在被窃取者身上**（⭐ 不是孤儿）；⭐ 指向**新**频率的释放
    /// 在 (β) 之前**既配不上旧频率、⭐ 又还没有新频率** ⇒ ⭐ 被记成孤儿、⭐ 新音符此后
    /// **只能由后续松键**结束（⭐ 用户可见的挂音）。⭐ **(β) 之后**新音符自己的释放
    /// **在窗口内也能生效**（⭐ 器件用 `pending_released` 记下、⭐ 提升点补 `gate_off`）
    /// ⇒ ⭐ 这一类孤儿不再产生（⭐ 判据：
    /// `a_release_for_the_stolen_note_works_inside_the_fade_window_and_after_promotion`）。
    pub orphan_note_offs: u64,
    /// 累计快照切换次数。
    pub snapshot_switches: u64,
    /// ⭐ 累计**真的生效**过的量化场景启动次数（[ARCH-RT-005 族]；幂等见证）。
    ///
    /// ⚠ 没有它，"到点触发恰好一次"这条判据没有牙齿：计数缺席时 `== 1` 与
    /// "重复触发"看起来一样。
    pub scene_launches_applied: u64,
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

/// A/B 盲听的**第二份可渲染版本**（[ARCH-RT-005]）。
///
/// ## 为什么是「再加一个 `SnapshotSlot`」，而不是给现有槽加第二指针
///
/// [`SnapshotSlot`] 的**纪元握手**（写者先 `ptr.store` 再 `epoch.fetch_add`，读者用
/// `reader_done` 证明「我不会再碰被淘汰的快照」）是一条**已经证明过的协议**
/// （`crate::snapshot` 的模块文档）。在同一个结构里再加一套指针/锚/纪元，就要把那条
/// 证明**重做一遍**；而「再来一个 `SnapshotSlot`」是把同一条协议**原样复用**。
///
/// ⚠ 第二份槽的**创建**在宿主（`yeban-app`）：本模块只**接受**传入的槽与退役生产者
/// ——「第二份版本从哪来」是投影/会话层的职责，⛔ 不是引擎的。
///
/// ## 片 1：只**持有**，⛔ 不渲染、⛔ 不淡化、⛔ 无预滚
///
/// 本片的渲染路径**一个字都没改**（[`EngineRuntime::render_block`] 的解构以 `..`
/// 收尾 ⇒ `audition` 根本不被绑定）。第二读者要到**片 2** 才会被 `begin_block()`；
/// 在此之前它只是「在册但从不读」⇒ **每量子 0 次原子操作、0 次分配**。
pub struct AuditionBranch {
    /// 第二份版本的读者。⚠ 在册化时它**立刻**锚住该槽当时的快照（见
    /// [`SnapshotReader::attach`]）⇒「连到的是哪一份」从构造起就可观测。
    reader: SnapshotReader,
    /// **第二个合成引擎**（[ARCH-RT-005] 片 2b）。
    ///
    /// ⚠ **必须在构造期建立**：`SynthEngine` 自带声部池与调度表，在回调里 `new`
    /// 就是分配 ⇒ 违反红线 7。两条分支各自的声部必须**同时存活**（这正是不能用
    /// 一个引擎先后渲染两份的原因：`begin_snapshot` 会重播声部）。
    /// ⚠ **装箱**：`SynthEngine` 很大，直接内联会把 [`EngineRuntime`] 撑大，
    /// 而它在栈上被构造 ⇒ 已实测到**栈溢出**（`device` 的判据）。
    /// 装箱后运行时只多一个指针；堆分配发生在**控制路径**的 `attach`。
    synth: Box<SynthEngine>,
    /// 第二分支的**输出缓冲**（构造期预分配固定长度，⛔ 回调内不增长）。
    block: Box<AudioBlock<DEFAULT_BLOCK_FRAMES>>,
    /// 第二引擎已武装到哪个修订号（`None` ＝ 还没武装）。
    armed_revision: Option<u64>,
    /// **等功率淡化表**：`fade[k] = crossfade_gains(k / fade_len)`，长度 `fade_len + 1`。
    ///
    /// ⭐ **为什么是一张表**：[`crate::mixer::crossfade_gains`] 里有 `cos`/`sin`，
    /// 属**超越函数** ⇒ ⛔ 不许出现在逐样本路径上（[MUST-GATE-001]）。
    /// 控制路径算一次、实时侧只做**整数索引** ⇒ 逐样本只有"两次乘一次加"。
    /// 它在控制路径分配、此后**不再增长**；`Drop` 只发生在运行时销毁时。
    fade: Box<[(f32, f32)]>,
    /// 距**切换帧**还剩多少帧（控制路径置入；回调内只减）。
    frames_until_switch: u64,
    /// 本次试听的**绝对切换帧**（来自 [`Transport::audition_switch_frame`]）。
    switch_frame: u64,
    /// 第二分支**真的被渲染过**的块数（[MUST-GATE-001] 的**覆盖度见证**）。
    ///
    /// ⭐ 没有它，"空闲时输出不变"那条判据可能是**空转**：只要淡化表还没算出来，
    /// 混合函数就会提前返回、不改一个样本 —— 于是"误入试听路径"也测不出来。
    /// 这个计数把"空闲时**一次都不该读**第二分支"变成**可观测**的。
    blocks_read: u64,
}

/// 试听被**拒绝**的原因（[ARCH-RT-005] 片 2b 的**受限版**口径）。
///
/// ⭐ **为什么要有这个枚举**：受限版让两条分支**共用同一套武装表**
/// （逐轨增益/声相/通道条/混响/卷积/PDC 都是按**主线**那一份快照武装的）。
/// ⇒ 一旦提案在这些**拓扑**上与主线不同，拿主线的武装表去渲染提案就会
/// **静默地把 A 的参数套在 B 的材料上**。本枚举让那种情况**拒绝试听**、
/// 并把「哪一类不一致」如实报出去。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditionRefusal {
    /// 主线侧**还没有任何快照** ⇒ 无从保证"共用武装表" ⇒ 拒绝。
    NoMainSnapshot,
    /// **音频配置**不同：采样率 / 块长 / 声道数 / 声相律。
    AudioConfig,
    /// **设备链**不同：轨道集合、每轨混音参数（增益/声相/静音/独奏/器件数）或插入参数。
    DeviceChain,
    /// **自动化目标**不同（宿主投影的定长指纹）。
    AutomationTargets,
    /// **PDC 计划**不同（拓扑序 / 累积延迟 / 补偿延迟）。
    PdcPlan,
}

impl AuditionRefusal {
    /// 稳定短名（用于读数与错误报告；⛔ 不改语义）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMainSnapshot => "no-main-snapshot",
            Self::AudioConfig => "audio-config",
            Self::DeviceChain => "device-chain",
            Self::AutomationTargets => "automation-targets",
            Self::PdcPlan => "pdc-plan",
        }
    }
}

/// **自动化目标的定长指纹**（[ARCH-RT-005] 片 2b）。
///
/// ⚠ **为什么不把自动化目标放进 [`EngineSnapshot`]**：快照是 `Arc` 共享的，
/// 它的最后一次 `Drop` 可能落在**音频线程**上；往里放 `BTreeMap`/`Vec` 就等于
/// 把一次释放搬进回调 —— [ARCH-RT-001] 正是为了这个才不让快照搬 `String`。
/// ⇒ 指纹是**定长 + `Copy`** 的：无堆、无 `Drop`。
///
/// 语义：`entities[i]` 是被自动化寻址的实体，`counts[i]` 是它的目标条数。
/// 它是**指纹**（足以发现不一致），⛔ 不是目标身份的完整表示。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationTargets {
    entities: [EntityId; MAX_TRACK_SLOTS],
    counts: [u16; MAX_TRACK_SLOTS],
    len: usize,
}

impl AutomationTargets {
    /// 没有任何自动化目标。
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entities: [EntityId::default(); MAX_TRACK_SLOTS],
            counts: [0; MAX_TRACK_SLOTS],
            len: 0,
        }
    }

    /// 从**已按实体升序去重**的 `(实体, 目标条数)` 序列投影（多余项被截断）。
    ///
    /// ⚠ 调用方负责升序去重：本类型**不排序、不分配**（定长写入）。
    #[must_use]
    pub fn from_sorted(pairs: &[(EntityId, u16)]) -> Self {
        let mut out = Self::empty();
        for (entity, count) in pairs.iter().take(MAX_TRACK_SLOTS) {
            out.entities[out.len] = *entity;
            out.counts[out.len] = *count;
            out.len += 1;
        }
        out
    }

    /// 有效项数。
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// 是否为空。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// 两份版本的**拓扑**是否一致；不一致 ⇒ `Some(原因)`（[ARCH-RT-005] 片 2b）。
///
/// ## 判什么、为什么只判这些
///
/// 受限版让两条分支共用主线的武装表 ⇒ **凡是喂进武装表的快照事实都必须相等**：
///
/// | 检查 | 喂进哪张武装表 |
/// | :--- | :--- |
/// | 采样率 / 块长 / 声道数 / 声相律 | `ParamTable` 的采样率、`armed_pan_law`、PDC 的对齐 |
/// | 轨道集合与每轨混音参数 | `armed_pan_gains`、`synth.set_track_gain_scales`、`armed_strips` 槽位 |
/// | 插入参数 | `armed_insert_slots` / 混响 / 卷积池 |
/// | 自动化目标指纹 | `ParamTable` 的槽位与 `armed_pan_*` |
/// | PDC 计划 | `CompensationBank` 的逐节点延迟 |
///
/// ⛔ **刻意不判**的：音符/编排（`schedules`）、音色（`tones`）、鼓机（`drums`）、
/// 走带位置 —— 那正是 A/B 要比较的**材料**，它们**允许**不同（分支 B 有自己的
/// [`SynthEngine`]，按 B 自己的快照武装）。
///
/// ⚠ 已知**未覆盖**：节拍器计划（母线级信号，目前由宿主保证两份一致）。
/// 这条局限是**明说的**，⛔ 不是遗漏。
#[must_use]
pub fn audition_topology_mismatch(
    main: &EngineSnapshot,
    proposal: &EngineSnapshot,
    main_automation: AutomationTargets,
    proposal_automation: AutomationTargets,
) -> Option<AuditionRefusal> {
    // ① 音频配置。
    if main.sample_rate() != proposal.sample_rate()
        || main.block_frames() != proposal.block_frames()
        || main.channels() != proposal.channels()
        || main.pan_law() != proposal.pan_law()
    {
        return Some(AuditionRefusal::AudioConfig);
    }
    // ② 设备链：轨道集合 + 每轨混音参数（⭐ 整体相等，见上表）。
    if main.tracks() != proposal.tracks() || main.inserts() != proposal.inserts() {
        return Some(AuditionRefusal::DeviceChain);
    }
    // ③ 自动化目标指纹。
    if main_automation != proposal_automation {
        return Some(AuditionRefusal::AutomationTargets);
    }
    // ④ PDC 计划。
    if main.pdc() != proposal.pdc() {
        return Some(AuditionRefusal::PdcPlan);
    }
    None
}

/// 量化档 → "**几个小节**"（⭐ `Off` 不走这里：它在控制面直接取当前帧）。
///
/// ⚠ 取值**只来自模型既有枚举**（[`yeban_model::LaunchQuantization`]）：小节整数倍，
/// ⛔ 没有 1/16、1/4、单拍 ⇒ ⛔ 不发明新档位。
const fn quantization_bars(quantization: yeban_model::LaunchQuantization) -> u64 {
    use yeban_model::LaunchQuantization as Q;
    match quantization {
        Q::Off => 1, // ⚠ 不可达（调用点已把它分出去）；给 1 以免除零/空转。
        Q::Bar => 1,
        Q::TwoBars => 2,
        Q::FourBars => 4,
        Q::EightBars => 8,
    }
}

/// 一次**排队等待量化点**的场景启动（[ARCH-RT-005 族]：⭐ 照 [`AuditionBranch`] 的形状）。
///
/// ⭐ **"何时开始"与"开始什么"共用同一个判定**：`switch_frame` 由 `Transport` 的
/// [`next_bar_tick`](crate::transport::next_bar_tick) ＋ `frames_for_tick` 算出
/// （量化档只有小节整数倍 ⇒ 与 `launch_quantization` 的现有取值一一对应），
/// `tempo_bpm` 是**该场景自己的 tempo 覆盖**（[`yeban_model::SceneV3::tempo`]，
/// ⛔ 场景不承载别的可执行状态 ⇒ 本机制只投射 tempo）。
///
/// ⚠ **实时侧只做整数比较**：`frames_until_switch` 在控制路径算好，回调内每块只减
/// 一次、减到 0 才动作 ⇒ 零浮点、零分配、零锁、零 I-O（[MUST-GATE-001]）。
/// ⚠ 触发**恰好一次**：`apply_launch` 之后状态机回到 `None`（幂等，⛔ 不重复触发）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingSceneLaunch {
    /// 目标场景身份（⭐ 只作读数；引擎不认识场景实体）。
    pub scene_id: Option<yeban_model::EntityId>,
    /// 该场景的 **tempo 覆盖**（BPM）。
    pub tempo_bpm: f64,
    /// 生效的**绝对帧**（由 `switch_frame` 与当前位置在控制路径算好）。
    pub switch_frame: u64,
    /// 距生效帧还剩多少帧（控制路径置入；回调内只减）。
    pub frames_until_switch: u64,
}

impl PendingSceneLaunch {
    /// 控制路径：按**当前位置**与**生效帧**武装一次启动。
    ///
    /// ⚠ 只许在控制路径调用（不分配，但语义上是控制面的事）。
    #[must_use]
    pub fn arm(
        scene_id: Option<yeban_model::EntityId>,
        tempo_bpm: f64,
        switch_frame: u64,
        position_frames: u64,
    ) -> Self {
        Self {
            scene_id,
            tempo_bpm,
            switch_frame,
            frames_until_switch: switch_frame.saturating_sub(position_frames),
        }
    }

    /// 本次启动的**绝对生效帧**。
    #[must_use]
    pub const fn switch_frame(&self) -> u64 {
        self.switch_frame
    }

    /// 距生效还有多少帧（`0` ＝ 本块内就该生效）。
    #[must_use]
    pub const fn frames_until_switch(&self) -> u64 {
        self.frames_until_switch
    }

    /// 实时路径：走完 `frames` 帧 ⇒ 若已跨过生效帧就返回 `true`（**恰好一次**）。
    ///
    /// ⚠ 只做整数减法与一次比较；⛔ 不改任何别的状态。
    pub const fn advance(&mut self, frames: u64) -> bool {
        self.frames_until_switch = self.frames_until_switch.saturating_sub(frames);
        self.frames_until_switch == 0
    }
}

impl AuditionBranch {
    /// 控制路径：按采样率**预计算等功率淡化表**，并记下本次的**切换帧**。
    ///
    /// ⚠ 只许在控制路径调用（会分配）。同一个采样率下**只建一次**表。
    pub fn prepare(&mut self, sample_rate: u32, switch_frame: u64, position_frames: u64) {
        let len = crate::mixer::crossfade_len_frames(sample_rate as f32).max(1);
        if self.fade.len() != len + 1 {
            self.fade = (0..=len)
                .map(|index| crate::mixer::crossfade_gains(index as f32 / len as f32))
                .collect();
        }
        self.switch_frame = switch_frame;
        self.frames_until_switch = switch_frame.saturating_sub(position_frames);
    }

    /// 本次试听的**绝对切换帧**（`0` ＝ 还没开始过）。
    #[must_use]
    pub const fn switch_frame(&self) -> u64 {
        self.switch_frame
    }

    /// 已存下的切换倒计时（帧）。
    #[must_use]
    pub const fn frames_until_switch(&self) -> u64 {
        self.frames_until_switch
    }

    /// 第二分支真的被渲染过的块数（空闲时必须是 `0`）。
    #[must_use]
    pub const fn blocks_read(&self) -> u64 {
        self.blocks_read
    }

    /// 淡化表（`fade[k] = crossfade_gains(k / fade_len)`）；空 ＝ 还没 prepare。
    #[must_use]
    pub fn fade_table(&self) -> &[(f32, f32)] {
        &self.fade
    }

    /// 在册化第二份版本。
    ///
    /// **必须在音频线程启动前调用**（与 [`SnapshotReader::attach`] 同一条纪律：
    /// 它会取该槽的锚锁 ⇒ 不许发生在回调里）。
    #[must_use]
    pub fn attach(slot: &Arc<SnapshotSlot>, retire: RetireProducer) -> Self {
        Self {
            reader: SnapshotReader::attach(slot, retire),
            synth: Box::new(SynthEngine::new(48_000)),
            block: Box::new(AudioBlock::new()),
            armed_revision: None,
            fade: Box::default(),
            frames_until_switch: 0,
            switch_frame: 0,
            blocks_read: 0,
        }
    }

    /// 本分支当前持有的快照**修订号**（`None` ＝ 还没锚到过快照）。
    ///
    /// ⚠ `attach` 之后它**立刻**是 `Some(…)` ⇒ 判据用它证明「第二读者真的连到了
    /// **第二份**槽」（连错槽会读到另一个修订号）。
    #[must_use]
    pub fn held_revision(&self) -> Option<u64> {
        self.reader.held().map(|snapshot| snapshot.revision())
    }
}

/// **等功率混合**：`out = out·g_out + other·g_in`，逐样本（[ARCH-RT-005] 片 2c）。
///
/// ## 切换帧语义（⭐ 判据钉的就是这条）
///
/// `frames_until_switch` 是"本块内第几帧开始淡化"（0-based）。于是：
/// * 帧 `i < frames_until_switch` ⇒ 表下标 `0` ⇒ `(1, 0)` ⇒ **纯 A**；
/// * 帧 `i == frames_until_switch` ⇒ 下标 `0` ⇒ 仍然纯 A（切换帧本身还是 A）；
/// * 帧 `i == frames_until_switch + fade_len` ⇒ 下标到顶 ⇒ `(0, 1)` ⇒ **纯 B**。
///
/// ⇒ 过渡**恰好从切换帧开始**、恰好 `fade_len` 帧后结束。
///
/// ## 实时约束
///
/// 零分配、零锁、零 I/O、**零超越函数** [MUST-GATE-001]：增益全部来自
/// **控制期**算好的表（[`AuditionBranch::prepare`]），这里只有整数索引与乘加。
fn blend_equal_power(
    out: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    other: &AudioBlock<DEFAULT_BLOCK_FRAMES>,
    frames: usize,
    frames_until_switch: u64,
    fade: &[(f32, f32)],
) {
    if fade.is_empty() {
        return;
    }
    let last = fade.len() - 1;
    let frames = frames.min(out.frames()).min(other.frames());
    let other_left = other.left();
    let other_right = other.right();
    let (out_left, out_right) = out.stereo_mut();
    for index in 0..frames {
        let offset = (index as u64).saturating_sub(frames_until_switch);
        #[allow(clippy::cast_possible_truncation)]
        let table_index = (offset as usize).min(last);
        let (gain_out, gain_in) = fade[table_index];
        out_left[index] = out_left[index] * gain_out + other_left[index] * gain_in;
        out_right[index] = out_right[index] * gain_out + other_right[index] * gain_in;
    }
}

/// **一条分支的共享混音状态**（[ARCH-RT-005] 片 2a 的借用视图）。
///
/// ⚠ **为什么是字段视图而不是 `&mut self`**：混音要同时持有 `current: &EngineSnapshot`
/// （由 `self.snapshot.begin_block()` 借出）与一批 `&mut` 字段。把字段**逐个借出来**
/// （`render_block` 顶部本来就 `let Self { … } = self;` 解构过一次）⇒ 两边的借用
/// 互不相交，因此合法。本结构**只是借用视图**：不拥有状态、不分配、不引入新语义。
struct BranchShare<'a> {
    pdc: &'a mut CompensationBank,
    params: &'a mut ParamTable,
    /// ⭐ **只读**的 MIDI CC 会话运行态（任务单 `#107`）。
    ///
    /// 为什么是**共享**借用（而不是 `&'a mut`）：写它的是**事件边界**
    /// （`render_block` 的批量出队闭包），逐块渲染只**读**它（把它当
    /// `yeban-sfz` 的 `Fn(u8) -> u8` 探针喂给 SFZ 的 `ampeg_*` 求值）。
    /// ⚠ 两条分支（主线 ＋ 试听）共用**同一份**借用 ⇒ 试听期两个版本看到**同一份** CC 状态。
    control_change: &'a ControlChangeState,
    bank: &'a mut MeterBank<SCRATCH_METERS>,
    armed_strips: &'a mut [(EntityId, Option<ChannelStrip>); MAX_TRACK_SLOTS],
    armed_insert_slots: &'a mut usize,
    reverb_pool: &'a mut [Reverb; MAX_TRACK_SLOTS],
    armed_reverbs: &'a mut [(EntityId, bool); MAX_TRACK_SLOTS],
    armed_reverb_slots: &'a mut usize,
    reverb_scratch: &'a mut [[f32; DEFAULT_BLOCK_FRAMES]; 2],
    conv_pool: &'a mut Box<[ConvolutionReverb]>,
    armed_convs: &'a mut [(EntityId, bool); MAX_TRACK_SLOTS],
    armed_conv_slots: &'a mut usize,
    conv_scratch: &'a mut [f32; 2 * DEFAULT_BLOCK_FRAMES],
    armed_pan_l: &'a mut [ParamSmoother; MAX_TRACK_SLOTS],
    armed_pan_r: &'a mut [ParamSmoother; MAX_TRACK_SLOTS],
    armed_pan_slots: &'a mut usize,
    armed_pan_armed: &'a mut [bool; MAX_TRACK_SLOTS],
    track_scratch: &'a mut [f32; DEFAULT_BLOCK_FRAMES],
    /// ⭐ **总线暂存池**（任务单 `#154`）：非 master 节点的立体声累积缓冲。
    ///
    /// 与 `track_scratch` 同一条纪律：**构造期**预分配、逐块复用 ⇒ 零分配。
    /// 只有 `RoutingPlan::uses_buses()` 为真时才清空/使用（默认路径整段跳过）。
    routing_buses: &'a mut [AudioBlock<DEFAULT_BLOCK_FRAMES>; MAX_TRACK_SLOTS],
    scratch_meters: &'a mut [MeterFrame; SCRATCH_METERS],
    insert_gain_reductions: &'a mut u64,
    insert_max_reduction_db: &'a mut f32,
    insert_current_reduction_db: &'a mut f32,
    insert_detector_level_db: &'a mut Option<f32>,
    insert_strip_frames: &'a mut u64,
    insert_reverb_frames: &'a mut u64,
    insert_convolution_frames: &'a mut u64,
    pan_automation_frames: &'a mut u64,
    meter_capacity_drops: &'a mut u64,
    pan_gains: &'a mut [(EntityId, f32, f32); MAX_TRACK_SLOTS],
}

/// 把 `share.track_scratch[..frames]` 里的**单声道**缓冲送过本轨已武装的插入链：
/// **通道条 → 混响 → 卷积混响**（`crate::insert` 模块文档 §2／§8.3／§9.4）。
///
/// 返回值：本轨的**通道条**是否真的处理过帧（用于"本量子有没有器件工作过"的量规）。
///
/// ## 两个调用点（任务单 `#158` 起）
///
/// | 调用点 | 输入 | 何时 |
/// | :--- | :--- | :--- |
/// | 3a'（逐轨循环） | 该轨自己的渲染结果 | **普通轨**（与 `#158` 之前逐字相同） |
/// | 3a⁗（总线转发） | 总线暂存池里**下混**后的累积信号 | **处理型总线**（有器件 ＋ 有入边） |
///
/// ⚠ 两个调用点**互斥**：处理型总线在 3a' 整段跳过。理由在 [`crate::rt`] 模块文档
/// 的"每量子每节点恰好一次"一节：器件的延迟线/弹道是**时间状态**，一个量子跑两遍
/// 会让混响尾巴以两倍速衰减。
///
/// ## 实时约束
///
/// 逐样本只有器件的乘加与一阶递归（EQ 的中间缓冲是**栈数组**）；器件把 `exp`
/// 全部留在 `set_params` 里 ⇒ 这里**零分配、零锁、零 I/O、零日志** [MUST-GATE-001]。
/// 三个 `if let` 分支在本轨没有对应器件时**整段跳过**（不是"参数取成透明"）。
fn process_insert_chain(
    share: &mut BranchShare<'_>,
    track: EntityId,
    frames: usize,
    current_db: &mut f32,
    detector_db: &mut f32,
) -> bool {
    let frames = frames.min(DEFAULT_BLOCK_FRAMES);
    let mut seen = false;

    // --- ① 通道条（EQ／滤波／动态）：逐样本 ---
    //
    // 位置：**声源与参数乘子之后、电平与 PDC 之前**。判据见
    // `tests/channel_strip_insert.rs` 的 C0/C10。
    //
    // 停住时喂的是静音：各级状态是**时间状态**（滤波器的积分器、压缩器的增益
    // 弹道），喂静音让它们按各自的时间常数回到初值 —— 与 PDC/"停住也喂静音"同一条纪律。
    if let Some(index) = share.armed_strips[..*share.armed_insert_slots]
        .iter()
        .position(|(id, _)| id == &track)
        && let (_, Some(strip)) = &mut share.armed_strips[index]
    {
        let before_reductions = strip.gain_reduction_count();
        let before_frames = strip.processed_frames();
        strip.process_mono(&mut share.track_scratch[..frames]);
        let reduced = strip
            .gain_reduction_count()
            .saturating_sub(before_reductions);
        *share.insert_gain_reductions = share.insert_gain_reductions.wrapping_add(reduced);
        let processed = strip.processed_frames().saturating_sub(before_frames);
        *share.insert_strip_frames = share.insert_strip_frames.wrapping_add(processed);
        let reduction_db = strip.max_gain_reduction_db();
        if reduction_db > *share.insert_max_reduction_db {
            *share.insert_max_reduction_db = reduction_db;
        }
        // **当前**衰减（量规）：与上面两条计数**同一台器件、同一个瞬间**读出。
        // 它把线性增益按需换算成 dB（`−compressor.gain_db()` ⇒ 一次 `log2`），
        // 但不碰样本、不分配、不加锁（红线 7 不受影响）。
        let measured_current_db = strip.current_gain_reduction_db();
        if measured_current_db > *current_db {
            *current_db = measured_current_db;
        }
        // **检波器**电平（量规，单位 dBFS）：同一个器件、同一个瞬间的**输入**侧。
        // 它同样只读一个字段（`compressor.level_db`，一个 `const fn`）
        // ⇒ 不碰样本、不分配、不加锁 ⇒ 渲染输出逐位不变。
        let measured_detector_db = strip.detector_level_db();
        if measured_detector_db > *detector_db {
            *detector_db = measured_detector_db;
        }
        seen = true;
    }

    // --- ② 混响级：逐样本（器件的接口是立体声块）---
    //
    // 单声道口径：把 `share.track_scratch` 同时喂给左右两路，取两路输出的中值
    // `(out_l + out_r) · 0.5`。那是恒等式而不是近似（`crate::insert` 模块文档
    // §8.3 给了代数证明）：湿路径的输入与"单声道输入"同解，宽度项在中值里恰好抵消。
    //
    // **默认口径**：本轨没有混响（`armed_reverbs` 里查不到武装项）⇒ 整段跳过；
    // `Reverb::is_active()`（`mix ≤ 1e-4`）为假时同样跳过 ⇒ 这两种情形下器件的状态
    // **不推进**（与器件自己的早返回守卫同口径）。
    if let Some(index) = share.armed_reverbs[..*share.armed_reverb_slots]
        .iter()
        .position(|(id, armed)| *armed && id == &track)
    {
        let reverb = &mut share.reverb_pool[index];
        if reverb.is_active() {
            share.reverb_scratch[0][..frames].copy_from_slice(&share.track_scratch[..frames]);
            share.reverb_scratch[1][..frames].copy_from_slice(&share.track_scratch[..frames]);
            let (left, right) = share.reverb_scratch.split_at_mut(1);
            reverb.process(&mut left[0][..frames], &mut right[0][..frames]);
            for frame in 0..frames {
                share.track_scratch[frame] =
                    (share.reverb_scratch[0][frame] + share.reverb_scratch[1][frame]) * 0.5;
            }
            *share.insert_reverb_frames = share.insert_reverb_frames.wrapping_add(frames as u64);
        }
    }

    // --- ③ 卷积混响级：逐样本（器件的接口是交错立体声块）---
    //
    // 单声道口径（与混响同款）：把 `share.track_scratch` **同时**写进左右两路，
    // 取两路输出的中值 `(out_l + out_r) · 0.5`。那是恒等式而不是近似：
    // 四条通路线性，中值 = `conv((h_LL + h_LR + h_RL + h_RR) / 2, m)`，
    // 而投影交进去的是 `h_LL = h_RR = ir`、`h_LR = h_RL = 0` ⇒ 恰好是
    // `conv(ir, m)`（§9.4 给了这段代数）。
    //
    // ⚠ IR 本身**不在**这条路径上：它只在快照边界换（且长度不变 ⇒ 零分配）。
    //
    // **默认口径**：本轨没有卷积混响（`armed_convs` 里查不到武装项）⇒ 整段跳过；
    // `ConvolutionReverb::is_active()`（`wet ≤ 1e-4` 或未配置）为假时同样跳过。
    //
    // ⚠ 它是 [ADR-0001 D32] 的**超越函数类**（旋转因子表由宿主 libm 的
    // `f32::cos`/`f32::sin` 在构造期算出）⇒ 本票不声称跨架构逐位相同。
    if let Some(index) = share.armed_convs[..*share.armed_conv_slots]
        .iter()
        .position(|(id, armed)| *armed && id == &track)
    {
        let conv = &mut share.conv_pool[index];
        if conv.is_active() {
            for frame in 0..frames {
                let mono = share.track_scratch[frame];
                share.conv_scratch[2 * frame] = mono;
                share.conv_scratch[2 * frame + 1] = mono;
            }
            conv.process(&mut share.conv_scratch[..2 * frames]);
            for frame in 0..frames {
                share.track_scratch[frame] =
                    (share.conv_scratch[2 * frame] + share.conv_scratch[2 * frame + 1]) * 0.5;
            }
            *share.insert_convolution_frames =
                share.insert_convolution_frames.wrapping_add(frames as u64);
        }
    }

    seen
}

/// **渲染一条分支**的逐轨材料到 `out`（逐轨合成 → 电平 → PDC → 声相 → 汇入母线）。
///
/// [ARCH-RT-005] 片 2a：本函数是**纯抽取** —— 函数体逐字等于抽取前 `render_block` 里
/// 的那一段，因此对同一条快照的输出**逐字节相同**（判据钉住这一点）。
///
/// 返回本量子产出的**逐轨电平帧数**（`produced`）。
///
/// ## 为什么边界停在"汇入母线"，⛔ 不含节拍器 / 主总线推子 / `apply_master`
///
/// 那三样是**母线级**信号：它们属于"这条总线"，⛔ 不属于 A/B 的任一份材料。
/// 片 2b 会在**混合之后**再走那一段 ⇒ 节拍器**不会**被淡化两次
/// （等功率淡化把同一个信号加两次会把它抬到 **+3 dB**）。
///
/// ## 实时约束
///
/// 零分配、零锁、零 I/O、零日志（[MUST-GATE-001] / 红线 7）：借用的字段全部
/// **构造期预分配**，本函数只读写它们。
///
/// ## ⚠ 试听期的**电平读数主语**（裁决 **(甲)** 下的**已知边界**）
///
/// 本函数每被调用一次就 `bank.begin_quantum()` 一次。试听期它被**调用两次**
/// （先主线 A、后分支 B）⇒ **本量子已发布的电平读数，主语是"后渲染那条"，即 B**。
///
/// ⛔ 这**不是输出正确性问题**：音频输出另有逐字节判据守着（见
/// `extracted_mix_path_matches_the_legacy_inline_path_bit_for_bit` 与集成场景 ㉗）。
/// 它是一条**读数口径**的边界。
///
/// 口径**改**的话只有两条路，且都**不在本片范围**：
/// * **(乙)** 把 `begin_quantum()` 提到两条分支**之前** ⇒ 读数归主线 A；
///   代价是会动 2a 那条**逐位等价参照判据**；
/// * **(丙)** 改读**混合之后**的母线 ⇒ 最贴听感，但 `bank` 是**按轨**的，
///   需要一套**尚未规定**的新口径。
///
/// 现状由判据 `audition_meter_reading_belongs_to_the_last_branch_rendered` 钉住
/// （⛔ 防静默漂移）。
///
/// ## 任务单 `#135`：`block_start_frame`（音频片段的**绝对帧原点**）
///
/// 音频摆放的区间是**绝对帧**（快照在控制线程按 tick → 帧算好），因此逐轨音频混合
/// 需要本量子第一帧的绝对位置。这个值由调用方从 [`crate::transport::Transport`]
/// **推进之前**读出（走带是唯一的时钟事实源），本函数只读它、⛔ 不推进它。
///
/// ⚠ **默认口径逐位不变**：工程没有音频摆放（`current.audio_clip_schedule()` 全空）
/// 或摆放没有注入 PCM 时，混合调用**整段跳过** ⇒ 本参数对输出零影响
/// （这也是 `extracted_mix_path_matches_the_legacy_inline_path_bit_for_bit` 仍然成立的
/// 理由：那份夹具没有音频摆放）。
#[allow(clippy::too_many_arguments)]
fn render_branch(
    share: &mut BranchShare<'_>,
    synth: &mut SynthEngine,
    out: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    current: &EngineSnapshot,
    frames: usize,
    running: bool,
    quantum: u64,
    block_start_frame: u64,
) -> usize {
    let master = current.master();
    let mut produced = 0usize;
    out.silence();
    out.set_frames(frames);
    share.bank.begin_quantum();
    // ⭐ 任务单 `#154`：本快照的路由计划（构造期编译好的纯数据）。
    //
    // ⚠ **默认口径**：`uses_buses()` 为假（所有节点都只直连 master）⇒ 下面的
    // 逐轨循环走**接线前那条求和路径**（一个字节都不改）；总线池既不清空也不遍历。
    let routing = current.routing();
    let buses_needed = routing.uses_buses();
    if buses_needed {
        for bus in share.routing_buses[..routing.len()].iter_mut() {
            bus.silence();
            bus.set_frames(frames);
        }
    }

    // --- 3a) 逐轨：渲染 → 电平 → 汇入母线 ---
    //
    // **走带冻结 ⇒ 输出静音、且不触发任何音符**（`notes` 契约见
    // `docs/ledger/engine-sound-notes.md` 的 needs N2）。电平仍然照常计量
    // （静音的读数），因此"每量子恰好一次批量发布"这条结构性契约与走带状态无关。
    let metered_tracks = current.tracks().keys().filter(|id| **id != master).count();
    // 给母线留一个槽位, 保证母线永远有电平可发。
    let track_budget = share.scratch_meters.len().saturating_sub(1);
    if metered_tracks > track_budget {
        *share.meter_capacity_drops = share
            .meter_capacity_drops
            .wrapping_add((metered_tracks - track_budget) as u64);
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
    // "没有快照的量子不更新"同口径（见 [`EngineStats::share.insert_current_reduction_db`]
    // 与 [`EngineStats::share.insert_detector_level_db`] 的口径第 3／4 条）。
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
    // ⭐ **CC 探针**（任务单 `#107`）：把会话运行态表借成 `yeban-sfz` 认识的
    // `Fn(u8) -> u8`（零捕获大小的栈上闭包）⇒ 逐块路径把它交给 SFZ 的 `ampeg_*` 求值。
    //
    // ⚠ **`None` 的条件是"整张表从未收到过 CC"**（[`ControlChangeState::is_armed`]）：
    // 没有 CC 事件的工程因此与接线前**逐位相同**（那条"无 CC ⇒ 逐位不变"的判据
    // 不必依赖"探针返回恰好等于基准值"这种巧合）。
    // ⚠ 借用：`share.control_change` 是 `&'a ControlChangeState`（共享引用，`Copy`）
    // ⇒ 先把**它本身**取出来，逐轨循环里就只借一个**外部**的只读引用，
    // ⛔ 不与 `share` 的 `&mut` 字段冲突。
    let cc_state: &ControlChangeState = share.control_change;
    let cc_probe = |controller: u8| cc_state.value(controller);
    let cc: Option<&dyn Fn(u8) -> u8> = if cc_state.is_armed() {
        Some(&cc_probe)
    } else {
        None
    };
    for id in current.tracks().keys() {
        if *id == master || produced >= track_budget {
            continue;
        }
        let track = *id;
        // ⭐ 任务单 `#158`：本轨是不是**处理型总线**（有已武装的插入器件 ＋ **真的收到**
        // 别的节点搬来的音频）。只有它把插入链搬到 3a⁗ 的总线转发阶段执行**一次**。
        //
        // 为什么三个条件缺一不可：
        // * `buses_needed` 为假（默认工程：所有节点直连 master）⇒ **短路**，⛔ 不做
        //   下面三次武装表扫描 ⇒ 默认路径一分钱都不多花；
        // * 没有已武装器件（无设备／全旁通／参数名都不认识）⇒ 插入链没有活要干，
        //   逐轨路径与 `#154` **逐位相同**（快路）；
        // * 没有入边 ⇒ 没有"发送"可处理：链继续在逐轨阶段按本轨自己的声源跑
        //   （接线前的行为），转发阶段不重复跑。
        //
        // ⚠ 三个扫描都落在**定长**切片上（`[..0]` 在默认工程里恒为空）：
        // 零分配、零锁、零 I/O [MUST-GATE-001]。
        let process_bus_inserts = buses_needed
            && routing.has_incoming_audio(&track)
            && (share.armed_strips[..*share.armed_insert_slots]
                .iter()
                .any(|(id, strip)| id == &track && strip.is_some())
                || share.armed_reverbs[..*share.armed_reverb_slots]
                    .iter()
                    .any(|(id, armed)| *armed && id == &track)
                || share.armed_convs[..*share.armed_conv_slots]
                    .iter()
                    .any(|(id, armed)| *armed && id == &track));
        if running {
            // 真的合成：快照里的音符调度表 → 声部池 → 单声道、声相之前的样本。
            // 参数/音符的投影全部在控制线程完成；这里只有整数相位递推、
            // 线性插值与 ADSR（全是 IEEE 精确类运算，见 `synth` 模块文档 §2）。
            synth.render_track_with_cc(
                track,
                current.schedule(&track),
                // ⭐ SFZ 音源（任务 `#66` 的步 2）：**借用快照里那一份摊平表**
                // （⛔ 不 clone 里面的 PCM `Vec`）。不在表里 ⇒ `None` ⇒ 逐位不变。
                current.sfz_plan(&track).map(|plan| plan.prepared()),
                &mut share.track_scratch[..frames],
                // ⭐ CC 探针（任务单 `#107`）：`None` ⇒ 整条 SFZ 包络路径与接线前逐位相同。
                cc,
            );
            // --- 3a⁰⁰) ⭐ 音频片段累加（任务单 `#135`）---
            // 位置：**合成器之后、参数乘子与插入链之前** —— 与离线
            // `render_core::TrackSource` 的口径一致（MIDI 基座先写、音频摆放
            // 按 `(start, end, gain 位型)` 全序在其上**累加**）。
            //
            // ⚠ **为什么不需要逐样本游标**：span 的区间是**绝对帧**，素材帧由
            // `block_start_frame − span.start` **导出**（走带是唯一时钟）⇒ 定位 /
            // 暂停恢复 / 换快照都自动正确，`EngineRuntime` ⛔ 一个字节都不增长。
            //
            // **默认口径**：本轨没有音频摆放（不在表里）⇒ 整段跳过 ⇒ 输出与接线前
            // **逐位相同**；有摆放但没注入 PCM ⇒ 逐 span 跳过，同样逐位相同。
            //
            // 逐样本只有整数比较 + 切片下标 + 一次乘加：**零分配、零释放、零锁、
            // 零 I/O、零超越函数** [MUST-GATE-001]（判据：`tests/rt_zero_alloc.rs` 的
            // 音频片段场景 ＋ `tests/audio_clip_render.rs`）。
            if let Some(clips) = current.audio_clip_schedule(&track) {
                crate::audio_clip::mix_into(
                    clips,
                    block_start_frame,
                    &mut share.track_scratch[..frames],
                );
            }
        } else {
            // 停住：不碰声部池（不触发、不推进、不窃取），只把静音喂给电平表。
            share.track_scratch[..frames].fill(0.0);
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
        share
            .params
            .apply(track, &mut share.track_scratch[..frames]);
        // --- 3a'~3a'''') 插入链：通道条 → 混响 → 卷积混响，**逐样本** ---
        // [crate::insert]。位置：**轨道自己的渲染与参数乘子之后、逐轨电平与 PDC 之前**
        // ⇒ 普通轨的电平读数与汇入母线的信号都是"插入之后"的结果。
        //
        // **默认口径**：本轨没有插入器件（三张武装表里都查不到）⇒ 助手里三个
        // `if let` 逐段跳过（不是"参数取成透明"）⇒ 这类轨的输出与接线前**逐位相同**
        // （证据见 `tests/compressor_insert.rs` 的 C0）。
        //
        // ⭐ **任务单 `#158`**：`process_bus_inserts` 为真时**这里整段跳过** —— 这一轨是
        // "处理型总线"（有已武装器件 ＋ 有入边），它的链在 3a⁗ 的总线转发阶段对
        // "发送之和 ＋ 自己的声源"跑**一次**（见 [`process_insert_chain`] 与模块头的
        // "每量子每节点恰好一次"）。否则器件的延迟线/弹道会在一个量子里推进两倍。
        if !process_bus_inserts
            && process_insert_chain(
                share,
                track,
                frames,
                &mut insert_current_db,
                &mut insert_detector_db,
            )
        {
            insert_current_seen = true;
        }
        // ⭐ 任务单 `#158`：**处理型总线**的电平帧改在 3a⁗ **插入之后**计量
        // （读"发送之和 ＋ 自己的声源"，仍是单声道、仍是"插入之后"的口径）；
        // 这里跳过 ⇒ 每量子每节点仍然**恰好一次**（`MeterBank::measure` 的契约）。
        if !process_bus_inserts {
            if let Some(frame) = share
                .bank
                .measure(track, quantum, &share.track_scratch[..frames])
            {
                share.scratch_meters[produced] = frame;
                produced += 1;
            } else {
                *share.meter_capacity_drops = share.meter_capacity_drops.wrapping_add(1);
                rt_probe::note_suppressed(RtDiagEvent::MeterCapacityDrop);
            }
        }
        // --- PDC：本轨输出 → 补偿延迟线 → 声相/母线求和 ---
        // [ARCH-PDC-001, ROAD-M2-004] 位置就是规范 §3.4 第 3 条说的
        // "在进入总线求和节点前"。延迟量在快照边界武装好（见 2b''），
        // 这里只做环形读写：**零分配、逐样本无分支**（`delay == 0` 时输出
        // 逐位等于输入，但延迟线**仍然记录** —— 见 `graph::DelayLine` 的
        // "环的不变量"）。
        //
        // ⚠ 刻意放在 `share.bank.measure` **之后**：逐轨电平的取样点因此**一位没动**
        // （仍是"该轨自己渲染出来的、声相之前的单声道结果"），PDC 是**求和节点
        // 输入侧**的事 —— 这正是规范那句话的位置。放到电平之前也能对齐相位，
        // 但那样会顺带改掉电平的读数口径。
        //
        // 走带停住时**也**喂延迟线（喂的是静音）：延迟线是时间状态，
        // 不推进它会让恢复播放时吐出上一次停住前的陈音频。
        share.pdc.apply(&track, &mut share.track_scratch[..frames]);
        // 汇入立体声母线：按本轨的**声相增益**分别写 L/R
        // （构造期算好的 `cos/sin`，见 `mixer` 模块文档 §1）。
        // 找不到该轨的增益（超出 `MAX_TRACK_SLOTS`）时按**居中**处理，
        // 而不是静音 —— 宁可声相不准，也不要一条轨无声。
        //
        // **声相自动化**（裁决 P4=(b)）只多一个分支：该槽位收到过被接受的
        // 声相目标 ⇒ 走**平滑路径**（逐样本把两个增益收敛到目标，零分配）；
        // 否则走**原来那条**常量路径 ⇒ 没有声相自动化的工程**逐位不变**。
        let pan_index = share.pan_gains[..*share.armed_pan_slots]
            .iter()
            .position(|(id, _, _)| *id == track);
        // ⭐ 本轨的去向（任务单 `#154`）。`routes` 在构造期编译好：
        //   * 有音频出边 ⇒ 每条边一项（含增益）；
        //   * 没有音频出边 ⇒ 一项"直连 master、单位增益"（接线前的行为）。
        let routes = routing.routes(&track);
        // ⚠ **逐位不变的快路**：只有一条"直连 master、单位增益"的出边时，走**原来那条**
        // 求和路径（常量/平滑两个分支逐字未动）⇒ 既有工程与三条承重指纹都不受影响。
        //
        // ⚠ 计划里**没有**这条轨（第 17 条之后的轨：总线池与声部池的上限都是
        // [`MAX_TRACK_SLOTS`]）时按**接线前的行为**直连 master —— 宁可忽略它那些
        // 图边，也不让一条轨在混音里凭空消失（"容量不足不静默"的读数在声部池那侧）。
        let single_unit_to_master = routes.is_empty()
            || (routes.len() == 1 && routes[0].destination() == master && routes[0].gain() == 1.0);
        if process_bus_inserts {
            // ⭐ 任务单 `#158`：**处理型总线**的自身声源先以"声相之后"的立体声并入
            // **自己的**总线暂存槽（单位增益）。出边的增益**不在这里乘**：等 3a⁗
            // 把"发送之和 ＋ 这份声源"一起送过插入链之后，再按每条出边乘一次。
            //
            // 位置：PDC 之后（与普通轨同一条纪律）、插入链**之前** —— 因为链必须在
            // **求和之后**跑（与离线渲染的插入点同口径），而且每量子只跑一次。
            if let Some(index) = pan_index.filter(|index| share.armed_pan_armed[*index]) {
                // 声相自动化：平滑器**每帧只推进一次**，与普通轨的暂存路径同款。
                {
                    let (first, second) = share.reverb_scratch.split_at_mut(1);
                    render_panned_into_smoothed(
                        &mut first[0][..frames],
                        &mut second[0][..frames],
                        &share.track_scratch[..frames],
                        &mut share.armed_pan_l[index],
                        &mut share.armed_pan_r[index],
                    );
                }
                *share.pan_automation_frames =
                    share.pan_automation_frames.wrapping_add(frames as u64);
            } else {
                let (gain_l, gain_r) = pan_index.map_or(
                    (
                        core::f32::consts::FRAC_1_SQRT_2,
                        core::f32::consts::FRAC_1_SQRT_2,
                    ),
                    |index| (share.pan_gains[index].1, share.pan_gains[index].2),
                );
                {
                    let (first, second) = share.reverb_scratch.split_at_mut(1);
                    render_panned_into(
                        &mut first[0][..frames],
                        &mut second[0][..frames],
                        &share.track_scratch[..frames],
                        gain_l,
                        gain_r,
                    );
                }
            }
            // `has_incoming_audio(&track)` 为真 ⇒ 它一定在总线池里；这里的 `if let`
            // 只是把"容量边界"写成显式跳过（宁可少一条总线的自声源，不 panic）。
            if let Some(bus_index) = routing.slot_of(&track) {
                sum_stereo_into_bus(
                    &mut share.routing_buses[bus_index],
                    &share.reverb_scratch[0][..frames],
                    &share.reverb_scratch[1][..frames],
                    frames,
                );
            }
        } else if let Some(index) = pan_index.filter(|index| share.armed_pan_armed[*index]) {
            if single_unit_to_master {
                sum_into_bus_smoothed(
                    out,
                    &share.track_scratch[..frames],
                    &mut share.armed_pan_l[index],
                    &mut share.armed_pan_r[index],
                );
            } else {
                // 声相自动化 + 非平凡路由：先把"声相之后"的立体声写进暂存
                // （平滑器**每帧只推进一次**），再按边分别搬运。
                {
                    let (first, second) = share.reverb_scratch.split_at_mut(1);
                    render_panned_into_smoothed(
                        &mut first[0][..frames],
                        &mut second[0][..frames],
                        &share.track_scratch[..frames],
                        &mut share.armed_pan_l[index],
                        &mut share.armed_pan_r[index],
                    );
                }
                route_panned(
                    routes,
                    routing,
                    master,
                    &share.reverb_scratch[0][..frames],
                    &share.reverb_scratch[1][..frames],
                    out,
                    share.routing_buses,
                );
            }
            *share.pan_automation_frames = share.pan_automation_frames.wrapping_add(frames as u64);
        } else {
            let (gain_l, gain_r) = pan_index.map_or(
                (
                    core::f32::consts::FRAC_1_SQRT_2,
                    core::f32::consts::FRAC_1_SQRT_2,
                ),
                |index| (share.pan_gains[index].1, share.pan_gains[index].2),
            );
            if single_unit_to_master {
                sum_into_bus(out, &share.track_scratch[..frames], gain_l, gain_r);
            } else {
                {
                    let (first, second) = share.reverb_scratch.split_at_mut(1);
                    render_panned_into(
                        &mut first[0][..frames],
                        &mut second[0][..frames],
                        &share.track_scratch[..frames],
                        gain_l,
                        gain_r,
                    );
                }
                route_panned(
                    routes,
                    routing,
                    master,
                    &share.reverb_scratch[0][..frames],
                    &share.reverb_scratch[1][..frames],
                    out,
                    share.routing_buses,
                );
            }
        }
    }

    // --- ⭐ 3a⁗) 总线转发：按**拓扑序**把每条总线的累积信号送到它的目的地 ------
    //
    // 为什么在逐轨循环**之后**：一条总线要等**所有**上游都送完才能转发（它的输入是
    // 累积和）；拓扑序保证上游总线先于下游总线处理 ⇒ 多级总线（aux → aux → master）
    // 一次遍历即可。
    //
    // 为什么先拷贝到 `reverb_scratch`：转发目标可能是**另一条总线**（`buses` 的可变
    // 借用互相冲突）⇒ 先读进暂存再写目标，零分配。
    //
    // ⭐ **任务单 `#158`**：转发之前，若这条总线是**处理型总线**（有已武装的插入器件
    // ＋ 有入边），先把累积信号**下混**成单声道送过插入链、再复制回两路。逐轨循环
    // 已为它**跳过**了同一条链 ⇒ 每量子恰好跑一次（裁决与代价见模块头）。
    // 下混的输入是"发送之和 ＋ 它自己的声源"（后者由逐轨循环并入本槽）。
    //
    // ⚠ **总线 PDC 仍未接**：本片只补插入链；边上的补偿延迟线与"总线级 PDC"是
    // 另一个切片（与 `#154` 登记的边界一致）。
    if buses_needed {
        let slots = routing.slots();
        for (index, node) in slots.iter().enumerate() {
            let routes = routing.routes(node);
            if routes.is_empty() {
                continue;
            }
            // ⭐ 与逐轨循环里的 `process_bus_inserts` 是**同一个谓词**（同一组武装表、
            // 同一个 `has_incoming_audio`）⇒ 两处不可能一处跑、一处不跑。
            let processing_bus = routing.has_incoming_audio(node)
                && (share.armed_strips[..*share.armed_insert_slots]
                    .iter()
                    .any(|(id, strip)| id == node && strip.is_some())
                    || share.armed_reverbs[..*share.armed_reverb_slots]
                        .iter()
                        .any(|(id, armed)| *armed && id == node)
                    || share.armed_convs[..*share.armed_conv_slots]
                        .iter()
                        .any(|(id, armed)| *armed && id == node));
            if processing_bus {
                // ① 下混（`(L + R) · 0.5`，裁决 **(甲)**）：先写进 `track_scratch`。
                {
                    let (bus_left, bus_right) = share.routing_buses[index].stereo_mut();
                    for frame in 0..frames {
                        share.track_scratch[frame] = (bus_left[frame] + bus_right[frame]) * 0.5;
                    }
                }
                // ② 插入链**一次**（与逐轨阶段同一个助手、同一组计数/量规）。
                if process_insert_chain(
                    share,
                    *node,
                    frames,
                    &mut insert_current_db,
                    &mut insert_detector_db,
                ) {
                    insert_current_seen = true;
                }
                // ③ 复制回两路（单声道 → L/R 同值）：下混的逆操作**不是**重建立体声，
                // 而是把处理后的中值放到两路 —— 这就是"单声道插入"的全部口径。
                {
                    let (bus_left, bus_right) = share.routing_buses[index].stereo_mut();
                    bus_left[..frames].copy_from_slice(&share.track_scratch[..frames]);
                    bus_right[..frames].copy_from_slice(&share.track_scratch[..frames]);
                }
                // ④ 电平帧（**插入之后**；逐轨循环已为它跳过 ⇒ 每量子恰好一次）。
                //
                // ⚠ `produced >= track_budget` 时**不重复记丢**：容量不足已经在逐轨
                // 循环之前按 `metered_tracks - track_budget` 一次性记过（见本函数开头
                // 的 `meter_capacity_drops` 段）⇒ 这里只是不发布这一帧。
                if produced < track_budget {
                    if let Some(frame) =
                        share
                            .bank
                            .measure(*node, quantum, &share.track_scratch[..frames])
                    {
                        share.scratch_meters[produced] = frame;
                        produced += 1;
                    } else {
                        *share.meter_capacity_drops = share.meter_capacity_drops.wrapping_add(1);
                        rt_probe::note_suppressed(RtDiagEvent::MeterCapacityDrop);
                    }
                }
            }
            {
                let (first, second) = share.reverb_scratch.split_at_mut(1);
                let (bus_left, bus_right) = share.routing_buses[index].stereo_mut();
                first[0][..frames].copy_from_slice(bus_left);
                second[0][..frames].copy_from_slice(bus_right);
            }
            route_panned(
                routes,
                routing,
                master,
                &share.reverb_scratch[0][..frames],
                &share.reverb_scratch[1][..frames],
                out,
                share.routing_buses,
            );
        }
    }

    // --- 插入链动态级的**当前**衰减与**检波器**电平读数（3a' 的收尾）：
    //     逐轨循环之后覆写**一次** ---
    //
    // 读数的搬运（不是计算）：本量子至少有一台武装通道条处理过帧时，把
    // `insert_current_db` 与 `insert_detector_db`（两者都是跨轨最大）覆写进统计。
    // 放在循环**之后**而不是循环**之内**有两个理由：
    //
    // 1. 一个量子只有**一个**读数（与 `share.insert_max_reduction_db` 的跨轨口径相同）。
    //    循环内每轨各写一次会让读数的终值取决于"最后一条轨是谁"（`BTreeMap`
    //    的键序），那是把实现细节当成语义；
    // 2. 它们是**量规**：一台都不工作时**不覆写**（保持上一次的值），
    //    见 [`EngineStats::share.insert_current_reduction_db`] 的口径第 3 条与
    //    [`EngineStats::share.insert_detector_level_db`] 的口径第 4 条。
    //
    // 零分配、零锁、零 I/O、零日志：一次 `f32` 字段写与一次 `Option<f32>` 字段写。
    if insert_current_seen {
        *share.insert_current_reduction_db = insert_current_db;
        // `insert_detector_db` 此时必定已被至少一台器件赋值过（`seen` 的定义）
        // ⇒ `Some(…)` 里不是 `NEG_INFINITY`。
        *share.insert_detector_level_db = Some(insert_detector_db);
    }
    produced
}

/// 渲染驱动：音频回调持有的全部可变状态。
pub struct EngineRuntime {
    snapshot: SnapshotReader,
    /// A/B 盲听的第二份可渲染版本（[ARCH-RT-005] 片 1：**持有**，⛔ 不参与渲染）。
    ///
    /// 未挂上时是 `None` ⇒ 渲染路径与「没有这个字段」**逐字节相同**。
    audition: Option<AuditionBranch>,
    /// 试听是否**已开始**（[ARCH-RT-005] 片 2b 的受限版）。
    ///
    /// ⚠ 片 2b 的这一步只做**准入判定**：拓扑不一致时拒绝（见
    /// [`AuditionRefusal`]）。⛔ 渲染路径**一个字没改** ⇒ 未试听时输出
    /// 仍逐字节不变、RT 成本仍为 0。
    audition_active: bool,
    /// ⭐ **排队等待量化点**的场景启动（`None` ＝ 没有排队的启动）。
    ///
    /// ⚠ `None` 时实时路径只多一次 `Option` 判别、⛔ 一个样本都不变 ⇒ 未排队时与
    /// 「没有这个字段」逐字节相同。
    pending_launch: Option<PendingSceneLaunch>,
    /// ⭐ 累计**真的生效**过的场景启动次数（缺席见证 ⇒ `==` 计数没有牙齿）。
    scene_launches_applied: u64,
    events: EventReceiver,
    /// ⭐ **MIDI CC 的会话运行态**（任务单 `#107`）。
    ///
    /// [`EngineEvent::ControlChange`] 的落点：事件在**量子边界**的批量出队里写它
    /// （[`Self::render_block`] 的 1) 段），逐块路径只在**旁路**读它
    /// （`render_branch` 把它的只读探针交给 [`SynthEngine::render_track_with_cc`]）。
    ///
    /// ⭐ ⭐ **追加修订段（任务单 `#110`）—— 被修订的是上一段的"落点"那一支**
    /// （⛔ 原文一字未删，口径同台账第 278 条）：[`EngineEvent::ControlChange`]
    /// **不再**是唯一的写入口 —— 快照边界的**重新武装**分支
    /// （[`Self::render_block`] 的 `2a-cc` 段）会按摊平表自带的
    /// [`crate::sfz_instrument::CcSeedTable`] 调 [`ControlChangeState::seed`]
    /// （`<control> set_ccN=` 的加载时初值）。⚠ 两条写入的**关系**：事件面**先**跑、
    /// 播种**只填从未收到过的槽** ⇒ 真 MIDI CC 永远赢；⚠ **表为空**（文件没有
    /// `set_ccN`）⇒ 播种**一次都不写** ⇒ 本字段的 `armed` 口径与读数**整条有效**。
    ///
    /// * **定长**：`[u8; 128]` ＋ 128 位"收到过"位图 ＝ `144 B`（[`crate::cc_state`]）
    ///   ⇒ ⛔ 无 `Vec`／⛔ 无 `HashMap`／⛔ 无 `Box`、无 `Drop`、无分配；
    /// * ⛔ **不进快照**：它是挥发性会话运行态（与 [`crate::transport::Transport`] 的
    ///   播放头同一层），[MODEL-ISO-001] 禁止把它塞进不可变模型投影；
    /// * **成本**：本字段把 `Self` 从 `180144 - 144 = 180000 B` 抬到 `180144 B`
    ///   （门槛 192 KiB = `196608` ⇒ 余量 `16464 B`，判据见 `tests/rt_new_boxed.rs` 与
    ///   `tests/sfz_instrument.rs` 的 S9）。
    control_change: ControlChangeState,
    meters: MeterPublisher,
    block: AudioBlock<DEFAULT_BLOCK_FRAMES>,
    scratch_events: [EngineEvent; SCRATCH_EVENTS],
    scratch_meters: [MeterFrame; SCRATCH_METERS],
    /// 单轨渲染结果（声相之前、单声道）。复用同一个缓冲，避免每轨一份。
    track_scratch: [f32; DEFAULT_BLOCK_FRAMES],
    /// ⭐ **总线暂存池**（任务单 `#154`）：最多 [`MAX_TRACK_SLOTS`] 条非 master 节点的
    /// **立体声累积缓冲**。
    ///
    /// 为什么是堆上的定长数组（`Box<[_; 16]>`，值尺寸 = **一个瘦指针 8 B**）：
    /// 内联 `[AudioBlock; 16]` 会把 `EngineRuntime` 抬 16 × 1032 B ≈ **16.1 KiB**
    /// （仍是 180144 B 的同一量级，但每次按值搬运行时都多背一截）；而本单只需要
    /// "预分配 + 零分配"，堆上那一块在 [`Self::new_boxed`] 的构造期分配一次。
    ///
    /// ⚠ 只在 [`crate::snapshot::EngineSnapshot::routing`] 的 `uses_buses()` 为真时
    /// 清空/使用 ⇒ 默认工程（全部直连 master）**一个字节都不碰**。
    routing_buses: Box<[AudioBlock<DEFAULT_BLOCK_FRAMES>; MAX_TRACK_SLOTS]>,
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
    /// **R76 的读数**：已经设好的声相自动化目标被**同量子的快照武装**取消掉的次数。
    ///
    /// 机制：`ParamSmoother::snap_to` **同时写 `value` 与 `target`**
    /// （`yeban-dsp/src/smoothing.rs:165`）⇒ 槽位**换主人**时那次 `snap_to`
    /// 会把同量子早先设好的自动化目标一并抹掉。⛔ 不许静默：这里逐次计数。
    pan_automation_resets: u64,
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
    /// ⭐ 环音符事件的窃取/重触发次数（⭐ 与 `events_applied` 分开；⭐ 见 `EngineStats`）。
    note_steals: u64,
    /// ⭐ `NoteOff` 找不到匹配声部的次数（⭐ 合并口径；⭐ 见 `EngineStats`）。
    orphan_note_offs: u64,
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
    /// ⚠ **本函数按值返回 ⇒ 每个调用点都要在自己的栈帧里留一份
    /// `size_of::<Self>()`（本机 = `180144 B`；⭐ 任务单 `#107` 的 CC 表 ＋144 B）的返回值位**。Windows 主线程默认栈
    /// ≈ 1 MiB（PE `StackReserve`）时，GUI 那条链会因此溢出 ⇒ **不装箱的调用点
    /// 应当改用 [`Self::new_boxed`]**（它直接在堆上构造，见那里的文档）。
    ///
    /// ⚠ 两个入口共用同一份字段清单（[`Self::write_into`]）⇒ ⛔ 不存在"两个构造函数
    /// 各写一张字段表、然后慢慢漂移"的形态；本函数的**签名**与**可观测输出**一个字没改。
    ///
    /// ⚠ **本函数自身的栈帧**：若直接在栈上就地构造（`MaybeUninit` ＋ `write_into`），
    /// `-C opt-level=0` 下要**两块** 180144 B（`MaybeUninit` 那块 ＋ `assume_init()` 的
    /// 返回值临时量）⇒ 实测帧 `360304 B`（各字段的临时量另在 `write_into` 的
    /// `176416 B` 里，调用期峰值共 `536720 B`）。因此本函数实现为
    /// "**先按堆构造、再按值交出**"：帧降到 `224 B`，调用期峰值由 `write_into`
    /// ＋ 器件构造决定（≈ `306928 B`），⛔ 比改动前（`359312 B` ＋ 器件帧 ⇒ `489296 B`）
    /// **更省**、⛔ 不是更费。
    /// 代价是构造期多一次 `size_of::<Self>()` 的堆分配（本函数本来就在构造期大量分配：
    /// 卷积混响池一处就是 ≈ 4.8 MiB）—— 这条差别写在提交信息与报告里。
    #[must_use]
    pub fn new(
        slot: &Arc<SnapshotSlot>,
        retire: RetireProducer,
        events: EventReceiver,
        meters: MeterPublisher,
    ) -> Self {
        // ⚠ ⛔ 不写成 `MaybeUninit` ＋ `assume_init()`：见上面那段读数（那正是 +180144 B
        // 的来源）。`*boxed` 是"把堆上那份值移出来"，⛔ 不会在栈上再建一份。
        *Self::new_boxed(slot, retire, events, meters)
    }

    /// ⭐ **装箱构造**：`size_of::<EngineRuntime>()`（本机 = `180144 B`）那一份值
    /// **直接在堆上**建成，构造期**没有** `EngineRuntime` 按值落到栈帧上。
    ///
    /// ## 为什么需要它：Windows 主线程默认栈 ≈ 1 MiB
    ///
    /// [`Self::new`] 按值返回 ⇒ 每个调用点都要在自己的栈帧里留一份 180144 B 的
    /// 返回值位，**改动前**函数自己还要一份（实测：`new` 的帧 `359312 B`／
    /// `yeban-app` 那侧的构建里是 `393616 B` —— 那是"单一大对象 × 多层按值副本"，
    /// 不是无关的多层累积）。GUI 那条链
    /// （`yeban-app`：`host::ProductionLoop::start` → `EngineHost::reload`／`open_device`
    /// → `engine_host::fresh_runtime` → `EngineRuntime::new`）在**打开设备之前**就已经
    /// ≥ 1.29 MiB ⇒ Windows 上**开界面即崩**（[R725] 的逐帧读数）。
    ///
    /// ## 做法（⛔ 不是 `Box::new(Self { … })`）
    ///
    /// `Box::<Self>::new_uninit()` 先只在堆上要一块 `size_of::<Self>()`，再由
    /// [`Self::write_into`] **就地**把字段写进去。⚠ 之所以不能写
    /// `Box::new(Self { … })`：Rust 的 `Box::new` 语义是"**先在栈上把值建出来、再搬进
    /// 堆**"，`-C opt-level=0`（本仓判据的档位）下那一份 180144 B 一定在栈帧里
    /// —— 那正是本函数要避开的东西。
    ///
    /// ## 与 [`Self::new`] 的关系
    ///
    /// [`Self::new`] **就是**本函数的按值适配（`*Self::new_boxed(…)`）⇒ 两个入口
    /// **共用**同一个 [`Self::write_into`]、同一份字段清单、同一条"混响延迟线只在
    /// 构造期分配"的纪律；差别只有**分配位置与栈用量**。
    /// ⛔ `new` 的签名与可观测输出一个字没改（内部改为"先按堆构造、再按值交出"，
    /// 构造期多一次 180144 B 的堆分配 —— 见 `new` 的文档）。
    ///
    /// ⚠ 就地写**不再有**"编译器不兜底字段完备性"的代价：共用清单的完备性由
    /// [`Self::write_into`] 末尾的**编译期穷尽解构**保证（清单与结构体定义一旦漂移，
    /// 本函数与 `new` 都**不再编译**）—— 即 R726 登记的那处风险已闭合。
    ///
    /// ## 栈读数（本机 `-C opt-level=0`，判据二进制逐帧实测）
    ///
    /// | 函数 | 自身帧 |
    /// | :--- | ---: |
    /// | `new_boxed` | **`304 B`** |
    /// | `write_into`（两个入口共用） | `176416 B` |
    /// | `SynthEngine::new`（器件构造、未动） | `129984 B` |
    ///
    /// ⇒ `new_boxed` **调用期峰值** = `304 + 176416 + 129984` = `306704 B`，其中
    /// 一个 `EngineRuntime` 都不在栈上；改动前 `new` 的调用期峰值 =
    /// `359312 + 129984` = `489296 B`（⚠ 两个字面读数是 `#107` **之前**实测的栈帧，⛔ 未重测）。
    /// 而**按值调用者**还要多留一份 180144 B 的
    /// 返回值位 —— 那一份是装箱消掉的（见判据 `tests/rt_new_boxed.rs` 的阈值差 `176 KiB`）。
    ///
    /// # Panics
    ///
    /// ⚠ `Box<MaybeUninit<_>>` 的析构**不碰**内容 ⇒ 若某个字段初始化 panic，已经写好的
    /// 字段不会被析构（会泄漏）。⛔ 不会 UB；且那条路本来就等于启动失败
    /// （`EngineRuntime` 的构造期在打开设备之前，没有半成品会逃出去）。
    #[must_use]
    pub fn new_boxed(
        slot: &Arc<SnapshotSlot>,
        retire: RetireProducer,
        events: EventReceiver,
        meters: MeterPublisher,
    ) -> Box<Self> {
        // 堆上只要一块 `size_of::<Self>()`（180144 B）；栈帧里只有一个 `Box` 指针。
        let mut place = Box::<Self>::new_uninit();
        // SAFETY: `write_into` 的有效性前提由本函数满足（`place` 是堆上一块有效、对齐、
        // 未初始化的 `Self`），它写满全部字段；`Box::assume_init` 再把
        // `Box<MaybeUninit<Self>>` 变成 `Box<Self>`（⛔ 不搬值、⛔ 不复制）。
        unsafe {
            Self::write_into(place.as_mut_ptr(), slot, retire, events, meters);
            place.assume_init()
        }
    }

    /// 构造期**唯一的**字段写入点（[`Self::new`] 与 [`Self::new_boxed`] 共用）。
    ///
    /// 字段清单与结构体定义**同名、同序**（78 个字段）⇒ "`EngineRuntime` 由什么组成"
    /// 只有一处描述，⛔ 不会出现两份会漂移的字段表。
    ///
    /// ## ⭐ 字段完备性是**编译期断言**（⛔ 不是文本守卫）
    ///
    /// 就地写（`Box::<Self>::new_uninit()` ＋ `ptr::write`）的代价是：编译器**不再**用
    /// 结构体字面量兜底"字段写全了没有"⇒ 结构体加了字段而这里忘了写，那个字段就是
    /// **未初始化**（UB）。而"清单里提没提这个字段名"这类**文本守卫**的锚点三次都不
    /// 可靠（台账 R729：两次误报、一次假绿）⇒ 本函数把这条不变量交给**编译器**判：末尾
    /// 用**同一串 token** 再展开一次**穷尽解构**（见 `write_fields!`）——
    ///
    /// * 清单里删掉一项 ⇒ **编译失败**（报 "pattern requires .. due to inaccessible
    ///   fields"；⚠ 照它的提示补 `..` 就等于把断言**关掉** ⇒ 正确修法是**把字段写回清单**）；
    /// * 结构体多一个字段而清单没跟上 ⇒ 同一条错误。
    ///
    /// ⚠ 即"清单 == 结构体字段集合"由 rustc 判、判的是**字段身份**（不只是个数），
    /// 且⛔ 不需要过程宏 / 新依赖。
    ///
    /// # Safety
    ///
    /// `dst` 必须指向一块**有效、对齐、当前未初始化**的 `EngineRuntime`。
    /// 本函数返回时 `*dst` 已**完整初始化**：每个字段恰好被 `ptr::write` 写一次，
    /// 因此调用方的 `assume_init()` 合法。
    unsafe fn write_into(
        dst: *mut Self,
        slot: &Arc<SnapshotSlot>,
        retire: RetireProducer,
        events: EventReceiver,
        meters: MeterPublisher,
    ) {
        // 混响延迟线的**唯一**分配点（[`crate::insert`] 模块文档 §8.4 / §8.5）：
        // 采样率取自**初始快照**——调用方（`EngineHost::reload` / `device` 腿）都是
        // 先用工程建好快照、再建运行时，因此这里拿到的就是本工程将要用的采样率。
        // `SnapshotSlot::current` 会在写者锁上短暂等待，因此只允许在**非实时**路径调用
        // （本函数就是那条路径：全部通道必须在打开设备之前建立）。
        let armed_reverb_sample_rate = slot.current().sample_rate();
        // 混响池：**逐台就地写进目的地**，⛔ 不像原来那样先建一个 `[Reverb; 16]`
        // 的栈上临时量（那一个就是 20 992 B）。分配次数、分配顺序与原来逐字相同：
        // 每台 `Reverb::new()` 之后立刻 `set_sample_rate`（唯一的 `Vec` 分配点）。
        for slot_index in 0..MAX_TRACK_SLOTS {
            let mut reverb = Reverb::new();
            // 构造期允许分配（`Vec` 延迟线在这里建好，回调内一次也不重建）。
            reverb.set_sample_rate(armed_reverb_sample_rate as f32);
            // SAFETY: `dst` 由调用方保证有效、对齐、未初始化；`slot_index < 16`
            // ⇒ 落在 `reverb_pool` 数组边界内；本字段在此只写这一次。
            unsafe {
                core::ptr::write(
                    core::ptr::addr_of_mut!((*dst).reverb_pool[slot_index]),
                    reverb,
                );
            }
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
        // ⭐ 就地写字段的**唯一**入口：它同时是"字段清单 == 结构体定义"的**编译期断言**。
        //
        // 为什么需要断言：`new_boxed` 用 `Box::<Self>::new_uninit()` ＋ 就地写 ⇒ 编译器
        // **不再**用结构体字面量兜底"字段写全了没有"⇒ 结构体加了字段而这里忘了写，
        // 那个字段就是**未初始化**（UB）。而**文本守卫**的锚点在 R729 里三次都不对
        // （两次误报、一次假绿）⇒ 这个不变量只能让**编译器**判。
        //
        // 做法（**一份清单、两种展开**）：下面那串 `put!(<字段>, <值>);` 里的每个 `$field`
        //   ① 生成一条 `ptr::write`（顺序 = 清单顺序 ⇒ 分配次数与顺序与改动前**逐字相同**）；
        //   ② 参与末尾那次**穷尽解构**（⛔ 不写 `..`）⇒ 清单与结构体字段**逐个**对上，
        //      少一项 / 多一项都在**编译期**红。⚠ 它判的是**字段身份**（不只是个数）。
        //
        // ⚠ 清单里的 `put!(…)` **不是宏调用**：`write_fields!` 的匹配器把它们当**行标记**
        //    吃掉 ⇒ 原清单每一行的**内容一个字符都没改**（只多了缩进），⛔ 不存在"改写
        //    写法引入新求值顺序"的可能（核验口径：`git diff -w` 只有增删、无改写）。
        //
        // ⚠ 宏而不是辅助函数：字段名必须**保持是"字段"**（`addr_of_mut!` 取的是 place），
        // 一旦变成函数参数就先求值成了一份值 ⇒ 大字段又被搬回栈上，那正是本函数要避开的。
        macro_rules! write_fields {
            // `@drained <字段>…`：**已经**在本函数别处就地写好的字段（目前只有 `reverb_pool`
            // ——上面那个逐台 `Reverb` 的循环）。它们参与穷尽解构、⛔ 不参与这里的写入。
            (@drained $($drained:ident),*; $( put!($field:ident, $value:expr); )*) => {{
                $(
                    // SAFETY: 调用方是本函数 —— `dst` 有效、对齐、未初始化；每个字段名在
                    // 本清单里恰好出现一次（`@drained` 那一组由本函数别处各写一次）
                    // ⇒ 每次展开都只写一个字段，写满后 `*dst` 完整初始化。
                    unsafe {
                        core::ptr::write(core::ptr::addr_of_mut!((*dst).$field), $value)
                    };
                )*
                // ⭐ **编译期字段普查**：穷尽解构（⛔ 无 `..`）⇒ 本清单必须与
                // `struct EngineRuntime` 的字段**逐个对上**，否则这一行不编译（报
                // "pattern requires .. due to inaccessible fields"）。⚠ 照它的提示补 `..`
                // 就等于把本断言**关掉** ⇒ 正确修法是**把漏掉的字段写回清单**。
                // 全部绑定为 `_` ⇒ 只读：⛔ 不搬值、⛔ 不分配、⛔ 不改任何字段。
                // SAFETY: 上面已把**全部**字段各写一次（`@drained` 那一组由本函数别处
                // 各写一次）⇒ `*dst` 是一份合法、已初始化的 `Self` ⇒ 取共享引用合法。
                let Self { $($drained: _,)* $($field: _,)* } = unsafe { &*dst };
            }};
        }
        // ---- 字段清单（与 `struct EngineRuntime` 的定义**同名、同序**）----
        // ⭐ 本清单是**唯一**的字段表：`write_fields!` 用同一串 token 展开出
        // ① 逐字段就地写、② 编译器核对的穷尽解构（见上面那一段）。
        write_fields! {
            // 上面那个循环已经就地写好的那一组：⛔ 不在下面的清单里，但**参与**穷尽解构。
            @drained reverb_pool;
            put!(snapshot, SnapshotReader::attach(slot, retire));
            put!(audition, None);
            put!(audition_active, false);
            put!(pending_launch, None);
            put!(scene_launches_applied, 0);
            put!(events, events);
            put!(control_change, ControlChangeState::new());
            put!(meters, meters);
            put!(block, AudioBlock::new());
            put!(scratch_events, [EngineEvent::IDLE; SCRATCH_EVENTS]);
            put!(scratch_meters, [MeterFrame::default(); SCRATCH_METERS]);
            put!(track_scratch, [0.0; DEFAULT_BLOCK_FRAMES]);
            // ⭐ 总线暂存池：**构造期**一次分配到堆上（`Box<[_; 16]>` = 瘦指针）
            // ⇒ 回调内零分配 [MUST-GATE-001]。
            put!(
                routing_buses,
                Box::new(core::array::from_fn(|_| AudioBlock::new()))
            );
            put!(bank, MeterBank::new());
            // 采样率先按 48 kHz 武装；第一次武装快照时按快照校准（`begin_snapshot`）。
            put!(synth, SynthEngine::new(48_000));
            // 走带默认**自由跑**：接入走带之前 `process_quantum` 的语义就是
            // "快照一发布就从 tick 0 起滚"，因此未收到任何命令时行为逐位不变。
            // 要"加载即停住"的控制面显式发一条 `Stop`（`yeban-app` 的 GUI 路径
            // 在 `EngineHost::reload` 之后就这么做；`reload` 自己**不**碰走带状态）。
            put!(
                transport,
                Transport::free_running(48_000, yeban_model::project::DEFAULT_BPM)
            );
            put!(transport_mirror, Arc::new(TransportMirror::new()));
            // 统计镜像：**构造期**分配一次（回调内只做原子存）[MUST-GATE-001]。
            put!(stats_mirror, Arc::new(EngineStatsMirror::new()));
            put!(limiter, BusLimiter::new());
            // PDC 延迟线池：**构造期**按上限预分配（回调内绝不再分配）。
            put!(
                pdc,
                CompensationBank::preallocated(PDC_SLOTS, MAX_PDC_DELAY_FRAMES)
            );
            put!(pdc_unarmed_nodes, 0);
            put!(pdc_clamped_frames, 0);
            put!(pdc_alignment_frames, 0);
            put!(engine_output_latency_frames, 0);
            put!(armed_pan_law, PanLaw::default());
            put!(
                armed_pan_gains,
                [(
                    EntityId::default(),
                    core::f32::consts::FRAC_1_SQRT_2,
                    core::f32::consts::FRAC_1_SQRT_2,
                ); MAX_TRACK_SLOTS]
            );
            put!(armed_pan_slots, 0);
            // 声相平滑器：**构造期**预分配（回调内绝不再分配）。`with_default_time`
            // 需要一个采样率来算 α；真正的采样率在快照边界用 `set_sample_rate` 校准
            // （同值提前返回 ⇒ 不重复算 `exp`）。
            // 初值采样率用 48 kHz（与 `ParamTable::new` 的判据默认值同一个数）；
            // 真正的采样率在**快照边界**用 `set_sample_rate` 校准（同值提前返回）。
            put!(
                armed_pan_l,
                core::array::from_fn(|_| ParamSmoother::with_default_time(48_000.0))
            );
            put!(
                armed_pan_r,
                core::array::from_fn(|_| ParamSmoother::with_default_time(48_000.0))
            );
            put!(armed_pan_armed, [false; MAX_TRACK_SLOTS]);
            put!(pan_automation_frames, 0);
            put!(pan_automation_value_rejects, 0);
            put!(pan_automation_resets, 0);
            put!(armed_master_gain, 1.0);
            put!(armed_metronome_enabled, false);
            put!(armed_metronome_ticks_per_beat, 0);
            put!(armed_metronome_beats_per_bar, 0);
            put!(metronome, MetronomeVoice::new());
            put!(limiter_gain_reductions, 0);
            put!(limiter_max_reduction, 0.0);
            put!(limiter_current_reduction, 0.0);
            // 插入链：**构造期**预分配全部槽位（回调内绝不再分配）。
            // `from_fn` 而不是 `[expr; N]`：`ChannelStrip` 没有 `const` 构造器，
            // 而 `Option<ChannelStrip>` 的"全 `None`"初值用函数形式表达最直接
            // （`from_fn` 在**构造期**调用，不在回调里）。
            put!(
                armed_strips,
                core::array::from_fn(|_| (EntityId::default(), None))
            );
            put!(armed_insert_slots, 0);
            // ⚠ `reverb_pool` **不在这里**：它已经在上面逐台就地写好了（那一段就是本字段）。
            put!(
                armed_reverbs,
                [(EntityId::default(), false); MAX_TRACK_SLOTS]
            );
            put!(armed_reverb_slots, 0);
            put!(armed_reverb_sample_rate, armed_reverb_sample_rate);
            put!(reverb_scratch, [[0.0; DEFAULT_BLOCK_FRAMES]; 2]);
            // 卷积混响池：**构造期**按初始快照的采样率把占位 IR 与预延迟线全部建好
            // （回调内只做同长度的换 IR 与 `set_params`）。
            put!(conv_pool, conv_pool);
            put!(armed_convs, [(EntityId::default(), false); MAX_TRACK_SLOTS]);
            put!(armed_conv_slots, 0);
            put!(armed_conv_ir_hashes, [0; MAX_TRACK_SLOTS]);
            put!(armed_conv_sample_rate, armed_conv_sample_rate);
            put!(conv_scratch, [0.0; 2 * DEFAULT_BLOCK_FRAMES]);
            put!(insert_gain_reductions, 0);
            put!(insert_max_reduction_db, 0.0);
            put!(insert_current_reduction_db, 0.0);
            // 冷值就是 `None`（= `EngineStats::default()` 的派生值）：`0.0` 是一个合法
            // 的检波器电平（0 dBFS）⇒ 不能用它表示"从没有过读数"。
            put!(insert_detector_level_db, None);
            put!(insert_strip_frames, 0);
            put!(insert_reverb_frames, 0);
            put!(insert_reverb_rate_rejects, 0);
            put!(insert_convolution_frames, 0);
            put!(insert_convolution_rejects, 0);
            // 参数目标表：**构造期**建满（回调内惰性分配的槽位都落在这张定长表里）。
            put!(params, params);
            put!(armed_master, armed_master);
            put!(armed_revision, None);
            put!(quanta, 0);
            put!(events_applied, 0);
            put!(note_steals, 0);
            put!(orphan_note_offs, 0);
            put!(event_bulk_pops, 0);
            put!(meter_frames, 0);
            put!(meter_bulk_publishes, 0);
            put!(meter_capacity_drops, 0);
            put!(ftz, None);
            put!(ftz_ready, false);
            put!(armed_quanta_per_second, None);
            put!(armed_scheduled_notes, 0);
            put!(armed_note_schedule_drops, 0);
        }
        // 先发布一次初值：控制面在**第一次量子之前**就能读到"Playing / tick 0"，
        // 而不是一个与引擎实际状态不符的冷值（`TransportReading::cold`）。
        // SAFETY: 上面已把**全部**字段各写一次 ⇒ `*dst` 现在是一份合法、已初始化的
        // `EngineRuntime` ⇒ 取共享引用合法（`publish` 只读这两个字段）。
        let runtime: &Self = unsafe { &*dst };
        runtime.transport.publish(&runtime.transport_mirror);
        // 统计镜像也先发布一次：控制面在**第一个量子之前**就能读到与引擎实际状态
        // 相符的数（`ftz` 与走带状态就是这一批里最有用的两个），而不是全零冷值。
        // 与上面那条同一个理由、同一个位置（非实时路径）。
        let cold = runtime.stats();
        runtime.stats_mirror.publish(&cold);
    }

    /// 挂上 A/B 盲听的**第二份可渲染版本**（[ARCH-RT-005] 片 1：**只持有**）。
    ///
    /// **只在音频线程启动前调用**（与 [`Self::new`] 同一条纪律；`attach` 会取槽的锚锁）。
    ///
    /// ⚠ 本方法刻意**追加**在 [`Self::new`] **之外** ⇒ `new` 的签名不变、既有调用点
    /// （`yeban-app` 的设备腿与若干判据）一个字都不用改。
    ///
    /// 片 1 的语义：挂上之后**输出逐字节不变**，且音频回调**零分配**。
    pub fn attach_audition_branch(&mut self, slot: &Arc<SnapshotSlot>, retire: RetireProducer) {
        self.audition = Some(AuditionBranch::attach(slot, retire));
    }

    /// 是否已挂上第二份可渲染版本（判据用它证明「管道不是死代码」）。
    #[must_use]
    pub fn has_audition_branch(&self) -> bool {
        self.audition.is_some()
    }

    /// **开始试听**（[ARCH-RT-005] 片 2b 的**准入判定**）。
    ///
    /// ## 受限版的口径（⭐ 本函数存在的理由）
    ///
    /// 两条分支**共用主线那一套武装表** ⇒ 凡喂进武装表的快照事实都必须一致。
    /// 不一致时本函数**拒绝**并如实报出原因（[`AuditionRefusal`]），
    /// ⛔ **绝不允许**「拿主线的武装表静默渲染提案」。
    ///
    /// ## 边界
    ///
    /// ⚠ 控制路径（会读 `held()` 与两次 `PartialEq`）⇒ **⛔ 不许在回调里调用**。
    /// 本函数**不分配**（`Arc::clone` 只加一次引用计数；比较全是借用）。
    ///
    /// # Errors
    ///
    /// 拓扑不一致时返回具体原因；主线侧还没有快照时返回
    /// [`AuditionRefusal::NoMainSnapshot`]。
    pub fn begin_audition(
        &mut self,
        proposal: &Arc<EngineSnapshot>,
        main_automation: AutomationTargets,
        proposal_automation: AutomationTargets,
    ) -> Result<(), AuditionRefusal> {
        let main = match self.snapshot.held() {
            Some(main) => Arc::clone(main),
            None => return Err(AuditionRefusal::NoMainSnapshot),
        };
        if let Some(reason) =
            audition_topology_mismatch(&main, proposal, main_automation, proposal_automation)
        {
            return Err(reason);
        }
        let sample_rate = proposal.sample_rate();
        let position_frames = self.transport.position_frames();
        // ---- [ARCH-RT-005] 片 3：**预滚 + 下一拍准备** ----
        //
        // ⭐ 语义：切换帧**永远不早于**"请求时刻 + 预滚帧数"。
        //    切换帧仍然落在**拍点**上：预滚不够就**顺延到再下一拍**（⛔ 不是把切换
        //    挪到拍与拍之间 —— 那会破坏"下一拍对齐"这条已钉住的语义）。
        //
        // ⚠ 预滚长度**按采样率缩放**（[`crate::mixer::audition_preroll_frames`]）：
        //    裸 2048 在 96 kHz 下小于 30 ms 淡化所需的 2880。
        //
        // 全部是整数运算（⛔ 无浮点、无超越函数、无分配）⇒ 控制路径，允许取锁/分配，
        // 但这里其实都不需要。
        let preroll = crate::mixer::audition_preroll_frames(sample_rate as f32) as u64;
        let mut tick = next_beat_tick(self.transport.position_ticks());
        while self
            .transport
            .frames_for_tick(tick)
            .saturating_sub(position_frames)
            < preroll
        {
            tick = next_beat_tick(tick);
        }
        let switch_frame = self.transport.frames_for_tick(tick);
        if let Some(branch) = self.audition.as_mut() {
            branch.prepare(sample_rate, switch_frame, position_frames);
        }
        self.audition_active = true;
        Ok(())
    }

    /// ⭐ 控制路径：**武装一次量化场景启动**（照 [`Self::begin_audition`] 的形状）。
    ///
    /// ⭐ **"何时开始"由量化档决定**：`quantization` 只有小节整数倍
    /// （[`yeban_model::LaunchQuantization`]）⇒ 生效帧 = 从当前位置算出的
    /// **第 n 个小节线**，经 [`crate::transport::next_bar_tick`] ＋ `frames_for_tick` 落到采样点。
    /// `Off` ⇒ 取**当前帧**（立即生效，⭐ 这就是"控制臂"：证明量化那条红是因为量化）。
    ///
    /// ⚠ `time_signature` 由控制面给出（引擎的快照里没有拍号 ⇒ ⛔ 不在这里发明）；
    /// ⚠ 本函数不分配、不取锁。
    pub fn queue_scene_launch(
        &mut self,
        scene_id: Option<yeban_model::EntityId>,
        tempo_bpm: f64,
        quantization: yeban_model::LaunchQuantization,
        time_signature: yeban_model::TimeSignature,
    ) {
        let position_frames = self.transport.position_frames();
        let switch_frame = match quantization {
            yeban_model::LaunchQuantization::Off => position_frames,
            other => {
                let step = quantization_bars(other);
                // 量化档是"1/2/4/8 小节"⇒ 从**下一个小节线**起再顺延 (step-1) 个小节。
                let mut tick = crate::transport::next_bar_tick(
                    self.transport.position_ticks(),
                    time_signature,
                );
                for _ in 1..step {
                    tick = crate::transport::next_bar_tick(tick, time_signature);
                }
                self.transport.frames_for_tick(tick)
            }
        };
        self.pending_launch = Some(PendingSceneLaunch::arm(
            scene_id,
            tempo_bpm,
            switch_frame,
            position_frames,
        ));
    }

    /// ⭐ 控制路径：**取消**尚未生效的启动（幂等；⛔ 已生效的不受影响）。
    pub fn cancel_scene_launch(&mut self) {
        self.pending_launch = None;
    }

    /// 当前是否有排队的场景启动。
    #[must_use]
    pub const fn has_pending_scene_launch(&self) -> bool {
        self.pending_launch.is_some()
    }

    /// ⭐ 累计**真的生效**过的场景启动次数。
    #[must_use]
    pub const fn scene_launches_applied(&self) -> u64 {
        self.scene_launches_applied
    }

    /// 结束试听（幂等；不分配）。
    pub fn end_audition(&mut self) {
        self.audition_active = false;
    }

    /// 试听是否已开始。
    #[must_use]
    pub const fn is_auditioning(&self) -> bool {
        self.audition_active
    }

    /// 走带口径：**距下一拍**还有多少帧（与试听是否已开始无关）。
    ///
    /// 判据用它把"预滚顺延"摆成可观测的：请求时刻到下一拍的距离**小于**预滚时，
    /// [`Self::begin_audition`] 必须顺延到**再下一拍**。
    #[must_use]
    pub fn frames_until_audition_switch(&self) -> u64 {
        self.transport.frames_until_audition_switch()
    }

    /// 本次试听**已存下的**倒计时（`None` ＝ 未挂第二分支）。
    ///
    /// ⭐ 挂上预滚之后它**必须 ≥ 预滚帧数**（否则第二分支的准备时间短于淡化）。
    #[must_use]
    pub fn audition_frames_until_switch(&self) -> Option<u64> {
        self.audition
            .as_ref()
            .map(AuditionBranch::frames_until_switch)
    }

    /// 第二分支**真的被渲染过**的块数（`None` ＝ 未挂上）。空闲时必须恒为 `0`。
    #[must_use]
    pub fn audition_blocks_read(&self) -> Option<u64> {
        self.audition.as_ref().map(AuditionBranch::blocks_read)
    }

    /// 本次试听的**绝对切换帧**（`None` ＝ 还没挂第二分支；`0` ＝ 还没开始过）。
    ///
    /// 它来自 [`Transport::audition_switch_frame`]（"下一拍"的绝对帧）⇒ 判据用它
    /// 证明"切换帧是由走带决定的"，⛔ 不是随手挑的常数。
    #[must_use]
    pub fn audition_switch_frame(&self) -> Option<u64> {
        self.audition.as_ref().map(AuditionBranch::switch_frame)
    }

    /// 第二分支当前持有的快照修订号（`None` ＝ 未挂上）。
    ///
    /// 片 1 里第二读者**从不 `begin_block`** ⇒ 这个值是「`attach` 那一刻锚住的」那
    /// 一份。它让「第二份版本真的在册且连对了槽」可被**观测**。
    #[must_use]
    pub fn audition_held_revision(&self) -> Option<u64> {
        self.audition
            .as_ref()
            .and_then(AuditionBranch::held_revision)
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
            note_steals: self.note_steals,
            orphan_note_offs: self.orphan_note_offs,
            snapshot_switches: self.snapshot.switches(),
            scene_launches_applied: self.scene_launches_applied,
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

    /// **R76 的读数**：已设好的声相自动化目标被**同量子的快照武装**取消掉的次数
    /// （机制见字段文档：`snap_to` 会同时写 `target`）。
    #[must_use]
    pub const fn pan_automation_resets(&self) -> u64 {
        self.pan_automation_resets
    }

    /// ⭐ **读一个 MIDI CC 的当前值**（任务单 `#107`；会话运行态）。
    ///
    /// * `Some(v)` ＝ 这个控制器**收到过**至少一条 [`EngineEvent::ControlChange`]，
    ///   当前值 `v`（最后一条赢）；
    /// * `None` ＝ 它**从未**收到过事件（⛔ 与"收到 0"可区分 —— 后者的读数是 `Some(0)`）。
    ///
    /// ⚠ 控制器号按 `& 0x7F` 掩码（与事件面的掩码同口径；`0xFF` 读的是 127）。
    /// 存在的理由（"读数"纪律）：判据要能**不靠音频输出**回答"事件面推一条 CC 之后，
    /// 引擎侧的状态到底变没变"，否则"CC 被静默丢弃"与"CC 生效了但听不出来"不可区分。
    /// 判据见 `tests/sfz_instrument.rs` 的 S63 与 `yeban-app` 的
    /// `tests/midi_input_notes.rs`（实时泵那条腿）。
    #[must_use]
    pub const fn control_change(&self, controller: u8) -> Option<u8> {
        self.control_change.received(controller)
    }

    /// ⭐ **整张 CC 表是否已 armed**（＝ 至少收到过一条 CC 事件；任务单 `#107`）。
    ///
    /// 它同时是**实时探针的开关**（[`Self::render_block`] 的 3a 段）：
    /// `false` ⇒ 传 `None` ⇒ 没有 CC 事件的工程与接线前**逐位相同**。
    #[must_use]
    pub const fn control_change_armed(&self) -> bool {
        self.control_change.is_armed()
    }

    /// 收到过事件的**控制器个数**（读数：分母 128；任务单 `#107`）。
    #[must_use]
    pub const fn control_change_received_count(&self) -> u32 {
        self.control_change.received_count()
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
            audition,
            audition_active,
            events,
            control_change,
            meters,
            block,
            scratch_events,
            scratch_meters,
            track_scratch,
            routing_buses,
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
            pan_automation_resets,
            pdc,
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
            note_steals,
            orphan_note_offs,
            event_bulk_pops,
            meter_frames,
            meter_bulk_publishes,
            meter_capacity_drops,
            pending_launch,
            scene_launches_applied,
            ..
        } = self;

        // --- ⭐ 首块修正：⭐ 让 tick 0 的环音符**不再被丢** [ARCH-RT-001] ---
        //
        // 症状：⭐ 本函数的顺序是「⭐ 出队并应用事件 → ⭐ 武装快照」，⭐ 而声部槽要**武装**
        // 时才占用 ⇒ ⭐ **首块**里事件到达时槽位还没影 ⇒ ⭐ `SynthEngine::note_on` 早退
        // 返回 `false` ⇒ ⭐ **tick 0 的音符静默消失**（⭐ app 线三探针实测：⭐ 单条
        // `C4@tick0` ⇒ `poly_notes_triggered=0`／`nonzero=0`／`orphan_note_offs=1`）。
        //
        // 修法：⭐ **只把"⭐ 音符路径需要的那三样"提前**（⭐ 槽位对齐 ＋ 增益门）⇒
        // ⚠ **参数/走带/PDC/节拍器/声相那一段仍留在出队之后的原处**（⭐ 见下），
        // ⭐ 因为 `SetParam`／`Transport` 需要的是"⭐ **上一份**已武装的参数"
        // ⇒ ⭐ 提前它们会把参数/走带语义**平移一个量子**。
        //
        // ⚠ **语义影响的准确表述**：⭐ **除首块外每量子逐位不变**；⭐ **首块在旧行为下
        // 输出为空**（⭐ 槽位没武装 ⇒ ⭐ 无输出可被改变）；⭐ 首块的事件**现在看到真实快照**
        // —— ⭐ 那是**更正确**的行为（⭐ 而不是看到过期默认值）。⭐ 参数/走带/声相那一段
        // **位置与取值都没动** ⇒ ⭐ ⭐ **本次改动的语义影响只剩音符路径**。
        //
        // ⚠ **必须有 skip 机制**：⭐ 实测（⭐ 判据 `insert_slot_exhaustion_is_counted_and_never_panics`
        // 抓到 `left: 2, right: 1`）`begin_block()` 在**同一量子第二次并不返回 `None`**
        // ⇒ ⭐ 不守就会把武装**跑两遍**（⭐ `track_drops` 由 1 变 2）⇒ ⭐ 故下面原处那三个
        // `synth.` 调用必须用 `early_armed` 守住。
        //
        // ⚠ **借用为什么合法**：⭐ `snapshot` 是上面 `let EngineRuntime { … } = self;` 解构出的
        // **字段之一** ⇒ ⭐ 它与下面闭包捕获的 `params`／`synth`／`transport`／`metronome`
        // 是**不相交的 `&mut`** ⇒ ⭐ `current: &Arc<EngineSnapshot>` 在本 `if let` 内、
        // **闭包之前**存活是合法的（⭐ 闭包从不碰 `snapshot`；⭐ 借用在此 `if let` 结束即止）。
        let mut early_armed = false;
        if !synth.has_armed_tracks()
            && let Some(current) = snapshot.begin_block()
        {
            // ⭐ 与后一个块各自取一次（⭐ 那个块里也是这么绑的）。
            let master = current.master();
            synth.begin_snapshot(
                current.sample_rate(),
                current.tracks().keys().filter(|id| **id != master),
                current.tones().iter().filter(|(id, _)| **id != master),
                current.drums().iter().filter(|(id, _)| **id != master),
            );
            synth.align_cursors(current.schedules().iter().filter(|(id, _)| **id != master));
            synth.set_track_gain_scales(current);
            early_armed = true;
        }

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
            // ⭐ 环音符事件：⭐ `NoteOn` ⇒ 推活音符；⭐ `NoteOff` ⇒ 松它。
            // ⚠ ⭐ 计数与 `events_applied` **分开**：⭐ `events_applied` 只数"⭐ 事件到了"，
            // ⭐ 这两个数"⭐ 窃取了"与"⭐ 松键没找到"（⭐ 判据⛔ 不许拿它们单独举证发声）。
            if let EngineEvent::NoteOn {
                track,
                pitch,
                velocity,
            } = event
                && synth.note_on(track, pitch, velocity)
            {
                *note_steals = note_steals.wrapping_add(1);
            }
            if let EngineEvent::NoteOff { track, pitch } = event
                && !synth.note_off(track, pitch)
            {
                *orphan_note_offs = orphan_note_offs.wrapping_add(1);
            }
            // ⭐ **MIDI CC**（任务单 `#107`）：把值推进**会话运行态表**。
            // 它只写两个整数（一次表写 ＋ 一次位或）⇒ 零分配、零锁、零 I/O
            // [MUST-GATE-001]。表本身是 [`EngineRuntime`] 的一个定长字段
            // （[`ControlChangeState`]，`144 B`），⛔ 不进快照 [MODEL-ISO-001]。
            // 写在这里的**位置**与 `NoteOn`/`NoteOff` 同一处（量子边界的批量出队）
            // ⇒ 同一批里的顺序 ＝ 到达顺序（FIFO，最后一条赢）。
            // ⚠ 计数：CC 没有"没生效"的分支（128 个控制器都是合法地址），因此它
            // **只**计进 `events_applied`（上面那个"事件到了"的读数），⛔ 不另开计数器
            // ——"无事可报"与"没有这条路径"在读数上必须可区分，`events_applied` 就是那条
            // 区分的见证（判据：`tests/sfz_instrument.rs` 的 S63）。
            if let EngineEvent::ControlChange { controller, value } = event {
                let _ = control_change.apply(controller, value);
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

                // --- 2a-cc) ⭐ `<control> set_ccN=` 的**播种**（任务单 `#110`）---
                //
                // 规范逐字（<https://sfzformat.com/opcodes/set_ccN/>）：
                //
                // > Sets a default initial value for MIDI CC number N, when the instrument
                // > is initially loaded. Used under the ‹`control`› header.
                //
                // ⇒ 播种点是**快照边界**（"乐器被装载/换上一份新表"那一刻），载体是
                // **摊平表自带的**初值表 [`crate::sfz_instrument::CcSeedTable`]
                // （`PreparedSfz::cc_seeds`，摊平期从**同一次解析**的
                // `Instrument::cc_defaults` 抄下）。
                //
                // ⭐ **为什么在这里而不是事件面**（自裁记录，⛔ 不是随手选）：
                // ① 初值是**控制侧摊平的结果**（"文件里真的写了 `set_ccN`" ⇒ 表非空），
                //    把它做成事件就得让控制侧**另发一批** `EngineEvent::ControlChange`
                //    —— 那会让"播种"受 SPSC 环容量影响（满环 ⇒ 悄悄少播一条），
                //    而摊平表是**按值随快照到达**的，⛔ 无丢件可能；
                // ② ⛔ **不新增首个 RT 状态**：播种规则是"只填**从未收到过**的槽"
                //    （[`ControlChangeState::seed`]）⇒ 它对"每次修订重新武装"**幂等**
                //    ⇒ ⛔ 不需要在 `EngineRuntime` 里记"这条轨播过没有"
                //    ⇒ `EngineRuntime` 尺寸 **＋0**（判据 S9／S91 现读 `180144`）。
                //
                // ⭐ **保命不变量的机械开关**：表为空 ⇒ `is_empty()` 为真 ⇒ 本段
                // **一次 `seed` 都不调** ⇒ `ControlChangeState::is_armed()` 保持 `false`
                // ⇒ `render_branch` 继续传 `cc = None` ⇒ 没有 `set_ccN` 的文件与接线前
                // **逐位相同**（判据 S87 的端到端指纹 ＋ 三个承重指纹）。
                //
                // ⚠ **与真 MIDI CC 的关系**：事件面（上面 1) 段）先跑、播种后跑
                // ⇒ 同一量子里的**真 CC 覆盖初值**；播种⛔ 不覆盖已经收到过的槽。
                // ⚠ 域外声明（`N > 127`，语料里 38 处 `set_cc400=63`）**不进表**、
                // 只计数（[`CcSeedTable::out_of_domain`]）—— ⛔ 不用 `& 0x7F` 去别名它们。
                // 零分配、零锁、零 I/O [MUST-GATE-001]：BTreeMap 迭代 ＋ 定长位测试。
                for plan in current.sfz().values() {
                    let seeds = plan.prepared().cc_seeds();
                    if seeds.is_empty() {
                        continue;
                    }
                    seeds.for_each(|cc, value| {
                        let _ = control_change.seed(cc, value);
                    });
                }

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
                // ⭐ 首块已提前做过这三件 ⇒ ⭐ 这里守住（⭐ 否则武装跑两遍：⭐ `begin_block()`
                // 同一量子第二次**不返回 `None`**，⭐ 由 `track_drops` 判据实测）。
                if !early_armed {
                    synth.begin_snapshot(
                        current.sample_rate(),
                        current.tracks().keys().filter(|id| **id != master),
                        current.tones().iter().filter(|(id, _)| **id != master),
                        current.drums().iter().filter(|(id, _)| **id != master),
                    );
                    synth
                        .align_cursors(current.schedules().iter().filter(|(id, _)| **id != master));
                    // ⭐ 逐轨增益门（与 `begin_snapshot` 同一处边界；⭐ 顺序：**先槽位对齐、后增益**）
                    // ⇒ ⭐ 活音符的 gain 与已调度音符同源（⛔ 不是两处各算一遍）。
                    synth.set_track_gain_scales(current);
                }
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
                let shortfall = pdc.rearm(current.pdc());
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
                        // R76：上一任留下的自动化目标会被这次武装的 `snap_to` **一并取消**
                        // （`snap_to` 同时写 `value` 与 `target`）⇒ **计数**，⛔ 不静默。
                        if armed_pan_armed[slot] {
                            *pan_automation_resets = pan_automation_resets.wrapping_add(1);
                        }
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

            let running = transport.is_playing();
            // ⭐ 本量子第一帧的**绝对帧**（走带推进之前读；任务单 `#135`）。
            // 音频摆放的区间是绝对帧 ⇒ 逐轨混合只需要这一个读数，⛔ 不需要任何游标。
            // 停住时 `running == false` ⇒ 该路径整段跳过，本读数不参与任何样本。
            let block_start_frame = transport.position_frames();
            {
                let mut share = BranchShare {
                    pdc: &mut *pdc,
                    params: &mut *params,
                    // ⭐ 共享借用：写它的是**上面**那个出队闭包，本处只读（NLL：可变借用已结束）。
                    control_change: &*control_change,
                    bank: &mut *bank,
                    armed_strips: &mut *armed_strips,
                    armed_insert_slots: &mut *armed_insert_slots,
                    reverb_pool: &mut *reverb_pool,
                    armed_reverbs: &mut *armed_reverbs,
                    armed_reverb_slots: &mut *armed_reverb_slots,
                    reverb_scratch: &mut *reverb_scratch,
                    conv_pool: &mut *conv_pool,
                    armed_convs: &mut *armed_convs,
                    armed_conv_slots: &mut *armed_conv_slots,
                    conv_scratch: &mut *conv_scratch,
                    armed_pan_l: &mut *armed_pan_l,
                    armed_pan_r: &mut *armed_pan_r,
                    armed_pan_slots: &mut *armed_pan_slots,
                    armed_pan_armed: &mut *armed_pan_armed,
                    track_scratch: &mut *track_scratch,
                    routing_buses: &mut *routing_buses,
                    scratch_meters: &mut *scratch_meters,
                    insert_gain_reductions: &mut *insert_gain_reductions,
                    insert_max_reduction_db: &mut *insert_max_reduction_db,
                    insert_current_reduction_db: &mut *insert_current_reduction_db,
                    insert_detector_level_db: &mut *insert_detector_level_db,
                    insert_strip_frames: &mut *insert_strip_frames,
                    insert_reverb_frames: &mut *insert_reverb_frames,
                    insert_convolution_frames: &mut *insert_convolution_frames,
                    pan_automation_frames: &mut *pan_automation_frames,
                    meter_capacity_drops: &mut *meter_capacity_drops,
                    pan_gains: &mut pan_gains,
                };
                produced = render_branch(
                    &mut share,
                    synth,
                    block,
                    current,
                    frames,
                    running,
                    quantum,
                    block_start_frame,
                );

                // ---- [ARCH-RT-005] 片 2b/2c：**并发渲染第二分支 + 等功率混合** ----
                //
                // ⚠ 未试听时**整段跳过** ⇒ 输出逐字节不变、RT 成本 0（片 1/2a 钉住的性质）。
                // ⚠ 第二引擎的武装**零分配**（与主线同一条纪律：同采样率下只写预分配槽位）。
                // ⚠ `begin_quantum()` 会被调用两次 ⇒ 试听期的电平读数是**后渲染那条**的
                //    （分支 B）。这是**明说的**口径，⛔ 不是遗漏。
                if *audition_active
                    && let Some(branch) = audition.as_mut()
                    && let Some(proposal) = branch.reader.begin_block()
                {
                    if branch.armed_revision != Some(proposal.revision()) {
                        branch.synth.begin_snapshot(
                            proposal.sample_rate(),
                            proposal.tracks().keys().filter(|id| **id != master),
                            proposal.tones().iter().filter(|(id, _)| **id != master),
                            proposal.drums().iter().filter(|(id, _)| **id != master),
                        );
                        branch.synth.align_cursors(
                            proposal.schedules().iter().filter(|(id, _)| **id != master),
                        );
                        branch.synth.set_track_gain_scales(proposal);
                        branch.armed_revision = Some(proposal.revision());
                    }
                    let _ = render_branch(
                        &mut share,
                        &mut branch.synth,
                        &mut branch.block,
                        proposal,
                        frames,
                        running,
                        quantum,
                        block_start_frame,
                    );
                    blend_equal_power(
                        block,
                        &branch.block,
                        frames,
                        branch.frames_until_switch,
                        &branch.fade,
                    );
                    branch.blocks_read = branch.blocks_read.saturating_add(1);
                    branch.frames_until_switch =
                        branch.frames_until_switch.saturating_sub(frames as u64);
                }
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

            // ⭐ 量化场景启动：**到点生效、恰好一次**。
            //
            // 走带是唯一时钟 ⇒ 生效判决放在推进之后：`frames_until_switch` 每块只减一次、
            // 减到 0 才动作（整数减法＋一次比较，零分配）。触发后状态机回到 `None`
            // ⇒ 幂等、⛔ 不重复触发。
            if let Some(launch) = pending_launch
                && launch.advance(frames as u64)
            {
                // tempo 覆盖：`arm` 保位置、只重算有理数（⛔ 不跳变）。
                // ⚠ 采样率沿用走带**已武装**的值（上一次 `arm` 来自同一份快照）
                // ⇒ `retune` 只换速度，⛔ 不在这里重新读快照（那要 `begin_block()` 借出）。
                transport.retune(launch.tempo_bpm);
                *pending_launch = None;
                *scene_launches_applied = scene_launches_applied.saturating_add(1);
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

/// 把一段**立体声**信号以**单位增益**累加进一条总线暂存槽（任务单 `#158`）。
///
/// 与 [`route_panned`] 的"目的地是总线"那一支是同一条逐样本公式，区别只有两点：
/// 增益固定 `1.0`（出边的增益留到 3a⁗ 转发时再乘），目的地由**下标**直接给出
/// （处理型总线的**自身声源**并入的是它自己的暂存槽）。
///
/// 逐样本一次"加"，零分配、零锁、零 I/O [MUST-GATE-001]。
fn sum_stereo_into_bus(
    bus: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    left: &[f32],
    right: &[f32],
    frames: usize,
) {
    let (bus_left, bus_right) = bus.stereo_mut();
    // 三个切片都由调用方按本量子的 `frames` 切片；`min` 只是把"有效帧数"写成显式
    // 上界（与 `sum_into_bus` 的 `zip` 同一条纪律：绝不越界、绝不 panic）。
    let frames = frames.min(bus_left.len()).min(left.len()).min(right.len());
    for frame in 0..frames {
        bus_left[frame] += left[frame];
        bus_right[frame] += right[frame];
    }
}

/// 母线汇流：把一条轨的**单声道、声相之前**渲染结果按声相增益写进左右两声道。
///
/// `(gain_l, gain_r)` 由 [`crate::mixer::pan_gains`] 在**构造期**算出
/// （`cos`/`sin` 属超越函数类，不进实时路径），实时侧这里只做两次乘加。
/// 居中（默认律）时 `(√2/2, √2/2)` ⇒ 每声道 −3.01 dB，与 `yeban-mcp` 的离线渲染同口径。
///
/// ⭐ 任务单 `#154` 之后本函数是**快路**：只有"一条直连 master、单位增益的出边"
/// 才走它（逐位等价于接线前的求和）。有发送/多目的地/边增益的轨走
/// [`render_panned_into`] + [`route_panned`]。
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

/// ⭐ 把一条轨的**声相之后**立体声写进暂存（**赋值**，不是累加）—— 任务单 `#154`。
///
/// 用途：一条轨有**多条**出边（发送 + 直连）或边增益时，声相只算一次、然后按边
/// 分别搬运（多次搬运用 [`route_panned`]）。`gain_l`/`gain_r` 与 [`sum_into_bus`]
/// **同一把尺子**（构造期 `cos/sin`）。
///
/// 实时约束：逐样本两次乘，零分配、零锁、零 I/O [MUST-GATE-001]。
fn render_panned_into(left: &mut [f32], right: &mut [f32], mono: &[f32], gain_l: f32, gain_r: f32) {
    for ((l, r), m) in left.iter_mut().zip(right.iter_mut()).zip(mono) {
        *l = *m * gain_l;
        *r = *m * gain_r;
    }
}

/// 与 [`render_panned_into`] 同形，但两个增益**逐帧**从各自的平滑器里取
/// （声相自动化，裁决 P4=(b)）。每帧每个平滑器**恰好推进一次** ⇒ 多目的地不会
/// 让平滑器按目的地数量加速。
fn render_panned_into_smoothed(
    left: &mut [f32],
    right: &mut [f32],
    mono: &[f32],
    gain_l: &mut ParamSmoother,
    gain_r: &mut ParamSmoother,
) {
    for ((l, r), m) in left.iter_mut().zip(right.iter_mut()).zip(mono) {
        *l = *m * gain_l.process();
        *r = *m * gain_r.process();
    }
}

/// ⭐ **按边搬运**：把暂存里的立体声按每条出边的增益加到目的地（任务单 `#154`）。
///
/// * 目的地 = `master` ⇒ 加进母线块 `out`；
/// * 目的地 = 另一条可渲染节点 ⇒ 加进它的**总线暂存块**（`buses[slot]`）；
/// * 目的地不在计划里（例如 `routing_graph.nodes` 里没有对应音轨的节点）⇒ 跳过
///   （没有可累积的实体；本片不发明"虚拟总线"）。
///
/// 逐样本一次"乘 + 加"，零分配、零锁、零 I/O [MUST-GATE-001]。
fn route_panned(
    routes: &[RouteTarget],
    routing: &RoutingPlan,
    master: EntityId,
    source_left: &[f32],
    source_right: &[f32],
    out: &mut AudioBlock<DEFAULT_BLOCK_FRAMES>,
    buses: &mut [AudioBlock<DEFAULT_BLOCK_FRAMES>; MAX_TRACK_SLOTS],
) {
    for route in routes {
        let gain = route.gain();
        if route.destination() == master {
            let (left, right) = out.stereo_mut();
            for ((l, r), (sl, sr)) in left
                .iter_mut()
                .zip(right.iter_mut())
                .zip(source_left.iter().zip(source_right))
            {
                *l += *sl * gain;
                *r += *sr * gain;
            }
        } else if let Some(index) = routing.slot_of(&route.destination()) {
            let (left, right) = buses[index].stereo_mut();
            for ((l, r), (sl, sr)) in left
                .iter_mut()
                .zip(right.iter_mut())
                .zip(source_left.iter().zip(source_right))
            {
                *l += *sl * gain;
                *r += *sr * gain;
            }
        }
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
    use crate::synth::NoteSchedule;
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

    /// ⭐ **同源对**：两棵快照共用**同一批 `EntityId`**、同一份图与轨道，
    /// 只有**延迟表**不同 ⇒ 只差 PDC 计划（⛔ 不是"两次随机生成"）。
    ///
    /// ⚠ 为什么必须同源：`EntityId::new()` 每次都不同 ⇒ 两次独立构造的快照
    /// 会在**轨道集合**上就不同，那样测出来的原因是 `DeviceChain`，⛔ 不是 `PdcPlan`。
    fn simple_pair_with_latency(
        revision: u64,
        latency_samples: u32,
    ) -> (EngineSnapshot, EngineSnapshot) {
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
        let build = |latencies: &LatencyTable| {
            EngineSnapshot::from_parts(
                revision,
                48_000,
                DEFAULT_BLOCK_FRAMES,
                2,
                master,
                tracks.clone(),
                &routing,
                latencies,
            )
            .expect("合法图")
        };
        let mut shifted = LatencyTable::new();
        shifted.set(track, latency_samples);
        (build(&LatencyTable::new()), build(&shifted))
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

    // -----------------------------------------------------------------------
    // [ARCH-RT-005 族 / A2] 量化**场景启动**：`launch_quantization` ＋ 场景 tempo 覆盖。
    //
    // ⭐ 与试听（`AuditionBranch`）同一形状：控制路径武装"绝对生效帧"，实时路径每块只做
    // 整数减法＋一次比较（零分配/零锁/零 I-O [MUST-GATE-001]）。
    // ⭐ 四条判据**各自能独立变色**：① 到点生效 ② `Off` 立即（控制臂）③ 取消 ④ 恰好一次。
    // -----------------------------------------------------------------------

    /// 4/4 拍（测试用的拍号；引擎的快照里没有拍号 ⇒ 由控制面给）。
    fn common_time() -> yeban_model::TimeSignature {
        yeban_model::TimeSignature {
            numerator: 4,
            denominator: 4,
        }
    }

    /// 跑到 `pending_scene_launch` 生效（或量子预算耗尽）⇒ 返回实际用的量子数。
    fn run_until_launch(rig: &mut Rig, budget: usize) -> usize {
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for used in 1..=budget {
            rig.runtime.process_quantum(&mut out, 2);
            if rig.runtime.stats().scene_launches_applied > 0 {
                return used;
            }
        }
        budget
    }

    /// ⭐ 判据 ①（**到点生效**）：排队后**不**立刻改速度；跨过生效帧后 tempo 变成该场景的。
    ///
    /// ⭐ 独立变色：把 `queue_scene_launch` 里的 `next_bar_tick` 换成"当前帧" ⇒ 第一条
    /// 断言（排队后立刻仍是原速度）会红。
    #[test]
    fn a_queued_scene_launch_applies_its_tempo_at_the_bar_boundary() {
        let mut rig = rig();
        let before = rig.runtime.transport().bpm();
        let scene = Some(EntityId::new());
        // 场景的 tempo 覆盖与工程速度**明显不同** ⇒ 生效与否一眼可辨。
        let scene_bpm = before + 40.0;
        rig.runtime.queue_scene_launch(
            scene,
            scene_bpm,
            yeban_model::LaunchQuantization::Bar,
            common_time(),
        );
        assert!(
            rig.runtime.has_pending_scene_launch(),
            "排队后必须有待生效项"
        );
        assert_eq!(
            rig.runtime.transport().bpm(),
            before,
            "⭐ 量化档不是 Off ⇒ 排队**不得**立刻改速度"
        );
        assert_eq!(rig.runtime.stats().scene_launches_applied, 0);

        let used = run_until_launch(&mut rig, 2_000);
        assert!(
            used < 2_000,
            "一小节（120 bpm、48 kHz ⇒ 96 000 帧 ≈ 750 块）内必须生效"
        );
        assert!(
            (rig.runtime.transport().bpm() - scene_bpm).abs() < 1e-9,
            "⭐ 到点后 tempo 必须变成该场景的 {}（实测 {}）",
            scene_bpm,
            rig.runtime.transport().bpm()
        );
        assert_eq!(rig.runtime.stats().scene_launches_applied, 1, "⭐ 恰好一次");
        assert!(
            !rig.runtime.has_pending_scene_launch(),
            "⭐ 生效后状态机必须回到 None（幂等）"
        );
    }

    /// ⭐ 判据 ②（**控制臂**）：`Off` 档 ⇒ **立即**生效（第一块内）。
    ///
    /// ⭐ 独立变色：若把 `Off` 也送进量化分支 ⇒ 这一条会红。它证明判据 ① 的红
    /// **是因为量化**，⛔ 不是因为启动机制本身不工作。
    #[test]
    fn an_off_quantised_launch_takes_effect_immediately() {
        let mut rig = rig();
        let scene_bpm = rig.runtime.transport().bpm() + 25.0;
        rig.runtime.queue_scene_launch(
            Some(EntityId::new()),
            scene_bpm,
            yeban_model::LaunchQuantization::Off,
            common_time(),
        );
        // ⭐ 一个量子就够（Off ⇒ 生效帧 = 当前帧 ⇒ 第一块内触发）。
        let used = run_until_launch(&mut rig, 1);
        assert_eq!(used, 1, "⭐ Off ⇒ 第一个量子内必须生效");
        assert!((rig.runtime.transport().bpm() - scene_bpm).abs() < 1e-9);
        assert_eq!(rig.runtime.stats().scene_launches_applied, 1);
    }

    /// ⭐ 判据 ③（**取消**）：排队后、**未到点**取消 ⇒ ⛔ 永不触发、tempo 不变。
    ///
    /// ⭐ 独立变色：删掉 `cancel_scene_launch` 的清零 ⇒ 这一条会红（会照常触发）。
    #[test]
    fn a_cancelled_scene_launch_never_fires() {
        let mut rig = rig();
        let before = rig.runtime.transport().bpm();
        rig.runtime.queue_scene_launch(
            Some(EntityId::new()),
            before + 40.0,
            yeban_model::LaunchQuantization::Bar,
            common_time(),
        );
        assert!(rig.runtime.has_pending_scene_launch());
        rig.runtime.cancel_scene_launch();
        assert!(
            !rig.runtime.has_pending_scene_launch(),
            "⭐ 取消后不得再有待生效项"
        );
        // 跑够**两倍**小节长度（足以让未取消的那次生效两次）。
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..1_600 {
            rig.runtime.process_quantum(&mut out, 2);
        }
        assert_eq!(
            rig.runtime.stats().scene_launches_applied,
            0,
            "⭐ 取消过的启动⛔ 一次都不许触发"
        );
        assert_eq!(
            rig.runtime.transport().bpm(),
            before,
            "⭐ 取消后 tempo 必须一字不变"
        );
    }

    /// ⭐ 判据 ④（**恰好一次**）：跨过生效点后继续跑很久 ⇒ 计数仍是 1（⛔ 不重复触发）。
    ///
    /// ⭐ 独立变色：若触发后不清 `pending_launch` ⇒ 每块都会 `retune` 且计数暴涨。
    #[test]
    fn a_scene_launch_fires_exactly_once() {
        let mut rig = rig();
        let scene_bpm = rig.runtime.transport().bpm() + 30.0;
        rig.runtime.queue_scene_launch(
            Some(EntityId::new()),
            scene_bpm,
            yeban_model::LaunchQuantization::Bar,
            common_time(),
        );
        let used = run_until_launch(&mut rig, 2_000);
        assert!(used < 2_000);
        let after_first = rig.runtime.stats().scene_launches_applied;
        assert_eq!(after_first, 1);
        // ⭐ 继续跑**再一个多小节** ⇒ 计数必须纹丝不动、tempo 不变。
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..1_200 {
            rig.runtime.process_quantum(&mut out, 2);
        }
        assert_eq!(
            rig.runtime.stats().scene_launches_applied,
            1,
            "⭐ 生效一次之后⛔ 不得再触发"
        );
        assert!((rig.runtime.transport().bpm() - scene_bpm).abs() < 1e-9);
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
        // ⚠ **CRLF 脆弱点**（`windows` 腿实测红过的一族）：`include_str!` 把文件**按字节
        // 原样**嵌进来 ⇒ Windows runner 的 `core.autocrlf=true` 让工作树里 `lib.rs` 变成
        // CRLF 时，嵌进来的也是 CRLF，而下面那根针里带的是 **LF** ⇒ `contains` 在 Windows 上
        // **必然为假**（Linux 腿的工作树是 LF ⇒ 只有 `windows` 腿红）。
        // ⇒ **读入后立刻按"两侧同口径"归一**，照 `yeban-model/src/project.rs` 的范式
        // （`replace("\r\n", "\n")` 治成对 CRLF，`replace('\r', "\n")` 治**孤立 CR**）。
        // ⚠ 归一必须做在**读入的文本**上，⛔ 不是做在**拼好的针**上 —— 针是运行期拼的，
        // 只归一针会掩盖"文件里到底写了什么"。
        let lib = include_str!("lib.rs")
            .replace("\r\n", "\n")
            .replace('\r', "\n");
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
        .with_schedules(schedules, 0)
    }

    /// 从一个给定快照装一台运行时（事件通道容量 64）。
    /// 判据（⭐ **首块 tick 0 的环音符必须发声**）[ARCH-RT-001]：⭐ 在**第一个量子之前**
    /// 把一条 `NoteOn` 推进环 ⇒ ⭐ 它必须**真的触发**、⭐ **出声**、⭐ 且**不被记成孤儿松键**。
    ///
    /// ⚠ **前置事实（⭐ 不靠嘴说）**：⭐ 修法之前，⭐ 首块在快照武装**之前**出队 ⇒ ⭐ 声部槽
    /// 还不存在 ⇒ ⭐ `SynthEngine::note_on` 早退返回 `false` ⇒ ⭐ 三个读数分别是
    /// `poly_notes_triggered == 0`／⭐ 输出**全零**／⭐ `orphan_note_offs == 1`
    /// （⭐ app 线三条探针实测：⭐ 单条 `C4@tick0`）。⭐ 故本判据就是那条缺陷的**正式证据**。
    ///
    /// ⚠ **为什么要**对照组****：⭐ 夹具的快照里**本来就有已调度音符** ⇒ ⭐ 只断言
    /// `nonzero > 0` 会**靠已调度音符蒙过** ⇒ ⭐ 故本判据跑**同一夹具的两组**
    /// （⭐ 不推事件 / 推一条 `NoteOn`）⭐ 并比较**增量**：⭐ 必须**恰好**多触发一条。
    #[test]
    fn a_note_on_in_the_first_block_actually_sounds() {
        // 对照组：⭐ 同一夹具、⭐ **不**推事件。
        let (control_snapshot, _) = note_snapshot_with_pan(1, 0.0);
        let mut control = rig_with_snapshot(control_snapshot);
        let mut control_out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        control.runtime.process_quantum(&mut control_out, 2);
        let control_stats = control.runtime.stats();

        // 实验组：⭐ 同一夹具、⭐ 在**首块之前**推一条 `NoteOn`。
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        assert_eq!(
            rig.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 60,
                velocity: 100,
            }]),
            1,
            "事件必须进环"
        );
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let stats = rig.runtime.stats();

        assert_eq!(
            stats.poly_notes_triggered,
            control_stats.poly_notes_triggered + 1,
            "⭐ 首块的 `NoteOn` 必须**恰好**多触发一条（⭐ 修法前它是 0 增量：`note_on` 早退）\
             —— ⭐ 用增量而不是绝对量，⭐ 是为了排除'靠已调度音符蒙过'"
        );
        assert!(
            out.iter().any(|value| *value != 0.0),
            "⭐ 必须有非零输出（⭐ 修法前该量子的活音符是静音）"
        );
        assert_eq!(
            stats.orphan_note_offs, control_stats.orphan_note_offs,
            "⭐ 修法前这条 `NoteOn` 会被记成 1 个孤儿松键（`note_on` 早退、声部从未触发）"
        );
        assert_eq!(
            stats.events_applied,
            control_stats.events_applied + 1,
            "事件必须被计入"
        );
    }

    /// ⭐ 活音符夹具：⭐ **一条轨、⭐ 无已调度音符**（⭐ 于是"⭐ 出声"只能来自环事件）
    /// ＋ ⭐ 返回该轨 id（⭐ 事件要按 id 寻址）。⭐ 形态照 [`simple_snapshot`]。
    fn live_note_snapshot(revision: u64) -> (EngineSnapshot, EntityId) {
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
        .expect("合法图");
        (snapshot, track)
    }

    /// 判据（⭐ **端到端配对 ＋ 逐字节内容差异**）：⭐ **无事件 ⇒ 精确全零**；
    /// ⭐ **推一条 `NoteOn` ⇒ 出声**（⭐ 给 `peak` 读数）；⭐ 且两次渲染的**内容**必须不同。
    ///
    /// ⚠ ⭐ 装备是 [`live_note_snapshot`]（⭐ **无已调度音符**）⇒ ⭐ "⭐ 出声"**只可能**来自环事件
    /// ⇒ ⭐ 不存在"⭐ 靠已调度音符蒙过"。⭐ ⛔ 不比字节数：⭐ 比**内容**（⭐ 逐样本）。
    #[test]
    fn a_live_note_sounds_while_no_events_stays_exactly_silent() {
        // 对照组：⭐ 不推事件 ⇒ ⭐ **精确全零**。
        let (quiet_snapshot, _) = live_note_snapshot(1);
        let mut quiet = rig_with_snapshot(quiet_snapshot);
        let mut quiet_out = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        quiet.runtime.process_quantum(&mut quiet_out, 2);
        assert!(
            quiet_out.iter().all(|value| *value == 0.0),
            "⭐ 无事件 ⇒ 输出必须**精确全零**（⛔ 不是'接近零'）"
        );

        // 实验组：⭐ 首块之前推一条 `NoteOn` ⇒ ⭐ 出声。
        let (snapshot, track) = live_note_snapshot(1);
        let mut rig = rig_with_snapshot(snapshot);
        assert_eq!(
            rig.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 69,
                velocity: 127,
            }]),
            1
        );
        let mut loud_out = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut loud_out, 2);
        let peak = loud_out.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
        let nonzero = loud_out.iter().filter(|v| **v != 0.0).count();
        assert!(peak > 0.0, "⭐ 活音符必须出声");
        assert!(
            rig.runtime.stats().poly_notes_triggered > 0,
            "⭐ 必须真的触发了一条"
        );

        // ⭐ 内容差异（⭐ 逐样本，⛔ 不比字节数）。
        let first_diff = quiet_out
            .iter()
            .zip(loud_out.iter())
            .position(|(a, b)| a != b)
            .expect("两次渲染的内容必须不同 ⇒ 事件真的改变了输出");
        // ⚠ ⭐ 我第一版写的是 `first_diff == 0`，⭐ **实测是 68**（⭐ 起振前有若干零样本：
        // ⭐ 波表首个采样与包络起振都可以是 0）⇒ ⭐ 那是**我的期望值错**、⭐ 不是代码错。
        // ⭐ 故只要求"⭐ 差异在**一块之内**出现"（⭐ 即首块就生效），⭐ 并把实测值印进读数。
        assert!(
            first_diff < DEFAULT_BLOCK_FRAMES * 2,
            "⭐ 差异必须在本块内出现（⭐ 实测 {first_diff}）"
        );
        eprintln!(
            "DIAGNOSTIC 端到端配对: peak={peak} nonzero={nonzero} 差异起点={first_diff} \
             对照组峰值={}",
            quiet_out.iter().fold(0.0f32, |a, v| a.max(v.abs()))
        );
    }

    /// 判据（⭐ **松键有效 —— 听觉口径**）：⭐ `NoteOff` 之后那一声必须**衰减到静音**，
    /// ⭐ 而**不松键**的对照组在同一窗口里**仍在响** ⇒ ⭐ 成对 ⇒ ⭐ 排除"⭐ 挂音"与"⭐ 本来就静音"。
    ///
    /// ⚠ ⭐ ⛔ **不拿 `active_voices() == 0` 当证据**：⭐ 裁定 (B) 下 `u64::MAX` 让
    /// [`yeban_dsp::polysynth::PolySynth::retire_finished`] 的谓词永不成立；⭐ 但 ⭐
    /// [`yeban_dsp::polysynth::PolySynth::render`] 会在包络回到 `Idle` 时**当场回收**声部
    /// （⭐ 与 `end_sample` 无关）⇒ ⭐ 松键后的声部计数**会归零** ⇒ ⭐ ⭐ **计数归零也不能证明
    /// 松键有效**：⭐ 有效与否**只按听觉口径**判（⭐ 见下面的尾部窗口：`nonzero == 0`／RMS ≈ 0）。
    #[test]
    fn releasing_a_live_note_decays_to_silence_while_holding_keeps_sounding() {
        let quanta = 256usize; // ⭐ 远大于 3 × 默认 release（50 ms）对应的量子数
        // ⭐ 对照组：⭐ **不**发 `NoteOff` ⇒ ⭐ 尾部窗口必须**仍在响**。
        let (held_snapshot, held_track) = live_note_snapshot(1);
        let mut held = rig_with_snapshot(held_snapshot);
        assert_eq!(
            held.sender.publish(&[EngineEvent::NoteOn {
                track: held_track,
                pitch: 69,
                velocity: 127,
            }]),
            1
        );
        let mut held_tail = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..quanta {
            held.runtime.process_quantum(&mut held_tail, 2);
        }
        let held_peak = held_tail.iter().fold(0.0f32, |a, v| a.max(v.abs()));

        // ⭐ 实验组：⭐ 先响一段、⭐ 再 `NoteOff`、⭐ 再渲染同一长度的尾部窗口。
        let (snapshot, track) = live_note_snapshot(1);
        let mut rig = rig_with_snapshot(snapshot);
        assert_eq!(
            rig.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 69,
                velocity: 127,
            }]),
            1
        );
        let mut sounding = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..8 {
            rig.runtime.process_quantum(&mut sounding, 2);
        }
        let sounding_peak = sounding.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(sounding_peak > 0.0, "前置：⭐ 松键之前必须真的在响");
        assert_eq!(
            rig.sender
                .publish(&[EngineEvent::NoteOff { track, pitch: 69 }]),
            1
        );
        let mut tail = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..quanta {
            rig.runtime.process_quantum(&mut tail, 2);
        }
        let tail_peak = tail.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        let tail_rms = (tail
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>()
            / tail.len() as f64)
            .sqrt();
        eprintln!(
            "DIAGNOSTIC 松键口径: 响时峰值={sounding_peak} 松键后尾部峰值={tail_peak} \
             尾部RMS={tail_rms} 不松键对照峰值={held_peak} 声部数={}",
            rig.runtime.stats().poly_active_voices
        );
        assert!(
            tail_rms < 1.0e-3,
            "⭐ 松键后尾部必须衰减到静音（⭐ RMS={tail_rms}、⭐ 峰值={tail_peak}）"
        );
        assert!(
            held_peak > 0.0,
            "⭐ 不松键的对照组在同一窗口必须**仍在响**（⭐ 否则本判据可能是'本来就静音'）"
        );
        assert_eq!(
            rig.runtime.stats().orphan_note_offs,
            0,
            "⭐ 这条 `NoteOff` 必须找到声部（⛔ 不是孤儿）"
        );
    }

    /// 判据（⭐ **窃取读数** ＋ ⭐ "⭐ 被窃取的是**未松键**者"）：
    /// ⭐ 灌满池（16 条，⭐ pitch 互不相同 ⇒ ⛔ 不触发"同 pitch 重触发"）⇒ ⭐ `note_steals == 0`；
    /// ⭐ 第 17 条 ⇒ ⭐ **池满 ⇒ 必然窃取**；⭐ ⭐ **对照组**：⭐ 把全部音符**松键**、⭐ 等 release
    /// 走完（`PolySynth::render` 会在包络 `Idle` 时当场回收）⇒ ⭐ 再推一条 ⇒ ⭐ **不得**再窃取
    /// ⇒ ⭐ 即"⭐ 被窃取的是未松键者"这条预期**由读数核对**（⭐ ⛔ 不是靠推理）。
    #[test]
    fn stealing_only_happens_while_the_voices_are_still_held() {
        let (snapshot, track) = live_note_snapshot(1);
        let mut rig = rig_with_snapshot(snapshot);
        let mut out = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];

        // ⭐ 16 条正好填满池。
        for step in 0..16u8 {
            assert_eq!(
                rig.sender.publish(&[EngineEvent::NoteOn {
                    track,
                    pitch: 60 + step,
                    velocity: 100,
                }]),
                1
            );
        }
        rig.runtime.process_quantum(&mut out, 2);
        let filled = rig.runtime.stats();
        assert_eq!(filled.note_steals, 0, "⭐ 恰好填满 ⇒ 还不到窃取");
        assert_eq!(filled.poly_active_voices, 16, "⭐ 池应当是满的");

        // ⭐ 第 17 条 ⇒ ⭐ 池满 ⇒ ⭐ 必须窃取。
        assert_eq!(
            rig.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 80,
                velocity: 100,
            }]),
            1
        );
        rig.runtime.process_quantum(&mut out, 2);
        let stolen = rig.runtime.stats();
        assert!(
            stolen.note_steals > 0,
            "⭐ 第 17 条必须触发窃取（⭐ 读数 note_steals={}）",
            stolen.note_steals
        );

        // ⭐ 对照组：⭐ 全部松键 ⇒ ⭐ release 走完 ⇒ ⭐ `render` 回收 ⇒ ⭐ 再推一条**不该**窃取。
        let mut released = 0usize;
        for step in 0..16u8 {
            released += rig.sender.publish(&[EngineEvent::NoteOff {
                track,
                pitch: 60 + step,
            }]);
        }
        released += rig
            .sender
            .publish(&[EngineEvent::NoteOff { track, pitch: 80 }]);
        assert_eq!(released, 17, "⭐ 17 条松键都必须进环");
        let steals_before = rig.runtime.stats().note_steals;
        for _ in 0..64 {
            rig.runtime.process_quantum(&mut out, 2);
        }
        let after_release = rig.runtime.stats();
        assert_eq!(
            rig.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 81,
                velocity: 100,
            }]),
            1
        );
        rig.runtime.process_quantum(&mut out, 2);
        let final_stats = rig.runtime.stats();
        eprintln!(
            "DIAGNOSTIC 窃取: 池满时 note_steals={} 回收后声部数={} 回收后再推一条 note_steals={} \
             孤儿松键={}",
            stolen.note_steals,
            after_release.poly_active_voices,
            final_stats.note_steals,
            final_stats.orphan_note_offs
        );
        assert_eq!(
            final_stats.note_steals, steals_before,
            "⭐ 已松键的声部被 `render` 回收 ⇒ ⭐ 新音符⛔ 不该再窃取（⭐ 这就是\
             '被窃取的是未松键者'的对照读数）"
        );
    }

    /// 判据（⭐ **淡出窗口内的释放** ＋ ⭐ **提升之后的释放**，⭐ 两半各自成对）：
    /// ⭐ 第 17 条窃取第 1 条之后，⭐ **在提升点之前**把**新音符自己的** `NoteOff`
    /// 推进环 ⇒ ⭐ 提升完成、⭐ release 走完之后 ⭐ 被窃取的那个槽必须**不再发声**
    /// （⭐ `poly_active_voices` 从 16 回到 15：⭐ 被窃取者那条已回收，⭐ 其余 15 条
    /// 仍按住）；⭐ 后半把同一件事放在**提升点之后**再做一遍（⭐ 走 `note_off` 的
    /// **现频率**支路）。
    ///
    /// ## 合并口径（⭐ 与 [`EngineStats::orphan_note_offs`] 的字段文档同一条）
    ///
    /// ① ⭐ 合并计数**不区分**"⭐ 从未存在过"与"⭐ 已被窃取/⭐ 已换音"；
    /// ② ⭐ **被窃取音符的释放必然变成孤儿**：⭐ 窃取那一刻槽位改宣称新频率 ⇒
    ///    ⭐ 指向旧频率的 `NoteOff` 配不上任何声部；
    /// ③ ⭐ **淡出窗口内**槽位**仍宣称被窃取者的频率** ⇒ ⭐ 指向**旧**频率的释放
    ///    **仍然作用在被窃取者身上**（⭐ 不是孤儿）；⭐ 指向**新**频率的释放（⭐ (β)
    ///    之前）⭐ 既配不上旧频率、⭐ 又还没有新频率 ⇒ ⭐ 被记成孤儿、⭐ 新音符此后
    ///    **只能由后续松键**结束（⭐ 用户可见的挂音）。⭐ **(β) 之后**新音符自己的释放
    ///    **在窗口内也生效**（⭐ 器件把它记进 `pending_released`、⭐ 提升点补 `gate_off`）
    ///    ⇒ ⭐ 这类孤儿不再产生。
    ///
    /// ## 两半各自红在哪（⭐ 旧行为下的读数）
    ///
    /// * **前半（窗口内）**：⭐ 窃取量子之后 `fade_remaining = 144 − 128 = 16` 帧
    ///   （⭐ `steal_fade_frames_for(48_000) = 3 ms × 48 kHz = 144`、⭐ 量子 = 128 帧
    ///   ⇒ ⭐ 提升点落在**下一量子**第 16 帧）⇒ ⭐ 事件在量子**开头**应用、⭐ 提升发生在
    ///   渲染循环里 ⇒ ⭐ 此刻推的 `NoteOff` 在时间上**先于**提升点。⭐ 旧行为：
    ///   ⭐ 孤儿 +1、⭐ 声部**不**进 release ⇒ ⭐ 收尾时 `poly_active_voices = 16`
    ///   （⭐ 下面第一组收尾断言因此会红）。⭐ 新行为：⭐ 孤儿 +0、⭐ 收尾时 = 15。
    /// * **后半（提升之后）**：⭐ 窃取后**先空跑**两个量子让提升点走完，⭐ **再**推新音符
    ///   自己的 `NoteOff`。⭐ 旧行为：⭐ `freq_hz` 在提升点**没被写过**（⭐ 缺陷 1）⇒
    ///   ⭐ 槽位仍宣称**被窃取者**的频率 ⇒ ⭐ 孤儿 +1、⭐ 挂音（⭐ `poly_active_voices
    ///   = 16`）；⭐ 新行为：⭐ 现频率支路命中 ⇒ ⭐ 孤儿 +0、⭐ 收尾时 = 15。
    #[test]
    fn a_release_for_the_stolen_note_works_inside_the_fade_window_and_after_promotion() {
        // ⭐ 夹具：16 条**互不相同**的 pitch（⭐ 60..=75 ⇒ ⛔ 不触发"同 pitch 重触发"）。
        let fill = |rig: &mut Rig, track: EntityId| {
            for step in 0..16u8 {
                assert_eq!(
                    rig.sender.publish(&[EngineEvent::NoteOn {
                        track,
                        pitch: 60 + step,
                        velocity: 100,
                    }]),
                    1
                );
            }
        };
        let mut out = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];

        // ---- 前半：指向**新音符**的释放落在淡出窗口**内**（⭐ 提升之前） ----
        let (snapshot, track) = live_note_snapshot(1);
        let mut in_window = rig_with_snapshot(snapshot);
        fill(&mut in_window, track);
        in_window.runtime.process_quantum(&mut out, 2);
        assert_eq!(
            in_window.runtime.stats().poly_active_voices,
            16,
            "⭐ 前置：池必须刚好满"
        );
        // ⭐ 第 17 条 ⇒ 窃取；本量子只走完 128/144 帧淡出 ⇒ 还剩 16 帧。
        assert_eq!(
            in_window.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 80,
                velocity: 100,
            }]),
            1
        );
        in_window.runtime.process_quantum(&mut out, 2);
        let window_stolen = in_window.runtime.stats();
        assert!(
            window_stolen.note_steals > 0,
            "⭐ 第 17 条必须触发窃取（⭐ note_steals={}）",
            window_stolen.note_steals
        );
        assert_eq!(
            window_stolen.poly_active_voices, 16,
            "⭐ 淡出窗口里被窃取者仍占着槽（⭐ 新音符还没提升）"
        );
        // ⭐ 窗口内推**新音符自己的**释放。
        assert_eq!(
            in_window
                .sender
                .publish(&[EngineEvent::NoteOff { track, pitch: 80 }]),
            1
        );
        in_window.runtime.process_quantum(&mut out, 2);
        assert_eq!(
            in_window.runtime.stats().orphan_note_offs,
            0,
            "⭐ 窗口内的释放必须被挂起音符接住（⛔ 不是孤儿）"
        );
        // ⭐ 提升点在上一量子第 16 帧已经走过 ⇒ 再跑 release(50 ms = 2400 帧) 的余量。
        for _ in 0..64 {
            in_window.runtime.process_quantum(&mut out, 2);
        }
        let window_settled = in_window.runtime.stats();
        assert_eq!(
            window_settled.poly_active_voices, 15,
            "⭐ 被窃取的那一个槽必须已经回收（⭐ 15 = 仍按住的 15 条）"
        );
        assert_eq!(window_settled.orphan_note_offs, 0);

        // ---- 后半：同一件事放在提升点**之后**（⭐ 走现频率支路） ----
        let (snapshot, track) = live_note_snapshot(1);
        let mut after_promotion = rig_with_snapshot(snapshot);
        fill(&mut after_promotion, track);
        after_promotion.runtime.process_quantum(&mut out, 2);
        assert_eq!(
            after_promotion.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 80,
                velocity: 100,
            }]),
            1
        );
        after_promotion.runtime.process_quantum(&mut out, 2);
        // ⭐ 空跑两个量子：144 帧淡出走完（⭐ 提升点已过），⭐ 且**没有**窗口内的释放。
        for _ in 0..2 {
            after_promotion.runtime.process_quantum(&mut out, 2);
        }
        assert_eq!(
            after_promotion.runtime.stats().poly_active_voices,
            16,
            "⭐ 提升之后、松键之前新音符应当**在响**（⭐ 覆盖度自检：⛔ 不是早已静音）"
        );
        assert_eq!(
            after_promotion
                .sender
                .publish(&[EngineEvent::NoteOff { track, pitch: 80 }]),
            1
        );
        for _ in 0..64 {
            after_promotion.runtime.process_quantum(&mut out, 2);
        }
        let after_settled = after_promotion.runtime.stats();
        assert_eq!(
            after_settled.poly_active_voices, 15,
            "⭐ 提升之后的释放也必须能回收声部（⭐ 15 = 仍按住的 15 条）"
        );
        assert_eq!(after_settled.orphan_note_offs, 0);
        eprintln!(
            "DIAGNOSTIC 淡出窗口释放: 窗口内 孤儿={} 收尾声部={} / 提升后 孤儿={} 收尾声部={}",
            window_settled.orphan_note_offs,
            window_settled.poly_active_voices,
            after_settled.orphan_note_offs,
            after_settled.poly_active_voices
        );
    }

    /// 判据（⭐ **镜像过桥**，⛔ 不是"接线"）：⭐ 环音符路径上那两个计数
    /// （[`EngineStats::note_steals`] / [`EngineStats::orphan_note_offs`]）必须**经**
    /// [`EngineRuntime::stats_mirror`] 的跨线程只读镜像**读得回来**，⭐ 且与引擎侧
    /// [`EngineRuntime::stats`] 的读数**逐值一致**。
    ///
    /// ⚠ 为什么这条与"结构体里有个字段"不同：⭐ 设备腿上运行时归**音频回调线程**所有
    /// ⇒ ⭐ 控制面**只能**读镜像（[`EngineRuntime::stats`] 拿不到）。⭐ 一个字段写对了、
    /// 但 `publish` 漏存（或漏读了 `EngineStats`）⭐ 在直读路径上**看不出来** ——
    /// ⭐ 只有从镜像读、⭐ 与引擎侧对照，⭐ 才证明它**过了桥**。
    ///
    /// ⚠ 两个读数都必须在**本判据里被驱动成非 0**（⭐ 窃取一次 ＋ ⭐ 一次找不到声部的松键）
    /// ⇒ ⭐ 否则 `0 == 0` 的相等什么都证明不了（⭐ 本判据因此与"冷值相等"那条区分开）。
    #[test]
    fn the_ring_note_counters_cross_the_stats_mirror() {
        let (snapshot, track) = live_note_snapshot(1);
        let mut rig = rig_with_snapshot(snapshot);
        let mut out = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];

        // ⭐ 填满 16 个声部（⭐ pitch 互不相同 ⇒ ⛔ 不触发同 pitch 重触发）。
        for step in 0..16u8 {
            assert_eq!(
                rig.sender.publish(&[EngineEvent::NoteOn {
                    track,
                    pitch: 60 + step,
                    velocity: 100,
                }]),
                1
            );
        }
        rig.runtime.process_quantum(&mut out, 2);
        // ⭐ 第 17 条 ⇒ ⭐ 池满 ⇒ ⭐ 窃取 ⇒ ⭐ `note_steals` 非 0。
        assert_eq!(
            rig.sender.publish(&[EngineEvent::NoteOn {
                track,
                pitch: 80,
                velocity: 100,
            }]),
            1
        );
        rig.runtime.process_quantum(&mut out, 2);
        // ⭐ 一条指向**从未存在过**的音的松键 ⇒ ⭐ `orphan_note_offs` 非 0。
        assert_eq!(
            rig.sender
                .publish(&[EngineEvent::NoteOff { track, pitch: 90 }]),
            1
        );
        rig.runtime.process_quantum(&mut out, 2);

        let engine = rig.runtime.stats();
        let mirror = rig.runtime.stats_mirror().read();
        assert!(
            engine.note_steals > 0 && engine.orphan_note_offs > 0,
            "⭐ 前置：两个计数都必须在本判据里被驱动成非 0（⭐ 否则 0 == 0 什么都证明不了；\
             ⭐ note_steals={} orphan_note_offs={}）",
            engine.note_steals,
            engine.orphan_note_offs
        );
        eprintln!(
            "DIAGNOSTIC 镜像过桥: 引擎 note_steals={} 镜像={} / 引擎 orphan_note_offs={} 镜像={}",
            engine.note_steals,
            mirror.note_steals,
            engine.orphan_note_offs,
            mirror.orphan_note_offs
        );
        assert_eq!(
            mirror.note_steals, engine.note_steals,
            "⭐ `note_steals` 必须过桥：⭐ 镜像读数与引擎侧逐值一致"
        );
        assert_eq!(
            mirror.orphan_note_offs, engine.orphan_note_offs,
            "⭐ `orphan_note_offs` 必须过桥：⭐ 镜像读数与引擎侧逐值一致"
        );
    }

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
        // ⭐ **R95 测量窗**：本夹具的输出比状态**晚一个量子** ⇒ 事件生效的第一个量子
        // 交付的仍是**自动化之前**（居中）的音频。因此先跑一个**丢弃**的量子，
        // 之后测到的 `first` 才是"自动化已经反映在音频里"的那一量子。
        // （未后移时，③ 与比值循环都在读错位的音频 ⇒ 注入 p02b 会漏过。）
        let mut shifted = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut shifted, 2);
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

    /// 判据（**R76**）：**换主人**的那次快照武装会把**已经设好的**自动化目标
    /// 一并取消（`ParamSmoother::snap_to` 同时写 `value` 与 `target`，
    /// `yeban-dsp/src/smoothing.rs:165`）。
    ///
    /// 这条语义**钉住**而不是假装不存在，并且要求它**可见**（⛔ 不静默）：
    /// 1. `pan_automation_resets() == 1`（取消被计数）；
    /// 2. 换主人之后的渲染输出**居中**（左右逐位相同）⇒ 目标真的回到了**静态**值；
    /// 3. `pan_automation_frames()` 在换主人那一量子**不再增长**（该槽位已不在平滑路径上）。
    ///
    /// 顺序很重要：换主人必须发生在**已经武装过之后**（否则事件连槽位都找不到，
    /// 那是另一条判据 `a_pan_event_before_the_first_arming_is_counted_not_silent`）。
    #[test]
    fn a_snapshot_swap_that_changes_the_owner_cancels_the_target_and_counts_it() {
        let (snapshot_a, track_a) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot_a);
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);

        // 轨 A 上先设好自动化目标（硬左）。
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
        let mut armed_quantum = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut armed_quantum, 2);
        assert!(
            rig.runtime.pan_automation_frames() > 0,
            "夹具前提：先让平滑路径真的跑起来（否则谈不上被取消）"
        );
        let frames_before = rig.runtime.pan_automation_frames();

        // 换一份**另一条轨**（轨 B）的快照 ⇒ 槽位换主人 ⇒ `snap_to` 取消目标。
        let (snapshot_b, track_b) = note_snapshot_with_pan(2, 0.0);
        assert_ne!(track_a, track_b, "夹具前提：两条轨身份必须不同");
        rig.slot.publish(snapshot_b);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);

        assert_eq!(
            rig.runtime.pan_automation_resets(),
            1,
            "换主人取消已设目标必须被**计数**（R76：⛔ 不静默）"
        );
        assert_eq!(
            rig.runtime.pan_automation_frames(),
            frames_before,
            "换主人之后该槽位不再走平滑路径（帧数不得再增长）"
        );
        assert!(
            out.iter().step_by(2).any(|l| *l != 0.0),
            "夹具前提：这一量子必须真的在出声（否则上面的读数没有意义）"
        );
        // ⚠ **本判据刻意不断言"渲染回到居中"**：实测**不成立** ——
        // 换主人之后 `peak_l = 0.7496`、`peak_r = 0.3992`（仍是偏左的输出）。
        // ⛔ 不写没验证过的声明。这条不对称已登记为**未解释的开放发现**
        // （见本文件模块文档的 R76 条目）：要么静态声相表没在这次武装里生效，
        // 要么还有**另一条**给母线写 L/R 的路径。在查清之前，本判据只钉**可复现的两条读数**：
        // `pan_automation_resets() == 1`（取消被计数）与 `pan_automation_frames()` 不再增长。
    }

    /// 判据（**两侧必须各自生效**）：硬左让**右**声道归零，硬右让**左**声道归零。
    ///
    /// 为什么值得单独一条：R55 的判据只测了"硬左"（左目标 1.0、右目标 0.0）⇒
    /// 若**右**槽位的事件没生效（右目标留在静态 √½），硬左那一半照样会过。
    /// 本判据用**硬右**那一半把它拆穿（左目标 0.0）：右槽位若失效，左声道**不会**归零。
    #[test]
    fn both_pan_sides_take_effect_independently() {
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);

        // **硬右**：左目标 0.0、右目标 1.0。
        let events = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: 0.0,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
                value: 1.0,
            },
        ];
        assert_eq!(rig.sender.publish(&events), 2);
        for _ in 0..96 {
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let peak_l = out.iter().step_by(2).fold(0.0f32, |a, l| a.max(l.abs()));
        let peak_r = out
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0.0f32, |a, r| a.max(r.abs()));
        assert!(
            peak_r > 0.0,
            "夹具前提：右声道必须有声（实得 peak_r = {peak_r}）"
        );
        assert!(
            peak_l <= peak_r * 0.01,
            "硬右 ⇒ **左**声道必须归零（左/右峰值 = {peak_l} / {peak_r}）—— \
             不归零说明**左槽位的事件没生效**（右槽位失效时它不会归零）"
        );
    }

    /// 判据（**隔离右槽位**）：用**两个不同的右目标值**比较 L/R 峰值比。
    ///
    /// 为什么必须单独一条：硬左（R 目标 0）与硬右（L 目标 0）**各自都能在右槽位失效时
    /// 通过** —— 硬左时右声道照样单调下降（只是停在 √½ 而不是 0），硬右时比的是**左**声道。
    /// 本判据把右目标从 `0.5` 改到 `0.0`：右槽位若失效，比值会**始终停在居中（≈1.0）**，
    /// 第 1 段就会红。
    #[test]
    fn the_right_pan_slot_is_isolated_by_two_target_values() {
        let (snapshot, track) = note_snapshot_with_pan(1, 0.0);
        let mut rig = rig_with_snapshot(snapshot);
        let mut warm = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut warm, 2);

        let ratio_after = |rig: &mut Rig| -> f32 {
            for _ in 0..96 {
                let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
                rig.runtime.process_quantum(&mut out, 2);
            }
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            rig.runtime.process_quantum(&mut out, 2);
            let peak_l = out.iter().step_by(2).fold(0.0f32, |a, l| a.max(l.abs()));
            let peak_r = out
                .iter()
                .skip(1)
                .step_by(2)
                .fold(0.0f32, |a, r| a.max(r.abs()));
            if peak_l > 0.0 { peak_r / peak_l } else { 0.0 }
        };

        // 段 1：左 1.0、右 **0.5** ⇒ 比值必须停在 ≈0.5（右槽位失效 ⇒ 停在 ≈1.0）。
        let half = [
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: 1.0,
            },
            EngineEvent::SetParam {
                target: crate::ring::ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
                value: 0.5,
            },
        ];
        assert_eq!(rig.sender.publish(&half), 2);
        let ratio_half = ratio_after(&mut rig);
        assert!(
            (0.3..0.7).contains(&ratio_half),
            "右目标是 0.5 ⇒ 比值必须停在 ≈0.5（右槽位失效会停在 ≈1.0；实得 {ratio_half}）"
        );

        // 段 2：右目标改 **0.0** ⇒ 比值必须掉到 ≈0（证明**右**槽位的事件真的生效）。
        let zero = [EngineEvent::SetParam {
            target: crate::ring::ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
            value: 0.0,
        }];
        assert_eq!(rig.sender.publish(&zero), 1);
        // ⭐ **瞬态牙**：右目标 0.5 → 0.0 的**第一个**量子，比值必须
        // **严格大于 0 且严格小于 `ratio_half`** —— 阶跃（把 `process()` 换成 `target()`）
        // 会在这一量子直接把右声道置 0 ⇒ 比值 0 ⇒ 必红。
        let mut first = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut first, 2);
        let peak_l = first.iter().step_by(2).fold(0.0f32, |a, l| a.max(l.abs()));
        let peak_r = first
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0.0f32, |a, r| a.max(r.abs()));
        let ratio_first = if peak_l > 0.0 { peak_r / peak_l } else { 0.0 };
        assert!(
            ratio_first > 0.0 && ratio_first < ratio_half,
            "改目标的第一个量子必须**还在平滑**（0 < 比值 < {ratio_half}；阶跃会给 0，实得 {ratio_first}）"
        );
        let ratio_zero = ratio_after(&mut rig);
        assert!(
            ratio_zero < 0.1,
            "右目标改成 0.0 ⇒ 比值必须掉到 ≈0（实得 {ratio_zero}）"
        );
    }

    /// 片 1（[ARCH-RT-005]）的判据：**挂上第二份可渲染版本不改变任何输出**，
    /// 且第二读者连着**对的槽**。
    ///
    /// 本判据在**库内**（`cargo test -p yeban-engine --lib`）⇒ 本机可跑、可注入。
    /// ⚠ 零分配那一条**不在**这里：它需要计数型全局分配器，落在
    /// `tests/rt_zero_alloc.rs` 的场景 ㉔。
    #[test]
    fn attaching_an_audition_branch_changes_no_output_byte() {
        const QUANTA: u64 = 256;
        const PROPOSAL_REVISION: u64 = 7;
        let mut output = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];

        // ---- 对照臂：完全不挂第二份版本 ----
        let (main_snapshot, _) = note_snapshot_with_pan(1, 0.0);
        let mut control = rig_with_snapshot(main_snapshot);
        let mut control_samples: Vec<f32> = Vec::new();
        for _ in 0..QUANTA {
            control.runtime.process_quantum(&mut output, 2);
            control_samples.extend_from_slice(&output);
        }

        // ---- 实验臂：挂上第二份版本（`attach` 是**非实时**路径：它会取锚锁）----
        let (main_again, _) = note_snapshot_with_pan(1, 0.0);
        let mut armed = rig_with_snapshot(main_again);
        let (proposal, _) = note_snapshot_with_pan(PROPOSAL_REVISION, 0.0);
        let proposal_slot = SnapshotSlot::new(proposal);
        let (proposal_retire, _proposal_queue) = retire_channel(8);
        armed
            .runtime
            .attach_audition_branch(&proposal_slot, proposal_retire);
        let mut armed_samples: Vec<f32> = Vec::new();
        for _ in 0..QUANTA {
            armed.runtime.process_quantum(&mut output, 2);
            armed_samples.extend_from_slice(&output);
        }

        // ① 逐字节相同 —— 本片的核心承诺。
        assert_eq!(
            armed_samples.len(),
            control_samples.len(),
            "两条路的样本数必须相同"
        );
        // ⚠ 用 `to_bits()` 比，⛔ 不用 `==`：`==` 会把 `NaN` 判成不等、把 `-0.0` 与
        // `0.0` 判成相等 —— 两者都不是「逐字节」该有的语义。
        // ⚠ 也⛔ 不用 `assert_eq!` 直接比两个 `Vec<f32>`：那会把 32 768 个浮点数全打出来，
        // 失败信息反而不可读。这里只报**首个不同的样本下标**。
        let first_diff = armed_samples
            .iter()
            .zip(control_samples.iter())
            .position(|(armed, control)| armed.to_bits() != control.to_bits());
        assert!(
            first_diff.is_none(),
            "挂上第二份可渲染版本后输出必须**逐字节相同**；首个不同的样本下标 = {first_diff:?}\
             （已挂 {} 个样本 / 未挂 {} 个）",
            armed_samples.len(),
            control_samples.len()
        );
        // 冷对照：两条路都不是全零音频（否则「相同」什么也不证明）。
        assert!(
            control_samples.iter().any(|sample| *sample != 0.0),
            "对照臂的输出不能是全零（本夹具本来就有已调度音符）"
        );
        // ③ 在册见证：第二读者连到的是**第二份**槽。
        assert!(armed.runtime.has_audition_branch(), "第二份版本必须已在册");
        assert_eq!(
            armed.runtime.audition_held_revision(),
            Some(PROPOSAL_REVISION),
            "第二读者必须连着**第二份**快照（连错槽会读到别的修订号）"
        );
        // ⛔ 未挂的那条路必须如实报「没挂」。
        assert!(!control.runtime.has_audition_branch());
        assert_eq!(control.runtime.audition_held_revision(), None);
    }

    /// [ARCH-RT-005] 片 2a 的**自校准参照**：新路径与旧内联路径各一次，
    /// 供 `extracted_mix_path_matches_the_legacy_inline_path_bit_for_bit` 逐位比对。
    impl EngineRuntime {
        /// ⚠ **测试专用**：用**新路径**（`render_branch`）渲染**一个**量子，返回交错输出的**位**。
        ///
        /// 它只把 `render_block` 顶部的解构搬过来 + 调一次 `render_branch` ⇒ 与生产路径
        /// **同一个函数**，⛔ 不是第二份实现。
        #[cfg(test)]
        #[must_use]
        fn render_branch_once(&mut self, frames: usize, running: bool) -> Vec<u32> {
            let Self {
                snapshot,
                quanta,
                block,
                synth,
                transport,
                armed_pan_gains,
                control_change,
                pdc,
                params,
                bank,
                armed_strips,
                armed_insert_slots,
                reverb_pool,
                armed_reverbs,
                armed_reverb_slots,
                reverb_scratch,
                conv_pool,
                armed_convs,
                armed_conv_slots,
                conv_scratch,
                armed_pan_l,
                armed_pan_r,
                armed_pan_slots,
                armed_pan_armed,
                track_scratch,
                routing_buses,
                scratch_meters,
                insert_gain_reductions,
                insert_max_reduction_db,
                insert_current_reduction_db,
                insert_detector_level_db,
                insert_strip_frames,
                insert_reverb_frames,
                insert_convolution_frames,
                pan_automation_frames,
                meter_capacity_drops,
                ..
            } = self;
            let quantum = *quanta;
            // ⭐ 与生产路径同源：音频片段用它作**绝对帧原点**（任务单 `#135`）。
            let block_start_frame = transport.position_frames();
            let out = block;
            let current = match snapshot.begin_block() {
                Some(current) => current,
                None => return Vec::new(),
            };
            // ⚠ 本副本不使用主总线 id（另一同型副本用它）⇒ 不绑定，避免 `unused_variables`。
            let produced = 0usize;
            // 与 `render_block` 同源：本量子的声相增益表是 `armed_pan_gains` 的副本。
            let mut pan_gains = [(EntityId::default(), 1.0f32, 1.0f32); MAX_TRACK_SLOTS];
            pan_gains[..*armed_pan_slots].copy_from_slice(&armed_pan_gains[..*armed_pan_slots]);
            let mut share = BranchShare {
                pdc,
                params,
                control_change: &*control_change,
                bank,
                armed_strips,
                armed_insert_slots,
                reverb_pool,
                armed_reverbs,
                armed_reverb_slots,
                reverb_scratch,
                conv_pool,
                armed_convs,
                armed_conv_slots,
                conv_scratch,
                armed_pan_l,
                armed_pan_r,
                armed_pan_slots,
                armed_pan_armed,
                track_scratch,
                routing_buses,
                scratch_meters,
                insert_gain_reductions,
                insert_max_reduction_db,
                insert_current_reduction_db,
                insert_detector_level_db,
                insert_strip_frames,
                insert_reverb_frames,
                insert_convolution_frames,
                pan_automation_frames,
                meter_capacity_drops,
                pan_gains: &mut pan_gains,
            };
            let _ = render_branch(
                &mut share,
                synth,
                out,
                current,
                frames,
                running,
                quantum,
                block_start_frame,
            );
            let _ = produced;
            let (left, right) = out.stereo_mut();
            left.iter()
                .chain(right.iter())
                .map(|sample| sample.to_bits())
                .collect()
        }

        /// ⚠ **测试专用**：**抽取前**那条内联混音路径的**逐字副本** —— 自校准的**参照实现**。
        ///
        /// 只做了两处机械替换（`block` → `out`、`self.pdc` → `pdc`），其余与抽取前一字不差。
        /// ⛔ 不进生产代码、⛔ 不替代 `render_branch`：唯一目的是让「抽取没有改变数值行为」
        /// 这条断言**不依赖任何本机测得的常量**（否则判据会绑在**架构**上，而 `TEST-SPEC-003`
        /// 正是跨架构确定性门）。先例：`crc32` 的逐位参照判据。
        #[cfg(test)]
        #[must_use]
        #[allow(clippy::too_many_lines)]
        fn legacy_mix_once(&mut self, frames: usize, running: bool) -> Vec<u32> {
            let Self {
                snapshot,
                quanta,
                block,
                synth,
                armed_pan_gains,
                pdc,
                params,
                bank,
                armed_strips,
                armed_insert_slots,
                reverb_pool,
                armed_reverbs,
                armed_reverb_slots,
                reverb_scratch,
                conv_pool,
                armed_convs,
                armed_conv_slots,
                conv_scratch,
                armed_pan_l,
                armed_pan_r,
                armed_pan_slots,
                armed_pan_armed,
                track_scratch,
                scratch_meters,
                insert_gain_reductions,
                insert_max_reduction_db,
                insert_current_reduction_db,
                insert_detector_level_db,
                insert_strip_frames,
                insert_reverb_frames,
                insert_convolution_frames,
                pan_automation_frames,
                meter_capacity_drops,
                ..
            } = self;
            let quantum = *quanta;
            let out = block;
            let current = match snapshot.begin_block() {
                Some(current) => current,
                None => return Vec::new(),
            };
            let master = current.master();
            let mut produced = 0usize;
            // 与 `render_block` 同源：本量子的声相增益表是 `armed_pan_gains` 的副本。
            let mut pan_gains = [(EntityId::default(), 1.0f32, 1.0f32); MAX_TRACK_SLOTS];
            pan_gains[..*armed_pan_slots].copy_from_slice(&armed_pan_gains[..*armed_pan_slots]);
            out.silence();
            out.set_frames(frames);
            bank.begin_quantum();

            // --- 3a) 逐轨：渲染 → 电平 → 汇入母线 ---
            //
            // **走带冻结 ⇒ 输出静音、且不触发任何音符**（`notes` 契约见
            // `docs/ledger/engine-sound-notes.md` 的 needs N2）。电平仍然照常计量
            // （静音的读数），因此"每量子恰好一次批量发布"这条结构性契约与走带状态无关。
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
                        // ⭐ 与 `render_branch` **同一份** SFZ 接线（片 2a 的抽取等价判据
                        // 要求这条参照路径与抽取后的路径逐字节相同）。
                        current.sfz_plan(&track).map(|plan| plan.prepared()),
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
                    // 它把线性增益按需换算成 dB（`−compressor.gain_db()` ⇒ 一次 `log2`），
                    // 但不碰样本、不分配、不加锁（红线 7 不受影响）。
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
                pdc.apply(&track, &mut track_scratch[..frames]);
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
                        out,
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
                    sum_into_bus(out, &track_scratch[..frames], gain_l, gain_r);
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
            let _ = produced;
            let (left, right) = out.stereo_mut();
            left.iter()
                .chain(right.iter())
                .map(|sample| sample.to_bits())
                .collect()
        }
    }

    /// [ARCH-RT-005] 片 2a 的**自校准等价判据**：抽取后的 `render_branch` 与
    /// **测试专用的旧内联参照**在同一次运行、同一条夹具、**同一量子号**上逐位产出相同输出。
    ///
    /// ⭐ **为什么不钉哈希常量**：常量会把判据绑在**架构**上（浮点/库函数差异），
    /// 而 `TEST-SPEC-003` 是跨架构确定性门 ⇒ 参照必须在**运行内自算**。
    ///
    /// 两条路各跑在**同源** `EngineRuntime` 上（同一夹具、同样预热 ⇒ 武装状态与走带
    /// 位置逐字段相同），因此比的是「同一个输入经两条实现」的结果。
    ///
    /// 注入实测：把 `legacy_mix_once` 的 `out` 收尾改成 `scale_bus(out, 0.5)` ⇒ 本判据变红。
    #[test]
    fn extracted_mix_path_matches_the_legacy_inline_path_bit_for_bit() {
        const PREHEAT: u64 = 8;
        let frames = DEFAULT_BLOCK_FRAMES;

        let (new_snapshot, _) = note_snapshot_with_pan(1, 0.25);
        let (old_snapshot, _) = note_snapshot_with_pan(1, 0.25);
        let mut new_rig = rig_with_snapshot(new_snapshot);
        let mut old_rig = rig_with_snapshot(old_snapshot);
        let mut sink = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..PREHEAT {
            new_rig.runtime.process_quantum(&mut sink, 2);
            old_rig.runtime.process_quantum(&mut sink, 2);
        }

        let new_bits = new_rig.runtime.render_branch_once(frames, true);
        let old_bits = old_rig.runtime.legacy_mix_once(frames, true);
        assert_eq!(new_bits.len(), old_bits.len(), "两条路的样本数必须相同");
        // ⚠ 按 `to_bits()` 逐位比：`==` 会把 `-0.0` 判等、把 `NaN` 判不等。
        let first_diff = new_bits
            .iter()
            .zip(old_bits.iter())
            .position(|(new, old)| new != old);
        assert!(
            first_diff.is_none(),
            "抽取后的 `render_branch` 必须与旧内联参照**逐位相同**；\
             首个不同的样本下标 = {first_diff:?}（共 {} 个样本）",
            new_bits.len()
        );
        // 冷对照：⛔ 不能是「两边都是静音」的假相同。
        let peak = new_bits
            .iter()
            .map(|bits| f32::from_bits(*bits).abs())
            .fold(0.0f32, f32::max);
        assert!(peak > 0.1, "夹具必须真的出声（峰值 {peak}）");
    }

    // -----------------------------------------------------------------------
    // [ARCH-RT-005] 片 2b：**拓扑不一致必须拒绝试听**（受限版的硬附加条件）
    // -----------------------------------------------------------------------

    /// 共用的"两份版本 + 两个指纹"装配。
    fn topology_case(main: EngineSnapshot, proposal: EngineSnapshot) -> Option<AuditionRefusal> {
        audition_topology_mismatch(
            &main,
            &proposal,
            AutomationTargets::empty(),
            AutomationTargets::empty(),
        )
    }

    /// ⭐ **正对照**：拓扑完全一致 ⇒ **不**拒绝。
    ///
    /// ⚠ 没有这一条，一个"永远返回 `Some(…)`"的实现在下面四条负判据上**全过**
    /// ⇒ 那四条证明不了"拒绝是**有判断的**"。
    #[test]
    fn audition_accepts_identical_topology() {
        let main = simple_snapshot(1);
        let proposal = main.clone();
        assert_eq!(topology_case(main, proposal), None);
    }

    /// ① **音频配置**不一致（这里用声相律）⇒ 拒绝。
    #[test]
    fn audition_refuses_when_audio_config_differs() {
        let main = simple_snapshot(1);
        let proposal = main.clone().with_pan_law(PanLaw::Linear);
        // ⭐ 证明这条夹具**只**差音频配置（其余维度逐项相同）。
        assert_eq!(main.tracks(), proposal.tracks());
        assert_eq!(main.inserts(), proposal.inserts());
        assert_eq!(main.pdc(), proposal.pdc());
        assert_ne!(main.pan_law(), proposal.pan_law());
        assert_eq!(
            topology_case(main, proposal),
            Some(AuditionRefusal::AudioConfig)
        );
    }

    /// ② **设备链**不一致（这里用轨道集合：多一条轨）⇒ 拒绝。
    #[test]
    fn audition_refuses_when_device_chain_differs() {
        let main = simple_snapshot(1);
        let proposal = snapshot_with_master_track(1, 2);
        // ⭐ 音频配置**相同** ⇒ 报出来的原因只可能是设备链。
        assert_eq!(main.sample_rate(), proposal.sample_rate());
        assert_eq!(main.channels(), proposal.channels());
        assert_eq!(main.pan_law(), proposal.pan_law());
        assert_ne!(main.tracks().len(), proposal.tracks().len());
        assert_eq!(
            topology_case(main, proposal),
            Some(AuditionRefusal::DeviceChain)
        );
    }

    /// ③ **自动化目标**指纹不一致 ⇒ 拒绝（快照本身完全相同）。
    #[test]
    fn audition_refuses_when_automation_targets_differ() {
        let main = simple_snapshot(1);
        // ⭐ 两份**快照完全相同**（只有指纹不同）⇒ 原因只能是自动化目标。
        let proposal = main.clone();
        let armed = AutomationTargets::from_sorted(&[(EntityId::new(), 1)]);
        assert_eq!(
            audition_topology_mismatch(&main, &proposal, AutomationTargets::empty(), armed),
            Some(AuditionRefusal::AutomationTargets)
        );
        let same = simple_snapshot(1);
        assert_eq!(topology_case(same.clone(), same), None);
    }

    /// ④ **PDC 计划**不一致（同一条轨多了延迟）⇒ 拒绝。
    #[test]
    fn audition_refuses_when_pdc_plan_differs() {
        let (main, proposal) = simple_pair_with_latency(1, 64);
        // ⚠ 先证明这条夹具**真的**只差 PDC（否则它可能先被别的检查拦住）。
        assert_eq!(main.sample_rate(), proposal.sample_rate());
        assert_eq!(main.channels(), proposal.channels());
        assert_eq!(main.pan_law(), proposal.pan_law());
        assert_ne!(main.pdc(), proposal.pdc(), "夹具必须真的差 PDC");
        assert_eq!(
            topology_case(main, proposal),
            Some(AuditionRefusal::PdcPlan)
        );
    }

    /// 拒绝原因有稳定短名（读数用）。
    #[test]
    fn audition_refusal_names_are_stable() {
        assert_eq!(AuditionRefusal::AudioConfig.as_str(), "audio-config");
        assert_eq!(AuditionRefusal::DeviceChain.as_str(), "device-chain");
        assert_eq!(
            AuditionRefusal::AutomationTargets.as_str(),
            "automation-targets"
        );
        assert_eq!(AuditionRefusal::PdcPlan.as_str(), "pdc-plan");
        assert_eq!(AuditionRefusal::NoMainSnapshot.as_str(), "no-main-snapshot");
    }

    // -----------------------------------------------------------------------
    // [ARCH-RT-005] 片 2b/2c：混合的**等功率**与**切换帧**，以及"空闲不变"
    // -----------------------------------------------------------------------

    /// ⚠ **未试听时输出逐字节不变**（防回退 2a / 2b-1 钉住的性质）。
    ///
    /// 两棵同源夹具：一棵**挂上**第二分支但**不开始试听**，一棵完全不挂。
    /// 输出必须逐位相同（`to_bits()`；⛔ 不用 `==`）。
    #[test]
    fn audition_idle_output_is_byte_identical() {
        const QUANTA: u64 = 64;
        let render = |attach: bool| -> (Vec<u32>, Option<u64>) {
            let (snapshot, _) = note_snapshot_with_pan(1, 0.0);
            let mut rig = rig_with_snapshot(snapshot);
            if attach {
                let (proposal, _) = note_snapshot_with_pan(7, 0.0);
                let slot = Arc::new(SnapshotSlot::new(proposal));
                let (retire, queue) = retire_channel(8);
                rig.runtime.attach_audition_branch(&slot, retire);
                // 队列必须活到判据结束（退役通道的另一端）。
                let _ = &queue;
            }
            let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
            let mut bits: Vec<u32> = Vec::new();
            for _ in 0..QUANTA {
                rig.runtime.process_quantum(&mut out, 2);
                bits.extend(out.iter().map(|s| s.to_bits()));
            }
            (bits, rig.runtime.audition_blocks_read())
        };
        let (attached, reads) = render(true);
        let (bare, _) = render(false);
        // ⭐ **覆盖度见证**：空闲时第二分支必须**一次都没被读**。
        //    没有这一条，本判据可能是空转：只要淡化表还没算出来，混合函数就会
        //    提前返回、不改一个样本 ⇒ "误入试听路径"也测不出来。
        assert_eq!(
            reads,
            Some(0),
            "未试听时第二分支**一次都不该被 `begin_block()`**"
        );
        let first_diff = attached.iter().zip(bare.iter()).position(|(a, b)| a != b);
        assert!(
            first_diff.is_none(),
            "未试听时输出必须逐字节不变；首个不同的样本下标 = {first_diff:?}"
        );
        // 冷对照：⛔ 不能是"两边都是静音"的假相同。
        let peak = attached
            .iter()
            .map(|b| f32::from_bits(*b).abs())
            .fold(0.0f32, f32::max);
        assert!(peak > 0.05, "夹具必须真的出声（峰值 {peak}）");
    }

    /// ⭐ **等功率**：淡化表逐项的 `淡出² + 淡入²` 与 1 的偏差 ≤ `1e-5`，**且不是线性**。
    ///
    /// ⚠ 只测等式**测不出线性**（线性曲线在中点也有 `0.5²+0.5²=0.5`，但那不是 1）
    /// ⇒ 必须**外加**"中点 ≈ 0.7071 且与线性值 0.5 相差 ≥ 0.2"这条反线性读数。
    #[test]
    fn audition_fade_table_is_equal_power_not_linear() {
        let (retire, _queue) = retire_channel(8);
        let slot = Arc::new(SnapshotSlot::new(simple_snapshot(1)));
        let mut branch = AuditionBranch::attach(&slot, retire);
        branch.prepare(48_000, 1_440, 0);
        let table = branch.fade_table();
        assert_eq!(table.len(), 1_441, "48 kHz / 30 ms ⇒ 1440 + 1");
        let mut worst = 0.0f32;
        for (out_gain, in_gain) in table {
            worst = worst.max((out_gain * out_gain + in_gain * in_gain - 1.0).abs());
        }
        assert!(worst <= 1e-5, "等功率偏差必须 ≤ 1e-5，实得 {worst}");
        let midpoint = table[720].0;
        assert!(
            (midpoint - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "中点必须 ≈ 0.7071，实得 {midpoint}"
        );
        assert!(
            (midpoint - 0.5).abs() >= 0.2,
            "中点与**线性**值 0.5 必须差 ≥ 0.2（证曲线不是线性），实得 {midpoint}"
        );
    }

    /// ⭐ **切换恰好发生在切换帧**：切换帧之前纯 A、之后按表过渡、`fade_len` 帧后纯 B。
    #[test]
    fn audition_blend_switches_exactly_at_the_switch_frame() {
        const FRAMES: usize = 16;
        let mut a = AudioBlock::new();
        a.set_frames(FRAMES);
        {
            let (left, right) = a.stereo_mut();
            for sample in left.iter_mut().chain(right.iter_mut()) {
                *sample = 1.0;
            }
        }
        let mut b = AudioBlock::new();
        b.set_frames(FRAMES);
        {
            let (left, right) = b.stereo_mut();
            for sample in left.iter_mut().chain(right.iter_mut()) {
                *sample = -1.0;
            }
        }
        // 长度为 5 的表（fade_len = 4）⇒ 切换帧 + 4 帧之后必须已经是纯 B。
        let table: Vec<(f32, f32)> = (0..=4)
            .map(|k| crate::mixer::crossfade_gains(k as f32 / 4.0))
            .collect();
        const UNTIL: u64 = 3;
        blend_equal_power(&mut a, &b, FRAMES, UNTIL, &table);
        let left: Vec<f32> = a.left()[..FRAMES].to_vec();
        for (index, sample) in left.iter().enumerate() {
            let index_u64 = index as u64;
            if index_u64 < UNTIL {
                assert_eq!(*sample, 1.0, "切换帧之前必须是纯 A（帧 {index}）");
            }
            if index_u64 == UNTIL {
                assert_eq!(*sample, 1.0, "切换帧本身仍是 A（表下标 0）");
            }
            if index_u64 == UNTIL + 4 {
                assert_eq!(*sample, -1.0, "`fade_len` 帧之后必须是纯 B（帧 {index}）");
            }
            if index_u64 > UNTIL && index_u64 < UNTIL + 4 {
                assert!(
                    *sample < 1.0 && *sample > -1.0,
                    "过渡区必须严格夹在两者之间（帧 {index} ⇒ {sample}）"
                );
            }
        }
    }

    /// ⭐ [ARCH-RT-005] 片 3：**预滚** —— 切换帧永远**不早于**"请求 + 预滚帧数"，
    /// 而且仍然落在**拍点**上（不够就顺延到**再下一拍**）。
    ///
    /// ⚠ 判据自己把走带**泵到预滚窗口内**（距下一拍 < 预滚），否则"顺延"永远不会被
    /// 触发 ⇒ 判据会变成空转。
    #[test]
    fn audition_preroll_defers_the_switch_to_a_later_beat() {
        let (snapshot, _) = note_snapshot_with_pan(1, 0.0);
        // ⚠ **必须克隆**做提案：`note_snapshot_with_pan` 每次都生成新的 `EntityId`
        //   ⇒ 两份独立构造的快照在**轨道集合**上就不同，会被判 `DeviceChain`
        //   （拓扑检查是对的：受限版共用武装表 ⇒ 轨道集合必须一致）。
        let proposal = snapshot.clone();
        let proposal_arc = Arc::new(proposal.clone());
        let mut rig = rig_with_snapshot(snapshot);
        let slot = Arc::new(SnapshotSlot::new(proposal));
        let (retire, queue) = retire_channel(8);
        rig.runtime.attach_audition_branch(&slot, retire);

        let preroll = crate::mixer::audition_preroll_frames(48_000.0) as u64;
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        let mut steps = 0u64;
        while rig.runtime.frames_until_audition_switch() >= preroll {
            rig.runtime.process_quantum(&mut out, 2);
            steps += 1;
            assert!(steps < 5_000, "夹具没能在合理步数内进入预滚窗口");
        }
        let until_before = rig.runtime.frames_until_audition_switch();
        assert!(
            until_before < preroll,
            "夹具必须落在预滚窗口内（实得 {until_before}，预滚 {preroll}）"
        );

        rig.runtime
            .begin_audition(
                &proposal_arc,
                AutomationTargets::empty(),
                AutomationTargets::empty(),
            )
            .expect("同源提案必须被接受");
        let stored = rig
            .runtime
            .audition_frames_until_switch()
            .expect("已挂第二分支");
        assert!(
            stored >= preroll,
            "切换必须**顺延**：存下的倒计时 {stored} 必须 ≥ 预滚 {preroll}（顺延前只有 {until_before}）"
        );
        assert_ne!(
            stored, until_before,
            "必须真的顺延到**再下一拍**，⛔ 不是把切换挪到拍与拍之间"
        );
        let _ = &queue;
    }

    /// 试听期某条轨的**峰值**读数（取该轨在若干个量子里的最大 `peak`）。
    fn run_and_peak(rig: &mut Rig, quanta: usize, track: EntityId) -> f32 {
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..quanta {
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut scratch = [MeterFrame::default(); 64];
        let drained = rig.collector.tick(&mut scratch);
        scratch[..drained]
            .iter()
            .filter(|frame| frame.node == track)
            .map(|frame| frame.peak)
            .fold(0.0f32, f32::max)
    }

    /// ⭐ [ARCH-RT-005] 裁决 **(甲)** 的**钉住**判据：试听期的电平读数**主语是后渲染那条（B）**。
    ///
    /// ⚠ **为什么必须让 B 与 A 可分辨**：如果两条分支同源（材料一样），它们的电平读数
    /// **逐位相同** ⇒ 这条判据**当前不可分辨**、就是一条空转判据。
    /// ⇒ 本判据的提案与主线**拓扑同源**（否则会被准入判定拒绝），但把该轨的**谱清空**
    /// ⇒ B 静音、A 有声 ⇒ 主语是谁**可观测**。
    ///
    /// ⛔ 若你要的是 (乙)/(丙) 的口径，先改那条裁决 —— 本判据会**如实变红**。
    #[test]
    fn audition_meter_reading_belongs_to_the_last_branch_rendered() {
        const QUANTA: usize = 8;
        let (main, track) = note_snapshot_with_pan(1, 0.0);

        // ① 平时（单分支、不试听）：读数主语是 A。
        let mut solo = rig_with_snapshot(main.clone());
        let peak_solo = run_and_peak(&mut solo, QUANTA, track);

        // ② 试听：拓扑同源 + **该轨谱清空** ⇒ B 静音。
        let mut schedules = main.schedules().clone();
        schedules.insert(track, NoteSchedule::from_sorted(Vec::new()));
        // 第 2 个参数是"被丢弃的音符数"（0 ⇒ 没有溢出）。
        let proposal = main.clone().with_schedules(schedules, 0);
        let proposal_arc = Arc::new(proposal.clone());
        let slot = Arc::new(SnapshotSlot::new(proposal));
        let (retire, queue) = retire_channel(8);
        let mut rig = rig_with_snapshot(main.clone());
        rig.runtime.attach_audition_branch(&slot, retire);
        rig.runtime
            .begin_audition(
                &proposal_arc,
                AutomationTargets::empty(),
                AutomationTargets::empty(),
            )
            .expect("拓扑同源（只差谱）⇒ 必须被接受");
        let peak_audition = run_and_peak(&mut rig, QUANTA, track);
        eprintln!(
            "试听期电平读数：[单分支 A] peak={peak_solo} / [试听期] peak={peak_audition} / \
             第二分支渲染块数={:?}",
            rig.runtime.audition_blocks_read()
        );

        assert!(
            peak_solo > 0.0,
            "基线必须**真的有信号**，否则本判据不可分辨（实得 {peak_solo}）"
        );
        assert_eq!(
            peak_audition, 0.0,
            "试听期读数的主语必须是**后渲染的 B**（其谱已清空 ⇒ 峰值必须为 0）；\
             实得 {peak_audition}（≠ 0 ⇒ 主语变成了 A，口径漂移了）"
        );
        let _ = &queue;
    }
    /// ⭐ **CC 事件面 ⇒ 引擎的会话运行态读数**（任务单 `#107`；判据 §7.1）。
    ///
    /// # 量什么（对象 ＋ 单位，先说再跑）
    ///
    /// 对象 ＝ [`EngineRuntime`] 的 CC 表；单位 ＝ `control_change(控制器) -> Option<u8>`
    /// 的读数（`None` ＝ 从未收到过，`Some(v)` ＝ 收到过且当前值为 `v`）。
    ///
    /// # 三条腿
    ///
    /// 1. **前**：`None`（⛔ 不是 `Some(0)`）＋ `control_change_armed() == false`；
    /// 2. **推进**：一批四条事件（两条同控制器 ⇒ **最后一条赢** ＋ 一条掩码过界）⇒
    ///    读数按预期变化，且 `events_applied` 把这个"事件到了"的事实钉住
    ///    （⇒ "CC 被静默丢弃"与"CC 生效了"在读数上可区分）；
    /// 3. **反向腿**：没喂过的控制器仍是 `None`（⛔ 不是 `Some(0)`）。
    #[test]
    fn control_change_events_advance_the_session_state() {
        let mut rig = rig();
        let mut output = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut output, 2);
        assert_eq!(
            rig.runtime.control_change(7),
            None,
            "① 一个事件都没喂 ⇒ None（⛔ 不是 Some(0)）"
        );
        assert!(!rig.runtime.control_change_armed());
        assert_eq!(rig.runtime.control_change_received_count(), 0);

        let batch = [
            EngineEvent::ControlChange {
                controller: 7,
                value: 100,
            },
            EngineEvent::ControlChange {
                controller: 10,
                value: 64,
            },
            // ⭐ 同控制器的第二条 ⇒ **最后一条赢**（同一批内的 FIFO）。
            EngineEvent::ControlChange {
                controller: 7,
                value: 32,
            },
            // ⭐ 越界字节：`0x87 & 0x7F = 7`、`0x80 & 0x7F = 0` ⇒ 覆盖 cc7 为 0。
            EngineEvent::ControlChange {
                controller: 0x87,
                value: 0x80,
            },
        ];
        assert_eq!(rig.sender.publish(&batch), 4, "四条事件必须全部写进环");
        rig.runtime.process_quantum(&mut output, 2);

        assert_eq!(
            rig.runtime.control_change(7),
            Some(0),
            "② 最后一条（掩码后 cc7 = 0）赢"
        );
        assert_eq!(rig.runtime.control_change(10), Some(64), "② 另一条控制器");
        assert_eq!(
            rig.runtime.control_change(11),
            None,
            "③ 反向腿：没喂过的控制器仍是 None"
        );
        assert_eq!(
            rig.runtime.control_change_received_count(),
            2,
            "② 收到过事件的控制器个数 = 2（7 与 10；掩码后的 7 不新增一个）"
        );
        assert!(rig.runtime.control_change_armed(), "② 表已 armed");
        assert_eq!(
            rig.runtime.stats().events_applied,
            4,
            "② 事件面：四条都到了（CC ⛔ 不静默丢弃 —— 它计入 events_applied）"
        );
    }
}
