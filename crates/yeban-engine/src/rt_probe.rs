//! 实时路径的**可插桩边界**：零锁等待与零阻塞 I/O 的**运行期**仪器 `[MUST-GATE-001]`。
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.1 第 1 条）：
//!
//! > **音频线程零安全违规 (Zero Glitch / Zero Alloc)**：实时音频回调函数内严禁出现任何堆内存分配
//! > （`malloc`）、堆释放（`free`/`drop`）、文件/网络 I/O 或互斥锁争用，
//! > **CI 运行时通过内存分配 Hook 进行严格断言**。
//!
//! 本模块补的是这句话里**最后两个词**的运行期证据：`互斥锁争用` 与 `文件/网络 I/O`。
//! 零分配/零释放由既有目标的**计数型全局分配器**承担（`tests/rt_zero_alloc.rs` 等）；
//! 本模块把**锁**与 **I/O** 变成同样可数的量。
//!
//! # 为什么探针住在**产品代码**里，而不在 `tests/` 里
//!
//! "实时路径上没有锁"这句话有两种读法，它们需要**不同**的证据：
//!
//! | 读法 | 证据 | 谁给 |
//! | :--- | :--- | :--- |
//! | 代码里没写 | `grep Mutex crates/yeban-engine/src/rt.rs` | 人/审查（**不是**运行期判据） |
//! | **运行期确实没有** | 探针在**同一条被执行到的路径**上读数为 0 | 本模块 |
//!
//! 第二种证据要求探针位置**真的在实时回调的调用树里** —— 而集成判据（`tests/*.rs`）只能看到
//! crate 的**公共面**，无法把自己的仪器插进 `render_block`。因此探针必须住在
//! `src/`（产品代码）里，并以**公共 API** 暴露读数。
//!
//! 反过来说：一个**安装在测试里、与实时路径无关**的计数器"读到 0"什么也证明不了
//! （它证明的是"没人碰过那个计数器"）。为避免这种空转，本模块提供**见证（witness）**：
//! 每次量子（[`quantum_enter`]）都会在**同一条热路径**上真的去操作那把锁
//! （`Mutex::try_lock`，**不等待**）并把成功次数记进窗口 ——
//! 于是判据可以同时断言"**探针被跑到过**（`quanta_visits == 量子数`）
//! 且它**真的能锁**（`lock_try_successes == quanta_visits`）"，
//! 而"阻塞锁 0 次、等待 0 次"就从"没写"升级为"在**确证被执行**的那段代码里没有发生"。
//!
//! # 两层计数器：线程窗口 + 进程总量（外加"外线程"桶）
//!
//! | 层 | 存储 | 语义 |
//! | :--- | :--- | :--- |
//! | **窗口** | `thread_local!` + `const` 初始化（无惰性分配、无析构器） | 只在 [`watch_current_thread`] 打开的**那个线程**上累加 |
//! | **总量** | `AtomicU64` | 进程内所有线程的累计（用于"别人的操作确实发生过"的对照） |
//! | **外线程** | `AtomicU64` | 来自**非声明 RT 线程**的操作（[`declare_rt_thread`]），用来防"把别人的锁算到 RT 头上" |
//!
//! 窗口是**线程局部**的：判据在实时线程上开窗口，别的线程在自己的窗口里活动，
//! 两者**物理上不可能**串账（不是靠约定，是靠存储位置）。
//!
//! # 成本（为什么可以留在产品代码里）
//!
//! | 情形 | 每量子成本 |
//! | :--- | :--- |
//! | 未武装（**发布构建的常态**） | 一次线程局部 `Cell::get`（≈ 1 ns，无分配、无原子、无系统调用） |
//! | 已武装（判据窗口内） | 同上 + 三次线程局部读写 + 一次 `Mutex::try_lock`（无争用时是单次 CAS）+ 一次 `Relaxed` 原子加 |
//!
//! 无论哪种情形都**不分配、不等待、不做系统调用**。这一点不是靠这段文字自证的：
//! `tests/rt_zero_alloc.rs` 的 10 000 量子窗口**同时**断言分配计数为 0，
//! 而那段窗口里 `quantum_enter` 每量子都被执行（`quanta_visits == 10000`）
//! ⇒ 探针自身的开销被**其它探针**证明是实时安全的（互相作证）。
//!
//! # 探针自身的陷阱：**新锁的首次加锁会分配**（macOS 实测，必须先在窗口外温暖）
//!
//! 在 macOS 上，**每个 `std::sync::Mutex` 实例的首次加锁都会堆分配一次 64 字节**
//! （本机用独立程序实测：10 个新 `Mutex` 各自的首次 `try_lock` 全部 `alloc=1 bytes=64`，
//! 同一实例的第二次为 `alloc=0`；跨线程也只在**首次**分配一次）。
//! 这直接威胁本模块：探针的见证动作就是"每量子 `try_lock` 一次"，
//! 若那把锁在**窗口内**才第一次被碰到，探针就会亲手制造它要检测的那次分配
//! （第一版实测：10 000 量子窗口 `allocations=1`，来源是探针自己，而不是引擎代码）。
//!
//! 因此[`declare_rt_thread`] 会在**窗口之外**先 [`warm_up`] 一次。
//! 同上，判据若自己新建 `RtLockProbe`，也必须先 `try_lock_once()` 一次再开窗口。
//! 附带的好处：这条平台行为也让"实时路径里新塞一把**裸** `std::sync::Mutex`"
//! 在**首次**触碰时被分配探针抓住（见 notes §5 的盲区证据 G2）。
//!
//! # 诚实边界（**这两条必须和判据一起读**）
//!
//! 1. **不是系统调用级拦截**。两个探针都只看得见"**经由本边界**"的操作：
//!    - 锁：只有走 [`RtLockProbe`] 的加锁才被计数。在实时路径里直接写
//!      `std::sync::Mutex::lock()`（**裸** `std::sync::Mutex`）**看不见** ——
//!      无争用时它只是一个原子 CAS，根本不进内核，进程内没有任何钩子能观测它；
//!    - I/O：只有走 [`diag`] 的诊断/I/O 才被计数。实时路径里直接 `println!` /
//!      `eprintln!` / `std::fs::*` / socket **看不见**（同样没有 syscall 钩子）。
//! 2. 因此本边界是**约定 + 判据**的组合拳，而不是内核级防火墙：
//!    约定是"实时路径上的诊断与锁必须走本模块"（`render_block` /
//!    `SnapshotReader::retire_or_stash` 的调用点就是这样接的），
//!    判据是"一旦走了本模块，就必须为 0"。
//!    另有一条**结构性**防线兜住"新写一把锁"这种改动：`scripts/guards/policy_check.py`
//!    与代码审查看的是源码形状；本模块把"运行期真的发生了"这一半补齐。
//! 3. [`diag`] **不能阻止** I/O，它只把"I/O 发生了"变成可数的读数。真正的处置在调用方：
//!    **实时回调的调用树里不许出现 [`diag`]**（见下一节）。
//!
//! # `N6` 裁决 = **选项 A**：实时回调上的诊断只记数，`diag` 留给非实时路径
//!
//! 历史（**实测**，`docs/ledger/gate-rt-zero-alloc-notes.md` §13）：实时回调的调用树里
//! 曾有 **5 个** [`diag`] 调用点 —— `rt.rs::render_block` 的电平容量溢出 ×3
//! （"轨道数 > 预算"的预检、`MeterBank::measure` 返回 `None`、母线放不下）、
//! 无快照分支 ×1，以及 `SnapshotReader::retire_or_stash` 的退役队列欠容 ×1。
//! 发布构建**不安装 sink** ⇒ 那时它们只是原子自增；
//! 但只要有人给发布构建装了写文件的 sink，**这些路径就是音频线程上的真实阻塞 I/O**
//! （本机实测：容量不足时 `io_requests == io_ops` 为 `400` / `10000`）。
//!
//! 选项 A 的处置：实时路径上的诊断改走 [`note_suppressed`] —— 一个**不读**
//! [`SINK`] 的纯计数入口。于是"实时回调不做 I/O"这句话对**实现**成立，
//! 而不是只对"今天没装 sink"这个配置成立。代价（负责人已接受）：溢出路径
//! **丢掉 sink 里那条逐事件记录**；留下的**计数**在
//! [`RtProbeTotals::suppressed_diag_events`] / [`suppressed_by_kind`]、
//! 以及那两条路径自己的领域读数
//! [`crate::rt::EngineStats::meter_capacity_drops`] /
//! [`crate::rt::EngineStats::snapshot_stash_events`] 里（都可从非 RT 线程查询）。
//!
//! ⚠ 这条裁决**没有**把 [`diag`] 变成私有/删除：它仍是"唯一 I/O 边界"，
//! 非实时路径（控制面自己的诊断）与判据的**正对照**（⑦c 证明边界真的有牙）都要用它。
//! 判据一侧的机械形式是"实时窗口里 `io_requests == 0 && io_ops == 0`"照旧成立，
//! **同时**要求溢出场景里 `suppressed_diag_events > 0`（那条纯计数出口真的被走到过）。
//! 4. **不覆盖** cpal/系统库内部自己的日志与内存操作、OS 缺页、`mmap`、
//!    以及"未来的采样器从磁盘流式读"这类尚未实现的路径 —— 它们不在本 crate 的调用树里。
//!    详见 `docs/ledger/gate-rt-zero-alloc-notes.md` §5 的覆盖范围表。

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread::LocalKey;

