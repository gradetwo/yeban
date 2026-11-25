//! T-FIX-291：CI 可跑的**活窗口**判据 —— 用仓库内合成 demo + 合成封面
//! 钉住演示分类的堆叠布局（默认最前 / 环位确定 / 切回不变）与
//! 「打开 = 未命名副本」（会话换工程、不记最近、源文件字节不变）。
//! 不写截图、不碰 `exp-demo-cover-assets`。
//!
//! 平台约束：一个文件只建一次活窗口。

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
use yeban_app::host::ProductionLoop;
use yeban_app::startup_chooser::NewProjectDetails;
use yeban_app::templates::TemplateRoots;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::image::Rect;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-inrepo-live-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // SAFETY: 本测试二进制里只有本条判据碰这两个环境变量。
        unsafe {
            std::env::remove_var(yeban_app::startup_chooser::DEMOS_DIR_ENV);
            std::env::remove_var(yeban_app::settings::SETTINGS_PATH_ENV);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write_synthetic_cover(path: &Path) {
    use yeban_ui_test_port::image::Rgb8Image;
    let mut image = Rgb8Image::new(Size::new(2, 2));
    for y in 0..2 {
        for x in 0..2 {
            image.set_pixel(x, y, [48, 96, 24]);
        }
    }
    std::fs::write(path, yeban_ui_test_port::png::encode_rgb8(&image)).expect("写合成封面");
}

#[test]
fn synthetic_demos_stack_layout_and_unnamed_copy_open() {
    use yeban_app::bridge::ViewState;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::{PointerButton, UiTestPort as _};

    let scratch = Scratch::new();
    let roots = TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    let demos = scratch.dir.join("demos");
    std::fs::create_dir_all(&demos).expect("建演示目录");
    let titles = ["合成甲", "合成乙", "合成丙"];
    // 文件名带数字前缀：生产扫描按路径排序，中文文件名路径序（丙、乙、甲）
    // 与语义序不一致，前缀使路径序与标题序对齐。
    for (index, title) in titles.iter().enumerate() {
        let stem = format!("{:02}-{title}", index + 1);
        let path = demos.join(format!("{stem}.yeban"));
        let mut archive = yeban_app::host::new_project_archive();
        archive.project.title = (*title).to_owned();
        yeban_app::save::save_archive_file(&archive, &path).expect("写合成 demo");
        write_synthetic_cover(&demos.join(format!("{stem}-cover.png")));
    }
    let source = demos.join("03-合成丙.yeban");
    let source_before = std::fs::read(&source).expect("读源文件字节");

    // SAFETY: 本测试二进制里只有本条判据碰这两个环境变量，且活窗口只有一条。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            scratch.dir.join("settings.json").as_os_str(),
        );
        std::env::set_var(yeban_app::startup_chooser::DEMOS_DIR_ENV, demos.as_os_str());
    }

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
    let mut live = LivePort::new(Size::new(1600, 1000), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");

    let save_picker = ScriptedPicker::answering_save(scratch.dir.join("副本.yeban"));
    let untitled_port: Rc<host::PortOpenProject> = {
        let port_after = Rc::clone(&undo);
        let picker_after: Rc<dyn FilePicker> = {
            let owned: Rc<ScriptedPicker> = Rc::clone(&save_picker);
            owned
        };
        Rc::new(host::PortOpenProject {
            port: Rc::clone(&undo),
            production: Some(Rc::clone(&production)),
            quanta: 0,
            after: Rc::new(move |ui, project, path, _view| {
                host::wire_save_with_engine(
                    ui,
                    host::SaveStatus {
                        target: None, // 未命名副本：首存才问路径
                        project: Rc::new(RefCell::new(project.clone())),
                    },
                    Some(Rc::clone(&port_after)),
                    Rc::clone(&picker_after),
                    None,
                    None,
                );
                let _ = path;
                Ok(())
            }),
        })
    };
    let chooser_picker = ScriptedPicker::answering_open(source.clone());
    {
        let ui = live.ui();
        host::wire_startup_chooser(
            ui,
            {
                let owned: Rc<ScriptedPicker> = Rc::clone(&chooser_picker);
                let picker: Rc<dyn FilePicker> = owned;
                picker
            },
            untitled_port.clone(),
            untitled_port,
            roots,
            NewProjectDetails::from_project(&placeholder),
        );
        ui.set_startup_chooser_open(true);
        ui.invoke_startup_category_selected(2);
        let rows: Vec<String> = ui
            .get_startup_rows()
            .iter()
            .map(|r| r.label.to_string())
            .collect();
        assert_eq!(rows, titles, "合成 demo 必须按路径序进入演示分类");
        assert_eq!(ui.get_startup_hidden_rows(), 0, "全量写入，无隐藏行");
    }

    // ---- 默认堆叠：只有最前（最后一条）有信息块 -------------------------
    let bounds = {
        let tree = live.refresh_tree(None).expect("重抓控件树");
        let info = tree
            .find_by_id("startup-stack-info-2")
            .expect("演示分类默认必须是堆叠，最前信息块在场");
        assert!(info.label.contains("合成丙"), "最前必须是最后一条");
        for index in 0..2 {
            assert!(
                tree.find_by_id(&format!("startup-stack-info-{index}"))
                    .is_none(),
                "非最前不得出现信息块"
            );
        }
        let mut found = Vec::new();
        for index in 0..3 {
            let node = tree
                .find_by_id(&format!("startup-row-{index}"))
                .unwrap_or_else(|| panic!("堆叠里第 {index} 张封面必须在控件树里"));
            found.push(node.bounds.expect("封面必须有几何包围盒"));
        }
        assert!(
            found[2].height >= found[0].height && found[2].height >= found[1].height,
            "最前封面必须不小于背面封面"
        );
        found
    };

    // ---- 确定性：切去唱片架再切回，环位逐值不变 -------------------------
    let click = |live: &mut LivePort<yeban_app::ui::MainWindow>, id: &str| {
        live.dispatch_pointer_down(id, 31.0, 10.0, PointerButton::Left)
            .expect("切换钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
    };
    click(&mut live, "startup-demo-view-shelf");
    click(&mut live, "startup-demo-view-stack");
    {
        let tree = live.refresh_tree(None).expect("切回堆叠后重抓控件树");
        for (index, before) in bounds.iter().enumerate() {
            let now = tree
                .find_by_id(&format!("startup-row-{index}"))
                .expect("封面必须在控件树里")
                .bounds
                .expect("封面必须有几何包围盒");
            assert_eq!(now, *before, "第 {index} 张切去切回环位必须逐值不变");
        }
    }

    // ---- 打开 = 未命名副本 ----------------------------------------------
    {
        let ui = live.ui();
        ui.invoke_startup_entry_activated(2);
        assert!(!ui.get_startup_chooser_open(), "打开后选择器必须关");
        assert_eq!(undo.project().title, "合成丙", "会话必须换成该 demo 工程");
    }
    assert_eq!(
        yeban_app::settings::load().recent_projects.len(),
        0,
        "演示打开不得记进最近清单"
    );
    assert_eq!(
        std::fs::read(&source).expect("再读源工程字节"),
        source_before,
        "打开之后源文件必须一字节不变"
    );
    // 写出来给调用方看：本判据不产证据图，1x/3x 截图留在手工/证据档。
    let _: Option<Rect> = None;
}
