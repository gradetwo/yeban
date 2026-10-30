//! ⭐⭐ 任务单 `#283`：**场景按钮真的激发场景**（引擎能激发、而用户按不到 ⇒ 接通）。
//!
//! ## 这条判据要证的句子
//!
//! 用户**真的点了** `.slint` 里的 `scene-launch-0-button`（走 Tier-1 无头夹具的
//! **真实指针按下 / 释放**，⛔ 不是直接 `invoke_scene_launch`），引擎的
//! `scene_launches_applied` 因此 **≥ 1**，且生效之后 `has_pending_scene_launch()` 回 `false`。
//!
//! ## 判据表（每条答一个不同的问题；⛔ 不靠真声卡、⛔ 不读环境变量）
//!
//! | 判据 | 答的问题 | 证什么 / **不**证什么 |
//! | :--- | :--- | :--- |
//! | [`a_real_click_on_the_scene_button_launches_it_in_the_engine`] | 产品路径的点击真的到引擎了吗 | 证 `.slint` 的 `TouchArea.clicked` → 宿主回调 → `queue_scene_launch` → 到点生效；⛔ 不证听感 |
//! | [`without_a_click_nothing_launches`] | "≥1" 会不会来自别处 | **正对照**：同一夹具不点 ⇒ `scene_launches_applied == 0`、没有 pending |
//! | [`the_off_quantisation_from_the_projection_is_honoured`] | 产品路径用的是工程配置的量化档吗 | 把投影的 `launch_quantization` 改成 `Off` ⇒ **一个量子内**生效（⛔ 不是写死 `Bar`） |
//! | [`the_current_scene_is_a_session_readout_not_a_projection_product`] | "当前场景"读数的归属对不对 | 投影恒 `None`；点击后变了；再投影 ⇒ 回到 `None`（换工程不带着旧场景） |
//! | [`the_scene_button_is_really_clickable_and_the_actionless_list_is_honest`] | 场景按钮移出清单有没有说谎 | `.slint` 里那一个按钮**同时**有 `clicked` 与同落点的 `accessible-action-default`；清单条目数与声明长度一致且不再含 `scene-launch-` |
//!
//! ## 无头可判（判据 ⑧）
//!
//! 全程走 `yeban-ui-test-port` 的 Tier-1 软件平台 ＋ 生产窗口构造入口
//! （`host::build_main_window`）＋ 生产接线 `host::wire_scene_launch`。
//! 指针事件由 `UiTestPort::dispatch_pointer_down/up` 注入 —— 那是**真的**进 Slint 的
//! `WindowEvent`，⛔ 不是"绕过界面直接调回调"。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证真声卡上的听感；⛔ 不证设备腿的事件环在**真** cpal 回调里被消费（那需要硬件，
//! 本文件只证到"事件已经写进既有 SPSC"这一层由 `yeban-engine` 自己的判据覆盖）；
//! ⛔ 不证绘制时间（那要 GPU 档，本机纪律不允许）。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_model::LaunchQuantization;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 窗口尺寸（与其余 Tier-1 判据同款）。
const WINDOW: Size = Size::new(1440, 900);
/// 一小节 = 120 bpm / 48 kHz / 4-4 ⇒ 96 000 帧；运行时量子 128 帧 ⇒ 750 块。
/// 判据给两倍预算：**生效必须在这段时间里发生**（否则读数恒 0 也能"通过"）。
const QUANTA_FOR_TWO_BARS: usize = 1_500;

/// 一条 Tier-1 活窗口 + 真的引擎 + 生产场景接线。
///
/// `quantization` 写进**投影**（`YebanProjectV1::transport.launch_quantization`）——
/// 产品回调读的是投影，⛔ 不是装配期的工程快照。
struct Fixture {
    port: LivePort<yeban_app::ui::MainWindow>,
    engine: Rc<RefCell<EngineHost>>,
    view: Rc<RefCell<ViewState>>,
    scene_name: String,
    base_bpm: f64,
    scene_bpm: f64,
}

