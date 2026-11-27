//! T-FIX-601：换工程后总览 A/B 状态条仍显示旧工程"A 已存 / 对比摘要"。
//!
//! ## 本文件要证的句子
//!
//! `song_overview_ab_status` 是窗口属性，Slint 默认值空串不会因换工程复位；
//! 而新会话 `SongOverviewCore.compare` 已被 `wire_song_overview` 重建为
//! `AbCompare::default()`（A/B 全空）。于是状态条残留旧工程的"A 已存"是关键假状态：
//! 用户按"对比"却收到"先存 A 与 B，再对比"。
//!
//! ## 判据（真行为，非字符串 grep）
//!
//! | 判据 | 验证内容 | 红对照（删装配末尾的复位） |
//! | :--- | :--- | :--- |
//! | `capture_a_then_rewire_clears_ab_status_and_compare_refuses` | 真 `invoke_song_overview_capture_a` 写入"A 已存" ⇒ 重新 `wire_song_overview`（换工程重装配，新 compare=default）⇒ ab-status 空、compare 回"先存 A 与 B，再对比" | 重新装配后 ab-status 仍是"A 已存" ⇒ 红 |

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::engine_host::EngineHost;
use yeban_app::host::{self, ProductionLoop};
use yeban_app::song_overview;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const NOW_MS: u64 = 1_700_000_000_000;

/// 可重装配的活台句柄（持有 `_production` 让引擎消费端一直在）。
type Rig = (
    LivePort<MainWindow>,
    Rc<RefCell<EngineHost>>,
    Rc<RefCell<ViewState>>,
    Rc<UndoPort>,
    ProductionLoop,
);

/// 建一台真引擎 + 真活窗口 + 产品那一个 `wire_song_overview`，返回可重装配的句柄。
fn rig() -> Rig {
    let project = demo_project();
    let production =
        ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
    let engine = production.engine_handle();
    engine.borrow_mut().stop();
    let view = ViewState::from_project(&project).expect("工程投影");
    let scene = yeban_app::scene::DemoScene::from_view(&view);
    let size = Size::new(1280, 800);
    let port = LivePort::new(size, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("主窗口");
    let live_view = Rc::new(RefCell::new(view));
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<ab-reset>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    song_overview::wire_song_overview(
        port.ui(),
        Rc::clone(&engine),
        Rc::clone(&live_view),
        Some(Rc::clone(&undo)),
    );
    (port, engine, live_view, undo, production)
}

/// 真行为：存 A ⇒ 状态条写"A 已存"；换工程重装配 ⇒ 状态条清空且"对比"拒绝。
#[test]
fn capture_a_then_rewire_clears_ab_status_and_compare_refuses() {
    let (port, engine, live_view, undo, _production) = rig();
    let ui = port.ui();

    // ① 真 capture-a：状态条写入"A 已存 · N 小节"（不是手工 set 属性）。
    ui.invoke_song_overview_capture_a();
    let stored = ui.get_song_overview_ab_status().to_string();
    assert!(
        stored.contains("A 已存"),
        "存 A 后状态条应显示 A 已存，实际: {stored}"
    );

    // ② 换工程重装配：main.rs 换工程会重跑 wire_callbacks → wire_song_overview，
    //    新 compare = AbCompare::default()。状态条必须跟着复位到空串。
    song_overview::wire_song_overview(
        ui,
        Rc::clone(&engine),
        Rc::clone(&live_view),
        Some(Rc::clone(&undo)),
    );
    let after_rewire = ui.get_song_overview_ab_status().to_string();
    assert!(
        after_rewire.is_empty(),
        "换工程重装配后 A/B 状态条必须清空（旧工程'{stored}'不得残留），实际: '{after_rewire}'"
    );

    // ③ 此时按"对比"：新会话 A/B 全空 ⇒ 提示先存 A 与 B，不得复用旧摘要。
    ui.invoke_song_overview_compare_ab();
    let compare = ui.get_song_overview_ab_status().to_string();
    assert!(
        compare.contains("先存 A 与 B") || compare.contains("先存A与B"),
        "空 compare 下点对比应提示先存 A 与 B，实际: {compare}"
    );
    assert!(
        !compare.contains("小节") || compare.contains("先存"),
        "旧工程的对比摘要不得带到新工程: {compare}"
    );
}
