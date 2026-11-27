//! ⭐ **节拍器顶栏按钮的指针入口判据**（`T-FIX-867`，来源 `ISS-MWYA29X5-206421`）。
//!
//! ## 本文件钉的句子
//!
//! 新 `TransportHeaderBar` 必须真的画出 `transport-metronome-button`；运行时
//! ControlTree 里能找到这个 id，按指针注入能拨动节拍器，且与 `Cmd/Ctrl+Alt+N`
//! 走**同一个落点**（`host::apply_action` 的 `ToggleMetronome` 臂）：
//!
//! 1. 点击后权威文档 `transport.metronome_enabled` 真的翻转；
//! 2. 界面镜面 `metronome-on` 立即跟随；
//! 3. 下一跳发布后播放侧快照跟着变；
//! 4. 播放中开 ⇒ 推两拍 `metronome_clicks` 为正；关 ⇒ 再推两拍不再新增；
//! 5. 拨动前后走带一直在跑。
//!
//! ## 与 T-558 的关系
//!
//! T-558 把双入口判据改成行为断言，但当时新顶栏没有按钮，只能用 `toggle-metronome`
//! 回调当按钮入口；本判据补上真正的指针入口。行为读数与 T-558 同口径。
//!
//! ## 边界
//!
//! - 关闭后的读数是"**不再新增**"而不是"归零"：本装配有权威会话，拨动只提交 op、
//!   ⛔ 不换引擎，因此累计计数不会重置。
//! - `set_platform` 每线程一次 ⇒ 本文件只有一个 `#[test]`。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{EngineTick, LiveUi, LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_app::engine_host::EditMark;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

const NOW_MS: u64 = 1_700_000_000_000;
const QUANTA_TWO_BEATS: u64 = 400;

fn assemble(tag: &str) -> (LiveUi, Rc<UndoPort>, MainWindow) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(tag, "yeban-app", project.clone(), NOW_MS).expect("打开会话"),
    ));
    let mut live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    live.start_engine().expect("起一代引擎");
    let window = live.ui().clone_strong();
    (live, port, window)
}

fn live_project(port: &UndoPort) -> yeban_model::YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 前提不成立）")
}

fn doc_enabled(port: &UndoPort) -> bool {
    live_project(port).transport.metronome_enabled
}

fn snapshot_enabled(live: &LiveUi) -> Option<bool> {
    live.engine_metronome_enabled()
}

fn clicks(live: &LiveUi) -> u64 {
    live.engine_metronome_clicks().expect("夹具已经起引擎")
}

fn heartbeat(live: &mut LiveUi, port: &UndoPort) -> EngineTick {
    let mark = EditMark::from_display(&port.display());
    let project = live_project(port);
    live.engine_heartbeat(mark, Some(&project))
}

fn press_space(window: &MainWindow) -> bool {
    window.invoke_key_action(" ".into(), false, false, false, false)
}

fn press_metronome(window: &MainWindow) -> bool {
    window.invoke_key_action("n".into(), false, true, true, false)
}

#[test]
fn pointer_click_on_the_header_button_flips_metronome() {
    let (mut live, port, window) = assemble("metronome-button-t867");

    assert!(!doc_enabled(&port), "夹具文档节拍器应为关");
    assert!(!window.get_metronome_on(), "装配镜面应来自文档（关）");
    assert_eq!(snapshot_enabled(&live), Some(false), "装配快照应为关");
    assert_eq!(clicks(&live), 0, "未播放时不应有咔哒");
    assert!(!live.engine_transport_running(), "装载即停住");

    // 入口前提：ControlTree 必须真的有这颗键，且 id 与旧件一致。
    let tree = live.tree_snapshot();
    assert!(
        tree.find_by_id("transport-metronome-button").is_some(),
        "新顶栏 ControlTree 必须含 transport-metronome-button；实际 dump: {}",
        tree.dump_json()
    );

    assert!(press_space(&window), "Space 必须被消费");
    assert!(live.engine_transport_running(), "Space 后走带应在跑");

    // 入口 ①：指针注入顶栏按钮（⛔ 不直接调 Rust 回调）。
    live.click_element("transport-metronome-button", 16.0, 12.0)
        .expect("节拍器按钮必须可按下");
    assert!(doc_enabled(&port), "按钮按下后权威文档必须翻成开");
    assert!(window.get_metronome_on(), "按钮按下后镜面必须立即跟随");
    assert_eq!(
        snapshot_enabled(&live),
        Some(false),
        "拨动只提交 op，尚未发布 ⇒ 快照应仍是上一代"
    );

    let published = heartbeat(&mut live, &port);
    assert!(
        published.published_revision.is_some(),
        "文档变了 ⇒ 下一跳必须发布新快照"
    );
    assert_eq!(snapshot_enabled(&live), Some(true), "发布后快照应为开");

    live.drive_audio(QUANTA_TWO_BEATS);
    let clicks_after_on = clicks(&live);
    assert!(
        clicks_after_on > 0,
        "播放中打开后推两拍必须数到咔哒，实际 {clicks_after_on}"
    );
    assert!(live.engine_transport_running(), "拨动节拍器不许停掉走带");

    // 入口 ②：`Cmd/Ctrl+Alt+N`（键盘入口，同一个落点）。
    assert!(press_metronome(&window), "Cmd/Ctrl+Alt+N 必须被消费");
    assert!(!doc_enabled(&port), "键盘按下后权威文档必须翻回关");
    assert!(!window.get_metronome_on(), "键盘按下后镜面必须立即跟随");

    let published = heartbeat(&mut live, &port);
    assert!(
        published.published_revision.is_some(),
        "文档再变 ⇒ 下一跳必须发布新快照"
    );
    assert_eq!(snapshot_enabled(&live), Some(false), "发布后快照应为关");

    let before_off = clicks(&live);
    live.drive_audio(QUANTA_TWO_BEATS);
    assert_eq!(
        clicks(&live),
        before_off,
        "关闭后同样推两拍不许再新增咔哒（累计计数不重置，等价可测信号）"
    );
    assert!(live.engine_transport_running(), "两个入口都不许停掉走带");
}
