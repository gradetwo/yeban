//! 实验判据：**文件型 demo + 封面图**（exp/demo-cover，Wang 点题，
//! 只做实验、不合入）。
//!
//! 用真实素材（三条文件型 demo 工程 + 专辑图）验证：
//!
//! 1. `YEBAN_DEMOS_DIR` 指向夹具目录时，演示工程分类就是文件型
//!    demo 三条（内置样本已按 Wang 2026-11-23 裁决移出清单），
//!    标题与 BPM 从容器真实读出（「下一个路口」66 /「那个午后」
//!    65.6 /「雨声太吵」129 BPM）；《雨声太吵》的曲风容器里没有
//!    字段，取自同名旁挂 `yusheng.meta.json` 的 `genre`
//!    （「Grunge-Pop」，该值转录自容器 `author` 字段
//!    「雨窗 · Rain-window Grunge-Pop（4:24 版）」里的风格名）；
//! 2. 三行 `has-cover` 都为真（宿主用 `Image::load_from_path`
//!    真实加载了同名封面）；
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
//! 5. 演示分类另有可切换的**堆叠模式**（与唱片架并存，默认
//!    堆叠，Wang 2026-11-23 15:11 终定；判据进场先验默认堆叠、
//!    再切唱片架走架子段、最后切回堆叠验环轨）：第二版按 Wang
//!    反馈改环轨散布——封面沿椭圆环
//!    轨松散散开、角度各不相同、重叠深，最前一张在环心回正；
//!    点任意露出的封面不直接换位，而是沿环次序**循环移位**
//!    逐格步进，直到被点封面转到最前（选中态同步、信息块跟
//!    最前走）；环位是序号的纯函数，切去切回逐值不变；堆叠
//!    态另落 `chooser-demos-stack2-1x.png` 与
//!    `chooser-demos-stack2-3x.png`；视图切换只影响演示分类；
//! 6. 文件型 demo 打开 = 未命名副本：会话换成该工程、不记最近、
//!    源文件字节不变。
//!
//! 素材目录：优先读环境变量 `YEBAN_DEMOS_DIR`，缺省回落到仓库同级
//! 的 `exp-demo-cover-assets/`；判据把三条文件型 demo（《雨声
//! 太吵》在素材根、《下一个路口》《那个午后》在 `all-demos/`）
//! 拷进临时演示目录、按路径序命名，再把 `YEBAN_DEMOS_DIR` 指到
//! 该临时目录。素材不在 ⇒ 本判据**响亮失败**（实验夹具缺失不许
//! 静默跳过）。
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../exp-demo-cover-assets")
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

