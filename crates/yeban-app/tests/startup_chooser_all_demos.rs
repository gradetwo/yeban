//! 实验判据：**全部新到 demo（8 条）进选择页**（exp/demo-cover 续轮，
//! Wang 点题，只做实验、不合入）。
//!
//! 素材是 2026-11-23 新到的三包 demo 工程（在仓库同级
//! `exp-demo-cover-assets/all-demos/`，不进仓库）：
//!
//! - 《雨声太吵》（最终版工程 + 专辑封面；曲风旁挂
//!   `雨声太吵.meta.json` 的 `genre` = Grunge-Pop，转录自其容器
//!   `author` 字段，与第一轮同一口径）；
//! - 《雨窗四版》四工程：潮 / 失重 / 失语 / 呢喃（同编曲两种人声
//!   处理；封面两张专辑图：潮·失 → 潮、失重；失语呢喃 → 失语、
//!   呢喃，同专辑共用封面，如实呈现）；
//! - 《民谣四版》三工程：下一个路口 / 回头路 / 那个午后（专辑
//!   《假如时光能够倒流》一张封面三条共用）。《能否》按 Wang
//!   裁决**不放条目**，工程留在 `all-demos-held/`，本判据断言它
//!   不出现。
//!
//! 条目标题一律容器 `title` 实读（八个工程的 title 本就是干净
//! 歌名，无版本/声别后缀）；曲风等无出处的字段留空、不编造。
//!
//! 终轮口径（Wang 2026-11-23 15:11/15:12 裁决）：演示清单只剩
//! 这 8 条文件型 demo（两个系统内置 demo 已从选择页拿掉）；
//! 打开演示分类的第一帧就是**堆叠默认视图**；切换控件上不显示
//! 「堆叠」二字（图标-only，靠证据图目视核）。
//!
//! 产出三张证据图（素材目录根，不进仓库）：
//! `chooser-final-stack-1x.png`（打开演示分类的第一帧，堆叠
//! 默认视图整帧）、`chooser-final-stack-3x.png`（全部封面 ∪
//! 信息块裁图）、`chooser-final-shelf-1x.png`（点唱片架段切到
//! 唱片架后的一帧）。
//!
//! 平台约束与 `tests/startup_chooser_demo_cover.rs` 同款：一个
//! 文件只建一次活窗口。素材不在 ⇒ 本判据**响亮失败**。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

use slint::Model as _;

/// 证据图输出目录（素材根；工作树在 `~/workspace/` 下，素材目录
/// 与工作树同级，故从 crate 目录上溯三级）。
fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../exp-demo-cover-assets")
}

/// 本轮的 demo 目录（8 条文件型 demo）。
fn all_demos_dir() -> PathBuf {
    assets_dir().join("all-demos")
}

/// 临时目录（`Drop` 时清理）。形态照 `tests/startup_chooser.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-all-demos-{}-{}",
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

