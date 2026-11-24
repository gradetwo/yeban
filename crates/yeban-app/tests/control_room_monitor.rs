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
use yeban_ui_test_port::inspect::{find_by_accessible_id, property_of};
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
    let (live, engine, project, _undo, _view_cell) = live_window_parts();
    (live, engine, project)
}

/// 与 [`live_window`] 同一次装配，但同时交回换工程要用的撤销端口与活投影。
#[allow(clippy::type_complexity)]
fn live_window_parts() -> (
    LivePort<yeban_app::ui::MainWindow>,
    Rc<RefCell<EngineHost>>,
    YebanProjectV1,
    Rc<UndoPort>,
    Rc<RefCell<ViewState>>,
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

    // 生产同款装配：混音台写入面先接（MUTE/SOLO 的真实处理器），最后
    // `wire_aether_actions` 用 `on_*` 覆盖语义接管新回调
    // （控制室三开关的生产处理器就注册在这里）。
    host::wire_mixer_edit(ui, &undo);
    host::wire_aether_actions(
        ui,
        Rc::clone(&undo),
        Rc::clone(&engine),
        Rc::clone(&view_cell),
    );
    (live, engine, project, undo, view_cell)
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

/// 下坞 MONO 按钮的**实际**亮灭读数（`[UI-TEST-001]` 语义属性）。
///
/// 读的是按钮元素的 `accessible-checked`（它本身绑在 `master-mono` 上），⛔ 不读像素、
/// 也不借用主窗口的 `control-room-mono` —— 判据要证的就是"按钮自己的状态"。
fn dock_mono_checked(window: &yeban_app::ui::MainWindow) -> bool {
    let handle = find_by_accessible_id(window, "dock-master-mono").expect(
        "下坞 MONO 按钮必须在控件树里（accessible-id = dock-master-mono；入口不在可见分支或 ID 改了）",
    );
    match property_of(&handle, "checked")
        .expect("按钮必须声明 accessible-checked")
        .as_str()
    {
        "true" => true,
        "false" => false,
        other => panic!("checked 只能取 true/false，实得 {other}"),
    }
}

/// 点**真实下坞按钮**（无障碍默认动作 = 鼠标点击的同一落点）。
fn press_dock_mono(window: &yeban_app::ui::MainWindow) {
    find_by_accessible_id(window, "dock-master-mono")
        .expect("下坞 MONO 按钮必须在控件树里")
        .invoke_accessible_default_action();
}

/// C7（T-FIX-215）：下坞 MONO 与控制室是**同一份**状态，按钮亮灭不得与实际监听相反。
///
/// 修前：下坞有独立 `master-mono`，先用控制室开 MONO ⇒ 下坞按钮仍显示灭；点它会把按钮
/// 点亮、却把实际监听关掉（灯与声音相反）。
#[test]
fn dock_mono_button_shares_the_control_room_state() {
    let (live, engine, _project) = live_window();
    let ui = live.ui();
    // 下坞 MONO 在混音台页（`if active-tab == "mixer" || "stacked"` 分支里）⇒ 先切到那一页，
    // 否则按钮根本不在控件树里（切页是用户看得见的一步，不是测试专用行为）。
    ui.invoke_apply_dock_tab("mixer".into());
    assert!(
        !engine.borrow().monitor_controls().mono(),
        "初始 MONO 标志须为关"
    );
    assert!(
        !dock_mono_checked(ui),
        "初始下坞 MONO 按钮必须是灭（与控制室同一状态）"
    );

    // ① 从**控制室**开 MONO ⇒ 下坞按钮必须同时亮。
    ui.set_control_room_mono(true);
    ui.invoke_control_room_toggle_mono();
    assert!(
        engine.borrow().monitor_controls().mono(),
        "控制室开 MONO ⇒ 引擎 MONO 标志必须为开"
    );
    assert!(
        dock_mono_checked(ui),
        "控制室开 MONO ⇒ 下坞按钮必须亮（修前它是灭的，灯与声音相反）"
    );

    // ② 点**下坞按钮** ⇒ 引擎 MONO 必须真翻一次（开→关），两个界面布尔相等。
    press_dock_mono(ui);
    assert!(
        !engine.borrow().monitor_controls().mono(),
        "点下坞 MONO ⇒ 引擎 MONO 必须真翻一次（开→关）"
    );
    assert!(
        !dock_mono_checked(ui),
        "点完下坞按钮必须灭（⛔ 不能出现“按钮亮、监听灭”）"
    );
    assert_eq!(
        dock_mono_checked(ui),
        ui.get_control_room_mono(),
        "两个界面表示的布尔必须相等"
    );

    // ③ 再点一次 ⇒ 翻回开；仍相等（钉住"只翻一次"，⛔ 翻两次 = 不变）。
    press_dock_mono(ui);
    assert!(
        engine.borrow().monitor_controls().mono(),
        "第二次点下坞 MONO 必须翻回开（若本地又翻一次属性，这里会不变）"
    );
    assert!(dock_mono_checked(ui), "第二次点击后下坞按钮必须亮");
    assert_eq!(dock_mono_checked(ui), ui.get_control_room_mono());
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

/// C8（T-FIX-270）：换工程后 MONO / 轨道 MUTE 仍须落到**新**引擎 / **新** Op 端口。
///
/// 复现 after 闭包的真实顺序：新工程装配出新的 `engine` / `UndoPort` / `ViewState`
/// 后，`wire_view_callbacks` 先重跑旧回调层，最后由
/// `rewire_aether_actions_after_project_switch` 按启动顺序补回 aether 层。
/// 删掉最后这一步，aether 的 handler 仍闭包着换工程前的旧句柄 ⇒ 本判据的 MONO
/// 只动旧引擎、MUTE 只写旧端口，对新句柄的两条断言都会变红。
#[test]
fn project_switch_rebinds_aether_over_ui_only_callbacks() {
    let (live, old_engine, project, old_undo, _old_view) = live_window_parts();
    let ui = live.ui();

    // 换工程 after：先按新工程装配出新的引擎 / 撤销端口 / 投影。
    let new_undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<control-room-monitor-after-switch>",
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("换工程后的撤销会话必须能打开"),
    ));
    let new_view = Rc::new(RefCell::new(
        ViewState::from_project(&project).expect("工程必须能投影"),
    ));
    let production = ProductionLoop::start(&project, 0).expect("换工程后的引擎重建");
    let new_engine = production.engine_handle();

    // 旧回调层重跑（生产同款的 `wire_view_callbacks`）。
    host::wire_view_callbacks(ui);
    // 修复：按启动顺序最后重装 aether 层，覆盖回真实 handler。
    // 红对照：`T_FIX_270_RED=1` 时故意跳过这一步，模拟修前行为。
    if std::env::var_os("T_FIX_270_RED").is_none() {
        host::rewire_aether_actions_after_project_switch(
            ui,
            Rc::clone(&new_undo),
            Rc::clone(&new_engine),
            Rc::clone(&new_view),
        );
    }

    // MONO：换工程后仍须写进**新**引擎的监听标志。
    assert!(
        !new_engine.borrow().monitor_controls().mono(),
        "C8 新引擎初始 MONO 标志须为关"
    );
    ui.set_control_room_mono(true);
    ui.invoke_control_room_toggle_mono();
    assert!(
        new_engine.borrow().monitor_controls().mono(),
        "C8 换工程后开 MONO ⇒ 新引擎 MONO 标志必须真变（旧行为只动旧引擎）"
    );
    assert!(
        !old_engine.borrow().monitor_controls().mono(),
        "C8 换工程后 MONO 不得再写旧引擎"
    );

    // 轨道 MUTE：换工程后仍须向**新**端口提交 Op，而不是只写旧端口 / 翻 UI 数组。
    let before: Vec<bool> = new_undo
        .try_project()
        .expect("工程")
        .tracks
        .values()
        .map(|track| track.mute)
        .collect();
    ui.invoke_track_toggle_mute(1);
    let after: Vec<bool> = new_undo
        .try_project()
        .expect("工程")
        .tracks
        .values()
        .map(|track| track.mute)
        .collect();
    assert_ne!(
        before, after,
        "C8 换工程后轨道 MUTE ⇒ 新端口的工程 mute 集合必须变化（旧行为只写旧端口）"
    );
    assert_eq!(
        old_undo.try_project().expect("工程").tracks,
        project.tracks,
        "C8 换工程后轨道 MUTE 不得再写旧端口"
    );
}