#[ignore = "requires external exp-demo-cover-assets fixture (not in the Git tree)"]
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
    // 另外两条文件型 demo 的素材在 `all-demos/` 里。
    let all_demos = assets.join("all-demos");
    for name in [
        "下一个路口_女声.yeban",
        "下一个路口_女声-cover.png",
        "那个午后_男声.yeban",
        "那个午后_男声-cover.png",
    ] {
        assert!(
            all_demos.join(name).is_file(),
            "实验素材缺失: {} 不在",
            all_demos.join(name).display()
        );
    }

    let scratch = Scratch::new();
    // 演示目录夹具：三条文件型 demo 拷进临时目录并按路径序命名
    // （01 / 02 / yusheng），扫描序 = 下一个路口 / 那个午后 /
    // 雨声太吵，《雨声太吵》仍是最后一条（本判据多数断言以它
    // 为对象）。标题与 BPM 都从容器实读，改名不影响读数。
    let demos_fixture = scratch.dir.join("demos");
    std::fs::create_dir_all(&demos_fixture).expect("建演示目录夹具");
    let copy_fixture = |from: &Path, to_name: &str| {
        std::fs::copy(from, demos_fixture.join(to_name)).expect("拷演示夹具");
    };
    copy_fixture(&all_demos.join("下一个路口_女声.yeban"), "01-lukou.yeban");
    copy_fixture(
        &all_demos.join("下一个路口_女声-cover.png"),
        "01-lukou-cover.png",
    );
    copy_fixture(&all_demos.join("那个午后_男声.yeban"), "02-wuhou.yeban");
    copy_fixture(
        &all_demos.join("那个午后_男声-cover.png"),
        "02-wuhou-cover.png",
    );
    copy_fixture(&demo_file, "yusheng.yeban");
    copy_fixture(&cover_file, "yusheng-cover.png");
    copy_fixture(&meta_file, "yusheng.meta.json");
    // 《雨声太吵》包内自带的样本树（samples/bass、samples/drums）
    // 与工程同级：打开时引擎按工程相对路径摊平 SFZ，夹具目录里
    // 必须同样摆着这一份，链到素材原件即可（只读）。
    #[cfg(unix)]
    std::os::unix::fs::symlink(assets.join("samples"), demos_fixture.join("samples"))
        .expect("链演示夹具样本目录");
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(assets.join("samples"), demos_fixture.join("samples"))
        .expect("链演示夹具样本目录");
    let demo_file = demos_fixture.join("yusheng.yeban");
    let demo_bytes_before = std::fs::read(&demo_file).expect("读源工程字节");

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
            demos_fixture.as_os_str(),
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
        assert_eq!(rows.len(), 3, "演示分类 = 文件型三条（内置样本已移出清单）");
        assert_eq!(
            rows[0].label.as_str(),
            "下一个路口",
            "第一条标题必须从容器真实读出"
        );
        assert_eq!(
            rows[0].bpm_label.as_str(),
            "66 BPM",
            "第一条 BPM 必须从容器实读"
        );
        assert!(
            rows[0].has_cover,
            "第一条文件型 demo 必须带上真实加载的封面"
        );
        assert_eq!(
            rows[1].label.as_str(),
            "那个午后",
            "第二条标题必须从容器真实读出"
        );
        assert_eq!(
            rows[1].bpm_label.as_str(),
            "65.6 BPM",
            "第二条 BPM 必须从容器实读"
        );
        assert!(
            rows[1].has_cover,
            "第二条文件型 demo 必须带上真实加载的封面"
        );
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
        // 第一条（《下一个路口》）：无旁挂 meta ⇒ 曲风留空、曲风
        // 行不出现；容器实读实算的轨道数行照常出现，其余无数据源
        // 的专辑页字段整行不出现（⛔ 不编造）。
        assert!(
            rows[0].genre.is_empty(),
            "《下一个路口》无旁挂，卡面曲风必须留空，实际 {}",
            rows[0].genre
        );
        let first_labels: Vec<String> = rows[0]
            .detail_rows
            .iter()
            .map(|row| row.label.to_string())
            .collect();
        assert!(
            first_labels.iter().any(|label| label == "轨道数"),
            "文件型 demo 详情必须有实算的轨道数行，实际 {first_labels:?}"
        );
        for absent in ["曲风", "版权声明", "许可协议", "作者网站"] {
            assert!(
                !first_labels.iter().any(|label| label == absent),
                "《下一个路口》无 {absent} 数据源，不许出现该行，实际 {first_labels:?}"
            );
        }
        assert!(rows[2].has_cover, "文件型 demo 必须带上真实加载的封面");
    }

    // ---- 默认视图 = 堆叠（Wang 2026-11-23 15:11 终定）：刚切进
    // 演示分类时最前信息块（最后一张《雨声太吵》）已在；随后点
    // 唱片架段切到架子形态，走下面的架子段判据。 ----------------
    {
        use yeban_ui_test_port::{PointerButton, UiTestPort as _};
        let tree = live.refresh_tree(None).expect("切进演示分类后重抓控件树");
        assert!(
            tree.find_by_id("startup-stack-info-2").is_some(),
            "演示分类默认必须是堆叠视图（最前信息块在）"
        );
        // 「堆叠」二字不上界面（Wang 2026-11-23 15:12）靠截图
        // 目视核：控件树的 label 是 accessible-label（读屏名，
        // 图标-only 按钮必须留名），树上分不出可见文字与读屏名，
        // 故此处只断言切换段在场，可见文字留给证据图亲验。
        assert!(
            tree.find_by_id("startup-demo-view-stack").is_some(),
            "演示分类必须有堆叠切换段（图标-only）"
        );
        live.dispatch_pointer_down("startup-demo-view-shelf", 31.0, 10.0, PointerButton::Left)
            .expect("唱片架切换钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
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

    // ---- 堆叠模式（第二版）：环轨散布 / 确定性 / 循环移位步进 ----
    {
        use yeban_ui_test_port::image::Rect;
        use yeban_ui_test_port::{PointerButton, UiTestPort as _};

        // 包围盒中心（旋转包围盒关于封面中心对称，中心即封面中心）。
        let center = |b: Rect| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);

        // 等转盘步进走完：反复推进 Slint 的计时器与动画，直到
        // 目标行的信息块出现（= 它已转到最前）；再多泵 450ms
        // 让最后一格 200ms 的滑动动画落定，几何才稳。
        fn wait_stack_top(live: &mut LivePort<yeban_app::ui::MainWindow>, info_id: &str) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
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
                    "堆叠步进 10 秒内未把 {info_id} 转到最前（循环移位未走完）"
                );
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let settle_until = std::time::Instant::now() + std::time::Duration::from_millis(450);
            while std::time::Instant::now() < settle_until {
                slint::platform::update_timers_and_animations();
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }

        // 点一张背面封面的露边：先现抓它的包围盒，它在最前封面
        // 的哪一侧，就点哪一侧的中段露条。
        let click_stack_card =
            |live: &mut LivePort<yeban_app::ui::MainWindow>, id: &str, front_cx: i32| {
                let bounds = {
                    let tree = live.refresh_tree(None).expect("点封面前重抓控件树");
                    tree.find_by_id(id)
                        .expect("封面必须在控件树里")
                        .bounds
                        .expect("封面必须有几何包围盒")
                };
                let (cx, _) = center(bounds);
                let dx = if cx < front_cx {
                    24.0
                } else {
                    bounds.width as f64 - 24.0
                };
                live.dispatch_pointer_down(id, dx, bounds.height as f64 / 2.0, PointerButton::Left)
                    .expect("背面封面露边必须可按下");
                live.dispatch_pointer_up(PointerButton::Left)
                    .expect("释放必须送达");
            };

        // 先收起架子模式里展开的第三张卡，进堆叠时信息块是收起态。
        live.refresh_tree(None).expect("收起前重抓控件树");
        live.dispatch_pointer_down("startup-row-details-2", 26.0, 12.0, PointerButton::Left)
            .expect("详情按钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        // 视图切换控件只在演示分类出现：切到最近分类必须消失，
        // 切回演示分类必须回来。（ui 借用只活在语句内，不跨
        // refresh_tree 的可变借用。）
        live.ui().invoke_startup_category_selected(1);
        {
            let tree = live.refresh_tree(None).expect("切分类后重抓控件树");
            assert!(
                tree.find_by_id("startup-demo-view-stack").is_none()
                    && tree.find_by_id("startup-demo-view-shelf").is_none(),
                "视图切换只许在演示分类出现，其他分类不得出现"
            );
        }
        live.ui().invoke_startup_category_selected(2);
        {
            let tree = live.refresh_tree(None).expect("切回演示分类后重抓控件树");
            assert!(
                tree.find_by_id("startup-demo-view-stack").is_some(),
                "演示分类必须有「堆叠」切换钮"
            );
        }
        // 切到堆叠：默认最前 = 最后一张（《雨声太吵》），只有它
        // 的信息块出现。三张封面与信息块的包围盒一并拷出。
        live.dispatch_pointer_down("startup-demo-view-stack", 31.0, 10.0, PointerButton::Left)
            .expect("堆叠切换钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        let (b0, b1, b2, info2_bounds) = {
            let tree = live.refresh_tree(None).expect("切堆叠后重抓控件树");
            let info2 = tree
                .find_by_id("startup-stack-info-2")
                .expect("堆叠模式默认最前（第三张）必须有信息块 startup-stack-info-2");
            assert!(
                info2.label.contains("雨声太吵"),
                "最前信息块必须是《雨声太吵》，实际 {}",
                info2.label
            );
            assert!(
                tree.find_by_id("startup-stack-info-0").is_none()
                    && tree.find_by_id("startup-stack-info-1").is_none(),
                "非最前专辑不得出现信息块"
            );
            let bounds_of = |id: &str| {
                tree.find_by_id(id)
                    .expect("堆叠里封面必须在控件树里")
                    .bounds
                    .expect("封面必须有几何包围盒")
            };
            (
                bounds_of("startup-row-0"),
                bounds_of("startup-row-1"),
                bounds_of("startup-row-2"),
                info2.bounds.expect("信息块必须有几何包围盒"),
            )
        };
        let (c0, c1, c2) = (center(b0), center(b1), center(b2));
        eprintln!(
            "stack2 环位包围盒：row0 x={} y={} {}x{}，row1 x={} y={} {}x{}，row2（最前） x={} y={} {}x{}",
            b0.x,
            b0.y,
            b0.width,
            b0.height,
            b1.x,
            b1.y,
            b1.width,
            b1.height,
            b2.x,
            b2.y,
            b2.width,
            b2.height
        );
        assert!(
            (248..300).contains(&b2.height),
            "最前封面边长应为 256（真机旋转包围盒略大），实际 height={}",
            b2.height
        );
        // 不规整要落在尺寸上：三张封面边长互不相同、最前最大。
        // （包围盒不含旋转——软件渲染器不实现 rotate，见 .slint
        // 堆叠注释——尺寸差异是无头路径能钉的「不规整」证据。）
        assert!(
            b0.height != b1.height && b1.height != b2.height && b0.height != b2.height,
            "三张封面边长必须互不相同（每槽尺寸各异），实际 {} / {} / {}",
            b0.height,
            b1.height,
            b2.height
        );
        assert!(
            b2.height > b0.height && b2.height > b1.height,
            "最前封面必须最大，实际最前 {} 对背面 {} / {}",
            b2.height,
            b0.height,
            b1.height
        );
        // 环形散布：两张背面封面分列最前封面的左上 / 右上环位，
        // 不是第一版那种等距对角错层。
        assert!(
            c0.0 < c2.0 - 40 && c0.1 < c2.1 - 10,
            "第一张应在最前封面的左上环位，实际中心 {c0:?} 对最前 {c2:?}"
        );
        assert!(
            c1.0 > c2.0 + 40 && c1.1 < c2.1 - 10,
            "第二张应在最前封面的右上环位，实际中心 {c1:?} 对最前 {c2:?}"
        );
        // 重叠要深：背面封面与最前封面横向交叠超过 60px。
        assert!(
            b0.x + b0.width as i32 > b2.x + 60 && b1.x < b2.x + b2.width as i32 - 60,
            "背面封面必须与最前封面深度重叠：左 [{},{}] 右 [{},{}] 对最前 [{},{}]",
            b0.x,
            b0.x + b0.width as i32,
            b1.x,
            b1.x + b1.width as i32,
            b2.x,
            b2.x + b2.width as i32
        );
        // 信息块在最前封面下方、不压图。
        assert!(
            info2_bounds.y > b2.y + b2.height as i32 - 4,
            "信息块必须在堆叠区下方，实际 y={}，最前封面底 {}",
            info2_bounds.y,
            b2.y + b2.height as i32
        );
        // 确定性：环位是纯序号函数——切去架子再切回堆叠，三张
        // 封面包围盒逐值不变。
        live.dispatch_pointer_down("startup-demo-view-shelf", 31.0, 10.0, PointerButton::Left)
            .expect("唱片架切换钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        live.dispatch_pointer_down("startup-demo-view-stack", 31.0, 10.0, PointerButton::Left)
            .expect("堆叠切换钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        {
            let tree = live.refresh_tree(None).expect("切回堆叠后重抓控件树");
            for (id, before) in [
                ("startup-row-0", b0),
                ("startup-row-1", b1),
                ("startup-row-2", b2),
            ] {
                let now = tree
                    .find_by_id(id)
                    .expect("封面必须在控件树里")
                    .bounds
                    .expect("封面必须有几何包围盒");
                assert_eq!(
                    now, before,
                    "{id} 切去切回后包围盒必须逐值不变（环位确定性）"
                );
            }
        }
        // 堆叠态截图：整页 1x + 堆叠区（全部封面 ∪ 最前信息块）3x。
        live.window().request_redraw();
        let stack_frame = live.window().capture().expect("堆叠态抓帧");
        write_png(&assets.join("chooser-demos-stack2-1x.png"), &stack_frame);
        let union_x = b0.x.min(b1.x).min(b2.x).min(info2_bounds.x);
        let union_y = b0.y.min(b1.y).min(b2.y).min(info2_bounds.y);
        let union_r = (b0.x + b0.width as i32)
            .max(b1.x + b1.width as i32)
            .max(b2.x + b2.width as i32)
            .max(info2_bounds.x + info2_bounds.width as i32);
        let union_b = (b0.y + b0.height as i32)
            .max(b1.y + b1.height as i32)
            .max(b2.y + b2.height as i32)
            .max(info2_bounds.y + info2_bounds.height as i32);
        let crop = crop_scaled_3x(
            &stack_frame,
            Rect {
                x: union_x,
                y: union_y,
                width: (union_r - union_x) as u32,
                height: (union_b - union_y) as u32,
            },
        );
        write_png(&assets.join("chooser-demos-stack2-3x.png"), &crop);
        // 循环移位一：点第二张（右上环位）——沿环次序步进两格
        // （最前 2 → 0 → 1），它转到最前并落在同一最前环位上，
        // 信息块与选中态同步。
        click_stack_card(&mut live, "startup-row-1", c2.0);
        wait_stack_top(&mut live, "startup-stack-info-1");
        {
            let tree = live.refresh_tree(None).expect("移位一后重抓控件树");
            assert!(
                tree.find_by_id("startup-stack-info-0").is_none()
                    && tree.find_by_id("startup-stack-info-2").is_none(),
                "移位一后只许最前（第二张）有信息块"
            );
            let nb1 = tree
                .find_by_id("startup-row-1")
                .expect("封面必须在控件树里")
                .bounds
                .expect("封面必须有几何包围盒");
            let nc1 = center(nb1);
            assert!(
                (nc1.0 - c2.0).abs() <= 6 && (nc1.1 - c2.1).abs() <= 6,
                "移位一后第二张必须落在原最前环位 {c2:?}，实际 {nc1:?}"
            );
        }
        assert_eq!(
            live.ui().get_startup_selected_entry(),
            1,
            "移位一必须同步选中态到第二张"
        );
        // 循环移位二：点第一张——再步进两格（最前 1 → 2 → 0），
        // 第一张到最前、落同一环位。
        click_stack_card(&mut live, "startup-row-0", c2.0);
        wait_stack_top(&mut live, "startup-stack-info-0");
        {
            let tree = live.refresh_tree(None).expect("移位二后重抓控件树");
            assert!(
                tree.find_by_id("startup-stack-info-1").is_none()
                    && tree.find_by_id("startup-stack-info-2").is_none(),
                "移位二后只许最前（第一张）有信息块"
            );
            let nb0 = tree
                .find_by_id("startup-row-0")
                .expect("封面必须在控件树里")
                .bounds
                .expect("封面必须有几何包围盒");
            let nc0 = center(nb0);
            assert!(
                (nc0.0 - c2.0).abs() <= 6 && (nc0.1 - c2.1).abs() <= 6,
                "移位二后第一张必须落在原最前环位 {c2:?}，实际 {nc0:?}"
            );
        }
        assert_eq!(
            live.ui().get_startup_selected_entry(),
            0,
            "移位二必须同步选中态到第一张"
        );
        // 切回唱片架：卡片恢复 272×350 架子形态，堆叠信息块消失。
        live.dispatch_pointer_down("startup-demo-view-shelf", 31.0, 10.0, PointerButton::Left)
            .expect("唱片架切换钮必须可按下");
        live.dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        {
            let tree = live.refresh_tree(None).expect("切回架子后重抓控件树");
            let card2 = tree
                .find_by_id("startup-row-2")
                .expect("切回架子后必须有第三张卡 startup-row-2");
            assert_eq!(
                card2.bounds.expect("卡必须有几何包围盒").height,
                350,
                "切回唱片架后卡片必须恢复 350 高"
            );
            assert!(
                tree.find_by_id("startup-stack-info-0").is_none(),
                "架子模式不得残留堆叠信息块"
            );
        }
    }

    // ---- 打开语义 ------------------------------------------------------
    // 《雨声太吵》的吉他轨引用系统音源库 karoryfer-emilyguitar
    // （assets/samples/karoryfer-emilyguitar/emily_basic.sfz）。
    // 该库装没装决定打开的成败，两条路都钉死真实行为：
    // - 在（探引擎实际会搜的四处根）：打开成功——选择器关、会话
    //   换成该工程、不记最近、源文件字节不变；
    // - 不在（本机即如此）：引擎换代必须响亮失败——选择器保持
    //   打开、模块回执点名缺失的 SFZ 文件、不记最近、源文件
    //   字节不变（默认不能静默）。
    // 「文件型 demo 打开成未命名副本」的成功语义另有
    // startup_chooser 判据用自包含工程全环境钉死，不在此重复。
    let emily_sfz = "karoryfer-emilyguitar/emily_basic.sfz";
    let emily_roots = [
        demos_fixture.join("assets/samples"),
        std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(".local/share/yeban/samples"))
            .unwrap_or_default(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/samples"),
    ];
    let emily_installed = emily_roots
        .iter()
        .any(|root| root.join(emily_sfz).is_file());
    {
        let ui = live.ui();
        ui.invoke_startup_entry_activated(2);
        if emily_installed {
            assert!(
                !ui.get_startup_chooser_open(),
                "文件型 demo 打开后选择器必须关；模块回执: {}；当前会话标题: {}",
                ui.get_module_status(),
                undo.project().title
            );
            assert_eq!(
                undo.project().title,
                "雨声太吵",
                "会话必须换成文件型 demo 那份工程"
            );
        } else {
            assert!(
                ui.get_startup_chooser_open(),
                "缺系统音源库时打开必须失败且选择器保持打开；模块回执: {}",
                ui.get_module_status()
            );
            let status = ui.get_module_status();
            assert!(
                status.contains("open-demo") && status.contains("emily_basic.sfz"),
                "失败回执必须点名缺失的 SFZ（响亮失败）；模块回执: {status}"
            );
        }
    }
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
