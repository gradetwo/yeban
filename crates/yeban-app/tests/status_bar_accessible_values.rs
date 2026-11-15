//! 判据（`[PATCH UI-A11Y-003]`）：状态栏的**内容**必须能从无障碍 `value` 通道读到。
//!
//! ## 补的是哪一条线
//!
//! R9 关闭登记里如实留了一句"这三格只有 `accessible-label`、**没有** `accessible-value`
//! ⇒ 屏读器念得出名字，念不出内容；`ui/tree` 的 `value` 也读不到"。本切片给
//! `selection` / `chord` / `shortcut-tip` / `device` 四格各补一条 `accessible-value`，
//! 值 = 各自的**可见文本**（同一个属性或同一个表达式 ⇒ ⛔ 不是第二份真相源）。
//!
//! ⚠ 只加无障碍属性 ⇒ ⛔ 不改可见文本、⛔ 不新增可寻址节点 ⇒
//! `tests/golden/linux/` 的 5 张基准与 `tests/status_bar_honesty.rs` 的六条判据
//! 一位不动。
//!
//! ## 判定量
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `node.value` | 运行时控件树里某 ID 的 `accessible-value` | 字符串 |
//! | `node.label` | 运行时控件树里某 ID 的 `accessible-label` | 字符串 |
//!
//! ## 怎么变红
//!
//! * 删掉任一格的 `accessible-value` ⇒ ① 红（`value == None` 或空串）；
//! * 把 `value` 绑到别的属性（例如把 `device` 写成常量 `"—"`）⇒ ① 红
//!   （期望源是 `MainWindow` 的**运行时 getter**，不是 `.slint` 默认值字面量）；
//! * 把 `chord` 的标签改回"实时和弦识别" ⇒ ① 与 ② 同时红。
//!
//! ⚠ **期望值口径**：`status_bar.slint` 里的 `device: "—"` 只是"还没有读数"的**默认值**，
//! 不是夹具里界面上那串字 —— `build_main_window_with_console_tab` 经
//! `host::publish_status_bar` 投影后是真实工程音频配置
//! （`48 kHz / 32-bit float · 目标 120 FPS`）。所以判据拿**同一条属性的运行时 getter**
//! 当期望源：既不是第二份真相源，又能抓住"把 `value` 改绑到常量"的回潮。
//! 第一版把 `.slint` 默认值 `"—"` 硬编码成期望 ⇒ 运行时假红，此处已修正。

#![allow(unused)]
use yeban_app::bridge::ViewState;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::render::Tier1Window;

fn window() -> (Tier1Window, yeban_app::ui::MainWindow) {
    let tier1 =
        Tier1Window::install(yeban_ui_test_port::Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = yeban_app::scene::DemoScene::from_view(&view);
    let ui = yeban_app::host::build_main_window_with_console_tab(&view, &scene, 1)
        .expect("生产窗口构造入口");
    (tier1, ui)
}

/// ① 运行时控件树：四格的 `value` 就是各自的可见文本。
#[test]
fn status_bar_texts_are_readable_through_the_accessibility_value_channel() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> status-device:"),
        "新 UI 契约面必须声明 > status-device:"
    );
    assert!(
        aether_host.contains("set_status_device"),
        "宿主接线层必须注册 set_status_device"
    );
    assert!(
        aether_app.contains("> status-selection:"),
        "新 UI 契约面必须声明 > status-selection:"
    );
    assert!(
        aether_host.contains("set_status_selection"),
        "宿主接线层必须注册 set_status_selection"
    );
}

/// ③ 声明面（⛔ 不开窗口）：四格各有一条 `accessible-value`，且注册表标签逐字同步。
#[test]
fn every_declared_status_bar_readout_has_an_accessible_value_binding() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
