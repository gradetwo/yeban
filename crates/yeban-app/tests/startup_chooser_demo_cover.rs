//! 实验判据：**文件型 demo + 封面图**（exp/demo-cover，Wang 点题，
//! 只做实验、不合入）。
//!
//! 用真实素材（《雨声太吵》工程 + Wang 上传的专辑图）验证：
//!
//! 1. `YEBAN_DEMOS_DIR` 指向素材目录时，演示工程分类多出一条文件型
//!    demo，标题与轨道数从容器真实读出（「雨声太吵」/ 非主总线 7 轨）；
//! 2. 该行 `has-cover` 为真（宿主用 `Image::load_from_path` 真实加载
//!    了同名封面），两条内置 demo 无封面；
//! 3. 整页渲染落盘 `chooser-demos-1x.png`，第三行区域裁出放大 3 倍
//!    落盘 `chooser-demos-row-3x.png`（都在素材目录里，**不进仓库**）；
//! 4. 文件型 demo 打开 = 未命名副本：会话换成该工程、不记最近、
//!    源文件字节不变。
//!
//! 素材目录：优先读环境变量 `YEBAN_DEMOS_DIR`，缺省回落到仓库同级
//! 的 `exp-demo-cover-assets/`。素材不在 ⇒ 本判据**响亮失败**
//! （实验夹具缺失不许静默跳过）。
//!
//! 平台约束与 `tests/startup_chooser.rs` 同款：一个文件只建一次
//! 活窗口。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

use slint::Model as _;

/// 实验素材目录（见文件头）。
fn assets_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(yeban_app::startup_chooser::DEMOS_DIR_ENV)
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../exp-demo-cover-assets")
}

/// 临时目录（`Drop` 时清理）。形态照 `tests/startup_chooser.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-demo-cover-{}-{}",
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

/// 把整帧写成 PNG（用测试港的编码器，与黄金图同一条写出路径）。
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
    let rect = rect.intersect(frame.size()).expect("行区域必须与整帧相交");
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

#[test]
fn demo_cover_live_window_screenshot() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use common::ScriptedPicker;
    use yeban_app::bridge::ViewState;
    use yeban_app::file_picker::FilePicker;
    use yeban_app::host;
    use yeban_app::host::ProductionLoop;
    use yeban_app::scene::DemoScene;
    use yeban_app::startup_chooser::NewProjectDetails;
    use yeban_app::templates::TemplateRoots;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let assets = assets_dir();
    let demo_file = assets.join("yusheng.yeban");
    let cover_file = assets.join("yusheng-cover.png");
    assert!(
        demo_file.is_file(),
        "实验素材缺失: {} 不在（先把 demo 工程放进素材目录）",
        demo_file.display()
    );
    assert!(
        cover_file.is_file(),
        "实验素材缺失: {} 不在（先把专辑图放进素材目录）",
        cover_file.display()
    );
    let demo_bytes_before = std::fs::read(&demo_file).expect("读源工程字节");

    let scratch = Scratch::new();
    let roots = TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    // 设置与演示目录都隔离/指到本实验的夹具上。SAFETY: 本测试二进
    // 制里只有这一条判据碰这两个环境变量。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            scratch.dir.join("settings.json").as_os_str(),
        );
        std::env::set_var(
            yeban_app::startup_chooser::DEMOS_DIR_ENV,
            assets.as_os_str(),
        );
    }

    // ---- 装配（与 startup_chooser_live_window_flow 同款形态） --------
    let placeholder = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&placeholder).expect("占位工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<chooser>",
            "yeban-app",
            placeholder.clone(),
            1_700_000_000_000,
        )
        .expect("撤销会话必须能打开"),
    ));
    let production = Rc::new(RefCell::new(
        ProductionLoop::start(&placeholder, 0).expect("引擎重建"),
    ));
    let mut live = LivePort::new(Size::new(1600, 1000), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");

    let copy_path = scratch.dir.join("副本.yeban");
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
    let chooser_picker = ScriptedPicker::answering_open(demo_file.clone());

    // ---- 接线 + 切到演示分类 + 行模型断言（ui 借用限在本块内） ------
    {
        let ui = live.ui();
        host::wire_startup_chooser(
            ui,
            {
                let owned: Rc<ScriptedPicker> = Rc::clone(&chooser_picker);
                let picker: Rc<dyn FilePicker> = owned;
                picker
            },
            titled,
            untitled_port,
            roots,
            NewProjectDetails::from_project(&placeholder),
        );
        ui.set_startup_chooser_open(true);
        ui.invoke_startup_category_selected(2);
        let rows: Vec<_> = ui.get_startup_rows().iter().collect();
        assert_eq!(rows.len(), 3, "演示分类 = 内置两条 + 文件型一条");
        assert!(!rows[0].has_cover, "内置 demo 不得有封面");
        assert!(!rows[1].has_cover, "内置 demo 不得有封面");
        assert_eq!(
            rows[2].label.as_str(),
            "雨声太吵",
            "文件型 demo 标题必须从容器真实读出"
        );
        assert!(
            rows[2].detail.contains("7 条轨道"),
            "文件型 demo 副文字必须是非主总线轨道数，实际 {}",
            rows[2].detail
        );
        assert!(rows[2].has_cover, "文件型 demo 必须带上真实加载的封面");
    }

    // ---- 整帧截图 ------------------------------------------------------
    live.window().request_redraw();
    let frame = live.window().capture().expect("抓帧");
    let full_png = assets.join("chooser-demos-1x.png");
    write_png(&full_png, &frame);

    // ---- 第三行裁图（控件树几何定位，放大 3 倍） ----------------------
    let bounds = {
        let tree = live.refresh_tree(None).expect("重抓控件树");
        let row_node = tree
            .find_by_id("startup-row-2")
            .expect("控件树里必须有第三行 startup-row-2");
        row_node.bounds.expect("第三行必须有几何包围盒")
    };
    let crop = crop_scaled_3x(&frame, bounds);
    let crop_png = assets.join("chooser-demos-row-3x.png");
    write_png(&crop_png, &crop);
    eprintln!(
        "demo-cover: 整帧 {} ({}x{})，第三行包围盒 x={} y={} {}x{}，裁图 {} ({}x{})",
        full_png.display(),
        frame.size().width,
        frame.size().height,
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        crop_png.display(),
        crop.size().width,
        crop.size().height,
    );

    // ---- 打开语义：未命名副本、不记最近、源文件字节不变 --------------
    {
        let ui = live.ui();
        ui.invoke_startup_entry_activated(2);
        assert!(
            !ui.get_startup_chooser_open(),
            "文件型 demo 打开后选择器必须关；模块回执: {}；当前会话标题: {}",
            ui.get_module_status(),
            undo.project().title
        );
    }
    assert_eq!(
        undo.project().title,
        "雨声太吵",
        "会话必须换成文件型 demo 那份工程"
    );
    assert_eq!(
        yeban_app::settings::load().recent_projects.len(),
        0,
        "演示打开不得记进最近清单"
    );
    assert_eq!(
        std::fs::read(&demo_file).expect("再读源工程字节"),
        demo_bytes_before,
        "打开之后源文件必须一字节不变"
    );
}
