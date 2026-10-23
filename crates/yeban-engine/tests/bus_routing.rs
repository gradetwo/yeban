//! ⭐ 任务单 `#154`：**实时引擎真的消费发送/总线路由**。
//!
//! 本文件判的是"`routing_graph` 的边被实时渲染读到了"，而不是"模型能写"：
//! 每条判据都从**音频输出**（或快照里的路由计划）取读数。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | R1 | 没有任何 route 边 ⇒ 输出与"一条显式单位 `TrackToBus`"逐位相同；既有混音夹具的 FNV 指纹等于改动前基线 | 实时侧不读默认路由 / 改动默认求和路径 |
//! | R2 | `SendToAux` 真的生效：只发送到 aux（aux→master）⇒ 输出与直连 master **逐位相同**；发送 + 直连 ⇒ 峰值 **2×** | 忽略 `SendToAux`（注入 I1）/ 只直连 |
//! | R3 | 边增益生效：`-6.0206 dB`（线性 **0.5**）⇒ 该路峰值 **0.5×** | 忽略 `gain_db`（注入 I2） |
//! | R4 | 多总线：两条轨分别送两条 aux，两条 aux→master 的增益分别 0.5/0.25 ⇒ 左/右峰值比 **2×** | 只支持一条总线 / 串错目的地 |
//! | R5 | 路由计划本身：发送边的目的地与线性增益、默认直连、拓扑序 | 计划构造漏边 / 默认项缺失 |
//!
//! 基线（`#169` 时代、同一夹具、同一量子数）：`sha256 = 9ca8bdcd…8065e0`、
//! `FNV-1a 64 = 0x39d201894a7a3dfd` —— 这两个都是 **`aarch64`/macOS** 的读数，
//! 在 `4ed3adf3` 上逐字复现。⭐ `#169` 的取证：同一份代码在 `x86_64` 上不是这两个数，
//! 且 Linux（glibc）与 Windows（ucrt）**彼此也不同**（宿主 libm 的最后一位）
//! ⇒ 那时指纹只能按平台钉。
//!
//! ⭐ **`#233` 收口**：那些宿主 libm 调用点（`pan_gains` 的 `cos`/`sin`、波表的 `sin`、
//! `math` 的 `exp2`/`log2`）已全部改为纯 Rust `libm` ⇒ 指纹**不再按平台分叉**，
//! 见 [`existing_mix_fnv`] 与 [`EXISTING_MIX_SHA256`]。
//!
//! ⚠ **本片（`#154`）不证明**"aux 上能听到混响"：那时总线上还没有效果器调用点
//! （报告 §7）。本片证明的是"信号真的被搬到了那条总线上"。⭐ **`#158` 已把那条
//! 调用点补上**（总线转发之前跑它的插入链），判据在 `tests/bus_insert.rs`；
//! 本文件的 R1/R1b/R2… 仍是"默认路径逐位不变"的**承重**判据，一字未改。

mod support;

use std::collections::BTreeMap;

