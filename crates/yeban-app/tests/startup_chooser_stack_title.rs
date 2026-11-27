//! T-FIX-689：demo 选择页堆叠模式最前卡片的标题必须完整可见。
//!
//! 前提（基线 a573730e 实测）：列表区高 468px，堆叠 Flickable 视口
//! 高 396px；信息块 y = stage-h + 8 = 376，高 88；标题框在信息块
//! 内 y 2..28，标题底 = 376 + 28 = 404，超出视口底 396 共 8px，
//! 标题底部被 Flickable 视口裁掉（曲风/BPM/详情更在折叠线以下）。
//! 本判据把 8 首歌名逐一转到最前，断言每首标题框完全落在视口内，
//! 并对最长歌名《下一个路口》抓 1x 整帧 + 3x 信息块裁图作视觉证据。
//!
//! 平台约束：一个文件只建一次活窗口。不写 `exp-demo-cover-assets`，
//! 证据图写临时目录（路径打印到 stderr），由人工拷入回执目录。

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

/// 8 首官方 demo 歌名（`demo_download::DEMOS` 同款顺序）；最长 5 字。
const TITLES: [&str; 8] = [
    "雨声太吵",
    "潮",
    "失重",
    "失语",
    "呢喃",
    "下一个路口",
    "回头路",
    "那个午后",
];

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-689-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // SAFETY: 本测试二进制里只有本条判据碰这两个环境变量，且活窗口只有一条。
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

fn write_png(path: &Path, image: &yeban_ui_test_port::image::Rgb8Image) {
    let bytes = yeban_ui_test_port::png::encode_rgb8(image);
    std::fs::write(path, bytes)
        .unwrap_or_else(|error| panic!("写截图 {} 失败: {error}", path.display()));
}

/// 从整帧裁出 `rect`（与图像求交夹取）并最近邻放大 3 倍。
fn crop_scaled_3x(
    frame: &yeban_ui_test_port::image::Rgb8Image,
    rect: yeban_ui_test_port::image::Rect,
) -> yeban_ui_test_port::image::Rgb8Image {
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::image::Rgb8Image;
    let rect = rect
        .intersect(frame.size())
        .expect("裁图区域必须与整帧相交");
    let mut out = Rgb8Image::new(Size::new(rect.width * 3, rect.height * 3));
    for oy in 0..rect.height * 3 {
        for ox in 0..rect.width * 3 {
            let rgb = frame
                .pixel(rect.x as u32 + ox / 3, rect.y as u32 + oy / 3)
                .expect("源像素必须在帧内");
            out.set_pixel(ox, oy, rgb);
        }
    }
    out
}

