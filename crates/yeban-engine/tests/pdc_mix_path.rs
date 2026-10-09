//! `ROAD-M2-004` 的**实际混音路径**判据：补偿计划真的接在 `EngineRuntime::render_block` 里。
//!
//! 规范原文（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.4 第 3 条
//! `[ARCH-PDC-001]`，`file:line` 见 `docs/ledger/engine-rt-notes.md` §2 的引用）：
//!
//! > **自动补偿对齐 (Delay Alignment)**：对于累积延迟为 $L_i$ 的并行分支，
//! > **在进入总线求和节点前**自动插入 $D_i = L_{\max} - L_i$ 采样点的环形延迟缓冲
//! > （PDC Delay Line）；实时音频引擎与 Rayon 离线母带渲染器完全共用同一套 PDC 算法。
//!
//! ## 这两个判据测的是"接线"，不是"算法"
//!
//! `PdcPlan::compute` 的数值面与 `DelayLine` 的行为面已经由 `src/graph.rs` 的
//! `pdc_alignment_invariant_holds_for_diamond` / `compensated_branches_line_up_sample_exactly`
//! 覆盖 —— 但**计划算得对**与**计划接进了混音路径**是两件事：本仓库曾长期处于
//! "`PdcPlan` 已算好、`CompensationBank` 只构造、`render_block` 一个字都不用它"的状态
//! （`docs/ledger/engine-rt-notes.md` §5.3 第 1 条）。这两个判据把那段缺口钉死：
//!
//! | # | 判据 | 怎么变红 |
//! | :-: | :--- | :--- |
//! | P1 | 短支路的输出在**汇入母线前**整体后移 `D(v)` 帧，逐位精确 | 删掉 `render_block` 里的 `self.pdc.apply(...)` 调用 ⇒ 移位量变 0 |
//! | P2 | 并联两条支路在求和节点上**采样级同相**（`D` 的差额被补齐） | 同上；另把 `set_delay` 的读写指针约定写反 ⇒ 逐位比对红 |
//! | P3 | 母线限制器 33 帧**回填**进 `LatencyTable` 之后，渲染输出**逐位不变**（那 33 帧在总线求和之后） | 把 `PdcPlan::compute` 的对齐基准换回 `arrival(master)` ⇒ 每条支路 +33 帧、逐位比对红 |
//! | P4 | 剪掉一条支路之后，幸存支路挪进的**槽位**不得重放被剪支路的音频（见该判据自己的文档块） | 删掉 `CompensationBank::rearm` 里"换主人就清线"那一句 ⇒ 继承的环把上一任的音频播出来 |
//! | P5 | 计划项数超过池的槽位 ⇒ `pdc_unarmed_nodes` 如实累加，`pdc_clamped_frames` **保持 0**（两个读数各归各的） | 把两条累加**互换**，或把任一条改成不累加（两种注入实测都让本判据红，见交付报告） |
//! | P6 | 补偿量超过线容量 ⇒ `pdc_clamped_frames` 按**帧**如实累加，`pdc_unarmed_nodes` **保持 0** | 同上 |
//!
//! 判据都**走产品路径**：模型 `YebanProjectV1` → `EngineSnapshot::from_project`
//! （`LatencyTable::from_project` 读 `DeviceDefinition::latency_samples`）→
//! `SnapshotSlot` → `EngineRuntime::process_quantum`。没有任何测试专用捷径。
//!
//! ## 覆盖范围的**诚实边界**（必须与"逐位相等"一起读）
//!
//! 1. **跑的是回调体，不是 cpal 回调线程**：与 `tests/rt_zero_alloc.rs` 的边界登记同口径 ——
//!    这里调 [`EngineRuntime::process_quantum`]（回调**内部**逻辑），没有声卡、没有
//!    `device` feature、没有 cpal 线程调度。`default-features = false` 下即可运行。
//! 2. **引擎目前没有真实的设备链延迟**：`DeviceDefinition::latency_samples` 是**上报值**，
//!    而引擎的节点处理（内置合成器）实际引入 0 帧。规范把"节点自身延迟"归给**节点**、
//!    把 `D_i` 归给 PDC，因此本判据只断言 PDC 该做的那一半：**插入 `D_i`**。
//!    P2 里"慢支路的自身延迟"由夹具把它的乐器触发时刻后移 `L` 帧如实模拟
//!    （等价于"乐器在 t=0 发声、设备链把它延后 L 帧"的**输出**），这一模拟在
//!    `src/graph.rs` 的 `compensated_branches_line_up_sample_exactly` 里就已经是既有做法
//!    （那里用一条显式的 `branch_delay`）。
//! 3. **混音路径是"平的"**：`render_block` 把每条非母线轨直接累加进 master
//!    （`sum_into_bus` 的文档已登记"发送/辅助汇流未做"）。因此这里能测的图是
//!    "两条并联支路各自直连 master"，而不是任意多级总线树。
//!
//! ## P5 / P6（`line/engine-24` 追加）：两条"不静默"读数此前**没有任何判据**
//!
//! `CompensationBank::rearm` 的两条上限（槽位不够 ⇒ `RearmShortfall::unarmed_nodes`；
//! 延迟超过线容量 ⇒ `RearmShortfall::clamped_frames`）由 `render_block` 的快照边界
//! 搬进 `EngineStats::pdc_unarmed_nodes` / `pdc_clamped_frames`。搬进去的理由写在那
//! 里的注释上：**"池装不下的部分不静默"**。
//!
//! **量什么／怎么量**：数 `crates/yeban-engine/tests/` 下对这两个字段的**引用条数**
//! （单位：条），再数其中带 `== 0` 的条数。量在**本票改动之前**的树上（`git stash`
//! 之后工作树即 `origin/main`）：`pdc_unarmed_nodes` **5** 条 / 其中 `== 0` **2** 条；
//! `pdc_clamped_frames` **3** 条 / 其中 `== 0` **1** 条 —— 两处 `== 0`
//! （`tests/idempotency_and_channel_consistency.rs` 与 `tests/rt_zero_alloc.rs`）
//! 都只是"池装得下"，**没有任何判据让它们非零过**：那个 `== 0` 满足于"这个数从来
//! 没被算错过"。
//!
//! **注入实测**（本票，两次，逐次还原并 `sha256` 核对）：把 `render_block` 里那两条
//! 累加**互换**（`unarmed_nodes += clamped_frames` 与反向），整个 `cargo test
//! -p yeban-engine --no-default-features`（**20** 个目标）全绿 —— 也就是说这两行
//! **写成什么都不会有判据变红**。⇒ 读者（界面/诊断）会把"漏了 4 条延迟线"读成
//! "钳了 0 帧"，把相位错位的**形状**读反。
//!
//! 修法就是补判据（实现那一行是对的，本票**不改它**）：P5 把计划撑过池容量，
//! P6 让 `D(v)` 超过线容量，两者都断言**自己的读数非零且对方的读数恰好为 0**，
//! 并在中途重发一份等价快照以钉住"累加"（而不是"覆写"或"重置"）。
//! 两条判据都带覆盖度见证：P5 数**池里实际武装了几条线**（必须恰好 `PDC_SLOTS`），
//! P6 读**那条被钳的线实际武装到的延迟**（必须是容量上界）—— 少了它们，
//! "夹具其实没撑过容量"也能让等号成立。