use support::{NoteSpec, Render, render};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::sfz_instrument::{PreparedRegion, SfzVoice, SfzVoicePool};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 既有混音夹具的逐位指纹 —— ⭐ **`#302` 重钉后只剩一个值**。
///
/// ⚠ 与 `#300` 的 `0x4be67b6b4bee4fad` 不同：`#302` 默认档去掉母线限制器的
/// 33 帧环 ⇒ 时序前移 33 帧 ⇒ 指纹变；峰值也由 `0.9000658`（被限制器压住）
/// 变为 `1.3172894`（不再压）。这是**修对**，不是回归。
///
/// ## 历史：`#169` 的三平台三值（保留作证据，⛔ 不再是断言口径）
///
/// 渲染链路里曾有**宿主 libm** 调用：[`yeban_engine::mixer::pan_gains`] 的 `cos`/`sin`、
/// `yeban_dsp::oscillator` 的波表 `sin`、`yeban_dsp::math` 的 `exp2`/`log2`。
/// 它们的最后一位随宿主 libm 实现而变 —— 正是 [ARCH-DET-001] 想禁用的那一类。
/// **同一份代码**（`4ed3adf3`）三档实测：
///
/// | 平台 | 指纹 | 出处（两次 CI 读数一致） |
/// | :--- | :--- | :--- |
/// | `aarch64` / macOS | `0x39d201894a7a3dfd` | `#154` 的原始基线；本轮在 `4ed3adf3` 复算，逐位相同 |
/// | `x86_64` / Linux（glibc） | `0xc3cd006ff3a94365` | run `39067449297`（`4ed3adf3`）＝ `14108933688450958181`；run `39039576568`（`fd33ff0f`）同值 |
/// | `x86_64` / Windows（ucrt） | `0xca85e11740967fad` | run `39067449297`（`4ed3adf3`）＝ `14593317657548062637`；run `39039576568`（`fd33ff0f`）同值 |
///
/// ## `#233` 之后
///
/// 上面三处宿主调用已改为纯 Rust `libm`（`libm::cosf`/`sinf`、`libm::sinf`、
/// `libm::exp2f`/`log2f`）⇒ **一个常数对三档都成立**。本机（macOS aarch64）实测
/// `0x28f6507e83c45b3d`；⭐ 下一轮 CI 的 Linux/Windows 腿必须读出**同一个值**，
/// 若不同 ⇒ 链路上还有**别的宿主依赖**（是本判据要抓的真缺陷，不是本文件的错）。
/// ⭐ `#300` **修对、⛔ 回归**：中性源侧中心 `0.707 → 1.0`（监听对齐导出，+3.0103 dB）
/// ⇒ 同一既有混音夹具的 FNV 由 `0x28f6507e83c45b3d` 变为 **`0x4be67b6b4bee4fad`**；
/// peak 由 `0.9000465` 微升到 `0.9000658`（母线限制器把上升压在 0.9 附近）。
/// 夹具、路由图、默认直连与判据本身一字未改。
const fn existing_mix_fnv() -> Option<u64> {
    Some(0x2d33_3312_d836_c439)
}

/// 既有混音夹具在 **`#233` 之前**实测的 `sha256`（外部 `shasum -a 256` 对原始 `f32` 字节，
/// 布局为 `left` 全部在前、`right` 全部在后，各 61 440 字节）。
///
/// ⚠ 这是 `#169` 时代 **`aarch64`/macOS** 的历史锚（`#233` 改了波表 `sin` 之后这个
/// 字节序列已不同）⇒ 本文件**只打印、不断言**它：断言的是指纹 [`existing_mix_fnv`]。
const EXISTING_MIX_SHA256: &str =
    "9ca8bdcdb48409257d05609630ca975012fe0cf7d33d1aa83f49ec00818065e0";

/// 一条测试轨的声明。
struct TrackSpec {
    id: EntityId,
    kind: TrackKind,
    pan: f32,
    notes: Vec<NoteSpec>,
}

impl TrackSpec {
    fn midi(id: EntityId, pan: f32) -> Self {
        Self {
            id,
            kind: TrackKind::Midi,
            pan,
            notes: vec![NoteSpec::at(0, 480, 60, 40)],
        }
    }

    fn aux(id: EntityId) -> Self {
        Self {
            id,
            kind: TrackKind::AuxReturn,
            pan: 0.0,
            notes: Vec::new(),
        }
    }
}

/// 一条测试边。
struct EdgeSpec {
    source: EntityId,
    destination: EntityId,
    kind: RoutingKind,
    gain_db: Option<f32>,
}