// ---------------------------------------------------------------------------
// 线程局部窗口（`const` 初始化 ⇒ 无惰性分配、无 TLS 析构器）
// ---------------------------------------------------------------------------

thread_local! {
    /// 本线程是否在观测窗口内。
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// 窗口内 [`quantum_enter`] 被调用的次数（"探针位置确实被跑到"的证据）。
    static QUANTA_VISITS: Cell<u64> = const { Cell::new(0) };
    /// 窗口内 [`RtLockProbe::try_lock_once`] **成功**取得的次数（见证：探针真的能操作锁）。
    static LOCK_TRY_SUCCESSES: Cell<u64> = const { Cell::new(0) };
    /// 窗口内 [`RtLockProbe::try_lock_once`] 遇到"已被持有"的次数（`try` 不等待，不算违规）。
    static LOCK_TRY_FAILURES: Cell<u64> = const { Cell::new(0) };
    /// 窗口内 **阻塞式** 加锁尝试次数（**必须为 0**）。
    static LOCK_BLOCKING: Cell<u64> = const { Cell::new(0) };
    /// 窗口内阻塞式加锁**真的等过**（发现已被别的线程持有）的次数（**必须为 0**）。
    static LOCK_WAITS: Cell<u64> = const { Cell::new(0) };
    /// 窗口内进入 I/O 边界的次数（**必须为 0**：连诊断事件都不许产生）。
    static IO_REQUESTS: Cell<u64> = const { Cell::new(0) };
    /// 窗口内**真的转交给了已安装 sink** 的次数（**必须为 0**）。
    static IO_OPS: Cell<u64> = const { Cell::new(0) };
    /// 窗口内走 [`note_suppressed`]（**只记数**的实时诊断出口）的次数。
    ///
    /// 它不是违规：这条出口**物理上到不了 sink**。它的作用是**见证** ——
    /// 判据可以同时断言"溢出路径真的被走到"（本计数 > 0）与"窗口里没有任何 I/O"
    /// （`io_requests == 0 && io_ops == 0`）⇒ 把"零 I/O"从"没跑到"升级为"跑到了也没有"。
    static SUPPRESSED: Cell<u64> = const { Cell::new(0) };
    /// 本线程的探针身份（0 = 尚未分配；首次需要时从全局计数器取一个）。
    static TOKEN: Cell<u64> = const { Cell::new(0) };
}

