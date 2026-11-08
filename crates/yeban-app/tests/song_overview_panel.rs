//! ⭐ **整曲多层可视化总览图（Song Overview）Phase 1** —— 端到端判据。
//!
//! ## 被补上的缺口
//!
//! `ui/workspace/song_overview_panel.slint`（底部、状态栏之上、可折叠 104 px）在
//! 此切片之前**一处都没有**接线：四层数据只在设计稿 §8.2 里是形状，四个交互只在
//! 设计稿 §7 里是文字。本片把面板接成四条真实的指针链：
//!
//! ```text
//! 点击画布 ⇒ seek-requested(分数) ⇒ song_overview::seek_tick_for_fraction
//!   ⇒ EngineHost::seek ⇒ TransportCommand::SeekTicks ⇒ apply_transport
//! 拖视口框体 ⇒ viewport-scroll-requested(px) ⇒ roll-scroll-x（编排视图跟滚）
//! 拖视口框右缘 ⇒ viewport-zoom-requested(tick/px) ⇒ set_roll_ticks_per_pixel
//!   + 重投影（bar_positions 跟着变）
//! 点图层胶囊 ⇒ layer-toggle(下标) ⇒ show-* 翻转（界面不自己翻状态）
//! 双击图层胶囊 ⇒ layer-solo(下标) ⇒ 独显 / 恢复全显
//! 悬停画布 ⇒ hover-requested(分数) ⇒ hover-readout（小节号 / LUFS / 张力 / 前两频段）
//! ```
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（面板在场 + 稳定 ID + 104 px 几何） | 11 个初始可见语义 ID 全在树里（悬停提示框初始不可见，悬停后才出现，见 ⑪）；面板高 104、画布 82、底部条 22；面板顶缘 = 窗高 − 24（状态栏）− 104 |
//! | ②（stub 数据到面板） | 总小节 16、频段颜色 16×8 = 128、标尺首项是段名 "Intro"、段名含 "Chorus" |
//! | ③（点画布第 5 小节 ⇒ 跳转） | 点画布 5.25/16 处（第 5 小节前 1/4）⇒ 引擎 `position_ticks == 5 × 3840`，界面镜面同值，日志末条 `SeekTicks`，恰 +1 条（吸附口径与标尺 `U-07 R1` 一致：就近小节线，平局向后） |
//! | ④（点在视口框里 ⇒ 照样跳转） | 点落在视口框体内的 2.25/16 处 ⇒ 照样落到第 2 小节起点（框体 ⛔ 不吞轻点；拖动才滚动，见 ⑦） |
//! | ⑤（点两缘 ⇒ 夹紧） | 点画布最右列 ⇒ 全曲末端 16 × 3840（与标尺判据 ④ 同一右缘值）；点最左列 ⇒ 0 |
//! | ⑥（视口框几何 = 滚动 / 缩放公式） | 框左缘 / 框宽与 `roll-scroll-x`、`roll-ticks-per-pixel` 的换算在 2 px 内；缩放到 60 后框宽翻倍（±2 px） |
//! | ⑦（拖框体 ⇒ 编排视图滚动） | 框体向右拖 100 px ⇒ `roll-scroll-x` +272（100 画布 px = 8170 tick ÷ 30 tick/px），走带位置一位不动（拖动 ⛔ 不跳转） |
//! | ⑧（拖框右缘 ⇒ 缩放 + 重投影） | 右缘向右拖 40 px ⇒ `roll-ticks-per-pixel` 从 30 变 34（±1），第 5 条小节线列位跟着变 |
//! | ⑨（点能量胶囊 ⇒ 隐藏） | 点 `[能量]` 胶囊 ⇒ `show-energy` 变 false、树上 `checked` 变 false；再点 ⇒ 恢复 true |
//! | ⑩（双击情绪胶囊 ⇒ 独显 / 恢复） | 双击 `[情绪]` ⇒ 只剩情绪层可见；再双击 ⇒ 四层全可见 |
//! | ⑪（悬停 ⇒ 小节读数） | 无按下移动到第 7 小节上 ⇒ `hover-visible` 真、读数含 "第 7 小节"、LUFS、张力与前两频段 |
//! | ⑫（折叠键 ⇒ 22 px / 恢复 104 px） | 点折叠键 ⇒ 面板高 22；再点 ⇒ 回 104，画布在场 |
//! | ⑬（视觉验收 1:1 + 3x） | 截图非黑、面板区多色；三张 PNG 落盘供执行员肉眼验收（1:1、1:1 最近邻放大 3x、大窗口重布局） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ **不证明 dsp 真实数据**：本片的数据是 stub view-model（设计稿 §8.2 列式形状的
//!   固定假数据，标题带「试算」）；dsp 分析线落地后由协调员派人换真实映射，属性名与
//!   几何一位不动。
//! - ⛔ **不证明生产装配**：`main.rs` 的 `wire_callbacks` 接线（`wire_song_overview` 的
//!   生产调用点）不在本执行员的文件清单里，由协调员跟进；本片在测试里调**同一个**
//!   `wire_song_overview`，接线函数本身是同一份。
//! - ⛔ **不证明出声**：只断言控制面位置与日志（与 `arrangement_ruler_seek.rs` 同款口径）。
//! - 5 张 Linux golden 会因此面板确定性过期（面板占 104 px + 控制台区上移）：
//!   本片**不**重录 golden（文件不在清单里），由协调员走手动门重录。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]` 只建
//! **一个** `LivePort`（与 `tests/arrangement_ruler_seek.rs` 同款）。

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::song_overview::{self, SongOverview};
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_engine::ring::TransportCommand;
use yeban_ui_test_port::render::{LivePort, write_artifact};
use yeban_ui_test_port::{Permission, PointerButton, Rect, Rgb8Image, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);
/// 提交时间戳（固定值 ⇒ 判据确定性，与 `tests/clip_drag.rs` 同款）。
const NOW_MS: u64 = 1_700_000_000_000;
/// 演示工程：16 小节、4/4（`bar_length_ticks` 3840）。
const BAR_TICKS: u64 = 3840;
const TOTAL_BARS: usize = 16;
/// 面板几何：总高 104（画布 82 + 底部 22），状态栏 24。
/// ⚠ 总高是 104，不是简报里写的 100：82 + 22 = 104（设计稿 §7.1 的硬约束是
/// 96–120 px，104 在内；「100 px」只是约数）。按 100 做会把底部条末 4 px
/// （标签尺下半）裁掉，视觉验收实测标签只剩顶部 1 px。
const PANEL_HEIGHT: u32 = 104;
const CANVAS_HEIGHT: u32 = 82;
const STRIP_HEIGHT: u32 = 22;
const STATUS_BAR_HEIGHT: u32 = 24;

