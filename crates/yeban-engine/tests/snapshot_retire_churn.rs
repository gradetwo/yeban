//! `[MUST-GATE-012]` 的**运行期**判据：高频交换压测下音频线程**零释放**、
//! 所有旧快照都在**主线程 60Hz 循环**中安全释放、退役队列**零泄漏**。
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.1 第 12 条）：
//!
//! > **[MUST-GATE-012] 音频退役回收队列零泄漏**：**高频交换压测**下，音频线程**无任何堆释放**，
//! > 所有旧快照均在主线程 **60Hz 循环**中安全释放。
//!
//! 规范来自 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.2（`[ARCH-RT-002]`）：
//! 音频线程把旧快照 `move` 进无锁 SPSC 队列，**主线程以 60Hz 轮询出队并负责 Drop**。
//!
//! # 这条判据把规范的三句话逐句变成可测的量
//!
//! | 规范里的词 | 本文件的判据 | 仪器 |
//! | :--- | :--- | :--- |
//! | 高频交换压测 | ≥ 50 000 次快照交换（5 轮 × 10 000）+ ≥ 52 560 个渲染量子 | `SnapshotSlot::publish` + 逐次切换握手 |
//! | 音频线程无任何堆释放 | 实时线程窗口内 `deallocations == 0` | 计数型全局分配器，**线程局部**武装 |
//! | （更强）无任何堆分配 | 同窗口 `allocations == 0`（单独报告，来源必须点名） | 同一个分配器 |
//! | 释放发生在**主线程** | ① 实时线程 `EngineSnapshot` 析构计数 `== 0`；② 归属窗口内"全局释放数 == 主线程释放数"；③ `RetireQueue::release_thread() == 主线程` 且 `foreign_drains == 0` | `snapshot::release_probe` + 分配器 |
//! | 60Hz 循环排空 | 节拍 = 每 **5 个量子**（128 帧 × 5 ÷ 48 kHz = **13.3 ms**，标称 75 Hz）；实测**平均**间隔 ≤ 6 个量子 = 16.0 ms ≤ 16.67 ms（⇒ 实际不慢于 60 Hz）；`pending` 有界并最终 `== 0` | `RetireQueue::drain` + `AUDIO_QUANTA` 音频时钟 |
//! | 零泄漏 | **等式**：`创建数 == 释放数 + 存活数`，且 `queue.pending() == 0 && slot.pending_len() == 0` | `release_probe::total()` 差值 |
//! | 控制面**看得见**这些读数 | `EngineStats` 的退役镜像与队列/槽的权威读数**逐项相等**（`pending`/`drained`/`drain_calls`/`pruned`/释放线程归属/`foreign_drains`/`stash`） | 音频线程退出循环后读一次 `EngineStats`（此刻主线程阻塞在 `join` ⇒ 同一静止时刻）+ 队列自身的读数 |
//! | （`line/engine-mirror-race`）**镜像不漂** | ① 静止点上 `镜像 == 权威`（逐项**等号**，无容差）；② 静止点前后各读一次全套读数 ⇒ 必须逐项相等（"静止"是**测出来**的）；③ 见证：精确待回收 ≥ 尾段积压、`drained`/`drain_calls`/`pruned` 严格 > 0；④ `pushed − drained == 精确待回收` | 静止点上 `Arc::strong_count(accounting) == 1`（生产端已不存在）+ `RetireAccounting` 的两个单调量 |
//!
//! ⚠ 判据 ① 是**本线修掉的那个真 bug** 的守门人：第一版 `RetireAccounting` 把
//! `pending` 做成"`drain` 后覆写为真实剩余"的量规，而那个覆写与音频线程的 `+1`
//! **不原子** ⇒ 静止点上镜像会**永久**少记一条（CI 原文：
//! `EngineStats 镜像与队列读数不一致：retire_pending 镜像=512 权威=513`）。
//! 定位证据 / 修法 / 注入见 `docs/ledger/engine-mirror-race-notes.md`。
//!
//! `line/engine-stats` 落地的就是最后一行：`needs` N2（`stash_events` 进统计面）与
//! N5（释放线程归属进统计面）在这里被**逐项对账**（不是"大概一致"）。
//! 语义与 needs 见 `docs/ledger/engine-stats-notes.md`。
//!
//! # 为什么必须 `harness = false`
//!
//! 与 [`rt_zero_alloc`](rt_zero_alloc.rs) / [`transport_rt_zero_alloc`](transport_rt_zero_alloc.rs)
//! 同因：`#[global_allocator]` 每个二进制只能定义一次，libtest 自己会在别的线程里分配/释放
//! ⇒ 窗口数字会被**判据自己**污染（教训 L22 的实测：`allocations=9 deallocations=3`）。
//!
//! 本文件与它们的**关键差别**：这里的被测对象是**真的另一个 OS 线程**（`std::thread::spawn`），
//! 因此分配计数器必须**按线程**武装（`thread_local`），否则主线程在窗口内的
//! `publish` / `SnapshotSlot` 分配会被算到"音频线程"头上（那会制造假红），
//! 而主线程的 `drain` 释放也会被算成"音频线程释放"（那会制造**假绿的反面**：无法归因）。
//!
//! # 这条判据怎么变红（注入实测见 `docs/ledger/gate-snapshot-churn-notes.md`）
//!
//! 1. 让音频线程**就地**释放旧快照（把 `SnapshotReader::retire_or_stash` 改成 `drop(old)`）
//!    ⇒ 实时窗口 `deallocations > 0`、`音频线程析构数 > 0`、`创建数 == 释放数 + 存活数` 也破；
//! 2. 让主线程**不排空**（删掉 60Hz 排空）⇒ 轮末 `pending != 0`、对账等式破（存在活的快照）；
//! 3. 让**别的线程**调用 `drain` ⇒ `foreign_drains > 0`；归属窗口的
//!    "全局释放数 == 主线程释放数"也会破；
//! 4. 在 `process_quantum` 调用树里加一次 `Vec::with_capacity(1)` ⇒ `allocations > 0`；
//! 5. （`line/engine-mirror-race`）让**记账少记一次**（`note_push` 第一次不加）
//!    ⇒ 静止点判据 ①/④ 当场红（镜像 511 vs 权威 512）；把**记账多记一次**则反向红。
//!    两个方向的注入记录见 `docs/ledger/engine-mirror-race-notes.md`。
//!
//! # 窗口边界（如实登记）
//!
//! 两个窗口刻意不同边界，各自对应一句规范原文：
//!
//! | 窗口 | 覆盖范围 | 覆盖的是什么规范句子 |
//! | :--- | :--- | :--- |
//! | 分配器（`alloc`/`dealloc`） | **回调路径**：第一个量子之前预热，最后一个量子之后关闭 | 「音频线程**无任何堆释放**」（回调内） |
//! | 快照析构（`release_probe`） | 音频线程**整个生命周期**，含 `EngineRuntime` 析构（读者退场） | 「所有旧快照均在主线程释放」——这句话没有关流例外 |
//!
//! 分配器窗口**不含**读者退场：`EngineRuntime` 析构会释放 SPSC 环自身的内存
//! （`rtrb` 的生产/消费端缓冲），那是**关流路径**的正当释放、不是回调行为。
//! 而"读者手里的旧快照"在退场时**不会**被释放：它仍被 anchor 或写者待回收清单持有
//! （这一条由 `stash_events == 0` + 轮末对账等式共同钉住，并有 stash 注入实测）。
//!
//! 仍然**没有**证明的：① 判定"音频线程"靠的是 `std::thread` 的线程身份，
//! 不是 cpal 的真实回调线程（本机不编译 cpal，见 AGENTS.md §5）；
//! ② "60Hz"是按**音频时钟**（量子数）模拟的节拍，不是墙上时钟定时器 ——
//! 平均间隔是测出来的（≤ 6 个量子 = 16.0 ms），单拍最大间隔受 OS 调度影响，只报告不断言。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::{EngineEvent, TransportCommand, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, release_probe, retire_channel};
use yeban_model::YebanProjectV1;

