//! 实验判据：**文件型 demo + 封面图**（exp/demo-cover，Wang 点题，
//! 只做实验、不合入）。
//!
//! 用真实素材（《雨声太吵》工程 + Wang 上传的专辑图）验证：
//!
//! 1. `YEBAN_DEMOS_DIR` 指向素材目录时，演示工程分类多出一条文件型
//!    demo，标题与 BPM 从容器真实读出（「雨声太吵」/ 129 BPM）；
//!    曲风容器里没有字段，取自同名旁挂 `yusheng.meta.json` 的
//!    `genre`（「Grunge-Pop」，该值转录自容器 `author` 字段
//!    「雨窗 · Rain-window Grunge-Pop（4:24 版）」里的风格名）；
//! 2. 该行 `has-cover` 为真（宿主用 `Image::load_from_path` 真实加载
//!    了同名封面），两条内置 demo 无封面；
//! 3. 卡片主面**没有**轨道数（`detail` 为空）；专辑页字段只进
//!    「详情」展开区的带标签行（`detail_rows`），按专辑页顺序
//!    排列、**有数据才出现该行**：《雨声太吵》实读实算 + 旁挂
//!    得 作者 雨窗 / 曲风 Grunge-Pop / 时长 4:32 / 小节数 146 /
//!    轨道数 7 共五行；发布日期 / 简介 / 制作名单 / 版权声明 /
//!    许可协议 / 作者网站在本 demo **没有任何数据源**，整行
//!    不出现；
//! 4. 整页渲染落盘 `chooser-demos-tile-1x.png`，文件型 demo 卡片
//!    区域裁出放大 3 倍落盘 `chooser-demos-tile-card-3x.png`；
//!    「详情」原地展开态另落 `chooser-demos-expand-1x.png` 与
//!    `chooser-demos-expand-card-3x.png`（都在素材目录里，
//!    **不进仓库**）；展开是手风琴：同一时刻只许一张卡展开，
//!    展开区长在卡内、封面全程可见（无浮层）；
//! 5. 文件型 demo 打开 = 未命名副本：会话换成该工程、不记最近、
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
    let meta_file = assets.join("yusheng.meta.json");
    assert!(
        meta_file.is_file(),
        "实验素材缺失: {} 不在（先把曲风旁挂放进素材目录）",
        meta_file.display()
    );
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
        assert_eq!(
            rows[2].genre.as_str(),
            "Grunge-Pop",
            "文件型 demo 曲风必须来自旁挂 yusheng.meta.json 的 genre"
        );
        assert_eq!(
            rows[2].bpm_label.as_str(),
            "129 BPM",
            "文件型 demo 的 BPM 必须从容器实读"
        );
        assert!(
            rows[2].detail.is_empty(),
            "卡片主面不得出现轨道数等技术信息，实际 detail={}",
            rows[2].detail
        );
        // 展开区带标签行：专辑页顺序、有数据才有行。本 demo 的
        // 版权 / 许可 / 网站 / 发布日期 / 简介 / 制作名单没有任何
        // 数据源，整行不许出现（⛔ 不编造）。
        let pairs: Vec<(String, String)> = rows[2]
            .detail_rows
            .iter()
            .map(|row| (row.label.to_string(), row.value.to_string()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("作者".to_string(), "雨窗".to_string()),
                ("曲风".to_string(), "Grunge-Pop".to_string()),
                ("时长".to_string(), "4:32".to_string()),
                ("小节数".to_string(), "146".to_string()),
                ("轨道数".to_string(), "7".to_string()),
            ],
            "《雨声太吵》详情行必须是实读实算 + 旁挂的五行，顺序按专辑页口径"
        );
        for absent in [
            "发布日期",
            "简介",
            "制作名单",
            "版权声明",
            "许可协议",
            "作者网站",
        ] {
            assert!(
                !rows[2].details.contains(absent),
                "无数据源的字段 {absent} 不许出现在详情里，实际 {}",
                rows[2].details
            );
        }
        assert!(
            rows[2].details.contains("作者 雨窗"),
            "详情纯文本（无障碍标签）里必须有实读的作者，实际 {}",
            rows[2].details
        );
        // 内置 demo：样本工程自带 author（实读
        // "Yeban Project Contributors"，无 `·` 注记段，整段即
        // 作者名）；无曲风、无旁挂 ⇒ 其余专辑页字段不出现。
        let builtin_labels: Vec<String> = rows[0]
            .detail_rows
            .iter()
            .map(|row| row.label.to_string())
            .collect();
        assert!(
            builtin_labels.iter().any(|label| label == "轨道数"),
            "内置 demo 详情必须有实算的轨道数行，实际 {builtin_labels:?}"
        );
        let builtin_author = rows[0]
            .detail_rows
            .iter()
            .find(|row| row.label.as_str() == "作者")
            .map(|row| row.value.to_string());
        assert_eq!(
            builtin_author.as_deref(),
            Some("Yeban Project Contributors"),
            "内置 demo 作者必须实读样本工程 author 字段"
        );
        for absent in ["曲风", "版权声明", "许可协议", "作者网站"] {
            assert!(
                !builtin_labels.iter().any(|label| label == absent),
                "内置 demo 无 {absent} 数据源，不许出现该行，实际 {builtin_labels:?}"
            );
        }
        assert!(rows[2].has_cover, "文件型 demo 必须带上真实加载的封面");
    }

    // ---- 整帧截图（架子在顶部：第一行两张卡 + 架板带） ----------------
    live.window().request_redraw();
    let frame = live.window().capture().expect("抓帧");
    let full_png = assets.join("chooser-demos-tile-1x.png");
    write_png(&full_png, &frame);

    // ---- 手风琴：展开 / 互斥 / 收起（都在首行，不用滚动） --------------
    {
        use yeban_ui_test_port::{PointerButton, UiTestPort as _};
        let click_details = |live: &mut LivePort<yeban_app::ui::MainWindow>, index: usize| {
            live.refresh_tree(None).expect("点详情前重抓控件树");
            live.dispatch_pointer_down(
                &format!("startup-row-details-{index}"),
                26.0,
                12.0,
                PointerButton::Left,
            )
            .expect("详情按钮必须可按下");
            live.dispatch_pointer_up(PointerButton::Left)
                .expect("释放必须送达");
        };
        // 展开第 0 张：面板出现、内容是带标签行、钮文案变「收起」。
        click_details(&mut live, 0);
        let tree = live.refresh_tree(None).expect("展开后重抓控件树");
        let panel = tree
            .find_by_id("startup-row-details-panel-0")
            .expect("点「详情」后控件树里必须出现卡内展开区 startup-row-details-panel-0");
        assert!(
            panel.label.contains("轨道数"),
            "展开区必须是带标签的专辑页行，实际标签 {}",
            panel.label
        );
        let toggle = tree
            .find_by_id("startup-row-details-0")
            .expect("详情钮必须在控件树里");
        assert_eq!(toggle.label, "收起", "展开后入口文案必须变「收起」");
        // 展开第 1 张：第 0 张自动收起（同一时刻只许一张展开）。
        click_details(&mut live, 1);
        let tree = live.refresh_tree(None).expect("展开第 1 张后重抓控件树");
        assert!(
            tree.find_by_id("startup-row-details-panel-0").is_none(),
            "展开另一张时当前张必须先收起（手风琴）"
        );
        assert!(
            tree.find_by_id("startup-row-details-panel-1").is_some(),
            "第 1 张的展开区必须出现"
        );
        // 再点第 1 张的「收起」：面板消失。
        click_details(&mut live, 1);
        let tree = live.refresh_tree(None).expect("收起后重抓控件树");
        assert!(
            tree.find_by_id("startup-row-details-panel-1").is_none(),
            "点「收起」后展开区必须消失"
        );
    }

    // ---- 滚轮把架子滚到第三张卡完整入镜，再裁图 ----------------------
    // 对话框可视高度只容得下一行多，第三张卡（第二行）在视口外，
    // 直接按包围盒裁会裁到视口外的底图。照
    // `tests/arrangement_vertical_scroll.rs` 的符号口径：看下面
    // 的行送负 delta。
    {
        let tree = live.refresh_tree(None).expect("重抓控件树");
        let list_node = tree
            .find_by_id("startup-chooser-list")
            .expect("控件树里必须有列表区 startup-chooser-list");
        let list_bounds = list_node.bounds.expect("列表区必须有几何包围盒");
        live.window()
            .dispatch(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(
                    (list_bounds.x + list_bounds.width as i32 / 2) as f32,
                    (list_bounds.y + list_bounds.height as i32 / 2) as f32,
                ),
                delta_x: 0.0,
                delta_y: -400.0,
            });
    }
    live.window().request_redraw();
    let frame = live.window().capture().expect("滚动后抓帧");

    // ---- 文件型 demo 卡片裁图（控件树几何定位，放大 3 倍） ------------
    let bounds = {
        let tree = live.refresh_tree(None).expect("滚动后重抓控件树");
        let row_node = tree
            .find_by_id("startup-row-2")
            .expect("控件树里必须有第三张卡 startup-row-2");
        row_node.bounds.expect("第三张卡必须有几何包围盒")
    };
    assert!(
        bounds.y >= 0 && bounds.y + bounds.height as i32 <= frame.size().height as i32,
        "滚动后第三张卡必须完整入镜，实际 y={} height={}",
        bounds.y,
        bounds.height
    );
    let crop = crop_scaled_3x(&frame, bounds);
    let crop_png = assets.join("chooser-demos-tile-card-3x.png");
    write_png(&crop_png, &crop);
    eprintln!(
        "demo-cover: 整帧 {} ({}x{})，第三张卡包围盒 x={} y={} {}x{}，裁图 {} ({}x{})",
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

    // ---- 《雨声太吵》详情：原地展开 + 展开态截图 ------------------------
    {
        use yeban_ui_test_port::{PointerButton, UiTestPort as _};
        live.refresh_tree(None).expect("点详情前重抓控件树");
        live.dispatch_pointer_down("startup-row-details-2", 26.0, 12.0, PointerButton::Left)
            .expect("详情按钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        // 展开后卡高 471px 超过列表可视高，再滚一段让整卡入镜。
        // 落点在卡面上，滚轮事件沿元素树上冒到 Flickable。
        {
            let tree = live.refresh_tree(None).expect("滚动前重抓控件树");
            let card_node = tree
                .find_by_id("startup-row-2")
                .expect("控件树里必须有第三张卡 startup-row-2");
            let card_bounds = card_node.bounds.expect("第三张卡必须有几何包围盒");
            live.window()
                .dispatch(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(
                        (card_bounds.x + card_bounds.width as i32 / 2) as f32,
                        (card_bounds.y + card_bounds.height as i32 / 2) as f32,
                    ),
                    delta_x: 0.0,
                    delta_y: -400.0,
                });
        }
        let tree = live.refresh_tree(None).expect("展开后重抓控件树");
        let panel = tree
            .find_by_id("startup-row-details-panel-2")
            .expect("点「详情」后控件树里必须出现卡内展开区 startup-row-details-panel-2");
        assert!(
            panel.label.contains("作者 雨窗") && panel.label.contains("曲风 Grunge-Pop"),
            "展开区必须带标签列出实读字段，实际 {}",
            panel.label
        );
        assert!(
            !panel.label.contains("版权声明") && !panel.label.contains("许可协议"),
            "无数据源的字段不许进展开区，实际 {}",
            panel.label
        );
        // 展开区长在同一张卡内部：卡变高、面板被卡包住、面板在
        // 封面与文字块下方（不是盖住整卡的浮层）。
        let card = tree
            .find_by_id("startup-row-2")
            .expect("控件树里必须有第三张卡 startup-row-2");
        let card_bounds = card.bounds.expect("卡必须有几何包围盒");
        let panel_bounds = panel.bounds.expect("展开区必须有几何包围盒");
        assert!(
            card_bounds.height > 350,
            "展开后卡片必须长高，实际 height={}",
            card_bounds.height
        );
        assert!(
            panel_bounds.x >= card_bounds.x
                && panel_bounds.y > card_bounds.y + 300
                && panel_bounds.y + panel_bounds.height as i32
                    <= card_bounds.y + card_bounds.height as i32,
            "展开区必须位于卡内下半截且不溢出：卡 y={} h={}，区 y={} h={}",
            card_bounds.y,
            card_bounds.height,
            panel_bounds.y,
            panel_bounds.height
        );
        live.window().request_redraw();
        let expand_frame = live.window().capture().expect("展开后抓帧");
        assert!(
            card_bounds.y >= 0
                && card_bounds.y + card_bounds.height as i32 <= expand_frame.size().height as i32,
            "展开后整卡必须完整入镜，实际 y={} height={}",
            card_bounds.y,
            card_bounds.height
        );
        write_png(&assets.join("chooser-demos-expand-1x.png"), &expand_frame);
        let crop = crop_scaled_3x(&expand_frame, card_bounds);
        write_png(&assets.join("chooser-demos-expand-card-3x.png"), &crop);
    }

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
