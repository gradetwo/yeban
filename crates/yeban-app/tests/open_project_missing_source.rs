//! ISS-MWY4XSFK-71563F：启动选择器打开「缺音源」工程时，即使引擎换代失败，也必须
//! **关掉选择器**、进入工程界面，并摆出可操作的缺失音源面板；错误不能只写状态栏。
//!
//! 判据用**产品形态**：
//! - 真的 `UndoPort` + `PortOpenProject`（`production: Some`，所以引擎换代真的跑）；
//! - 真的落盘容器，里面一条轨绑着一个**未安装**的内置音源路径（SFZ 摊平必失败）；
//! - 真的救援面板装配（启动时传空表，与 `main.rs` 本单后的装配一致）；
//! - 打开走 `wire_open_project`（后台读盘）⇒ `wait_for_pending_open_project` ⇒
//!   `apply_pending_open_project`（与生产排空同一段）。
//!
//! 反向腿：把 `close_startup_chooser` 的 `ui_swapped` 分支去掉 ⇒ 选择器仍开着；
//! 把 `publish_missing_sounds_for_project` 调用去掉 ⇒ 面板不开；把
//! `apply_engine_error` / toast 去掉 ⇒ 可见错误只剩状态栏。三条都会让本判据红。
//!
//! 平台约束：`Tier1Window::install` 每线程一次；本文件两条判据各跑在自己的线程上，
//! 各装一次，互不干扰。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use common::ScriptedPicker;
use slint::Model as _;
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

/// 内置清单里**未安装**的一件（`assets/samples/` 下没有这个目录）。
const MISSING_SFZ: &str = "assets/samples/freepats-drawbar-organ/DrawbarOrganEmulation-SFZ-20190712/DrawbarOrganEmulation-20190712.sfz";

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct SettingsEnv {
    previous: Option<OsString>,
}

impl SettingsEnv {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os(yeban_app::settings::SETTINGS_PATH_ENV);
        // SAFETY: 调用方持有 ENV_LOCK。
        unsafe {
            std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, path);
        }
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
            "yeban-open-missing-source-{}-{}",
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

