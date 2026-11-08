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
    let (_tier1, ui) = window();
    // `app.slint` 的 `SessionView` 是 `visible: !root.arrangement-view`，而
    // `MainWindow.arrangement-view` 默认 `true` ⇒ 不显式切过去，那两格不进运行时树。
    ui.set_arrangement_view(false);
    let tree = tree_from_element_root(&ui, None).expect("运行时控件树");

    // 期望源 = **同一条 MainWindow 属性的运行时 getter**，⛔ 不复制 `.slint` 默认值
    // （`status_bar_accessible_values.rs` 第一版把默认值硬编码成期望 ⇒ 投影后假红）。
    let scene_name = ui.get_current_scene_name().to_string();
    let header = if scene_name.is_empty() {
        "场景".to_owned()
    } else {
        format!("场景 · {scene_name}")
    };

    for (id, expected) in [
        ("transport-timecode", ui.get_timecode().to_string()),
        ("scene-launch-column-header", header),
        (
            "session-scene-launch-status",
            ui.get_scene_launch_status().to_string(),
        ),
    ] {
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("控件树里没有可寻址节点 `{id}`"));
        assert_eq!(
            node.value.as_deref(),
            Some(expected.as_str()),
            "`{id}` 的无障碍值必须等于运行时可见文本 `{expected:?}`（屏读器与 `ui/tree` 读同一串字节）"
        );
    }
}

/// ② 声明面（⛔ 不开窗口）：三格各有一条绑定、与可见文本同源、静态名字不许退化。
#[test]
fn every_target_readout_binds_the_same_property_as_its_visible_text() {
    let transport = include_str!("../ui/transport.slint");
    let session = include_str!("../ui/workspace/session_view.slint");

    for (source, name, binding) in [
        (
            transport,
            "ui/transport.slint",
            "accessible-value: root.timecode;",
        ),
        (
            session,
            "ui/workspace/session_view.slint",
            "accessible-value: root.scene-launch-status;",
        ),
        (
            session,
            "ui/workspace/session_view.slint",
            "accessible-value: root.current-scene-name == \"\" ? \"场景\" : \"场景 · \" + root.current-scene-name;",
        ),
    ] {
        assert!(
            source.contains(binding),
            "`{name}` 必须含绑定 `{binding}` —— 少一条就有一格读不出内容"
        );
    }

    // 同源：值绑的表达式与可见 `text` 逐字相同 ⇒ ⛔ 不是第二份真相源。
    for (source, name, binding) in [
        (transport, "ui/transport.slint", "text: root.timecode;"),
        (
            session,
            "ui/workspace/session_view.slint",
            "text: root.scene-launch-status;",
        ),
        (
            session,
            "ui/workspace/session_view.slint",
            "text: root.current-scene-name == \"\" ? \"场景\" : \"场景 · \" + root.current-scene-name;",
        ),
    ] {
        assert!(
            source.contains(binding),
            "`{name}` 的可见文本 `{binding}` 必须与无障碍值同源"
        );
    }

    // 静态名字不许被改成「绑动态内容」的旧做法（那样补 value 会重复朗读）。
    for (source, label) in [
        (transport, "accessible-label: \"主时间码\";"),
        (session, "accessible-label: \"场景一键激发列\";"),
        (session, "accessible-label: \"场景启动状态\";"),
    ] {
        assert!(source.contains(label), "静态名字 `{label}` 不许退化");
    }

    // 只加属性、不新增可寻址节点 ⇒ 控件树清单与 5 张 golden 基准一位不动。
    assert_eq!(
        transport.matches("accessible-id:").count(),
        15,
        "transport.slint 的可寻址节点数必须是 15（本票 0 新增）"
    );
    assert_eq!(
        session.matches("accessible-id:").count(),
        9,
        "session_view.slint 的可寻址节点数必须是 9（本票 0 新增）"
    );
    assert_eq!(
        transport.matches("accessible-value:").count(),
        4,
        "transport.slint 的无障碍值必须是 4 条：既有 3 条 ＋ 本票 1 条"
    );
    assert_eq!(
        session.matches("accessible-value:").count(),
        2,
        "session_view.slint 的无障碍值必须是 2 条：既有 0 条 ＋ 本票 2 条"
    );
}