// ---------------------------------------------------------------------------
// 进程总量 / 外线程桶 / 线程身份
// ---------------------------------------------------------------------------

/// 进程内累计的量子探针进入次数（所有线程）。
static QUANTA_VISITS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 进程内累计的阻塞式加锁尝试（所有线程）。
static LOCK_BLOCKING_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 进程内累计的锁等待（所有线程）。
static LOCK_WAITS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 进程内累计的 I/O 边界进入次数（所有线程）。
static IO_REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 进程内累计的"真的转交给 sink"的次数（所有线程）。
static IO_OPS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 非声明 RT 线程上的阻塞式加锁尝试（**防串账**的对照量）。
static FOREIGN_LOCK_BLOCKING: AtomicU64 = AtomicU64::new(0);
/// 非声明 RT 线程上的锁等待。
static FOREIGN_LOCK_WAITS: AtomicU64 = AtomicU64::new(0);
/// 非声明 RT 线程上的 I/O 边界进入次数。
static FOREIGN_IO_REQUESTS: AtomicU64 = AtomicU64::new(0);
/// 非声明 RT 线程上的"真的转交给 sink"的次数。
static FOREIGN_IO_OPS: AtomicU64 = AtomicU64::new(0);
/// 进程内累计的**被抑制诊断**（[`note_suppressed`]；所有线程）。
static SUPPRESSED_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 被抑制诊断按**事件种类**的累计分布（下标 = [`RtDiagEvent::index`]）。
///
/// 为什么按种类留一份：`NoSnapshot` 除了这里**没有别的家**（它不像那两条溢出路径
/// 那样有 `EngineStats` 的领域计数器）⇒ 不按种类留就等于"悄悄丢掉一个事实"。
static SUPPRESSED_BY_KIND: [AtomicU64; RtDiagEvent::COUNT] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
/// 非声明 RT 线程上的被抑制诊断（防串账的对照量）。
static FOREIGN_SUPPRESSED: AtomicU64 = AtomicU64::new(0);
/// 线程身份发号器（`ThreadId` 的数值形态在 stable 上不可用，因此自己发号）。
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(0);
/// 被判据声明为"实时线程"的那个身份（0 = 未声明）。
static RT_TOKEN: AtomicU64 = AtomicU64::new(0);
/// 已安装的诊断出口（`OnceLock` 的读取是**原子 load**，实时路径上不阻塞、不分配）。
static SINK: OnceLock<Arc<dyn RtDiagSink>> = OnceLock::new();
/// 实时路径上**唯一**被允许出现的锁对象（生产代码里无人加锁；判据在此处见证）。
static RT_PATH_LOCK: RtLockProbe = RtLockProbe::new();

/// 自增一个线程局部计数（thread teardown 期间静默跳过，**绝不**从探针里 panic）。
fn bump(cell: &'static LocalKey<Cell<u64>>) {
    let _ = cell.try_with(|count| count.set(count.get().saturating_add(1)));
}

/// 读一个线程局部计数。
fn read(cell: &'static LocalKey<Cell<u64>>) -> u64 {
    cell.try_with(Cell::get).unwrap_or(0)
}

/// 本线程是否在观测窗口内。
fn armed_here() -> bool {
    // `try_with`（而不是 `with`）：线程 teardown 期间线程局部量不可用，
    // 而此时**绝不能**从探针里 panic 出去。
    ARMED.try_with(Cell::get).unwrap_or(false)
}

