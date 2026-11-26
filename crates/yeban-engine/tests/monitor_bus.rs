//! T-FIX-191：控制室 MON A / MON B **真正切换监听总线**（不再是只改界面状态）。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | B1 | B 路增益 0.5 ＋ 选中 B ⇒ 每帧逐位 `== 基线 × 0.5` | 总线级没接 / 选路没生效 |
//! | B2 | B 路静音 ⇒ 全写 0；100 个量子后取消静音 ⇒ 之后与基线**逐位相同** | 静音没接 / 关不断 |
//! | B3 | A 路增益 0.25、B 路增益 0.5 ⇒ 选 A 得 ×0.25、选 B 得 ×0.5（两路独立） | 两路共用一份增益 |
//! | B4 | B 路开 100 个量子后切回 A ⇒ 切回后的帧与基线**逐位相同** | 切路带状态 / 切不回去 |
//! | B5 | `monitor_bypass = true` ＋ B 路选中＋静音＋增益 ⇒ 输出与基线**逐位相同**（离线渲染旁通） | 旁通漏了总线级 |
//! | B6 | 默认增益 unity 下选中 B ⇒ 输出与基线**逐位相同**（未校准前切换透明） | 默认增益不是 1.0 |
//!
//! 夹具与 `tests/monitor_controls.rs` 同款（一条 MIDI 轨，声相偏左 ⇒ `L ≠ R`；
//! 120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本；4 拍 C4 音，渲染 200 量子）。
//! 判据全部走**产品路径**：`YebanProjectV1` → `EngineSnapshot::from_project` →
//! `EngineRuntime::process_quantum`（`support::render_with`；`at` 闭包在每个量子
//! 边界上写总线状态 —— 与控制面写原子标志是**同一只** `MonitorControls`）。
//!
//! 设计说明（诚实口径）：两路默认增益均为 unity、均不静音 ⇒ 未校准前切路本身
//! 不改变帧内容（B6 钉住这一点）；可听差异来自各自独立的增益 trim／静音校准，
//! B1–B3 用 API 设定 trim 证明路由真实 —— 本单不虚构一个"备用监听 −6 dB"之类
//! 的默认差值（那是产品校准决策，不该由修 placebo 的单子擅自定）。

mod support;

use std::collections::BTreeMap;

use support::{NoteSpec, render_with};
use yeban_engine::mixer::MonitorBus;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 渲染长度：200 × 128 = 25,600 帧 ≈ 0.53 s @48 kHz。
const QUANTA: usize = 200;

/// 组装工程：一条 MIDI 轨（声相偏左）＋ 主总线轨。
fn monitor_project(note: NoteSpec, track_pan: f32) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    let mut pool: BTreeMap<EntityId, ClipPoolEntry> = BTreeMap::new();

    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut notes = BTreeMap::new();
    let mut entry = MidiNote::new(note_id, note.start_tick, note.pitch, note.duration_ticks);
    entry.velocity = note.velocity;
    notes.insert(note_id, entry);
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    let mut track_model = TrackV3 {
        id: track,
        name: "Track".to_owned(),
        kind: TrackKind::Midi,
        pan: track_pan,
        volume_db: 0.0,
        ..TrackV3::default()
    };
    track_model.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: note.start_tick + note.duration_ticks,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
    tracks.insert(track, track_model);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );

    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool: pool,
        ..YebanProjectV1::default()
    }
}

/// 夹具音符：中音 C 起、4 拍长、力度 100。
fn note() -> NoteSpec {
    NoteSpec {
        start_tick: 0,
        duration_ticks: 960 * 4,
        pitch: 60,
        velocity: 100,
    }
}

/// 夹具工程：声相 −0.5（左偏 ⇒ `L ≠ R` 且双声道都有声）。
fn project() -> YebanProjectV1 {
    monitor_project(note(), -0.5)
}

/// 基线渲染（总线 A、增益 unity、不静音）。
fn baseline() -> support::Render {
    render_with(&project(), QUANTA, 1, |_, _| {})
}

