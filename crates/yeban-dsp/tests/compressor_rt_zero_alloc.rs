//! `yeban_dsp::compressor` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要这个文件（`yeban-engine` 的 `rt_zero_alloc` 覆盖不到它）
//!
//! `crates/yeban-engine/tests/rt_zero_alloc.rs` 的判据汇总当前报 **40 / 40 通过**
//! （本 worktree 实测，见提交说明），但那条判据覆盖的是**引擎的音频路径**：
//! `EngineRuntime::render_block` → 逐轨合成/电平 → 母线限制器 → SPSC 发布。
//!
//! 本模块的 [`yeban_dsp::compressor`] **还没有被引擎调用**，核实依据：
//!
//! - `grep -rn 'compressor' crates/ --include=*.rs` 除 `yeban-dsp` 自身的
//!   `src/lib.rs` 声明与 `src/compressor.rs` 之外**零命中**；
//! - `grep -rn 'use yeban_dsp' crates/yeban-engine/src/` 只命中 `envelope`、
//!   `filter`、`math`、`oscillator`、`meter` 五个模块，**没有** `compressor`。
//!
//! ⇒ 引擎那条判据的"全 0"**不构成**本模块零分配的证据。⛔ 不许把它当成本模块
//! 的读数引用。本文件把同一套仪器**对准** `compressor` 本身。
//!
//! # 仪器口径（与引擎那条判据的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数
//!    （libtest 起线程/收结果不会污染读数）。
//! 2. **窗口只包住实时路径**：`Vec::with_capacity` 之类的准备在窗口**外**做。
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**，不是"没有计数器"。
//!
//! ⚠ 本文件**只**覆盖"堆分配/释放"两个分量。锁与阻塞 I/O 的分量由引擎的
//! `rt_probe`（`RtLockProbe` / `diag`）承担，本 crate **没有**那套探针 ⇒
//! 本文件对锁与 I/O **不表态**（不是"已证明为 0"）。`compressor` 的逐样本路径里
//! 没有锁、没有 I/O 调用，但那是**源码形状**的证据，不是运行期读数。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::compressor::{Compressor, CompressorParams};

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

/// 立体声观测窗口的帧数（`128` 与本仓库的固定处理块长同值 [ARCH-DET-001]）。
const WINDOW_FRAMES: usize = 128;
/// 判据 1 的量子数（每量子 128 帧 × 2 声道）。
const RT_QUANTA: u64 = 10_000;
/// 判据 1 里每多少个量子换一次参数（覆盖 `set_params`）。
const PARAM_CHANGE_EVERY: u64 = 1_000;
/// 判据 1 里每多少个量子换一次采样率（覆盖 `set_sample_rate`）。
const SAMPLE_RATE_CHANGE_EVERY: u64 = 2_000;

