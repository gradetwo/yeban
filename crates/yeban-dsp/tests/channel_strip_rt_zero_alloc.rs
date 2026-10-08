//! `yeban_dsp::channel_strip` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要这个文件（`yeban-engine` 的 `rt_zero_alloc` 覆盖不到它）
//!
//! `crates/yeban-engine/tests/rt_zero_alloc.rs` 覆盖的是**引擎的音频路径**
//! （`EngineRuntime::render_block` → 逐轨合成/电平 → 母线限制器 → SPSC 发布）。
//!
//! 本模块的 [`yeban_dsp::channel_strip`] **已经被引擎引用并构造**（`db1850f` 起），
//! 核实依据：
//!
//! ```text
//! $ grep -rn 'ChannelStrip::new' crates/yeban-engine/src/
//! crates/yeban-engine/src/rt.rs:985: *entry = (*id, Some(ChannelStrip::new(params, sample_rate)));
//! $ grep -n 'pub use yeban_dsp::channel_strip' crates/yeban-engine/src/insert.rs
//! 165:pub use yeban_dsp::channel_strip::{
//! ```
//!
//! 但引擎那条判据**没有武装**通道条，因此它的"全 0"仍然**不构成**本器件零分配的
//! 证据（同一条核实纪律，换成"引擎侧有没有走到这条路径"的问法）：
//!
//! ```text
//! $ grep -c 'strip' crates/yeban-engine/tests/rt_zero_alloc.rs
//! 0
//! ```
//!
//! ⛔ 不许把引擎那条判据的全 0 当成 `channel_strip` 的读数引用。本文件把同一套仪器
//! **对准** `channel_strip` 本身。
//!
//! ⚠ 本文件**只**覆盖"堆分配 / 堆释放"两个分量。锁与阻塞 I/O 的分量由引擎的
//! `rt_probe`（`RtLockProbe` / `diag`）承担，本 crate **没有**那套探针 ⇒
//! 本文件对锁与 I/O **不表态**（不是"已证明为 0"）。`channel_strip` 的逐样本路径里
//! 没有锁、没有 I/O 调用，但那是**源码形状**的证据，不是运行期读数。
//!
//! # 仪器口径（与 `compressor_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数
//!    （libtest 起线程 / 收结果不会污染读数）。
//! 2. **窗口只包住实时路径**：`ChannelStrip::new` 与缓冲区准备在窗口**外**做。
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次
//!    `Vec::<u8>::with_capacity(1)`，要求 `allocations == 1 && deallocations == 1`。
//!    若计数器坏了，判据 2 会红 —— 因此判据 1 的"全 0"是**测出来的 0**。
//!
//! # 本器件的特别之处：EQ 级有一处"块 API 适配"
//!
//! `channel_strip` 的②级调用 [`yeban_dsp::shaping::ShapingEq::process`]，而那是一个
//! 接受四个切片的**块** API。本器件用**栈数组**（`[f32; EQ_CHUNK_FRAMES] = 64`）
//! 做中间缓冲 —— 判据 1 因此也覆盖了那条路径（默认参数下 EQ 是启用的，
//! 且 `128` 帧的块跨过两个 chunk 边界）。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::channel_strip::{ChannelStrip, ChannelStripParams, FilterParams};

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
        // 它既是一次分配也是一次释放。
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
/// 判据 1 的量子数。
const RT_QUANTA: u64 = 10_000;
/// 判据 1 里每多少个量子换一次参数（覆盖 `set_params`，含 EQ / 滤波 / 压缩三组）。
const PARAM_CHANGE_EVERY: u64 = 1_000;
/// 判据 1 里每多少个量子换一次采样率（覆盖 `set_sample_rate`）。
const SAMPLE_RATE_CHANGE_EVERY: u64 = 2_000;
/// 判据 1 里每多少个量子 `reset` 一次（覆盖 `reset`）。
const RESET_EVERY: u64 = SAMPLE_RATE_CHANGE_EVERY * 4;

