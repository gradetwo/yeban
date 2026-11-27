//! T-FIX-820：Clip 矩阵片格必须按 (track_id, clip_idx) 选中该轨片段。
//!
//! 旧实现：`on_clip_trigger(_track, clip_idx)` 丢弃轨道，直接取全局
//! `clip-ulids[clip_idx]` ⇒ 点第 2 轨第 1 格会选中第 1 轨的第 1 个片段。
//!
//! 本判据：两轨各 ≥2 片段，点第 2 轨第 1 格；断言 `selected-clip-ulid` 是
//! 第 2 轨投影里的第 1 个片段，且与全局第 1 个不同；回执含“已选中”。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr as _;

use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_test_port::inspect::install_testing_backend;

/// `demo_project()` 的前两条轨各只有 1 个片段；各补一个真实 placement，
/// 得到两轨各 ≥2 片段。新片段的身份是新 ULID，顺序由投影的 BTreeMap 身份序决定。
fn two_tracks_with_two_clips_each() -> YebanProjectV1 {
    let mut project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("demo 投影");
    let track_ids: Vec<EntityId> = view
        .tracks
        .iter()
        .take(2)
        .map(|track| EntityId::from_str(&track.id).expect("轨道 ULID"))
        .collect();
    let template = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .next()
        .copied()
        .expect("demo 至少有一个片段");
    for (slot, track_id) in track_ids.iter().enumerate() {
        let mut placement = template;
        placement.id = EntityId::new();
        placement.start_tick = 2_000 + u64::try_from(slot).unwrap_or(0) * 300;
        placement.duration_ticks = 480;
        project
            .tracks
            .get_mut(track_id)
            .expect("取到目标轨道")
            .clips
            .insert(placement.id, placement);
    }
    project
}

#[test]
fn second_track_first_cell_selects_that_tracks_first_clip() {
    install_testing_backend();

    let project = two_tracks_with_two_clips_each();
    let view = ViewState::from_project(&project).expect("投影两轨片段");
    let scene = DemoScene::from_view(&view);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<t820>", "yeban-app", project, 1_700_000_000_000).expect("打开撤销会话"),
    ));
    let view_cell = Rc::new(RefCell::new(view.clone()));
    let engine = Rc::new(RefCell::new(EngineHost::new()));
    let ui = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");

    // 生产同款顺序：先选中的真实入口，再最后覆盖的 aether actions。
    host::wire_clip_scope(&ui, Rc::clone(&view_cell), Some(Rc::clone(&port)));
    host::wire_aether_actions(&ui, port, engine, view_cell);

    // `TrackData.id` 是 1-based；第 2 轨 = track_index 1。
    let track_index = 1usize;
    let expected = view
        .clips
        .iter()
        .find(|clip| clip.track_index == track_index)
        .map(|clip| clip.placement_id.clone())
        .expect("第 2 轨至少有 2 个片段");
    let global_first = view
        .clips
        .first()
        .map(|clip| clip.placement_id.clone())
        .expect("全局至少有一个片段");
    assert_ne!(expected, global_first, "夹具必须让两轨首片段不同");

    ui.invoke_clip_trigger(2, 0);

    assert_eq!(
        ui.get_selected_clip_ulid().as_str(),
        expected,
        "第 2 轨第 1 格必须选中该轨第 1 个片段，不能选中全局第 1 个"
    );
    assert!(
        ui.get_toast_message().contains("已选中"),
        "回执必须如实写“已选中”；实际={}",
        ui.get_toast_message()
    );
}
