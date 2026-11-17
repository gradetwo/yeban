//! 帮助系统判据（line/onboarding-help）：四块各自的真落点。
//!
//! * 手册：章节表 ↔ `docs/manual/` 源文件 ↔ README 登记表逐项对账，
//!   每章解析出标题与足量正文。
//! * 状态存储：往返 / 损坏回退 / 版本回退 / 环境变量路径，全部走显式路径
//!   或持锁的环境变量（⛔ 不碰真实家目录）。
//! * 课程判定：`lessons::evaluate` 只认**开课基准之外**的新轨道与新音符——
//!   基准内（演示工程自带）的内容再多也不许假通过；鼓课音高范围外不计。
//! * 活窗口全链：菜单命令 → 四个面板真打开；引导切步真导航（底部页签 /
//!   检查器）；「不再显示」与「走完」都落盘；样例课从开课到结课全程走
//!   真实写入口（菜单建轨命令 + `UndoPort::commit_ops` 的真实 Op），
//!   判定不过 ⇒ 步不推进，判定过 ⇒ 推进且进度落盘可读回。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use slint::{ComponentHandle, Model};
use yeban_app::bridge::ViewState;
use yeban_app::help_state::{self, HelpState};
use yeban_app::host::{self, ProductionLoop};
use yeban_app::input::InputContext;
use yeban_app::lessons::{self, LessonCheck};
use yeban_app::manual;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

// ------------------------------------------------------------ 环境变量守卫

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// `YEBAN_ONBOARDING_FILE` 的 RAII 守卫（Drop 时恢复原值）。
///
/// SAFETY（Rust 2024 的 `set_var` 是 `unsafe`）：本文件全部 env 判据先拿
/// [`ENV_LOCK`]，持锁期间本测试二进制内没有其它线程读这个变量。
struct StatePathEnv {
    previous: Option<OsString>,
}

impl StatePathEnv {
    /// ⚠ 调用方必须先持有 [`ENV_LOCK`]。
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os(help_state::HELP_STATE_PATH_ENV);
        // SAFETY: 调用方持有 ENV_LOCK；见类型注释。
        unsafe {
            std::env::set_var(help_state::HELP_STATE_PATH_ENV, path);
        }
        Self { previous }
    }
}

impl Drop for StatePathEnv {
    fn drop(&mut self) {
        // SAFETY: 守卫在 ENV_LOCK 释放前析构，恢复进入时读到的原值。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(help_state::HELP_STATE_PATH_ENV, value),
                None => std::env::remove_var(help_state::HELP_STATE_PATH_ENV),
            }
        }
    }
}

fn temp_state_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "yeban-onboarding-{tag}-{}.json",
        std::process::id()
    ))
}

// ------------------------------------------------------------ 手册

#[test]
fn manual_chapters_match_repo_sources_and_readme() {
    let expected: [(&str, &str); 6] = [
        ("01-overview", "界面总览"),
        ("02-projects", "新建与保存工程"),
        ("03-tracks-clips", "轨道与片段"),
        ("04-piano-roll", "钢琴卷帘编辑"),
        ("05-mixer-effects", "混音台与效果"),
        ("06-export", "导出音频"),
    ];
    assert_eq!(manual::CHAPTERS.len(), expected.len(), "首批手册必须六章");
    let readme = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/manual/README.md"
    ))
    .expect("docs/manual/README.md 必须在仓里");
    for (chapter, (id, title)) in manual::CHAPTERS.iter().zip(expected) {
        assert_eq!(chapter.id, id);
        assert_eq!(chapter.title, title);
        assert!(
            readme.contains(&format!("`{id}.md`")),
            "README 登记表必须含 {id}.md"
        );
        let lines = manual::parse_chapter(chapter.markdown);
        assert!(
            lines
                .iter()
                .any(|line| line.level == 1 && line.text == title),
            "{id} 必须有一级标题且与目录标题一致"
        );
        assert!(
            lines.len() >= 12,
            "{id} 正文行数过少（{} 行），不许是空壳章节",
            lines.len()
        );
    }
}