/// 本线程的探针身份（首次调用时分配一个稳定编号，不分配内存）。
///
/// 为什么不直接用 `std::thread::ThreadId`：它的数值形态（`as_u64`）在 stable 上不可用，
/// 而探针需要把身份存进原子量。自己发号即可满足"同一个线程每次拿到同一个值"这一唯一要求。
#[must_use]
pub fn current_thread_token() -> u64 {
    TOKEN
        .try_with(|token| {
            let current = token.get();
            if current != 0 {
                return current;
            }
            let assigned = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            token.set(assigned);
            assigned
        })
        .unwrap_or(0)
}

/// 把**当前线程**声明为实时线程（判据在开窗口之前、实时线程上调用一次）。
///
/// 声明之后，来自其它线程的锁/I/O 操作会被记进 `foreign_*` 桶
/// （见 [`RtProbeTotals`]），而**不会**出现在实时线程的窗口读数里。
///
/// ⚠ 本函数**同时温暖**实时路径锁（[`warm_up`]）：这不是洁癖，而是**实测的必要条件** ——
/// 在 macOS 上，**一个 `std::sync::Mutex` 的首次加锁会堆分配一次 64 字节**（本机实测，
/// 见 `docs/ledger/gate-rt-zero-alloc-notes.md` §6）。若不先在窗口之外碰一次，
/// 探针自己的见证 `try_lock` 就会在窗口内制造它要检测的那次分配
/// （第一次实测：10 000 量子窗口 `allocations=1`，来源正是探针自己）。
pub fn declare_rt_thread() {
    RT_TOKEN.store(current_thread_token(), Ordering::Relaxed);
    warm_up();
}

/// 在**窗口之外**先把探针锁"开一次"，把首次加锁的一次性开销挤出测量窗口。
///
/// 见 [`declare_rt_thread`] 的说明与 notes §6 的实测读数。调用它**不需要**任何特权，
/// 也不会往任何窗口里计数（未武装时不累加线程局部计数）。
pub fn warm_up() {
    let _ = RT_PATH_LOCK.try_lock_once();
}

/// 被声明为实时线程的那个身份（0 = 尚未声明）。
#[must_use]
pub fn rt_thread_token() -> u64 {
    RT_TOKEN.load(Ordering::Relaxed)
}

/// 当前线程相对于已声明 RT 线程是否是"外线程"（未声明时恒为 `false`）。
fn foreign_here(token: u64) -> bool {
    let rt = RT_TOKEN.load(Ordering::Relaxed);
    rt != 0 && token != rt
}

// ---------------------------------------------------------------------------
// 窗口开关与读数
// ---------------------------------------------------------------------------

/// 开始观测**本线程**上的量子探针 / 锁探针 / I/O 边界（在窗口之外调用）。
pub fn watch_current_thread() {
    let _ = ARMED.try_with(|armed| armed.set(true));
}

/// 结束观测**本线程**（计数保留，供 [`window_current_thread`] 读回）。
pub fn unwatch_current_thread() {
    let _ = ARMED.try_with(|armed| armed.set(false));
}

/// 清空**本线程**的窗口计数（不改动开关）。
pub fn reset_current_thread() {
    for cell in [
        &QUANTA_VISITS,
        &LOCK_TRY_SUCCESSES,
        &LOCK_TRY_FAILURES,
        &LOCK_BLOCKING,
        &LOCK_WAITS,
        &IO_REQUESTS,
        &IO_OPS,
        &SUPPRESSED,
    ] {
        let _ = cell.try_with(|count| count.set(0));
    }
}

/// 本线程最近一个观测窗口的读数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RtProbeWindow {
    /// [`quantum_enter`] 被调用的次数（= 本窗口内处理的量子数）。
    pub quanta_visits: u64,
    /// [`RtLockProbe::try_lock_once`] 成功取得锁的次数（见证：探针真的能锁）。
    pub lock_try_successes: u64,
    /// [`RtLockProbe::try_lock_once`] 因已被持有而失败的次数（`try` 不等待，不算违规）。
    pub lock_try_failures: u64,
    /// **阻塞式**加锁尝试次数（`MUST-GATE-001` 要求 0）。
    pub lock_blocking_attempts: u64,
    /// 阻塞式加锁**真的等过**的次数（`MUST-GATE-001` 要求 0）。
    pub lock_waits: u64,
    /// 进入 I/O 边界的次数（`MUST-GATE-001` 要求 0）。
    pub io_requests: u64,
    /// **真的转交给已安装 sink**（即真的发生 I/O）的次数（`MUST-GATE-001` 要求 0）。
    pub io_ops: u64,
    /// 走 [`note_suppressed`]（**纯计数**的实时诊断出口）的次数。
    ///
    /// 普通场景**要求 0**（没有溢出 ⇒ 那条出口根本不该被走到，见 `tests/rt_zero_alloc.rs`）；
    /// 溢出场景要求 **> 0** 作为"真的走到了、而且没有 I/O"的见证。
    pub suppressed_diag_events: u64,
}

