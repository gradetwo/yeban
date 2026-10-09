//! `yeban_dsp::convolution_reverb` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要一个独立文件
//!
//! `convolution_reverb` 是**新写**的模块，引擎**还没有**接线（核实方式：`docs/ledger/
//! integration-rulings-notes.md` 的"未接线模块名"口径 —— 那段现位于**第 88 行**，
//! 只记 `drums` 一个模块名；`convolution` / `convolution_stereo` /
//! `convolution_reverb` 都不在已接线清单里，本票复核一致）。
//! 因此 `crates/yeban-engine/tests/rt_zero_alloc.rs` 那条判据**根本走不到**本器件，
//! 它的"全 0"不构成这里的证据。本文件把同一套仪器**对准** `convolution_reverb`。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配 / 堆释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎的 `rt_probe` 承担，本 crate **没有**那套探针 ⇒
//!   本文件对锁与 I/O **不表态**（不是"已证明为 0"）。
//!
//! # 本器件特有的一个分量：**调参**不分配
//!
//! [`ConvolutionReverb::set_params`] 在音频线程上会被逐块调用（拖预延迟/干湿旋钮）。
//! 预延迟线按 `MAX_PRE_DELAY_FRAMES` 帧**预分配**，因此改预延迟只是改一个 `usize`。
//! 判据 1 的窗口里**故意**每 250 个量子换一次 `pre_delay_s`（`0.0` ↔ `0.1`）与
//! `ir_gain_db`（`0.0` ↔ `+6.0206`），把这条钉住。
//!
//! # 仪器口径（与 `convolution_stereo_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：`set_sample_rate` / `set_impulse_response`（两个分配
//!    入口）与缓冲区准备都在窗口**外**做；
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::convolution::CONV_BLOCK_FRAMES;
use yeban_dsp::convolution::CONV_LATENCY;
use yeban_dsp::convolution::CONV_MAX_IR_FRAMES;
use yeban_dsp::convolution_reverb::{
    ConvolutionReverb, ConvolutionReverbParams, MAX_IR_GAIN_DB, MAX_PRE_DELAY_FRAMES,
    MAX_PRE_DELAY_SECONDS, MIN_IR_GAIN_DB,
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
    /// 当前存活的**字节**数（单位：字节）。只在 `ARMED` 时维护。
    static LIVE_BYTES: Cell<u64> = const { Cell::new(0) };
    /// 窗口内的**峰值**存活字节数（单位：字节）。
    static PEAK_BYTES: Cell<u64> = const { Cell::new(0) };
}

/// 本线程是否在分配观测窗口内（`try_with`：线程 teardown 期间绝不 panic）。
fn alloc_armed_here() -> bool {
    ARMED.try_with(Cell::get).unwrap_or(false)
}

/// 记一次存活字节的净增（`delta` 为负表示释放）。
fn track_bytes(delta: i64) {
    if !alloc_armed_here() {
        return;
    }
    let _ = LIVE_BYTES.try_with(|live| {
        let next = if delta >= 0 {
            live.get().saturating_add(delta.unsigned_abs())
        } else {
            live.get().saturating_sub(delta.unsigned_abs())
        };
        live.set(next);
        let _ = PEAK_BYTES.try_with(|peak| {
            if next > peak.get() {
                peak.set(next);
            }
        });
    });
}

// SAFETY: 每个方法都只是"按线程计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用状态（计数器是线程局部 `Cell`，访问不分配、不加锁）。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if alloc_armed_here() {
            let _ = ALLOCATIONS.try_with(|cell| cell.set(cell.get() + 1));
        }
        // SAFETY: 契约由本类型的调用方持有；这里原样转发给系统分配器。
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            track_bytes(layout.size() as i64);
        }
        pointer
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if alloc_armed_here() {
            let _ = DEALLOCATIONS.try_with(|cell| cell.set(cell.get() + 1));
        }
        track_bytes(-(layout.size() as i64));
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
        let pointer = unsafe { System.realloc(ptr, layout, new_size) };
        track_bytes(new_size as i64 - layout.size() as i64);
        pointer
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

