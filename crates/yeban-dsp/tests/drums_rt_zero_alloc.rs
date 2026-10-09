//! `yeban_dsp::drums` 的**运行期零分配判据**（[ARCH-RT-001] / `MUST-GATE-001`）。
//!
//! # 为什么需要这个文件
//!
//! 鼓机在**音频线程**被触发（引擎侧的轨道槽在走带时逐块调 [`DrumMachine::trigger`]
//! 与 [`DrumMachine::render`]）⇒ [ARCH-RT-001] 的"零分配 / 零释放 / 零锁 /
//! 零阻塞 I/O / 零日志"是**最高约束**，不调整、不放宽。
//!
//! `yeban-dsp` 的公共面会被**不经过 `yeban-engine`** 的消费者直接调用
//! （例：离线母带渲染 `crates/yeban-render`），因此"引擎那条路全 0"不构成
//! "本器件自身全 0"的完整证据。本文件把仪器直接对准本器件。
//!
//! ⚠ 覆盖范围（明说，不暗示）：
//!
//! - 本文件**只**覆盖"堆分配 / 堆释放"两个分量；
//! - 锁与阻塞 I/O 的分量由引擎侧的 `rt_probe`（`RtLockProbe` / `diag`）承担，
//!   本 crate **没有**那套探针 ⇒ 本文件对锁与 I/O **不表态**
//!   （"已证明为 0"是**没有**的，只有源码形状：`trigger` / `render` /
//!   `set_params` 里没有锁、没有 I/O、没有日志）；
//! - 同一个窗口里也走了 `retire_finished` / `reset` / `set_params` /
//!   `set_sample_rate`（它们在音频线程的快照边界上被调用）。
//!
//! # 仪器口径（与 `polysynth_rt_zero_alloc.rs` / `limiter_rt_zero_alloc.rs` 相同）
//!
//! 1. **按线程武装**：`thread_local!` + `const` 初始化，只有本判据自己的线程计数；
//! 2. **窗口只包住实时路径**：鼓组参数、事件列表、输出缓冲都在窗口**外**准备；
//! 3. **判据有牙（正对照）**：判据 2 在窗口**内**做一次 `Vec::<u8>::with_capacity(1)`，
//!    要求 `allocations == 1 && deallocations == 1`。计数器坏了 ⇒ 判据 2 红
//!    ⇒ 判据 1 的"全 0"是**测出来的 0**，不是"没有计数器"；
//! 4. **反面对照**：判据 4 / 5 在窗口内做两条**真实会发生的**分配
//!    （"每个回调现开一块输出缓冲"与"每个回调现攒一份事件列表"）——
//!    它们是本器件在音频线程上最可能的两种误用 ⇒ 计数器必须抓到。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use yeban_dsp::drums::{DRUM_SLOTS, DrumHit, DrumKitParams, DrumMachine, DrumVoice};

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
/// 每多少个量子换一次鼓组参数（覆盖 `set_params` 的器件内分支）。
const PARAMS_EVERY: u64 = 1_000;
/// 每多少个量子回收一次槽位（覆盖 `retire_finished`）。
const RETIRE_EVERY: u64 = 500;
/// 每多少个量子 `reset` 一次（覆盖"池回到空"的路径）。
const RESET_EVERY: u64 = 4_000;
/// 每多少个量子换一次采样率（覆盖 `set_sample_rate` 的系数重算路径）。
const RATE_EVERY: u64 = 3_000;

// ---------------------------------------------------------------------------
// 判据 0：新面（延迟上报）的读数与构造性依据
// ---------------------------------------------------------------------------