#[ignore = "requires external exp-demo-cover-assets fixture (not in the Git tree)"]
#[test]
fn all_demos_live_window_screenshots() {
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
    use yeban_ui_test_port::image::Rect;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;
    use yeban_ui_test_port::{PointerButton, UiTestPort as _};

    let assets = assets_dir();
    let demos = all_demos_dir();
    // 素材齐全性（响亮失败，不静默跳过）：8 工程 + 8 封面 + 1 旁挂。
    for name in [
        "雨声太吵.yeban",
        "潮_切片版.yeban",
        "失重_切片版.yeban",
        "失语_叙事版.yeban",
        "呢喃_叙事版.yeban",
        "下一个路口_女声.yeban",
        "回头路_男声.yeban",
        "那个午后_男声.yeban",
        "雨声太吵-cover.png",
        "潮_切片版-cover.png",
        "失重_切片版-cover.png",
        "失语_叙事版-cover.jpg",
        "呢喃_叙事版-cover.jpg",
        "下一个路口_女声-cover.png",
        "回头路_男声-cover.png",
        "那个午后_男声-cover.png",
        "雨声太吵.meta.json",
    ] {
        assert!(
            demos.join(name).is_file(),
            "实验素材缺失: {} 不在",
            demos.join(name).display()
        );
    }
    assert!(
        !demos.join("能否_女声.yeban").exists(),
        "《能否》按裁决不放条目，不得出现在本轮 demo 目录里"
    );

    let scratch = Scratch::new();
    let roots = TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    // SAFETY: 本测试二进制里只有这一条判据碰这两个环境变量。
    unsafe {
        std::env::set_var(
            yeban_app::settings::SETTINGS_PATH_ENV,
            scratch.dir.join("settings.json").as_os_str(),
        );
        std::env::set_var(yeban_app::startup_chooser::DEMOS_DIR_ENV, demos.as_os_str());
    }

    // ---- 装配（与 startup_chooser_demo_cover 同款形态） -------------
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
                        Some(path.to_path_buf())
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
    let chooser_picker = ScriptedPicker::answering_open(demos.join("雨声太吵.yeban"));

    // 文件型条目的期望（扫描按路径排序后的顺序）：标题全部是容器
    // 实读的干净歌名；封面映射逐条钉死（潮·失 → 潮/失重、失语
    // 呢喃 → 失语/呢喃、假如时光能够倒流 → 民谣三条、雨声太吵 →
    // 自己）——封面文件与工程同目录同 stem 的 `-cover` 约定，对
    // 位与否由 has-cover + 截图目视双查，映射本身在素材落位时
    // 已按此表拷入。
    let expected_file_titles = [
        "下一个路口",
        "呢喃",
        "回头路",
        "失语",
        "失重",
        "潮",
        "那个午后",
        "雨声太吵",
    ];

    // ---- 接线 + 切到演示分类 + 行模型断言 ----------------------------
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
        assert_eq!(rows.len(), 8, "演示分类 = 文件型八条（内置样本已移出清单）");
        for (offset, expected) in expected_file_titles.iter().enumerate() {
            let row = &rows[offset];
            assert_eq!(
                row.label.as_str(),
                *expected,
                "第 {} 条文件型 demo 标题必须是容器实读的干净歌名",
                offset + 1
            );
            assert!(
                row.has_cover,
                "《{expected}》必须带上真实加载的封面（同专辑共用封面如实呈现）"
            );
            assert!(
                !row.label.contains("能否"),
                "《能否》按裁决不放条目，不得出现"
            );
        }
        // 曲风：只有《雨声太吵》有旁挂出处，其余留空、不编造。
        assert_eq!(rows[7].genre.as_str(), "Grunge-Pop", "雨声太吵曲风来自旁挂");
        for row in &rows[0..7] {
            assert!(
                row.genre.is_empty(),
                "《{}》无曲风出处，卡面曲风必须留空，实际 {}",
                row.label,
                row.genre
            );
        }
        // 全表打印（回执对账用：BPM / 时长 / 小节 / 轨道逐条）。
        for (index, row) in rows.iter().enumerate() {
            let pairs: Vec<String> = row
                .detail_rows
                .iter()
                .map(|r| format!("{}={}", r.label, r.value))
                .collect();
            eprintln!(
                "all-demos row{index}: label={} genre={} bpm={} cover={} details[{}]",
                row.label,
                row.genre,
                row.bpm_label,
                row.has_cover,
                pairs.join(" ")
            );
        }
    }

    // ---- 默认视图 = 堆叠：第一帧就是堆叠（不点任何切换钮） ---------
    // 堆叠为默认视图的判据形态：不碰切换控件，切进演示分类后
    // 最前信息块（最后一行《雨声太吵》）必须已在控件树里——
    // 架子形态没有信息块，它在场即证明当前是堆叠视图。
    let center = |b: Rect| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
    fn wait_stack_top(live: &mut LivePort<yeban_app::ui::MainWindow>, info_id: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            slint::platform::update_timers_and_animations();
            let found = {
                let tree = live.refresh_tree(None).expect("等步进时重抓控件树");
                tree.find_by_id(info_id).is_some()
            };
            if found {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "堆叠步进 15 秒内未把 {info_id} 转到最前（循环移位未走完）"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let settle_until = std::time::Instant::now() + std::time::Duration::from_millis(450);
        while std::time::Instant::now() < settle_until {
            slint::platform::update_timers_and_animations();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // 默认最前 = 最后一行（《雨声太吵》，行 7）。等它落定。
    wait_stack_top(&mut live, "startup-stack-info-7");
    let (cover_bounds, info_bounds) = {
        let tree = live.refresh_tree(None).expect("堆叠态重抓控件树");
        let info = tree
            .find_by_id("startup-stack-info-7")
            .expect("默认最前（《雨声太吵》）必须有信息块 startup-stack-info-7");
        assert!(
            info.label.contains("雨声太吵"),
            "最前信息块必须是《雨声太吵》，实际 {}",
            info.label
        );
        for index in 0..7 {
            assert!(
                tree.find_by_id(&format!("startup-stack-info-{index}"))
                    .is_none(),
                "非最前专辑不得出现信息块（行 {index}）"
            );
        }
        let mut bounds = Vec::new();
        for index in 0..8 {
            let node = tree
                .find_by_id(&format!("startup-row-{index}"))
                .unwrap_or_else(|| panic!("堆叠里第 {index} 张封面必须在控件树里"));
            bounds.push(node.bounds.expect("封面必须有几何包围盒"));
        }
        (bounds, info.bounds.expect("信息块必须有几何包围盒"))
    };
    for (index, b) in cover_bounds.iter().enumerate() {
        eprintln!(
            "all-demos stack row{index}: x={} y={} {}x{}",
            b.x, b.y, b.width, b.height
        );
    }
    let front = cover_bounds[7];
    assert!(
        info_bounds.y > front.y + front.height as i32 - 4,
        "信息块必须在堆叠区下方、不压最前封面，实际 y={}，最前封面底 {}",
        info_bounds.y,
        front.y + front.height as i32
    );
    // 每张封面至少部分入镜（环位钳制在本 Flickable 内，整帧外
    // 的整张丢失不许发生；窗口由本判据构造为 1600×1000）。
    for (index, b) in cover_bounds.iter().enumerate() {
        assert!(
            b.x + b.width as i32 > 0 && b.x < 1600 && b.y + b.height as i32 > 0 && b.y < 1000,
            "第 {index} 张封面必须至少部分入镜，实际 x={} y={} {}x{}",
            b.x,
            b.y,
            b.width,
            b.height
        );
    }

    // ---- 第一帧截图：整帧 1x + 全部封面 ∪ 信息块裁 3x ---------------
    // 此帧未点过任何切换钮，就是打开演示分类的第一帧（堆叠
    // 默认视图）；切换控件上无「堆叠」二字靠本帧目视核。
    live.window().request_redraw();
    let stack_frame = live.window().capture().expect("堆叠态抓帧");
    write_png(&assets.join("chooser-final-stack-1x.png"), &stack_frame);
    let union_x = cover_bounds
        .iter()
        .map(|b| b.x)
        .chain([info_bounds.x])
        .min()
        .unwrap_or(0);
    let union_y = cover_bounds
        .iter()
        .map(|b| b.y)
        .chain([info_bounds.y])
        .min()
        .unwrap_or(0);
    let union_r = cover_bounds
        .iter()
        .map(|b| b.x + b.width as i32)
        .chain([info_bounds.x + info_bounds.width as i32])
        .max()
        .unwrap_or(0);
    let union_b = cover_bounds
        .iter()
        .map(|b| b.y + b.height as i32)
        .chain([info_bounds.y + info_bounds.height as i32])
        .max()
        .unwrap_or(0);
    let crop = crop_scaled_3x(
        &stack_frame,
        Rect {
            x: union_x,
            y: union_y,
            width: (union_r - union_x) as u32,
            height: (union_b - union_y) as u32,
        },
    );
    write_png(&assets.join("chooser-final-stack-3x.png"), &crop);

    // ---- 循环移位抽查：点第 0 行（槽位 1，左环位露边）转到最前 ------
    {
        let (front_cx, _) = center(front);
        let b0 = cover_bounds[0];
        let (c0x, _) = center(b0);
        let dx = if c0x < front_cx {
            24.0
        } else {
            b0.width as f64 - 24.0
        };
        live.dispatch_pointer_down(
            "startup-row-0",
            dx,
            b0.height as f64 / 2.0,
            PointerButton::Left,
        )
        .expect("背面封面露边必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        wait_stack_top(&mut live, "startup-stack-info-0");
        assert_eq!(
            live.ui().get_startup_selected_entry(),
            0,
            "循环移位后选中态必须同步到第 0 行"
        );
        let tree = live.refresh_tree(None).expect("移位后重抓控件树");
        let nb0 = tree
            .find_by_id("startup-row-0")
            .expect("封面必须在控件树里")
            .bounds
            .expect("封面必须有几何包围盒");
        let (ncx, ncy) = center(nb0);
        let (fcx, fcy) = center(front);
        assert!(
            (ncx - fcx).abs() <= 6 && (ncy - fcy).abs() <= 6,
            "移位后第 0 行必须落在原最前环位 ({fcx},{fcy})，实际 ({ncx},{ncy})"
        );
    }

    // ---- 切到唱片架：整帧一帧（8 条都在清单里） ----------------------
    live.dispatch_pointer_down("startup-demo-view-shelf", 31.0, 10.0, PointerButton::Left)
        .expect("唱片架切换钮必须可按下");
    live.dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");
    {
        let tree = live.refresh_tree(None).expect("切到唱片架后重抓控件树");
        assert!(
            tree.find_by_id("startup-stack-info-0").is_none(),
            "架子模式不得残留堆叠信息块"
        );
        let card0 = tree
            .find_by_id("startup-row-0")
            .expect("切到唱片架后必须有第一张卡 startup-row-0");
        assert_eq!(
            card0.bounds.expect("卡必须有几何包围盒").height,
            350,
            "唱片架卡片必须恢复 350 高"
        );
    }
    live.window().request_redraw();
    let shelf_frame = live.window().capture().expect("唱片架态抓帧");
    write_png(&assets.join("chooser-final-shelf-1x.png"), &shelf_frame);
}