/// 一次**字节**观测的读数（单位：字节），`allocations` 是同窗口的分配次数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ByteReading {
    allocations: u64,
    live_bytes: u64,
    peak_bytes: u64,
}

/// 在一个观测窗口里跑 `body`，返回窗口内的分配次数与**峰值存活字节数**。
///
/// 峰值口径：窗口内 `LIVE_BYTES` 的最大值（单位：字节）。窗口**之前**就已存活的
/// 分配不计入（它们的 `alloc` 未武装），因此这个读数数的是**窗口内新分配**的净存活
/// 峰值；两个输入长度的读数可以直接相减比较。
fn byte_window<F: FnOnce()>(body: F) -> ByteReading {
    ALLOCATIONS.with(|c| c.set(0));
    DEALLOCATIONS.with(|c| c.set(0));
    LIVE_BYTES.with(|c| c.set(0));
    PEAK_BYTES.with(|c| c.set(0));
    ARMED.with(|c| c.set(true));
    body();
    ARMED.with(|c| c.set(false));
    ByteReading {
        allocations: ALLOCATIONS.with(Cell::get),
        live_bytes: LIVE_BYTES.with(Cell::get),
        peak_bytes: PEAK_BYTES.with(Cell::get),
    }
}

// ---------------------------------------------------------------------------
// 判据规模（改动这些常量就是改动判据强度）
// ---------------------------------------------------------------------------

/// 观测窗口的帧数（与 [`CONV_BLOCK_FRAMES`] 同值，即引擎的固定块长）。
const WINDOW_FRAMES: usize = CONV_BLOCK_FRAMES;
/// 判据 1 的量子数（每量子 128 帧 × 2 声道）。
const RT_QUANTA: u64 = 5_000;
/// 判据 1 里每多少个量子 `reset()` 一次（覆盖 `reset`）。
const RESET_EVERY: u64 = 2_000;
/// 判据 1 里每多少个量子换一次参数（覆盖 `set_params` 的零分配）。
const RETUNE_EVERY: u64 = 250;
/// 判据 1 的 IR 长度（帧）：`1_024 / 128 = 8` 个分区 × 4 条通路。
const IR_FRAMES: usize = 1_024;
/// 判据用的采样率（Hz）。48 kHz 下 `0.1 s` = 4 800 帧 < 上限 9 600。
const SAMPLE_RATE: f32 = 48_000.0;

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

/// 一台已配置好的外壳（两个分配入口都在**窗口之外**调用）。
fn configured_shell() -> ConvolutionReverb {
    let mut shell = ConvolutionReverb::new();
    shell.set_sample_rate(SAMPLE_RATE);
    let ir_a = make_ir(IR_FRAMES, 0.0);
    let ir_b = make_ir(IR_FRAMES, 1.0);
    let ir_c = make_ir(IR_FRAMES, 2.0);
    let ir_d = make_ir(IR_FRAMES, 3.0);
    assert_eq!(
        shell.set_impulse_response(&ir_a, &ir_b, &ir_c, &ir_d),
        IR_FRAMES
    );
    shell.set_params(ConvolutionReverbParams {
        pre_delay_s: 0.0,
        dry: 0.5,
        wet: 0.5,
        ir_gain_db: 0.0,
    });
    shell
}

// ---------------------------------------------------------------------------
// 判据 0：公共常量（代价契约）
// ---------------------------------------------------------------------------

