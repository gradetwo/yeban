//! 浮动提示系统（Floating Tooltip System）的接线与运行时契约。
//!
//! ## 判据
//! ① 悬停控件触发气泡更新，提取目标控件绝对坐标与尺寸
//! ② 悬停时就地切换 `show-tooltips` 开关，提示状态无缝响应且无需重新晃动鼠标
//! ③ 指针离开目标控件时，提示文本与激活状态立即清除
//! ④ 相邻控件直接切换时，坐标与提示文本正确同步
//! ⑤ 顶层气泡声明位于 MainWindow 根层级（`z: 10000`），具备动态窗口边缘防溢出裁剪与底部翻转保护

use slint::ComponentHandle;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::Theme;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Size, UiTestPort};

fn build_rig() -> LivePort<yeban_app::ui::MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("LivePort 必须成功装配");
    // 与真实启动路径（main.rs 紧跟主题后调 apply_language）一致：首帧语言
    // 默认 zh-CN，覆盖 Slint 按系统 locale 的自动选择；否则英文系统上
    // @tr 提示文案为英文，本判据硬编码的中文提示串永远找不到
    //（win/mac CI 实证：4 项同以 floating_tooltip.rs:46 失败）。
    host::apply_language(port.ui(), yeban_app::cli::Language::ZhCn);
    port
}

/// 在顶部 `TransportHeaderBar` 按钮行内定位目标提示按钮，返回 `(x, y)` 指针坐标与预期的按钮顶部 `y`。
/// - 原生系统菜单栏平台（macOS/Windows，`supports_native_menu_bar == true`）下，`TransportHeaderBar` 位于窗口 `y = 0..48`，按钮中心 `y = 24.0`，顶部 `12.0`。
/// - 非原生回退菜单栏平台（Linux/Headless `Tier1Platform`）下，顶部 `MenuBar` 占位 `40px`，`TransportHeaderBar` 下移至 `y = 40..88`，按钮中心 `y = 64.0`，顶部 `52.0`。
fn find_button_pos(
    port: &mut LivePort<yeban_app::ui::MainWindow>,
    expected_tip: &str,
) -> (f64, f64, f32) {
    for (scan_y, expected_top_y) in [(24.0_f64, 12.0_f32), (64.0_f64, 52.0_f32)] {
        let mut x = 40.0_f64;
        while x <= 600.0 {
            port.dispatch_pointer_move(x, scan_y)
                .expect("指针移动必须送达");
            if port.ui().global::<Theme<'_>>().get_hover_tip().as_str() == expected_tip {
                return (x, scan_y, expected_top_y);
            }
            x += 8.0;
        }
    }
    panic!("未在顶部栏找到目标提示按钮: {expected_tip}");
}

#[test]
fn hover_shows_tooltip_with_exact_button_coordinates() {
    let mut port = build_rig();

    // 移动指针至左侧检查器按钮
    let (insp_x, insp_y, expected_top_y) = find_button_pos(&mut port, "检查器与音色库 [I]");

    let theme = port.ui().global::<Theme<'_>>();
    assert_eq!(
        theme.get_hover_tip(),
        "检查器与音色库 [I]",
        "悬停按钮必须报告对应的提示文案"
    );
    assert!(
        theme.get_has_hover_tip(),
        "悬停时 has-hover-tip 必须为 true"
    );
    assert!(
        (theme.get_hover_tip_x() - insp_x as f32).abs() <= 20.0,
        "提示横坐标必须位于目标按钮中央附近"
    );
    assert_eq!(
        theme.get_hover_tip_y(),
        expected_top_y,
        "提示纵坐标必须对齐按钮绝对位置"
    );
    assert_eq!(
        theme.get_hover_tip_target_h(),
        24.0,
        "提示必须感知目标控件高度"
    );

    // 验证实际渲染的气泡确实紧贴按钮下方（预期 top_y + target_h(24) + 6），而非被回退菜单栏再次下推 40px
    let with_tip = port.window().capture().expect("开启提示截图");
    port.ui().global::<Theme<'_>>().set_show_tooltips(false);
    let without_tip = port.window().capture().expect("关闭提示截图");
    let button_row_bottom = (expected_top_y + 26.0) as u32;
    let mut min_changed_y = usize::MAX;
    let mut max_changed_y = 0usize;
    for y in button_row_bottom..800u32 {
        let mut row_diff = false;
        for x in 0..1280u32 {
            if with_tip.pixel(x, y) != without_tip.pixel(x, y) {
                row_diff = true;
                break;
            }
        }
        if row_diff {
            min_changed_y = min_changed_y.min(y as usize);
            max_changed_y = max_changed_y.max(y as usize);
        }
    }
    let expected_bubble_top = (expected_top_y + 24.0 + 6.0) as usize;
    assert!(
        min_changed_y >= expected_bubble_top.saturating_sub(2)
            && min_changed_y <= expected_bubble_top + 2
            && max_changed_y >= expected_bubble_top + 18
            && max_changed_y <= expected_bubble_top + 24,
        "渲染出的气泡必须紧贴按钮下方 (预期 top≈{expected_bubble_top}, 实测 diff y={min_changed_y}..={max_changed_y}, scan_y={insp_y})"
    );
}

