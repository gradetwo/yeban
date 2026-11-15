//! 重构 UI（Aether）的功能接线判据。
//!
//! 新 UI 的 `MainWindow` 与 main 版接线层通过"契约兼容面"对接：数据数组
//! (`tracks` / `track-names` / `notes` …) 两版同名，`apply_view` 写入后组件树直接
//! 消费；少数不同名的显示态由 [`yeban_app::host::sync_aether_display`] 同步。
//! 本文件钉住这两条，以及"新 UI 的控件回调不是静默的"。
//!
//! 真实 `.yeban` 工程（17 轨 / 2867 音符 / 10 段落）的容器导入另由
//! `yeban-app --headless --open <file>` 的批处理出口验证（不把 120MB 资产带进仓库）。

use slint::Model;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Size, UiTestPort};

fn build_rig(view: &ViewState) -> LivePort<yeban_app::ui::MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(view, &scene)
    })
    .expect("LivePort 必须成功装配 MainWindow")
}

/// 契约面（旧接线写入）的显示态必须镜像到新 UI 的显示属性。
#[test]
fn sync_aether_display_mirrors_contract_state() {
    let port = build_rig(&ViewState::demo());
    let ui = port.ui();

    ui.set_playing(true);
    ui.set_recording(true);
    ui.set_cycle_enabled(true);
    ui.set_timecode("002.03.04".into());
    ui.set_bpm_display("132.00".into());
    ui.set_master_meter_level(0.72);

    host::sync_aether_display(ui);

    assert!(ui.get_is_playing(), "playing -> is-playing");
    assert!(ui.get_is_recording(), "recording -> is-recording");
    assert!(ui.get_is_looping(), "cycle-enabled -> is-looping");
    assert_eq!(ui.get_timecode_text().as_str(), "002.03.04");
    assert_eq!(ui.get_bpm_val(), 132, "bpm-display -> bpm-val");
    assert!(
        (ui.get_master_meter_l() - 0.72).abs() < 1e-4,
        "master-meter-level -> master-meter-l"
    );
}

/// 工程投影必须驱动新 UI 的同名数据数组（导入后看得见轨道/片段/音符）。
#[test]
fn apply_view_drives_new_ui_data_arrays() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let port = build_rig(&view);
    let ui = port.ui();

    host::apply_view(ui, &view, 1920.0, 0.0);

    assert!(
        !view.tracks.is_empty(),
        "参照工程必须有音轨, 否则本判据是空绿"
    );
    assert_eq!(
        ui.get_tracks().row_count(),
        view.tracks.len(),
        "新 UI 的 tracks 必须等于投影轨道数"
    );
    assert_eq!(
        ui.get_track_names().row_count(),
        view.tracks.len(),
        "新 UI 的 track-names 必须与投影等长"
    );
    assert_eq!(
        ui.get_track_mutes().row_count(),
        view.tracks.len(),
        "混音开关数组必须与投影等长"
    );
}

/// 新 UI 独有的轨道回调必须真的产生状态变化（不是弹完 toast 什么都不做）。
#[test]
fn new_ui_track_callback_is_not_silent() {
    let port = build_rig(&ViewState::demo());
    let ui = port.ui();
    let before = ui
        .get_tracks()
        .row_data(0)
        .expect("演示视图必须至少有一条轨道")
        .muted;

    ui.invoke_track_toggle_mute(1);

    let after = ui.get_tracks().row_data(0).expect("轨道仍在").muted;
    assert_ne!(before, after, "点击静音必须翻转该轨的 muted");
}

