//! ⭐ **T-FIX-397 的验收判据**：录音进行中不得换工程（T-FIX-391 守卫的端到端判据）。
//!
//! 复验给出的红对照探针（`zz391_guard_probe.rs`，管理侧 VM 上的临时文件）按仓内判据
//! 形态落成本文件；探针本身不进补丁。装配沿用既有判据：
//! `record_lands_in_pools` 的 `SilenceFactory` 起录 ＋ `open_project_ui` 的
//! `PortOpenProject`（`after` 空实现、`production: None`）。
//!
//! ## 判据要证的句子
//!
//! 1. 起录成功后，守卫读到的录音器**真的在录**（⛔ 不读界面属性）；
//! 2. 录音进行中调 `open_archive_runtime` ⇒ `Err`，且当前工程指纹**一位不动**；
//! 3. 停录之后同一调用 ⇒ `Ok`（守卫只拦"正在录"，⛔ 拦死切换）。
//!
//! ## 反向腿
//!
//! 把 `open_archive_runtime` 里的守卫短路掉 ⇒ ① 的 `is_err` 立刻变红。

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::record_device::SilenceFactory;
use yeban_app::record_wav::{RecordOptions, RecordPlan};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);

fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

fn fixture_project() -> yeban_model::YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    project
}

#[test]
fn guard_refuses_switch_while_recording_and_allows_it_after_stop() {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let project = fixture_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("建窗口");
    tier1.resize(WINDOW).expect("resize");

    let port = Rc::new(UndoPort::new(
        UndoSession::open("t-fix-397", "yeban-app", project.clone(), NOW).expect("开会话"),
    ));
    let dir = std::env::temp_dir().join(format!(
        "yeban-t397-{}-{}",
        std::process::id(),
        yeban_model::ids::EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let wav = dir.join("take.wav");

    let outcome = Rc::new(host::UiRecordOutcome);
    host::wire_record(
        &ui,
        Some(Rc::new(RefCell::new(project.clone()))),
        Some(Rc::clone(&port)),
        RecordPlan {
            path: wav,
            options: options(),
        },
        Rc::new(SilenceFactory),
        outcome,
    );

    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "起录必须成功");
    std::thread::sleep(Duration::from_millis(80));
    assert!(host::recording_in_progress(), "守卫读到的录音器必须正在录");

    let other = yeban_model::samples::default_project();
    let archive = yeban_model::container::ProjectArchive {
        project: other,
        history_dag: Vec::new(),
        proposals: None,
        assets: Vec::new(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };
    let opener = host::PortOpenProject {
        port: Rc::clone(&port),
        production: None,
        quanta: 0,
        after: Rc::new(|_, _, _, _| Ok(())),
    };
    let before = port.fingerprint();

    // ① 录音进行中：必须拒绝，且当前工程指纹一位不动。
    let refused = opener.open_archive_runtime(&ui, "t-fix-397", &archive, None, None);
    assert!(refused.is_err(), "录音进行中切换必须被拦，实得 Ok");
    assert_eq!(port.fingerprint(), before, "被拦的切换不得动当前工程");
    assert!(
        ui.get_recording(),
        "被拦的切换不得改动录音状态（界面与录音器一致）"
    );

    // ② 停录：同一调用必须放行。
    ui.invoke_toggle_record();
    assert!(!ui.get_recording(), "停录之后必须归零");
    assert!(!host::recording_in_progress(), "停录后守卫必须读到未录音");
    let allowed = opener.open_archive_runtime(&ui, "t-fix-397", &archive, None, None);
    assert!(allowed.is_ok(), "停录后切换必须放行，实得 Err: {allowed:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
