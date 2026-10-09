//! `line/engine-sound` 的运行期零分配判据 J5：
//! **合成路径接入之后，实时回调窗口仍然零分配、零释放** [MUST-GATE-001, ARCH-RT-001]。
//!
//! # 为什么必须单开一个 `harness = false` 目标
//!
//! `rt_zero_alloc.rs`（既有的 `[MUST-GATE-001]` 目标）用的是
//! `yeban_model::samples::filled_project()`，它的 4 个音符只覆盖前 1.5 秒 ——
//! 10,000 个量子的窗口里**绝大多数时间没有声部在跑**，于是"零分配"可能只是
//! "什么都没做"。本目标用**音符铺满整个窗口**的夹具（256 个交叠音符）复测，
//! 并**同时**断言窗口内确实有非零样本（覆盖度自检，防假绿）。
//!
//! 计数型全局分配器是**进程全局**的，libtest 自己的线程会污染计数
//! （第一版实测 `allocations=9`），因此这里与既有目标一样关掉 libtest
//! （`[[test]] harness = false`），只让主线程跑测量。
//!
//! # 场景 5 / 6（`line/engine-wiring` 追加）：每轨插入压缩器
//!
//! `crate::insert` 把 `TrackV3.devices` 的内置效果器投影成**每轨压缩器**之后，
//! 逐样本路径多了"查表 + `Compressor::process_mono`"、快照边界多了
//! `Compressor::new` / `set_params` / `set_sample_rate`（含 `exp`）。前四个场景的夹具
//! **没有**已识别的效果器设备 ⇒ 插入链整段跳过 ⇒ 对它是空转。
//! 因此追加两个场景：**10,000 量子**（逐样本路径）与 **31 次快照交换**（重新武装路径），
//! 两者都断言 `allocations == 0 && deallocations == 0`，并对"压缩器真的在压"
//! （`insert_gain_reductions > 0`）与"窗口里真的有声"做覆盖度自检。
//!
//! # 场景 7 / 8（`line/engine-wiring-2` 追加）：每轨插入**通道条**
//!
//! 场景 5 / 6 的夹具只有**动态参数** ⇒ 投影出来的通道条 `eq_enabled = false` /
//! `filter_enabled = false`（见 `crate::insert` 模块文档 §4.1）⇒ **EQ 级与滤波级
//! 在那些场景里整级不执行**，"零分配"对它们是空转。而 EQ 级是通道条里**唯一**
//! 用栈数组做块处理的一级（`yeban_dsp::channel_strip` 模块注释 §3 的
//! `[f32; 64]` × 2）⇒ 必须单独覆盖。
//!
//! 场景 7 因此用**两条轨的工程**：轨 A 只有动态参数，轨 B 是完整通道条
//! （EQ ＋ 滤波 ＋ 动态）⇒ 一次 10,000 量子窗口同时覆盖两条逐样本路径与两个
//! 武装槽位。场景 8 对同一份工程做 31 次快照交换 ⇒ 覆盖**重新武装**
//! （`ChannelStrip::set_params` / `set_sample_rate`，含 EQ 系数与 `exp`）。
//! 两个场景都做覆盖度自检：`insert_strip_frames > 0`（整链处理过）与
//! `insert_gain_reductions > 0`（动态级真的压过）。
//!
//! # 场景 9 / 10（`line/engine-reverb` 追加）：每轨插入**混响**
//!
//! 这是本文件里**唯一**一个"武装一件器件需要堆"的器件：`Reverb::set_sample_rate`
//! 会分配并释放延迟线（`crates/yeban-dsp/src/reverb.rs:73`）。本票把那条分配钉在
//! **构造期**（`EngineRuntime::new`，音频回调之外），快照边界只允许 `set_params`。
//! 场景 9 用两条轨（一条同时带通道条＋混响、一条只有混响）跑 10,000 个量子；
//! 场景 10 做 31 次同采样率重新武装，随后发布一份 **44.1 kHz** 的快照 ——
//! 引擎在那里**拒绝**重建延迟线（`insert_reverb_rate_rejects` +1）而不是在音频线程
//! 分配，最后换回 48 kHz 证明重武装仍然可行。三段的每一个窗口都断言
//! `allocations == 0 && deallocations == 0`。覆盖度自检取**整窗的精确帧数**
//! （`2 × 10,001 × 128`）而不是"大于 0"。
//!
//! # 场景 11 / 12（`line/engine-drums` 追加）：每轨**鼓机音源**
//!
//! 鼓机（`yeban_dsp::drums`）与前面所有器件有**两处结构差别**，两处都必须被覆盖：
//!
//! 1. 它是**音源**（`trigger` + `render(position, out)`），不是插入链上的逐样本变换
//!    ⇒ 它的逐样本路径在 `SynthEngine::render_track` 里，**且**每个起音要过
//!    `DrumHit` 的构造与键位映射查找（`DrumNoteMap::voice_for`）；
//! 2. 它**不持有堆**：`set_sample_rate` 只重算在响槽位的系数（对比：混响在那里
//!    **拒绝**武装）⇒ **换采样率也必须零分配**，而这条路径前面的场景从没走过
//!    （场景 10 的换采样率分支恰恰是"拒绝"）。
//!
//! 场景 11 用两条轨（一条鼓机、一条复音合成器）跑 10,000 个量子，覆盖度自检取
//! **精确的触发数**（由夹具的音符栅格算出，不是"大于 0"）；场景 12 做 31 次同采样率
//! 重新武装 ＋ 一次 **44.1 kHz**（鼓机照常武装）＋ 换回 48 kHz ＋ 换一套键位映射
//! ＋ 换回复音合成器，每一步都断言零分配零释放。
//!
//! # 场景 13 / 14（`line/engine-2` 追加）：波形选择与**第二条振荡器**
//!
//! 器件 `yeban_dsp::polysynth` 早就是双振荡器，但引擎侧到本票才把
//! `osc1_wave` / `osc2_wave` / `osc2_level` / `osc2_detune_cents` 四个参数投影进
//! `ToneParams`。既有全部场景的夹具都**没有**振荡器参数 ⇒ `osc2_level` 恒为 0
//! ⇒ 器件的第二条支路一帧也不执行，这条武装路径与逐样本路径都是空转。
//!
//! 场景 13 用一条轨（音符铺满窗口）＋一台**只写振荡器键**的设备
//! （`osc1_wave = 4`、`osc2` 开在电平 0.6、失谐 11 音分）跑 10,000 个量子；
//! 场景 14 在同一份夹具上做 31 次重新武装，**每一轮换一次两个波形下标**，
//! 并做两条覆盖度见证：① 实时侧读数 `EngineRuntime::armed_tone` 逐轮等于发布值；
//! ② 31 轮里至少有一次输出指纹发生变化（参数真的到了器件）。
//! 两个场景的每一个窗口都断言 `allocations == 0 && deallocations == 0`。
//!
//! ⚠ **本目标只测四元组里的两个分量**（`allocations` / `deallocations`）：
//! 它没有锁探针，也没有 I/O 边界（那是 `tests/rt_zero_alloc.rs` 的
//! `[MUST-GATE-001]` 目标）。因此这两个场景**不**声称"六分量全 0"。

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};

