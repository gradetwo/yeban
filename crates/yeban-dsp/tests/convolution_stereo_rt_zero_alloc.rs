//! `yeban_dsp::convolution_stereo` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要一个独立文件
//!
//! `convolution_stereo` 是**新写**的模块，引擎**还没有**接线（核实方式：`docs/ledger/
//! integration-rulings-notes.md:88` 的"未接线模块名"口径 + 本票复核，该处现记
//! `drums` 一个模块名，`convolution` / `convolution_stereo` 都不在已接线清单里）。
//! 因此 `crates/yeban-engine/tests/rt_zero_alloc.rs` 那条判据**根本走不到**本器件，
//! 它的"全 0"不构成这里的证据。本文件把同一套仪器**对准** `convolution_stereo`。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配 / 堆释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎的 `rt_probe` 承担，本 crate **没有**那套探针 ⇒
//!   本文件对锁与 I/O **不表态**（不是"已证明为 0"）。
//!
//! # 仪器口径（与 `convolution_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：`set_impulse_response`（唯一分配入口）与缓冲区准备
//!    都在窗口**外**做；
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::convolution::{CONV_BLOCK_FRAMES, CONV_LATENCY};
use yeban_dsp::convolution_stereo::{TRUE_STEREO_PATHS, TrueStereoConvolution};

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

/// 观测窗口的帧数（与 [`CONV_BLOCK_FRAMES`] 同值，即引擎的固定块长）。
const WINDOW_FRAMES: usize = CONV_BLOCK_FRAMES;
/// 判据 1 的量子数（每量子 128 帧 × **2 声道**）。
const RT_QUANTA: u64 = 5_000;
/// 判据 1 里每多少个量子 `reset()` 一次（覆盖 `reset`）。
const RESET_EVERY: u64 = 2_000;
/// 判据 1 的 IR 长度（帧）：`1_024 / 128 = 8` 个分区 × 4 条通路。
const IR_FRAMES: usize = 1_024;

/// 确定性重填一个交错块（不用分配，也不用 RNG 对象）。
fn refill(block: &mut [f32], quantum: u64, scale: f32) {
    for (i, s) in block.iter_mut().enumerate() {
        *s = scale * (quantum as f32 * 0.37 + i as f32 * 0.011).sin();
    }
}

/// 确定性 IR（同样的理由）。
fn make_ir(frames: usize, seed: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (i as f32 * 0.13 + seed).sin() * 0.1 / (1.0 + i as f32 * 0.001))
        .collect()
}

// ---------------------------------------------------------------------------
// 判据 0：公共常量（代价契约）
// ---------------------------------------------------------------------------

/// 量什么：`convolution_stereo` 的公共常量（单位：条 / 帧）。
///
/// 判据：真立体声是 `2 × 2 = 4` 条通路；延迟沿用单声道核的 `0`。
#[test]
fn public_constants_pin_the_topology() {
    assert_eq!(TRUE_STEREO_PATHS, 4, "2 个输入声道 × 2 个输出声道");
    assert_eq!(CONV_LATENCY, 0, "四条通路的延迟都是 0 ⇒ 本器件也是 0");
    eprintln!(
        "[yeban-dsp/RT] convolution_stereo 常量: paths={TRUE_STEREO_PATHS} \
         latency={CONV_LATENCY} block={CONV_BLOCK_FRAMES}"
    );
}

// ---------------------------------------------------------------------------
// 判据 1：实时路径零分配
// ---------------------------------------------------------------------------

