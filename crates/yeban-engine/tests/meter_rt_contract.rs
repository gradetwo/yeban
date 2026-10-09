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
//! | S8 | 丢帧读数在**运行时路径**上可读（`EngineStats::meter_dropped_frames`），且 `meter_frames + dropped == 本应发布帧数`；两个臂的窗口都零分配 | 把 `stats()` 的该字段写死 `0`，或把它接到 `meter_capacity_drops` |
//! | S9 | **换采样率之后才第一次被计量的节点**必须用新采样率的弹道：96 kHz 下"只发布一份快照"与"再发布一份等价快照"的电平帧逐位相同 | 让 `MeterSlot::fresh` 回到 `LevelDetector::new()`（器件默认 375 量子/s） |
//! | S10 | 母线电平**立体声联动**：只有**右**声道有信号时峰值/均方必须按那个声道报数（`line/engine-27` 追加） | 把 `measure_bus_stereo` 的 `analyze_stereo(left, right)` 换成 `analyze(left)`（实测变红，见该场景文档） |
//! | S11 | **快照的采样率必须转发到电平池**：96 kHz 的每量子释放乘子必须是 48 kHz 的**平方根**（`line/engine-27` 追加） | 把 `render_block` 的 `bank.set_quanta_per_second(quanta_per_second)` 写死 `375.0`（实测变红，见该场景文档） |
//!
//! 覆盖度自检（S1 末尾）：quanta / 帧数必须真的达到压测规模，避免"窗口里什么都没跑"的假绿。

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::level::{self, LevelDetector};
use yeban_engine::meter::{
    MeterBank, MeterBoard, MeterCollector, MeterFrame, MeterPublisher, SCRATCH_METERS,
    meter_channel,
};
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{EntityId, SampleRate};

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