/// 量什么：`channel_strip` 的逐样本路径在 **10 000 个量子**里的
/// **堆分配次数与堆释放次数**（单位：次数）。
///
/// 窗口里跑的是：`process_stereo`（128 帧）+ `process_mono`（128 帧），
/// 并夹着 `set_params`（三级参数全换）/ `set_sample_rate` / `reset`。
/// 默认参数下 EQ、滤波、动态**三级全开** ⇒ EQ 的栈数组路径也被覆盖。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（缓冲区、参数、构造）----
    let mut left = vec![0.0f32; WINDOW_FRAMES];
    let mut right = vec![0.0f32; WINDOW_FRAMES];
    let mut mono = vec![0.0f32; WINDOW_FRAMES];

    // 参数组 A：默认（三级全开，EQ 平坦，截止 20 kHz，压缩 −12/4:1）。
    let params_a = ChannelStripParams::DEFAULT;
    // 参数组 B：三级全开但**全部非平凡** —— EQ 三段都动、截止 1 kHz、压缩更狠。
    let params_b = ChannelStripParams {
        input_gain_db: 6.0,
        eq: yeban_dsp::shaping::EqParams {
            low_gain: 6.0,
            low_freq: 300.0,
            mid_gain: -6.0,
            mid_freq: 2_000.0,
            mid_q: 2.0,
            high_gain: 6.0,
            high_freq: 6_000.0,
        },
        filter: FilterParams {
            cutoff_hz: 1_000.0,
            resonance: 0.5,
            drive: 0.5,
        },
        compressor: yeban_dsp::compressor::CompressorParams {
            threshold_db: -30.0,
            ratio: 20.0,
            knee_db: 1.0,
            ..yeban_dsp::compressor::CompressorParams::DEFAULT
        },
        output_gain_db: -3.0,
        ..ChannelStripParams::DEFAULT
    };
    let mut strip = ChannelStrip::new(params_a, 48_000.0);

    // ---- 观测窗口 ----
    let mut expected_frames: u64 = 0;
    let mut quanta_since_reset: u64 = 0;
    // ⚠ 窗口里必须**每量子重填**信号：原地处理会把同一块反复改写，十几个量子之后
    // 它趋近于 0，`gain_reduction_count` 就再也涨不动了（`compressor` 线的实测教训）。
    // 重填用确定性相位推进，**不分配**（只是若干次 `sin`/`cos`）。
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
            strip.process_stereo(&mut left, &mut right);
            strip.process_mono(&mut mono);
            quanta_since_reset += 1;
            expected_frames = quanta_since_reset * (WINDOW_FRAMES as u64 * 2);
            if quantum % PARAM_CHANGE_EVERY == 0 {
                strip.set_params(if quantum % (PARAM_CHANGE_EVERY * 2) == 0 {
                    params_a
                } else {
                    params_b
                });
            }
            if quantum % SAMPLE_RATE_CHANGE_EVERY == 0 {
                strip.set_sample_rate(if quantum % (SAMPLE_RATE_CHANGE_EVERY * 2) == 0 {
                    48_000.0
                } else {
                    96_000.0
                });
            }
            if quantum % RESET_EVERY == 0 {
                strip.reset();
                quanta_since_reset = 0;
                expected_frames = 0;
            }
        }
    });

    // ---- 覆盖度自检（否则"全 0"可能是"什么都没跑"）----
    assert!(expected_frames > 0, "断言本身失效（期望值被算成 0）");
    assert_eq!(
        strip.processed_frames(),
        expected_frames,
        "量子记账：每量子 {WINDOW_FRAMES} 帧立体声 + {WINDOW_FRAMES} 帧单声道"
    );
    assert!(
        strip.gain_reduction_count() > 0,
        "窗口里从未压缩过 ⇒ 判据测错了对象（动态级没被走到）"
    );
    assert!(
        strip.output_peak().is_finite() && strip.output_rms().is_finite(),
        "路径产出了非有限读数"
    );
    assert!(
        left.iter()
            .chain(right.iter())
            .chain(mono.iter())
            .all(|s| s.is_finite()),
        "路径产出了非有限值"
    );
    eprintln!(
        "[yeban-dsp/RT] channel_strip {RT_QUANTA} 量子 × ({WINDOW_FRAMES} 帧立体声 + {WINDOW_FRAMES} 帧单声道)，\
         三级全开（含 EQ 的栈数组块路径）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    eprintln!(
        "[yeban-dsp/RT] 覆盖度读数: processed_frames={} gain_reduction_count={} output_peak={:.6} output_rms={:.6}",
        strip.processed_frames(),
        strip.gain_reduction_count(),
        strip.output_peak(),
        strip.output_rms()
    );
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

