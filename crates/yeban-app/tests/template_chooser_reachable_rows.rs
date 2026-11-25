//! 判据：**模板选择器第 65 行起可达**（T-FIX-293）。
//!
//! 旧口径：宿主把行模型截断在 64 行（`TEMPLATE_CHOOSER_MAX_ROWS`），
//! 第 65 行起永远进不了选择器，只剩一句「还有 N 行未画出」。
//! 新口径：行模型全量写入，`Flickable` 纵向滚动到达任意一行；
//! `hidden_rows` 恒为 0（只剩诊断意义）；搜索框按来源 / 曲风 /
//! 子曲风 / 模板名过滤，过滤不重排 `entry_index`。
//!
//! ## 本文件钉住的口径
//!
//! 1. 70 条模板（3 曲风 × 2 子曲风，带分组标题）全进模型：
//!    条目行 70、标题行 10（1 来源 + 3 曲风 + 6 子曲风），`hidden_rows` 为 0。
//! 2. 选中并打开模型里的最后一条：会话换成那份模板工程（标题对上）。
//! 3. 搜索「模板 42」只剩 4 行，打开它仍是全量下标（不是重排后的 0）。
//! 4. 搜索无命中 ⇒ 空模型；清空搜索 ⇒ 回到 80 行。
//!
//! ## 平台约束：活窗口判据只有一条
//!
//! `slint::platform::set_platform` 是线程局部的（与
//! `tests/project_templates.rs` 同款约束）：一个文件里只建一次活窗口。
//! 「滚动到底部」无头不驱动指针滚动：模型完整 + 最后一行可打开是
//! 它的代理判据（`Flickable` 的 `content-height` = 行数 × 28px，
//! 最后一行在滚动范围内）；本机无显示，未做截图验证。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::ScriptedPicker;
use slint::Model as _;
use yeban_app::file_picker::FilePicker;
use yeban_app::host;
use yeban_app::templates::{TemplateRoots, list_templates};

