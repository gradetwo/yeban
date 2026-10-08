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

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};

mod support;

use support::{MixSpec, NoteSpec, note_project, tuned_project};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue};

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
             + 31 次插入链重新武装，实时窗口内零分配零释放"
        );
        ExitCode::SUCCESS
    } else {
        for failure in &failures {
            eprintln!("[engine-sound/J5] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}