/// 同 [`engine_rig`]，但**电平 SPSC 的容量**由调用方给定。
///
/// 容量 1 是"制造 UI 落后"的标准夹具：一个量子要发"非母线轨数 + 1"帧，
/// 控制面不抽干 ⇒ 第二个量子起环就是满的 ⇒ 每量子都有帧被丢。
fn engine_rig_with_meter_capacity(
    meter_capacity: usize,
) -> (
    Arc<SnapshotSlot>,
    yeban_engine::snapshot::RetireQueue,
    MeterCollector,
    EngineRuntime,
) {
    let snapshot =
        EngineSnapshot::from_project(&yeban_model::samples::filled_project(), 1).expect("夹具快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, collector) = meter_channel(meter_capacity);
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

/// S8：**电平丢帧的读数**（`EngineStats::meter_dropped_frames`）在**完整运行时路径**上可读，
/// 而且它是"本应发布的帧数"的精确分解。
///
/// 与 S6 的区别：S6 直接操作 `MeterPublisher`（**发布侧句柄在手**）；S8 走
/// `EngineRuntime::process_quantum`，读数只从 `EngineStats` 出 —— 那才是设备腿的形态
/// （`EngineRuntime` 归 cpal 回调线程所有，控制面只能读 `stats()` / 镜像）。
///
/// 两个测量臂：
/// * **臂 A**（容量 4096、不抽干）⇒ 零丢帧，它给出独立的基线 `本应发布的帧数`；
/// * **臂 B**（容量 1、不抽干）⇒ 必须丢帧（覆盖度见证），且
///   `meter_frames + meter_dropped_frames == 基线`（**等号**，无容差）。
///
/// 两个臂的窗口都用 [`measure`] 包住 ⇒ "环满时的发布路径"仍然零分配、零释放。
fn scenario_meter_drop_readout(report: &mut Report) {
    const QUANTA: usize = 64;
    let mut output = [0.0f32; 128 * 2];

    // --- 臂 A：容量足够 ⇒ 零丢帧（"读数不假警报"的对照）---
    let (_slot_a, _queue_a, _collector_a, mut generous) = engine_rig_with_meter_capacity(4096);
    let counts = measure("S8a 容量 4096 不抽干", || {
        for _ in 0..QUANTA {
            generous.process_quantum(&mut output, 2);
        }
    });
    report.expect_zero("S8a", counts);
    let baseline = generous.stats();
    report.check(
        baseline.meter_dropped_frames == 0 && !baseline.is_meter_lagging(),
        format!(
            "S8 容量足够时不许报丢帧: meter_dropped_frames={} is_meter_lagging={}",
            baseline.meter_dropped_frames,
            baseline.is_meter_lagging()
        ),
    );
    report.check(
        baseline.meter_frames > 0,
        "S8 覆盖度：窗口里必须真的发布了电平帧".to_owned(),
    );

    // --- 臂 B：容量 1 ⇒ 每量子丢帧 ---
    let (_slot_b, _queue_b, _collector_b, mut starved) = engine_rig_with_meter_capacity(1);
    let counts = measure("S8b 容量 1 不抽干", || {
        for _ in 0..QUANTA {
            starved.process_quantum(&mut output, 2);
        }
    });
    report.expect_zero("S8b", counts);
    let stats = starved.stats();
    println!(
        "[meter-rt] S8 丢帧读数: 基线的本应发布帧数={} 臂B写入={} 臂B丢弃={} \
         capacity_drops={} quanta={}",
        baseline.meter_frames,
        stats.meter_frames,
        stats.meter_dropped_frames,
        stats.meter_capacity_drops,
        stats.quanta
    );
    report.check(
        stats.meter_dropped_frames > 0,
        format!(
            "S8 容量 1 + 不抽干必须丢帧, 实际 {}",
            stats.meter_dropped_frames
        ),
    );
    report.check(
        stats.meter_frames + stats.meter_dropped_frames == baseline.meter_frames,
        format!(
            "S8 本应发布的帧数 = 写进队列 + 丢掉: {} + {} ≠ 基线 {}",
            stats.meter_frames, stats.meter_dropped_frames, baseline.meter_frames
        ),
    );
    // 两个失败面**不是**同一件事：SPSC 环满（发布批次放得下）与批次容量不足
    // （`meter_capacity_drops`）在同一个窗口里必须能被分开读出来。
    report.check(
        stats.meter_capacity_drops == 0
            && stats.meter_bulk_publishes == baseline.meter_bulk_publishes,
        format!(
            "S8 丢帧来自 SPSC 环满, 不是批次容量不足: capacity_drops={} bulk_publishes={}/{}",
            stats.meter_capacity_drops, stats.meter_bulk_publishes, baseline.meter_bulk_publishes
        ),
    );
    report.check(
        stats.is_meter_lagging() && stats.is_meter_lagging_since(&baseline),
        "S8 粘滞判定与增量判定都必须为真".to_owned(),
    );
    report.check(
        stats.meter_dropped_since_last_read(&baseline) == stats.meter_dropped_frames
            && stats.meter_dropped_since_last_read(&stats) == 0,
        "S8 增量判定: 对更早基线给全量, 对自己的基线给 0".to_owned(),
    );
    // 设备腿形态：跨线程只读镜像必须带上这个字段（静止点上与权威读数逐字段相等）。
    let mirror = starved.stats_mirror();
    report.check(
        mirror.read() == stats,
        format!(
            "S8 镜像必须与权威读数逐字段相等（含 meter_dropped_frames={}）",
            stats.meter_dropped_frames
        ),
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

/// S9：换采样率之后**才**第一次被计量的节点，其弹道系数必须跟着采样率。
///
/// 量什么：96 kHz 工程下同一条轨的 `peak_hold` / `rms_smoothed`（单位：线性幅度），
/// 以及整个输出块的 `f32` **位模式**（对照基线的有效性见证）。
///
/// 两条臂的**唯一**差别是"快照有没有在中间被重新发布一次"：
/// * 臂 A：一份 96 kHz 快照（修订 1）跑完 400 个量子；
/// * 臂 B：同一份工程，在第 0 个量子之后发布**等价**的第二份（修订 2）
///   ⇒ 快照边界再跑一次 `MeterBank::set_quanta_per_second`。
///
/// 等价快照不改变渲染输出（两臂的输出位模式必须逐位相等 —— 本判据同时钉住这一点），
/// 因此两臂的电平帧**必须逐位相同**：一条轨的弹道不该取决于"它的槽位是在哪一次修订
/// 被建立的"。旧实现里 `MeterSlot::fresh` 用 `LevelDetector::new()`（硬编码
/// 375 量子/s = 48 kHz）⇒ 臂 A 的弹道按 40 dB/s（而不是契约的 20 dB/s）走，
/// 而臂 B 在重发布时被刷成 750 量子/s ⇒ 本判据在重发布之后的第一个量子
/// （`MeterFrame::quantum = 2`，实测 `rms_smoothed` 0.007567941 vs 0.0055855257）就红。
fn scenario_ballistics_follow_the_armed_sample_rate(report: &mut Report) {
    const QUANTA: usize = 400;

    /// 跑一条臂：96 kHz 夹具 + 可选"第 0 个量子之后重新发布一份等价快照"。
    ///
    /// 返回 `(每量子的该轨电平帧, 全部输出样本的位模式, 每个量子都拿到帧了吗)`。
    fn run(republish_after_first: bool) -> (Vec<MeterFrame>, Vec<u32>, bool) {
        let mut project = yeban_model::samples::filled_project();
        project.audio_config.sample_rate = SampleRate::Hz96000;
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("96 kHz 夹具快照");
        let track = *snapshot
            .tracks()
            .keys()
            .find(|id| **id != snapshot.master())
            .expect("夹具里必须有至少一条普通轨");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, _queue) = retire_channel(64);
        let (_sender, receiver) = event_channel(64);
        let (publisher, mut collector) = meter_channel(8192);
        let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);

        let mut output = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        let mut frames = Vec::with_capacity(QUANTA);
        let mut bits = Vec::with_capacity(QUANTA * DEFAULT_BLOCK_FRAMES * 2);
        let mut metered_every_quantum = true;
        for quantum in 0..QUANTA {
            runtime.process_quantum(&mut output, 2);
            if republish_after_first && quantum == 0 {
                // 发布在**实时测量窗口之外**：控制线程允许分配。
                let next = EngineSnapshot::from_project(&project, 2).expect("等价快照");
                slot.publish(next);
            }
            bits.extend(output.iter().map(|sample| sample.to_bits()));
            let mut scratch = [MeterFrame::default(); SCRATCH_METERS];
            let drained = collector.tick(&mut scratch);
            match scratch[..drained].iter().find(|frame| frame.node == track) {
                Some(frame) => frames.push(*frame),
                None => metered_every_quantum = false,
            }
        }
        (frames, bits, metered_every_quantum)
    }

    let (single, bits_single, single_metered) = run(false);
    let (republished, bits_republished, republished_metered) = run(true);

    report.check(
        single_metered && republished_metered,
        "S9 覆盖度：每个量子都必须产出该轨的电平帧（槽位真的被建立并被计量）",
    );
    report.check(
        single.len() == QUANTA && republished.len() == QUANTA,
        format!(
            "S9 覆盖度：两臂各应有 {QUANTA} 条轨电平帧，实测 {} / {}",
            single.len(),
            republished.len()
        ),
    );
    report.check(
        bits_single == bits_republished,
        "S9 对照基线：重新发布一份等价快照不得改变渲染输出（否则两臂的读数不可比）",
    );
    if single.len() == QUANTA && republished.len() == QUANTA {
        let mut divergence: Option<(usize, MeterFrame, MeterFrame)> = None;
        for (index, (early, late)) in single.iter().zip(republished.iter()).enumerate() {
            if early.peak_hold.to_bits() != late.peak_hold.to_bits()
                || early.rms_smoothed.to_bits() != late.rms_smoothed.to_bits()
            {
                divergence = Some((index, *early, *late));
                break;
            }
        }
        report.check(
            divergence.is_none(),
            format!(
                "S9：96 kHz 下'只发布一份快照'与'再发布一份等价快照'的电平帧必须逐位相同 \
                 —— 槽位建立的时刻不得改变弹道；首个分歧 {divergence:?}"
            ),
        );
    }
}

/// S10：母线电平**立体声联动**（`MeterBank::measure_bus_stereo`）：峰值取两声道最大，
/// 均方按两声道平均。
///
/// 量什么：`MeterBank::<8>` 在"只有**右**声道有信号"时的 `peak` / `rms`
/// （单位：线性幅度），以及反方向与两路满幅两个对照点。
///
/// 为什么单独立一条：`render_block` 步骤 3c 正是把这个函数接到**两条**声道上
/// （`measure_bus_stereo(master, quantum, block.left(), block.right())`）。
/// 既有的电平判据只喂过"左满 / 右零"的输入 —— **只读左声道**也全绿
/// （本票注入实测：`analyze_stereo(left, right)` 改成 `analyze(left)`，
/// 全量 24 个目标里无一条变红）⇒ "右声道在母线计量里消失"此前没有任何判据。
fn scenario_bus_meter_is_stereo_linked(report: &mut Report) {
    let master = EntityId::new();
    let mut bank = MeterBank::<8>::new();

    // 只有**右**声道有信号 ⇒ 联动峰值必须等于右声道的峰值（只读左会得到 ~0）。
    let right_only = bank.measure_bus_stereo(master, 0, &[0.0f32; 64], &[0.75f32; 64]);
    report.check(
        right_only.is_sane(),
        format!("S10 右声道独有信号的母线帧必须有限: {right_only:?}"),
    );
    report.check(
        (right_only.peak - 0.75).abs() < 1e-6,
        format!(
            "S10 母线峰值必须取两声道最大（右声道独有 ⇒ 0.75），实际 {}",
            right_only.peak
        ),
    );
    // 均方按两声道平均 ⇒ rms = sqrt((0² + 0.75²) / 2) = 0.75 / √2。
    let expected_rms = 0.75 / std::f32::consts::SQRT_2;
    report.check(
        (right_only.rms - expected_rms).abs() < 1e-5,
        format!(
            "S10 母线均方必须按两声道平均: 期望 rms={expected_rms}, 实际 {}",
            right_only.rms
        ),
    );

    // 反向：只有**左**声道有信号 —— 同一条契约的另一半。
    let left_only = bank.measure_bus_stereo(master, 1, &[0.5f32; 64], &[0.0f32; 64]);
    let expected_left_rms = 0.5 / std::f32::consts::SQRT_2;
    report.check(
        (left_only.peak - 0.5).abs() < 1e-6 && (left_only.rms - expected_left_rms).abs() < 1e-5,
        format!("S10 左声道独有信号的母线帧必须同口径: {left_only:?}"),
    );

    // 覆盖度：两声道都给满幅 ⇒ 峰值与均方都仍是满幅（**不是**两路相加）。
    let both = bank.measure_bus_stereo(master, 2, &[1.0f32; 64], &[1.0f32; 64]);
    report.check(
        (both.peak - 1.0).abs() < 1e-6 && (both.rms - 1.0).abs() < 1e-5,
        format!("S10 两声道同为满幅时峰值/均方都必须是满幅: {both:?}"),
    );
    println!(
        "[meter-rt] S10 母线立体声联动: 右独有 peak={} rms={}；两路满幅 peak={} rms={}",
        right_only.peak, right_only.rms, both.peak, both.rms
    );
}

/// 一条臂的**每量子释放乘子**（线性幅度之比，无量纲）与拿到的电平帧数。
///
/// 跑法：96 kHz / 48 kHz 的整份 `filled_project` 各跑 900 个量子，取该轨
/// `peak > 0` 的**最后**一个量子之后连续两个静音量子的 `peak_hold` 之比 ——
/// 静音窗口里 `peak_hold[q+1] = peak_hold[q] × 释放乘子`，因此这个比值就是
/// 器件当前的每量子释放乘子（与电平状态无关）。
fn release_multiplier(rate: SampleRate) -> (f32, usize) {
    const QUANTA: usize = 300;
    // 夹具刻意用**一条 120 tick 的短音符**（48 kHz 下 3 000 样本 ≈ 23 个量子；
    // 96 kHz 下 6 000 样本 ≈ 47 个量子）：`filled_project` 的音符在 900 个量子内
    // **一直出声** ⇒ 静音窗口不存在，无法读"每量子释放乘子"。
    let fixture = support::note_project(&[support::NoteSpec::at(0, 120, 69, 127)]);
    let track = fixture.track;
    let mut project = fixture.project.clone();
    project.audio_config.sample_rate = rate;
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("夹具快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, mut collector) = meter_channel(65_536);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);

    let mut output = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
    let mut frames = Vec::with_capacity(QUANTA);
    for _ in 0..QUANTA {
        runtime.process_quantum(&mut output, 2);
        let mut scratch = [MeterFrame::default(); SCRATCH_METERS];
        let drained = collector.tick(&mut scratch);
        if let Some(frame) = scratch[..drained].iter().find(|frame| frame.node == track) {
            frames.push(*frame);
        }
    }
    let last_sound = frames
        .iter()
        .rposition(|frame| frame.peak > 0.0)
        .expect("夹具必须真的出声");
    let index = last_sound + 1;
    assert!(
        index + 2 < frames.len(),
        "静音窗口必须够长（last_sound={last_sound} 帧数={}）",
        frames.len()
    );
    (
        frames[index + 2].peak_hold / frames[index + 1].peak_hold,
        frames.len(),
    )
}

/// S11：电平弹道必须跟随**快照的采样率**（`render_block` 的采样率转发）。
///
/// 量什么：同一条轨的**每量子释放乘子**（线性幅度之比，无量纲）在 48 kHz 与
/// 96 kHz 两条臂上的读数。契约：每量子释放乘子 = `10^(−1 / qps)`，
/// `qps = 采样率 ÷ 128` ⇒ 96 kHz 的乘子必须是 48 kHz 乘子的**平方根**。
///
/// 为什么单独立一条：`MeterBank::set_quanta_per_second` **自己**由 `src/meter.rs`
/// 的单元判据覆盖（`a_slot_activated_after_the_rate_change_uses_the_armed_ballistics`），
/// 但"`render_block` 把快照的采样率**转发**给它"这一步挡不住注入：
/// 本票注入实测（已还原）—— 把 `bank.set_quanta_per_second(quanta_per_second)`
/// 改写成 `bank.set_quanta_per_second(375.0)`（硬编码 48 kHz），本文件与全量
/// 24 个目标**全绿**。后果：96 kHz 工程的峰值保持按 40 dB/s 衰减（契约值的两倍）、
/// 平滑 RMS 的时间常数减半。
fn scenario_ballistics_forward_the_snapshot_sample_rate(report: &mut Report) {
    let (m48, n48) = release_multiplier(SampleRate::Hz48000);
    let (m96, n96) = release_multiplier(SampleRate::Hz96000);
    // 契约值：20 dB/s ÷ (20 · 375 量子/s) = 1/375 个数量级 ⇒ 10^(−1/375)。
    let expected48 = 10f32.powf(-1.0 / 375.0);

    report.check(
        n48 == 300 && n96 == 300,
        format!("S11 覆盖度：两臂都必须每个量子都拿到该轨的电平帧（{n48} / {n96}）"),
    );
    report.check(
        m48 > 0.0 && m48 < 1.0 && m96 > 0.0 && m96 < 1.0,
        format!("S11 覆盖度：两臂都必须真的在释放（m48={m48} m96={m96}）"),
    );
    report.check(
        (m48 - expected48).abs() < 1e-5,
        format!("S11 48 kHz 的每量子释放乘子应 = {expected48}，实际 {m48}"),
    );
    report.check(
        (m96 - m48.sqrt()).abs() < 1e-5,
        format!(
            "S11 96 kHz 的每量子释放乘子必须是 48 kHz 的平方根（弹道必须跟随快照采样率）: \
             m48={m48} 期望 m96={} 实际 {m96}",
            m48.sqrt()
        ),
    );
    println!(
        "[meter-rt] S11 弹道转发: m48={m48} m96={m96} 期望 m96={}",
        m48.sqrt()
    );
}

fn main() -> ExitCode {
    let mut report = Report::new();

    scenario_zero_alloc_with_real_meters(&mut report);
    scenario_publish_contract_per_quantum(&mut report);
    scenario_silence_is_finite(&mut report);
    scenario_ui_latest_wins_and_overflow_is_observable(&mut report);
    scenario_meter_drop_readout(&mut report);
    scenario_real_levels_and_zero_alloc_consumer(&mut report);
    scenario_ballistics_follow_the_armed_sample_rate(&mut report);
    scenario_bus_meter_is_stereo_linked(&mut report);
    scenario_ballistics_forward_the_snapshot_sample_rate(&mut report);

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
