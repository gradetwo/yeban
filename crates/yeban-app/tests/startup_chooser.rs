//! 判据：**启动工程选择器**（line/startup-chooser，学 Logic Pro
//! 「Choose a Project」的内容与思想，Wang 2026-11-17 令）。
//!
//! ## 本文件钉住的口径
//!
//! 1. **路由**：无 `--open` 的窗口启动 ⇒ 先出选择器；带 `--open`
//!    ⇒ 直进工程；批处理开关 ⇒ 永远到不了选择器。
//! 2. **最近**：按记录顺序列（最近在前）；文件已不存在的条目
//!    置灰不可点，点它只报错、当前工程不动、选择器不关。
//! 3. **演示与模板**：一律未命名副本 —— 从选择器打开模板后保存
//!    副本，模板文件**字节不变**；模板打开**不**进最近清单。
//! 4. **新建**：Details 的字段真的落进新工程（速度 / 调号 / 拍号 /
//!    采样率从界面读数构造，不是摆设）。
//! 5. **打开已有**：经选择器打开磁盘工程后，它排进最近清单第一位。
//!
//! ## 平台约束：活窗口判据只有一条
//!
//! `slint::platform::set_platform` 是线程局部的（与
//! `tests/project_templates.rs` 同款约束）：一个文件里只建一次活窗口。
//! 纯函数判据（路由 / 行模型 / 汇集）不建窗口。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

use yeban_app::cli::{Language, StartupRoute};
use yeban_app::settings::{RecentProject, Settings};
use yeban_app::startup_chooser::{
    ChooserRowKind, NewProjectDetails, demo_rows, gather, recent_rows, template_rows,
};
use yeban_app::templates::{TemplateRoots, TemplateSource};

use slint::Model as _;