/// 量什么：`convolution_reverb` 的公共常量（单位：帧 / 秒 / dB）。
///
/// 判据：预延迟上限 = `100 ms`（`0.1 s`）与 `9 600` 帧（`100 ms @ 96 kHz`）；
/// IR 增益区间 `-60 … +24 dB`；延迟沿用卷积核的 `0`。
#[test]
fn public_constants_pin_the_cost_contract() {
    assert_eq!(MAX_PRE_DELAY_SECONDS, 0.1, "预延迟上限 = 100 ms");
    assert_eq!(MAX_PRE_DELAY_FRAMES, 9_600, "100 ms @ 96 kHz");
    assert_eq!(MIN_IR_GAIN_DB, -60.0);
    assert_eq!(MAX_IR_GAIN_DB, 24.0);
    assert_eq!(CONV_LATENCY, 0, "卷积核零延迟 ⇒ 本器件零延迟");
    eprintln!(
        "[yeban-dsp/RT] convolution_reverb 常量: max_pre_delay_s={MAX_PRE_DELAY_SECONDS} \
         max_pre_delay_frames={MAX_PRE_DELAY_FRAMES} ir_gain_db=[{MIN_IR_GAIN_DB}, \
         {MAX_IR_GAIN_DB}] latency={CONV_LATENCY} block={CONV_BLOCK_FRAMES}"
    );
}

// ---------------------------------------------------------------------------
// 判据 1：实时路径零分配（含逐块调参）
// ---------------------------------------------------------------------------

/// 量什么：`convolution_reverb` 的**逐块路径**在 **5 000 个量子**里的堆分配次数与
/// 堆释放次数（单位：次数）。每个量子是 `128` 帧 × `2` 声道的交错块。
///
/// 窗口里跑的是：一台装了四条 `1_024` 帧 IR 的 `ConvolutionReverb` 的 `process`，
/// 每 [`RETUNE_EVERY`] 个量子换一次 `pre_delay_s` / `ir_gain_db`（`set_params`），
/// 每 [`RESET_EVERY`] 个量子夹一次 `reset()`。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_5_000_quanta() {
    let mut shell = configured_shell();

    let mut block = vec![0.0f32; 2 * WINDOW_FRAMES];

    // ---- 观测窗口 ----
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            refill(&mut block, quantum, 0.6);
            shell.process(&mut block);
            if quantum % RETUNE_EVERY == 0 {
                let phase = (quantum / RETUNE_EVERY).is_multiple_of(2);
                shell.set_params(ConvolutionReverbParams {
                    pre_delay_s: if phase { 0.0 } else { MAX_PRE_DELAY_SECONDS },
                    dry: 0.5,
                    wet: 0.5,
                    ir_gain_db: if phase { 0.0 } else { 6.020_6 },
                });
            }
            if quantum % RESET_EVERY == 0 {
                shell.reset();
            }
        }
    });

    // ---- 覆盖度自检（否则"全 0"可能是"什么都没跑"）----
    let last = RT_QUANTA - 1;
    let mut expected = vec![0.0f32; 2 * WINDOW_FRAMES];
    refill(&mut expected, last, 0.6);
    let changed = block
        .iter()
        .zip(&expected)
        .filter(|(out, inp)| (*out - *inp).abs() > 1e-4)
        .count();
    assert!(
        changed >= WINDOW_FRAMES,
        "窗口最后一块的输出与输入太像（{changed}/{} 个样本改变）⇒ \
         外壳没真的参与运算，本判据测的是空壳",
        2 * WINDOW_FRAMES
    );
    assert!(block.iter().all(|v| v.is_finite()), "路径产出了非有限值");
    let peak = block.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "输出全零 ⇒ 判据测错了对象");

    eprintln!(
        "[yeban-dsp/RT] convolution_reverb {RT_QUANTA} 量子 × ({WINDOW_FRAMES} 帧立体声，\
         IR={IR_FRAMES} 帧 × 4 条通路，每 {RETUNE_EVERY} 量子换参数、每 {RESET_EVERY} \
         量子 reset 一次): allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    eprintln!(
        "[yeban-dsp/RT] 覆盖度读数: 最后一块改变样本={changed}/{} peak={peak:.6} \
         ir_frames={} pre_delay_frames={}",
        2 * WINDOW_FRAMES,
        shell.ir_frames(),
        shell.pre_delay_frames()
    );
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