/// 走带与工具栏修复：空格/播放/录音不再双触发抵消，画笔工具映射到 Tool::Pencil(2)。
#[test]
fn transport_and_tool_mapping_do_not_double_toggle() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let undo = std::rc::Rc::new(yeban_app::undo::UndoPort::new(
        yeban_app::undo::UndoSession::open(
            "<wiring-test>",
            "yeban-app",
            project.clone(),
            1_700_000_000_000,
        )
        .expect("撤销会话必须能打开"),
    ));
    let production = host::ProductionLoop::start(&project, 0).expect("引擎重建");
    let engine = production.engine_handle();
    let view_cell = std::rc::Rc::new(std::cell::RefCell::new(view.clone()));
    let project_rc = std::rc::Rc::new(project.clone());

    let port = build_rig(&view);
    let ui = port.ui();

    host::wire_transport(
        ui,
        std::rc::Rc::clone(&engine),
        Some(std::rc::Rc::clone(&project_rc)),
        Some(std::rc::Rc::clone(&undo)),
    );
    host::wire_keys(
        ui,
        std::rc::Rc::new(std::cell::RefCell::new(
            yeban_app::input::InputContext::default(),
        )),
        Some(std::rc::Rc::clone(&undo)),
        Some(std::rc::Rc::clone(&engine)),
        Some(std::rc::Rc::clone(&project_rc)),
    );
    host::wire_tool_select(ui, Some(std::rc::Rc::clone(&undo)));
    host::wire_aether_actions(
        ui,
        std::rc::Rc::clone(&undo),
        std::rc::Rc::clone(&engine),
        view_cell,
    );

    // ProductionLoop::start 默认起播；产品启动会先 stop() 停在起点
    let _ = engine.borrow_mut().stop();
    assert!(!engine.borrow().transport().state.is_running());
    let log_before = engine.borrow().transport_journal().len();

    // 真实点击播放（TransportHeaderBar -> root.toggle-play()）只应触发 1 次，启动走带
    ui.invoke_toggle_play();
    assert!(
        engine.borrow().transport().state.is_running(),
        "播放回调必须启动引擎（不能被二次触发抵消）"
    );
    assert_eq!(
        engine.borrow().transport_journal().len(),
        log_before + 1,
        "一次播放触发只能写 1 条引擎命令"
    );

    // 键盘空格通过 FocusScope::key_pressed -> trigger-shortcut 也只触发 1 次（Play -> Stop）
    port.window().key_press(slint::platform::Key::Space);
    assert!(
        !engine.borrow().transport().state.is_running(),
        "空格键必须只触发 1 次走带切换（由播放切回停止）"
    );
    assert_eq!(
        engine.borrow().transport_journal().len(),
        log_before + 2,
        "空格键只能再写 1 条引擎命令"
    );

    // "draw" 映射到 Pencil (2) 而不是 Velocity (4)，"b" 键可双向切换并同步 active-tool-name
    ui.invoke_edit_tool_selected("draw".into());
    assert_eq!(
        ui.get_active_tool(),
        2,
        "\"draw\" 必须映射到 Tool::Pencil (2)"
    );
    assert_eq!(ui.get_active_tool_name().as_str(), "draw");
    port.window().type_char('b');
    assert_eq!(ui.get_active_tool(), 1, "B 键应将铅笔切回选区 (1)");
    assert_eq!(ui.get_active_tool_name().as_str(), "pointer");

    // 循环快捷键 C 同步更新 cycle-enabled 与 is-looping
    let loop_before = ui.get_is_looping();
    port.window().type_char('c');
    assert_eq!(ui.get_cycle_enabled(), !loop_before);
    assert_eq!(ui.get_is_looping(), !loop_before);
}