/// 读回本线程的窗口读数。
#[must_use]
pub fn window_current_thread() -> RtProbeWindow {
    RtProbeWindow {
        quanta_visits: read(&QUANTA_VISITS),
        lock_try_successes: read(&LOCK_TRY_SUCCESSES),
        lock_try_failures: read(&LOCK_TRY_FAILURES),
        lock_blocking_attempts: read(&LOCK_BLOCKING),
        lock_waits: read(&LOCK_WAITS),
        io_requests: read(&IO_REQUESTS),
        io_ops: read(&IO_OPS),
        suppressed_diag_events: read(&SUPPRESSED),
    }
}

/// 进程内累计读数（跨线程；用**差值**做单窗口对账）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RtProbeTotals {
    /// 所有线程累计的 [`quantum_enter`] 次数。
    pub quanta_visits: u64,
    /// 所有线程累计的阻塞式加锁尝试次数。
    pub lock_blocking_attempts: u64,
    /// 所有线程累计的锁等待次数。
    pub lock_waits: u64,
    /// 所有线程累计的 I/O 边界进入次数。
    pub io_requests: u64,
    /// 所有线程累计的"真的转交给 sink"的次数。
    pub io_ops: u64,
    /// **非声明 RT 线程**上的阻塞式加锁尝试次数（防串账的对照量）。
    pub foreign_lock_blocking_attempts: u64,
    /// 非声明 RT 线程上的锁等待次数。
    pub foreign_lock_waits: u64,
    /// 非声明 RT 线程上的 I/O 边界进入次数。
    pub foreign_io_requests: u64,
    /// 非声明 RT 线程上的"真的转交给 sink"的次数。
    pub foreign_io_ops: u64,
    /// 所有线程累计的**被抑制诊断**（[`note_suppressed`]；这些调用**没有**产生 I/O）。
    pub suppressed_diag_events: u64,
    /// 被抑制诊断按事件种类的累计分布（下标 = [`RtDiagEvent::index`]）。
    ///
    /// 它是"溢出路径丢掉的细节"里**唯一**被刻意留下的那一半：没有文件行、没有时间戳，
    /// 但"哪一类事件被抑制了多少次"不丢。
    pub suppressed_by_kind: [u64; RtDiagEvent::COUNT],
    /// 非声明 RT 线程上的被抑制诊断次数。
    pub foreign_suppressed_diag_events: u64,
}

/// 读回进程总量。
#[must_use]
pub fn totals() -> RtProbeTotals {
    RtProbeTotals {
        quanta_visits: QUANTA_VISITS_TOTAL.load(Ordering::Relaxed),
        lock_blocking_attempts: LOCK_BLOCKING_TOTAL.load(Ordering::Relaxed),
        lock_waits: LOCK_WAITS_TOTAL.load(Ordering::Relaxed),
        io_requests: IO_REQUESTS_TOTAL.load(Ordering::Relaxed),
        io_ops: IO_OPS_TOTAL.load(Ordering::Relaxed),
        foreign_lock_blocking_attempts: FOREIGN_LOCK_BLOCKING.load(Ordering::Relaxed),
        foreign_lock_waits: FOREIGN_LOCK_WAITS.load(Ordering::Relaxed),
        foreign_io_requests: FOREIGN_IO_REQUESTS.load(Ordering::Relaxed),
        foreign_io_ops: FOREIGN_IO_OPS.load(Ordering::Relaxed),
        suppressed_diag_events: SUPPRESSED_TOTAL.load(Ordering::Relaxed),
        suppressed_by_kind: [
            SUPPRESSED_BY_KIND[0].load(Ordering::Relaxed),
            SUPPRESSED_BY_KIND[1].load(Ordering::Relaxed),
            SUPPRESSED_BY_KIND[2].load(Ordering::Relaxed),
        ],
        foreign_suppressed_diag_events: FOREIGN_SUPPRESSED.load(Ordering::Relaxed),
    }
}

// ---------------------------------------------------------------------------
// 实时路径上的两个探针位置
// ---------------------------------------------------------------------------

/// 实时路径**每个量子**调用一次（[`crate::rt::EngineRuntime`] 的 `render_block` 开头）。
///
/// 未武装时只做一次线程局部读取。武装时：
///
/// 1. 记一次"探针位置被执行到"（`quanta_visits`）；
/// 2. 在**同一条热路径**上对 [`rt_path_lock`] 做一次 `try_lock`（**不等待**），
///    成功的次数记进 `lock_try_successes` —— 这是"探针的牙"：
///    它证明这条路径**真的**能操作那把锁，因此"阻塞加锁 0 次"是**测出来的 0**，
///    而不是"这段代码根本没跑"。
pub fn quantum_enter() {
    if !armed_here() {
        return;
    }
    bump(&QUANTA_VISITS);
    QUANTA_VISITS_TOTAL.fetch_add(1, Ordering::Relaxed);
    let _ = RT_PATH_LOCK.try_lock_once();
}

