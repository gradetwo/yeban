//! `yeban_dsp::smoothing::ParamSmoother` 与
//! `yeban_dsp::oscillator::WavetableOscillator` 的**运行期零分配判据**
//! （[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么是这两个器件、在这一票
//!
//! 本票给这两个类型的**参数入口**补齐了非有限守卫
//! （`ParamSmoother::set_target` / `snap_to`、`WavetableOscillator::set_phase`）。
//! 这些入口**在实时路径上被调用**：参数自动化的事件边界就在音频回调里
//!（`yeban-engine` 的 `ParamTable::accept` 逐事件调 `set_target` / `snap_to`），
//! 而 `ParamSmoother::process` / `WavetableOscillator::process_block` 是逐样本路径。
//! 因此"入口 ＋ 逐样本"两段必须落在**同一个**观测窗口里，读数必须是 `0`。
//!
//! # 仪器口径（与 `reverb_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` ＋ `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：波表的构造（本 crate 里唯一会分配的一步）在窗口**外**；
//! 3. **判据有牙（正对照）**：判据 3 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 3 会红 ——
//!    因此判据 1／2 的"全 0"是**测出来的 0**。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配 / 堆释放"两个分量。锁与阻塞 I/O 的分量由引擎的
//!   `rt_probe` 承担，本 crate **没有**那套探针 ⇒ 本文件对锁与 I/O **不表态**
//!   （不是"已证明为 0"）；
//! - 两个被测类型都**不含**堆所有权（`ParamSmoother` 全是 `f32`，
//!   `WavetableOscillator` 的波表由调用方持有），因此本文件**没有**
//!   "构造期分配入口"的反对照（那种对照在 `reverb_rt_zero_alloc.rs` 里是
//!   `set_sample_rate`）。判据 3 的正对照就是这里的全部"牙"。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::oscillator::{FACTORY_RECIPES, Wavetable, WavetableOscillator};
use yeban_dsp::smoothing::ParamSmoother;

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

/// 一个量子／一个块的帧数（引擎的固定块长）。
const QUANTUM_FRAMES: usize = 128;
/// 观测窗口的量子数。
const RT_QUANTA: u64 = 5_000;
/// 每多少个量子动一次参数。
const RETUNE_EVERY: u64 = 8;
/// 每多少个量子有一次"吸附"（平滑器）／换采样率（振荡器）。
const SNAP_EVERY: u64 = 64;
/// 判据用的采样率（Hz）。
const SAMPLE_RATE: f32 = 48_000.0;

// ---------------------------------------------------------------------------
// 判据 1：平滑器的"参数入口 ＋ 逐样本"零分配
// ---------------------------------------------------------------------------

/// 量什么：`ParamSmoother` 在 **5 000 个量子**里的堆分配次数与堆释放次数
/// （单位：次数）。每个量子是 `128` 个 `process()` 调用。
///
/// 窗口里跑的是：每 [`RETUNE_EVERY`] 个量子一次 `set_target`（**合法值与
/// `NaN`／`±∞` 交替**，两条分支都在窗口内），每 [`SNAP_EVERY`] 个量子一次
/// `snap_to`（同样交替），逐样本 `process()`。
///
/// 判据：`allocations == 0 && deallocations == 0`；且窗口结束时值仍是有限数
/// （非有限目标不得毒化递归状态）。
#[test]
fn the_smoother_parameter_entries_allocate_nothing_over_5_000_quanta() {
    let mut smoother = ParamSmoother::with_default_time(SAMPLE_RATE);
    smoother.snap_to(0.5);
    let mut injected = 0u64;
    let mut finite = true;
    let mut sum = 0.0f32;

    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            // 两个周期的奇偶**分开算**：`RETUNE_EVERY` 整除 `SNAP_EVERY`，若共用同一个
            // 奇偶量，`snap_to` 的敌意分支永远不会被执行（实测：共用时该分支执行 0 次）。
            let valid_target = (quantum / RETUNE_EVERY).is_multiple_of(2);
            if quantum % RETUNE_EVERY == 0 {
                if valid_target {
                    smoother.set_target(0.8);
                } else {
                    smoother.set_target(f32::NAN);
                    injected += 1;
                }
            }
            for _ in 0..QUANTUM_FRAMES {
                sum += smoother.process();
            }
            if quantum % SNAP_EVERY == 0 {
                if (quantum / SNAP_EVERY).is_multiple_of(2) {
                    smoother.snap_to(0.2);
                } else {
                    smoother.snap_to(f32::NEG_INFINITY);
                    injected += 1;
                }
            }
        }
        finite = smoother.value().is_finite() && sum.is_finite();
    });

    // ---- 覆盖度自检（否则"全 0"可能是"什么都没跑"）----
    assert!(
        injected > 0,
        "窗口里没有注入过非有限参数 ⇒ 守卫分支没被覆盖"
    );
    assert!(finite, "非有限参数毒化了平滑器的递归状态");
    assert!(sum > 0.0, "窗口里的输出恒非正 ⇒ 判据测的是空壳");
    assert!(
        smoother.value() != 0.5,
        "窗口结束时值仍是初始的 0.5 ⇒ 平滑器根本没被驱动"
    );

    eprintln!(
        "[yeban-dsp/RT] ParamSmoother {RT_QUANTA} 量子 × ({QUANTUM_FRAMES} 帧，每 \
         {RETUNE_EVERY} 量子 set_target、每 {SNAP_EVERY} 量子 snap_to，含 {injected} 次非有限注入): \
         allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    eprintln!(
        "[yeban-dsp/RT] 覆盖度读数: 末值={} 输出和={sum:.3}",
        smoother.value()
    );
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