/// 等指定行的信息块出现在控件树里（即该行已转到最前并落定）。
fn wait_stack_top(live: &mut LivePort<yeban_app::ui::MainWindow>, row: usize) {
    let info_id = format!("startup-stack-info-{row}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        slint::platform::update_timers_and_animations();
        let found = {
            let tree = live.refresh_tree(None).expect("等步进时重抓控件树");
            tree.find_by_id(&info_id).is_some()
        };
        if found {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "堆叠步进 15 秒内未把第 {row} 行转到最前（循环移位未走完）"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let settle_until = std::time::Instant::now() + std::time::Duration::from_millis(450);
    while std::time::Instant::now() < settle_until {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn stack_front_title_fully_visible_for_all_eight_demos() {
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
    // 文件名带数字前缀：生产扫描按路径排序，前缀使路径序与标题序对齐。
    for (index, title) in TITLES.iter().enumerate() {
        let stem = format!("{:02}-{title}", index + 1);
        let path = demos.join(format!("{stem}.yeban"));
        let mut archive = yeban_app::host::new_project_archive();
        archive.project.title = (*title).to_owned();
        yeban_app::save::save_archive_file(&archive, &path).expect("写合成 demo");
        write_synthetic_cover(&demos.join(format!("{stem}-cover.png")));
    }

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
            after: Rc::new(move |ui, path, _view| {
                let Some(project) = port_after.try_project() else {
                    return Err("打开后端口里没有工程（当前工程一位没动）".to_owned());
                };
                host::wire_save_with_engine(
                    ui,
                    host::SaveStatus {
                        target: None,
                        project: Rc::new(RefCell::new(project)),
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
    let chooser_picker = ScriptedPicker::answering_open(demos.join("01-雨声太吵.yeban"));
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
        assert_eq!(rows, TITLES, "8 首 demo 必须按路径序进入演示分类");
    }

    // 证据图输出目录（临时目录，路径打印出来供人工拷入回执目录）。
    let evidence = std::env::temp_dir().join("yeban-689-evidence");
    let _ = std::fs::remove_dir_all(&evidence);
    std::fs::create_dir_all(&evidence).expect("建证据目录");
    eprintln!("689 evidence dir: {}", evidence.display());

    // 标题框在信息块内的相对位置（.slint: 标题 UiText x 12 / y 2 /
    // 宽 248 / 高 26；信息块本身不 clip，裁切只发生在 Flickable 视口）。
    const TITLE_DY: i32 = 2;
    const TITLE_H: i32 = 26;

    // 逐首转到最前并断言标题框完全落在堆叠 Flickable 视口内。
    // 默认最前 = 最后一行（行 7《那个午后》）；每步点当前 slot-1 卡
    // （左环位露边）的左沿，顶层沿环次序前进一格。
    let mut top: usize = 7;
    for step in 0..8usize {
        let expected = TITLES[top];
        wait_stack_top(&mut live, top);
        let (info_bounds, list_bounds) = {
            let tree = live.refresh_tree(None).expect("重抓控件树");
            let info = tree
                .find_by_id(&format!("startup-stack-info-{top}"))
                .unwrap_or_else(|| panic!("第 {top} 行（《{expected}》）转到最前时必须有信息块"));
            assert!(
                info.label.contains(expected),
                "最前信息块必须是《{expected}》，实际 {}",
                info.label
            );
            for index in 0..8 {
                if index != top {
                    assert!(
                        tree.find_by_id(&format!("startup-stack-info-{index}"))
                            .is_none(),
                        "非最前专辑不得出现信息块（行 {index}）"
                    );
                }
            }
            let list = tree
                .find_by_id("startup-chooser-list")
                .expect("列表区必须在控件树里");
            (
                info.bounds.expect("信息块必须有几何包围盒"),
                list.bounds.expect("列表区必须有几何包围盒"),
            )
        };
        // 堆叠 Flickable 在列表区内 y 72、视口高 = 列表高 - 72。
        let viewport_top = list_bounds.y + 72;
        let viewport_bottom = list_bounds.y + list_bounds.height as i32;
        let title_top = info_bounds.y + TITLE_DY;
        let title_bottom = info_bounds.y + TITLE_DY + TITLE_H;
        eprintln!(
            "689 step{step} top={top} 《{expected}》 info y={} h={} list y={} h={} viewport {}..{} title {}..{}",
            info_bounds.y,
            info_bounds.height,
            list_bounds.y,
            list_bounds.height,
            viewport_top,
            viewport_bottom,
            title_top,
            title_bottom,
        );
        assert!(
            title_top >= viewport_top && title_bottom <= viewport_bottom,
            "《{expected}》标题框必须完全落在堆叠视口内：标题 {}..{}，视口 {}..{}（信息块 y={} 高={}，列表 y={} 高={}）",
            title_top,
            title_bottom,
            viewport_top,
            viewport_bottom,
            info_bounds.y,
            info_bounds.height,
            list_bounds.y,
            list_bounds.height,
        );

        // 最长歌名《下一个路口》（行 5）抓 1x 整帧 + 3x 信息块裁图。
        if top == 5 {
            live.window().request_redraw();
            let frame = live.window().capture().expect("最长歌名态抓帧");
            write_png(&evidence.join("stack-title-longest-1x.png"), &frame);
            let crop = crop_scaled_3x(
                &frame,
                Rect {
                    x: info_bounds.x,
                    y: info_bounds.y,
                    width: info_bounds.width,
                    height: info_bounds.height,
                },
            );
            write_png(&evidence.join("stack-title-longest-3x.png"), &crop);
            eprintln!(
                "689 longest-title evidence: {}",
                evidence.join("stack-title-longest-3x.png").display()
            );
        }

        if step == 7 {
            break;
        }
        // 下一步：点当前 slot-1 卡（行 (top+1)%8）的左沿露边。
        let next = (top + 1) % 8;
        let card = {
            let tree = live.refresh_tree(None).expect("点卡前重抓控件树");
            tree.find_by_id(&format!("startup-row-{next}"))
                .unwrap_or_else(|| panic!("第 {next} 张封面必须在控件树里"))
                .bounds
                .expect("封面必须有几何包围盒")
        };
        live.dispatch_pointer_down(
            &format!("startup-row-{next}"),
            24.0,
            card.height as f64 / 2.0,
            PointerButton::Left,
        )
        .expect("背面封面左沿露边必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        top = next;
    }
}