/// 基线必须真的出声且左右不同 —— 否则 B1–B3 的"变了"断言是空转。
#[test]
fn baseline_is_audible_and_stereo_asymmetric() {
    let base = baseline();
    let audible = base
        .left
        .iter()
        .zip(base.right.iter())
        .filter(|(l, r)| l.abs() > 1e-6 || r.abs() > 1e-6)
        .count();
    let asymmetric = base
        .left
        .iter()
        .zip(base.right.iter())
        .filter(|(l, r)| (*l - *r).abs() > 1e-6)
        .count();
    println!(
        "[monitor-bus B0] 可听帧 {audible} / {}；左右不同帧 {asymmetric}",
        base.left.len()
    );
    assert!(
        audible > 10_000,
        "基线必须真的出声（可听帧 {audible}），否则 B1–B3 是空转"
    );
    assert!(
        asymmetric > 10_000,
        "基线左右必须不同（不同帧 {asymmetric}），否则总线增益无从观测"
    );
}

/// B1：B 路增益 0.5 ＋ 选中 B ⇒ 每帧逐位 `== 基线 × 0.5`。
#[test]
fn bus_b_with_half_gain_scales_every_frame_bit_exactly() {
    let base = baseline();
    let alt = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            let controls = rig.runtime.monitor_controls();
            controls.set_bus_gain(MonitorBus::B, 0.5);
            controls.set_bus(MonitorBus::B);
        }
    });
    let changed = base
        .left
        .iter()
        .zip(alt.left.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[monitor-bus B1] B 路改动的左声道帧 {changed} / {}",
        base.left.len()
    );
    assert!(
        changed > 10_000,
        "B 路增益必须真的改动输出（改动帧 {changed}）⇒ 否则总线级没接"
    );
    for (index, ((bl, br), (al, ar))) in base
        .left
        .iter()
        .zip(base.right.iter())
        .zip(alt.left.iter().zip(alt.right.iter()))
        .enumerate()
    {
        assert_eq!(
            al.to_bits(),
            (bl * 0.5f32).to_bits(),
            "B1 第 {index} 帧：B 路左声道必须逐位 == 基线×0.5"
        );
        assert_eq!(
            ar.to_bits(),
            (br * 0.5f32).to_bits(),
            "B1 第 {index} 帧：B 路右声道必须逐位 == 基线×0.5"
        );
    }
}

/// B2：B 路静音 ⇒ 全写 0；100 个量子后取消静音 ⇒ 之后与基线逐位相同。
#[test]
fn bus_b_mute_silences_and_unmute_restores_bit_exactly() {
    const OFF_AT: usize = 100;
    let base = baseline();
    let muted = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            let controls = rig.runtime.monitor_controls();
            controls.set_bus(MonitorBus::B);
            controls.set_bus_mute(MonitorBus::B, true);
        }
        if quantum == OFF_AT {
            rig.runtime
                .monitor_controls()
                .set_bus_mute(MonitorBus::B, false);
        }
    });
    let cut = OFF_AT * 128;
    let muted_nonzero = muted.left[..cut]
        .iter()
        .chain(muted.right[..cut].iter())
        .filter(|v| **v != 0.0)
        .count();
    println!(
        "[monitor-bus B2] 静音段非零帧 {muted_nonzero}；取消后逐位比对 {} 帧",
        base.left.len() - cut
    );
    assert_eq!(
        muted_nonzero, 0,
        "B2 静音段必须全写 0（非零帧 {muted_nonzero}）"
    );
    assert_eq!(
        base.left[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        muted.left[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "B2 取消静音后的左声道必须与基线逐位相同"
    );
    assert_eq!(
        base.right[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        muted.right[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "B2 取消静音后的右声道必须与基线逐位相同"
    );
}

/// B3：A 路增益 0.25、B 路增益 0.5 ⇒ 选 A 得 ×0.25、选 B 得 ×0.5（两路独立）。
#[test]
fn bus_gains_are_independent_per_bus() {
    let base = baseline();
    let via_a = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            let controls = rig.runtime.monitor_controls();
            controls.set_bus_gain(MonitorBus::A, 0.25);
            controls.set_bus_gain(MonitorBus::B, 0.5);
            controls.set_bus(MonitorBus::A);
        }
    });
    let via_b = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            let controls = rig.runtime.monitor_controls();
            controls.set_bus_gain(MonitorBus::A, 0.25);
            controls.set_bus_gain(MonitorBus::B, 0.5);
            controls.set_bus(MonitorBus::B);
        }
    });
    for (index, ((bl, br), (al, ar))) in base
        .left
        .iter()
        .zip(base.right.iter())
        .zip(via_a.left.iter().zip(via_a.right.iter()))
        .enumerate()
    {
        assert_eq!(
            al.to_bits(),
            (bl * 0.25f32).to_bits(),
            "B3 第 {index} 帧：A 路左声道必须逐位 == 基线×0.25"
        );
        assert_eq!(
            ar.to_bits(),
            (br * 0.25f32).to_bits(),
            "B3 第 {index} 帧：A 路右声道必须逐位 == 基线×0.25"
        );
    }
    for (index, ((bl, br), (al, ar))) in base
        .left
        .iter()
        .zip(base.right.iter())
        .zip(via_b.left.iter().zip(via_b.right.iter()))
        .enumerate()
    {
        assert_eq!(
            al.to_bits(),
            (bl * 0.5f32).to_bits(),
            "B3 第 {index} 帧：B 路左声道必须逐位 == 基线×0.5"
        );
        assert_eq!(
            ar.to_bits(),
            (br * 0.5f32).to_bits(),
            "B3 第 {index} 帧：B 路右声道必须逐位 == 基线×0.5"
        );
    }
}

