//! `yeban_dsp::vocal_per_note_eq` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要这个文件
//!
//! `PerNoteEq::process` 的零分配是 P2-1 的验收门（PLAN-vocaledit-phase2.md
//! §3 第 6 项）。本模块**还没有被引擎调用**（新模块、无接线），引擎的
//! `rt_zero_alloc` 判据覆盖不到它 ⇒ 本文件把同一套仪器对准它本身。
//!
//! # 仪器口径（与 `compressor_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数
//!    （libtest 起线程/收结果不会污染读数）。
//! 2. **窗口只包住实时路径**：处理器、缓冲区、正弦相位都在窗口**外**准备。
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**，不是"没有计数器"。
//!
//! ⚠ 本文件**只**覆盖"堆分配/释放"两个分量。锁与阻塞 I/O 的分量由引擎的
//! `rt_probe` 承担，本 crate **没有**那套探针 ⇒ 本文件对锁与 I/O **不表态**
//! （不是"已证明为 0"）。`PerNoteEq::process` 的逐样本路径里没有锁、
//! 没有 I/O 调用，但那是**源码形状**的证据，不是运行期读数。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::vocal_per_note_eq::{NoteEqBand, PerNoteEq, PerNoteEqConfig};
use yeban_dsp::vocal_refine::RefinedNote;

// ---------------------------------------------------------------------------
// 计数型全局分配器（按线程武装）
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
            let _ = ALLOCATIONS.try_with(|cell| cell.set(cell.get() + 1));
        }
        // SAFETY: 契约由本类型的调用方持有；这里原样转发给系统分配器。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if alloc_armed_here() {
            let _ = DEALLOCATIONS.try_with(|cell| cell.set(cell.get() + 1));
        }
        // SAFETY: 同上，原样转发。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // `realloc` 在实时路径上同样禁止。这里**同时**记进两个分量：
        // 它既是一次分配也是一次释放。若判据要求"两个分量都为 0"，
        // 单记一个分量也能抓住它，但分开记能让失败信息指明是哪一种。
        if alloc_armed_here() {
            let _ = ALLOCATIONS.try_with(|cell| cell.set(cell.get() + 1));
            let _ = DEALLOCATIONS.try_with(|cell| cell.set(cell.get() + 1));
        }
        // SAFETY: 同上，原样转发。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// 一次观测的读数（单位：**次数**）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    allocations: u64,
    deallocations: u64,
}

impl Reading {
    fn total(&self) -> u64 {
        self.allocations + self.deallocations
    }
}

/// 在一个观测窗口里跑 `body`，返回窗口内的分配/释放次数。
fn window<F: FnOnce()>(body: F) -> Reading {
    ALLOCATIONS.with(|c| c.set(0));
    DEALLOCATIONS.with(|c| c.set(0));
    ARMED.with(|c| c.set(true));
    body();
    ARMED.with(|c| c.set(false));
    Reading {
        allocations: ALLOCATIONS.with(Cell::get),
        deallocations: DEALLOCATIONS.with(Cell::get),
    }
}

// ---------------------------------------------------------------------------
// 判据规模（改动这些常量就是改动判据强度）
// ---------------------------------------------------------------------------

/// 观测窗口的帧数（`128` 与本仓库的固定处理块长同值 [ARCH-DET-001]）。
const WINDOW_FRAMES: usize = 128;
/// 判据 1 的量子数（每量子 128 帧）。
const RT_QUANTA: u64 = 10_000;
/// 每多少个量子复位一次（覆盖 `reset`；复位后位置归零，EQ 区间重新生效）。
const RESET_EVERY: u64 = 2_000;

