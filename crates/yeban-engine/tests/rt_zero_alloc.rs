//! `[MUST-GATE-001]` 的**运行期**断言：实时回调路径零分配、零释放。
//!
//! 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`）：
//!
//! > **音频线程零安全违规 (Zero Glitch / Zero Alloc)**：实时音频回调函数内严禁出现任何堆内存分配
//! > （`malloc`）、堆释放（`free`/`drop`）、文件/网络 I/O 或互斥锁争用，
//! > **CI 运行时通过内存分配 Hook 进行严格断言**。
//!
//! 在此之前本仓库只有**结构性**判据（`AudioBlock<128>` 内联、每块恰好一次批量 SPSC 调用、
//! 退役队列把 `drop` 推到主线程）。结构性判据证明的是"**我们没写**分配"，不是"**运行期确实没有**分配"。
//! 本文件补上后者：安装一个**计数型全局分配器**，只在被测窗口内计数。
//!
//! # 为什么这个目标是 `harness = false`（没有 libtest）
//!
//! 计数器是**进程全局**的，而 `#[global_allocator]` 每个二进制只能定义一次 ——
//! 所以任何**别的线程**在窗口内的分配都会被算进来。第一版用 `#[test]` 写，CI 实测报
//! `allocations=9 deallocations=3`：那不是实时路径在分配，而是 **libtest 自己在起线程/收结果**。
//! 那种假红比没有判据更坏（会训练人忽略它）。
//!
//! 因此本目标关掉 libtest（`[[test]] harness = false`，见 `Cargo.toml`）：
//! 进程里只有主线程跑测量，数字**无歧义**。代价是它不再以 `#[test]` 形式出现在 libtest 汇总里 ——
//! 但 `cargo test --all-targets` **仍会构建并运行**它，且以**退出码**判定。
//!
//! # 这条判据怎么变红（可由注入验证）
//!
//! 在 `process_quantum` 的调用树里加一句 `let _ = Vec::<u8>::with_capacity(1);` ⇒ 窗口内 `allocations != 0`；
//! 把退役队列的 `Arc` 就地 `drop` 而不是推给主线程 ⇒ `deallocations != 0`。

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

/// `true` 时才开始计数 —— 只在被测窗口内打开，避免把进程启动/输出自己的分配算进去。
static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法都只是"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用状态（计数器是原子量）。`unsafe` 块的边界就是 `System` 的调用本身。
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
            // 重新分配在实时路径上同样禁止（可能搬迁并复制），因此也计入分配。
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// 在"必须零分配"的窗口内执行 `body`，返回 `(allocations, deallocations)`。
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

/// 全部通道都在"打开设备之前"建立（回调内不允许分配），这也是产品路径的契约。
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

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    let (slot, mut queue, mut runtime) = rig();
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
    if allocations != 0 {
        failures.push(format!(
            "实时回调窗口内发生了 {allocations} 次堆分配 —— [MUST-GATE-001] 一票否决"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "实时回调窗口内发生了 {deallocations} 次堆释放（drop/free）—— [MUST-GATE-001] 一票否决"
        ));
    }

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
        if allocations != 0 {
            failures.push(format!(
                "快照切换期间的实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "实时线程**释放**了旧快照 {deallocations} 次（revision={revision}）—— 旧快照必须经退役队列交回主线程"
            ));
        }
        // 主线程侧（允许 dealloc）：把读者确认过的旧快照真正释放掉。
        released_off_thread += queue.drain(64);
    }
    if released_off_thread == 0 {
        failures.push(
            "主线程侧没有从退役队列回收任何旧快照 —— 快照根本没被替换，场景 2 是空转".to_owned(),
        );
    }

    // ---- 场景 3：覆盖度自检（防止"窗口里其实什么都没跑"这种假绿）----
    let stats = runtime.stats();
    let quanta = stats.quanta;
    if quanta < 10_000 {
        failures.push(format!(
            "只处理了 {quanta} 个量子，未达到 10,000 的压测规模"
        ));
    }
    println!(
        "[MUST-GATE-001] 汇总: quanta={quanta} event_bulk_pops={} meter_frames={} 主线程回收={released_off_thread}",
        stats.event_bulk_pops, stats.meter_frames
    );

    if failures.is_empty() {
        println!("[MUST-GATE-001] ok: 10,000 量子 + 63 次快照交换，实时窗口内零分配零释放");
        ExitCode::SUCCESS
    } else {
        for failure in &failures {
            eprintln!("[MUST-GATE-001] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}