/// 量什么：`convolution_stereo` 的**逐块路径**在 **5 000 个量子**里的堆分配次数与
/// 堆释放次数（单位：次数）。每个量子是 `128` 帧 × `2` 声道的交错块。
///
/// 窗口里跑的是：一个装了四条 `1_024` 帧 IR 的 `TrueStereoConvolution` 的 `process`，
/// 每 [`RESET_EVERY`] 个量子夹一次 `reset()`。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_5_000_stereo_quanta() {
    // ---- 窗口**外**的准备（IR、构造、缓冲区）----
    let h_ll = make_ir(IR_FRAMES, 0.0);
    let h_lr = make_ir(IR_FRAMES, 1.0);
    let h_rl = make_ir(IR_FRAMES, 2.0);
    let h_rr = make_ir(IR_FRAMES, 3.0);
    let mut conv = TrueStereoConvolution::new();
    assert_eq!(
        conv.set_impulse_response(&h_ll, &h_lr, &h_rl, &h_rr),
        IR_FRAMES
    );
    assert!(conv.is_configured());
    assert_eq!(conv.latency_samples(), 0);

    let mut block = vec![0.0f32; WINDOW_FRAMES * 2];

    // ---- 观测窗口 ----
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            refill(&mut block, quantum, 0.6);
            conv.process(&mut block);
            if quantum % RESET_EVERY == 0 {
                conv.reset();
            }
        }
    });

    // ---- 覆盖度自检（否则"全 0"可能是"什么都没跑"）----
    let last = RT_QUANTA - 1;
    let mut expected = vec![0.0f32; WINDOW_FRAMES * 2];
    refill(&mut expected, last, 0.6);
    let changed = block
        .iter()
        .zip(&expected)
        .filter(|(out, inp)| (*out - *inp).abs() > 1e-4)
        .count();
    assert!(
        changed >= WINDOW_FRAMES,
        "窗口最后一块的输出与输入太像（{changed} 个样本改变，共 {}）⇒ \
         四条卷积核没真的参与运算，本判据测的是空壳",
        WINDOW_FRAMES * 2
    );
    assert!(block.iter().all(|v| v.is_finite()), "路径产出了非有限值");
    let peak = block.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "输出全零 ⇒ 判据测错了对象");

    eprintln!(
        "[yeban-dsp/RT] convolution_stereo {RT_QUANTA} 量子 × ({WINDOW_FRAMES} 帧立体声，\
         4 条 IR 各 {IR_FRAMES} 帧 = {} 个分区，每 {RESET_EVERY} 量子 reset 一次): \
         allocations={} deallocations={}",
        IR_FRAMES / CONV_BLOCK_FRAMES,
        reading.allocations,
        reading.deallocations
    );
    eprintln!(
        "[yeban-dsp/RT] 覆盖度读数: 最后一块改变样本={changed}/{} peak={peak:.6} \
         ir_frames={}",
        WINDOW_FRAMES * 2,
        conv.ir_frames()
    );
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

// ---------------------------------------------------------------------------
// 判据 2 / 3：仪器的正对照与无效注入
// ---------------------------------------------------------------------------

/// 量什么：正对照 —— 在**同一个**观测窗口里显式分配一次，读数必须是
/// `allocations == 1 && deallocations == 1`（单位：次数）。
///
/// 判据：两条都要**非 0 且相等**。若计数器坏了（永远读 0），这条会红 ⇒
/// 判据 1 的"全 0"才是有牙的 0。
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
    assert_eq!(
        reading.allocations + reading.deallocations,
        0,
        "`Vec::new()` 不该分配"
    );
}

// ---------------------------------------------------------------------------
// 判据 4 / 5：`set_impulse_response` 的分配边界
// ---------------------------------------------------------------------------

/// 量什么：**同长度**换四条 IR + 一次 `process`，在窗口里的分配/释放读数
/// （单位：次数）。
///
/// 为什么单列一条：这是本器件**唯一**的分配入口，接线的第一问就是"换 IR 能不能在
/// 音频线程上做"。实测答案是"**同长度**可以"（四条核的缓冲区已够大 ⇒ 只 `fill`）。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn swapping_irs_of_the_same_length_allocates_nothing() {
    let a = make_ir(IR_FRAMES, 0.0);
    let b = make_ir(IR_FRAMES, 2.0);
    let mut conv = TrueStereoConvolution::new();
    conv.set_impulse_response(&a, &a, &a, &a); // 窗口外：建容量
    let mut block = vec![0.0f32; WINDOW_FRAMES * 2];

    let reading = window(|| {
        conv.set_impulse_response(&b, &b, &b, &b);
        refill(&mut block, 7, 0.5);
        conv.process(&mut block);
    });

    eprintln!(
        "[yeban-dsp/RT] convolution_stereo 同长度换 4 条 IR（{IR_FRAMES} 帧）+ process: \
         allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert_eq!(reading.allocations, 0, "同长度换 IR 不该分配");
    assert_eq!(reading.deallocations, 0, "同长度换 IR 不该释放");
}

/// 量什么：**改变长度**换 IR（`1_024` → `128` 帧）在窗口里的分配/释放读数
/// （单位：次数）。
///
/// 判据：`allocations > 0`。这是上一条判据的**反对照**：它证明
/// `set_impulse_response` **真的**是分配入口 —— 所以"同长度换 IR 全 0"不是
/// "这个方法根本不分配"的假象，也所以**换 IR 不能放在音频线程上**。
#[test]
fn changing_the_ir_length_inside_the_window_would_allocate() {
    let long = make_ir(IR_FRAMES, 0.0);
    let short = make_ir(CONV_BLOCK_FRAMES, 1.0);
    let mut conv = TrueStereoConvolution::new();
    conv.set_impulse_response(&long, &long, &long, &long);

    let reading = window(|| {
        conv.set_impulse_response(&short, &short, &short, &short);
    });

    eprintln!(
        "[yeban-dsp/RT] convolution_stereo 改长度换 4 条 IR（{IR_FRAMES} → \
         {CONV_BLOCK_FRAMES} 帧）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "改长度换 IR 居然零分配 ⇒ 上一条判据的反对照失效（本器件可能整体不分配）"
    );
}