mod support;

use support::{MixSpec, NoteSpec, note_project, tuned_project, two_track_project};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, SampleRate};

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法只做"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有跨调用状态（计数器是原子量）。`unsafe` 块的边界就是 `System` 的调用本身。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `layout` 合法（`GlobalAlloc` 的契约）。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr` 来自本分配器且 `layout` 与分配时一致。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            // 重新分配在实时路径上同样禁止（可能搬迁并复制），因此也计入分配。
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure<F: FnOnce()>(label: &str, body: F) -> (usize, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    body();
    ARMED.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let deallocations = DEALLOCATIONS.load(Ordering::SeqCst);
    println!("[engine-sound/J5] {label}: allocations={allocations} deallocations={deallocations}");
    (allocations, deallocations)
}

/// 256 个交叠音符的夹具：`i` 从 0 起每 240 tick 起音、时值 480 tick。
///
/// 覆盖窗口 = `256 × 240 + 480 = 61920` tick = 1,548,000 样本，
/// 比判据的 10,000 个量子（1,280,000 样本）更长 ⇒ **整个测量窗口里都有声部在跑**。
fn saturated_notes() -> Vec<NoteSpec> {
    (0..256u64)
        .map(|index| NoteSpec::at(index * 240, 480, 60 + (index % 12) as u8, 100))
        .collect()
}

/// 场景 11 / 12 的**键位映射**（五个鼓件各一个音高）。
const DRUM_MAP: [u8; 5] = [36, 38, 42, 46, 39];

/// 场景 11 / 12 的鼓机音符：与 [`saturated_notes`] 同一个时间栅格（每 240 tick 起音），
/// 但音高**轮流落在 [`DRUM_MAP`] 的五个音高上** ⇒ 每一记都命中一个鼓件。
fn drum_notes() -> Vec<NoteSpec> {
    (0..256u64)
        .map(|index| NoteSpec::at(index * 240, 480, DRUM_MAP[(index % 5) as usize], 100))
        .collect()
}

/// 一台**完整键位映射**的鼓机设备（`InternalInstrument`）。
fn drum_device(extra: &[(&str, f32)], map: [f32; 5]) -> DeviceDefinition {
    let mut params = vec![
        ParameterValue {
            name: "kick_note".to_owned(),
            value: map[0],
            unit: None,
        },
        ParameterValue {
            name: "snare_note".to_owned(),
            value: map[1],
            unit: None,
        },
        ParameterValue {
            name: "closed_hat_note".to_owned(),
            value: map[2],
            unit: None,
        },
        ParameterValue {
            name: "open_hat_note".to_owned(),
            value: map[3],
            unit: None,
        },
        ParameterValue {
            name: "clap_note".to_owned(),
            value: map[4],
            unit: None,
        },
    ];
    params.extend(extra.iter().map(|(name, value)| ParameterValue {
        name: (*name).to_owned(),
        value: *value,
        unit: None,
    }));
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params,
        latency_samples: 0,
    }
}

/// 场景 13 / 14 的**振荡器设备**（只写振荡器键 ⇒ 滤波器旁通、音源是复音合成器）。
///
/// 刻意**不**带任何鼓机键位（`kick_note` 等）：带上就会被识别成鼓机，
/// 场景 13 / 14 要测的恰恰是复音合成器的**振荡器**路径。
fn osc_device(
    osc1_wave: f32,
    osc2_wave: f32,
    osc2_level: f32,
    osc2_detune_cents: f32,
) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Hollow".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: vec![
            ParameterValue {
                name: "osc1_wave".to_owned(),
                value: osc1_wave,
                unit: None,
            },
            ParameterValue {
                name: "osc2_wave".to_owned(),
                value: osc2_wave,
                unit: None,
            },
            ParameterValue {
                name: "osc2_level".to_owned(),
                value: osc2_level,
                unit: None,
            },
            ParameterValue {
                name: "osc2_detune_cents".to_owned(),
                value: osc2_detune_cents,
                unit: None,
            },
        ],
        latency_samples: 0,
    }
}

/// 把一台振荡器设备挂到夹具轨上（**只改夹具**，不改模型层）。
fn set_oscillators(
    project: &mut yeban_model::YebanProjectV1,
    track: EntityId,
    osc1_wave: f32,
    osc2_wave: f32,
    osc2_level: f32,
    osc2_detune_cents: f32,
) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = vec![osc_device(
        osc1_wave,
        osc2_wave,
        osc2_level,
        osc2_detune_cents,
    )];
}

