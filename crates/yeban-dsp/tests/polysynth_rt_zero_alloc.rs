//! `yeban_dsp::polysynth` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要这个文件（与引擎那条判据的分工）
//!
//! `crates/yeban-engine/tests/synth_rt_zero_alloc.rs` 覆盖的是**引擎的音频路径**
//! （`EngineRuntime::render_block` → `SynthEngine::render_track` → 本器件）。
//! 上移之后那条判据**仍然覆盖**本器件 —— 但它覆盖的是"引擎经由 re-export 调到的那条路"。
//!
//! 本文件把同一套仪器**对准** `yeban_dsp::polysynth::PolySynth` **本身**，理由是
//! `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:111` 把本器件定为 `yeban-dsp`
//! 的公共面：dsp 的任何消费者（含将来的离线母带渲染 `crates/yeban-render`）
//! 都直接调用它，而**不经过** `yeban-engine`。因此"引擎那条路全 0"不构成
//! "本器件自身全 0"的完整证据。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配/释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎的 `rt_probe`（`RtLockProbe` / `diag`）承担，
//!   本 crate **没有**那套探针 ⇒ 本文件对锁与 I/O **不表态**
//!   （"已证明为 0"是**没有**的，只有源码形状：`render`/`note_on` 里没有锁、没有 I/O）。
//!
//! # 仪器口径（与 `limiter_rt_zero_alloc.rs` 相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数。
//! 2. **窗口只包住实时路径**：波表库、参数、缓冲区、音符请求都在窗口**外**准备。
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。若计数器坏了，判据 2 会红 ——
//!    因此判据 1 的"全 0"是**测出来的 0**，不是"没有计数器"。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::oscillator::{HOLLOW, ORGAN};
use yeban_dsp::polysynth::{
    NoteEvent, OscSettings, PolySynth, PolySynthParams, PolySynthTables, VOICES_PER_SLOT,
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

/// 观测窗口的帧数（`128` 与本仓库的固定处理块长同值 [ARCH-DET-001]）。
const WINDOW_FRAMES: usize = 128;
/// 判据 1 的量子数（每量子 128 帧单声道）。
const RT_QUANTA: u64 = 10_000;
/// 每多少个量子触发一个**新音符**（覆盖 `note_on` 的空槽分支）。
const NOTE_EVERY: u64 = 8;
/// 每多少个量子换一次音色参数（覆盖 `set_params` 的器件内分支）。
const PARAMS_EVERY: u64 = 1_000;
/// 每多少个量子换一次**采样率**（覆盖 `set_sample_rate` 的"在响声部重算"分支）。
///
/// 48 kHz ⇄ 96 kHz 交替：每次都要走完整段重算（在响声部的相位增量与 mip 级、
/// 包络系数、滤波器系数，含 `exp` 与 `tan`）⇒ 这条分支是被**逐次走到**的，
/// 不是"从没执行过"的空转。
const RATE_EVERY: u64 = 1_000;
/// 每多少个量子回收一次声部（覆盖 `retire_finished`）。
const RETIRE_EVERY: u64 = 500;
/// 每多少个量子 `seek` 一次（覆盖 `reset`）。
const RESET_EVERY: u64 = 4_000;
/// 每多少个量子塞一个**短音符**（终点 = 当前位置 + 512 帧）⇒ 覆盖回收分支。
const SHORT_NOTE_EVERY: u64 = 2_000;

// ---------------------------------------------------------------------------
// 判据 0：新面（延迟上报）的读数与构造性依据
// ---------------------------------------------------------------------------

/// 量什么：`PolySynth::latency_samples()` 的读数（单位：帧）。
///
/// 判据：恒为 `0`，且与采样率、参数、声部容量、起点都无关（它是编译期常量返回）。
/// 口径依据：[ARCH-PDC-001] 要求每个内置设备精确上报处理延迟；本器件是**声源**，
/// 没有前视缓冲、没有延迟线、没有过采样往返 ⇒ 它引入的延迟是 `0` 帧。
/// 与 `yeban_dsp::reverb` / 压缩器 / 通道条同口径（都上报 `0`）。
#[test]
fn the_reported_latency_is_the_zero_constant() {
    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    for sample_rate in [44_100u32, 48_000, 96_000] {
        let mut synth = PolySynth::<VOICES_PER_SLOT>::new(sample_rate);
        synth.set_params(
            PolySynthParams::new()
                .with_oscillators(OscSettings::new(0, 1.0, 0.0), OscSettings::new(1, 0.5, 7.0))
                .with_filter(1_200.0, 0.3, 0.2, false)
                .with_envelope(0.004, 0.06, 0.6, 0.04),
            &tables,
        );
        assert_eq!(
            synth.latency_samples(),
            0,
            "声源器件不上报延迟 ⇒ 采样率 {sample_rate} Hz 下读数必须是 0"
        );
        synth.note_on(NoteEvent::new(0, 10_000, 440.0, 1.0), &tables);
        assert_eq!(synth.latency_samples(), 0, "有音符在鸣时读数不变");
        synth.set_steal_fade_frames(0);
        assert_eq!(synth.latency_samples(), 0, "硬窃取配置下读数不变");
    }
    // 声部容量是编译期参数 ⇒ 换容量也不改读数。
    assert_eq!(PolySynth::<4>::new(48_000).latency_samples(), 0);
    assert_eq!(PolySynth::<64>::new(48_000).latency_samples(), 0);
    eprintln!(
        "[yeban-dsp/RT] polysynth 延迟上报: latency_samples()=0 帧（与采样率/参数/容量无关）"
    );
}

/// 量什么：把同一音符的起点从 `0` 移到 `P` 之后，两次渲染的**逐位关系**
/// （单位：帧；比较用 `f32::to_bits`），以及起音首个非零帧的**绝对**下标。
///
/// 判据：① 移位版的前 `P` 帧全为 `0.0`；② 第 `P` 帧起与原版**逐位相同**；
/// ③ **绝对锚点**：原版首个非零帧的下标恰为 `1`；④ 两次渲染整体不同
/// （夹具真的出声 ⇒ 这条判据有牙）。
/// ① ② 是"`latency_samples() == 0`"的构造性依据（输出没有任何整体后移）；
/// ③ 是**绝对**刻度。只有 ① ② 时，"把起音整体推后一帧"这类错法与"调用方把起点
/// 写晚一帧"在输出上不可区分 —— 本机实测：注入 `now < start + 1` 时 ① ② 仍为真
///（整段波形一致后移），加上 ③ 才把它变红。
///
/// ⚠ `1` 是本机实测的读数：`HOLLOW` 波表的第 0 个样本是 `0.0`（各次谐波在相位 0
/// 处都是 `0.0`），`Adsr` 处理的第一帧电平也是 `0.0` ⇒ 第 0 帧被处理但输出为 `0.0`，
/// 首个非零样本落在第 1 帧。
#[test]
fn a_delayed_trigger_shifts_the_waveform_by_exactly_the_trigger_offset() {
    /// 起点偏移（帧）。取非零且非块长整数倍的值，避免与 `128` 的块边界混淆。
    const OFFSET: usize = 100;
    /// 渲染帧数（帧）。
    const FRAMES: usize = 1_024;
    /// 起音首个非零样本的**绝对**下标（帧；本机实测，见本条文档）。
    const FIRST_NONZERO: Option<usize> = Some(1);

    let tables = PolySynthTables::from_recipes(&[HOLLOW]);
    let render_at = |start: u64| -> Vec<f32> {
        let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
        synth.set_params(PolySynthParams::new(), &tables);
        synth.note_on(NoteEvent::new(start, 10_000_000, 440.0, 1.0), &tables);
        let mut out = vec![0.0f32; FRAMES];
        synth.render(&tables, 0, &mut out);
        out
    };
    let at_zero = render_at(0);
    let at_offset = render_at(OFFSET as u64);

    assert!(
        at_zero.iter().any(|sample| *sample != 0.0),
        "夹具必须出声（否则下面四条都可能空过）"
    );
    assert!(
        at_offset[..OFFSET].iter().all(|sample| *sample == 0.0),
        "起点之前必须一帧都不出声"
    );
    let aligned = (0..FRAMES - OFFSET).all(|index| at_zero[index] == at_offset[OFFSET + index]);
    assert!(
        aligned,
        "移位后的波形不是原波形整体后移 {OFFSET} 帧 ⇒ 器件引入了额外延迟或提前"
    );
    assert_eq!(
        at_zero.iter().position(|sample| *sample != 0.0),
        FIRST_NONZERO,
        "起音首个非零帧的绝对下标必须是 {FIRST_NONZERO:?} 帧 ⇒ 起音没有被整体推后"
    );
    assert_ne!(
        at_zero.iter().map(|s| s.to_bits()).collect::<Vec<u32>>(),
        at_offset.iter().map(|s| s.to_bits()).collect::<Vec<u32>>(),
        "移动起点居然没改变输出 ⇒ 这条判据没有牙"
    );
    eprintln!(
        "[yeban-dsp/RT] polysynth 起点移位: offset={OFFSET} 帧, 前 {OFFSET} 帧全 0, \
         [ {OFFSET} .. {FRAMES} ) 逐位相同, 首个非零帧={FIRST_NONZERO:?}"
    );
}

/// 量什么：`polysynth` 的**实时路径**在 10 000 个量子（每量子 128 帧单声道）里的
/// **堆分配次数与释放次数**（单位：次数）。
///
/// 实时路径 = `PolySynth::note_on`（含窃取选择与 3 ms 淡出状态机）
/// ＋ `PolySynth::render`（逐样本：整数相位 + 双振荡器 + ADSR + 可选低通）
/// ＋ `retire_finished` / `reset`。
/// 窗口里**同时**夹着 `set_params`（换滤波器/包络系数）与 `set_sample_rate`
/// （换采样率 ⇒ 在响声部的相位增量、mip 级、包络系数与滤波器系数整段重算，
/// 见 [`yeban_dsp::polysynth::PolySynth::set_sample_rate`]）。
///
/// ⚠ **覆盖度陷阱（第一版踩到）**：第一版的音符终点写成 `96_000`，而窗口是
/// 1 280 000 帧 —— 第 750 个量子之后所有音符都已过终点，触发的"新音符"一进门
/// 就进 Release 并立刻归零 ⇒ 池子基本是空的，窗口的后 92% 只跑"静音快路径"。
/// 现在音符终点取 `10_000_000`（远超窗口），并额外每 2 000 个量子塞一个
/// **短音符**（终点 = 当前位置 + 512）来覆盖"过终点 ⇒ 释放 ⇒ 回收"；
/// 读数里还加了**非零样本数**的覆盖度自检。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（波表库、参数、缓冲区、音符请求）----
    let tables = PolySynthTables::from_recipes(&[HOLLOW, ORGAN]);
    let mut synth = PolySynth::<VOICES_PER_SLOT>::new(48_000);
    synth.set_params(PolySynthParams::new(), &tables);
    let single = PolySynthParams::new();
    let dual = PolySynthParams::new()
        .with_oscillators(OscSettings::new(0, 1.0, 0.0), OscSettings::new(1, 0.5, 7.0))
        .with_filter(1_200.0, 0.3, 0.2, false)
        .with_envelope(0.004, 0.06, 0.6, 0.04);
    let mut block = vec![0.0f32; WINDOW_FRAMES];

    // 音符请求也在窗口外建好（`NoteEvent` 是 `Copy`，窗口内只是赋值）。
    // 终点远超窗口 ⇒ 声部在整个窗口里持续发声、持续触发窃取。
    let events: Vec<NoteEvent> = (0..32u64)
        .map(|index| {
            let freq = 110.0 + index as f32 * 7.0;
            NoteEvent::new(index * 64, 10_000_000, freq, 1.0)
        })
        .collect();

    // ---- 观测窗口 ----
    let mut position: u64 = 0;
    let mut triggered: u64 = 0;
    let mut rendered_frames: u64 = 0;
    let mut nonzero_frames: u64 = 0;
    // 新面（`latency_samples`）的运行期零分配判据：在**同一个观测窗口**内每个量子
    // 取一次读数，并把读数累加 —— 分配计数因此也覆盖这个新成员。
    let mut latency_calls: u64 = 0;
    let mut latency_sum: usize = 0;
    // 窗口里换过多少次采样率（覆盖度自检用）。
    let mut rate_changes: u64 = 0;
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            latency_calls += 1;
            latency_sum += synth.latency_samples();
            if quantum % NOTE_EVERY == 0 {
                // 循环取音符 ⇒ 反复撞满声部池 ⇒ 窃取与淡出状态机每个量子窗口都被走到。
                let event = events[(triggered as usize) % events.len()];
                synth.note_on(event, &tables);
                triggered += 1;
            }
            if quantum % SHORT_NOTE_EVERY == 0 {
                // 短音符：512 帧之后过终点 ⇒ 覆盖"释放 ⇒ 回收"。
                synth.note_on(
                    NoteEvent::new(position, position + 512, 3_000.0, 0.5),
                    &tables,
                );
                triggered += 1;
            }
            if quantum % RETIRE_EVERY == 0 {
                synth.retire_finished(position);
            }
            if quantum % PARAMS_EVERY == 0 {
                synth.set_params(
                    if (quantum / PARAMS_EVERY).is_multiple_of(2) {
                        single
                    } else {
                        dual
                    },
                    &tables,
                );
            }
            if quantum % RATE_EVERY == 0 {
                // 换采样率：在响声部（含被窃取声部挂起的新音符）的重算整段都在窗口内
                // ⇒ 这条分支的零分配是被测的，不是假定的。
                rate_changes += 1;
                synth.set_sample_rate(if (quantum / RATE_EVERY).is_multiple_of(2) {
                    48_000
                } else {
                    96_000
                });
            }
            if quantum % RESET_EVERY == 0 {
                synth.reset();
            }
            synth.render(&tables, position, &mut block);
            rendered_frames += WINDOW_FRAMES as u64;
            nonzero_frames += block.iter().filter(|sample| **sample != 0.0).count() as u64;
            position += WINDOW_FRAMES as u64;
        }
    });

    // 覆盖度自检：窗口里真的在触发、真的在渲染、真的窃取过、
    // 而且**大部分帧真的有声音**（否则"全 0"可能就是"什么都没跑"）。
    assert_eq!(
        rendered_frames,
        RT_QUANTA * WINDOW_FRAMES as u64,
        "量子记账：每量子 {WINDOW_FRAMES} 帧"
    );
    assert_eq!(synth.notes_triggered(), triggered, "触发计数必须对得上");
    assert!(
        synth.voice_steals() > 0,
        "窗口里从未窃取 ⇒ 窃取/淡出分支没被覆盖（假绿）"
    );
    assert!(
        nonzero_frames * 10 > rendered_frames * 9,
        "有声音的帧只占 {nonzero_frames}/{rendered_frames} —— 窗口大部分是静音快路径（假绿）"
    );
    assert!(
        block.iter().all(|sample| sample.is_finite()),
        "路径产出了非有限值"
    );
    assert_eq!(
        latency_calls, RT_QUANTA,
        "延迟读数必须在窗口里的每个量子都被取一次"
    );
    assert_eq!(latency_sum, 0, "窗口里累加的延迟读数必须恒为 0 帧");
    assert_eq!(
        rate_changes,
        RT_QUANTA / RATE_EVERY,
        "换采样率分支必须在窗口里被逐次走到（否则这一段是空转）"
    );
    eprintln!(
        "[yeban-dsp/RT] polysynth 10 000 量子 × {WINDOW_FRAMES} 帧: \
         allocations={} deallocations={} rendered_frames={} nonzero_frames={} \
         triggered={} steals={} latency_calls={latency_calls} latency_sum={latency_sum} \
         rate_changes={rate_changes}",
        reading.allocations,
        reading.deallocations,
        rendered_frames,
        nonzero_frames,
        triggered,
        synth.voice_steals()
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

/// 量什么：**波表库必须建在窗口外**。在窗口内调
/// `PolySynthTables::from_recipes` 一定会分配（判据 1 的"零分配"因此**要求**
/// 调用方把建库放在打开设备之前）。
///
/// 判据：窗口内的分配次数 > 0。这条是**反面对照** —— 它证明判据 1 的零分配
/// 不是"因为分配器看不见分配"，而是因为那条路径真的不分配。
#[test]
fn building_the_table_bank_inside_the_window_would_allocate() {
    let reading = window(|| {
        let tables = PolySynthTables::from_recipes(&[HOLLOW]);
        assert_eq!(tables.len(), 1);
    });
    eprintln!(
        "[yeban-dsp/RT] 窗口内建一张波表: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "建波表居然没分配 ⇒ 本判据的仪表口径需要重看"
    );
}
