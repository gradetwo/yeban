//! T-FIX-806：GUI 打开一个引用了 `user:` 预制的工程时，必须与 MCP 打开路径
//! 走**同一段**校验：解析内嵌副本 + 校验全部轨引用。存在内嵌副本 ⇒ 可打开且
//! 副本随运行期端口保留；不存在 ⇒ 拒绝打开、当前工程一位不动、回执响亮。
//!
//! 反向腿：把 `strict_preset_references` 打开前，缺失副本的工程会被静默打开
//! （本文件第二条判据在修前必红）。

#![allow(unused)]

#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::open::{OpenError, ProjectOpenOptions, open_project_document_file};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession, project_fingerprint};
use yeban_model::Preset;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::Tier1Window;

const NOW_MS: u64 = 1_760_000_000_000;
const USER_ID: &str = "user:01J806PRESETOPEN0000000000";

/// 临时目录（`Drop` 时清理，绝不污染仓库）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-preset-open-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn strict_options() -> ProjectOpenOptions {
    ProjectOpenOptions {
        strict_missing_assets: true,
        strict_preset_references: true,
        ..ProjectOpenOptions::default()
    }
}

fn project_with_user_preset(id: &str) -> yeban_model::YebanProjectV1 {
    let mut project = yeban_model::samples::demo_project();
    // demo 工程带音频摆放，本判据只测预制校验；清空片段与资产引用，
    // 避免 `strict_missing_assets` 先于预制校验报缺资产。
    project.clip_pool.clear();
    project.assets.clear();
    project.embedded_samples.clear();
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    let track_id = project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("默认工程至少有一条非主总线轨")
        .id;
    let track = project.tracks.get_mut(&track_id).expect("轨存在");
    track.preset = Some(Box::new(yeban_model::preset::TrackPresetState {
        preset_id: Some(id.to_owned()),
        ..Default::default()
    }));
    project
}

fn user_preset_bytes(id: &str) -> Vec<u8> {
    let mut preset =
        Preset::parse(include_str!("../../../assets/presets/piano-warm-room.json").as_bytes())
            .expect("工厂预制必须能解析");
    preset.id = id.to_owned();
    preset.name = "T-FIX-806 embedded user preset".to_owned();
    serde_json::to_vec(&preset).expect("预制必须能序列化")
}

fn write_container(
    path: &Path,
    project: &yeban_model::YebanProjectV1,
    embedded: Option<(&str, &[u8])>,
) {
    let entries: Vec<(&str, &[u8])> = embedded.into_iter().collect();
    let bytes = yeban_model::container::write_project_container_with_presets_borrowed(
        project,
        b"",
        &[],
        &entries,
    )
    .expect("写出容器");
    std::fs::write(path, bytes).expect("落盘");
}

fn window(initial: &yeban_model::YebanProjectV1) -> (Tier1Window, MainWindow) {
    let tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(initial).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("构造窗口");
    (tier1, ui)
}

fn runtime(port: &Rc<UndoPort>) -> Rc<dyn host::OpenProjectRuntime> {
    Rc::new(host::PortOpenProject {
        port: Rc::clone(port),
        production: None,
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    })
}

/// 判据 1：引用 `user:` 预制、用户库没有、工程包里**有**内嵌副本 ⇒ 可打开，
/// 且副本留在运行期端口（供 T-FIX-807 面板消费）。
#[test]
fn embedded_user_preset_opens_and_is_retained() {
    let scratch = Scratch::new("embedded-present");
    let entry = yeban_model::preset::embedded_entry_name(USER_ID).expect("条目名");
    let preset = user_preset_bytes(USER_ID);
    let project = project_with_user_preset(USER_ID);
    let path = scratch.join("embedded.yeban");
    write_container(&path, &project, Some((&entry, &preset)));

    let opened =
        open_project_document_file(&path, &strict_options()).expect("内嵌副本存在时必须可打开");
    assert_eq!(
        opened.archive.embedded_presets.len(),
        1,
        "内嵌副本必须随归档读回"
    );
    assert_eq!(opened.archive.embedded_presets[0].0, entry);
    assert_eq!(opened.archive.embedded_presets[0].1, preset);

    let initial = yeban_model::samples::demo_project();
    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<preset-open>", "yeban-app", initial, NOW_MS).expect("会话"),
    ));
    let runtime = runtime(&port);
    runtime
        .open_archive(
            &ui,
            "embedded.yeban",
            opened.archive,
            Some(scratch.dir.as_path()),
            Some(path.as_path()),
        )
        .expect("内嵌副本存在时运行期换工程必须成功");
    assert_eq!(
        port.embedded_presets(),
        vec![(entry, preset)],
        "内嵌副本必须保留在运行期端口，供面板 / 保存消费"
    );
}

/// 判据 2：同一引用、但工程包里**没有**内嵌副本、用户库也没有 ⇒ 拒绝打开；
/// 当前工程一位不动，回执写明缺失标识与"一位没动"。
#[test]
fn missing_user_preset_refuses_open_and_keeps_current_project() {
    let scratch = Scratch::new("embedded-missing");
    let project = project_with_user_preset(USER_ID);
    let path = scratch.join("missing.yeban");
    write_container(&path, &project, None);

    let err = open_project_document_file(&path, &strict_options())
        .expect_err("缺内嵌副本且用户库没有时必须拒绝打开");
    match err {
        OpenError::PresetReference { detail } => {
            assert!(
                detail.contains(USER_ID),
                "错误必须点名缺失预制，实际：{detail}"
            );
            assert!(
                detail.contains("缺用户预制"),
                "错误必须是缺用户预制，实际：{detail}"
            );
        }
        other => panic!("必须是 PresetReference，实际：{other}"),
    }

    // 端到端：后台打开 ⇒ 排空 ⇒ 当前工程指纹不变 + 回执 outcome=error。
    let initial = yeban_model::samples::demo_project();
    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<preset-open-missing>", "yeban-app", initial, NOW_MS).expect("会话"),
    ));
    let before = port.fingerprint().expect("初始指纹");
    let picker: Rc<dyn yeban_app::file_picker::FilePicker> =
        ScriptedPicker::answering_open(path.clone());
    let runtime = runtime(&port);
    let state = host::wire_open_project(&ui, picker, Rc::clone(&runtime), false);
    ui.invoke_open_project();
    assert!(
        host::wait_for_pending_open_project(&state),
        "必须等到工作线程"
    );
    assert!(
        host::apply_pending_open_project(&ui, &state, &runtime),
        "必须排空结果"
    );
    assert_eq!(
        port.fingerprint().as_deref(),
        Some(before.as_str()),
        "打开失败后当前工程必须一位不动"
    );
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("outcome=error"),
        "回执必须是 outcome=error，实际：{receipt}"
    );
    assert!(
        receipt.contains(USER_ID),
        "回执必须点名缺失预制，实际：{receipt}"
    );
    assert!(
        receipt.contains("当前工程一位没动"),
        "回执必须说明当前工程未变，实际：{receipt}"
    );
}