/// 实时路径上唯一被允许出现的锁对象。
///
/// 生产代码**不**对它加锁（它的存在是为了让"实时路径上出现锁"这件事可被计数）：
/// 判据注入 `rt_probe::rt_path_lock().lock()` 就能把锁判据打红（见 notes 的注入记录 I1）。
#[must_use]
pub fn rt_path_lock() -> &'static RtLockProbe {
    &RT_PATH_LOCK
}

/// 实时路径的诊断出口（**唯一**的 I/O 边界）。
///
/// 语义分三层，都是可数的：
///
/// | 层 | 计数 | 含义 |
/// | :--- | :--- | :--- |
/// | 进入边界 | `io_requests` | "有人想在实时路径上产生一条诊断" —— 判据要求 **0** |
/// | 转发给 sink | `io_ops` | "真的发生了 I/O"（sink 由外部安装） —— 判据要求 **0** |
/// | 外线程对照 | `foreign_io_*` | 同样的操作发生在非 RT 线程上（防串账） |
///
/// 未安装 sink 时是**纯计数**（发布构建的常态：没有 I/O 出口）。安装 sink 之后
/// 每次调用都会真的走一遍 sink 的 I/O —— 判据用它证明"这个边界真的有牙"。
///
/// [`RtDiagEvent`] 是 `Copy` 且无负载：实时路径上不构造字符串、不分配。
///
/// ⚠ **实时回调的调用树里不许调用本函数**（`N6` 裁决 = 选项 A，见模块文档）：
/// 那里一律走 [`note_suppressed`]。本函数留给非实时路径与判据的正对照。
pub fn diag(event: RtDiagEvent) {
    let token = current_thread_token();
    let armed = armed_here();
    let foreign = foreign_here(token);

    IO_REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if foreign {
        FOREIGN_IO_REQUESTS.fetch_add(1, Ordering::Relaxed);
    }
    if armed {
        bump(&IO_REQUESTS);
    }

    if let Some(sink) = SINK.get() {
        IO_OPS_TOTAL.fetch_add(1, Ordering::Relaxed);
        if foreign {
            FOREIGN_IO_OPS.fetch_add(1, Ordering::Relaxed);
        }
        if armed {
            bump(&IO_OPS);
        }
        sink.emit(event);
    }
}

/// **只记数、绝不产生 I/O** 的实时诊断出口（`N6` 裁决 = 选项 A）。
///
/// 与 [`diag`] 的唯一区别：本函数**从不读** [`SINK`] ⇒ 实时回调到 `sink.emit`
/// **不存在**调用路径。成本是三次 `Relaxed` 原子加（总量 + 种类分布 + 可能的
/// 外线程桶）与至多一次线程局部 `Cell` 写；**不分配、不加锁、不等待、不做系统调用**。
///
/// # 计数在哪读（非 RT 读者）
///
/// | 读数 | 位置 | 覆盖到哪一层 |
/// | :--- | :--- | :--- |
/// | 本窗口走了几次纯计数出口 | [`RtProbeWindow::suppressed_diag_events`] | 判据窗口（线程局部） |
/// | 进程总量 + 按种类分布 + 外线程桶 | [`totals`]（[`RtProbeTotals`]） | 任何线程都能查 |
/// | 便利读取单个种类 | [`suppressed_by_kind`] | 同上 |
/// | 那两条溢出路径的**领域**计数 | [`crate::rt::EngineStats::meter_capacity_drops`] / [`crate::rt::EngineStats::snapshot_stash_events`] | 控制面 60Hz 循环（`EngineRuntime::stats()`） |
///
/// # 什么时候用哪一个
///
/// - **实时回调的调用树**（`EngineRuntime::render_block`、
///   `SnapshotReader::retire_or_stash`）：只能用本函数；
/// - **非实时路径**（控制面自己的诊断、判据的正对照）：用 [`diag`]。
///
/// 两者**都**是"可数的"：本函数记的是"实时路径上有一条想产生 I/O 的诊断，
/// 但按裁决**只记数**"；[`diag`] 记的是"真的走了 I/O 边界"。
pub fn note_suppressed(event: RtDiagEvent) {
    let token = current_thread_token();
    let armed = armed_here();
    let foreign = foreign_here(token);

    SUPPRESSED_TOTAL.fetch_add(1, Ordering::Relaxed);
    SUPPRESSED_BY_KIND[event.index()].fetch_add(1, Ordering::Relaxed);
    if foreign {
        FOREIGN_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
    }
    if armed {
        bump(&SUPPRESSED);
    }
}

/// 某一类被抑制诊断的**进程累计**次数（非 RT 读者用；`totals()` 的按种类视图）。
#[must_use]
pub fn suppressed_by_kind(event: RtDiagEvent) -> u64 {
    SUPPRESSED_BY_KIND[event.index()].load(Ordering::Relaxed)
}

/// 实时路径可能产生的诊断事件（无负载、`Copy`，因此热路径上零分配）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtDiagEvent {
    /// 退役队列满：读者把旧快照暂时寄存（`SnapshotReader::retire_or_stash`）。
    SnapshotRetireStash,
    /// 电平计量容量不足：某节点本量子没有被计量（`EngineRuntime::render_block`）。
    MeterCapacityDrop,
    /// 音频线程拿到了 `None` 快照（写者尚未发布任何快照）。
    NoSnapshot,
}

