//! T-FIX-299：设置写回失败 / 设置文件损坏必须让用户看见。
//!
//! 此前这六个写回入口（语言、最近工程、调试输出、音频设备、跟随模式、
//! 窗口布局）与 `load_from` 的损坏分支都只打一行 `eprintln!`：桌面双击
//! 启动的用户看不到终端，会看到“这次切换明明生效了，下次启动又变回去”。
//!
//! 本文件的判据分三层，各自能独立变红：
//!
//! | # | 判据 | 观测面 |
//! | :-- | :--- | :--- |
//! | ① | 写回失败（注入：设置路径的父级是一个普通文件，`save_to` 的
//! | | `create_dir_all` 以 `NotADirectory` 失败 —— root 也绕不过，
//! | | 不靠权限位）时记警告，文案点名“本次已生效、下次启动会回退”，
//! | | 且调用方不失败（函数正常返回） | `settings::take_pending_warnings()` |
//! | ② | 损坏的设置文件：`load_from` 回默认、记警告、原文件备份
//! | | （`settings.json.corrupt-<毫秒>`，原样）后移走；第二次读
//! | | 不再重复警告；缺文件仍是静默容错 | `load_from` 返回值 + 备份文件 |
//! | ③ | 写回失败时**界面**出现警告：活窗口里调
//! | | `host::apply_language_and_save`，错误级浮动提示可见且文案
//! | | 点名写回失败 | 真 `MainWindow` 的 toast 属性（无像素） |
//! | ④ | 帮助状态折入落盘失败：`help_state::load()` 仍返回折入结果
//! | | （本次生效），同时记一条点名“帮助状态折入”的警告 | 返回值 + 台账 |
//! | ⑤ | 正常路径回归：写回成功时台账为空 | 台账 |
//!
//! 进程级环境说明：`YEBAN_SETTINGS_PATH` / `YEBAN_ONBOARDING_FILE`
//! 是进程级变量，判据之间用 [`ENV_LOCK`] 串行（与
//! `tests/file_picker_dialogs.rs` 同一条纪律）；待发警告台账也是进程
//! 级的，每个判据开头先取空，避免别的判据的残留。写失败注入用“父级
//! 是文件”而不用“只读目录”：本机测试以 root 跑，权限位拦不住 root。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use yeban_app::bridge::ViewState;
use yeban_app::cli::Language;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 进程级环境 + 警告台账的串行锁（见文件头说明）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 两个设置路径环境变量的 RAII 守卫（`Drop` 时恢复原值）。
///
/// SAFETY（Rust 2024 的 `set_var` 是 `unsafe`）：本文件的全部 env 判据都先拿
/// [`ENV_LOCK`]，且调用方持有锁的期间本测试二进制里没有其它线程读这两个变量；
/// `Drop` 在同一把锁内恢复。
struct TestEnv {
    previous_settings: Option<OsString>,
    previous_onboarding: Option<OsString>,
}

impl TestEnv {
    /// ⚠ 调用方**必须**先持有 [`ENV_LOCK`]（本函数不再锁一次：那是同一把非重入锁）。
    fn new() -> Self {
        Self {
            previous_settings: std::env::var_os(yeban_app::settings::SETTINGS_PATH_ENV),
            previous_onboarding: std::env::var_os(yeban_app::help_state::HELP_STATE_PATH_ENV),
        }
    }

    fn set_settings(&self, path: &Path) {
        // SAFETY: 调用方持有 ENV_LOCK；本测试进程内没有其它线程读这个变量。
        unsafe {
            std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, path);
        }
    }

    fn set_onboarding(&self, path: &Path) {
        // SAFETY: 同上。
        unsafe {
            std::env::set_var(yeban_app::help_state::HELP_STATE_PATH_ENV, path);
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // SAFETY: 守卫比调用方的 ENV_LOCK 先析构（同一测试内后声明者先 drop），
        // 且恢复的是进入时读到的原值。
        unsafe {
            match &self.previous_settings {
                Some(value) => std::env::set_var(yeban_app::settings::SETTINGS_PATH_ENV, value),
                None => std::env::remove_var(yeban_app::settings::SETTINGS_PATH_ENV),
            }
            match &self.previous_onboarding {
                Some(value) => {
                    std::env::set_var(yeban_app::help_state::HELP_STATE_PATH_ENV, value);
                }
                None => std::env::remove_var(yeban_app::help_state::HELP_STATE_PATH_ENV),
            }
        }
    }
}

