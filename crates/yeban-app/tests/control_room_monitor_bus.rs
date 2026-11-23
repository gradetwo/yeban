//! T-FIX-191：控制室 MON A / MON B **真正切换监听总线**（活窗口判据）。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | D1 | 活窗口里调 `monitor-switch("alt")` ⇒ 引擎总线真变 B，且状态行报 `监听 alt`；切回 `"main"` ⇒ 总线回 A | 回调只读设备名写状态（修前行为） |
//! | D2 | 两台同工程引擎（设备回调路径）⇒ B 路增益 0.5 的那台每帧逐位 `== 基线 × 0.5`；切回 A 的那台与基线**逐位相同** | 总线级只在 UI 层 / 没进 `process_quantum` |
//! | D3 | B 路选中＋增益 0.5 时 `reload` ⇒ 换代后总线仍是 B、增益仍是 0.5（宿主把旧值拷进新运行时） | 换代静默归零 ⇒ 界面与引擎脱节 |
//! | D4 | 调 `monitor-switch("bogus")` ⇒ 引擎总线不动（仍是 A），不 panic | 未知名字静默切总线 / panic |
//!
//! 装配与 `tests/control_room_monitor.rs`（T-FIX-190）同款：Tier-1 活窗口
//! （Slint testing backend ＋ 生产同款 `wire_*` 装配，先旧回调、最后
//! `wire_aether_actions` 覆盖接管）。注意：`active-monitor` 只活在 ControlRoom
//! 组件内部，MainWindow 没有对应的 `control-room-active-monitor` 属性 ——
//! 判据只调 `monitor-switch(name)`（`.slint` 的 TouchArea 正是先翻组件内属性、
//! 再调这个回调），测的就是生产行为。
//!
//! 帧判据（D2）用两台全新 `EngineHost`（同工程、同 `reload`），与 190 的 C4 同款。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::host::ProductionLoop;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_engine::device::render_callback;
use yeban_engine::mixer::MonitorBus;
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

/// 渲染长度：64 × 128 = 8,192 帧 ≈ 0.17 s @48 kHz（覆盖起音＋稳态）。
const QUANTA: usize = 64;

/// 活窗口装配（与 `aether_callback_matrix.rs` 同款顺序）。
fn live_window() -> (
    LivePort<yeban_app::ui::MainWindow>,
    Rc<RefCell<EngineHost>>,
    YebanProjectV1,
) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<control-room-monitor-bus>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // 生产同款装配：最后 `wire_aether_actions` 用 `on_*` 覆盖语义接管新回调
    // （MON A/B 的生产处理器就注册在这里）。
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );
    (live, engine, project)
}

/// 经设备回调路径渲染 `QUANTA` 个量子（`process_quantum` 的监听级**生效**）。
fn render_via_callback(host: &mut EngineHost, quanta: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; 128 * 2 * quanta];
    let runtime = host
        .runtime_for_criteria()
        .expect("控制驱动形态下运行时必须在宿主手里");
    for quantum in 0..quanta {
        let start = quantum * 128 * 2;
        render_callback(runtime, &mut out[start..start + 128 * 2], 2);
    }
    out
}

/// D1：活窗口里 `monitor-switch` 真的切换引擎总线，且状态行报出所选路。
#[test]
fn monitor_switch_in_the_live_window_selects_the_engine_bus() {
    let (live, engine, _project) = live_window();
    let ui = live.ui();
    assert_eq!(
        engine.borrow().monitor_bus(),
        MonitorBus::A,
        "初始总线须为 A"
    );
    ui.invoke_monitor_switch("alt".into());
    assert_eq!(
        engine.borrow().monitor_bus(),
        MonitorBus::B,
        "D1 活窗口切到 alt ⇒ 引擎总线必须真变 B（修前只改状态行）"
    );
    let status = ui.get_status_selection().to_string();
    println!("[control-room-bus D1] 状态行 = {status}");
    assert!(
        status.contains("监听 alt"),
        "D1 状态行须报 监听 alt，实得 {status}"
    );
    // 切回去 ⇒ 总线也回去。
    ui.invoke_monitor_switch("main".into());
    assert_eq!(
        engine.borrow().monitor_bus(),
        MonitorBus::A,
        "D1 活窗口切回 main ⇒ 引擎总线必须回 A"
    );
}