/// 量什么：`compressor` 的逐样本路径在 10 000 个量子里的**堆分配次数与释放次数**
/// （单位：次数），窗口内还夹着 `set_params` / `set_sample_rate` / `reset`。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（缓冲区、参数）----
    // ⚠ 立体声与单声道必须用**不同的**缓冲区：单声道路径会把 `left` 的每个样本
    // 都乘一遍增益（幅度变小），而 `process_stereo` 的帧数取 `min(len)`；同一块
    // 反复压会让后面的量子处理的帧数越来越少。第一版这样写，量子记账对不上
    // （实测 767 616，期望 3 840 000）—— 那是夹具的缺陷，不是库的行为。
    let mut left = vec![0.0f32; WINDOW_FRAMES];
    let mut right = vec![0.0f32; WINDOW_FRAMES];
    let mut mono = vec![0.0f32; WINDOW_FRAMES];
    let params_a = CompressorParams::DEFAULT;
    let params_b = CompressorParams {
        threshold_db: -30.0,
        ratio: 20.0,
        knee_db: 1.0,
        ..CompressorParams::DEFAULT
    };
    let mut comp = Compressor::new(params_a, 48_000.0);
    // 预填非零信号，避免"全 0 走的是别的分支"。
    for (i, s) in left.iter_mut().enumerate() {
        *s = 0.5 * (i as f32 * 0.05).sin();
    }
    for (i, s) in right.iter_mut().enumerate() {
        *s = 0.4 * (i as f32 * 0.07).cos();
    }
    for (i, s) in mono.iter_mut().enumerate() {
        *s = 0.6 * (i as f32 * 0.11).sin();
    }

    // 每量子处理的**声道样本数**：立体声按帧推进但按声道记账（2×128），
    // 单声道 1×128。
    const PER_QUANTUM: u64 = (WINDOW_FRAMES * 2 + WINDOW_FRAMES) as u64;
    // `reset()` 的节拍（量子）。它把 `processed_samples` 清零 ——
    // 记账必须按"自上次重置以来的量子数"算。
    const RESET_EVERY: u64 = SAMPLE_RATE_CHANGE_EVERY * 4;

    // ---- 观测窗口 ----
    let mut expected_samples: u64 = 0;
    let mut quanta_since_reset: u64 = 0;
    // ⚠ 窗口里必须**每量子重填**信号：原地压缩会把同一块反复乘增益，
    // 十几个量子之后它下溢到 0，`reduction_count` 就再也涨不动了
    // （第一版就是这样，重置之后读数停在 0 ⇒ 覆盖度自检变红）。
    // 重填用确定性相位推进，**不分配**（只是 128 次 `sin`/`cos`）。
    let mut phase: f32 = 0.0;
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            for s in left.iter_mut() {
                *s = 0.8 * phase.sin();
                phase += 0.05;
                if phase > core::f32::consts::TAU {
                    phase -= core::f32::consts::TAU;
                }
            }
            for s in right.iter_mut() {
                *s = 0.7 * phase.sin();
                phase += 0.07;
                if phase > core::f32::consts::TAU {
                    phase -= core::f32::consts::TAU;
                }
            }
            for s in mono.iter_mut() {
                *s = 0.9 * phase.sin();
                phase += 0.11;
                if phase > core::f32::consts::TAU {
                    phase -= core::f32::consts::TAU;
                }
            }
            comp.process_stereo(&mut left, &mut right);
            comp.process_mono(&mut mono);
            quanta_since_reset += 1;
            expected_samples = quanta_since_reset * PER_QUANTUM;
            if quantum % PARAM_CHANGE_EVERY == 0 {
                comp.set_params(if quantum % (PARAM_CHANGE_EVERY * 2) == 0 {
                    params_a
                } else {
                    params_b
                });
            }
            if quantum % SAMPLE_RATE_CHANGE_EVERY == 0 {
                comp.set_sample_rate(if quantum % (SAMPLE_RATE_CHANGE_EVERY * 2) == 0 {
                    48_000.0
                } else {
                    96_000.0
                });
            }
            if quantum % RESET_EVERY == 0 {
                comp.reset();
                quanta_since_reset = 0;
                expected_samples = 0;
            }
        }
    });

    // 覆盖度自检：窗口里真的在处理（否则"全 0"可能是"什么都没跑"）。
    assert!(expected_samples > 0, "断言本身失效（期望值被算成 0）");
    assert_eq!(
        comp.processed_samples(),
        expected_samples,
        "量子记账：每量子 {WINDOW_FRAMES} 帧立体声（按声道记 ×2）+ {WINDOW_FRAMES} 帧单声道"
    );
    assert!(
        comp.reduction_count() > 0,
        "窗口里从未压缩过 ⇒ 判据测错了对象"
    );
    assert!(
        left.iter()
            .chain(right.iter())
            .chain(mono.iter())
            .all(|s| s.is_finite()),
        "路径产出了非有限值"
    );
    eprintln!(
        "[yeban-dsp/RT] compressor 10 000 量子 × ({} 帧立体声 + {} 帧单声道): \
         allocations={} deallocations={}",
        WINDOW_FRAMES, WINDOW_FRAMES, reading.allocations, reading.deallocations
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
