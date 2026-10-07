//! `[MUST-GATE-001]` 的**运行期**断言：实时回调路径 **零分配 / 零释放 / 零锁等待 / 零阻塞 I/O**。
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.1 第 1 条）：
//!
//! > **音频线程零安全违规 (Zero Glitch / Zero Alloc)**：实时音频回调函数内严禁出现任何堆内存分配
//! > （`malloc`）、堆释放（`free`/`drop`）、文件/网络 I/O 或互斥锁争用，
//! > **CI 运行时通过内存分配 Hook 进行严格断言**。
//!
//! # 这条判据的四个分量（本文件逐分量都要证）
//!
//! | 分量 | 仪器 | 判据 |
//! | :--- | :--- | :--- |
//! | 堆分配 | 计数型全局分配器（**按线程**武装） | 窗口内 `allocations == 0` |
//! | 堆释放 | 同一个分配器的 `dealloc`/`realloc` | 窗口内 `deallocations == 0` |
//! | 锁争用 | [`yeban_engine::rt_probe::RtLockProbe`]（**见证型**：每量子做一次非阻塞试探） | 窗口内 `lock_blocking == 0 && lock_waits == 0` |
//! | 阻塞 I/O | [`yeban_engine::rt_probe::diag`]（实时路径上**唯一**的诊断/I-O 边界） | 窗口内 `io_requests == 0 && io_ops == 0` |
//!
//! 前两个分量自 `rt_zero_alloc` 的第一版起就有（10 000 量子 + 63 次快照交换）；
//! **本版补的是后两个**，并把场景从 2 个扩到 6 个（每个场景都断言**四元组全 0**）。
//!
//! # 为什么"没写锁"不等于"运行期零锁"（见证的必要性）
//!
//! `crates/yeban-engine/src/rt.rs` 的实时路径里本来就没有 `Mutex::lock`。但
//! "grep 不到锁"只是**源码形状**的证据，它无法回答"**这条路径运行起来到底碰没碰到锁**"——
//! 一个永远读 0 的计数器与"没有计数器"在读数上完全一样。因此探针必须**有牙**：
//!
//! 1. **位置见证**：`EngineRuntime::render_block` 每个量子调用一次
//!    [`rt_probe::quantum_enter`]，窗口内 `quanta_visits` **必须等于**本窗口处理的量子数
//!    （本文件每个场景都断言这条）⇒ 探针位置确实在**被执行到**的代码里；
//! 2. **能力见证**：同一次 `quantum_enter` 会对 `rt_probe::rt_path_lock()` 做一次
//!    `try_lock`（**不等待**），成功次数记进 `lock_try_successes` ⇒ 断言
//!    `lock_try_successes == quanta_visits` 即证明"这条路径**真的能操作那把锁**，
//!    因此在同一个窗口里 `lock_blocking == 0` 是**测出来的 0**；
//! 3. **正对照**：判据 ⑦ 在**非 RT 路径**上让同一个探针非 0（含"已被别的线程持有"的
//!    争用对照，观察到 `lock_waits == 1`），证明读数有判别力；
//! 4. **注入**：④ 组注入（见模块文档末尾）各自把判据打红后**逐字节还原**。
//!
//! # 十三个场景（在既有 `harness = false` 风格上扩展）
//!
//! | # | 场景 | 覆盖的实时路径 |
//! | :-: | :--- | :--- |
//! | ① | 纯渲染 10 000 量子（`filled_project`） | 事件批量出队 → 快照边界 → 逐轨合成/电平 → 母线限制器 → 两路 SPSC 发布 |
//! | ② | 快照交换 63 次逐步 + 1 000 次高频 | `SnapshotReader::begin_block` 的原子切换 + 旧快照入退役队列 |
//! | ③ | 走带 200 轮命令 + 2 000 量子播放 + 500 量子停住 | `EngineEvent::Transport` 出队应用、整数 tick 推进、`SeekTicks` 的声部释放、停住分支 |
//! | ④ | 电平计量 10 000 量子 + UI 侧 60Hz 抽干 | 每轨/母线电平状态机 + **每量子恰好一次**批量发布 |
//! | ⑤ | 自动化求值 2 000 量子（每量子一批 `SetParam`） | 控制侧 `automation_value_at` → SPSC → 实时侧出队（**见 §needs：实时侧只计数，不改 DSP**） |
//! | ⑥ | 限制器/混音链 2 000 量子（滤波器 + 声相 + **主总线推子** + 前瞻限制 + 声部窃取） | `BusLimiter::apply`、声相增益乘加、`scale_bus` 的主总线逐样本乘、声部窃取路径 |
//! | ⑬ | 回调缓冲长度边界：1/2/3/127/128/129/1024/1025 帧 × 25 轮 + 非帧对齐缓冲 | `process_quantum` 的**任意长度**切块与逐帧交错拷贝、尾部残余样本契约 |
//! | ⑭ | 采样率 × 项目声明 `block_size` 全组合切换（2 000 量子） | `render_block` 的**重新武装**分支：`MeterBank::set_quanta_per_second`、`SynthEngine::begin_snapshot`、`Transport::arm` |
//! | ⑮ | 播放中「`Op` 层编辑 / 撤销 → 重新发布快照」2 000 轮 + 主线程排空 | `Op::apply`/`apply_inverse` 的模型改写 → 快照重建 → 原子发布 → 切换 + 旧快照入退役队列 |
//! | ⑯ | 满批事件洪峰 128 条/量子 × 500 量子（参数 + 音符 + 走带混排） | `EventReceiver::drain_with` 的**满块**边界（`[EngineEvent; SCRATCH_EVENTS]`） |
//! | ⑰ | PDC 补偿延迟线：2 000 量子稳态 + 1 000 量子跨快照**重新武装**（32 → 96 帧） | `CompensationBank::rearm` 的 `set_delay` 分支 + 逐样本环形延迟读写（`ROAD-M2-004` 接线之后新增；行为判据在 `tests/pdc_mix_path.rs`） |
//! | ⑱ | 退役队列欠容（容量 1 + 控制面**故意**不排空）64 轮 | `SnapshotReader::retire_or_stash` 的 `PushError::Full` 分支 ⇒ `note_suppressed(SnapshotRetireStash)`（`N6` 选项 A） |
//! | ⑲ | 电平容量不足（轨道数 `SCRATCH_METERS + 44` = 300）100 量子 | `render_block` 的"轨道数 > 暂存槽 − 1"分支 ⇒ `note_suppressed(MeterCapacityDrop)`（`N6` 选项 A） |
//!
//! # 覆盖范围的**边界登记**（本判据没有覆盖什么，必须和"全 0"一起读）
//!
//! | 路径 | 状态 | 事实 |
//! | :--- | :--- | :--- |
//! | cpal 真回调线程 / 设备开流关流 | **未覆盖** | 需要 `device` feature（cpal）与一台有声卡的机器；本机按纪律不编译重依赖，托管 runner 无音频设备 ⇒ 登记为 needs，**不用** `NullBackend` 冒充 |
//! | 插件（VST3/CLAP）路径 | **不存在** | `yeban-plugin-host` / `yeban-vst` 是**故意空的**骨架（v2.0.0 阶段，见 `AGENTS.md` 附录 C.4）⇒ 没有路径可覆盖，不是缺口 |
//! | 采样器磁盘流式读 | **不存在** | `yeban-sfz` 尚未接入 `synth`（`synth` 目前是内置波表） |
//! | **退役队列欠容 / 电平容量溢出** | **已覆盖（纯计数）** | `N6` 裁决 = **选项 A** 已落地：这两条溢出分支（`SnapshotReader::retire_or_stash` 的 `PushError::Full` 与 `EngineRuntime::render_block` 的电平容量不足）**只调** `rt_probe::note_suppressed` —— 一个**不读** `SINK` 的纯计数出口（`crates/yeban-engine/src/rt_probe.rs`）。本文件新增两条场景把它们纳入覆盖集：⑱ 退役队列容量 1 + 控制面不排空；⑲ 轨道数 `SCRATCH_METERS + 44`。两条都断言**四元组全 0**（含 `io_requests == 0 && io_ops == 0`，而 witness sink 是**真的装着**的）**且** `suppressed_diag_events > 0`（那条出口真的被走到 ⇒ "零 I/O"是**跑出来的**，不是"没跑到"）。历史（选项 A 之前，**实测**）：这两条路径走的是 `diag`，在装了 sink 的判据二进制里实测 `io_requests == io_ops`（退役欠容 `400`、电平容量 `10000`）⇒ 当时它们**不在**本判据的覆盖集内，且"实时路径上从不产生诊断事件"**不成立** |
//!
//! # 为什么必须 `harness = false`（实测教训 L22）
//!
//! 计数工具是**进程全局**的，而 `#[global_allocator]` 每个二进制只能定义一次 ——
//! 第一版用 `#[test]` 写，CI 实测 `allocations=9 deallocations=3`：那不是实时路径在分配，
//! 而是 **libtest 自己在起线程/收结果**。那种假红比没有判据更坏。因此本目标关掉 libtest：
//! 进程里只有本判据自己的线程，数字无歧义。代价是它不出现在 libtest 汇总里 ——
//! 但 `cargo test --all-targets` **仍会构建并运行**它，并以**退出码**判定。
//!
//! 本版把分配计数器也改成**按线程**武装（`thread_local!` + `const` 初始化）：
//! 判据 ⑪ 要求"实时窗口**跨**一个外线程"的同时读数仍然无歧义，而全局武装会让
//! 外线程的分配被算进实时窗口（又一次"判据测错对象"）。
//!
//! 判据 ⑫（`line/engine-stats` 新增）把同一套仪器对准**控制面的读取路径**：
//! `EngineStats` 现在带着退役队列的原子量（`pending`/`drained`/释放线程归属），
//! 控制面要在 60Hz 循环里每帧读它 ⇒ 必须证明"读它也**不**引入分配/锁/I-O"，
//! 否则"控制面能看见"就要拿渲染路径来换。⑫b 同时实测**注入口径**：
//! `Vec::new()` 不分配（无效注入），`Vec::with_capacity(1)` 才分配（有效注入）。
//!
//! # 本判据怎么变红（八组注入，实测记录见 `docs/ledger/gate-rt-zero-alloc-notes.md` §4 与 §12）
//!
//! | # | 注入点（`crates/yeban-engine/src/`） | 变红的判据 |
//! | :-: | :--- | :--- |
//! | I1 | `rt.rs::render_block` 里加 `drop(rt_probe::rt_path_lock().lock());` | ①~⑥ 的**锁**分量 + ⑦ 的对照 |
//! | I2 | `rt.rs::render_block` 里加 `rt_probe::diag(RtDiagEvent::NoSnapshot);` | ①~⑥ 的 **I/O** 分量（`io_requests`/`io_ops`） |
//! | I3 | `rt.rs::render_block` 里加 `Vec::<u8>::with_capacity(1)` | ①~⑥ 的**分配/释放**分量 |
//! | I4 | 删掉 `render_block` 开头的 `rt_probe::quantum_enter()` | **位置见证**（`quanta_visits != 0` 那条）—— 探针被摘掉就"空转"，判据必须发现 |
//! | I5 | `process_quantum` 的 `frames < DEFAULT_BLOCK_FRAMES` 分支加一次 `Vec::with_capacity(1)` | **仅** ⑬（非整量子长度分支） |
//! | I6 | `render_block` 的重新武装分支里按 `sample_rate != 48_000` 加一次分配 | **仅** ⑭（采样率切换） |
//! | I7 | 同上的分支里按"有轨 `volume_db > 0`"（= 编辑已生效）加一次分配 | **仅** ⑮（播放中的编辑/撤销） |
//! | I8 | `events.drain_with` 的闭包里按 `applied == SCRATCH_EVENTS` 加一次分配 | **仅** ⑯（满批出队边界） |
//! | I9 | `rt.rs::render_block` 的电平容量不足分支把 `note_suppressed` **换回** `rt_probe::diag` | **仅** ⑲（溢出路径的 I/O 分量） |
//!
//! # I9 的实测记录（`N6` 选项 A 的验收证据；本节只在本文件里留档，账本由集成者补记）
//!
//! **注入**（`crates/yeban-engine/src/rt.rs` 电平容量不足分支加回一行
//! `rt_probe::diag(RtDiagEvent::MeterCapacityDrop);`）后逐字重跑本判据：
//!
//! ```text
//! [MUST-GATE-001] 判据 ⑲ FAIL [MUST-GATE-001] 电平容量不足走纯计数：四元组全 0（含 io_requests/io_ops）
//!              ⑲电平容量不足；四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=100 io_ops=100]；子窗口=1 探针经过=100（期望 100）试探成功=100 试探失败=0；纯计数诊断=100；…
//! [MUST-GATE-001] 判据汇总: 37 / 38 通过
//! [MUST-GATE-001] FAIL: 1 条判据未通过
//! ```
//!
//! 三条要点：① 变红的**只有** ⑲（溢出路径的 I/O 分量），⑲c 与其它 36 条不动；
//! ② `io_requests == io_ops == 100` = 100 个量子各一次真实写（sink 装着）；
//! ③ `alloc=0 dealloc=0` ⇒ 归因**干净**：红的原因是 I/O，不是分配。
//!
//! **还原**：`cp` 回注入前的副本后
//! `cmp crates/yeban-engine/src/rt.rs <备份>` ⇒ 无差异（exit 0），
//! `shasum -a 256` 两侧同为 `ad8b283b0feb87891205ccb02a8004824e19cb97ea748455ef9f99f0595f5358`；
//! 重跑本判据 ⇒ **38 / 38 通过**（退出码 0）。
//!
//! # 覆盖范围的诚实边界（**必须和读数一起读**）
//!
//! 探针**不是**系统调用级拦截（`docs/ledger/gate-rt-zero-alloc-notes.md` §5 有完整表）：
//!
//! - 锁探针只看得见走 [`RtLockProbe`] 的加锁。实时路径里**裸**写 `std::sync::Mutex::lock()`
//!   看不见（无争用时它只是原子 CAS，不进内核）；
//! - I/O 探针只看得见走 `rt_probe::diag` 的诊断。**裸** `println!`/`eprintln!`/`std::fs::*`
//!   看不见。本文件的"盲区证据 G1"会**实测**这一点（注入裸 `eprintln!` ⇒ 判据**不变红**），
//!   它是已知盲区，不是判据的牙；
//! - 不在覆盖内的还有：cpal/系统库内部的日志、OS 缺页与 `mmap`、`rtrb` 队列本身的原子操作
//!   （那是无锁 SPSC，不是锁）、以及尚未实现的采样器磁盘流式读。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::graph::{LatencyTable, PdcPlan};
use yeban_engine::meter::{MeterCollector, MeterFrame, SCRATCH_METERS, meter_channel};
use yeban_engine::ring::{
    DEFAULT_EVENT_CAPACITY, EngineEvent, EventSender, ParamAddress, SCRATCH_EVENTS,
    TransportCommand, event_channel,
};
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::rt_probe::{self, RtDiagEvent, RtDiagSink, RtLockProbe};
use yeban_engine::snapshot::{
    EngineSnapshot, RetireQueue, SnapshotSlot, TrackParams, retire_channel,
};
use yeban_engine::transport::TransportState;
use yeban_model::samples::filled_project;
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, BlockSize, CurveType,
    EntityId, Op, RoutingEdge, RoutingGraph, RoutingKind, SampleRate, StampedOp, TrackV3,
    YebanProjectV1,
};

