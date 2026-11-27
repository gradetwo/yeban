//! 帮助锚点注册表判据（T-FIX-487）：注册表每个 `present` target 能打开；
//! 每个 `missing` / `external` 锚点不会被当作已覆盖；同名 UI id 不因
//! 字符串同名被误判为帮助落点；应用内章节清单与注册表中可打开章节一致；
//! 四个现有帮助入口行为不变（经 `help-topic:` 入口同样可达）。

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use slint::Model;
use yeban_app::bridge::ViewState;
use yeban_app::help_state;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::input::InputContext;
use yeban_app::manual::{self, HelpTopicStatus};
use yeban_app::scene::DemoScene;
use yeban_app::settings;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 两个存储路径变量的 RAII 守卫（与 `onboarding_help.rs` 同款）：调用方
/// 必须先持有 [`ENV_LOCK`]，持锁期间本测试二进制内没有其它线程读这两个变量。
struct StatePathEnv {
    previous_settings: Option<OsString>,
    previous_onboarding: Option<OsString>,
}

impl StatePathEnv {
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
    std::env::temp_dir().join(format!(
        "yeban-help-anchor-{tag}-{}.json",
        std::process::id()
    ))
}

fn temp_legacy_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yeban-help-anchor-legacy-{tag}-{}.json",
        std::process::id()
    ))
}

