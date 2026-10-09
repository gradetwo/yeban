//! `yeban_dsp::reverb` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要一个独立文件
//!
//! Freeverb 是本 crate 里**最早**接进引擎音频路径的器件之一，但它此前**没有**
//! 本 crate 级的运行期零分配判据：`tests/` 下按器件一份（`compressor` /
//! `channel_strip` / `limiter` / `polysynth` / `drums` / `convolution*`），
//! 只有 `reverb` 缺。
//!
//! 引擎侧**有**一条覆盖（`crates/yeban-engine/tests/synth_rt_zero_alloc.rs`
//! 的场景 9/10 把混响插在轨上跑在同一个观测窗口里），但那条判据的三个分量都不是
//! 本器件能独占的：
//!
//! 1. 它的窗口是**引擎**的（同一条窗口里还有合成器、通道条、混音、计量），
//!    器件级回归只能定位到"引擎里有东西分配了"；
//! 2. 它**不调用** [`Reverb::reset`] —— `reset` 是本次新增的器件成员，引擎侧
//!    目前没有任何调用点（核实方式：`grep -rn 'reset' crates/yeban-engine/src/rt.rs`
//!    里没有对混响的 `reset`）；
//! 3. 本 crate 的测试不能依赖另一个 crate 的测试文件存在。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配 / 堆释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎的 `rt_probe` 承担，本 crate **没有**那套探针 ⇒
//!   本文件对锁与 I/O **不表态**（不是"已证明为 0"）。
//!
//! # 仪器口径（与 `convolution_reverb_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：`set_sample_rate`（唯一的分配入口）与缓冲区准备都在
//!    窗口**外**做；
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::reverb::{REVERB_LATENCY, Reverb, ReverbParams};

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
// 判据规模与夹具（改动这些常量就是改动判据强度）
// ---------------------------------------------------------------------------

/// 观测窗口的帧数（每个量子）：引擎的固定块长。
const QUANTUM_FRAMES: usize = 128;
/// 判据 1 的量子数（每量子 `128` 帧 × 2 声道）。
const RT_QUANTA: u64 = 5_000;
/// 判据 1 里每多少个量子 `set_params` 一次（覆盖逐块调参）。
const RETUNE_EVERY: u64 = 250;
/// 判据 1 里每多少个量子 `reset()` 一次（覆盖新增的器件成员）。
const RESET_EVERY: u64 = 2_000;
/// 判据用的采样率（Hz）。
const SAMPLE_RATE: f32 = 48_000.0;

/// 一台**已配置**的混响（唯一的分配入口 `set_sample_rate` 在窗口之外调用）。
fn configured_reverb() -> Reverb {
    let mut verb = Reverb::new();
    verb.set_sample_rate(SAMPLE_RATE);
    verb.set_params(ReverbParams {
        size: 0.7,
        damp: 0.3,
        mix: 0.6,
        width: 0.8,
        predelay: 0.02,
    });
    verb
}

/// 确定性重填一对声道块（不用分配，也不用 RNG 对象）。
fn refill(left: &mut [f32], right: &mut [f32], quantum: u64, scale: f32) {
    for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
        let t = i as f32 * 0.013;
        *l = scale * (quantum as f32 * 0.37 + t).sin();
        *r = scale * (quantum as f32 * 0.29 - t).cos();
    }
}

// ---------------------------------------------------------------------------
// 判据 0：公共常量（延迟上报口径）
// ---------------------------------------------------------------------------

/// 量什么：`reverb` 的公共延迟常量（单位：帧）。
///
/// 判据：`REVERB_LATENCY == 0`，且与任何采样率无关（它是编译期常量）。
/// 口径依据：[ARCH-PDC-001] 要求每个内置设备精确上报处理延迟；Freeverb 的预延迟
/// 只推迟湿路 ⇒ 净延迟是 `0`。
#[test]
fn the_reported_latency_is_the_zero_constant() {
    assert_eq!(REVERB_LATENCY, 0, "Freeverb 的预延迟不是 PDC 延迟");
    let mut verb = configured_reverb();
    assert_eq!(verb.latency_samples(), REVERB_LATENCY);
    // 换采样率与拖预延迟都不得改变读数。
    for sample_rate in [44_100.0f32, 96_000.0] {
        verb.set_sample_rate(sample_rate);
        assert_eq!(verb.latency_samples(), 0);
    }
    for predelay in [0.0f32, 0.05, 0.1] {
        verb.set_params(ReverbParams {
            predelay,
            ..Default::default()
        });
        assert_eq!(verb.latency_samples(), 0, "预延迟 {predelay} s");
    }
    eprintln!(
        "[yeban-dsp/RT] reverb 延迟上报: REVERB_LATENCY={REVERB_LATENCY} 帧（与采样率、\
         预延迟无关）"
    );
}

// ---------------------------------------------------------------------------
// 判据 1：实时路径零分配（含逐块调参与 reset）
// ---------------------------------------------------------------------------

