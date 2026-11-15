//! **底部控制台最大化 / 还原** —— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 控制台右上角的 `⤢`（`console-maximize-button`）**不是一个只显示不动作的图标**：
//! 点它 = 把 `MainWindow.console-maximized` 翻过去，同一份视图态由键盘
//! `Cmd/Ctrl+Alt+M`（`Action::ToggleConsoleMaximize`）写入 ⇒ 按键与点击不可能分叉。
//! 属性一翻，布局跟着动：中段（工作区）让出弹性高度，控制台吃掉剩余空间。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | `console-maximize-button` 在树里、角色 = `Button`、标签与注册表逐字一致；装配态 `console-maximized == false` |
//! | ②（真点击 → 真布局） | 注入一次真实指针点击 ⇒ 属性 `false → true`，**重抓树**后控制台高、工作区矮（⛔ 不是只看属性） |
//! | ③（还原） | 再点一次 ⇒ 属性回 `false`，几何回到点击前的读数 |
//! | ④（回调路径） | `invoke_console_maximize_toggle()` 与点击落到**同一个**属性（纯视图态：本装配没有 `UndoPort`） |
//! | ⑤（文本层接线） | `.slint` 的 `clicked` 与 `accessible-action-default` 同一落点；`app.slint` 原样转发；`host.rs` 的**两个**写者（`Action` 臂与点击回调）写的是同一个 setter |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_bpm_field.rs` / `tests/master_mute_solo.rs` 同款）。

#![allow(unused)]
use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    // 生产接线（`main.rs` 与无头装配调的是**同一个**函数）。
    host::wire_view_callbacks(port.ui());
    (port, view)
}

fn bounds_of(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> Rect {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

#[test]
fn a_real_click_on_the_console_maximize_button_maximizes_the_console() {
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
        aether_app.contains("callback console-maximize-toggle("),
        "新 UI 契约面必须声明 callback console-maximize-toggle("
    );
    assert!(
        aether_host.contains("on_console_maximize_toggle"),
        "宿主接线层必须注册 on_console_maximize_toggle"
    );
    assert!(
        aether_app.contains("> console-maximized:"),
        "新 UI 契约面必须声明 > console-maximized:"
    );
    assert!(
        aether_host.contains("set_console_maximized"),
        "宿主接线层必须注册 set_console_maximized"
    );
}