// ---------------------------------------------------------------------------
// 判据 2：波表振荡器的"参数入口 ＋ 逐块"零分配
// ---------------------------------------------------------------------------

/// 量什么：`WavetableOscillator` 在 **5 000 个量子**里的堆分配次数与堆释放次数
/// （单位：次数）。每个量子是 `128` 帧的 `process_block`。
///
/// 窗口里跑的是：每 [`RETUNE_EVERY`] 个量子一次 `set_phase` ＋ `set_frequency`
/// ＋ `set_sample_rate`（**合法值与 `NaN`／`±∞` 交替**），逐块 `process_block`。
/// 波表在窗口**外**构造（那是本 crate 里唯一会分配的一步）。
///
/// 判据：`allocations == 0 && deallocations == 0`；输出全部有限；且**至少有一个
/// 量子的输出不是常数**（否则这个夹具只是在跑一个冻结的振荡器）。
#[test]
fn the_oscillator_parameter_entries_allocate_nothing_over_5_000_quanta() {
    let table = Wavetable::from_recipe(FACTORY_RECIPES[0].1);
    let mut oscillator = WavetableOscillator::new(SAMPLE_RATE);
    oscillator.set_frequency(&table, 440.0);
    let mut buffer = [0.0f32; QUANTUM_FRAMES];
    let mut injected = 0u64;
    let mut moving = 0u64;
    let mut finite = true;

    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            // 奇偶量与判据 1 同口径（两个周期分开算，敌意分支才真的被执行）。
            let valid = (quantum / RETUNE_EVERY).is_multiple_of(2);
            if quantum % RETUNE_EVERY == 0 {
                if valid {
                    oscillator.set_phase(0.25);
                    oscillator.set_frequency(&table, 440.0);
                } else {
                    oscillator.set_phase(f32::NAN);
                    oscillator.set_frequency(&table, f32::INFINITY);
                    injected += 1;
                }
            }
            if quantum % SNAP_EVERY == 0 {
                if (quantum / SNAP_EVERY).is_multiple_of(2) {
                    oscillator.set_sample_rate(SAMPLE_RATE, &table);
                } else {
                    oscillator.set_sample_rate(f32::NEG_INFINITY, &table);
                    injected += 1;
                }
            }
            oscillator.process_block(&table, &mut buffer);
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            for sample in &buffer {
                if !sample.is_finite() {
                    finite = false;
                }
                low = low.min(*sample);
                high = high.max(*sample);
            }
            if high > low {
                moving += 1;
            }
        }
    });

    // ---- 覆盖度自检 ----
    assert!(
        injected > 0,
        "窗口里没有注入过非有限参数 ⇒ 守卫分支没被覆盖"
    );
    assert!(finite, "输出里出现非有限值");
    assert!(
        moving > 0,
        "5 000 个量子里的输出没有一个块是变化的 ⇒ 夹具只在跑一个冻结的振荡器"
    );
    assert!(
        oscillator.phase().is_finite(),
        "相位被非有限值毒化成 {}",
        oscillator.phase()
    );

    eprintln!(
        "[yeban-dsp/RT] WavetableOscillator {RT_QUANTA} 量子 × ({QUANTUM_FRAMES} 帧，每 \
         {RETUNE_EVERY} 量子 set_phase/set_frequency、每 {SNAP_EVERY} 量子 set_sample_rate，\
         含 {injected} 次非有限注入): allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    eprintln!("[yeban-dsp/RT] 覆盖度读数: 变化块={moving}/{RT_QUANTA}");
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

// ---------------------------------------------------------------------------
// 判据 3：仪器的正对照
// ---------------------------------------------------------------------------

/// 量什么：正对照 —— 在**同一个**观测窗口里显式分配一次，读数必须是
/// `allocations == 1 && deallocations == 1`（单位：次数）。
///
/// 判据：两条都要**非 0 且相等**。若计数器坏了（永远读 0），这条会红 ⇒
/// 判据 1／2 的"全 0"才是有牙的 0。
#[test]
fn the_counter_has_teeth_a_deliberate_allocation_is_counted() {
    let reading = window(|| {
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