/// 量什么：`PerNoteEq::process` 的逐样本路径在 10 000 个量子里的**堆分配次数与释放次数**
/// （单位：次数），窗口内还夹着 `reset`。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（处理器、缓冲区）----
    // A 音 [0.5, 1.0) s、440 Hz、-12 dB；位置每 2000 量子复位一次 ⇒
    // EQ 区间在窗口内反复生效（覆盖度自检的 `differing` 靠它涨）。
    let notes = [RefinedNote {
        pre_s: 0.5,
        vop_s: 0.55,
        off_s: 1.0,
        anchored: true,
        confidence: 0.9,
        f0_median_hz: 440.0,
        hnr_db: 10.0,
    }];
    let bands = [NoteEqBand {
        note_index: 0,
        center_hz: 440.0,
        gain_db: -12.0,
        q: 1.0,
    }];
    let mut eq = PerNoteEq::new(48_000, &notes, &bands, PerNoteEqConfig::default());
    let mut input = vec![0.0f32; WINDOW_FRAMES];
    let mut output = vec![0.0f32; WINDOW_FRAMES];
    // 440 Hz 正弦，相位连续推进（重填不分配）。
    let mut phase: f32 = 0.0;
    let mut differing: u64 = 0;

    // ---- 观测窗口 ----
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            for s in input.iter_mut() {
                *s = 0.5 * phase.sin();
                phase += 2.0 * core::f32::consts::PI * 440.0 / 48_000.0;
                if phase > core::f32::consts::TAU {
                    phase -= core::f32::consts::TAU;
                }
            }
            let n = eq.process(&input, &mut output);
            for (o, i) in output.iter().zip(input.iter()).take(n) {
                if o.to_bits() != i.to_bits() {
                    differing += 1;
                }
            }
            if quantum % RESET_EVERY == 0 {
                eq.reset();
            }
        }
    });

    // 覆盖度自检：窗口里真的在处理（否则"全 0"可能是"什么都没跑"）。
    assert_eq!(
        eq.processed_samples(),
        RT_QUANTA * WINDOW_FRAMES as u64,
        "量子记账：每量子 {WINDOW_FRAMES} 帧"
    );
    assert!(
        differing > 0,
        "窗口里 EQ 从未生效（输出与输入处处相同）⇒ 判据测错了对象"
    );
    eprintln!(
        "[yeban-dsp/RT] vocal_per_note_eq 10 000 量子 × {WINDOW_FRAMES} 帧: \
         allocations={} deallocations={} differing={differing}",
        reading.allocations, reading.deallocations
    );
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

/// 量什么：正对照 —— 在**同一个**观测窗口里显式分配一次，读数必须是
/// `allocations == 1 && deallocations == 1`（单位：次数）。
///
/// 判据：两条都不为 0。若计数器坏了（永远读 0），这条会红 ⇒
/// 上一条判据的"全 0"才是有牙的 0。
#[test]
fn the_counter_has_teeth_a_deliberate_allocation_is_counted() {
    let reading = window(|| {
        // ⚠ `Vec::new()` **不分配**（容量 0）—— 用它当注入是无效注入。
        let v = Vec::<u8>::with_capacity(1);
        assert_eq!(v.capacity(), 1);
        // `v` 在这里 drop ⇒ 记 1 次释放。
    });
    eprintln!(
        "[yeban-dsp/RT] 正对照 `Vec::with_capacity(1)`: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert_eq!(reading.allocations, 1, "计数器没数到分配 ⇒ 仪器坏了");
    assert_eq!(reading.deallocations, 1, "计数器没数到释放 ⇒ 仪器坏了");
}

/// 量什么：`Vec::new()`（容量 0）在窗口里的读数（单位：次数）。
///
/// 判据：全 0。这条把"无效注入"登记下来 —— 引用它的人不会把
/// `Vec::new()` 当成一次真的分配。
#[test]
fn an_empty_vec_does_not_allocate() {
    let reading = window(|| {
        let v = Vec::<u8>::new();
        assert_eq!(v.capacity(), 0);
    });
    eprintln!(
        "[yeban-dsp/RT] `Vec::new()`（无效注入的口径）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert_eq!(reading.total(), 0, "`Vec::new()` 不该分配");
}