/// 组装工程（音轨 + 片段池 + 路由图）。
fn build_project(master: EntityId, tracks: &[TrackSpec], edges: &[EdgeSpec]) -> YebanProjectV1 {
    let mut map = BTreeMap::new();
    let mut pool = BTreeMap::new();
    for spec in tracks {
        let mut track = TrackV3 {
            id: spec.id,
            name: "Track".to_owned(),
            kind: spec.kind,
            pan: spec.pan,
            ..TrackV3::default()
        };
        if !spec.notes.is_empty() {
            let clip = EntityId::new();
            let placement = EntityId::new();
            let mut note_map = BTreeMap::new();
            for note in &spec.notes {
                let id = EntityId::new();
                let mut entry = MidiNote::new(id, note.start_tick, note.pitch, note.duration_ticks);
                entry.velocity = note.velocity;
                note_map.insert(id, entry);
            }
            pool.insert(
                clip,
                ClipPoolEntry {
                    id: clip,
                    name: "Clip".to_owned(),
                    content: ClipContent::Midi { notes: note_map },
                },
            );
            let span = spec
                .notes
                .iter()
                .map(|note| note.start_tick + note.duration_ticks)
                .max()
                .unwrap_or(480);
            track.clips.insert(
                placement,
                ClipPlacement {
                    id: placement,
                    clip_id: clip,
                    start_tick: 0,
                    duration_ticks: span,
                    loop_config: LoopConfig::default(),
                    muted: false,
                },
            );
        }
        map.insert(spec.id, track);
    }
    map.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut routing = RoutingGraph {
        nodes: Vec::new(),
        ..RoutingGraph::default()
    };
    for spec in tracks {
        routing.nodes.push(spec.id);
    }
    routing.nodes.push(master);
    for edge in edges {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: edge.source,
                destination_node: edge.destination,
                kind: edge.kind,
                gain_db: edge.gain_db,
            },
        );
    }

    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks: map,
        routing_graph: routing,
        clip_pool: pool,
        ..YebanProjectV1::default()
    }
}

fn render_project(project: &YebanProjectV1) -> Render {
    render(project, 120)
}