mod support;

use support::{MixSpec, NoteSpec, note_project, tuned_project};

// ---------------------------------------------------------------------------
// 判据规模（改动这些常量就是改动判据强度，必须同步 notes）
// ---------------------------------------------------------------------------

/// ① 纯渲染窗口的量子数（沿用第一版 `[MUST-GATE-001]` 的规模）。
const PURE_QUANTA: u64 = 10_000;
/// ② 逐步交换次数（每次交换后处理 1 个量子）。
const SWAP_STEPWISE: u64 = 63;
/// ② 高频交换次数（每量子一发一换）。
const SWAP_BURST: u64 = 1_000;
/// ③ 走带命令轮数（`Stop`/`Play`/`SeekTicks`/`Play` 轮转）。
const TRANSPORT_ROUNDS: u64 = 200;
/// ③ 播放中的量子数。
const TRANSPORT_PLAY_QUANTA: u64 = 2_000;
/// ③ 停住后的量子数（时钟必须冻结、输出必须静音）。
const TRANSPORT_STOP_QUANTA: u64 = 500;
/// ④ 计量场景的量子数（每 5 个量子做一次 UI 抽干）。
const METER_QUANTA: u64 = 10_000;
/// ④ UI 抽干节拍（量子）。
const METER_TICK_QUANTA: u64 = 5;
/// ④ 单独测量的 UI 抽干次数。
const METER_DRAIN_ROUNDS: u64 = 1_000;
/// ⑤ 自动化场景的量子数（每量子一批 `SetParam`）。
const AUTOMATION_QUANTA: u64 = 2_000;
/// ⑥ 混音链场景的量子数。
const MIX_QUANTA: u64 = 2_000;
/// ⑫ 控制面读取 `EngineStats` 的窗口：每量子读一次（读数路径不许引入分配/锁/I-O）。
const STATS_QUANTA: u64 = 2_000;
/// ⑪ 外线程活动窗口内的量子数（窗口**跨**外线程的 3 次加锁 + 2 次真实 I/O）。
const ATTRIBUTION_QUANTA: u64 = 400;
/// 外线程的阻塞加锁次数（判据 ⑪ 要求它们落在 `foreign_*` 桶里）。
const FOREIGN_LOCKS: u64 = 3;
/// 外线程的诊断（真实 I/O）次数。
const FOREIGN_DIAGS: u64 = 2;
/// 线程握手的等待预算（超时 ⇒ 记失败而不是挂死）。
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(5);
/// 每量子 128 帧 @ 120 BPM / 960 PPQ / 48 kHz ⇒ 1 tick = 25 样本。
const SAMPLES_PER_TICK: u64 = 25;
/// ⑬ 回调缓冲的**边界长度**（帧）：真实 `cpal` 回调拿到的长度不保证是
/// `DEFAULT_BLOCK_FRAMES` 的倍数（设备协商结果可以是 1~几 k 帧）。
const CALLBACK_FRAME_EDGES: [usize; 8] = [1, 2, 3, 127, 128, 129, 1024, 1025];
/// ⑬ 非帧对齐的缓冲**样本**数：129 = 64 帧 + 1 个不属于任何完整帧的残余样本。
const CALLBACK_ODD_TAIL_SAMPLES: usize = 129;
/// ⑬ 每种长度各推几轮（覆盖"同一长度反复来"的稳态）。
const CALLBACK_ROUNDS: u64 = 25;
/// ⑭ 每个（采样率, 项目声明缓冲长度）组合下渲染的量子数。
const SAMPLE_RATE_QUANTA: u64 = 40;
/// ⑮ 播放中的"编辑 / 撤销 → 重新发布快照"轮数（= 发布次数 = 窗口数）。
const UNDO_CHURN_QUANTA: u64 = 2_000;
/// ⑮ 主线程排空退役队列的节拍（量子）：128 帧 @48 kHz ⇒ 5 量子 ≈ 13.3 ms ≈ 75 Hz，
/// 与规范要求的 60 Hz 同量级（且比它更密）。
const UNDO_CHURN_DRAIN_EVERY: u64 = 5;
/// ⑯ 洪峰持续量子数。
const FLOOD_QUANTA: u64 = 500;
/// ⑰ PDC 延迟线稳态窗口的量子数。
const PDC_QUANTA: u64 = 2_000;
/// ⑰ PDC **跨快照重新武装**窗口的量子数（覆盖 `rearm` 的 `set_delay` 分支）。
const PDC_REARM_QUANTA: u64 = 1_000;
/// ⑱ 故意不足的**退役队列容量**（条）：生产装配是 64（`Rig::new`）。
///
/// 取 1 是**实测过的**语义（`rt.rs::stats_flag_snapshot_lagging_when_the_reader_cannot_keep_up`）：
/// 容量 1 + 控制面不排空 ⇒ 恰好发生**一次**寄存，之后读者**停止切换**快照
/// （`begin_block` 见到 `stash.is_some()` 就直接用旧快照）。
const STASH_RETIRE_CAPACITY: usize = 1;
/// ⑱ 的快照交换轮数（每轮：窗口外发布 + 窗口内 1 个量子）。
const STASH_ROUNDS: u64 = 64;
/// ⑲ 的轨道数：**故意超过**电平暂存容量（`SCRATCH_METERS`）—— 母线要占一个槽，
/// 因此真正的预算只有 `SCRATCH_METERS - 1`。多出的 44 条是"每条量子被丢一次"的量。
const OVERSIZE_TRACKS: usize = SCRATCH_METERS + 44;
/// ⑲ 的量子数（每个量子都会在 `render_block` 的电平容量分支产生**一次**纯计数诊断）。
const METER_OVERFLOW_QUANTA: u64 = 100;

// ---------------------------------------------------------------------------
// 计数型全局分配器（**按线程**武装：判据 ⑪ 要在窗口里跑别的线程）
// ---------------------------------------------------------------------------

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

thread_local! {
    /// 本线程是否在观测窗口内（`const` 初始化 ⇒ 无惰性分配、无析构器）。
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

/// 本线程是否在分配观测窗口内（`try_with`：线程 teardown 期间绝不 panic）。
fn alloc_armed_here() -> bool {
    ARMED.try_with(Cell::get).unwrap_or(false)
}

// SAFETY: 每个方法都只是"按线程计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用状态（计数器是线程局部 `Cell`，访问不分配、不加锁）。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if alloc_armed_here() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: 由调用方保证 `layout` 合法（`GlobalAlloc` 的契约）。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if alloc_armed_here() {
            let _ = DEALLOCATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: 由调用方保证 `ptr` 来自本分配器且 `layout` 与分配时一致。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if alloc_armed_here() {
            // 重新分配在实时路径上同样禁止（可能搬迁并复制），因此也计入分配。
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// ---------------------------------------------------------------------------
// 四元组读数与观测窗口
// ---------------------------------------------------------------------------

/// `[MUST-GATE-001]` 的四元组（锁与 I/O 各带两个分量：尝试/等待、请求/实际发生）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Quad {
    /// 窗口内的堆分配次数（**必须为 0**）。
    allocations: u64,
    /// 窗口内的堆释放次数（**必须为 0**）。
    deallocations: u64,
    /// 窗口内的**阻塞式**加锁尝试（**必须为 0**）。
    lock_blocking: u64,
    /// 窗口内阻塞式加锁**真的等过**的次数（**必须为 0**）。
    lock_waits: u64,
    /// 窗口内进入 I/O 边界的次数（**必须为 0**）。
    io_requests: u64,
    /// 窗口内**真的转交给 sink**（真的发生 I/O）的次数（**必须为 0**）。
    io_ops: u64,
}

impl Quad {
    /// 四元组是否全 0。
    const fn is_zero(&self) -> bool {
        self.allocations == 0
            && self.deallocations == 0
            && self.lock_blocking == 0
            && self.lock_waits == 0
            && self.io_requests == 0
            && self.io_ops == 0
    }

    /// 累加（用于"多个子窗口合成一个场景"）。
    fn add(&mut self, other: &Self) {
        self.allocations = self.allocations.saturating_add(other.allocations);
        self.deallocations = self.deallocations.saturating_add(other.deallocations);
        self.lock_blocking = self.lock_blocking.saturating_add(other.lock_blocking);
        self.lock_waits = self.lock_waits.saturating_add(other.lock_waits);
        self.io_requests = self.io_requests.saturating_add(other.io_requests);
        self.io_ops = self.io_ops.saturating_add(other.io_ops);
    }

    /// 人读的一行。
    fn describe(&self) -> String {
        format!(
            "alloc={} dealloc={} lock_blocking={} lock_waits={} io_requests={} io_ops={}",
            self.allocations,
            self.deallocations,
            self.lock_blocking,
            self.lock_waits,
            self.io_requests,
            self.io_ops
        )
    }
}

/// 一个观测窗口的读数（四元组 + 探针见证量）。
#[derive(Clone, Copy, Debug, Default)]
struct Reading {
    /// 四元组。
    quad: Quad,
    /// 窗口内 `render_block` 经过探针的次数（**必须等于**本窗口处理的量子数）。
    visits: u64,
    /// 窗口内探针**成功**操作那把锁的次数（**必须等于** `visits`：探针真有牙）。
    try_successes: u64,
    /// 窗口内探针试探失败的次数（正常情况下 0：实时路径上无人持有那把锁）。
    try_failures: u64,
    /// 窗口内走 `rt_probe::note_suppressed`（**纯计数**的实时诊断出口）的次数。
    ///
    /// 它不是违规（那条出口到不了 sink），而是**见证**：溢出场景靠它区分
    /// "溢出分支真的被走到、而且没有 I/O"与"根本没跑到溢出分支"。
    suppressed: u64,
}

/// 在"必须四元组全 0"的窗口内执行 `body`。
///
/// 分配计数与探针窗口都是**线程局部**的：本线程之外的活动（判据 ⑪ 的外线程）
/// 物理上不可能被算进这个窗口。
fn window<F: FnOnce()>(body: F) -> Reading {
    let _ = ALLOCATIONS.try_with(|count| count.set(0));
    let _ = DEALLOCATIONS.try_with(|count| count.set(0));
    rt_probe::reset_current_thread();
    rt_probe::watch_current_thread();
    let _ = ARMED.try_with(|armed| armed.set(true));

    body();

    let _ = ARMED.try_with(|armed| armed.set(false));
    rt_probe::unwatch_current_thread();
    let probe = rt_probe::window_current_thread();
    Reading {
        quad: Quad {
            allocations: ALLOCATIONS.try_with(Cell::get).unwrap_or(0),
            deallocations: DEALLOCATIONS.try_with(Cell::get).unwrap_or(0),
            lock_blocking: probe.lock_blocking_attempts,
            lock_waits: probe.lock_waits,
            io_requests: probe.io_requests,
            io_ops: probe.io_ops,
        },
        visits: probe.quanta_visits,
        try_successes: probe.lock_try_successes,
        try_failures: probe.lock_try_failures,
        suppressed: probe.suppressed_diag_events,
    }
}

// ---------------------------------------------------------------------------
// 场景累加器与判据表
// ---------------------------------------------------------------------------

/// 一个场景的累计读数（由若干子窗口合成）+ 见证自检。
#[derive(Default)]
struct Scenario {
    label: &'static str,
    windows: u64,
    expected_quanta: u64,
    visits: u64,
    try_successes: u64,
    try_failures: u64,
    /// 窗口内走纯计数诊断出口的次数（见 [`Reading::suppressed`]）。
    suppressed: u64,
    quad: Quad,
    mismatches: Vec<String>,
    notes: Vec<String>,
}

impl Scenario {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            ..Self::default()
        }
    }

    /// 吸收一个子窗口，并断言"探针位置见证"：窗口里确实跑了 `expected_quanta` 个量子。
    fn absorb(&mut self, expected_quanta: u64, reading: &Reading) {
        self.windows += 1;
        self.expected_quanta = self.expected_quanta.saturating_add(expected_quanta);
        self.visits = self.visits.saturating_add(reading.visits);
        self.try_successes = self.try_successes.saturating_add(reading.try_successes);
        self.try_failures = self.try_failures.saturating_add(reading.try_failures);
        self.suppressed = self.suppressed.saturating_add(reading.suppressed);
        self.quad.add(&reading.quad);
        if reading.visits != expected_quanta {
            self.mismatches.push(format!(
                "子窗口 #{} 期望探针经过 {expected_quanta} 次，实际 {} 次",
                self.windows, reading.visits
            ));
        }
    }

    /// 记一条"覆盖度"证据（人读 + 防假绿）。
    fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// 见证自检是否成立：探针被跑到过、并且真的能操作那把锁。
    fn witness_ok(&self) -> bool {
        self.visits == self.expected_quanta
            && self.mismatches.is_empty()
            && self.try_successes == self.visits
            && self.try_failures == 0
    }

    fn detail(&self) -> String {
        format!(
            "{}；四元组[{}]；子窗口={} 探针经过={}（期望 {}）试探成功={} 试探失败={}；纯计数诊断={}；{}",
            self.label,
            self.quad.describe(),
            self.windows,
            self.visits,
            self.expected_quanta,
            self.try_successes,
            self.try_failures,
            self.suppressed,
            if self.notes.is_empty() {
                "（无附加证据）".to_owned()
            } else {
                self.notes.join("；")
            }
        )
    }
}

