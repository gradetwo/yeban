//! **第四批补面**：此前**没有任何零分配判据覆盖**的逐样本／逐帧路径的
//! **运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么要单独一个文件（机械读数）
//!
//! 本 crate 的分配探针此前分布在 11 个 `tests/*_rt_zero_alloc.rs` 与
//! `parameter_entry_rt_zero_alloc.rs` 里，它们覆盖的是
//! `limiter` / `compressor` / `reverb` / `convolution` / `convolution_stereo` /
//! `convolution_reverb` / `channel_strip` / `polysynth` / `drums`，以及
//! `comb` / `delay` / `oscillator` / `smoother` 的参数入口。
//! 第四批的注入实测发现**下面 8 条逐样本路径一个探针都没有**：
//! 往 `noise::NoiseGen::process`、`shaping::BitCrusher::process`、
//! `meter::{LevelDetector::analyze, TruePeakDetector::process}`、
//! `loudness::GatedLoudness::add_stereo`、`oversample::Oversampler2x::upsample`、
//! `block::{accumulate, scale_into, mix2_into, peak}`、`loop_window::LoopWindow::fade_in`
//! 里各插入一次 `Vec::<u8>::with_capacity(1)` 时，**全库 457 条判据全绿**
//! （第四批注入表的 D02／D03／D04／D05／D06／D07／D08／D09）。
//! 这不是"某条判据判别力不足"，而是**该面根本没有判据**。
//!
//! # 仪器口径（与 `parameter_entry_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` ＋ `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：会分配的那几步（`GatedLoudness::new_48k` 的环形缓冲等）
//!    都在窗口**外**；
//! 3. **判据有牙（正对照）**：判据 9 在窗口**内**做一次
//!    `Vec::<u8>::with_capacity(1)`，要求 `allocations == 1 && deallocations == 1`。
//!    若计数器坏了，判据 9 会红 —— 因此其余判据的"全 0"是**测出来的 0**。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配／堆释放"两个分量；锁与阻塞 I/O 本 crate **没有**探针
//!   ⇒ 本文件对锁与 I/O **不表态**（不是"已证明为 0"）；
//! - 判据的"有活干"自检用**输出值的有限性与非平凡性**（不是墙钟）⇒ 判据不依赖运行速度。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::block::{accumulate, mix2_into, peak, scale_into};
use yeban_dsp::loop_window::LoopWindow;
use yeban_dsp::loudness::GatedLoudness;
use yeban_dsp::meter::{LevelDetector, TruePeakDetector};
use yeban_dsp::noise::{NoiseColour, NoiseGen};
use yeban_dsp::oversample::Oversampler2x;
use yeban_dsp::shaping::{BitCrusher, CrushParams};

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

/// "零分配"的期望读数。
const SILENT: Reading = Reading {
    allocations: 0,
    deallocations: 0,
};

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

/// 一个量子的帧数（引擎的固定块长）。
const QUANTUM_FRAMES: usize = 128;
/// 判据用的采样率（Hz）。
const SAMPLE_RATE: f32 = 48_000.0;

/// 一段样本的绝对值和（用于"夹具真的算出了东西"的自检）。
fn sum_abs(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |acc, v| acc + v.abs())
}

// ---------------------------------------------------------------------------
// 判据 1：块原语
// ---------------------------------------------------------------------------