// ------------------------------------------------------------ 状态存储

#[test]
fn help_state_round_trips_and_falls_back_safely() {
    let path = temp_state_path("roundtrip");
    let _ = std::fs::remove_file(&path);

    // 不存在 ⇒ 默认（首次启动语义：该弹引导）。
    let fresh = help_state::load_from(&path);
    assert!(fresh.should_auto_show_onboarding());

    let mut state = HelpState {
        onboarding_dismissed: true,
        ..HelpState::default()
    };
    state.lessons.insert(
        "first-melody".to_owned(),
        help_state::LessonProgress {
            steps_done: 3,
            finished: true,
        },
    );
    help_state::save_to(&path, &state).expect("状态必须能落盘");
    let back = help_state::load_from(&path);
    assert_eq!(back, state);
    assert!(!back.should_auto_show_onboarding());

    // 损坏 ⇒ 默认，不 panic、不把坏文件当状态。
    std::fs::write(&path, "{not json").expect("写坏文件");
    assert_eq!(help_state::load_from(&path), HelpState::default());

    // 未来版本 ⇒ 默认（不覆盖新版文件的内容语义由 load 的回退保证）。
    std::fs::write(&path, "{\"version\": 99}").expect("写未来版本文件");
    assert_eq!(help_state::load_from(&path), HelpState::default());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn help_state_path_env_override_is_honored() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let path = temp_state_path("envpath");
    let _ = std::fs::remove_file(&path);
    let _guard = StatePathEnv::set(&path);
    assert_eq!(
        help_state::state_path().expect("路径必须可解析"),
        path,
        "YEBAN_ONBOARDING_FILE 必须优先于默认路径"
    );
    let state = HelpState {
        onboarding_finished: true,
        ..HelpState::default()
    };
    help_state::save(&state).expect("经 env 路径落盘");
    assert_eq!(help_state::load(), state);
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------------ 课程格式与判定

fn project_with_new_midi_track(notes: &[(u64, u8, u64)]) -> (YebanProjectV1, BTreeSet<EntityId>) {
    let mut project = yeban_model::samples::filled_project();
    let baseline: BTreeSet<EntityId> = project.tracks.keys().copied().collect();
    let track_id = EntityId::new();
    let clip_id = EntityId::new();
    let mut note_map = BTreeMap::new();
    for (start, pitch, duration) in notes {
        let id = EntityId::new();
        note_map.insert(id, MidiNote::new(id, *start, *pitch, *duration));
    }
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "练习片段".to_owned(),
            content: ClipContent::Midi { notes: note_map },
        },
    );
    let mut track = TrackV3 {
        id: track_id,
        name: "练习轨".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    let placement_id = EntityId::new();
    track.clips.insert(
        placement_id,
        ClipPlacement {
            id: placement_id,
            clip_id,
            start_tick: 0,
            duration_ticks: 15_360,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
        },
    );
    project.tracks.insert(track_id, track);
    (project, baseline)
}

#[test]
fn lesson_catalog_is_data_and_round_trips() {
    let catalog = lessons::catalog();
    assert_eq!(catalog.len(), 2, "首批样例课必须两节");
    assert_eq!(catalog[0].id, "first-melody");
    assert_eq!(catalog[0].title, "写第一段旋律");
    assert_eq!(catalog[1].id, "first-drum-track");
    assert_eq!(catalog[1].title, "编第一条鼓轨");
    for lesson in &catalog {
        assert!(lesson.steps.len() >= 3, "{} 至少三步", lesson.id);
        for step in &lesson.steps {
            assert!(!step.title.is_empty() && !step.body.is_empty());
        }
        let json = serde_json::to_string(lesson).expect("课程必须可序列化");
        let back: lessons::Lesson = serde_json::from_str(&json).expect("课程必须可反序列化");
        assert_eq!(&back, lesson, "课程数据格式必须无损往返");
    }
}

