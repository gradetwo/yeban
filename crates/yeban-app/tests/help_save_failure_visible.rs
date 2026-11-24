//! T-FIX-311 判据：引导 / 课程落盘失败必须被用户看见。
//!
//! * 写失败注入：`YEBAN_SETTINGS_PATH` 指到「父级是普通文件」的路径——
//!   `settings::save_to` 先 `create_dir_all`，root 也绕不过权限位这一关。
//! * 引导（`onboarding_dismiss_forever` / 走完 `onboarding_next`）：
//!   写失败走 `report_notice` 错误级常驻提示（toast + 提示条），层照关、
//!   本次照常生效，但落盘读回仍是默认。
//! * 课程（开课 / 完成一步）：写失败走课程反馈区，课照开、步照推进，
//!   反馈必须写明「本次已生效，但下次启动可能回退」。
//! * 成功路径回归：可写路径下不弹错误提示，状态真落盘、反馈清空。

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use yeban_app::bridge::ViewState;
use yeban_app::help_state::{self, HelpState};
use yeban_app::host::{self, ProductionLoop};
use yeban_app::input::InputContext;
use yeban_app::scene::DemoScene;
use yeban_app::settings;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

// ------------------------------------------------------------ 环境变量守卫

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 两个存储路径变量的 RAII 守卫（Drop 时恢复原值），与
/// `onboarding_help.rs` 同款：`YEBAN_SETTINGS_PATH`（settings 主文件）
/// 与 `YEBAN_ONBOARDING_FILE`（旧独立文件，只在折入时定位）。
///
/// SAFETY（Rust 2024 的 `set_var` 是 `unsafe`）：本文件全部 env 判据先拿
/// [`ENV_LOCK`]，持锁期间本测试二进制内没有其它线程读这两个变量。
struct StatePathEnv {
    previous_settings: Option<OsString>,
    previous_onboarding: Option<OsString>,
}

impl StatePathEnv {
    /// ⚠ 调用方必须先持有 [`ENV_LOCK`]。
    fn set(settings_path: &Path, legacy_path: &Path) -> Self {
        let previous_settings = std::env::var_os(settings::SETTINGS_PATH_ENV);
        let previous_onboarding = std::env::var_os(help_state::HELP_STATE_PATH_ENV);
        // SAFETY: 调用方持有 ENV_LOCK；见类型注释。
        unsafe {
            std::env::set_var(settings::SETTINGS_PATH_ENV, settings_path);
            std::env::set_var(help_state::HELP_STATE_PATH_ENV, legacy_path);
        }
        Self {
            previous_settings,
            previous_onboarding,
        }
    }
}

impl Drop for StatePathEnv {
    fn drop(&mut self) {
        // SAFETY: 守卫在 ENV_LOCK 释放前析构，恢复进入时读到的原值。
        unsafe {
            match &self.previous_settings {
                Some(value) => std::env::set_var(settings::SETTINGS_PATH_ENV, value),
                None => std::env::remove_var(settings::SETTINGS_PATH_ENV),
            }
            match &self.previous_onboarding {
                Some(value) => std::env::set_var(help_state::HELP_STATE_PATH_ENV, value),
                None => std::env::remove_var(help_state::HELP_STATE_PATH_ENV),
            }
        }
    }
}

fn temp_state_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yeban-help311-{tag}-{}.json", std::process::id()))
}

fn temp_legacy_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yeban-help311-legacy-{tag}-{}.json",
        std::process::id()
    ))
}

/// 写失败注入：返回 `(settings 路径, 占位文件路径)`——settings 路径的父级
/// 是一个普通文件，`save_to` 的 `create_dir_all` 必败；用完删掉占位文件。
fn broken_settings_path(tag: &str) -> (PathBuf, PathBuf) {
    let blocker = std::env::temp_dir().join(format!(
        "yeban-help311-blocker-{tag}-{}",
        std::process::id()
    ));
    std::fs::write(&blocker, b"not a directory").expect("写占位文件");
    (blocker.join("settings.json"), blocker)
}

// ------------------------------------------------------------ 活窗口装配