impl Fixture {
    fn new(quantization: LaunchQuantization) -> Self {
        let mut project = yeban_model::samples::filled_project();
        project.transport.launch_quantization = quantization;
        // 场景 0 的 tempo 覆盖必须与工程速度**明显不同** —— 否则"到点生效"读不出来。
        let base_bpm = project.bpm;
        let scene_id = *project
            .scenes
            .keys()
            .next()
            .expect("夹具必须至少有一个场景");
        project
            .scenes
            .get_mut(&scene_id)
            .expect("场景身份来自同一张表")
            .tempo = Some(base_bpm + 17.0);
        let view = ViewState::from_project(&project).expect("filled_project 必须能投影");
        assert_eq!(
            view.launch_quantization, quantization,
            "投影必须如实带出工程配置的量化档"
        );
        let scene = DemoScene::from_view(&view);
        let first = view.scenes.first().expect("夹具必须至少有一个场景");
        let scene_name = first.name.clone();
        let scene_bpm = first.tempo.expect("夹具刚写过场景 tempo 覆盖");
        let mut engine = EngineHost::new();
        engine.reload(&project, 0).expect("引擎必须能建起来");
        let engine = Rc::new(RefCell::new(engine));
        let view = Rc::new(RefCell::new(view));
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window(&view.borrow(), &scene)
        })
        .expect("Tier-1 平台 + 生产窗口构造入口");
        // 生产接线：场景按钮 → 引擎（**同一个** `Rc<RefCell<EngineHost>>` 与视图）。
        host::wire_scene_launch(port.ui(), Rc::clone(&engine), Rc::clone(&view));
        // 判据要点的是场景按钮 ⇒ 先切到 Session 视图（默认是编曲视图）。
        port.ui().set_arrangement_view(false);
        Self {
            port,
            engine,
            view,
            scene_name,
            base_bpm,
            scene_bpm,
        }
    }

    /// 点击 `scene-launch-0-button`：**真实的**指针按下 + 释放（先重抓几何）。
    fn click_first_scene(&mut self) {
        self.port
            .refresh_tree(None)
            .expect("切到 Session 之后必须能重抓控件树");
        let bounds = self
            .port
            .tree()
            .find_by_id("scene-launch-0-button")
            .expect("场景按钮必须在 Session 视图的控件树里")
            .bounds
            .expect("场景按钮必须有几何");
        assert!(
            bounds.width > 0 && bounds.height > 0,
            "场景按钮的几何必须非零: {bounds:?}"
        );
        // ⭐ 回执行是同一次交付的一部分：它必须在运行时树里（否则"没有引擎"那一路
        // 用户什么原因都看不到）。
        let status = self
            .port
            .tree()
            .find_by_id("session-scene-launch-status")
            .expect("场景启动回执必须在 Session 视图的控件树里")
            .bounds
            .expect("回执行必须有几何");
        println!("[scene-launch] 回执行几何: {status:?}");
        self.port
            .dispatch_pointer_down("scene-launch-0-button", 2.0, 2.0, PointerButton::Left)
            .expect("指针按下必须落到活窗口");
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("指针释放必须落到活窗口");
    }

    /// 推量子直到引擎真的生效（或预算耗尽）⇒ 返回 `(推了多少块, 推完之后的引擎速度)`。
    ///
    /// ⚠ 速度读数必须从**引擎的** `Transport` 取（`EngineRuntime::transport().bpm()`）；
    /// 控制侧的 `TransportReading` 镜面**没有** bpm 这一格（它只镜像状态与位置），
    /// 因此"tempo 真的变成了场景的覆盖值"只能读运行时本身。
    fn pump_until_applied(&self, budget: usize) -> (usize, f64) {
        let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
        let mut engine = self.engine.borrow_mut();
        let runtime = engine
            .runtime_for_criteria()
            .expect("控制驱动档：运行时归宿主所有");
        let mut used = budget;
        for step in 1..=budget {
            runtime.process_quantum(&mut block, 2);
            if runtime.stats().scene_launches_applied > 0 {
                used = step;
                break;
            }
        }
        (used, runtime.transport().bpm())
    }
}

