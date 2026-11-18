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
    // 双语 i18n（line/ui-i18n）后菜单标题在源码里写作 `title: @tr("…");`
    // —— 针随表示形式更新，判据意图（七个主菜单必须在）不变。
    for title in [
        "title: @tr(\"文件\");",
        "title: @tr(\"编辑\");",
        "title: @tr(\"轨道\");",
        "title: @tr(\"走带\");",
        "title: @tr(\"混音\");",
        "title: @tr(\"视图\");",
        "title: @tr(\"诊断\");",
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

fn is_close_rgb(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual[0].abs_diff(expected[0]) <= 3
        && actual[1].abs_diff(expected[1]) <= 3
        && actual[2].abs_diff(expected[2]) <= 3
}

/// 回归判据: 编曲视口与钢琴卷帘播放头在 tick 0 (启动 / 归零)、标尺定位、横向滚动及缩放下
/// 必须与轨道/小节/片段起点逐像素对齐，且向左侧滚动越界时被严格裁剪不侵入控制头/琴键区。
#[test]
fn playhead_aligns_with_timeline_origin_across_seek_rewind_scroll_and_zoom() {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use yeban_app::bridge::{NewTrackKind, track_create_ops};
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_model::EntityId;
    use yeban_model::music::MidiNote;
    use yeban_model::ops::Op;
    use yeban_model::project::{ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig};

    let base = yeban_model::samples::default_project();
    let seed_ops = track_create_ops(&base, NewTrackKind::Midi).expect("创建 MIDI 轨");
    let track_id = seed_ops
        .iter()
        .find_map(|op| match op {
            Op::AddTrack { track } => Some(track.id),
            _ => None,
        })
        .expect("必须返回新建轨道 ID");

    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<playhead-align>", "yeban-app", base, 1_700_000_000_000)
            .expect("撤销会话"),
    ));
    undo.commit_ops(1_700_000_000_001, "test: add track", seed_ops)
        .expect("提交新建轨道");

    let clip_id = EntityId::new();
    let placement_id = EntityId::new();
    let n0_id = EntityId::new();
    let n1_id = EntityId::new();
    let mut notes = BTreeMap::new();
    notes.insert(n0_id, MidiNote::new(n0_id, 0, 60, 960));
    notes.insert(n1_id, MidiNote::new(n1_id, 3840, 64, 960));

    let clip = ClipPoolEntry {
        id: clip_id,
        name: "Lead".into(),
        content: ClipContent::Midi { notes },
    };
    let placement = ClipPlacement {
        id: placement_id,
        clip_id,
        start_tick: 0,
        duration_ticks: 3840 * 4,
        loop_config: LoopConfig::default(),
        muted: false,
        offset_samples: None,
    };
    undo.commit_ops(
        1_700_000_000_002,
        "test: add lead clip",
        vec![
            Op::AddClip { clip },
            Op::AddClipPlacement {
                track_id,
                placement,
            },
        ],
    )
    .expect("提交测试片段");

    let project = undo.try_project().expect("权威工程");
    let production =
        host::ProductionLoop::start_with_audio_source(&project, 0, None).expect("引擎装配");
    let engine = production.engine_handle();
    engine.borrow_mut().stop();

    let view = ViewState::from_project(&project).expect("工程投影");
    let scene = DemoScene::from_view(&view);
    let live_view = Rc::new(RefCell::new(view.clone()));

    let mut live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    {
        let ui = live.ui();
        host::wire_transport(ui, Rc::clone(&engine), None, None);
        host::wire_ruler_seek(ui, Rc::clone(&engine), Rc::clone(&live_view));
        host::wire_zoom_controls(ui, &undo);
        host::wire_roll_edit(ui, &undo);
        host::wire_track_select(ui, &undo);
        host::wire_clip_scope(ui, Rc::clone(&live_view), Some(Rc::clone(&undo)));
        host::wire_aether_actions(
            ui,
            Rc::clone(&undo),
            Rc::clone(&engine),
            Rc::clone(&live_view),
        );

        ui.set_show_left_inspector(false);
        ui.set_show_clip_matrix(false);
        ui.set_show_control_room(false);
        ui.set_ai_drawer_open(false);
        ui.set_show_lower_dock(true);
        ui.invoke_apply_dock_tab("pianoroll".into());
        ui.invoke_clip_select(placement_id.to_canonical_string().into());
        host::apply_view(ui, &view, 1440.0, 0.0);
        host::apply_transport(ui, engine.borrow().transport());
        host::sync_aether_display(ui);
    }

    // 编曲播放头颜色（六版，Wang 2026-11-17 15:09 亲定）：竖线与菱形头
    // 由 Theme.accent (#387691) 改纯白 #ffffff；本测试的播放头像素探针
    // 跟随播放头颜色，只钉位置——x=245、2px 线宽、12px 内缩等几何值
    // 一律不动（六版只改了颜色，未动时间轴起点与播放头左缘几何；
    // 旧 accent 探针在六版头 CI run 44039699951 唯一一败即源于此）。
    // 重设计补丁包第四轮（Wang 2026-11-18 定判据按补丁包修改）：
    // 播放头改走 Tokens.skin-text-strong（扁平 #f4f7fa），探针色
    // 同步换钉；几何钉（x=245、2px 线宽、12px 内缩）一律不动。
    const PLAYHEAD_RGB: [u8; 3] = [0xf4, 0xf7, 0xfa];

    // 1. 初始冷启动 tick 0: 播放头与 Bar 1 / 轨道内容起点逐像素对齐，时间码 = 001.01.000
    assert_eq!(live.ui().get_playhead_ticks(), 0);
    assert_eq!(live.ui().get_playhead_px(), 0.0);
    assert_eq!(live.ui().get_timecode().as_str(), "001.01.000");
    assert_eq!(live.ui().get_timecode_text().as_str(), "001.01.000");

    let note0_id = format!("note-{}-rect", n0_id.to_canonical_string());
    let note1_id = format!("note-{}-rect", n1_id.to_canonical_string());
    let (canvas, grid0, pr_playhead0, note0_bounds) = {
        let tree = live.refresh_tree(None).expect("控件树");
        (
            tree.find_by_id("workspace-arrangement-canvas")
                .and_then(|n| n.bounds)
                .expect("编曲画布包围盒"),
            tree.find_by_id("piano-roll-grid")
                .and_then(|n| n.bounds)
                .expect("卷帘网格"),
            tree.find_by_id("piano-roll-playhead")
                .and_then(|n| n.bounds)
                .expect("卷帘播放头"),
            tree.find_by_id(&note0_id)
                .and_then(|n| n.bounds)
                .expect("tick 0 音符包围盒"),
        )
    };
    let timeline_origin_x = canvas.x + live.ui().get_track_headers_width().round() as i32 + 5;
    let ruler_probe_y = (canvas.y + 15) as u32;

    let img0 = live.window().capture().expect("帧 0");
    for y in (canvas.y + 1) as u32..(canvas.y + 54) as u32 {
        for x in (timeline_origin_x - 5)..timeline_origin_x {
            let px = img0.pixel(x as u32, y).unwrap();
            assert!(
                !is_close_rgb(px, PLAYHEAD_RGB),
                "tick 0 播放头与菱形头不得向左侵入分隔线/轨道控制头 (x={x}, y={y}, rgb={px:?})"
            );
        }
    }
    assert!(
        is_close_rgb(
            img0.pixel(timeline_origin_x as u32, ruler_probe_y).unwrap(),
            PLAYHEAD_RGB
        ),
        "tick 0 编曲播放头左侧必须正好落在时间轴起点 x={timeline_origin_x}"
    );
    assert!(
        is_close_rgb(
            img0.pixel(timeline_origin_x as u32 + 1, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "tick 0 编曲播放头 2px 线宽必须落在 [{timeline_origin_x}, {})",
        timeline_origin_x + 2
    );
    assert!(
        is_close_rgb(
            img0.pixel(timeline_origin_x as u32, (canvas.y + 62) as u32)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "tick 0 编曲播放头必须贯穿到轨道泳道起点 x={timeline_origin_x}"
    );

    assert_eq!(
        pr_playhead0.x,
        grid0.x + 12,
        "tick 0 卷帘播放头必须对齐 12px 起点内缩"
    );
    assert_eq!(
        pr_playhead0.x, note0_bounds.x,
        "tick 0 卷帘播放头必须与 start_tick=0 音符左沿完全对齐"
    );

    // 2. 点击标尺定位到 Bar 3 (tick 7680 => tpp 30 下 x = 256px)
    live.ui().invoke_ruler_seek(256.0);
    assert_eq!(live.ui().get_playhead_ticks(), 7680);
    assert_eq!(live.ui().get_playhead_px(), 256.0);
    assert_eq!(live.ui().get_timecode_text().as_str(), "003.01.000");

    let pr_playhead_seek = {
        let tree_seek = live.refresh_tree(None).expect("seek 后控件树");
        tree_seek
            .find_by_id("piano-roll-playhead")
            .and_then(|n| n.bounds)
            .expect("seek 后卷帘播放头")
    };
    let img_seek = live.window().capture().expect("seek 帧");
    assert!(
        is_close_rgb(
            img_seek
                .pixel((timeline_origin_x + 256) as u32, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "seek 到第 3 小节后编曲播放头必须移至 x={}",
        timeline_origin_x + 256
    );
    assert_eq!(
        pr_playhead_seek.x,
        grid0.x + 12 + 256,
        "seek 后卷帘播放头必须同步移动 +256px"
    );

    // 3. 点击归零 (Rewind): 播放头与时间码必须回到原点
    live.ui().invoke_transport_rewind();
    assert_eq!(live.ui().get_playhead_ticks(), 0);
    assert_eq!(live.ui().get_playhead_px(), 0.0);
    assert_eq!(live.ui().get_timecode_text().as_str(), "001.01.000");

    let pr_playhead_rew = {
        let tree_rew = live.refresh_tree(None).expect("归零后控件树");
        tree_rew
            .find_by_id("piano-roll-playhead")
            .and_then(|n| n.bounds)
            .expect("归零后卷帘播放头")
    };
    let img_rew = live.window().capture().expect("归零帧");
    assert!(
        is_close_rgb(
            img_rew
                .pixel(timeline_origin_x as u32, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "归零后编曲播放头必须精准回到时间轴起点 x={timeline_origin_x}"
    );
    assert_eq!(pr_playhead_rew.x, grid0.x + 12);

    // 4. 定位到 Bar 2 (tick 3840 => 128px) 并横向滚动 scroll_x=128px (应正好对齐时间轴左沿)
    live.ui().invoke_ruler_seek(128.0);
    assert_eq!(live.ui().get_playhead_ticks(), 3840);
    {
        let ui = live.ui();
        let fresh = host::project_with_view_state(&project, ui).expect("投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 128.0);
    }
    let (pr_playhead_scroll, note1_scroll) = {
        let tree_scroll = live.refresh_tree(None).expect("滚动后控件树");
        (
            tree_scroll
                .find_by_id("piano-roll-playhead")
                .and_then(|n| n.bounds)
                .expect("滚动后卷帘播放头"),
            tree_scroll
                .find_by_id(&note1_id)
                .and_then(|n| n.bounds)
                .expect("Bar 2 音符"),
        )
    };
    let img_scroll = live.window().capture().expect("滚动帧");
    assert!(
        is_close_rgb(
            img_scroll
                .pixel(timeline_origin_x as u32, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "Bar 2 播放头在 scroll_x=128px 时必须落在时间轴左沿 x={timeline_origin_x}"
    );
    assert_eq!(
        pr_playhead_scroll.x, note1_scroll.x,
        "横向滚动后卷帘播放头与 Bar 2 音符左沿必须逐像素重合"
    );

    // 5. 继续向右滚动越过播放头 (scroll_x=256px, 播放头在 128px): 必须被裁剪，不得泄漏进控制头
    {
        let ui = live.ui();
        let fresh = host::project_with_view_state(&project, ui).expect("投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 256.0);
    }
    let img_clipped = live.window().capture().expect("裁剪帧");
    for y in (canvas.y + 1) as u32..(canvas.y + 54) as u32 {
        for x in (timeline_origin_x - 5)..(timeline_origin_x + 4) {
            let px = img_clipped.pixel(x as u32, y).unwrap();
            assert!(
                !is_close_rgb(px, PLAYHEAD_RGB),
                "播放头滚出左视口后不得泄漏进控制头/分隔线或时间轴左沿 (x={x}, y={y}, rgb={px:?})"
            );
        }
    }

    // 6. 缩放至 tpp=60 (tick 3840 => 64px) 且 scroll_x=0: 编曲与卷帘播放头同时精确落在 +64px
    {
        let ui = live.ui();
        assert!(host::set_roll_ticks_per_pixel(ui, 60));
        let fresh = host::project_with_view_state(&project, ui).expect("缩放后投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 0.0);
    }
    assert_eq!(live.ui().get_playhead_px(), 64.0);
    let (pr_playhead_zoom, note1_zoom) = {
        let tree_zoom = live.refresh_tree(None).expect("缩放后控件树");
        (
            tree_zoom
                .find_by_id("piano-roll-playhead")
                .and_then(|n| n.bounds)
                .expect("缩放后卷帘播放头"),
            tree_zoom
                .find_by_id(&note1_id)
                .and_then(|n| n.bounds)
                .expect("缩放后 Bar 2 音符"),
        )
    };
    let img_zoom = live.window().capture().expect("缩放帧");
    assert!(
        is_close_rgb(
            img_zoom
                .pixel((timeline_origin_x + 64) as u32, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "缩放至 tpp=60 后编曲播放头必须落在 x={}",
        timeline_origin_x + 64
    );
    assert_eq!(pr_playhead_zoom.x, grid0.x + 12 + 64);
    assert_eq!(
        pr_playhead_zoom.x, note1_zoom.x,
        "缩放后卷帘播放头与 Bar 2 音符左沿必须保持逐像素重合"
    );

    // 7. 恢复缩放 tpp=30 并在 scroll_x=128px 时点击标尺 x=128px: 应加回滚动偏移定位到 Bar 3 (tick 7680)
    {
        let ui = live.ui();
        assert!(host::set_roll_ticks_per_pixel(ui, 30));
        let fresh = host::project_with_view_state(&project, ui).expect("恢复缩放投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 128.0);
        ui.invoke_ruler_seek(128.0);
        assert_eq!(
            ui.get_playhead_ticks(),
            7680,
            "横向滚动 128px 后点击标尺局部 x=128px 必须定位到 Bar 3 (tick 7680)"
        );
        assert_eq!(ui.get_timecode_text().as_str(), "003.01.000");
    }
    let img_scrolled_seek = live.window().capture().expect("滚动后定位帧");
    assert!(
        is_close_rgb(
            img_scrolled_seek
                .pixel((timeline_origin_x + 128) as u32, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "scroll_x=128 定位到 Bar 3 后播放头必须回到屏幕列 x={}",
        timeline_origin_x + 128
    );

    // 8. 切换轨道控制头到右侧 (AbletonLive 镜像布局): tick 0 播放头必须精确对齐 canvas.x，
    //    且左侧滚动越界/右侧控制头绝不出现播放头伪影
    {
        let ui = live.ui();
        ui.invoke_command_execute("track-toggle-headers-side".into());
        ui.invoke_transport_rewind();
        let fresh = host::project_with_view_state(&project, ui).expect("右置控制头投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 0.0);
        assert_eq!(ui.get_playhead_ticks(), 0);
    }
    let img_right = live.window().capture().expect("右置控制头 tick 0 帧");
    let right_origin_x = canvas.x;
    assert!(
        is_close_rgb(
            img_right
                .pixel(right_origin_x as u32, ruler_probe_y)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "右置轨道头模式下 tick 0 播放头必须对齐时间轴最左沿 x={right_origin_x}"
    );
    assert!(
        is_close_rgb(
            img_right
                .pixel(right_origin_x as u32, (canvas.y + 62) as u32)
                .unwrap(),
            PLAYHEAD_RGB
        ),
        "右置轨道头模式下 tick 0 播放头必须贯穿到轨道泳道起点 x={right_origin_x}"
    );
    let headers_w = live.ui().get_track_headers_width().round() as i32;
    let canvas_right = canvas.x + i32::try_from(canvas.width).unwrap();
    let right_timeline_end = canvas_right - headers_w - 5;
    // 泄漏判定改连续行程规则（补丁包第四轮改钉）：播放头是贯穿
    // 全高的 2px 竖线，真泄漏必在分隔线带内形成 >= 30px 的连续
    // 竖行程；和弦名 / 提示气泡等 skin-text-strong 文字字形至多
    // 约 12px 行程，不得计入泄漏。实证：右置切换后出现的提示
    // 气泡「[视图] 轨道头已切换到右侧」文字落入分隔线带，旧的
    // 逐像素扫描将其误判为播放头泄漏（帧已存档
    // aether-right-header.png 亲看确认）。
    for x in right_timeline_end..(right_timeline_end + 5) {
        let mut run = 0u32;
        let mut max_run = 0u32;
        for y in (canvas.y + 1) as u32..(canvas.y + 54) as u32 {
            if is_close_rgb(img_right.pixel(x as u32, y).unwrap(), PLAYHEAD_RGB) {
                run += 1;
                max_run = max_run.max(run);
            } else {
                run = 0;
            }
        }
        assert!(
            max_run < 30,
            "右置轨道头模式下 tick 0 播放头不得泄漏到右侧分隔线 (x={x}, 最长连续行程 {max_run}px)"
        );
    }
    for x in right_timeline_end..canvas_right {
        let px = img_right.pixel(x as u32, (canvas.y + 2) as u32).unwrap();
        assert!(
            !is_close_rgb(px, PLAYHEAD_RGB),
            "右置轨道头模式下播放头菱形顶盖不得泄漏进右置控制头顶区 (x={x}, rgb={px:?})"
        );
    }
    // 将播放头定位到超出右置时间轴右沿的位置 (45000 ticks / tpp=30 => 1500px > timeline_width)，
    // 验证右侧 5px 分隔线与右置控制头顶区完全裁剪、绝不溢出
    {
        let ui = live.ui();
        ui.invoke_ruler_seek(1500.0);
        let fresh = host::project_with_view_state(&project, ui).expect("右越界裁剪投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 0.0);
    }
    let img_right_clipped = live.window().capture().expect("右置越界裁剪帧");
    // 同上：连续行程规则（提示气泡等文字不得计入泄漏）。
    for x in right_timeline_end..(right_timeline_end + 5) {
        let mut run = 0u32;
        let mut max_run = 0u32;
        for y in (canvas.y + 1) as u32..(canvas.y + 54) as u32 {
            if is_close_rgb(img_right_clipped.pixel(x as u32, y).unwrap(), PLAYHEAD_RGB) {
                run += 1;
                max_run = max_run.max(run);
            } else {
                run = 0;
            }
        }
        assert!(
            max_run < 30,
            "播放头越过右置时间轴右沿后不得泄漏到右侧分隔线 (x={x}, 最长连续行程 {max_run}px)"
        );
    }
    for x in right_timeline_end..canvas_right {
        let px = img_right_clipped
            .pixel(x as u32, (canvas.y + 2) as u32)
            .unwrap();
        assert!(
            !is_close_rgb(px, PLAYHEAD_RGB),
            "播放头越过右置时间轴右沿后不得泄漏进右置控制头顶区 (x={x}, rgb={px:?})"
        );
    }
    live.ui()
        .invoke_command_execute("track-toggle-headers-side".into());
    live.ui().invoke_transport_rewind();

    // 9. 最大化底部停靠栏 (Lower Dock Maximized): 卷帘播放头仍与网格起点 / tick 0 音符左沿对齐
    {
        let ui = live.ui();
        let fresh = host::project_with_view_state(&project, ui).expect("最大化下坞前投影");
        *live_view.borrow_mut() = fresh.clone();
        host::apply_view(ui, &fresh, 1440.0, 0.0);
        ui.set_console_maximized(true);
        ui.invoke_transport_rewind();
    }
    let (grid_max, pr_max, note0_max) = {
        let tree_max = live.refresh_tree(None).expect("最大化后控件树");
        (
            tree_max
                .find_by_id("piano-roll-grid")
                .and_then(|n| n.bounds)
                .expect("最大化卷帘网格"),
            tree_max
                .find_by_id("piano-roll-playhead")
                .and_then(|n| n.bounds)
                .expect("最大化卷帘播放头"),
            tree_max
                .find_by_id(&note0_id)
                .and_then(|n| n.bounds)
                .expect("最大化 tick 0 音符"),
        )
    };
    assert_eq!(pr_max.x, grid_max.x + 12);
    assert_eq!(
        pr_max.x, note0_max.x,
        "下坞最大化态下卷帘播放头与 tick 0 音符左沿必须保持逐像素对齐"
    );
    // 冷启动建窗兜底: 即便首帧未拿到窗口尺寸，tick > 0 的可见音符也不应被错误裁剪为空
    let cold_ui = host::build_main_window(&view, &scene).expect("冷启动构造 MainWindow");
    assert!(
        cold_ui.get_note_ulids().row_count() >= 2,
        "冷启动默认投影必须保留 Bar 1/Bar 2 可见音符而非被 0 视口宽裁掉"
    );
}
