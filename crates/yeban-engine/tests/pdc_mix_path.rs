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
//!
//! 两条判据都**走产品路径**：模型 `YebanProjectV1` → `EngineSnapshot::from_project`
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

mod support;

use std::collections::BTreeMap;

use support::render;
use yeban_engine::mixer::LIMITER_THRESHOLD;
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