// ---------------------------------------------------------------------------
// 压测规模（判据常量：改动它们就是改动判据强度，必须同步 notes）
// ---------------------------------------------------------------------------

/// 交换轮数（"分多轮"：每轮一个独立的 槽 + 队列 + 音频线程，独立对账）。
const ROUNDS: usize = 5;
/// 每轮的高频交换次数 ⇒ 总计 [`ROUNDS`] × 10 000 = 50 000 次。
const SWAPS_PER_ROUND: u64 = 10_000;
/// 每轮尾段"只发布、不排空"的交换数：故意制造退役积压，供主线程归属窗口消费。
const BACKLOG_PER_ROUND: u64 = 512;
/// 退役队列容量：必须容得下尾段积压（否则会走 `stash` 寄存路径）。
const RETIRE_CAPACITY: usize = 4096;
/// 一次 `drain` 的上限（主线程排空"整批"）。
const DRAIN_ALL: usize = 1 << 20;
/// 60Hz 排空的节拍：16.67 ms ÷ (128 帧 ÷ 48 kHz = 2.667 ms) = 6.25 个量子。
///
/// 标称取 **5 个量子 = 13.3 ms**（75 Hz）而**不是** 6.25，理由是"检查粒度"：
/// 节拍检查只能在**渲染量子边界**上看到音频时钟，主线程又会在每交换之间离开自旋等待
/// 去构造快照，因此**实测**间隔会比标称多 0.3~1 个量子（实测 6.31 个量子的原因见 notes）。
/// 标称 6 会让实测均值落到 16.8 ms（**比 60 Hz 略慢**）；标称 5 则实测均值 ≈ 5.3 个量子
/// = 14.1 ms，于是"实际排空频率 ≥ 60 Hz"这句话是**测出来的**而不是赚来的。
const QUANTA_PER_60HZ_TICK: u64 = 5;
/// 实测**平均**间隔的上限（量子数）：6 个量子 = 16.0 ms ≤ 16.67 ms ⇒ 实际不慢于 60 Hz。
const MAX_MEAN_QUANTA_PER_TICK: u64 = 6;

// ⚠ **刻意没有**"相邻两次节拍最大间隔"的硬判据。实测教训（本机 M2，同一台机器上还有别的
// 构筑在跑）：这个量在 5 ~ 67 个量子之间跳（67 个量子 = 179 ms），而同一时刻
// `max_pending_before_drain` 始终只有 3 —— 因为主线程被 OS 抢占时**同时也停止了发布**，
// 所以"间隔被拉长"量的是**墙上时钟的调度**，不是排空循环的节拍。
// 它照常**打印**出来（可观测），但判据只钉"平均速率"这条与调度无关的量。
/// 单次交换的等待预算：超过它说明音频线程没在推进（而不是"慢"）⇒ 记失败而不是挂死。
const SWAP_TIMEOUT: Duration = Duration::from_secs(5);
/// "排空前积压"的上限（条）。
///
/// 稳态下 `pending` 远小于此（实测见 notes：6 个量子的节拍 + 每交换约 2 个量子
/// ⇒ 积压通常在个位数）。这个上限是给 **CI runner 的调度抖动**留的余量：
/// 主线程被抢占几百毫秒时积压会涨，但"跟上"这件事的硬保证由
/// `stash_events == 0`（容量 4096）与轮末 `pending == 0` 承担。
const MAX_PENDING_BEFORE_DRAIN: usize = 256;
/// 音频线程每轮最多能跑的量子数上限（防跑飞；同时也是"长时压测"的规模证明）。
const MAX_AUDIO_QUANTA: u64 = 4_000_000;

// 音频线程状态机（单轮内只有主线程写、音频线程读）。
const AUDIO_RUNNING: u8 = 0;
const AUDIO_STOP: u8 = 1;

// ---------------------------------------------------------------------------
// 计数型全局分配器（**按线程**武装）
// ---------------------------------------------------------------------------

struct CountingAllocator;