mod support;

use std::collections::BTreeMap;

use support::{render, render_snapshot};
use yeban_engine::graph::LatencyTable;
use yeban_engine::mixer::{BUS_LIMITER_LATENCY_FRAMES, LIMITER_THRESHOLD};
use yeban_engine::rt::{MAX_PDC_DELAY_FRAMES, PDC_SLOTS};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 夹具栅格：120 BPM / 48 kHz / 960 PPQ ⇒ **1 tick = 25 样本**
/// （与 `tests/synth_render.rs` 的 `SAMPLES_PER_TICK` 同源）。
const SAMPLES_PER_TICK: u64 = 25;

/// 慢支路设备链上报的自身延迟（采样点）。400 = 16 tick，正好落在夹具栅格上。
const SLOW_LATENCY: u32 = 400;

/// 慢支路的乐器被触发的 tick：把设备链延迟按样本平移到**音符起点**上
/// （见文件头的边界登记第 2 条）。
const SLOW_START_TICK: u64 = SLOW_LATENCY as u64 / SAMPLES_PER_TICK;

/// 音符时值（tick）：足够长，整个测量窗口里两支路都在发声。
const NOTE_TICKS: u64 = 960;

/// 力度刻意取小：两支路叠加后仍在 [`LIMITER_THRESHOLD`] 之下，
/// 于是母线限制器是**纯 33 帧延迟**（`gain == 1.0` 时逐位恒等），
/// 下面"2 × 单支路波形"的断言才是线性的。
const VELOCITY: u8 = 40;

/// 渲染窗口：24 量子 × 128 帧 = 3072 帧（> 400 + 33 + 一个量子的余量）。
const QUANTA: usize = 24;

/// 本夹具里**唯一的**一组实体身份。
///
/// 参考渲染与被测渲染必须共用同一组身份：`SynthEngine::begin_snapshot` 按键序分配
/// 声部槽，换一组随机 ULID 就会换槽。判据不该依赖"槽序不影响样本"这条没有被
/// 机械证明的假设。
#[derive(Clone, Copy)]
struct Ids {
    master: EntityId,
    fast: EntityId,
    slow: EntityId,
}

fn ids() -> Ids {
    Ids {
        master: EntityId::new(),
        fast: EntityId::new(),
        slow: EntityId::new(),
    }
}

