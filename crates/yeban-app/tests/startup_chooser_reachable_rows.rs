//! 判据：**启动选择器演示分类第 65 行起可达**（T-FIX-293）。
//!
//! 旧口径：宿主把行模型截断在 64 行（`CHOOSER_MAX_ROWS`），
//! 第 65 行起永远进不了选择器，只剩一句「还有 N 行未画出」。
//! 新口径：行模型全量写入，`Flickable` 纵向滚动到达任意一行；
//! `hidden_rows` 恒为 0（只剩诊断意义）；搜索框按当前分类的字段过滤
//! （演示：标题 / 曲风），过滤不重排 `entry_index`；换分类不清空搜索串。
//!
//! ## 本文件钉住的口径
//!
//! 1. 70 条演示全进模型，`hidden_rows` 为 0。
//! 2. 切换分类后打开模型里的最后一条：会话换成那份演示工程（标题对上）。
//! 3. 搜索「演示 42」只剩 1 行，打开它仍是全量下标。
//! 4. 换分类不清空搜索串（切到工厂分类再切回，搜索串还在）；
//!    搜索无命中 ⇒ 空模型；清空 ⇒ 回到 70 行。
//!
//! ## 平台约束：活窗口判据只有一条
//!
//! `slint::platform::set_platform` 是线程局部的（与
//! `tests/startup_chooser.rs` 同款约束）：一个文件里只建一次活窗口。
//! 「滚动到底部」无头不驱动指针滚动：模型完整 + 最后一行可打开是
//! 它的代理判据（`Flickable` 的 `content-height` = 行数 × 卡片高，
//! 最后一行在滚动范围内）；本机无显示，未做截图验证。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use slint::Model as _;
use yeban_app::file_picker::FilePicker;
use yeban_app::host;
use yeban_app::startup_chooser::NewProjectDetails;
use yeban_app::templates::TemplateRoots;

/// 撤销会话的"现在"（判据不需要墙钟，与同类判据同款常量）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照
/// `tests/startup_chooser.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-startup-chooser-reachable-{}-{}",
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

/// 70 条文件型演示：`演示 00.yeban`…`演示 69.yeban`（标题 = 文件名去后缀，
/// 打开后靠标题辨认）。
fn write_seventy_demos(demos_dir: &Path) {
    for index in 0..70 {
        let name = format!("演示 {index:02}");
        let path = demos_dir.join(format!("{name}.yeban"));
        let mut archive = yeban_app::host::new_project_archive();
        archive.project.title = name;
        yeban_app::save::save_archive_file(&archive, &path).expect("写演示夹具");
    }
}

