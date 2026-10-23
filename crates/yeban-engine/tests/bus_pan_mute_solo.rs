//! ⭐ 任务单 `#299`：**实时监听腿**真的应用**总线的 pan／mute／solo**。
//!
//! `#296` ＋ `#298` 让离线导出有了总线的推子／声相／静音／独奏（`yeban-render` 的
//! `OutputStage`），但实时监听腿此前**没有落点**：`EngineSnapshot::master_gain` 只取
//! `volume_db`（`snapshot.rs:629-632`）、`rt.rs` 的 3a'' 只有 `scale_bus(master_gain)`、
//! 有入边的辅助总线在 3a⁗ 转发时也**不看**自己的 `pan`/`mute`/`solo`
//! ⇒ "AI 写了总线 pan，监听时听不到"。
//!
//! 本文件钉住修后的事实（全部走产品路径：模型 → `EngineSnapshot` → `EngineRuntime`）：
//!
//! | 编号 | 判据 | 读数（本机实测） |
//! | :--- | :--- | :--- |
//! | B1 | 中性（无总线 / 中性总线）**逐位不变**（改动前指纹） | `0xa12e1a18f4d24439` |
//! | B2 | 主总线 `mute=true` ⇒ 监听**归零** | peak `0.000000000` |
//! | B3 | 辅助总线 `mute=true` ⇒ 整条总线**归零** | peak `0.000000000` |
//! | B4 | 主总线 `pan=-1` ⇒ 右声道**逐位**静音 | L `0.548988640` / R `0` |
//! | B5 | 主总线 / 辅助总线 `pan=-0.5` ⇒ L/R 分叉，**两条腿逐位相同**，且比 = 离线值 `2.414213181` | L `0.507199347` / R `0.210088864`，比 `2.414213419` |
//! | B6 | 别的轨 solo ⇒ **未 solo 的总线静音**；`solo_safe` ⇒ 回到中性基线（逐位） | peak `0` / 指纹 `0xa12e1a18f4d24439` |
//! | B7 | 快照投影：`mute ⇒ master_gain` 位 0；`pan==0 ⇒ (1,1)` 逐位；总线 stage 的 `(pan_l,pan_r)` 与 `mixer::pan_gains` **逐位相同** | 见各断言的 `println!` |
//!
//! ## 已登记的边界（⛔ 本单不修）
//!
//! 1. **辅助总线的推子**在监听里仍不生效（只乘自有源）⇒ 与离线 B2（乘整条总线）
//!    不一致。登记为下一票；本单只做 pan／mute／solo。
//! 2. **主总线的 `solo`/`solo_safe` 不参与**：离线导出也不看（`render_core.rs:920`
//!    只判 `mute`）。若看，任何一条轨 solo 都会把整条母线静音。
//! 3. **中性声相**下监听比导出低 3 dB（源侧 `0.707/0.707` vs 离线 `1/1`）——
//!    既有缺陷，量在 `crates/yeban-app/tests/neutral_pan_leg.rs`，登记为下一票。
//!
//! 实时红线：所有新增量都在**构造期**算好（快照字段），回调里只有乘法
//! ⇒ 零分配场景见 `tests/rt_zero_alloc.rs` 的 ㊸（已加非恒等 pan/gate 覆盖）。

mod support;

use std::collections::BTreeMap;

use support::Render;
use yeban_engine::mixer::{PanLaw, pan_gains};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 改动前（`1271f7d0`）的中性夹具指纹 —— 本单的**正对照**。
///
/// 同一夹具在改动前实测 `0xa12e1a18f4d24439`（无总线与中性总线**同一个值**，
/// 与 `tests/bus_routing.rs` 的 R2 同一条事实：中性总线路由逐位等价于直连）。
const NEUTRAL_FINGERPRINT: u64 = 0xa12e_1a18_f4d2_4439;

/// 离线（app，`#298` 提交信息里的实测值）在 `pan = -0.5` 的 L/R 峰值比。
const OFFLINE_PAN_HALF_RATIO: f64 = 2.414_213_181;

const QUANTA: usize = 375;