/// ⭐ 判据 ①：**产品路径**（真的点 `.slint` 的按钮）⇒ 引擎 `scene_launches_applied ≥ 1`
/// ＋ 生效之后 `has_pending_scene_launch()` 为 **false**。
///
/// 独立变色：把 `session_view.slint` 的 `TouchArea.clicked` 摘掉 ⇒ 本判据的 applied 恒 0，
/// 红在 `left: 0 / right: 1`。
#[test]
fn a_real_click_on_the_scene_button_launches_it_in_the_engine() {
    let mut fixture = Fixture::new(LaunchQuantization::Bar);
    assert_eq!(fixture.engine.borrow().scene_launches_applied(), Some(0));
    assert_eq!(
        fixture.engine.borrow().has_pending_scene_launch(),
        Some(false)
    );
    assert!(fixture.view.borrow().current_scene.is_none());

    fixture.click_first_scene();

    // 点击之后、量子边界之前：请求已武装，但**还没生效**（量化档不是 Off）。
    assert_eq!(
        fixture.engine.borrow().has_pending_scene_launch(),
        Some(true),
        "Bar 档点击后必须留下一个待生效项"
    );
    assert_eq!(
        fixture.engine.borrow().scene_launches_applied(),
        Some(0),
        "Bar 档⛔ 不得在点击那一刻就改速度"
    );

    let (used, applied_bpm) = fixture.pump_until_applied(QUANTA_FOR_TWO_BARS);
    assert!(
        used < QUANTA_FOR_TWO_BARS,
        "一小节（750 块）内必须生效，实测推了 {used} 块还没生效"
    );
    {
        let engine = fixture.engine.borrow();
        assert_eq!(engine.scene_launches_applied(), Some(1), "⭐ 恰好一次");
        assert_eq!(
            engine.has_pending_scene_launch(),
            Some(false),
            "⭐ 生效之后状态机必须回到 None"
        );
    }
    assert!(
        (applied_bpm - fixture.scene_bpm).abs() < 1e-9,
        "到点后 tempo 必须变成场景的 {}（实测 {applied_bpm}）",
        fixture.scene_bpm
    );
    assert!(
        (applied_bpm - fixture.base_bpm).abs() > 1.0,
        "夹具前提：场景 tempo 必须与工程速度可区分"
    );
    println!(
        "[scene-launch] 点击 ⇒ 读数：base={:.3} → scene={:.3}（实测 {applied_bpm:.3}），\
         生效块={used}，applied={:?}，pending={:?}",
        fixture.base_bpm,
        fixture.scene_bpm,
        fixture.engine.borrow().scene_launches_applied(),
        fixture.engine.borrow().has_pending_scene_launch(),
    );
}

/// ⭐ 判据 ②（**正对照**）：同一夹具、**不点击** ⇒ `scene_launches_applied == 0`。
///
/// 没有它，"≥ 1" 可能来自夹具装配（例如 `reload` 自己发过一次启动）。
#[test]
fn without_a_click_nothing_launches() {
    let fixture = Fixture::new(LaunchQuantization::Bar);
    assert_eq!(
        fixture.engine.borrow().scene_launches_applied(),
        Some(0),
        "⛔ 没有点击就不许有已生效的启动"
    );
    let (used, bpm) = fixture.pump_until_applied(QUANTA_FOR_TWO_BARS);
    assert_eq!(used, QUANTA_FOR_TWO_BARS, "预算耗尽（⛔ 从未生效）");
    assert_eq!(fixture.engine.borrow().scene_launches_applied(), Some(0));
    assert_eq!(
        fixture.engine.borrow().has_pending_scene_launch(),
        Some(false)
    );
    assert!((bpm - fixture.base_bpm).abs() < 1e-9, "⛔ 速度一位不变");
}

/// ⭐ 判据 ③：**投影里的量化档真的被产品路径用了**（`Off` ⇒ 一个量子内生效）。
///
/// 独立变色：把 `wire_scene_launch` 里的 `quantization: view.launch_quantization` 写死成
/// `Bar` ⇒ 本判据红在"第一块必须生效，实测 750 块"。
#[test]
fn the_off_quantisation_from_the_projection_is_honoured() {
    let mut fixture = Fixture::new(LaunchQuantization::Off);
    fixture.click_first_scene();
    let (used, bpm) = fixture.pump_until_applied(2);
    assert_eq!(used, 1, "⭐ Off ⇒ 第一个量子内必须生效");
    assert_eq!(fixture.engine.borrow().scene_launches_applied(), Some(1));
    assert_eq!(
        fixture.engine.borrow().has_pending_scene_launch(),
        Some(false)
    );
    assert!(
        (bpm - fixture.scene_bpm).abs() < 1e-9,
        "Off ⇒ 立即换成场景 tempo（实测 {bpm}）"
    );
}