/// 撤销会话的"现在"（判据不需要墙钟，与同类判据同款常量）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照
/// `tests/project_templates.rs`。
struct Scratch {
    dir: PathBuf,
    user_dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let id = yeban_model::ids::EntityId::new().to_canonical_string();
        let dir = std::env::temp_dir().join(format!(
            "yeban-template-chooser-reachable-{}-{id}",
            std::process::id()
        ));
        // 用户模板根必须位于用户主目录下（生产同款约束）；测试目录在
        // Drop 里整棵删除，不污染仓库或用户的其它模板。
        let user_dir = yeban_model::preset::user_home_dir()
            .expect("测试机必须能解析用户主目录")
            .join(".yeban-template-chooser-tests")
            .join(format!("{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        std::fs::create_dir_all(&user_dir).expect("建用户模板测试目录");
        Self { dir, user_dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn roots(&self) -> TemplateRoots {
        TemplateRoots {
            factory: self.join("factory"),
            user: self.user_dir.clone(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.user_dir);
    }
}

/// 70 条模板：3 曲风 × 2 子曲风，名字 `模板 00`…`模板 69`（标题 = 名字，
/// 打开后靠标题辨认）。工厂根留空 ⇒ 只有「我的模板」一个来源标题。
fn write_seventy_templates(roots: &TemplateRoots) {
    const STYLES: [&str; 3] = ["流行", "古典", "电子"];
    const SUBSTYLES: [&str; 2] = ["摇滚", "抒情"];
    for index in 0..70 {
        let style = STYLES[index % STYLES.len()];
        let substyle = SUBSTYLES[index % SUBSTYLES.len()];
        let name = format!("模板 {index:02}");
        let path = roots
            .user
            .join(style)
            .join(substyle)
            .join(format!("{name}.yeban"));
        std::fs::create_dir_all(path.parent().expect("模板父目录")).expect("建模板目录");
        let mut archive = yeban_app::host::new_project_archive();
        archive.project.title = name;
        yeban_app::save::save_archive_file(&archive, &path).expect("写模板夹具");
    }
}

/// 70 条模板全进模型；打开最后一条；搜索过滤不重排下标。
#[test]
fn template_chooser_reaches_row_beyond_sixty_four_and_search_keeps_index() {
    use yeban_app::bridge::ViewState;
    use yeban_app::host::ProductionLoop;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let scratch = Scratch::new();
    let roots = scratch.roots();
    std::fs::create_dir_all(&roots.factory).expect("建工厂根");
    write_seventy_templates(&roots);
    // 全量清单的下标是宿主打开的依据：先算好「模板 42」的那一份。
    let listing = list_templates(&roots);
    assert_eq!(listing.len(), 70, "夹具：70 条模板必须全被列表读回");
    let index_of_42 = listing
        .iter()
        .position(|entry| entry.name == "模板 42")
        .expect("夹具必须有模板 42");

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
    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // 未命名变体的 after（与 `tests/project_templates.rs` 同一条语义：
    // 保存落点 = None；保存要问的路径由脚本选择器给）。
    let copy_path = scratch.join("副本.yeban");
    let picker = ScriptedPicker::answering_save(copy_path.clone());
    let after: host::OpenProjectAfter = {
        let port_after = Rc::clone(&undo);
        let picker_after: Rc<dyn FilePicker> = {
            let owned: Rc<ScriptedPicker> = Rc::clone(&picker);
            owned
        };
        Rc::new(move |ui, project, _path, _view| {
            host::wire_save_with_engine(
                ui,
                host::SaveStatus {
                    target: None,
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
    let runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: Some(production),
        quanta: 0,
        after,
    });
    host::wire_project_templates(ui, Some(Rc::clone(&undo)), runtime, roots.clone());

    // ---- 1. 70 条全进模型：条目 70 + 标题 10，hidden_rows = 0 ------------
    ui.invoke_new_from_template();
    assert!(ui.get_template_chooser_open(), "菜单回调必须打开模板选择器");
    let rows: Vec<(i32, String, i32)> = ui
        .get_template_chooser_rows()
        .iter()
        .map(|row| (row.kind, row.label.to_string(), row.entry_index))
        .collect();
    assert_eq!(
        rows.iter().filter(|row| row.0 == 3).count(),
        70,
        "70 条模板必须全进模型"
    );
    assert_eq!(rows.len(), 80, "70 条目行 + 1 来源 + 3 曲风 + 6 子曲风");
    assert_eq!(
        ui.get_template_chooser_hidden_rows(),
        0,
        "全量写入 ⇒ hidden_rows 恒为 0"
    );

    // ---- 2. 打开模型里的最后一条（第 65 行之后） -------------------------
    let last = rows.iter().rfind(|row| row.0 == 3).expect("必须有条目行");
    let last_label = last.1.clone();
    ui.invoke_template_selected(last.2);
    assert!(!ui.get_template_chooser_open(), "选中后选择器必须关");
    assert_eq!(
        undo.project().title,
        last_label,
        "打开最后一条后会话必须是那份模板（标题对上）"
    );

    // ---- 3. 搜索「模板 42」：只剩 4 行，下标仍是全量下标 ------------------
    ui.invoke_new_from_template();
    ui.set_template_chooser_search_text("模板 42".into());
    ui.invoke_template_search_changed("模板 42".into());
    let rows: Vec<(i32, String, i32)> = ui
        .get_template_chooser_rows()
        .iter()
        .map(|row| (row.kind, row.label.to_string(), row.entry_index))
        .collect();
    assert_eq!(
        rows,
        vec![
            (0, "我的模板".to_owned(), -1),
            (1, "流行".to_owned(), -1),
            (2, "摇滚".to_owned(), -1),
            (3, "模板 42".to_owned(), index_of_42 as i32),
        ],
        "搜索后只剩命中组；条目下标必须是全量清单下标，⛔ 不重排"
    );
    assert_eq!(ui.get_template_chooser_hidden_rows(), 0, "过滤不是截断");
    ui.invoke_template_selected(index_of_42 as i32);
    assert!(!ui.get_template_chooser_open(), "选中后选择器必须关");
    assert_eq!(
        undo.project().title,
        "模板 42",
        "按全量下标打开的必须是模板 42"
    );

    // ---- 4. 搜索无命中 ⇒ 空模型；清空 ⇒ 回到 80 行 -----------------------
    ui.invoke_new_from_template();
    ui.set_template_chooser_search_text("不存在的名字".into());
    ui.invoke_template_search_changed("不存在的名字".into());
    assert_eq!(
        ui.get_template_chooser_rows().iter().count(),
        0,
        "无命中时行模型为空"
    );
    assert_eq!(ui.get_template_chooser_hidden_rows(), 0, "空模型也不是截断");
    ui.set_template_chooser_search_text("".into());
    ui.invoke_template_search_changed("".into());
    assert_eq!(
        ui.get_template_chooser_rows().iter().count(),
        80,
        "清空搜索后回到 80 行"
    );
    assert_eq!(ui.get_template_chooser_hidden_rows(), 0);
}
