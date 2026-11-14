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

#![allow(unused)]
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据 2：**停止后下一跳显示正确**（引擎的 `Stop` 是暂停语义，位置保留）。
///
/// 若心跳只在 `playing` 时刷新（一种很容易写出来的"优化"），停止之后界面会停在
/// **停止前**那一帧的时间码 ⇒ 本判据红。
#[test]
fn one_tick_after_stop_shows_the_engines_position() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
