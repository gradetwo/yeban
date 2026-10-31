//! `U-05`：**章节卡片 ⇒ 走带定位到该段起点** —— 真实指针 + 无障碍默认动作。
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
//! ⛔ 不碰工程、不碰撤销栈：定位是会话运行态（`MODEL-ISO-001` 第二层）。
//!
//! ## 判据
//!
//! | # | 证的事 |
//! | :--- | :--- |
//! | ① | 真实指针点卡片中心 ⇒ 引擎位置 == 该段 `start_tick`，动作日志**恰加一条** `SeekTicks` |
//! | ② | 无障碍默认动作（不点像素）⇒ 同一条落点（与 `TouchArea` 的 down 同一个回调） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]`
//! 只建**一个** `LivePort`（与 `tests/arrangement_ruler_seek.rs` 同款）。

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
}

/// ①：真实指针点章节卡片中心 ⇒ 引擎到该段起点，且动作日志恰加一条 `SeekTicks`。
#[test]
fn clicking_a_section_card_seeks_to_its_start() {
    let mut rig = Rig::new();
    let (index, target) = rig.non_zero_section();
    let card = format!("section-{index}-card");
    let bounds = rig
        .port
        .tree()
        .find_by_id(&card)
        .unwrap_or_else(|| panic!("控件树里必须有 {card}"))
        .bounds
        .expect("章节卡片必须有几何才谈得上点击");
    let journal_before = rig.journal_len();

    rig.port
        .dispatch_pointer_down(
            &card,
            f64::from(bounds.width) / 2.0,
            f64::from(bounds.height) / 2.0,
            PointerButton::Left,
        )
        .unwrap_or_else(|error| panic!("章节卡片必须可按下: {error:?}"));
    rig.port
        .dispatch_pointer_up(PointerButton::Left)
        .expect("释放必须送达");

    assert_eq!(
        rig.position(),
        target,
        "点章节卡片 ⇒ 走带必须到该段起点（第 {index} 段 = tick {target}）"
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

/// ②：无障碍默认动作（不点像素）⇒ 与指针 down 同一个落点。
#[test]
fn the_section_card_default_action_seeks_too() {
    let rig = Rig::new();
    let (index, target) = rig.non_zero_section();
    let card = format!("section-{index}-card");
    let handle =
        find_by_accessible_id(rig.ui(), &card).unwrap_or_else(|| panic!("{card} 必须可寻址"));
    handle.invoke_accessible_default_action();
    assert_eq!(
        rig.position(),
        target,
        "屏读器 / 自动化不需要点像素坐标就能跳段"
    );
}