/// B4：B 路开 100 个量子后切回 A ⇒ 切回后的帧与基线逐位相同。
#[test]
fn switching_back_to_bus_a_restores_the_baseline_bit_exactly() {
    const SWITCH_AT: usize = 100;
    let base = baseline();
    let toggled = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            let controls = rig.runtime.monitor_controls();
            controls.set_bus_gain(MonitorBus::B, 0.5);
            controls.set_bus(MonitorBus::B);
        }
        if quantum == SWITCH_AT {
            rig.runtime.monitor_controls().set_bus(MonitorBus::A);
        }
    });
    let cut = SWITCH_AT * 128;
    let changed_before = base.left[..cut]
        .iter()
        .zip(toggled.left[..cut].iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[monitor-bus B4] 切回前改动的左声道帧 {changed_before} / {cut}；切回后逐位比对 {} 帧",
        base.left.len() - cut
    );
    assert!(
        changed_before > 5_000,
        "切回前 B 路必须真的起作用（改动帧 {changed_before}）⇒ 否则 B4 没测到切路"
    );
    assert_eq!(
        base.left[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        toggled.left[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "B4 切回 A 后的左声道必须与基线逐位相同"
    );
    assert_eq!(
        base.right[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        toggled.right[cut..]
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "B4 切回 A 后的右声道必须与基线逐位相同"
    );
}

/// B5：旁通开 ＋ B 路选中＋静音＋增益 ⇒ 输出与基线逐位相同（离线渲染的旁通口径）。
#[test]
fn monitor_bypass_keeps_offline_render_bit_identical() {
    let base = baseline();
    let bypassed = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            let controls = rig.runtime.monitor_controls();
            controls.set_bus_gain(MonitorBus::B, 0.5);
            controls.set_bus_mute(MonitorBus::B, true);
            controls.set_bus(MonitorBus::B);
            rig.runtime.set_monitor_bypass(true);
        }
    });
    assert_eq!(
        base.left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        bypassed
            .left
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "B5 旁通时 B 路选中＋静音，左声道仍须与基线逐位相同"
    );
    assert_eq!(
        base.right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        bypassed
            .right
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "B5 旁通时 B 路选中＋静音，右声道仍须与基线逐位相同"
    );
}

/// B6：默认增益 unity 下选中 B ⇒ 输出与基线逐位相同（未校准前切换透明）。
#[test]
fn selecting_bus_b_with_default_unity_gain_is_transparent() {
    let base = baseline();
    let alt = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            rig.runtime.monitor_controls().set_bus(MonitorBus::B);
        }
    });
    assert_eq!(
        base.left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        alt.left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "B6 默认 unity 下选中 B，左声道必须与基线逐位相同"
    );
    assert_eq!(
        base.right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        alt.right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "B6 默认 unity 下选中 B，右声道必须与基线逐位相同"
    );
}