/// D2：设备回调路径上 B 路增益真改帧（逐位 ×0.5）；切回 A 的那台逐位复原。
#[test]
fn bus_b_gain_changes_device_callback_frames_and_switching_back_restores() {
    let project = yeban_model::samples::filled_project();

    // 第一对：A 路（基线）vs B 路增益 0.5。
    let mut plain = EngineHost::new();
    plain.reload(&project, 0).expect("引擎必须能建起来");
    let mut alt = EngineHost::new();
    alt.reload(&project, 0).expect("引擎必须能建起来");
    alt.set_monitor_bus_gain(MonitorBus::B, 0.5);
    alt.set_monitor_bus(MonitorBus::B);

    let base = render_via_callback(&mut plain, QUANTA);
    let alt_frames = render_via_callback(&mut alt, QUANTA);
    let audible = base.iter().step_by(2).filter(|v| v.abs() > 1e-6).count();
    let changed = base
        .iter()
        .zip(alt_frames.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[control-room-bus D2] 基线可听帧 {audible}；B 路改动帧 {changed} / {}",
        base.len()
    );
    assert!(
        audible > 1_000,
        "D2 基线必须真的出声（可听帧 {audible}），否则比对是空转"
    );
    assert!(
        changed > 1_000,
        "D2 设备回调路径上 B 路增益必须真的改动输出（改动帧 {changed}）"
    );
    for (index, (b, d)) in base.iter().zip(alt_frames.iter()).enumerate() {
        assert_eq!(
            d.to_bits(),
            (b * 0.5f32).to_bits(),
            "D2 第 {index} 帧：B 路输出必须逐位 == 基线×0.5"
        );
    }

    // 第二对：同一工程、同一 B 路增益，但渲染前切回 A ⇒ 与基线逐位相同。
    let mut plain2 = EngineHost::new();
    plain2.reload(&project, 0).expect("引擎必须能建起来");
    let mut back = EngineHost::new();
    back.reload(&project, 0).expect("引擎必须能建起来");
    back.set_monitor_bus_gain(MonitorBus::B, 0.5);
    back.set_monitor_bus(MonitorBus::B);
    back.set_monitor_bus(MonitorBus::A);
    let base2 = render_via_callback(&mut plain2, QUANTA);
    let back_frames = render_via_callback(&mut back, QUANTA);
    assert_eq!(
        base2.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        back_frames.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "D2 切回 A 的那台必须与基线逐位相同"
    );
}

/// D3：B 路选中＋增益 0.5 时 `reload` ⇒ 换代后总线仍是 B、增益仍是 0.5。
#[test]
fn reload_preserves_the_monitor_bus_and_gain() {
    let (live, engine, project) = live_window();
    let ui = live.ui();
    ui.invoke_monitor_switch("alt".into());
    engine.borrow().set_monitor_bus_gain(MonitorBus::B, 0.5);
    assert_eq!(engine.borrow().monitor_bus(), MonitorBus::B);
    engine
        .borrow_mut()
        .reload(&project, 0)
        .expect("reload 必须成功");
    assert_eq!(
        engine.borrow().monitor_bus(),
        MonitorBus::B,
        "D3 B 路选中时 reload ⇒ 换代后总线仍须是 B（⛔ 不静默归零）"
    );
    assert_eq!(
        engine.borrow().monitor_bus_gain(MonitorBus::B),
        0.5,
        "D3 B 路增益 0.5 时 reload ⇒ 换代后增益仍须是 0.5"
    );
    assert_eq!(
        engine.borrow().monitor_bus_gain(MonitorBus::A),
        1.0,
        "D3 换代后 A 路增益仍须是 1.0（两路独立）"
    );
}

/// D4：未知名字不切总线、不 panic（只显示）。
#[test]
fn unknown_monitor_name_leaves_the_bus_unchanged() {
    let (live, engine, _project) = live_window();
    let ui = live.ui();
    assert_eq!(engine.borrow().monitor_bus(), MonitorBus::A);
    ui.invoke_monitor_switch("bogus".into());
    assert_eq!(
        engine.borrow().monitor_bus(),
        MonitorBus::A,
        "D4 未知名字 ⛔ 不许切总线（仍须是 A）"
    );
    let status = ui.get_status_selection().to_string();
    assert!(
        status.contains("监听 bogus"),
        "D4 状态行仍显示请求的名字，实得 {status}"
    );
}
