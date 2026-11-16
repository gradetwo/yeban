//! ⭐ **延音踏板（CC64）与释放族的时机**（本片）。
//!
//! 规范：`trigger=release` 在 "note-off **or** sustain pedal off" 时发声；`release_key`
//! "same as release, but is not affected by sustain pedal"（<https://sfzformat.com/opcodes/trigger/>）。
//!
//! 本文件只钉**池级**的三件事（引擎/设备面的事件接线见台账 §6 的"仍未做"）：
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | P1 | 踏板踩下、键松开 ⇒ **只有** `release_key` 响 | `created == 1`，首帧 = release_key 样本值 |
//! | P2 | 踏板松开 ⇒ **只有** `release` 响（⛔ 不重复 release_key） | `created == 1`，首帧 = release 样本值 |
//! | P3 | 重触发清掉挂起位 | 再按同键后 `sustained(note) == false` |

use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};

const HZ: f32 = 48_000.0;
const FRAMES: usize = 4_800;
const NOTE: u8 = 60;

fn flat(value: f32) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(vec![value; FRAMES]),
        sample_hz: HZ,
        loop_points: None,
    }
}

/// 只含**一条**区域的 SFZ（样本值由调用方给）—— 判据里首帧可直接反解。
fn one_region(text: &str, value: f32) -> PreparedSfz {
    let instrument = SfzInstrument::load(text.to_owned(), &Default::default()).expect("解析");
    PreparedSfz::prepare_with_samples(&instrument, &[("x.wav".to_owned(), flat(value))], HZ)
        .expect("摊平")
}

#[test]
fn p1_with_the_pedal_down_only_release_key_regions_fire_on_key_up() {
    let plan = one_region("<region> sample=x.wav key=60 trigger=release_key", 0.125);
    let mut pool = SfzVoicePool::new(HZ);
    assert_eq!(pool.trigger_note(&plan, NOTE, 127, u64::MAX, 1.0, None), 0);
    assert!(!pool.set_pedal(true), "第一次踩下不是 down→up 跃变");
    assert_eq!(
        pool.trigger_release(&plan, NOTE, 480, 1.0, None),
        1,
        "踏板踩下时 release_key 照规范立刻响"
    );
    let mut out = vec![0.0f32; 16];
    pool.render_into(&plan, 0, &mut out);
    assert!(
        (out[0] - 0.125).abs() <= 1.0e-7,
        "首帧 {} != release_key 样本 0.125",
        out[0]
    );
}

#[test]
fn p2_on_pedal_up_only_release_regions_fire() {
    let plan = one_region("<region> sample=x.wav key=60 trigger=release", 0.25);
    let mut pool = SfzVoicePool::new(HZ);
    assert_eq!(pool.trigger_note(&plan, NOTE, 127, u64::MAX, 1.0, None), 0);
    let _ = pool.set_pedal(true);
    assert_eq!(pool.trigger_release(&plan, NOTE, 480, 1.0, None), 0);
    pool.mark_sustained(NOTE);
    assert!(pool.set_pedal(false), "松开必须是 down→up");
    assert_eq!(
        pool.trigger_pedal_up(&plan, NOTE, 480, 1.0, None),
        1,
        "踏板松开只让 release 一个区域响"
    );
    let mut out = vec![0.0f32; 16];
    pool.render_into(&plan, 0, &mut out);
    assert!(
        (out[0] - 0.25).abs() <= 1.0e-7,
        "首帧 {} != release 样本 0.25",
        out[0]
    );
}

#[test]
fn p3_retriggering_the_key_clears_the_sustained_flag() {
    let plan = one_region("<region> sample=x.wav key=60", 0.5);
    let mut pool = SfzVoicePool::new(HZ);
    let _ = pool.set_pedal(true);
    let _ = pool.trigger_note(&plan, NOTE, 127, u64::MAX, 1.0, None);
    pool.mark_sustained(NOTE);
    assert!(pool.sustained(NOTE));
    let _ = pool.trigger_note(&plan, NOTE, 100, u64::MAX, 1.0, None);
    assert!(!pool.sustained(NOTE), "重触发必须清掉挂起位");
}

#[test]
fn p4_the_synth_defers_both_the_release_voice_and_the_note_stop_until_pedal_up() {
    use std::collections::BTreeMap;

    use yeban_engine::graph::LatencyTable;
    use yeban_engine::snapshot::{EngineSnapshot, TrackParams};
    use yeban_engine::synth::SynthEngine;
    use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind, TrackV3};

    // 只有 attack（0.5）与 release（0.25）：release_key 由 P1 单独钉。
    let plan = {
        let instrument = SfzInstrument::load(
            "<region> sample=x.wav key=60\n<region> sample=y.wav key=60 trigger=release".to_owned(),
            &Default::default(),
        )
        .expect("解析");
        PreparedSfz::prepare_with_samples(
            &instrument,
            &[
                ("x.wav".to_owned(), flat(0.5)),
                ("y.wav".to_owned(), flat(0.25)),
            ],
            HZ,
        )
        .expect("摊平")
    };
    // 最小工程：一条轨 → master（与 `sfz_arm_perf` 的夹具同形；`volume_db = 0` ⇒ 轨增益 1.0）。
    let master = EntityId::new();
    let track = EntityId::new();
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
        },
    );
    let mut tracks = BTreeMap::new();
    let track_v3 = TrackV3 {
        id: track,
        volume_db: 0.0,
        ..TrackV3::default()
    };
    tracks.insert(track, TrackParams::from_track(&track_v3, 0));
    let snapshot = EngineSnapshot::from_parts(
        1,
        48_000,
        512,
        2,
        master,
        tracks,
        &routing,
        &LatencyTable::new(),
    )
    .expect("图合法");
    let mut engine = SynthEngine::new(48_000);
    engine.arm_from_snapshot(&snapshot).expect("武装");
    let _ = engine.note_on_with_sources(track, NOTE, 127, Some(&plan), None, true);
    let mut out = vec![0.0f32; 64];
    engine.render_track(track, None, Some(&plan), &mut out);
    assert!((out[0] - 0.5).abs() <= 1.0e-7, "起音首帧 {}", out[0]);
    // 踏板踩下 + 键松开：不停声部、也不建 release 声部。
    assert_eq!(engine.set_sustain_pedal(track, true, Some(&plan)), 0);
    assert!(engine.note_off_with_sources(track, NOTE, Some(&plan), true));
    let mut out = vec![0.0f32; 64];
    engine.render_track(track, None, Some(&plan), &mut out);
    assert!(
        (out[0] - 0.5).abs() <= 1.0e-7,
        "踏板踩下时起音声部必须继续响（实得 {}）",
        out[0]
    );
    // 踏板松开：release 声部建立，且起音声部在这里才被放掉。
    assert_eq!(engine.set_sustain_pedal(track, false, Some(&plan)), 1);
    let mut out = vec![0.0f32; 64];
    engine.render_track(track, None, Some(&plan), &mut out);
    assert!(
        (out[0] - 0.25).abs() <= 1.0e-7,
        "踏板松开后只剩 release 声部（实得 {}）",
        out[0]
    );
}