#[test]
fn lesson_checks_only_count_work_after_the_baseline() {
    let melody = lessons::lesson("first-melody").expect("旋律课必须在");
    let new_track_check = &melody.steps[0].check;
    let first_notes_check = &melody.steps[1].check;

    // 基准 = 演示工程的全部轨道：工程里音符再多，两步都不许过。
    let project = yeban_model::samples::filled_project();
    let baseline: BTreeSet<EntityId> = project.tracks.keys().copied().collect();
    assert!(!lessons::evaluate(new_track_check, &baseline, &project).done);
    assert!(!lessons::evaluate(first_notes_check, &baseline, &project).done);

    // 新轨 + 3 个音：建轨步过，音符步不过（要 4 个）。
    let (project, baseline) =
        project_with_new_midi_track(&[(0, 60, 480), (960, 62, 480), (1920, 64, 480)]);
    assert!(lessons::evaluate(new_track_check, &baseline, &project).done);
    let verdict = lessons::evaluate(first_notes_check, &baseline, &project);
    assert!(!verdict.done);
    assert!(
        verdict.progress.contains("3/4"),
        "进度必须报实测数字，实际：{}",
        verdict.progress
    );

    // 新轨 + 4 个音 / 3 音高 / 跨度 ≥1 小节：音符步过。
    let (project, baseline) = project_with_new_midi_track(&[
        (0, 60, 480),
        (960, 62, 480),
        (1920, 64, 480),
        (3840, 60, 480),
    ]);
    assert!(lessons::evaluate(first_notes_check, &baseline, &project).done);
}

#[test]
fn drum_lesson_check_ignores_notes_outside_the_gm_range() {
    let drums = lessons::lesson("first-drum-track").expect("鼓课必须在");
    let LessonCheck::NotesOnNewTrack { .. } = drums.steps[1].check else {
        panic!("鼓课第二步必须是音符判定");
    };
    let check = &drums.steps[1].check;
    // 8 个音全在 GM 鼓区外（音高 100+）：一个都不许计。
    let outside: Vec<(u64, u8, u64)> = (0..8u64).map(|i| (i * 480, 100 + i as u8, 240)).collect();
    let (project, baseline) = project_with_new_midi_track(&outside);
    assert!(!lessons::evaluate(check, &baseline, &project).done);
    // 底鼓 36 + 军鼓 38 交替 8 个、跨度过半小节：过。
    let inside: Vec<(u64, u8, u64)> = (0..8u64)
        .map(|i| (i * 480, if i % 2 == 0 { 36 } else { 38 }, 240))
        .collect();
    let (project, baseline) = project_with_new_midi_track(&inside);
    assert!(lessons::evaluate(check, &baseline, &project).done);
}

// ------------------------------------------------------------ 源码面

#[test]
fn help_menu_and_host_arms_exist_in_source() {
    let app = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.slint"))
        .expect("app.slint 必须可读");
    assert!(app.contains("title: \"帮助\""), "菜单栏必须有帮助菜单");
    let host = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/host.rs"))
        .expect("host.rs 必须可读");
    for command in [
        "help-manual",
        "help-shortcuts",
        "help-onboarding",
        "help-lessons",
    ] {
        assert!(
            app.contains(&format!("command-execute(\"{command}\")")),
            "帮助菜单必须有 {command} 入口"
        );
        assert!(
            host.contains(&format!("\"{command}\" =>")),
            "host 命令分发必须有 {command} 的落点"
        );
    }
}

// ------------------------------------------------------------ 活窗口全链

