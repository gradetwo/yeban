//! 电平线的**运行期判据**（`harness = false`）：真峰值/RMS 计算、每量子一次批量发布、
//! UI 侧"丢弃旧帧只取最新"、以及"带真实电平的量子仍然零分配/零释放"。
//! [ARCH-UI-002, ROAD-M2-008, MUST-GATE-001]
//!
//! # 为什么也是 `harness = false`
//!
//! 计数型分配器是**进程全局**的（`#[global_allocator]` 每个二进制只能定义一次）。
//! libtest 自己会在别的线程里起线程/收结果/打印 ⇒ 窗口内必然出现不属于实时路径的分配，
//! 判据会**测错对象**（教训 L22 的实测：`allocations=9 deallocations=3`）。
//! 于是本目标提供自己的 `main()`，用退出码判定；`cargo test --all-targets` 仍会跑到它。
//!
//! # 场景（单线程顺序跑，数字无歧义）
//!
//! | # | 判据 | 变红方式（注入） |
//! | :-: | :--- | :--- |
//! | S1 | 带真实电平的 10,000 量子零分配 + 零释放 | 在 `render_block` 里 `Vec::with_capacity(1)` |
//! | S2 | 每量子**恰好一次**批量发布，帧数 = 非母线轨数 + 1 | 每轨单独 `publish` 一次 |
//! | S3 | 静音 ⇒ 峰值 0、dBFS 负无穷、无 `NaN`（输入用 `silent_project`） | 用 `0/0` 求 RMS、或删掉钳位 |
//! | S4 | 消费者落后时只取最新，旧帧永不覆盖新帧 | 把 `supersedes` 改成 `true`/`>` 取反 |
//! | S5 | 满幅正弦 ⇒ 峰值 ≈ 0 dBFS；幅度单调；`NaN`/`Inf` 不产生 `NaN` | 删掉 `sanitize_sample` |
//! | S6 | 队列溢出**可观测**：`dropped > 0` 且 UI 看到的 quantum 落后于生产者 | 把 dropped 计数删掉 |
//! | S7 | `drain_latest` 抽干整批且自身零分配 | 在 collector 里分配临时 `Vec` |
//!
//! 覆盖度自检（S1 末尾）：quanta / 帧数必须真的达到压测规模，避免"窗口里什么都没跑"的假绿。

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::level::{self, LevelDetector};
use yeban_engine::meter::{
    MeterBank, MeterBoard, MeterCollector, MeterFrame, MeterPublisher, SCRATCH_METERS,
    meter_channel,
};
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::EntityId;

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

/// `true` 时才开始计数 —— 只在被测窗口内打开。
static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法都只是"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有任何跨调用状态（计数器是原子量）。
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
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// 在"必须零分配"的窗口内执行 `body`，返回 `(allocations, deallocations)`。
fn measure<F: FnOnce()>(label: &str, body: F) -> (usize, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    body();
    ARMED.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let deallocations = DEALLOCATIONS.load(Ordering::SeqCst);
    println!("[meter-rt] {label}: allocations={allocations} deallocations={deallocations}");
    (allocations, deallocations)
}

/// 累加失败项；任一失败 ⇒ 退出码非零。
struct Report {
    failures: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Self {
            failures: Vec::new(),
        }
    }

    fn check(&mut self, condition: bool, message: impl Into<String>) {
        if !condition {
            self.failures.push(message.into());
        }
    }

    fn expect_zero(&mut self, label: &str, counts: (usize, usize)) {
        let (allocations, deallocations) = counts;
        self.check(
            allocations == 0,
            format!(
                "[{label}] 实时窗口内发生了 {allocations} 次堆分配 —— [MUST-GATE-001] 一票否决"
            ),
        );
        self.check(
            deallocations == 0,
            format!(
                "[{label}] 实时窗口内发生了 {deallocations} 次堆释放 —— [MUST-GATE-001] 一票否决"
            ),
        );
    }
}

