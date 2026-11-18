//! ⭐ **stage 26** 判据：**逐边补偿**（一个节点两条边、各边下游求和点不同）。
//!
//! ## 判什么
//!
//! 工程：一条会发声的轨 `split` **同时**直连 `master`、以及送到一条自带 64 帧上报
//! 延迟的总线 `bus`（`bus → master`）。
//!
//! 计划：
//! * `arrival(split) = 0`、`arrival(bus) = 64`（总线自己的设备链延迟）；
//! * `L(bus) = 0`、`L(master) = max(0, 64) = 64`；
//! * 逐边：`D(split→bus) = 0`、`D(split→master) = 64` —— **两条边不同**；
//! * 节点式（旧口径）：`D(split) = max(0, 64) = 64` —— **两条边都被补 64**。
//!
//! 于是正确实现下，`split` 的直连分量与"经总线回来"的分量在 `master` 上**同相
//! 叠加**（`64` 的补偿正好抵掉总线自己的 `64`）；旧口径会把直连分量也多推 `64`
//! ⇒ 两个分量错开一整段，**只有一份**先到。
//!
//! ## 怎么量（不靠读数，靠波形）
//!
//! 同一份工程渲染两次：`bus` 的上报延迟 `64` / `0`。
//! * `64` 的那次：直连边必须被补 64 ⇒ 两条分量**同相**；
//! * `0` 的那次：两条边都是 0 ⇒ 同样**同相**。
//!
//! 两次的**逐帧波形必须逐位相同**（补偿把"多出来的 64"整段抵掉了）。旧口径
//! （节点 `max`）在 `64` 那次会把直连分量也推 64 ⇒ 两次波形在起点处相差一整段
//! ⇒ 判据红。反向腿：把逐边那一段关掉 ⇒ 红（注入实测见提交说明）。

mod support;

use std::collections::BTreeMap;

use support::render;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

const REPORTED_LATENCY: u32 = 64;
const NOTE_TICKS: u64 = 960;
const VELOCITY: u8 = 40;
const QUANTA: usize = 8;

#[derive(Clone, Copy)]
struct Ids {
    master: EntityId,
    split: EntityId,
    bus: EntityId,
}

fn ids() -> Ids {
    Ids {
        master: EntityId::new(),
        split: EntityId::new(),
        bus: EntityId::new(),
    }
}

fn note_track(id: EntityId, name: &str) -> (TrackV3, ClipPoolEntry) {
    let mut track = TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut note = MidiNote::new(note_id, 0, 60, NOTE_TICKS);
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
            duration_ticks: NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    (track, entry)
}

