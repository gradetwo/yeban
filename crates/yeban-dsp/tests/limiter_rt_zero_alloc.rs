//! `yeban_dsp::limiter` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要这个文件（与引擎那条判据的分工）
//!
//! `crates/yeban-engine/tests/rt_zero_alloc.rs` 的场景 ⑥ 覆盖的是**引擎的音频路径**
//! （`EngineRuntime::render_block` 里的 `BusLimiter::process_stereo` 调用点）。
//! 上移之后那条判据**仍然覆盖**本器件 —— 但它覆盖的是"引擎经由 re-export 调到的那条路"。
//!
//! 本文件把同一套仪器**对准** `yeban_dsp::limiter::Limiter` **本身**，理由是
//! [ADR-0001 D44(c)]（`docs/adr/ADR-0001-workspace-topology-and-version-pinning.md:465`）
//! 把本器件定为 `yeban-dsp` 的公共面：dsp 的任何消费者（含将来的离线母带渲染）
//! 都直接调用它，而**不经过** `yeban-engine`。因此"引擎那条路全 0"不构成
//! "本器件自身全 0"的完整证据。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配/释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎的 `rt_probe`（`RtLockProbe` / `diag`）承担，
//!   本 crate **没有**那套探针 ⇒ 本文件对锁与 I/O **不表态**
//!   （"已证明为 0"是**没有**的，只有源码形状："`process_stereo` 里没有锁、没有 I/O"）。
//!
//! # 仪器口径（与 `compressor_rt_zero_alloc.rs` / `channel_strip_rt_zero_alloc.rs` 相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数。
//! 2. **窗口只包住实时路径**：缓冲区与参数准备在窗口**外**做。
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**，不是"没有计数器"。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::limiter::{
    LIMITER_RELEASE_PER_SAMPLE, LIMITER_THRESHOLD, LOOKAHEAD_SAMPLES, Limiter,
};

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
        // `realloc` 在实时路径上同样禁止。这里**同时**记进两个分量。
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

/// 立体声观测窗口的帧数（`128` 与本仓库的固定处理块长同值 [ARCH-DET-001]）。
const WINDOW_FRAMES: usize = 128;
/// 判据 1 的量子数（每量子 128 帧立体声）。
const RT_QUANTA: u64 = 10_000;
/// 判据 1 里每多少个量子换一次阈值（覆盖 `set_threshold`）。
const THRESHOLD_CHANGE_EVERY: u64 = 1_000;
/// 判据 1 里每多少个量子换一次释放量（覆盖 `set_release_per_sample`）。
const RELEASE_CHANGE_EVERY: u64 = 2_000;
/// 判据 1 里每多少个量子 `reset()` 一次（覆盖 `reset`）。
const RESET_EVERY: u64 = 4_000;

/// 量什么：`limiter` 的逐样本路径在 10 000 个量子（每量子 128 帧 × 2 声道）里的
/// **堆分配次数与释放次数**（单位：次数），窗口内还夹着
/// `set_threshold` / `set_release_per_sample` / `reset`。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（缓冲区与注入参数）----
    let mut left = vec![0.0f32; WINDOW_FRAMES];
    let mut right = vec![0.0f32; WINDOW_FRAMES];
    let mut limiter = Limiter::new();

    // ---- 观测窗口 ----
    // ⚠ 窗口里必须**每量子重填**信号：器件原地改写，且增益 < 1 时后续样本会被
    // 反复压低 ⇒ 不重填的话信号会衰减、`reduction_count` 就再也涨不动。
    // 重填用确定性相位推进，**不分配**（只是 128 次 `sin`）。
    let mut processed_frames: u64 = 0;
    let mut phase: f32 = 0.0;
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            for frame in 0..WINDOW_FRAMES {
                // 幅度 1.6：**确定**越过 0.9 阈值 ⇒ 压限分支每个量子都被走到。
                let value = 1.6 * phase.sin();
                phase += 0.05;
                if phase > core::f32::consts::TAU {
                    phase -= core::f32::consts::TAU;
                }
                left[frame] = value;
                right[frame] = value * 0.5;
            }
            let frames = limiter.process_stereo(&mut left, &mut right);
            processed_frames += frames as u64;
            if quantum % THRESHOLD_CHANGE_EVERY == 0 {
                limiter.set_threshold(if quantum % (THRESHOLD_CHANGE_EVERY * 2) == 0 {
                    LIMITER_THRESHOLD
                } else {
                    0.5
                });
            }
            if quantum % RELEASE_CHANGE_EVERY == 0 {
                limiter.set_release_per_sample(if quantum % (RELEASE_CHANGE_EVERY * 2) == 0 {
                    LIMITER_RELEASE_PER_SAMPLE
                } else {
                    0.0
                });
            }
            if quantum % RESET_EVERY == 0 {
                limiter.reset();
            }
        }
    });

    // 覆盖度自检：窗口里真的在处理（否则"全 0"可能是"什么都没跑"）。
    assert_eq!(
        processed_frames,
        RT_QUANTA * WINDOW_FRAMES as u64,
        "量子记账：每量子 {WINDOW_FRAMES} 帧立体声"
    );
    assert!(limiter.engaged(), "窗口里从未压过 ⇒ 判据测错了对象（假绿）");
    assert!(
        limiter.reduction_count() > 0,
        "窗口里从未压过 ⇒ 判据测错了对象（假绿）"
    );
    assert!(
        left.iter().chain(right.iter()).all(|s| s.is_finite()),
        "路径产出了非有限值"
    );
    assert_eq!(limiter.latency_samples(), LOOKAHEAD_SAMPLES);
    eprintln!(
        "[yeban-dsp/RT] limiter 10 000 量子 × ({WINDOW_FRAMES} 帧立体声): \
         allocations={} deallocations={} processed_frames={} reductions={}",
        reading.allocations,
        reading.deallocations,
        processed_frames,
        limiter.reduction_count()
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