/// 量什么：`DrumMachine::latency_samples()` 的读数（单位：帧）。
///
/// 判据：恒为 `0`，且与采样率、鼓组参数、槽位容量、触发都无关
/// （它是编译期常量返回）。
/// 口径依据：[ARCH-PDC-001] 要求每个内置设备精确上报处理延迟；本器件是**声源**，
/// 没有前视缓冲、没有延迟线、没有过采样往返 ⇒ 它引入的延迟是 `0` 帧。
/// 与 `yeban_dsp::reverb` / 压缩器 / 通道条同口径（都上报 `0`）。
#[test]
fn the_reported_latency_is_the_zero_constant() {
    for sample_rate in [44_100u32, 48_000, 96_000] {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(sample_rate);
        assert_eq!(
            machine.latency_samples(),
            0,
            "声源器件不上报延迟 ⇒ 采样率 {sample_rate} Hz 下读数必须是 0"
        );
        machine.set_params(DrumKitParams::DEFAULT);
        machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
        let mut block = vec![0.0f32; 64];
        machine.render(0, &mut block);
        assert_eq!(machine.latency_samples(), 0, "有鼓击在响时读数不变");
        machine.set_sample_rate(96_000);
        assert_eq!(machine.latency_samples(), 0, "换采样率后读数不变");
        machine.set_steal_fade_frames(0);
        assert_eq!(machine.latency_samples(), 0, "硬窃取配置下读数不变");
    }
    // 槽位容量是编译期参数 ⇒ 换容量也不改读数。
    assert_eq!(DrumMachine::<4>::new(48_000).latency_samples(), 0);
    assert_eq!(DrumMachine::<64>::new(48_000).latency_samples(), 0);
    eprintln!("[yeban-dsp/RT] drums 延迟上报: latency_samples()=0 帧（与采样率/参数/容量无关）");
}

