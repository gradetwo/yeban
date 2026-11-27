//! T-FIX-579：卷帘网格选择（1/1..1/64 / 附点 / 三连音）与摇摆量化。
//!
//! 判据只钉**换算与展开**，不依赖窗口：网格 tick 的唯一换算
//! [`roll_grid_ticks_for`]，摇摆落点 [`quantize_tick_with_swing`]，以及
//! 片段量化展开 [`quantize_clip_ops_for_with_swing`]。

use yeban_app::bridge::ViewState;
use yeban_app::host::{
    RollGridMode, quantize_clip_ops_for_with_swing, quantize_tick_with_swing, roll_grid_ticks_for,
};
use yeban_model::YebanProjectV1;
use yeban_model::ops::Op;
use yeban_model::samples::filled_project;

/// 权威工程里第一个带音符的 MIDI 片段：（摆放 ULID，片段身份）。
fn first_midi_clip(project: &YebanProjectV1) -> (String, yeban_model::EntityId) {
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                && entry.content.notes().is_some_and(|notes| !notes.is_empty())
            {
                return (placement.id.to_canonical_string(), placement.clip_id);
            }
        }
    }
    panic!("filled_project 必须含至少一个带音符的 MIDI 片段");
}

/// 同一像素位移在两种网格下必须落到**不同** tick —— 网格选择真的进了吸附换算。
#[test]
fn the_same_pixel_offset_snaps_to_different_ticks_after_grid_switch() {
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    // 横向 40px × 30 tick/px = 1200 tick。
    let sixteenth = view.snapped_tick_at(
        0.0,
        40.0,
        roll_grid_ticks_for(4, Some(RollGridMode::Straight)),
    );
    let quarter = view.snapped_tick_at(
        0.0,
        40.0,
        roll_grid_ticks_for(2, Some(RollGridMode::Straight)),
    );
    assert_eq!(sixteenth, 1200, "1/16（240 tick）应吸到 1200");
    assert_eq!(quarter, 960, "1/4（960 tick）应吸到 960");
    assert_ne!(
        sixteenth, quarter,
        "网格切换后同一像素位移必须落到不同 tick（否则选择器只是装饰）"
    );
}

/// 1/1..1/64 / 附点 / 三连音的 tick 表（PPQ 960，单一换算）。
#[test]
fn musical_grid_units_map_to_the_exact_ppq_ticks() {
    let straight: Vec<u64> = (0..7)
        .map(|index| roll_grid_ticks_for(index, Some(RollGridMode::Straight)))
        .collect();
    assert_eq!(straight, vec![3840, 1920, 960, 480, 240, 120, 60]);

    assert_eq!(
        roll_grid_ticks_for(4, Some(RollGridMode::Dotted)),
        360,
        "1/16 附点 = 360 tick"
    );
    assert_eq!(
        roll_grid_ticks_for(4, Some(RollGridMode::Triplet)),
        160,
        "1/16 三连音 = 160 tick"
    );
    assert_eq!(
        roll_grid_ticks_for(2, Some(RollGridMode::Dotted)),
        1440,
        "1/4 附点 = 1440 tick"
    );
    assert_eq!(
        roll_grid_ticks_for(2, Some(RollGridMode::Triplet)),
        640,
        "1/4 三连音 = 640 tick"
    );
}

/// 摇摆 0% 与旧 `quantize_tick` 逐位一致；50% 把后一格推后半格。
#[test]
fn swing_zero_is_bit_identical_and_half_swing_delays_the_offbeat() {
    for start in [0_u64, 100, 240, 360, 480, 1_200, 2_401] {
        assert_eq!(
            quantize_tick_with_swing(start, 240, 1000, 0),
            yeban_model::clip_ops::quantize_tick(start, 240, 1000),
            "swing=0 必须与旧量化逐位一致（start={start}）"
        );
    }
    // 240 tick 网格：后一格 240 → 50% 推后 60 tick = 300。
    assert_eq!(quantize_tick_with_swing(240, 240, 1000, 500), 300);
    assert_eq!(quantize_tick_with_swing(250, 240, 1000, 500), 300);
    // 强度 50% ⇒ 只走一半位移。
    assert_eq!(quantize_tick_with_swing(240, 240, 500, 500), 270);
}

/// 片段展开真的用摇摆落点：每颗音符的新起点等于逐音符纯函数结果。
#[test]
fn clip_quantize_expansion_uses_the_swung_grid() {
    let mut project = filled_project();
    let (clip_ulid, clip_id) = first_midi_clip(&project);
    // 构造一颗**后格**音符（240 = 1/16 网格的第 1 个奇数格）⇒ 50% 摇摆必须把它推后。
    {
        let entry = project.clip_pool.get_mut(&clip_id).expect("片段在池里");
        let notes = entry.content.notes_mut().expect("夹具前提：MIDI 片段");
        if let Some(note) = notes.values_mut().next() {
            note.start_tick = 240;
        }
    }
    let notes = project
        .clip_pool
        .get(&clip_id)
        .and_then(|entry| entry.content.notes())
        .expect("夹具前提：MIDI 片段");

    let expected: Vec<(yeban_model::EntityId, u64)> = notes
        .iter()
        .filter_map(|(note_id, note)| {
            let snapped = quantize_tick_with_swing(note.start_tick, 240, 1000, 500);
            (snapped != note.start_tick).then_some((*note_id, snapped))
        })
        .collect();

    let ops = quantize_clip_ops_for_with_swing(&project, &clip_ulid, 240, 1000, 500, None)
        .expect("量化展开");
    assert_eq!(ops.len(), expected.len(), "有变化的音符数必须一致");
    for (op, (note_id, snapped)) in ops.iter().zip(expected.iter()) {
        let Op::SetNoteTiming {
            note_id: op_note,
            new_start_tick,
            ..
        } = op
        else {
            panic!("量化只应产出 SetNoteTiming");
        };
        assert_eq!(op_note, note_id, "身份按片段键序");
        assert_eq!(new_start_tick, snapped, "摇摆落点必须与纯函数逐位一致");
    }
    assert!(
        ops.iter().any(|op| matches!(op, Op::SetNoteTiming { old_start_tick, new_start_tick, .. } if *new_start_tick > *old_start_tick)),
        "50% 摇摆应至少把一颗后格音符推后"
    );
}
