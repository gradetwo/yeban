//! `yeban_dsp::convolution` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要一个独立文件
//!
//! `convolution` 是**新写**的模块，引擎**还没有**接线（核实方式：`docs/ledger/
//! integration-rulings-notes.md:88` 的"未接线模块名"口径 + 本票复核）。因此
//! `crates/yeban-engine/tests/rt_zero_alloc.rs` 那条判据**根本走不到**本器件，
//! 它的"全 0"不构成这里的证据。本文件把同一套仪器**对准** `convolution` 本身。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配 / 堆释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎的 `rt_probe`（`RtLockProbe` / `diag`）承担，本 crate
//!   **没有**那套探针 ⇒ 本文件对锁与 I/O **不表态**（不是"已证明为 0"）。
//!   `process` / `reset` 的源码形状里没有锁、没有 I/O，但那是**源码形状**的证据，
//!   不是运行期读数。
//!
//! # 仪器口径（与 `limiter_rt_zero_alloc.rs` 的三处关键决定相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：`set_impulse_response`（唯一分配入口）与缓冲区准备
//!    都在窗口**外**做；
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::convolution::{
    CONV_BINS, CONV_BLOCK_FRAMES, CONV_FFT_FRAMES, CONV_LATENCY, CONV_MAX_IR_FRAMES, Convolution,
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

/// 立体声观测窗口的帧数（与 [`CONV_BLOCK_FRAMES`] 同值，即引擎的固定块长）。
const WINDOW_FRAMES: usize = CONV_BLOCK_FRAMES;
/// 判据 1 的量子数（每量子 128 帧 × 2 声道）。
const RT_QUANTA: u64 = 10_000;
/// 判据 1 里每多少个量子 `reset()` 一次（覆盖 `reset`）。
const RESET_EVERY: u64 = 4_000;
/// 判据 1 的 IR 长度（帧）：`1_024 / 128 = 8` 个分区 —— 足够覆盖"环形索引 + OLA"，
/// 又不至于让 debug 档的 10 000 量子跑成分钟级。
const IR_FRAMES: usize = 1_024;

/// 确定性重填一个块（不用分配，也不用 RNG 对象）。
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

/// 量什么：`convolution` 的公共常量（单位：帧 / bin / 个）。
///
/// 判据：与"分块卷积的解析定义"一致。这些常量是**代价契约**的一部分：
/// `2·128 = 256` 点变换、`129` 个一侧谱 bin、延迟 `0`、上限 `10 s @48 kHz`。
/// 改动其中任何一个都会改变内存与 CPU 成本，因此在这里钉死。
#[test]
fn public_constants_pin_the_cost_contract() {
    assert_eq!(CONV_BLOCK_FRAMES, 128, "块长必须是引擎的固定量子");
    assert_eq!(
        CONV_FFT_FRAMES, 256,
        "两个 128 点序列的线性卷积需要 256 点变换"
    );
    assert_eq!(CONV_BINS, 129, "一侧谱 bin 数 = 256/2 + 1");
    assert_eq!(CONV_LATENCY, 0, "UPOLA 的输出块与输入块对齐 ⇒ 零延迟");
    assert_eq!(CONV_MAX_IR_FRAMES, 480_000, "10 s @48 kHz");
    eprintln!(
        "[yeban-dsp/RT] convolution 常量: block={CONV_BLOCK_FRAMES} fft={CONV_FFT_FRAMES} \
         bins={CONV_BINS} latency={CONV_LATENCY} max_ir={CONV_MAX_IR_FRAMES}"
    );
}

// ---------------------------------------------------------------------------
// 判据 1：实时路径零分配
// ---------------------------------------------------------------------------

/// 量什么：`convolution` 的逐块路径在 **10 000 个量子**里的**堆分配次数与堆释放次数**
/// （单位：次数）。
///
/// 窗口里跑的是：两条真实立体声通路（`Convolution` × 2，各带一条 `1_024` 帧的 IR）
/// 的 `process`，每 [`RESET_EVERY`] 个量子夹一次 `reset()`。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（IR、构造、缓冲区）----
    let ir_left = make_ir(IR_FRAMES, 0.0);
    let ir_right = make_ir(IR_FRAMES, 1.0);
    let mut left_conv = Convolution::new();
    let mut right_conv = Convolution::new();
    assert_eq!(left_conv.set_impulse_response(&ir_left), IR_FRAMES);
    assert_eq!(right_conv.set_impulse_response(&ir_right), IR_FRAMES);
    assert_eq!(left_conv.partitions(), IR_FRAMES / CONV_BLOCK_FRAMES);

    let mut left = vec![0.0f32; WINDOW_FRAMES];
    let mut right = vec![0.0f32; WINDOW_FRAMES];

    // ---- 观测窗口 ----
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            refill(&mut left, quantum, 0.6);
            refill(&mut right, quantum, 0.5);
            left_conv.process(&mut left);
            right_conv.process(&mut right);
            if quantum % RESET_EVERY == 0 {
                left_conv.reset();
                right_conv.reset();
            }
        }
    });

    // ---- 覆盖度自检（否则"全 0"可能是"什么都没跑"）----
    let last = RT_QUANTA - 1;
    let mut expected = vec![0.0f32; WINDOW_FRAMES];
    refill(&mut expected, last, 0.6);
    let changed = left
        .iter()
        .zip(&expected)
        .filter(|(out, inp)| (*out - *inp).abs() > 1e-4)
        .count();
    assert!(
        changed >= WINDOW_FRAMES / 2,
        "窗口最后一块的输出与输入太像（{changed}/{WINDOW_FRAMES} 个样本改变）⇒ \
         卷积核没真的参与运算，本判据测的是空壳"
    );
    assert!(
        left.iter().chain(right.iter()).all(|v| v.is_finite()),
        "路径产出了非有限值"
    );
    let peak = left
        .iter()
        .chain(right.iter())
        .fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "输出全零 ⇒ 判据测错了对象");

    eprintln!(
        "[yeban-dsp/RT] convolution {RT_QUANTA} 量子 × ({WINDOW_FRAMES} 帧立体声，\
         IR={IR_FRAMES} 帧 = {} 个分区，每 {RESET_EVERY} 量子 reset 一次): \
         allocations={} deallocations={}",
        IR_FRAMES / CONV_BLOCK_FRAMES,
        reading.allocations,
        reading.deallocations
    );
    eprintln!(
        "[yeban-dsp/RT] 覆盖度读数: 最后一块改变样本={changed}/{WINDOW_FRAMES} peak={peak:.6} \
         ir_frames={} partitions={}",
        left_conv.ir_frames(),
        left_conv.partitions()
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

// ---------------------------------------------------------------------------
// 判据 4 / 5：`set_impulse_response` 的分配边界
// ---------------------------------------------------------------------------

/// 量什么：**同长度**换 IR + 一次 `process`，在窗口里的分配/释放读数（单位：次数）。
///
/// 为什么单列一条：这是本器件**唯一**的分配入口，接线的第一问就是"换 IR 能不能在
/// 音频线程上做"。实测答案是"**同长度**可以"（缓冲区已够大 ⇒ 只 `fill`，不 `vec!`）。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn swapping_an_ir_of_the_same_length_allocates_nothing() {
    let ir_a = make_ir(IR_FRAMES, 0.0);
    let ir_b = make_ir(IR_FRAMES, 2.0);
    let mut conv = Convolution::new();
    conv.set_impulse_response(&ir_a); // 窗口外：建容量
    let mut block = vec![0.0f32; WINDOW_FRAMES];

    let reading = window(|| {
        conv.set_impulse_response(&ir_b);
        refill(&mut block, 7, 0.5);
        conv.process(&mut block);
    });

    eprintln!(
        "[yeban-dsp/RT] convolution 同长度换 IR（{IR_FRAMES} 帧）+ process: \
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
    let mut conv = Convolution::new();
    conv.set_impulse_response(&long);

    let reading = window(|| {
        conv.set_impulse_response(&short);
    });

    eprintln!(
        "[yeban-dsp/RT] convolution 改长度换 IR（{IR_FRAMES} → {CONV_BLOCK_FRAMES} 帧）: \
         allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "改长度换 IR 居然零分配 ⇒ 上一条判据的反对照失效（本器件可能整体不分配）"
    );
}

// ---------------------------------------------------------------------------
// 判据 6：IR 的**取值校验**也是零分配的，而且拒绝后不留下毒
// ---------------------------------------------------------------------------

/// 量什么：**同长度**换入一条含 `NaN` 的 IR（即"校验并拒绝"这条路）加一次 `process`，
/// 在窗口里的分配/释放读数（单位：次数）。
///
/// 为什么单列一条：[`Convolution::set_impulse_response`] 现在多了一道取值校验
/// （IR 的分区频谱必须逐项有限）。这道校验跑在**换 IR** 这条路上，而实测已经证明
/// "同长度换 IR"在音频线程上零分配 —— 于是校验不能自己引入分配
/// （例如 `iter().copied().collect::<Vec<_>>()` 那种写法就会）。
/// 判据把"校验零分配"与"拒绝的后果"一起钉住：
///
/// 1. `allocations == 0 && deallocations == 0`；
/// 2. 拒绝之后 `is_configured()` 为假，且同一块**逐位**直通（没有 `NaN` 泄漏）。
///
/// 注入（实测见报告）：把 `set_impulse_response` 的有限性扫描写成
/// `let probe = self.ir_re[..span].to_vec();` ⇒ 本判据在分配断言处变红。
#[test]
fn rejecting_a_non_finite_ir_of_the_same_length_allocates_nothing() {
    let good = make_ir(IR_FRAMES, 0.0);
    let mut bad = make_ir(IR_FRAMES, 4.0);
    bad[5] = f32::NAN;
    let mut conv = Convolution::new();
    assert_eq!(conv.set_impulse_response(&good), IR_FRAMES);

    let mut block = vec![0.0f32; WINDOW_FRAMES];
    refill(&mut block, 13, 0.5);
    let expected = block.clone();

    let reading = window(|| {
        assert_eq!(
            conv.set_impulse_response(&bad),
            0,
            "含 NaN 的同长度 IR 必须被拒绝"
        );
        conv.process(&mut block);
    });

    assert!(!conv.is_configured(), "拒绝之后必须回到未配置");
    assert_eq!(conv.partitions(), 0, "拒绝之后不得留下分区");
    let non_finite = block.iter().filter(|v| !v.is_finite()).count();
    for (i, (out, want)) in block.iter().zip(&expected).enumerate() {
        assert_eq!(
            out.to_bits(),
            want.to_bits(),
            "样本 {i}: 拒绝之后必须逐位直通（非有限样本 {non_finite} 个）"
        );
    }

    eprintln!(
        "[yeban-dsp/RT] convolution 同长度换入含 NaN 的 IR（{IR_FRAMES} 帧）+ process: \
         allocations={} deallocations={} 非有限样本={non_finite}",
        reading.allocations, reading.deallocations
    );
    assert_eq!(reading.allocations, 0, "IR 的取值校验不该分配");
    assert_eq!(reading.deallocations, 0, "IR 的取值校验不该释放");
}
