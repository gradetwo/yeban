//! T-FIX-917：首启选择器经「打开已有工程」换入缺音源工程时，界面已经换成新工程
//! 但引擎换代失败 —— 选择器必须关，错误必须超出报错回执行，而不是把用户卡在弹窗后。
//!
//! 这是可复现的 Linux 无头探针：真 `UndoPort` + 真 `PortOpenProject`（`production:
//! Some`）+ 真落盘缺音源容器 + 真 `wire_startup_chooser`。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use common::ScriptedPicker;
use yeban_app::bridge::ViewState;
use yeban_app::file_picker::FilePicker;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::SamplerBinding;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::render::Tier1Window;

const NOW_MS: u64 = 1_760_000_000_000;
const MISSING_SFZ: &str = "assets/samples/freepats-drawbar-organ/DrawbarOrganEmulation-SFZ-20190712/DrawbarOrganEmulation-20190712.sfz";

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct SettingsEnv {
    previous: Option<OsString>,
}

impl SettingsEnv {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os(yeban_app::settings::SETTINGS_PATH_ENV);
        // SAFETY: 调用方持有 ENV_LOCK。
        unsafe { std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, path) };
        Self { previous }
    }
}

impl Drop for SettingsEnv {
    fn drop(&mut self) {
        // SAFETY: 调用方持有 ENV_LOCK，且本守卫先于它析构。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, value),
                None => std::env::remove_var(yeban_app::settings::SETTINGS_PATH_ENV),
            }
        }
    }
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-917-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn window(initial: &yeban_model::YebanProjectV1) -> (Tier1Window, MainWindow) {
    let tier1 = Tier1Window::install(Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(initial).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 1).expect("生产窗口构造入口");
    (tier1, ui)
}

/// 落一份每条非主总线轨都绑**未安装** SFZ 的工程。
fn missing_source_archive(scratch: &Scratch) -> (PathBuf, yeban_model::YebanProjectV1) {
    let initial = yeban_model::samples::project_with_notes(64);
    let mut project = initial.clone();
    for track in project.tracks.values_mut() {
        if track.kind == yeban_model::TrackKind::Master {
            continue;
        }
        track.sampler = Some(SamplerBinding {
            note_octave_override: None,
            path: MISSING_SFZ.to_owned(),
        });
    }
    let archive = yeban_model::container::ProjectArchive {
        project,
        history_dag: Vec::new(),
        proposals: None,
        assets: yeban_model::samples::filled_project_assets(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };
    let path = scratch.dir.join("from-chooser-missing-source.yeban");
    yeban_app::save::save_archive_file(&archive, &path).expect("写出缺音源容器");
    (path, initial)
}

#[test]
fn startup_open_existing_closes_the_chooser_when_the_engine_fails_after_the_ui_swapped() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = Scratch::new();
    let _settings = SettingsEnv::set(&scratch.dir.join("settings.json"));
    let (path, initial) = missing_source_archive(&scratch);
    let (tier1, ui) = window(&initial);

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<917>", "yeban-app", initial.clone(), NOW_MS).expect("撤销会话"),
    ));
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&initial, 0).expect("初始工程起引擎"),
    ));
    assert!(production.borrow().has_engine(), "夹具必须真的起引擎");
    let concrete = Rc::new(host::PortOpenProject {
        port: Rc::clone(&port),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    });
    let titled: Rc<dyn host::OpenProjectRuntime> = concrete.clone();
    let picker: Rc<dyn FilePicker> = ScriptedPicker::answering_open(path.clone());
    let roots = yeban_app::templates::TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    host::wire_startup_chooser(
        &ui,
        picker,
        titled,
        concrete,
        roots,
        yeban_app::startup_chooser::NewProjectDetails::from_project(&initial),
    );
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_open_existing();

    // 前提：界面/端口真的已经换到缺音源工程（引擎换代在 SFZ 摊平处失败）。
    let loaded_sampler = port
        .try_project()
        .expect("界面已经换了新工程")
        .tracks
        .values()
        .find_map(|track| track.sampler.as_ref())
        .map(|sampler| sampler.path.clone());
    assert_eq!(
        loaded_sampler.as_deref(),
        Some(MISSING_SFZ),
        "选择器路径必须真的把缺音源工程换进端口"
    );

    // ① 用户已看到主界面换了工程 ⇒ 选择器不得继续挡着。
    assert!(
        !ui.get_startup_chooser_open(),
        "界面已换工程后选择器必须自动关闭（⛔ 让用户点取消才消失）"
    );
    // ② 失败必须超出状态栏：引擎错误横幅 + 带恢复路径的 toast。
    assert!(
        !ui.get_engine_error().to_string().is_empty(),
        "引擎换代失败必须点亮 engine-error 横幅"
    );
    let toast = ui.get_toast_message().to_string();
    assert!(
        toast.contains("音源") || toast.contains("诊断") || toast.contains("重新打开"),
        "提示条必须给恢复路径，实际：{toast}"
    );
    // ③ 回执仍如实登记换代失败（不是假成功）。
    assert!(
        !ui.get_module_status_succeeded(),
        "回执必须标成失败，不能假成功"
    );
    let receipt = ui.get_module_status_text().to_string();
    assert!(
        receipt.contains("引擎换代失败"),
        "回执必须登记失败原因，实际：{receipt}"
    );

    drop(tier1);
}

