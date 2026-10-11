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
    // ⚠ 每个进程只能装一次平台 ⇒ 本文件只有一个 `#[test]`。
    install_testing_backend_with_event_loop();

    let view = ViewState::from_project(&yeban_model::samples::filled_project())
        .expect("filled_project 必须能投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");

    // ⭐ 真入口（⛔ 不注入替身）：这一条要证的正是**产品**那条路。
    // 端口 `0`、秒数很小 —— 无端口机器（CI）上采集入口会立刻返回，
    // ⚠ 但"立刻"**不是**本判据的依赖：下面用 join 同步，不看时间。
    let paths = host::ModulePaths {
        midi_in_port: "0".to_owned(),
        midi_in_seconds: 0.05,
        ..host::ModulePaths::default()
    };
    let state = host::wire_module_actions_with_capture(&ui, None, paths, None);

    ui.invoke_midi_in();
    // ---- 1. 中间态：同步写下（与 `tests/ui_module_actions.rs` 的 ④-a 同一读数）----
    let pending = ui.get_module_status().to_string();
    assert!(
        pending.starts_with("module:action=midi-in:outcome=pending"),
        "触发后必须立刻处于中间态: {pending}"
    );
    // ⚠ 这里**不**断言 `is_in_flight()`：真入口在无端口机器上微秒级就返回，
    // 而"在飞"这一窗口的**确定性**判定由 `tests/ui_module_actions.rs` 的判据 ⑤
    // 用**注入替身**（把窗口拉住）完成 —— ⛔ 不在这里写一个会抖的断言。

    // ---- 2. 确定性等待：join 工作线程（它已经把排空闭包投进事件循环）----
    assert!(
        host::wait_for_pending_midi_in(&state),
        "必须真的 join 到一个工作线程（否则本判据的前提不成立）"
    );
    assert!(
        state.has_pending(),
        "join 之后结果必须已经在 Send 的槽里 —— ⛔ 界面这时**还**不该变"
    );
    assert_eq!(
        ui.get_module_status(),
        slint::SharedString::from(pending.clone()),
        "还没跑事件循环 ⇒ 界面必须仍是中间态（这一条把'谁改了界面'钉死）"
    );

    // ---- 3. 跑掉事件循环里排队的那一条 ⇒ 结果在 **UI 线程**上落界面 ----
    drain_event_loop();
    assert!(
        !state.has_pending(),
        "事件循环跑过之后槽必须被排空（⇒ 那一跳真的执行了，⛔ 不是我手工调的）"
    );

    // ---- 4. 终态：两档各自断言 ----
    let receipt = ui.get_module_status().to_string();
    let midi_receipt = ui.get_module_midi_in_status().to_string();
    assert_eq!(
        midi_receipt, receipt,
        "两条状态行必须是同一串字节（`apply_pending_midi_in` 一次计算、写两处）"
    );
    assert!(
        receipt.starts_with("module:action=midi-in:outcome="),
        "终态必须带 outcome: {receipt}"
    );
    assert!(
        !receipt.contains("outcome=pending"),
        "终态不得还是中间态: {receipt}"
    );
    assert_eq!(
        ui.get_module_midi_in_status_succeeded(),
        ui.get_module_status_succeeded(),
        "两条状态行的成功位必须一致"
    );
    if cfg!(feature = "midi-input") {
        assert!(
            !receipt.contains("error=feature-disabled"),
            "⭐ `midi-input` 档不该报 feature 未启用: {receipt}"
        );
    } else {
        // ⭐ 默认档：⭐ 必须点名 feature ＋ 一句怎么开（措辞来自 `MidiInError::Display`）。
        assert!(
            receipt.contains("error=feature-disabled")
                && receipt.contains("midi-input")
                && receipt.contains("--features midi-input"),
            "默认档必须给出点名 feature 且带'怎么开'的**结构化**说法: {receipt}"
        );
    }
    // ⭐ 把这一档**实际**给出的说法写进 CI 日志（`report_line` 直写 fd 2 ⇒ CI 日志可见）。
    report_line(&format!(
        "[ui-modules-async] feature={} event-loop-hop=ok receipt={receipt}",
        if cfg!(feature = "midi-input") {
            "on"
        } else {
            "off"
        }
    ));
}