/// 把一段输出折进 FNV-1a 64 中间值（跨量子累积用；只做整数运算，**不分配**）。
fn osc_fold(mut hash: u64, samples: &[f32]) -> u64 {
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// 从**新建的**实时侧渲染 `quanta` 个量子，返回输出的累积指纹（`osc1_wave` 是唯一变量）。
///
/// ⚠ 全部构造（工程 → 快照 → `EngineRuntime`）都在测量窗口**之外** —— 这个函数只用于
/// 场景 14 的差分覆盖度见证，不参与"零分配"读数。
fn osc_render_fingerprint(osc1_wave: f32, quanta: usize) -> u64 {
    let fixture = note_project(&saturated_notes());
    let mut project = fixture.project;
    let track = fixture.track;
    set_oscillators(&mut project, track, osc1_wave, 1.0, 0.6, 11.0);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("差分夹具快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(8192);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut output = vec![0.0f32; 128 * 2];
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..quanta {
        runtime.process_quantum(&mut output, 2);
        hash = osc_fold(hash, &output);
    }
    hash
}

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    // ---- 装配（**允许分配**：全部发生在打开设备之前）----
    let fixture = note_project(&saturated_notes());
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, mut queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(8192);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut output = vec![0.0f32; 128 * 2];

    // 预热：包络模板武装、波表读取的首个 mip 级选择等一次性路径。
    // 不计入实时窗口（否则会把"初始化"记到实时路径头上 —— 那是假红）。
    runtime.process_quantum(&mut output, 2);

    // ---- 场景 1：音符铺满窗口，连续 10,000 个量子必须零分配、零释放 ----
    let mut nonzero = 0usize;
    let mut peak = 0.0f32;
    let (allocations, deallocations) = measure("10_000 quanta (saturated notes)", || {
        for _ in 0..10_000 {
            runtime.process_quantum(&mut output, 2);
            for sample in &output {
                if *sample != 0.0 {
                    nonzero += 1;
                }
                peak = peak.max(sample.abs());
            }
        }
    });
    if allocations != 0 {
        failures.push(format!(
            "合成路径在实时窗口内分配了 {allocations} 次堆内存 —— [MUST-GATE-001] 一票否决"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "合成路径在实时窗口内释放了 {deallocations} 次 —— [MUST-GATE-001] 一票否决"
        ));
    }
    // 覆盖度自检：窗口里必须**真的**在出声，否则"零分配"是空转。
    if nonzero == 0 {
        failures
            .push("10,000 个量子的窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let stats = runtime.stats();
    if stats.notes_triggered < 200 {
        failures.push(format!(
            "只触发了 {} 个音符，未达到压测规模（应 ≥ 200）",
            stats.notes_triggered
        ));
    }
    if stats.voice_steals != 0 {
        failures.push(format!(
            "夹具不应触及复音上限，但发生了 {} 次声部窃取",
            stats.voice_steals
        ));
    }

    // ---- 场景 2：高频快照交换（每次交换都要重新对齐声部池）----
    let mut switches = 0u64;
    for revision in 2..=64u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&fixture.project, revision).expect("快照");
        slot.publish(next);
        let (allocations, deallocations) = measure("snapshot swap + quantum", || {
            runtime.process_quantum(&mut output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "快照交换期间的实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "实时线程释放了 {deallocations} 次（revision={revision}）—— 旧快照必须经退役队列交回主线程"
            ));
        }
        switches += queue.drain(64) as u64;
    }
    if switches == 0 {
        failures.push("主线程侧没有从退役队列回收任何旧快照 —— 场景 2 是空转".to_owned());
    }

    // ---- 场景 3：回到 `filled_project`（真实密度的规范样本）复测 ----
    let filled = yeban_model::samples::filled_project();
    let filled_snapshot = EngineSnapshot::from_project(&filled, 1).expect("规范样本快照");
    let filled_slot = SnapshotSlot::new(filled_snapshot);
    let (filled_retire, _filled_queue) = retire_channel(8);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut filled_runtime = EngineRuntime::new(&filled_slot, filled_retire, receiver, publisher);
    let mut filled_output = vec![0.0f32; 128 * 2];
    filled_runtime.process_quantum(&mut filled_output, 2);
    let mut filled_nonzero = 0usize;
    let (allocations, deallocations) = measure("filled_project 4_000 quanta", || {
        for _ in 0..4_000 {
            filled_runtime.process_quantum(&mut filled_output, 2);
            for sample in &filled_output {
                if *sample != 0.0 {
                    filled_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "filled_project 窗口不是零分配零释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if filled_nonzero == 0 {
        failures
            .push("filled_project 的 4,000 个量子没有任何非零样本 —— 覆盖度自检失败".to_owned());
    }
    let filled_stats = filled_runtime.stats();
    if filled_stats.scheduled_notes == 0 {
        failures.push("filled_project 的快照没有调度任何音符".to_owned());
    }

    // ---- 场景 4：**整条混音链**（滤波器 + 声相 + 母线限制器）仍然零分配 ----
    //
    // 为什么必须单独一个场景：前三个场景里母线是"等增益复制 + 不压限"的，
    // 声部滤波器也从未被调用。本线的三个新器件（声部低通、声相增益、前瞻限制器）
    // 都在实时路径上，**必须**单独证明它们不分配。
    //
    // 夹具设计（每一项都对应一个器件）：
    //   * 256 个交叠音符（沿用 `saturated_notes`）⇒ 整个窗口有声部在跑；
    //   * `+6 dB` 音量 ⇒ 峰值约 1.42 > 阈值 0.9 ⇒ **限制器真的在工作**；
    //   * `pan = -1.0` ⇒ 右声道应当**逐位静音**（声相定律真的在线）；
    //   * `cutoff = 2 kHz` ⇒ 声部低通真的被调用；
    //   * 另外再叠 40 个**同时起音**的长音符 ⇒ 逼出声部窃取淡出（池只有 16 声部）。
    let mut mix_notes = saturated_notes();
    mix_notes.extend((0..40u64).map(|index| NoteSpec::at(0, 96_000, 48 + (index % 12) as u8, 100)));
    let mix_fixture = tuned_project(
        &mix_notes,
        MixSpec {
            volume_db: 6.0,
            pan: -1.0,
            cutoff_hz: Some(2_000.0),
            resonance: 0.4,
        },
    );
    let mix_snapshot =
        EngineSnapshot::from_project(&mix_fixture.project, 1).expect("混音链夹具必须能编译成快照");
    let mix_slot = SnapshotSlot::new(mix_snapshot);
    let (mix_retire, _mix_queue) = retire_channel(8);
    let (_sender, mix_receiver) = event_channel(64);
    let (mix_publisher, _mix_collector) = meter_channel(8192);
    let mut mix_runtime = EngineRuntime::new(&mix_slot, mix_retire, mix_receiver, mix_publisher);
    let mut mix_output = vec![0.0f32; 128 * 2];
    mix_runtime.process_quantum(&mut mix_output, 2);

    let mut mix_nan = 0usize;
    let (allocations, deallocations) =
        measure("mix chain (filter+pan+limiter+steal) 2_000 quanta", || {
            for _ in 0..2_000 {
                mix_runtime.process_quantum(&mut mix_output, 2);
                for sample in &mix_output {
                    if sample.is_nan() {
                        mix_nan += 1;
                    }
                }
            }
        });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "混音链在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if mix_nan != 0 {
        failures.push(format!("混音链输出了 {mix_nan} 个 NaN 样本"));
    }
    let mix_stats = mix_runtime.stats();
    if mix_stats.limiter_gain_reductions == 0 {
        failures.push(
            "混音链夹具没有驱动限制器（reductions = 0）—— 这条零分配判据没有覆盖限制器".to_owned(),
        );
    }
    if mix_stats.voice_steals == 0 {
        failures.push("混音链夹具没有触发声部窃取 —— 淡出路径没有被这条零分配判据覆盖".to_owned());
    }
    // 声相真的在线：全左 ⇒ 右声道必须**逐位**静音。
    let right_nonzero = mix_output
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|sample| **sample != 0.0)
        .count();
    if right_nonzero != 0 {
        failures.push(format!(
            "全左声相下右声道仍有 {right_nonzero} 个非零样本 —— 声相定律没有生效"
        ));
    }
    println!(
        "[engine-mix/J5] 混音链: quanta={} reductions={} steals={} 最大压限={:.4} 右声道非零={right_nonzero}",
        mix_stats.quanta,
        mix_stats.limiter_gain_reductions,
        mix_stats.voice_steals,
        mix_stats.limiter_max_reduction,
    );

    // ---- 场景 5：**每轨插入压缩器**（`crate::insert`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景：前四个场景的夹具都**没有**已识别的效果器设备
    // ⇒ `armed_compressors` 全是 `None` ⇒ 插入链**整段跳过**，"零分配"对它是空转。
    // 接线之后逐样本路径多了"查表 + `Compressor::process_mono`"，快照边界多了
    // `Compressor::new` / `set_params` / `set_sample_rate`（三者都会重算含 `exp` 的
    // 一阶低通系数）—— 这两条路径都必须证明不分配 [MUST-GATE-001, ARCH-RT-001]。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 256 个交叠音符（沿用 `saturated_notes`）⇒ 整个窗口有声部在跑；
    //   * 阈值 −30 dBFS / 比率 8 ⇒ 压缩器**每个量子真的在压**（行为覆盖，防假绿）；
    //   * 场景 6 再叠"每量子换一次快照" ⇒ 覆盖**重新武装**路径（`set_params`）。
    let mut insert_fixture = note_project(&saturated_notes());
    let insert_track = insert_fixture.track;
    {
        let entry = insert_fixture
            .project
            .tracks
            .get_mut(&insert_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Comp".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let insert_snapshot =
        EngineSnapshot::from_project(&insert_fixture.project, 1).expect("插入链夹具快照");
    assert!(
        !insert_snapshot.inserts().is_empty(),
        "插入链夹具必须真的挂上压缩器，否则本场景是空转"
    );
    let insert_slot = SnapshotSlot::new(insert_snapshot);
    let (insert_retire, mut insert_queue) = retire_channel(64);
    let (_sender, insert_receiver) = event_channel(64);
    let (insert_publisher, _insert_collector) = meter_channel(8192);
    let mut insert_runtime = EngineRuntime::new(
        &insert_slot,
        insert_retire,
        insert_receiver,
        insert_publisher,
    );
    let mut insert_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`Compressor::new`）与首个量子的一次性路径。
    insert_runtime.process_quantum(&mut insert_output, 2);
    if insert_runtime.armed_insert_slot_count() != 1 {
        failures.push(format!(
            "插入链夹具应武装 1 台压缩器，实际 {} 台",
            insert_runtime.armed_insert_slot_count()
        ));
    }

    let mut insert_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert compressor 10_000 quanta", || {
        for _ in 0..10_000 {
            insert_runtime.process_quantum(&mut insert_output, 2);
            for sample in &insert_output {
                if *sample != 0.0 {
                    insert_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入压缩器在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if insert_nonzero == 0 {
        failures.push("插入链窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let insert_stats = insert_runtime.stats();
    if insert_stats.insert_gain_reductions == 0 {
        failures.push(
            "插入压缩器一次都没压到样本（reductions = 0）—— 这条零分配判据没有覆盖压缩器"
                .to_owned(),
        );
    }
    println!(
        "[engine-wiring/J5] 插入压缩器: quanta={} 压过样本={} 最大衰减={:.3} dB 非零样本={insert_nonzero}",
        insert_stats.quanta,
        insert_stats.insert_gain_reductions,
        insert_stats.insert_max_reduction_db,
    );

    // ---- 场景 6：快照交换时**重新武装**压缩器（`set_params` / `set_sample_rate`）----
    let mut insert_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&insert_fixture.project, revision).expect("快照");
        insert_slot.publish(next);
        let (allocations, deallocations) = measure("insert re-arm + quantum", || {
            insert_runtime.process_quantum(&mut insert_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装插入压缩器时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装插入压缩器时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        insert_switches += insert_queue.drain(64) as u64;
    }
    if insert_switches == 0 {
        failures.push("插入链场景没有从退役队列回收任何旧快照 —— 场景 6 是空转".to_owned());
    }

    // ---- 场景 7：**每轨插入通道条**（`crate::insert`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景：见文件头 "场景 7 / 8"。要点是场景 5 / 6 的夹具只让
    // **动态级**启用 ⇒ 通道条里那一级用**栈数组**做块处理的 EQ（以及滤波级）没有被覆盖。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨，各自的音符都铺满整个窗口（沿用 `saturated_notes`）；
    //   * 轨 A 只有动态参数（`threshold_db = −30` / `ratio = 8`）⇒ 只有动态级启用；
    //   * 轨 B 有 EQ ＋ 滤波 ＋ 动态参数 ⇒ **EQ 级与滤波级真的执行**（块路径被走到）；
    //   * 两条轨 ⇒ 两个武装槽位（覆盖"多轨各占一槽"）。
    let insert_notes = saturated_notes();
    let (mut strip_project, strip_only_track, compressor_only_track) =
        two_track_project(&insert_notes, &insert_notes);
    {
        let entry = strip_project
            .tracks
            .get_mut(&compressor_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Comp".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    {
        let entry = strip_project
            .tracks
            .get_mut(&strip_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Strip".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                // EQ 级：低架提升 ＋ 高架衰减（⇒ 三个双二阶都真的跑）。
                ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 6.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "eq_high_gain".to_owned(),
                    value: -12.0,
                    unit: Some("dB".to_owned()),
                },
                // 滤波级：900 Hz 低通 ⇒ 两个声道各一份四极梯形状态都真的跑。
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: 900.0,
                    unit: Some("Hz".to_owned()),
                },
                // 动态级：与轨 A 同一口径 ⇒ 两条轨都会压。
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let strip_snapshot = EngineSnapshot::from_project(&strip_project, 1).expect("通道条夹具快照");
    if strip_snapshot.inserts().len() != 2 {
        failures.push(format!(
            "通道条夹具应有 2 条插入链（两条轨各一台器件），实际 {} 条",
            strip_snapshot.inserts().len()
        ));
    }
    let strip_slot = SnapshotSlot::new(strip_snapshot);
    let (strip_retire, mut strip_queue) = retire_channel(64);
    let (_sender, strip_receiver) = event_channel(64);
    let (strip_publisher, _strip_collector) = meter_channel(8192);
    let mut strip_runtime =
        EngineRuntime::new(&strip_slot, strip_retire, strip_receiver, strip_publisher);
    let mut strip_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`ChannelStrip::new` 的 EQ／滤波／压缩系数计算）与首个量子。
    strip_runtime.process_quantum(&mut strip_output, 2);
    if strip_runtime.armed_insert_slot_count() != 2 {
        failures.push(format!(
            "通道条夹具应武装 2 台器件（两条轨各一台），实际 {} 台",
            strip_runtime.armed_insert_slot_count()
        ));
    }
    if strip_runtime.armed_strip(&strip_only_track).is_none()
        || strip_runtime.armed_strip(&compressor_only_track).is_none()
    {
        failures.push("通道条夹具的两条轨都必须武装进实时侧".to_owned());
    }

    let mut strip_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert channel strip 10_000 quanta", || {
        for _ in 0..10_000 {
            strip_runtime.process_quantum(&mut strip_output, 2);
            for sample in &strip_output {
                if *sample != 0.0 {
                    strip_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入通道条在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if strip_nonzero == 0 {
        failures.push("通道条窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let strip_stats = strip_runtime.stats();
    // 覆盖度：整链处理过帧（EQ／滤波／动态三级都在被调用），且动态级真的压过。
    if strip_stats.insert_strip_frames == 0 {
        failures.push(
            "通道条一次都没有处理过帧（insert_strip_frames = 0）—— 这条零分配判据没有覆盖通道条"
                .to_owned(),
        );
    }
    if strip_stats.insert_gain_reductions == 0 {
        failures.push(
            "通道条的动态级一次都没压到帧（reductions = 0）—— 这条零分配判据没有覆盖动态级"
                .to_owned(),
        );
    }
    println!(
        "[engine-wiring-2/J7] 插入通道条: quanta={} 整链处理帧数={} 动态级压过帧数={} 最大衰减={:.3} dB 非零样本={strip_nonzero}",
        strip_stats.quanta,
        strip_stats.insert_strip_frames,
        strip_stats.insert_gain_reductions,
        strip_stats.insert_max_reduction_db,
    );

    // ---- 场景 8：快照交换时**重新武装**通道条（`set_params` / `set_sample_rate`）----
    let mut strip_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&strip_project, revision).expect("快照");
        strip_slot.publish(next);
        let (allocations, deallocations) = measure("strip re-arm + quantum", || {
            strip_runtime.process_quantum(&mut strip_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装插入通道条时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装插入通道条时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        strip_switches += strip_queue.drain(64) as u64;
    }
    if strip_switches == 0 {
        failures.push("通道条场景没有从退役队列回收任何旧快照 —— 场景 8 是空转".to_owned());
    }

    // ---- 场景 9：**每轨插入混响**（`crate::insert` 的 `reverb`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景（本场景与前面所有场景的**结构差别**）：
    // `Reverb::set_sample_rate` 会分配并释放延迟线（`Vec`）—— 它是整个引擎里唯一一个
    // "武装一件器件需要堆"的器件。本票把那条分配**钉在构造期**
    // （`EngineRuntime::new`），快照边界只允许 `set_params`。这条判据就是那个说法的
    // 运行期证据：10,000 个量子（逐样本路径）与 31 次快照交换（重新武装路径）
    // 都必须零分配零释放 [MUST-GATE-001, ARCH-RT-001]。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨，各自的音符都铺满整个窗口（沿用 `saturated_notes`）；
    //   * 轨 A 同时带**通道条**与**混响**旋钮（同一台设备两件器件，覆盖"一条轨两级"）；
    //   * 轨 B 只有混响旋钮（覆盖"一台设备只出混响"）；
    //   * 湿声 1.0 + 衰减最长（`size = 1.0`）⇒ 混响**每个量子都在处理**，
    //     且尾巴足够长（覆盖度自检因此不是空转）。
    let reverb_notes = saturated_notes();
    let (mut reverb_project, reverb_both_track, reverb_only_track) =
        two_track_project(&reverb_notes, &reverb_notes);
    {
        let entry = reverb_project
            .tracks
            .get_mut(&reverb_both_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Strip+Reverb".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                // 通道条：EQ ＋ 滤波 ＋ 动态（复用场景 7 的口径）。
                ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 6.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: 6_000.0,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
                // 混响：全湿、最长衰减、带预延迟。
                ParameterValue {
                    name: "reverb_size".to_owned(),
                    value: 1.0,
                    unit: None,
                },
                ParameterValue {
                    name: "reverb_wet".to_owned(),
                    value: 1.0,
                    unit: None,
                },
                ParameterValue {
                    name: "reverb_predelay".to_owned(),
                    value: 0.02,
                    unit: Some("s".to_owned()),
                },
            ],
            latency_samples: 0,
        }];
    }
    {
        let entry = reverb_project
            .tracks
            .get_mut(&reverb_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Reverb".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "reverb_size".to_owned(),
                    value: 0.9,
                    unit: None,
                },
                ParameterValue {
                    name: "reverb_wet".to_owned(),
                    value: 0.8,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let reverb_snapshot = EngineSnapshot::from_project(&reverb_project, 1).expect("混响夹具快照");
    if reverb_snapshot.inserts().len() != 2 {
        failures.push(format!(
            "混响夹具应有 2 条插入链（两条轨各一台器件），实际 {} 条",
            reverb_snapshot.inserts().len()
        ));
    }
    let reverb_slot = SnapshotSlot::new(reverb_snapshot);
    let (reverb_retire, mut reverb_queue) = retire_channel(64);
    let (_sender, reverb_receiver) = event_channel(64);
    let (reverb_publisher, _reverb_collector) = meter_channel(8192);
    let mut reverb_runtime = EngineRuntime::new(
        &reverb_slot,
        reverb_retire,
        reverb_receiver,
        reverb_publisher,
    );
    let mut reverb_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`Reverb::set_params`）与首个量子。延迟线本身在 `new` 里就分配好了。
    reverb_runtime.process_quantum(&mut reverb_output, 2);
    if reverb_runtime.armed_reverb_slot_count() != 2 {
        failures.push(format!(
            "混响夹具应武装 2 台混响，实际 {} 台",
            reverb_runtime.armed_reverb_slot_count()
        ));
    }
    if reverb_runtime.armed_reverb_sample_rate() != 48_000 {
        failures.push(format!(
            "混响延迟线池应按初始快照的 48 kHz 武装，实际 {} Hz",
            reverb_runtime.armed_reverb_sample_rate()
        ));
    }
    if reverb_runtime.armed_reverb(&reverb_both_track).is_none()
        || reverb_runtime.armed_reverb(&reverb_only_track).is_none()
    {
        failures.push("混响夹具的两条轨都必须武装进实时侧".to_owned());
    }

    let mut reverb_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert reverb 10_000 quanta", || {
        for _ in 0..10_000 {
            reverb_runtime.process_quantum(&mut reverb_output, 2);
            for sample in &reverb_output {
                if *sample != 0.0 {
                    reverb_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入混响在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if reverb_nonzero == 0 {
        failures.push("混响窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let reverb_stats = reverb_runtime.stats();
    // 覆盖度：混响真的处理过帧。门槛取**场景 9 的实际规模**而不是 0：
    // `> 0` 对"只跑了一个量子"也成立，那条读数无法区分"整窗都在处理"与"只处理了一次"。
    let expected_reverb_frames = 2 * (10_000 + 1) * 128;
    if reverb_stats.insert_reverb_frames != expected_reverb_frames {
        failures.push(format!(
            "混响整窗处理帧数={}（期望 {} = 2 条轨 × 10,001 个量子 × 128 帧）—— \
             这条零分配判据没有覆盖整条逐样本路径",
            reverb_stats.insert_reverb_frames, expected_reverb_frames
        ));
    }
    if reverb_stats.insert_reverb_rate_rejects != 0 {
        failures.push(format!(
            "采样率没有变，混响不该被拒绝武装，实际拒绝 {} 次",
            reverb_stats.insert_reverb_rate_rejects
        ));
    }
    println!(
        "[engine-wiring-3/J9] 插入混响: quanta={} 混响处理帧数={} 非零样本={reverb_nonzero} 武装采样率={}",
        reverb_stats.quanta,
        reverb_stats.insert_reverb_frames,
        reverb_runtime.armed_reverb_sample_rate(),
    );

    // ---- 场景 10：重新武装混响 + **换采样率时拒绝重建延迟线**（都零分配）----
    //
    // 两半各自对应一条真实路径：
    //   * 前 31 次交换 = **同一个采样率** ⇒ 走 `set_params`（以及槽位重占时的
    //     `set_sample_rate` 同值复位，它走 `fill(0.0)` 分支、**零分配**）；
    //   * 最后一次 = **换采样率**（44.1 kHz）⇒ 引擎**拒绝**重建延迟线
    //     （`Reverb::set_sample_rate` 会 `Vec` 重分配 + 释放），整段不武装并计数。
    //     一个"照着场景 7/8 写"的实现会在这里调用 `set_sample_rate` ⇒ 本场景变红。
    let mut reverb_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&reverb_project, revision).expect("快照");
        reverb_slot.publish(next);
        let (allocations, deallocations) = measure("reverb re-arm + quantum", || {
            reverb_runtime.process_quantum(&mut reverb_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装插入混响时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装插入混响时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        reverb_switches += reverb_queue.drain(64) as u64;
    }
    if reverb_switches == 0 {
        failures.push("混响场景没有从退役队列回收任何旧快照 —— 场景 10 是空转".to_owned());
    }
    if reverb_runtime.armed_reverb_slot_count() != 2 {
        failures.push("同采样率重新武装之后两条轨的混响都必须仍被武装".to_owned());
    }

    // 换采样率：44.1 kHz 的快照**不**武装混响（延迟线不为新采样率重建）。
    let mut shifted = reverb_project.clone();
    shifted.audio_config.sample_rate = SampleRate::Hz44100;
    let shifted_snapshot = EngineSnapshot::from_project(&shifted, 33).expect("换采样率快照");
    reverb_slot.publish(shifted_snapshot);
    let before_rejects = reverb_runtime.stats().insert_reverb_rate_rejects;
    let (allocations, deallocations) = measure("reverb rate-mismatch re-arm + quantum", || {
        reverb_runtime.process_quantum(&mut reverb_output, 2);
    });
    if allocations != 0 {
        failures.push(format!(
            "换采样率时实时路径分配了 {allocations} 次 —— 延迟线绝不能在音频线程重建"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "换采样率时实时线程释放了 {deallocations} 次 —— 延迟线绝不能在音频线程重建"
        ));
    }
    let after = reverb_runtime.stats();
    if after.insert_reverb_rate_rejects != before_rejects + 1 {
        failures.push(format!(
            "换采样率应恰好累加 1 次拒绝，实测 {} -> {}",
            before_rejects, after.insert_reverb_rate_rejects
        ));
    }
    if reverb_runtime.armed_reverb_slot_count() != 0 {
        failures.push(format!(
            "换采样率之后混响必须整段不武装，实际仍武装 {} 台",
            reverb_runtime.armed_reverb_slot_count()
        ));
    }
    // 换回 48 kHz：必须能重新武装（守卫是"拒绝这一份"，不是"永久停用"）。
    let back_snapshot = EngineSnapshot::from_project(&reverb_project, 34).expect("换回快照");
    reverb_slot.publish(back_snapshot);
    let (allocations, deallocations) = measure("reverb rate-restore re-arm + quantum", || {
        reverb_runtime.process_quantum(&mut reverb_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换回 48 kHz 重新武装混响时分配/释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if reverb_runtime.armed_reverb_slot_count() != 2 {
        failures.push("换回 48 kHz 之后两条轨的混响必须重新武装".to_owned());
    }
    println!(
        "[engine-wiring-3/J10] 混响重新武装: 交换={} 次；换采样率后拒绝累计={} 次；武装采样率={} Hz",
        reverb_switches,
        reverb_runtime.stats().insert_reverb_rate_rejects,
        reverb_runtime.armed_reverb_sample_rate(),
    );

    // ---- 场景 11：**每轨鼓机音源**（`crate::drums`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景（见文件头 "场景 11 / 12"）：鼓机是**音源**，它的逐样本
    // 路径在 `SynthEngine::render_track` 里（触发 + 渲染），而不是插入链上的变换；
    // 且它的 `set_sample_rate` **不分配**（与混响相反）。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨：一根鼓机轨（音符的音高轮流命中五个鼓件）＋ 一条复音合成器轨
    //     （`cutoff_hz` ⇒ 同一窗口里另一条音源仍在跑）；
    //   * 鼓机的尾巴取器件默认值（底鼓 0.40 s）⇒ 整窗都有槽位在响。
    let (mut drum_project, drum_track, drum_synth_track) =
        two_track_project(&drum_notes(), &saturated_notes());
    {
        let entry = drum_project
            .tracks
            .get_mut(&drum_track)
            .expect("夹具里必须有那条鼓机轨");
        entry.devices = vec![drum_device(
            &[("kick_decay_s", 0.40), ("master_level", 0.9)],
            [36.0, 38.0, 42.0, 46.0, 39.0],
        )];
    }
    {
        let entry = drum_project
            .tracks
            .get_mut(&drum_synth_track)
            .expect("夹具里必须有那条复音轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Hollow".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: 800.0,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "resonance".to_owned(),
                    value: 0.3,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let drum_snapshot = EngineSnapshot::from_project(&drum_project, 1).expect("鼓机夹具快照");
    if drum_snapshot.drums().len() != 1 {
        failures.push(format!(
            "鼓机夹具应只有 1 条鼓机轨，实际 {} 条（只写了 cutoff_hz 的那条不算）",
            drum_snapshot.drums().len()
        ));
    }
    let drum_slot = SnapshotSlot::new(drum_snapshot);
    let (drum_retire, mut drum_queue) = retire_channel(64);
    let (_drum_sender, drum_receiver) = event_channel(64);
    let (drum_publisher, _drum_collector) = meter_channel(8192);
    let mut drum_runtime =
        EngineRuntime::new(&drum_slot, drum_retire, drum_receiver, drum_publisher);
    let mut drum_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`DrumMachine::set_params`）与首个量子。
    drum_runtime.process_quantum(&mut drum_output, 2);
    if drum_runtime.armed_drum_slot_count() != 1 {
        failures.push(format!(
            "鼓机夹具应武装 1 台鼓机，实际 {} 台",
            drum_runtime.armed_drum_slot_count()
        ));
    }
    if drum_runtime.armed_drums(&drum_track).is_none() {
        failures.push("鼓机轨必须武装进实时侧（复音轨不得武装）".to_owned());
    }

    // 覆盖度自检的**精确**期望：落在整窗里的起音数（由夹具栅格算出，不是"大于 0"）。
    // 窗口 = 预热 1 个量子 ＋ 下面 10,000 个量子 = 10,001 × 128 帧。
    let window_end = (10_001u64) * 128;
    let expected_drum_hits = (0..256u64)
        .filter(|index| index * 240 * 25 < window_end)
        .count();
    let mut drum_nonzero = 0usize;
    let (allocations, deallocations) = measure("drum instrument 10_000 quanta", || {
        for _ in 0..10_000 {
            drum_runtime.process_quantum(&mut drum_output, 2);
            for sample in &drum_output {
                if *sample != 0.0 {
                    drum_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨鼓机音源在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if drum_nonzero == 0 {
        failures.push("鼓机窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let drum_stats = drum_runtime.stats();
    if drum_stats.drum_hits != expected_drum_hits as u64 {
        failures.push(format!(
            "鼓击数={}（期望 {expected_drum_hits} = 窗口内的起音数）—— 这条零分配判据\
             没有覆盖整条触发路径",
            drum_stats.drum_hits
        ));
    }
    if drum_stats.insert_strip_frames != 0 {
        failures.push("鼓机是音源、不是插入器件：`insert_strip_frames` 必须为 0".to_owned());
    }
    println!(
        "[engine-drums/J11] 鼓机音源: quanta={} 鼓击={} 非零样本={drum_nonzero} 武装槽位={}",
        drum_stats.quanta,
        drum_stats.drum_hits,
        drum_runtime.armed_drum_slot_count(),
    );

    // ---- 场景 12：重新武装鼓机（同采样率 / 换采样率 / 换映射 / 换回复音合成器）----
    //
    // 四段各自对应一条真实路径：
    //   * 前 31 次交换 = **同一个采样率、同一份映射** ⇒ 一个字段都不写（状态全保留）；
    //   * 第 32 次 = **换采样率**（44.1 kHz）⇒ `DrumMachine::set_sample_rate` 重算在响
    //     槽位的系数。⚠ 这与混响**相反**（混响在那里拒绝武装）：鼓机没有延迟线，
    //     所以它必须能跟着采样率走，而且**不许分配**；
    //   * 第 33 次 = **换一套键位映射**（同一轨）⇒ 只 `set_params`（状态保留）；
    //   * 第 34 次 = **换回复音合成器** ⇒ 鼓机 `reset`、武装槽位归 0。
    let mut drum_switches = 0u64;
    for revision in 2..=32u64 {
        let next = EngineSnapshot::from_project(&drum_project, revision).expect("快照");
        drum_slot.publish(next);
        let (allocations, deallocations) = measure("drum re-arm + quantum", || {
            drum_runtime.process_quantum(&mut drum_output, 2);
        });
        if allocations != 0 || deallocations != 0 {
            failures.push(format!(
                "重新武装鼓机时实时路径分配/释放: allocations={allocations} deallocations={deallocations}（revision={revision}）"
            ));
        }
        drum_switches += drum_queue.drain(64) as u64;
    }
    if drum_switches == 0 {
        failures.push("鼓机场景没有从退役队列回收任何旧快照 —— 场景 12 是空转".to_owned());
    }
    if drum_runtime.armed_drum_slot_count() != 1 {
        failures.push("同采样率重新武装之后鼓机必须仍被武装".to_owned());
    }

    // 换采样率：鼓机照常武装（它没有需要重建的缓冲），且零分配。
    let mut shifted_drums = drum_project.clone();
    shifted_drums.audio_config.sample_rate = SampleRate::Hz44100;
    let shifted_drum_snapshot =
        EngineSnapshot::from_project(&shifted_drums, 33).expect("换采样率快照");
    drum_slot.publish(shifted_drum_snapshot);
    let (allocations, deallocations) = measure("drum rate-change re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换采样率重新武装鼓机时分配/释放: allocations={allocations} deallocations={deallocations}\
             —— `DrumMachine::set_sample_rate` 只重算系数，不许碰堆"
        ));
    }
    let armed_after_rate_change = drum_runtime.armed_drum_slot_count();
    if armed_after_rate_change != 1 {
        failures.push(format!(
            "换采样率之后鼓机必须仍被武装（对比：混响在同样的分支里拒绝），实际 {armed_after_rate_change} 台"
        ));
    }

    // 换回 48 kHz。
    let back_drum_snapshot = EngineSnapshot::from_project(&drum_project, 34).expect("换回快照");
    drum_slot.publish(back_drum_snapshot);
    let (allocations, deallocations) = measure("drum rate-restore re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换回 48 kHz 重新武装鼓机时分配/释放: allocations={allocations} deallocations={deallocations}"
        ));
    }

    // 换一套键位映射（同一轨）：整体上移一个八度。
    let mut remapped = drum_project.clone();
    {
        let entry = remapped
            .tracks
            .get_mut(&drum_track)
            .expect("夹具里必须有那条鼓机轨");
        entry.devices = vec![drum_device(&[], [48.0, 50.0, 54.0, 58.0, 51.0])];
    }
    let remapped_snapshot = EngineSnapshot::from_project(&remapped, 35).expect("换映射快照");
    drum_slot.publish(remapped_snapshot);
    let (allocations, deallocations) = measure("drum remap re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换键位映射重新武装鼓机时分配/释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    let remapped_params = drum_runtime.armed_drums(&drum_track);
    if remapped_params.map(|params| params.notes().kick()) != Some(48) {
        failures.push(format!(
            "换映射之后实时侧读到的底鼓音高不是 48：{remapped_params:?}"
        ));
    }

    // 换回复音合成器：鼓机必须被 `reset`、武装槽位归 0，且零分配。
    let mut synth_again = drum_project.clone();
    {
        let entry = synth_again
            .tracks
            .get_mut(&drum_track)
            .expect("夹具里必须有那条鼓机轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Hollow".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![ParameterValue {
                name: "cutoff_hz".to_owned(),
                value: 800.0,
                unit: Some("Hz".to_owned()),
            }],
            latency_samples: 0,
        }];
    }
    let synth_again_snapshot = EngineSnapshot::from_project(&synth_again, 36).expect("换回快照");
    drum_slot.publish(synth_again_snapshot);
    let (allocations, deallocations) = measure("drum -> poly synth re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "从鼓机换回复音合成器时分配/释放: allocations={allocations} deallocations={deallocations}\
             —— `DrumMachine::reset` 是赋值，不许释放"
        ));
    }
    let armed_after_swap = drum_runtime.armed_drum_slot_count();
    if armed_after_swap != 0 {
        failures.push(format!(
            "换回复音合成器之后鼓机必须不武装，实际仍武装 {armed_after_swap} 台"
        ));
    }
    if drum_runtime.armed_drums(&drum_track).is_some() {
        failures.push("换回复音合成器之后该轨的鼓机读数必须变 None".to_owned());
    }
    println!(
        "[engine-drums/J12] 鼓机重新武装: 同采样率交换={} 次；换采样率后武装槽位={}；\
         换映射后底鼓音高={:?}；换回复音后武装槽位={}",
        drum_switches,
        armed_after_rate_change,
        remapped_params.map(|params| params.notes().kick()),
        armed_after_swap,
    );

    // ---- 场景 13：**波形选择 + 第二条振荡器**在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景：器件（`yeban_dsp::polysynth`）早就是双振荡器，但引擎侧
    // 到本票才把 `osc1_wave` / `osc2_wave` / `osc2_level` / `osc2_detune_cents` 四个参数
    // 投影进 `ToneParams` ⇒ 这条**武装路径**（`poly_synth_params` → `set_params`）与
    // "第二条支路真的在逐样本路径上跑"都没有被既有场景走过（既有夹具的
    // `osc2_level` 恒为 0 ⇒ 第二条支路一帧也不执行）。
    //
    // 夹具：音符铺满窗口（同场景 1 的栅格）＋一条轨挂**只写振荡器键**的设备
    // （⇒ 滤波器旁通、音源是复音合成器、`osc1_wave = 4`（glass）、
    // `osc2` 开在电平 0.6、失谐 11 音分、波表 1（organ））。
    let osc_fixture = note_project(&saturated_notes());
    let mut osc_project = osc_fixture.project;
    let osc_track = osc_fixture.track;
    set_oscillators(&mut osc_project, osc_track, 4.0, 1.0, 0.6, 11.0);
    let osc_snapshot = EngineSnapshot::from_project(&osc_project, 1).expect("振荡器夹具快照");
    match osc_snapshot.tone(&osc_track) {
        Some(tone)
            if tone.osc1_wave() == 4 && tone.osc2_wave() == 1 && tone.osc2_level() == 0.6 =>
        {
            if !tone.is_bypass() {
                failures.push("只有振荡器键的夹具必须是滤波器旁通".to_owned());
            }
        }
        other => failures.push(format!("夹具快照的振荡器投影不对：{other:?}")),
    }
    let osc_slot = SnapshotSlot::new(osc_snapshot);
    let (osc_retire, mut osc_queue) = retire_channel(64);
    let (_osc_sender, osc_receiver) = event_channel(64);
    let (osc_publisher, _osc_collector) = meter_channel(8192);
    let mut osc_runtime = EngineRuntime::new(&osc_slot, osc_retire, osc_receiver, osc_publisher);
    let mut osc_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`PolySynth::set_params`，含 `tan`）与首个量子。
    osc_runtime.process_quantum(&mut osc_output, 2);
    if osc_runtime
        .armed_tone(&osc_track)
        .map(|tone| tone.osc1_wave())
        != Some(4)
    {
        failures.push("预热之后实时侧必须已经武装 glass（下标 4）".to_owned());
    }

    let mut osc_nonzero = 0usize;
    let (allocations, deallocations) =
        measure("oscillator (waveform + 2nd osc) 10_000 quanta", || {
            for _ in 0..10_000 {
                osc_runtime.process_quantum(&mut osc_output, 2);
                for sample in &osc_output {
                    if *sample != 0.0 {
                        osc_nonzero += 1;
                    }
                }
            }
        });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "波形选择 / 第二条振荡器在实时窗口内分配/释放了内存: \
             allocations={allocations} deallocations={deallocations}"
        ));
    }
    if osc_nonzero == 0 {
        failures
            .push("波形/第二条支路窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let osc_stats = osc_runtime.stats();
    if osc_stats.notes_triggered < 200 {
        failures.push(format!(
            "振荡器窗口只触发了 {} 个音符，未达到压测规模（应 ≥ 200）",
            osc_stats.notes_triggered
        ));
    }

    // ---- 场景 14：**重新武装振荡器参数**（波形下标 / 波表 / 电平 / 失谐）----
    //
    // 每一轮都换一次 `osc1_wave` 与 `osc2_wave`：覆盖 `EngineSnapshot::from_project`
    // 的投影、`begin_snapshot` 的 `tone != slot.tone` 分支与器件的 `set_params`。
    // 覆盖度见证有两条：
    //   ① 每一轮的实时侧读数（`armed_tone`）等于发布的波形下标；
    //   ② 循环之后的**差分**检查：两台新建的实时侧只差一个 `osc1_wave`，输出指纹必须不同
    //      （见下面的注释：从**同一台** runtime 的相邻量子比指纹是**没有牙的**）。
    let mut osc_switches = 0u64;
    let mut osc_rearms_matching = 0u64;
    for revision in 2..=32u64 {
        let osc1_wave = (revision % 5) as f32;
        let osc2_wave = ((revision + 2) % 5) as f32;
        set_oscillators(&mut osc_project, osc_track, osc1_wave, osc2_wave, 0.6, 11.0);
        let next = EngineSnapshot::from_project(&osc_project, revision).expect("快照");
        osc_slot.publish(next);
        let (allocations, deallocations) = measure("oscillator re-arm + quantum", || {
            osc_runtime.process_quantum(&mut osc_output, 2);
        });
        if allocations != 0 || deallocations != 0 {
            failures.push(format!(
                "重新武装振荡器参数时实时路径分配/释放: \
                 allocations={allocations} deallocations={deallocations}（revision={revision}）"
            ));
        }
        match osc_runtime.armed_tone(&osc_track) {
            Some(tone) if u32::from(tone.osc1_wave()) == revision as u32 % 5 => {
                osc_rearms_matching += 1;
            }
            other => failures.push(format!(
                "重新武装后波形下标不是 {}：{other:?}（revision={revision}）",
                revision % 5
            )),
        }
        osc_switches += osc_queue.drain(64) as u64;
    }
    if osc_switches == 0 {
        failures.push("振荡器场景没有从退役队列回收任何旧快照 —— 场景 14 是空转".to_owned());
    }
    if osc_rearms_matching != 31 {
        failures.push(format!(
            "31 次重新武装里只有 {osc_rearms_matching} 次的实时侧波形下标与发布值一致"
        ));
    }

    // ⚠ **差分**覆盖度见证（这一条才有牙）。
    //
    // 第一版写的是"同一台 runtime 的相邻量子指纹必须变化"，本票的注入 I1
    // （`poly_synth_params` 丢掉 `.with_oscillators(..)`）实测**不红** —— 因为相邻量子
    // 之间声部状态本来就在推进（相位、包络、失谐拍频），指纹变化与波形无关。
    // 改成"两台**新建**的实时侧只差一个 `osc1_wave`"：声部状态从零开始、逐量子同步推进，
    // 唯一的差别就是波表 ⇒ 指纹不同才是"波形真的到了器件"。I1 下这一条变红（实测）。
    let low = osc_render_fingerprint(0.0, 8);
    let high = osc_render_fingerprint(4.0, 8);
    if low == high {
        failures.push(
            "只差一个 `osc1_wave` 的两台实时侧给出了相同的输出指纹 —— 波形参数没有真的到器件"
                .to_owned(),
        );
    }
    println!(
        "[engine-osc/J13] 波形选择 + 第二条振荡器: quanta={} 非零样本={osc_nonzero} \
         重新武装={osc_rearms_matching}/31 退役回收={osc_switches} 次；\
         差分指纹 wave0={low:#018x} wave4={high:#018x}",
        osc_stats.quanta,
    );

    println!(
        "[engine-sound/J5] 汇总: quanta={} scheduled_notes={} notes_triggered={} voice_steals={} \
         非零样本={nonzero} 峰值={peak:.6} filled(nonzero={filled_nonzero}, scheduled={}, triggered={})",
        stats.quanta,
        stats.scheduled_notes,
        stats.notes_triggered,
        stats.voice_steals,
        filled_stats.scheduled_notes,
        filled_stats.notes_triggered,
    );

    if failures.is_empty() {
        println!(
            "[engine-sound/J5] ok: 10,000 量子（音符铺满窗口）+ 63 次快照交换 + \
             filled_project 4,000 量子 + 2,000 量子整条混音链 + 10,000 量子每轨插入压缩器 \
             + 31 次插入链重新武装 + 10,000 量子每轨插入通道条（EQ＋滤波＋动态） \
             + 31 次通道条重新武装 + 10,000 量子每轨插入混响（两条轨，含通道条＋混响同一台设备） \
             + 31 次混响重新武装 + 换采样率时的拒绝路径 \
             + 10,000 量子每轨鼓机音源（两条轨，一条鼓机＋一条复音） \
             + 31 次鼓机重新武装 + 换采样率 / 换键位映射 / 换回复音合成器 \
             + 10,000 量子波形选择＋第二条振荡器 + 31 次振荡器重新武装，实时窗口内零分配零释放"
        );
        ExitCode::SUCCESS
    } else {
        for failure in &failures {
            eprintln!("[engine-sound/J5] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}
