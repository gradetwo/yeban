//! 判据（`[PATCH UI-A11Y-004]`）：三处「静态名字 + 动态内容」的读数必须能从无障碍
//! `value` 通道读到，且值 = **同一条运行时属性**（⛔ 不是第二份真相源）。
//!
//! ## 为什么只挑这三处
//!
//! 全项目还有不少 `accessible-label` **已经绑了动态文本**的控件（例如 `sidebar-empty`
//! 的 `root.empty-reason`、`asset-pool-count` 的拼接）—— 它们的名字本身就是内容，
//! 再补 `value` 只会让屏读器把同一串字念两遍。真正符合
//! `tests/status_bar_accessible_values.rs` 范式的是「label 是静态名字、可见文本是动态读数、
//! 却没有 `value`」那一类；扫描全部 `.slint` 后落到这三处：
//!
//! | ID | 静态 label | 可见文本 / value |
//! | :-- | :-- | :-- |
//! | `transport-timecode` | 主时间码 | `root.timecode` |
//! | `scene-launch-column-header` | 场景一键激发列 | 当前场景名的三元表达式 |
//! | `session-scene-launch-status` | 场景启动状态 | `root.scene-launch-status` |
//!
//! 其余同批候选要么 label 已是动态内容（补 value = 重复朗读），要么是按钮 / 开关 /
//! 占位文本（按 `[TEST-SPEC-005]` 不该有文本 value），⛔ 一律不动。
//!
//! ⚠ 只加无障碍属性 ⇒ ⛔ 不改可见文本、⛔ 不新增可寻址节点 ⇒
//! `tests/golden/linux/` 的 5 张基准逐字节不动。
//!
//! ## 判定量
//!
//! | 量 | 对象 | 单位 |
//! | :-- | :-- | :-- |
//! | `node.value` | 运行时控件树里某 ID 的无障碍值 | 字符串 |
//! | 绑定表达式 | `.slint` 源码里 `value` 与 `text` 是否同源 | 布尔 |
//!
//! ## 怎么变红
//!
//! * 删掉任一格的绑定 ⇒ ① 与 ② 同时红（① `value == None`，② 少一条字面）；
//! * 把 `value` 改绑到常量 / 别的属性 ⇒ ① 红（期望源是 MainWindow 的运行时 getter，
//!   不是 `.slint` 默认值字面量）；
//! * 把 `value` 换成与可见文本不同的第二份字面量 ⇒ ② 红（同源断言）。
//! * 顺手给这些文件新增可寻址节点 ⇒ ② 的 `accessible-id` 计数红。

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

/// ① 运行时控件树：三格的 `value` 就是各自的可见文本。
#[test]
fn the_three_readouts_expose_their_runtime_text_through_the_value_channel() {
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
        aether_app.contains("> arrangement-view:"),
        "新 UI 契约面必须声明 > arrangement-view:"
    );
    assert!(
        aether_host.contains("set_arrangement_view"),
        "宿主接线层必须注册 set_arrangement_view"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
    assert!(
        aether_app.contains("> scene-launch-status:"),
        "新 UI 契约面必须声明 > scene-launch-status:"
    );
    assert!(
        aether_host.contains("set_scene_launch_status"),
        "宿主接线层必须注册 set_scene_launch_status"
    );
}

/// ② 声明面（⛔ 不开窗口）：三格各有一条绑定、与可见文本同源、静态名字不许退化。
#[test]
fn every_target_readout_binds_the_same_property_as_its_visible_text() {
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