/// 每个判据一个独立临时目录（用例名 + 进程号区分，跑完删掉）。
fn test_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("yeban-settings-299-{name}-{}", std::process::id()))
}

/// 写回失败注入：`dir/blocker` 是一个普通文件，于是
/// `dir/blocker/settings.json` 的 `create_dir_all` 以 `NotADirectory`
/// 失败 —— root 也绕不过。返回被堵住的设置路径。
fn blocked_settings_path(dir: &Path) -> PathBuf {
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("写堵住父级的普通文件");
    blocker.join("settings.json")
}

/// 判据 ①：写回失败记警告，文案点名事项与回退，调用方不失败。
///
/// 注：这个注入下读也失败（父级是文件），于是台账里会有两条警告 ——
/// 一条“设置文件无法读取”（`save_language` 先读出现有设置），一条
/// “语言选择写回失败”。生产上可读文件 + 不可写目录时只有后一条；
/// 这里不断言条数，只断言写回那条在场、文案点名回退。
#[test]
fn settings_save_failure_records_warning_and_keeps_caller_contract() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("save-failure");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let _env = TestEnv::new();
    _env.set_settings(&blocked_settings_path(&dir));

    // 调用方不失败：函数正常返回（void），语言已在本会话生效是设计。
    yeban_app::settings::save_language(Language::EnUs);

    let warnings = yeban_app::settings::take_pending_warnings();
    assert!(
        warnings.iter().any(
            |warning| warning.message_in(Language::ZhCn).contains("语言选择")
                && warning
                    .message_in(Language::ZhCn)
                    .contains("已在本会话生效")
                && warning
                    .message_in(Language::ZhCn)
                    .contains("下次启动会回到旧值")
        ),
        "写回失败必须记一条点名语言选择与回退的警告，实际台账：{warnings:?}"
    );
    assert!(
        yeban_app::settings::take_pending_warnings().is_empty(),
        "取走即清空，不重复展示"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ②：损坏的设置文件回默认、记警告、原文件备份后移走；第二次读不再警告。
#[test]
fn settings_corrupt_file_warns_and_preserves_original() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("corrupt");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("settings.json");
    std::fs::write(&path, "{ not json").expect("写损坏文件");

    let loaded = yeban_app::settings::load_from(&path);
    assert_eq!(
        loaded,
        yeban_app::settings::Settings::default(),
        "损坏文件回默认"
    );
    let warnings = yeban_app::settings::take_pending_warnings();
    assert_eq!(warnings.len(), 1, "坏文件必须恰好记一条警告");
    assert!(
        warnings[0].message_in(Language::ZhCn).contains("已损坏")
            && warnings[0].message_in(Language::ZhCn).contains("备份"),
        "警告须点名损坏与备份：{}",
        warnings[0].message_in(Language::ZhCn)
    );
    assert!(!path.exists(), "损坏文件必须被移走（备份后丢弃）");
    let backups: Vec<_> = std::fs::read_dir(&dir)
        .expect("读临时目录")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("settings.json.corrupt-"))
        })
        .collect();
    assert_eq!(backups.len(), 1, "恰好留一份备份");
    assert_eq!(
        std::fs::read_to_string(&backups[0]).expect("备份必须能读"),
        "{ not json",
        "备份须是原文件原样"
    );

    // 备份后文件已不在 ⇒ 第二次读是“缺文件”，不再警告。
    assert_eq!(
        yeban_app::settings::load_from(&path),
        yeban_app::settings::Settings::default()
    );
    assert!(
        yeban_app::settings::take_pending_warnings().is_empty(),
        "备份后不再重复警告"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-FIX-432：损坏文件经 `load_from` 记的结构化警告，在英文态下渲染为英文。
#[test]
fn settings_corrupt_file_warning_renders_in_english() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("corrupt-en");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("settings.json");
    std::fs::write(&path, "{ not json").expect("写损坏文件");

    let _ = yeban_app::settings::load_from(&path);
    let warnings = yeban_app::settings::take_pending_warnings();
    assert_eq!(warnings.len(), 1, "坏文件必须恰好记一条警告");
    let rendered = warnings[0].message_in(Language::EnUs);
    assert!(
        rendered.starts_with("Settings file is corrupt (")
            && rendered.contains("defaults are in use")
            && rendered.contains("backed up to"),
        "英文态损坏警告须为英文、点名损坏与备份，实际：{rendered}"
    );
    assert!(
        !rendered.contains("已损坏") && !rendered.contains("备份"),
        "英文态损坏警告不得残留中文，实际：{rendered}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ② 的另一半：缺文件是首次启动，静默容错，不警告。
#[test]
fn settings_missing_file_is_silent_first_run() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("missing");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("settings.json");

    assert_eq!(
        yeban_app::settings::load_from(&path),
        yeban_app::settings::Settings::default()
    );
    assert!(
        yeban_app::settings::take_pending_warnings().is_empty(),
        "缺文件是首次启动，不警告"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ④：帮助状态折入落盘失败，本次仍返回折入结果，同时记警告。
#[test]
fn help_state_fold_failure_records_warning_but_keeps_folded_state() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("help-fold");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let _env = TestEnv::new();
    _env.set_settings(&blocked_settings_path(&dir));
    let legacy = dir.join("onboarding.json");
    std::fs::write(&legacy, r#"{"version":1,"onboarding_dismissed":true}"#)
        .expect("写旧帮助状态文件");
    _env.set_onboarding(&legacy);

    let state = yeban_app::help_state::load();
    assert!(
        state.onboarding_dismissed,
        "折入失败不挡读取：本次先按折入结果走"
    );
    let warnings = yeban_app::settings::take_pending_warnings();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message_in(Language::ZhCn).contains("帮助状态折入")),
        "折入落盘失败必须记一条点名帮助状态折入的警告，实际台账：{warnings:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ⑤：正常路径回归 —— 写回成功时台账为空，读回的值就是写的值。
#[test]
fn settings_normal_roundtrip_records_no_warning() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("roundtrip");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let _env = TestEnv::new();
    let path = dir.join("settings.json");
    _env.set_settings(&path);

    yeban_app::settings::save_language(Language::FrFr);
    yeban_app::settings::save_debug_output(true);
    assert!(
        yeban_app::settings::take_pending_warnings().is_empty(),
        "正常写回不许记警告"
    );
    let loaded = yeban_app::settings::load();
    assert_eq!(loaded.language, Some(Language::FrFr));
    assert!(loaded.debug_output);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ③：写回失败时**界面**出现警告 —— 活窗口里调语言切换入口，
/// 错误级浮动提示可见，文案点名写回失败与下次启动回退。
///
/// `slint::platform::set_platform` 线程局部、每线程只能装配一次 ⇒
/// 本文件只有一个建窗口的判据（其余判据都不建窗口）。
#[test]
fn write_failure_toast_is_visible_in_live_window() {
    let _guard = ENV_LOCK.lock().expect("判据间串行锁");
    let _ = yeban_app::settings::take_pending_warnings();
    let dir = test_dir("toast");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let _env = TestEnv::new();
    // 建窗阶段先指到可写的临时文件：建窗末尾的 drain 不应看到任何警告。
    let good_path = dir.join("settings.json");
    _env.set_settings(&good_path);

    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();
    assert!(
        !ui.get_show_floating_toast(),
        "建窗后没有任何待发警告，提示不应出现"
    );

    // 切换到被堵住的设置路径再切语言：写回失败。
    _env.set_settings(&blocked_settings_path(&dir));
    host::apply_language_and_save(ui, Language::EnUs);

    assert!(ui.get_show_floating_toast(), "写回失败时浮动提示必须出现");
    assert!(
        ui.get_toast_is_error(),
        "设置写回失败是错误级常驻提示，不是信息级"
    );
    let message = ui.get_toast_message();
    // T-FIX-432：已切到英文态，提示须为英文（事项与回退都点名），不得残留中文。
    assert!(
        message.contains("Language choice")
            && message.contains("old value will return on next launch"),
        "英文态提示文案须为英文、点名事项与回退，实际：{message}"
    );
    assert!(
        !message.contains("语言选择") && !message.contains("写回设置失败"),
        "英文态提示不得残留中文模板，实际：{message}"
    );
    // 语言本身已在本会话生效（调用方不失败的设计）。
    assert_eq!(ui.get_language_code().as_str(), "en-US");
    let _ = std::fs::remove_dir_all(&dir);
}