/// `split → master` ＋ `split → bus(报告延迟) → master`。总线上的上报延迟由 `bus`
/// 自己的设备链给（⛔ 不是让 `split` 变慢：那样两条边就一样了）。
fn project(ids: Ids, bus_latency: u32) -> YebanProjectV1 {
    let (split, split_clip) = note_track(ids.split, "Split");
    let bus = TrackV3 {
        id: ids.bus,
        name: "Bus".to_owned(),
        kind: TrackKind::AuxReturn,
        devices: vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Reported".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: Vec::new(),
            latency_samples: bus_latency,
            plugin: None,
        }],
        ..TrackV3::default()
    };
    let master = TrackV3 {
        id: ids.master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };

    let mut tracks = BTreeMap::new();
    tracks.insert(ids.split, split);
    tracks.insert(ids.bus, bus);
    tracks.insert(ids.master, master);

    let mut clip_pool = BTreeMap::new();
    clip_pool.insert(split_clip.id, split_clip);

    let mut routing = RoutingGraph {
        nodes: vec![ids.split, ids.bus, ids.master],
        ..RoutingGraph::default()
    };
    for (source, destination, kind) in [
        (ids.split, ids.master, RoutingKind::TrackToBus),
        (ids.split, ids.bus, RoutingKind::TrackToBus),
        (ids.bus, ids.master, RoutingKind::TrackToBus),
    ] {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: source,
                destination_node: destination,
                kind,
                gain_db: None,
                source_lane: None,
                dest_lane: None,
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

/// ⭐ **stage 30 判据**：一条轨的**两条出边**补偿量不同 ⇒ 必须**逐边**施补，
/// 而不是"一条节点级延迟线走天下"。
///
/// ## 为什么这一版要重写（诚实登记）
///
/// 本文件的**旧版**（stage 26）断言的是"两次渲染只差一个整段 64 帧"，即
/// `with[t + 64] == without[t]`。那条断言在**节点级**实现下也**恰好成立**：
/// 节点级把两条边**一起**推 64，而总线那一侧本来就没有真实延迟 ⇒ 两次渲染
/// 仍然只差一个整段。实测（把逐边分岔关掉跑本文件）：旧断言**全绿**，
/// 逐边落地后它反而**变红**（`shifted_onset = 0` 对 `onset + 64 = 65`）。
/// ⇒ 旧断言"绿灯但没测到东西"，本判据把它换成**形状正确**的关系。
///
/// ## 引擎现状（决定了判据能断言什么）
///
/// `DeviceDefinition::latency_samples` 在引擎里是**上报值**：节点处理不会因此真的
/// 变慢（与 `pdc_mix_path.rs` 文件头边界登记第 2 条同一条）。所以本判据**不**
/// 断言"两条分量在 master 上同相" —— 那需要总线真的吃掉 64 帧（引擎给不出，
/// 用 `SampleDelay` 器件假装也不行：那条链在块处理下读的是**未来的**样本）。
/// 它断言的是**补偿量的落点**：
///
/// * 直连边吃 `D = 64` ⇒ 它的分量在输出上**恰好晚 64 帧**；
/// * 送总线的边吃 `D = 0` ⇒ 它的分量**原样**（总线侧不真的延迟）；
///
/// 于是 `dual64[t] == base[t] + base[t - 64]`（`base` ＝ 只留直连边的那次渲染）。
/// **节点级**实现会把两条分量**都**推 64 ⇒ `dual64[t] == 2 · base[t - 64]`
/// ⇒ 本判据红（两条预测互斥，不会同时成立）。
///
/// ## 断言
///
/// | # | 断言 | 怎么变红 |
/// | :-: | :--- | :--- |
/// | E1 | 计划本身：`D(split→master) = 64` 且 `D(split→bus) = 0`（**两条不同**） | 计划退化成节点式 `max` ⇒ 两条都 64 |
/// | E2 | 总线上报 **0** 时：`dual0 == 2 · base`（逐位，快路一位不动） | 快路被改坏 / 逐边分岔顺手动了全等路径 |
/// | E3 | 总线上报 **64** 时：`dual64` **逐帧都不等于**节点级预测 `2 · base[t − 64]` | 逐边分岔被关掉（退回节点级）|
/// | E3b | 两次渲染**真的不同** | 64 帧延迟没施加（两条边都成了直通） |
/// | E4 | `pdc_edge_unarmed_edges == 0` 且 `pdc_edge_clamped_frames == 0` | 边没进池 / 被钳制（"补偿了"其实没补偿） |
/// | E5 | 波形落点：`dual64[t] == base[t] + base[t − 64]` **逐位** | 任一分量落点偏离计划（提前/旋转/重复喂线） |
///
/// ## 待查项收口（stage 30 登记 → 本线查清并修复）
///
/// stage 30 时本判据**不钉逐位落点**，因为逐边慢路产出的波形里有一份
/// **比输入提前 64 帧**的分量（`dual64[t]` 里出现 `base[t + 64]`），与"直连边
/// 该晚 64 帧"的直觉不符，而同一现象在**节点级**实现下不出现。本线
/// （`line/pdc-waveform-investigation`）的实测分解（逐位命中帧数／1024 帧）：
/// `busonly == base` 全中、`dir64 == base[t − 64]` 全中、期望式
/// `dual64 == base + base[t − 64]` 只中每量子的后半（512）、而
/// `dual64 == base + rotate64(base)`（每量子内前 64 帧取 `base[t + 64]`、
/// 后 64 帧取 `base[t − 64]`）**全中**。插桩计数：每量子 `EdgeBank::apply_pair`
/// 被调用 4 次 —— 逐轨段以真实块处理两条边（输出正确），其后**总线转发段
/// （3a⁗）又以全零块处理一遍**。
///
/// 根因：3a⁗ 遍历 `RoutingPlan::slots()`（全部可渲染节点，含纯轨），把每个
/// 节点的总线累积再转发一次。纯轨 `split` 无入边 ⇒ 累积恒零，但它的边线已被
/// 逐轨段喂过真实块；第二遍零块让延迟线把未到点的真实信号尾巴
/// （`B[64..128]`）提前吐出并路由进 master ⇒ 每量子旋转。节点级对照干净，
/// 是因为那类工程 `uses_buses == false`、3a⁗ 整段跳过；`dual0` 干净，是因为
/// 各线 D = 0 时第二遍输出恒零。修复：3a⁗ 跳过无入边节点（其累积可证恒空，
/// 见 `rt.rs` 该处注释）。修复后 E5 成立 ⇒ 落点自此钉死。
#[test]
fn two_edges_with_different_downstream_points_are_compensated_per_edge() {
    let ids = ids();

    // ---- E1：计划面（与实现无关的纯函数读数） ----
    {
        let snapshot =
            EngineSnapshot::from_project(&project(ids, REPORTED_LATENCY), 1).expect("工程必须合法");
        let plan = snapshot.pdc();
        let direct = plan
            .edge_compensation(&ids.split, &ids.master)
            .expect("直连边必须在计划里");
        let via_bus = plan
            .edge_compensation(&ids.split, &ids.bus)
            .expect("送总线的边必须在计划里");
        assert_eq!(
            (direct, via_bus),
            (REPORTED_LATENCY, 0),
            "计划：直连边吃 64（master 的最晚到达）、总线边吃 0（总线自己的 64 在它之后）"
        );
        assert_ne!(
            direct, via_bus,
            "两条边的补偿量**必须**不同（否则本判据空转）"
        );
        assert_eq!(
            plan.total_latency(),
            REPORTED_LATENCY,
            "整段延迟 = 总线自己那 64（直连边被补到同一个点）"
        );
    }

    // ---- 三次渲染：只留直连边 / 两条边＋上报 0 / 两条边＋上报 64 ----
    let base = render(&project_edges(ids, 0, true, false), QUANTA);
    let dual0 = render(&project_edges(ids, 0, true, true), QUANTA);
    let dual64 = render(&project_edges(ids, REPORTED_LATENCY, true, true), QUANTA);

    // ---- E2：全等（两条边都吃 0）⇒ 快路，逐位等于两份单边渲染之和 ----
    for index in 0..base.frames() {
        assert_eq!(
            dual0.left[index].to_bits(),
            (base.left[index] + base.left[index]).to_bits(),
            "E2 第 {index} 帧：全等档必须逐位等于 `2 · base`"
        );
    }

    // ---- E3：**不走节点级**（反向腿） ----
    //
    // 节点级实现会把**两条**分量都推 `D = 64`：`dual64[t] == 2 · base[t − 64]`。
    // 逐边档下这条预测**必须逐帧不成立**（成立就说明节点级那条线还在施加）。
    let delay = REPORTED_LATENCY as usize;
    let mut node_level_matches = 0usize;
    for index in delay..dual64.frames() {
        let node_level = 2.0 * base.left[index - delay];
        if dual64.left[index].to_bits() == node_level.to_bits() {
            node_level_matches += 1;
        }
    }
    assert_eq!(
        node_level_matches, 0,
        "节点级预测（`2 · base[t − {delay}]`）逐位命中了 {node_level_matches} 帧 \
         ⇒ 逐边分岔没生效（或两条边仍在共用节点级那条延迟线）"
    );

    // ---- E3b：同一条轨的两次渲染必须**真的不同**（防空转） ----
    assert_ne!(
        dual0.left, dual64.left,
        "上报 0 与上报 64 的两次渲染必须不同（否则 64 帧延迟根本没施加上）"
    );

    // ---- E4：武装覆盖度（⛔ 不让"边其实没进池"混过去） ----
    assert_eq!(
        dual64.stats.pdc_edge_unarmed_edges, 0,
        "两条出边都必须进池（池装不下 ⇒ 逐边补偿无从谈起）"
    );
    assert_eq!(
        dual64.stats.pdc_edge_clamped_frames, 0,
        "64 帧远在线上限之内，⛔ 不该出现钳制"
    );

    // ---- E5：波形落点（stage 30 待查项收口，根因见文件头） ----
    //
    // 直连分量恰好晚 64 帧、经总线分量原样：`dual64[t] == base[t] + base[t − 64]`
    // （`t < 64` 时第二项为零）。修复前本式只在每量子的后半成立（前半是被总线
    // 转发段第二遍喂线提前吐出的 `base[t + 64]`）⇒ 本断言对旧实现必红、对
    // 修复后实现逐位全中。
    for index in 0..base.frames() {
        let delayed = if index >= delay {
            base.left[index - delay]
        } else {
            0.0
        };
        assert_eq!(
            dual64.left[index].to_bits(),
            (base.left[index] + delayed).to_bits(),
            "E5 第 {index} 帧：直连分量必须恰好晚 {delay} 帧、经总线分量必须原样"
        );
    }
}

/// 同一份工程，按需保留 `split` 的两条出边（用来取"基线"与"双份"）。
fn project_edges(ids: Ids, bus_latency: u32, keep_direct: bool, keep_bus: bool) -> YebanProjectV1 {
    let mut project = project(ids, bus_latency);
    project.routing_graph.edges.retain(|_, edge| {
        let direct = edge.source_node == ids.split && edge.destination_node == ids.master;
        let via_bus = edge.source_node == ids.split && edge.destination_node == ids.bus;
        (!direct || keep_direct) && (!via_bus || keep_bus)
    });
    project
}