/// 量什么：正对照 —— 在**同一个**观测窗口里显式分配一次，读数必须是
/// `allocations == 1 && deallocations == 1`（单位：次数）。
///
/// 判据：两条都要**非 0 且相等**。若计数器坏了（永远读 0），这条会红 ⇒
/// 上一条判据的"全 0"才是有牙的 0。
#[test]
fn the_counter_has_teeth_a_deliberate_allocation_is_counted() {
    let reading = window(|| {
        // ⚠ `Vec::new()` **不分配**（容量 0）—— 用它当注入是无效注入（见判据 3）。
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

/// 量什么：**EQ 级开启**与**EQ 级旁通**两种参数下，`process_stereo` 的
/// 10 000 量子分配读数（单位：次数）。
///
/// 为什么单列一条：本器件唯一一处"块 API 适配"就在 EQ 级（栈数组 + chunk 循环）。
/// 判据 1 用的是默认参数（EQ 启用），但它在窗口里会被 `set_params` 切成两种参数；
/// 这条判据把"EQ 恒启用"与"EQ 恒旁通"**分开**各跑 10 000 量子，让两个读数各自无歧义。
///
/// 判据：两条路径都 `allocations == 0 && deallocations == 0`，且两条路径的
/// 输出**不同**（证明 EQ 开关真的改变了行为，不是"两次都跑了个空壳"）。
#[test]
fn the_eq_block_path_allocates_nothing_with_and_without_the_eq() {
    let mut eq_l = vec![0.0f32; WINDOW_FRAMES];
    let mut eq_r = vec![0.0f32; WINDOW_FRAMES];
    let mut bypass_l = vec![0.0f32; WINDOW_FRAMES];
    let mut bypass_r = vec![0.0f32; WINDOW_FRAMES];

    let with_eq = ChannelStripParams {
        eq: yeban_dsp::shaping::EqParams {
            low_gain: 9.0,
            high_gain: -9.0,
            ..yeban_dsp::shaping::EqParams::default()
        },
        ..ChannelStripParams::DEFAULT
    };
    let without_eq = ChannelStripParams {
        eq_enabled: false,
        ..with_eq
    };

    let mut a = ChannelStrip::new(with_eq, 48_000.0);
    let mut b = ChannelStrip::new(without_eq, 48_000.0);

    let reading = window(|| {
        for _ in 0..RT_QUANTA {
            for (i, s) in eq_l.iter_mut().enumerate() {
                *s = 0.6 * ((i as f32) * 0.11).sin();
            }
            for (i, s) in eq_r.iter_mut().enumerate() {
                *s = 0.6 * ((i as f32) * 0.13).cos();
            }
            bypass_l.copy_from_slice(&eq_l);
            bypass_r.copy_from_slice(&eq_r);
            a.process_stereo(&mut eq_l, &mut eq_r);
            b.process_stereo(&mut bypass_l, &mut bypass_r);
        }
    });

    let differing = eq_l
        .iter()
        .zip(bypass_l.iter())
        .filter(|(x, y)| x.to_bits() != y.to_bits())
        .count();
    assert_eq!(
        a.processed_frames(),
        RT_QUANTA * WINDOW_FRAMES as u64,
        "EQ 启用侧的量子记账"
    );
    assert_eq!(b.processed_frames(), a.processed_frames());
    assert!(
        a.gain_reduction_count() > 0 && b.gain_reduction_count() > 0,
        "两侧的动态级都必须真的压缩过（否则判据测的是空壳）"
    );
    eprintln!(
        "[yeban-dsp/RT] channel_strip EQ 启用 vs 旁通各 {RT_QUANTA} 量子: allocations={} deallocations={}；\
         两侧最后一帧差异计数={differing}/{WINDOW_FRAMES}",
        reading.allocations, reading.deallocations
    );
    assert!(
        differing > 0,
        "EQ 启用与旁通的输出逐位相同 ⇒ 这条判据里 EQ 开关没牙"
    );
    assert_eq!(reading.allocations, 0, "EQ 块路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "EQ 块路径发生堆释放");
}
