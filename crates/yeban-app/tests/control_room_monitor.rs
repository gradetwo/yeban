//! T-FIX-190：控制室 DIM / MONO / SIDE 三个开关**真正接进引擎监听输出级**（活窗口判据）。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | C1 | 活窗口里翻 `control-room-dim` ＋调 `control-room-toggle-dim` ⇒ 引擎 DIM 标志真变，且状态行仍报 `DIM=开(-20dB)` | 回调只调状态（修前行为） |
//! | C2 | 同上 MONO；且下坞 `master-toggle-mono`（`execute-action`）⇒ 引擎 MONO 标志真变 | MONO 臂没覆盖 / 回调没接引擎 |
//! | C3 | 同上 SIDE | 回调只调状态 |
//! | C4 | 两台同工程引擎（设备回调路径 `render_callback`）⇒ DIM 开的那台每帧逐位 `== 基线 × 0.1` | 监听级只在 UI 层 / 没进 `process_quantum` |
//! | C5 | 同一对引擎再经 `render_into`（离线渲染）⇒ DIM 开的那台与基线**逐位相同**（旁通） | 旁通没接 ⇒ 监听开关漏进导出 |
//! | C6 | DIM 开着 `reload` ⇒ 换代后 DIM 仍是开的（宿主把旧值拷进新运行时） | 换代静默归零 ⇒ 界面与引擎脱节 |
//!
//! 装配与 `tests/aether_callback_matrix.rs` 同款：Tier-1 活窗口（Slint testing
//! backend ＋ 生产同款 `wire_*` 装配，先旧回调、最后 `wire_aether_actions` 覆盖
//! 接管）。`.slint` 的 TouchArea 是"先翻属性、再调回调" ⇒ 判据里用
//! `set_control_room_*` ＋ `invoke_control_room_toggle_*` 复现**同一顺序**
//! （⛔ 不只调回调：那样读到的还是翻转前的值，测的不是生产行为）。
//!
//! 帧判据（C4/C5）不用活窗口那台引擎的渲染 —— 活窗口的引擎在 C1–C3 里没有泵过
//! 量子，位置仍在 tick 0；但为与"开"的那台**同一状态**比对，C4/C5 用两台全新
//! `EngineHost`（同工程、同 `reload`）分别走设备回调与离线渲染。两台独立引擎
//! 同工程渲染逐位相同是既有确定性契约（见 `yeban-engine` 的 M6）。

use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EngineHost;
use yeban_app::host;
use yeban_app::host::ProductionLoop;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_engine::device::render_callback;
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
            "<control-room-monitor>",
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
    // （控制室三开关的生产处理器就注册在这里）。
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

/// C1：DIM 开关从活窗口真的写进引擎，且状态行仍报 `DIM=开(-20dB)`。
#[test]
fn dim_toggle_in_the_live_window_sets_the_engine_flag() {
    let (live, engine, _project) = live_window();
    let ui = live.ui();
    assert!(
        !engine.borrow().monitor_controls().dim(),
        "初始 DIM 标志须为关"
    );
    // `.slint` 的顺序：先翻属性、再调回调。
    ui.set_control_room_dim(true);
    ui.invoke_control_room_toggle_dim();
    assert!(
        engine.borrow().monitor_controls().dim(),
        "C1 活窗口开 DIM ⇒ 引擎 DIM 标志必须真变（修前只改状态行）"
    );
    let status = ui.get_status_selection().to_string();
    println!("[control-room C1] 状态行 = {status}");
    assert!(
        status.contains("DIM=开(-20dB)"),
        "C1 状态行仍须报 DIM=开(-20dB)，实得 {status}"
    );
    // 关回去 ⇒ 标志也回去。
    ui.set_control_room_dim(false);
    ui.invoke_control_room_toggle_dim();
    assert!(
        !engine.borrow().monitor_controls().dim(),
        "C1 活窗口关 DIM ⇒ 引擎 DIM 标志必须回去"
    );
}

/// C2：MONO 开关从活窗口真的写进引擎；下坞 `master-toggle-mono` 同样写进引擎。
#[test]
fn mono_toggle_and_master_mono_action_set_the_engine_flag() {
    let (live, engine, _project) = live_window();
    let ui = live.ui();
    assert!(
        !engine.borrow().monitor_controls().mono(),
        "初始 MONO 标志须为关"
    );
    ui.set_control_room_mono(true);
    ui.invoke_control_room_toggle_mono();
    assert!(
        engine.borrow().monitor_controls().mono(),
        "C2 活窗口开 MONO ⇒ 引擎 MONO 标志必须真变"
    );
    // 下坞臂：`master-toggle-mono` 翻属性 ＋ 调同一条回调 ⇒ 标志跟着翻回去。
    ui.invoke_command_execute("master-toggle-mono".into());
    assert!(
        !engine.borrow().monitor_controls().mono(),
        "C2 下坞 master-toggle-mono ⇒ 引擎 MONO 标志必须跟着翻回去（同属一个 mono 标志）"
    );
    assert!(
        !ui.get_control_room_mono(),
        "C2 下坞臂翻回后界面属性也须为关"
    );
}