const NOW_MS: u64 = 1_700_000_000_000;

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照
/// `tests/project_templates.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-startup-chooser-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn roots(&self) -> TemplateRoots {
        TemplateRoots {
            factory: self.join("factory"),
            user: self.join("user"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write_project_file(path: &Path) -> Vec<u8> {
    std::fs::create_dir_all(path.parent().expect("父目录")).expect("建目录");
    let archive = yeban_app::host::new_project_archive();
    yeban_app::save::save_archive_file(&archive, path).expect("写工程夹具");
    std::fs::read(path).expect("读回工程夹具")
}

// ---------------------------------------------------------------------------
// 纯函数判据
// ---------------------------------------------------------------------------

/// 路由：无参 ⇒ 选择器；`--open` ⇒ 直进；无头 ⇒ 直进（到不了 GUI）。
#[test]
fn startup_route_follows_open_argument() {
    let args = |items: &[&str]| -> Vec<String> { items.iter().map(ToString::to_string).collect() };
    let bare = yeban_app::cli::parse(&args(&[])).expect("空参数必须可解析");
    assert_eq!(
        bare.startup_route(),
        StartupRoute::Chooser,
        "无工程参数启动必须先出选择器"
    );
    let opened = yeban_app::cli::parse(&args(&["--open", "song.yeban"])).expect("带 open 解析");
    assert_eq!(
        opened.startup_route(),
        StartupRoute::Direct,
        "带工程路径启动必须直进工程、不出选择器"
    );
    let headless = yeban_app::cli::parse(&args(&["--headless"])).expect("无头解析");
    assert_eq!(
        headless.startup_route(),
        StartupRoute::Direct,
        "批处理 / 无头路径永远到不了选择器"
    );
}

/// 最近行模型：顺序 = 记录顺序；失效条目置灰且副文字写明原因；
/// 相对时间跟打开时刻走。
#[test]
fn recent_rows_keep_order_and_disable_missing_files() {
    let scratch = Scratch::new();
    let alive = scratch.join("活着.yeban");
    write_project_file(&alive);
    let ghost = scratch.join("已删除.yeban");
    let settings = Settings {
        recent_projects: vec![
            RecentProject {
                path: alive.clone(),
                opened_at_unix_ms: NOW_MS - 5 * 60_000,
            },
            RecentProject {
                path: ghost.clone(),
                opened_at_unix_ms: NOW_MS - 60_000,
            },
        ],
        ..Settings::default()
    };
    let data = gather(&settings, &scratch.roots());
    assert_eq!(data.recent.len(), 2);
    assert!(data.recent[0].exists, "在盘上的条目必须判为存在");
    assert!(!data.recent[1].exists, "不在盘上的条目必须判为不存在");

    let rows = recent_rows(&data.recent, Language::ZhCn, NOW_MS);
    assert_eq!(rows.len(), 2, "最近清单一条一行");
    assert_eq!(rows[0].label, "活着");
    assert!(rows[0].enabled, "存在的条目可点");
    assert!(
        rows[0].detail.contains("5 分钟前"),
        "副文字必须带相对时间，实际 {}",
        rows[0].detail
    );
    assert_eq!(rows[1].label, "已删除");
    assert!(!rows[1].enabled, "失效条目必须置灰不可点");
    assert!(
        rows[1].detail.contains("文件已不存在"),
        "失效条目的副文字必须写明原因，实际 {}",
        rows[1].detail
    );
}

/// 汇集：演示工程恒为内置两条（轨道数与模型样本现数一致）；模板
/// 按来源分组、条目下标指全量清单。
#[test]
fn gather_collects_demos_and_templates() {
    let scratch = Scratch::new();
    let roots = scratch.roots();
    let factory_template = roots
        .factory
        .join("流行")
        .join("抒情")
        .join("工厂模板.yeban");
    write_project_file(&factory_template);
    let user_template = roots
        .user
        .join("摇滚")
        .join("硬摇滚")
        .join("我的模板.yeban");
    write_project_file(&user_template);

    let data = gather(&Settings::default(), &roots);
    assert_eq!(data.demos.len(), 2, "演示工程 = 内置样本 default + filled");
    let filled = yeban_model::samples::filled_project();
    let filled_tracks = filled
        .tracks
        .keys()
        .filter(|id| **id != filled.master_bus_track_id)
        .count();
    let demo_rows = demo_rows(&data.demos, Language::ZhCn);
    assert!(
        demo_rows
            .iter()
            .any(|row| row.details.contains(&format!("{filled_tracks} 条轨道"))),
        "演示行模型「详情」里的轨道数必须与 filled 样本现数一致，实际 {demo_rows:?}"
    );
    assert!(
        demo_rows.iter().all(|row| row.detail.is_empty()),
        "演示行主面不得带轨道数等技术信息（detail 恒空），实际 {demo_rows:?}"
    );

    let factory_rows = template_rows(&data.templates, TemplateSource::Factory);
    let labels: Vec<&str> = factory_rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, ["流行", "抒情", "工厂模板"], "工厂分类按三级分组");
    assert_eq!(factory_rows[2].kind, ChooserRowKind::Entry);
    let user_rows = template_rows(&data.templates, TemplateSource::User);
    assert_eq!(user_rows.len(), 3);
    assert_eq!(
        user_rows[2].entry_index,
        Some(1),
        "条目下标必须是全量模板清单的下标（工厂在前）"
    );
}

/// Details 初值 = 规范空工程的出厂字段（不改 Details 建出的工程
/// 与既有空工程除创建时间外逐字段一致）。
#[test]
fn new_project_details_default_to_the_canonical_empty_project() {
    let canonical = yeban_model::samples::default_project();
    let details = NewProjectDetails::from_project(&canonical);
    let built = details.build_project(NOW_MS);
    assert_eq!(built.bpm, canonical.bpm);
    assert_eq!(built.time_signature, canonical.time_signature);
    assert_eq!(built.key_signature, canonical.key_signature);
    assert_eq!(
        built.audio_config.sample_rate, canonical.audio_config.sample_rate,
        "采样率初值必须来自规范空工程"
    );
    assert_eq!(built.tracks.len(), canonical.tracks.len());
}

// ---------------------------------------------------------------------------
// 活窗口判据（本文件唯一一条）
// ---------------------------------------------------------------------------

/// 生产同款装配里走完选择器的五条腿：失效最近条目拒开、打开已有
/// 工程进最近第一位、模板未命名副本不回写且不记最近、新建工程
/// Details 落盘、演示工程打开。
#[test]
fn startup_chooser_live_window_flow() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use common::ScriptedPicker;
    use yeban_app::bridge::ViewState;
    use yeban_app::file_picker::FilePicker;
    use yeban_app::host;
    use yeban_app::host::ProductionLoop;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let scratch = Scratch::new();
    let roots = scratch.roots();

    // ---- 夹具：磁盘工程 B、工厂模板 T、失效的最近条目 -------------------
    let project_b_path = scratch.join("工程B.yeban");
    {
        let mut archive = yeban_app::host::new_project_archive();
        archive.project.title = "工程B".to_owned();
        yeban_app::save::save_archive_file(&archive, &project_b_path).expect("写工程 B");
    }
    let template_path = roots.factory.join("流行").join("抒情").join("模板甲.yeban");
    {
        std::fs::create_dir_all(template_path.parent().expect("模板父目录")).expect("建模板目录");
        let mut archive = yeban_app::host::new_project_archive();
        archive.project.title = "模板甲".to_owned();
        archive.project.bpm = 88.0;
        yeban_app::save::save_archive_file(&archive, &template_path).expect("写模板甲");
    }
    let template_bytes = std::fs::read(&template_path).expect("读模板字节");
    let ghost_path = scratch.join("不存在.yeban");

    // 设置文件隔离到临时路径，并预置一条失效的最近记录。
    let settings_path = scratch.join("settings.json");
    // SAFETY: 本测试二进制内只有本条判据碰设置文件（其余判据都走
    // 显式 Settings 值），且活窗口判据全文件只有一条。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            settings_path.as_os_str(),
        );
    }
    yeban_app::settings::record_recent_project(&ghost_path, NOW_MS - 60_000);
    assert_eq!(
        yeban_app::settings::load().recent_projects.len(),
        1,
        "预置的失效最近记录必须落盘"
    );

    // ---- 装配（生产同款形态）：会话 = 占位工程（默认样本） --------------
    let placeholder = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&placeholder).expect("占位工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<chooser>", "yeban-app", placeholder.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let placeholder_fingerprint = undo.fingerprint().expect("占位指纹");
    let production = Rc::new(RefCell::new(
        ProductionLoop::start(&placeholder, 0).expect("引擎重建"),
    ));
    let live = LivePort::new(Size::new(1600, 1000), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // after 两个变体（与 main.rs 的 make_after 同一条语义）：titled 的
    // 保存落点跟着文件走；untitled 的落点为空（副本首存问路径）。
    let copy_path = scratch.join("副本.yeban");
    let save_picker = ScriptedPicker::answering_save(copy_path.clone());
    let make_after = |untitled: bool| -> host::OpenProjectAfter {
        let port_after = Rc::clone(&undo);
        let picker_after: Rc<dyn FilePicker> = {
            let owned: Rc<ScriptedPicker> = Rc::clone(&save_picker);
            owned
        };
        Rc::new(move |ui, project, path, _view| {
            host::wire_save_with_engine(
                ui,
                host::SaveStatus {
                    target: if untitled {
                        None
                    } else {
                        Some(PathBuf::from(path))
                    },
                    project: project.clone(),
                },
                Some(Rc::clone(&port_after)),
                Rc::clone(&picker_after),
                None,
                None,
            );
            Ok(())
        })
    };
    let titled_port: Rc<host::PortOpenProject> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after: make_after(false),
    });
    let untitled_port: Rc<host::PortOpenProject> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: Some(Rc::clone(&production)),
        quanta: 0,
        after: make_after(true),
    });
    let titled: Rc<dyn host::OpenProjectRuntime> = titled_port;
    let chooser_picker = ScriptedPicker::answering_open(project_b_path.clone());
    host::wire_startup_chooser(
        ui,
        {
            let owned: Rc<ScriptedPicker> = Rc::clone(&chooser_picker);
            let picker: Rc<dyn FilePicker> = owned;
            picker
        },
        titled,
        untitled_port,
        roots.clone(),
        NewProjectDetails::from_project(&placeholder),
    );
    ui.set_startup_chooser_open(true);

    // ---- 1. 最近：失效条目置灰，点开只报错、工程不动、弹窗不关 ---------
    ui.invoke_startup_category_selected(1);
    let rows = ui.get_startup_rows();
    assert_eq!(rows.iter().count(), 1, "最近分类应有一条失效记录");
    let ghost_row = rows.iter().next().expect("第一行");
    assert!(!ghost_row.enabled, "失效条目必须置灰");
    assert!(
        ghost_row.detail.contains("文件已不存在"),
        "失效条目副文字必须写明原因，实际 {}",
        ghost_row.detail
    );
    ui.invoke_startup_entry_activated(0);
    assert!(ui.get_startup_chooser_open(), "点失效条目后选择器不得关");
    assert_eq!(
        undo.fingerprint().as_deref(),
        Some(placeholder_fingerprint.as_str()),
        "点失效条目后当前工程一位不动"
    );
    assert!(
        ui.get_module_status().contains("recent-missing"),
        "失效条目回执必须报 recent-missing，实际 {}",
        ui.get_module_status()
    );

    // ---- 2. 打开已有工程…：B 打开且排进最近第一位 ------------------------
    ui.invoke_startup_open_existing();
    assert!(!ui.get_startup_chooser_open(), "打开成功后选择器必须关");
    assert_eq!(undo.project().title, "工程B", "会话必须换成工程 B");
    let recent = yeban_app::settings::load().recent_projects;
    assert_eq!(recent.len(), 2, "打开 B 后最近清单 = B + 失效条目");
    assert_eq!(
        recent[0].path, project_b_path,
        "刚打开的工程必须排最近第一位"
    );
    assert_eq!(recent[1].path, ghost_path, "旧条目顺延一位");

    // ---- 3. 工程模板：未命名副本打开，模板字节不变，不记最近 -------------
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_category_selected(3);
    let rows = ui.get_startup_rows();
    let shape: Vec<(i32, String)> = rows
        .iter()
        .map(|row| (row.kind, row.label.to_string()))
        .collect();
    assert_eq!(
        shape,
        vec![
            (0, "流行".to_owned()),
            (1, "抒情".to_owned()),
            (2, "模板甲".to_owned()),
        ],
        "工程模板分类必须按 曲风 → 子曲风 → 模板名 分组"
    );
    let entry_index = rows
        .iter()
        .find(|row| row.kind == 2)
        .map(|row| row.entry_index)
        .expect("必须有模板条目行");
    ui.invoke_startup_entry_activated(entry_index);
    assert!(!ui.get_startup_chooser_open(), "模板打开后选择器必须关");
    assert_eq!(undo.project().title, "模板甲", "会话必须换成模板副本");
    assert_eq!(
        yeban_app::settings::load().recent_projects.len(),
        2,
        "从模板打开不得记进最近清单"
    );
    // 副本无路径关联 ⇒ 首存问路径、写新文件；模板字节一字不变。
    ui.invoke_save_project();
    assert_eq!(
        save_picker.call_count("save"),
        1,
        "模板副本首存必须问一次路径"
    );
    assert!(copy_path.exists(), "副本必须写到问来的新路径");
    assert_eq!(
        std::fs::read(&template_path).expect("再读模板字节"),
        template_bytes,
        "打开并保存副本之后，模板文件必须一字节不变"
    );

    // ---- 4. 新建工程：Details 读数真的落进工程 ---------------------------
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_category_selected(0);
    ui.set_startup_tempo_text("96".into());
    for _ in 0..8 {
        ui.invoke_startup_cycle_key(1);
    }
    assert_eq!(
        ui.get_startup_key_label().as_str(),
        "C 大调",
        "调号步进 8 档必须到 C 大调，实际 {}",
        ui.get_startup_key_label()
    );
    ui.invoke_startup_new_project();
    assert!(!ui.get_startup_chooser_open(), "新建成功后选择器必须关");
    let created = undo.project();
    assert_eq!(created.bpm, 96.0, "Details 的速度必须落进新工程");
    assert_eq!(
        created.key_signature,
        Some(yeban_model::project::KeySignature {
            sharps_flats: 0,
            is_minor: false,
        }),
        "Details 的调号必须落进新工程"
    );
    assert_eq!(
        created.tracks.len(),
        1,
        "新工程必须是空工程形状（只有主总线）"
    );

    // ---- 5. 演示工程：filled 样本以未命名副本打开 -------------------------
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_category_selected(2);
    let rows = ui.get_startup_rows();
    assert_eq!(rows.iter().count(), 2, "演示分类恒有两条");
    ui.invoke_startup_entry_activated(1);
    assert!(!ui.get_startup_chooser_open(), "演示打开后选择器必须关");
    let demo_project = undo.project();
    let filled = yeban_model::samples::filled_project();
    assert_eq!(
        demo_project.tracks.len(),
        filled.tracks.len(),
        "演示打开的必须是 filled 样本那份工程（轨道数对上）"
    );
    assert_eq!(
        yeban_app::settings::load().recent_projects.len(),
        2,
        "演示打开不得记进最近清单"
    );
}