/// 主菜单使用窗口级 Slint 原生 `MenuBar`（macOS / Windows 等支持原生菜单栏的系统由宿主放入系统/窗口菜单栏，
/// 无原生系统菜单栏时回退到窗口顶部 `MenuBarImpl`），不再放在 `TransportHeaderBar` 内；
/// 各区域自带的局部菜单（编排头、控制台头、左侧检查器、卷帘右键）保留在各自区域，且菜单命令分发仍真改状态。
#[test]
fn native_menu_bar_replaces_transport_menu_strip_and_keeps_local_menus() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use slint::{LogicalPosition, platform::PointerEventButton};
    use yeban_app::undo::{UndoPort, UndoSession};

    let app_slint = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.slint"))
        .expect("读取 ui/app.slint");
    let transport_slint = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/ui/transport_bar.slint"
    ))
    .expect("读取 ui/transport_bar.slint");
    let arrangement_slint =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/arrangement.slint"))
            .expect("读取 ui/arrangement.slint");
    let lower_dock_slint =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/lower_dock.slint"))
            .expect("读取 ui/lower_dock.slint");
    let left_panel_slint =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/left_panel.slint"))
            .expect("读取 ui/left_panel.slint");
    let piano_roll_slint = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/ui/console/piano_roll.slint"
    ))
    .expect("读取 ui/console/piano_roll.slint");

    // 1. MainWindow declares native Window-level MenuBar with all 7 menus and no in-window top menu overlay.
    assert!(
        app_slint.contains("MenuBar {"),
        "MainWindow 必须声明 Slint 原生 Window 级 MenuBar"
    );
    for title in [
        "title: \"文件\";",
        "title: \"编辑\";",
        "title: \"轨道\";",
        "title: \"走带\";",
        "title: \"混音\";",
        "title: \"视图\";",
        "title: \"诊断\";",
    ] {
        assert!(
            app_slint.contains(title),
            "MainWindow MenuBar 必须包含主菜单: {title}"
        );
    }
    assert!(
        !app_slint.contains("active-top-menu"),
        "app.slint 不应保留旧的窗口内自定义顶部菜单覆盖层"
    );

    // 2. TransportHeaderBar no longer houses the main application menu bar strip.
    assert!(
        !transport_slint.contains("toggle-top-menu")
            && !transport_slint.contains("active-top-menu")
            && !transport_slint.contains("execute-menu-command")
            && !transport_slint.contains("id: \"file\""),
        "TransportHeaderBar 内不应再绘制主菜单栏条目"
    );

    // 3. Local region menus remain in their respective region components.
    assert!(arrangement_slint.contains("active-local-menu"));
    assert!(lower_dock_slint.contains("dock-instrument-menu-open"));
    assert!(left_panel_slint.contains("show-instrument-menu"));
    assert!(piano_roll_slint.contains("context-menu-popup"));

    // 4. Live window: Tier-1 renders cleanly with MenuBar and menu commands execute real state changes.
    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("工程投影");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<menu-bar-test>",
            "yeban-app",
            project.clone(),
            1_700_000_000_000,
        )
        .expect("撤销会话"),
    ));
    let prod = host::ProductionLoop::without_engine();
    let engine = prod.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));

    let mut port = build_rig(&view);
    let ui = port.ui();
    host::wire_track_strip(ui, &undo);
    host::wire_aether_actions(ui, Rc::clone(&undo), engine, view_cell);

    let baseline = port.window().capture().expect("默认帧渲染");
    // In Tier-1 (no native OS menu bar), clicking the fallback MenuBarItem ("文件" at top-left, 40px tall bar)
    // opens a popup without filling y=650, and Escape closes it cleanly.
    port.window().pointer_move(LogicalPosition::new(22.0, 20.0));
    port.window()
        .pointer_down(LogicalPosition::new(22.0, 20.0), PointerEventButton::Left);
    port.window()
        .pointer_up(LogicalPosition::new(22.0, 20.0), PointerEventButton::Left);
    let with_menu_open = port.window().capture().expect("打开原生 MenuBar 下拉帧");
    let mut upper_diff = 0usize;
    for py in 45..260 {
        for px in 10..240 {
            if baseline.pixel(px, py) != with_menu_open.pixel(px, py) {
                upper_diff += 1;
            }
        }
    }
    assert!(
        upper_diff > 100,
        "点击回退 MenuBar“文件”项必须在下拉区真实绘出菜单 (upper_diff={upper_diff})"
    );
    assert_eq!(
        baseline.pixel(40, 650),
        with_menu_open.pixel(40, 650),
        "MenuBar 下拉菜单不得竖直拉满至 y=650"
    );
    port.window().key_press(slint::platform::Key::Escape);

    let tracks_before = undo.try_project().unwrap().tracks.len();
    ui.invoke_command_execute("track-add-midi".into());
    assert_eq!(
        undo.try_project().unwrap().tracks.len(),
        tracks_before + 1,
        "MenuBar 命令 track-add-midi 必须真实新增一条 MIDI 轨"
    );

    let hotspots_before = ui.get_show_hotspots();
    ui.invoke_command_execute("toggle-hotspots".into());
    assert_ne!(
        ui.get_show_hotspots(),
        hotspots_before,
        "MenuBar 命令 toggle-hotspots 必须切换状态"
    );

    ui.invoke_command_execute("tab-mixer".into());
    assert_eq!(ui.get_console_tab(), 1, "tab-mixer 必须切到混音台");

    // toggle-chord-lane from MenuBar/local menu genuinely toggles show_chord_track
    assert!(!ui.get_show_chord_track());
    ui.invoke_command_execute("toggle-chord-lane".into());
    assert!(
        ui.get_show_chord_track(),
        "toggle-chord-lane 必须真实切换和弦轨显隐"
    );
    ui.invoke_command_execute("toggle-chord-lane".into());
    assert!(!ui.get_show_chord_track());

    // Modals inside key-handler must fit inside window bounds even with Tier-1 fallback 40px MenuBarImpl
    ui.set_export_wav_open(true);
    port.refresh_tree(None).expect("刷新导出弹窗控件树");
    let export_bounds = port
        .tree()
        .find_by_id("export-wav-dialog")
        .expect("export-wav-dialog 必须在树里")
        .bounds
        .expect("export-wav-dialog 必须有几何");
    assert!(
        export_bounds.y >= 0
            && export_bounds.y + i32::try_from(export_bounds.height).unwrap() <= 800,
        "回退 MenuBar 下导出模态底部不得越出窗口高度 800: {export_bounds:?}"
    );
    port.ui().set_export_wav_open(false);
}

