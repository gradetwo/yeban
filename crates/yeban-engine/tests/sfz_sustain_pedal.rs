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
