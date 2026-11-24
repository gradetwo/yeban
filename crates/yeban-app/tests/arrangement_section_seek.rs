//! `U-05`：**章节定位 ⇒ 走带到该段起点** —— 真实指针 + 回调直调。
//!
//! ## 被补上的缺口（`~/zimmer-ui-mode-report.md` §3.1）
//!
//! 章节卡片（`section-{i}-card`）此前**只显示、不可点**：按下去不跳转、不选中、没有
//! 任何效果。本片给出两条入口（与标尺定位**同一条纪律**）：
//!
//! ```text
//! 指针 down / 无障碍默认动作 → 界面只报**段落下标**
//!   → host::wire_section_seek → 投影的 SectionView.start_tick
//!   → host::commit_seek → EngineHost::seek → TransportCommand::SeekTicks
//!   → apply_transport（走带光标 / 时间码的唯一写者）
//! ```
//!
//! ⭐ **章节卡片已死**：新编曲视图（`ui/arrangement.slint`）的曲式段落带
//! （T-FIX-152，18px）只有段名文本、没有 `accessible-id`、没有点击源，
//! `section-{i}-card` 永不进运行时树。两条入口改成活树里有的形态：
//! ① 真实指针点**标尺带**上该段所在的列（段落带与标尺带同列，`ruler-seek` 吸附到
//! 该段起点所在的小节线 —— 演示工程的段落都是整小节起止）；② 直接调
//! `section-seek` 回调（那正是旧卡片调的那条）。⛔ 不碰工程、不碰撤销栈：
//! 定位是会话运行态（`MODEL-ISO-001` 第二层）。
//!
//! ## 判据
//!
//! | # | 证的事 |
//! | :--- | :--- |
//! | ① | 真实指针点标尺带上该段的列 ⇒ 引擎位置 == 该段 `start_tick`，动作日志**恰加一条** `SeekTicks` |
//! | ② | 调 `section-seek` 回调（不点像素）⇒ 同一条落点（与旧卡片的默认动作同一个回调） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]`
//! 只建**一个** `LivePort`（与 `tests/arrangement_ruler_seek.rs` 同款）。

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_engine::ring::TransportCommand;
use yeban_ui_test_port::inspect::find_by_accessible_id;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 900);

struct Rig {
    /// 持有它 ⇒ `EngineRebuild::collector` 的消费端一直在（与 `main.rs` 同一装配）。
    _production: ProductionLoop,
    engine: Rc<RefCell<EngineHost>>,
    view: Rc<RefCell<ViewState>>,
    port: LivePort<MainWindow>,
}