/// 与产品路径一致的装配：**所有通道在"打开设备之前"建立**，回调内零分配。
fn engine_rig() -> (
    Arc<SnapshotSlot>,
    yeban_engine::snapshot::RetireQueue,
    MeterCollector,
    EngineRuntime,
) {
    engine_rig_with(&yeban_model::samples::filled_project())
}

/// 同 [`engine_rig`]，但夹具由调用方给定。
fn engine_rig_with(
    project: &yeban_model::YebanProjectV1,
) -> (
    Arc<SnapshotSlot>,
    yeban_engine::snapshot::RetireQueue,
    MeterCollector,
    EngineRuntime,
) {
    let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, collector) = meter_channel(8192);
    let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    (slot, queue, collector, runtime)
}

/// **真正静音**的夹具：`filled_project` 去掉全部 MIDI 音符（保留轨道/路由/片段）。
///
/// ⚠ 为什么必须显式清空音符（`line/engine-sound` 的交叉发现）：
/// 本文件此前用 `filled_project()` 当"静音输入"，那是**建立在渲染占位静音这个前提上的**
/// ——`render_track_into` 曾经写 `out.fill(0.0)`，所以"夹具工程的前 128 帧"恒为静音。
/// `line/engine-sound` 把真实合成接上之后，`filled_project` 的第 0 个音符恰好在
/// tick 0 起音 ⇒ S3 会立刻变红，而**红的原因不是电平口径坏了**，是夹具不再静音。
/// 判据的意图是"静音输入 ⇒ 峰值 0 / 有限 / 无 NaN / dBFS 负无穷"，
/// 因此这里把输入改成**真的**静音；S3 的判别力不变，反而更强
/// （旧版测的是"占位渲染恰好静音"，新版测的是"真实合成链上的静音"）。
fn silent_project() -> yeban_model::YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    for entry in project.clip_pool.values_mut() {
        if let Some(notes) = entry.content.notes_mut() {
            notes.clear();
        }
    }
    project
}

/// 一段满幅正弦（测试信号；引擎本身不生成信号，只测量）。
fn sine(amplitude: f32, cycles: f32, len: usize) -> Vec<f32> {
    (0..len)
        .map(|index| {
            let phase = cycles * std::f32::consts::TAU * index as f32 / len as f32;
            amplitude * phase.sin()
        })
        .collect()
}

/// S1 + 覆盖度：带真实电平的 10,000 个量子零分配/零释放；发布次数与帧数对得上。
fn scenario_zero_alloc_with_real_meters(report: &mut Report) {
    let (slot, mut queue, mut collector, mut runtime) = engine_rig();
    let mut output = vec![0.0f32; 128 * 2];
    let mut scratch = [MeterFrame::default(); SCRATCH_METERS];

    // 预热：一次性的惰性路径（FPU 武装 / 弹道系数设置）不算在实时路径头上。
    runtime.process_quantum(&mut output, 2);
    runtime.process_quantum(&mut output, 2);

    // 40 轮 × 256 个量子 = 10,240 个量子；每轮之间在**窗口外**把 UI 队列抽干。
    let mut windows = 0usize;
    for round in 0..40 {
        // 每两轮换一次快照（控制线程侧允许分配），逼实时路径在切换中保持零分配。
        if round % 2 == 1 && round < 20 {
            let project = yeban_model::samples::filled_project();
            let next = EngineSnapshot::from_project(&project, round as u64 + 2).expect("快照");
            slot.publish(next);
        }
        let counts = measure("metered quantum x256", || {
            for _ in 0..256 {
                runtime.process_quantum(&mut output, 2);
            }
        });
        report.expect_zero("S1", counts);
        windows += 1;
        // 窗口外：UI 抽干（丢弃旧帧、只取最新）+ 主线程回收旧快照（都允许分配）。
        let _ = collector.drain_latest(&mut scratch);
        queue.drain(64);
        slot.prune();
    }

    let stats = runtime.stats();
    let quanta = stats.quanta;
    let expected_frames = quanta * 4; // 3 条普通轨 + 1 条母线
    report.check(
        quanta >= 10_000,
        format!("S1 覆盖度不足: 只处理了 {quanta} 个量子 (< 10,000)"),
    );
    report.check(
        stats.meter_bulk_publishes == quanta,
        format!(
            "S1 每量子必须恰好一次批量发布: quanta={quanta} publishes={}",
            stats.meter_bulk_publishes
        ),
    );
    report.check(
        stats.meter_frames == expected_frames,
        format!(
            "S1 帧数应为 (轨道数+母线)×量子数 = {expected_frames}, 实际 {}",
            stats.meter_frames
        ),
    );
    report.check(
        stats.meter_capacity_drops == 0,
        format!("S1 不应发生容量丢弃, 实际 {}", stats.meter_capacity_drops),
    );
    report.check(windows == 40, "S1 必须真的跑了 40 个测量窗口");
    println!(
        "[meter-rt] S1 汇总: quanta={quanta} publishes={} frames={} capacity_drops={}",
        stats.meter_bulk_publishes, stats.meter_frames, stats.meter_capacity_drops
    );
}

