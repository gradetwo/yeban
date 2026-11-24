//! **Session「→ 编曲」按钮**（`session-back-to-arrangement-button`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! Session 视图里那个红边的「→ 编曲」按钮**不再**是"能点、但什么也没发生"的死点击：
//! 一次真实指针点击把 `arrangement-view` 写成 `true`，主工作区从 Session 触发矩阵
//! 换回时间轴；同一个属性上再按一次**不会**把用户踢回 Session（写 `true`，⛔ 不是翻转）。
//! 纯视图态 —— 不改工程、不进撤销栈（`MODEL-ISO-001` 的挥发性层）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（装配态） | 默认 `arrangement-view == true`：Session 子树与返回键都不在运行时树里 |
//! | ②（可寻址） | 切到 Session 后返回键在场、角色 = `Button`、名称与注册表逐字一致、几何非零 |
//! | ③（真点击） | 注入一次真实指针点击 ⇒ 属性 `false → true`；编曲画布回到树里、Session 画布与返回键消失 |
//! | ④（逐位还原） | 点击后的整帧像素与装配态默认帧**逐字节相同**；且 Session 帧必须与默认帧不同（否则 ④ 证明不了切换） |
//! | ⑤（写 true 不翻转） | Session 态下 `invoke_back_to_arrangement()` ⇒ `true`；再调一次仍是 `true` |
//! | ⑥（文本层接线） | `session_view.slint` 的 `clicked` 与 `accessible-action-default` 同一落点；`app.slint` 原样转发；宿主是唯一落点 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/sidebar_item.rs` / `tests/scene_launch_wiring.rs` 同款）。

use yeban_app::bridge::ViewState;
use yeban_app::cli::Language;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ 三轨 + 一场景全部装得下）。
const WINDOW: Size = Size::new(1440, 900);

const BACK_BUTTON: &str = "session-back-to-arrangement-button";

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
    // ⭐ T-FIX-233 返工（复验在 `LANG=en_US.UTF-8` 下 0/1 红）：本判据要逐字核对
    //   `session_view.slint` 的 `@tr("返回编曲 (Back to Arrangement)")` 与注册表名，
    //   而 `build_main_window` 不走启动路径的 `apply_language`，Slint 会按系统
    //   locale 自动选词表（英文系统读到 `"Back to Arrangement"`，见 host.rs 的
    //   `apply_language` 文档）⇒ 必须在测试入口把语言钉成源语言 zh-CN；产品代码
    //   不动，只把测试前提钉死（同 T-FIX-228 的 `pane_no_overlap` 判据）。
    host::apply_language(port.ui(), Language::ZhCn);
    (port, view)
}

