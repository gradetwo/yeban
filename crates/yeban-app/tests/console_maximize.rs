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
    let (mut port, view) = live_window();

    // ---- ① 可寻址：节点在场、角色 / 标签与注册表一致 ----
    {
        let tree = port.refresh_tree(None).expect("重抓");
        let node = tree
            .find_by_id("console-maximize-button")
            .expect("`console-maximize-button` 必须在运行时控件树里");
        assert_eq!(
            node.role.as_str(),
            ElementKind::Button.accessible_role(),
            "最大化按钮的角色必须与注册表一致"
        );
        assert_eq!(
            node.label, "最大化 / 还原底部控制台",
            "无障碍名必须与注册表一致"
        );
        let registry = ElementRegistry::from_view(&view);
        let meta = registry
            .get("console-maximize-button")
            .expect("`console-maximize-button` 必须在装配时的注册表里（[UI-TEST-001]）");
        assert_eq!(meta.kind, ElementKind::Button);
        assert_eq!(meta.label, "最大化 / 还原底部控制台");
    }
    assert!(
        !port.ui().get_console_maximized(),
        "装配态必须是**未最大化**（默认帧与既有基准一位不变）"
    );

    let console_before = bounds_of(&mut port, "console-tab-rail");
    let canvas_before = bounds_of(&mut port, "workspace-arrangement-canvas");
    assert!(
        console_before.height > 0 && canvas_before.height > 0,
        "前提：未最大化时控制台与工作区都要有高度（否则下面的对照不成立）: \
         console={console_before:?} canvas={canvas_before:?}"
    );

    // ---- ② 真实点击：按下 → 松手（命中 `TouchArea`，不是直接调 Rust 回调） ----
    port.dispatch_pointer_down("console-maximize-button", 2.0, 2.0, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
    assert!(
        port.ui().get_console_maximized(),
        "点击之后 `console-maximized` 必须翻成 true"
    );

    let console_after = bounds_of(&mut port, "console-tab-rail");
    let canvas_after = port
        .refresh_tree(None)
        .expect("重抓")
        .find_by_id("workspace-arrangement-canvas")
        .and_then(|node| node.bounds);
    assert!(
        console_after.height > console_before.height + 200,
        "最大化之后控制台必须显著变高：{} → {}（差距不足以说明布局真的跟着属性走）",
        console_before.height,
        console_after.height
    );
    assert!(
        canvas_after.is_none_or(|bounds| bounds.height < canvas_before.height),
        "最大化之后工作区必须让出高度：{} → {:?}",
        canvas_before.height,
        canvas_after.map(|bounds| bounds.height)
    );

    // ---- ③ 再点一次：属性与几何都回原样 ----
    port.dispatch_pointer_down("console-maximize-button", 2.0, 2.0, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
    assert!(
        !port.ui().get_console_maximized(),
        "第二次点击必须还原成 false"
    );
    let console_restored = bounds_of(&mut port, "console-tab-rail");
    let canvas_restored = bounds_of(&mut port, "workspace-arrangement-canvas");
    assert_eq!(
        (console_restored.height, canvas_restored.height),
        (console_before.height, canvas_before.height),
        "还原之后的几何必须回到点击前的读数"
    );

    // ---- ④ 回调路径：与点击落到同一个属性（本装配没有 `UndoPort` ⇒ 纯视图态） ----
    port.ui().invoke_console_maximize_toggle();
    assert!(
        port.ui().get_console_maximized(),
        "`invoke_console_maximize_toggle()` 必须写**同一个** `console-maximized`"
    );
    port.ui().invoke_console_maximize_toggle();
    assert!(!port.ui().get_console_maximized(), "再调一次必须还原");

    // ---- ⑤ 文本层：点击源 / 转发 / 两个写者 ----
    let tabs = include_str!("../ui/console/console_tabs.slint");
    let button = tabs
        .split("accessible-id: \"console-maximize-button\"")
        .nth(1)
        .expect("`console_tabs.slint` 里必须有最大化按钮");
    let block = button
        .split("\n    }")
        .next()
        .expect("最大化按钮的元素块必须能切出来");
    // ⚠ 只数**带分号的调用**：上面那条注释里也提到了 `root.console-maximize-toggle()`，
    // 数裸名字会把注释算成第三个落点（CI run 40947349788 实测 left=3 / right=2）。
    assert_eq!(
        block.matches("root.console-maximize-toggle();").count(),
        2,
        "`clicked` 与 `accessible-action-default` 必须**同一落点**: {block}"
    );
    assert!(
        block.contains("accessible-action-default =>"),
        "最大化按钮必须有无障碍动作落点: {block}"
    );
    assert!(
        block.contains("TouchArea {") && block.contains("clicked =>"),
        "最大化按钮必须有 `TouchArea.clicked`（屏读器之外的鼠标入口）: {block}"
    );

    let app = include_str!("../ui/app.slint");
    assert!(
        app.contains("callback console-maximize-toggle();"),
        "`app.slint` 必须声明最大化回调"
    );
    assert!(
        app.contains("root.console-maximize-toggle();"),
        "`app.slint` 必须把子组件的回调原样转给主窗口"
    );
    assert!(
        app.contains("console-maximized: root.console-maximized;"),
        "`ConsoleTabs` 实例必须拿到同一份视图态（按钮的 `accessible-checked` 读它）"
    );
    assert!(
        app.contains("vertical-stretch: root.console-maximized ? 0 : Tokens.workspace-stretch;"),
        "中段的弹性高度必须绑定到 `console-maximized`（不是死值）"
    );

    let source = include_str!("../src/host.rs");
    assert_eq!(
        source
            .matches("ui.set_console_maximized(!ui.get_console_maximized())")
            .count(),
        2,
        "`console-maximized` 必须**恰好**有两个写者：`Action::ToggleConsoleMaximize` 臂 \
         与点击回调（键盘 / 点击同一属性，⛔ 不是两套状态）"
    );
    assert!(
        !source.contains("console_expanded") && !source.contains("console-expanded"),
        "旧属性名 `console-expanded` 必须已消失（它只被翻转、从未驱动布局）"
    );
}