impl RtDiagEvent {
    /// 事件种类数（[`Self::index`] 的取值范围是 `0..COUNT`）。
    pub const COUNT: usize = 3;

    /// 稳定下标：只用于把被抑制诊断分进 [`SUPPRESSED_BY_KIND`] 的桶。
    ///
    /// 改动这些数字就等于改动"按种类的累计读数"的口径，因此它是**契约**而非实现细节。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::SnapshotRetireStash => 0,
            Self::MeterCapacityDrop => 1,
            Self::NoSnapshot => 2,
        }
    }

    /// 供 sink 打的固定文本（**不含**任何格式化参数 ⇒ 实时路径上不分配）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SnapshotRetireStash => "snapshot-retire-stash",
            Self::MeterCapacityDrop => "meter-capacity-drop",
            Self::NoSnapshot => "no-snapshot",
        }
    }
}

/// 诊断出口的后端（**测试专用计数后端**就在这里接）。
///
/// 契约：`emit` 会被**任意线程**调用（包括实时线程），因此实现必须自己做判断 ——
/// 本 trait 不提供任何"这是不是实时线程"的信息（探针不做策略，只做计数）。
pub trait RtDiagSink: Send + Sync {
    /// 处理一条诊断事件。
    fn emit(&self, event: RtDiagEvent);
}

/// 安装诊断出口（**整个进程只能装一次**，通常在打开设备之前）。
///
/// # Errors
///
/// 已经装过时把 `sink` 原样退回（`OnceLock` 的语义），调用方据此报错而不是静默吞掉。
pub fn install_diag_sink(sink: Arc<dyn RtDiagSink>) -> Result<(), Arc<dyn RtDiagSink>> {
    SINK.set(sink)
}

// ---------------------------------------------------------------------------
// 见证型锁
// ---------------------------------------------------------------------------

/// 计数 / 见证型锁：包住一个真的 `std::sync::Mutex<()>`，并把三种加锁行为分开计数。
///
/// | 方法 | 语义 | 计数 |
/// | :--- | :--- | :--- |
/// | [`Self::lock`] | **阻塞式**加锁（红线 7 禁止实时路径使用） | 尝试 +1；若已被持有则等待 +1 |
/// | [`Self::try_lock_once`] | **非阻塞**试探（实时路径上的见证） | 成功/失败分开计数 |
///
/// 它**不是**"测试替身"：内部就是标准库的 `Mutex`，行为（含争用时真的阻塞）与产品里
/// 任何一把锁一致 —— 因此"在它上面观察到等待"等价于"在这把锁上真的等待过"。
pub struct RtLockProbe {
    inner: Mutex<()>,
}

impl RtLockProbe {
    /// 新建一把探针锁（`const`，可放进 `static`）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(()),
        }
    }

    /// **阻塞式**加锁：计入"阻塞尝试"；一旦发现已被别的线程持有，再计一次"等待"，然后真的等。
    ///
    /// 中毒时取回内部数据（实时/关流路径上宁可继续也不 panic，红线 7 的精神）。
    pub fn lock(&self) -> MutexGuard<'_, ()> {
        let token = current_thread_token();
        let armed = armed_here();
        let foreign = foreign_here(token);

        LOCK_BLOCKING_TOTAL.fetch_add(1, Ordering::Relaxed);
        if foreign {
            FOREIGN_LOCK_BLOCKING.fetch_add(1, Ordering::Relaxed);
        }
        if armed {
            bump(&LOCK_BLOCKING);
        }

        match self.inner.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => {
                LOCK_WAITS_TOTAL.fetch_add(1, Ordering::Relaxed);
                if foreign {
                    FOREIGN_LOCK_WAITS.fetch_add(1, Ordering::Relaxed);
                }
                if armed {
                    bump(&LOCK_WAITS);
                }
                self.inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            }
        }
    }

    /// **非阻塞**试探：立即返回是否拿到锁（不等待 ⇒ 实时路径上允许）。
    ///
    /// 返回 `true` 表示**真的**取到并立刻释放了锁 —— 这正是
    /// [`quantum_enter`] 每量子做一次的那件事，也是"探针有牙"的证据。
    ///
    /// ⚠ 对一个**刚刚新建**的探针，第一次调用可能触发一次性的 64 字节分配
    /// （macOS 上每个 `Mutex` 实例的首次加锁都会，见模块文档）⇒ 判据必须先在
    /// **窗口之外**调用本方法一次再开窗口。
    pub fn try_lock_once(&self) -> bool {
        match self.inner.try_lock() {
            Ok(guard) => {
                drop(guard);
                if armed_here() {
                    bump(&LOCK_TRY_SUCCESSES);
                }
                true
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                drop(poisoned.into_inner());
                if armed_here() {
                    bump(&LOCK_TRY_SUCCESSES);
                }
                true
            }
            Err(TryLockError::WouldBlock) => {
                if armed_here() {
                    bump(&LOCK_TRY_FAILURES);
                }
                false
            }
        }
    }
}

