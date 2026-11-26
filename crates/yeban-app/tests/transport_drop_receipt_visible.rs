//! ⭐ `T-FIX-240`：**走带命令排到超时也没生效时，用户会被告知** —— 端到端判据。
//!
//! ## 被补上的缺口
//!
//! `T-FIX-271` 已经把"这批走带命令写进环几条 / 后置条件成立没有"如实记进
//! `TransportDispatch`，但**界面侧一个读者都没有**（全仓 `last_transport_dispatch`
//! 在 `engine_host.rs` 之外 0 命中）⇒ 满环时按 Play 依旧"没反应、也没人说话"。
//!
//! 本片把那条回执路径接通，并用真窗口 + 真生产循环断言：
//!
//! ```text
//! 环满 ⇒ send_transport 如实报告 is_dropped（T-FIX-271 的既有读数）
//!   ⇒ 尾巴留在 pending 队列（T-FIX-240）
//!   ⇒ 过了补写期限仍写不进去 ⇒ 记账为真丢
//!   ⇒ ProductionLoop::tick 读到增量 ⇒ report_notice（提示条 ＋ 模块回执行）
//! ```
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（调用方拿得到 dropped 结果） | 满环上发 Play ⇒ `dispatch.written == 0` 且 `dispatch.is_dropped()` |
//! | ②（界面真的说了） | 超期后的第一跳 `tick` ⇒ `module-status-text` 是错误态且写明"走带命令…没有生效"（⛔ 不只是改了返回类型） |
//! | ③（不重复弹） | 再跳一次，读数不再增长 ⇒ 回执行保持不变（⛔ 不每帧刷一条） |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ 不证明"补写最终生效"—— 那条在 `engine_host` 的单元判据
//!   `a_play_dropped_by_a_full_ring_is_retried_until_it_lands` 里自证（裸尖端红）。
//! - ⛔ 不证明真声卡形态：本文件用判据缝装一条事件环、**不推量子**（设备腿等价形态）。

#![allow(unused)]
use slint::ComponentHandle as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EditMark;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_engine::ring::{EngineEvent, TransportCommand};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款窗口尺寸。
const WINDOW: Size = Size::new(1440, 900);

#[test]
fn a_transport_command_that_times_out_in_the_ring_is_visible_in_the_ui() {
    let project = demo_project();
    let mut production = ProductionLoop::without_engine();
    let engine = production.engine_handle();
    // 判据缝：装一条**容量 2** 的事件环，把读者留在本测试里（⛔ 不推量子 ⇒ 设备腿形态）。
    engine.borrow_mut().install_event_channel_for_criteria(2);

    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");

    // 把环灌满（两条旧 Stop）。
    let filler: Vec<EngineEvent> = (0..2)
        .map(|_| EngineEvent::Transport {
            command: TransportCommand::Stop,
        })
        .collect();
    assert_eq!(
        engine.borrow_mut().send_engine_events(&filler),
        2,
        "灌环必须真的写满容量 2"
    );

    // 判据 ①：满环上的 Play 一条都写不进去，调用方如实拿到 dropped。
    let _reading = engine
        .borrow_mut()
        .send_transport(&[TransportCommand::Play]);
    let dispatch = engine.borrow().last_transport_dispatch();
    assert_eq!(dispatch.requested, 1);
    assert_eq!(dispatch.written, 0, "满环上 Play 写不进环");
    assert!(
        dispatch.is_dropped(),
        "调用方必须能看见'这条命令没进环'（T-FIX-271 的读数）"
    );

    // 读者始终不排空 ⇒ pending 过了补写期限也写不进去 ⇒ 记为真丢。
    std::thread::sleep(std::time::Duration::from_millis(1_200));

    // 判据 ②：下一跳 `tick` 必须把这条"没生效"投到界面（提示条 ＋ 模块回执行）。
    production.tick(port.ui(), &view, EditMark::new(None, 0), None);
    let text = port.ui().get_module_status_text().to_string();
    assert!(
        !port.ui().get_module_status_succeeded(),
        "丢命令的回执必须是错误态（当前: {text}）"
    );
    assert!(
        text.contains("走带命令") && text.contains("没有生效"),
        "回执行必须写明走带命令没有生效（当前: {text}）"
    );

    // 判据 ③：再跳一次，没有新的丢失 ⇒ 回执行不被重复覆盖。
    production.tick(port.ui(), &view, EditMark::new(None, 0), None);
    assert_eq!(
        port.ui().get_module_status_text().to_string(),
        text,
        "没有新的丢失时不得每帧再弹一条（⛔ 不刷屏）"
    );
}
