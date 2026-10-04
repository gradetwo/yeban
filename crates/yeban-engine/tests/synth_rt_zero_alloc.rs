//! `line/engine-sound` 的运行期零分配判据 J5：
//! **合成路径接入之后，实时回调窗口仍然零分配、零释放** [MUST-GATE-001, ARCH-RT-001]。
//!
//! # 为什么必须单开一个 `harness = false` 目标
//!
//! `rt_zero_alloc.rs`（既有的 `[MUST-GATE-001]` 目标）用的是
//! `yeban_model::samples::filled_project()`，它的 4 个音符只覆盖前 1.5 秒 ——
//! 10,000 个量子的窗口里**绝大多数时间没有声部在跑**，于是"零分配"可能只是
//! "什么都没做"。本目标用**音符铺满整个窗口**的夹具（256 个交叠音符）复测，
//! 并**同时**断言窗口内确实有非零样本（覆盖度自检，防假绿）。
//!
//! 计数型全局分配器是**进程全局**的，libtest 自己的线程会污染计数
//! （第一版实测 `allocations=9`），因此这里与既有目标一样关掉 libtest
//! （`[[test]] harness = false`），只让主线程跑测量。

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};

mod support;

use support::{NoteSpec, note_project};

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法只做"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有跨调用状态（计数器是原子量）。`unsafe` 块的边界就是 `System` 的调用本身。
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

fn measure<F: FnOnce()>(label: &str, body: F) -> (usize, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    body();
    ARMED.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let deallocations = DEALLOCATIONS.load(Ordering::SeqCst);
    println!("[engine-sound/J5] {label}: allocations={allocations} deallocations={deallocations}");
    (allocations, deallocations)
}

/// 256 个交叠音符的夹具：`i` 从 0 起每 240 tick 起音、时值 480 tick。
///
/// 覆盖窗口 = `256 × 240 + 480 = 61920` tick = 1,548,000 样本，
/// 比判据的 10,000 个量子（1,280,000 样本）更长 ⇒ **整个测量窗口里都有声部在跑**。
fn saturated_notes() -> Vec<NoteSpec> {
    (0..256u64)
        .map(|index| NoteSpec::at(index * 240, 480, 60 + (index % 12) as u8, 100))
        .collect()
}

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    // ---- 装配（**允许分配**：全部发生在打开设备之前）----
    let fixture = note_project(&saturated_notes());
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, mut queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(8192);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut output = vec![0.0f32; 128 * 2];

    // 预热：包络模板武装、波表读取的首个 mip 级选择等一次性路径。
    // 不计入实时窗口（否则会把"初始化"记到实时路径头上 —— 那是假红）。
    runtime.process_quantum(&mut output, 2);

    // ---- 场景 1：音符铺满窗口，连续 10,000 个量子必须零分配、零释放 ----
    let mut nonzero = 0usize;
    let mut peak = 0.0f32;
    let (allocations, deallocations) = measure("10_000 quanta (saturated notes)", || {
        for _ in 0..10_000 {
            runtime.process_quantum(&mut output, 2);
            for sample in &output {
                if *sample != 0.0 {
                    nonzero += 1;
                }
                peak = peak.max(sample.abs());
            }
        }
    });
    if allocations != 0 {
        failures.push(format!(
            "合成路径在实时窗口内分配了 {allocations} 次堆内存 —— [MUST-GATE-001] 一票否决"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "合成路径在实时窗口内释放了 {deallocations} 次 —— [MUST-GATE-001] 一票否决"
        ));
    }
    // 覆盖度自检：窗口里必须**真的**在出声，否则"零分配"是空转。
    if nonzero == 0 {
        failures
            .push("10,000 个量子的窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let stats = runtime.stats();
    if stats.notes_triggered < 200 {
        failures.push(format!(
            "只触发了 {} 个音符，未达到压测规模（应 ≥ 200）",
            stats.notes_triggered
        ));
    }
    if stats.voice_steals != 0 {
        failures.push(format!(
            "夹具不应触及复音上限，但发生了 {} 次声部窃取",
            stats.voice_steals
        ));
    }

    // ---- 场景 2：高频快照交换（每次交换都要重新对齐声部池）----
    let mut switches = 0u64;
    for revision in 2..=64u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&fixture.project, revision).expect("快照");
        slot.publish(next);
        let (allocations, deallocations) = measure("snapshot swap + quantum", || {
            runtime.process_quantum(&mut output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "快照交换期间的实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "实时线程释放了 {deallocations} 次（revision={revision}）—— 旧快照必须经退役队列交回主线程"
            ));
        }
        switches += queue.drain(64) as u64;
    }
    if switches == 0 {
        failures.push("主线程侧没有从退役队列回收任何旧快照 —— 场景 2 是空转".to_owned());
    }

    // ---- 场景 3：回到 `filled_project`（真实密度的规范样本）复测 ----
    let filled = yeban_model::samples::filled_project();
    let filled_snapshot = EngineSnapshot::from_project(&filled, 1).expect("规范样本快照");
    let filled_slot = SnapshotSlot::new(filled_snapshot);
    let (filled_retire, _filled_queue) = retire_channel(8);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut filled_runtime = EngineRuntime::new(&filled_slot, filled_retire, receiver, publisher);
    let mut filled_output = vec![0.0f32; 128 * 2];
    filled_runtime.process_quantum(&mut filled_output, 2);
    let mut filled_nonzero = 0usize;
    let (allocations, deallocations) = measure("filled_project 4_000 quanta", || {
        for _ in 0..4_000 {
            filled_runtime.process_quantum(&mut filled_output, 2);
            for sample in &filled_output {
                if *sample != 0.0 {
                    filled_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "filled_project 窗口不是零分配零释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if filled_nonzero == 0 {
        failures
            .push("filled_project 的 4,000 个量子没有任何非零样本 —— 覆盖度自检失败".to_owned());
    }
    let filled_stats = filled_runtime.stats();
    if filled_stats.scheduled_notes == 0 {
        failures.push("filled_project 的快照没有调度任何音符".to_owned());
    }

    println!(
        "[engine-sound/J5] 汇总: quanta={} scheduled_notes={} notes_triggered={} voice_steals={} \
         非零样本={nonzero} 峰值={peak:.6} filled(nonzero={filled_nonzero}, scheduled={}, triggered={})",
        stats.quanta,
        stats.scheduled_notes,
        stats.notes_triggered,
        stats.voice_steals,
        filled_stats.scheduled_notes,
        filled_stats.notes_triggered,
    );

    if failures.is_empty() {
        println!(
            "[engine-sound/J5] ok: 10,000 量子（音符铺满窗口）+ 63 次快照交换 + \
             filled_project 4,000 量子，实时窗口内零分配零释放"
        );
        ExitCode::SUCCESS
    } else {
        for failure in &failures {
            eprintln!("[engine-sound/J5] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}
