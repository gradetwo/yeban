//! **播放推进期间时间码刷新**的端到端判据（任务单 `#228`）。
//!
//! ## 被钉住的缺口（`#227` 如实登记的既有行为）
//!
//! `src/main.rs` 的 60Hz 心跳（`ProductionLoop::tick`）**从不**调
//! [`yeban_app::host::apply_transport`]，而它是 `playing` ＋ `timecode` 的**唯一**写者。
//! 于是引擎在播、界面顶栏的数字**一位不动** —— 用户看到的形状是"按了播放，像卡了"。
//!
//! 本文件驱动的是**生产装配**：真 `EngineHost`（`ProductionLoop::start`）、真
//! `EngineHost::transport()` 原子镜面、真 `host::apply_transport`、真 Tier-1 软件
//! 光栅化窗口（`host::build_main_window`，与 `run_gui` 同一段装配）。
//!
//! ## 判据
//!
//! 1. [`timecode_moves_while_the_transport_runs`] —— 播放中**每一跳**都读界面的
//!    `timecode`：它必须跟着引擎 `position_ticks` 前进（逐跳打印 tick / 时间码）。
//! 2. [`one_tick_after_stop_shows_the_engines_position`] —— 停止后**下一跳**的显示
//!    必须等于"引擎 `position_ticks` 的时间码"（证明停止腿也刷新；⛔ 差一帧）。
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ `run_gui` 的事件循环没有被执行（本机/CI 无显示器）。这里驱动的是它调用的
//!   **同一个** `ProductionLoop::tick`。
//! - ⛔ "真机上数字真的动"只能人眼复验（见任务单报告 §8 的复验步骤）。

use yeban_app::bridge::{TimecodeGrid, ViewState, demo_project, timecode_for_ticks};
use yeban_app::engine_host::EditMark;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::LivePort;
use yeban_ui_test_port::image::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 一个"活窗口 + 投影"的夹具（与 `src/main.rs` 共用 `host::build_main_window`）。
struct Window {
    port: LivePort<yeban_app::ui::MainWindow>,
    view: ViewState,
}