fn peak_l(render: &Render) -> f32 {
    render
        .left
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

fn peak_r(render: &Render) -> f32 {
    render
        .right
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

/// R1：**默认路由不变**。
#[test]
fn r1_missing_edges_are_bit_identical_to_the_explicit_unit_direct_edge() {
    let master = EntityId::new();
    let track = EntityId::new();
    let flat = build_project(master, &[TrackSpec::midi(track, 0.0)], &[]);
    let explicit = build_project(
        master,
        &[TrackSpec::midi(track, 0.0)],
        &[EdgeSpec {
            source: track,
            destination: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        }],
    );
    let flat = render_project(&flat);
    let explicit = render_project(&explicit);
    println!(
        "[#154/R1] 无边 vs 显式单位直连: fingerprint {:#018x} / {:#018x}、峰值 {} / {}",
        flat.fingerprint(),
        explicit.fingerprint(),
        flat.peak(),
        explicit.peak()
    );
    assert_eq!(
        flat.fingerprint(),
        explicit.fingerprint(),
        "没有 route 边与显式单位 TrackToBus 必须逐位相同"
    );
}

/// R1b：**既有混音夹具**与改动前的基线一致。
///
/// 三条判据，按"是否依赖平台"分开：
/// 1. **与平台无关**：默认路径 ≡ 同一夹具换一条**显式单位 `TrackToBus`** 边（逐位）。
///    默认路由一旦被改坏（换目的地 / 改增益 / 多一条边），这条必红 —— 三档都跑；
/// 2. **与平台无关**：峰值仍是 `0.9000465`（容差 `1e-6`）⇒ 增益/求和路径没变；
/// 3. **已实测平台**：逐位等于该平台的改动前读数（见 [`existing_mix_fnv`]）。
#[test]
fn r1b_existing_mix_fixture_matches_the_post_302_baseline() {
    let notes = [
        NoteSpec::at(0, 480, 60, 100),
        NoteSpec::at(240, 480, 64, 90),
        NoteSpec::at(480, 960, 67, 80),
    ];
    let master = EntityId::new();
    let track = EntityId::new();
    let tracks = [TrackSpec {
        id: track,
        kind: TrackKind::Midi,
        pan: 0.0,
        notes: notes.to_vec(),
    }];
    let project = build_project(master, &tracks, &[]);
    let out = render_project(&project);
    // 同一夹具 ＋ 一条**显式**的单位 `TrackToBus` 边：`#154` 之前的行为。
    let explicit = build_project(
        master,
        &tracks,
        &[EdgeSpec {
            source: track,
            destination: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        }],
    );
    let explicit_out = render_project(&explicit);
    let baseline = existing_mix_fnv();
    let baseline_text = match baseline {
        Some(value) => format!("{value:#018x}"),
        None => "未实测".to_owned(),
    };
    println!(
        "[#154/R1b] 既有夹具: fingerprint={:#018x}（本平台基线 {baseline_text}）、peak={}、sha256_baseline={EXISTING_MIX_SHA256}",
        out.fingerprint(),
        out.peak()
    );
    println!(
        "[#154/R1b] 默认路径 {:#018x} vs 显式单位直连 {:#018x}",
        out.fingerprint(),
        explicit_out.fingerprint()
    );
    assert_eq!(
        out.fingerprint(),
        explicit_out.fingerprint(),
        "默认路径必须与显式单位 TrackToBus 逐位相同（与平台无关）"
    );
    assert!(
        (out.peak() - 1.317_289_4).abs() <= 1.0e-6,
        "既有夹具的峰值必须是 1.3172894（`#302` 默认关限制器 ⇒ 不再被压到 0.9），实测 {}",
        out.peak()
    );
    if let Some(expected) = baseline {
        assert_eq!(
            out.fingerprint(),
            expected,
            "既有混音夹具的逐位指纹必须等于 `#302` 重钉后的单值基线（三平台应当相同）"
        );
    }
}

/// 发送到 aux、aux 回 master 的工程。
fn send_fixture(
    gain_db: Option<f32>,
    also_direct: bool,
) -> (EntityId, EntityId, EntityId, YebanProjectV1) {
    let master = EntityId::new();
    let track = EntityId::new();
    let aux = EntityId::new();
    let mut edges = vec![
        EdgeSpec {
            source: track,
            destination: aux,
            kind: RoutingKind::SendToAux,
            gain_db,
        },
        EdgeSpec {
            source: aux,
            destination: master,
            kind: RoutingKind::BusToMaster,
            gain_db: None,
        },
    ];
    if also_direct {
        edges.push(EdgeSpec {
            source: track,
            destination: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        });
    }
    let project = build_project(
        master,
        &[TrackSpec::midi(track, 0.0), TrackSpec::aux(aux)],
        &edges,
    );
    (master, track, aux, project)
}

/// 只直连 master 的对照工程。
fn direct_fixture() -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    build_project(master, &[TrackSpec::midi(track, 0.0)], &[])
}

/// R2：**send 真的生效**。
#[test]
fn r2_a_send_to_aux_reaches_master_and_a_second_path_doubles_the_peak() {
    let direct = render_project(&direct_fixture());
    let (_, _, _, send_only_project) = send_fixture(None, false);
    let (_, _, _, send_and_direct_project) = send_fixture(None, true);
    let send_only = render_project(&send_only_project);
    let send_and_direct = render_project(&send_and_direct_project);

    println!(
        "[#154/R2] 直连 peak={:.9}；只 send peak={:.9}（fingerprint {} vs 直连 {}）；\
         send+直连 peak={:.9}（比值 {:.9}）",
        direct.peak(),
        send_only.peak(),
        send_only.fingerprint(),
        direct.fingerprint(),
        send_and_direct.peak(),
        send_and_direct.peak() / direct.peak()
    );
    assert!(direct.peak() > 0.01, "夹具必须真的出声");
    assert_eq!(
        send_only.fingerprint(),
        direct.fingerprint(),
        "只 send（aux→master 单位增益）必须与直连逐位相同——证明信号真的走到了 aux 再回来，而不是被丢掉"
    );
    let ratio = send_and_direct.peak() / direct.peak();
    assert!(
        (ratio - 2.0).abs() < 1e-6,
        "send + 直连 ⇒ 母线上听到两次（峰值比 2.0，实得 {ratio}）"
    );
}

/// R2b：**只 send、不直连、aux 也没有回主总线的边** ⇒ aux 的默认路由（直连 master）
/// 仍让母线上听到 **一倍**（与直连逐位相同）。
#[test]
fn r2b_an_aux_without_an_explicit_return_edge_defaults_to_master() {
    let master = EntityId::new();
    let track = EntityId::new();
    let aux = EntityId::new();
    let project = build_project(
        master,
        &[TrackSpec::midi(track, 0.0), TrackSpec::aux(aux)],
        &[EdgeSpec {
            source: track,
            destination: aux,
            kind: RoutingKind::SendToAux,
            gain_db: None,
        }],
    );
    let direct = render_project(&direct_fixture());
    let send_only = render_project(&project);
    println!(
        "[#154/R2b] 只 send、aux 无回主边：peak={:.9}（直连 {:.9}）；fingerprint {} vs {}",
        send_only.peak(),
        direct.peak(),
        send_only.fingerprint(),
        direct.fingerprint()
    );
    assert!(direct.peak() > 0.01, "夹具必须真的出声");
    assert_eq!(
        send_only.fingerprint(),
        direct.fingerprint(),
        "aux 没有显式出边时默认直连 master ⇒ 只 send 仍听到一倍（不是静音）"
    );
}

/// R3：**edge gain 生效**（−6.0206 dB ⇒ 线性 0.5）。
#[test]
fn r3_edge_gain_scales_that_path_by_half() {
    let direct = render_project(&direct_fixture());
    let (_, _, _, unit_project) = send_fixture(None, false);
    let (_, _, _, half_project) = send_fixture(Some(-6.020_6), false);
    let unit = render_project(&unit_project);
    let half = render_project(&half_project);
    let ratio = half.peak() / unit.peak();
    println!(
        "[#154/R3] 发送增益 None ⇒ peak={:.9}；-6.0206 dB ⇒ peak={:.9}；比值={:.9}（目标 0.5）；直连对照 peak={:.9}",
        unit.peak(),
        half.peak(),
        ratio,
        direct.peak()
    );
    assert!(unit.peak() > 0.01, "夹具必须真的出声");
    assert!(
        (ratio - 0.5).abs() < 1e-6,
        "−6.0206 dB 的边增益必须把该路贡献减半（实得 {ratio}）"
    );
    // 非零对照：增益真的被乘进去了，而不是"两路都静音"。
    assert!(half.peak() > 0.005, "半增益那一臂仍必须听到声音");
}

/// R4：**多总线**——两条轨分别送两条 aux，两条 aux 回 master 的增益 0.5 / 0.25。
#[test]
fn r4_two_tracks_reach_two_buses_with_their_own_gains() {
    let master = EntityId::new();
    let left_track = EntityId::new();
    let right_track = EntityId::new();
    let aux_left = EntityId::new();
    let aux_right = EntityId::new();
    let project = build_project(
        master,
        &[
            TrackSpec::midi(left_track, -1.0),
            TrackSpec::midi(right_track, 1.0),
            TrackSpec::aux(aux_left),
            TrackSpec::aux(aux_right),
        ],
        &[
            EdgeSpec {
                source: left_track,
                destination: aux_left,
                kind: RoutingKind::SendToAux,
                gain_db: None,
            },
            EdgeSpec {
                source: right_track,
                destination: aux_right,
                kind: RoutingKind::SendToAux,
                gain_db: None,
            },
            EdgeSpec {
                source: aux_left,
                destination: master,
                kind: RoutingKind::BusToMaster,
                gain_db: Some(-6.020_6),
            },
            EdgeSpec {
                source: aux_right,
                destination: master,
                kind: RoutingKind::BusToMaster,
                gain_db: Some(-12.041_2),
            },
        ],
    );
    let out = render_project(&project);
    let ratio = peak_l(&out) / peak_r(&out);
    println!(
        "[#154/R4] 两轨两声相 → 两 aux → master：peak_l={:.9} peak_r={:.9} ratio={:.9}（目标 2.0）\
         fingerprint={:#018x}",
        peak_l(&out),
        peak_r(&out),
        ratio,
        out.fingerprint()
    );
    assert!(
        peak_l(&out) > 0.005 && peak_r(&out) > 0.002,
        "两条总线都必须真的出声"
    );
    assert!(
        (ratio - 2.0).abs() < 1e-5,
        "aux 增益 0.5 / 0.25 ⇒ 左/右峰值比 2.0（实得 {ratio}）"
    );
}

/// R5：路由计划本身（不经过音频）——证明 `routing_graph` 的边真的进了快照。
#[test]
fn r5_the_plan_carries_the_edges_defaults_and_topological_order() {
    let (master, track, aux, project) = send_fixture(Some(-6.020_6), true);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");
    let plan = snapshot.routing();
    let track_routes = plan.routes(&track);
    let aux_routes = plan.routes(&aux);
    println!(
        "[#154/R5] slots={:?}；track routes={:?}；aux routes={:?}；uses_buses={}；master={:?}",
        plan.slots().len(),
        track_routes
            .iter()
            .map(|route| (route.destination(), route.gain()))
            .collect::<Vec<_>>(),
        aux_routes
            .iter()
            .map(|route| (route.destination(), route.gain()))
            .collect::<Vec<_>>(),
        plan.uses_buses(),
        master
    );
    assert_eq!(plan.len(), 2, "两条非 master 可渲染节点都必须在计划里");
    assert!(plan.uses_buses(), "有非 master 目的地 ⇒ 必须启用总线暂存");
    assert_eq!(
        track_routes.len(),
        2,
        "轨 A 的一条 send + 一条直连都在计划里"
    );
    assert!(
        track_routes
            .iter()
            .any(|route| route.destination() == aux && (route.gain() - 0.5).abs() < 1e-6),
        "SendToAux 边的目的地与线性增益（−6.0206 dB ⇒ 0.5）必须被编译进计划"
    );
    assert!(
        track_routes
            .iter()
            .any(|route| route.destination() == master && route.gain() == 1.0),
        "TrackToBus 单位增益边必须被编译进计划"
    );
    assert_eq!(aux_routes.len(), 1, "aux 只有一条 BusToMaster 出边");
    assert_eq!(aux_routes[0].destination(), master);
    // 拓扑序：轨 A 在 aux 之前（边 A→aux），aux 在 master 之前。
    let slots = plan.slots();
    let a = slots
        .iter()
        .position(|node| *node == track)
        .expect("A 在计划里");
    let b = slots
        .iter()
        .position(|node| *node == aux)
        .expect("aux 在计划里");
    assert!(a < b, "拓扑序必须把上游（A）排在下游（aux）之前");

    // 默认项：没有任何边的节点得到"直连 master、单位增益"一项。
    let plain_master = EntityId::new();
    let plain_track = EntityId::new();
    let plain = build_project(plain_master, &[TrackSpec::midi(plain_track, 0.0)], &[]);
    let plain_snapshot = EngineSnapshot::from_project(&plain, 1).expect("快照");
    let plain_routes = plain_snapshot.routing().routes(&plain_track);
    assert_eq!(plain_routes.len(), 1);
    assert_eq!(plain_routes[0].destination(), plain_master);
    assert_eq!(plain_routes[0].gain(), 1.0);
    assert!(!plain_snapshot.routing().uses_buses());
}

/// R6：**尺寸红线**（只打印 + 上界；精确值由报告登记）。
///
/// 本单的代价 = `EngineRuntime` 多一个 `Box<[_; 16]>`（瘦指针 **8 B**）＋
/// `EngineSnapshot` 多一个 `RoutingPlan`（`Vec` + `BTreeMap` + `bool`）。
#[test]
fn r6_sizes_stay_under_the_192_kib_red_line() {
    let runtime = core::mem::size_of::<EngineRuntime>();
    let snapshot = core::mem::size_of::<EngineSnapshot>();
    let voice = core::mem::size_of::<SfzVoice>();
    let pool = core::mem::size_of::<SfzVoicePool>();
    let region = core::mem::size_of::<PreparedRegion>();
    println!(
        "[#154/R6] size_of: EngineRuntime={runtime} EngineSnapshot={snapshot} \
         SfzVoice={voice} SfzVoicePool={pool} PreparedRegion={region} \
         （门槛 196608；余量 = {}）",
        196_608 - runtime
    );
    assert!(runtime < 196_608, "EngineRuntime 必须 < 192 KiB");
    assert_eq!(voice, 40, "SfzVoice 必须仍是 40 B");
    assert_eq!(pool, 1_712, "SfzVoicePool 必须仍是 1712 B");
    assert_eq!(region, 6_160, "PreparedRegion 必须仍是 6160 B");
}