/// S2：每量子**一次**批量发布；帧数 = 非母线轨数 + 1；节点顺序确定。
fn scenario_publish_contract_per_quantum(report: &mut Report) {
    let (_slot, _queue, mut collector, mut runtime) = engine_rig();
    let mut output = vec![0.0f32; 128 * 2];
    let quanta = 7u64;
    for _ in 0..quanta {
        runtime.process_quantum(&mut output, 2);
    }
    let stats = runtime.stats();
    report.check(
        stats.meter_bulk_publishes == quanta,
        format!(
            "S2 每个量子恰好一次批量发布: 期望 {quanta}, 实际 {}",
            stats.meter_bulk_publishes
        ),
    );
    report.check(
        stats.meter_frames == quanta * 4,
        format!(
            "S2 帧数应为 4×{quanta} = {}, 实际 {}",
            quanta * 4,
            stats.meter_frames
        ),
    );

    let mut scratch = [MeterFrame::default(); SCRATCH_METERS];
    let drained = collector.tick(&mut scratch);
    report.check(
        drained == (quanta * 4) as usize,
        format!("S2 UI 一次 tick 应抽到 {} 条, 实际 {drained}", quanta * 4),
    );
    report.check(
        collector.bulk_pop_calls() == 1,
        format!(
            "S2 UI 每 tick 恰好一次批量读: 实际 {}",
            collector.bulk_pop_calls()
        ),
    );

    // 每个量子内: 4 条帧的 quantum 相同, 且节点互不相同(母线只出现一次)
    let first_batch = &scratch[..4];
    let quantum = first_batch[0].quantum;
    report.check(
        first_batch.iter().all(|frame| frame.quantum == quantum),
        "S2 同一量子的帧必须共享 quantum",
    );
    let mut nodes: Vec<EntityId> = first_batch.iter().map(|frame| frame.node).collect();
    nodes.sort_unstable();
    nodes.dedup();
    report.check(
        nodes.len() == 4,
        format!("S2 4 条帧必须来自 4 个不同节点, 实际 {}", nodes.len()),
    );
    let mut quanta_seen: Vec<u64> = scratch[..drained].iter().map(|f| f.quantum).collect();
    quanta_seen.dedup();
    report.check(
        quanta_seen == (1..=quanta).collect::<Vec<u64>>(),
        "S2 量子序号必须严格递增且每量子一批",
    );
}