// ---------------------------------------------------------------------------
// 判据 2：仪器的正对照
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

// ---------------------------------------------------------------------------
// 判据 3 / 4：两个分配入口的反对照
// ---------------------------------------------------------------------------

/// 量什么：在窗口里**改 IR 长度**（`1_024` → `128` 帧）的分配/释放读数（单位：次数）。
///
/// 判据：`allocations > 0`。这是判据 1 的**反对照**：它证明
/// `set_impulse_response` **真的**是分配入口，所以"改长度"不能放在音频线程上；
/// 也所以判据 1 的"全 0"不是"这个器件根本不分配"的假象。
#[test]
fn changing_the_ir_length_inside_the_window_would_allocate() {
    let mut shell = configured_shell();
    let short = make_ir(CONV_BLOCK_FRAMES, 9.0);

    let reading = window(|| {
        shell.set_impulse_response(&short, &short, &short, &short);
    });

    eprintln!(
        "[yeban-dsp/RT] convolution_reverb 改长度换 4 条 IR（{IR_FRAMES} → \
         {CONV_BLOCK_FRAMES} 帧）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "改长度换 IR 居然零分配 ⇒ 判据 1 的反对照失效"
    );
}

/// 量什么：在窗口里从零建一台外壳（`new` ＋ `set_sample_rate`）的分配/释放读数
/// （单位：次数）。
///
/// 判据：`allocations > 0`。这条证明 `set_sample_rate` **真的**在预分配两条
/// `MAX_PRE_DELAY_FRAMES` 帧的延迟线 —— 所以判据 1 里"换 `pre_delay_s` 不分配"
/// 是"线上限已经建好"的结果，不是"根本没有延迟线"。
#[test]
fn building_the_delay_lines_inside_the_window_would_allocate() {
    let reading = window(|| {
        let mut fresh = ConvolutionReverb::new();
        fresh.set_sample_rate(SAMPLE_RATE);
        assert!(!fresh.is_configured(), "只设采样率不算已配置");
    });

    eprintln!(
        "[yeban-dsp/RT] convolution_reverb 窗口内 `new` + `set_sample_rate`: \
         allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "预延迟线居然零分配 ⇒ 判据 1 的反对照失效"
    );
}

// ---------------------------------------------------------------------------
// 判据 5：参数跨度覆盖了整个上限
// ---------------------------------------------------------------------------

/// 量什么：预延迟从 `0.0` 扫到上限时，`set_params` ＋ `process` 的分配读数
/// （单位：次数）。
///
/// 判据：`allocations == 0 && deallocations == 0`，且 `pre_delay_frames()` 真的
/// 到达过 `MAX_PRE_DELAY_FRAMES`（否则这条测的是"没换过参数"）。
#[test]
fn retuning_across_the_whole_pre_delay_range_allocates_nothing() {
    let mut shell = configured_shell();
    // 96 kHz 下 `0.1 s` 恰好是帧数上限 `9 600` ⇒ 这次扫描真的覆盖整条延迟线。
    shell.set_sample_rate(96_000.0);
    let mut block = vec![0.0f32; 2 * WINDOW_FRAMES];
    refill(&mut block, 0, 0.5);

    let mut reached_max = false;
    let reading = window(|| {
        for step in 0..=100u32 {
            let seconds = MAX_PRE_DELAY_SECONDS * step as f32 / 100.0;
            shell.set_params(ConvolutionReverbParams {
                pre_delay_s: seconds,
                dry: 0.3,
                wet: 0.7,
                ir_gain_db: -12.0,
            });
            if shell.pre_delay_frames() == MAX_PRE_DELAY_FRAMES {
                reached_max = true;
            }
            shell.process(&mut block);
        }
    });

    eprintln!(
        "[yeban-dsp/RT] convolution_reverb 预延迟全程扫描（0.0 → {MAX_PRE_DELAY_SECONDS} s，\
         101 步）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reached_max,
        "扫描没有触到帧数上限 {MAX_PRE_DELAY_FRAMES} ⇒ 判据覆盖不足"
    );
    assert_eq!(reading.allocations, 0, "扫预延迟不该分配");
    assert_eq!(reading.deallocations, 0, "扫预延迟不该释放");
}