/// 夹具的混音开关（只列本单要动的字段）。
#[derive(Clone, Copy, Debug, Default)]
struct Mix {
    lead_mute: bool,
    lead_solo: bool,
    master_mute: bool,
    master_pan: f32,
    master_volume_db: f32,
    bus_pan: f32,
    bus_mute: bool,
    bus_safe: bool,
}

fn add_edge(
    project: &mut YebanProjectV1,
    source: EntityId,
    destination: EntityId,
    kind: RoutingKind,
) {
    let id = EntityId::new();
    project.routing_graph.edges.insert(
        id,
        RoutingEdge {
            id,
            source_node: source,
            destination_node: destination,
            kind,
            gain_db: None,
        },
    );
}

/// 工程：`master` ＋ `lead`；`with_bus` 时 `lead → bus → master`，否则 `lead → master`。
fn project(mix: Mix, with_bus: bool) -> YebanProjectV1 {
    let master = EntityId::new();
    let lead = EntityId::new();
    let bus = EntityId::new();

    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        mute: mix.lead_mute,
        solo: mix.lead_solo,
        ..TrackV3::default()
    };
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note = EntityId::new();
    lead_track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 480,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut pool = BTreeMap::new();
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    note,
                    MidiNote {
                        velocity: 100,
                        ..MidiNote::new(note, 0, 69, 480)
                    },
                )]),
            },
        },
    );
    let master_track = TrackV3 {
        id: master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        mute: mix.master_mute,
        pan: mix.master_pan,
        volume_db: mix.master_volume_db,
        ..TrackV3::default()
    };
    let bus_track = TrackV3 {
        id: bus,
        name: "Bus".to_owned(),
        kind: TrackKind::AuxReturn,
        mute: mix.bus_mute,
        solo_safe: mix.bus_safe,
        pan: mix.bus_pan,
        ..TrackV3::default()
    };

    let mut tracks = BTreeMap::from([(master, master_track), (lead, lead_track)]);
    if with_bus {
        tracks.insert(bus, bus_track);
    }
    let nodes = if with_bus {
        vec![lead, bus, master]
    } else {
        vec![lead, master]
    };
    let mut project = YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks,
        clip_pool: pool,
        routing_graph: RoutingGraph {
            nodes,
            ..RoutingGraph::default()
        },
        ..YebanProjectV1::default()
    };
    if with_bus {
        add_edge(&mut project, lead, bus, RoutingKind::SendToAux);
        add_edge(&mut project, bus, master, RoutingKind::BusToMaster);
    } else {
        add_edge(&mut project, lead, master, RoutingKind::TrackToBus);
    }
    project.validate().expect("夹具必须合法");
    project
}

fn render(project: &YebanProjectV1) -> Render {
    support::render(project, QUANTA)
}