const PANEL: &str = "song-overview-panel";
const CANVAS: &str = "song-overview-canvas";
const LENS: &str = "song-overview-viewport";
const RESIZE: &str = "song-overview-viewport-resize";
const ENERGY_PILL: &str = "song-overview-layer-energy-pill";
const TENSION_PILL: &str = "song-overview-layer-tension-pill";
const COLLAPSE: &str = "song-overview-collapse-button";

/// 生产形状的夹具：真引擎（`ProductionLoop`）＋ 真活窗口 ＋ **产品那一个**接线函数。
struct Rig {
    /// 持有它 ⇒ `EngineRebuild::collector` 的消费端一直在（与 `main.rs` 同一装配）。
    _production: ProductionLoop,
    engine: Rc<RefCell<EngineHost>>,
    /// 与 `main.rs` 传给 `wire_clip_scope` / `wire_ruler_seek` 的**同一种**活投影句柄。
    view: Rc<RefCell<ViewState>>,
    port: LivePort<MainWindow>,
    project: yeban_model::YebanProjectV1,
    /// stub 数据的单一事实源（换工程后调 `refresh_song_overview` 即更新）。
    _state: Rc<RefCell<SongOverview>>,
}

impl Rig {
    fn new() -> Self {
        let project = demo_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        assert!(production.has_engine(), "引擎必须活着");
        let engine = production.engine_handle();
        // `src/main.rs` 的同一行：新引擎默认自由跑，而界面初始态是停住。
        engine.borrow_mut().stop();

        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        let live_view = Rc::new(RefCell::new(view));
        let undo = Rc::new(UndoPort::new(
            UndoSession::open("<song-overview>", "yeban-app", project.clone(), NOW_MS)
                .expect("打开"),
        ));
        // ⭐ 与产品装配**同一对**函数：`main.rs` 的 `wire_callbacks` 调的就是这两个，
        // 再加本片的 `wire_song_overview`（生产调用点由协调员在 `main.rs` 里补）。
        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        let state = song_overview::wire_song_overview(
            port.ui(),
            Rc::clone(&engine),
            Rc::clone(&live_view),
            Some(Rc::clone(&undo)),
        );
        host::apply_transport(port.ui(), engine.borrow().transport());

        let mut rig = Self {
            _production: production,
            engine,
            view: live_view,
            port,
            project,
            _state: state,
        };
        // 控件树先抓一次：`dispatch_pointer_down` 按**缓存的**包围盒换算窗口坐标。
        rig.port
            .refresh_tree(None)
            .expect("必须能抓控件树（面板几何来自这一次抓取）");
        rig
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    /// 重新抓一次控件树（UI 状态变了之后调用；几何会更新）。
    fn refresh(&mut self) {
        self.port.refresh_tree(None).expect("状态变后必须能重抓树");
    }

    /// 取某个**语义 ID** 在运行时控件树里的包围盒（每次都重读 —— 端口里那份会过期）。
    fn bounds(&self, element_id: &str) -> Rect {
        self.port
            .tree()
            .find_by_id(element_id)
            .expect("控件树里必须有该语义 ID")
            .bounds
            .expect("该元素必须有几何包围盒")
    }

    /// 引擎的走带位置（**事实源**，不是界面镜面）。
    fn position(&self) -> u64 {
        self.engine.borrow().transport().position_ticks
    }

    fn journal_len(&self) -> usize {
        self.engine.borrow().transport_journal().len()
    }

    /// 动作日志的末条命令。
    fn last_command(&self) -> Option<TransportCommand> {
        self.engine
            .borrow()
            .transport_journal()
            .last()
            .map(|record| record.command)
    }

    /// 在画布上真实按下一次：局部坐标 `(x, y)` ⇒ 走命中测试，⛔ 不直接调回调。
    fn click_canvas(&mut self, local_x: f64, local_y: f64) {
        self.port
            .dispatch_pointer_down(CANVAS, local_x, local_y, PointerButton::Left)
            .expect("Interactive 权限下画布必须可按下");
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
    }

    /// 在画布上按**横坐标分数**点一次（`y` 取画布中点 41 px）。
    fn click_canvas_fraction(&mut self, fraction: f64) {
        let canvas = self.bounds(CANVAS);
        self.click_canvas(f64::from(canvas.width) * fraction, 41.0);
    }

    /// 在某个语义 ID 的包围盒中心真实按下一次。
    fn click_center(&mut self, element_id: &str) {
        let bounds = self.bounds(element_id);
        self.port
            .dispatch_pointer_down(
                element_id,
                f64::from(bounds.width) / 2.0,
                f64::from(bounds.height) / 2.0,
                PointerButton::Left,
            )
            .expect("Interactive 权限下该元素必须可按下");
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
    }

    /// 在某个语义 ID 上真实双击一次（两次快速按下 / 释放 ⇒ Slint 的 `double-clicked`）。
    fn double_click_center(&mut self, element_id: &str) {
        for _ in 0..2 {
            let bounds = self.bounds(element_id);
            self.port
                .dispatch_pointer_down(
                    element_id,
                    f64::from(bounds.width) / 2.0,
                    f64::from(bounds.height) / 2.0,
                    PointerButton::Left,
                )
                .expect("Interactive 权限下该元素必须可按下");
            self.port
                .dispatch_pointer_up(PointerButton::Left)
                .expect("释放必须送达");
        }
    }

    /// 投影画出的第 `index` 条小节线在**编排视图**里的列（= 界面自己的读数）。
    fn bar_line_px(&self, index: usize) -> f64 {
        let model = self.ui().get_bar_positions();
        let rows = slint::Model::row_count(&model);
        assert!(
            index < rows,
            "投影必须画出第 {index} 条小节线（实得 {rows} 条）"
        );
        f64::from(model.iter().nth(index).expect("逐项必有值"))
    }

    /// 换横向缩放并**重投影**（与 `arrangement_ruler_seek.rs` 的 `set_zoom` 同一手法）：
    /// 缩放是视图态 ⇒ 走带与工程都不动，只有网格的像素口径变。
    fn set_zoom(&mut self, ticks_per_pixel: u64) {
        if !host::set_roll_ticks_per_pixel(self.ui(), ticks_per_pixel) {
            return;
        }
        let fresh = host::project_with_view_state(&self.project, self.ui())
            .expect("演示工程在新缩放下必须能投影");
        let scroll = self.ui().get_roll_scroll_x();
        host::apply_view(self.ui(), &fresh, f64::from(WINDOW.width) as f32, scroll);
        *self.view.borrow_mut() = fresh;
        self.refresh();
    }

    /// 把一帧裁成面板区（`x`/`y`/`w`/`h` 都是窗口逻辑像素）。
    fn crop_panel(frame: &Rgb8Image, x: u32, y: u32, w: u32, h: u32) -> Rgb8Image {
        let mut bytes = Vec::with_capacity((w * h * 3) as usize);
        for row in y..y + h {
            let start = ((row * frame.width() + x) * 3) as usize;
            let end = start + (w * 3) as usize;
            bytes.extend_from_slice(&frame.pixels()[start..end]);
        }
        Rgb8Image::from_raw(Size::new(w, h), bytes).expect("裁剪尺寸必须自洽")
    }

    /// 最近邻放大三倍（每像素 → 3×3 同色块，像素一位不动，只放大看）。
    fn upscale_3x(image: &Rgb8Image) -> Rgb8Image {
        let (w, h) = (image.width(), image.height());
        let mut bytes = Vec::with_capacity((w * h * 27) as usize);
        for row in 0..h {
            let start = (row * w * 3) as usize;
            let end = start + (w * 3) as usize;
            let line = &image.pixels()[start..end];
            for _ in 0..3 {
                for px in line.as_chunks::<3>().0 {
                    for _ in 0..3 {
                        bytes.extend_from_slice(px);
                    }
                }
            }
        }
        Rgb8Image::from_raw(Size::new(w * 3, h * 3), bytes).expect("放大尺寸必须自洽")
    }
}

/// ①：面板在场 + 11 个稳定 ID（初始可见者）+ 104 px 几何（画布 82 + 底部 22，状态栏之上）。
///
/// `song-overview-hover-readout` 不在初始树里：提示框 `visible` 绑 `hover-visible`，
/// 初始 false ⇒ 不可见节点不进树；它在悬停后出现，由判据 ⑪ 断言。
#[test]
fn panel_is_present_with_stable_ids_and_104px_geometry() {
    let rig = Rig::new();
    let tree = rig.port.tree();
    for id in [
        "song-overview-panel",
        "song-overview-canvas",
        "song-overview-playhead",
        "song-overview-viewport",
        "song-overview-viewport-resize",
        "song-overview-density-strip",
        "song-overview-layer-tension-pill",
        "song-overview-layer-energy-pill",
        "song-overview-layer-bands-pill",
        "song-overview-layer-density-pill",
        "song-overview-collapse-button",
    ] {
        assert!(
            tree.contains(id),
            "控件树里必须有语义 ID `{id}`（DoD §3.6 稳定 ID）"
        );
    }

    let panel = rig.bounds(PANEL);
    assert_eq!(panel.height, PANEL_HEIGHT, "面板总高必须是 104 px");
    assert_eq!(
        panel.y,
        i32::try_from(WINDOW.height).expect("窗高在 i32 内")
            - i32::try_from(STATUS_BAR_HEIGHT).expect("状态栏高在 i32 内")
            - i32::try_from(PANEL_HEIGHT).expect("面板高在 i32 内"),
        "面板顶缘 = 窗高 − 状态栏 24 − 面板 104（状态栏之上）"
    );
    assert_eq!(panel.x, 0, "面板从窗口左缘起");
    assert_eq!(
        panel.width, WINDOW.width,
        "面板占满窗口宽（画布左右留白在面板内部）"
    );

    let canvas = rig.bounds(CANVAS);
    assert_eq!(canvas.height, CANVAS_HEIGHT, "主画布必须是 82 px");
    assert_eq!(canvas.y, panel.y, "画布顶缘 = 面板顶缘");
    let strip = rig.bounds("song-overview-density-strip");
    assert_eq!(strip.height, STRIP_HEIGHT, "底部条必须是 22 px");
    assert_eq!(
        strip.y,
        panel.y + i32::try_from(CANVAS_HEIGHT).expect("画布高在 i32 内"),
        "底部条顶缘 = 面板顶缘 + 82"
    );
}

/// ②：stub 数据到面板（总小节 16、频段颜色 128、标尺首项是段名）。
#[test]
fn stub_data_reaches_the_panel() {
    let rig = Rig::new();
    assert_eq!(
        rig.ui().get_song_overview_total_bars(),
        i32::try_from(TOTAL_BARS).expect("小节数在 i32 内"),
        "stub 必须是 16 小节（演示工程）"
    );
    let band_colors = rig.ui().get_song_overview_band_colors();
    assert_eq!(
        slint::Model::row_count(&band_colors),
        TOTAL_BARS * 8,
        "频段颜色必须是 16 小节 × 8 频段 = 128 项（§8.2 列式形状）"
    );
    let ruler = rig.ui().get_song_overview_ruler_labels();
    assert_eq!(
        slint::Model::row_count(&ruler),
        TOTAL_BARS,
        "标尺标签必须每小节一项"
    );
    assert_eq!(
        ruler.iter().next().expect("首项必有值").to_string(),
        "Intro",
        "第 0 小节的标尺必须是段名 Intro（段落起点优先于小节号）"
    );
    let sections: Vec<String> = rig
        .ui()
        .get_song_overview_section_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    assert!(
        sections.iter().any(|name| name == "Chorus"),
        "段名里必须有 Chorus（演示工程四段）：{sections:?}"
    );
    // 调号在 stub 里是空串（`song_overview.rs:124` 的既定设计：dsp 线落地后由
    // `song_meta.key_signature` 填）；张力数组本身必须每小节一项。
    assert!(
        rig.ui().get_song_overview_key_signature().is_empty(),
        "stub 的调号必须是空串（dsp 线填真实值之前，⛔ 不编一个假调号）"
    );
    assert_eq!(
        slint::Model::row_count(&rig.ui().get_song_overview_tension()),
        TOTAL_BARS,
        "张力必须是每小节一项"
    );
    assert_eq!(
        slint::Model::row_count(&rig.ui().get_song_overview_tension_colors()),
        TOTAL_BARS,
        "张力颜色必须是每小节一项（随调式效价变色）"
    );
}

/// ③：点画布 5.25/16 处（第 5 小节前 1/4）⇒ 引擎到第 5 小节起点，日志恰 +1 条 `SeekTicks`。
///
/// 吸附口径与标尺 `U-07 R1` 一致（就近小节线，平局向后）：5.25/16 × 61440 = 20160
/// tick = 5.25 小节 ⇒ 最近的是第 5 小节起点 19200。点在小节正中（5.5/16）会
/// 平局向后落到第 6 小节起点 —— 那是吸附的定义，不是本片的 bug。
#[test]
fn clicking_canvas_seeks_to_the_clicked_bar() {
    let mut rig = Rig::new();
    let journal_before = rig.journal_len();
    rig.click_canvas_fraction(5.25 / 16.0);

    let expected = 5 * BAR_TICKS;
    assert_eq!(
        rig.position(),
        expected,
        "点在第 5 小节前 1/4 处必须落到第 5 小节起点（5 × 3840）"
    );
    assert_eq!(
        rig.ui().get_playhead_ticks(),
        i32::try_from(expected).expect("tick 在 i32 内"),
        "界面镜面必须回写引擎读数（`apply_transport` 是唯一写者）"
    );
    assert_eq!(
        rig.last_command(),
        Some(TransportCommand::SeekTicks(expected)),
        "必须发既有的 `SeekTicks`，⛔ 不新造第二条定位通路"
    );
    assert_eq!(
        rig.journal_len(),
        journal_before + 1,
        "一次按下 = 一条走带命令（⛔ 不在拖动 / 释放里重复发）"
    );
}

/// ④：点落在视口框体内的 2.25/16 处 ⇒ 照样落到第 2 小节起点（框体 ⛔ 不吞轻点）。
#[test]
fn clicking_inside_the_viewport_lens_still_seeks() {
    let mut rig = Rig::new();
    // 默认缩放 30 下框体占画布左段约 37% ⇒ 2.25/16 处落在框体内。
    rig.click_canvas_fraction(2.25 / 16.0);
    assert_eq!(
        rig.position(),
        2 * BAR_TICKS,
        "点在视口框内的第 2 小节必须照样跳转（轻点 < 3 px ⇒ 跳转，拖动才滚动）"
    );
}

/// ⑤：点两缘 ⇒ 夹紧（最右列 ⇒ 全曲末端 16 × 3840；最左列 ⇒ 0）。
#[test]
fn clicking_canvas_edges_clamps_to_the_span() {
    let mut rig = Rig::new();
    rig.click_canvas_fraction(0.999);
    assert_eq!(
        rig.position(),
        16 * BAR_TICKS,
        "点在画布最右列必须夹到全曲末端（16 × 3840，与标尺判据 ④ 同一右缘值），⛔ 不送出全曲之外"
    );
    rig.click_canvas_fraction(0.001);
    assert_eq!(rig.position(), 0, "点在画布最左列必须回到 0");
}

/// ⑥：视口框几何 = 滚动 / 缩放公式（30 与 60 两档，误差 ±2 px）。
#[test]
fn viewport_lens_matches_scroll_and_zoom() {
    let mut rig = Rig::new();
    let canvas = rig.bounds(CANVAS);
    let lens = rig.bounds(LENS);
    // 默认：滚动 0 ⇒ 框左缘 = 画布左缘；框宽 = 画布宽 × 30 / 61440 × 752…直接按公式算。
    let span_ticks = f64::from(rig.ui().get_song_overview_span_ticks());
    #[allow(clippy::cast_precision_loss)]
    let tpp = host::roll_ticks_per_pixel(rig.ui()) as f64;
    let expected_width = f64::from(canvas.width) * tpp / span_ticks * f64::from(canvas.width);
    assert!(
        (f64::from(lens.width) - expected_width).abs() <= 2.0,
        "默认缩放下框宽必须是公式值 {expected_width:.1} px（实得 {} px）",
        lens.width
    );
    assert!(
        (f64::from(lens.x) - f64::from(canvas.x)).abs() <= 2.0,
        "滚动为 0 时框左缘 = 画布左缘（实得左缘 {}，画布左缘 {}）",
        lens.x,
        canvas.x
    );

    rig.set_zoom(60);
    let lens = rig.bounds(LENS);
    #[allow(clippy::cast_precision_loss)]
    let tpp = host::roll_ticks_per_pixel(rig.ui()) as f64;
    let expected_width = f64::from(canvas.width) * tpp / span_ticks * f64::from(canvas.width);
    assert!(
        (f64::from(lens.width) - expected_width).abs() <= 2.0,
        "缩放到 60 后框宽必须是公式值 {expected_width:.1} px（实得 {} px）",
        lens.width
    );
    assert_eq!(
        host::roll_ticks_per_pixel(rig.ui()),
        60,
        "缩放属性本身必须是 60"
    );
}

/// ⑦：拖框体向右 100 px ⇒ `roll-scroll-x` +272（±3），走带位置一位不动。
///
/// 换算：100 画布 px = 100/752 × 61440 = 8170 tick；÷ 30 tick/px = 272 px。
/// 框体本身在画布上只移 100 px（它是缩略图），编排视图滚的是等价的 272 px。
#[test]
fn dragging_the_lens_body_scrolls_the_arrangement() {
    let mut rig = Rig::new();
    let lens = rig.bounds(LENS);
    let start_x = f64::from(lens.x) + f64::from(lens.width) / 2.0;
    let start_y = f64::from(lens.y) + f64::from(lens.height) / 2.0;
    let scroll_before = rig.ui().get_roll_scroll_x();
    assert_eq!(scroll_before, 0.0, "前提：初始滚动为 0");

    rig.port
        .dispatch_pointer_down(
            LENS,
            f64::from(lens.width) / 2.0,
            f64::from(lens.height) / 2.0,
            PointerButton::Left,
        )
        .expect("框体必须可按下");
    // 移动事件的坐标是**窗口绝对坐标**（测试口的 `dispatch_pointer_move` 口径）。
    for step in 1..=4 {
        let fraction = f64::from(step) / 4.0;
        rig.port
            .dispatch_pointer_move(start_x + 100.0 * fraction, start_y)
            .expect("拖动必须送达");
    }
    rig.port
        .dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    let scroll_after = rig.ui().get_roll_scroll_x();
    assert!(
        (scroll_after - 272.3).abs() <= 3.0,
        "框体向右拖 100 px ⇒ 滚动必须是约 272 px（100/752×61440÷30，实得 {scroll_after} px）"
    );
    assert_eq!(
        rig.position(),
        0,
        "拖动框体 ⛔ 不跳转：走带位置必须仍是 0（位移 ≥ 3 px ⇒ 只滚动）"
    );
    rig.refresh();
    let lens_after = rig.bounds(LENS);
    assert!(
        (f64::from(lens_after.x) - f64::from(lens.x) - 100.0).abs() <= 2.0,
        "框体本身必须跟着向右移 100 px（实得左缘 {}，原左缘 {}）",
        lens_after.x,
        lens.x
    );
}

/// ⑧：拖框右缘向右 40 px ⇒ 缩放 30 → 34（±1），第 5 条小节线列位跟着变。
#[test]
fn dragging_the_lens_right_edge_zooms_and_reprojects() {
    let mut rig = Rig::new();
    let bar_before = rig.bar_line_px(5);
    let lens = rig.bounds(LENS);
    // 右缘把手：局部 (3, 41)（把手宽 6、高 82）。
    let start_x = f64::from(lens.x) + f64::from(lens.width) - 3.0;
    let start_y = f64::from(lens.y) + 41.0;
    rig.port
        .dispatch_pointer_down(RESIZE, 3.0, 41.0, PointerButton::Left)
        .expect("右缘把手必须可按下");
    for step in 1..=4 {
        let fraction = f64::from(step) / 4.0;
        rig.port
            .dispatch_pointer_move(start_x + 40.0 * fraction, start_y)
            .expect("拖动必须送达");
    }
    rig.port
        .dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    // 新框宽分数 ≈ (276 − 3 + 40) / 752 ⇒ 新视口宽 tick ≈ 25817 ⇒ tpp ≈ 34。
    let tpp = host::roll_ticks_per_pixel(rig.ui());
    assert!(
        tpp.abs_diff(34) <= 1,
        "右缘向右拖 40 px ⇒ 缩放应从 30 变到约 34 tick/px（实得 {tpp}）"
    );
    rig.refresh();
    let bar_after = rig.bar_line_px(5);
    assert!(
        (bar_after - bar_before).abs() > 1.0,
        "缩放后编排网格必须重投影：第 5 条小节线从 {bar_before:.1} 列移到 {bar_after:.1} 列"
    );
}

/// ⑨：点 `[能量]` 胶囊 ⇒ `show-energy` 翻 false、树上 `checked` 翻 false；再点 ⇒ 恢复。
#[test]
fn clicking_the_energy_pill_hides_and_restores_the_layer() {
    let mut rig = Rig::new();
    assert!(
        rig.ui().get_song_overview_show_energy(),
        "前提：能量层初始可见"
    );
    rig.click_center(ENERGY_PILL);
    assert!(
        !rig.ui().get_song_overview_show_energy(),
        "点一次 `[能量]` ⇒ 能量层必须隐藏（状态由 Rust 翻，界面只上报下标）"
    );
    rig.refresh();
    assert_eq!(
        rig.port
            .tree()
            .find_by_id(ENERGY_PILL)
            .expect("胶囊必须在树里")
            .checked,
        Some(false),
        "树上 `[能量]` 的 `checked` 必须跟着变 false（屏读器与断言同源）"
    );
    rig.click_center(ENERGY_PILL);
    assert!(
        rig.ui().get_song_overview_show_energy(),
        "再点一次 ⇒ 能量层必须恢复可见"
    );
}

/// ⑩：双击 `[情绪]` 胶囊 ⇒ 只剩情绪层可见；再双击 ⇒ 四层全可见。
#[test]
fn double_clicking_a_pill_solos_then_restores_all_layers() {
    let mut rig = Rig::new();
    rig.double_click_center(TENSION_PILL);
    assert!(
        rig.ui().get_song_overview_show_tension(),
        "双击 `[情绪]` ⇒ 情绪层必须可见（独显目标）"
    );
    assert!(
        !rig.ui().get_song_overview_show_energy()
            && !rig.ui().get_song_overview_show_bands()
            && !rig.ui().get_song_overview_show_density(),
        "双击 `[情绪]` ⇒ 其余三层必须全隐藏（独显）"
    );
    rig.double_click_center(TENSION_PILL);
    assert!(
        rig.ui().get_song_overview_show_tension()
            && rig.ui().get_song_overview_show_energy()
            && rig.ui().get_song_overview_show_bands()
            && rig.ui().get_song_overview_show_density(),
        "已独显时再双击 ⇒ 四层必须全恢复可见"
    );
}

/// ⑪：无按下移动到第 7 小节上 ⇒ 悬停读数出现（含小节号、LUFS、张力、前两频段）。
#[test]
fn hovering_the_canvas_shows_the_bar_readout() {
    let mut rig = Rig::new();
    assert!(
        !rig.ui().get_song_overview_hover_visible(),
        "前提：悬停前读数不可见"
    );
    let canvas = rig.bounds(CANVAS);
    // 第 7 小节中部：分数 6.5/16（画布外区域，不在视口框上 —— 框只盖左段）。
    let x = f64::from(canvas.x) + f64::from(canvas.width) * (6.5 / 16.0);
    let y = f64::from(canvas.y) + 41.0;
    rig.port
        .dispatch_pointer_move(x, y)
        .expect("悬停移动必须送达");
    assert!(
        rig.ui().get_song_overview_hover_visible(),
        "无按下移动到画布上 ⇒ 悬停读数必须出现（`moved` 在无按下时也要触发）"
    );
    let text = rig.ui().get_song_overview_hover_text().to_string();
    assert!(
        text.contains("第 7 小节"),
        "读数必须含当前小节号（实得 \"{text}\"）"
    );
    assert!(
        text.contains("LUFS"),
        "读数必须含短时 LUFS（实得 \"{text}\"）"
    );
    assert!(
        text.contains("张力"),
        "读数必须含张力分值（实得 \"{text}\"）"
    );
    assert!(
        text.contains("突出"),
        "读数必须含最突出的前两个频段（实得 \"{text}\"）"
    );
    // 提示框随悬停出现 ⇒ 此时才进控件树（初始不可见时不进树，见判据 ①）。
    rig.refresh();
    assert!(
        rig.port.tree().contains("song-overview-hover-readout"),
        "悬停后提示框必须出现在控件树里"
    );
}

/// ⑫：点折叠键 ⇒ 面板高 22；再点 ⇒ 回 100，画布在场。
#[test]
fn collapse_button_folds_the_panel_to_22px_and_back() {
    let mut rig = Rig::new();
    rig.click_center(COLLAPSE);
    rig.refresh();
    let panel = rig.bounds(PANEL);
    assert_eq!(
        panel.height, 22,
        "点折叠键 ⇒ 面板必须缩到 22 px（只剩底部条）"
    );
    // 折叠态留一张截图供肉眼验收（折叠后画布区是否真的让位给底部条）。
    let frame = rig.port.window().capture().expect("折叠态必须能截图");
    let path = write_artifact(
        "song-overview-panel-collapsed",
        &Rig::crop_panel(&frame, 0, 772, 1440, 128),
    )
    .expect("折叠态截图必须能落盘");
    println!("折叠态截图: {}", path.display());
    rig.click_center(COLLAPSE);
    rig.refresh();
    let panel = rig.bounds(PANEL);
    assert_eq!(panel.height, PANEL_HEIGHT, "再点 ⇒ 面板必须恢复 104 px");
    assert!(rig.port.tree().contains(CANVAS), "恢复后画布必须回到树里");
}

/// ⑬：视觉验收 1:1 + 3x —— 截图非黑、面板区多色，三张 PNG 落盘供肉眼验收。
///
/// 3x 的做法：测试口没有缩放因子接口（`Tier1Window` 只有 `resize`，逻辑像素
/// 尺寸不变 ⇒ 单纯放大窗口不会放大面板），因此 3x 取 1:1 面板区的最近邻放大
/// （像素一位不动，只放大看）；另截一张 4320×2700 大窗口，证面板在重布局后
/// 仍在正确位置（画布随窗口变宽、视口框重算）。
#[test]
fn visual_acceptance_1x_and_3x() {
    let mut rig = Rig::new();
    // 先悬停到第 7 小节上，让提示框也进 1:1 截图（验收读数外观）。
    let canvas = rig.bounds(CANVAS);
    rig.port
        .dispatch_pointer_move(
            f64::from(canvas.x) + f64::from(canvas.width) * (6.5 / 16.0),
            f64::from(canvas.y) + 41.0,
        )
        .expect("悬停移动必须送达");

    // 1:1：面板区 = 窗口底部 128 行（面板 104 + 状态栏 24）。
    let frame = rig.port.window().capture().expect("1:1 截图必须能取");
    assert_eq!(frame.width(), WINDOW.width, "1:1 帧宽必须是 1440");
    assert_eq!(frame.height(), WINDOW.height, "1:1 帧高必须是 900");
    assert!(!frame.is_all_black(), "1:1 帧 ⛔ 不能全黑");
    let panel_1x = Rig::crop_panel(&frame, 0, 772, 1440, 128);
    assert!(
        panel_1x.distinct_color_count() > 16,
        "1:1 面板区必须多色（热力图 + 能量 + 张力 + 胶囊，实得 {} 色）",
        panel_1x.distinct_color_count()
    );
    let path_1x =
        write_artifact("song-overview-panel-1x", &panel_1x).expect("1:1 面板截图必须能落盘");
    println!("1:1 面板截图: {}", path_1x.display());

    // 3x：1:1 面板区最近邻放大三倍（像素一位不动，只放大看）。
    let panel_3x = Rig::upscale_3x(&panel_1x);
    assert_eq!(panel_3x.width(), 4320, "3x 面板宽必须是 4320");
    assert_eq!(panel_3x.height(), 384, "3x 面板高必须是 384");
    assert!(
        panel_3x.distinct_color_count() > 16,
        "3x 面板区必须多色（实得 {} 色）",
        panel_3x.distinct_color_count()
    );
    let path_3x =
        write_artifact("song-overview-panel-3x", &panel_3x).expect("3x 面板截图必须能落盘");
    println!("3x 面板截图: {}", path_3x.display());

    // 大窗口重布局：窗口放大到 4320×2700 再截，面板必须仍在底部正确位置。
    rig.port
        .window()
        .resize(Size::new(4320, 2700))
        .expect("窗口必须能放大到 4320×2700");
    let frame_big = rig.port.window().capture().expect("大窗口截图必须能取");
    assert_eq!(frame_big.width(), 4320, "大窗口帧宽必须是 4320");
    assert_eq!(frame_big.height(), 2700, "大窗口帧高必须是 2700");
    let panel_big = Rig::crop_panel(&frame_big, 0, 2700 - 384, 4320, 384);
    assert!(
        panel_big.distinct_color_count() > 16,
        "大窗口面板区必须多色（实得 {} 色）",
        panel_big.distinct_color_count()
    );
    let path_big = write_artifact("song-overview-panel-big-window", &panel_big)
        .expect("大窗口面板截图必须能落盘");
    println!("大窗口面板截图: {}", path_big.display());
}
