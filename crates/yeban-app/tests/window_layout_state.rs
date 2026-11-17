//! 窗口布局持久化判据（line/ui-window-state）。
//!
//! 守四件事：
//! - 判据 ①（应用↔快照往返）：`host::apply_window_layout` 写进去的布局，
//!   `host::snapshot_window_layout` 逐字段读得回来（面板量全量＋窗口
//!   尺寸/最大化态）。窗口**位置**不在本判据的断言面内：测试端口是
//!   软件窗口、没有位置概念（真实桌面走 winit，位置由
//!   `settings::WindowGeometry` 的存取往返在 settings 单测里钉住）。
//! - 判据 ②（恢复默认全链路）：预置一份「语言 fr-FR ＋ 已存布局」的
//!   设置文件 → 按 `main.rs` 的启动顺序读出并应用 → 经真实回调接线
//!   `invoke_reset_window_layout` 触发菜单动作 ⇒ 界面回 .slint 初值、
//!   文件里布局被清掉、**语言保留**。
//! - 判据 ③（.slint 初值 ↔ settings 默认值/钳位界对账）：五条面板尺寸
//!   的默认值与 min/max 在 `ui/app.slint` 与 `settings.rs` 两处各写了
//!   一份（.slint 是声明面、settings 是读回钳位面），任一处单独改动
//!   ⇒ 本判据红。外加两个显隐初值与左栏标签页初值的字面对账。
//!
//! 设置文件本身的存取往返、旧文件兼容与逐字段降级在
//! `src/settings.rs` 的单测里钉住，不在这里重复。

use yeban_app::bridge::ViewState;
use yeban_app::cli::Language;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::settings::{self, Settings, WindowGeometry, WindowLayout};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

fn build_rig(view: &ViewState) -> LivePort<yeban_app::ui::MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(view, &scene)
    })
    .expect("LivePort 必须成功装配 MainWindow")
}

/// 一份与默认值处处不同的布局（每个字段都离开初值，往返才验得住）。
fn distinctive_layout() -> WindowLayout {
    WindowLayout {
        window: Some(WindowGeometry {
            x: 30,
            y: 40,
            width: 1500,
            height: 880,
            maximized: true,
        }),
        left_panel_width: 300.0,
        clip_matrix_width: 200.0,
        control_room_width: 260.0,
        lower_dock_height: 400.0,
        track_headers_width: 200.0,
        show_left_inspector: false,
        show_clip_matrix: true,
        show_lower_dock: false,
        show_control_room: true,
        left_panel_tab: "browser".to_owned(),
        console_tab: 2,
        console_maximized: true,
        sidebar_collapsed: true,
    }
}

/// 判据 ①：应用一份布局后，快照逐字段等于应用值（窗口位置除外，
/// 理由见文件头）。
#[test]
fn applied_layout_roundtrips_through_snapshot() {
    let view = ViewState::demo();
    let port = build_rig(&view);
    let ui = port.ui();
    let layout = distinctive_layout();
    host::apply_window_layout(ui, &layout);
    let snapshot = host::snapshot_window_layout(ui);
    assert_eq!(snapshot.left_panel_width, layout.left_panel_width);
    assert_eq!(snapshot.clip_matrix_width, layout.clip_matrix_width);
    assert_eq!(snapshot.control_room_width, layout.control_room_width);
    assert_eq!(snapshot.lower_dock_height, layout.lower_dock_height);
    assert_eq!(snapshot.track_headers_width, layout.track_headers_width);
    assert_eq!(snapshot.show_left_inspector, layout.show_left_inspector);
    assert_eq!(snapshot.show_clip_matrix, layout.show_clip_matrix);
    assert_eq!(snapshot.show_lower_dock, layout.show_lower_dock);
    assert_eq!(snapshot.show_control_room, layout.show_control_room);
    assert_eq!(snapshot.left_panel_tab, layout.left_panel_tab);
    assert_eq!(snapshot.console_tab, layout.console_tab);
    assert_eq!(snapshot.console_maximized, layout.console_maximized);
    assert_eq!(snapshot.sidebar_collapsed, layout.sidebar_collapsed);
    let window = snapshot.window.expect("快照必须带窗口几何");
    let applied = layout.window.expect("应用的布局带窗口几何");
    assert_eq!(window.width, applied.width, "窗口宽（物理像素）须往返");
    assert_eq!(window.height, applied.height, "窗口高（物理像素）须往返");
    assert_eq!(window.maximized, applied.maximized, "最大化态须往返");
}

