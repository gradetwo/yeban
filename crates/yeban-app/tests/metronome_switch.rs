//! ⭐ **节拍器开关的双入口行为判据**（`#16` / `T-FIX-558`）。
//!
//! ## 本文件钉的句子
//!
//! 走带区那颗 `transport-metronome-button` 与键盘 `Cmd/Ctrl+Alt+N` 是**同一个落点**
//! （`host::apply_action` 的 `ToggleMetronome` 臂）。两入口各按一次，必须：
//!
//! 1. **权威文档** `transport.metronome_enabled` 真的翻转（开 → `true`，关 → `false`）；
//! 2. 界面镜面 `metronome-on` 立即跟随；
//! 3. 下一跳发布后，**播放侧快照**跟着变；
//! 4. 播放中开 ⇒ 推两拍后 `metronome_clicks` **为正**；关 ⇒ 同样推两拍**不再新增**咔哒；
//! 5. 拨动前后走带**一直在跑**（⛔ 不换引擎、不停走带）。
//!
//! ## 为什么不再用源码字符串 grep
//!
//! 旧判据只断言 `app.slint` / `host.rs` 含有 `key-action` / `on_key_action` /
//! `toggle-play` / `on_toggle_play` 字样 —— 那只能证明"字符串在文件里"，
//! 把任一入口的落点或镜面跟随改坏，它仍然全绿。`T-FIX-550` 审计把这条记为
//! **空转判据**；本文件用真窗口、真指针、真按键、真文档、真咔哒计数重写。
//!
//! ## 装配
//!
//! 与产品进程同一形态：`LiveUi`（真 Tier-1 活窗口）＋ `UndoPort` 权威会话 ＋
//! 起一代引擎（`engine_quanta = 0`，装载即停住）。`Space` 起播；按钮入口走
//! `.slint` 声明的 `invoke_toggle_metronome`（新 UI 顶栏不再进运行时控件树，指针
//! 无法按 ID 寻址；该回调就是按钮的生产入口）；键盘入口走 `invoke_key_action`；
//! 发布用 `engine_heartbeat`；发声用 `drive_audio` 推量子。
//!
//! ## 边界
//!
//! - ⛔ 没有人耳听；证的是"快照里有没有节拍器计划 ⇒ 渲染出没出咔哒"。
//! - 关闭后的读数是"**不再新增**"而不是"归零"：本装配有权威会话，拨动只提交 op、
//!   ⛔ 不换引擎，因此累计计数不会重置；"不再新增"是同一件事的可测信号。
//! - `set_platform` 每线程一次 ⇒ 本文件只有一个 `#[test]`。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 编译与判决交 CI；本机只做 `cargo fmt` 与静态核对。

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

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 覆盖 ≥2 拍：120 BPM / 48 kHz ⇒ 400 个量子 = 51_200 帧，无论开关落在拍内哪里
/// 都至少过一次咔哒。
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
fn both_entries_flip_the_document_and_the_clicks_follow_playback() {
    let (mut live, port, window) = assemble("metronome-switch-t558");

    // 前提读数：文档 / 镜面 / 快照 / 咔哒 / 走带全部对齐在"关、停"。
    assert!(!doc_enabled(&port), "夹具文档节拍器应为关");
    assert!(!window.get_metronome_on(), "装配镜面应来自文档（关）");
    assert_eq!(snapshot_enabled(&live), Some(false), "装配快照应为关");
    assert_eq!(clicks(&live), 0, "未播放时不应有咔哒");
    assert!(!live.engine_transport_running(), "装载即停住");

    // 起播：后面所有咔哒读数都必须发生在 Playing 里。
    assert!(press_space(&window), "Space 必须被消费");
    assert!(live.engine_transport_running(), "Space 后走带应在跑");

    // 入口 ①：走带区按钮的 `.slint` 回调（按钮按下的生产入口）。
    window.invoke_toggle_metronome();
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
