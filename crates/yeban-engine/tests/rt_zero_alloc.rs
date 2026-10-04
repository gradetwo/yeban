//! `[MUST-GATE-001]` 的**运行期**断言：实时回调路径零分配、零释放。
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`）：
//!
//! > **音频线程零安全违规 (Zero Glitch / Zero Alloc)**：实时音频回调函数内严禁出现任何堆内存分配
//! > （`malloc`）、堆释放（`free`/`drop`）、文件/网络 I/O 或互斥锁争用，
//! > **CI 运行时通过内存分配 Hook 进行严格断言**。
//!
//! 在此之前本仓库只有**结构性**判据（`AudioBlock<128>` 内联、每块恰好一次批量 SPSC 调用、
//! 退役队列把 `drop` 推到主线程）。结构性判据证明的是"**我们没写**分配"，不是"**运行期确实没有**分配" ——
//! 而 `Vec` 增长、`format!`、`Arc::new`、甚至某个 std API 的内部惰性初始化都可能悄悄分配。
//! 本文件补上后者：安装一个**计数型全局分配器**，只在 `process_quantum` 的窗口内计数。
//!
//! # 为什么这个二进制里只能有一个真正执行的 `#[test]`
//!
//! `#[global_allocator]` 每个二进制只能定义一次，而计数器是**进程全局**的。
//! 若同一二进制里有别的测试**并行**运行，它们自己的分配会被算进窗口里 ⇒ 判据会假红。
//! 因此第二个场景（快照退役压测）写在**同一个** `#[test]` 里顺序执行；
//! 另有 `#[ignore]` 的占位测试说明这条约束（它被 ignore 掉，不参与并行）。
//!
//! # 这条判据能怎么变红（实测过）
//!
//! - 在 `process_quantum` 里加一句 `let _ = Vec::<u8>::with_capacity(1);` ⇒ `allocations != 0` 立刻红；
//! - 把退役队列的 `Arc` 就地 `drop`（而不是推给主线程）⇒ `deallocations != 0` 立刻红。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

/// `true` 时才开始计数 —— 只在被测窗口内打开，避免把测试框架自己的分配算进去。
static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法都只是"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用的状态（计数器是原子量）。`unsafe` 块的边界就是 `System` 的调用本身。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `layout` 合法（`GlobalAlloc` 的契约）。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr` 来自本分配器且 `layout` 与分配时一致。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            // 重新分配在实时路径上同样是禁止的（可能搬迁并复制），因此也计数。
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// 把"这一段必须零分配"写成可复用的窗口，顺便让失败信息带上具体数字。
fn measure<F: FnOnce()>(label: &str, body: F) -> (usize, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    body();
    ARMED.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let deallocations = DEALLOCATIONS.load(Ordering::SeqCst);
    println!("[MUST-GATE-001] {label}: allocations={allocations} deallocations={deallocations}");
    (allocations, deallocations)
}

/// 组装一条最小的实时链路（全部通道在"打开设备之前"建立，符合回调内不分配的契约）。
fn rig() -> (
    Arc<SnapshotSlot>,
    yeban_engine::snapshot::RetireQueue,
    EngineRuntime,
) {
    let project = yeban_model::samples::filled_project();
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(256);
    let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    (slot, queue, runtime)
}

#[test]
fn real_time_path_allocates_and_frees_nothing() {
    let (slot, mut queue, mut runtime) = rig();

    // 缓冲在窗口之外分配。
    let mut output = vec![0.0f32; 128 * 2];

    // 预热一次：让一次性的惰性路径（FPU 状态武装等）先跑完，
    // 否则第一次调用会把"初始化"记到实时路径头上 —— 那是假红。
    runtime.process_quantum(&mut output, 2);

    // ---- 场景 1：稳定状态下连续 10,000 个量子必须零分配、零释放 ----
    let (allocations, deallocations) = measure("10_000 quanta", || {
        for _ in 0..10_000 {
            runtime.process_quantum(&mut output, 2);
        }
    });
    assert_eq!(
        allocations, 0,
        "实时回调窗口内发生了 {allocations} 次堆分配 —— [MUST-GATE-001] 一票否决"
    );
    assert_eq!(
        deallocations, 0,
        "实时回调窗口内发生了 {deallocations} 次堆释放（`drop`/`free`）—— [MUST-GATE-001] 一票否决"
    );

    // ---- 场景 2：高频快照交换（[MUST-GATE-012] 的"高频交换"形态）----
    // 控制线程反复发布新快照；实时线程必须**只把旧快照推给退役队列**，绝不就地释放。
    let project = yeban_model::samples::filled_project();
    let mut released_off_thread = 0usize;
    for revision in 2..=64u64 {
        // 发布在窗口之外：它**允许**分配（控制线程不是实时线程）。
        let next = EngineSnapshot::from_project(&project, revision).expect("快照");
        slot.publish(next);
        let (allocations, deallocations) = measure("snapshot swap+quantum", || {
            runtime.process_quantum(&mut output, 2);
        });
        assert_eq!(
            allocations, 0,
            "快照切换期间的实时路径不得分配（revision={revision}）"
        );
        assert_eq!(
            deallocations, 0,
            "实时线程**释放**了旧快照（revision={revision}）—— 旧快照必须经退役队列交回主线程"
        );
        // 主线程侧（允许 dealloc）：把读者确认过的旧快照真正释放掉。
        released_off_thread += queue.drain(64);
    }
    assert!(
        released_off_thread > 0,
        "主线程侧应当从退役队列回收旧快照（否则快照根本没被替换，场景 2 是空转）"
    );

    // ---- 场景 3：整段"发布 + 处理 + 回收"的总账 ----
    // 证明上面两个窗口加起来覆盖了 10k+ 次量子与 63 次快照交换。
    let stats = runtime.stats();
    let quanta = stats.quanta;
    assert!(quanta >= 10_000, "应当至少处理过 10,000 个量子");
    println!(
        "[MUST-GATE-001] 汇总: quanta={quanta} event_bulk_pops={} meter_frames={} 主线程回收={released_off_thread}",
        stats.event_bulk_pops, stats.meter_frames
    );
}

/// 这条测试**故意**被忽略：它只是把"本文件为什么只能有一个真正执行的 `#[test]`"写成代码。
///
/// 计数器是进程全局的，`#[global_allocator]` 每个二进制只能有一个；
/// 若真有第二个测试并行跑，它自己的分配会被算进窗口 ⇒ 假红。
/// 需要第二个场景时，请**加进上面那条测试**里顺序执行，而不是新开一个 `#[test]`。
#[test]
#[ignore = "分配计数器是进程全局的; 本文件只能有一个真正执行的 #[test]"]
fn a_second_test_would_pollute_the_global_allocation_counter() {
    let _ = rig();
}