fn build_live_window(
    project: &YebanProjectV1,
    undo: &Rc<UndoPort>,
) -> (
    yeban_ui_test_port::render::LivePort<yeban_app::ui::MainWindow>,
    ProductionLoop,
) {
    // T-FIX-328：T-FIX-299 的待发警告台账是进程内全局的（取走即清空），
    // 本文件 6 条测试共用一个进程；上一条失败测试留下的警告会在建窗时
    // 被 report_pending_settings_warnings 取走并弹 toast，污染下一条测
    // 试的窗口。生产进程每次启动台账都是空的，这里先清掉，恢复该不变量。
    // 各条测试自己的动作都在建窗之后，照常记警告、照常断言。
    let _ = settings::take_pending_warnings();
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let production = ProductionLoop::start(project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));
    let project_rc = Rc::new(project.clone());

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    host::wire_transport(
        ui,
        Rc::clone(&engine),
        Some(Rc::clone(&project_rc)),
        Some(Rc::clone(undo)),
    );
    host::wire_view_callbacks(ui);
    host::wire_keys(
        ui,
        Rc::new(RefCell::new(InputContext::default())),
        Some(Rc::clone(undo)),
        Some(Rc::clone(&engine)),
        Some(Rc::clone(&project_rc)),
    );
    host::wire_bpm_commit(ui, undo);
    host::wire_mixer_edit(ui, undo);
    host::wire_track_arm(ui);
    host::wire_track_select(ui, undo);
    host::wire_tool_select(ui, Some(Rc::clone(undo)));
    host::wire_track_strip(ui, undo);
    host::wire_undo(ui, undo);
    host::wire_scene_launch(ui, Rc::clone(&engine), Rc::clone(&view_cell));
    host::wire_clip_scope(ui, Rc::clone(&view_cell), Some(Rc::clone(undo)));
    host::wire_sidebar_contents(ui, Rc::clone(&view_cell));
    let _recorder = host::wire_record(
        ui,
        Some(Rc::clone(&project_rc)),
        Some(Rc::clone(undo)),
        yeban_app::record_wav::RecordPlan::production(),
        Rc::new(yeban_app::record_device::SilenceFactory),
        Rc::new(host::UiRecordOutcome),
    );
    host::wire_aether_actions(ui, Rc::clone(undo), engine, view_cell);
    (live, production)
}

fn open_undo(project: &YebanProjectV1) -> Rc<UndoPort> {
    Rc::new(UndoPort::new(
        UndoSession::open("<help311>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ))
}

// ------------------------------------------------------------ 引导：写失败

#[test]
fn onboarding_dismiss_forever_save_failure_shows_error_notice() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let (settings_path, blocker) = broken_settings_path("dismiss");
    let legacy = temp_legacy_path("dismiss");
    let _guard = StatePathEnv::set(&settings_path, &legacy);

    let project = yeban_model::samples::filled_project();
    let undo = open_undo(&project);
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    ui.invoke_onboarding_dismiss_forever();
    assert!(!ui.get_onboarding_open(), "不再显示必须关层（本次已生效）");
    assert!(
        ui.get_show_floating_toast(),
        "写失败必须弹出提示，不能只打 stderr"
    );
    assert!(
        ui.get_toast_is_error(),
        "写失败必须是错误级提示（信息级 4 秒就收走了）"
    );
    let toast = ui.get_toast_message();
    assert!(
        toast.contains("未能保存"),
        "提示必须点名未能保存，实际：{toast}"
    );
    assert!(
        toast.contains("回退"),
        "提示必须说明下次启动可能回退，实际：{toast}"
    );
    assert!(
        ui.get_module_status_text().contains("未能保存"),
        "提示条（module-status）也必须写同一句话"
    );
    // 没落盘：读回仍是默认，不许假装已保存。
    assert_eq!(
        help_state::load_from(&settings_path),
        HelpState::default(),
        "写失败不许假装已落盘"
    );

    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_file(&legacy);
}

#[test]
fn onboarding_finish_save_failure_shows_error_notice() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let (settings_path, blocker) = broken_settings_path("finish");
    let legacy = temp_legacy_path("finish");
    let _guard = StatePathEnv::set(&settings_path, &legacy);

    let project = yeban_model::samples::filled_project();
    let undo = open_undo(&project);
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    assert!(
        yeban_app::help::show_onboarding_if_first_launch(ui),
        "全新状态下首次启动必须自动打开引导"
    );
    // 七步导览：六次切步到最后一步，第七次走完。
    for _ in 0..7 {
        ui.invoke_onboarding_next();
    }
    assert!(!ui.get_onboarding_open(), "走完必须关层（本次已生效）");
    assert!(
        ui.get_show_floating_toast() && ui.get_toast_is_error(),
        "走完落盘失败必须是错误级提示"
    );
    let toast = ui.get_toast_message();
    assert!(
        toast.contains("新手引导设置未能保存"),
        "走完的提示必须点名新手引导设置未能保存，实际：{toast}"
    );
    assert!(toast.contains("回退"), "提示必须说明下次启动可能回退");
    assert!(
        !help_state::load_from(&settings_path).onboarding_finished,
        "写失败不许假装已走完"
    );

    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_file(&legacy);
}

// ------------------------------------------------------------ 课程：写失败

#[test]
fn start_lesson_save_failure_warns_in_feedback_but_still_starts() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let (settings_path, blocker) = broken_settings_path("start");
    let legacy = temp_legacy_path("start");
    let _guard = StatePathEnv::set(&settings_path, &legacy);

    let project = yeban_model::samples::filled_project();
    let undo = open_undo(&project);
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    ui.invoke_command_execute("help-lessons".into());
    assert!(ui.get_lessons_open());
    ui.invoke_lessons_start("first-melody".into());
    assert!(
        ui.get_lesson_active(),
        "开课本身不许因落盘失败而失败（进行中课程按内存状态走）"
    );
    let feedback = ui.get_lesson_feedback();
    assert!(
        feedback.contains("学习进度未能保存"),
        "开课写失败必须在课程反馈区提示，实际：{feedback}"
    );
    assert!(
        feedback.contains("回退"),
        "反馈必须说明下次启动可能回退，实际：{feedback}"
    );
    assert!(
        help_state::load_from(&settings_path).lessons.is_empty(),
        "写失败不许假装进度已落盘"
    );

    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_file(&legacy);
}

