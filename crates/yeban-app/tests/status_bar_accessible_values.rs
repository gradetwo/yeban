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
    let (_tier1, ui) = window();
    let tree = tree_from_element_root(&ui, None).expect("运行时控件树");

    // `selection` / `device` 两格由宿主 `host::publish_status_bar` 投影成**运行时读数**。
    // ⚠ `status_bar.slint` 里的 `device: "—"` 只是"还没有读数"的**默认值**，不是界面上
    // 那串字（夹具走 `build_main_window_with_console_tab` ⇒ 投影后是真实工程音频配置
    // `48 kHz / 32-bit float · 目标 120 FPS`）。
    // ⇒ 期望源必须是**同一条 MainWindow 属性的运行时 getter**，⛔ 不复制字面量：
    //   复制出来的那份既是第二份真相源，又会在投影后立刻变成假红（本判据第一版的教训）。
    //   把任一格的 `accessible-value` 改绑到常量 / 别的属性 ⇒ getter 与 value 分叉 ⇒ 红。
    for (id, runtime_text) in [
        (
            "status-bar-selection",
            ui.get_status_selection().to_string(),
        ),
        ("status-bar-device", ui.get_status_device().to_string()),
    ] {
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("控件树里没有可寻址节点 `{id}`"));
        assert_eq!(
            node.value.as_deref(),
            Some(runtime_text.as_str()),
            "`{id}` 的 `accessible-value` 必须等于运行时可见文本 `{runtime_text:?}`（屏读器与 `ui/tree` 读同一串字节）"
        );
        assert!(
            !runtime_text.is_empty(),
            "`{id}` 的运行时读数不该是空串 —— 空串会让「读到了但没内容」变成一种假成功"
        );
    }

    // `chord` **没有**宿主写者、也没有 MainWindow 属性 ⇒ 永远停在 `.slint` 的无读数
    // 默认值 `(none)`（`status_bar_honesty.rs` 判据 ① 钉死、判据 ③ 守着"无写者"）。
    // 这一格没有 getter 可对齐，只能按固定形态逐字断言。
    let chord = tree
        .find_by_id("status-bar-chord")
        .expect("控件树里没有 `status-bar-chord`");
    assert_eq!(
        chord.value.as_deref(),
        Some("和弦 (none)"),
        "`chord` 的 `accessible-value` 必须就是那一串无读数可见文本"
    );

    // 快捷键提示：`value` 必须非空且与那一串键位承诺同源（⛔ 不硬编码整句，
    // 免得把 `d_ui_27` 的文案判据复制成第二份）。
    let tip = tree
        .find_by_id("status-bar-shortcut-tip")
        .expect("控件树里没有 `status-bar-shortcut-tip`");
    let tip_value = tip.value.as_deref().unwrap_or_default();
    assert!(
        tip_value.contains("空格") && tip_value.contains("Tab") && !tip_value.trim().is_empty(),
        "提示的 `value` 必须就是那串可见的键位承诺，实际 {tip_value:?}"
    );

    // ② 和弦格的标签必须是**如实描述**（原名"实时和弦识别"是假承诺：app 侧
    // 没有 `yeban-theory` 边、这一格没有宿主写者、永远停在 `(none)`）。
    assert_eq!(
        chord.label.trim(),
        "和弦识别（当前版本无读数）",
        "`chord` 的无障碍名必须如实说明当前版本没有和弦识别读数"
    );
}

/// ③ 声明面（⛔ 不开窗口）：四格各有一条 `accessible-value`，且注册表标签逐字同步。
#[test]
fn every_declared_status_bar_readout_has_an_accessible_value_binding() {
    let bar = include_str!("../ui/status_bar.slint");
    let elements = include_str!("../src/elements.rs");

    for binding in [
        "accessible-value: root.selection;",
        "accessible-value: \"和弦 \" + root.chord;",
        "accessible-value: root.tip;",
        "accessible-value: root.device;",
    ] {
        assert!(
            bar.contains(binding),
            "`ui/status_bar.slint` 必须含绑定 `{binding}` —— 少一条就有一格读不出内容"
        );
    }
    assert_eq!(
        bar.matches("accessible-value:").count(),
        5,
        "状态栏的 `accessible-value` 必须是 5 条：既有 `save-status` 一条 ＋ 本次补的四条"
    );
    assert!(
        elements.contains("\"和弦识别（当前版本无读数）\""),
        "`src/elements.rs` 的静态注册表标签必须与 `.slint` 逐字相同"
    );
    assert!(
        !bar.contains("accessible-label: \"实时和弦识别\""),
        "旧的假承诺标签不许回来"
    );
}