thread_local! {
    /// 本线程是否在观测窗口内（`const` 初始化 ⇒ 无惰性分配、无析构器 ⇒ 分配器里可安全访问）。
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn armed_here() -> bool {
    // `try_with`（而不是 `with`）：线程 teardown 期间线程局部量不可用，
    // 而此时**绝不能**从分配器里 panic 出去。
    ARMED.try_with(Cell::get).unwrap_or(false)
}

// SAFETY: 每个方法都只是"按线程计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用状态（计数器是线程局部的 `Cell`，访问不分配、不加锁）。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if armed_here() {
            let _ = ALLOCS.try_with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: 由调用方保证 `layout` 合法（`GlobalAlloc` 的契约）。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if armed_here() {
            let _ = DEALLOCS.try_with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: 由调用方保证 `ptr` 来自本分配器且 `layout` 与分配时一致。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if armed_here() {
            // 重新分配在实时路径上同样禁止（可能搬迁并复制），因此也计入分配。
            let _ = ALLOCS.try_with(|count| count.set(count.get().saturating_add(1)));
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// 本线程一个观测窗口的分配读数。
#[derive(Clone, Copy, Debug, Default)]
struct AllocWindow {
    allocations: u64,
    deallocations: u64,
}

fn arm_alloc_window() {
    let _ = ALLOCS.try_with(|count| count.set(0));
    let _ = DEALLOCS.try_with(|count| count.set(0));
    let _ = ARMED.try_with(|armed| armed.set(true));
}

fn close_alloc_window() -> AllocWindow {
    let _ = ARMED.try_with(|armed| armed.set(false));
    AllocWindow {
        allocations: ALLOCS.try_with(Cell::get).unwrap_or(0),
        deallocations: DEALLOCS.try_with(Cell::get).unwrap_or(0),
    }
}

// ---------------------------------------------------------------------------
// 线程间握手（单轮内只有一个音频线程 ⇒ 普通静态量即可，且**零分配**）
// ---------------------------------------------------------------------------

static AUDIO_STATE: AtomicU8 = AtomicU8::new(AUDIO_RUNNING);
/// 音频线程最近一次**已完成**的块所看到的快照 revision。
static LAST_REVISION: AtomicU64 = AtomicU64::new(1);
/// 音频线程累计完成的渲染量子数（= 主线程 60Hz 排空的"音频时钟"）。
static AUDIO_QUANTA: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// 一轮压测
// ---------------------------------------------------------------------------

/// 一轮压测的参数。
#[derive(Clone, Copy, Debug)]
struct RoundSpec {
    label: &'static str,
    swaps: u64,
    backlog: u64,
    /// 是否在**播放中**交换快照（借 `transport_rt_zero_alloc` 的既有 harness：`Play` + `SeekTicks`）。
    transport: bool,
}

/// 音频线程在窗口内测到的全部读数（由 `join` 交回主线程；**窗口外**才能构造/打印）。
#[derive(Clone, Copy, Debug, Default)]
struct AudioReport {
    thread_id: Option<ThreadId>,
    window: AllocWindow,
    /// 本线程在窗口内跑过的 `EngineSnapshot` 析构次数（**必须为 0**）。
    snapshot_releases: u64,
    quanta: u64,
    switches: u64,
    stash_events: u64,
    transport_commands: u64,
    transport_quanta: u64,
    elapsed_ms: u128,
    /// **`EngineStats` 的退役读数镜像**（`line/engine-stats` 的交付：`needs` N2/N5）。
    ///
    /// 它在音频线程**退出循环之后**读一次（此时主线程正阻塞在 `join` 上，
    /// 也就是说：镜像与队列/槽的权威读数**处于同一个静止时刻**）⇒ 下面的逐项对账
    /// 是等号，而不是"差不多"。
    mirror: RetireMirror,
}

/// `EngineStats` 里的退役队列 / 释放线程读数（与队列自身的读数逐项对账）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RetireMirror {
    pending: u64,
    drained: u64,
    drain_calls: u64,
    pruned: u64,
    release_thread_is_main: bool,
    foreign_drains: u64,
    stash_events: u64,
}

/// 一轮压测的全部读数 + 该轮的失败项。
#[derive(Debug, Default)]
struct RoundReport {
    label: String,
    swaps: u64,
    publishes: u64,
    audio: AudioReport,
    /// 主线程 60Hz 循环排空次数与回收条数。
    drain_calls: u64,
    drained_during_churn: u64,
    pruned_during_churn: u64,
    /// 尾段积压（主线程归属窗口的输入）。
    backlog_before_attribution: usize,
    drained_in_attribution: usize,
    pruned_in_attribution: usize,
    /// 归属窗口 A（只 `prune`）：全局释放数 vs **主线程**释放数（必须相等）。
    released_in_prune: u64,
    watched_in_prune: u64,
    prune_window: AllocWindow,
    /// 归属窗口 B（只 `drain`，此时队列里每条都是**最后一个强引用**）：
    /// 全局释放数 / 主线程释放数 / 实际出队条数 —— 三者必须**逐一相等**。
    released_in_drain: u64,
    watched_in_drain: u64,
    drain_window: AllocWindow,
    /// 两个归属窗口合计（打印用）。
    released_in_attribution: u64,
    /// 排空节拍。
    max_quanta_between_drains: u64,
    max_pending_before_drain: usize,
    /// 60Hz 节拍**触发次数**（不管队列当时是否有货；空队列的 `drain` 不计入
    /// `RetireQueue::drain_calls`，所以节拍本身要单独数）。
    tick_triggers: u64,
    /// 高频交换段结束时的音频时钟读数（"平均排空间隔"判据的分母）。
    churn_quanta: u64,
    /// 零泄漏对账。
    created: u64,
    released_total: u64,
    live: usize,
    queue_pending: usize,
    slot_pending: usize,
    release_thread_is_main: bool,
    foreign_drains: u64,
    /// 音频线程读到的 `EngineStats` 退役读数镜像（与 `queue_*` / `release_thread_is_main`
    /// / `foreign_drains` **逐项对账**；见 `run_round` 里的对账块）。
    mirror: RetireMirror,
    /// 静止点见证（`line/engine-mirror-race`）：
    /// ① 静止点前后两次全套读数是否**逐项相等**（真的静止）；
    /// ② 静止点上 `RetireAccounting` 的 `Arc` 强引用数（必须为 1 ⇒ 生产者端已不存在）；
    /// ③ 静止点上队列的**精确**待回收条数（判据 ③ 的非平凡见证）。
    static_double_read_equal: bool,
    static_producer_ends: usize,
    static_exact_pending: usize,
    /// 最后一个"应该什么都不剩"的窗口的读数。
    quiet_window: AllocWindow,
    quiet_released: u64,
    wall_ms: u128,
    /// 音频时间等价长度（量子 × 128 帧 ÷ 48 kHz）。
    audio_seconds: f64,
    failures: Vec<String>,
}

/// 在预算内等待音频线程观察到 `target` revision；超时返回 `false`（**不挂死**）。
///
/// # 为什么 60Hz 排空循环长在这里面
///
/// "主线程 60Hz 循环"的节拍必须**与交换速率解耦**：若只在每次交换之后检查一次，
/// "一次交换里跑过几个量子"就决定了排空间隔。把节拍检查放进**自旋等待**
/// （主线程此刻唯一在做的事）之后，排空间隔由**音频时钟**决定 ⇒ 标称
/// [`QUANTA_PER_60HZ_TICK`] 个量子（13.3 ms），实测平均 ≤
/// [`MAX_MEAN_QUANTA_PER_TICK`] 个量子（16.0 ms ≤ 16.67 ms）。
fn wait_for_revision(
    target: u64,
    budget: Duration,
    queue: &mut yeban_engine::snapshot::RetireQueue,
    slot: &SnapshotSlot,
    report: &mut RoundReport,
    last_drain_quanta: &mut u64,
) -> bool {
    let started = Instant::now();
    let mut spins = 0u32;
    loop {
        // ---- 60Hz 节拍：音频时钟每走过 QUANTA_PER_60HZ_TICK 个量子（13.3 ms）排空一次 ----
        let quanta = AUDIO_QUANTA.load(Ordering::Acquire);
        let gap = quanta.saturating_sub(*last_drain_quanta);
        if gap > report.max_quanta_between_drains {
            report.max_quanta_between_drains = gap;
        }
        if gap >= QUANTA_PER_60HZ_TICK {
            report.tick_triggers += 1;
            let pending = queue.pending();
            if pending > report.max_pending_before_drain {
                report.max_pending_before_drain = pending;
            }
            report.drained_during_churn += queue.drain(DRAIN_ALL) as u64;
            report.pruned_during_churn += slot.prune() as u64;
            *last_drain_quanta = quanta;
        }
        if LAST_REVISION.load(Ordering::Acquire) >= target {
            return true;
        }
        std::hint::spin_loop();
        spins = spins.wrapping_add(1);
        if spins.is_multiple_of(4096) && started.elapsed() > budget {
            return false;
        }
    }
}

/// 等一次交换被观察到，但**不**排空（尾段积压阶段专用）。
fn wait_for_revision_inert(target: u64, budget: Duration) -> bool {
    let started = Instant::now();
    let mut spins = 0u32;
    while LAST_REVISION.load(Ordering::Acquire) < target {
        std::hint::spin_loop();
        spins = spins.wrapping_add(1);
        if spins.is_multiple_of(4096) && started.elapsed() > budget {
            return false;
        }
    }
    true
}

/// 跑一轮：独立的 槽 / 退役队列 / 音频线程 / 对账。
fn run_round(spec: &RoundSpec, project: &YebanProjectV1, main_thread: ThreadId) -> RoundReport {
    let mut report = RoundReport {
        label: spec.label.to_owned(),
        ..RoundReport::default()
    };
    let mut failures: Vec<String> = Vec::new();

    let started = Instant::now();

    // 通道与夹具全部在"启动音频线程之前"建好（产品契约：回调内不许建通道）。
    let initial = EngineSnapshot::from_project(project, 1).expect("夹具工程必须能投影成快照");
    let sample_rate = initial.sample_rate();
    let slot: Arc<SnapshotSlot> = SnapshotSlot::new(initial);
    let (retire, mut queue) = retire_channel(RETIRE_CAPACITY);
    let (mut sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);

    let audio_total_before = release_probe::total();
    let scheduled = spec.swaps + spec.backlog;

    AUDIO_STATE.store(AUDIO_RUNNING, Ordering::Release);
    LAST_REVISION.store(1, Ordering::Release);
    AUDIO_QUANTA.store(0, Ordering::Release);

    // ---- 音频线程：预热 → 开窗口 → 渲染循环 → 关窗口 → 交回读数 ----
    let audio_handle = {
        let mut runtime = runtime;
        std::thread::spawn(move || -> AudioReport {
            // 线程 id 在开窗口**之前**取（首次 `thread::current()` 可能惰性初始化）。
            let thread_id = std::thread::current().id();
            let mut output = vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2];

            // 预热：一次性惰性路径（FPU 武装、首次武装快照）先跑完，不许记进窗口。
            runtime.process_quantum(&mut output, 2);
            LAST_REVISION.store(runtime.revision().unwrap_or(1), Ordering::Release);

            // ⚠ 两个窗口的**边界刻意不同**（见模块文档「窗口边界」）：
            //   * 分配器窗口只覆盖**回调路径**（到最后一个量子为止）—— 读者退场会释放
            //     SPSC 环本身的内存，那是**关流路径**的正当释放，不该算进"回调零分配"；
            //   * 快照析构窗口覆盖音频线程的**整个生命周期**（含读者退场）——
            //     因为"旧快照绝不在音频线程释放"这句话没有窗口例外。
            release_probe::reset_current_thread();
            release_probe::watch_current_thread();
            arm_alloc_window();
            let window_started = Instant::now();

            while AUDIO_STATE.load(Ordering::Acquire) == AUDIO_RUNNING {
                runtime.process_quantum(&mut output, 2);
                let quanta = AUDIO_QUANTA.fetch_add(1, Ordering::Release) + 1;
                if quanta > MAX_AUDIO_QUANTA {
                    break;
                }
                if let Some(revision) = runtime.revision()
                    && revision != LAST_REVISION.load(Ordering::Relaxed)
                {
                    LAST_REVISION.store(revision, Ordering::Release);
                }
            }
            let elapsed = window_started.elapsed();
            let window = close_alloc_window();
            let stats = runtime.stats();
            let stash_events = runtime.snapshot_stash_events();
            let mirror = RetireMirror {
                pending: stats.retire_pending,
                drained: stats.retire_drained,
                drain_calls: stats.retire_drain_calls,
                pruned: stats.retire_pruned,
                release_thread_is_main: stats.release_thread_is_main,
                foreign_drains: stats.foreign_drains,
                stash_events: stats.snapshot_stash_events,
            };
            let quanta = AUDIO_QUANTA.load(Ordering::Acquire);
            // 读者退场：`SnapshotReader::drop` 把 `held` / `stash` 交还给各自的最后持有者。
            // 这一步**仍在快照析构窗口内** ⇒ 任何"在音频线程上释放快照"的实现都会被抓到
            // （`stash` 注入实测见 notes）。
            drop(runtime);
            let snapshot_releases = release_probe::released_by_current_thread();
            release_probe::unwatch_current_thread();
            AudioReport {
                thread_id: Some(thread_id),
                window,
                snapshot_releases,
                quanta,
                switches: stats.snapshot_switches,
                stash_events,
                transport_commands: stats.transport_commands,
                transport_quanta: stats.transport_quanta,
                elapsed_ms: elapsed.as_millis(),
                mirror,
            }
        })
    };

    // 等音频线程真的开跑（跑过一个量子），否则第一次 publish 的握手会立刻成功但没意义。
    // 带预算：音频线程若 panic，这里必须**记失败**而不是挂死。
    let startup_deadline = Instant::now();
    while AUDIO_QUANTA.load(Ordering::Acquire) == 0 {
        if startup_deadline.elapsed() > SWAP_TIMEOUT {
            failures.push("音频线程在预算内没有跑出一个量子".to_owned());
            break;
        }
        std::hint::spin_loop();
    }

    if spec.transport {
        let accepted = sender.publish(&[EngineEvent::Transport {
            command: TransportCommand::Play,
        }]);
        if accepted != 1 {
            failures.push("走带 Play 命令没被事件通道接受（通道太小？）".to_owned());
        }
    }

    // ---- 高频交换 + 主线程 60Hz 排空 ----
    let mut revision = 1u64;
    let mut last_drain_quanta = 0u64;
    let mut transport_commands_sent = u64::from(spec.transport);
    let mut handshake_failures = 0usize;

    for index in 0..scheduled {
        let in_backlog = index >= spec.swaps;
        if index == spec.swaps {
            // 高频交换段的终点（音频时钟读数），供"平均排空间隔"判据使用。
            report.churn_quanta = AUDIO_QUANTA.load(Ordering::Acquire);
        }
        revision += 1;
        let snapshot = match EngineSnapshot::from_project(project, revision) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                failures.push(format!("revision {revision} 投影失败: {error}"));
                break;
            }
        };
        // 发布在窗口之外：控制线程**允许**分配（规范只约束音频线程）。
        slot.publish(snapshot);

        // 等音频线程观察到本次交换；**这段自旋等待就是主线程 60Hz 排空循环的本体**
        // （尾段刻意不排空 ⇒ 制造退役积压）。
        let observed = if in_backlog {
            wait_for_revision_inert(revision, SWAP_TIMEOUT)
        } else {
            wait_for_revision(
                revision,
                SWAP_TIMEOUT,
                &mut queue,
                &slot,
                &mut report,
                &mut last_drain_quanta,
            )
        };
        if !observed {
            failures.push(format!(
                "音频线程在 {:?} 内没有观察到 revision {revision}（交换握手失败）",
                SWAP_TIMEOUT
            ));
            handshake_failures += 1;
            if handshake_failures > 8 {
                break;
            }
        }

        if spec.transport && !in_backlog && index % 500 == 0 {
            let accepted = sender.publish(&[EngineEvent::Transport {
                command: TransportCommand::SeekTicks(index * 37),
            }]);
            if accepted == 1 {
                transport_commands_sent += 1;
            } else {
                failures.push(format!("走带 SeekTicks(index={index}) 未被接受"));
            }
        }
    }
    report.churn_quanta = AUDIO_QUANTA.load(Ordering::Acquire);

    // ---- 尾段：只发布不排空，制造退役积压（归属窗口的输入）----
    report.backlog_before_attribution = queue.pending();

    // ---- 停音频线程并回收窗口读数 ----
    AUDIO_STATE.store(AUDIO_STOP, Ordering::Release);
    let audio = match audio_handle.join() {
        Ok(audio) => audio,
        Err(_) => {
            failures.push("音频线程 panic 了".to_owned());
            AudioReport::default()
        }
    };
    report.audio = audio;
    report.swaps = spec.swaps;
    report.publishes = scheduled;

    // ---- `EngineStats` 镜像与队列/槽的权威读数**逐项对账**（`line/engine-stats` 的判据 ③）----
    //
    // 时刻：音频线程已退出（它在那之前读了镜像），主线程还没做任何 prune/drain ⇒
    // 此刻 `queue.pending()/dropped()/drain_calls()/foreign_drains()` 与 `slot.pruned()`
    // 相对镜像那次读取**一个字都没动** ⇒ 下面每一项都必须是**等号**。
    // 这就是 N5 的"释放线程归属进 EngineStats"与 `gate-snapshot-churn` 探针同口径的证据。
    let pending_after_audio = queue.pending() as u64;
    let pruned_after_audio = slot.pruned();
    let mirror = report.audio.mirror;
    let mirror_checks: [(&str, u64, u64); 5] = [
        ("retire_pending", mirror.pending, pending_after_audio),
        ("retire_drained", mirror.drained, queue.dropped()),
        (
            "retire_drain_calls",
            mirror.drain_calls,
            queue.drain_calls(),
        ),
        ("retire_pruned", mirror.pruned, pruned_after_audio),
        (
            "foreign_drains",
            mirror.foreign_drains,
            queue.foreign_drains(),
        ),
    ];
    for (label, mirrored, authoritative) in mirror_checks {
        if mirrored != authoritative {
            failures.push(format!(
                "EngineStats 镜像与队列读数不一致：{label} 镜像={mirrored} 权威={authoritative}"
            ));
        }
    }
    if mirror.release_thread_is_main != (queue.release_thread() == Some(main_thread)) {
        failures.push(format!(
            "EngineStats 的释放线程归属与队列自身的判据不一致：镜像={} 队列={:?}（主线程={main_thread:?}）",
            mirror.release_thread_is_main,
            queue.release_thread()
        ));
    }
    if mirror.stash_events != report.audio.stash_events {
        failures.push(format!(
            "EngineStats 的 stash 计数与 EngineRuntime::snapshot_stash_events() 不一致：{} vs {}",
            mirror.stash_events, report.audio.stash_events
        ));
    }
    report.mirror = mirror;

    // ---- 静止点见证（判据 ②）：证明"同一静止时刻"不是嘴上说的 ----
    //
    // `line/engine-mirror-race` 新增。上面那次对账的前提是"镜像与权威读数处在同一个
    // 静止时刻"。本线先把那个前提**测出来**，再谈等号：
    //
    //   ① 生产端**已经不存在**：`RetireAccounting` 的 `Arc` 只有队列这一端
    //      （音频线程退出时 `EngineRuntime` 析构把 `RetireProducer` 丢了）
    //      ⇒ 静止点之后**不可能**再有 push，这是结构事实而不是调度猜测；
    //   ② 静止点前后各读一次**全套读数**，两次必须逐项相等
    //      （中间刻意让出 CPU 若干次：真有在飞的 push/drain，这两次就会不同）。
    //
    // ⚠ 第一版（`line/engine-stats`）在这里只写了"音频线程已退出"就断言"同一静止时刻"。
    // 静止点本身没错（`join` 之后确实没有写者），错的是**镜像**：它会在静止点上
    // **永久**漂（见 `src/snapshot.rs` 的 `RetireAccounting` 窗口图与
    // `docs/ledger/engine-mirror-race-notes.md`）。所以判据 ① 的等号是抓真 bug 的那一条，
    // 判据 ② 只是把"这两个数真的是同一时刻的"钉死。
    let snapshot_readings = |queue: &yeban_engine::snapshot::RetireQueue,
                             slot: &SnapshotSlot|
     -> (u64, u64, u64, u64, u64, u64, u64) {
        (
            queue.pending() as u64,
            queue.dropped(),
            queue.drain_calls(),
            queue.foreign_drains(),
            slot.pruned(),
            queue.accounting().pushed(),
            queue.accounting().pending(),
        )
    };
    let static_read_a = snapshot_readings(&queue, &slot);
    for _ in 0..64 {
        std::hint::spin_loop();
    }
    std::thread::yield_now();
    let static_read_b = snapshot_readings(&queue, &slot);
    report.static_double_read_equal = static_read_a == static_read_b;
    if !report.static_double_read_equal {
        failures.push(format!(
            "静止点前后两次读数不相等（静止是假的）：第一次={static_read_a:?} 第二次={static_read_b:?}"
        ));
    }
    report.static_producer_ends = Arc::strong_count(queue.accounting());
    if report.static_producer_ends != 1 {
        failures.push(format!(
            "静止点上还有 {} 个 `RetireAccounting` 强引用（应为 1 = 只剩队列这一端）—— 生产者可能还在",
            report.static_producer_ends
        ));
    }
    // 静止点上的**精确**待回收条数（判据 ③ 的非平凡见证用它，也就是 `pending_after_audio`）。
    report.static_exact_pending = queue.pending();

    // ---- 判据 ③：见证 —— 参与比较的值必须是**非平凡**的 ----
    //
    // "两个 0 相等"和"两个空集合相等"永远为真；没有这条，①的等号可能是**空转的绿**。
    // 这里要求：精确待回收条数 ≥ 尾段积压条数（512 次"只发布不排空"必然制造这么多条），
    // 且累计出队 / 非空 drain 次数 / prune 条数都**严格大于 0**。
    if report.static_exact_pending < spec.backlog as usize {
        failures.push(format!(
            "见证不成立：静止点精确待回收 {} 条 < 尾段积压 {} 条 —— 压测没造出非平凡值",
            report.static_exact_pending, spec.backlog
        ));
    }
    if mirror.drained == 0 || mirror.drain_calls == 0 || mirror.pruned == 0 {
        failures.push(format!(
            "见证不成立：镜像里的 drained={} / drain_calls={} / pruned={} 有 0 —— 比较的是空读数",
            mirror.drained, mirror.drain_calls, mirror.pruned
        ));
    }
    // 同一个静止点上，"入队数 − 出队数"必须**自己**也等于精确读数
    // （这是镜像的构造等式；它把"少记/多记一次"这类真错直接变成红）。
    let pushed_now = queue.accounting().pushed();
    let drained_now = queue.accounting().drained();
    if pushed_now.saturating_sub(drained_now) != pending_after_audio {
        failures.push(format!(
            "静止点上入队/出队账不平：pushed={pushed_now} − drained={drained_now} ≠ 精确待回收={pending_after_audio}"
        ));
    }
    println!(
        "[MUST-GATE-012] 静止点见证「{}」: 精确待回收={} 镜像={} pushed={} drained={} drain_calls={} pruned={} \
         双读相等={} 生产端强引用={}",
        report.label,
        report.static_exact_pending,
        mirror.pending,
        pushed_now,
        drained_now,
        queue.drain_calls(),
        pruned_after_audio,
        report.static_double_read_equal,
        report.static_producer_ends,
    );

    // ---- 主线程归属窗口（拆成两步，"释放发生在哪个线程"才是**精确**的）----
    //
    // 为什么拆开：队列条目与写者侧待回收清单条目是**同一个快照的两条强引用**
    // （`publish` 把旧 anchor 塞进 pending，读者切换时又把同一个 `Arc` 推进队列）。
    // 若在一个窗口里 drain+prune 一起做，**释放**发生在哪一步取决于谁最后放手
    // ⇒ "drain 在别的线程跑"这种注入会被 prune 掩盖（实测：注入 I3 第一版
    // 只靠 `released == watched` 是**抓不住**的，是 `foreign_drains` 抓到的）。
    //
    // 因此：
    //   窗口 A = 只 `prune`：清掉写者侧清单（此时队列还各持一份 ⇒ 多半不释放）；
    //   窗口 B = 只 `drain`：队列里每条都是**最后一个强引用** ⇒
    //             `全局释放数 == 主线程释放数 == 出队条数`，三者逐一相等。
    let prune_total_before = release_probe::total();
    report.prune_window = {
        release_probe::reset_current_thread();
        release_probe::watch_current_thread();
        arm_alloc_window();
        report.pruned_in_attribution = slot.prune();
        let window = close_alloc_window();
        report.watched_in_prune = release_probe::released_by_current_thread();
        release_probe::unwatch_current_thread();
        window
    };
    report.released_in_prune = release_probe::total().saturating_sub(prune_total_before);

    let drain_total_before = release_probe::total();
    report.drain_window = {
        release_probe::reset_current_thread();
        release_probe::watch_current_thread();
        arm_alloc_window();
        report.drained_in_attribution = queue.drain(DRAIN_ALL);
        let window = close_alloc_window();
        report.watched_in_drain = release_probe::released_by_current_thread();
        release_probe::unwatch_current_thread();
        window
    };
    report.released_in_drain = release_probe::total().saturating_sub(drain_total_before);
    report.released_in_attribution = report.released_in_prune + report.released_in_drain;

    // ---- 静默窗口：此时应该什么都不剩（再排一次不该释放任何东西）----
    let quiet_total_before = release_probe::total();
    report.quiet_window = {
        arm_alloc_window();
        let drained = queue.drain(DRAIN_ALL);
        let pruned = slot.prune();
        if drained != 0 || pruned != 0 {
            failures.push(format!(
                "排空之后仍有残留：drain={drained} prune={pruned} —— 60Hz 循环没排干净"
            ));
        }
        close_alloc_window()
    };
    report.quiet_released = release_probe::total().saturating_sub(quiet_total_before);

    report.queue_pending = queue.pending();
    report.slot_pending = slot.pending_len();
    report.release_thread_is_main = queue.release_thread() == Some(main_thread);
    report.foreign_drains = queue.foreign_drains();
    report.drain_calls = queue.drain_calls();
    report.created = 1 + scheduled;
    report.released_total = release_probe::total().saturating_sub(audio_total_before);
    // 存活 = anchor 持有的那一个；`current()` 克隆了一份，所以 strong_count 应为 2。
    let anchor = slot.current();
    report.live = Arc::strong_count(&anchor).saturating_sub(1);

    report.wall_ms = started.elapsed().as_millis();
    report.audio_seconds =
        (report.audio.quanta as f64) * (DEFAULT_BLOCK_FRAMES as f64) / f64::from(sample_rate);

    // -----------------------------------------------------------------------
    // 判据
    // -----------------------------------------------------------------------
    // ① 音频线程窗口内**零释放**（规范点名的硬要求）。
    if report.audio.thread_id == Some(main_thread) {
        failures
            .push("音频线程与主线程是同一个线程 —— 这条判据就没有并行性，必须换真线程".to_owned());
    }
    if report.audio.window.deallocations != 0 {
        failures.push(format!(
            "音频线程窗口内发生了 {} 次堆释放 —— [MUST-GATE-012] 一票否决",
            report.audio.window.deallocations
        ));
    }
    // ② 分配数单独报告（规范只点名释放；分配也应为 0，不为 0 必须点名来源）。
    if report.audio.window.allocations != 0 {
        failures.push(format!(
            "音频线程窗口内发生了 {} 次堆分配 —— `process_quantum` 调用树必须零分配",
            report.audio.window.allocations
        ));
    }
    // ④ 释放**只在主线程**：直接测量"音频线程上跑了几个快照析构"。
    if report.audio.snapshot_releases != 0 {
        failures.push(format!(
            "音频线程上释放了 {} 个快照 —— 旧快照必须经退役队列交回主线程",
            report.audio.snapshot_releases
        ));
    }
    if !report.release_thread_is_main {
        failures.push(format!(
            "退役队列的释放线程不是主线程：release_thread={:?}（主线程={main_thread:?}）",
            queue.release_thread()
        ));
    }
    if report.foreign_drains != 0 {
        failures.push(format!(
            "有 {} 次 `drain` 发生在释放线程之外 —— 释放没有集中在主线程",
            report.foreign_drains
        ));
    }
    // 归属窗口 A（prune）：全局释放数必须**全部**记在主线程账上。
    if report.released_in_prune != report.watched_in_prune {
        failures.push(format!(
            "prune 窗口里有 {} 次释放，但只有 {} 次发生在主线程 —— 释放线程不唯一",
            report.released_in_prune, report.watched_in_prune
        ));
    }
    // 归属窗口 B（drain，纯 drain）：三者逐一相等 ⇒ 每个被出队的旧快照都是**主线程**释放的。
    if report.released_in_drain != report.watched_in_drain
        || report.released_in_drain != report.drained_in_attribution as u64
    {
        failures.push(format!(
            "drain 窗口：出队 {} 条、全局释放 {} 次、主线程释放 {} 次 —— 三者必须相等",
            report.drained_in_attribution, report.released_in_drain, report.watched_in_drain
        ));
    }
    if report.released_in_drain == 0 {
        failures.push("drain 窗口一次释放都没发生 —— 这条判据是空转（假绿）".to_owned());
    }
    if report.pruned_in_attribution < spec.backlog as usize
        || report.drained_in_attribution < spec.backlog as usize
    {
        failures.push(format!(
            "归属窗口清掉的不够多：prune={} drain={}，尾段积压是 {} 条 —— 压测没造出积压",
            report.pruned_in_attribution, report.drained_in_attribution, spec.backlog
        ));
    }
    if report.prune_window.allocations != 0 || report.drain_window.allocations != 0 {
        failures.push(format!(
            "主线程 60Hz 排空窗口内有分配：prune={} drain={} —— 排空路径必须零分配",
            report.prune_window.allocations, report.drain_window.allocations
        ));
    }
    if report.quiet_released != 0 || report.quiet_window.allocations != 0 {
        failures.push(format!(
            "静默窗口仍有动作：released={} allocations={}",
            report.quiet_released, report.quiet_window.allocations
        ));
    }
    // ③ 零泄漏对账等式。
    if report.created != report.released_total + report.live as u64 {
        failures.push(format!(
            "零泄漏对账等式破：创建 {} ≠ 释放 {} + 存活 {}",
            report.created, report.released_total, report.live
        ));
    }
    if report.live != 1 {
        failures.push(format!(
            "轮末存活 {} 个快照（应恰为 anchor 持有的 1 个）—— 有引用没交回",
            report.live
        ));
    }
    // ⑤ 60Hz 排空后队列必须空。
    if report.queue_pending != 0 || report.slot_pending != 0 {
        failures.push(format!(
            "轮末仍有待回收项：queue.pending={} slot.pending={} —— 60Hz 循环没排空",
            report.queue_pending, report.slot_pending
        ));
    }
    // 队列容量必须够 —— 否则音频线程会走 `stash`（不是违规，但会让"高频"打折）。
    if report.audio.stash_events != 0 {
        failures.push(format!(
            "发生了 {} 次退役队列满寄存（容量 {}）—— 高频压测被容量打折",
            report.audio.stash_events, RETIRE_CAPACITY
        ));
    }
    // ⑥ 压测规模自检（防止"窗口里其实没什么都没跑"的假绿）。
    if report.audio.switches < spec.swaps {
        failures.push(format!(
            "音频线程只切换了 {} 次快照，压测要求 ≥ {} 次",
            report.audio.switches, spec.swaps
        ));
    }
    if report.drained_during_churn == 0 {
        failures.push("60Hz 循环一次都没排空 —— 排空路径未被覆盖".to_owned());
    }
    // ⑤ 60Hz 节奏：标称节拍 = 每 [`QUANTA_PER_60HZ_TICK`] 个量子（13.3 ms，标称 75 Hz），
    // 判据钉**平均**速率：`churn_quanta / tick_triggers ≤ MAX_MEAN_QUANTA_PER_TICK`
    // （6 个量子 = 16.0 ms ≤ 16.67 ms ⇒ 实际不慢于 60 Hz）。
    // 用"平均"而不是"最大间隔"的理由见 [`QUANTA_PER_60HZ_TICK`] 旁边的那段实测教训。
    if report
        .tick_triggers
        .saturating_mul(MAX_MEAN_QUANTA_PER_TICK)
        < report.churn_quanta
    {
        failures.push(format!(
            "60Hz 节拍只跑了 {} 拍，但音频时钟走过 {} 个量子 ⇒ 平均间隔 > {} 个量子（16.0 ms）",
            report.tick_triggers, report.churn_quanta, MAX_MEAN_QUANTA_PER_TICK
        ));
    }
    // 积压必须有界：60Hz 循环"跟得上"的直接读数（容量 4096 还是硬上限）。
    if report.max_pending_before_drain > MAX_PENDING_BEFORE_DRAIN {
        failures.push(format!(
            "排空前积压达到 {} 条（上限 {MAX_PENDING_BEFORE_DRAIN}）—— 主线程排空没跟上交换速率",
            report.max_pending_before_drain
        ));
    }
    // ⑦ 走带交互（仅在 transport 轮）。
    if spec.transport {
        if report.audio.transport_commands < transport_commands_sent {
            failures.push(format!(
                "走带只应用了 {} 条命令（发出 {} 条）—— 播放中交换没被真正覆盖",
                report.audio.transport_commands, transport_commands_sent
            ));
        }
        if report.audio.transport_quanta == 0 {
            failures.push("播放中交换：没有任何推进量子（走带根本没在走）".to_owned());
        }
    }

    report.failures = failures;
    report
}

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();
    let main_thread = std::thread::current().id();
    let project = yeban_model::samples::filled_project();

    println!(
        "[MUST-GATE-012] 规模: {ROUNDS} 轮 × {SWAPS_PER_ROUND} 次交换 + 每轮 {BACKLOG_PER_ROUND} 次尾段积压 = {} 次发布（目标 ≥ 50 000）",
        ROUNDS as u64 * (SWAPS_PER_ROUND + BACKLOG_PER_ROUND)
    );

    // ---- 仪器自检 1：分配计数器必须真的能看见分配/释放 ----
    // "一个永远不会红的测量工具比没有测量更糟"（DEVELOPMENT_LEDGER 第 8 轮）。
    arm_alloc_window();
    let probe = Vec::<u8>::with_capacity(64);
    let allocator_selfcheck = close_alloc_window();
    drop(probe);
    arm_alloc_window();
    drop(Vec::<u8>::with_capacity(64));
    let allocator_selfcheck_free = close_alloc_window();
    if allocator_selfcheck.allocations == 0 || allocator_selfcheck_free.deallocations == 0 {
        failures.push(format!(
            "计数分配器没有判别力：alloc={} dealloc={}",
            allocator_selfcheck.allocations, allocator_selfcheck_free.deallocations
        ));
    }
    println!(
        "[MUST-GATE-012] 仪器自检: 分配器 alloc={} dealloc={}",
        allocator_selfcheck.allocations, allocator_selfcheck_free.deallocations
    );

    // ---- 仪器自检 2：释放探针必须真的能看见"本线程释放了一个快照" ----
    release_probe::reset_current_thread();
    release_probe::watch_current_thread();
    let before_total = release_probe::total();
    drop(EngineSnapshot::from_project(&project, 0).expect("自检快照"));
    let probe_releases = release_probe::released_by_current_thread();
    release_probe::unwatch_current_thread();
    let probe_total = release_probe::total().saturating_sub(before_total);
    if probe_releases != 1 || probe_total != 1 {
        failures.push(format!(
            "释放探针没有判别力：本线程释放 {probe_releases}（应 1）、全局 {probe_total}（应 1）"
        ));
    }
    println!("[MUST-GATE-012] 仪器自检: 释放探针 本线程={probe_releases} 全局={probe_total}");

    // ---- 五轮压测（最后一轮在播放中交换，借走带 harness）----
    let specs: Vec<RoundSpec> = (0..ROUNDS)
        .map(|index| {
            let transport = index + 1 == ROUNDS;
            RoundSpec {
                label: if transport {
                    "播放中交换（走带 harness）"
                } else if index == 0 {
                    "高频交换"
                } else {
                    "高频交换（复跑）"
                },
                swaps: SWAPS_PER_ROUND,
                backlog: BACKLOG_PER_ROUND,
                transport,
            }
        })
        .collect();

    let mut total_swaps = 0u64;
    let mut total_backlog = 0u64;
    let mut total_wall_ms = 0u128;
    let mut total_audio_seconds = 0.0f64;
    let mut total_released = 0u64;
    let mut total_created = 0u64;
    let mut total_audio_allocations = 0u64;
    let mut total_audio_deallocations = 0u64;
    let mut total_audio_releases = 0u64;
    let mut total_attribution = 0u64;

    for spec in &specs {
        let report = run_round(spec, &project, main_thread);
        total_swaps += report.swaps;
        total_backlog += report.publishes - report.swaps;
        total_wall_ms += report.wall_ms;
        total_audio_seconds += report.audio_seconds;
        total_released += report.released_total;
        total_created += report.created;
        total_audio_allocations += report.audio.window.allocations;
        total_audio_deallocations += report.audio.window.deallocations;
        total_audio_releases += report.audio.snapshot_releases;
        total_attribution += report.released_in_attribution;

        println!(
            "[MUST-GATE-012] 轮「{}」: swaps={} publishes={} quanta={} audio_s={:.1} wall_ms={} audio_ms={} \
             audio_alloc={} audio_dealloc={} audio_releases={} switches={} stash={} \
             drain_calls={} drained(churn)={} pruned(churn)={} max_pending_before_drain={} \
             max_quanta_between_drains={} tick_triggers={} backlog={} \
             prune(pruned={} released={} watched={} alloc={}) drain(drained={} released={} watched={} alloc={}) \
             created={} released={} live={} queue_pending={} slot_pending={} release_thread_is_main={} foreign_drains={} \
             stats_mirror(pending={} drained={} drain_calls={} pruned={} release_thread_is_main={} foreign_drains={} stash={})",
            report.label,
            report.swaps,
            report.publishes,
            report.audio.quanta,
            report.audio_seconds,
            report.wall_ms,
            report.audio.elapsed_ms,
            report.audio.window.allocations,
            report.audio.window.deallocations,
            report.audio.snapshot_releases,
            report.audio.switches,
            report.audio.stash_events,
            report.drain_calls,
            report.drained_during_churn,
            report.pruned_during_churn,
            report.max_pending_before_drain,
            report.max_quanta_between_drains,
            report.tick_triggers,
            report.backlog_before_attribution,
            report.pruned_in_attribution,
            report.released_in_prune,
            report.watched_in_prune,
            report.prune_window.allocations,
            report.drained_in_attribution,
            report.released_in_drain,
            report.watched_in_drain,
            report.drain_window.allocations,
            report.created,
            report.released_total,
            report.live,
            report.queue_pending,
            report.slot_pending,
            report.release_thread_is_main,
            report.foreign_drains,
            report.mirror.pending,
            report.mirror.drained,
            report.mirror.drain_calls,
            report.mirror.pruned,
            report.mirror.release_thread_is_main,
            report.mirror.foreign_drains,
            report.mirror.stash_events,
        );

        for failure in &report.failures {
            let line = format!("轮「{}」: {failure}", report.label);
            eprintln!("[MUST-GATE-012] FAIL: {line}");
            failures.push(line);
        }
    }

    // ---- 全局规模自检（"高频"必须是真的高频）----
    if total_swaps < 50_000 {
        failures.push(format!(
            "总交换次数 {total_swaps} < 50 000 —— 没达到 [MUST-GATE-012] 的「高频」规模"
        ));
    }
    if total_created != total_released + ROUNDS as u64 {
        failures.push(format!(
            "全局对账等式破：创建 {total_created} ≠ 释放 {total_released} + 存活 {}（每轮 1 个 anchor）",
            ROUNDS
        ));
    }
    if total_audio_deallocations != 0 || total_audio_releases != 0 {
        failures.push(format!(
            "音频线程总计释放 {total_audio_deallocations} 次堆 / {total_audio_releases} 个快照 —— 一票否决"
        ));
    }

    println!(
        "[MUST-GATE-012] 汇总: rounds={ROUNDS} swaps={total_swaps} (+{total_backlog} 尾段积压) \
         wall_ms={total_wall_ms} audio_time_s={total_audio_seconds:.1} \
         音频线程 alloc={total_audio_allocations} dealloc={total_audio_deallocations} \
         快照析构={total_audio_releases}；创建={total_created} 释放={total_released} 存活={ROUNDS}；\
         主线程归属窗口释放={total_attribution}"
    );
    println!(
        "[MUST-GATE-012] 对账等式: 创建 {total_created} == 释放 {total_released} + 存活 {ROUNDS} ⇒ {}",
        if total_created == total_released + ROUNDS as u64 {
            "成立（零泄漏）"
        } else {
            "不成立"
        }
    );

    if failures.is_empty() {
        println!(
            "[MUST-GATE-012] ok: {total_swaps} 次高频交换 / {total_audio_seconds:.1} 秒等价音频时长，\
             音频线程堆释放 0 次、快照析构 0 次；全部旧快照在主线程 60Hz 循环中释放；零泄漏对账成立"
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("[MUST-GATE-012] 失败 {} 项", failures.len());
        ExitCode::FAILURE
    }
}