fn build_live_window(
    project: &yeban_model::project::YebanProjectV1,
    undo: &Rc<UndoPort>,
) -> (
    yeban_ui_test_port::render::LivePort<yeban_app::ui::MainWindow>,
    ProductionLoop,
) {
    let view = ViewState::from_project(project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let production = ProductionLoop::start(project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));
    let project_rc = Rc::new(RefCell::new(project.clone()));

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

// ------------------------------------------------------------ 注册表本身

#[test]
fn registry_has_no_duplicate_anchors() {
    let mut seen = BTreeSet::new();
    for row in manual::HELP_ANCHORS {
        assert!(
            seen.insert(row.anchor),
            "注册表锚点 {} 重复登记",
            row.anchor
        );
    }
}

#[test]
fn every_present_row_has_exactly_one_target() {
    for row in manual::HELP_ANCHORS {
        if row.is_covered() {
            assert!(
                row.chapter.is_some() ^ row.entry.is_some(),
                "Present 行 {} 必须有且仅有一个落点（章节或入口）",
                row.anchor
            );
        }
    }
}

#[test]
fn present_chapter_anchors_resolve_to_real_chapters() {
    for row in manual::HELP_ANCHORS {
        let Some(chapter_id) = row.chapter else {
            continue;
        };
        assert!(row.is_covered(), "章节锚点 {chapter_id} 必须是 Present");
        let chapter = manual::chapter(chapter_id)
            .unwrap_or_else(|| panic!("章节锚点 {chapter_id} 在 CHAPTERS 里没有对应章"));
        let lines = manual::parse_chapter(chapter.markdown);
        assert!(
            lines
                .iter()
                .any(|line| line.level == 1 && line.text == chapter.title),
            "{chapter_id} 必须有一级标题且与目录标题一致"
        );
        assert!(
            lines.len() >= 12,
            "{chapter_id} 正文行数过少（{} 行），不许是空壳章节",
            lines.len()
        );
    }
}

#[test]
fn present_entry_anchors_are_the_four_existing_entries() {
    let mut entries: Vec<&str> = manual::HELP_ANCHORS
        .iter()
        .filter_map(|row| row.entry)
        .collect();
    entries.sort_unstable();
    assert_eq!(
        entries,
        [
            "help-lessons",
            "help-manual",
            "help-onboarding",
            "help-shortcuts"
        ],
        "入口锚点必须恰好是四个既有帮助入口，不许多、不许少"
    );
}

#[test]
fn registry_openable_chapters_match_chapter_table() {
    let via_registry: Vec<&str> = manual::covered_anchors()
        .filter_map(|row| row.chapter)
        .collect();
    let via_table: Vec<&str> = manual::CHAPTERS.iter().map(|c| c.id).collect();
    assert_eq!(
        via_registry, via_table,
        "注册表中可打开的章节必须与 CHAPTERS 一致（同序、同集）"
    );
}

#[test]
fn missing_and_external_anchors_are_never_covered() {
    // 机制本身：Missing / External 行永远不算覆盖（构造行直接验证谓词）。
    for status in [HelpTopicStatus::Missing, HelpTopicStatus::External] {
        let row = manual::HelpAnchor {
            anchor: "theory-harmony",
            status,
            chapter: None,
            entry: None,
        };
        assert!(!row.is_covered(), "{status:?} 行不许算已覆盖",);
    }
    // 表内现状：非 Present 行（将来登记五册锚点时会有）也不许进已覆盖清单。
    for row in manual::HELP_ANCHORS {
        if !row.is_covered() {
            assert!(
                !manual::covered_anchors().any(|covered| covered.anchor == row.anchor),
                "非 Present 行 {} 混进了已覆盖清单",
                row.anchor
            );
        }
    }
}

#[test]
fn all_349_manual_section_anchors_are_registered_without_duplicates_or_dangling() {
    let expected: BTreeSet<&str> = include_str!("fixtures/manual_anchors_349.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        expected.len(),
        349,
        "权威清单 349 个主题（348 External + onboarding Present）"
    );
    for anchor in &expected {
        assert!(
            manual::resolve_help_anchor(anchor).is_some(),
            "手册锚点 {anchor} 悬空"
        );
    }
    let external_rows: Vec<&str> = manual::HELP_ANCHORS
        .iter()
        .filter(|r| matches!(r.status, HelpTopicStatus::External))
        .map(|r| r.anchor)
        .collect();
    assert_eq!(
        external_rows.len(),
        348,
        "349 条中 onboarding 由 Present 行承载"
    );
    let mut registered: BTreeSet<&str> = external_rows.into_iter().collect();
    registered.insert("getting-started-onboarding");
    assert_eq!(
        registered, expected,
        "External 集合必须与权威清单 349 完全一致"
    );
    for row in manual::HELP_ANCHORS {
        if row.status == HelpTopicStatus::External {
            assert!(
                row.chapter.is_none() && row.entry.is_none(),
                "External 行 {} 不许有落点",
                row.anchor
            );
        }
    }
    let on = manual::resolve_help_anchor("getting-started-onboarding").expect("onboarding");
    assert!(on.is_covered(), "onboarding 必须仍可打开");
    assert_eq!(on.entry, Some("help-onboarding"));
    assert!(
        manual::resolve_help_anchor("help-onboarding").is_none(),
        "旧锚点名不许双名并存"
    );
}

#[test]
fn ui_ids_are_not_help_anchors() {
    // 同名 UI id 与近似串一律不是帮助落点：注册表只做精确匹配。
    for anchor in [
        "plugin-scan-button",
        "plugin-scan-paths",
        "plugin-scan-status",
        "01-overview-extra",
        "help-manual-extra",
        "",
    ] {
        assert!(
            manual::resolve_help_anchor(anchor).is_none(),
            "锚点 {anchor:?} 不在注册表里，不许被当作已覆盖"
        );
        assert!(
            !manual::covered_anchors().any(|row| row.anchor == anchor),
            "锚点 {anchor:?} 不许出现在已覆盖清单里"
        );
    }
    for anchor in ["export-wav-dialog", "plugin-scan"] {
        let row = manual::resolve_help_anchor(anchor).expect("手册 External 行");
        assert_eq!(row.status, HelpTopicStatus::External);
        assert!(!row.is_covered());
    }
}

// ------------------------------------------------------------ 活窗口全链

#[test]
fn help_topic_command_opens_present_targets_and_notices_others() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let state_path = temp_state_path("live");
    let legacy_path = temp_legacy_path("live");
    let _ = std::fs::remove_file(&state_path);
    let _ = std::fs::remove_file(&legacy_path);
    let _guard = StatePathEnv::set(&state_path, &legacy_path);

    let project = yeban_model::samples::filled_project();
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<help-anchor-registry>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    // ---- 章节锚点：真打开对应章 ----
    ui.invoke_command_execute("help-topic:04-piano-roll".into());
    assert!(ui.get_help_manual_open(), "章节锚点必须打开手册");
    assert_eq!(ui.get_manual_chapter_index(), 3);
    let first_line = ui
        .get_manual_lines()
        .row_data(0)
        .expect("第四章必须有内容行");
    assert_eq!(first_line.text.as_str(), "钢琴卷帘编辑");
    ui.invoke_help_manual_close();
    assert!(!ui.get_help_manual_open());

    // ---- 入口锚点：走既有入口 ----
    ui.invoke_command_execute("help-topic:help-shortcuts".into());
    assert!(ui.get_help_shortcuts_open(), "入口锚点必须打开速查");
    ui.invoke_help_shortcuts_close();
    assert!(!ui.get_help_shortcuts_open());
    ui.invoke_command_execute("help-topic:help-lessons".into());
    assert!(ui.get_lessons_open(), "入口锚点必须打开课程");
    ui.invoke_lessons_close();
    assert!(!ui.get_lessons_open());
    ui.invoke_command_execute("help-topic:help-manual".into());
    assert!(ui.get_help_manual_open(), "入口锚点必须打开手册");
    ui.invoke_help_manual_close();

    // ---- 同名 UI id：什么都不打开，只报提示 ----
    ui.invoke_command_execute("help-topic:export-wav-dialog".into());
    assert!(
        !ui.get_help_manual_open() && !ui.get_help_shortcuts_open() && !ui.get_lessons_open(),
        "同名 UI id 不许打开任何帮助面板"
    );
    assert!(ui.get_show_floating_toast(), "表外锚点必须报提示，不许静默");
    assert!(
        ui.get_toast_message().contains("export-wav-dialog"),
        "提示必须写明锚点名，实际：{}",
        ui.get_toast_message()
    );
    assert!(
        !ui.get_toast_is_error(),
        "找不到主题是信息级提示，不是错误级"
    );

    // ---- 表外锚点：同样只报提示 ----
    ui.invoke_command_execute("help-topic:theory-harmony".into());
    assert!(!ui.get_help_manual_open(), "表外锚点不许打开手册");
    assert!(
        ui.get_toast_message().contains("theory-harmony"),
        "提示必须写明锚点名，实际：{}",
        ui.get_toast_message()
    );
}