/// 判据 ②：恢复默认布局的全链路 —— 界面回初值、已存布局清掉、
/// 语言保留（见文件头）。
#[test]
fn reset_restores_defaults_clears_stored_layout_and_keeps_language() {
    let dir = std::env::temp_dir().join(format!("yeban-window-layout-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("settings.json");
    // SAFETY: 本测试二进制里只有本条测试读写该环境变量（判据 ① 不碰
    // 设置文件），且设置路径在每次 load/save 时现读，作用域限于本测试；
    // 结束时摘除。写法与 examples/i18n_shots.rs 的既有先例一致。
    unsafe {
        std::env::set_var(settings::SETTINGS_PATH_ENV, &path);
    }
    // 预置：语言 fr-FR ＋ 一份已存布局（模拟上次退出留下的文件）。
    settings::save(&Settings {
        language: Some(Language::FrFr),
        layout: Some(distinctive_layout()),
    })
    .expect("预置设置文件必须写成功");

    let view = ViewState::demo();
    let port = build_rig(&view);
    let ui = port.ui();
    // 模拟 main.rs 的启动恢复：读设置 → 有布局就应用。
    let stored = settings::load();
    let stored_layout = stored.layout.expect("预置布局须读得出来");
    host::apply_window_layout(ui, &stored_layout);
    assert_eq!(
        ui.get_left_panel_width(),
        300.0,
        "启动恢复须先把已存布局应用上（否则下面验的是空转）"
    );

    // 经真实接线触发菜单动作（回调在 build_main_window 里挂好）。
    ui.invoke_reset_window_layout();

    let defaults = WindowLayout::default();
    assert_eq!(ui.get_left_panel_width(), defaults.left_panel_width);
    assert_eq!(ui.get_lower_dock_height(), defaults.lower_dock_height);
    assert_eq!(ui.get_show_clip_matrix(), defaults.show_clip_matrix);
    assert_eq!(
        ui.get_left_panel_tab().as_str(),
        defaults.left_panel_tab.as_str()
    );
    assert_eq!(ui.get_console_tab(), defaults.console_tab);
    assert_eq!(ui.get_sidebar_collapsed(), defaults.sidebar_collapsed);

    let after = settings::load();
    assert_eq!(
        after.language,
        Some(Language::FrFr),
        "恢复默认布局不许动语言选择"
    );
    assert_eq!(after.layout, None, "已存布局须被清掉（下次启动不再恢复）");

    // SAFETY: 与上面的 set_var 配对摘除，理由同上。
    unsafe {
        std::env::remove_var(settings::SETTINGS_PATH_ENV);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 从一行 .slint 声明里按出现顺序抽出所有 `<数字>px` 的数字部分。
fn px_numbers(line: &str) -> Vec<f32> {
    let bytes = line.as_bytes();
    let mut numbers = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if line[i..].starts_with("px") {
                numbers.push(line[start..i].parse().expect("px 前的数字部分必须可解析"));
            }
        } else {
            i += 1;
        }
    }
    numbers
}

/// 取一条尺寸属性的声明行，返回（默认值，min，max）——声明行形状是
/// `in-out property <length> NAME: Dpx; // [min: Apx, max: Bpx]`。
fn length_declaration(source: &str, name: &str) -> (f32, f32, f32) {
    let needle = format!("property <length> {name}:");
    let line = source
        .lines()
        .find(|line| line.contains(&needle))
        .unwrap_or_else(|| panic!("app.slint 必须声明 {name}"));
    let numbers = px_numbers(line);
    assert_eq!(
        numbers.len(),
        3,
        "{name} 声明行须含 默认/min/max 三个 px 数: {line}"
    );
    (numbers[0], numbers[1], numbers[2])
}

/// 判据 ③：.slint 初值与 settings 的默认值/钳位界逐条对账（见文件头）。
#[test]
fn slint_initial_values_match_settings_defaults_and_ranges() {
    const APP_SLINT: &str = include_str!("../ui/app.slint");
    let defaults = WindowLayout::default();
    let cases = [
        (
            "left-panel-width",
            defaults.left_panel_width,
            settings::LEFT_PANEL_WIDTH_RANGE,
        ),
        (
            "clip-matrix-width",
            defaults.clip_matrix_width,
            settings::CLIP_MATRIX_WIDTH_RANGE,
        ),
        (
            "control-room-width",
            defaults.control_room_width,
            settings::CONTROL_ROOM_WIDTH_RANGE,
        ),
        (
            "lower-dock-height",
            defaults.lower_dock_height,
            settings::LOWER_DOCK_HEIGHT_RANGE,
        ),
        (
            "track-headers-width",
            defaults.track_headers_width,
            settings::TRACK_HEADERS_WIDTH_RANGE,
        ),
    ];
    for (name, default, range) in cases {
        let (slint_default, slint_min, slint_max) = length_declaration(APP_SLINT, name);
        assert_eq!(
            slint_default, default,
            "{name} 默认值在 .slint 与 settings 两处不一致"
        );
        assert_eq!(
            (slint_min, slint_max),
            range,
            "{name} 钳位界在 .slint 与 settings 两处不一致"
        );
    }
    assert!(
        APP_SLINT.contains("property <bool> show-left-inspector: true;"),
        "show-left-inspector 初值须为 true（settings 默认值与之对账）"
    );
    assert!(
        APP_SLINT.contains("property <bool> show-lower-dock: true;"),
        "show-lower-dock 初值须为 true（settings 默认值与之对账）"
    );
    assert!(
        APP_SLINT.contains("property <string> left-panel-tab: \"inspector\";"),
        "left-panel-tab 初值须为 inspector（settings 默认值与之对账）"
    );
}