/// 判据表的一行。
struct Criterion {
    id: &'static str,
    title: &'static str,
    ok: bool,
    detail: String,
}

/// 判据收集器：所有判据都跑完再统一打印（一条红不掩盖后面的读数）。
#[derive(Default)]
struct Report {
    criteria: Vec<Criterion>,
}

impl Report {
    fn new() -> Self {
        Self::default()
    }

    fn assert(
        &mut self,
        id: &'static str,
        title: &'static str,
        ok: bool,
        detail: impl Into<String>,
    ) {
        self.criteria.push(Criterion {
            id,
            title,
            ok,
            detail: detail.into(),
        });
    }

    /// 普通场景共用的判定：四元组全 0 **且** 见证成立 **且** 连"纯计数诊断"都为 0。
    ///
    /// 最后那一条是 `N6` 选项 A 之后**收紧**的：普通场景里没有溢出 ⇒ 那条纯计数出口
    /// 根本不该被走到。溢出场景**不用**本方法（它们要求 `suppressed > 0`，见 ⑱/⑲）。
    fn scenario(&mut self, id: &'static str, title: &'static str, scenario: &Scenario) {
        let ok = scenario.quad.is_zero() && scenario.witness_ok() && scenario.suppressed == 0;
        self.assert(id, title, ok, scenario.detail());
    }

    fn failures(&self) -> usize {
        self.criteria.iter().filter(|c| !c.ok).count()
    }

    fn finish(self) -> ExitCode {
        let passed = self.criteria.iter().filter(|c| c.ok).count();
        let total = self.criteria.len();
        for criterion in &self.criteria {
            println!(
                "[MUST-GATE-001] 判据 {} {} {}",
                criterion.id,
                if criterion.ok { "PASS" } else { "FAIL" },
                criterion.title
            );
            println!("             {}", criterion.detail);
        }
        println!("[MUST-GATE-001] 判据汇总: {passed} / {total} 通过");
        if self.failures() == 0 {
            println!(
                "[MUST-GATE-001] ok: 十三场景（纯渲染 / 快照交换 / 走带 / 电平计量 / 自动化 / 混音链 / \
                 回调缓冲长度边界 / 采样率与声明缓冲切换 / 播放中编辑-撤销 / 满批事件洪峰 / \
                 PDC 补偿延迟线 / 退役队列欠容 / 电平容量不足）四元组全 0；两条溢出路径（N6 选项 A）\
                 走纯计数出口而**仍然** io_requests==0 && io_ops==0（且 suppressed_diag_events>0 ⇒ \
                 真的跑到了溢出）；控制面读取 EngineStats 的读取路径同样全 0；\
                 探针有牙（正对照 + 注入）；线程归属与外线程活动已对账"
            );
            ExitCode::SUCCESS
        } else {
            for criterion in self.criteria.iter().filter(|c| !c.ok) {
                eprintln!(
                    "[MUST-GATE-001] FAIL 判据 {} {}: {}",
                    criterion.id, criterion.title, criterion.detail
                );
            }
            eprintln!("[MUST-GATE-001] FAIL: {} 条判据未通过", self.failures());
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// 见证型 I/O 后端（**真的**写文件 + 打印；计数用）
// ---------------------------------------------------------------------------

/// 见证型诊断后端：每次 [`rt_probe::diag`] 都会**真的**写一个临时文件并打印一行。
///
/// 它存在的唯一理由：证明 `rt_probe::diag` 这个边界**有牙**（不是空转的计数器）。
/// 刻意**不分配**（`&File: Write` ⇒ 不需要锁；`stderr()` 的锁是 std 内部的），
/// 因此注入 I2 变红时"分配分量仍是 0"⇒ 变红**只能**归因于 I/O（归因干净）。
struct WitnessSink {
    file: std::fs::File,
    path: std::path::PathBuf,
    emits: AtomicU64,
    bytes: AtomicU64,
}

impl WitnessSink {
    fn emits(&self) -> u64 {
        self.emits.load(Ordering::SeqCst)
    }

    fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::SeqCst)
    }
}

impl RtDiagSink for WitnessSink {
    fn emit(&self, event: RtDiagEvent) {
        use std::io::Write;

        let line = event.as_str().as_bytes();
        // ① 真实文件写（阻塞式 `write` 系统调用）：`&File` 也是 `Write` ⇒ 不需要锁。
        let _ = (&self.file).write_all(line);
        let _ = (&self.file).write_all(b"\n");
        // ② 真实控制台打印（stderr 的阻塞式写）。
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(line);
        let _ = stderr.write_all(b"\n");
        drop(stderr);

        self.emits.fetch_add(1, Ordering::SeqCst);
        self.bytes
            .fetch_add((line.len() as u64 + 1) * 2, Ordering::SeqCst);
    }
}

/// 安装见证 sink（整个进程一次）；返回句柄供判据读计数。
///
/// 安装后立刻**真实地走一遍边界**（一次自检写）：这既是"边界通不通"的自检，
/// 也把首次加锁 / 首次写控制台的一次性开销挤到**任何窗口之外**
/// （macOS 上每个 `std::sync::Mutex` 实例的首次加锁会分配 64 字节，见 `rt_probe` 模块文档）。
fn install_witness_sink() -> Arc<WitnessSink> {
    let path = std::env::temp_dir().join("yeban-rt-zero-alloc-witness.log");
    let file = std::fs::File::create(&path).expect("见证 sink 必须能创建临时文件");
    let witness = Arc::new(WitnessSink {
        file,
        path,
        emits: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
    });
    let sink: Arc<dyn RtDiagSink> = witness.clone();
    if rt_probe::install_diag_sink(sink).is_err() {
        panic!("本进程只允许安装一次诊断出口");
    }
    rt_probe::diag(RtDiagEvent::NoSnapshot);
    witness
}

// ---------------------------------------------------------------------------
// 夹具装配
// ---------------------------------------------------------------------------

/// 判据用的引擎装配（快照槽 + 退役队列 + 事件生产端 + 电平消费者 + 渲染驱动）。
struct Rig {
    slot: Arc<SnapshotSlot>,
    queue: RetireQueue,
    sender: EventSender,
    collector: MeterCollector,
    runtime: EngineRuntime,
    output: Vec<f32>,
}

impl Rig {
    fn new(project: &YebanProjectV1, revision: u64, meter_capacity: usize) -> Self {
        Self::with_capacities(project, revision, meter_capacity, 64, 64)
    }

    /// 显式给出**全部四个**通道容量。
    ///
    /// 为什么需要它：⑯ 要把事件通道压到"每量子一整批（`SCRATCH_EVENTS`）"的边界，
    /// 而 [`Self::new`] 的 64 格装不下一批 128 条 —— 用默认容量测"满批出队"会
    /// 悄悄只写进去一半，判据却仍然全绿（那种绿什么也没证明）。
    fn with_capacities(
        project: &YebanProjectV1,
        revision: u64,
        meter_capacity: usize,
        retire_capacity: usize,
        event_capacity: usize,
    ) -> Self {
        let snapshot =
            EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译成快照");
        Self::from_snapshot(snapshot, meter_capacity, retire_capacity, event_capacity)
    }

    /// 用**已经构造好的快照**装配（⑲ 需要"轨道数超过电平暂存容量"的快照，
    /// 那不是任何真实工程投影得出来的 —— 走 [`EngineSnapshot::from_parts`] 直接造）。
    fn from_snapshot(
        snapshot: EngineSnapshot,
        meter_capacity: usize,
        retire_capacity: usize,
        event_capacity: usize,
    ) -> Self {
        let slot = SnapshotSlot::new(snapshot);
        let (retire, queue) = retire_channel(retire_capacity);
        let (sender, receiver) = event_channel(event_capacity);
        let (publisher, collector) = meter_channel(meter_capacity);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Self {
            slot,
            queue,
            sender,
            collector,
            runtime,
            output: vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2],
        }
    }

    /// 把输出缓冲准备到至少 `samples` 长（`resize` 会分配 ⇒ **只在窗口之外调用**）。
    fn reserve_output(&mut self, samples: usize) {
        if self.output.len() < samples {
            self.output.resize(samples, 0.0);
        }
    }

    /// 推一次**任意帧数**的回调缓冲（交错、2 声道）——
    /// 这是真实 `cpal` 回调的形态，长度由设备协商决定 [ARCH-TOP-002]。
    fn callback_frames(&mut self, frames: usize) {
        let samples = frames * 2;
        let Self {
            runtime, output, ..
        } = self;
        runtime.process_quantum(&mut output[..samples], 2);
    }

    /// 推一次**非帧对齐**的缓冲：`samples` 个样本里最后 1 个不属于任何完整帧
    /// （`process_quantum` 的契约是"保持它的原值"，见该函数的文档）。
    fn callback_samples(&mut self, samples: usize) {
        let Self {
            runtime, output, ..
        } = self;
        runtime.process_quantum(&mut output[..samples], 2);
    }

    /// 推一个 128 帧的量子。
    fn step(&mut self) {
        self.output.fill(0.0);
        self.runtime.process_quantum(&mut self.output, 2);
    }

    /// 在观测窗口内推 `quanta` 个量子。
    fn pump(&mut self, quanta: u64) -> Reading {
        window(|| {
            for _ in 0..quanta {
                self.step();
            }
        })
    }

    /// 预热：一次性惰性路径（FPU 武装、首份快照武装、波表/包络模板）不计入判据。
    fn preheat(&mut self) {
        self.step();
    }

    /// 发一条走带命令（**窗口之外**：控制线程允许分配）。
    fn send_transport(&mut self, command: TransportCommand) {
        let batch = [EngineEvent::Transport { command }];
        let written = self.sender.publish(&batch);
        assert_eq!(written, 1, "走带命令必须被通道接受（{command:?}）");
    }

    fn stats(&self) -> EngineStats {
        self.runtime.stats()
    }
}

/// 256 个交叠音符（整个测量窗口里都有声部在跑）。
fn saturated_notes() -> Vec<NoteSpec> {
    (0..256u64)
        .map(|index| NoteSpec::at(index * 240, 480, 60 + (index % 12) as u8, 100))
        .collect()
}

// ---------------------------------------------------------------------------
// 场景 ① 纯渲染
// ---------------------------------------------------------------------------