/// S3：静音 ⇒ 峰值 0、dBFS 负无穷、静音下限有限、无 NaN。
fn scenario_silence_is_finite(report: &mut Report) {
    // 输入是**真的**静音夹具（不是"填充工程恰好被占位渲染成静音"，见 `silent_project`）。
    let (_slot, _queue, mut collector, mut runtime) = engine_rig_with(&silent_project());
    let mut output = vec![0.0f32; 128 * 2];
    runtime.process_quantum(&mut output, 2);
    let mut scratch = [MeterFrame::default(); 8];
    let drained = collector.tick(&mut scratch);
    report.check(drained == 4, format!("S3 应抽到 4 条, 实际 {drained}"));
    for frame in &scratch[..drained] {
        report.check(frame.is_sane(), format!("S3 静音帧不得含 NaN: {frame:?}"));
        report.check(frame.peak == 0.0, "S3 静音峰值必须是 0");
        report.check(
            frame.peak_dbfs() == f32::NEG_INFINITY,
            "S3 静音峰值 dBFS 必须是负无穷",
        );
        report.check(
            frame.peak_dbfs_clamped(level::SILENCE_FLOOR_DBFS) == level::SILENCE_FLOOR_DBFS,
            "S3 UI 侧应拿到静音下限(有限值)",
        );
    }
}

/// S4 + S6：UI 侧"丢弃旧帧、只取最新"；消费者落后/队列溢出时旧帧不得冒充新帧，
/// 且溢出必须可观测。
fn scenario_ui_latest_wins_and_overflow_is_observable(report: &mut Report) {
    let node_a = EntityId::new();
    let node_b = EntityId::new();

    // --- S4a: 消费者落后但队列未溢出 ⇒ drain_latest 只留最新 ---
    let (mut publisher, mut collector) = meter_channel(4096);
    let quanta = 400u64;
    for quantum in 0..quanta {
        let frames = [
            MeterFrame::new(node_a, quantum, 0.1, 0.05),
            MeterFrame::new(node_b, quantum, 0.2, 0.1),
        ];
        publisher.publish(&frames);
    }
    report.check(
        publisher.dropped() == 0,
        format!("S4a 未溢出时不应丢帧, 实际 {}", publisher.dropped()),
    );
    let mut scratch = [MeterFrame::default(); 8];
    let unique = collector.drain_latest(&mut scratch);
    report.check(unique == 2, format!("S4a 应留 2 个节点, 实际 {unique}"));
    report.check(collector.pending() == 0, "S4a drain_latest 必须把积压抽干");
    for frame in &scratch[..unique] {
        report.check(
            frame.quantum == quanta - 1,
            format!("S4a 取到的必须是最后一个量子, 实际 {}", frame.quantum),
        );
    }

    // --- S4b: 乱序/迟到的旧帧不得覆盖新值 ---
    let mut board = MeterBoard::new();
    board.ingest(&scratch[..unique]);
    let updated = board.ingest(&[MeterFrame::new(node_a, 3, 1.0, 1.0)]);
    report.check(updated == 0, "S4b 迟到的旧帧不得更新 UI");
    report.check(
        board.latest(&node_a).map(|f| f.quantum) == Some(quanta - 1),
        "S4b UI 必须保持最新量子",
    );

    // --- S6: 队列溢出 ⇒ 丢的是**最新**帧, 因此 dropped>0 且 UI 看到的量子落后 ---
    let (mut publisher, mut collector) = meter_channel(8);
    let offered = 20u64;
    let mut written = 0usize;
    for quantum in 0..offered {
        written += publisher.publish(&[MeterFrame::new(node_a, quantum, 0.5, 0.25)]);
    }
    report.check(
        written == 8,
        format!("S6 容量 8 只能写入 8 条, 实际 {written}"),
    );
    report.check(
        publisher.dropped() == offered - 8,
        format!(
            "S6 丢弃必须被计数: 期望 {}, 实际 {}",
            offered - 8,
            publisher.dropped()
        ),
    );
    let mut scratch = [MeterFrame::default(); 4];
    let unique = collector.drain_latest(&mut scratch);
    report.check(unique == 1, "S6 只应有一个节点");
    let seen = scratch[0].quantum;
    report.check(
        seen == 7,
        format!("S6 队列里只剩前 8 帧, 最新可见量子应为 7, 实际 {seen}"),
    );
    report.check(
        seen < offered - 1,
        "S6 UI 必须能看出自己不是最新(dropped>0 且 quantum 落后)",
    );
    report.check(
        collector.frames_drained() == 8,
        format!("S6 应消费 8 帧, 实际 {}", collector.frames_drained()),
    );
}