/// ⭐ 判据 ④：**"当前场景"是会话读数**，⛔ 不是投影产物。
///
/// 前后值逐字给出：点击前 `None`，点击后 `Some(场景 0)`；完整重投影之后回到 `None`
/// （换工程 ⇒ 不带着旧场景），同时界面属性 `current-scene-index` 也回到 `-1`。
#[test]
fn the_current_scene_is_a_session_readout_not_a_projection_product() {
    let mut fixture = Fixture::new(LaunchQuantization::Bar);
    assert!(
        ViewState::from_project(&yeban_model::samples::filled_project())
            .expect("投影")
            .current_scene
            .is_none(),
        "⛔ 投影不得生产「当前场景」"
    );
    assert_eq!(fixture.port.ui().get_current_scene_index(), -1);
    assert_eq!(fixture.port.ui().get_current_scene_name().as_str(), "");

    fixture.click_first_scene();

    let clicked = fixture
        .view
        .borrow()
        .current_scene
        .clone()
        .expect("点击之后 ViewState 里必须有当前场景");
    assert_eq!(clicked.index, 0);
    assert_eq!(clicked.name, fixture.scene_name);
    assert_eq!(
        fixture.port.ui().get_current_scene_index(),
        0,
        "界面属性与 ViewState 同源"
    );
    assert_eq!(
        fixture.port.ui().get_current_scene_name().as_str(),
        fixture.scene_name.as_str()
    );
    println!(
        "[scene-launch] 当前场景：前=None → 后=Some({} #{})",
        clicked.name, clicked.index
    );

    // 再走一次完整重投影（`from_project` 的投影 + 同一个注入点）⇒ 读数回到"没有"。
    let fresh = ViewState::from_project(&yeban_model::samples::filled_project()).expect("投影");
    host::apply_view(fixture.port.ui(), &fresh, WINDOW.width as f32, 0.0);
    assert_eq!(
        fixture.port.ui().get_current_scene_index(),
        -1,
        "完整重投影必须把当前场景清回 -1（换工程不带着旧场景）"
    );
    assert_eq!(fixture.port.ui().get_current_scene_name().as_str(), "");
}

/// ⭐ 判据 ⑤：**场景按钮移出清单没有说谎** —— 那一个按钮**同时**有 `clicked` 与同落点的
/// `accessible-action-default`，且 `KNOWN_ACTIONLESS_CONTROLS` 的条目数与声明长度一致。
/// ⚠ 条数**不写死**：清单是随每个"修好一个 ⇒ 移出一条"收缩的活文档（规则见
/// `elements.rs` 的判据注释），写死 N 会让每一次合法的移出都变成这里的红。
///
/// 这条是文本层判据（与 `elements.rs` 的
/// `controls_without_any_click_source_are_exactly_the_known_empty_ones` 同一份事实的
/// **正面**一侧）：那条证"清单外的都没有"，这条证"移出去的那一个真的有动作"。
#[test]
fn the_scene_button_is_really_clickable_and_the_actionless_list_is_honest() {
    let source = include_str!("../ui/workspace/session_view.slint");
    let launch = source
        .split("accessible-id: \"scene-launch-\" + scene_index + \"-button\"")
        .nth(1)
        .expect("`session_view.slint` 里必须有场景按钮");
    let block = launch
        .split("\n    }")
        .next()
        .expect("场景按钮的元素块必须能切出来");
    assert!(
        block.contains("root.scene-launch(scene_index)"),
        "场景按钮必须真的把点击转给宿主: {block}"
    );
    let clicks = block.matches("clicked =>").count();
    let actions = block.matches("accessible-action-default =>").count();
    assert_eq!(clicks, 1, "场景按钮必须恰好一个 `clicked` 落点");
    assert_eq!(actions, 1, "场景按钮必须恰好一个无障碍动作落点");
    assert_eq!(
        block.matches("root.scene-launch(scene_index)").count(),
        2,
        "`clicked` 与 `accessible-action-default` 必须**同一落点**"
    );

    // 清单的真实项数（源码层读数；`elements.rs` 的判据把它与运行时集合逐条对账）。
    let elements = include_str!("../src/elements.rs");
    let declaration = "const KNOWN_ACTIONLESS_CONTROLS: [&str;";
    let rest = elements
        .split(declaration)
        .nth(1)
        .expect("清单声明必须存在");
    let (len_text, rest) = rest
        .split_once("] = [")
        .expect("清单必须形如 `[&str; N] = [`");
    let declared: usize = len_text.trim().parse().expect("清单声明的长度必须是整数");
    let body = rest.split("];").next().expect("清单必须有结尾");
    let entries = body
        .lines()
        .filter(|line| line.trim_start().starts_with('"'))
        .count();
    assert_eq!(
        entries, declared,
        "KNOWN_ACTIONLESS_CONTROLS 的条目数必须与声明长度一致"
    );
    assert!(
        !body.contains("scene-launch-"),
        "场景按钮必须**真的**从清单里移出（⛔ 只删表行不接线）"
    );
}