/// 量什么：`reverb` 的**逐块路径**在 **5 000 个量子**里的堆分配次数与堆释放次数
/// （单位：次数）。每个量子是 `128` 帧 × `2` 声道。
///
/// 窗口里跑的是：一台已配置混响的 `process`，每 [`RETUNE_EVERY`] 个量子
/// `set_params`（`mix` / `size` / `predelay` 三个旋钮一起动），每 [`RESET_EVERY`]
/// 个量子 `reset()` 一次。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_5_000_quanta() {
    let mut verb = configured_reverb();
    let mut left = vec![0.0f32; QUANTUM_FRAMES];
    let mut right = vec![0.0f32; QUANTUM_FRAMES];

    // ---- 观测窗口 ----
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            refill(&mut left, &mut right, quantum, 0.6);
            verb.process(&mut left, &mut right);
            if quantum % RETUNE_EVERY == 0 {
                let phase = (quantum / RETUNE_EVERY).is_multiple_of(2);
                verb.set_params(ReverbParams {
                    size: if phase { 0.2 } else { 0.95 },
                    damp: if phase { 0.0 } else { 1.0 },
                    mix: 0.6,
                    width: 0.8,
                    predelay: if phase { 0.0 } else { 0.1 },
                });
            }
            if quantum % RESET_EVERY == 0 {
                verb.reset();
            }
        }
    });

    // ---- 覆盖度自检（否则"全 0"可能是"什么都没跑"）----
    let last = RT_QUANTA - 1;
    let mut expected_left = vec![0.0f32; QUANTUM_FRAMES];
    let mut expected_right = vec![0.0f32; QUANTUM_FRAMES];
    refill(&mut expected_left, &mut expected_right, last, 0.6);
    let changed = left
        .iter()
        .zip(&expected_left)
        .filter(|(out, inp)| (*out - *inp).abs() > 1e-4)
        .count();
    assert!(
        changed > 0,
        "窗口最后一块的输出与输入逐样本相同 ⇒ 混响没真的参与运算，本判据测的是空壳"
    );
    assert!(left.iter().all(|v| v.is_finite()), "路径产出了非有限值");
    assert!(right.iter().all(|v| v.is_finite()));
    let peak = left
        .iter()
        .chain(right.iter())
        .fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "输出全零 ⇒ 判据测错了对象");

    eprintln!(
        "[yeban-dsp/RT] reverb {RT_QUANTA} 量子 × ({QUANTUM_FRAMES} 帧立体声，每 \
         {RETUNE_EVERY} 量子换参数、每 {RESET_EVERY} 量子 reset 一次): \
         allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    eprintln!(
        "[yeban-dsp/RT] 覆盖度读数: 最后一块改变样本={changed}/{QUANTUM_FRAMES} peak={peak:.6}"
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
// 判据 3：分配入口的反对照
// ---------------------------------------------------------------------------

/// 量什么：在窗口里从零建一台混响（`new` ＋ `set_sample_rate`）的分配/释放读数
/// （单位：次数）。
///
/// 判据：`allocations > 0`。这条证明 `set_sample_rate` **真的**在预分配
/// `24` 条延迟线（16 条梳状 ＋ 8 条全通，外加两条预延迟线）—— 所以判据 1 的
/// "全 0"不是"这个器件根本不分配"的假象，也说明它**必须**在音频回调之外调用。
#[test]
fn building_the_delay_lines_inside_the_window_would_allocate() {
    let reading = window(|| {
        let mut fresh = Reverb::new();
        fresh.set_sample_rate(SAMPLE_RATE);
        assert!(fresh.is_configured());
    });
    eprintln!(
        "[yeban-dsp/RT] reverb 窗口内 `new` + `set_sample_rate`: allocations={} \
         deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "延迟线居然零分配 ⇒ 判据 1 的反对照失效"
    );
}

// ---------------------------------------------------------------------------
// 判据 4：参数跨度覆盖整段预延迟（含退化值）
// ---------------------------------------------------------------------------

/// 量什么：预延迟从 `0.0` 扫到 `0.1 s`（`101` 步）时，`set_params` ＋ `process` ＋
/// `reset` 的分配读数（单位：次数）；随后再喂一组**非有限**参数（`NaN` / `±inf`），
/// 读数同样要为零。
///
/// 判据：两段都 `allocations == 0 && deallocations == 0`，且输出逐样本有限
/// （退化参数不得毒化总线）。
#[test]
fn retuning_across_the_whole_predelay_range_allocates_nothing() {
    let mut verb = configured_reverb();
    let mut left = vec![0.0f32; QUANTUM_FRAMES];
    let mut right = vec![0.0f32; QUANTUM_FRAMES];
    refill(&mut left, &mut right, 0, 0.5);

    let mut finite = true;
    let reading = window(|| {
        for step in 0..=100u32 {
            let predelay = 0.1 * step as f32 / 100.0;
            verb.set_params(ReverbParams {
                size: 0.7,
                damp: 0.3,
                mix: 0.6,
                width: 0.8,
                predelay,
            });
            verb.process(&mut left, &mut right);
            verb.reset();
        }
        // 退化参数：非有限值一律回落到默认值，处理仍然不得分配、不得产出 NaN。
        verb.set_params(ReverbParams {
            size: f32::NAN,
            damp: f32::INFINITY,
            mix: 0.5,
            width: f32::NAN,
            predelay: f32::NAN,
        });
        verb.process(&mut left, &mut right);
        verb.reset();
        finite = left.iter().all(|v| v.is_finite()) && right.iter().all(|v| v.is_finite());
    });

    eprintln!(
        "[yeban-dsp/RT] reverb 预延迟全程扫描（0.0 → 0.1 s，101 步）＋ 一组非有限参数: \
         allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(finite, "退化参数毒化了混响总线");
    assert_eq!(reading.allocations, 0, "扫预延迟不该分配");
    assert_eq!(reading.deallocations, 0, "扫预延迟不该释放");
}