#[test]
fn toggle_tooltips_while_hovering_is_reactive() {
    let mut port = build_rig();

    // 1. 悬停按钮
    let _ = find_button_pos(&mut port, "检查器与音色库 [I]");
    let theme = port.ui().global::<Theme<'_>>();
    assert_eq!(theme.get_hover_tip(), "检查器与音色库 [I]");
    assert!(theme.get_show_tooltips());

    // 2. 就地关闭提示开关 (模拟快捷键 T 或点击 [T])
    theme.set_show_tooltips(false);
    assert!(!theme.get_show_tooltips());
    // 目标标识依然被稳定追踪，但顶层呈现被开关关闭
    assert_eq!(theme.get_hover_tip(), "检查器与音色库 [I]");
    assert!(theme.get_has_hover_tip());

    // 3. 再次就地开启提示开关
    theme.set_show_tooltips(true);
    assert!(theme.get_show_tooltips());
    assert_eq!(
        theme.get_hover_tip(),
        "检查器与音色库 [I]",
        "重新开启开关后，无需重新划入鼠标即可立即保持悬停提示"
    );
    assert!(theme.get_has_hover_tip());
}

#[test]
fn cursor_exit_clears_hover_state() {
    let mut port = build_rig();

    // 悬停
    let _ = find_button_pos(&mut port, "检查器与音色库 [I]");
    assert_eq!(
        port.ui().global::<Theme<'_>>().get_hover_tip(),
        "检查器与音色库 [I]"
    );

    // 移入空白无按钮区域
    port.dispatch_pointer_move(500.0, 200.0)
        .expect("指针移动必须送达");
    let theme = port.ui().global::<Theme<'_>>();
    assert_eq!(theme.get_hover_tip(), "", "移出控件后提示文案必须清空");
    assert!(
        !theme.get_has_hover_tip(),
        "移出控件后 has-hover-tip 必须置为 false"
    );
}

#[test]
fn direct_switch_between_buttons_updates_coordinates() {
    let mut port = build_rig();

    let (clip_x, clip_y, _) = find_button_pos(&mut port, "Clip 激发矩阵 [Tab]");
    port.dispatch_pointer_move(500.0, 200.0)
        .expect("指针移动必须送达");

    // 悬停按钮 1
    let _ = find_button_pos(&mut port, "检查器与音色库 [I]");
    let theme = port.ui().global::<Theme<'_>>();
    assert_eq!(theme.get_hover_tip(), "检查器与音色库 [I]");
    let tip1_x = theme.get_hover_tip_x();

    // 直接滑动至相邻按钮 2
    port.dispatch_pointer_move(clip_x, clip_y)
        .expect("指针移动必须送达");
    let theme = port.ui().global::<Theme<'_>>();
    assert_eq!(theme.get_hover_tip(), "Clip 激发矩阵 [Tab]");
    let tip2_x = theme.get_hover_tip_x();
    assert!(tip2_x > tip1_x, "直接切换相邻按钮必须实时更新横向绝对位置");
}

#[test]
fn top_level_overlay_declaration_guarantees() {
    let app_slint = include_str!("../ui/app.slint");
    assert!(
        app_slint.contains("z: 10000;"),
        "全局气泡必须拥有顶级 z-index (10000)"
    );
    assert!(
        app_slint.contains("Math.max(8px, Math.min(root.width - self.width - 8px, Theme.hover-tip-x - self.width / 2))"),
        "气泡横向必须经过双向窗口边缘安全钳制"
    );
    assert!(
        app_slint.contains("Theme.hover-tip-y + Theme.hover-tip-target-h + 34px > root.height"),
        "气泡纵向必须包含底部溢出翻转条件"
    );
    assert!(
        app_slint.contains("Math.max(6px, Theme.hover-tip-y - self.height - 6px)"),
        "气泡翻转至上方时必须预留顶部安全边距"
    );
}