#[test]
fn clicking_back_to_arrangement_returns_to_the_arrangement_view() {
    let (mut port, view) = live_window();

    // ① 装配态：主视图默认是编曲；Session 画布与返回键都不在运行时树里。
    assert!(
        port.ui().get_arrangement_view(),
        "装配态 `arrangement-view` 必须是 true"
    );
    assert!(
        port.tree().contains("workspace-arrangement-canvas"),
        "装配态必须在运行时树里含 `workspace-arrangement-canvas`"
    );
    assert!(
        !port.tree().contains("workspace-session-canvas"),
        "装配态不许在运行时树里含 `workspace-session-canvas`"
    );
    assert!(
        !port.tree().contains(BACK_BUTTON),
        "装配态不许在运行时树里含返回键 `{BACK_BUTTON}`"
    );
    // ④ 的基线：先抓装配态整帧；指针此时在 (0,0)，与点击收尾后的归位位置相同。
    let default_frame = port.window().capture().expect("装配态必须能渲染整帧");

    // ② 切到 Session：返回键在场、角色 / 名称与注册表一致、几何非零。
    port.ui().set_arrangement_view(false);
    port.refresh_tree(None)
        .expect("切到 Session 之后必须能重抓控件树");
    assert!(
        !port.ui().get_arrangement_view(),
        "切到 Session 后 `arrangement-view` 必须是 false"
    );
    assert!(
        port.tree().contains("workspace-session-canvas"),
        "切到 Session 后必须含 `workspace-session-canvas`"
    );
    let button = port
        .tree()
        .find_by_id(BACK_BUTTON)
        .expect("Session 视图必须有返回键")
        .clone();
    assert_eq!(button.role.as_str(), "button", "返回键角色必须是 Button");
    assert!(
        button.label.contains("返回编曲"),
        "返回键标签必须含「返回编曲」，实际 {:?}",
        button.label
    );
    let registry = ElementRegistry::from_view(&view);
    let meta = registry
        .iter()
        .find(|meta| meta.id == BACK_BUTTON)
        .expect("注册表必须登记返回键");
    assert_eq!(meta.kind, ElementKind::Button, "注册表角色必须是 Button");
    assert_eq!(
        button.label, meta.label,
        "返回键的运行时名称必须与注册表逐字一致"
    );
    let bounds = button.bounds.expect("返回键必须有几何");
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "返回键几何必须非零: {bounds:?}"
    );
    // ④ 的对照：Session 帧必须与装配态不同，否则下面的“点击后还原”证明不了切换。
    let session_frame = port.window().capture().expect("Session 视图必须能渲染整帧");
    assert!(
        session_frame.pixels() != default_frame.pixels(),
        "Session 帧必须与装配态帧不同，否则本判据测不到视图切换"
    );

    // ③ 真点击：指针按下 / 释放都走生产事件链，不是直接 `invoke_back_to_arrangement`。
    port.dispatch_pointer_down(BACK_BUTTON, 2.0, 2.0, PointerButton::Left)
        .expect("返回键必须可按下");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("返回键必须可释放");
    port.refresh_tree(None).expect("点击之后必须能重抓控件树");
    assert!(
        port.ui().get_arrangement_view(),
        "真实点击返回键后 `arrangement-view` 必须是 true"
    );
    assert!(
        port.tree().contains("workspace-arrangement-canvas"),
        "真实点击后编曲画布必须回到运行时树里"
    );
    assert!(
        !port.tree().contains("workspace-session-canvas"),
        "真实点击后 Session 画布必须从运行时树里消失"
    );
    assert!(
        !port.tree().contains(BACK_BUTTON),
        "真实点击后返回键必须从运行时树里消失"
    );

    // ④ 逐位还原：把指针移回装配态的位置后，点击后的整帧必须与默认帧逐字节相同。
    //    （指针 hover 是像素的一部分；不归位会把“按钮下的 hover 高亮”误算成视图差异。）
    port.dispatch_pointer_move(0.0, 0.0)
        .expect("点击收尾后指针必须能归位");
    let after_frame = port.window().capture().expect("点击后必须能渲染整帧");
    assert_eq!(
        after_frame.pixels(),
        default_frame.pixels(),
        "点击返回键后的整帧必须与装配态默认帧逐字节相同"
    );

    // ⑤ 写 true 不翻转：Session 态下直接调用回调两次，属性都应保持 true。
    port.ui().set_arrangement_view(false);
    port.ui().invoke_back_to_arrangement();
    assert!(
        port.ui().get_arrangement_view(),
        "Session 态下 `invoke_back_to_arrangement()` 必须写 true"
    );
    port.ui().set_arrangement_view(false);
    port.ui().invoke_back_to_arrangement();
    assert!(
        port.ui().get_arrangement_view(),
        "第二次 `invoke_back_to_arrangement()` 仍必须写 true（⛔ 不是翻转）"
    );

    // ⑥ 文本层接线：`.slint` 两条落点逐字相同；`app.slint` 原样转发；宿主是唯一注册者。
    let session_view = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/workspace/session_view.slint"),
    )
    .expect("读 session_view.slint");
    assert_eq!(
        session_view.matches("root.back-to-arrangement();").count(),
        2,
        "`session_view.slint` 的 clicked 与 accessible-action-default 必须都是 `root.back-to-arrangement();`"
    );
    let app_slint = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    assert!(
        app_slint.contains("back-to-arrangement => { root.back-to-arrangement(); }"),
        "`app.slint` 必须原样转发 `back-to-arrangement`"
    );
    let host_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        host_source.contains("ui.on_back_to_arrangement({"),
        "宿主的 `wire_view_callbacks` 必须注册 `on_back_to_arrangement`"
    );
}