/// 70 条演示全进模型；打开最后一条；搜索过滤不重排下标；换分类不清空搜索。
#[test]
fn startup_chooser_reaches_demo_row_beyond_sixty_four_and_search_keeps_index() {
    use yeban_app::bridge::ViewState;
    use yeban_app::host::ProductionLoop;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let scratch = Scratch::new();
    let roots = scratch.roots();
    let demos_dir = scratch.join("demos");
    std::fs::create_dir_all(&demos_dir).expect("建演示目录");
    write_seventy_demos(&demos_dir);

    // 设置文件隔离到临时路径，并把演示目录指给选择器。
    let settings_path = scratch.join("settings.json");
    // SAFETY: 本测试二进制内只有本条判据碰设置文件与演示目录
    // 环境变量，且活窗口判据全文件只有一条。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            settings_path.as_os_str(),
        );
        std::env::set_var(
            yeban_app::startup_chooser::DEMOS_DIR_ENV,
            demos_dir.as_os_str(),
        );
    }

    // ---- 装配（生产同款形态）：会话 = 占位工程 ---------------------------
    let placeholder = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&placeholder).expect("占位工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<chooser>", "yeban-app", placeholder.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    let production = Rc::new(RefCell::new(
        ProductionLoop::start(&placeholder, 0).expect("引擎重建"),
    ));
    let live = LivePort::new(Size::new(1600, 1000), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // after 两个变体（与 `tests/startup_chooser.rs` 同一条语义）：
    // titled 的保存落点跟着文件走；untitled 的落点为空（副本首存问路径）。
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
                        Some(path.to_path_buf())
                    },
                    project: Rc::new(RefCell::new(project.clone())),
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
    let chooser_picker = ScriptedPicker::answering_open(scratch.join("unused.yeban"));
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

    // ---- 1. 演示分类：70 条全进模型，hidden_rows = 0 ---------------------
    ui.invoke_startup_category_selected(2);
    let rows: Vec<(String, i32)> = ui
        .get_startup_rows()
        .iter()
        .map(|row| (row.label.to_string(), row.entry_index))
        .collect();
    assert_eq!(rows.len(), 70, "70 条演示必须全进模型");
    assert_eq!(
        ui.get_startup_hidden_rows(),
        0,
        "全量写入 ⇒ hidden_rows 恒为 0"
    );

    // ---- 2. 打开模型里的最后一条（第 65 行之后） -------------------------
    let last = rows.last().expect("必须有演示行");
    let last_label = last.0.clone();
    ui.invoke_startup_entry_activated(last.1);
    assert!(!ui.get_startup_chooser_open(), "演示打开后选择器必须关");
    assert_eq!(
        undo.project().title,
        last_label,
        "打开最后一条后会话必须是那份演示（标题对上）"
    );

    // ---- 3. 搜索「演示 42」：只剩 1 行，打开它仍是全量下标 ---------------
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_category_selected(2);
    ui.set_startup_search_text("演示 42".into());
    ui.invoke_startup_search_changed("演示 42".into());
    let rows: Vec<(String, i32)> = ui
        .get_startup_rows()
        .iter()
        .map(|row| (row.label.to_string(), row.entry_index))
        .collect();
    assert_eq!(rows.len(), 1, "搜索后只剩命中那一条");
    assert_eq!(rows[0].0, "演示 42");
    ui.invoke_startup_entry_activated(rows[0].1);
    assert!(!ui.get_startup_chooser_open(), "演示打开后选择器必须关");
    assert_eq!(
        undo.project().title,
        "演示 42",
        "按全量下标打开的必须是演示 42"
    );

    // ---- 4. 换分类不清空搜索串；清空 ⇒ 回到 70 行 ------------------------
    ui.set_startup_chooser_open(true);
    ui.invoke_startup_category_selected(2);
    assert_eq!(
        ui.get_startup_rows().iter().count(),
        1,
        "重开演示分类时搜索串还在，仍只剩命中那一条"
    );
    assert_eq!(
        ui.get_startup_search_text().as_str(),
        "演示 42",
        "搜索串必须留在输入框里"
    );
    ui.invoke_startup_category_selected(3);
    assert_eq!(
        ui.get_startup_rows().iter().count(),
        0,
        "工厂模板分类为空（夹具里没有工厂模板）"
    );
    ui.invoke_startup_category_selected(2);
    assert_eq!(
        ui.get_startup_rows().iter().count(),
        1,
        "切回演示分类，搜索串还在，仍只剩命中那一条"
    );
    ui.set_startup_search_text("".into());
    ui.invoke_startup_search_changed("".into());
    assert_eq!(
        ui.get_startup_rows().iter().count(),
        70,
        "清空搜索后回到 70 行"
    );

    // ---- 5. 搜索无命中 ⇒ 空模型 -------------------------------------------
    ui.set_startup_search_text("不存在的名字".into());
    ui.invoke_startup_search_changed("不存在的名字".into());
    assert_eq!(
        ui.get_startup_rows().iter().count(),
        0,
        "无命中时行模型为空"
    );
    assert_eq!(ui.get_startup_hidden_rows(), 0, "空模型也不是截断");
}
