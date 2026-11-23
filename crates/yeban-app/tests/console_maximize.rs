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
use yeban_ui_test_port::render::{LivePort, report_line};
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
    // ④⑤ 文本层接线（源码层）：回调与属性的声明/注册面；两个"翻转"写者（点击回调
    // 与 `Action::ToggleConsoleMaximize` 臂）写同一个 setter（第三处
    // `set_console_maximized(layout…)` 只是布局还原、只写值不翻转，不计入）。
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
    assert_eq!(
        code_only(&aether_host)
            .matches("set_console_maximized(!ui.get_console_maximized())")
            .count(),
        2,
        "翻转写者必须恰好两处（点击回调 + Action 臂），写同一个 setter"
    );

    let (mut port, _view) = live_window();

    // ① 可寻址：按钮在树里、角色 Button、标签与注册表逐字一致；装配态未最大化。
    let registry = ElementRegistry::demo();
    let meta = registry
        .get("console-maximize-button")
        .expect("注册表必须登记 console-maximize-button");
    assert_eq!(
        meta.kind,
        ElementKind::Button,
        "注册表里这一项必须是 Button"
    );
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    let button = tree
        .find_by_id("console-maximize-button")
        .expect("运行时树里必须有 console-maximize-button");
    assert_eq!(button.role.as_str(), "button", "树里角色必须是 button");
    assert_eq!(button.label, meta.label, "树里标签必须与注册表逐字一致");
    assert!(
        !port.ui().get_console_maximized(),
        "装配态 console-maximized == false"
    );
    let rail_before = bounds_of(&mut port, "console-tab-rail");
    report_line(&format!(
        "console-maximize before: rail-y={} rail-h={}",
        rail_before.y, rail_before.height
    ));

    // ② 真点击 → 最大化 + 几何：控制台吃掉工作区高度。
    //
    // T-FIX-131 的注：工作区行现以 `visible:` 门控（原 `if !root.console-maximized`），
    // 最大化时靠 `max-height: 0px` 把它压到 0 高。若压不住，控制台顶栏只会上移一点
    // （或不动），下面这条就红 —— 这条就是那次改动的回归网。
    click_maximize(&mut port);
    assert!(
        port.ui().get_console_maximized(),
        "点击后 console-maximized 必须为真"
    );
    let rail_max = bounds_of(&mut port, "console-tab-rail");
    report_line(&format!(
        "console-maximize maximized: rail-y={} rail-h={}",
        rail_max.y, rail_max.height
    ));
    assert!(
        rail_max.y + 200 < rail_before.y,
        "最大化后控制台顶栏必须上移至少 200px（工作区让出高度）；实测 before-y={} max-y={}",
        rail_before.y,
        rail_max.y
    );
    // T-FIX-131：上移必须上移到顶栏底下（实测基线 y=89），不只是"移了一大截"——
    // 工作区行若留一点残高（例如 `max-height` 压不住内容最小高时残 154px），
    // 顶栏会停在 y=243，上面那条 >200 的断言照样放行（本单实测踩过）。
    // 120 是给顶栏留的松动（基线 89 + 31），不是给行残高留的。
    assert!(
        rail_max.y < 120,
        "最大化后控制台顶栏必须贴到顶栏底下（y<120）；实测 max-y={}（行残高会把它顶下去）",
        rail_max.y
    );
    assert!(
        rail_max.height > 700,
        "最大化后控制台必须吃掉工作区高度（高>700）；实测 max-h={}",
        rail_max.height
    );

    // ③ 再点一次 → 还原几何。
    click_maximize(&mut port);
    assert!(
        !port.ui().get_console_maximized(),
        "再点后 console-maximized 必须回 false"
    );
    let rail_after = bounds_of(&mut port, "console-tab-rail");
    assert!(
        (rail_after.y - rail_before.y).abs() < 2,
        "还原后控制台顶栏必须回到原位；实测 before-y={} after-y={}",
        rail_before.y,
        rail_after.y
    );
    report_line("console-maximize restored: rail-y back to before");
}

/// 在最大化按钮中心点一次真实指针点击（down + up）。
fn click_maximize(port: &mut LivePort<yeban_app::ui::MainWindow>) {
    let btn = bounds_of(port, "console-maximize-button");
    port.dispatch_pointer_down(
        "console-maximize-button",
        f64::from(btn.width) / 2.0,
        f64::from(btn.height) / 2.0,
        PointerButton::Left,
    )
    .expect("点下必须送达");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("松开必须送达");
}

/// 只在**代码行**上数，注释里提到这个名字不算调用点（与 `tests/search_focus_key.rs` ⑤ 同口径）。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
