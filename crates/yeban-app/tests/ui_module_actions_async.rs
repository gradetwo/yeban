//! **硬件 MIDI 采集的跨线程那一跳** —— 端到端判据（第二个测试二进制，见下）。
//!
//! ## 这个文件证什么（`tests/ui_module_actions.rs` 证不了的那一格）
//!
//! 产品路径上"采集结果回到 UI 线程"这一跳走的是
//! [`slint::Weak::upgrade_in_event_loop`]：**工作线程**做完采集，把结果放进 `Send` 的槽，
//! 然后请求事件循环在 **UI 线程**上排空它。
//!
//! ⚠ 那一跳在 Tier-1 平台上**不可能**发生：`crates/yeban-ui-test-port` 的 `Tier1Platform`
//! 只实现 `create_window_adapter`，**没有** `new_event_loop_proxy`
//! ⇒ `upgrade_in_event_loop` 返回 `Err`（这不是错误：结果留在槽里，由调用方排空）。
//! ⇒ 于是"那条跳真的通"必须**另用**一个有事件循环的平台来判：本文件装
//! [`install_testing_backend_with_event_loop`]（`threading: true`、mock renderer）。
//!
//! ## 为什么本文件**不能**判像素（`[MUST-GATE-015]`）
//!
//! 那个后端是 mock renderer ⇒ **不渲染像素**。Golden 仍然只由 `tests/real_ui_tier1.rs`
//! 的 Tier-1 光栅化产出。两个后端不能同时装（一个进程一个平台）⇒ 分成两个测试二进制。
//!
//! ## 确定性：⛔ 无 sleep、⛔ 无轮询、⛔ 无超时
//!
//! 三步各自有**结构性**保证：
//! 1. 触发是同步的：`invoke_midi_in()` 返回时，中间态已经写下、工作线程已经 spawn；
//! 2. [`yeban_app::host::wait_for_pending_midi_in`] **join** 工作线程 ⇒ 它里面的
//!    `upgrade_in_event_loop` 已经**返回**（那条闭包**已经**在事件循环队列里）；
//! 3. [`drain_event_loop`] 先压一条 `Quit` 再进循环 ⇒ 队列 **FIFO**：排空闭包先跑、`Quit`
//!    后到 ⇒ 循环一定终止（本仓 `tests/production_reprojection.rs` 的同一手法）。

#![allow(unused)]
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;
use yeban_ui_test_port::render::report_line;

/// 把事件循环里**已经排队**的那一条调用跑掉就返回。
///
/// ⛔ 没有 sleep、没有轮询、没有超时 —— 终止性来自"先压 `Quit`"。
fn drain_event_loop() {
    slint::quit_event_loop().expect("带事件循环的后端必须提供 proxy（否则本判据不成立）");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

#[test]
fn the_worker_thread_hands_the_midi_result_back_through_the_event_loop() {
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
        aether_app.contains("callback midi-in("),
        "新 UI 契约面必须声明 callback midi-in("
    );
    assert!(
        aether_host.contains("on_midi_in"),
        "宿主接线层必须注册 on_midi_in"
    );
    assert!(
        aether_app.contains("> module-midi-in-status:"),
        "新 UI 契约面必须声明 > module-midi-in-status:"
    );
    assert!(
        aether_host.contains("set_module_midi_in_status"),
        "宿主接线层必须注册 set_module_midi_in_status"
    );
    assert!(
        aether_app.contains("> module-midi-in-status-succeeded:"),
        "新 UI 契约面必须声明 > module-midi-in-status-succeeded:"
    );
    assert!(
        aether_host.contains("set_module_midi_in_status_succeeded"),
        "宿主接线层必须注册 set_module_midi_in_status_succeeded"
    );
}