/// 文件对话框取消：选择器留着、工程一位不动、不得误报成功。
#[test]
fn startup_open_existing_cancel_keeps_the_chooser_and_the_project_untouched() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = Scratch::new();
    let _settings = SettingsEnv::set(&scratch.dir.join("settings.json"));
    let initial = yeban_model::samples::project_with_notes(64);
    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<917-cancel>", "yeban-app", initial.clone(), NOW_MS).expect("撤销会话"),
    ));
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&initial, 0).expect("起引擎"),
    ));
    let concrete = Rc::new(host::PortOpenProject {
        port: Rc::clone(&port),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    });
    let titled: Rc<dyn host::OpenProjectRuntime> = concrete.clone();
    let picker: Rc<dyn FilePicker> = ScriptedPicker::cancelling();
    let roots = yeban_app::templates::TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    host::wire_startup_chooser(
        &ui,
        picker,
        titled,
        concrete,
        roots,
        yeban_app::startup_chooser::NewProjectDetails::from_project(&initial),
    );
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_open_existing();
    assert!(
        ui.get_startup_chooser_open(),
        "文件对话框取消不是打开成功 ⇒ 选择器必须留着"
    );
    assert_eq!(
        port.try_project().expect("原工程仍在").title,
        initial.title,
        "取消后当前工程必须一位不动"
    );
    assert!(
        !ui.get_module_status_text()
            .to_string()
            .contains("outcome=ok"),
        "取消不得误报成功"
    );
}

/// 打开真失败（文件不可读）且界面没换：选择器留着，回执明确失败。
#[test]
fn startup_open_existing_bad_file_keeps_the_chooser_and_reports_failure() {
    let _env_guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = Scratch::new();
    let _settings = SettingsEnv::set(&scratch.dir.join("settings.json"));
    let initial = yeban_model::samples::project_with_notes(64);
    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<917-bad>", "yeban-app", initial.clone(), NOW_MS).expect("撤销会话"),
    ));
    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&initial, 0).expect("起引擎"),
    ));
    let concrete = Rc::new(host::PortOpenProject {
        port: Rc::clone(&port),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    });
    let titled: Rc<dyn host::OpenProjectRuntime> = concrete.clone();
    let bad_path = scratch.dir.join("not-a-project.yeban");
    std::fs::write(&bad_path, b"not a project").expect("写坏文件");
    let picker: Rc<dyn FilePicker> = ScriptedPicker::answering_open(bad_path);
    let roots = yeban_app::templates::TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    host::wire_startup_chooser(
        &ui,
        picker,
        titled,
        concrete,
        roots,
        yeban_app::startup_chooser::NewProjectDetails::from_project(&initial),
    );
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_open_existing();
    assert!(
        ui.get_startup_chooser_open(),
        "界面未换的打开失败必须留在选择器里"
    );
    assert!(
        !ui.get_module_status_succeeded(),
        "打开失败必须标成失败回执"
    );
    let receipt = ui.get_module_status_text().to_string();
    assert!(
        receipt.contains("失败") && receipt.contains("当前工程一位没动"),
        "打开失败必须响亮回执，实际：{receipt}"
    );
    assert_eq!(
        port.try_project().expect("原工程仍在").title,
        initial.title,
        "打开失败后当前工程必须一位不动"
    );
}