/// C3：SIDE 开关从活窗口真的写进引擎。
#[test]
fn side_toggle_in_the_live_window_sets_the_engine_flag() {
    let (live, engine, _project) = live_window();
    let ui = live.ui();
    assert!(
        !engine.borrow().monitor_controls().side(),
        "初始 SIDE 标志须为关"
    );
    ui.set_control_room_side(true);
    ui.invoke_control_room_toggle_side();
    assert!(
        engine.borrow().monitor_controls().side(),
        "C3 活窗口开 SIDE ⇒ 引擎 SIDE 标志必须真变"
    );
    ui.set_control_room_side(false);
    ui.invoke_control_room_toggle_side();
    assert!(
        !engine.borrow().monitor_controls().side(),
        "C3 活窗口关 SIDE ⇒ 引擎 SIDE 标志必须回去"
    );
}

/// C4＋C5：设备回调路径上 DIM 真改帧（逐位 ×0.1）；离线渲染路径上 DIM 不改帧（旁通）。
#[test]
fn dim_changes_device_callback_frames_but_not_offline_render_frames() {
    let project = yeban_model::samples::filled_project();
    let mut plain = EngineHost::new();
    plain.reload(&project, 0).expect("引擎必须能建起来");
    let mut dimmed = EngineHost::new();
    dimmed.reload(&project, 0).expect("引擎必须能建起来");
    dimmed.set_monitor_dim(true);

    // C4：设备回调路径（`render_callback` ⇒ `process_quantum`，监听级生效）。
    let base = render_via_callback(&mut plain, QUANTA);
    let dim_frames = render_via_callback(&mut dimmed, QUANTA);
    let audible = base.iter().step_by(2).filter(|v| v.abs() > 1e-6).count();
    let changed = base
        .iter()
        .zip(dim_frames.iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[control-room C4] 基线可听帧 {audible}；DIM 改动帧 {changed} / {}",
        base.len()
    );
    assert!(
        audible > 1_000,
        "C4 基线必须真的出声（可听帧 {audible}），否则比对是空转"
    );
    assert!(
        changed > 1_000,
        "C4 设备回调路径上 DIM 必须真的改动输出（改动帧 {changed}）"
    );
    for (index, (b, d)) in base.iter().zip(dim_frames.iter()).enumerate() {
        assert_eq!(
            d.to_bits(),
            (b * 0.1f32).to_bits(),
            "C4 第 {index} 帧：DIM 输出必须逐位 == 基线×0.1"
        );
    }

    // C5：同一对引擎（内部状态已同步推进了 QUANTA 个量子）再经离线渲染
    // （`render_into` ⇒ 旁通监听级）⇒ DIM 开的那台与基线逐位相同。
    let mut base_offline = vec![0.0f32; 128 * 2 * QUANTA];
    let mut dim_offline = vec![0.0f32; 128 * 2 * QUANTA];
    assert_eq!(
        plain.render_into(&mut base_offline, 2, QUANTA as u64),
        QUANTA as u64
    );
    assert_eq!(
        dimmed.render_into(&mut dim_offline, 2, QUANTA as u64),
        QUANTA as u64
    );
    assert_eq!(
        base_offline.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        dim_offline.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "C5 离线渲染时 DIM 开的那台必须与基线逐位相同（旁通）"
    );
    // 反向腿：离线渲染本身不是静音（否则 C5 的"相同"是空转）。
    let offline_audible = base_offline.iter().filter(|v| v.abs() > 1e-6).count();
    assert!(
        offline_audible > 1_000,
        "C5 离线渲染必须真的出声（可听帧 {offline_audible}），否则旁通断言是空转"
    );
}

/// C6：DIM 开着 `reload` ⇒ 换代后 DIM 仍是开的。
#[test]
fn reload_preserves_the_monitor_flags() {
    let (live, engine, project) = live_window();
    let ui = live.ui();
    ui.set_control_room_dim(true);
    ui.invoke_control_room_toggle_dim();
    assert!(engine.borrow().monitor_controls().dim());
    engine
        .borrow_mut()
        .reload(&project, 0)
        .expect("reload 必须成功");
    assert!(
        engine.borrow().monitor_controls().dim(),
        "C6 DIM 开着 reload ⇒ 换代后 DIM 仍须是开的（⛔ 不静默归零）"
    );
}
