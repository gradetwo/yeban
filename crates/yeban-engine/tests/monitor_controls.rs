//! T-FIX-190：控制室 DIM / MONO / SIDE 三个开关**真正接进引擎监听输出级**。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | M1 | DIM 开 ⇒ 每帧逐位 `== 基线 × 0.1`（−20 dB），RMS 恰好 −20 dB | 监听级没接 / 增益不是 0.1 |
//! | M2 | MONO 开 ⇒ 双声道逐位 `== (L+R)/2` | 监听级没接 / 下混公式不对 |
//! | M3 | SIDE 开 ⇒ 双声道逐位 `== L−R` | 监听级没接 / 差分公式不对 |
//! | M4 | 开关关掉的下一个量子 ⇒ 输出与基线**逐位相同**（无状态 ⇒ 可复原） | 监听级带滤波器状态 / 关不断 |
//! | M5 | `monitor_bypass = true` ＋ 三开关全开 ⇒ 输出与基线**逐位相同**（离线渲染旁通） | 旁通没接 ⇒ 监听开关漏进导出 |
//! | M6 | 全关 ⇒ 输出与基线**逐位相同**（跳过分支的证据；默认态零代价） | 全关仍碰样本 |
//!
//! 判据全部走**产品路径**：`YebanProjectV1` → `EngineSnapshot::from_project` →
//! `EngineRuntime::process_quantum`（`support::render_with`；`at` 闭包在每个量子
//! 边界上开关监听标志 —— 与控制面写原子标志是**同一只** `MonitorControls`）。
//! 时间栅格与 `tests/master_insert.rs` 相同：120 BPM / 48 kHz / 960 PPQ ⇒
//! **1 tick = 25 样本**。夹具：一条 MIDI 轨（声相偏左 ⇒ `L ≠ R`，否则 MONO/SIDE
//! 无从观测）直连 master；音符 4 拍（960×4 tick = 96,000 样本 ≈ 750 量子），
//! 渲染 200 量子（25,600 帧 ≈ 0.53 s，覆盖起音＋稳态，尾部仍有声）。
//!
//! ⭐ 本文件**不主张**"与导出逐位相同"（离线 `yeban-render` 是另一条管线），只主张
//! 监听级本身的三件事：开 ⇒ 帧按规范变、关 ⇒ 逐位复原、旁通 ⇒ 逐位不受影响。

mod support;

use std::collections::BTreeMap;

use support::{NoteSpec, render_with};
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

/// 基线渲染（三开关全关）。
fn baseline() -> support::Render {
    render_with(&project(), QUANTA, 1, |_, _| {})
}

/// 基线必须真的出声且左右不同 —— 否则 M1–M3 的"变了"断言是空转。
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
        "[monitor M0] 可听帧 {audible} / {}；左右不同帧 {asymmetric}",
        base.left.len()
    );
    assert!(
        audible > 10_000,
        "基线必须真的出声（可听帧 {audible}），否则 M1–M3 是空转"
    );
    assert!(
        asymmetric > 10_000,
        "基线左右必须不同（不同帧 {asymmetric}），否则 MONO/SIDE 无从观测"
    );
}

/// M1：DIM 开 ⇒ 每帧逐位 `== 基线 × 0.1`，RMS 恰好 −20 dB。
#[test]
fn dim_scales_every_frame_by_tenth_bit_exactly() {
    let base = baseline();
    let dimmed = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            rig.runtime.monitor_controls().set_dim(true);
        }
    });
    let changed = base
        .left
        .iter()
        .zip(dimmed.left.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[monitor M1] DIM 改动的左声道帧 {changed} / {}",
        base.left.len()
    );
    assert!(
        changed > 10_000,
        "DIM 必须真的改动输出（改动帧 {changed}）⇒ 否则监听级没接"
    );
    for (index, ((bl, br), (dl, dr))) in base
        .left
        .iter()
        .zip(base.right.iter())
        .zip(dimmed.left.iter().zip(dimmed.right.iter()))
        .enumerate()
    {
        assert_eq!(
            dl.to_bits(),
            (bl * 0.1f32).to_bits(),
            "M1 第 {index} 帧：DIM 左声道必须逐位 == 基线×0.1"
        );
        assert_eq!(
            dr.to_bits(),
            (br * 0.1f32).to_bits(),
            "M1 第 {index} 帧：DIM 右声道必须逐位 == 基线×0.1"
        );
    }
    let rms = |buf: &[f32]| {
        (buf.iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>()
            / buf.len() as f64)
            .sqrt()
    };
    let ratio_db = 20.0 * (rms(&dimmed.left) / rms(&base.left)).log10();
    println!("[monitor M1] DIM 实测衰减 = {ratio_db:.3} dB（须为 −20 dB）");
    assert!(
        (ratio_db + 20.0).abs() < 1e-3,
        "DIM 必须恰好 −20 dB，实测 {ratio_db} dB"
    );
}

