//! 新 UI 钢琴卷帘的**真实数据与编辑**判据（报告反馈 ①③④）。
//!
//! 旧实现是 16×64 的演示网格：音符来自 `root.notes`（所有轨/所有片段），
//! 点击映射到"第一条轨/第一个 MIDI 片段"，因此：
//! * 卷帘内容与选中轨/片段不一致；
//! * 点击无法落到真实工程（等于不能编辑）。
//!
//! 本判据把`MainWindow`的钢琴卷帘替换成 `console/piano_roll.slint` 之后钉住：
//! 1. 卷帘音符 = **选中片段**的音符（不是全工程）；
//! 2. 铅笔点击提交 `Op::AddNote` 到**该片段**，工程音符 +1；
//! 3. 横向缩放 / 滚动改的是同一条投影（`ticks-per-pixel` / `roll-scroll-x`）。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model;
use yeban_app::bridge::{
    NOTE_INSET_X_PX, NOTE_INSET_Y_PX, PITCH_LANE_HEIGHT_PX, ROLL_HEADER_HEIGHT_PX,
    ROLL_KEY_COLUMN_WIDTH_PX, ViewState,
};
use yeban_app::host;
use yeban_app::host::ProductionLoop;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

fn note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .map(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum()
}

fn first_midi_clip(project: &YebanProjectV1) -> (EntityId, std::collections::BTreeSet<EntityId>) {
    for (id, entry) in &project.clip_pool {
        if let ClipContent::Midi { notes } = &entry.content {
            return (*id, notes.keys().copied().collect());
        }
    }
    panic!("夹具必须有 MIDI 片段");
}

#[test]
fn piano_roll_shows_the_selected_clip_and_edits_reach_the_project() {
    let project = yeban_model::samples::filled_project();
    let (clip_id, clip_notes) = first_midi_clip(&project);
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<aether-roll>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::without_engine();
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_roll_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_zoom_controls(ui, &undo);
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(&undo)));
    host::wire_aether_actions(ui, Rc::clone(&undo), engine, view_cell);

    // ① 选中那个片段（与用户点时间线块 / 片段头同一条入口）⇒ 卷帘只画它的音符。
    let placement = yeban_app::bridge::ViewState::from_project(&project)
        .unwrap()
        .clips
        .iter()
        .find(|clip| clip.clip_id == clip_id.to_canonical_string())
        .map(|clip| (clip.placement_id.clone(), clip.track_index))
        .expect("片段必须有摆放");
    ui.invoke_clip_select(placement.0.into());
    let drawn = ui.get_note_ulids();
    let drawn_ids: Vec<String> = drawn.iter().map(|id| id.to_string()).collect();
    assert_eq!(
        drawn_ids.len(),
        clip_notes.len(),
        "卷帘音符数必须等于选中片段的音符数（不是全工程 {}）",
        note_count(&project)
    );
    for id in &drawn_ids {
        assert!(
            clip_notes
                .iter()
                .any(|note| note.to_canonical_string() == *id),
            "卷帘出现了片段外的音符 {id}"
        );
    }
    eprintln!(
        "[roll/scope] clip {} 条音符 -> 卷帘 {} 条",
        clip_notes.len(),
        drawn_ids.len()
    );

    // ② 铅笔点击：坐标按卷帘的键盘列 / 行头 / 泳道换算；提交到同一片段。
    let notes_before = note_count(&undo.try_project().unwrap());
    ui.set_active_tool(2);
    let x = ROLL_KEY_COLUMN_WIDTH_PX + NOTE_INSET_X_PX + 40.0;
    let y = ROLL_HEADER_HEIGHT_PX + NOTE_INSET_Y_PX + PITCH_LANE_HEIGHT_PX * 3.0 + 4.0;
    ui.invoke_clicked(x, y);
    let notes_after = note_count(&undo.try_project().unwrap());
    assert!(
        notes_after > notes_before,
        "铅笔点击必须提交真实 Op::AddNote（{notes_before} -> {notes_after}）"
    );
    let edited = undo.try_project().unwrap();
    let in_clip = edited
        .clip_pool
        .get(&clip_id)
        .and_then(|entry| entry.content.notes())
        .map_or(0, std::collections::BTreeMap::len);
    assert_eq!(
        in_clip,
        clip_notes.len() + 1,
        "新音符必须落在选中片段里，实测 {in_clip}"
    );
    eprintln!("[roll/edit] project {notes_before} -> {notes_after}, clip = {in_clip}");

    // ③ 横向缩放 / 滚动：同一份投影（ticks-per-pixel / roll-scroll-x）。
    let tpp_before = ui.get_roll_ticks_per_pixel();
    ui.invoke_zoom_h_step(true);
    let tpp_after = ui.get_roll_ticks_per_pixel();
    assert!(
        tpp_after < tpp_before,
        "H+ 必须放大卷帘：{tpp_before} -> {tpp_after}"
    );
    // 夹具只有 1 小节：先连续放大，让内容宽超过视口，滚动才有可移动的余量。
    for _ in 0..6 {
        ui.invoke_zoom_h_step(true);
    }
    let scroll_before = ui.get_roll_scroll_x();
    ui.invoke_scroll_requested(160.0);
    let scroll_after = ui.get_roll_scroll_x();
    eprintln!(
        "[roll/zoom] tpp={} content={} viewport=1440 scroll {scroll_before}->{scroll_after}",
        ui.get_roll_ticks_per_pixel(),
        ui.get_arrangement_content_px()
    );
    assert!(
        scroll_after > scroll_before,
        "滚轮 / 拖动必须横向滚动卷帘：{scroll_before} -> {scroll_after}"
    );
    eprintln!("[roll/zoom] tpp {tpp_before}->{tpp_after}, scroll {scroll_before}->{scroll_after}");
}