impl Window {
    fn new(project: &yeban_model::project::YebanProjectV1) -> Self {
        let view = ViewState::from_project(project).expect("工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let size = Size::new(scene.viewport_width, scene.viewport_height);
        let port = LivePort::new(size, Permission::ReadOnly, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");
        Self { port, view }
    }

    fn ui(&self) -> &yeban_app::ui::MainWindow {
        self.port.ui()
    }
}

/// 与 `src/main.rs` 同一段装配：建一代引擎 → 记下开局标记 → 显式 `Stop`（界面初始态
/// 是 `playing: false`）→ 立刻把引擎读数写回界面一次。
fn production_and_window() -> (ProductionLoop, Window) {
    let project = demo_project();
    let window = Window::new(&project);
    let production = ProductionLoop::start(&project, 0).expect("引擎重建");
    assert!(production.has_engine(), "引擎应当活着");
    production
        .engine_handle()
        .borrow_mut()
        .mark_applied(EditMark::new(None, 0));
    // `src/main.rs` 的同一行：新引擎默认自由跑，而界面初始态是停住 ⇒ 显式 `Stop`。
    production.engine_handle().borrow_mut().stop();
    host::apply_transport(window.ui(), production.engine_handle().borrow().transport());
    (production, window)
}

/// 界面此刻的 `timecode` 文本（顶栏显示的唯一来源）。
fn ui_timecode(window: &Window) -> String {
    window.ui().get_timecode().to_string()
}

/// "引擎此刻的位置按界面上的拍号网格应该显示什么" —— 与生产唯一换算实现同源。
fn expected_timecode(window: &Window, position_ticks: u64) -> String {
    let grid = TimecodeGrid::from_injected(
        window.ui().get_timecode_ticks_beat(),
        window.ui().get_timecode_ticks_bar(),
    );
    timecode_for_ticks(grid, position_ticks)
}

/// 判据 1：**播放中每一跳的时间码都必须变**。
///
/// 若 `ProductionLoop::tick` 不调 `apply_transport`（修前的形状），第一跳之后
/// `ui_timecode` 恒为 `001.01.000` ⇒ 第一次逐跳比较就红。
#[test]
fn timecode_moves_while_the_transport_runs() {
    let (mut production, window) = production_and_window();
    assert_eq!(
        ui_timecode(&window),
        expected_timecode(&window, 0),
        "开局显示必须等于引擎位置 0 的时间码"
    );
    assert!(!window.ui().get_playing(), "开局显示态是停住");

    // 与 `wire_transport` 的 `on_toggle_play` 同一条路：发 `Play`，动作后立刻回写一次。
    let started = production.engine_handle().borrow_mut().play();
    host::apply_transport(window.ui(), started);
    assert!(started.state.is_running(), "`Play` 之后引擎必须在跑");

    let mut seen: Vec<(u64, u64, String)> = vec![(0, started.position_ticks, ui_timecode(&window))];
    report_line(&format!(
        "[timecode-tick] tick=0 engine_position_ticks={} ui_timecode={} ui_playing={}",
        started.position_ticks,
        ui_timecode(&window),
        window.ui().get_playing()
    ));
    for frame in 1..=8u64 {
        // 设备回调那一侧的替身：真的推一个量子（与 `production_meter_leg` 同款）。
        production.engine_handle().borrow_mut().drive_audio(1);
        let tick = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
        assert!(
            tick.readings.is_some(),
            "第 {frame} 跳必须真的走到引擎心跳（没有引擎 ⇒ 本判据没有意义）"
        );
        assert!(window.ui().get_playing(), "播放中 `playing` 必须为真");
        let reading = production.engine_handle().borrow().transport();
        let timecode = ui_timecode(&window);
        report_line(&format!(
            "[timecode-tick] tick={frame} engine_position_ticks={} ui_timecode={} ui_playing={}",
            reading.position_ticks,
            timecode,
            window.ui().get_playing()
        ));
        assert_eq!(
            timecode,
            expected_timecode(&window, reading.position_ticks),
            "第 {frame} 跳：界面时间码必须等于引擎 `position_ticks` 的时间码 \
             （不是上一跳的陈旧值）"
        );
        seen.push((frame, reading.position_ticks, timecode));
    }

    assert!(
        seen.last().expect("至少一跳").1 > seen.first().expect("至少一跳").1,
        "8 跳之后引擎位置必须真的前进（夹具前提；实得 {seen:?}）"
    );
    for pair in seen.windows(2) {
        assert_ne!(
            pair[0].2, pair[1].2,
            "tick {} → {} 的时间码没有变化（{}）：播放推进期间界面数字必须跟着走",
            pair[0].0, pair[1].0, pair[0].2
        );
    }
    let final_position = production
        .engine_handle()
        .borrow()
        .transport()
        .position_ticks;
    assert_eq!(
        ui_timecode(&window),
        expected_timecode(&window, final_position),
        "最后一跳之后显示必须仍与引擎位置一致"
    );
}

/// 判据 2：**停止后下一跳显示正确**（引擎的 `Stop` 是暂停语义，位置保留）。
///
/// 若心跳只在 `playing` 时刷新（一种很容易写出来的"优化"），停止之后界面会停在
/// **停止前**那一帧的时间码 ⇒ 本判据红。
#[test]
fn one_tick_after_stop_shows_the_engines_position() {
    let (mut production, window) = production_and_window();
    production.engine_handle().borrow_mut().play();
    for frame in 1..=4u64 {
        production.engine_handle().borrow_mut().drive_audio(1);
        let _ = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
        report_line(&format!(
            "[timecode-stop] tick={frame} playing={} ui_timecode={}",
            window.ui().get_playing(),
            ui_timecode(&window)
        ));
    }
    let before_stop = ui_timecode(&window);
    assert!(window.ui().get_playing(), "停止前必须在播");

    // 模拟"两次心跳之间过去了真实时间"：引擎位置前进，但**这一帧界面还没刷新**
    // （生产里音频线程由真实时钟驱动，本判据由控制面显式推量子）。
    production.engine_handle().borrow_mut().drive_audio(3);
    let advanced = production.engine_handle().borrow().transport();
    assert!(
        advanced.position_ticks > 0,
        "夹具前提：推进后引擎位置必须大于 0"
    );
    assert_ne!(
        before_stop,
        expected_timecode(&window, advanced.position_ticks),
        "夹具前提：界面此刻必须落后于引擎（否则本条判据区分不出「停止腿没刷新」）"
    );

    // 与 `wire_transport` 的 `on_stop` 同一条路：发 `Stop`（位置保留）。
    let stopped = production.engine_handle().borrow_mut().stop();
    assert!(!stopped.state.is_running(), "`Stop` 之后引擎必须停住");
    // 再让边界到来一次（控制驱动的形态），然后走**下一跳**。
    production.engine_handle().borrow_mut().drive_audio(1);
    let _ = production.tick(window.ui(), &window.view, EditMark::new(None, 0), None);
    let reading = production.engine_handle().borrow().transport();
    report_line(&format!(
        "[timecode-stop] after_stop engine_position_ticks={} engine_state={:?} \
         ui_timecode={} ui_playing={} before_stop_ui_timecode={}",
        reading.position_ticks,
        reading.state,
        ui_timecode(&window),
        window.ui().get_playing(),
        before_stop
    ));
    assert!(!window.ui().get_playing(), "停止后 `playing` 必须为假");
    assert_eq!(
        ui_timecode(&window),
        expected_timecode(&window, reading.position_ticks),
        "停止后下一跳的显示必须等于引擎 `position_ticks` 的时间码（⛔ 差一帧）"
    );
    assert_eq!(
        reading.position_ticks, stopped.position_ticks,
        "`Stop` 之后位置必须保留（暂停语义）"
    );
}