/// 量什么：把同一击的起点从 `0` 移到 `P` 之后，两次渲染的**逐位关系**
/// （单位：帧；比较用 `f32::to_bits`），以及发声首帧的**绝对**读数。
///
/// 判据：① 移位版的前 `P` 帧全为 `0.0`；② 第 `P` 帧起与原版**逐位相同**；
/// ③ **绝对锚点**：起点击在第 0 帧时，第 0 帧就已经有非零输出；④ 两次渲染
/// 整体不同（夹具真的出声 ⇒ 这条判据有牙）。
/// ① ② 是"`latency_samples() == 0`"的构造性依据（输出没有任何整体后移）；
/// ③ 是**绝对**刻度。只有 ① ② 时，"把起音整体推后一帧"这类错法与"调用方把起点
/// 写晚一帧"在输出上不可区分 —— 本机实测：注入 `now < start + 1` 时 ① ② 仍为真
///（整段波形一致后移），加上 ③ 才把它变红。
///
/// ⚠ 夹具用**闭镲**：它的发生器在第 0 帧就给出非零样本（本机实测
/// `out[0] = 0.0029026011`），因此"第 0 帧有没有声音"是可直接读的绝对刻度。
/// （底鼓的第 0 帧恰为 `0.0`：正弦在相位 0、包络首步也是 `0.0` ⇒ 它读不出这个刻度。）
#[test]
fn a_delayed_hit_shifts_the_waveform_by_exactly_the_hit_offset() {
    /// 起点偏移（帧）。取非零且非块长整数倍的值，避免与 `128` 的块边界混淆。
    const OFFSET: usize = 100;
    /// 渲染帧数（帧）。
    const FRAMES: usize = 1_024;

    let render_at = |start: u64| -> Vec<f32> {
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        machine.set_params(DrumKitParams::DEFAULT);
        machine.trigger(DrumHit::new(DrumVoice::ClosedHat, start, 1.0));
        let mut out = vec![0.0f32; FRAMES];
        machine.render(0, &mut out);
        out
    };
    let at_zero = render_at(0);
    let at_offset = render_at(OFFSET as u64);

    assert!(
        at_zero.iter().any(|sample| *sample != 0.0),
        "夹具必须出声（否则下面四条都可能空过）"
    );
    assert_ne!(
        at_zero[0], 0.0,
        "起点落在第 0 帧时，第 0 帧就必须有非零输出 ⇒ 触发帧就是发声首帧"
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
    assert_ne!(
        at_zero.iter().map(|s| s.to_bits()).collect::<Vec<u32>>(),
        at_offset.iter().map(|s| s.to_bits()).collect::<Vec<u32>>(),
        "移动起点居然没改变输出 ⇒ 这条判据没有牙"
    );
    eprintln!(
        "[yeban-dsp/RT] drums 起点移位: offset={OFFSET} 帧, 前 {OFFSET} 帧全 0, \
         [ {OFFSET} .. {FRAMES} ) 逐位相同, out[0]={}",
        at_zero[0]
    );
}

/// 量什么：`drums` 的**实时路径**在 10 000 个量子（每量子 128 帧单声道）里的
/// **堆分配次数与释放次数**（单位：次数）。
///
/// 实时路径 = [`DrumMachine::trigger`]（含确定性窃取选择、3 ms 淡出状态机、
/// 闭镲 choke 开镲）＋ [`DrumMachine::render`]（逐样本：整数相位、正弦、
/// 6 条方波、带通、`Adsr`、白噪声）＋ `retire_finished` / `reset`。
/// 窗口里**同时**夹着 `set_params` 与 `set_sample_rate`。
///
/// ## 覆盖度陷阱（`polysynth` 的第一版踩过，本判据必须避开）
///
/// `polysynth` 的第一版零分配判据把音符终点写死在窗口内 ⇒ 窗口后段
/// 92% 只跑"静音快路径" ⇒ 判据在**空转**。本判据的覆盖度自检有五条：
///
/// 1. **有声帧占比 ≥ 90%** —— 窗口里真的在发声；
/// 2. **5 个音色都真的占用过槽位**（逐量子读 [`DrumMachine::debug_slot`]，
///    把出现过的音色判别值记进 `[bool; 5]`）；
/// 3. `triggers()` 等于夹具自己的记账；
/// 4. `voice_steals() > 0` —— 窃取/淡出分支真的被走到；
/// 5. `hat_chokes() > 0` —— 闭镲 choke 开镲那条分支真的被走到。
///
/// 判据：`allocations == 0 && deallocations == 0`。
#[test]
fn rt_path_allocates_nothing_over_10_000_quanta() {
    // ---- 窗口**外**的准备（参数、事件、缓冲区）----
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    let default_kit = DrumKitParams::DEFAULT;
    let wide_kit = {
        let mut kit = DrumKitParams::DEFAULT;
        kit.kick.decay_s = 1.2;
        kit.snare.noise_level = 0.31;
        kit.hihat.base_hz = 1_100.0;
        kit.clap.bursts = 6;
        kit.master_level = 0.7;
        kit
    };
    // 事件在窗口外建好：长尾的底鼓/开镲让槽位在整个窗口里都有声。
    let hits: Vec<(DrumVoice, f32)> = DrumVoice::ALL.iter().map(|voice| (*voice, 0.8)).collect();
    let mut block = vec![0.0f32; WINDOW_FRAMES];

    // ---- 观测窗口 ----
    let mut position: u64 = 0;
    let mut triggered: u64 = 0;
    let mut rendered_frames: u64 = 0;
    let mut nonzero_frames: u64 = 0;
    let mut voices_seen = [false; 5];
    // 新面（`latency_samples`）的运行期零分配判据：在**同一个观测窗口**内每个量子
    // 取一次读数，并把读数累加 —— 分配计数因此也覆盖这个新成员。
    let mut latency_calls: u64 = 0;
    let mut latency_sum: usize = 0;
    let reading = window(|| {
        for quantum in 0..RT_QUANTA {
            latency_calls += 1;
            latency_sum += machine.latency_samples();
            // 每量子一击 ⇒ 16 个槽位在第 17 个量子就撞满，之后**每量子都走一次
            // 窃取路径** ⇒ 窃取与 3 ms 淡出被密集覆盖。
            //
            // ⚠ 循环取事件 ⇒ 每个音色都被触发到；长尾音色（底鼓 1.2 s、
            // 开镲 0.32 s）保证"几乎每一帧都有声"。
            let (voice, gain) = hits[(triggered as usize) % hits.len()];
            machine.trigger(DrumHit::new(voice, position, gain));
            triggered += 1;
            // 每 5 个量子补一记开镲 —— 保证 `ClosedHat` 之后的 choke 分支一定被
            // 走到（只有开镲在响时闭镲才 choke）。
            if quantum % 5 == 0 {
                machine.trigger(DrumHit::new(DrumVoice::OpenHat, position, 1.0));
                triggered += 1;
            }
            if quantum % RETIRE_EVERY == 0 {
                machine.retire_finished(position);
            }
            if quantum % PARAMS_EVERY == 0 {
                machine.set_params(if (quantum / PARAMS_EVERY).is_multiple_of(2) {
                    default_kit
                } else {
                    wide_kit
                });
            }
            if quantum % RATE_EVERY == 0 {
                machine.set_sample_rate(if (quantum / RATE_EVERY).is_multiple_of(2) {
                    48_000
                } else {
                    44_100
                });
            }
            if quantum % RESET_EVERY == 0 {
                machine.reset();
            }
            machine.render(position, &mut block);
            rendered_frames += WINDOW_FRAMES as u64;
            nonzero_frames += block.iter().filter(|sample| **sample != 0.0).count() as u64;
            for index in 0..DRUM_SLOTS {
                if let Some(slot) = machine.debug_slot(index)
                    && slot.0
                    && slot.3 == 0
                {
                    voices_seen[usize::from(slot.1)] = true;
                }
            }
            position += WINDOW_FRAMES as u64;
        }
    });

    // ---- 覆盖度自检（见本判据文档）----
    assert_eq!(
        rendered_frames,
        RT_QUANTA * WINDOW_FRAMES as u64,
        "量子记账：每量子 {WINDOW_FRAMES} 帧"
    );
    assert_eq!(machine.triggers(), triggered, "触发计数必须对得上");
    assert!(
        machine.voice_steals() > 0,
        "窗口里从未窃取 ⇒ 窃取/淡出分支没被覆盖（假绿）"
    );
    assert!(
        machine.hat_chokes() > 0,
        "窗口里从未 choke 开镲 ⇒ 闭镲那条分支没被覆盖（假绿）"
    );
    assert!(
        voices_seen.iter().all(|seen| *seen),
        "有音色在窗口里从未占用过槽位：{voices_seen:?}（假绿）"
    );
    assert!(
        nonzero_frames * 10 > rendered_frames * 9,
        "有声音的帧只占 {nonzero_frames}/{rendered_frames} —— 窗口大部分是静音快路径（假绿）"
    );
    // ⚠ **逐槽位**覆盖度（"有声音的帧"是**逐帧**读数，它抓不到"16 个槽位里
    // 大部分静音"：只要有一个槽位在响，那一帧就算有声）。本机实测的正常值
    // 见下面的 eprintln；阈值 12 是"16 个槽位里平均至少 12 个在发声"。
    #[allow(clippy::cast_precision_loss)]
    let sounding_per_frame = machine.sounding_slot_frames() as f64 / rendered_frames as f64;
    assert!(
        sounding_per_frame >= 12.0,
        "平均同时发声的槽位只有 {sounding_per_frame:.2}/{DRUM_SLOTS} —— 大部分槽位在空转（假绿）"
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
    eprintln!(
        "[yeban-dsp/RT] drums 10 000 量子 × {WINDOW_FRAMES} 帧: \
         allocations={} deallocations={} rendered_frames={} nonzero_frames={} \
         triggered={} steals={} chokes={} voices_seen={voices_seen:?}          sounding_slot_frames={} ({sounding_per_frame:.2}/{DRUM_SLOTS} 每帧) latency_calls={latency_calls} latency_sum={latency_sum}",
        reading.allocations,
        reading.deallocations,
        rendered_frames,
        nonzero_frames,
        machine.triggers(),
        machine.voice_steals(),
        machine.hat_chokes(),
        machine.sounding_slot_frames()
    );
    assert_eq!(reading.allocations, 0, "实时路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "实时路径发生堆释放");
}

/// 量什么：正对照 —— 在**同一个**观测窗口里显式分配一次，读数必须是
/// `allocations == 1 && deallocations == 1`（单位：次数）。
///
/// 判据：两条都恰好为 1。若计数器坏了（永远读 0），这条会红
/// ⇒ 判据 1 的"全 0"才是有牙的 0。
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

/// 量什么：**本器件连构造都不分配**。
///
/// 与 `polysynth` 不同，鼓机**没有**波表库那样的构造期分配：槽位池是定长数组
/// （`[Slot; 16]`，约 6 KiB，落在栈上）。这条判据把这个差别钉住 ——
/// 若有人日后给槽位加了 `Vec`/`Box`，这里会红。
///
/// 判据：窗口内构造 `DrumMachine` 的分配次数为 0。
#[test]
fn constructing_a_machine_does_not_allocate() {
    let reading = window(|| {
        let machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        assert_eq!(machine.slots(), DRUM_SLOTS);
        assert_eq!(machine.active_slots(), 0);
    });
    eprintln!(
        "[yeban-dsp/RT] 窗口内构造 DrumMachine: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert_eq!(reading.total(), 0, "构造鼓机不该分配");
}

/// 量什么：**反面对照 A** —— 在窗口内为每个回调现开一块输出缓冲
/// （`vec![0.0f32; 128]`，实时路径最常见的误用之一）。
///
/// 判据：分配次数 > 0。它证明判据 1 的零分配**不是**"因为分配器看不见分配"，
/// 而是因为那条路径真的不分配；同时它把"输出缓冲必须由调用方预分配"
/// （[`DrumMachine::render`] 的 `&mut [f32]` 契约）变成**可测**的。
#[test]
fn allocating_the_block_inside_the_window_would_be_caught() {
    let reading = window(|| {
        let mut block = vec![0.0f32; WINDOW_FRAMES];
        let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
        machine.trigger(DrumHit::new(DrumVoice::Kick, 0, 1.0));
        machine.render(0, &mut block);
        assert_eq!(block.len(), WINDOW_FRAMES);
    });
    eprintln!(
        "[yeban-dsp/RT] 反面对照 A（回调内现开缓冲）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "回调内现开缓冲居然没被数到 ⇒ 本判据的仪表口径需要重看"
    );
}

/// 量什么：**反面对照 B** —— 在窗口内现攒一份事件列表
/// （`Vec<DrumHit>`；控制侧的常规做法，但**不能**发生在音频线程上）。
///
/// 判据：分配次数 > 0。
#[test]
fn allocating_the_event_list_inside_the_window_would_be_caught() {
    let reading = window(|| {
        let mut events: Vec<DrumHit> = Vec::new();
        for (index, voice) in DrumVoice::ALL.iter().enumerate() {
            events.push(DrumHit::new(*voice, index as u64 * 64, 1.0));
        }
        assert_eq!(events.len(), DrumVoice::ALL.len());
    });
    eprintln!(
        "[yeban-dsp/RT] 反面对照 B（回调内现攒事件表）: allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
    assert!(
        reading.allocations > 0,
        "回调内现攒事件表居然没被数到 ⇒ 本判据的仪表口径需要重看"
    );
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

/// 量什么：`render` 在 **`u64` 时间轴末端**（`position` 使 `position + k` 溢出，
/// 走 `saturating_add` 的饱和分支）时的运行期堆分配/释放次数（单位：次数）。
///
/// 观测方式：窗口里连续喂 `SATURATING_BLOCKS` 个 128 帧块，位置固定在
/// `u64::MAX − HEAD_FRAMES` ⇒ 每块的后 `HEAD_FRAMES` 帧都落在饱和分支上；
/// 每块同时触发一击（`trigger` 也在实时路径上），因此逐槽位循环在整个窗口里
/// 都有活干，不走 `render` 开头的静音短路。
///
/// 判据：`allocations == 0 && deallocations == 0`；覆盖度自检：窗口里确有非零
/// 输出样本（饱和分支所在的逐样本循环真的被执行）。
///
/// 为什么单独一条：`saturating_add` 是本票新增的分支，它必须与其余实时路径同
/// 一个口径 —— 零分配、零释放 [ARCH-RT-001 / `MUST-GATE-001`]。
#[test]
fn saturating_positions_allocate_nothing() {
    /// 饱和分支在每块里覆盖的帧数（帧）：块长减去它就落在未溢出的区段上。
    const HEAD_FRAMES: u64 = 64;
    /// 窗口里的块数（块）。
    const SATURATING_BLOCKS: u64 = 2_000;

    let position = u64::MAX - HEAD_FRAMES;
    let mut machine = DrumMachine::<DRUM_SLOTS>::new(48_000);
    machine.set_params(DrumKitParams::DEFAULT);
    let mut block = vec![0.0f32; WINDOW_FRAMES];
    let mut triggered: u64 = 0;
    let mut nonzero_frames: u64 = 0;

    let reading = window(|| {
        for block_index in 0..SATURATING_BLOCKS {
            let voice = DrumVoice::ALL[(block_index as usize) % DrumVoice::ALL.len()];
            machine.trigger(DrumHit::new(voice, position, 1.0));
            triggered += 1;
            machine.render(position, &mut block);
            nonzero_frames += block.iter().filter(|sample| **sample != 0.0).count() as u64;
        }
    });

    assert_eq!(machine.triggers(), triggered, "触发计数必须对得上");
    assert!(
        nonzero_frames > 0,
        "窗口里一个非零样本都没有 ⇒ 饱和分支可能没被执行（假绿）"
    );
    assert!(
        block.iter().all(|sample| sample.is_finite()),
        "末端饱和路径产出了非有限值"
    );
    assert_eq!(reading.allocations, 0, "末端饱和路径发生堆分配");
    assert_eq!(reading.deallocations, 0, "末端饱和路径发生堆释放");
    eprintln!(
        "[yeban-dsp/RT] drums 末端饱和: blocks={SATURATING_BLOCKS} × {WINDOW_FRAMES} 帧 at \
         position=u64::MAX-{HEAD_FRAMES}, 每块后 {HEAD_FRAMES} 帧饱和, triggered={triggered}, \
         nonzero_frames={nonzero_frames} allocations={} deallocations={}",
        reading.allocations, reading.deallocations
    );
}