/// 一条 MIDI 轨 + 一个可选音符片段。
fn note_track(
    id: EntityId,
    name: &str,
    start_tick: Option<u64>,
) -> (TrackV3, Option<ClipPoolEntry>) {
    let mut track = TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    let Some(start_tick) = start_tick else {
        return (track, None);
    };
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut note = MidiNote::new(note_id, start_tick, 60, NOTE_TICKS);
    note.velocity = VELOCITY;
    let mut note_map = BTreeMap::new();
    note_map.insert(note_id, note);
    let entry = ClipPoolEntry {
        id: clip,
        name: format!("{name} clip"),
        content: ClipContent::Midi { notes: note_map },
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: start_tick + NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    (track, Some(entry))
}

/// "两条并联支路各自直连 master"的工程。
///
/// - `slow_latency`：慢支路的**设备链上报延迟**（`ARCH-PDC-001` 的 `latency_samples`）。
///   设备恒定存在（延迟为 0 时也在），这样两个变体的设备数一致。
/// - `fast_start` / `slow_start`：两条支路的乐器触发 tick（`None` = 该支路静音）。
fn project(
    ids: Ids,
    slow_latency: u32,
    fast_start: Option<u64>,
    slow_start: Option<u64>,
) -> YebanProjectV1 {
    let (fast, fast_clip) = note_track(ids.fast, "Fast", fast_start);
    let (mut slow, slow_clip) = note_track(ids.slow, "Slow", slow_start);
    slow.devices = vec![DeviceDefinition {
        id: EntityId::new(),
        name: "Reported".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: slow_latency,
    }];

    let mut tracks = BTreeMap::new();
    tracks.insert(ids.fast, fast);
    tracks.insert(ids.slow, slow);
    tracks.insert(
        ids.master,
        TrackV3 {
            id: ids.master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut clip_pool = BTreeMap::new();
    if let Some(clip) = fast_clip {
        clip_pool.insert(clip.id, clip);
    }
    if let Some(clip) = slow_clip {
        clip_pool.insert(clip.id, clip);
    }

    let mut routing = RoutingGraph {
        nodes: vec![ids.fast, ids.slow, ids.master],
        ..RoutingGraph::default()
    };
    for source in [ids.fast, ids.slow] {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: source,
                destination_node: ids.master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }

    YebanProjectV1 {
        master_bus_track_id: ids.master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

/// 逐位断言 `shifted[t] == reference[t - delay]`，且前 `delay` 帧是**逐位静音**。
///
/// 返回"逐位相同的**非零**样本数"（防空转：两次渲染都是静音时它也满足逐位相等）。
fn assert_shifted_exactly(
    channel: &str,
    reference: &[f32],
    shifted: &[f32],
    delay: usize,
) -> usize {
    assert_eq!(
        reference.len(),
        shifted.len(),
        "{channel}：两条渲染必须等长"
    );
    for (index, sample) in shifted[..delay].iter().enumerate() {
        assert_eq!(
            sample.to_bits(),
            0.0f32.to_bits(),
            "{channel} 第 {index} 帧：PDC 延迟线前 {delay} 帧必须是**逐位静音**\
             （延迟线的历史是零）；实测 {sample:e}"
        );
    }
    let mut matched_nonzero = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for index in delay..shifted.len() {
        let expected = reference[index - delay];
        if shifted[index].to_bits() == expected.to_bits() {
            if expected != 0.0 {
                matched_nonzero += 1;
            }
        } else if mismatches.len() < 3 {
            mismatches.push(format!(
                "t={index}: 实测 {:#010x} ({:e}) vs 参考 t={} 的 {:#010x} ({:e})",
                shifted[index].to_bits(),
                shifted[index],
                index - delay,
                expected.to_bits(),
                expected
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{channel}：PDC 必须在汇入母线前把该支路整体后移 {delay} 帧；\
         前 3 处逐位差异：{mismatches:?}"
    );
    matched_nonzero
}

/// P1：短支路的输出在**汇入母线前**整体后移 `D(v)` 帧，**逐位精确**。
///
/// 图的形状：`fast → master`、`slow(自身 400) → master`。于是
/// `L_max = 400`、`D(fast) = 400`、`D(slow) = 0`（`slow` 已在关键路径上）。
/// 只有 `fast` 出声明，因此总线的每一帧都必须等于"零延迟参考渲染"的同一帧后移 400。
///
/// 变红的注入：删掉 `render_block` 逐轨循环里的 `self.pdc.apply(...)` ⇒ 移位量变 0，
/// 第一处差异出现在 `t == delay`。
#[test]
fn pdc_compensation_is_applied_on_the_real_mix_path_sample_exactly() {
    let ids = ids();
    let reference_project = project(ids, 0, Some(0), None);
    let pdc_project = project(ids, SLOW_LATENCY, Some(0), None);
    let delay = SLOW_LATENCY as usize;

    // ---- 前提：计划本身（与实现无关的纯函数读数） ----
    let reference_snapshot =
        EngineSnapshot::from_project(&reference_project, 1).expect("参考工程必须合法");
    assert_eq!(
        reference_snapshot.pdc().total_latency(),
        0,
        "零延迟工程的 L_max 必须是 0"
    );
    assert_eq!(reference_snapshot.pdc().compensation(&ids.fast), Some(0));

    let pdc_snapshot = EngineSnapshot::from_project(&pdc_project, 1).expect("被测工程必须合法");
    assert_eq!(
        pdc_snapshot.pdc().total_latency(),
        SLOW_LATENCY,
        "L_max 由慢支路上报的自身延迟决定 [ARCH-PDC-001]"
    );
    assert_eq!(
        pdc_snapshot.pdc().compensation(&ids.fast),
        Some(SLOW_LATENCY)
    );
    assert_eq!(
        pdc_snapshot.pdc().compensation(&ids.slow),
        Some(0),
        "慢支路已在关键路径上，不需要补偿"
    );

    // ---- 行为面：真实混音路径的逐位比对 ----
    let reference = render(&reference_project, QUANTA);
    let shifted = render(&pdc_project, QUANTA);
    assert_eq!(reference.frames(), QUANTA * 128);
    assert!(
        reference.nonzero() > 0,
        "参考渲染必须真的出声 —— 否则下面的逐位相等是『两边都是静音』的空转假绿"
    );
    assert!(
        reference.left[..delay].iter().any(|sample| *sample != 0.0),
        "参考渲染在前 {delay} 帧里必须有信号，否则移位观察不到"
    );

    let matched_left = assert_shifted_exactly("左声道", &reference.left, &shifted.left, delay);
    let matched_right = assert_shifted_exactly("右声道", &reference.right, &shifted.right, delay);
    assert!(
        matched_left > 1_000 && matched_right > 1_000,
        "逐位相同的**非零**样本太少（左 {matched_left} / 右 {matched_right}），判据疑似空转"
    );
    println!(
        "[pdc-mix] P1 逐位后移 {delay} 帧：非零匹配 左={matched_left} 右={matched_right} \
         （参考峰值 {:.6}；L_max={}）",
        reference.peak(),
        pdc_snapshot.pdc().total_latency()
    );
}

/// P2：并联两条支路在求和节点上**采样级同相**。
///
/// - `fast`：自身延迟 0 ⇒ `D = 400`；
/// - `slow`：自身延迟 400 ⇒ `D = 0`，其设备链延迟由夹具把乐器触发后移 400 帧模拟
///   （见文件头的边界登记第 2 条）。
///
/// 补偿正确时两条支路在**同一采样点**到达求和节点 ⇒ 总线 == `2 ×` 单支路参考波形
/// 后移 400 帧，**逐位**成立。对照组（同样的音符摆放、零延迟计划）在前 400 帧里
/// **不是**静音 ⇒ 对齐不是音符摆放自动带来的，而是 PDC 做的。
#[test]
fn pdc_lines_up_a_parallel_diamond_sample_exactly_at_the_summing_node() {
    let ids = ids();
    let delay = SLOW_LATENCY as usize;
    // 参考：只有 fast 出声明、零延迟 ⇒ 每一帧就是"单支路波形"。
    let reference = render(&project(ids, 0, Some(0), None), QUANTA);
    // 被测：D(fast)=400、D(slow)=0，两支路都必须落在第 400 帧上。
    let aligned = render(
        &project(ids, SLOW_LATENCY, Some(0), Some(SLOW_START_TICK)),
        QUANTA,
    );
    // 对照：同样的音符摆放，但计划里没有任何延迟 ⇒ 两支路错开 400 帧。
    let uncompensated = render(&project(ids, 0, Some(0), Some(SLOW_START_TICK)), QUANTA);

    assert!(
        reference.nonzero() > 0,
        "参考渲染必须真的出声 —— 否则两条支路的对齐是『都对到静音上』的假绿"
    );
    assert!(
        reference.peak() * 2.0 < LIMITER_THRESHOLD,
        "两支路叠加后的峰值 {:.6} 必须仍在母线限制器阈值 {LIMITER_THRESHOLD} 之下，\
         否则下面的 `2 ×` 断言经过的是非线性软膝、不再是逐位等式",
        reference.peak() * 2.0
    );

    // ---- 对齐：前 400 帧逐位静音，之后逐位等于 2 × 单支路参考 ----
    for (index, sample) in aligned.left[..delay].iter().enumerate() {
        assert_eq!(
            sample.to_bits(),
            0.0f32.to_bits(),
            "第 {index} 帧：两条支路都还没到达求和节点，必须是逐位静音（慢支路的 D=0，\
             但它的乐器在 400 帧后才发声；快支路要补满 400 帧）"
        );
    }
    let mut mismatches: Vec<String> = Vec::new();
    let mut matched = 0usize;
    for index in delay..aligned.frames() {
        let single = reference.left[index - delay];
        let expected = single + single; // 两条支路的贡献逐位相同，求和 == 2 × 单支路
        if aligned.left[index].to_bits() == expected.to_bits() {
            if expected != 0.0 {
                matched += 1;
            }
        } else if mismatches.len() < 3 {
            mismatches.push(format!(
                "t={index}: 实测 {:#010x} ({:e}) vs 期望 2 × 参考 t={} = {:#010x} ({:e})",
                aligned.left[index].to_bits(),
                aligned.left[index],
                index - delay,
                expected.to_bits(),
                expected
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "并联支路必须在同一条采样线上到达求和节点（D 的差额被补齐）；前 3 处差异：{mismatches:?}"
    );
    assert!(
        matched > 1_000,
        "逐位相同的**非零**样本只有 {matched} 个，判据疑似空转"
    );

    // ---- 非空转：同样的摆放、零延迟计划 ⇒ 两支路错开（对齐不是摆放带来的） ----
    assert!(
        uncompensated.left[..delay]
            .iter()
            .any(|sample| *sample != 0.0),
        "零延迟计划下快支路应当在第 0 帧附近就出声（两支路错开 {delay} 帧）；\
         这里静音说明对齐与 PDC 无关，判据没有测到被测对象"
    );
    println!(
        "[pdc-mix] P2 两支路在第 {delay} 帧对齐：非零匹配={matched}；\
         对照（零延迟）前 {delay} 帧非零样本={}",
        uncompensated.left[..delay]
            .iter()
            .filter(|sample| **sample != 0.0)
            .count()
    );
}

/// P3：**母线限制器的 33 帧回填不改变渲染输出**（[ADR-0001 D44(b)]）。
///
/// ## 被测量
///
/// 同一条工程、同一份音符摆放，走**两条投影路径**：
///
/// ```text
/// A（生产路径）  EngineSnapshot::from_project(project)
///                ⇒ LatencyTable::from_project + LatencyTable::add(master, 33)
/// B（注入路径）  EngineSnapshot::from_project_with_latencies(project, &from_project(project))
///                ⇒ 只有设备链延迟，没有那 33 帧
/// ```
///
/// 期望：`A` 的 `output_latency()` 恰好比 `B` 多 33 帧，而**每条支路的 `D(v)` 与
/// 整个渲染缓冲逐位相同** —— 因为限制器接在**总线求和之后**，它不是任何支路的相位。
///
/// ## 为什么这条判据必须存在
///
/// "把 33 帧塞进 `LatencyTable`"有两种实现：正确的（进 `output_latency`，不碰 `D`）
/// 与错误的（当成 `arrival(master)` 的 `L_max` ⇒ 给**每条**支路白加 33 帧延迟线）。
/// 后者会静默改变所有工程的输出（整体后移 33 帧），而且听起来"更响"的假绿
/// （只说"加上了延迟"）看不出来。这里用逐位比对把两者分开。
///
/// 变红的注入（实测）：
///
/// 1. 把 `crates/yeban-engine/src/graph.rs` 的
///    `let total_latency = latency.get(&master)...` 换回 `arrival.get(&master)`；
/// 2. 或只把 `compensation` 的基准从 `total_latency` 换成 `output_latency`。
///
/// 两者都让**样本级**比对变红，字面读数相同，第一处逐位差异落在 **`t == 434`**
/// （夹具的 400 帧对齐 + 33 帧）：
/// `声道0 t=434: 回填 0x00000000 (0e0) vs 未回填 0x39d5226a (4.0652166e-4)`。
///
/// ⚠ 覆盖边界（诚实登记）：本判据的样本级比对只能被"**改变了 `D(v)`**"的注入打红。
/// 因为 `EngineSnapshot` 的 A/B 两份只差延迟表一项，而 RT 路径**不读**任何
/// `latency`/`arrival` 读数（`grep latency_samples crates/yeban-engine/src/rt.rs`
/// 零命中），所以"计划面相同而样本不同"的状态在当前架构里**不可达**。
/// 样本级比对因此是**第二道**防线（防未来某次改动让 RT 路径改读别的 PDC 读数），
/// 不是唯一防线。见报告的"没红的注入"。
#[test]
fn bus_limiter_latency_backfill_keeps_the_render_bit_identical() {
    let ids = ids();
    let project = project(ids, SLOW_LATENCY, Some(0), None);

    let backfilled = EngineSnapshot::from_project(&project, 1).expect("生产路径快照");
    let raw_table = LatencyTable::from_project(&project);
    let explicit =
        EngineSnapshot::from_project_with_latencies(&project, 1, &raw_table).expect("注入路径快照");

    // 渲染会**接管**快照所有权 ⇒ 先把全部计划读数取出来。
    let l_max_backfilled = backfilled.pdc().total_latency();
    let l_max_explicit = explicit.pdc().total_latency();
    let output_backfilled = backfilled.pdc().output_latency();
    let output_explicit = explicit.pdc().output_latency();
    let comp_fast = backfilled.pdc().compensation(&ids.fast);
    let comp_slow = backfilled.pdc().compensation(&ids.slow);

    // ---- 行为面（主判据）：逐位相同（左右两声道、每一位）----
    let a = render_snapshot(backfilled, QUANTA);
    let b = render_snapshot(explicit, QUANTA);
    assert_eq!(a.frames(), b.frames(), "两次渲染必须等长");
    assert_eq!(a.frames(), QUANTA * 128);
    assert!(
        a.nonzero() > 0,
        "渲染必须真的出声 —— 否则下面的逐位相等是『两边都是静音』的空转假绿"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (channel, (left, right)) in [(&a.left, &b.left), (&a.right, &b.right)]
        .into_iter()
        .enumerate()
    {
        for (index, (x, y)) in left.iter().zip(right.iter()).enumerate() {
            if x.to_bits() != y.to_bits() && mismatches.len() < 3 {
                mismatches.push(format!(
                    "声道{channel} t={index}: 回填 {:#010x} ({:e}) vs 未回填 {:#010x} ({:e})",
                    x.to_bits(),
                    x,
                    y.to_bits(),
                    y
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "母线限制器延迟的回填**不得**移动任何支路（33 帧在总线求和之后）；\
         前 3 处逐位差异：{mismatches:?}"
    );
    assert_eq!(
        a.fingerprint(),
        b.fingerprint(),
        "逐位指纹必须相同（含左右两声道全部样本的位模式）"
    );

    // ---- 计划面（第二道防线）：唯一变化的读数是 output_latency ----
    assert_eq!(
        l_max_backfilled, l_max_explicit,
        "对齐基准 L_max 不许被那 33 帧改变"
    );
    assert_eq!(
        output_backfilled,
        output_explicit + BUS_LIMITER_LATENCY_FRAMES,
        "引擎输出延迟必须恰好多出母线限制器的 33 帧"
    );
    assert_eq!(
        output_explicit, SLOW_LATENCY,
        "注入路径只有设备链延迟（= 慢支路的 400）"
    );
    assert_eq!(comp_fast, Some(SLOW_LATENCY));
    assert_eq!(comp_slow, Some(0));

    println!(
        "[pdc-mix] P3 母线限制器回填 33 帧不动渲染：非零样本={} 指纹={:#018x} \
         （L_max={l_max_backfilled}；output_latency {output_explicit} → {output_backfilled}）",
        a.nonzero(),
        a.fingerprint(),
    );
}

/// P4：**剪掉一条支路之后，幸存支路挪进的槽位不得重放被剪支路的音频**。
///
/// 槽位是**按 PDC 计划键序的下标**绑定的（`PdcPlan::compensation` 是按键升序的
/// `BTreeMap`）⇒ 键集合一变，后面的节点就各下移一格、继承上一任的环。
/// 夹具（`support::pdc_rebind_fixture`）按身份升序钉住四个角色：
///
/// | 角色 | 键序 | 上报延迟 | 音符 | 满计划 `D` | 剪枝后 `D` |
/// | :--- | :--- | :--- | :--- | :--- | :--- |
/// | `removed` | 1 | 0 | **有**（唯一出声） | 400 | 不存在 |
/// | `keeper` | 2 | 0 | 无 | 400 | 400 |
/// | `slow` | 3 | 400 | 无 | 0 | 0 |
/// | `master` | 4 | 0 | — | 0 | 0 |
///
/// 中途发布剪掉 `removed` 的快照 ⇒ `keeper` 从第 2 槽下移到第 1 槽，而那一槽的环里
/// 留着 `removed` 的 400 帧历史。`keeper` 自己不出声 ⇒ **正确输出是静音**。
///
/// 窗口口径：跳过切换之后的 [`BUS_LIMITER_LATENCY_FRAMES`] 帧 —— 母线限制器前瞻环里
/// 装着切换前的音频，那是**真实播放过**的声音（不是泄漏）⇒ 从 `switch + 33` 到渲染
/// 结束的每一帧、左右两声道都必须**逐位零**。
///
/// 变红的注入：删掉 `CompensationBank::rearm` 里"换主人就清线"那一句 ⇒ `keeper` 把
/// `removed` 的 400 帧播出来，本判据在那 400 帧里逐位红（实测红行见交付报告）。
#[test]
fn pruning_a_branch_never_replays_the_removed_branch_from_the_reused_slot() {
    /// 切换发生在第几个量子边界。8 × 128 = 1024 帧：`removed` 的音符早已在响，
    /// 环里至少写进了 1024 个样本（> 400 帧历史）。
    const SWITCH_QUANTUM: usize = 8;
    /// 渲染长度（量子）：必须覆盖"切换 + 在途 33 帧 + 400 帧泄漏窗口"。
    const QUANTA: usize = 32;

    let fixture = support::pdc_rebind_fixture();
    let mut rebindings_before_switch = 0u64;
    let mut rebindings_after_switch = 0u64;
    let render = support::render_with(&fixture.project, QUANTA, 1, |quantum, rig| {
        if quantum == SWITCH_QUANTUM {
            rebindings_before_switch = rig.runtime.pdc_rebindings();
            let pruned =
                EngineSnapshot::from_project(&fixture.pruned, 2).expect("剪枝快照必须能编译");
            rig.slot.publish(pruned);
        }
        if quantum + 1 == QUANTA {
            rebindings_after_switch = rig.runtime.pdc_rebindings();
        }
    });

    let switch_frame = SWITCH_QUANTUM * 128;
    let leak_start = switch_frame + BUS_LIMITER_LATENCY_FRAMES as usize;
    let leak_frames = support::PDC_REBIND_LATENCY as usize;

    // ---- 覆盖度 ①：切换之前**真的有音频流过那条延迟线** ----
    let before_nonzero = render.left[..switch_frame]
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    assert!(
        before_nonzero > 0,
        "切换之前左声道一帧非零都没有 ⇒ `removed` 支路根本没写进延迟线，判据是空转"
    );
    // ---- 覆盖度 ②：槽位重绑**真的发生过**（清线分支的唯一入口）----
    assert!(
        rebindings_after_switch > rebindings_before_switch,
        "剪枝之后槽位重绑计数没有增长（{rebindings_before_switch} → {rebindings_after_switch}）\
         ⇒ 本判据对 `rearm` 的清线分支是空转"
    );
    assert!(
        render.frames() >= leak_start + leak_frames,
        "渲染窗口只有 {} 帧，覆盖不到泄漏窗口 [{leak_start}, {})",
        render.frames(),
        leak_start + leak_frames
    );

    let mut offenders: Vec<usize> = Vec::new();
    for frame in leak_start..render.frames() {
        if render.left[frame].to_bits() != 0.0f32.to_bits()
            || render.right[frame].to_bits() != 0.0f32.to_bits()
        {
            offenders.push(frame);
        }
    }
    assert!(
        offenders.is_empty(),
        "第 {switch_frame} 帧剪掉 `removed` 之后，幸存支路在 [{leak_start}, {}) 上必须逐位静音\
         （它继承的槽位必须被清成零历史）；实测 {} 个非零帧，前 3 个 {:?}（值 {:?}）",
        render.frames(),
        offenders.len(),
        &offenders[..offenders.len().min(3)],
        offenders
            .iter()
            .take(3)
            .map(|frame| render.left[*frame])
            .collect::<Vec<f32>>()
    );

    println!(
        "[pdc-mix] P4 剪枝不重放上一任的音频：切换前非零帧={before_nonzero} 重绑计数 \
         {rebindings_before_switch} → {rebindings_after_switch}；静音窗口 [{leak_start}, {}) \
         逐位零；渲染 {} 帧",
        render.frames(),
        render.frames()
    );
}

/// `branches` 里每条轨各一条边**直连** `master`、延迟全部为 0 的工程。
///
/// PDC 计划对**每一个可达节点**都分一条补偿延迟线（`D(v)` 恒为 0，但线仍然存在
/// —— `PdcPlan::compensation` 的键集合是"能到达 master 的节点"）⇒ 计划项数
/// 恰好是 `branches.len() + 1`（含 master）。这是把计划**撑过** [`PDC_SLOTS`]
/// 的最省形状：不需要任何设备、不需要任何音符。
fn fan_in_project(master: EntityId, branches: &[EntityId]) -> YebanProjectV1 {
    let mut tracks = BTreeMap::new();
    let mut nodes = vec![master];
    let mut routing = RoutingGraph::default();
    for branch in branches {
        tracks.insert(
            *branch,
            TrackV3 {
                id: *branch,
                name: "Branch".to_owned(),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
        nodes.push(*branch);
        let edge = EntityId::new();
        routing.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: *branch,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    routing.nodes = nodes;
    YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        ..YebanProjectV1::default()
    }
}

/// P5：计划项数**超过池的槽位**时，漏掉的节点数必须进 `pdc_unarmed_nodes`，
/// 而 `pdc_clamped_frames` 必须**保持 0**（两个读数各归各的，不得互换）。
///
/// **量什么／单位**：`EngineStats::pdc_unarmed_nodes` 数的是**节点个数**（个），
/// `EngineStats::pdc_clamped_frames` 数的是**采样帧数**（帧）。两者量纲不同
/// ⇒ 互换之后读数在类型上合法、在语义上是错的，而本判据就是拦这一条的那道门。
///
/// **为什么窗口里有两次武装**：首次 `process_quantum`（`armed_revision` 是 `None`）
/// 与中途发布等价快照各武装一次 ⇒ 两个读数都必须**累加**（`wrapping_add`）而不是
/// 被覆写或被重置。期望值因此是"每次武装的差额 × 2"。
///
/// **变红的注入**（本票实测，均已整体还原）：① 把 `render_block` 里两条累加互换；
/// ② 把任一条改成不累加。两种注入下本判据的两条等号同时红。
#[test]
fn an_exhausted_pdc_pool_is_counted_as_unarmed_nodes_and_never_as_clamped_frames() {
    /// 渲染长度（量子）：覆盖两次武装之后的若干量子。
    const WINDOW: usize = 8;
    /// 中途重发等价快照的量子边界 ⇒ 第二次武装。
    const REPUBLISH_QUANTUM: usize = 4;

    let master = EntityId::new();
    let branches: Vec<EntityId> = (0..PDC_SLOTS + 3).map(|_| EntityId::new()).collect();
    let project = fan_in_project(master, &branches);

    // ---- 前提（与实现无关的纯函数读数）：计划真的比池大 ----
    let planned = EngineSnapshot::from_project(&project, 1)
        .expect("夹具工程必须合法")
        .pdc()
        .compensated_len();
    assert_eq!(
        planned,
        branches.len() + 1,
        "可达节点数是 {} 条支路 + master；实测 {planned}",
        branches.len()
    );
    let per_rearm = planned - PDC_SLOTS;
    assert!(per_rearm > 0, "夹具必须真的把计划撑过池容量");

    // ---- 行为面：走产品路径，在窗口内部读池里实际武装了几条线 ----
    let nodes: Vec<EntityId> = project.tracks.keys().copied().collect();
    let mut armed_lines_in_pool = 0usize;
    let render = support::render_with(&project, WINDOW, 1, |quantum, rig| {
        if quantum == REPUBLISH_QUANTUM {
            rig.slot
                .publish(EngineSnapshot::from_project(&project, 2).expect("等价快照必须合法"));
        }
        if quantum + 1 == WINDOW {
            armed_lines_in_pool = nodes
                .iter()
                .filter(|node| rig.runtime.armed_pdc_delay(node).is_some())
                .count();
        }
    });

    assert_eq!(
        render.stats.pdc_unarmed_nodes,
        2 * per_rearm as u64,
        "首发与等价快照各武装一次，每次漏掉 {per_rearm} 个节点（计划 {planned} 项 / 池 {PDC_SLOTS} 槽）\
         ⇒ 必须累加成 {}；实测 {}（把两条累加互换、或改成不累加，这条就红）",
        2 * per_rearm,
        render.stats.pdc_unarmed_nodes
    );
    assert_eq!(
        render.stats.pdc_clamped_frames, 0,
        "本夹具的 D(v) 全为 0 ⇒ 一帧都没有被容量钳掉；这个读数量的是**帧**，不是**节点**"
    );
    // ---- 覆盖度见证：池里**恰好** PDC_SLOTS 条线（其余节点的 `armed_pdc_delay` 是 `None`）----
    // 少了它，"夹具其实没撑过容量"也能让上面那条等号成立（两边都是 0）。
    assert_eq!(
        armed_lines_in_pool, PDC_SLOTS,
        "池里恰好能有 {PDC_SLOTS} 条延迟线；实测 {armed_lines_in_pool} 条被武装"
    );
    println!(
        "[pdc-mix] P5 池用尽：计划 {planned} 项 / 池 {PDC_SLOTS} 槽 ⇒ 每次武装漏 {per_rearm} 个节点，\
         两次共 {}；pdc_clamped_frames={}；池内实武装 {armed_lines_in_pool} 条",
        render.stats.pdc_unarmed_nodes, render.stats.pdc_clamped_frames
    );
}

/// P6：补偿延迟量**超过线容量**时，被钳掉的**帧数**必须进 `pdc_clamped_frames`，
/// 而 `pdc_unarmed_nodes` 必须**保持 0**（两个读数各归各的，不得互换）。
///
/// 夹具与 [`SLOW_LATENCY`] 同形，只把慢支路上报的延迟换成"超出容量 + `OVER` 帧"：
/// `L_max = 慢支路上报值` ⇒ `D(fast) = L_max`、`D(slow) = 0`。
/// `DelayLine::set_delay` 把超过 `capacity - 1`（= [`MAX_PDC_DELAY_FRAMES`]）的值钳到
/// 容量上界 ⇒ 差额恰好是 `OVER` 帧/次武装。
///
/// **变红的注入**：同 P5（互换 / 不累加）。
#[test]
fn an_over_long_pdc_delay_is_counted_in_frames_and_never_as_unarmed_nodes() {
    /// 渲染长度（量子）。
    const WINDOW: usize = 8;
    /// 中途重发等价快照的量子边界。
    const REPUBLISH_QUANTUM: usize = 4;
    /// 让 `D(fast)` 超出线容量的余量（采样点）。
    const OVER: u32 = 808;

    let ids = ids();
    let wanted = MAX_PDC_DELAY_FRAMES as u32 + OVER;
    // 两条支路都发声：慢支路 `D = 0` ⇒ 窗口里立刻有音频（"真的渲染过"的见证），
    // 而快支路被钳到 `MAX_PDC_DELAY_FRAMES` 帧 ⇒ 它的声音落在这个窗口之外。
    let project = project(ids, wanted, Some(0), Some(0));

    // ---- 前提：计划里的 `D(fast)` 真的超出容量 ----
    let planned = EngineSnapshot::from_project(&project, 1).expect("夹具工程必须合法");
    let d_fast = planned
        .pdc()
        .compensation(&ids.fast)
        .expect("fast 必须在本计划里") as usize;
    assert_eq!(
        d_fast, wanted as usize,
        "L_max 由慢支路上报的自身延迟决定，而 fast 的累积延迟是 0"
    );
    assert!(
        d_fast > MAX_PDC_DELAY_FRAMES,
        "夹具必须真的超出线容量（D(fast)={d_fast} 帧）"
    );
    let per_rearm = d_fast - MAX_PDC_DELAY_FRAMES;

    // ---- 行为面 ----
    let mut armed_fast = None;
    let render = support::render_with(&project, WINDOW, 1, |quantum, rig| {
        if quantum == REPUBLISH_QUANTUM {
            rig.slot
                .publish(EngineSnapshot::from_project(&project, 2).expect("等价快照必须合法"));
        }
        if quantum + 1 == WINDOW {
            armed_fast = rig.runtime.armed_pdc_delay(&ids.fast);
        }
    });

    assert_eq!(
        render.stats.pdc_clamped_frames,
        2 * per_rearm as u64,
        "首发与等价快照各武装一次，每次钳掉 {per_rearm} 帧（想要的 {d_fast} 帧 > 容量 \
         {MAX_PDC_DELAY_FRAMES}）⇒ 必须累加成 {}；实测 {}（把两条累加互换、或改成不累加，\
         这条就红）",
        2 * per_rearm,
        render.stats.pdc_clamped_frames
    );
    assert_eq!(
        render.stats.pdc_unarmed_nodes, 0,
        "本夹具只有 3 个计划项（≪ 池容量）⇒ 没有任何节点被漏掉；这个读数量的是**节点**，不是**帧**"
    );
    // ---- 覆盖度见证 ①：那条被钳的线真的武装到了**容量上界**（不是"根本没武装"）----
    assert_eq!(
        armed_fast,
        Some(MAX_PDC_DELAY_FRAMES),
        "fast 的延迟线必须武装到容量上界（钳制之后的值）"
    );
    // ---- 覆盖度见证 ②：窗口里真的在出声（否则"钳了 {per_rearm} 帧"是静音上的算术）----
    assert!(
        render.nonzero() > 0,
        "夹具必须真的在窗口里出声（慢支路 D=0）"
    );
    println!(
        "[pdc-mix] P6 容量钳制：D(fast)={d_fast} 帧 > 容量 {MAX_PDC_DELAY_FRAMES} ⇒ 每次钳 \
         {per_rearm} 帧，两次共 {}；pdc_unarmed_nodes={}；池内 D(fast)={armed_fast:?}；\
         窗口非零样本 {}",
        render.stats.pdc_clamped_frames,
        render.stats.pdc_unarmed_nodes,
        render.nonzero()
    );
}