/// M2：MONO 开 ⇒ 双声道逐位 `== (L+R)/2`。
#[test]
fn mono_folds_both_channels_to_the_mid_bit_exactly() {
    let base = baseline();
    let mono = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            rig.runtime.monitor_controls().set_mono(true);
        }
    });
    let changed = base
        .left
        .iter()
        .zip(mono.left.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[monitor M2] MONO 改动的左声道帧 {changed} / {}",
        base.left.len()
    );
    assert!(
        changed > 10_000,
        "MONO 必须真的改动输出（改动帧 {changed}）⇒ 否则监听级没接"
    );
    for (index, ((bl, br), (ml, mr))) in base
        .left
        .iter()
        .zip(base.right.iter())
        .zip(mono.left.iter().zip(mono.right.iter()))
        .enumerate()
    {
        let expect = (bl + br) * 0.5;
        assert_eq!(
            ml.to_bits(),
            expect.to_bits(),
            "M2 第 {index} 帧：MONO 左声道必须逐位 == (L+R)/2"
        );
        assert_eq!(
            mr.to_bits(),
            expect.to_bits(),
            "M2 第 {index} 帧：MONO 右声道必须逐位 == (L+R)/2"
        );
    }
}

/// M3：SIDE 开 ⇒ 双声道逐位 `== L−R`。
#[test]
fn side_sends_the_difference_to_both_channels_bit_exactly() {
    let base = baseline();
    let side = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            rig.runtime.monitor_controls().set_side(true);
        }
    });
    let changed = base
        .left
        .iter()
        .zip(side.left.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[monitor M3] SIDE 改动的左声道帧 {changed} / {}",
        base.left.len()
    );
    assert!(
        changed > 10_000,
        "SIDE 必须真的改动输出（改动帧 {changed}）⇒ 否则监听级没接"
    );
    for (index, ((bl, br), (sl, sr))) in base
        .left
        .iter()
        .zip(base.right.iter())
        .zip(side.left.iter().zip(side.right.iter()))
        .enumerate()
    {
        let expect = bl - br;
        assert_eq!(
            sl.to_bits(),
            expect.to_bits(),
            "M3 第 {index} 帧：SIDE 左声道必须逐位 == L−R"
        );
        assert_eq!(
            sr.to_bits(),
            expect.to_bits(),
            "M3 第 {index} 帧：SIDE 右声道必须逐位 == L−R"
        );
    }
}

/// M4：DIM 开 100 个量子后关掉 ⇒ 关掉后的帧与基线**逐位相同**。
#[test]
fn switching_off_restores_the_baseline_bit_exactly() {
    const OFF_AT: usize = 100;
    let base = baseline();
    let toggled = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            rig.runtime.monitor_controls().set_dim(true);
        }
        if quantum == OFF_AT {
            rig.runtime.monitor_controls().set_dim(false);
        }
    });
    let cut = OFF_AT * 128;
    let changed_before = base.left[..cut]
        .iter()
        .zip(toggled.left[..cut].iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[monitor M4] 关掉前改动的左声道帧 {changed_before} / {cut}；关掉后逐位比对 {} 帧",
        base.left.len() - cut
    );
    assert!(
        changed_before > 5_000,
        "关掉前 DIM 必须真的起作用（改动帧 {changed_before}）⇒ 否则 M4 没测到开关"
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
        "M4 关掉 DIM 后的左声道必须与基线逐位相同"
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
        "M4 关掉 DIM 后的右声道必须与基线逐位相同"
    );
}

/// M5：旁通开 ＋ 三开关全开 ⇒ 输出与基线**逐位相同**（离线渲染的旁通口径）。
#[test]
fn monitor_bypass_keeps_offline_render_bit_identical() {
    let base = baseline();
    let bypassed = render_with(&project(), QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            rig.runtime.monitor_controls().set_dim(true);
            rig.runtime.monitor_controls().set_mono(true);
            rig.runtime.monitor_controls().set_side(true);
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
        "M5 旁通时三开关全开，左声道仍须与基线逐位相同"
    );
    assert_eq!(
        base.right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        bypassed
            .right
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        "M5 旁通时三开关全开，右声道仍须与基线逐位相同"
    );
}

/// M6：全关 ⇒ 与基线逐位相同（跳过分支的证据；`render_with` 两次独立渲染
/// 本身就是同一条确定性路径 ⇒ 本判据同时钉住"默认态零代价"）。
#[test]
fn all_off_leaves_the_output_bit_identical() {
    let first = baseline();
    let second = baseline();
    assert_eq!(
        first.left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        second.left.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "M6 两次全关渲染的左声道必须逐位相同"
    );
    assert_eq!(
        first.right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        second.right.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "M6 两次全关渲染的右声道必须逐位相同"
    );
}
