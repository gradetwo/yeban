//! T-FIX-770：卷帘单击命中必须先裁纵向音高窗口。
//!
//! 窗口外低音 48 被 `pitch_lane_with_base` 钳到最底道，与窗口内 60 逐像素折叠；
//! 它在 `view.notes` 里排在后面（“最后绘制”），修前会抢走命中，宿主再按可见集
//! 丢弃 ⇒ 点击可见音符被静默吞掉。

use std::str::FromStr as _;

use yeban_app::bridge::{
    PITCH_LANE_BASE, ViewState, pitch_lane_y_with_base, visible_pitch_range_for,
};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, MidiNote, YebanProjectV1};

fn fixture_id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 同一 MIDI 剪辑、同一 x/时值：一颗窗口内 60，一颗窗口外 48（排在后面）。
fn folded_pitch_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = yeban_model::samples::filled_project();
    let mut clip_id = None;
    for entry in project.clip_pool.values() {
        if matches!(entry.content, ClipContent::Midi { .. }) {
            clip_id = Some(entry.id);
        }
    }
    let clip_id = clip_id.expect("夹具必须有 MIDI 片段");
    let visible_id = fixture_id(10);
    let offscreen_id = fixture_id(20);
    let ClipContent::Midi { notes } = &mut project
        .clip_pool
        .get_mut(&clip_id)
        .expect("片段池条目")
        .content
    else {
        unreachable!();
    };
    notes.clear();
    notes.insert(visible_id, MidiNote::new(visible_id, 0, 60, 480));
    notes.insert(offscreen_id, MidiNote::new(offscreen_id, 0, 48, 480));
    (project, visible_id, offscreen_id)
}

#[test]
fn a_click_on_the_bottom_visible_row_ignores_the_folded_offscreen_low_note() {
    let (project, visible_id, offscreen_id) = folded_pitch_project();
    let view = ViewState::from_project(&project).expect("投影");

    let visible_index = (0..view.notes.len())
        .find(|&i| view.notes[i].id == visible_id.to_canonical_string())
        .expect("可见音符必须在投影里");
    let offscreen_index = (0..view.notes.len())
        .find(|&i| view.notes[i].id == offscreen_id.to_canonical_string())
        .expect("窗口外低音必须在投影里");
    assert!(
        offscreen_index > visible_index,
        "夹具前提：窗口外低音必须排在后面（修前会抢命中）"
    );

    let (min_pitch, max_pitch) = visible_pitch_range_for(PITCH_LANE_BASE, 16);
    assert!(
        !(min_pitch..=max_pitch).contains(&view.notes[offscreen_index].pitch),
        "夹具前提：48 必须在窗口 {min_pitch}..={max_pitch} 之外"
    );

    let y = pitch_lane_y_with_base(60, PITCH_LANE_BASE) + 6.0;
    let x = view.notes[visible_index].x + view.notes[visible_index].width / 2.0;
    let hit =
        view.hit_test_visible_scoped_with_base(0.0, 10_000.0, x, y, 12.0, None, PITCH_LANE_BASE);
    assert_eq!(
        hit,
        Some(visible_index),
        "点击底行可见音符必须命中它；窗口外低音 48 不得折叠抢命中（实际 {hit:?}）"
    );
}