impl Default for RtLockProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探针自检：武装之后，非阻塞试探**成功**会被计进窗口（"牙"的存在性）。
    /// 用**差值**断言（libtest 会并行跑别的测试，窗口计数也可能被本模块自己的别的测试碰到）。
    #[test]
    fn try_lock_witness_is_counted_inside_the_window() {
        reset_current_thread();
        watch_current_thread();
        let probe = RtLockProbe::new();
        assert!(probe.try_lock_once(), "无人争用时应立即拿到");
        assert!(probe.try_lock_once());
        let window = window_current_thread();
        unwatch_current_thread();
        assert!(window.lock_try_successes >= 2, "窗口必须看得见试探成功");
        assert_eq!(window.lock_try_failures, 0, "无人争用不会有试探失败");
    }

    /// 探针自检：阻塞式加锁被计成"尝试 + 未等待"（无人争用的情形）。
    #[test]
    fn blocking_lock_without_contention_counts_an_attempt_but_no_wait() {
        reset_current_thread();
        watch_current_thread();
        let probe = RtLockProbe::new();
        drop(probe.lock());
        let window = window_current_thread();
        unwatch_current_thread();
        assert_eq!(window.lock_blocking_attempts, 1);
        assert_eq!(window.lock_waits, 0, "无人争用 ⇒ 不该记等待");
    }

    /// 探针自检：**未武装**的线程不累加窗口，但**总量**照样动（窗口是线程局部的）。
    #[test]
    fn window_is_thread_local_while_totals_are_process_wide() {
        let before = totals();
        reset_current_thread();
        unwatch_current_thread();
        diag(RtDiagEvent::MeterCapacityDrop);
        let window = window_current_thread();
        let after = totals();
        assert_eq!(window.io_requests, 0, "未武装的线程不该往窗口里计数");
        assert!(
            after.io_requests > before.io_requests,
            "总量必须无条件累加（否则'别人的操作'无从对照）"
        );
    }

    /// 诊断事件的名字是固定文本（实时路径上不格式化、不分配）。
    #[test]
    fn diag_event_names_are_static() {
        assert_eq!(
            RtDiagEvent::SnapshotRetireStash.as_str(),
            "snapshot-retire-stash"
        );
        assert_eq!(
            RtDiagEvent::MeterCapacityDrop.as_str(),
            "meter-capacity-drop"
        );
        assert_eq!(RtDiagEvent::NoSnapshot.as_str(), "no-snapshot");
    }

    /// 按种类的下标是**稳定的双射**（错一个就会让两类的读数串账）。
    #[test]
    fn diag_event_indices_are_distinct_and_in_range() {
        let indices = [
            RtDiagEvent::SnapshotRetireStash.index(),
            RtDiagEvent::MeterCapacityDrop.index(),
            RtDiagEvent::NoSnapshot.index(),
        ];
        assert_eq!(RtDiagEvent::COUNT, indices.len());
        for (position, index) in indices.iter().enumerate() {
            assert!(*index < RtDiagEvent::COUNT, "下标必须落在桶内");
            assert_eq!(
                indices.iter().position(|other| other == index),
                Some(position),
                "下标必须互不相同"
            );
        }
    }

    /// `N6` 选项 A 的核心不变式：`note_suppressed` **只记数** ——
    /// 窗口里看得见它（>`0`），同时 `io_requests`/`io_ops` 仍为 `0`；
    /// 种类分布与外线程桶照样对账。
    #[test]
    fn note_suppressed_counts_without_crossing_the_io_boundary() {
        let totals_before = totals();
        reset_current_thread();
        watch_current_thread();
        note_suppressed(RtDiagEvent::MeterCapacityDrop);
        note_suppressed(RtDiagEvent::MeterCapacityDrop);
        note_suppressed(RtDiagEvent::NoSnapshot);
        let window = window_current_thread();
        unwatch_current_thread();
        let totals_after = totals();

        assert_eq!(window.suppressed_diag_events, 3, "窗口必须看得见纯计数出口");
        assert_eq!(
            window.io_requests, 0,
            "纯计数出口不得进入 I/O 边界（N6 选项 A）"
        );
        assert_eq!(window.io_ops, 0, "纯计数出口不得转发给 sink");
        assert!(
            totals_after.suppressed_diag_events > totals_before.suppressed_diag_events,
            "进程总量必须无条件累加"
        );
        assert!(
            totals_after.suppressed_by_kind[RtDiagEvent::MeterCapacityDrop.index()]
                >= totals_before.suppressed_by_kind[RtDiagEvent::MeterCapacityDrop.index()] + 2,
            "按种类分布必须记下那两次电平容量抑制"
        );
        assert!(
            suppressed_by_kind(RtDiagEvent::NoSnapshot)
                > totals_before.suppressed_by_kind[RtDiagEvent::NoSnapshot.index()],
            "便利读取器与 totals() 的按种类视图必须同源"
        );
    }
}