/// 量什么：`set_mono_impulse_response` / `set_stereo_impulse_response` 在**一次调用**
/// 里的**峰值存活字节数**（单位：字节），输入长度分别取 1× 与 4× 的接受上限
/// [`CONV_MAX_IR_FRAMES`]。
///
/// 判据：两种输入长度的峰值字节**完全相等**，且两次报告的接受帧数都等于上限。
///
/// 为什么：上限的文档承诺是"载入一个误选的超长文件有一个**有界的、可预测的**后果"。
/// 临时零切片若按**未钳制**的输入长度分配，超出的部分一个字节都不参与卷积却照样
/// 占内存，那半句承诺就不成立。本机实测（修复前）：1× 上限 = 32 894 336 B、
/// 4× 上限 = 38 654 336 B，差 5 760 000 B = 3 × 480 000 帧 × 4 B。
///
/// 怎么变红：把 `let frames = h_l.len().min(CONV_MAX_IR_FRAMES);` 改回
/// `let silence = vec![0.0f32; h_l.len()];`（单声道那条同理）⇒ 4× 的读数比 1× 大
/// 3 × 480 000 × 4 B。
#[test]
fn an_over_long_convenience_ir_does_not_scale_the_temporary_with_the_input() {
    /// 超长输入的倍数（相对接受上限）。
    const OVER: usize = 4;

    let capped = vec![0.1f32; CONV_MAX_IR_FRAMES];
    let over_long = vec![0.1f32; CONV_MAX_IR_FRAMES * OVER];

    for (label, one_x, four_x) in [
        ("set_mono_impulse_response", &capped, &over_long),
        ("set_stereo_impulse_response", &capped, &over_long),
    ] {
        let mut at_cap = ConvolutionReverb::new();
        at_cap.set_sample_rate(48_000.0);
        let mut over = ConvolutionReverb::new();
        over.set_sample_rate(48_000.0);
        let (mut accepted_cap, mut accepted_over) = (0usize, 0usize);

        let (mono, stereo) = if label.starts_with("set_mono") {
            (true, false)
        } else {
            (false, true)
        };
        let reading_cap = byte_window(|| {
            accepted_cap = if mono {
                at_cap.set_mono_impulse_response(one_x)
            } else if stereo {
                at_cap.set_stereo_impulse_response(one_x, one_x)
            } else {
                0
            };
        });
        let reading_over = byte_window(|| {
            accepted_over = if mono {
                over.set_mono_impulse_response(four_x)
            } else if stereo {
                over.set_stereo_impulse_response(four_x, four_x)
            } else {
                0
            };
        });

        assert_eq!(
            accepted_cap, CONV_MAX_IR_FRAMES,
            "{label}: 1× 输入必须被接受到上限"
        );
        assert_eq!(
            accepted_over, CONV_MAX_IR_FRAMES,
            "{label}: 4× 输入必须被截断到同一条上限"
        );
        assert!(
            reading_cap.peak_bytes > 0,
            "{label}: 仪表必须看得见分配（否则相等是空转）"
        );
        assert_eq!(
            reading_over.peak_bytes,
            reading_cap.peak_bytes,
            "{label}: 4× 上限的输入多占了 {} 字节的峰值存活内存 —— 临时零切片没有钳到上限",
            reading_over.peak_bytes as i64 - reading_cap.peak_bytes as i64
        );
        eprintln!(
            "[yeban-dsp/RT] {label} 超长输入: 1× peak={} B / 4× peak={} B (相等) accepted={accepted_cap}",
            reading_cap.peak_bytes, reading_over.peak_bytes
        );
    }
}