fn peak(channel: &[f32]) -> f32 {
    channel
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

fn stereo_ratio(render: &Render) -> f64 {
    f64::from(peak(&render.left)) / f64::from(peak(&render.right))
}

/// ⭐ B1：中性（无总线 ＋ 中性总线）**逐位等于改动前**。
#[test]
fn b1_a_neutral_bus_is_bit_identical_to_the_pre_change_fingerprint() {
    let direct = render(&project(Mix::default(), false));
    let bus = render(&project(Mix::default(), true));
    println!(
        "[#299/B1] 中性直连 fp={:#018x}；中性总线 fp={:#018x}；改动前基线={NEUTRAL_FINGERPRINT:#018x}",
        direct.fingerprint(),
        bus.fingerprint()
    );
    assert_eq!(
        direct.fingerprint(),
        NEUTRAL_FINGERPRINT,
        "中性直连必须逐位等于改动前"
    );
    assert_eq!(
        bus.fingerprint(),
        NEUTRAL_FINGERPRINT,
        "中性总线必须逐位等于改动前（幂等快路）"
    );
}

/// ⭐ B2 ＋ B3：主总线 / 辅助总线 `mute=true` ⇒ 监听归零。
#[test]
fn b2_b3_master_and_bus_mute_silence_the_monitoring_output() {
    let master_muted = render(&project(
        Mix {
            master_mute: true,
            ..Mix::default()
        },
        false,
    ));
    let bus_muted = render(&project(
        Mix {
            bus_mute: true,
            ..Mix::default()
        },
        true,
    ));
    println!(
        "[#299/B2] 主总线 mute: peak={:.9}（改动前同夹具 peak=0.548988640）",
        master_muted.peak()
    );
    println!(
        "[#299/B3] 辅助总线 mute: peak={:.9}（改动前同夹具 peak=0.548988640）",
        bus_muted.peak()
    );
    assert_eq!(
        master_muted.peak().to_bits(),
        0.0f32.to_bits(),
        "主总线 mute 必须让监听**逐位**归零"
    );
    assert_eq!(
        bus_muted.peak().to_bits(),
        0.0f32.to_bits(),
        "辅助总线 mute 必须让整条总线**逐位**归零"
    );
}

/// ⭐ B4：主总线 `pan=-1` ⇒ 右声道逐位静音、左声道出声。
#[test]
fn b4_master_pan_hard_left_silences_the_right_channel_bitwise() {
    let hard_left = render(&project(
        Mix {
            master_pan: -1.0,
            ..Mix::default()
        },
        false,
    ));
    let l = peak(&hard_left.left);
    let r = peak(&hard_left.right);
    println!("[#299/B4] 主总线 pan=-1: peak_l={l:.9} peak_r={r:.9}");
    assert_eq!(
        r.to_bits(),
        0.0f32.to_bits(),
        "主总线全左时右声道必须逐位静音"
    );
    assert!(l > 0.01, "主总线全左时左声道必须出声: {l}");
}

/// ⭐ B5：主总线与辅助总线在 `pan=-0.5` 上**逐位相同**，且比 = 离线值。
#[test]
fn b5_master_and_bus_pan_half_match_the_offline_ratio_bitwise() {
    let master_pan = render(&project(
        Mix {
            master_pan: -0.5,
            ..Mix::default()
        },
        false,
    ));
    let bus_pan = render(&project(
        Mix {
            bus_pan: -0.5,
            ..Mix::default()
        },
        true,
    ));
    let master_ratio = stereo_ratio(&master_pan);
    let bus_ratio = stereo_ratio(&bus_pan);
    println!(
        "[#299/B5] 主总线 pan=-0.5: L={:.9} R={:.9} ratio={master_ratio:.9}；\
         辅助总线 pan=-0.5: L={:.9} R={:.9} ratio={bus_ratio:.9}；\
         离线（app，`#298`）= {OFFLINE_PAN_HALF_RATIO:.9}",
        peak(&master_pan.left),
        peak(&master_pan.right),
        peak(&bus_pan.left),
        peak(&bus_pan.right),
    );
    assert_eq!(
        master_pan.fingerprint(),
        bus_pan.fingerprint(),
        "同一条 pan 曲线：主总线级与辅助总线级必须逐位相同"
    );
    assert!(
        (master_ratio - OFFLINE_PAN_HALF_RATIO).abs() < 1e-6,
        "L/R 比必须落在离线读数 {OFFLINE_PAN_HALF_RATIO} 的 1e-6 内，实测 {master_ratio}"
    );
    assert!(
        (bus_ratio - OFFLINE_PAN_HALF_RATIO).abs() < 1e-6,
        "辅助总线的 L/R 比必须落在离线读数 {OFFLINE_PAN_HALF_RATIO} 的 1e-6 内，实测 {bus_ratio}"
    );
    // 快照里的增益必须就是 `mixer::pan_gains` 那条曲线（逐位）。
    let expected = pan_gains(-0.5, PanLaw::default());
    for (label, project) in [
        (
            "master",
            project(
                Mix {
                    master_pan: -0.5,
                    ..Mix::default()
                },
                false,
            ),
        ),
        (
            "bus",
            project(
                Mix {
                    bus_pan: -0.5,
                    ..Mix::default()
                },
                true,
            ),
        ),
    ] {
        let snapshot = EngineSnapshot::from_project(&project, 1).expect("快照");
        if label == "master" {
            assert_eq!(
                (
                    snapshot.master_pan_left().to_bits(),
                    snapshot.master_pan_right().to_bits()
                ),
                (expected.0.to_bits(), expected.1.to_bits()),
                "主总线 pan 必须与 mixer::pan_gains(-0.5) 逐位相同"
            );
        } else {
            let master = snapshot.master();
            let bus = *snapshot
                .tracks()
                .keys()
                .find(|id| **id != master && snapshot.routing().has_incoming_audio(id))
                .expect("夹具必须有一条有入边的总线");
            let slot = snapshot.routing().slot_of(&bus).expect("总线在路由槽里");
            let stage = snapshot
                .bus_output_stage(slot)
                .expect("stage 表与 slots 平行");
            assert_eq!(
                (stage.pan_left().to_bits(), stage.pan_right().to_bits()),
                (expected.0.to_bits(), expected.1.to_bits()),
                "辅助总线 pan 必须与 mixer::pan_gains(-0.5) 逐位相同"
            );
        }
    }
}

/// ⭐ B6：别的轨 solo ⇒ 未 solo 的总线静音；`solo_safe` ⇒ 回到中性基线（逐位）。
#[test]
fn b6_bus_solo_gate_and_solo_safe() {
    let not_soloed = render(&project(
        Mix {
            lead_solo: true,
            ..Mix::default()
        },
        true,
    ));
    let safe = render(&project(
        Mix {
            lead_solo: true,
            bus_safe: true,
            ..Mix::default()
        },
        true,
    ));
    println!(
        "[#299/B6] lead solo / bus 未 solo: peak={:.9}；bus solo_safe: peak={:.9} fp={:#018x}",
        not_soloed.peak(),
        safe.peak(),
        safe.fingerprint()
    );
    assert_eq!(
        not_soloed.peak().to_bits(),
        0.0f32.to_bits(),
        "有轨 solo 而总线既不 solo 也不 solo_safe ⇒ 整条总线必须静音"
    );
    assert_eq!(
        safe.fingerprint(),
        NEUTRAL_FINGERPRINT,
        "总线 solo_safe ⇒ 必须回到中性基线（逐位）"
    );
}

/// ⭐ B7：快照投影读数（mute⇒0、pan==0⇒(1,1)、总线 stage 恒等/非恒等）。
#[test]
fn b7_snapshot_projection_readings() {
    // 主总线 mute ⇒ 推子位 0。
    let muted = EngineSnapshot::from_project(
        &project(
            Mix {
                master_mute: true,
                master_volume_db: -6.0,
                ..Mix::default()
            },
            false,
        ),
        1,
    )
    .expect("快照");
    println!(
        "[#299/B7] master mute + volume -6dB ⇒ master_gain={:.9}（bits={:#010x}）",
        muted.master_gain(),
        muted.master_gain().to_bits()
    );
    assert_eq!(
        muted.master_gain().to_bits(),
        0.0f32.to_bits(),
        "主总线 mute 必须把推子折成 0.0"
    );

    // 中性主总线 ⇒ pan 逐位 (1,1)；中性总线 stage 恒等。
    let neutral = EngineSnapshot::from_project(&project(Mix::default(), true), 1).expect("快照");
    assert_eq!(
        (
            neutral.master_pan_left().to_bits(),
            neutral.master_pan_right().to_bits()
        ),
        (1.0f32.to_bits(), 1.0f32.to_bits()),
        "pan == 0 的主总线必须逐位恒等"
    );
    assert!(
        neutral
            .bus_outputs()
            .iter()
            .all(|stage| stage.is_identity()),
        "中性总线的输出级必须全部恒等"
    );
    assert_eq!(
        neutral.bus_outputs().len(),
        neutral.routing().slots().len(),
        "stage 表必须与 routing.slots() 逐下标平行"
    );

    // 主总线 `solo` 不参与（与离线同口径），`volume_db` 仍走 track_gain。
    let solo_master = EngineSnapshot::from_project(
        &{
            let mut project = project(Mix::default(), false);
            project
                .tracks
                .get_mut(&project.master_bus_track_id)
                .expect("master")
                .solo = true;
            project
                .tracks
                .get_mut(&project.master_bus_track_id)
                .expect("master")
                .volume_db = -6.0;
            project
        },
        1,
    )
    .expect("快照");
    println!(
        "[#299/B7] master solo=true, volume=-6dB ⇒ master_gain={:.9}（不看 solo）",
        solo_master.master_gain()
    );
    assert!(
        solo_master.master_gain() > 0.0,
        "主总线的 solo 必须不参与（离线也不看）"
    );
}
