//! T-FIX-721：空工程导出必须在**用户可见的回执**里说清「无可渲染内容、导出为静音」。
//!
//! 修前：`render_core` 的 `render-empty` 检查只 `eprintln!`；GUI 成功回执只有
//! 「导出 WAV 完成」，用户拿到全静音 WAV 却不知道原因（dou-win-133 实机）。
//!
//! 本判据覆盖两路：
//! 1. 空工程（只有 Master）：报告 `render_empty == true`、`silence_note()` 含
//!    「无可渲染内容」与「静音」；GUI 回执含 `silent_export=true`，可见警示行与提示条
//!    都明确说「全静音」。
//! 2. 有内容（一条 MIDI clip）：报告 `render_empty == false`、无 `silence_note()`；
//!    GUI 回执**逐字等于**既有 `wav_export_receipt`（⛔ 不多字段），警示行与提示条
//!    都不出现「静音」⇒ 不误报。
//!
//! 红对照：删掉 host 的 `silent_export=true` / 警示行 ⇒ 空工程臂红；把提示改成无条件
//! 出现 ⇒ 有内容臂红。
//!
//! 平台约束：`slint::platform::set_platform` 是线程局部的，本文件只有一个 `#[test]`。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
mod live;

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr as _;

use common::wav_export_receipt;
use yeban_app::bridge::ViewState;
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::host;
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1, samples::default_project,
};
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;

fn drain_event_loop() {
    slint::quit_event_loop().expect("带事件循环的后端必须提供 proxy");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

/// 经真窗口导出一次，返回（结构化回执，成功标志，可见警示行，提示条）。
fn export_through_gui(
    project: &YebanProjectV1,
    target: &Path,
    label: &str,
) -> (String, bool, String, String) {
    let port = Rc::new(UndoPort::new(
        UndoSession::open(label, "yeban-app", project.clone(), 1).expect("打开撤销会话"),
    ));
    let view = ViewState::from_project(project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let mut paths = ModulePaths::from_env();
    paths.export_wav = target.to_path_buf();
    let _midi = wire_module_actions_with(
        &ui,
        Some(Rc::clone(&port)),
        paths,
        Rc::new(yeban_app::file_picker::NoPicker),
    );

    assert_eq!(ui.get_module_status().as_str(), "", "导出前回执应为空");
    ui.invoke_export_wav();
    assert!(
        host::wait_for_pending_export(),
        "必须有一个刚结束的导出工作线程可 join"
    );
    drain_event_loop();
    (
        ui.get_module_status().to_string(),
        ui.get_module_status_succeeded(),
        ui.get_module_clipping_warning().to_string(),
        ui.get_toast_message().to_string(),
    )
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-721-{tag}-{}-{}",
        std::process::id(),
        EntityId::new()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn sha_hex(path: &Path) -> String {
    AssetHash::of_bytes(&std::fs::read(path).expect("读产物"))
        .as_str()
        .to_owned()
}

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID")
}

/// Master ＋ 一条带 MIDI clip 的轨（与 `gui_export_clipping.rs` 同款最小内容工程）。
fn project_with_clip() -> YebanProjectV1 {
    let master = fixture_id("A1");
    let track_id = fixture_id("A2");
    let clip_id = fixture_id("A3");
    let placement_id = fixture_id("A4");
    let edge = fixture_id("A5");

    let mut project = YebanProjectV1 {
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut notes = BTreeMap::new();
    for (index, tick) in [0_u64, 480, 960, 1440].into_iter().enumerate() {
        let id = fixture_id(&format!("B{index}"));
        notes.insert(id, MidiNote::new(id, tick, 60, 240));
    }
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "Motif".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    let mut track = TrackV3 {
        id: track_id,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: 1920,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(master);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    project
}

#[test]
fn an_empty_export_is_visibly_labelled_silent_and_a_content_export_is_not() {
    install_testing_backend_with_event_loop();
    let dir = scratch_dir("silence");

    // ---------- 空工程：报告 + GUI 回执都必须响亮 ----------
    let empty = default_project();
    let empty_options = WavExportOptions::resolve(&empty, None, None, None, None);
    let empty_oracle = dir.join("empty-oracle.wav");
    let direct = export_wav(&empty, &empty_oracle, empty_options).expect("空工程导出必须成功");
    assert!(direct.render_empty, "只有 Master 的工程必须标记为空导出");
    let note = direct.silence_note().expect("空导出必须有 silence_note");
    assert!(
        note.contains("无可渲染内容") && note.contains("静音"),
        "silence_note 必须同时说清原因与静音结果: {note}"
    );
    assert!(
        direct.summary().contains("无可渲染内容"),
        "summary() 也必须多出这一行: {}",
        direct.summary()
    );

    let empty_path = dir.join("empty-gui.wav");
    let (empty_receipt, empty_ok, empty_warning, empty_toast) =
        export_through_gui(&empty, &empty_path, "gui empty");
    assert!(
        empty_ok,
        "空工程导出必须成功（本单选择完成回执标注，而非阻止）"
    );
    assert!(
        empty_receipt.contains(":silent_export=true"),
        "空导出回执必须含 silent_export=true: {empty_receipt}"
    );
    assert!(
        empty_warning.contains("无可渲染内容") && empty_warning.contains("静音"),
        "空导出的可见警示行必须说清静音: {empty_warning}"
    );
    assert!(
        empty_toast.contains("全静音"),
        "成功提示条必须明确写「全静音」: {empty_toast}"
    );
    assert_eq!(
        sha_hex(&empty_path),
        sha_hex(&empty_oracle),
        "提示只是文字：GUI 产物与直接调用必须逐字节一致"
    );

    // ---------- 有内容：回执逐字不变，且不得误报 ----------
    let content = project_with_clip();
    let content_options = WavExportOptions::resolve(&content, None, None, None, None);
    let content_oracle = dir.join("content-oracle.wav");
    let content_direct =
        export_wav(&content, &content_oracle, content_options).expect("有内容工程导出必须成功");
    assert!(
        !content_direct.render_empty,
        "有一条 clip 的工程不得被标记为空导出"
    );
    assert!(
        content_direct.silence_note().is_none(),
        "有内容时 silence_note 必须为 None"
    );

    let content_path = dir.join("content-gui.wav");
    let (content_receipt, content_ok, content_warning, content_toast) =
        export_through_gui(&content, &content_path, "gui content");
    assert!(content_ok, "有内容工程导出必须成功: {content_receipt}");
    assert!(
        !content_receipt.contains("silent_export"),
        "有内容回执不得出现静音字段: {content_receipt}"
    );
    assert_eq!(
        content_receipt,
        wav_export_receipt(&content_direct, &content_path, false),
        "有内容时回执必须与既有形状逐字相同（无条件加字段 ⇒ 本条红）"
    );
    assert!(
        content_warning.is_empty(),
        "有内容时警示行必须为空（无条件提示 ⇒ 本条红）: {content_warning}"
    );
    assert!(
        !content_toast.contains("全静音"),
        "有内容时提示条不得误报静音: {content_toast}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