fn scenario_pure_render(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();

    let mut scenario = Scenario::new("①纯渲染");
    scenario.absorb(PURE_QUANTA, &rig.pump(PURE_QUANTA));
    let stats = rig.stats();
    scenario.note(format!(
        "quanta={} meter_frames={} event_bulk_pops={} limiter_reductions={}",
        stats.quanta, stats.meter_frames, stats.event_bulk_pops, stats.limiter_gain_reductions
    ));
    report.scenario(
        "①",
        "[MUST-GATE-001] 纯渲染 10 000 量子：四元组全 0",
        &scenario,
    );

    // 覆盖度自检：窗口里必须真的跑了那么多量子（防"窗口里什么都没跑"的假绿）。
    report.assert(
        "①c",
        "覆盖度：10 000 量子确实被处理",
        stats.quanta >= PURE_QUANTA && stats.event_bulk_pops >= PURE_QUANTA,
        format!(
            "quanta={}（要求 ≥ {PURE_QUANTA}）event_bulk_pops={}（每量子一次批量出队）",
            stats.quanta, stats.event_bulk_pops
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ② 快照交换（逐步 + 高频）
// ---------------------------------------------------------------------------

fn scenario_snapshot_swap(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();

    let mut scenario = Scenario::new("②快照交换");
    let mut released_on_main = 0u64;

    // 逐步：每次发布在窗口**之外**（控制线程允许分配），窗口里只处理 1 个量子。
    for revision in 2..=(SWAP_STEPWISE + 1) {
        let next = EngineSnapshot::from_project(&project, revision).expect("快照必须能编译");
        rig.slot.publish(next);
        scenario.absorb(1, &rig.pump(1));
        released_on_main += rig.queue.drain(64) as u64;
    }

    // 高频：每量子一发一换（1 000 次），主线程按 8 次交换的节拍排空退役队列。
    for offset in 0..SWAP_BURST {
        let revision = SWAP_STEPWISE + 2 + offset;
        let next = EngineSnapshot::from_project(&project, revision).expect("快照必须能编译");
        rig.slot.publish(next);
        scenario.absorb(1, &rig.pump(1));
        if offset % 8 == 0 {
            released_on_main += rig.queue.drain(64) as u64;
        }
    }
    released_on_main += rig.queue.drain(64) as u64;

    let stats = rig.stats();
    scenario.note(format!(
        "发布={} 实际切换={} 主线程回收={released_on_main} 退役队列满寄存={}",
        SWAP_STEPWISE + SWAP_BURST,
        stats.snapshot_switches,
        rig.runtime.snapshot_stash_events()
    ));
    report.scenario(
        "②",
        "[MUST-GATE-001] 快照交换（63 次逐步 + 1 000 次高频）：四元组全 0",
        &scenario,
    );

    report.assert(
        "②c",
        "覆盖度：高频交换真的发生且旧快照在主线程回收",
        stats.snapshot_switches >= SWAP_BURST && released_on_main > 0,
        format!(
            "实际切换={}（要求 ≥ {SWAP_BURST}）主线程回收={released_on_main}（> 0）",
            stats.snapshot_switches
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ③ 走带（Play / Stop / Seek）
// ---------------------------------------------------------------------------

fn scenario_transport(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();

    let mut scenario = Scenario::new("③走带");

    // 命令在窗口**之外**发布，窗口里只包含"出队应用 + 渲染量子"。
    for round in 0..TRANSPORT_ROUNDS {
        let command = match round % 4 {
            0 => TransportCommand::Stop,
            1 => TransportCommand::Play,
            2 => TransportCommand::SeekTicks(round * 37),
            _ => TransportCommand::Play,
        };
        rig.send_transport(command);
        scenario.absorb(1, &rig.pump(1));
    }

    // 稳定播放 2 000 量子（走带推进 + 声部合成 + 读数镜面）。
    rig.send_transport(TransportCommand::Play);
    scenario.absorb(TRANSPORT_PLAY_QUANTA, &rig.pump(TRANSPORT_PLAY_QUANTA));

    // 停住：应用 `Stop` 的那个量子也纳入窗口（它走的是同一条实时路径）。
    rig.send_transport(TransportCommand::Stop);
    scenario.absorb(1, &rig.pump(1));
    let frozen_before = rig.runtime.position_ticks();
    scenario.absorb(TRANSPORT_STOP_QUANTA, &rig.pump(TRANSPORT_STOP_QUANTA));
    let frozen_after = rig.runtime.position_ticks();

    let stats = rig.stats();
    scenario.note(format!(
        "走带命令={} 推进量子={} 位置 tick={} 冻结前后={frozen_before}->{frozen_after}",
        stats.transport_commands, stats.transport_quanta, stats.position_ticks
    ));
    report.scenario(
        "③",
        "[MUST-GATE-001] 走带 Play/Stop/Seek：四元组全 0",
        &scenario,
    );

    report.assert(
        "③c",
        "覆盖度：命令被应用、推进发生过、停住窗口时钟冻结",
        stats.transport_commands >= TRANSPORT_ROUNDS
            && stats.transport_quanta > 0
            && stats.transport_state == TransportState::Stopped
            && frozen_before == frozen_after,
        format!(
            "命令={}（要求 ≥ {TRANSPORT_ROUNDS}）推进量子={} 状态={:?} 停住窗口位置 {} -> {}",
            stats.transport_commands,
            stats.transport_quanta,
            stats.transport_state,
            frozen_before,
            frozen_after
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ④ 电平计量（每 5 个量子一次 UI 抽干）
// ---------------------------------------------------------------------------

fn scenario_metering(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();

    let mut scenario = Scenario::new("④电平计量");
    let mut scratch = [MeterFrame::default(); 512];
    let mut ui_ticks = 0u64;
    let mut drained = 0u64;
    let mut insane = 0u64;

    for _ in 0..(METER_QUANTA / METER_TICK_QUANTA) {
        scenario.absorb(METER_TICK_QUANTA, &rig.pump(METER_TICK_QUANTA));
        // UI 抽干在窗口**之外**（它不是实时线程的工作）。
        let popped = rig.collector.tick(&mut scratch);
        for frame in &scratch[..popped] {
            if !frame.is_sane() {
                insane += 1;
            }
        }
        drained += popped as u64;
        ui_ticks += 1;
    }

    // UI 抽干本身也要零分配（单独一个窗口；它不是实时路径，但同属"UI 线程不得拖累"）。
    let drain_reading = window(|| {
        for _ in 0..METER_DRAIN_ROUNDS {
            let popped = rig.collector.tick(&mut scratch);
            std::hint::black_box(popped);
        }
    });

    let stats = rig.stats();
    scenario.note(format!(
        "UI 抽干={ui_ticks} 次/共 {drained} 帧 非有限帧={insane} 每量子批量发布={} 计量帧={}",
        stats.meter_bulk_publishes, stats.meter_frames
    ));
    report.scenario(
        "④",
        "[MUST-GATE-001] 电平计量 10 000 量子：四元组全 0",
        &scenario,
    );

    report.assert(
        "④c",
        "覆盖度：每量子恰好一次批量发布、UI 真的抽到帧且全部有限",
        stats.meter_bulk_publishes >= METER_QUANTA && drained > 0 && insane == 0,
        format!(
            "批量发布={}（要求 ≥ {METER_QUANTA}）抽到帧={drained} 非有限帧={insane}",
            stats.meter_bulk_publishes
        ),
    );

    report.assert(
        "④d",
        "UI 侧抽干 1 000 次：零分配零释放（不把控制侧的内存压力带进实时线程）",
        drain_reading.quad.allocations == 0 && drain_reading.quad.deallocations == 0,
        format!(
            "抽干窗口四元组[{}]（探针经过={}）",
            drain_reading.quad.describe(),
            drain_reading.visits
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑤ 自动化求值（控制侧求值 → SPSC → 实时侧出队）
// ---------------------------------------------------------------------------

fn scenario_automation(report: &mut Report) {
    // 夹具：一条 MIDI 轨 + 一条**音量自动化泳道**（三点 ⇒ 线性插值真的在动）。
    let mut fixture = note_project(&[NoteSpec::at(0, 4_800, 60, 100)]);
    let track = fixture.track;
    let target = AutomationTarget::TrackVolume { track_id: track };
    let mut lane = AutomationLane {
        target,
        points: BTreeMap::new(),
        read_enabled: true,
        write_mode: AutomationWriteMode::Off,
        domain: None,
    };
    for (tick, value) in [(0u64, -6.0f32), (2_400, 3.0), (4_800, -12.0)] {
        let id = EntityId::new();
        lane.points.insert(
            id,
            AutomationPoint {
                id,
                tick,
                value,
                curve: CurveType::Linear,
            },
        );
    }
    fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨")
        .automation_lanes
        .insert(target, lane);

    let mut rig = Rig::new(&fixture.project, 1, 4096);
    rig.preheat();

    let mut scenario = Scenario::new("⑤自动化求值");
    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    let mut published = 0u64;

    for quantum in 0..AUTOMATION_QUANTA {
        // 控制侧：**模型层的唯一求值入口**，一量子一次（tick 由样本位置换算）。
        let tick = (quantum * DEFAULT_BLOCK_FRAMES as u64) / SAMPLES_PER_TICK;
        let value = fixture
            .project
            .automation_value_at(&target, tick)
            .expect("目标必须存在于夹具工程里")
            .unwrap_or(0.0);
        lowest = lowest.min(value);
        highest = highest.max(value);

        // 窗口**之外**发布（控制线程允许分配）。
        let batch = [EngineEvent::SetParam {
            target: ParamAddress::new(track, 0),
            value,
        }];
        published += rig.sender.publish(&batch) as u64;

        scenario.absorb(1, &rig.pump(1));
    }

    let stats = rig.stats();
    scenario.note(format!(
        "求值 {AUTOMATION_QUANTA} 次（值域 {lowest:.3} .. {highest:.3}）发布 {published} 条 实时侧应用 {} 条",
        stats.events_applied
    ));
    report.scenario(
        "⑤",
        "[MUST-GATE-001] 自动化求值 2 000 量子：四元组全 0",
        &scenario,
    );

    report.assert(
        "⑤c",
        "覆盖度：自动化曲线真的被求值（值在动）且事件真的被实时侧出队",
        highest > lowest && stats.events_applied >= AUTOMATION_QUANTA,
        format!(
            "值域 {lowest:.3} .. {highest:.3}（必须不同）实时侧应用={}（要求 ≥ {AUTOMATION_QUANTA}）",
            stats.events_applied
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑥ 限制器 / 混音链
// ---------------------------------------------------------------------------

fn scenario_mix_chain(report: &mut Report) {
    // 夹具逐项对应一个器件：+6 dB ⇒ 真的驱动限制器；pan = -1 ⇒ 右声道逐位静音；
    // cutoff 2 kHz ⇒ 声部低通真的被调用；40 个同时起音的长音符 ⇒ 逼出声部窃取。
    let mut notes = saturated_notes();
    notes.extend((0..40u64).map(|index| NoteSpec::at(0, 96_000, 48 + (index % 12) as u8, 100)));
    let mut fixture = tuned_project(
        &notes,
        MixSpec {
            volume_db: 6.0,
            pan: -1.0,
            cutoff_hz: Some(2_000.0),
            resonance: 0.4,
        },
    );
    // **主总线推子**也要在窗口内被走到：母线 +6 dB ⇒ `rt::scale_bus` 的逐样本乘
    // 真的执行（否则这条 `[MUST-GATE-001]` 判据对新增的那一次乘是**零覆盖**）。
    // 取 +6 dB（而不是 −6 dB）是因为推子在限制器**之前**：抬高只会让限制器压得更多，
    // 因此 ⑥c 的 `limiter_gain_reductions > 0` 这条覆盖度断言仍然确定成立。
    fixture
        .project
        .tracks
        .get_mut(&fixture.master)
        .expect("`tuned_project` 的母线在 tracks 里")
        .volume_db = 6.0;

    let mut rig = Rig::new(&fixture.project, 1, 8192);
    rig.preheat();

    let mut scenario = Scenario::new("⑥限制器/混音链");
    let mut nan = 0u64;
    let reading = window(|| {
        for _ in 0..MIX_QUANTA {
            rig.step();
            nan += rig.output.iter().filter(|sample| sample.is_nan()).count() as u64;
        }
    });
    scenario.absorb(MIX_QUANTA, &reading);

    let stats = rig.stats();
    let armed_master_gain = rig.runtime.armed_master_gain();
    let right_nonzero = rig
        .output
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|sample| **sample != 0.0)
        .count();
    scenario.note(format!(
        "限制器压过={} 最大压限={:.4} 声部窃取={} 触发音符={} NaN={nan} 右声道非零={right_nonzero} \
         主总线武装增益={armed_master_gain:.4}",
        stats.limiter_gain_reductions,
        stats.limiter_max_reduction,
        stats.voice_steals,
        stats.notes_triggered
    ));
    report.scenario(
        "⑥",
        "[MUST-GATE-001] 限制器/混音链 2 000 量子：四元组全 0",
        &scenario,
    );

    report.assert(
        "⑥c",
        "覆盖度：限制器真的压过、声部窃取真的发生、声相真的在线、\
         **主总线推子**真的被武装成非单位增益、输出无 NaN",
        stats.limiter_gain_reductions > 0
            && stats.voice_steals > 0
            && right_nonzero == 0
            && nan == 0
            && armed_master_gain.to_bits() != 1.0f32.to_bits(),
        format!(
            "压过样本={}（要求 > 0）窃取={}（要求 > 0）右声道非零={right_nonzero}（要求 0）\
             NaN={nan}（要求 0）主总线武装增益={armed_master_gain}（要求 ≠ 1.0，否则新增的乘没被走到）",
            stats.limiter_gain_reductions, stats.voice_steals
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑬ 回调缓冲长度边界（cpal 协商出来的长度不保证是 128 的倍数）
// ---------------------------------------------------------------------------

/// 非帧对齐缓冲的哨兵值。
///
/// 用 `NaN` 而不是一个普通数值：本文件的判据 ⑥ 已实测**输出无 NaN**，
/// 因此"没有被写过的残余样本"与"引擎写出的样本"不可能混淆
/// （任取一个普通数都有与真实样本碰撞的可能）。
const TAIL_SENTINEL: f32 = f32::NAN;

/// ⑬：把**回调缓冲长度**当成被测变量。
///
/// 为什么这条路径可能分配：`EngineRuntime::process_quantum` 是唯一把"设备给的任意
/// 长度交错缓冲"切成整量子的地方（`while offset < total_frames` + 逐帧交错拷贝）。
/// 一个自然的实现会在那里 `to_vec()`/`resize()`，或为尾部残块新建缓冲。
/// 既有六个场景**全部**用 `DEFAULT_BLOCK_FRAMES * 2` 个样本（恰好 1 个量子）
/// ⇒ 这条切块路径从未被覆盖过。
fn scenario_callback_buffer_edges(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();
    rig.send_transport(TransportCommand::Play);
    // 最大缓冲与非对齐缓冲先备好 ⇒ 本场景里的分配全在窗口之外。
    let largest = *CALLBACK_FRAME_EDGES.last().unwrap_or(&1);
    rig.reserve_output(largest * 2);
    rig.reserve_output(CALLBACK_ODD_TAIL_SAMPLES);

    let mut scenario = Scenario::new("⑬回调缓冲长度边界");
    let mut expected_blocks = 0u64;
    let mut aligned_sentinel_leaks = 0u64;
    let mut odd_tail_sentinels = 0u64;
    let mut odd_tail_expected = 0u64;
    let mut frames_served = 0u64;

    let reading = window(|| {
        for _ in 0..CALLBACK_ROUNDS {
            for &frames in &CALLBACK_FRAME_EDGES {
                expected_blocks += frames.div_ceil(DEFAULT_BLOCK_FRAMES) as u64;
                frames_served += frames as u64;
                rig.output.fill(TAIL_SENTINEL);
                rig.callback_frames(frames);
                // 帧对齐的缓冲里**每一个**样本都必须被写过 ⇒ 不该残留哨兵。
                aligned_sentinel_leaks += rig.output[..frames * 2]
                    .iter()
                    .filter(|sample| sample.is_nan())
                    .count() as u64;
            }
            // 非帧对齐：129 个样本 = 64 帧 + 1 个残余样本。
            let frames = CALLBACK_ODD_TAIL_SAMPLES / 2;
            expected_blocks += frames.div_ceil(DEFAULT_BLOCK_FRAMES) as u64;
            odd_tail_expected += 1;
            rig.output.fill(TAIL_SENTINEL);
            rig.callback_samples(CALLBACK_ODD_TAIL_SAMPLES);
            odd_tail_sentinels += rig.output[..CALLBACK_ODD_TAIL_SAMPLES]
                .iter()
                .filter(|sample| sample.is_nan())
                .count() as u64;
        }
    });
    scenario.absorb(expected_blocks, &reading);
    scenario.note(format!(
        "长度集合={CALLBACK_FRAME_EDGES:?} × {CALLBACK_ROUNDS} 轮 + 每轮一次 {CALLBACK_ODD_TAIL_SAMPLES} 样本；\
         共喂入 {frames_served} 帧 / {expected_blocks} 个量子；帧对齐缓冲残留哨兵={aligned_sentinel_leaks}（要求 0）"
    ));
    report.scenario(
        "⑬",
        "[MUST-GATE-001] 回调缓冲长度边界（1..1025 帧 + 非帧对齐）：四元组全 0",
        &scenario,
    );

    // 单次 1 025 帧的调用必须真的被切成 9 个量子（"多量子路径被动到"的定点证据，
    // 而不是靠总量推算）。
    rig.output.fill(TAIL_SENTINEL);
    let split = window(|| rig.callback_frames(largest));
    report.assert(
        "⑬c",
        "覆盖度：非帧对齐缓冲只留下约定的残余样本；单次 1 025 帧真的被切成 9 个量子",
        aligned_sentinel_leaks == 0
            && odd_tail_sentinels == odd_tail_expected
            && split.visits == largest.div_ceil(DEFAULT_BLOCK_FRAMES) as u64
            && split.visits > 1,
        format!(
            "帧对齐残留哨兵={aligned_sentinel_leaks}（要求 0）；非对齐 129 样本的残余哨兵={odd_tail_sentinels}\
             （要求 {odd_tail_expected}，即每轮恰好 1 个）；单次 {largest} 帧 ⇒ 量子数={}（要求 9）",
            split.visits
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑭ 采样率 / 项目声明缓冲长度切换（离线工程 → 换声卡）
// ---------------------------------------------------------------------------

/// ⑭：渲染途中切换**采样率**与项目声明的 `block_size`（每条都是一次快照修订）。
///
/// 为什么这条路径可能分配：新修订会在 `render_block` 内部走**重新武装**分支 ——
/// `MeterBank::set_quanta_per_second`、`SynthEngine::begin_snapshot`（按新采样率
/// 重算每条轨的相位/频率）、`Transport::arm`（新的 samples-per-tick）。
/// 一个自然的实现会在那里重建 `Vec`/`BTreeMap`。既有场景②只改 `revision`
/// （内容完全相同）⇒ **重新武装分支的"内容变了"那一半从未被覆盖**。
///
/// 顺带钉住一条**历史 bug**：弹道系数必须按**处理量子**（`sample_rate / 128`）折算，
/// 而不是按项目声明的 `block_size`（例如 256）—— 后者会让峰值保持按 10 dB/s 衰减。
/// 因此这里让 `block_size` 遍历全部合法取值，并断言武装值**只随采样率变**。
fn scenario_declared_audio_config_switch(report: &mut Report) {
    let fixture = note_project(&saturated_notes());
    let mut project = fixture.project;
    let mut rig = Rig::new(&project, 1, 4096);
    rig.send_transport(TransportCommand::Play);
    rig.preheat();

    let mut scenario = Scenario::new("⑭采样率/声明缓冲切换");
    let mut observed: Vec<f32> = Vec::new();
    let mut expected: Vec<f32> = Vec::new();
    let mut revision = 2u64;
    let ticks_before = rig.stats().position_ticks;

    for _ in 0..2 {
        for rate in SampleRate::ALL {
            for block in BlockSize::ALL {
                project.audio_config.sample_rate = rate;
                project.audio_config.block_size = block;
                let next =
                    EngineSnapshot::from_project(&project, revision).expect("快照必须能编译");
                rig.slot.publish(next);
                revision += 1;
                scenario.absorb(SAMPLE_RATE_QUANTA, &rig.pump(SAMPLE_RATE_QUANTA));
                // 读数在**窗口之外**（`observed`/`expected` 的 push 允许分配）。
                observed.push(rig.stats().quanta_per_second.unwrap_or(f32::NAN));
                expected.push(rate.hz() as f32 / DEFAULT_BLOCK_FRAMES as f32);
            }
        }
    }

    let stats = rig.stats();
    let distinct = observed
        .iter()
        .fold(Vec::<f32>::new(), |mut seen, value| {
            if !seen.contains(value) {
                seen.push(*value);
            }
            seen
        })
        .len();
    scenario.note(format!(
        "切换 {} 次（{} 采样率 × {} 声明缓冲 × 2 轮）；武装的量子/秒={distinct} 个不同值；\
         走带位置 {ticks_before} -> {} tick；合成样本={}",
        observed.len(),
        SampleRate::ALL.len(),
        BlockSize::ALL.len(),
        stats.position_ticks,
        stats.rendered_samples
    ));
    report.scenario(
        "⑭",
        "[MUST-GATE-001] 采样率/声明缓冲切换 2 000 量子：四元组全 0",
        &scenario,
    );

    report.assert(
        "⑭c",
        "覆盖度：每次切换都真的重新武装（量子/秒逐项等于 采样率/128，且与声明缓冲无关）",
        observed == expected
            && distinct == SampleRate::ALL.len()
            && stats.position_ticks > ticks_before,
        format!(
            "武装值逐项比对={}（要求 true；{} 项）；不同值={distinct}（要求 {}）；\
             走带 {ticks_before} -> {}（要求前进）",
            observed == expected,
            observed.len(),
            SampleRate::ALL.len(),
            stats.position_ticks
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑮ 播放中"编辑 / 撤销" + 快照高频交换（撤销交互实际碰到的路径）
// ---------------------------------------------------------------------------

/// ⑮：**播放中**每量子做一次「走 `Op` 层的编辑或它的逆操作（撤销）→ 重新发布快照」，
/// 并按 60 Hz 量级在主线程排空退役队列。
///
/// 为什么这条路径可能分配：一次撤销 = 控制线程上的模型改写 + 一次完整的
/// `EngineSnapshot::from_project`（新调度表 `BTreeMap`、新声相增益表、新 PDC 计划）+
/// 一次原子发布；实时侧则走"检测到新修订 ⇒ 重新武装 + 旧快照入退役队列"。
/// 既有场景②只发布**等价**快照（内容不变），场景③不做发布 ⇒
/// "一边出声一边被编辑/撤销"这一组合从未被覆盖。
///
/// **诚实边界**：这里的"播放中"是主线程按量子节拍驱动
/// `process_quantum`（判据声明的实时线程），**不是** cpal 的真回调线程；
/// 模型改写与快照构造本身在**窗口之外**（控制线程允许分配）。
fn scenario_undo_churn_while_playing(report: &mut Report) {
    let fixture = note_project(&saturated_notes());
    let track = fixture.track;
    let mut project = fixture.project;
    let mut rig = Rig::new(&project, 1, 4096);
    rig.send_transport(TransportCommand::Play);
    rig.preheat();

    // 真正的 `Op` 层（不是直接改字段）：一次编辑 + 它的逆操作 = 撤销交互的那一对。
    let base = project
        .tracks
        .get(&track)
        .expect("夹具必须有那条 MIDI 轨")
        .volume_db;
    let edit = StampedOp::user_ui(
        1,
        Op::SetParam {
            target: AutomationTarget::TrackVolume { track_id: track },
            old_val: base,
            new_val: base + 6.0,
        },
    );

    let mut scenario = Scenario::new("⑮播放中编辑/撤销");
    let mut released_on_main = 0u64;
    let mut edits = 0u64;
    let mut undos = 0u64;

    for index in 0..UNDO_CHURN_QUANTA {
        // 控制线程：编辑 / 撤销（窗口之外，允许分配）。
        if index % 2 == 0 {
            edit.apply(&mut project)
                .expect("SetParam 必须能应用到夹具工程");
            edits += 1;
        } else {
            edit.apply_inverse(&mut project)
                .expect("SetParam 的逆操作必须能应用");
            undos += 1;
        }
        let next = EngineSnapshot::from_project(&project, index + 2).expect("快照必须能编译");
        rig.slot.publish(next);
        // 窗口里恰好一个量子：快照切换 + 重新武装 + 合成 + 电平 + 走带。
        scenario.absorb(1, &rig.pump(1));
        // 主线程排空退役队列（窗口之外；这是规范指定的释放位置 [ARCH-RT-002]）。
        if index % UNDO_CHURN_DRAIN_EVERY == 0 {
            released_on_main += rig.queue.drain(64) as u64;
        }
    }
    released_on_main += rig.queue.drain(64) as u64;

    let stats = rig.stats();
    scenario.note(format!(
        "编辑={edits} 撤销={undos} 发布={UNDO_CHURN_QUANTA}；实际切换={} 主线程回收={released_on_main}；\
         走带位置={} tick 状态={:?}；释放线程=主线程?{} 外线程排空={} 退役满寄存={}",
        stats.snapshot_switches,
        stats.position_ticks,
        stats.transport_state,
        stats.release_thread_is_main,
        stats.foreign_drains,
        stats.snapshot_stash_events
    ));
    report.scenario(
        "⑮",
        "[MUST-GATE-001] 播放中编辑/撤销 2 000 轮 + 2 000 次快照交换：四元组全 0",
        &scenario,
    );

    report.assert(
        "⑮c",
        "覆盖度：编辑与撤销都真的应用过；快照真的切过；走带在走；旧快照只在主线程释放且无积压",
        edits > 0
            && undos > 0
            && stats.snapshot_switches >= UNDO_CHURN_QUANTA
            && stats.position_ticks > 0
            && released_on_main > 0
            && stats.release_thread_is_main
            && stats.foreign_drains == 0
            && stats.snapshot_stash_events == 0,
        format!(
            "编辑={edits}（要求 >0）撤销={undos}（要求 >0）切换={}（要求 ≥{UNDO_CHURN_QUANTA}）\
             位置={}（要求 >0）主线程回收={released_on_main}（要求 >0）释放归属主线程={} \
             外线程排空={}（要求 0）退役满寄存={}（要求 0）",
            stats.snapshot_switches,
            stats.position_ticks,
            stats.release_thread_is_main,
            stats.foreign_drains,
            stats.snapshot_stash_events
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑯ 满批事件洪峰（`SCRATCH_EVENTS` 条/量子的出队块边界）
// ---------------------------------------------------------------------------

/// ⑯：每量子发布**一整批** `SCRATCH_EVENTS` = 128 条事件（参数 + 音符 + 走带混排），
/// 把实时侧的批量出队压到栈上临时缓冲的**满块边界**。
///
/// 为什么这条路径可能分配：`EventReceiver::drain_with` 要把整批 128 条搬进
/// `[EngineEvent; SCRATCH_EVENTS]` 栈数组并逐条回调。一个自然的实现会
/// `Vec::from_iter`/`extend`，或在"批比缓冲大"时分配溢出缓冲。
/// 既有场景⑤每量子只发 **1** 条 ⇒ 满块边界从未被覆盖。
///
/// **诚实边界（与场景⑤同一条，必须一起读）**：实时侧目前**只把
/// `EngineEvent::Transport` 应用到状态机**；`SetParam` / `NoteOn` / `NoteOff`
/// 走到"出队 + 计数"为止，**没有**作用于 DSP。因此本场景证明的是
/// "满批出队与计数这条路径零分配"，**不是**"128 个参数变化都影响了声音"。
fn scenario_event_flood(report: &mut Report) {
    let fixture = note_project(&saturated_notes());
    let track = fixture.track;
    let project = fixture.project;
    // 事件通道必须装得下一整批，否则"满批"会悄悄退化成"半批"。
    let mut rig = Rig::with_capacities(&project, 1, 4096, 64, DEFAULT_EVENT_CAPACITY);
    rig.send_transport(TransportCommand::Play);
    rig.preheat();

    let mut scenario = Scenario::new("⑯满批事件洪峰");
    let mut batch = [EngineEvent::IDLE; SCRATCH_EVENTS];
    let mut published = 0u64;

    for _ in 0..FLOOD_QUANTA {
        for (slot, event) in batch.iter_mut().enumerate() {
            *event = match slot % 4 {
                0 => EngineEvent::SetParam {
                    target: ParamAddress::new(track, (slot % 8) as u16),
                    value: (slot as f32) * 0.5 - 8.0,
                },
                1 => EngineEvent::NoteOn {
                    track,
                    pitch: 48 + (slot % 12) as u8,
                    velocity: 100,
                },
                2 => EngineEvent::NoteOff {
                    track,
                    pitch: 48 + (slot % 12) as u8,
                },
                _ => EngineEvent::Transport {
                    command: TransportCommand::Play,
                },
            };
        }
        // 发布在窗口之外；窗口里只有出队 + 渲染。
        published += rig.sender.publish(&batch) as u64;
        scenario.absorb(1, &rig.pump(1));
    }

    let stats = rig.stats();
    scenario.note(format!(
        "每量子 {SCRATCH_EVENTS} 条 × {FLOOD_QUANTA} 量子 = 尝试 {} 条，通道接受 {published} 条；\
         实时侧应用={} 批量出队={}（要求 = {FLOOD_QUANTA}）；通道容量={}",
        FLOOD_QUANTA * SCRATCH_EVENTS as u64,
        stats.events_applied,
        stats.event_bulk_pops,
        rig.sender.capacity()
    ));
    report.scenario(
        "⑯",
        "[MUST-GATE-001] 满批事件洪峰（128 条/量子）500 量子：四元组全 0",
        &scenario,
    );

    report.assert(
        "⑯c",
        "覆盖度：每一条都被通道接受并被实时侧出队计数（满块边界真的走到）",
        published == FLOOD_QUANTA * SCRATCH_EVENTS as u64
            && stats.events_applied >= FLOOD_QUANTA * SCRATCH_EVENTS as u64
            && stats.event_bulk_pops >= FLOOD_QUANTA,
        format!(
            "接受={published}（要求 {}）实时侧应用={}（要求 ≥{}）批量出队={}（要求 ≥{FLOOD_QUANTA}）",
            FLOOD_QUANTA * SCRATCH_EVENTS as u64,
            stats.events_applied,
            FLOOD_QUANTA * SCRATCH_EVENTS as u64,
            stats.event_bulk_pops
        ),
    );
}

// ---------------------------------------------------------------------------
// 场景 ⑰ PDC 补偿延迟线（`ROAD-M2-004` 接线之后新增）
// ---------------------------------------------------------------------------

/// `filled_project()` 里各节点自身的延迟在**跨快照重新武装**时乘上的倍数。
///
/// 用一个既非 1、也非 0 的倍数，是为了让"重新武装真的改了延迟"可观测：
/// 32 → 96 帧，两支路的 `D` 都换值，`set_delay` 分支必须真的走到。
const PDC_REARM_FACTOR: u32 = 3;

/// ⑰：**PDC 延迟线的逐样本环形读写真的在实时窗口里跑**，且四元组仍全 0。
///
/// 为什么必须有这一条（而不是"① 已经跑过 filled_project 了"就算覆盖）：
/// ① 用的 `filled_project()` 确实带一个上报 32 帧延迟的设备，但**如果**装配里
/// `PdcPlan::compensation` 全为 0、或 `apply` 没被调用，① 的四元组读数**一模一样**——
/// "零分配"这条读数对"延迟线有没有真的处理样本"是完全盲的。因此本场景额外钉住：
///
/// 1. `armed_pdc_delay(v) == plan.compensation(v)`（逐节点，装配读数 vs 计划）；
/// 2. 至少一条轨的武装延迟 **> 0**，且窗口里**真的有非零样本**流过；
/// 3. 跨快照重新武装到另一组延迟之后，读数等于**新**计划，且 `pdc_clamped_frames` /
///    `pdc_unarmed_nodes` 仍为 0（池装得下）。
///
/// 变红的注入：删掉 `render_block` 里的 `self.pdc.apply(...)` ⇒ 断言 2 的
/// `pdc_processed_blocks` 停在预热那一次数值上（**实测**：注入后 ⑰c 红，
/// 见 `docs/ledger/engine-rt-notes.md` 的记录口径）；把 `rearm` 改成每次
/// `from_plan`（重新分配）⇒ 本场景的四元组分配分量红。
///
/// ⚠ 本场景**不能**替代行为判据：它证明的是"非零延迟的环形读写真的在零分配窗口里
/// 跑过"，而"延迟量正确、并联支路采样级同相"由 `tests/pdc_mix_path.rs` 的 P1/P2
/// 逐位判据负责（那里逐帧比对 `shifted[t] == reference[t - D]`）。
fn scenario_pdc_delay_lines(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();

    // ---- 装配读数 vs 计划：逐节点相等（控制线程读数，窗口之外）----
    let plan = EngineSnapshot::from_project(&project, 1).expect("快照必须能编译");
    let armed = armed_pdc(&rig, &project, plan.pdc());
    let (positive, total) = (armed.positive, armed.total);

    let mut scenario = Scenario::new("⑰PDC 延迟线");

    // 窗口 1：稳态逐样本环形读写（2 000 量子）。
    let mut nonzero = 0u64;
    let reading = window(|| {
        for _ in 0..PDC_QUANTA {
            rig.step();
            nonzero += rig.output.iter().filter(|sample| **sample != 0.0).count() as u64;
        }
    });
    scenario.absorb(PDC_QUANTA, &reading);
    scenario.note(format!(
        "武装延迟>0 的节点={positive} 延迟和={total} 帧；窗口内非零样本={nonzero}"
    ));

    // 窗口 2：跨快照**重新武装**到另一组延迟（32 → 96 帧）。
    let mut latencies = LatencyTable::new();
    for (node, frames) in LatencyTable::from_project(&project).iter() {
        latencies.set(*node, frames.saturating_mul(PDC_REARM_FACTOR));
    }
    let rearmed = EngineSnapshot::from_project_with_latencies(&project, 2, &latencies)
        .expect("快照必须能编译");
    rig.slot.publish(rearmed);
    scenario.absorb(PDC_REARM_QUANTA, &rig.pump(PDC_REARM_QUANTA));

    let stats = rig.stats();
    let rearm_plan = EngineSnapshot::from_project_with_latencies(&project, 2, &latencies)
        .expect("快照必须能编译");
    let after = armed_pdc(&rig, &project, rearm_plan.pdc());

    report.scenario(
        "⑰",
        "[MUST-GATE-001] PDC 补偿延迟线（2 000 量子稳态 + 1 000 量子重新武装）：四元组全 0",
        &scenario,
    );

    report.assert(
        "⑰c",
        "覆盖度：武装延迟逐节点等于计划、至少一条 > 0、窗口里真有样本流过、重新武装后等于新计划",
        armed.mismatches.is_empty()
            && positive >= 1
            && total > 0
            && nonzero > 0
            && after.mismatches.is_empty()
            && after.positive >= 1
            && after.total == total * u64::from(PDC_REARM_FACTOR)
            && stats.pdc_processed_blocks >= PDC_QUANTA + PDC_REARM_QUANTA
            && stats.pdc_unarmed_nodes == 0
            && stats.pdc_clamped_frames == 0,
        format!(
            "装配差异={:?}（要求空）武装>0 节点={positive}（≥1）延迟和={total}（>0）窗口非零样本={nonzero}（>0）\
             重新武装后差异={:?}（要求空）延迟和={}（要求 {}）延迟线真的处理过={}（要求 ≥{}）             未武装节点={} 被钳帧数={}（均要求 0）",
            armed.mismatches,
            after.mismatches,
            after.total,
            total * u64::from(PDC_REARM_FACTOR),
            stats.pdc_processed_blocks,
            PDC_QUANTA + PDC_REARM_QUANTA,
            stats.pdc_unarmed_nodes,
            stats.pdc_clamped_frames
        ),
    );
}

/// 逐节点比对"装置里武装的延迟"与"计划里的 `D(v)`"。
struct ArmedPdc {
    mismatches: Vec<String>,
    positive: usize,
    total: u64,
}

fn armed_pdc(rig: &Rig, project: &YebanProjectV1, plan: &PdcPlan) -> ArmedPdc {
    let mut armed = ArmedPdc {
        mismatches: Vec::new(),
        positive: 0,
        total: 0,
    };
    for id in project.tracks.keys() {
        match (plan.compensation(id), rig.runtime.armed_pdc_delay(id)) {
            (Some(want), Some(got)) if want as usize == got => {
                if got > 0 {
                    armed.positive += 1;
                    armed.total += got as u64;
                }
            }
            (None, None) => {}
            (want, got) => armed
                .mismatches
                .push(format!("{id:?}: 计划 {want:?} vs 武装 {got:?}")),
        }
    }
    armed
}

// ---------------------------------------------------------------------------
// 场景 ⑱ / ⑲ 溢出路径（`N6` 裁决 = 选项 A：纯计数，不经 sink）
// ---------------------------------------------------------------------------
//
// 这两条场景补的是本判据**历史上明确登记为"不在覆盖集内"**的两条路径
// （见模块文档的边界表与 `docs/ledger/gate-rt-zero-alloc-notes.md` §13 的实测）：
// 它们曾经调用 `rt_probe::diag` ⇒ 在装了 witness sink 的这个二进制里实测
// `io_requests == io_ops == 400`（退役欠容）与 `10000`（电平容量），
// 因此当时把它们纳入就会把判据打红 —— 而"红"的正是它们**真的做了 I/O**。
//
// 选项 A 之后它们走 `rt_probe::note_suppressed`（不读 `SINK`），于是这两条场景
// 可以**同时**断言两件事：
//   1. 四元组全 0 —— 尤其 `io_requests == 0 && io_ops == 0`（sink 是真的装着的）；
//   2. `suppressed_diag_events > 0` —— 那条纯计数出口**真的被走到**。
// 只断言 (1) 会有一个致命的假绿：把溢出分支删掉/绕开，读数**一模一样**。
// 两条合起来才是"**跑到了溢出，也没有任何 I/O**"。

/// ⑱：退役队列**容量不足**（容量 1 + 控制面**故意**不排空）64 轮。
///
/// 覆盖的实时路径：`render_block` → `SnapshotReader::begin_block` → `retire_or_stash`
/// 的 `Err(rtrb::PushError::Full)` 分支。生产的正确处置是"控制面降速 + 排空"
/// （见 `EngineStats::is_snapshot_lagging`）；本场景测的是**降速之前**那一刻：
/// 溢出确实发生、诊断只记数、音频线程上一次 I/O 都没有。
fn scenario_retire_queue_overflow(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::with_capacities(&project, 1, 4096, STASH_RETIRE_CAPACITY, 64);
    rig.preheat();
    // 预热之后读一次基线（寄存是**粘滞**的，见 `begin_block` 的 `stash.is_none()` 前提）。
    let stash_before = rig.stats().snapshot_stash_events;

    let mut scenario = Scenario::new("⑱退役队列欠容");
    for revision in 2..=(STASH_ROUNDS + 1) {
        // 发布在窗口之外（控制线程允许分配），窗口里只处理 1 个量子。
        let next = EngineSnapshot::from_project(&project, revision).expect("快照必须能编译");
        rig.slot.publish(next);
        scenario.absorb(1, &rig.pump(1));
    }

    let stats = rig.stats();
    let stash_events = stats.snapshot_stash_events.saturating_sub(stash_before);
    scenario.note(format!(
        "退役队列容量={STASH_RETIRE_CAPACITY} 发布={STASH_ROUNDS} 实际切换={} 领域计数 stash_events={stash_events} \
         （读者停在 revision={:?}）；纯计数诊断={}（按种类累计到进程为止 stash={}）",
        stats.snapshot_switches,
        rig.runtime.revision(),
        scenario.suppressed,
        rt_probe::suppressed_by_kind(RtDiagEvent::SnapshotRetireStash)
    ));

    report.assert(
        "⑱",
        "[MUST-GATE-001] 退役队列欠容走纯计数：四元组全 0（含 io_requests/io_ops）",
        scenario.quad.is_zero() && scenario.witness_ok() && scenario.suppressed >= 1,
        scenario.detail(),
    );

    // 覆盖度 + **两个计数器的交叉核对**：纯计数出口的次数必须**恰好等于**那条路径
    // 自己的领域计数（`retire_or_stash` 里两者在同一分支上各加一次）。
    report.assert(
        "⑱c",
        "覆盖度：溢出真的发生（领域计数 > 0）且纯计数出口次数与它逐次相等",
        stash_events >= 1 && scenario.suppressed == stash_events,
        format!(
            "领域计数 stash_events={stash_events}（要求 ≥1）纯计数诊断={}（要求相等）；\
             退役队列里滞留={}（容量 {STASH_RETIRE_CAPACITY}）",
            scenario.suppressed, stats.retire_pending
        ),
    );
}

/// ⑲：电平计量**容量不足**（轨道数 > `SCRATCH_METERS - 1`）100 量子。
///
/// 快照用 [`EngineSnapshot::from_parts`] 直接造（真实的工程投影不会给出 300 条轨，
/// 而"轨道数超过暂存槽"正是要测的那条分支）。覆盖的实时路径：
/// `render_block` 的"轨道数 > 预算 ⇒ 记 `meter_capacity_drops` + 纯计数诊断"分支。
fn scenario_meter_capacity_overflow(report: &mut Report) {
    let mut rig = Rig::from_snapshot(oversized_meter_snapshot(1), 4096, 64, 64);
    rig.preheat();

    // 每量子被丢掉的节点数 = 轨道数 − (暂存槽 − 1 个母线槽)。
    let drops_per_quantum = (OVERSIZE_TRACKS - (SCRATCH_METERS - 1)) as u64;
    let before = rig.stats();

    let mut scenario = Scenario::new("⑲电平容量不足");
    scenario.absorb(METER_OVERFLOW_QUANTA, &rig.pump(METER_OVERFLOW_QUANTA));

    let stats = rig.stats();
    let drops = stats
        .meter_capacity_drops
        .saturating_sub(before.meter_capacity_drops);
    let publishes = stats
        .meter_bulk_publishes
        .saturating_sub(before.meter_bulk_publishes);
    let frames = stats.meter_frames.saturating_sub(before.meter_frames);
    scenario.note(format!(
        "轨道数={OVERSIZE_TRACKS}（母线 + {} 条普通轨；暂存槽={SCRATCH_METERS}，母线占 1）\
         量子={METER_OVERFLOW_QUANTA}；容量丢弃={drops}（要求 {drops_per_quantum}/量子）\
         批量发布={publishes}（要求 1/量子）；纯计数诊断={}；\
         ⚠ 电平队列写入帧数={frames} **不是**计量工作量：SPSC 环容量 4096、本场景不抽干 \
         ⇒ 15 个满批（{SCRATCH_METERS} 帧/量子）之后 `publish` 只能写 0 条",
        OVERSIZE_TRACKS - 1,
        scenario.suppressed
    ));

    report.assert(
        "⑲",
        "[MUST-GATE-001] 电平容量不足走纯计数：四元组全 0（含 io_requests/io_ops）",
        scenario.quad.is_zero() && scenario.witness_ok() && scenario.suppressed >= 1,
        scenario.detail(),
    );

    report.assert(
        "⑲c",
        "覆盖度：容量溢出真的发生（丢弃计数逐量子对得上、每量子一次批量发布）且纯计数出口每量子恰好一次",
        drops == drops_per_quantum * METER_OVERFLOW_QUANTA
            && publishes == METER_OVERFLOW_QUANTA
            && scenario.suppressed == METER_OVERFLOW_QUANTA,
        format!(
            "容量丢弃={drops}（要求 {} = {drops_per_quantum}×{METER_OVERFLOW_QUANTA}；含 MeterBank 淘汰数，\
             因此这条等式也证明电平槽从未溢出）批量发布={publishes}（要求 {METER_OVERFLOW_QUANTA}）\
             纯计数诊断={}（要求 {METER_OVERFLOW_QUANTA}）",
            drops_per_quantum * METER_OVERFLOW_QUANTA,
            scenario.suppressed
        ),
    );
}

/// 造一份**轨道数超过电平暂存容量**的快照：母线 + [`OVERSIZE_TRACKS`] 条普通轨。
///
/// 走的是与真实投影**同一套**下层构造（`TrackParams::from_track` + `from_parts`），
/// 只是绕开了"工程 → 快照"的投影（真实工程不会长成这样）。
fn oversized_meter_snapshot(revision: u64) -> EngineSnapshot {
    let master = EntityId::new();
    let mut routing = RoutingGraph {
        nodes: vec![master],
        ..RoutingGraph::default()
    };
    let mut tracks: BTreeMap<EntityId, TrackParams> = BTreeMap::new();
    tracks.insert(
        master,
        TrackParams::from_track(
            &TrackV3 {
                id: master,
                ..TrackV3::default()
            },
            0,
        ),
    );
    for _ in 0..OVERSIZE_TRACKS {
        let track = EntityId::new();
        routing.nodes.push(track);
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
    }
    EngineSnapshot::from_parts(
        revision,
        SampleRate::Hz48000.hz(),
        DEFAULT_BLOCK_FRAMES,
        2,
        master,
        tracks,
        &routing,
        &LatencyTable::new(),
    )
    .expect("超量轨道快照必须能编译")
}

// ---------------------------------------------------------------------------
// 判据 ⑦ 探针有牙（正对照：非 RT 路径上让读数非 0）
// ---------------------------------------------------------------------------

/// ⑦b 的争用对照锁（`static` ⇒ 两个线程共享同一把）。
static CONTENTION_LOCK: RtLockProbe = RtLockProbe::new();
/// ⑦b 的握手：持有方已取到锁。
static CONTENTION_HELD: AtomicBool = AtomicBool::new(false);
/// ⑦b 的握手：持有方已放手。
static CONTENTION_RELEASED: AtomicBool = AtomicBool::new(false);
/// ⑦b 持有方持锁的时长：主线程要在这段时间内进窗口并阻塞加锁。
///
/// 刻意**不**用"主线程发信号让持有方放手"的握手 —— 那会死锁
/// （主线程阻塞在 `lock()` 上等持有方，持有方等主线程的信号）。固定时长把等待变成
/// 确定的：主线程先等 `CONTENTION_HELD`，再进窗口，而窗口内的 `lock()` 必然撞上持有方。
const CONTENTION_HOLD: Duration = Duration::from_millis(250);
/// ⑪ 的握手：外线程可以开始活动了。
static FOREIGN_GO: AtomicBool = AtomicBool::new(false);
/// ⑪ 的握手：外线程的活动已完成。
static FOREIGN_DONE: AtomicBool = AtomicBool::new(false);
/// ⑪ 外线程用的锁（**不是**实时路径那把 —— 不许干扰"试探永远成功"这条见证）。
static ATTRIBUTION_LOCK: RtLockProbe = RtLockProbe::new();

/// 有界自旋等待（超时返回 `false`：判据记失败而不是挂死）。
fn spin_until(flag: &AtomicBool, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while !flag.load(Ordering::Acquire) {
        if Instant::now() > deadline {
            return false;
        }
        std::hint::spin_loop();
    }
    true
}

fn probe_teeth(report: &mut Report, witness: &Arc<WitnessSink>) {
    // ---- ⑦a：非 RT（控制侧）阻塞加锁必须被计数 ----
    // ⚠ 先"温暖"（窗口之外碰一次）：macOS 上每个新 `Mutex` 的首次加锁会分配 64 字节
    // （见 `rt_probe` 模块文档）—— 不温暖的话探针自己就会把分配分量打红。
    let probe = RtLockProbe::new();
    let _ = probe.try_lock_once();
    let locking = window(|| {
        for _ in 0..3 {
            drop(probe.lock());
        }
    });
    report.assert(
        "⑦a",
        "锁探针有牙：非 RT 路径上 3 次阻塞加锁被计数（0 次等待、四元组其余分量仍 0）",
        locking.quad.lock_blocking == 3
            && locking.quad.lock_waits == 0
            && locking.quad.allocations == 0
            && locking.quad.deallocations == 0
            && locking.quad.io_requests == 0
            && locking.quad.io_ops == 0,
        format!(
            "窗口四元组[{}]（锁分量必须非 0：证明这个计数器不是空转）",
            locking.quad.describe()
        ),
    );

    // ---- ⑦b：**已被别的线程持有** ⇒ 观察到"等待"（比"尝试非 0"更强）----
    //
    // 握手设计：持有方取锁 → 置 `HELD` → **睡固定时长**（不依赖主线程的信号，
    // 否则会死锁：主线程阻塞在 `lock()` 上等持有方，而持有方在等主线程的信号）。
    // 主线程在这段时长内进入窗口并阻塞加锁 ⇒ 稳定观察到 1 次等待。
    let _ = CONTENTION_LOCK.try_lock_once(); // 温暖（窗口之外）
    CONTENTION_HELD.store(false, Ordering::Release);
    CONTENTION_RELEASED.store(false, Ordering::Release);
    let holder = thread::spawn(|| {
        let _guard = CONTENTION_LOCK.lock();
        CONTENTION_HELD.store(true, Ordering::Release);
        thread::sleep(CONTENTION_HOLD);
        CONTENTION_RELEASED.store(true, Ordering::Release);
    });
    let held = spin_until(&CONTENTION_HELD, HANDSHAKE_LIMIT);
    let contended = window(|| {
        // 这一句会真的**阻塞**约 `CONTENTION_HOLD`：持有方还没放手。
        drop(CONTENTION_LOCK.lock());
    });
    let joined = holder.join().is_ok();
    let released = CONTENTION_RELEASED.load(Ordering::Acquire);
    report.assert(
        "⑦b",
        "等待有牙：锁已被别的线程持有时，阻塞加锁被记成 1 次等待",
        held
            && joined
            && released
            && contended.quad.lock_blocking == 1
            && contended.quad.lock_waits == 1,
        format!(
            "握手(held={held} joined={joined} released={released}) 争用窗口四元组[{}]（要求 lock_waits == 1）",
            contended.quad.describe()
        ),
    );

    // ---- ⑦c：I/O 边界必须真的写文件 + 打印（计数 + 字节 + 文件内容三重证据）----
    let path = witness.path.clone();
    let lines_before = count_lines(&path, RtDiagEvent::MeterCapacityDrop.as_str());
    let emits_before = witness.emits();
    let bytes_before = witness.bytes();

    let io_reading = window(|| {
        for _ in 0..3 {
            rt_probe::diag(RtDiagEvent::MeterCapacityDrop);
        }
    });

    let emits_after = witness.emits();
    let bytes_after = witness.bytes();
    let lines_after = count_lines(&path, RtDiagEvent::MeterCapacityDrop.as_str());
    report.assert(
        "⑦c",
        "I/O 探针有牙：非 RT 路径上 3 次诊断 = 3 次真实写（文件 + 打印）",
        io_reading.quad.io_requests == 3
            && io_reading.quad.io_ops == 3
            && emits_after.saturating_sub(emits_before) == 3
            && bytes_after > bytes_before
            && lines_after.saturating_sub(lines_before) == 3,
        format!(
            "窗口四元组[{}]；sink 计数 emits {} -> {} bytes {} -> {}；见证文件里该事件行数 {} -> {}",
            io_reading.quad.describe(),
            emits_before,
            emits_after,
            bytes_before,
            bytes_after,
            lines_before,
            lines_after
        ),
    );

    // ---- ⑦d：见证 sink 本身**零分配** ⇒ 注入 I2 变红只能归因于 I/O（不是分配）----
    report.assert(
        "⑦d",
        "见证 sink 零分配：同一窗口的分配分量仍为 0（注入变红的归因干净）",
        io_reading.quad.allocations == 0 && io_reading.quad.deallocations == 0,
        format!(
            "诊断窗口 alloc={} dealloc={}（真实文件/控制台 I/O 不计入堆分配）",
            io_reading.quad.allocations, io_reading.quad.deallocations
        ),
    );
}

/// 数一数见证文件里某个诊断事件出现了几行（**读文件在窗口之外**）。
fn count_lines(path: &std::path::Path, needle: &str) -> u64 {
    std::fs::read_to_string(path)
        .map(|content| content.lines().filter(|line| *line == needle).count() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 判据 ⑩ 窗口语义（窗口外不影响判据 / 窗口内有牙）
// ---------------------------------------------------------------------------

fn window_semantics(report: &mut Report) {
    // ---- ⑩a：窗口**之外**发生分配 1 000 次 ⇒ 随后的窗口读数不受影响 ----
    let mut noise: Vec<Box<[u8; 64]>> = Vec::new();
    for _ in 0..1_000 {
        noise.push(Box::new([0u8; 64]));
    }
    let empty = window(|| {
        std::hint::black_box(&noise);
    });
    let outside_ok = empty.quad.is_zero() && empty.visits == 0;

    // ---- ⑩b：窗口**之内**分配 1 次 ⇒ 必须被看见（分配探针有牙）----
    let inside = window(|| {
        std::hint::black_box(Vec::<u8>::with_capacity(64));
    });
    let inside_ok = inside.quad.allocations == 1 && inside.quad.deallocations == 1;

    // ---- ⑩c：未武装时的锁/I/O 操作**不进窗口**，但进程总量照样涨（对照量）----
    rt_probe::unwatch_current_thread();
    rt_probe::reset_current_thread();
    let totals_before = rt_probe::totals();
    let probe = RtLockProbe::new();
    drop(probe.lock());
    rt_probe::diag(RtDiagEvent::NoSnapshot);
    let totals_after = rt_probe::totals();
    let unarmed_window = rt_probe::window_current_thread();
    let unarmed_ok = unarmed_window.lock_blocking_attempts == 0
        && unarmed_window.io_requests == 0
        && totals_after.lock_blocking_attempts > totals_before.lock_blocking_attempts
        && totals_after.io_requests > totals_before.io_requests;

    report.assert(
        "⑩",
        "ARMED 窗口语义：窗口外不计数（无噪声）、窗口内有牙（1 次分配被看见）、未武装不进窗口",
        outside_ok && inside_ok && unarmed_ok,
        format!(
            "窗口外 1 000 次分配后空窗口[{}]；窗口内 1 次分配 {} alloc/{} dealloc；\
             未武装窗口 lock_blocking={} io_requests={}（总量已 +{} / +{}）",
            empty.quad.describe(),
            inside.quad.allocations,
            inside.quad.deallocations,
            unarmed_window.lock_blocking_attempts,
            unarmed_window.io_requests,
            totals_after.lock_blocking_attempts - totals_before.lock_blocking_attempts,
            totals_after.io_requests - totals_before.io_requests
        ),
    );
}

// ---------------------------------------------------------------------------
// 判据 ⑪ 线程归属（外线程的活动不许算进实时窗口）
// ---------------------------------------------------------------------------

fn thread_attribution(report: &mut Report, witness: &Arc<WitnessSink>) {
    let totals_before = rt_probe::totals();
    let emits_before = witness.emits();

    // 温暖外线程用的那把锁（窗口之外，主线程）：macOS 上首次加锁会分配 64 字节；
    // 否则那笔分配会落在**外线程**上（它的窗口没开 ⇒ 不会污染实时窗口，但会污染
    // "外线程四元组"的干净读数）。见 `rt_probe` 模块文档。
    let _ = ATTRIBUTION_LOCK.try_lock_once();

    // 外线程在**主线程窗口开着的时候**做 3 次阻塞加锁 + 2 次真实 I/O（写文件 + 打印）。
    let foreign = thread::spawn(|| {
        if !spin_until(&FOREIGN_GO, HANDSHAKE_LIMIT) {
            return rt_probe::window_current_thread();
        }
        for _ in 0..FOREIGN_LOCKS {
            drop(ATTRIBUTION_LOCK.lock());
        }
        for _ in 0..FOREIGN_DIAGS {
            rt_probe::diag(RtDiagEvent::SnapshotRetireStash);
        }
        let window = rt_probe::window_current_thread();
        FOREIGN_DONE.store(true, Ordering::Release);
        window
    });

    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();

    let mut scenario = Scenario::new("⑪线程归属");
    let mut foreign_finished = false;
    let reading = window(|| {
        for _ in 0..(ATTRIBUTION_QUANTA / 2) {
            rig.step();
        }
        FOREIGN_GO.store(true, Ordering::Release);
        foreign_finished = spin_until(&FOREIGN_DONE, HANDSHAKE_LIMIT);
        for _ in 0..(ATTRIBUTION_QUANTA / 2) {
            rig.step();
        }
    });
    scenario.absorb(ATTRIBUTION_QUANTA, &reading);

    let foreign_window = foreign.join().expect("外线程必须正常结束");
    let totals_after = rt_probe::totals();
    let emits_after = witness.emits();

    let lock_delta = totals_after
        .lock_blocking_attempts
        .saturating_sub(totals_before.lock_blocking_attempts);
    let foreign_lock_delta = totals_after
        .foreign_lock_blocking_attempts
        .saturating_sub(totals_before.foreign_lock_blocking_attempts);
    let io_delta = totals_after
        .io_requests
        .saturating_sub(totals_before.io_requests);
    let foreign_io_delta = totals_after
        .foreign_io_requests
        .saturating_sub(totals_before.foreign_io_requests);
    let emit_delta = emits_after.saturating_sub(emits_before);

    scenario.note(format!(
        "外线程：阻塞加锁 {FOREIGN_LOCKS} 次（归属 foreign={foreign_lock_delta} / 全局 +{lock_delta}），\
         诊断 {FOREIGN_DIAGS} 次（归属 foreign={foreign_io_delta} / 全局 +{io_delta}），\
         见证 sink 实写 {emit_delta} 次；外线程自己的窗口 lock={} io={}",
        foreign_window.lock_blocking_attempts, foreign_window.io_requests
    ));
    report.scenario(
        "⑪",
        "[MUST-GATE-001] 实时窗口跨外线程活动：四元组仍全 0",
        &scenario,
    );

    report.assert(
        "⑪c",
        "线程归属：外线程的操作落在 foreign_* 桶里，且它真的发生了（不是空转）",
        foreign_finished
            && foreign_lock_delta == FOREIGN_LOCKS
            && foreign_io_delta == FOREIGN_DIAGS
            && emit_delta == FOREIGN_DIAGS
            && foreign_window.lock_blocking_attempts == 0
            && foreign_window.io_requests == 0,
        format!(
            "握手完成={foreign_finished}；foreign 加锁={foreign_lock_delta}（要求 {FOREIGN_LOCKS}）\
             foreign 诊断={foreign_io_delta}（要求 {FOREIGN_DIAGS}）见证 sink 实写={emit_delta}\
             （要求 {FOREIGN_DIAGS}）；外线程窗口 lock={} io={}（要求 0/0：它没开窗口）",
            foreign_window.lock_blocking_attempts, foreign_window.io_requests
        ),
    );
}

// ---------------------------------------------------------------------------
// 判据 ⑫ 控制面读取 EngineStats 的**读取路径**：无锁、零分配、零 I/O
// ---------------------------------------------------------------------------

/// `[MUST-GATE-001]` 的延伸：**读**引擎健康读数这件事本身不许把实时路径拖下水。
///
/// 背景（`needs` N2/N5 的落地）：`EngineStats` 现在多了退役队列的
/// `pending` / `drained` / `pruned` 与"释放线程归属"（跨线程原子量）。
/// 控制面要在 60Hz 循环里每帧读它 —— 如果读它需要加锁或分配，那么"控制面能看见"
/// 就会以"渲染路径被 Q 读者拖慢"为代价。因此这里断言：**每量子读一次 `stats()`**
/// 的窗口里四元组仍全 0，且探针位置见证照旧成立（`visits == quanta`）。
///
/// ⚠ 本判据的**注入口径**（实测，见 notes 注入 E3）：让读取路径变红的是
/// `Vec::with_capacity(1)`（真的分配），**不是** `Vec::new()` —— 后者容量为 0、
/// 不触碰分配器。这条实测写进 ⑫b，避免后人照着"加一次 `Vec::new()`"去做无效注入。
fn stats_read_path(report: &mut Report) {
    let project = filled_project();
    let mut rig = Rig::new(&project, 1, 4096);
    rig.preheat();
    rig.step();

    let mut last = rig.stats();
    let mut scenario = Scenario::new("⑫统计面读取");
    let reading = window(|| {
        for _ in 0..STATS_QUANTA {
            rig.step();
            // 真的读一次（`black_box` 防优化掉），并断言**只读**：读数不回退。
            let stats = std::hint::black_box(rig.stats());
            assert!(
                stats.quanta >= last.quanta,
                "读 stats() 不得让引擎读数回退（{} < {}）",
                stats.quanta,
                last.quanta
            );
            last = stats;
        }
    });
    scenario.absorb(STATS_QUANTA, &reading);
    scenario.note(format!(
        "窗口内读了 {} 次 stats()（含退役队列原子量）；最后 quanta={} retire_pending={} \
         release_thread_is_main={} foreign_drains={}",
        STATS_QUANTA,
        last.quanta,
        last.retire_pending,
        last.release_thread_is_main,
        last.foreign_drains
    ));
    report.scenario(
        "⑫",
        "控制面读取 EngineStats：每量子读一次，四元组仍全 0（无锁 / 零分配 / 零 I-O）",
        &scenario,
    );

    // ---- ⑫b：注入口径实测（哪种注入真的能让 ⑫ 变红）----
    let vec_new = window(|| {
        std::hint::black_box(Vec::<u8>::new());
    });
    let vec_capacity = window(|| {
        std::hint::black_box(Vec::<u8>::with_capacity(1));
    });
    report.assert(
        "⑫b",
        "注入口径：`Vec::new()` **不分配**（无效注入），`Vec::with_capacity(1)` 分配 1 次（有效注入）",
        vec_new.quad.allocations == 0
            && vec_new.quad.deallocations == 0
            && vec_capacity.quad.allocations == 1
            && vec_capacity.quad.deallocations == 1,
        format!(
            "`Vec::new()` 窗口[{}]；`Vec::with_capacity(1)` 窗口[{}]",
            vec_new.quad.describe(),
            vec_capacity.quad.describe()
        ),
    );
}

// ---------------------------------------------------------------------------
// 仪器自检（"一个永远不会红的测量工具比没有测量更糟"）
// ---------------------------------------------------------------------------

fn instrument_self_check(report: &mut Report) {
    // 分配器：窗口内分配 1 次必须被看见（⑩b 已证）；这里证明**释放**也被看见。
    let dropped = window(|| {
        let buffer = Vec::<u8>::with_capacity(128);
        std::hint::black_box(&buffer);
        drop(buffer);
    });
    report.assert(
        "S",
        "仪器自检：分配器在窗口内能看见 1 次分配 + 1 次释放（有判别力）",
        dropped.quad.allocations == 1 && dropped.quad.deallocations == 1,
        format!(
            "自检窗口[{}]（若这里是 0，后面的所有 0 都没有意义）",
            dropped.quad.describe()
        ),
    );

    // ---- 探针自身的陷阱：**新** Mutex 的首次加锁在 macOS 上会分配一次 64 字节 ----
    //
    // 实测（本机 `rustc -O` 独立程序，见 notes §6）：10 个新 `Mutex` 各自的首次
    // `try_lock` 全部 `alloc=1 bytes=64`，同一实例第二次为 0。
    // 因此"探针必须在窗口之外先温暖一次"是**必要条件**：否则探针自己就会制造
    // 它要检测的那次分配（第一版实测 ① 窗口 `allocations=1` 的来源就是探针）。
    //
    // 首触开销是**平台相关**的（Linux 上可能恒为 0）⇒ 只**打印**冷读数，
    // 断言的是"温暖之后恒为 0"这条与平台无关的不变式。
    let cold_probe = RtLockProbe::new();
    let cold = window(|| {
        let _ = std::hint::black_box(cold_probe.try_lock_once());
    });
    let warm = window(|| {
        let _ = std::hint::black_box(cold_probe.try_lock_once());
    });
    report.assert(
        "T",
        "探针自身的陷阱已登记：新锁首触可能分配一次（平台相关，读数只打印），温暖之后恒为 0",
        warm.quad.allocations == 0 && warm.quad.deallocations == 0 && cold.quad.allocations <= 1,
        format!(
            "冷窗口[{}]（macOS 实测应为 alloc=1 bytes=64）暖窗口[{}]（必须为 0）",
            cold.quad.describe(),
            warm.quad.describe()
        ),
    );
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let mut report = Report::new();

    // 主线程就是本判据的"实时线程"：声明之后，别的线程的操作落进 `foreign_*` 桶。
    rt_probe::declare_rt_thread();
    let witness = install_witness_sink();

    instrument_self_check(&mut report);
    window_semantics(&mut report);
    scenario_pure_render(&mut report);
    scenario_snapshot_swap(&mut report);
    scenario_transport(&mut report);
    scenario_metering(&mut report);
    scenario_automation(&mut report);
    scenario_mix_chain(&mut report);
    scenario_callback_buffer_edges(&mut report);
    scenario_declared_audio_config_switch(&mut report);
    scenario_undo_churn_while_playing(&mut report);
    scenario_event_flood(&mut report);
    scenario_pdc_delay_lines(&mut report);
    scenario_retire_queue_overflow(&mut report);
    scenario_meter_capacity_overflow(&mut report);
    probe_teeth(&mut report, &witness);
    thread_attribution(&mut report, &witness);
    stats_read_path(&mut report);

    // 见证文件是临时产物：读完就删（不留垃圾，也不进仓库）。
    let _ = std::fs::remove_file(&witness.path);

    report.finish()
}