#[test]
fn missing_source_open_closes_the_chooser_and_shows_the_rescue_panel_even_when_the_engine_fails() {
    let _env_guard = ENV_LOCK.lock().expect("ENV_LOCK");
    let scratch = Scratch::new();
    let _settings = SettingsEnv::set(&scratch.dir.join("settings.json"));

    let initial = yeban_model::samples::project_with_notes(64);
    let (path, expected_path) = {
        let mut project = initial.clone();
        let mut bound = 0usize;
        for track in project.tracks.values_mut() {
            if track.kind == yeban_model::TrackKind::Master {
                continue;
            }
            track.sampler = Some(SamplerBinding {
                note_octave_override: None,
                path: MISSING_SFZ.to_owned(),
            });
            bound += 1;
        }
        assert!(bound > 0, "样本工程必须至少有一条可绑音源的展示轨");
        let archive = yeban_model::container::ProjectArchive {
            project,
            history_dag: Vec::new(),
            proposals: None,
            assets: yeban_model::samples::filled_project_assets(),
            embedded_presets: Vec::new(),
            load_notes: Vec::new(),
        };
        let path = scratch.dir.join("missing-source.yeban");
        yeban_app::save::save_archive_file(&archive, &path).expect("写出缺音源容器");
        (path, MISSING_SFZ.to_owned())
    };

    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<missing-source-open>",
            "yeban-app",
            initial.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    // 与 `main.rs` 本单后的装配一致：启动时没有缺失也装配，运行期换工程才能重播。
    host::wire_missing_sounds_dialog(&ui, Rc::clone(&port), Vec::new());
    assert!(
        !ui.get_missing_sounds_open(),
        "空表装配时救援面板必须保持关闭（默认帧一位不变）"
    );

    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&initial, 0).expect("初始工程必须能起引擎"),
    ));
    assert!(
        production.borrow().has_engine(),
        "判据夹具必须真的起引擎（否则引擎换代根本不会跑）"
    );
    let runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&port),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after: Rc::new(|_, _, _| Ok(())),
    });
    let picker = ScriptedPicker::answering_open(path.clone());
    let picker: Rc<dyn FilePicker> = {
        let owned: Rc<ScriptedPicker> = Rc::clone(&picker);
        owned
    };
    let state = host::wire_open_project(&ui, picker, Rc::clone(&runtime), false);
    ui.set_startup_chooser_open(true);

    ui.invoke_open_project();
    assert!(
        host::wait_for_pending_open_project(&state),
        "后台打开必须交回一个工作线程（join）"
    );
    let applied = host::apply_pending_open_project(&ui, &state, &runtime);

    // ① 引擎换代在 SFZ 摊平处失败 ⇒ 打开结果仍是失败（回执 `outcome=error`），
    //    但界面已经换了。`applied` 只表示"排空真的落了一个结果"，不是成功布尔。
    assert!(applied, "后台结果必须被排空（成功 / 失败都算落了结果）");
    assert_eq!(
        port.try_project()
            .expect("界面已经换了新工程")
            .tracks
            .values()
            .find_map(|track| track.sampler.as_ref())
            .map(|sampler| sampler.path.as_str()),
        Some(expected_path.as_str()),
        "界面 / 端口里的工程必须真的是缺音源的那一份"
    );

    // ② 选择器必须关：否则用户被卡在旧弹窗后面，看不到下面的面板与错误。
    assert!(
        !ui.get_startup_chooser_open(),
        "引擎换代失败时界面已经换掉 ⇒ 启动选择器也必须关闭"
    );

    // ③ 缺失音源面板必须出现，且「下载」是可操作的（这条轨对得上内置清单）。
    assert!(
        ui.get_missing_sounds_open(),
        "缺音源工程打开后必须摆出可操作的救援面板"
    );
    assert!(
        ui.get_missing_row_count() >= 1,
        "面板至少要有这一条缺音源的轨，实际 {}",
        ui.get_missing_row_count()
    );
    assert!(
        ui.get_missing_can_download()
            .iter()
            .any(|can_download| can_download),
        "内置清单里能对上的缺件必须给「下载原音源」入口"
    );
    ui.invoke_missing_close();
    assert!(
        !ui.get_missing_sounds_open(),
        "救援面板的关闭动作必须可操作（点击后关得上）"
    );

    // ④ 错误呈现必须超出状态栏一行：既有引擎错误横幅 + 带恢复路径的提示条。
    let banner = ui.get_engine_error().to_string();
    assert!(
        !banner.is_empty(),
        "引擎换代失败必须点亮 engine-error 横幅（⛔ 只写状态栏）"
    );
    assert!(
        banner.contains("音源") || banner.contains("诊断"),
        "横幅必须给恢复方向（音源管理 / 诊断），实际：{banner}"
    );
    let toast = ui.get_toast_message().to_string();
    assert!(
        toast.contains("丢失音源面板")
            || toast.contains("缺失音源面板")
            || toast.contains("重新打开"),
        "提示条必须写明恢复路径，实际：{toast}"
    );
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("outcome=error") && receipt.contains("引擎换代失败"),
        "回执必须如实登记换代失败，实际：{receipt}"
    );
}

#[test]
fn startup_chooser_open_existing_closes_and_shows_rescue_on_engine_failure() {
    let _env_guard = ENV_LOCK.lock().expect("ENV_LOCK");
    let scratch = Scratch::new();
    let _settings = SettingsEnv::set(&scratch.dir.join("settings.json"));

    let initial = yeban_model::samples::project_with_notes(64);
    let (path, expected_path) = {
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
        let path = scratch.dir.join("missing-source-from-chooser.yeban");
        yeban_app::save::save_archive_file(&archive, &path).expect("写出缺音源容器");
        (path, MISSING_SFZ.to_owned())
    };

    let (_tier1, ui) = window(&initial);
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<missing-source-chooser>",
            "yeban-app",
            initial.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    host::wire_missing_sounds_dialog(&ui, Rc::clone(&port), Vec::new());

    let production = Rc::new(RefCell::new(
        host::ProductionLoop::start(&initial, 0).expect("初始工程必须能起引擎"),
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

    assert_eq!(
        port.try_project()
            .expect("界面已经换了新工程")
            .tracks
            .values()
            .find_map(|track| track.sampler.as_ref())
            .map(|sampler| sampler.path.as_str()),
        Some(expected_path.as_str()),
        "选择器路径必须真的把缺音源工程换进端口"
    );
    assert!(
        !ui.get_startup_chooser_open(),
        "选择器「打开已有工程」遇到换代失败也必须关（⛔ 卡在弹窗后）"
    );
    assert!(
        ui.get_missing_sounds_open() && ui.get_missing_row_count() >= 1,
        "选择器路径也必须摆出可操作的缺失音源面板"
    );
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("outcome=error"),
        "选择器回执必须如实登记打开失败，实际：{receipt}"
    );
    assert!(
        !ui.get_engine_error().to_string().is_empty(),
        "换代失败必须点亮引擎错误横幅"
    );
}