fn build_live_window(
    project: &YebanProjectV1,
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

#[test]
fn help_system_end_to_end_in_a_live_window() {
    let _lock = ENV_LOCK.lock().expect("env 锁");
    let state_path = temp_state_path("live");
    let _ = std::fs::remove_file(&state_path);
    let _guard = StatePathEnv::set(&state_path);

    let project = yeban_model::samples::filled_project();
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<onboarding-help>", "yeban-app", project.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let (live, _production) = build_live_window(&project, &undo);
    let ui = live.ui();

    // ---- 引导：首次启动自动打开，切步真导航 ----
    assert!(
        yeban_app::help::show_onboarding_if_first_launch(ui),
        "全新状态下首次启动必须自动打开引导"
    );
    assert!(ui.get_onboarding_open());
    assert_eq!(ui.get_onboarding_step_index(), 0);
    ui.invoke_onboarding_next();
    ui.invoke_onboarding_next();
    ui.invoke_onboarding_next();
    assert_eq!(ui.get_onboarding_step_index(), 3, "第三步应是钢琴卷帘步");
    assert_eq!(
        ui.global::<yeban_app::ui::Theme<'_>>()
            .get_active_dock_tab()
            .as_str(),
        "pianoroll",
        "钢琴卷帘步必须真的把底部页签切到钢琴卷帘"
    );
    ui.invoke_onboarding_next();
    assert_eq!(
        ui.global::<yeban_app::ui::Theme<'_>>()
            .get_active_dock_tab()
            .as_str(),
        "mixer",
        "混音台步必须真的把底部页签切到混音控制台"
    );
    ui.set_show_left_inspector(false);
    ui.invoke_onboarding_next();
    assert_eq!(ui.get_onboarding_step_index(), 5);
    assert!(
        ui.get_show_left_inspector(),
        "检查器步必须真的把左栏检查器打开"
    );
    // 走完最后一步 ⇒ 落盘 finished，关层，之后不再自动弹。
    ui.invoke_onboarding_next();
    assert_eq!(ui.get_onboarding_step_index(), 6);
    ui.invoke_onboarding_next();
    assert!(!ui.get_onboarding_open());
    assert!(help_state::load_from(&state_path).onboarding_finished);
    assert!(!yeban_app::help::show_onboarding_if_first_launch(ui));

    // ---- 引导：「不再显示」落盘；「跳过」不落盘 ----
    help_state::save_to(&state_path, &HelpState::default()).expect("重置状态");
    assert!(yeban_app::help::show_onboarding_if_first_launch(ui));
    ui.invoke_onboarding_dismiss_forever();
    assert!(!ui.get_onboarding_open());
    assert!(help_state::load_from(&state_path).onboarding_dismissed);
    assert!(!yeban_app::help::show_onboarding_if_first_launch(ui));

    help_state::save_to(&state_path, &HelpState::default()).expect("重置状态");
    ui.invoke_command_execute("help-onboarding".into());
    assert!(ui.get_onboarding_open(), "帮助菜单必须能重开引导");
    ui.invoke_onboarding_skip();
    assert!(!ui.get_onboarding_open());
    let state = help_state::load_from(&state_path);
    assert!(
        !state.onboarding_dismissed && !state.onboarding_finished,
        "跳过只关层，不许落盘成已完成 / 不再显示"
    );

    // ---- 手册：菜单打开、六章目录、切章换内容 ----
    ui.invoke_command_execute("help-manual".into());
    assert!(ui.get_help_manual_open());
    assert_eq!(ui.get_manual_chapters().row_count(), 6);
    assert!(ui.get_manual_lines().row_count() >= 12);
    ui.invoke_help_manual_select(3);
    assert_eq!(ui.get_manual_chapter_index(), 3);
    let first_line = ui
        .get_manual_lines()
        .row_data(0)
        .expect("第四章必须有内容行");
    assert_eq!(first_line.text.as_str(), "钢琴卷帘编辑");
    ui.invoke_help_manual_close();
    assert!(!ui.get_help_manual_open());

    // ---- 速查：行数与策略表单源逐行对账 ----
    ui.invoke_command_execute("help-shortcuts".into());
    assert!(ui.get_help_shortcuts_open());
    assert_eq!(
        ui.get_help_shortcut_rows().row_count(),
        yeban_app::cli::shortcut_rows().len(),
        "速查行数必须等于 cli 策略表的行数（同源，不许手抄）"
    );
    ui.invoke_help_shortcuts_close();
    assert!(!ui.get_help_shortcuts_open());

    // ---- 课程：开课 → 没做不许过 → 真建轨 → 真写音符 → 结课落盘 ----
    ui.invoke_command_execute("help-lessons".into());
    assert!(ui.get_lessons_open());
    assert_eq!(ui.get_lesson_rows().row_count(), 2);
    assert!(!ui.get_lesson_active());
    ui.invoke_lessons_start("first-melody".into());
    assert!(ui.get_lesson_active());
    assert_eq!(ui.get_lesson_step_index(), 0);
    // 什么都没做就点检查：步不许推进，反馈必须写明。
    ui.invoke_lesson_check_step();
    assert_eq!(ui.get_lesson_step_index(), 0, "没建轨不许过第一步");
    assert!(!ui.get_lesson_feedback().is_empty(), "判定不过必须给反馈");
    // 真建轨（菜单命令的真实落点）。
    let tracks_before = undo.with_project(|p| p.tracks.len()).expect("工程必须在");
    ui.invoke_command_execute("track-add-midi".into());
    let (tracks_after, new_track_id) = undo
        .with_project(|p| {
            let baseline: BTreeSet<EntityId> = project.tracks.keys().copied().collect();
            let new_id = p.tracks.keys().find(|id| !baseline.contains(id)).copied();
            (p.tracks.len(), new_id)
        })
        .expect("工程必须在");
    assert_eq!(tracks_after, tracks_before + 1, "菜单建轨必须真增一条轨");
    let new_track_id = new_track_id.expect("必须能定位新轨");
    ui.invoke_lesson_check_step();
    assert_eq!(ui.get_lesson_step_index(), 1, "建轨后第一步必须通过");

    // 真写音符：片段进池 + 摆放 + 音符，全走 UndoPort 的真实 Op。
    let clip_id = EntityId::new();
    undo.commit_ops(
        NOW_MS,
        "判据: 建练习片段",
        vec![
            Op::AddClip {
                clip: ClipPoolEntry {
                    id: clip_id,
                    name: "练习片段".to_owned(),
                    content: ClipContent::Midi {
                        notes: BTreeMap::new(),
                    },
                },
            },
            Op::AddClipPlacement {
                track_id: new_track_id,
                placement: ClipPlacement {
                    id: EntityId::new(),
                    clip_id,
                    start_tick: 0,
                    duration_ticks: 15_360,
                    loop_config: LoopConfig::default(),
                    muted: false,
                    offset_samples: None,
                },
            },
        ],
    )
    .expect("片段与摆放必须能提交");
    let write_notes = |notes: &[(u64, u8)]| {
        let ops: Vec<Op> = notes
            .iter()
            .map(|(start, pitch)| Op::AddNote {
                track_id: new_track_id,
                clip_id,
                note: MidiNote::new(EntityId::new(), *start, *pitch, 480),
            })
            .collect();
        undo.commit_ops(NOW_MS, "判据: 写音符", ops)
            .expect("音符必须能提交");
    };
    // 第二步：4 个音 / 3 音高 / 跨度 ≥1 小节。
    write_notes(&[(0, 60), (960, 62), (1920, 64), (3840, 60)]);
    ui.invoke_lesson_check_step();
    assert_eq!(ui.get_lesson_step_index(), 2, "写够 4 个音后第二步必须通过");
    // 第三步：累计 8 个音 / ≥4 音高 / 跨度 ≥2 小节 ⇒ 结课。
    write_notes(&[(4800, 65), (5760, 67), (6720, 69), (7680, 71)]);
    ui.invoke_lesson_check_step();
    assert!(!ui.get_lesson_active(), "最后一步通过后应回到课程列表");
    let state = help_state::load_from(&state_path);
    let progress = state.lesson_progress("first-melody");
    assert!(progress.finished, "结课状态必须落盘");
    assert_eq!(progress.steps_done, 3);
    assert!(state.active_lesson.is_none(), "结课后不许留进行中课程");
    ui.invoke_lessons_close();
    assert!(!ui.get_lessons_open());

    let _ = std::fs::remove_file(&state_path);
}