/// S5 + S7：真峰值/RMS 口径（正弦/单调/NaN）+ `drain_latest` 自身零分配。
fn scenario_real_levels_and_zero_alloc_consumer(report: &mut Report) {
    let node = EntityId::new();

    // 实时路径每轨调用的**同一个函数**: MeterBank::measure。
    let mut bank = MeterBank::<8>::new();
    bank.begin_quantum();
    let held = bank
        .measure(node, 0, &sine(1.0, 10.0, 4800))
        .expect("容量足够");
    report.check(
        held.is_sane() && held.peak_dbfs().abs() < 0.1,
        format!("S5 满幅正弦峰值应 ≈ 0 dBFS, 实际 {}", held.peak_dbfs()),
    );
    report.check(
        (level::dbfs(held.rms) + 3.0103).abs() < 0.05,
        format!(
            "S5 正弦块 RMS 应 ≈ -3.01 dBFS, 实际 {}",
            level::dbfs(held.rms)
        ),
    );

    // 幅度单调
    let mut previous = f32::NEG_INFINITY;
    for amplitude in [0.01f32, 0.1, 0.5, 1.0] {
        let mut detector = LevelDetector::new();
        let reading = detector.analyze(&sine(amplitude, 10.0, 4800));
        report.check(
            reading.peak_dbfs() > previous,
            format!("S5 电平必须随幅度单调增: {amplitude}"),
        );
        previous = reading.peak_dbfs();
    }

    // NaN / Inf 钳位
    bank.begin_quantum();
    let hostile = bank
        .measure(node, 1, &[f32::NAN, f32::INFINITY, -1.0e30, 0.25])
        .expect("容量足够");
    report.check(
        hostile.is_sane(),
        format!("S5 NaN/Inf 输入必须被钳位: {hostile:?}"),
    );
    report.check(
        hostile.peak <= level::MAX_LINEAR_MAGNITUDE && hostile.peak_dbfs().is_finite(),
        "S5 钳位后的峰值必须有限且有界",
    );

    // S7: drain_latest 抽干 4096 条帧而自身零分配
    let (mut publisher, mut collector) = meter_channel(4096);
    let frames: Vec<MeterFrame> = (0..2048u64)
        .map(|quantum| MeterFrame::new(node, quantum, 0.5, 0.25))
        .collect();
    publisher.publish(&frames);
    let mut scratch = [MeterFrame::default(); 4];
    let mut unique = 0usize;
    let counts = measure("drain_latest x2048", || {
        unique = collector.drain_latest(&mut scratch);
    });
    report.expect_zero("S7", counts);
    report.check(unique == 1 && scratch[0].quantum == 2047, "S7 只留最新一帧");
}

fn main() -> ExitCode {
    let mut report = Report::new();

    scenario_zero_alloc_with_real_meters(&mut report);
    scenario_publish_contract_per_quantum(&mut report);
    scenario_silence_is_finite(&mut report);
    scenario_ui_latest_wins_and_overflow_is_observable(&mut report);
    scenario_real_levels_and_zero_alloc_consumer(&mut report);

    if report.failures.is_empty() {
        println!(
            "[ARCH-UI-002] ok: 真峰值/RMS 计量 + 每量子一次批量发布 + UI 取最新 + 10,000 量子零分配"
        );
        ExitCode::SUCCESS
    } else {
        for failure in &report.failures {
            eprintln!("[ARCH-UI-002] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}

/// 编译期提醒：消费端类型必须能在 UI 线程里独立存在（不借用音频线程状态）。
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<MeterPublisher>();
    assert_send::<MeterCollector>();
    assert_send::<MeterBoard>();
    let _ = SCRATCH_METERS;
};