#[test]
fn lesson_step_save_failure_advances_step_and_warns() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let (settings_path, blocker) = broken_settings_path("step");
    let legacy = temp_legacy_path("step");
    let _guard = StatePathEnv::set(&settings_path, &legacy);

    let project = yeban_model::samples::filled_project();
    let undo = open_undo(&project);
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    ui.invoke_command_execute("help-lessons".into());
    ui.invoke_lessons_start("first-melody".into());
    assert!(ui.get_lesson_active());
    // 真建轨：第一步判定（新轨道）通过。
    ui.invoke_command_execute("track-add-midi".into());
    ui.invoke_lesson_check_step();
    assert_eq!(
        ui.get_lesson_step_index(),
        1,
        "判定过了步必须推进（落盘失败不回滚内存进度）"
    );
    let feedback = ui.get_lesson_feedback();
    assert!(
        feedback.contains("学习进度未能保存"),
        "完成一步写失败必须在课程反馈区提示，实际：{feedback}"
    );
    assert!(
        feedback.contains("回退"),
        "反馈必须说明下次启动可能回退，实际：{feedback}"
    );
    assert!(
        !help_state::load_from(&settings_path)
            .lesson_progress("first-melody")
            .finished,
        "写失败不许假装进度已落盘"
    );

    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_file(&legacy);
}

// ------------------------------------------------------------ 成功路径回归

#[test]
fn save_success_path_stays_quiet_and_persists() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let settings_path = temp_state_path("ok");
    let legacy = temp_legacy_path("ok");
    let _ = std::fs::remove_file(&settings_path);
    let _ = std::fs::remove_file(&legacy);
    let _guard = StatePathEnv::set(&settings_path, &legacy);

    let project = yeban_model::samples::filled_project();
    let undo = open_undo(&project);
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    ui.invoke_onboarding_dismiss_forever();
    assert!(!ui.get_show_floating_toast(), "成功路径不许误弹错误提示");
    assert!(
        help_state::load_from(&settings_path).onboarding_dismissed,
        "成功路径必须真落盘"
    );

    ui.invoke_command_execute("help-lessons".into());
    ui.invoke_lessons_start("first-melody".into());
    assert!(ui.get_lesson_active());
    assert!(
        ui.get_lesson_feedback().is_empty(),
        "成功路径课程反馈必须清空"
    );

    let _ = std::fs::remove_file(&settings_path);
    let _ = std::fs::remove_file(&legacy);
}

// ------------------------------------------------------------ 源码钉子

#[test]
fn help_rs_no_longer_discards_save_results() {
    let help = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/help.rs"))
        .expect("help.rs 必须可读");
    assert!(
        !help.contains("let _ = help_state::save"),
        "三处 let _ = 必须全部接住 Result"
    );
    assert_eq!(
        help.matches("help_state::save(&state)").count(),
        3,
        "三处落盘点必须都在（update_state / 开课 / 完成一步）"
    );
    assert!(
        help.contains("save_failure_message"),
        "失败文案必须走统一口径"
    );
    assert!(
        help.contains("report_notice"),
        "引导落盘失败必须走 report_notice"
    );
}
