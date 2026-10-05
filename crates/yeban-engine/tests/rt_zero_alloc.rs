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
//! # 六个场景（在既有 `harness = false` 风格上扩展）
//!
//! | # | 场景 | 覆盖的实时路径 |
//! | :-: | :--- | :--- |
//! | ① | 纯渲染 10 000 量子（`filled_project`） | 事件批量出队 → 快照边界 → 逐轨合成/电平 → 母线限制器 → 两路 SPSC 发布 |
//! | ② | 快照交换 63 次逐步 + 1 000 次高频 | `SnapshotReader::begin_block` 的原子切换 + 旧快照入退役队列 |
//! | ③ | 走带 200 轮命令 + 2 000 量子播放 + 500 量子停住 | `EngineEvent::Transport` 出队应用、整数 tick 推进、`SeekTicks` 的声部释放、停住分支 |
//! | ④ | 电平计量 10 000 量子 + UI 侧 60Hz 抽干 | 每轨/母线电平状态机 + **每量子恰好一次**批量发布 |
//! | ⑤ | 自动化求值 2 000 量子（每量子一批 `SetParam`） | 控制侧 `automation_value_at` → SPSC → 实时侧出队（**见 §needs：实时侧只计数，不改 DSP**） |
//! | ⑥ | 限制器/混音链 2 000 量子（滤波器 + 声相 + 前瞻限制 + 声部窃取） | `BusLimiter::apply`、声相增益乘加、声部窃取路径 |
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
//! # 本判据怎么变红（④ 组注入，实测记录见 `docs/ledger/gate-rt-zero-alloc-notes.md` §4）
//!
//! | # | 注入点（`crates/yeban-engine/src/`） | 变红的判据 |
//! | :-: | :--- | :--- |
//! | I1 | `rt.rs::render_block` 里加 `drop(rt_probe::rt_path_lock().lock());` | ①~⑥ 的**锁**分量 + ⑦ 的对照 |
//! | I2 | `rt.rs::render_block` 里加 `rt_probe::diag(RtDiagEvent::NoSnapshot);` | ①~⑥ 的 **I/O** 分量（`io_requests`/`io_ops`） |
//! | I3 | `rt.rs::render_block` 里加 `Vec::<u8>::with_capacity(1)` | ①~⑥ 的**分配/释放**分量 |
//! | I4 | 删掉 `render_block` 开头的 `rt_probe::quantum_enter()` | **位置见证**（`quanta_visits != 0` 那条）—— 探针被摘掉就"空转"，判据必须发现 |
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
use yeban_engine::meter::{MeterCollector, MeterFrame, meter_channel};
use yeban_engine::ring::{EngineEvent, EventSender, ParamAddress, TransportCommand, event_channel};
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::rt_probe::{self, RtDiagEvent, RtDiagSink, RtLockProbe};
use yeban_engine::snapshot::{EngineSnapshot, RetireQueue, SnapshotSlot, retire_channel};
use yeban_engine::transport::TransportState;
use yeban_model::samples::filled_project;
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, CurveType, EntityId,
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
            "{}；四元组[{}]；子窗口={} 探针经过={}（期望 {}）试探成功={} 试探失败={}；{}",
            self.label,
            self.quad.describe(),
            self.windows,
            self.visits,
            self.expected_quanta,
            self.try_successes,
            self.try_failures,
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

    /// 六场景共用的判定：四元组全 0 **且** 见证成立。
    fn scenario(&mut self, id: &'static str, title: &'static str, scenario: &Scenario) {
        let ok = scenario.quad.is_zero() && scenario.witness_ok();
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
                "[MUST-GATE-001] ok: 六场景（纯渲染 / 快照交换 / 走带 / 电平计量 / 自动化 / 混音链）\
                 四元组全 0；探针有牙（正对照 + 注入）；线程归属与外线程活动已对账"
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
        let snapshot =
            EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译成快照");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, queue) = retire_channel(64);
        let (sender, receiver) = event_channel(64);
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
    let fixture = tuned_project(
        &notes,
        MixSpec {
            volume_db: 6.0,
            pan: -1.0,
            cutoff_hz: Some(2_000.0),
            resonance: 0.4,
        },
    );

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
    let right_nonzero = rig
        .output
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|sample| **sample != 0.0)
        .count();
    scenario.note(format!(
        "限制器压过={} 最大压限={:.4} 声部窃取={} 触发音符={} NaN={nan} 右声道非零={right_nonzero}",
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
        "覆盖度：限制器真的压过、声部窃取真的发生、声相真的在线、输出无 NaN",
        stats.limiter_gain_reductions > 0
            && stats.voice_steals > 0
            && right_nonzero == 0
            && nan == 0,
        format!(
            "压过样本={}（要求 > 0）窃取={}（要求 > 0）右声道非零={right_nonzero}（要求 0）NaN={nan}（要求 0）",
            stats.limiter_gain_reductions, stats.voice_steals
        ),
    );
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
    probe_teeth(&mut report, &witness);
    thread_attribution(&mut report, &witness);

    // 见证文件是临时产物：读完就删（不留垃圾，也不进仓库）。
    let _ = std::fs::remove_file(&witness.path);

    report.finish()
}