/// 量什么：`block` 的四个标量原语在 **10 000 个量子**里的堆分配／堆释放次数
/// （单位：次数）。每个量子处理 `128` 帧。
///
/// 判据：`allocations == 0 && deallocations == 0`，且校验和非零、有限
///（后者防"夹具什么都没算"的假绿）。
///
/// 注入实测：在 `accumulate` 首行插入一次 `Vec::<u8>::with_capacity(1)`
/// （第四批 D08）⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn block_primitives_allocate_nothing_over_10_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 10_000;
    let input = [0.25f32; QUANTUM_FRAMES];
    let mut output = [0.0f32; QUANTUM_FRAMES];
    let mut other = [0.5f32; QUANTUM_FRAMES];
    let mut checksum = 0.0f32;
    let reading = window(|| {
        for _ in 0..QUANTA {
            accumulate(&input, &mut output, 0.5);
            scale_into(&input, &mut other, 0.5);
            mix2_into(&input, &other, &mut output, 0.25, 0.25);
            checksum += peak(&output);
        }
    });
    assert!(
        checksum.is_finite() && checksum > 0.0,
        "夹具没有算出非平凡结果（checksum={checksum}）⇒ 本判据测的是空壳"
    );
    assert_eq!(reading, SILENT, "块原语的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 2：循环微平滑窗
// ---------------------------------------------------------------------------

/// 量什么：`LoopWindow::{fade_in, fade_out}` 在 **5 000 个量子**里的分配／释放次数。
///
/// 判据：全 `0`；且淡出后的首样本逐位为 `0`（窗真的作用了，不是空跑）。
///
/// 注入实测：在 `fade_in` 首行插入一次堆分配（第四批 D09）⇒ 全库 457 条判据**全绿**
/// ⇒ 本判据变红。
#[test]
fn loop_window_fades_allocate_nothing_over_5_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 5_000;
    let fades = LoopWindow::new();
    let mut block = [1.0f32; QUANTUM_FRAMES];
    let reading = window(|| {
        for _ in 0..QUANTA {
            fades.fade_in(&mut block);
            fades.fade_out(&mut block);
        }
    });
    assert_eq!(
        block[QUANTUM_FRAMES - 1].to_bits(),
        0.0f32.to_bits(),
        "淡出必须以 0 结束 ⇒ 窗真的作用了"
    );
    assert!(block.iter().all(|v| v.is_finite()));
    assert_eq!(reading, SILENT, "循环微平滑窗的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 3：有色噪声
// ---------------------------------------------------------------------------

/// 量什么：`NoiseGen::process` 在 **10 000 个量子**（三种颜色轮换 ＋ 每 8 个量子
/// 一次 `set_corner_hz`）里的分配／释放次数。
///
/// 判据：全 `0`；且三色都必须真的走过（各色输出之和有限且不全为 `0`）。
///
/// 注入实测：在 `NoiseGen::process` 首行插入一次堆分配（第四批 D02）
/// ⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn noise_colours_allocate_nothing_over_10_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 10_000;
    let mut generator = NoiseGen::new();
    let mut sums = [0.0f32; 3];
    let reading = window(|| {
        for quantum in 0..QUANTA {
            let colour = (quantum % 3) as usize;
            generator.set_colour(match colour {
                0 => NoiseColour::White,
                1 => NoiseColour::Pink,
                _ => NoiseColour::Brown,
            });
            if quantum % 8 == 0 {
                generator.set_corner_hz(50.0);
            }
            for index in 0..QUANTUM_FRAMES {
                // 非对称的确定序列：白噪声是逐样本直通，交替 ±0.5 的**和恰为 0**
                // ⇒ 那会让"三色都跑过"的自检假红（第一版实测就是这样）。
                let white = if index % 2 == 0 { 0.5 } else { 0.25 };
                sums[colour] += generator.process(white, SAMPLE_RATE);
            }
        }
    });
    assert!(
        sums.iter().all(|s| s.is_finite()) && sums.iter().all(|s| s.abs() > 0.0),
        "三种颜色都必须真的跑过：{sums:?}"
    );
    assert_eq!(reading, SILENT, "有色噪声的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 4／5：电平表两条路径
// ---------------------------------------------------------------------------

/// 量什么：`LevelDetector::analyze` 在 **5 000 个量子**里的分配／释放次数。
///
/// 判据：全 `0`；且保持值的读数非平凡。
///
/// 注入实测：在 `analyze` 首行插入一次堆分配（第四批 D04）
/// ⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn level_detector_analysis_allocates_nothing_over_5_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 5_000;
    let mut detector = LevelDetector::new();
    let samples = [0.5f32; QUANTUM_FRAMES];
    let mut checksum = 0.0f32;
    let reading = window(|| {
        for _ in 0..QUANTA {
            checksum += detector.analyze(&samples).peak_hold;
        }
    });
    assert!(
        checksum.is_finite() && checksum > 0.0,
        "电平表读数恒为 0 ⇒ 本判据测的是空壳"
    );
    assert_eq!(reading, SILENT, "电平表的实时路径分配了内存");
}

/// 量什么：`TruePeakDetector::process` 在 **5 000 个量子**里的分配／释放次数。
///
/// 判据：全 `0`；且真峰值读数落在 `(0, 1]`。
///
/// 注入实测：在 `process` 首行插入一次堆分配（第四批 D05）
/// ⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn true_peak_detection_allocates_nothing_over_5_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 5_000;
    let mut detector = TruePeakDetector::new();
    let samples = [0.5f32; QUANTUM_FRAMES];
    let mut last = 0.0f32;
    let reading = window(|| {
        for _ in 0..QUANTA {
            last = detector.process(&samples);
        }
    });
    assert!(
        last.is_finite() && last > 0.0 && last <= 1.0,
        "真峰值读数 {last} 不在 (0, 1]"
    );
    assert_eq!(reading, SILENT, "真峰值检测的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 6：门限响度
// ---------------------------------------------------------------------------

/// 量什么：`GatedLoudness::add_stereo` 在 **5 000 个量子**里的分配／释放次数
/// （环形缓冲等**构造期**分配在窗口外）。
///
/// 判据：全 `0`；且窗口内各段的分配计数保持 `0`（窗口结束时才读）。
///
/// 注入实测：在 `push_frame` 调用点前插入一次堆分配（第四批 D06）
/// ⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn gated_loudness_accumulation_allocates_nothing_over_5_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 5_000;
    // 构造期分配（环形缓冲）在窗口外：
    let mut meter = GatedLoudness::new_48k();
    let left = [0.25f32; QUANTUM_FRAMES];
    let right = [-0.25f32; QUANTUM_FRAMES];
    let reading = window(|| {
        for _ in 0..QUANTA {
            meter.add_stereo(&left, &right);
        }
    });
    assert!(
        meter.momentary_lufs().is_finite() && meter.short_term_lufs().is_finite(),
        "窗口跑完之后两个读数必须有限（否则状态被毒化）"
    );
    assert_eq!(reading, SILENT, "门限响度的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 7：过采样往返
// ---------------------------------------------------------------------------

/// 量什么：`Oversampler2x::process_round_trip` 在 **5 000 个量子**里的
/// 分配／释放次数（`Oversampler2x` 的内部状态是定长数组 ⇒ 无构造期分配）。
///
/// 判据：全 `0`；且输出非平凡。
///
/// 注入实测：在 `upsample` 首行插入一次堆分配（第四批 D07）
/// ⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn oversampling_round_trip_allocates_nothing_over_5_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 5_000;
    let mut oversampler = Oversampler2x::new();
    let input = [0.5f32; QUANTUM_FRAMES];
    let mut output = [0.0f32; QUANTUM_FRAMES];
    let mut upsampled = [0.0f32; 2 * QUANTUM_FRAMES];
    let mut scratch = [0.0f32; 4_096];
    let mut checksum = 0.0f32;
    let reading = window(|| {
        for _ in 0..QUANTA {
            oversampler.process_round_trip(&input, &mut output, &mut upsampled, &mut scratch);
            checksum += output[QUANTUM_FRAMES - 1];
        }
    });
    assert!(
        checksum.is_finite() && checksum != 0.0,
        "往返输出恒为 0 ⇒ 本判据测的是空壳"
    );
    assert_eq!(reading, SILENT, "过采样往返的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 8：位深整形（bit crusher）
// ---------------------------------------------------------------------------

/// 量什么：`BitCrusher::process` 在 **5 000 个量子**里的分配／释放次数。
///
/// 判据：全 `0`；且输出非平凡且有限。
///
/// 注入实测：在 `BitCrusher::process` 的帧计数检查之后插入一次堆分配
/// （第四批 D03）⇒ 全库 457 条判据**全绿** ⇒ 本判据变红。
#[test]
fn bit_crusher_allocates_nothing_over_5_000_quanta() {
    /// 观测窗口的量子数。
    const QUANTA: u64 = 5_000;
    let mut crusher = BitCrusher::new();
    let params = CrushParams {
        bits: 8.0,
        down: 4.0,
        aa: 0.5,
    };
    let in_l = [0.5f32; QUANTUM_FRAMES];
    let in_r = [-0.5f32; QUANTUM_FRAMES];
    let mut out_l = [0.0f32; QUANTUM_FRAMES];
    let mut out_r = [0.0f32; QUANTUM_FRAMES];
    let mut checksum = 0.0f32;
    let reading = window(|| {
        for _ in 0..QUANTA {
            crusher.process(&in_l, &in_r, &mut out_l, &mut out_r, params, SAMPLE_RATE);
            // 整块的绝对值和（不是末样本）：位深整形有采样保持与预加重，
            // 单个下标可能是保持期里的 0（第一版取末样本时实测为 0 ⇒ 假红）。
            checksum += sum_abs(&out_l) + sum_abs(&out_r);
        }
    });
    assert!(
        checksum.is_finite() && checksum != 0.0,
        "位深整形输出恒为 0 ⇒ 本判据测的是空壳"
    );
    assert_eq!(reading, SILENT, "位深整形的实时路径分配了内存");
}

// ---------------------------------------------------------------------------
// 判据 9：正对照（计数器有牙）
// ---------------------------------------------------------------------------

/// 量什么：窗口**内**的一次蓄意分配（`Vec::<u8>::with_capacity(1)`）的计数
/// （单位：次数）。
///
/// 判据：`allocations == 1 && deallocations == 1`。它是判据 1–8 的"全 0"的**牙**：
/// 若计数器坏了（恒 `0`），本判据会红。
#[test]
fn the_counter_has_teeth_a_deliberate_allocation_is_counted() {
    let reading = window(|| {
        let probe: Vec<u8> = Vec::with_capacity(1);
        assert_eq!(probe.capacity(), 1);
    });
    assert_eq!(
        reading,
        Reading {
            allocations: 1,
            deallocations: 1,
        },
        "计数器没有牙 ⇒ 其余判据的'全 0'不可信"
    );
}