#[test]
fn mixer_faders_and_pans_drag_smoothly_without_undo_spam() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use slint::platform::PointerEventButton;
    use slint::{LogicalPosition, Model};
    use yeban_app::bridge::ViewState;
    use yeban_app::host;
    use yeban_app::host::ProductionLoop;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{ActionOutcome, UiAction, UndoPort, UndoSession};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    assert!(view.tracks.len() >= 2, "测试需要至少两条非主总线轨道");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<aether-mixer-smooth>",
            "yeban-app",
            project.clone(),
            1_700_000_000_000,
        )
        .expect("撤销会话必须能打开"),
    ));
    let production = ProductionLoop::without_engine();
    let engine = production.engine_handle();
    let view_cell = Rc::new(RefCell::new(view.clone()));
    let project_rc = Rc::new(project.clone());

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口必须装配成功");
    let ui = live.ui();
    let input = Rc::new(RefCell::new(yeban_app::input::InputContext::default()));
    host::wire_input(ui, Rc::clone(&input));
    host::wire_transport(
        ui,
        Rc::clone(&engine),
        Some(Rc::clone(&project_rc)),
        Some(Rc::clone(&undo)),
    );
    host::wire_view_callbacks(ui);
    host::wire_mixer_edit(ui, &undo);
    host::wire_track_select(ui, &undo);
    host::wire_aether_actions(ui, Rc::clone(&undo), engine, view_cell);

    // 1. 轨道音量推子: grab -> 4 次平滑 drag -> release -> 1 次原子撤销 -> 双击复位
    let track0_id: yeban_model::EntityId = ui
        .get_track_ids()
        .row_data(0)
        .expect("第 0 轨 id")
        .as_str()
        .parse()
        .expect("合法 ULID");
    let vol_initial = ui
        .get_track_volume_fractions()
        .row_data(0)
        .expect("track_volume_fractions[0]");
    let db_initial = undo.try_project().unwrap().tracks[&track0_id].volume_db;

    ui.invoke_mixer_fader_grab(0, 0.0);
    let mut seen_vols = Vec::new();
    for step in 1..=4 {
        // 向下拖动像素 (+8 px/步): 88 px 行程对应 66 dB，故每步下降 6 dB
        ui.invoke_mixer_fader_drag(0, (step as f32) * 8.0);
        let cur_frac = ui.get_track_volume_fractions().row_data(0).unwrap();
        let cur_trk_db = ui.get_tracks().row_data(0).unwrap().volume_db;
        seen_vols.push((cur_frac, cur_trk_db));
        assert!(
            !undo.display().can_undo,
            "推子拖动第 {step} 步期间不得产生撤销记录"
        );
        assert!(
            (undo.try_project().unwrap().tracks[&track0_id].volume_db - db_initial).abs() < 1e-4,
            "推子拖动第 {step} 步期间不得提前写工程"
        );
    }
    assert!(
        seen_vols[0].0 < vol_initial,
        "向下拖动后首步必须小于起点 {vol_initial}: {seen_vols:?}"
    );
    for win in seen_vols.windows(2) {
        assert!(
            win[1].0 < win[0].0 && win[1].1 < win[0].1,
            "track_volume_fractions 与 tracks[].volume_db 必须同步单调变化: {seen_vols:?}"
        );
    }

    ui.invoke_mixer_fader_release(0);
    assert!(undo.display().can_undo, "推子松手必须恰好提交 1 次撤销记录");
    let db_after_release = undo.try_project().unwrap().tracks[&track0_id].volume_db;
    assert!(db_after_release < db_initial - 1.0);
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));
    assert!(!undo.display().can_undo, "全部拖动只应占 1 步撤销历史");
    assert!(
        (undo.try_project().unwrap().tracks[&track0_id].volume_db - db_initial).abs() < 1e-4,
        "撤销一次必须完整恢复音量"
    );

    ui.invoke_mixer_fader_reset(0);
    assert!(
        undo.try_project().unwrap().tracks[&track0_id]
            .volume_db
            .abs()
            < 1e-4,
        "双击推子复位必须归零 (0 dB)"
    );
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));

    // 2. 轨道声像: grab -> 4 次平滑 drag -> release -> 1 次原子撤销 -> 双击复位
    let track1_id: yeban_model::EntityId = ui
        .get_track_ids()
        .row_data(1)
        .expect("第 1 轨 id")
        .as_str()
        .parse()
        .expect("合法 ULID");
    let pan_initial = ui
        .get_track_pan_values()
        .row_data(1)
        .expect("track_pan_values[1]");
    let model_pan_initial = undo.try_project().unwrap().tracks[&track1_id].pan;

    ui.invoke_mixer_pan_grab(1, 0.0);
    let mut seen_pans = Vec::new();
    for step in 1..=4 {
        // 向右拖动像素 (+4.5 px/步): 36 px 行程对应 [-1, +1]
        ui.invoke_mixer_pan_drag(1, (step as f32) * 4.5);
        let pv = ui.get_track_pan_values().row_data(1).unwrap();
        let ps = ui.get_track_pans().row_data(1).unwrap().to_string();
        let ts = ui.get_tracks().row_data(1).unwrap().pan.to_string();
        assert_eq!(ps, ts, "track_pans 与 tracks[].pan 在拖动期必须同步");
        seen_pans.push(pv);
        assert!(
            !undo.display().can_undo,
            "声像拖动第 {step} 步期间不得产生撤销记录"
        );
    }
    assert!(
        seen_pans[0] > pan_initial,
        "向右拖动声像后首步必须大于起点 {pan_initial}: {seen_pans:?}"
    );
    for win in seen_pans.windows(2) {
        assert!(
            win[1] > win[0],
            "track_pan_values 必须单调平滑递增: {seen_pans:?}"
        );
    }
    ui.invoke_mixer_pan_release(1);
    assert!(undo.display().can_undo, "声像松手必须恰提交 1 次撤销记录");
    let model_pan_after = undo.try_project().unwrap().tracks[&track1_id].pan;
    assert!(model_pan_after > model_pan_initial + 0.3);
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));
    assert!(!undo.display().can_undo, "声像拖动只应占 1 步撤销历史");
    // 先拖到非零再测双击复位 (避免原值为 0.0 时合理跳过提交)
    ui.invoke_mixer_pan_grab(1, 0.0);
    ui.invoke_mixer_pan_drag(1, 18.0);
    ui.invoke_mixer_pan_release(1);
    assert!(undo.try_project().unwrap().tracks[&track1_id].pan.abs() > 0.3);
    ui.invoke_mixer_pan_reset(1);
    assert!(
        undo.try_project().unwrap().tracks[&track1_id].pan.abs() < 1e-4,
        "双击声像必须居中归零"
    );
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));
    assert!(!undo.display().can_undo);

    // 3. 主总线回调 grab -> 4 次 drag -> release -> 复位
    let _master_frac_0 = ui.get_master_volume_fraction();
    ui.invoke_mixer_master_fader_grab(0.0);
    for step in 1..=4 {
        ui.invoke_mixer_master_fader_drag((step as f32) * 8.0);
        assert!(!undo.display().can_undo, "主总线拖动中不得产生撤销记录");
    }
    let master_disp_mid = ui.get_master_volume_display().to_string();
    assert!(
        master_disp_mid.starts_with('-'),
        "主总线拉低后读数应为负 dB: {master_disp_mid}"
    );
    ui.invoke_mixer_master_fader_release();
    assert!(undo.display().can_undo, "主总线松手提交 1 次撤销");
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));
    assert!(!undo.display().can_undo);

    // 4. 真实 Tier1Window 指针事件在 Mixer 第 0 轨推子上平滑拖动: pointer_down -> 4x pointer_move -> pointer_up
    ui.set_show_left_inspector(false);
    ui.set_show_control_room(false);
    ui.set_ai_drawer_open(false);
    ui.set_show_lower_dock(true);
    ui.set_console_maximized(true);
    ui.invoke_apply_dock_tab("mixer".into());
    let _ = live.window().capture().expect("布局渲染帧");

    let track0_db_before_ptr = undo.try_project().unwrap().tracks[&track0_id].volume_db;
    let track0_frac_start = ui.get_track_volume_fractions().row_data(0).unwrap();

    // 第 0 轨通道条宽 82px；在 Tier-1 (顶部含 40px MenuBarImpl) 最大化下，88px 推子轨迹约在 x=32..60, y=346..434
    let start_x = 46.0_f32;
    let start_y = 370.0_f32;
    live.window()
        .pointer_move(LogicalPosition::new(start_x, start_y));
    live.window().pointer_down(
        LogicalPosition::new(start_x, start_y),
        PointerEventButton::Left,
    );

    let mut dragged_fracs = Vec::new();
    for step in 1..=4 {
        let next_y = start_y + (step as f32) * 8.0;
        live.window()
            .pointer_move(LogicalPosition::new(start_x, next_y));
        dragged_fracs.push(ui.get_track_volume_fractions().row_data(0).unwrap());
        assert!(
            !undo.display().can_undo,
            "真实指针拖动第 {step} 步不得写撤销栈"
        );
    }
    assert!(
        dragged_fracs[0] < track0_frac_start,
        "向下拖动真实第 0 轨推子必须平滑降低 track_volume_fractions[0]: start={track0_frac_start}, steps={dragged_fracs:?}"
    );
    for win in dragged_fracs.windows(2) {
        assert!(
            win[1] < win[0],
            "每次真实 pointer_move 必须连续推进推子分度: {dragged_fracs:?}"
        );
    }
    live.window().pointer_up(
        LogicalPosition::new(start_x, start_y + 32.0),
        PointerEventButton::Left,
    );
    assert!(
        undo.display().can_undo,
        "真实 pointer_up 必须提交 1 次撤销记录"
    );
    let track0_db_after_ptr = undo.try_project().unwrap().tracks[&track0_id].volume_db;
    assert!(
        track0_db_after_ptr < track0_db_before_ptr - 1.0,
        "第 0 轨工程 volume_db 必须更新: {track0_db_before_ptr} -> {track0_db_after_ptr}"
    );
    assert!(matches!(
        undo.perform(UiAction::Undo),
        ActionOutcome::Changed { .. }
    ));
    assert!(
        !undo.display().can_undo,
        "真实指针拖动必须只占 1 步撤销记录"
    );
}