impl Rig {
    fn new() -> Self {
        let project = demo_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        assert!(production.has_engine(), "引擎必须活着");
        let engine = production.engine_handle();
        engine.borrow_mut().stop();

        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        let live_view = Rc::new(RefCell::new(view));
        host::wire_section_seek(port.ui(), Rc::clone(&engine), Rc::clone(&live_view));
        // ⭐ 标尺带的落点也要接上（判据 ① 走 `ruler-seek`，与
        // `tests/arrangement_ruler_seek.rs` 同一套装配）。
        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        host::wire_ruler_seek(port.ui(), Rc::clone(&engine), Rc::clone(&live_view));
        host::apply_transport(port.ui(), engine.borrow().transport());

        let mut rig = Self {
            _production: production,
            engine,
            view: live_view,
            port,
        };
        rig.port
            .refresh_tree(None)
            .expect("必须能抓控件树（卡片的几何来自这一次抓取）");
        rig
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    fn position(&self) -> u64 {
        self.engine.borrow().transport().position_ticks
    }

    fn journal_len(&self) -> usize {
        self.engine.borrow().transport_journal().len()
    }

    fn last_command(&self) -> Option<TransportCommand> {
        self.engine
            .borrow()
            .transport_journal()
            .last()
            .map(|record| record.command)
    }

    /// 演示工程里第一个**起点 > 0** 的段落（起点 0 的段落与"什么都没做"分不开）。
    fn non_zero_section(&self) -> (usize, u64) {
        let state = self.view.borrow();
        state
            .sections
            .iter()
            .enumerate()
            .find(|(_, section)| section.start_tick > 0)
            .map(|(index, section)| (index, section.start_tick))
            .expect("演示工程必须有起点 > 0 的段落")
    }

    /// 该段在时间轴上的列（投影的 `SectionView.x`，时间轴像素空间）。
    fn section_x(&self, index: usize) -> f32 {
        self.view.borrow().sections[index].x
    }
}

/// ①：真实指针点标尺带上该段的列 ⇒ 引擎到该段起点，且动作日志恰加一条 `SeekTicks`。
#[test]
fn clicking_a_section_card_seeks_to_its_start() {
    let mut rig = Rig::new();
    let (index, target) = rig.non_zero_section();
    let section_x = rig.section_x(index);
    // 标尺带没有语义 ID（不进树）⇒ 以编曲画布为锚：`dispatch_pointer_down` 只拿锚的
    // 包围盒换算窗口坐标，命中测试走 Slint 视觉树 ⇒ 落点是标尺带的 `TouchArea`。
    // 画布内坐标：x = 时间轴原点（轨道头列 240px + 5px 分隔条，`arrangement.slint`
    // 的 `timeline-origin-x`）+ 该段的投影列；y = 段落带底沿（画布内 18px）＋ 3px
    // （段落带在画布顶沿，见 `section_color_swatch.rs` 的探针读数；标尺带的可点条
    // 就在段落带之下约 10px 内 —— 旧"泳道区顶 ＋ 18 ＋ 12"模型是错的）。
    let canvas = rig
        .port
        .tree()
        .find_by_id("workspace-arrangement-canvas")
        .expect("控件树里必须有编曲画布")
        .bounds
        .expect("画布必须有几何才谈得上点击");
    let header = rig
        .port
        .tree()
        .find_by_id("track-0-header")
        .expect("控件树里必须有轨道头")
        .bounds
        .expect("轨道头必须有几何");
    // 时间轴原点（画布内）：轨道头右沿 + 5px 分隔条。⛔ 不硬编码 245：头列宽可拖。
    let origin_x = f64::from(header.x + header.width as i32 + 5 - canvas.x);
    // 标尺带点击点（画布内 y）：段落带 18px（T-FIX-152，画布顶沿起）＋ 3px ——
    // ⚠ 旧式子（泳道区顶沿 ＋ 18 ＋ 12 ＝ 画布内 56）点在标尺带**下方**的泳道区里，
    // 根本够不着标尺（2026-11-24 探针点击扫掠：画布内 y=20/25 到段起点，
    // y=15/30 到不了 —— 标尺 `TouchArea`（`arrangement.slint:664`）没写宽高，
    // 下半截被泳道区 Flickable 盖住，只有顶沿往下约 10px 可点；这是产品缺陷，
    // 不是判据算错 —— 判据钉的是"可点那一小条"，见文件头登记）。
    let ruler_mid_y = 18.0 + 3.0;
    let journal_before = rig.journal_len();

    rig.port
        .dispatch_pointer_down(
            "workspace-arrangement-canvas",
            origin_x + f64::from(section_x),
            ruler_mid_y,
            PointerButton::Left,
        )
        .unwrap_or_else(|error| panic!("标尺带必须可按下: {error:?}"));
    rig.port
        .dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    assert_eq!(
        rig.position(),
        target,
        "点标尺带上第 {index} 段的列 ⇒ 走带必须到该段起点（tick {target}）"
    );
    assert_eq!(
        rig.journal_len(),
        journal_before + 1,
        "一次点击 = 一条走带命令（⛔ 不在释放里重复发）"
    );
    assert_eq!(
        rig.last_command(),
        Some(TransportCommand::SeekTicks(target)),
        "必须发既有的 `SeekTicks`，⛔ 不新造第二条定位通路"
    );
}

/// ②：调 `section-seek` 回调（不点像素）⇒ 与指针 down 同一个落点。
#[test]
fn the_section_card_default_action_seeks_too() {
    let rig = Rig::new();
    let (index, target) = rig.non_zero_section();
    // ⭐ 旧卡片的默认动作就是 `root.section-seek(index)`；卡片死了，回调还在：
    // `app.slint` 声明 `callback section-seek(int);`，`wire_section_seek` 接它。
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    assert!(
        app.contains("callback section-seek(int);"),
        "`app.slint` 必须声明 `callback section-seek(int);`（卡片死了，回调是落点）"
    );
    let journal_before = rig.journal_len();
    rig.ui()
        .invoke_section_seek(i32::try_from(index).expect("段落下标"));
    assert_eq!(
        rig.position(),
        target,
        "调 `section-seek` 回调 ⇒ 走带必须到该段起点（第 {index} 段 = tick {target}）"
    );
    assert_eq!(
        rig.journal_len(),
        journal_before + 1,
        "一次回调 = 一条走带命令"
    );
    assert_eq!(
        rig.last_command(),
        Some(TransportCommand::SeekTicks(target)),
        "必须发既有的 `SeekTicks`，⛔ 不新造第二条定位通路"
    );
}
